//! Cursor adapter.
//!
//! Source paths:
//!   - macOS: `~/Library/Application Support/Cursor/logs/`
//!   - Windows: `%APPDATA%\Cursor\logs\`
//!   - Linux: `~/.config/Cursor/logs/`
//!
//! Capability: partial - model + approximate tokens; per-tool-call data is
//! inconsistent across Cursor versions.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;
use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct CursorAdapter;

impl CursorAdapter {
    pub fn new() -> Self { Self }
}

impl Default for CursorAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for CursorAdapter {
    fn agent(&self) -> Agent { Agent::Cursor }
    fn capability(&self) -> Capability {
        Capability::Partial { missing: &["per_tool_call_breakdown", "exact_token_counts"] }
    }
    fn session_root(&self) -> Option<PathBuf> {
        let home = agentwatch_core::paths::home_dir()?;
        #[cfg(target_os = "macos")]
        {
            return Some(home.join("Library").join("Application Support").join("Cursor").join("logs"));
        }
        #[cfg(target_os = "linux")]
        {
            return Some(home.join(".config").join("Cursor").join("logs"));
        }
        #[cfg(target_os = "windows")]
        {
            return Some(home.join("AppData").join("Roaming").join("Cursor").join("logs"));
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            return None;
        }
    }
    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> { Ok(Vec::new()) }
    fn parse_line(&mut self, _source: &SourcePath, _line: &str, _offset: u64) -> Result<Vec<ParseResult>, AdapterError> {
        Ok(Vec::new())
    }
}
