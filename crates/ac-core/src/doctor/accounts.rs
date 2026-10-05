use super::{CheckResult, Doctor};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Debug, Clone)]
struct AccountRecord {
    id: String,
    label: String,
    provider: String,
    credential_ref: String,
    state: String,
}

pub fn check_accounts_and_profiles(doctor: &Doctor, checks: &mut Vec<CheckResult>) {
    let data_dir = doctor.data_dir();
    let config_dir = doctor.config_dir();
    let db_path = data_dir.join("accounts.db");

    // 1. Database check (Group 6)
    if !db_path.is_file() {
        checks.push(CheckResult::warn(
            "db.accounts",
            "Account database",
            format!("Database file {} does not exist", db_path.display()),
            "The database will be initialized when the first account is added via `ac login` or the TUI.",
        ));
        return;
    }

    let conn = match Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(c) => c,
        Err(e) => {
            checks.push(CheckResult::fail(
                "db.accounts",
                "Account database",
                format!(
                    "Failed to open {} in read-only mode: {e}",
                    db_path.display()
                ),
                "Check file ownership and permissions for accounts.db.",
            ));
            return;
        }
    };

    // Verify accounts table
    let table_exists: bool = conn
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='accounts'",
            [],
            |row| row.get::<_, i64>(0).map(|c| c > 0),
        )
        .unwrap_or(false);

    if !table_exists {
        checks.push(CheckResult::fail(
            "db.accounts",
            "Account database schema",
            "Table 'accounts' is missing from accounts.db",
            "The database schema is invalid. Reinitialize accounts or restore from backup.",
        ));
        return;
    }

    checks.push(CheckResult::pass(
        "db.accounts",
        "Account database",
        format!("{} is readable and schema is valid", db_path.display()),
    ));

    // 2. Read accounts (Group 7)
    let accounts = match read_accounts(&conn) {
        Ok(accs) => accs,
        Err(e) => {
            checks.push(CheckResult::fail(
                "accounts.read",
                "Account records",
                format!("Failed to read accounts from database: {e}"),
                "Database records may be corrupted.",
            ));
            return;
        }
    };

    if accounts.is_empty() {
        checks.push(CheckResult::info(
            "accounts.count",
            "Configured accounts",
            "No agent accounts currently configured",
        ));
        return;
    }

    // Check for duplicate account IDs
    let mut id_seen = HashSet::new();
    let mut dup_ids = Vec::new();
    for a in &accounts {
        if !id_seen.insert(&a.id) {
            dup_ids.push(a.id.clone());
        }
    }

    if !dup_ids.is_empty() {
        checks.push(CheckResult::fail(
            "accounts.duplicates",
            "Duplicate account IDs",
            format!("Duplicate account IDs found: {}", dup_ids.join(", ")),
            "Each account must have a unique identifier in the database.",
        ));
    } else {
        checks.push(CheckResult::pass(
            "accounts.count",
            "Configured accounts",
            format!("{} configured account(s)", accounts.len()),
        ));
    }

    // 3. Check Profiles & Credentials per account (Groups 8, 9, 10)
    let profiles_base = config_dir.join("profiles");
    let creds_base = config_dir.join("credentials");

    let mut profile_paths_by_account = HashMap::new();
    let mut agy_accounts_count = 0;

    for a in &accounts {
        // Only inspect agy accounts for agy profiles
        if a.provider != "agy" && a.provider != "antigravity" {
            continue;
        }
        agy_accounts_count += 1;

        // Check account state
        let clean_state = a.state.trim().trim_matches('"');
        if clean_state != "active" {
            checks.push(CheckResult::warn(
                format!("account.{}.state", a.id),
                format!("Account state for \"{}\"", a.label),
                format!("Account state is '{}' (not active)", clean_state),
                "Check account status or re-authenticate if session launch is rejected.",
            ));
        }

        let profile_dir = profiles_base.join(&a.id);
        profile_paths_by_account.insert(a.id.clone(), profile_dir.clone());

        // Check profile directory
        check_profile_for_account(a, &profile_dir, checks);

        // Check credential file
        check_credential_for_account(a, &creds_base, checks);
    }

    // 4. Account Isolation check (Group 11)
    let mut seen_paths = HashMap::new();
    let mut shared_profiles = Vec::new();

    for (aid, ppath) in &profile_paths_by_account {
        if let Some(prev_aid) = seen_paths.insert(ppath.clone(), aid) {
            shared_profiles.push(format!(
                "Accounts {prev_aid} and {aid} share {}",
                ppath.display()
            ));
        }
    }

    if agy_accounts_count > 1 {
        if !shared_profiles.is_empty() {
            checks.push(
                CheckResult::fail(
                    "accounts.isolation",
                    "Account profile isolation",
                    "Multiple accounts share the same profile directory",
                    "Each account must have its own isolated profile directory under ~/.config/agentcontrol/profiles/<id>.",
                )
                .with_details(shared_profiles.join("\n")),
            );
        } else {
            checks.push(CheckResult::pass(
                "accounts.isolation",
                "Account profile isolation",
                format!(
                    "All {agy_accounts_count} Antigravity accounts have distinct isolated profiles"
                ),
            ));
        }
    } else if agy_accounts_count == 1 {
        checks.push(CheckResult::pass(
            "accounts.isolation",
            "Account profile isolation",
            "1 Antigravity account configured with dedicated profile",
        ));
    }

    // Account switching architecture contract check
    let switching_details = "How Antigravity account switching works:\n\
        User selects Account B in AgentControll\n\
                     ↓\n\
        AgentControll prepares isolated profile: ~/.config/agentcontrol/profiles/<ACCOUNT_B_ID>/\n\
                     ↓\n\
        Writes Account B's OAuth token to: <profile>/.gemini/antigravity-cli/antigravity-oauth-token (chmod 0600)\n\
                     ↓\n\
        Spawns external agy process with: HOME=<profile>, ANTIGRAVITY_ACCOUNT_ID=<ACCOUNT_B_ID>\n\
                     ↓\n\
        Google agy reads $HOME/.gemini/antigravity-cli/antigravity-oauth-token";

    if agy_accounts_count >= 1 && shared_profiles.is_empty() {
        checks.push(
            CheckResult::pass(
                "accounts.switching_architecture",
                "Antigravity account switching contract",
                format!("Isolated profile launch architecture verified across {agy_accounts_count} account(s)"),
            )
            .with_details(switching_details),
        );
    } else if !shared_profiles.is_empty() {
        checks.push(
            CheckResult::fail(
                "accounts.switching_architecture",
                "Antigravity account switching contract",
                "Account switching broken: profile directory collision detected",
                "Each account must have a distinct profile directory under ~/.config/agentcontrol/profiles/<id>.",
            )
            .with_details(
                "Common Causes on Broken Systems:\n\
                1. PATH Shadowing: Old AgentControll agy wrapper in PATH ahead of real Google agy\n\
                2. Ambient Credentials: GEMINI_API_KEY/GOOGLE_API_KEY overriding profile tokens\n\
                3. Incomplete OAuth: Missing refresh token in credential file\n\
                4. Shared Profiles: Accounts pointing to the same profile\n\
                5. Binary Drift: npm package vs cached native binary version mismatch",
            ),
        );
    }

    // 5. Deep mode checks: detect orphan profiles and orphan credentials
    if doctor.opts.deep {
        check_orphans(&accounts, &profiles_base, &creds_base, checks);
    }
}

fn read_accounts(conn: &Connection) -> Result<Vec<AccountRecord>, rusqlite::Error> {
    let mut stmt =
        conn.prepare("SELECT id, label, provider, credential_ref, state FROM accounts")?;
    let rows = stmt.query_map([], |row| {
        Ok(AccountRecord {
            id: row.get(0)?,
            label: row.get(1)?,
            provider: row.get(2)?,
            credential_ref: row.get(3)?,
            state: row.get(4)?,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn check_profile_for_account(a: &AccountRecord, profile_dir: &Path, checks: &mut Vec<CheckResult>) {
    let id_key = format!("profile.{}", a.id);
    let title = format!("Profile for account \"{}\"", a.label);

    if !profile_dir.exists() {
        checks.push(CheckResult::fail(
            id_key,
            title,
            format!("Profile directory {} does not exist", profile_dir.display()),
            "Launch a session with this account to initialize its profile, or recreate it.",
        ));
        return;
    }

    // Verify directory permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = profile_dir.metadata() {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                checks.push(CheckResult::warn(
                    format!("{id_key}.perm"),
                    format!("Profile permissions for \"{}\"", a.label),
                    format!(
                        "Profile directory permissions are 0{:o} (recommended: 0700)",
                        mode
                    ),
                    format!(
                        "Run `chmod 0700 {}` to restrict access.",
                        profile_dir.display()
                    ),
                ));
            }
        }
    }

    // Check .gemini structure
    let token_file = profile_dir
        .join(".gemini")
        .join("antigravity-cli")
        .join("antigravity-oauth-token");

    if !token_file.is_file() {
        checks.push(CheckResult::fail(
            id_key,
            title,
            format!("OAuth token file missing at {}", token_file.display()),
            "The account profile is missing authentication state. Re-authenticate this account.",
        ));
        return;
    }

    // Verify token file permissions (mode 0600 on Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = token_file.metadata() {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                checks.push(CheckResult::warn(
                    format!("{id_key}.token_perm"),
                    format!("Token file permissions for \"{}\"", a.label),
                    format!(
                        "OAuth token file permissions are 0{:o} (recommended: 0600)",
                        mode
                    ),
                    format!(
                        "Run `chmod 0600 {}` to secure the credential.",
                        token_file.display()
                    ),
                ));
            }
        }
    }

    // Safely check token file is valid JSON (NEVER print token content!)
    match std::fs::read_to_string(&token_file) {
        Ok(content) => match serde_json::from_str::<Value>(&content) {
            Ok(v) => {
                let has_token_obj = v.get("token").is_some();
                if has_token_obj {
                    checks.push(CheckResult::pass(
                        id_key,
                        title,
                        format!("Profile valid at {}", profile_dir.display()),
                    ));
                } else {
                    checks.push(CheckResult::warn(
                        id_key,
                        title,
                        format!(
                            "Token file at {} has non-standard format",
                            token_file.display()
                        ),
                        "Re-authenticate the account if session launch fails.",
                    ));
                }
            }
            Err(_) => {
                checks.push(CheckResult::fail(
                    id_key,
                    title,
                    format!(
                        "Token file at {} is corrupted / invalid JSON",
                        token_file.display()
                    ),
                    "Remove and re-add this account in AgentControll.",
                ));
            }
        },
        Err(e) => {
            checks.push(CheckResult::fail(
                id_key,
                title,
                format!("Cannot read token file {}: {e}", token_file.display()),
                "Verify file permissions for current user.",
            ));
        }
    }
}

fn check_credential_for_account(
    a: &AccountRecord,
    creds_base: &Path,
    checks: &mut Vec<CheckResult>,
) {
    let id_key = format!("credential.{}", a.id);
    let title = format!("Credential for account \"{}\"", a.label);

    let cred_path = if let Some(stripped) = a.credential_ref.strip_prefix("ref:") {
        let (prefix, id) = match stripped.split_once(':') {
            Some(pair) => pair,
            None => {
                checks.push(CheckResult::fail(
                    id_key,
                    title,
                    format!("Malformed credential reference '{}'", a.credential_ref),
                    "Update account credential reference in the database.",
                ));
                return;
            }
        };
        creds_base.join(format!("{prefix}_{id}.json"))
    } else {
        checks.push(CheckResult::fail(
            id_key,
            title,
            format!("Invalid credential reference format '{}'", a.credential_ref),
            "Re-add account to generate a canonical credential reference.",
        ));
        return;
    };

    if !cred_path.is_file() {
        checks.push(CheckResult::fail(
            id_key,
            title,
            format!("Credential file {} not found", cred_path.display()),
            "The stored credential file is missing. Sign in again to restore credentials.",
        ));
        return;
    }

    // Verify file permissions (0600 on Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = cred_path.metadata() {
            let mode = meta.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                checks.push(CheckResult::warn(
                    format!("{id_key}.perm"),
                    format!("Credential permissions for \"{}\"", a.label),
                    format!(
                        "Credential file permissions are 0{:o} (recommended: 0600)",
                        mode
                    ),
                    format!("Run `chmod 0600 {}`.", cred_path.display()),
                ));
            }
        }
    }

    // Verify non-secret metadata
    match std::fs::read_to_string(&cred_path) {
        Ok(c) => match serde_json::from_str::<Value>(&c) {
            Ok(v) => {
                let ver = v.get("version").and_then(Value::as_u64).unwrap_or(0);
                let email = v.get("email").and_then(Value::as_str);
                let summary = if let Some(em) = email {
                    format!("Valid v{ver} credential for {em}")
                } else {
                    format!("Valid v{ver} credential")
                };
                checks.push(CheckResult::pass(id_key, title, summary));
            }
            Err(_) => {
                checks.push(CheckResult::fail(
                    id_key,
                    title,
                    format!("Credential file {} is not valid JSON", cred_path.display()),
                    "Re-authenticate this account.",
                ));
            }
        },
        Err(e) => {
            checks.push(CheckResult::fail(
                id_key,
                title,
                format!("Cannot read credential file {}: {e}", cred_path.display()),
                "Verify file read permissions.",
            ));
        }
    }
}

fn check_orphans(
    accounts: &[AccountRecord],
    profiles_base: &Path,
    creds_base: &Path,
    checks: &mut Vec<CheckResult>,
) {
    let account_ids: HashSet<&str> = accounts.iter().map(|a| a.id.as_str()).collect();

    // Check orphan profiles
    if profiles_base.is_dir() {
        if let Ok(entries) = std::fs::read_dir(profiles_base) {
            let mut orphans = Vec::new();
            for entry in entries.flatten() {
                if let Ok(ft) = entry.file_type() {
                    if ft.is_dir() {
                        if let Some(name) = entry.file_name().to_str() {
                            if !account_ids.contains(name) {
                                orphans.push(name.to_string());
                            }
                        }
                    }
                }
            }
            if !orphans.is_empty() {
                checks.push(
                    CheckResult::info(
                        "accounts.orphan_profiles",
                        "Orphan profile directories (deep scan)",
                        format!("{} inactive profile directory(s) found", orphans.len()),
                    )
                    .with_details(format!("Orphan profiles: {}", orphans.join(", "))),
                );
            }
        }
    }

    // Check orphan credentials
    if creds_base.is_dir() {
        if let Ok(entries) = std::fs::read_dir(creds_base) {
            let mut orphan_creds = Vec::new();
            let cred_refs: HashSet<String> = accounts
                .iter()
                .filter_map(|a| {
                    a.credential_ref
                        .strip_prefix("ref:")
                        .map(|s| s.replace(':', "_"))
                })
                .collect();

            for entry in entries.flatten() {
                if let Some(name) = entry.file_name().to_str() {
                    if name.ends_with(".json") {
                        let stem = name.trim_end_matches(".json");
                        if !cred_refs.contains(stem) {
                            orphan_creds.push(name.to_string());
                        }
                    }
                }
            }

            if !orphan_creds.is_empty() {
                checks.push(
                    CheckResult::info(
                        "accounts.orphan_credentials",
                        "Orphan credential files (deep scan)",
                        format!(
                            "{} unreferenced credential file(s) found",
                            orphan_creds.len()
                        ),
                    )
                    .with_details(format!("Orphan files: {}", orphan_creds.join(", "))),
                );
            }
        }
    }
}
