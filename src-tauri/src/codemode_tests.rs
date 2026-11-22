//! CM1a: the code-mode sandbox's limits and ambient-power checks against a
//! fake host, and the broker route against the real `app_broker::execute`
//! (Always-ask stays Always-ask in Bypass, Read runs without a card, a
//! read-only run can't write, every inner call is audited).

use super::*;
use crate::app_actions::Card;
use crate::app_mcp::AppAccess;
use std::cell::RefCell;
use std::path::PathBuf;

/// A fake host: `echo` returns its arguments, `big` returns `n` bytes of
/// text, `fail` refuses, `count` counts; everything else is unknown.
#[derive(Default)]
struct Fake {
    seen: RefCell<Vec<(String, Value)>>,
}

impl ToolHost for Fake {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        self.seen
            .borrow_mut()
            .push((tool.to_string(), args.clone()));
        match tool {
            "echo" => Ok(args),
            "big" => {
                let n = args["n"].as_u64().unwrap_or(0) as usize;
                Ok(json!({ "text": "y".repeat(n) }))
            }
            "fail" => Err("fail refused: nothing ran.".into()),
            "count" => Ok(json!(self.seen.borrow().len())),
            other => Err(format!("unknown tool `{other}`")),
        }
    }

    fn tools(&self) -> Vec<ToolInfo> {
        ["echo", "big", "fail", "count", "codemode.run"]
            .iter()
            .map(|n| ToolInfo {
                name: n.to_string(),
                tier: "read".into(),
                summary: String::new(),
                signature: String::new(),
            })
            .collect()
    }
}

fn quick() -> Limits {
    Limits {
        // Generous: these tests aren't about the deadline, and must not
        // flake when two lanes build at once.
        cpu: Duration::from_secs(2),
        ..Limits::default()
    }
}

fn ok(script: &str) -> Value {
    let report = run(script, &Fake::default(), &quick());
    match report.outcome {
        Ok(value) => value,
        Err(error) => panic!("{script}: {error}"),
    }
}

fn err(script: &str) -> RunError {
    err_with(script, &quick())
}

fn err_with(script: &str, limits: &Limits) -> RunError {
    match run(script, &Fake::default(), limits).outcome {
        Ok(value) => panic!("{script} returned {value}"),
        Err(error) => error,
    }
}

#[test]
fn a_result_round_trips_as_json_and_tools_return_parsed_values() {
    assert_eq!(
        ok(r#"return { a: 1, b: [true, null, "x"], c: { d: 2.5 } };"#),
        json!({ "a": 1, "b": [true, null, "x"], "c": { "d": 2.5 } })
    );
    assert_eq!(ok("const x = 1;"), Value::Null, "no return gives null");
    assert_eq!(ok("return undefined;"), Value::Null);
    let fake = Fake::default();
    let report = run(
        r#"const a = tools.echo({ q: "hi", n: [1, 2] });
           const b = await callTool("echo", { q: a.q + "!" });
           console.log("got", b.q, { n: a.n });
           return [a, b, Object.keys(tools)];"#,
        &fake,
        &quick(),
    );
    assert_eq!(
        report.outcome.clone().unwrap(),
        json!([
            { "q": "hi", "n": [1, 2] },
            { "q": "hi!" },
            ["echo", "big", "fail", "count"]
        ]),
        "the nested code-mode tool is never listed"
    );
    assert_eq!(report.console, vec![r#"got hi! {"n":[1,2]}"#.to_string()]);
    assert_eq!(report.calls.len(), 2);
    assert!(report.calls.iter().all(|c| c.ok && !c.truncated));
    assert_eq!(fake.seen.borrow()[1].1, json!({ "q": "hi!" }));
    let shown = report.to_json();
    assert_eq!(shown["ok"], json!(true));
    assert_eq!(shown["calls"].as_array().unwrap().len(), 2);
}

#[test]
fn a_refusal_throws_in_the_script_and_can_be_caught() {
    assert_eq!(
        ok(r#"try { tools.fail({}); return "ran"; } catch (e) { return e.message; }"#),
        json!("fail refused: nothing ran.")
    );
    match err("tools.fail({});") {
        RunError::Thrown { message, line } => {
            assert!(message.contains("fail refused"), "{message}");
            assert_eq!(line, Some(1));
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_throw_is_an_error_with_its_line() {
    match err("const a = 1;\nconst b = 2;\nthrow new TypeError(\"bad \" + (a + b));") {
        RunError::Thrown { message, line } => {
            assert_eq!(message, "TypeError: bad 3");
            assert_eq!(line, Some(3));
        }
        other => panic!("{other:?}"),
    }
    match err(r#"throw "plain";"#) {
        RunError::Thrown { message, .. } => assert_eq!(message, "plain"),
        other => panic!("{other:?}"),
    }
    match err("await Promise.reject(new Error(\"later\"));") {
        RunError::Thrown { message, .. } => assert!(message.contains("later"), "{message}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_syntax_error_is_reported_with_its_line() {
    match err("const a = 1;\nconst b = ;\nreturn a;") {
        RunError::Syntax { message, line } => {
            assert!(!message.is_empty());
            assert_eq!(line, Some(2), "{message}");
        }
        other => panic!("{other:?}"),
    }
    let shown = run("return (", &Fake::default(), &quick()).to_json();
    assert_eq!(shown["error"]["code"], json!("syntax"));
}

#[test]
fn an_infinite_loop_stops_at_the_cpu_deadline_and_cannot_be_caught() {
    let limits = Limits {
        cpu: Duration::from_millis(200),
        ..Limits::default()
    };
    let began = Instant::now();
    assert_eq!(
        err_with("while (true) {}", &limits),
        RunError::CpuDeadline {
            limit: Duration::from_millis(200)
        }
    );
    // A catch around the loop doesn't swallow the interrupt.
    assert!(matches!(
        err_with("for (;;) { try { while (true) {} } catch (e) {} }", &limits),
        RunError::CpuDeadline { .. }
    ));
    // Nor does a promise job loop the script waits behind.
    let spin = err_with(
        "const spin = () => Promise.resolve().then(spin); spin(); await new Promise(() => {});",
        &limits,
    );
    assert!(matches!(spin, RunError::CpuDeadline { .. }), "{spin:?}");
    // Jobs still queued once the body has returned are dropped with the
    // runtime, not run.
    assert_eq!(
        ok("const spin = () => Promise.resolve().then(spin); spin(); return 1;"),
        json!(1)
    );
    // A wait on nothing (no timers, no I/O) ends at once with a clear error.
    match err("await new Promise(() => {});") {
        RunError::Thrown { message, .. } => assert!(message.contains("never settled"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert!(
        began.elapsed() < Duration::from_secs(30),
        "no hang: {:?}",
        began.elapsed()
    );
}

/// Time inside a host call (a card waiting) doesn't count toward the CPU
/// deadline; the overall cap still stops the run.
#[test]
fn host_time_is_outside_the_cpu_deadline_but_inside_the_overall_cap() {
    struct Slow(Duration);
    impl ToolHost for Slow {
        fn call(&self, _tool: &str, _args: Value) -> Result<Value, String> {
            std::thread::sleep(self.0); // sleep-ok: stands in for a card the person answers
            Ok(json!(true))
        }
        fn tools(&self) -> Vec<ToolInfo> {
            Vec::new()
        }
    }
    let limits = Limits {
        cpu: Duration::from_millis(100),
        overall: Duration::from_secs(5),
        ..Limits::default()
    };
    let report = run(
        "callTool('wait'); callTool('wait'); return 'done';",
        &Slow(Duration::from_millis(150)),
        &limits,
    );
    assert_eq!(report.outcome, Ok(json!("done")));
    // The 300 ms in the host is outside the script's own time.
    assert!(
        report.script_ms + 250 <= report.wall_ms,
        "{} of {}",
        report.script_ms,
        report.wall_ms
    );
    let limits = Limits {
        cpu: Duration::from_secs(5),
        overall: Duration::from_millis(100),
        ..Limits::default()
    };
    let report = run(
        "try { callTool('wait'); } catch (e) {} callTool('wait'); return 'ran on';",
        &Slow(Duration::from_millis(150)),
        &limits,
    );
    assert!(
        matches!(report.outcome, Err(RunError::OverallDeadline { .. })),
        "{:?}",
        report.outcome
    );
    assert_eq!(report.calls.len(), 1, "nothing runs after the cap");
}

#[test]
fn a_memory_bomb_stops_at_the_memory_limit() {
    let limits = Limits {
        memory_bytes: 8 << 20,
        cpu: Duration::from_secs(5),
        ..Limits::default()
    };
    // QuickJS-ng concatenates as ropes, so doubling with `+` hits the
    // engine's string length cap before the memory limit: stopped all the same.
    match err_with("let s = 'x'; while (true) s = s + s;", &limits) {
        RunError::Memory { .. } => {}
        RunError::Thrown { message, .. } => assert!(message.contains("too long"), "{message}"),
        other => panic!("{other:?}"),
    }
    for bomb in [
        "let s = 'xy'; for (;;) s = s.repeat(2);",
        "let s = 'x'; for (;;) { s = s + s; s.charCodeAt(s.length - 1); s = s.split('').join(''); }",
        "const a = []; while (true) a.push({ n: a.length, s: 'abcdefgh' + a.length });",
        "const a = []; while (true) a.push(new Array(10000).fill(1));",
    ] {
        let error = err_with(bomb, &limits);
        assert!(
            matches!(error, RunError::Memory { .. }),
            "{bomb}: {error:?}"
        );
    }
    // A caught OOM that keeps allocating still ends, at the deadline at worst.
    let limits = Limits {
        memory_bytes: 8 << 20,
        cpu: Duration::from_millis(300),
        ..Limits::default()
    };
    let error = err_with(
        "for (;;) { try { let s = 'x'; while (true) s = s + s; } catch (e) {} }",
        &limits,
    );
    assert!(
        matches!(
            error,
            RunError::Memory { .. } | RunError::CpuDeadline { .. }
        ),
        "{error:?}"
    );
}

#[test]
fn deep_recursion_is_a_stack_overflow_not_a_crash() {
    assert_eq!(
        err("function f(n) { return f(n + 1) + 1; } return f(0);"),
        RunError::StackOverflow
    );
}

#[test]
fn the_call_count_is_capped_and_the_cap_cannot_be_caught() {
    let limits = Limits {
        max_calls: 3,
        ..quick()
    };
    let fake = Fake::default();
    let report = run(
        "for (let i = 0; i < 10; i++) { try { tools.count(); } catch (e) {} } return 'finished';",
        &fake,
        &limits,
    );
    assert_eq!(report.outcome, Err(RunError::TooManyCalls { max: 3 }));
    assert_eq!(fake.seen.borrow().len(), 3, "the fourth call never ran");
    assert_eq!(report.calls.len(), 3);
}

#[test]
fn an_oversize_inner_result_is_cut_and_flagged_and_an_oversize_output_is_refused() {
    let limits = Limits {
        max_call_result_bytes: 1000,
        max_output_bytes: 2000,
        ..quick()
    };
    let report = run(
        "const r = tools.big({ n: 5000 }); return { truncated: r.truncated, bytes: r.bytes, head: r.preview.length };",
        &Fake::default(),
        &limits,
    );
    assert_eq!(
        report.outcome,
        Ok(json!({ "truncated": true, "bytes": 5011, "head": 1000 }))
    );
    assert!(report.calls[0].truncated);
    let small = run(
        "return tools.big({ n: 10 }).text;",
        &Fake::default(),
        &limits,
    );
    assert_eq!(small.outcome, Ok(json!("y".repeat(10))));
    assert!(!small.calls[0].truncated);
    assert_eq!(
        err_with("return 'z'.repeat(5000);", &limits),
        RunError::OutputTooLarge {
            bytes: 5002,
            max: 2000
        }
    );
    // A script over its own cap never runs.
    let limits = Limits {
        max_script_bytes: 10,
        ..quick()
    };
    assert!(matches!(
        err_with("return 'this is long';", &limits),
        RunError::ScriptTooLarge { .. }
    ));
}

#[test]
fn a_result_that_is_not_json_is_an_error() {
    assert!(matches!(
        err("const a = {}; a.self = a; return a;"),
        RunError::NotJson(_)
    ));
    assert!(matches!(err("return 10n;"), RunError::NotJson(_)));
    // A function or symbol has no JSON: null, as JSON.stringify says.
    assert_eq!(ok("return () => 1;"), Value::Null);
}

#[test]
fn the_console_is_bounded() {
    let limits = Limits {
        max_console_bytes: 100,
        ..quick()
    };
    let report = run(
        "for (let i = 0; i < 100; i++) console.log('line', i); return 1;",
        &Fake::default(),
        &limits,
    );
    assert_eq!(report.outcome, Ok(json!(1)));
    assert!(report.console_truncated);
    assert!(report.console.len() < 20, "{}", report.console.len());
    assert_eq!(report.console[0], "line 0");
    assert_eq!(report.to_json()["consoleTruncated"], json!(true));
}

#[test]
fn no_ambient_power_is_reachable() {
    for name in REMOVED_GLOBALS
        .iter()
        .chain(["Performance", "import"].iter())
    {
        let script = format!("return typeof globalThis[{name:?}];");
        assert_eq!(ok(&script), json!("undefined"), "{name}");
    }
    // Every global the script can see, for the record of what is there.
    let seen = ok("return Object.getOwnPropertyNames(globalThis).sort();");
    let seen: Vec<&str> = seen
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for absent in [
        "fetch",
        "setTimeout",
        "process",
        "require",
        "WebAssembly",
        "std",
        "os",
    ] {
        assert!(!seen.contains(&absent), "{absent} in {seen:?}");
    }
    assert!(seen.contains(&"tools") && seen.contains(&"callTool"));
    // Dynamic import has no loader: it rejects, whatever the name.
    for spec in ["os", "std", "./x.js", "https://example.com/x.js"] {
        let script = format!(
            "try {{ await import({spec:?}); return 'loaded'; }} catch (e) {{ return 'refused'; }}"
        );
        assert_eq!(ok(&script), json!("refused"), "{spec}");
    }
    // eval and Function stay, and reach nothing more.
    assert_eq!(ok("return eval('typeof fetch');"), json!("undefined"));
    assert_eq!(
        ok("return new Function('return typeof setTimeout')();"),
        json!("undefined")
    );
    // The injected bindings can't be replaced by the script.
    assert_eq!(
        ok("try { tools = null; } catch (e) {} try { tools.echo = null; } catch (e) {} return typeof tools.echo;"),
        json!("function")
    );
}

#[test]
fn a_script_cannot_start_a_nested_run() {
    for name in ["codemode.run", "run_code", "mcp__hollow_app__run_code"] {
        let fake = Fake::default();
        let script = format!(
            "try {{ callTool({name:?}, {{ script: 'return 1' }}); return 'ran'; }} catch (e) {{ return e.message; }}"
        );
        let report = run(&script, &fake, &quick());
        let message = report.outcome.unwrap();
        assert!(
            message.as_str().unwrap().contains("can't start another"),
            "{name}: {message}"
        );
        assert!(fake.seen.borrow().is_empty(), "{name} reached the host");
    }
    // A host that re-enters `run` on the same thread is refused too.
    struct Reenter;
    impl ToolHost for Reenter {
        fn call(&self, _tool: &str, _args: Value) -> Result<Value, String> {
            match run("return 1;", &Fake::default(), &Limits::default()).outcome {
                Err(RunError::Nested) => Err("nested refused".into()),
                other => Ok(json!(format!("{other:?}"))),
            }
        }
        fn tools(&self) -> Vec<ToolInfo> {
            Vec::new()
        }
    }
    let report = run("return callTool('inner');", &Reenter, &quick());
    match report.outcome {
        Err(RunError::Thrown { message, .. }) => assert!(message.contains("nested refused")),
        other => panic!("{other:?}"),
    }
    // Once the outer run is over, a new run works.
    assert_eq!(ok("return 2;"), json!(2));
}

#[test]
fn two_runs_share_no_state() {
    assert_eq!(
        ok("globalThis.leak = 41; Object.prototype.polluted = 1; return 1;"),
        json!(1)
    );
    assert_eq!(
        ok("return [typeof leak, typeof ({}).polluted];"),
        json!(["undefined", "undefined"])
    );
}

// ── The broker route ─────────────────────────────────────────────────────

/// A test asker: always this verdict, keeping each card's text.
struct Recording(Verdict, RefCell<Vec<String>>);

impl Asker for Recording {
    fn ask(&self, card: &Card, _caller: &Caller) -> Verdict {
        self.1.borrow_mut().push(card.text());
        self.0
    }
}

/// A mock app on a temp vault with a live chat run (the
/// `tier_audit_tests` bed).
struct Bed {
    _dir: tempfile::TempDir,
    v: PathBuf,
    app: tauri::App<tauri::test::MockRuntime>,
    handle: Option<crate::automated_history::HistoryRunHandle>,
}

impl Bed {
    fn new() -> Bed {
        let dir = tempfile::Builder::new()
            .prefix("codemode-")
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
        let handle = crate::automated_history::try_begin_run(&v, "chat", None).unwrap();
        Bed {
            _dir: dir,
            v,
            app,
            handle: Some(handle),
        }
    }

    fn caller(&self, access: AppAccess, perm: &str) -> Caller {
        Caller {
            run_id: self.handle.as_ref().unwrap().id.clone(),
            surface: "chat".into(),
            scope: "s-20261117-codemode".into(),
            access,
            depth: 0,
            perm: perm.into(),
        }
    }

    /// Run `script` through a [`BrokerToolHost`] for `caller`.
    fn run(&self, script: &str, caller: Caller, asker: &dyn Asker) -> RunReport {
        let host = || -> Box<dyn app_actions::Host> { Box::new(self.app.handle().clone()) };
        let broker =
            BrokerToolHost::new(&host, &self.v, caller, asker, &app_broker::APP_POLICY, true);
        run(script, &broker, &quick())
    }

    /// The code-mode lines of the audit log.
    fn audit(&self) -> Vec<Value> {
        std::fs::read_to_string(self.v.join(app_broker::AUDIT_FILE))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            .filter(|l| l["via"] == json!("codemode"))
            .collect()
    }

    fn finish(&mut self) {
        if let Some(handle) = self.handle.take() {
            crate::automated_history::finish_run(&self.v, handle, "success").unwrap();
        }
    }
}

/// CM1a's acceptance: an Always-ask action called from a script in a
/// `bypassPermissions` chat (standing approvals on) still shows its card;
/// on Deny the script sees the refusal as a thrown error and the handler
/// never ran (the standing orders file is byte-for-byte unchanged). The
/// call is audited as an inner code-mode call with the card's answer.
#[test]
fn an_always_ask_action_still_asks_in_bypass_and_a_deny_runs_nothing() {
    let mut bed = Bed::new();
    crate::workspaces::main_agent_workspace(&bed.v).unwrap();
    let path = bed.v.join("workspaces/main/STANDING_ORDERS.md");
    let seed = std::fs::read_to_string(&path).unwrap();
    let id = "workspaces.edit_standing_orders";
    assert!(app_actions::always_asks(id));
    let deny = Recording(Verdict::Denied, RefCell::new(Vec::new()));
    assert!(deny.standing_approvals(), "standing approvals are on");
    let caller = bed.caller(AppAccess::Write, "bypassPermissions");
    let script = format!(
        r#"try {{
             tools[{id:?}]({{ op: "add", line: "- Never email anyone." }});
             return "ran";
           }} catch (e) {{ return "refused: " + e.message; }}"#
    );
    let report = bed.run(&script, caller, &deny);
    // The Deny ends the run: the script's catch never gets to go on.
    match &report.outcome {
        Err(RunError::CardRefused { message }) => {
            assert!(message.starts_with("The user declined"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(deny.1.borrow().len(), 1, "Bypass did not skip the card");
    assert!(
        deny.1.borrow()[0].contains("After: - Never email anyone."),
        "{}",
        deny.1.borrow()[0]
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        seed,
        "the handler never ran"
    );
    assert_eq!(report.calls.len(), 1);
    assert!(!report.calls[0].ok);
    let audit = bed.audit();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["action"], json!(id));
    assert_eq!(audit[0]["tool"], json!("codemode"));
    assert_eq!(audit[0]["ok"], json!(false));
    assert_eq!(audit[0]["card"], json!(Verdict::Denied.reason()));
    assert_eq!(audit[0]["perm"], json!("bypassPermissions"));
    assert_eq!(audit[0]["tier"], json!("confirm"));
    bed.finish();
}

/// A Read action runs with no card, and its audit line says ok with no
/// card; a second call in the same script gets its own line.
#[test]
fn a_read_action_runs_without_a_card_and_is_audited() {
    let mut bed = Bed::new();
    std::fs::create_dir_all(bed.v.join("notes")).unwrap();
    std::fs::write(bed.v.join("notes/plan.md"), "# Plan\n\nShip CM1a.\n").unwrap();
    let ask = Recording(Verdict::Denied, RefCell::new(Vec::new()));
    let caller = bed.caller(AppAccess::Write, "default");
    let report = bed.run(
        r#"const note = tools["vault.vault_note"]({ path: "notes/plan.md" });
           const again = callTool("vault.vault_note", { path: "notes/plan.md" });
           return { has: JSON.stringify(note).includes("Ship CM1a."), same: JSON.stringify(note) === JSON.stringify(again) };"#,
        caller,
        &ask,
    );
    assert_eq!(report.outcome, Ok(json!({ "has": true, "same": true })));
    assert!(ask.1.borrow().is_empty(), "a read shows no card");
    let audit = bed.audit();
    assert_eq!(audit.len(), 2, "{audit:?}");
    for line in &audit {
        assert_eq!(line["action"], json!("vault.vault_note"));
        assert_eq!(line["ok"], json!(true));
        assert_eq!(line["card"], Value::Null);
        assert_eq!(line["tier"], json!("read"));
    }
    bed.finish();
}

/// A Write action from a read-only (plan mode) run is refused before
/// anything runs, the script sees why, and the refusal is audited. A
/// read-only run's tool list has no Write action in it.
#[test]
fn a_write_action_in_a_read_only_run_is_refused_and_audited() {
    let mut bed = Bed::new();
    let ask = Recording(Verdict::Allowed, RefCell::new(Vec::new()));
    let caller = bed.caller(AppAccess::Read, "plan");
    let report = bed.run(
        r#"const listed = "goals.create_goal" in tools;
           try { callTool("goals.create_goal", { title: "x" }); return "ran"; }
           catch (e) { return { listed, refused: e.message }; }"#,
        caller.clone(),
        &ask,
    );
    let value = report.outcome.unwrap();
    assert_eq!(value["listed"], json!(false));
    assert!(
        value["refused"]
            .as_str()
            .unwrap()
            .contains("can only read it"),
        "{value}"
    );
    assert!(ask.1.borrow().is_empty());
    assert!(
        !bed.v.join("goals").exists()
            || std::fs::read_dir(bed.v.join("goals"))
                .unwrap()
                .next()
                .is_none()
    );
    let audit = bed.audit();
    assert_eq!(audit.len(), 1, "{audit:?}");
    assert_eq!(audit[0]["action"], json!("goals.create_goal"));
    assert_eq!(audit[0]["ok"], json!(false));
    assert!(audit[0]["error"]
        .as_str()
        .unwrap()
        .contains("can only read it"));
    // The broker host lists only what this access allows, never code mode.
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let broker = BrokerToolHost::new(&host, &bed.v, caller, &ask, &app_broker::APP_POLICY, true);
    let tools = broker.tools();
    assert!(!tools.is_empty());
    assert!(
        tools.iter().all(|t| t.tier == "read"),
        "read-only lists reads"
    );
    assert!(tools.iter().any(|t| t.name == "vault.vault_note"));
    assert!(tools.iter().all(|t| !is_nested_tool(&t.name)));
    // A nested code-mode id through the broker host is refused and audited.
    assert!(broker
        .call("codemode.run", json!({ "script": "return 1" }))
        .unwrap_err()
        .contains("can't start another"));
    assert_eq!(bed.audit().len(), 2);
    bed.finish();
}

/// With Hollow app tools switched off, an inner call is refused like a
/// direct one.
#[test]
fn the_switch_applies_to_inner_calls() {
    let mut bed = Bed::new();
    let ask = Recording(Verdict::Allowed, RefCell::new(Vec::new()));
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let broker = BrokerToolHost::new(
        &host,
        &bed.v,
        bed.caller(AppAccess::Write, "bypassPermissions"),
        &ask,
        &app_broker::APP_POLICY,
        false,
    );
    let refused = broker.call("vault.vault_tree", json!({})).unwrap_err();
    assert!(refused.contains("switched off"), "{refused}");
    bed.finish();
}

// ── Review findings (CM1a review, 2026-11-17) ────────────────────────────

/// Runs `script` on its own thread and fails, instead of hanging the suite,
/// if it hasn't finished within a wide no-hang bound.
fn run_bounded(script: &'static str, limits: Limits) -> RunReport {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(move || {
            let _ = tx.send(run(script, &Fake::default(), &limits));
        })
        .unwrap();
    rx.recv_timeout(Duration::from_secs(60))
        .unwrap_or_else(|_| panic!("no hang: {script} still running after 60 s"))
}

/// Blocker 1: a throwing (or endless) `toJSON` inside console.log must not
/// have its interrupt swallowed by the console, or the loop runs forever.
#[test]
fn a_looping_to_json_in_console_log_still_stops_at_the_deadline() {
    let limits = Limits {
        cpu: Duration::from_millis(300),
        ..Limits::default()
    };
    let report = run_bounded(
        "for (;;) console.log({ toJSON() { for (;;) {} } });",
        limits,
    );
    assert!(
        matches!(report.outcome, Err(RunError::CpuDeadline { .. })),
        "{:?}",
        report.outcome
    );
    // A toJSON that throws reaches the script as an exception it can see.
    assert_eq!(
        ok("try { console.log({ toJSON() { throw new Error('nope'); } }); return 'logged'; } catch (e) { return e.message; }"),
        json!("nope")
    );
    assert_eq!(ok("console.log(undefined, () => 1); return 1;"), json!(1));
}

/// Major 2: a huge tool name is refused before it is recorded or sent, so
/// the report stays small.
#[test]
fn a_huge_tool_name_is_refused_unrecorded() {
    let fake = Fake::default();
    let report = run(
        "const n = 'x'.repeat(8e6); let k = 0; for (let i = 0; i < 50; i++) { try { callTool(n); } catch (e) { k++; } } return k;",
        &fake,
        &quick(),
    );
    assert_eq!(report.outcome, Ok(json!(50)));
    assert!(report.calls.is_empty());
    assert!(fake.seen.borrow().is_empty());
    assert!(report.to_json().to_string().len() < 64 << 10);
    assert_eq!(
        ok(&format!(
            "try {{ callTool('x'.repeat({})); }} catch (e) {{ return e.message; }}",
            MAX_TOOL_NAME_BYTES + 1
        )),
        json!(format!(
            "a tool name is at most {MAX_TOOL_NAME_BYTES} bytes; nothing was called"
        ))
    );
}

/// Major 3: error text is cut to about 2 KiB.
#[test]
fn huge_error_messages_are_cut() {
    for script in [
        "throw new Error('x'.repeat(3e7));",
        "throw new Error('x'.repeat(3e5));",
        "throw { big: 'y'.repeat(1e6) };",
    ] {
        match run(script, &Fake::default(), &quick()).outcome {
            Err(RunError::Memory { .. }) => {}
            Err(RunError::Thrown { message, .. }) => {
                assert!(
                    message.len() <= MAX_MESSAGE_BYTES + 64,
                    "{script}: {}",
                    message.len()
                );
                assert!(message.contains("(cut,"), "{script}");
            }
            other => panic!("{script}: {other:?}"),
        }
    }
    let shown = run(
        "throw new Error('z'.repeat(3e5));",
        &Fake::default(),
        &quick(),
    )
    .to_json();
    assert!(
        shown.to_string().len() < 4096,
        "{}",
        shown.to_string().len()
    );
}

/// Minor 13: catastrophic regex backtracking is stopped by the deadline;
/// shared memory and Atomics are gone.
#[test]
fn regex_backtracking_stops_and_atomics_are_absent() {
    let limits = Limits {
        cpu: Duration::from_millis(300),
        ..Limits::default()
    };
    let report = run_bounded("return /(a+)+$/.test('a'.repeat(40) + 'b');", limits);
    assert!(
        matches!(report.outcome, Err(RunError::CpuDeadline { .. })),
        "{:?}",
        report.outcome
    );
    assert_eq!(
        ok("return [typeof SharedArrayBuffer, typeof Atomics];"),
        json!(["undefined", "undefined"])
    );
}

/// Minor 10: the line that crosses the console cap is cut, not dropped.
#[test]
fn the_console_line_over_the_cap_is_cut() {
    let limits = Limits {
        max_console_bytes: 20,
        ..quick()
    };
    let report = run(
        "console.log('abcdef'); console.log('0123456789012345'); console.log('never'); return 1;",
        &Fake::default(),
        &limits,
    );
    assert!(report.console_truncated);
    assert_eq!(
        report.console,
        vec!["abcdef".to_string(), "012345678901".to_string()]
    );
}

/// Major 4: after a declined card the run stops, so a retry loop around an
/// Always-ask call shows exactly one card.
#[test]
fn a_declined_card_ends_the_run_and_a_retry_loop_asks_once() {
    let mut bed = Bed::new();
    crate::workspaces::main_agent_workspace(&bed.v).unwrap();
    let path = bed.v.join("workspaces/main/STANDING_ORDERS.md");
    let seed = std::fs::read_to_string(&path).unwrap();
    let deny = Recording(Verdict::Denied, RefCell::new(Vec::new()));
    let caller = bed.caller(AppAccess::Write, "bypassPermissions");
    let report = bed.run(
        r#"for (;;) {
             try { tools["workspaces.edit_standing_orders"]({ op: "add", line: "- Again." }); }
             catch (e) {}
           }"#,
        caller.clone(),
        &deny,
    );
    assert!(
        matches!(&report.outcome, Err(RunError::CardRefused { message }) if message.contains("declined")),
        "{:?}",
        report.outcome
    );
    assert_eq!(deny.1.borrow().len(), 1, "exactly one card");
    assert_eq!(report.calls.len(), 1);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), seed);
    assert_eq!(report.to_json()["error"]["code"], json!("card_refused"));
    // The same host refuses any later call unsent, even one with no card.
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let broker = BrokerToolHost::new(&host, &bed.v, caller, &deny, &app_broker::APP_POLICY, true);
    let id = "workspaces.edit_standing_orders";
    assert!(broker
        .call(id, json!({ "op": "add", "line": "- x" }))
        .is_err());
    assert!(broker.stop_run().is_some());
    let later = broker.call("vault.vault_tree", json!({})).unwrap_err();
    assert!(later.contains("declined"), "{later}");
    assert_eq!(
        deny.1.borrow().len(),
        2,
        "the second host asked once, then never"
    );
    bed.finish();
}

/// CM1c: `codemode.run` through `app_broker::dispatch` (what the shared HTTP
/// endpoint, the app socket and `hollow serve` all call): the script runs
/// as the caller, the outer call and each inner call are audited, a card is
/// still shown for an Always-ask inner action in Bypass, a declined card
/// stops the script, and the answer is the report.
#[test]
fn codemode_run_is_served_through_dispatch_and_stays_gated() {
    let mut bed = Bed::new();
    std::fs::create_dir_all(bed.v.join("notes")).unwrap();
    std::fs::write(bed.v.join("notes/plan.md"), "# Plan\n\nShip CM1c.\n").unwrap();
    let caller = bed.caller(AppAccess::Write, "bypassPermissions");
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let ask = Recording(Verdict::Denied, RefCell::new(Vec::new()));
    let maker = BrokerToolHost::new(
        &host,
        &bed.v,
        caller.clone(),
        &ask,
        &app_broker::APP_POLICY,
        true,
    );
    let send = |script: &str| {
        let req = maker.request("codemode.run", json!({ "script": script }));
        app_broker::dispatch(&host, &bed.v, &req, true, &ask, &app_broker::APP_POLICY)
    };
    // A read script: the report carries the value and the calls it made.
    let report = send(r#"return tools["vault.vault_note"]({ path: "notes/plan.md" });"#).unwrap();
    assert_eq!(report["ok"], json!(true), "{report}");
    assert!(report.to_string().contains("Ship CM1c."));
    assert_eq!(report["calls"][0]["tool"], json!("vault.vault_note"));
    assert!(ask.1.borrow().is_empty());
    // An Always-ask inner action still asks (denied here), and the run stops.
    crate::workspaces::main_agent_workspace(&bed.v).unwrap();
    let denied = send(
        r#"tools["workspaces.edit_standing_orders"]({ op: "add", line: "- Never email anyone." }); return "after";"#,
    )
    .unwrap();
    assert_eq!(denied["ok"], json!(false), "{denied}");
    assert_eq!(denied["error"]["code"], json!("card_refused"), "{denied}");
    // The script argument is required.
    let missing = app_broker::dispatch(
        &host,
        &bed.v,
        &maker.request("codemode.run", json!({})),
        true,
        &ask,
        &app_broker::APP_POLICY,
    );
    assert!(missing.is_err());
    // The outer call is audited as itself (script digested by the audit),
    // beside the inner calls' codemode lines.
    let lines: Vec<Value> = std::fs::read_to_string(bed.v.join(app_broker::AUDIT_FILE))
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert!(
        lines
            .iter()
            .filter(|l| l["action"] == json!("codemode.run"))
            .count()
            >= 2,
        "{lines:?}"
    );
    assert!(lines.iter().any(|l| l["via"] == json!("codemode")));
    bed.finish();
}

/// CM1c: the tool is first-class, listed, and not marked read-only; a nested
/// call is not listed to the script.
#[test]
fn run_script_is_a_first_class_tool_and_not_read_only() {
    assert_eq!(crate::app_mcp::tool_for("codemode.run"), Some("run_script"));
    let (level, why) = app_actions::stability_of("codemode.run");
    assert_eq!(level.as_str(), "experimental");
    assert!(why.is_some());
    let action = app_actions::find("codemode.run").unwrap();
    assert_eq!(action.tier, app_actions::Tier::Read);
    assert!(is_nested_tool("codemode.run") && is_nested_tool("run_script"));
}

/// CM1c: an asker that goes invalid (a revoked attempt) or whose client hung
/// up ends a running script at its next call, and a call after the overall
/// limit never starts; `sessions.wait` and sign-ins are refused in scripts.
struct Flips {
    calls: std::cell::Cell<usize>,
    hung_up_after: usize,
}

impl Asker for Flips {
    fn ask(&self, _card: &Card, _caller: &Caller) -> Verdict {
        Verdict::Denied
    }
    fn hung_up(&self) -> bool {
        self.calls.set(self.calls.get() + 1);
        self.calls.get() > self.hung_up_after
    }
}

#[test]
fn a_hung_up_client_ends_the_script_and_long_waits_are_refused() {
    let mut bed = Bed::new();
    std::fs::create_dir_all(bed.v.join("notes")).unwrap();
    std::fs::write(bed.v.join("notes/plan.md"), "# Plan\n").unwrap();
    let caller = bed.caller(AppAccess::Write, "default");
    let flips = Flips {
        calls: std::cell::Cell::new(0),
        hung_up_after: 1,
    };
    let report = bed.run(
        r#"const a = tools["vault.vault_note"]({ path: "notes/plan.md" });
           const b = tools["vault.vault_note"]({ path: "notes/plan.md" });
           const c = tools["vault.vault_note"]({ path: "notes/plan.md" });
           return "finished";"#,
        caller.clone(),
        &flips,
    );
    assert!(
        matches!(report.outcome, Err(RunError::CardRefused { .. })),
        "{:?}",
        report.outcome
    );
    // Checked before each call and after it: the first runs (not yet hung
    // up before it), the hang-up is seen after it, so the second never starts.
    assert_eq!(report.calls.len(), 1, "{:?}", report.calls);
    // The long-waiting actions are refused, and not listed.
    let ask = Recording(Verdict::Denied, RefCell::new(Vec::new()));
    let host = || -> Box<dyn app_actions::Host> { Box::new(bed.app.handle().clone()) };
    let broker = BrokerToolHost::new(&host, &bed.v, caller, &ask, &app_broker::APP_POLICY, true);
    let refused = broker
        .call("sessions.wait", json!({ "key": "s-1" }))
        .unwrap_err();
    assert!(refused.contains("call it directly"), "{refused}");
    assert!(broker
        .tools()
        .iter()
        .all(|t| t.name != "sessions.wait" && !t.name.contains("sign_in")));
    // No call is made once the overall limit has passed (the interrupt
    // handler, the pre-call check and the post-call check each stop it).
    let limits = Limits {
        overall: Duration::ZERO,
        ..quick()
    };
    std::thread::sleep(Duration::from_millis(5)); // sleep-ok: the zero overall limit must be past before the call
    let late = run(
        r#"return tools["vault.vault_note"]({ path: "notes/plan.md" });"#,
        &broker,
        &limits,
    );
    assert!(late.calls.is_empty(), "{:?}", late.calls);
    assert!(late.outcome.is_err());
    bed.finish();
}

#[cfg(unix)]
#[test]
fn audit_inner_refuses_symlinks_and_non_regular_files() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("outside.txt");
    std::fs::write(&target, "untouched").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(app_broker::AUDIT_FILE)).unwrap();

    let req = AppRequest {
        v: 1,
        action: "vault.vault_note".into(),
        run_id: "run-cm".into(),
        surface: "chat".into(),
        scope: "s-1".into(),
        access: AppAccess::Read.as_str().into(),
        depth: 0,
        perm: "acceptEdits".into(),
        proof: String::new(),
        tool_args: json!({}),
        args: json!({}),
        tool: "run_action".into(),
    };
    audit_inner(dir.path(), &req, Some("read"), None, Ok(()));
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "untouched",
        "symlink target must never be written to"
    );

    std::fs::remove_file(dir.path().join(app_broker::AUDIT_FILE)).unwrap();
    std::fs::create_dir(dir.path().join(app_broker::AUDIT_FILE)).unwrap();
    audit_inner(dir.path(), &req, Some("read"), None, Ok(()));
    assert!(dir.path().join(app_broker::AUDIT_FILE).is_dir());
}
