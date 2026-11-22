//! A2 (2026-11-15): account usage across paired Macs, so two Hollows that
//! spread runs over the same logins don't both pick the same account.
//!
//! **The problem.** The same Claude login can be used on this Mac and on the
//! Mac mini. Each Hollow measures its accounts alone and picks the
//! least-loaded one (`claude_accounts::pick`), so within one stale
//! measurement window both can pick the same account: double-booking.
//!
//! **Serving** ([`local_accounts`]): `GET /v1/machine/usage` (capability
//! `usage`, machine token only, `docs/phone-api/machines.md`) answers this
//! Hollow's snapshot: per account a shared key, label, provider, the load of
//! its fuller live window, whether it is out of allowance and until when,
//! when that was measured, and the runs this Hollow started on it in the
//! last ten minutes. No token, email or account id crosses.
//!
//! **Shared keys** ([`shared_key`]): `ak-` and 16 hex digits of a SHA-256 of
//! something both Macs can read for the same login and that is not a secret:
//! a Claude account's organisation uuid (the `anthropic-organization-id`
//! header of an added account's usage probe, or `~/.claude.json`'s
//! `oauthAccount.organizationUuid` for the Claude Code login), and for Codex
//! the ChatGPT user and workspace ids in the sign-in's `id_token` claims.
//! An organisation can hold several seats, so [`merge`] uses a key only when
//! it names one account on each side, and not when both sides know the
//! seven-day window's reset and they differ by more than an hour (two seats).
//!
//! **Fetching** ([`cached`]): only peers whose `GET /v1/machine` lists
//! `usage`, in the background (`Hub::usage_snapshots`), at most every
//! [`REFRESH_AFTER_SECS`], kept [`FRESH_SECS`]. A run start never waits: it
//! reads what is held, and a peer that is down or older contributes nothing.
//!
//! **Choosing** ([`merge`], pure): for an account known by the same key on
//! both Macs, the fresher measurement wins and a peer's out-of-allowance
//! flag counts; a peer pick in the last ten minutes breaks equal-load ties
//! only. It never changes measured load or outranks more headroom. With no peer data
//! the candidates come back unchanged.

use crate::claude_accounts::Candidate;
use crate::phone_access as wire;
use chrono::{DateTime, SecondsFormat, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

/// A peer's snapshot is used for this long after it was fetched.
pub(crate) const FRESH_SECS: i64 = 180;
/// A new fetch starts at most this often (only when a pick asks).
pub(crate) const REFRESH_AFTER_SECS: i64 = 45;
/// A pick counts as recent for this long.
pub(crate) const RECENT_PICK_SECS: i64 = 10 * 60;
/// Two measurements of one account's seven-day reset this far apart mean
/// two accounts (seats of one organisation).
const SAME_RESET_SECS: i64 = 60 * 60;
/// A peer's clock may run this far ahead of ours.
const CLOCK_SKEW_SECS: i64 = 5 * 60;
/// A peer's reading of an account is used only while it is at most this
/// old (its `measuredAt`, a future one counted as now).
pub(crate) const READING_MAX_SECS: i64 = 5 * 60;
/// A peer's block counts until its `blockedUntil`, at most this far ahead
/// (one weekly window).
const BLOCK_MAX_SECS: i64 = 7 * 24 * 3600 + 3600;
/// The whole background fetch gives up after this long.
const REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const KEY_DOMAIN: &str = "hollow-account-key/1";
/// The usage shape this client reads (the server's `wire::USAGE_API`).
pub(crate) const USAGE_API: u32 = 1;
/// A snapshot's accounts kept, at most (the server's
/// `wire::USAGE_ACCOUNTS_MAX`).
pub(crate) const ACCOUNTS_MAX: usize = 16;
const TEXT_MAX: usize = 80;

/// The opaque key both Macs compute for one login: `ak-` and the first 16
/// hex digits of SHA-256 over a domain, the kind of id and the id. `kind` is
/// `claude-org` or `codex-user`; `id` is never a credential.
pub(crate) fn shared_key(kind: &str, id: &str) -> String {
    let digest = Sha256::digest(format!("{KEY_DOMAIN}\n{kind}\n{}", id.trim()).as_bytes());
    let mut out = String::from("ak-");
    for byte in &digest[..8] {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A uuid-like id from a header or a file: hex digits and dashes, 8 to 64.
pub(crate) fn plain_id(raw: &str) -> Option<String> {
    let id = raw.trim();
    ((8..=64).contains(&id.len()) && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        .then(|| id.to_ascii_lowercase())
}

pub(crate) fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn parse_time(text: Option<&str>) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Text for the wire: one line, at most [`TEXT_MAX`] characters, and
/// empty when it holds an `@` (a label that is an email never crosses).
pub(crate) fn wire_text(text: &str) -> String {
    if text.contains('@') {
        return String::new();
    }
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(TEXT_MAX)
        .collect()
}

// ── The client's reading of `GET /v1/machine/usage` ──────────────────────
//
// Every field has a default, so a field a later Hollow drops reads as
// "unknown" and one it adds is ignored.

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PeerUsage {
    pub usage_api: u32,
    pub machine_id: String,
    pub observed_at: Option<String>,
    pub accounts: Vec<PeerAccount>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct PeerAccount {
    pub key: String,
    pub provider: String,
    pub label: String,
    pub billing: Option<String>,
    pub routed: bool,
    pub load_percent: Option<f64>,
    pub blocked: bool,
    pub blocked_until: Option<String>,
    pub seven_day_resets_at: Option<String>,
    pub measured_at: Option<String>,
    pub recent_picks: u32,
    pub last_picked_at: Option<String>,
}

// ── Choosing ─────────────────────────────────────────────────────────────

/// One of this Hollow's Claude accounts as [`merge`] sees it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Local {
    pub candidate: Candidate,
    /// Its [`shared_key`], once known.
    pub key: Option<String>,
    /// When its load was measured here.
    pub measured_at: Option<DateTime<Utc>>,
    /// Its seven-day window's reset (unix seconds), when measured.
    pub seven_day_resets_at: Option<i64>,
}

/// This Hollow's candidates with what paired Macs know folded in (see the
/// module docs). `ceiling` is the routing's (chat) ceiling. With no peer
/// snapshot, exactly the local candidates.
pub(crate) fn merge(
    local: Vec<Local>,
    peers: &[PeerUsage],
    ceiling: f64,
    now: DateTime<Utc>,
) -> Vec<Candidate> {
    if peers.is_empty() {
        return local.into_iter().map(|l| l.candidate).collect();
    }
    let mut local = local;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for key in local.iter().filter_map(|l| l.key.as_deref()) {
        *counts.entry(key).or_default() += 1;
    }
    let unique: Vec<Option<String>> = local
        .iter()
        .map(|l| l.key.clone().filter(|k| counts.get(k.as_str()) == Some(&1)))
        .collect();
    let mut busy = vec![false; local.len()];
    for (i, account) in local.iter_mut().enumerate() {
        let Some(key) = unique[i].as_deref() else {
            continue;
        };
        for peer in peers {
            let mut matches = peer
                .accounts
                .iter()
                .filter(|p| p.key == key && p.provider == "claude");
            let (Some(theirs), None) = (matches.next(), matches.next()) else {
                continue;
            };
            // Their reading counts only as the same account's: both sides
            // know the weekly reset and it agrees. Seats of one organisation
            // share its key, and a seat with no window yet would otherwise
            // take another seat's load or block. A known disagreement means
            // another account outright, picks included.
            let their_reset = parse_time(theirs.seven_day_resets_at.as_deref());
            let same = match (account.seven_day_resets_at, their_reset) {
                (Some(ours), Some(their)) => {
                    if (ours - their.timestamp()).abs() > SAME_RESET_SECS {
                        continue;
                    }
                    true
                }
                _ => false,
            };
            if same {
                if let Some(reading) = reading(theirs, now) {
                    if let Some((at, load)) = reading.load {
                        if account.measured_at.is_none_or(|ours| at > ours) {
                            account.candidate.load = Some(load);
                            account.measured_at = Some(at);
                        }
                    }
                    if reading.blocked {
                        account.candidate.blocked = true;
                    }
                }
            }
            let picked = parse_time(theirs.last_picked_at.as_deref()).is_some_and(|at| {
                let age = (now - at).num_seconds();
                (-CLOCK_SKEW_SECS..=RECENT_PICK_SECS).contains(&age)
            });
            if theirs.recent_picks > 0 && picked {
                busy[i] = true;
            }
        }
    }
    let _ = ceiling;
    // A peer's recent pick breaks ties, never invents load that outranks
    // measured headroom on either machine.
    for (account, _) in local.iter_mut().zip(&busy).filter(|(_, busy)| **busy) {
        account.candidate.last_used = u64::MAX;
    }
    local.into_iter().map(|l| l.candidate).collect()
}

/// A peer's reading, checked: a load in 0..=100, used while measured at
/// most [`READING_MAX_SECS`] ago (a future time counts as now, so it never
/// beats a fresher local one), and blocked only with a load and a
/// `blockedUntil` still ahead, at most [`BLOCK_MAX_SECS`] away.
///
/// The age limits the load only: a block runs to its `blockedUntil`, so a
/// reading older than the limit still blocks (the peer probes every ten
/// minutes and the snapshot is held three).
struct Reading {
    /// The load and when, while fresh enough to use.
    load: Option<(DateTime<Utc>, f64)>,
    blocked: bool,
}

fn reading(theirs: &PeerAccount, now: DateTime<Utc>) -> Option<Reading> {
    let at = parse_time(theirs.measured_at.as_deref())?.min(now);
    let load = theirs
        .load_percent
        .filter(|load| load.is_finite() && (0.0..=100.0).contains(load))?;
    let blocked = theirs.blocked
        && parse_time(theirs.blocked_until.as_deref())
            .is_some_and(|until| until > now && (until - now).num_seconds() <= BLOCK_MAX_SECS);
    let fresh = (now - at).num_seconds() <= READING_MAX_SECS;
    Some(Reading {
        load: fresh.then_some((at, load)),
        blocked,
    })
}

// ── Held snapshots (the app) ─────────────────────────────────────────────

struct Held {
    fetched: DateTime<Utc>,
    usage: PeerUsage,
}

static HELD: Mutex<Option<HashMap<String, Held>>> = Mutex::new(None);
static LAST_KICK: Mutex<Option<DateTime<Utc>>> = Mutex::new(None);
static REFRESHING: AtomicBool = AtomicBool::new(false);

/// The paired Macs' snapshots fetched in the last [`FRESH_SECS`], starting a
/// background fetch when the last one is older than [`REFRESH_AFTER_SECS`].
/// Never waits.
pub(crate) fn cached(now: DateTime<Utc>) -> Vec<PeerUsage> {
    kick(now);
    HELD.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map(|held| fresh(held, now))
        .unwrap_or_default()
}

fn fresh(held: &HashMap<String, Held>, now: DateTime<Utc>) -> Vec<PeerUsage> {
    let mut out: Vec<(&String, &Held)> = held
        .iter()
        .filter(|(_, h)| (now - h.fetched).num_seconds() <= FRESH_SECS)
        .collect();
    out.sort_by(|a, b| a.0.cmp(b.0));
    out.into_iter().map(|(_, h)| h.usage.clone()).collect()
}

fn kick(now: DateTime<Utc>) {
    {
        let mut last = LAST_KICK.lock().unwrap_or_else(PoisonError::into_inner);
        if last.is_some_and(|at| (now - at).num_seconds() < REFRESH_AFTER_SECS) {
            return;
        }
        if REFRESHING.swap(true, Ordering::SeqCst) {
            return;
        }
        *last = Some(now);
    }
    tauri::async_runtime::spawn(async {
        // Cleared however the fetch ends: done, timed out or panicked.
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                REFRESHING.store(false, Ordering::SeqCst);
            }
        }
        let _done = Done;
        let _ = tokio::time::timeout(REFRESH_TIMEOUT, refresh()).await;
    });
}

/// Fetch every paired Mac's snapshot, keeping each as it arrives, so a
/// round cut off by its timeout keeps the answers it already had. When
/// the paired Macs can't be read, what is held stays.
async fn refresh() {
    let Ok(hub) = crate::commands::peer_api::Hub::for_app() else {
        return;
    };
    let with_held = |change: &dyn Fn(&mut HashMap<String, Held>)| {
        let mut held = HELD.lock().unwrap_or_else(PoisonError::into_inner);
        change(held.get_or_insert_with(HashMap::new));
    };
    hub.usage_snapshots(
        |ids| with_held(&|held| prune(held, ids)),
        |id, usage| with_held(&|held| keep(held, id.clone(), usage.clone(), Utc::now())),
    )
    .await;
}

/// Drop the snapshots of Macs no longer paired (absent from `paired`).
fn prune(held: &mut HashMap<String, Held>, paired: &[String]) {
    held.retain(|id, _| paired.contains(id));
}

/// One Mac's answer: a snapshot replaces what is held for it; none (it
/// failed or has none) keeps its last good one until [`FRESH_SECS`], as
/// [`fresh`] reads it.
fn keep(
    held: &mut HashMap<String, Held>,
    machine_id: String,
    usage: Option<PeerUsage>,
    now: DateTime<Utc>,
) {
    if let Some(usage) = usage {
        held.insert(
            machine_id,
            Held {
                fetched: now,
                usage,
            },
        );
    }
}

/// Whether a Mac's `GET /v1/machine` offers the snapshot.
pub(crate) fn lists_usage(capabilities: &[String]) -> bool {
    capabilities.iter().any(|c| c == "usage")
}

/// A fetched snapshot, kept only when it names the Mac it was asked of.
pub(crate) fn from_peer(machine_id: &str, usage: Option<PeerUsage>) -> Option<PeerUsage> {
    usage.filter(|u| u.machine_id == machine_id)
}

// ── Serving (`GET /v1/machine/usage`) ────────────────────────────────────

/// This Hollow's accounts as another Mac reads them: the Claude accounts
/// (`claude_accounts::usage_snapshot`) and the Codex sign-in. Empty in a
/// process that doesn't route accounts (tests, the MCP subcommands).
pub(crate) fn local_accounts(now: DateTime<Utc>) -> Vec<wire::AccountUsage> {
    if !crate::claude_accounts::routes_here() {
        return Vec::new();
    }
    let mut out = crate::claude_accounts::usage_snapshot(now);
    if let Some(codex) = codex_account(now) {
        out.push(codex);
    }
    out.truncate(wire::USAGE_ACCOUNTS_MAX);
    out
}

/// The Codex sign-in as one account (N17): Codex has one login per Mac
/// (`provider_accounts`), so on the mini it is the one account its Codex
/// runs use. Its load is the last plan reading a run reported.
fn codex_account(now: DateTime<Utc>) -> Option<wire::AccountUsage> {
    let identity = crate::provider_accounts::codex_identity()?;
    let limits = crate::provider_accounts::limits::latest();
    Some(codex_usage(identity, limits.as_ref(), now))
}

pub(crate) fn codex_usage(
    identity: crate::provider_accounts::CodexIdentity,
    limits: Option<&crate::provider_accounts::limits::PlanLimits>,
    now: DateTime<Utc>,
) -> wire::AccountUsage {
    let windows: Vec<(f64, Option<DateTime<Utc>>, &str)> = limits
        .map(|l| {
            l.windows
                .iter()
                .map(|w| {
                    (
                        w.used_percent,
                        parse_time(w.resets_at.as_deref()),
                        w.label.as_str(),
                    )
                })
                .filter(|(_, reset, _)| reset.is_none_or(|at| at > now))
                .collect()
        })
        .unwrap_or_default();
    let load = windows
        .iter()
        .map(|w| w.0)
        .fold(None, |high: Option<f64>, load| {
            Some(high.map_or(load, |h| h.max(load)))
        });
    let full: Vec<&(f64, Option<DateTime<Utc>>, &str)> =
        windows.iter().filter(|w| w.0 >= 100.0).collect();
    let blocked_until = full.iter().filter_map(|w| w.1).max();
    let label = match &identity.plan {
        Some(plan) => format!("Codex ({plan})"),
        None => "Codex".to_string(),
    };
    wire::AccountUsage {
        key: identity.key,
        provider: "codex",
        label: wire_text(&label),
        billing: identity.billing.as_deref().map(wire_text),
        routed: true,
        load_percent: load,
        blocked: !full.is_empty(),
        blocked_until: blocked_until.map(rfc3339),
        seven_day_resets_at: windows
            .iter()
            .find(|w| w.2 == "7d limit")
            .and_then(|w| w.1)
            .map(rfc3339),
        measured_at: limits.map(|l| l.observed_at.clone()),
        recent_picks: 0,
        last_picked_at: None,
    }
}

/// The Claude Code login's organisation, from `~/.claude.json`'s
/// `oauthAccount` (no tokens there): `(uuid, name)`.
pub(crate) fn claude_login_org() -> Option<(String, Option<String>)> {
    let home = dirs::home_dir()?;
    let path = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or(home)
        .join(".claude.json");
    let meta = std::fs::metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > 16 * 1024 * 1024 {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    login_org_of(&json)
}

pub(crate) fn login_org_of(json: &serde_json::Value) -> Option<(String, Option<String>)> {
    let account = json.get("oauthAccount")?;
    let uuid = account
        .get("organizationUuid")
        .and_then(|v| v.as_str())
        .and_then(plain_id)?;
    let name = account
        .get("organizationName")
        .and_then(|v| v.as_str())
        .map(wire_text)
        .filter(|n| !n.is_empty());
    Some((uuid, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    const NOW: i64 = 1_790_000_000;
    const RESET: i64 = NOW + 3 * 24 * 3600;

    fn local(id: &str, load: Option<f64>, key: Option<&str>, measured: i64) -> Local {
        Local {
            candidate: Candidate {
                id: id.into(),
                load,
                blocked: false,
                last_used: 0,
            },
            key: key.map(str::to_owned),
            measured_at: Some(at(measured)),
            seven_day_resets_at: Some(RESET),
        }
    }

    fn theirs(key: &str, load: f64, measured: i64) -> PeerAccount {
        PeerAccount {
            key: key.into(),
            provider: "claude".into(),
            label: "Work".into(),
            load_percent: Some(load),
            measured_at: Some(rfc3339(at(measured))),
            seven_day_resets_at: Some(rfc3339(at(RESET))),
            ..PeerAccount::default()
        }
    }

    fn picked(mut account: PeerAccount, when: i64) -> PeerAccount {
        account.recent_picks = 1;
        account.last_picked_at = Some(rfc3339(at(when)));
        account
    }

    fn peer(accounts: Vec<PeerAccount>) -> PeerUsage {
        PeerUsage {
            usage_api: USAGE_API,
            machine_id: "m-0000000000000001".into(),
            observed_at: None,
            accounts,
        }
    }

    fn two() -> Vec<Local> {
        vec![
            local("acct-a", Some(10.0), Some("ak-a"), NOW - 60),
            local("acct-b", Some(20.0), Some("ak-b"), NOW - 60),
        ]
    }

    #[test]
    fn shared_keys_are_stable_opaque_and_per_kind() {
        let key = shared_key("claude-org", "1f0e8d3c-0000-4000-8000-00000000000a");
        assert_eq!(
            key,
            shared_key("claude-org", " 1f0e8d3c-0000-4000-8000-00000000000a ")
        );
        assert!(key.starts_with("ak-") && key.len() == 19);
        assert!(key[3..].chars().all(|c| c.is_ascii_hexdigit()));
        assert!(!key.contains("1f0e8d3c"));
        assert_ne!(
            key,
            shared_key("codex-user", "1f0e8d3c-0000-4000-8000-00000000000a")
        );
        assert_eq!(plain_id(" ABCDEF12-3456 "), Some("abcdef12-3456".into()));
        assert_eq!(plain_id("sk-ant-oat01-x"), None);
        assert_eq!(plain_id("abc"), None);
        assert_eq!(USAGE_API, wire::USAGE_API);
        assert_eq!(ACCOUNTS_MAX, wire::USAGE_ACCOUNTS_MAX);
    }

    #[test]
    fn with_no_peer_the_candidates_are_exactly_the_local_ones() {
        let locals = two();
        let want: Vec<Candidate> = locals.iter().map(|l| l.candidate.clone()).collect();
        assert_eq!(merge(locals.clone(), &[], 90.0, at(NOW)), want);
        // A peer that knows none of these keys changes nothing either.
        let stranger = peer(vec![picked(theirs("ak-zzz", 99.0, NOW), NOW)]);
        assert_eq!(merge(locals, &[stranger], 90.0, at(NOW)), want);
    }

    #[test]
    fn the_fresher_measurement_wins_and_a_peer_block_counts() {
        let fresher = peer(vec![
            theirs("ak-a", 55.0, NOW - 10),
            theirs("ak-b", 1.0, NOW - 600),
        ]);
        let merged = merge(two(), &[fresher], 90.0, at(NOW));
        assert_eq!(merged[0].load, Some(55.0), "the peer measured A later");
        assert_eq!(merged[1].load, Some(20.0), "the peer's B reading is older");

        let mut out = theirs("ak-b", 100.0, NOW - 30);
        out.blocked = true;
        out.blocked_until = Some(rfc3339(at(NOW + 3600)));
        let merged = merge(two(), &[peer(vec![out.clone()])], 90.0, at(NOW));
        assert!(merged[1].blocked);
        assert_eq!(merged[1].load, Some(100.0));
        // A block that has ended doesn't count.
        out.blocked_until = Some(rfc3339(at(NOW - 1)));
        assert!(!merge(two(), &[peer(vec![out])], 90.0, at(NOW))[1].blocked);
    }

    #[test]
    fn hostile_peer_values_never_bend_the_choice() {
        let a_load = |account: PeerAccount, locals: Vec<Local>| {
            merge(locals, &[peer(vec![account])], 90.0, at(NOW))[0].clone()
        };
        // Loads out of 0..=100 or not numbers.
        for load in [1e308, f64::INFINITY, f64::NAN, -1.0, 100.5] {
            assert_eq!(
                a_load(theirs("ak-a", load, NOW - 10), two()).load,
                Some(10.0),
                "{load}"
            );
        }
        // A measurement from the future counts as now: it never beats a
        // reading made here just now.
        let mut fresh_here = two();
        fresh_here[0].measured_at = Some(at(NOW));
        assert_eq!(
            a_load(theirs("ak-a", 70.0, NOW + 3600), fresh_here).load,
            Some(10.0)
        );
        // Too old to use.
        assert_eq!(
            a_load(theirs("ak-a", 70.0, NOW - READING_MAX_SECS - 60), two()).load,
            Some(10.0)
        );
        // Blocked with no end, an end far off, or no load: not blocked.
        let mut endless = theirs("ak-a", 100.0, NOW - 10);
        endless.blocked = true;
        assert!(!a_load(endless.clone(), two()).blocked);
        endless.blocked_until = Some(rfc3339(at(NOW + 30 * 24 * 3600)));
        assert!(!a_load(endless.clone(), two()).blocked);
        endless.blocked_until = Some(rfc3339(at(NOW + 3600)));
        endless.load_percent = None;
        assert!(!a_load(endless.clone(), two()).blocked);
        endless.load_percent = Some(100.0);
        assert!(
            a_load(endless.clone(), two()).blocked,
            "a sound block counts"
        );
        // A stale reading's load is not used, but its block still runs to
        // its end.
        let mut stale = endless.clone();
        stale.measured_at = Some(rfc3339(at(NOW - READING_MAX_SECS - 600)));
        let merged = a_load(stale.clone(), two());
        assert!(merged.blocked, "a stale block with its end ahead counts");
        assert_eq!(merged.load, Some(10.0), "a stale load does not");
        stale.blocked_until = Some(rfc3339(at(NOW - 1)));
        assert!(!a_load(stale, two()).blocked, "an ended one doesn't");
        // A weekly reset missing on either side: another seat may share the
        // key, so neither load nor block crosses; a pick still ranks.
        for side in ["ours", "theirs"] {
            let mut locals = two();
            let mut seat = picked(endless.clone(), NOW - 5);
            if side == "ours" {
                locals[0].seven_day_resets_at = None;
            } else {
                seat.seven_day_resets_at = None;
            }
            let merged = merge(locals, &[peer(vec![seat])], 90.0, at(NOW));
            assert!(!merged[0].blocked, "{side}");
            let load = merged[0].load.unwrap();
            assert_eq!(load, 10.0, "{side}: peer activity must not invent usage");
        }
    }

    #[test]
    fn a_failed_fetch_keeps_the_last_snapshot_until_it_is_stale() {
        let mut held = HashMap::new();
        let m1 = "m-0000000000000001".to_string();
        let m2 = "m-0000000000000002".to_string();
        prune(&mut held, &[m1.clone(), m2.clone()]);
        keep(&mut held, m1.clone(), Some(peer(vec![])), at(NOW));
        keep(&mut held, m2.clone(), Some(peer(vec![])), at(NOW));
        assert_eq!(fresh(&held, at(NOW)).len(), 2);
        // m1 fails this round; m2 is no longer paired.
        prune(&mut held, std::slice::from_ref(&m1));
        keep(&mut held, m1.clone(), None, at(NOW + 60));
        assert_eq!(held.keys().collect::<Vec<_>>(), [&m1]);
        assert_eq!(fresh(&held, at(NOW + FRESH_SECS)).len(), 1, "still good");
        assert!(fresh(&held, at(NOW + FRESH_SECS + 1)).is_empty(), "stale");
        // Only a Mac that lists `usage` is asked, and only its own snapshot kept.
        assert!(lists_usage(&["chats".into(), "usage".into()]));
        assert!(!lists_usage(&["chats".into(), "events".into()]));
        assert!(from_peer("m-0000000000000001", Some(peer(vec![]))).is_some());
        assert!(from_peer("m-00000000000000ff", Some(peer(vec![]))).is_none());
        assert!(from_peer("m-0000000000000001", None).is_none());
    }

    #[test]
    fn a_label_that_is_an_email_never_crosses() {
        assert_eq!(wire_text("person@example.com"), "");
        assert_eq!(wire_text("  Work \n plan "), "Work plan");
    }

    #[test]
    fn a_peer_pick_breaks_ties_without_fabricating_load() {
        let ceiling = 90.0;
        let mac = peer(vec![
            picked(theirs("ak-a", 10.0, NOW - 60), NOW - 30),
            theirs("ak-b", 20.0, NOW - 60),
        ]);
        let merged = merge(two(), std::slice::from_ref(&mac), ceiling, at(NOW));
        let a = merged[0].load.unwrap();
        assert_eq!(a, 10.0);
        assert_eq!(
            crate::claude_accounts::pick(&merged, ceiling, None),
            Some("acct-a".into())
        );
        // Nothing else eligible: the picked account stays the choice.
        let mut locals = two();
        locals[1].candidate.blocked = true;
        let merged = merge(locals, std::slice::from_ref(&mac), ceiling, at(NOW));
        assert_eq!(merged[0].load, Some(10.0));
        assert_eq!(
            crate::claude_accounts::pick(&merged, ceiling, None),
            Some("acct-a".into())
        );
        // An old pick is no longer busy.
        let old = peer(vec![picked(
            theirs("ak-a", 10.0, NOW - 60),
            NOW - RECENT_PICK_SECS - 5,
        )]);
        assert_eq!(merge(two(), &[old], ceiling, at(NOW))[0].load, Some(10.0));
        // Two busy accounts keep their order between them.
        let both = peer(vec![
            picked(theirs("ak-a", 10.0, NOW - 60), NOW),
            picked(theirs("ak-b", 20.0, NOW - 60), NOW),
        ]);
        let mut three = two();
        three.push(local("acct-c", Some(40.0), Some("ak-c"), NOW - 60));
        let merged = merge(three, &[both], ceiling, at(NOW));
        let (a, b, c) = (
            merged[0].load.unwrap(),
            merged[1].load.unwrap(),
            merged[2].load.unwrap(),
        );
        assert!(a < b && b < c && c < ceiling, "{a} {b} {c}");
        let mut tied = two();
        tied[1].candidate.load = Some(10.0);
        let tied = merge(tied, &[mac], ceiling, at(NOW));
        assert_eq!(
            crate::claude_accounts::pick(&tied, ceiling, None),
            Some("acct-b".into())
        );
    }

    #[test]
    fn a_key_that_names_two_accounts_is_not_used() {
        // Two seats of one organisation here: the same key twice.
        let mut seats = two();
        seats[1].key = Some("ak-a".into());
        let mac = peer(vec![picked(theirs("ak-a", 80.0, NOW), NOW)]);
        let want: Vec<Candidate> = seats.iter().map(|l| l.candidate.clone()).collect();
        assert_eq!(merge(seats, &[mac], 90.0, at(NOW)), want);
        // Twice on the peer's side.
        let twice = peer(vec![theirs("ak-a", 80.0, NOW), theirs("ak-a", 1.0, NOW)]);
        assert_eq!(merge(two(), &[twice], 90.0, at(NOW))[0].load, Some(10.0));
        // One each side, but their weeks reset at different times: two seats.
        let mut other_seat = theirs("ak-a", 80.0, NOW);
        other_seat.seven_day_resets_at = Some(rfc3339(at(RESET - 2 * 24 * 3600)));
        assert_eq!(
            merge(two(), &[peer(vec![other_seat])], 90.0, at(NOW))[0].load,
            Some(10.0)
        );
        // A Codex entry never matches a Claude account.
        let mut codex = theirs("ak-a", 80.0, NOW);
        codex.provider = "codex".into();
        assert_eq!(
            merge(two(), &[peer(vec![codex])], 90.0, at(NOW))[0].load,
            Some(10.0)
        );
    }

    #[test]
    fn an_old_or_odd_snapshot_reads_without_failing() {
        let read: PeerUsage = serde_json::from_value(json!({
            "usageApi": 1, "machineId": "m-0000000000000001",
            "accounts": [{"key": "ak-a", "provider": "claude", "later": true}]
        }))
        .unwrap();
        assert_eq!(read.accounts[0].load_percent, None);
        assert_eq!(read.accounts[0].recent_picks, 0);
        let merged = merge(two(), &[read], 90.0, at(NOW));
        assert_eq!(merged[0].load, Some(10.0));
    }

    #[test]
    fn codex_is_one_labelled_account_billing_its_workspace() {
        let claims = json!({
            "sub": "auth0|x",
            "email": "person@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_user_id": "user-123",
                "chatgpt_account_id": "ws-456",
                "chatgpt_plan_type": "team",
                "organizations": [{"id": "org-1", "title": "Acme", "is_default": true}]
            }
        });
        let identity = crate::provider_accounts::codex_identity_of(&claims).unwrap();
        assert_eq!(identity.key, shared_key("codex-user", "user-123\nws-456"));
        let limits = crate::provider_accounts::limits::PlanLimits {
            observed_at: rfc3339(at(NOW - 30)),
            windows: vec![
                crate::provider_accounts::limits::PlanWindow {
                    id: "primary".into(),
                    label: "5h limit".into(),
                    used_percent: 30.0,
                    resets_at: Some(rfc3339(at(NOW + 3600))),
                },
                crate::provider_accounts::limits::PlanWindow {
                    id: "secondary".into(),
                    label: "7d limit".into(),
                    used_percent: 64.0,
                    resets_at: Some(rfc3339(at(RESET))),
                },
            ],
        };
        let one = codex_usage(identity, Some(&limits), at(NOW));
        assert_eq!(one.provider, "codex");
        assert_eq!(one.label, "Codex (ChatGPT Team)");
        assert_eq!(one.billing.as_deref(), Some("Acme"));
        assert_eq!(one.load_percent, Some(64.0));
        assert!(!one.blocked);
        assert_eq!(one.seven_day_resets_at, Some(rfc3339(at(RESET))));
        let text = serde_json::to_string(&one).unwrap();
        assert!(!text.contains("person@example.com") && !text.contains("user-123"));
        // No ChatGPT ids, no account.
        assert!(crate::provider_accounts::codex_identity_of(&json!({})).is_none());
    }

    #[test]
    fn the_login_org_comes_from_claude_json_without_tokens() {
        let file = json!({"oauthAccount": {
            "accountUuid": "a-1", "emailAddress": "x@y.z",
            "organizationUuid": "1F0E8D3C-0000-4000-8000-00000000000A",
            "organizationName": "  Acme  "
        }});
        assert_eq!(
            login_org_of(&file),
            Some((
                "1f0e8d3c-0000-4000-8000-00000000000a".into(),
                Some("Acme".into())
            ))
        );
        assert_eq!(login_org_of(&json!({"oauthAccount": {}})), None);
    }
}
