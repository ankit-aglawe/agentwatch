//! Writer side of the store. The TUI process owns this; the webapp is read-only.
//!
//! Idempotency: events are keyed on `AgentEvent.id` which is the PRIMARY KEY.
//! Inserts use `INSERT OR IGNORE`, so re-parsing a source file produces the
//! same id for the same line and the second write is a no-op.

use agentwatch_core::{AgentEvent, EventKind, Microcents};
use chrono::Local;
use rusqlite::{params, Connection};
use std::path::Path;

use crate::{schema, StoreError};

pub struct Writer {
    conn: Connection,
}

impl Writer {
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(schema::PRAGMA_WAL)?;
        conn.execute_batch(schema::PRAGMA_BUSY_TIMEOUT)?;
        conn.execute_batch(schema::PRAGMA_FOREIGN_KEYS)?;
        conn.execute_batch(schema::CREATE_EVENTS)?;
        // Idempotent migration: ignore "duplicate column" errors for the
        // ALTER TABLE on older DBs.
        if let Err(e) = conn.execute_batch(schema::ALTER_ADD_PROJECT) {
            if !e.to_string().contains("duplicate column name") {
                return Err(StoreError::Sqlite(e));
            }
        }
        conn.execute_batch(schema::CREATE_EVENTS_INDEX)?;
        conn.execute_batch(schema::CREATE_EVENTS_SESSION_INDEX)?;
        conn.execute_batch(schema::CREATE_EVENTS_PROJECT_INDEX)?;
        conn.execute_batch(schema::CREATE_DAILY_TOTALS)?;
        conn.execute_batch(schema::CREATE_SOURCE_STATE)?;
        conn.pragma_update(None, "user_version", crate::SCHEMA_VERSION)?;
        Ok(Self { conn })
    }

    /// Has this source changed since we last ingested it?
    /// Compares mtime + size. Returns true if we need to (re-)parse.
    pub fn source_needs_ingest(
        &self,
        agent: &str,
        path: &str,
        mtime_ms: i64,
        size_bytes: u64,
    ) -> Result<bool, StoreError> {
        let row = self.conn.query_row(
            "SELECT last_mtime_ms, last_size_bytes FROM source_state
             WHERE agent = ?1 AND path = ?2",
            params![agent, path],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );
        match row {
            Ok((stored_mtime, stored_size)) => {
                Ok(stored_mtime != mtime_ms || stored_size != size_bytes as i64)
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(true),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Record that we successfully ingested this source up to its current mtime.
    pub fn mark_source_ingested(
        &self,
        agent: &str,
        path: &str,
        mtime_ms: i64,
        size_bytes: u64,
    ) -> Result<(), StoreError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.conn.execute(
            "INSERT INTO source_state (agent, path, last_mtime_ms, last_size_bytes, last_ingested_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(agent, path) DO UPDATE SET
                 last_mtime_ms = excluded.last_mtime_ms,
                 last_size_bytes = excluded.last_size_bytes,
                 last_ingested_at_ms = excluded.last_ingested_at_ms",
            params![agent, path, mtime_ms, size_bytes as i64, now_ms],
        )?;
        Ok(())
    }

    /// Write many events in a single transaction. 100× faster than calling
    /// `insert` per event because each `INSERT` no longer triggers its own
    /// fsync. Use this from ingest and demo.
    pub fn insert_batch(&mut self, events: &[AgentEvent]) -> Result<usize, StoreError> {
        if events.is_empty() {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let mut inserted = 0usize;
        for event in events {
            let (cost, input_tokens, output_tokens) = extract_metrics(&event.kind);
            let kind_tag = kind_tag(&event.kind);
            let payload = serde_json::to_string(event).map_err(|e| {
                StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
            })?;
            let timestamp_ms = event.timestamp.timestamp_millis();
            let agent = event.agent.display_name();

            let n = tx.execute(
                "INSERT OR IGNORE INTO events
                    (id, agent, session_id, timestamp_ms, kind, cost_microcents,
                     input_tokens, output_tokens, project, payload, source_offset)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    event.id.to_string(),
                    agent,
                    event.session_id,
                    timestamp_ms,
                    kind_tag,
                    cost.as_u64() as i64,
                    input_tokens as i64,
                    output_tokens as i64,
                    event.project,
                    payload,
                    event.source_offset.map(|o| o as i64),
                ],
            )?;
            if n == 1 {
                let local_date = event
                    .timestamp
                    .with_timezone(&Local)
                    .format("%Y-%m-%d")
                    .to_string();
                tx.execute(
                    "INSERT INTO daily_totals (date_local, agent, cost_microcents, event_count)
                     VALUES (?1, ?2, ?3, 1)
                     ON CONFLICT(date_local, agent) DO UPDATE SET
                         cost_microcents = cost_microcents + excluded.cost_microcents,
                         event_count = event_count + 1",
                    params![local_date, agent, cost.as_u64() as i64],
                )?;
                inserted += 1;
            }
        }
        tx.commit()?;
        Ok(inserted)
    }

    /// Write one event. Idempotent on `event.id`.
    pub fn insert(&self, event: &AgentEvent) -> Result<(), StoreError> {
        let (cost, input_tokens, output_tokens) = extract_metrics(&event.kind);
        let kind_tag = kind_tag(&event.kind);
        let payload = serde_json::to_string(event).map_err(|e| {
            StoreError::Sqlite(rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
        })?;

        let timestamp_ms = event.timestamp.timestamp_millis();
        let agent = event.agent.display_name();

        // Returns 1 if inserted, 0 if a row with this id already exists.
        let inserted = self.conn.execute(
            "INSERT OR IGNORE INTO events
                (id, agent, session_id, timestamp_ms, kind, cost_microcents,
                 input_tokens, output_tokens, project, payload, source_offset)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                event.id.to_string(),
                agent,
                event.session_id,
                timestamp_ms,
                kind_tag,
                cost.as_u64() as i64,
                input_tokens as i64,
                output_tokens as i64,
                event.project,
                payload,
                event.source_offset.map(|o| o as i64),
            ],
        )?;

        if inserted == 1 {
            self.update_daily_totals(event, cost)?;
        }

        Ok(())
    }

    fn update_daily_totals(
        &self,
        event: &AgentEvent,
        cost: Microcents,
    ) -> Result<(), StoreError> {
        // Use the user's local TZ for daily aggregation (matches what the user
        // means by "today"). All future "today's spend" queries pass a LocalDay
        // computed the same way.
        let local_date = event
            .timestamp
            .with_timezone(&Local)
            .format("%Y-%m-%d")
            .to_string();
        let agent = event.agent.display_name();

        self.conn.execute(
            "INSERT INTO daily_totals (date_local, agent, cost_microcents, event_count)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(date_local, agent) DO UPDATE SET
                 cost_microcents = cost_microcents + excluded.cost_microcents,
                 event_count = event_count + 1",
            params![local_date, agent, cost.as_u64() as i64],
        )?;

        Ok(())
    }
}

fn extract_metrics(kind: &EventKind) -> (Microcents, u64, u64) {
    match kind {
        EventKind::ModelCall {
            cost_microcents,
            input_tokens,
            output_tokens,
            ..
        } => (*cost_microcents, *input_tokens, *output_tokens),
        _ => (Microcents::ZERO, 0, 0),
    }
}

fn kind_tag(kind: &EventKind) -> &'static str {
    match kind {
        EventKind::SessionStart { .. } => "session_start",
        EventKind::SessionEnd { .. } => "session_end",
        EventKind::ModelCall { .. } => "model_call",
        EventKind::ToolCall { .. } => "tool_call",
        EventKind::FileEdit { .. } => "file_edit",
        EventKind::UserMessage { .. } => "user_message",
        EventKind::AssistantMessage { .. } => "assistant_message",
        EventKind::Unknown { .. } => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentwatch_core::{Agent, EventKind, SessionEndReason};
    use chrono::Utc;
    use tempfile::TempDir;
    use uuid::Uuid;

    fn sample_model_call(id: Uuid) -> AgentEvent {
        AgentEvent {
            id,
            agent: Agent::ClaudeCode,
            session_id: "s1".into(),
            timestamp: Utc::now(),
            kind: EventKind::ModelCall {
                model: "claude-opus-4-7".into(),
                input_tokens: 1_000,
                output_tokens: 200,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_microcents: Microcents::from_cents(150),
                duration_ms: 3000,
            },
            project: Some("test-project".into()),
            source_offset: Some(0),
            raw: None,
        }
    }

    #[test]
    fn open_creates_schema() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("db.sqlite");
        let _w = Writer::open(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn insert_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let w = Writer::open(&tmp.path().join("db.sqlite")).unwrap();
        let event = sample_model_call(Uuid::new_v4());

        w.insert(&event).unwrap();
        w.insert(&event).unwrap();

        let count: i64 = w
            .conn
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1, "second insert should be a no-op");
    }

    #[test]
    fn daily_totals_accumulate() {
        let tmp = TempDir::new().unwrap();
        let w = Writer::open(&tmp.path().join("db.sqlite")).unwrap();

        w.insert(&sample_model_call(Uuid::new_v4())).unwrap();
        w.insert(&sample_model_call(Uuid::new_v4())).unwrap();
        w.insert(&sample_model_call(Uuid::new_v4())).unwrap();

        let total: i64 = w
            .conn
            .query_row(
                "SELECT cost_microcents FROM daily_totals WHERE agent = 'Claude Code'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // 3 × 150 cents = 450 cents = 45_000 microcents
        assert_eq!(total, 45_000);

        let events: i64 = w
            .conn
            .query_row(
                "SELECT event_count FROM daily_totals WHERE agent = 'Claude Code'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events, 3);
    }

    #[test]
    fn non_modelcall_events_have_zero_cost() {
        let tmp = TempDir::new().unwrap();
        let w = Writer::open(&tmp.path().join("db.sqlite")).unwrap();

        let event = AgentEvent {
            id: Uuid::new_v4(),
            agent: Agent::ClaudeCode,
            session_id: "s1".into(),
            timestamp: Utc::now(),
            kind: EventKind::SessionEnd {
                reason: SessionEndReason::Completed,
            },
            project: None,
            source_offset: None,
            raw: None,
        };

        w.insert(&event).unwrap();

        let cost: i64 = w
            .conn
            .query_row(
                "SELECT cost_microcents FROM events WHERE session_id = 's1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(cost, 0);
    }
}
