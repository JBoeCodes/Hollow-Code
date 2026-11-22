//! When an agent runs out of usage (repo ideas 2026-10-27 #8, after
//! openbot's usage-limit gate: hold until the reset).
//!
//! A chat turn that ended because its agent ran out, with nowhere to move
//! (R1's stop when no other Claude account has room, or a usage-limit
//! result on a turn that can't fail over: Claude Code's own login, failover
//! off, or another agent), says when that agent has room again: its closing
//! error starts "Waiting for Claude until 16:40", the soonest reset Hollow
//! knows (an added account's, for R1's stop; else the attempt's own last
//! `rate_limit_event`), or "until its usage resets" when nothing says. The
//! time is `chat_resume::reset_label`'s, as R1's stop always showed it.
//!
//! **Continue on another agent.** Settings, Models, When an
//! agent runs out is Wait by default (`agent_settings::when_out`). Set to an
//! agent, a turn that ran out continues there ([`continuation`]), only after
//! a turn that changed nothing: it called no tool at all, in any attempt
//! (so never mid-edit), it was the person's own turn (never a check-in, a
//! wake, a night lane or a child task, which wait: the Main agent's daily
//! stop and the night's spend guard stay whole), and it didn't itself
//! continue one (one move per request). The turn's end leaves the plan here
//! ([`plan`]); the run pump sends it after `done` as the conversation's next
//! turn on that agent (`commands::continue_on`), unless the person already
//! queued a message of their own. That turn starts a fresh session on the
//! other agent, so the context compiler re-grounds it with the conversation
//! and its task-state record; the next message the person sends goes to the
//! agent they pick, as always.

use crate::agent::AgentKind;
use crate::claude_quota::{StreamQuota, StreamWindow};
use once_cell::sync::Lazy;
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// "Waiting for Claude until 16:40", or "… until its usage resets".
pub(crate) fn waiting(agent: &str, resets: Option<&str>) -> String {
    match resets {
        Some(at) => format!("Waiting for {agent} until {at}"),
        None => format!("Waiting for {agent} until its usage resets"),
    }
}

/// When the attempt's own account has room again, from its last
/// `rate_limit_event` (as `claude_accounts` reads an account's windows):
/// the latest reset of the live windows at or past their limit, or of every
/// live window when the event says `rejected` without a full one. `None`
/// when one of them has no reset time, or nothing holds the account back.
pub(crate) fn stream_reset(quota: &StreamQuota, now: i64) -> Option<i64> {
    let live: Vec<StreamWindow> = [quota.five_hour, quota.seven_day]
        .into_iter()
        .flatten()
        .filter(|w| w.resets_at.is_none_or(|at| at > now))
        .collect();
    let full: Vec<StreamWindow> = live
        .iter()
        .copied()
        .filter(|w| w.utilization >= 1.0)
        .collect();
    let holding = if !full.is_empty() {
        full
    } else if quota.status.as_deref() == Some("rejected") {
        live
    } else {
        return None;
    };
    holding
        .iter()
        .map(|w| w.resets_at)
        .collect::<Option<Vec<i64>>>()?
        .into_iter()
        .max()
}

/// The reset Claude Code's classic usage-limit text carries itself
/// ("Claude AI usage limit reached|1790374200", epoch seconds), when it is
/// still ahead of `now`.
pub(crate) fn said_reset(text: &str, now: i64) -> Option<i64> {
    if let Some((_, tail)) = text.split_once("limit reached|") {
        let digits: String = tail.chars().take_while(char::is_ascii_digit).collect();
        let at: i64 = (digits.len() == 10)
            .then(|| digits.parse().ok())
            .flatten()?;
        return (at > now).then_some(at);
    }
    // Claude Code prints its short reset time in the host's local timezone.
    // Never infer a monthly reset from the words "monthly spend limit".
    static CLOCK: Lazy<regex::Regex> = Lazy::new(|| {
        regex::Regex::new(r"(?i)\bresets?\s+(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b")
            .expect("reset clock regex")
    });
    let captures = CLOCK.captures(text)?;
    let hour: u32 = captures[1].parse().ok()?;
    let minute: u32 = captures
        .get(2)
        .map_or(Some(0), |m| m.as_str().parse().ok())?;
    if !(1..=12).contains(&hour) || minute > 59 {
        return None;
    }
    let hour = hour % 12
        + if captures[3].eq_ignore_ascii_case("pm") {
            12
        } else {
            0
        };
    use chrono::TimeZone;
    let local = chrono::DateTime::from_timestamp(now, 0)?.with_timezone(&chrono::Local);
    let mut day = local.date_naive();
    for _ in 0..2 {
        let clock = day.and_hms_opt(hour, minute, 0)?;
        if let Some(at) = chrono::Local
            .from_local_datetime(&clock)
            .earliest()
            .filter(|at| at.timestamp() > now)
        {
            return Some(at.timestamp());
        }
        day = day.succ_opt()?;
    }
    None
}

/// The closing error of a turn whose agent hit its usage limit and could
/// not move: the waiting line, and, when no reset is known, the provider's
/// own words (`said`, already labelled "usage limit reached: …"), which may
/// name one.
pub(crate) fn limit_message(agent: &str, resets: Option<&str>, said: Option<&str>) -> String {
    let said = said
        .map(|s| s.trim().trim_end_matches('.').trim())
        .filter(|s| !s.is_empty());
    match (resets, said) {
        (None, Some(said)) => format!("{} ({said}).", waiting(agent, None)),
        _ => format!("{}: its usage limit was reached.", waiting(agent, resets)),
    }
}

/// What a turn's end knows about it, for [`continuation`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutTurn {
    pub agent: AgentKind,
    /// It ended because its agent ran out with nowhere to move.
    pub ran_out: bool,
    /// Its tool calls, every attempt together; `None` when the agent
    /// reports none at all.
    pub tool_calls: Option<usize>,
    /// The person's own turn: depth 0, attended, not a Main agent wake.
    pub person_sent: bool,
    /// It continued one that ran out ([`was_continuation`]).
    pub continued: bool,
}

/// The agent a turn that ran out continues on: the person's choice (`None`
/// is Wait), when it is another agent this Mac has, and only after a turn
/// that changed nothing (see the module doc).
pub(crate) fn continuation(
    turn: &OutTurn,
    choice: Option<AgentKind>,
    available: &[AgentKind],
) -> Option<AgentKind> {
    let to = choice?;
    (turn.ran_out
        && turn.tool_calls == Some(0)
        && turn.person_sent
        && !turn.continued
        && to != turn.agent
        && available.contains(&to))
    .then_some(to)
}

/// A continuation a turn's end leaves for the run pump.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Continuation {
    pub session_key: String,
    pub cwd: String,
    pub permission_mode: Option<String>,
    pub canvases: Option<String>,
    pub from: AgentKind,
    pub to: AgentKind,
    /// When `from` has room again ([`waiting`]'s time), if known.
    pub resets: Option<String>,
    /// [`stops`] of the conversation when the turn started: a Stop since
    /// (on the turn, or as it ended) means no continuation.
    pub stops: u64,
}

impl Continuation {
    /// This turn's continuation on `to`: the turn's mode is inherited onto
    /// another agent, not picked for it, so it goes through
    /// [`crate::agent::inherited_mode`] (Grok never gets auto with its
    /// sandbox off, Antigravity never its unavailable auto; lane Q review
    /// of #1363, 2026-11-13).
    pub(crate) fn onto(self, to: AgentKind, resets: Option<String>) -> Continuation {
        let permission_mode = match self.permission_mode {
            Some(mode) if to != self.from => {
                Some(crate::agent::inherited_mode(Some(self.from), to, &mode).to_string())
            }
            same => same,
        };
        Continuation {
            to,
            resets,
            permission_mode,
            ..self
        }
    }
}

/// What the continuation sends, as the person's next message (their
/// setting sent it): why, and to start from the conversation and its task
/// state, which a fresh session on `to` is given in front of it.
pub(crate) fn continue_prompt(next: &Continuation) -> String {
    let when = next
        .resets
        .as_deref()
        .map(|at| format!(" (it has room again at {at})"))
        .unwrap_or_default();
    format!(
        "{from} ran out of usage before it could answer my last message{when}, and it changed nothing. Please continue that request here on {to}: start from the task state and the conversation above, then carry on.",
        from = next.from.label(),
        to = next.to.label(),
    )
}

/// Stops the person made, by conversation ([`note_stop`]).
static STOPS: Lazy<Mutex<HashMap<String, u64>>> = Lazy::new(Default::default);

/// The person stopped conversation `key`'s turn (the composer's Stop, Stop
/// and send, `sessions.stop`): its planned continuation is dropped, and one
/// planned or sent around it sees the count move (review of #1065: a Stop
/// that lands as the turn ends must still win).
pub(crate) fn note_stop(key: &str) {
    *locked(&STOPS).entry(key.to_string()).or_default() += 1;
    locked(&PLANNED).remove(key);
}

/// How many Stops conversation `key` has had (this process).
pub(crate) fn stops(key: &str) -> u64 {
    locked(&STOPS).get(key).copied().unwrap_or(0)
}

/// Plans by conversation, until the pump takes them.
static PLANNED: Lazy<Mutex<HashMap<String, Continuation>>> = Lazy::new(Default::default);
/// The client turn ids continuations were sent with, until they run.
static SENT: Lazy<Mutex<HashSet<String>>> = Lazy::new(Default::default);
/// At most this many sent ids are remembered (one that never ran, such as
/// a continuation withdrawn from the queue, would otherwise stay).
const SENT_MAX: usize = 64;

fn locked<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Leave `next` for the pump to send after this turn's `done`.
pub(crate) fn plan(next: Continuation) {
    locked(&PLANNED).insert(next.session_key.clone(), next);
}

/// The continuation planned for conversation `key`, once.
pub(crate) fn take(key: &str) -> Option<Continuation> {
    locked(&PLANNED).remove(key)
}

/// A continuation was sent with this client turn id.
pub(crate) fn mark_sent(client_turn_id: &str) {
    let mut sent = locked(&SENT);
    if sent.len() >= SENT_MAX {
        sent.clear();
    }
    sent.insert(client_turn_id.to_string());
}

/// Whether the turn with this client turn id is a continuation (asked
/// once, as it runs).
pub(crate) fn was_continuation(client_turn_id: &str) -> bool {
    locked(&SENT).remove(client_turn_id)
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_reset_clock_handles_monthly_spend_text_and_midnight() {
        use chrono::TimeZone;
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 18, 22, 47, 0)
            .single()
            .unwrap();
        let expected = chrono::Local
            .with_ymd_and_hms(2026, 11, 18, 23, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(
            super::said_reset(
                "You've hit your monthly spend limit; your 5-hour limit resets 11:30pm",
                now.timestamp()
            ),
            Some(expected)
        );
        let midnight = chrono::Local
            .with_ymd_and_hms(2026, 11, 19, 0, 30, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(
            super::said_reset("resets 12:30am", now.timestamp()),
            Some(midnight)
        );
        assert_eq!(
            super::said_reset("monthly spend limit", now.timestamp()),
            None
        );
        assert_eq!(super::said_reset("resets 13:99pm", now.timestamp()), None);
    }
    use super::*;

    #[test]
    fn a_turn_continues_only_after_changing_nothing() {
        let all = AgentKind::all();
        let out = OutTurn {
            agent: AgentKind::Claude,
            ran_out: true,
            tool_calls: Some(0),
            person_sent: true,
            continued: false,
        };
        let codex = Some(AgentKind::Codex);
        assert_eq!(continuation(&out, codex, &all), Some(AgentKind::Codex));
        assert_eq!(continuation(&out, None, &all), None, "Wait, the default");
        for (why, turn) in [
            (
                "it didn't run out",
                OutTurn {
                    ran_out: false,
                    ..out
                },
            ),
            (
                "it called a tool: maybe mid-edit",
                OutTurn {
                    tool_calls: Some(1),
                    ..out
                },
            ),
            (
                "its tool calls are unknown",
                OutTurn {
                    tool_calls: None,
                    ..out
                },
            ),
            (
                "a check-in, wake, lane or child",
                OutTurn {
                    person_sent: false,
                    ..out
                },
            ),
            (
                "it was itself a continuation",
                OutTurn {
                    continued: true,
                    ..out
                },
            ),
        ] {
            assert_eq!(continuation(&turn, codex, &all), None, "{why}");
        }
        assert_eq!(
            continuation(&out, Some(AgentKind::Claude), &all),
            None,
            "never onto the agent that ran out"
        );
        assert_eq!(
            continuation(&out, codex, &[AgentKind::Claude]),
            None,
            "an agent this Mac doesn't have"
        );
    }

    #[test]
    fn a_continuation_onto_grok_maps_an_inherited_auto() {
        let turn = |mode: &str| Continuation {
            session_key: "s-ao-mode".into(),
            cwd: "/v".into(),
            permission_mode: Some(mode.into()),
            canvases: None,
            from: AgentKind::Claude,
            to: AgentKind::Claude,
            resets: None,
            stops: 0,
        };
        let mode = |next: Continuation| next.permission_mode;
        let grok = AgentKind::Grok;
        let at = Some("16:40".to_string());
        let next = turn("auto").onto(grok, at.clone());
        assert_eq!((next.to, next.resets.clone()), (grok, at));
        assert_eq!(
            mode(next).as_deref(),
            Some("acceptEdits"),
            "never Grok's unsandboxed auto"
        );
        assert_eq!(
            mode(turn("auto").onto(AgentKind::Codex, None)).as_deref(),
            Some("auto")
        );
        assert_eq!(
            mode(turn("bypassPermissions").onto(grok, None)).as_deref(),
            Some("bypassPermissions")
        );
        let unset = Continuation {
            permission_mode: None,
            ..turn("auto")
        };
        assert_eq!(mode(unset.onto(grok, None)), None);
    }

    #[test]
    fn a_plan_is_taken_once_and_a_continuation_known_once() {
        let next = Continuation {
            session_key: "s-ao-1".into(),
            cwd: "/v".into(),
            permission_mode: Some("acceptEdits".into()),
            canvases: None,
            from: AgentKind::Claude,
            to: AgentKind::Codex,
            resets: Some("16:40".into()),
            stops: stops("s-ao-1"),
        };
        plan(next.clone());
        assert_eq!(take("s-ao-1"), Some(next.clone()));
        assert_eq!(take("s-ao-1"), None);
        // A Stop drops the plan and moves the count.
        plan(next.clone());
        note_stop("s-ao-1");
        assert_eq!(take("s-ao-1"), None);
        assert_eq!(stops("s-ao-1"), next.stops + 1);
        mark_sent("out-ao-1");
        assert!(was_continuation("out-ao-1"));
        assert!(!was_continuation("out-ao-1"));
        assert_eq!(
            continue_prompt(&next),
            "Claude ran out of usage before it could answer my last message (it has room again at 16:40), and it changed nothing. Please continue that request here on Codex: start from the task state and the conversation above, then carry on."
        );
    }

    fn window(utilization: f64, resets_at: Option<i64>) -> Option<StreamWindow> {
        Some(StreamWindow {
            utilization,
            resets_at,
        })
    }

    fn quota(
        five_hour: Option<StreamWindow>,
        seven_day: Option<StreamWindow>,
        status: &str,
    ) -> StreamQuota {
        StreamQuota {
            five_hour,
            seven_day,
            status: Some(status.into()),
            using_overage: None,
        }
    }

    #[test]
    fn the_reset_is_the_latest_of_the_windows_holding_the_account() {
        let now = 1_000;
        // The five-hour window is full: its reset.
        assert_eq!(
            stream_reset(
                &quota(
                    window(1.0, Some(5_000)),
                    window(0.4, Some(90_000)),
                    "rejected"
                ),
                now
            ),
            Some(5_000)
        );
        // Both full: the later.
        assert_eq!(
            stream_reset(
                &quota(
                    window(1.02, Some(5_000)),
                    window(1.0, Some(90_000)),
                    "rejected"
                ),
                now
            ),
            Some(90_000)
        );
        // Rejected with no full window: every live one.
        assert_eq!(
            stream_reset(
                &quota(
                    window(0.9, Some(5_000)),
                    window(0.5, Some(9_000)),
                    "rejected"
                ),
                now
            ),
            Some(9_000)
        );
        // A window that already reset holds nothing.
        assert_eq!(
            stream_reset(
                &quota(window(1.0, Some(500)), window(0.5, Some(9_000)), "allowed"),
                now
            ),
            None
        );
        // Not out, or a holding window without a time: unknown.
        assert_eq!(
            stream_reset(&quota(window(0.5, Some(5_000)), None, "allowed"), now),
            None
        );
        assert_eq!(
            stream_reset(
                &quota(window(1.0, None), window(1.0, Some(9_000)), "rejected"),
                now
            ),
            None
        );
    }

    #[test]
    fn the_classic_limit_text_carries_its_reset() {
        let raw = "usage limit reached: Claude AI usage limit reached|1790374200";
        assert_eq!(said_reset(raw, 1_790_000_000), Some(1_790_374_200));
        assert_eq!(said_reset(raw, 1_790_374_200), None, "already passed");
        assert_eq!(said_reset("limit reached|12345", 0), None);
        assert_eq!(said_reset("You've hit your limit", 0), None);
    }

    #[test]
    fn the_notice_says_when_or_keeps_the_providers_words() {
        assert_eq!(
            waiting("Claude", Some("16:40")),
            "Waiting for Claude until 16:40"
        );
        assert_eq!(
            limit_message("Claude", Some("Fri 01:20"), Some("usage limit reached: x")),
            "Waiting for Claude until Fri 01:20: its usage limit was reached."
        );
        assert_eq!(
            limit_message(
                "Codex",
                None,
                Some("usage limit reached: try again at 4:40 PM.")
            ),
            "Waiting for Codex until its usage resets (usage limit reached: try again at 4:40 PM)."
        );
        assert_eq!(
            limit_message("Claude", None, None),
            "Waiting for Claude until its usage resets: its usage limit was reached."
        );
    }
}
