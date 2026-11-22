//! Accounts and the reserve (COO rebuild L7), moved from the company code's
//! `cfo.rs` (decision 8 of 11-09; removed 11-10): unattended work is
//! budgeted in each account's own 5-hour and weekly usage percentages, the numbers the
//! providers show.
//!
//! Per window and per account, the person's reserve comes first; unattended
//! work (subagents, automations, night items, Hollow's own check-ins and
//! wakes) may start on an account only while it is under the pool ceiling
//! (`100 - reserve`) in both windows, or while the person has approved extra
//! usage. An account never measured, whose window has reset since it was
//! read, or whose reading has no reset time and is older than
//! [`STALE_MINUTES`], counts as having room (its first run measures it).
//! Agents with no readings at all have room. The person's own prompts are
//! never held here.
//!
//! The hold and the pick use ONE set of accounts: the ones the run may
//! actually start on ([`pool`]: for Claude, `claude_accounts`' routed account
//! and the accounts long work may move to, the same organisation unless
//! crossing is on, only while failover is on). An account in another
//! organisation with room never lifts a hold it couldn't take.
//!
//! Readings are the in-memory ones Hollow already keeps (Claude accounts'
//! stream and refresh readings, Codex's passive rate-limit events); nothing
//! here makes a provider request. Any agent whose readings Hollow keeps is
//! counted the same way.

use super::settings::CfoSettings;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

/// A reading with no reset time is trusted this long.
pub(crate) const STALE_MINUTES: i64 = 30;
/// Resets this close count as a tie, broken by headroom.
const NEAR_TIE_MINUTES: i64 = 30;
/// Every CFO hold's text starts with this, so the run pump can tell a held
/// row's cause from a permission hold or a budget hold.
pub(crate) const HOLD_MARK: &str = "CFO hold: ";

/// One window reading: used percent and reset time.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Window {
    pub used: f64,
    pub resets_at: Option<String>,
}

/// One account's readings, from whichever agent owns it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Reading {
    pub agent: String,
    pub account: String,
    pub label: String,
    pub enabled: bool,
    pub five_hour: Option<Window>,
    pub weekly: Option<Window>,
    /// When the windows were read; none when unknown.
    pub observed_at: Option<DateTime<Utc>>,
}

/// One account as Settings, Usage shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CfoAccount {
    pub agent: String,
    pub account: String,
    pub label: String,
    pub five_hour_percent: Option<f64>,
    pub weekly_percent: Option<f64>,
    pub five_hour_resets_at: Option<String>,
    pub weekly_resets_at: Option<String>,
    /// Unattended work may start on this account now.
    pub room: bool,
    /// When a full account has room again: the reset of the window past the
    /// line (RFC 3339), when known.
    pub room_at: Option<String>,
    pub note: String,
}

fn parse(t: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(t)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// The live readings Hollow holds now (no request is made): enabled Claude
/// accounts only, and Claude Code's login when no added account is enabled
/// (the router then runs there); Codex's login.
pub(crate) fn readings() -> Vec<Reading> {
    let mut out = Vec::new();
    if let Some(view) = crate::claude_accounts::accounts_view() {
        let window = |w: &Option<crate::claude_accounts::WindowView>| {
            w.as_ref().map(|w| Window {
                used: w.used_percent,
                resets_at: w.resets_at.clone(),
            })
        };
        for account in view.accounts.iter().filter(|a| a.enabled) {
            out.push(Reading {
                agent: "claude".into(),
                account: account.id.clone(),
                label: account.label.clone(),
                enabled: true,
                five_hour: account.usage.as_ref().and_then(|u| window(&u.five_hour)),
                weekly: account.usage.as_ref().and_then(|u| window(&u.seven_day)),
                observed_at: account.usage.as_ref().and_then(|u| parse(&u.observed_at)),
            });
        }
        if !view.accounts.iter().any(|a| a.enabled) {
            if let Some(login) = &view.login_usage {
                out.push(Reading {
                    agent: "claude".into(),
                    account: crate::claude_accounts::LOGIN_ID.into(),
                    label: "Claude Code login".into(),
                    enabled: true,
                    five_hour: window(&login.five_hour),
                    weekly: window(&login.seven_day),
                    observed_at: parse(&login.observed_at),
                });
            }
        }
    }
    let codex = crate::provider_accounts::limits::latest();
    let pick = |id: &str| {
        codex.as_ref().and_then(|l| {
            l.windows.iter().find(|w| w.id == id).map(|w| Window {
                used: w.used_percent,
                resets_at: w.resets_at.clone(),
            })
        })
    };
    if codex.is_some() || crate::agent::available().contains(&crate::agent::AgentKind::Codex) {
        out.push(Reading {
            agent: "codex".into(),
            account: "login".into(),
            label: "Codex login".into(),
            enabled: true,
            five_hour: pick("primary"),
            weekly: pick("secondary"),
            observed_at: codex.as_ref().and_then(|l| parse(&l.observed_at)),
        });
    }
    out
}

/// The accounts of `agent` a run may start on: its readings, narrowed to
/// `eligible` when the agent's router names them (Claude: the routed account
/// and the ones long work may move to; `None`: no router, every reading).
pub(crate) fn pool(readings: &[Reading], agent: &str, eligible: Option<&[String]>) -> Vec<Reading> {
    readings
        .iter()
        .filter(|r| r.agent == agent)
        .filter(|r| eligible.is_none_or(|ids| ids.contains(&r.account)))
        .cloned()
        .collect()
}

/// The accounts a run of `agent` may start on, from the live router.
pub(crate) fn live_pool(readings: &[Reading], agent: &str) -> Vec<Reading> {
    let eligible = (agent == "claude")
        .then(crate::claude_accounts::long_work_eligible)
        .flatten();
    pool(readings, agent, eligible.as_deref())
}

/// The window as read, or nothing once its reset time has passed, or once a
/// reading with no reset time is older than [`STALE_MINUTES`].
fn current(
    window: &Option<Window>,
    observed: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Option<Window> {
    window
        .clone()
        .filter(|w| match w.resets_at.as_deref().and_then(parse) {
            Some(reset) => reset > now,
            None => observed.is_none_or(|at| now - at <= Duration::minutes(STALE_MINUTES)),
        })
}

/// Evaluate every reading against the policy.
pub(crate) fn evaluate(
    settings: &CfoSettings,
    readings: &[Reading],
    now: DateTime<Utc>,
) -> Vec<CfoAccount> {
    let ceiling = settings.pool_ceiling();
    let extra = settings.extra_usage_open(now);
    readings
        .iter()
        .map(|r| {
            // A window whose reset time has passed is unmeasured until the
            // next reading, never still "past the reserve".
            let five_hour = current(&r.five_hour, r.observed_at, now);
            let weekly = current(&r.weekly, r.observed_at, now);
            let over = |w: &Option<Window>| w.as_ref().is_some_and(|w| w.used >= ceiling);
            let spent = |w: &Option<Window>| w.as_ref().is_some_and(|w| w.used >= 100.0);
            // The later reset of the windows past the line: room only then.
            let room_at = |past: &dyn Fn(&Option<Window>) -> bool| {
                [&five_hour, &weekly]
                    .into_iter()
                    .filter(|w| past(w))
                    .filter_map(|w| w.as_ref().and_then(|w| w.resets_at.clone()))
                    .filter_map(|t| parse(&t).map(|at| (at, t)))
                    .max_by_key(|(at, _)| *at)
                    .map(|(_, t)| t)
            };
            let (room, room_again, note) = if !r.enabled {
                (
                    false,
                    None,
                    "Turned off in Settings → Usage → In rotation".to_string(),
                )
            } else if spent(&five_hour) || spent(&weekly) {
                if extra {
                    (
                        true,
                        None,
                        "Out of included usage; extra usage is approved".to_string(),
                    )
                } else {
                    (
                        false,
                        room_at(&spent),
                        "Out of included usage; extra usage needs your approval".to_string(),
                    )
                }
            } else if over(&five_hour) || over(&weekly) {
                let which = if over(&five_hour) { "5-hour" } else { "weekly" };
                if extra {
                    // Approved extra usage lifts the reserve line too.
                    (
                        true,
                        None,
                        format!("Past the reserve in its {which} window; extra usage is approved"),
                    )
                } else {
                    (
                        false,
                        room_at(&over),
                        format!("Past the reserve ({ceiling:.0}% line) in its {which} window"),
                    )
                }
            } else if five_hour.is_none() && weekly.is_none() {
                (
                    true,
                    None,
                    "Not measured yet; its first run measures it".to_string(),
                )
            } else {
                (true, None, "Room for unattended work".to_string())
            };
            CfoAccount {
                agent: r.agent.clone(),
                account: r.account.clone(),
                label: r.label.clone(),
                five_hour_percent: five_hour.as_ref().map(|w| w.used),
                weekly_percent: weekly.as_ref().map(|w| w.used),
                five_hour_resets_at: five_hour.as_ref().and_then(|w| w.resets_at.clone()),
                weekly_resets_at: weekly.as_ref().and_then(|w| w.resets_at.clone()),
                room,
                room_at: room_again,
                note,
            }
        })
        .collect()
}

fn agent_label(agent: &str) -> String {
    crate::agent::AgentKind::from_opt(agent)
        .map(|k| k.label().to_string())
        .unwrap_or_else(|| agent.to_string())
}

/// The hold for unattended work on `agent`, given the readings of the
/// accounts it may start on ([`pool`]), or None when one of them has room.
/// An agent with no readings at all (no accounts tracked) has room: its
/// first run measures it.
pub(crate) fn hold_for(
    settings: &CfoSettings,
    readings: &[Reading],
    agent: &str,
    now: DateTime<Utc>,
) -> Option<String> {
    let mine = pool(readings, agent, None);
    if mine.is_empty() {
        return None;
    }
    let evaluated = evaluate(settings, &mine, now);
    if evaluated.iter().any(|a| a.room) {
        return None;
    }
    let soonest = evaluated
        .iter()
        .filter_map(|a| a.room_at.as_deref().and_then(parse))
        .min();
    Some(format!(
        "{HOLD_MARK}unattended work on {} waits: every account it may run on is past the line ({}). Your {}% reserve for your own prompts comes first (Settings → Usage).{}",
        agent_label(agent),
        evaluated
            .iter()
            .map(|a| format!("{}: {}", a.label, a.note))
            .collect::<Vec<_>>()
            .join("; "),
        settings.reserve_percent,
        soonest
            .map(|t| format!(" The next window resets at {}.", t.to_rfc3339()))
            .unwrap_or_default()
    ))
}

/// What the CFO says about the account a turn starts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccountChoice {
    /// No override: the person's prompt (the account spread decides, and it
    /// is never held), or an agent with no readings.
    Spread,
    /// Unattended work goes here: the account with room whose window resets
    /// soonest (its unused allowance would be lost first); resets within
    /// half an hour of each other go to the one with more headroom.
    Use { account: String, label: String },
    /// No account it may run on has room and no extra usage is approved.
    Hold(String),
}

/// The account for one turn of `agent`, from the readings of the accounts
/// it may start on ([`pool`]).
pub(crate) fn choose_account(
    settings: &CfoSettings,
    readings: &[Reading],
    agent: &str,
    attended: bool,
    now: DateTime<Utc>,
) -> AccountChoice {
    if attended {
        return AccountChoice::Spread;
    }
    let mine = pool(readings, agent, None);
    if mine.is_empty() {
        return AccountChoice::Spread;
    }
    if let Some(hold) = hold_for(settings, &mine, agent, now) {
        return AccountChoice::Hold(hold);
    }
    let evaluated = evaluate(settings, &mine, now);
    let reset = |a: &CfoAccount| {
        [
            a.five_hour_resets_at.as_deref(),
            a.weekly_resets_at.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter_map(parse)
        .filter(|t| *t > now)
        .min()
    };
    let used = |a: &CfoAccount| {
        a.five_hour_percent
            .unwrap_or(0.0)
            .max(a.weekly_percent.unwrap_or(0.0))
    };
    // The earliest reset among accounts with room anchors a 30-minute
    // window; inside it the one with the most headroom wins (a single
    // anchor keeps the choice independent of the accounts' order).
    let roomy: Vec<&CfoAccount> = evaluated.iter().filter(|a| a.room).collect();
    let earliest = roomy.iter().filter_map(|a| reset(a)).min();
    let best = roomy
        .iter()
        .filter(|a| match (earliest, reset(a)) {
            (Some(first), Some(t)) => (t - first).num_minutes() <= NEAR_TIE_MINUTES,
            (Some(_), None) => false,
            (None, _) => true,
        })
        .min_by(|a, b| used(a).total_cmp(&used(b)).then(a.account.cmp(&b.account)))
        .copied();
    match best {
        Some(a) => AccountChoice::Use {
            account: a.account.clone(),
            label: a.label.clone(),
        },
        None => AccountChoice::Spread,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn reading(
        agent: &str,
        account: &str,
        five: Option<f64>,
        week: Option<f64>,
        resets_in_minutes: i64,
    ) -> Reading {
        let now = Utc::now();
        let resets = (now + Duration::minutes(resets_in_minutes)).to_rfc3339();
        let w = |used: f64| Window {
            used,
            resets_at: Some(resets.clone()),
        };
        Reading {
            agent: agent.into(),
            account: account.into(),
            label: account.into(),
            enabled: true,
            five_hour: five.map(w),
            weekly: week.map(w),
            observed_at: Some(now),
        }
    }

    #[test]
    fn the_reserve_holds_unattended_work_and_never_the_persons_prompt() {
        let settings = CfoSettings::default();
        assert_eq!(settings.pool_ceiling(), 80.0);
        let now = Utc::now();
        let busy = [
            reading("claude", "a", Some(85.0), Some(10.0), 180),
            reading("claude", "b", Some(10.0), Some(81.0), 180),
        ];
        let hold = hold_for(&settings, &busy, "claude", now).unwrap();
        assert!(hold.starts_with(HOLD_MARK), "{hold}");
        assert!(hold.contains("20% reserve"), "{hold}");
        assert!(hold.contains("5-hour") && hold.contains("weekly"), "{hold}");
        assert!(matches!(
            choose_account(&settings, &busy, "claude", false, now),
            AccountChoice::Hold(_)
        ));
        assert_eq!(
            choose_account(&settings, &busy, "claude", true, now),
            AccountChoice::Spread,
            "the person's prompt is never held"
        );
        // Another agent's readings never hold this one; untracked has room.
        assert_eq!(hold_for(&settings, &busy, "codex", now), None);
        let codex = [reading("codex", "login", None, None, 180)];
        assert_eq!(hold_for(&settings, &codex, "codex", now), None);
        // Codex readings count the same way as Claude's.
        let codex_full = [reading("codex", "login", Some(90.0), Some(20.0), 180)];
        assert!(hold_for(&settings, &codex_full, "codex", now).is_some());
    }

    #[test]
    fn only_the_accounts_a_run_may_start_on_lift_a_hold() {
        let settings = CfoSettings::default();
        let now = Utc::now();
        let all = [
            reading("claude", "work", Some(95.0), None, 60),
            reading("claude", "personal", Some(5.0), None, 60),
        ];
        // Personal is another organisation: not in the pool, so no room.
        let mine = pool(&all, "claude", Some(&["work".to_string()]));
        assert!(hold_for(&settings, &mine, "claude", now).is_some());
        let crossing = pool(&all, "claude", Some(&["work".into(), "personal".into()]));
        assert_eq!(
            choose_account(&settings, &crossing, "claude", false, now),
            AccountChoice::Use {
                account: "personal".into(),
                label: "personal".into()
            }
        );
    }

    #[test]
    fn the_soonest_reset_account_with_room_is_chosen_and_near_ties_go_to_headroom() {
        let settings = CfoSettings::default();
        let now = Utc::now();
        let pool = [
            reading("claude", "late", Some(10.0), Some(10.0), 240),
            reading("claude", "soon", Some(60.0), Some(30.0), 60),
            reading("claude", "full", Some(95.0), Some(30.0), 5),
        ];
        assert_eq!(
            choose_account(&settings, &pool, "claude", false, now),
            AccountChoice::Use {
                account: "soon".into(),
                label: "soon".into()
            }
        );
        // Ten minutes apart is a tie: the one with more headroom wins.
        let tie = [
            reading("claude", "busier", Some(70.0), None, 60),
            reading("claude", "roomier", Some(20.0), None, 70),
        ];
        assert_eq!(
            choose_account(&settings, &tie, "claude", false, now),
            AccountChoice::Use {
                account: "roomier".into(),
                label: "roomier".into()
            }
        );
        // The window is anchored on the earliest reset, whatever the order:
        // 60, 85 and 110 minutes out pick within 60 to 90 only.
        for order in [[0, 1, 2], [2, 1, 0], [1, 2, 0]] {
            let chain = [
                reading("claude", "first", Some(70.0), None, 60),
                reading("claude", "second", Some(40.0), None, 85),
                reading("claude", "third", Some(10.0), None, 110),
            ];
            let pool: Vec<Reading> = order.iter().map(|i| chain[*i].clone()).collect();
            assert_eq!(
                choose_account(&settings, &pool, "claude", false, now),
                AccountChoice::Use {
                    account: "second".into(),
                    label: "second".into()
                }
            );
        }
        // No readings at all: no override, nothing held.
        assert_eq!(
            choose_account(&settings, &[], "grok", false, now),
            AccountChoice::Spread
        );
    }

    #[test]
    fn an_account_past_its_reset_is_unmeasured_not_full() {
        let settings = CfoSettings::default();
        let now = Utc::now();
        let mut stale = reading("codex", "login", Some(95.0), Some(20.0), 180);
        assert!(hold_for(&settings, std::slice::from_ref(&stale), "codex", now).is_some());
        stale.five_hour.as_mut().unwrap().resets_at =
            Some((now - Duration::minutes(5)).to_rfc3339());
        assert_eq!(hold_for(&settings, &[stale.clone()], "codex", now), None);
        let view = evaluate(&settings, &[stale], now);
        assert!(view[0].room);
        assert_eq!(view[0].five_hour_percent, None, "unmeasured, not 95%");
    }

    #[test]
    fn an_old_reading_with_no_reset_time_is_unmeasured() {
        let settings = CfoSettings::default();
        let now = Utc::now();
        let mut old = reading("codex", "login", Some(95.0), None, 0);
        old.five_hour.as_mut().unwrap().resets_at = None;
        old.observed_at = Some(now - Duration::minutes(10));
        assert!(hold_for(&settings, std::slice::from_ref(&old), "codex", now).is_some());
        old.observed_at = Some(now - Duration::minutes(STALE_MINUTES + 1));
        assert_eq!(hold_for(&settings, &[old], "codex", now), None);
    }

    #[test]
    fn extra_usage_lifts_the_hold_until_it_expires() {
        let now = Utc::now();
        let spent = [reading("claude", "a", Some(100.0), Some(60.0), 120)];
        let mut settings = CfoSettings {
            reserve_percent: 0,
            ..CfoSettings::default()
        };
        assert!(hold_for(&settings, &spent, "claude", now)
            .unwrap()
            .contains("needs your approval"));
        settings.extra_usage_until = Some((now + Duration::hours(2)).to_rfc3339());
        assert_eq!(hold_for(&settings, &spent, "claude", now), None);
        assert!(matches!(
            choose_account(&settings, &spent, "claude", false, now),
            AccountChoice::Use { .. }
        ));
        settings.extra_usage_until = Some((now - Duration::hours(1)).to_rfc3339());
        assert!(hold_for(&settings, &spent, "claude", now).is_some());
    }

    #[test]
    fn approved_extra_usage_lifts_the_reserve_line_too() {
        let now = Utc::now();
        let past_line = [reading("claude", "a", Some(85.0), None, 120)];
        let mut settings = CfoSettings::default();
        assert!(hold_for(&settings, &past_line, "claude", now).is_some());
        settings.extra_usage_until = Some((now + Duration::hours(4)).to_rfc3339());
        assert_eq!(hold_for(&settings, &past_line, "claude", now), None);
        let view = evaluate(&settings, &past_line, now);
        assert!(view[0].room && view[0].note.contains("extra usage is approved"));
    }

    #[test]
    fn a_full_account_says_when_it_has_room_again() {
        let settings = CfoSettings::default();
        let now = Utc::now();
        let full = [reading("claude", "a", Some(90.0), Some(10.0), 120)];
        let view = evaluate(&settings, &full, now);
        assert!(!view[0].room);
        assert!(view[0].room_at.is_some());
        assert!(
            view[0].note.starts_with("Past the reserve"),
            "{}",
            view[0].note
        );
    }
}
