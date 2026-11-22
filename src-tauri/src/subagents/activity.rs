//! Bounded, read-only activity for the Subagents view. The step spine is
//! written from stream events; no transcript polling or model call is needed.
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubagentGroup {
    pub id: String,
    pub title: String,
    pub parts: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ActivityStep {
    pub name: String,
    pub target: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ReportedProgress {
    pub completed: u32,
    pub total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SubagentActivity {
    pub current: Option<ActivityStep>,
    pub recent: Vec<ActivityStep>,
    pub progress: Option<ReportedProgress>,
    pub outcome: Option<String>,
    pub artifacts: Vec<String>,
    pub reported_prs: Vec<String>,
}

fn short(text: &str) -> String {
    crate::claude_runner::redact(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(180)
        .collect()
}

/// Only recognized display fields, never arbitrary tool payloads or results.
pub(super) fn target(input: &Value) -> String {
    [
        "file_path",
        "path",
        "command",
        "cmd",
        "query",
        "description",
    ]
    .iter()
    .find_map(|k| input.get(k).and_then(Value::as_str))
    .map(short)
    .unwrap_or_default()
}

pub(crate) fn progress(input: &Value) -> Option<ReportedProgress> {
    let items = input
        .get("plan")
        .or_else(|| input.get("todos"))
        .or_else(|| input.get("items"))?
        .as_array()?;
    if items.is_empty() || items.len() > 10_000 {
        return None;
    }
    Some(ReportedProgress {
        completed: items
            .iter()
            .filter(|v| {
                v.get("status").and_then(Value::as_str) == Some("completed")
                    || v.get("completed").and_then(Value::as_bool) == Some(true)
            })
            .count() as u32,
        total: items.len() as u32,
    })
}

pub(super) fn read(conn: &Connection, key: &str, status: &str) -> Result<SubagentActivity, String> {
    if status == "queued" {
        return Ok(SubagentActivity::default());
    }
    let ord: Option<i64> = conn
        .query_row(
            "SELECT MAX(ord) FROM turns WHERE session_key = ?1",
            [key],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    let Some(ord) = ord else {
        return Ok(SubagentActivity::default());
    };
    // Include unfinished steps: finalization closes them after the turn, and
    // that write produces no event. Never retain an interim interrupted label.
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, SubagentActivity>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let cache_key = if status != "running" {
        conn.path().filter(|p| !p.is_empty()).map(|path| {
            conn.query_row("SELECT COALESCE(MAX(id), 0), (SELECT COUNT(*) FROM steps WHERE session_key = ?1 AND turn_ord = ?2 AND status = 'running') FROM events WHERE session_key = ?1 AND turn_ord = ?2", params![key, ord], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))
                .map(|(id, pending)| format!("{path}|{key}|{ord}|{status}|{id}|{pending}"))
        }).transpose().map_err(|e| e.to_string())?
    } else {
        None
    };
    if let Some(cached) = cache_key
        .as_ref()
        .and_then(|k| cache.lock().ok()?.get(k).cloned())
    {
        return Ok(cached);
    }
    let mut out = SubagentActivity::default();
    let mut stmt = conn.prepare("SELECT kind, name, status, checkpoint FROM steps WHERE session_key = ?1 AND turn_ord = ?2 ORDER BY seq DESC LIMIT 8").map_err(|e| e.to_string())?;
    let steps = stmt
        .query_map(params![key, ord], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    for (index, step) in steps.enumerate() {
        let (kind, name, state, checkpoint) = step.map_err(|e| e.to_string())?;
        let checkpoint: Value = checkpoint
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let input: Value = checkpoint
            .get("input")
            .and_then(Value::as_str)
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        let step = ActivityStep {
            name: name.unwrap_or_else(|| "Thinking".into()),
            target: checkpoint
                .get("displayTarget")
                .and_then(Value::as_str)
                .map(short)
                .unwrap_or_else(|| target(&input)),
            status: if state == "running" && status != "running" {
                "interrupted".into()
            } else {
                state.clone()
            },
        };
        if index == 0 && state == "running" && status == "running" {
            out.current = Some(step);
        } else if kind != "model" && out.recent.len() < 3 {
            out.recent.push(step);
        }
    }
    // Find the latest reported plan even when many tools ran after it.
    let plan: Option<String> = conn.query_row(
        "SELECT payload FROM events WHERE session_key = ?1 AND turn_ord = ?2 AND (kind = 'plan' OR (kind = 'toolUse' AND json_extract(payload, '$.name') IN ('TodoWrite', 'update_plan', 'functions.update_plan', 'todo_list'))) ORDER BY id DESC LIMIT 1",
        params![key, ord], |r| r.get(0)).optional().map_err(|e| e.to_string())?;
    out.progress = plan
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| {
            serde_json::from_value::<ReportedProgress>(v.clone())
                .ok()
                .filter(|p| p.total > 0 && p.completed <= p.total)
                .or_else(|| progress(&v["input"]))
        });
    // Only named writes from this turn count as files, with an honest label
    // in the UI. Shell-created files have no such record and are omitted.
    let mut stmt = conn.prepare("SELECT checkpoint FROM steps WHERE session_key = ?1 AND turn_ord = ?2 AND kind = 'write' AND status = 'ok' ORDER BY seq DESC LIMIT 20").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![key, ord], |r| r.get::<_, Option<String>>(0))
        .map_err(|e| e.to_string())?;
    for text in rows {
        let value: Value = text
            .map_err(|e| e.to_string())?
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        for path in value
            .get("paths")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            if out.artifacts.len() < 3 && !out.artifacts.iter().any(|p| p == path) {
                out.artifacts.push(path.to_string());
            }
        }
    }
    if status != "running" && status != "queued" {
        let text: Option<String> = conn.query_row("SELECT json_extract(payload, '$.text') FROM transcripts WHERE session_key = ?1 AND turn_ord = ?2", params![key, ord], |r| r.get(0)).optional().map_err(|e| e.to_string())?.flatten();
        if let Some(text) = text {
            use crate::child_status::Outcome;
            let outcome = crate::child_status::parse(&text);
            if outcome != Outcome::Unknown {
                out.outcome = Some(short(&outcome.words()));
            }
            // Reported PR links are evidence of the agent's report, not a
            // claim that a PR exists or merged. No remote request is made.
            static PR: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let re = PR.get_or_init(|| regex::Regex::new(r"https://(?:github\.com/[^/\s)]+/[^/\s)]+/pull/\d+|gitlab\.com/[^\s)]+/-/merge_requests/\d+)").expect("PR pattern"));
            for m in re.find_iter(&text).take(3) {
                if !out.reported_prs.iter().any(|p| p == m.as_str()) {
                    out.reported_prs.push(m.as_str().to_string());
                }
            }
        }
    }
    if let (Some(key), Ok(mut cache)) = (cache_key, cache.lock()) {
        if cache.len() >= 512 {
            cache.clear();
        }
        cache.insert(key, out.clone());
    }
    Ok(out)
}

pub(super) fn join_swarm(row: &mut super::SubagentRow, swarms: &[crate::main_agent::Swarm]) {
    for swarm in swarms {
        if let Some((index, task)) = swarm.tasks.iter().enumerate().find(|(_, t)| {
            t.session
                .as_ref()
                .is_some_and(|k| row.conversations.contains(k))
        }) {
            let old =
                crate::chat_title::short_title(&crate::main_agent::swarm_part_header(swarm, index));
            let generated = crate::chat_title::short_title(&task.title);
            if row
                .title
                .as_deref()
                .is_none_or(|title| title == old || title == generated)
            {
                row.title = Some(task.title.clone());
            }
            row.group = Some(SubagentGroup {
                id: swarm.id.clone(),
                title: swarm.goal.clone(),
                parts: swarm.tasks.len() as u32,
            });
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reports_only_explicit_plans_and_bounded_targets() {
        assert_eq!(
            progress(&json!({"plan":[{"status":"completed"},{"status":"in_progress"}]})),
            Some(ReportedProgress {
                completed: 1,
                total: 2
            })
        );
        assert_eq!(progress(&json!({"todos":[]})), None);
        assert_eq!(
            progress(&json!({"items":[{"completed":true},{"completed":false}]})),
            Some(ReportedProgress {
                completed: 1,
                total: 2
            })
        );
        assert_eq!(
            target(&json!({"file_path":"src/日本語.ts", "content":"private body"})),
            "src/日本語.ts"
        );
        assert_eq!(target(&json!({"content":"private body"})), "");
    }

    #[test]
    fn latest_turn_only_and_finished_runs_never_show_live_steps() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE turns(session_key TEXT, ord INTEGER); CREATE TABLE events(session_key TEXT, turn_ord INTEGER, kind TEXT, payload TEXT, id INTEGER); CREATE TABLE transcripts(session_key TEXT, turn_ord INTEGER, payload TEXT);").unwrap();
        conn.execute_batch(crate::step_spine::SCHEMA).unwrap();
        conn.execute("INSERT INTO turns VALUES('s',1),('s',2)", [])
            .unwrap();
        conn.execute("INSERT INTO steps(session_key,turn_ord,seq,kind,name,started_at,status,checkpoint) VALUES('s',2,1,'tool','Read','now','running',?1)", [json!({"input":json!({"path":"current.rs"}).to_string()}).to_string()]).unwrap();
        conn.execute(
            "INSERT INTO transcripts VALUES('s',2,?1)",
            [json!({"text":"Report\nPartial: native check remains"}).to_string()],
        )
        .unwrap();
        let running = read(&conn, "s", "running").unwrap();
        assert_eq!(running.current.unwrap().target, "current.rs");
        let done = read(&conn, "s", "done").unwrap();
        assert!(done.current.is_none());
        assert_eq!(done.recent[0].status, "interrupted");
        assert_eq!(
            done.outcome.as_deref(),
            Some("Partial: native check remains")
        );
        assert_eq!(read(&conn, "s", "queued").unwrap().outcome, None);
        conn.execute(
            "INSERT INTO events VALUES('s',2,'toolUse',?1,1)",
            [
                json!({"name":"TodoWrite","input":{"truncated":true,"preview":"large plan"}})
                    .to_string(),
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO events VALUES('s',2,'plan',?1,2)",
            [json!({"completed":12,"total":20}).to_string()],
        )
        .unwrap();
        assert_eq!(
            read(&conn, "s", "running").unwrap().progress,
            Some(ReportedProgress {
                completed: 12,
                total: 20
            })
        );
    }
}
