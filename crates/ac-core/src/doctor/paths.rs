use super::{CheckResult, Doctor};
use std::path::Path;

pub fn check_paths(doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let config_dir = doctor.config_dir();
    let data_dir = doctor.data_dir();
    let cache_dir = doctor.cache_dir();

    check_directory(
        "paths.config",
        "AgentControll configuration directory",
        &config_dir,
        true, // required
        checks,
    );

    check_directory(
        "paths.data",
        "AgentControll data directory",
        &data_dir,
        true, // required
        checks,
    );

    check_directory(
        "paths.cache",
        "AgentControll cache directory",
        &cache_dir,
        false, // optional (created on demand)
        checks,
    );
}

fn check_directory(
    id: &str,
    title: &str,
    path: &Path,
    required: bool,
    checks: &mut Vec<CheckResult>,
) {
    if !path.exists() {
        if required {
            checks.push(CheckResult::warn(
                id,
                title,
                format!("Directory {} does not exist", path.display()),
                "Directory will be initialized on first use, or check file permissions if you expect existing configurations.",
            ));
        } else {
            checks.push(CheckResult::info(
                id,
                title,
                format!(
                    "Directory {} does not exist (optional, created on demand)",
                    path.display()
                ),
            ));
        }
        return;
    }

    if !path.is_dir() {
        checks.push(CheckResult::fail(
            id,
            title,
            format!("Path {} exists but is not a directory", path.display()),
            format!(
                "Remove or rename the file at {} and let AgentControll recreate the directory.",
                path.display()
            ),
        ));
        return;
    }

    // Check permissions and readability
    match std::fs::read_dir(path) {
        Ok(entries) => {
            let count = entries.flatten().count();
            checks.push(CheckResult::pass(
                id,
                title,
                format!("{} ({} items, readable)", path.display(), count),
            ));
        }
        Err(e) => {
            checks.push(CheckResult::fail(
                id,
                title,
                format!("Cannot read directory {}: {}", path.display(), e),
                "Check file ownership and permissions (`chmod 0700` / `chown`).",
            ));
        }
    }
}
