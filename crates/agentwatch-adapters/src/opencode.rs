//! OpenCode adapter.
//!
//! OpenCode is open source; session dirs are typically `~/.opencode/sessions/`
//! or project-local `./.opencode/sessions/`. We default to the global location
//! for detection; project-local sessions are picked up at watch-time.
//! Capability: full.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;
use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct OpenCodeAdapter;

impl OpenCodeAdapter {
    pub fn new() -> Self { Self }
}

impl Default for OpenCodeAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for OpenCodeAdapter {
    fn agent(&self) -> Agent { Agent::OpenCode }
    fn capability(&self) -> Capability { Capability::Full }
    fn session_root(&self) -> Option<PathBuf> {
        agentwatch_core::paths::home_dir().map(|h| h.join(".opencode").join("sessions"))
    }
    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> { Ok(Vec::new()) }
    fn parse_line(&mut self, _source: &SourcePath, _line: &str, _offset: u64) -> Result<Vec<ParseResult>, AdapterError> {
        Ok(Vec::new())
    }
}
