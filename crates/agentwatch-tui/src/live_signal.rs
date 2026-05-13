//! Real-time "what is the user doing right now" signal.
//!
//! Harness tools (claude-mem, openclaw, hermes, etc.) typically observe the
//! primary coding agent's activity and write it to their own log files. By
//! scanning these, we get a near-instant signal of what the user is asking
//! their agent right now — faster than the main session log which only
//! updates after the assistant responds.
//!
//! Architecture: each harness gets its own `LiveSignalSource` implementation.
//! `read_latest()` polls them all and returns the freshest signal. Adding a
//! new harness = one new module + register in `SOURCES`.
//!
//! Signals older than 5 minutes are discarded — they're historical, not live.

use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct LiveSignal {
    pub timestamp: DateTime<Utc>,
    pub user_prompt: String,
    /// Which harness tool produced this signal — surfaced in the TUI banner.
    pub source: &'static str,
}

/// One harness tool's reader. Each implementation knows where its tool writes
/// and how to parse the most recent user-request from its log format.
pub trait LiveSignalSource: Sync {
    fn name(&self) -> &'static str;
    fn read(&self) -> Option<LiveSignal>;
}

/// The registry of supported harness tools. To add a new one (openclaw, hermes,
/// agentic-stack, AgentHandover, etc.) implement `LiveSignalSource` and add it
/// here.
fn sources() -> Vec<Box<dyn LiveSignalSource>> {
    vec![Box::new(ClaudeMemSource)]
}

/// Poll every registered source and return the freshest signal (if any).
pub fn read_latest() -> Option<LiveSignal> {
    sources()
        .iter()
        .filter_map(|s| s.read())
        .max_by_key(|s| s.timestamp)
}

// ============ claude-mem ============

struct ClaudeMemSource;

impl LiveSignalSource for ClaudeMemSource {
    fn name(&self) -> &'static str {
        "claude-mem"
    }
    fn read(&self) -> Option<LiveSignal> {
        let dir = claude_mem_dir()?;
        if !dir.exists() {
            return None;
        }
        let newest = newest_jsonl(&dir)?;
        let line = last_meaningful_line(&newest)?;
        parse_signal(&line)
    }
}

fn claude_mem_dir() -> Option<PathBuf> {
    let home = agentwatch_core::paths::home_dir()?;
    // claude-mem writes inside ~/.claude/projects with a directory name
    // derived from its install path. Both common variants are checked.
    let projects = home.join(".claude").join("projects");
    if !projects.exists() {
        return None;
    }
    // Look for any directory containing "claude-mem" in its name.
    let entries = std::fs::read_dir(&projects).ok()?;
    for entry in entries.flatten() {
        let p = entry.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.contains("claude-mem") && p.is_dir() {
            return Some(p);
        }
    }
    None
}

fn newest_jsonl(dir: &std::path::Path) -> Option<PathBuf> {
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata().ok()?.modified().ok()?;
        match &newest {
            Some((m, _)) if *m >= modified => {}
            _ => newest = Some((modified, path)),
        }
    }
    newest.map(|(_, p)| p)
}

/// Read the file backwards and return the last line that has a
/// `user_request` (skip queue-operation `dequeue` lines that have no content).
fn last_meaningful_line(path: &std::path::Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    for line in contents.lines().rev() {
        if line.contains("<user_request>") {
            return Some(line.to_string());
        }
    }
    None
}

fn parse_signal(line: &str) -> Option<LiveSignal> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    let timestamp = v
        .get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))?;
    let content = v.get("content").and_then(|c| c.as_str())?;
    let prompt = extract_user_request(content)?;
    // Only return signals less than 5 minutes old.
    if (Utc::now() - timestamp).num_seconds() > 300 {
        return None;
    }
    Some(LiveSignal {
        timestamp,
        user_prompt: prompt,
        source: "claude-mem",
    })
}

fn extract_user_request(content: &str) -> Option<String> {
    let start = content.find("<user_request>")?;
    let after = &content[start + "<user_request>".len()..];
    let end = after.find("</user_request>")?;
    let text = after[..end].trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}
