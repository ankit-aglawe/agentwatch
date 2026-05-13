//! Detect the user's Claude Code subscription tier from local settings.
//!
//! Anthropic's CLI caches subscription info locally. Several open-source
//! analytics tools (`ccusage`, `phuryn/claude-usage`, `Claude-Code-Usage-Monitor`)
//! read these files for the same purpose. We follow the same convention.
//!
//! Files probed (in order):
//!   1. `~/.claude.json` — user settings file
//!   2. `~/.claude/settings.json` — alt location seen in older versions
//!
//! We deliberately DO NOT read `~/.claude/.credentials.json` — that file holds
//! the OAuth token. Subscription tier (a public-ish fact about the user) lives
//! in the settings file.
//!
//! Field names probed (case-insensitive, with several variants):
//!   - `subscriptionType`, `subscription_type`, `plan`, `accountType`, `tier`
//!
//! Returns `None` if no recognizable plan can be detected — caller falls back
//! to a sensible default (Max 20 is the most generous, least alarming).

use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectedPlan {
    Pro,
    Max5,
    Max20,
    Team,
    Enterprise,
}

impl DetectedPlan {
    pub fn label(self) -> &'static str {
        match self {
            DetectedPlan::Pro => "Pro",
            DetectedPlan::Max5 => "Max 5",
            DetectedPlan::Max20 => "Max 20",
            DetectedPlan::Team => "Team",
            DetectedPlan::Enterprise => "Enterprise",
        }
    }

    /// Approximate 5-hour rolling token cap. Anthropic doesn't publish the
    /// exact numbers, so these are community-reported approximations.
    pub fn five_hour_token_cap(self) -> u64 {
        match self {
            DetectedPlan::Pro => 44_000,
            DetectedPlan::Max5 => 88_000,
            DetectedPlan::Max20 => 220_000,
            DetectedPlan::Team => 500_000,
            DetectedPlan::Enterprise => 1_000_000,
        }
    }
}

/// Try to detect the user's plan from local Claude Code settings.
/// Returns `None` if no recognizable plan field is found.
pub fn detect_plan() -> Option<DetectedPlan> {
    for path in candidate_settings_files() {
        if let Some(plan) = read_plan_from_file(&path) {
            return Some(plan);
        }
    }
    None
}

fn candidate_settings_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = crate::paths::home_dir() {
        out.push(home.join(".claude.json"));
        out.push(home.join(".claude").join("settings.json"));
    }
    out
}

fn read_plan_from_file(path: &std::path::Path) -> Option<DetectedPlan> {
    if !path.exists() {
        return None;
    }
    let contents = std::fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&contents).ok()?;
    find_plan_in_value(&value)
}

/// Walk the JSON tree looking for a recognizable plan-like field. Keys are
/// matched case-insensitively across several common naming conventions.
fn find_plan_in_value(value: &serde_json::Value) -> Option<DetectedPlan> {
    const PLAN_KEYS: &[&str] = &[
        "subscriptiontype",
        "subscription_type",
        "plan",
        "accounttype",
        "account_type",
        "tier",
    ];
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key_lower = k.to_ascii_lowercase();
                if PLAN_KEYS.contains(&key_lower.as_str()) {
                    if let Some(s) = v.as_str() {
                        if let Some(plan) = normalize_plan_string(s) {
                            return Some(plan);
                        }
                    }
                }
                if let Some(found) = find_plan_in_value(v) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(found) = find_plan_in_value(v) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

/// Map a raw plan string from the settings file to a `DetectedPlan`. Tries
/// many variants because Anthropic has renamed the field at least twice and
/// different versions of the CLI write different values.
fn normalize_plan_string(s: &str) -> Option<DetectedPlan> {
    let lower = s.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.contains("enterprise") {
        return Some(DetectedPlan::Enterprise);
    }
    if lower.contains("team") {
        return Some(DetectedPlan::Team);
    }
    // Max variants: "max_20x", "max-20x", "max20", "max_20", "max 20"
    if lower.contains("max") && (lower.contains("20") || lower.contains("twenty")) {
        return Some(DetectedPlan::Max20);
    }
    if lower.contains("max") && (lower.contains("5") || lower.contains("five")) {
        return Some(DetectedPlan::Max5);
    }
    if lower == "max" || lower.starts_with("max ") || lower.starts_with("max_") {
        // Unqualified "max" — assume the cheaper Max 5 tier to be conservative.
        return Some(DetectedPlan::Max5);
    }
    if lower.contains("pro") {
        return Some(DetectedPlan::Pro);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_pro() {
        assert_eq!(normalize_plan_string("pro"), Some(DetectedPlan::Pro));
        assert_eq!(normalize_plan_string("Pro"), Some(DetectedPlan::Pro));
    }

    #[test]
    fn normalize_max_variants() {
        assert_eq!(normalize_plan_string("max_5x"), Some(DetectedPlan::Max5));
        assert_eq!(normalize_plan_string("max-20x"), Some(DetectedPlan::Max20));
        assert_eq!(normalize_plan_string("max20"), Some(DetectedPlan::Max20));
        assert_eq!(normalize_plan_string("Max 20"), Some(DetectedPlan::Max20));
        assert_eq!(normalize_plan_string("max"), Some(DetectedPlan::Max5));
    }

    #[test]
    fn normalize_team_enterprise() {
        assert_eq!(normalize_plan_string("team"), Some(DetectedPlan::Team));
        assert_eq!(
            normalize_plan_string("enterprise"),
            Some(DetectedPlan::Enterprise)
        );
    }

    #[test]
    fn normalize_unknown() {
        assert_eq!(normalize_plan_string(""), None);
        assert_eq!(normalize_plan_string("xyz"), None);
    }

    #[test]
    fn find_in_nested_json() {
        let json = serde_json::json!({
            "user": { "subscriptionType": "max_20x" },
            "preferences": {}
        });
        assert_eq!(find_plan_in_value(&json), Some(DetectedPlan::Max20));
    }
}
