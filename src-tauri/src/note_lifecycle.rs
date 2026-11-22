//! One lifecycle policy for remembered notes (agent effectiveness F7, wave 2
//! · lane S), shared by approved recall (`vault_learning::recall`) and the
//! host retrieval broker (`retrieval_broker`: automatic retrieval,
//! `search_vault`, `vault.retrieve`).
//!
//! Until 2026-10-06 recall left out superseded learnings and required
//! approval, but retrieval read `learnings/` like any note, so a learning the
//! user had replaced came back as plain evidence, often ranked ahead of its
//! correction (the old statement usually shares more words with the
//! question).
//!
//! **The metadata** ([`Lifecycle`], [`of`]) is read from a note's
//! frontmatter by whoever already reads the note (the retrieval scan, the
//! semantic half, recall's listing), so nothing is read twice. Only a
//! learning has one: a `learnings/<name>.md` note (the notes recall lists,
//! [`is_learning_path`]) with `type: learned-context`. For it:
//! - approved when `trust: approved`;
//! - superseded: a `superseded_by:` link (what `supersede_context` writes),
//!   or `status: superseded` with no link; the link may chain, and is
//!   followed only to another learning;
//! - `supersedes:` (a replacement may name what it replaced);
//! - its date: `learned_at`, else `date`, else `updated`;
//! - its statement key, for exact duplicates.
//!
//! Every other note has no lifecycle and is untouched, whatever its
//! frontmatter says: a project filed under `projects/superseded/` with
//! `status: superseded`, an ADR or a Notion page with a Superseded status or
//! its own `superseded_by` link is still ordinary evidence (review fix,
//! 2026-10-06). A note read by path to resolve a current version is refused
//! when the vault walk would skip it (`vault::walk_skips`), so the stage
//! never cites what retrieval keeps out.
//!
//! **The policy.** A request is current-state unless it asks about the past
//! ([`history_cue`]: "used to", "history" and a few close words, "before" or
//! "changed" pinned to the past, or an explicit past date). Then:
//! - current-state: a superseded learning is left out, and the current version
//!   it names (followed along the chain, at most [`MAX_CHAIN`] links) takes
//!   its place in the ranking unless it already ranks ahead;
//! - historical: the superseded learning stays, labelled `historical: superseded
//!   by <path>`, and its current version ranks directly ahead of it;
//! - exact duplicates (two learnings with the same statement, whitespace and
//!   case aside) collapse to the newest; nothing semantic is compared here;
//! - an unapproved learning is labelled, never recalled.
//!
//! Learnings are never edited or deleted here: the audit history stays on
//! disk, and only what a run is given changes. Every decision is recorded in
//! the run's manifest ([`crate::run_manifest::LifecycleRecord`]): path,
//! status, action, reason, never text.
//!
//! The retrieval stage ([`apply`]) is linear in the candidate set (maps, one
//! pass) and reads at most [`MAX_READS`] notes to resolve current versions
//! that aren't candidates themselves.

use crate::directory_connections::Passage;
use crate::run_manifest::LifecycleDecision;
use chrono::{Datelike, NaiveDate};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

/// Supersession links followed to find the current version.
pub const MAX_CHAIN: usize = 8;
/// Notes the retrieval stage may read to resolve current versions.
pub const MAX_READS: usize = 64;
/// What the evidence envelope says when a passage carries a label.
pub const EVIDENCE_NOTE: &str = "A passage labelled historical was replaced by the current version it names: use it only for what was true before. Prefer the current version.";

/// A note's lifecycle metadata (see the module doc).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lifecycle {
    /// `type: learned-context` under `learnings/` (always true for a
    /// lifecycle [`of`] returns; kept explicit for hand-built ones).
    pub learning: bool,
    /// `trust: approved`.
    pub approved: bool,
    /// Replaced: a `superseded_by` link or `status: superseded`.
    pub superseded: bool,
    /// Vault-relative.
    pub superseded_by: Option<String>,
    /// Vault-relative.
    pub supersedes: Option<String>,
    /// `learned_at`, else `date`, else `updated`, as written.
    pub dated: Option<String>,
    /// SHA-256 of a learning's normalized statement.
    pub statement_key: Option<String>,
}

impl Lifecycle {
    /// `superseded`, `unapproved` or `current`.
    pub fn status(&self) -> &'static str {
        if self.superseded {
            "superseded"
        } else if self.learning && !self.approved {
            "unapproved"
        } else {
            "current"
        }
    }

    /// The date part of [`Lifecycle::dated`].
    pub fn day(&self) -> Option<&str> {
        self.dated
            .as_deref()
            .map(|d| d.get(..10).unwrap_or(d))
            .filter(|d| !d.is_empty())
    }
}

/// A learning's statement: its body before the first `## ` section.
pub fn statement(body: &str) -> &str {
    body.split("\n## ").next().unwrap_or("").trim()
}

/// Whether the vault-relative `rel` is where a learning lives: a markdown
/// note directly in `learnings/`, not hidden (what recall lists, so both
/// routes judge the same notes).
pub fn is_learning_path(rel: &str) -> bool {
    rel.strip_prefix("learnings/").is_some_and(|name| {
        name.len() > ".md".len()
            && name.ends_with(".md")
            && !name.contains('/')
            && !name.contains('\\')
            && !name.starts_with('.')
    })
}

/// A learning link from a frontmatter value, or `None` when it is empty,
/// could leave the vault, or names anything but another learning
/// ([`is_learning_path`]; `supersede_context` only links learnings).
fn link(value: Option<&String>) -> Option<String> {
    let v = value?
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim_start_matches("vault:")
        .trim_start_matches("./")
        .trim();
    let unsafe_path = v.is_empty()
        || v.starts_with('/')
        || v.contains('\\')
        || v.split('/').any(|part| part == ".." || part.is_empty());
    (!unsafe_path && is_learning_path(v)).then(|| v.to_string())
}

fn nonempty<'a>(fm: &'a BTreeMap<String, String>, key: &str) -> Option<&'a String> {
    fm.get(key).filter(|v| !v.trim().is_empty())
}

/// The lifecycle of the note at vault-relative `rel`, or `None` for any
/// note that isn't a learning (see the module doc): only learnings are
/// judged.
pub fn of(rel: &str, fm: &BTreeMap<String, String>, body: &str) -> Option<Lifecycle> {
    if !is_learning_path(rel) || fm.get("type").map(|t| t.trim()) != Some("learned-context") {
        return None;
    }
    let superseded_by = link(nonempty(fm, "superseded_by"));
    let status_superseded = fm
        .get("status")
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("superseded"));
    let supersedes = link(nonempty(fm, "supersedes"));
    let normalized = statement(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    let statement_key =
        (!normalized.is_empty()).then(|| format!("{:x}", Sha256::digest(normalized.as_bytes())));
    Some(Lifecycle {
        learning: true,
        approved: fm.get("trust").map(|t| t.trim()) == Some("approved"),
        // An unusable link still marks the note replaced.
        superseded: nonempty(fm, "superseded_by").is_some() || status_superseded,
        superseded_by,
        supersedes,
        dated: ["learned_at", "date", "updated"]
            .iter()
            .find_map(|k| nonempty(fm, k))
            .map(|v| v.trim().to_string()),
        statement_key,
    })
}

// ── History intent ─────────────────────────────────────────────────────

/// Words that make a request historical. Hollow's own vocabulary: the
/// manifest records which one matched, never the request. "before" and
/// "changed" count only pinned to the past ([`pinned_cue`]).
const HISTORY_WORDS: &[&str] = &[
    "history",
    "historical",
    "historically",
    "previously",
    "formerly",
    "originally",
];
/// Phrases right after "before" (or "changed since") that pin it to a past
/// moment.
const ANCHOR_PHRASES: &[[&str; 2]] = &[
    ["the", "change"],
    ["the", "switch"],
    ["we", "switched"],
    ["last", "week"],
    ["last", "month"],
    ["last", "year"],
];
/// Words before a bare year that make it a date ("in 2025").
const YEAR_LEADS: &[&str] = &["in", "since", "during", "until", "till", "by", "of"];
const MONTHS: &[&str] = &[
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

/// What makes `request` ask about the past, if anything: a word from
/// [`HISTORY_WORDS`], `used to`, a pinned `before` or `changed`
/// ([`pinned_cue`]), or `a past date`. Today is this Mac's date.
pub fn history_cue(request: &str) -> Option<&'static str> {
    history_cue_on(request, chrono::Local::now().date_naive())
}

/// [`history_cue`] against a given today (tests).
pub fn history_cue_on(request: &str, today: NaiveDate) -> Option<&'static str> {
    let lower = request.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '-')
        .flat_map(|w| {
            // Keep an ISO date whole; split any other hyphenated word.
            if is_iso_date(w) {
                vec![w]
            } else {
                w.split('-').collect()
            }
        })
        .filter(|w| !w.is_empty())
        .collect();
    for (i, w) in words.iter().enumerate() {
        if let Some(cue) = HISTORY_WORDS.iter().copied().find(|h| h == w) {
            return Some(cue);
        }
        if *w == "used" && words.get(i + 1) == Some(&"to") {
            return Some("used to");
        }
    }
    if let Some(cue) = pinned_cue(&lower, today) {
        return Some(cue);
    }
    past_date(&words, today).then_some("a past date")
}

/// "before" and "changed" are everyday words ("run the tests before
/// merging", "list what changed in this PR"), so they make a request
/// historical only pinned to the past (HF04: 82 of the 88 historical runs in
/// 500 were these two words, unpinned). "before" counts ending a sentence
/// ("what was the config before?"), in "before it changed", or with an
/// anchor after it ([`anchored`]); "changed" counts as "changed since" an
/// anchor. Words keep inner dots and hyphens here, so "v1.2", "10-07" and
/// "before-and-after" stay whole.
fn pinned_cue(lower: &str, today: NaiveDate) -> Option<&'static str> {
    // (word, whether it ends its sentence).
    let tokens: Vec<(&str, bool)> = lower
        .split_whitespace()
        .map(|raw| {
            let word = raw.trim_matches(|c: char| !c.is_alphanumeric());
            // Only a sentence's end: "as I said before, the deploy…" and
            // "Before: … After: …" are not history.
            (word, raw.ends_with(|c: char| ".?!".contains(c)))
        })
        .collect();
    for (i, &(word, end)) in tokens.iter().enumerate() {
        let next = |k: usize| tokens.get(i + k).map_or("", |t| t.0);
        if word == "before" {
            let pinned = end
                || i + 1 == tokens.len()
                || (next(1) == "it" && next(2) == "changed")
                || anchored(&tokens[i + 1..], today);
            if pinned {
                return Some("before");
            }
        }
        if word == "changed"
            && !end
            && next(1) == "since"
            && !tokens[i + 1].1
            && anchored(&tokens[i + 2..], today)
        {
            return Some("changed");
        }
    }
    None
}

/// Whether the words after a cue pin it to a past moment, within its clause:
/// a date on or before `today` (2026-10-07, 10-07, October 7, a year), a
/// version (v2, v1.2, 0.5.3) or "yesterday('s)" in the next three words ("the
/// 10-07 switch"), or an [`ANCHOR_PHRASES`] phrase right after. A future
/// date is a deadline ("finish it before October 30"), not history.
fn anchored(after: &[(&str, bool)], today: NaiveDate) -> bool {
    let mut words = Vec::new();
    for &(word, end) in after.iter().take(3) {
        words.push(word);
        if end {
            break;
        }
    }
    words.iter().enumerate().any(|(n, w)| {
        w.starts_with("yesterday")
            || is_version(w)
            || past_date_word(w, words.get(n + 1).copied(), today)
    }) || ANCHOR_PHRASES.iter().any(|p| words.starts_with(p))
}

/// A date as one word, on or before `today`: an ISO date, a month and day
/// this year (10-07, 10/7), a month's name (with the day after it, if any) or
/// a year. "may" and "march" count only with a day after them ("before it may
/// break" is no date).
fn past_date_word(w: &str, next: Option<&str>, today: NaiveDate) -> bool {
    if is_iso_date(w) {
        return NaiveDate::parse_from_str(w, "%Y-%m-%d").is_ok_and(|d| d <= today);
    }
    if let Some(y) = year_of(w) {
        return y <= today.year();
    }
    let number = |s: &str, max: u32| {
        ((1..=2).contains(&s.len()) && s.chars().all(|c| c.is_ascii_digit()))
            .then(|| s.parse::<u32>().ok())
            .flatten()
            .filter(|n| (1..=max).contains(n))
    };
    let on_or_before = |month: u32, day: Option<u32>| {
        month < today.month() || (month == today.month() && day.is_none_or(|d| d <= today.day()))
    };
    if let Some(month) = month_of(w) {
        let day = next.and_then(|d| number(d.trim_end_matches(|c: char| !c.is_ascii_digit()), 31));
        if matches!(w, "may" | "march") && day.is_none() {
            return false;
        }
        return on_or_before(month, day);
    }
    let Some((month, day)) = w.split_once(['-', '/']) else {
        return false;
    };
    match (number(month, 12), number(day, 31)) {
        (Some(m), Some(d)) => on_or_before(m, Some(d)),
        _ => false,
    }
}

/// A version as one word: v2, v1.2 or 0.5.3.
fn is_version(w: &str) -> bool {
    let (prefixed, rest) = match w.strip_prefix('v') {
        Some(rest) => (true, rest),
        None => (false, w),
    };
    rest.starts_with(|c: char| c.is_ascii_digit())
        && rest.chars().all(|c| c.is_ascii_digit() || c == '.')
        && (prefixed || rest.contains('.'))
}

fn is_iso_date(w: &str) -> bool {
    w.len() == 10 && NaiveDate::parse_from_str(w, "%Y-%m-%d").is_ok()
}

fn month_of(word: &str) -> Option<u32> {
    if word.len() < 3 {
        return None;
    }
    let full = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    MONTHS
        .iter()
        .position(|m| word.starts_with(m) && full.iter().any(|f| f.starts_with(word)))
        .map(|i| i as u32 + 1)
}

fn year_of(word: &str) -> Option<i32> {
    (word.len() == 4 && word.chars().all(|c| c.is_ascii_digit()))
        .then(|| word.parse().ok())
        .flatten()
        .filter(|y| (1970..=2200).contains(y))
}

/// An explicit date before `today`: an ISO date, a month with a year
/// ("September 2026", "15 Sept 2026", "Sept 15, 2026"; a month without a day
/// counts once it has ended), or a past year after "in", "since" and the
/// like.
fn past_date(words: &[&str], today: NaiveDate) -> bool {
    for (i, w) in words.iter().enumerate() {
        if is_iso_date(w) {
            if NaiveDate::parse_from_str(w, "%Y-%m-%d").is_ok_and(|d| d < today) {
                return true;
            }
            continue;
        }
        if let Some(month) = month_of(w) {
            // "15 September 2026" / "September 15, 2026" / "September 2026".
            let day_before = i
                .checked_sub(1)
                .and_then(|j| words[j].parse::<u32>().ok())
                .filter(|d| (1..=31).contains(d));
            let mut day = day_before;
            let mut year = None;
            for next in words.iter().skip(i + 1).take(2) {
                if let Some(y) = year_of(next) {
                    year = Some(y);
                    break;
                }
                if day.is_none() {
                    day = next.parse::<u32>().ok().filter(|d| (1..=31).contains(d));
                }
            }
            if let Some(year) = year {
                let date = match day {
                    Some(d) => NaiveDate::from_ymd_opt(year, month, d),
                    // The month's last day: a month counts once it has ended.
                    None => NaiveDate::from_ymd_opt(year, month, 1).and_then(|first| {
                        first
                            .checked_add_months(chrono::Months::new(1))
                            .and_then(|next| next.pred_opt())
                    }),
                };
                if date.is_some_and(|d| d < today) {
                    return true;
                }
            }
            continue;
        }
        if let Some(year) = year_of(w) {
            let led = i
                .checked_sub(1)
                .is_some_and(|j| YEAR_LEADS.contains(&words[j]));
            if led && year < today.year() {
                return true;
            }
        }
    }
    false
}

// ── Current versions ───────────────────────────────────────────────────

/// What a lookup knows about a note: `None` when it can't be read (missing,
/// outside the vault, unreadable); `Some(None)` for a note without a
/// lifecycle.
pub type Known = Option<Option<Lifecycle>>;

/// The current version of the superseded note `start`: its link followed
/// while the target is itself superseded, at most [`MAX_CHAIN`] links.
/// `None` when a link is missing, unreadable or loops, or `start` has no
/// link (`status: superseded` alone).
pub fn current_version(start: &Lifecycle, lookup: &mut dyn FnMut(&str) -> Known) -> Option<String> {
    let mut next = start.superseded_by.clone()?;
    let mut seen = HashSet::new();
    for _ in 0..MAX_CHAIN {
        if !seen.insert(next.clone()) {
            return None;
        }
        match lookup(&next)? {
            Some(lc) if lc.superseded => next = lc.superseded_by?,
            _ => return Some(next),
        }
    }
    None
}

/// The label a historical entry carries.
pub fn historical_label(current: Option<&str>, current_day: Option<&str>) -> String {
    match (current, current_day) {
        (Some(path), Some(day)) => {
            format!("historical: superseded by {path} (current since {day})")
        }
        (Some(path), None) => format!("historical: superseded by {path}"),
        _ => "historical: superseded (no current version was found)".into(),
    }
}

/// The label a current version carries next to what it replaced.
pub fn current_label(replaced: &str) -> String {
    format!("current: supersedes {replaced}")
}

/// The label an unapproved learning carries.
pub const UNAPPROVED_LABEL: &str =
    "unapproved learning: the user never accepted it, so treat it as an unverified note";

pub(crate) fn decision(
    route: &str,
    path: &str,
    status: &str,
    action: &str,
    current: Option<&str>,
    reason: String,
) -> LifecycleDecision {
    LifecycleDecision {
        route: route.into(),
        path: path.into(),
        status: status.into(),
        action: action.into(),
        current: current.map(str::to_string),
        reason,
    }
}

/// Why a superseded note was left out or kept.
pub(crate) fn superseded_reason(current: Option<&str>, history: Option<&str>) -> String {
    let by = match current {
        Some(path) => format!("Superseded by {path}"),
        None => "Superseded, and no current version was found".into(),
    };
    match history {
        Some(cue) => {
            format!("{by}; kept as history because the request asks about the past ({cue}).")
        }
        None => format!("{by}; the request doesn't ask for history."),
    }
}

// ── The retrieval stage ────────────────────────────────────────────────

/// One note read for the stage: its lifecycle and, when retrieval may cite
/// it, a passage at its statement's first line.
struct ReadNote {
    lifecycle: Option<Lifecycle>,
    passage: Option<Passage>,
}

/// Reads notes the candidate set doesn't hold, at most [`MAX_READS`].
struct Reader<'a> {
    vault: &'a Path,
    kinds: Option<&'a [String]>,
    notes: HashMap<String, Option<ReadNote>>,
    reads: usize,
}

impl Reader<'_> {
    fn read(&mut self, rel: &str) -> Option<&ReadNote> {
        if !self.notes.contains_key(rel) {
            let note = if self.reads < MAX_READS {
                self.reads += 1;
                read_note(self.vault, rel, self.kinds)
            } else {
                None
            };
            self.notes.insert(rel.to_string(), note);
        }
        self.notes.get(rel).and_then(Option::as_ref)
    }
}

/// `rel`'s lifecycle and, if retrieval may cite it (an eligible markdown
/// note of a wanted kind, at most 1 MB), its statement's first line as a
/// passage. `None` for anything the vault walk would skip (a hidden
/// component, the retired message stores): retrieval never sees those, so
/// neither does this.
fn read_note(vault: &Path, rel: &str, kinds: Option<&[String]>) -> Option<ReadNote> {
    if crate::vault::walk_skips(rel) {
        return None;
    }
    let path = crate::vault::safe_join(vault, rel).ok()?;
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > crate::vault_search::RETRIEVAL_MAX_NOTE_BYTES {
        return None;
    }
    let content = std::fs::read_to_string(&path).ok()?;
    let (fm, body) = crate::frontmatter::parse_map(&content);
    let lifecycle = of(rel, &fm, &body);
    let item = crate::vault_index::classify(rel, &fm);
    let citable = rel.ends_with(".md")
        && crate::retrieval_broker::vault_eligible(rel)
        && kinds.is_none_or(|k| k.contains(&item.kind));
    let passage = citable
        .then(|| {
            let fm_end = content.len() - body.len();
            let lead = body.len() - body.trim_start().len();
            let first = statement(&body).lines().find(|l| !l.trim().is_empty())?;
            let line = content[..(fm_end + lead).min(content.len())]
                .matches('\n')
                .count()
                + 1;
            Some(Passage {
                // The broker's form (`vault.join(rel)`), not the
                // canonical one `safe_join` resolved, so paths compare.
                path: vault.join(rel).to_string_lossy().into(),
                line,
                excerpt: first.trim().chars().take(900).collect(),
                source: item.kind.clone(),
                coverage: "excerpt".into(),
                score: 0,
                semantic: None,
                lifecycle: None,
                note: lifecycle.clone(),
                current: None,
            })
        })
        .flatten();
    Some(ReadNote { lifecycle, passage })
}

/// The label a listed note carries when it was replaced (the
/// `search_vault` keyword list, which lists rather than ranks): its current
/// version resolved along the chain. `None` for a note that wasn't.
pub fn listing_label(vault: &Path, rel: &str) -> Option<String> {
    let mut reads = 0;
    let mut lookup = |r: &str| -> Known {
        reads += 1;
        if reads > MAX_CHAIN + 1 {
            return None;
        }
        read_note(vault, r, None).map(|n| n.lifecycle)
    };
    let lc = lookup(rel)??;
    if !lc.superseded {
        return (lc.status() == "unapproved").then(|| UNAPPROVED_LABEL.to_string());
    }
    let current = current_version(&lc, &mut lookup);
    Some(historical_label(current.as_deref(), None))
}

/// What [`apply`] did.
pub struct Applied {
    pub passages: Vec<Passage>,
    pub decisions: Vec<LifecycleDecision>,
}

/// The lifecycle policy over the broker's whole ranked candidate set (see
/// the module doc): `history` is the request's [`history_cue`]. Vault
/// passages carry their note's lifecycle (`Passage::note`); connected
/// project files have none and pass through. Linear in the candidate set.
pub fn apply(
    vault: &Path,
    ranked: Vec<Passage>,
    history: Option<&str>,
    kinds: Option<&[String]>,
) -> Applied {
    let rel_of = |abs: &str| -> Option<String> {
        Path::new(abs)
            .strip_prefix(vault)
            .ok()
            .map(|r| r.to_string_lossy().into_owned())
    };
    let mut reader = Reader {
        vault,
        kinds,
        notes: HashMap::new(),
        reads: 0,
    };
    // What the candidates already say (no read needed), and each path's
    // first position.
    let mut known: HashMap<String, Lifecycle> = HashMap::new();
    let mut first_at: HashMap<String, usize> = HashMap::new();
    for (i, p) in ranked.iter().enumerate() {
        first_at.entry(p.path.clone()).or_insert(i);
        if let (Some(lc), Some(rel)) = (&p.note, rel_of(&p.path)) {
            known.entry(rel).or_insert_with(|| lc.clone());
        }
    }
    if known.is_empty() {
        return Applied {
            passages: ranked,
            decisions: Vec::new(),
        };
    }
    // Exact duplicates among current learnings: the newest (then the first
    // path) is kept.
    let mut keep_of: HashMap<&str, (&str, &str)> = HashMap::new(); // key -> (dated, rel)
    for (rel, lc) in &known {
        if lc.superseded {
            continue;
        }
        if let Some(key) = lc.statement_key.as_deref() {
            let dated = lc.dated.as_deref().unwrap_or("");
            let entry = keep_of.entry(key).or_insert((dated, rel.as_str()));
            if (dated, std::cmp::Reverse(rel.as_str())) > (entry.0, std::cmp::Reverse(entry.1)) {
                *entry = (dated, rel.as_str());
            }
        }
    }
    let duplicate_of: HashMap<String, String> = known
        .iter()
        .filter(|(_, lc)| !lc.superseded)
        .filter_map(|(rel, lc)| {
            let keep = keep_of.get(lc.statement_key.as_deref()?)?.1;
            (keep != rel).then(|| (rel.clone(), keep.to_string()))
        })
        .collect();

    let mut lookup_known = known.clone();
    let mut current_of: HashMap<String, Option<String>> = HashMap::new();
    let mut slots: Vec<Option<Passage>> = ranked.into_iter().map(Some).collect();
    let mut out: Vec<Passage> = Vec::with_capacity(slots.len());
    let mut out_first: HashMap<String, usize> = HashMap::new();
    let mut decided: HashSet<(String, &'static str)> = HashSet::new();
    let mut decisions = Vec::new();

    for i in 0..slots.len() {
        let Some(mut p) = slots[i].take() else {
            continue; // moved ahead as a current version
        };
        let Some(lc) = p.note.clone() else {
            out_first.entry(p.path.clone()).or_insert(out.len());
            out.push(p);
            continue;
        };
        let rel = rel_of(&p.path).unwrap_or_else(|| p.path.clone());
        if let Some(keep) = duplicate_of.get(&rel) {
            if decided.insert((rel.clone(), "collapsed")) {
                decisions.push(decision(
                    "retrieval",
                    &rel,
                    "duplicate",
                    "collapsed",
                    Some(keep),
                    format!("An exact duplicate of {keep}; the newer one is kept."),
                ));
            }
            continue;
        }
        if lc.superseded {
            let current = current_of
                .entry(rel.clone())
                .or_insert_with(|| {
                    current_version(&lc, &mut |r: &str| -> Known {
                        if let Some(lc) = lookup_known.get(r) {
                            return Some(Some(lc.clone()));
                        }
                        let read = reader.read(r)?;
                        if let Some(lc) = &read.lifecycle {
                            lookup_known.insert(r.to_string(), lc.clone());
                        }
                        Some(read.lifecycle.clone())
                    })
                })
                .clone();
            let action = if history.is_some() {
                "historical"
            } else {
                "excluded"
            };
            if decided.insert((rel.clone(), action)) {
                decisions.push(decision(
                    "retrieval",
                    &rel,
                    "superseded",
                    action,
                    current.as_deref(),
                    superseded_reason(current.as_deref(), history),
                ));
            }
            // The current version ranks here, ahead of what it replaced.
            if let Some(cur) = current.as_deref() {
                let cur_abs = vault.join(cur).to_string_lossy().into_owned();
                if let Some(&at) = out_first.get(&cur_abs) {
                    if out[at].lifecycle.is_none() {
                        out[at].lifecycle = Some(current_label(&rel));
                    }
                    if decided.insert((cur.to_string(), "current")) {
                        decisions.push(decision(
                            "retrieval",
                            cur,
                            "current",
                            "current",
                            None,
                            format!("The current version of {rel}; it already ranked ahead."),
                        ));
                    }
                } else {
                    let moved = first_at
                        .get(&cur_abs)
                        .filter(|&&j| j > i)
                        .and_then(|&j| slots[j].take());
                    let was_candidate = moved.is_some();
                    let placed = moved.or_else(|| {
                        reader.read(cur).and_then(|n| {
                            n.passage.as_ref().map(|p| Passage {
                                path: p.path.clone(),
                                line: p.line,
                                excerpt: p.excerpt.clone(),
                                source: p.source.clone(),
                                coverage: p.coverage.clone(),
                                score: 0,
                                semantic: None,
                                lifecycle: None,
                                note: p.note.clone(),
                                current: None,
                            })
                        })
                    });
                    if let Some(mut head) = placed {
                        head.lifecycle = Some(current_label(&rel));
                        out_first.insert(head.path.clone(), out.len());
                        out.push(head);
                        if decided.insert((cur.to_string(), "promoted")) {
                            decisions.push(decision(
                                "retrieval",
                                cur,
                                "current",
                                "promoted",
                                None,
                                format!(
                                    "The current version of {rel}; ranked in its place{}.",
                                    if was_candidate {
                                        ""
                                    } else {
                                        " (it shares too few words with the request to rank by itself)"
                                    }
                                ),
                            ));
                        }
                    }
                }
            }
            if history.is_some() {
                p.lifecycle = Some(historical_label(
                    current.as_deref(),
                    current
                        .as_deref()
                        .and_then(|c| lookup_known.get(c))
                        .and_then(Lifecycle::day),
                ));
                p.current = current
                    .as_deref()
                    .map(|c| vault.join(c).to_string_lossy().into_owned());
                out_first.entry(p.path.clone()).or_insert(out.len());
                out.push(p);
            }
            continue;
        }
        if lc.status() == "unapproved" {
            p.lifecycle = Some(UNAPPROVED_LABEL.into());
            if decided.insert((rel.clone(), "labelled")) {
                decisions.push(decision(
                    "retrieval",
                    &rel,
                    "unapproved",
                    "labelled",
                    None,
                    "A learned-context note nobody approved: labelled, never recalled.".into(),
                ));
            }
        }
        out_first.entry(p.path.clone()).or_insert(out.len());
        out.push(p);
    }
    Applied {
        passages: out,
        decisions,
    }
}

/// Keep every historical passage behind its current version after a
/// reranker reordered the list: a current version that ranks later moves
/// directly ahead of the first historical passage that names it. Linear.
pub fn current_first(passages: Vec<Passage>) -> Vec<Passage> {
    if passages.iter().all(|p| p.current.is_none()) {
        return passages;
    }
    let mut first_at: HashMap<String, usize> = HashMap::new();
    for (i, p) in passages.iter().enumerate() {
        first_at.entry(p.path.clone()).or_insert(i);
    }
    let mut slots: Vec<Option<Passage>> = passages.into_iter().map(Some).collect();
    let mut out = Vec::with_capacity(slots.len());
    for i in 0..slots.len() {
        let Some(p) = slots[i].take() else { continue };
        if let Some(j) = p
            .current
            .as_ref()
            .and_then(|c| first_at.get(c))
            .copied()
            .filter(|&j| j > i)
        {
            if let Some(head) = slots[j].take() {
                out.push(head);
            }
        }
        out.push(p);
    }
    out
}

/// The F7 fixture shared by the broker, `search_vault`, recall and the
/// evaluation tests.
#[cfg(test)]
pub(crate) mod fixture {
    use std::fs;
    use std::path::Path;

    pub fn put(root: &Path, rel: &str, text: &str) {
        let p = root.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, text).unwrap();
    }

    pub const OLD_CADENCE: &str = "learnings/kestrel-cadence-old00001.md";
    pub const NEW_CADENCE: &str = "learnings/kestrel-cadence-new00002.md";
    /// A current-state question the OLD statement matches better.
    pub const NOW_Q: &str = "How often do Kestrel status reports go out?";
    /// A historical question ("before", "changed").
    pub const HISTORY_Q: &str = "What was the Kestrel status report cadence before it changed?";

    pub fn learning(statement: &str, learned_at: &str, id: &str) -> String {
        format!(
            "---\ntype: learned-context\ntitle: Kestrel cadence\ntarget_kind: project\ntarget_path: projects/Kestrel pilot.md\nsubject: Kestrel reporting\ntrust: approved\nproposal_id: review-{id}\nlearned_at: {learned_at}\n---\n\n{statement}\n"
        )
    }

    /// An approved learning and the approved correction that replaced it,
    /// linked the way the app links them (`supersede_context` marks only the
    /// old note). The old statement shares more words with both questions.
    pub fn superseded_learning_vault(v: &Path) {
        put(
            v,
            "projects/Kestrel pilot.md",
            "---\ntitle: Kestrel pilot\n---\n\nA synthetic field pilot.\n",
        );
        put(
            v,
            OLD_CADENCE,
            &learning(
                "Kestrel status reports go out every Friday, drafted on Thursday.",
                "2026-07-01T09:00:00Z",
                "old00001",
            ),
        );
        put(
            v,
            NEW_CADENCE,
            &learning(
                "Correction: the steering group moved Kestrel reporting to every second Tuesday on 2026-09-15.",
                "2026-09-15T09:00:00Z",
                "new00002",
            ),
        );
        crate::vault_learning::supersede_context(v, OLD_CADENCE, NEW_CADENCE).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn metadata_is_read_from_frontmatter_and_plain_notes_have_none() {
        const L: &str = "learnings/a.md";
        assert_eq!(of(L, &fm(&[("title", "Plain")]), "Body"), None);
        let lc = of(
            L,
            &fm(&[
                ("type", "learned-context"),
                ("trust", "approved"),
                ("superseded_by", "vault:learnings/new.md"),
                ("learned_at", "2026-07-01T09:00:00Z"),
            ]),
            "Reports go  out on FRIDAY.\n\n## Why Hollow proposed this\n\nx",
        )
        .unwrap();
        assert!(lc.learning && lc.approved && lc.superseded);
        assert_eq!(lc.superseded_by.as_deref(), Some("learnings/new.md"));
        assert_eq!(lc.status(), "superseded");
        assert_eq!(lc.day(), Some("2026-07-01"));
        let same = of(
            "learnings/b.md",
            &fm(&[("type", "learned-context")]),
            "reports go out on friday.",
        )
        .unwrap();
        assert_eq!(
            same.statement_key, lc.statement_key,
            "whitespace and case aside"
        );
        assert_eq!(same.status(), "unapproved");
        // A link that could leave the vault, or names anything but another
        // learning, is no link; status alone still marks the learning replaced.
        for target in [
            "../outside.md",
            "emails/inbox/x.md",
            ".trash/old.md",
            "learnings/.trash/z.md",
            "learnings/sub/x.md",
            "notes/new.md",
        ] {
            let odd = of(
                L,
                &fm(&[
                    ("type", "learned-context"),
                    ("superseded_by", target),
                    ("status", "Superseded"),
                ]),
                "x",
            )
            .unwrap();
            assert!(odd.superseded && odd.superseded_by.is_none(), "{target}");
        }
    }

    /// Review fix: only learnings are judged. A superseded project, an ADR
    /// with its own link, a Notion page's status and a learned-context note
    /// filed outside `learnings/` (recall never lists one) have none.
    #[test]
    fn only_learnings_have_a_lifecycle() {
        let superseded = fm(&[("status", "superseded")]);
        assert_eq!(
            of(
                "projects/superseded/HR Analytics Dashboard.md",
                &superseded,
                "x"
            ),
            None
        );
        assert_eq!(
            of(
                "decisions/adr-7.md",
                &fm(&[
                    ("status", "Superseded"),
                    ("superseded_by", "decisions/adr-9.md")
                ]),
                "x"
            ),
            None
        );
        assert_eq!(
            of(
                "notes/replacement.md",
                &fm(&[("supersedes", "learnings/a.md")]),
                "x"
            ),
            None
        );
        let learning = fm(&[("type", "learned-context"), ("trust", "approved")]);
        for rel in [
            "notes/a.md",
            "learnings/sub/a.md",
            "learnings/.a.md",
            "learnings/a.txt",
            "projects/learnings/a.md",
        ] {
            assert_eq!(of(rel, &learning, "x"), None, "{rel}");
        }
        assert!(of("learnings/a.md", &learning, "x").is_some());
        assert!(is_learning_path("learnings/kestrel-cadence-old00001.md"));
        assert!(!is_learning_path("learnings/.md"));
    }

    #[test]
    fn history_cues_are_words_or_explicit_past_dates() {
        let today = day("2026-10-06");
        let cue = |q: &str| history_cue_on(q, today);
        assert_eq!(
            cue("What was the cadence before it changed?"),
            Some("before")
        );
        // HF04: "changed" alone is no longer a cue (it was).
        assert_eq!(cue("How has the cadence changed"), None);
        assert_eq!(cue("We used to send them weekly"), Some("used to"));
        assert_eq!(cue("the report history"), Some("history"));
        assert_eq!(cue("what did we decide on 2026-09-01"), Some("a past date"));
        assert_eq!(cue("plans for 2026-12-01"), None);
        assert_eq!(cue("the cadence in September 2026"), Some("a past date"));
        assert_eq!(
            cue("the cadence in October 2026"),
            None,
            "a month counts once ended"
        );
        assert_eq!(cue("what happened on 3 Oct 2026"), Some("a past date"));
        assert_eq!(cue("what happened on Sept 15, 2026"), Some("a past date"));
        assert_eq!(cue("what we shipped in 2025"), Some("a past date"));
        assert_eq!(
            cue("order 2025 sensors"),
            None,
            "a bare number is not a date"
        );
        assert_eq!(cue("How often do Kestrel status reports go out now?"), None);
        assert_eq!(cue("may I see the march plan"), None);
        // HF04: "before" inside a hyphenated word is no longer a cue (it was).
        assert_eq!(cue("the before-and-after photos"), None);
    }

    /// HF04 (red first): "before" and "changed" made 82 of 88 historical
    /// runs in 500 ("before merging", "what changed"); they count only pinned
    /// to the past.
    #[test]
    fn before_and_changed_need_an_anchor() {
        let today = day("2026-10-16");
        let cue = |q: &str| history_cue_on(q, today);
        assert_eq!(cue("run the tests before merging"), None);
        assert_eq!(cue("list what changed in this PR"), None);
        assert_eq!(cue("Check the diff before you push."), None);
        assert_eq!(
            cue("what did we decide before the 10-07 switch"),
            Some("before")
        );
        assert_eq!(
            cue("how was this originally configured"),
            Some("originally")
        );
        assert_eq!(cue("what was the config before"), Some("before"));
        assert_eq!(cue("How was the deploy set up before?"), Some("before"));
        assert_eq!(cue("what changed since 2026-10-01"), Some("changed"));
        assert_eq!(cue("what changed since the switch"), Some("changed"));
        assert_eq!(cue("how did it work before v1.2 shipped"), Some("before"));
        assert_eq!(cue("the setup before 0.5.3"), Some("before"));
        assert_eq!(cue("the plan before last week"), Some("before"));
        assert_eq!(cue("what we used before yesterday's fix"), Some("before"));
        assert_eq!(cue("the queue before October 7"), Some("before"));
        // Review: a future date is a deadline, a comma or colon doesn't end
        // the clause, and "may"/"march" need a day.
        assert_eq!(cue("finish the migration before October 30"), None);
        assert_eq!(cue("ship it before 2027"), None);
        assert_eq!(cue("land it before 2026-12-01"), None);
        assert_eq!(cue("as I said before, the deploy is broken"), None);
        assert_eq!(cue("Before: the old screen. After: the new one"), None);
        assert_eq!(cue("check it before it may break"), None);
        assert_eq!(cue("what did we have before 2026-10-01"), Some("before"));
        assert_eq!(cue("the plan before May 3"), Some("before"));
    }

    #[test]
    fn the_current_version_follows_the_chain_and_stops_on_a_loop() {
        let node = |by: Option<&str>| Lifecycle {
            superseded: by.is_some(),
            superseded_by: by.map(str::to_string),
            ..Default::default()
        };
        let notes: HashMap<&str, Lifecycle> = HashMap::from([
            ("b", node(Some("c"))),
            ("c", node(None)),
            ("x", node(Some("y"))),
            ("y", node(Some("x"))),
        ]);
        let mut lookup = |r: &str| -> Known { notes.get(r).map(|l| Some(l.clone())) };
        assert_eq!(
            current_version(&node(Some("b")), &mut lookup).as_deref(),
            Some("c")
        );
        assert_eq!(
            current_version(&node(Some("x")), &mut lookup),
            None,
            "a loop"
        );
        assert_eq!(current_version(&node(Some("missing")), &mut lookup), None);
        assert_eq!(current_version(&node(None), &mut lookup), None);
    }

    fn passage(path: &str, current: Option<&str>) -> Passage {
        Passage {
            path: path.into(),
            line: 1,
            excerpt: "x".into(),
            source: "note".into(),
            coverage: "excerpt".into(),
            score: 1,
            semantic: None,
            lifecycle: None,
            note: None,
            current: current.map(str::to_string),
        }
    }

    #[test]
    fn current_first_moves_a_reranked_current_version_ahead() {
        let reranked = vec![
            passage("a", None),
            passage("old", Some("new")),
            passage("b", None),
            passage("new", None),
            passage("new", None),
        ];
        let order: Vec<String> = current_first(reranked)
            .into_iter()
            .map(|p| p.path)
            .collect();
        assert_eq!(order, ["a", "new", "old", "b", "new"]);
    }

    #[test]
    fn the_stage_is_linear_in_the_candidate_set() {
        // 40,000 passages, half of them superseded learnings whose current
        // versions are candidates further down: one pass with maps.
        let vault = Path::new("/v");
        let mut ranked = Vec::new();
        for i in 0..10_000 {
            let mut old = passage(&format!("/v/learnings/old{i}.md"), None);
            old.note = Some(Lifecycle {
                learning: true,
                approved: true,
                superseded: true,
                superseded_by: Some(format!("learnings/new{i}.md")),
                ..Default::default()
            });
            ranked.push(old);
            ranked.push(passage(&format!("/v/notes/n{i}.md"), None));
        }
        for i in 0..10_000 {
            let mut new = passage(&format!("/v/learnings/new{i}.md"), None);
            new.note = Some(Lifecycle {
                learning: true,
                approved: true,
                ..Default::default()
            });
            ranked.push(new);
            ranked.push(passage(&format!("/v/notes/m{i}.md"), None));
        }
        let started = std::time::Instant::now();
        let applied = apply(vault, ranked, None, None);
        let took = started.elapsed();
        eprintln!("lifecycle stage on 40,000 passages: {took:?}");
        assert_eq!(applied.passages.len(), 30_000);
        assert!(applied.passages[0].path.ends_with("new0.md"));
        assert!(took < std::time::Duration::from_secs(5), "{took:?}");
    }
}
