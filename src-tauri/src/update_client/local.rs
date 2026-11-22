//! Local builds (row U10, updates design section 7): a finished branch
//! build from a build workspace's Changes, Hollow app (**Build app**) is
//! offered in the top bar's Update button as "Local build of <branch>" and
//! installed by the same `hollow install-update` helper as a channel update,
//! under the same restart checks and the same no-expiry "Update when chats
//! finish" wait. The build workspace's old Install and restart card, its
//! banner and the 30-minute waiter are gone.
//!
//! What differs from a channel update, all decided here:
//!
//! - **A copy of its own.** The built `Hollow.app` is copied (an APFS
//!   clone, `ditto`) into `updates/local/<build id>/` when it is offered,
//!   and that copy is hashed, checked and handed to the installer as
//!   `--staged`, so a rebuild or anything else writing the build target
//!   can't change the bytes between the hash and the install.
//! - **The bytes are the receipt's.** The copy is hashed
//!   (`self_install::bundle_digest`) before it is offered and again before
//!   the installer starts: a bundle that changed since the build recorded
//!   its receipt is refused, as the old install did (review A·S4/B·F12).
//! - **The team** ([`expected_team`]): local builds are signed on this Mac
//!   (the Developer ID, or "Hollow Dev Signing" until U0) and not notarized.
//!   The installer expects the bundle's own team only when it equals the
//!   configured team (`HOLLOW_UPDATE_TEAM`, else TEAM000000), which then
//!   must pass the Developer ID requirement too; any other bundle is checked
//!   as ad hoc (`--expected-team -`), so a self-signed build without a team
//!   passes and a build signed by a different team is refused.
//! - **Always `--allow-downgrade`**: a branch build may carry a lower
//!   version than a Nightly the person installed.
//! - **Channel checks pause** while a local build waits in the button; Not
//!   now, a new build, or the install itself ends that. A channel update
//!   that was ready is kept aside and is back in the button when the local
//!   build leaves it (same channel, a downgrade still allowed).
//! - **"Update when chats finish" is bound to what it was armed for**: a
//!   Local build offered while a channel update's wait runs ends that wait
//!   (and the other way round), so it never installs a build the person
//!   didn't choose.
//! - **"Updated" means this build runs**: the version alone can't tell a
//!   branch build of the same version from the app it replaced, so the
//!   notice and the receipt need the running app's source fingerprint (or,
//!   without one, its clean commit) to be the build's ([`is_running`]).
//! - **The build receipt** (`self_install_receipt`, `last-result.json`)
//!   still records the install: [`settle_receipt`] writes it at the next
//!   launch from the installer's outcome, already acknowledged, because the
//!   Update button's own notice says "Updated to the local build of …".

use crate::install_brings::Report;
use crate::self_install::{BuildReceipt, InstallResult, InstallState, InstallerReport};
use crate::update_install::verify::Team;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// What the window shows for a local build (`UpdateRelease.local`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateLocalBuild {
    pub workspace_id: String,
    /// The branch it was built from ("Local build of <branch>").
    pub branch: String,
    /// The build's receipt id.
    pub build_id: String,
    /// Built with uncommitted changes.
    pub dirty: bool,
}

/// A finished build to offer ([`super::client::Client::offer_local`]).
#[derive(Debug, Clone)]
pub(crate) struct LocalOffer {
    pub(crate) receipt: BuildReceipt,
    pub(crate) branch: String,
    /// What's new, plain text ([`notes`]).
    pub(crate) notes: String,
    /// The person (or an approved agent install) asked for this build, so
    /// a Not now on it no longer hides it.
    pub(crate) asked: bool,
}

impl LocalOffer {
    pub(crate) fn view(&self) -> UpdateLocalBuild {
        UpdateLocalBuild {
            workspace_id: self.receipt.workspace_id.clone(),
            branch: self.branch.clone(),
            build_id: self.receipt.build_id.clone(),
            dirty: self.receipt.dirty,
        }
    }
}

/// What `last-update.json` keeps of a local install, so the next launch
/// can settle its build receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LocalRecord {
    pub(crate) receipt: BuildReceipt,
    pub(crate) branch: String,
}

pub(crate) const REVIEW_COPY: &str =
    "This is a review copy of Hollow. Install local builds from the everyday Hollow.";
pub(crate) const CHANGED: &str =
    "The built app changed after its build finished, so Hollow won't install it. Build it again.";

/// The team the installer expects of a local build: the bundle's own team
/// when it equals the configured one (and then the Developer ID requirement
/// applies), else ad hoc (which a bundle signed by another team fails).
pub(crate) fn expected_team(bundle_team: Option<&str>, configured: Option<&Team>) -> Team {
    match (bundle_team, configured) {
        (Some(got), Some(Team::Id(want))) if got == want => Team::Id(want.clone()),
        _ => Team::AdHoc,
    }
}

/// Whether the running app is the build `receipt` recorded: the same source
/// fingerprint when both have one, else the same commit (at least 7
/// characters, one a prefix of the other) with neither built dirty.
pub(crate) fn is_running(receipt: &BuildReceipt, running: &crate::app_menu::BuildIdentity) -> bool {
    let (want, got) = (&receipt.source_fingerprint, &running.source_fingerprint);
    if !want.is_empty() && !got.is_empty() {
        return want == got;
    }
    let (a, b) = (receipt.commit.as_str(), running.commit.as_str());
    !receipt.dirty
        && !running.dirty
        && a.len().min(b.len()) >= 7
        && (a.starts_with(b) || b.starts_with(a))
}

/// How many pull requests and commits the notes list before "and N more".
const NOTE_ROWS: usize = 30;

/// What's new for a local build, from `install_brings` (pull requests and
/// direct commits between the installed commit and the build), as plain
/// text for the Update button's notes.
pub(crate) fn notes(report: &Report) -> String {
    let plural = |n: usize, noun: &str| format!("{n} {noun}{}", if n == 1 { "" } else { "s" });
    let mut out = Vec::new();
    if !report.pull_requests.is_empty() || !report.commits.is_empty() {
        out.push(format!(
            "Brings {} and {} since the installed build.",
            plural(report.pull_requests.len(), "pull request"),
            plural(report.commits.len(), "direct commit")
        ));
    }
    let rows: Vec<String> = report
        .pull_requests
        .iter()
        .map(|pr| format!("#{} {}", pr.number, pr.title))
        .chain(report.commits.iter().map(|c| {
            let sha: String = c.sha.chars().take(8).collect();
            format!("{sha} {}", c.subject)
        }))
        .collect();
    let more = rows.len().saturating_sub(NOTE_ROWS);
    out.extend(rows.into_iter().take(NOTE_ROWS));
    if more > 0 {
        out.push(format!("and {more} more"));
    }
    if report.missing > 0 {
        out.push(format!(
            "This build drops {} from the installed build.",
            plural(report.missing, "commit")
        ));
    }
    if let Some(note) = report.note.as_deref().filter(|n| !n.trim().is_empty()) {
        out.push(note.trim().to_string());
    }
    out.join("\n")
}

/// After the restart: write the local install's outcome to the build
/// receipt (`last-result.json` in `dir`) once, acknowledged, so agents'
/// `self_install_receipt` and the Hollow repository lookup still see which
/// build went in, and the old notice doesn't repeat the Update button's.
/// Returns whether it wrote. A `last-result.json` already about this
/// install is left alone.
pub(crate) fn settle_receipt(
    dir: &Path,
    install_id: &str,
    local: &LocalRecord,
    outcome: &super::last::UpdateLastResult,
    log: &Path,
) -> Result<bool, String> {
    use super::last::UpdateLastResult as R;
    if crate::self_install::result_id_in(dir).as_deref() == Some(install_id) {
        return Ok(false);
    }
    let (ok, installed, reason, at) = match outcome {
        R::Updated { at, .. } => (true, "yes", String::new(), at.clone()),
        R::WentBack { reason, at, .. } => (false, "no", reason.clone(), at.clone()),
        // A go back's record never carries a Local build; read one as not
        // installed all the same.
        R::NotInstalled { reason, at, .. } | R::NotWentBack { reason, at, .. } => (
            false,
            "no",
            reason
                .clone()
                .unwrap_or_else(|| "The installer didn't swap the app.".into()),
            at.clone(),
        ),
    };
    let short = |s: &str| s.chars().take(12).collect::<String>();
    let result = InstallResult {
        id: install_id.to_string(),
        ok,
        started_at: at.clone(),
        finished_at: at,
        commit: short(&local.receipt.commit),
        branch: local.branch.clone(),
        log: log.to_string_lossy().into(),
        message: reason.clone(),
        installer_ran: Some(true),
        installer: Some(InstallerReport {
            ok,
            reason,
            installed_commit: if ok {
                local.receipt.commit.clone()
            } else {
                String::new()
            },
            installed: installed.into(),
            install_id: install_id.to_string(),
            ..InstallerReport::default()
        }),
        state: if ok {
            InstallState::Installed
        } else {
            InstallState::Failed
        },
        build: Some(local.receipt.clone()),
    };
    crate::self_install::write_result_in(dir, &result)?;
    crate::self_install::acknowledge_in(dir, install_id)?;
    Ok(true)
}
