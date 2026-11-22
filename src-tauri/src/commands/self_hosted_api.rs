//! Settings, Models, Self-hosted models (self-hosted models Q4). Thin:
//! the registry, its file and Test live in `self_hosted_registry`. Every
//! command is the person's Settings click; the COO reaches each through its
//! own `settings.agents.*` action (the `agent_*` functions below, COO parity
//! L13), which never returns the token command. Every change and Test also answers only
//! Hollow's main window (Q4 review), as PDF preview and canvas items do: a
//! `md-preview-*` document window can call app commands.
//! Q6's wake (`self_hosted_wake`) is the
//! person's keystroke, send or Start now in a chat on the model, or the
//! COO's external `settings.agents.wake_self_hosted_model`, since waking a
//! model spends money.
//!
//! Q10: Background jobs (`self_hosted_jobs`): which self-hosted model each
//! kind of Hollow's own model call goes to, Run them now, and a held job's
//! Run as now, Try again and Discard. Every command that changes anything
//! answers the main window only; the COO's actions ask on their cards.

use crate::self_hosted_jobs::{self as jobs, JobKind, SelfHostedJobs, SelfHostedRetry};
use crate::self_hosted_registry::{
    self as registry, SelfHostedEntry, SelfHostedModels, SelfHostedTest,
};
use crate::self_hosted_wake::{SelfHostedWake, SelfHostedWakeStatus};

/// Refuses a call from any window but Hollow's main one.
fn main_window(webview: &tauri::Webview) -> Result<(), String> {
    if webview.label() != "main" {
        return Err("Change self-hosted models in Hollow's main window.".into());
    }
    Ok(())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|e| e.to_string())?
}

/// The saved entries, the preset and the form's limits. Never fails: an
/// unreadable file is the view's `error`.
#[tauri::command]
pub async fn self_hosted_models() -> Result<SelfHostedModels, String> {
    blocking(|| Ok(registry::list())).await
}

/// Add an entry (`edit` false: a new id) or replace the one with its id.
#[tauri::command]
pub async fn save_self_hosted_model(
    webview: tauri::Webview,
    entry: SelfHostedEntry,
    edit: bool,
) -> Result<SelfHostedModels, String> {
    main_window(&webview)?;
    blocking(move || registry::save(entry, edit)).await
}

/// Remove an entry and stop its forwarder.
#[tauri::command]
pub async fn remove_self_hosted_model(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedModels, String> {
    main_window(&webview)?;
    blocking(move || registry::remove(&id)).await
}

/// Switch an entry on or off; off stops its forwarder.
#[tauri::command]
pub async fn set_self_hosted_model_enabled(
    webview: tauri::Webview,
    id: String,
    enabled: bool,
) -> Result<SelfHostedModels, String> {
    main_window(&webview)?;
    blocking(move || registry::set_enabled(&id, enabled)).await
}

/// Test: health, model list and one 1-token completion through the
/// entry's forwarder. A cold start can take several minutes.
#[tauri::command]
pub async fn test_self_hosted_model(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedTest, String> {
    main_window(&webview)?;
    registry::test(id).await
}

/// Wake a chat's self-hosted model (Q6): its forwarder, then its health
/// path, waiting through a cold start; at most one wake per entry per
/// 2 minutes, and a call while one runs shares it. Hollow's main window
/// only (Q6 review): a `md-preview-*` document window can call app commands.
#[tauri::command]
pub async fn wake_self_hosted_model(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedWake, String> {
    if webview.label() != "main" {
        return Err("Start self-hosted models in Hollow's main window.".into());
    }
    crate::self_hosted_wake::wake(id).await
}

/// The entry's last wake, its age and answer (Q6): read from memory, never
/// a call to the model, so the chip can show a start already running.
#[tauri::command]
pub async fn self_hosted_wake_status(
    webview: tauri::Webview,
    id: String,
) -> Result<Option<SelfHostedWakeStatus>, String> {
    if webview.label() != "main" {
        return Err("Start self-hosted models in Hollow's main window.".into());
    }
    Ok(crate::self_hosted_wake::status(&id))
}

/// Background jobs (Q10): each job kind's model, what waits for each
/// entry, and the setting's or queue's error. Never fails.
#[tauri::command]
pub async fn self_hosted_jobs() -> Result<SelfHostedJobs, String> {
    blocking(|| Ok(jobs::view())).await
}

/// Send a job kind to a self-hosted model (`entry`), or As now (`None`).
#[tauri::command]
pub async fn set_self_hosted_job(
    webview: tauri::Webview,
    kind: JobKind,
    entry: Option<String>,
) -> Result<SelfHostedJobs, String> {
    main_window(&webview)?;
    blocking(move || jobs::set(kind, entry.as_deref())).await
}

/// Run them now: wake `entry` and run every job waiting for it.
#[tauri::command]
pub async fn run_self_hosted_jobs_now(
    webview: tauri::Webview,
    entry: String,
) -> Result<SelfHostedJobs, String> {
    main_window(&webview)?;
    blocking(move || {
        jobs::run_now(&entry)?;
        Ok(jobs::view())
    })
    .await
}

/// Run as now (Q10 review): send a held job (its kind went back to As now)
/// where its kind runs today. Only the person's click sends it there.
#[tauri::command]
pub async fn run_self_hosted_job_as_now(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedJobs, String> {
    main_window(&webview)?;
    blocking(move || {
        jobs::run_as_now(&id)?;
        Ok(jobs::view())
    })
    .await
}

/// Try again (Q10 review round 2): a job that gave up waits again, its
/// attempts cleared. A drain of its model starts now (`started`), unless
/// one already runs in this or another Hollow process (`underWay`); then
/// the next drain runs it (review round 4, R3), as when the drain lock
/// can't be probed (neither). `entry` and `name`: the model it waits for
/// now (Q10 follow-up).
#[tauri::command]
pub async fn retry_self_hosted_job(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedRetry, String> {
    main_window(&webview)?;
    blocking(move || jobs::try_again(&id)).await
}

/// Discard a queued job without running it (Q10 review).
#[tauri::command]
pub async fn discard_self_hosted_job(
    webview: tauri::Webview,
    id: String,
) -> Result<SelfHostedJobs, String> {
    main_window(&webview)?;
    blocking(move || {
        jobs::discard(&id)?;
        Ok(jobs::view())
    })
    .await
}

// ── The COO's own entry points (L13 COO parity) ─────────────────────────
//
// Plain functions the registry's `settings.agents.*` actions call, never the
// `#[tauri::command]` functions above (they answer the main window only).
// Nothing here returns a token command: an entry's `auth` is shown as
// `{kind: "tokenCommand", set: true}` or `{kind: "none"}`, and any text of a
// token command that turns up in a result (an error that quotes it) is
// replaced. A token command goes IN only as the `tokenCommand` secret param.
//
// Errors an agent reads are Hollow's own words (L13 review): a model's
// answer can echo the bearer token, a token command's stderr can quote its
// secret arguments, and a malformed registry's serde text can quote any of
// the file. So the registry's read errors, Test's and a wake's errors and
// every kept job error reach an agent as fixed sentences with a short code;
// Settings shows the details to the person.

use crate::app_actions::{Args, Card, Cx};
use crate::self_hosted_registry::{SelfHostedAuth, DEFAULT_HEALTH_PATH};
use serde::Serialize;
use serde_json::{json, Map, Value};

#[cfg(test)]
const HIDDEN: &str = registry::HIDDEN_TOKEN_COMMAND;

/// Where the person reads the details an agent is not given.
const DETAILS_IN_SETTINGS: &str = "Settings, Models, Self-hosted models shows the details";

/// What an agent reads for an unreadable or malformed registry file.
pub(crate) const AGENT_REGISTRY_ERROR: &str = "The self-hosted models list can't be read or doesn't match this Hollow's format (self-hosted-registry). Every change is refused until it is fixed or moved aside; Settings, Models, Self-hosted models shows the details.";

/// Every text of a saved token command, for [`scrub`].
fn token_texts(entries: &[SelfHostedEntry]) -> Vec<String> {
    registry::token_command_texts(entries)
}

/// `value` with every string's token command text replaced.
pub(crate) fn scrub(value: Value, secrets: &[String]) -> Value {
    match value {
        Value::String(s) => Value::String(registry::hide_token_commands(&s, secrets)),
        Value::Array(items) => Value::Array(items.into_iter().map(|v| scrub(v, secrets)).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, v)| (k, scrub(v, secrets)))
                .collect(),
        ),
        other => other,
    }
}

fn saved_secrets() -> Vec<String> {
    registry::entries()
        .map(|entries| token_texts(&entries))
        .unwrap_or_default()
}

/// What an agent reads for a background jobs file that can't be read.
pub(crate) const AGENT_JOBS_FILE_ERROR: &str = "A background jobs file can't be read (self-hosted-jobs-file). Nothing changed; Settings, Models, Self-hosted models shows the details.";

/// An error as an agent reads it: a registry or jobs file's read error (its
/// serde text can quote the file, a kept error in it included) as a fixed
/// sentence, anything else with the token commands' text hidden.
pub(crate) fn agent_error(error: &str, secrets: &[String]) -> String {
    if error.contains(registry::FILE) {
        return AGENT_REGISTRY_ERROR.to_string();
    }
    if error.contains(jobs::JOBS_FILE) || error.contains(jobs::QUEUE_FILE) {
        return AGENT_JOBS_FILE_ERROR.to_string();
    }
    registry::hide_token_commands(error, secrets)
}

/// A result as the COO sees it: serialized, then scrubbed of token commands.
fn clean<T: Serialize>(result: Result<T, String>, secrets: &[String]) -> Result<Value, String> {
    let value = serde_json::to_value(result.map_err(|e| agent_error(&e, secrets))?)
        .map_err(|_| "Hollow couldn't encode the result.".to_string())?;
    Ok(scrub(value, secrets))
}

#[cfg(test)]
fn scrub_text(text: &str, secrets: &[String]) -> String {
    registry::hide_token_commands(text, secrets)
}

/// The list with each entry's `auth` masked, and the file's own read error
/// and each invalid entry's reason in fixed words.
pub(crate) fn mask_models(mut models: SelfHostedModels) -> Result<Value, String> {
    let secrets = token_texts(&models.entries);
    if models.error.is_some() {
        models.error = Some(AGENT_REGISTRY_ERROR.to_string());
    }
    for why in models.invalid.values_mut() {
        *why = registry::hide_token_commands(why, &secrets);
    }
    let mut value = serde_json::to_value(&models)
        .map_err(|_| "Hollow couldn't encode the result.".to_string())?;
    let mask = |entry: &mut Value| {
        let set = entry
            .pointer("/auth/kind")
            .and_then(Value::as_str)
            .map(|k| k == "tokenCommand")
            .unwrap_or(false);
        if let Some(obj) = entry.as_object_mut() {
            obj.insert(
                "auth".into(),
                if set {
                    json!({ "kind": "tokenCommand", "set": true })
                } else {
                    json!({ "kind": "none" })
                },
            );
        }
    };
    if let Some(entries) = value.get_mut("entries").and_then(Value::as_array_mut) {
        entries.iter_mut().for_each(mask);
    }
    if let Some(preset) = value.get_mut("preset") {
        mask(preset);
    }
    Ok(scrub(value, &secrets))
}

/// A Test's result for an agent: the numbers as they are, its error in
/// fixed words from what failed (the health check, the model list, the
/// 1-token request), never the model's or the token command's own.
pub(crate) fn agent_test_result(mut test: SelfHostedTest) -> SelfHostedTest {
    if test.error.is_none() {
        return test;
    }
    let ok = |s: u16| (200..300).contains(&s);
    test.error = Some(match (test.health_status, test.status) {
        (None, _) => format!("Hollow couldn't reach the model (self-hosted-health): its forwarder, its token command or the health check failed. {DETAILS_IN_SETTINGS} (Test)."),
        (Some(h), _) if !ok(h) => format!("The health check answered {h} (self-hosted-health). {DETAILS_IN_SETTINGS} (Test)."),
        (Some(_), Some(s)) if !ok(s) => format!("The model answered {s} to the 1-token request (self-hosted-request). {DETAILS_IN_SETTINGS} (Test)."),
        (Some(_), _) => format!("The 1-token request didn't finish (self-hosted-request): the model stopped, was switched off or didn't answer in time. {DETAILS_IN_SETTINGS} (Test)."),
    });
    test
}

/// A wake's answer for an agent: its error in fixed words.
pub(crate) fn agent_wake_result(mut wake: SelfHostedWake) -> SelfHostedWake {
    if wake.error.is_some() {
        wake.error = Some(match wake.status {
            Some(status) => format!("The health check answered {status} (self-hosted-wake). {DETAILS_IN_SETTINGS}."),
            None => format!("The model didn't come up (self-hosted-wake): its forwarder, its token command or the address failed, or it took too long. {DETAILS_IN_SETTINGS}."),
        });
    }
    wake
}

/// The background jobs for an agent: every kept error (a file that can't
/// be read, a drain's or a rerun's last error, kept before any later auth
/// change) as a fixed sentence.
pub(crate) fn agent_jobs_view(mut view: SelfHostedJobs) -> SelfHostedJobs {
    if view.error.is_some() {
        view.error = Some(format!("The background jobs setting can't be read (self-hosted-jobs-file), so every kind is held and nothing changes until it is fixed or moved aside. {DETAILS_IN_SETTINGS}."));
    }
    if view.queue_error.is_some() {
        view.queue_error = Some(format!("The background jobs queue can't be read (self-hosted-queue), so jobs that would wait are skipped. {DETAILS_IN_SETTINGS}."));
    }
    for row in &mut view.kinds {
        if row
            .problem
            .as_deref()
            .is_some_and(|p| p.contains("can't be read"))
        {
            row.problem = Some(format!("Held: a file it needs can't be read (self-hosted-registry). {DETAILS_IN_SETTINGS}."));
        }
    }
    for waiting in &mut view.waiting {
        if waiting.last_error.is_some() {
            waiting.last_error = Some(format!("The last run of its waiting jobs stopped with an error (self-hosted-drain). {DETAILS_IN_SETTINGS}."));
        }
    }
    for held in &mut view.held {
        if held.last_error.is_some() {
            held.last_error = Some(format!(
                "Its last rerun failed (self-hosted-job). {DETAILS_IN_SETTINGS}."
            ));
        }
    }
    view
}

pub(crate) async fn agent_list() -> Result<Value, String> {
    blocking(|| mask_models(registry::list())).await
}

fn text_of(args: &Map<String, Value>, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The entry a save's params describe. `tokenCommand` is one command line
/// split on spaces (no shell): its first word is the program. Absent: an
/// open endpoint when adding, the saved auth when editing; `none` clears it.
pub(crate) fn entry_from_args(
    args: &Map<String, Value>,
    saved: Option<&SelfHostedEntry>,
) -> Result<SelfHostedEntry, String> {
    let need = |key: &str| {
        text_of(args, key).ok_or_else(|| format!("`{key}` is needed. Nothing changed."))
    };
    let preset = registry::preset();
    let auth = match args.get("tokenCommand").and_then(Value::as_str) {
        Some(line) if line.trim().eq_ignore_ascii_case("none") => SelfHostedAuth::None,
        Some(line) if !line.trim().is_empty() => {
            let mut words = line.split_whitespace().map(str::to_string);
            let program = words.next().unwrap_or_default();
            SelfHostedAuth::TokenCommand {
                program,
                args: words.collect(),
            }
        }
        _ => saved
            .map(|e| e.auth.clone())
            .unwrap_or(SelfHostedAuth::None),
    };
    Ok(SelfHostedEntry {
        id: need("id")?,
        name: need("name")?,
        base_url: need("baseUrl")?,
        api_style: text_of(args, "apiStyle")
            .or_else(|| saved.map(|e| e.api_style.clone()))
            .unwrap_or_else(|| registry::API_ANTHROPIC.to_string()),
        served_model: need("servedModel")?,
        context_window: args
            .get("contextWindow")
            .and_then(Value::as_u64)
            .map(|n| n.min(u64::from(u32::MAX)) as u32)
            .ok_or("`contextWindow` is needed. Nothing changed.")?,
        effort_levels: match args.get("effortLevels").and_then(Value::as_array) {
            Some(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            None => saved
                .map(|e| e.effort_levels.clone())
                .unwrap_or(preset.effort_levels),
        },
        auth,
        health_path: text_of(args, "healthPath")
            .or_else(|| saved.map(|e| e.health_path.clone()))
            .unwrap_or_else(|| DEFAULT_HEALTH_PATH.to_string()),
        enabled: args
            .get("enabled")
            .and_then(Value::as_bool)
            .or_else(|| saved.map(|e| e.enabled))
            .unwrap_or(true),
    })
}

/// Save from the COO: the saved entry is read, the omitted fields merged
/// from it and the result written under one registry lock
/// (`registry::save_with`), and a file that can't be read refuses the save.
pub(crate) async fn agent_save(args: Map<String, Value>) -> Result<Value, String> {
    let edit = args.get("edit").and_then(Value::as_bool).unwrap_or(false);
    blocking(move || {
        let id = text_of(&args, "id").ok_or("`id` is needed. Nothing changed.")?;
        let mut secrets = saved_secrets();
        let saved = registry::save_with(&id, edit, |saved| {
            let entry = entry_from_args(&args, saved)?;
            secrets.extend(token_texts(std::slice::from_ref(&entry)));
            Ok(entry)
        });
        match saved {
            Ok(models) => mask_models(models),
            Err(why) => Err(agent_error(&why, &secrets)),
        }
    })
    .await
}

pub(crate) async fn agent_remove(id: String) -> Result<Value, String> {
    blocking(move || {
        let secrets = saved_secrets();
        registry::remove(&id)
            .map_err(|why| agent_error(&why, &secrets))
            .and_then(mask_models)
    })
    .await
}

pub(crate) async fn agent_set_enabled(id: String, enabled: bool) -> Result<Value, String> {
    blocking(move || {
        let secrets = saved_secrets();
        registry::set_enabled(&id, enabled)
            .map_err(|why| agent_error(&why, &secrets))
            .and_then(mask_models)
    })
    .await
}

pub(crate) async fn agent_test(id: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(registry::test(id).await.map(agent_test_result), &secrets)
}

pub(crate) async fn agent_wake(id: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        crate::self_hosted_wake::wake(id)
            .await
            .map(agent_wake_result),
        &secrets,
    )
}

pub(crate) fn agent_wake_status(id: &str) -> Result<Value, String> {
    let secrets = saved_secrets();
    let status = crate::self_hosted_wake::status(id).map(|mut s| {
        s.result = s.result.map(agent_wake_result);
        s
    });
    clean(Ok(status), &secrets)
}

pub(crate) async fn agent_jobs() -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        blocking(|| Ok(agent_jobs_view(jobs::view()))).await,
        &secrets,
    )
}

pub(crate) async fn agent_set_job(kind: &str, entry: Option<String>) -> Result<Value, String> {
    let kind: JobKind = serde_json::from_value(Value::String(kind.to_string()))
        .map_err(|_| format!("`{kind}` is not a job kind. Nothing changed."))?;
    let secrets = saved_secrets();
    clean(
        blocking(move || jobs::set(kind, entry.as_deref()).map(agent_jobs_view)).await,
        &secrets,
    )
}

pub(crate) async fn agent_run_now(entry: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        blocking(move || {
            jobs::run_now(&entry)?;
            Ok(agent_jobs_view(jobs::view()))
        })
        .await,
        &secrets,
    )
}

pub(crate) async fn agent_run_as_now(id: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        blocking(move || {
            jobs::run_as_now(&id)?;
            Ok(agent_jobs_view(jobs::view()))
        })
        .await,
        &secrets,
    )
}

pub(crate) async fn agent_retry(id: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        blocking(move || {
            jobs::try_again(&id).map(|mut retry| {
                retry.jobs = agent_jobs_view(retry.jobs);
                retry
            })
        })
        .await,
        &secrets,
    )
}

pub(crate) async fn agent_discard(id: String) -> Result<Value, String> {
    let secrets = saved_secrets();
    clean(
        blocking(move || {
            jobs::discard(&id)?;
            Ok(agent_jobs_view(jobs::view()))
        })
        .await,
        &secrets,
    )
}

// ── Cards ───────────────────────────────────────────────────────────────

fn arg(a: &Args, key: &str) -> String {
    a.0.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The saved entry's name and address, or the id and a plain "address not
/// found" (a card never fails a sweep over a missing entry).
fn entry_label(id: &str) -> (String, String) {
    match registry::find(id) {
        Ok(entry) => (format!("{} ({})", entry.name, entry.id), entry.base_url),
        Err(_) => (
            id.to_string(),
            "an address not found in the saved list".into(),
        ),
    }
}

/// The entry a held job waits for.
fn job_entry(job_id: &str) -> String {
    jobs::view()
        .held
        .into_iter()
        .find(|j| j.id == job_id)
        .map(|j| j.entry)
        .unwrap_or_default()
}

pub(crate) fn save_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let token = match a.0.get("tokenCommand").and_then(Value::as_str) {
        Some(t) if t.trim().eq_ignore_ascii_case("none") => "none (open endpoint)",
        Some(t) if !t.trim().is_empty() => "set (its text is never shown)",
        _ => "unchanged (none for a new entry)",
    };
    let edit = a.0.get("edit").and_then(Value::as_bool).unwrap_or(false);
    Ok(Some(
        Card::new("Hollow app tools: save a self-hosted model")
            .line(format!(
                "{} the self-hosted model {} ({}).",
                if edit { "Replace" } else { "Add" },
                arg(a, "name"),
                arg(a, "id")
            ))
            .line(format!("Address the token goes to: {}.", arg(a, "baseUrl")))
            .line(format!("Token command: {token}.")),
    ))
}

pub(crate) fn remove_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (label, _) = entry_label(&arg(a, "id"));
    Ok(Some(
        Card::new("Hollow app tools: remove a self-hosted model").line(format!(
            "Remove {label} and stop its forwarder. Chats and background jobs set to it stop working."
        )),
    ))
}

pub(crate) fn set_job_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let kind = arg(a, "kind");
    let line = match a.0.get("entry").and_then(Value::as_str).map(str::trim) {
        Some(id) if !id.is_empty() => {
            let (label, address) = entry_label(id);
            format!("Send {kind} calls to {label} at {address}. Private data in those calls leaves this Mac for that host.")
        }
        _ => format!("Send {kind} calls back to where they run today (As now)."),
    };
    Ok(Some(
        Card::new("Hollow app tools: route background model calls").line(line),
    ))
}

pub(crate) fn run_as_now_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    Ok(Some(
        Card::new("Hollow app tools: run a held job as now").line(format!(
            "Send held job {} to where its kind runs today, which may be a hosted model. Private data in it goes there.",
            arg(a, "id")
        )),
    ))
}

pub(crate) fn discard_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    Ok(Some(
        Card::new("Hollow app tools: discard a background job").line(format!(
            "Drop queued job {} without running it. It cannot be brought back.",
            arg(a, "id")
        )),
    ))
}

pub(crate) fn test_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (label, address) = entry_label(&arg(a, "id"));
    Ok(Some(
        Card::new("Hollow app tools: test a self-hosted model")
            .line(format!(
                "Wake {label} and send it a 1-token request. A cold GPU can take minutes and costs money while it is up."
            ))
            .to(address),
    ))
}

pub(crate) fn wake_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (label, address) = entry_label(&arg(a, "id"));
    Ok(Some(
        Card::new("Hollow app tools: wake a self-hosted model")
            .line(format!(
                "Start {label} and wait through a cold start. Its GPU costs money while it is up."
            ))
            .to(address),
    ))
}

pub(crate) fn run_now_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (label, address) = entry_label(&arg(a, "entry"));
    Ok(Some(
        Card::new("Hollow app tools: run background jobs now")
            .line(format!(
                "Wake {label} and run every job waiting for it. Costs GPU time and sends the jobs' private data (recordings, chats, vault text) to it."
            ))
            .to(address),
    ))
}

pub(crate) fn retry_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let id = arg(a, "id");
    let (label, address) = entry_label(&job_entry(&id));
    Ok(Some(
        Card::new("Hollow app tools: retry a background job")
            .line(format!(
                "Run job {id} again on {label}, waking it. Costs GPU time and sends the job's private data to it."
            ))
            .to(address),
    ))
}

#[cfg(test)]
mod coo_tests {
    use super::*;
    use crate::self_hosted_registry::SelfHostedModels;

    fn with_token(program: &str, args: &[&str]) -> SelfHostedEntry {
        SelfHostedEntry {
            auth: SelfHostedAuth::TokenCommand {
                program: program.into(),
                args: args.iter().map(|s| s.to_string()).collect(),
            },
            ..registry::preset()
        }
    }

    fn models(entries: Vec<SelfHostedEntry>) -> SelfHostedModels {
        SelfHostedModels {
            entries,
            preset: registry::preset(),
            levels: vec!["low".into()],
            min_context_window: 1,
            max_context_window: 2,
            error: None,
            invalid: Default::default(),
        }
    }

    #[test]
    fn list_masks_every_token_command() {
        let entry = with_token("/opt/secret-bin/mint", &["--audience", "hush-hush-aud"]);
        let value = mask_models(models(vec![entry])).unwrap();
        let text = value.to_string();
        for leak in [
            "secret-bin",
            "mint",
            "hush-hush-aud",
            "print-identity-token",
            "gcloud",
        ] {
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
        assert_eq!(
            value.pointer("/entries/0/auth"),
            Some(&json!({ "kind": "tokenCommand", "set": true }))
        );
        let open = SelfHostedEntry {
            auth: SelfHostedAuth::None,
            ..registry::preset()
        };
        let value = mask_models(models(vec![open])).unwrap();
        assert_eq!(
            value.pointer("/entries/0/auth"),
            Some(&json!({ "kind": "none" }))
        );
    }

    #[test]
    fn scrub_hides_a_token_command_quoted_in_an_error() {
        let entry = with_token("get-token-tool", &["--scope", "x-scope-77"]);
        let secrets = token_texts(&[entry]);
        let value = scrub(
            json!({ "error": "get-token-tool --scope x-scope-77 failed", "other": ["get-token-tool"] }),
            &secrets,
        );
        let text = value.to_string();
        assert!(
            !text.contains("get-token-tool") && !text.contains("x-scope-77"),
            "{text}"
        );
        assert!(text.contains(HIDDEN));
        assert!(!scrub_text("get-token-tool died", &secrets).contains("get-token-tool"));
    }

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn save_params_become_an_entry_and_the_token_command_splits() {
        let base = json!({
            "id": "mine", "name": "Mine", "baseUrl": "https://m.example/", "servedModel": "m-1",
            "contextWindow": 131072, "tokenCommand": "tok-prog --aud a b"
        });
        let entry = entry_from_args(&params(base.clone()), None).unwrap();
        assert_eq!(entry.id, "mine");
        assert_eq!(entry.context_window, 131_072);
        assert_eq!(entry.health_path, DEFAULT_HEALTH_PATH);
        assert!(entry.enabled);
        assert_eq!(
            entry.auth,
            SelfHostedAuth::TokenCommand {
                program: "tok-prog".into(),
                args: vec!["--aud".into(), "a".into(), "b".into()],
            }
        );
        // Editing without a token command keeps the saved one; "none" clears it.
        let mut no_token = params(base.clone());
        no_token.remove("tokenCommand");
        let kept = entry_from_args(&no_token, Some(&entry)).unwrap();
        assert_eq!(kept.auth, entry.auth);
        assert_eq!(
            entry_from_args(&no_token, None).unwrap().auth,
            SelfHostedAuth::None
        );
        no_token.insert("tokenCommand".into(), json!("none"));
        assert_eq!(
            entry_from_args(&no_token, Some(&entry)).unwrap().auth,
            SelfHostedAuth::None
        );
        let mut missing = params(base);
        missing.remove("servedModel");
        assert!(entry_from_args(&missing, None)
            .unwrap_err()
            .contains("servedModel"));
    }
}
