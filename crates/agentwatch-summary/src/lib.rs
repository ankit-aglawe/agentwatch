//! Weekly / monthly markdown summary generator (cherry-pick E3).
//!
//! Second shareable artifact for the weekly-review crowd - distinct audience
//! from the live TUI HUD. The markdown output is screenshot-friendly and
//! sometimes paste-into-Slack-friendly.
//!
//! Optional `--vibe` mode generates a 3-sentence narrative via the user's
//! own SLM API key. We never bundle the call ourselves (no built-in spend).
//! Without an API key, `--vibe` degrades gracefully to a non-vibe summary.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SummaryError {
    #[error("store: {0}")]
    Store(#[from] agentwatch_store::StoreError),
}

pub enum Span {
    Week,
    Month,
}

pub struct SummaryConfig {
    pub span: Span,
    /// If true and user has API key in env, write a 3-sentence narrative.
    pub vibe: bool,
}

pub fn render(_config: SummaryConfig) -> Result<String, SummaryError> {
    // Day 10 work: pull aggregates from store, format markdown, optionally
    // call user's SLM for vibe paragraph.
    Ok(String::new())
}
