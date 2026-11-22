//! One price table (master plan F2, 2026-10-08): per-million-token prices,
//! each with a dated source, for the models Hollow runs.
//!
//! **When it is used.** Only for a model the agent itself could not price:
//! a Claude `modelUsage` entry whose `costBasis` is `unknown`
//! (`usage::claude_result_usage`), and a Codex run, which reports tokens but
//! never a price ([`list_usd`], overnight 9 A13: the row is `estimated` at
//! the price the person entered). A provider's own figure is kept whenever
//! the provider says it priced the run, because only then is it known to be
//! right. A model in neither place stays unpriced: its cost is unknown,
//! never $0 (no built-in row prices a Codex model).
//!
//! **Where it lives.** The built-in rows ([`BUILT_IN`]) plus the person's
//! own rows in `usage-prices.json` beside `agent-settings.json` (app data,
//! outside the vault), written only by `usage.set_price` (a confirm action:
//! a price changes every later cost). A stored row replaces the built-in row
//! for the same model. Schema v1, camelCase: `{schemaVersion: 1, prices:
//! [Price]}`. An unreadable file is reported and ignored (built-ins apply).
//!
//! **One rate per model**, never chosen per call: Claude's `modelUsage` is
//! cumulative on `--resume` and turns are recorded as differences, so a tier
//! picked per call would reprice the session's history (see `usage.rs`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{PoisonError, RwLock};

pub const FILE: &str = "usage-prices.json";
const SCHEMA: u32 = 1;
const MODEL_MAX: usize = 80;
const SOURCE_MAX: usize = 200;
/// No model costs more than this per million tokens; a larger value is a
/// typo (cents for dollars).
const PRICE_MAX: f64 = 1_000.0;

/// One model's list price, USD per million tokens.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Price {
    pub model: String,
    pub input: f64,
    /// A cache write (Claude Code writes the one-hour cache: 2× input).
    pub cache_write: f64,
    pub cache_read: f64,
    pub output: f64,
    /// Where the figures come from, in words.
    pub source: String,
    /// The date the source was read (`YYYY-MM-DD`).
    pub as_of: String,
}

/// The source every built-in row cites.
const LIST_2026_09_25: &str =
    "Anthropic list price (claude-api model table cached 2026-09-25); cache write = 2× input (one-hour cache)";

fn row(model: &str, input: f64, cache_read: f64, output: f64, source: &str, as_of: &str) -> Price {
    Price {
        model: model.into(),
        input,
        cache_write: input * 2.0,
        cache_read,
        output,
        source: source.into(),
        as_of: as_of.into(),
    }
}

/// The built-in rows. Haiku 5.5 is the 2026-10-07 row Claude Code 2.1.286
/// could not price (its ≤100K-token prompt rate; longer prompts cost 5×).
pub fn built_in() -> Vec<Price> {
    vec![
        row(
            "claude-fable-5-1",
            10.0,
            0.25,
            50.0,
            LIST_2026_09_25,
            "2026-09-25",
        ),
        row(
            "claude-opus-5-5",
            4.0,
            0.20,
            20.0,
            LIST_2026_09_25,
            "2026-09-25",
        ),
        row(
            "claude-sonnet-5-5",
            2.0,
            0.20,
            10.0,
            LIST_2026_09_25,
            "2026-09-25",
        ),
        row(
            "claude-haiku-4-5",
            1.0,
            0.10,
            5.0,
            LIST_2026_09_25,
            "2026-09-25",
        ),
        row(
            "claude-haiku-5-5",
            0.10,
            0.01,
            0.50,
            "Anthropic launch price, prompts up to 100K tokens (2026-10-07)",
            "2026-10-07",
        ),
    ]
    .into_iter()
    .chain(gemini_rows())
    .collect()
}

const GEMINI_2026_11_20: &str =
    "Google Gemini API standard tier, paid (aistudio.google.com/docs/pricing, read 2026-11-20)";

/// A Gemini row: Gemini bills no cache write (implicit caching reads only),
/// so a write costs the input rate. Keyed by the base model (`gemini-4-argon`,
/// never a `-low`/`-high` variant: Antigravity's ledger names the base).
fn gemini(model: &str, input: f64, cache_read: f64, output: f64, note: &str) -> Price {
    Price {
        model: model.into(),
        input,
        cache_write: input,
        cache_read,
        output,
        source: format!("{GEMINI_2026_11_20}{note}"),
        as_of: "2026-11-20".into(),
    }
}

/// Antigravity's models at Google's list prices (the prepaid AI Studio
/// credits bill at these, so a run is `estimated`, never `reported`). One
/// rate per model: Argon and Carbon rise to $4 / $0.20 / $20 on 2026-12-01,
/// and the 3.x Flash rows to $1.50 / $0.15 / $7.50 on 2027-01-01 (they hold
/// today's rate until then; change the row on that date). Gemini 3.7 Flash
/// is not on the page, so it stays unpriced. Prompts over 200K tokens on 3.1
/// Pro bill higher ($4 / $0.40 / $18); the row is the up-to-200K rate.
fn gemini_rows() -> Vec<Price> {
    let rise = "; rises 2026-12-01 to 4.00 / 0.20 / 20.00";
    let flash = "; rises 2027-01-01 to 1.50 / 0.15 / 7.50";
    vec![
        gemini("gemini-4-argon", 2.0, 0.10, 10.0, rise),
        gemini("gemini-4-carbon", 2.0, 0.10, 10.0, rise),
        gemini("gemini-4-neon", 3.0, 0.15, 15.0, ""),
        gemini("gemini-4-hydrogen", 1.5, 0.075, 7.5, ""),
        gemini("gemini-3.8-flash", 0.75, 0.075, 3.75, flash),
        gemini("gemini-3.6-flash", 0.75, 0.075, 3.75, flash),
        gemini(
            "gemini-3.1-pro",
            2.0,
            0.20,
            12.0,
            "; prompts over 200K tokens bill 4.00 / 0.40 / 18.00",
        ),
    ]
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    schema_version: u32,
    #[serde(default)]
    prices: Vec<Price>,
}

/// The person's rows, once read (`None` until the first lookup).
static STORED: RwLock<Option<Vec<Price>>> = RwLock::new(None);

fn stored_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::agent_settings::app_data_dir()
        .ok()
        .map(|dir| dir.join(FILE))
}

fn read_file(path: &Path) -> Result<Vec<Price>, String> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("{FILE} unreadable: {e}")),
    };
    let stored: Stored =
        serde_json::from_str(&text).map_err(|e| format!("{FILE} needs repair: {e}"))?;
    if stored.schema_version != SCHEMA {
        return Err(format!(
            "Unsupported {FILE} version {}",
            stored.schema_version
        ));
    }
    for price in &stored.prices {
        validate(price)?;
    }
    Ok(stored.prices)
}

#[cfg(test)]
thread_local! {
    /// Tests: the rows this thread's lookups see as the person's own (the
    /// stored file is never read under test), so a test can price a model
    /// without touching any other test's table.
    static TEST_STORED: std::cell::RefCell<Vec<Price>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Tests: run `f` with `rows` as this thread's saved prices.
#[cfg(test)]
pub fn with_saved_rows<T>(rows: Vec<Price>, f: impl FnOnce() -> T) -> T {
    TEST_STORED.with(|cell| *cell.borrow_mut() = rows);
    let out = f();
    TEST_STORED.with(|cell| cell.borrow_mut().clear());
    out
}

#[cfg(test)]
fn test_rows() -> Option<Vec<Price>> {
    Some(TEST_STORED.with(|cell| cell.borrow().clone()))
}

#[cfg(not(test))]
fn test_rows() -> Option<Vec<Price>> {
    None
}

fn stored() -> Vec<Price> {
    if let Some(rows) = test_rows() {
        return rows;
    }
    if let Some(rows) = STORED
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
    {
        return rows.clone();
    }
    let rows = match stored_path().map(|p| read_file(&p)) {
        Some(Ok(rows)) => rows,
        Some(Err(error)) => {
            eprintln!("[usage] {error}; using the built-in prices");
            Vec::new()
        }
        None => Vec::new(),
    };
    *STORED.write().unwrap_or_else(PoisonError::into_inner) = Some(rows.clone());
    rows
}

/// The table as it applies: the person's rows, then every built-in row they
/// did not replace, sorted by model.
pub fn table() -> Vec<Price> {
    merge(stored(), built_in())
}

fn merge(stored: Vec<Price>, built_in: Vec<Price>) -> Vec<Price> {
    let mut rows = stored;
    for price in built_in {
        if !rows.iter().any(|p| p.model == price.model) {
            rows.push(price);
        }
    }
    rows.sort_by(|a, b| a.model.cmp(&b.model));
    rows
}

/// One row of [`table`] as Settings, Advanced, Prices lists it (overnight 9
/// A13): the price, and whether it is the person's own row (which Remove
/// drops, so the built-in row, if any, applies again).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceRow {
    #[serde(flatten)]
    pub price: Price,
    pub saved: bool,
}

/// [`table`], each row marked saved or built in.
pub fn rows() -> Vec<PriceRow> {
    let saved = stored();
    table()
        .into_iter()
        .map(|price| PriceRow {
            saved: saved.iter().any(|row| row.model == price.model),
            price,
        })
        .collect()
}

/// The price for `model` (already normalized: no date suffix), if any.
pub fn lookup(model: &str) -> Option<Price> {
    table().into_iter().find(|p| p.model == model)
}

/// RF-817-F1: whether a row for model id `row` prices `model`: the same id,
/// or that id plus a release date in either form the providers use,
/// Anthropic's `-YYYYMMDD` (`claude-haiku-4-5-20251001`) or OpenAI's
/// `-YYYY-MM-DD` (`gpt-5-2025-08-07`). Never a longer id that merely starts
/// with it: `gpt-5` prices neither `gpt-5-mini`, `gpt-5.1` nor `gpt-50`.
fn row_matches(row: &str, model: &str) -> bool {
    let Some(rest) = model.strip_prefix(row) else {
        return false;
    };
    let Some(date) = rest.strip_prefix('-') else {
        return rest.is_empty();
    };
    let date = date.as_bytes();
    match date.len() {
        8 => date.iter().all(u8::is_ascii_digit),
        10 => date.iter().enumerate().all(|(at, b)| match at {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        }),
        _ => false,
    }
}

/// The row that prices `model` ([`row_matches`]); the longest when several
/// do (a row saved for a dated id beats the undated one).
fn row_for<'a>(table: &'a [Price], model: &str) -> Option<&'a Price> {
    table
        .iter()
        .filter(|p| row_matches(&p.model, model))
        .max_by_key(|p| p.model.len())
}

/// A running turn's cost so far, in USD, from its per-message tokens by
/// model (turn guards, 2026-10-09: the night guard reads it mid-turn, before
/// the CLI's own figure exists). Deliberately high, never low: `input`
/// (uncached input plus cache writes) is priced at the cache-write rate, and
/// a model with no row (or `unknown`) at the dearest row in the table. A
/// dated id matches its row ([`row_matches`]). The real cost replaces this
/// at the turn's end.
pub fn estimate_usd<'a>(
    by_model: impl IntoIterator<Item = (&'a String, &'a crate::usage::TokenCounts)>,
) -> f64 {
    let table = table();
    let dearest = table
        .iter()
        .max_by(|a, b| a.output.total_cmp(&b.output))
        .cloned();
    by_model
        .into_iter()
        .filter_map(|(model, tokens)| {
            let price = row_for(&table, model).or(dearest.as_ref())?;
            let m = |n: u64| n as f64 / 1_000_000.0;
            Some(
                m(tokens.input) * price.cache_write
                    + m(tokens.cached_input) * price.cache_read
                    + m(tokens.output) * price.output,
            )
        })
        .sum()
}

/// OB07: a lower bound, in USD, on what a run that reported no cost spent
/// (killed, interrupted, an unverified resume), from the tokens it streamed
/// by model: the ledger's `cost_floor_usd`, never its cost. The mirror of
/// [`estimate_usd`], deliberately low, never high: `input` (uncached input
/// plus cache writes) is priced at the input rate, and a model with no row
/// (`unknown`, and Codex unless someone adds a row for it) adds nothing. A
/// dated id matches its row ([`row_matches`]). `None` when no model has a
/// row, or the floor is $0.
pub fn floor_usd<'a>(
    by_model: impl IntoIterator<Item = (&'a String, &'a crate::usage::TokenCounts)>,
) -> Option<f64> {
    let table = table();
    let floor: f64 = by_model
        .into_iter()
        .filter_map(|(model, tokens)| {
            let price = row_for(&table, model)?;
            let m = |n: u64| n as f64 / 1_000_000.0;
            Some(
                m(tokens.input) * price.input
                    + m(tokens.cached_input) * price.cache_read
                    + m(tokens.output) * price.output,
            )
        })
        .sum();
    (floor.is_finite() && floor > 0.0).then_some(floor)
}

/// Overnight 9 A13: a Codex run's cost at the price the person entered for
/// its model (Codex reports tokens, never a price): uncached input at the
/// input rate, cached input at the cache-read rate, output at the output
/// rate. A dated id matches its row ([`row_matches`]). `None` when no row
/// matches, so a model nobody priced stays unpriced. Returns the row too,
/// so the ledger can say which price it used.
pub fn list_usd(model: &str, tokens: &crate::usage::TokenCounts) -> Option<(f64, Price)> {
    let price = row_for(&table(), model)?.clone();
    let m = |n: u64| n as f64 / 1_000_000.0;
    let usd = m(tokens.input) * price.input
        + m(tokens.cached_input) * price.cache_read
        + m(tokens.output) * price.output;
    usd.is_finite().then_some((usd, price))
}

pub fn validate(price: &Price) -> Result<(), String> {
    let model = price.model.trim();
    if model.is_empty()
        || model.len() > MODEL_MAX
        || !model
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '.' | '_'))
    {
        return Err(format!(
            "'{}' is not a model id (lowercase letters, digits, '-', '.', '_').",
            price.model
        ));
    }
    for (name, value) in [
        ("input", price.input),
        ("cacheWrite", price.cache_write),
        ("cacheRead", price.cache_read),
        ("output", price.output),
    ] {
        if !value.is_finite() || !(0.0..=PRICE_MAX).contains(&value) {
            return Err(format!(
                "{name} must be between 0 and {PRICE_MAX} dollars per million tokens."
            ));
        }
    }
    if price.source.trim().is_empty() || price.source.chars().count() > SOURCE_MAX {
        return Err(format!(
            "Say where the price comes from (1-{SOURCE_MAX} characters)."
        ));
    }
    if chrono::NaiveDate::parse_from_str(price.as_of.trim(), "%Y-%m-%d").is_err() {
        return Err("asOf must be a date (YYYY-MM-DD).".into());
    }
    Ok(())
}

/// Save (or, with `remove`, drop) the person's row for `price.model`, and
/// return the table as it now applies. GUI only (`usage.set_price`).
pub fn set(price: Price, remove: bool) -> Result<Vec<Price>, String> {
    let path = stored_path().ok_or("Prices can only be changed in the app.")?;
    let rows = set_at(&path, price, remove)?;
    *STORED.write().unwrap_or_else(PoisonError::into_inner) = Some(rows.clone());
    Ok(merge(rows, built_in()))
}

/// Write the person's rows to `path` and return them (not merged).
fn set_at(path: &Path, price: Price, remove: bool) -> Result<Vec<Price>, String> {
    if !remove {
        validate(&price)?;
    }
    let mut rows = read_file(path)?;
    rows.retain(|p| p.model != price.model.trim());
    if !remove {
        rows.push(Price {
            model: price.model.trim().to_string(),
            source: price.source.trim().to_string(),
            as_of: price.as_of.trim().to_string(),
            ..price
        });
    }
    let value = serde_json::to_value(Stored {
        schema_version: SCHEMA,
        prices: rows.clone(),
    })
    .map_err(|e| e.to_string())?;
    let map: std::collections::BTreeMap<String, serde_json::Value> =
        serde_json::from_value(value).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    }
    crate::setup::publish_config(path, &map)?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Overnight 9 A13: the Prices list marks the person's rows (one that
    /// replaces a built-in row too) and leaves the built-in ones unmarked;
    /// a Codex model is priced only by a row for it.
    #[test]
    fn the_prices_list_marks_saved_rows() {
        let mine = Price {
            model: "claude-opus-5-5".into(),
            source: "my contract".into(),
            ..built_in()[1].clone()
        };
        let codex = Price {
            model: "gpt-test".into(),
            source: "test row".into(),
            ..built_in()[1].clone()
        };
        with_saved_rows(vec![mine.clone(), codex], || {
            let rows = rows();
            let get = |model: &str| rows.iter().find(|r| r.price.model == model).unwrap();
            assert!(get("claude-opus-5-5").saved);
            assert_eq!(get("claude-opus-5-5").price.source, "my contract");
            assert!(get("gpt-test").saved);
            assert!(!get("claude-haiku-5-5").saved);
            let json = serde_json::to_value(get("gpt-test")).unwrap();
            assert_eq!(
                (json["model"].as_str(), json["saved"].as_bool()),
                (Some("gpt-test"), Some(true))
            );
            let tokens = crate::usage::TokenCounts {
                input: 1_000_000,
                cached_input: 0,
                output: 0,
            };
            assert_eq!(
                list_usd("gpt-test-2026-10-01", &tokens).map(|(usd, _)| usd),
                Some(4.0)
            );
            assert_eq!(list_usd("gpt-other", &tokens), None);
        });
        assert!(rows().iter().all(|r| !r.saved));
    }

    /// RF-817-F1: a row prices its own model id, never a longer one that
    /// merely starts with it (`gpt-5` is not `gpt-5-mini`'s price), at all
    /// three sites. A bare year, or a date with more after it, is a
    /// different model too.
    #[test]
    fn a_price_row_never_prices_a_longer_model_id() {
        let gpt5 = Price {
            model: "gpt-5".into(),
            input: 1.0,
            cache_write: 1.0,
            cache_read: 0.10,
            output: 8.0,
            source: "test row".into(),
            as_of: "2026-10-20".into(),
        };
        with_saved_rows(vec![gpt5], || {
            let t = crate::usage::TokenCounts {
                input: 0,
                cached_input: 0,
                output: 1_000_000,
            };
            for sibling in [
                "gpt-5-mini",
                "gpt-5.1",
                "gpt-5-codex",
                "gpt-50",
                "gpt-5-2025",
                "gpt-5-2025-08",
                "gpt-5-202508071",
                "gpt-5-2025-08-07-mini",
                "gpt-5-20250807-mini",
                "gpt-5-2025_08_07",
            ] {
                assert_eq!(list_usd(sibling, &t), None, "{sibling}");
                assert_eq!(floor_usd([(&sibling.to_string(), &t)]), None, "{sibling}");
                // Unpriced, so the running estimate errs high: the dearest
                // row (Fable's $50 output), never gpt-5's $8.
                let estimate = estimate_usd([(&sibling.to_string(), &t)]);
                assert!((estimate - 50.0).abs() < 1e-9, "{sibling}: {estimate}");
            }
            let own = list_usd("gpt-5", &t).map(|(usd, p)| (usd, p.model));
            assert_eq!(own, Some((8.0, "gpt-5".to_string())));
        });
    }

    /// RF-817-F1: a dated id still finds its row, in both date forms the
    /// providers use: Anthropic's `-YYYYMMDD` and OpenAI's `-YYYY-MM-DD`.
    #[test]
    fn a_dated_model_id_still_finds_its_row() {
        let codex = Price {
            model: "gpt-5".into(),
            input: 1.0,
            cache_write: 1.0,
            cache_read: 0.10,
            output: 8.0,
            source: "test row".into(),
            as_of: "2026-10-20".into(),
        };
        with_saved_rows(vec![codex], || {
            let t = crate::usage::TokenCounts {
                input: 0,
                cached_input: 0,
                output: 1_000_000,
            };
            let priced = |model: &str| list_usd(model, &t).map(|(usd, p)| (usd, p.model));
            for dated in ["gpt-5-2025-08-07", "gpt-5-20250807"] {
                assert_eq!(priced(dated), Some((8.0, "gpt-5".to_string())), "{dated}");
                let floor = floor_usd([(&dated.to_string(), &t)]);
                assert_eq!(floor, Some(8.0), "{dated}");
                let estimate = estimate_usd([(&dated.to_string(), &t)]);
                assert!((estimate - 8.0).abs() < 1e-9, "{dated}: {estimate}");
            }
            assert_eq!(
                priced("claude-haiku-4-5-20251001"),
                Some((5.0, "claude-haiku-4-5".to_string()))
            );
            assert_eq!(
                priced("claude-opus-5-5-2026-10-01"),
                Some((20.0, "claude-opus-5-5".to_string()))
            );
        });
    }

    /// Turn guards: the running estimate is high, never low. Input (cache
    /// writes included) at the cache-write rate, a dated id at its row, an
    /// unknown model at the dearest row.
    #[test]
    fn a_running_estimate_errs_high() {
        use crate::usage::TokenCounts;
        let m = |input, cached_input, output| TokenCounts {
            input,
            cached_input,
            output,
        };
        let one = |model: &str, t| estimate_usd([(&model.to_string(), &t)]);
        // Opus 5.5: $4 input → $8 cache write, $0.20 cache read, $20 output.
        let opus = one("claude-opus-5-5", m(1_000_000, 1_000_000, 1_000_000));
        assert!((opus - 28.2).abs() < 1e-9, "{opus}");
        let dated = one("claude-opus-5-5-20261001", m(0, 0, 1_000_000));
        assert!((dated - 20.0).abs() < 1e-9, "{dated}");
        let haiku = one("claude-haiku-5-5", m(0, 0, 1_000_000));
        assert!((haiku - 0.5).abs() < 1e-9, "its own row: {haiku}");
        let unknown = one("unknown", m(0, 0, 1_000_000));
        assert!(
            (unknown - 50.0).abs() < 1e-9,
            "the dearest row (Fable): {unknown}"
        );
        assert_eq!(estimate_usd(std::iter::empty()), 0.0);
    }

    /// OB07: the partial floor errs low, the mirror of the estimate above.
    /// Input (cache writes included) at the input rate; Codex and `unknown`
    /// add nothing, and alone have no floor at all.
    #[test]
    fn a_partial_floor_errs_low_and_skips_unpriced_models() {
        use crate::usage::TokenCounts;
        let m = |input, cached_input, output| TokenCounts {
            input,
            cached_input,
            output,
        };
        let one = |model: &str, t| floor_usd([(&model.to_string(), &t)]);
        // Opus 5.5: 1M cache reads × $0.20/M + 10K output × $20/M
        // = $0.20 + $0.20 = $0.40.
        let killed = one("claude-opus-5-5", m(0, 1_000_000, 10_000)).unwrap();
        assert!((killed - 0.40).abs() < 1e-9, "{killed}");
        // Input at $4/M, never the $8 cache-write rate the estimate uses.
        let opus = "claude-opus-5-5".to_string();
        let input = one("claude-opus-5-5-20261001", m(1_000_000, 0, 0)).unwrap();
        assert!((input - 4.0).abs() < 1e-9, "{input}");
        let estimate = estimate_usd([(&opus, &m(1_000_000, 0, 0))]);
        assert!(input < estimate, "{input} vs {estimate}");
        assert_eq!(
            one("gpt-5.3-codex", m(1_000, 1_000, 1_000)),
            None,
            "Codex: no row"
        );
        assert_eq!(
            one("unknown", m(0, 0, 1_000_000)),
            None,
            "never the dearest row"
        );
        assert_eq!(
            one("claude-opus-5-5", m(0, 0, 0)),
            None,
            "no tokens, no floor"
        );
        // Mixed: the unpriced model's tokens add nothing.
        let codex = "gpt-5.3-codex".to_string();
        let mixed = floor_usd([(&opus, &m(0, 0, 1_000_000)), (&codex, &m(0, 0, 1_000_000))]);
        assert!((mixed.unwrap() - 20.0).abs() < 1e-9, "{mixed:?}");
    }

    /// 2026-11-20: Antigravity's models carry Google's published rates,
    /// keyed by base model, no cache write beyond the input rate, valid under
    /// the same rules as a saved row; a level variant or 3.7 Flash has none.
    #[test]
    fn gemini_rows_price_the_base_models_and_only_those() {
        for p in built_in().iter().filter(|p| p.model.starts_with("gemini-")) {
            validate(p).unwrap();
            assert_eq!(p.cache_write, p.input, "{}", p.model);
        }
        let neon = lookup("gemini-4-neon").unwrap();
        assert_eq!(
            (neon.input, neon.cache_read, neon.output),
            (3.0, 0.15, 15.0)
        );
        let hydrogen = lookup("gemini-4-hydrogen").unwrap();
        assert_eq!(
            (hydrogen.input, hydrogen.cache_read, hydrogen.output),
            (1.5, 0.075, 7.5)
        );
        assert!(
            lookup("gemini-4-neon-high").is_none(),
            "a variant is not a model"
        );
        assert!(
            lookup("gemini-3.7-flash").is_none(),
            "not on the page: unpriced"
        );
    }

    #[test]
    fn every_built_in_row_is_valid_dated_and_sourced() {
        let rows = built_in();
        for model in [
            "claude-fable-5-1",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-4-5",
            "claude-haiku-5-5",
        ] {
            let p = rows.iter().find(|p| p.model == model).expect(model);
            validate(p).unwrap();
        }
        let opus = lookup("claude-opus-5-5").unwrap();
        assert_eq!(
            (opus.input, opus.cache_write, opus.cache_read, opus.output),
            (4.0, 8.0, 0.20, 20.0)
        );
        let haiku = lookup("claude-haiku-5-5").unwrap();
        assert_eq!(
            (
                haiku.input,
                haiku.cache_write,
                haiku.cache_read,
                haiku.output
            ),
            (0.10, 0.20, 0.01, 0.50),
            "the 2026-10-07 row is unchanged"
        );
        // Codex and unknown models have no row: their cost stays unknown.
        assert!(lookup("gpt-5.3-codex").is_none());
    }

    #[test]
    fn a_saved_row_replaces_the_built_in_one_and_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE);
        let mine = Price {
            model: "claude-opus-5-5".into(),
            input: 3.0,
            cache_write: 6.0,
            cache_read: 0.3,
            output: 15.0,
            source: "negotiated rate".into(),
            as_of: "2026-10-08".into(),
        };
        let table = merge(set_at(&path, mine.clone(), false).unwrap(), built_in());
        let opus = table.iter().find(|p| p.model == "claude-opus-5-5").unwrap();
        assert_eq!(opus, &mine);
        assert_eq!(
            table
                .iter()
                .filter(|p| p.model == "claude-opus-5-5")
                .count(),
            1
        );
        let table = merge(set_at(&path, mine.clone(), true).unwrap(), built_in());
        assert_eq!(
            table
                .iter()
                .find(|p| p.model == "claude-opus-5-5")
                .unwrap()
                .input,
            4.0
        );
        // Validation.
        let bad = |f: fn(&mut Price)| {
            let mut p = mine.clone();
            f(&mut p);
            set_at(&path, p, false).unwrap_err()
        };
        bad(|p| p.input = f64::NAN);
        bad(|p| p.output = 5_000.0);
        bad(|p| p.source = " ".into());
        bad(|p| p.as_of = "yesterday".into());
        bad(|p| p.model = "Opus 5.5".into());
    }
}
