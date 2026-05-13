//! Money math without float drift.
//!
//! 1 microcent = 1/100 of a cent = 1/10,000 of a dollar.
//!
//! At the high end of a heavy user (10M tokens of input + output per day, mixed
//! across providers), totals reach ~$50/day. In microcents that's 500,000 units
//! per day, or ~180M per year. Well inside u64's range (which holds up to
//! ~1.8e19 microcents, ~$1.8e15).
//!
//! Floats were considered and rejected. Summing many small `f64` values causes
//! drift on the order of 1 cent per few thousand events. Users will compare
//! agentwatch totals against their vendor bills. The totals must match exactly.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Sub, SubAssign};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Microcents(pub u64);

impl Microcents {
    pub const ZERO: Microcents = Microcents(0);
    pub const PER_CENT: u64 = 100;
    pub const PER_DOLLAR: u64 = 10_000;

    #[inline]
    pub fn from_cents(cents: u64) -> Self {
        Microcents(cents.saturating_mul(Self::PER_CENT))
    }

    #[inline]
    pub fn from_dollars(dollars: u64) -> Self {
        Microcents(dollars.saturating_mul(Self::PER_DOLLAR))
    }

    /// Convert from a per-million-token price (the common vendor format) and a
    /// token count. Uses integer math throughout.
    ///
    /// Example: `from_per_million(1500, 1_000_000)` → 1500 cents = $15.00 →
    /// 150_000 microcents.
    #[inline]
    pub fn from_per_million(price_cents_per_million_tokens: u64, tokens: u64) -> Self {
        let microcents_per_million = price_cents_per_million_tokens.saturating_mul(Self::PER_CENT);
        Self::from_microcents_per_million(microcents_per_million, tokens)
    }

    /// Higher-resolution variant of `from_per_million` - accepts a price in
    /// microcents-per-million-tokens. Use this when the upstream price is
    /// sub-cent (e.g. Gemini Flash at $0.075/M = 750 microcents/M).
    ///
    /// Example: `from_microcents_per_million(750, 1_000_000)` → 750 microcents
    /// = $0.075.
    #[inline]
    pub fn from_microcents_per_million(microcents_per_million: u64, tokens: u64) -> Self {
        let total = (microcents_per_million as u128 * tokens as u128) / 1_000_000;
        Microcents(total.min(u64::MAX as u128) as u64)
    }

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    pub fn to_dollars_f64(self) -> f64 {
        self.0 as f64 / Self::PER_DOLLAR as f64
    }
}

impl fmt::Display for Microcents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let dollars = self.0 / Self::PER_DOLLAR;
        let remainder = self.0 % Self::PER_DOLLAR;
        // remainder is in microcents; render as cents.subcents
        let cents = remainder / Self::PER_CENT;
        write!(f, "${}.{:02}", dollars, cents)
    }
}

impl Add for Microcents {
    type Output = Microcents;
    #[inline]
    fn add(self, rhs: Self) -> Self::Output {
        Microcents(self.0.saturating_add(rhs.0))
    }
}

impl AddAssign for Microcents {
    #[inline]
    fn add_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_add(rhs.0);
    }
}

impl Sub for Microcents {
    type Output = Microcents;
    #[inline]
    fn sub(self, rhs: Self) -> Self::Output {
        Microcents(self.0.saturating_sub(rhs.0))
    }
}

impl SubAssign for Microcents {
    #[inline]
    fn sub_assign(&mut self, rhs: Self) {
        self.0 = self.0.saturating_sub(rhs.0);
    }
}

impl Sum for Microcents {
    fn sum<I: Iterator<Item = Microcents>>(iter: I) -> Self {
        iter.fold(Microcents::ZERO, |acc, x| acc + x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_cents_basic() {
        assert_eq!(Microcents::from_cents(150).as_u64(), 15_000);
        assert_eq!(Microcents::from_cents(0).as_u64(), 0);
    }

    #[test]
    fn from_dollars_basic() {
        assert_eq!(Microcents::from_dollars(3).as_u64(), 30_000);
    }

    #[test]
    fn display_two_decimals() {
        assert_eq!(format!("{}", Microcents::from_cents(150)), "$1.50");
        assert_eq!(format!("{}", Microcents::from_cents(2)), "$0.02");
        assert_eq!(format!("{}", Microcents::ZERO), "$0.00");
        assert_eq!(format!("{}", Microcents::from_dollars(100)), "$100.00");
    }

    #[test]
    fn add_no_drift_over_many_events() {
        // 100,000 events of 0.0001 cents each = exactly $1.00
        // (Wouldn't pass with f64 sums: drift accumulates to ~$0.0001).
        let one_microcent = Microcents(1);
        let total: Microcents = (0..1_000_000).map(|_| one_microcent).sum();
        assert_eq!(total, Microcents::from_dollars(100));
    }

    #[test]
    fn from_per_million_anthropic_pricing() {
        // Claude Opus 4.7 input: $15/M tokens = 1500 cents/M.
        // 1M input tokens → exactly $15.00 = 150_000 microcents.
        let cost = Microcents::from_per_million(1500, 1_000_000);
        assert_eq!(cost, Microcents::from_dollars(15));
    }

    #[test]
    fn from_per_million_small_token_count() {
        // 1000 input tokens at $15/M → $0.015 = 150 microcents.
        let cost = Microcents::from_per_million(1500, 1000);
        assert_eq!(cost.as_u64(), 150);
    }

    #[test]
    fn sub_saturates_at_zero() {
        let a = Microcents::from_cents(50);
        let b = Microcents::from_cents(100);
        assert_eq!(a - b, Microcents::ZERO);
    }

    #[test]
    fn serde_roundtrip_transparent() {
        let value = Microcents::from_cents(42);
        let json = serde_json::to_string(&value).unwrap();
        // transparent representation: just a number, no wrapping object.
        assert_eq!(json, "4200");
        let parsed: Microcents = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, value);
    }
}
