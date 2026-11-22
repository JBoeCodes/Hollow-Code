//! Explicit local batches. The normal chat queue still owns admission and holds.
use crate::{app_actions::Caller, app_sessions, config::Config, session_store};
use serde_json::{json, Value};
use std::{collections::VecDeque, path::Path};

pub(crate) const TOOL: &str = "start_workers";
pub(crate) const MAX_WORKERS: usize = 25;

struct Worker {
    title: String,
    prompt: String,
    request: crate::delegation::Request,
}
struct Plan {
    agent: String,
    model: Option<String>,
    workers: Vec<Worker>,
}
// One batch per caller run. Keep its results so retries report the original
// chats instead of silently creating another set. The lock is held only for
// this bookkeeping, never across an enqueue, so no turn waits on another's
// batch (#1068 review: a stopped turn could start workers after that wait).
struct Batch {
    owner: String,
    fingerprint: String,
    starting: bool,
    results: Vec<Value>,
}
static BATCHES: tokio::sync::Mutex<VecDeque<Batch>> =
    tokio::sync::Mutex::const_new(VecDeque::new());

fn plan(
    vault: &Path,
    caller: &Caller,
    workspace: &str,
    count: usize,
    tasks: &Value,
) -> Result<Plan, String> {
    if caller.surface != "chat"
        || app_sessions::effective_depth(vault, caller) != 0
        || caller.access != crate::app_mcp::AppAccess::Write
        || crate::main_agent::unattended::is_unattended_run(vault, &caller.run_id)
    {
        return Err(
            "Only an attended main-agent conversation can create a worker batch. Nothing started."
                .into(),
        );
    }
    let parent = session_store::read_session(vault, &caller.scope)?.summary;
    let global = crate::workspaces::main_agent_cwd(vault).is_some_and(|cwd| cwd == parent.cwd);
    if !global {
        // L8: the lead's fixed conversation or a successor of its handovers.
        let Some(lead) = crate::chat_chain::lead_root(vault, &caller.scope) else {
            return Err("Open the project's lead to create workers. Nothing started.".into());
        };
        let all = crate::workspaces::list(vault)?;
        let project = crate::project_agents::project_of(vault, &all, &lead)
            .ok_or("The lead's project is unavailable. Nothing started.")?;
        crate::project_agents::validate(vault, Path::new(&project.cwd), &lead)?;
    }
    let list = tasks
        .as_array()
        .ok_or("tasks must be a list. Nothing started.")?;
    if count == 0 || count > MAX_WORKERS || count != list.len() {
        return Err(format!("Provide exactly count tasks (1–{MAX_WORKERS}), one per requested worker. Nothing started."));
    }
    app_sessions::resolve_workspace(vault, workspace)?;
    let mut workers = Vec::new();
    let mut titles = std::collections::HashSet::new();
    for (i, task) in list.iter().enumerate() {
        let field = |name: &str| task.get(name).and_then(Value::as_str).unwrap_or("").trim();
        let title = field("title");
        let prompt = field("prompt");
        if title.is_empty()
            || title.chars().count() > 120
            || title.contains(['\n', '\r'])
            || prompt.is_empty()
            || prompt.len() > 32_000
            || !titles.insert(title.to_lowercase())
        {
            return Err(format!("Task {} needs a distinct one-line title (1–120 characters) and prompt (1–32000 bytes). Nothing started.", i + 1));
        }
        workers.push(Worker {
            title: title.into(),
            prompt: prompt.into(),
            request: crate::delegation::parse(task.get("packet"))?,
        });
    }
    Ok(Plan {
        agent: parent.agent,
        model: session_store::chat_run_model(vault, &caller.run_id)?,
        workers,
    })
}

pub(crate) async fn start(
    app: Option<&tauri::AppHandle>,
    config: &Config,
    caller: &Caller,
    workspace: &str,
    count: usize,
    tasks: &Value,
) -> Result<Value, String> {
    // The person's swarm limit (Settings, Background work, COO, Largest swarm (chats)) bounds a
    // worker batch too, so one setting caps every fan-out (#1068 review).
    let limit = (crate::main_agent::saved_swarm_max()? as usize).min(MAX_WORKERS);
    // The lease outlives a stopped turn's child sweep, so the turn's own stop
    // slot decides too (Stop, or no slot: it is finishing).
    let live = || {
        crate::automated_history::run_is_live(&config.vault_path, &caller.run_id)
            && !crate::chat::stop_requested_for(&caller.scope)
    };
    start_with(&config.vault_path, caller, workspace, count, tasks, limit, live, |task, agent, model| async move {
        let target = app_sessions::resolve_workspace(&config.vault_path, workspace)?;
        let text = format!("{}\n\n{}\n\nComplete only this assigned task. Report what changed, verification and any blockers to the project's lead.", task.title, task.prompt);
        // X1: a worker's tracker issue (its own title; never refused: a
        // worker names no issue).
        let facts = app_sessions::work_facts(config, caller, &target, TOOL, &task.prompt, Some(&task.title), &task.request, None, (&Some(agent.clone()), &model, &None));
        // X2: a refused link (recorded) still starts an audited, untied run.
        let work = Some(crate::tracker::work::prepare(config, facts.clone()).unwrap_or(crate::tracker::work::Prepared { facts, resolution: crate::tracker::work::Resolution::None, track: false }));
        let (prompt, packet, rendered) = app_sessions::child_prompt(&config.vault_path, caller, task.request, &text);
        app_sessions::queue_turn(
            app, config, caller, target, prompt, None, Some(agent), model, None,
            app_sessions::mode_for(caller, None),
            app_sessions::ChildRecord { tool: TOOL, packet, rendered, work, placement: None, mode_cap: None, mode_lift: None },
            app_sessions::effective_depth(&config.vault_path, caller) + 1, Vec::new(),
        ).await
    }).await
}

async fn start_with<F, Fut>(
    vault: &Path,
    caller: &Caller,
    workspace: &str,
    count: usize,
    tasks: &Value,
    limit: usize,
    live: impl Fn() -> bool,
    mut queue: F,
) -> Result<Value, String>
where
    F: FnMut(Worker, String, Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    // Validate the whole batch before the first enqueue.
    let planned = plan(vault, caller, workspace, count, tasks)?;
    if count > limit {
        return Err(format!("A worker batch runs 1 to {limit} chats (Settings → Background work → COO → Largest swarm (chats)). Nothing started."));
    }
    let fingerprint =
        crate::run_manifest::sha256_hex(json!([workspace, count, tasks]).to_string().as_bytes());
    let owner = format!("{}:{}", vault.display(), caller.run_id);
    {
        let mut batches = BATCHES.lock().await;
        if let Some(b) = batches.iter().find(|b| b.owner == owner) {
            if b.fingerprint != fingerprint {
                return Err("This turn already requested a different worker batch. Read its worker chats; request additional work in a new turn. Nothing started.".into());
            }
            if b.starting {
                return Err("This turn's worker batch is still starting. Wait for its result; nothing else was started.".into());
            }
            return Ok(report(count, &b.results));
        }
        if !live() {
            return Err("The asking turn has ended. Nothing started.".into());
        }
        let recorded = crate::delegation::for_run(vault, &caller.run_id);
        if recorded.iter().any(|r| r.tool == TOOL) {
            return Err("This run already created workers. Read its child chats with list_sessions {mine:true}; nothing else was started.".into());
        }
        // Every worker needs its parent record: Stop, completion notices and
        // cost totals follow it, and a run keeps only so many.
        let room = crate::delegation::MAX_RECORDED_PER_RUN.saturating_sub(recorded.len());
        if count > room {
            return Err(format!("This turn has room to track {room} more chats, not {count}. Start the workers in a new turn. Nothing started."));
        }
        if batches.len() >= 64 {
            batches.pop_front();
        }
        batches.push_back(Batch {
            owner: owner.clone(),
            fingerprint,
            starting: true,
            results: Vec::new(),
        });
    }
    let mut results = Vec::new();
    for task in planned.workers {
        let title = task.title.clone();
        // Rechecked per worker: a stop, or another delegation of this run
        // taking the last record, ends the batch here.
        let full = crate::delegation::for_run(vault, &caller.run_id).len()
            >= crate::delegation::MAX_RECORDED_PER_RUN;
        let queued = if !live() {
            Err("The asking turn ended before this worker was queued. Not started.".into())
        } else if full {
            Err("This turn can't track another chat. Not started.".into())
        } else {
            queue(task, planned.agent.clone(), planned.model.clone()).await
        };
        let result = match queued {
            Ok(mut row) => {
                if let Some(key) = row["sessionKey"].as_str() {
                    if let Err(error) = session_store::remember_queued_worker(vault, key, &title) {
                        row["visibilityWarning"] = json!(error);
                    }
                }
                row["title"] = json!(title);
                row
            }
            Err(error) => json!({"title": title, "error": error}),
        };
        results.push(result);
    }
    let mut batches = BATCHES.lock().await;
    if let Some(b) = batches.iter_mut().find(|b| b.owner == owner) {
        b.results = results.clone();
        b.starting = false;
    }
    Ok(report(count, &results))
}

fn report(count: usize, results: &[Value]) -> Value {
    let queued = results
        .iter()
        .filter(|r| r["sessionKey"].is_string())
        .count();
    json!({
        "requested": count, "queued": queued, "failed": count - queued, "workers": results,
        "note": "Local Hollow worker chats; queued does not mean running. Existing chat, build and resource limits still apply. Report each returned chat and any failures, then end your turn. Read results with read_session on follow-up; do not start replacements for a repeated call.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{agent::AgentKind, app_mcp::AppAccess};

    fn fixture() -> (tempfile::TempDir, Caller, String) {
        let dir = tempfile::tempdir().unwrap();
        let project = crate::workspaces::create(dir.path(), "Portal", "Build Portal").unwrap();
        let caller = Caller {
            run_id: "r-worker-batch".into(),
            surface: "chat".into(),
            scope: crate::project_agents::key(&project.id).unwrap(),
            access: AppAccess::Write,
            depth: 0,
            perm: "acceptEdits".into(),
        };
        session_store::start_turn(
            dir.path(),
            &caller.scope,
            "codex",
            &project.cwd,
            &caller.run_id,
            "Do five Portal tasks",
        )
        .unwrap();
        (dir, caller, project.id)
    }
    fn tasks() -> Value {
        json!((1..=5).map(|i| json!({"title": format!("Portal task {i}"), "prompt": format!("Complete task {i}; report verification.")})).collect::<Vec<_>>())
    }

    #[tokio::test]
    async fn five_workers_are_separate_local_queue_rows_and_retry_creates_none() {
        let (dir, caller, project) = fixture();
        let vault = dir.path();
        let cwd = Path::new(
            &crate::workspaces::read(vault, &project)
                .unwrap()
                .workspace
                .cwd,
        )
        .to_path_buf();
        let tasks = tasks();
        let out = start_with(
            vault,
            &caller,
            &project,
            5,
            &tasks,
            MAX_WORKERS,
            || true,
            |worker, agent, model| {
                let cwd = cwd.clone();
                async move {
                    let accepted = crate::chat::accept_turn(
                        vault,
                        crate::chat::ChatSendRequest {
                            prompt: format!("{}\n{}", worker.title, worker.prompt),
                            session_id: None,
                            session_key: None,
                            cwd,
                            model,
                            permission_mode: Some("acceptEdits".into()),
                            agent: AgentKind::from_opt(&agent).unwrap(),
                            client_turn_id: None,
                            app_depth: 1,
                            ultracode: false,
                            canvases: None,
                            continued_from: vec![],
                            unattended: false,
                            typed: false,
                            host_asker: None,
                            canned: None,
                            worker: false,
                        },
                        chrono::Utc::now(),
                    )?;
                    Ok(json!({"sessionKey": accepted.session_key, "queueId": accepted.queue_id}))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(out["queued"], 5);
        assert_eq!(out["failed"], 0);
        let groups = crate::workspaces::sessions_by_workspace(vault).unwrap();
        let visible = &groups
            .iter()
            .find(|g| g.workspace_id == project)
            .unwrap()
            .sessions;
        assert_eq!(
            visible.len(),
            6,
            "the COO and all five queued workers are visible"
        );
        assert_eq!(visible.iter().filter(|s| s.turn_count == 0).count(), 5);
        assert!(out["workers"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w.get("visibilityWarning").is_none()));
        let rows = crate::run_queue::live_rows(vault).unwrap();
        assert_eq!(rows.len(), 5);
        let keys: std::collections::HashSet<_> = rows.iter().map(|r| &r.source_id).collect();
        assert_eq!(keys.len(), 5);
        for row in rows {
            assert_eq!(row.payload["cwd"], cwd.to_string_lossy().as_ref());
            assert_eq!(row.payload["agent"], "codex");
            assert_eq!(row.payload["permissionMode"], "acceptEdits");
            assert_eq!(row.payload["appDepth"], 1);
        }
        let repeated = start_with(
            vault,
            &caller,
            &project,
            5,
            &tasks,
            MAX_WORKERS,
            || true,
            |_, _, _| async { panic!("retry must not enqueue") },
        )
        .await
        .unwrap();
        assert_eq!(out, repeated);
        let mut changed = tasks.clone();
        changed[0]["prompt"] = json!("Different work");
        assert!(start_with(
            vault,
            &caller,
            &project,
            5,
            &changed,
            MAX_WORKERS,
            || true,
            |_, _, _| async { panic!("changed batch must not enqueue") }
        )
        .await
        .is_err());
        assert_eq!(crate::run_queue::live_rows(vault).unwrap().len(), 5);
    }

    #[test]
    fn count_and_all_tasks_are_checked_before_queueing() {
        let (dir, caller, project) = fixture();
        let vault = dir.path();
        assert!(plan(vault, &caller, &project, 4, &tasks()).is_err());
        assert!(plan(vault, &caller, &project, 0, &json!([])).is_err());
        assert!(plan(vault, &caller, &project, 26, &json!(vec![json!({}); 26])).is_err());
        let mut bad = tasks();
        bad[4]["title"] = bad[0]["title"].clone();
        assert!(plan(vault, &caller, &project, 5, &bad).is_err());
        bad = tasks();
        bad[4]["packet"] = json!(false);
        assert!(plan(vault, &caller, &project, 5, &bad).is_err());
        assert!(plan(vault, &caller, "missing-project", 5, &tasks()).is_err());
        assert!(crate::run_queue::live_rows(vault).unwrap().is_empty());
    }

    #[test]
    fn only_attended_global_and_project_coordinators_can_plan_workers() {
        let (dir, mut caller, project) = fixture();
        let vault = dir.path();
        assert_eq!(
            plan(vault, &caller, &project, 5, &tasks()).unwrap().agent,
            "codex"
        );
        // L1: a lead's turn an agent sent still counts from depth 0; any
        // other chat at depth 1 is refused.
        caller.depth = 1;
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_ok());
        let lead = caller.scope.clone();
        caller.scope = "s-20261110-ordinary".into();
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_err());
        caller.scope = lead;
        caller.depth = 0;
        caller.access = AppAccess::Read;
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_err());
        caller.access = AppAccess::Write;
        let other = crate::workspaces::create(vault, "Other", "Other").unwrap();
        session_store::with_write(vault, "test move", |tx| {
            tx.execute(
                "UPDATE sessions SET cwd = ?1 WHERE key = ?2",
                rusqlite::params![other.cwd, caller.scope],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_err());
        crate::workspaces::ensure_main_agent(vault).unwrap();
        caller.scope = "s-global-main".into();
        session_store::start_turn(
            vault,
            &caller.scope,
            "claude",
            &crate::workspaces::main_agent_cwd(vault).unwrap(),
            "r-global",
            "Do five tasks",
        )
        .unwrap();
        assert_eq!(
            plan(vault, &caller, &project, 5, &tasks()).unwrap().agent,
            "claude"
        );
    }

    #[test]
    fn removed_own_worktree_allows_workers_but_a_reused_path_does_not() {
        let (dir, caller, project) = fixture();
        let vault = dir.path();
        let repo = vault.join("repo");
        let old_tree = vault.join("old-worktree");
        std::fs::create_dir_all(&old_tree).unwrap();
        crate::build_workspaces::bind_repo_for_test(vault, &project, &repo, &old_tree);
        let old_cwd = old_tree
            .canonicalize()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        session_store::with_write(vault, "test previous worktree", |tx| {
            tx.execute(
                "UPDATE sessions SET cwd = ?1 WHERE key = ?2",
                rusqlite::params![old_cwd, caller.scope],
            )
            .map_err(|e| e.to_string())
        })
        .unwrap();
        crate::build_workspaces::mark_removed_for_test(vault, &project);
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_ok());
        assert_eq!(
            session_store::session_by_key(vault, &caller.scope)
                .unwrap()
                .unwrap()
                .cwd,
            old_cwd,
            "worker planning must not rewrite main-agent history"
        );
        let other = crate::workspaces::create(vault, "Reuse", "Other work").unwrap();
        crate::build_workspaces::bind_repo_for_test(vault, &other.id, &repo, &old_tree);
        assert!(plan(vault, &caller, &project, 5, &tasks()).is_err());
        assert!(crate::run_queue::live_rows(vault).unwrap().is_empty());
    }

    /// #1068 review: the swarm limit bounds a batch, a batch that would
    /// overflow the run's delegation records starts nothing, and a turn
    /// stopped while waiting (or mid-batch) queues nothing more.
    #[tokio::test]
    async fn limits_record_room_and_a_stopped_turn_start_nothing_more() {
        async fn never(_: Worker, _: String, _: Option<String>) -> Result<Value, String> {
            panic!("must not enqueue")
        }
        // A run that already recorded 28 delegations has room for 4.
        let (busy_dir, busy, busy_project) = fixture();
        // Now, so the records outlive the delegation log's retention window.
        let at = chrono::Utc::now().to_rfc3339();
        let record = json!({"kind": "consultation", "tool": "ask_agent", "agent": "codex",
            "packetBytes": 0, "packetSha256": "", "constraints": 0, "sources": 0,
            "fromTaskState": 0, "omitted": 0, "at": at});
        for _ in 0..28 {
            let _ = session_store::insert_delegation(
                busy_dir.path(),
                &busy.run_id,
                &at,
                &record.to_string(),
                crate::delegation::MAX_RECORDED_PER_RUN,
            );
        }
        assert_eq!(
            crate::delegation::for_run(busy_dir.path(), &busy.run_id).len(),
            28
        );
        let full = start_with(
            busy_dir.path(),
            &busy,
            &busy_project,
            5,
            &tasks(),
            6,
            || true,
            never,
        )
        .await;
        assert!(full.unwrap_err().contains("room to track 4"));

        let (dir, caller, project) = fixture();
        let vault = dir.path();
        let over = start_with(vault, &caller, &project, 5, &tasks(), 4, || true, never).await;
        assert!(over.unwrap_err().contains("1 to 4"));
        let ended = start_with(vault, &caller, &project, 5, &tasks(), 6, || false, never).await;
        assert!(ended.unwrap_err().contains("ended"));
        // Stopped after the first worker: the rest are reported, not queued.
        let calls = std::cell::Cell::new(0);
        let out = start_with(
            vault,
            &caller,
            &project,
            5,
            &tasks(),
            6,
            || {
                calls.set(calls.get() + 1);
                calls.get() <= 2
            },
            |_, _, _| async { Ok(json!({"sessionKey": "s-one"})) },
        )
        .await
        .unwrap();
        assert_eq!(
            (out["queued"].as_u64(), out["failed"].as_u64()),
            (Some(1), Some(4))
        );
        // A same-turn call while its batch is still starting waits for nothing
        // and starts nothing; no other turn ever waits on this batch.
        let (other_dir, other, other_project) = fixture();
        let fingerprint = crate::run_manifest::sha256_hex(
            json!([other_project, 5, tasks()]).to_string().as_bytes(),
        );
        BATCHES.lock().await.push_back(Batch {
            owner: format!("{}:{}", other_dir.path().display(), other.run_id),
            fingerprint,
            starting: true,
            results: Vec::new(),
        });
        let starting = start_with(
            other_dir.path(),
            &other,
            &other_project,
            5,
            &tasks(),
            6,
            || true,
            never,
        )
        .await;
        assert!(starting.unwrap_err().contains("still starting"));
    }

    #[tokio::test]
    async fn partial_failures_are_reported_without_hiding_accepted_workers() {
        let (dir, caller, project) = fixture();
        let out = start_with(
            dir.path(),
            &caller,
            &project,
            5,
            &tasks(),
            MAX_WORKERS,
            || true,
            |task, _, _| async move {
                if task.title.ends_with('3') {
                    Err("queue unavailable".into())
                } else {
                    Ok(json!({"sessionKey": format!("s-{}",task.title)}))
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(out["queued"], 4);
        assert_eq!(out["failed"], 1);
        assert_eq!(out["workers"][2]["error"], "queue unavailable");
    }
}
