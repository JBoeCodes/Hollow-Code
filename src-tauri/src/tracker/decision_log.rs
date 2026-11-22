//! A decision record's build log comment (DR1b, the comment half of DR1).
//!
//! When a decision is recorded or its status changes, and the record names a
//! tracker issue Hollow has agent work on (`issue: ENG-12`), Hollow composes the
//! T3g build log comment with four lines: `What:` (the decision), `Why:`,
//! `Rejected:` (left out when the record has none) and `Verified:`. It goes
//! the way every T3g comment goes ([`super::work`], [`super::work_outbox`]):
//!
//! - `trackAgentWork` off: nothing.
//! - `dryRun`: one `tracker_write` audit row (action `comment`, outcome
//!   `skipped`), no queued write and no request.
//! - `on`: a [`WorkEvent::Decision`] fact on the newest run linked to that
//!   issue; the outbox renders it and sends it under the same locks, caps and
//!   redaction as the other build log comments.
//!
//! A decision with no `issue`, or whose issue has no linked run, produces
//! nothing at all. Everything here is local and never fails the caller.

use super::chat_work;
use super::settings::AgentWork;
use super::work::{self, WorkEvent, WorkLink};
use crate::records::Decision;
use chrono::{DateTime, Utc};
use std::path::Path;

/// The audit row's request key: one per decision, change and second, so a
/// repeat of the same call adds no row.
fn request_key(path: &str, change: &str, now: DateTime<Utc>) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(format!("{path}\0{change}\0{}", now.timestamp()));
    let hex: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    format!("decision-log-{hex}")
}

/// The fact a decision's change makes.
pub(crate) fn event_of(d: &Decision, change: &str) -> WorkEvent {
    let rejected = d
        .rejected
        .iter()
        .map(|r| match &r.reason {
            Some(reason) => format!("{} -> {reason}", r.option),
            None => r.option.clone(),
        })
        .collect::<Vec<_>>()
        .join("; ");
    WorkEvent::Decision {
        change: change.into(),
        what: d.text.clone(),
        status: d.status.clone(),
        why: d
            .why
            .clone()
            .or_else(|| d.evidence.as_ref().map(|e| format!("see {e}"))),
        rejected,
        verified: d.verified.clone(),
    }
}

/// Decision `d` was `change`d (`recorded`, or its new status).
pub(crate) fn after_change(vault: &Path, d: &Decision, change: &str) {
    after_change_at(vault, d, change, Utc::now());
}

pub(crate) fn after_change_at(vault: &Path, d: &Decision, change: &str, now: DateTime<Utc>) {
    let Some(key) = d.issue.as_deref().map(str::trim).filter(|k| !k.is_empty()) else {
        return;
    };
    let Some(link) = work::link_for_issue_key(vault, key) else {
        return;
    };
    match chat_work::running_mode(vault) {
        AgentWork::Off => {}
        AgentWork::DryRun => dry_run(vault, &link, d, change, now),
        AgentWork::On => {
            if !work::tracking(vault) {
                return;
            }
            match work::add_event(vault, &link.session_key, &event_of(d, change)) {
                Ok(_) => crate::tracker_sync::request_soon(),
                Err(error) => eprintln!("[tracker work] decision comment not recorded: {error}"),
            }
        }
    }
}

fn dry_run(vault: &Path, link: &WorkLink, d: &Decision, change: &str, now: DateTime<Utc>) {
    let Some(issue_id) = link.issue_id.as_deref() else {
        return;
    };
    let key = request_key(&d.path, change, now);
    let seen = crate::audit::rows_of(vault, &link.session_key, crate::audit::kind::TRACKER_WRITE)
        .iter()
        .any(|r| r.request_key.as_deref() == Some(&key));
    if seen {
        return;
    }
    work::record_tracker_write(
        vault,
        &work::TrackerWrite {
            session_key: &link.session_key,
            issue_id,
            issue_key: link.issue_key.as_deref(),
            action: "comment",
            outcome: "skipped",
            request_key: Some(&key),
            detail: Some(format!(
                "Dry run: Hollow would post a decision build log comment on {} ({change}); nothing was sent.",
                link.issue_key.as_deref().unwrap_or(issue_id)
            )),
            ..work::TrackerWrite::default()
        },
    );
}
