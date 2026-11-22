//! Owner Mac for automations (M4, 2026-11-15): `runs_on:` in a workflow's
//! frontmatter names the one Mac whose scheduler fires it, so a `workflows/`
//! folder synced to two Macs (CX6b) never fires an automation twice.
//!
//! **The value.** A machine id (`m-` and 16 hex digits, M2's `machineId`,
//! stable) or a machine slug (`machine.json`'s `slug`, the person's name for
//! the Mac, which can change). A Mac owns an automation when the value is its
//! id or its slug ([`Seat::is`]). Hollow writes the id ([`Seat::stamp`]); a
//! person may write a slug by hand, and renaming this Mac's slug rewrites the
//! automations that named the old one ([`after_slug_change`]). `this` and
//! `this-mac` are input only ([`for_file`]); stored, they would mean every Mac.
//!
//! **The rule** ([`decide`]):
//! 1. `runs_on` names this Mac: it fires here.
//! 2. It names a Mac paired with this one (M2's records), or this Mac knows
//!    no other Mac yet: it runs there, never here.
//! 3. It names nothing this Mac knows (a malformed value, or a Mac that isn't
//!    this one or a paired one): it fires nowhere.
//! 4. No `runs_on`: it fires here only while this Mac holds the vault alone.
//!    The vault is shared ([`Gate::new`]) once another Mac has been seen: a
//!    paired Mac (either side of M2's peer link), or an automation in the
//!    folder owned by a Mac that isn't this one. A sync remote alone doesn't
//!    count (it may be a backup). Shared, an ownerless automation fires
//!    nowhere.
//!
//! Cases 3 and 4 are surfaced: a Hollow notification once per automation and
//! reason in each run of Hollow, besides `Workflow.runsOn` on the wire.
//!
//! **Where it is checked.** The scheduler's pass (`workflows::enqueue_due`:
//! heal, cron, end, polled triggers) and its startup repair see only the
//! automations that fire here, so a Mac never writes another Mac's
//! automation file either; event triggers (`event_triggers::tick`) bind only
//! those; the run-queue claim skips a cron, trigger or retry row whose
//! automation no longer fires here by the same rule (a synced edit after it
//! was queued, `run_pump::claim_verdict`). Run now is the person's click on
//! this Mac and runs wherever it is clicked. Wakes (`wake_me`) and in-chat
//! pending turns (`automation_in_chat`) are per-Mac state and can't
//! double-fire.
//!
//! **Stamped.** `runs_on` is a permission field (`workflow_permissions`):
//! the Macs hold different logins (the mini has the person's GitHub, Google
//! Cloud, Linear and Claude logins), so moving an automation to another Mac
//! by hand or by sync is held for review like any stamped change. A save
//! through Hollow re-stamps it; a slug rename's rewrite moves only the owner
//! of an otherwise-matching stamp.
//!
//! **This Mac** ([`Seat::current`]) is read once a minute at most (the
//! scheduler, event triggers and claims share it); a slug change forgets it.

use crate::types::Workflow;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

/// The frontmatter key.
pub(crate) const KEY: &str = "runs_on";

/// Input words for "this Mac".
const THIS_MAC: [&str; 2] = ["this", "this-mac"];

/// Another Mac this one knows (M2's pairing records).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Peer {
    pub id: Option<String>,
    pub slug: Option<String>,
    pub name: String,
    /// Slugs it had before a rename that still resolve to it
    /// (`peer_machines::SLUG_ALIAS_SECS`).
    pub aliases: Vec<String>,
}

impl Peer {
    fn is(&self, owner: &str) -> bool {
        self.id.as_deref() == Some(owner)
            || self.slug.as_deref() == Some(owner)
            || self.aliases.iter().any(|a| a == owner)
    }
}

/// Which Mac this is, for the owner check, and the Macs paired with it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Seat {
    pub slug: Option<String>,
    pub id: Option<String>,
    pub peers: Vec<Peer>,
}

#[cfg(test)]
thread_local! {
    /// The seat [`Seat::current`] answers on this test thread; default: no
    /// slug, no id, no peers (never the real `machine.json`).
    static TEST_SEAT: std::cell::RefCell<Seat> = std::cell::RefCell::new(Seat::default());
}

/// Run `f` with `seat` as this thread's [`Seat::current`].
#[cfg(test)]
pub(crate) fn with_seat<T>(seat: Seat, f: impl FnOnce() -> T) -> T {
    let before = TEST_SEAT.with(|s| s.replace(seat));
    let out = f();
    TEST_SEAT.with(|s| *s.borrow_mut() = before);
    out
}

/// How long a [`SeatCache`] copy is used.
const FRESH: std::time::Duration = std::time::Duration::from_secs(60);

/// This Mac's seat as last read, for at most [`FRESH`]. A pairing, an
/// unpairing or a slug change forgets it ([`Seat::forget`]); a read that
/// found nothing is never kept.
pub(crate) struct SeatCache {
    held: Mutex<Option<(std::time::Instant, Seat)>>,
    forgotten: std::sync::atomic::AtomicU64,
}

impl SeatCache {
    pub(crate) const fn new() -> Self {
        Self {
            held: Mutex::new(None),
            forgotten: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// The kept seat while fresh, else `read()`'s, kept only when it
    /// found this Mac. Nothing found reads as [`Seat::default`]: no id, no
    /// slug, no peers, so ownerless automations fire as before M4.
    pub(crate) fn get(&self, read: impl FnOnce() -> Option<Seat>) -> Seat {
        let mut held = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, seat)) = held.as_ref() {
            if at.elapsed() < FRESH {
                return seat.clone();
            }
        }
        match read() {
            Some(seat) => {
                *held = Some((std::time::Instant::now(), seat.clone()));
                seat
            }
            None => {
                *held = None;
                Seat::default()
            }
        }
    }

    pub(crate) fn forget(&self) {
        *self.held.lock().unwrap_or_else(|e| e.into_inner()) = None;
        self.forgotten
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// How many times it was forgotten (tests).
    #[cfg(test)]
    pub(crate) fn forgotten(&self) -> u64 {
        self.forgotten.load(std::sync::atomic::Ordering::SeqCst)
    }
}

/// This process's [`SeatCache`].
pub(crate) static SEAT: SeatCache = SeatCache::new();

impl Seat {
    /// A Mac named `slug` with id `id` and no peers.
    #[cfg(test)]
    pub(crate) fn mac(slug: &str, id: &str) -> Seat {
        Seat {
            slug: Some(slug.to_string()),
            id: Some(id.to_string()),
            peers: Vec::new(),
        }
    }

    /// This seat, paired with `peers`.
    #[cfg(test)]
    pub(crate) fn with_peers(mut self, peers: &[&Seat]) -> Seat {
        self.peers = peers
            .iter()
            .map(|p| Peer {
                id: p.id.clone(),
                slug: p.slug.clone(),
                name: p.slug.clone().unwrap_or_default(),
                aliases: Vec::new(),
            })
            .collect();
        self
    }

    /// This Mac: `machine.json`'s id (written once if missing, as M2 does)
    /// and slug, and the Macs paired with it, read at most once a minute.
    #[cfg(not(test))]
    pub(crate) fn current() -> Seat {
        SEAT.get(Seat::read)
    }

    /// None when this Mac's id can't be read or written (a damaged or
    /// unreadable `machine.json`): the pass then fires as before M4.
    #[cfg(not(test))]
    fn read() -> Option<Seat> {
        let id = match crate::context_roots::ensure_machine_id() {
            Ok(id) => id,
            Err(e) => {
                eprintln!(
                    "[runs_on] this Mac's id can't be read ({e}); automations fire as before"
                );
                return None;
            }
        };
        let mut peers: Vec<Peer> = crate::peer_machines::Machines::for_app()
            .and_then(|m| m.list().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|r| Peer {
                aliases: r.live_old_slugs(chrono::Utc::now()),
                id: Some(r.machine_id),
                slug: r.slug,
                name: r.name,
            })
            .collect();
        peers.extend(
            crate::phone_access::paired_machines()
                .into_iter()
                .map(|(id, name)| Peer {
                    id,
                    slug: None,
                    name,
                    aliases: Vec::new(),
                }),
        );
        Some(Seat {
            slug: crate::context_roots::machine_slug(),
            id: Some(id),
            peers,
        })
    }

    #[cfg(test)]
    pub(crate) fn current() -> Seat {
        TEST_SEAT.with(|s| s.borrow().clone())
    }

    /// Drop [`Seat::current`]'s copy: this Mac's slug changed, or a Mac was
    /// paired or unpaired (`peer_machines`, `phone_access`'s store).
    pub(crate) fn forget() {
        SEAT.forget();
    }

    /// Whether `owner` names this Mac: its id, else its slug.
    pub(crate) fn is(&self, owner: &str) -> bool {
        let owner = owner.trim();
        !owner.is_empty()
            && (self.id.as_deref() == Some(owner) || self.slug.as_deref() == Some(owner))
    }

    /// How Hollow writes this Mac as an owner: its id (stable), else its
    /// slug.
    pub(crate) fn stamp(&self) -> Option<&str> {
        self.id.as_deref().or(self.slug.as_deref())
    }

    fn peer(&self, owner: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.is(owner))
    }
}

/// A well-formed owner: a machine slug or a machine id.
pub(crate) fn valid_owner(value: &str) -> bool {
    crate::context_roots::valid_slug(value).is_ok() || crate::phone_access::valid_machine_id(value)
}

/// Whether an automation fires on this Mac.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Here {
    Fire,
    /// It runs on another Mac (logged, not surfaced: that is its place).
    Elsewhere(String),
    /// It runs on no Mac this one knows (surfaced once).
    Nowhere(String),
}

impl Here {
    fn why(&self) -> Option<&str> {
        match self {
            Here::Fire => None,
            Here::Elsewhere(why) | Here::Nowhere(why) => Some(why),
        }
    }
}

/// The owner rule (module docs) for one automation's `runs_on`.
pub(crate) fn decide(runs_on: Option<&str>, seat: &Seat, shared: bool) -> Here {
    match runs_on.map(str::trim).filter(|v| !v.is_empty()) {
        Some(owner) if seat.is(owner) => Here::Fire,
        Some(owner) if seat.peer(owner).is_some() => {
            let name = seat.peer(owner).map(|p| p.name.as_str()).unwrap_or(owner);
            Here::Elsewhere(format!("runs on {name} ({owner})"))
        }
        // Another Mac's id: its own Hollow knows it.
        Some(owner) if crate::phone_access::valid_machine_id(owner) => {
            Here::Elsewhere(format!("runs on {owner}"))
        }
        Some(owner) if valid_owner(owner) => Here::Nowhere(format!(
            "runs_on {owner} isn't this Mac or a Mac paired with it, so it runs nowhere"
        )),
        Some(owner) => Here::Nowhere(format!(
            "runs_on {owner:?} isn't a Mac (a machine id or slug), so it runs nowhere"
        )),
        None if shared => Here::Nowhere(
            "has no owner Mac (runs_on) and another Mac shares this vault, so it runs nowhere"
                .into(),
        ),
        None => Here::Fire,
    }
}

/// The owner check for one pass over a `workflows/` folder: the seat and
/// whether the folder is shared.
#[derive(Debug, Clone)]
pub(crate) struct Gate {
    seat: Seat,
    shared: bool,
}

impl Gate {
    /// `workflows` is the whole folder (a pass's `vault_workflows::list`).
    /// Shared: a paired Mac, or an automation owned by another Mac's id (when
    /// this Mac knows its own) or by a paired Mac's slug. A malformed or
    /// unknown owner affects only its own automation, never this.
    pub(crate) fn new(seat: Seat, workflows: &[Workflow]) -> Gate {
        let other_mac = |owner: &str| {
            !seat.is(owner)
                && ((seat.id.is_some() && crate::phone_access::valid_machine_id(owner))
                    || seat.peer(owner).is_some())
        };
        let shared = !seat.peers.is_empty()
            || workflows.iter().any(|w| {
                w.runs_on
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|o| !o.is_empty() && other_mac(o))
            });
        Gate { seat, shared }
    }

    pub(crate) fn shared(&self) -> bool {
        self.shared
    }

    pub(crate) fn decide(&self, w: &Workflow) -> Here {
        decide(w.runs_on.as_deref(), &self.seat, self.shared)
    }

    /// [`Gate::decide`], then the ownerless tiebreak
    /// ([`crate::runs_on_claim`]) for an automation with no owner that would
    /// fire here because this Mac knows of no other.
    pub(crate) fn decide_at(&self, vault: &Path, w: &Workflow, now: i64) -> Here {
        let here = self.decide(w);
        tiebreak(here, vault, &self.seat, w, now)
    }

    /// The automations of `workflows` that fire here, in order. A skip is
    /// logged once per automation and reason; the ones that run nowhere and
    /// weren't reported before are one notification for the pass. A Mac that
    /// can't read its own id says once per run that its owned automations
    /// are paused.
    pub(crate) fn keep(&self, vault: &Path, workflows: Vec<Workflow>) -> Vec<Workflow> {
        let mut nowhere: Vec<(String, String)> = Vec::new();
        let mut paused: Vec<String> = Vec::new();
        let now = chrono::Utc::now().timestamp();
        prune_claims(vault, &workflows, now);
        let kept = workflows
            .into_iter()
            .filter(|w| {
                let here = self.decide_at(vault, w, now);
                if here != Here::Fire && w.runs_on.as_deref().is_some_and(|o| !o.trim().is_empty())
                {
                    paused.push(w.title.clone());
                }
                if let Some(why) = here.why() {
                    if first_time(&w.slug, why) {
                        eprintln!("[scheduler] {} {why}", w.slug);
                        if matches!(here, Here::Nowhere(_)) {
                            nowhere.push((w.title.clone(), why.to_string()));
                        }
                    }
                }
                here == Here::Fire
            })
            .collect();
        let known = self.seat.id.is_some() || self.seat.slug.is_some();
        if known && !nowhere.is_empty() {
            notify(vault, &nowhere);
        }
        // An unreadable `machine.json` leaves this Mac unable to tell which
        // automations are its own: the ones with an owner pause, so say so
        // once (not one notification per automation).
        if !known && !paused.is_empty() && first_time("", UNREADABLE) {
            notify_unreadable(vault, &paused);
        }
        kept
    }
}

/// `workflows` (the whole folder) cut to those that fire on this Mac.
pub(crate) fn owned_here(vault: &Path, workflows: Vec<Workflow>) -> Vec<Workflow> {
    let gate = Gate::new(Seat::current(), &workflows);
    gate.keep(vault, workflows)
}

/// The claim's folder-wide fact (whether it is shared) for a vault and
/// seat, kept [`CLAIM_FRESH`] so a run of claims reads the folder once.
static CLAIM_SHARED: Mutex<Option<(std::path::PathBuf, Seat, std::time::Instant, bool)>> =
    Mutex::new(None);
const CLAIM_FRESH: std::time::Duration = std::time::Duration::from_secs(10);

/// The run-queue claim's check: why `w` no longer fires here (the same
/// rule as the scheduler's), or None.
pub(crate) fn claim_skip(vault: &Path, w: &Workflow) -> Option<String> {
    let seat = Seat::current();
    let shared = {
        let mut held = CLAIM_SHARED.lock().unwrap_or_else(|e| e.into_inner());
        match held.as_ref() {
            Some((v, s, at, shared)) if v == vault && *s == seat && at.elapsed() < CLAIM_FRESH => {
                *shared
            }
            _ => {
                let shared = Gate::new(seat.clone(), &crate::vault_workflows::list(vault)).shared();
                *held = Some((
                    vault.to_path_buf(),
                    seat.clone(),
                    std::time::Instant::now(),
                    shared,
                ));
                shared
            }
        }
    };
    let here = decide(w.runs_on.as_deref(), &seat, shared);
    let here = tiebreak(here, vault, &seat, w, chrono::Utc::now().timestamp());
    here.why()
        .map(|why| format!("Not run on this Mac: {} {why}.", w.title))
}

/// An ownerless automation that [`decide`] lets fire here (no other Mac is
/// known) still needs this Mac to be the one of two unpaired Macs that
/// share the folder: [`crate::runs_on_claim`] picks it with no talk between
/// them. Without an id, or if the claim can't be written, it fires as before.
fn tiebreak(here: Here, vault: &Path, seat: &Seat, w: &Workflow, now: i64) -> Here {
    use crate::runs_on_claim::Turn;
    let ownerless = w
        .runs_on
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
        // A manual automation has no schedule to double-fire: no claim.
        && w.schedule != "manual";
    let (Here::Fire, true, Some(me)) = (&here, ownerless, seat.id.as_deref()) else {
        return here;
    };
    match crate::runs_on_claim::turn_for(vault, &w.slug, me, now) {
        Turn::Mine => Here::Fire,
        Turn::Waiting => Here::Elsewhere("is waiting out its claim window (no owner)".into()),
        Turn::Theirs(id) => {
            Here::Elsewhere(format!("has no owner and {id} holds it (lowest id wins)"))
        }
    }
}

/// Automations and reasons already reported in this run of Hollow.
static REPORTED: Mutex<Option<HashSet<String>>> = Mutex::new(None);

fn first_time(slug: &str, why: &str) -> bool {
    REPORTED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(HashSet::new)
        .insert(format!("{slug}\n{why}"))
}

/// Whether `slug` was reported with a reason containing `part` (tests).
#[cfg(test)]
pub(crate) fn reported(slug: &str, part: &str) -> bool {
    REPORTED.lock().unwrap().iter().flatten().any(|k| {
        k.split_once('\n')
            .is_some_and(|(s, w)| s == slug && w.contains(part))
    })
}

/// How many "can't read this Mac" notifications went out (tests: no app is
/// registered there, so none reaches a vault's file).
#[cfg(test)]
static UNREADABLE_SENT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The reason key for the once-per-run "can't read this Mac" notification.
const UNREADABLE: &str = "this Mac's machine.json can't be read";

/// One notification: automations with an owner Mac are paused because this
/// Mac can't read its own id.
fn notify_unreadable(vault: &Path, paused: &[String]) {
    #[cfg(test)]
    UNREADABLE_SENT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let n = paused.len();
    let _ = crate::notifications::create_registered(
        vault,
        crate::notifications::NotificationInput {
            title: format!(
                "{n} automation{} paused: this Mac can't read its id",
                if n == 1 { "" } else { "s" }
            ),
            body: format!(
                "{} {} an owner Mac, and Hollow can't read this Mac's machine.json to tell whether it is this one. They run again once it can be read.",
                paused.join(", "),
                if n == 1 { "has" } else { "have" }
            ),
            kind: "warning".into(),
            source: Some("workflow".into()),
            action_label: None,
            action_url: None,
            notify_native: false,
        },
    );
}

/// One notification for a pass's newly found automations that run nowhere.
fn notify(vault: &Path, nowhere: &[(String, String)]) {
    let n = nowhere.len();
    let lines: Vec<String> = nowhere
        .iter()
        .map(|(title, why)| format!("{title}: {why}."))
        .collect();
    let _ = crate::notifications::create_registered(
        vault,
        crate::notifications::NotificationInput {
            title: format!(
                "{n} automation{} won't run on any Mac",
                if n == 1 { "" } else { "s" }
            ),
            body: format!(
                "{} Set each one's owner: save it with Runs on, or put runs_on: <this Mac's name> in its file.",
                lines.join(" ")
            ),
            kind: "warning".into(),
            source: Some("workflow".into()),
            action_label: None,
            action_url: None,
            notify_native: false,
        },
    );
}

/// A save's `runsOn` as the file stores it: empty removes it (`None`),
/// `this`/`this-mac` is `seat`'s [`Seat::stamp`], anything else must be a
/// machine slug or id.
pub(crate) fn for_file(raw: &str, seat: &Seat) -> Result<Option<String>, String> {
    let value = raw.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if THIS_MAC.contains(&value.to_ascii_lowercase().as_str()) {
        return seat.stamp().map(|n| Some(n.to_string())).ok_or_else(|| {
            "runsOn \"this\" needs this Mac's id or name, and machine.json can't be read. Nothing changed.".to_string()
        });
    }
    if valid_owner(value) {
        Ok(Some(value.to_string()))
    } else {
        Err(format!(
            "runsOn {value:?} isn't a Mac: use \"this\", a machine slug (lower-case letters, digits and single dashes) or a machine id (m- and 16 hex digits). Nothing changed."
        ))
    }
}

/// The owner a new automation gets when its save names none: this Mac,
/// but only once the vault is shared (before that, no owner keeps today's
/// behavior and survives a slug change).
pub(crate) fn default_for_new(vault: &Path, seat: &Seat) -> Option<String> {
    let gate = Gate::new(seat.clone(), &crate::vault_workflows::list(vault));
    gate.shared()
        .then(|| seat.stamp().map(str::to_string))
        .flatten()
}

/// A card line's value: where the automation runs.
pub(crate) fn label(runs_on: Option<&str>, seat: &Seat) -> String {
    match runs_on.map(str::trim).filter(|v| !v.is_empty()) {
        Some(owner) if seat.is(owner) => match seat.slug.as_deref() {
            Some(slug) if slug != owner => format!("this Mac ({slug}, {owner})"),
            _ => format!("this Mac ({owner})"),
        },
        Some(owner) => match seat.peer(owner) {
            Some(peer) => format!("{} ({owner})", peer.name),
            None => owner.to_string(),
        },
        None => "any Mac that holds it, while no other Mac shares the vault (no owner)".into(),
    }
}

/// This Mac's slug changed from `old` to `new` (Settings, Context, Home):
/// forget the cached seat, and move every automation whose `runs_on` named
/// the old slug to the new one (to this Mac's id when the slug was
/// removed), so a rename never strands them. Returns the moved slugs.
pub(crate) fn after_slug_change(
    vault: &Path,
    old: Option<&str>,
    new: Option<&str>,
    id: Option<&str>,
) -> Vec<String> {
    Seat::forget();
    let Some(old) = old.filter(|o| Some(*o) != new) else {
        return Vec::new();
    };
    let Some(owner) = new.or(id) else {
        return Vec::new();
    };
    let mut moved = Vec::new();
    for w in crate::vault_workflows::list(vault) {
        if w.runs_on.as_deref().map(str::trim) != Some(old) {
            continue;
        }
        match crate::vault_workflows::set_owner(vault, &w.slug, Some(owner)) {
            Ok(()) => moved.push(w.slug),
            Err(e) => eprintln!("[runs_on] could not move {} to {owner}: {e}", w.slug),
        }
    }
    // A pass that ran between the first forget and the rewrites cached the
    // new slug beside files still naming the old one: drop that copy too.
    Seat::forget();
    moved
}

/// The claim files of automations that were deleted, got an owner or went
/// manual, and ones no Mac has renewed for days, deleted at most every ten
/// minutes. `workflows` is the whole folder (a short list would prune the
/// rest's claims, so only a pass over the folder calls this).
fn prune_claims(vault: &Path, workflows: &[Workflow], now: i64) {
    static LAST: Mutex<i64> = Mutex::new(0);
    // An empty list may be a failed read: prune nothing rather than every claim.
    if workflows.is_empty() {
        return;
    }
    {
        let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
        if now - *last < 600 {
            return;
        }
        *last = now;
    }
    let keep: HashSet<String> = workflows
        .iter()
        .filter(|w| {
            w.runs_on
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        })
        .filter(|w| w.schedule != "manual")
        .map(|w| w.slug.clone())
        .collect();
    crate::runs_on_claim::prune(vault, &keep, now);
}

/// A paired Mac's `id` announced a new slug (`peer_machines::refresh_slug`
/// returned the `old` one): the seat is forgotten, and every automation
/// whose `runs_on` named `old` is re-stamped to the Mac's id, which a later
/// rename can't strand. Returns the moved slugs.
pub(crate) fn after_peer_rename(vault: &Path, old: &str, id: &str) -> Vec<String> {
    Seat::forget();
    let mut moved = Vec::new();
    for w in crate::vault_workflows::list(vault) {
        if w.runs_on.as_deref().map(str::trim) != Some(old) {
            continue;
        }
        match crate::vault_workflows::set_owner(vault, &w.slug, Some(id)) {
            Ok(()) => moved.push(w.slug),
            Err(e) => eprintln!("[runs_on] could not move {} to {id}: {e}", w.slug),
        }
    }
    Seat::forget();
    moved
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOOK_ID: &str = "m-00000000000000bb";
    const MINI_ID: &str = "m-00000000000000aa";

    fn wf(slug: &str, runs_on: Option<&str>) -> Workflow {
        let head = runs_on
            .map(|o| format!("runs_on: {o}\n"))
            .unwrap_or_default();
        let content = format!("---\ntitle: {slug}\nschedule: 0 9 * * *\n{head}---\nDo it.\n");
        let mut w = crate::vault_workflows::parse_content(slug, &content)
            .unwrap()
            .0;
        w.slug = slug.into();
        w
    }

    fn book() -> Seat {
        Seat::mac("macbook-pro", BOOK_ID)
    }

    fn mini() -> Seat {
        Seat::mac("mini", MINI_ID)
    }

    #[test]
    fn decide_fires_only_on_the_owner() {
        let (book, mini) = (book(), mini());
        assert_eq!(decide(Some(BOOK_ID), &book, true), Here::Fire);
        assert_eq!(decide(Some("macbook-pro"), &book, true), Here::Fire);
        assert_eq!(
            decide(Some(&format!(" {MINI_ID} ")), &mini, false),
            Here::Fire
        );
        assert!(matches!(
            decide(
                Some("macbook-pro"),
                &mini.clone().with_peers(&[&book]),
                true
            ),
            Here::Elsewhere(_)
        ));
        // Malformed owners fire nowhere and are surfaced.
        for owner in ["Mac Mini", "a--b"] {
            for seat in [&book, &mini] {
                let here = decide(Some(owner), seat, false);
                assert!(matches!(here, Here::Nowhere(_)), "{owner} on {seat:?}");
            }
        }
        // A slug that isn't this Mac or a paired one runs nowhere; another
        // Mac's id runs there.
        assert!(matches!(
            decide(Some("ghost"), &book, false),
            Here::Nowhere(_)
        ));
        assert!(matches!(
            decide(Some(MINI_ID), &book, false),
            Here::Elsewhere(_)
        ));
        let paired = book.clone().with_peers(&[&mini]);
        assert!(matches!(
            decide(Some("ghost"), &paired, true),
            Here::Nowhere(_)
        ));
        assert!(matches!(
            decide(Some(MINI_ID), &paired, true),
            Here::Elsewhere(why) if why.contains("mini")
        ));
        // A Mac with no id or name owns nothing.
        assert!(!matches!(
            decide(Some("macbook-pro"), &Seat::default(), false),
            Here::Fire
        ));
        // No owner: today's behavior alone, nowhere (surfaced) once shared.
        assert_eq!(decide(None, &book, false), Here::Fire);
        assert_eq!(decide(Some("  "), &Seat::default(), false), Here::Fire);
        assert!(matches!(decide(None, &book, true), Here::Nowhere(_)));
    }

    /// This Mac's claim on `slug` made long ago, so it has matured.
    fn matured(v: &Path, slug: &str, id: &str) {
        let old = chrono::Utc::now().timestamp() - 1000;
        crate::runs_on_claim::turn_for(v, slug, id, old);
    }

    fn kept(gate: &Gate, v: &Path, folder: &[Workflow]) -> Vec<String> {
        gate.keep(v, folder.to_vec())
            .into_iter()
            .map(|w| w.slug)
            .collect()
    }

    #[test]
    fn the_vault_is_shared_once_another_mac_is_seen_never_by_a_remote_alone() {
        let v = tempfile::TempDir::new().unwrap();
        let alone = vec![wf("sh-a", None), wf("sh-b", Some(BOOK_ID))];
        let gate = Gate::new(book(), &alone);
        assert!(!gate.shared());
        matured(v.path(), "sh-a", BOOK_ID);
        assert_eq!(kept(&gate, v.path(), &alone), vec!["sh-a", "sh-b"]);

        let with_mini = vec![wf("sh-c", None), wf("sh-d", Some(MINI_ID))];
        let gate = Gate::new(book(), &with_mini);
        assert!(gate.shared(), "another Mac's id in the folder");
        assert!(kept(&gate, v.path(), &with_mini).is_empty());

        let paired = book().with_peers(&[&mini()]);
        let gate = Gate::new(paired.clone(), &alone);
        assert!(gate.shared(), "a paired Mac");
        assert_eq!(kept(&gate, v.path(), &alone), vec!["sh-b"]);
        assert!(reported("sh-a", "no owner Mac"), "surfaced once");
        // A paired Mac's slug counts too.
        let by_slug = vec![wf("sh-e", None), wf("sh-f", Some("mini"))];
        let lone = Seat {
            peers: Vec::new(),
            ..paired
        };
        assert!(
            !Gate::new(lone, &by_slug).shared(),
            "an unknown slug is no Mac"
        );
    }

    /// Review round 2: a typo or an unknown owner stops only its own
    /// automation; the solo user's ownerless ones keep firing.
    #[test]
    fn a_malformed_or_unknown_owner_never_shares_the_vault() {
        let v = tempfile::TempDir::new().unwrap();
        let folder = vec![
            wf("solo-none", None),
            wf("solo-typo", Some("Mac Mini")),
            wf("solo-old-slug", Some("old-name")),
        ];
        matured(v.path(), "solo-none", BOOK_ID);
        for seat in [book(), Seat::default(), Seat { id: None, ..book() }] {
            let gate = Gate::new(seat.clone(), &folder);
            assert!(!gate.shared(), "{seat:?}");
            assert_eq!(
                kept(&gate, v.path(), &folder),
                vec!["solo-none"],
                "{seat:?}"
            );
        }
        assert!(reported("solo-typo", "isn't a Mac"));
        assert!(reported("solo-old-slug", "isn't this Mac"));
        // With no id of its own, this Mac can't tell another Mac's id from
        // a stale one of its own: not evidence either.
        let unknown = Seat::default();
        let gate = Gate::new(
            unknown,
            &[wf("solo-id", Some(MINI_ID)), wf("solo-n2", None)],
        );
        assert!(!gate.shared());
    }

    /// Review round 2: a read that found nothing is never kept, so the next
    /// pass reads again; a found seat is kept until forgotten.
    #[test]
    fn the_seat_cache_keeps_only_a_found_seat_and_forgets_on_demand() {
        let cache = SeatCache::new();
        let reads = std::cell::Cell::new(0);
        let none = || {
            reads.set(reads.get() + 1);
            None
        };
        assert_eq!(
            cache.get(none),
            Seat::default(),
            "nothing found: fire as before"
        );
        assert_eq!(cache.get(none), Seat::default());
        assert_eq!(reads.get(), 2, "an empty read is not kept");
        let found = || {
            reads.set(reads.get() + 1);
            Some(book())
        };
        assert_eq!(cache.get(found), book());
        assert_eq!(cache.get(found), book());
        assert_eq!(reads.get(), 3, "a found seat is kept");
        cache.forget();
        assert_eq!(cache.get(found), book());
        assert_eq!(reads.get(), 4, "forgotten, it reads again");
        assert_eq!(cache.forgotten(), 1);
    }

    /// Review round 2: pairing or forgetting a Mac on either side of M2's
    /// link forgets the seat, so `shared` follows at once.
    #[test]
    fn pairing_and_unpairing_a_mac_forget_the_seat() {
        let dir = tempfile::TempDir::new().unwrap();
        let before = SEAT.forgotten();
        let peers = crate::peer_machines::Machines::at(dir.path());
        let record: crate::peer_machines::MachineRecord =
            serde_json::from_value(serde_json::json!({
                "machineId": MINI_ID,
                "slug": "mini",
                "name": "Mac mini",
                "sshHosts": ["mini"],
                "deviceId": "mc-0000000a",
                "pairedAt": "2026-11-15T00:00:00Z",
            }))
            .unwrap();
        peers.upsert(record).unwrap();
        assert!(SEAT.forgotten() > before, "a paired Mac");
        let mid = SEAT.forgotten();
        peers.forget(MINI_ID).unwrap();
        assert!(SEAT.forgotten() > mid, "a forgotten Mac");
    }

    #[test]
    fn two_seats_over_one_folder_fire_each_owned_automation_exactly_once() {
        let folder = vec![
            wf("the-hollow-herald", Some(BOOK_ID)),
            wf("herald-by-slug", Some("macbook-pro")),
            wf("nightly-build", Some("mini")),
            wf("by-id", Some(MINI_ID)),
            wf("no-owner", None),
            wf("ghost-owner", Some("ghost")),
            wf("bad-owner", Some("Not A Slug")),
        ];
        let v = tempfile::TempDir::new().unwrap();
        let fired = |seat: Seat| -> Vec<String> {
            Gate::new(seat, &folder)
                .keep(v.path(), folder.clone())
                .into_iter()
                .map(|w| w.slug)
                .collect()
        };
        let book_seat = book().with_peers(&[&mini()]);
        let mini_seat = mini().with_peers(&[&book()]);
        assert_eq!(
            fired(book_seat),
            vec!["the-hollow-herald", "herald-by-slug"]
        );
        assert_eq!(fired(mini_seat), vec!["nightly-build", "by-id"]);
        assert!(reported("ghost-owner", "isn't this Mac or a Mac paired"));
        assert!(reported("bad-owner", "isn't a Mac"));
        assert!(reported("no-owner", "no owner Mac"));
    }

    /// Review round 3: a Mac that can't read its own `machine.json` pauses
    /// the automations that name an owner, and says so once (the ownerless
    /// ones keep firing); it never goes quiet.
    #[test]
    fn an_unreadable_seat_pauses_owned_automations_and_says_so_once() {
        let folder = vec![
            wf("unread-owned-a", Some(MINI_ID)),
            wf("unread-owned-b", Some("macbook-pro")),
            wf("unread-free", None),
        ];
        let v = tempfile::TempDir::new().unwrap();
        let fired = |gate: &Gate| -> Vec<String> {
            gate.keep(v.path(), folder.clone())
                .into_iter()
                .map(|w| w.slug)
                .collect()
        };
        let gate = Gate::new(Seat::default(), &folder);
        assert_eq!(fired(&gate), vec!["unread-free"]);
        assert!(reported("", UNREADABLE), "one notice for the whole pass");
        // A second pass still pauses them, and no second notice goes out:
        // exactly one was sent over both passes (counted where it is sent:
        // with no app registered in tests, none reaches the vault's file).
        // No other test reads an unreadable seat.
        assert_eq!(fired(&gate), vec!["unread-free"]);
        assert_eq!(UNREADABLE_SENT.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn for_file_resolves_this_to_the_id_and_refuses_what_names_no_mac() {
        let book = book();
        assert_eq!(for_file("", &book), Ok(None));
        assert_eq!(for_file("  ", &book), Ok(None));
        assert_eq!(for_file("this", &book), Ok(Some(BOOK_ID.into())));
        assert_eq!(for_file("This-Mac", &book), Ok(Some(BOOK_ID.into())));
        let id_less = Seat {
            id: None,
            ..book.clone()
        };
        assert_eq!(for_file("this", &id_less), Ok(Some("macbook-pro".into())));
        assert!(for_file("this", &Seat::default()).is_err());
        assert_eq!(for_file(" mini ", &book), Ok(Some("mini".into())));
        assert_eq!(for_file(MINI_ID, &book), Ok(Some(MINI_ID.into())));
        for bad in ["Mini", "mac mini", "-mini", "m_123", "a--b"] {
            assert!(for_file(bad, &book).is_err(), "{bad}");
        }
    }

    fn input(title: &str, runs_on: Option<&str>) -> crate::types::WorkflowInput {
        serde_json::from_value(serde_json::json!({
            "title": title,
            "schedule": "manual",
            "prompt": "Do it.",
            "runsOn": runs_on,
        }))
        .unwrap()
    }

    fn owner_of(v: &Path, slug: &str) -> Option<String> {
        crate::vault_workflows::read_full(v, slug)
            .unwrap()
            .0
            .runs_on
    }

    #[test]
    fn a_new_automation_is_stamped_only_once_the_vault_is_shared() {
        use crate::vault_workflows::upsert_input;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let w = with_seat(book(), || {
            upsert_input(v, None, &input("Owner Alone M4", None)).unwrap()
        });
        assert_eq!(w.runs_on, None, "alone, a new automation has no owner");
        let w = with_seat(book().with_peers(&[&mini()]), || {
            upsert_input(v, None, &input("Owner Paired M4", None)).unwrap()
        });
        assert_eq!(w.runs_on.as_deref(), Some(BOOK_ID), "shared: this Mac's id");
    }

    #[test]
    fn a_save_keeps_changes_or_removes_the_owner_and_restamps_it() {
        use crate::vault_workflows::upsert_input;
        use crate::workflow_permissions::verify;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let file =
            |slug: &str| std::fs::read_to_string(v.join(format!("workflows/{slug}.md"))).unwrap();
        with_seat(book(), || {
            let w = upsert_input(v, None, &input("Owner Save M4", Some("this"))).unwrap();
            assert_eq!(w.runs_on.as_deref(), Some(BOOK_ID));
            assert!(file(&w.slug).contains(&format!("\nruns_on: {BOOK_ID}\n")));
            assert!(verify(v, &w.slug).is_ok(), "a save stamps its owner");
            upsert_input(v, Some(&w.slug), &input("Owner Save M4", None)).unwrap();
            assert_eq!(
                owner_of(v, &w.slug).as_deref(),
                Some(BOOK_ID),
                "omitted keeps it"
            );
            upsert_input(v, Some(&w.slug), &input("Owner Save M4", Some("mini"))).unwrap();
            assert_eq!(owner_of(v, &w.slug).as_deref(), Some("mini"));
            assert!(verify(v, &w.slug).is_ok());
            upsert_input(v, Some(&w.slug), &input("Owner Save M4", Some(""))).unwrap();
            assert_eq!(owner_of(v, &w.slug), None);
            assert!(!file(&w.slug).contains("runs_on"));
            let before = file(&w.slug);
            let err = upsert_input(v, Some(&w.slug), &input("Owner Save M4", Some("Mac Mini")))
                .unwrap_err();
            assert!(err.contains("isn't a Mac"), "{err}");
            assert_eq!(file(&w.slug), before);
        });
    }

    #[test]
    fn a_hand_edit_of_the_owner_is_held_for_review() {
        use crate::workflow_permissions::verify;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let w = with_seat(book(), || {
            crate::vault_workflows::upsert_input(v, None, &input("Owner Hand M4", Some("this")))
                .unwrap()
        });
        let path = v.join(&w.path);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace(BOOK_ID, MINI_ID)).unwrap();
        assert!(verify(v, &w.slug).is_err(), "moved to another Mac by hand");
    }

    #[test]
    fn a_slug_rename_or_removal_moves_the_automations_that_named_it() {
        use crate::workflow_permissions::verify;
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        crate::workflow_permissions::initialize(v).unwrap();
        let (by_slug, by_id, other) = with_seat(book(), || {
            let save = |t: &str, o: &str| {
                crate::vault_workflows::upsert_input(v, None, &input(t, Some(o))).unwrap()
            };
            (
                save("Owner Rename Slug M4", "macbook-pro"),
                save("Owner Rename Id M4", "this"),
                save("Owner Rename Other M4", "mini"),
            )
        });
        // Renamed: the slug-named one follows; the id-named one never
        // needed to; another Mac's is untouched. Each stays approved.
        let moved = after_slug_change(v, Some("macbook-pro"), Some("air"), Some(BOOK_ID));
        assert_eq!(moved, vec![by_slug.slug.clone()]);
        assert_eq!(owner_of(v, &by_slug.slug).as_deref(), Some("air"));
        assert_eq!(owner_of(v, &by_id.slug).as_deref(), Some(BOOK_ID));
        assert_eq!(owner_of(v, &other.slug).as_deref(), Some("mini"));
        assert!(
            verify(v, &by_slug.slug).is_ok(),
            "the rewrite re-stamps the owner"
        );
        let renamed = Seat::mac("air", BOOK_ID);
        let folder = crate::vault_workflows::list(v);
        let gate = Gate::new(renamed, &folder);
        for w in [&by_slug, &by_id] {
            let w = folder.iter().find(|f| f.slug == w.slug).unwrap();
            assert_eq!(gate.decide(w), Here::Fire, "{} still fires here", w.slug);
        }
        // Removed: it moves to this Mac's id.
        let moved = after_slug_change(v, Some("air"), None, Some(BOOK_ID));
        assert_eq!(moved, vec![by_slug.slug.clone()]);
        assert_eq!(owner_of(v, &by_slug.slug).as_deref(), Some(BOOK_ID));
        let unnamed = Seat {
            slug: None,
            ..book()
        };
        let folder = crate::vault_workflows::list(v);
        let w = folder.iter().find(|f| f.slug == by_slug.slug).unwrap();
        assert_eq!(Gate::new(unnamed, &folder).decide(w), Here::Fire);
        // Nothing to move to, or no change: nothing written.
        assert!(after_slug_change(v, Some("x"), None, None).is_empty());
        assert!(after_slug_change(v, Some("mini"), Some("mini"), Some(BOOK_ID)).is_empty());
    }

    #[test]
    fn claim_skip_follows_the_folder_rule() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        let owned = wf("x", Some("mini"));
        let none = wf("y", None);
        matured(v, "y", MINI_ID);
        with_seat(mini(), || {
            assert_eq!(claim_skip(v, &owned), None);
            assert_eq!(claim_skip(v, &none), None, "alone, ownerless runs");
        });
        with_seat(book().with_peers(&[&mini()]), || {
            let why = claim_skip(v, &owned).expect("owned elsewhere");
            assert!(why.contains("runs on mini"), "{why}");
        });
        with_seat(book(), || {
            let why = claim_skip(v, &owned).expect("an unknown slug");
            assert!(why.contains("runs nowhere"), "{why}");
        });
        with_seat(book().with_peers(&[&mini()]), || {
            let why = claim_skip(v, &none).expect("shared, ownerless");
            assert!(why.contains("no owner Mac"), "{why}");
        });
    }

    #[test]
    fn an_owned_automation_never_touches_a_claim_and_an_ownerless_one_waits() {
        let v = tempfile::TempDir::new().unwrap();
        let now = 1_800_000_000;
        let gate = Gate::new(book(), &[]);
        let owned = wf("owned-x", Some(BOOK_ID));
        assert_eq!(gate.decide_at(v.path(), &owned, now), Here::Fire);
        assert!(crate::runs_on_claim::read_claims(v.path(), "owned-x", now).is_empty());
        let free = wf("free-x", None);
        assert_eq!(
            gate.decide_at(v.path(), &free, now),
            Here::Fire,
            "alone: at once"
        );
        let later = now + crate::runs_on_claim::WINDOW_SECS;
        assert_eq!(gate.decide_at(v.path(), &free, later), Here::Fire);
        // The other Mac, lower id, with its own claim in the same folder.
        let low = Seat::mac("mini", MINI_ID);
        let gate_low = Gate::new(low, &[]);
        gate_low.decide_at(v.path(), &free, now);
        assert!(matches!(
            gate.decide_at(v.path(), &free, later),
            Here::Elsewhere(why) if why.contains(MINI_ID)
        ));
        assert_eq!(gate_low.decide_at(v.path(), &free, later), Here::Fire);
    }

    fn peer_record(slug: Option<&str>) -> crate::peer_machines::MachineRecord {
        serde_json::from_value(serde_json::json!({
            "machineId": MINI_ID, "slug": slug, "name": "Mac mini",
            "sshHosts": ["mini"], "deviceId": "mc-0000000a",
            "pairedAt": "2026-11-15T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn a_peer_rename_updates_the_record_keeps_the_old_slug_and_restamps_owners() {
        let dir = tempfile::TempDir::new().unwrap();
        let machines = crate::peer_machines::Machines::at(dir.path());
        machines.upsert(peer_record(Some("mini"))).unwrap();
        let now = chrono::Utc::now();
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("mini"), Some("macbook-pro"), now),
            None,
            "same name"
        );
        let old = machines.refresh_slug(MINI_ID, Some("studio"), Some("macbook-pro"), now);
        assert_eq!(old.as_deref(), Some("mini"));
        let rec = machines.get(MINI_ID).unwrap().unwrap();
        assert_eq!(rec.slug.as_deref(), Some("studio"));
        assert_eq!(rec.live_old_slugs(now), vec!["mini"]);
        // The old slug still resolves to the peer, so a stamp naming it runs there.
        let seat = book().with_peers(&[&mini()]);
        let mut seat = seat;
        seat.peers[0].slug = rec.slug.clone();
        seat.peers[0].aliases = rec.live_old_slugs(now);
        assert!(matches!(
            decide(Some("mini"), &seat, true),
            Here::Elsewhere(_)
        ));
        assert!(matches!(
            decide(Some("studio"), &seat, true),
            Here::Elsewhere(_)
        ));
        // An announced slug Hollow wouldn't accept changes nothing.
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("Not A Slug"), Some("macbook-pro"), now),
            None
        );
        assert_eq!(
            machines.get(MINI_ID).unwrap().unwrap().slug.as_deref(),
            Some("studio")
        );
    }

    #[test]
    fn a_peer_slug_alias_expires() {
        let dir = tempfile::TempDir::new().unwrap();
        let machines = crate::peer_machines::Machines::at(dir.path());
        machines.upsert(peer_record(Some("mini"))).unwrap();
        let now = chrono::Utc::now();
        machines.refresh_slug(MINI_ID, Some("studio"), Some("macbook-pro"), now);
        let rec = machines.get(MINI_ID).unwrap().unwrap();
        let edge = now + chrono::Duration::seconds(crate::peer_machines::SLUG_ALIAS_SECS);
        assert_eq!(
            rec.live_old_slugs(edge - chrono::Duration::seconds(1)),
            vec!["mini"]
        );
        assert!(rec
            .live_old_slugs(edge + chrono::Duration::seconds(1))
            .is_empty());
        // The next announcement prunes it.
        machines.refresh_slug(
            MINI_ID,
            Some("studio"),
            Some("macbook-pro"),
            edge + chrono::Duration::seconds(1),
        );
        assert!(machines.get(MINI_ID).unwrap().unwrap().old_slugs.is_empty());
    }

    #[test]
    fn after_peer_rename_moves_owners_naming_the_old_slug_to_the_peer_id() {
        let dir = tempfile::TempDir::new().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("workflows")).unwrap();
        for (slug, owner) in [("old-owner", "mini"), ("other", "somebody-else")] {
            std::fs::write(
                v.join("workflows").join(format!("{slug}.md")),
                format!("---\ntitle: {slug}\nschedule: manual\nruns_on: {owner}\n---\nDo it.\n"),
            )
            .unwrap();
        }
        let moved = after_peer_rename(v, "mini", MINI_ID);
        assert_eq!(moved, vec!["old-owner"]);
        let got: Vec<_> = crate::vault_workflows::list(v)
            .into_iter()
            .map(|w| (w.slug, w.runs_on))
            .collect();
        assert!(got.contains(&("old-owner".into(), Some(MINI_ID.into()))));
        assert!(got.contains(&("other".into(), Some("somebody-else".into()))));
    }

    /// Review: a peer can't take another Mac's name and rename away to drag
    /// that Mac's owners to itself, and an absent slug is no information.
    #[test]
    fn a_peer_cannot_hijack_another_macs_slug_or_clear_its_own() {
        let dir = tempfile::TempDir::new().unwrap();
        let machines = crate::peer_machines::Machines::at(dir.path());
        let other = "m-00000000000000cc";
        machines.upsert(peer_record(Some("mini"))).unwrap();
        let mut victim = peer_record(Some("studio"));
        victim.machine_id = other.into();
        victim.device_id = "mc-0000000c".into();
        machines.upsert(victim).unwrap();
        let now = chrono::Utc::now();
        let get = |id: &str| machines.get(id).unwrap().unwrap().slug;
        // Taking a paired Mac's slug, or this Mac's, changes nothing.
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("studio"), Some("macbook-pro"), now),
            None
        );
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("macbook-pro"), Some("macbook-pro"), now),
            None
        );
        assert_eq!(get(MINI_ID).as_deref(), Some("mini"));
        // The victim renames; its old slug stays an alias nobody else can take.
        assert_eq!(
            machines
                .refresh_slug(other, Some("studio-2"), Some("macbook-pro"), now)
                .as_deref(),
            Some("studio")
        );
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("studio"), Some("macbook-pro"), now),
            None
        );
        assert_eq!(get(MINI_ID).as_deref(), Some("mini"));
        // No slug announced: neither a clear nor a rename.
        assert_eq!(
            machines.refresh_slug(MINI_ID, None, Some("macbook-pro"), now),
            None
        );
        assert_eq!(
            machines.refresh_slug(MINI_ID, Some("  "), Some("macbook-pro"), now),
            None
        );
        assert_eq!(get(MINI_ID).as_deref(), Some("mini"));
        // A Mac whose old name another Mac holds gets no re-stamp (None).
        let mut dup = peer_record(Some("dup"));
        dup.machine_id = "m-00000000000000dd".into();
        dup.device_id = "mc-0000000d".into();
        machines.upsert(dup).unwrap();
        // Legacy state: a second record already naming "mini" is possible.
        let mut twin = peer_record(Some("mini"));
        twin.machine_id = "m-00000000000000ee".into();
        twin.device_id = "mc-0000000e".into();
        machines.upsert(twin).unwrap();
        assert_eq!(
            machines.refresh_slug(
                "m-00000000000000ee",
                Some("twin-new"),
                Some("macbook-pro"),
                now
            ),
            None,
            "its old name is still held by another Mac"
        );
    }

    #[test]
    fn claims_are_checked_pruned_and_skip_manual_automations() {
        use crate::runs_on_claim as c;
        let v = tempfile::TempDir::new().unwrap();
        let now = 1_800_000_000;
        // A claim whose file name isn't its own id is ignored.
        std::fs::create_dir_all(v.path().join(c::DIR)).unwrap();
        let forged = format!(r#"{{"id":"{MINI_ID}","since":0,"beat":{now}}}"#);
        std::fs::write(
            v.path().join(c::DIR).join(format!("a.{BOOK_ID}.json")),
            forged,
        )
        .unwrap();
        assert!(c::read_claims(v.path(), "a", now).is_empty());
        // A beat in the future is clamped to now.
        let future = format!(
            r#"{{"id":"{MINI_ID}","since":{now},"beat":{}}}"#,
            now + 99_999
        );
        std::fs::write(
            v.path().join(c::DIR).join(format!("b.{MINI_ID}.json")),
            future,
        )
        .unwrap();
        let got = c::read_claims(v.path(), "b", now);
        assert_eq!(got[0].beat, now);
        // Manual automations take no claim.
        let gate = Gate::new(book(), &[]);
        let mut manual = wf("man", None);
        manual.schedule = "manual".into();
        assert_eq!(gate.decide_at(v.path(), &manual, now), Here::Fire);
        assert!(c::read_claims(v.path(), "man", now).is_empty());
        // Prune: unknown automation, owned now, and old beats go; live stays.
        let live = wf("live", None);
        gate.decide_at(v.path(), &live, now);
        gate.decide_at(v.path(), &wf("gone", None), now);
        let old = format!(
            r#"{{"id":"{MINI_ID}","since":0,"beat":{}}}"#,
            now - c::PRUNE_SECS - 1
        );
        std::fs::write(
            v.path().join(c::DIR).join(format!("live.{MINI_ID}.json")),
            old,
        )
        .unwrap();
        let keep: HashSet<String> = ["live".to_string()].into();
        assert_eq!(
            c::prune(v.path(), &keep, now),
            4,
            "a, b, gone and the old one; live's own stays"
        );
        assert_eq!(c::read_claims(v.path(), "live", now).len(), 1);
        assert!(c::read_claims(v.path(), "gone", now).is_empty());
    }
}
