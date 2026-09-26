//! Generic PTY adapter.
//!
//! Provides process spawning inside a pseudo-terminal (PTY), input/output
//! streaming, pattern-based event extraction (Confidence::Low), process lifecycle
//! detection (crash, exit, exit codes), and pause/resume/stop control.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{bail, Context, Result};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};
use regex::Regex;
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::adapter::AdapterHandle;
use crate::types::{
    AccountSwitchMode, AdapterEvent, AgentCommand, Confidence, ProviderCapabilities, SessionContext,
};

/// Kind of event that can be emitted when a regex pattern matches PTY output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyPatternKind {
    Ready,
    ApprovalRequested { tool_name: Option<String> },
    QuestionRaised,
    RateLimitSignal { back_off_secs: Option<u64> },
    Completed { summary: Option<String> },
}

/// A regex rule that triggers an `AdapterEvent` when matched against output.
#[derive(Debug, Clone)]
pub struct PtyPatternRule {
    pub pattern: Regex,
    pub kind: PtyPatternKind,
}

impl PtyPatternRule {
    pub fn new(pattern: &str, kind: PtyPatternKind) -> Result<Self> {
        let pattern = Regex::new(pattern).context("compiling regex pattern")?;
        Ok(Self { pattern, kind })
    }
}

/// Configuration for the Generic PTY adapter.
#[derive(Debug, Clone)]
pub struct PtyConfig {
    pub program: String,
    pub args: Vec<String>,
    pub envs: HashMap<String, String>,
    /// Inherited environment variables to remove from the child.
    pub env_remove: Vec<String>,
    pub patterns: Vec<PtyPatternRule>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            program: "/bin/sh".into(),
            args: vec![],
            envs: HashMap::new(),
            env_remove: vec![],
            patterns: vec![],
        }
    }
}

impl PtyConfig {
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: vec![],
            envs: HashMap::new(),
            env_remove: vec![],
            patterns: vec![],
        }
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn with_args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        for arg in args {
            self.args.push(arg.into());
        }
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.envs.insert(key.into(), val.into());
        self
    }

    pub fn with_pattern(mut self, pattern: &str, kind: PtyPatternKind) -> Result<Self> {
        self.patterns.push(PtyPatternRule::new(pattern, kind)?);
        Ok(self)
    }
}

/// Active Generic PTY adapter instance.
pub struct GenericPtyAdapter;

impl GenericPtyAdapter {
    /// Provider capabilities for Generic PTY.
    /// Changing accounts requires restarting the underlying CLI process.
    pub fn capabilities() -> ProviderCapabilities {
        ProviderCapabilities::new(
            AccountSwitchMode::RequiresRestart,
            false, // CLI process cannot resume session
            false, // CLI process cannot restore snapshot
        )
    }

    /// Spawns a process in a PTY and returns an `AdapterHandle`.
    pub fn spawn(
        ctx: &SessionContext,
        config: PtyConfig,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        // Enforce workspace isolation
        if let Some(ref ws) = ctx.workspace_path {
            let p = PathBuf::from(ws);
            if !p.exists() || !p.is_dir() {
                bail!("Workspace path does not exist or is not a directory: {}", ws);
            }
        }

        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("allocating PTY")?;

        let mut cmd = CommandBuilder::new(&config.program);
        cmd.args(&config.args);

        // Workspace cwd isolation
        if let Some(ref ws) = ctx.workspace_path {
            cmd.cwd(ws);
        }

        // Environment variables
        for k in &config.env_remove {
            cmd.env_remove(k);
        }
        for (k, v) in &config.envs {
            cmd.env(k, v);
        }

        let mut child = pair.slave.spawn_command(cmd).context("spawning PTY child process")?;
        drop(pair.slave); // Crucial: close slave in parent so master gets EOF on child exit

        let child_pid = child.process_id();
        let reader = pair.master.try_clone_reader().context("cloning PTY reader")?;
        let writer = pair.master.take_writer().context("taking PTY writer")?;
        let writer = Arc::new(Mutex::new(writer));
        let master = Arc::new(Mutex::new(pair.master));

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<AgentCommand>(64);
        let event_tx_cmd = event_tx.clone();
        let writer_cmd = writer.clone();
        let master_cmd = master.clone();

        // ── Command loop ──────────────────────────────────────────────────────
        let cmd_task = tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                match cmd {
                    AgentCommand::Input { data } => {
                        let w = writer_cmd.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Ok(mut lock) = w.lock() {
                                let _ = lock.write_all(data.as_bytes());
                                let _ = lock.flush();
                            }
                        })
                        .await;
                    }
                    AgentCommand::Resize { rows, cols } => {
                        let m = master_cmd.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Ok(lock) = m.lock() {
                                let _ = lock.resize(portable_pty::PtySize {
                                    rows,
                                    cols,
                                    pixel_width: 0,
                                    pixel_height: 0,
                                });
                            }
                        })
                        .await;
                    }
                    AgentCommand::Respond { allow, response } => {
                        let text = if let Some(r) = response {
                            format!("{}\r\n", r)
                        } else if allow {
                            "y\r\n".to_string()
                        } else {
                            "n\r\n".to_string()
                        };
                        let w = writer_cmd.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Ok(mut lock) = w.lock() {
                                let _ = lock.write_all(text.as_bytes());
                                let _ = lock.flush();
                            }
                        })
                        .await;
                    }
                    AgentCommand::Steer { message } => {
                        let text = format!("{}\r\n", message);
                        let w = writer_cmd.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            if let Ok(mut lock) = w.lock() {
                                let _ = lock.write_all(text.as_bytes());
                                let _ = lock.flush();
                            }
                        })
                        .await;
                    }
                    AgentCommand::Pause => {
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGSTOP);
                            }
                        }
                        let _ = event_tx_cmd.send(AdapterEvent::Paused).await;
                    }
                    AgentCommand::Resume => {
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGCONT);
                            }
                        }
                        let _ = event_tx_cmd.send(AdapterEvent::Resumed).await;
                    }
                    AgentCommand::Stop { reason: _ } => {
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGTERM);
                            }
                        }
                        tokio::time::sleep(Duration::from_millis(300)).await;
                        #[cfg(unix)]
                        if let Some(pid) = child_pid {
                            unsafe {
                                libc::kill(pid as i32, libc::SIGKILL);
                            }
                        }
                    }
                    AgentCommand::Snapshot => {
                        let _ = event_tx_cmd
                            .send(AdapterEvent::SnapshotProduced {
                                summary: "PTY session snapshot".into(),
                            })
                            .await;
                    }
                    AgentCommand::Start { .. } => {}
                    AgentCommand::SwitchAccount { .. } => {}
                }
            }
        });

        // ── Reader & lifecycle thread ─────────────────────────────────────────
        let patterns = config.patterns;
        let event_tx_reader = event_tx;
        let mut reader = reader;
        let dropped_chunks = Arc::new(AtomicU64::new(0));
        let dropped_for_guard = dropped_chunks.clone();

        let reader_thread = std::thread::spawn(move || {
            // Emitted initially on successful launch
            let _ = event_tx_reader.blocking_send(AdapterEvent::Ready);

            // 8 KB buffer — large enough for typical AGY output bursts so we
            // do fewer send() calls per frame, vastly reducing the odds of the
            // channel filling up and causing back-pressure.
            let mut buf = [0u8; 8192];
            let mut line_buffer = String::new();
            let mut total_bytes: u64 = 0;
            let mut total_chunks: u64 = 0;

            debug!("PTY reader thread started (pid={:?})", child_pid);

            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        // EOF reached
                        debug!(
                            "PTY reader EOF after {} bytes in {} chunks ({} dropped)",
                            total_bytes,
                            total_chunks,
                            dropped_chunks.load(Ordering::Relaxed)
                        );
                        break;
                    }
                    Ok(n) => {
                        total_bytes += n as u64;
                        total_chunks += 1;
                        let chunk = String::from_utf8_lossy(&buf[..n]).to_string();

                        // Non-blocking send: if the channel is full, drop
                        // the chunk rather than blocking this thread.
                        // Blocking here causes the PTY OS buffer to fill,
                        // which makes the child process (AGY) block on
                        // write(), appearing "stuck at Generating...".
                        match event_tx_reader.try_send(AdapterEvent::OutputChunk {
                            text: chunk.clone(),
                            confidence: Confidence::Low,
                        }) {
                            Ok(()) => {}
                            Err(mpsc::error::TrySendError::Full(_)) => {
                                let n_dropped = dropped_chunks.fetch_add(1, Ordering::Relaxed) + 1;
                                if n_dropped == 1 || n_dropped % 50 == 0 {
                                    warn!(
                                        "PTY output channel full — dropped {} chunk(s); \
                                         consumer may be too slow",
                                        n_dropped
                                    );
                                }
                            }
                            Err(mpsc::error::TrySendError::Closed(_)) => {
                                debug!("PTY event channel closed, reader exiting");
                                break;
                            }
                        }

                        line_buffer.push_str(&chunk);

                        // Pattern matching
                        for rule in &patterns {
                            if let Some(captures) = rule.pattern.captures(&line_buffer) {
                                match &rule.kind {
                                    PtyPatternKind::Ready => {
                                        let _ = event_tx_reader.blocking_send(AdapterEvent::Ready);
                                    }
                                    PtyPatternKind::ApprovalRequested { tool_name } => {
                                        let tname = tool_name
                                            .clone()
                                            .unwrap_or_else(|| "command".into());
                                        let prompt = captures
                                            .get(0)
                                            .map(|m| m.as_str().to_string())
                                            .unwrap_or_default();
                                        let _ = event_tx_reader.blocking_send(
                                            AdapterEvent::ApprovalRequested {
                                                tool_name: tname,
                                                prompt,
                                            },
                                        );
                                    }
                                    PtyPatternKind::QuestionRaised => {
                                        let prompt = captures
                                            .get(0)
                                            .map(|m| m.as_str().to_string())
                                            .unwrap_or_default();
                                        let _ = event_tx_reader.blocking_send(
                                            AdapterEvent::QuestionRaised { prompt },
                                        );
                                    }
                                    PtyPatternKind::RateLimitSignal { back_off_secs } => {
                                        let _ = event_tx_reader.blocking_send(
                                            AdapterEvent::RateLimitSignal {
                                                back_off_secs: *back_off_secs,
                                            },
                                        );
                                    }
                                    PtyPatternKind::Completed { summary } => {
                                        let _ = event_tx_reader.blocking_send(
                                            AdapterEvent::Completed {
                                                summary: summary.clone(),
                                            },
                                        );
                                    }
                                }
                            }
                        }

                        // Cap buffer to avoid unbounded growth
                        if line_buffer.len() > 16384 {
                            let keep = line_buffer.split_off(line_buffer.len() - 8192);
                            line_buffer = keep;
                        }
                    }
                    Err(e) => {
                        debug!("PTY reader read error or EOF: {e}");
                        break;
                    }
                }
            }

            // Detect process exit status
            trace!("PTY reader: waiting for child process exit");
            match child.wait() {
                Ok(status) if status.success() => {
                    let _ = event_tx_reader.blocking_send(AdapterEvent::Completed {
                        summary: Some("PTY process exited normally".into()),
                    });
                }
                Ok(status) => {
                    let code = status.exit_code();
                    let _ = event_tx_reader.blocking_send(AdapterEvent::Crashed {
                        exit_code: Some(code as i32),
                        reason: Some(format!("PTY process exited with status {}", code)),
                    });
                }
                Err(e) => {
                    let _ = event_tx_reader.blocking_send(AdapterEvent::Crashed {
                        exit_code: None,
                        reason: Some(format!("PTY process wait error: {e}")),
                    });
                }
            }
        });

        // Drop guard that kills child and terminates threads
        let drop_guard = Box::new(PtyDropGuard {
            child_pid,
            cmd_task_abort: cmd_task.abort_handle(),
            _reader_join: Some(reader_thread),
            _dropped_chunks: dropped_for_guard,
        });

        Ok(AdapterHandle::with_cmd_tx(drop_guard, cmd_tx))
    }
}

struct PtyDropGuard {
    child_pid: Option<u32>,
    cmd_task_abort: tokio::task::AbortHandle,
    _reader_join: Option<std::thread::JoinHandle<()>>,
    _dropped_chunks: Arc<AtomicU64>,
}

impl Drop for PtyDropGuard {
    fn drop(&mut self) {
        let dropped = self._dropped_chunks.load(Ordering::Relaxed);
        if dropped > 0 {
            warn!(
                "PTY adapter teardown: {} output chunk(s) were dropped due to back-pressure",
                dropped
            );
        }
        self.cmd_task_abort.abort();
        #[cfg(unix)]
        if let Some(pid) = self.child_pid {
            unsafe {
                libc::kill(pid as i32, libc::SIGKILL);
            }
        }
    }
}

/// Factory for creating Generic PTY adapters.
pub struct GenericPtyAdapterFactory {
    config: PtyConfig,
}

impl GenericPtyAdapterFactory {
    pub fn new(config: PtyConfig) -> Self {
        Self { config }
    }
}

impl crate::adapter::AdapterFactory for GenericPtyAdapterFactory {
    fn capabilities(&self, _agent_type: &str) -> ProviderCapabilities {
        GenericPtyAdapter::capabilities()
    }

    fn create(
        &mut self,
        ctx: SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        GenericPtyAdapter::spawn(&ctx, self.config.clone(), event_tx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Id;

    #[tokio::test]
    async fn test_pty_process_startup_and_output() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "test task".into(), "generic-pty".into());

        let config = PtyConfig::new("echo").with_arg("hello from pty");
        let _handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_ready = false;
        let mut got_output = false;
        let mut got_completed = false;

        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                match evt {
                    AdapterEvent::Ready => got_ready = true,
                    AdapterEvent::OutputChunk { text, confidence } => {
                        if text.contains("hello from pty") {
                            got_output = true;
                            assert_eq!(confidence, Confidence::Low);
                        }
                    }
                    AdapterEvent::Completed { .. } => {
                        got_completed = true;
                        break;
                    }
                    _ => {}
                }
            }
        }

        assert!(got_ready, "should receive Ready");
        assert!(got_output, "should receive OutputChunk");
        assert!(got_completed, "should receive Completed");
    }

    #[tokio::test]
    async fn test_pty_crash_detection() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "crash test".into(), "generic-pty".into());

        let config = PtyConfig::new("sh").with_args(["-c", "exit 42"]);
        let _handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_crashed = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                if let AdapterEvent::Crashed { exit_code, .. } = evt {
                    assert_eq!(exit_code, Some(42));
                    got_crashed = true;
                    break;
                }
            }
        }
        assert!(got_crashed, "should receive Crashed event with status 42");
    }

    #[tokio::test]
    async fn test_pty_workspace_binding() {
        let temp_dir = tempfile::tempdir().unwrap();
        let ws_path = temp_dir.path().to_str().unwrap().to_string();

        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "ws test".into(), "generic-pty".into())
            .with_workspace(&ws_path);

        let config = PtyConfig::new("pwd");
        let _handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut output_pwd = String::new();
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                match evt {
                    AdapterEvent::OutputChunk { text, .. } => {
                        output_pwd.push_str(&text);
                    }
                    AdapterEvent::Completed { .. } => break,
                    _ => {}
                }
            }
        }
        assert!(
            output_pwd.contains(temp_dir.path().file_name().unwrap().to_str().unwrap()),
            "output ({output_pwd}) should contain workspace dir"
        );
    }

    #[tokio::test]
    async fn test_pty_workspace_nonexistent_fails() {
        let (tx, _rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "ws fail".into(), "generic-pty".into())
            .with_workspace("/nonexistent/directory/that/does/not/exist");

        let config = PtyConfig::new("ls");
        let res = GenericPtyAdapter::spawn(&ctx, config, tx);
        assert!(res.is_err(), "nonexistent workspace must fail spawn");
    }

    #[tokio::test]
    async fn test_pty_pattern_matching_approval_and_input() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "approval test".into(), "generic-pty".into());

        // A script that prints a prompt, reads response, then prints confirmed
        let config = PtyConfig::new("sh")
            .with_args([
                "-c",
                "printf 'Do you want to run bash? '; read ans; echo \"answer was $ans\"",
            ])
            .with_pattern(
                r"Do you want to run (\w+)\?",
                PtyPatternKind::ApprovalRequested {
                    tool_name: Some("bash".into()),
                },
            )
            .unwrap();

        let mut handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_approval_req = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                if let AdapterEvent::ApprovalRequested { tool_name, prompt } = evt {
                    assert_eq!(tool_name, "bash");
                    assert!(prompt.contains("Do you want to run"));
                    got_approval_req = true;
                    // Send response
                    handle
                        .send_command(AgentCommand::Respond {
                            allow: true,
                            response: Some("yes_sure".into()),
                        })
                        .unwrap();
                    break;
                }
            }
        }
        assert!(got_approval_req, "should match approval request pattern");

        // Now verify output contains the answer
        let mut got_answer = false;
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                if let AdapterEvent::OutputChunk { text, .. } = evt {
                    if text.contains("answer was yes_sure") {
                        got_answer = true;
                        break;
                    }
                }
            }
        }
        assert!(got_answer, "should receive response in output");
    }

    #[tokio::test]
    async fn test_pty_graceful_shutdown() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "shutdown test".into(), "generic-pty".into());

        // Process that sleeps for 60 seconds unless killed
        let config = PtyConfig::new("sleep").with_arg("60");
        let mut handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        // Wait for ready
        let first = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(matches!(first, Ok(Some(AdapterEvent::Ready))));

        // Issue Stop command
        handle
            .send_command(AgentCommand::Stop {
                reason: Some("graceful test stop".into()),
            })
            .unwrap();

        // Process should exit
        let mut exited = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                if matches!(evt, AdapterEvent::Completed { .. } | AdapterEvent::Crashed { .. }) {
                    exited = true;
                    break;
                }
            }
        }
        assert!(exited, "process should terminate upon Stop command");
    }

    #[tokio::test]
    async fn test_pty_pause_and_resume() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "pause resume test".into(), "generic-pty".into());

        let config = PtyConfig::new("sh").with_args(["-c", "sleep 10"]);
        let mut handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let _ = rx.recv().await; // Ready

        handle.send_command(AgentCommand::Pause).unwrap();
        let paused_evt = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(matches!(paused_evt, Ok(Some(AdapterEvent::Paused))));

        handle.send_command(AgentCommand::Resume).unwrap();
        let resumed_evt = tokio::time::timeout(Duration::from_millis(500), rx.recv()).await;
        assert!(matches!(resumed_evt, Ok(Some(AdapterEvent::Resumed))));

        handle.stop();
    }

    #[tokio::test]
    async fn test_pty_stderr_capture() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "stderr test".into(), "generic-pty".into());

        let config = PtyConfig::new("sh").with_args(["-c", "echo 'error message on stderr' >&2"]);
        let _handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_stderr = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                if let AdapterEvent::OutputChunk { text, .. } = evt {
                    if text.contains("error message on stderr") {
                        got_stderr = true;
                        break;
                    }
                }
            }
        }
        assert!(got_stderr, "stderr output should be captured by PTY");
    }

    #[tokio::test]
    async fn test_pty_pattern_question_and_rate_limit() {
        let (tx, mut rx) = mpsc::channel(32);
        let ctx = SessionContext::new(Id::new(), "pattern test".into(), "generic-pty".into());

        let config = PtyConfig::new("sh")
            .with_args([
                "-c",
                "echo 'Please choose target branch?'; sleep 0.05; echo 'Rate limit exceeded 429'",
            ])
            .with_pattern(r"Please choose target branch\?", PtyPatternKind::QuestionRaised)
            .unwrap()
            .with_pattern(
                r"Rate limit exceeded",
                PtyPatternKind::RateLimitSignal {
                    back_off_secs: Some(30),
                },
            )
            .unwrap();

        let _handle = GenericPtyAdapter::spawn(&ctx, config, tx).unwrap();

        let mut got_question = false;
        let mut got_rate_limit = false;

        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            if let Ok(Some(evt)) = tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                match evt {
                    AdapterEvent::QuestionRaised { prompt } => {
                        if prompt.contains("Please choose target branch") {
                            got_question = true;
                        }
                    }
                    AdapterEvent::RateLimitSignal { back_off_secs } => {
                        if back_off_secs == Some(30) {
                            got_rate_limit = true;
                        }
                    }
                    _ => {}
                }
                if got_question && got_rate_limit {
                    break;
                }
            }
        }

        assert!(got_question, "should match question pattern");
        assert!(got_rate_limit, "should match rate limit pattern");
    }
}
