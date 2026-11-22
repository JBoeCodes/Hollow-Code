//! The `budgets` unit tests (split out of `budgets.rs` by S05, 2026-10-16,
//! no behaviour change). Attached from `budgets.rs` as `#[path] mod tests`,
//! so `super::` is still `crate::budgets`.

use super::*;
use crate::usage_ledger::first_valid_local;
use chrono::{FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use tempfile::TempDir;

fn tmp() -> TempDir {
    // Never a dot-prefixed root (CLAUDE.md → Quirks to know about: walk_notes prunes it).
    tempfile::Builder::new()
        .prefix("hollow-budgets-")
        .tempdir()
        .unwrap()
}

fn at(offset_hours: i32, y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<FixedOffset> {
    FixedOffset::east_opt(offset_hours * 3600)
        .unwrap()
        .with_ymd_and_hms(y, m, d, h, min, 0)
        .single()
        .unwrap()
}

fn utc(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn budget(id: &str, scope: &str, metric: Metric, window: UsageWindow, limit: f64) -> BudgetDef {
    BudgetDef {
        id: id.into(),
        scope: Scope::parse(scope).unwrap(),
        metric,
        window,
        limit,
        warn_percent: DEFAULT_WARN_PERCENT,
        hard_stop: true,
        created_at: String::new(),
        updated_at: String::new(),
    }
}

fn file_with(budgets: Vec<BudgetDef>) -> BudgetsFile {
    BudgetsFile {
        budgets,
        ..BudgetsFile::default()
    }
}

fn tokens(n: u64) -> impl Fn(&BudgetDef, &WindowBounds) -> Result<UsageTotals, String> {
    move |_, _| {
        Ok(UsageTotals {
            tokens: n,
            runs: 1,
            ..UsageTotals::default()
        })
    }
}

fn never_read() -> impl Fn(&BudgetDef, &WindowBounds) -> Result<UsageTotals, String> {
    |_, _| panic!("the ledger must not be read")
}

fn ledger_down() -> impl Fn(&BudgetDef, &WindowBounds) -> Result<UsageTotals, String> {
    |_, _| Err("database is locked".to_string())
}

fn pause(window_key: &str, held: bool) -> BudgetPause {
    BudgetPause {
        window_key: window_key.into(),
        window_end: String::new(),
        episode: "2026-09-24T12:00:00.000Z".into(),
        paused_at: "2026-09-24T12:00:00.000Z".into(),
        observed: 1200.0,
        limit: 1000.0,
        held,
        review_item_id: Some("r1".into()),
    }
}

// --- windows (PR 8 plan tests 17-18) ---------------------------------

#[test]
fn window_bounds_follow_local_day_week_and_month() {
    let now = at(-4, 2026, 9, 25, 10, 0); // a Friday, UTC−4
    let day = window_bounds_in(UsageWindow::Day, &now);
    assert_eq!(day.key, "2026-09-25");
    assert_eq!(day.start, utc("2026-09-25T04:00:00Z"));
    assert_eq!(day.end, utc("2026-09-26T04:00:00Z"));

    let week = window_bounds_in(UsageWindow::Week, &now);
    assert_eq!(week.key, "2026-W39");
    assert_eq!(week.start, utc("2026-09-21T04:00:00Z"), "Monday");
    assert_eq!(week.end, utc("2026-09-28T04:00:00Z"));

    let month = window_bounds_in(UsageWindow::Month, &now);
    assert_eq!(month.key, "2026-09");
    assert_eq!(month.start, utc("2026-09-01T04:00:00Z"));
    assert_eq!(month.end, utc("2026-10-01T04:00:00Z"));

    // December rolls into the next year; an ISO week can belong to the
    // previous ISO year (2026 has 53 weeks).
    let new_year = at(0, 2027, 1, 1, 9, 0);
    let week = window_bounds_in(UsageWindow::Week, &new_year);
    assert_eq!(week.key, "2026-W53");
    assert_eq!(week.start, utc("2026-12-28T00:00:00Z"));
    let dec = window_bounds_in(UsageWindow::Month, &at(9, 2026, 12, 15, 9, 0));
    assert_eq!(dec.key, "2026-12");
    assert_eq!(dec.start, utc("2026-11-30T15:00:00Z"));
    assert_eq!(dec.end, utc("2026-12-31T15:00:00Z"));
}

#[test]
fn local_midnight_survives_dst_gaps_and_the_fallback_never_panics() {
    let midnight = NaiveDate::from_ymd_opt(2026, 9, 6)
        .unwrap()
        .and_time(NaiveTime::MIN);
    let plus = |minutes: i64| midnight + chrono::TimeDelta::minutes(minutes);
    let exists_from =
        |first: NaiveDateTime| move |n: NaiveDateTime| (n >= first).then(|| n.and_utc());

    // A resolvable midnight is used as-is.
    assert_eq!(
        first_valid_local(midnight, exists_from(midnight), 0),
        midnight.and_utc()
    );
    // A gap covering 00:00-00:59 (America/Santiago style) → 01:00.
    assert_eq!(
        first_valid_local(midnight, exists_from(plus(60)), 0),
        plus(60).and_utc()
    );
    // A 30-minute gap (Lord Howe style) → 00:30, found by 15-minute steps.
    assert_eq!(
        first_valid_local(midnight, exists_from(plus(30)), 0),
        plus(30).and_utc()
    );
    // The search reaches 24 hours later and no further.
    assert_eq!(
        first_valid_local(midnight, exists_from(plus(24 * 60)), 0),
        plus(24 * 60).and_utc()
    );
    // A resolver that never resolves falls back to midnight at the given
    // UTC offset (UTC−4 → 04:00Z), without panicking.
    assert_eq!(
        first_valid_local(midnight, exists_from(plus(24 * 60 + 15)), -4 * 3600),
        plus(4 * 60).and_utc()
    );
    assert_eq!(
        first_valid_local(midnight, |_| None, 9 * 3600),
        plus(-9 * 60).and_utc()
    );
    // The step arithmetic cannot overflow at the edge of the calendar.
    assert_eq!(
        first_valid_local(NaiveDateTime::MAX, |_| None, 0),
        NaiveDateTime::MAX.and_utc()
    );
}

// --- scopes and intents -----------------------------------------------

#[test]
fn scopes_round_trip_and_reject_bad_input() {
    for raw in [
        "global",
        "heartbeat",
        "agent:claude",
        "agent:codex",
        "workflow:brief",
    ] {
        assert_eq!(Scope::parse(raw).unwrap().key(), raw);
    }
    assert_eq!(
        Scope::parse("agent:agy").unwrap(),
        Scope::Agent(AgentKind::Antigravity)
    );
    for bad in [
        "",
        "agent:gpt",
        "workflow:",
        "workflow:../x",
        "workflow:.hidden",
        "workflow:a/b",
        "team",
    ] {
        assert!(Scope::parse(bad).is_err(), "{bad:?} should be refused");
    }
    let json = serde_json::to_string(&Scope::Workflow("brief".into())).unwrap();
    assert_eq!(json, "\"workflow:brief\"");
    assert!(serde_json::from_str::<Scope>("\"agent:nope\"").is_err());
}

#[test]
fn only_automations_and_their_delegates_are_enforced() {
    let claude = Some(AgentKind::Claude);
    assert!(RunIntent::workflow("brief", claude).enforced());
    assert!(RunIntent::heartbeat(claude).enforced());
    assert!(!RunIntent::chat(claude).enforced());
    assert!(!RunIntent::oneshot(claude).enforced());
    assert!(!RunIntent::ingest(claude).enforced());
    let under = |surface: Option<Surface>, scope: Option<&str>| {
        RunIntent::delegate(
            Some(AgentKind::Codex),
            ParentRun {
                surface,
                scope: scope.map(str::to_string),
            },
        )
    };
    assert!(under(Some(Surface::Workflow), Some("brief")).enforced());
    assert!(under(Some(Surface::Heartbeat), None).enforced());
    assert!(under(None, None).enforced(), "unknown parent fails closed");
    assert!(!under(Some(Surface::Chat), Some("s-1")).enforced());

    let brief = Scope::Workflow("brief".into());
    assert!(RunIntent::workflow("brief", claude).applies(&brief));
    assert!(!RunIntent::workflow("other", claude).applies(&brief));
    assert!(under(Some(Surface::Workflow), Some("brief")).applies(&brief));
    assert!(!under(Some(Surface::Chat), Some("brief")).applies(&brief));
    assert!(under(Some(Surface::Heartbeat), None).applies(&Scope::Heartbeat));
    assert!(RunIntent::chat(claude).applies(&Scope::Global));
    assert!(RunIntent::chat(claude).applies(&Scope::Agent(AgentKind::Claude)));
    assert!(!RunIntent::chat(claude).applies(&Scope::Agent(AgentKind::Codex)));
    assert!(!RunIntent::chat(None).applies(&Scope::Agent(AgentKind::Claude)));
}

// PR 8 plan test 19 (pure half; the SQL read is added with preflight).
#[test]
fn scope_filters_select_the_right_ledger_rows() {
    let row = |surface, scope, parent, agent, input, cached, output| LedgerRow {
        surface,
        scope,
        parent_surface: parent,
        agent,
        input_tokens: Some(input),
        cached_input_tokens: Some(cached),
        output_tokens: Some(output),
        cost_usd: None,
        cost_status: "reported",
        gpu_time: false,
    };
    let rows = [
        row(
            "chat",
            Some("s-20260925-aaaaaaaa"),
            None,
            Some("claude"),
            10,
            1000,
            1,
        ),
        row("workflow", Some("brief"), None, Some("codex"), 20, 1000, 2),
        row(
            "delegate",
            Some("brief"),
            Some("workflow"),
            Some("claude"),
            40,
            1000,
            4,
        ),
        row("heartbeat", None, None, Some("claude"), 80, 1000, 8),
        row(
            "oneshot",
            Some("task_assist"),
            None,
            Some("claude"),
            160,
            1000,
            16,
        ),
        // A delegate under chat whose session key happens to equal a slug.
        row(
            "delegate",
            Some("brief"),
            Some("chat"),
            Some("claude"),
            320,
            1000,
            32,
        ),
        row(
            "delegate",
            None,
            Some("heartbeat"),
            Some("codex"),
            640,
            1000,
            64,
        ),
    ];
    let total = |scope: &str| UsageTotals::from_rows(&rows, &Scope::parse(scope).unwrap());
    assert_eq!(
        total("global").tokens,
        1397,
        "cache reads are never counted"
    );
    assert_eq!(total("global").cached_input_tokens, 7000);
    assert_eq!(total("global").runs, 7);
    assert_eq!(total("agent:codex").tokens, 22 + 704);
    assert_eq!(total("workflow:brief").tokens, 22 + 44);
    assert_eq!(total("heartbeat").tokens, 88 + 704);
    assert_eq!(total("agent:antigravity").runs, 0);
}

#[test]
fn totals_count_reported_and_estimated_cost_and_flag_gaps() {
    let rows = [
        LedgerRow {
            surface: "workflow",
            cost_usd: Some(0.25),
            cost_status: "estimated",
            ..LedgerRow::default()
        },
        LedgerRow {
            surface: "workflow",
            cost_usd: Some(0.5),
            cost_status: "reported",
            ..LedgerRow::default()
        },
        LedgerRow {
            surface: "workflow",
            input_tokens: Some(100),
            cost_status: "unpriced",
            ..LedgerRow::default()
        },
        LedgerRow {
            surface: "workflow",
            input_tokens: Some(-5),
            cost_status: "partial",
            ..LedgerRow::default()
        },
        LedgerRow {
            surface: "workflow",
            cost_usd: Some(f64::NAN),
            cost_status: "unknown",
            ..LedgerRow::default()
        },
    ];
    let t = UsageTotals::from_rows(&rows, &Scope::Global);
    assert!((t.cost_usd - 0.75).abs() < 1e-9);
    assert!(t.estimated);
    assert_eq!(t.tokens, 100, "negative counters are ignored");
    assert_eq!((t.unpriced_runs, t.partial_runs, t.unknown_runs), (1, 1, 1));
    assert_eq!(
        enforceability(&Scope::Global, Metric::CostUsd, None, Some(&t)).0,
        Enforceability::Partial
    );

    assert_eq!(t.uncosted_partial_runs, 1, "the partial row had no cost");
    assert_eq!(
        (t.missing(Metric::Tokens), t.missing(Metric::CostUsd)),
        (1, 3)
    );

    // Only runs that reported nothing for the metric make a budget
    // partial. A killed run's partial row whose cost was captured is a
    // lower bound, not a gap.
    let killed = UsageTotals {
        tokens: 500,
        cost_usd: 1.0,
        partial_runs: 3,
        runs: 3,
        ..UsageTotals::default()
    };
    for metric in [Metric::Tokens, Metric::CostUsd] {
        assert_eq!(
            enforceability(&Scope::Global, metric, None, Some(&killed)).0,
            Enforceability::Enforced
        );
    }

    // A partial Claude row with tokens but no cost (killed, interrupted
    // or an unverified resume) is MISSING for a spending budget, never
    // $0 (D2), and alone it shows no spending figure at all.
    let uncosted = [LedgerRow {
        surface: "workflow",
        agent: Some("claude"),
        input_tokens: Some(4000),
        output_tokens: Some(200),
        cost_usd: None,
        cost_status: "partial",
        ..LedgerRow::default()
    }];
    let u = UsageTotals::from_rows(&uncosted, &Scope::Global);
    assert_eq!((u.partial_runs, u.uncosted_partial_runs), (1, 1));
    let (level, reason) = enforceability(&Scope::Global, Metric::CostUsd, None, Some(&u));
    assert_eq!(level, Enforceability::Partial);
    assert!(reason.unwrap().contains("lower bound"));
    assert_eq!(
        enforceability(&Scope::Global, Metric::Tokens, None, Some(&u)).0,
        Enforceability::Enforced
    );
    assert_eq!(u.observed(Metric::CostUsd), None, "a dash, never $0");
    assert_eq!(u.observed(Metric::Tokens), Some(4200.0));
    let nothing = UsageTotals {
        runs: 2,
        unknown_runs: 2,
        ..UsageTotals::default()
    };
    assert_eq!(nothing.observed(Metric::Tokens), None);
    assert_eq!(
        UsageTotals::default().observed(Metric::Tokens),
        Some(0.0),
        "no runs is a real 0"
    );
    let codex = UsageTotals {
        tokens: 500,
        unpriced_runs: 1,
        ..UsageTotals::default()
    };
    assert_eq!(
        enforceability(&Scope::Global, Metric::Tokens, None, Some(&codex)).0,
        Enforceability::Enforced
    );
    let (level, reason) = enforceability(&Scope::Global, Metric::CostUsd, None, Some(&codex));
    assert_eq!(level, Enforceability::Partial);
    assert_eq!(
        reason.as_deref(),
        Some("1 run in this window reported no spending figure, so the total shown is a lower bound.")
    );
}

/// Q8 reviews: a self-hosted model's runs ride the Claude harness (agent
/// `claude`) but are neither Claude's tokens nor its spend. A Claude budget
/// leaves them out (rows and live runs alike); a Global one counts their
/// tokens and no spend, never a lower bound for them, and the budget card
/// lists them as GPU time, not as runs with no usage figures.
#[test]
fn self_hosted_runs_count_under_global_only_and_never_as_missing_spend() {
    let row = |status, gpu_time, cost_usd| LedgerRow {
        surface: "chat",
        agent: Some("claude"),
        input_tokens: Some(1_000),
        cost_usd,
        cost_status: status,
        gpu_time,
        ..LedgerRow::default()
    };
    let claude = Scope::Agent(AgentKind::Claude);
    let rows = [
        row("reported", false, Some(0.5)),
        row("unpriced", true, None),
        row("partial", true, None),
        row("unknown", true, None),
    ];
    let c = UsageTotals::from_rows(&rows, &claude);
    assert_eq!(
        (c.runs, c.tokens, c.gpu_time_runs),
        (1, 1_000, 0),
        "Claude's own run only"
    );
    let g = UsageTotals::from_rows(&rows, &Scope::Global);
    assert_eq!(
        (
            g.runs,
            g.gpu_time_runs,
            g.unpriced_runs,
            g.uncosted_partial_runs
        ),
        (4, 3, 0, 0)
    );
    assert!((g.cost_usd - 0.5).abs() < 1e-9);
    assert_eq!(g.tokens, 4_000, "their tokens count under Global");
    assert_eq!(
        enforceability(&Scope::Global, Metric::CostUsd, None, Some(&g)),
        (Enforceability::Enforced, None),
        "not a lower bound for spending"
    );
    assert_eq!(
        (g.missing(Metric::CostUsd), g.missing(Metric::Tokens)),
        (0, 1)
    );
    assert_eq!(
        (
            g.shown_unknown(Metric::CostUsd),
            g.shown_unknown(Metric::Tokens)
        ),
        (0, 1),
        "the spending card lists it as GPU time instead"
    );
    // A live self-hosted run reserves its tokens for Global only.
    let live = InFlightRun {
        agent: Some("claude".into()),
        surface: "chat".into(),
        tokens: 5_000,
        gpu_time: true,
        ..InFlightRun::default()
    };
    assert!(!claude.matches_row(&live.row()));
    assert!(Scope::Global.matches_row(&live.row()));
    assert!(claude.matches_row(
        &InFlightRun {
            gpu_time: false,
            ..live.clone()
        }
        .row()
    ));
    // The same rows as Claude Code's own (not GPU time) are gaps.
    let gaps = [row("unpriced", false, None), row("partial", false, None)];
    let gaps = UsageTotals::from_rows(&gaps, &claude);
    assert_eq!(gaps.missing(Metric::CostUsd), 2);
}

/// OB07: a killed run's priced floor (`usage_ledger.cost_floor_usd`) is never
/// spend. Read from the real ledger, the row stays an uncosted partial: no
/// dollars, a dash for a spending budget, and a $0.10 budget is not over.
#[test]
fn budgets_never_count_a_partial_runs_cost_floor() {
    let dir = tmp();
    let v = dir.path();
    let now = Utc::now();
    let mut row = crate::session_store::tests::usage_row_fixture(
        "killed",
        &crate::session_store::db_time(now),
        "cancelled",
    );
    row.surface = "workflow".into();
    row.scope = Some("brief".into());
    row.cached_input_tokens = Some(1_000_000);
    row.output_tokens = Some(10_000);
    row.cost_status = "partial".into();
    row.cost_floor_usd = Some(0.40);
    crate::session_store::upsert_usage(v, &row).unwrap();
    let def = budget("b1", "global", Metric::CostUsd, UsageWindow::Day, 0.10);
    let bounds = window_bounds_in(UsageWindow::Day, &now);
    let totals = LedgerReader::new(v).totals(&def, &bounds).unwrap();
    assert_eq!(totals.cost_usd, 0.0, "the floor is not spend");
    assert_eq!((totals.partial_runs, totals.uncosted_partial_runs), (1, 1));
    assert_eq!(
        totals.observed(Metric::CostUsd),
        None,
        "a dash, never the floor"
    );
    assert!(totals.value(Metric::CostUsd) < def.limit);
}

// --- evaluate (PR 8 plan tests 20-22, 24, 25, 29 at the pure level) ---

#[test]
fn evaluation_is_clear_without_budgets_and_never_reads_the_ledger() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let intents = [
        RunIntent::chat(Some(AgentKind::Claude)),
        RunIntent::workflow("brief", Some(AgentKind::Claude)),
        RunIntent::heartbeat(Some(AgentKind::Codex)),
        RunIntent::delegate(None, ParentRun::default()),
    ];
    let empty = BudgetsFile::default();
    // Budgets exist, but none covers these runs.
    let other = file_with(vec![
        budget(
            "b1",
            "workflow:other",
            Metric::Tokens,
            UsageWindow::Day,
            10.0,
        ),
        budget(
            "b2",
            "agent:antigravity",
            Metric::Tokens,
            UsageWindow::Day,
            10.0,
        ),
    ]);
    for intent in &intents {
        for file in [&empty, &other] {
            let eval = evaluate(file, intent, &never_read(), &now);
            assert_eq!(eval.blocked, None);
            assert!(eval.warnings.is_empty() && eval.to_pause.is_empty());
            assert!(eval.checks.is_empty());
        }
    }
}

#[test]
fn an_enforced_run_over_a_hard_limit_is_blocked_and_asks_for_a_pause() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let file = file_with(vec![budget(
        "b1",
        "workflow:brief",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let eval = evaluate(
        &file,
        &RunIntent::workflow("brief", Some(AgentKind::Claude)),
        &tokens(1200),
        &now,
    );
    let blocked = eval.blocked.expect("blocked");
    assert!(
        blocked.message.starts_with(BUDGET_PAUSED_PREFIX),
        "{}",
        blocked.message
    );
    assert!(
        blocked.message.contains("1,200 tokens of 1,000 tokens"),
        "{}",
        blocked.message
    );
    assert_eq!(blocked.level, NoticeLevel::Paused);
    assert_eq!(eval.to_pause.len(), 1);
    assert_eq!(eval.to_pause[0].bounds.key, "2026-09-25");
    assert_eq!(eval.checks[0].state, BudgetState::Over);
    assert!(eval.seed_levels.contains("b1:limit") && eval.seed_levels.contains("b1:warn"));

    // The same budget only warns chat, never pauses it.
    let file = file_with(vec![budget(
        "g",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let chat = evaluate(
        &file,
        &RunIntent::chat(Some(AgentKind::Claude)),
        &tokens(1200),
        &now,
    );
    assert_eq!(chat.blocked, None);
    assert!(chat.to_pause.is_empty());
    assert_eq!(chat.warnings.len(), 1);
    assert_eq!(chat.warnings[0].level, NoticeLevel::Limit);
    assert!(!chat.warnings[0].blocking);
    assert!(!chat.checks[0].enforced);
}

#[test]
fn a_soft_budget_over_its_limit_warns_even_an_automation() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let mut soft = budget("s", "global", Metric::Tokens, UsageWindow::Week, 1000.0);
    soft.hard_stop = false;
    let eval = evaluate(
        &file_with(vec![soft]),
        &RunIntent::heartbeat(Some(AgentKind::Claude)),
        &tokens(5000),
        &now,
    );
    assert_eq!(eval.blocked, None);
    assert!(eval.to_pause.is_empty());
    assert_eq!(eval.warnings[0].level, NoticeLevel::Limit);
}

#[test]
fn an_unreadable_ledger_blocks_only_enforced_hard_stop_budgets() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let hard = budget("h", "global", Metric::Tokens, UsageWindow::Day, 1000.0);
    let mut soft = hard.clone();
    soft.hard_stop = false;
    let workflow = RunIntent::workflow("brief", Some(AgentKind::Claude));

    let eval = evaluate(
        &file_with(vec![hard.clone()]),
        &workflow,
        &ledger_down(),
        &now,
    );
    let blocked = eval.blocked.expect("fails closed");
    assert_eq!(blocked.level, NoticeLevel::Unavailable);
    assert!(blocked.message.starts_with(BUDGET_PAUSED_PREFIX));
    assert!(blocked.message.contains("database is locked"));
    assert!(eval.to_pause.is_empty(), "no pause without an observation");
    assert_eq!(eval.checks[0].state, BudgetState::Unknown);

    assert_eq!(
        evaluate(&file_with(vec![soft]), &workflow, &ledger_down(), &now).blocked,
        None
    );
    assert_eq!(
        evaluate(
            &file_with(vec![hard]),
            &RunIntent::chat(Some(AgentKind::Claude)),
            &ledger_down(),
            &now
        )
        .blocked,
        None
    );
}

#[test]
fn crossing_the_warn_percent_warns_and_seeds_the_level() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let file = file_with(vec![budget(
        "w",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let workflow = RunIntent::workflow("brief", Some(AgentKind::Claude));
    let eval = evaluate(&file, &workflow, &tokens(850), &now);
    assert_eq!(eval.blocked, None);
    assert_eq!(eval.warnings.len(), 1);
    assert_eq!(eval.warnings[0].level, NoticeLevel::Warn);
    assert!(
        eval.warnings[0].message.contains("85%"),
        "{}",
        eval.warnings[0].message
    );
    assert_eq!(eval.seed_levels, BTreeSet::from(["w:warn".to_string()]));
    assert_eq!(eval.checks[0].state, BudgetState::Warning);

    let calm = evaluate(&file, &workflow, &tokens(100), &now);
    assert!(calm.warnings.is_empty() && calm.seed_levels.is_empty());
    assert_eq!(calm.checks[0].state, BudgetState::Ok);
}

#[test]
fn pauses_block_until_they_lapse_or_are_held_across_the_reset() {
    let today = at(0, 2026, 9, 25, 12, 0);
    let mut file = file_with(vec![budget(
        "b1",
        "workflow:brief",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let workflow = RunIntent::workflow("brief", Some(AgentKind::Claude));

    // A current pause blocks even when this window's usage is lower.
    file.pauses.insert("b1".into(), pause("2026-09-25", false));
    let eval = evaluate(&file, &workflow, &tokens(10), &today);
    assert!(eval.blocked.is_some());
    assert!(eval.to_pause.is_empty(), "already paused");
    assert!(eval.lapsed.is_empty());
    // Chat under the same pause is told, not blocked.
    let chat = evaluate(&file, &RunIntent::chat(None), &tokens(10), &today);
    assert_eq!(chat.blocked, None);
    assert!(chat.checks.is_empty(), "a workflow scope never covers chat");

    // Yesterday's unheld pause has lapsed: clear, and reported as lapsed.
    file.pauses.insert("b1".into(), pause("2026-09-24", false));
    let eval = evaluate(&file, &workflow, &tokens(10), &today);
    assert_eq!(eval.blocked, None);
    assert_eq!(eval.lapsed, vec!["b1".to_string()]);
    assert_eq!(eval.checks[0].state, BudgetState::Ok);
    assert_eq!(
        remove_lapsed(&mut file, &eval.lapsed),
        vec![("b1".to_string(), Some("r1".to_string()))]
    );
    assert!(file.pauses.is_empty());

    // "Keep paused" survives the reset.
    file.pauses.insert("b1".into(), pause("2026-09-24", true));
    let eval = evaluate(&file, &workflow, &tokens(10), &today);
    let blocked = eval.blocked.expect("held");
    assert!(
        blocked.message.contains("kept paused"),
        "{}",
        blocked.message
    );
    assert!(eval.lapsed.is_empty());

    // A pause whose budget is gone, or no longer hard-stops, lapses.
    let mut gone = BudgetsFile::default();
    gone.pauses.insert("b1".into(), pause("2026-09-25", true));
    assert_eq!(
        evaluate(&gone, &workflow, &never_read(), &today).lapsed,
        vec!["b1"]
    );
    file.budgets[0].hard_stop = false;
    let eval = evaluate(&file, &workflow, &tokens(10), &today);
    assert_eq!(eval.lapsed, vec!["b1"]);
    assert_eq!(eval.blocked, None);
}

#[test]
fn spending_headroom_is_the_smallest_left_under_enforced_hard_stops() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let spend = |n: f64| {
        move |_: &BudgetDef, _: &WindowBounds| -> Result<UsageTotals, String> {
            Ok(UsageTotals {
                cost_usd: n,
                ..UsageTotals::default()
            })
        }
    };
    let file = file_with(vec![
        budget("d", "global", Metric::CostUsd, UsageWindow::Day, 5.0),
        budget(
            "m",
            "agent:claude",
            Metric::CostUsd,
            UsageWindow::Month,
            3.5,
        ),
    ]);
    let workflow = RunIntent::workflow("brief", Some(AgentKind::Claude));
    let eval = evaluate(&file, &workflow, &spend(3.0), &now);
    assert!((eval.max_budget_usd.unwrap() - 0.5).abs() < 1e-9);
    let eval = evaluate(&file, &workflow, &spend(3.4999), &now);
    assert_eq!(eval.max_budget_usd, Some(0.01), "floored at one cent");
    assert_eq!(
        evaluate(
            &file,
            &RunIntent::chat(Some(AgentKind::Claude)),
            &spend(1.0),
            &now
        )
        .max_budget_usd,
        None,
        "chat is never stopped"
    );
}

#[test]
fn a_store_failure_blocks_only_enforced_intents() {
    let error = "budget settings at /x/budgets.json are unreadable (EOF)";
    for intent in [
        RunIntent::workflow("brief", None),
        RunIntent::heartbeat(None),
        RunIntent::delegate(None, ParentRun::default()),
    ] {
        let notice = load_failure_notice(&intent, error).expect("blocked");
        assert!(notice.blocking);
        assert!(notice.message.starts_with(BUDGET_PAUSED_PREFIX));
        assert!(notice.message.contains(error));
    }
    assert_eq!(load_failure_notice(&RunIntent::chat(None), error), None);
    assert_eq!(load_failure_notice(&RunIntent::oneshot(None), error), None);
}

// --- validation (PR 8 plan test 32, pure half) ------------------------

#[test]
fn budgets_that_can_never_trip_are_refused() {
    let codex_workflow = |scope: &Scope| match scope {
        Scope::Workflow(slug) if slug == "w" => Some(AgentKind::Codex),
        Scope::Heartbeat => Some(AgentKind::Antigravity),
        _ => None,
    };
    let check = |scope: &str, metric: Metric| {
        validate_def(
            &budget("n", scope, metric, UsageWindow::Day, 10.0),
            &[],
            &codex_workflow,
        )
    };
    assert!(check("agent:codex", Metric::CostUsd)
        .unwrap_err()
        .contains("no price of its own"));
    // Antigravity reports per-step tokens since 2026-11-20, no price of its own.
    assert!(check("agent:antigravity", Metric::CostUsd)
        .unwrap_err()
        .contains("no price of its own"));
    assert!(check("agent:antigravity", Metric::Tokens).is_ok());
    let err = check("workflow:w", Metric::CostUsd).unwrap_err();
    assert!(err.contains("Codex"), "names the agent: {err}");
    let err = check("heartbeat", Metric::CostUsd).unwrap_err();
    assert!(err.contains("Antigravity"), "names the agent: {err}");
    assert!(check("heartbeat", Metric::Tokens).is_ok());
    assert!(check("agent:codex", Metric::Tokens).is_ok());
    assert!(check("workflow:unresolved", Metric::CostUsd).is_ok());
    assert!(check("global", Metric::CostUsd).is_ok());

    let existing = budget("a", "global", Metric::Tokens, UsageWindow::Day, 10.0);
    let dup = budget("b", "global", Metric::Tokens, UsageWindow::Day, 20.0);
    assert!(validate_def(&dup, std::slice::from_ref(&existing), &codex_workflow).is_err());
    // Editing the same budget is not a duplicate; another window is fine.
    assert!(validate_def(&existing, std::slice::from_ref(&existing), &codex_workflow).is_ok());
    let weekly = budget("c", "global", Metric::Tokens, UsageWindow::Week, 20.0);
    assert!(validate_def(&weekly, std::slice::from_ref(&existing), &codex_workflow).is_ok());

    let mut bad = existing.clone();
    bad.limit = 0.0;
    assert!(validate_def(&bad, &[], &codex_workflow).is_err());
    bad.limit = f64::INFINITY;
    assert!(validate_def(&bad, &[], &codex_workflow).is_err());
    bad.limit = 5.0;
    bad.warn_percent = 0;
    assert!(validate_def(&bad, &[], &codex_workflow).is_err());
    bad.warn_percent = 101;
    assert!(validate_def(&bad, &[], &codex_workflow).is_err());

    // A workflow whose agent later changes becomes unenforceable.
    assert_eq!(
        enforceability(
            &Scope::Workflow("w".into()),
            Metric::CostUsd,
            Some(AgentKind::Antigravity),
            None
        )
        .0,
        Enforceability::Never
    );
    assert_eq!(
        enforceability(
            &Scope::Global,
            Metric::Tokens,
            None,
            Some(&UsageTotals::default())
        )
        .0,
        Enforceability::Enforced
    );
}

// --- store: path, round trip, corrupt file, tamper --------------------

#[test]
fn the_app_store_is_per_vault_under_test_and_outside_every_vault() {
    let (a, b) = (tmp(), tmp());
    let sa = Store::for_app(a.path()).unwrap();
    let sb = Store::for_app(b.path()).unwrap();
    assert_ne!(sa, sb);
    assert_eq!(sa, Store::for_app(a.path()).unwrap());
    let root = crate::test_temp::process_dir("hollow-test-budgets-");
    assert!(sa.path().starts_with(&root), "{:?}", sa.path());
    assert!(!sa.path().starts_with(a.path()));
    assert_eq!(sa.path().file_name().unwrap(), FILE_NAME);
    if let Some(real) = dirs::data_local_dir() {
        assert!(!sa.path().starts_with(real.join("com.hollow.app")));
    }
}

#[test]
fn a_saved_file_round_trips_privately_in_camel_case() {
    let dir = tmp();
    let store = Store::at(dir.path().join("nested").join(FILE_NAME));
    assert_eq!(
        store.load().unwrap(),
        BudgetsFile::default(),
        "missing = no budgets"
    );

    let mut file = file_with(vec![budget(
        "b1",
        "workflow:brief",
        Metric::CostUsd,
        UsageWindow::Week,
        2.5,
    )]);
    file.pauses.insert("b1".into(), pause("2026-W39", false));
    file.notices.insert(
        notice_key(false, "b1", "2026-W39", NoticeLevel::Warn),
        "2026-09-25T12:00:00.000Z".into(),
    );
    store.save(&file).unwrap();
    assert_eq!(store.load().unwrap(), file);

    let raw: Value = serde_json::from_slice(&fs::read(store.path()).unwrap()).unwrap();
    assert_eq!(raw["schemaVersion"], 1);
    assert_eq!(raw["budgets"][0]["scope"], "workflow:brief");
    assert_eq!(raw["budgets"][0]["metric"], "costUsd");
    assert_eq!(raw["budgets"][0]["window"], "week");
    assert_eq!(raw["budgets"][0]["warnPercent"], 80);
    assert_eq!(raw["budgets"][0]["hardStop"], true);
    assert_eq!(raw["pauses"]["b1"]["windowKey"], "2026-W39");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(store.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let leftovers: Vec<_> = fs::read_dir(store.path().parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no temp files left behind");

    // A hand-written file without the optional keys uses the defaults.
    fs::write(
        store.path(),
        r#"{"schemaVersion":1,"budgets":[{"id":"x","scope":"global","metric":"tokens","window":"day","limit":5}]}"#,
    )
    .unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.budgets[0].warn_percent, DEFAULT_WARN_PERCENT);
    assert!(loaded.budgets[0].hard_stop, "missing hardStop fails safe");
    assert!(loaded.pauses.is_empty() && loaded.notices.is_empty());
}

// PR 8 plan test 23 (store half; preflight's use of it comes later).
#[test]
fn a_corrupt_budgets_file_fails_closed_notifies_once_and_is_never_overwritten() {
    let vault = tmp();
    let dir = tmp();
    let store = Store::at(dir.path().join(FILE_NAME));
    fs::write(store.path(), "{not json").unwrap();

    let err = store.load().unwrap_err();
    assert!(err.contains("unreadable"), "{err}");
    assert!(
        err.contains(&store.path().display().to_string()),
        "names the file: {err}"
    );
    assert!(err.contains("move or repair"), "{err}");

    assert!(store.save(&BudgetsFile::default()).is_err());
    let wrote = store.with_file(vault.path(), |file| {
        file.budgets
            .push(budget("b", "global", Metric::Tokens, UsageWindow::Day, 1.0));
        Ok(((), true))
    });
    assert!(wrote.is_err());
    assert_eq!(
        fs::read(store.path()).unwrap(),
        b"{not json",
        "bytes unchanged"
    );

    let workflow = RunIntent::workflow("brief", None);
    assert!(load_failure_notice(&workflow, &err).is_some());
    assert!(load_failure_notice(&RunIntent::heartbeat(None), &err).is_some());
    assert!(load_failure_notice(&RunIntent::chat(None), &err).is_none());

    report_load_error(vault.path(), &err);
    report_load_error(vault.path(), &err);
    assert_eq!(
        sent_for_test(vault.path()),
        vec!["Automations are paused: budget settings unreadable".to_string()],
        "one notice per distinct error per hour"
    );
    let health = crate::health::status(vault.path(), HEALTH_SUBSYSTEM);
    assert_eq!(
        health.last.map(|e| e.outcome),
        Some(crate::health::Outcome::Failed)
    );

    // Repairing the file (here: removing it) is picked up by the next
    // read: health says so once, and a later breakage is announced at
    // once instead of waiting out the hour.
    fs::remove_file(store.path()).unwrap();
    assert_eq!(store.load().unwrap(), BudgetsFile::default());
    report_load_ok(vault.path(), &store);
    report_load_ok(vault.path(), &store);
    let health = crate::health::status(vault.path(), HEALTH_SUBSYSTEM);
    assert_eq!(
        health.last.map(|e| e.outcome),
        Some(crate::health::Outcome::Ok)
    );
    assert_eq!(health.consecutive_failures, 0);
    let lines = fs::read_to_string(vault.path().join(crate::health::FILE)).unwrap();
    assert_eq!(
        lines.lines().count(),
        2,
        "one failure, one recovery: {lines}"
    );

    fs::write(store.path(), "{not json").unwrap();
    let again = store.load().unwrap_err();
    assert_eq!(again, err);
    report_load_error(vault.path(), &again);
    assert_eq!(sent_for_test(vault.path()).len(), 2);
}

#[test]
fn unknown_schemas_and_invalid_budgets_are_treated_as_corrupt() {
    let dir = tmp();
    let store = Store::at(dir.path().join(FILE_NAME));
    let cases = [
        (r#"{"schemaVersion":2,"budgets":[]}"#, "schema version (2"),
        (r#"{"budgets":[]}"#, "schema version (missing"),
        (
            r#"{"schemaVersion":1,"budgets":[{"id":"a","scope":"global","metric":"tokens","window":"day","limit":-1}]}"#,
            "invalid",
        ),
        (
            r#"{"schemaVersion":1,"budgets":[{"id":"a","scope":"global","metric":"tokens","window":"day","limit":1},{"id":"a","scope":"heartbeat","metric":"tokens","window":"day","limit":1}]}"#,
            "duplicate budget id",
        ),
        (
            r#"{"schemaVersion":1,"budgets":[{"id":"a","scope":"global","metric":"tokens","window":"day","limit":1},{"id":"b","scope":"global","metric":"tokens","window":"day","limit":9}]}"#,
            "two day tokens budgets",
        ),
        (
            r#"{"schemaVersion":1,"budgets":[{"id":"a","scope":"workflow:../x","metric":"tokens","window":"day","limit":1}]}"#,
            "unreadable",
        ),
        (
            r#"{"schemaVersion":1,"budgets":[{"id":"a","scope":"global","metric":"tokens","window":"year","limit":1}]}"#,
            "unreadable",
        ),
    ];
    for (body, expected) in cases {
        fs::write(store.path(), body).unwrap();
        let err = store.load().unwrap_err();
        assert!(err.contains(expected), "{body} → {err}");
        assert!(store.save(&BudgetsFile::default()).is_err());
        assert_eq!(fs::read_to_string(store.path()).unwrap(), body);
    }
    // Hollow itself never writes an invalid file.
    fs::remove_file(store.path()).unwrap();
    let mut bad = budget("a", "global", Metric::Tokens, UsageWindow::Day, 1.0);
    bad.warn_percent = 0;
    assert!(store.save(&file_with(vec![bad])).is_err());
    assert!(!store.path().exists());
}

// PR 8 plan test 34, host-save half. The run-end half (pause at the
// claim-time limit, 8b) is the next test.
#[test]
fn foreign_edits_are_flagged_once_at_host_saves_and_merged_not_discarded() {
    let vault = tmp();
    let dir = tmp();
    let store = Store::at(dir.path().join(FILE_NAME));
    let original = file_with(vec![budget(
        "b1",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    store.save(&original).unwrap();
    let (_, host) = store.snapshot().unwrap();
    assert!(store.is_host_hash(host.as_deref()));

    // Something else rewrites the limit.
    let mut edited = original.clone();
    edited.budgets[0].limit = 10_000_000.0;
    fs::write(store.path(), serde_json::to_vec(&edited).unwrap()).unwrap();
    let (_, foreign) = store.snapshot().unwrap();
    assert!(!store.is_host_hash(foreign.as_deref()));
    assert!(!store.foreign_edit_seen(), "a plain read flags nothing");

    // A host write of a different budget flags it once and keeps the edit.
    store
        .with_file(vault.path(), |file| {
            file.budgets.push(budget(
                "b2",
                "heartbeat",
                Metric::Tokens,
                UsageWindow::Week,
                5.0,
            ));
            Ok(((), true))
        })
        .unwrap();
    assert!(store.foreign_edit_seen());
    assert_eq!(
        sent_for_test(vault.path()),
        vec!["Budgets changed outside Hollow".to_string()]
    );
    let saved = store.load().unwrap();
    assert_eq!(
        saved.budgets[0].limit, 10_000_000.0,
        "flagged, not discarded"
    );
    assert_eq!(saved.budgets.len(), 2);

    // Hollow's own save is trusted again: no second notice.
    store.with_file(vault.path(), |_| Ok(((), false))).unwrap();
    assert_eq!(sent_for_test(vault.path()).len(), 1);
}

// PR 8 plan test 34 part 1 (deferred from 8a, done in 8b): an agent that
// raises its own limit in budgets.json during the run gets no extra room.
// The run's claim read limit 1,000; the file now says 10,000,000; usage
// is 1,200. `after_run_claimed` pauses at the CLAIM's limit and says so;
// without the claim (plain `after_run`) the raised limit would stand.
#[test]
fn a_limit_raised_during_the_run_is_held_to_the_claims_limit_at_run_end() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    let intent = RunIntent::workflow("brief", CLAUDE);
    let Preflight::Clear(claimed) = preflight(&store, v, &intent, Utc::now()) else {
        panic!("nothing used yet");
    };
    assert_eq!(claimed.claimed_limits.get("b1"), Some(&1000.0));
    assert!(claimed.file_hash.is_some());
    // Mid-run: something other than Hollow raises the limit, and the run
    // uses 1,200 tokens.
    let path = store.as_ref().unwrap().path().to_path_buf();
    let mut raised: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    raised["budgets"][0]["limit"] = json!(10_000_000.0);
    fs::write(&path, serde_json::to_vec(&raised).unwrap()).unwrap();
    ledger(v, "r1", "workflow", Some("brief"), 1200);

    // Unclaimed, the raised limit stands (8a's behaviour, kept for chat).
    let control = tmp();
    let cv = control.path();
    let control_store = app_store(
        cv,
        vec![budget(
            "b1",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            10_000_000.0,
        )],
    );
    ledger(cv, "r1", "workflow", Some("brief"), 1200);
    assert!(after_run(&control_store, cv, &intent, Utc::now()).is_empty());

    let effects = after_run_claimed(&store, v, &intent, Some(&claimed), Utc::now());
    assert!(
        effects.contains(&BudgetEffect::ClaimLimitKept {
            budget_id: "b1".into()
        }),
        "{effects:?}"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, BudgetEffect::Paused { .. })),
        "{effects:?}"
    );
    let file = loaded(&store);
    assert_eq!(
        file.pauses["b1"].limit, 1000.0,
        "paused at the claim's limit"
    );
    assert_eq!(
        file.budgets[0].limit, 10_000_000.0,
        "the edit itself is kept (flagged, not discarded)"
    );
    assert!(
        preflight(&store, v, &intent, Utc::now())
            .blocked()
            .is_some(),
        "the next claim is blocked"
    );

    // An unchanged file never uses the claim's limits: a claim at 1,000
    // over a file still at 1,000 pauses the ordinary way, once.
    let plain = tmp();
    let pv = plain.path();
    let plain_store = app_store(
        pv,
        vec![budget(
            "b1",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    let Preflight::Clear(plain_claim) = preflight(&plain_store, pv, &intent, Utc::now()) else {
        panic!("nothing used yet");
    };
    ledger(pv, "r1", "workflow", Some("brief"), 1200);
    let effects = after_run_claimed(&plain_store, pv, &intent, Some(&plain_claim), Utc::now());
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, BudgetEffect::ClaimLimitKept { .. })),
        "{effects:?}"
    );
    assert!(effects
        .iter()
        .any(|e| matches!(e, BudgetEffect::Paused { .. })));
}

#[test]
fn trust_on_first_use_covers_an_existing_file_and_flags_a_created_one() {
    let vault = tmp();
    let dir = tmp();
    // An existing file seen for the first time is trusted as-is.
    let existing = Store::at(dir.path().join("existing.json"));
    let body = serde_json::to_vec(&file_with(vec![budget(
        "b1",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1.0,
    )]))
    .unwrap();
    fs::write(existing.path(), &body).unwrap();
    existing.load_for_write(vault.path()).unwrap();
    assert!(!existing.foreign_edit_seen());

    // A store Hollow saw absent, then created by someone else, is flagged.
    let created = Store::at(dir.path().join("created.json"));
    assert_eq!(created.load().unwrap(), BudgetsFile::default());
    fs::write(created.path(), &body).unwrap();
    created.load_for_write(vault.path()).unwrap();
    assert!(created.foreign_edit_seen());
}

// --- notices, episodes, suggestions -----------------------------------

#[test]
fn notices_are_pruned_when_their_window_or_budget_is_gone() {
    let now = at(0, 2026, 9, 25, 12, 0);
    let mut file = file_with(vec![budget(
        "b1",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1.0,
    )]);
    let stamp = "2026-09-25T00:00:00.000Z".to_string();
    let keep = notice_key(false, "b1", "2026-09-25", NoticeLevel::Warn);
    let keep_chat = notice_key(true, "b1", "2026-09-25", NoticeLevel::Limit);
    assert_eq!(keep_chat, "chat:b1:2026-09-25:limit");
    for key in [
        keep.clone(),
        keep_chat.clone(),
        notice_key(false, "b1", "2026-09-24", NoticeLevel::Warn),
        notice_key(true, "gone", "2026-09-25", NoticeLevel::Warn),
        "garbage".to_string(),
    ] {
        file.notices.insert(key, stamp.clone());
    }
    assert!(prune_notices(&mut file, &now));
    assert_eq!(
        file.notices.keys().cloned().collect::<Vec<_>>(),
        vec![keep, keep_chat]
    );
    assert!(!prune_notices(&mut file, &now));
}

#[test]
fn a_new_pause_episode_is_named_by_its_start() {
    let now = at(-4, 2026, 9, 25, 10, 30);
    let request = PauseRequest {
        budget_id: "b1".into(),
        bounds: window_bounds_in(UsageWindow::Day, &now),
        observed: 1200.0,
        limit: 1000.0,
    };
    let pause = new_pause(&request, &now);
    assert_eq!(pause.episode, "2026-09-25T14:30:00.000Z");
    assert_eq!(pause.paused_at, pause.episode);
    assert_eq!(pause.window_key, "2026-09-25");
    assert_eq!(pause.window_end, "2026-09-26T04:00:00.000Z");
    assert!(!pause.held && pause.review_item_id.is_none());
    assert_eq!(
        pause_fingerprint("b1", &pause.episode),
        "budget-pause:b1:2026-09-25t14:30:00.000z"
    );
}

// PR 8 plan test 35 (pure half; the ledger's first row comes later).
#[test]
fn suggestions_need_a_week_of_data() {
    assert_eq!(suggest_limit(&[]), None);
    assert_eq!(suggest_limit(&[0.0]), None);
    assert_eq!(suggest_limit(&[100.0, 400.0, 250.0]), Some(600.0));
    assert_eq!(suggest_limit(&[123.0]), Some(190.0)); // 184.5 → 190
    let cents = suggest_limit(&[0.37]).unwrap(); // 0.555 → 0.56
    assert!((cents - 0.56).abs() < 1e-9, "{cents}");
    assert_eq!(suggest_limit(&[80_000.0, f64::NAN]), Some(120_000.0));

    let now = utc("2026-09-25T12:00:00Z");
    assert!(!suggestions_available(None, now));
    // Six days, 23:59:59 of data is still under a week.
    assert!(!suggestions_available(
        Some(utc("2026-09-18T12:00:01Z")),
        now
    ));
    assert_eq!(ledger_days(Some(utc("2026-09-18T12:00:01Z")), now), 6);
    assert!(suggestions_available(
        Some(utc("2026-09-18T12:00:00Z")),
        now
    ));
}

#[test]
fn amounts_read_naturally() {
    assert_eq!(
        format_amount(Metric::Tokens, 1_234_567.4, false),
        "1,234,567 tokens"
    );
    assert_eq!(format_amount(Metric::Tokens, 999.0, false), "999 tokens");
    assert_eq!(format_amount(Metric::CostUsd, 1.5, true), "$1.50 estimated");
    assert_eq!(format_amount(Metric::CostUsd, 0.004, false), "$0.00");
}

// --- preflight, after_run and the pause lifecycle (real store + ledger)

const CLAUDE: Option<AgentKind> = Some(AgentKind::Claude);

/// One finished ledger row at `at` counting `tokens` for this surface.
fn ledger_at(
    v: &Path,
    key: &str,
    surface: &str,
    scope: Option<&str>,
    tokens: i64,
    at: DateTime<Utc>,
) {
    let mut row = crate::session_store::tests::usage_row_fixture(
        key,
        &crate::session_store::db_time(at),
        "success",
    );
    row.surface = surface.into();
    row.scope = scope.map(str::to_string);
    row.input_tokens = Some(tokens);
    row.cached_input_tokens = Some(0);
    row.output_tokens = Some(0);
    row.cost_status = "unpriced".into();
    crate::session_store::upsert_usage(v, &row).unwrap();
}

fn ledger(v: &Path, key: &str, surface: &str, scope: Option<&str>, tokens: i64) {
    ledger_at(v, key, surface, scope, tokens, Utc::now());
}

/// The per-vault app store, seeded with `budgets`.
fn app_store(v: &Path, budgets: Vec<BudgetDef>) -> Result<Store, String> {
    let store = Store::for_app(v);
    store.as_ref().unwrap().save(&file_with(budgets)).unwrap();
    store
}

fn loaded(store: &Result<Store, String>) -> BudgetsFile {
    store.as_ref().unwrap().load().unwrap()
}

fn pause_items(v: &Path) -> Vec<crate::review_queue::ReviewItem> {
    crate::review_queue::list(v, Some(REVIEW_KIND)).unwrap()
}

fn with_limit(limit: f64) -> Value {
    json!({ "newLimit": limit })
}

fn input(id: Option<&str>, scope: &str, metric: Metric, limit: f64) -> BudgetInput {
    BudgetInput {
        id: id.map(str::to_string),
        scope: scope.into(),
        metric,
        window: "day".into(),
        limit,
        warn_percent: None,
        hard_stop: None,
    }
}

fn no_agent(_: &Scope) -> Option<AgentKind> {
    None
}

// PR 8 plan test 20.
#[test]
fn preflight_blocks_an_automation_over_its_hard_limit_and_pauses_once() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    let intent = RunIntent::workflow("brief", CLAUDE);

    let first = preflight(&store, v, &intent, Utc::now());
    let notice = first.blocked().expect("blocked").clone();
    assert!(
        notice.message.starts_with(BUDGET_PAUSED_PREFIX),
        "{}",
        notice.message
    );
    let file = loaded(&store);
    let pause = file.pauses.get("b1").expect("paused").clone();
    let items = pause_items(v);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].status, "pending");
    assert_eq!(items[0].source, REVIEW_SOURCE);
    assert_eq!(items[0].source_path, None);
    assert_eq!(
        items[0].fingerprint,
        pause_fingerprint("b1", &pause.episode)
    );
    assert_eq!(pause.review_item_id.as_deref(), Some(items[0].id.as_str()));
    assert_eq!(items[0].payload["scopeLabel"], "the “brief” automation");
    assert_eq!(
        sent_for_test(v),
        vec![format!("{BUDGET_PAUSED_PREFIX} the “brief” automation")]
    );

    // Still blocked, still one item and one notification.
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    assert_eq!(pause_items(v).len(), 1);
    assert_eq!(sent_for_test(v).len(), 1);
    // Another workflow, and chat, run.
    assert!(
        preflight(&store, v, &RunIntent::workflow("other", CLAUDE), Utc::now())
            .blocked()
            .is_none()
    );
    assert!(preflight(&store, v, &RunIntent::chat(CLAUDE), Utc::now())
        .blocked()
        .is_none());
    // Read-only checks agree and write nothing.
    assert!(is_blocked(&store, v, &intent, Utc::now()).is_some());
    assert!(matches!(
        preflight_readonly(&store, Some(v), &intent, Utc::now()),
        Preflight::Blocked(_)
    ));
    let holds = workflow_holds(
        v,
        &[crate::vault_workflows::seed_workflow_with(
            v,
            "Brief",
            "manual",
            "BODY",
            |fm| fm.agent = Some("claude".into()),
        )],
        Utc::now(),
    );
    assert!(holds
        .get("brief")
        .is_some_and(|m| m.starts_with(BUDGET_PAUSED_PREFIX)));
}

// PR 8 plan test 21.
#[test]
fn preflight_warns_chat_and_never_blocks_or_pauses() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "g",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "c1", "chat", Some("s-1"), 1200);
    let chat = RunIntent::chat(CLAUDE);
    let Preflight::Warn(_, notices) = preflight(&store, v, &chat, Utc::now()) else {
        panic!("chat is warned");
    };
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].level, NoticeLevel::Limit);
    assert!(!notices[0].blocking);
    assert!(
        loaded(&store).pauses.is_empty(),
        "chat never pauses at claim"
    );
    assert!(pause_items(v).is_empty());
    assert_eq!(sent_for_test(v), vec!["Budget limit reached".to_string()]);
    // Once per window: the next turn gets no conversation notice.
    assert!(matches!(
        preflight(&store, v, &chat, Utc::now()),
        Preflight::Clear(_)
    ));
    assert_eq!(sent_for_test(v).len(), 1);
}

// PR 8 plan test 22.
#[test]
fn preflight_is_clear_without_budgets_and_never_writes() {
    let dir = tmp();
    let v = dir.path();
    let store = Store::for_app(v);
    for intent in [
        RunIntent::chat(CLAUDE),
        RunIntent::workflow("brief", CLAUDE),
        RunIntent::heartbeat(CLAUDE),
        RunIntent::delegate(None, ParentRun::default()),
    ] {
        assert!(matches!(
            preflight(&store, v, &intent, Utc::now()),
            Preflight::Clear(_)
        ));
        assert!(after_run(&store, v, &intent, Utc::now()).is_empty());
        assert!(is_blocked(&store, v, &intent, Utc::now()).is_none());
    }
    assert!(!store.as_ref().unwrap().path().exists(), "nothing written");
    assert!(sent_for_test(v).is_empty());
}

// PR 8 plan test 23 (through preflight).
#[test]
fn a_corrupt_budgets_file_holds_automations_through_preflight_only() {
    let dir = tmp();
    let v = dir.path();
    let store = Store::for_app(v);
    let path = store.as_ref().unwrap().path().to_path_buf();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{not json").unwrap();
    for intent in [
        RunIntent::workflow("brief", CLAUDE),
        RunIntent::heartbeat(CLAUDE),
    ] {
        let notice = preflight(&store, v, &intent, Utc::now())
            .blocked()
            .expect("fails closed")
            .clone();
        assert!(notice.message.contains("unreadable"), "{}", notice.message);
    }
    assert!(matches!(
        preflight(&store, v, &RunIntent::chat(CLAUDE), Utc::now()),
        Preflight::Clear(_)
    ));
    assert!(after_run(&store, v, &RunIntent::chat(CLAUDE), Utc::now()).is_empty());
    assert!(save(
        &store,
        v,
        input(None, "global", Metric::Tokens, 5.0),
        &no_agent,
        Utc::now()
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{not json", "never overwritten");
    assert_eq!(
        sent_for_test(v).len(),
        1,
        "one notice per distinct error per hour"
    );
    let view = view(&store, v, &no_agent, Utc::now());
    assert!(view.store_error.is_some() && view.blocked.all_automations);
}

// PR 8 plan test 36: the chat funnel's order (row → after_run → release)
// lets the very next claimed automation see the pause.
#[test]
fn a_chat_turn_that_crosses_a_hard_limit_blocks_the_next_claimed_automation() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "g",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "c1", "chat", Some("s-1"), 400);
    assert!(
        preflight(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now())
            .blocked()
            .is_none()
    );
    // The turn's row, then its after_run.
    ledger(v, "c2", "chat", Some("s-1"), 700);
    let effects = after_run(&store, v, &RunIntent::chat(CLAUDE), Utc::now());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, BudgetEffect::Paused { budget_id, .. } if budget_id == "g")),
        "{effects:?}"
    );
    assert_eq!(pause_items(v).len(), 1);
    assert!(
        preflight(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    assert!(
        preflight(&store, v, &RunIntent::heartbeat(CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    assert!(
        preflight(&store, v, &RunIntent::chat(CLAUDE), Utc::now())
            .blocked()
            .is_none(),
        "chat keeps working"
    );
    // A second after_run opens nothing new.
    assert!(!after_run(&store, v, &RunIntent::chat(CLAUDE), Utc::now())
        .iter()
        .any(|e| matches!(e, BudgetEffect::Paused { .. })));
    assert_eq!(pause_items(v).len(), 1);
}

// PR 8 plan test 25.
#[test]
fn warn_percent_crossing_notifies_once_per_window() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "w",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 850);
    let intent = RunIntent::workflow("brief", CLAUDE);
    let effects = after_run(&store, v, &intent, Utc::now());
    assert_eq!(
        effects,
        vec![BudgetEffect::Warned {
            key: notice_key(
                false,
                "w",
                &window_bounds_in(UsageWindow::Day, &local(Utc::now())).key,
                NoticeLevel::Warn
            )
        }]
    );
    assert!(
        after_run(&store, v, &intent, Utc::now()).is_empty(),
        "once per window"
    );
    assert_eq!(sent_for_test(v), vec!["Budget warning".to_string()]);
    // The next window starts fresh: a row there warns again.
    let tomorrow = Utc::now() + chrono::Duration::days(1);
    ledger_at(v, "r2", "workflow", Some("brief"), 900, tomorrow);
    assert_eq!(after_run(&store, v, &intent, tomorrow).len(), 1);
}

// PR 8 plan test 27.
#[test]
fn resume_requires_a_limit_above_observed_and_claims_before_accepting() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    let intent = RunIntent::workflow("brief", CLAUDE);
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let item = pause_items(v).remove(0);

    let err = resume(&store, v, "b1", 1200.0, Utc::now()).unwrap_err();
    assert!(err.contains("above this day's usage"), "{err}");
    assert!(loaded(&store).pauses.contains_key("b1"));
    let err = accept_pause(&store, v, &item, Some(&with_limit(1100.0)), Utc::now()).unwrap_err();
    assert!(err.contains("above"), "{err}");
    assert_eq!(pause_items(v)[0].status, "pending", "nothing claimed");
    assert!(
        accept_pause(&store, v, &item, None, Utc::now()).is_err(),
        "a new limit is required"
    );

    let accepted = accept_pause(&store, v, &item, Some(&with_limit(1500.0)), Utc::now()).unwrap();
    assert_eq!(accepted.status, "accepted");
    assert_eq!(accepted.payload["newLimit"], 1500.0);
    let file = loaded(&store);
    assert!(file.pauses.is_empty());
    assert_eq!(file.budgets[0].limit, 1500.0);
    assert!(matches!(
        preflight(&store, v, &intent, Utc::now()),
        Preflight::Clear(_)
    ));

    // Settings, Resume claims and accepts a still-pending card.
    save(
        &store,
        v,
        input(Some("b1"), "global", Metric::Tokens, 1000.0),
        &no_agent,
        Utc::now(),
    )
    .unwrap();
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let second = pause_items(v)
        .into_iter()
        .find(|i| i.status == "pending")
        .expect("a new episode opens a new card");
    resume(&store, v, "b1", 2000.0, Utc::now()).unwrap();
    let after: Vec<_> = pause_items(v)
        .into_iter()
        .filter(|i| i.id == second.id)
        .collect();
    assert_eq!(after[0].status, "accepted");
    assert!(loaded(&store).pauses.is_empty());
}

// PR 8 plan test 28.
#[test]
fn keep_paused_holds_across_the_reset_and_rolls_back_if_reject_fails() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "heartbeat",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "h1", "heartbeat", None, 1200);
    let intent = RunIntent::heartbeat(CLAUDE);
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let item = pause_items(v).remove(0);

    let rejected = hold_pause_and_reject(&store, v, &item).unwrap();
    assert_eq!(rejected.status, "rejected");
    assert!(loaded(&store).pauses["b1"].held);
    let tomorrow = Utc::now() + chrono::Duration::days(1);
    let held = preflight(&store, v, &intent, tomorrow);
    assert!(held
        .blocked()
        .is_some_and(|n| n.message.contains("kept paused")));
    assert!(
        loaded(&store).pauses.contains_key("b1"),
        "held across the reset"
    );

    // A card already rejected elsewhere: the hold is rolled back.
    let mut file = loaded(&store);
    file.pauses.get_mut("b1").unwrap().held = false;
    store.as_ref().unwrap().save(&file).unwrap();
    assert!(hold_pause_and_reject(&store, v, &item).is_err());
    assert!(!loaded(&store).pauses["b1"].held, "restored");

    // Resume above tomorrow's usage clears it; the rejection stays.
    resume(&store, v, "b1", 1500.0, tomorrow).unwrap();
    assert!(loaded(&store).pauses.is_empty());
    assert_eq!(pause_items(v)[0].status, "rejected");
    assert!(matches!(
        preflight(&store, v, &intent, tomorrow),
        Preflight::Clear(_)
    ));
}

/// The IPC routing for pause cards (`resolve_review_item` and
/// `snooze_review_item` delegate to these): "reject" is Keep paused,
/// never the plain queue rejection; a card whose fingerprint changed is
/// refused before anything is claimed; a pause is never snoozed.
#[test]
fn pause_cards_route_reject_to_keep_paused_check_the_fingerprint_and_refuse_snooze() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "heartbeat",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "h1", "heartbeat", None, 1200);
    assert!(
        preflight(&store, v, &RunIntent::heartbeat(CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    let item = pause_items(v).remove(0);

    let err = refuse_pause_snooze(v, &item.id).unwrap_err();
    assert!(err.contains("can't be snoozed"), "{err}");
    assert_eq!(pause_items(v)[0].status, "pending");

    for action in ["accept", "reject"] {
        let err = resolve_pause(
            &store,
            v,
            &item,
            action,
            Some("budget-pause:b1:someone-else"),
            Some(&with_limit(5000.0)),
            Utc::now(),
        )
        .unwrap_err();
        assert!(err.contains("changed after you opened it"), "{err}");
        let now_item = pause_items(v).remove(0);
        assert_eq!(now_item.status, "pending", "nothing claimed on a {action}");
        let file = loaded(&store);
        assert!(!file.pauses["b1"].held && file.budgets[0].limit == 1000.0);
    }
    assert!(resolve_pause(&store, v, &item, "snooze", None, None, Utc::now()).is_err());

    let fingerprint = item.fingerprint.to_uppercase();
    let rejected = resolve_pause(
        &store,
        v,
        &item,
        " Reject ",
        Some(&fingerprint),
        None,
        Utc::now(),
    )
    .unwrap();
    assert_eq!(rejected.status, "rejected");
    assert!(
        loaded(&store).pauses["b1"].held,
        "Keep paused holds the pause across the reset"
    );
    // A non-pause item can still be snoozed.
    let other = crate::review_queue::enqueue(
        v,
        crate::review_queue::NewReviewItem {
            kind: "heartbeat_task".into(),
            fingerprint: "task:x".into(),
            payload: json!({ "title": "x" }),
            risk: "local_write".into(),
            source: "heartbeat".into(),
            source_path: None,
        },
    )
    .unwrap();
    assert!(refuse_pause_snooze(v, &other.id).is_ok());
}

/// PR 8 plan test 27's failure half: a save that fails after the claim
/// leaves the card pending with the error, and the pause and the old
/// limit in place.
#[cfg(unix)]
#[test]
fn a_raise_whose_save_fails_after_the_claim_leaves_the_card_pending() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    assert!(
        preflight(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    let item = pause_items(v).remove(0);
    let parent = store
        .as_ref()
        .unwrap()
        .path()
        .parent()
        .unwrap()
        .to_path_buf();
    // Reads still work; the atomic save's temp file cannot be created.
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o500)).unwrap();
    let result = accept_pause(&store, v, &item, Some(&with_limit(1500.0)), Utc::now());
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o700)).unwrap();
    let err = result.unwrap_err();
    let after = pause_items(v)
        .into_iter()
        .find(|i| i.id == item.id)
        .unwrap();
    assert_eq!(after.status, "pending", "{after:?}");
    let recorded = after.error.as_deref().unwrap_or_default();
    assert!(
        !recorded.is_empty() && err.starts_with(recorded.trim_end_matches('…')),
        "{recorded} / {err}"
    );
    let file = loaded(&store);
    assert!(
        file.pauses.contains_key("b1"),
        "the pause is still in budgets.json"
    );
    assert_eq!(file.budgets[0].limit, 1000.0);
    // Once the store is writable again the same card resumes.
    let accepted = accept_pause(&store, v, &item, Some(&with_limit(1500.0)), Utc::now()).unwrap();
    assert_eq!(accepted.status, "accepted");
}

// PR 8 plan test 29.
#[test]
fn an_unheld_pause_lapses_at_the_window_reset_and_supersedes_its_review() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    let intent = RunIntent::workflow("brief", CLAUDE);
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let tomorrow = Utc::now() + chrono::Duration::days(1);
    assert!(matches!(
        preflight(&store, v, &intent, tomorrow),
        Preflight::Clear(_)
    ));
    assert!(loaded(&store).pauses.is_empty());
    let item = pause_items(v).remove(0);
    assert_eq!(item.status, "rejected");
    assert!(item
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with(crate::review_queue::SUPERSEDED_PREFIX)));
    // Its card now only explains that the pause ended.
    let err = accept_pause(&store, v, &item, Some(&with_limit(5000.0)), tomorrow).unwrap_err();
    assert_eq!(err, ENDED_MESSAGE);
}

// PR 8 plan tests 30 and 31.
#[test]
fn raising_in_settings_clears_the_pause_repausing_opens_a_fresh_card_and_delete_supersedes() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "agent:claude",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    let intent = RunIntent::workflow("brief", CLAUDE);
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let first = pause_items(v).remove(0);

    // A lower edit keeps the pause; a raise above usage clears it.
    save(
        &store,
        v,
        input(Some("b1"), "agent:claude", Metric::Tokens, 900.0),
        &no_agent,
        Utc::now(),
    )
    .unwrap();
    assert!(loaded(&store).pauses.contains_key("b1"));
    save(
        &store,
        v,
        input(Some("b1"), "agent:claude", Metric::Tokens, 1500.0),
        &no_agent,
        Utc::now(),
    )
    .unwrap();
    assert!(loaded(&store).pauses.is_empty());
    let superseded = pause_items(v)
        .into_iter()
        .find(|i| i.id == first.id)
        .unwrap();
    assert!(superseded.error.unwrap().starts_with("Superseded:"));

    // Lowering back re-pauses as a NEW episode with its own card.
    std::thread::sleep(std::time::Duration::from_millis(5));
    save(
        &store,
        v,
        input(Some("b1"), "agent:claude", Metric::Tokens, 1000.0),
        &no_agent,
        Utc::now(),
    )
    .unwrap();
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let items = pause_items(v);
    assert_eq!(items.len(), 2);
    let pending: Vec<_> = items.iter().filter(|i| i.status == "pending").collect();
    assert_eq!(pending.len(), 1);
    assert_ne!(pending[0].fingerprint, first.fingerprint);
    assert_eq!(
        loaded(&store).pauses["b1"].review_item_id.as_deref(),
        Some(pending[0].id.as_str())
    );

    delete(&store, v, "b1").unwrap();
    let file = loaded(&store);
    assert!(file.budgets.is_empty() && file.pauses.is_empty());
    assert!(pause_items(v).iter().all(|i| i.status == "rejected"));
    delete(&store, v, "b1").unwrap();
}

// PR 8 plan test 32 (through save).
#[test]
fn save_refuses_budgets_that_can_never_trip_and_duplicates() {
    let dir = tmp();
    let v = dir.path();
    let store = Store::for_app(v);
    let codex_w = |scope: &Scope| match scope {
        Scope::Workflow(slug) if slug == "w" => Some(AgentKind::Codex),
        _ => None,
    };
    let err = save(
        &store,
        v,
        input(None, "workflow:w", Metric::CostUsd, 5.0),
        &codex_w,
        Utc::now(),
    )
    .unwrap_err();
    assert!(err.contains("Codex"), "{err}");
    assert!(save(
        &store,
        v,
        input(None, "agent:antigravity", Metric::CostUsd, 5.0),
        &codex_w,
        Utc::now()
    )
    .is_err());
    let saved = save(
        &store,
        v,
        input(None, "agent:codex", Metric::Tokens, 5.0),
        &codex_w,
        Utc::now(),
    )
    .unwrap();
    assert!(saved.id.starts_with("b-") && saved.hard_stop);
    assert_eq!(saved.warn_percent, DEFAULT_WARN_PERCENT);
    assert!(save(
        &store,
        v,
        input(None, "agent:codex", Metric::Tokens, 9.0),
        &codex_w,
        Utc::now()
    )
    .unwrap_err()
    .contains("already"));
    assert!(save(
        &store,
        v,
        input(Some("gone"), "global", Metric::Tokens, 9.0),
        &codex_w,
        Utc::now()
    )
    .is_err());
    let view = view(&store, v, &codex_w, Utc::now());
    assert_eq!(view.budgets.len(), 1);
    assert_eq!(view.budgets[0].enforceability, Enforceability::Enforced);
    assert!(!view.suggestions_available);
}

// PR 8 plan test 33.
#[test]
fn workflow_rename_moves_its_budget_and_delete_removes_it() {
    let dir = tmp();
    let v = dir.path();
    let seeded = crate::vault_workflows::seed_workflow(v, "Budget Rename Source", "manual", "BODY");
    let store = app_store(
        v,
        vec![budget(
            "b1",
            &format!("workflow:{}", seeded.slug),
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    let renamed = crate::vault_workflows::upsert(
        v,
        Some(&seeded.slug),
        "Budget Rename Target",
        "",
        "manual",
        None,
        None,
        None,
        "acceptEdits",
        &[],
        &[],
        None,
        "BODY",
    )
    .unwrap();
    assert_ne!(renamed.slug, seeded.slug);
    assert_eq!(
        loaded(&store).budgets[0].scope,
        Scope::Workflow(renamed.slug.clone())
    );
    assert!(
        crate::workflow_permissions::verify(v, &renamed.slug).is_ok(),
        "the rename re-stamp and the budget move co-exist"
    );
    crate::vault_workflows::delete(v, &renamed.slug).unwrap();
    assert!(loaded(&store).budgets.is_empty());
}

// Paperclip PR 5 obligation (refresh note §3.3): a pause opened inside a
// tracked run's funnel is a host write of `.review-queue.json`, never a
// flag; a direct write of the queue in that run still is.
#[test]
fn a_pause_opened_in_a_tracked_funnel_raises_no_flag_but_a_direct_queue_write_does() {
    use crate::automated_history::{finish_run, try_begin_run};
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "g",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    let handle = try_begin_run(v, "workflow", Some("brief")).unwrap();
    ledger(v, "r1", "workflow", Some("brief"), 1500);
    let effects = after_run(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now());
    assert!(effects
        .iter()
        .any(|e| matches!(e, BudgetEffect::Paused { .. })));
    let run = finish_run(v, handle, "success").unwrap();
    assert!(run.flags.is_empty(), "{:#?}", run.flags);

    let handle = try_begin_run(v, "workflow", Some("brief")).unwrap();
    fs::write(v.join(crate::review_queue::FILE_NAME), "{\"items\":[]}").unwrap();
    let run = finish_run(v, handle, "success").unwrap();
    assert_eq!(run.flags.len(), 1, "{:#?}", run.flags);
    assert_eq!(run.flags[0].kind, "stateFile");
    assert_eq!(run.flags[0].path, crate::review_queue::FILE_NAME);
}

// Paperclip PR 8b, refresh O3 and plan test 13(1): the kill path in the
// funnel order. A workflow run its guard stopped writes ONE
// `budget_stopped` partial row, its `after_run` opens the pause while the
// run is still tracked, its run-log row carries the exit marker, and it
// finishes with no flags; the next claim of the workflow is blocked.
#[test]
fn a_budget_killed_workflow_run_pauses_its_scope_and_finishes_with_no_flags() {
    use crate::automated_history::{finish_run, try_begin_run};
    let dir = tmp();
    let v = dir.path();
    let w = crate::vault_workflows::seed_workflow_with(v, "Brief", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    let store = app_store(
        v,
        vec![budget(
            "b1",
            &format!("workflow:{}", w.slug),
            Metric::Tokens,
            UsageWindow::Day,
            18_050.0,
        )],
    );
    let intent = RunIntent::workflow(&w.slug, CLAUDE);
    let Preflight::Clear(claimed) = preflight(&store, v, &intent, Utc::now()) else {
        panic!("nothing used yet");
    };
    let handle = try_begin_run(v, "workflow", Some(w.slug.as_str())).unwrap();
    // The run: a guard on the claim's budget stops it on msg-003's final.
    let mut ledger = crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(
            v,
            Surface::Workflow,
            Some(w.slug.clone()),
            Some(handle.id.clone()),
        ),
        CLAUDE,
        None,
    );
    ledger.mark_spawned();
    let mut guard = BudgetGuard::new(&claimed).expect("a token budget watches the run");
    guard.bind_run(ledger.key());
    let mut state = crate::agent::ParseState::default();
    let mut stop = None;
    for line in crate::usage::tests::CHAT_TOOL.lines().take(19) {
        for evt in crate::agent::parse_line(AgentKind::Claude, line, &mut state) {
            if ledger.observe(&evt) {
                if let Some(notice) = guard.observe(ledger.running_total()).stop {
                    stop = Some(notice);
                }
            }
        }
    }
    let notice = stop.expect("the guard stopped the run");
    ledger.finish(crate::usage::LedgerOutcome::BudgetStopped);
    let rows = crate::session_store::usage_rows_since(v, "").unwrap();
    assert_eq!(rows.len(), 1, "exactly one row");
    assert_eq!(rows[0].outcome, "budget_stopped");
    assert_eq!(rows[0].cost_status, "partial");
    // The funnel: after_run (tracked), the run log, then finish_run.
    let effects = after_run(&store, v, &intent, Utc::now());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, BudgetEffect::Paused { .. })),
        "{effects:?}"
    );
    crate::vault_workflows::finalize_run(
        v,
        &w.slug,
        "error",
        Some(notice.message.as_str()),
        1,
        None,
        crate::types::RunLogEntry {
            at: Utc::now().to_rfc3339(),
            status: "error".into(),
            duration_sec: 1,
            note: notice.message.clone(),
            run_id: Some(handle.id.clone()),
            error_tail: None,
            exit_kind: Some(crate::vault_workflows::EXIT_KIND_BUDGET.into()),
        },
    )
    .unwrap();
    let run = finish_run(v, handle, "error").unwrap();
    assert!(run.flags.is_empty(), "{:#?}", run.flags);
    // The next claim is blocked by the pause.
    assert!(preflight(&store, v, &intent, Utc::now())
        .blocked()
        .is_some());
    let log = crate::vault_workflows::read_full(v, &w.slug).unwrap().2;
    assert_eq!(
        log.last().and_then(|e| e.exit_kind.as_deref()),
        Some("budget")
    );
}

// Paperclip PR 8b, plan test 13(2): Claude's backstop exit in the funnel
// order. An enforced spending budget sets the backstop at the claim; the
// run ends with the recorded `error_max_budget_usd` shape; its row is
// `budget_stopped` but REPORTED from `modelUsage` (not partial); its
// `after_run` pauses the scope and the next claim is blocked.
#[cfg(unix)]
#[test]
fn a_backstop_exit_records_a_reported_row_pauses_its_scope_and_blocks_the_next_claim() {
    let dir = tmp();
    let v = dir.path();
    let w = crate::vault_workflows::seed_workflow_with(v, "Backstop", "manual", "BODY", |fm| {
        fm.agent = Some("claude".into());
    });
    let store = app_store(
        v,
        vec![budget(
            "c1",
            &format!("workflow:{}", w.slug),
            Metric::CostUsd,
            UsageWindow::Day,
            0.10,
        )],
    );
    let intent = RunIntent::workflow(&w.slug, CLAUDE);
    let Preflight::Clear(claimed) = preflight(&store, v, &intent, Utc::now()) else {
        panic!("nothing spent yet");
    };
    assert!(
        claimed.max_budget_usd.is_some(),
        "an enforced spending budget sets the backstop"
    );
    assert!(
        claimed.checks.is_empty(),
        "no token budget: no mid-run guard"
    );
    let meter =
        crate::usage_ledger::Meter::for_vault(v, Surface::Workflow, Some(w.slug.clone()), None)
            .with_budget(claimed);
    let mut run = crate::usage_ledger::UsageRun::begin(meter, CLAUDE, None);
    run.set_billing(crate::usage::Billing::Api);
    let fixture = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/fixtures/usage/claude-max-budget.jsonl"
    );
    let cmd =
        crate::process_group::test_support::isolated_shell("cat \"$F\"; exit 1", &[("F", fixture)]);
    let collected = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(crate::agent::collect_command(
            AgentKind::Claude,
            cmd,
            crate::agent::ParseState::default(),
            Some(std::time::Duration::from_secs(20)),
            None,
            run,
        ));
    let notice = collected
        .budget_stop
        .expect("the backstop exit is a budget stop");
    assert!(
        notice.message.starts_with(BUDGET_STOP_PREFIX),
        "{}",
        notice.message
    );
    let rows = crate::session_store::usage_rows_since(v, "").unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "budget_stopped");
    assert_eq!(rows[0].cost_status, "reported");
    assert!(rows[0].cost_usd.unwrap() > 0.10);
    let effects = after_run(&store, v, &intent, Utc::now());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, BudgetEffect::Paused { .. })),
        "{effects:?}"
    );
    assert!(
        preflight(&store, v, &intent, Utc::now())
            .blocked()
            .is_some(),
        "the next claim is blocked"
    );
}

// Paperclip PR 8b: a warning a chat turn's guard delivered mid-turn is
// recorded under its conversation key, so the next turn's preflight
// does not deliver it again; without the record it would.
#[test]
fn a_mid_turn_chat_warning_is_not_delivered_again_by_the_next_turn() {
    let chat = RunIntent::chat(CLAUDE);
    let turn = |record: bool| {
        let dir = tmp();
        let v = dir.path().to_path_buf();
        let store = app_store(
            &v,
            vec![budget(
                "t",
                "global",
                Metric::Tokens,
                UsageWindow::Day,
                1000.0,
            )],
        );
        ledger(&v, "r1", "chat", Some("s-1"), 100);
        let Preflight::Clear(claimed) = preflight(&store, &v, &chat, Utc::now()) else {
            panic!("under the warn level");
        };
        let mut guard =
            BudgetGuard::for_agent(&claimed, AgentKind::Claude).expect("a token budget");
        let warned = guard.observe(counts(750, 0, 0)).warnings;
        assert_eq!(warned.len(), 1, "{warned:?}");
        assert_eq!(warned[0].level, NoticeLevel::Warn);
        if record {
            record_conversation_notices(&store, &v, &warned, Utc::now());
        }
        // The turn's own row now puts the day at the warn level.
        ledger(&v, "r2", "chat", Some("s-1"), 750);
        let next = match preflight(&store, &v, &chat, Utc::now()) {
            Preflight::Clear(_) => Vec::new(),
            Preflight::Warn(_, notices) => notices,
            Preflight::Blocked(notice) => panic!("chat is never blocked: {notice:?}"),
        };
        (dir, next)
    };
    let (_recorded_dir, recorded) = turn(true);
    assert!(
        recorded.is_empty(),
        "delivered mid-turn, not again: {recorded:?}"
    );
    let (_control_dir, unrecorded) = turn(false);
    assert_eq!(
        unrecorded.len(),
        1,
        "without the record the next turn delivers it"
    );
}

#[test]
fn a_duplicate_item_id_is_refused_rather_than_acted_on() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    assert!(
        preflight(&store, v, &RunIntent::heartbeat(CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    let item = pause_items(v).remove(0);
    // Plant a second queue item with the same id (the queue does not
    // enforce unique ids and the file is agent-writable).
    let path = v.join(crate::review_queue::FILE_NAME);
    let mut raw: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut forged = raw["items"][0].clone();
    forged["fingerprint"] = json!("budget-pause:other:1");
    raw["items"].as_array_mut().unwrap().push(forged);
    fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
    let err = accept_pause(&store, v, &item, Some(&with_limit(5000.0)), Utc::now()).unwrap_err();
    assert_eq!(err, DUPLICATE_ITEM_MESSAGE);
    assert!(hold_pause_and_reject(&store, v, &item).is_err());
    assert!(loaded(&store).pauses.contains_key("b1"));
    assert_eq!(loaded(&store).budgets[0].limit, 1000.0);
}

#[test]
fn a_card_is_matched_by_budgets_json_never_by_its_payload() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![
            budget(
                "b1",
                "workflow:brief",
                Metric::Tokens,
                UsageWindow::Day,
                1000.0,
            ),
            budget(
                "b2",
                "global",
                Metric::Tokens,
                UsageWindow::Day,
                1_000_000.0,
            ),
        ],
    );
    ledger(v, "r1", "workflow", Some("brief"), 1200);
    assert!(
        preflight(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now())
            .blocked()
            .is_some()
    );
    let mut item = pause_items(v).remove(0);
    // An edited payload naming another budget changes nothing.
    item.payload["budgetId"] = json!("b2");
    accept_pause(
        &store,
        v,
        &item,
        Some(&json!({ "budgetId": "b2", "newLimit": 5000.0 })),
        Utc::now(),
    )
    .unwrap();
    let file = loaded(&store);
    assert_eq!(file.budgets[0].limit, 5000.0, "b1 was raised");
    assert_eq!(file.budgets[1].limit, 1_000_000.0, "b2 untouched");
    // An item whose fingerprint names another episode is not this pause's.
    let mut foreign = item.clone();
    foreign.id = "review-forged".into();
    assert_eq!(pause_for_item(&file, &foreign), None);
}

#[test]
fn skip_details_read_as_one_short_sentence() {
    let def = budget(
        "b1",
        "workflow:brief",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    );
    let bounds = window_bounds_in(UsageWindow::Day, &at(0, 2026, 9, 25, 12, 0));
    let notice = limit_notice(&def, &bounds, 1200.0, false, true);
    assert_eq!(
        skip_detail(&notice),
        "the “brief” automation, 1,200 tokens of 1,000 tokens this day"
    );
    let unavailable = load_failure_notice(&RunIntent::workflow("brief", None), "x").unwrap();
    assert!(skip_detail(&unavailable).contains("could not be read"));
}

#[test]
fn intents_take_the_stamped_agent_or_the_default() {
    assert_eq!(
        intent_agent_with(Some("codex"), CLAUDE),
        Some(AgentKind::Codex)
    );
    assert_eq!(intent_agent_with(Some("  "), CLAUDE), CLAUDE);
    assert_eq!(intent_agent_with(None, None), None);
}

// ── Mid-run guard (Paperclip PR 8b) ─────────────────────────────────

fn counts(input: u64, cached: u64, output: u64) -> Option<crate::usage::TokenCounts> {
    Some(crate::usage::TokenCounts {
        input,
        cached_input: cached,
        output,
    })
}

fn eval_budget(file: &BudgetsFile, intent: &RunIntent, observed: u64) -> RunBudget {
    let now = at(0, 2026, 9, 25, 12, 0);
    let eval = evaluate(file, intent, &tokens(observed), &now);
    RunBudget {
        intent: intent.clone(),
        seed_levels: eval.seed_levels.clone(),
        max_budget_usd: eval.max_budget_usd,
        spend_grant: None,
        file_hash: None,
        checks: guard_checks(file, &eval, &|_, _| BTreeMap::new()),
        claimed_limits: claimed_limits(&eval),
        vault: None,
        fallback: None,
    }
}

// PR 8 plan test 6: an enforced run is stopped once, on the crossing,
// counting input (cache creation included) plus output, never cache reads.
#[test]
fn the_guard_stops_an_enforced_run_once_when_it_crosses_the_limit() {
    let file = file_with(vec![budget(
        "t",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let workflow = RunIntent::workflow("brief", CLAUDE);
    let run = eval_budget(&file, &workflow, 600);
    assert!(run.usage_stream());
    assert_eq!(run.checks.len(), 1);
    assert_eq!(run.checks[0].baseline, 600.0);
    assert!(run.checks[0].enforced);
    let mut guard = BudgetGuard::new(&run).expect("a token budget watches the run");
    // Cache reads never count: 600 + 100 + 10 is under the limit.
    assert_eq!(
        guard.observe(counts(100, 50_000, 10)),
        GuardOutcome::default()
    );
    assert_eq!(guard.observe(None), GuardOutcome::default());
    let crossed = guard.observe(counts(300, 0, 100));
    let stop = crossed.stop.expect("stopped at 1,000");
    assert!(
        stop.message.starts_with(BUDGET_STOP_PREFIX),
        "{}",
        stop.message
    );
    assert!(
        stop.message.contains("1,000 tokens of 1,000 tokens"),
        "{}",
        stop.message
    );
    assert_eq!(stop.observed, Some(1000.0));
    assert!(stop.blocking);
    assert_eq!(guard.stop_notice(), Some(&stop));
    // Once: later growth reports nothing more.
    assert_eq!(guard.observe(counts(900, 0, 900)), GuardOutcome::default());
}

// PR 8 plan tests 7-8: a warn-only run (chat, a soft budget) is never
// stopped; each level warns once, and a level the baseline already met
// (seeded at the claim) never warns again.
#[test]
fn the_guard_only_warns_warn_only_runs_and_each_level_once() {
    let file = file_with(vec![budget(
        "t",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let chat = RunIntent::chat(CLAUDE);
    let run = eval_budget(&file, &chat, 100);
    assert!(!run.checks[0].enforced, "chat only warns");
    let mut guard = BudgetGuard::new(&run).unwrap();
    let warned = guard.observe(counts(750, 0, 0));
    assert_eq!(warned.stop, None);
    assert_eq!(warned.warnings.len(), 1);
    assert_eq!(warned.warnings[0].level, NoticeLevel::Warn);
    assert_eq!(
        guard.observe(counts(760, 0, 0)).warnings.len(),
        0,
        "warn once"
    );
    let limit = guard.observe(counts(2_000, 0, 0));
    assert_eq!(limit.stop, None, "chat is never stopped");
    assert_eq!(limit.warnings.len(), 1);
    assert_eq!(limit.warnings[0].level, NoticeLevel::Limit);
    assert!(guard.observe(counts(5_000, 0, 0)).warnings.is_empty());

    // Seeded: the baseline already met the warn level at the claim.
    let seeded = eval_budget(&file, &chat, 850);
    assert!(seeded.seed_levels.contains("t:warn"));
    let mut guard = BudgetGuard::new(&seeded).unwrap();
    assert!(guard.observe(counts(10, 0, 0)).warnings.is_empty());
    assert_eq!(
        guard.observe(counts(200, 0, 0)).warnings[0].level,
        NoticeLevel::Limit
    );

    // A soft budget never stops even an automation.
    let mut soft = budget("s", "global", Metric::Tokens, UsageWindow::Day, 1000.0);
    soft.hard_stop = false;
    let run = eval_budget(
        &file_with(vec![soft]),
        &RunIntent::workflow("brief", CLAUDE),
        0,
    );
    assert!(!run.checks[0].enforced);
    let mut guard = BudgetGuard::new(&run).unwrap();
    let over = guard.observe(counts(1_500, 0, 0));
    assert_eq!(over.stop, None);
    assert_eq!(over.warnings[0].level, NoticeLevel::Limit);
}

// PR 8 plan test 9: spending budgets are the backstop's, not the
// guard's; no token budget means no guard and no partial messages.
#[test]
fn only_token_budgets_arm_the_guard() {
    let file = file_with(vec![budget(
        "c",
        "global",
        Metric::CostUsd,
        UsageWindow::Day,
        5.0,
    )]);
    let run = eval_budget(&file, &RunIntent::workflow("brief", CLAUDE), 0);
    assert!(run.checks.is_empty());
    assert!(!run.usage_stream());
    assert!(BudgetGuard::new(&run).is_none());
    assert!(BudgetGuard::new(&RunBudget::none(RunIntent::chat(CLAUDE))).is_none());
}

// PR 8 plan test 10: a delegate under an automation is enforced, one
// under chat only warns (the parent comes from team-mcp's environment).
#[test]
fn a_delegates_guard_follows_its_parent() {
    let file = file_with(vec![budget(
        "t",
        "global",
        Metric::Tokens,
        UsageWindow::Day,
        1000.0,
    )]);
    let parent = |surface| ParentRun {
        surface: Some(surface),
        scope: Some("brief".into()),
    };
    let under_workflow = RunIntent::delegate(CLAUDE, parent(Surface::Workflow));
    let mut guard = BudgetGuard::new(&eval_budget(&file, &under_workflow, 0)).unwrap();
    assert!(guard.observe(counts(1_000, 0, 1)).stop.is_some());
    let under_chat = RunIntent::delegate(CLAUDE, parent(Surface::Chat));
    let mut guard = BudgetGuard::new(&eval_budget(&file, &under_chat, 0)).unwrap();
    let outcome = guard.observe(counts(1_000, 0, 1));
    assert_eq!(outcome.stop, None);
    assert_eq!(outcome.warnings.len(), 1);
}

// A stop notice is safe in a run-log note whatever the workflow slug.
#[test]
fn stop_notices_keep_the_run_log_note_convention() {
    let def = budget(
        "t",
        "workflow:a<b>—c",
        Metric::Tokens,
        UsageWindow::Day,
        10.0,
    );
    let bounds = window_bounds_in(UsageWindow::Day, &at(0, 2026, 9, 25, 12, 0));
    let notice = stop_notice(&def, &bounds, 12.0);
    assert!(
        !notice.message.contains('—')
            && !notice.message.contains('<')
            && !notice.message.contains('>')
    );
    assert!(notice.message.starts_with(BUDGET_STOP_PREFIX));
    let backstop = backstop_notice(AgentKind::Claude);
    assert_eq!(
        backstop.message,
        "Budget limit reached: Claude stopped at this run's spending backstop."
    );
}

// The claim's RunBudget carries the guard's checks (real preflight).
#[test]
fn preflight_hands_the_run_its_token_checks() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "b1",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "r1", "workflow", Some("brief"), 300);
    let intent = RunIntent::workflow("brief", CLAUDE);
    let Preflight::Clear(run) = preflight(&store, v, &intent, Utc::now()) else {
        panic!("under the limit");
    };
    assert_eq!(run.checks.len(), 1);
    assert_eq!(run.checks[0].baseline, 300.0);
    assert!(run.checks[0].enforced);
    let Preflight::Clear(read_only) = preflight_readonly(&store, Some(v), &intent, Utc::now())
    else {
        panic!("under the limit");
    };
    assert_eq!(read_only.checks, run.checks);
}

/// One live workflow run of "brief" on Claude, as the reservation holds it.
fn live_brief(tokens: u64) -> InFlightRun {
    InFlightRun {
        surface: "workflow".into(),
        scope: Some("brief".into()),
        parent_surface: None,
        agent: Some("claude".into()),
        tokens,
        gpu_time: false,
    }
}

// Merged plan slice 3 (review A·R6, decision 6): concurrent runs share
// one in-flight reservation, so two runs that each claimed the same
// remaining budget cannot both spend it. Before it each guard saw only
// its own run (400 + 350 and 400 + 300, both under 1,000), so N runs
// could overrun a hard stop about N times.
#[test]
fn concurrent_runs_share_one_reservation_and_cannot_both_spend_what_is_left() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "g",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(v, "earlier", "chat", Some("s-1"), 400);
    let intent = RunIntent::workflow("brief", CLAUDE);
    let claim = || match preflight(&store, v, &intent, Utc::now()) {
        Preflight::Clear(budget) => budget,
        other => panic!("600 tokens are left: {other:?}"),
    };
    let (a, b) = (claim(), claim());
    assert_eq!((a.checks[0].baseline, b.checks[0].baseline), (400.0, 400.0));
    let mut guard_a = BudgetGuard::new(&a).expect("a token budget");
    let mut guard_b = BudgetGuard::new(&b).expect("a token budget");
    guard_a.bind_run("run-a");
    guard_b.bind_run("run-b");

    reserve(v, "run-a", live_brief(350));
    assert!(
        guard_a.observe(counts(350, 0, 0)).stop.is_none(),
        "750 of 1,000"
    );
    reserve(v, "run-b", live_brief(300));
    let stopped = guard_b
        .observe(counts(300, 0, 0))
        .stop
        .expect("B counts A's 350: 1,050 of 1,000");
    assert_eq!(stopped.observed, Some(1050.0));
    // A, the next time it grows, counts B's tokens too.
    reserve(v, "run-a", live_brief(360));
    let stopped = guard_a
        .observe(counts(360, 0, 0))
        .stop
        .expect("A counts B's 300: 1,060 of 1,000");
    assert_eq!(stopped.observed, Some(1060.0));
    release(v, "run-a", Some(360));
    release(v, "run-b", Some(300));
}

// Merged plan slice 3 (review A·R6) in dollars. A stream reports no
// cost, so a spending ceiling is held at the claim: each live Claude
// claim holds the `--max-budget-usd` backstop it was given, and a
// concurrent claim gets only what is left. Before it every claim got the
// whole remaining $0.60, so two runs could spend $1.20 of it.
#[test]
fn concurrent_claims_share_one_spending_ceiling() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "c",
            "workflow:brief",
            Metric::CostUsd,
            UsageWindow::Day,
            1.0,
        )],
    );
    let mut row = crate::session_store::tests::usage_row_fixture(
        "earlier",
        &crate::session_store::db_time(Utc::now()),
        "success",
    );
    row.surface = "workflow".into();
    row.scope = Some("brief".into());
    row.cost_usd = Some(0.40);
    row.cost_status = "reported".into();
    crate::session_store::upsert_usage(v, &row).unwrap();
    let claim = |agent: Option<AgentKind>| match preflight(
        &store,
        v,
        &RunIntent::workflow("brief", agent),
        Utc::now(),
    ) {
        Preflight::Clear(budget) => budget,
        other => panic!("60 cents are left: {other:?}"),
    };
    let near = |usd: Option<f64>, want: f64| usd.is_some_and(|usd| (usd - want).abs() < 1e-9);

    let a = claim(CLAUDE);
    assert!(near(a.max_budget_usd, 0.60), "{:?}", a.max_budget_usd);
    let b = claim(CLAUDE);
    assert!(
        near(b.max_budget_usd, 0.01),
        "A holds the 60 cents: B gets the one-cent floor, not $0.60 again: {:?}",
        b.max_budget_usd
    );
    assert!(a.spend_grant.is_some(), "a Claude claim holds its backstop");
    // A team-mcp style read-only claim in this process sees the holds too.
    let Preflight::Clear(c) = preflight_readonly(
        &store,
        Some(v),
        &RunIntent::workflow("brief", CLAUDE),
        Utc::now(),
    ) else {
        panic!("the read-only claim runs");
    };
    assert!(near(c.max_budget_usd, 0.01), "{:?}", c.max_budget_usd);
    drop((b, c));
    // Codex gets no backstop, so its claim holds nothing.
    let codex = claim(Some(AgentKind::Codex));
    assert!(codex.spend_grant.is_none());
    assert_eq!(granted_usd(v).len(), 1, "only A still holds dollars");
    // A's run is over: its hold goes with its budget.
    let held = a.clone();
    drop(a);
    assert_eq!(granted_usd(v).len(), 1, "a clone still holds it");
    drop(held);
    assert!(granted_usd(v).is_empty());
    let d = claim(CLAUDE);
    assert!(near(d.max_budget_usd, 0.60), "{:?}", d.max_budget_usd);
}

// Wave 2 review nit 6 (closed as unreachable): only enforced claims hold
// dollars, and a chat is not enforced, so a Claude chat under the same
// agent budget takes no backstop and no hold. That is one of the three
// reasons a second live dollar hold can't exist today, so halving the
// first claim's backstop would only cut a lone automation's dollars. If
// chats ever become enforced, this fails and the split needs revisiting.
#[test]
fn a_claude_chat_claim_holds_no_dollars() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "c",
            "agent:claude",
            Metric::CostUsd,
            UsageWindow::Day,
            1.0,
        )],
    );
    let Preflight::Clear(chat) = preflight(&store, v, &RunIntent::chat(CLAUDE), Utc::now()) else {
        panic!("a chat only warns");
    };
    assert_eq!(chat.max_budget_usd, None, "a chat gets no backstop");
    assert!(chat.spend_grant.is_none(), "a chat holds no dollars");
    assert!(granted_usd(v).is_empty());
    // The automation beside it still gets the whole budget.
    let Preflight::Clear(run) =
        preflight(&store, v, &RunIntent::workflow("brief", CLAUDE), Utc::now())
    else {
        panic!("nothing is spent yet");
    };
    assert_eq!(run.max_budget_usd, Some(1.0));
    assert!(run.spend_grant.is_some());
    assert_eq!(granted_usd(v), vec![1.0]);
}

// Merged plan slice 3: across parent and delegates. A team-mcp delegate
// runs in another process, so its usage reaches the parent's guard
// through the ledger's checkpoints (reread at most every
// RESERVATION_REFRESH, and on the first check), and the delegate's own
// guard sees the parent's checkpoints the same way.
#[test]
fn a_parent_and_its_delegate_see_each_others_checkpointed_usage() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "w",
            "workflow:brief",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    let checkpoint = |key: &str, surface: &str, parent: Option<&str>, tokens: i64| {
        let mut row = crate::session_store::tests::usage_row_fixture(
            key,
            &crate::session_store::db_time(Utc::now()),
            "running",
        );
        row.surface = surface.into();
        row.parent_surface = parent.map(str::to_string);
        row.scope = Some("brief".into());
        row.input_tokens = Some(tokens);
        row.output_tokens = Some(0);
        row.cost_status = "partial".into();
        crate::session_store::upsert_usage(v, &row).unwrap();
    };
    let intent = RunIntent::workflow("brief", CLAUDE);
    let Preflight::Clear(parent_claim) = preflight(&store, v, &intent, Utc::now()) else {
        panic!("nothing used yet");
    };
    let mut parent = BudgetGuard::new(&parent_claim).expect("a token budget");
    parent.bind_run("parent");
    // The delegate's checkpoint, written by team-mcp after the claim.
    checkpoint("delegate-1", "delegate", Some("workflow"), 700);
    let stopped = parent
        .observe(counts(400, 0, 0))
        .stop
        .expect("the delegate's 700 + the parent's 400 of 1,000");
    assert_eq!(stopped.observed, Some(1100.0));

    // The other way round: a delegate's read-only claim, then the
    // parent's checkpoint.
    let delegate = RunIntent::delegate(
        CLAUDE,
        ParentRun {
            surface: Some(Surface::Workflow),
            scope: Some("brief".into()),
        },
    );
    let Preflight::Clear(delegate_claim) =
        preflight_readonly(&store, Some(v), &delegate, Utc::now())
    else {
        panic!("700 of 1,000");
    };
    assert_eq!(delegate_claim.checks[0].baseline, 700.0);
    let mut teammate = BudgetGuard::new(&delegate_claim).expect("a token budget");
    teammate.bind_run("delegate-2");
    checkpoint("parent", "workflow", None, 250);
    let stopped = teammate
        .observe(counts(100, 0, 0))
        .stop
        .expect("700 + the parent's 250 + the delegate's 100 of 1,000");
    assert_eq!(stopped.observed, Some(1050.0));
}

// Merged plan slice 3: every usage row publishes its running total to
// the reservation as it grows, and its final row releases it, kept only
// while a guard that started earlier may still need it.
#[test]
fn a_usage_run_reserves_its_running_total_and_releases_it_after_its_final_row() {
    let dir = tmp();
    let v = dir.path();
    let mut run = crate::usage_ledger::UsageRun::begin(
        crate::usage_ledger::Meter::for_vault(v, Surface::Workflow, Some("brief".into()), None),
        CLAUDE,
        None,
    );
    run.mark_spawned();
    let mut state = crate::agent::ParseState::default();
    for line in crate::usage::tests::CHAT_TOOL.lines().take(19) {
        for evt in crate::agent::parse_line(AgentKind::Claude, line, &mut state) {
            run.observe(&evt);
        }
    }
    let running = run.running_total().expect("streamed usage");
    let key = run.key().to_string();
    let reserved = reserved_runs(v);
    let live = reserved.get(&key).expect("reserved while it runs");
    assert_eq!(live.tokens, running.input + running.output);
    assert_eq!(
        (
            live.surface.as_str(),
            live.scope.as_deref(),
            live.agent.as_deref()
        ),
        ("workflow", Some("brief"), Some("claude"))
    );
    let watching = RunBudget {
        vault: Some(v.to_path_buf()),
        ..token_budget_for_test(RunIntent::workflow("brief", CLAUDE), 1e9, 0.0, true)
    };
    let guard = BudgetGuard::new(&watching).expect("a token budget");
    let _ = run.finish(crate::usage::LedgerOutcome::Success);
    assert!(
        reserved_runs(v).contains_key(&key),
        "kept while an earlier guard may need it"
    );
    drop(guard);
    assert!(reserved_runs(v).is_empty(), "then released");
}

// Merged plan slice 3, decision 6: at its own `agent:` ceiling an
// enforced run moves to the first candidate with room; the verdict names
// the switch, and the switch lands on the run's receipt.
#[test]
fn an_agent_ceiling_moves_the_run_to_an_agent_with_room_and_onto_its_receipt() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![budget(
            "ac",
            "agent:claude",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    // The fixture row is Claude's.
    ledger(v, "r1", "chat", Some("s-1"), 1200);
    let intent = RunIntent::workflow("brief", CLAUDE);
    let candidates = || vec![AgentKind::Claude, AgentKind::Codex];
    let Preflight::Clear(budget) =
        preflight_with_fallback(&store, v, &intent, &candidates, Utc::now())
    else {
        panic!("Codex has room");
    };
    let fallback = budget.fallback.clone().expect("the run moved");
    assert_eq!(
        (fallback.from, fallback.to),
        (AgentKind::Claude, AgentKind::Codex)
    );
    assert_eq!(budget.intent.agent, Some(AgentKind::Codex));
    assert!(
        fallback.reason.starts_with(BUDGET_PAUSED_PREFIX),
        "{}",
        fallback.reason
    );
    assert_eq!(pause_items(v).len(), 1, "Claude's own ceiling still holds");

    let mut collector =
        crate::evidence::EvidenceCollector::new("workflow", Some("brief"), "run-moved");
    collector.record_switch(fallback.receipt_switch());
    let receipt = collector
        .finish(v, "success", Vec::new())
        .expect("the receipt");
    assert_eq!(
        receipt.switches,
        vec![crate::evidence::ReceiptSwitch {
            kind: crate::evidence::SWITCH_AGENT.into(),
            from: "Claude".into(),
            to: "Codex".into(),
            reason: fallback.reason.clone(),
        }]
    );
}

// Decision 6: with no agent that has room the ceiling still stops the
// run, and says so. A candidate at its own ceiling is skipped, and so is
// one a hard-stop budget over it could not measure. A ceiling every agent shares never asks for candidates.
#[test]
fn with_no_agent_that_has_room_the_ceiling_stops_the_run_and_says_so() {
    let dir = tmp();
    let v = dir.path();
    let store = app_store(
        v,
        vec![
            budget(
                "ac",
                "agent:claude",
                Metric::Tokens,
                UsageWindow::Day,
                1000.0,
            ),
            budget(
                "ax",
                "agent:codex",
                Metric::Tokens,
                UsageWindow::Day,
                1000.0,
            ),
            budget("g", "global", Metric::Tokens, UsageWindow::Day, 100_000.0),
        ],
    );
    ledger(v, "claude-1", "chat", Some("s-1"), 1200);
    let mut codex = crate::session_store::tests::usage_row_fixture(
        "codex-1",
        &crate::session_store::db_time(Utc::now()),
        "success",
    );
    codex.agent = Some("codex".into());
    codex.input_tokens = Some(1500);
    codex.output_tokens = Some(0);
    codex.cost_status = "unpriced".into();
    crate::session_store::upsert_usage(v, &codex).unwrap();
    let intent = RunIntent::workflow("brief", CLAUDE);
    let candidates = || vec![AgentKind::Codex];
    let Preflight::Blocked(notice) =
        preflight_with_fallback(&store, v, &intent, &candidates, Utc::now())
    else {
        panic!("nobody has room");
    };
    assert_eq!(notice.scope.as_deref(), Some("agent:claude"));
    assert!(
        notice.message.starts_with(BUDGET_PAUSED_PREFIX),
        "{}",
        notice.message
    );
    assert!(notice.message.ends_with(NO_ROOM_NOTE), "{}", notice.message);

    let shared_dir = tmp();
    let shared = shared_dir.path();
    let store = app_store(
        shared,
        vec![budget(
            "g",
            "global",
            Metric::Tokens,
            UsageWindow::Day,
            1000.0,
        )],
    );
    ledger(shared, "r1", "chat", Some("s-1"), 1200);
    let never = || -> Vec<AgentKind> { unreachable!("a global ceiling holds every agent") };
    let Preflight::Blocked(notice) =
        preflight_with_fallback(&store, shared, &intent, &never, Utc::now())
    else {
        panic!("over the global limit");
    };
    assert_eq!(notice.scope.as_deref(), Some("global"));
    assert!(!notice.message.contains(NO_ROOM_NOTE));
}
