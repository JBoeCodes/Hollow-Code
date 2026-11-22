//! The one tracker mirror renderer, shared by sync and migration.
//!
//! Existing files are patched, never recomposed. Only tracker-owned keys
//! and delimited body blocks change; vault-only frontmatter and prose keep
//! their bytes. JSON flow values are valid YAML and retain typed detail lists.

use super::{RelationKind, TrackerIssue, TrackerProject};
use crate::frontmatter::{self, KeyEdit};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(crate) type Values = BTreeMap<String, Value>;

fn string(value: Option<&str>) -> Value {
    Value::String(value.unwrap_or("").to_string())
}

/// Human-readable editable values, the baseline for the three-way merge.
pub(crate) fn values(
    issue: &TrackerIssue,
    projects: &BTreeMap<String, TrackerProject>,
    issues: &BTreeMap<String, TrackerIssue>,
) -> Values {
    let parent = issue.parent_id.as_ref().and_then(|id| issues.get(id));
    let project = issue.project_id.as_ref().and_then(|id| projects.get(id));
    BTreeMap::from([
        ("title".into(), json!(issue.title)),
        ("description".into(), json!(issue.description)),
        ("status".into(), json!(issue.state.name)),
        ("priority".into(), json!(priority(issue.priority))),
        (
            "assignee".into(),
            string(issue.assignee.as_ref().map(|u| u.name.as_str())),
        ),
        ("labels".into(), json!(issue.labels)),
        ("project".into(), string(project.map(|p| p.name.as_str()))),
        ("due".into(), string(issue.due.as_deref())),
        (
            "parent_key".into(),
            string(
                parent
                    .map(|p| p.key.as_str())
                    .or(issue.parent_id.as_deref()),
            ),
        ),
        (
            "estimate".into(),
            issue.estimate.map_or(Value::Null, |v| json!(v)),
        ),
        (
            "cycle".into(),
            json!(issue.cycle.as_ref().map(cycle_name).unwrap_or_default()),
        ),
        (
            "milestone".into(),
            string(issue.milestone.as_ref().map(|m| m.name.as_str())),
        ),
    ])
}

pub(crate) fn cycle_name(cycle: &super::TrackerCycle) -> String {
    let name = cycle
        .name
        .clone()
        .unwrap_or_else(|| format!("Cycle {}", cycle.number));
    format!(
        "{name} ({} to {})",
        cycle.starts_at.get(..10).unwrap_or(&cycle.starts_at),
        cycle.ends_at.get(..10).unwrap_or(&cycle.ends_at)
    )
}

pub(crate) fn priority(value: u8) -> &'static str {
    match value {
        1 => "Urgent",
        2 => "High",
        3 => "Medium",
        4 => "Low",
        _ => "",
    }
}

fn edits(content: &str, fields: &Values) -> Result<String, String> {
    let pairs: Vec<(&str, KeyEdit)> = fields
        .iter()
        .map(|(k, v)| {
            let edit = match v {
                Value::String(text) => KeyEdit::Quoted(text.clone()),
                _ => KeyEdit::Block(vec![format!("{k}: {v}")]),
            };
            (k.as_str(), edit)
        })
        .collect();
    let old = if content.is_empty() {
        "---\n# Tracker mirror\n---\n"
    } else {
        content
    };
    // Do not churn a differently quoted but semantically identical value.
    let current = local_keys(old);
    let pairs: Vec<_> = pairs
        .into_iter()
        .filter(|(k, _)| current.get(*k) != fields.get(*k))
        .collect();
    if pairs.is_empty() {
        Ok(old.to_string())
    } else {
        frontmatter::patch(old, &pairs, None)
    }
}

fn local_keys(content: &str) -> Values {
    frontmatter::parse(content)
        .pairs
        .into_iter()
        .map(|(k, raw)| {
            let value = if matches!(
                k.as_str(),
                "labels"
                    | "sub_issues"
                    | "blocked_by"
                    | "blocking"
                    | "related"
                    | "links"
                    | "attachments"
                    | "estimate"
                    | "comment_count"
            ) {
                serde_json::from_str(&raw).unwrap_or(Value::String(raw))
            } else {
                Value::String(raw)
            };
            (k, value)
        })
        .collect()
}

fn markers(name: &str) -> (String, String) {
    (
        format!("<!-- tracker:{name} -->"),
        format!("<!-- /tracker:{name} -->"),
    )
}

// Remote prose cannot terminate an owned block. Encoding the ampersand
// first makes decoding unambiguous even if the original text uses entities.
fn encode(text: &str) -> String {
    text.replace('&', "&amp;").replace("<!--", "&lt;!--")
}
fn decode(text: &str) -> String {
    text.replace("&lt;!--", "<!--").replace("&amp;", "&")
}

fn bounds(content: &str, name: &str) -> Result<Option<(usize, usize)>, String> {
    let (open, close) = markers(name);
    let starts: Vec<_> = content.match_indices(&open).collect();
    let ends: Vec<_> = content.match_indices(&close).collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([(a, _)], [(b, _)]) if b >= &(a + open.len()) => Ok(Some((a + open.len(), *b))),
        _ => Err(format!("The tracker's {name} block is damaged; restore its markers before syncing. The file was preserved.")),
    }
}

pub(crate) fn block(content: &str, name: &str) -> Result<Option<String>, String> {
    Ok(bounds(content, name)?.map(|(a, b)| {
        let inner = content[a..b].strip_prefix('\n').unwrap_or(&content[a..b]);
        decode(inner.strip_suffix('\n').unwrap_or(inner))
    }))
}

fn replace_block(content: &str, name: &str, value: &str) -> Result<String, String> {
    if block(content, name)?.as_deref() == Some(value) {
        return Ok(content.into());
    }
    let encoded = encode(value);
    match bounds(content, name)? {
        Some((a, b)) => Ok(format!("{}\n{encoded}\n{}", &content[..a], &content[b..])),
        None => {
            let (open, close) = markers(name);
            let separator = if content.ends_with('\n') { "" } else { "\n" };
            Ok(format!("{content}{separator}{open}\n{encoded}\n{close}\n"))
        }
    }
}

pub(crate) fn local_values(content: &str) -> Result<Values, String> {
    let mut all = local_keys(content);
    let description = block(content, "description")?.unwrap_or_default();
    all.insert("description".into(), json!(description));
    Ok(super::Field::ALL
        .into_iter()
        .map(|f| {
            let default = match f {
                super::Field::Labels => json!([]),
                super::Field::Estimate => Value::Null,
                _ => json!(""),
            };
            (f.key().into(), all.remove(f.key()).unwrap_or(default))
        })
        .collect())
}

pub(crate) fn vault_notes(content: &str) -> Result<String, String> {
    let mut body = frontmatter::parse(content).body;
    for name in ["description", "comments", "conflicts"] {
        if let Some((a, b)) = bounds(&body, name)? {
            let (open, close) = markers(name);
            let end = b + close.len();
            let end = if body[end..].starts_with('\n') {
                end + 1
            } else {
                end
            };
            body.replace_range(a - open.len()..end, "");
        }
    }
    Ok(body)
}

/// `project_path` is the vault-relative project mirror, when one exists.
#[allow(clippy::too_many_arguments)] // The shared renderer takes the full, explicit mirror context.
pub(crate) fn task(
    old: &str,
    provider: &str,
    issue: &TrackerIssue,
    values: &Values,
    project_path: Option<&str>,
    state: &str,
    pending: bool,
    conflicts: &str,
) -> Result<String, String> {
    let mut fields: Values = values
        .iter()
        .filter(|(key, _)| super::Field::from_key(key).is_some())
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    fields.remove("description");
    let relations = |kind| {
        issue
            .relations
            .iter()
            .filter(|r| r.kind == kind)
            .map(|r| r.key.clone())
            .collect::<Vec<_>>()
    };
    fields.extend(BTreeMap::from([
        ("status_type".into(), json!(issue.state.kind.as_str())),
        ("scope".into(), json!("work")),
        ("tracker".into(), json!(provider)),
        ("tracker_id".into(), json!(issue.id)),
        ("tracker_key".into(), json!(issue.key)),
        ("tracker_url".into(), json!(issue.url)),
        (
            "tracker_team".into(),
            json!(super::linear::team_key_of(&issue.team_id)
                .or_else(|| issue.key.rsplit_once('-').map(|(team, _)| team))
                .unwrap_or("")),
        ),
        (
            "tracker_project_id".into(),
            string(issue.project_id.as_deref()),
        ),
        ("tracker_state".into(), json!(state)),
        (
            "tracker_pending".into(),
            json!(if pending { "true" } else { "false" }),
        ),
        (
            "project_url".into(),
            json!(project_path
                .map(|p| format!("vault:{p}"))
                .unwrap_or_default()),
        ),
        (
            "creator".into(),
            string(issue.creator.as_ref().map(|u| u.name.as_str())),
        ),
        ("created_at".into(), json!(issue.created_at)),
        ("updated_at".into(), json!(issue.updated_at)),
        ("sub_issues".into(), json!(issue.children)),
        (
            "blocked_by".into(),
            json!(relations(RelationKind::BlockedBy)),
        ),
        ("blocking".into(), json!(relations(RelationKind::Blocks))),
        (
            "related".into(),
            json!(issue
                .relations
                .iter()
                .filter(|r| matches!(r.kind, RelationKind::Related | RelationKind::DuplicateOf))
                .map(|r| r.key.clone())
                .collect::<Vec<_>>()),
        ),
        ("links".into(), json!(issue.links)),
        ("attachments".into(), json!(issue.attachments)),
        ("comment_count".into(), json!(issue.comments.len())),
    ]));
    let mut text = edits(old, &fields)?;
    let prior = frontmatter::parse(old);
    if state != "active"
        || issue.state.kind.is_closed()
        || prior.get("tracker_state") != Some("active")
        || prior
            .get("status_type")
            .and_then(super::StateKind::parse)
            .is_none_or(|kind| kind.is_closed())
    {
        text = clear_claim(&text)?;
    }
    text = replace_block(
        &text,
        "description",
        values
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    let comments = issue
        .comments
        .iter()
        .map(|c| {
            format!(
                "### {}, {}\n{}",
                c.author.as_deref().unwrap_or("Unknown author"),
                c.created_at,
                c.body
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    text = replace_block(&text, "comments", &comments)?;
    // An empty conflicts block is intentional: removing the note later must
    // not remove any adjacent vault-only prose.
    replace_block(&text, "conflicts", conflicts)
}

/// Unavailable records can outlive their cached remote snapshot. Mark
/// only state and conflicts in that case, keeping the bound file intact.
pub(crate) fn unavailable_record(old: &str, conflicts: &str) -> Result<String, String> {
    let next = edits(
        old,
        &Values::from([
            ("tracker_state".into(), json!("moved")),
            ("tracker_pending".into(), json!(false)),
        ]),
    )?;
    replace_block(&clear_claim(&next)?, "conflicts", conflicts)
}

fn clear_claim(text: &str) -> Result<String, String> {
    frontmatter::patch(
        text,
        &[
            (crate::vault_tasks::CLAIMED_BY_KEY, KeyEdit::Remove),
            (crate::vault_tasks::CLAIMED_AT_KEY, KeyEdit::Remove),
        ],
        None,
    )
}

pub(crate) fn project_values(project: &TrackerProject) -> Values {
    BTreeMap::from([
        ("title".into(), json!(project.name)),
        ("description".into(), json!(project.description)),
        ("status".into(), json!(project.status.name)),
        ("target_date".into(), string(project.target_date.as_deref())),
        ("start_date".into(), string(project.start_date.as_deref())),
        (
            "lead".into(),
            string(project.lead.as_ref().map(|u| u.name.as_str())),
        ),
    ])
}

pub(crate) fn local_project_values(content: &str) -> Result<Values, String> {
    let mut all = local_keys(content);
    all.insert(
        "description".into(),
        json!(block(content, "description")?.unwrap_or_default()),
    );
    Ok([
        "title",
        "description",
        "status",
        "target_date",
        "start_date",
        "lead",
    ]
    .into_iter()
    .map(|key| (key.into(), all.remove(key).unwrap_or(json!(""))))
    .collect())
}

pub(crate) fn patch_local(old: &str, fields: &Values) -> Result<String, String> {
    let edits: Vec<_> = fields
        .iter()
        .map(|(key, value)| {
            let edit = match value {
                Value::Null => frontmatter::KeyEdit::Remove,
                Value::String(value) => frontmatter::KeyEdit::Quoted(value.clone()),
                _ => frontmatter::KeyEdit::Quoted(value.to_string()),
            };
            (key.as_str(), edit)
        })
        .collect();
    frontmatter::patch(old, &edits, None)
}

pub(crate) fn project_local(
    old: &str,
    values: &Values,
    pending: bool,
    conflicts: &str,
) -> Result<String, String> {
    let mut fields = values.clone();
    fields.remove("description");
    fields.insert("tracker_pending".into(), json!(pending));
    let text = edits(old, &fields)?;
    let text = replace_block(
        &text,
        "description",
        values
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or(""),
    )?;
    replace_block(&text, "conflicts", conflicts)
}

pub(crate) fn project(
    old: &str,
    provider: &str,
    project: &TrackerProject,
) -> Result<String, String> {
    project_with_notes(old, provider, project, false, "")
}

pub(crate) fn project_with_notes(
    old: &str,
    provider: &str,
    project: &TrackerProject,
    pending: bool,
    conflicts: &str,
) -> Result<String, String> {
    let fields = BTreeMap::from([
        ("title".into(), json!(project.name)),
        ("scope".into(), json!("work")),
        ("status".into(), json!(project.status.name)),
        ("status_type".into(), json!(project.status.kind)),
        ("target_date".into(), string(project.target_date.as_deref())),
        ("start_date".into(), string(project.start_date.as_deref())),
        (
            "lead".into(),
            string(project.lead.as_ref().map(|u| u.name.as_str())),
        ),
        ("tracker".into(), json!(provider)),
        ("tracker_project".into(), json!(project.id)),
        ("tracker_state".into(), json!("active")),
        ("tracker_url".into(), json!(project.url)),
        (
            "tracker_team".into(),
            json!(project
                .team_ids
                .iter()
                .find_map(|id| super::linear::team_key_of(id))
                .unwrap_or("")),
        ),
        (
            "tracker_pending".into(),
            json!(if pending { "true" } else { "false" }),
        ),
    ]);
    let text = replace_block(&edits(old, &fields)?, "description", &project.description)?;
    replace_block(&text, "conflicts", conflicts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_preserve_notes_and_remote_marker_text_round_trips() {
        let old = "---\ntitle: hi\nunknown:\n  - keep\n---\nMy notes.\n";
        let value = "a &lt;!--\n<!-- /tracker:description -->\nb";
        let next = replace_block(old, "description", value).unwrap();
        assert!(next.starts_with(old));
        assert_eq!(block(&next, "description").unwrap().as_deref(), Some(value));
        assert_eq!(replace_block(&next, "description", value).unwrap(), next);
        assert!(replace_block("<!-- tracker:description -->\nbroken", "description", "x").is_err());
    }
}
