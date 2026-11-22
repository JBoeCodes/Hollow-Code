//! The update client's state machine and the offer rule, both pure.
//!
//! Phases: `notSetUp` (inert, with the reason), `idle`, `checking`,
//! `upToDate`, `available`, `downloading`, `ready` (downloaded, its minisign
//! signature, unpack and the app's own codesign, team, bundle id and version
//! checks all passed), `installing` and `failed`. The frontend shows the
//! Update button only in `ready`, so it appears only for a verified download.

use crate::update_install::verify::compare_versions;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// An update channel (design section 2): Nightly gets every merged change
/// that passes its checks, Main only what Alex publishes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(rename = "UpdateChannel"))]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    #[default]
    Main,
    Nightly,
}

impl Channel {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Channel::Main => "main",
            Channel::Nightly => "nightly",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Channel::Main => "Main",
            Channel::Nightly => "Nightly",
        }
    }
}

/// One published build, as the window shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateRelease {
    pub version: String,
    /// What's new, plain text (the manifest's `notes`; empty when absent).
    pub notes: String,
    /// When it was published (RFC 3339), when the manifest says.
    pub pub_date: Option<String>,
    pub channel: Channel,
    /// The built commit (40 hex), when the manifest says.
    pub commit: Option<String>,
    /// Lower than the running version: offered once after a channel switch.
    pub downgrade: bool,
    /// A finished branch build from a build workspace (row U10), shown as
    /// "Local build of <branch>"; `None` for a channel build.
    pub local: Option<super::local::UpdateLocalBuild>,
}

/// Where the client is. Internally tagged by `phase`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "phase", rename_all = "camelCase")]
pub enum UpdatePhase {
    /// Inert: no public key, host or token, or a review copy.
    NotSetUp {
        reason: String,
    },
    /// Nothing checked yet in this launch.
    Idle,
    Checking,
    /// Nothing to offer. `skipped` names a newest version the person went
    /// back from, which is not offered until a newer one is published.
    UpToDate {
        skipped: Option<String>,
    },
    /// Found, not yet downloaded.
    Available {
        release: UpdateRelease,
    },
    /// `percent` is `None` while the size is unknown.
    Downloading {
        release: UpdateRelease,
        percent: Option<u8>,
    },
    /// Downloaded and verified: the only phase with an Update button.
    Ready {
        release: UpdateRelease,
    },
    /// The installer was started; Hollow quits in a moment.
    Installing {
        release: UpdateRelease,
    },
    /// Go back (row U6) started the installer to put `version` back;
    /// Hollow quits in a moment. Like `installing`, nothing else starts.
    GoingBack {
        version: String,
    },
    /// The last check, download or install failed; `reason` is for a person.
    Failed {
        reason: String,
    },
}

/// The whole state the window gets (`update_get_state`, the `update-state`
/// event). Never carries the token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    pub phase: UpdatePhase,
    /// The running version (`build-provenance.json`).
    pub running_version: String,
    pub channel: Channel,
    /// The build has a public key, host and team (the token aside).
    pub configured: bool,
    /// A token is stored in the Keychain.
    pub connected: bool,
    /// A Connect Hollow link brought a different token than the stored one:
    /// it waits (in memory only) for the person's `update_confirm_connect`
    /// or `update_discard_connect`.
    pub pending_connect: bool,
    /// "Update when chats finish" is waiting (in memory only: gone if
    /// Hollow quits).
    pub waiting: bool,
    /// Row U6: "Go back when chats finish" is waiting (in memory only).
    /// `update_cancel_wait` cancels it too.
    pub go_back_waiting: bool,
    /// The ready version was hidden with "Not now".
    pub dismissed: bool,
    pub skipped_version: Option<String>,
    /// A channel switch allows one lower version.
    pub downgrade_allowed: bool,
    /// The last finished check (RFC 3339).
    pub last_checked_at: Option<String>,
    /// A sentence to show beside the phase (a channel switch, an offline
    /// check that kept the ready download), or `None`.
    pub note: Option<String>,
    /// Row U9: the channels the picker may offer (what the update host's
    /// `/me` last said; both until it answers). Empty: none.
    pub allowed_channels: Vec<Channel>,
    /// Row U9: whose token this is (`/me`), or `None` until it answers.
    pub person_name: Option<String>,
}

/// What happened, for [`apply`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Event {
    Unconfigured(String),
    CheckStarted,
    NothingNew {
        skipped: Option<String>,
    },
    Offered(UpdateRelease),
    Progress(Option<u8>),
    Verified(UpdateRelease),
    InstallStarted,
    Failed(String),
    /// A failed install refusal or a cancelled wait: back to `ready`.
    BackToReady,
}

/// The next phase. A ready download stays ready (and its button stays)
/// while a check runs; progress and install only follow their own phases.
pub(crate) fn apply(phase: &UpdatePhase, event: Event) -> UpdatePhase {
    use UpdatePhase as P;
    match (phase, event) {
        // A go back that started is ended only by the go back itself
        // (row U6), which sets the phase directly.
        (P::GoingBack { .. }, _) => phase.clone(),
        // Nothing but its own failure (or refusal) interrupts an install
        // that started.
        (P::Installing { .. }, Event::Failed(reason)) => P::Failed { reason },
        (P::Installing { release }, Event::BackToReady) => P::Ready {
            release: release.clone(),
        },
        (P::Installing { .. }, _) => phase.clone(),
        (_, Event::Unconfigured(reason)) => P::NotSetUp { reason },
        (P::Ready { .. }, Event::CheckStarted) => phase.clone(),
        (P::Downloading { .. }, Event::CheckStarted) => phase.clone(),
        (_, Event::CheckStarted) => P::Checking,
        (_, Event::NothingNew { skipped }) => P::UpToDate { skipped },
        (_, Event::Offered(release)) => P::Available { release },
        (P::Available { release } | P::Downloading { release, .. }, Event::Progress(percent)) => {
            P::Downloading {
                release: release.clone(),
                percent,
            }
        }
        (_, Event::Progress(_)) => phase.clone(),
        (_, Event::Verified(release)) => P::Ready { release },
        (P::Ready { release }, Event::InstallStarted) => P::Installing {
            release: release.clone(),
        },
        (_, Event::InstallStarted) => phase.clone(),
        (_, Event::Failed(reason)) => P::Failed { reason },
        (_, Event::BackToReady) => phase.clone(),
    }
}

impl UpdatePhase {
    /// An install or a go back is starting: nothing else may begin.
    pub(crate) fn busy(&self) -> bool {
        matches!(self, Self::Installing { .. } | Self::GoingBack { .. })
    }
}

/// Whether a published version is offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decision {
    /// Higher than the running version, or lower once after a channel switch.
    Offer { downgrade: bool },
    /// The running version, a lower one, or one that doesn't compare.
    Current,
    /// The version the person went back from (never offered until a newer
    /// one is published).
    Skipped(String),
}

/// The offer rule (design sections 2 and 9): higher is offered; equal or
/// lower is not, except a lower one while a channel switch allows it once;
/// the skipped version never is; versions that don't compare never are.
pub(crate) fn decide(
    running: &str,
    published: &str,
    skipped: Option<&str>,
    downgrade_allowed: bool,
) -> Decision {
    if skipped == Some(published) {
        return Decision::Skipped(published.to_string());
    }
    match compare_versions(published, running) {
        Some(Ordering::Greater) => Decision::Offer { downgrade: false },
        Some(Ordering::Less) if downgrade_allowed => Decision::Offer { downgrade: true },
        _ => Decision::Current,
    }
}

/// A version as the manifest and the installer use it: dotted numbers,
/// 2 to 4 parts, at most 32 characters (it also names a staging folder).
pub(crate) fn valid_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    version.len() <= 32
        && (2..=4).contains(&parts.len())
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()))
}
