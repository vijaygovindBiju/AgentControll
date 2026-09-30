//! Mock adapter — emits a scripted sequence of `AdapterEvent`s.
//!
//! Used for all Phase 1 tests. The script is a `Vec<(delay_ms, AdapterEvent)>`.
//! Events are sent to the session manager via the channel after the specified delay.

use anyhow::Result;
use tokio::sync::mpsc;
use tracing::debug;

use crate::{
    adapter::{AdapterFactory, AdapterHandle},
    types::{AdapterEvent, Id},
};

/// A single scripted event with an optional delay.
#[derive(Debug, Clone)]
pub struct ScriptedEvent {
    /// Delay before emitting this event, in milliseconds.
    pub delay_ms: u64,
    pub event: AdapterEvent,
}

impl ScriptedEvent {
    pub fn immediate(event: AdapterEvent) -> Self {
        Self { delay_ms: 0, event }
    }

    pub fn delayed(delay_ms: u64, event: AdapterEvent) -> Self {
        Self { delay_ms, event }
    }
}

/// A script for a mock adapter session.
pub type MockScript = Vec<ScriptedEvent>;

/// A simple script that: starts ready, works, then completes.
pub fn default_script() -> MockScript {
    vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            10,
            AdapterEvent::OutputChunk {
                text: "Working on task...".into(),
                confidence: crate::types::Confidence::High,
            },
        ),
        ScriptedEvent::delayed(
            20,
            AdapterEvent::Completed {
                summary: Some("Task complete".into()),
            },
        ),
    ]
}

/// A script that immediately fails on start.
pub fn start_fail_script(reason: &str) -> MockScript {
    vec![ScriptedEvent::immediate(AdapterEvent::StartFailed {
        reason: reason.to_owned(),
    })]
}

/// A script that crashes after starting.
pub fn crash_script() -> MockScript {
    vec![
        ScriptedEvent::immediate(AdapterEvent::Ready),
        ScriptedEvent::delayed(
            10,
            AdapterEvent::Crashed {
                exit_code: Some(1),
                reason: Some("simulated crash".into()),
            },
        ),
    ]
}

/// Factory for `MockAdapter`s. Each call uses the next script in the queue.
///
/// Construct with a list of scripts (one per expected `create` call).
/// Panics in tests if more sessions are created than scripts provided.
pub struct MockAdapterFactory {
    scripts: std::collections::VecDeque<MockScript>,
    capabilities: Option<crate::types::ProviderCapabilities>,
}

impl MockAdapterFactory {
    /// Create a factory that returns the same script for every session.
    pub fn always(script: MockScript) -> Self {
        // We store many copies since VecDeque pops front
        Self {
            scripts: std::collections::VecDeque::from(vec![
                script.clone(),
                script.clone(),
                script.clone(),
                script.clone(),
                script.clone(),
                script,
            ]),
            capabilities: None,
        }
    }

    /// Create a factory with a specific list of scripts (consumed in order).
    pub fn sequence(scripts: Vec<MockScript>) -> Self {
        Self {
            scripts: std::collections::VecDeque::from(scripts),
            capabilities: None,
        }
    }

    pub fn with_capabilities(mut self, caps: crate::types::ProviderCapabilities) -> Self {
        self.capabilities = Some(caps);
        self
    }
}

impl AdapterFactory for MockAdapterFactory {
    fn capabilities(&self, _agent_type: &str) -> crate::types::ProviderCapabilities {
        self.capabilities.clone().unwrap_or_else(|| {
            crate::types::ProviderCapabilities::new(
                crate::types::AccountSwitchMode::RequiresRestart,
                true,
                true,
            )
        })
    }

    fn create(
        &mut self,
        ctx: crate::types::SessionContext,
        event_tx: mpsc::Sender<AdapterEvent>,
    ) -> Result<AdapterHandle> {
        let script = self
            .scripts
            .pop_front()
            .ok_or_else(|| anyhow::anyhow!("MockAdapterFactory: no more scripts"))?;

        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let abort_handle = spawn_mock_task(ctx.session_id, script, event_tx, cmd_rx);
        Ok(AdapterHandle::with_cmd_tx(Box::new(abort_handle), cmd_tx))
    }
}

/// Spawn the async task that drives the mock script.
fn spawn_mock_task(
    session_id: Id,
    script: MockScript,
    event_tx: mpsc::Sender<AdapterEvent>,
    mut cmd_rx: mpsc::Receiver<crate::types::AgentCommand>,
) -> tokio::task::AbortHandle {
    let handle = tokio::spawn(async move {
        // Discard or log incoming commands in background
        let cmd_sid = session_id.clone();
        tokio::spawn(async move {
            while let Some(cmd) = cmd_rx.recv().await {
                debug!("MockAdapter [{}]: received command {:?}", cmd_sid, cmd);
            }
        });

        for scripted in script {
            if scripted.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(scripted.delay_ms)).await;
            }
            debug!(
                "MockAdapter [{}]: emitting {:?}",
                session_id, scripted.event
            );
            if event_tx.send(scripted.event).await.is_err() {
                break; // channel closed (session stopped)
            }
        }
    });
    handle.abort_handle()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn default_script_emits_ready_first() {
        let mut factory = MockAdapterFactory::always(default_script());
        let (tx, mut rx) = mpsc::channel(16);
        let _handle = factory
            .create_simple(Id::new(), "test task".into(), "mock".into(), tx)
            .unwrap();

        let first = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
            .await
            .expect("timeout")
            .expect("channel closed");

        assert!(matches!(first, AdapterEvent::Ready));
    }

    #[tokio::test]
    async fn start_fail_script_emits_start_failed() {
        let mut factory = MockAdapterFactory::always(start_fail_script("test error"));
        let (tx, mut rx) = mpsc::channel(16);
        let _handle = factory
            .create_simple(Id::new(), "test".into(), "mock".into(), tx)
            .unwrap();

        let event = rx.recv().await.unwrap();
        assert!(matches!(event, AdapterEvent::StartFailed { .. }));
    }
}
