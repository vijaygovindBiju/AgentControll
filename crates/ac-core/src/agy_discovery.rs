//! Antigravity conversation discovery and profile synchronization.
//!
//! Discovers existing Antigravity (`agy`) conversations created both inside
//! Agent Control account profiles and externally through the host `agy` CLI
//! (`~/.gemini/antigravity-cli/`).
//!
//! External conversations are indexed without modifying Antigravity's original
//! database records. Resumption utilizes Antigravity's supported CLI flag
//! `--conversation <id>` while maintaining account profile and workspace isolation.

use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use anyhow::{bail, Context, Result};
use rusqlite::{types::Value, Connection, OpenFlags};
use serde::{Deserialize, Serialize};

use crate::types::Id;

/// Represents a discovered conversation from Antigravity storage.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredConversation {
    /// Canonical Antigravity conversation identifier (UUID string).
    pub conversation_id: String,
    /// User-visible title (if empty, preview or default is used).
    pub title: String,
    /// Conversation preview text snippet.
    pub preview: String,
    /// Total interaction step count in the conversation trajectory.
    pub step_count: u32,
    /// ISO 8601 / RFC 3339 timestamp of last modification.
    pub last_modified_time: String,
    /// Associated workspace directory (if resolved and existing on disk).
    pub workspace_path: Option<String>,
    /// Whether this conversation originated outside Agent Control.
    pub is_external: bool,
    /// Associated account ID (if discovered from an Agent Control profile).
    pub account_id: Option<Id>,
    /// Path to the underlying conversation database file.
    pub conversation_db_path: PathBuf,
}

/// Fallback or default home directory.
fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
}

/// Default host `~/.gemini` directory.
pub fn default_host_gemini_dir() -> PathBuf {
    home_dir().join(".gemini")
}

/// Decode a `file:///...` URI string to a normalised absolute `PathBuf`.
pub fn parse_workspace_uri(uri: &str) -> Option<PathBuf> {
    let trimmed = uri.trim();
    if trimmed.is_empty() {
        return None;
    }

    let path_str = if let Some(rest) = trimmed.strip_prefix("file://") {
        rest
    } else if trimmed.starts_with('/') {
        trimmed
    } else {
        return None;
    };

    let decoded = percent_decode_path(path_str);
    let p = PathBuf::from(decoded);
    if p.is_absolute() {
        Some(p)
    } else {
        None
    }
}

/// Helper to percent-decode URL path components.
/// Strictly decodes `%XX` hex sequences while preserving `+` and literal path chars.
pub fn percent_decode_path(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or(""), 16)
            {
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Validate that a conversation `.db` file exists, is non-empty, and has a SQLite header.
pub fn validate_conversation_db(path: &Path) -> Result<()> {
    if !path.is_file() {
        bail!("Conversation database does not exist: {}", path.display());
    }
    let metadata =
        fs::metadata(path).with_context(|| format!("Reading metadata for {}", path.display()))?;
    if metadata.len() < 100 {
        bail!(
            "Conversation database is empty or truncated (size={} bytes): {}",
            metadata.len(),
            path.display()
        );
    }

    // Check first 16 bytes for standard SQLite 3 header: "SQLite format 3\0"
    let mut header = [0u8; 16];
    let mut file = fs::File::open(path)
        .with_context(|| format!("Opening conversation db: {}", path.display()))?;
    use std::io::Read;
    if file.read_exact(&mut header).is_err() || &header != b"SQLite format 3\0" {
        bail!(
            "Conversation database is corrupted or invalid SQLite: {}",
            path.display()
        );
    }

    Ok(())
}

/// Read conversation records from a specific `conversation_summaries.db` file.
/// Never mutates the source database; uses read-only SQLite access.
pub fn read_conversations_from_db(
    db_path: &Path,
    conversations_dir: &Path,
    is_external: bool,
    account_id: Option<Id>,
) -> Result<Vec<DiscoveredConversation>> {
    if !db_path.is_file() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .with_context(|| format!("Opening read-only SQLite database at {}", db_path.display()))?;

    // Check if the expected table exists
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'conversation_summaries'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);

    if !table_exists {
        return Ok(Vec::new());
    }

    let mut stmt = conn.prepare(
        "SELECT conversation_id, title, preview, step_count, last_modified_time, workspace_uris
         FROM conversation_summaries
         ORDER BY last_modified_time DESC",
    )?;

    let rows = stmt.query_map([], |row| {
        let cid: String = row.get(0)?;
        let title: String = row.get(1).unwrap_or_default();
        let preview: String = row.get(2).unwrap_or_default();
        let step_count: u32 = row.get(3).unwrap_or(0);
        let last_modified_time: String = row.get(4).unwrap_or_default();
        let workspace_uris: String = row.get(5).unwrap_or_default();
        Ok((
            cid,
            title,
            preview,
            step_count,
            last_modified_time,
            workspace_uris,
        ))
    })?;

    let mut results = Vec::new();
    for item in rows.flatten() {
        let (cid, title, preview, step_count, last_modified_time, workspace_uris) = item;
        let cid = cid.trim().to_string();
        if cid.is_empty() {
            continue;
        }

        // Verify that the conversation trajectory db actually exists on disk and is valid
        let conv_file = conversations_dir.join(format!("{cid}.db"));
        if !conv_file.is_file() || validate_conversation_db(&conv_file).is_err() {
            tracing::debug!(
                "Skipping conversation {} because db file does not exist or is invalid: {}",
                cid,
                conv_file.display()
            );
            continue;
        }

        // Resolve workspace path from JSON array in workspace_uris
        let mut resolved_workspace = None;
        if let Ok(uris) = serde_json::from_str::<Vec<String>>(&workspace_uris) {
            for u in uris {
                if let Some(p) = parse_workspace_uri(&u) {
                    resolved_workspace = Some(p.to_string_lossy().to_string());
                    break;
                }
            }
        }

        results.push(DiscoveredConversation {
            conversation_id: cid,
            title,
            preview,
            step_count,
            last_modified_time,
            workspace_path: resolved_workspace,
            is_external,
            account_id: account_id.clone(),
            conversation_db_path: conv_file,
        });
    }

    Ok(results)
}

/// Discover conversations created in the host Antigravity directory outside Agent Control.
pub fn discover_external_conversations(
    host_gemini: Option<&Path>,
) -> Result<Vec<DiscoveredConversation>> {
    let base = host_gemini
        .map(PathBuf::from)
        .unwrap_or_else(default_host_gemini_dir);
    let agy_dir = base.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    read_conversations_from_db(&summaries_db, &conv_dir, true, None)
}

/// Discover conversations stored in Agent Control isolated account profiles.
pub fn discover_profile_conversations(
    profiles_base: Option<&Path>,
) -> Result<Vec<DiscoveredConversation>> {
    let base = profiles_base
        .map(PathBuf::from)
        .unwrap_or_else(crate::agy_auth::profiles_dir);

    if !base.is_dir() {
        return Ok(Vec::new());
    }

    let entries =
        fs::read_dir(&base).with_context(|| format!("Reading profiles at {}", base.display()))?;
    let mut all = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.is_empty() || dir_name.starts_with('.') {
            continue;
        }
        let account_id = Id(dir_name.to_string());
        let agy_dir = path.join(".gemini").join("antigravity-cli");
        let summaries_db = agy_dir.join("conversation_summaries.db");
        let conv_dir = agy_dir.join("conversations");

        if let Ok(convos) =
            read_conversations_from_db(&summaries_db, &conv_dir, false, Some(account_id))
        {
            all.extend(convos);
        }
    }

    Ok(all)
}

/// Discover all conversations (both profile and external), avoiding duplicates.
pub fn discover_all_conversations(
    host_gemini: Option<&Path>,
    profiles_base: Option<&Path>,
) -> Result<Vec<DiscoveredConversation>> {
    let mut seen = HashSet::new();
    let mut combined = Vec::new();

    // 1. Profile conversations take priority (already associated with specific accounts)
    if let Ok(profile_convos) = discover_profile_conversations(profiles_base) {
        for c in profile_convos {
            if seen.insert(c.conversation_id.clone()) {
                combined.push(c);
            }
        }
    }

    // 2. External conversations
    if let Ok(ext_convos) = discover_external_conversations(host_gemini) {
        for c in ext_convos {
            if seen.insert(c.conversation_id.clone()) {
                combined.push(c);
            }
        }
    }

    // Sort by last_modified_time descending (newest first)
    combined.sort_by(|a, b| b.last_modified_time.cmp(&a.last_modified_time));

    Ok(combined)
}

/// Find the source conversation trajectory file across host and profile storage.
pub fn find_conversation_file(
    conversation_id: &str,
    host_gemini: Option<&Path>,
    profiles_base: Option<&Path>,
) -> Option<PathBuf> {
    let cid = conversation_id.trim();
    if cid.is_empty() {
        return None;
    }

    // 1. Check host storage
    let host_base = host_gemini
        .map(PathBuf::from)
        .unwrap_or_else(default_host_gemini_dir);
    let host_file = host_base
        .join("antigravity-cli")
        .join("conversations")
        .join(format!("{cid}.db"));
    if host_file.is_file() && validate_conversation_db(&host_file).is_ok() {
        return Some(host_file);
    }

    // 2. Check profile storage
    let prof_base = profiles_base
        .map(PathBuf::from)
        .unwrap_or_else(crate::agy_auth::profiles_dir);
    if let Ok(entries) = fs::read_dir(&prof_base) {
        for entry in entries.flatten() {
            let candidate = entry
                .path()
                .join(".gemini")
                .join("antigravity-cli")
                .join("conversations")
                .join(format!("{cid}.db"));
            if candidate.is_file() && validate_conversation_db(&candidate).is_ok() {
                return Some(candidate);
            }
        }
    }

    None
}

/// Copy the selected conversation's summary row into the isolated profile.
/// Antigravity resolves `--conversation` through this index as well as the
/// trajectory database, so linking only `<id>.db` is insufficient.
fn sync_conversation_summary(
    source_db: &Path,
    target_db: &Path,
    conversation_id: &str,
) -> Result<()> {
    if !source_db.is_file() {
        bail!(
            "Conversation summary database does not exist: {}",
            source_db.display()
        );
    }
    if let Some(parent) = target_db.parent() {
        fs::create_dir_all(parent)?;
    }
    let source = Connection::open_with_flags(
        source_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )?;
    let create_sql: String = source
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='conversation_summaries'",
            [],
            |row| row.get(0),
        )
        .context("Host conversation summary table is missing")?;
    let source_columns = table_columns(&source, "conversation_summaries")?;
    if !source_columns
        .iter()
        .any(|column| column == "conversation_id")
    {
        bail!("Conversation summary table has no conversation_id column");
    }
    let source_values: Vec<Value> = {
        let sql = format!("SELECT * FROM conversation_summaries WHERE conversation_id = ?1");
        let mut statement = source.prepare(&sql)?;
        let count = statement.column_count();
        let mut rows = statement.query([conversation_id])?;
        let Some(row) = rows.next()? else {
            bail!("Conversation {conversation_id} is missing from host summary index");
        };
        (0..count)
            .map(|index| row.get(index))
            .collect::<rusqlite::Result<_>>()?
    };

    let target = Connection::open(target_db)?;
    let target_table_exists: bool = target.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='conversation_summaries')",
        [],
        |row| row.get(0),
    )?;
    if !target_table_exists {
        target.execute_batch(&create_sql)?;
    }
    let target_columns = table_columns(&target, "conversation_summaries")?;
    let columns: Vec<String> = source_columns
        .iter()
        .filter(|column| target_columns.contains(column))
        .cloned()
        .collect();
    if !columns.iter().any(|column| column == "conversation_id") {
        bail!("Profile conversation summary table has no conversation_id column");
    }
    let indexes: Vec<usize> = columns
        .iter()
        .map(|column| {
            source_columns
                .iter()
                .position(|source| source == column)
                .unwrap()
        })
        .collect();
    let quoted = columns
        .iter()
        .map(|column| format!("\"{}\"", column.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(", ");
    let placeholders = (1..=columns.len())
        .map(|index| format!("?{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let sql =
        format!("INSERT OR IGNORE INTO conversation_summaries ({quoted}) VALUES ({placeholders})");
    let values: Vec<Value> = indexes
        .iter()
        .map(|index| source_values[*index].clone())
        .collect();
    target.execute(&sql, rusqlite::params_from_iter(values))?;
    Ok(())
}

fn table_columns(connection: &Connection, table: &str) -> Result<Vec<String>> {
    let mut statement = connection.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    let rows = statement.query_map([], |row| row.get(1))?;
    Ok(rows.collect::<rusqlite::Result<Vec<String>>>()?)
}

/// Ensure a conversation file is accessible inside the given account profile.
///
/// If the file is not already in the profile's `.gemini/antigravity-cli/conversations/`
/// directory, it locates the external conversation file from host storage and symlinks it
/// into the profile so `agy --conversation <id>` can open and resume it natively.
/// Conversations belonging to other account profiles are strictly rejected to enforce
/// account profile isolation.
#[cfg(unix)]
pub fn link_conversation_to_profile(
    account_id: &Id,
    conversation_id: &str,
    host_gemini: Option<&Path>,
) -> Result<PathBuf> {
    let home = crate::agy_auth::profile_dir(account_id)
        .map_err(|e| anyhow::anyhow!("Profile directory error: {e}"))?;
    let target_dir = home
        .join(".gemini")
        .join("antigravity-cli")
        .join("conversations");
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("Creating conversations dir at {}", target_dir.display()))?;

    let target_file = target_dir.join(format!("{conversation_id}.db"));
    let host_base = host_gemini
        .map(PathBuf::from)
        .unwrap_or_else(default_host_gemini_dir);
    let host_agy_dir = host_base.join("antigravity-cli");
    let host_file = host_agy_dir
        .join("conversations")
        .join(format!("{conversation_id}.db"));
    let profile_agy_dir = home.join(".gemini").join("antigravity-cli");
    if target_file.is_file() {
        validate_conversation_db(&target_file)?;
        if host_file.is_file() {
            sync_conversation_summary(
                &host_agy_dir.join("conversation_summaries.db"),
                &profile_agy_dir.join("conversation_summaries.db"),
                conversation_id,
            )?;
        }
        return Ok(target_file);
    }

    // 1. Check external host storage
    let source_file = if host_file.is_file() && validate_conversation_db(&host_file).is_ok() {
        host_file.clone()
    } else {
        // Check if it belongs to another account profile (account isolation violation)
        let prof_base = crate::agy_auth::profiles_dir();
        let mut in_other_profile = false;
        if let Ok(entries) = fs::read_dir(&prof_base) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p != home {
                    let candidate = p
                        .join(".gemini")
                        .join("antigravity-cli")
                        .join("conversations")
                        .join(format!("{conversation_id}.db"));
                    if candidate.is_file() && validate_conversation_db(&candidate).is_ok() {
                        in_other_profile = true;
                        break;
                    }
                }
            }
        }

        if in_other_profile {
            bail!(
                "Conversation '{}' belongs to another account profile; cannot resume across account boundaries",
                conversation_id
            );
        }

        bail!(
            "Conversation '{}' not found in external Antigravity storage",
            conversation_id
        );
    };

    validate_conversation_db(&source_file)?;

    // Create symlink into the profile
    if target_file.symlink_metadata().is_ok() {
        let _ = fs::remove_file(&target_file);
    }
    std::os::unix::fs::symlink(&source_file, &target_file).with_context(|| {
        format!(
            "Symlinking conversation {} -> {}",
            source_file.display(),
            target_file.display()
        )
    })?;

    if source_file == host_file {
        if let Err(error) = sync_conversation_summary(
            &host_agy_dir.join("conversation_summaries.db"),
            &profile_agy_dir.join("conversation_summaries.db"),
            conversation_id,
        ) {
            let _ = fs::remove_file(&target_file);
            return Err(error).context("Importing external conversation into profile index");
        }
    }

    tracing::info!(
        "Linked external conversation {} into profile {}",
        conversation_id,
        account_id.0
    );

    Ok(target_file)
}

#[cfg(not(unix))]
pub fn link_conversation_to_profile(
    account_id: &Id,
    conversation_id: &str,
    host_gemini: Option<&Path>,
) -> Result<PathBuf> {
    let home = crate::agy_auth::profile_dir(account_id)
        .map_err(|e| anyhow::anyhow!("Profile directory error: {e}"))?;
    let target_dir = home
        .join(".gemini")
        .join("antigravity-cli")
        .join("conversations");
    fs::create_dir_all(&target_dir)
        .with_context(|| format!("Creating conversations dir at {}", target_dir.display()))?;

    let target_file = target_dir.join(format!("{conversation_id}.db"));
    if target_file.is_file() {
        validate_conversation_db(&target_file)?;
        return Ok(target_file);
    }

    let host_base = host_gemini
        .map(PathBuf::from)
        .unwrap_or_else(default_host_gemini_dir);
    let host_file = host_base
        .join("antigravity-cli")
        .join("conversations")
        .join(format!("{conversation_id}.db"));

    if !host_file.is_file() {
        bail!(
            "Conversation '{}' not found in external Antigravity storage",
            conversation_id
        );
    }
    validate_conversation_db(&host_file)?;

    fs::copy(&host_file, &target_file).with_context(|| {
        format!(
            "Copying conversation {} -> {}",
            host_file.display(),
            target_file.display()
        )
    })?;

    let host_agy_dir = host_base.join("antigravity-cli");
    let profile_agy_dir = home.join(".gemini").join("antigravity-cli");
    sync_conversation_summary(
        &host_agy_dir.join("conversation_summaries.db"),
        &profile_agy_dir.join("conversation_summaries.db"),
        conversation_id,
    )?;

    Ok(target_file)
}
