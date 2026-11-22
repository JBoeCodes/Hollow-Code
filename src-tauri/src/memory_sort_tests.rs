//! PX6 tests: every rule of the auto-memory sort, on synthetic text only.
//! Nothing here reads a real folder.

use super::*;

const PATH: &str = "projects/-Users-someone-code-app/memory/entry.md";

fn today() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 19).unwrap()
}

fn sort(text: &str) -> MemorySort {
    classify(PATH, Some(text), None, today())
}

fn check(text: &str, layer: SortLayer, rule: &str) -> MemorySort {
    let got = sort(text);
    assert_eq!(got.layer, layer, "{text:?}: {}", got.reason);
    assert_eq!(got.rule, rule, "{text:?}: {}", got.reason);
    assert!(
        got.reason.contains(rule),
        "the reason names its rule: {}",
        got.reason
    );
    got
}

fn fm(fields: &str, body: &str) -> String {
    format!("---\n{fields}\n---\n\n{body}\n")
}

#[test]
fn only_auto_memory_paths_have_a_slug() {
    assert_eq!(
        auto_memory_slug("projects/-Users-x-app/memory/a.md"),
        Some("-Users-x-app")
    );
    assert_eq!(auto_memory_slug("projects/x/memory/MEMORY.md"), Some("x"));
    assert_eq!(auto_memory_slug("projects/x/memory/sub/a.MD"), Some("x"));
    for no in [
        "projects/x/a.md",
        "memory/a.md",
        "projects//memory/a.md",
        "projects/x/memory/a.txt",
        "projects/x/memory/",
        "projects/x/memory",
        "other/x/memory/a.md",
        "CLAUDE.md",
    ] {
        assert_eq!(auto_memory_slug(no), None, "{no}");
    }
}

#[test]
fn an_index_file_is_stale_whatever_it_says() {
    for path in ["projects/x/memory/MEMORY.md", "projects/x/memory/memory.md"] {
        let got = classify(
            path,
            Some("The mac mini runs the gates on src/ files."),
            None,
            today(),
        );
        assert_eq!(got.layer, SortLayer::Stale);
        assert_eq!(got.rule, "index");
        assert!(got.reason.contains("index"));
    }
    let unread = classify("projects/x/memory/MEMORY.md", None, None, today());
    assert_eq!(unread.rule, "index");
}

#[test]
fn an_unread_entry_stays_personal() {
    let got = classify(PATH, None, None, today());
    assert_eq!(got.layer, SortLayer::Personal);
    assert_eq!(got.rule, "unread");
}

#[test]
fn an_entry_with_almost_no_words_is_empty() {
    check("", SortLayer::Stale, "empty");
    check("   \n\n", SortLayer::Stale, "empty");
    check(
        &fm("name: x\ntype: user", "# A heading only"),
        SortLayer::Stale,
        "empty",
    );
    check("Use tabs.", SortLayer::Stale, "empty");
    // A description counts as words.
    let described = fm(
        "name: x\ndescription: Answers stay short and plain for every reader\ntype: user",
        "",
    );
    assert_ne!(sort(&described).layer, SortLayer::Stale);
}

#[test]
fn a_superseded_entry_is_stale() {
    let body = "The old rule about worktrees and their target folders, kept for the record.";
    let status = check(
        &fm("status: superseded", body),
        SortLayer::Stale,
        "superseded",
    );
    assert!(status.reason.contains("status field says superseded"));
    check(
        &fm("status: Archived", body),
        SortLayer::Stale,
        "superseded",
    );
    check(
        &fm("superseded_by: newer-entry.md", body),
        SortLayer::Stale,
        "superseded",
    );
    check(
        &fm("superseded-by: newer-entry.md", body),
        SortLayer::Stale,
        "superseded",
    );
    let phrase = check(
        "Use port 1420 for the preview window. No longer true since the move.",
        SortLayer::Stale,
        "superseded",
    );
    assert!(phrase.reason.contains("\"no longer true\""));
    // Another status is not stale.
    assert_ne!(sort(&fm("status: active", body)).layer, SortLayer::Stale);
}

#[test]
fn a_dated_old_entry_about_a_passing_state_is_stale() {
    let old = check(
        "On 2026-08-10 the build box is currently offline for repairs.",
        SortLayer::Stale,
        "dated-old",
    );
    assert!(old.reason.contains("2026-08-10"), "{}", old.reason);
    assert!(old.reason.contains("101 days"), "{}", old.reason);
    assert!(old.reason.contains("\"currently\""), "{}", old.reason);
    // Old but lasting: no passing state.
    assert_ne!(
        sort("On 2026-08-10 we learned to keep each build output apart, always.").layer,
        SortLayer::Stale
    );
    // A passing state, but recent.
    assert_ne!(
        sort("On 2026-11-10 the build box is currently offline for repairs.").layer,
        SortLayer::Stale
    );
    // The newest date counts.
    assert_ne!(
        sort("Since 2026-08-01, rechecked 2026-11-15: the build box is currently offline.").layer,
        SortLayer::Stale
    );
    // Exactly the limit is not over it.
    let edge = today() - chrono::Duration::days(STALE_DAYS);
    assert_ne!(
        sort(&format!(
            "On {edge} the build box is currently offline for repairs."
        ))
        .layer,
        SortLayer::Stale
    );
    // No date in the text: the file's modified time.
    let july = NaiveDate::from_ymd_opt(2026, 7, 1)
        .unwrap()
        .and_hms_opt(12, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let got = classify(
        PATH,
        Some("The build box is currently offline for repairs, ask first."),
        Some(july),
        today(),
    );
    assert_eq!(got.layer, SortLayer::Stale);
    assert_eq!(got.rule, "dated-old");
    let fresh = classify(
        PATH,
        Some("The build box is currently offline for repairs, ask first."),
        Some(chrono::Utc::now().timestamp()),
        today(),
    );
    assert_ne!(fresh.layer, SortLayer::Stale);
}

#[test]
fn home_rules_name_machines_paths_tools_and_disks() {
    check(
        "Keep build caches under ~/Library/Caches/build-x/bin and nowhere else.",
        SortLayer::Home,
        "home-path",
    );
    check(
        "Logs land in the Application Support folder, beside the settings.",
        SortLayer::Home,
        "home-folder",
    );
    check(
        "The mac mini answers over the tailnet only; use its LAN address otherwise.",
        SortLayer::Home,
        "named-machine",
    );
    check(
        "Install tools with Homebrew and keep launchd jobs small and few.",
        SortLayer::Home,
        "machine-tool",
    );
    check(
        "The external SSD holds the runner data; check free space first.",
        SortLayer::Home,
        "disk-runner",
    );
}

#[test]
fn personal_rules_name_the_persons_preferences_and_type() {
    check(
        "He prefers short answers and dislikes long tables in replies.",
        SortLayer::Personal,
        "preference",
    );
    for kind in ["user", "feedback", "\"feedback\""] {
        let got = check(
            &fm(
                &format!("name: x\ntype: {kind}"),
                "Writes in plain sentences and reads everything on paper.",
            ),
            SortLayer::Personal,
            "memory-type",
        );
        assert!(got.reason.contains("has type"), "{}", got.reason);
    }
}

#[test]
fn project_rules_name_a_codebase() {
    check(
        "Shared bindings are generated by ts_bindings.rs into ipc.ts output.",
        SortLayer::Project,
        "code-path",
    );
    check(
        "Keep new screens in src/components and helpers in scripts/ beside them.",
        SortLayer::Project,
        "code-folder",
    );
    check(
        "Open a pull request from a fresh branch and rebase before the merge.",
        SortLayer::Project,
        "pr-merge",
    );
    check(
        "Run clippy and vitest through the gate script, never directly.",
        SortLayer::Project,
        "gate-test",
    );
    check(
        &fm(
            "name: x\ntype: project",
            "Release notes are written by hand for each version.",
        ),
        SortLayer::Project,
        "memory-type",
    );
    // A reference type adds nothing.
    let reference = fm(
        "name: x\ntype: reference",
        "Release notes are written by hand for each version.",
    );
    assert_eq!(sort(&reference).rule, "default");
}

#[test]
fn the_most_signs_win_and_a_tie_takes_the_four_question_order() {
    // Project outnumbers Home.
    let got = sort("On the mini: run cargo test, clippy and vitest in the gate before each merge.");
    assert_eq!(got.layer, SortLayer::Project, "{}", got.reason);
    assert!(got.reason.contains("Home had"), "{}", got.reason);
    // Home 1, Project 1: Home.
    let tie = check(
        "The runner keeps a fixture copy for every new checkout here.",
        SortLayer::Home,
        "disk-runner",
    );
    assert!(tie.reason.contains("Project had 1"), "{}", tie.reason);
    // Personal 1, Project 1: Personal.
    check(
        "She prefers that every fixture stays tiny and readable.",
        SortLayer::Personal,
        "preference",
    );
    // One rule's words are capped.
    let capped =
        sort("pr prs pull request merge merged branch rebase main worktree commit, on the ssd.");
    assert_eq!(capped.layer, SortLayer::Project, "{}", capped.reason);
    assert!(
        capped.reason.contains("(3 signs; Home had 1)"),
        "{}",
        capped.reason
    );
}

#[test]
fn no_sign_at_all_is_personal_by_default() {
    check(
        "Water the plants on the balcony every second morning, please.",
        SortLayer::Personal,
        "default",
    );
}

#[test]
fn words_match_whole_words_only() {
    assert!(!has_word("a decision", "ci"));
    assert!(!has_word("we prefer it", "pr"));
    assert!(!has_word("the domain name", "main"));
    assert!(has_word("open a pr (#12) today", "pr"));
    assert!(has_word("ci: green", "ci"));
    assert!(has_word("caf\u{e9} pr", "pr"));
    assert!(!has_word("", "pr"));
    // A word inside another one only: no prefer, so no Personal sign.
    let got = sort("Domain names and decisions get written down in a ledger book.");
    assert_eq!(got.rule, "default", "{}", got.reason);
}

#[test]
fn dates_are_whole_and_valid() {
    assert_eq!(
        dates("from 2026-08-10 to 2026-11-01."),
        vec![
            NaiveDate::from_ymd_opt(2026, 8, 10).unwrap(),
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()
        ]
    );
    assert!(dates("12026-08-10 and 2026-08-101 and 2026-13-01").is_empty());
    assert!(dates("short").is_empty());
    assert_eq!(dates("caf\u{e9} 2026-08-10").len(), 1);
}

#[test]
fn frontmatter_is_read_and_an_unclosed_one_is_body() {
    let p = parse("---\nname: x\ntype: 'user'\n---\nbody here\n");
    assert_eq!(p.field("name"), Some("x"));
    assert_eq!(p.field("type"), Some("user"));
    assert_eq!(p.body, "body here\n");
    let open = parse("---\nname: x\nbody without an end\n");
    assert!(open.fields.is_empty());
    assert!(open.body.starts_with("---"));
    let bom = parse("\u{feff}---\ntype: project\n---\nb");
    assert_eq!(bom.field("type"), Some("project"));
    assert_eq!(parse("no frontmatter").body, "no frontmatter");
}

#[test]
fn a_reason_is_one_sentence_of_rules_never_the_files_lines() {
    let text = "The zebra-quartz box sits on the tailnet; Homebrew runs there.";
    let got = sort(text);
    assert_eq!(got.layer, SortLayer::Home);
    assert!(!got.reason.contains("zebra"), "{}", got.reason);
    assert!(!got.reason.contains('\n'));
    assert!(got.reason.ends_with('.'));
    assert!(got.reason.starts_with("Home by rules "), "{}", got.reason);
    assert!(!got.reason.contains('\u{2014}') && !got.reason.contains('\u{2013}'));
    // Deterministic.
    assert_eq!(sort(text), got);
}

#[test]
fn every_rule_id_is_reachable_and_unique() {
    let mut ids: Vec<&str> = RULES.iter().map(|r| r.id).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "rule ids are unique");
    for r in RULES {
        assert!(!r.words.is_empty(), "{} has words", r.id);
        assert_ne!(r.layer, SortLayer::Stale, "{} counts for a layer", r.id);
    }
}
