//! The context prioritizer: decides, per request, how much retrieved
//! evidence and which "attach when relevant" connectors reach the agent.
//! Less context means a faster first token, fewer input tokens and fewer
//! tool schemas for the model to read.
//!
//! Local rules always run: a connector set to "when relevant" is attached
//! only when the request names it or shares a meaningful word with its name
//! or description. With Jev switched on (Settings, Context), TypeSafe's
//! relevance model scores the candidates instead: passages it rates as
//! unrelated are dropped and the rest reordered. A connector the request
//! names is attached outright and never sent; Jev scores at most
//! [`jev::MAX_CANDIDATES`] of the rest (locally preselected), and each one it
//! scores is attached when it judges it needed. Anything it doesn't score
//! (a failure, a missing score, past its limit) keeps its local decision;
//! nothing is ever invented or rewritten. Jev is sent the request's own
//! words only ([`ConnectorQuery`]).
//!
//! What each request decided goes back to its caller, which records it on
//! that run's own context manifest (`run_manifest`, agent effectiveness
//! F11). There is no process-global "last report": two turns preparing at
//! once used to overwrite each other's.
//!
//! A chat turn reuses its conversation's last Jev relevance decision
//! ([`select_connectors_in_session`], S28) for up to [`SESSION_TTL`] while
//! the inventory is unchanged and the request names no connector and shares
//! no word with one left out. Only relevance is kept, never an approval.
use crate::directory_connections::Passage;
use crate::jev::{self, Candidate, Question};
use crate::run_manifest::JevUse;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

/// Jev scores are 0..=3. Below this a passage is "unrelated" and dropped.
const EVIDENCE_KEEP: f64 = 1.0;
/// At or above this a connector is "probably useful" and attached.
const TOOL_ATTACH: f64 = 2.0;

/// Words that say nothing about what a request is about. Also the base of
/// `source_evidence`'s generic words (its relevance floor).
pub(crate) const STOP: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "what", "when", "where", "which",
    "about", "have", "has", "please", "can", "could", "would", "should", "you", "your", "our",
    "are", "was", "were", "will", "all", "any", "some", "get", "make", "use", "using", "need",
    "want",
];

fn terms(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOP.contains(&w.as_str()))
        .collect()
}

/// Local relevance of a connector to a request: the request mentions its
/// id or name, or shares a meaningful word with its name or description.
pub fn locally_relevant(request: &str, id: &str, name: &str, description: &str) -> bool {
    let request_lower = request.to_lowercase();
    if !id.is_empty() && request_lower.contains(&id.to_lowercase()) {
        return true;
    }
    if !name.trim().is_empty() && request_lower.contains(&name.trim().to_lowercase()) {
        return true;
    }
    let wanted = terms(request);
    let offered: BTreeSet<String> = terms(&format!(
        "{name} {description} {}",
        id.replace(['-', '_'], " ")
    ));
    wanted.intersection(&offered).next().is_some()
}

/// A connector the prioritizer may attach or leave out.
pub struct ConnectorCandidate<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub description: &'a str,
}

/// How one "when relevant" connector's attach decision was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ConnectorBasis"))]
#[serde(rename_all = "kebab-case")]
pub enum Basis {
    /// The request names it (id or name): attached, decided before Jev and
    /// never sent to it.
    Named,
    /// Jev scored it; attached at [`TOOL_ATTACH`] or above.
    Jev,
    /// Jev is off (no key or switched off): local relevance.
    Local,
    /// Jev was asked and failed (timeout, HTTP error, unreadable or
    /// malformed answer): local relevance.
    LocalFallback,
    /// Jev answered but gave no valid score for this one (missing, wrong
    /// id, not a number, out of range): local relevance.
    Unscored,
    /// Past the [`jev::MAX_CANDIDATES`] one request scores, after local
    /// preselection: local relevance.
    BeyondLimit,
    /// The conversation's last Jev decision for it, reused with no call
    /// ([`select_connectors_in_session`], S28): relevance only; whether
    /// it runs without a prompt is read afresh every turn.
    Cached,
}

/// One candidate's decision, for the per-run manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConnectorDecision {
    pub id: String,
    pub attach: bool,
    pub basis: Basis,
    /// Jev's score, when [`Basis::Jev`] (or the reused one, [`Basis::Cached`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub score: Option<f64>,
    /// Why Jev's answer wasn't used, when it wasn't.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// What [`select_connectors`] decided: exactly one decision per candidate,
/// in the candidates' order.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ConnectorSelection {
    /// `local`, `jev` (Jev answered, perhaps partly), `jev-fallback` or
    /// `cached` (the conversation's last Jev decision reused, S28).
    pub engine: String,
    pub decisions: Vec<ConnectorDecision>,
    /// When Jev scored only some of those sent, or some were past its 24.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub note: Option<String>,
    /// The Jev call, for the manifest's `jev` list (moved there by
    /// `RunManifest::set_connectors`); `None` when nothing was left to send.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub jev: Option<JevUse>,
}

impl ConnectorSelection {
    /// The ids to attach.
    pub fn attached(&self) -> impl Iterator<Item = &str> {
        self.decisions
            .iter()
            .filter(|d| d.attach)
            .map(|d| d.id.as_str())
    }
}

/// What connector selection reads for one request (wave 1 follow-up to
/// C1). Jev is sent `request` only: the words the person or the workflow
/// gave this turn, never text they didn't type (Hollow sends Jev no new
/// content). `local` is what the local rules (named, locally relevant)
/// read: the request plus the display names of the notes a reference in it
/// resolved to (`TurnRequest::selection_query`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConnectorQuery {
    pub request: String,
    pub local: String,
}

impl ConnectorQuery {
    /// A query with nothing resolved: both readers get `text`.
    pub fn plain(text: &str) -> Self {
        ConnectorQuery {
            request: text.to_string(),
            local: text.to_string(),
        }
    }
}

/// Which "when relevant" connectors to attach for `query`: exactly one
/// decision per candidate, whatever Jev does.
///
/// 1. A connector the request names is attached outright and not sent.
/// 2. With Jev on, at most [`jev::MAX_CANDIDATES`] of the rest go in one
///    request, chosen locally: the locally relevant ones first, then library
///    order. The rest keep their local decision ([`Basis::BeyondLimit`]), so
///    a library of 50 costs one bounded request (in a run, the preparation
///    deadline at worst), not three.
/// 3. A failed request leaves every sent one to local relevance, and a
///    partial answer leaves each unscored one to it, with the reason.
///
/// As a run prepares it (agent effectiveness F8, O2): Jev's call goes through
/// `jev`'s guard (an identical ranking is reused; a failing Jev is paused)
/// and is given up at `deadline`, each recorded on the call's manifest
/// entry (`cached`, `circuit-open`, `deadline`). Named connectors are decided
/// before any of that, so neither the deadline nor the breaker can drop one.
pub async fn select_connectors(
    query: &ConnectorQuery,
    candidates: &[ConnectorCandidate<'_>],
    deadline: &crate::prep_deadline::PrepDeadline,
    jev: &jev::Target<'_>,
) -> ConnectorSelection {
    let served = std::sync::Mutex::new(None);
    let how = &served;
    let mut selection =
        select_connectors_via(query, candidates, move |request, items| async move {
            let ranking = jev::rank(jev, request, &items, Question::Tool, deadline, true).await?;
            *how.lock().unwrap_or_else(|p| p.into_inner()) = Some(ranking.served);
            Some(ranking.result)
        })
        .await;
    let served = *served.lock().unwrap_or_else(|p| p.into_inner());
    if let (Some(call), Some(how)) = (selection.jev.as_mut(), served) {
        call.served(how);
    }
    selection
}

/// [`select_connectors`] with Jev's scoring given: `score` gets the text
/// Jev is sent (`query.request`, never `query.local`) and the items to send
/// (never more than [`jev::MAX_CANDIDATES`]), and answers like
/// [`jev::score_each`].
pub(crate) async fn select_connectors_via<'q, F, Fut>(
    query: &'q ConnectorQuery,
    candidates: &[ConnectorCandidate<'_>],
    score: F,
) -> ConnectorSelection
where
    F: FnOnce(&'q str, Vec<Candidate>) -> Fut,
    Fut: std::future::Future<Output = Option<Result<Vec<Option<f64>>, String>>>,
{
    let request = query.local.as_str();
    if candidates.is_empty() {
        return ConnectorSelection {
            engine: "local".into(),
            ..Default::default()
        };
    }
    let local: Vec<bool> = candidates
        .iter()
        .map(|c| locally_relevant(request, c.id, c.name, c.description))
        .collect();
    let decide = |i: usize, attach: bool, basis: Basis, score, reason| ConnectorDecision {
        id: candidates[i].id.to_string(),
        attach,
        basis,
        score,
        reason,
    };
    let mut decisions: Vec<Option<ConnectorDecision>> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| named(request, c).then(|| decide(i, true, Basis::Named, None, None)))
        .collect();
    // Local preselection: locally relevant first, then library order.
    let mut open: Vec<usize> = (0..candidates.len())
        .filter(|&i| decisions[i].is_none())
        .collect();
    open.sort_by_key(|&i| !local[i]);
    let sent: Vec<usize> = open.iter().copied().take(jev::MAX_CANDIDATES).collect();
    let beyond: Vec<usize> = open.iter().copied().skip(jev::MAX_CANDIDATES).collect();
    let (outcome, jev) = if sent.is_empty() {
        (None, None)
    } else {
        let items = sent
            .iter()
            .map(|&i| Candidate {
                title: candidates[i].name.to_string(),
                excerpt: candidates[i].description.to_string(),
            })
            .collect();
        let started = Instant::now();
        let outcome = score(&query.request, items).await;
        let latency = crate::evidence::ms_since(started);
        let jev = JevUse::of("connectors", outcome.as_ref(), sent.len(), latency).sent_at(started);
        (outcome, Some(jev))
    };
    let (engine, note) = match outcome {
        None => {
            for &i in &open {
                decisions[i] = Some(decide(i, local[i], Basis::Local, None, None));
            }
            ("local", None)
        }
        Some(Err(e)) => {
            for &i in &sent {
                decisions[i] = Some(decide(
                    i,
                    local[i],
                    Basis::LocalFallback,
                    None,
                    Some(e.clone()),
                ));
            }
            for &i in &beyond {
                decisions[i] = Some(decide(
                    i,
                    local[i],
                    Basis::BeyondLimit,
                    None,
                    Some(beyond_reason()),
                ));
            }
            ("jev-fallback", Some(e))
        }
        Some(Ok(scores)) => {
            let mut unscored = 0;
            for (k, &i) in sent.iter().enumerate() {
                decisions[i] = Some(match scores.get(k).copied().flatten() {
                    Some(s) => decide(i, s >= TOOL_ATTACH, Basis::Jev, Some(s), None),
                    None => {
                        unscored += 1;
                        decide(
                            i,
                            local[i],
                            Basis::Unscored,
                            None,
                            Some("Jev's answer had no valid score for this connector.".into()),
                        )
                    }
                });
            }
            for &i in &beyond {
                decisions[i] = Some(decide(
                    i,
                    local[i],
                    Basis::BeyondLimit,
                    None,
                    Some(beyond_reason()),
                ));
            }
            let mut parts = Vec::new();
            if unscored > 0 {
                parts.push(format!(
                    "Jev scored {} of {} connectors; the rest used local relevance.",
                    sent.len() - unscored,
                    sent.len()
                ));
            }
            if !beyond.is_empty() {
                parts.push(format!(
                    "{} connector(s) past Jev's {} used local relevance.",
                    beyond.len(),
                    jev::MAX_CANDIDATES
                ));
            }
            ("jev", (!parts.is_empty()).then(|| parts.join(" ")))
        }
    };
    ConnectorSelection {
        engine: engine.into(),
        decisions: decisions
            .into_iter()
            .enumerate()
            .map(|(i, d)| d.unwrap_or_else(|| decide(i, local[i], Basis::Local, None, None)))
            .collect(),
        note,
        jev,
    }
}

/// Connector selection for a turn that keeps its conversation's last
/// decision (overnight OB18, OB25: a check-in with nothing new, a resumed
/// follow-up that adds nothing): each candidate `kept` decided keeps that
/// decision; any other (added since, or no earlier manifest) is decided
/// locally from `query`, as with Jev off. Jev is never called.
pub(crate) async fn keep_connectors(
    query: &ConnectorQuery,
    candidates: &[ConnectorCandidate<'_>],
    kept: Option<&ConnectorSelection>,
) -> ConnectorSelection {
    // A scorer that never answers: Jev off, so nothing is sent.
    let local = select_connectors_via(query, candidates, |_, _| async { None });
    let mut selection = local.await;
    selection.jev = None;
    let Some(kept) = kept else {
        return selection;
    };
    for decision in &mut selection.decisions {
        // A connector this turn's own words name is attached, whatever
        // the last turn decided.
        if decision.basis == Basis::Named {
            continue;
        }
        if let Some(last) = kept.decisions.iter().find(|d| d.id == decision.id) {
            *decision = last.clone();
        }
    }
    selection
}

// ── A conversation's last relevance decision (S28) ──

/// How long a conversation's Jev relevance decision is reused.
pub(crate) const SESSION_TTL: Duration = Duration::from_secs(30 * 60);
/// Conversations whose decision is kept at most (the oldest goes first).
const SESSION_ENTRIES: usize = 256;

/// One conversation's last Jev relevance decision.
#[derive(Debug)]
struct Remembered {
    session: String,
    at: Instant,
    /// [`inventory`] of the candidates it was made for.
    inventory: [u8; 32],
    /// Per connector id Jev decided: attach, and its score. Only relevance:
    /// never whether a connector runs without a prompt.
    relevance: Vec<(String, bool, Option<f64>)>,
}

/// Each conversation's last Jev relevance decision (S28), so consecutive
/// turns that change nothing about which connectors matter don't ask Jev
/// again (89% of consecutive turns decided the same). In this process's
/// memory only, at most `entries` conversations, each for at most `ttl`
/// from when Jev decided; lost on restart, which only costs a call.
#[derive(Debug)]
pub(crate) struct SessionCache {
    ttl: Duration,
    entries: usize,
    kept: std::sync::Mutex<Vec<Remembered>>,
}

impl SessionCache {
    pub(crate) const fn new(ttl: Duration, entries: usize) -> Self {
        SessionCache {
            ttl,
            entries,
            kept: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn kept(&self) -> std::sync::MutexGuard<'_, Vec<Remembered>> {
        self.kept.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// `session`'s last decision as this turn's, or None (ask as usual)
    /// when there is none within the TTL for this `inventory`, when it
    /// doesn't cover every candidate, when the request names a candidate,
    /// or when it shares a word with one the decision left out.
    fn reuse(
        &self,
        session: &str,
        inventory: &[u8; 32],
        query: &ConnectorQuery,
        candidates: &[ConnectorCandidate<'_>],
        now: Instant,
    ) -> Option<ConnectorSelection> {
        let request = query.local.as_str();
        let mut kept = self.kept();
        kept.retain(|r| now.saturating_duration_since(r.at) < self.ttl);
        let last = kept
            .iter()
            .find(|r| r.session == session && r.inventory == *inventory)?;
        let mut decisions = Vec::with_capacity(candidates.len());
        for c in candidates {
            if named(request, c) {
                return None;
            }
            let (_, attach, score) = last.relevance.iter().find(|(id, _, _)| id == c.id)?;
            if !attach && shares_a_word(request, c) {
                return None;
            }
            decisions.push(ConnectorDecision {
                id: c.id.to_string(),
                attach: *attach,
                basis: Basis::Cached,
                score: *score,
                reason: None,
            });
        }
        // On the manifest as a reused ranking: nothing sent.
        let mut call = JevUse::of::<()>("connectors", Some(&Ok(())), decisions.len(), 0);
        call.served(jev::Served::Cached);
        Some(ConnectorSelection {
            engine: "cached".into(),
            decisions,
            note: None,
            jev: Some(call),
        })
    }

    /// Keep what Jev decided in `selection` as `session`'s last decision
    /// (replacing the one before). A selection Jev decided nothing in (off,
    /// failed, all named) keeps nothing, and nor does one `jev::Guard`
    /// served from an identical ranking is adopted only with its original
    /// decision clock, so warming never stretches the conversation TTL.
    fn remember(
        &self,
        session: &str,
        inventory: [u8; 32],
        selection: &ConnectorSelection,
        now: Instant,
    ) {
        let served = selection.jev.as_ref().map(|call| call.outcome.as_str());
        let decided = if served == Some("cached") {
            let Some(at) = selection.jev.as_ref().and_then(|call| call.decision_at.0) else {
                self.kept().retain(|r| r.session != session);
                return;
            };
            at
        } else {
            now
        };
        if now.saturating_duration_since(decided) >= self.ttl {
            return;
        }
        let relevance: Vec<(String, bool, Option<f64>)> = selection
            .decisions
            .iter()
            .filter(|d| d.basis == Basis::Jev)
            .map(|d| (d.id.clone(), d.attach, d.score))
            .collect();
        if relevance.is_empty() {
            return;
        }
        let mut kept = self.kept();
        kept.retain(|r| now.saturating_duration_since(r.at) < self.ttl);
        kept.retain(|r| r.session != session);
        while kept.len() >= self.entries.max(1) {
            kept.remove(0);
        }
        kept.push(Remembered {
            session: session.to_string(),
            at: decided,
            inventory,
            relevance,
        });
    }
}

/// The app's (one per process).
pub(crate) static SESSIONS: SessionCache = SessionCache::new(SESSION_TTL, SESSION_ENTRIES);

/// Speculative work may inspect reuse eligibility but never adopt a new
/// decision for a turn that has not consumed it.
pub(crate) fn reuses_connectors(
    session: &str,
    query: &ConnectorQuery,
    candidates: &[ConnectorCandidate<'_>],
    jev: &jev::Target<'_>,
) -> bool {
    jev.key.is_some()
        && jev.is_current()
        && SESSIONS
            .reuse(
                session,
                &target_inventory(jev, candidates),
                query,
                candidates,
                Instant::now(),
            )
            .is_some()
}

fn target_inventory(jev: &jev::Target<'_>, candidates: &[ConnectorCandidate<'_>]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(inventory(&jev.endpoint, candidates));
    hash.update(jev.generation.to_le_bytes());
    hash.finalize().into()
}

/// What a relevance decision was made for: the Jev endpoint and every
/// candidate's id, name and description, in order. A connector added,
/// removed, renamed or redescribed is a new inventory. Approvals are not
/// part of it: they never decide relevance.
fn inventory(endpoint: &str, candidates: &[ConnectorCandidate<'_>]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(endpoint.as_bytes());
    for c in candidates {
        for part in [c.id, c.name, c.description] {
            hash.update([0]);
            hash.update(part.as_bytes());
        }
    }
    hash.finalize().into()
}

/// The [`SessionCache`] key for conversation `session` of the vault at
/// `vault`: both, since a copied vault keeps its session keys. Empty (no
/// reuse) when `session` is.
pub(crate) fn session_key(vault: &std::path::Path, session: &str) -> String {
    if session.is_empty() {
        return String::new();
    }
    format!("{}\n{session}", vault.display())
}

/// Whether `request` shares a meaningful word with `c` ([`locally_relevant`]),
/// a trailing `s` stripped on both sides ("emails" shares "email"): for
/// [`SessionCache::reuse`], where a near miss only costs a Jev call.
fn shares_a_word(request: &str, c: &ConnectorCandidate<'_>) -> bool {
    let stems = |text: &str| -> BTreeSet<String> {
        terms(text)
            .iter()
            .map(|w| w.strip_suffix('s').unwrap_or(w.as_str()).to_string())
            .collect()
    };
    let id = c.id.replace(['-', '_'], " ");
    let offered = format!("{} {} {id}", c.name, c.description);
    let wanted = stems(request);
    locally_relevant(request, c.id, c.name, c.description)
        || wanted.intersection(&stems(&offered)).next().is_some()
}

/// [`select_connectors`] for a chat turn of conversation `session` (S28):
/// its last Jev relevance decision is reused, with no call and basis
/// [`Basis::Cached`], while it is under [`SESSION_TTL`] old, the inventory
/// is unchanged, the request names no candidate and shares no word with one
/// it left out ([`SessionCache::reuse`]); otherwise Jev is asked as usual
/// and what it decides becomes the conversation's last decision. With no
/// session, Jev off or no candidates, exactly [`select_connectors`].
pub(crate) async fn select_connectors_in_session(
    session: Option<(&str, &SessionCache)>,
    query: &ConnectorQuery,
    candidates: &[ConnectorCandidate<'_>],
    deadline: &crate::prep_deadline::PrepDeadline,
    jev: &jev::Target<'_>,
) -> ConnectorSelection {
    let usable = jev.key.is_some() && jev.is_current() && !candidates.is_empty();
    let Some((session, cache)) = session.filter(|(key, _)| usable && !key.is_empty()) else {
        return select_connectors(query, candidates, deadline, jev).await;
    };
    let inventory = target_inventory(jev, candidates);
    if let Some(reused) = cache.reuse(session, &inventory, query, candidates, Instant::now()) {
        return reused;
    }
    let selection = select_connectors(query, candidates, deadline, jev).await;
    cache.remember(session, inventory, &selection, Instant::now());
    selection
}

fn beyond_reason() -> String {
    format!(
        "Jev scores at most {} connectors per request; this one was not among those sent.",
        jev::MAX_CANDIDATES
    )
}

fn named(request: &str, c: &ConnectorCandidate<'_>) -> bool {
    let r = request.to_lowercase();
    (!c.id.is_empty() && r.contains(&c.id.to_lowercase()))
        || (!c.name.trim().is_empty() && r.contains(&c.name.trim().to_lowercase()))
}

/// One retrieved passage's decision, in the order passages came in.
#[derive(Clone, Debug, PartialEq)]
pub struct PassageDecision {
    pub kept: bool,
    /// Jev's score, when it answered.
    pub score: Option<f64>,
    /// `local`, `jev` or `jev-fallback`.
    pub basis: &'static str,
    /// Why it was dropped, or why Jev's answer wasn't used.
    pub reason: Option<String>,
}

/// What [`prioritize_passages_via`] decided for one request.
pub struct Prioritized {
    /// The passages to send: unchanged, or Jev's order without the dropped.
    pub passages: Vec<Passage>,
    /// The coverage note when Jev left some out.
    pub note: Option<String>,
    /// `local`, `jev` or `jev-fallback`.
    pub engine: &'static str,
    /// One per passage given, in that order.
    pub decisions: Vec<PassageDecision>,
    /// The Jev call; `None` when there was nothing to rank (under two).
    pub jev: Option<JevUse>,
}

/// Reorder and trim retrieved passages when Jev is on; unchanged otherwise.
/// `score` is Jev's passage scoring (answers like [`jev::score`]:
/// `directory_connections::prioritized_context` passes the real one).
pub(crate) async fn prioritize_passages_via<F, Fut>(passages: Vec<Passage>, score: F) -> Prioritized
where
    F: FnOnce(Vec<Candidate>) -> Fut,
    Fut: std::future::Future<Output = Option<Result<Vec<f64>, String>>>,
{
    let total = passages.len();
    let local = |basis: &'static str, reason: Option<String>| PassageDecision {
        kept: true,
        score: None,
        basis,
        reason,
    };
    if total < 2 {
        return Prioritized {
            passages,
            note: None,
            engine: "local",
            decisions: (0..total).map(|_| local("local", None)).collect(),
            jev: None,
        };
    }
    let items: Vec<Candidate> = passages
        .iter()
        .map(|p| Candidate {
            title: p.path.rsplit('/').next().unwrap_or(&p.path).to_string(),
            excerpt: p.excerpt.clone(),
        })
        .collect();
    let started = Instant::now();
    let result = score(items).await;
    let jev = Some(
        JevUse::of(
            "passages",
            result.as_ref(),
            total,
            crate::evidence::ms_since(started),
        )
        .sent_at(started),
    );
    match result {
        None => Prioritized {
            passages,
            note: None,
            engine: "local",
            decisions: (0..total).map(|_| local("local", None)).collect(),
            jev,
        },
        Some(Err(e)) => Prioritized {
            passages,
            note: None,
            engine: "jev-fallback",
            decisions: (0..total)
                .map(|_| local("jev-fallback", Some(e.clone())))
                .collect(),
            jev,
        },
        Some(Ok(scores)) => {
            let decisions = (0..total)
                .map(|i| {
                    let s = scores.get(i).copied().unwrap_or(0.0);
                    let kept = s >= EVIDENCE_KEEP;
                    PassageDecision {
                        kept,
                        score: Some(s),
                        basis: "jev",
                        reason: (!kept)
                            .then(|| format!("Jev rated it unrelated ({s} of 3, below 1).")),
                    }
                })
                .collect();
            let kept = apply_scores(passages, &scores);
            let dropped = total - kept.len();
            let note = (dropped > 0).then(|| format!("{dropped} lower-relevance passage(s) were left out by the context prioritizer."));
            Prioritized {
                passages: kept,
                note,
                engine: "jev",
                decisions,
                jev,
            }
        }
    }
}

/// Drop passages scored below [`EVIDENCE_KEEP`] and order the rest by score,
/// keeping the local order among equal scores. Never adds or edits one.
pub(crate) fn apply_scores(passages: Vec<Passage>, scores: &[f64]) -> Vec<Passage> {
    let mut ranked: Vec<(Passage, f64)> = passages
        .into_iter()
        .zip(scores.iter().copied().chain(std::iter::repeat(0.0)))
        .filter(|(_, s)| *s >= EVIDENCE_KEEP)
        .collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked.into_iter().map(|(p, _)| p).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(path: &str) -> Passage {
        Passage {
            path: path.into(),
            line: 1,
            excerpt: "x".into(),
            source: "vault".into(),
            coverage: "complete".into(),
            score: 1,
            semantic: None,
            lifecycle: None,
            note: None,
            current: None,
        }
    }

    use crate::jev::fake::{serve, Reply};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    /// `n` connectors `conn-1…`, named `Zeta1x…`: no two names overlap and
    /// no description shares a word with the requests below.
    fn library(n: usize) -> Vec<(String, String)> {
        (1..=n)
            .map(|i| (format!("conn-{i}"), format!("Zeta{i}x")))
            .collect()
    }

    fn candidates(owned: &[(String, String)]) -> Vec<ConnectorCandidate<'_>> {
        owned
            .iter()
            .map(|(id, name)| ConnectorCandidate {
                id,
                name,
                description: "",
            })
            .collect()
    }

    /// `select_connectors` with Jev at `url` (the loopback fake); also the
    /// titles it sent.
    async fn via(
        request: &str,
        list: &[ConnectorCandidate<'_>],
        url: &str,
    ) -> (ConnectorSelection, Vec<String>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = url.to_string();
        let query = ConnectorQuery::plain(request);
        let selection = select_connectors_via(&query, list, |text, items| async move {
            *log.lock().unwrap() = items.iter().map(|c| c.title.clone()).collect();
            Some(crate::jev::score_each_with("k", &url, text, &items, Question::Tool).await)
        })
        .await;
        let sent = sent.lock().unwrap().clone();
        (selection, sent)
    }

    fn ids(selection: &ConnectorSelection) -> Vec<&str> {
        selection.decisions.iter().map(|d| d.id.as_str()).collect()
    }

    #[tokio::test]
    async fn exactly_one_decision_per_candidate_at_every_size() {
        for n in [0, 1, 24, 25, 50] {
            let owned = library(n);
            let list = candidates(&owned);
            let jev = serve(Reply::ScoreAll(0.0));
            let (selection, sent) = via("summarise the notes", &list, &jev.url).await;
            assert_eq!(selection.decisions.len(), n, "{n} candidates");
            let want: Vec<&str> = owned.iter().map(|(id, _)| id.as_str()).collect();
            assert_eq!(ids(&selection), want, "{n}: in the candidates' order");
            assert_eq!(sent.len(), n.min(crate::jev::MAX_CANDIDATES), "{n}");
            assert_eq!(
                jev.seen.lock().unwrap().clone(),
                if n == 0 { vec![] } else { vec![n.min(24)] },
                "{n}: one bounded request, never more than 24"
            );
            for (i, d) in selection.decisions.iter().enumerate() {
                if i < 24 {
                    assert_eq!((d.basis, d.score, d.attach), (Basis::Jev, Some(0.0), false));
                } else {
                    assert_eq!((d.basis, d.attach), (Basis::BeyondLimit, false), "{n}/{i}");
                    assert!(d.reason.as_deref().unwrap().contains("at most 24"));
                }
            }
            // Jev off: the same cardinality, all local.
            let off = select_connectors_via(
                &ConnectorQuery::plain("summarise the notes"),
                &list,
                |_, _| async { None },
            )
            .await;
            assert_eq!(off.decisions.len(), n);
            assert!(off
                .decisions
                .iter()
                .all(|d| d.basis == Basis::Local && !d.attach));
        }
    }

    #[tokio::test]
    async fn a_named_connector_is_kept_wherever_it_sits_and_never_sent() {
        let owned = library(50);
        let list = candidates(&owned);
        for position in [1, 24, 25, 50] {
            let request = format!("please use Zeta{position}x for this");
            let jev = serve(Reply::ScoreAll(0.0));
            let (selection, sent) = via(&request, &list, &jev.url).await;
            assert_eq!(selection.decisions.len(), 50);
            let d = &selection.decisions[position - 1];
            assert_eq!(d.id, format!("conn-{position}"));
            assert_eq!(
                (d.attach, d.basis),
                (true, Basis::Named),
                "position {position}"
            );
            assert!(
                !sent.contains(&format!("Zeta{position}x")),
                "decided before Jev"
            );
            assert_eq!(sent.len(), 24, "49 open, 24 sent");
            assert_eq!(selection.attached().collect::<Vec<_>>(), [d.id.as_str()]);
            // Also by id.
            let by_id = format!("attach conn-{position} now");
            let (selection, _) = via(&by_id, &list, &jev.url).await;
            assert!(selection.decisions[position - 1].attach);
        }
    }

    /// Overnight OB18, OB25: a turn that skips retrieval keeps its
    /// conversation's last decision for each connector that decision
    /// covered, decides the rest locally, and sends Jev nothing (no call on
    /// its manifest).
    #[tokio::test]
    async fn a_kept_decision_is_reused_with_no_jev_call() {
        let owned = library(3);
        let list = candidates(&owned);
        let jev = serve(Reply::ScoreAll(2.0));
        let (last, _) = via("summarise the notes", &list[..2], &jev.url).await;
        assert_eq!(last.attached().collect::<Vec<_>>(), ["conn-1", "conn-2"]);
        let sent = jev.seen.lock().unwrap().len();
        let query = ConnectorQuery::plain("yes, merge it");
        let kept = keep_connectors(&query, &list, Some(&last)).await;
        assert_eq!(jev.seen.lock().unwrap().len(), sent, "nothing sent");
        assert!(kept.jev.is_none());
        assert_eq!(ids(&kept), ["conn-1", "conn-2", "conn-3"]);
        assert_eq!(&kept.decisions[..2], &last.decisions[..]);
        let third = &kept.decisions[2];
        assert_eq!((third.basis, third.attach), (Basis::Local, false));
        let none = keep_connectors(&query, &list, None).await;
        assert!(none.jev.is_none());
        assert!(none
            .decisions
            .iter()
            .all(|d| d.basis == Basis::Local && !d.attach));
        // R1 review (S1): a connector this turn names is attached, even
        // when the last turn left it out.
        let (dropped, _) = via("summarise the notes", &list, &jev.url).await;
        let dropped = ConnectorSelection {
            decisions: dropped
                .decisions
                .into_iter()
                .map(|mut d| {
                    d.attach = false;
                    d
                })
                .collect(),
            ..ConnectorSelection::default()
        };
        let named = ConnectorQuery::plain("yes, use Zeta2x");
        let kept = keep_connectors(&named, &list, Some(&dropped)).await;
        assert_eq!(kept.attached().collect::<Vec<_>>(), ["conn-2"]);
        assert_eq!(kept.decisions[1].basis, Basis::Named);
        assert!(kept.jev.is_none());
    }

    /// S28: [`select_connectors_in_session`] for conversation `session`, with
    /// Jev at `url` behind a fresh guard (so its identical-request reuse
    /// never hides a call: every call counted here is the session cache's).
    async fn in_session(
        session: &str,
        request: &str,
        list: &[ConnectorCandidate<'_>],
        url: &str,
        sessions: &SessionCache,
    ) -> ConnectorSelection {
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let target = crate::jev::Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("k".into()),
            endpoint: url.to_string(),
        };
        let deadline = crate::prep_deadline::PrepDeadline::with_budget(Duration::from_secs(30));
        select_connectors_in_session(
            Some((session, sessions)),
            &ConnectorQuery::plain(request),
            list,
            &deadline,
            &target,
        )
        .await
    }

    /// S28: two turns of one conversation with the same inventory and
    /// unrelated requests make one Jev call; the second reuses the first's
    /// decision (basis `cached`, a `cached` use on its manifest). Another
    /// conversation, a changed inventory or an expired decision asks again.
    #[tokio::test]
    async fn unrelated_turns_of_one_conversation_make_one_jev_call() {
        let owned = library(3);
        let list = candidates(&owned);
        let scores = vec![Some(json!(3)), Some(json!(0)), Some(json!(2))];
        let jev = serve(Reply::Scores(scores));
        let seen = || jev.seen.lock().unwrap().len();
        let sessions = SessionCache::new(SESSION_TTL, 8);
        let first = in_session("s1", "summarise the notes", &list, &jev.url, &sessions).await;
        assert_eq!(first.engine, "jev");
        assert_eq!(first.attached().collect::<Vec<_>>(), ["conn-1", "conn-3"]);
        let second = in_session("s1", "what about tomorrow", &list, &jev.url, &sessions).await;
        assert_eq!(seen(), 1, "one Jev call for two turns");
        assert_eq!(second.engine, "cached");
        assert!(second.decisions.iter().all(|d| d.basis == Basis::Cached));
        assert_eq!(ids(&second), ["conn-1", "conn-2", "conn-3"]);
        assert_eq!(second.attached().collect::<Vec<_>>(), ["conn-1", "conn-3"]);
        assert_eq!(second.decisions[2].score, Some(2.0));
        let call = second.jev.as_ref().expect("a use on the manifest");
        assert_eq!(
            (call.outcome.as_str(), call.items, call.latency_ms),
            ("cached", 3, None)
        );
        // Only ids and relevance are kept: no request text.
        assert!(!format!("{:?}", *sessions.kept()).contains("notes"));
        // Another conversation asks.
        in_session("s2", "what about tomorrow", &list, &jev.url, &sessions).await;
        assert_eq!(seen(), 2);
        // A connector added since: a new inventory, asked again.
        let more = library(4);
        let grown = candidates(&more);
        let asked = in_session("s1", "what about tomorrow", &grown, &jev.url, &sessions).await;
        assert_eq!((seen(), asked.engine.as_str()), (3, "jev"));
        // Expired: asked again.
        let quick = SessionCache::new(Duration::from_millis(100), 8);
        in_session("s1", "summarise the notes", &list, &jev.url, &quick).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let late = in_session("s1", "what about tomorrow", &list, &jev.url, &quick).await;
        assert_eq!((seen(), late.engine.as_str()), (5, "jev"));
    }

    /// S28: a request that names a connector (Gmail) asks Jev again, and so
    /// does one sharing a word with a connector the last decision left out.
    #[tokio::test]
    async fn naming_a_connector_or_a_left_out_ones_word_asks_jev_again() {
        let list = [
            ConnectorCandidate {
                id: "gmail",
                name: "Gmail",
                description: "Read and send email",
            },
            ConnectorCandidate {
                id: "conn-2",
                name: "Zeta2x",
                description: "",
            },
        ];
        let jev = serve(Reply::ScoreAll(0.0));
        let seen = || jev.seen.lock().unwrap().len();
        let sessions = SessionCache::new(SESSION_TTL, 8);
        in_session("s1", "summarise the notes", &list, &jev.url, &sessions).await;
        assert_eq!(seen(), 1);
        let named = in_session("s1", "draft this in Gmail", &list, &jev.url, &sessions).await;
        assert_eq!(seen(), 2, "a named connector asks Jev again");
        assert_eq!(
            (named.decisions[0].basis, named.decisions[0].attach),
            (Basis::Named, true)
        );
        // Gmail's last decision wasn't Jev's: the next turn asks, the one
        // after reuses.
        in_session("s1", "what about tomorrow", &list, &jev.url, &sessions).await;
        assert_eq!(seen(), 3);
        let reused = in_session("s1", "and next week", &list, &jev.url, &sessions).await;
        assert_eq!((seen(), reused.engine.as_str()), (3, "cached"));
        // "email" is in left-out Gmail's description: asked.
        let shared = in_session("s1", "check my email", &list, &jev.url, &sessions).await;
        assert_eq!((seen(), shared.engine.as_str()), (4, "jev"));
        // A plural is the same word: "emails" shares left-out Gmail's
        // "email" (a trailing `s` stripped on both sides).
        let plural = in_session("s1", "sort my emails", &list, &jev.url, &sessions).await;
        assert_eq!((seen(), plural.engine.as_str()), (5, "jev"));
        let gmail = &list[0];
        let plain = locally_relevant("sort my emails", gmail.id, gmail.name, gmail.description);
        assert!(!plain, "plain overlap misses the plural");
        assert!(shares_a_word("sort my emails", &list[0]));
        assert!(!shares_a_word("sort my notes", &list[0]));
    }

    /// A warmed score becomes a conversation decision at its original
    /// timestamp, so preparation preserves reuse without extending its TTL.
    #[tokio::test]
    async fn a_warmed_ranking_keeps_conversation_reuse_at_the_original_time() {
        let owned = library(2);
        let list = candidates(&owned);
        let jev = serve(Reply::ScoreAll(3.0));
        let seen = || jev.seen.lock().unwrap().len();
        let sessions = SessionCache::new(SESSION_TTL, 8);
        // One guard for every turn, so an identical request is reused.
        let guard = crate::jev::Guard::new(crate::jev::Tuning::APP);
        let target = crate::jev::Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("k".into()),
            endpoint: jev.url.clone(),
        };
        let deadline = crate::prep_deadline::PrepDeadline::with_budget(Duration::from_secs(30));
        let first = ConnectorQuery::plain("summarise the notes");
        let later = ConnectorQuery::plain("what about tomorrow");
        let s1 = Some(("s1", &sessions));
        let s2 = Some(("s2", &sessions));
        let one = select_connectors_in_session(s1, &first, &list, &deadline, &target).await;
        assert_eq!((seen(), one.engine.as_str()), (1, "jev"));
        // Another conversation, the identical request: the guard's ranking.
        let two = select_connectors_in_session(s2, &first, &list, &deadline, &target).await;
        assert_eq!(seen(), 1);
        assert_eq!(two.jev.as_ref().map(|c| c.outcome.as_str()), Some("cached"));
        let times = sessions.kept();
        let first_at = times.iter().find(|r| r.session == "s1").unwrap().at;
        let second_at = times.iter().find(|r| r.session == "s2").unwrap().at;
        assert!(
            second_at <= first_at,
            "adoption cannot renew the score clock"
        );
        drop(times);
        // The adopted score is now the conversation's prior decision.
        let next = select_connectors_in_session(s2, &later, &list, &deadline, &target).await;
        assert_eq!((seen(), next.engine.as_str()), (1, "cached"));
        // s1's own decision, which Jev made, is still reused.
        let reused = select_connectors_in_session(s1, &later, &list, &deadline, &target).await;
        assert_eq!((seen(), reused.engine.as_str()), (1, "cached"));
    }

    /// S28 review: the cache is keyed on the vault as well as the
    /// conversation, so a copied vault's same session key asks afresh; an
    /// empty session key never reuses.
    #[tokio::test]
    async fn one_session_key_in_two_vaults_is_two_conversations() {
        let a = session_key(std::path::Path::new("/vaults/a"), "s1");
        let b = session_key(std::path::Path::new("/vaults/copy-of-a"), "s1");
        assert_ne!(a, b);
        assert_eq!(session_key(std::path::Path::new("/vaults/a"), ""), "");
        let owned = library(2);
        let list = candidates(&owned);
        let jev = serve(Reply::ScoreAll(3.0));
        let seen = || jev.seen.lock().unwrap().len();
        let sessions = SessionCache::new(SESSION_TTL, 8);
        in_session(&a, "summarise the notes", &list, &jev.url, &sessions).await;
        let copy = in_session(&b, "what about tomorrow", &list, &jev.url, &sessions).await;
        assert_eq!((seen(), copy.engine.as_str()), (2, "jev"));
        let own = in_session(&a, "and next week", &list, &jev.url, &sessions).await;
        assert_eq!((seen(), own.engine.as_str()), (2, "cached"));
    }

    #[tokio::test]
    async fn no_failure_or_odd_answer_drops_a_named_connector() {
        let owned = library(50);
        let list = candidates(&owned);
        let oversized = format!("{{\"answers\":{{}},\"pad\":\"{}\"}}", "x".repeat(70 * 1024));
        let many: Vec<Option<serde_json::Value>> = (0..40).map(|_| Some(json!(3.0))).collect();
        let cases: Vec<(&str, Reply)> = vec![
            ("malformed JSON", Reply::Raw("{not json".into())),
            ("no answers", Reply::Raw(json!({"result": "ok"}).to_string())),
            ("server error", Reply::Status("500 Internal Server Error")),
            ("oversized", Reply::Raw(oversized)),
            (
                "wrong ids",
                Reply::Raw(json!({"answers": {"candidate_x": {"type": "score", "score": 3}, "c0": {"type": "score", "score": 3}}}).to_string()),
            ),
            (
                "partial",
                Reply::Scores(vec![Some(json!(3.0)), Some(json!("NaN")), Some(json!(7)), None]),
            ),
            ("over-long", Reply::Scores(many)),
        ];
        for (what, reply) in cases {
            for position in [1, 24, 25, 50] {
                let request = format!("please use Zeta{position}x for this");
                let jev = serve(reply.clone());
                let (selection, _) = via(&request, &list, &jev.url).await;
                assert_eq!(selection.decisions.len(), 50, "{what}");
                let d = &selection.decisions[position - 1];
                assert_eq!(
                    (d.attach, d.basis),
                    (true, Basis::Named),
                    "{what} at {position}"
                );
            }
        }
    }

    #[tokio::test]
    async fn a_timeout_keeps_the_named_25th_and_falls_back_with_the_reason() {
        let owned = library(50);
        let list = candidates(&owned);
        let jev = serve(Reply::Silent);
        let (selection, _) = via("please use Zeta25x for this", &list, &jev.url).await;
        assert_eq!(selection.engine, "jev-fallback");
        assert_eq!(selection.decisions.len(), 50);
        assert_eq!(selection.decisions[24].basis, Basis::Named);
        assert!(selection.decisions[24].attach);
        for (i, d) in selection
            .decisions
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 24)
        {
            // 49 open: the 24 sent fall back with the error; those past them
            // were never sent and say so.
            if i < 24 {
                assert_eq!(d.basis, Basis::LocalFallback, "{i}");
                // reqwest's own 4 s bound and the outer one race; either
                // way it reads as a timeout (F11: the manifest's outcome).
                assert_eq!(d.reason.as_deref(), Some("Jev timed out."), "{i}");
            } else {
                assert_eq!(d.basis, Basis::BeyondLimit, "{i}");
            }
            assert!(!d.attach);
        }
    }

    #[tokio::test]
    async fn partial_scores_fall_back_locally_for_each_unscored_one() {
        let owned: Vec<(String, String, String)> = vec![
            ("crm".into(), "Pipeline".into(), "Customer deals".into()),
            (
                "mbox".into(),
                "Mailbox".into(),
                "Read and send email".into(),
            ),
            ("cal".into(), "Diary".into(), "Meetings and events".into()),
            ("wiki".into(), "Handbook".into(), "Company policies".into()),
            ("pay".into(), "Payroll".into(), "Salary runs".into()),
        ];
        let list: Vec<ConnectorCandidate> = owned
            .iter()
            .map(|(id, name, description)| ConnectorCandidate {
                id,
                name,
                description,
            })
            .collect();
        // Locally, only `mbox` matches ("email").
        let jev = serve(Reply::Scores(vec![
            Some(json!(3.0)),
            None,
            Some(json!("2.5")),
            Some(json!(9)),
            Some(json!(1.0)),
        ]));
        let (selection, sent) = via("check my email about the offsite", &list, &jev.url).await;
        assert_eq!(sent[0], "Mailbox", "locally relevant ones are sent first");
        let by: Vec<(&str, bool, Basis)> = selection
            .decisions
            .iter()
            .map(|d| (d.id.as_str(), d.attach, d.basis))
            .collect();
        assert_eq!(
            by,
            [
                ("crm", false, Basis::Unscored),
                ("mbox", true, Basis::Jev),
                ("cal", false, Basis::Unscored),
                ("wiki", false, Basis::Unscored),
                ("pay", false, Basis::Jev),
            ]
        );
        assert_eq!(selection.engine, "jev");
        assert!(selection.decisions[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("no valid score"));
        // An unscored one that is locally relevant is attached locally.
        let jev = serve(Reply::Scores(vec![None, Some(json!(0.0))]));
        let (selection, _) = via("check my email about the offsite", &list, &jev.url).await;
        assert_eq!(
            (selection.decisions[1].attach, selection.decisions[1].basis),
            (true, Basis::Unscored)
        );
    }

    #[tokio::test]
    async fn preselection_sends_the_locally_relevant_first_and_the_rest_stay_local() {
        // 30 connectors; the last three share "invoice" with the request.
        let owned: Vec<(String, String, String)> = (1..=30)
            .map(|i| {
                let about = if i > 27 { "invoice tools" } else { "unrelated" };
                (format!("c{i}"), format!("Quux{i}y"), about.to_string())
            })
            .collect();
        let list: Vec<ConnectorCandidate> = owned
            .iter()
            .map(|(id, name, description)| ConnectorCandidate {
                id,
                name,
                description,
            })
            .collect();
        let jev = serve(Reply::ScoreAll(2.0));
        let (selection, sent) = via("find the invoice", &list, &jev.url).await;
        assert_eq!(&sent[..3], ["Quux28y", "Quux29y", "Quux30y"]);
        assert_eq!(sent.len(), 24);
        assert_eq!(selection.decisions.len(), 30);
        for d in &selection.decisions[..21] {
            assert_eq!((d.basis, d.attach), (Basis::Jev, true), "{}", d.id);
        }
        for d in &selection.decisions[21..27] {
            assert_eq!((d.basis, d.attach), (Basis::BeyondLimit, false), "{}", d.id);
        }
        for d in &selection.decisions[27..] {
            assert_eq!((d.basis, d.attach), (Basis::Jev, true), "{}", d.id);
        }
    }

    /// F11: two turns preparing at once (interleaved at every await) each
    /// keep their own record. Before, both wrote one process-global "last
    /// report", and the first turn's read back the second's passages
    /// (ae1-r3-red.log).
    #[tokio::test]
    async fn two_concurrent_turns_keep_their_own_manifests() {
        async fn turn(
            request: &'static str,
            paths: &'static [&'static str],
            owned: Vec<(String, String)>,
            jev: Option<Vec<f64>>,
        ) -> crate::run_manifest::RunManifest {
            let passages = paths.iter().map(|p| passage(p)).collect();
            let scores = jev.clone();
            let ranked = prioritize_passages_via(passages, |_| async move {
                tokio::task::yield_now().await;
                scores.map(Ok::<_, String>)
            })
            .await;
            tokio::task::yield_now().await;
            let list = candidates(&owned);
            let query = ConnectorQuery::plain(request);
            let selection = select_connectors_via(&query, &list, |_, items| async move {
                tokio::task::yield_now().await;
                jev.map(|_| Ok::<_, String>(vec![Some(0.0); items.len()]))
            })
            .await;
            let mut m = crate::run_manifest::RunManifest::new();
            m.set_retrieval(
                crate::run_manifest::RetrievalRecord {
                    engine: ranked.engine.into(),
                    candidates: ranked
                        .decisions
                        .iter()
                        .map(|d| crate::run_manifest::RetrievalCandidate {
                            kept: d.kept,
                            jev_score: d.score,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
                ranked.jev,
            );
            m.set_connectors(selection);
            m
        }
        let (alpha, beta) = tokio::join!(
            turn(
                "use Zeta1x for alpha",
                &["a1.md", "a2.md", "a3.md"],
                library(5),
                Some(vec![3.0, 0.0, 2.0]),
            ),
            turn("turn beta", &["b1.md"], library(2), None),
        );
        let retrieval = |m: &crate::run_manifest::RunManifest| {
            let r = m.retrieval.clone().unwrap();
            (
                r.engine,
                r.candidates.len(),
                r.candidates.iter().filter(|c| c.kept).count(),
            )
        };
        assert_eq!(retrieval(&alpha), ("jev".into(), 3, 2));
        assert_eq!(retrieval(&beta), ("local".into(), 1, 1));
        let connectors = |m: &crate::run_manifest::RunManifest| {
            let c = m.connectors.clone().unwrap();
            (
                c.engine.clone(),
                c.decisions.len(),
                c.attached().map(str::to_string).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            connectors(&alpha),
            ("jev".into(), 5, vec!["conn-1".to_string()])
        );
        assert_eq!(connectors(&beta), ("local".into(), 2, vec![]));
        let uses = |m: &crate::run_manifest::RunManifest| {
            m.jev
                .iter()
                .map(|j| (j.kind.clone(), j.outcome.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            uses(&alpha),
            [
                ("passages".into(), "ok".into()),
                ("connectors".into(), "ok".into())
            ]
        );
        // Beta had one passage (nothing to rank) and Jev off for connectors.
        assert_eq!(uses(&beta), [("connectors".to_string(), "off".to_string())]);
    }

    #[tokio::test]
    async fn passage_decisions_carry_scores_and_reasons() {
        let passages = vec![passage("a.md"), passage("b.md"), passage("c.md")];
        let ranked =
            prioritize_passages_via(passages, |_| async { Some(Ok(vec![0.5, 3.0, 1.0])) }).await;
        let order: Vec<&str> = ranked.passages.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(order, ["b.md", "c.md"]);
        assert_eq!(ranked.decisions.len(), 3);
        assert!(!ranked.decisions[0].kept);
        assert!(ranked.decisions[0]
            .reason
            .as_deref()
            .unwrap()
            .contains("unrelated"));
        assert_eq!(ranked.decisions[1].score, Some(3.0));
        assert!(ranked.note.unwrap().contains("1 lower-relevance"));
        let failed = prioritize_passages_via(vec![passage("a.md"), passage("b.md")], |_| async {
            Some(Err("Jev timed out.".to_string()))
        })
        .await;
        assert_eq!(failed.engine, "jev-fallback");
        assert!(failed.decisions.iter().all(|d| d.kept
            && d.basis == "jev-fallback"
            && d.reason.as_deref() == Some("Jev timed out.")));
        assert_eq!(failed.jev.unwrap().outcome, "timeout");
    }

    /// Agent effectiveness F8: each Jev call that was sent carries when it
    /// started and how long it took, and the manifest puts it on the run's
    /// preparation timeline (`connectorsJev`, `retrievalJev`); a use that
    /// sent nothing (Jev off) has no mark. Never serialized.
    #[tokio::test]
    async fn sent_jev_calls_carry_their_mark_onto_the_timeline() {
        use crate::prep_timing::Step;
        let owned = library(3);
        let list = candidates(&owned);
        let jev = serve(Reply::ScoreAll(0.0));
        let (mut selection, _) = via("summarise the notes", &list, &jev.url).await;
        let call = selection.jev.as_ref().expect("a connector call");
        let mark = call.mark.0.expect("a sent call has a mark");
        assert_eq!(Some(mark.ms), call.latency_ms);
        let json = serde_json::to_value(call).unwrap();
        assert!(json.get("mark").is_none(), "{json}");
        let mut manifest = crate::run_manifest::RunManifest::new();
        manifest.set_connectors(selection.clone());
        assert_eq!(manifest.timer.0.get(Step::ConnectorsJev), Some(mark));
        selection.jev = None;
        let off =
            select_connectors_via(&ConnectorQuery::plain("x"), &list, |_, _| async { None }).await;
        assert!(
            off.jev.as_ref().unwrap().mark.0.is_none(),
            "off sent nothing"
        );
        let mut quiet = crate::run_manifest::RunManifest::new();
        quiet.set_connectors(off);
        assert_eq!(quiet.timer.0.get(Step::ConnectorsJev), None);

        let ranked = prioritize_passages_via(vec![passage("a.md"), passage("b.md")], |_| async {
            Some(Ok(vec![2.0, 3.0]))
        })
        .await;
        let call = ranked.jev.expect("a passage call");
        let mark = call.mark.0.expect("sent");
        manifest.set_retrieval(Default::default(), Some(call));
        assert_eq!(manifest.timer.0.get(Step::RetrievalJev), Some(mark));
        let off =
            prioritize_passages_via(vec![passage("a.md"), passage("b.md")], |_| async { None })
                .await;
        assert!(off.jev.unwrap().mark.0.is_none());
    }

    /// Agent effectiveness F8 (O2): as a run selects, through one guard and
    /// under the preparation deadline. A reused ranking reads `cached` and
    /// keeps Jev's decisions; a stalled Jev is cut at the deadline and a
    /// paused one sends nothing, both falling back locally with the reason;
    /// the named connector is attached in every case.
    #[tokio::test]
    async fn as_a_run_selects_reuse_the_deadline_and_the_breaker_are_recorded() {
        use crate::jev::{Guard, Target, Tuning};
        use crate::prep_deadline::PrepDeadline;
        use std::time::Duration;
        let owned = library(30);
        let list = candidates(&owned);
        let query = ConnectorQuery::plain("please use Zeta30x for this");
        let far = PrepDeadline::with_budget(Duration::from_secs(30));
        let healthy = serve(Reply::ScoreAll(3.0));
        let guard = Guard::new(Tuning::APP);
        let at = Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("k".into()),
            endpoint: healthy.url.clone(),
        };
        let first = select_connectors(&query, &list, &far, &at).await;
        let second = select_connectors(&query, &list, &far, &at).await;
        let call = second.jev.clone().unwrap();
        assert_eq!(
            (call.outcome.as_str(), call.items, call.latency_ms),
            ("cached", 24, None)
        );
        assert!(call.mark.0.is_none(), "nothing sent: no timeline mark");
        assert_eq!(first.decisions, second.decisions, "the same decisions");
        assert_eq!(second.engine, "jev");
        assert_eq!(healthy.seen.lock().unwrap().len(), 1);

        let stalled = serve(Reply::Silent);
        let guard = Guard::new(Tuning::APP);
        let at = Target {
            guard: &guard,
            generation: guard.generation(),
            allowed: None,
            key: Some("k".into()),
            endpoint: stalled.url.clone(),
        };
        let deadline = PrepDeadline::with_budget(Duration::from_millis(300));
        let cut = select_connectors(&query, &list, &deadline, &at).await;
        let call = cut.jev.clone().unwrap();
        assert_eq!((call.outcome.as_str(), call.items), ("deadline", 24));
        assert!(call.latency_ms.unwrap() < 1500);
        assert_eq!(cut.engine, "jev-fallback");
        let named = &cut.decisions[29];
        assert_eq!((named.attach, named.basis), (true, Basis::Named));
        let sent = cut
            .decisions
            .iter()
            .filter(|d| d.basis == Basis::LocalFallback)
            .collect::<Vec<_>>();
        assert_eq!(sent.len(), 24);
        assert!(sent
            .iter()
            .all(|d| d.reason.as_deref() == Some(crate::prep_deadline::JEV_MISSED)));
        // Two more failures open the breaker; the next sends nothing.
        for _ in 0..2 {
            let soon = PrepDeadline::with_budget(Duration::from_millis(300));
            select_connectors(&query, &list, &soon, &at).await;
        }
        let paused = select_connectors(&query, &list, &far, &at).await;
        let call = paused.jev.clone().unwrap();
        assert_eq!(
            (call.outcome.as_str(), call.items, call.latency_ms),
            ("circuit-open", 0, None)
        );
        assert!(paused.decisions[29].attach, "the named one stays attached");
        assert!(paused.decisions[0]
            .reason
            .as_deref()
            .unwrap()
            .starts_with("Jev is paused"));
        assert_eq!(stalled.seen.lock().unwrap().len(), 3);
        let mut manifest = crate::run_manifest::RunManifest::new();
        manifest.set_connectors(paused);
        assert!(manifest.fallbacks[0].detail.contains("Jev is paused"));
        assert_eq!(
            manifest
                .timer
                .0
                .get(crate::prep_timing::Step::ConnectorsJev),
            None
        );
    }

    #[test]
    fn a_decision_serializes_for_the_run_manifest() {
        let named = ConnectorDecision {
            id: "crm".into(),
            attach: true,
            basis: Basis::Named,
            score: None,
            reason: None,
        };
        assert_eq!(
            serde_json::to_value(&named).unwrap(),
            json!({"id": "crm", "attach": true, "basis": "named"})
        );
        let beyond = ConnectorDecision {
            basis: Basis::BeyondLimit,
            reason: Some("r".into()),
            attach: false,
            ..named.clone()
        };
        assert_eq!(
            serde_json::to_value(&beyond).unwrap(),
            json!({"id": "crm", "attach": false, "basis": "beyond-limit", "reason": "r"})
        );
        for (basis, name) in [
            (Basis::Jev, "jev"),
            (Basis::Local, "local"),
            (Basis::LocalFallback, "local-fallback"),
            (Basis::Unscored, "unscored"),
        ] {
            assert_eq!(serde_json::to_value(basis).unwrap(), json!(name));
        }
    }

    #[test]
    fn local_relevance_matches_names_and_shared_words() {
        assert!(locally_relevant(
            "draft a reply in superhuman",
            "superhuman-mail",
            "Superhuman Mail",
            ""
        ));
        assert!(locally_relevant(
            "check my email for the invoice",
            "sh",
            "Mail",
            "Read and send email"
        ));
        assert!(!locally_relevant(
            "refactor the parser",
            "superhuman-mail",
            "Superhuman Mail",
            "Read and send email"
        ));
        // Stopwords and short words never match on their own.
        assert!(!locally_relevant(
            "can you do it",
            "x",
            "Tool",
            "can do it for you"
        ));
    }

    #[test]
    fn scores_drop_unrelated_and_keep_order_among_ties() {
        let kept = apply_scores(
            vec![passage("a"), passage("b"), passage("c"), passage("d")],
            &[2.0, 0.5, 3.0, 2.0],
        );
        let paths: Vec<_> = kept.iter().map(|p| p.path.as_str()).collect();
        assert_eq!(paths, ["c", "a", "d"]);
        // A missing score counts as unrelated.
        assert_eq!(
            apply_scores(vec![passage("a"), passage("b")], &[3.0]).len(),
            1
        );
    }
}
