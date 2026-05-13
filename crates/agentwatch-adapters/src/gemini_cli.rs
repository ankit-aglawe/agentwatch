//! Gemini CLI adapter.
//!
//! Google's `gemini-cli`. Session logs typically at:
//!   - `~/.gemini/sessions/`
//!   - or `~/.config/gemini-cli/sessions/` depending on install
//!
//! We detect against `~/.gemini/sessions/` first; future versions can probe
//! multiple candidates.

use agentwatch_core::{Agent, Capability};
use std::path::PathBuf;
use crate::{Adapter, AdapterError, ParseResult, SourcePath};

pub struct GeminiCliAdapter;

impl GeminiCliAdapter {
    pub fn new() -> Self { Self }
}

impl Default for GeminiCliAdapter {
    fn default() -> Self { Self::new() }
}

impl Adapter for GeminiCliAdapter {
    fn agent(&self) -> Agent { Agent::GeminiCli }
    fn capability(&self) -> Capability { Capability::Full }
    fn session_root(&self) -> Option<PathBuf> {
        agentwatch_core::paths::home_dir().map(|h| h.join(".gemini").join("sessions"))
    }
    fn discover_sources(&self) -> Result<Vec<SourcePath>, AdapterError> { Ok(Vec::new()) }
    fn parse_line(&mut self, _source: &SourcePath, _line: &str, _offset: u64) -> Result<Vec<ParseResult>, AdapterError> {
        Ok(Vec::new())
    }
}
