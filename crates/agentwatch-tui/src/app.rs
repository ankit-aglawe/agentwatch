//! TUI application state. Pulls data from the SQLite store on each refresh tick.

use agentwatch_store::{
    AgentSummary, Breakdown, DailyCost, LatestActivity, LiveMetrics, Reader, RecentEvent,
    SessionSummary, TodaySummary, TokenBucket,
};
use chrono::{DateTime, Duration, Utc};

use crate::live_signal::{self, LiveSignal};
use crate::runway::{Plan, RunwayState};

pub struct App {
    pub latest: Option<LatestActivity>,
    pub today: TodaySummary,
    pub runway: RunwayState,
    pub metrics: LiveMetrics,
    pub agents: Vec<AgentSummary>,
    pub sessions: Vec<SessionSummary>,
    pub recent: Vec<RecentEvent>,
    pub sparkline: Vec<TokenBucket>,
    pub tool_breakdown: Vec<Breakdown>,
    pub model_breakdown: Vec<Breakdown>,
    pub project_breakdown: Vec<Breakdown>,
    pub hot_files: Vec<Breakdown>,
    pub daily_costs: Vec<DailyCost>,
    pub week_tokens: u64,
    /// Live signal from claude-mem (the most recent user prompt to Claude Code).
    /// `None` if claude-mem isn't installed or no fresh prompt found.
    pub live: Option<LiveSignal>,
    /// Largest input_tokens seen in the most-active session (proxy for context fill).
    pub top_session_context: u64,
    /// Model used by that top session — drives the context-window cap.
    pub top_session_model: String,
    pub last_refreshed: DateTime<Utc>,
    pub db_present: bool,
}

impl App {
    pub fn new() -> Self {
        Self {
            latest: None,
            today: TodaySummary::default(),
            runway: RunwayState::compute(Plan::default(), 0, 0, 0, None),
            metrics: LiveMetrics::default(),
            agents: Vec::new(),
            sessions: Vec::new(),
            recent: Vec::new(),
            sparkline: Vec::new(),
            tool_breakdown: Vec::new(),
            model_breakdown: Vec::new(),
            project_breakdown: Vec::new(),
            hot_files: Vec::new(),
            daily_costs: Vec::new(),
            week_tokens: 0,
            live: None,
            top_session_context: 0,
            top_session_model: "—".into(),
            last_refreshed: Utc::now(),
            db_present: agentwatch_core::paths::db_path().exists(),
        }
    }

    pub fn refresh(&mut self) {
        let db = agentwatch_core::paths::db_path();
        self.db_present = db.exists();
        if !self.db_present {
            *self = Self {
                last_refreshed: Utc::now(),
                db_present: false,
                ..Self::new()
            };
            return;
        }
        let Ok(reader) = Reader::open(&db) else {
            self.last_refreshed = Utc::now();
            return;
        };

        self.latest = reader.latest().ok().flatten();
        self.today = reader.today_summary().unwrap_or_default();
        self.metrics = reader.live_metrics(30).unwrap_or_default();
        self.agents = reader.agent_summary(5).unwrap_or_default();
        // Fetch enough sessions to fill any terminal. The render side caps to fit.
        self.sessions = reader.recent_sessions(50).unwrap_or_default();

        // Hoisted: primary agent — used by sparkline, week_tokens, runway.
        let primary_agent = self
            .latest
            .as_ref()
            .map(|l| l.agent.clone())
            .unwrap_or_else(|| "Claude Code".into());
        self.recent = reader.recent_events(15).unwrap_or_default();
        self.sparkline = reader
            .token_sparkline("Claude Code", 60, 1)
            .unwrap_or_default();
        self.tool_breakdown = reader.tool_breakdown(24, 6).unwrap_or_default();
        self.model_breakdown = reader.model_breakdown(24, 5).unwrap_or_default();
        self.project_breakdown = reader.project_breakdown(24, 6).unwrap_or_default();
        self.hot_files = reader.hot_files_today(24, 6).unwrap_or_default();
        self.daily_costs = reader.daily_cost_series(7).unwrap_or_default();
        self.week_tokens = reader
            .tokens_in_last_days(&primary_agent, 7)
            .unwrap_or_default()
            .total_billable();

        // Context-window proxy for the most active session.
        if let Some(top) = self.sessions.first() {
            if let Ok((ctx, model)) = reader.session_peak_context(&top.session_id) {
                self.top_session_context = ctx;
                self.top_session_model = model;
            }
        }

        // Headline runway state — still useful for compact mode and status line.
        let now = Utc::now();
        let w5h = (now - Duration::hours(5)).timestamp_millis();
        let w30m = (now - Duration::minutes(30)).timestamp_millis();
        let w5m = (now - Duration::minutes(5)).timestamp_millis();
        let end = now.timestamp_millis();
        let last5h = reader
            .tokens_in_window(&primary_agent, w5h, end)
            .unwrap_or_default();
        let last30m = reader
            .tokens_in_window(&primary_agent, w30m, end)
            .unwrap_or_default();
        let last5m = reader
            .tokens_in_window(&primary_agent, w5m, end)
            .unwrap_or_default();
        let seconds_idle = self
            .latest
            .as_ref()
            .map(|l| (now - l.timestamp()).num_seconds().max(0) as u64);
        self.runway = RunwayState::compute(
            Plan::default(),
            last5h.total_billable(),
            last30m.total_billable(),
            last5m.total_billable(),
            seconds_idle,
        );

        // Read claude-mem's live signal. Cheap (one file read of the newest jsonl).
        self.live = live_signal::read_latest();

        self.last_refreshed = Utc::now();
    }

    pub fn has_activity(&self) -> bool {
        self.today.event_count > 0 || !self.sessions.is_empty()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
