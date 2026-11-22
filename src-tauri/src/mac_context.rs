//! "Connect this Mac's context" (two-Macs F3, 2026-11-15): the two-Macs form
//! of "Back up and sync your vault". When another Mac appears (a runner named
//! in `slot-remote`, or a Mac paired in Machines, M2), Hollow offers once per
//! Mac pair to connect this Mac's context layers: the personal clone, the
//! organization clone (read only), this Mac's name (slug) and its home file
//! `home/<slug>.md`, and the vault row, which stays held until V6 gives the
//! vault a remote.
//!
//! **The record.** `mac-context-offers.json` in the app data folder, written
//! whole through a temp file and a rename (0600):
//! `{"version":1,"pairs":{"<a>+<b>":{"offeredAt","answer","answeredAt"}}}`.
//! A pair key is this Mac's `machineId` and the other Mac's id, sorted, so
//! A-B and B-A are one pair. A runner has no Hollow id of its own: it is
//! `runner:<alias>`. A file that doesn't parse reads as empty and is refused,
//! never overwritten, by a write. Fields a later Hollow adds are kept.
//!
//! **When it is offered** ([`offer_due`]): a pair with no record; else never
//! once the answer is Connected or Not for this Mac; else (Not now, or
//! offered and never answered) once [`RENEW_DAYS`] have passed since the
//! later of the offer and the answer. The check runs a minute after launch
//! and after a pairing or a runner is named ([`after_change`]); a due pair
//! with a layer still missing here gets one in-app notification (Open goes to
//! Settings, Sources, Context, where the card is) and its `offeredAt`. The
//! card shows while the pair is due or offered and unanswered
//! ([`card_open`]).
//!
//! **Connect** ([`mac_context_connect`], the person's click): the request
//! names layers and repo addresses, never a path. Each clone goes to the
//! layer's saved clone folder, else its default (`~/Hollow/Context/<layer>`),
//! checked by `context_roots`' path rules; a folder that is already a git
//! clone is kept (and saved as the root when it wasn't), one holding other
//! files is refused. git clones into a hidden staging folder beside the root
//! and the folder is renamed into place only when the clone finished, so a
//! failure leaves nothing half-written; the root is saved after. git runs
//! with no prompt of any kind (ssh in batch mode), hooks and fsmonitor off;
//! an address carrying credentials is refused (`valid_remote`) and every
//! message is redacted. Home names this Mac when it has no name (the
//! suggested slug unless one is given) and writes a short `home/<slug>.md`
//! only when none is there (a hard link from a temp file, so an existing
//! file is never replaced). Layers report one by one; a failed one leaves
//! the others as they finished. Nothing is committed or pushed. One Connect
//! runs at a time, and the pair is recorded Connected only when every step
//! worked and no personal, org or home row is still missing.

use crate::context_roots::{self as roots, Layer};
use crate::sync_engine::{self, RootKind};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::Duration;

/// A Not now (or an unanswered offer) comes back after this many days.
pub(crate) const RENEW_DAYS: i64 = 14;
const FILE_NAME: &str = "mac-context-offers.json";
const VERSION: u32 = 1;
/// A clone's limit.
const CLONE_TIMEOUT: Duration = Duration::from_secs(600);
/// How long after launch the first check runs.
const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The notification's source.
const SOURCE: &str = "mac-context";

// ── The record ──────────────────────────────────────────────────────────

/// The person's answer to the offer for one pair.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Answer {
    Connected,
    NotNow,
    Never,
}

impl Answer {
    pub(crate) fn parse(text: &str) -> Result<Answer, String> {
        match text {
            "connected" => Ok(Answer::Connected),
            "not-now" => Ok(Answer::NotNow),
            "never" => Ok(Answer::Never),
            other => Err(format!("{other:?} is not an answer: use not-now or never.")),
        }
    }
}

/// One Mac pair's offer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PairRecord {
    #[serde(default)]
    pub offered_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub answer: Option<Answer>,
    #[serde(default)]
    pub answered_at: Option<DateTime<Utc>>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// `mac-context-offers.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct OfferState {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub pairs: BTreeMap<String, PairRecord>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

/// The key for the pair of Macs `a` and `b`: the same for either order.
pub(crate) fn pair_key(a: &str, b: &str) -> String {
    if a <= b {
        format!("{a}+{b}")
    } else {
        format!("{b}+{a}")
    }
}

/// Whether the offer for `pair` is due at `now`: a new pair always; a
/// Connected or Not for this Mac answer never; a Not now, or an offer left
/// unanswered, once [`RENEW_DAYS`] have passed since the later of the two.
pub(crate) fn offer_due(state: &OfferState, pair: &str, now: DateTime<Utc>) -> bool {
    let Some(record) = state.pairs.get(pair) else {
        return true;
    };
    match record.answer {
        Some(Answer::Connected | Answer::Never) => false,
        Some(Answer::NotNow) | None => {
            let last = record.offered_at.max(record.answered_at);
            last.is_none_or(|at| now - at >= ChronoDuration::days(RENEW_DAYS))
        }
    }
}

/// Whether the card shows for `pair`: due, or offered and not answered since.
pub(crate) fn card_open(state: &OfferState, pair: &str, now: DateTime<Utc>) -> bool {
    if offer_due(state, pair, now) {
        return true;
    }
    state.pairs.get(pair).is_some_and(|r| match r.answer {
        None => true,
        Some(Answer::NotNow) => r.offered_at > r.answered_at,
        Some(_) => false,
    })
}

/// The record file in the app data folder (or a test's own).
pub(crate) struct Offers {
    path: PathBuf,
}

static OFFERS_LOCK: Mutex<()> = Mutex::new(());

impl Offers {
    pub(crate) fn for_app() -> Result<Self, String> {
        Ok(Self {
            path: crate::agent_settings::app_data_dir()?.join(FILE_NAME),
        })
    }

    #[cfg(test)]
    pub(crate) fn at(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
        }
    }

    /// The record; a missing file or one that doesn't parse is empty.
    pub(crate) fn read(&self) -> OfferState {
        self.load().unwrap_or_default()
    }

    fn load(&self) -> Result<OfferState, String> {
        match fs::read_to_string(&self.path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| {
                format!(
                    "{} isn't valid ({e}); fix or remove it first.",
                    self.path.display()
                )
            }),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(OfferState::default()),
            Err(e) => Err(format!("Couldn't read {}: {e}", self.path.display())),
        }
    }

    /// Read, change and write the record under one lock. A file that
    /// doesn't parse is refused and left as it is.
    pub(crate) fn change<T>(&self, edit: impl FnOnce(&mut OfferState) -> T) -> Result<T, String> {
        let _held = OFFERS_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.load()?;
        let out = edit(&mut state);
        state.version = VERSION;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|e| format!("Couldn't make {}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(&state).map_err(|e| e.to_string())?;
        crate::vault::atomic_write_private(&self.path, format!("{text}\n").as_bytes())?;
        Ok(out)
    }

    /// Save the person's answer for `pair`.
    pub(crate) fn answer(
        &self,
        pair: &str,
        answer: Answer,
        now: DateTime<Utc>,
    ) -> Result<(), String> {
        self.change(|s| {
            let r = s.pairs.entry(pair.to_string()).or_default();
            r.answer = Some(answer);
            r.answered_at = Some(now);
        })
    }

    /// Stamp `pair` as offered now (the check stamps through [`stamp_due`]).
    #[cfg(test)]
    pub(crate) fn offered(&self, pair: &str, now: DateTime<Utc>) -> Result<(), String> {
        self.change(|s| {
            s.pairs.entry(pair.to_string()).or_default().offered_at = Some(now);
        })
    }
}

// ── The other Macs ──────────────────────────────────────────────────────

/// Another Mac this one is connected to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OtherMac {
    /// A paired Mac's `machineId`, or `runner:<alias>`.
    pub id: String,
    pub name: String,
    /// `runner` or `peer`.
    pub kind: String,
}

/// The runner this Mac names (`slot-remote`) and every paired Mac.
fn other_macs() -> Vec<OtherMac> {
    let mut out = Vec::new();
    if let Some(alias) = crate::remote_session::configured_runner() {
        out.push(OtherMac {
            id: format!("runner:{alias}"),
            name: alias,
            kind: "runner".into(),
        });
    }
    if let Some(machines) = crate::peer_machines::Machines::for_app() {
        match machines.list() {
            Ok(list) => out.extend(list.into_iter().map(|m| OtherMac {
                id: m.machine_id,
                name: m.name,
                kind: "peer".into(),
            })),
            Err(e) => eprintln!("[mac-context] paired Macs not read: {e}"),
        }
    }
    out
}

// ── The layers on this Mac ──────────────────────────────────────────────

/// One row of the card.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayerRow {
    /// `personal`, `org`, `home` or `vault`.
    pub layer: String,
    /// `connected`, `missing` or `held` (the vault, until V6).
    pub state: String,
    pub text: String,
    /// Where a clone goes (the saved root, else the default).
    pub target: Option<String>,
    /// A repo address this Mac already knows for the layer (no credentials).
    pub url: Option<String>,
}

/// The card: which Mac it is about and what this Mac has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MacContextOffer {
    /// The pair key, `None` when no other Mac is connected (asked with `any`).
    pub pair: Option<String>,
    pub other_name: Option<String>,
    /// `runner` or `peer`.
    pub other_kind: Option<String>,
    /// The offer is due or waiting: the card shows Not now and Not for this Mac.
    pub open: bool,
    pub layers: Vec<LayerRow>,
    pub slug: Option<String>,
    pub suggested_slug: Option<String>,
}

/// What this Mac has, as plain facts (tests build their own).
#[derive(Clone, Debug, Default)]
pub(crate) struct Here {
    pub home: PathBuf,
    pub saved: roots::RootSettings,
    pub slug: Option<String>,
    pub personal_remote: Option<String>,
    pub vault_remote: Option<String>,
    pub vault_set_up: bool,
}

impl Here {
    fn read() -> Here {
        let config = sync_engine::settings_path()
            .map(|p| sync_engine::config_at(&p))
            .unwrap_or_default();
        let personal_remote = config
            .root(sync_engine::PERSONAL)
            .and_then(|r| r.remote.clone())
            .filter(|u| sync_engine::valid_remote(u).is_ok());
        let vault = config.roots.iter().find(|r| r.kind == RootKind::Vault);
        Here {
            home: dirs::home_dir().unwrap_or_default(),
            saved: roots::settings(),
            slug: roots::machine_slug(),
            personal_remote,
            vault_remote: vault.and_then(|r| r.remote.clone()),
            vault_set_up: vault.is_some(),
        }
    }

    /// A layer's folder: the saved root, else the default.
    fn target(&self, layer: Layer) -> PathBuf {
        self.saved.get(layer).map(PathBuf::from).unwrap_or_else(|| {
            self.home
                .join("Hollow")
                .join("Context")
                .join(layer.as_str())
        })
    }

    /// The layer's saved root, usable now and a real git clone.
    fn cloned(&self, layer: Layer) -> Option<PathBuf> {
        roots::root_in(&self.saved, layer, &self.home)
            .filter(|r| crate::context_settings::is_plain_git_dir(r))
    }
}

/// The card's rows for what this Mac has.
pub(crate) fn layer_rows(here: &Here) -> Vec<LayerRow> {
    let row =
        |layer: &str, state: &str, text: String, target: Option<String>, url: Option<String>| {
            LayerRow {
                layer: layer.into(),
                state: state.into(),
                text,
                target,
                url,
            }
        };
    let shown = |p: &Path| p.display().to_string();
    let mut out = Vec::new();
    for (layer, name) in [(Layer::Personal, "Personal"), (Layer::Org, "Organization")] {
        let target = here.target(layer);
        let url = match layer {
            Layer::Personal => here.personal_remote.clone(),
            Layer::Org => None,
        };
        out.push(match here.cloned(layer) {
            Some(root) => row(
                layer.as_str(),
                "connected",
                format!("{name} context is cloned at {}.", shown(&root)),
                Some(shown(&root)),
                None,
            ),
            None => row(
                layer.as_str(),
                "missing",
                format!(
                    "Clone the {} repo into {}.",
                    name.to_lowercase(),
                    shown(&target)
                ),
                Some(shown(&target)),
                url,
            ),
        });
    }
    let home_file = here.slug.as_deref().and_then(|s| roots::home_file(s).ok());
    let home_there = match (&home_file, here.cloned(Layer::Personal)) {
        (Some(rel), Some(root)) => roots::resolve_in_root(&root, rel)
            .ok()
            .and_then(|p| fs::symlink_metadata(p).ok())
            .is_some_and(|m| m.is_file()),
        _ => false,
    };
    out.push(match (&here.slug, home_there) {
        (Some(slug), true) => row(
            "home",
            "connected",
            format!("This Mac is {slug}, with its home file home/{slug}.md."),
            None,
            None,
        ),
        (Some(slug), false) => row(
            "home",
            "missing",
            format!("This Mac is {slug}; write home/{slug}.md in the personal clone."),
            None,
            None,
        ),
        (None, _) => row(
            "home",
            "missing",
            "Name this Mac and write its home file in the personal clone.".into(),
            None,
            None,
        ),
    });
    out.push(match (&here.vault_remote, here.vault_set_up) {
        (Some(remote), _) => row(
            "vault",
            "held",
            format!(
                "Vault sync names {}; cloning the vault here waits for its first push (V6).",
                sync_engine::redact_urls(remote)
            ),
            None,
            None,
        ),
        _ => row(
            "vault",
            "held",
            "The vault has no remote yet (its first push is held), so this Mac keeps its own vault.".into(),
            None,
            None,
        ),
    });
    out
}

/// A personal, org or home row is missing here.
fn any_missing(rows: &[LayerRow]) -> bool {
    rows.iter().any(|r| r.state == "missing")
}

/// The card for the first other Mac whose offer is open, or with `any`, the
/// first other Mac (or none) whatever its answer, so the person can connect
/// later from Settings.
pub(crate) fn offer_in(
    here: &Here,
    this_id: &str,
    others: &[OtherMac],
    state: &OfferState,
    now: DateTime<Utc>,
    any: bool,
) -> Option<MacContextOffer> {
    let layers = layer_rows(here);
    let open = if any_missing(&layers) {
        others
            .iter()
            .find(|o| card_open(state, &pair_key(this_id, &o.id), now))
    } else {
        None
    };
    if open.is_none() && !any {
        return None;
    }
    let other = open.or_else(|| others.first());
    Some(MacContextOffer {
        pair: other.map(|o| pair_key(this_id, &o.id)),
        other_name: other.map(|o| o.name.clone()),
        other_kind: other.map(|o| o.kind.clone()),
        open: open.is_some(),
        layers,
        slug: here.slug.clone(),
        suggested_slug: None,
    })
}

/// The pairs to notify now: due, with a layer missing here.
pub(crate) fn due_pairs(
    here: &Here,
    this_id: &str,
    others: &[OtherMac],
    state: &OfferState,
    now: DateTime<Utc>,
) -> Vec<(String, OtherMac)> {
    if !any_missing(&layer_rows(here)) {
        return Vec::new();
    }
    others
        .iter()
        .map(|o| (pair_key(this_id, &o.id), o.clone()))
        .filter(|(pair, _)| offer_due(state, pair, now))
        .collect()
}

// ── The check ───────────────────────────────────────────────────────────

static VAULT: OnceLock<PathBuf> = OnceLock::new();

/// Launch: remember the vault (for the notification) and check a minute
/// later, on its own thread.
pub(crate) fn start(vault: PathBuf) {
    if VAULT.set(vault).is_err() {
        return;
    }
    std::thread::spawn(|| {
        std::thread::sleep(FIRST_CHECK);
        check();
    });
}

/// A runner was named or a Mac paired: check again, on its own thread. A
/// no-op before [`start`] (tests, the headless CLI).
pub(crate) fn after_change() {
    if VAULT.get().is_some() {
        std::thread::spawn(check);
    }
}

/// The due pairs, each stamped offered in the same read-change-write, so
/// two checks at once (launch and a pairing) never both notify one pair.
pub(crate) fn stamp_due(
    offers: &Offers,
    here: &Here,
    this_id: &str,
    others: &[OtherMac],
    now: DateTime<Utc>,
) -> Result<Vec<(String, OtherMac)>, String> {
    offers.change(|state| {
        let due = due_pairs(here, this_id, others, state, now);
        for (pair, _) in &due {
            state.pairs.entry(pair.clone()).or_default().offered_at = Some(now);
        }
        due
    })
}

/// Notify each due pair once and stamp it offered.
fn check() {
    let Some(vault) = VAULT.get() else { return };
    let others = other_macs();
    if others.is_empty() {
        return;
    }
    let Ok(this_id) = roots::ensure_machine_id() else {
        return;
    };
    let Ok(offers) = Offers::for_app() else {
        return;
    };
    let due = match stamp_due(&offers, &Here::read(), &this_id, &others, Utc::now()) {
        Ok(due) => due,
        Err(e) => {
            eprintln!("[mac-context] offer not recorded: {e}");
            return;
        }
    };
    for (_, other) in due {
        let input = crate::notifications::NotificationInput {
            title: "Connect this Mac's context".into(),
            body: format!(
                "{} is connected to this Mac. Clone your personal and org context here and name this Mac: Settings, Sources, Context.",
                other.name
            ),
            kind: "info".into(),
            source: Some(SOURCE.into()),
            action_label: Some("Open".into()),
            action_url: Some("settings:layers".into()),
            notify_native: false,
        };
        if let Some(Err(e)) = crate::notifications::create_registered(vault, input) {
            eprintln!("[mac-context] not raised: {e}");
        }
    }
}

// ── Connect ─────────────────────────────────────────────────────────────

/// What Connect is asked to do. No path: every target is a layer's root.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    /// The card's pair, recorded as Connected when every step works.
    #[serde(default)]
    pub pair: Option<String>,
    /// Ticked rows: `personal`, `org`, `home`, `vault`.
    pub layers: Vec<String>,
    #[serde(default)]
    pub personal_url: Option<String>,
    #[serde(default)]
    pub org_url: Option<String>,
    /// This Mac's name when it has none (else the suggested one).
    #[serde(default)]
    pub slug: Option<String>,
}

/// One layer's result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub layer: String,
    /// `done`, `skipped`, `held`, `failed` or `refused`.
    pub state: String,
    pub text: String,
}

fn step(layer: &str, state: &str, text: impl Into<String>) -> Step {
    Step {
        layer: layer.into(),
        state: state.into(),
        text: text.into(),
    }
}

/// Every step's result, and whether the pair is now Connected.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectReport {
    pub steps: Vec<Step>,
    pub connected: bool,
}

/// Where Connect reads and saves roots (tests use their own).
pub(crate) struct RootsAt {
    pub home: PathBuf,
    pub settings: PathBuf,
}

/// git's words on a failed clone, as the row says them (never a secret).
fn clone_failure(code: Option<i32>, stderr: &str, url: &str) -> String {
    let shown = sync_engine::redact_urls(url);
    let check = crate::context_settings::classify_access(code, stderr);
    match check.state.as_str() {
        "denied" => format!(
            "This Mac can't read {shown}: sign in to git here (an SSH key, or gh auth login), or ask the repo's owner for access, then Connect again. Nothing was cloned."
        ),
        "offline" => format!("Couldn't reach {shown}. Try again online. Nothing was cloned."),
        _ => {
            let last = stderr.trim().lines().last().unwrap_or("").trim();
            format!(
                "The clone of {shown} failed ({}). Nothing was cloned.",
                if last.is_empty() {
                    "git gave no reason".to_string()
                } else {
                    sync_engine::redact_urls(last)
                }
            )
        }
    }
}

/// A folder that has nothing in it (or doesn't exist).
fn empty_or_absent(dir: &Path) -> Result<bool, String> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(true),
        Err(e) => Err(format!("Couldn't read {}: {e}", dir.display())),
    }
}

/// Clone `url` into `layer`'s root (saved, else the default) and save the
/// root. See the module docs for the rules.
pub(crate) async fn clone_layer(at: &RootsAt, layer: Layer, url: Option<&str>) -> Step {
    let name = layer.as_str();
    let saved = roots::settings_at(&at.settings);
    let wanted = saved
        .get(layer)
        .map(PathBuf::from)
        .unwrap_or_else(|| at.home.join("Hollow").join("Context").join(name));
    let target = match roots::normalize_root(&wanted.to_string_lossy(), &at.home) {
        Ok(t) => t,
        Err(e) => return step(name, "refused", format!("{}: {e}", wanted.display())),
    };
    let save = |target: &Path| -> Result<(), String> {
        if saved.get(layer) == Some(&*target.to_string_lossy()) {
            return Ok(());
        }
        roots::set_root_at(
            &at.settings,
            &at.home,
            layer,
            Some(&target.to_string_lossy()),
        )
        .map(drop)
        .map_err(|e| e.to_string())
    };
    // Kept apart from the other layer's saved root before anything is
    // written (the save checks it again; this one comes before the clone).
    let other = match layer {
        Layer::Personal => Layer::Org,
        Layer::Org => Layer::Personal,
    };
    if let Some(o) = saved.get(other) {
        if roots::overlap(&target, Path::new(o)) {
            return step(name, "refused", roots::PathError::Overlap.to_string());
        }
    }
    if crate::context_settings::is_plain_git_dir(&target) {
        return match save(&target) {
            Ok(()) => step(
                name,
                "skipped",
                format!("Already cloned at {}.", target.display()),
            ),
            Err(e) => step(name, "failed", e),
        };
    }
    match empty_or_absent(&target) {
        Ok(true) => {}
        Ok(false) => {
            return step(
                name,
                "refused",
                format!(
                    "{} holds files and isn't a git clone; Hollow won't clone over it. Empty it or choose another folder in Settings, Context.",
                    target.display()
                ),
            )
        }
        Err(e) => return step(name, "failed", e),
    }
    let Some(url) = url.map(str::trim).filter(|u| !u.is_empty()) else {
        return step(
            name,
            "failed",
            "Paste the repo's address first (git@github.com:owner/repo.git or https://).",
        );
    };
    if let Err(e) = sync_engine::valid_remote(url) {
        return step(name, "refused", e);
    }
    let (Some(parent), Some(leaf)) = (target.parent(), target.file_name()) else {
        return step(
            name,
            "refused",
            format!("{} has no parent folder.", target.display()),
        );
    };
    if let Err(e) = fs::create_dir_all(parent) {
        return step(
            name,
            "failed",
            format!("Couldn't make {}: {e}", parent.display()),
        );
    }
    let bytes: [u8; 6] = rand::random();
    let tag: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let staging_name = format!(".{}.hollow-clone-{tag}", leaf.to_string_lossy());
    let staging = parent.join(&staging_name);
    let args = [
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.sshCommand=ssh -o BatchMode=yes",
        "-c",
        "protocol.ext.allow=never",
        "clone",
        "--quiet",
        "--no-recurse-submodules",
        "--",
        url,
        staging_name.as_str(),
    ];
    let out = crate::build_workspaces::git_raw(parent, &args, CLONE_TIMEOUT, 64 * 1024).await;
    let failed = match &out {
        Ok(o) if o.code == Some(0) => None,
        Ok(o) => Some(clone_failure(o.code, &o.stderr, url)),
        Err(e) => Some(format!(
            "The clone of {} didn't finish: {}. Nothing was cloned.",
            sync_engine::redact_urls(url),
            sync_engine::redact_urls(e)
        )),
    };
    if let Some(why) = failed {
        let _ = fs::remove_dir_all(&staging);
        return step(name, "failed", why);
    }
    if !crate::context_settings::is_plain_git_dir(&staging) {
        let _ = fs::remove_dir_all(&staging);
        return step(
            name,
            "failed",
            "git finished but left no plain .git folder; nothing was kept.",
        );
    }
    // An empty folder in the way goes (rename can't replace it on every
    // filesystem); one that filled meanwhile stays and the clone is dropped.
    if target.exists() {
        if let Err(e) = fs::remove_dir(&target) {
            let _ = fs::remove_dir_all(&staging);
            return step(
                name,
                "failed",
                format!(
                    "{} changed while cloning ({e}); nothing was kept.",
                    target.display()
                ),
            );
        }
    }
    if let Err(e) = fs::rename(&staging, &target) {
        let _ = fs::remove_dir_all(&staging);
        return step(
            name,
            "failed",
            format!("Couldn't move the clone into {}: {e}", target.display()),
        );
    }
    if let Err(e) = save(&target) {
        // Ours, just made: removed so nothing is left half set up.
        let _ = fs::remove_dir_all(&target);
        return step(
            name,
            "failed",
            format!("The clone couldn't be saved as the {name} folder: {e}"),
        );
    }
    step(
        name,
        "done",
        format!(
            "Cloned {} into {}.",
            sync_engine::redact_urls(url),
            target.display()
        ),
    )
}

/// The short home file Connect writes for `slug`.
pub(crate) fn home_starter(slug: &str) -> String {
    format!(
        "# {slug}\n\nFacts about this Mac ({slug}) that every chat running here should know: what it is for, what runs on it, where things live. Hollow reads this file at the start of each chat on this Mac.\n"
    )
}

/// Write `home/<slug>.md` under `root` only when there is none: no symlink
/// on the way, never replacing a file.
pub(crate) fn write_home_file(root: &Path, slug: &str) -> Step {
    let rel = match roots::home_file(slug) {
        Ok(rel) => rel,
        Err(e) => return step("home", "refused", e.to_string()),
    };
    let full = match roots::resolve_in_root(root, &rel) {
        Ok(p) => p,
        Err(e) => return step("home", "refused", format!("{rel}: {e}")),
    };
    if fs::symlink_metadata(&full).is_ok() {
        return step(
            "home",
            "skipped",
            format!("This Mac is {slug}; {rel} is already there."),
        );
    }
    let Some(dir) = full.parent() else {
        return step("home", "refused", format!("{rel} has no folder."));
    };
    if let Err(e) = fs::create_dir_all(dir) {
        return step(
            "home",
            "failed",
            format!("Couldn't make {rel}'s folder: {e}"),
        );
    }
    // Checked again after making the folder: a link put there meanwhile.
    if let Err(e) = roots::resolve_in_root(root, &rel) {
        return step("home", "refused", format!("{rel}: {e}"));
    }
    let bytes: [u8; 6] = rand::random();
    let tag: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let tmp = dir.join(format!(".{slug}.md.hollow-{tag}"));
    let wrote = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(home_starter(slug).as_bytes())?;
        f.sync_all()?;
        // Fails when a file appeared at the name: never replaced.
        fs::hard_link(&tmp, &full)
    })();
    let _ = fs::remove_file(&tmp);
    match wrote {
        Ok(()) => step(
            "home",
            "done",
            format!("This Mac is {slug}; wrote {rel} in the personal clone."),
        ),
        Err(e) if e.kind() == ErrorKind::AlreadyExists => step(
            "home",
            "skipped",
            format!("This Mac is {slug}; {rel} is already there."),
        ),
        Err(e) => step("home", "failed", format!("Couldn't write {rel}: {e}")),
    }
}

/// Every ticked layer in order (personal before home, which needs it). The
/// slug is named through `name_mac` (the app's saves move automations too).
pub(crate) async fn connect_in(
    at: &RootsAt,
    req: &ConnectRequest,
    slug_now: Option<String>,
    suggested: Option<String>,
    name_mac: impl FnOnce(&str) -> Result<Option<String>, String>,
) -> Vec<Step> {
    let wants = |l: &str| req.layers.iter().any(|x| x == l);
    let mut steps = Vec::new();
    for l in &req.layers {
        if !matches!(l.as_str(), "personal" | "org" | "home" | "vault") {
            steps.push(step(l, "refused", format!("{l:?} is not a context layer.")));
        }
    }
    if wants("personal") {
        steps.push(clone_layer(at, Layer::Personal, req.personal_url.as_deref()).await);
    }
    if wants("org") {
        steps.push(clone_layer(at, Layer::Org, req.org_url.as_deref()).await);
    }
    if wants("home") {
        let slug = match slug_now {
            Some(s) => Ok(s),
            None => match req
                .slug
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .or(suggested)
            {
                None => Err("Type a name for this Mac first.".to_string()),
                Some(s) => roots::valid_slug(&s)
                    .map_err(|e| e.to_string())
                    .and_then(|()| name_mac(&s))
                    .and_then(|saved| saved.ok_or_else(|| "The name wasn't saved.".to_string())),
            },
        };
        let saved = roots::settings_at(&at.settings);
        steps.push(match slug {
            Err(e) => step("home", "failed", e),
            Ok(slug) => match roots::root_in(&saved, Layer::Personal, &at.home) {
                Some(root) => write_home_file(&root, &slug),
                None => step(
                    "home",
                    "failed",
                    format!("This Mac is {slug}; its home file waits for the personal clone."),
                ),
            },
        });
    }
    if wants("vault") {
        steps.push(step(
            "vault",
            "held",
            "Not cloned: the vault has no remote until its first push (V6). This Mac keeps its own vault.",
        ));
    }
    steps
}

/// The pair is Connected (final) only when every ticked step worked and no
/// personal, org or home row is still missing here (an unticked row too).
pub(crate) fn connected_now(steps: &[Step], rows_after: &[LayerRow]) -> bool {
    // No rows means they weren't read: never taken as nothing missing.
    !rows_after.is_empty() && all_connected(steps) && !any_missing(rows_after)
}

/// Every step either worked or had nothing to do.
pub(crate) fn all_connected(steps: &[Step]) -> bool {
    !steps.is_empty()
        && steps
            .iter()
            .all(|s| matches!(s.state.as_str(), "done" | "skipped" | "held"))
}

/// One Connect at a time in this process: a second click while clones run
/// is refused, never a second clone into the same folders.
static CONNECTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Held while a Connect runs; dropped (also on a panic) to let the next in.
pub(crate) struct ConnectGuard(());

impl Drop for ConnectGuard {
    fn drop(&mut self) {
        CONNECTING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

pub(crate) fn begin_connect() -> Result<ConnectGuard, String> {
    use std::sync::atomic::Ordering;
    CONNECTING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .map(|_| ConnectGuard(()))
        .map_err(|_| "Connect is already running on this Mac. Wait for it to finish.".to_string())
}

// ── Commands ────────────────────────────────────────────────────────────

/// The card (Settings, Context): the open offer, or with `any` the layers
/// whatever the answer. Reads only (this Mac's id is made on first need, as
/// pairing does).
#[tauri::command]
pub async fn mac_context_offer(any: Option<bool>) -> Result<Option<MacContextOffer>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let others = other_macs();
        let this_id = roots::ensure_machine_id()?;
        let state = Offers::for_app()?.read();
        let here = Here::read();
        let mut offer = offer_in(
            &here,
            &this_id,
            &others,
            &state,
            Utc::now(),
            any == Some(true),
        );
        if let Some(o) = offer.as_mut().filter(|o| o.slug.is_none()) {
            o.suggested_slug = roots::suggested_slug();
        }
        Ok(offer)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The person's Not now (`not-now`) or Not for this Mac (`never`).
#[tauri::command]
pub async fn mac_context_answer(pair: String, answer: String) -> Result<(), String> {
    let answer = Answer::parse(&answer)?;
    if answer == Answer::Connected {
        return Err("Connected is recorded by Connect itself.".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        Offers::for_app()?.answer(&pair, answer, Utc::now())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Connect: clone the ticked layers here, name this Mac and write its home
/// file. Each step reports on its own; the pair is Connected when all work
/// and nothing is left missing. One at a time ([`begin_connect`]).
#[tauri::command]
pub async fn mac_context_connect(
    state: tauri::State<'_, crate::commands::AppState>,
    request: ConnectRequest,
) -> Result<ConnectReport, String> {
    let _running = begin_connect()?;
    let vault = state.config.vault_path.clone();
    let at = RootsAt {
        home: dirs::home_dir().ok_or("Couldn't find the home folder.")?,
        settings: roots::settings_path()?,
    };
    let (slug_now, suggested) = tauri::async_runtime::spawn_blocking(|| {
        let now = roots::machine_slug();
        let suggested = now.is_none().then(roots::suggested_slug).flatten();
        (now, suggested)
    })
    .await
    .map_err(|e| e.to_string())?;
    let steps = connect_in(&at, &request, slug_now, suggested, |slug| {
        crate::context_settings::save_machine_slug(&vault, Some(slug))
    })
    .await;
    let rows = tauri::async_runtime::spawn_blocking(|| layer_rows(&Here::read()))
        .await
        .ok();
    // Rows that couldn't be read are never "nothing missing".
    let connected = rows.is_some_and(|rows| connected_now(&steps, &rows));
    if let (true, Some(pair)) = (connected, request.pair.as_deref()) {
        // The clones are done either way: a failed save is logged, never
        // the report lost (the card then stays open to answer again).
        let saved = Offers::for_app().and_then(|o| o.answer(pair, Answer::Connected, Utc::now()));
        if let Err(e) = saved {
            eprintln!("[mac-context] Connected not recorded: {e}");
        }
    }
    Ok(ConnectReport { steps, connected })
}

#[cfg(test)]
#[path = "mac_context_tests.rs"]
mod tests;
