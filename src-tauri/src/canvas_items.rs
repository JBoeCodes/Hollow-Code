//! Host-confirmed canvas copies. No bridge or agent action can invoke this path.
use crate::{
    shared_docs, vault,
    workspace_canvases::{self, Canvas, Scope},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasItemSource {
    pub canvas: Canvas,
    pub project: Option<String>,
    pub project_url: Option<String>,
}

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct CanvasPlanItems {
    pub workspace_id: String,
    pub id: String,
    pub revision: u64,
    pub heading: String,
    pub rows: Vec<(u32, String)>,
    /// None previews. Some confirms exactly that prospective document.
    pub expected_preview: Option<String>,
}

pub fn source(v: &Path, app: &Path, workspace: &str, id: &str) -> Result<CanvasItemSource, String> {
    if workspace == "quick-chats" || workspace == "main" {
        return Err("Save this canvas in a project workspace first.".into());
    }
    crate::workspaces::summary(v, workspace)?;
    let brief = vault::read_note(v, &format!("workspaces/{workspace}/WORKSPACE.md"))?;
    let (fields, _) = crate::frontmatter::parse_map(&brief.content);
    let project = crate::brief_read::project_of(v, &fields);
    if project.is_none()
        && ["tracker_project_path", "notion_project_path", "project_url"]
            .iter()
            .any(|key| {
                fields
                    .get(*key)
                    .is_some_and(|value| !value.trim().is_empty())
            })
    {
        return Err("The workspace's linked project is unavailable locally. Restore its project note before adding items.".into());
    }
    let canvas = workspace_canvases::read(
        v,
        app,
        Scope {
            workspace_id: Some(workspace),
            session_key: None,
        },
        id,
    )?
    .ok_or("The saved canvas is unavailable.")?;
    Ok(CanvasItemSource {
        canvas,
        project: project.as_ref().map(|p| p.1.clone()),
        project_url: project.map(|p| format!("vault:{}", p.0.trim_start_matches("project:"))),
    })
}

pub(crate) fn append_plan(current: &str, request: &CanvasPlanItems) -> Result<String, String> {
    let heading = request.heading.trim();
    if current.len() > 1024 * 1024
        || request
            .expected_preview
            .as_ref()
            .is_some_and(|p| p.len() > 1024 * 1024)
        || heading.is_empty()
        || heading.chars().count() > 120
        || heading.chars().any(char::is_control)
        || heading.contains(['<', '>', '`'])
        || heading.trim_end_matches('#').is_empty()
        || heading.trim_end_matches('#').ends_with(char::is_whitespace)
        || request.rows.is_empty()
        || request.rows.len() > 50
        || request.revision == 0
        || request.id.is_empty()
        || request.id.len() > 64
        || !request
            .id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(
            "Choose 1–50 items and a plain heading of at most 120 characters; Plan must fit 1 MB."
                .into(),
        );
    }
    let mut seen = std::collections::BTreeSet::new();
    let newline = if current.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut added = String::new();
    for (index, text) in &request.rows {
        if !seen.insert(index)
            || *index > 100_000
            || text.trim().is_empty()
            || text.chars().count() > 500
            || text.chars().any(char::is_control)
        {
            return Err(
                "Each selected item needs unique identity and one line of 1–500 characters.".into(),
            );
        }
        let marker = format!(
            "<!-- canvas-item:{}:v{}:{} -->",
            request.id, request.revision, index
        );
        if current.contains(&marker) {
            continue;
        }
        let mut escaped = String::new();
        for ch in text.trim().chars() {
            if "\\`*_[]<>#!".contains(ch) {
                escaped.push('\\');
            }
            escaped.push(ch);
        }
        added.push_str(&format!(
            "- [ ] {escaped} (canvas {} v{}) {marker}{newline}",
            request.id, request.revision
        ));
    }
    if added.is_empty() {
        return Ok(current.into());
    }
    let lines: Vec<_> = current.split_inclusive('\n').collect();
    let mut offsets = Vec::new();
    let mut offset = 0;
    for line in &lines {
        offsets.push(offset);
        offset += line.len();
    }
    let mut heads = Vec::new();
    crate::plan_rows::visit_rows(current, |n, row| {
        let line = row.hashed.trim_start();
        let level = line.bytes().take_while(|b| *b == b'#').count();
        if !row.code && (1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' ') {
            let text = line[level + 1..].trim();
            let stripped = text.trim_end_matches('#');
            let title = if stripped.len() != text.len() && stripped.ends_with(char::is_whitespace) {
                stripped.trim_end()
            } else {
                text
            };
            heads.push((offsets[n as usize - 1], level, title.to_owned()));
        }
        true
    });
    let matches: Vec<_> = heads
        .iter()
        .enumerate()
        .filter(|(_, h)| h.2 == heading)
        .collect();
    if matches.len() > 1 {
        return Err("More than one Plan heading has that name. Choose a unique heading.".into());
    }
    let (at, prefix) = if let Some((i, h)) = matches.first() {
        (
            heads
                .iter()
                .skip(i + 1)
                .find(|next| next.1 <= h.1)
                .map_or(current.len(), |h| h.0),
            String::new(),
        )
    } else {
        (current.len(), format!("## {heading}{newline}{newline}"))
    };
    // Never put copied rows inside an unfinished fenced block/frontmatter.
    let mut fence: Option<(char, usize)> = None;
    let mut frontmatter = false;
    for (i, line) in current[..at].lines().enumerate() {
        if i == 0 && line.trim_start_matches('\u{feff}') == "---" {
            frontmatter = current.lines().skip(1).any(|l| l == "---" || l == "...");
            continue;
        }
        if frontmatter {
            if line == "---" || line == "..." {
                frontmatter = false;
            }
            continue;
        }
        if let Some((ch, len, rest)) = crate::plan_rows::fence_run(line) {
            match fence {
                Some((old, count)) if old == ch && len >= count && rest.trim().is_empty() => {
                    fence = None
                }
                None if ch != '`' || !rest.contains('`') => fence = Some((ch, len)),
                _ => {}
            }
        }
    }
    if fence.is_some() || frontmatter {
        return Err(
            "Close the unfinished code fence or frontmatter in Plan before adding items.".into(),
        );
    }
    let before = &current[..at];
    let gap = if before.ends_with(newline) || before.is_empty() {
        newline.to_string()
    } else {
        format!("{newline}{newline}")
    };
    let result = format!("{before}{gap}{prefix}{added}{newline}{}", &current[at..]);
    if result.len() > 1024 * 1024 {
        return Err("The resulting Plan would exceed 1 MB.".into());
    }
    Ok(result)
}

pub fn plan(v: &Path, app: &Path, request: &CanvasPlanItems) -> Result<String, String> {
    let saved = source(v, app, &request.workspace_id, &request.id)?;
    if saved.canvas.revision != request.revision {
        return Err(
            "The saved canvas changed. Close and reopen Add items to review its current version."
                .into(),
        );
    }
    // read_note and update_doc both read exact UTF-8 bytes through safe_join.
    // A missing Plan is an explicit refusal, never an implicit file creation.
    let current = vault::read_note(v, &format!("workspaces/{}/PLAN.md", request.workspace_id))
        .map_err(|e| format!("PLAN.md could not be read. Restore or create it in this workspace, then preview again: {e}"))?.content;
    let preview = append_plan(&current, request)?;
    if request.expected_preview.is_none() {
        return Ok(preview);
    }
    if preview == current && request.expected_preview.as_deref() == Some(current.as_str()) {
        return Ok(current);
    }
    shared_docs::update_doc(v, &request.workspace_id, "PLAN.md", |current| {
        let next = append_plan(current, request)?;
        if request.expected_preview.as_deref() != Some(next.as_str()) {
            return Err(
                "Plan changed. Nothing was overwritten. Preview Plan again, then Confirm.".into(),
            );
        }
        Ok(next)
    })
    .map(|n| n.content)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> CanvasPlanItems {
        CanvasPlanItems {
            workspace_id: "w".into(),
            id: "rollout".into(),
            revision: 1,
            heading: "Release checks".into(),
            rows: (0..5).map(|n| (n, format!("Step {n}"))).collect(),
            expected_preview: None,
        }
    }
    #[test]
    fn an_unlinked_workspace_has_no_project_destination() {
        let dir = tempfile::tempdir().unwrap();
        let app = tempfile::tempdir().unwrap();
        let ws =
            crate::workspaces::create(dir.path(), "Unlinked workspace", "Test copies").unwrap();
        workspace_canvases::record(
            dir.path(),
            app.path(),
            Scope {
                workspace_id: Some(&ws.id),
                session_key: None,
            },
            "<hc-canvas id=\"rollout\" title=\"Rollout\"><ul><li>First</li></ul></hc-canvas>",
            None,
            None,
            None,
            "2026-10-25T23:00:00Z",
        )
        .unwrap();
        let result = source(dir.path(), app.path(), &ws.id, "rollout").unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap()["project"],
            serde_json::Value::Null
        );
    }
    #[test]
    fn five_rows_go_under_the_chosen_heading_once() {
        let r = request();
        let before = "# Plan\n\n## Release checks\nExisting text\n\n## Later\nKeep this\n";
        let after = append_plan(before, &r).unwrap();
        assert_eq!(after.matches("- [ ]").count(), 5);
        assert!(after.find("Step 4").unwrap() < after.find("## Later").unwrap());
        assert!(after.contains("Existing text"));
        assert!(after.contains("canvas rollout v1"));
        assert_eq!(append_plan(&after, &r).unwrap(), after);
    }
    #[test]
    fn ambiguous_heading_and_multiline_rows_are_refused() {
        assert!(append_plan("## Release checks\n## Release checks\n", &request()).is_err());
        let mut r = request();
        r.rows[0].1 = "Step\n## injected".into();
        assert!(append_plan("# Plan\n", &r).is_err());
    }

    #[test]
    fn closing_hashes_match_and_heading_markers_cannot_hide_copies() {
        let mut r = request();
        let saved = append_plan("# Plan\n## Release checks ##\n\n## Later\n", &r).unwrap();
        assert_eq!(saved.matches("## Release checks").count(), 1);
        assert!(saved.find("Step 4").unwrap() < saved.find("## Later").unwrap());
        r.heading = "<!-- canvas-item:rollout:v1:0 -->".into();
        assert!(append_plan("# Plan\n", &r).is_err());
        r.heading = "Checks #".into();
        assert!(append_plan("# Plan\n", &r).is_err());
        let r = request();
        assert!(append_plan("---\ntitle: Plan\n...\n# Plan\n", &r)
            .unwrap()
            .contains("Step 4"));
        let saved = append_plan("## Release checks\r\n\r\n## Later\r\n", &r).unwrap();
        assert!(saved.find("Step 4").unwrap() < saved.find("## Later").unwrap());
        assert!(!saved.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn fenced_headings_are_not_destinations_and_unclosed_fences_are_refused() {
        let r = request();
        let saved = append_plan(
            "# Plan\n```md\n## Release checks\n```\n\n## Release checks\n",
            &r,
        )
        .unwrap();
        assert!(saved.find("Step 0").unwrap() > saved.rfind("## Release checks").unwrap());
        assert!(append_plan("# Plan\n```\nunfinished\n", &r).is_err());
        let mut r = request();
        r.rows = vec![(0, "<script> & [link](bad)".into())];
        let saved = append_plan("# Plan\r\n", &r).unwrap();
        assert!(saved.contains("\\<script\\>"));
        assert!(!saved.replace("\r\n", "").contains('\n'));
    }
}
