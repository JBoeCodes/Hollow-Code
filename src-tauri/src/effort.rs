//! Reasoning effort for every agent and every model (user decision,
//! 2026-10-02: "agents shouldn't require a certain effort level, they
//! should all work with every model").
//!
//! Hollow keeps one list of levels, [`LEVELS`], for every agent. A run maps
//! the saved level to the nearest one the chosen model takes ([`resolve`]),
//! never failing on it:
//! - Claude Code takes all five on every model (`--effort`; Haiku accepted
//!   `max`, probe 2026-10-02).
//! - Codex lists each model's levels in its own catalog
//!   (`$CODEX_HOME/models_cache.json`, `supported_reasoning_levels`).
//! - Antigravity selects effort by model id (`gemini-3.1-pro-low` /
//!   `-high`); a conflicting `--effort` is an error, and a model without
//!   variants takes none. The variants come from `agy models`.
//! - Grok's levels differ per model (`grok-4.5` has no `xhigh`); an
//!   invalid level is refused before any API call and the error names the
//!   valid ones, which [`crate::agent_models`] reads once per model.
//!
//! What the catalogs taught is kept in memory ([`remember`]); before a
//! catalog has loaded, Antigravity runs without an effort and Grok within
//! the levels every model takes ([`GROK_SAFE`]).
//!
//! **Ultracode** (merged plan Track U, 2026-10-02) is a sixth, per-chat
//! choice above `max`, never a saved default ([`ULTRACODE`]). Claude Code
//! runs standing dynamic-workflow orchestration with `--effort ultracode`
//! when the installed CLI takes it ([`set_claude_ultracode`]), and otherwise
//! `xhigh` with a note. Every other agent gets its model's top level; the
//! string `ultracode` is never sent to them.
use crate::agent::AgentKind;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Hollow's effort levels, lowest first. Every agent offers all of them.
pub const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// The per-chat choice above [`LEVELS`]: Claude Code's standing
/// dynamic-workflow orchestration, at the top effort elsewhere. Accepted by
/// [`canonical`] for a run; Settings defaults stay [`LEVELS`]
/// (`agent_settings::normalize` refuses it), and [`nearest`] never returns it.
pub const ULTRACODE: &str = "ultracode";

/// What Claude runs instead when its CLI can't take ultracode.
const ULTRACODE_FALLBACK: &str = "xhigh";

/// The levels every Grok model seen so far takes, for a model whose own
/// levels aren't known yet.
const GROK_SAFE: &[&str] = &["low", "medium", "high"];
/// The levels every Codex catalog model takes, for a model the catalog
/// doesn't list.
const CODEX_SAFE: &[&str] = &["low", "medium", "high", "xhigh"];

/// A run's level in Hollow's terms: one of [`LEVELS`] or [`ULTRACODE`],
/// with Codex's old `minimal` read as `low`. `None` for anything else.
pub fn canonical(level: &str) -> Option<&'static str> {
    let level = level.trim();
    if level == "minimal" {
        return Some("low");
    }
    if level == ULTRACODE {
        return Some(ULTRACODE);
    }
    LEVELS.iter().copied().find(|l| *l == level)
}

fn rank(level: &str) -> Option<usize> {
    match level {
        "minimal" => Some(0),
        "ultra" => Some(LEVELS.len()),
        ULTRACODE => Some(LEVELS.len() + 1),
        _ => LEVELS.iter().position(|l| *l == level),
    }
}

/// `level` held at or below `ceiling` (one of [`LEVELS`]), for runs that
/// must finish within a fixed wall clock (the heartbeat's): a higher level,
/// ultracode included, becomes `ceiling`; anything else passes through for
/// [`resolve`] to map or drop as usual. Ranked trimmed, as [`canonical`]
/// reads it.
pub fn at_most(level: Option<String>, ceiling: &'static str) -> Option<String> {
    match (
        level.as_deref().map(str::trim).and_then(rank),
        rank(ceiling),
    ) {
        (Some(have), Some(cap)) if have > cap => Some(ceiling.to_string()),
        _ => level,
    }
}

/// The supported level closest to `requested`; on a tie the lower one.
/// `None` when nothing is supported.
pub fn nearest<'a>(requested: &str, supported: &[&'a str]) -> Option<&'a str> {
    let want = rank(requested)? as i64;
    supported
        .iter()
        .filter(|s| **s != ULTRACODE)
        .filter_map(|s| rank(s).map(|r| (*s, r as i64)))
        .min_by_key(|(_, r)| ((r - want).abs(), *r))
        .map(|(s, _)| s)
}

/// What a run passes for effort: the level flag (`None` leaves it out) and,
/// for Antigravity, the model id that carries the level.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub effort: Option<String>,
    pub model: Option<String>,
    /// Why the run doesn't get what was asked for, when that matters to the
    /// user (an ultracode request on a CLI without it).
    pub note: Option<String>,
}

impl Choice {
    /// The level actually sent: the flag, or for Antigravity the level its
    /// variant id carries.
    pub fn sent(&self) -> Option<String> {
        self.effort.clone().or_else(|| {
            self.model
                .as_deref()
                .and_then(|id| split_agy(id).1)
                .map(str::to_string)
        })
    }
}

/// Whether the installed Claude CLI takes `--effort ultracode`, as last
/// probed (`agent::claude_ultracode_supported`); `None` before any probe.
static CLAUDE_ULTRACODE: Mutex<Option<bool>> = Mutex::new(None);

/// Record the probe's answer for [`resolve`].
pub fn set_claude_ultracode(supported: bool) {
    if let Ok(mut known) = CLAUDE_ULTRACODE.lock() {
        *known = Some(supported);
    }
}

fn claude_ultracode() -> Option<bool> {
    CLAUDE_ULTRACODE.lock().ok().and_then(|known| *known)
}

/// The note shown when Claude can't run ultracode.
pub const ULTRACODE_UNAVAILABLE: &str =
    "Ultracode isn't available on this Claude Code version, so workflows are off; this turn ran at xhigh.";

fn cache() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static CACHE: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn key(kind: AgentKind, model: &str) -> String {
    format!("{}:{model}", kind.as_str())
}

/// Record the levels `model` takes (Grok, from its refusal), or for
/// Antigravity the variant ids of a base model (`level=id` pairs).
pub fn remember(kind: AgentKind, model: &str, levels: Vec<String>) {
    if let Ok(mut map) = cache().lock() {
        map.insert(key(kind, model), levels);
    }
}

fn known(kind: AgentKind, model: &str) -> Option<Vec<String>> {
    cache().lock().ok()?.get(&key(kind, model)).cloned()
}

/// An Antigravity model id split into its base and level suffix:
/// `gemini-3.1-pro-low` → (`gemini-3.1-pro`, `low`). An id without a level
/// suffix is its own base.
pub fn split_agy(id: &str) -> (&str, Option<&str>) {
    for level in ["low", "medium", "high", "max", "xhigh", "minimal"] {
        if let Some(base) = id.strip_suffix(level).and_then(|b| b.strip_suffix('-')) {
            if !base.is_empty() {
                return (base, Some(level));
            }
        }
    }
    (id, None)
}

/// The base model of an Antigravity id, whether it names a level variant
/// (`gemini-4-neon-high` → `gemini-4-neon`) or the base itself. The picker
/// lists bases only (one Effort control, 2026-11-20), so a saved variant id
/// reads as its base everywhere a model is compared.
pub fn agy_base(id: &str) -> &str {
    split_agy(id.trim()).0
}

/// The levels an Antigravity base model offers, lowest first, from its
/// variants in `agy models` (`gemini-4-neon` → low, medium, high, xhigh).
/// Empty before the catalog has loaded or for a model without variants.
/// (Tests read it here; the ACP turn reads it to keep a level the model has.)
#[cfg_attr(not(test), allow(dead_code))]
pub fn agy_levels(base: &str) -> Vec<String> {
    let mut levels: Vec<String> = known(AgentKind::Antigravity, agy_base(base))
        .unwrap_or_default()
        .iter()
        .filter_map(|pair| pair.split_once('=').map(|(level, _)| level.to_string()))
        .collect();
    levels.sort_by_key(|level| rank(level).unwrap_or(usize::MAX));
    levels.dedup();
    levels
}

/// Remember Antigravity's variants from its catalog ids: every base model
/// with level-suffixed ids gets its `level=id` pairs.
pub fn remember_agy_catalog<'a>(ids: impl IntoIterator<Item = &'a str>) {
    let mut bases: HashMap<String, Vec<String>> = HashMap::new();
    for id in ids {
        if let (base, Some(level)) = split_agy(id) {
            bases
                .entry(base.to_string())
                .or_default()
                .push(format!("{level}={id}"));
        }
    }
    for (base, variants) in bases {
        remember(AgentKind::Antigravity, &base, variants);
    }
}

/// Grok's refusal of an unknown level: `… use one of: xhigh, high, medium, low`.
pub fn parse_grok_levels(text: &str) -> Vec<String> {
    let Some(at) = text.find("use one of:") else {
        return Vec::new();
    };
    text[at + "use one of:".len()..]
        .lines()
        .next()
        .unwrap_or("")
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| rank(s).is_some())
        .collect()
}

/// Each Codex catalog model's levels (`slug` → `supported_reasoning_levels`).
pub fn parse_codex_levels(catalog: &serde_json::Value) -> HashMap<String, Vec<String>> {
    let mut out = HashMap::new();
    for m in catalog["models"].as_array().into_iter().flatten() {
        let Some(slug) = m["slug"].as_str() else {
            continue;
        };
        let levels: Vec<String> = m["supported_reasoning_levels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                l["effort"]
                    .as_str()
                    .or_else(|| l.as_str())
                    .map(str::to_string)
            })
            .collect();
        if !levels.is_empty() {
            out.insert(slug.to_string(), levels);
        }
    }
    out
}

fn codex_catalog_levels(model: Option<&str>) -> Option<Vec<String>> {
    let home = std::env::var_os("CODEX_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| dirs::home_dir().map(|p| p.join(".codex")))?;
    let bytes = std::fs::read(home.join("models_cache.json")).ok()?;
    if bytes.len() > 2_000_000 {
        return None;
    }
    let catalog: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let levels = parse_codex_levels(&catalog);
    match model {
        Some(model) => levels.get(model).cloned(),
        // The default model is Codex's choice: only what every model takes.
        None => {
            let mut all = levels.values();
            let first = all.next()?.clone();
            Some(all.fold(first, |acc, l| {
                acc.into_iter().filter(|x| l.contains(x)).collect()
            }))
        }
    }
}

/// The effort (and, for Antigravity, the model id) a run of `kind` on
/// `model` passes for the saved level `requested`.
pub fn resolve(kind: AgentKind, model: Option<&str>, requested: Option<&str>) -> Choice {
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    let Some(requested) = requested.and_then(canonical) else {
        return Choice::default();
    };
    // A self-hosted model's levels (Q3, `self_hosted::remember`), ahead of
    // every Claude rule: the nearest level it takes, never one it lacks,
    // ultracode read as `max`.
    if kind == AgentKind::Claude {
        if let Some(levels) = model.and_then(|m| known(kind, m)) {
            let level = if requested == ULTRACODE {
                "max"
            } else {
                requested
            };
            return resolve_from(level, Some(levels.as_slice()), &[]);
        }
    }
    match kind {
        AgentKind::Claude if requested == ULTRACODE => match claude_ultracode() {
            Some(true) => Choice {
                effort: Some(ULTRACODE.into()),
                ..Choice::default()
            },
            // Unprobed counts as unsupported: never a flag the CLI may not take.
            _ => Choice {
                effort: Some(ULTRACODE_FALLBACK.into()),
                note: Some(ULTRACODE_UNAVAILABLE.into()),
                ..Choice::default()
            },
        },
        AgentKind::Claude => Choice {
            effort: Some(requested.into()),
            ..Choice::default()
        },
        AgentKind::Codex => {
            let levels = codex_catalog_levels(model);
            resolve_from(requested, levels.as_deref(), CODEX_SAFE)
        }
        AgentKind::Grok => {
            let levels = model.and_then(|m| known(kind, m));
            resolve_from(requested, levels.as_deref(), GROK_SAFE)
        }
        AgentKind::Antigravity => {
            let Some(model) = model else {
                return Choice::default();
            };
            let (base, _) = split_agy(model);
            let variants = known(kind, base).unwrap_or_default();
            pick_agy_variant(requested, &variants)
                .map(|id| Choice {
                    model: Some(id),
                    ..Choice::default()
                })
                .unwrap_or_default()
        }
    }
}

fn resolve_from(requested: &str, levels: Option<&[String]>, safe: &[&str]) -> Choice {
    let levels: Vec<&str> = match levels {
        Some(levels) if !levels.is_empty() => levels.iter().map(String::as_str).collect(),
        _ => safe.to_vec(),
    };
    Choice {
        effort: nearest(requested, &levels).map(str::to_string),
        ..Choice::default()
    }
}

/// The variant id (from `level=id` pairs) nearest to `requested`.
fn pick_agy_variant(requested: &str, variants: &[String]) -> Option<String> {
    let pairs: Vec<(&str, &str)> = variants.iter().filter_map(|v| v.split_once('=')).collect();
    let levels: Vec<&str> = pairs.iter().map(|(l, _)| *l).collect();
    let level = nearest(requested, &levels)?;
    pairs
        .iter()
        .find(|(l, _)| *l == level)
        .map(|(_, id)| id.to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn at_most_lowers_only_levels_above_the_ceiling() {
        let capped = |level: Option<&str>| at_most(level.map(str::to_string), "high");
        assert_eq!(capped(Some("max")).as_deref(), Some("high"));
        assert_eq!(capped(Some(" max ")).as_deref(), Some("high"), "trimmed");
        assert_eq!(capped(Some(" low ")).as_deref(), Some(" low "));
        assert_eq!(capped(Some("xhigh")).as_deref(), Some("high"));
        assert_eq!(capped(Some(ULTRACODE)).as_deref(), Some("high"));
        assert_eq!(capped(Some("high")).as_deref(), Some("high"));
        assert_eq!(capped(Some("medium")).as_deref(), Some("medium"));
        assert_eq!(capped(Some("minimal")).as_deref(), Some("minimal"));
        assert_eq!(
            capped(Some("turbo")).as_deref(),
            Some("turbo"),
            "unknown passes through for resolve to drop"
        );
        assert_eq!(capped(None), None);
        assert_eq!(
            at_most(Some("max".into()), "max").as_deref(),
            Some("max"),
            "a level at the ceiling is kept"
        );
    }

    #[test]
    fn nearest_picks_the_closest_level_and_the_lower_on_a_tie() {
        assert_eq!(nearest("max", &["low", "medium", "high"]), Some("high"));
        assert_eq!(
            nearest("xhigh", &["low", "high", "max"]),
            Some("high"),
            "tie goes lower"
        );
        assert_eq!(
            nearest("medium", &["low", "high"]),
            Some("low"),
            "tie goes lower"
        );
        assert_eq!(nearest("low", &["medium", "high"]), Some("medium"));
        assert_eq!(
            nearest("max", &["low", "medium", "high", "xhigh", "max", "ultra"]),
            Some("max")
        );
        assert_eq!(nearest("high", &[]), None);
        assert_eq!(nearest("turbo", &["low"]), None);
    }

    #[test]
    fn saved_levels_are_canonical_and_minimal_reads_as_low() {
        assert_eq!(canonical("minimal"), Some("low"));
        assert_eq!(canonical(" xhigh "), Some("xhigh"));
        assert_eq!(canonical("turbo"), None);
        assert_eq!(
            resolve(AgentKind::Claude, Some("haiku"), Some("turbo")),
            Choice::default(),
            "unknown is left out, never a failure"
        );
        assert_eq!(
            resolve(AgentKind::Claude, Some("haiku"), None),
            Choice::default()
        );
    }

    #[test]
    fn claude_takes_every_level_on_every_model() {
        for level in LEVELS {
            assert_eq!(
                resolve(AgentKind::Claude, Some("haiku"), Some(level))
                    .effort
                    .as_deref(),
                Some(level)
            );
        }
    }

    #[test]
    fn codex_levels_come_from_its_catalog_per_model() {
        let catalog = serde_json::json!({ "models": [
            { "slug": "gpt-5.5", "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "medium" }, { "effort": "high" }, { "effort": "xhigh" }] },
            { "slug": "gpt-6-astra", "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "max" }, { "effort": "ultra" }] },
            { "slug": "no-levels" }
        ]});
        let levels = parse_codex_levels(&catalog);
        assert_eq!(levels["gpt-5.5"], vec!["low", "medium", "high", "xhigh"]);
        assert!(!levels.contains_key("no-levels"));
        assert_eq!(
            resolve_from("max", levels.get("gpt-5.5").map(Vec::as_slice), CODEX_SAFE)
                .effort
                .as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            resolve_from(
                "max",
                levels.get("gpt-6-astra").map(Vec::as_slice),
                CODEX_SAFE
            )
            .effort
            .as_deref(),
            Some("max")
        );
        assert_eq!(
            resolve_from("max", None, CODEX_SAFE).effort.as_deref(),
            Some("xhigh"),
            "an unlisted model stays within what every model takes"
        );
    }

    #[test]
    fn grok_levels_are_read_from_its_refusal() {
        let out = "Error: --effort/--reasoning-effort: unknown effort level 'bogus'; use one of: xhigh, high, medium, low\n";
        assert_eq!(
            parse_grok_levels(out),
            vec!["xhigh", "high", "medium", "low"]
        );
        assert!(parse_grok_levels("some other error").is_empty());
        remember(
            AgentKind::Grok,
            "grok-test-old",
            vec!["high".into(), "medium".into(), "low".into()],
        );
        assert_eq!(
            resolve(AgentKind::Grok, Some("grok-test-old"), Some("max"))
                .effort
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            resolve(AgentKind::Grok, Some("grok-test-unknown"), Some("xhigh"))
                .effort
                .as_deref(),
            Some("high"),
            "unknown model: the safe levels"
        );
    }

    #[test]
    fn antigravity_picks_the_model_variant_for_the_level() {
        assert_eq!(
            split_agy("gemini-3.1-pro-low"),
            ("gemini-3.1-pro", Some("low"))
        );
        assert_eq!(
            split_agy("claude-opus-4-6-thinking"),
            ("claude-opus-4-6-thinking", None)
        );
        assert_eq!(
            split_agy("gpt-oss-120b-medium"),
            ("gpt-oss-120b", Some("medium"))
        );
        remember_agy_catalog([
            "gemini-t1-pro-high",
            "gemini-t1-pro-low",
            "gemini-t1-flash-high",
            "gemini-t1-flash-medium",
            "gemini-t1-flash-low",
            "claude-x-thinking",
        ]);
        let pick =
            |model: &str, level: &str| resolve(AgentKind::Antigravity, Some(model), Some(level));
        assert_eq!(
            pick("gemini-t1-pro-low", "medium"),
            Choice {
                model: Some("gemini-t1-pro-low".into()),
                ..Choice::default()
            },
            "no medium: tie goes lower"
        );
        assert_eq!(
            pick("gemini-t1-pro-low", "max").model.as_deref(),
            Some("gemini-t1-pro-high")
        );
        assert_eq!(
            pick("gemini-t1-flash-low", "high").model.as_deref(),
            Some("gemini-t1-flash-high")
        );
        assert_eq!(
            pick("gemini-t1-flash", "medium").model.as_deref(),
            Some("gemini-t1-flash-medium"),
            "a base id gets its variant"
        );
        assert_eq!(
            pick("claude-x-thinking", "high"),
            Choice::default(),
            "a model without variants takes no effort"
        );
        assert_eq!(
            resolve(AgentKind::Antigravity, None, Some("high")),
            Choice::default(),
            "agy's own default model: left alone"
        );
        assert_eq!(
            pick("never-listed-low", "high"),
            Choice::default(),
            "before the catalog loads: left alone"
        );
    }

    /// 2026-11-20: the picker lists base models and Hollow's one Effort
    /// control picks the variant. A saved variant id reads as its base, each
    /// base reports only the levels it has, and an effort the model lacks
    /// maps to the nearest one it takes (Argon stops at high, Neon has xhigh).
    #[test]
    fn antigravity_base_models_take_one_effort_control() {
        remember_agy_catalog([
            "gemini-t2-neon-xhigh",
            "gemini-t2-neon-low",
            "gemini-t2-neon-high",
            "gemini-t2-neon-medium",
            "gemini-t2-argon-high",
            "gemini-t2-argon-low",
            "gemini-t2-argon-medium",
        ]);
        assert_eq!(agy_base("gemini-t2-neon-high"), "gemini-t2-neon");
        assert_eq!(agy_base(" gemini-t2-neon "), "gemini-t2-neon");
        assert_eq!(
            agy_levels("gemini-t2-neon"),
            ["low", "medium", "high", "xhigh"]
        );
        assert_eq!(
            agy_levels("gemini-t2-argon-low"),
            ["low", "medium", "high"],
            "a variant id asks for its base"
        );
        assert!(agy_levels("never-listed").is_empty());
        let on = |model: &str, level: &str| {
            resolve(AgentKind::Antigravity, Some(model), Some(level))
                .model
                .unwrap_or_default()
        };
        assert_eq!(on("gemini-t2-neon", "xhigh"), "gemini-t2-neon-xhigh");
        assert_eq!(on("gemini-t2-neon", "max"), "gemini-t2-neon-xhigh");
        assert_eq!(on("gemini-t2-argon", "xhigh"), "gemini-t2-argon-high");
        assert_eq!(on("gemini-t2-argon", "low"), "gemini-t2-argon-low");
        assert_eq!(
            on("gemini-t2-neon-high", "low"),
            "gemini-t2-neon-low",
            "Hollow's effort wins over a saved variant's level"
        );
    }

    /// Track U (2026-10-02): ultracode is a run's level, never a saved one,
    /// never a level `nearest` hands back, and never sent to anything but a
    /// Claude CLI that takes it. Elsewhere it is the model's top level.
    #[test]
    fn ultracode_maps_to_each_agents_top_and_only_claude_sees_the_string() {
        assert_eq!(canonical(" ultracode "), Some(ULTRACODE));
        assert!(!LEVELS.contains(&ULTRACODE), "never a Settings default");
        assert_eq!(nearest(ULTRACODE, &["low", "high", "max"]), Some("max"));
        assert_eq!(nearest(ULTRACODE, &["low", "medium", "high"]), Some("high"));
        assert_eq!(
            nearest("max", &["high", ULTRACODE]),
            Some("high"),
            "never returned"
        );
        assert_eq!(nearest(ULTRACODE, &[ULTRACODE]), None);

        let catalog = serde_json::json!({ "models": [
            { "slug": "gpt-test", "supported_reasoning_levels": [{ "effort": "low" }, { "effort": "xhigh" }, { "effort": "ultracode" }] },
        ]});
        let codex = parse_codex_levels(&catalog);
        let codex = resolve_from(
            ULTRACODE,
            codex.get("gpt-test").map(Vec::as_slice),
            CODEX_SAFE,
        );
        assert_eq!(
            codex.sent().as_deref(),
            Some("xhigh"),
            "even a catalog naming it never gets the string"
        );
        remember(
            AgentKind::Grok,
            "grok-test-u",
            vec!["low".into(), "high".into()],
        );
        assert_eq!(
            resolve(AgentKind::Grok, Some("grok-test-u"), Some(ULTRACODE))
                .sent()
                .as_deref(),
            Some("high")
        );
        remember_agy_catalog(["gemini-u1-pro-high", "gemini-u1-pro-low"]);
        let agy = resolve(
            AgentKind::Antigravity,
            Some("gemini-u1-pro-low"),
            Some(ULTRACODE),
        );
        assert_eq!(
            (agy.effort.as_deref(), agy.model.as_deref()),
            (None, Some("gemini-u1-pro-high"))
        );
        assert_eq!(
            agy.sent().as_deref(),
            Some("high"),
            "the variant's level is what was sent"
        );
        for choice in [codex, agy] {
            assert!(choice.note.is_none());
        }
    }

    /// Claude gets `--effort ultracode` only once the CLI is known to take
    /// it; before the probe, or after one that failed (a CLI upgrade that
    /// dropped it), the run degrades to xhigh with a note and never fails.
    #[test]
    fn claude_ultracode_follows_the_probe_and_degrades_with_a_note() {
        let _probe = PROBE_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let saved = claude_ultracode();
        let run = || resolve(AgentKind::Claude, Some("opus"), Some(ULTRACODE));
        *CLAUDE_ULTRACODE.lock().unwrap() = None;
        assert_eq!(
            run(),
            Choice {
                effort: Some("xhigh".into()),
                note: Some(ULTRACODE_UNAVAILABLE.into()),
                ..Choice::default()
            }
        );
        set_claude_ultracode(false);
        assert_eq!(run().effort.as_deref(), Some("xhigh"));
        set_claude_ultracode(true);
        assert_eq!(
            run(),
            Choice {
                effort: Some(ULTRACODE.into()),
                ..Choice::default()
            }
        );
        assert_eq!(
            resolve(AgentKind::Claude, Some("opus"), Some("max"))
                .effort
                .as_deref(),
            Some("max"),
            "ordinary levels are untouched"
        );
        *CLAUDE_ULTRACODE.lock().unwrap() = saved;
    }

    /// Serializes the tests that set the process-wide probe answer.
    pub(crate) static PROBE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
}
