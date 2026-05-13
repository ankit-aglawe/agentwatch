//! Read-only access to the SQLite store.
//!
//! Used by `agentwatch status`, the webapp, `agentwatch report`, and any other
//! consumer that needs to look at events without owning the writer.

use std::path::Path;

use agentwatch_core::Microcents;
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OpenFlags};

use crate::StoreError;

pub struct Reader {
    conn: Connection,
}

impl Reader {
    /// Open the DB read-only. Will succeed even if no writer has touched the file
    /// yet — we treat missing tables as "no events" so `status` works on day 0.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        Ok(Self { conn })
    }

    /// Latest tracked event across all agents, or `None` if the DB is empty
    /// or the `events` table doesn't exist yet.
    pub fn latest(&self) -> Result<Option<LatestActivity>, StoreError> {
        let row = self.conn.query_row(
            "SELECT agent, session_id, timestamp_ms, kind, payload
             FROM events
             ORDER BY timestamp_ms DESC
             LIMIT 1",
            [],
            |row| {
                Ok(LatestActivity {
                    agent: row.get::<_, String>(0)?,
                    session_id: row.get::<_, String>(1)?,
                    timestamp_ms: row.get::<_, i64>(2)?,
                    kind: row.get::<_, String>(3)?,
                    payload: row.get::<_, String>(4)?,
                })
            },
        );
        match row {
            Ok(r) => Ok(Some(r)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                Ok(None)
            }
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Count of events landed today in the user's local timezone.
    pub fn today_event_count(&self) -> Result<u64, StoreError> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let result = self.conn.query_row(
            "SELECT COALESCE(SUM(event_count), 0) FROM daily_totals WHERE date_local = ?1",
            [&today],
            |row| row.get::<_, i64>(0),
        );
        match result {
            Ok(n) => Ok(n.max(0) as u64),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                Ok(0)
            }
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// AI-specific metrics for the TUI summary line — burn rate, cache hit
    /// rate, I/O ratio, average latency. Computed against the last `minutes`
    /// minutes (typically 30) for a stable signal.
    pub fn live_metrics(&self, minutes: i64) -> Result<LiveMetrics, StoreError> {
        let start_ms =
            (chrono::Utc::now() - chrono::Duration::minutes(minutes)).timestamp_millis();
        let mut m = LiveMetrics::default();
        // Single query for all the sums.
        let result = self.conn.query_row(
            "SELECT
                COALESCE(SUM(input_tokens), 0),
                COALESCE(SUM(output_tokens), 0),
                COALESCE(SUM(input_tokens + output_tokens), 0),
                COUNT(*),
                COALESCE(SUM(json_extract(payload, '$.kind.cache_read_tokens')), 0),
                COALESCE(SUM(json_extract(payload, '$.kind.cache_write_tokens')), 0),
                COALESCE(SUM(json_extract(payload, '$.kind.duration_ms')), 0)
             FROM events
             WHERE kind = 'model_call' AND timestamp_ms >= ?1",
            [start_ms],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?.max(0) as u64,
                    row.get::<_, i64>(1)?.max(0) as u64,
                    row.get::<_, i64>(2)?.max(0) as u64,
                    row.get::<_, i64>(3)?.max(0) as u64,
                    row.get::<_, i64>(4)?.max(0) as u64,
                    row.get::<_, i64>(5)?.max(0) as u64,
                    row.get::<_, i64>(6)?.max(0) as u64,
                ))
            },
        );
        match result {
            Ok((input, output, total, n, cache_read, cache_write, total_duration_ms)) => {
                m.input_tokens = input;
                m.output_tokens = output;
                m.cache_read_tokens = cache_read;
                m.cache_write_tokens = cache_write;
                m.model_calls = n;
                m.tokens_per_minute = if minutes > 0 { total / minutes as u64 } else { 0 };
                m.tokens_per_second = m.tokens_per_minute / 60;
                // Cache hit rate = cache_read / (cache_read + raw input).
                let denom = cache_read + input;
                m.cache_hit_rate = if denom > 0 {
                    (cache_read as f32 / denom as f32) * 100.0
                } else {
                    0.0
                };
                // I/O ratio: input vs output. 1:0.05 means heavy context.
                m.io_ratio_output_per_input = if input > 0 {
                    output as f32 / input as f32
                } else {
                    0.0
                };
                // Avg latency per model call.
                m.avg_latency_ms = if n > 0 { total_duration_ms / n } else { 0 };
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such")) => {}
            Err(e) => return Err(StoreError::Sqlite(e)),
        }
        Ok(m)
    }

    /// Rich snapshot for the TUI front page. One query trip; safe on missing tables.
    pub fn today_summary(&self) -> Result<TodaySummary, StoreError> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let start_ms = local_day_start_ms(&today);
        let end_ms = start_ms + 86_400_000;

        let mut summary = TodaySummary::default();

        // Daily totals across agents.
        let totals_result = self.conn.query_row(
            "SELECT COALESCE(SUM(cost_microcents), 0),
                    COALESCE(SUM(event_count), 0),
                    COALESCE(COUNT(DISTINCT agent), 0)
             FROM daily_totals
             WHERE date_local = ?1",
            [&today],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        );
        match totals_result {
            Ok((cost, events, agents)) => {
                summary.total_cost = Microcents(cost.max(0) as u64);
                summary.event_count = events.max(0) as u64;
                summary.active_agents = agents.max(0) as u32;
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => {}
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) => {}
            Err(e) => return Err(StoreError::Sqlite(e)),
        }

        // Per-event-kind counts since midnight local.
        let mut tool_calls = 0u64;
        let mut model_calls = 0u64;
        let mut user_messages = 0u64;
        let mut files_touched = 0u64;
        let mut input_tokens_today = 0u64;
        let mut output_tokens_today = 0u64;

        let kind_query = self.conn.prepare(
            "SELECT kind, COUNT(*) FROM events
             WHERE timestamp_ms >= ?1 AND timestamp_ms < ?2
             GROUP BY kind",
        );
        match kind_query {
            Ok(mut stmt) => {
                let rows = stmt.query_map([start_ms, end_ms], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                });
                if let Ok(rows) = rows {
                    for row in rows.flatten() {
                        let n = row.1.max(0) as u64;
                        match row.0.as_str() {
                            "tool_call" => tool_calls = n,
                            "model_call" => model_calls = n,
                            "user_message" => user_messages = n,
                            "file_edit" => files_touched = n,
                            _ => {}
                        }
                    }
                }
            }
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) => {}
            Err(e) => return Err(StoreError::Sqlite(e)),
        }

        // Token totals from model_call payloads.
        let token_query = self.conn.prepare(
            "SELECT COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0)
             FROM events
             WHERE timestamp_ms >= ?1 AND timestamp_ms < ?2",
        );
        match token_query {
            Ok(mut stmt) => {
                let row = stmt.query_row([start_ms, end_ms], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?))
                });
                if let Ok((inp, out)) = row {
                    input_tokens_today = inp.max(0) as u64;
                    output_tokens_today = out.max(0) as u64;
                }
            }
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) => {}
            Err(e) => return Err(StoreError::Sqlite(e)),
        }

        // Distinct file_edit paths today (counts unique files, not edit count).
        let files_query = self.conn.query_row::<i64, _, _>(
            "SELECT COUNT(DISTINCT json_extract(payload, '$.kind.path'))
             FROM events
             WHERE kind = 'file_edit' AND timestamp_ms >= ?1 AND timestamp_ms < ?2",
            [start_ms, end_ms],
            |row| row.get(0),
        );
        if let Ok(n) = files_query {
            summary.distinct_files_touched = n.max(0) as u64;
        }

        summary.tool_calls = tool_calls;
        summary.model_calls = model_calls;
        summary.user_messages = user_messages;
        summary.file_edit_events = files_touched;
        summary.input_tokens = input_tokens_today;
        summary.output_tokens = output_tokens_today;

        Ok(summary)
    }

    /// Token totals for the rolling last N days for one agent.
    /// Used for the "this week" quota line in the cost insights panel.
    pub fn tokens_in_last_days(
        &self,
        agent_display_name: &str,
        days: i64,
    ) -> Result<WindowTokens, StoreError> {
        let start_ms =
            (chrono::Utc::now() - chrono::Duration::days(days)).timestamp_millis();
        let end_ms = chrono::Utc::now().timestamp_millis();
        self.tokens_in_window(agent_display_name, start_ms, end_ms)
    }

    /// Peak total context size (input + cache_read + cache_write) for a
    /// session. Claude Code's `input_tokens` is just the NEW input for that
    /// turn — true "context fill" requires summing the cached portions too.
    pub fn session_peak_context(&self, session_id: &str) -> Result<(u64, String), StoreError> {
        let q = self.conn.query_row(
            "SELECT COALESCE(MAX(
                       input_tokens
                     + COALESCE(json_extract(payload, '$.kind.cache_read_tokens'), 0)
                     + COALESCE(json_extract(payload, '$.kind.cache_write_tokens'), 0)
                   ), 0),
                   COALESCE(MAX(json_extract(payload, '$.kind.model')), '?')
             FROM events
             WHERE session_id = ?1 AND kind = 'model_call'",
            [session_id],
            |row| Ok((row.get::<_, i64>(0)?.max(0) as u64, row.get::<_, String>(1)?)),
        );
        match q {
            Ok(t) => Ok(t),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok((0, "?".into())),
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such")) =>
            {
                Ok((0, "?".into()))
            }
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Sum tokens for one agent across a time window (milliseconds, UTC).
    /// Used to compute the rate-limit headline ("About X hours left").
    pub fn tokens_in_window(
        &self,
        agent_display_name: &str,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<WindowTokens, StoreError> {
        let result = self.conn.query_row(
            "SELECT COALESCE(SUM(input_tokens), 0),
                    COALESCE(SUM(output_tokens), 0),
                    COALESCE(COUNT(*), 0)
             FROM events
             WHERE agent = ?1 AND timestamp_ms >= ?2 AND timestamp_ms < ?3",
            params![agent_display_name, start_ms, end_ms],
            |row| {
                Ok(WindowTokens {
                    input: row.get::<_, i64>(0)?.max(0) as u64,
                    output: row.get::<_, i64>(1)?.max(0) as u64,
                    event_count: row.get::<_, i64>(2)?.max(0) as u64,
                })
            },
        );
        match result {
            Ok(t) => Ok(t),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(WindowTokens::default()),
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                Ok(WindowTokens::default())
            }
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Total cost spent today across all agents.
    pub fn today_cost(&self) -> Result<Microcents, StoreError> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let result = self.conn.query_row(
            "SELECT COALESCE(SUM(cost_microcents), 0) FROM daily_totals WHERE date_local = ?1",
            [&today],
            |row| row.get::<_, i64>(0),
        );
        match result {
            Ok(n) => Ok(Microcents(n.max(0) as u64)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(Microcents::ZERO),
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                Ok(Microcents::ZERO)
            }
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Most recent N events across all agents, newest first.
    pub fn recent_events(&self, limit: usize) -> Result<Vec<RecentEvent>, StoreError> {
        let mut out = Vec::with_capacity(limit);
        let mut stmt = match self.conn.prepare(
            "SELECT agent, timestamp_ms, kind, payload, input_tokens, output_tokens
             FROM events
             ORDER BY timestamp_ms DESC
             LIMIT ?1",
        ) {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                return Ok(out);
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let rows = stmt.query_map([limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? + row.get::<_, i64>(5)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (agent, ts_ms, kind, payload, tokens) = row;
                let (verb, target) = derive_verb(kind.as_str(), payload.as_str());
                out.push(RecentEvent {
                    timestamp: DateTime::<Utc>::from_timestamp_millis(ts_ms)
                        .unwrap_or(DateTime::<Utc>::UNIX_EPOCH),
                    agent,
                    kind,
                    verb,
                    target,
                    tokens: tokens.max(0) as u64,
                });
            }
        }
        Ok(out)
    }

    /// Token buckets over a sliding window for sparkline rendering.
    /// Oldest bucket first, current bucket last.
    pub fn token_sparkline(
        &self,
        agent_display_name: &str,
        num_buckets: usize,
        bucket_minutes: i64,
    ) -> Result<Vec<TokenBucket>, StoreError> {
        let bucket_ms = bucket_minutes * 60 * 1000;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let oldest_start = now_ms - bucket_ms * num_buckets as i64;
        let mut buckets: Vec<TokenBucket> = (0..num_buckets)
            .map(|i| TokenBucket {
                start_ms: oldest_start + (i as i64) * bucket_ms,
                ..Default::default()
            })
            .collect();
        let stmt_result = self.conn.prepare(
            "SELECT timestamp_ms, input_tokens, output_tokens FROM events
             WHERE agent = ?1 AND timestamp_ms >= ?2 AND timestamp_ms < ?3",
        );
        let mut stmt = match stmt_result {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                return Ok(buckets);
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let rows = stmt.query_map(params![agent_display_name, oldest_start, now_ms], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
            ))
        });
        if let Ok(rows) = rows {
            for r in rows.flatten() {
                let (ts, inp, outp) = r;
                let idx = ((ts - oldest_start) / bucket_ms) as usize;
                if idx < buckets.len() {
                    buckets[idx].tokens += (inp.max(0) + outp.max(0)) as u64;
                    buckets[idx].events += 1;
                }
            }
        }
        Ok(buckets)
    }

    /// Per-tool breakdown of tool_call events in the last `hours` hours.
    pub fn tool_breakdown(
        &self,
        hours: i64,
        limit: usize,
    ) -> Result<Vec<Breakdown>, StoreError> {
        self.breakdown_inner(
            "tool_call",
            "$.kind.tool",
            hours,
            limit,
            true, // order by count
        )
    }

    /// Per-model breakdown of model_call events in the last `hours` hours.
    /// Filters out the `<synthetic>` placeholder model used by claude-mem and
    /// other observer-style tools — that's not a real model the user picked.
    pub fn model_breakdown(
        &self,
        hours: i64,
        limit: usize,
    ) -> Result<Vec<Breakdown>, StoreError> {
        let mut all = self.breakdown_inner(
            "model_call",
            "$.kind.model",
            hours,
            limit + 4,
            false,
        )?;
        all.retain(|b| !is_synthetic_model(&b.label));
        all.truncate(limit);
        Ok(all)
    }

    /// Hot files: top N files by edit count in the last `hours` hours.
    pub fn hot_files_today(
        &self,
        hours: i64,
        limit: usize,
    ) -> Result<Vec<Breakdown>, StoreError> {
        self.breakdown_inner(
            "file_edit",
            "$.kind.path",
            hours,
            limit,
            true,
        )
    }

    /// Daily cost totals for the last N days (newest first).
    /// Used for the "today vs yesterday vs 7d avg" comparison and the trend sparkline.
    pub fn daily_cost_series(&self, days: u32) -> Result<Vec<DailyCost>, StoreError> {
        let mut out = Vec::with_capacity(days as usize);
        let today = chrono::Local::now().date_naive();
        for d in 0..days {
            let date = today - chrono::Duration::days(d as i64);
            let date_str = date.format("%Y-%m-%d").to_string();
            let q = self.conn.query_row(
                "SELECT COALESCE(SUM(cost_microcents), 0),
                        COALESCE(SUM(event_count), 0)
                 FROM daily_totals WHERE date_local = ?1",
                [&date_str],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
            );
            let (cost, events) = match q {
                Ok(t) => t,
                Err(rusqlite::Error::QueryReturnedNoRows) => (0, 0),
                Err(rusqlite::Error::SqliteFailure(_, msg))
                    if msg.as_deref().is_some_and(|m| m.contains("no such")) =>
                {
                    (0, 0)
                }
                Err(e) => return Err(StoreError::Sqlite(e)),
            };
            out.push(DailyCost {
                date_local: date_str,
                cost_microcents: Microcents(cost.max(0) as u64),
                event_count: events.max(0) as u64,
            });
        }
        Ok(out)
    }

    /// Per-project breakdown over the last `hours` hours. Uses the indexed
    /// `project` column directly — much faster than json_extract.
    pub fn project_breakdown(
        &self,
        hours: i64,
        limit: usize,
    ) -> Result<Vec<Breakdown>, StoreError> {
        let start_ms =
            (chrono::Utc::now() - chrono::Duration::hours(hours)).timestamp_millis();
        let q = self.conn.prepare(
            "SELECT project,
                    COUNT(*) AS count,
                    COALESCE(SUM(cost_microcents), 0) AS cost_microcents
             FROM events
             WHERE project IS NOT NULL AND timestamp_ms >= ?1
             GROUP BY project
             ORDER BY cost_microcents DESC, count DESC
             LIMIT ?2",
        );
        let mut stmt = match q {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such")) =>
            {
                return Ok(Vec::new())
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let rows = stmt.query_map([start_ms, limit as i64], |r| {
            let label: Option<String> = r.get(0)?;
            let count: i64 = r.get(1)?;
            let cost: i64 = r.get(2)?;
            Ok((label.unwrap_or_else(|| "?".into()), count.max(0) as u64, cost.max(0) as u64))
        });
        let mut out = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                out.push(Breakdown {
                    label: row.0,
                    count: row.1,
                    tokens: row.2, // re-use the tokens field for cost in microcents
                });
            }
        }
        Ok(out)
    }

    /// Per-session summary, newest activity first. The htop "process list."
    pub fn recent_sessions(&self, limit: usize) -> Result<Vec<SessionSummary>, StoreError> {
        let mut out = Vec::new();
        let q = self.conn.prepare(
            "SELECT session_id, agent,
                    MAX(timestamp_ms) AS last_ts,
                    MIN(timestamp_ms) AS first_ts,
                    COALESCE(SUM(input_tokens + output_tokens), 0) AS tokens,
                    COALESCE(SUM(cost_microcents), 0) AS cost,
                    COUNT(*) AS event_count,
                    MAX(project) AS project
             FROM events
             GROUP BY session_id, agent
             ORDER BY last_ts DESC
             LIMIT ?1",
        );
        let mut stmt = match q {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such")) =>
            {
                return Ok(out)
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let now = chrono::Utc::now().timestamp_millis();
        let rows = stmt.query_map([limit as i64], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<String>>(7)?,
            ))
        });
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                // Status thresholds — chosen for human glance:
                //   < 60s   = ACTIVE (typing in it right now)
                //   < 30min = idle (paused; might come back)
                //   else    = done (probably abandoned/finished)
                // LOOPING is applied after this and overrides.
                let age_ms = (now - row.2).max(0);
                let status = if age_ms < 60_000 {
                    SessionStatus::Active
                } else if age_ms < 30 * 60_000 {
                    SessionStatus::Idle
                } else {
                    SessionStatus::Done
                };
                // Find the most recent model for this session (best-effort).
                let model = self
                    .latest_model_for_session(&row.0)
                    .unwrap_or_else(|_| "—".to_string());
                let peak_context = self
                    .session_peak_context(&row.0)
                    .map(|(t, _)| t)
                    .unwrap_or(0);
                out.push(SessionSummary {
                    session_id: row.0,
                    agent: row.1,
                    model,
                    project: row.7.unwrap_or_else(|| "—".into()),
                    last_event_ms: row.2,
                    first_event_ms: row.3,
                    total_tokens: row.4.max(0) as u64,
                    total_cost: Microcents(row.5.max(0) as u64),
                    event_count: row.6.max(0) as u64,
                    status,
                    looping_score: 0,
                    peak_context,
                });
            }
        }

        // Looping detection — conservative, only flag genuine stuck patterns.
        // See `session_looping_score` for the criteria. Threshold of 6+ to
        // avoid flagging normal iterative debugging.
        for session in out.iter_mut() {
            if let Ok(score) = self.session_looping_score(&session.session_id) {
                if score >= 6 {
                    session.looping_score = score;
                    session.status = SessionStatus::Looping;
                }
            }
        }
        Ok(out)
    }

    fn latest_model_for_session(&self, session_id: &str) -> Result<String, StoreError> {
        let q = self.conn.query_row(
            "SELECT json_extract(payload, '$.kind.model')
             FROM events
             WHERE session_id = ?1 AND kind = 'model_call'
             ORDER BY timestamp_ms DESC LIMIT 1",
            [session_id],
            |row| row.get::<_, Option<String>>(0),
        );
        match q {
            Ok(Some(m)) => Ok(m),
            Ok(None) => Ok("—".to_string()),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok("—".to_string()),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Returns the max repetition count of any read-style (tool, target) pair
    /// observed in the last 90 seconds — a tight window typical of genuine
    /// "stuck in a loop" patterns.
    ///
    /// Heuristic guards against false positives:
    ///   - Excludes Edit / Write / MultiEdit — these legitimately repeat
    ///     during iterative refactoring; we can't see the diff to know if
    ///     the content actually changed.
    ///   - Requires `target` to be non-null — null-target tools (like
    ///     TaskUpdate) aren't loop signals.
    ///   - Tight window (90s) — real loops fire in bursts, not over 5 min.
    ///   - Threshold of 6+ at the call site — distinguishes "Claude looked
    ///     at this twice while debugging" from "Claude is stuck."
    ///
    /// Note: this is intentionally conservative. Better to miss some real
    /// loops than to wrongly flag productive sessions.
    fn session_looping_score(&self, session_id: &str) -> Result<u32, StoreError> {
        let cutoff =
            (chrono::Utc::now() - chrono::Duration::seconds(90)).timestamp_millis();
        let q = self.conn.query_row(
            "SELECT COALESCE(MAX(cnt), 0) FROM (
               SELECT COUNT(*) AS cnt FROM events
               WHERE session_id = ?1
                 AND kind = 'tool_call'
                 AND timestamp_ms >= ?2
                 AND json_extract(payload, '$.kind.target') IS NOT NULL
                 AND json_extract(payload, '$.kind.tool') NOT IN
                     ('Edit', 'Write', 'MultiEdit', 'NotebookEdit')
               GROUP BY json_extract(payload, '$.kind.tool'),
                        json_extract(payload, '$.kind.target')
             )",
            params![session_id, cutoff],
            |row| row.get::<_, i64>(0),
        );
        match q {
            Ok(n) => Ok(n.max(0) as u32),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Per-agent summary for the top-of-screen bars. One row per detected agent
    /// (even if currently inactive — INACTIVE status).
    pub fn agent_summary(&self, hours_5h: i64) -> Result<Vec<AgentSummary>, StoreError> {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let start_5h = now_ms - hours_5h * 3600 * 1000;
        let start_5m = now_ms - 5 * 60 * 1000;

        // Discover the set of agents we know about (from adapters via doctor —
        // but for now we just SELECT DISTINCT from events).
        let q = self.conn.prepare(
            "SELECT agent,
                    COALESCE(SUM(input_tokens + output_tokens), 0) AS tokens_5h,
                    COALESCE(SUM(cost_microcents), 0) AS cost_5h,
                    COUNT(*) AS events_5h,
                    MAX(timestamp_ms) AS last_ts
             FROM events
             WHERE timestamp_ms >= ?1
             GROUP BY agent",
        );
        let mut stmt = match q {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such")) =>
            {
                return Ok(Vec::new())
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let rows = stmt.query_map([start_5h], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
            ))
        });
        let mut out = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (agent, tokens_5h, cost_5h, events_5h, last_ts) = row;
                // Tokens in last 5 min
                let q2 = self.conn.query_row(
                    "SELECT COALESCE(SUM(input_tokens + output_tokens), 0) FROM events
                     WHERE agent = ?1 AND timestamp_ms >= ?2",
                    params![agent, start_5m],
                    |row| row.get::<_, i64>(0),
                );
                let tokens_5m = q2.unwrap_or(0).max(0) as u64;
                let seconds_idle = ((now_ms - last_ts) / 1000).max(0) as u64;
                let status = if tokens_5m > 50_000 {
                    AgentLiveStatus::Heavy
                } else if seconds_idle <= 60 {
                    AgentLiveStatus::Active
                } else if seconds_idle <= 1800 {
                    AgentLiveStatus::Idle
                } else if events_5h > 0 {
                    AgentLiveStatus::Quiet
                } else {
                    AgentLiveStatus::Inactive
                };
                out.push(AgentSummary {
                    agent,
                    tokens_5h: tokens_5h.max(0) as u64,
                    cost_5h: Microcents(cost_5h.max(0) as u64),
                    tokens_5m,
                    seconds_idle,
                    status,
                });
            }
        }
        // Sort by tokens_5h desc.
        out.sort_by(|a, b| b.tokens_5h.cmp(&a.tokens_5h));
        Ok(out)
    }

    fn breakdown_inner(
        &self,
        kind: &str,
        path: &str,
        hours: i64,
        limit: usize,
        order_by_count: bool,
    ) -> Result<Vec<Breakdown>, StoreError> {
        let start_ms =
            (chrono::Utc::now() - chrono::Duration::hours(hours)).timestamp_millis();
        let order_col = if order_by_count { "count" } else { "tokens" };
        let sql = format!(
            "SELECT json_extract(payload, '{path}') AS label,
                    COUNT(*) AS count,
                    COALESCE(SUM(input_tokens + output_tokens), 0) AS tokens
             FROM events
             WHERE kind = '{kind}' AND timestamp_ms >= ?1
             GROUP BY label
             HAVING label IS NOT NULL
             ORDER BY {order_col} DESC
             LIMIT ?2"
        );
        let stmt_result = self.conn.prepare(&sql);
        let mut stmt = match stmt_result {
            Ok(s) => s,
            Err(rusqlite::Error::SqliteFailure(_, msg))
                if msg.as_deref().is_some_and(|m| m.contains("no such table")) =>
            {
                return Ok(Vec::new())
            }
            Err(e) => return Err(StoreError::Sqlite(e)),
        };
        let rows = stmt.query_map([start_ms, limit as i64], |r| {
            let label: Option<String> = r.get(0)?;
            let count: i64 = r.get(1)?;
            let tokens: i64 = r.get(2)?;
            Ok((
                label.unwrap_or_else(|| "?".into()),
                count.max(0) as u64,
                tokens.max(0) as u64,
            ))
        });
        let mut out = Vec::new();
        if let Ok(rows) = rows {
            for row in rows.flatten() {
                out.push(Breakdown {
                    label: row.0,
                    count: row.1,
                    tokens: row.2,
                });
            }
        }
        Ok(out)
    }
}

/// One day's cost totals — used by the 7-day trend sparkline + comparison.
#[derive(Debug, Clone)]
pub struct DailyCost {
    pub date_local: String,
    pub cost_microcents: Microcents,
    pub event_count: u64,
}

/// Status of a single session — drives the row color in the sessions table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Active,
    Idle,
    Done,
    Looping,
}

impl SessionStatus {
    pub fn label(self) -> &'static str {
        match self {
            SessionStatus::Active => "ACTIVE",
            SessionStatus::Idle => "idle",
            SessionStatus::Done => "done",
            SessionStatus::Looping => "LOOPING",
        }
    }
}

/// One session in the sessions table.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub session_id: String,
    pub agent: String,
    pub model: String,
    pub project: String,
    pub first_event_ms: i64,
    pub last_event_ms: i64,
    pub total_tokens: u64,
    pub total_cost: Microcents,
    pub event_count: u64,
    pub status: SessionStatus,
    pub looping_score: u32,
    /// Peak input_tokens seen on any model_call in this session. Proxy for
    /// "how full is the context window right now."
    pub peak_context: u64,
}

/// Live status of one agent — drives the top-bar color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLiveStatus {
    /// Heavy burn in the last 5 minutes.
    Heavy,
    /// Active in the last minute.
    Active,
    /// Active in the last 30 minutes but quiet now.
    Idle,
    /// Saw events in the last 5h but nothing in 30 min.
    Quiet,
    /// No events in the last 5h.
    Inactive,
}

impl AgentLiveStatus {
    pub fn label(self) -> &'static str {
        match self {
            AgentLiveStatus::Heavy => "HEAVY",
            AgentLiveStatus::Active => "ACTIVE",
            AgentLiveStatus::Idle => "idle",
            AgentLiveStatus::Quiet => "quiet",
            AgentLiveStatus::Inactive => "INACTIVE",
        }
    }
}

/// One row in the top-of-screen agent bars.
#[derive(Debug, Clone)]
pub struct AgentSummary {
    pub agent: String,
    pub tokens_5h: u64,
    pub cost_5h: Microcents,
    pub tokens_5m: u64,
    pub seconds_idle: u64,
    pub status: AgentLiveStatus,
}

/// Power-user metrics: burn rate, cache hit rate, I/O ratio, latency.
/// Computed over a recent window (typically 30 minutes).
#[derive(Debug, Clone, Default, Copy)]
pub struct LiveMetrics {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub model_calls: u64,
    /// Average tokens/minute over the window.
    pub tokens_per_minute: u64,
    /// Average tokens/second over the window.
    pub tokens_per_second: u64,
    /// % of input tokens served from cache (0-100).
    pub cache_hit_rate: f32,
    /// Output tokens per input token. Low = context-heavy session.
    pub io_ratio_output_per_input: f32,
    /// Average duration of a model call in milliseconds.
    pub avg_latency_ms: u64,
}

/// Recent activity row, denormalized for the Live activity pane.
#[derive(Debug, Clone)]
pub struct RecentEvent {
    pub timestamp: DateTime<Utc>,
    pub agent: String,
    pub kind: String,
    pub verb: String,
    pub target: Option<String>,
    pub tokens: u64,
}

/// One bucket in a token sparkline.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenBucket {
    pub start_ms: i64,
    pub tokens: u64,
    pub events: u64,
}

/// Breakdown of one slice (tool name, model name, file path) over a window.
#[derive(Debug, Clone)]
pub struct Breakdown {
    pub label: String,
    pub count: u64,
    pub tokens: u64,
}

/// Token sums for one agent over a time window.
#[derive(Debug, Clone, Default, Copy)]
pub struct WindowTokens {
    pub input: u64,
    pub output: u64,
    pub event_count: u64,
}

impl WindowTokens {
    pub fn total_billable(&self) -> u64 {
        self.input + self.output
    }
}

/// Aggregate view of "today" — feeds the front-page Today pane.
#[derive(Debug, Clone, Default)]
pub struct TodaySummary {
    pub event_count: u64,
    pub active_agents: u32,
    pub total_cost: Microcents,
    pub tool_calls: u64,
    pub model_calls: u64,
    pub user_messages: u64,
    pub file_edit_events: u64,
    pub distinct_files_touched: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Convert a local YYYY-MM-DD string to its UTC start-of-day millisecond timestamp.
fn local_day_start_ms(local_date: &str) -> i64 {
    use chrono::{NaiveDate, TimeZone};
    let parsed = match NaiveDate::parse_from_str(local_date, "%Y-%m-%d") {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let local_dt = match parsed.and_hms_opt(0, 0, 0) {
        Some(t) => t,
        None => return 0,
    };
    chrono::Local
        .from_local_datetime(&local_dt)
        .single()
        .map(|t| t.with_timezone(&chrono::Utc).timestamp_millis())
        .unwrap_or(0)
}

/// True for placeholder model names produced by observer/memory tools, not
/// actual models the user selected. We hide these from the Models breakdown.
fn is_synthetic_model(label: &str) -> bool {
    let l = label.trim().trim_matches('<').trim_matches('>').to_ascii_lowercase();
    matches!(l.as_str(), "synthetic" | "unknown-model" | "" | "?" | "none")
}

/// Derive a friendly verb + target from the kind + raw payload.
/// Shared between LatestActivity::verb_and_target and recent_events.
fn derive_verb(kind: &str, payload_json: &str) -> (String, Option<String>) {
    let payload: serde_json::Value =
        serde_json::from_str(payload_json).unwrap_or(serde_json::Value::Null);
    let kind_obj = payload.get("kind").cloned().unwrap_or(serde_json::Value::Null);
    match kind {
        "tool_call" => {
            let tool = kind_obj
                .get("tool")
                .and_then(|v| v.as_str())
                .unwrap_or("?")
                .to_string();
            let target = kind_obj
                .get("target")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            (tool, target)
        }
        "file_edit" => {
            let path = kind_obj
                .get("path")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            ("Edit".into(), path)
        }
        "model_call" => {
            let model = kind_obj
                .get("model")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            ("Model".into(), model)
        }
        "session_start" => ("Session start".into(), None),
        "session_end" => ("Session end".into(), None),
        "user_message" => ("User".into(), None),
        "assistant_message" => ("Assistant".into(), None),
        _ => (kind.to_string(), None),
    }
}

/// One row from the `events` table — the latest activity across all agents.
#[derive(Debug, Clone)]
pub struct LatestActivity {
    pub agent: String,
    pub session_id: String,
    pub timestamp_ms: i64,
    pub kind: String,
    pub payload: String,
}

impl LatestActivity {
    pub fn timestamp(&self) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp_millis(self.timestamp_ms).unwrap_or(DateTime::<Utc>::UNIX_EPOCH)
    }

    /// Friendly verb derived from the event kind + payload.
    /// Returns something like "editing src/auth.ts" or "running npm test".
    pub fn verb_and_target(&self) -> (String, Option<String>) {
        let payload: serde_json::Value =
            serde_json::from_str(&self.payload).unwrap_or(serde_json::Value::Null);
        let kind = payload.get("kind").cloned().unwrap_or(serde_json::Value::Null);

        match self.kind.as_str() {
            "tool_call" => {
                let tool = kind
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .unwrap_or("doing something");
                let target = kind
                    .get("target")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let verb = match tool {
                    "Read" => "reading",
                    "Edit" => "editing",
                    "Bash" => "running",
                    "Grep" => "searching",
                    "Glob" => "looking for",
                    "WebSearch" => "searching the web for",
                    _ => "using a tool on",
                };
                (verb.to_string(), target)
            }
            "file_edit" => {
                let path = kind
                    .get("path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                ("editing".to_string(), path)
            }
            "model_call" => {
                let model = kind
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                ("thinking with".to_string(), model)
            }
            "session_start" => ("starting a session".to_string(), None),
            "session_end" => ("finished a session".to_string(), None),
            _ => ("active".to_string(), None),
        }
    }
}
