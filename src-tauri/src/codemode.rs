//! Code mode (CM1a, overnight 2026-11-17): an agent writes one short
//! JavaScript script instead of ten tool calls. The script runs in a QuickJS
//! sandbox (`rquickjs`, MIT) whose only power is the tool calls the host
//! injects; only its final value, as JSON and size-capped, comes back, with a
//! bounded console log beside it. Not Claude Code's experimental Code Mode.
//!
//! Decisions (record here when they change):
//!
//! - **Engine:** QuickJS-ng through `rquickjs` 0.14 with default features
//!   only. QuickJS itself has no `os`/`std` module, no network, no timers
//!   and no module loader unless a host adds them; quickjs-libc is not
//!   compiled in and no loader is set, so `import()` rejects. The context is
//!   built with an explicit intrinsic list (no `Performance`) and
//!   [`REMOVED_GLOBALS`] are deleted before the script runs.
//!   `eval` and `Function` stay: they compile more code inside the same
//!   sandbox and give no power the script doesn't already have.
//! - **Convention:** the script is the body of an `async` function, so
//!   `return x` gives the result and `await` is allowed (agents write it out
//!   of habit; tool calls are synchronous, so awaiting one is a no-op). The
//!   body starts on line 1 of the evaluated source, so error lines are the
//!   script's own. A missing `return` gives `null`. Promise jobs are drained
//!   after the call, under the same deadline.
//! - **Tools:** one frozen global `tools` object with a method per tool the
//!   host lists (`tools["tasks.dashboard"]({})`), plus `callTool(name, args)`
//!   for any name. Each returns the parsed JSON result synchronously or
//!   throws an `Error` whose message is the host's refusal. An inner result
//!   over [`Limits::max_call_result_bytes`] reaches the script as
//!   `{ truncated: true, bytes, preview }` (the head of its JSON text) and the
//!   call is flagged in the report. Through [`BrokerToolHost`] the broker's
//!   `fit_result` has already cut a result to 80,000 characters, so the two
//!   caps overlap there. A tool name over [`MAX_TOOL_NAME_BYTES`] is refused
//!   before it is recorded; error and refusal text is cut to
//!   [`MAX_MESSAGE_BYTES`].
//! - **Two clocks:** [`Limits::cpu`] (default 10 s) is script time, wall time
//!   minus time spent inside host calls: the host time is subtracted, because an
//!   inner call may wait on the person's Allow card (up to two minutes) and
//!   must not be killed mid-card. [`Limits::overall`] (default 10 min) caps
//!   the whole run, host calls included; it is checked by the interrupt
//!   handler and after every host call. A breach is sticky: once recorded,
//!   the interrupt handler answers "stop" at every poll (QuickJS polls about
//!   every 10,000 operations, and inside regex matching), the interrupt's
//!   error can't be caught by the script, and the natives refuse to run.
//!   No native here ever catches and drops an exception while the script
//!   runs (that would clear the interrupt): `console.log` passes a throwing
//!   `toJSON` back to the script.
//! - **Memory:** a counting allocator (`Budget`, default 32 MiB) refuses
//!   past the limit and records that it did, because QuickJS reports an
//!   allocation it couldn't make as a thrown `null` once even the error
//!   object can't be built; QuickJS's own limit sits at twice that as a
//!   backstop. Max stack 512 KiB. A run that hit the limit and then failed
//!   is reported as [`RunError::Memory`] (even if the script caught the first
//!   refusal and threw something else); deep recursion as
//!   [`RunError::StackOverflow`]. Doubling a string with `+` builds a rope
//!   and stops at QuickJS's string length cap ("string too long") first.
//! - **Calls:** at most [`Limits::max_calls`] (default 50) per run; the next
//!   one records a sticky breach, so the script stops within the next
//!   interrupt poll and the run reports [`RunError::TooManyCalls`].
//! - **Declined cards:** once an inner call's card is declined, times out or
//!   is cancelled, [`ToolHost::stop_run`] stops the run the same sticky way
//!   ([`RunError::CardRefused`]), and [`BrokerToolHost`] refuses any later
//!   call unsent, so a script can't catch a Deny and raise card after card.
//! - **Shared memory:** `SharedArrayBuffer` and `Atomics` are deleted
//!   (QuickJS also refuses `Atomics.wait` on a thread that can't block, its
//!   default), so nothing can block the thread outside the interrupt.
//! - **No nesting:** a script cannot start another code-mode run, by tool
//!   name ([`is_nested_tool`]) or by re-entering [`run`] on the same thread.
//! - **Fresh state:** every run builds its own runtime and context and drops
//!   them before returning, so two runs share nothing.
//! - **Broker route:** [`BrokerToolHost`] sends each inner call through the
//!   same steps as a direct `hollow_app` call (`app_broker::authorize`, the
//!   policy's screen, the retry guard, `app_broker::execute` with the
//!   caller's `Caller` and the real `Asker`, the policy's reword), so tiers,
//!   cards, Always-ask and run liveness all apply unchanged. Each inner call
//!   appends one audit line to `.app-actions.jsonl` with the broker's fields
//!   plus `"via": "codemode"` and `"tool": "codemode"`. `app_broker::audit`
//!   takes no such field and this lane doesn't own `app_broker.rs`, so the
//!   line is written here, without the broker's rotation lock.
//!
//! Seam notes for CM1c (serving this through the owned MCP):
//!
//! - (a) `run_effects.rs` (about lines 388 to 414) classifies an unknown
//!   `hollow_app` tool such as `codemode` as `Class::Ignore`, so changes made
//!   in a script miss the run receipt and the retry guard. Record each inner
//!   change call as a keyed effect, or classify code mode as Unknown.
//! - (b) `approvals.rs` `take_binding_for` binds only the first card to the
//!   tool row; a second card-needing inner call waits `BIND_WAIT` (750 ms)
//!   and then shows an unbound card. Bind per inner card, for example by the
//!   outer `tool_use_id`.
//! - (c) The client's read timeout (`app_mcp.rs` `broker_wait_secs`) must
//!   cover [`Limits::overall`] plus one card, and the interrupt handler
//!   should also check `asker.valid()` so a client that hung up ends the
//!   script.
//! - (d) [`BrokerToolHost::new`]'s `caller` must come from
//!   `app_broker::authorize` of the outer request: `request` mints a valid
//!   proof for any caller it is given.
//! - (e) Call [`run`] on a plain thread with at least 2 MiB of stack, never a
//!   tokio worker (`execute` uses `block_on`).
//! - (f) The outer call needs its own audit line (for example with a hash
//!   of the script), and [`NESTED_TOOLS`] should list the real tool name
//!   CM1c picks.
//! - (g) Give `app_broker::audit` a `via` field, so `audit_inner` (which
//!   skips `AUDIT_LOCK` and rotation) can go.
//! - (h) Build one `BrokerToolHost` per run: a refused card is remembered on
//!   the host, so a reused host would refuse every later run's card calls.

use crate::app_actions::{self, Caller};
use crate::app_broker::{self, AppRequest, Asker, Policy};
use crate::app_mcp::AppGrant;
use crate::approvals::Verdict;
use rquickjs::allocator::{Allocator, RustAllocator};
use rquickjs::context::intrinsic;
use rquickjs::function::{Opt, Rest};
use rquickjs::promise::PromiseState;
use rquickjs::{Context, Ctx, Exception, Function, Runtime};
use serde_json::{json, Value};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Tool names that would start another code-mode run: never callable from a
/// script, never listed to one.
pub(crate) const NESTED_TOOLS: &[&str] = &[
    "codemode",
    "codemode.run",
    "codemode.run_script",
    "run_code",
    "run_script",
];

/// Globals deleted before the script runs. Most never exist in this build
/// (QuickJS has no fetch, timers, process or loader); deleting them anyway
/// keeps the list the contract the tests check.
pub(crate) const REMOVED_GLOBALS: &[&str] = &[
    "fetch",
    "XMLHttpRequest",
    "WebSocket",
    "setTimeout",
    "setInterval",
    "setImmediate",
    "clearTimeout",
    "clearInterval",
    "clearImmediate",
    "queueMicrotask",
    "process",
    "require",
    "module",
    "exports",
    "std",
    "os",
    "WebAssembly",
    "Deno",
    "Bun",
    "navigator",
    "performance",
    "print",
    "scriptArgs",
    "SharedArrayBuffer",
    "Atomics",
];

/// A tool name longer than this is refused before it is recorded or sent.
pub(crate) const MAX_TOOL_NAME_BYTES: usize = 200;
/// Error and refusal text in a report is cut to this many bytes.
pub(crate) const MAX_MESSAGE_BYTES: usize = 2048;

/// `text` cut to `max` bytes on a char boundary, marked when cut.
fn cut_message(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let at = floor_char_boundary(&text, max);
    format!("{}... (cut, {} bytes in all)", &text[..at], text.len())
}

/// The filename QuickJS gives the script, for line numbers.
const FILENAME: &str = "agent-script.js";
/// The prelude's own filename, so its frames never read as script lines.
const PRELUDE_FILENAME: &str = "hollow-prelude.js";

/// Whether `name` (an action id or tool name) would start a code-mode run.
pub(crate) fn is_nested_tool(name: &str) -> bool {
    let name = name.trim();
    let bare = name.rsplit("__").next().unwrap_or(name);
    NESTED_TOOLS.contains(&name) || NESTED_TOOLS.contains(&bare)
}

// ── Host seam ────────────────────────────────────────────────────────────

/// One tool a script may call, as the host lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolInfo {
    /// The name the script calls (`tools[name]`): an action id for
    /// [`BrokerToolHost`].
    pub name: String,
    /// `read`, `write`, `confirm` or `external` (empty when not tiered).
    pub tier: String,
    /// One line on what it does.
    pub summary: String,
    /// Its parameters, compactly (`app_actions::signature`).
    pub signature: String,
}

/// What a script's tool calls reach. Implemented by the host that serves
/// code mode (CM1c: the owned MCP) and by tests.
pub(crate) trait ToolHost {
    /// Run one tool call. `Err` is the refusal or failure text the script
    /// sees as a thrown `Error`.
    fn call(&self, tool: &str, args: Value) -> Result<Value, String>;
    /// The tools the script gets methods for.
    fn tools(&self) -> Vec<ToolInfo>;
    /// Asked after every call: `Some(reason)` ends the run at the next interrupt
    /// poll ([`RunError::CardRefused`]; the host's natives refuse until then). [`BrokerToolHost`] says so once the
    /// person declined a card (or it timed out or was cancelled), so a
    /// script can't catch the refusal and raise card after card.
    fn stop_run(&self) -> Option<String> {
        None
    }
}

// ── Limits and results ───────────────────────────────────────────────────

/// Every bound on one run. [`Default`] is the conservative production set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Limits {
    /// Script time: wall time minus time spent inside host calls.
    pub cpu: Duration,
    /// The whole run, host calls (and their cards) included.
    pub overall: Duration,
    /// The QuickJS allocator limit.
    pub memory_bytes: usize,
    /// QuickJS's max stack size.
    pub stack_bytes: usize,
    /// Inner tool calls per run.
    pub max_calls: usize,
    /// One inner result, as JSON text, before it is cut. The broker's own
    /// `fit_result` already cuts an action's result to 80,000 characters, so
    /// through [`BrokerToolHost`] this mostly binds other hosts.
    pub max_call_result_bytes: usize,
    /// The script's final value, as JSON text.
    pub max_output_bytes: usize,
    /// The console log, all lines together.
    pub max_console_bytes: usize,
    /// The script's source.
    pub max_script_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            cpu: Duration::from_secs(10),
            overall: Duration::from_secs(10 * 60),
            memory_bytes: 32 << 20,
            stack_bytes: 512 << 10,
            max_calls: 50,
            max_call_result_bytes: 256 << 10,
            max_output_bytes: 64 << 10,
            max_console_bytes: 16 << 10,
            max_script_bytes: 64 << 10,
        }
    }
}

/// Why a run produced no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RunError {
    ScriptTooLarge {
        bytes: usize,
        max: usize,
    },
    /// The script doesn't parse.
    Syntax {
        message: String,
        line: Option<u32>,
    },
    /// The script threw (or its promise rejected) and didn't catch it.
    Thrown {
        message: String,
        line: Option<u32>,
    },
    CpuDeadline {
        limit: Duration,
    },
    OverallDeadline {
        limit: Duration,
    },
    Memory {
        limit: usize,
    },
    StackOverflow,
    TooManyCalls {
        max: usize,
    },
    OutputTooLarge {
        bytes: usize,
        max: usize,
    },
    /// The final value can't be JSON (a cycle, a BigInt).
    NotJson(String),
    /// A code-mode run inside a code-mode run.
    Nested,
    /// The person declined (or didn't answer) a card an inner call showed.
    CardRefused {
        message: String,
    },
    /// The engine itself failed (should not happen).
    Engine(String),
}

impl RunError {
    /// A stable code for the result JSON.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            RunError::ScriptTooLarge { .. } => "script_too_large",
            RunError::Syntax { .. } => "syntax",
            RunError::Thrown { .. } => "thrown",
            RunError::CpuDeadline { .. } => "cpu_deadline",
            RunError::OverallDeadline { .. } => "overall_deadline",
            RunError::Memory { .. } => "memory",
            RunError::StackOverflow => "stack_overflow",
            RunError::TooManyCalls { .. } => "too_many_calls",
            RunError::OutputTooLarge { .. } => "output_too_large",
            RunError::NotJson(_) => "not_json",
            RunError::Nested => "nested",
            RunError::CardRefused { .. } => "card_refused",
            RunError::Engine(_) => "engine",
        }
    }

    fn line(&self) -> Option<u32> {
        match self {
            RunError::Syntax { line, .. } | RunError::Thrown { line, .. } => *line,
            _ => None,
        }
    }
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let at = |line: &Option<u32>| line.map(|l| format!(" (line {l})")).unwrap_or_default();
        match self {
            RunError::ScriptTooLarge { bytes, max } => write!(f, "The script is {bytes} bytes, over the {max}-byte limit; nothing ran."),
            RunError::Syntax { message, line } => write!(f, "The script doesn't parse{}: {message}. Nothing ran.", at(line)),
            RunError::Thrown { message, line } => write!(f, "The script threw{}: {message}", at(line)),
            RunError::CpuDeadline { limit } => write!(f, "The script ran past its {} s time limit and was stopped. Tool calls it made before that did run.", limit.as_secs_f64()),
            RunError::OverallDeadline { limit } => write!(f, "The run passed its overall limit of {} s (tool calls and cards included) and was stopped. Tool calls it made before that did run.", limit.as_secs()),
            RunError::Memory { limit } => write!(f, "The script ran out of memory (limit {} MiB) and was stopped.", limit >> 20),
            RunError::StackOverflow => write!(f, "The script overflowed its stack (too deep a recursion) and was stopped."),
            RunError::TooManyCalls { max } => write!(f, "The script made more than {max} tool calls and was stopped. The first {max} did run."),
            RunError::OutputTooLarge { bytes, max } => write!(f, "The script's result is {bytes} bytes of JSON, over the {max}-byte limit. Return less (pick fields, count, or summarize)."),
            RunError::NotJson(why) => write!(f, "The script's result can't be turned into JSON: {why}"),
            RunError::Nested => write!(f, "A code-mode script can't start another code-mode run; nothing ran."),
            RunError::CardRefused { message } => write!(f, "{message} The script was stopped there: after a card is declined, a script asks nothing more. Tool calls it made before that did run."),
            RunError::Engine(why) => write!(f, "The script sandbox failed: {why}"),
        }
    }
}

/// One inner tool call, as the report lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CallRecord {
    pub tool: String,
    pub ok: bool,
    /// The result was over [`Limits::max_call_result_bytes`] and cut.
    pub truncated: bool,
    pub millis: u64,
}

/// What one run produced.
#[derive(Debug, Clone)]
pub(crate) struct RunReport {
    pub outcome: Result<Value, RunError>,
    pub console: Vec<String>,
    pub console_truncated: bool,
    pub calls: Vec<CallRecord>,
    /// Time running script code.
    pub script_ms: u64,
    /// The whole run.
    pub wall_ms: u64,
}

impl RunReport {
    fn failed(error: RunError, started: Instant) -> RunReport {
        RunReport {
            outcome: Err(error),
            console: Vec::new(),
            console_truncated: false,
            calls: Vec::new(),
            script_ms: 0,
            wall_ms: millis(started.elapsed()),
        }
    }

    /// The report as the tool result an agent reads.
    pub(crate) fn to_json(&self) -> Value {
        let calls: Vec<Value> = self
            .calls
            .iter()
            .map(
                |c| json!({ "tool": c.tool, "ok": c.ok, "truncated": c.truncated, "ms": c.millis }),
            )
            .collect();
        let mut out = match &self.outcome {
            Ok(value) => json!({ "ok": true, "result": value }),
            Err(error) => json!({
                "ok": false,
                "error": { "code": error.code(), "message": error.to_string(), "line": error.line() },
            }),
        };
        out["console"] = json!(self.console);
        if self.console_truncated {
            out["consoleTruncated"] = json!(true);
        }
        out["calls"] = json!(calls);
        out["scriptMs"] = json!(self.script_ms);
        out["wallMs"] = json!(self.wall_ms);
        out
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

// ── The run ──────────────────────────────────────────────────────────────

thread_local! {
    /// Set while [`run`] runs on this thread (the nested-run guard).
    static RUNNING: Cell<bool> = const { Cell::new(false) };
}

struct NestGuard;

impl NestGuard {
    fn enter() -> Option<NestGuard> {
        RUNNING.with(|r| (!r.replace(true)).then_some(NestGuard))
    }
}

impl Drop for NestGuard {
    fn drop(&mut self) {
        RUNNING.with(|r| r.set(false));
    }
}

/// A limit that stops the script (uncatchably, through the interrupt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Breach {
    Cpu,
    Overall,
    TooManyCalls,
    CardRefused,
}

/// The run's shared state: what the native functions and the interrupt
/// handler see.
struct State {
    /// The host for this run only: cleared before [`run`] returns (see the
    /// SAFETY note there).
    host: Cell<Option<&'static dyn ToolHost>>,
    limits: Limits,
    started: Instant,
    in_host: Cell<Duration>,
    breach: Cell<Option<Breach>>,
    /// Set by [`Budget`] when it refused an allocation over the limit.
    out_of_memory: Rc<Cell<bool>>,
    /// Why the host stopped the run ([`ToolHost::stop_run`]).
    stopped: RefCell<Option<String>>,
    calls: RefCell<Vec<CallRecord>>,
    console: RefCell<(Vec<String>, usize, bool)>,
}

impl State {
    fn script_time(&self) -> Duration {
        self.started.elapsed().saturating_sub(self.in_host.get())
    }

    /// Whether to stop the script now; records why.
    fn should_interrupt(&self) -> bool {
        if self.breach.get().is_some() {
            return true;
        }
        if self.started.elapsed() > self.limits.overall {
            self.breach.set(Some(Breach::Overall));
            return true;
        }
        if self.script_time() > self.limits.cpu {
            self.breach.set(Some(Breach::Cpu));
            return true;
        }
        false
    }

    fn breach_error(&self) -> Option<RunError> {
        self.breach.get().map(|b| match b {
            Breach::Cpu => RunError::CpuDeadline {
                limit: self.limits.cpu,
            },
            Breach::Overall => RunError::OverallDeadline {
                limit: self.limits.overall,
            },
            Breach::TooManyCalls => RunError::TooManyCalls {
                max: self.limits.max_calls,
            },
            Breach::CardRefused => RunError::CardRefused {
                message: self.stopped.borrow().clone().unwrap_or_default(),
            },
        })
    }

    fn log(&self, line: String) {
        let mut console = self.console.borrow_mut();
        let (lines, used, cut) = &mut *console;
        if *cut {
            return;
        }
        let room = self.limits.max_console_bytes.saturating_sub(*used + 1);
        if line.len() > room {
            *cut = true;
            let at = floor_char_boundary(&line, room);
            if at > 0 {
                *used += at + 1;
                lines.push(line[..at].to_string());
            }
            return;
        }
        *used += line.len() + 1;
        lines.push(line);
    }
}

/// Run `script` against `host` within `limits`. Never panics on script
/// input and never hangs past `limits.overall` (plus one host call).
pub(crate) fn run(script: &str, host: &dyn ToolHost, limits: &Limits) -> RunReport {
    let started = Instant::now();
    if script.len() > limits.max_script_bytes {
        return RunReport::failed(
            RunError::ScriptTooLarge {
                bytes: script.len(),
                max: limits.max_script_bytes,
            },
            started,
        );
    }
    let Some(_guard) = NestGuard::enter() else {
        return RunReport::failed(RunError::Nested, started);
    };
    let tools: Vec<String> = host
        .tools()
        .into_iter()
        .map(|t| t.name)
        .filter(|name| !is_nested_tool(name))
        .collect();
    // SAFETY: the 'static lifetime is a lie the closures need (rquickjs
    // functions must be 'static). The reference lives in `state.host`, which
    // only the runtime built in `sandbox` (its functions and interrupt
    // handler) reads; `sandbox` drops that runtime before it returns, and the
    // slot is cleared right after, while `host` is still borrowed by this
    // call. Nothing reads it after `run` returns.
    let erased: &'static dyn ToolHost =
        unsafe { std::mem::transmute::<&dyn ToolHost, &'static dyn ToolHost>(host) };
    let state = Rc::new(State {
        host: Cell::new(Some(erased)),
        limits: limits.clone(),
        started,
        in_host: Cell::new(Duration::ZERO),
        breach: Cell::new(None),
        out_of_memory: Rc::new(Cell::new(false)),
        stopped: RefCell::new(None),
        calls: RefCell::new(Vec::new()),
        console: RefCell::new((Vec::new(), 0, false)),
    });
    let outcome = sandbox(script, &tools, &state);
    state.host.set(None);
    let outcome = match state.breach_error() {
        Some(breach) => Err(breach),
        None => outcome,
    };
    let (console, _, console_truncated) = state.console.take();
    RunReport {
        outcome,
        console,
        console_truncated,
        calls: state.calls.take(),
        script_ms: millis(state.script_time()),
        wall_ms: millis(started.elapsed()),
    }
}

/// Wires `tools`, `callTool` and `console` from three native values; the
/// natives themselves are reachable only through these wrappers.
const PRELUDE: &str = r#"(function (call, log, names) {
  "use strict";
  const define = (key, value) => Object.defineProperty(globalThis, key, { value, writable: false, configurable: false, enumerable: false });
  const tools = Object.create(null);
  for (const name of names) {
    Object.defineProperty(tools, name, { value: function (args) { return call(name, args); }, enumerable: true });
  }
  define("tools", Object.freeze(tools));
  define("callTool", function callTool(name, args) { return call(String(name), args); });
  const write = (...parts) => { log(...parts); };
  define("console", Object.freeze({ log: write, info: write, warn: write, error: write, debug: write }));
})"#;

fn engine(error: rquickjs::Error) -> RunError {
    RunError::Engine(cut_message(error.to_string(), MAX_MESSAGE_BYTES))
}

fn sandbox(script: &str, tools: &[String], state: &Rc<State>) -> Result<Value, RunError> {
    let limits = &state.limits;
    let rt = Runtime::new_with_alloc(Budget {
        limit: limits.memory_bytes,
        used: 0,
        hit: Rc::clone(&state.out_of_memory),
    })
    .map_err(engine)?;
    // A backstop only: `Budget` refuses first and records it.
    rt.set_memory_limit(limits.memory_bytes.saturating_mul(2));
    rt.set_max_stack_size(limits.stack_bytes);
    let watch = Rc::clone(state);
    rt.set_interrupt_handler(Some(Box::new(move || watch.should_interrupt())));
    let ctx = Context::custom::<(
        intrinsic::Date,
        intrinsic::Eval,
        intrinsic::RegExpCompiler,
        intrinsic::RegExp,
        intrinsic::Json,
        intrinsic::Proxy,
        intrinsic::MapSet,
        intrinsic::TypedArrays,
        intrinsic::Promise,
        intrinsic::WeakRef,
    )>(&rt)
    .map_err(engine)?;
    let outcome = ctx.with(|ctx| -> Result<Value, RunError> {
        install(&ctx, tools, state).map_err(|e| failure(&ctx, e, state, false))?;
        let source = format!("(async function () {{ {script}\n}})");
        let mut options = rquickjs::context::EvalOptions::default();
        options.global = true;
        options.strict = true;
        options.filename = Some(FILENAME.into());
        let func: Function = ctx
            .eval_with_options(source, options)
            .map_err(|e| failure(&ctx, e, state, true))?;
        let value: rquickjs::Value = func.call(()).map_err(|e| failure(&ctx, e, state, false))?;
        let Some(promise) = value.as_promise().cloned() else {
            return Err(RunError::Engine("the script wrapper returned no promise".into()));
        };
        while promise.state() == PromiseState::Pending
            && !state.should_interrupt()
            && ctx.execute_pending_job()
        {}
        if let Some(breach) = state.breach_error() {
            return Err(breach);
        }
        let value: rquickjs::Value = match promise.state() {
            PromiseState::Resolved => promise
                .result::<rquickjs::Value>()
                .unwrap_or(Ok(rquickjs::Value::new_undefined(ctx.clone())))
                .map_err(|e| failure(&ctx, e, state, false))?,
            PromiseState::Rejected => {
                let reason = match promise.result::<rquickjs::Value>() {
                    Some(Err(rquickjs::Error::Exception)) => ctx.catch(),
                    Some(Ok(v)) => v,
                    Some(Err(other)) => return Err(engine(other)),
                    None => rquickjs::Value::new_undefined(ctx.clone()),
                };
                return Err(classify(state, describe(&ctx, &reason), false));
            }
            PromiseState::Pending => {
                return Err(RunError::Thrown {
                    message: "the script awaited something that never settled (there are no timers or I/O here)".into(),
                    line: None,
                });
            }
        };
        output(&ctx, value, state)
    });
    // The context and runtime drop here, before `run` clears the host.
    drop(ctx);
    drop(rt);
    outcome
}

/// QuickJS's allocations, counted against the run's memory limit. Out of
/// memory, QuickJS reports a thrown `null` when it can't even allocate the
/// error object, so the refusal is recorded here, where it is certain.
/// Allocates through `rquickjs`'s Rust allocator.
struct Budget {
    limit: usize,
    used: usize,
    hit: Rc<Cell<bool>>,
}

/// Counted per allocation on top of its usable size (the Rust allocator's
/// header, rounded up).
const ALLOC_OVERHEAD: usize = 16;

impl Budget {
    /// Whether `more` bytes fit; records the refusal when they don't.
    fn fits(&mut self, more: usize) -> bool {
        let fits = self
            .used
            .checked_add(more)
            .and_then(|n| n.checked_add(ALLOC_OVERHEAD))
            .is_some_and(|n| n <= self.limit);
        if !fits {
            self.hit.set(true);
        }
        fits
    }
}

// SAFETY: every pointer comes from and goes back to `RustAllocator`, which
// upholds the trait's contract; this wrapper only counts and refuses.
unsafe impl Allocator for Budget {
    fn alloc(&mut self, size: usize) -> *mut u8 {
        if !self.fits(size) {
            return std::ptr::null_mut();
        }
        let ptr = RustAllocator.alloc(size);
        if !ptr.is_null() {
            // SAFETY: `ptr` was just returned by `RustAllocator`.
            self.used += unsafe { RustAllocator::usable_size(ptr) } + ALLOC_OVERHEAD;
        }
        ptr
    }

    fn calloc(&mut self, count: usize, size: usize) -> *mut u8 {
        let Some(total) = count.checked_mul(size) else {
            return std::ptr::null_mut();
        };
        if total == 0 || !self.fits(total) {
            return std::ptr::null_mut();
        }
        let ptr = RustAllocator.calloc(count, size);
        if !ptr.is_null() {
            // SAFETY: `ptr` was just returned by `RustAllocator`.
            self.used += unsafe { RustAllocator::usable_size(ptr) } + ALLOC_OVERHEAD;
        }
        ptr
    }

    unsafe fn dealloc(&mut self, ptr: *mut u8) {
        self.used = self
            .used
            .saturating_sub(RustAllocator::usable_size(ptr) + ALLOC_OVERHEAD);
        RustAllocator.dealloc(ptr);
    }

    unsafe fn realloc(&mut self, ptr: *mut u8, new_size: usize) -> *mut u8 {
        let old = RustAllocator::usable_size(ptr);
        if new_size > old && !self.fits(new_size - old) {
            return std::ptr::null_mut();
        }
        let moved = RustAllocator.realloc(ptr, new_size);
        if !moved.is_null() {
            self.used = self.used.saturating_sub(old) + RustAllocator::usable_size(moved);
        }
        moved
    }

    unsafe fn usable_size(ptr: *mut u8) -> usize {
        RustAllocator::usable_size(ptr)
    }
}

fn install<'js>(ctx: &Ctx<'js>, tools: &[String], state: &Rc<State>) -> rquickjs::Result<()> {
    let globals = ctx.globals();
    for name in REMOVED_GLOBALS {
        globals.remove(*name)?;
    }
    let st = Rc::clone(state);
    let call = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, name: String, args: Opt<rquickjs::Value<'js>>| {
            host_call(&ctx, &st, &name, args.0)
        },
    )?;
    let st = Rc::clone(state);
    let log = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>, parts: Rest<rquickjs::Value<'js>>| -> rquickjs::Result<()> {
            if st.breach.get().is_some() {
                return Err(Exception::throw_internal(&ctx, "the run was stopped"));
            }
            let mut line = Vec::with_capacity(parts.0.len());
            for part in &parts.0 {
                // An exception (a throwing toJSON, the interrupt inside it)
                // propagates to the script: swallowing it here would clear
                // the interrupt and let a loop around console.log run on.
                line.push(show(&ctx, part)?);
            }
            st.log(line.join(" "));
            Ok(())
        },
    )?;
    let mut options = rquickjs::context::EvalOptions::default();
    options.filename = Some(PRELUDE_FILENAME.into());
    let setup: Function = ctx.eval_with_options(PRELUDE, options)?;
    setup.call::<_, ()>((call, log, tools.to_vec()))?;
    Ok(())
}

/// A console argument as text: a string as is, else its JSON, else its
/// type (`undefined`, `[function]`). Never catches: an exception from the
/// script's own `toJSON` (or the interrupt inside one) is returned.
fn show<'js>(ctx: &Ctx<'js>, value: &rquickjs::Value<'js>) -> rquickjs::Result<String> {
    if let Some(s) = value.as_string() {
        return s.to_string();
    }
    match ctx.json_stringify(value.clone())? {
        Some(text) => text.to_string(),
        None if value.is_undefined() => Ok("undefined".into()),
        None => Ok(format!("[{}]", value.type_name())),
    }
}

/// The native behind `tools[...]` and `callTool`.
fn host_call<'js>(
    ctx: &Ctx<'js>,
    state: &State,
    name: &str,
    args: Option<rquickjs::Value<'js>>,
) -> rquickjs::Result<rquickjs::Value<'js>> {
    if state.breach.get().is_some() {
        return Err(Exception::throw_internal(ctx, "the run was stopped"));
    }
    if name.len() > MAX_TOOL_NAME_BYTES {
        return Err(Exception::throw_type(
            ctx,
            &format!("a tool name is at most {MAX_TOOL_NAME_BYTES} bytes; nothing was called"),
        ));
    }
    if is_nested_tool(name) {
        return Err(Exception::throw_message(ctx, &RunError::Nested.to_string()));
    }
    if state.started.elapsed() > state.limits.overall {
        state.breach.set(Some(Breach::Overall));
        return Err(Exception::throw_internal(
            ctx,
            "the run's time limit passed; no more calls start",
        ));
    }
    if state.calls.borrow().len() >= state.limits.max_calls {
        state.breach.set(Some(Breach::TooManyCalls));
        return Err(Exception::throw_internal(
            ctx,
            &RunError::TooManyCalls {
                max: state.limits.max_calls,
            }
            .to_string(),
        ));
    }
    let args = match args {
        None => json!({}),
        Some(v) if v.is_undefined() || v.is_null() => json!({}),
        Some(v) => {
            let text = ctx
                .json_stringify(v)?
                .map(|s| s.to_string())
                .transpose()?
                .unwrap_or_else(|| "{}".into());
            serde_json::from_str(&text).map_err(|e| {
                Exception::throw_type(ctx, &format!("tool arguments must be JSON: {e}"))
            })?
        }
    };
    let Some(host) = state.host.get() else {
        return Err(Exception::throw_internal(ctx, "the run has ended"));
    };
    // A hung-up client or a refused card stops the run before the next call
    // starts, not only after it (a call that would show a card included).
    if let Some(reason) = host.stop_run() {
        *state.stopped.borrow_mut() = Some(cut_message(reason, MAX_MESSAGE_BYTES));
        state.breach.set(Some(Breach::CardRefused));
        return Err(Exception::throw_internal(ctx, "the run was stopped"));
    }
    let began = Instant::now();
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.call(name, args)))
        .unwrap_or_else(|_| {
            Err(format!(
                "{name} failed inside Hollow (it panicked); nothing more is known."
            ))
        });
    let spent = began.elapsed();
    state.in_host.set(state.in_host.get() + spent);
    let mut record = CallRecord {
        tool: name.to_string(),
        ok: outcome.is_ok(),
        truncated: false,
        millis: millis(spent),
    };
    let result = outcome.and_then(|value| {
        let text = serde_json::to_string(&value).map_err(|e| e.to_string())?;
        if text.len() <= state.limits.max_call_result_bytes {
            return Ok(text);
        }
        record.truncated = true;
        let cut = floor_char_boundary(&text, state.limits.max_call_result_bytes);
        Ok(json!({ "truncated": true, "bytes": text.len(), "preview": &text[..cut] }).to_string())
    });
    state.calls.borrow_mut().push(record);
    if let Some(reason) = host.stop_run() {
        *state.stopped.borrow_mut() = Some(cut_message(reason, MAX_MESSAGE_BYTES));
        state.breach.set(Some(Breach::CardRefused));
        return Err(Exception::throw_internal(ctx, "the run was stopped"));
    }
    if state.started.elapsed() > state.limits.overall {
        state.breach.set(Some(Breach::Overall));
        return Err(Exception::throw_internal(
            ctx,
            "the run passed its overall limit",
        ));
    }
    match result {
        Ok(text) => ctx.json_parse(text),
        Err(refusal) => Err(Exception::throw_message(
            ctx,
            &cut_message(refusal, MAX_MESSAGE_BYTES),
        )),
    }
}

fn floor_char_boundary(text: &str, max: usize) -> usize {
    let mut cut = max.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    cut
}

/// The final value as JSON, within the output cap.
fn output<'js>(
    ctx: &Ctx<'js>,
    value: rquickjs::Value<'js>,
    state: &State,
) -> Result<Value, RunError> {
    let text = match ctx.json_stringify(value) {
        Ok(Some(text)) => text.to_string().map_err(engine)?,
        Ok(None) => "null".to_string(),
        Err(rquickjs::Error::Exception) => {
            let thrown = describe(ctx, &ctx.catch());
            return Err(match classify(state, thrown, false) {
                RunError::Thrown { message, .. } => RunError::NotJson(message),
                other => other,
            });
        }
        Err(other) => return Err(engine(other)),
    };
    if text.len() > state.limits.max_output_bytes {
        return Err(RunError::OutputTooLarge {
            bytes: text.len(),
            max: state.limits.max_output_bytes,
        });
    }
    serde_json::from_str(&text)
        .map_err(|e| RunError::NotJson(cut_message(e.to_string(), MAX_MESSAGE_BYTES)))
}

/// What a thrown value says: its name, message and the script line.
struct Thrown {
    name: String,
    message: String,
    line: Option<u32>,
}

fn describe<'js>(ctx: &Ctx<'js>, value: &rquickjs::Value<'js>) -> Thrown {
    if let Some(error) = value
        .as_object()
        .and_then(|o| Exception::from_object(o.clone()))
    {
        let name: String = error.as_object().get("name").unwrap_or_default();
        let message = error.message().unwrap_or_default();
        let line = error.stack().as_deref().and_then(script_line);
        let _ = ctx.catch();
        return Thrown {
            name,
            message,
            line,
        };
    }
    // Once, after the script has stopped: an exception here (a throwing
    // toJSON) is cleared and the value's type stands in.
    let message = show(ctx, value).unwrap_or_else(|_| {
        let _ = ctx.catch();
        format!("[{}]", value.type_name())
    });
    Thrown {
        name: String::new(),
        message,
        line: None,
    }
}

/// The first `agent-script.js:<line>` frame in a QuickJS stack.
fn script_line(stack: &str) -> Option<u32> {
    let marker = format!("{FILENAME}:");
    stack.match_indices(&marker).find_map(|(at, _)| {
        let before = stack[..at].chars().next_back();
        if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.') {
            return None;
        }
        let digits: String = stack[at + marker.len()..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

/// A thrown value as the run's error, a breach or engine limit first.
fn classify(state: &State, thrown: Thrown, parsing: bool) -> RunError {
    if let Some(breach) = state.breach_error() {
        return breach;
    }
    let lower = thrown.message.to_ascii_lowercase();
    // QuickJS throws `null` when it can't even build the error object, so
    // the allocator's record decides, not the message alone.
    if state.out_of_memory.get() || lower.contains("out of memory") {
        return RunError::Memory {
            limit: state.limits.memory_bytes,
        };
    }
    if lower.contains("stack overflow") || lower.contains("maximum call stack") {
        return RunError::StackOverflow;
    }
    let message = if thrown.name.is_empty() || thrown.message.starts_with(&thrown.name) {
        thrown.message
    } else {
        format!("{}: {}", thrown.name, thrown.message)
    };
    let message = cut_message(message, MAX_MESSAGE_BYTES);
    if parsing {
        RunError::Syntax {
            message,
            line: thrown.line,
        }
    } else {
        RunError::Thrown {
            message,
            line: thrown.line,
        }
    }
}

/// An rquickjs error from a call as the run's error.
fn failure<'js>(ctx: &Ctx<'js>, error: rquickjs::Error, state: &State, parsing: bool) -> RunError {
    match error {
        rquickjs::Error::Exception => {
            let thrown = describe(ctx, &ctx.catch());
            classify(state, thrown, parsing)
        }
        rquickjs::Error::Allocation => state.breach_error().unwrap_or(RunError::Memory {
            limit: state.limits.memory_bytes,
        }),
        other => state.breach_error().unwrap_or_else(|| engine(other)),
    }
}

// ── The broker route ─────────────────────────────────────────────────────

/// Serves a script's calls as Hollow app actions, through the same steps as
/// a direct `hollow_app` call for `caller` (see the module doc).
pub(crate) struct BrokerToolHost<'a> {
    /// Makes the context a handler runs on (`app_broker::dispatch`'s `host`).
    pub host: &'a dyn Fn() -> Box<dyn app_actions::Host>,
    pub vault: &'a Path,
    /// The run the code-mode call itself was authorized for.
    pub caller: Caller,
    /// Shows (or refuses) a card: the real one, bound to the code-mode call.
    pub asker: &'a dyn Asker,
    /// `app_broker::APP_POLICY` in the app; `serve_actions::SERVE_POLICY` in
    /// `hollow serve`.
    pub policy: &'a Policy,
    /// Settings, Connectors, Hollow app (`app_mcp::enabled()`).
    pub enabled: bool,
    /// The refusal of the first card the person didn't allow: the run stops
    /// there ([`ToolHost::stop_run`]) and any later call is refused unsent.
    refused: RefCell<Option<String>>,
}

/// The whole-run limit when served to an agent (`codemode.run`). No call
/// starts after it (`host_call`), so the last call, up to a card's two
/// minutes plus its action, still ends inside the 450 s the MCP server waits
/// for a change-tier answer and Codex's 480 s tool timeout.
pub(crate) const SERVED_OVERALL: Duration = Duration::from_secs(240);

/// Actions a script may not call: they wait longer than the run's whole
/// budget (`sessions.wait` up to 315 s; a sign-in up to 5 minutes), so the
/// client would give up while the script still ran. Call them directly.
fn waits_too_long(id: &str) -> bool {
    id == "sessions.wait" || id.contains("sign_in")
}

/// `codemode.run`, served: runs `args`' script as `caller` through a fresh
/// [`BrokerToolHost`] (seam note h) and answers the report as JSON. The
/// outer call was authorized by `app_broker::authorize` (seam note d) and
/// is audited by `dispatch` like any call, with its arguments digested
/// (seam note f). Runs on the calling thread, which must not be a tokio
/// worker (seam note e), like every handler `execute` runs.
pub(crate) fn serve(
    host: &dyn Fn() -> Box<dyn app_actions::Host>,
    vault: &Path,
    caller: Caller,
    asker: &dyn Asker,
    policy: &Policy,
    enabled: bool,
    args: &Value,
) -> Result<Value, String> {
    let script = args
        .get("script")
        .and_then(Value::as_str)
        .ok_or("codemode.run: `script` is required.")?;
    if !asker.valid() {
        return Err(app_broker::RUN_ENDED.into());
    }
    let mut limits = Limits {
        overall: SERVED_OVERALL,
        ..Limits::default()
    };
    if let Some(bytes) = args.get("maxBytes").and_then(Value::as_u64) {
        limits.max_output_bytes = usize::try_from(bytes).unwrap_or(usize::MAX).clamp(
            crate::mcp_envelope::MIN_MAX_BYTES,
            crate::mcp_envelope::HARD_MAX_BYTES,
        );
    } else {
        limits.max_output_bytes = crate::mcp_envelope::DEFAULT_MAX_BYTES;
    }
    let broker = BrokerToolHost::new(host, vault, caller, asker, policy, enabled);
    let report = run(script, &broker, &limits);
    Ok(app_actions::fit_result(report.to_json()))
}

/// What an inner call is refused with when it names code mode.
const NESTED_REFUSAL: &str = "A code-mode script can't start another code-mode run; nothing ran.";

impl<'a> BrokerToolHost<'a> {
    /// A host for one code-mode run. `caller` must be the identity the
    /// broker's `authorize` returned for the code-mode call itself (see the
    /// seam notes): [`Self::request`] mints a valid proof for whatever
    /// caller it is given.
    pub(crate) fn new(
        host: &'a dyn Fn() -> Box<dyn app_actions::Host>,
        vault: &'a Path,
        caller: Caller,
        asker: &'a dyn Asker,
        policy: &'a Policy,
        enabled: bool,
    ) -> Self {
        BrokerToolHost {
            host,
            vault,
            caller,
            asker,
            policy,
            enabled,
            refused: RefCell::new(None),
        }
    }

    /// The request a direct `run_action` for `id` would carry. Its proof is
    /// minted for the caller's own identity, which the code-mode call was
    /// already authorized under, so it grants nothing new; it lets the
    /// unchanged `authorize` re-check the switch, surface, registry, tier and
    /// arguments.
    fn request(&self, id: &str, args: Value) -> AppRequest {
        let c = &self.caller;
        let grant = AppGrant {
            access: c.access,
            depth: c.depth,
            perm: c.perm.clone(),
            list_depth: c.depth,
        };
        AppRequest {
            v: 1,
            action: id.to_string(),
            run_id: c.run_id.clone(),
            surface: c.surface.clone(),
            scope: c.scope.clone(),
            access: c.access.as_str().to_string(),
            depth: c.depth,
            perm: c.perm.clone(),
            proof: app_broker::proof_for(&c.run_id, &c.surface, &c.scope, &grant),
            tool_args: json!({ "id": id, "args": args.clone() }),
            args,
            tool: "run_action".into(),
        }
    }

    /// The broker's retry guard, for the call made either way an agent
    /// could have made it directly: by its first-class tool or `run_action`.
    fn repeated(&self, req: &AppRequest, caller: &Caller) -> Option<String> {
        if let Some(refused) = app_broker::repeat_refusal(req, caller) {
            return Some(refused);
        }
        let tool = crate::app_mcp::tool_for(req.action.trim())?;
        let direct = AppRequest {
            tool: tool.to_string(),
            tool_args: req.args.clone(),
            ..req.clone()
        };
        app_broker::repeat_refusal(&direct, caller)
    }
}

impl ToolHost for BrokerToolHost<'_> {
    fn call(&self, tool: &str, args: Value) -> Result<Value, String> {
        let id = tool.trim();
        let req = self.request(id, args);
        let asker = Tracked {
            inner: self.asker,
            answered: Cell::new(None),
        };
        let earlier = self.refused.borrow().clone();
        let outcome = if let Some(refused) = earlier {
            Err(refused)
        } else if is_nested_tool(id) {
            Err(NESTED_REFUSAL.to_string())
        } else if waits_too_long(id) {
            Err(format!("{id} waits longer than a script's time limit; call it directly instead of from a script. Nothing ran."))
        } else {
            app_broker::authorize(&req, self.enabled).and_then(|(action, args, caller)| {
                (self.policy.screen)(action)?;
                if let Some(refused) = self.repeated(&req, &caller) {
                    return Err(refused);
                }
                (self.policy.reword)(
                    action.id,
                    app_broker::execute(self.host, self.vault, action, args, caller, &asker),
                )
            })
        };
        let tier = app_actions::find(id).map(|a| a.tier.as_str());
        audit_inner(
            self.vault,
            &req,
            tier,
            asker.answered.get(),
            outcome.as_ref().map(|_| ()).map_err(String::as_str),
        );
        if let (Some(verdict), Err(refusal)) = (asker.answered.get(), &outcome) {
            if verdict != Verdict::Allowed {
                self.refused
                    .borrow_mut()
                    .get_or_insert_with(|| refusal.clone());
            }
        }
        outcome
    }

    fn stop_run(&self) -> Option<String> {
        // Seam note (c): a client that hung up or a revoked attempt ends the
        // script at its next call, not after its whole time limit.
        self.refused.borrow().clone().or_else(|| {
            (!self.asker.valid() || self.asker.hung_up()).then(|| app_broker::RUN_ENDED.to_string())
        })
    }

    fn tools(&self) -> Vec<ToolInfo> {
        let (access, depth) = (self.caller.access, self.caller.depth);
        app_actions::ACTIONS
            .iter()
            .filter(|a| access.allows(a.tier))
            .filter(|a| a.id != "sessions.start_workers" || depth == 0)
            .filter(|a| {
                depth < crate::app_mcp::MAX_START_DEPTH
                    || !crate::app_mcp::tool_for(a.id)
                        .is_some_and(|t| ["start_session", "send_to_session"].contains(&t))
            })
            .filter(|a| !is_nested_tool(a.id) && !waits_too_long(a.id))
            .map(|a| ToolInfo {
                name: a.id.to_string(),
                tier: a.tier.as_str().to_string(),
                summary: app_actions::cut(a.summary.split(". ").next().unwrap_or(a.summary), 160),
                signature: app_actions::signature(a.params),
            })
            .collect()
    }
}

/// The caller's asker, remembering only this inner call's card answer (the
/// app's `AppAsker` keeps the last answer of the whole request, which would
/// leak one inner call's verdict into the next one's audit line).
struct Tracked<'a> {
    inner: &'a dyn Asker,
    answered: Cell<Option<Verdict>>,
}

impl Asker for Tracked<'_> {
    fn valid(&self) -> bool {
        self.inner.valid()
    }
    fn current(&self) -> bool {
        self.inner.current()
    }
    fn permit(&self) -> Option<crate::shared_mcp::Permit> {
        self.inner.permit()
    }
    fn ask(&self, card: &app_actions::Card, caller: &Caller) -> Verdict {
        let verdict = self.inner.ask(card, caller);
        self.answered.set(Some(verdict));
        verdict
    }
    fn binds_grants(&self) -> bool {
        self.inner.binds_grants()
    }
    fn can_show(&self) -> bool {
        self.inner.can_show()
    }
    fn standing_approvals(&self) -> bool {
        self.inner.standing_approvals()
    }
    fn refusal(&self, action: &app_actions::Action) -> String {
        self.inner.refusal(action)
    }
    fn answered(&self) -> Option<Verdict> {
        self.answered.get()
    }
}

/// One audit line for an inner call: `app_broker::audit`'s fields plus
/// `via` and `tool`, so a reader tells it from a direct call. One write of
/// the whole line in append mode; best effort, like the broker's.
fn audit_inner(
    vault: &Path,
    req: &AppRequest,
    tier: Option<&str>,
    card: Option<Verdict>,
    outcome: Result<(), &str>,
) {
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(req.args.to_string().as_bytes()));
    let line = json!({
        "ts": chrono::Utc::now().to_rfc3339(),
        "runId": req.run_id.trim(),
        "surface": req.surface.trim(),
        "scope": req.scope.trim(),
        "depth": req.depth,
        "perm": req.perm.trim(),
        "action": req.action.trim().chars().take(120).collect::<String>(),
        "tier": tier,
        "ok": outcome.is_ok(),
        "card": card.map(Verdict::reason),
        "error": outcome.err().map(|e| crate::claude_runner::redact(&e.chars().take(200).collect::<String>())),
        "argsDigest": &digest[..16],
        "via": "codemode",
        "tool": "codemode",
    });
    if let Ok(mut file) = app_broker::open_audit_append(&vault.join(app_broker::AUDIT_FILE)) {
        use std::io::Write;
        let _ = file.write_all(format!("{line}\n").as_bytes());
    }
}

#[cfg(test)]
#[path = "codemode_tests.rs"]
mod tests;
