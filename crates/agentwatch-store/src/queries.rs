//! Read-side queries. Used by both the TUI and the webapp.
//!
//! All queries take an explicit `LocalDay` (or range) to make timezone handling
//! visible at the call site. The store itself never inspects `chrono::Local`.

use agentwatch_core::{Agent, Microcents};
use chrono::{DateTime, Utc};
use rusqlite::Connection;

use crate::StoreError;

/// A single calendar day in the user's local timezone, formatted YYYY-MM-DD.
/// All daily-rollup queries are keyed on this string.
#[derive(Debug, Clone)]
pub struct LocalDay(pub String);

impl LocalDay {
    pub fn today() -> Self {
        LocalDay(chrono::Local::now().format("%Y-%m-%d").to_string())
    }
}

pub struct Queries<'a> {
    pub conn: &'a Connection,
}

impl<'a> Queries<'a> {
    pub fn today_spend(&self, _day: &LocalDay) -> Result<Microcents, StoreError> {
        // Day 5 work: SELECT SUM(cost_microcents) FROM daily_totals WHERE date_local = ?
        Ok(Microcents::ZERO)
    }

    pub fn spend_by_agent(
        &self,
        _day: &LocalDay,
    ) -> Result<Vec<(Agent, Microcents)>, StoreError> {
        Ok(Vec::new())
    }

    pub fn recent_activity(
        &self,
        _limit: usize,
    ) -> Result<Vec<(DateTime<Utc>, Agent, String)>, StoreError> {
        Ok(Vec::new())
    }

    pub fn hot_files(
        &self,
        _day: &LocalDay,
        _limit: usize,
    ) -> Result<Vec<(String, u64)>, StoreError> {
        Ok(Vec::new())
    }
}
