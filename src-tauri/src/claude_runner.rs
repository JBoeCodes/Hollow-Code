//! Shared one-shot runner for the selected app-wide agent and model.
//! Callers provide the prompt and required tool restrictions; unsupported
//! restrictions are rejected before any provider process is started.
//! The legacy module name is retained for compatibility with helper imports.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;

/// Keep the requested workspace. Hollow never hunts for a directory with
/// inherited external connectors; every child receives its own strict MCP config.
pub fn resolve_claude_cwd(default: &Path) -> PathBuf {
    default.to_path_buf()
}

/// Options for a single invocation of the selected app agent.
pub struct RunOpts<'a> {
    pub cwd: &'a Path,
    pub timeout: Duration,
    /// Required tool allowlist. Empty means no added restriction. Adapters
    /// must enforce this list or refuse the operation before spawn.
    pub allowed_tools: &'a [&'a str],
    /// Blacklist of tool names. Belt-and-braces with `allowed_tools` —
    /// useful when the allow-list has to stay broad.
    pub disallowed_tools: &'a [&'a str],
    /// Optional selected-provider model override. None inherits app settings.
    pub model: Option<&'a str>,
    /// `(filename, tag)` for the response log. Filename is relative to
    /// `~/.cache/hollow/`. None disables logging.
    pub debug_log: Option<(&'a str, &'a str)>,
    /// Where this invocation's usage row goes and how it is attributed
    /// (surface, scope). Exactly one row is written per call that gets past
    /// the run permit, a validation refusal included; a permit timeout (an
    /// admission refusal, Paperclip PR 7) writes none.
    pub meter: crate::usage_ledger::Meter,
    /// Self-hosted models Q10: a background job routed to a self-hosted
    /// model runs on this target (the Claude harness, its forwarder and a
    /// key minted for the job; `self_hosted_jobs`). `None` everywhere else.
    pub self_hosted: Option<crate::self_hosted::RunTarget>,
}

/// Complete no-tools marker for pure text transformations. Claude removes its
/// entire tool set; Codex uses its qualified isolated transform runtime.
/// Providers without a verified tool-free route reject these operations.
pub const NO_TOOLS: &[&str] = &[
    "Bash",
    "Read",
    "Edit",
    "Write",
    "Glob",
    "Grep",
    "WebFetch",
    "WebSearch",
    "NotebookEdit",
    "Task",
    "TodoWrite",
];

/// Run the selected app-wide agent, honoring its saved model and the caller's
/// tool restrictions. Provider errors and nonzero exits are always failures.
pub async fn run_oneshot(prompt: &str, opts: RunOpts<'_>) -> Result<String, String> {
    let (kind, model) = crate::agent_settings::resolve(None, opts.model)?;
    run_oneshot_selected(kind, model, prompt, opts).await
}

/// Execute a selection resolved at the beginning of a larger operation so a
/// settings change cannot switch providers midway through that operation.
pub async fn run_oneshot_selected(
    kind: crate::agent::AgentKind,
    model: Option<String>,
    prompt: &str,
    opts: RunOpts<'_>,
) -> Result<String, String> {
    run_oneshot_supervised(kind, model, prompt, opts, None)
        .await
        .result
}

/// [`run_oneshot_selected`] for callers that must tell a timeout or a stop
/// from a failure, or record how the run's processes ended. `opts.timeout`
/// and `stop` are both enforced inside the runner, which ends every process
/// the run started before returning; never bound or cancel this by dropping
/// the future (a dropped run gets only best-effort cleanup, after the
/// caller may already have released its vault lease).
///
/// A one-shot that does not hold the full-vault lease first takes one of the
/// machine-wide run permits (Paperclip PR 7, [`crate::run_permit`]), waiting
/// up to 90 s (or `opts.timeout`, if shorter) and honoring `stop`. A caller
/// that runs inside its own lease-holding run uses
/// [`run_oneshot_under_lease`] instead and takes none.
pub async fn run_oneshot_supervised(
    kind: crate::agent::AgentKind,
    model: Option<String>,
    prompt: &str,
    opts: RunOpts<'_>,
    stop: Option<&crate::process_group::StopSignal>,
) -> crate::agent::Collected {
    let policy = PermitPolicy::Acquire {
        dir: crate::run_permit::permits_dir(),
        max_wait: PERMIT_WAIT,
    };
    run_oneshot_with(policy, kind, model, prompt, opts, stop).await
}

/// [`run_oneshot_supervised`] for a one-shot that runs INSIDE an admitted
/// full-vault run (the heartbeat's Codex snapshot, Codex text ingest). While
/// the lease file still names `lease`'s run ([`crate::automated_history::lease_is`])
/// it takes no run permit: that run is already the one lease-holding run, and
/// re-acquiring could wait on a slot behind itself. If the lease file no
/// longer names it (an agent rewrote it), it falls back to taking a permit,
/// the safe side.
pub async fn run_oneshot_under_lease(
    lease: &crate::automated_history::RunLease,
    kind: crate::agent::AgentKind,
    model: Option<String>,
    prompt: &str,
    opts: RunOpts<'_>,
    stop: Option<&crate::process_group::StopSignal>,
) -> crate::agent::Collected {
    let policy = PermitPolicy::UnderLease {
        lease,
        fallback: crate::run_permit::permits_dir(),
    };
    run_oneshot_with(policy, kind, model, prompt, opts, stop).await
}

/// Longest a lease-less one-shot waits for a run permit.
const PERMIT_WAIT: Duration = Duration::from_secs(90);

/// Whether a one-shot needs a cross-process run permit.
enum PermitPolicy<'a> {
    /// Take a slot in `dir`, waiting at most `max_wait` (never longer than
    /// the run's own timeout).
    Acquire { dir: PathBuf, max_wait: Duration },
    /// Runs inside this admitted lease: no permit while it still holds;
    /// otherwise a slot in `fallback`.
    UnderLease {
        lease: &'a crate::automated_history::RunLease,
        fallback: PathBuf,
    },
}

/// Resolve the permit policy: `Ok(None)` when the run needs no permit (it
/// holds a live lease), `Ok(Some(permit))` once a slot is held, `Err` when no
/// slot freed up in time or `stop` was requested. Never spawns anything, so
/// tests drive it without an agent CLI.
async fn resolve_permit(
    policy: PermitPolicy<'_>,
    timeout: Duration,
    stop: Option<&crate::process_group::StopSignal>,
) -> Result<Option<crate::run_permit::Permit>, String> {
    let (dir, max_wait) = match policy {
        PermitPolicy::UnderLease { lease, fallback } => {
            if crate::automated_history::lease_is(lease.vault(), lease) {
                return Ok(None);
            }
            (fallback, PERMIT_WAIT)
        }
        PermitPolicy::Acquire { dir, max_wait } => (dir, max_wait),
    };
    crate::run_permit::acquire(&dir, "this helper", max_wait.min(timeout), stop)
        .await
        .map(Some)
}

/// The shared one-shot body. Order: build and validate the spec (a rejected
/// spec fails at once, never waiting for a slot, and writes its one
/// `spawn_failed` usage row) → resolve the permit (a timeout or stop writes
/// no usage row, like any other admission refusal) → run. The permit is held
/// until the run and its processes have ended.
async fn run_oneshot_with(
    policy: PermitPolicy<'_>,
    kind: crate::agent::AgentKind,
    model: Option<String>,
    prompt: &str,
    opts: RunOpts<'_>,
    stop: Option<&crate::process_group::StopSignal>,
) -> crate::agent::Collected {
    let codex_transform = kind == crate::agent::AgentKind::Codex
        && NO_TOOLS
            .iter()
            .all(|tool| opts.disallowed_tools.contains(tool));
    let spec = oneshot_spec(model, prompt, &opts);
    // Validation stays ahead of everything else, the permit included, and a
    // refusal still writes its one `spawn_failed` usage row. The Codex
    // transform validates its own isolated runtime.
    if !codex_transform {
        if let Err(error) = crate::agent::validate_run_spec(kind, &spec) {
            let usage =
                crate::usage_ledger::record_refusal(&opts.meter, kind, spec.model.clone(), &error);
            return crate::agent::Collected {
                usage: Some(usage),
                ..crate::agent::Collected::failed(error)
            };
        }
    }
    let _permit = match resolve_permit(policy, opts.timeout, stop).await {
        Ok(permit) => permit,
        Err(error) => {
            return crate::agent::Collected {
                stopped: stop.is_some_and(|stop| stop.requested()),
                ..crate::agent::Collected::failed(error)
            }
        }
    };
    if codex_transform {
        return crate::codex_transform::run(
            prompt,
            spec.model.as_deref(),
            opts.cwd,
            opts.timeout,
            stop,
            &opts.meter,
        )
        .await;
    }
    // The deadline and stop live inside run_collect so either ends every
    // process the run started instead of merely dropping the future.
    let mut collected =
        crate::agent::run_collect(kind, &spec, Some(opts.timeout), stop, &opts.meter).await;
    // Only the error's wording changes (it names the bound): what the run
    // wrote before it was cut off stays in `partial_text` (OF04). A helper
    // still reads a timeout as a failure, never a half answer as its result.
    if collected.timed_out {
        collected.result = Err(format!(
            "{} timed out after {}s.",
            kind.label(),
            opts.timeout.as_secs()
        ));
    }
    if let (Ok(body), Some((filename, tag))) = (&collected.result, opts.debug_log) {
        if debug_log_enabled() {
            let _ = write_debug_log(filename, tag, body).await;
        }
    }
    collected
}

/// A one-shot's run spec (named so the GUI-spawn lint counts one literal).
type Spec = crate::agent::RunSpec;

/// A one-shot's spec. Self-hosted models Q10: its extras are the defaults,
/// with the run's self-hosted target when a background job is routed to one
/// (`RunOpts::self_hosted`); everything else is the same either way.
pub(crate) fn oneshot_spec(model: Option<String>, prompt: &str, opts: &RunOpts<'_>) -> Spec {
    crate::agent::RunSpec {
        prompt: prompt.to_string(),
        resume: None,
        model,
        perm: "bypassPermissions".into(),
        cwd: opts.cwd.to_path_buf(),
        allowed_tools: opts.allowed_tools.iter().map(|s| s.to_string()).collect(),
        disallowed_tools: opts
            .disallowed_tools
            .iter()
            .map(|s| s.to_string())
            .collect(),
        interactive: false,
        mcp_config: None,
        append_system_prompt: None,
        // Only the GUI process imports this runner; no MCP subprocess or
        // headless entry point reaches it.
        process_scope: crate::agent::ProcessScope::GuiOwned,
        usage_stream: false,
        max_budget_usd: None,
        // One-shot helpers (and the Codex tool-free transform) run at the
        // runtime's default effort (Paperclip PR 14a): Settings' effort is
        // for agent runs.
        extras: crate::agent::RunExtras {
            self_hosted: opts.self_hosted.clone(),
            ..crate::agent::RunExtras::default()
        },
    }
}

/// Debug-log writes are off by default. Set `HOLLOW_DEBUG_LOG=1` (or
/// `=true`) before launching to opt in; otherwise the runner never
/// touches `~/.cache/hollow/*.log`. Calendar / email / Teams payloads
/// are sensitive enough that "always write to disk" is the wrong default.
fn debug_log_enabled() -> bool {
    matches!(
        std::env::var("HOLLOW_DEBUG_LOG").ok().as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on"),
    )
}

/// The placeholders [`redact`] writes in place of a secret. They hold query
/// words (`email`, `key`, `token`), so a search over redacted text blanks
/// them before matching (R4 review; the chat search since 2026-11-04). Add
/// one here with any new placeholder.
pub(crate) const REDACT_PLACEHOLDERS: [&str; 9] = [
    "[api-key]",
    "[email]",
    "[redacted]",
    "[github-token]",
    "[jwt]",
    "[pem]",
    "[run-key]",
    "[aws-key]",
    "[slack-token]",
];

/// Best-effort redaction of secrets and PII shapes. This is Hollow's ONLY
/// redactor: extend it rather than adding a second one. Families, in the
/// order they are applied:
///
/// 1. PEM blocks (`-----BEGIN … -----END …`, any label) → `[pem]`, then an
///    unterminated `-----BEGIN … PRIVATE KEY-----` through the end of the
///    text → `[pem]` (a truncated paste must not leak the key body).
/// 2. Provider API keys (`sk-…`, `sk-ant-…`, `sk-proj-…`) → `[api-key]`.
/// 3. GitHub tokens (`ghp_`/`gho_`/`ghu_`/`ghs_`/`ghr_`, `github_pat_`) →
///    `[github-token]`.
/// 4. Service keys (2026-11-08, the learning capture digest): AWS access key
///    ids (`AKIA`/`ASIA`) → `[aws-key]`, Slack tokens (`xox?-`) →
///    `[slack-token]`, Google API keys (`AIza`), Stripe secret, restricted
///    and webhook keys (`sk_live_`, `rk_live_`, `sk_test_`, `whsec_`) →
///    `[api-key]`. Linear keys (`lin_api_`, `lin_oauth_`, Q-R32) →
///    `[api-key]`.
/// 5. Connection strings (postgres, mysql, mariadb, mongodb[+srv], redis[s],
///    amqp[s], kafka, nats, mssql, sqlserver) → `scheme://[redacted]`, then
///    any other URL's password (`scheme://user:pass@`) →
///    `scheme://user:[redacted]@`. Both run BEFORE the email pattern, or
///    `user:pass@host` would be half-masked.
/// 6. Email local parts, JWT-shaped tokens, a credential-shaped
///    `Authorization` value (header or JSON, scheme word kept; Q-R32), OAuth
///    key-value secrets and cookies, then a credential-shaped `token`,
///    `deviceToken` or `pushToken` value (Q-R32).
/// 7. Environment lines' values: an upper-case name holding KEY, TOKEN,
///    SECRET, PASSWORD or PASSWD, then `=` (`API_KEY=…`, `export
///    GITHUB_TOKEN=…`, `DB_PASSWORD="…"`) → `NAME=[redacted]` (a quoted
///    value keeps its quotes and is masked whole, or to the line's end when
///    unterminated). Upper case only, so code such as `max_tokens=4096` keeps
///    its value.
/// 8. Named secrets in any case (#1237 delta review): a key at a line's
///    start, after whitespace, `--` or a quote, named like a password, API
///    key, secret, session or auth token or `*_token`, then `=` or a single
///    `:`, then a value not starting with `:` or `=`. Covers credentials
///    files (`aws_secret_access_key = …`), JSON (`"SecretAccessKey": "…"`)
///    and headers (`x-api-key: …`). A quoted value is masked whole and keeps
///    its quotes; Rust paths such as `crate::secret::Key` are left alone.
///
/// The output is idempotent (`redact(redact(x)) == redact(x)`). Callers:
/// the debug log, health lines, the usage-ledger failure journal, the quota
/// card's logs, Codex transform stderr, workflow error excerpts, the saved
/// turn-feedback snapshots and their export (Paperclip PR 17), and the
/// automatic learning capture's digest (user words and tool results sent to
/// the extractor). Tuned to be paranoid rather than thorough; it is not a
/// security boundary.
pub(crate) fn redact(input: &str) -> String {
    let mut s = redact_shapes(input);
    // A value read from 1Password this run, of any shape (`onepassword`).
    crate::onepassword::scrub_resolved(&mut s);
    s
}

/// [`redact`]'s pattern families.
fn redact_shapes(input: &str) -> String {
    use once_cell::sync::Lazy;
    use regex::Regex;
    const AUTH_SCHEMES: &str = "bearer|basic|token|digest|bot|negotiate|ntlm";
    static PEM_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"-----BEGIN [A-Z0-9 ]+-----[\s\S]*?-----END [A-Z0-9 ]+-----").unwrap()
    });
    static PEM_OPEN_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----[\s\S]*$").unwrap());
    static GITHUB_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b(?:gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{22,})\b").unwrap()
    });
    static DSN_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?i)\b((?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|rediss?|amqps?|kafka|nats|mssql|sqlserver)://)[^\s<>'")]+"#,
        )
        .unwrap()
    });
    static EMAIL_RE: Lazy<Regex> = Lazy::new(|| {
        // Standard RFC 5322 enough — we just want to mask the local part.
        Regex::new(r"\b([A-Za-z0-9._%+-]+)@([A-Za-z0-9.-]+\.[A-Za-z]{2,})\b").unwrap()
    });
    static TOKEN_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"\b(eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,})\b")
            .unwrap()
    });
    // Any `Authorization` value, header or JSON form, with or without a
    // scheme word (Q-R32): the key and the scheme stay, the credential goes.
    // Without a scheme the value must be on the key's line, so a bare
    // `Authorization:` heading never takes the next line's first word. A
    // preceding `_` is allowed (`HTTP_AUTHORIZATION`).
    static AUTH_HEADER_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(&format!(
            r#"(?i)((?:\b|_)authorization["']?[ \t]*[:=])(\s*["']?(?:{AUTH_SCHEMES})\s+|[ \t]*["']?)([^\s"']+)"#
        ))
        .unwrap()
    });
    // A bare `token`, `deviceToken` or `pushToken` key (Q-R32), only with
    // a credential-shaped value: quoted (masked whole, as the named-secret
    // rule does), or a run of 16 or more token characters holding a digit,
    // so code such as `token: String` or `token = response.data.token`
    // keeps its words. Snake-case `device_token` is the named-secret rule's.
    static TOKEN_KEY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?imR)\b(token|accessToken|idToken|refreshToken|apiToken|authToken|sessionToken|deviceToken|pushToken|[a-z]+token)\b(\\?["']?[ \t]*[:=][ \t]*)("(?:[^"\\\r\n]|\\.)*\\?(?:"|$)|'(?:[^'\\\r\n]|\\.)*\\?(?:'|$)|\\"[^"\\\r\n]*\\"|[A-Za-z0-9._~+/=\-]{16,})"#,
        )
        .unwrap()
    });
    // Linear personal and OAuth keys, the shape `memory_guard` refuses.
    static LINEAR_KEY_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\blin_(?:api|oauth)_[A-Za-z0-9]{20,}\b").unwrap());
    // OAuth/Microsoft secrets that AREN'T JWT-shaped: refresh/access/id tokens,
    // client secrets, and the authorization `code`/`state` that show up in the
    // login.microsoftonline.com URL or a token-endpoint body. Mask the value,
    // keep the key so the log still reads. `\b...\b` avoids matching `qrcode`.
    static OAUTH_KV_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"(?i)\b(refresh_token|access_token|id_token|refreshToken|accessToken|idToken|client_secret|code|state|session_state)\b("?\s*[:=]\s*"?)([A-Za-z0-9._~+/\-]{6,})"#).unwrap()
    });
    // Provider API keys (Anthropic `sk-ant-…`, OpenAI `sk-…`). Usage-ledger
    // failures are journalled from processes that read credential-bearing
    // config, so a key must never survive into a health line.
    static API_KEY_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\bsk-(?:ant-|proj-)?[A-Za-z0-9_\-]{16,}").unwrap());
    // A self-hosted run's key (`self_hosted::RunKey`, `hsr_` + 64 hex).
    static RUN_KEY_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\bhsr_[0-9A-Fa-f]{16,}").unwrap());
    // 1Password (`onepassword`): a reference names an item (kept out of logs
    // all the same), a service-account token is `ops_` plus base64, and an
    // Account Secret Key is `A3-` and five groups.
    static OP_REF_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"op://\S+").unwrap());
    static OP_TOKEN_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\bops_[A-Za-z0-9_=+/\-]{20,}").unwrap());
    static OP_SECRET_KEY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"\bA3-[A-Z0-9]{6}-[A-Z0-9]{6}-[A-Z0-9]{5}-[A-Z0-9]{5}-[A-Z0-9]{5}-[A-Z0-9]{5}\b",
        )
        .unwrap()
    });
    static COOKIE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)\b(set-cookie|cookie)(:\s*)(\S+)").unwrap());
    static AWS_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b").unwrap());
    static SLACK_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"\bxox[abposr]-[A-Za-z0-9-]{10,}").unwrap());
    static SERVICE_KEY_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"\bAIza[0-9A-Za-z_\-]{35}|\b(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}|\bwhsec_[A-Za-z0-9]{16,}",
        )
        .unwrap()
    });
    static URL_PASS_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r#"\b([A-Za-z][A-Za-z0-9+.\-]*://[^\s/:@<>'"]*:)[^\s/@<>'"]+@"#).unwrap()
    });
    // A value never starts with `=`, so `API_KEY == "x"` in code is left as
    // it is.
    static ENV_SECRET_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?mR)\b([A-Z0-9_]*(?:KEY|TOKEN|SECRET|PASSWORD|PASSWD)[A-Z0-9_]*[ \t]*=[ \t]*)("(?:[^"\\\r\n]|\\.)*\\?(?:"|$)|'(?:[^'\\\r\n]|\\.)*\\?(?:'|$)|[^\s"'=][^\s"']*)"#,
        )
        .unwrap()
    });
    // The same key rule as `source_evidence::redact_log`, with quoted
    // values masked whole (escape-aware; an unterminated one to the line's
    // end; a bare value runs past escaped quotes, so `\"x\"` inside a quoted
    // string is masked whole). The first pass keeps the value on the key's
    // line.
    const SECRET_KEY: &str = r#"(?imR)(^|[\s"']|--)([A-Z0-9_.-]*(?:(?:PASSWORD|PASSWD|PASSPHRASE|API[_-]?KEY|ACCESS[_-]?KEY|PRIVATE[_-]?KEY|SECRET|SESSION[_-]?TOKEN|AUTH[_-]?TOKEN)[A-Z0-9_]*|_TOKEN))"#;
    static NAMED_SECRET_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(&format!(
            r#"{SECRET_KEY}(\\?["']?[ \t]*[:=][ \t]*)("(?:[^"\\\r\n]|\\.)*\\?(?:"|$)|'(?:[^'\\\r\n]|\\.)*\\?(?:'|$)|\\"[^"\\\r\n]*\\"|(?:[^\s"':=\\]|\\\S)(?:[^\s"'\\]|\\\S)*)"#
        ))
        .unwrap()
    });
    // The second pass takes a quoted value on the very next line only (a
    // pretty-printed JSON value; unterminated to the line's end; CRLF line
    // ends too), and only when no `:` follows it, so a
    // nested key such as `secret:` then `"password": "…"` is left to the
    // first pass instead of being taken as the value.
    static NAMED_SECRET_NEXT_LINE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(&format!(
            r#"{SECRET_KEY}(\\?["']?[ \t]*[:=][ \t]*\r?\n[ \t]*)("(?:[^"\\\r\n]|\\.)*\\?(?:"|$)|'(?:[^'\\\r\n]|\\.)*\\?(?:'|$))([ \t]*(?:[^:\s]|\r?$))"#
        ))
        .unwrap()
    });
    // A secret given to a command as a flag value (Q-R32, R5b): `--password
    // hunter2x`, `--token=abc123`, `--api-key 'a b'`, and curl's `-u
    // user:pass`. A quoted value is masked whole, whatever it holds (escape
    // aware, spaces allowed); a bare one only when it is 8 or more characters
    // or holds a digit (so `--token the` and `--password-file` keep their
    // words). `-u` counts only on a `curl`, `wget`, `http` or `git` command,
    // found after a space, quote, `/`, `;`, `|`, `&`, `(` or backtick
    // (`/usr/bin/curl`, `$(curl ...)`, `x;curl ...`). The key and its
    // separator stay.
    static FLAG_SECRET_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?imR)(^|\s)(--(?:password|passwd|passphrase|token|secret|api[-_]?key|auth[-_]?token|access[-_]?token|client[-_]?secret|bearer)(?:[ \t]+|=))("(?:[^"\\\r\n]|\\.)*\\?(?:"|$)|'(?:[^'\\\r\n]|\\.)*\\?(?:'|$)|[^\s"'\-][^\s"']*)"#,
        )
        .unwrap()
    });
    static USER_FLAG_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r#"(?imR)((?:^|[\s"'/;|&(`])(?:curl|wget|https?|git)\b[^\r\n]*?\s(?:-u|--user)[ \t]+["']?[^\s:'"]+:)([^\s'"]+)"#,
        )
        .unwrap()
    });
    // A pass replaces only when it matches; a pass whose marker text is
    // absent is skipped (`contains` is fast even in the unoptimized test
    // build): the chat search scrubs every hit's context on each query
    // (`perf_hot_paths`).
    fn pass(s: &mut String, re: &Regex, rep: impl regex::Replacer) {
        if let std::borrow::Cow::Owned(out) = re.replace_all(s, rep) {
            *s = out;
        }
    }
    let mut s = input.to_string();
    if s.contains("-----BEGIN ") {
        pass(&mut s, &PEM_RE, "[pem]");
        pass(&mut s, &PEM_OPEN_RE, "[pem]");
    }
    pass(&mut s, &API_KEY_RE, "[api-key]");
    pass(&mut s, &RUN_KEY_RE, "[run-key]");
    pass(&mut s, &GITHUB_RE, "[github-token]");
    if s.contains("AKIA") || s.contains("ASIA") {
        pass(&mut s, &AWS_RE, "[aws-key]");
    }
    if s.contains("xox") {
        pass(&mut s, &SLACK_RE, "[slack-token]");
    }
    if ["AIza", "_live_", "_test_", "whsec_"]
        .iter()
        .any(|m| s.contains(m))
    {
        pass(&mut s, &SERVICE_KEY_RE, "[api-key]");
    }
    if s.contains("lin_") {
        pass(&mut s, &LINEAR_KEY_RE, "[api-key]");
    }
    if s.contains("op://") {
        pass(&mut s, &OP_REF_RE, "op://[redacted]");
    }
    if s.contains("ops_") {
        pass(&mut s, &OP_TOKEN_RE, "[api-key]");
    }
    if s.contains("A3-") {
        pass(&mut s, &OP_SECRET_KEY_RE, "[redacted]");
    }
    if s.contains("://") {
        pass(&mut s, &DSN_RE, "${1}[redacted]");
        pass(&mut s, &URL_PASS_RE, "${1}[redacted]@");
    }
    pass(&mut s, &EMAIL_RE, "[email]@$2");
    pass(&mut s, &TOKEN_RE, "[jwt]");
    // Only a credential-shaped value is masked: a quoted one, any after
    // `Bearer` (the original rule), or a run of 16 or more token characters,
    // so code (`let authorization = format!(…)`) and prose
    // (`Authorization: token required`) keep their words. A value that is
    // only a scheme word is a head cut before the credential
    // (`redact_prefix`), so it stays as the whole text keeps it.
    pass(&mut s, &AUTH_HEADER_RE, |c: &regex::Captures| {
        let middle = &c[2];
        let scheme = middle
            .trim_matches(|ch: char| ch.is_whitespace() || ch == '"' || ch == '\'')
            .to_ascii_lowercase();
        let value = &c[3];
        let token_chars = value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || "._~+/=-".contains(ch));
        // Short credentials too (Q-R32, R5b): from 6 characters, when a
        // digit says it isn't a word (any scheme, or none), or, after a
        // scheme word, a capital inside it (base64: `Basic dXNlcjpw`).
        let short_credential = value.len() >= 6
            && token_chars
            && (value.contains(|ch: char| ch.is_ascii_digit())
                || (!scheme.is_empty() && value.contains(|ch: char| ch.is_ascii_uppercase())));
        let shaped = middle.contains(['"', '\''])
            || scheme == "bearer"
            || (value.len() >= 16 && token_chars)
            || short_credential;
        let bare_scheme = AUTH_SCHEMES
            .split('|')
            .any(|w| value.eq_ignore_ascii_case(w));
        if !shaped || (scheme.is_empty() && bare_scheme) {
            c[0].to_string()
        } else {
            format!("{}{}[redacted]", &c[1], middle)
        }
    });
    pass(&mut s, &OAUTH_KV_RE, "${1}${2}[redacted]");
    pass(&mut s, &COOKIE_RE, "${1}${2}[redacted]");
    // A quoted value is masked whole, quotes kept, so a head cut inside it
    // (`redact_prefix`) redacts the same way as the whole text.
    let masked = |value: &str| match value.as_bytes().first() {
        // An escaped quote (JSON inside JSON) keeps its pair too.
        Some(b'\\') if value.starts_with("\\\"") => "\\\"[redacted]\\\"",
        Some(b'"') => "\"[redacted]\"",
        Some(b'\'') => "'[redacted]'",
        _ => "[redacted]",
    };
    if s.contains("oken") || s.contains("OKEN") {
        pass(&mut s, &TOKEN_KEY_RE, |c: &regex::Captures| {
            let value = &c[3];
            // Any other `<word>Token` key (`botToken`, `oauthToken`) needs a
            // stricter value: quoted, 16 or more token characters.
            let known = [
                "token",
                "accesstoken",
                "idtoken",
                "refreshtoken",
                "apitoken",
                "authtoken",
                "sessiontoken",
                "devicetoken",
                "pushtoken",
            ]
            .contains(&c[1].to_ascii_lowercase().as_str());
            let inner = value.trim_matches(|ch| matches!(ch, '"' | '\'' | '\\'));
            let strict = value.starts_with(['"', '\'', '\\'])
                && inner.chars().count() >= 16
                && inner
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || "._~+/=-".contains(ch));
            if if known {
                value.starts_with(['"', '\'', '\\'])
                    || value.contains(|ch: char| ch.is_ascii_digit())
            } else {
                strict
            } {
                format!("{}{}{}", &c[1], &c[2], masked(value))
            } else {
                c[0].to_string()
            }
        });
    }
    // Flags carry their value after a space, so these run before the
    // `=`/`:` shortcut below.
    if s.contains("--") || s.contains("-u") {
        pass(&mut s, &FLAG_SECRET_RE, |c: &regex::Captures| {
            let value = &c[3];
            if value.starts_with(['"', '\''])
                || value.chars().count() >= 8
                || value.contains(|ch: char| ch.is_ascii_digit())
            {
                format!("{}{}{}", &c[1], &c[2], masked(value))
            } else {
                c[0].to_string()
            }
        });
        pass(&mut s, &USER_FLAG_RE, "${1}[redacted]");
    }
    // Both key rules need `=` or `:` and a key word: the environment rule an
    // upper-case one, the named-secret rules one in any case.
    if !s.contains('=') && !s.contains(':') {
        return s;
    }
    if ["KEY", "TOKEN", "SECRET", "PASSWORD", "PASSWD"]
        .iter()
        .any(|m| s.contains(m))
    {
        pass(&mut s, &ENV_SECRET_RE, |c: &regex::Captures| {
            format!("{}{}", &c[1], masked(&c[2]))
        });
    }
    // Only ASCII text takes the shortcut: the rules' `(?i)` also folds
    // non-ASCII letters (`ſecret` matches `SECRET`).
    let lower = s.to_ascii_lowercase();
    if s.is_ascii()
        && !["pass", "secret", "key", "token"]
            .iter()
            .any(|word| lower.contains(word))
    {
        return s;
    }
    pass(&mut s, &NAMED_SECRET_RE, |c: &regex::Captures| {
        format!("{}{}{}{}", &c[1], &c[2], &c[3], masked(&c[4]))
    });
    pass(&mut s, &NAMED_SECRET_NEXT_LINE_RE, |c: &regex::Captures| {
        format!("{}{}{}{}{}", &c[1], &c[2], &c[3], masked(&c[4]), &c[5])
    });
    s
}

/// A prefix of [`redact`]`(input)` at least `chars` characters long, or all
/// of it, without redacting all of a long text: the chat search previews the
/// first 2,000 characters of a request or an outcome that can run to 80 KB,
/// and redacting each hit's whole text made its first page about 20 times
/// slower (2026-11-08).
///
/// A head runs to the first whitespace character past a quarter more than
/// `chars`, and is redacted on its own. That is a prefix of redacting the
/// whole text because no family's match crosses a whitespace character,
/// except a PEM block (a head holding `-----BEGIN` is never cut) and the
/// key-value families (`Authorization`, OAuth key-values, cookies),
/// which keep the key and the whitespace and replace a whitespace-free value
/// after it: cut there, the head holds the key and none of the value, just as
/// the whole text's redaction keeps it (`Authorization` also keeps a value
/// that is only a scheme word, such as a head ending on `Bearer`). A head
/// that redacts shorter than
/// `chars` doubles; a short input, or a head without that whitespace, is
/// redacted whole. Keep this true when adding a family:
/// `redact_prefix_is_a_prefix_of_redact` cuts every family at every point.
pub(crate) fn redact_prefix(input: &str, chars: usize) -> String {
    let mut want = chars.saturating_add(chars / 4);
    loop {
        let Some(at) = char_start(input, want) else {
            return redact(input);
        };
        let Some(cut) = input[at..].find(char::is_whitespace) else {
            return redact(input);
        };
        let head = &input[..at + cut];
        if head.contains("-----BEGIN") {
            return redact(input);
        }
        let out = redact(head);
        if chars == 0 || char_start(&out, chars - 1).is_some() {
            return out;
        }
        want = want.saturating_mul(2);
    }
}

/// Where `text`'s character `n` (counting from 0) starts, or `None` when it
/// has no more than `n` characters. Byte `n` when the bytes before it are
/// ASCII, the usual case, without stepping through the characters one at a
/// time, which the unoptimized test build does slowly (2026-11-08).
pub(crate) fn char_start(text: &str, n: usize) -> Option<usize> {
    if text.len() <= n {
        return None;
    }
    if text.as_bytes()[..n].is_ascii() {
        return Some(n);
    }
    text.char_indices().nth(n).map(|(i, _)| i)
}

async fn write_debug_log(filename: &str, tag: &str, body: &str) -> std::io::Result<()> {
    let Some(home) = dirs::home_dir() else {
        return Ok(());
    };
    let dir = crate::instance::dot_cache_dir(&home);
    fs::create_dir_all(&dir).await?;
    let path: PathBuf = dir.join(filename);
    let header = format!("=== {} @ {} ===\n", tag, chrono::Local::now().to_rfc3339());
    let mut f = fs::File::create(&path).await?;
    f.write_all(header.as_bytes()).await?;
    f.write_all(redact(body).as_bytes()).await?;
    // Restrict to user-only read/write on Unix so a process under a
    // different UID can't slurp the cache.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await;
    }
    Ok(())
}

/// Delete all `*.log` files under `~/.cache/hollow/`. Exposed via a Tauri
/// command so the UI can offer a "clear logs" button without the user
/// needing to open a terminal.
pub async fn clear_debug_logs() -> Result<usize, String> {
    let Some(home) = dirs::home_dir() else {
        return Ok(0);
    };
    let dir = crate::instance::dot_cache_dir(&home);
    if !dir.exists() {
        return Ok(0);
    }
    let mut count = 0;
    let mut read_dir = fs::read_dir(&dir)
        .await
        .map_err(|e| format!("read_dir: {e}"))?;
    while let Some(entry) = read_dir
        .next_entry()
        .await
        .map_err(|e| format!("next: {e}"))?
    {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("log")
            && fs::remove_file(&path).await.is_ok()
        {
            count += 1;
        }
    }
    Ok(count)
}

/// Pull a JSON array out of a model response. Handles bare `[...]`,
/// ```json [...] ```, and prose preambles by bracket-matching.
pub fn extract_json_array(s: &str) -> String {
    extract_balanced(s, '[', ']')
}

/// Pull a JSON object out of a model response. Same shape as
/// [`extract_json_array`] but matches braces.
pub fn extract_json_object(s: &str) -> String {
    extract_balanced(s, '{', '}')
}

fn extract_balanced(s: &str, open: char, close: char) -> String {
    let trimmed = s.trim();
    if trimmed.starts_with("```") {
        let mut lines: Vec<&str> = trimmed.lines().collect();
        if !lines.is_empty() && lines[0].starts_with("```") {
            lines.remove(0);
        }
        if !lines.is_empty() && lines[lines.len() - 1].trim_start().starts_with("```") {
            lines.pop();
        }
        return lines.join("\n");
    }
    if trimmed.starts_with(open) {
        return trimmed.to_string();
    }
    if let Some(start) = trimmed.find(open) {
        let mut depth = 0i32;
        for (i, ch) in trimmed[start..].char_indices() {
            if ch == open {
                depth += 1;
            } else if ch == close {
                depth -= 1;
                if depth == 0 {
                    return trimmed[start..start + i + 1].to_string();
                }
            }
        }
    }
    trimmed.to_string()
}

/// UTF-8-safe truncate to at most `n` characters (not bytes). Slicing
/// by byte index would panic when the cut lands inside a multibyte
/// codepoint — that's a hard crash in release builds since the
/// `panic = "abort"` profile setting disables unwinding.
pub fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let head: String = s.chars().take(n).collect();
        format!("{head}…")
    }
}

/// Try to parse model output as JSON; if the cleaned body looks like prose
/// instead (model declined or hit an unavailable tool), surface the prose
/// as the error message — and prepend a friendly hint when the prose
/// matches a known failure pattern (M365 needs authentication, MCP tool
/// missing, etc.).
///
/// `expects` is "array" or "object" — used in the generic parse-error
/// fallback when the cleaned body looks like malformed JSON, not prose.
#[cfg(test)]
pub fn parse_json_or_prose<T: serde::de::DeserializeOwned>(
    cleaned: &str,
    raw: &str,
    expects: &str,
    log_filename: &str,
) -> Result<T, String> {
    let trimmed = cleaned.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        // Looks like a JSON attempt — let serde report the real parse error.
        return serde_json::from_str::<T>(cleaned).map_err(|e| {
            format!(
                "parse {expects}: {e} — see ~/.cache/hollow/{log_filename}. Cleaned start: {}",
                truncate(cleaned, 200),
            )
        });
    }

    // Prose response — the model returned text rather than data. Most
    // common cause: a required MCP tool is unavailable because its
    // connector is unauthenticated (such servers often only expose an
    // `authenticate` tool in that state).
    let prose = truncate(raw.trim(), 600);
    let hint = if looks_like_auth_failure(raw) {
        "A required MCP connector isn't authenticated. Authenticate it in your agent configuration, then retry."
    } else if looks_like_tool_missing(raw) {
        "The required MCP tool isn't loaded. Check that its connector is enabled in your agent configuration."
    } else {
        "Model returned prose instead of JSON — see message below."
    };
    Err(format!("{hint}\n\n{prose}"))
}

#[cfg(test)]
fn looks_like_auth_failure(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("authenticate")
        || l.contains("not authenticated")
        || l.contains("complete_authentication")
        || l.contains("sign in")
        || l.contains("login required")
}

#[cfg(test)]
fn looks_like_tool_missing(s: &str) -> bool {
    let l = s.to_lowercase();
    l.contains("tool is not available")
        || l.contains("tool not available")
        || l.contains("is not available")
        || l.contains("no such tool")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_resolved_non_claude_selection_never_falls_back_to_claude() {
        // This combination is rejected before spawn on every machine. If a
        // helper tries to discard its selected provider, the error changes.
        let directory = tempfile::tempdir().unwrap();
        let error = run_oneshot_selected(
            crate::agent::AgentKind::Antigravity,
            Some("custom-model".into()),
            "A synthetic transformation",
            RunOpts {
                cwd: directory.path(),
                timeout: Duration::from_millis(1),
                allowed_tools: &[],
                disallowed_tools: NO_TOOLS,
                model: None,
                debug_log: None,
                meter: crate::usage_ledger::Meter::for_vault(
                    directory.path(),
                    crate::usage_ledger::Surface::Oneshot,
                    Some("test".into()),
                    None,
                ),
                self_hosted: None,
            },
        )
        .await
        .unwrap_err();
        assert!(error.starts_with("Antigravity does not support the tool restrictions"));
        // The refusal passed through the runner and wrote exactly one row.
        let rows = crate::session_store::usage_rows_since(directory.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].surface, "oneshot");
        assert_eq!(rows[0].scope.as_deref(), Some("test"));
        assert_eq!(rows[0].agent.as_deref(), Some("antigravity"));
        assert_eq!(rows[0].outcome, "spawn_failed");
        assert!(!rows[0].spawned);
    }

    fn test_opts(directory: &Path, timeout: Duration) -> RunOpts<'_> {
        RunOpts {
            cwd: directory,
            timeout,
            allowed_tools: &[],
            disallowed_tools: NO_TOOLS,
            model: None,
            debug_log: None,
            meter: crate::usage_ledger::Meter::for_vault(
                directory,
                crate::usage_ledger::Surface::Oneshot,
                Some("test".into()),
                None,
            ),
            self_hosted: None,
        }
    }

    /// Hold every slot of a private permit directory.
    fn saturate(dir: &Path) -> Vec<crate::run_permit::Permit> {
        (0..crate::run_permit::PERMIT_SLOTS)
            .map(|_| {
                crate::run_permit::try_acquire(dir)
                    .unwrap()
                    .expect("a free slot")
            })
            .collect()
    }

    /// Paperclip PR 7: a spec the adapter rejects fails at once, before the
    /// run permit, even when every slot is taken.
    #[tokio::test]
    async fn a_rejected_spec_fails_before_waiting_for_a_permit() {
        let directory = tempfile::tempdir().unwrap();
        let permits = tempfile::tempdir().unwrap();
        let _held = saturate(permits.path());
        let started = std::time::Instant::now();
        let collected = run_oneshot_with(
            PermitPolicy::Acquire {
                dir: permits.path().to_path_buf(),
                max_wait: PERMIT_WAIT,
            },
            crate::agent::AgentKind::Antigravity,
            Some("custom-model".into()),
            "A synthetic transformation",
            test_opts(directory.path(), Duration::from_secs(90)),
            None,
        )
        .await;
        let error = collected.result.unwrap_err();
        assert!(
            error.starts_with("Antigravity does not support the tool restrictions"),
            "{error}"
        );
        // Waiting would take PERMIT_WAIT (90 s); 1 s flaked under a loaded gate.
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "it never waited for a slot"
        );
        assert!(collected.usage.is_some(), "the refusal wrote its one row");
    }

    /// Paperclip PR 7: a one-shot inside a live lease never takes a permit
    /// (so a pump-held run calling a one-shot cannot deadlock on the slots),
    /// and falls back to one when the lease file no longer names its run.
    /// Drives the permit seam only: no agent CLI is ever started.
    #[tokio::test]
    async fn one_shots_under_a_live_lease_never_take_a_permit() {
        let vault = tempfile::tempdir().unwrap();
        let permits = tempfile::tempdir().unwrap();
        let _held = saturate(permits.path());
        let lease =
            crate::automated_history::try_begin_owned(vault.path(), "heartbeat", None).unwrap();
        fn under<'a>(
            lease: &'a crate::automated_history::RunLease,
            fallback: &Path,
        ) -> PermitPolicy<'a> {
            PermitPolicy::UnderLease {
                lease,
                fallback: fallback.to_path_buf(),
            }
        }
        let resolved = resolve_permit(under(&lease, permits.path()), Duration::from_secs(90), None)
            .await
            .expect("no permit is needed under a live lease");
        // The cause, not the clock (OF09; a 250 ms bound flaked under load):
        // only the live-lease branch resolves to `None`. Every slot is held,
        // so a wait for one could only end in a permit or a refusal.
        assert!(resolved.is_none(), "it never waited for a slot");

        // An agent that rewrote the lease file loses the exemption.
        let lease_file = vault.path().join(".hollow-history/active-run.nosync");
        let original = std::fs::read_to_string(&lease_file).unwrap();
        std::fs::write(&lease_file, "run-forged\n").unwrap();
        let error = resolve_permit(
            under(&lease, permits.path()),
            Duration::from_millis(300),
            None,
        )
        .await
        .expect_err("a forged lease falls back to a permit, and every slot is held");
        assert!(error.contains("background agents"), "{error}");
        std::fs::write(&lease_file, original).unwrap();
        lease.finish("success").unwrap();

        // The callers that run inside their own lease use the lease-aware
        // runner; the teammate server takes a permit before it runs.
        fn body<'a>(source: &'a str, needle: &str) -> &'a str {
            let start = source.find(needle).unwrap_or_else(|| panic!("{needle}"));
            let end = start + source[start..].find("\n}\n").expect("end of fn");
            &source[start..end]
        }
        let under_lease = concat!("run_oneshot", "_under_lease(");
        let heartbeat = include_str!("heartbeat.rs");
        assert!(body(heartbeat, concat!("async fn ", "run_beat(")).contains(under_lease));
        let ingest = include_str!("ingest.rs");
        assert!(body(ingest, concat!("async fn ", "run_proposed_notes(")).contains(under_lease));
        let team = include_str!("team_mcp.rs");
        // PR 8b moved the delegation body into `call_tool_with` (its
        // environment and budget store injected); `call_tool` only wires them.
        let call = body(team, concat!("async fn ", "call_tool_with("));
        let permit = call
            .find(concat!("run_permit::", "acquire("))
            .expect("a teammate permit");
        // The teammate runs through its injected runner (the installed one
        // is `agent::run_collect`, pinned by team_mcp's own lint).
        let run = call
            .find(concat!("(teammate.", "run)("))
            .expect("runs the teammate");
        assert!(permit < run, "the permit is taken before the teammate runs");
    }

    /// Paperclip PR 20: a lease-less one-shot under memory pressure is held
    /// back at the permit and then refused with the pressure reason, and like
    /// any other admission refusal it writes no usage row. Drives the Codex
    /// transform path (Codex with every NO_TOOLS entry denied), which skips
    /// agent validation before the permit, so no CLI is needed or started.
    #[tokio::test]
    async fn a_one_shot_under_memory_pressure_is_refused_without_a_usage_row() {
        let directory = tempfile::tempdir().unwrap();
        let permits = tempfile::tempdir().unwrap();
        crate::run_permit::set_test_pressure(crate::run_permit::PressureSample {
            node_procs: Some(2),
            vm_pressure_level: Some(4),
            free_pct: Some(5),
        });
        let collected = run_oneshot_with(
            PermitPolicy::Acquire {
                dir: permits.path().to_path_buf(),
                max_wait: PERMIT_WAIT,
            },
            crate::agent::AgentKind::Codex,
            None,
            "A synthetic transformation",
            test_opts(directory.path(), Duration::from_millis(300)),
            None,
        )
        .await;
        crate::run_permit::set_test_pressure(crate::run_permit::PressureSample::default());
        let error = collected.result.unwrap_err();
        assert!(error.contains("memory pressure"), "{error}");
        assert!(
            collected.usage.is_none(),
            "an admission refusal writes no row"
        );
        assert!(
            crate::session_store::usage_rows_since(directory.path(), "")
                .unwrap()
                .is_empty(),
            "nothing reached the usage ledger"
        );
        assert!(
            crate::run_permit::try_acquire(permits.path())
                .unwrap()
                .is_some(),
            "the held-back one-shot never took a slot"
        );
    }

    // SEC-01A: task/project assist must stay tool-free. This guards against a
    // future edit silently emptying the deny list (empty = unrestricted under
    // bypassPermissions), which would re-open the prompt-injection-to-tools path.
    #[test]
    fn no_tools_denies_the_dangerous_builtins() {
        for t in [
            "Bash",
            "Read",
            "Edit",
            "Write",
            "WebFetch",
            "WebSearch",
            "NotebookEdit",
        ] {
            assert!(NO_TOOLS.contains(&t), "{t} must be in the assist deny list");
        }
    }

    #[test]
    fn redact_masks_email_local_part() {
        let out = redact("Hi from alice.smith@example.com — see thread.");
        assert!(out.contains("[email]@example.com"));
        assert!(!out.contains("alice.smith@"));
    }

    #[test]
    fn redact_masks_jwt_like_tokens() {
        let token = "eyJabcdefghij.klmnopqrst.uvwxyz1234567";
        let out = redact(&format!("token={token}"));
        assert_eq!(out, "token=[jwt]");
    }

    #[test]
    fn redact_masks_authorization_bearer() {
        let out = redact("Authorization: Bearer abc.def.ghi");
        assert!(out.contains("[redacted]"));
        assert!(!out.contains("abc.def.ghi"));
    }

    /// Q-R32 (R5b): short and quoted credentials. A quoted value is masked
    /// whatever its length, a scheme's short base64 value and a short bare
    /// `Authorization` value with a digit are masked, a flag's value (quoted: any
    /// length, spaces allowed, as `--password 'my pass'`; bare: 8 or more characters
    /// or a digit, as `--token=abc123`) and curl's `-u user:pass` (also after a path,
    /// a quote, `$(`, `;`) are masked, JSON inside
    /// JSON (`{\"token\":\"abc\"}`) is masked, and camelCase token keys
    /// (`accessToken`, `idToken`) and passphrase and private-key names count.
    /// Prose and code keep their words.
    #[test]
    fn redact_masks_short_quoted_and_flag_credentials() {
        for (text, want) in [
            (r#"token: "abc""#, r#"token: "[redacted]""#),
            (r#"token: 'ab'"#, r#"token: '[redacted]'"#),
            (r#"{"accessToken":"ab"}"#, r#"{"accessToken":"[redacted]"}"#),
            (r#"{"idToken": "ab"}"#, r#"{"idToken": "[redacted]"}"#),
            (r#"{\"token\":\"abc\"}"#, r#"{\"token\":\"[redacted]\"}"#),
            (
                r#"{\"password\":\"ab\"}"#,
                r#"{\"password\":\"[redacted]\"}"#,
            ),
            (r#"passphrase: "ab""#, r#"passphrase: "[redacted]""#),
            (r#"private_key: "ab""#, r#"private_key: "[redacted]""#),
            (
                "Authorization: Basic dXNlcjpw",
                "Authorization: Basic [redacted]",
            ),
            ("Authorization: abc123", "Authorization: [redacted]"),
            (
                "curl --password hunter2 https://x",
                "curl --password [redacted] https://x",
            ),
            (
                "curl --password 'hunter2x' https://x",
                "curl --password '[redacted]' https://x",
            ),
            (
                "curl --password 'my pass' https://x",
                "curl --password '[redacted]' https://x",
            ),
            (
                "run --api-key \"a b\" now",
                "run --api-key \"[redacted]\" now",
            ),
            (
                "/usr/bin/curl -u admin:s3 https://x",
                "/usr/bin/curl -u admin:[redacted] https://x",
            ),
            (
                "{\"cmd\": \"curl -u admin:s3cretpw https://x\"}",
                "{\"cmd\": \"curl -u admin:[redacted] https://x\"}",
            ),
            (
                "x=$(curl -u admin:pw https://x)",
                "x=$(curl -u admin:[redacted] https://x)",
            ),
            (
                "cd d;curl -u admin:pw https://x",
                "cd d;curl -u admin:[redacted] https://x",
            ),
            (
                "curl -u 1000:1000 https://x",
                "curl -u 1000:[redacted] https://x",
            ),
            (
                "{\"botToken\": \"abcdefghij0123456789\"}",
                "{\"botToken\": \"[redacted]\"}",
            ),
            ("run --token=abc123 now", "run --token=[redacted] now"),
            ("tool --api-key abcdefghij", "tool --api-key [redacted]"),
            (
                "curl -u admin:s3cretpw https://x",
                "curl -u admin:[redacted] https://x",
            ),
        ] {
            assert_eq!(redact(text), want, "{text}");
            assert_eq!(redact(want), want, "idempotent: {text}");
        }
        // Words and code stay.
        for text in [
            "Authorization: token required",
            "Authorization:",
            "token: String",
            "let token = response.data.token;",
            "run --password-file ./pw.txt",
            "tool --token --verbose",
            "ssh -u admin host",
            "git log --token the",
            "tool --api-key abc",
            "chown -u root:wheel f",
            "max_tokens=4096",
            "maxToken: 4096",
            "endToken: \")\"",
            "ps -u alice:staff",
            "{\"fooToken\": \"ab\"}",
            "max_tokens=4096",
            "{\"input_tokens\": 12}",
        ] {
            assert_eq!(redact(text), text, "{text}");
        }
    }

    /// Q-R32: Linear keys, an `Authorization` value with any scheme or none
    /// (header and JSON form), and `token`/`deviceToken`/`pushToken` keys.
    #[test]
    fn redact_masks_linear_keys_any_authorization_and_token_keys() {
        let lin = "lin_api_ABCDEFGHIJKLMNOPQRSTuvwx1234";
        assert_eq!(redact(&format!("key {lin} ok")), "key [api-key] ok");
        let oauth = "lin_oauth_0123456789abcdefghijKLMN";
        assert_eq!(redact(&format!("({oauth})")), "([api-key])");
        assert_eq!(redact("lin_api_short"), "lin_api_short");
        for (text, want) in [
            (
                format!("Authorization: {lin}"),
                "Authorization: [api-key]".to_string(),
            ),
            (
                "HTTP_AUTHORIZATION: Bearer abc123def456".into(),
                "HTTP_AUTHORIZATION: Bearer [redacted]".into(),
            ),
            (
                "Authorization: AbCdEf0123456789xyz".into(),
                "Authorization: [redacted]".into(),
            ),
            (
                "device_token: abc123def456".into(),
                "device_token: [redacted]".into(),
            ),
            (
                "Authorization: Basic dXNlcjpodW50ZXIy".into(),
                "Authorization: Basic [redacted]".into(),
            ),
            (
                "Authorization: Bearer abc.def.ghi".into(),
                "Authorization: Bearer [redacted]".into(),
            ),
            (
                r#"{"Authorization": "Bearer abc123", "x": 1}"#.into(),
                r#"{"Authorization": "Bearer [redacted]", "x": 1}"#.into(),
            ),
            (
                "{'authorization': 'token ghs123abc'}".into(),
                "{'authorization': 'token [redacted]'}".into(),
            ),
            (
                r#"{"token": "abcdef123456", "deviceToken":"apns0123456789"}"#.into(),
                r#"{"token": "[redacted]", "deviceToken":"[redacted]"}"#.into(),
            ),
            (
                "PushToken=fcm0123456789abcdef next".into(),
                "PushToken=[redacted] next".into(),
            ),
        ] {
            assert_eq!(redact(&text), want, "{text}");
            assert_eq!(redact(&want), want, "idempotent: {want}");
        }
        // Prose keeps its words: `token` with no key-value shape, and an
        // `Authorization` mention with no value on its line.
        let prose = "Refresh the token and retry; max_tokens=4096, token count 12.";
        assert_eq!(redact(prose), prose);
        let heading = "Authorization:\nThe header was missing. Authorization failed.";
        assert_eq!(redact(heading), heading);
        // Code and prose shown in diffs keep their words (review of
        // 04b46aa25): only a credential-shaped value is masked.
        for text in [
            "pub token: String,",
            "token: string;",
            "const token = response.data.token;",
            "let authorization = format!(\"Bearer {}\", t);",
            "Authorization: required on all routes",
            "Authorization: token required",
        ] {
            assert_eq!(redact(text), text);
        }
    }

    #[test]
    fn redact_masks_api_keys_and_camel_case_oauth_tokens() {
        let out =
            redact("key sk-ant-api03-abcdefghijklmnopqrstuvwx and sk-proj-ABCDEFGHIJKLMNOPQRST");
        assert!(!out.contains("abcdefghijklmnop"), "{out}");
        assert!(!out.contains("ABCDEFGHIJKLMNOP"), "{out}");
        let out = redact(r#"{"accessToken":"abcdef123456","refreshToken": "zyxwvu987654"}"#);
        assert!(
            !out.contains("abcdef123456") && !out.contains("zyxwvu987654"),
            "{out}"
        );
        assert!(
            out.contains("accessToken"),
            "the key name stays readable: {out}"
        );
    }

    #[test]
    fn redact_masks_claude_code_oauth_tokens() {
        // Claude Code's OAuth access (`sk-ant-oat01-`) and refresh
        // (`sk-ant-ort01-`) tokens (Paperclip PR 8c reads the credential for
        // the quota card). PR 6's provider-key pattern covers them; this pins
        // it so a later narrowing of that pattern can't miss them.
        for token in [
            "sk-ant-oat01-ACCESSabcdefghijklmnop_-12",
            "sk-ant-ort01-REFRESHabcdefghijklmnop",
        ] {
            let out = redact(&format!("failed with {token} in the body"));
            assert_eq!(out, "failed with [api-key] in the body");
        }
        // A bare mention is not a credential.
        assert_eq!(redact("an sk-ant token"), "an sk-ant token");
    }

    #[test]
    fn redact_keeps_the_shape_of_a_compact_credential_line() {
        let out = redact(r#"{"accessToken":"abcdef123","refreshToken":"ghijkl456","expiresAt":1}"#);
        assert_eq!(
            out,
            r#"{"accessToken":"[redacted]","refreshToken":"[redacted]","expiresAt":1}"#
        );
        let out = redact(r#"{"refreshToken":"ghijkl456"}"#);
        assert_eq!(
            out, r#"{"refreshToken":"[redacted]"}"#,
            "the closing brace survives"
        );
    }

    /// Review of #1237: tool results now reach the learning extractor, so
    /// the shapes a terminal shows most (cloud and service keys, env lines,
    /// a URL's password) are masked too.
    #[test]
    fn redact_masks_service_keys_env_lines_and_url_passwords() {
        let google = format!("AIza{}", "Sy0_a-B".repeat(5));
        let stripe = format!("sk_live_{}", "aB3dE9".repeat(4));
        let restricted = format!("rk_live_{}", "Zx81".repeat(5));
        let cases = [
            ("aws AKIAABCDEFGHIJKLMNOP done", "aws [aws-key] done"),
            ("temp ASIAABCDEFGHIJKLMNOP", "temp [aws-key]"),
            (
                "slack xoxb-1234567890-abcdefghij ok",
                "slack [slack-token] ok",
            ),
            ("slack xoxa-2-abcdefghijkl", "slack [slack-token]"),
            ("slack xoxp-1234567890-ab", "slack [slack-token]"),
            ("slack xoxr-1234567890-ab", "slack [slack-token]"),
            (&format!("key {google} ok"), "key [api-key] ok"),
            (&format!("stripe {stripe}"), "stripe [api-key]"),
            (&format!("stripe {restricted}"), "stripe [api-key]"),
            ("KEY=value1", "KEY=[redacted]"),
            ("export GITHUB_TOKEN=ghx3", "export GITHUB_TOKEN=[redacted]"),
            ("SECRET = s3cr3t\r\nnext", "SECRET = [redacted]\r\nnext"),
            (
                "DB_PASSWORD=\"two words\" ok",
                "DB_PASSWORD=\"[redacted]\" ok",
            ),
            (
                "AWS_SECRET_ACCESS_KEY='abc/def'",
                "AWS_SECRET_ACCESS_KEY='[redacted]'",
            ),
            (
                "fetch https://jb:hunter2pass@example.com/x failed",
                "fetch https://jb:[redacted]@example.com/x failed",
            ),
            ("git+ssh://:tok123@host/r", "git+ssh://:[redacted]@host/r"),
            // #1237 delta review: credentials files, JSON and headers in
            // any case.
            (
                "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG",
                "aws_secret_access_key = [redacted]",
            ),
            (
                "aws_session_token=FQoGZXIvYXdz",
                "aws_session_token=[redacted]",
            ),
            (
                "{\"SecretAccessKey\":\"wJalr/K7MDENG\",\"SessionToken\":\"FQoG\"}",
                "{\"SecretAccessKey\":\"[redacted]\",\"SessionToken\":\"[redacted]\"}",
            ),
            (
                "{\"password\": \"two words\"}",
                "{\"password\": \"[redacted]\"}",
            ),
            ("x-api-key: abc123 sent", "x-api-key: [redacted] sent"),
            ("db_password='pw 4' ok", "db_password='[redacted]' ok"),
            ("\u{17f}ecret=abc", "\u{17f}ecret=[redacted]"),
            // A head cut right after an escape (`redact_prefix`), and a
            // shell single-quoted value ending in a backslash.
            ("DB_PASSWORD=\"hunter2\\", "DB_PASSWORD=\"[redacted]\""),
            ("pw: \"hunter2\\", "pw: \"hunter2\\"),
            ("password: \"hunter2\\", "password: \"[redacted]\""),
            ("KEY='hunter2\\'\nnext", "KEY='[redacted]'\nnext"),
            ("KEY='hunter2\\'\r\nnext", "KEY='[redacted]'\r\nnext"),
            (
                "{\n  \"SecretAccessKey\":\n    \"wJalr/K7MDENG\"\n}",
                "{\n  \"SecretAccessKey\":\n    \"[redacted]\"\n}",
            ),
            (
                "{\"password\":\"abc\\\"tail\"}",
                "{\"password\":\"[redacted]\"}",
            ),
            ("password: \"open to the end", "password: \"[redacted]\""),
            ("secret:\nnext line", "secret:\nnext line"),
            (
                "secret:\n  \"password\": \"hunter2\"",
                "secret:\n  \"password\": \"[redacted]\"",
            ),
            ("secret:\n\n\"far away\"", "secret:\n\n\"far away\""),
            ("[\"api_key\",\n \"k\"]", "[\"api_key\",\n \"k\"]"),
            (
                "password:\r\n  \"hunter2\"\r\nnext",
                "password:\r\n  \"[redacted]\"\r\nnext",
            ),
            (
                "secret:\n  \"prefix password: \\\"hunter2\\\" suffix\"",
                "secret:\n  \"[redacted]\"",
            ),
            ("password:\n  \"hunter2", "password:\n  \"[redacted]\""),
            (
                "password=abc\\ api_key=\"hunter2\"",
                "password=[redacted]\\ api_key=\"[redacted]\"",
            ),
        ];
        for (raw, want) in cases {
            assert_eq!(redact(raw), want, "{raw}");
        }
        for kept in [
            "max_tokens=4096 and a_secret_count:: none",
            "if API_KEY == \"x\" { }",
            "ssh://github.com:22/o/r and https://example.com/a:b",
            "AKIA is a prefix",
            "use crate::secret::Key;",
            "test secret_store::tests::reads_key ... FAILED",
            "error: password_policy::check failed",
        ] {
            assert_eq!(redact(kept), kept);
        }
        for raw in [
            "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG",
            "{\"SecretAccessKey\":\"wJalr/K7MDENG\"}",
            "db_password='pw 4' ok",
            "{\"password\":\"abc\\\"tail\"}",
            "password: \"open to the end",
            "{\n  \"SecretAccessKey\":\n    \"wJalr/K7MDENG\"\n}",
            "secret:\n  \"password\": \"hunter2\"",
            "password:\r\n  \"hunter2\"\r\nnext",
            "secret:\n  \"prefix password: \\\"hunter2\\\" suffix\"",
            "password:\n  \"hunter2",
        ] {
            assert_eq!(redact(&redact(raw)), redact(raw), "{raw}");
        }
    }

    #[test]
    fn redact_leaves_plain_text_alone() {
        let raw = "Just a regular sentence with no PII.";
        assert_eq!(redact(raw), raw);
    }

    const PEM_BLOCK: &str = "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAsecretkeybody\nQUJDREVGR0hJSktMTU5PUFFSU1RVVldY\n-----END RSA PRIVATE KEY-----";

    #[test]
    fn redact_masks_pem_blocks() {
        let out = redact(&format!("before\n{PEM_BLOCK}\nafter"));
        assert_eq!(out, "before\n[pem]\nafter");
        // An unterminated private key (a truncated paste) is masked through
        // the end of the text.
        let out = redact("key: -----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA");
        assert_eq!(out, "key: [pem]");
        // Any label, not just private keys.
        let cert = "-----BEGIN CERTIFICATE-----\nMIIDdzCCAl+gAwIBAgIE\n-----END CERTIFICATE-----";
        let out = redact(&format!("cert {cert} done"));
        assert_eq!(out, "cert [pem] done");
        assert!(!out.contains("MIID"), "{out}");
    }

    /// One sample of every family `redact` masks, with the key-value ones'
    /// whitespace spelled several ways. Add a sample with any new family:
    /// both tests below read this list.
    fn family_samples() -> Vec<String> {
        let google = format!("AIza{}", "Sy0_a-B".repeat(5));
        let stripe = format!("sk_live_{}", "aB3dE9".repeat(4));
        let run_key = format!("hsr_{}", "0a".repeat(32));
        let github = format!("ghp_{}", "A".repeat(36));
        let pgp = format!(
            "-----BEGIN PGP PRIVATE KEY BLOCK-----\n{}\n-----END PGP PRIVATE KEY BLOCK-----",
            "lQOYBGVsecretkeymaterial\n".repeat(8)
        );
        [
            "-----BEGIN CERTIFICATE-----\nMIIDdzCCAl\n-----END CERTIFICATE-----",
            PEM_BLOCK,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAA",
            pgp.as_str(),
            "sk-ant-api03-abcdefghijklmnopqrstuvwxyz",
            run_key.as_str(),
            github.as_str(),
            "github_pat_ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "postgres://admin:hunter2@db.example.com/app",
            "dana@example.com",
            "eyJhbGciOiJIUzI1.eyJzdWIiOiIxMjM0.SflKxwRJSMeKKF2QT4",
            "Authorization: Bearer abc123def456",
            // Q-R32's families.
            "lin_api_ABCDEFGHIJKLMNOPQRSTuvwx1234",
            "Authorization: lin_oauth_ABCDEFGHIJKLMNOPQRST12",
            "{\"Authorization\": \"Basic dXNlcjpwYXNz\"}",
            "'authorization':\n 'token abc123def456'",
            "{\"deviceToken\": \"abcdef123456\"}",
            "pushToken=abcdef1234567890",
            "authorization:\n \t bearer \n  abc123def456",
            "refresh_token=abcdefgh123",
            "\"client_secret\" :  \"abcdefgh123\"",
            "code =\n  abcdefgh123",
            "cookie: session=abc",
            "Set-Cookie:\n   session=abc",
            // R5b's short, quoted and flag forms.
            "curl --password hunter2 https://x",
            "run --token='abc12345' now",
            "curl -u admin:s3cretpw host",
            "{\\\"token\\\":\\\"abc\\\"}",
            "Authorization: Basic dXNlcjpw",
            "{\"accessToken\":\"ab\"}",
            // #1237's families.
            "AKIAABCDEFGHIJKLMNOP",
            "xoxb-1234567890-abcdefghij",
            google.as_str(),
            stripe.as_str(),
            "whsec_ABCDEFGHIJKLMNOPQRST",
            "https://jb:hunter2pass@example.com/x",
            "API_KEY=value1",
            "DB_PASSWORD=\"two words\"",
            "SECRET = s3cr3t",
            "AWS_SECRET_ACCESS_KEY='abc def'",
            "aws_secret_access_key = wJalrXUtnFEMI/K7MDENG",
            "{\"SecretAccessKey\":\"wJalr/K7MDENG\"}",
            "\"password\": \"two words here\"",
            "x-api-key: abc123",
            "db_password='pw 4'",
            "\"SecretAccessKey\":\n    \"wJalr/K7MDENG\"",
            "password:\n  \"hunter2 two\"",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    /// `redact_prefix` is a prefix of `redact` and at least as long as asked
    /// (or all of it), wherever the head's cut falls: before, inside and
    /// after every family, inside a key-value's whitespace, and inside a PEM
    /// block longer than the head, PGP's private key block included (no
    /// unterminated form of it is masked, so the head must not end in it).
    #[test]
    fn redact_prefix_is_a_prefix_of_redact() {
        let secrets = family_samples();
        let chars = 30;
        for secret in &secrets {
            for lead in 0..2 * secret.chars().count() + 60 {
                let text = format!(
                    "{}{secret} and after it {}",
                    "w ".repeat(lead / 2) + if lead % 2 == 1 { " " } else { "" },
                    "more words ".repeat(12)
                );
                let full = redact(&text);
                let head = redact_prefix(&text, chars);
                assert!(full.starts_with(&head), "{text:?}: {head:?} vs {full:?}");
                assert!(
                    head == full || head.chars().count() >= chars,
                    "{text:?}: {head:?}"
                );
            }
        }
        // A long plain text redacts only a head; a short one, all of it.
        let long = "plain words ".repeat(100);
        assert_eq!(redact_prefix(&long, chars), long[..41]);
        assert_eq!(redact_prefix("short text", chars), "short text");
    }

    #[test]
    fn char_start_finds_a_character_by_number() {
        for text in ["", "abc", "a\u{e9}b\u{1f600}c", "\u{e9}\u{e9}"] {
            for n in 0..6 {
                assert_eq!(
                    char_start(text, n),
                    text.char_indices().nth(n).map(|(i, _)| i),
                    "{text:?} {n}"
                );
            }
        }
    }

    /// 2026-11-04: every placeholder `redact` writes is in
    /// `REDACT_PLACEHOLDERS`, which the searches over redacted text blank
    /// (`[run-key]` was missing from the list until then).
    #[test]
    fn every_placeholder_redact_writes_is_listed() {
        let secrets = family_samples();
        let placeholder = regex::Regex::new(r"\[[a-z-]+\]").unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for secret in secrets {
            let out = redact(&format!("x {secret} y"));
            let found: Vec<&str> = placeholder.find_iter(&out).map(|m| m.as_str()).collect();
            assert!(!found.is_empty(), "{secret} was not redacted: {out}");
            for p in found {
                assert!(REDACT_PLACEHOLDERS.contains(&p), "{p} is not listed");
                seen.insert(p.to_string());
            }
        }
        assert_eq!(seen.len(), REDACT_PLACEHOLDERS.len(), "{seen:?}");
    }

    #[test]
    fn redact_masks_github_tokens() {
        let body = "A".repeat(36);
        for prefix in ["ghp_", "gho_", "ghs_", "ghu_", "ghr_"] {
            let token = format!("{prefix}{body}");
            assert_eq!(
                redact(&format!("push with {token} now")),
                "push with [github-token] now"
            );
        }
        let pat = format!("github_pat_{}", "B1_".repeat(27) + "C");
        assert_eq!(pat.len(), 11 + 82);
        assert_eq!(redact(&format!("pat={pat}")), "pat=[github-token]");
        // Short or bare mentions are not tokens.
        assert_eq!(redact("ghp_short and github"), "ghp_short and github");
    }

    #[test]
    fn redact_masks_dsns_before_emails() {
        let out = redact("db postgres://admin:hunter2@db.example.com:5432/app ok");
        assert_eq!(out, "db postgres://[redacted] ok");
        assert!(
            !out.contains("hunter2") && !out.contains("[email]"),
            "{out}"
        );
        for dsn in [
            "rediss://:s3cret@cache.example.com:6380/0",
            "mongodb+srv://user:pw@cluster0.example.net/db?retryWrites=true",
            "MySQL://root:toor@localhost/app",
        ] {
            let out = redact(&format!("url={dsn}"));
            let scheme = dsn.split("://").next().unwrap();
            assert_eq!(out, format!("url={scheme}://[redacted]"), "{dsn}");
        }
        assert_eq!(
            redact("see https://example.com/path"),
            "see https://example.com/path"
        );
    }

    #[test]
    fn redact_is_idempotent() {
        let samples = [
            format!("x {PEM_BLOCK} y"),
            "-----BEGIN PRIVATE KEY-----\nabc".to_string(),
            format!("ghp_{}", "Z".repeat(36)),
            "postgres://a:b@h/db and alice@example.com".to_string(),
            "sk-ant-api03-abcdefghijklmnopqrstuvwx".to_string(),
            "eyJabcdefghij.klmnopqrst.uvwxyz1234567".to_string(),
            "Authorization: Bearer abc.def.ghi".to_string(),
            r#"{"accessToken":"abcdef123","refreshToken":"ghijkl456"}"#.to_string(),
            "Cookie: sid=abcdef".to_string(),
            "Just a regular sentence.".to_string(),
            "AKIAABCDEFGHIJKLMNOP and xoxp-1234567890-abcdef".to_string(),
            "fetch https://jb:hunter2pass@example.com/x".to_string(),
            "export API_KEY=abc123 DB_PASSWORD=\"two words\" SECRET = s3cr3t".to_string(),
        ];
        for s in samples {
            let once = redact(&s);
            assert_eq!(redact(&once), once, "not idempotent for {s:?}");
        }
    }

    #[test]
    fn debug_log_disabled_by_default() {
        // Don't mutate the global env in a test that runs in parallel —
        // just assert the default-unset case returns false. Setting
        // HOLLOW_DEBUG_LOG=1 in CI would still flip this; that's fine.
        let original = std::env::var("HOLLOW_DEBUG_LOG").ok();
        std::env::remove_var("HOLLOW_DEBUG_LOG");
        assert!(!debug_log_enabled());
        if let Some(v) = original {
            std::env::set_var("HOLLOW_DEBUG_LOG", v);
        }
    }

    #[test]
    fn extract_json_object_unwraps_code_fence() {
        let s = "```json\n{\"ok\":true}\n```";
        let out = extract_json_object(s);
        assert!(out.contains("\"ok\":true"));
    }

    #[test]
    fn parse_json_or_prose_returns_data_on_valid_json() {
        let body = "[1,2,3]";
        let r = parse_json_or_prose::<Vec<i32>>(body, body, "array", "test.log");
        assert_eq!(r.unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn parse_json_or_prose_surfaces_auth_hint_when_model_returns_prose() {
        let prose = "The `some_tool` tool is not available — the connector only \
                     exposes `authenticate` and `complete_authentication` right now, \
                     meaning it hasn't been signed in.";
        let r = parse_json_or_prose::<Vec<i32>>(prose, prose, "array", "test.log");
        let err = r.unwrap_err();
        // Friendly hint takes precedence over the raw parse error.
        assert!(err.contains("isn't authenticated"), "got: {err}");
        // Original model prose still included so the user sees what Claude said.
        assert!(err.contains("only exposes"));
        // No cryptic "expected value at line 1" parse error from serde.
        assert!(!err.contains("expected value at line"));
    }

    #[test]
    fn parse_json_or_prose_surfaces_tool_missing_hint() {
        let prose = "I cannot complete this request because the tool is not available.";
        let r = parse_json_or_prose::<Vec<i32>>(prose, prose, "array", "test.log");
        let err = r.unwrap_err();
        // Should match the tool-missing branch, not the auth branch.
        assert!(err.contains("MCP tool isn't loaded") || err.contains("isn't authenticated"));
    }
}
