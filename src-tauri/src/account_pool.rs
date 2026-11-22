//! Never-lose-a-turn routing for unattended runs (master plan #16,
//! decision 6, 2026-10-08).
//!
//! An [`AccountPool`] is the accounts one agent kind can run on. Only Claude
//! has one today ([`ClaudePool`] over `claude_accounts`); Codex accounts can
//! join later behind the same trait. Chat keeps R1's own attempt loop
//! (`chat.rs`), which resumes the native session; this module decides the
//! same question for an automation. A night-queue item is a chat turn, so it
//! fails over inside its turn through R1 (the same `decide_failover`, with a
//! native resume rather than a restart), and its account is picked like long
//! work (`chat::ChatSendRequest::unattended`).
//!
//! **An automation restarts; it does not resume.** Automations run without a
//! persisted native session (`--no-session-persistence`), so there is
//! nothing to `--resume`. When a Claude automation's account runs out
//! mid-run, the run starts once more, right away, on another eligible
//! account of the same organisation: same lease, same History run, same
//! receipt (which says "Switched to B after N tool calls (A hit its
//! limit)"), the original prompt plus [`restart_note`]. Whatever the first
//! attempt wrote stays in place. Real checkpoint resume waits for the step
//! spine (F6); until then one switch per run (one resumable segment).
//!
//! **Never** on an outage (an overload, a 5xx, a network error: decided by
//! `chat_resume::decide_failover`, shared with R1), never onto or from
//! Claude Code's own login (the machine login is not an added account),
//! never to another organisation unless the person allowed it
//! (`claude_accounts::failover_target`), never after a person's stop or a
//! budget stop, and never under a spending backstop (an enforced spending
//! budget's `--max-budget-usd`), which could not be split honestly between
//! two processes.

use crate::chat_resume::{decide_failover, Exhaustion, Failover, FailoverFacts};
use crate::claude_quota::StreamQuota;
use crate::result_kind::ResultKind;

/// What [`crate::chat_resume::decide_failover`] needs as a "session" for a
/// run that restarts fresh: any non-empty value, since nothing is resumed.
const FRESH_SESSION: &str = "fresh-session";

/// Switches per automation run, at most (one resumable segment until F6).
pub const MAX_RUN_SWITCHES: u32 = 1;

/// The accounts one agent kind can run on.
pub trait AccountPool {
    async fn refresh(&self) {}
    fn reject(&self, _account: &str, _reset: Option<i64>) {}
    /// Failover is on and added accounts exist.
    fn enabled(&self) -> bool;
    /// Whether a run's own last rate-limit event puts its account out of room.
    fn stream_exhausted(&self, quota: &StreamQuota) -> bool;
    /// One fresh probe of `account`; `None` when it could not tell.
    fn probe_exhausted(&self, account: &str) -> impl std::future::Future<Output = Option<bool>>;
    /// The eligible account to move to, never one in `exclude` (the
    /// exhausted one last), inside its organisation unless allowed.
    fn next(&self, exclude: &[String]) -> Option<String>;
    fn label(&self, account: &str) -> String;
}

/// The GUI's Claude accounts.
pub struct ClaudePool;

impl AccountPool for ClaudePool {
    async fn refresh(&self) {
        crate::claude_accounts::refresh_before_pick().await;
    }
    fn reject(&self, account: &str, reset: Option<i64>) {
        crate::claude_accounts::reject_account(account, reset);
    }
    fn enabled(&self) -> bool {
        crate::claude_accounts::failover_enabled()
    }
    fn stream_exhausted(&self, quota: &StreamQuota) -> bool {
        crate::claude_accounts::stream_exhausted(quota)
    }
    async fn probe_exhausted(&self, account: &str) -> Option<bool> {
        crate::claude_accounts::probe_exhausted(account).await
    }
    fn next(&self, exclude: &[String]) -> Option<String> {
        crate::claude_accounts::failover_target(exclude)
    }
    fn label(&self, account: &str) -> String {
        crate::claude_accounts::label(account)
    }
}

/// How one automation attempt ended, as the decision reads it.
#[derive(Debug, Clone)]
pub struct AttemptFacts<'a> {
    /// The added account the attempt ran on (`None` or `login`: the
    /// machine's own login, which never fails over).
    pub account: Option<&'a str>,
    pub kind: ResultKind,
    pub outage: bool,
    pub last_quota: Option<&'a StreamQuota>,
    /// The person stopped the run, or a budget did.
    pub stopped: bool,
    /// An enforced spending budget gave the run a `--max-budget-usd`.
    pub spending_backstop: bool,
    /// Switches this run already made.
    pub switches: u32,
}

/// A decided switch: the account to restart on and both labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Switch {
    pub to: String,
    pub from_label: String,
    pub to_label: String,
}

/// Whether an automation attempt moves to another account. `None`: the run
/// ends as it always did (PR 9b's classifier and retry then apply).
pub async fn judge<P: AccountPool>(pool: &P, facts: &AttemptFacts<'_>) -> Option<Switch> {
    let account = facts
        .account
        .filter(|a| !a.trim().is_empty() && *a != crate::claude_accounts::LOGIN_ID)?;
    if facts.stopped || facts.spending_backstop {
        return None;
    }
    let stream_exhausted = facts.last_quota.is_some_and(|q| pool.stream_exhausted(q));
    if !facts.outage
        && (facts.kind == ResultKind::ProviderQuota
            || (facts.kind == ResultKind::Transient && stream_exhausted))
    {
        pool.reject(
            account,
            facts
                .last_quota
                .and_then(|q| crate::agent_out::stream_reset(q, chrono::Utc::now().timestamp())),
        );
        pool.refresh().await;
    }
    if facts.switches >= MAX_RUN_SWITCHES {
        return None;
    }
    let tried = vec![account.to_string()];
    let mut shared = FailoverFacts {
        enabled: pool.enabled(),
        account: Some(account),
        kind: facts.kind,
        outage: facts.outage,
        session: Some(FRESH_SESSION),
        stream: facts
            .last_quota
            .map(|quota| Exhaustion::from(Some(pool.stream_exhausted(quota))))
            .unwrap_or(Exhaustion::Unknown),
        probe: None,
        tried: &tried,
    };
    let mut decision = decide_failover(&shared, || pool.next(&tried));
    if decision == Failover::Probe {
        shared.probe = Some(Exhaustion::from(pool.probe_exhausted(account).await));
        if shared.probe == Some(Exhaustion::Exhausted) {
            pool.reject(account, None);
            pool.refresh().await;
        }
        decision = decide_failover(&shared, || pool.next(&tried));
    }
    match decision {
        Failover::Switch { to } => Some(Switch {
            from_label: pool.label(account),
            to_label: pool.label(&to),
            to,
        }),
        _ => None,
    }
}

/// What the restarted attempt is told after its prompt.
pub fn restart_note(from_label: &str, tool_calls: u32) -> String {
    format!(
        "<hollow-account-switch>An earlier attempt of this same run, on the Claude account \
         {from_label}, stopped after {tool_calls} tool call(s) when that account ran out of usage. \
         Anything it changed is still in place: check the current state first and continue from \
         it, without redoing finished work or repeating anything it already sent or \
         published.</hollow-account-switch>"
    )
}

/// The receipt's reason for the switch.
pub fn switch_reason(from_label: &str, tool_calls: u32) -> String {
    format!(
        "{from_label} hit its limit after {tool_calls} tool call(s); the run restarted in a fresh \
         session"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Fake {
        enabled: bool,
        stream: bool,
        probe: Option<bool>,
        next: Option<&'static str>,
        probes: Cell<u32>,
    }

    impl AccountPool for Fake {
        fn enabled(&self) -> bool {
            self.enabled
        }
        fn stream_exhausted(&self, _: &StreamQuota) -> bool {
            self.stream
        }
        async fn probe_exhausted(&self, _: &str) -> Option<bool> {
            self.probes.set(self.probes.get() + 1);
            self.probe
        }
        fn next(&self, exclude: &[String]) -> Option<String> {
            self.next
                .map(str::to_string)
                .filter(|n| !exclude.contains(n))
        }
        fn label(&self, account: &str) -> String {
            account.to_uppercase()
        }
    }

    fn quota() -> StreamQuota {
        StreamQuota {
            five_hour: None,
            seven_day: None,
            status: None,
            using_overage: None,
        }
    }

    fn pool() -> Fake {
        Fake {
            enabled: true,
            stream: true,
            probe: None,
            next: Some("b"),
            probes: Cell::new(0),
        }
    }

    #[tokio::test]
    async fn a_quota_rejection_is_remembered_and_refreshed_before_selection() {
        struct RefreshPool(Cell<bool>, Cell<bool>);
        impl AccountPool for RefreshPool {
            fn enabled(&self) -> bool {
                true
            }
            fn stream_exhausted(&self, _: &StreamQuota) -> bool {
                false
            }
            async fn probe_exhausted(&self, _: &str) -> Option<bool> {
                Some(false)
            }
            fn reject(&self, account: &str, _: Option<i64>) {
                assert_eq!(account, "a");
                self.0.set(true);
            }
            async fn refresh(&self) {
                assert!(self.0.get());
                self.1.set(true);
            }
            fn next(&self, _: &[String]) -> Option<String> {
                assert!(self.1.get(), "refresh must precede target selection");
                Some("fresh-best".into())
            }
            fn label(&self, account: &str) -> String {
                account.into()
            }
        }
        let pool = RefreshPool(Cell::new(false), Cell::new(false));
        let q = quota();
        assert_eq!(judge(&pool, &facts(&q)).await.unwrap().to, "fresh-best");
        let pool = RefreshPool(Cell::new(false), Cell::new(false));
        let mut terminal = facts(&q);
        terminal.switches = MAX_RUN_SWITCHES;
        assert!(judge(&pool, &terminal).await.is_none());
        assert!(
            pool.0.get(),
            "the last rejected account must stay excluded from later runs"
        );
    }

    fn facts<'a>(quota: &'a StreamQuota) -> AttemptFacts<'a> {
        AttemptFacts {
            account: Some("a"),
            kind: ResultKind::ProviderQuota,
            outage: false,
            last_quota: Some(quota),
            stopped: false,
            spending_backstop: false,
            switches: 0,
        }
    }

    #[tokio::test]
    async fn an_exhausted_account_moves_the_run_once() {
        let q = quota();
        let switch = judge(&pool(), &facts(&q)).await.unwrap();
        assert_eq!(
            switch,
            Switch {
                to: "b".into(),
                from_label: "A".into(),
                to_label: "B".into()
            }
        );
        let again = AttemptFacts {
            switches: 1,
            ..facts(&q)
        };
        assert!(judge(&pool(), &again).await.is_none(), "one switch per run");
    }

    #[tokio::test]
    async fn outages_stops_the_login_and_backstops_never_move() {
        let q = quota();
        let cases = [
            AttemptFacts {
                outage: true,
                ..facts(&q)
            },
            AttemptFacts {
                account: Some("login"),
                ..facts(&q)
            },
            AttemptFacts {
                account: None,
                ..facts(&q)
            },
            AttemptFacts {
                stopped: true,
                ..facts(&q)
            },
            AttemptFacts {
                spending_backstop: true,
                ..facts(&q)
            },
            AttemptFacts {
                kind: ResultKind::Refusal,
                ..facts(&q)
            },
        ];
        for case in cases {
            assert!(judge(&pool(), &case).await.is_none(), "{case:?}");
        }
        let off = Fake {
            enabled: false,
            ..pool()
        };
        assert!(judge(&off, &facts(&q)).await.is_none(), "setting off");
        let none = Fake {
            next: None,
            ..pool()
        };
        assert!(
            judge(&none, &facts(&q)).await.is_none(),
            "no eligible account"
        );
    }

    #[tokio::test]
    async fn without_its_own_event_one_probe_decides() {
        let q = quota();
        let unknown = AttemptFacts {
            kind: ResultKind::Transient,
            last_quota: None,
            ..facts(&q)
        };
        let probed = Fake {
            probe: Some(true),
            ..pool()
        };
        assert!(judge(&probed, &unknown).await.is_some());
        assert_eq!(probed.probes.get(), 1);
        let unsure = Fake {
            probe: None,
            ..pool()
        };
        assert!(
            judge(&unsure, &unknown).await.is_none(),
            "unknown never moves"
        );
    }

    #[tokio::test]
    async fn a_direct_quota_rejection_does_not_need_a_probe() {
        let q = quota();
        let rejected = AttemptFacts {
            last_quota: None,
            ..facts(&q)
        };
        let stale = Fake {
            stream: false,
            probe: Some(false),
            ..pool()
        };
        assert!(judge(&stale, &rejected).await.is_some());
        assert_eq!(stale.probes.get(), 0);
    }

    #[test]
    fn the_restart_note_names_the_account_and_never_asks_to_redo() {
        let note = restart_note("Admin", 41);
        assert!(note.contains("Admin") && note.contains("41"));
        assert!(note.contains("without redoing finished work"));
        assert_eq!(
            switch_reason("Admin", 41),
            "Admin hit its limit after 41 tool call(s); the run restarted in a fresh session"
        );
    }
}
