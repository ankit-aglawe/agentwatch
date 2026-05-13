//! SQLite-backed event storage for agentwatch.
//!
//! Invariants enforced here:
//! - **#3 Disk-full handling.** Writer surfaces ENOSPC as `StoreError::DiskFull`
//!   so the TUI can show a visible banner instead of silently corrupting.
//! - **#5 Single-writer process model.** A PID lock file in the data dir
//!   guarantees only one writer per host. A second `agentwatch` invocation
//!   attaches read-only or refuses with a clear message.
//!
//! Performance choices (locked in PLAN.md):
//! - WAL mode + `busy_timeout = 5000`.
//! - Composite index on `(timestamp, agent)` for time-range queries.
//! - Daily-aggregate rollup table for sub-100ms "today's spend" queries at
//!   1M+ event scale.

pub mod pid_lock;
pub mod queries;
pub mod reader;
pub mod schema;
pub mod writer;

pub use reader::{
    AgentLiveStatus, AgentSummary, Breakdown, DailyCost, LatestActivity, LiveMetrics, Reader,
    RecentEvent, SessionStatus, SessionSummary, TodaySummary, TokenBucket, WindowTokens,
};
pub use writer::Writer;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("disk full — pausing writes")]
    DiskFull,
    #[error("another agentwatch instance is already writing (pid {pid})")]
    AlreadyLocked { pid: u32 },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub const SCHEMA_VERSION: u32 = 1;
