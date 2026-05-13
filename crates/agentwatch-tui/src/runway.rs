//! Compute the calm headline state from real usage data.
//!
//! Honest v0.1: we don't pretend to know Anthropic's exact rate-limit math.
//! The community-reported "Pro = 44k tokens / 5h" numbers are off by 10-20×
//! against real heavy usage (one observed user logged 5M tokens / 5h with no
//! rate-limit warning). Rather than show a misleading "% of cap used" tank,
//! we show absolute token volumes and base the calm headline on whether the
//! user is currently active or idle.
//!
//! When Anthropic publishes real numbers — or when we add per-user calibration
//! from observed rate-limit responses — we'll plug accurate cap math back in.

use chrono::Duration;

/// Plan label is kept around for the subline ("Max 20 plan"), but it no longer
/// drives the headline state since the community caps don't match reality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    Pro,
    Max5,
    Max20,
}

impl Plan {
    pub fn label(self) -> &'static str {
        match self {
            Plan::Pro => "Pro",
            Plan::Max5 => "Max 5",
            Plan::Max20 => "Max 20",
        }
    }
}

impl Default for Plan {
    fn default() -> Self {
        // Auto-detect from local Claude Code settings; fall back to Max 20.
        match agentwatch_core::plan_detect::detect_plan() {
            Some(agentwatch_core::plan_detect::DetectedPlan::Pro) => Plan::Pro,
            Some(agentwatch_core::plan_detect::DetectedPlan::Max5) => Plan::Max5,
            Some(agentwatch_core::plan_detect::DetectedPlan::Max20) => Plan::Max20,
            // Team and Enterprise default to Max 20 — closest fit until we
            // expand the Plan enum.
            Some(_) => Plan::Max20,
            None => Plan::Max20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthLevel {
    /// No events at all in the last hour.
    Quiet,
    /// Activity exists but the agent has been idle 5+ minutes.
    Idle,
    /// At least one event in the last 5 minutes.
    Active,
    /// Heavy activity — > 50k tokens in the last 5 minutes.
    Heavy,
}

impl HealthLevel {
    pub fn emoji(self) -> &'static str {
        match self {
            HealthLevel::Quiet => "🟢",
            HealthLevel::Idle => "🟢",
            HealthLevel::Active => "🟢",
            HealthLevel::Heavy => "🟡",
        }
    }

    pub fn headline(self) -> &'static str {
        match self {
            HealthLevel::Quiet => "Quiet so far.",
            HealthLevel::Idle => "You're good.",
            HealthLevel::Active => "Claude Code is working.",
            HealthLevel::Heavy => "Heavy usage — keep an eye on it.",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct RunwayState {
    pub level: HealthLevel,
    pub plan: Plan,
    /// Tokens used in the current 5-hour window (input + output).
    pub tokens_5h: u64,
    /// Tokens used in the last 30 minutes — the burn-rate sample.
    pub tokens_30m: u64,
    /// Tokens used in the last 5 minutes — drives the activity state.
    pub tokens_5m: u64,
    /// Seconds since the most recent event, if known.
    pub seconds_idle: Option<u64>,
}

impl RunwayState {
    pub fn compute(
        plan: Plan,
        tokens_5h: u64,
        tokens_30m: u64,
        tokens_5m: u64,
        seconds_idle: Option<u64>,
    ) -> Self {
        let level = if tokens_5h == 0 {
            HealthLevel::Quiet
        } else if tokens_5m > 50_000 {
            HealthLevel::Heavy
        } else if let Some(s) = seconds_idle {
            if s > 300 {
                HealthLevel::Idle
            } else {
                HealthLevel::Active
            }
        } else if tokens_5m > 0 {
            HealthLevel::Active
        } else {
            HealthLevel::Idle
        };
        Self {
            level,
            plan,
            tokens_5h,
            tokens_30m,
            tokens_5m,
            seconds_idle,
        }
    }

    /// Plain-English description of recent activity. No fake percentages.
    pub fn detail_phrase(&self) -> String {
        match self.level {
            HealthLevel::Quiet => "no events yet".to_string(),
            HealthLevel::Heavy => format!(
                "{} tokens in the last 5 minutes — heavy chunk",
                format_short_tokens(self.tokens_5m)
            ),
            HealthLevel::Active => format!(
                "{} tokens in the last 5 minutes",
                format_short_tokens(self.tokens_5m)
            ),
            HealthLevel::Idle => match self.seconds_idle {
                Some(s) => format!("last event {}", format_idle_age(Duration::seconds(s as i64))),
                None => "idle".to_string(),
            },
        }
    }

    /// Format for the subline below the headline.
    pub fn subline(&self) -> String {
        let total_5h = format_short_tokens(self.tokens_5h);
        match self.level {
            HealthLevel::Quiet => "Start using Claude Code and this will fill in.".to_string(),
            _ => format!(
                "Claude Code — {} tokens in the last 5 hours ({} plan)",
                total_5h,
                self.plan.label()
            ),
        }
    }
}

fn format_short_tokens(n: u64) -> String {
    if n < 1_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{:.1}k", n as f32 / 1_000.0)
    } else {
        format!("{:.1}M", n as f32 / 1_000_000.0)
    }
}

fn format_idle_age(dur: Duration) -> String {
    let secs = dur.num_seconds().max(0);
    if secs < 60 {
        format!("{}s ago", secs)
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quiet_when_no_tokens_in_5h() {
        let s = RunwayState::compute(Plan::Max20, 0, 0, 0, None);
        assert_eq!(s.level, HealthLevel::Quiet);
    }

    #[test]
    fn heavy_when_burst_in_last_5min() {
        let s = RunwayState::compute(Plan::Max20, 1_000_000, 200_000, 100_000, Some(20));
        assert_eq!(s.level, HealthLevel::Heavy);
    }

    #[test]
    fn active_with_recent_events() {
        let s = RunwayState::compute(Plan::Max20, 100_000, 10_000, 1_000, Some(15));
        assert_eq!(s.level, HealthLevel::Active);
    }

    #[test]
    fn idle_when_no_events_in_last_5min_but_some_in_5h() {
        let s = RunwayState::compute(Plan::Max20, 100_000, 5_000, 0, Some(600));
        assert_eq!(s.level, HealthLevel::Idle);
    }

    #[test]
    fn detail_phrase_quiet() {
        let s = RunwayState::compute(Plan::Max20, 0, 0, 0, None);
        assert_eq!(s.detail_phrase(), "no events yet");
    }

    #[test]
    fn detail_phrase_idle_uses_age() {
        let s = RunwayState::compute(Plan::Max20, 50_000, 0, 0, Some(900));
        let phrase = s.detail_phrase();
        assert!(phrase.contains("15m ago"), "got: {phrase}");
    }

    #[test]
    fn subline_quiet() {
        let s = RunwayState::compute(Plan::Max20, 0, 0, 0, None);
        assert!(s.subline().contains("Start using"));
    }

    #[test]
    fn subline_active_shows_5h_total() {
        let s = RunwayState::compute(Plan::Max20, 1_234_567, 50_000, 5_000, Some(10));
        let sub = s.subline();
        assert!(sub.contains("1.2M"), "expected 1.2M in: {sub}");
        assert!(sub.contains("Max 20"));
    }

    #[test]
    fn format_short_tokens_ranges() {
        assert_eq!(format_short_tokens(999), "999");
        assert_eq!(format_short_tokens(1_234), "1.2k");
        assert_eq!(format_short_tokens(1_234_567), "1.2M");
    }
}
