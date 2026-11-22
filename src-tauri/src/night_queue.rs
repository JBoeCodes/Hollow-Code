//! What is left of the night queue (master plan 2026-10, F1; the overnight
//! shift, #1; see `docs/SCHEDULER.md`).
//!
//! **The Tonight queue was removed 11-10 (S6).** Its screen, its Tauri
//! commands (`night_queue_*`), its agent actions, the drain that started
//! items inside the `tonight` window, the morning report and the runbook
//! driving code are gone: nothing adds an item or starts a night turn any
//! more. The `night_queue` table (and the runbook tables, see
//! `session_store::RUNBOOK_SCHEMA`) stay in the session database untouched, so
//! existing databases open the same way and history rows keep their meaning.
//!
//! What stays is what live code reads:
//!
//! - [`NightSpend`] / [`SPEND`], the mid-turn cost guard `chat` weighs at
//!   its safe points (inert unless a reading was published, which only
//!   tests do now), with [`mid_turn_note`] and [`stopped_mid_turn`];
//! - [`spent_tonight`], [`night_budget`], [`budget_read`]:
//!   what tonight's night rows cost, weighed against the overnight stop by
//!   `budgets` and `wake_me` (old night rows still count);
//! - [`left_on_runner`], what a conversation left on the Mac mini, read by
//!   `chat` and `session_store`;
//! - [`NIGHT_PREFIX`] and [`MORNING_REVIEW_OPEN`], the message openings the
//!   replay bench and `main_agent` recognise in old transcripts.

use crate::session_store::{with_read, with_write};
use chrono::{DateTime, Utc};
use rusqlite::params;
use std::path::Path;

/// The table (`session_store::open` runs it; inert since 11-10, S6: nothing writes it).
pub(crate) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS night_queue( \
   id INTEGER PRIMARY KEY AUTOINCREMENT, \
   workspace TEXT NOT NULL, \
   prompt TEXT NOT NULL, \
   runbook TEXT, \
   agent TEXT, \
   model TEXT, \
   budget_usd REAL, \
   kind TEXT NOT NULL, \
   added_by TEXT NOT NULL, \
   status TEXT NOT NULL, \
   added_at TEXT NOT NULL, \
   night TEXT, \
   queue_id INTEGER, \
   started_at TEXT, \
   finished_at TEXT, \
   note TEXT, \
   session_key TEXT, \
   run_workspace TEXT, \
   runbook_run INTEGER \
 ); \
 CREATE INDEX IF NOT EXISTS idx_night_queue_status ON night_queue(status, id);";

/// How a night turn's message started (the replay bench and tests still
/// recognise old transcripts by it).
#[cfg(test)]
pub(crate) const NIGHT_PREFIX: &str = "⟲ Hollow (night queue):";
/// What the morning review's Main agent turn says after `WAKE_PREFIX` (an
/// isolated, unattended turn; S07's replay bench tells it by this).
#[cfg(test)]
pub(crate) const MORNING_REVIEW_OPEN: &str = "the morning review of last night's queue";
/// Columns added after F1 shipped (`session_store::open` adds them).
pub(crate) const ADDED_COLUMNS: &[(&str, &str)] = &[
    ("session_key", "session_key TEXT"),
    ("run_workspace", "run_workspace TEXT"),
    ("runbook_run", "runbook_run INTEGER"),
];

fn sql(what: &'static str) -> impl Fn(rusqlite::Error) -> String {
    move |e| format!("night queue {what}: {e}")
}

// ── The mid-turn cost guard (turn guards, 2026-10-09) ──────────────────────

/// What the night guard weighs while night turns run. A night turn's real
/// cost is known only when it ends, so one long turn could overshoot the
/// overnight stop by its own cost (the Phase 3 lane A review finding). Each
/// running night turn now publishes a running estimate
/// (`usage_prices::estimate_usd` over its own messages; Claude only), and at
/// each of its safe points (`chat_resume::SafePoints`) asks
/// [`NightSpend::check`] whether the night's committed spend (what the drain
/// last read: finished night turns plus unattended runs) plus every live
/// night turn's estimate has reached the cap. If so the turn ends there, its
/// usage row keeps the estimate as a partial cost, its item becomes
/// `stopped` ([`stopped_mid_turn`]). At the turn's end the estimate is
/// replaced by the real cost ([`NightSpend::reconcile`]); once a reading
/// lists that turn's run as committed its entry goes. Since 11-10 (S6) no
/// drain publishes a reading, so only tests do and every check says no.
///
/// Entries are per turn (its run id), never per conversation: a runbook's
/// step turns share one conversation, each with its own entry.
///
/// Process-wide, like the run queue's pump: night turns run in this process.
/// Inactive (every check says no) until the drain has read the night once,
/// and for a conversation the drain doesn't list as a running night item.
#[derive(Debug)]
pub(crate) struct NightSpend {
    state: std::sync::Mutex<SpendState>,
}

#[derive(Debug)]
struct SpendState {
    /// The drain's last reading: committed spend, the cap, and the
    /// conversations of the night items still running.
    snapshot: Option<(f64, f64, std::collections::BTreeSet<String>)>,
    /// Per run id.
    turns: std::collections::BTreeMap<String, LiveTurn>,
}

/// One night turn's cost so far.
#[derive(Debug, Default, Clone, PartialEq)]
pub(crate) struct LiveTurn {
    /// The turn's conversation.
    pub session: String,
    /// Earlier attempts of the turn (an R1 switch at a limit, which starts a
    /// new usage row): their real cost, or their estimate when none was
    /// reported. A ceiling move keeps one row, so nothing moves here.
    pub prior: f64,
    /// The running attempt's estimate, or the real cost once reconciled.
    pub live: f64,
    /// The turn has ended and `live` is final.
    pub done: bool,
}

impl LiveTurn {
    pub(crate) fn total(&self) -> f64 {
        self.prior + self.live
    }
}

impl NightSpend {
    pub(crate) const fn new() -> Self {
        NightSpend {
            state: std::sync::Mutex::new(SpendState {
                snapshot: None,
                turns: std::collections::BTreeMap::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, SpendState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// A reading (the removed drain's, each tick with a night to weigh).
    /// `counted` are the runs whose cost `committed` already holds; their
    /// entries go. Tests only now.
    #[cfg(test)]
    pub(crate) fn snapshot(
        &self,
        committed: f64,
        cap: f64,
        running: std::collections::BTreeSet<String>,
        counted: &std::collections::BTreeSet<String>,
    ) {
        let mut state = self.lock();
        state.turns.retain(|run, _| !counted.contains(run));
        state.snapshot = Some((committed, cap, running));
    }

    /// No night to weigh: every check says no.
    #[cfg(test)]
    pub(crate) fn clear(&self) {
        let mut state = self.lock();
        state.snapshot = None;
        state.turns.retain(|_, turn| !turn.done);
    }

    /// Publish run `run`'s (conversation `session`) running estimate and
    /// weigh the night: `Some((total, cap))` once committed spend plus every
    /// live turn reaches the cap. `None` when the drain doesn't list
    /// `session` as a running night item.
    pub(crate) fn check(&self, session: &str, run: &str, live_usd: f64) -> Option<(f64, f64)> {
        let mut state = self.lock();
        let (committed, cap, watched) = {
            let (committed, cap, running) = state.snapshot.as_ref()?;
            (*committed, *cap, running.contains(session))
        };
        if !watched {
            return None;
        }
        let turn = state
            .turns
            .entry(run.to_string())
            .or_insert_with(|| LiveTurn {
                session: session.to_string(),
                ..LiveTurn::default()
            });
        if !turn.done {
            turn.live = live_usd.max(0.0);
        }
        let total = committed + state.turns.values().map(LiveTurn::total).sum::<f64>();
        (total >= cap).then_some((total, cap))
    }

    /// An earlier attempt of run `run` closed its own usage row (an R1
    /// switch): its cost (or its estimate, when none was reported) carries
    /// over.
    pub(crate) fn close_attempt(&self, run: &str, cost: Option<f64>) {
        if let Some(turn) = self.lock().turns.get_mut(run).filter(|t| !t.done) {
            turn.prior += cost.unwrap_or(turn.live).max(0.0);
            turn.live = 0.0;
        }
    }

    /// Run `run` ended: the estimate gives way to the real cost of its last
    /// row (kept when none was reported). Returns the turn as it was.
    pub(crate) fn reconcile(&self, run: &str, real: Option<f64>) -> Option<LiveTurn> {
        let mut state = self.lock();
        let turn = state.turns.get_mut(run).filter(|t| !t.done)?;
        let before = turn.clone();
        if let Some(real) = real.filter(|r| r.is_finite() && *r >= 0.0) {
            turn.live = real;
        }
        turn.done = true;
        Some(before)
    }

    /// What conversation `session`'s live turns have cost so far, not yet
    /// committed. Tests only now (the drain that held it back is gone).
    #[cfg(test)]
    pub(crate) fn so_far(&self, session: &str) -> f64 {
        self.lock()
            .turns
            .values()
            .filter(|t| t.session == session)
            .map(LiveTurn::total)
            .sum()
    }

    #[cfg(test)]
    pub(crate) fn turn(&self, run: &str) -> Option<LiveTurn> {
        self.lock().turns.get(run).cloned()
    }
}

/// The night guard's ledger (see [`NightSpend`]).
pub(crate) static SPEND: NightSpend = NightSpend::new();

/// The note on an item its own turn's guard stopped. An infinite total is
/// a fail-closed reading (an unreadable ledger, an unreported
/// cost).
pub(crate) fn mid_turn_note(total: f64, cap: f64) -> String {
    if total.is_finite() {
        format!(
            "Stopped at a safe point mid-turn: tonight reached its overnight stop (about ${total:.2} of ${cap:.2}, counting running turns' estimates); what it did so far is kept."
        )
    } else {
        "Stopped at a safe point mid-turn: a night turn's cost wasn't reported, so the night stops (fail closed); what it did so far is kept.".to_string()
    }
}

/// The night guard stopped `session_key`'s turn: the running item whose
/// conversation it is becomes `stopped`, so nothing reads the ended turn as
/// failed. False when no running item owns it.
pub(crate) fn stopped_mid_turn(
    vault: &Path,
    session_key: &str,
    note: &str,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    with_write(vault, "night queue", |tx| {
        tx.execute(
            "UPDATE night_queue SET status = 'stopped', note = ?2, finished_at = ?3 \
             WHERE status = 'running' AND session_key = ?1",
            params![session_key, note, crate::session_store::db_time(now)],
        )
        .map(|n| n > 0)
        .map_err(sql("status"))
    })
}

/// Tests elsewhere (chat's whole-turn night guard): one chat item claimed
/// for `night` and running in conversation `session_key`. Returns its id.
#[cfg(test)]
pub(crate) fn seed_running_chat_item(
    vault: &Path,
    session_key: &str,
    night: &str,
    now: DateTime<Utc>,
) -> i64 {
    seed_running_item(vault, session_key, night, Some(77), now)
}

/// A running night row for tests, with its run-queue row id when it has one.
#[cfg(test)]
pub(crate) fn seed_running_item(
    vault: &Path,
    session_key: &str,
    night: &str,
    queue_id: Option<i64>,
    now: DateTime<Utc>,
) -> i64 {
    let at = crate::session_store::db_time(now);
    with_write(vault, "night queue", |tx| {
        tx.execute(
            "INSERT INTO night_queue(workspace, prompt, kind, added_by, status, added_at, night, \
             queue_id, started_at, session_key) \
             VALUES ('w1', 'Summarise the week', 'non-code', 'person', 'running', ?1, ?2, ?3, ?1, ?4)",
            params![at, night, queue_id, session_key],
        )
        .map_err(sql("add"))?;
        Ok(tx.last_insert_rowid())
    })
    .unwrap()
}

/// A row's status and note, for tests.
#[cfg(test)]
pub(crate) fn status_and_note(vault: &Path, id: i64) -> (String, Option<String>) {
    with_read(vault, |conn| {
        conn.query_row(
            "SELECT status, note FROM night_queue WHERE id = ?1",
            params![id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(sql("read"))
    })
    .unwrap()
}

/// The run-queue rows night `night`'s items became: each turn's row, and
/// each step turn of an old runbook item (its tables stay, see the module doc).
fn night_rows(vault: &Path, night: &str) -> Result<Vec<i64>, String> {
    with_read(vault, |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT queue_id FROM night_queue WHERE night = ?1 AND queue_id IS NOT NULL \
                 UNION SELECT s.queue_id FROM runbook_steps s JOIN night_queue n \
                 ON n.runbook_run = s.run_id WHERE n.night = ?1 AND s.queue_id IS NOT NULL",
            )
            .map_err(sql("spend"))?;
        let ids = stmt
            .query_map(params![night], |r| r.get::<_, i64>(0))
            .map_err(sql("spend"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(sql("spend"));
        ids
    })
}

/// What tonight's finished night turns cost (with their helpers), and how
/// many of them have a cost nobody reported. A turn still queued or running
/// isn't counted (its cost is known only when it ends; its reserve held
/// its place when the drain existed). A read failure is an error (the night then
/// starts nothing), never $0.
pub(crate) fn spent_tonight(vault: &Path, night: &str) -> Result<(f64, u32), String> {
    let (mut spent, mut unknown) = (0.0, 0u32);
    for q in night_rows(vault, night)? {
        let Some(row) = crate::run_queue::get(vault, q)? else {
            continue;
        };
        if row.status.is_live() {
            continue;
        }
        let Some(run) = row.history_run_id else {
            continue;
        };
        let rollup = crate::descendants::rollup(vault, &run);
        spent += rollup.known_usd;
        unknown += rollup.unknown;
    }
    Ok((spent, unknown))
}

/// Tonight's spend and cap, as `budgets` and `wake_me` weigh them (decision 7; review
/// fix 2026-10-09). `own` is the night turns' cost ([`spent_tonight`]);
/// `automations` is what unattended runs spent since the `tonight`
/// occurrence began (or the overnight window, if earlier), whatever the
/// clock says now, so a window running past 07:00 keeps counting it. The
/// cap is the lower of the setting and the $40 hard stop, always. Fails
/// closed: a read error, or a night turn whose cost nobody reported (a
/// provider that doesn't say), stops the night at the cap.
pub(crate) fn night_budget(
    own: Result<(f64, u32), String>,
    automations: Result<(f64, u32), String>,
    setting_cap: f64,
) -> (f64, f64) {
    let cap = setting_cap.min(crate::budgets::OVERNIGHT_LIMIT_USD);
    match (own, automations) {
        (Ok((_, unknown)), _) if unknown > 0 => (f64::INFINITY, cap),
        (Ok((own, _)), Ok((auto, _))) => (own + auto, cap),
        _ => (f64::INFINITY, cap),
    }
}

/// [`night_budget`] read from the vault's records: the occurrence `night`
/// that began at `start`, at `now`. A read failure is an error rather than
/// the stop (a due wake-up then waits a minute instead of being dropped;
/// morning M1).
pub(crate) fn budget_read(
    vault: &Path,
    night: &str,
    start: DateTime<Utc>,
    now: DateTime<Utc>,
    setting_cap: f64,
) -> Result<(f64, f64), String> {
    let from = crate::budgets::overnight_window(now.with_timezone(&chrono::Local))
        .map(|(s, _)| s.with_timezone(&Utc).min(start))
        .unwrap_or(start);
    let own = spent_tonight(vault, night)?;
    let automations =
        crate::budgets::unattended_spend(vault, from, now + chrono::Duration::seconds(1))?;
    Ok(night_budget(Ok(own), Ok(automations), setting_cap))
}

/// What a night item's conversation left on the Mac mini, from its
/// `remoteRun` notices (oldest first): after the last turn that started
/// there, a return that failed, or none at all (Hollow quit before the turn
/// finished), means its work is still there, and the next turn in that
/// conversation there brings it back. `None` when it all came back (applied,
/// unchanged, or on a branch here) or nothing ran there. `self_hosted`:
/// the conversation is on a self-hosted model, whose turns run on this Mac
/// (self-hosted models Q7), so the sentence says to use a model that is not
/// self-hosted (Claude or Codex, Q9).
pub(crate) fn left_on_runner(events: &[String], self_hosted: bool) -> Option<String> {
    let parsed: Vec<serde_json::Value> = events
        .iter()
        .filter_map(|e| serde_json::from_str(e).ok())
        .collect();
    let phase = |e: &serde_json::Value| e.get("phase").and_then(|p| p.as_str()).map(str::to_string);
    let start = parsed
        .iter()
        .rposition(|e| phase(e).as_deref() == Some("start"))?;
    let host = parsed[start]
        .get("host")
        .and_then(|h| h.as_str())
        .filter(|h| !h.is_empty() && h.len() <= 80)
        .unwrap_or("its runner");
    let backs: Vec<&serde_json::Value> = parsed[start + 1..]
        .iter()
        .filter(|e| phase(e).as_deref() == Some("back"))
        .collect();
    // A worktree's failed return says where its work is (`on`): "here" when
    // it came back but couldn't be applied (it is on a branch here).
    let failed = backs.iter().any(|e| {
        e.get("outcome").and_then(|o| o.as_str()) == Some("failed")
            && e.get("on").and_then(|o| o.as_str()) != Some("here")
    });
    // Q7 review: only a self-hosted conversation is told about its model.
    let model = if self_hosted {
        "; a turn on a self-hosted model runs on this Mac, so use a model that is not self-hosted for it"
    } else {
        ""
    };
    (backs.is_empty() || failed).then(|| {
        format!(
            "Its work is still on the runner ({host}): the next turn there from this conversation brings it back (set the workspace's Runs on to Remote runner first if it runs here{model})."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    /// The fail-closed readings: an unreadable ledger or queue, or a night
    /// turn whose cost nobody reported, is an infinite spend; automations'
    /// unknowns are reported, not blocking; the cap is the lower of the
    /// setting and the $40 hard stop.
    #[test]
    fn night_budget_fails_closed() {
        let inf = (f64::INFINITY, 40.0);
        assert_eq!(
            night_budget(Ok((2.0, 0)), Err("locked".into()), 40.0),
            inf,
            "unreadable ledger"
        );
        assert_eq!(
            night_budget(Err("locked".into()), Ok((0.0, 0)), 40.0),
            inf,
            "unreadable queue"
        );
        assert_eq!(
            night_budget(Ok((2.0, 1)), Ok((0.0, 0)), 40.0),
            inf,
            "a night turn's cost unknown"
        );
        assert_eq!(
            night_budget(Ok((2.0, 0)), Ok((0.0, 3)), 40.0),
            (2.0, 40.0),
            "automations' unknowns are reported, not blocking"
        );
        assert_eq!(night_budget(Ok((2.0, 0)), Ok((1.0, 0)), 60.0), (3.0, 40.0));
    }

    /// Turn guards: the night guard weighs committed spend plus every live
    /// night turn's estimate; it ignores a conversation the drain doesn't
    /// list; an R1 attempt's real cost carries over; at the turn's end the
    /// estimate gives way to the real cost, and once the drain counts that
    /// run as committed its entry goes. Entries are per run: a runbook's step
    /// turns share a conversation and are each weighed.
    #[test]
    fn the_night_guard_weighs_estimates_and_reconciles_at_the_end() {
        let spend = NightSpend::new();
        let set = |keys: &[&str]| -> std::collections::BTreeSet<String> {
            keys.iter().map(|k| k.to_string()).collect()
        };
        let near = |got: Option<(f64, f64)>, total: f64| {
            got.is_some_and(|(t, cap)| (t - total).abs() < 1e-9 && cap == 40.0)
        };
        assert_eq!(spend.check("a", "run-a", 100.0), None, "no reading yet");
        spend.snapshot(30.0, 40.0, set(&["a", "b"]), &set(&[]));
        assert_eq!(
            spend.check("other", "run-o", 100.0),
            None,
            "not a night item"
        );
        assert_eq!(spend.check("a", "run-a", 4.0), None, "30 + 4");
        assert_eq!(spend.check("b", "run-b1", 5.0), None, "30 + 4 + 5");
        // An R1 switch: the closed attempt's real $3.50 carries over.
        spend.close_attempt("run-a", Some(3.5));
        assert_eq!(spend.turn("run-a").unwrap().prior, 3.5);
        assert_eq!(spend.check("a", "run-a", 1.0), None, "30 + 3.5 + 1 + 5");
        assert!(
            near(spend.check("a", "run-a", 1.6), 40.1),
            "30 + 3.5 + 1.6 + 5"
        );
        assert!(
            near(spend.check("b", "run-b1", 5.0), 40.1),
            "b sees a's estimate"
        );
        // a ends: its real cost replaces the estimate, and stays put.
        let was = spend.reconcile("run-a", Some(1.25)).unwrap();
        assert_eq!((was.prior, was.live, was.done), (3.5, 1.6, false));
        assert_eq!(spend.turn("run-a").unwrap().total(), 4.75);
        assert_eq!(spend.check("a", "run-a", 9.0), None, "final: 30 + 4.75 + 5");
        assert_eq!(spend.reconcile("run-a", Some(99.0)), None, "once");
        // b is a runbook: step 1 ends, step 2 is a new run in the same
        // conversation, weighed in full.
        spend.reconcile("run-b1", Some(5.0));
        assert!(
            near(spend.check("b", "run-b2", 0.5), 40.25),
            "30 + 4.75 + step 1's 5 + step 2's 0.5: step 2 is weighed"
        );
        assert_eq!(spend.so_far("b"), 5.5);
        // The drain counts run-a and run-b1 as committed and drops them.
        spend.snapshot(39.75, 40.0, set(&["b"]), &set(&["run-a", "run-b1"]));
        assert_eq!((spend.turn("run-a"), spend.turn("run-b1")), (None, None));
        assert_eq!(spend.check("b", "run-b2", 0.2), None, "39.75 + 0.2");
        assert!(
            near(spend.check("b", "run-b2", 0.25), 40.0),
            "at the stop is over"
        );
        // Fail closed: an infinite reading stops at once, and says why.
        spend.snapshot(f64::INFINITY, 40.0, set(&["b"]), &set(&[]));
        assert!(spend.check("b", "run-b2", 0.0).is_some());
        assert!(mid_turn_note(f64::INFINITY, 40.0).contains("fail closed"));
        // No night: every check says no.
        spend.clear();
        assert_eq!(spend.check("b", "run-b2", 100.0), None);
    }

    /// Turn guards: a turn the night guard stopped leaves its item `stopped`
    /// with the guard's note (never `failed`); a conversation no running
    /// item owns changes nothing.
    #[test]
    fn a_mid_turn_stop_marks_the_item_stopped_with_the_notice() {
        let dir = vault();
        let v = dir.path();
        let now = Utc::now();
        let a = seed_running_item(v, "s-night-a", "2026-10-09", Some(77), now);
        let note = mid_turn_note(40.6, 40.0);
        assert!(!stopped_mid_turn(v, "s-someone-else", &note, now).unwrap());
        assert!(stopped_mid_turn(v, "s-night-a", &note, now).unwrap());
        let (status, note_read) = status_and_note(v, a);
        assert_eq!(status, "stopped");
        assert!(
            note_read
                .as_deref()
                .unwrap()
                .contains("about $40.60 of $40.00"),
            "{note_read:?}"
        );
        assert!(
            !stopped_mid_turn(v, "s-night-a", &note, now).unwrap(),
            "once"
        );
    }

    /// Overnight 11-02: what a night item's turns left on the Mac mini, read
    /// from its `remoteRun` notices.
    #[test]
    fn work_left_on_the_mac_mini_is_named_from_its_notices() {
        use serde_json::json;
        let e = |v: serde_json::Value| v.to_string();
        let start = e(json!({ "type": "remoteRun", "phase": "start", "host": "runner-host" }));
        let back = |outcome: &str| {
            e(
                json!({ "type": "remoteRun", "phase": "back", "host": "runner-host", "outcome": outcome }),
            )
        };
        // Never ran there, or ran here instead.
        assert_eq!(left_on_runner(&[], false), None);
        let stayed = e(json!({ "type": "remoteRun", "phase": "stayed", "host": "runner-host" }));
        assert_eq!(left_on_runner(std::slice::from_ref(&stayed), false), None);
        // Came back: applied, unchanged, on a branch here.
        for outcome in ["applied", "unchanged", "keptAside"] {
            assert_eq!(
                left_on_runner(&[start.clone(), back(outcome)], false),
                None,
                "{outcome}"
            );
        }
        // Still there: a failed return, or none (Hollow quit mid-turn).
        let said = left_on_runner(&[start.clone(), back("failed")], false).unwrap();
        assert_eq!(
            said,
            "Its work is still on the runner (runner-host): the next turn there from this conversation brings it back (set the workspace's Runs on to Remote runner first if it runs here)."
        );
        // Self-hosted models Q7 review: a conversation on a self-hosted
        // model is told its turns run here; no other is.
        assert_eq!(
            left_on_runner(&[start.clone(), back("failed")], true).unwrap(),
            "Its work is still on the runner (runner-host): the next turn there from this conversation brings it back (set the workspace's Runs on to Remote runner first if it runs here; a turn on a self-hosted model runs on this Mac, so use a model that is not self-hosted for it)."
        );
        assert_eq!(
            left_on_runner(&[start.clone(), back("applied")], true),
            None
        );
        assert!(left_on_runner(std::slice::from_ref(&start), false).is_some());
        // A failed return whose work came back anyway (on a branch here,
        // review of #1085), built as chat.rs builds it.
        let mut here = crate::remote_session::back_notice(
            "runner-host",
            &Err(
                "The runner's work came back but couldn't be applied here; it is on w-mini-1."
                    .into(),
            ),
        );
        here["on"] = json!("here");
        assert_eq!(
            left_on_runner(&[start.clone(), here.to_string()], false),
            None
        );
        let mut there = crate::remote_session::back_notice("runner-host", &Err("no route".into()));
        there["on"] = json!("runner");
        assert!(left_on_runner(&[start.clone(), there.to_string()], false).is_some());
        let folder = crate::remote_session::folder_notice("runner-host", &Err("later".into()));
        assert!(left_on_runner(&[start.clone(), folder.to_string()], false).is_some());
        // A build turn: the worktree came back, its folder didn't.
        let folder_failed = e(
            json!({ "type": "remoteRun", "phase": "back", "scope": "folder", "host": "runner-host", "outcome": "failed" }),
        );
        assert!(left_on_runner(&[start.clone(), back("applied"), folder_failed], false).is_some());
        // Only the last start counts: an earlier failure a later turn
        // brought back is settled.
        assert_eq!(
            left_on_runner(
                &[
                    start.clone(),
                    back("failed"),
                    start.clone(),
                    back("applied")
                ],
                false
            ),
            None
        );
        // An earlier turn left work there and the latest ran here: still there.
        assert!(left_on_runner(&[start.clone(), stayed], false).is_some());
        // Junk and odd hosts are tolerated.
        let odd = e(json!({ "type": "remoteRun", "phase": "start", "host": "" }));
        assert!(left_on_runner(&["not json".into(), odd], false)
            .unwrap()
            .contains("(its runner)"));
    }

    /// A night row's run-queue row, claimed and running with `run`, plus a
    /// usage row for it that is still `running`.
    fn running_turn(v: &Path, session: &str, run: &str, now: DateTime<Utc>) -> i64 {
        let q = crate::run_queue::enqueue(
            v,
            crate::run_queue::NewRow::new(
                crate::run_queue::RunSource::Chat,
                Some(session),
                "night",
                "test",
            ),
            now,
        )
        .unwrap()
        .id();
        crate::run_queue::claim(v, q, now).unwrap();
        crate::run_queue::mark_running(v, q, run, now).unwrap();
        q
    }

    /// Review fix (high): a turn still running has no cost yet; it must not
    /// read as an unreported cost (which stopped every night item a minute
    /// after it started). Once it ends with no cost, it does.
    #[test]
    fn a_running_turn_is_not_read_as_an_unknown_cost() {
        let dir = vault();
        let v = dir.path();
        let now = Utc::now();
        let night = "2026-10-08";
        let q = running_turn(v, "s-1", "R-night", now);
        seed_running_item(v, "s-1", night, Some(q), now);
        let mut row = crate::session_store::tests::usage_row_fixture(
            "s-1",
            &crate::session_store::db_time(now),
            "running",
        );
        row.run_id = Some("R-night".into());
        crate::session_store::upsert_usage(v, &row).unwrap();
        assert_eq!(
            spent_tonight(v, night),
            Ok((0.0, 0)),
            "running: not counted"
        );
        crate::run_queue::finish(v, q, crate::run_queue::RowStatus::Done, None, now).unwrap();
        assert_eq!(
            spent_tonight(v, night).unwrap().1,
            1,
            "finished with no cost: unknown"
        );
    }

    /// Q8 review (medium): a self-hosted night turn is billed by GPU time.
    /// Its running estimate is 0 (`UsageRun::night_estimate`), so it never
    /// moves the night towards its stop, and its finished row (no price) is
    /// neither spend nor an unreported cost: nothing fails closed.
    #[test]
    fn a_self_hosted_night_turn_neither_spends_nor_fails_closed() {
        let dir = vault();
        let v = dir.path();
        let now = Utc::now();
        let night = "2026-10-08";
        let q = running_turn(v, "s-qwen", "R-qwen", now);
        seed_running_item(v, "s-qwen", night, Some(q), now);
        // The live guard, $1 short of the stop: the turn's 0 never reaches it.
        let spend = NightSpend::new();
        let watched = std::collections::BTreeSet::from(["s-qwen".to_string()]);
        spend.snapshot(39.0, 40.0, watched, &std::collections::BTreeSet::new());
        let dir_run = tempfile::tempdir().unwrap();
        let mut run = crate::usage_ledger::UsageRun::begin(
            crate::usage_ledger::Meter::for_vault(
                dir_run.path(),
                crate::usage_ledger::Surface::Chat,
                Some("s-qwen".into()),
                Some("R-qwen".into()),
            ),
            Some(crate::agent::AgentKind::Claude),
            None,
        );
        run.set_billing(crate::usage::Billing::SelfHosted);
        run.observe(&crate::agent::AgentEvent::MessageUsage {
            message_id: "m-1".into(),
            model: Some("qwen3.8-27b".into()),
            tokens: crate::usage::TokenCounts {
                input: 2_000_000,
                cached_input: 0,
                output: 500_000,
            },
            is_final: false,
            cache_write: 0,
            thread: None,
        });
        assert_eq!(spend.check("s-qwen", "R-qwen", run.night_estimate()), None);
        // It ends with no price: the estimate (0) stays.
        let mut row = crate::session_store::tests::usage_row_fixture(
            "s-qwen",
            &crate::session_store::db_time(now),
            "success",
        );
        row.run_id = Some("R-qwen".into());
        row.billing = crate::usage::Billing::SelfHosted.as_str().into();
        row.cost_status = "unpriced".into();
        row.input_tokens = Some(2_000_000);
        crate::session_store::upsert_usage(v, &row).unwrap();
        spend.reconcile("R-qwen", None);
        assert_eq!(spend.turn("R-qwen").unwrap().total(), 0.0);
        crate::run_queue::finish(v, q, crate::run_queue::RowStatus::Done, None, now).unwrap();
        assert_eq!(
            spent_tonight(v, night),
            Ok((0.0, 0)),
            "no spend and nothing unknown"
        );
        let (spent, cap) = night_budget(spent_tonight(v, night), Ok((0.0, 0)), 40.0);
        assert_eq!((spent, cap), (0.0, 40.0), "never the fail-closed reading");

        // Q8 second review: a Codex consultation of unknown cost beside the
        // self-hosted turn is still unknown, so the night fails closed.
        let mut codex = crate::session_store::tests::usage_row_fixture(
            "s-qwen-codex",
            &crate::session_store::db_time(now),
            "success",
        );
        codex.surface = "delegate".into();
        codex.agent = Some("codex".into());
        codex.run_id = Some("R-qwen".into());
        crate::session_store::upsert_usage(v, &codex).unwrap();
        assert_eq!(spent_tonight(v, night), Ok((0.0, 1)));
        let (spent, _) = night_budget(spent_tonight(v, night), Ok((0.0, 0)), 40.0);
        assert_eq!(spent, f64::INFINITY, "fails closed on the Codex cost");
    }
}
