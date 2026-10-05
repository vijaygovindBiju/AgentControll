use super::{CheckResult, Doctor};
use std::path::{Path, PathBuf};

pub fn check_installation(doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let native_version = env!("CARGO_PKG_VERSION");

    // Check availability of AgentControll public executables
    let commands = ["agentcontroll", "ac", "agent-control", "agentcontrold"];
    let mut resolved_cmds = Vec::new();
    let mut missing_cmds = Vec::new();

    for cmd in commands {
        match find_in_path(cmd) {
            Some(paths) => {
                let first = paths[0].display().to_string();
                if paths.len() > 1 {
                    resolved_cmds.push(format!(
                        "{cmd} -> {first} (+{} duplicate in PATH)",
                        paths.len() - 1
                    ));
                } else {
                    resolved_cmds.push(format!("{cmd} -> {first}"));
                }
            }
            None => {
                missing_cmds.push(cmd);
            }
        }
    }

    if missing_cmds.is_empty() {
        checks.push(
            CheckResult::pass(
                "installation.executables",
                "AgentControll command availability",
                format!("All {} core commands found in PATH", commands.len()),
            )
            .with_details(resolved_cmds.join("\n")),
        );
    } else if missing_cmds.len() < commands.len() {
        checks.push(CheckResult::warn(
            "installation.executables",
            "AgentControll command availability",
            format!("Found {} of {} commands (missing: {})", commands.len() - missing_cmds.len(), commands.len(), missing_cmds.join(", ")),
            "Ensure the directory containing AgentControll binaries (e.g. ~/.npm-global/bin or ~/.cargo/bin) is in your PATH.",
        ).with_details(resolved_cmds.join("\n")));
    } else {
        checks.push(CheckResult::fail(
            "installation.executables",
            "AgentControll command availability",
            "None of the AgentControll commands were found in PATH",
            "Install AgentControll via npm (`npm install -g agentcontroll`) or build from source (`cargo build --release`).",
        ));
    }

    // Inspect npm global package version if present
    let npm_version = detect_npm_version(doctor);
    match npm_version {
        Some(ref npm_ver) => {
            if npm_ver == native_version {
                checks.push(CheckResult::pass(
                    "installation.npm_sync",
                    "Package & native binary version consistency",
                    format!("npm package {npm_ver} matches native binary {native_version}"),
                ));
            } else {
                checks.push(CheckResult::warn(
                    "installation.npm_sync",
                    "Package & native binary version consistency",
                    format!("npm package is v{npm_ver}, but running native binary is v{native_version}"),
                    format!("Run `npm update -g agentcontroll` or check that your PATH resolves the expected binary version."),
                ));
            }
        }
        None => {
            checks.push(CheckResult::info(
                "installation.npm_sync",
                "Package & native binary version consistency",
                format!(
                    "Running standalone native binary v{native_version} (npm package not detected)"
                ),
            ));
        }
    }

    // Check cached binary directories (e.g. ~/.cache/agentcontrol/bin/vX.Y.Z)
    let cache_dir = doctor.cache_dir().join("bin");
    if cache_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            let mut cached_versions = Vec::new();
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            cached_versions.push(name.to_string());
                        }
                    }
                }
            }
            cached_versions.sort();
            if !cached_versions.is_empty() {
                let current_tag = format!("v{native_version}");
                let has_current = cached_versions.contains(&current_tag);
                let details = format!("Cached versions: {}", cached_versions.join(", "));
                if has_current || npm_version.is_none() {
                    checks.push(
                        CheckResult::pass(
                            "installation.cache",
                            "Binary cache consistency",
                            format!("{} version directory found in cache", cached_versions.len()),
                        )
                        .with_details(details),
                    );
                } else {
                    checks.push(CheckResult::warn(
                        "installation.cache",
                        "Binary cache consistency",
                        format!("Cache contains [{}], but expected {current_tag}", cached_versions.join(", ")),
                        "The launcher may be executing an older cached binary release. Run `npm install -g agentcontroll@latest`.",
                    ).with_details(details));
                }
            }
        }
    }
}

/// Helper to search for all occurrences of a binary in PATH.
pub fn find_in_path(binary: &str) -> Option<Vec<PathBuf>> {
    let path_val = std::env::var("PATH").ok()?;
    let mut matches = Vec::new();
    for dir in path_val.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = Path::new(dir).join(binary);
        if candidate.is_file() {
            // Check executable permission on Unix
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Ok(meta) = candidate.metadata() {
                    if meta.permissions().mode() & 0o111 != 0 {
                        matches.push(candidate);
                    }
                }
            }
            #[cfg(not(unix))]
            {
                matches.push(candidate);
            }
        }
    }
    if matches.is_empty() {
        None
    } else {
        Some(matches)
    }
}

/// Detects npm package version if installed globally or in node_modules.
fn detect_npm_version(doctor: &Doctor) -> Option<String> {
    let home = doctor.home_dir();
    let candidates = [
        home.join(".npm-global/lib/node_modules/agentcontroll/package.json"),
        PathBuf::from("/usr/local/lib/node_modules/agentcontroll/package.json"),
        PathBuf::from("/usr/lib/node_modules/agentcontroll/package.json"),
    ];

    for candidate in &candidates {
        if candidate.is_file() {
            if let Ok(content) = std::fs::read_to_string(candidate) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(v) = json.get("version").and_then(|v| v.as_str()) {
                        return Some(v.to_string());
                    }
                }
            }
        }
    }

    None
}
