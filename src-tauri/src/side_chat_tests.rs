//! 7.6 tests: a side chat is seeded from its parent without touching it, is
//! never stronger than it, and stays off the chat lists.

use super::*;
use crate::session_store as store;

/// A fixed time: tests never read the clock.
fn at() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-20T09:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn vault() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    // The store is keyed by the vault folder's name: one per test.
    let vault = tmp.path().join(tmp.path().file_name().unwrap());
    std::fs::create_dir_all(&vault).unwrap();
    (tmp, vault)
}

const PARENT: &str = "s-20261120-side0001";

/// One finished turn of `key` in mode `mode`.
fn turn(vault: &Path, key: &str, run: &str, asked: &str, answer: &str, mode: &str) -> i64 {
    let cwd = vault.to_string_lossy().into_owned();
    let ord = store::start_turn(vault, key, "claude", &cwd, run, asked).unwrap();
    store::record_turn_permission(vault, key, ord, mode).unwrap();
    store::append_event(vault, key, ord, "assistant", answer).unwrap();
    store::finish_turn(vault, key, ord, "success").unwrap();
    ord
}

/// The parent's rows that a side chat must never change: its turns and its
/// session row's counters.
fn parent_snapshot(vault: &Path, key: &str) -> (Vec<(i64, String)>, i64, String) {
    store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT ord, asked FROM turns WHERE session_key = ?1 ORDER BY ord")
            .unwrap();
        let turns = stmt
            .query_map(params![key], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let (count, last): (i64, String) = conn
            .query_row(
                "SELECT turn_count, last_activity_at FROM sessions WHERE key = ?1",
                params![key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        Ok((turns, count, last))
    })
    .unwrap()
}

fn with_parent() -> (tempfile::TempDir, std::path::PathBuf) {
    let (tmp, v) = vault();
    turn(
        &v,
        PARENT,
        "run-1",
        "Pick a database",
        "Use Postgres.",
        "acceptEdits",
    );
    turn(
        &v,
        PARENT,
        "run-2",
        "Pick a host",
        "Use Fly.",
        "acceptEdits",
    );
    (tmp, v)
}

#[test]
fn a_side_chat_is_never_stronger_than_its_parent_and_reads_only_by_default() {
    let claude = Some(AgentKind::Claude);
    // Nothing asked: read only, whatever the parent runs in.
    assert_eq!(side_mode(None, "bypassPermissions", claude), "plan");
    assert_eq!(side_mode(None, "plan", claude), "plan");
    // The parent's own mode, asked for by name.
    assert_eq!(
        side_mode(Some("acceptEdits"), "acceptEdits", claude),
        "acceptEdits"
    );
    assert_eq!(
        side_mode(Some("bypassPermissions"), "bypassPermissions", claude),
        "bypassPermissions"
    );
    // Asked for more than the parent has: lowered to the parent's.
    assert_eq!(
        side_mode(Some("bypassPermissions"), "acceptEdits", claude),
        "acceptEdits"
    );
    assert_eq!(side_mode(Some("auto"), "default", claude), "default");
    assert_eq!(side_mode(Some("acceptEdits"), "plan", claude), "plan");
    // An unknown name reads as Hollow's fallback, then is lowered like any.
    assert_eq!(side_mode(Some("everything"), "plan", claude), "plan");
    // Every agent: never above the ceiling, on every pair of modes.
    for kind in [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Grok,
        AgentKind::Antigravity,
    ] {
        for ceiling in crate::app_mcp::PERMISSION_MODES {
            for asked in crate::app_mcp::PERMISSION_MODES {
                let got = side_mode(Some(asked), ceiling, Some(kind));
                assert!(
                    crate::app_mcp::perm_rank(got) <= crate::app_mcp::perm_rank(ceiling),
                    "{kind:?}: {asked} under {ceiling} gave {got}"
                );
                assert!(
                    crate::agent::mode_capability(kind, got)
                        .within(crate::agent::mode_capability(kind, ceiling)),
                    "{kind:?}: {asked} under {ceiling} gave {got}"
                );
            }
        }
    }
}

#[test]
fn opening_makes_a_side_chat_of_the_parents_agent_and_folder_and_never_writes_the_parent() {
    let (_tmp, v) = with_parent();
    let before = parent_snapshot(&v, PARENT);
    let side = open(&v, PARENT, false, at()).unwrap();
    assert_eq!(side.fork_kind.as_deref(), Some(FORK_KIND));
    assert_eq!(side.parent_key.as_deref(), Some(PARENT));
    assert_eq!(side.agent, "claude");
    assert_eq!(side.cwd, v.to_string_lossy());
    assert_eq!(side.turn_count, 0);
    assert_ne!(side.key, PARENT);
    // Not a subagent: none of the subagent machinery claims it.
    assert_ne!(side.fork_kind.as_deref(), Some(store::FORK_SUBAGENT));
    assert_eq!(
        parent_snapshot(&v, PARENT),
        before,
        "the parent is only read"
    );
}

#[test]
fn an_empty_side_chat_is_reused_and_a_used_one_is_continued_until_a_new_one_is_asked_for() {
    let (_tmp, v) = with_parent();
    let first = open(&v, PARENT, false, at()).unwrap();
    // Empty: even a request for a new one gets the same row, so there are no
    // empty rows to list.
    assert_eq!(open(&v, PARENT, true, at()).unwrap().key, first.key);
    assert_eq!(open(&v, PARENT, false, at()).unwrap().key, first.key);
    turn(
        &v,
        &first.key,
        "run-s1",
        "Which one was cheaper?",
        "Fly.",
        "plan",
    );
    // Used: reopened as it was, or a new one on request.
    assert_eq!(open(&v, PARENT, false, at()).unwrap().key, first.key);
    let second = open(&v, PARENT, true, at()).unwrap();
    assert_ne!(second.key, first.key);
    assert_eq!(second.parent_key.as_deref(), Some(PARENT));
    // Only the one with a turn is listed.
    let listed: Vec<String> = list(&v, PARENT)
        .unwrap()
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(listed, vec![first.key.clone()]);
    turn(
        &v,
        &second.key,
        "run-s2",
        "And the slowest?",
        "Neither.",
        "plan",
    );
    let listed: Vec<String> = list(&v, PARENT)
        .unwrap()
        .into_iter()
        .map(|s| s.key)
        .collect();
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&first.key) && listed.contains(&second.key));
}

#[test]
fn a_side_chat_is_refused_for_the_main_agent_and_where_it_cannot_be_read() {
    let (_tmp, v) = with_parent();
    // Nothing sent yet: no row to read.
    let err = open(&v, "s-20261120-side0009", false, at()).unwrap_err();
    assert!(err.contains("first message"), "{err}");
    // A project lead's chat is an ordinary chat here.
    turn(&v, "s-project-demo", "run-l", "Plan", "Done", "plan");
    assert!(open(&v, "s-project-demo", false, at()).is_ok());
    // The Main agent keeps its own page.
    let main = crate::workspaces::main_agent_workspace(&v).unwrap();
    let main_key = "s-20261120-main0001";
    let ord = store::start_turn(&v, main_key, "claude", &main.cwd, "run-m", "Check in").unwrap();
    store::finish_turn(&v, main_key, ord, "success").unwrap();
    let err = open(&v, main_key, false, at()).unwrap_err();
    assert!(err.contains("own page"), "{err}");
    // No side chat of a side chat.
    let side = open(&v, PARENT, false, at()).unwrap();
    let err = open(&v, &side.key, false, at()).unwrap_err();
    assert!(err.contains("side chat can't have"), "{err}");
    // A malformed key.
    assert!(open(&v, "../etc", false, at()).is_err());
}

#[test]
fn side_chats_stay_off_the_chat_lists_and_search() {
    let (_tmp, v) = with_parent();
    let side = open(&v, PARENT, false, at()).unwrap();
    turn(
        &v,
        &side.key,
        "run-s1",
        "Pick a database again",
        "Postgres.",
        "plan",
    );
    let listed = store::list_sessions(&v, 100).unwrap();
    assert!(listed.iter().any(|s| s.key == PARENT));
    assert!(
        listed.iter().all(|s| s.key != side.key),
        "a side chat is not a row of Conversation history or the sidebar"
    );
    let (page, _more) = store::list_sessions_after(&v, |_| true, None, 50).unwrap();
    assert!(page.iter().all(|s| s.key != side.key));
    // Still readable by its key, which is how the pane opens it.
    assert_eq!(
        store::session_by_key(&v, &side.key)
            .unwrap()
            .unwrap()
            .turn_count,
        1
    );
}

#[test]
fn the_first_question_is_seeded_from_the_parent_and_names_the_chat() {
    let (_tmp, v) = with_parent();
    let side = open(&v, PARENT, false, at()).unwrap();
    let (input, _effort) = prepare(&v, &side.key, "Why Fly?", Some("c1".into()), false).unwrap();
    assert_eq!(input.session_key.as_deref(), Some(side.key.as_str()));
    // The existing continue path: the parent's recorded turns and task state
    // go into the fresh native session. Never the parent's native session.
    assert_eq!(input.continued_from, vec![PARENT.to_string()]);
    assert_eq!(input.session_id, None);
    assert_eq!(input.agent.as_deref(), Some("claude"));
    assert_eq!(input.cwd.as_deref(), Some(v.to_string_lossy().as_ref()));
    assert!(input.typed);
    assert_eq!(input.client_turn_id.as_deref(), Some("c1"));
    assert_eq!(
        store::session_by_key(&v, &side.key)
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Why Fly?")
    );
    // The compiler puts what the parent said into that fresh session's
    // message, and reads the parent without writing it.
    let before = parent_snapshot(&v, PARENT);
    let compiled = crate::context_compiler::compile_turn(
        &v,
        &side.key,
        &input.continued_from,
        i64::MAX,
        crate::context_compiler::Entry::Composer,
        crate::context_compiler::Session::New,
        false,
        crate::task_state::Reader::AppTools,
        "Why Fly?",
    );
    assert!(compiled.body.contains("Use Postgres."), "{}", compiled.body);
    assert!(compiled.body.contains("Use Fly."), "{}", compiled.body);
    assert!(compiled.body.ends_with("Why Fly?"), "{}", compiled.body);
    assert_eq!(parent_snapshot(&v, PARENT), before);
    // A resumed native session already holds it: no second bundle.
    let resumed = crate::context_compiler::compile_turn(
        &v,
        &side.key,
        &input.continued_from,
        i64::MAX,
        crate::context_compiler::Entry::Composer,
        crate::context_compiler::Session::Resume,
        false,
        crate::task_state::Reader::AppTools,
        "And Postgres?",
    );
    assert!(!resumed.body.contains("Use Fly."), "{}", resumed.body);
}

#[test]
fn a_later_question_resumes_the_side_chats_own_native_session() {
    let (_tmp, v) = with_parent();
    let side = open(&v, PARENT, false, at()).unwrap();
    let ord = turn(&v, &side.key, "run-s1", "Why Fly?", "Closest.", "plan");
    store::record_harness_id(&v, &side.key, "native-side-1").unwrap();
    let _ = ord;
    let (input, _) = prepare(&v, &side.key, "And Postgres?", None, false).unwrap();
    assert_eq!(input.session_id.as_deref(), Some("native-side-1"));
    // Its own native session, not the parent's.
    store::record_harness_id(&v, PARENT, "native-parent-1").unwrap();
    let (input, _) = prepare(&v, &side.key, "And Postgres?", None, false).unwrap();
    assert_eq!(input.session_id.as_deref(), Some("native-side-1"));
}

#[test]
fn a_side_question_inherits_the_parents_mode_as_a_ceiling_and_defaults_to_read_only() {
    let (_tmp, v) = vault();
    turn(
        &v,
        PARENT,
        "run-1",
        "Edit the config",
        "Done.",
        "acceptEdits",
    );
    let side = open(&v, PARENT, false, at()).unwrap();
    let (read_only, _) = prepare(&v, &side.key, "What changed?", None, false).unwrap();
    assert_eq!(read_only.permission_mode.as_deref(), Some("plan"));
    let (same, _) = prepare(&v, &side.key, "What changed?", None, true).unwrap();
    assert_eq!(same.permission_mode.as_deref(), Some("acceptEdits"));
    // The parent later runs in a stricter mode: the side chat follows it down.
    turn(&v, PARENT, "run-2", "Just look", "Looked.", "plan");
    let (same, _) = prepare(&v, &side.key, "What changed?", None, true).unwrap();
    assert_eq!(same.permission_mode.as_deref(), Some("plan"));
}

#[test]
fn admission_lowers_any_send_into_a_side_chat_and_leaves_other_chats_alone() {
    let (_tmp, v) = vault();
    turn(
        &v,
        PARENT,
        "run-1",
        "Edit the config",
        "Done.",
        "acceptEdits",
    );
    let side = open(&v, PARENT, false, at()).unwrap();
    // A caller naming bypass (a hollow_app send, a phone, a held send) is
    // lowered to the parent's mode; none asked is read only.
    assert_eq!(
        clamp_admitted(&v, &side.key, Some("bypassPermissions".into())).as_deref(),
        Some("acceptEdits")
    );
    assert_eq!(
        clamp_admitted(&v, &side.key, Some("auto".into())).as_deref(),
        Some("acceptEdits")
    );
    assert_eq!(clamp_admitted(&v, &side.key, None).as_deref(), Some("plan"));
    assert_eq!(
        clamp_admitted(&v, &side.key, Some("plan".into())).as_deref(),
        Some("plan")
    );
    // The parent itself, an ordinary chat, a chat not yet stored: unchanged.
    assert_eq!(
        clamp_admitted(&v, PARENT, Some("bypassPermissions".into())).as_deref(),
        Some("bypassPermissions")
    );
    assert_eq!(clamp_admitted(&v, PARENT, None), None);
    assert_eq!(
        clamp_admitted(&v, "s-20261120-nothere1", Some("auto".into())).as_deref(),
        Some("auto")
    );
}

#[test]
fn a_side_chat_counts_like_any_chat_not_as_its_parents_subagent() {
    let (_tmp, v) = with_parent();
    let side = open(&v, PARENT, false, at()).unwrap();
    turn(&v, &side.key, "run-s1", "Why Fly?", "Closest.", "plan");
    // Its turns, run ids and counters are its own rows.
    let own = parent_snapshot(&v, &side.key);
    assert_eq!(own.0.len(), 1);
    assert_eq!(own.1, 1);
    assert_eq!(
        parent_snapshot(&v, PARENT).1,
        2,
        "the parent's count is unchanged"
    );
    assert_eq!(
        store::session_by_key(&v, &side.key)
            .unwrap()
            .unwrap()
            .fork_kind
            .as_deref(),
        Some("side")
    );
    assert_ne!("side", store::FORK_SUBAGENT);
}
