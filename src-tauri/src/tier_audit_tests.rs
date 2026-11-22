//! The confirm-tier audit (overnight E4): every app action, swept through
//! the broker's real `execute` in every permission mode and from an
//! automation, shows its card exactly where [`app_actions::asks_in_mode`]
//! says it must, and a Deny leaves the vault byte-for-byte as it was. The
//! user's own clicks (`USER_ONLY`) are reached by no agent tool, and the
//! Main agent's unattended allow list holds no action that asks.
//!
//! Only calls that must show a card (or must be refused) are executed: a
//! call the mode approves would run its real handler, so the sweep checks
//! those through `asks_in_mode` and the classification test in
//! `app_broker.rs` (`every_confirm_action_is_classified_for_auto`) instead.

use crate::app_actions::{self, Args, Caller, Kind, Tier, ACTIONS, ALWAYS_ASK, AUTO_ASK_CLASSES};
use crate::app_broker::{execute, Asker};
use crate::app_mcp::AppAccess;
use crate::approvals::Verdict;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// Denies every card, keeping its text.
struct Deny(RefCell<Vec<String>>);

impl Asker for Deny {
    fn ask(&self, card: &app_actions::Card, _caller: &Caller) -> Verdict {
        self.0.borrow_mut().push(card.text());
        Verdict::Denied
    }
}

/// The smallest arguments that pass `validate`: every required parameter,
/// typed.
fn dummy_args(action: &app_actions::Action) -> Value {
    let mut args = Map::new();
    for p in action.params.iter().filter(|p| p.required) {
        let value = match p.kind {
            Kind::Str | Kind::Text | Kind::Secret | Kind::Any => json!("x"),
            Kind::Int(_) | Kind::Num => json!(1),
            Kind::Bool => json!(false),
            Kind::StrList => json!([]),
            Kind::OneOf(values) => json!(values[0]),
            Kind::Object => json!({}),
        };
        args.insert(p.name.to_string(), value);
    }
    Value::Object(args)
}

/// Every file (with its bytes) and folder under `root`, for a before/after
/// comparison.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if path.is_dir() {
                out.insert(rel, None);
                stack.push(path);
            } else {
                let bytes =
                    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                out.insert(rel, Some(bytes));
            }
        }
    }
    out
}

struct Bed {
    _dir: tempfile::TempDir,
    v: PathBuf,
    app: tauri::App<tauri::test::MockRuntime>,
    handle: Option<crate::automated_history::HistoryRunHandle>,
}

impl Bed {
    fn new(surface: &str) -> Bed {
        let dir = tempfile::Builder::new()
            .prefix("tier-audit-")
            .tempdir()
            .unwrap();
        let v = dir.path().to_path_buf();
        let mut config = crate::config::Config::placeholder();
        config.vault_path = v.clone();
        config.claude_projects_path = v.join("no-claude-projects");
        let app = tauri::test::mock_builder()
            .manage(crate::commands::AppState::new(
                config,
                v.join("config.json"),
            ))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let handle = crate::automated_history::try_begin_run(&v, surface, None).unwrap();
        Bed {
            _dir: dir,
            v,
            app,
            handle: Some(handle),
        }
    }

    fn caller(&self, surface: &str, scope: &str, perm: &str) -> Caller {
        Caller {
            run_id: self.handle.as_ref().unwrap().id.clone(),
            surface: surface.into(),
            scope: scope.into(),
            access: AppAccess::Write,
            depth: 0,
            perm: perm.into(),
        }
    }

    fn run(
        &self,
        action: &'static app_actions::Action,
        args: Args,
        caller: Caller,
        asker: &Deny,
    ) -> Result<Value, String> {
        let host = || -> Box<dyn app_actions::Host> { Box::new(self.app.handle().clone()) };
        execute(&host, &self.v, action, args, caller, asker)
    }

    fn finish(&mut self) {
        if let Some(handle) = self.handle.take() {
            crate::automated_history::finish_run(&self.v, handle, "success").unwrap();
        }
    }
}

/// The modes a chat can carry, plus one spelling that isn't canonical (an
/// unknown mode asks for everything).
const MODES: [&str; 6] = [
    "bypassPermissions",
    "auto",
    "acceptEdits",
    "default",
    "plan",
    "Auto",
];

/// Other unknown spellings ask exactly what `Auto` asks (the sweep runs
/// `Auto` through the broker; these only through `asks_in_mode`, which is
/// all that tells modes apart, to keep the gate fast; overnight D7).
#[test]
fn every_unknown_spelling_asks_like_any_other() {
    for action in ACTIONS.iter().filter(|a| a.tier != Tier::Read) {
        for raw in variants(action) {
            let args = app_actions::validate(action, &raw).unwrap();
            let auto = app_actions::asks_in_mode(action, &args, "Auto");
            for mode in ["bypass", "", "BYPASSPERMISSIONS", "dontAsk"] {
                assert_eq!(
                    app_actions::asks_in_mode(action, &args, mode),
                    auto,
                    "{} in `{mode}`",
                    action.id
                );
            }
        }
    }
}

/// The arguments each action is swept with: the dummy ones, and for the
/// write actions whose card depends on their arguments, the call that
/// should ask (a stronger session; a sensitive and a plain setting).
fn variants(action: &app_actions::Action) -> Vec<Value> {
    match action.id {
        "sessions.start" => {
            vec![json!({ "workspace": "vault", "prompt": "hi", "permission": "bypassPermissions" })]
        }
        "settings.change" => vec![
            json!({ "key": "agentTools.app", "value": false }),
            json!({ "key": "agents.defaultAgent", "value": "codex" }),
        ],
        _ => vec![dummy_args(action)],
    }
}

/// The sweep: in every mode, each change the mode must ask about (every
/// confirm and external action, and each write action whose card the call
/// builds) shows exactly one card, or its card builder refuses first, and,
/// denied, changes nothing in the vault. Calls the mode approves are never
/// executed (they would run their real handler). The set that asks is
/// pinned per mode: in bypass exactly the always-ask list; in `auto` the
/// external actions, the auto-ask classes, the always-ask list, a stronger
/// session start, a swarm and a sensitive setting; in any other spelling,
/// everything with a card.
#[test]
fn every_card_the_mode_requires_is_shown_and_a_deny_changes_nothing() {
    let auto_classes: BTreeSet<&str> = AUTO_ASK_CLASSES
        .iter()
        .flat_map(|class| class.ids.iter().copied())
        .collect();
    let always: BTreeSet<&str> = ALWAYS_ASK.iter().copied().collect();
    let confirm_or_external: BTreeSet<&str> = ACTIONS
        .iter()
        .filter(|a| matches!(a.tier, Tier::Confirm | Tier::External))
        .map(|a| a.id)
        .collect();
    let mut executed = 0;
    for mode in MODES {
        let mut bed = Bed::new("chat");
        let caller = bed.caller("chat", "s-20261010-tier-audit", mode);
        // Ids whose call asked: confirm/external, and write ones with a card.
        let (mut asking, mut asking_write) = (BTreeSet::new(), BTreeSet::new());
        let mut write_cards = BTreeSet::new();
        for action in ACTIONS.iter().filter(|a| a.tier != Tier::Read) {
            for raw in variants(action) {
                let args = app_actions::validate(action, &raw)
                    .unwrap_or_else(|e| panic!("{}: sweep arguments rejected: {e}", action.id));
                // What the broker will put on the card, built the way it builds it.
                let cx = app_actions::Cx {
                    host: Box::new(bed.app.handle().clone()),
                    caller: caller.clone(),
                    approval: None,
                };
                let card = app_actions::card_for(action, &cx, &args);
                let gated = action.tier != Tier::Write;
                assert!(
                    !(gated && matches!(card, Ok(None))),
                    "{}: a {} action always has a card",
                    action.id,
                    action.tier.as_str()
                );
                if matches!(card, Ok(None)) {
                    continue;
                }
                if !gated && card.is_ok() {
                    write_cards.insert(action.id);
                }
                let asks = app_actions::asks_in_mode(action, &args, mode);
                if gated && asks {
                    asking.insert(action.id);
                }
                if !gated && asks && card.is_ok() {
                    asking_write.insert(action.id);
                }
                // A call the mode approves would run its handler: never here.
                if card.is_ok() && !asks {
                    continue;
                }
                let before = snapshot(&bed.v);
                let asker = Deny(RefCell::new(Vec::new()));
                let result = bed.run(action, args, caller.clone(), &asker);
                let cards = asker.0.borrow().len();
                if card.is_ok() {
                    assert_eq!(
                        cards, 1,
                        "{} in `{mode}`: the card must be shown once",
                        action.id
                    );
                    let error = result.expect_err(action.id);
                    assert!(
                        error.contains("declined"),
                        "{} in `{mode}`: {error}",
                        action.id
                    );
                } else {
                    assert_eq!(
                        cards, 0,
                        "{} in `{mode}`: refused before any card",
                        action.id
                    );
                    assert!(
                        result.is_err(),
                        "{} in `{mode}` ran after its card refused",
                        action.id
                    );
                }
                assert!(
                    snapshot(&bed.v) == before,
                    "{} in `{mode}`: a denied or refused call changed the vault",
                    action.id
                );
                executed += 1;
            }
        }
        match mode {
            "bypassPermissions" => {
                assert_eq!(asking, always, "bypass asks only the always-ask list");
                assert!(asking_write.is_empty(), "bypass: {asking_write:?}");
            }
            "auto" => {
                let expected: BTreeSet<&str> = confirm_or_external
                    .iter()
                    .copied()
                    .filter(|id| {
                        app_actions::find(id).unwrap().tier == Tier::External
                            || auto_classes.contains(id)
                            || always.contains(id)
                    })
                    .collect();
                assert_eq!(asking, expected, "what `auto` asks about");
                assert_eq!(
                    asking_write,
                    BTreeSet::from(["sessions.start", "sessions.swarm", "settings.change"]),
                    "the write calls `auto` asks about"
                );
            }
            _ => {
                assert_eq!(
                    asking, confirm_or_external,
                    "`{mode}` asks about every confirm and external action"
                );
                assert_eq!(
                    asking_write, write_cards,
                    "`{mode}` asks about every write call with a card"
                );
            }
        }
        bed.finish();
    }
    assert!(executed > 300, "the sweep ran {executed} calls");
}

/// From an automation, a confirm or external action that its stamped
/// allowed tools don't name is refused before any card, in every mode, and
/// nothing changes; an external one is refused even when the stamp names it.
#[test]
fn an_automation_reaches_no_card_and_no_external_action() {
    let mut bed = Bed::new("workflow");
    for mode in ["bypassPermissions", "auto", "acceptEdits"] {
        let caller = bed.caller("workflow", "tier-audit-flow", mode);
        for action in ACTIONS
            .iter()
            .filter(|a| matches!(a.tier, Tier::Confirm | Tier::External))
        {
            let args = app_actions::validate(action, &dummy_args(action)).unwrap();
            let before = snapshot(&bed.v);
            let asker = Deny(RefCell::new(Vec::new()));
            let error = bed
                .run(action, args, caller.clone(), &asker)
                .expect_err(action.id);
            assert!(
                asker.0.borrow().is_empty(),
                "{}: an automation can't ask",
                action.id
            );
            assert!(
                error.contains("automation") || error.contains("can't run it"),
                "{} in `{mode}`: {error}",
                action.id
            );
            assert!(
                snapshot(&bed.v) == before,
                "{}: refused, yet the vault changed",
                action.id
            );
        }
    }
    // Named in the automation's allowed tools, every external action is
    // still refused, without a card.
    let caller = bed.caller("workflow", "tier-audit-flow", "bypassPermissions");
    let external: Vec<&app_actions::Action> = ACTIONS
        .iter()
        .filter(|a| a.tier == Tier::External)
        .collect();
    assert!(!external.is_empty());
    let named: Vec<String> = external
        .iter()
        .flat_map(|a| {
            let tool = crate::app_mcp::tool_for(a.id).unwrap_or(a.id);
            [
                format!("mcp__hollow_app__{tool}"),
                format!("mcp__hollow_app__{}", a.id),
            ]
        })
        .collect();
    crate::app_broker::register_workflow_run(
        &caller.run_id,
        &named,
        crate::tasks_mcp::TasksAccess::ReadWrite,
    );
    for action in external {
        assert!(
            crate::app_broker::opted_in(&caller, action),
            "{}: the stamp names it",
            action.id
        );
        let args = app_actions::validate(action, &dummy_args(action)).unwrap();
        let asker = Deny(RefCell::new(Vec::new()));
        let error = bed
            .run(action, args, caller.clone(), &asker)
            .expect_err(action.id);
        assert!(asker.0.borrow().is_empty(), "{}", action.id);
        assert!(
            error.contains("leaves this Mac") || error.contains("can't run it"),
            "{}: {error}",
            action.id
        );
    }
    bed.finish();
}

/// The Main agent's unattended allow list: each id is a write action (the
/// broker skips its card when nobody is watching, so a confirm one would run
/// unasked), none is always-ask or in an auto-ask class, and the one that
/// can ask, a session start in a stronger mode, is refused above the cap.
#[test]
fn the_unattended_allow_list_holds_no_action_that_asks() {
    let auto_classes: BTreeSet<&str> = AUTO_ASK_CLASSES
        .iter()
        .flat_map(|class| class.ids.iter().copied())
        .collect();
    for id in crate::main_agent::unattended::SAFE {
        let action = app_actions::find(id).unwrap_or_else(|| panic!("{id} is not an action"));
        assert_eq!(action.tier, Tier::Write, "{id}");
        assert!(!app_actions::always_asks(id), "{id}");
        assert!(!auto_classes.contains(id), "{id}");
    }
    let dir = tempfile::tempdir().unwrap();
    let cap = crate::main_agent::UNATTENDED_START_CAP;
    for mode in crate::app_mcp::PERMISSION_MODES {
        let args = Map::from_iter([
            ("workspace".to_string(), json!("some-workspace")),
            ("prompt".to_string(), json!("hi")),
            ("permission".to_string(), json!(mode)),
        ]);
        let result =
            crate::main_agent::unattended::allows(dir.path(), "sessions.start", false, &args, true);
        if crate::app_mcp::perm_rank(mode) > crate::app_mcp::perm_rank(cap) {
            assert!(result.is_err(), "{mode} is above the unattended cap");
        }
    }
}

/// A user-only command is the person's click: no agent-facing module calls
/// it. The registry side (never an action, never a guessable id) is in
/// `app_actions.rs`; this is the other half, the handlers and tool servers.
/// It sees direct calls written `name(` in those modules' production code,
/// not a command passed as a value or the inner helper a command calls.
#[test]
fn no_agent_tool_calls_a_user_only_command() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files: Vec<PathBuf> = [
        "app_actions.rs",
        "app_broker.rs",
        "main_agent.rs",
        "night_queue.rs",
        "workflows.rs",
    ]
    .iter()
    .map(|f| src.join(f))
    .collect();
    for entry in std::fs::read_dir(&src).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.ends_with("_mcp.rs") {
            files.push(entry.path());
        }
    }
    let mut scanned = 0;
    for file in &files {
        let text = std::fs::read_to_string(file).unwrap();
        // Production code only: tests may call anything. A test module is
        // inline (`mod tests {`) or in its own file (`#[path = …]`, B11–B13);
        // cut at whichever comes first.
        let code = ["#[cfg(test)]\nmod tests", "#[cfg(test)]\n#[path"]
            .iter()
            .filter_map(|marker| text.find(marker))
            .min()
            .map_or(text.as_str(), |at| &text[..at]);
        for line in code.lines() {
            let line = line.trim_start();
            if line.starts_with("//") {
                continue;
            }
            for name in app_actions::USER_ONLY {
                let call = format!("{name}(");
                let mut from = 0;
                while let Some(at) = line[from..].find(&call) {
                    let start = from + at;
                    let before = line[..start].chars().last();
                    let is_ident = before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
                    let is_def = line[..start].trim_end().ends_with("fn");
                    assert!(
                        is_ident || is_def,
                        "{} calls the user-only command {name}: {line}",
                        file.display()
                    );
                    from = start + call.len();
                }
            }
        }
        scanned += 1;
    }
    assert!(scanned >= 12, "{scanned}");
}

/// Approves every card, keeping its text.
struct Allow(RefCell<Vec<String>>);

impl Asker for Allow {
    fn ask(&self, card: &app_actions::Card, _caller: &Caller) -> Verdict {
        self.0.borrow_mut().push(card.text());
        Verdict::Allowed
    }
}

/// G1: `workspaces.edit_standing_orders` through the broker in Bypass: the
/// card shows the line before and after, a deny changes nothing, an approval
/// edits exactly that line, a bad call is refused before any card, and a bad
/// argument never validates.
#[test]
fn the_coo_edits_a_standing_orders_line_only_from_a_card() {
    let mut bed = Bed::new("chat");
    crate::workspaces::main_agent_workspace(&bed.v).unwrap();
    let path = bed.v.join("workspaces/main/STANDING_ORDERS.md");
    let seed = std::fs::read_to_string(&path).unwrap();
    let action = app_actions::find("workspaces.edit_standing_orders").unwrap();
    let caller = bed.caller("chat", "s-20261115-g1", "bypassPermissions");
    let old = "- Create or update tasks.";
    let raw = json!({ "op": "replace", "before": old, "line": "- Create tasks, and update the ones it made." });
    let args = || app_actions::validate(action, &raw).unwrap();
    let deny = Deny(RefCell::new(Vec::new()));
    let denied = bed.run(action, args(), caller.clone(), &deny).unwrap_err();
    assert!(denied.contains("declined"), "{denied}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), seed);
    let card = deny.0.borrow()[0].clone();
    assert!(card.contains(&format!("Before: {old}")), "{card}");
    assert!(
        card.contains("After: - Create tasks, and update the ones it made."),
        "{card}"
    );
    let allow = Allow(RefCell::new(Vec::new()));
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let done = execute(&host, &bed.v, action, args(), caller.clone(), &allow).unwrap();
    assert_eq!(allow.0.borrow().len(), 1);
    assert_eq!(done["before"], json!(old));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        seed.replace(old, "- Create tasks, and update the ones it made.")
    );
    // Add and remove show their own before and after.
    let add = app_actions::validate(
        action,
        &json!({ "op": "add", "line": "- Never email anyone." }),
    )
    .unwrap();
    let deny = Deny(RefCell::new(Vec::new()));
    assert!(bed.run(action, add, caller.clone(), &deny).is_err());
    let card = deny.0.borrow()[0].clone();
    assert!(
        card.contains("Before: (none, a new line)")
            && card.contains("After: - Never email anyone."),
        "{card}"
    );
    let rm = app_actions::validate(
        action,
        &json!({ "op": "remove", "before": "- Create tasks, and update the ones it made." }),
    )
    .unwrap();
    let deny = Deny(RefCell::new(Vec::new()));
    assert!(bed.run(action, rm, caller.clone(), &deny).is_err());
    let card = deny.0.borrow()[0].clone();
    assert!(card.contains("After: (removed)"), "{card}");
    // A line that isn't there is refused before a card is shown.
    let miss = app_actions::validate(action, &json!({ "op": "remove", "before": "- not a line" }))
        .unwrap();
    let deny = Deny(RefCell::new(Vec::new()));
    assert!(bed.run(action, miss, caller, &deny).is_err());
    assert!(deny.0.borrow().is_empty());
    // Arguments are validated: an unknown op, a stray key, a non-number index.
    for bad in [
        json!({ "op": "wipe" }),
        json!({ "op": "add", "line": "x", "path": "../x" }),
        json!({ "op": "add", "line": "x", "index": "two" }),
        json!({}),
    ] {
        assert!(app_actions::validate(action, &bad).is_err(), "{bad}");
    }
    bed.finish();
}
