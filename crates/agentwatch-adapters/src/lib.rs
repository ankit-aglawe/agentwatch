//! Per-agent adapters that turn local log files into `AgentEvent` streams.
//!
//! Every adapter implements the `Adapter` trait. New agents are added by
//! creating a new module here and adding the variant to `agentwatch-core`'s
//! `Agent` enum.
//!
//! Streaming contract (Invariant #2): adapters MUST parse line-by-line via
//! `BufReader`, never `fs::read_to_string`. Session log files can be GB-scale.

use agentwatch_core::{Agent, AgentEvent, Capability};
use chrono::{DateTime, Utc};
use std::path::PathBuf;
use thiserror::Error;

pub mod claude_code;
pub mod claude_desktop;
pub mod codex_cli;
pub mod cursor;
pub mod gemini_cli;
pub mod opencode;
pub mod windsurf;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("no log directory found for {agent:?}")]
    LogDirMissing { agent: Agent },
}

/// A source the adapter watches - typically a single session log file.
#[derive(Debug, Clone)]
pub struct SourcePath {
    pub path: PathBuf,
    pub session_id: String,
}

/// The outcome of parsing one line from a source.
#[derive(Debug)]
pub enum ParseResult {
    /// A normalized event ready to land in storage.
    Event(AgentEvent),
    /// An intentionally-skipped line (e.g. heartbeats, blank lines).
    Skip { reason: SkipReason },
    /// A line the adapter could not recognize. Signals parser drift; logged
    /// loudly so we know to update the parser.
    UnknownLine { raw: String, hint: Option<String> },
}

#[derive(Debug, Clone, Copy)]
pub enum SkipReason {
    Heartbeat,
    Blank,
    Comment,
    DuplicateAfterRetry,
}

/// What `agentwatch doctor` and the first-run flow learn about each adapter.
#[derive(Debug, Clone)]
pub struct DetectionResult {
    pub agent: Agent,
    pub capability: Capability,
    /// The path we scanned (if any) - printed in the first-run table.
    pub session_root: Option<PathBuf>,
    /// Number of session files modified in the last 30 days.
    pub session_count_30d: u32,
    /// Most recent activity, if any.
    pub last_activity: Option<DateTime<Utc>>,
    /// One of: Active | InstalledOnly | NotDetected.
    pub status: DetectionStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionStatus {
    /// Installed AND has session activity in last 30 days. Track by default.
    Active,
    /// Path exists but no recent sessions. Standby - auto-track once activity appears.
    InstalledOnly,
    /// Neither binary nor session dir found.
    NotDetected,
}

impl DetectionResult {
    pub fn glyph(&self) -> &'static str {
        match self.status {
            DetectionStatus::Active => "●",
            DetectionStatus::InstalledOnly => "◐",
            DetectionStatus::NotDetected => "◯",
        }
    }
}

/// The contract every per-agent adapter implements.
///
/// Adapters are stateful - they remember byte offsets per source so they can
/// resume after restart without re-emitting events. Adapters MUST be `Send`
/// so they can move between threads in the watcher pool.
pub trait Adapter: Send {
    fn agent(&self) -> Agent;
    fn capability(&self) -> Capability;

    /// Where this adapter looks for session logs on this machine.
    /// Returns `None` when the OS-specific path is unknown.
    fn session_root(&self) -> Option<PathBuf>;

    /// Default detection: walk session_root and count session files modified
    /// in the last 30 days. Adapters can override for finer-grained logic.
    fn detect(&self) -> DetectionResult {
        let root = self.session_root();
        let (status, count, last) = match root.as_ref() {
            Some(p) if p.exists() => scan_session_root(p),
            _ => (DetectionStatus::NotDetected, 0, None),
        };
        DetectionResult {
            agent: self.agent(),
            capability: self.capability(),
            session_root: root,
            session_count_30d: count,
            last_activity: last,
            status,
        }
    }

    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError>;

    /// Parse one buffered line. Streaming - adapter never reads whole files.
    ///
    /// Returns 0 or more `ParseResult`s for a single input line. A Claude Code
    /// assistant message, for example, can produce one `ModelCall` and several
    /// `ToolCall` events in one source line.
    fn parse_line(
        &mut self,
        source: &SourcePath,
        line: &str,
        offset: u64,
    ) -> Result<Vec<ParseResult>, AdapterError>;
}

/// Walk a session directory and count files modified in the last 30 days.
/// Returns (status, count, most_recent).
fn scan_session_root(root: &std::path::Path) -> (DetectionStatus, u32, Option<DateTime<Utc>>) {
    let cutoff = Utc::now() - chrono::Duration::days(30);
    let mut count = 0u32;
    let mut latest: Option<DateTime<Utc>> = None;

    // Walk one level deep - adapters typically organize as <root>/<project>/<session>.jsonl
    // or <root>/<session>. Walking depth-2 captures both shapes.
    for top in walk_one_level(root) {
        if top.is_file() {
            consider(&top, cutoff, &mut count, &mut latest);
        } else if top.is_dir() {
            for inner in walk_one_level(&top) {
                if inner.is_file() {
                    consider(&inner, cutoff, &mut count, &mut latest);
                }
            }
        }
    }

    let status = if count > 0 {
        DetectionStatus::Active
    } else {
        DetectionStatus::InstalledOnly
    };

    (status, count, latest)
}

fn walk_one_level(dir: &std::path::Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|iter| {
            iter.filter_map(Result::ok)
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default()
}

fn consider(
    path: &std::path::Path,
    cutoff: DateTime<Utc>,
    count: &mut u32,
    latest: &mut Option<DateTime<Utc>>,
) {
    let modified = match path.metadata().and_then(|m| m.modified()) {
        Ok(m) => DateTime::<Utc>::from(m),
        Err(_) => return,
    };
    if modified >= cutoff {
        *count += 1;
        match latest {
            Some(prev) if *prev >= modified => {}
            _ => *latest = Some(modified),
        }
    }
}

/// Build the canonical list of adapters and ask each one to detect itself.
///
/// `agentwatch doctor` and the first-run flow both call this.
pub fn discover() -> Vec<DetectionResult> {
    let adapters: Vec<Box<dyn Adapter>> = vec![
        Box::new(claude_code::ClaudeCodeAdapter::new()),
        Box::new(claude_desktop::ClaudeDesktopAdapter::new()),
        Box::new(codex_cli::CodexCliAdapter::new()),
        Box::new(cursor::CursorAdapter::new()),
        Box::new(gemini_cli::GeminiCliAdapter::new()),
        Box::new(windsurf::WindsurfAdapter::new()),
        Box::new(opencode::OpenCodeAdapter::new()),
    ];
    adapters.iter().map(|a| a.detect()).collect()
}
