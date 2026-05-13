//! Codex CLI adapter.
//!
//! Source: `~/.codex/sessions/`. Capability: full.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;
use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct CodexCliAdapter;

impl CodexCliAdapter {
    pub fn new() -> Self { Self }
}

impl Default for CodexCliAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for CodexCliAdapter {
    fn agent(&self) -> Agent { Agent::CodexCli }
    fn capability(&self) -> Capability { Capability::Full }
    fn session_root(&self) -> Option<PathBuf> {
        agentwatch_core::paths::home_dir().map(|h| h.join(".codex").join("sessions"))
    }
    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> { Ok(Vec::new()) }
    fn parse_line(&mut self, _source: &SourcePath, _line: &str, _offset: u64) -> Result<Vec<ParseResult>, AdapterError> {
        Ok(Vec::new())
    }
}
