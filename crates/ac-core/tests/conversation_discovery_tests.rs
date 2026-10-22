//! Automated tests for Antigravity external conversation discovery and resumption.
//!
//! Covers:
//! 1. Discover conversation inside AC (profile storage)
//! 2. Discover conversation outside AC (host storage)
//! 3. Resume externally created conversation (linking, flags, prompt suppression)
//! 4. Restart AC and rediscover
//! 5. Prevent duplicate entries
//! 6. Handle unknown conversation IDs
//! 7. Handle inaccessible / deleted / corrupted conversations
//! 8. Preserve account & workspace isolation
//! 9. Verify failed resume does not silently launch replacement
//! 10. Session lifecycle compatibility

use std::{
    fs,
    path::{Path, PathBuf},
};

use ac_core::{
    adapter::CompositeAdapterFactory,
    agy_discovery::{
        discover_external_conversations, discover_profile_conversations, find_conversation_file,
        link_conversation_to_profile, parse_workspace_uri, validate_conversation_db,
    },
    agy_launch::AgyLaunchOptions,
    event_store::EventStore,
    session::{
        manager::{SessionManager, SessionManagerHandle},
        state_machine,
    },
    types::{AgentSession, Id, SessionState},
};
use rusqlite::Connection;
use tempfile::TempDir;

/// Helper to create a valid SQLite conversation database file.
fn create_valid_conv_db(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let conn = Connection::open(path).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS messages (id INTEGER PRIMARY KEY, role TEXT, content TEXT)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO messages (role, content) VALUES ('user', 'Hello Antigravity')",
        [],
    )
    .unwrap();
}

/// Helper to create a mock `conversation_summaries.db` with sample records.
fn create_mock_summaries(
    db_path: &Path,
    conv_id: &str,
    title: &str,
    preview: &str,
    workspace_dir: Option<&Path>,
) {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let conn = Connection::open(db_path).unwrap();
    conn.execute(
        "CREATE TABLE IF NOT EXISTS conversation_summaries (
            conversation_id TEXT PRIMARY KEY,
            title TEXT,
            preview TEXT,
            step_count INTEGER,
            last_modified_time TEXT,
            workspace_uris TEXT
        )",
        [],
    )
    .unwrap();

    let uris_json = if let Some(ws) = workspace_dir {
        let uri = format!("file://{}", ws.display());
        serde_json::to_string(&vec![uri]).unwrap()
    } else {
        "[]".to_string()
    };

    conn.execute(
        "INSERT OR REPLACE INTO conversation_summaries
         (conversation_id, title, preview, step_count, last_modified_time, workspace_uris)
         VALUES (?1, ?2, ?3, 7, '2026-10-22T08:30:00Z', ?4)",
        rusqlite::params![conv_id, title, preview, uris_json],
    )
    .unwrap();
}

fn create_spawned_manager(
    store: EventStore,
    host_gemini: Option<PathBuf>,
    profiles_base: Option<PathBuf>,
) -> SessionManagerHandle {
    let (cmd_tx, cmd_rx) = tokio::sync::mpsc::channel(64);
    let (event_tx, _event_rx) = tokio::sync::broadcast::channel(128);
    let factory = Box::new(CompositeAdapterFactory::new());
    let mut manager = SessionManager::new(store, cmd_rx, event_tx, 3, factory)
        .with_gemini_dirs(host_gemini, profiles_base);

    // Initial discovery
    let _ = manager.discover_conversations();

    tokio::spawn(async move {
        manager.run().await;
    });

    SessionManagerHandle::new(cmd_tx)
}

#[test]
fn test_1_discover_inside_ac_profile() {
    let tmp = TempDir::new().unwrap();
    let profiles_base = tmp.path().join("profiles");
    let account_id = Id("acct-discovery-1".to_string());
    let profile_dir = profiles_base.join(&account_id.0);
    let agy_dir = profile_dir.join(".gemini").join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "11111111-2222-3333-4444-555555555555";
    create_valid_conv_db(&conv_dir.join(format!("{conv_id}.db")));
    let ws_dir = tmp.path().join("workspace");
    fs::create_dir_all(&ws_dir).unwrap();
    create_mock_summaries(
        &summaries_db,
        conv_id,
        "Profile Task",
        "Preview of internal conversation",
        Some(&ws_dir),
    );

    let discovered = discover_profile_conversations(Some(&profiles_base)).unwrap();
    assert_eq!(discovered.len(), 1);
    let item = &discovered[0];
    assert_eq!(item.conversation_id, conv_id);
    assert_eq!(item.title, "Profile Task");
    assert_eq!(item.preview, "Preview of internal conversation");
    assert_eq!(item.step_count, 7);
    assert_eq!(item.is_external, false);
    assert_eq!(item.account_id, Some(account_id));
    assert_eq!(item.workspace_path, Some(ws_dir.display().to_string()));
}

#[test]
fn test_2_discover_outside_ac_host() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    create_valid_conv_db(&conv_dir.join(format!("{conv_id}.db")));
    let ws_dir = tmp.path().join("ext_workspace");
    fs::create_dir_all(&ws_dir).unwrap();
    create_mock_summaries(
        &summaries_db,
        conv_id,
        "External CLI Run",
        "Preview of standalone agy run",
        Some(&ws_dir),
    );

    let discovered = discover_external_conversations(Some(&host_gemini)).unwrap();
    assert_eq!(discovered.len(), 1);
    let item = &discovered[0];
    assert_eq!(item.conversation_id, conv_id);
    assert_eq!(item.title, "External CLI Run");
    assert_eq!(item.is_external, true);
    assert_eq!(item.account_id, None);
    assert_eq!(item.workspace_path, Some(ws_dir.display().to_string()));
}

#[test]
fn test_3_resume_externally_created_conversation_linking_and_flags() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "cccccccc-dddd-eeee-ffff-000000000000";
    let source_file = conv_dir.join(format!("{conv_id}.db"));
    create_valid_conv_db(&source_file);
    let host_summaries = agy_dir.join("conversation_summaries.db");
    create_mock_summaries(
        &host_summaries,
        conv_id,
        "Outside conversation",
        "Existing work from host agy",
        None,
    );

    // Set custom config dir so link_conversation_to_profile knows where profile dir lives
    let cfg_dir = tmp.path().join("agentcontrol_config");
    std::env::set_var("AGENTCONTROL_CONFIG_DIR", &cfg_dir);

    let account_id = Id("test-acct-resume".to_string());
    let linked = link_conversation_to_profile(&account_id, conv_id, Some(&host_gemini)).unwrap();
    assert!(linked.is_file());
    #[cfg(unix)]
    assert!(linked.symlink_metadata().unwrap().file_type().is_symlink());

    // Verify original external db was NOT modified
    validate_conversation_db(&source_file).unwrap();

    // Antigravity needs the summary index entry under the isolated HOME too.
    let profile_summaries = linked
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("conversation_summaries.db");
    let profile_conn = Connection::open(&profile_summaries).unwrap();
    let imported_title: String = profile_conn
        .query_row(
            "SELECT title FROM conversation_summaries WHERE conversation_id = ?1",
            [conv_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(imported_title, "Outside conversation");
    let host_conn = Connection::open(&host_summaries).unwrap();
    let host_count: i64 = host_conn
        .query_row("SELECT COUNT(*) FROM conversation_summaries", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(host_count, 1, "host summary index remains intact");

    // Verify launch options CLI builder emits --conversation flag
    let mut opts = AgyLaunchOptions::default();
    opts.conversation_id = Some(conv_id.to_string());
    opts.working_dir = Some("/tmp/work".to_string());
    let args = opts.args();
    assert!(args.contains(&"--conversation".to_string()));
    assert!(args.contains(&conv_id.to_string()));
}

#[tokio::test]
async fn test_4_restart_and_rediscover() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "dddddddd-eeee-ffff-1111-222222222222";
    create_valid_conv_db(&conv_dir.join(format!("{conv_id}.db")));
    let ws_dir = tmp.path().join("restart_ws");
    fs::create_dir_all(&ws_dir).unwrap();
    create_mock_summaries(
        &summaries_db,
        conv_id,
        "Rediscover After Restart",
        "Preview restart",
        Some(&ws_dir),
    );

    let db_path = tmp.path().join("events.db");
    let event_store = EventStore::open(&db_path).unwrap();
    let empty_profiles = tmp.path().join("empty_profiles");

    // 1st daemon session manager run
    let handle1 = create_spawned_manager(
        event_store,
        Some(host_gemini.clone()),
        Some(empty_profiles.clone()),
    );
    let sessions1 = handle1.list().await.unwrap();
    assert_eq!(sessions1.len(), 1);
    assert_eq!(sessions1[0].id.0, conv_id);
    assert_eq!(sessions1[0].is_external, true);
    assert_eq!(sessions1[0].state, SessionState::Idle);

    // Simulate daemon restart: 2nd session manager run with same event_store & gemini dir
    let event_store2 = EventStore::open(&db_path).unwrap();
    let handle2 = create_spawned_manager(event_store2, Some(host_gemini), Some(empty_profiles));
    let sessions2 = handle2.list().await.unwrap();
    assert_eq!(sessions2.len(), 1);
    assert_eq!(sessions2[0].id.0, conv_id);
    assert_eq!(sessions2[0].is_external, true);
    assert_eq!(sessions2[0].state, SessionState::Idle);
}

#[tokio::test]
async fn test_5_prevent_duplicate_entries() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "eeeeeeee-ffff-0000-1111-222222222222";
    create_valid_conv_db(&conv_dir.join(format!("{conv_id}.db")));
    create_mock_summaries(&summaries_db, conv_id, "Dedup Test", "Dedup preview", None);

    let db_path = tmp.path().join("events.db");
    let event_store = EventStore::open(&db_path).unwrap();
    let empty_profiles = tmp.path().join("empty_profiles");
    let handle = create_spawned_manager(event_store, Some(host_gemini), Some(empty_profiles));

    // Run discovery multiple times via handle
    handle.discover().await.unwrap();
    handle.discover().await.unwrap();
    let sessions = handle.list().await.unwrap();

    let matching: Vec<_> = sessions.into_iter().filter(|s| s.id.0 == conv_id).collect();
    assert_eq!(
        matching.len(),
        1,
        "Must not create duplicate session entries"
    );
}

#[test]
fn test_6_handle_unknown_conversation_ids() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");

    let unknown_id = "99999999-9999-9999-9999-999999999999";
    let found = find_conversation_file(unknown_id, Some(&host_gemini), None);
    assert!(found.is_none(), "Unknown conversation ID must return None");

    // Preflight linking unknown conversation must fail cleanly
    let account_id = Id("fake-acct".to_string());
    let res = link_conversation_to_profile(&account_id, unknown_id, Some(&host_gemini));
    assert!(res.is_err(), "Linking unknown conversation must error");
}

#[test]
fn test_7_handle_inaccessible_or_deleted_or_corrupted_conversations() {
    let tmp = TempDir::new().unwrap();
    let conv_file = tmp.path().join("bad.db");

    // 1. Non-existent file
    assert!(validate_conversation_db(&conv_file).is_err());

    // 2. Empty / truncated file (< 100 bytes)
    fs::write(&conv_file, b"short").unwrap();
    assert!(validate_conversation_db(&conv_file).is_err());

    // 3. Corrupted / non-SQLite header
    let junk = vec![0x42; 256];
    fs::write(&conv_file, &junk).unwrap();
    let err = validate_conversation_db(&conv_file).unwrap_err();
    assert!(err.to_string().contains("corrupted or invalid SQLite"));

    // 4. Valid file passes
    let _ = fs::remove_file(&conv_file);
    create_valid_conv_db(&conv_file);
    assert!(validate_conversation_db(&conv_file).is_ok());
}

#[test]
fn test_8_preserve_account_and_workspace_isolation() {
    // 1. Workspace URI parsing with percent-encoding and literal '+' preservation
    let uri = "file:///home/user/my%20project%20dir";
    let decoded = parse_workspace_uri(uri).unwrap();
    assert_eq!(decoded, PathBuf::from("/home/user/my project dir"));

    let plus_uri = "file:///home/user/c%2B%2B_repo/src+main";
    let decoded_plus = parse_workspace_uri(plus_uri).unwrap();
    assert_eq!(decoded_plus, PathBuf::from("/home/user/c++_repo/src+main"));

    // 2. Invalid URI schemes rejected
    assert!(parse_workspace_uri("http://example.com").is_none());
    assert!(parse_workspace_uri("").is_none());

    // 3. Account profile separation & cross-account linking rejection
    let tmp = TempDir::new().unwrap();
    let cfg_dir = tmp.path().join("config");
    std::env::set_var("AGENTCONTROL_CONFIG_DIR", &cfg_dir);

    let acct_a = Id("account-A".to_string());
    let acct_b = Id("account-B".to_string());
    let dir_a = ac_core::agy_auth::profile_dir(&acct_a).unwrap();
    let dir_b = ac_core::agy_auth::profile_dir(&acct_b).unwrap();
    assert_ne!(
        dir_a, dir_b,
        "Profiles must be strictly isolated directories"
    );

    // Put a conversation in Account B's profile
    let b_conv_dir = dir_b
        .join(".gemini")
        .join("antigravity-cli")
        .join("conversations");
    fs::create_dir_all(&b_conv_dir).unwrap();
    let b_cid = "bbbbbbbb-1111-2222-3333-444444444444";
    create_valid_conv_db(&b_conv_dir.join(format!("{b_cid}.db")));

    let host_gemini = tmp.path().join("empty_host_gemini");
    fs::create_dir_all(&host_gemini).unwrap();

    // Account A must NOT be able to link or resume Account B's conversation
    let res = link_conversation_to_profile(&acct_a, b_cid, Some(&host_gemini));
    assert!(
        res.is_err(),
        "Account A linking Account B's conversation must fail"
    );
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("belongs to another account profile"),
        "Got: {err_msg}"
    );

    // Account B can access its own conversation
    let res_b = link_conversation_to_profile(&acct_b, b_cid, Some(&host_gemini));
    assert!(
        res_b.is_ok(),
        "Account B must be able to link its own conversation"
    );
}

#[tokio::test]
async fn test_9_failed_resume_does_not_silently_launch_replacement() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let missing_conv_id = "88888888-7777-6666-5555-444444444444";

    let db_path = tmp.path().join("events.db");
    let event_store = EventStore::open(&db_path).unwrap();
    let empty_profiles = tmp.path().join("empty_profiles");
    let handle = create_spawned_manager(event_store, Some(host_gemini), Some(empty_profiles));

    // Create a session configured with a missing conversation_id
    let mut launch = AgyLaunchOptions::default();
    launch.conversation_id = Some(missing_conv_id.to_string());
    let session_id = handle
        .create_with_launch(
            "Missing resume task".to_string(),
            "agy".to_string(),
            None,
            None,
            Some(launch),
        )
        .await
        .unwrap();

    // Start must fail because the conversation file does not exist
    let res = handle.start(session_id.clone()).await;
    assert!(
        res.is_err(),
        "Start must fail when conversation file is missing"
    );
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("conversation file is missing or corrupted"),
        "Unexpected error: {err_msg}"
    );

    // Verify session remained Idle and was NOT silently launched or replaced
    let s = handle
        .get(session_id)
        .await
        .unwrap()
        .expect("session must still exist");
    assert_eq!(s.state, SessionState::Idle);
}

#[test]
fn test_10_session_lifecycle_compatibility() {
    let conv_id = Id("12345678-abcd-ef01-2345-6789abcdef01".to_string());
    let mut session = AgentSession::new(
        conv_id.clone(),
        "Discovered External Session".to_string(),
        "agy".to_string(),
    );
    session.is_external = true;
    assert_eq!(session.state, SessionState::Idle);

    // Idle -> Starting
    let r1 = state_machine::apply(&mut session, SessionState::Starting);
    assert_eq!(
        r1,
        state_machine::TransitionResult::Changed {
            from: SessionState::Idle,
            to: SessionState::Starting,
        }
    );

    // Starting -> Working
    let r2 = state_machine::apply(&mut session, SessionState::Working);
    assert_eq!(
        r2,
        state_machine::TransitionResult::Changed {
            from: SessionState::Starting,
            to: SessionState::Working,
        }
    );

    // Working -> Paused
    let r3 = state_machine::apply(&mut session, SessionState::Paused);
    assert_eq!(
        r3,
        state_machine::TransitionResult::Changed {
            from: SessionState::Working,
            to: SessionState::Paused,
        }
    );

    // Paused -> Working
    let r4 = state_machine::apply(&mut session, SessionState::Working);
    assert_eq!(
        r4,
        state_machine::TransitionResult::Changed {
            from: SessionState::Paused,
            to: SessionState::Working,
        }
    );

    // Working -> Stopping
    let r5 = state_machine::apply(&mut session, SessionState::Stopping);
    assert_eq!(
        r5,
        state_machine::TransitionResult::Changed {
            from: SessionState::Working,
            to: SessionState::Stopping,
        }
    );

    // Stopping -> Stopped
    let r6 = state_machine::apply(&mut session, SessionState::Stopped);
    assert_eq!(
        r6,
        state_machine::TransitionResult::Changed {
            from: SessionState::Stopping,
            to: SessionState::Stopped,
        }
    );

    assert_eq!(session.id, conv_id);
    assert_eq!(session.is_external, true);
}

#[tokio::test]
async fn test_11_resume_idle_conversation_via_resume_cmd() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "11223344-5566-7788-99aa-bbccddeeff00";
    let conv_file = conv_dir.join(format!("{conv_id}.db"));
    create_valid_conv_db(&conv_file);

    let db_path = tmp.path().join("events.db");
    let event_store = EventStore::open(&db_path).unwrap();
    let empty_profiles = tmp.path().join("empty_profiles");
    let handle = create_spawned_manager(event_store, Some(host_gemini), Some(empty_profiles));

    // Create a mock session targeting this conversation (starts in Idle)
    let mut launch = AgyLaunchOptions::default();
    launch.conversation_id = Some(conv_id.to_string());
    let session_id = handle
        .create_with_launch(
            "Resume idle task".to_string(),
            "mock".to_string(),
            None,
            None,
            Some(launch),
        )
        .await
        .unwrap();

    let s_before = handle.get(session_id.clone()).await.unwrap().unwrap();
    assert_eq!(s_before.state, SessionState::Idle);

    // Calling resume on an Idle session transitions it to Working via cmd_start
    handle.resume(session_id.clone()).await.unwrap();

    let s_after = handle.get(session_id).await.unwrap().unwrap();
    assert_eq!(s_after.state, SessionState::Working);
}

#[tokio::test]
async fn test_12_inaccessible_workspace_fails_start() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");

    let conv_id = "aabbccdd-0011-2233-4455-667788990011";
    create_valid_conv_db(&conv_dir.join(format!("{conv_id}.db")));

    // Summaries references a nonexistent deleted workspace directory
    let missing_ws = tmp.path().join("nonexistent_deleted_ws");
    create_mock_summaries(
        &summaries_db,
        conv_id,
        "Inaccessible Workspace Task",
        "Preview with missing workspace",
        Some(&missing_ws),
    );

    let db_path = tmp.path().join("events.db");
    let event_store = EventStore::open(&db_path).unwrap();
    let empty_profiles = tmp.path().join("empty_profiles");
    let handle = create_spawned_manager(event_store, Some(host_gemini), Some(empty_profiles));

    // Session is discovered with the missing workspace path
    let session_id = Id(conv_id.to_string());
    let s = handle
        .get(session_id.clone())
        .await
        .unwrap()
        .expect("session must be discovered");
    assert_eq!(s.state, SessionState::Idle);

    // Starting must fail cleanly because the workspace directory was deleted / does not exist
    let res = handle.start(session_id.clone()).await;
    assert!(
        res.is_err(),
        "Starting with nonexistent workspace must fail"
    );
    let err = res.unwrap_err().to_string();
    assert!(err.contains("Workspace path does not exist"), "Got: {err}");

    let s = handle.get(session_id).await.unwrap().unwrap();
    assert_eq!(s.state, SessionState::Idle);
}

#[test]
fn test_13_corrupted_conversation_skipped_during_discovery() {
    let tmp = TempDir::new().unwrap();
    let host_gemini = tmp.path().join("host_gemini");
    let agy_dir = host_gemini.join("antigravity-cli");
    let summaries_db = agy_dir.join("conversation_summaries.db");
    let conv_dir = agy_dir.join("conversations");
    fs::create_dir_all(&conv_dir).unwrap();

    let bad_conv_id = "bad00000-0000-0000-0000-000000000000";
    // Write 10 bytes junk (truncated/corrupted SQLite)
    fs::write(conv_dir.join(format!("{bad_conv_id}.db")), b"corrupt").unwrap();

    create_mock_summaries(
        &summaries_db,
        bad_conv_id,
        "Corrupted Db Summary",
        "Preview of corrupt db",
        None,
    );

    let discovered = discover_external_conversations(Some(&host_gemini)).unwrap();
    assert_eq!(
        discovered.len(),
        0,
        "Corrupted trajectory file must be skipped during discovery"
    );
}
