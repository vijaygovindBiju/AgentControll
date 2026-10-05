use super::{CheckResult, Doctor};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

pub async fn check_daemon(doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let socket_path = doctor.socket_path();

    if !socket_path.exists() {
        checks.push(CheckResult::info(
            "daemon.status",
            "AgentControll daemon",
            format!(
                "Daemon is not currently running (socket not found at {})",
                socket_path.display()
            ),
        ));
        return;
    }

    // Attempt quick status probe with 500ms timeout
    let probe_result = tokio::time::timeout(Duration::from_millis(500), async {
        let stream = UnixStream::connect(&socket_path).await?;
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);

        let req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "status",
            "params": {},
            "id": 1
        });
        writer.write_all(req.to_string().as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;

        let mut line = String::new();
        reader.read_line(&mut line).await?;
        let res: serde_json::Value = serde_json::from_str(&line)?;
        Ok::<serde_json::Value, anyhow::Error>(res)
    })
    .await;

    match probe_result {
        Ok(Ok(val)) => {
            let result_obj = val.get("result").unwrap_or(&val);
            let pid = result_obj.get("pid").and_then(|p| p.as_u64());
            let sessions = result_obj
                .get("sessions")
                .and_then(|s| s.as_u64())
                .unwrap_or(0);
            let uptime = result_obj
                .get("uptime_seconds")
                .and_then(|u| u.as_u64())
                .unwrap_or(0);

            let summary = if let Some(p) = pid {
                format!("Daemon running (pid {p}, {sessions} active session(s), uptime {uptime}s)")
            } else {
                format!("Daemon responsive ({sessions} active session(s))")
            };

            checks.push(CheckResult::pass(
                "daemon.status",
                "AgentControll daemon",
                summary,
            ));
        }
        Ok(Err(e)) => {
            checks.push(CheckResult::warn(
                "daemon.status",
                "AgentControll daemon",
                format!("Socket exists at {} but daemon did not respond: {e}", socket_path.display()),
                "A stale socket may exist from an earlier daemon instance. The next launch will clean it up automatically.",
            ));
        }
        Err(_) => {
            checks.push(CheckResult::warn(
                "daemon.status",
                "AgentControll daemon",
                format!(
                    "Socket exists at {} but connection timed out",
                    socket_path.display()
                ),
                "Verify if a daemon process is hanging or if socket is stale.",
            ));
        }
    }
}
