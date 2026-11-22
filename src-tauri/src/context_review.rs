//! The review card (CX20): one list for everything Hollow found that could
//! become context, each item decided by the person's click on that item.
//!
//! Two sources feed it:
//! - the context scanner's proposals (CX19, `context_scan`): instruction and
//!   memory files on this Mac, staged under `<app support>/Imports/<id>/`;
//! - the Org facts chat extraction queued (CX9, `memory_homes::pending_org`).
//!
//! Each item gets a proposed layer (a project's file: Project; a file in the
//! person's own agent folders: Personal; a queued org fact: Org), the target
//! for every layer it may go to, and why Accept is off where it is. A Claude
//! Code auto-memory entry (`projects/<slug>/memory/*.md`, PX6) takes the layer
//! `memory_sort` proposed at scan time (Home, Personal, Project or Stale) and
//! shows its reason; Stale proposes Skip (its Accept is always off), and its
//! Project choice writes nothing (it needs the project's lead).
//!
//! **Rules.**
//! - [`list`] reads only. Nothing is written before the person's Accept or
//!   Skip on one item; [`accept`] takes exactly one item and there is no batch
//!   form (the command map has none either).
//! - Personal copies the file into the personal-context clone under
//!   `memory/imported/<agent>/...`, through `memory_guard::check_layer_in_root`
//!   (`WriteLayer::Personal`), opened create-new: an existing file is never
//!   overwritten.
//! - Project writes nothing: the file is that project's own, and a change to
//!   it is a pull request in that repo, which this card never opens.
//! - Home (PX6, Claude Code's auto-memory entries only) copies the file into
//!   this Mac's topic folder `home/<machine>/<file>.md` in the personal
//!   clone, through `check_layer_in_root` (`WriteLayer::Home`), create-new.
//!   Without the machine name it is off.
//! - Org becomes a pull request through CX8 (`context_propose::propose`), so
//!   the window confirms first. The request is built here; the caller hands
//!   in the function that opens it, so tests see the request and push
//!   nothing.
//! - A secret-like file, one with a blocking rule hit and one the scan didn't
//!   read can't be accepted. A file is read again at accept time and refused
//!   when its SHA-256 is not the one the scan recorded.
//! - Skip and Accept are recorded in Hollow's app data
//!   (`context-review.json`), never in the vault: a skipped item stays listed
//!   as skipped until Unskip, an accepted one leaves the list.

use crate::context_propose::{self, FileChange, Outcome, Request};
use crate::context_scan::{self, Proposal, ScanManifest, Scope};
use crate::memory_guard::{self, Verdict, WriteLayer};
use crate::memory_homes::PendingOrg;
use crate::session_import::staging::Staging;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The decisions file in Hollow's app data.
pub(crate) const DECISIONS_FILE: &str = "context-review.json";
/// The most decisions kept; the oldest go first.
const DECISIONS_MAX: usize = 4000;
/// Where a Personal import lands in the personal clone.
const PERSONAL_DIR: &str = "memory/imported";
/// Where an Org import of a scanned file lands in the org repo.
const ORG_DIR: &str = "ai-tools/imported";
/// The most characters of an org fact the card shows.
const EXCERPT_CHARS: usize = context_scan::EXCERPT_CHARS;

/// The layer an item goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) enum ContextReviewLayer {
    Personal,
    Project,
    Org,
    /// This Mac's topic folder in the personal clone (PX6, auto-memory only).
    Home,
    /// Proposed Skip (PX6): never accepted, only skipped.
    Stale,
}

/// Why an auto-memory entry got its proposed layer (PX6, `memory_sort`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewSort {
    pub layer: ContextReviewLayer,
    /// The deciding rule (`index`, `home-path`, `default` ...).
    pub rule: String,
    /// One sentence naming the rule that fired.
    pub reason: String,
}

/// A scan rule hit: the rule, its group, how bad and the line. Never the text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewFlag {
    pub rule: String,
    pub group: String,
    #[cfg_attr(test, ts(type = "\"warn\" | \"block\""))]
    pub severity: String,
    pub line: usize,
}

/// One layer an item may go to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewChoice {
    pub layer: ContextReviewLayer,
    /// Personal and Home: the file in the personal clone; Org: the path in
    /// the org repo; Project: the project's folder (an auto-memory entry's
    /// project slug).
    pub target: Option<String>,
    /// Why Accept is off for this layer.
    #[cfg_attr(
        test,
        ts(
            type = "\"no-personal-root\" | \"no-org-root\" | \"no-machine\" | \"target-exists\" | \"refused\" | \"not-a-project-file\" | \"org-only\" | \"stale\" | null"
        )
    )]
    pub disabled: Option<String>,
    /// The refusal sentence when `disabled` is `refused`; the sort's reason
    /// when it is `stale`.
    pub detail: Option<String>,
    /// Accepting opens a pull request (Org).
    pub outward: bool,
}

/// One thing to review.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewItem {
    /// `ctx-<hex>` for a scanned file, `org-<hex>` for a queued org fact.
    pub id: String,
    #[cfg_attr(test, ts(type = "\"scan\" | \"org\""))]
    pub source: String,
    /// The tool a scanned file configures (`claudeCode`, `codex` ...).
    pub agent: Option<String>,
    /// `instructions`, `rule`, `command`, `skill`, `memory` or `orgFact`.
    pub kind: String,
    /// `user` or `project` for a scanned file.
    pub scope: Option<String>,
    pub root_label: String,
    /// A scanned file's absolute path; an org fact's path in the org repo.
    pub path: String,
    pub rel_path: String,
    pub size: u64,
    pub modified: Option<i64>,
    /// A clean file's first characters, or the org fact's lines.
    pub excerpt: Option<String>,
    /// Why there is no excerpt (the scanner's code).
    pub withheld: Option<String>,
    pub secret_like: bool,
    pub flags: Vec<ContextReviewFlag>,
    /// An org fact's scan report lines (warnings only; empty when clean).
    pub scan_notes: Vec<String>,
    pub duplicate_of: Option<String>,
    /// The chat an org fact was learned in.
    pub chat: Option<String>,
    pub proposed_layer: ContextReviewLayer,
    /// An auto-memory entry's sort (PX6); `None` for every other item.
    pub sort: Option<ContextReviewSort>,
    pub choices: Vec<ContextReviewChoice>,
    /// Why no layer can take it.
    #[cfg_attr(test, ts(type = "\"secret\" | \"block-flag\" | \"unread\" | null"))]
    pub blocked: Option<String>,
    /// The blocking rule and its line, for `block-flag`.
    pub blocked_rule: Option<String>,
    pub skipped: bool,
}

/// The card's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewList {
    /// The scan the files come from; `None` before any scan.
    pub scan_id: Option<String>,
    pub items: Vec<ContextReviewItem>,
    /// Items accepted before (no longer listed).
    pub accepted: usize,
}

/// What one Accept did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub(crate) struct ContextReviewAccepted {
    pub id: String,
    pub layer: ContextReviewLayer,
    /// The file written (Personal), the org path (Org) or the project folder.
    pub target: Option<String>,
    /// Whether a file was written on this Mac.
    pub written: bool,
    pub pr_url: Option<String>,
    /// `merged` or `waiting`.
    pub pr_status: Option<String>,
    pub pr_reason: Option<String>,
}

// ── Decisions (app data) ────────────────────────────────────────────────

/// One recorded Skip or Accept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Decision {
    pub id: String,
    /// `skipped` or `accepted`.
    pub action: String,
    #[serde(default)]
    pub layer: Option<ContextReviewLayer>,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    pub at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Decisions {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub decisions: Vec<Decision>,
}

impl Decisions {
    fn get(&self, id: &str, action: &str) -> Option<&Decision> {
        self.decisions
            .iter()
            .find(|d| d.id == id && d.action == action)
    }

    /// An Org Accept for `id` is opening its pull request (a marker younger
    /// than [`OPENING_FOR`]; an older one is from a run that never finished).
    fn in_flight(&self, id: &str) -> bool {
        self.get(id, "opening").is_some_and(|d| {
            chrono::DateTime::parse_from_rfc3339(&d.at).is_ok_and(|at| {
                chrono::Utc::now().signed_duration_since(at)
                    < chrono::Duration::seconds(OPENING_FOR.as_secs() as i64)
            })
        })
    }
}

/// How long an `opening` marker holds a second Org Accept back. CX8's run
/// is a clone, a push, `gh pr create` and a few merge checks.
const OPENING_FOR: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const IN_FLIGHT: &str =
    "A pull request for this item is being opened, or was opened and not recorded. Wait for it, then check the org repo.";

static DECISIONS_LOCK: Mutex<()> = Mutex::new(());
/// One Accept at a time: an Org Accept runs `git` and `gh` for a while, and a
/// second click on the same item must find the first one's record.
static ACCEPT_LOCK: Mutex<()> = Mutex::new(());

/// `<app data>/context-review.json`.
pub(crate) fn decisions_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(DECISIONS_FILE))
}

/// The decisions in `file`. A missing file is none; a damaged one is an
/// error (never silently emptied).
pub(crate) fn read_decisions(file: &Path) -> Result<Decisions, String> {
    let _held = DECISIONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_unlocked(file)
}

fn read_unlocked(file: &Path) -> Result<Decisions, String> {
    match std::fs::read_to_string(file) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| format!("the review card's record is unreadable: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Decisions::default()),
        Err(e) => Err(format!("read the review card's record: {e}")),
    }
}

fn change_decisions<T>(
    file: &Path,
    f: impl FnOnce(&mut Decisions) -> Result<T, String>,
) -> Result<T, String> {
    let _held = DECISIONS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut all = read_unlocked(file)?;
    let out = f(&mut all)?;
    all.version = 1;
    if all.decisions.len() > DECISIONS_MAX {
        let extra = all.decisions.len() - DECISIONS_MAX;
        all.decisions.drain(..extra);
    }
    let raw = serde_json::to_string_pretty(&all).map_err(|e| format!("encode record: {e}"))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("create the record's folder: {e}"))?;
    }
    crate::vault::atomic_write_private(file, raw.as_bytes())?;
    Ok(out)
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Skip item `id`: it stays listed as skipped until Unskip. `false` when it
/// was already skipped. An accepted item can't be skipped.
pub(crate) fn skip_at(file: &Path, id: &str) -> Result<bool, String> {
    check_id(id)?;
    change_decisions(file, |all| {
        if all.get(id, "accepted").is_some() {
            return Err("That item was already accepted.".into());
        }
        if all.in_flight(id) {
            return Err(IN_FLIGHT.into());
        }
        if all.get(id, "skipped").is_some() {
            return Ok(false);
        }
        all.decisions.push(Decision {
            id: id.to_string(),
            action: "skipped".into(),
            layer: None,
            target: None,
            url: None,
            at: now(),
        });
        Ok(true)
    })
}

/// Take back a Skip. `false` when the item wasn't skipped.
pub(crate) fn unskip_at(file: &Path, id: &str) -> Result<bool, String> {
    check_id(id)?;
    change_decisions(file, |all| {
        let before = all.decisions.len();
        all.decisions
            .retain(|d| !(d.id == id && d.action == "skipped"));
        Ok(all.decisions.len() != before)
    })
}

fn record_accepted(file: &Path, decision: Decision) -> Result<(), String> {
    change_decisions(file, |all| {
        all.decisions.retain(|d| d.id != decision.id);
        all.decisions.push(decision);
        Ok(())
    })
}

/// Before an Org Accept pushes: record that its pull request is being opened,
/// so a second click (or one after a failed record) can't open another.
fn mark_opening(file: &Path, id: &str) -> Result<(), String> {
    change_decisions(file, |all| {
        if all.get(id, "accepted").is_some() {
            return Err("That item was already accepted.".into());
        }
        if all.in_flight(id) {
            return Err(IN_FLIGHT.into());
        }
        all.decisions.retain(|d| d.id != id);
        all.decisions.push(Decision {
            id: id.to_string(),
            action: "opening".into(),
            layer: Some(ContextReviewLayer::Org),
            target: None,
            url: None,
            at: now(),
        });
        Ok(())
    })
}

/// Drop every decision for `id`.
fn forget(file: &Path, id: &str) -> Result<(), String> {
    change_decisions(file, |all| {
        all.decisions.retain(|d| d.id != id);
        Ok(())
    })
}

/// A queued org fact's id on the card: its org path and its lines, so a
/// later fact with the same title but other text is a new item. (The queue's
/// own id is the path alone, since facts for one path share a proposal.)
pub(crate) fn org_review_id(p: &PendingOrg) -> String {
    let mut h = Sha256::new();
    h.update(
        p.files
            .first()
            .map(|f| f.path.as_str())
            .unwrap_or("")
            .as_bytes(),
    );
    for line in &p.lines {
        h.update([0]);
        h.update(line.as_bytes());
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("org-{}", &hex[..12])
}

fn check_id(id: &str) -> Result<(), String> {
    let ok = (id.starts_with("ctx-") || id.starts_with("org-"))
        && id.len() <= 32
        && id[4..]
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        && id.len() > 4;
    if ok {
        Ok(())
    } else {
        Err("That isn't a review item's id.".into())
    }
}

// ── Where things go ─────────────────────────────────────────────────────

/// The clones and this Mac's name, as Settings, Sources, Context has them.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Roots<'a> {
    pub personal: Option<&'a Path>,
    pub org: Option<&'a Path>,
    pub machine: Option<&'a str>,
}

fn agent_slug(agent: context_scan::Agent) -> &'static str {
    use context_scan::Agent::*;
    match agent {
        ClaudeCode => "claude-code",
        Codex => "codex",
        Cursor => "cursor",
        Gemini => "gemini",
        Windsurf => "windsurf",
        Copilot => "copilot",
        Cline => "cline",
        Generic => "agents",
    }
}

fn wire<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn with_md(path: String) -> String {
    if path.to_ascii_lowercase().ends_with(".md") {
        path
    } else {
        format!("{path}.md")
    }
}

/// A name safe in both clones: letters, digits, space, `-`, `_` and `.`,
/// trimmed, never starting with a dot, and never `.`, `..` or empty (those
/// become `file`).
fn plain_name(part: &str) -> String {
    let mapped: String = part
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_. ".contains(c) {
                c
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = mapped.trim().trim_start_matches('.').trim();
    let name: String = trimmed.chars().take(80).collect();
    let name = name.trim();
    if name.is_empty() || name == "." || name == ".." {
        "file".into()
    } else {
        name.to_string()
    }
}

fn plain_rel(rel: &str) -> String {
    rel.split('/')
        .filter(|p| !p.is_empty())
        .map(plain_name)
        .collect::<Vec<_>>()
        .join("/")
}

/// Where a scanned file lands in the personal clone:
/// `memory/imported/<agent>/<path below its folder>` for the person's own
/// agent folders, with the project's name in between for a project's file.
pub(crate) fn personal_target(p: &Proposal) -> String {
    let agent = agent_slug(p.agent);
    let rel = plain_rel(&p.rel_path);
    with_md(match p.scope {
        Scope::User => format!("{PERSONAL_DIR}/{agent}/{rel}"),
        Scope::Project => format!("{PERSONAL_DIR}/{agent}/{}/{rel}", plain_name(&p.root_label)),
    })
}

/// A file name ending in a lowercase `.md` (the Home layer compares it
/// literally).
fn md_name(name: &str) -> String {
    let plain = plain_name(name);
    if plain.to_ascii_lowercase().ends_with(".md") {
        format!("{}.md", &plain[..plain.len() - 3])
    } else {
        format!("{plain}.md")
    }
}

/// Where an auto-memory entry lands in the Home layer (PX6): this Mac's
/// topic folder, `home/<machine>/<file name>.md`; `None` without a machine
/// name.
pub(crate) fn home_target(p: &Proposal, machine: Option<&str>) -> Option<String> {
    let slug = machine?;
    let name = p.rel_path.rsplit('/').next().unwrap_or(&p.rel_path);
    Some(format!("home/{slug}/{}", md_name(name)))
}

/// An auto-memory entry's sort as the card shows it.
fn sort_of(p: &Proposal) -> Option<ContextReviewSort> {
    use crate::memory_sort::SortLayer;
    let s = p.memory_sort.as_ref()?;
    Some(ContextReviewSort {
        layer: match s.layer {
            SortLayer::Home => ContextReviewLayer::Home,
            SortLayer::Personal => ContextReviewLayer::Personal,
            SortLayer::Project => ContextReviewLayer::Project,
            SortLayer::Stale => ContextReviewLayer::Stale,
        },
        rule: s.rule.clone(),
        reason: s.reason.clone(),
    })
}

/// Where a scanned file lands in the org repo (CX8's allowed folders).
pub(crate) fn org_target(p: &Proposal) -> String {
    let agent = agent_slug(p.agent);
    let rel = plain_rel(&p.rel_path);
    with_md(format!("{ORG_DIR}/{agent}/{rel}"))
}

/// A project file's folder: the path above its `rel_path`.
fn project_folder(p: &Proposal) -> Option<String> {
    p.path
        .strip_suffix(&p.rel_path)
        .map(|s| s.trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

fn blocked_of(p: &Proposal) -> (Option<String>, Option<String>) {
    if p.secret_like {
        return (Some("secret".into()), None);
    }
    if let Some(f) = p.flags.iter().find(|f| f.severity == "block") {
        return (
            Some("block-flag".into()),
            Some(format!("{} (line {})", f.rule, f.line)),
        );
    }
    if p.hash.is_none() {
        return (Some("unread".into()), None);
    }
    (None, None)
}

fn choice(layer: ContextReviewLayer, target: Option<String>) -> ContextReviewChoice {
    ContextReviewChoice {
        layer,
        target,
        disabled: None,
        detail: None,
        outward: layer == ContextReviewLayer::Org,
    }
}

fn off(mut c: ContextReviewChoice, code: &str, detail: Option<String>) -> ContextReviewChoice {
    c.disabled = Some(code.into());
    c.detail = detail;
    c
}

fn personal_choice(target: String, roots: &Roots) -> ContextReviewChoice {
    clone_choice(
        ContextReviewLayer::Personal,
        WriteLayer::Personal,
        target,
        roots,
    )
}

/// A layer in the personal clone (Personal or Home): off without the clone,
/// when the layer's rules refuse the path, or when the file is there.
fn clone_choice(
    layer: ContextReviewLayer,
    guard: WriteLayer,
    target: String,
    roots: &Roots,
) -> ContextReviewChoice {
    let c = choice(layer, Some(target.clone()));
    let Some(root) = roots.personal else {
        return off(c, "no-personal-root", None);
    };
    if let Err(r) = memory_guard::check_layer_in_root(root, guard, roots.machine, &target, &[]) {
        return off(c, "refused", Some(r.message()));
    }
    if std::fs::symlink_metadata(root.join(&target)).is_ok() {
        return off(c, "target-exists", None);
    }
    c
}

/// The Home choice for an auto-memory entry (PX6).
fn home_choice(p: &Proposal, roots: &Roots) -> ContextReviewChoice {
    match home_target(p, roots.machine) {
        Some(target) => clone_choice(ContextReviewLayer::Home, WriteLayer::Home, target, roots),
        None if roots.personal.is_none() => off(
            choice(ContextReviewLayer::Home, None),
            "no-personal-root",
            None,
        ),
        None => off(choice(ContextReviewLayer::Home, None), "no-machine", None),
    }
}

fn scan_choices(p: &Proposal, roots: &Roots) -> Vec<ContextReviewChoice> {
    let personal = personal_choice(personal_target(p), roots);
    let sort = sort_of(p);
    let project = match (p.scope, &sort) {
        (Scope::Project, _) => choice(ContextReviewLayer::Project, project_folder(p)),
        // An auto-memory entry about one codebase: a suggested pull request
        // for that project's lead; Hollow writes nothing (PX6).
        (Scope::User, Some(_)) => choice(
            ContextReviewLayer::Project,
            crate::memory_sort::auto_memory_slug(&p.rel_path).map(str::to_string),
        ),
        (Scope::User, None) => off(
            choice(ContextReviewLayer::Project, None),
            "not-a-project-file",
            None,
        ),
    };
    let target = org_target(p);
    let org = choice(ContextReviewLayer::Org, Some(target.clone()));
    let org = match (roots.org, context_propose::validate_path(&target)) {
        (None, _) => off(org, "no-org-root", None),
        (_, Err(e)) => off(org, "refused", Some(e)),
        _ => org,
    };
    let mut out = vec![personal, project, org];
    if let Some(sort) = sort {
        out.push(home_choice(p, roots));
        if sort.layer == ContextReviewLayer::Stale {
            out.push(off(
                choice(ContextReviewLayer::Stale, None),
                "stale",
                Some(sort.reason),
            ));
        }
    }
    out
}

fn scan_item(p: &Proposal, roots: &Roots, skipped: bool) -> ContextReviewItem {
    let (blocked, blocked_rule) = blocked_of(p);
    ContextReviewItem {
        id: p.id.clone(),
        source: "scan".into(),
        agent: Some(wire(&p.agent)),
        kind: wire(&p.kind),
        scope: Some(wire(&p.scope)),
        root_label: p.root_label.clone(),
        path: p.path.clone(),
        rel_path: p.rel_path.clone(),
        size: p.size,
        modified: p.modified,
        excerpt: p.excerpt.clone(),
        withheld: p.withheld.clone(),
        secret_like: p.secret_like,
        flags: p
            .flags
            .iter()
            .map(|f| ContextReviewFlag {
                rule: f.rule.clone(),
                group: f.group.clone(),
                severity: f.severity.clone(),
                line: f.line,
            })
            .collect(),
        scan_notes: Vec::new(),
        duplicate_of: p.duplicate_of.clone(),
        chat: None,
        proposed_layer: match (sort_of(p), p.scope) {
            (Some(sort), _) => sort.layer,
            (None, Scope::Project) => ContextReviewLayer::Project,
            (None, Scope::User) => ContextReviewLayer::Personal,
        },
        sort: sort_of(p),
        choices: scan_choices(p, roots),
        blocked,
        blocked_rule,
        skipped,
    }
}

fn org_choices(p: &PendingOrg, roots: &Roots) -> Vec<ContextReviewChoice> {
    let path = p.files.first().map(|f| f.path.clone());
    let org = choice(ContextReviewLayer::Org, path);
    let org = match roots.org {
        None => off(org, "no-org-root", None),
        Some(root) => match p.request(true, root) {
            Err(e) => off(org, "refused", Some(e)),
            Ok(_) => org,
        },
    };
    let elsewhere = |layer| off(choice(layer, None), "org-only", None);
    vec![
        elsewhere(ContextReviewLayer::Personal),
        elsewhere(ContextReviewLayer::Project),
        org,
    ]
}

fn org_item(p: &PendingOrg, roots: &Roots, skipped: bool) -> ContextReviewItem {
    let path = p.files.first().map(|f| f.path.clone()).unwrap_or_default();
    let excerpt: String = p.lines.join(" ").chars().take(EXCERPT_CHARS).collect();
    let notes: Vec<String> = p
        .scan
        .iter()
        .filter(|l| l.as_str() != "no findings")
        .cloned()
        .collect();
    let blocked = notes.iter().any(|l| l.starts_with("block"));
    ContextReviewItem {
        id: org_review_id(p),
        source: "org".into(),
        agent: None,
        kind: "orgFact".into(),
        scope: None,
        root_label: p.title.clone(),
        rel_path: path.clone(),
        path,
        size: p.files.first().map(|f| f.content.len() as u64).unwrap_or(0),
        modified: chrono::DateTime::parse_from_rfc3339(&p.created_at)
            .ok()
            .map(|t| t.timestamp()),
        excerpt: (!excerpt.is_empty()).then_some(excerpt),
        withheld: None,
        secret_like: false,
        flags: Vec::new(),
        scan_notes: notes,
        duplicate_of: None,
        chat: p.chat.clone(),
        proposed_layer: ContextReviewLayer::Org,
        sort: None,
        choices: org_choices(p, roots),
        blocked: blocked.then(|| "block-flag".to_string()),
        blocked_rule: None,
        skipped,
    }
}

/// The card's list: the scan's proposals, then the queued org facts, each
/// with its layers and whether it was skipped; accepted items are left out.
/// Reads only.
pub(crate) fn list(
    manifest: Option<&ScanManifest>,
    pending: &[PendingOrg],
    decisions: &Decisions,
    roots: &Roots,
) -> ContextReviewList {
    let mut items = Vec::new();
    let mut accepted = 0;
    let mut add = |id: &str, make: &dyn Fn(bool) -> ContextReviewItem| {
        if decisions.get(id, "accepted").is_some() {
            accepted += 1;
            return;
        }
        items.push(make(decisions.get(id, "skipped").is_some()));
    };
    if let Some(m) = manifest {
        for p in &m.proposals {
            add(&p.id, &|skipped| scan_item(p, roots, skipped));
        }
    }
    for p in pending {
        add(&org_review_id(p), &|skipped| org_item(p, roots, skipped));
    }
    ContextReviewList {
        scan_id: manifest.map(|m| m.scan_id.clone()),
        items,
        accepted,
    }
}

// ── Finding an item ─────────────────────────────────────────────────────

/// The newest staged scan that finished, else `None`.
pub(crate) fn latest_manifest(staging: &Staging) -> Option<ScanManifest> {
    staging.ids().into_iter().find_map(|id| {
        staging
            .read_json::<ScanManifest>(&id, context_scan::MANIFEST)
            .ok()
            .flatten()
            .filter(|m| m.state == "ready")
    })
}

/// Scan `scan_id`'s manifest, else the newest one.
pub(crate) fn manifest_for(
    staging: &Staging,
    scan_id: Option<&str>,
) -> Result<Option<ScanManifest>, String> {
    match scan_id {
        Some(id) => context_scan::manifest(staging, id).map(Some),
        None => Ok(latest_manifest(staging)),
    }
}

/// The item an Accept names.
#[derive(Debug, Clone)]
pub(crate) enum Found {
    Scan(Proposal),
    Org(PendingOrg),
}

impl Found {
    fn id(&self) -> String {
        match self {
            Found::Scan(p) => p.id.clone(),
            Found::Org(p) => org_review_id(p),
        }
    }
}

/// Find item `id` in scan `scan_id` (else the newest scan) or the org queue.
pub(crate) fn find(
    manifest: Option<&ScanManifest>,
    pending: &[PendingOrg],
    id: &str,
) -> Result<Found, String> {
    check_id(id)?;
    if let Some(p) = pending.iter().find(|p| org_review_id(p) == id) {
        return Ok(Found::Org(p.clone()));
    }
    manifest
        .and_then(|m| m.proposals.iter().find(|p| p.id == id))
        .map(|p| Found::Scan(p.clone()))
        .ok_or_else(|| "That item is no longer here; scan again.".to_string())
}

// ── Accept ──────────────────────────────────────────────────────────────

/// Everything an Accept touches outside the item.
pub(crate) struct Ctx<'a> {
    pub roots: Roots<'a>,
    /// The decisions record.
    pub decisions: &'a Path,
    /// The org queue (`memory_homes`), so a submitted fact leaves it.
    pub pending_file: &'a Path,
    /// The vault whose memory log records a Personal copy (`None` in tests
    /// that don't look at the log).
    pub vault: Option<&'a Path>,
}

/// The sentence for a disabled layer (the window has its own words for the
/// card; this is the refusal an Accept returns).
fn disabled_sentence(c: &ContextReviewChoice) -> String {
    match c.disabled.as_deref() {
        Some("no-personal-root") => {
            "Connect your personal context in Settings, Sources, Context first. Nothing was written."
                .into()
        }
        Some("no-org-root") => {
            "Connect the organization context in Settings, Sources, Context first. Nothing was proposed."
                .into()
        }
        Some("target-exists") => format!(
            "{} is already in your personal context; Hollow never overwrites it. Nothing was written.",
            c.target.as_deref().unwrap_or("That file")
        ),
        Some("no-machine") => {
            "This Mac has no machine name yet, so Hollow can't tell which home folder is its own. Set it in Settings, Sources, Context first. Nothing was written."
                .into()
        }
        Some("stale") => format!(
            "Hollow proposes Skip for this entry. {} Skip it, or pick another layer.",
            c.detail.as_deref().unwrap_or("")
        ),
        Some("not-a-project-file") => {
            "This file is in your own agent folder, not a project's repo.".into()
        }
        Some("org-only") => {
            "A fact learned for everyone in your organization goes to the organization layer, or Skip it."
                .into()
        }
        _ => c
            .detail
            .clone()
            .unwrap_or_else(|| "That layer can't take this item.".into()),
    }
}

fn blocked_sentence(code: &str, rule: Option<&str>) -> String {
    match code {
        "secret" => "This file looks like it holds a secret, so Hollow won't copy it anywhere. Take the secret out, then scan again.".into(),
        "block-flag" => format!(
            "A blocking rule matched{}, so Hollow won't copy this. Fix the file, then scan again.",
            rule.map(|r| format!(" ({r})")).unwrap_or_default()
        ),
        _ => "Hollow didn't read this file in the scan (too large, over the read budget or unreadable), so it can't check what it would copy.".into(),
    }
}

/// Read a scanned file again and check it is the one the scan saw: a plain
/// file (no link), within the scanner's size cap, UTF-8, same SHA-256.
pub(crate) fn read_verified(p: &Proposal) -> Result<String, String> {
    let want = p
        .hash
        .as_deref()
        .ok_or_else(|| blocked_sentence("unread", None))?;
    let path = Path::new(&p.path);
    let gone = || {
        format!(
            "{} is gone or changed since the scan. Scan again; nothing was written.",
            p.rel_path
        )
    };
    let meta = std::fs::symlink_metadata(path).map_err(|_| gone())?;
    if meta.file_type().is_symlink() || !meta.is_file() || meta.len() > context_scan::FILE_MAX {
        return Err(gone());
    }
    let file = std::fs::File::open(path).map_err(|_| gone())?;
    let mut bytes = Vec::new();
    file.take(context_scan::FILE_MAX + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| gone())?;
    let got: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        return Err(format!(
            "{} changed since the scan. Scan again to review the new version; nothing was written.",
            p.rel_path
        ));
    }
    String::from_utf8(bytes).map_err(|_| format!("{} isn't text.", p.rel_path))
}

/// Copy `text` into the personal clone at `target`: the layer's rules
/// (Personal, or Home for PX6), no link anywhere on the way, never over an
/// existing file.
fn write_new(
    root: &Path,
    layer: WriteLayer,
    machine: Option<&str>,
    target: &str,
    text: &str,
) -> Result<(), String> {
    match memory_guard::check_layer_in_root(root, layer, machine, target, &[("file", text)]) {
        Ok(Verdict::Write) => {}
        Ok(Verdict::Card(_)) => {
            return Err(format!(
                "{target} waits for its own confirm card; nothing was written."
            ))
        }
        Err(r) => return Err(r.message()),
    }
    let exists = || {
        format!("{target} is already in your personal context; Hollow never overwrites it. Nothing was written.")
    };
    let full = crate::context_roots::resolve_in_root(root, target)
        .map_err(|e| format!("{target}: {e}"))?;
    if std::fs::symlink_metadata(&full).is_ok() {
        return Err(exists());
    }
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {target}'s folder: {e}"))?;
    }
    // The folders just made are checked again: none may be a link.
    let full = crate::context_roots::resolve_in_root(root, target)
        .map_err(|e| format!("{target}: {e}"))?;
    // Create-new never follows or replaces what is there (a link included).
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&full)
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                exists()
            } else {
                format!("write {target}: {e}")
            }
        })?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|e| format!("write {target}: {e}"))
}

/// The CX8 request for an Org Accept: a queued fact's own request (its lines
/// appended to an existing org file), or a scanned file's text as one new
/// file under `ai-tools/imported/`. `attended` is true: it is the person's
/// click.
pub(crate) fn org_request(found: &Found, org_root: &Path) -> Result<Request, String> {
    match found {
        Found::Org(p) => p.request(true, org_root),
        Found::Scan(p) => {
            let text = read_verified(p)?;
            let target = org_target(p);
            let title: String = format!("Add {} from {}", p.rel_path, p.root_label)
                .chars()
                .take(120)
                .collect();
            Ok(Request {
                files: vec![FileChange {
                    path: target,
                    content: text,
                }],
                title,
                reason: format!(
                    "Imported from {} on its owner's Mac by their Accept on Hollow's review card: they judged it true for everyone in your organization.",
                    p.root_label
                ),
                chat: None,
                attended: true,
            })
        }
    }
}

/// Accept one item into `layer`. `propose` opens the pull request for Org
/// (CX8's `propose` with the real runner; a test hands in its own).
pub(crate) fn accept(
    ctx: &Ctx,
    found: &Found,
    layer: ContextReviewLayer,
    propose: &dyn Fn(&Request) -> Result<Outcome, String>,
) -> Result<ContextReviewAccepted, String> {
    let _held = ACCEPT_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let id = found.id();
    let decisions = read_decisions(ctx.decisions)?;
    if decisions.get(&id, "accepted").is_some() {
        return Err("That item was already accepted.".into());
    }
    if decisions.in_flight(&id) {
        return Err(IN_FLIGHT.into());
    }
    let item = match found {
        Found::Scan(p) => scan_item(p, &ctx.roots, false),
        Found::Org(p) => org_item(p, &ctx.roots, false),
    };
    if let Some(code) = &item.blocked {
        return Err(blocked_sentence(code, item.blocked_rule.as_deref()));
    }
    let c = item
        .choices
        .iter()
        .find(|c| c.layer == layer)
        .ok_or("That layer can't take this item.")?;
    if c.disabled.is_some() {
        return Err(disabled_sentence(c));
    }
    let mut done = ContextReviewAccepted {
        id: id.clone(),
        layer,
        target: c.target.clone(),
        written: false,
        pr_url: None,
        pr_status: None,
        pr_reason: None,
    };
    match (layer, found) {
        (ContextReviewLayer::Project, _) => {}
        (ContextReviewLayer::Personal, Found::Scan(p)) => {
            let root = ctx.roots.personal.ok_or("no personal root")?;
            let target = personal_target(p);
            let text = read_verified(p)?;
            write_new(
                root,
                WriteLayer::Personal,
                ctx.roots.machine,
                &target,
                &text,
            )?;
            done.written = true;
            if let Some(vault) = ctx.vault {
                memory_guard::record_or_warn(
                    vault,
                    memory_guard::NewEntry {
                        actor: "person".into(),
                        action: Some(memory_guard::Action::Saved),
                        subject: format!("import:{id}"),
                        path: Some(target.clone()),
                        note: Some(format!(
                            "copied {} from {} into the personal context (review card)",
                            p.rel_path, p.root_label
                        )),
                        ..Default::default()
                    },
                );
            }
        }
        (ContextReviewLayer::Home, Found::Scan(p)) => {
            let root = ctx.roots.personal.ok_or("no personal root")?;
            let target = home_target(p, ctx.roots.machine).ok_or("no machine name")?;
            let text = read_verified(p)?;
            write_new(root, WriteLayer::Home, ctx.roots.machine, &target, &text)?;
            done.written = true;
            if let Some(vault) = ctx.vault {
                memory_guard::record_or_warn(
                    vault,
                    memory_guard::NewEntry {
                        actor: "person".into(),
                        action: Some(memory_guard::Action::Saved),
                        subject: format!("import:{id}"),
                        path: Some(target.clone()),
                        note: Some(format!(
                            "copied {} from {} into this Mac's home folder (review card)",
                            p.rel_path, p.root_label
                        )),
                        ..Default::default()
                    },
                );
            }
        }
        // Stale is never accepted, and Home takes only auto-memory entries;
        // their choices are off or missing, so this is a guard.
        (ContextReviewLayer::Stale, _) | (ContextReviewLayer::Home, Found::Org(_)) => {
            return Err("That layer can't take this item.".into())
        }
        (ContextReviewLayer::Personal, Found::Org(_)) => {
            return Err(disabled_sentence(&off(
                choice(layer, None),
                "org-only",
                None,
            )))
        }
        (ContextReviewLayer::Org, _) => {
            let root = ctx.roots.org.ok_or("no org root")?;
            let req = org_request(found, root)?;
            // The marker goes down before anything is pushed: if the push
            // works and the record after it fails, a second click is refused
            // instead of opening a second pull request.
            mark_opening(ctx.decisions, &id)?;
            let outcome = match propose(&req) {
                Ok(o) => o,
                Err(e) => {
                    if let Err(clear) = forget(ctx.decisions, &id) {
                        eprintln!("[context-review] the opening marker stays: {clear}");
                    }
                    return Err(e);
                }
            };
            done.pr_url = Some(outcome.url);
            done.pr_status = Some(outcome.status);
            done.pr_reason = (!outcome.reason.is_empty()).then_some(outcome.reason);
        }
    }
    record_accepted(
        ctx.decisions,
        Decision {
            id: id.clone(),
            action: "accepted".into(),
            layer: Some(layer),
            target: done.target.clone(),
            url: done.pr_url.clone(),
            at: now(),
        },
    )?;
    // A submitted fact leaves the queue, and then its decision goes too: the
    // card no longer lists it, and a later fact for the same file is new.
    if let Found::Org(p) = found {
        // A fact that joined the same file while the PR was opening keeps it
        // queued: it wasn't in this pull request.
        let same = crate::memory_homes::pending_org_at(ctx.pending_file)
            .map(|all| all.iter().any(|q| q.id == p.id && org_review_id(q) == id))
            .unwrap_or(false);
        let removed = if same {
            crate::memory_homes::remove_pending_at(ctx.pending_file, &p.id)
        } else {
            Ok(false)
        };
        match removed {
            Ok(_) => {
                if let Err(e) = forget(ctx.decisions, &id) {
                    eprintln!("[context-review] the accepted fact's record stays: {e}");
                }
            }
            Err(e) => eprintln!("[context-review] the PR is open but the queued fact stays: {e}"),
        }
    }
    Ok(done)
}

#[cfg(test)]
#[path = "context_review_tests.rs"]
mod tests;
