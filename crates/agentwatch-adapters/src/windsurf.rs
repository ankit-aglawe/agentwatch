//! Windsurf adapter.
//!
//! Source: `~/.codeium/windsurf/`. Capability: full where exposed.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;
use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct WindsurfAdapter;

impl WindsurfAdapter {
    pub fn new() -> Self { Self }
}

impl Default for WindsurfAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for WindsurfAdapter {
    fn agent(&self) -> Agent { Agent::Windsurf }
    fn capability(&self) -> Capability { Capability::Full }
    fn session_root(&self) -> Option<PathBuf> {
        agentwatch_core::paths::home_dir().map(|h| h.join(".codeium").join("windsurf"))
    }
    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> { Ok(Vec::new()) }
    fn parse_line(&mut self, _source: &SourcePath, _line: &str, _offset: u64) -> Result<Vec<ParseResult>, AdapterError> {
        Ok(Vec::new())
    }
}
