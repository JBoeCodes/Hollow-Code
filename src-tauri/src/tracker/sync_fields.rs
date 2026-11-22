//! Resolve editable mirror values against the scoped team's metadata.
//! Never accept an arbitrary state, assignee, label, project, cycle or
//! milestone id from frontmatter. Ambiguous names are refused, not guessed.

use super::mirror::Values;
use super::{ipc::TrackerTeamMeta, *};
use serde_json::Value;
use std::collections::BTreeMap;

fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    if value.is_null() {
        return Ok("");
    }
    value
        .as_str()
        .ok_or_else(|| format!("The tracker field {field} must be text."))
}

fn resolve<'a>(
    value: &str,
    field: &str,
    choices: impl Iterator<Item = (&'a str, &'a str)>,
) -> Result<Option<String>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    let choices: Vec<_> = choices.collect();
    if let Some((id, _)) = choices.iter().find(|(id, _)| *id == value) {
        return Ok(Some((*id).into()));
    }
    let matches: Vec<_> = choices
        .iter()
        .filter(|(_, name)| name.eq_ignore_ascii_case(value.trim()))
        .collect();
    match matches.as_slice() {
        [(id, _)] => Ok(Some((*id).to_string())),
        [] => Err(format!(
            "The tracker field {field} has no matching choice in this team."
        )),
        _ => Err(format!(
            "The tracker field {field} is ambiguous; choose its id."
        )),
    }
}

/// A person by id, then by exact name, then by anything else they answer
/// to ([`TrackerUser::answers_to`]: display name, email, the email's local
/// part), with [`resolve`]'s errors.
fn resolve_member(
    value: &str,
    field: &str,
    members: &[TrackerUser],
) -> Result<Option<String>, String> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    if let Some(user) = members.iter().find(|u| u.id == value) {
        return Ok(Some(user.id.clone()));
    }
    let named: Vec<&TrackerUser> = members
        .iter()
        .filter(|u| u.name.eq_ignore_ascii_case(value.trim()))
        .collect();
    let hits = if named.is_empty() {
        members.iter().filter(|u| u.answers_to(value)).collect()
    } else {
        named
    };
    resolve(
        value,
        field,
        hits.iter().map(|u| (u.id.as_str(), value.trim())),
    )
}

pub(crate) fn patch(
    values: &Values,
    meta: &TrackerTeamMeta,
    projects: &BTreeMap<String, TrackerProject>,
    issues: &BTreeMap<String, TrackerIssue>,
    current_project: Option<&str>,
) -> Result<IssuePatch, String> {
    let mut patch = IssuePatch::default();
    let project = if let Some(value) = values.get("project") {
        resolve(
            text(value, "project")?,
            "project",
            projects
                .values()
                .filter(|p| p.team_ids.contains(&meta.team.id))
                .map(|p| (p.id.as_str(), p.name.as_str())),
        )?
    } else {
        current_project.map(str::to_string)
    };
    for (key, value) in values {
        let field =
            Field::from_key(key).ok_or_else(|| "This tracker field is read-only.".to_string())?;
        match field {
            Field::Title => {
                let value = text(value, key)?;
                if value.trim().is_empty() {
                    return Err("A task needs a title.".into());
                }
                patch.title = Some(value.into());
            }
            Field::Description => patch.description = Some(text(value, key)?.into()),
            Field::Status => {
                patch.state_id = Some(
                    resolve(
                        text(value, key)?,
                        key,
                        meta.states.iter().map(|s| (s.id.as_str(), s.name.as_str())),
                    )?
                    .ok_or("A task needs a status.")?,
                )
            }
            Field::Priority => {
                let number = if let Some(n) = value.as_u64() {
                    Some(n)
                } else {
                    let word = text(value, key)?;
                    (0..=4)
                        .find(|n| super::mirror::priority(*n).eq_ignore_ascii_case(word))
                        .map(u64::from)
                };
                patch.priority = Some(
                    number
                        .filter(|n| *n <= 4)
                        .ok_or("Priority must be empty, Urgent, High, Medium or Low.")?
                        as u8,
                );
            }
            Field::Assignee => {
                patch.assignee_id = Some(resolve_member(text(value, key)?, key, &meta.members)?)
            }
            Field::Labels => {
                let labels = value
                    .as_array()
                    .ok_or("Labels must be a list of names or ids.")?;
                let mut ids = Vec::new();
                for label in labels {
                    let id = resolve(
                        text(label, key)?,
                        key,
                        meta.labels.iter().map(|l| (l.id.as_str(), l.name.as_str())),
                    )?
                    .ok_or("A label cannot be empty.")?;
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
                patch.label_ids = Some(ids);
            }
            Field::Project => {
                patch.project_id = Some(resolve(
                    text(value, key)?,
                    key,
                    projects
                        .values()
                        .filter(|p| p.team_ids.contains(&meta.team.id))
                        .map(|p| (p.id.as_str(), p.name.as_str())),
                )?)
            }
            Field::Due => {
                let day = text(value, key)?;
                if !day.is_empty()
                    && (day.len() != 10
                        || chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").is_err())
                {
                    return Err("Due must be a date in YYYY-MM-DD form.".into());
                }
                patch.due = Some((!day.is_empty()).then(|| day.into()));
            }
            Field::Parent => {
                patch.parent_id = Some(resolve(
                    text(value, key)?,
                    key,
                    issues
                        .values()
                        .filter(|i| i.team_id == meta.team.id)
                        .map(|i| (i.id.as_str(), i.key.as_str())),
                )?)
            }
            Field::Estimate => {
                patch.estimate = Some(if value.is_null() || value.as_str() == Some("") {
                    None
                } else {
                    Some(
                        value
                            .as_f64()
                            .filter(|n| n.is_finite() && *n >= 0.0)
                            .ok_or("Estimate must be an empty value or a non-negative number.")?,
                    )
                })
            }
            Field::Cycle => {
                let choices: Vec<_> = meta
                    .cycles
                    .iter()
                    .map(|c| (c.id.as_str(), super::mirror::cycle_name(c)))
                    .collect();
                patch.cycle_id = Some(resolve(
                    text(value, key)?,
                    key,
                    choices.iter().map(|(id, name)| (*id, name.as_str())),
                )?)
            }
            Field::Milestone => {
                let id = resolve(
                    text(value, key)?,
                    key,
                    meta.milestones
                        .iter()
                        .filter(|m| m.project_id.as_deref() == project.as_deref())
                        .map(|m| (m.id.as_str(), m.name.as_str())),
                )?;
                patch.milestone_id = Some(id);
            }
        }
    }
    Ok(patch)
}

pub(crate) fn normalize(values: &Values) -> Result<Values, String> {
    values
        .iter()
        .map(|(key, value)| {
            let field = Field::from_key(key)
                .or_else(|| serde_json::from_value::<Field>(Value::String(key.clone())).ok())
                .ok_or("This tracker field is read-only.")?;
            Ok((field.key().into(), value.clone()))
        })
        .collect()
}

pub(crate) fn project_patch(
    values: &Values,
    meta: &TrackerTeamMeta,
) -> Result<ProjectPatch, String> {
    let mut patch = ProjectPatch::default();
    for (key, value) in values {
        let value = text(value, key)?;
        match key.as_str() {
            "title" => {
                if value.trim().is_empty() {
                    return Err("A project needs a title.".into());
                }
                patch.name = Some(value.into());
            }
            "description" => patch.description = Some(value.into()),
            "lead" => {
                patch.lead_id = Some(resolve(
                    value,
                    key,
                    meta.members
                        .iter()
                        .map(|u| (u.id.as_str(), u.name.as_str())),
                )?)
            }
            "start_date" | "target_date" => {
                if !value.is_empty()
                    && (value.len() != 10
                        || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err())
                {
                    return Err("Project dates must use YYYY-MM-DD form.".into());
                }
                let date = (!value.is_empty()).then(|| value.into());
                if key == "start_date" {
                    patch.start_date = Some(date);
                } else {
                    patch.target_date = Some(date);
                }
            }
            _ => return Err("That project field is read-only in the local mirror.".into()),
        }
    }
    Ok(patch)
}

pub(crate) fn validate_new(
    input: &NewIssue,
    meta: &TrackerTeamMeta,
    projects: &BTreeMap<String, TrackerProject>,
    issues: &BTreeMap<String, TrackerIssue>,
) -> Result<(), String> {
    let mut values = BTreeMap::from([
        ("title".into(), Value::String(input.title.clone())),
        ("labels".into(), serde_json::json!(input.label_ids)),
    ]);
    for (key, id) in [
        ("status", &input.state_id),
        ("assignee", &input.assignee_id),
        ("project", &input.project_id),
        ("parent_key", &input.parent_id),
        ("cycle", &input.cycle_id),
        ("milestone", &input.milestone_id),
        ("due", &input.due),
    ] {
        if let Some(id) = id {
            values.insert(key.into(), Value::String(id.clone()));
        }
    }
    if let Some(priority) = input.priority {
        values.insert("priority".into(), serde_json::json!(priority));
    }
    if let Some(estimate) = input.estimate {
        values.insert("estimate".into(), serde_json::json!(estimate));
    }
    patch(&values, meta, projects, issues, None).map(|_| ())
}
