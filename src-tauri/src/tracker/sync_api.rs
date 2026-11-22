//! Cached reads and durable user writes for the tracker IPC. Agent tiers
//! and task-broker seams are the following A6 change.

use super::*;

pub(crate) fn cached(config: &Config) -> Result<Ledger, String> {
    load(
        &config.vault_path,
        config.tracker.provider.as_deref().unwrap_or(""),
        config.tracker.team_id.as_deref().unwrap_or(""),
    )
}

pub(crate) fn apply_status(config: &Config, status: &mut TrackerStatus) {
    match cached(config) {
        Ok(state) => {
            status.last_sync = state.last_sync.clone();
            status.last_error = state.last_error;
            status.rate = state.rate;
            status.next_sync = state.retry_at.or_else(|| {
                state
                    .last_sync
                    .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|at| {
                        (at + ChronoDuration::seconds(config.tracker.poll_seconds as i64))
                            .to_rfc3339()
                    })
            });
            status.outbox = state
                .outbox
                .len()
                .saturating_add(state.creates.values().filter(|e| e.issue.is_none()).count())
                .min(u32::MAX as usize) as u32;
            if status.state == TrackerRunState::Ready && status.last_error.is_some() {
                status.state = TrackerRunState::Error;
            }
            if matches!(
                status.state,
                TrackerRunState::Ready | TrackerRunState::Error
            ) && is_syncing(&config.vault_path)
            {
                status.state = TrackerRunState::Syncing;
            }
        }
        Err(error) => {
            status.last_error = Some(error);
            if status.state == TrackerRunState::Ready {
                status.state = TrackerRunState::Error;
            }
        }
    }
}

// ── The audit record (X2) ──────────────────────────────────────────────────

/// One audit row for a tracker write made here (person or agent, through
/// the IPC or an agent action): who asked comes from the enclosing
/// [`crate::audit::asking`] scope (an agent action's caller), else the
/// person. `result` is the write's: `Ok` with an optional note, or why it
/// failed. Never fails the write.
#[allow(clippy::too_many_arguments)]
pub(super) fn audit_write(
    root: &Path,
    action: &'static str,
    asked_by: Option<String>,
    (issue_id, issue_key): (Option<&str>, Option<&str>),
    before: Option<Value>,
    after: Option<Value>,
    request_key: Option<&str>,
    result: Result<Option<String>, String>,
) {
    let asked_by = asked_by.unwrap_or_else(crate::audit::asker);
    // An agent's session is the row's session; the person, an automation
    // and other runs (`<surface>:<id>`) have none.
    let session = crate::audit::is_session(&asked_by).then(|| asked_by.clone());
    let (outcome, detail) = match result {
        Ok(note) => ("ok", note),
        Err(why) => ("failed", Some(why)),
    };
    crate::audit::tracker_write(
        root,
        crate::audit::Write {
            action,
            session_key: session.as_deref(),
            asked_by,
            issue_id,
            issue_key,
            before,
            after,
            request_key,
            outcome,
            detail,
            ..crate::audit::Write::default()
        },
    );
}

/// A refusal before the audited write function ran (the tracker off, no
/// adapter, a request that doesn't validate): its failed row, then the
/// same result.
fn refused_before<T>(
    root: &Path,
    action: &'static str,
    issue: (Option<&str>, Option<&str>),
    after: Option<Value>,
    request_key: Option<&str>,
    step: Result<T, String>,
) -> Result<T, String> {
    if let Err(why) = &step {
        audit_write(
            root,
            action,
            None,
            issue,
            None,
            after,
            request_key,
            Err(why.clone()),
        );
    }
    step
}

/// The audit row of a queued edit's send (the pass): the changed fields as
/// the tracker had them and as sent. Keyed by the issue and a hash of the
/// base and pending values, so an edit the tracker keeps refusing is one
/// failed row, and its later success one ok row.
pub(super) fn audit_send(
    root: &Path,
    asked_by: Option<String>,
    (id, key): (&str, Option<&str>),
    base: &Values,
    pending: &Values,
    result: Result<Option<String>, String>,
) {
    let before: serde_json::Map<String, Value> = pending
        .keys()
        .map(|k| (k.clone(), base.get(k).cloned().unwrap_or(Value::Null)))
        .collect();
    let after: serde_json::Map<String, Value> = pending
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(json!([before, after]).to_string().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    let request = format!("update:{id}:{hex}");
    audit_write(
        root,
        "update",
        Some(asked_by.unwrap_or_else(|| "person".into())),
        (Some(id), key),
        Some(Value::Object(before)),
        Some(Value::Object(after)),
        Some(&request),
        result,
    );
}

/// A write's receipt as an audit result: confirmed, or why not (an
/// unconfirmed write may have gone and is never sent again).
fn receipt(
    result: &Result<TrackerWriteOutcome, String>,
    confirmed: bool,
) -> Result<Option<String>, String> {
    match result {
        Ok(_) if confirmed => Ok(None),
        Ok(outcome) => Err(format!("Unconfirmed: {}", outcome.note)),
        Err(why) => Err(why.clone()),
    }
}

fn project_row(state: &Ledger, project: &TrackerProject) -> TrackerProjectRow {
    let issues: Vec<_> = state
        .records
        .values()
        .filter_map(|r| {
            r.issue
                .as_ref()
                .filter(|i| i.project_id.as_deref() == Some(&project.id) && r.state == "active")
        })
        .collect();
    TrackerProjectRow {
        id: project.id.clone(),
        key: project.key.clone(),
        name: project.name.clone(),
        status_name: project.status.name.clone(),
        status_kind: project.status.kind.clone(),
        lead: project.lead.as_ref().map(|u| u.name.clone()),
        target_date: project.target_date.clone(),
        progress: project.progress,
        issue_count: issues.len() as u32,
        open_count: issues.iter().filter(|i| !i.state.kind.is_closed()).count() as u32,
        url: project.url.clone(),
        workspace_id: state
            .records
            .get(&project.id)
            .and_then(|r| r.workspace_id.clone()),
        team_key: project
            .team_ids
            .iter()
            .find(|team| state.has_team(team))
            .and_then(|team| team_key_for(state, team)),
    }
}

fn record_lock(record: &Record) -> TrackerLock {
    TrackerLock {
        kind: if record.remote_missing {
            TrackerLockKind::OutOfScope
        } else {
            TrackerLockKind::Linked
        },
        fields: Field::ALL
            .into_iter()
            .map(|f| f.key().to_string())
            .chain(READ_ONLY_KEYS.iter().map(|k| k.to_string()))
            .chain(IDENTITY_KEYS.iter().map(|k| k.to_string()))
            .collect(),
    }
}

/// A queued edit remains the value the person sees while offline. The
/// baseline stays remote-owned; this is only a projection for the editor.
fn visible_issue(state: &Ledger, record: &Record) -> Option<TrackerIssue> {
    match state.outbox.get(&record.id) {
        Some(pending) => projected_issue(state, record, &pending.values),
        None => record.issue.clone(),
    }
}

pub(super) fn projected_issue(
    state: &Ledger,
    record: &Record,
    values: &Values,
) -> Option<TrackerIssue> {
    let mut issue = record.issue.clone()?;
    let Some(meta) = state.meta_for(&issue.team_id) else {
        return Some(issue);
    };
    // Resolve fields independently: an unchanged ambiguous display name
    // must not hide an accepted status, title or other unrelated edit.
    let mut patches = Vec::new();
    let mut project_id = issue.project_id.clone();
    let projects = state.projects();
    let issues = state.issues();
    if let Some(value) = values.get("project") {
        if let Ok(patch) = tracker::sync_fields::patch(
            &Values::from([("project".into(), value.clone())]),
            meta,
            &projects,
            &issues,
            project_id.as_deref(),
        ) {
            if let Some(project) = &patch.project_id {
                project_id = project.clone();
            }
            patches.push(patch);
        }
    }
    for (key, value) in values.iter().filter(|(key, _)| key.as_str() != "project") {
        if let Ok(patch) = tracker::sync_fields::patch(
            &Values::from([(key.clone(), value.clone())]),
            meta,
            &projects,
            &issues,
            project_id.as_deref(),
        ) {
            patches.push(patch);
        }
    }
    for patch in patches {
        if let Some(title) = patch.title {
            issue.title = title;
        }
        if let Some(description) = patch.description {
            issue.description = description;
        }
        if let Some(id) = patch.state_id {
            if let Some(state) = meta.states.iter().find(|s| s.id == id) {
                issue.state = state.clone();
            }
        }
        if let Some(priority) = patch.priority {
            issue.priority = priority;
        }
        if let Some(id) = patch.assignee_id {
            issue.assignee = id.and_then(|id| meta.members.iter().find(|u| u.id == id).cloned());
        }
        if let Some(ids) = patch.label_ids {
            issue.labels = ids
                .iter()
                .filter_map(|id| {
                    meta.labels
                        .iter()
                        .find(|l| &l.id == id)
                        .map(|l| l.name.clone())
                })
                .collect();
        }
        if let Some(project) = patch.project_id {
            issue.project_id = project;
        }
        if let Some(due) = patch.due {
            issue.due = due;
        }
        if let Some(parent) = patch.parent_id {
            issue.parent_id = parent;
        }
        if let Some(estimate) = patch.estimate {
            issue.estimate = estimate;
        }
        if let Some(id) = patch.cycle_id {
            issue.cycle = id.and_then(|id| meta.cycles.iter().find(|c| c.id == id).cloned());
        }
        if let Some(id) = patch.milestone_id {
            issue.milestone =
                id.and_then(|id| meta.milestones.iter().find(|m| m.id == id).cloned());
        }
    }
    Some(issue)
}

/// Rendering a durable edit must resolve a changed state, never silently
/// reuse the old kind. Other display-name ambiguities keep cached identity.
pub(super) fn render_issue(
    state: &Ledger,
    record: &Record,
    values: &Values,
) -> Result<TrackerIssue, String> {
    let issue =
        projected_issue(state, record, values).ok_or("A staged tracker edit is not an issue.")?;
    let original = record
        .issue
        .as_ref()
        .ok_or("A staged tracker edit is not an issue.")?;
    if let Some(status) = values
        .get("status")
        .filter(|value| **value != json!(original.state.name))
    {
        let patch = tracker::sync_fields::patch(
            &Values::from([("status".into(), status.clone())]),
            state
                .meta_for(&issue.team_id)
                .ok_or("Restore tracker metadata before replaying a status edit.")?,
            &state.projects(),
            &state.issues(),
            issue.project_id.as_deref(),
        )?;
        if patch.state_id.as_deref() != Some(issue.state.id.as_str()) {
            return Err(
                "The staged tracker status has no matching state kind. Its journal was preserved."
                    .into(),
            );
        }
    }
    Ok(issue)
}

fn canonical(state: &Ledger, record: &Record, values: &Values) -> Result<Values, String> {
    let issue = record
        .issue
        .as_ref()
        .ok_or("That tracker record is not an issue.")?;
    tracker::sync_fields::patch(
        values,
        state
            .meta_for(&issue.team_id)
            .ok_or("Sync the tracker before editing.")?,
        &state.projects(),
        &state.issues(),
        issue.project_id.as_deref(),
    )?;
    let preview =
        projected_issue(state, record, values).ok_or("That tracker record is not an issue.")?;
    let all = mirror::values(&preview, &state.projects(), &state.issues());
    Ok(values
        .keys()
        .map(|key| (key.clone(), all[key].clone()))
        .collect())
}

/// The team a request names: its key (or id), or the shared team when none
/// is given. Only the config's own teams resolve; anything else (another
/// team's key or id) is refused before any request.
pub(crate) fn resolve_team(config: &Config, requested: Option<&str>) -> Result<String, String> {
    resolve_routed(config, tracker::linear::Work::Tracked, requested)
}

/// [`resolve_team`] with the team routing rule (`linear::team_for`, T1):
/// on Linear a named team must be AI or HOL (LP, PRO and the rest are
/// refused before any request), and with none named, Hollow-only work goes
/// to the personal team, tracked work to the shared one.
pub(crate) fn resolve_routed(
    config: &Config,
    work: tracker::linear::Work,
    requested: Option<&str>,
) -> Result<String, String> {
    use tracker::linear::{team_for, Work, PERSONAL_TEAM_KEY, PROVIDER};
    let shared = config
        .tracker
        .team_id
        .clone()
        .ok_or("Choose a task tracker and its team in Settings, Sources, Task tracker.")?;
    let linear = config.tracker.provider.as_deref() == Some(PROVIDER);
    let named = requested.map(str::trim).filter(|w| !w.is_empty());
    if linear {
        team_for(work, named)?;
    }
    let want = match (named, work) {
        (Some(want), _) => want,
        (None, Work::Tracked) => return Ok(shared),
        (None, Work::HollowOnly) if config.tracker.personal_team_id.is_some() => PERSONAL_TEAM_KEY,
        (None, Work::HollowOnly) => return Err("Hollow-only work (a commitment, agent or hollow-lesson label) goes to team Personal, which isn't set up here; name teamKey AI to file it in the shared team. Nothing was sent.".into()),
    };
    let named = |id: &Option<String>, key: &Option<String>| {
        id.as_deref() == Some(want) || key.as_deref().is_some_and(|k| k.eq_ignore_ascii_case(want))
    };
    if named(&config.tracker.team_id, &config.tracker.team_key) {
        return Ok(shared);
    }
    if named(
        &config.tracker.personal_team_id,
        &config.tracker.personal_team_key,
    ) {
        return Ok(config.tracker.personal_team_id.clone().unwrap_or_default());
    }
    // `personalTeamId` alone is enough: its key is the provider's own.
    if let (Some(id), Some(personal)) = (
        tracker::linear::allowed_team(want),
        config.tracker.personal_team_id.as_deref(),
    ) {
        if id == personal {
            return Ok(personal.to_string());
        }
    }
    Err("Hollow works only with its shared team and, when set up, its personal team (Personal). Nothing was sent.".into())
}

/// The team a mirrored issue (by id or key) or project belongs to, for
/// choosing the adapter before a write; the shared team when it isn't
/// mirrored (the write itself then refuses).
fn item_team(config: &Config, id: &str) -> Result<String, String> {
    let state = cached(config)?;
    let found = state
        .records
        .get(id)
        .or_else(|| state.records.values().find(|r| r.key == id))
        .and_then(|r| state.team_of_record(r));
    Ok(found
        .filter(|team| config.tracker.team_ids().contains(team))
        .unwrap_or_else(|| state.scope.clone()))
}

/// Focus a loaded ledger on the team of record `id`, so the single-team
/// checks below run against that team.
fn focus_item(state: &mut Ledger, id: &str) {
    let team = state
        .records
        .get(id)
        .or_else(|| state.records.values().find(|r| r.key == id))
        .and_then(|r| state.team_of_record(r));
    if let Some(team) = team.filter(|t| state.has_team(t)) {
        state.swap_to(&team);
    }
}

/// The id a create journals for `team`: None for the shared team (so its
/// requests read as before the second team), the team's id otherwise.
fn journal_team(config: &Config, team: &str) -> Option<String> {
    (config.tracker.team_id.as_deref() != Some(team)).then(|| team.to_string())
}

/// A team's short key: the synced metadata's, else the provider's pin.
pub(crate) fn team_key_for(state: &Ledger, team_id: &str) -> Option<String> {
    state
        .meta_for(team_id)
        .map(|m| m.team.key.clone())
        .filter(|k| !k.is_empty())
        .or_else(|| tracker::linear::team_key_of(team_id).map(str::to_string))
}

/// The team id a key names among the teams the ledger holds.
fn team_id_for_key(state: &Ledger, key: &str) -> Option<String> {
    state
        .teams()
        .into_iter()
        .find(|team| team_key_for(state, team).is_some_and(|k| k.eq_ignore_ascii_case(key)))
}

pub(crate) fn view(config: &Config, query: &TrackerViewQuery) -> Result<TrackerView, String> {
    let state = cached(config)?;
    let projects = state.projects();
    let search = query.search.as_deref().unwrap_or("").to_lowercase();
    let team_filter = query
        .team_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .map(|key| team_id_for_key(&state, key).unwrap_or_default());
    let mut rows = Vec::new();
    for record in state.records.values().filter(|r| r.kind == "task") {
        let Some(issue) = visible_issue(&state, record) else {
            continue;
        };
        if team_filter
            .as_ref()
            .is_some_and(|team| &issue.team_id != team)
        {
            continue;
        }
        if !query.include_archived && record.state != "active" {
            continue;
        }
        if query
            .state_kinds
            .as_ref()
            .is_some_and(|k| !k.contains(&issue.state.kind))
        {
            continue;
        }
        if query.state_kinds.is_none() && !query.include_closed && issue.state.kind.is_closed() {
            continue;
        }
        if query
            .assignee_id
            .as_ref()
            .is_some_and(|id| issue.assignee.as_ref().is_none_or(|u| &u.id != id))
        {
            continue;
        }
        if query
            .project_id
            .as_ref()
            .is_some_and(|id| issue.project_id.as_ref() != Some(id))
        {
            continue;
        }
        if query
            .label
            .as_ref()
            .is_some_and(|l| !issue.labels.contains(l))
        {
            continue;
        }
        if query
            .cycle_id
            .as_ref()
            .is_some_and(|id| issue.cycle.as_ref().is_none_or(|c| &c.id != id))
        {
            continue;
        }
        if !search.is_empty()
            && !format!("{} {}", issue.key, issue.title)
                .to_lowercase()
                .contains(&search)
        {
            continue;
        }
        rows.push(TrackerViewRow {
            id: issue.id.clone(),
            key: issue.key.clone(),
            title: issue.title.clone(),
            status_name: issue.state.name.clone(),
            status_kind: issue.state.kind,
            priority: issue.priority,
            labels: issue.labels.clone(),
            project: issue
                .project_id
                .as_ref()
                .and_then(|id| projects.get(id))
                .map(|p| p.name.clone()),
            project_id: issue.project_id.clone(),
            due: issue.due.clone(),
            assignee: issue.assignee.as_ref().map(|u| u.name.clone()),
            assignee_id: issue.assignee.as_ref().map(|u| u.id.clone()),
            estimate: issue.estimate,
            cycle: issue.cycle.as_ref().map(|c| {
                c.name
                    .clone()
                    .unwrap_or_else(|| format!("Cycle {}", c.number))
            }),
            milestone: issue.milestone.as_ref().map(|m| m.name.clone()),
            team_key: team_key_for(&state, &issue.team_id),
            updated_at: issue.updated_at.clone(),
            url: issue.url.clone(),
            path: Some(record.path.clone()),
            lock: Some(record_lock(record)),
            pending: state.outbox.contains_key(&issue.id),
            conflict: !record.conflicts.is_empty(),
            archived: record.state != "active",
            source_label: None,
        });
    }
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let total = rows.len().min(u32::MAX as usize) as u32;
    rows.truncate(query.limit.unwrap_or(1000).min(10_000) as usize);
    let note = state
        .last_sync
        .is_none()
        .then(|| "The task tracker hasn't completed its first sync yet.".into());
    Ok(TrackerView {
        provider: config.tracker.provider.clone(),
        display_name: config
            .tracker
            .provider_info()
            .map(|p| p.display_name.into()),
        team_key: config.tracker.team_key.clone(),
        team_keys: config
            .tracker
            .team_ids()
            .iter()
            .filter_map(|team| team_key_for(&state, team))
            .collect(),
        last_sync: state.last_sync.clone(),
        sync_error: state.last_error.clone(),
        states: state
            .meta
            .as_ref()
            .map(|m| m.states.clone())
            .unwrap_or_default(),
        rows,
        projects: projects
            .values()
            .filter(|p| {
                team_filter
                    .as_ref()
                    .is_none_or(|team| p.team_ids.contains(team))
            })
            .map(|p| project_row(&state, p))
            .collect(),
        total,
        note,
    })
}

pub(crate) fn issue(config: &Config, id: &str) -> Result<TrackerIssueDetail, String> {
    let state = cached(config)?;
    issue_in_state(&config.vault_path, &state, id)
}

pub(super) fn issue_in_state(
    root: &Path,
    state: &Ledger,
    id: &str,
) -> Result<TrackerIssueDetail, String> {
    let record = state
        .records
        .get(id)
        .or_else(|| state.records.values().find(|r| r.key == id))
        .ok_or("The tracker issue isn't mirrored yet. Sync first.")?;
    let issue = visible_issue(state, record).ok_or("That tracker record is not an issue.")?;
    let old = read(root, &record.path)?;
    Ok(TrackerIssueDetail {
        project: issue
            .project_id
            .as_ref()
            .and_then(|id| state.records.get(id))
            .and_then(|r| r.project.as_ref())
            .map(|p| project_row(state, p)),
        issue,
        path: Some(record.path.clone()),
        workspace_id: record.workspace_id.clone(),
        lock: Some(record_lock(record)),
        writable_fields: if record.state == "active" {
            state.writable_fields().to_vec()
        } else {
            Vec::new()
        },
        conflicts: record
            .conflicts
            .iter()
            .filter_map(|c| {
                Some(TrackerConflict {
                    field: Field::from_key(&c.field)?,
                    local: c.local.to_string(),
                    remote: c.remote.to_string(),
                    at: c.at.clone(),
                })
            })
            .collect(),
        pending: state
            .outbox
            .get(&record.id)
            .map(|o| o.values.keys().filter_map(|k| Field::from_key(k)).collect())
            .unwrap_or_default(),
        vault_notes: old.as_deref().map(mirror::vault_notes).transpose()?,
    })
}

pub(crate) fn team_meta(config: &Config) -> Result<TrackerTeamMeta, String> {
    cached(config)?
        .meta
        .ok_or_else(|| "The tracker's team metadata isn't mirrored yet. Sync first.".into())
}

/// The issue detail's activity: a mirrored issue's change history, read from
/// the tracker on demand (read-only, never stored in the mirror). The
/// frontend shows its synthetic feed when this fails.
pub(crate) async fn history(config: &Config, id: &str) -> Result<tracker::TrackerHistory, String> {
    enabled(config)?;
    let team = item_team(config, id)?;
    let adapter = tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())?;
    history_with(config, adapter.as_ref(), id).await
}

pub(crate) async fn history_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    id: &str,
) -> Result<tracker::TrackerHistory, String> {
    enabled(config)?;
    let state = cached(config)?;
    if remote.provider() != state.provider {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let record = state
        .records
        .get(id)
        .or_else(|| state.records.values().find(|r| r.key == id))
        .ok_or("The tracker issue isn't mirrored yet. Sync first.")?;
    let issue = record
        .issue
        .as_ref()
        .filter(|i| state.has_team(&i.team_id))
        .ok_or("Only an issue of the team Hollow syncs has a history here.")?;
    if record.remote_missing {
        return Err("The tracker no longer has this issue.".into());
    }
    remote
        .issue_history(&issue.id)
        .await
        .map_err(|e| e.to_string())
}

/// Durable local acceptance. The runtime wakes immediately and sends after
/// 250 ms; offline edits remain visible and survive a process restart.
pub(crate) async fn edit(
    config: &Config,
    edit: TrackerEdit,
) -> Result<TrackerWriteOutcome, String> {
    let root = &config.vault_path;
    let id = edit.id.clone();
    let after = serde_json::to_value(&edit.values).ok();
    let result = match enabled(config) {
        Err(why) => Err(why),
        Ok(()) => {
            let _guard = root_lock(root).lock_owned().await;
            file_lock(root).and_then(|_file| {
                let mut state = cached(config)?;
                let _lease = crate::automated_history::tracker_mirror_write_lease(
                    root,
                    &folders(&state, []),
                )?;
                edit_in_lease(root, &mut state, edit)
            })
        }
    };
    // X2: a refused edit is audited here; an accepted one when it is sent.
    if let Err(why) = &result {
        audit_write(
            root,
            "update",
            None,
            (Some(&id), None),
            None,
            after,
            None,
            Err(why.clone()),
        );
    }
    result
}

/// The broker already holds sync exclusion and its own live History lease.
/// Share the editor's journal and renderer without trying to acquire a
/// second background lease inside that authorization.
pub(super) fn edit_in_lease(
    root: &Path,
    state: &mut Ledger,
    edit: TrackerEdit,
) -> Result<TrackerWriteOutcome, String> {
    edit_local_in_lease(root, state, edit, &Values::new())
}

pub(super) fn edit_local_in_lease(
    root: &Path,
    state: &mut Ledger,
    edit: TrackerEdit,
    local: &Values,
) -> Result<TrackerWriteOutcome, String> {
    if state.stored_in_hollow {
        return Err(STORED_IN_HOLLOW.into());
    }
    restore_staged(root, state)?;
    let mut record = state
        .records
        .get(&edit.id)
        .cloned()
        .ok_or("The tracker issue isn't mirrored yet.")?;
    if record.state != "active" {
        return Err(
            "This tracker issue is archived or unavailable; it cannot be edited here.".into(),
        );
    }
    let issue = record
        .issue
        .clone()
        .ok_or("That tracker record is not an issue.")?;
    let values = tracker::sync_fields::normalize(&edit.values)?;
    if values.keys().any(|key| {
        Field::from_key(key).is_none_or(|field| !state.writable_fields().contains(&field))
    }) {
        return Err("The tracker does not allow one of these fields. Sync and read its writable fields before editing.".into());
    }
    let expected = tracker::sync_fields::normalize(&edit.expected)?;
    if values.keys().any(|key| !expected.contains_key(key)) {
        return Err("Every changed tracker field needs the value the editor read.".into());
    }
    let expected: Values = expected
        .into_iter()
        .filter(|(key, _)| values.contains_key(key))
        .collect();
    let expected = canonical(state, &record, &expected)?;
    let values = canonical(state, &record, &values)?;
    let old = read_mirror(
        root,
        &state.provider,
        &record.id,
        &record.kind,
        &record.path,
    )?
    .ok_or("The tracker mirror is missing; sync before editing.")?;
    let mut current = mirror::local_values(&old)?;
    if values
        .keys()
        .any(|key| expected.get(key) != current.get(key))
    {
        return Err("This task changed after the editor read it. Reload it before editing.".into());
    }
    let picked = tracker::sync_fields::patch(
        &values,
        state
            .meta_for(&issue.team_id)
            .ok_or("Sync the tracker before editing.")?,
        &state.projects(),
        &state.issues(),
        issue.project_id.as_deref(),
    )?;
    let previous = state.outbox.get(&record.id).cloned();
    record.conflicts.retain(|c| !values.contains_key(&c.field));
    record.error = None;
    let before = current.clone();
    current.extend(values.clone());
    let issue = render_issue(state, &record, &current)?;
    // The editor may make several changes before the next pull. Keep every
    // difference from the remote baseline, including earlier queued fields.
    let queued = stage_edit(state, &record, before, current.clone());
    // Q-R23: the project this edit picked, by provider id (a name resolved
    // now may name another project after a rename).
    if let Some(pending) = state.outbox.get_mut(&record.id) {
        pending.project_id = if values.contains_key("project") {
            picked.project_id.clone().flatten()
        } else {
            kept_project_id(previous.as_ref(), &pending.values)
        };
    }
    stage_local(state, &record.id, &old, local);
    save(root, state)?;
    let project_path = issue
        .project_id
        .as_ref()
        .and_then(|id| state.records.get(id))
        .map(|r| r.path.as_str());
    let mut next = mirror::task(
        &old,
        &state.provider,
        &issue,
        &current,
        project_path,
        &record.state,
        queued,
        &if queued {
            format!(
                "Waiting to send to the tracker.\n{}",
                conflict_text(&record, "The tracker")
            )
        } else {
            conflict_text(&record, "The tracker")
        },
    )?;
    next = mirror::patch_local(&next, local)?;
    write_local(root, &record.path, &next, Some(&old))?;
    if queued {
        if let Some(pending) = state.outbox.get_mut(&record.id) {
            pending.staged = None;
        }
    } else {
        state.outbox.remove(&record.id);
    }
    state.records.insert(record.id.clone(), record.clone());
    save(root, state)?;
    request_soon();
    Ok(TrackerWriteOutcome {
        id: Some(record.id),
        key: Some(record.key),
        path: Some(record.path),
        queued,
        note: if queued {
            "Saved locally. Waiting to send to the tracker."
        } else {
            "Saved locally. No pending tracker changes."
        }
        .into(),
    })
}

pub(super) fn stage_edit(
    state: &mut Ledger,
    record: &Record,
    before: Values,
    current: Values,
) -> bool {
    let pending: Values = current
        .iter()
        .filter(|(key, value)| record.base.get(*key) != Some(*value))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let queued = !pending.is_empty();
    // A field acknowledgement belongs to the same durable intent as its
    // values, even if the process stops before the mirror write.
    state.records.insert(record.id.clone(), record.clone());
    // Even a complete revert needs a journal until its file write lands.
    // Otherwise a crash would let capture resend the previous local edit.
    state.outbox.insert(
        record.id.clone(),
        Outbox {
            values: pending,
            base: record.base.clone(),
            staged: Some(StagedEdit {
                before,
                after: current,
                local_before: Values::new(),
                local_after: Values::new(),
            }),
            // X2: who asked, for the audit row the send leaves.
            asked_by: Some(crate::audit::asker()),
            project_id: None,
            generation: state.generation,
        },
    );
    queued
}

pub(super) fn stage_local(state: &mut Ledger, id: &str, old: &str, local: &Values) {
    let fm = frontmatter::parse(old);
    let staged = state
        .outbox
        .get_mut(id)
        .and_then(|o| o.staged.as_mut())
        .unwrap();
    staged.local_before = local
        .keys()
        .map(|key| {
            (
                key.clone(),
                fm.get(key).map(|v| json!(v)).unwrap_or(Value::Null),
            )
        })
        .collect();
    staged.local_after = local
        .iter()
        .map(|(key, value)| {
            (
                key.clone(),
                match value {
                    Value::Null => Value::Null,
                    Value::String(_) => value.clone(),
                    _ => json!(value.to_string()),
                },
            )
        })
        .collect();
}

/// The journal key is supplied by the caller and reused for a retry. Before
/// the first request we persist `attempted`; a lost reply is never blindly
/// retried. Recovery searches only scoped issues newer than the journal.
pub(crate) async fn create_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    agent_key: &str,
    input: NewIssue,
) -> Result<TrackerWriteOutcome, String> {
    let after =
        json!({"title": input.title, "projectId": input.project_id, "stateId": input.state_id});
    let result = create_unaudited(config, remote, agent_key, input).await;
    let issue = result.as_ref().ok().and_then(|o| o.id.clone());
    let key = result.as_ref().ok().and_then(|o| o.key.clone());
    audit_write(
        &config.vault_path,
        "create",
        None,
        (issue.as_deref(), key.as_deref()),
        None,
        Some(after),
        Some(agent_key),
        receipt(&result, issue.is_some()),
    );
    result
}

async fn create_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    agent_key: &str,
    input: NewIssue,
) -> Result<TrackerWriteOutcome, String> {
    enabled(config)?;
    if !valid_id(agent_key) || input.title.trim().is_empty() {
        return Err("A tracker create needs a stable key and a nonblank title.".into());
    }
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let mut state = cached(config)?;
    let team = resolve_team(config, input.team_id.as_deref())?;
    if !state.has_team(&team) {
        return Err("Sync the tracker before creating a task.".into());
    }
    state.swap_to(&team);
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let _lease = crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
    if let Some(existing) = state.creates.get(agent_key) {
        if existing.input != input {
            return Err("That create key already names a different request.".into());
        }
    } else {
        tracker::sync_fields::validate_new(
            &input,
            state
                .meta
                .as_ref()
                .ok_or("Sync the tracker before creating a task.")?,
            &state.projects(),
            &state.issues(),
        )?;
        state.creates.insert(
            agent_key.into(),
            Create {
                agent_key: agent_key.into(),
                input: input.clone(),
                at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                attempted: false,
                issue: None,
                error: None,
                issue_id: crate::tracker_sync::new_issue_id(),
                resends: 0,
            },
        );
        save(root, &state)?;
    }
    let mut issues = state.issues();
    recover_creates(root, remote, &mut state, &mut issues)
        .await
        .map_err(|e| e.to_string())?;
    let entry = state.creates.get_mut(agent_key).unwrap();
    if entry.issue.is_none() && !entry.attempted {
        // An entry an older build journaled gets its id now, saved with
        // `attempted` before the request (Q-R22).
        if entry.issue_id.is_none() {
            entry.issue_id = crate::tracker_sync::new_issue_id();
        }
        entry.attempted = true;
        let input = crate::tracker_sync::create_input(entry);
        save(root, &state)?;
        match remote.create_issue(input).await {
            Ok(issue) if issue.team_id == state.scope && valid_id(&issue.id) => {
                let entry = state.creates.get_mut(agent_key).unwrap();
                entry.issue = Some(issue);
                entry.error = None;
            }
            Ok(_) => {
                state.creates.get_mut(agent_key).unwrap().error = Some("The provider returned an out-of-scope create result. It was discarded; the attempted journal was retained and will not send another create.".into());
                save(root, &state)?;
                return Err(TrackerError::OutOfScope.to_string());
            }
            Err(error) => {
                state.creates.get_mut(agent_key).unwrap().error = Some(error.to_string());
                save(root, &state)?;
                request_soon();
                return Ok(TrackerWriteOutcome { id: None, key: None, path: None, queued: true, note: "The create's outcome is unconfirmed. Hollow will check the tracker before retrying.".into() });
            }
        }
    }
    save(root, &state)?;
    request_soon();
    let entry = &state.creates[agent_key];
    Ok(TrackerWriteOutcome {
        id: entry.issue.as_ref().map(|i| i.id.clone()),
        key: entry.issue.as_ref().map(|i| i.key.clone()),
        path: entry
            .issue
            .as_ref()
            .map(|i| format!("tasks/tracker-{}.md", i.id)),
        queued: entry.issue.is_none(),
        note: entry.error.clone().unwrap_or_else(|| {
            "Created in the tracker; its mirror arrives on the next pass.".into()
        }),
    })
}

pub(crate) async fn create(
    config: &Config,
    task: TrackerNewTask,
) -> Result<TrackerWriteOutcome, String> {
    create_named(config, task, None).await
}

pub(crate) async fn create_named(
    config: &Config,
    task: TrackerNewTask,
    agent_key: Option<&str>,
) -> Result<TrackerWriteOutcome, String> {
    let after = json!({"title": task.title, "projectId": task.project_id});
    let prepared = prepare_create(config, task);
    let (adapter, key, input) = refused_before(
        &config.vault_path,
        "create",
        (None, None),
        Some(after),
        agent_key,
        prepared,
    )?;
    create_with(config, adapter.as_ref(), agent_key.unwrap_or(&key), input).await
}

/// `create_named`'s checks and request, before any tracker call.
fn prepare_create(
    config: &Config,
    task: TrackerNewTask,
) -> Result<(Box<dyn TrackerAdapter>, String, NewIssue), String> {
    let (team, key, input) = prepare_input(config, task)?;
    let adapter = tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())?;
    Ok((adapter, key, input))
}

/// The request, its journal key and its team, before any adapter exists.
fn prepare_input(
    config: &Config,
    task: TrackerNewTask,
) -> Result<(String, String, NewIssue), String> {
    enabled(config)?;
    let work = tracker::linear::Work::of_labels(&task.labels);
    let team = resolve_routed(config, work, task.team_key.as_deref())?;
    let state = cached(config)?;
    let meta = state
        .meta_for(&team)
        .ok_or("Sync the tracker before creating a task.")?;
    let values = BTreeMap::from([
        ("title".into(), json!(task.title)),
        (
            "description".into(),
            json!(task.description.unwrap_or_default()),
        ),
        ("labels".into(), json!(task.labels)),
        ("priority".into(), json!(task.priority.unwrap_or(0))),
    ]);
    let mut values = values;
    for (key, value) in [
        ("status", task.state_id),
        ("assignee", task.assignee_id),
        ("project", task.project_id),
        ("due", task.due),
        ("parent_key", task.parent_id),
        ("cycle", task.cycle_id),
        ("milestone", task.milestone_id),
    ] {
        if let Some(value) = value {
            values.insert(key.into(), json!(value));
        }
    }
    if let Some(estimate) = task.estimate {
        values.insert("estimate".into(), json!(estimate));
    }
    let patch =
        tracker::sync_fields::patch(&values, meta, &state.projects(), &state.issues(), None)?;
    let input = NewIssue {
        title: patch.title.unwrap(),
        description: patch.description,
        state_id: patch.state_id,
        priority: patch.priority,
        assignee_id: patch.assignee_id.flatten(),
        label_ids: patch.label_ids.unwrap_or_default(),
        project_id: patch.project_id.flatten(),
        due: patch.due.flatten(),
        parent_id: patch.parent_id.flatten(),
        estimate: patch.estimate.flatten(),
        cycle_id: patch.cycle_id.flatten(),
        milestone_id: patch.milestone_id.flatten(),
        team_id: journal_team(config, &team),
        id: None,
    };
    // This IPC shape predates the agent action's idempotency key. Reuse an
    // identical request after a lost reply or restart instead of generating
    // a new journal key. A6 callers use create_with with their own stable key.
    use sha2::{Digest, Sha256};
    let key = format!(
        "task-{:x}",
        Sha256::digest(
            serde_json::to_vec(&input).map_err(|_| "The create request could not be encoded.")?
        )
    );
    Ok((team, key, input))
}

pub(crate) async fn create_project(
    config: &Config,
    project: TrackerNewProject,
    agent_key: Option<&str>,
) -> Result<TrackerWriteOutcome, String> {
    let adapter = refused_before(
        &config.vault_path,
        "create",
        (None, None),
        Some(json!({"project": project.name, "workspaceId": project.workspace_id})),
        agent_key,
        enabled(config).and_then(|()| {
            let team = resolve_team(config, project.team_key.as_deref())?;
            tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())
        }),
    )?;
    create_project_with(config, adapter.as_ref(), project, agent_key).await
}

pub(crate) async fn create_project_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    project: TrackerNewProject,
    agent_key: Option<&str>,
) -> Result<TrackerWriteOutcome, String> {
    let after = json!({"project": project.name, "workspaceId": project.workspace_id});
    // The create journal's own key (the caller's, or the request's digest).
    let mut used: Option<String> = None;
    let result = create_project_unaudited(config, remote, project, agent_key, &mut used).await;
    let request = used.or_else(|| agent_key.map(str::to_string));
    let id = result.as_ref().ok().and_then(|o| o.id.clone());
    let key = result.as_ref().ok().and_then(|o| o.key.clone());
    audit_write(
        &config.vault_path,
        "create",
        None,
        (id.as_deref(), key.as_deref()),
        None,
        Some(after),
        request.as_deref(),
        receipt(&result, id.is_some()),
    );
    result
}

async fn create_project_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    project: TrackerNewProject,
    agent_key: Option<&str>,
    used: &mut Option<String>,
) -> Result<TrackerWriteOutcome, String> {
    enabled(config)?;
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let mut state = cached(config)?;
    let team = resolve_team(config, project.team_key.as_deref())?;
    if !state.has_team(&team) {
        return Err("Sync the tracker before creating a project.".into());
    }
    state.swap_to(&team);
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let meta = state
        .meta
        .as_ref()
        .ok_or("Sync the tracker before creating a project.")?;
    let workspace = project.workspace_id.clone();
    let already = match &workspace {
        Some(id) => crate::project_portfolio::tracked_id(root, id)?,
        None => None,
    };
    let values = Values::from([
        ("title".into(), json!(project.name)),
        (
            "description".into(),
            json!(project.description.unwrap_or_default()),
        ),
        ("lead".into(), json!(project.lead_id.unwrap_or_default())),
        (
            "target_date".into(),
            json!(project.target_date.unwrap_or_default()),
        ),
    ]);
    let patch = tracker::sync_fields::project_patch(&values, meta)?;
    let input = NewProject {
        name: patch.name.unwrap(),
        description: patch.description,
        lead_id: patch.lead_id.flatten(),
        target_date: patch.target_date.flatten(),
        team_id: journal_team(config, &team),
        ..Default::default()
    };
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(
        serde_json::to_vec(&input).map_err(|_| "The project create could not be encoded.")?,
    );
    // The same name tracked from two workspaces is two requests.
    if let Some(id) = &workspace {
        digest.update(b"\0workspace:");
        digest.update(id.as_bytes());
    }
    let fallback = format!("project-{:x}", digest.finalize());
    let key = agent_key.unwrap_or(&fallback);
    *used = Some(key.to_string());
    if !valid_id(key) {
        return Err("A project create needs a stable request key.".into());
    }
    let _lease = crate::automated_history::tracker_mirror_write_lease(
        root,
        &folders(
            &state,
            workspace.iter().map(|id| format!("workspaces/{id}")),
        ),
    )?;
    // A tracked workspace is refused before any request, unless this is the
    // retry of the very create that tracked it (finishing its link).
    if let Some(tracked) = &already {
        let retry = state.project_creates.get(key).is_some_and(|e| {
            e.workspace_id == workspace && e.project.as_ref().is_some_and(|p| &p.id == tracked)
        });
        if !retry {
            return Err("This project is already tracked; open it from Tasks, Projects.".into());
        }
    }
    if let Some(entry) = state.project_creates.get(key) {
        if entry.input != input || entry.workspace_id != workspace {
            return Err("That project create key already names a different request.".into());
        }
    } else {
        state.project_creates.insert(
            key.into(),
            ProjectCreate {
                input: input.clone(),
                attempted: false,
                project: None,
                error: None,
                out_of_scope: false,
                workspace_id: workspace.clone(),
            },
        );
        save(root, &state)?;
    }
    if !state.project_creates[key].attempted {
        state.project_creates.get_mut(key).unwrap().attempted = true;
        save(root, &state)?;
        match remote.create_project(input).await {
            Ok(project) if valid_id(&project.id) && project.team_ids.contains(&state.scope) => {
                state.project_creates.get_mut(key).unwrap().project = Some(project);
                state.projects_at = None;
            }
            Ok(_) => {
                state.project_creates.get_mut(key).unwrap().out_of_scope = true;
                state.project_creates.get_mut(key).unwrap().error = Some("The provider returned an out-of-scope project. Its result was discarded; this create will not be resent.".into());
            }
            Err(TrackerError::OutOfScope) => {
                let entry = state.project_creates.get_mut(key).unwrap();
                entry.out_of_scope = true;
                entry.error = Some(TrackerError::OutOfScope.to_string());
            }
            Err(error) => {
                state.project_creates.get_mut(key).unwrap().error = Some(format!("Project create is unconfirmed: {error}. It will not be resent; check the tracker before any new request."));
            }
        }
        save(root, &state)?;
    }
    let entry = &state.project_creates[key];
    if entry.out_of_scope {
        return Err(TrackerError::OutOfScope.to_string());
    }
    // Track in: link the workspace and record its baseline (a retry, never a
    // resend, finishes what a crash cut short; so does the next pass).
    if let (Some(id), Some(made)) = (&workspace, entry.project.clone()) {
        finish_track_in(root, &mut state, id, &made)?;
    }
    let entry = &state.project_creates[key];
    request_soon();
    Ok(TrackerWriteOutcome {
        id:entry.project.as_ref().map(|p| p.id.clone()), key:entry.project.as_ref().and_then(|p| p.key.clone()),
        path:entry.project.as_ref().and_then(|p| state.records.get(&p.id))
            .filter(|record| record.state == "active").map(|record| record.path.clone()),
        queued:entry.project.is_none(), note:entry.error.clone().unwrap_or_else(|| if entry.project.is_some() {"Created in the tracker. Its mirror arrives on the next pass.".into()} else {"Project create is unconfirmed and will not be resent. Check the tracker before any new request.".into()}),
    })
}

pub(crate) async fn archive(config: &Config, id: &str) -> Result<TrackerWriteOutcome, String> {
    let adapter = refused_before(
        &config.vault_path,
        "archive",
        (Some(id), None),
        Some(json!({"archived": true})),
        Some(&format!("archive:{id}")),
        enabled(config).and_then(|()| {
            let team = item_team(config, id)?;
            tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())
        }),
    )?;
    archive_with(config, adapter.as_ref(), id).await
}

pub(crate) async fn archive_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    id: &str,
) -> Result<TrackerWriteOutcome, String> {
    let result = archive_unaudited(config, remote, id).await;
    let key = result.as_ref().ok().and_then(|o| o.key.clone());
    let confirmed = result.as_ref().is_ok_and(|o| !o.queued);
    audit_write(
        &config.vault_path,
        "archive",
        None,
        (Some(id), key.as_deref()),
        Some(json!({"archived": false})),
        Some(json!({"archived": true})),
        Some(&format!("archive:{id}")),
        receipt(&result, confirmed),
    );
    result
}

async fn archive_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    id: &str,
) -> Result<TrackerWriteOutcome, String> {
    enabled(config)?;
    let _guard = root_lock(&config.vault_path).lock_owned().await;
    let _file = file_lock(&config.vault_path)?;
    let mut state = cached(config)?;
    focus_item(&mut state, id);
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let record = state
        .records
        .get(id)
        .cloned()
        .ok_or("Sync the tracker before archiving that issue.")?;
    if record.kind != "task" || record.issue.is_none() || record.remote_missing {
        return Err("Only an available tracker issue can be archived.".into());
    }
    let _lease = crate::automated_history::tracker_mirror_write_lease(
        &config.vault_path,
        &folders(&state, []),
    )?;
    if !state.archives.contains_key(id) {
        state.archives.insert(
            id.into(),
            Archive {
                attempted: false,
                confirmed: false,
                error: None,
            },
        );
        save(&config.vault_path, &state)?;
    }
    if !state.archives[id].attempted {
        state.archives.get_mut(id).unwrap().attempted = true;
        save(&config.vault_path, &state)?;
        match remote.archive_issue(id).await {
            Ok(()) => state.archives.get_mut(id).unwrap().confirmed = true,
            Err(error) => state.archives.get_mut(id).unwrap().error = Some(error.to_string()),
        }
        save(&config.vault_path, &state)?;
    }
    request_soon();
    Ok(TrackerWriteOutcome {
        id: Some(id.into()),
        key: Some(record.key.clone()),
        path: Some(record.path.clone()),
        queued: !state.archives[id].confirmed,
        note: if state.archives[id].confirmed {"Archived in the tracker. Its local mirror is retained."} else {"The archive is unconfirmed and will not be resent. Sync and check the tracker; the local mirror is retained."}.into(),
    })
}

/// Finish a "Track in" create: name the project in the workspace's
/// PROJECT.md (the pass's own renderer), then record its sync baseline (the
/// values just written), so the pass adopts that file as an existing mirror
/// with nothing to send. Idempotent. Refused when the project is already
/// mirrored at another path, so one tracker project never gets two mirrors.
pub(super) fn finish_track_in(
    root: &Path,
    state: &mut Ledger,
    workspace_id: &str,
    made: &TrackerProject,
) -> Result<(), String> {
    let path = format!("workspaces/{workspace_id}/PROJECT.md");
    if let Some(record) = state.records.get(&made.id) {
        if record.path != path {
            return Err(format!(
                "This project is already mirrored at {}; Hollow left {path} alone.",
                record.path
            ));
        }
    }
    crate::project_portfolio::link_tracked(root, workspace_id, &state.provider, made)?;
    record_baseline(root, state, &path, made)
}

fn record_baseline(
    root: &Path,
    state: &mut Ledger,
    path: &str,
    made: &TrackerProject,
) -> Result<(), String> {
    if state.records.contains_key(&made.id) {
        return Ok(());
    }
    state.records.insert(
        made.id.clone(),
        Record {
            id: made.id.clone(),
            key: made.key.clone().unwrap_or_default(),
            kind: "project".into(),
            path: path.to_string(),
            workspace_id: path.split('/').nth(1).map(str::to_string),
            base: mirror::project_values(made),
            conflicts: Vec::new(),
            error: None,
            remote_missing: false,
            state: "active".into(),
            issue: None,
            project: Some(made.clone()),
        },
    );
    save(root, state)
}

/// The pass's first step: finish every Track in whose create succeeded but
/// whose link or baseline a crash cut short, before any mirror is found or
/// any workspace is made for the project. A file that already names the
/// project (its folder renamed since) gets the baseline where it is, and a
/// workspace that is gone is never recreated: that one is left to the pass.
pub(super) fn finish_track_ins(root: &Path, state: &mut Ledger) -> Result<(), String> {
    let pending: Vec<(String, TrackerProject)> = state
        .project_creates
        .values()
        .filter_map(|e| Some((e.workspace_id.clone()?, e.project.clone()?)))
        .filter(|(_, made)| !state.records.contains_key(&made.id))
        .collect();
    if pending.is_empty() {
        return Ok(());
    }
    let found = adopted_paths(root, &state.provider.clone())?;
    let _lease = crate::automated_history::tracker_mirror_write_lease(
        root,
        &folders(
            state,
            pending.iter().map(|(ws, _)| format!("workspaces/{ws}")),
        ),
    )?;
    for (ws, made) in pending {
        if let Some(path) = found.get(&made.id) {
            if path_kind("project", path) {
                record_baseline(root, state, path, &made)?;
            }
        } else if root.join(format!("workspaces/{ws}")).is_dir() {
            finish_track_in(root, state, &ws, &made)?;
        }
    }
    Ok(())
}

/// Longest comment Hollow posts, in UTF-8 bytes (the adapter's own limit).
pub(crate) const COMMENT_MAX: usize = 2_000;

pub(crate) async fn comment(
    config: &Config,
    issue_id: &str,
    body: &str,
    request_key: &str,
) -> Result<TrackerWriteOutcome, String> {
    let adapter = refused_before(
        &config.vault_path,
        "comment",
        (Some(issue_id), None),
        Some(json!({"bytes": body.trim().len()})),
        Some(request_key),
        enabled(config).and_then(|()| {
            let team = item_team(config, issue_id)?;
            tracker::adapter_for(&config.tracker, &team).map_err(|e| e.to_string())
        }),
    )?;
    comment_with(config, adapter.as_ref(), issue_id, body, request_key).await
}

/// Post the person's comment on a mirrored issue of the scoped team
/// (overhaul B5): journaled before the request under `request_key`; the
/// adapter reads the issue's team before it may send; a reply that is lost
/// after that is unconfirmed and never resent; a confirmed comment joins the
/// mirrored thread at once (the next pass reads it again by id).
pub(crate) async fn comment_with(
    config: &Config,
    remote: &dyn TrackerAdapter,
    issue_id: &str,
    body: &str,
    request_key: &str,
) -> Result<TrackerWriteOutcome, String> {
    let bytes = body.trim().len();
    let result = comment_unaudited(config, remote, issue_id, body, request_key).await;
    let comment = result.as_ref().ok().and_then(|o| o.id.clone());
    let key = result.as_ref().ok().and_then(|o| o.key.clone());
    audit_write(
        &config.vault_path,
        "comment",
        None,
        (Some(issue_id), key.as_deref()),
        None,
        Some(json!({"bytes": bytes, "commentId": comment})),
        Some(request_key),
        receipt(&result, comment.is_some()),
    );
    result
}

async fn comment_unaudited(
    config: &Config,
    remote: &dyn TrackerAdapter,
    issue_id: &str,
    body: &str,
    request_key: &str,
) -> Result<TrackerWriteOutcome, String> {
    enabled(config)?;
    let body = body.trim();
    if body.is_empty() {
        return Err("Write a comment first.".into());
    }
    if body.len() > COMMENT_MAX {
        return Err(format!(
            "A comment holds at most {COMMENT_MAX} bytes (accented letters and emoji take more than one)."
        ));
    }
    if !valid_id(request_key) {
        return Err("A comment needs a stable request key.".into());
    }
    let root = &config.vault_path;
    let _guard = root_lock(root).lock_owned().await;
    let _file = file_lock(root)?;
    let mut state = cached(config)?;
    focus_item(&mut state, issue_id);
    if remote.provider() != state.provider
        || remote.scope().await.map_err(|e| e.to_string())?.id != state.scope
    {
        return Err(TrackerError::OutOfScope.to_string());
    }
    let record = state
        .records
        .get(issue_id)
        .cloned()
        .ok_or("Sync the tracker before commenting on that issue.")?;
    let issue = record
        .issue
        .as_ref()
        .filter(|i| i.team_id == state.scope)
        .ok_or("Only an issue of the team Hollow syncs can take a comment.")?;
    if record.kind != "task" || record.state != "active" || record.remote_missing {
        return Err("Only an available tracker issue can take a comment.".into());
    }
    let key = issue.key.clone();
    let _lease = crate::automated_history::tracker_mirror_write_lease(root, &folders(&state, []))?;
    if let Some(entry) = state.comments.get(request_key) {
        if entry.issue_id != issue_id || entry.body != body {
            return Err("That comment key already names a different comment.".into());
        }
    } else {
        state.comments.insert(
            request_key.into(),
            CommentPost {
                issue_id: issue_id.into(),
                body: body.into(),
                attempted: false,
                comment_id: None,
                error: None,
            },
        );
        save(root, &state)?;
    }
    if !state.comments[request_key].attempted {
        state.comments.get_mut(request_key).unwrap().attempted = true;
        save(root, &state)?;
        let authorized = std::sync::atomic::AtomicBool::new(false);
        let authorize = || -> Result<(), TrackerError> {
            authorized.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        };
        let receipt = remote
            .create_issue_comment(issue_id, body, &authorize)
            .await;
        match receipt {
            Ok(change) if change.team_id == state.scope => {
                state.comments.get_mut(request_key).unwrap().comment_id =
                    Some(change.comment.id.clone());
                if let Some(mirrored) = state
                    .records
                    .get_mut(issue_id)
                    .and_then(|r| r.issue.as_mut())
                {
                    if !mirrored.comments.iter().any(|c| c.id == change.comment.id) {
                        mirrored.comments.push(change.comment);
                    }
                }
            }
            Ok(_) => {
                state.comments.get_mut(request_key).unwrap().error = Some(
                    "The tracker answered for another team; the comment is unconfirmed and will not be resent.".into(),
                );
            }
            // Nothing was sent before the team check passed: the comment
            // can be written again.
            Err(error) if !authorized.load(std::sync::atomic::Ordering::SeqCst) => {
                state.comments.remove(request_key);
                save(root, &state)?;
                return Err(error.to_string());
            }
            Err(error) => {
                state.comments.get_mut(request_key).unwrap().error = Some(format!(
                    "The comment is unconfirmed: {error}. It will not be resent; check the tracker before writing it again."
                ));
            }
        }
        save(root, &state)?;
        request_soon();
    }
    let entry = &state.comments[request_key];
    Ok(TrackerWriteOutcome {
        id: entry.comment_id.clone(),
        key: Some(key),
        path: Some(record.path.clone()),
        queued: false,
        // Attempted with no receipt and no error: Hollow stopped while
        // sending. It may have posted; it is never sent again.
        note: entry.error.clone().unwrap_or_else(|| {
            if entry.comment_id.is_some() {
                "Comment posted.".into()
            } else {
                "The comment is unconfirmed: Hollow stopped while sending it. It will not be resent; check the tracker before writing it again.".into()
            }
        }),
    })
}

/// Tests only: `create_named`'s checks, then the request through an adapter
/// aimed at the test stub instead of Linear (the key never leaves the process).
#[cfg(test)]
pub(crate) async fn create_with_team_for_test(
    config: &Config,
    url: &str,
    task: TrackerNewTask,
) -> Result<TrackerWriteOutcome, String> {
    let (team, key, input) = prepare_input(config, task)?;
    let shared = tracker::linear::LinearAdapter::with_url(
        url,
        config.tracker.team_id.as_deref().unwrap(),
        "lin_api_fixture_only_never_real",
    )
    .map_err(|e| e.to_string())?;
    let adapter = match shared.for_team(&team) {
        None => Box::new(shared) as Box<dyn TrackerAdapter>,
        Some(other) => other.map_err(|e| e.to_string())?,
    };
    create_with(config, adapter.as_ref(), &key, input).await
}
