//! Phase 8 Fuzzing & Robustness Test Suite.
//!
//! Stress-tests:
//!  - Control API deserialization against arbitrary malformed, oversized, and pathological JSON
//!  - Claude Code stream event parser against corrupt, partial, and hostile input
//!  - Generic PTY regex pattern classifier against ReDoS patterns, ANSI sequences, and binary data
//!  - Session State Machine property testing: 10,000 random transition trials ensuring no invariant violations

use ac_core::{
    adapter::claude::parse_claude_event,
    adapter::pty::{PtyPatternKind, PtyPatternRule},
    session::state_machine::{apply, is_valid_transition, TransitionResult},
    types::{AgentSession, Id, SessionState},
};
use regex::Regex;
use serde_json::json;

// ── 1. Control API wire parser fuzzing ────────────────────────────────────────

#[test]
fn test_fuzz_control_api_wire_parser() {
    let pathological_inputs = vec![
        "",
        "   ",
        "\0",
        "\0\0\0",
        "{",
        "}",
        "[]",
        "null",
        "12345",
        "true",
        "{\"v\": 1}",                                  // missing id, cmd
        "{\"v\": \"one\", \"id\": 1, \"cmd\": false}", // invalid types
        "{\"v\": 1, \"id\": \"test\", \"cmd\": null}",
        "{\"\": \"\"}",
        "{\"v\": 1, \"id\": \"test\", \"cmd\": \"session.start\", \"params\": null}",
        "{\"v\": 999999999999999999999999999999999999999999999999999999999999999}",
        "/* comment */ {\"v\": 1}",
        "<xml></xml>",
        "NaN",
        "Infinity",
        "-Infinity",
        "{\"v\": 1, \"id\": \"\\u0000\\uD800\", \"cmd\": \"test\"}", // unpaired surrogate
        "{\"v\": 1, \"id\": \"test\", \"cmd\": \"\\r\\n\\t\\b\\f\"}",
    ];

    for input in pathological_inputs {
        let parsed: Result<ac_core::types::ApiRequest, _> = serde_json::from_str(input);
        // We assert that the parser safely either succeeds or returns an error, NEVER panics.
        let _ = parsed;
    }

    // Deeply nested JSON fuzzing (500 levels of nesting)
    let mut deep_json = String::new();
    for _ in 0..500 {
        deep_json.push_str("{\"a\":");
    }
    deep_json.push_str("1");
    for _ in 0..500 {
        deep_json.push('}');
    }
    let parsed_deep: Result<ac_core::types::ApiRequest, _> = serde_json::from_str(&deep_json);
    let _ = parsed_deep; // Handled gracefully by serde without stack overflow

    // Massive 2MB payload
    let massive_string = "A".repeat(2 * 1024 * 1024);
    let massive_json = format!(
        "{{\"v\": 1, \"id\": \"massive\", \"cmd\": \"session.create\", \"params\": {{\"data\": \"{massive_string}\"}}}}"
    );
    let parsed_massive: Result<ac_core::types::ApiRequest, _> = serde_json::from_str(&massive_json);
    let _ = parsed_massive;
}

// ── 2. Claude Code NDJSON parser fuzzing ───────────────────────────────────────

#[test]
fn test_fuzz_claude_ndjson_parser() {
    let corrupt_claude_lines = vec![
        "",
        "\n",
        "Not a JSON line",
        "\x1b[31mRed text ANSI escape\x1b[0m",
        "{\"type\": \"unknown_type_that_does_not_exist\"}",
        "{\"type\": \"tool_use\"}", // missing name / input
        "{\"type\": \"tool_use\", \"name\": null}",
        "{\"type\": \"tool_use\", \"name\": \"Bash\", \"input\": 12345}", // input not object
        "{\"type\": \"progress\"}",
        "{\"type\": \"question\"}", // missing question text
        "{\"type\": \"completed\", \"exit_code\": \"not_a_number\"}",
        "{\"type\": \"rate_limit\", \"retry_after_ms\": -500}",
        "{\"type\": \"error\", \"message\": null}",
        "{\"ready\": true, \"unexpected\": [1, 2, 3, {}]}",
        "{\"type\": \"tool_use\", \"name\": \"Bash\", \"input\": {\"command\": \"\\0\\0\\0\"}}",
        "{\"type\": \"output\", \"text\": \"\x00\x01\x02\x03\x04\x05\x06\x07\x08\"}",
    ];

    for line in corrupt_claude_lines {
        // Must never panic on corrupt or unexpected inputs
        let event_opt = parse_claude_event(line);
        let _ = event_opt;
    }

    // Fuzz with large stream chunks
    let large_output = json!({
        "type": "output",
        "text": "X".repeat(500_000)
    })
    .to_string();
    let ev = parse_claude_event(&large_output);
    assert!(ev.is_some());
}

// ── 3. Generic PTY regex pattern classifier robustness ────────────────────────

#[test]
fn test_fuzz_pty_pattern_classifier_robustness() {
    let patterns = vec![
        PtyPatternRule::new(
            r"Do you want to proceed\? \(y/n\)",
            PtyPatternKind::ApprovalRequested { tool_name: None },
        )
        .unwrap(),
        PtyPatternRule::new(
            r"Rate limit(?:ed)?",
            PtyPatternKind::RateLimitSignal {
                back_off_secs: Some(60),
            },
        )
        .unwrap(),
        PtyPatternRule::new(
            r"Process finished with exit code (\d+)",
            PtyPatternKind::Completed { summary: None },
        )
        .unwrap(),
    ];

    // 1. Pathological and high-entropy chunk fuzzing
    let repeat_chunk = "A".repeat(100_000);
    let fuzz_chunks = vec![
        "\x1b[0m\x1b[1;32muser@host\x1b[0m:\x1b[1;34m~/repo\x1b[0m$ ",
        "\x1b[?2004h? Do you want to proceed? (y/n) \x1b[?2004l",
        "\r\n\r\n\r\n\x00\x00\x00\x1b[2J\x1b[H",
        "Error 429: Too Many Requests (Rate limit exceeded, retry in 60s)",
        "Rate limited: 120 seconds cooldown",
        "Process finished with exit code 0",
        "Process exited with code 1",
        "Random text with no special patterns whatsoever",
        repeat_chunk.as_str(), // 100KB buffer chunk
    ];

    for chunk in fuzz_chunks {
        for rule in &patterns {
            let _ = rule.pattern.captures(chunk);
        }
    }

    // 2. Catastrophic backtracking test on custom pattern config
    // Ensure regex evaluation completes in bounded time
    let custom_regex = Regex::new(r"^(a+)+$").unwrap();
    let hostile_match_str = "a".repeat(25) + "X";
    let start = std::time::Instant::now();
    let is_match = custom_regex.is_match(&hostile_match_str);
    assert!(!is_match);
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "Regex took too long: potential ReDoS!"
    );
}

// ── 4. Session State Machine property testing (10,000 trials) ─────────────────

#[test]
fn test_property_state_machine_random_transitions() {
    let all_states = [
        SessionState::Idle,
        SessionState::Starting,
        SessionState::Working,
        SessionState::WaitingForHuman,
        SessionState::Paused,
        SessionState::RateLimited,
        SessionState::Stopping,
        SessionState::Stopped,
        SessionState::Failed,
        SessionState::Crashed,
        SessionState::Restarting,
        SessionState::HandedOff,
    ];

    // Simple pseudo-random LCG generator for deterministic reproducibility without extra dependencies
    let mut seed: u64 = 0xDEADBEEFCAFEBABE;
    let mut next_rand = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        seed
    };

    let mut session = AgentSession::new(Id::new(), "test".into(), "mock".into());

    for _ in 0..10_000 {
        let current = session.state.clone();
        let target_idx = (next_rand() as usize) % all_states.len();
        let target = all_states[target_idx].clone();

        let valid = is_valid_transition(&current, &target);
        let res = apply(&mut session, target.clone());

        if valid {
            assert_eq!(
                res,
                TransitionResult::Changed {
                    from: current.clone(),
                    to: target.clone()
                },
                "Valid transition from {current} to {target} failed!"
            );
            assert_eq!(session.state, target);
        } else {
            assert_eq!(
                res,
                TransitionResult::Rejected {
                    current: current.clone(),
                    attempted: target.clone()
                },
                "Invalid transition from {current} to {target} was accepted!"
            );
            assert_eq!(session.state, current);
        }

        // If terminal state reached, reset session for the next sequence
        if session.state.is_terminal() {
            session = AgentSession::new(Id::new(), "test".into(), "mock".into());
        }
    }
}
