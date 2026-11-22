//! WD1 tests: the window's lean cold open of a long chat stays inside its
//! budget, keeps what collapsed rows read, and an expanded row gets the
//! whole result back.

use super::*;

#[test]
fn reasoning_round_trip_is_separate_and_not_cut_at_the_event_limit() {
    let (_tmp, vault) = vault();
    let key = "s-20261120-reasoning";
    let ord = start_turn(
        &vault,
        key,
        "codex",
        &vault.to_string_lossy(),
        "reason-run",
        "Solve it",
    )
    .unwrap();
    let reasoning = "文".repeat(12_000);
    append_event(&vault, key, ord, "reasoning", &reasoning).unwrap();
    append_event(&vault, key, ord, "assistant", "Answer").unwrap();
    let read = read_conversation(&vault, key).unwrap();
    let answer = read.turns.iter().find(|t| t.role == "assistant").unwrap();
    assert_eq!(answer.reasoning, reasoning);
    assert_eq!(answer.text, "Answer");
    let context = conversation_entries(&vault, key, ord + 1).unwrap();
    assert!(!context.iter().any(|t| t.text.contains('文')));
    let old: StoredTranscript = serde_json::from_str("{}").unwrap();
    assert!(old.reasoning.is_empty());
}

/// The fixture's lean cold open may carry at most this many bytes (measured
/// 255,234 on 11-18; its full read 828,294). Raising it needs a reason in
/// the build log.
const FIXTURE_LEAN_BUDGET: usize = 256 * 1024;

fn vault() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    // The store is keyed by the vault folder's name: one per test.
    let vault = tmp.path().join(tmp.path().file_name().unwrap());
    std::fs::create_dir_all(&vault).unwrap();
    (tmp, vault)
}

fn tool(vault: &Path, key: &str, ord: i64, id: &str, name: &str, input: Value, result: &str) {
    let used = json!({ "id": id, "name": name, "input": input });
    append_event(vault, key, ord, "toolUse", &used.to_string()).unwrap();
    let done = json!({ "id": id, "content": result });
    append_event(vault, key, ord, "toolResult", &done.to_string()).unwrap();
}

/// A Read result of `n` numbered lines, as Claude Code prints them.
fn numbered(n: usize) -> String {
    (1..=n)
        .map(|i| format!("{i:>6}\u{2192}let value_{i} = compute_something_long({i});"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A long chat: `turns` turns, each a Read, a Bash run, a subagent start
/// and a reply.
fn long_chat(vault: &Path, key: &str, turns: usize) {
    let cwd = vault.to_string_lossy().into_owned();
    let read = numbered(100);
    let bash = "test output line that goes on for a while\n".repeat(120);
    let start =
        json!({ "sessionKey": "s-20261118-child001", "note": "x".repeat(1_800) }).to_string();
    for t in 0..turns {
        let run = format!("run-{t}");
        let ord = start_turn(vault, key, "claude", &cwd, &run, "Do the next step").unwrap();
        tool(
            vault,
            key,
            ord,
            &format!("r{t}"),
            "Read",
            json!({ "file_path": "/x/a.rs" }),
            &read,
        );
        tool(
            vault,
            key,
            ord,
            &format!("b{t}"),
            "Bash",
            json!({ "command": "npm test" }),
            &bash,
        );
        tool(
            vault,
            key,
            ord,
            &format!("s{t}"),
            "mcp__hollow_app__start_session",
            json!({ "title": "child" }),
            &start,
        );
        append_event(
            vault,
            key,
            ord,
            "assistant",
            &"Done with that step. ".repeat(25),
        )
        .unwrap();
        finish_turn(vault, key, ord, "success").unwrap();
    }
}

#[test]
fn a_long_chats_cold_open_stays_inside_its_budget() {
    let (_tmp, v) = vault();
    let key = "s-20261118-wd1a0001";
    long_chat(&v, key, 60);
    let full = read_conversation(&v, key).unwrap();
    let full_bytes = serde_json::to_vec(&full).unwrap().len();
    let mut lean = full.clone();
    lean_for_window(&mut lean);
    let lean_bytes = serde_json::to_vec(&lean).unwrap().len();
    eprintln!("WD1 fixture: 60 turns, full {full_bytes} bytes, lean {lean_bytes} bytes");
    assert!(
        full_bytes > 800_000,
        "the fixture is a long chat: {full_bytes}"
    );
    assert!(
        lean_bytes <= FIXTURE_LEAN_BUDGET,
        "the lean cold open passed its budget: {lean_bytes} > {FIXTURE_LEAN_BUDGET}"
    );
    // Every turn and tool is still there, in order.
    assert_eq!(lean.turns.len(), full.turns.len());
    for (a, b) in lean.turns.iter().zip(&full.turns) {
        assert_eq!(
            a.tools.iter().map(|t| &t.id).collect::<Vec<_>>(),
            b.tools.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
        assert_eq!(a.text, b.text);
    }
}

#[test]
fn a_preview_keeps_what_a_collapsed_row_reads_and_expanding_brings_it_whole() {
    let (_tmp, v) = vault();
    let key = "s-20261118-wd1a0002";
    long_chat(&v, key, 2);
    let cwd = v.to_string_lossy().into_owned();
    let ord = start_turn(&v, key, "claude", &cwd, "run-pr", "Ship it").unwrap();
    let pr = format!(
        "{}\nhttps://github.com/o/r/pull/77\n",
        "Creating pull request\n".repeat(80)
    );
    tool(
        &v,
        key,
        ord,
        "pr",
        "Bash",
        json!({ "command": "gh pr create --fill" }),
        &pr,
    );
    tool(
        &v,
        key,
        ord,
        "small",
        "Bash",
        json!({ "command": "ls" }),
        "a\nb\n",
    );
    finish_turn(&v, key, ord, "success").unwrap();

    let mut lean = read_conversation(&v, key).unwrap();
    lean_for_window(&mut lean);
    let find = |id: &str| {
        lean.turns
            .iter()
            .flat_map(|t| &t.tools)
            .find(|t| t.id == id)
            .unwrap()
            .clone()
    };
    // A long Read: a preview, its full length and its line count.
    let read = find("r0");
    let whole = numbered(100);
    assert_eq!(read.result_chars, Some(whole.chars().count() as u64));
    assert_eq!(read.result_lines, Some(100));
    assert_eq!(
        read.result.as_deref().unwrap().chars().count(),
        WIRE_PREVIEW_CHARS
    );
    assert!(whole.starts_with(read.result.as_deref().unwrap()));
    // A long Bash run: a preview, no line count.
    let bash = find("b0");
    assert!(bash.result_chars.is_some());
    assert_eq!(bash.result_lines, None);
    // Kept whole: Hollow's own tool (the subagent key is parsed from it),
    // gh pr create (the PR link), and anything short.
    let start = find("s0");
    assert_eq!(start.result_chars, None);
    assert!(start
        .result
        .as_deref()
        .unwrap()
        .contains("s-20261118-child001"));
    assert!(find("pr").result.as_deref().unwrap().contains("/pull/77"));
    assert_eq!(find("small").result.as_deref(), Some("a\nb\n"));
    // The agents' read stays whole.
    let full = read_conversation(&v, key).unwrap();
    assert!(full
        .turns
        .iter()
        .flat_map(|t| &t.tools)
        .all(|t| t.result_chars.is_none()));

    // Expanding: from the preview's own turn.
    assert_eq!(read.result_turn, Some(1));
    assert_eq!(
        read_tool_result(&v, key, 1, "r0").unwrap(),
        Some(whole.clone())
    );
    assert_eq!(read_tool_result(&v, key, 2, "r1").unwrap(), Some(whole));
    let err = read_tool_result(&v, key, 2, "nope").unwrap_err();
    assert!(err.contains("isn't in this chat"), "{err}");
    assert!(read_tool_result(&v, key, 1, "").is_err());
    assert!(read_tool_result(&v, "../x", 1, "r0").is_err());
}

/// Codex tool ids repeat in every run (`item_0`, ...): each preview is read
/// back from its own turn, never another turn's call with the same id.
#[test]
fn a_repeated_tool_id_is_read_back_from_its_own_turn() {
    let (_tmp, v) = vault();
    let key = "s-20261118-wd1a0003";
    let cwd = v.to_string_lossy().into_owned();
    for (run, body) in [("run-a", "A"), ("run-b", "B")] {
        let ord = start_turn(&v, key, "codex", &cwd, run, "go").unwrap();
        let out = body.repeat(2_000);
        tool(
            &v,
            key,
            ord,
            "item_0",
            "exec_command",
            json!({ "cmd": "make" }),
            &out,
        );
        finish_turn(&v, key, ord, "success").unwrap();
    }
    let mut lean = read_conversation(&v, key).unwrap();
    lean_for_window(&mut lean);
    let turns: Vec<i64> = lean
        .turns
        .iter()
        .flat_map(|t| &t.tools)
        .filter_map(|t| t.result_turn)
        .collect();
    assert_eq!(turns, vec![1, 2]);
    assert_eq!(
        read_tool_result(&v, key, 1, "item_0").unwrap(),
        Some("A".repeat(2_000))
    );
    assert_eq!(
        read_tool_result(&v, key, 2, "item_0").unwrap(),
        Some("B".repeat(2_000))
    );
}

#[test]
fn what_collapsed_rows_parse_stays_whole_and_the_rest_is_cut() {
    let call = |id: &str, name: &str, input: Value| ConversationTool {
        id: id.into(),
        name: name.into(),
        input,
        result: Some("x".repeat(5_000)),
        is_error: false,
        status: "done".into(),
        parent_tool_use_id: None,
        result_chars: None,
        result_lines: None,
        result_turn: None,
    };
    let whole = [
        call("", "Bash", json!({ "command": "ls" })),
        call("a", "mcp__hollow_app__start_session", json!({})),
        call(
            "b",
            "mcp__hollow_app__run_action",
            json!({ "id": "sessions.route_to_runner" }),
        ),
        call(
            "c",
            "exec_command",
            json!({ "cmd": ["gh", "pr", "create", "--fill"] }),
        ),
        call("d", "mcp__hollow_questions__ask_user_questions", json!({})),
    ];
    for mut tool in whole {
        lean_tool(&mut tool, 1);
        assert_eq!(tool.result_chars, None, "{}", tool.name);
    }
    let cut = [
        call("e", "mcp__hollow_app__read_session", json!({ "key": "k" })),
        call(
            "f",
            "mcp__hollow_app__run_action",
            json!({ "id": "vault.read" }),
        ),
        call("g", "Bash", json!({ "command": "cat -n x" })),
    ];
    for mut tool in cut {
        lean_tool(&mut tool, 1);
        assert_eq!(tool.result_chars, Some(5_000), "{}", tool.name);
        assert_eq!(tool.result_lines, None, "{}", tool.name);
    }
}

#[test]
fn numbered_lines_match_the_windows_count() {
    assert_eq!(numbered_lines("     1\u{2192}a\n     2\tb\nplain\n3x"), 2);
    assert_eq!(numbered_lines(""), 0);
}
