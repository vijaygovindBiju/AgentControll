use super::{installation::find_in_path, CheckResult, Doctor};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub async fn check_agy(doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    // 1. Resolve binary using AgentControll's own resolution logic
    let resolved_str = if let Some(ref custom_bin) = doctor.opts.agy_bin {
        custom_bin.to_string_lossy().to_string()
    } else {
        crate::adapter::resolve_real_antigravity_bin()
    };

    let resolved_path = PathBuf::from(&resolved_str);
    let resolved_exists = resolved_path.is_file();

    // Check all instances in PATH
    let path_matches = find_in_path("agy").unwrap_or_default();

    if !resolved_exists && path_matches.is_empty() {
        checks.push(CheckResult::fail(
            "agy.binary",
            "Antigravity CLI (agy) executable resolution",
            "No `agy` executable found on system",
            "Install the official Google Antigravity CLI (`agy`) and ensure it is accessible in ~/.local/bin or PATH.",
        ));
        return;
    }

    let actual_bin = if resolved_exists {
        resolved_path
    } else {
        path_matches[0].clone()
    };

    // Check binary characteristics
    let (is_wrapper, size_bytes) = inspect_binary_nature(&actual_bin);

    let size_mb = size_bytes / (1024 * 1024);
    let details = format!(
        "Path: {}\nSize: {} MB ({} bytes)\nInstances in PATH: {}",
        actual_bin.display(),
        size_mb,
        size_bytes,
        if path_matches.is_empty() {
            "none (resolved via HOME fallback)".to_string()
        } else {
            format!("{}", path_matches.len())
        }
    );

    if is_wrapper {
        checks.push(CheckResult::warn(
            "agy.binary",
            "Antigravity CLI (agy) executable resolution",
            format!("Executable at {} appears to be an older AgentControll wrapper, not the official Google agy binary", actual_bin.display()),
            "AgentControll does not own the `agy` command. Replace it with the official Google Antigravity CLI binary.",
        ).with_details(details));
    } else {
        checks.push(
            CheckResult::pass(
                "agy.binary",
                "Antigravity CLI (agy) executable resolution",
                format!(
                    "External Google CLI located at {} (AgentControll wrapper: not present)",
                    actual_bin.display()
                ),
            )
            .with_details(details),
        );
    }

    // 2. Safely run `agy --version` with a 2-second timeout
    let version_output = run_agy_version(&actual_bin).await;
    match version_output {
        Ok(v) if !v.trim().is_empty() => {
            checks.push(CheckResult::pass(
                "agy.version",
                "Antigravity CLI version",
                format!("agy version {}", v.trim()),
            ));
        }
        Ok(_) => {
            checks.push(CheckResult::warn(
                "agy.version",
                "Antigravity CLI version",
                "Running `agy --version` returned an empty response",
                "Verify that your `agy` executable executes properly from the terminal.",
            ));
        }
        Err(e) => {
            checks.push(CheckResult::warn(
                "agy.version",
                "Antigravity CLI version",
                format!("Failed to retrieve version: {e}"),
                "Verify that `agy` has execute permissions and runs without hanging.",
            ));
        }
    }
}

/// Inspects whether a file appears to be an old AgentControll wrapper or the external Google binary.
/// Official Google agy binary is dynamically linked Go/C++ stripped ELF ~200MB+.
/// Old AgentControll wrapper was a Rust binary under 20MB containing ac_cli/agentcontrol strings.
fn inspect_binary_nature(path: &Path) -> (bool, u64) {
    let size = match std::fs::metadata(path) {
        Ok(m) => m.len(),
        Err(_) => return (false, 0),
    };

    // If binary is small (< 25MB), inspect for AgentControll signatures
    if size < 25 * 1024 * 1024 {
        if let Ok(bytes) = std::fs::read(path) {
            // Check for AgentControll signatures
            let signature = b"AgentControll".iter().cloned().collect::<Vec<u8>>();
            let socket_sig = b"agentcontrol.sock".iter().cloned().collect::<Vec<u8>>();
            if contains_subslice(&bytes, &signature) || contains_subslice(&bytes, &socket_sig) {
                return (true, size);
            }
        }
    }

    (false, size)
}

fn contains_subslice(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

/// Runs `agy --version` with a 2-second timeout.
async fn run_agy_version(bin_path: &Path) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(bin_path);
    cmd.arg("--version");
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    // Strip ambient credentials for cleanliness
    for k in crate::agy_auth::AMBIENT_AUTH_ENV {
        cmd.env_remove(k);
    }

    let timeout_duration = Duration::from_secs(2);
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn {}: {e}", bin_path.display()))?;

    match tokio::time::timeout(timeout_duration, child.wait_with_output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                let out_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !out_str.is_empty() {
                    Ok(out_str)
                } else {
                    Ok(String::from_utf8_lossy(&output.stderr).trim().to_string())
                }
            } else {
                let err_str = String::from_utf8_lossy(&output.stderr).trim().to_string();
                Err(format!(
                    "Process exited with status {}: {}",
                    output.status, err_str
                ))
            }
        }
        Ok(Err(e)) => Err(format!("I/O error during version check: {e}")),
        Err(_) => Err("Timed out waiting for `agy --version` after 2 seconds".into()),
    }
}
