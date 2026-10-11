//! API-equivalent pricing for the models each provider logs locally, and the
//! per-day accumulator the providers report to the UI.
//!
//! Prices are USD per million tokens (standard tier), taken from the official
//! pricing pages (checked 2026-10-11):
//! - Anthropic: https://platform.claude.com/docs/en/about-claude/pricing
//! - OpenAI:    https://developers.openai.com/api/docs/pricing
//! - Google:    https://ai.google.dev/gemini-api/docs/pricing (2026 rates)

use serde::Serialize;
use std::path::Path;
use std::time::SystemTime;

/// USD per million tokens.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

impl Price {
    fn scaled(self, input_mult: f64, output_mult: f64) -> Self {
        Self {
            input: self.input * input_mult,
            output: self.output * output_mult,
            cache_write_5m: self.cache_write_5m * input_mult,
            cache_write_1h: self.cache_write_1h * input_mult,
            cache_read: self.cache_read * input_mult,
        }
    }
}

/// Tokens of one request, split by billing category. `input` excludes cached tokens.
#[derive(Clone, Copy, Debug, Default)]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_write_5m: u64,
    pub cache_write_1h: u64,
    pub cache_read: u64,
}

impl TokenUsage {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    /// Everything the model read for this request (what prompt-size tiers are based on).
    pub fn prompt(&self) -> u64 {
        self.input + self.cache_write_5m + self.cache_write_1h + self.cache_read
    }

    pub fn cost(&self, p: &Price) -> f64 {
        (self.input as f64 * p.input
            + self.output as f64 * p.output
            + self.cache_write_5m as f64 * p.cache_write_5m
            + self.cache_write_1h as f64 * p.cache_write_1h
            + self.cache_read as f64 * p.cache_read)
            / 1_000_000.0
    }
}

/// Today's accumulated usage for one provider.
#[derive(Serialize, Clone, Debug, Default)]
pub struct DailyUsage {
    pub tokens: u64,
    /// API-equivalent cost of the tokens whose model has a published price.
    pub cost: f64,
    /// Tokens from models without a published price (not included in `cost`).
    pub unpriced_tokens: u64,
    pub unpriced_models: Vec<String>,
}

impl DailyUsage {
    pub fn add(&mut self, model: &str, usage: &TokenUsage, price: Option<Price>) {
        let total = usage.total();
        if total == 0 {
            return;
        }
        self.tokens += total;
        match price {
            Some(p) => self.cost += usage.cost(&p),
            None => {
                self.unpriced_tokens += total;
                if !self.unpriced_models.iter().any(|m| m == model) {
                    self.unpriced_models.push(model.to_string());
                }
            }
        }
    }
}

/// `model` is `prefix` or `prefix-<suffix>` (dated snapshot, variant, ...).
fn matches(model: &str, prefix: &str) -> bool {
    model == prefix
        || (model.starts_with(prefix) && model.as_bytes().get(prefix.len()) == Some(&b'-'))
}

// ── Anthropic ─────────────────────────────────────────────────────────────────

struct ClaudeRow {
    prefix: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    /// Supports `inference_geo` (Claude 4.6 and later): "us" costs 1.1x.
    geo: bool,
    /// Fast mode price multiplier, when the model supports it.
    fast: Option<f64>,
}

const fn c(
    prefix: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    geo: bool,
    fast: Option<f64>,
) -> ClaudeRow {
    ClaudeRow {
        prefix,
        input,
        output,
        cache_read,
        geo,
        fast,
    }
}

// Ordered: more specific prefixes first.
const CLAUDE: &[ClaudeRow] = &[
    c("claude-fable-5-1", 10.0, 50.0, 0.25, true, None),
    c("claude-mythos-5-1", 10.0, 50.0, 0.25, true, None),
    c("claude-fable-5", 10.0, 50.0, 1.0, true, None),
    c("claude-mythos-5", 10.0, 50.0, 1.0, true, None),
    c("claude-opus-5-5", 4.0, 20.0, 0.20, true, Some(2.0)),
    c("claude-opus-5", 5.0, 25.0, 0.50, true, Some(2.0)),
    c("claude-opus-4-8", 5.0, 25.0, 0.50, true, Some(2.0)),
    c("claude-opus-4-7", 5.0, 25.0, 0.50, true, None),
    c("claude-opus-4-6", 5.0, 25.0, 0.50, true, None),
    c("claude-opus-4-5", 5.0, 25.0, 0.50, false, None),
    c("claude-opus-4-1", 15.0, 75.0, 1.50, false, None),
    c("claude-opus-4", 15.0, 75.0, 1.50, false, None),
    c("claude-sonnet-5-5", 2.0, 10.0, 0.10, true, None),
    c("claude-sonnet-5", 2.0, 10.0, 0.20, true, None),
    c("claude-sonnet-4-6", 3.0, 15.0, 0.30, true, None),
    c("claude-sonnet-4-5", 3.0, 15.0, 0.30, false, None),
    c("claude-sonnet-4", 3.0, 15.0, 0.30, false, None),
    c("claude-3-7-sonnet", 3.0, 15.0, 0.30, false, None),
    c("claude-haiku-5-5", 0.10, 0.50, 0.01, true, None),
    c("claude-haiku-4-5", 1.0, 5.0, 0.10, false, None),
    c("claude-3-5-haiku", 0.80, 4.0, 0.08, false, None),
];

/// Price of one Claude request. `prompt_tokens` selects Haiku 5.5's long-prompt tier.
pub fn claude(model: &str, prompt_tokens: u64, fast: bool, geo_us: bool) -> Option<Price> {
    let row = CLAUDE.iter().find(|r| matches(model, r.prefix))?;
    let (mut input, mut output, mut cache_read) = (row.input, row.output, row.cache_read);
    if row.prefix == "claude-haiku-5-5" && prompt_tokens > 100_000 {
        (input, output, cache_read) = (0.50, 2.50, 0.05);
    }
    let mut price = Price {
        input,
        output,
        cache_write_5m: input * 1.25,
        cache_write_1h: input * 2.0,
        cache_read,
    };
    if fast {
        if let Some(m) = row.fast {
            price = price.scaled(m, m);
        }
    }
    if geo_us && row.geo {
        price = price.scaled(1.1, 1.1);
    }
    Some(price)
}

// ── OpenAI ────────────────────────────────────────────────────────────────────

struct OpenAiRow {
    prefix: &'static str,
    input: f64,
    /// `None`: no cached-input discount.
    cached: Option<f64>,
    output: f64,
    cache_write: Option<f64>,
    /// Requests over 272K input tokens cost 2x input / 1.5x output.
    long_context: bool,
}

const fn o(
    prefix: &'static str,
    input: f64,
    cached: Option<f64>,
    output: f64,
    cache_write: Option<f64>,
    long_context: bool,
) -> OpenAiRow {
    OpenAiRow {
        prefix,
        input,
        cached,
        output,
        cache_write,
        long_context,
    }
}

const OPENAI: &[OpenAiRow] = &[
    o("gpt-6-astra", 10.0, Some(1.0), 50.0, Some(12.5), true),
    o("gpt-6.1-sol", 2.0, Some(0.10), 10.0, Some(2.5), true),
    o("gpt-6-sol", 2.0, Some(0.20), 10.0, Some(2.5), true),
    o("gpt-6-luna", 0.10, Some(0.01), 0.50, Some(0.125), true),
    o("gpt-5.6-sol", 4.0, Some(0.40), 20.0, Some(5.0), true),
    o("gpt-5.6-terra", 2.0, Some(0.20), 12.0, Some(2.5), true),
    o("gpt-5.6-luna", 0.20, Some(0.02), 1.20, Some(0.25), true),
    o("gpt-5.5-pro", 30.0, None, 180.0, None, true),
    o("gpt-5.5", 5.0, Some(0.50), 30.0, None, true),
    o("gpt-5.4-mini", 0.75, Some(0.075), 4.50, None, false),
    o("gpt-5.4-nano", 0.20, Some(0.02), 1.25, None, false),
    o("gpt-5.4-pro", 30.0, None, 180.0, None, true),
    o("gpt-5.4", 2.50, Some(0.25), 15.0, None, true),
    o("gpt-5.3-codex", 1.75, Some(0.175), 14.0, None, false),
    o("gpt-5.2-pro", 21.0, None, 168.0, None, false),
    o("gpt-5.2", 1.75, Some(0.175), 14.0, None, false),
    o("gpt-5.1", 1.25, Some(0.125), 10.0, None, false),
    o("gpt-5-mini", 0.25, Some(0.025), 2.0, None, false),
    o("gpt-5-nano", 0.05, Some(0.005), 0.40, None, false),
    o("gpt-5-pro", 15.0, None, 120.0, None, false),
    o("gpt-5", 1.25, Some(0.125), 10.0, None, false),
];

pub fn openai(model: &str, prompt_tokens: u64) -> Option<Price> {
    let row = OPENAI.iter().find(|r| matches(model, r.prefix))?;
    let price = Price {
        input: row.input,
        output: row.output,
        cache_write_5m: row.cache_write.unwrap_or(row.input),
        cache_write_1h: row.cache_write.unwrap_or(row.input),
        cache_read: row.cached.unwrap_or(row.input),
    };
    Some(if row.long_context && prompt_tokens > 272_000 {
        price.scaled(2.0, 1.5)
    } else {
        price
    })
}

// ── Google ────────────────────────────────────────────────────────────────────

struct GeminiRow {
    prefix: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    /// Prices for prompts over 200K tokens, when the model has that tier.
    long: Option<(f64, f64, f64)>,
}

const fn g(
    prefix: &'static str,
    input: f64,
    output: f64,
    cache_read: f64,
    long: Option<(f64, f64, f64)>,
) -> GeminiRow {
    GeminiRow {
        prefix,
        input,
        output,
        cache_read,
        long,
    }
}

const GEMINI: &[GeminiRow] = &[
    g("gemini-3.8-flash", 0.75, 3.75, 0.075, None),
    g("gemini-3.6-flash", 0.75, 3.75, 0.075, None),
    g("gemini-3.5-flash-lite", 0.30, 2.50, 0.03, None),
    g("gemini-3.1-flash-lite", 0.25, 1.50, 0.025, None),
    g("gemini-3.1-pro", 2.0, 12.0, 0.20, Some((4.0, 18.0, 0.40))),
    g("gemini-3-flash", 0.50, 3.0, 0.05, None),
    g(
        "gemini-2.5-pro",
        1.25,
        10.0,
        0.125,
        Some((2.50, 15.0, 0.25)),
    ),
    g("gemini-2.5-flash-lite", 0.10, 0.40, 0.01, None),
    g("gemini-2.5-flash", 0.30, 2.50, 0.03, None),
];

pub fn gemini(model: &str, prompt_tokens: u64) -> Option<Price> {
    let row = GEMINI.iter().find(|r| matches(model, r.prefix))?;
    let (input, output, cache_read) = match row.long {
        Some(long) if prompt_tokens > 200_000 => long,
        _ => (row.input, row.output, row.cache_read),
    };
    // Gemini bills cache storage per hour, not writes; writes are plain input.
    Some(Price {
        input,
        output,
        cache_write_5m: input,
        cache_write_1h: input,
        cache_read,
    })
}

/// Any provider's model, routed by its name (Antigravity can run all three).
pub fn any(model: &str, prompt_tokens: u64) -> Option<Price> {
    if model.starts_with("claude") {
        claude(model, prompt_tokens, false, false)
    } else if model.starts_with("gemini") {
        gemini(model, prompt_tokens)
    } else {
        openai(model, prompt_tokens)
    }
}

// ── Local-day helpers ─────────────────────────────────────────────────────────

/// Local date (YYYY-MM-DD) of an RFC 3339 timestamp.
pub fn local_date_of(rfc3339: &str) -> Option<chrono::NaiveDate> {
    chrono::DateTime::parse_from_rfc3339(rfc3339)
        .ok()
        .map(|d| d.with_timezone(&chrono::Local).date_naive())
}

pub fn local_date_of_unix(secs: i64) -> Option<chrono::NaiveDate> {
    chrono::DateTime::from_timestamp(secs, 0).map(|d| d.with_timezone(&chrono::Local).date_naive())
}

pub fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

/// Whether the file was written on `day` or later (local time); older files can't hold that day's usage.
pub fn modified_since(path: &Path, day: chrono::NaiveDate) -> bool {
    let Ok(modified) = std::fs::metadata(path).and_then(|m| m.modified()) else {
        return false;
    };
    let Ok(age) = modified.duration_since(SystemTime::UNIX_EPOCH) else {
        return false;
    };
    local_date_of_unix(age.as_secs() as i64).is_some_and(|d| d >= day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_prefixes_pick_the_specific_model() {
        assert_eq!(
            claude("claude-opus-5-5", 0, false, false).unwrap().input,
            4.0
        );
        assert_eq!(claude("claude-opus-5", 0, false, false).unwrap().input, 5.0);
        assert_eq!(
            claude("claude-opus-4-1-20250805", 0, false, false)
                .unwrap()
                .input,
            15.0
        );
        assert_eq!(
            claude("claude-sonnet-4-5-20250929", 0, false, false)
                .unwrap()
                .input,
            3.0
        );
        assert_eq!(
            claude("claude-sonnet-5-5", 0, false, false)
                .unwrap()
                .cache_read,
            0.10
        );
        assert!(claude("<synthetic>", 0, false, false).is_none());
    }

    #[test]
    fn claude_cache_writes_and_modifiers() {
        let p = claude("claude-opus-5-5", 0, false, false).unwrap();
        assert_eq!((p.cache_write_5m, p.cache_write_1h), (5.0, 8.0));
        let fast = claude("claude-opus-5-5", 0, true, false).unwrap();
        assert_eq!((fast.input, fast.output), (8.0, 40.0));
        // Opus 4.6 has no fast mode: billed at standard rates.
        assert_eq!(
            claude("claude-opus-4-6", 0, true, false).unwrap().input,
            5.0
        );
        let us = claude("claude-sonnet-4-6", 0, false, true).unwrap();
        assert!((us.input - 3.3).abs() < 1e-9);
        assert_eq!(
            claude("claude-sonnet-4-5", 0, false, true).unwrap().input,
            3.0
        );
        assert_eq!(
            claude("claude-haiku-5-5", 150_000, false, false)
                .unwrap()
                .input,
            0.50
        );
    }

    #[test]
    fn openai_prefixes_and_long_context() {
        assert_eq!(openai("gpt-6-sol", 0).unwrap().cache_read, 0.20);
        assert_eq!(openai("gpt-6.1-sol", 0).unwrap().cache_read, 0.10);
        assert_eq!(openai("gpt-5.1-codex", 0).unwrap().input, 1.25);
        assert_eq!(openai("gpt-5-codex", 0).unwrap().input, 1.25);
        assert_eq!(openai("gpt-5-mini", 0).unwrap().input, 0.25);
        assert_eq!(openai("gpt-5.4-mini", 0).unwrap().input, 0.75);
        let long = openai("gpt-6-sol", 300_000).unwrap();
        assert_eq!((long.input, long.output), (4.0, 15.0));
        assert!(openai("gpt-daybreak-blue-latest", 0).is_none());
    }

    #[test]
    fn gemini_prefixes_and_tiers() {
        assert_eq!(gemini("gemini-3.8-flash", 0).unwrap().cache_read, 0.075);
        assert_eq!(
            gemini("gemini-3.1-pro-preview", 250_000).unwrap().input,
            4.0
        );
        assert_eq!(gemini("gemini-3-flash-preview", 0).unwrap().input, 0.50);
        assert!(gemini("gemini-pro-default", 0).is_none());
    }

    #[test]
    fn daily_usage_tracks_unpriced_models() {
        let mut d = DailyUsage::default();
        let u = TokenUsage {
            input: 1_000_000,
            output: 0,
            ..Default::default()
        };
        d.add(
            "claude-opus-5-5",
            &u,
            claude("claude-opus-5-5", 0, false, false),
        );
        d.add("mystery", &u, None);
        d.add("mystery", &u, None);
        assert_eq!(d.tokens, 3_000_000);
        assert_eq!(d.cost, 4.0);
        assert_eq!(d.unpriced_tokens, 2_000_000);
        assert_eq!(d.unpriced_models, vec!["mystery"]);
    }
}
