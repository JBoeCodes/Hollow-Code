//! The overnight installer (row U7, updates design section 8).
//!
//! With "Install updates overnight" on, a thread looks every 10 minutes
//! and, inside the local window 2:00 to 5:00 AM, installs the verified
//! channel update that waits in the Update button when all of these hold:
//! nothing runs or waits (the same `Work` checks as Restart to update, plus
//! build slots and self-hosted jobs), nothing started in the last 10
//! minutes, and the Mac is on power (`pmset -g ps`; a Mac with no battery
//! reports AC Power). Otherwise it looks again; when the window ends it
//! tries the next night. It never installs a Local build (a build someone
//! staged by hand), the skipped version, a version hidden with Not now, a
//! build from another channel or one the token may no longer use, nor while
//! a link's token waits for the person or a go back waits. The install
//! itself is [`Client::install`], so the verify-before-spawn rule, the
//! restart checks, the last look and the one-time downgrade rule are the
//! manual path's. App code only: no agent action reaches it.
//!
//! The next launch posts one Hollow notification about it
//! ([`super::last::overnight_notice`], [`notice`]).

use super::client::Client;
use super::install::{UpdateInstallOutcome, WaitKind};
use super::settings::UpdateSettings;
use super::state::{UpdatePhase, UpdateRelease};
use crate::phone_access::lid::{parse_source, Source};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// How often the thread looks.
pub(crate) const EVERY: Duration = Duration::from_secs(10 * 60);
/// Nothing may have started this recently.
pub(crate) const QUIET: Duration = Duration::from_secs(10 * 60);
/// The window, in minutes after local midnight: 2:00 up to 5:00.
pub(crate) const WINDOW: std::ops::Range<u32> = 120..300;

/// The wall time (ms) the last run was admitted, stamped by
/// `run_guard::begin_tracking` (every chat turn, automation and build
/// workspace turn passes it). 0: none in this launch.
static LAST_RUN_STARTED_MS: AtomicU64 = AtomicU64::new(0);

/// A run was admitted now.
pub(crate) fn note_run_started() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    LAST_RUN_STARTED_MS.fetch_max(now, Ordering::SeqCst);
}

/// What the overnight installer reads besides the client: local time, the
/// power source, recent starts and work the restart checks don't list.
pub(crate) trait Surroundings: Send + Sync {
    /// Minutes after local midnight at `now_ms`.
    fn local_minute(&self, now_ms: u64) -> u32;
    fn power(&self) -> Source;
    /// When the last run started (wall ms), if one did in this launch.
    fn last_started_ms(&self) -> Option<u64>;
    /// A build slot or a self-hosted job that runs now, named.
    fn other_work(&self) -> Option<String>;
}

/// Why nothing was installed on this look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Hold {
    Off,
    NothingReady,
    LocalBuild,
    OtherChannel,
    Skipped,
    NotNow,
    PendingToken,
    GoBackWaiting,
    OutOfWindow,
    Work(String),
    StartedRecently,
    OnBattery,
    PowerUnknown,
    /// The build in the button isn't the one the look decided on (replaced
    /// by a newer check, or a Local build) by the time of the install.
    Changed,
}

/// Prefix of [`Client::install`]'s refusal when [`recheck`] holds it.
const RECHECK: &str = "OVERNIGHT_HOLD: ";

/// What [`Client::install`] reads again for the overnight installer, under
/// its lock both before verifying and at the claim (review: the look's
/// facts can go stale while codesign and the work checks run): the same
/// version as the look, still a channel build, the setting still on, not
/// skipped, not hidden with Not now, and the channel unchanged and allowed.
pub(crate) fn recheck(
    release: &UpdateRelease,
    expected: &str,
    s: &UpdateSettings,
) -> Result<(), Hold> {
    if release.version != expected || release.local.is_some() {
        return Err(Hold::Changed);
    }
    if !s.install_overnight {
        return Err(Hold::Off);
    }
    if release.channel != s.channel || !super::people::allowed(s).contains(&s.channel) {
        return Err(Hold::OtherChannel);
    }
    if s.skipped_version.as_deref() == Some(expected) {
        return Err(Hold::Skipped);
    }
    if s.dismissed_version.as_deref() == Some(expected) {
        return Err(Hold::NotNow);
    }
    Ok(())
}

/// The holds [`recheck`] gives, for reading its refusal back.
const RECHECK_HOLDS: [Hold; 5] = [
    Hold::Changed,
    Hold::Off,
    Hold::OtherChannel,
    Hold::Skipped,
    Hold::NotNow,
];

fn recheck_words(hold: &Hold) -> String {
    format!("{RECHECK}{hold:?}")
}

/// The facts one look decides on (all but power, read last and only when
/// everything else allows an install, so no `pmset` runs by day).
#[derive(Debug, Clone)]
pub(crate) struct Facts {
    pub(crate) enabled: bool,
    /// The release in the button, when the phase is `ready`.
    pub(crate) ready: Option<UpdateRelease>,
    pub(crate) channel: super::state::Channel,
    pub(crate) channel_allowed: bool,
    pub(crate) skipped: Option<String>,
    pub(crate) dismissed: bool,
    pub(crate) pending_token: bool,
    pub(crate) go_back_waiting: bool,
    pub(crate) minute: u32,
    /// What runs, named (`None`: nothing).
    pub(crate) work: Option<String>,
    pub(crate) since_last_start: Option<Duration>,
}

/// The rule, pure. `power` is asked only when everything else allows.
pub(crate) fn decide(f: &Facts, power: impl FnOnce() -> Source) -> Result<(), Hold> {
    if !f.enabled {
        return Err(Hold::Off);
    }
    let Some(release) = &f.ready else {
        return Err(Hold::NothingReady);
    };
    if release.local.is_some() {
        return Err(Hold::LocalBuild);
    }
    if release.channel != f.channel || !f.channel_allowed {
        return Err(Hold::OtherChannel);
    }
    if f.skipped.as_deref() == Some(release.version.as_str()) {
        return Err(Hold::Skipped);
    }
    if f.dismissed {
        return Err(Hold::NotNow);
    }
    if f.pending_token {
        return Err(Hold::PendingToken);
    }
    if f.go_back_waiting {
        return Err(Hold::GoBackWaiting);
    }
    if !WINDOW.contains(&f.minute) {
        return Err(Hold::OutOfWindow);
    }
    if let Some(what) = &f.work {
        return Err(Hold::Work(what.clone()));
    }
    if f.since_last_start.is_some_and(|ago| ago < QUIET) {
        return Err(Hold::StartedRecently);
    }
    match power() {
        Source::Power => Ok(()),
        Source::Battery => Err(Hold::OnBattery),
        Source::Unknown => Err(Hold::PowerUnknown),
    }
}

/// What one look did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Look {
    Held(Hold),
    /// The installer started; Hollow quits in a moment (unless the last
    /// look before the quit calls it off, and the build is ready again).
    Started(String),
    /// The install path refused (its own checks): the words.
    Refused(String),
}

impl Client {
    /// One overnight look (the thread calls it every [`EVERY`]).
    pub(crate) fn overnight_look(self: &Arc<Self>, env: &dyn Surroundings) -> Look {
        let now_ms = self.d.clock.now_ms();
        let facts = self.overnight_facts(env, now_ms);
        if let Err(hold) = decide(&facts, || env.power()) {
            return Look::Held(hold);
        }
        // Turned off since the facts were read: nothing.
        if !self.settings.read().install_overnight {
            return Look::Held(Hold::Off);
        }
        let Some(expected) = facts.ready.as_ref().map(|r| r.version.clone()) else {
            return Look::Held(Hold::NothingReady);
        };
        // The manual path, armed for a channel update only (`None`): a
        // Local build that replaced it meanwhile is refused (review B1).
        // It reads the version, skip, Not now and channel again under its
        // lock ([`recheck`]).
        match self.install(None, Some(&None), Some(&expected)) {
            Ok(UpdateInstallOutcome::Started { version }) => Look::Started(version),
            Ok(
                UpdateInstallOutcome::WorkRunning { work }
                | UpdateInstallOutcome::WorkChanged { work },
            ) => Look::Held(Hold::Work(names(work))),
            Err(why) if why == super::client::NO_READY => Look::Held(Hold::NothingReady),
            Err(why) => match RECHECK_HOLDS.into_iter().find(|h| why == recheck_words(h)) {
                Some(hold) => Look::Held(hold),
                None => Look::Refused(why),
            },
        }
    }

    /// [`recheck`] for [`Client::install`] (`expected`: the overnight
    /// installer's version; `None`, a person's install: nothing to check).
    pub(super) fn overnight_recheck(
        &self,
        release: &UpdateRelease,
        expected: Option<&str>,
    ) -> Result<(), String> {
        match expected {
            Some(expected) => {
                recheck(release, expected, &self.settings.read()).map_err(|h| recheck_words(&h))
            }
            None => Ok(()),
        }
    }

    fn overnight_facts(&self, env: &dyn Surroundings, now_ms: u64) -> Facts {
        let settings = self.settings.read();
        let state = self.state();
        let ready = match state.phase {
            UpdatePhase::Ready { release } => Some(release),
            _ => None,
        };
        let work = match self.d.work.running() {
            Ok(running) if running.is_empty() => self.d.work.blocked().or_else(|| env.other_work()),
            Ok(running) => Some(names(running)),
            // A list that can't be read counts as work (fail closed).
            Err(why) => Some(why),
        };
        Facts {
            enabled: settings.install_overnight,
            channel_allowed: state.allowed_channels.contains(&settings.channel),
            channel: settings.channel,
            ready,
            skipped: settings.skipped_version,
            dismissed: state.dismissed,
            pending_token: state.pending_connect,
            go_back_waiting: self.waiting_for(Some(WaitKind::GoBack)),
            minute: env.local_minute(now_ms),
            work,
            since_last_start: env
                .last_started_ms()
                .map(|at| Duration::from_millis(now_ms.saturating_sub(at))),
        }
    }

    /// Look every [`EVERY`] (or at once after a settings change), `rounds`
    /// times (tests), or for good (`None`). It keeps looking after an
    /// install starts: the app's quit ends the thread, and when the last
    /// look before the quit calls the install off (work started), the build
    /// is ready again and a later look installs it (review: stopping here
    /// missed every later night).
    pub(crate) fn run_overnight(
        self: &Arc<Self>,
        env: &dyn Surroundings,
        every: Duration,
        rounds: Option<usize>,
    ) -> Vec<Look> {
        let mut looks = Vec::new();
        loop {
            let look = self.overnight_look(env);
            if let Look::Refused(why) = &look {
                eprintln!("[updates] the overnight install didn't start: {why}");
            }
            looks.push(look);
            if rounds.is_some_and(|r| looks.len() >= r) {
                return looks;
            }
            self.overnight_nap(every);
        }
    }

    /// Wake the overnight thread (the setting changed).
    pub(crate) fn wake_overnight(&self) {
        *self
            .overnight_wake
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = true;
        self.overnight_cv.notify_all();
    }

    fn overnight_nap(&self, d: Duration) {
        if self.d.naps == super::client::Naps::Clock {
            return self.d.clock.sleep(d);
        }
        let deadline = std::time::Instant::now() + d;
        let mut woken = self
            .overnight_wake
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        while !*woken {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            woken = self
                .overnight_cv
                .wait_timeout(woken, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *woken = false;
    }
}

fn names(work: Vec<super::install::UpdateWork>) -> String {
    work.into_iter()
        .map(|w| w.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The app's surroundings: local time from `chrono`, `pmset -g ps`, the
/// run stamp above, `hollow-slot`'s locks and the self-hosted job queue.
pub(crate) struct Mac;

impl Surroundings for Mac {
    fn local_minute(&self, now_ms: u64) -> u32 {
        use chrono::{TimeZone, Timelike};
        chrono::Local
            .timestamp_millis_opt(now_ms as i64)
            .single()
            .map(|t| t.hour() * 60 + t.minute())
            // A time that can't be placed is outside the window.
            .unwrap_or(u32::MAX)
    }

    fn power(&self) -> Source {
        std::process::Command::new("/usr/bin/pmset")
            .args(["-g", "ps"])
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| parse_source(&String::from_utf8_lossy(&out.stdout)).0)
            .unwrap_or(Source::Unknown)
    }

    fn last_started_ms(&self) -> Option<u64> {
        Some(LAST_RUN_STARTED_MS.load(Ordering::SeqCst)).filter(|ms| *ms > 0)
    }

    fn other_work(&self) -> Option<String> {
        let slots = crate::build_slots::status();
        if slots.untrusted.is_none()
            && slots
                .slots
                .iter()
                .chain([&slots.cargo, &slots.cargo_lane_1])
                .any(|lock| lock.held)
        {
            return Some("A build job".into());
        }
        crate::self_hosted_jobs::view()
            .waiting
            .iter()
            .any(|w| w.draining)
            .then(|| "A self-hosted job".into())
    }
}

/// The morning notification (title, body, kind), plain text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Notice {
    pub(crate) title: String,
    pub(crate) body: String,
    /// `success` or `warning` (`notifications::Notification::kind`).
    pub(crate) kind: &'static str,
}

/// Longest notes in the notification body.
const NOTES_MAX: usize = 1500;
/// Longest reason (from the installer's journal or helper) in the body.
const REASON_MAX: usize = 300;

/// A reason as the notification shows it: the home folder written `~`
/// (it can name the person's account), cut at [`REASON_MAX`] characters.
pub(crate) fn plain_reason(why: &str, home: Option<&str>) -> String {
    let mut why = why.trim().to_string();
    if let Some(home) = home
        .map(|h| h.trim_end_matches('/'))
        .filter(|h| !h.is_empty())
    {
        why = why.replace(home, "~");
    }
    if why.chars().count() > REASON_MAX {
        let cut: String = why.chars().take(REASON_MAX).collect();
        format!("{}…", cut.trim_end())
    } else {
        why
    }
}

fn home() -> Option<String> {
    std::env::var("HOME").ok()
}

pub(crate) fn notice(result: &super::last::UpdateLastResult, running: &str) -> Notice {
    use super::last::UpdateLastResult as R;
    match result {
        R::Updated { version, notes, .. } => {
            let notes = notes.trim();
            let body = if notes.is_empty() {
                "No notes came with this update.".to_string()
            } else if notes.chars().count() > NOTES_MAX {
                let cut: String = notes.chars().take(NOTES_MAX).collect();
                format!("{cut}…")
            } else {
                notes.to_string()
            };
            Notice {
                title: format!("Hollow updated overnight to {version}"),
                body,
                kind: "success",
            }
        }
        R::WentBack {
            attempted,
            restored,
            reason,
            ..
        } => Notice {
            title: format!("The overnight update to {attempted} was rolled back"),
            body: format!(
                "Hollow went back to {}.{}",
                restored.as_deref().unwrap_or(running),
                match plain_reason(reason, home().as_deref()) {
                    r if r.is_empty() => String::new(),
                    r => format!(" {r}"),
                }
            ),
            kind: "warning",
        },
        R::NotInstalled {
            attempted, reason, ..
        } => Notice {
            title: format!("The overnight update to {attempted} didn't install"),
            body: match reason
                .as_deref()
                .map(|why| plain_reason(why, home().as_deref()))
            {
                Some(why) if !why.is_empty() => format!("Hollow is still on {running}. {why}"),
                _ => format!("Hollow is still on {running}."),
            },
            kind: "warning",
        },
        // A go back is never overnight; worded plainly all the same.
        R::NotWentBack { .. } => Notice {
            title: "Hollow didn't update overnight".into(),
            body: format!("Hollow is on {running}."),
            kind: "warning",
        },
    }
}
