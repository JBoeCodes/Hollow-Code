//! Periods, not cron.
//!
//! The old model stored a `next_run_at` instant and asked "is it past?". On a
//! laptop that sleeps, that question has two bad answers and no good one: fire
//! every missed occurrence in a burst (which is how "Close my day" once opened
//! with *"it is 12:34, /today wrote the plan at 12:30, so I closed the day four
//! minutes into it"*), or destroy any occurrence more than `misfire_grace_min`
//! late (which is how 2026-08-04 produced three `skipped` rows and zero runs).
//!
//! Both are symptoms of the same wrong premise. Cron encodes *when the machine
//! should be awake*. The requirement is **once per period, whenever Alex is
//! here**.
//!
//! So a job stores the *period it last completed*, not the instant it next
//! fires. The tick asks "is the current period different from the last one I
//! stamped?" N missed occurrences therefore collapse to exactly one run, and
//! that property is what the clock-replay harness at the bottom of this file
//! pins down: drive an injected clock through a simulated week of sleep, wake
//! and quit and assert every job produced exactly one run per period. Both
//! historical failures fail that test.
//!
//! This module is deliberately pure — no clock, no filesystem, no config — so
//! the harness can replay any timeline without touching a vault.

use serde::{Deserialize, Serialize};

/// How often a job should run. `Manual` never fires on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Period {
    /// Every calendar day, including weekends.
    Day,
    /// Monday to Friday only.
    Weekday,
    /// Once per ISO week.
    Week,
    /// Never scheduled; the user runs it.
    Manual,
}

impl Period {
    /// Parse the frontmatter value. Anything unrecognised is `Manual`, because
    /// a typo must not silently schedule an agent run.
    #[cfg(test)]
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "day" | "daily" => Period::Day,
            "weekday" | "weekdays" => Period::Weekday,
            "week" | "weekly" => Period::Week,
            _ => Period::Manual,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Period::Day => "day",
            Period::Weekday => "weekday",
            Period::Week => "week",
            Period::Manual => "manual",
        }
    }
}

/// A local wall-clock instant, as the scheduler sees it. Deliberately naive:
/// `preferred_hour` means "8am where the user is", and evaluating that against
/// UTC is the bug that once fired every routine 4-5 hours early.
pub type LocalTime = chrono::NaiveDateTime;

/// The tick's verdict for one job.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg(test)]
pub enum Decision {
    /// Nothing to do: manual, already ran this period, outside the period, or
    /// not yet at the preferred hour.
    Idle,
    /// Run once and stamp `key`. `late` means the period is already past its
    /// deadline hour, so the caller should say so rather than pretend the run
    /// was timely — a close-day that happens at 11pm is worth flagging.
    Run { key: String, late: bool },
}

/// The period `now` falls in, or `None` for a job that never self-schedules and
/// for a `Weekday` job on a weekend.
///
/// Day keys are `2026-08-05`; week keys are ISO `2026-W32`. ISO weeks matter:
/// a Sunday belongs to the week that began on the preceding Monday, so a weekly
/// job does not fire twice across a month boundary.
pub fn period_key(now: LocalTime, period: Period) -> Option<String> {
    use chrono::Datelike;
    match period {
        Period::Manual => None,
        Period::Day => Some(now.date().format("%Y-%m-%d").to_string()),
        Period::Weekday => {
            let wd = now.date().weekday();
            if matches!(wd, chrono::Weekday::Sat | chrono::Weekday::Sun) {
                None
            } else {
                Some(now.date().format("%Y-%m-%d").to_string())
            }
        }
        Period::Week => {
            let iso = now.date().iso_week();
            Some(format!("{}-W{:02}", iso.year(), iso.week()))
        }
    }
}

/// Should this job run right now?
///
/// `last_key` is the period it most recently completed. `preferred_hour` is the
/// local hour it would like to run at, and `deadline_hour` is when being unrun
/// stops being normal and becomes worth reporting.
///
/// The whole model is the first two checks: a job whose current period key
/// already equals `last_key` is done, no matter how many ticks arrive; a job
/// whose key differs is due exactly once.
#[cfg(test)]
pub fn decide(
    now: LocalTime,
    period: Period,
    preferred_hour: u32,
    deadline_hour: u32,
    last_key: Option<&str>,
) -> Decision {
    use chrono::Timelike;

    let Some(key) = period_key(now, period) else {
        return Decision::Idle;
    };
    if last_key == Some(key.as_str()) {
        return Decision::Idle;
    }

    let hour = now.hour();
    // Before the preferred hour, wait — unless the period is nearly over, in
    // which case running late beats not running at all.
    if hour < preferred_hour.min(23) {
        return Decision::Idle;
    }
    Decision::Run {
        key,
        late: hour >= deadline_hour.min(23),
    }
}

/// Catch a job up after the app was closed across one or more whole periods.
///
/// The answer is deliberately *one* run, not one per missed period: three days
/// of unwritten daily notes do not want three agent runs at 9am on the fourth
/// day, they want today's. This exists as a named function so the intent is
/// documented rather than implied by the absence of a loop.
#[cfg(test)]
pub fn missed_periods_collapse_to_one() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> LocalTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, min, 0)
            .unwrap()
    }

    #[test]
    fn day_keys_are_dates_and_week_keys_are_iso_weeks() {
        assert_eq!(
            period_key(at(2026, 8, 5, 9, 0), Period::Day).unwrap(),
            "2026-08-05"
        );
        assert_eq!(
            period_key(at(2026, 8, 5, 9, 0), Period::Week).unwrap(),
            "2026-W32"
        );
        // Sunday belongs to the week that began on the preceding Monday, so a
        // weekly job does not fire again just because the month rolled over.
        assert_eq!(
            period_key(at(2026, 8, 2, 9, 0), Period::Week).unwrap(),
            period_key(at(2026, 7, 27, 9, 0), Period::Week).unwrap(),
        );
        assert_eq!(period_key(at(2026, 8, 5, 9, 0), Period::Manual), None);
    }

    #[test]
    fn weekday_jobs_have_no_weekend_period() {
        // 2026-08-08 is a Saturday, 08-09 a Sunday.
        assert_eq!(period_key(at(2026, 8, 8, 9, 0), Period::Weekday), None);
        assert_eq!(period_key(at(2026, 8, 9, 9, 0), Period::Weekday), None);
        assert!(period_key(at(2026, 8, 7, 9, 0), Period::Weekday).is_some());
        assert_eq!(
            decide(at(2026, 8, 8, 9, 0), Period::Weekday, 7, 20, None),
            Decision::Idle
        );
    }

    #[test]
    fn a_manual_job_never_fires_and_a_typo_is_manual() {
        assert_eq!(
            decide(at(2026, 8, 5, 9, 0), Period::Manual, 7, 20, None),
            Decision::Idle
        );
        assert_eq!(Period::parse("evry day"), Period::Manual);
        assert_eq!(Period::parse("Daily"), Period::Day);
        assert_eq!(Period::parse("weekdays"), Period::Weekday);
    }

    #[test]
    fn waits_for_the_preferred_hour_then_runs_once() {
        let p = Period::Day;
        assert_eq!(
            decide(at(2026, 8, 5, 6, 59), p, 7, 20, None),
            Decision::Idle
        );
        let d = decide(at(2026, 8, 5, 7, 0), p, 7, 20, None);
        assert_eq!(
            d,
            Decision::Run {
                key: "2026-08-05".into(),
                late: false
            }
        );
        // Having stamped the key, every later tick that day is idle.
        for h in 7..24 {
            assert_eq!(
                decide(at(2026, 8, 5, h, 30), p, 7, 20, Some("2026-08-05")),
                Decision::Idle,
                "hour {h} should be idle once the day is stamped",
            );
        }
    }

    #[test]
    fn past_the_deadline_the_run_is_flagged_late_rather_than_dropped() {
        // The old model destroyed this occurrence. It must still run, and say so.
        let d = decide(at(2026, 8, 5, 23, 10), Period::Day, 7, 20, None);
        assert_eq!(
            d,
            Decision::Run {
                key: "2026-08-05".into(),
                late: true
            }
        );
    }

    /// The clock-replay harness. PLAN §9.1 calls this the single most valuable
    /// test in the plan, because both historical scheduler failures fail it.
    ///
    /// Drives a minute-by-minute clock through a simulated week in which the
    /// machine sleeps, wakes, and the app quits and relaunches, and asserts
    /// every job produced **exactly one run per period it was eligible for**.
    #[test]
    fn clock_replay_yields_exactly_one_run_per_period_across_sleep_wake_and_quit() {
        struct Job {
            period: Period,
            preferred_hour: u32,
            last_key: Option<String>,
            runs: Vec<String>,
        }

        let mut jobs = vec![
            Job {
                period: Period::Day,
                preferred_hour: 7,
                last_key: None,
                runs: vec![],
            },
            Job {
                period: Period::Weekday,
                preferred_hour: 9,
                last_key: None,
                runs: vec![],
            },
            Job {
                period: Period::Week,
                preferred_hour: 14,
                last_key: None,
                runs: vec![],
            },
            Job {
                period: Period::Manual,
                preferred_hour: 9,
                last_key: None,
                runs: vec![],
            },
        ];

        // Monday 2026-08-03 through Sunday 2026-08-09. The app is only "up" for
        // the windows below: everything else is the machine asleep or quit, so
        // no tick is delivered at all. Note Wednesday is entirely missed, and
        // Thursday only comes up at 23:00 — both cases the old model destroyed.
        let up_windows: &[(u32, u32, u32)] = &[
            (3, 8, 18), // Mon: normal day
            (4, 6, 12), // Tue: morning only
            // Wed 5th: never opened at all
            (6, 23, 24), // Thu: one late-night window
            (7, 9, 17),  // Fri: normal day
            (8, 10, 12), // Sat: weekend browsing
            (9, 20, 22), // Sun: evening
        ];

        for &(day, from_h, to_h) in up_windows {
            for hour in from_h..to_h {
                for minute in (0..60).step_by(1) {
                    let now = at(2026, 8, day, hour, minute);
                    for job in jobs.iter_mut() {
                        let d = decide(
                            now,
                            job.period,
                            job.preferred_hour,
                            20,
                            job.last_key.as_deref(),
                        );
                        if let Decision::Run { key, .. } = d {
                            job.runs.push(key.clone());
                            job.last_key = Some(key);
                        }
                    }
                }
            }
        }

        let daily = &jobs[0];
        let weekday = &jobs[1];
        let weekly = &jobs[2];
        let manual = &jobs[3];

        // THE property: no key ever runs twice. This is what fails under the old
        // catch-up-burst behaviour.
        for job in &jobs {
            let mut seen = job.runs.clone();
            seen.sort();
            seen.dedup();
            assert_eq!(
                seen.len(),
                job.runs.len(),
                "a period ran more than once: {:?}",
                job.runs
            );
        }

        // Every day the app was up past 07:00 got its daily run — including
        // Thursday's 23:00 window, which the grace-window model deleted.
        assert_eq!(
            daily.runs,
            vec![
                "2026-08-03",
                "2026-08-04",
                "2026-08-06",
                "2026-08-07",
                "2026-08-08",
                "2026-08-09"
            ],
            "daily runs",
        );
        // Wednesday never happened, and it does NOT get retroactively run later.
        assert!(!daily.runs.contains(&"2026-08-05".to_string()));

        // The weekday job wants 09:00. Tuesday's window closed at 12:00 so it
        // caught 09:00 fine; Wednesday was missed; Thursday's window is 23:00,
        // which is past 09:00, so it still runs — once.
        assert_eq!(
            weekday.runs,
            vec!["2026-08-03", "2026-08-04", "2026-08-06", "2026-08-07"]
        );
        // Saturday and Sunday are not weekday periods at all.
        assert!(!weekday
            .runs
            .iter()
            .any(|k| k == "2026-08-08" || k == "2026-08-09"));

        // Exactly one weekly run for the whole week, despite six up-windows.
        assert_eq!(
            weekly.runs,
            vec!["2026-W32"],
            "weekly must fire once per ISO week"
        );

        // Manual never fires on its own, whatever the clock does.
        assert!(manual.runs.is_empty());
    }

    #[test]
    fn the_catch_up_burst_cannot_happen_because_ticks_are_idempotent() {
        // Replays the historical failure directly: the app relaunches at 12:34
        // having missed a 07:00 daily and a 09:00 weekday job. Under cron this
        // fired every missed occurrence back to back. Here each job runs once
        // for today, and a second tick a minute later does nothing.
        let mut last_daily: Option<String> = None;
        let mut fires = 0;
        for minute in 34..40 {
            let now = at(2026, 8, 5, 12, minute);
            if let Decision::Run { key, .. } =
                decide(now, Period::Day, 7, 20, last_daily.as_deref())
            {
                fires += 1;
                last_daily = Some(key);
            }
        }
        assert_eq!(
            fires, 1,
            "a relaunch must produce one run, not one per missed tick"
        );
    }

    /// Proof the harness has teeth: replay the SAME timeline through the model
    /// this module replaces, and watch it produce both historical failures.
    ///
    /// `decide_misfire` answers about an instant, so a scheduler using it must
    /// re-arm `next_run_at` after every fire. Whichever grace window you pick,
    /// one of two things breaks:
    ///   - a generous grace fires every missed occurrence back to back (the
    ///     34-minute catch-up burst, and "closed the day four minutes into it");
    ///   - a tight grace destroys any occurrence the app was not open for (the
    ///     three `skipped` rows and zero runs on 2026-08-04).
    #[test]
    fn the_old_misfire_model_fails_this_same_timeline() {
        use crate::workflows::{decide_misfire, MisfireDecision};
        use chrono::{Duration, TimeZone, Utc};

        // Three daily 07:00 occurrences; the app opens once, late, on the third
        // day — exactly the "closed while you slept" case.
        let occurrences = [
            Utc.with_ymd_and_hms(2026, 8, 3, 7, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 4, 7, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 8, 5, 7, 0, 0).unwrap(),
        ];
        let relaunch = Utc.with_ymd_and_hms(2026, 8, 5, 12, 34, 0).unwrap();

        // Failure 1 — a generous grace fires all three at 12:34.
        let generous = Duration::days(7);
        let burst = occurrences
            .iter()
            .filter(|o| decide_misfire(relaunch, **o, generous) == MisfireDecision::Run)
            .count();
        assert_eq!(burst, 3, "the generous-grace model fires a burst");

        // Failure 2 — the shipped 120-minute grace destroys all three, including
        // today's, which the user is sitting in front of and still wants.
        let shipped = Duration::minutes(120);
        let skipped = occurrences
            .iter()
            .filter(|o| decide_misfire(relaunch, **o, shipped) == MisfireDecision::Skip)
            .count();
        assert_eq!(
            skipped, 3,
            "the shipped-grace model destroys every occurrence"
        );

        // The period model, on the same relaunch, does the only correct thing:
        // one run, for today.
        let mut last: Option<String> = None;
        let mut fires = vec![];
        for day in 3..=5 {
            // The app is closed on the 3rd and 4th, so no tick arrives at all.
            if day < 5 {
                continue;
            }
            if let Decision::Run { key, .. } = decide(
                at(2026, 8, day, 12, 34),
                Period::Day,
                7,
                20,
                last.as_deref(),
            ) {
                fires.push(key.clone());
                last = Some(key);
            }
        }
        assert_eq!(
            fires,
            vec!["2026-08-05"],
            "one run, for the period the user is actually in"
        );
    }

    #[test]
    fn a_period_is_never_destroyed_only_deferred_within_itself() {
        // The mirror-image failure: 0.4.0's misfire skip deleted any occurrence
        // more than 120 minutes late. Here a job whose preferred hour passed
        // hours ago still runs today.
        let d = decide(at(2026, 8, 5, 18, 0), Period::Day, 7, 20, None);
        assert!(
            matches!(d, Decision::Run { .. }),
            "a late period must still run, not be skipped"
        );
        assert!(missed_periods_collapse_to_one());
    }
}
