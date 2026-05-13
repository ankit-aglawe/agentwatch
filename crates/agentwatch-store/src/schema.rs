//! SQLite schema and migration scaffolding.
//!
//! The schema is intentionally small at v0.1 and indexed for the common
//! "today's spend" and "hourly buckets" queries.

pub const CREATE_EVENTS: &str = "
    CREATE TABLE IF NOT EXISTS events (
        id TEXT PRIMARY KEY,
        agent TEXT NOT NULL,
        session_id TEXT NOT NULL,
        timestamp_ms INTEGER NOT NULL,
        kind TEXT NOT NULL,
        cost_microcents INTEGER NOT NULL DEFAULT 0,
        input_tokens INTEGER NOT NULL DEFAULT 0,
        output_tokens INTEGER NOT NULL DEFAULT 0,
        project TEXT,
        payload JSON NOT NULL,
        source_offset INTEGER
    );
";

pub const CREATE_EVENTS_INDEX: &str = "
    CREATE INDEX IF NOT EXISTS events_ts_agent ON events(timestamp_ms, agent);
";

pub const CREATE_EVENTS_SESSION_INDEX: &str = "
    CREATE INDEX IF NOT EXISTS events_session_ts ON events(session_id, timestamp_ms);
";

pub const CREATE_EVENTS_PROJECT_INDEX: &str = "
    CREATE INDEX IF NOT EXISTS events_project_ts ON events(project, timestamp_ms);
";

/// Idempotent migration: add `project` column if the table predates it.
/// rusqlite will error if the column already exists; the writer catches that.
pub const ALTER_ADD_PROJECT: &str = "ALTER TABLE events ADD COLUMN project TEXT;";

/// Pre-aggregated daily totals. Updated by the writer on every event so the
/// 'today's spend' query hits one indexed row instead of scanning events.
pub const CREATE_DAILY_TOTALS: &str = "
    CREATE TABLE IF NOT EXISTS daily_totals (
        date_local TEXT NOT NULL,
        agent TEXT NOT NULL,
        cost_microcents INTEGER NOT NULL DEFAULT 0,
        event_count INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (date_local, agent)
    );
";

/// Per-source ingest progress. Lets us skip files whose mtime hasn't changed
/// since last ingest. The (agent, path) pair is unique.
pub const CREATE_SOURCE_STATE: &str = "
    CREATE TABLE IF NOT EXISTS source_state (
        agent TEXT NOT NULL,
        path TEXT NOT NULL,
        last_mtime_ms INTEGER NOT NULL,
        last_size_bytes INTEGER NOT NULL DEFAULT 0,
        last_ingested_at_ms INTEGER NOT NULL,
        PRIMARY KEY (agent, path)
    );
";

pub const PRAGMA_WAL: &str = "PRAGMA journal_mode = WAL;";
pub const PRAGMA_BUSY_TIMEOUT: &str = "PRAGMA busy_timeout = 5000;";
pub const PRAGMA_FOREIGN_KEYS: &str = "PRAGMA foreign_keys = ON;";
