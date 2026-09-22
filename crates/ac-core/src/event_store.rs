//! Append-only SQLite event store.
//!
//! The store owns two tables:
//!  - `events`   — all `AgentEvent`s in sequence-number order, never updated or deleted.
//!  - `snapshots`— periodic derived-state snapshots bound to a sequence number.
//!
//! On open, the store verifies that sequence numbers are gapless.

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use std::path::Path;

use crate::types::{AgentEvent, EventKind, Id};

const SCHEMA_VERSION: u32 = 1;

/// Append-only SQLite event store.
pub struct EventStore {
    conn: Connection,
}

impl EventStore {
    /// Open (or create) the event store at the given path.
    pub fn open(path: &Path) -> Result<Self> {
        // Create parent directory if it doesn't exist
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating db directory: {}", parent.display()))?;
        }

        let conn = Connection::open(path)
            .with_context(|| format!("opening sqlite db: {}", path.display()))?;

        let store = Self { conn };
        store.init_schema()?;
        store.verify_sequence_integrity()?;
        Ok(store)
    }

    /// Open an in-memory event store (for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()
            .context("opening in-memory sqlite db")?;
        let store = Self { conn };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;

             CREATE TABLE IF NOT EXISTS schema_meta (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );

             INSERT OR IGNORE INTO schema_meta (key, value)
             VALUES ('version', '{SCHEMA_VERSION}');

             CREATE TABLE IF NOT EXISTS events (
                 seq          INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_id     TEXT    NOT NULL UNIQUE,
                 version      INTEGER NOT NULL DEFAULT 1,
                 kind         TEXT    NOT NULL,
                 session_id   TEXT,
                 payload      TEXT    NOT NULL,
                 triggered_by TEXT    NOT NULL,
                 timestamp    TEXT    NOT NULL
             );

             CREATE INDEX IF NOT EXISTS idx_events_session
                 ON events (session_id) WHERE session_id IS NOT NULL;
             CREATE INDEX IF NOT EXISTS idx_events_kind
                 ON events (kind);
             CREATE INDEX IF NOT EXISTS idx_events_timestamp
                 ON events (timestamp);

             CREATE TABLE IF NOT EXISTS snapshots (
                 id           INTEGER PRIMARY KEY AUTOINCREMENT,
                 event_seq    INTEGER NOT NULL,
                 payload      TEXT    NOT NULL,
                 created_at   TEXT    NOT NULL
             );",
        ))
        .context("initialising event store schema")?;
        Ok(())
    }

    /// Verify that all sequence numbers in the events table are gapless.
    /// A gap means the log has been tampered with or is corrupt.
    fn verify_sequence_integrity(&self) -> Result<()> {
        let gap_count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM (
                 SELECT seq, LAG(seq, 1, 0) OVER (ORDER BY seq) AS prev
                 FROM events
             ) WHERE seq - prev > 1",
            [],
            |row| row.get(0),
        )
        .context("checking sequence integrity")?;

        if gap_count > 0 {
            bail!(
                "Event store integrity violation: {gap_count} sequence gap(s) detected. \
                 The event log may have been tampered with. \
                 Resolve before starting the daemon."
            );
        }
        Ok(())
    }

    /// Append an event. Assigns and returns the sequence number.
    pub fn append(&mut self, event: &mut AgentEvent) -> Result<u64> {
        let kind_str = event.kind.to_string();
        let payload_str = serde_json::to_string(&event.payload)
            .context("serialising event payload")?;
        let ts_str = event.timestamp.to_rfc3339();
        let session_id = event.session_id.as_ref().map(|id| id.0.as_str());

        self.conn.execute(
            "INSERT INTO events (event_id, version, kind, session_id, payload, triggered_by, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                event.id.0,
                event.version,
                kind_str,
                session_id,
                payload_str,
                event.triggered_by,
                ts_str,
            ],
        )
        .context("appending event to store")?;

        let seq = self.conn.last_insert_rowid() as u64;
        event.seq = seq;
        Ok(seq)
    }

    /// Return the current maximum sequence number (0 if the store is empty).
    pub fn max_seq(&self) -> Result<u64> {
        let seq: Option<i64> = self.conn
            .query_row("SELECT MAX(seq) FROM events", [], |row| row.get(0))
            .context("querying max seq")?;
        Ok(seq.unwrap_or(0) as u64)
    }

    /// Return all events, in sequence order, optionally starting from `since_seq`.
    pub fn query(
        &self,
        since_seq: u64,
        session_id: Option<&Id>,
        limit: Option<u64>,
    ) -> Result<Vec<AgentEvent>> {
        let limit_clause = limit
            .map(|l| format!("LIMIT {l}"))
            .unwrap_or_default();

        let rows = if let Some(sid) = session_id {
            let sql = format!(
                "SELECT seq, event_id, version, kind, session_id, payload, triggered_by, timestamp \
                 FROM events WHERE seq > ?1 AND session_id = ?2 ORDER BY seq {limit_clause}"
            );
            let mut stmt = self.conn.prepare(&sql).context("preparing query")?;
            let rows = stmt
                .query_map(params![since_seq as i64, sid.0], row_to_event)
                .context("querying events")?
                .collect::<Result<Vec<_>, _>>()
                .context("collecting events")?;
            rows
        } else {
            let sql = format!(
                "SELECT seq, event_id, version, kind, session_id, payload, triggered_by, timestamp \
                 FROM events WHERE seq > ?1 ORDER BY seq {limit_clause}"
            );
            let mut stmt = self.conn.prepare(&sql).context("preparing query")?;
            let rows = stmt
                .query_map(params![since_seq as i64], row_to_event)
                .context("querying events")?
                .collect::<Result<Vec<_>, _>>()
                .context("collecting events")?;
            rows
        };

        Ok(rows)
    }

    /// Return a single event by sequence number.
    pub fn get_by_seq(&self, seq: u64) -> Result<Option<AgentEvent>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, event_id, version, kind, session_id, payload, triggered_by, timestamp \
             FROM events WHERE seq = ?1",
        )?;
        let mut rows = stmt
            .query_map(params![seq as i64], row_to_event)
            .context("querying event by seq")?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Save a derived-state snapshot bound to the given event sequence number.
    pub fn save_snapshot(&mut self, event_seq: u64, payload: &serde_json::Value) -> Result<()> {
        let payload_str = serde_json::to_string(payload).context("serialising snapshot")?;
        let now = chrono::Utc::now().to_rfc3339();
        self.conn.execute(
            "INSERT INTO snapshots (event_seq, payload, created_at) VALUES (?1, ?2, ?3)",
            params![event_seq as i64, payload_str, now],
        )
        .context("saving snapshot")?;
        Ok(())
    }

    /// Load the latest snapshot, returning the event_seq it was taken at and
    /// the payload. Returns `None` if no snapshot exists.
    pub fn load_latest_snapshot(&self) -> Result<Option<(u64, serde_json::Value)>> {
        let mut stmt = self.conn.prepare(
            "SELECT event_seq, payload FROM snapshots ORDER BY event_seq DESC LIMIT 1",
        )?;
        let mut rows = stmt.query_map([], |row| {
            let seq: i64 = row.get(0)?;
            let payload_str: String = row.get(1)?;
            Ok((seq as u64, payload_str))
        })?;
        match rows.next() {
            Some(r) => {
                let (seq, payload_str) = r?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_str).context("deserialising snapshot")?;
                Ok(Some((seq, payload)))
            }
            None => Ok(None),
        }
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentEvent> {
    let seq: i64 = row.get(0)?;
    let event_id: String = row.get(1)?;
    let version: u16 = row.get(2)?;
    let kind_str: String = row.get(3)?;
    let session_id: Option<String> = row.get(4)?;
    let payload_str: String = row.get(5)?;
    let triggered_by: String = row.get(6)?;
    let timestamp_str: String = row.get(7)?;

    let kind: EventKind = serde_json::from_value(serde_json::Value::String(kind_str.clone()))
        .unwrap_or_else(|_| EventKind::DaemonStarted); // fallback for unknown kinds during migration

    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).unwrap_or(serde_json::Value::Null);

    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp_str)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::Utc::now());

    Ok(AgentEvent {
        seq: seq as u64,
        id: Id(event_id),
        version,
        kind,
        session_id: session_id.map(Id),
        payload,
        triggered_by,
        timestamp,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentEvent, EventKind, Id};
    use serde_json::json;

    fn make_event(kind: EventKind, session_id: Option<Id>) -> AgentEvent {
        AgentEvent::new(kind, session_id, json!({"test": true}), "system")
    }

    #[test]
    fn append_and_query() {
        let mut store = EventStore::open_in_memory().unwrap();

        let mut e1 = make_event(EventKind::SessionCreated, Some(Id::from("s1")));
        let mut e2 = make_event(EventKind::SessionStarted, Some(Id::from("s1")));

        let seq1 = store.append(&mut e1).unwrap();
        let seq2 = store.append(&mut e2).unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let events = store.query(0, None, None).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
    }

    #[test]
    fn query_by_session() {
        let mut store = EventStore::open_in_memory().unwrap();

        let mut e1 = make_event(EventKind::SessionCreated, Some(Id::from("s1")));
        let mut e2 = make_event(EventKind::SessionCreated, Some(Id::from("s2")));
        let mut e3 = make_event(EventKind::SessionStarted, Some(Id::from("s1")));

        store.append(&mut e1).unwrap();
        store.append(&mut e2).unwrap();
        store.append(&mut e3).unwrap();

        let s1_events = store.query(0, Some(&Id::from("s1")), None).unwrap();
        assert_eq!(s1_events.len(), 2);

        let s2_events = store.query(0, Some(&Id::from("s2")), None).unwrap();
        assert_eq!(s2_events.len(), 1);
    }

    #[test]
    fn query_since_seq() {
        let mut store = EventStore::open_in_memory().unwrap();
        for i in 0..5 {
            let mut e = make_event(EventKind::DaemonStarted, None);
            e.payload = json!({"i": i});
            store.append(&mut e).unwrap();
        }
        let events = store.query(3, None, None).unwrap();
        assert_eq!(events.len(), 2); // seq 4 and 5
    }

    #[test]
    fn max_seq_empty() {
        let store = EventStore::open_in_memory().unwrap();
        assert_eq!(store.max_seq().unwrap(), 0);
    }

    #[test]
    fn snapshot_round_trip() {
        let mut store = EventStore::open_in_memory().unwrap();
        let payload = json!({"sessions": [], "accounts": []});
        store.save_snapshot(42, &payload).unwrap();

        let loaded = store.load_latest_snapshot().unwrap().unwrap();
        assert_eq!(loaded.0, 42);
        assert_eq!(loaded.1, payload);
    }

    #[test]
    fn get_by_seq() {
        let mut store = EventStore::open_in_memory().unwrap();
        let mut e = make_event(EventKind::SessionCreated, Some(Id::from("s1")));
        let seq = store.append(&mut e).unwrap();

        let loaded = store.get_by_seq(seq).unwrap().unwrap();
        assert_eq!(loaded.seq, seq);
        assert_eq!(loaded.session_id, Some(Id::from("s1")));
    }

    #[test]
    fn persistence_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("events.db");

        {
            let mut store = EventStore::open(&db_path).unwrap();
            let mut e = make_event(EventKind::SessionCreated, Some(Id::from("s1")));
            store.append(&mut e).unwrap();
        }

        // Reopen — should reconstruct successfully
        let store = EventStore::open(&db_path).unwrap();
        let events = store.query(0, None, None).unwrap();
        assert_eq!(events.len(), 1);
    }
}
