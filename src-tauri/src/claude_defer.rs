//! AG4b: a Claude tool call Hollow must ask about becomes an approval card.
//!
//! An ask-first Claude chat turn (`--permission-mode default`, local, not a
//! self-hosted model) carries one PreToolUse hook, the hidden
//! `hollow claude-defer-hook` command. For the tools Claude would ask about
//! ([`GATED`]) the hook answers `defer`: the `-p` turn ends with
//! `stop_reason: tool_deferred` and a `deferred_tool_use {id, name, input}`
//! (probed on Claude Code 2.1.326, `docs/reference/interrupts.md`). The chat
//! loop ([`crate::chat`], `Next::Deferred`) shows an approval card
//! ([`crate::approvals`]), records the answer for that exact call id in a
//! private answers file, and resumes the same native session. The hook fires
//! again for the same id and reads the answer: allow runs the call, deny
//! refuses it with the person's words. Every other call is deferred again, so
//! a second call in the same turn gets its own card.
//!
//! Deny by default: a missing, unreadable or non-boolean answer never allows.
//! The hook decides nothing about an ungated tool (no output: Claude's own
//! flow applies). Pure functions only here, apart from the answers file and
//! [`run_hook`]; the loop and the card live in `chat.rs` and `approvals.rs`.

use serde_json::{json, Value};
use std::io::Read as _;
use std::path::Path;

/// The hidden `hollow` subcommand the hook runs.
pub(crate) const COMMAND: &str = "claude-defer-hook";
/// The env var (set inside the hook's command line) naming the answers file.
pub(crate) const ENV_ANSWERS: &str = "HOLLOW_DEFER_ANSWERS";

/// The tools Claude's default mode asks about: the hook defers these and
/// lets every other tool follow Claude's own flow.
pub(crate) const GATED: [&str; 5] = ["Bash", "Edit", "MultiEdit", "NotebookEdit", "Write"];

/// A call the turn ended on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeferredCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

/// The deferred call in a stream-json `result` line, `None` for any other
/// line (an ordinary result, a deferral with no id or name).
pub(crate) fn deferred_call(line: &str) -> Option<DeferredCall> {
    let value: Value = serde_json::from_str(line).ok()?;
    if value.get("type").and_then(Value::as_str) != Some("result")
        || value.get("stop_reason").and_then(Value::as_str) != Some("tool_deferred")
    {
        return None;
    }
    let call = value.get("deferred_tool_use")?;
    let text = |key: &str| {
        call.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(DeferredCall {
        id: text("id")?,
        name: text("name")?,
        input: call.get("input").cloned().unwrap_or(Value::Null),
    })
}

/// The most characters a card shows for one call (the command, or the path
/// and the change). Longer is never shown cut: [`action_text`] declines it,
/// since an Allow would run or write the whole thing.
pub(crate) const ACTION_MAX: usize = 2000;
/// The most lines a card shows, so blank lines cannot push the rest of a
/// call out of sight.
const LINES_MAX: usize = 40;
/// The longest run of spaces a card shows, for the same reason.
const SPACES_MAX: usize = 60;

/// Characters that change how text reads without being control characters:
/// soft hyphen, Arabic letter mark, zero-width and bidi marks and overrides,
/// line and paragraph separators, word joiners and isolates, the byte-order
/// mark and the tag block. A card showing one could read differently from
/// what runs, so a call holding one is declined.
fn is_deceptive(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{061C}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
        | '\u{2060}'..='\u{2069}' | '\u{FEFF}' | '\u{E0000}'..='\u{E007F}')
}

/// What the card says: the tool, what it would do and the whole change (a
/// command, a path, the old and new text of an edit, a write's content), line
/// breaks kept. `Err(reason)` when the card could not show all of what Allow
/// does (over [`ACTION_MAX`] characters, [`LINES_MAX`] lines, a run of
/// [`SPACES_MAX`] spaces, or a character [`is_deceptive`]); the call is then
/// declined with that reason, never shown cut.
pub(crate) fn action_text(call: &DeferredCall) -> Result<String, String> {
    let input = &call.input;
    let field = |key: &str| input.get(key).and_then(Value::as_str);
    let mut parts: Vec<String> = Vec::new();
    let verb = match call.name.as_str() {
        "Bash" => "Run a shell command",
        "Write" => "Write a file",
        "NotebookEdit" => "Edit a notebook",
        "MultiEdit" => "Edit a file (several changes)",
        _ => "Edit a file",
    };
    let mut head = verb.to_string();
    if call.name == "Bash" {
        if input
            .get("dangerouslyDisableSandbox")
            .and_then(Value::as_bool)
            == Some(true)
        {
            head.push_str(" (sandbox off)");
        }
        if input.get("run_in_background").and_then(Value::as_bool) == Some(true) {
            head.push_str(" (in the background)");
        }
    }
    if call.name == "NotebookEdit" {
        if let Some(mode) = field("edit_mode") {
            head.push_str(&format!(" ({mode})"));
        }
    }
    let mut add = |label: &str, text: Option<String>| {
        if let Some(text) = text {
            parts.push(if label.is_empty() {
                text
            } else {
                format!("{label}:\n{text}")
            });
        }
    };
    let own = |key: &str| field(key).map(str::to_string);
    match call.name.as_str() {
        "Bash" => add("", own("command")),
        "Write" => {
            add("", own("file_path"));
            add("Content", own("content"));
        }
        "MultiEdit" => {
            add("", own("file_path"));
            for (n, edit) in input
                .get("edits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .enumerate()
            {
                let get = |key: &str| edit.get(key).and_then(Value::as_str).map(str::to_string);
                add(&format!("Change {} replaces", n + 1), get("old_string"));
                add("With", get("new_string"));
            }
        }
        "NotebookEdit" => {
            add("", own("notebook_path"));
            add("Cell", own("cell_id"));
            add("New source", own("new_source"));
        }
        _ => {
            add("", own("file_path"));
            add("Replaces", own("old_string"));
            add("With", own("new_string"));
        }
    }
    let detail = parts.join("\n");
    if detail.chars().any(is_deceptive) {
        return Err(
            "the call holds characters that could make an approval card read differently from what runs; ask for it in plain text".into(),
        );
    }
    let detail: String = detail
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .collect();
    let detail = detail.trim();
    if detail.chars().count() > ACTION_MAX
        || detail.lines().count() > LINES_MAX
        || detail.contains(&" ".repeat(SPACES_MAX))
    {
        return Err(
            "the call is too long or too padded to review in an approval card; ask for a smaller one".into(),
        );
    }
    Ok(if detail.is_empty() {
        head
    } else if detail.contains('\n') {
        format!("{head}:\n{detail}")
    } else {
        format!("{head}: {detail}")
    })
}

/// What the hook should print for a PreToolUse `input` (the hook's stdin
/// JSON), given the answers file's content. `None` prints nothing, so Claude
/// decides as it would without the hook.
pub(crate) fn hook_output(input: &str, answers: &Value) -> Option<String> {
    let value: Value = serde_json::from_str(input).ok()?;
    let name = value.get("tool_name").and_then(Value::as_str)?;
    if !GATED.contains(&name) {
        return None;
    }
    let id = value
        .get("tool_use_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if id.is_empty() {
        // No id to bind an answer to: never ask, never allow.
        return Some(
            json!({"hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": "Hollow could not identify this tool call to ask the person about it",
            }})
            .to_string(),
        );
    }
    let (decision, reason) = match answers.get(id) {
        // Only the boolean `true` allows.
        Some(answer) if answer.get("approved").and_then(Value::as_bool) == Some(true) => (
            "allow",
            "The person approved this call in Hollow".to_string(),
        ),
        Some(answer) => (
            "deny",
            match answer.get("reason").and_then(Value::as_str) {
                Some(why) if !why.trim().is_empty() => {
                    format!("The person declined this call in Hollow: {why}")
                }
                _ => "The person declined this call in Hollow".to_string(),
            },
        ),
        None => ("defer", "Hollow asks the person first".to_string()),
    };
    Some(
        json!({"hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": decision,
            "permissionDecisionReason": reason,
        }})
        .to_string(),
    )
}

/// Record the person's answer for call `id` in the answers file (a JSON
/// object keyed by call id), keeping earlier answers. Written to a sibling
/// file and renamed, owner-only, so the hook never reads half a file.
pub(crate) fn record_answer(
    path: &Path,
    id: &str,
    approved: bool,
    reason: Option<&str>,
) -> std::io::Result<()> {
    let mut answers = read_answers(path);
    if !answers.is_object() {
        answers = json!({});
    }
    answers[id] = json!({"approved": approved, "reason": reason});
    let tmp = path.with_extension("tmp");
    {
        use std::io::Write as _;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(answers.to_string().as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

/// The answers file as JSON; `{}` when missing or unreadable (deny by
/// default is the hook's, so a bad file never allows).
pub(crate) fn read_answers(path: &Path) -> Value {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}))
}

/// The shell command line the hook entry runs: the answers path in the env,
/// the app's own binary, the hidden command. Both paths are single-quoted.
pub(crate) fn hook_command(exe: &Path, answers: &Path) -> String {
    let quote = |p: &Path| format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"));
    format!("{ENV_ANSWERS}={} {} {COMMAND}", quote(answers), quote(exe))
}

/// `base` (the run's `--settings` JSON, when it has one) with one PreToolUse
/// hook on the gated tools added. A base that is not a JSON object is
/// returned unchanged, so a pin in it is never dropped.
pub(crate) fn settings_with_hook(base: Option<String>, command: &str) -> Option<String> {
    let mut value = match base.as_deref() {
        None => json!({}),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(v) if v.is_object() => v,
            _ => return base,
        },
    };
    let entry = json!({
        "matcher": GATED.join("|"),
        "hooks": [{"type": "command", "command": command}],
    });
    if !value.get("hooks").is_some_and(Value::is_object) {
        value["hooks"] = json!({});
    }
    let hooks = &mut value["hooks"];
    if !hooks.get("PreToolUse").is_some_and(Value::is_array) {
        hooks["PreToolUse"] = json!([]);
    }
    if let Some(list) = hooks["PreToolUse"].as_array_mut() {
        list.push(entry);
    }
    Some(value.to_string())
}

/// `hollow claude-defer-hook`: read the hook input from stdin, the answers
/// file from [`ENV_ANSWERS`], print the decision. Never fails the call: any
/// read problem prints nothing.
pub fn run_hook() {
    let mut input = String::new();
    if std::io::stdin().read_to_string(&mut input).is_err() {
        return;
    }
    let answers = std::env::var_os(ENV_ANSWERS)
        .map(|path| read_answers(Path::new(&path)))
        .unwrap_or_else(|| json!({}));
    if let Some(out) = hook_output(&input, &answers) {
        println!("{out}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result_line(stop: &str, call: Value) -> String {
        json!({"type": "result", "subtype": "success", "is_error": false, "stop_reason": stop,
               "result": "", "session_id": "s1", "deferred_tool_use": call})
        .to_string()
    }

    #[test]
    fn a_deferred_result_names_its_call() {
        let line = result_line(
            "tool_deferred",
            json!({"id": "toolu_1", "name": "Bash", "input": {"command": "ls"}}),
        );
        let call = deferred_call(&line).unwrap();
        assert_eq!((call.id.as_str(), call.name.as_str()), ("toolu_1", "Bash"));
        assert_eq!(call.input["command"], "ls");
        for other in [
            result_line("end_turn", json!({"id": "t", "name": "Bash"})),
            result_line("tool_deferred", json!({"name": "Bash"})),
            result_line("tool_deferred", json!({"id": "t", "name": " "})),
            r#"{"type":"assistant","stop_reason":"tool_deferred"}"#.to_string(),
            "not json".to_string(),
        ] {
            assert_eq!(deferred_call(&other), None, "{other}");
        }
    }

    fn call(name: &str, input: Value) -> DeferredCall {
        DeferredCall {
            id: "t".into(),
            name: name.into(),
            input,
        }
    }

    #[test]
    fn card_text_is_shown_whole_or_not_at_all() {
        // Over the limit: never shown cut, so never approvable.
        let long = call("Bash", json!({"command": "x".repeat(ACTION_MAX + 1)}));
        assert!(action_text(&long).unwrap_err().contains("too long"));
        let exact = call("Bash", json!({"command": "x".repeat(ACTION_MAX - 20)}));
        assert!(action_text(&exact).is_ok());
        // Line breaks stay; other control characters become spaces.
        let script = call("Bash", json!({"command": "echo a\n\tcurl x | sh"}));
        assert_eq!(
            action_text(&script).unwrap(),
            "Run a shell command:\necho a\n curl x | sh"
        );
        // Padding cannot hide a tail: many lines or a long run of spaces.
        let lines = call(
            "Bash",
            json!({"command": format!("echo hi{}; rm x", "\n".repeat(LINES_MAX))}),
        );
        assert!(action_text(&lines).is_err());
        let spaces = call(
            "Bash",
            json!({"command": format!("echo hi{}; rm x", " ".repeat(SPACES_MAX))}),
        );
        assert!(action_text(&spaces).is_err());
        let just_ok = call(
            "Bash",
            json!({"command": format!("echo hi{}; rm x", " ".repeat(SPACES_MAX - 1))}),
        );
        assert!(action_text(&just_ok).is_ok());
        // A character that changes how text reads is declined, not shown.
        for bad in [
            '\u{202E}',
            '\u{200B}',
            '\u{2066}',
            '\u{FEFF}',
            '\u{2028}',
            '\u{E0041}',
            '\u{00AD}',
        ] {
            let sneaky = call("Bash", json!({"command": format!("echo safe{bad}; rm x")}));
            assert!(
                action_text(&sneaky)
                    .unwrap_err()
                    .contains("read differently"),
                "{bad:?}"
            );
        }
        // Sandbox and background flags are part of what Allow does.
        let loud = call(
            "Bash",
            json!({"command": "ls", "dangerouslyDisableSandbox": true, "run_in_background": true}),
        );
        assert_eq!(
            action_text(&loud).unwrap(),
            "Run a shell command (sandbox off) (in the background): ls"
        );
    }

    #[test]
    fn edits_and_writes_show_the_change_not_just_the_path() {
        let edit = call(
            "Edit",
            json!({"file_path": "/a/b.rs", "old_string": "let a = 1;", "new_string": "let a = 2;"}),
        );
        assert_eq!(
            action_text(&edit).unwrap(),
            "Edit a file:\n/a/b.rs\nReplaces:\nlet a = 1;\nWith:\nlet a = 2;"
        );
        let multi = call(
            "MultiEdit",
            json!({"file_path": "/a/b.rs", "edits": [
                {"old_string": "a", "new_string": "b"}, {"old_string": "c", "new_string": "d"}]}),
        );
        let text = action_text(&multi).unwrap();
        assert!(
            text.starts_with("Edit a file (several changes):\n/a/b.rs"),
            "{text}"
        );
        assert!(text.contains("Change 1 replaces:\na\nWith:\nb"), "{text}");
        assert!(text.contains("Change 2 replaces:\nc\nWith:\nd"), "{text}");
        let write = call(
            "Write",
            json!({"file_path": "/a/c.txt", "content": "héllo"}),
        );
        assert_eq!(
            action_text(&write).unwrap(),
            "Write a file:\n/a/c.txt\nContent:\nhéllo"
        );
        let cell = call(
            "NotebookEdit",
            json!({"notebook_path": "/n.ipynb", "cell_id": "c1", "new_source": "print(1)", "edit_mode": "replace"}),
        );
        assert_eq!(
            action_text(&cell).unwrap(),
            "Edit a notebook (replace):\n/n.ipynb\nCell:\nc1\nNew source:\nprint(1)"
        );
        // A change too big to show is declined, never shown cut.
        let big = call(
            "Write",
            json!({"file_path": "/a", "content": "y".repeat(ACTION_MAX)}),
        );
        assert!(action_text(&big).is_err());
        let bare = call("Write", Value::Null);
        assert_eq!(action_text(&bare).unwrap(), "Write a file");
    }

    fn decision(out: Option<String>) -> Option<(String, String)> {
        let v: Value = serde_json::from_str(&out?).unwrap();
        let o = &v["hookSpecificOutput"];
        assert_eq!(o["hookEventName"], "PreToolUse");
        Some((
            o["permissionDecision"].as_str().unwrap().to_string(),
            o["permissionDecisionReason"].as_str().unwrap().to_string(),
        ))
    }

    fn hook_input(tool: &str, id: &str) -> String {
        json!({"tool_name": tool, "tool_use_id": id, "session_id": "s1"}).to_string()
    }

    #[test]
    fn the_hook_defers_until_that_exact_call_is_answered() {
        let none = json!({});
        assert_eq!(
            decision(hook_output(&hook_input("Bash", "a"), &none))
                .unwrap()
                .0,
            "defer"
        );
        // Another call's answer never covers this one.
        let answers = json!({"a": {"approved": true}});
        assert_eq!(
            decision(hook_output(&hook_input("Bash", "a"), &answers))
                .unwrap()
                .0,
            "allow"
        );
        assert_eq!(
            decision(hook_output(&hook_input("Bash", "b"), &answers))
                .unwrap()
                .0,
            "defer"
        );
        // A call with no id is never matched to an answer or asked about.
        assert_eq!(
            decision(hook_output(&hook_input("Bash", ""), &answers))
                .unwrap()
                .0,
            "deny"
        );
    }

    #[test]
    fn only_the_boolean_true_allows_and_a_decline_carries_the_words() {
        for bad in [json!("true"), json!(1), json!(null), json!("yes")] {
            let answers = json!({"a": {"approved": bad}});
            assert_eq!(
                decision(hook_output(&hook_input("Write", "a"), &answers))
                    .unwrap()
                    .0,
                "deny"
            );
        }
        let answers = json!({"a": {"approved": false, "reason": "use staging"}});
        let (kind, why) = decision(hook_output(&hook_input("Edit", "a"), &answers)).unwrap();
        assert_eq!(kind, "deny");
        assert!(why.ends_with("use staging"), "{why}");
        let answers = json!({"a": {"approved": false, "reason": "  "}});
        let (_, why) = decision(hook_output(&hook_input("Edit", "a"), &answers)).unwrap();
        assert_eq!(why, "The person declined this call in Hollow");
    }

    #[test]
    fn ungated_tools_and_bad_input_print_nothing() {
        let none = json!({});
        assert_eq!(hook_output(&hook_input("Read", "a"), &none), None);
        assert_eq!(
            hook_output(&hook_input("mcp__hollow_app__run_action", "a"), &none),
            None
        );
        assert_eq!(hook_output("not json", &none), None);
        assert_eq!(hook_output("{}", &none), None);
    }

    #[test]
    fn answers_accumulate_and_a_bad_file_never_allows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("answers.json");
        assert_eq!(read_answers(&path), json!({}));
        record_answer(&path, "a", true, None).unwrap();
        record_answer(&path, "b", false, Some("no")).unwrap();
        let answers = read_answers(&path);
        assert_eq!(answers["a"]["approved"], true);
        assert_eq!(answers["b"]["reason"], "no");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        std::fs::write(&path, "{broken").unwrap();
        assert_eq!(
            decision(hook_output(&hook_input("Bash", "a"), &read_answers(&path)))
                .unwrap()
                .0,
            "defer"
        );
        // Recording over a broken file starts clean.
        record_answer(&path, "c", true, None).unwrap();
        assert_eq!(read_answers(&path)["c"]["approved"], true);
    }

    #[test]
    fn settings_keep_the_base_and_gain_one_hook() {
        let command = hook_command(Path::new("/A b/it's/hollow"), Path::new("/t/ans wers.json"));
        assert_eq!(
            command,
            "HOLLOW_DEFER_ANSWERS='/t/ans wers.json' '/A b/it'\\''s/hollow' claude-defer-hook"
        );
        let added: Value =
            serde_json::from_str(&settings_with_hook(None, &command).unwrap()).unwrap();
        let entry = &added["hooks"]["PreToolUse"][0];
        assert_eq!(entry["matcher"], "Bash|Edit|MultiEdit|NotebookEdit|Write");
        assert_eq!(entry["hooks"][0]["command"], command.as_str());
        let base =
            json!({"env": {"A": "1"}, "hooks": {"PreToolUse": [{"matcher": "X"}], "Stop": []}});
        let merged: Value =
            serde_json::from_str(&settings_with_hook(Some(base.to_string()), &command).unwrap())
                .unwrap();
        assert_eq!(merged["env"]["A"], "1");
        assert_eq!(merged["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        assert!(merged["hooks"]["Stop"].is_array());
        // A base that is not an object is left alone: no pin is dropped.
        assert_eq!(
            settings_with_hook(Some("[1]".into()), &command),
            Some("[1]".into())
        );
    }
}
