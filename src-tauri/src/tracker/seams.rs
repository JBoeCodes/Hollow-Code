//! Local task and phone writers enter the same durable tracker editor.
//! Take sync exclusion before the caller's History lease. Nothing here
//! makes a provider request or loads a credential.

use super::*;
use crate::vault_tasks;

type RunningPolicy = Result<(String, String), String>;
static RUNNING: Lazy<std::sync::Mutex<BTreeMap<std::path::PathBuf, RunningPolicy>>> =
    Lazy::new(|| std::sync::Mutex::new(BTreeMap::new()));

/// Setup registers the immutable config used by both background writers.
/// Saving a config file cannot replace this process's policy.
pub(crate) fn register_running(config: &Config) {
    let policy = enabled(config).map(|()| {
        (
            config.tracker.provider.clone().unwrap_or_default(),
            config.tracker.team_id.clone().unwrap_or_default(),
        )
    });
    RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(config.vault_path.clone())
        .or_insert(policy);
}

pub(crate) fn hold_running_edit_lock(root: &Path, wait: Duration) -> Result<EditGuard, String> {
    let (provider, scope) = RUNNING
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(root)
        .cloned()
        .ok_or(
            "The running task tracker configuration is unavailable. Restart Hollow before editing.",
        )??;
    let guard = hold_edit_lock(root, wait)?;
    let state = cached_at(root)?;
    if state.provider != provider || state.scope != scope {
        return Err(
            "The tracker history differs from the running configuration. No task was changed."
                .into(),
        );
    }
    Ok(guard)
}

pub(crate) fn task_meta(content: &str) -> Option<TrackerTaskMeta> {
    let fm = frontmatter::parse(content);
    let provider = fm
        .get("tracker")
        .filter(|p| !p.is_empty() && *p != "none")?;
    Some(TrackerTaskMeta {
        provider: provider.into(),
        id: fm.get("tracker_id").unwrap_or("").into(),
        key: fm
            .get("tracker_key")
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        url: fm
            .get("tracker_url")
            .filter(|v| !v.is_empty())
            .map(str::to_string),
        status_type: fm.get("status_type").and_then(StateKind::parse),
        priority: (0..=4)
            .find(|n| mirror::priority(*n).eq_ignore_ascii_case(fm.get("priority").unwrap_or("")))
            .unwrap_or(0),
        lock: TrackerLock {
            kind: TrackerLockKind::Unreadable,
            fields: Field::ALL.iter().map(|f| f.key().into()).collect(),
        },
    })
}

pub(crate) fn mark_locks(root: &Path, tasks: &mut [crate::types::Task]) {
    if !tasks.iter().any(|t| t.tracker.is_some()) {
        return;
    }
    let state = cached_at(root);
    for task in tasks {
        let Some(meta) = &mut task.tracker else {
            continue;
        };
        let Ok(state) = &state else {
            continue;
        };
        let rel = task.url.strip_prefix("vault:").unwrap_or(&task.url);
        meta.lock.kind = match state.records.get(&meta.id) {
            Some(record)
                if state.provider == meta.provider
                    && record.kind == "task"
                    && record.path == rel =>
            {
                if record.remote_missing || record.state != "active" {
                    TrackerLockKind::RemoteMissing
                } else {
                    TrackerLockKind::Linked
                }
            }
            _ => TrackerLockKind::Moved,
        };
    }
}

pub(crate) struct EditGuard {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    _file: crate::flock::HeldLock,
}

pub(crate) fn hold_edit_lock(root: &Path, wait: Duration) -> Result<EditGuard, String> {
    let until = std::time::Instant::now() + wait;
    let lock = root_lock(root);
    loop {
        if let Ok(guard) = lock.clone().try_lock_owned() {
            return Ok(EditGuard {
                _guard: guard,
                _file: file_lock(root)?,
            });
        }
        if std::time::Instant::now() >= until {
            return Err(
                "Task tracker sync is busy. Nothing was saved; retry when it finishes.".into(),
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(crate) fn tracked(root: &Path, rel: &str) -> bool {
    read(root, rel).ok().flatten().is_some_and(|text| {
        let fm = frontmatter::parse(&text);
        fm.get("tracker")
            .is_some_and(|p| !p.is_empty() && p != "none")
    })
}

pub(crate) fn cached_at(root: &Path) -> Result<Ledger, String> {
    plain(root, STATE)?;
    let raw = fs::read(root.join(STATE)).map_err(|_| {
        "The tracker history is unavailable. Restore it before editing its mirrors."
    })?;
    let state: Ledger = serde_json::from_slice(&raw)
        .map_err(|_| "The tracker history is unreadable. Restore it before editing its mirrors.")?;
    load(root, &state.provider, &primary_scope(&state))
}

pub(crate) fn detail(root: &Path, rel: &str, id: &str) -> Result<TrackerIssueDetail, String> {
    let state = cached_at(root)?;
    let record = record_at(&state, rel)?;
    if record.kind != "task" || record.id != id {
        return Err(
            "TASK_LINKED: This task's tracker identity does not match its mirror path.".into(),
        );
    }
    api::issue_in_state(root, &state, &record.id)
}

pub(crate) async fn user_edit(
    config: &Config,
    rel: String,
    expected: Option<String>,
    fields: serde_json::Map<String, Value>,
) -> Result<String, String> {
    enabled(config)?;
    let root = config.vault_path.clone();
    let provider = config.tracker.provider.clone();
    let scope = config.tracker.team_id.clone();
    tokio::task::spawn_blocking(move || {
        let _sync = hold_edit_lock(&root, Duration::from_secs(20))?;
        let state = cached_at(&root)?;
        if Some(state.provider.as_str()) != provider.as_deref()
            || Some(state.scope.as_str()) != scope.as_deref()
        {
            return Err(
                "The tracker history differs from the running configuration. No task was changed."
                    .into(),
            );
        }
        let _lease =
            crate::automated_history::tracker_mirror_write_lease(&root, &folders(&state, []))?;
        let result = edit_task(&root, &rel, expected.as_deref().unwrap_or(""), &fields)?;
        result["contentHash"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "The tracker edit has no receipt.".into())
    })
    .await
    .map_err(|_| "The tracker editor stopped before returning its receipt.".to_string())?
}

pub(crate) fn record_at<'a>(state: &'a Ledger, rel: &str) -> Result<&'a Record, String> {
    state.records.values().find(|r| r.path == rel)
        .ok_or_else(|| "TASK_LINKED: This tracker mirror has no baseline at its current path. Sync or restore its history before editing.".into())
}

/// Called under the running tracker lock, before a local ownership write.
pub(crate) fn check_claim(root: &Path, rel: &str) -> Result<(), String> {
    let state = cached_at(root)?;
    let record = record_at(&state, rel)?;
    if record.kind != "task" || record.remote_missing || record.state != "active" {
        return Err("TASK_LINKED: Only an available tracker task can carry a local claim.".into());
    }
    read_mirror(root, &state.provider, &record.id, "task", rel)?
        .ok_or("TASK_LINKED: Restore the tracker task before claiming it.")?;
    Ok(())
}

fn edit_project(
    root: &Path,
    state: &mut Ledger,
    mut record: Record,
    expected_hash: &str,
    fields: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    if record.project.is_none() || record.remote_missing || record.state != "active" {
        return Err("TASK_LINKED: Only an available tracker project can be edited.".into());
    }
    if state.stored_in_hollow {
        return Err(format!("TASK_LINKED: {STORED_IN_HOLLOW}"));
    }
    let old = read_mirror(root, &state.provider, &record.id, "project", &record.path)?
        .ok_or("TASK_LINKED: Restore the tracker project before editing.")?;
    if expected_hash.is_empty() || vault_tasks::content_hash(&old) != expected_hash {
        return Err("TASK_CONFLICT: This project changed after it was read. Read it again and reapply the edit.".into());
    }
    let mut values = Values::new();
    let mut local = Values::new();
    for (key, value) in fields {
        let local_key = match key.as_str() {
            "priority" => Some("priority"),
            "currentState" => Some("current_state"),
            "desiredState" => Some("desired_state"),
            "nonGoals" => Some("non_goals"),
            "successEvidence" => Some("success_evidence"),
            "goalUrls" => Some("goal_urls"),
            _ => None,
        };
        if let Some(key) = local_key {
            let value = if key == "goal_urls" {
                let urls: Vec<String> = serde_json::from_value(value.clone())
                    .map_err(|_| "TASK_INVALID: goalUrls must be a list of URLs.")?;
                json!(crate::vault_projects::normalize_goal_urls(&urls)?.join(", "))
            } else if value.is_null() {
                Value::Null
            } else {
                let text = value
                    .as_str()
                    .ok_or("TASK_INVALID: A project local field must be text.")?;
                if key == "priority" && !text.is_empty() {
                    json!(vault_tasks::normalize_priority(text)
                        .ok_or("TASK_INVALID: Choose P1, P2, P3 or P4.")?)
                } else {
                    json!(text)
                }
            };
            local.insert(key.into(), value);
            continue;
        }
        let key = match key.as_str() {
            "targetDate" => "target_date",
            "startDate" => "start_date",
            "title" | "description" | "status" | "lead" => key,
            _ => return Err("TASK_INVALID: Read the tracker project fields before editing. Unsupported local fields were not changed.".into()),
        };
        values.insert(
            key.into(),
            if value.is_null() {
                json!("")
            } else {
                value.clone()
            },
        );
    }
    let mut writable = values.clone();
    // Current project snapshots have status names without scoped status ids.
    // Keep the requested status in the durable outbox; sync parks it as a conflict.
    writable.remove("status");
    let project_team = record
        .project
        .as_ref()
        .and_then(|p| p.team_ids.iter().find(|t| state.has_team(t)).cloned())
        .unwrap_or_else(|| state.scope.clone());
    tracker::sync_fields::project_patch(
        &writable,
        state
            .meta_for(&project_team)
            .ok_or("Sync the tracker before editing.")?,
    )?;
    if values.get("status").is_some_and(|v| !v.is_string()) {
        return Err("TASK_INVALID: A project status must be a string.".into());
    }
    let before = mirror::local_project_values(&old)?;
    let mut current = before.clone();
    current.extend(values.clone());
    record.conflicts.retain(|c| !values.contains_key(&c.field));
    record.error = None;
    let queued = api::stage_edit(state, &record, before, current.clone());
    api::stage_local(state, &record.id, &old, &local);
    save(root, state)?;
    let next = mirror::project_local(
        &old,
        &current,
        queued,
        &conflict_text(&record, "The tracker"),
    )?;
    let next = mirror::patch_local(&next, &local)?;
    write_local(root, &record.path, &next, Some(&old))?;
    if queued {
        state.outbox.get_mut(&record.id).unwrap().staged = None;
    } else {
        state.outbox.remove(&record.id);
    }
    save(root, state)?;
    request_soon();
    Ok(
        json!({"ref":format!("vault:{}",record.path),"disposition":"applied","contentHash":vault_tasks::content_hash(&next),"tracker":{"queued":queued,"note":"Saved locally for tracker reconciliation. Unsupported statuses remain conflict notes."}}),
    )
}

/// Accept old companion state words only when no exact provider choice
/// exists. A kind with several choices is ambiguous, never guessed.
fn status_alias(state: &Ledger, team: &str, value: &Value) -> Result<Value, String> {
    let Some(word) = value.as_str() else {
        return Ok(value.clone());
    };
    let meta = state
        .meta_for(team)
        .ok_or("Sync the task tracker before editing.")?;
    if meta
        .states
        .iter()
        .any(|s| s.id == word || s.name.eq_ignore_ascii_case(word.trim()))
    {
        return Ok(value.clone());
    }
    let kind = match word.trim().to_ascii_lowercase().as_str() {
        "not started" | "todo" => StateKind::Unstarted,
        "planning" | "waiting" | "blocked" | "backlog" => StateKind::Backlog,
        "in progress" | "in sandbox-testing" => StateKind::Started,
        "done" | "delivered" => StateKind::Completed,
        "canceled" | "cancelled" => StateKind::Canceled,
        _ => return Ok(value.clone()),
    };
    let choices: Vec<_> = meta.states.iter().filter(|s| s.kind == kind).collect();
    match choices.as_slice() {
        [choice] => Ok(json!(choice.id)),
        _ => Err("TASK_INVALID: That legacy status has no unique tracker choice. Read the available statuses and choose an exact name or id.".into()),
    }
}

pub(crate) fn edit_task(
    root: &Path,
    rel: &str,
    expected_hash: &str,
    fields: &serde_json::Map<String, Value>,
) -> Result<Value, String> {
    edit_task_at(root, rel, expected_hash, fields, Local::now().date_naive())
}

pub(crate) fn edit_task_at(
    root: &Path,
    rel: &str,
    expected_hash: &str,
    fields: &serde_json::Map<String, Value>,
    today: chrono::NaiveDate,
) -> Result<Value, String> {
    let result = edit_task_unaudited(root, rel, expected_hash, fields, today);
    // X2: a refused edit is audited here; an accepted one when it is sent.
    if let Err(why) = &result {
        let id = cached_at(root).ok().and_then(|state| {
            record_at(&state, rel)
                .ok()
                .map(|r| (r.id.clone(), r.key.clone()))
        });
        api::audit_write(
            root,
            "update",
            None,
            (
                id.as_ref().map(|(id, _)| id.as_str()),
                id.as_ref().map(|(_, key)| key.as_str()),
            ),
            None,
            Some(json!({"ref": rel, "fields": Value::Object(fields.clone())})),
            None,
            Err(why.clone()),
        );
    }
    result
}

fn edit_task_unaudited(
    root: &Path,
    rel: &str,
    expected_hash: &str,
    fields: &serde_json::Map<String, Value>,
    today: chrono::NaiveDate,
) -> Result<Value, String> {
    let expected_hash = expected_hash.trim();
    let mut state = cached_at(root)?;
    restore_staged(root, &mut state)?;
    let record = record_at(&state, rel)?.clone();
    if record.kind == "project" {
        return edit_project(root, &mut state, record, expected_hash, fields);
    }
    if record.kind != "task"
        || record.issue.is_none()
        || record.remote_missing
        || record.state != "active"
    {
        return Err("TASK_LINKED: Only an available tracker issue can be edited.".into());
    }
    let old = read_mirror(root, &state.provider, &record.id, "task", rel)?
        .ok_or("TASK_LINKED: Restore the tracker mirror before editing.")?;
    if expected_hash.is_empty() || vault_tasks::content_hash(&old) != expected_hash {
        return Err("TASK_CONFLICT: This task changed after it was read. Read it again and reapply the edit.".into());
    }
    let mut values = Values::new();
    let mut local = Values::new();
    for (key, value) in fields {
        if let Some(key) = match key.as_str() {
            "dueTime" => Some("due_time"),
            "durationMin" => Some("duration_min"),
            "recurrence" => Some("recurrence"),
            "section" => Some("section"),
            _ => None,
        } {
            let value = crate::task_broker::tracker_local_value(key, value)?;
            local.insert(key.into(), value);
            continue;
        }
        let key = match key.as_str() {
            "parentKey" => "parent_key",
            "projectUrl" => {
                let path = value
                    .as_str()
                    .unwrap_or("")
                    .strip_prefix("vault:")
                    .unwrap_or(value.as_str().unwrap_or(""));
                let project = record_at(&state, path)?;
                if project.kind != "project" || project.remote_missing {
                    return Err("TASK_INVALID: Choose an available tracker project.".into());
                }
                values.insert("project".into(), json!(project.id));
                continue;
            }
            key => key,
        };
        let value = if key == "status" {
            status_alias(
                &state,
                &record
                    .issue
                    .as_ref()
                    .map(|i| i.team_id.clone())
                    .unwrap_or_default(),
                value,
            )?
        } else if key == "priority"
            && value
                .as_str()
                .is_some_and(|v| matches!(v, "P1" | "P2" | "P3" | "P4"))
        {
            json!(value.as_str().unwrap().as_bytes()[1] - b'0')
        } else {
            value.clone()
        };
        values.insert(key.into(), value);
    }
    let mut values = tracker::sync_fields::normalize(&values)?;
    let rolled_to = if let Some(status) = values.get("status") {
        let meta = state
            .meta_for(
                &record
                    .issue
                    .as_ref()
                    .map(|i| i.team_id.clone())
                    .unwrap_or_default(),
            )
            .ok_or("Sync the task tracker before editing.")?;
        let patch = tracker::sync_fields::patch(
            &Values::from([("status".into(), status.clone())]),
            meta,
            &state.projects(),
            &state.issues(),
            record.issue.as_ref().and_then(|i| i.project_id.as_deref()),
        )?;
        let target = meta
            .states
            .iter()
            .find(|s| Some(s.id.as_str()) == patch.state_id.as_deref());
        let fm = frontmatter::parse(&old);
        let recurrence = local.get("recurrence").and_then(Value::as_str).or_else(|| {
            if local.contains_key("recurrence") {
                None
            } else {
                fm.get("recurrence")
            }
        });
        // Use the same completion-date calculation as local recurring tasks.
        let next = if target.is_some_and(|s| s.kind == StateKind::Completed) {
            recurrence.and_then(|r| crate::quick_add::next_occurrence(r, today))
        } else {
            None
        };
        if let Some(next) = next {
            let mut choices = meta
                .states
                .iter()
                .filter(|s| s.kind == StateKind::Unstarted);
            let choice = choices.next().filter(|_| choices.next().is_none())
                .ok_or("TASK_INVALID: A recurring task needs a unique unstarted tracker state. Nothing was saved.")?;
            values.insert("status".into(), json!(choice.id));
            values.insert("due".into(), json!(next.to_string()));
            Some(next.to_string())
        } else {
            None
        }
    } else {
        None
    };
    let expected = mirror::local_values(&old)?;
    let outcome = api::edit_local_in_lease(
        root,
        &mut state,
        TrackerEdit {
            id: record.id,
            values,
            expected,
        },
        &local,
    )?;
    let next = read_mirror(
        root,
        &state.provider,
        outcome.id.as_deref().unwrap_or(""),
        "task",
        rel,
    )?
    .ok_or("The tracker mirror could not be read after editing.")?;
    Ok(json!({
        "ref": format!("vault:{rel}"),
        "disposition": "applied",
        "contentHash": vault_tasks::content_hash(&next),
        "status": frontmatter::parse(&next).get("status"),
        "rolledForwardTo": rolled_to,
        "tracker": {"queued":outcome.queued,"note":outcome.note},
    }))
}
