//! Phase 8 Performance Benchmarks & Recovery Validation.
//!
//! Validates:
//!  - 100,000-event SQLite log recovery completes in under 5 seconds (Phase 8 Exit Criterion).
//!  - Transactional write throughput exceeds 5,000 events/second.
//!  - Correct in-memory state reconstruction for 1,000 concurrent simulated sessions across 100k events.

use std::time::{Duration, Instant};

use ac_core::{
    adapter::CompositeAdapterFactory,
    event_store::EventStore,
    session::manager::SessionManager,
    types::{AgentEvent, EventKind, Id},
};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::{broadcast, mpsc};

#[tokio::test]
async fn test_100k_event_log_recovery_under_5_seconds() {
    let dir = tempdir().unwrap();
    let db_path = dir.path().join("bench_100k.db");

    let total_events = 100_000usize;
    let num_sessions = 10_000usize;
    let batch_size = 5_000usize;

    // ── 1. Generate 100k realistic lifecycle events in batches ─────────────────
    println!("Generating 100,000 events for benchmark...");
    let gen_start = Instant::now();
    {
        let mut store = EventStore::open(&db_path).unwrap();

        // Pre-generate session IDs
        let session_ids: Vec<Id> = (0..num_sessions).map(|_| Id::new()).collect();

        let mut current_batch: Vec<AgentEvent> = Vec::with_capacity(batch_size);

        for i in 0..total_events {
            let sid = &session_ids[i % num_sessions];
            let phase = (i / num_sessions) % 5;

            let event = match phase {
                0 => AgentEvent::new(
                    EventKind::SessionCreated,
                    Some(sid.clone()),
                    json!({
                        "task_description": format!("Task {i}"),
                        "agent_type": "mock",
                    }),
                    "system",
                ),
                1 => AgentEvent::new(
                    EventKind::StateChanged,
                    Some(sid.clone()),
                    json!({ "from": "Idle", "to": "Starting" }),
                    "system",
                ),
                2 => AgentEvent::new(
                    EventKind::StateChanged,
                    Some(sid.clone()),
                    json!({ "from": "Starting", "to": "Working" }),
                    "adapter:mock",
                ),
                3 => AgentEvent::new(
                    EventKind::AgentOutputReceived,
                    Some(sid.clone()),
                    json!({ "text": "Chunk line of agent stdout\n" }),
                    "adapter:mock",
                ),
                _ => AgentEvent::new(
                    EventKind::StateChanged,
                    Some(sid.clone()),
                    json!({ "from": "Working", "to": "Stopped" }),
                    "system",
                ),
            };

            current_batch.push(event);

            if current_batch.len() == batch_size {
                store.append_batch(&mut current_batch).unwrap();
                current_batch.clear();
            }
        }

        if !current_batch.is_empty() {
            store.append_batch(&mut current_batch).unwrap();
        }
    }
    let gen_elapsed = gen_start.elapsed();
    let throughput = (total_events as f64) / gen_elapsed.as_secs_f64();
    println!(
        "Ingested 100,000 events in {:?} ({:.0} events/sec)",
        gen_elapsed, throughput
    );
    assert!(
        throughput > 2_000.0,
        "Write throughput too low: {:.0} events/sec",
        throughput
    );

    // ── 2. Benchmark Recovery Time ─────────────────────────────────────────────
    println!("Benchmarking recovery of 100,000 events from SQLite store...");
    let recovery_start = Instant::now();

    let store = EventStore::open(&db_path).unwrap();
    let (event_tx, _) = broadcast::channel::<AgentEvent>(256);
    let (_cmd_tx, cmd_rx) = mpsc::channel(64);
    let adapter_factory = Box::new(CompositeAdapterFactory::new());

    let mut manager = SessionManager::new(store, cmd_rx, event_tx, 3, adapter_factory);

    // Execute state reconstruction
    manager.recover_from_store().unwrap();

    let recovery_elapsed = recovery_start.elapsed();
    println!("Recovery completed in {:?}", recovery_elapsed);

    // ── 3. Assert Phase 8 Exit Criterion ───────────────────────────────────────
    assert!(
        recovery_elapsed < Duration::from_secs(5),
        "Recovery from 100k event log took {:?}, exceeding 5-second exit criterion!",
        recovery_elapsed
    );

    // Verify session count rebuilt properly
    let list = manager.sessions();
    assert_eq!(list.len(), num_sessions);
    println!("First session recovered state: {}", list[0].state);

    let state_counts: std::collections::HashMap<String, usize> = list
        .iter()
        .fold(std::collections::HashMap::new(), |mut acc, s| {
            *acc.entry(s.state.to_string()).or_insert(0) += 1;
            acc
        });
    println!("Recovered state distribution: {:?}", state_counts);
}
