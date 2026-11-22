//! Remote runners (2026-11-03): what a workspace's runner turns may reach
//! (Runs on → On the runner: GitHub, Google Cloud) and where the GitHub
//! credential comes from (Settings, Devices, Remote runner).
//!
//! COO parity (L13, 2026-11-10): each command's body is a plain
//! `pub(crate)` function the Hollow app actions call too (`build.runner_*`,
//! `build.set_runner_access`), never the command itself, behind the card
//! builders below. Granting credentials, saving a token and naming the
//! runner are confirm actions that still ask in `auto`; ssh, gcloud and the
//! billed Google Cloud machine are external. A token goes in only, as a
//! secret parameter: nothing here returns one.

use crate::app_actions::{cut, Args, Card, Cx};
use crate::commands::AppState;
use crate::runner_access::{self, CloudAccess, GithubSource, GithubStatus, RunnerAccess};
use crate::runner_setup::{self, SetupParams};
use serde_json::{json, Value};
use tauri::State;

/// Workspace `id`'s runner access (all off until the person turns one on).
#[tauri::command]
pub async fn runner_access_get(
    id: String,
    state: State<'_, AppState>,
) -> Result<RunnerAccess, String> {
    Ok(runner_access::access_for(&state.config.vault_path, &id))
}

/// Set workspace `id`'s runner access; returns what it is now.
#[tauri::command]
pub async fn runner_access_set(
    id: String,
    access: RunnerAccess,
    state: State<'_, AppState>,
) -> Result<RunnerAccess, String> {
    runner_access::set_access(&state.config.vault_path, &id, access)
}

/// Where the runner's GitHub credential comes from, whether a token is
/// saved and whether this Mac's gh is signed in (never a token).
pub(crate) async fn github_status() -> Result<GithubStatus, String> {
    tokio::task::spawn_blocking(runner_access::github_status)
        .await
        .map_err(|e| e.to_string())
}

/// Where the runner's GitHub credential comes from, whether a runner token
/// is saved and whether this Mac's gh is signed in (never a token).
#[tauri::command]
pub async fn runner_github_status() -> Result<GithubStatus, String> {
    github_status().await
}

/// Save `token` as the GitHub runner token in this Mac's Keychain; the
/// status after, which never carries it.
pub(crate) async fn github_save_token(token: String) -> Result<GithubStatus, String> {
    tokio::task::spawn_blocking(move || runner_access::save_github_token(&token))
        .await
        .map_err(|e| e.to_string())??;
    github_status().await
}

/// Save the GitHub runner token in this Mac's Keychain.
#[tauri::command]
pub async fn runner_github_save_token(token: String) -> Result<GithubStatus, String> {
    github_save_token(token).await
}

/// Remove the saved GitHub runner token; the status after.
pub(crate) async fn github_clear_token() -> Result<GithubStatus, String> {
    tokio::task::spawn_blocking(runner_access::clear_github_token)
        .await
        .map_err(|e| e.to_string())??;
    github_status().await
}

/// Remove the saved GitHub runner token.
#[tauri::command]
pub async fn runner_github_clear_token() -> Result<GithubStatus, String> {
    github_clear_token().await
}

/// Use `source` as the runner's GitHub credential; the status after.
pub(crate) async fn github_set_source(source: GithubSource) -> Result<GithubStatus, String> {
    runner_access::set_github_source(source)?;
    github_status().await
}

/// Use the runner token (`runner-token`) or this Mac's gh login (`gh-login`).
#[tauri::command]
pub async fn runner_github_set_source(source: GithubSource) -> Result<GithubStatus, String> {
    github_set_source(source).await
}

/// Name `host` as this Mac's runner, or turn the runner off (`None`, or a
/// blank host); the runner now in use.
pub(crate) fn name_runner(host: Option<String>) -> Result<Option<String>, String> {
    let host = host.map(|h| h.trim().to_string()).filter(|h| !h.is_empty());
    crate::runner_owner::set_runner(host.as_deref())
}

/// Remote runners 2 (2026-11-03): name `host` as this Mac's runner, or
/// turn the runner off (`null`); returns the runner now in use. The
/// person's click in Settings, Devices, Remote runner, or the COO's
/// `build.runner_set` on its card.
#[tauri::command]
pub async fn runner_set(host: Option<String>) -> Result<Option<String>, String> {
    name_runner(host)
}

/// The read-only runner check of `host` (ssh), not kept as the runner's
/// status.
pub(crate) async fn check_host(host: String) -> Result<crate::remote_status::RunnerStatus, String> {
    let host = host.trim().to_string();
    tokio::task::spawn_blocking(move || crate::remote_status::check("ssh", &host))
        .await
        .map_err(|e| e.to_string())
}

/// Ask `host` how it is before naming it the runner (Set up → Check): the
/// same read-only check as Check now, not kept as the runner's status.
#[tauri::command]
pub async fn runner_check_host(host: String) -> Result<crate::remote_status::RunnerStatus, String> {
    check_host(host).await
}

/// Record this Hollow as the configured runner's owner, then check it
/// again; the fresh status.
pub(crate) async fn record_owner() -> Result<crate::remote_status::RunnerStatus, String> {
    tokio::task::spawn_blocking(|| {
        let host = crate::remote_session::configured_runner()
            .ok_or("No remote runner is set up on this Mac.")?;
        crate::runner_owner::record_owner("ssh", &host)?;
        Ok::<_, String>(crate::remote_status::status(true))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Record this Hollow as the runner's owner (`~/hollow-remote/owner.json`
/// there), then check it again; the fresh status.
#[tauri::command]
pub async fn runner_record_owner() -> Result<crate::remote_status::RunnerStatus, String> {
    record_owner().await
}

/// Set up's steps for `params` (runs nothing but `ssh -G`).
pub(crate) async fn setup_plan(params: SetupParams) -> Result<runner_setup::SetupPlan, String> {
    tokio::task::spawn_blocking(move || {
        let ctx = runner_setup::Ctx::real()?;
        Ok(runner_setup::plan(&ctx, &params))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Remote runners 3 (2026-11-03): Set up's steps for `params` (its fields
/// with defaults, what isn't valid, every step's check and apply commands,
/// for Google Cloud the monthly estimate). Runs nothing but `ssh -G`.
#[tauri::command]
pub async fn runner_setup_plan(params: SetupParams) -> Result<runner_setup::SetupPlan, String> {
    setup_plan(params).await
}

/// Every step's read-only check commands, in order.
pub(crate) async fn setup_check(
    params: SetupParams,
) -> Result<Vec<runner_setup::StepState>, String> {
    tokio::task::spawn_blocking(move || {
        let ctx = runner_setup::Ctx::real()?;
        runner_setup::check(&ctx, &params)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set up → Check: every step's read-only check commands, in order.
#[tauri::command]
pub async fn runner_setup_check(
    params: SetupParams,
) -> Result<Vec<runner_setup::StepState>, String> {
    setup_check(params).await
}

/// One step's apply commands, then its check.
pub(crate) async fn setup_apply(
    params: SetupParams,
    step: String,
) -> Result<runner_setup::StepState, String> {
    tokio::task::spawn_blocking(move || {
        let ctx = runner_setup::Ctx::real()?;
        runner_setup::apply(&ctx, &params, &step)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set up → a step's Apply (the person's click, after its confirm): its
/// apply commands, then its check.
#[tauri::command]
pub async fn runner_setup_apply(
    params: SetupParams,
    step: String,
) -> Result<runner_setup::StepState, String> {
    setup_apply(params, step).await
}

/// The Google Cloud machine's monthly estimate for `params`.
pub(crate) async fn cloud_estimate(params: SetupParams) -> Result<runner_setup::Estimate, String> {
    tokio::task::spawn_blocking(move || {
        let ctx = runner_setup::Ctx::real()?;
        runner_setup::estimate(&runner_setup::filled(&ctx, &params))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The Google Cloud machine's monthly estimate (approximate list prices).
#[tauri::command]
pub async fn runner_cloud_estimate(params: SetupParams) -> Result<runner_setup::Estimate, String> {
    cloud_estimate(params).await
}

/// Create the Google Cloud machine with the estimate (in cents) the person
/// confirmed; any other runs nothing.
pub(crate) async fn cloud_create(
    params: SetupParams,
    estimate_cents: u64,
) -> Result<runner_setup::StepState, String> {
    tokio::task::spawn_blocking(move || {
        let ctx = runner_setup::Ctx::real()?;
        runner_setup::create_cloud(&ctx, &params, estimate_cents)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Create the Google Cloud machine, only with the estimate (in cents) the
/// person confirmed; any other runs nothing.
#[tauri::command]
pub async fn runner_cloud_create(
    params: SetupParams,
    estimate_cents: u64,
) -> Result<runner_setup::StepState, String> {
    cloud_create(params, estimate_cents).await
}

// ---- COO parity (L13): what an agent reads back from Set up ---------------
//
// A step's state carries each command's raw output and sentences built from
// it: `ssh -G`'s whole configuration (a ProxyCommand can hold a credential),
// gcloud's and the runner's own words. An agent gets allowlisted fields
// only: the step, its state in fixed words, whether it needs the person's
// hands, and each command as the plan shows it with its exit code. Settings
// shows the output to the person.

/// One command a step ran, as an agent reads it: never its output.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentRanCmd {
    /// The command as the plan shows it (built from Set up's fields).
    pub shown: String,
    /// Its exit code; `None` when it didn't run (or didn't finish).
    pub code: Option<i32>,
}

/// One step's state, as an agent reads it (see above).
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AgentStepState {
    pub id: String,
    /// `done`, `missing`, `unknown` or `failed`.
    pub state: &'static str,
    /// Fixed words for the state, never a command's.
    pub summary: &'static str,
    /// sudo wanted a password there: the step's by-hand commands do it.
    pub needs_by_hand: bool,
    pub ran: Vec<AgentRanCmd>,
}

/// Where the person reads what the commands printed.
const OUTPUT_IN_SETTINGS: &str =
    "Settings, Devices, Remote runner, Set up shows what the commands printed.";

/// `state` with every output-derived string left out.
pub(crate) fn for_agent(state: runner_setup::StepState) -> AgentStepState {
    let summary = match state.state {
        "done" => "Done: its check passes.",
        "missing" => "Missing: its check didn't find it. build.runner_setup_apply applies it, or its by-hand commands do (build.runner_setup_plan lists them).",
        "unknown" => "Couldn't tell: a check command failed or didn't answer.",
        "failed" => "The apply stopped at a command; what ran before it stays.",
        _ => "See Settings for this step.",
    };
    AgentStepState {
        id: state.id,
        state: state.state,
        summary,
        needs_by_hand: state.needs_by_hand,
        ran: state
            .ran
            .into_iter()
            .map(|r| AgentRanCmd {
                shown: r.shown,
                code: r.code,
            })
            .collect(),
    }
}

/// `build.runner_setup_check`: every step's check, allowlisted.
pub(crate) async fn agent_setup_check(params: SetupParams) -> Result<Value, String> {
    let states = setup_check(params).await?;
    Ok(json!({
        "steps": states.into_iter().map(for_agent).collect::<Vec<_>>(),
        "output": OUTPUT_IN_SETTINGS,
    }))
}

/// `build.runner_setup_apply`: one step's apply and check, allowlisted.
pub(crate) async fn agent_setup_apply(params: SetupParams, step: String) -> Result<Value, String> {
    let state = setup_apply(params, step).await?;
    Ok(json!({ "step": for_agent(state), "output": OUTPUT_IN_SETTINGS }))
}

/// `build.runner_cloud_create`: the machine step's state, allowlisted.
pub(crate) async fn agent_cloud_create(
    params: SetupParams,
    estimate_cents: u64,
) -> Result<Value, String> {
    let state = cloud_create(params, estimate_cents).await?;
    Ok(json!({ "step": for_agent(state), "output": OUTPUT_IN_SETTINGS }))
}

// ---- COO parity (L13): the cards the Hollow app actions show --------------

/// A string argument (validated already), or "".
fn arg(a: &Args, key: &str) -> String {
    a.0.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// The `params` object of the `build.runner_setup_*` and
/// `build.runner_cloud_*` actions (Set up's fields; blank ones take their
/// defaults, as `build.runner_setup_plan` shows).
pub(crate) fn setup_params_schema() -> Value {
    let s = |about: &str| json!({ "type": "string", "description": about });
    let n = |about: &str| json!({ "type": "integer", "minimum": 0, "description": about });
    json!({
        "type": "object",
        "properties": {
            "path": { "type": "string", "enum": ["mac", "cloud"], "description": "mac: a Mac you own, always on; cloud: a Google Cloud machine." },
            "admin": s("mac: an SSH destination for an admin account there (user@host or an alias), setup only."),
            "alias": s("The SSH Host alias this Mac reaches the runner by."),
            "account": s("mac: the runner account there."),
            "keyFile": s("mac: the private key this Mac uses for the runner (~/.ssh/...)."),
            "hostName": s("mac: its address for the Host block."),
            "project": s("cloud: the Google Cloud project id."),
            "zone": s("cloud: the zone (default us-central1-a)."),
            "machineType": s("cloud: one of the plan's machineTypes (default e2-standard-8)."),
            "diskGb": n("cloud: boot disk, 20 to 2000 GB."),
            "idleMinutes": n("cloud: idle minutes before it powers off."),
            "hoursPerMonth": n("cloud: expected hours a month, 1 to 744 (the estimate)."),
            "billingAccount": s("cloud: the billing account for the budget alert."),
            "budgetUsd": n("cloud: the monthly budget alert in USD."),
            "instance": s("cloud: the machine's name."),
            "serviceAccount": s("cloud: the no-role service account's name.")
        },
        "additionalProperties": false
    })
}

/// The `host` argument of `build.runner_set`: `off` (or blank) turns the
/// runner off (`None`); anything else is the host to name.
pub(crate) fn host_choice(raw: &str) -> Option<String> {
    let host = raw.trim();
    (!host.is_empty() && !host.eq_ignore_ascii_case("off")).then(|| host.to_string())
}

fn source_label(source: GithubSource) -> &'static str {
    match source {
        GithubSource::RunnerToken => "the runner token saved in this Mac's Keychain",
        GithubSource::GhLogin => "this Mac's gh login",
    }
}

fn token_saved() -> bool {
    matches!(
        crate::secret_store::present(runner_access::GITHUB_TOKEN_KEY),
        Ok(true)
    )
}

/// `build.runner_github_save_token`: refused before any card when the
/// token doesn't look like one (the refusal never echoes it); the card
/// shows it only as ••••.
pub(crate) fn save_token_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let token = a.0.get("token").and_then(Value::as_str).unwrap_or_default();
    runner_access::github_token_ok(token).map_err(|why| format!("{why} Nothing was saved."))?;
    let replaces = if token_saved() {
        "It replaces the runner token saved now."
    } else {
        "No runner token is saved now."
    };
    Ok(Some(
        Card::new("Hollow app tools: save the GitHub runner token")
            .line("Save a GitHub token in this Mac's Keychain for runner turns that have GitHub on (push, pull requests, merges).")
            .line("Token: ••••")
            .line(replaces)
            .line(format!(
                "The runner's GitHub credential comes from {} now.",
                source_label(runner_access::github_source())
            )),
    ))
}

/// `build.runner_github_clear_token`.
pub(crate) fn clear_token_card(_cx: &Cx, _a: &Args) -> Result<Option<Card>, String> {
    let mut card = Card::new("Hollow app tools: remove the GitHub runner token").line(
        "Remove the GitHub runner token saved in this Mac's Keychain. Getting it back means making a new token on GitHub.",
    );
    card = card.line(if token_saved() {
        "A runner token is saved now."
    } else {
        "No runner token is saved now, so this changes nothing."
    });
    if runner_access::github_source() == GithubSource::RunnerToken {
        card = card.line("The runner token is the GitHub source now: runner turns with GitHub on can't reach GitHub until a token is saved again.");
    }
    Ok(Some(card))
}

/// `build.runner_github_set_source`: old → new, and what the broader gh
/// login means.
pub(crate) fn set_source_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let new: GithubSource = serde_json::from_value(json!(arg(a, "source")))
        .map_err(|_| "source is runner-token or gh-login. Nothing changed.".to_string())?;
    let old = runner_access::github_source();
    let mut card = Card::new("Hollow app tools: the runner's GitHub credential").line(format!(
        "GitHub credential for runner turns: {} → {}.",
        source_label(old),
        source_label(new)
    ));
    if new == GithubSource::GhLogin {
        card = card.line("This Mac's gh login is broader than a runner token: runner turns with GitHub on can do whatever that login may do, in every repository it reaches.");
    }
    Ok(Some(card))
}

/// `build.runner_set`: old → new runner, refused for a host that isn't one.
pub(crate) fn runner_set_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let new = host_choice(&arg(a, "host"));
    if let Some(host) = &new {
        if !crate::remote_session::valid_host(host) || matches!(host.as_str(), "local" | "none") {
            return Err(format!("{host} is not an SSH host name. Nothing changed."));
        }
    }
    let old = crate::remote_session::configured_runner();
    let shown = |h: &Option<String>| h.clone().unwrap_or_else(|| "Off".into());
    let mut card = Card::new("Hollow app tools: this Mac's remote runner")
        .line(format!("Remote runner: {} → {}.", shown(&old), shown(&new)))
        .line("Writes ~/.config/hollow/slot-remote.");
    card = card.line(match &new {
        Some(host) => format!("hollow-slot builds and gates, and turns set to run on the remote runner, then run on {host} over ssh, with their checkout sent there. Naming it turns nothing on by itself."),
        None => "Builds, gates and turns set to run on the remote runner then run on this Mac.".into(),
    });
    Ok(Some(card))
}

/// `build.runner_check_host`: names the host it asks.
pub(crate) fn check_host_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let host = arg(a, "host");
    if !crate::remote_session::valid_host(&host) {
        return Err(format!("{host} is not an SSH host name. Nothing was sent."));
    }
    Ok(Some(
        Card::new("Hollow app tools: check a runner host")
            .line(format!("Connect to {host} over ssh and run the read-only runner check (its preflight line, sessions, gates and owner). Nothing changes there or here."))
            .to(format!("ssh {host}")),
    ))
}

/// `build.runner_record_owner`: refused when no runner is set up; says
/// when it replaces another Hollow's record.
pub(crate) fn record_owner_card(_cx: &Cx, _a: &Args) -> Result<Option<Card>, String> {
    let host = crate::remote_session::configured_runner()
        .ok_or("No remote runner is set up on this Mac. Nothing was sent.")?;
    let mut card = Card::new("Hollow app tools: record this Hollow as the runner's owner")
        .line(format!("Write ~/hollow-remote/owner.json on {host}: this Hollow's install id and the fingerprint of the SSH key this Mac uses there. Then check the runner again."));
    let last = crate::remote_status::status(false);
    if last.owner.as_ref().is_some_and(|o| o.state == "other") {
        card = card.line(
            "The last check found another Hollow recorded as its owner: this replaces that record.",
        );
    }
    Ok(Some(
        card.to(format!("ssh {host} (~/hollow-remote/owner.json)")),
    ))
}

/// The setup fields with their defaults, refused while any is missing or
/// not valid (Check, Apply and Create refuse then too).
fn setup_fields(a: &Args) -> Result<(runner_setup::Ctx, SetupParams), String> {
    let params: SetupParams =
        serde_json::from_value(a.0.get("params").cloned().unwrap_or(Value::Null))
            .map_err(|e| format!("`params` doesn't fit: {e}. Nothing ran."))?;
    let ctx = runner_setup::Ctx::real()?;
    let p = runner_setup::filled(&ctx, &params);
    let problems = runner_setup::problems(&p);
    if !problems.is_empty() {
        return Err(format!("{} Nothing ran.", problems.join(" ")));
    }
    Ok((ctx, p))
}

/// Where a setup path's commands go.
fn setup_destination(p: &SetupParams) -> String {
    if p.path == "cloud" {
        format!(
            "Google Cloud project {} (gcloud on this Mac) and ssh {}",
            p.project, p.alias
        )
    } else {
        format!(
            "ssh {} (admin login, setup only) and ssh {} (runner account)",
            p.admin, p.alias
        )
    }
}

/// `build.runner_setup_check`.
pub(crate) fn setup_check_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (_, p) = setup_fields(a)?;
    let what = if p.path == "cloud" {
        "the Google Cloud path"
    } else {
        "the Mac path"
    };
    Ok(Some(
        Card::new("Hollow app tools: check the runner setup")
            .line(format!("Run every Set up step's read-only check commands for {what} (ssh with BatchMode, gcloud reads). Nothing changes."))
            .to(setup_destination(&p)),
    ))
}

/// `build.runner_setup_apply`: the step, its confirm text, what it changes
/// and every command it runs, exactly as shown.
pub(crate) fn setup_apply_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (ctx, p) = setup_fields(a)?;
    let id = arg(a, "step");
    let steps = runner_setup::plan_steps(&ctx, &p);
    let step = steps
        .iter()
        .find(|s| s.id == id)
        .ok_or_else(|| format!("There is no step {id} on this path. Nothing ran."))?;
    if step.id == "cloud" {
        return Err("The machine is created with build.runner_cloud_create, after its monthly estimate. Nothing ran.".into());
    }
    if step.apply.is_empty() {
        return Err(format!("{} has nothing to apply. Nothing ran.", step.title));
    }
    let mut card = Card::new("Hollow app tools: apply a runner setup step")
        .line(format!("Step: {}.", step.title));
    if let Some(confirm) = &step.confirm {
        card = card.line(cut(confirm, 400));
    }
    if !step.changes.is_empty() {
        card = card.line(format!("Changes: {}.", step.changes.join(", ")));
    }
    for c in step.apply.iter().take(12) {
        card = card.line(format!("{}: {}", c.who, cut(&c.shown, 300)));
    }
    if step.apply.len() > 12 {
        card = card.line(format!("and {} more command(s).", step.apply.len() - 12));
    }
    card = card.line("Then its check runs. Private keys stay on disk; none comes back.");
    Ok(Some(card.to(setup_destination(&p))))
}

/// `build.runner_cloud_create`: refused unless `estimateCents` is this
/// machine's monthly estimate now; the card names Google Cloud, the
/// billing account and what it costs.
pub(crate) fn cloud_create_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (_, p) = setup_fields(a)?;
    if p.path != "cloud" {
        return Err("Create is for the Google Cloud path (params.path cloud). Nothing ran.".into());
    }
    let est = runner_setup::estimate(&p).map_err(|e| format!("{e} Nothing ran."))?;
    let shown =
        a.0.get("estimateCents")
            .and_then(Value::as_u64)
            .unwrap_or(0);
    let usd = |cents: u64| format!("${}.{:02}", cents / 100, cents % 100);
    if shown != est.total_cents {
        return Err(format!(
            "estimateCents ({}) isn't this machine's monthly estimate ({}). Read build.runner_cloud_estimate, then try again. Nothing ran.",
            usd(shown),
            usd(est.total_cents)
        ));
    }
    let mut card = Card::new("Hollow app tools: create a billed Google Cloud runner")
        .line(format!(
            "Create the Google Cloud machine {} ({}) in {}, project {}: no public address, reached through IAP only, a service account with no roles, Cloud NAT, a firewall rule and an idle power-off timer.",
            p.instance, est.machine_label, p.zone, p.project
        ))
        .line(format!(
            "This costs money: about {} a month at {} hours (compute ${:.2}, disk ${:.2} for {} GB, NAT ${:.2}), approximate {} list prices from {}.",
            usd(est.total_cents),
            est.hours,
            est.compute_usd,
            est.disk_usd,
            est.disk_gb,
            est.nat_usd,
            est.prices_region,
            est.prices_taken
        ))
        .line(format!(
            "A budget alert of ${} a month on billing account {}. The disk and NAT address keep billing while the machine is off, until it is deleted.",
            p.budget_usd, p.billing_account
        ))
        .to(format!(
            "Google Cloud project {} (billed to billing account {})",
            p.project, p.billing_account
        ));
    card.bind = Some(json!({ "estimateCents": est.total_cents }));
    Ok(Some(card))
}

fn cloud_label(access: &RunnerAccess) -> String {
    match access.cloud {
        CloudAccess::Off => "off".into(),
        CloudAccess::Limited => "on as the person's account, production changes refused".into(),
        CloudAccess::Full => "on as the person's account, production changes included".into(),
        CloudAccess::ServiceAccount => format!(
            "as the service account {}",
            access.service_account.as_deref().unwrap_or("(none named)")
        ),
    }
}

/// `build.set_runner_access`: old → new for GitHub and Google Cloud,
/// refused for what isn't a workspace (or is the COO's or Chats).
pub(crate) fn runner_access_card(cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let id = arg(a, "id");
    let vault = cx.vault();
    if crate::remote_folder::never_moves(&id) {
        return Err("The COO and Chats always run on this Mac, so they have no runner access. Nothing changed.".into());
    }
    if id.is_empty()
        || id.contains('/')
        || id.starts_with('.')
        || !vault.join("workspaces").join(&id).is_dir()
    {
        return Err(format!("{id} is not a workspace. Nothing changed."));
    }
    let new: RunnerAccess = serde_json::from_value(Value::Object(a.0.clone()))
        .map_err(|e| format!("the arguments don't fit: {e}. Nothing changed."))?;
    let old = runner_access::access_for(&vault, &id);
    let on = |b: bool| if b { "on" } else { "off" };
    let mut card = Card::new("Hollow app tools: runner access")
        .line(format!(
            "What workspace {id}'s turns on the remote runner may reach besides their checkout."
        ))
        .line(format!(
            "GitHub (push, pull requests, merges): {} → {}.",
            on(old.github),
            on(new.github)
        ))
        .line(format!(
            "Google Cloud: {} → {}.",
            cloud_label(&old),
            cloud_label(&new)
        ));
    if new.github && !old.github {
        card = card.line(format!(
            "GitHub's credential is {}, handed to those turns one request at a time.",
            source_label(runner_access::github_source())
        ));
    }
    if new.cloud != CloudAccess::Off && new.cloud != old.cloud {
        card = card
            .line("Google Cloud tokens come from this Mac's gcloud login, one request at a time.");
    }
    Ok(Some(card))
}
