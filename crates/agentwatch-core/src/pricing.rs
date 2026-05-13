//! Pricing lookup, loaded from an embedded LiteLLM-shaped JSON snapshot.
//!
//! The data file lives at `crates/agentwatch-core/data/pricing.json` and mirrors
//! the schema of [LiteLLM's `model_prices_and_context_window.json`][litellm] so
//! the CI refresh workflow can swap in upstream data verbatim.
//!
//! [litellm]: https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json
//!
//! Math: LiteLLM gives prices as USD per single token (float). We convert each
//! field to integer microcents-per-million-tokens at parse time (once, cached
//! in a `OnceLock`). All cost math at runtime is then integer-only - no float
//! drift, totals match vendor bills to the microcent.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::Deserialize;
use thiserror::Error;

use crate::money::Microcents;

/// Embedded snapshot. Refreshed weekly via CI. See `pricing-meta.json`.
const PRICING_JSON: &str = include_str!("../data/pricing.json");
const PRICING_META_JSON: &str = include_str!("../data/pricing-meta.json");

#[derive(Debug, Error)]
pub enum PricingError {
    #[error("unknown model: {0}")]
    UnknownModel(String),
}

/// Raw LiteLLM-shaped record. Optional fields stay `Option` because not every
/// provider exposes cache pricing.
#[derive(Debug, Clone, Deserialize)]
struct RawPrice {
    #[serde(default)]
    input_cost_per_token: f64,
    #[serde(default)]
    output_cost_per_token: f64,
    #[serde(default)]
    cache_read_input_token_cost: Option<f64>,
    #[serde(default)]
    cache_creation_input_token_cost: Option<f64>,
    #[serde(default)]
    #[allow(dead_code)]
    litellm_provider: Option<String>,
}

/// What we hand back to consumers. Integers all the way through.
///
/// Prices are stored in **microcents per million tokens** so we can represent
/// sub-cent prices (e.g. Gemini Flash at $0.075/M = 750 microcents/M) without
/// float drift. 1 cent = 100 microcents.
#[derive(Debug, Clone, Copy)]
pub struct ModelPrice {
    pub input_microcents_per_million: u64,
    pub output_microcents_per_million: u64,
    pub cache_read_microcents_per_million: u64,
    pub cache_write_microcents_per_million: u64,
}

impl ModelPrice {
    pub fn cost(
        &self,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
    ) -> Microcents {
        Microcents::from_microcents_per_million(self.input_microcents_per_million, input_tokens)
            + Microcents::from_microcents_per_million(self.output_microcents_per_million, output_tokens)
            + Microcents::from_microcents_per_million(self.cache_read_microcents_per_million, cache_read_tokens)
            + Microcents::from_microcents_per_million(self.cache_write_microcents_per_million, cache_write_tokens)
    }
}

/// Snapshot metadata - when the pricing data was last refreshed.
#[derive(Debug, Clone, Deserialize)]
pub struct PricingMeta {
    pub snapshot_date: String,
    pub source_url: String,
    pub source_commit: String,
    pub model_count: u32,
    #[serde(default)]
    pub note: String,
}

fn table() -> &'static HashMap<String, ModelPrice> {
    static CELL: OnceLock<HashMap<String, ModelPrice>> = OnceLock::new();
    CELL.get_or_init(|| {
        let raw: HashMap<String, serde_json::Value> = serde_json::from_str(PRICING_JSON)
            .expect("embedded pricing.json must parse - broken at compile time");
        raw.into_iter()
            .filter_map(|(model, value)| {
                if model.starts_with('_') {
                    return None; // comment fields
                }
                let parsed: RawPrice = match serde_json::from_value(value) {
                    Ok(p) => p,
                    Err(_) => return None,
                };
                Some((model, convert(&parsed)))
            })
            .collect()
    })
}

fn convert(raw: &RawPrice) -> ModelPrice {
    ModelPrice {
        input_microcents_per_million: usd_per_token_to_microcents_per_million(raw.input_cost_per_token),
        output_microcents_per_million: usd_per_token_to_microcents_per_million(raw.output_cost_per_token),
        cache_read_microcents_per_million: raw
            .cache_read_input_token_cost
            .map(usd_per_token_to_microcents_per_million)
            .unwrap_or(0),
        cache_write_microcents_per_million: raw
            .cache_creation_input_token_cost
            .map(usd_per_token_to_microcents_per_million)
            .unwrap_or(0),
    }
}

/// Convert LiteLLM's USD-per-single-token to our microcents-per-million-tokens
/// integer. Rounds to nearest. Saturating cast.
fn usd_per_token_to_microcents_per_million(usd_per_token: f64) -> u64 {
    // USD/token × 100 cents/USD × 100 microcents/cent × 1_000_000 tokens/M
    // = microcents/M
    let microcents_per_million = usd_per_token * 100.0 * 100.0 * 1_000_000.0;
    if microcents_per_million.is_nan() || microcents_per_million < 0.0 {
        return 0;
    }
    let rounded = microcents_per_million.round();
    if rounded > u64::MAX as f64 {
        return u64::MAX;
    }
    rounded as u64
}

/// Look up a model's pricing. Returns `None` for unknown models - the caller
/// must record $0 cost AND emit a warning event so the user knows pricing is
/// missing (silent $0 would deceive). See `EventKind::Unknown`.
///
/// Lookup is multi-stage:
///   1. Exact match (`claude-sonnet-4-6`)
///   2. Prefix match - strip trailing `-YYYYMMDD` date suffix
///      (`claude-sonnet-4-5-20250929` → `claude-sonnet-4-5`)
///   3. Family fallback - match on family stem
///      (`claude-sonnet-4-5` → matches any `claude-sonnet-4-x` entry)
///
/// This avoids having to enumerate every dated model variant in pricing.json.
pub fn lookup(model: &str) -> Option<ModelPrice> {
    let t = table();
    // Stage 1: exact.
    if let Some(p) = t.get(model) {
        return Some(*p);
    }
    // Stage 2: strip dated suffix like `-20250929`.
    if let Some(stripped) = strip_date_suffix(model) {
        if let Some(p) = t.get(stripped) {
            return Some(*p);
        }
    }
    // Stage 3: family-stem fallback. Walk our known models and find one whose
    // stem matches. e.g. `claude-sonnet-4-5` should hit `claude-sonnet-4-6`
    // entry (both are Sonnet 4.x, similar pricing).
    let stem = model_family_stem(model);
    if !stem.is_empty() {
        for (k, v) in t.iter() {
            if model_family_stem(k) == stem {
                return Some(*v);
            }
        }
    }
    None
}

/// Strip a trailing `-YYYYMMDD` suffix from a model name. Returns the prefix
/// without the date if found.
fn strip_date_suffix(model: &str) -> Option<&str> {
    let bytes = model.as_bytes();
    if bytes.len() < 9 {
        return None;
    }
    let len = bytes.len();
    // Match -YYYYMMDD
    if bytes[len - 9] == b'-' && bytes[len - 8..].iter().all(|b| b.is_ascii_digit()) {
        return Some(&model[..len - 9]);
    }
    None
}

/// Reduce a model name to its family stem for fallback matching.
/// `claude-sonnet-4-5-20250929` → `claude-sonnet-4`
/// `claude-opus-4-7-1m` → `claude-opus-4`
/// `gpt-4o-mini` → `gpt-4o`
fn model_family_stem(model: &str) -> String {
    let cleaned = strip_date_suffix(model).unwrap_or(model);
    // Drop the last hyphen-separated segment if it looks like a minor version
    // or qualifier (single digit, `mini`, `latest`, `1m`).
    let segments: Vec<&str> = cleaned.split('-').collect();
    if segments.len() <= 2 {
        return cleaned.to_string();
    }
    let last = segments[segments.len() - 1];
    let is_minor = last.len() <= 2
        && (last.chars().all(|c| c.is_ascii_digit())
            || last.ends_with('m')
            || matches!(last, "mini" | "latest" | "preview"));
    if is_minor {
        segments[..segments.len() - 1].join("-")
    } else {
        cleaned.to_string()
    }
}

/// Total models we have pricing for.
pub fn known_model_count() -> usize {
    table().len()
}

/// All known model names. Useful for `agentwatch pricing list`.
pub fn known_models() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = table().keys().map(|s| s.as_str()).collect();
    names.sort_unstable();
    names
}

/// When the embedded snapshot was generated. Used for the staleness warning.
pub fn snapshot_meta() -> &'static PricingMeta {
    static CELL: OnceLock<PricingMeta> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str(PRICING_META_JSON)
            .expect("embedded pricing-meta.json must parse - broken at compile time")
    })
}

/// True if the embedded snapshot is older than `days_threshold` days.
/// Used by the TUI to decide whether to show the friendly "my prices are old"
/// banner.
pub fn snapshot_is_stale(days_threshold: i64) -> bool {
    let snapshot = match chrono::NaiveDate::parse_from_str(
        &snapshot_meta().snapshot_date,
        "%Y-%m-%d",
    ) {
        Ok(d) => d,
        Err(_) => return false, // bad date in meta is its own bug; don't show banner
    };
    let today = chrono::Local::now().date_naive();
    (today - snapshot).num_days() > days_threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opus_input_only() {
        let price = lookup("claude-opus-4-7").unwrap();
        // 1M input tokens at $15/M = $15.00.
        let cost = price.cost(1_000_000, 0, 0, 0);
        assert_eq!(cost, Microcents::from_dollars(15));
    }

    #[test]
    fn opus_full_call() {
        let price = lookup("claude-opus-4-7").unwrap();
        // 100k input + 10k output:
        //   input:  100,000 * 1500 / 1,000,000 = 150 cents = 15_000 microcents
        //   output: 10,000  * 7500 / 1,000,000 = 75  cents = 7_500 microcents
        //   total = 225 cents = $2.25
        let cost = price.cost(100_000, 10_000, 0, 0);
        assert_eq!(cost, Microcents::from_cents(225));
    }

    #[test]
    fn unknown_model_returns_none() {
        assert!(lookup("claude-x-future-1").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn haiku_micro_call() {
        let price = lookup("claude-haiku-4-5-20251001").unwrap();
        // 1000 input + 100 output:
        //   input:  1000 * 100 / 1_000_000 = 0.1 cents = 10 microcents
        //   output: 100  * 500 / 1_000_000 = 0.05 cents = 5 microcents
        //   total  = 15 microcents
        let cost = price.cost(1000, 100, 0, 0);
        assert_eq!(cost.as_u64(), 15);
    }

    #[test]
    fn cache_pricing_separate() {
        let price = lookup("claude-opus-4-7").unwrap();
        // 1M cache-read at $1.50/M = $1.50.
        let cost = price.cost(0, 0, 1_000_000, 0);
        assert_eq!(cost, Microcents::from_dollars(1) + Microcents::from_cents(50));
    }

    #[test]
    fn gpt_4o_mini_pricing() {
        let price = lookup("gpt-4o-mini").unwrap();
        // 1M input at $0.15/M = $0.15.
        let cost = price.cost(1_000_000, 0, 0, 0);
        assert_eq!(cost, Microcents::from_cents(15));
    }

    #[test]
    fn gemini_flash_pricing() {
        let price = lookup("gemini-2.0-flash").unwrap();
        // 1M input at $0.075/M = $0.075 = 7.5 cents = 750 microcents.
        let cost = price.cost(1_000_000, 0, 0, 0);
        assert_eq!(cost.as_u64(), 750);
    }

    #[test]
    fn snapshot_meta_loads() {
        let meta = snapshot_meta();
        assert!(!meta.snapshot_date.is_empty());
        assert!(meta.model_count > 0);
    }

    #[test]
    fn known_models_includes_anthropic_and_openai() {
        let models = known_models();
        assert!(models.contains(&"claude-opus-4-7"));
        assert!(models.contains(&"gpt-4o-mini"));
        assert!(models.contains(&"gemini-2.5-pro"));
    }

    #[test]
    fn known_model_count_matches_meta() {
        // The meta file says how many models we shipped; the loader should
        // find the same number. Drift between the two is a real bug.
        let meta = snapshot_meta();
        assert_eq!(known_model_count() as u32, meta.model_count);
    }

    #[test]
    fn zero_cost_models_lookup_cleanly() {
        // Open-tier models with $0 cost should still be lookup-able.
        let price = lookup("windsurf-large").unwrap();
        assert_eq!(price.cost(1_000_000, 1_000_000, 0, 0), Microcents::ZERO);
    }
}
