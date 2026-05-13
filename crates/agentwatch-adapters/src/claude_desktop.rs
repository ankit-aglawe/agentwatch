//! Claude Desktop adapter.
//!
//! Distinct from Claude Code CLI. The Anthropic claude.ai desktop app with skills
//! and MCP support. Session logs at:
//!   - macOS: `~/Library/Application Support/Claude/sessions/`
//!   - Windows: `%APPDATA%\Claude\sessions\`
//!   - Linux: `~/.config/Claude/sessions/`
//!
//! Capability: TBD pending first-launch inspection of the JSON shape. Conservative
//! default of `ModelOnly` until we verify what's exposed.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;

use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct ClaudeDesktopAdapter;

impl ClaudeDesktopAdapter {
    pub fn new() -> Self { Self }
}

impl Default for ClaudeDesktopAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for ClaudeDesktopAdapter {
    fn agent(&self) -> Agent { Agent::ClaudeDesktop }
    fn capability(&self) -> Capability { Capability::ModelOnly }
    fn session_root(&self) -> Option<PathBuf> {
        let home = agentwatch_core::paths::home_dir()?;
        #[cfg(target_os = "macos")]
        {
            return Some(home.join("Library").join("Application Support").join("Claude").join("sessions"));
        }
        #[cfg(target_os = "linux")]
        {
            return Some(home.join(".config").join("Claude").join("sessions"));
        }
        #[cfg(target_os = "windows")]
        {
            // %APPDATA% resolution is handled by directories-next normally; this
            // is a coarse approximation that's good enough for detection.
            return Some(home.join("AppData").join("Roaming").join("Claude").join("sessions"));
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
