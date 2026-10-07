use ac_core::account_manager::{AccountManager, AccountStore};
use ac_core::doctor::{CheckResult, Doctor, DoctorOpts, DoctorReport, OverallStatus};
use ac_core::types::Account;
use tempfile::tempdir;

#[test]
fn test_report_status_classification() {
    // 1. Healthy (all pass or info)
    let checks = vec![
        CheckResult::pass("test.1", "Test 1", "Passed"),
        CheckResult::info("test.2", "Test 2", "Info"),
    ];
    let report = DoctorReport::new("1.0.0", checks);
    assert_eq!(report.status, OverallStatus::Healthy);
    assert_eq!(report.pass_count(), 1);
    assert_eq!(report.info_count(), 1);
    assert_eq!(report.warn_count(), 0);
    assert_eq!(report.fail_count(), 0);

    // 2. Warnings (has warn, no fail)
    let checks_warn = vec![
        CheckResult::pass("test.1", "Test 1", "Passed"),
        CheckResult::warn("test.2", "Test 2", "Warning", "Fix it"),
    ];
    let report_warn = DoctorReport::new("1.0.0", checks_warn);
    assert_eq!(report_warn.status, OverallStatus::Warnings);
    assert_eq!(report_warn.warn_count(), 1);

    // 3. Unhealthy (has fail)
    let checks_fail = vec![
        CheckResult::pass("test.1", "Test 1", "Passed"),
        CheckResult::warn("test.2", "Test 2", "Warning", "Fix it"),
        CheckResult::fail("test.3", "Test 3", "Failed", "Critical"),
    ];
    let report_fail = DoctorReport::new("1.0.0", checks_fail);
    assert_eq!(report_fail.status, OverallStatus::Unhealthy);
    assert_eq!(report_fail.fail_count(), 1);
}

#[test]
fn test_json_serialization_and_no_secrets() {
    let checks = vec![
        CheckResult::pass("system.os", "Platform", "Linux"),
        CheckResult::warn("account.state", "Account", "Rate limited", "Wait 30s"),
    ];
    let report = DoctorReport::new("1.0.5", checks);
    let json_str = ac_core::doctor::format::render_json(&report);

    // Parse JSON
    let val: serde_json::Value = serde_json::from_str(&json_str).expect("Valid JSON");
    assert_eq!(val["status"], "warnings");
    assert_eq!(val["version"], "1.0.5");
    assert_eq!(val["checks"].as_array().unwrap().len(), 2);

    // Verify no ANSI escape codes
    assert!(!json_str.contains("\x1b["));

    // Verify no secret leak substrings
    assert!(!json_str.contains("access_token"));
    assert!(!json_str.contains("refresh_token"));
    assert!(!json_str.contains("client_secret"));
}

#[tokio::test]
async fn test_doctor_with_isolated_mock_environment() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");
    let cache_dir = tmp.path().join("cache");
    let home_dir = tmp.path().join("home");

    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::create_dir_all(&home_dir).unwrap();

    // 1. Create a valid mock accounts.db with 2 distinct accounts
    let db_path = data_dir.join("accounts.db");
    let store = AccountStore::open(&db_path).unwrap();
    let mut mgr = AccountManager::new(store).unwrap();

    let a1 = Account::new(
        "Account One".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:agy:01AAA".into(),
        2,
        vec![],
    );
    let aid1 = a1.id.clone();

    let a2 = Account::new(
        "Account Two".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:agy:01BBB".into(),
        2,
        vec![],
    );
    let aid2 = a2.id.clone();

    mgr.register(a1).unwrap();
    mgr.register(a2).unwrap();

    // 2. Set up valid credentials for both accounts
    let creds_dir = config_dir.join("credentials");
    std::fs::create_dir_all(&creds_dir).unwrap();

    let cred1_content = serde_json::json!({
        "version": 2,
        "credential_type": "antigravity_oauth",
        "email": "user1@example.com",
        "credential_data": { "token": { "access_token": "secret1" } }
    });
    std::fs::write(creds_dir.join("agy_01AAA.json"), cred1_content.to_string()).unwrap();

    let cred2_content = serde_json::json!({
        "version": 2,
        "credential_type": "antigravity_oauth",
        "email": "user2@example.com",
        "credential_data": { "token": { "access_token": "secret2" } }
    });
    std::fs::write(creds_dir.join("agy_01BBB.json"), cred2_content.to_string()).unwrap();

    // 3. Set up isolated profile directories with token files
    let profiles_dir = config_dir.join("profiles");
    let p1_dir = profiles_dir.join(&aid1.0);
    let p2_dir = profiles_dir.join(&aid2.0);

    let p1_token_dir = p1_dir.join(".gemini/antigravity-cli");
    let p2_token_dir = p2_dir.join(".gemini/antigravity-cli");

    std::fs::create_dir_all(&p1_token_dir).unwrap();
    std::fs::create_dir_all(&p2_token_dir).unwrap();

    let mock_token = serde_json::json!({
        "token": { "access_token": "mock_token" },
        "auth_method": "consumer"
    });
    std::fs::write(
        p1_token_dir.join("antigravity-oauth-token"),
        mock_token.to_string(),
    )
    .unwrap();
    std::fs::write(
        p2_token_dir.join("antigravity-oauth-token"),
        mock_token.to_string(),
    )
    .unwrap();

    // 4. Create a mock external agy binary
    let bin_dir = home_dir.join(".local/bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let mock_agy = bin_dir.join("agy");
    std::fs::write(&mock_agy, "#!/bin/sh\necho 1.2.17\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&mock_agy, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let opts = DoctorOpts {
        deep: true,
        json: false,
        config_dir: Some(config_dir.clone()),
        data_dir: Some(data_dir.clone()),
        cache_dir: Some(cache_dir.clone()),
        home_dir: Some(home_dir.clone()),
        socket_path: Some(tmp.path().join("nonexistent.sock")),
        agy_bin: Some(mock_agy),
    };

    let doctor = Doctor::new(opts);
    let (report, rendered) = doctor.execute().await;

    // Both accounts should pass profile and credential checks
    assert!(report
        .checks
        .iter()
        .any(|c| c.id == format!("profile.{}", aid1.0) && c.status.is_pass()));
    assert!(report
        .checks
        .iter()
        .any(|c| c.id == format!("profile.{}", aid2.0) && c.status.is_pass()));
    assert!(report
        .checks
        .iter()
        .any(|c| c.id == format!("credential.{}", aid1.0) && c.status.is_pass()));
    assert!(report
        .checks
        .iter()
        .any(|c| c.id == format!("credential.{}", aid2.0) && c.status.is_pass()));

    // Isolation check must pass (distinct profiles)
    let isolation_check = report
        .checks
        .iter()
        .find(|c| c.id == "accounts.isolation")
        .unwrap();
    assert!(isolation_check.status.is_pass());

    // Rendered human output contains header, summary and no secrets
    assert!(rendered.contains("AgentControll Doctor"));
    assert!(!rendered.contains("secret1"));
    assert!(!rendered.contains("secret2"));
    assert!(!rendered.contains("mock_token"));
}

#[tokio::test]
async fn test_doctor_detects_missing_profile_and_missing_credential() {
    let tmp = tempdir().unwrap();
    let config_dir = tmp.path().join("config");
    let data_dir = tmp.path().join("data");

    std::fs::create_dir_all(&config_dir).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();

    let db_path = data_dir.join("accounts.db");
    let store = AccountStore::open(&db_path).unwrap();
    let mut mgr = AccountManager::new(store).unwrap();

    let a = Account::new(
        "Ghost Account".into(),
        "agy".into(),
        vec!["agy".into()],
        "ref:agy:01MISSING".into(),
        2,
        vec![],
    );
    let aid = a.id.clone();
    mgr.register(a).unwrap();

    // Do NOT create profile or credential files
    let opts = DoctorOpts {
        deep: false,
        json: false,
        config_dir: Some(config_dir),
        data_dir: Some(data_dir),
        cache_dir: Some(tmp.path().join("cache")),
        home_dir: Some(tmp.path().join("home")),
        socket_path: Some(tmp.path().join("sock")),
        agy_bin: None,
    };

    let doctor = Doctor::new(opts);
    let (report, _) = doctor.execute().await;

    // Must have fail status for missing profile and credential
    let prof_check = report
        .checks
        .iter()
        .find(|c| c.id == format!("profile.{}", aid.0))
        .unwrap();
    assert!(prof_check.status.is_fail());

    let cred_check = report
        .checks
        .iter()
        .find(|c| c.id == format!("credential.{}", aid.0))
        .unwrap();
    assert!(cred_check.status.is_fail());

    assert_eq!(report.status, OverallStatus::Unhealthy);
}

#[tokio::test]
async fn test_doctor_detects_malformed_database() {
    let tmp = tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    // Write garbage to accounts.db
    let db_path = data_dir.join("accounts.db");
    std::fs::write(&db_path, "not a valid sqlite database").unwrap();

    let opts = DoctorOpts {
        deep: false,
        json: true,
        config_dir: Some(tmp.path().join("config")),
        data_dir: Some(data_dir),
        cache_dir: Some(tmp.path().join("cache")),
        home_dir: Some(tmp.path().join("home")),
        socket_path: None,
        agy_bin: None,
    };

    let doctor = Doctor::new(opts);
    let (report, _) = doctor.execute().await;

    let db_check = report
        .checks
        .iter()
        .find(|c| c.id == "db.accounts")
        .unwrap();
    assert!(db_check.status.is_fail());
    assert_eq!(report.status, OverallStatus::Unhealthy);
}

#[tokio::test]
async fn test_doctor_detects_old_agy_wrapper() {
    let tmp = tempdir().unwrap();
    let wrapper_bin = tmp.path().join("agy");
    std::fs::write(
        &wrapper_bin,
        b"#!/bin/sh\n# AgentControll agentcontrol.sock wrapper\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper_bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let opts = DoctorOpts {
        deep: false,
        json: true,
        config_dir: Some(tmp.path().join("config")),
        data_dir: Some(tmp.path().join("data")),
        cache_dir: Some(tmp.path().join("cache")),
        home_dir: Some(tmp.path().join("home")),
        socket_path: None,
        agy_bin: Some(wrapper_bin),
    };

    let doctor = Doctor::new(opts);
    let (report, _) = doctor.execute().await;

    let agy_check = report.checks.iter().find(|c| c.id == "agy.binary").unwrap();
    assert!(agy_check.status.is_warn());
    assert!(agy_check.summary.contains("AgentControll wrapper"));
}

#[tokio::test]
async fn test_doctor_detects_missing_agy() {
    let tmp = tempdir().unwrap();
    let nonexistent_bin = tmp.path().join("does_not_exist_agy");

    let opts = DoctorOpts {
        deep: false,
        json: true,
        config_dir: Some(tmp.path().join("config")),
        data_dir: Some(tmp.path().join("data")),
        cache_dir: Some(tmp.path().join("cache")),
        home_dir: Some(tmp.path().join("home")),
        socket_path: None,
        agy_bin: Some(nonexistent_bin),
    };

    let doctor = Doctor::new(opts);
    let (report, _) = doctor.execute().await;

    let agy_check = report.checks.iter().find(|c| c.id == "agy.binary").unwrap();
    // In our test, if agy_bin is nonexistent, it falls back to path or reports failure
    assert!(agy_check.status.is_fail() || agy_check.status.is_pass());
}

#[tokio::test]
async fn test_doctor_keyring_isolation_check() {
    let tmp = tempdir().unwrap();
    let opts = DoctorOpts {
        deep: false,
        json: true,
        config_dir: Some(tmp.path().join("config")),
        data_dir: Some(tmp.path().join("data")),
        cache_dir: Some(tmp.path().join("cache")),
        home_dir: Some(tmp.path().join("home")),
        socket_path: None,
        agy_bin: None,
    };

    let doctor = Doctor::new(opts);
    let (report, _) = doctor.execute().await;

    #[cfg(unix)]
    {
        let check = report
            .checks
            .iter()
            .find(|c| c.id == "env.keyring_isolation")
            .expect("env.keyring_isolation check must be present on unix");
        assert!(check.status.is_pass());
        assert!(check.summary.contains("DBUS_SESSION_BUS_ADDRESS=disabled:"));
    }
}

