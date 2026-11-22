//! The agent work outbox (X1): the tracker writes Hollow makes for tracked
//! agent work, journaled in the sync ledger (`.tracker-sync.json`, field
//! `work`) beside the edit outbox, and sent by the sync pass that already
//! holds the tracker lock, the process lock and the History lease.
//!
//! Lifted from the company activity journal (removed 11-10), without its roles:
//! every write is persisted as attempted before the provider call, and a
//! comment embeds its record id, so a lost reply or a crash between persist
//! and send is confirmed by listing the issue's comments (exactly one exact
//! body) and never sent twice. A status move is idempotent: an uncertain
//! one is confirmed by reading the issue back, and skipped when the issue
//! moved elsewhere (the tracker wins).
//!
//! Caps (X0 section 4): at most 4 sends a pass, 30 tracking comments an
//! hour across Hollow and 4 an issue (the rest wait), one progress comment
//! an issue every 15 minutes (notes in between fold into it), and a low rate
//! budget pauses tracking first. Writes go to the configured team only, and
//! only while Settings' "Track agent work" is on. Never Done from here, but
//! for T3b's close of the issue Hollow created for a Waiting on you item.

use super::work::{self, PendingEvent, WorkEvent};
use super::{IssuePatch, NewIssue, StateKind, TrackerAdapter, TrackerError, TrackerIssue};
use crate::config::Config;
use crate::tracker::ipc::TrackerSyncOutcome;
use crate::tracker_sync::{Create, Ledger};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_ENTRIES: usize = 2_000;
/// Sends (comments, moves, creates) in one pass.
pub(crate) const MAX_SENDS: usize = 4;
/// New facts taken from the sessions store in one pass.
pub(crate) const MAX_NEW: usize = 32;
pub(crate) const COMMENTS_PER_HOUR: usize = 30;
pub(crate) const COMMENTS_PER_ISSUE_HOUR: usize = 4;
/// An uncertain write is rechecked this many times, then let go (Skipped).
pub(crate) const MAX_RECHECKS: u32 = 3;
/// A send that provably didn't go out for a passing reason waits this long.
const RETRY_MINUTES: i64 = 5;
/// Settled entries keep only their record id after this many hours.
const SETTLED_BODY_HOURS: i64 = 24;
/// A failed create (attempted, no issue, an error) is let go after this.
const FAILED_CREATE_MINUTES: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OpKind {
    Comment,
    /// Backlog or unstarted to the team's first started state.
    MoveStarted,
    /// From Hollow's started state to the next started state (In Review).
    MoveReview,
    /// T3b: a Waiting on you item's own issue to the team's completed
    /// state, once its item is answered, dismissed or done. Never another.
    MoveDone,
    /// A write a later build journaled that this one can't read: kept,
    /// never sent, so a newer ledger still loads here.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Delivery {
    Pending,
    Unconfirmed,
    Confirmed,
    /// Settled without a confirmed write: the issue moved elsewhere (the
    /// tracker wins), it left the team, tracking went off, or an uncertain
    /// write stayed unconfirmed after its rechecks. `reason` says which.
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkOp {
    /// `work-<32 hex>`, embedded in a comment's body.
    pub id: String,
    pub session_key: String,
    pub team_id: String,
    pub issue_id: String,
    pub kind: OpKind,
    /// A comment's fixed body, record id included.
    #[serde(default)]
    pub body: Option<String>,
    /// A move's target state.
    #[serde(default)]
    pub state_id: Option<String>,
    #[serde(default)]
    pub progress: bool,
    pub delivery: Delivery,
    #[serde(default)]
    pub comment_id: Option<String>,
    /// Durable cooldown for an uncertain write's recheck.
    #[serde(default)]
    pub next_check_at: Option<String>,
    pub queued_at: String,
    /// When the attempt was persisted (the caps count these).
    #[serde(default)]
    pub sent_at: Option<String>,
    /// The tracker's time on the confirmed write (the issue's `updatedAt`
    /// after a move, the comment's after a comment).
    #[serde(default)]
    pub remote_at: Option<String>,
    /// Why it was skipped or is waiting.
    #[serde(default)]
    pub reason: Option<String>,
    /// Rechecks of an uncertain write so far.
    #[serde(default)]
    pub checks: u32,
    /// A move's issue state, and the issue's `updatedAt`, when it was
    /// queued: a different state when it is sent means a person moved it.
    #[serde(default)]
    pub seen_state: Option<String>,
    #[serde(default)]
    pub seen_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct WorkJournal {
    version: u32,
    entries: BTreeMap<String, WorkOp>,
}

impl Default for WorkJournal {
    fn default() -> Self {
        Self {
            version: 1,
            entries: BTreeMap::new(),
        }
    }
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
}

fn stamp(at: Option<&str>) -> Option<DateTime<Utc>> {
    at.and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The reason a move waits after its pre-send re-read failed (nothing sent).
const REREAD_FAILED: &str = "not sent: re-read failed";

impl WorkJournal {
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&WorkOp> {
        self.entries.get(key)
    }

    #[cfg(test)]
    pub fn entries(&self) -> impl Iterator<Item = (&String, &WorkOp)> {
        self.entries.iter()
    }

    /// After ledger decode: corrupt history must not start a fresh send.
    pub fn validate(&self, teams: &[String]) -> Result<(), String> {
        if self.version != 1 || self.entries.len() > MAX_ENTRIES {
            return Err(
                "The agent work outbox's version or size is invalid. No files were changed.".into(),
            );
        }
        let mut ids = BTreeSet::new();
        for (key, op) in &self.entries {
            // A later build's write: only its identity is checked; it is
            // never sent from here.
            if op.kind == OpKind::Unknown {
                if !identifier(key) {
                    return Err("The agent work outbox has an invalid or out-of-scope entry. No files were changed.".into());
                }
                continue;
            }
            let token = op.id.strip_prefix("work-").unwrap_or("");
            let bad = !identifier(key)
                || token.len() != 32
                || !token.bytes().all(|b| b.is_ascii_hexdigit())
                || !ids.insert(&op.id)
                || !identifier(&op.issue_id)
                || !teams.contains(&op.team_id)
                || (op.kind == OpKind::Comment)
                    != op.body.as_ref().is_some_and(|b| b.contains(&op.id))
                || (op.kind != OpKind::Comment && op.state_id.is_none())
                || (op.kind == OpKind::Comment
                    && (op.delivery == Delivery::Confirmed) != op.comment_id.is_some())
                || (op.delivery == Delivery::Unconfirmed && op.next_check_at.is_none())
                || [&op.next_check_at, &op.sent_at, &op.remote_at]
                    .iter()
                    .any(|at| at.is_some() && stamp(at.as_deref()).is_none());
            if bad {
                return Err("The agent work outbox has an invalid or out-of-scope entry. No files were changed.".into());
            }
        }
        Ok(())
    }

    /// Add `op` under `key` once; an existing key keeps its entry. At the cap
    /// the oldest settled entries make room; a journal of unsettled writes
    /// only (rechecks bound them) drops the new one rather than failing.
    pub fn enqueue(&mut self, key: &str, op: WorkOp) -> Result<(), String> {
        if !identifier(key) {
            return Err("An agent work write needs a stable key.".into());
        }
        if self.entries.contains_key(key) {
            return Ok(());
        }
        while self.entries.len() >= MAX_ENTRIES {
            let Some(old) = self
                .entries
                .iter()
                .find(|(_, op)| matches!(op.delivery, Delivery::Confirmed | Delivery::Skipped))
                .map(|(k, _)| k.clone())
            else {
                eprintln!("[tracker work] outbox full of unsettled writes; one was not queued");
                return Ok(());
            };
            self.entries.remove(&old);
        }
        self.entries.insert(key.into(), op);
        Ok(())
    }

    /// Settle every waiting write as Skipped (tracking went off).
    pub(crate) fn skip_pending(&mut self, reason: &str) -> usize {
        let mut n = 0;
        for op in self.entries.values_mut() {
            if op.delivery == Delivery::Pending {
                op.delivery = Delivery::Skipped;
                op.next_check_at = None;
                op.reason = Some(reason.into());
                n += 1;
            }
        }
        n
    }

    /// Settled entries older than a day keep only their record id.
    fn prune_bodies(&mut self, at: DateTime<Utc>) -> bool {
        let mut changed = false;
        for op in self.entries.values_mut() {
            let settled = matches!(op.delivery, Delivery::Confirmed | Delivery::Skipped);
            let old = stamp(Some(&op.queued_at))
                .is_some_and(|t| at - t > Duration::hours(SETTLED_BODY_HOURS));
            if settled && old && op.body.as_deref().is_some_and(|b| b != op.id) {
                op.body = Some(op.id.clone());
                changed = true;
            }
        }
        changed
    }

    /// Pending first (in queue order), then uncertain writes whose recheck is due.
    pub fn due_keys(&self, at: DateTime<Utc>) -> Vec<String> {
        let mut due: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, op)| op.kind != OpKind::Unknown)
            .filter(|(_, op)| match op.delivery {
                // A Pending write may carry a retry cooldown.
                Delivery::Pending => op
                    .next_check_at
                    .as_deref()
                    .is_none_or(|t| stamp(Some(t)).is_some_and(|t| t <= at)),
                Delivery::Unconfirmed => {
                    stamp(op.next_check_at.as_deref()).is_some_and(|t| t <= at)
                }
                _ => false,
            })
            .collect();
        due.sort_by(|(ka, a), (kb, b)| (&a.next_check_at, ka).cmp(&(&b.next_check_at, kb)));
        due.into_iter().map(|(k, _)| k.clone()).collect()
    }

    /// Tracking comments attempted since `since`, for `issue` or all.
    pub fn comments_since(&self, issue: Option<&str>, since: DateTime<Utc>) -> usize {
        self.entries
            .values()
            .filter(|op| op.kind == OpKind::Comment)
            .filter(|op| issue.is_none_or(|i| op.issue_id == i))
            .filter(|op| stamp(op.sent_at.as_deref()).is_some_and(|t| t >= since))
            .count()
    }

    /// A move to started for `issue` is queued, uncertain, or confirmed
    /// after the mirror last read the issue (two runs starting together move
    /// it once).
    pub fn move_in_flight(&self, issue: &TrackerIssue) -> bool {
        let mirrored = stamp(Some(&issue.updated_at));
        self.entries.values().any(|op| {
            op.issue_id == issue.id
                && op.kind == OpKind::MoveStarted
                && match op.delivery {
                    Delivery::Pending | Delivery::Unconfirmed => true,
                    Delivery::Confirmed => stamp(op.sent_at.as_deref()) > mirrored,
                    Delivery::Skipped => false,
                }
        })
    }

    /// When the issue's newest progress comment was queued.
    pub fn last_progress_at(&self, issue: &str) -> Option<DateTime<Utc>> {
        self.entries
            .values()
            .filter(|op| op.progress && op.issue_id == issue)
            .filter_map(|op| stamp(Some(&op.queued_at)))
            .max()
    }

    /// Hollow made the issue's last change: its newest confirmed move set the
    /// state the mirror shows, and the mirror changed no later than Hollow's
    /// newest confirmed write there. Returns the state Hollow set.
    pub fn hollow_moved_last(&self, issue: &TrackerIssue) -> Option<(OpKind, String)> {
        let ours: Vec<&WorkOp> = self
            .entries
            .values()
            .filter(|op| op.issue_id == issue.id && op.delivery == Delivery::Confirmed)
            .collect();
        let last_move = ours
            .iter()
            .filter(|op| !matches!(op.kind, OpKind::Comment | OpKind::Unknown))
            .max_by_key(|op| stamp(op.sent_at.as_deref()))?;
        let state = last_move.state_id.clone()?;
        let newest = ours
            .iter()
            .filter_map(|op| stamp(op.remote_at.as_deref()))
            .max()?;
        let mirrored = stamp(Some(&issue.updated_at))?;
        // A few seconds' grace: the issue's own time can trail the write's.
        let ours_last = mirrored <= newest + Duration::seconds(5);
        (issue.state.id == state && ours_last).then_some((last_move.kind, state))
    }

    /// Whether a queued move still applies to `issue`, as read now.
    pub fn move_wanted(&self, op: &WorkOp, issue: &TrackerIssue) -> bool {
        match op.kind {
            // Still backlog or unstarted, and still the state seen when it was
            // queued (a person's Backlog to Todo move is a move too).
            OpKind::MoveStarted => {
                matches!(issue.state.kind, StateKind::Backlog | StateKind::Unstarted)
                    && op
                        .seen_state
                        .as_deref()
                        .is_none_or(|seen| seen == issue.state.id)
            }
            OpKind::MoveReview => {
                matches!(
                    self.hollow_moved_last(issue),
                    Some((OpKind::MoveStarted, _))
                ) && op.state_id.as_deref() != Some(issue.state.id.as_str())
            }
            // Still open: a person who closed or canceled it already wins.
            OpKind::MoveDone => {
                !matches!(issue.state.kind, StateKind::Completed | StateKind::Canceled)
            }
            OpKind::Comment => true,
            OpKind::Unknown => false,
        }
    }

    fn restore(
        &mut self,
        key: &str,
        old: WorkOp,
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<(), TrackerError> {
        self.entries.insert(key.into(), old);
        persist(self).map_err(|_| journal_write_error())
    }

    fn settle(
        &mut self,
        key: &str,
        delivery: Delivery,
        comment_id: Option<String>,
        remote_at: Option<String>,
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<Delivery, TrackerError> {
        let old = self.entries[key].clone();
        let op = self.entries.get_mut(key).unwrap();
        op.delivery = delivery;
        op.comment_id = comment_id;
        op.next_check_at = None;
        op.remote_at = remote_at;
        if persist(self).is_err() {
            self.entries.insert(key.into(), old);
            return Err(journal_write_error());
        }
        Ok(delivery)
    }

    fn skip(
        &mut self,
        key: &str,
        reason: &str,
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<Delivery, TrackerError> {
        self.entries.get_mut(key).unwrap().reason = Some(reason.into());
        self.settle(key, Delivery::Skipped, None, None, persist)
    }

    /// A send that provably did not go out: a lasting cause (the issue left
    /// the team or is gone) settles it as Skipped; a passing one (rate,
    /// network, auth) puts it back with a cooldown. Either way the caller
    /// moves on to the next write.
    fn not_sent(
        &mut self,
        key: &str,
        old: WorkOp,
        error: TrackerError,
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<Delivery, TrackerError> {
        if matches!(error, TrackerError::OutOfScope | TrackerError::NotFound) {
            self.entries.insert(key.into(), old);
            return self.skip(
                key,
                "The issue is no longer in the team, or is gone.",
                persist,
            );
        }
        let mut waiting = old;
        waiting.next_check_at = Some((Utc::now() + Duration::minutes(RETRY_MINUTES)).to_rfc3339());
        self.restore(key, waiting, persist)?;
        Err(error)
    }

    /// Send or reconcile one write. `allowed` is re-read right before the
    /// mutation (the switch, the configured team). `persist` must durably
    /// save the whole ledger. An uncertain outcome is never sent again.
    pub async fn deliver_one(
        &mut self,
        key: &str,
        remote: &dyn TrackerAdapter,
        allowed: &(dyn Fn() -> bool + Send + Sync),
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<Delivery, TrackerError> {
        let op = self
            .entries
            .get(key)
            .cloned()
            .ok_or_else(|| TrackerError::Provider("Agent work write not found.".into()))?;
        match op.delivery {
            Delivery::Confirmed | Delivery::Skipped => return Ok(op.delivery),
            _ if !allowed() => return Ok(op.delivery),
            Delivery::Unconfirmed => return self.reconcile(key, &op, remote, persist).await,
            Delivery::Pending => {}
        }
        // Q-R24: an automatic move re-reads the issue just before it is
        // journaled and sent, so a status a person set after this pass's
        // snapshot (Done, say) is never overwritten. Nothing was sent yet,
        // so a failed read leaves the write waiting.
        if op.kind != OpKind::Comment {
            if !allowed() {
                return Ok(op.delivery);
            }
            match remote.issue(&op.issue_id).await {
                Ok(fresh) if fresh.team_id != op.team_id => {
                    return self.skip(key, "The issue moved elsewhere; the tracker wins.", persist)
                }
                Ok(fresh) if !self.move_wanted(&op, &fresh) => {
                    return self.skip(key, "The issue moved since; the tracker wins.", persist)
                }
                Ok(_) => {
                    let entry = self.entries.get_mut(key).unwrap();
                    if entry.reason.as_deref() == Some(REREAD_FAILED) {
                        entry.reason = None;
                    }
                }
                Err(TrackerError::OutOfScope | TrackerError::NotFound) => {
                    return self.skip(
                        key,
                        "The issue is no longer in the team, or is gone.",
                        persist,
                    )
                }
                // Nothing was sent: back off like any other no-send, and say
                // so instead of recording a failed update every pass.
                Err(error) => {
                    let entry = self.entries.get_mut(key).unwrap();
                    entry.reason = Some(REREAD_FAILED.into());
                    entry.next_check_at =
                        Some((Utc::now() + Duration::minutes(RETRY_MINUTES)).to_rfc3339());
                    persist(self).map_err(|_| journal_write_error())?;
                    return Err(error);
                }
            }
        }
        // Durable attempt before the provider call.
        {
            let entry = self.entries.get_mut(key).unwrap();
            entry.delivery = Delivery::Unconfirmed;
            entry.next_check_at = Some((Utc::now() + Duration::seconds(60)).to_rfc3339());
            entry.sent_at = Some(now());
        }
        if persist(self).is_err() {
            self.restore(key, op, persist)?;
            return Err(journal_write_error());
        }
        if op.kind != OpKind::Comment {
            let state = op.state_id.clone().unwrap_or_default();
            if !allowed() {
                self.restore(key, op, persist)?;
                return Ok(Delivery::Pending);
            }
            let patch = IssuePatch {
                state_id: Some(state.clone()),
                ..IssuePatch::default()
            };
            return match remote.update_issue(&op.issue_id, patch).await {
                Ok(issue) if issue.team_id == op.team_id && issue.state.id == state => self.settle(
                    key,
                    Delivery::Confirmed,
                    None,
                    Some(issue.updated_at),
                    persist,
                ),
                Ok(_) => self.skip(key, "The issue moved elsewhere; the tracker wins.", persist),
                // Known no-send: the scope read refused it before any mutation.
                Err(error @ (TrackerError::OutOfScope | TrackerError::NotFound)) => {
                    self.not_sent(key, op, error, persist)
                }
                Err(error) => Err(error),
            };
        }
        let body = op.body.clone().unwrap_or_default();
        let authorized = AtomicBool::new(false);
        let authorize = || {
            if !allowed() {
                return Err(TrackerError::OutOfScope);
            }
            authorized.store(true, Ordering::SeqCst);
            Ok(())
        };
        let receipt = remote
            .create_issue_comment(&op.issue_id, &body, &authorize)
            .await;
        if !authorized.load(Ordering::SeqCst) {
            // The adapter sends nothing before `authorize`: a proven no-send.
            return match receipt.err() {
                Some(error) => self.not_sent(key, op, error, persist),
                // Tracking went off between the checks: it waits.
                None => self.restore(key, op, persist).map(|()| Delivery::Pending),
            };
        }
        match receipt {
            Ok(change)
                if change.issue_id == op.issue_id
                    && change.team_id == op.team_id
                    && change.comment.body == body
                    && identifier(&change.comment.id) =>
            {
                let at = Some(change.updated_at).filter(|t| !t.is_empty());
                self.settle(
                    key,
                    Delivery::Confirmed,
                    Some(change.comment.id),
                    at,
                    persist,
                )
            }
            Ok(_) => Err(TrackerError::Provider(
                "An agent work comment returned an invalid receipt; it stays unconfirmed.".into(),
            )),
            Err(error) => Err(error),
        }
    }

    async fn reconcile(
        &mut self,
        key: &str,
        op: &WorkOp,
        remote: &dyn TrackerAdapter,
        persist: &mut (dyn FnMut(&WorkJournal) -> Result<(), String> + Send),
    ) -> Result<Delivery, TrackerError> {
        {
            let entry = self.entries.get_mut(key).unwrap();
            entry.next_check_at = Some((Utc::now() + Duration::hours(1)).to_rfc3339());
            entry.checks += 1;
        }
        if persist(self).is_err() {
            self.entries.insert(key.into(), op.clone());
            return Err(journal_write_error());
        }
        let last = self.entries[key].checks >= MAX_RECHECKS;
        let uncertain = "Its outcome stayed unconfirmed after its rechecks; it was not sent again.";
        let gone = "The issue is no longer in the team, or is gone.";
        if op.kind != OpKind::Comment {
            let issue = match remote.issue(&op.issue_id).await {
                Ok(issue) if issue.team_id == op.team_id => issue,
                Ok(_) | Err(TrackerError::OutOfScope | TrackerError::NotFound) => {
                    return self.skip(key, gone, persist)
                }
                Err(_) if last => return self.skip(key, uncertain, persist),
                Err(error) => return Err(error),
            };
            return if Some(&issue.state.id) == op.state_id.as_ref() {
                self.settle(
                    key,
                    Delivery::Confirmed,
                    None,
                    Some(issue.updated_at),
                    persist,
                )
            } else {
                self.skip(key, "The issue moved elsewhere; the tracker wins.", persist)
            };
        }
        let body = op.body.as_deref().unwrap_or_default();
        let comments = match remote.issue_comments(&op.issue_id).await {
            Ok(comments) => comments,
            Err(TrackerError::OutOfScope | TrackerError::NotFound) => {
                return self.skip(key, gone, persist)
            }
            Err(_) if last => return self.skip(key, uncertain, persist),
            Err(error) => return Err(error),
        };
        let found: Vec<_> = comments
            .into_iter()
            .filter(|c| {
                c.issue_id == op.issue_id
                    && c.team_id == op.team_id
                    && c.comment.body.contains(&op.id)
            })
            .collect();
        match found.as_slice() {
            [one] if one.comment.body == body && identifier(&one.comment.id) => {
                let at = Some(one.updated_at.clone()).filter(|t| !t.is_empty());
                self.settle(
                    key,
                    Delivery::Confirmed,
                    Some(one.comment.id.clone()),
                    at,
                    persist,
                )
            }
            // None, several or edited: never resent; let go after the rechecks.
            _ if last => self.skip(key, uncertain, persist),
            _ => Ok(Delivery::Unconfirmed),
        }
    }
}

fn journal_write_error() -> TrackerError {
    TrackerError::Provider(
        "The agent work outbox could not be saved; nothing was acknowledged.".into(),
    )
}

/// A new write for `issue`, keyed `ev<event>-<suffix>` so the journal keeps
/// event order (a start's move before its comment).
fn op(event: &PendingEvent, team_id: &str, issue_id: &str, kind: OpKind) -> WorkOp {
    WorkOp {
        id: format!("work-{:032x}", rand::random::<u128>()),
        session_key: event.session_key.clone(),
        team_id: team_id.into(),
        issue_id: issue_id.into(),
        kind,
        body: None,
        state_id: None,
        progress: false,
        delivery: Delivery::Pending,
        comment_id: None,
        next_check_at: None,
        queued_at: now(),
        sent_at: None,
        remote_at: None,
        reason: None,
        checks: 0,
        seen_state: None,
        seen_at: None,
    }
}

fn key(event: i64, suffix: &str) -> String {
    format!("ev{event:012}-{suffix}")
}

/// The team's started states by position.
fn started_states(state: &Ledger, team: &str) -> Vec<super::TrackerState> {
    let mut states: Vec<_> = state
        .meta_for(team)
        .map(|m| m.states.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.kind == StateKind::Started)
        .collect();
    states.sort_by(|a, b| a.position.total_cmp(&b.position));
    states
}

/// The team's first completed state (Done), T3b's close.
pub(crate) fn completed_state(state: &Ledger, team: &str) -> Option<super::TrackerState> {
    state
        .meta_for(team)?
        .states
        .iter()
        .filter(|s| s.kind == StateKind::Completed)
        .min_by(|a, b| a.position.total_cmp(&b.position))
        .cloned()
}

/// What one pass may still send.
struct Budget {
    sends: usize,
}

impl Budget {
    fn spend(&mut self, remote: &dyn TrackerAdapter) -> bool {
        if self.sends == 0 || remote.rate().is_some_and(|r| r.is_low()) {
            return false;
        }
        self.sends -= 1;
        true
    }
}

/// The ledger's issue, or a create's issue the mirror hasn't pulled yet.
fn issue_in(state: &Ledger, id: &str) -> Option<TrackerIssue> {
    state
        .records
        .get(id)
        .and_then(|r| r.issue.clone())
        .or_else(|| {
            state
                .creates
                .values()
                .filter_map(|c| c.issue.clone())
                .find(|i| i.id == id)
        })
        .filter(|i| state.has_team(&i.team_id))
}

/// One sync pass of agent work (called from the pass that holds the locks).
/// It never ends the pass: what goes wrong lands in the report. While the
/// switch is off it makes zero tracker requests and lets every waiting write
/// and untaken fact go, so turning it on never back-posts.
pub(crate) async fn sync(
    config: &Config,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    report: &mut TrackerSyncOutcome,
) {
    let root = config.vault_path.clone();
    let scope = state.scope.clone();
    let allowed = || {
        config.tracker.track_agent_work
            && work::ai_scope(config)
            && crate::tracker_sync::enabled(config).is_ok()
            && config.tracker.team_ids().contains(&scope)
    };
    let mut persist = |ledger: &Ledger| crate::tracker_sync::save(&root, ledger);
    if !allowed() {
        // Normally done already at the pass's start (`cancel_if_off`).
        if let Err(error) = cancel_if_off(config, state) {
            report.errors.push(error);
        }
        return;
    }
    let mut budget = Budget { sends: MAX_SENDS };
    if let Err(error) = intake(&root, remote, state, &mut budget, &mut persist, report).await {
        report.errors.push(format!("Agent work intake: {error}"));
    }
    deliver(&root, remote, state, &mut budget, &allowed, report).await;
    if state.work.prune_bodies(Utc::now()) {
        let _ = persist(&*state);
    }
    let _ = work::prune_events(&root);
}

/// The switch is off (or the scope isn't team AI): settle waiting writes
/// as Skipped and take every untaken fact, saved before anything else. The
/// sync pass calls this first, before any remote request, so a pass that
/// fails offline still cancels and a later switch-on never back-posts.
/// Returns whether it changed the ledger.
pub(crate) fn cancel_if_off(config: &Config, state: &mut Ledger) -> Result<bool, String> {
    let on = config.tracker.track_agent_work
        && work::ai_scope(config)
        && config.tracker.team_id.as_deref() == Some(state.scope.as_str());
    if on {
        return Ok(false);
    }
    let skipped = state.work.skip_pending("Tracking agent work is off.");
    if skipped > 0 {
        crate::tracker_sync::save(&config.vault_path, state)
            .map_err(|e| format!("Agent work could not save its outbox: {e}"))?;
    }
    work::take_all_events(&config.vault_path).map_err(|e| format!("Agent work facts: {e}"))?;
    Ok(skipped > 0)
}

/// Turn ready host facts into outbox writes (and creates). Facts are marked
/// taken only after the ledger that holds their writes is saved.
async fn intake(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    budget: &mut Budget,
    persist: &mut (dyn FnMut(&Ledger) -> Result<(), String> + Send),
    report: &mut TrackerSyncOutcome,
) -> Result<(), String> {
    let events = work::ready_events(root, MAX_NEW)?;
    let notes = work::progress_events(root, 200)?;
    if events.is_empty() && notes.is_empty() {
        return Ok(());
    }
    let mut taken: Vec<i64> = Vec::new();
    for event in &events {
        // A later build's fact: taken, never sent.
        if event.event == WorkEvent::Unknown {
            taken.push(event.id);
            continue;
        }
        let link = work::link_for(root, &event.session_key);
        // No link, or a helper's (depth 2): nothing to post.
        let Some(link) = link.filter(|l| l.depth < 2) else {
            taken.push(event.id);
            continue;
        };
        if let WorkEvent::Create {
            create_key,
            project_id,
            title,
            description,
            team_id,
            labels,
            assign_viewer,
            due,
        } = &event.event
        {
            // Each team's pass creates its own (team AI's unless the fact
            // names HOL, T3a); the other pass leaves the fact for it.
            let team = team_id.as_deref().unwrap_or(work::AI_TEAM_ID);
            if state.scope != team {
                continue;
            }
            let facts = (
                create_key.as_str(),
                project_id.as_str(),
                title.as_str(),
                description.as_str(),
            );
            match create(
                root,
                remote,
                state,
                budget,
                persist,
                &event.session_key,
                facts,
                (labels, *assign_viewer, due.as_deref()),
            )
            .await
            {
                Created::Issue(issue) => {
                    work::fill_created(root, &event.session_key, &issue.id, &issue.key)?;
                    taken.push(event.id);
                }
                Created::Wait => {}
                Created::Failed(why) => {
                    // The create key's failure stays in the audit (a repeat
                    // adds no row), so a chat isn't decided again (T3a).
                    work::record_tracker_write(
                        root,
                        &work::TrackerWrite {
                            session_key: &event.session_key,
                            issue_id: "",
                            action: "create",
                            outcome: "failed",
                            request_key: Some(create_key),
                            detail: Some(why.clone()),
                            ..work::TrackerWrite::default()
                        },
                    );
                    // Untie the run and take its facts: nothing waits on it.
                    report.errors.push(why);
                    work::drop_link(root, &event.session_key)?;
                }
            }
            continue;
        }
        let Some(issue) = link.issue_id.as_deref().and_then(|id| issue_in(state, id)) else {
            let why = "An agent work fact names an issue outside the mirror; it was dropped.";
            report.errors.push(why.into());
            // X2: refused before any request (another team's issue, or gone).
            work::record_tracker_write(
                root,
                &work::TrackerWrite {
                    session_key: &event.session_key,
                    issue_id: link.issue_id.as_deref().unwrap_or(""),
                    issue_key: link.issue_key.as_deref(),
                    action: if matches!(event.event, WorkEvent::Started { .. }) {
                        "update"
                    } else {
                        "comment"
                    },
                    outcome: "failed",
                    request_key: Some(&format!("fact-{}", event.id)),
                    detail: Some(why.into()),
                    ..work::TrackerWrite::default()
                },
            );
            taken.push(event.id);
            continue;
        };
        // The op belongs to the issue's own team (the shared team or the
        // personal one); each team's pass delivers its own.
        let scope = issue.team_id.clone();
        let started = started_states(state, &scope);
        match &event.event {
            WorkEvent::Started { .. } => {
                let open = matches!(issue.state.kind, StateKind::Backlog | StateKind::Unstarted)
                    && !state.work.move_in_flight(&issue);
                if let (true, Some(first)) = (open, started.first()) {
                    let mut move_op = op(event, &scope, &issue.id, OpKind::MoveStarted);
                    move_op.state_id = Some(first.id.clone());
                    move_op.seen_state = Some(issue.state.id.clone());
                    move_op.seen_at = Some(issue.updated_at.clone());
                    state.work.enqueue(&key(event.id, "a-move"), move_op)?;
                }
                let mut comment = op(event, &scope, &issue.id, OpKind::Comment);
                comment.body = Some(work::render_started(
                    &event.event,
                    &event.session_key,
                    &comment.id,
                ));
                state.work.enqueue(&key(event.id, "b-comment"), comment)?;
            }
            WorkEvent::Outcome { status, .. } => {
                let mut comment = op(event, &scope, &issue.id, OpKind::Comment);
                let head = work::LogHead::of(root, &event.session_key, &event.at, &issue);
                comment.body = Some(work::render_outcome(
                    &event.event,
                    &head,
                    &event.session_key,
                    &comment.id,
                ));
                state.work.enqueue(&key(event.id, "a-comment"), comment)?;
                // In Review only on success, only when Hollow made the last
                // move (to started), and never Done. Rechecked before sending.
                if status == "success" {
                    if let Some((OpKind::MoveStarted, from)) = state.work.hollow_moved_last(&issue)
                    {
                        if let Some(next) = started.iter().skip_while(|s| s.id != from).nth(1) {
                            let mut review = op(event, &scope, &issue.id, OpKind::MoveReview);
                            review.state_id = Some(next.id.clone());
                            state.work.enqueue(&key(event.id, "b-review"), review)?;
                        }
                    }
                }
            }
            // T3b and T3e: only the issue Hollow created for this Waiting on
            // you item or commitment, and only while it is open; never
            // another issue.
            WorkEvent::Resolved { .. } => {
                let own = work::linked_by::is_copy(&link.linked_by) && link.created_issue;
                let open = !matches!(issue.state.kind, StateKind::Completed | StateKind::Canceled);
                if own && open {
                    // No completed state: the fact stays, audited once, and
                    // is tried again on a later pass.
                    let Some(done) = completed_state(state, &scope) else {
                        work::record_tracker_write(
                            root,
                            &work::TrackerWrite {
                                session_key: &event.session_key,
                                issue_id: &issue.id,
                                issue_key: Some(&issue.key),
                                action: "update",
                                outcome: "skipped",
                                request_key: Some(&format!("fact-{}-no-completed-state", event.id)),
                                detail: Some("The issue's team has no completed state Hollow can close it in, so the close waits.".into()),
                                ..work::TrackerWrite::default()
                            },
                        );
                        continue;
                    };
                    let mut comment = op(event, &scope, &issue.id, OpKind::Comment);
                    comment.body = Some(work::render_resolved(
                        &event.event,
                        &link.linked_by,
                        &comment.id,
                    ));
                    state.work.enqueue(&key(event.id, "a-comment"), comment)?;
                    let mut close = op(event, &scope, &issue.id, OpKind::MoveDone);
                    close.state_id = Some(done.id);
                    close.seen_state = Some(issue.state.id.clone());
                    close.seen_at = Some(issue.updated_at.clone());
                    state.work.enqueue(&key(event.id, "b-done"), close)?;
                }
            }
            // DR1b: a decision's build log comment, on the issue its record names.
            WorkEvent::Decision { .. } => {
                let mut comment = op(event, &scope, &issue.id, OpKind::Comment);
                comment.body = Some(work::render_decision(
                    &event.event,
                    &event.at,
                    &event.session_key,
                    &comment.id,
                ));
                state.work.enqueue(&key(event.id, "a-comment"), comment)?;
            }
            WorkEvent::Progress { .. } | WorkEvent::Create { .. } | WorkEvent::Unknown => {}
        }
        taken.push(event.id);
    }
    // Progress: one comment per issue every 15 minutes, folding the notes.
    let mut by_issue: BTreeMap<String, Vec<&PendingEvent>> = BTreeMap::new();
    for note in &notes {
        let link = work::link_for(root, &note.session_key);
        match link.filter(|l| l.depth < 2).and_then(|l| l.issue_id) {
            Some(issue) if issue_in(state, &issue).is_some() => {
                by_issue.entry(issue).or_default().push(note)
            }
            _ => taken.push(note.id),
        }
    }
    let fold_after = Utc::now() - Duration::minutes(work::PROGRESS_MINUTES);
    for (issue_id, notes) in by_issue {
        if state
            .work
            .last_progress_at(&issue_id)
            .is_some_and(|at| at > fold_after)
        {
            continue;
        }
        let lines: Vec<work::Note> = notes
            .iter()
            .filter_map(|n| match &n.event {
                WorkEvent::Progress { line, link, checks } => {
                    Some((line.clone(), link.clone(), checks.clone()))
                }
                _ => None,
            })
            .collect();
        // Keyed by the first note, so a retry after a lost mark folds the
        // same notes into the same entry.
        let first = notes[0];
        let last = notes.last().unwrap();
        let Some(issue) = issue_in(state, &issue_id) else {
            continue;
        };
        let scope = issue.team_id.clone();
        // T3g: dated at the newest note, naming every run that wrote one.
        let mut head = work::LogHead::of(root, &last.session_key, &last.at, &issue);
        let mut whos: Vec<String> = Vec::new();
        for note in &notes {
            if whos.len() < 3 && !whos.contains(&note.session_key) {
                whos.push(note.session_key.clone());
            }
        }
        if whos.len() > 1 {
            let names: Vec<String> = whos.iter().map(|k| work::who_of(root, k)).collect();
            head.who = names.join(" and ");
        }
        let mut comment = op(last, &scope, &issue_id, OpKind::Comment);
        comment.progress = true;
        comment.body = Some(work::render_progress(
            &lines,
            &head,
            &last.session_key,
            &comment.id,
        ));
        state.work.enqueue(&key(first.id, "progress"), comment)?;
        taken.extend(notes.iter().map(|n| n.id));
    }
    persist(&*state)?;
    work::mark_journaled(root, &taken)
}

enum Created {
    Issue(TrackerIssue),
    /// Not yet (budget, or the pass's create recovery confirms a lost reply).
    Wait,
    /// Refused or failed: nothing will be created.
    Failed(String),
}

/// The create's project is live, mirrored, and in the configured team AI;
/// or, in team HOL's own pass, a create with no project (T3a).
fn project_in_scope(state: &Ledger, project_id: &str) -> bool {
    if state.scope == super::linear::PERSONAL_TEAM_ID {
        return project_id.is_empty();
    }
    state.scope == work::AI_TEAM_ID
        && state.records.get(project_id).is_some_and(|r| {
            r.kind == "project"
                && r.state == "active"
                && !r.remote_missing
                && r.project
                    .as_ref()
                    .is_some_and(|p| p.team_ids.iter().any(|t| t == &state.scope))
        })
}

/// A create in team HOL's pass (T3a): its team named and its `agent`
/// label from the team's metadata (filed without it when the label is
/// missing; `tracker.ensure_labels` makes it). Team AI's is unchanged.
fn hol_input(state: &Ledger, mut input: NewIssue) -> NewIssue {
    if state.scope != super::linear::PERSONAL_TEAM_ID {
        return input;
    }
    input.team_id = Some(state.scope.clone());
    if let Some(label) = state.meta.as_ref().and_then(|meta| {
        meta.labels
            .iter()
            .find(|l| l.name.eq_ignore_ascii_case(AGENT_LABEL))
    }) {
        input.label_ids = vec![label.id.clone()];
    }
    input
}

/// Sends again of a copy's create that never landed (a Waiting on you item,
/// T3b, or a commitment, T3e).
pub(crate) const WAITING_RESENDS: u32 = 3;

/// The label a chat's issue in team HOL carries (D8's HOL labels).
pub(crate) const AGENT_LABEL: &str = "agent";

/// A create that names its labels or asks for the viewer (T3b): its team
/// (HOL's own pass names it), the named labels the team has (by name, any
/// case) and the signed-in user from the team's metadata. Returns whether
/// the assignee was wanted but unknown.
fn labelled_input(
    state: &Ledger,
    mut input: NewIssue,
    labels: &[String],
    assign: bool,
) -> (NewIssue, bool) {
    if state.scope == super::linear::PERSONAL_TEAM_ID {
        input.team_id = Some(state.scope.clone());
    }
    let meta = state.meta.as_ref();
    input.label_ids = labels
        .iter()
        .filter_map(|name| {
            meta?
                .labels
                .iter()
                .find(|l| l.name.trim().eq_ignore_ascii_case(name))
                .map(|l| l.id.clone())
        })
        .collect();
    let viewer = assign
        .then(|| meta.and_then(|m| m.viewer.as_ref()).map(|v| v.id.clone()))
        .flatten();
    let unassigned = assign && viewer.is_none();
    input.assignee_id = viewer;
    (input, unassigned)
}

/// T1's labels for a T3b create: each named label this team keeps
/// (`linear::labels_for`) that its metadata lacks is made now through the
/// adapter's `create_label` (one send each, audited as `label`) and added
/// to the metadata. False when the budget ran out (the create waits).
async fn ensure_labels_for(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    budget: &mut Budget,
    session_key: &str,
    labels: &[String],
) -> bool {
    let keeps = super::linear::labels_for(&state.scope);
    for name in labels {
        let have = state.meta.as_ref().is_none_or(|m| {
            m.labels
                .iter()
                .any(|l| l.name.trim().eq_ignore_ascii_case(name))
        });
        if have || !keeps.iter().any(|k| k.eq_ignore_ascii_case(name)) {
            continue;
        }
        if !budget.spend(remote) {
            return false;
        }
        let made = remote.create_label(name).await;
        work::record_tracker_write(
            root,
            &work::TrackerWrite {
                session_key,
                issue_id: "",
                action: "label",
                outcome: if made.is_ok() { "ok" } else { "failed" },
                after: Some(serde_json::json!({"team": state.scope, "label": name})),
                detail: made.as_ref().err().map(ToString::to_string),
                ..work::TrackerWrite::default()
            },
        );
        if let (Ok(label), Some(meta)) = (made, state.meta.as_mut()) {
            meta.labels.push(label);
        }
    }
    true
}

/// A start's create, through the ledger's create journal: one key, one
/// create, an attempted one never resent (the pass's create recovery
/// confirms a lost reply). The project is checked right before each send.
async fn create(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    budget: &mut Budget,
    persist: &mut (dyn FnMut(&Ledger) -> Result<(), String> + Send),
    session_key: &str,
    (create_key, project_id, title, description): (&str, &str, &str, &str),
    (labels, assign, due): (&[String], bool, Option<&str>),
) -> Created {
    let refused = || {
        let why = "An agent work create names a project that is moved, missing or outside team AI; nothing was created.";
        // X2: refused before any request.
        work::record_tracker_write(
            root,
            &work::TrackerWrite {
                session_key,
                issue_id: "",
                action: "create",
                outcome: "failed",
                after: Some(serde_json::json!({"title": title, "projectId": project_id})),
                request_key: Some(create_key),
                detail: Some(why.into()),
                ..work::TrackerWrite::default()
            },
        );
        Created::Failed(why.into())
    };
    // T3b (and T3e's commitments): a copy's create that provably never reached the
    // tracker (not attempted, with an error: the pass's recovery read its
    // id back absent) is sent again with the same id, up to
    // `WAITING_RESENDS` times; then it gives up for good, audited under
    // `<create key>-gave-up`, and the item is never filed again.
    if super::waiting_work::is_copy_key(session_key) {
        if let Some(entry) = state.creates.get_mut(create_key) {
            if entry.issue.is_none() && !entry.attempted && entry.error.is_some() {
                if entry.resends >= WAITING_RESENDS {
                    let why = format!(
                        "A copy's create (Waiting on you or a commitment) did not reach the tracker after {WAITING_RESENDS} more tries; Hollow gave up on it. Last error: {}",
                        entry.error.clone().unwrap_or_default()
                    );
                    work::record_tracker_write(
                        root,
                        &work::TrackerWrite {
                            session_key,
                            issue_id: "",
                            action: "create",
                            outcome: "failed",
                            request_key: Some(&format!(
                                "{create_key}{}",
                                super::waiting_work::GAVE_UP
                            )),
                            detail: Some(why.clone()),
                            ..work::TrackerWrite::default()
                        },
                    );
                    return Created::Failed(why);
                }
                entry.resends += 1;
                entry.error = None;
                entry.at = now();
                if persist(&*state).is_err() {
                    return Created::Wait;
                }
            }
        }
    }
    if let Some(entry) = state.creates.get(create_key) {
        if let Some(issue) = entry.issue.clone() {
            return Created::Issue(issue);
        }
        // An unsent retry keeps its error (Q-R22), so a create that never
        // lands gives up on the same clock as one that stays unconfirmed.
        if entry.attempted || entry.error.is_some() {
            let old = DateTime::parse_from_rfc3339(&entry.at).is_ok_and(|t| {
                Utc::now() - t.with_timezone(&Utc) > Duration::minutes(FAILED_CREATE_MINUTES)
            });
            if entry.error.is_some() && old {
                return Created::Failed("An agent work create failed or stayed unconfirmed; its run is no longer tied to an issue.".into());
            }
            if entry.attempted {
                return Created::Wait;
            }
        }
    } else {
        if !project_in_scope(state, project_id) || title.trim().is_empty() {
            return refused();
        }
        if !ensure_labels_for(root, remote, state, budget, session_key, labels).await {
            return Created::Wait;
        }
        let plain = NewIssue {
            title: title.into(),
            description: Some(description.into()),
            project_id: Some(project_id.into()).filter(|p: &String| !p.is_empty()),
            due: due.map(str::to_string),
            ..NewIssue::default()
        };
        let (input, unassigned) = if labels.is_empty() && !assign {
            (hol_input(state, plain), false)
        } else {
            labelled_input(state, plain, labels, assign)
        };
        if unassigned {
            // T3b: never a guessed or hard-coded person; said once per key.
            work::record_tracker_write(
                root,
                &work::TrackerWrite {
                    session_key,
                    issue_id: "",
                    action: "create",
                    outcome: "skipped",
                    request_key: Some(&format!("{create_key}-unassigned")),
                    detail: Some("The signed-in tracker user isn't known yet, so this issue is created unassigned.".into()),
                    ..work::TrackerWrite::default()
                },
            );
        }
        state.creates.insert(
            create_key.into(),
            Create {
                agent_key: create_key.into(),
                input,
                at: now(),
                attempted: false,
                issue: None,
                error: None,
                issue_id: crate::tracker_sync::new_issue_id(),
                resends: 0,
            },
        );
        if persist(&*state).is_err() {
            state.creates.remove(create_key);
            return Created::Wait;
        }
    }
    // Every send, a resumed one too: the project as the mirror has it now.
    let input = state.creates[create_key].input.clone();
    let project = input.project_id.clone().unwrap_or_default();
    if !project_in_scope(state, &project) {
        state.creates.remove(create_key);
        let _ = persist(&*state);
        return refused();
    }
    if !budget.spend(remote) {
        return Created::Wait;
    }
    // An entry an older build journaled gets its id now, saved with
    // `attempted` before the request (Q-R22).
    let entry = state.creates.get_mut(create_key).unwrap();
    if entry.issue_id.is_none() {
        entry.issue_id = crate::tracker_sync::new_issue_id();
    }
    entry.attempted = true;
    if persist(&*state).is_err() {
        state.creates.get_mut(create_key).unwrap().attempted = false;
        return Created::Wait;
    }
    let input = crate::tracker_sync::create_input(&state.creates[create_key]);
    let result = remote.create_issue(input).await;
    let outcome = match &result {
        Ok(issue) if issue.team_id == state.scope && identifier(&issue.id) => {
            let entry = state.creates.get_mut(create_key).unwrap();
            entry.issue = Some(issue.clone());
            entry.error = None;
            "ok"
        }
        Ok(_) => {
            state.creates.get_mut(create_key).unwrap().error = Some("The provider returned an out-of-scope create result; it was discarded and will not be resent.".into());
            "failed"
        }
        Err(error) => {
            state.creates.get_mut(create_key).unwrap().error = Some(error.to_string());
            "failed"
        }
    };
    work::record_tracker_write(
        root,
        &work::TrackerWrite {
            session_key,
            issue_id: result.as_ref().map(|i| i.id.as_str()).unwrap_or(""),
            issue_key: result.as_ref().ok().map(|i| i.key.as_str()),
            action: "create",
            outcome,
            after: Some(serde_json::json!({"title": title, "projectId": project})),
            request_key: Some(create_key),
            detail: state.creates[create_key].error.clone(),
            ..work::TrackerWrite::default()
        },
    );
    let _ = persist(&*state);
    match state.creates[create_key].issue.clone() {
        Some(issue) => Created::Issue(issue),
        None => Created::Wait,
    }
}

/// A queued move is still wanted, checked against the freshly pulled mirror
/// before it is sent (and again against a re-read right before the write,
/// in [`WorkJournal::deliver_one`]): a person's move always wins.
fn move_still_wanted(state: &Ledger, op: &WorkOp) -> bool {
    let Some(issue) = issue_in(state, &op.issue_id) else {
        return false;
    };
    state.work.move_wanted(op, &issue)
}

/// Send what is due, within the pass's budget and the comment caps. A write
/// that fails is recorded and the next one goes; a rate, auth or network
/// failure stops tracking for this pass (the person's sync is not held).
async fn deliver(
    root: &Path,
    remote: &dyn TrackerAdapter,
    state: &mut Ledger,
    budget: &mut Budget,
    allowed: &(dyn Fn() -> bool + Send + Sync),
    report: &mut TrackerSyncOutcome,
) {
    let at = Utc::now();
    let hour = at - Duration::hours(1);
    for key in state.work.due_keys(at) {
        let op = state.work.get(&key).unwrap().clone();
        // Each team's pass sends only the writes made in its own team.
        if op.team_id != state.scope {
            continue;
        }
        let pending = op.delivery == Delivery::Pending;
        if pending && op.kind != OpKind::Comment && !move_still_wanted(state, &op) {
            let mut journal = state.work.clone();
            let why = "The issue moved since; the tracker wins.";
            let _ = journal.skip(&key, why, &mut |next| {
                state.work = next.clone();
                crate::tracker_sync::save(root, state)
            });
            state.work = journal;
            // X2: a move let go unsent is still one audited write.
            work::record_tracker_write(
                root,
                &work::TrackerWrite {
                    session_key: &op.session_key,
                    issue_id: &op.issue_id,
                    action: "update",
                    outcome: "skipped",
                    before: Some(serde_json::json!({"state": op.seen_state})),
                    after: Some(serde_json::json!({"state": op.state_id})),
                    request_key: Some(&op.id),
                    detail: Some(why.into()),
                    ..work::TrackerWrite::default()
                },
            );
            continue;
        }
        if pending && op.kind == OpKind::Comment {
            let over = state.work.comments_since(None, hour) >= COMMENTS_PER_HOUR
                || state.work.comments_since(Some(&op.issue_id), hour) >= COMMENTS_PER_ISSUE_HOUR;
            if over {
                continue;
            }
        }
        if !budget.spend(remote) {
            break;
        }
        let mut journal = state.work.clone();
        let result = journal
            .deliver_one(&key, remote, allowed, &mut |next| {
                state.work = next.clone();
                crate::tracker_sync::save(root, state)
            })
            .await;
        state.work = journal;
        // X2: one audit row per write and outcome (a recheck that changes
        // nothing adds none; a write that waits for tracking adds none).
        let outcome = match &result {
            Ok(Delivery::Confirmed) => Some("ok"),
            Ok(Delivery::Skipped) => Some("skipped"),
            Ok(Delivery::Unconfirmed) => Some("unconfirmed"),
            Ok(Delivery::Pending) => None,
            // A re-read that failed before anything was journaled or sent.
            Err(_)
                if state
                    .work
                    .get(&key)
                    .is_some_and(|o| o.reason.as_deref() == Some(REREAD_FAILED)) =>
            {
                Some("not_sent")
            }
            Err(_) => Some("failed"),
        };
        let settled = state.work.get(&key).cloned().unwrap_or_else(|| op.clone());
        let detail = match &result {
            Err(error) if settled.reason.as_deref() == Some(REREAD_FAILED) => {
                Some(format!("{REREAD_FAILED} ({error})"))
            }
            Err(error) => Some(error.to_string()),
            Ok(_) => settled.reason.clone(),
        };
        let (before, after) = if op.kind == OpKind::Comment {
            let kind = if op.progress { "progress" } else { "comment" };
            (
                None,
                Some(serde_json::json!({"comment": kind, "commentId": settled.comment_id})),
            )
        } else {
            (
                Some(serde_json::json!({"state": op.seen_state})),
                Some(serde_json::json!({"state": op.state_id})),
            )
        };
        if let Some(outcome) = outcome {
            work::record_tracker_write(
                root,
                &work::TrackerWrite {
                    session_key: &op.session_key,
                    issue_id: &op.issue_id,
                    action: if op.kind == OpKind::Comment {
                        "comment"
                    } else {
                        "update"
                    },
                    outcome,
                    before,
                    after,
                    request_key: Some(&op.id),
                    detail,
                    ..work::TrackerWrite::default()
                },
            );
        }
        match result {
            Ok(Delivery::Confirmed) => report.pushed += 1,
            Ok(_) => {}
            Err(
                error @ (TrackerError::RateLimited { .. }
                | TrackerError::Auth
                | TrackerError::Network(_)),
            ) => {
                report
                    .errors
                    .push(format!("Agent work paused for this pass: {error}"));
                break;
            }
            Err(error) => report
                .errors
                .push(format!("An agent work write failed: {error}")),
        }
    }
}

#[cfg(test)]
#[path = "work_outbox_tests.rs"]
mod tests;
