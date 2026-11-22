//! Records (master plan 2026-10, F4): decisions, facts about people, and the
//! generated part of each person's note. See `docs/RECORDS.md`.
//!
//! **Decisions** live in `decisions/<date>-<slug>.md` with frontmatter
//! `type: decision`, `decided_at`, `decided_by`, `scope` (`global` or a
//! workspace id), `status` (`settled`, `superseded`, `revisit`, `retracted`),
//! `supersedes` / `superseded_by` and `evidence` (an anchor), and, all
//! optional (DR1), `rejected` (`option -> reason; option -> reason`),
//! `constraint: true` and `derives_from` (comma-separated vault paths).
//! **Person
//! facts** live in `records/facts/<date>-<slug>.md` (`type: person-fact`,
//! `person: people/<Name>.md`). Every record links its anchor (the one
//! linking rule).
//!
//! **Only the person accepts.** [`accept_proposal`] turns a pending `record`
//! review item (from the extraction pipeline, F3) into a file, and only from
//! `resolve_review_item` (user-only). Agents read records and can propose;
//! they never settle or supersede (`supersede_decision` is confirm tier).
//!
//! **People notes** get one fenced block Hollow writes between
//! [`GENERATED_START`] and [`GENERATED_END`]: what they owe the person and
//! what the person owes them (open commitments), their decisions and the
//! accepted facts about them. Everything outside the fence is the person's
//! own text and stays byte-for-byte ([`splice_generated`]).
//!
//! **Retrieval.** Settled decisions join the trusted recall
//! (`vault_learning::recall_with`, through [`decision_contexts`]) only when
//! the request shares words with them: an unrelated decision costs no bytes.
//! A superseded one is history (recalled only for a past-tense request,
//! behind its replacement); one under `revisit` is not settled and is left
//! out. A `retracted` one is withdrawn: kept, never deleted, recalled as
//! labelled history like a superseded one. A `constraint: true` decision is
//! never cut by the recall budget, and every decision's rejected options and
//! its superseded or retracted state ride in its recalled statement.

use crate::frontmatter::{self, KeyEdit};
use crate::review_queue::{self, ReviewItem};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

pub const DECISIONS_DIR: &str = "decisions";
pub const FACTS_DIR: &str = "records/facts";
pub const STATUSES: [&str; 4] = ["settled", "superseded", "revisit", "retracted"];
pub const GENERATED_START: &str =
    "<!-- hollow:generated:start · Hollow writes this section from your records; edits inside it are replaced -->";
pub const GENERATED_END: &str = "<!-- hollow:generated:end -->";
const TEXT_MAX: usize = 500;
/// DR1 bounds on a `rejected` line: entries, and characters in each part.
const REJECTED_MAX: usize = 8;
const REJECTED_OPTION_MAX: usize = 200;
const REJECTED_REASON_MAX: usize = 300;

/// One option a decision turned down, and why (DR1).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RejectedOption {
    pub option: String,
    pub reason: Option<String>,
}

/// One decision record.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    /// Vault-relative.
    pub path: String,
    pub text: String,
    pub decided_at: String,
    /// A name, or `unknown`.
    pub decided_by: String,
    pub scope: String,
    pub status: String,
    pub supersedes: Option<String>,
    pub superseded_by: Option<String>,
    pub evidence: Option<String>,
    pub source: Option<String>,
    /// When it was superseded (`YYYY-MM-DD`), for as-of reads.
    pub superseded_at: Option<String>,
    /// When it was retracted (`YYYY-MM-DD`), for as-of reads (DR1).
    pub retracted_at: Option<String>,
    /// Options turned down, with why (DR1). Empty on an older record.
    pub rejected: Vec<RejectedOption>,
    /// A hard constraint (DR1): recall never cuts it. Only a person sets it,
    /// by editing the record; there is no agent action for it.
    pub constraint: bool,
    /// Vault paths of the findings or records this one answers (DR1).
    pub derives_from: Vec<String>,
    /// The tracker issue key its build-log comment goes on (DR1b), e.g.
    /// `ENG-12`. None: no comment is ever made for it.
    pub issue: Option<String>,
    /// Why it was decided, for the build-log comment (DR1b).
    pub why: Option<String>,
    /// How it was checked, for the build-log comment (DR1b).
    pub verified: Option<String>,
}

/// One accepted fact about a person.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub path: String,
    pub text: String,
    /// `people/<Name>.md`.
    pub person: String,
    pub stated_at: String,
    pub evidence: Option<String>,
    pub source: Option<String>,
}

fn md_files(vault: &Path, dir: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(vault.join(dir)) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.ends_with(".md") && !name.starts_with('.') && !name.starts_with('_'))
                .then(|| {
                    std::fs::read_to_string(e.path())
                        .ok()
                        .map(|raw| (format!("{dir}/{name}"), raw))
                })
                .flatten()
        })
        .collect();
    out.sort();
    out
}

fn body_text(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("Evidence:"))
        .unwrap_or("")
        .to_string()
}

fn opt(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn parse_decision(path: &str, raw: &str) -> Option<Decision> {
    let (fm, body) = frontmatter::parse_map(raw);
    if fm.get("type").map(String::as_str) != Some("decision") {
        return None;
    }
    let text = opt(&fm, "title").unwrap_or_else(|| body_text(&body));
    let status = opt(&fm, "status").unwrap_or_else(|| "settled".into());
    Some(Decision {
        path: path.into(),
        text,
        decided_at: opt(&fm, "decided_at").unwrap_or_default(),
        decided_by: opt(&fm, "decided_by").unwrap_or_else(|| "unknown".into()),
        scope: opt(&fm, "scope").unwrap_or_else(|| "global".into()),
        status: if STATUSES.contains(&status.as_str()) {
            status
        } else {
            "settled".into()
        },
        supersedes: opt(&fm, "supersedes"),
        superseded_by: opt(&fm, "superseded_by"),
        evidence: opt(&fm, "evidence"),
        source: opt(&fm, "source"),
        superseded_at: opt(&fm, "superseded_at"),
        retracted_at: opt(&fm, "retracted_at"),
        rejected: opt(&fm, "rejected")
            .map(|v| parse_rejected(&v))
            .unwrap_or_default(),
        constraint: opt(&fm, "constraint")
            .is_some_and(|v| matches!(v.to_lowercase().as_str(), "true" | "yes")),
        derives_from: opt(&fm, "derives_from")
            .map(|v| {
                v.split(',')
                    .map(|p| p.trim().trim_start_matches("vault:").to_string())
                    .filter(|p| !p.is_empty())
                    .collect()
            })
            .unwrap_or_default(),
        issue: opt(&fm, "issue"),
        why: opt(&fm, "why"),
        verified: opt(&fm, "verified"),
    })
}

/// A `rejected` frontmatter value: `option -> reason; option -> reason`
/// (the reason is optional; a `;` ends an entry, so neither part holds one).
/// At most [`REJECTED_MAX`] entries, each part clipped.
pub fn parse_rejected(raw: &str) -> Vec<RejectedOption> {
    raw.split(';')
        .filter_map(|part| {
            let (option, reason) = match part.split_once("->") {
                Some((o, r)) => (o, Some(r)),
                None => (part, None),
            };
            let option = one_line(option, REJECTED_OPTION_MAX);
            if option.is_empty() {
                return None;
            }
            let reason = reason
                .map(|r| one_line(r, REJECTED_REASON_MAX))
                .filter(|r| !r.is_empty());
            Some(RejectedOption { option, reason })
        })
        .take(REJECTED_MAX)
        .collect()
}

/// The canonical `rejected` frontmatter value ([`parse_rejected`]'s input).
fn rejected_value(list: &[RejectedOption]) -> String {
    list.iter()
        .map(|r| match &r.reason {
            Some(reason) => format!("{} -> {reason}", r.option),
            None => r.option.clone(),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// What recall says about turned-down options (DR1); empty when none.
fn rejected_sentence(list: &[RejectedOption]) -> String {
    if list.is_empty() {
        return String::new();
    }
    let items = list
        .iter()
        .map(|r| match &r.reason {
            Some(reason) => format!("{} ({reason})", r.option),
            None => r.option.clone(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!(" Rejected: {items}. Do not repeat these without confirmation.")
}

fn parse_fact(path: &str, raw: &str) -> Option<Fact> {
    let (fm, body) = frontmatter::parse_map(raw);
    if fm.get("type").map(String::as_str) != Some("person-fact") {
        return None;
    }
    Some(Fact {
        path: path.into(),
        text: opt(&fm, "title").unwrap_or_else(|| body_text(&body)),
        person: opt(&fm, "person")?,
        stated_at: opt(&fm, "stated_at").unwrap_or_default(),
        evidence: opt(&fm, "evidence"),
        source: opt(&fm, "source"),
    })
}

/// Every decision record, newest first.
pub fn decisions(vault: &Path) -> Vec<Decision> {
    let mut out: Vec<Decision> = md_files(vault, DECISIONS_DIR)
        .into_iter()
        .filter_map(|(p, raw)| parse_decision(&p, &raw))
        .collect();
    out.sort_by(|a, b| b.decided_at.cmp(&a.decided_at).then(a.path.cmp(&b.path)));
    out
}

/// Every accepted person fact, newest first.
pub fn facts(vault: &Path) -> Vec<Fact> {
    let mut out: Vec<Fact> = md_files(vault, FACTS_DIR)
        .into_iter()
        .filter_map(|(p, raw)| parse_fact(&p, &raw))
        .collect();
    out.sort_by(|a, b| b.stated_at.cmp(&a.stated_at).then(a.path.cmp(&b.path)));
    out
}

/// What extraction compares new decisions and facts with (F3 dedup).
pub fn existing_texts(vault: &Path) -> Vec<String> {
    decisions(vault)
        .into_iter()
        .map(|d| d.text)
        .chain(facts(vault).into_iter().map(|f| f.text))
        .collect()
}

/// A decision as it stood on `as_of` (`YYYY-MM-DD`): `None` when it wasn't
/// decided yet; a decision superseded after that day was still settled
/// then; likewise a retraction after that day (`retracted_at`). A supersede
/// before `superseded_at` was recorded reads as settled
/// (uncaptured is not known: the person sees the note's current status).
pub fn as_of(d: &Decision, as_of: &str) -> Option<Decision> {
    let day = as_of.get(..10).unwrap_or(as_of);
    if d.decided_at.as_str() > day {
        return None;
    }
    let mut d = d.clone();
    if d.status == "superseded" && d.superseded_at.as_deref().is_some_and(|at| at > day) {
        d.status = "settled".into();
        d.superseded_by = None;
    }
    if d.status == "retracted" && d.retracted_at.as_deref().is_some_and(|at| at > day) {
        d.status = "settled".into();
    }
    Some(d)
}

/// Decisions filtered: `scope` (`global`, a workspace id; a workspace also
/// sees global ones), a `status`, and words that must all appear;
/// `as_of_day` reads them as they stood that day ([`as_of`]).
pub fn find_decisions(
    vault: &Path,
    scope: Option<&str>,
    status: Option<&str>,
    query: Option<&str>,
    as_of_day: Option<&str>,
) -> Vec<Decision> {
    let words: Vec<String> = query
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    let day = as_of_day.map(str::trim).filter(|d| !d.is_empty());
    decisions(vault)
        .into_iter()
        .filter_map(|d| match day {
            Some(day) => as_of(&d, day),
            None => Some(d),
        })
        .filter(|d| {
            scope.is_none_or(|s| s.trim().is_empty() || d.scope == s.trim() || d.scope == "global")
        })
        .filter(|d| status.is_none_or(|s| s.trim().is_empty() || d.status == s.trim()))
        .filter(|d| {
            let hay = format!("{} {} {}", d.text, d.decided_by, d.scope).to_lowercase();
            words.iter().all(|w| hay.contains(w))
        })
        .collect()
}

// ── Writing ────────────────────────────────────────────────────────────────

fn slug(text: &str) -> String {
    let s: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .take(8)
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "record".into()
    } else {
        s.chars().take(60).collect()
    }
}

fn one_line(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// Create `<dir>/<date>-<slug>[-n].md`; its path.
fn create_unique(
    vault: &Path,
    dir: &str,
    date: &str,
    text: &str,
    content: &str,
) -> Result<String, String> {
    let base = format!("{dir}/{date}-{}", slug(text));
    for n in 0..50 {
        let rel = if n == 0 {
            format!("{base}.md")
        } else {
            format!("{base}-{}.md", n + 1)
        };
        if vault.join(&rel).exists() {
            continue;
        }
        match crate::vault::create_note(vault, &rel, content) {
            Ok(_) => return Ok(rel),
            Err(e) if vault.join(&rel).exists() => {
                let _ = e;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(format!("Couldn't find a free name for {base}.md"))
}

/// What a person sets when accepting a record (all optional).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AcceptEdits {
    pub text: Option<String>,
    pub owner: Option<String>,
    /// A decision's scope: `global` or a workspace id.
    pub scope: Option<String>,
    /// A fact's person: `people/<Name>.md`.
    pub person: Option<String>,
}

/// The workspace a source belongs to, from its id
/// (`recording:workspaces/<id>/…`).
fn source_workspace(source_id: &str) -> Option<String> {
    if let Some(scope) = source_id.strip_prefix("proposal:") {
        return Some(scope.to_string()).filter(|s| !s.is_empty() && s != "global");
    }
    let rest = source_id.strip_prefix("recording:workspaces/")?;
    rest.split('/')
        .next()
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// The person note a name means, only when one note's title or file name
/// is exactly that name (case-insensitive). Never a first-name guess.
pub fn person_for(vault: &Path, name: &str) -> Option<String> {
    let name = name.trim().to_lowercase();
    if name.is_empty() {
        return None;
    }
    let hits: Vec<String> = crate::vault_people::list(vault)
        .into_iter()
        .filter(|p| {
            p.name.to_lowercase() == name
                || p.path
                    .trim_start_matches("people/")
                    .trim_end_matches(".md")
                    .to_lowercase()
                    == name
                || crate::people_dossier::matches(vault, &p.path, &name)
        })
        .map(|p| p.path)
        .collect();
    (hits.len() == 1).then(|| hits[0].clone())
}

/// Accept a pending `record` review item (user-only): write the decision or
/// the fact, refresh the person's note, then mark the item accepted. A fact
/// whose person isn't a known note refuses until the person picks one.
pub fn accept_proposal(
    vault: &Path,
    item: &ReviewItem,
    edited: Option<&Value>,
    now: DateTime<Utc>,
) -> Result<String, String> {
    if item.kind != crate::extract::RECORD_KIND {
        return Err(format!("review item {} is not a record proposal", item.id));
    }
    let mut p: crate::extract::Proposal = serde_json::from_value(item.payload.clone())
        .map_err(|e| format!("invalid record proposal {}: {e}", item.id))?;
    let edits: AcceptEdits = edited
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| format!("invalid record edits: {e}"))?
        .unwrap_or_default();
    if let Some(text) = edits
        .text
        .as_deref()
        .map(|t| one_line(t, TEXT_MAX))
        .filter(|t| !t.is_empty())
    {
        p.text = text;
    }
    if let Some(owner) = edits.owner.clone() {
        p.owner = Some(one_line(&owner, 80)).filter(|o| !o.is_empty());
    }
    // EV1: a quote that names a conversation turn or a transcript line must be
    // there, checked now (extraction checked it once; an agent's proposal never
    // was). A miss refuses before anything is claimed or written, so the item
    // stays pending, with the nearest real line shown.
    if let crate::quote_check::Quote::Miss { nearest } =
        crate::quote_check::check(vault, &p.anchor, &p.evidence_excerpt)
    {
        return Err(crate::quote_check::refusal(&p.anchor, nearest.as_deref()));
    }
    // Local days throughout (as-of reads are local). A decision whose source
    // dates it (an imported brief bullet) keeps that date, if not ahead.
    let today = now.with_timezone(&chrono::Local).date_naive();
    let date = today.to_string();
    let decided_on = chrono::NaiveDate::parse_from_str(&p.at, "%Y-%m-%d")
        .ok()
        .filter(|d| *d <= today)
        .map(|d| d.to_string())
        .unwrap_or_else(|| date.clone());
    let content = match p.kind.as_str() {
        "decision" => {
            let scope = edits
                .scope
                .clone()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| source_workspace(&p.source_id))
                .unwrap_or_else(|| "global".into());
            if scope != "global" && crate::workspaces::summary(vault, &scope).is_err() {
                return Err(format!(
                    "No workspace '{scope}': choose global or a workspace id."
                ));
            }
            let mut lines = vec!["---".to_string()];
            frontmatter::push_kv(&mut lines, "type", "decision");
            frontmatter::push_kv(&mut lines, "title", &p.text);
            frontmatter::push_kv(&mut lines, "decided_at", &decided_on);
            frontmatter::push_kv(
                &mut lines,
                "decided_by",
                p.owner.as_deref().unwrap_or("unknown"),
            );
            frontmatter::push_kv(&mut lines, "scope", &scope);
            frontmatter::push_kv(&mut lines, "status", "settled");
            // Read again here, not trusted as queued: the same parse and cap
            // as a proposal, so a hand-edited queue item can't write a bad line.
            let rejected = p
                .rejected
                .as_deref()
                .map(|r| rejected_value(&parse_rejected(r)))
                .unwrap_or_default();
            frontmatter::push_kv(&mut lines, "rejected", &rejected);
            if let Some(notes) = &p.notes {
                for (key, value) in [
                    ("issue", &notes.issue),
                    ("why", &notes.why),
                    ("verified", &notes.verified),
                ] {
                    if let Some(value) = value.as_deref().map(|v| one_line(v, 300)) {
                        frontmatter::push_kv(&mut lines, key, &value);
                    }
                }
            }
            frontmatter::push_kv(&mut lines, "evidence", &p.anchor);
            frontmatter::push_kv(&mut lines, "source", &p.source_title);
            frontmatter::push_kv(&mut lines, "accepted_at", &now.to_rfc3339());
            lines.push("---".into());
            (DECISIONS_DIR, lines)
        }
        "person_fact" => {
            let person = match edits.person.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                Some(path) => {
                    let path = path.trim_start_matches("vault:").to_string();
                    if !path.starts_with("people/") || !path.ends_with(".md") || !vault.join(&path).is_file() {
                        return Err(format!("{path} isn't a person's note in people/."));
                    }
                    path
                }
                None => p
                    .counterparty
                    .as_deref()
                    .and_then(|n| person_for(vault, n))
                    .ok_or_else(|| {
                        format!(
                            "Choose who this is about: no single people/ note is exactly '{}'. Hollow doesn't guess.",
                            p.counterparty.as_deref().unwrap_or("unnamed")
                        )
                    })?,
            };
            let mut lines = vec!["---".to_string()];
            frontmatter::push_kv(&mut lines, "type", "person-fact");
            frontmatter::push_kv(&mut lines, "title", &p.text);
            frontmatter::push_kv(&mut lines, "person", &person);
            frontmatter::push_kv(&mut lines, "stated_at", &date);
            frontmatter::push_kv(&mut lines, "evidence", &p.anchor);
            frontmatter::push_kv(&mut lines, "source", &p.source_title);
            frontmatter::push_kv(&mut lines, "accepted_at", &now.to_rfc3339());
            lines.push("---".into());
            (FACTS_DIR, lines)
        }
        other => return Err(format!("A record proposal can't be a {other}.")),
    };
    let (dir, lines) = content;
    let body = format!(
        "{}\n\n# {}\n\n{}\n\nEvidence: “{}” — {}\n",
        lines.join("\n"),
        p.text,
        p.text,
        p.evidence_excerpt,
        p.anchor
    );
    let approved = serde_json::to_value(&p).map_err(|e| e.to_string())?;
    review_queue::claim(vault, &item.id, Some(approved))?;
    let rel = match create_unique(vault, dir, &date, &p.text, &body) {
        Ok(rel) => rel,
        Err(e) => {
            let _ = review_queue::fail(vault, &item.id, &e);
            return Err(e);
        }
    };
    review_queue::accept(vault, &item.id)?;
    refresh_people_for(vault, &rel);
    refresh_global_summary(vault);
    announce(vault, &rel, "recorded");
    Ok(rel)
}

/// DR1b: tell the tracker's build log that decision `rel` was `change`d
/// (recorded, or its new status). Local and never fails the caller: a decision
/// with no `issue` makes nothing, and what a decision with one does (audit
/// only, or a queued comment) is the tracker's `trackAgentWork` mode's call.
fn announce(vault: &Path, rel: &str, change: &str) {
    if let Some(d) = std::fs::read_to_string(vault.join(rel))
        .ok()
        .and_then(|raw| parse_decision(rel, &raw))
    {
        crate::tracker::decision_log::after_change(vault, &d, change);
    }
}

/// Supersede decision `old` with `new` (confirm tier; never automatic):
/// `old` becomes `superseded` with `superseded_by: new`, `new` gets
/// `supersedes: old`. Both must be decisions; `old` must not be superseded
/// already, and `new` must be settled and not already supersede another.
pub fn supersede_decision(
    vault: &Path,
    old: &str,
    new: &str,
) -> Result<(Decision, Decision), String> {
    let read = |rel: &str| -> Result<Decision, String> {
        let rel = rel.trim().trim_start_matches("vault:");
        if !rel.starts_with("decisions/") {
            return Err(format!("{rel} is not under decisions/."));
        }
        let full = crate::vault::safe_join(vault, rel)?;
        let raw = std::fs::read_to_string(full).map_err(|e| format!("{rel}: {e}"))?;
        parse_decision(rel, &raw).ok_or_else(|| format!("{rel} is not a decision record."))
    };
    let (o, n) = (read(old)?, read(new)?);
    if o.path == n.path {
        return Err("A decision can't supersede itself.".into());
    }
    if o.status == "superseded" {
        return Err(format!(
            "{} is already superseded by {}.",
            o.path,
            o.superseded_by.unwrap_or_default()
        ));
    }
    if o.status == "retracted" {
        return Err(format!(
            "{} is retracted (withdrawn); settle it again before superseding it.",
            o.path
        ));
    }
    if n.status != "settled" {
        return Err(format!("{} is {}, not settled.", n.path, n.status));
    }
    if n.supersedes.as_deref().is_some_and(|s| s != o.path) {
        return Err(format!(
            "{} already supersedes {}.",
            n.path,
            n.supersedes.unwrap_or_default()
        ));
    }
    crate::vault::mutate_note(vault, &o.path, |raw| {
        frontmatter::patch(
            raw,
            &[
                ("status", KeyEdit::Set("superseded".into())),
                ("superseded_by", KeyEdit::Set(n.path.clone())),
                (
                    "superseded_at",
                    KeyEdit::Set(chrono::Local::now().format("%Y-%m-%d").to_string()),
                ),
            ],
            None,
        )
    })?;
    crate::vault::mutate_note(vault, &n.path, |raw| {
        frontmatter::patch(raw, &[("supersedes", KeyEdit::Set(o.path.clone()))], None)
    })?;
    refresh_people_for(vault, &o.path);
    refresh_global_summary(vault);
    announce(vault, &o.path, "superseded");
    Ok((read(&o.path)?, read(&n.path)?))
}

/// Set a decision's status to `settled`, `revisit` or `retracted` (a
/// person's call; the agent action offers only the first two). A retracted
/// decision stays in `decisions/`; a person can settle it again.
pub fn set_decision_status(vault: &Path, rel: &str, status: &str) -> Result<Decision, String> {
    let rel = rel.trim().trim_start_matches("vault:");
    if !matches!(status, "settled" | "revisit" | "retracted") {
        return Err(
            "A decision can be set to settled, revisit or retracted; superseding has its own action."
                .into(),
        );
    }
    if !rel.starts_with("decisions/") {
        return Err(format!("{rel} is not under decisions/."));
    }
    let raw =
        std::fs::read_to_string(crate::vault::safe_join(vault, rel)?).map_err(|e| e.to_string())?;
    let d = parse_decision(rel, &raw).ok_or("not a decision record")?;
    if d.status == "superseded" {
        return Err(
            "A superseded decision stays superseded; settle its replacement instead.".into(),
        );
    }
    if status == "retracted" {
        if let Some(older) = d.supersedes.as_deref() {
            return Err(format!(
                "{rel} replaced {older}, so retracting it would leave that one superseded by a withdrawn decision. Retract is for a decision that replaced nothing."
            ));
        }
    }
    let retracted_at = if status == "retracted" {
        KeyEdit::Set(chrono::Local::now().format("%Y-%m-%d").to_string())
    } else {
        KeyEdit::Remove
    };
    let changed = d.status != status;
    crate::vault::mutate_note(vault, rel, |raw| {
        frontmatter::patch(
            raw,
            &[
                ("status", KeyEdit::Set(status.into())),
                ("retracted_at", retracted_at.clone()),
            ],
            None,
        )
    })?;
    refresh_global_summary(vault);
    if changed {
        announce(vault, rel, status);
    }
    let raw = std::fs::read_to_string(vault.join(rel)).map_err(|e| e.to_string())?;
    parse_decision(rel, &raw).ok_or_else(|| "not a decision record".into())
}

// ── People notes ───────────────────────────────────────────────────────────

/// Replace the generated block of `note` with `generated`, keeping every
/// byte outside it. With no block yet, it is appended after one blank line.
pub fn splice_generated(note: &str, generated: &str) -> Result<String, String> {
    let block = format!(
        "{GENERATED_START}\n{}\n{GENERATED_END}",
        generated.trim_end()
    );
    if let Some(start) = note.find(GENERATED_START) {
        // The end that closes this block: the first one after its start.
        // A start with no end, or a second start, means the fence was
        // edited by hand; writing would risk the person's text, so refuse.
        let Some(len) = note[start..].find(GENERATED_END) else {
            return Err("its Hollow-written block has a start marker but no end marker; restore the end line (or remove the start line) and Hollow will write it again".into());
        };
        let end = start + len + GENERATED_END.len();
        if note[end..].contains(GENERATED_START) {
            return Err("it has more than one Hollow-written block; remove the extra one and Hollow will write it again".into());
        }
        return Ok(format!("{}{block}{}", &note[..start], &note[end..]));
    }
    let sep = if note.is_empty() || note.ends_with("\n\n") {
        ""
    } else if note.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };
    Ok(format!("{note}{sep}{block}\n"))
}

/// A link from a person's note (`people/<Name>.md`) to vault path `target`.
pub fn link(person: &str, target: &str) -> String {
    let up = "../".repeat(person.matches('/').count());
    format!("<{up}{target}>")
}

/// The person's open commitments, both ways.
fn commitments_with(vault: &Path, person: &str, name: &str) -> (Vec<String>, Vec<String>) {
    let (mut owes_me, mut i_owe) = (Vec::new(), Vec::new());
    let name = name.to_lowercase();
    for c in crate::vault_commitments::list(vault) {
        if c.status != "open" {
            continue;
        }
        let theirs =
            c.person_path.as_deref() == Some(person) || c.counterparty.to_lowercase() == name;
        if !theirs {
            continue;
        }
        let due = c
            .due
            .as_deref()
            .map(|d| format!(" (due {d})"))
            .unwrap_or_default();
        let line = format!("- [{}]({}){due}", c.title, link(person, &c.vault_path));
        if c.direction == "owed_to_me" {
            owes_me.push(line);
        } else {
            i_owe.push(line);
        }
    }
    (owes_me, i_owe)
}

/// The generated section for one person.
pub fn generated_for(vault: &Path, person: &str, name: &str) -> String {
    let (owes_me, i_owe) = commitments_with(vault, person, name);
    let lower = name.to_lowercase();
    let decided: Vec<String> = decisions(vault)
        .into_iter()
        .filter(|d| {
            !matches!(d.status.as_str(), "superseded" | "retracted")
                && d.decided_by.to_lowercase() == lower
        })
        .map(|d| {
            format!(
                "- {} — {} [{}]({})",
                d.decided_at,
                d.text,
                d.status,
                link(person, &d.path)
            )
        })
        .collect();
    let known: Vec<String> = facts(vault)
        .into_iter()
        .filter(|f| f.person == person)
        .map(|f| {
            format!(
                "- {} ([{}]({}))",
                f.text,
                f.stated_at,
                link(person, &f.path)
            )
        })
        .collect();
    let section = |title: &str, lines: Vec<String>| {
        if lines.is_empty() {
            format!("### {title}\n\nNothing recorded.")
        } else {
            format!("### {title}\n\n{}", lines.join("\n"))
        }
    };
    let mut parts = vec![
        "## From your records".to_string(),
        section("Owes you", owes_me),
        section("You owe", i_owe),
    ];
    parts.extend(crate::people_dossier::extra_sections(vault, person, name));
    parts.push(section("Their decisions", decided));
    parts.push(section("Facts you accepted", known));
    parts.join("\n\n")
}

/// Regenerate one person's block; whether the note changed.
pub fn regenerate_person(vault: &Path, person: &str) -> Result<bool, String> {
    let person = person.trim().trim_start_matches("vault:");
    if !person.starts_with("people/") || !person.ends_with(".md") {
        return Err(format!("{person} isn't a person's note."));
    }
    let name = crate::vault_people::list(vault)
        .into_iter()
        .find(|p| p.path == person)
        .map(|p| p.name)
        .ok_or_else(|| format!("{person} isn't a person's note."))?;
    let generated = generated_for(vault, person, &name);
    let before = std::fs::read_to_string(vault.join(person)).map_err(|e| e.to_string())?;
    let after = splice_generated(&before, &generated).map_err(|e| format!("{person}: {e}"))?;
    if after == before {
        return Ok(false);
    }
    crate::vault::mutate_note(vault, person, |raw| {
        if raw != before {
            // Changed underneath: splice into what is there now.
            return splice_generated(raw, &generated);
        }
        Ok(after.clone())
    })?;
    Ok(true)
}

/// Refresh the notes of the people a record names (best effort).
fn refresh_people_for(vault: &Path, rel: &str) {
    let Ok(raw) = std::fs::read_to_string(vault.join(rel)) else {
        return;
    };
    let mut people = Vec::new();
    if let Some(f) = parse_fact(rel, &raw) {
        people.push(f.person);
    }
    if let Some(d) = parse_decision(rel, &raw) {
        if let Some(p) = person_for(vault, &d.decided_by) {
            people.push(p);
        }
    }
    for p in people {
        if let Err(e) = regenerate_person(vault, &p) {
            eprintln!("[records] {p}: {e}");
        }
    }
}

// ── The decision ledger (#20) ──────────────────────────────────────────────

/// The person's decisions file; its generated block lists the global
/// settled decisions (the plan's recommendation: generated section, free
/// text above it).
pub const PROFILE_DECISIONS: &str = "me/Decisions.md";
/// Global decisions listed in [`PROFILE_DECISIONS`] at most.
const SUMMARY_MAX: usize = 40;

/// The generated block of `me/Decisions.md`.
pub fn global_summary(vault: &Path) -> String {
    let settled: Vec<String> = decisions(vault)
        .into_iter()
        .filter(|d| d.scope == "global" && d.status == "settled")
        .take(SUMMARY_MAX)
        .map(|d| {
            format!(
                "- {} — {} ([record]({}))",
                d.decided_at,
                d.text,
                link(PROFILE_DECISIONS, &d.path)
            )
        })
        .collect();
    if settled.is_empty() {
        "## Settled decisions (from decisions/)\n\nNone recorded yet.".into()
    } else {
        format!(
            "## Settled decisions (from decisions/)\n\n{}",
            settled.join("\n")
        )
    }
}

/// Refresh `me/Decisions.md`'s generated block when the file exists (never
/// creates the person's profile file). Best effort.
pub fn refresh_global_summary(vault: &Path) {
    if !vault.join(PROFILE_DECISIONS).is_file() {
        return;
    }
    let generated = global_summary(vault);
    if let Err(e) = crate::vault::mutate_note(vault, PROFILE_DECISIONS, |raw| {
        splice_generated(raw, &generated)
    }) {
        eprintln!("[records] {PROFILE_DECISIONS}: {e}");
    }
}

/// A settled decision a proposed one may contradict: near-identical wording
/// that changes a value, or the same subject words with different wording.
/// `None` when nothing settled is close. A flag, never an edit.
pub fn conflict_with(vault: &Path, text: &str) -> Option<Decision> {
    // Close wording that isn't the same wording: a changed value, a "not",
    // a different day. The person reads both; Hollow never decides which.
    decisions(vault)
        .into_iter()
        .filter(|d| d.status == "settled")
        .find(|d| crate::extract::near(text, &d.text) && !same_wording(text, &d.text))
}

/// The same words, ignoring case, spacing and punctuation.
fn same_wording(a: &str, b: &str) -> bool {
    let words = |s: &str| {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(str::to_lowercase)
            .collect::<Vec<_>>()
    };
    words(a) == words(b)
}

/// What an agent (or the person) proposes as a decision.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProposeDecision {
    pub text: String,
    pub decided_by: Option<String>,
    pub scope: Option<String>,
    /// Where it was decided: an anchor, a path or a session turn.
    pub evidence: String,
    /// A settled decision the proposer thinks it contradicts.
    pub conflicts_with: Option<String>,
    /// Options turned down and why, `option -> reason; option -> reason`
    /// (DR1). Rides to the record when the person accepts. There is no way to
    /// propose a constraint: only a person marks one.
    pub rejected: Option<String>,
    /// The tracker issue key (`ENG-12`) a build-log comment goes on when it is
    /// recorded or its status changes (DR1b). Without one, none is made.
    pub issue: Option<String>,
    /// Why it was decided, for that comment (DR1b).
    pub why: Option<String>,
    /// How it was checked, for that comment (DR1b).
    pub verified: Option<String>,
}

/// Queue a proposed decision for the person (#20: agents propose, cite and
/// flag a conflict; they never settle or supersede). A settled decision with
/// the same wording refuses; a conflicting one is named on the card.
pub fn propose_decision(
    vault: &Path,
    input: &ProposeDecision,
    source: &str,
) -> Result<Value, String> {
    let text = one_line(&input.text, TEXT_MAX);
    if text.is_empty() {
        return Err("A decision needs its text.".into());
    }
    let evidence = one_line(&input.evidence, 400);
    if evidence.is_empty() {
        return Err(
            "Say where it was decided (evidence): an anchor, a file or a conversation turn.".into(),
        );
    }
    if let Some(same) = decisions(vault)
        .into_iter()
        .find(|d| d.status == "settled" && same_wording(&text, &d.text))
    {
        return Err(format!(
            "Already settled: {} ({}). Cite it instead.",
            same.text, same.path
        ));
    }
    let named = match input
        .conflicts_with
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(path) => Some(
            decisions(vault)
                .into_iter()
                .find(|d| d.path == path.trim_start_matches("vault:"))
                .ok_or_else(|| format!("{path} isn't a decision record."))?,
        ),
        None => None,
    };
    let conflict = named.or_else(|| conflict_with(vault, &text));
    let flag = conflict.as_ref().map(|d| {
        format!(
            "Possibly contradicts settled decision “{}” ({})",
            d.text, d.path
        )
    });
    let scope = input
        .scope
        .clone()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(ws) = scope.as_deref().filter(|s| *s != "global") {
        if crate::workspaces::summary(vault, ws).is_err() {
            return Err(format!(
                "No workspace '{ws}': use global or a workspace id."
            ));
        }
    }
    let p = crate::extract::Proposal {
        kind: "decision".into(),
        text,
        owner: input
            .decided_by
            .clone()
            .map(|o| one_line(&o, 80))
            .filter(|o| !o.is_empty()),
        counterparty: None,
        due: None,
        anchor: evidence.clone(),
        at: "proposed".into(),
        evidence_excerpt: evidence,
        source_id: format!("proposal:{}", scope.as_deref().unwrap_or("global")),
        source_title: scope.clone().unwrap_or_else(|| "global".into()),
        possible_duplicate: flag.clone(),
        rejected: input
            .rejected
            .as_deref()
            .map(|r| rejected_value(&parse_rejected(r)))
            .filter(|r| !r.is_empty()),
        notes: Some(crate::extract::DecisionNotes {
            issue: input.issue.clone(),
            why: input.why.clone(),
            verified: input.verified.clone(),
        })
        .filter(|n| n.issue.is_some() || n.why.is_some() || n.verified.is_some()),
    };
    let item = crate::extract::enqueue_record(vault, source, None, &p)?;
    Ok(serde_json::json!({ "reviewId": item.id, "status": item.status, "conflict": flag }))
}

/// The one-time importer (#20, on request): each top-level bullet of a
/// workspace brief's `## Decisions…` section becomes a decision proposal,
/// read as written (no model): its first `YYYY-MM-DD` noted, scope the
/// workspace. Dedup and sticky fingerprints keep a second run from
/// repeating any.
pub fn import_brief_decisions(vault: &Path, workspace: &str) -> Result<Value, String> {
    let rel = format!("workspaces/{workspace}/WORKSPACE.md");
    let raw = std::fs::read_to_string(crate::vault::safe_join(vault, &rel)?)
        .map_err(|e| format!("{rel}: {e}"))?;
    let bullets = brief_decision_bullets(&raw);
    if bullets.is_empty() {
        return Err(format!("{rel} has no ## Decisions section with bullets."));
    }
    let mut known: Vec<String> = existing_texts(vault);
    known.extend(
        review_queue::list(vault, Some(crate::extract::RECORD_KIND))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|i| {
                i.payload
                    .get("text")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            }),
    );
    let (mut queued, mut skipped) = (Vec::new(), Vec::new());
    for bullet in bullets {
        let text = one_line(&bullet, TEXT_MAX);
        if !matches!(
            crate::extract::local_dup(&text, &known),
            crate::extract::Dup::New
        ) {
            skipped.push(text);
            continue;
        }
        let date = text
            .split(|c: char| !(c.is_ascii_digit() || c == '-'))
            .find(|w| w.len() == 10 && chrono::NaiveDate::parse_from_str(w, "%Y-%m-%d").is_ok())
            .map(str::to_string);
        let p = crate::extract::Proposal {
            kind: "decision".into(),
            text: text.clone(),
            owner: None,
            counterparty: None,
            due: None,
            anchor: format!("{rel}#decisions"),
            at: date.clone().unwrap_or_else(|| "undated".into()),
            evidence_excerpt: one_line(&bullet, 400),
            source_id: format!("recording:workspaces/{workspace}/WORKSPACE.md"),
            source_title: format!("{workspace} brief"),
            possible_duplicate: None,
            rejected: None,
            notes: None,
        };
        let item = crate::extract::enqueue_record(vault, "chat", Some(rel.clone()), &p)?;
        known.push(text.clone());
        queued.push(serde_json::json!({ "text": text, "reviewId": item.id }));
    }
    Ok(serde_json::json!({ "queued": queued, "skipped": skipped }))
}

/// The top-level bullets of the first `## Decisions…` section, each with
/// its continuation and nested lines joined.
pub fn brief_decision_bullets(brief: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for line in brief.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if inside {
                break;
            }
            inside = heading.trim().to_lowercase().starts_with("decisions");
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            out.push(rest.trim().to_string());
        } else if !line.trim().is_empty() && line.starts_with(char::is_whitespace) {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(line.trim().trim_start_matches("- "));
            }
        }
    }
    out.into_iter()
        .map(|b| b.replace("**", ""))
        .filter(|b| !b.is_empty())
        .collect()
}

// ── Recall ─────────────────────────────────────────────────────────────────

/// Decisions as trusted-recall entries (`vault_learning::recall_with`):
/// settled and superseded ones (history), never `revisit`. A retracted one
/// is history too: labelled, recalled only for a past-tense request, like a
/// superseded one. Not global, so recalled only when the request shares
/// words with them or names them. A settled `constraint: true` decision is
/// marked so the recall budget never cuts it (DR1); a superseded or
/// retracted one is no longer binding, so it is not.
pub fn decision_contexts(
    vault: &Path,
    scope: Option<&str>,
) -> Vec<crate::vault_learning::LearningContext> {
    decisions(vault)
        .into_iter()
        .filter(|d| d.status != "revisit")
        .filter(|d| d.scope == "global" || scope == Some(d.scope.as_str()))
        .map(|d| {
            let retracted = d.status == "retracted";
            let superseded = d.status == "superseded" || retracted;
            let label = if retracted {
                "Retracted: this decision was withdrawn; do not follow it without confirmation. "
                    .to_string()
            } else if d.status == "superseded" {
                match d.superseded_by.as_deref() {
                    Some(by) => format!("Superseded by {by}: do not follow without confirmation. "),
                    None => "Superseded: do not follow without confirmation. ".to_string(),
                }
            } else if d.constraint {
                "Constraint: ".to_string()
            } else {
                String::new()
            };
            let statement = format!(
                "{label}{} {}{}: {} (decided by {}{}; see {}).{}",
                if retracted { "Decided" } else { "Settled" },
                d.decided_at,
                if d.scope == "global" {
                    String::new()
                } else {
                    format!(" in {}", d.scope)
                },
                d.text,
                d.decided_by,
                d.evidence
                    .as_deref()
                    .map(|e| format!(", evidence {e}"))
                    .unwrap_or_default(),
                d.path,
                rejected_sentence(&d.rejected)
            );
            crate::vault_learning::LearningContext {
                path: d.path.clone(),
                title: d.text.clone(),
                target_kind: "decision-record".into(),
                target_path: d.path.clone(),
                subject: d.scope.clone(),
                statement,
                source_run_id: None,
                learned_at: d.decided_at.clone(),
                supersedes: d.supersedes.clone(),
                superseded_by: d.superseded_by.clone(),
                lifecycle: crate::note_lifecycle::Lifecycle {
                    learning: true,
                    approved: true,
                    superseded,
                    superseded_by: d.superseded_by.clone(),
                    supersedes: d.supersedes.clone(),
                    dated: Some(d.decided_at.clone()),
                    ..Default::default()
                },
                constraint: d.constraint && !superseded,
                retracted,
            }
        })
        .collect()
}

/// One person, as the records see them: the note, the open commitments both
/// ways, their decisions and the accepted facts. `reference` is a
/// `people/` path or an exact name.
pub fn person_record(vault: &Path, reference: &str) -> Result<Value, String> {
    let reference = reference.trim().trim_start_matches("vault:");
    let path = if reference.starts_with("people/") {
        reference.to_string()
    } else {
        person_for(vault, reference)
            .ok_or_else(|| format!("No single people/ note is exactly '{reference}'."))?
    };
    let person = crate::vault_people::list(vault)
        .into_iter()
        .find(|p| p.path == path)
        .ok_or_else(|| format!("{path} isn't a person's note."))?;
    let lower = person.name.to_lowercase();
    let commitments: Vec<_> = crate::vault_commitments::list(vault)
        .into_iter()
        .filter(|c| c.status == "open")
        .filter(|c| {
            c.person_path.as_deref() == Some(path.as_str())
                || c.counterparty.to_lowercase() == lower
        })
        .collect();
    let (owes_me, i_owe): (Vec<_>, Vec<_>) = commitments
        .into_iter()
        .partition(|c| c.direction == "owed_to_me");
    Ok(serde_json::json!({
        "person": person,
        "owesMe": owes_me,
        "iOwe": i_owe,
        "decisions": decisions(vault).into_iter().filter(|d| d.decided_by.to_lowercase() == lower).collect::<Vec<_>>(),
        "facts": facts(vault).into_iter().filter(|f| f.person == path).collect::<Vec<_>>(),
    }))
}

// ── Commands ───────────────────────────────────────────────────────────────

/// Everyone with a `people/` note, with how many open commitments each way.
#[tauri::command]
pub fn records_people(state: tauri::State<'_, crate::commands::AppState>) -> Vec<Value> {
    let vault = &state.config.vault_path;
    let open: Vec<_> = crate::vault_commitments::list(vault)
        .into_iter()
        .filter(|c| c.status == "open")
        .collect();
    crate::vault_people::list(vault)
        .into_iter()
        .map(|p| {
            let lower = p.name.to_lowercase();
            let theirs = |dir: &str| {
                open.iter()
                    .filter(|c| c.direction == dir)
                    .filter(|c| c.person_path.as_deref() == Some(p.path.as_str()) || c.counterparty.to_lowercase() == lower)
                    .count()
            };
            serde_json::json!({ "name": p.name, "path": p.path, "role": p.role, "owesMe": theirs("owed_to_me"), "iOwe": theirs("owed_by_me") })
        })
        .collect()
}

/// One person (see [`person_record`]).
#[tauri::command]
pub fn records_person(
    state: tauri::State<'_, crate::commands::AppState>,
    reference: String,
) -> Result<Value, String> {
    person_record(&state.config.vault_path, &reference)
}

/// Decisions, filtered (see [`find_decisions`]).
#[tauri::command]
pub fn records_decisions(
    state: tauri::State<'_, crate::commands::AppState>,
    scope: Option<String>,
    status: Option<String>,
    query: Option<String>,
    as_of: Option<String>,
) -> Vec<Decision> {
    find_decisions(
        &state.config.vault_path,
        scope.as_deref(),
        status.as_deref(),
        query.as_deref(),
        as_of.as_deref(),
    )
}

/// Propose a decision for the person to settle (#20).
#[tauri::command]
pub fn records_propose_decision(
    state: tauri::State<'_, crate::commands::AppState>,
    input: ProposeDecision,
) -> Result<Value, String> {
    propose_decision(&state.config.vault_path, &input, "chat")
}

/// Propose a workspace brief's `## Decisions` bullets as decision records
/// (#20's one-time importer, on request).
#[tauri::command]
pub fn records_import_brief_decisions(
    state: tauri::State<'_, crate::commands::AppState>,
    workspace: String,
) -> Result<Value, String> {
    import_brief_decisions(&state.config.vault_path, &workspace)
}

/// Supersede one decision with another (confirm tier for agents).
#[tauri::command]
pub fn records_supersede_decision(
    state: tauri::State<'_, crate::commands::AppState>,
    old: String,
    new: String,
) -> Result<Value, String> {
    let (o, n) = supersede_decision(&state.config.vault_path, &old, &new)?;
    Ok(serde_json::json!({ "superseded": o, "current": n }))
}

/// Settle a decision again, or mark it for revisit (user-only).
#[tauri::command]
pub fn records_set_decision_status(
    state: tauri::State<'_, crate::commands::AppState>,
    path: String,
    status: String,
) -> Result<Decision, String> {
    set_decision_status(&state.config.vault_path, &path, &status)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::extract::tests::{block_on, source, vault_with_transcript, Fake, ANSWER};
    use crate::extract::{run, RECORD_KIND};

    pub(crate) fn person(v: &Path, name: &str, body: &str) -> String {
        std::fs::create_dir_all(v.join("people")).unwrap();
        let rel = format!("people/{name}.md");
        std::fs::write(
            v.join(&rel),
            format!("---\ntitle: {name}\nrole: Ops\n---\n{body}"),
        )
        .unwrap();
        rel
    }

    fn record_items(v: &Path) -> Vec<ReviewItem> {
        review_queue::list(v, Some(RECORD_KIND)).unwrap()
    }

    #[test]
    fn regenerating_keeps_the_persons_text_byte_for_byte() {
        let mine = "---\ntitle: Ana Ruiz\n---\n## Snapshot\r\nLikes   tables.\n\n<!-- a comment of mine -->\nTrailing words";
        let once = splice_generated(mine, "## From your records\n\nA").unwrap();
        assert!(once.starts_with(mine), "{once:?}");
        let twice = splice_generated(&once, "## From your records\n\nB").unwrap();
        assert!(twice.starts_with(mine));
        assert!(twice.contains("\nB\n") && !twice.contains("\nA\n"));
        // Text after the block is kept too.
        let after = format!("{twice}\nMy own note after.\n");
        let again = splice_generated(&after, "C").unwrap();
        assert!(
            again.starts_with(mine) && again.ends_with("\nMy own note after.\n"),
            "{again:?}"
        );
        assert_eq!(
            splice_generated(&again, "C").unwrap(),
            again,
            "unchanged is unchanged"
        );
    }

    /// Review fix: a hand-broken fence is refused, never "repaired" by a
    /// splice that would swallow the person's text.
    #[test]
    fn a_broken_fence_is_refused_and_a_stray_end_is_harmless() {
        let orphan = format!("A\n{GENERATED_START}\ngen\nB is mine\n");
        assert!(splice_generated(&orphan, "x")
            .unwrap_err()
            .contains("no end marker"));
        let twice = format!(
            "{GENERATED_START}\na\n{GENERATED_END}\nmine\n{GENERATED_START}\nb\n{GENERATED_END}\n"
        );
        assert!(splice_generated(&twice, "x")
            .unwrap_err()
            .contains("more than one"));
        // An end marker above the block (quoted by the person) stays theirs.
        let quoted = format!("I wrote {GENERATED_END} here.\n");
        let once = splice_generated(&quoted, "x").unwrap();
        assert!(once.starts_with(&quoted));
        assert_eq!(
            splice_generated(&once, "x").unwrap(),
            once,
            "stable, no second block"
        );
        assert_eq!(
            splice_generated(&once, "y")
                .unwrap()
                .matches(GENERATED_START)
                .count(),
            1
        );
    }

    /// EV1: accepting re-checks a quote that names a transcript line: a wrong
    /// one is refused with the nearest real line, nothing is written and the
    /// item stays pending; the right one passes.
    #[test]
    fn accepting_refuses_a_quote_that_is_not_at_its_anchor() {
        let dir = vault_with_transcript();
        let v = dir.path();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n",
        )
        .unwrap();
        block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            chrono::Utc::now(),
        ))
        .unwrap();
        let items = record_items(v);
        let decision = items
            .iter()
            .find(|i| i.payload["kind"] == "decision")
            .unwrap();
        let mut planted = decision.clone();
        planted.payload["evidenceExcerpt"] =
            serde_json::json!("the Kestrel budget ceiling fell to 40,000 EUR");
        let err = accept_proposal(v, &planted, None, chrono::Utc::now()).unwrap_err();
        assert!(
            err.contains(
                "The quote isn't at workspaces/ops/sources/recording-ops-sync-abc.md@00:20"
            ),
            "{err}"
        );
        assert!(
            err.contains("Kestrel budget ceiling rose to 55,000 EUR"),
            "the nearest line: {err}"
        );
        assert!(err.contains("Nothing was saved."));
        assert!(decisions(v).is_empty(), "nothing was written");
        assert_eq!(
            record_items(v).len(),
            items.len(),
            "the item is still pending"
        );
        // The unedited proposal's quote is at its anchor.
        accept_proposal(v, decision, None, chrono::Utc::now()).unwrap();
        assert_eq!(decisions(v).len(), 1);
    }

    #[test]
    fn accepting_writes_records_and_the_persons_block() {
        let dir = vault_with_transcript();
        let v = dir.path();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n",
        )
        .unwrap();
        let ana = person(v, "Ana", "## My notes\nMet at the offsite.\n");
        let before = std::fs::read_to_string(v.join(&ana)).unwrap();
        block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            chrono::Utc::now(),
        ))
        .unwrap();
        let items = record_items(v);
        let decision = items
            .iter()
            .find(|i| i.payload["kind"] == "decision")
            .unwrap();
        let fact = items
            .iter()
            .find(|i| i.payload["kind"] == "person_fact")
            .unwrap();
        let now = chrono::Utc::now();
        let d = accept_proposal(v, decision, None, now).unwrap();
        assert!(d.starts_with("decisions/"), "{d}");
        let recorded = &decisions(v)[0];
        assert_eq!(recorded.scope, "ops", "from the recording's workspace");
        assert_eq!(recorded.decided_by, "Ana");
        assert_eq!(
            recorded.evidence.as_deref(),
            Some("workspaces/ops/sources/recording-ops-sync-abc.md@00:20")
        );
        let f = accept_proposal(v, fact, None, now).unwrap();
        assert!(f.starts_with("records/facts/"));
        let after = std::fs::read_to_string(v.join(&ana)).unwrap();
        assert!(after.starts_with(&before), "the person's text is untouched");
        assert!(after.contains("### Their decisions"));
        assert!(after.contains("Kestrel budget ceiling rose to 55,000 EUR"));
        assert!(after.contains("Ana works from the Boston office"));
        // Accepted once: a second accept refuses.
        let again = review_queue::list(v, Some(RECORD_KIND)).unwrap();
        assert!(again.iter().all(|i| i.status == "accepted"));
    }

    #[test]
    fn a_fact_about_an_unmatched_person_waits_for_the_person() {
        let dir = vault_with_transcript();
        let v = dir.path();
        person(v, "Ana Ruiz", "");
        person(v, "Ana Lopez", "");
        block_on(run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            chrono::Utc::now(),
        ))
        .unwrap();
        let fact = record_items(v)
            .into_iter()
            .find(|i| i.payload["kind"] == "person_fact")
            .unwrap();
        let err = accept_proposal(v, &fact, None, chrono::Utc::now()).unwrap_err();
        assert!(err.contains("Choose who this is about"), "{err}");
        assert!(facts(v).is_empty());
        let picked = serde_json::json!({"person": "people/Ana Ruiz.md"});
        accept_proposal(v, &fact, Some(&picked), chrono::Utc::now()).unwrap();
        assert_eq!(facts(v)[0].person, "people/Ana Ruiz.md");
    }

    fn decision_file(v: &Path, name: &str, text: &str, date: &str) -> String {
        std::fs::create_dir_all(v.join(DECISIONS_DIR)).unwrap();
        let rel = format!("{DECISIONS_DIR}/{name}.md");
        std::fs::write(
            v.join(&rel),
            format!("---\ntype: decision\ntitle: {text}\ndecided_at: {date}\ndecided_by: Alex\nscope: global\nstatus: settled\nevidence: chat:s-1#turn2\n---\n\n# {text}\n\n{text}\n"),
        )
        .unwrap();
        rel
    }

    #[test]
    fn supersede_chains_and_refusals() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let a = decision_file(
            v,
            "2026-10-01-a",
            "Bypass asks for confirmation",
            "2026-10-01",
        );
        let b = decision_file(v, "2026-10-02-b", "Bypass asks nothing", "2026-10-02");
        let c = decision_file(
            v,
            "2026-10-05-c",
            "Bypass asks only for sensitive actions",
            "2026-10-05",
        );
        assert!(supersede_decision(v, &a, &a).is_err());
        supersede_decision(v, &a, &b).unwrap();
        assert!(supersede_decision(v, &a, &c)
            .unwrap_err()
            .contains("already superseded"));
        supersede_decision(v, &b, &c).unwrap();
        let all: BTreeMap<String, Decision> = decisions(v)
            .into_iter()
            .map(|d| (d.path.clone(), d))
            .collect();
        assert_eq!(all[&a].superseded_by.as_deref(), Some(b.as_str()));
        assert_eq!(all[&b].supersedes.as_deref(), Some(a.as_str()));
        assert_eq!(all[&b].superseded_by.as_deref(), Some(c.as_str()));
        assert_eq!(all[&c].status, "settled");
        assert_eq!(
            find_decisions(v, None, Some("settled"), None, None).len(),
            1
        );
        // Revisit and back; a superseded one stays superseded.
        assert_eq!(
            set_decision_status(v, &c, "revisit").unwrap().status,
            "revisit"
        );
        assert!(set_decision_status(v, &a, "settled").is_err());
    }

    /// F4 verify: a relevant decision is cited through the trusted recall;
    /// an unrelated one costs nothing; a revisit isn't recalled; a replaced
    /// one is history only.
    #[test]
    fn the_relevant_decision_is_recalled_and_an_unrelated_one_is_not() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let old = decision_file(
            v,
            "2026-10-01-a",
            "Kestrel vendor shortlist is Acme and Birch",
            "2026-10-01",
        );
        let new = decision_file(
            v,
            "2026-10-04-b",
            "Kestrel vendor shortlist is Acme only",
            "2026-10-04",
        );
        decision_file(
            v,
            "2026-10-03-c",
            "Office plants are watered on Mondays",
            "2026-10-03",
        );
        let revisit = decision_file(
            v,
            "2026-10-03-d",
            "Kestrel launch slips to November",
            "2026-10-03",
        );
        supersede_decision(v, &old, &new).unwrap();
        set_decision_status(v, &revisit, "revisit").unwrap();
        let recall =
            crate::vault_learning::recall(v, "who is on the Kestrel vendor shortlist?", &[]);
        let paths: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, vec![new.as_str()], "{paths:?}");
        assert!(recall
            .prompt_block
            .contains("Settled 2026-10-04: Kestrel vendor shortlist is Acme only"));
        // Review fix: the "Settled <date>, decided by Alex" framing and the
        // path don't match on their own.
        let framing = crate::vault_learning::recall(v, "what did Alex settle in 2026?", &[]);
        assert!(framing.prompt_block.is_empty(), "{}", framing.prompt_block);
        // Another workspace's decision stays there.
        let ops = decision_file(
            v,
            "2026-10-06-e",
            "Kestrel vendor review happens on Tuesdays",
            "2026-10-06",
        );
        crate::vault::mutate_note(v, &ops, |raw| {
            frontmatter::patch(raw, &[("scope", KeyEdit::Set("ops".into()))], None)
        })
        .unwrap();
        let q = "when is the Kestrel vendor review?";
        let elsewhere =
            crate::vault_learning::recall_with_scope(v, q, &[], None, Some("rfp"), None);
        assert!(!elsewhere.items.iter().any(|i| i.path == ops));
        let here = crate::vault_learning::recall_with_scope(v, q, &[], None, Some("ops"), None);
        assert!(here.items.iter().any(|i| i.path == ops));
        let unrelated = crate::vault_learning::recall(v, "draft the parking RFP answer", &[]);
        assert!(
            unrelated.prompt_block.is_empty(),
            "{}",
            unrelated.prompt_block
        );
    }

    /// A decision file with extra frontmatter lines (DR1).
    fn decision_with(v: &Path, name: &str, text: &str, date: &str, extra: &str) -> String {
        std::fs::create_dir_all(v.join(DECISIONS_DIR)).unwrap();
        let rel = format!("{DECISIONS_DIR}/{name}.md");
        std::fs::write(
            v.join(&rel),
            format!("---\ntype: decision\ntitle: {text}\ndecided_at: {date}\ndecided_by: Alex\nscope: global\nstatus: settled\nevidence: chat:s-1#turn2\n{extra}---\n\n# {text}\n\n{text}\n"),
        )
        .unwrap();
        rel
    }

    const BUDGET_QUERY: &str = "kestrel vendor shortlist budget policy review";

    /// DR1: a retrieval at the slot limit keeps every constraint, even when
    /// they rank below five other relevant decisions.
    #[test]
    fn a_retrieval_at_the_slot_limit_keeps_every_constraint() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let strong = "Kestrel vendor shortlist budget policy review";
        for i in 0..6 {
            decision_with(
                v,
                &format!("2026-10-1{i}-strong"),
                &format!("{strong} round {i}"),
                &format!("2026-10-1{i}"),
                "",
            );
        }
        let c1 = decision_with(
            v,
            "2026-09-01-c1",
            "Never email the Kestrel vendors directly",
            "2026-09-01",
            "constraint: true\n",
        );
        let c2 = decision_with(
            v,
            "2026-09-02-c2",
            "Kestrel quotes stay in EUR",
            "2026-09-02",
            "constraint: true\n",
        );
        let recall = crate::vault_learning::recall(v, BUDGET_QUERY, &[]);
        let paths: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&c1.as_str()), "{paths:?}");
        assert!(paths.contains(&c2.as_str()), "{paths:?}");
        assert_eq!(
            paths.len(),
            5,
            "constraints take slots from the rest: {paths:?}"
        );
        // Without constraints the same vault still fills five slots.
        std::fs::remove_file(v.join(&c1)).unwrap();
        std::fs::remove_file(v.join(&c2)).unwrap();
        let plain = crate::vault_learning::recall(v, BUDGET_QUERY, &[]);
        assert_eq!(plain.items.len(), 5);
    }

    /// DR1: the character budget never cuts a constraint either; the other
    /// decisions give way.
    #[test]
    fn the_character_budget_never_cuts_a_constraint() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let long = |lead: &str| format!("{lead} {}", "filler ".repeat(95));
        for i in 0..2 {
            decision_with(
                v,
                &format!("2026-10-1{i}-strong"),
                &long("Kestrel vendor shortlist budget policy review"),
                &format!("2026-10-1{i}"),
                "",
            );
        }
        let c1 = decision_with(
            v,
            "2026-09-01-c1",
            &long("Kestrel vendor hard limit"),
            "2026-09-01",
            "constraint: true\n",
        );
        let c2 = decision_with(
            v,
            "2026-09-02-c2",
            &long("Kestrel vendor hard currency"),
            "2026-09-02",
            "constraint: true\n",
        );
        let recall = crate::vault_learning::recall(v, BUDGET_QUERY, &[]);
        let paths: Vec<&str> = recall.items.iter().map(|i| i.path.as_str()).collect();
        assert!(paths.contains(&c1.as_str()), "{paths:?}");
        assert!(paths.contains(&c2.as_str()), "{paths:?}");
        assert!(recall.prompt_block.contains(&c1) && recall.prompt_block.contains(&c2));
    }

    /// DR1: a superseded record is history with a plain label; a retracted
    /// one is history too, labelled, never recalled as current.
    #[test]
    fn superseded_and_retracted_records_carry_a_label() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let old = decision_with(
            v,
            "2026-10-01-a",
            "Kestrel vendor shortlist is Acme and Birch",
            "2026-10-01",
            "",
        );
        let new = decision_with(
            v,
            "2026-10-04-b",
            "Kestrel vendor shortlist is Acme only",
            "2026-10-04",
            "",
        );
        let gone = decision_with(
            v,
            "2026-10-05-c",
            "Kestrel vendor audit happens in spring",
            "2026-10-05",
            "",
        );
        supersede_decision(v, &old, &new).unwrap();
        set_decision_status(v, &gone, "retracted").unwrap();
        let current = crate::vault_learning::recall(v, "Kestrel vendor shortlist and audit", &[]);
        assert!(current
            .items
            .iter()
            .all(|i| i.path != old && i.path != gone));
        let past = crate::vault_learning::recall(
            v,
            "what was the Kestrel vendor shortlist and audit previously?",
            &[],
        );
        let line = |p: &str| {
            past.prompt_block
                .lines()
                .find(|l| l.contains(&format!("[{p}]")))
                .unwrap_or_else(|| panic!("{p} missing: {}", past.prompt_block))
                .to_string()
        };
        assert!(
            line(&old).contains(&format!(
                "Superseded by {new}: do not follow without confirmation"
            )),
            "{}",
            line(&old)
        );
        assert!(line(&gone).contains("Retracted"), "{}", line(&gone));
        assert!(
            !line(&gone).contains("Settled 2026-10-05"),
            "{}",
            line(&gone)
        );
    }

    /// DR1: rejected options travel with the decision, with the instruction
    /// not to repeat them; the options don't make an unrelated request match.
    #[test]
    fn rejected_options_are_recalled_with_the_instruction() {
        let dir = vault_with_transcript();
        let v = dir.path();
        decision_with(
            v,
            "2026-10-04-b",
            "Kestrel vendor queue runs on files",
            "2026-10-04",
            "rejected: Use Redis -> too heavy for one box; Use SQLite -> no atomic rename\n",
        );
        let recall =
            crate::vault_learning::recall(v, "how does the Kestrel vendor queue run?", &[]);
        assert!(
            recall.prompt_block.contains(
                "Rejected: Use Redis (too heavy for one box); Use SQLite (no atomic rename). Do not repeat these without confirmation."
            ),
            "{}",
            recall.prompt_block
        );
        let aside = crate::vault_learning::recall(v, "should we use redis for sessions?", &[]);
        assert!(aside.prompt_block.is_empty(), "{}", aside.prompt_block);
    }

    /// DR1: an old record with none of the new keys reads exactly as before.
    #[test]
    fn an_old_record_parses_as_before() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let rel = decision_with(
            v,
            "2026-10-01-a",
            "Bypass asks for confirmation",
            "2026-10-01",
            "",
        );
        let d = decisions(v).into_iter().find(|d| d.path == rel).unwrap();
        assert_eq!(d.status, "settled");
        assert!(d.rejected.is_empty());
        assert!(!d.constraint);
        assert!(d.derives_from.is_empty());
        // DR1b: no issue, why or verified either, so it never makes a comment.
        assert!(d.issue.is_none() && d.why.is_none() && d.verified.is_none());
        let recall = crate::vault_learning::recall(v, "does bypass ask for confirmation?", &[]);
        assert!(
            recall.prompt_block.contains(
                "Settled 2026-10-01: Bypass asks for confirmation (decided by Alex, evidence chat:s-1#turn2; see decisions/2026-10-01-a.md)."
            ),
            "{}",
            recall.prompt_block
        );
        assert!(!recall.prompt_block.contains("Rejected"));
    }

    /// DR1: the new keys parse; retraction is a person's status that leaves
    /// the record in place.
    #[test]
    fn the_new_keys_parse_and_retraction_keeps_the_record() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let rel = decision_with(
            v,
            "2026-10-04-b",
            "Kestrel vendor queue runs on files",
            "2026-10-04",
            "rejected: Use Redis -> too heavy; Use SQLite\nconstraint: true\nderives_from: findings/a.md, vault:findings/b.md\n",
        );
        let d = decisions(v).into_iter().find(|d| d.path == rel).unwrap();
        assert_eq!(d.rejected.len(), 2);
        assert_eq!(d.rejected[0].option, "Use Redis");
        assert_eq!(d.rejected[0].reason.as_deref(), Some("too heavy"));
        assert_eq!(d.rejected[1].option, "Use SQLite");
        assert_eq!(d.rejected[1].reason, None);
        assert!(d.constraint);
        assert_eq!(d.derives_from, vec!["findings/a.md", "findings/b.md"]);
        let r = set_decision_status(v, &rel, "retracted").unwrap();
        assert_eq!(r.status, "retracted");
        assert!(v.join(&rel).is_file());
        // The keys survive the status edit, and a person can settle it again.
        assert_eq!(r.rejected.len(), 2);
        assert!(r.constraint);
        assert_eq!(
            set_decision_status(v, &rel, "settled").unwrap().status,
            "settled"
        );
        let err = set_decision_status(v, &rel, "superseded").unwrap_err();
        assert!(err.contains("retracted"), "{err}");
    }

    /// DR1 review: a retraction is dated, as-of reads see it settled before
    /// that day, and settling again clears the date.
    #[test]
    fn a_retraction_is_dated_for_as_of_reads() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let rel = decision_with(
            v,
            "2026-10-04-b",
            "Kestrel queue runs on files",
            "2026-10-04",
            "",
        );
        set_decision_status(v, &rel, "retracted").unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let d = decisions(v).remove(0);
        assert_eq!(d.retracted_at.as_deref(), Some(today.as_str()));
        let before = as_of(&d, "2026-10-05").unwrap();
        assert_eq!(before.status, "settled");
        assert_eq!(as_of(&d, &today).unwrap().status, "retracted");
        set_decision_status(v, &rel, "settled").unwrap();
        let again = decisions(v).remove(0);
        assert_eq!(again.status, "settled");
        assert_eq!(again.retracted_at, None);
        assert!(!std::fs::read_to_string(v.join(&rel))
            .unwrap()
            .contains("retracted_at"));
    }

    /// DR1 review: retracting the replacement of an older decision is refused.
    #[test]
    fn retracting_a_decision_that_replaced_another_is_refused() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let old = decision_with(
            v,
            "2026-10-01-a",
            "Kestrel shortlist is Acme and Birch",
            "2026-10-01",
            "",
        );
        let new = decision_with(
            v,
            "2026-10-04-b",
            "Kestrel shortlist is Acme only",
            "2026-10-04",
            "",
        );
        supersede_decision(v, &old, &new).unwrap();
        let err = set_decision_status(v, &new, "retracted").unwrap_err();
        assert!(
            err.contains("Retract is for a decision that replaced nothing"),
            "{err}"
        );
        assert_eq!(
            decisions(v).iter().find(|d| d.path == new).unwrap().status,
            "settled"
        );
    }

    /// DR1 review: a proposal's rejected text reaches the record, and is read
    /// again at accept (capped), whatever the queue item holds.
    #[test]
    fn rejected_text_reaches_the_record_and_is_sanitised_at_accept() {
        let dir = vault_with_transcript();
        let v = dir.path();
        propose_decision(
            v,
            &ProposeDecision {
                text: "The Kestrel queue runs on plain files".into(),
                evidence: "s-1#turn2".into(),
                rejected: Some("Use Redis -> too heavy; Use SQLite".into()),
                ..Default::default()
            },
            "chat",
        )
        .unwrap();
        let mut item = review_queue::list(v, Some(crate::extract::RECORD_KIND))
            .unwrap()
            .remove(0);
        assert_eq!(
            item.payload["rejected"],
            "Use Redis -> too heavy; Use SQLite"
        );
        let many: Vec<String> = (0..12).map(|i| format!("Option {i} -> why {i}")).collect();
        item.payload["rejected"] = serde_json::Value::String(many.join("; "));
        accept_proposal(v, &item, None, Utc::now()).unwrap();
        let d = decisions(v).remove(0);
        assert_eq!(d.rejected.len(), REJECTED_MAX);
        assert_eq!(d.rejected[0].option, "Option 0");
    }

    /// DR1 review: constraints are exempt from the budget up to 12; more are
    /// cut and the decision log says so. Retracted gets its own note.
    #[test]
    fn at_most_twelve_constraints_are_exempt_and_the_cut_is_logged() {
        let dir = vault_with_transcript();
        let v = dir.path();
        for i in 0..14 {
            decision_with(
                v,
                &format!("2026-09-{:02}-c", i + 1),
                &format!("Kestrel hard limit number {i}"),
                &format!("2026-09-{:02}", i + 1),
                "constraint: true\n",
            );
        }
        let recall = crate::vault_learning::recall(v, "kestrel hard limit", &[]);
        assert_eq!(recall.items.len(), 12, "{}", recall.prompt_block);
        let cut: Vec<_> = recall
            .lifecycle
            .iter()
            .filter(|d| d.status == "constraint" && d.action == "cut")
            .collect();
        assert_eq!(cut.len(), 2);
    }

    #[test]
    fn a_retracted_entry_is_noted_as_withdrawn_not_replaced() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let gone = decision_with(
            v,
            "2026-10-05-c",
            "Kestrel vendor audit happens in spring",
            "2026-10-05",
            "",
        );
        set_decision_status(v, &gone, "retracted").unwrap();
        let past =
            crate::vault_learning::recall(v, "what was the Kestrel vendor audit previously?", &[]);
        assert!(
            past.prompt_block
                .contains("were withdrawn with no replacement"),
            "{}",
            past.prompt_block
        );
        assert!(
            !past
                .prompt_block
                .contains("were replaced by the entry they name"),
            "{}",
            past.prompt_block
        );
    }
}

/// Master plan #20: the decision ledger.
#[cfg(test)]
mod ledger_tests {
    use super::*;
    use crate::extract::tests::vault_with_transcript;

    fn decision(v: &Path, name: &str, text: &str, date: &str, scope: &str) -> String {
        std::fs::create_dir_all(v.join(DECISIONS_DIR)).unwrap();
        let rel = format!("{DECISIONS_DIR}/{name}.md");
        std::fs::write(
            v.join(&rel),
            format!("---\ntype: decision\ntitle: {text}\ndecided_at: {date}\ndecided_by: Alex\nscope: {scope}\nstatus: settled\nevidence: chat:s-1#turn2\n---\n\n# {text}\n"),
        )
        .unwrap();
        rel
    }

    #[test]
    fn a_contradiction_raises_a_flag_not_an_edit() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let settled = decision(
            v,
            "2026-10-02-budget",
            "The Kestrel budget ceiling is 48,500 EUR",
            "2026-10-02",
            "global",
        );
        let before = std::fs::read(v.join(&settled)).unwrap();
        let out = propose_decision(
            v,
            &ProposeDecision {
                text: "The Kestrel budget ceiling is 55,000 EUR".into(),
                evidence: "s-20261009-ab#turn4".into(),
                ..Default::default()
            },
            "chat",
        )
        .unwrap();
        let flag = out["conflict"].as_str().unwrap();
        assert!(
            flag.starts_with("Possibly contradicts settled decision") && flag.contains(&settled),
            "{flag}"
        );
        assert_eq!(
            std::fs::read(v.join(&settled)).unwrap(),
            before,
            "never an edit"
        );
        let item = &review_queue::list(v, Some(crate::extract::RECORD_KIND)).unwrap()[0];
        assert_eq!(item.payload["possibleDuplicate"].as_str(), Some(flag));
        // The same wording again (case and punctuation aside) is refused.
        let again = propose_decision(
            v,
            &ProposeDecision {
                text: "the Kestrel budget ceiling is 48,500 EUR.".into(),
                evidence: "x".into(),
                ..Default::default()
            },
            "chat",
        )
        .unwrap_err();
        assert!(again.starts_with("Already settled"), "{again}");
        assert!(
            propose_decision(
                v,
                &ProposeDecision {
                    text: "x".into(),
                    ..Default::default()
                },
                "chat"
            )
            .is_err(),
            "evidence required"
        );
    }

    #[test]
    fn decisions_read_as_of_a_day() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let a = decision(
            v,
            "2026-10-01-a",
            "Bypass asks for confirmation",
            "2026-10-01",
            "global",
        );
        let b = decision(
            v,
            "2026-10-04-b",
            "Bypass asks nothing",
            "2026-10-04",
            "global",
        );
        supersede_decision(v, &a, &b).unwrap();
        // Pretend the supersede happened on 2026-10-04.
        crate::vault::mutate_note(v, &a, |raw| {
            frontmatter::patch(
                raw,
                &[("superseded_at", KeyEdit::Set("2026-10-04".into()))],
                None,
            )
        })
        .unwrap();
        let then: Vec<(String, String)> = find_decisions(v, None, None, None, Some("2026-10-03"))
            .into_iter()
            .map(|d| (d.path, d.status))
            .collect();
        assert_eq!(then, vec![(a.clone(), "settled".to_string())]);
        let now: Vec<String> = find_decisions(v, None, Some("settled"), None, Some("2026-10-09"))
            .into_iter()
            .map(|d| d.path)
            .collect();
        assert_eq!(now, vec![b]);
    }

    #[test]
    fn the_brief_importer_proposes_once_and_changes_no_brief() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let brief = "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n\n## Outcome\n\nx\n\n## Decisions and constraints\n\n- **Notion edits (2026-10-08, user):** edits are write tier\n  while two-way sync is on.\n- **Main agent:**\n  - M-Q1 act on safe classes.\n\n## History\n\n- not a decision\n";
        std::fs::write(v.join("workspaces/ops/WORKSPACE.md"), brief).unwrap();
        assert_eq!(
            brief_decision_bullets(brief),
            vec![
                "Notion edits (2026-10-08, user): edits are write tier while two-way sync is on."
                    .to_string(),
                "Main agent: M-Q1 act on safe classes.".to_string(),
            ]
        );
        let first = import_brief_decisions(v, "ops").unwrap();
        assert_eq!(first["queued"].as_array().unwrap().len(), 2);
        let item = review_queue::list(v, Some(crate::extract::RECORD_KIND))
            .unwrap()
            .into_iter()
            .find(|i| {
                i.payload["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("Notion edits"))
            })
            .unwrap();
        assert_eq!(item.payload["at"], "2026-10-08");
        assert_eq!(
            item.payload["anchor"],
            "workspaces/ops/WORKSPACE.md#decisions"
        );
        let second = import_brief_decisions(v, "ops").unwrap();
        assert!(second["queued"].as_array().unwrap().is_empty());
        assert_eq!(second["skipped"].as_array().unwrap().len(), 2);
        assert_eq!(
            std::fs::read_to_string(v.join("workspaces/ops/WORKSPACE.md")).unwrap(),
            brief
        );
        assert!(import_brief_decisions(v, "missing").is_err());
    }

    /// Review fixes: close wording without numbers is a conflict to flag,
    /// not "Already settled"; a proposed workspace scope survives accept.
    #[test]
    fn a_not_is_a_conflict_and_a_proposed_scope_is_kept() {
        let dir = vault_with_transcript();
        let v = dir.path();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\n---\n# Ops\n",
        )
        .unwrap();
        let settled = decision(
            v,
            "2026-10-02-fri",
            "Deploy the release on Fridays through the staging pipeline with approval",
            "2026-10-02",
            "global",
        );
        let out = propose_decision(
            v,
            &ProposeDecision {
                text: "Do not deploy the release on Fridays through the staging pipeline with approval".into(),
                evidence: "s-1#turn2".into(),
                scope: Some("ops".into()),
                ..Default::default()
            },
            "chat",
        )
        .unwrap();
        assert!(out["conflict"].as_str().unwrap().contains(&settled));
        assert!(propose_decision(
            v,
            &ProposeDecision {
                text: "x y z".into(),
                evidence: "e".into(),
                scope: Some("nope".into()),
                ..Default::default()
            },
            "chat"
        )
        .is_err());
        let item = review_queue::list(v, Some(crate::extract::RECORD_KIND))
            .unwrap()
            .remove(0);
        accept_proposal(v, &item, None, Utc::now()).unwrap();
        let mine = decisions(v)
            .into_iter()
            .find(|d| d.text.starts_with("Do not"))
            .unwrap();
        assert_eq!(mine.scope, "ops");
    }

    #[test]
    fn me_decisions_keeps_the_persons_text_and_lists_global_settled_ones() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let mine = "# Decisions\n\nSettled decisions, newest first.\n\n## 2026-09-26 — Started using Hollow\n\n- The vault is the single source of truth.\n";
        std::fs::write(v.join(PROFILE_DECISIONS), mine).unwrap();
        let a = decision(
            v,
            "2026-10-01-a",
            "Bypass asks for confirmation",
            "2026-10-01",
            "global",
        );
        let b = decision(
            v,
            "2026-10-04-b",
            "Bypass asks nothing",
            "2026-10-04",
            "global",
        );
        decision(
            v,
            "2026-10-05-c",
            "Ops uses the night queue",
            "2026-10-05",
            "ops",
        );
        supersede_decision(v, &a, &b).unwrap();
        let text = std::fs::read_to_string(v.join(PROFILE_DECISIONS)).unwrap();
        assert!(text.starts_with(mine), "{text}");
        assert!(
            text.contains(
                "- 2026-10-04 — Bypass asks nothing ([record](<../decisions/2026-10-04-b.md>))"
            ),
            "{text}"
        );
        assert!(
            !text.contains("Bypass asks for confirmation"),
            "superseded leaves the list"
        );
        assert!(
            !text.contains("night queue"),
            "workspace-scoped stays in its workspace"
        );
        // No file, no creation.
        std::fs::remove_file(v.join(PROFILE_DECISIONS)).unwrap();
        refresh_global_summary(v);
        assert!(!v.join(PROFILE_DECISIONS).exists());
    }
}
