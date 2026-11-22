//! Host integration for the authenticated transport. Config selection happens
//! once before spawn; an uncertain HTTP call is never retried over stdio.

use crate::core_host::Core;
use crate::{app_mcp, shared_mcp, tasks_mcp, usage_ledger};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

struct Host {
    core: Core,
    vault: PathBuf,
    server: shared_mcp::Server,
    registry: Arc<shared_mcp::Registry>,
}

static HOST: OnceLock<Host> = OnceLock::new();
static ATTEMPTS: OnceLock<Mutex<HashMap<String, Weak<Attempt>>>> = OnceLock::new();

pub(crate) fn start(core: Core, vault: PathBuf) {
    tauri::async_runtime::spawn(async move {
        let registry = Arc::new(shared_mcp::Registry::default());
        match shared_mcp::start(registry.clone()).await {
            Ok(server) => {
                let _ = HOST.set(Host {
                    core,
                    vault,
                    server,
                    registry,
                });
            }
            Err(error) => eprintln!("[shared MCP] {error}; retaining stdio"),
        }
    });
}

pub(crate) struct Attempt {
    grants: Vec<shared_mcp::Capability>,
    tag: String,
    run: String,
    active: std::sync::atomic::AtomicBool,
}

impl std::fmt::Debug for Attempt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("McpAttempt")
    }
}

impl Attempt {
    pub(crate) fn revoke(&self) {
        self.active
            .store(false, std::sync::atomic::Ordering::Release);
        for grant in &self.grants {
            grant.revoke();
        }
    }
    pub(crate) fn activate(&self) {
        for grant in &self.grants {
            grant.activate();
        }
        self.active
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

impl Drop for Attempt {
    fn drop(&mut self) {
        self.revoke();
        crate::run_auth::remove_config(&self.tag);
    }
}

pub(crate) fn active_tag(run: &str) -> Option<String> {
    let attempts = ATTEMPTS.get()?.lock().ok()?;
    let mut found = attempts
        .values()
        .filter_map(Weak::upgrade)
        .filter(|a| a.run == run && a.active.load(std::sync::atomic::Ordering::Acquire));
    let tag = found.next()?.tag.clone();
    found.next().is_none().then_some(tag)
}

pub(crate) fn attempt(tag: &str) -> Option<Arc<Attempt>> {
    ATTEMPTS.get()?.lock().ok()?.get(tag)?.upgrade()
}

/// The command itself owns pending grants. A failed spawn or a command that
/// is never spawned drops them; adoption adds a Binding owner and activates.
#[cfg(unix)]
pub(crate) fn retain(cmd: &mut tokio::process::Command, owner: Arc<Attempt>) {
    // SAFETY: this callback only retains parent-side state and returns Ok.
    // No allocation, lock, or reference counting occurs in the forked child.
    unsafe {
        cmd.pre_exec(move || {
            let _ = &owner;
            Ok(())
        });
    }
}

/// Only the local, supported owned-MCP adapters call this function. Missing
/// host/context leaves the original selected entry in place before dispatch.
pub(crate) fn config(config: &str, tag: &str) -> (String, Option<Arc<Attempt>>) {
    let Some(host) = HOST.get() else {
        return (config.into(), None);
    };
    config_at(host, config, tag)
}

fn config_at(host: &Host, config: &str, tag: &str) -> (String, Option<Arc<Attempt>>) {
    let Ok(mut value) = serde_json::from_str::<Value>(config) else {
        return (config.into(), None);
    };
    let Some(servers) = value.get_mut("mcpServers").and_then(Value::as_object_mut) else {
        return (config.into(), None);
    };
    let ids: Vec<String> =
        serde_json::from_str(&crate::run_auth::bound_runs_value(config)).unwrap_or_default();
    if ids.len() != 1 {
        return (config.into(), None);
    }
    let run = ids[0].clone();
    let common: BTreeMap<String, String> = [
        usage_ledger::ENV_RUN_ID,
        usage_ledger::ENV_RUN_SURFACE,
        usage_ledger::ENV_RUN_SCOPE,
        usage_ledger::ENV_VAULT_ROOT,
    ]
    .into_iter()
    .filter_map(|key| {
        servers
            .iter()
            .filter(|(name, _)| name.starts_with("hollow_"))
            .find_map(|(_, server)| server.get("env")?.get(key)?.as_str())
            .map(|v| (key.into(), v.into()))
    })
    .collect();
    let mut capabilities = Vec::new();
    for (name, entry) in servers.iter_mut() {
        let mut env: BTreeMap<String, String> = entry
            .get("env")
            .and_then(Value::as_object)
            .map(|env| {
                env.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
        for (key, value) in &common {
            env.entry(key.clone()).or_insert_with(|| value.clone());
        }
        let Some(handler) = handler(host, name, &env, entry) else {
            continue;
        };
        let vault = host.vault.clone();
        let live = Arc::new(move |run: &str| crate::automated_history::run_is_live(&vault, run));
        let timeout =
            crate::agent::codex_tool_timeout_secs(name).unwrap_or(if name == "hollow_computer" {
                180
            } else if name.starts_with("hollow_") {
                60
            } else {
                480
            });
        let Some(grant) =
            host.registry
                .register(name, &run, live, Duration::from_secs(timeout), handler)
        else {
            continue;
        };
        // Bound-run ids and the private file's tag are retained explicitly
        // by build_scoped. HTTP entries contain only HTTP schema fields.
        *entry = json!({"type":"http", "url":format!("http://127.0.0.1:{}/mcp/{name}", host.server.port), "headers":{"Authorization":grant.header()}});
        capabilities.push(grant);
    }
    if capabilities.is_empty() {
        return (config.into(), None);
    }
    let attempt = Arc::new(Attempt {
        grants: capabilities,
        tag: tag.into(),
        run,
        active: std::sync::atomic::AtomicBool::new(false),
    });
    let mut attempts = ATTEMPTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    attempts.retain(|_, owner| owner.strong_count() > 0);
    attempts.insert(tag.into(), Arc::downgrade(&attempt));
    (value.to_string(), Some(attempt))
}

fn get(env: &BTreeMap<String, String>, key: &str) -> Option<String> {
    env.get(key).cloned().filter(|s| !s.is_empty())
}

fn handler(
    host: &Host,
    name: &str,
    env: &BTreeMap<String, String>,
    entry: &Value,
) -> Option<shared_mcp::Handler> {
    if !matches!(
        name,
        "hollow_app"
            | "hollow_tasks"
            | "hollow_vault"
            | "hollow_code_context"
            | "hollow_questions"
            | "hollow_computer"
            | "hollow_team"
            | "hollow_services"
    ) {
        return crate::remote_mcp::shared_handler(entry);
    }
    let core = host.core.clone();
    let vault = host.vault.clone();
    let env = env.clone();
    let name = name.to_string();
    let tag = get(&env, crate::process_group::RUN_TAG_ENV)?;
    // All selected entries are host-minted. Refuse incomplete authority at
    // conversion instead of borrowing values from another attempt.
    let session = get(&env, usage_ledger::ENV_RUN_SCOPE).or_else(|| get(&env, "HOLLOW_VAULT"));
    if matches!(name.as_str(), "hollow_team" | "hollow_computer") && session.is_none() {
        return None;
    }
    if name == "hollow_team"
        && [
            "HOLLOW_PRIMARY_AGENT",
            "HOLLOW_VAULT",
            crate::team_mcp::ENV_PARENT_PERM,
        ]
        .iter()
        .any(|key| get(&env, key).is_none())
    {
        return None;
    }
    Some(Arc::new(move |req, permit| {
        let (core, vault, env, name, session, tag) = (
            core.clone(),
            vault.clone(),
            env.clone(),
            name.clone(),
            session.clone().unwrap_or_default(),
            tag.clone(),
        );
        Box::pin(async move {
            if name == "hollow_team" {
                return crate::team_mcp::handle_in(&req, crate::team_mcp::Context { env, permit })
                    .await;
            }
            let approval = shared_mcp::Approval {
                core: core.clone(),
                session,
                run: tag,
                permit: permit.clone(),
            };
            if name == "hollow_computer" {
                let ctx = crate::computer_mcp::Context {
                    bypass: get(&env, crate::computer_mcp::ENV_BYPASS).as_deref() == Some("1"),
                    audit_root: get(&env, "HOLLOW_VAULT").map(PathBuf::from),
                    approval: Some(approval),
                };
                return crate::computer_mcp::handle_in(&req, &ctx).await;
            }
            tokio::task::spawn_blocking(move || {
                if !permit.valid() {
                    return None;
                }
                match name.as_str() {
                    "hollow_questions" => crate::question_mcp::handle(&req),
                    "hollow_vault" => crate::vault_mcp::handle(
                        &req,
                        &crate::vault_mcp::Target::from_env(|key| get(&env, key)),
                    ),
                    // CC2: parses and queries the checkout's index on this
                    // blocking worker.
                    "hollow_code_context" => crate::code_context_mcp::handle(
                        &req,
                        &crate::code_context::Target::from_env(|key| get(&env, key)),
                    ),
                    "hollow_app" => {
                        let identity = app_identity(&env)?;
                        let access = identity.access;
                        let depth = get(&env, app_mcp::ENV_LIST_DEPTH)
                            .and_then(|s| s.parse::<u32>().ok())
                            .unwrap_or(identity.depth)
                            .min(identity.depth);
                        let broker = App {
                            core,
                            vault,
                            identity,
                            approval,
                            note: Mutex::new(None),
                        };
                        app_mcp::handle(
                            &req,
                            &app_mcp::Ctx {
                                access,
                                depth,
                                broker: Some(&broker),
                            },
                        )
                    }
                    "hollow_tasks" => {
                        let access = get(&env, tasks_mcp::ENV_ACCESS)
                            .and_then(|s| tasks_mcp::TasksAccess::parse(&s))?;
                        let identity = tasks_mcp::RunIdentity {
                            run_id: get(&env, usage_ledger::ENV_RUN_ID)?,
                            surface: get(&env, usage_ledger::ENV_RUN_SURFACE)?,
                            scope: get(&env, usage_ledger::ENV_RUN_SCOPE)?,
                            access,
                            proof: get(&env, tasks_mcp::ENV_PROOF)?,
                        };
                        let broker = Tasks {
                            core,
                            vault: vault.clone(),
                            identity,
                            permit,
                        };
                        tasks_mcp::handle(
                            &req,
                            &tasks_mcp::Ctx {
                                vault_root: Some(vault),
                                access,
                                client: Some(&broker),
                            },
                        )
                    }
                    "hollow_services" => {
                        let broker = Services { vault, env, permit };
                        crate::services_mcp::handle(&req, Some(&broker))
                    }
                    _ => None,
                }
            })
            .await
            .ok()
            .flatten()
        })
    }))
}

fn app_identity(env: &BTreeMap<String, String>) -> Option<app_mcp::RunIdentity> {
    Some(app_mcp::RunIdentity {
        run_id: get(env, usage_ledger::ENV_RUN_ID)?,
        surface: get(env, usage_ledger::ENV_RUN_SURFACE)?,
        scope: get(env, usage_ledger::ENV_RUN_SCOPE)?,
        access: get(env, app_mcp::ENV_ACCESS).and_then(|s| app_mcp::AppAccess::parse(&s))?,
        depth: get(env, app_mcp::ENV_DEPTH)
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
        perm: get(env, app_mcp::ENV_PERM).unwrap_or_default(),
        proof: get(env, app_mcp::ENV_PROOF)?,
    })
}

struct App {
    core: Core,
    vault: PathBuf,
    identity: app_mcp::RunIdentity,
    approval: shared_mcp::Approval,
    note: Mutex<Option<String>>,
}

impl app_mcp::Broker for App {
    fn call(&self, action: &str, args: &Value, via: app_mcp::Via<'_>) -> Result<Value, String> {
        if !self.approval.valid() {
            return Err(crate::app_broker::RUN_ENDED.into());
        }
        let req = serde_json::from_value(self.identity.request(action, args, via))
            .map_err(|e| format!("Invalid app identity: {e}"))?;
        let answer =
            crate::app_broker::dispatch_http(&self.core, &self.vault, &req, &self.approval);
        let (result, note) = app_mcp::parse_app_answer(&answer.to_string())?;
        if note.is_some() {
            *self.note.lock().unwrap_or_else(|p| p.into_inner()) = note;
        }
        Ok(result)
    }
    fn take_note(&self) -> Option<String> {
        self.note.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}

struct Tasks {
    core: Core,
    vault: PathBuf,
    identity: tasks_mcp::RunIdentity,
    permit: shared_mcp::Permit,
}

impl tasks_mcp::BrokerClient for Tasks {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
        let req = serde_json::from_value(self.identity.request(op, args))
            .map_err(|e| format!("Invalid task identity: {e}"))?;
        if !self.permit.valid() {
            return Err(crate::app_broker::RUN_ENDED.into());
        }
        let done = crate::task_broker::dispatch_at_checked(
            &self.vault,
            &req,
            tasks_mcp::enabled(),
            chrono::Local::now(),
            &|| self.permit.current(),
        )?;
        if done.review_changed > 0 {
            let _ = self
                .core
                .emit(crate::task_broker::REVIEW_EVENT, done.review_changed);
        }
        Ok(done.result)
    }
}

struct Services {
    vault: PathBuf,
    env: BTreeMap<String, String>,
    permit: shared_mcp::Permit,
}

impl crate::services_mcp::Broker for Services {
    fn call(&self, op: &str, args: &Value) -> Result<Value, String> {
        use crate::services_mcp::*;
        let id = Identity {
            scope: get(&self.env, ENV_SCOPE).ok_or("Missing service scope")?,
            run_id: get(&self.env, ENV_RUN).ok_or("Missing service run")?,
            chat: get(&self.env, ENV_CHAT).ok_or("Missing service chat")?,
            perm: get(&self.env, ENV_PERM).ok_or("Missing service mode")?,
        };
        let proof = get(&self.env, ENV_PROOF).ok_or("Missing service proof")?;
        let req = serde_json::from_value(request(op, &id, &proof, args))
            .map_err(|e| format!("Invalid service identity: {e}"))?;
        tauri::async_runtime::block_on(crate::services::dispatch(
            &crate::services::registry(),
            &req,
            |scope| {
                crate::build_workspaces::workspace_for_worktree(
                    &self.vault,
                    std::path::Path::new(scope),
                )
                .is_some()
            },
            |run| self.permit.valid() && crate::automated_history::run_is_live(&self.vault, run),
            std::env::vars_os(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn command_ownership_adoption_and_revoke_fence_the_selected_entry() {
        let dir = tempfile::tempdir().unwrap();
        let run = crate::automated_history::try_begin_run(dir.path(), "chat", Some("test-shared"))
            .unwrap();
        let registry = Arc::new(shared_mcp::Registry::default());
        let server = shared_mcp::start(registry.clone()).await.unwrap();
        let core = Core::new(crate::core_host::RecordingHost::with(None, None));
        let host = Host {
            core,
            vault: dir.path().into(),
            server,
            registry,
        };
        let tag = crate::process_group::RunTag::new();
        let config = json!({"mcpServers":{"hollow_questions":{"command":"selected-stdio","args":["questions-mcp"],"env":{
            usage_ledger::ENV_RUN_ID:run.id, usage_ledger::ENV_RUN_SCOPE:"test-shared", usage_ledger::ENV_RUN_SURFACE:"chat", crate::process_group::RUN_TAG_ENV:tag.as_str()
        }}}}).to_string();
        let (http, owner) = config_at(&host, &config, tag.as_str());
        let http: Value = serde_json::from_str(&http).unwrap();
        let entry = &http["mcpServers"]["hollow_questions"];
        assert!(entry.get("command").is_none());
        let client = reqwest::Client::new();
        let ping = || {
            client
                .post(entry["url"].as_str().unwrap())
                .header(
                    "Authorization",
                    entry["headers"]["Authorization"].as_str().unwrap(),
                )
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"ping"}))
        };
        assert_eq!(ping().send().await.unwrap().status(), 401);
        let mut cmd = tokio::process::Command::new("/bin/sh");
        cmd.env(
            crate::run_auth::BOUND_RUNS_ENV,
            crate::run_auth::bound_runs_value(&config),
        );
        retain(&mut cmd, owner.unwrap());
        let binding = crate::run_auth::adopt_into(
            crate::run_auth::Peers::leaked(),
            cmd.as_std(),
            tag.as_str(),
            std::process::id() as i32,
        )
        .unwrap();
        assert_eq!(active_tag(&run.id).as_deref(), Some(tag.as_str()));
        assert_eq!(ping().send().await.unwrap().status(), 200);
        binding.revoke();
        assert!(active_tag(&run.id).is_none());
        assert_eq!(ping().send().await.unwrap().status(), 401);
        drop(binding);
        drop(cmd);
        assert!(attempt(tag.as_str()).is_none());

        let tag = crate::process_group::RunTag::new();
        let (_, pending) = config_at(&host, &config, tag.as_str());
        let mut abandoned = tokio::process::Command::new("/missing-executable");
        retain(&mut abandoned, pending.unwrap());
        assert!(abandoned.spawn().is_err());
        drop(abandoned);
        assert!(attempt(tag.as_str()).is_none());
    }
}
