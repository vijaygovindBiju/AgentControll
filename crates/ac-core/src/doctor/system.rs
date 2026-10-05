use super::{CheckResult, Doctor};
use std::env;

pub fn check_system(_doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let os = env::consts::OS;
    let arch = env::consts::ARCH;

    // OS & Arch compatibility
    let (status_title, os_detail) = match os {
        "linux" => {
            let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .map(|k| k.trim().to_string())
                .unwrap_or_else(|_| "unknown kernel".to_string());
            ("Linux", format!("kernel {kernel}"))
        }
        "macos" => ("macOS", "Darwin".to_string()),
        "windows" => ("Windows", "PTY/Unix features limited".to_string()),
        other => (other, "untested platform".to_string()),
    };

    if os == "windows" {
        checks.push(CheckResult::warn(
            "system.platform",
            "Platform compatibility",
            format!("{status_title} {arch} ({os_detail})"),
            "AgentControll Antigravity account profiles and PTY features require a Unix-like environment (Linux / macOS).",
        ));
    } else {
        checks.push(CheckResult::pass(
            "system.platform",
            "Platform compatibility",
            format!("{status_title} {arch} ({os_detail})"),
        ));
    }

    // Shell
    let shell = env::var("SHELL").unwrap_or_else(|_| "unknown".to_string());
    checks.push(CheckResult::pass("system.shell", "User shell", shell));

    // HOME
    match env::var("HOME") {
        Ok(h) if !h.trim().is_empty() => {
            checks.push(CheckResult::pass("system.home", "Home directory (HOME)", h));
        }
        _ => {
            checks.push(CheckResult::fail(
                "system.home",
                "Home directory (HOME)",
                "HOME environment variable is unset or empty",
                "Set the HOME environment variable in your shell configuration.",
            ));
        }
    }

    // PATH
    match env::var("PATH") {
        Ok(p) if !p.trim().is_empty() => {
            let count = p.split(':').filter(|s| !s.is_empty()).count();
            checks.push(
                CheckResult::pass(
                    "system.path",
                    "System PATH",
                    format!("{count} search directories configured"),
                )
                .with_details(p),
            );
        }
        _ => {
            checks.push(CheckResult::fail(
                "system.path",
                "System PATH",
                "PATH environment variable is empty",
                "Ensure your shell exports standard system directories in PATH.",
            ));
        }
    }

    // XDG directories
    let mut xdg_info = Vec::new();
    for var in [
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_CACHE_HOME",
        "XDG_RUNTIME_DIR",
    ] {
        if let Ok(val) = env::var(var) {
            if !val.trim().is_empty() {
                xdg_info.push(format!("{var}={val}"));
            }
        }
    }
    let summary = if xdg_info.is_empty() {
        "Default XDG fallback paths in use (no explicit overrides)".to_string()
    } else {
        format!("{} explicit XDG variables set", xdg_info.len())
    };
    checks.push(
        CheckResult::pass("system.xdg", "XDG environment", summary)
            .with_details(xdg_info.join(", ")),
    );
}
