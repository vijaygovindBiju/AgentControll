use super::{CheckResult, Doctor};
use std::env;

pub fn check_environment(_doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let watched_vars = [
        "GEMINI_API_KEY",
        "GOOGLE_API_KEY",
        "GOOGLE_APPLICATION_CREDENTIALS",
        "GOOGLE_GENAI_USE_VERTEXAI",
        "GOOGLE_CLOUD_ACCESS_TOKEN",
    ];

    let mut detected = Vec::new();

    for var in watched_vars {
        if let Ok(val) = env::var(var) {
            if !val.trim().is_empty() {
                detected.push(var);
            }
        }
    }

    if detected.is_empty() {
        checks.push(CheckResult::pass(
            "env.ambient_credentials",
            "Ambient Google credentials",
            "No conflicting ambient Google credential environment variables detected",
        ));
    } else {
        checks.push(CheckResult::warn(
            "env.ambient_credentials",
            "Ambient Google credentials",
            format!("Ambient Google credentials detected in shell: {}", detected.join(", ")),
            "AgentControll isolates sessions by removing these ambient credentials upon launch, but unset them in your shell to avoid unintended bleed.",
        ));
    }

    // Check D-Bus session bus and desktop keyring isolation
    #[cfg(unix)]
    {
        let dbus_addr = env::var("DBUS_SESSION_BUS_ADDRESS").ok();
        let details = if let Some(ref addr) = dbus_addr {
            format!(
                "Host D-Bus address: {addr}\n\
                Child agy sessions are launched with DBUS_SESSION_BUS_ADDRESS=disabled: to prevent GNOME Keyring / KWallet token interception over D-Bus."
            )
        } else {
            "No host D-Bus session bus active in ambient shell (headless/container environment).\n\
            Child agy sessions enforce DBUS_SESSION_BUS_ADDRESS=disabled: for hermetic token isolation."
                .to_string()
        };

        checks.push(
            CheckResult::pass(
                "env.keyring_isolation",
                "Desktop Keyring & D-Bus isolation",
                "Hermetic environment isolation configured (DBUS_SESSION_BUS_ADDRESS=disabled: on child processes)",
            )
            .with_details(details),
        );
    }
}
