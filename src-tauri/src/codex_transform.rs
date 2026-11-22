//! Pure text transformations through Codex, with an actually empty model tool
//! list. This is separate from the interactive adapter: read-only sandboxing
//! alone still permits reads and cannot replace a tool boundary.
//!
//! Codex has no public --tools='' switch. Its bundled model catalog plus
//! supported feature overrides can remove tools. A local Responses fixture
//! verified no advertised tools (the tools field is omitted) on each version
//! in `codex_probe::VERIFIED` (0.146.0, 0.153.4 since 2026-10-07, 0.162.1 since 2026-10-14); the
//! guard asks the installed CLI its version once per binary
//! (`codex_probe`) and fails closed, saying which version and what to do,
//! until a new CLI is verified.
use crate::agent::{self, AgentEvent, AgentKind, ParseState};
use crate::codex_probe::{self, CodexHelperStatus, ReadEnd};
use crate::process_group;
use crate::usage::LedgerOutcome;
use crate::usage_ledger::{Meter, UsageRun};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
#[cfg(test)]
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const DISABLED_FEATURES: &[&str] = &[
    "apps",
    "plugins",
    "shell_tool",
    "unified_exec",
    "code_mode",
    "code_mode_host",
    "code_mode_only",
    "browser_use",
    "browser_use_external",
    "browser_use_full_cdp_access",
    "computer_use",
    "goals",
    "multi_agent",
    "multi_agent_v2",
    "image_generation",
    "in_app_browser",
    "memories",
    "skill_search",
    "tool_suggest",
    "workspace_dependencies",
    "current_time_reminder",
    "token_budget",
    "enable_request_compression",
    "hooks",
];

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!(
            "hollow-codex-transform-{:032x}",
            rand::random::<u128>()
        ));
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&path)
            .map_err(|e| format!("Create isolated agent folder: {e}"))?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Keep every supported model's metadata while removing all tool exposure.
/// Unknown user-specified model IDs receive the runtime's own fallback metadata
/// with the exact requested ID; model access is still checked by the provider.
fn transform_catalog(raw: &[u8], requested_model: Option<&str>) -> Result<Value, String> {
    let mut catalog: Value =
        serde_json::from_slice(raw).map_err(|e| format!("Read Codex model catalog: {e}"))?;
    let models = catalog
        .get_mut("models")
        .and_then(Value::as_array_mut)
        .filter(|models| !models.is_empty())
        .ok_or("Codex returned an empty model catalog.")?;
    if let Some(model) = requested_model.filter(|model| !model.trim().is_empty()) {
        if !models
            .iter()
            .any(|entry| entry.get("slug").and_then(Value::as_str) == Some(model))
        {
            let mut fallback = models[0].clone();
            let object = fallback
                .as_object_mut()
                .ok_or("Invalid Codex model catalog entry.")?;
            object.insert("slug".into(), json!(model));
            object.insert("display_name".into(), json!(model));
            models.push(fallback);
        }
    }
    for model in models {
        let object = model
            .as_object_mut()
            .ok_or("Invalid Codex model catalog entry.")?;
        object.insert("shell_type".into(), json!("disabled"));
        object.insert("apply_patch_tool_type".into(), Value::Null);
        object.insert("experimental_supported_tools".into(), json!([]));
        // In the verified runtime, text-only modality also removes view_image.
        object.insert("input_modalities".into(), json!(["text"]));
        object.insert("tool_mode".into(), json!("function"));
    }
    Ok(catalog)
}

fn transformation_command(
    binary: &Path,
    catalog: &Path,
    model: Option<&str>,
    cwd: &Path,
) -> Command {
    let mut command = Command::new(binary);
    command.env("PATH", agent::cli_path());
    command.args([
        "exec",
        "--ignore-user-config",
        "--ignore-rules",
        "--ephemeral",
        "--skip-git-repo-check",
        "--json",
        "--color",
        "never",
        "--sandbox",
        "read-only",
    ]);
    command.arg("-c").arg(format!(
        "model_catalog_json={}",
        serde_json::to_string(&catalog.to_string_lossy()).unwrap()
    ));
    command.args([
        "-c",
        "web_search=\"disabled\"",
        "-c",
        "tools.update_plan.enabled=false",
        "-c",
        "project_doc_max_bytes=0",
    ]);
    for feature in DISABLED_FEATURES {
        command.arg("--disable").arg(feature);
    }
    if let Some(model) = model.filter(|model| !model.trim().is_empty()) {
        command.arg("--model").arg(model);
    }
    // Stdin avoids exposing the transformation's source text in process args.
    command
        .arg("-")
        .current_dir(cwd)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Every stdout event reaches the usage ledger FIRST, and only then is the
/// exit status checked: Codex can finish a turn (`turn.completed` with its
/// usage) and still exit nonzero, and that usage must not be lost.
fn collect_output(output: std::process::Output, ledger: &mut UsageRun) -> Result<String, String> {
    let mut state = ParseState::default();
    let mut events = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        for event in agent::parse_line(AgentKind::Codex, line, &mut state) {
            ledger.observe(&event);
            events.push(event);
        }
    }
    if !output.status.success() {
        return Err(format!(
            "Codex transformation failed: {}",
            crate::claude_runner::truncate(
                &crate::claude_runner::redact(String::from_utf8_lossy(&output.stderr).trim()),
                500
            )
        ));
    }
    let mut text = String::new();
    for event in events {
        match event {
            AgentEvent::AssistantText(value) => text = value,
            AgentEvent::AssistantDelta(value) => text.push_str(&value),
            AgentEvent::ToolUse { .. } | AgentEvent::ToolResult { .. } => {
                return Err("Codex exposed a tool during a restricted transformation. This runtime must be re-verified.".into());
            }
            AgentEvent::Result {
                is_error: true,
                text,
                subtype,
                ..
            } => {
                return Err(format!("Codex transformation: {}", text.unwrap_or(subtype)));
            }
            _ => {}
        }
    }
    if text.trim().is_empty() {
        return Err("Codex returned no transformation result.".into());
    }
    Ok(text)
}

/// Why a transformation ended early, besides an ordinary failure.
enum Interrupted {
    TimedOut,
    Stopped,
    Failed(String),
}

impl From<String> for Interrupted {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for Interrupted {
    fn from(error: &str) -> Self {
        Self::Failed(error.to_string())
    }
}

/// How a transformation's ending is recorded in the usage ledger. Only the
/// transformation step is a model turn: a failure before it (the version or
/// catalog step, or the version guard) never started the agent.
fn transform_outcome(ended: &Result<String, Interrupted>, spawned: bool) -> LedgerOutcome {
    match ended {
        Ok(_) => LedgerOutcome::Success,
        Err(Interrupted::TimedOut) => LedgerOutcome::Timeout,
        Err(Interrupted::Stopped) => LedgerOutcome::Cancelled,
        Err(Interrupted::Failed(_)) if spawned => LedgerOutcome::Error,
        Err(Interrupted::Failed(_)) => LedgerOutcome::SpawnFailed,
    }
}

/// Callers supply all context in `prompt`; the caller's folder is intentionally
/// not used so transforms cannot inherit a project's tools or instructions.
/// `timeout` is one deadline across all three steps, and `stop` cancels any of
/// them. Each step runs in its own process group and is settled before it
/// returns, so a timeout or stop ends everything the CLI started instead of
/// only dropping a future; the outcome is the weakest step's. Writes exactly
/// one usage row for `meter` (Codex is unpriced; billing is read from
/// Hollow's own environment, since `--ignore-user-config` drops user config).
pub async fn run(
    prompt: &str,
    model: Option<&str>,
    _cwd: &Path,
    timeout: Duration,
    stop: Option<&process_group::StopSignal>,
    meter: &Meter,
) -> agent::Collected {
    run_with(
        prompt,
        model,
        timeout,
        stop,
        meter,
        agent::installed_binary(AgentKind::Codex),
        &codex_probe::APP,
    )
    .await
}

/// [`run`] with the Codex at `binary` (`None`: not installed) and the
/// version answers in `versions` (the app's cache in production, a fresh
/// one and a fake CLI in tests).
async fn run_with(
    prompt: &str,
    model: Option<&str>,
    timeout: Duration,
    stop: Option<&process_group::StopSignal>,
    meter: &Meter,
    binary: Option<PathBuf>,
    versions: &codex_probe::Cache,
) -> agent::Collected {
    let mut ledger = UsageRun::begin(
        meter.clone(),
        Some(AgentKind::Codex),
        model.map(str::to_string),
    );
    let mut outcome = None;
    let mut spawned = false;
    let ended = transform(
        Cli { binary, versions },
        prompt,
        model,
        timeout,
        stop,
        &mut outcome,
        &mut ledger,
        &mut spawned,
    )
    .await;
    if spawned {
        ledger.mark_spawned();
        // Only a run that reached its model turn has a bill to classify
        // (`--ignore-user-config` drops user config, hence `false`).
        ledger.set_billing(crate::usage::classify_process_env(AgentKind::Codex, false));
    }
    let ledger_outcome = transform_outcome(&ended, spawned);
    let (result, timed_out, stopped) = match ended {
        Ok(text) => (Ok(text), false, false),
        Err(Interrupted::TimedOut) => (
            Err(format!(
                "Codex transformation timed out after {}s.",
                timeout.as_secs()
            )),
            true,
            false,
        ),
        Err(Interrupted::Stopped) => (
            Err("Codex transformation stopped.".to_string()),
            false,
            true,
        ),
        Err(Interrupted::Failed(error)) => (Err(error), false, false),
    };
    agent::Collected {
        result,
        // A tool-free transform's output is all-or-nothing.
        partial_text: String::new(),
        timed_out,
        stopped,
        outcome,
        usage: Some(ledger.finish(ledger_outcome)),
        // Codex is checked before it starts, never stopped mid-run (PR 8b).
        budget_stop: None,
        budget_warnings: Vec::new(),
        // The transform drives its own processes, unclocked (OB08a).
        activity: None,
    }
}

/// The Codex a transformation runs and where its version answers live.
struct Cli<'a> {
    binary: Option<PathBuf>,
    versions: &'a codex_probe::Cache,
}

#[allow(clippy::too_many_arguments)]
async fn transform(
    cli: Cli<'_>,
    prompt: &str,
    model: Option<&str>,
    timeout: Duration,
    stop: Option<&process_group::StopSignal>,
    outcome: &mut Option<process_group::StopOutcome>,
    ledger: &mut UsageRun,
    spawned: &mut bool,
) -> Result<String, Interrupted> {
    // A stop requested before the transform starts wins over every other
    // ending (even a missing CLI), so the run records one `cancelled` row and
    // callers need no stop check of their own before calling the runner.
    if stop.is_some_and(process_group::StopSignal::requested) {
        return Err(Interrupted::Stopped);
    }
    let scratch = Scratch::new()?;
    let deadline = Instant::now() + timeout;
    // The version guard (F9, wave 4 · V1): asked once per binary, and a
    // version the local fixture check hasn't proved fails closed with a
    // sentence naming it, the verified ones and the fallback, which the
    // usage row keeps as its note.
    let status = match cli.binary.as_deref() {
        None => CodexHelperStatus::missing(),
        Some(binary) => match codex_probe::read_version(
            cli.versions,
            binary,
            &scratch.0,
            deadline.saturating_duration_since(Instant::now()),
            stop,
            outcome,
        )
        .await
        {
            Ok((version, served)) => CodexHelperStatus::of_version(&version, served),
            Err(ReadEnd::Stopped) => return Err(Interrupted::Stopped),
            Err(ReadEnd::TimedOut) => return Err(Interrupted::TimedOut),
            Err(ReadEnd::Missing) => CodexHelperStatus::missing(),
            Err(ReadEnd::Failed(reason)) => CodexHelperStatus::unknown(&reason),
        },
    };
    if !status.verified() {
        ledger.set_note(&status.note());
        return Err(status.message.into());
    }
    let binary = cli.binary.unwrap_or_default();
    let mut catalog_command = Command::new(&binary);
    catalog_command
        .args(["debug", "models", "--bundled"])
        .current_dir(&scratch.0)
        .env("PATH", agent::cli_path());
    let catalog_output = step(
        &mut catalog_command,
        None,
        deadline,
        stop,
        outcome,
        "Read Codex model metadata",
        None,
    )
    .await?;
    if !catalog_output.status.is_some_and(|status| status.success()) {
        return Err("Codex could not provide its bundled model metadata.".into());
    }
    let catalog = transform_catalog(&catalog_output.stdout, model)?;
    let catalog_path = scratch.0.join("models.json");
    tokio::fs::write(
        &catalog_path,
        serde_json::to_vec(&catalog).map_err(|e| e.to_string())?,
    )
    .await
    .map_err(|e| format!("Write isolated model catalog: {e}"))?;
    let mut command = transformation_command(&binary, &catalog_path, model, &scratch.0);
    let input = format!("{}\n\n{}", agent::WORKSPACE_POLICY, prompt);
    let output = step(
        &mut command,
        Some(input.as_bytes()),
        deadline,
        stop,
        outcome,
        "Start Codex transformation",
        Some(spawned),
    )
    .await?;
    if let Some(error) = output.input_error {
        return Err(format!("Write Codex transformation input: {error}").into());
    }
    if let Some(error) = output.wait_error {
        return Err(format!("Wait for Codex transformation: {error}").into());
    }
    let status = output
        .status
        .ok_or("Wait for Codex transformation: exit status unavailable")?;
    Ok(collect_output(
        std::process::Output {
            status,
            stdout: output.stdout,
            stderr: output.stderr.into_bytes(),
        },
        ledger,
    )?)
}

/// Run one step inside the shared deadline, folding its stop outcome in.
/// `spawned` (the transformation step only) is set as soon as the process
/// ran, before the stop and timeout checks.
async fn step(
    cmd: &mut Command,
    input: Option<&[u8]>,
    deadline: Instant,
    stop: Option<&process_group::StopSignal>,
    outcome: &mut Option<process_group::StopOutcome>,
    what: &str,
    spawned: Option<&mut bool>,
) -> Result<process_group::GroupOutput, Interrupted> {
    if stop.is_some_and(process_group::StopSignal::requested) {
        return Err(Interrupted::Stopped);
    }
    let remaining = deadline.saturating_duration_since(Instant::now());
    let output = process_group::run_to_completion(cmd, input, remaining, stop)
        .await
        .map_err(|e| format!("{what}: {e}"))?;
    if let Some(flag) = spawned {
        *flag = true;
    }
    *outcome = process_group::StopOutcome::combine(*outcome, output.outcome);
    if output.stopped {
        return Err(Interrupted::Stopped);
    }
    if output.timed_out {
        return Err(Interrupted::TimedOut);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_removes_file_shell_patch_and_experimental_tools_for_all_models() {
        let raw = br#"{"models":[{"slug":"custom-a","shell_type":"shell_command","apply_patch_tool_type":"freeform","experimental_supported_tools":["clock"],"input_modalities":["text","image"],"tool_mode":"code_mode_only"}]}"#;
        let result = transform_catalog(raw, Some("another-account-model")).unwrap();
        let models = result["models"].as_array().unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[1]["slug"], "another-account-model");
        for model in models {
            assert_eq!(model["shell_type"], "disabled");
            assert!(model["apply_patch_tool_type"].is_null());
            assert_eq!(model["experimental_supported_tools"], json!([]));
            assert_eq!(model["input_modalities"], json!(["text"]));
            assert_eq!(model["tool_mode"], "function");
        }
    }
    #[test]
    fn catalog_rejects_missing_empty_and_malformed_metadata() {
        for value in [
            b"broken".as_slice(),
            br#"{}"#,
            br#"{"models":[]}"#,
            br#"{"models":[null]}"#,
        ] {
            assert!(transform_catalog(value, None).is_err());
        }
    }
    #[test]
    fn command_isolates_configuration_and_never_puts_source_text_in_arguments() {
        let command = transformation_command(
            Path::new("codex"),
            Path::new("/tmp/my models.json"),
            Some("custom-model"),
            Path::new("/tmp"),
        );
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for option in [
            "--ignore-user-config",
            "--ignore-rules",
            "--ephemeral",
            "read-only",
            "custom-model",
        ] {
            assert!(args.contains(&option.to_string()));
        }
        assert!(args.contains(&"model_catalog_json=\"/tmp/my models.json\"".into()));
        for feature in DISABLED_FEATURES {
            assert!(args.windows(2).any(|pair| pair == ["--disable", *feature]));
        }
        assert_eq!(args.last().unwrap(), "-");
    }
    /// Run explicitly when qualifying an installed CLI version. The only
    /// Responses request goes to a local fixture that rejects before inference;
    /// no prompt, account token or model request is sent to an external server.
    #[tokio::test]
    #[ignore = "requires installed Codex; local HTTP fixture only"]
    async fn installed_codex_advertises_zero_tools_to_local_responses_fixture() {
        use tokio::io::AsyncReadExt;
        let binary = agent::find_binary(AgentKind::Codex)
            .expect("Codex must be installed for qualification");
        let scratch = Scratch::new().unwrap();
        let catalog_output = Command::new(&binary)
            .args(["debug", "models", "--bundled"])
            .current_dir(&scratch.0)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(catalog_output.status.success());
        let source: Value = serde_json::from_slice(&catalog_output.stdout).unwrap();
        let model = source["models"][0]["slug"].as_str().unwrap();
        for requested in [Some(model), None, Some("hollow-qualification-custom-model")] {
            let catalog = transform_catalog(&catalog_output.stdout, requested).unwrap();
            let path = scratch.0.join("models.json");
            std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = tokio::spawn(async move {
                tokio::time::timeout(Duration::from_secs(20), async {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut raw = Vec::new();
                let mut chunk = [0u8; 4096];
                let (header_end, content_length) = loop {
                    let size = stream.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    raw.extend_from_slice(&chunk[..size]);
                    if let Some(index) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..index]);
                        let length = headers.lines().find_map(|line| line.split_once(':')
                            .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                            .map(|(_, value)| value.trim().parse::<usize>().unwrap())).unwrap();
                        break (index + 4, length);
                    }
                };
                while raw.len() < header_end + content_length {
                    let size = stream.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    raw.extend_from_slice(&chunk[..size]);
                }
                let request: Value = serde_json::from_slice(&raw[header_end..header_end + content_length]).unwrap();
                let body = r#"{"error":{"message":"Local fixture stopped before inference","type":"invalid_request_error"}}"#;
                stream.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).as_bytes()).await.unwrap();
                request.get("tools").cloned().unwrap_or_else(|| json!([]))
            }).await.expect("Local fixture timed out")
            });
            let mut command = transformation_command(&binary, &path, requested, &scratch.0);
            // No account at all: an empty Codex home (night ST12 review).
            let home = scratch.0.join("codex-home");
            std::fs::create_dir_all(&home).unwrap();
            command.env("CODEX_HOME", &home);
            command.arg("-c").arg("model_provider=\"hollow-fixture\"");
            command.arg("-c").arg(format!("model_providers.hollow-fixture={{name=\"Local fixture\",base_url=\"http://127.0.0.1:{port}/v1\",wire_api=\"responses\",requires_openai_auth=false,request_max_retries=0}}"));
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(b"Hello local verification fixture.")
                .await
                .unwrap();
            let output = tokio::time::timeout(Duration::from_secs(25), child.wait_with_output())
                .await
                .unwrap()
                .unwrap();
            assert!(
                !output.status.success(),
                "Fixture must reject before inference"
            );
            assert_eq!(
                server.await.unwrap(),
                json!([]),
                "Restricted transformation {requested:?} must advertise zero tools"
            );
        }
    }

    /// Night ST12 (2026-10-14): the other half of qualifying a CLI version.
    /// The fixture above stops before inference, so it never shows what the
    /// CLI prints for an answer. Here the local fixture answers with a minimal
    /// Responses event stream ("{\"result\":true}"), and the CLI's real
    /// `--json` output must parse to exactly that through `collect_output`.
    /// Still loopback only: no account, prompt or model request leaves.
    /// Prints the CLI's stdout so a version's events can be pinned below.
    #[tokio::test]
    #[ignore = "requires installed Codex; local HTTP fixture only"]
    async fn installed_codex_json_output_parses_from_a_local_fixture_answer() {
        use tokio::io::AsyncReadExt;
        let binary = agent::find_binary(AgentKind::Codex)
            .expect("Codex must be installed for qualification");
        let scratch = Scratch::new().unwrap();
        let catalog_output = Command::new(&binary)
            .args(["debug", "models", "--bundled"])
            .current_dir(&scratch.0)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(catalog_output.status.success());
        let catalog = transform_catalog(&catalog_output.stdout, None).unwrap();
        let path = scratch.0.join("models.json");
        std::fs::write(&path, serde_json::to_vec(&catalog).unwrap()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            let (header_end, content_length) = loop {
                let size = stream.read(&mut chunk).await.unwrap();
                assert!(size > 0);
                raw.extend_from_slice(&chunk[..size]);
                if let Some(index) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&raw[..index]);
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    break (index + 4, length);
                }
            };
            while raw.len() < header_end + content_length {
                let size = stream.read(&mut chunk).await.unwrap();
                if size == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..size]);
            }
            let events = [
                (
                    "response.created",
                    r#"{"type":"response.created","response":{"id":"resp_hollow"}}"#,
                ),
                (
                    "response.output_item.done",
                    r#"{"type":"response.output_item.done","item":{"type":"message","role":"assistant","id":"msg_hollow","content":[{"type":"output_text","text":"{\"result\":true}"}]}}"#,
                ),
                (
                    "response.completed",
                    r#"{"type":"response.completed","response":{"id":"resp_hollow","usage":{"input_tokens":3,"input_tokens_details":null,"output_tokens":4,"output_tokens_details":null,"total_tokens":7}}}"#,
                ),
            ];
            let body: String = events
                .iter()
                .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
                .collect();
            let request: Value =
                serde_json::from_slice(&raw[header_end..header_end + content_length]).unwrap();
            assert_eq!(
                request.get("tools").cloned().unwrap_or_else(|| json!([])),
                json!([]),
                "no tools"
            );
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{body}").as_bytes()).await.unwrap();
            stream.shutdown().await.ok();
        });
        let mut command = transformation_command(&binary, &path, None, &scratch.0);
        // No account at all: an empty Codex home (night ST12 review).
        let home = scratch.0.join("codex-home");
        std::fs::create_dir_all(&home).unwrap();
        command.env("CODEX_HOME", &home);
        command.arg("-c").arg("model_provider=\"hollow-fixture\"");
        command.arg("-c").arg(format!("model_providers.hollow-fixture={{name=\"Local fixture\",base_url=\"http://127.0.0.1:{port}/v1\",wire_api=\"responses\",requires_openai_auth=false,request_max_retries=0,stream_max_retries=0}}"));
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"Answer the fixture.")
            .await
            .unwrap();
        let output = tokio::time::timeout(Duration::from_secs(25), child.wait_with_output())
            .await
            .unwrap()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
        println!(
            "CODEX STDOUT BEGIN\n{}CODEX STDOUT END",
            String::from_utf8_lossy(&output.stdout)
        );
        println!("CODEX STDERR: {}", String::from_utf8_lossy(&output.stderr));
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = test_ledger(dir.path());
        assert_eq!(
            collect_output(output, &mut ledger).unwrap(),
            "{\"result\":true}"
        );
    }

    /// codex-cli 0.162.1's real `exec --json` output for a one-message answer,
    /// captured from `installed_codex_json_output_parses_from_a_local_fixture_answer`
    /// on 2026-10-14 (night ST12). It keeps this version's events parsing as
    /// the parser changes; a new CLI is checked by rerunning the ignored test
    /// and pinning its output here.
    #[cfg(unix)]
    #[test]
    fn codex_0_162_1_exec_json_parses() {
        use std::os::unix::process::ExitStatusExt;
        let stdout = concat!(
            r#"{"type":"thread.started","thread_id":"01a13930-2758-71a2-baf8-589198a7f8d7"}"#,
            "\n",
            r#"{"type":"turn.started"}"#,
            "\n",
            r#"{"type":"item.completed","item":{"id":"item_0","type":"agent_message","text":"{\"result\":true}"}}"#,
            "\n",
            r#"{"type":"turn.completed","usage":{"input_tokens":3,"cached_input_tokens":0,"cache_write_input_tokens":0,"output_tokens":4,"reasoning_output_tokens":0}}"#,
            "\n",
        );
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = test_ledger(dir.path());
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        };
        assert_eq!(
            collect_output(output, &mut ledger).unwrap(),
            "{\"result\":true}"
        );
        let mut state = ParseState::default();
        let tokens = stdout
            .lines()
            .flat_map(|line| agent::parse_line(AgentKind::Codex, line, &mut state))
            .find_map(|event| match event {
                AgentEvent::Result {
                    is_error: false,
                    usage,
                    ..
                } => usage.tokens,
                _ => None,
            })
            .expect("turn.completed carries the usage");
        assert_eq!(
            (tokens.input, tokens.cached_input, tokens.output),
            (3, 0, 4)
        );
    }

    #[cfg(unix)]
    #[test]
    fn output_uses_assistant_result_and_rejects_provider_errors_or_tools() {
        use std::os::unix::process::ExitStatusExt;
        let make = |stdout: &str| std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout: stdout.as_bytes().to_vec(),
            stderr: vec![],
        };
        let dir = tempfile::tempdir().unwrap();
        let mut ledger = test_ledger(dir.path());
        assert_eq!(collect_output(make(r#"{"type":"item.completed","item":{"type":"agent_message","text":"{\"result\":true}"}}"#), &mut ledger).unwrap(), "{\"result\":true}");
        assert!(collect_output(
            make(r#"{"type":"turn.failed","error":{"message":"Account unavailable"}}"#),
            &mut ledger
        )
        .unwrap_err()
        .contains("Account unavailable"));
        assert!(collect_output(make(r#"{"type":"item.started","item":{"type":"command_execution","id":"one","command":"read private file"}}"#), &mut ledger).is_err());
        let _ = ledger.finish(LedgerOutcome::Error);
    }

    fn test_ledger(vault: &Path) -> UsageRun {
        UsageRun::begin(
            Meter::for_vault(
                vault,
                crate::usage_ledger::Surface::Oneshot,
                Some("test".into()),
                None,
            ),
            Some(AgentKind::Codex),
            None,
        )
    }

    #[cfg(unix)]
    #[test]
    fn collect_output_keeps_usage_when_codex_exits_nonzero_after_turn_completed() {
        use std::os::unix::process::ExitStatusExt;
        let stdout = crate::usage::tests::CODEX_EXEC.as_bytes().to_vec();
        let expected = crate::usage::TokenCounts {
            input: 11_849,
            cached_input: 11_904,
            output: 5,
        };
        let dir = tempfile::tempdir().unwrap();

        let mut failed = test_ledger(dir.path());
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(256),
            stdout: stdout.clone(),
            stderr: b"late failure".to_vec(),
        };
        let error = collect_output(output, &mut failed).unwrap_err();
        assert!(error.starts_with("Codex transformation failed:"), "{error}");
        assert_eq!(failed.accumulator().running_total(), Some(expected));
        let _ = failed.finish(LedgerOutcome::Error);

        let mut ok = test_ledger(dir.path());
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(0),
            stdout,
            stderr: vec![],
        };
        assert!(collect_output(output, &mut ok).is_ok());
        assert_eq!(ok.accumulator().running_total(), Some(expected));
        let _ = ok.finish(LedgerOutcome::Success);
    }

    #[test]
    fn transform_outcome_maps_every_ending() {
        assert_eq!(
            transform_outcome(&Ok("x".into()), true),
            LedgerOutcome::Success
        );
        assert_eq!(
            transform_outcome(&Err(Interrupted::TimedOut), true),
            LedgerOutcome::Timeout
        );
        assert_eq!(
            transform_outcome(&Err(Interrupted::Stopped), false),
            LedgerOutcome::Cancelled
        );
        assert_eq!(
            transform_outcome(&Err(Interrupted::Failed("x".into())), true),
            LedgerOutcome::Error
        );
        assert_eq!(
            transform_outcome(&Err(Interrupted::Failed("x".into())), false),
            LedgerOutcome::SpawnFailed
        );
    }

    /// F9 (wave 4 · V1): the guard reads the version once per binary and an
    /// unverified CLI stops before any model turn, saying which version,
    /// the verified ones and the fallback, on the result and the usage row.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_version_guard_names_an_unverified_codex_and_runs_nothing() {
        let vault = tempfile::tempdir().unwrap();
        let meter = Meter::for_vault(
            vault.path(),
            crate::usage_ledger::Surface::Oneshot,
            Some("test".into()),
            None,
        );
        let versions = codex_probe::Cache::default();
        let cli = tempfile::tempdir().unwrap();
        let unverified = codex_probe::tests::fake_codex(cli.path(), "codex-cli 0.160.0");
        let run = |binary: Option<PathBuf>| {
            run_with(
                "prompt",
                None,
                Duration::from_secs(10),
                None,
                &meter,
                binary,
                &versions,
            )
        };
        let refused = run(Some(unverified.clone())).await;
        let error = refused.result.unwrap_err();
        for part in ["codex-cli 0.160.0", "0.146.0, 0.153.4", "Nothing ran"] {
            assert!(error.contains(part), "{part}: {error}");
        }
        let again = run(Some(unverified)).await;
        assert!(again.result.is_err());
        let runs = std::fs::read_to_string(cli.path().join("runs")).unwrap();
        assert_eq!(
            runs.lines().count(),
            1,
            "one version read, then the cache; no other step ran"
        );
        let missing = run(None).await.result.unwrap_err();
        assert!(missing.contains("isn't installed"), "{missing}");

        let rows = crate::session_store::usage_rows_since(vault.path(), "").unwrap();
        assert_eq!(rows.len(), 3);
        for row in &rows {
            assert_eq!(row.outcome, "spawn_failed");
            assert!(!row.spawned);
            let usage: Value = serde_json::from_str(row.usage_json.as_deref().unwrap()).unwrap();
            assert!(
                usage["note"].as_str().unwrap().starts_with("Not run:"),
                "{usage}"
            );
        }

        // A verified version passes the guard and goes on to the catalog
        // step (which this fake CLI fails): the guard didn't stop it.
        let verified_dir = tempfile::tempdir().unwrap();
        let verified = codex_probe::tests::fake_codex(verified_dir.path(), "codex-cli 0.153.4");
        let passed = run(Some(verified)).await.result.unwrap_err();
        assert_eq!(
            passed,
            "Codex could not provide its bundled model metadata."
        );
        let runs = std::fs::read_to_string(verified_dir.path().join("runs")).unwrap();
        assert_eq!(runs.lines().count(), 2, "version, then the catalog");
    }

    /// No process is executed: the stop is checked before the first step.
    #[tokio::test]
    async fn a_transform_stopped_before_its_first_step_writes_one_cancelled_row() {
        let dir = tempfile::tempdir().unwrap();
        let stop = process_group::StopSignal::new();
        stop.request();
        let meter = Meter::for_vault(
            dir.path(),
            crate::usage_ledger::Surface::Oneshot,
            Some("test".into()),
            None,
        );
        let collected = run(
            "prompt",
            None,
            dir.path(),
            Duration::from_secs(5),
            Some(&stop),
            &meter,
        )
        .await;
        assert!(collected.stopped);
        let rows = crate::session_store::usage_rows_since(dir.path(), "").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].surface, "oneshot");
        assert_eq!(rows[0].agent.as_deref(), Some("codex"));
        assert_eq!(rows[0].outcome, "cancelled");
        assert!(!rows[0].spawned);
        assert_eq!(rows[0].cost_status, "unknown");
    }
}
