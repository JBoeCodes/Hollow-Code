//! The client: one per app, every effect behind a trait ([`Deps`]), so the
//! tests drive it with fakes, temp folders and an instant clock.
//!
//! A check ([`Client::check`]): inert with the reason when not set up; else
//! ask the channel, check the manifest (`manifest.rs`), apply the offer rule
//! (`state::decide`); reuse a staged build already verified for that
//! version, or download it (the plugin checks its signature), unpack it
//! (`unpack.rs`), verify the bundle with the installer's own checks
//! (`update_install::verify`: no escaping symlinks, `codesign --verify`, the
//! Developer ID team, bundle id, version equal to the manifest's) and only
//! then become `ready`. Every phase change is emitted as `update-state`.
//!
//! **Epochs.** A channel switch, a connect and a disconnect bump `epoch`; a
//! check or download that started under an older epoch drops its result and
//! queues one more check. Nothing prunes or discards staged files while an
//! install is starting.
//!
//! **The Keychain.** [`Client::new`] reads nothing; the token is first read
//! by [`Client::load_connection`] on the schedule thread (`secret_store`
//! caches its one item per process, but its first read can prompt).

use super::config::{self, Config};
use super::fetch::{CheckError, Fetcher, Pending};
use super::install::{
    self, HelperArgs, UpdateInstallOutcome, UpdateWork, WaitAction, WaitEnd, WaitKind,
};
use super::last::{self, UpdateLastResult};
use super::local::{self, LocalOffer, LocalRecord};
use super::manifest;
use super::settings::{self, UpdateSettings};
use super::stage::Staging;
use super::state::{
    apply, decide, Channel, Decision, Event, UpdatePhase, UpdateRelease, UpdateState,
};
use super::token::{self, TokenStore};
use super::unpack::Limits;
use crate::update_install::effects::{Clock, Signer};
use crate::update_install::verify::{self, BundleInfo, Team};
use crate::update_install::Layout;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The state event (payload [`UpdateState`]).
pub(crate) const EVENT: &str = "update-state";
/// The after-restart notice event (payload [`UpdateLastResult`]).
pub(crate) const LAST_RESULT_EVENT: &str = "update-last-result";
/// The first check waits this long after launch, so startup isn't slowed.
pub(crate) const LAUNCH_DELAY: Duration = Duration::from_secs(45);
/// Then a check every 30 minutes.
pub(crate) const EVERY: Duration = Duration::from_secs(30 * 60);
/// A refused token doubles the wait up to this.
pub(crate) const MAX_BACKOFF: Duration = Duration::from_secs(4 * 60 * 60);

pub(crate) const NO_READY: &str = "No verified update is ready to install.";
pub(crate) const INSTALLING: &str = "An update is installing; change this after Hollow reopens.";
pub(crate) const DOWNGRADE_EXPIRED: &str = "The older build was offered for 24 hours after the channel switch; that time is over, so Hollow checked the channel again.";
pub(crate) const NOT_AN_APP: &str =
    "This Hollow isn't running from an installed Hollow.app, so it can't install updates.";

/// Where the window hears about changes.
pub(crate) trait Sink: Send + Sync {
    fn state(&self, state: &UpdateState);
}

/// What runs now, as the restart checks see it (`lifecycle`).
pub(crate) trait Work: Send + Sync {
    /// What a quit would end.
    fn running(&self) -> Result<Vec<UpdateWork>, String>;
    /// `lifecycle::check_restart_work`: these keys still cover what runs.
    fn confirm(&self, keys: &[String]) -> Result<(), String>;
    /// `self_install::install_blocked`: a candidate build, a vault lease, a
    /// build workspace turn or a running automation.
    fn blocked(&self) -> Option<String>;
}

/// A started installer.
pub(crate) trait Started: Send {
    /// Watch it briefly (about 500 ms): the words for a person if it already
    /// exited, `None` while it runs (waiting for Hollow to quit).
    fn exited_early(&mut self) -> Option<String>;
    /// One look (`try_wait`): the words if it has exited.
    fn exited(&mut self) -> Option<String>;
    /// Stop it: Hollow decided not to quit. It has moved nothing yet (it
    /// waits for the app to quit before its journal), and its lock is a dead
    /// holder's for the next run.
    fn stop(&mut self);
}

/// Starting the installer and quitting.
pub(crate) trait Helper: Send + Sync {
    fn spawn(&self, exe: &Path, args: &[OsString], log: &Path) -> Result<Box<dyn Started>, String>;
    /// Quit Hollow a moment from now, unless `last_look` (run just before
    /// the quit) says no.
    fn quit(&self, last_look: Box<dyn FnOnce() -> bool + Send>);
}

pub(crate) struct Paths {
    pub(crate) settings: PathBuf,
    pub(crate) staging: PathBuf,
    pub(crate) last: PathBuf,
    /// The running bundle (`bundle_of(current_exe)`); `None` in a dev run.
    pub(crate) installed_app: Option<PathBuf>,
    /// The installer's journal folder.
    pub(crate) state_dir: Option<PathBuf>,
    /// The Sessions folder to snapshot, when it exists.
    pub(crate) sessions: Option<PathBuf>,
}

pub(crate) struct Deps {
    pub(crate) config: Config,
    pub(crate) paths: Paths,
    pub(crate) fetcher: Box<dyn Fetcher>,
    pub(crate) tokens: Box<dyn TokenStore>,
    pub(crate) signer: Box<dyn Signer + Send + Sync>,
    pub(crate) work: Box<dyn Work>,
    pub(crate) helper: Box<dyn Helper>,
    pub(crate) clock: Box<dyn Clock + Send + Sync>,
    pub(crate) sink: Box<dyn Sink>,
    pub(crate) limits: Limits,
    pub(crate) own_pid: u32,
    /// How the schedule waits: on the clock (tests), or on a condvar that
    /// [`Client::wake_schedule`] interrupts (the app).
    pub(crate) naps: Naps,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Naps {
    Clock,
    Interruptible,
}

#[derive(Debug, Clone)]
struct Staged {
    release: UpdateRelease,
    bundle: PathBuf,
    /// A Local build (row U10): its receipt binds the bytes, and channel
    /// checks pause while it waits.
    local: Option<LocalOffer>,
}

impl Staged {
    fn is_local(s: &Option<Staged>) -> bool {
        s.as_ref().is_some_and(|s| s.local.is_some())
    }

    /// What "Update when chats finish" was armed for: `None` a channel
    /// update (a newer one from a check keeps the wait), `Some(id)` that
    /// Local build only.
    fn wait_for(s: &Option<Staged>) -> WaitFor {
        s.as_ref()
            .and_then(|s| s.local.as_ref())
            .map(|l| l.receipt.build_id.clone())
    }
}

/// See [`Staged::wait_for`].
pub(crate) type WaitFor = Option<String>;

pub(super) struct Inner {
    pub(super) phase: UpdatePhase,
    staged: Option<Staged>,
    pub(super) note: Option<String>,
    connected: bool,
    /// A link's token that differs from the stored one, until the person
    /// confirms or discards it. Memory only: never in IPC, events or files.
    pending_token: Option<String>,
    /// Row U9: why the channel moved or can't be checked (`people.rs`).
    /// Shown as `note` when nothing else is, until the person picks a
    /// channel, disconnects, or access comes back.
    access_note: Option<String>,
    /// The channel update a Local build replaced in the button (review M3):
    /// it comes back when the Local build is withdrawn, on the same channel.
    shelved: Option<Staged>,
}

pub(crate) struct Client {
    pub(super) d: Deps,
    pub(super) settings: settings::Store,
    staging: Staging,
    inner: Mutex<Inner>,
    busy: AtomicBool,
    /// Another check was asked for while one ran, or a check's result went
    /// stale: run once more.
    requeue: AtomicBool,
    epoch: AtomicU64,
    /// The next wait of the schedule, in ms (doubles on a refused token).
    next_wait_ms: AtomicU64,
    /// The running "when chats finish" wait: its ticket and what it does
    /// (an update, or row U6's go back). One at a time.
    wait: Mutex<Option<(Arc<AtomicU8>, WaitAction)>>,
    /// Set by [`Client::wake_schedule`]: the schedule's nap ends at once.
    wake: Mutex<bool>,
    wake_cv: std::sync::Condvar,
    /// Row U7: the overnight thread's nap, ended by a settings change.
    pub(super) overnight_wake: Mutex<bool>,
    pub(super) overnight_cv: std::sync::Condvar,
}

/// Clears the busy flag however a check ends.
struct Busy<'a>(&'a AtomicBool);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Client {
    /// Reads no Keychain item (see [`Client::load_connection`]).
    pub(crate) fn new(d: Deps) -> Self {
        let settings = settings::Store::new(d.paths.settings.clone());
        let phase = match d
            .config
            .resolve(settings.read().channel, Some(placeholder()))
        {
            Ok(_) => UpdatePhase::Idle,
            Err(reason) => UpdatePhase::NotSetUp { reason },
        };
        Self {
            staging: Staging::new(d.paths.staging.clone()),
            settings,
            inner: Mutex::new(Inner {
                phase,
                staged: None,
                note: None,
                connected: false,
                pending_token: None,
                access_note: None,
                shelved: None,
            }),
            busy: AtomicBool::new(false),
            requeue: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            next_wait_ms: AtomicU64::new(EVERY.as_millis() as u64),
            wait: Mutex::new(None),
            wake: Mutex::new(false),
            wake_cv: std::sync::Condvar::new(),
            overnight_wake: Mutex::new(false),
            overnight_cv: std::sync::Condvar::new(),
            d,
        }
    }

    /// Whether a token is stored (the first Keychain read, off the main
    /// thread).
    pub(crate) fn load_connection(&self) {
        let connected = self
            .d
            .tokens
            .get()
            .ok()
            .flatten()
            .is_some_and(|t| token::valid_shape(&t));
        {
            let mut inner = self.lock();
            inner.connected = connected;
            if !connected && matches!(inner.phase, UpdatePhase::Idle) {
                inner.phase = UpdatePhase::NotSetUp {
                    reason: config::NOT_CONNECTED.into(),
                };
            }
        }
        self.emit();
    }

    fn installing(&self) -> bool {
        self.lock().phase.busy()
    }

    /// Whether `epoch` is still current; if not, one more check is queued.
    fn current(&self, epoch: u64) -> bool {
        if self.epoch.load(Ordering::SeqCst) == epoch {
            return true;
        }
        self.requeue.store(true, Ordering::SeqCst);
        false
    }

    /// Bump the epoch. Callers hold the `inner` lock, so a check's
    /// compare-and-apply ([`Client::apply_if_current`]) can't interleave.
    fn bump(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }

    /// Under the `inner` lock: when `epoch` is still current and no install
    /// is starting, run `f` (which may also touch the staging folder) and
    /// emit afterwards. A stale epoch queues one more check.
    fn apply_if_current(&self, epoch: u64, f: impl FnOnce(&mut Inner)) -> bool {
        {
            let mut inner = self.lock();
            if self.epoch.load(Ordering::SeqCst) != epoch {
                self.requeue.store(true, Ordering::SeqCst);
                return false;
            }
            if inner.phase.busy() {
                return false;
            }
            f(&mut inner);
        }
        self.emit();
        true
    }

    /// Clear the downgrade permission only if no newer switch set it since
    /// `seen` (the switch time this check started with).
    pub(crate) fn clear_downgrade(&self, seen: Option<u64>) {
        let _ = self.settings.update(|s| {
            if s.allow_downgrade_since_ms == seen {
                s.allow_downgrade_once = false;
            }
        });
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Whether a wait for `kind` runs (`None`: any wait).
    pub(super) fn waiting_for(&self, kind: Option<WaitKind>) -> bool {
        self.wait
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|(t, action)| {
                t.load(Ordering::SeqCst) != install::CANCELLED
                    && kind.is_none_or(|k| action.kind() == k)
            })
    }

    fn waiting(&self) -> bool {
        self.waiting_for(Some(WaitKind::Update))
    }

    #[cfg(test)]
    pub(crate) fn settings_store_for_tests(&self) -> &settings::Store {
        &self.settings
    }

    pub(crate) fn settings(&self) -> UpdateSettings {
        self.settings.read()
    }

    /// The whole state for the window. Never the token.
    pub(crate) fn state(&self) -> UpdateState {
        let s = self.settings.read();
        let inner = self.lock();
        let dismissed = match &inner.phase {
            // A Local build's Not now withdraws it instead (row U10).
            UpdatePhase::Ready { release } if release.local.is_none() => {
                s.dismissed_version.as_deref() == Some(release.version.as_str())
            }
            _ => false,
        };
        let downgrade_allowed = s.downgrade_allowed(self.d.clock.now_ms());
        let allowed_channels = super::people::allowed(&s);
        UpdateState {
            phase: inner.phase.clone(),
            running_version: self.d.config.running_version.clone(),
            channel: s.channel,
            configured: self.d.config.configured(),
            connected: inner.connected,
            pending_connect: inner.pending_token.is_some(),
            waiting: self.waiting(),
            go_back_waiting: self.waiting_for(Some(WaitKind::GoBack)),
            dismissed,
            skipped_version: s.skipped_version,
            downgrade_allowed,
            last_checked_at: s.last_checked_at,
            note: inner.note.clone().or_else(|| inner.access_note.clone()),
            allowed_channels,
            person_name: s.person_name,
        }
    }

    pub(super) fn emit(&self) {
        self.d.sink.state(&self.state());
    }

    fn event(&self, event: Event) {
        {
            let mut inner = self.lock();
            inner.phase = apply(&inner.phase, event);
        }
        self.emit();
    }

    pub(super) fn now(&self) -> String {
        crate::update_install::iso(self.d.clock.now_ms())
    }

    // ── Checking and downloading ────────────────────────────────────────

    /// One check, and the download when something is offered. Returns at
    /// once when a check is already running.
    pub(crate) fn check(&self) -> UpdateState {
        loop {
            if self.busy.swap(true, Ordering::SeqCst) {
                // The running check runs once more when it ends.
                self.requeue.store(true, Ordering::SeqCst);
                return self.state();
            }
            {
                let _busy = Busy(&self.busy);
                if !self.installing() {
                    self.check_inner();
                }
            }
            if !self.requeue.swap(false, Ordering::SeqCst) || self.installing() {
                return self.state();
            }
        }
    }

    fn check_inner(&self) {
        // A Local build waits in the button: the channel waits for it.
        if Staged::is_local(&self.lock().staged) {
            return;
        }
        let epoch = self.epoch.load(Ordering::SeqCst);
        let settings = self.settings.read();
        let token = self.d.tokens.get().ok().flatten();
        let ready = match self.d.config.resolve(settings.channel, token) {
            Ok(ready) => ready,
            Err(reason) => {
                if reason == config::NOT_CONNECTED {
                    self.lock().connected = false;
                }
                return self.event(Event::Unconfigured(reason));
            }
        };
        self.lock().connected = true;
        self.event(Event::CheckStarted);
        // Row U9: who the token is and which channels it may use, first.
        let Some((settings, endpoint)) = self.learn_access(epoch, &ready) else {
            return;
        };
        let found = self.d.fetcher.check(&endpoint, &ready.token);
        if !self.current(epoch) {
            return;
        }
        let _ = self
            .settings
            .update(|s| s.last_checked_at = Some(self.now()));
        let channel = settings.channel;
        let refused = matches!(
            found,
            Err(CheckError::Unauthorized) | Err(CheckError::Forbidden)
        );
        self.back_off(refused);
        let pending = match found {
            Ok(Some(pending)) => pending,
            Ok(None) | Err(CheckError::NothingPublished) => {
                return self.nothing_new(epoch, None, None)
            }
            Err(CheckError::Unauthorized) => {
                return self.failed(epoch, super::people::REVOKED.into(), false)
            }
            Err(CheckError::Forbidden) => {
                return self.failed(
                    epoch,
                    format!(
                        "Your updates token can't use the {} channel. Choose another channel in Settings, Updates.",
                        channel.label()
                    ),
                    false,
                )
            }
            Err(CheckError::Network(why)) => {
                return self.failed(
                    epoch,
                    format!("Couldn't reach the update server: {why}"),
                    true,
                )
            }
        };
        let manifest = match manifest::parse(pending.manifest(), channel, &ready.host) {
            Ok(m) if pending.download_url() == m.url => m,
            Ok(_) => {
                return self.failed(
                    epoch,
                    "The update server's manifest names two different downloads.".into(),
                    false,
                )
            }
            Err(why) => return self.failed(epoch, why, false),
        };
        let now_ms = self.d.clock.now_ms();
        let seen = settings.allow_downgrade_since_ms;
        if settings.allow_downgrade_once && !settings.downgrade_allowed(now_ms) {
            // The 24 hours after the switch are over.
            self.clear_downgrade(seen);
        }
        match decide(
            &self.d.config.running_version,
            &manifest.version,
            settings.skipped_version.as_deref(),
            settings.downgrade_allowed(now_ms),
        ) {
            Decision::Skipped(version) => self.nothing_new(epoch, Some(version), Some(seen)),
            Decision::Current => self.nothing_new(epoch, None, Some(seen)),
            Decision::Offer { downgrade } => {
                if !downgrade && self.current(epoch) {
                    // The channel offers something higher: no downgrade is
                    // left to allow.
                    self.clear_downgrade(seen);
                }
                self.offer(epoch, manifest.release(downgrade), pending)
            }
        }
    }

    /// Row U9: ask `/me`, keep the answer, move off a channel no longer
    /// allowed. `None` ends the check (a refused token, no channel allowed,
    /// a stale epoch); else the settings and endpoint the check goes on with.
    fn learn_access(
        &self,
        epoch: u64,
        ready: &config::Ready,
    ) -> Option<(UpdateSettings, reqwest::Url)> {
        use super::people::{self, Access, MeAnswer};
        let answer = self.d.fetcher.me(&ready.host, &ready.token);
        if !self.current(epoch) {
            return None;
        }
        match answer {
            MeAnswer::Refused => {
                let _ = self
                    .settings
                    .update(|s| s.last_checked_at = Some(self.now()));
                self.back_off(true);
                self.failed(epoch, people::REVOKED.into(), false);
                return None;
            }
            MeAnswer::Known(me) => {
                let mut access = Access::Same;
                {
                    // Under the lock, as a disconnect or connect bumps the
                    // epoch: an answer for a token that has gone is dropped.
                    let mut inner = self.lock();
                    if self.epoch.load(Ordering::SeqCst) != epoch {
                        self.requeue.store(true, Ordering::SeqCst);
                        return None;
                    }
                    if matches!(inner.phase, UpdatePhase::Installing { .. }) {
                        return None;
                    }
                    if self
                        .settings
                        .update(|s| access = people::reconcile(s, &me))
                        .is_ok()
                    {
                        inner.access_note = people::access_note(&access);
                    }
                }
                self.emit();
                if access == Access::NoChannel {
                    let _ = self
                        .settings
                        .update(|s| s.last_checked_at = Some(self.now()));
                    self.back_off(true);
                    self.failed(epoch, people::NO_CHANNEL.into(), false);
                    return None;
                }
            }
            MeAnswer::Unsupported | MeAnswer::Network(_) => {}
        }
        let settings = self.settings.read();
        let endpoint = config::endpoint(&ready.host, settings.channel);
        Some((settings, endpoint))
    }

    /// The schedule's next wait: doubled (up to [`MAX_BACKOFF`]) while the
    /// server refuses the token, else [`EVERY`].
    fn back_off(&self, refused: bool) {
        let every = EVERY.as_millis() as u64;
        let next = if refused {
            let now = self.next_wait_ms.load(Ordering::SeqCst).max(every);
            (now * 2).min(MAX_BACKOFF.as_millis() as u64)
        } else {
            every
        };
        self.next_wait_ms.store(next, Ordering::SeqCst);
    }

    /// Nothing to offer. `answered`: the channel has a build, seen with the
    /// switch time it carried (so that switch's downgrade permission has
    /// nothing left to do and is cleared). A withdrawn build also ends
    /// "Update when chats finish".
    fn nothing_new(&self, epoch: u64, skipped: Option<String>, answered: Option<Option<u64>>) {
        let applied = self.apply_if_current(epoch, |inner| {
            inner.staged = None;
            inner.shelved = None;
            inner.note = None;
            self.staging.prune(None);
            inner.phase = apply(&inner.phase, Event::NothingNew { skipped });
        });
        if !applied {
            return;
        }
        if let Some(seen) = answered {
            self.clear_downgrade(seen);
        }
        self.cancel_update_wait();
    }

    /// A check failed. `offline`: a ready download stays ready (its button
    /// too), with a note.
    fn failed(&self, epoch: u64, reason: String, offline: bool) {
        self.apply_if_current(epoch, |inner| {
            if offline && matches!(inner.phase, UpdatePhase::Ready { .. }) {
                inner.note = Some(format!(
                    "{reason}. The update already downloaded is still ready."
                ));
            } else {
                inner.phase = apply(&inner.phase, Event::Failed(reason));
            }
        });
    }

    fn offer(&self, epoch: u64, release: UpdateRelease, pending: Box<dyn Pending>) {
        let version = release.version.clone();
        // Already staged and verified in this launch: ready again (it is
        // verified once more before the installer starts).
        let already = {
            let inner = self.lock();
            inner
                .staged
                .as_ref()
                .filter(|s| s.release.version == version)
                .cloned()
        };
        if let Some(staged) = already {
            return self.become_ready(epoch, release, staged.bundle);
        }
        // Staged by an earlier launch: check it again, no download.
        if let Some(bundle) = self.staging.ready(&version) {
            if self.verify_staged(&bundle, &version).is_ok() {
                return self.become_ready(epoch, release, bundle);
            }
            if !self.apply_if_current(epoch, |_| self.staging.discard(&version)) {
                return;
            }
        }
        let offered = release.clone();
        if !self.apply_if_current(epoch, |inner| {
            inner.phase = apply(&inner.phase, Event::Offered(offered));
        }) {
            return;
        }
        let mut last_percent: Option<u8> = None;
        // `false` stops the download: a switch, a connect or a disconnect
        // made it stale, so a new channel isn't stuck behind it.
        let mut progress = |received: u64, total: Option<u64>| {
            if self.epoch.load(Ordering::SeqCst) != epoch {
                return false;
            }
            let percent = total
                .filter(|t| *t > 0)
                .map(|t| ((received.min(t) * 100) / t) as u8);
            if percent != last_percent || last_percent.is_none() {
                last_percent = percent.or(Some(0));
                self.event(Event::Progress(percent));
            }
            true
        };
        let bytes = match pending.download(&mut progress) {
            Ok(bytes) => bytes,
            Err(why) => return self.failed(epoch, why, false),
        };
        // A switch, a disconnect or an install started meanwhile: drop it.
        if !self.current(epoch) || self.installing() {
            return;
        }
        let now = self.now();
        let staged = self
            .staging
            .stage(&version, &bytes, self.d.limits, &now, |bundle| {
                self.verify_staged(bundle, &version)
            });
        match staged {
            Ok((bundle, _info)) => self.become_ready(epoch, release, bundle),
            Err(why) => self.failed(epoch, why, false),
        }
    }

    fn become_ready(&self, epoch: u64, release: UpdateRelease, bundle: PathBuf) {
        self.apply_if_current(epoch, |inner| {
            self.staging.prune(Some(&release.version));
            inner.staged = Some(Staged {
                release: release.clone(),
                bundle,
                local: None,
            });
            inner.shelved = None;
            inner.note = None;
            inner.phase = apply(&inner.phase, Event::Verified(release));
        });
    }

    /// The app's own checks of a staged bundle, before the button shows and
    /// again before the installer (which runs the bundle's own code, so it
    /// can't vouch for itself) is started: no symlink leaves it, it is not
    /// inside the installed app, `codesign --verify --deep --strict`, the
    /// Developer ID team, bundle id `com.hollow.app` and the version.
    pub(crate) fn verify_staged(&self, bundle: &Path, version: &str) -> Result<BundleInfo, String> {
        let team = self.d.config.team.clone()?;
        self.verify_bundle(bundle, Some(version), &team, "The downloaded update")
    }

    /// The checks of [`Client::verify_staged`] against `team`, `what` naming
    /// the bundle for a person.
    fn verify_bundle(
        &self,
        bundle: &Path,
        version: Option<&str>,
        team: &Team,
        what: &str,
    ) -> Result<BundleInfo, String> {
        let refused = |why: String| format!("{what} didn't pass Hollow's checks: {why}.");
        // Resolved first, so a symlinked staging path can't hide an escape.
        let bundle = &bundle
            .canonicalize()
            .map_err(|e| refused(format!("it can't be read ({e})")))?;
        if let Some(link) = verify::escaping_symlinks(bundle).into_iter().next() {
            return Err(refused(format!(
                "a symlink points outside the app ({})",
                link.display()
            )));
        }
        if let Some(app) = &self.d.paths.installed_app {
            verify::check_location(bundle, &Layout::for_app(app)).map_err(refused)?;
        }
        verify::check_bundle(bundle, self.d.signer.as_ref(), Some(team), version).map_err(refused)
    }

    /// A Local build's checks (row U10): the bytes its receipt recorded,
    /// then the installer's own checks against the team [`local::expected_team`]
    /// picks. Returns the info and that team.
    fn verify_local(
        &self,
        bundle: &Path,
        offer: &LocalOffer,
    ) -> Result<(BundleInfo, Team), String> {
        let digest = crate::self_install::bundle_digest(bundle)
            .map_err(|e| format!("The built app can't be read ({e}). Build it again."))?;
        if digest != offer.receipt.bundle_digest {
            return Err(local::CHANGED.into());
        }
        let own = self.d.signer.team(bundle).ok().flatten();
        let team = local::expected_team(own.as_deref(), self.d.config.team.as_ref().ok());
        let info = self.verify_bundle(bundle, None, &team, "The local build")?;
        Ok((info, team))
    }

    // ── Settings, token, Not now ────────────────────────────────────────

    /// Store a token the person typed in Settings (checked for shape first;
    /// the error never repeats it). It replaces a stored one.
    pub(crate) fn connect(&self, given: &str) -> Result<bool, String> {
        let token = token::accept(given)?;
        self.d.tokens.set(&token)?;
        // Row U9: a new token may be another person's; `/me` says next check.
        let _ = self.settings.update(super::people::forget);
        {
            let mut inner = self.lock();
            self.bump();
            inner.connected = true;
            inner.pending_token = None;
            inner.access_note = None;
            if matches!(&inner.phase, UpdatePhase::NotSetUp { reason } if reason == config::NOT_CONNECTED)
            {
                inner.phase = UpdatePhase::Idle;
            }
        }
        self.emit();
        Ok(true)
    }

    /// A Connect Hollow link's token (shape already checked). Stored at
    /// once when no token is stored; held for the person's confirmation
    /// when a different one is (a link can come from anywhere).
    pub(crate) fn connect_from_link(&self, token: String) -> Result<LinkConnect, String> {
        match self.d.tokens.get() {
            Ok(None) => self.connect(&token).map(|_| LinkConnect::Stored),
            Ok(Some(stored)) if stored == token => {
                self.lock().pending_token = None;
                self.emit();
                Ok(LinkConnect::Same)
            }
            // A different token, or a Keychain that can't be read: ask.
            _ => {
                self.lock().pending_token = Some(token);
                self.emit();
                Ok(LinkConnect::Pending)
            }
        }
    }

    /// The person's "Use the new token".
    pub(crate) fn confirm_connect(&self) -> Result<UpdateState, String> {
        let token = self
            .lock()
            .pending_token
            .take()
            .ok_or("No new updates token is waiting.")?;
        self.connect(&token)?;
        Ok(self.state())
    }

    /// The person's "Keep the current token".
    pub(crate) fn discard_connect(&self) -> UpdateState {
        self.lock().pending_token = None;
        self.emit();
        self.state()
    }

    /// Remove the token. Refused while an install is starting (its staged
    /// build must stay).
    pub(crate) fn disconnect(&self) -> Result<bool, String> {
        if self.installing() {
            return Err(INSTALLING.into());
        }
        self.d.tokens.delete()?;
        let _ = self.settings.update(super::people::forget);
        {
            let mut inner = self.lock();
            if inner.phase.busy() {
                // An install started between the two looks: the token is
                // gone, but the build being installed stays.
                inner.connected = false;
                drop(inner);
                self.emit();
                return Ok(false);
            }
            self.bump();
            inner.connected = false;
            inner.pending_token = None;
            inner.shelved = None;
            inner.access_note = None;
            // A Local build doesn't need the token: it stays.
            if !Staged::is_local(&inner.staged) {
                inner.staged = None;
                inner.note = None;
                inner.phase = apply(
                    &inner.phase,
                    Event::Unconfigured(config::NOT_CONNECTED.into()),
                );
            }
        }
        if !Staged::is_local(&self.lock().staged) {
            self.cancel_update_wait();
        }
        self.emit();
        Ok(false)
    }

    /// Switch channel. A switch allows one lower version (the other
    /// channel's newest may be older than what runs) and says so.
    pub(crate) fn set_channel(&self, channel: Channel) -> Result<UpdateState, String> {
        let current = self.settings.read();
        // Row U9: only a channel the token may use (`/me`).
        super::people::refuse_channel(&current, channel)?;
        if current.channel == channel {
            return Ok(self.state());
        }
        let now_ms = self.d.clock.now_ms();
        {
            // One lock from the install test to the phase write: an install
            // can't start in between.
            let mut inner = self.lock();
            if inner.phase.busy() {
                return Err(INSTALLING.into());
            }
            self.settings.update(|s| {
                s.channel = channel;
                s.allow_downgrade_once = true;
                s.allow_downgrade_since_ms = Some(now_ms);
                s.dismissed_version = None;
            })?;
            self.bump();
            inner.shelved = None;
            inner.access_note = None;
            inner.note = Some(format!(
                "Switched to {}. If its newest build is older than this one, Hollow offers it once.",
                channel.label()
            ));
            // A waiting Local build stays; the channel is checked after it.
            if Staged::is_local(&inner.staged) {
                drop(inner);
                self.emit();
                return Ok(self.state());
            }
            inner.staged = None;
            if !matches!(inner.phase, UpdatePhase::NotSetUp { .. }) {
                inner.phase = UpdatePhase::Idle;
            }
        }
        self.cancel_update_wait();
        self.emit();
        Ok(self.state())
    }

    /// A sentence beside the phase (a refused link).
    pub(crate) fn set_note(&self, note: String) {
        self.lock().note = Some(note);
        self.emit();
    }

    /// "Install updates overnight" (row U7). Takes effect at once: the
    /// overnight thread looks again now, and with the setting off its next
    /// look installs nothing.
    pub(crate) fn set_overnight(&self, enabled: bool) -> Result<UpdateSettings, String> {
        let saved = self.settings.update(|s| s.install_overnight = enabled)?;
        self.wake_overnight();
        Ok(saved)
    }

    /// "This Mac publishes to Main" (row U8, `publish.rs`; People, row U9, reads it too).
    pub(crate) fn set_publisher(&self, enabled: bool) -> Result<UpdateSettings, String> {
        self.settings.update(|s| s.publisher = enabled)
    }

    /// "Not now": hide this version's button until a newer one is ready.
    pub(crate) fn dismiss(&self, version: &str) -> Result<UpdateState, String> {
        // Not now on a Local build: it leaves the button (and isn't offered
        // again at launch); the channel's checks resume.
        let local = {
            let inner = self.lock();
            inner
                .staged
                .as_ref()
                .filter(|s| s.release.version == version)
                .and_then(|s| s.local.as_ref())
                .map(|l| l.receipt.build_id.clone())
        };
        if let Some(build_id) = local {
            self.settings
                .update(|s| s.dismissed_local_build = Some(build_id.clone()))?;
            self.withdraw_local(Some(&build_id));
            return Ok(self.state());
        }
        if !super::state::valid_version(version) {
            return Err("That isn't a version.".into());
        }
        self.settings
            .update(|s| s.dismissed_version = Some(version.to_string()))?;
        self.emit();
        Ok(self.state())
    }

    // ── Local builds (row U10) ──────────────────────────────────────────

    /// Offer a finished branch build in the Update button as "Local build of
    /// <branch>". The bundle is first copied (an APFS clone) into
    /// `updates/local/<build id>/`, and that copy is what is hashed against
    /// the receipt, checked with the installer's checks and handed to the
    /// installer, so the build target changing later (a rebuild, a Not now
    /// on the saved build) can't change what gets installed. It replaces a
    /// channel update that was ready, which is kept aside and comes back
    /// when the Local build is withdrawn (review M3); channel checks pause
    /// while it waits. A wait ("Update when chats finish") armed for
    /// anything else ends here: it never installs a build the person didn't
    /// choose (review B1); the agent's approved install arms its own wait
    /// right after. Refused in a review copy, while an install starts, and
    /// (unless `asked`) for the build the person hid with Not now.
    pub(crate) fn offer_local(&self, offer: LocalOffer) -> Result<UpdateState, String> {
        if self.d.config.review_copy {
            return Err(local::REVIEW_COPY.into());
        }
        let build_id = offer.receipt.build_id.clone();
        if !offer.asked
            && self.settings.read().dismissed_local_build.as_deref() == Some(build_id.as_str())
        {
            return Ok(self.state());
        }
        if self.installing() {
            return Err(INSTALLING.into());
        }
        // The same build already in the button keeps its copy (never
        // rewritten while it is staged).
        let existing = {
            let inner = self.lock();
            (Staged::wait_for(&inner.staged).as_deref() == Some(build_id.as_str()))
                .then(|| inner.staged.as_ref().map(|s| s.bundle.clone()))
                .flatten()
        };
        let fresh = existing.is_none();
        let bundle = match existing {
            Some(bundle) => bundle,
            None => self.copy_local(&offer)?,
        };
        let checked = self.verify_local(&bundle, &offer);
        let (info, _team) = match checked {
            Ok(checked) => checked,
            Err(why) => {
                if fresh {
                    let _ = std::fs::remove_dir_all(self.local_dir(&build_id));
                }
                return Err(why);
            }
        };
        let release = UpdateRelease {
            version: info.version,
            notes: offer.notes.clone(),
            pub_date: Some(offer.receipt.recorded_at.clone()),
            channel: self.settings.read().channel,
            commit: Some(offer.receipt.commit.clone()),
            downgrade: false,
            local: Some(offer.view()),
        };
        let same = {
            let mut inner = self.lock();
            if matches!(inner.phase, UpdatePhase::Installing { .. }) {
                return Err(INSTALLING.into());
            }
            // A check or download under way drops its result.
            self.bump();
            let same = Staged::wait_for(&inner.staged).as_deref() == Some(build_id.as_str());
            if !Staged::is_local(&inner.staged) && inner.staged.is_some() {
                inner.shelved = inner.staged.take();
            }
            inner.staged = Some(Staged {
                release: release.clone(),
                bundle,
                local: Some(offer),
            });
            inner.note = None;
            inner.phase = UpdatePhase::Ready { release };
            same
        };
        if !same {
            self.cancel_update_wait();
        }
        self.prune_local(Some(&build_id));
        if self.settings.read().dismissed_local_build.is_some() {
            let _ = self.settings.update(|s| s.dismissed_local_build = None);
        }
        self.emit();
        Ok(self.state())
    }

    /// `updates/local`, beside the channel's staging folder.
    fn local_root(&self) -> PathBuf {
        self.d.paths.staging.with_file_name("local")
    }

    /// The folder of one Local build's copy (an id that isn't a plain name
    /// gets a fixed one).
    fn local_dir(&self, build_id: &str) -> PathBuf {
        let plain = !build_id.is_empty()
            && build_id.len() <= 80
            && build_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        self.local_root()
            .join(if plain { build_id } else { "build" })
    }

    /// Copy the build's bundle into its own folder under [`Self::local_root`].
    fn copy_local(&self, offer: &LocalOffer) -> Result<PathBuf, String> {
        use crate::update_install::effects::{Fs, RealFs};
        use std::os::unix::fs::DirBuilderExt;
        let dir = self.local_dir(&offer.receipt.build_id);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&dir)
            .map_err(|e| format!("Could not make the local build's folder: {e}"))?;
        let bundle = dir.join("Hollow.app");
        RealFs
            .copy_bundle(Path::new(&offer.receipt.bundle), &bundle)
            .map_err(|why| {
                let _ = std::fs::remove_dir_all(&dir);
                format!("The built app couldn't be copied for installing ({why}). Build it again.")
            })?;
        Ok(bundle)
    }

    /// Remove every Local build copy but `keep`'s. Never while an install
    /// starts (the installer reads the staged copy).
    fn prune_local(&self, keep: Option<&str>) {
        if self.installing() {
            return;
        }
        let keep = keep.map(|id| self.local_dir(id));
        let Ok(entries) = std::fs::read_dir(self.local_root()) else {
            return;
        };
        for entry in entries.flatten() {
            if keep.as_deref() == Some(entry.path().as_path()) {
                continue;
            }
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }

    /// Take the Local build out of the button (any, or only `build_id`): a
    /// new build started, or Not now. Its wait ends; a channel update it
    /// replaced comes back (same channel, still allowed), else the phase
    /// goes back to what the client would show with nothing staged, and the
    /// schedule checks the channel again. Nothing while an install starts.
    pub(crate) fn withdraw_local(&self, build_id: Option<&str>) -> bool {
        let withdrawn = {
            let mut inner = self.lock();
            let mine = inner
                .staged
                .as_ref()
                .and_then(|s| s.local.as_ref())
                .is_some_and(|l| build_id.is_none_or(|id| l.receipt.build_id == id));
            if !mine || matches!(inner.phase, UpdatePhase::Installing { .. }) {
                false
            } else {
                self.bump();
                inner.staged = None;
                inner.note = None;
                let settings = self.settings.read();
                let now_ms = self.d.clock.now_ms();
                let back = inner.shelved.take().filter(|s| {
                    s.release.channel == settings.channel
                        && (!s.release.downgrade || settings.downgrade_allowed(now_ms))
                        && s.bundle.is_dir()
                });
                if let Some(back) = back {
                    inner.phase = UpdatePhase::Ready {
                        release: back.release.clone(),
                    };
                    inner.staged = Some(back);
                } else {
                    let token = inner.connected.then(placeholder);
                    inner.phase = match self.d.config.resolve(settings.channel, token) {
                        Ok(_) => UpdatePhase::Idle,
                        Err(reason) => UpdatePhase::NotSetUp { reason },
                    };
                }
                true
            }
        };
        if withdrawn {
            self.prune_local(None);
            self.cancel_update_wait();
            self.emit();
            self.wake_schedule();
        }
        withdrawn
    }

    /// An install of a Local build is starting: a new app build must not
    /// start meanwhile (`self_install::start_build`).
    pub(crate) fn installing_local(&self) -> bool {
        let inner = self.lock();
        matches!(inner.phase, UpdatePhase::Installing { .. }) && Staged::is_local(&inner.staged)
    }

    /// The Local build waiting in the button, if any (its receipt id).
    #[cfg(test)]
    pub(crate) fn local_build_id(&self) -> Option<String> {
        self.lock()
            .staged
            .as_ref()
            .and_then(|s| s.local.as_ref())
            .map(|l| l.receipt.build_id.clone())
    }

    // ── Installing ──────────────────────────────────────────────────────

    /// "Restart to update". With `confirmed` (the keys the restart dialog
    /// showed), only that work may be running, as `setup_restart` checks;
    /// without, nothing may run. Work that runs is an answer
    /// ([`UpdateInstallOutcome::WorkRunning`]), so the window can offer the
    /// wait.
    ///
    /// Order: verify the staged bundle (codesign can take seconds), then
    /// look at running work and claim, then start the installer; and just
    /// before the quit, look at running work once more (`last_look`).
    pub(crate) fn install_now(
        self: &Arc<Self>,
        confirmed: Option<&[String]>,
    ) -> Result<UpdateInstallOutcome, String> {
        self.install(confirmed, None, None)
    }

    /// `from_wait`: what the wait was armed for; another kind of build in
    /// the button is not installed (review B1). `overnight`: the overnight
    /// installer (row U7, `overnight.rs`) started it for this version; it
    /// marks the record for the morning notification, and both here and at
    /// the claim the version, the skip, Not now and the channel are read
    /// again, so a build replaced or hidden since its look is refused
    /// ([`super::overnight::recheck`]). Every other check is the same.
    pub(super) fn install(
        self: &Arc<Self>,
        confirmed: Option<&[String]>,
        from_wait: Option<&WaitFor>,
        overnight: Option<&str>,
    ) -> Result<UpdateInstallOutcome, String> {
        let staged = {
            let inner = self.lock();
            if from_wait.is_some_and(|armed| *armed != Staged::wait_for(&inner.staged)) {
                return Err(NO_READY.into());
            }
            match (&inner.phase, &inner.staged) {
                (UpdatePhase::Ready { .. }, Some(staged)) => {
                    self.overnight_recheck(&staged.release, overnight)?;
                    staged.clone()
                }
                _ => return Err(NO_READY.into()),
            }
        };
        // A last look that calls the install off waits again only for
        // "Update when chats finish"; the overnight installer looks again
        // itself, in its window and on power (row U7 review), so a wait
        // armed here would install at any hour.
        let from_wait = from_wait.is_some() && overnight.is_none();
        let version = staged.release.version.clone();
        // A downgrade is allowed for 24 hours after the switch only; past
        // that, check the channel again instead of installing it.
        if staged.release.downgrade
            && !self
                .settings
                .read()
                .downgrade_allowed(self.d.clock.now_ms())
        {
            {
                let mut inner = self.lock();
                if matches!(&inner.staged, Some(s) if s.release == staged.release) {
                    inner.staged = None;
                    inner.phase = UpdatePhase::Idle;
                }
            }
            let seen = self.settings.read().allow_downgrade_since_ms;
            self.clear_downgrade(seen);
            self.check();
            self.set_note(DOWNGRADE_EXPIRED.into());
            return Err(DOWNGRADE_EXPIRED.into());
        }
        let checked = match &staged.local {
            Some(offer) => self.verify_local(&staged.bundle, offer),
            None => self
                .verify_staged(&staged.bundle, &version)
                .and_then(|info| self.d.config.team.clone().map(|team| (info, team))),
        };
        let (info, team) = match checked {
            Ok(checked) => checked,
            Err(why) => {
                {
                    let mut inner = self.lock();
                    let same = matches!(&inner.staged, Some(s) if s.release == staged.release);
                    if same && matches!(inner.phase, UpdatePhase::Ready { .. }) {
                        inner.staged = None;
                        inner.phase = apply(&inner.phase, Event::Failed(why.clone()));
                    }
                }
                if !self.installing() && staged.local.is_none() {
                    self.staging.discard(&version);
                } else if staged.local.is_some() && !Staged::is_local(&self.lock().staged) {
                    self.prune_local(None);
                }
                self.emit();
                return Err(why);
            }
        };
        if let Some(outcome) = self.work_in_the_way(confirmed)? {
            return Ok(outcome);
        }
        // Claim: still ready with the same build.
        {
            let mut inner = self.lock();
            let same = matches!(&inner.phase, UpdatePhase::Ready { release } if *release == staged.release);
            if !same {
                return Err(NO_READY.into());
            }
            self.overnight_recheck(&staged.release, overnight)?;
            inner.phase = apply(&inner.phase, Event::InstallStarted);
        }
        self.emit();
        match self.start_installer(&staged, &info, team, overnight.is_some()) {
            Ok(started) => {
                let me = self.clone();
                let keys = confirmed.map(<[String]>::to_vec);
                let release = staged.release.clone();
                self.d.helper.quit(Box::new(move || {
                    me.last_look(started, keys.as_deref(), &release, from_wait)
                }));
                Ok(UpdateInstallOutcome::Started { version })
            }
            Err(why) => {
                // The build is still good: it stays ready, with the reason.
                {
                    let mut inner = self.lock();
                    inner.note = Some(why.clone());
                    inner.phase = apply(&inner.phase, Event::BackToReady);
                }
                self.emit();
                Err(why)
            }
        }
    }

    /// `Some(outcome)` when work is in the way of a restart.
    pub(super) fn work_in_the_way(
        &self,
        confirmed: Option<&[String]>,
    ) -> Result<Option<UpdateInstallOutcome>, String> {
        match confirmed {
            None => {
                let running = self.d.work.running()?;
                if !running.is_empty() {
                    return Ok(Some(UpdateInstallOutcome::WorkRunning { work: running }));
                }
                if let Some(why) = self.d.work.blocked() {
                    return Ok(Some(UpdateInstallOutcome::WorkRunning {
                        work: vec![UpdateWork {
                            key: "busy".into(),
                            name: why,
                        }],
                    }));
                }
                Ok(None)
            }
            Some(keys) => match self.d.work.confirm(keys) {
                Ok(()) => Ok(None),
                Err(why) if why.starts_with("RESTART_WORK_CHANGED") => {
                    Ok(Some(UpdateInstallOutcome::WorkChanged {
                        work: self.d.work.running().unwrap_or_default(),
                    }))
                }
                Err(why) => Err(why),
            },
        }
    }

    /// Just before the quit: if work started meanwhile, stop the installer
    /// (it has moved nothing), go back to ready with a note, and when this
    /// came from "Update when chats finish", wait again. `true` quits.
    fn last_look(
        self: &Arc<Self>,
        mut started: Box<dyn Started>,
        confirmed: Option<&[String]>,
        release: &UpdateRelease,
        from_wait: bool,
    ) -> bool {
        let in_the_way = match self.work_in_the_way(confirmed) {
            Ok(None) => None,
            Ok(Some(
                UpdateInstallOutcome::WorkRunning { work }
                | UpdateInstallOutcome::WorkChanged { work },
            )) => Some(
                work.into_iter()
                    .map(|w| w.name)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Ok(Some(UpdateInstallOutcome::Started { .. })) => None,
            Err(why) => Some(why),
        };
        let note = match in_the_way {
            Some(what) => {
                started.stop();
                format!(
                    "Hollow didn't restart: {what} started meanwhile. The update is still ready."
                )
            }
            // One more look at the installer itself: quitting with no
            // installer running would leave Hollow closed and not updated.
            None => match started.exited() {
                Some(why) => why,
                None => {
                    let _ = self.settings.update(|s| {
                        if release.downgrade {
                            s.allow_downgrade_once = false;
                        }
                        s.dismissed_version = None;
                    });
                    return true;
                }
            },
        };
        let _ = std::fs::remove_file(&self.d.paths.last);
        {
            let mut inner = self.lock();
            inner.note = Some(note);
            inner.phase = apply(&inner.phase, Event::BackToReady);
        }
        self.emit();
        if from_wait {
            self.clear_wait_slot();
            let _ = self.install_when_idle(install::POLL);
        }
        false
    }

    /// Record the start and spawn the installer from the staged bundle
    /// (already verified), then watch it briefly: one that dies at once
    /// leaves Hollow running.
    fn start_installer(
        &self,
        staged: &Staged,
        info: &BundleInfo,
        team: Team,
        overnight: bool,
    ) -> Result<Box<dyn Started>, String> {
        let version = &staged.release.version;
        let app = self
            .d
            .paths
            .installed_app
            .clone()
            .ok_or(NOT_AN_APP.to_string())?;
        let state_dir = self.d.paths.state_dir.clone().ok_or(
            "Hollow can't find its installer folder, so it can't install updates.".to_string(),
        )?;
        std::fs::create_dir_all(&state_dir).map_err(|e| e.to_string())?;
        let install_id = crate::update_install::run::new_install_id(self.d.clock.now_ms());
        let args = install::helper_args(&HelperArgs {
            staged: staged.bundle.clone(),
            version: version.clone(),
            team,
            app,
            state_dir: state_dir.clone(),
            sessions: self.d.paths.sessions.clone().filter(|p| p.exists()),
            // A Local build may be lower than an installed Nightly (U10).
            allow_downgrade: staged.release.downgrade || staged.local.is_some(),
            install_id: install_id.clone(),
        });
        let record = last::Started {
            schema: 1,
            install_id,
            from_version: self.d.config.running_version.clone(),
            to_version: version.clone(),
            notes: staged.release.notes.clone(),
            channel: staged.release.channel,
            at: self.now(),
            pid: self.d.own_pid,
            build_id: staged.local.as_ref().map(|l| l.receipt.build_id.clone()),
            acknowledged: false,
            kind: last::StartedKind::Install,
            restore_data: false,
            skip_settled: false,
            overnight,
            overnight_notified: false,
            local: staged.local.as_ref().map(|l| LocalRecord {
                receipt: l.receipt.clone(),
                branch: l.branch.clone(),
            }),
        };
        last::write(&self.d.paths.last, &record)?;
        let exe = staged
            .bundle
            .join("Contents")
            .join("MacOS")
            .join(&info.executable);
        let spawned = self
            .d
            .helper
            .spawn(&exe, &args, &state_dir.join(install::LOG))
            .and_then(|mut started| match started.exited_early() {
                Some(why) => Err(why),
                None => Ok(started),
            });
        if spawned.is_err() {
            let _ = std::fs::remove_file(&self.d.paths.last);
        }
        spawned
    }

    /// "Update when chats finish": wait, with no expiry, until nothing runs,
    /// then install. In memory only. `interval` is the wait between looks.
    pub(crate) fn install_when_idle(
        self: &Arc<Self>,
        interval: Duration,
    ) -> Result<UpdateState, String> {
        let armed = {
            let inner = self.lock();
            if !matches!(inner.phase, UpdatePhase::Ready { .. }) {
                return Err(NO_READY.into());
            }
            Staged::wait_for(&inner.staged)
        };
        self.wait_until_idle(WaitAction::Update, interval, armed)
    }

    /// Start a "when chats finish" wait for `action`. One of the same kind
    /// already waiting is kept; one of the other kind is cancelled (the
    /// person's latest choice wins). `armed`: what an update's wait may
    /// install ([`Staged::wait_for`]); a go back ignores it.
    pub(super) fn wait_until_idle(
        self: &Arc<Self>,
        action: WaitAction,
        interval: Duration,
        armed: WaitFor,
    ) -> Result<UpdateState, String> {
        let ticket = {
            let mut slot = self.wait.lock().unwrap_or_else(|e| e.into_inner());
            let live = slot
                .as_ref()
                .filter(|(t, _)| t.load(Ordering::SeqCst) != install::CANCELLED)
                .map(|(t, running)| (t.clone(), *running));
            if let Some((t, running)) = live {
                if running.kind() == action.kind() {
                    drop(slot);
                    return Ok(self.state());
                }
                if !install::cancel(&t) {
                    // Its install or go back is starting right now.
                    return Err(INSTALLING.into());
                }
            }
            let ticket = Arc::new(AtomicU8::new(install::WAITING));
            *slot = Some((ticket.clone(), action));
            ticket
        };
        self.emit();
        let me = self.clone();
        std::thread::Builder::new()
            .name("hollow-update-wait".into())
            .spawn(move || me.wait_loop_for(&ticket, action, interval, armed))
            .map_err(|e| {
                self.cancel_wait();
                format!("Could not start the wait: {e}")
            })?;
        Ok(self.state())
    }

    /// "Update when chats finish"'s loop ([`Client::wait_loop_for`]),
    /// armed for what the button holds now.
    #[cfg(test)]
    pub(crate) fn wait_loop(self: &Arc<Self>, ticket: &AtomicU8, interval: Duration) -> WaitEnd {
        let armed = Staged::wait_for(&self.lock().staged);
        self.wait_loop_for(ticket, WaitAction::Update, interval, armed)
    }

    /// Look every `interval`, with no expiry, until nothing runs, then do
    /// `action` (install the ready update, or go back). An update's wait
    /// installs only what it was `armed` for: a channel update (a newer one
    /// a check found too), or that one Local build; anything else in the
    /// button ends it (review B1).
    pub(crate) fn wait_loop_for(
        self: &Arc<Self>,
        ticket: &AtomicU8,
        action: WaitAction,
        interval: Duration,
        armed: WaitFor,
    ) -> WaitEnd {
        let end = install::wait_until_idle(ticket, interval, &|d| self.d.clock.sleep(d), || {
            if action == WaitAction::Update {
                let inner = self.lock();
                if !matches!(inner.phase, UpdatePhase::Ready { .. }) {
                    // A newer check is replacing the build: keep waiting for it.
                    return Ok(false);
                }
                if Staged::wait_for(&inner.staged) != armed {
                    install::cancel(ticket);
                    return Ok(false);
                }
            }
            let idle = self.d.work.running().map(|w| w.is_empty()).unwrap_or(false)
                && self.d.work.blocked().is_none();
            if !idle || !install::claim(ticket) {
                return Ok(false);
            }
            let started = match action {
                WaitAction::Update => match self.install(None, Some(&armed), None) {
                    Ok(UpdateInstallOutcome::Started { .. }) => Ok(true),
                    Ok(_) => Ok(false),
                    Err(why) if why == NO_READY => Ok(false),
                    Err(why) => Err(why),
                },
                WaitAction::GoBack { restore_data } => self.go_back_from_wait(restore_data),
            };
            if matches!(started, Ok(false)) {
                install::release(ticket);
            }
            started
        });
        {
            let mut slot = self.wait.lock().unwrap_or_else(|e| e.into_inner());
            if slot
                .as_ref()
                .is_some_and(|(t, _)| std::ptr::eq(t.as_ref(), ticket))
            {
                *slot = None;
            }
        }
        self.emit();
        end
    }

    /// Stop the "when chats finish" wait (an update's or a go back's).
    /// Nothing when no wait runs.
    pub(crate) fn cancel_wait(&self) -> UpdateState {
        self.cancel_wait_for(None)
    }

    /// Stop only an update's wait: a check that finds nothing, a channel
    /// switch and a disconnect end that one, never a go back's.
    pub(crate) fn cancel_update_wait(&self) -> UpdateState {
        self.cancel_wait_for(Some(WaitKind::Update))
    }

    fn cancel_wait_for(&self, kind: Option<WaitKind>) -> UpdateState {
        let cancelled = {
            let mut slot = self.wait.lock().unwrap_or_else(|e| e.into_inner());
            match slot.as_ref() {
                Some((t, action))
                    if kind.is_none_or(|k| action.kind() == k) && install::cancel(t) =>
                {
                    *slot = None;
                    true
                }
                _ => false,
            }
        };
        if cancelled {
            self.emit();
        }
        self.state()
    }

    /// Drop a finished wait's slot so a new one can start (the last look
    /// found work and waits again).
    pub(super) fn clear_wait_slot(&self) {
        self.wait.lock().unwrap_or_else(|e| e.into_inner()).take();
    }

    // ── After the restart ───────────────────────────────────────────────

    pub(crate) fn last_result(&self) -> Option<UpdateLastResult> {
        last::read(
            &self.d.paths.last,
            self.d.paths.state_dir.as_deref(),
            &self.d.config.running_version,
            Some(&crate::app_menu::build_identity()),
            self.d.own_pid,
        )
    }

    /// After a Local build's install (row U10): write its outcome to the
    /// build receipt in `dir` (`self_install`'s folder) once. `Ok(false)`
    /// when there is nothing to settle.
    pub(crate) fn settle_local_receipt(&self, dir: &Path) -> Result<bool, String> {
        let Some(result) = self.last_result() else {
            return Ok(false);
        };
        let Some(started) = last::read_started(&self.d.paths.last) else {
            return Ok(false);
        };
        let Some(local) = started.local.as_ref() else {
            return Ok(false);
        };
        let log = self
            .d
            .paths
            .state_dir
            .as_deref()
            .map(|d| d.join(install::LOG))
            .unwrap_or_default();
        local::settle_receipt(dir, &started.install_id, local, &result, &log)
    }

    /// Row U7: the morning notification after an overnight install, once.
    pub(crate) fn overnight_notice(&self) -> Option<super::overnight::Notice> {
        last::overnight_notice(
            &self.d.paths.last,
            self.d.paths.state_dir.as_deref(),
            &self.d.config.running_version,
            Some(&crate::app_menu::build_identity()),
            self.d.own_pid,
        )
    }

    pub(crate) fn ack_last_result(&self) -> Result<(), String> {
        last::acknowledge(&self.d.paths.last)
    }

    // ── The schedule ────────────────────────────────────────────────────

    /// Check after `launch_delay`, then every `every`. `rounds` stops it
    /// (tests); the app passes `None`.
    pub(crate) fn run_schedule(
        &self,
        launch_delay: Duration,
        every: Duration,
        rounds: Option<usize>,
    ) {
        self.nap(launch_delay);
        let mut done = 0usize;
        loop {
            self.check();
            done += 1;
            if rounds.is_some_and(|r| done >= r) {
                return;
            }
            let backed_off = Duration::from_millis(self.next_wait_ms.load(Ordering::SeqCst));
            self.nap(backed_off.max(every));
        }
    }
}

impl Client {
    /// Check now from the schedule: its current nap (the launch delay, 30
    /// minutes or a long backoff) ends at once. A check already running
    /// is followed by one more.
    pub(crate) fn wake_schedule(&self) {
        *self.wake.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.wake_cv.notify_all();
    }

    fn nap(&self, d: Duration) {
        if self.d.naps == Naps::Clock {
            return self.d.clock.sleep(d);
        }
        let deadline = std::time::Instant::now() + d;
        let mut woken = self.wake.lock().unwrap_or_else(|e| e.into_inner());
        while !*woken {
            let now = std::time::Instant::now();
            if now >= deadline {
                break;
            }
            woken = self
                .wake_cv
                .wait_timeout(woken, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
        *woken = false;
    }
}

/// A token-shaped placeholder: [`Client::new`] only needs to know one exists.
fn placeholder() -> String {
    "A".repeat(43)
}

/// What a Connect Hollow link did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LinkConnect {
    /// No token was stored: this one is now.
    Stored,
    /// The same token was already stored.
    Same,
    /// A different one is stored: this one waits for the person.
    Pending,
}
