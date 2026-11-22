//! Commitments as tracker issues (two-Macs T3e, 2026-11-15; decision c5 of
//! `workspaces/main/outputs/hollow-next-build-decisions-2026-11-14.md`).
//!
//! A commitment the person owes ([`crate::vault_commitments`], direction
//! `owed_by_me`, open, not snoozed) that appeared after the current "Track
//! agent work" mode began gets one issue: assigned to the signed-in user,
//! labelled `commitment`, due on the commitment's own due date (none when
//! it has none). When it is marked done or dropped in Hollow the issue gets
//! one comment saying so and moves to its team's completed state.
//!
//! The machinery is T3b's ([`super::waiting_work`]): the same mode clock,
//! team rule, filing, idempotency, close gate and outbox. Only what a
//! commitment is, where it belongs and how it ended live here. Where: a
//! commitment whose source note is in a workspace that names its tracker
//! project involves the team (team AI, generic title, nothing of the
//! commitment's words); any other is HOL with its own title, redacted.

use super::chat_work::{self, Running};
use super::waiting_work::{self, Absence, Absences, Copy, Ended};
use super::work::{self, WorkLink};
use crate::config::Config;
use crate::tracker::settings::AgentWork;
use crate::vault_commitments::{self, Commitment};
use chrono::{DateTime, NaiveDate, Utc};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Link keys of commitment copies start with this.
pub(crate) const KEY_PREFIX: &str = "commitment:";
/// The label every copy carries (T1 keeps it in AI and HOL).
pub(crate) const LABEL: &str = "commitment";
/// The absence count's family (beside T3b's).
const FAMILY: &str = "commitment";
/// Team AI's title: the kind only, never the commitment's words.
pub(crate) const GENERIC_TITLE: &str = "A commitment you made";

/// A commitment's id: from its note path, which never changes (status
/// edits rewrite the note in place) and carries a random suffix, so a new
/// commitment never reuses an old one's id.
pub(crate) fn commitment_id(vault_path: &str) -> String {
    hex32(&format!("commitment\n{vault_path}"))
}

/// The copy's link key.
pub(crate) fn link_key(id: &str) -> String {
    format!("{KEY_PREFIX}{id}")
}

/// The copy's create key (audit and create journal).
pub(crate) fn create_key(id: &str) -> String {
    format!("work-{}", hex32(&format!("commitment-create\n{id}")))
}

fn hex32(text: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex[..32].to_string()
}

/// The ledger as read now: every commitment that parsed and the id of every
/// note in the folder (one that didn't parse is present, never absent).
#[derive(Debug, Default, Clone)]
pub(crate) struct Snapshot {
    pub commitments: Vec<Commitment>,
    pub present: HashSet<String>,
}

/// Read the commitments folder whole; `None` when it can't be (then nothing
/// closes on absence). A missing folder is an empty ledger only while the
/// vault root exists and no commitment copy is open: a folder gone with
/// copies still open (a moved vault, an unmounted volume, a sync mid-way) is
/// an incomplete read, never every commitment removed.
pub(crate) fn snapshot(vault: &Path) -> Option<Snapshot> {
    let mut present = HashSet::new();
    match std::fs::read_dir(vault.join("commitments")) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let open_copies = !work::open_links_by(vault, work::linked_by::COMMITMENT).is_empty();
            if open_copies || !vault.is_dir() {
                return None;
            }
        }
        Ok(entries) => {
            for entry in entries {
                let name = entry.ok()?.file_name();
                let name = name.to_str()?;
                if !name.starts_with('.') && name.ends_with(".md") {
                    present.insert(commitment_id(&format!("commitments/{name}")));
                }
            }
        }
        Err(_) => return None,
    }
    Some(Snapshot {
        commitments: vault_commitments::list(vault),
        present,
    })
}

/// The sync pass's call (after T3b's): local only; the outbox sends.
pub(crate) fn reconcile(config: &Config) -> waiting_work::Reconciled {
    let running = chat_work::running_of(config);
    if running.mode == AgentWork::Off {
        waiting_work::forget_absent(&config.vault_path, FAMILY);
        return waiting_work::Reconciled::default();
    }
    let snap = snapshot(&config.vault_path);
    let ledger = work::read_ledger(config);
    let can_close = |link: &WorkLink| {
        ledger
            .as_ref()
            .is_some_and(|l| waiting_work::closable(l, link))
    };
    reconcile_commitments(
        &config.vault_path,
        &running,
        snap.as_ref(),
        Utc::now(),
        &can_close,
    )
}

/// File copies of commitments that appeared, and close copies of those
/// marked done or dropped (or gone from a whole read on two passes a
/// minute apart). `snap` is `None` when the folder couldn't be read.
pub(crate) fn reconcile_commitments(
    vault: &Path,
    running: &Running,
    snap: Option<&Snapshot>,
    now: DateTime<Utc>,
    can_close: &dyn Fn(&WorkLink) -> bool,
) -> waiting_work::Reconciled {
    let mut done = waiting_work::Reconciled::default();
    let Some(since) = waiting_work::clock(vault, running, snap.is_some(), now, FAMILY) else {
        return done;
    };
    let Some(snap) = snap else {
        return done;
    };
    let today = now
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%d")
        .to_string();
    let mut new = 0;
    for c in snap.commitments.iter().filter(|c| wanted(c, since, &today)) {
        if new >= waiting_work::MAX_NEW {
            break;
        }
        let id = commitment_id(&c.vault_path);
        let key = link_key(&id);
        let create = create_key(&id);
        if waiting_work::decided(vault, &key, &create, now) {
            continue;
        }
        let workspace = workspace_of(vault, c);
        let copy = Copy {
            key: &key,
            create: &create,
            workspace_id: workspace.as_deref(),
            linked_by: work::linked_by::COMMITMENT,
            own_title: &c.title,
            generic_title: GENERIC_TITLE.into(),
            description: description(workspace.as_deref()),
            label: LABEL,
            due: due_of(c),
            what: "commitment".into(),
            facts: serde_json::json!({"kind": "commitment"}),
        };
        if waiting_work::file_copy(vault, running, &copy) {
            new += 1;
            done.filed += 1;
        }
    }
    if running.mode != AgentWork::On {
        return done;
    }
    let by_id: HashMap<String, &Commitment> = snap
        .commitments
        .iter()
        .map(|c| (commitment_id(&c.vault_path), c))
        .collect();
    let before = waiting_work::take_absent(vault, FAMILY);
    let mut still = Absences::new();
    for link in work::open_links_by(vault, work::linked_by::COMMITMENT) {
        // A create not confirmed yet closes on a later pass.
        if !link.created_issue || link.issue_id.is_none() {
            continue;
        }
        let Some(id) = link.session_key.strip_prefix(KEY_PREFIX) else {
            continue;
        };
        let Some(ended) = ended_of(by_id.get(id).copied(), snap.present.contains(id)) else {
            continue;
        };
        let gate = Absence {
            before: &before,
            still: &mut still,
            now,
        };
        if waiting_work::settle(vault, &link, &create_key(id), ended, gate, can_close) {
            done.closed += 1;
        }
    }
    waiting_work::keep_absent(vault, FAMILY, still);
    done
}

/// Open, the person's to keep, awake today, and new since the mode began.
fn wanted(c: &Commitment, since: DateTime<Utc>, today: &str) -> bool {
    c.status == "open"
        && c.direction == "owed_by_me"
        && !vault_commitments::is_snoozed(c, today)
        && DateTime::parse_from_rfc3339(&c.created).is_ok_and(|at| at.with_timezone(&Utc) >= since)
}

/// The commitment's own due date, when it is a real `YYYY-MM-DD`.
fn due_of(c: &Commitment) -> Option<String> {
    c.due
        .as_deref()
        .map(str::trim)
        .filter(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok())
        .map(str::to_string)
}

/// The workspace its source note is in (`workspaces/<id>/...`), if any.
fn workspace_of(vault: &Path, c: &Commitment) -> Option<String> {
    let rest = c.source_path.as_deref()?.strip_prefix("workspaces/")?;
    let id = rest.split('/').next().filter(|id| !id.is_empty())?;
    crate::workspaces::folder(vault, id).ok()?;
    Some(id.to_string())
}

/// Host facts only: nothing of the commitment's words, counterparty or
/// source.
pub(crate) fn description(workspace: Option<&str>) -> String {
    format!(
        "A commitment you made is kept in Hollow ({}). Mark it done there; this issue closes on its own when it is marked done or dropped.\n\n+++ Agent notes\n\nCreated by Hollow as a copy of a commitment in its ledger (Settings, Track agent work). Hollow stays where it is kept; it comments how it ended here and closes this issue.\n\n+++",
        workspace.map_or_else(|| "no workspace".to_string(), |id| format!("workspace {id}")),
    )
}

/// How a linked commitment ended: its stored status first (done or
/// dropped); a note gone from a whole read only by absence; `None` while it
/// is open (snoozed or reopened included) or its note didn't parse.
pub(crate) fn ended_of(c: Option<&Commitment>, present: bool) -> Option<Ended> {
    match c.map(|c| c.status.as_str()) {
        Some("fulfilled") => waiting_work::ended("Marked done in Hollow.", true),
        Some("cancelled") => waiting_work::ended("Dropped in Hollow.", true),
        Some(_) => None,
        None if present => None,
        None => waiting_work::ended("Removed from Hollow's commitments.", false),
    }
}

#[cfg(test)]
#[path = "commitment_work_tests.rs"]
mod tests;
