//! Export a Result as Markdown (overnight F6): the person saves one of a
//! workspace's Results to a `.md` file they pick, outside Hollow, with where
//! it came from and its sources listed after it. Local only: nothing is sent.
//!
//! The file is the Result's own text (a Markdown Result as written; another
//! text kind inside a fenced block), then "About this Result" (workspace,
//! file, exact hash, review state, the run that last wrote it) and "Sources":
//! the workspace's `SOURCES.md` entries (or, without one, the files in its
//! `sources/` folder) and the notes and web links the document itself cites.
//! PDFs, images and other binary Results are refused with a plain reason;
//! they open in their own app.
//!
//! One Tauri command (the picker's), and the COO's `workspaces.export_result_markdown` (a confirm
//! card, a file that is not there yet: [`export_new`]).

use crate::workspace_results::{self, WorkResult};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Text Results larger than this are refused (they are not documents).
const MAX_TEXT: u64 = 5 * 1024 * 1024;

/// Text kinds by extension: Markdown as is, the rest in a fenced block.
const FENCED: &[&str] = &["txt", "csv", "tsv", "json", "yaml", "yml", "html"];

/// The first `n` characters of `s` (never splitting one).
fn head(s: &str, n: usize) -> &str {
    s.char_indices().nth(n).map_or(s, |(at, _)| &s[..at])
}

/// A fence longer than any backtick run in `body`.
fn fence_for(body: &str) -> String {
    let longest = body.split(|c| c != '`').map(str::len).max().unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}

/// What an export wrote.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Exported {
    pub target: String,
    pub sources: usize,
}

/// One source line: what it is and where it came from.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SourceLine {
    pub name: String,
    pub origin: Option<String>,
}

/// The workspace's sources: `SOURCES.md` headings with their `Origin:`
/// bullet, else the files in `sources/`. Missing both: none.
pub(crate) fn workspace_sources(dir: &Path) -> Vec<SourceLine> {
    if let Ok(text) = std::fs::read_to_string(dir.join("SOURCES.md")) {
        let mut lines = Vec::new();
        for block in format!("\n{text}").split("\n## ").skip(1) {
            let mut rows = block.lines();
            let name = rows.next().unwrap_or("").trim().to_string();
            if name.is_empty() {
                continue;
            }
            let origin = rows
                .map(str::trim)
                .find_map(|l| {
                    l.strip_prefix("- Origin:")
                        .or_else(|| l.strip_prefix("- **Origin:**"))
                        .map(|o| o.trim().to_string())
                })
                .filter(|o| !o.is_empty());
            lines.push(SourceLine { name, origin });
        }
        if !lines.is_empty() {
            return lines;
        }
    }
    let mut files: Vec<String> = std::fs::read_dir(dir.join("sources"))
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
        .into_iter()
        .map(|name| SourceLine {
            name: format!("sources/{name}"),
            origin: None,
        })
        .collect()
}

/// Notes (`[[wikilinks]]`) and web links the document cites, in order, once.
pub(crate) fn cited(body: &str) -> Vec<String> {
    // Links inside fenced code are examples, not citations.
    let mut prose = String::new();
    let mut in_fence = false;
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            in_fence = !in_fence;
            continue;
        }
        if !in_fence {
            prose.push_str(line);
            prose.push('\n');
        }
    }
    let body = prose.as_str();
    let mut out: Vec<String> = Vec::new();
    let mut push = |s: String| {
        if !s.is_empty() && !out.contains(&s) {
            out.push(s);
        }
    };
    let mut rest = body;
    while let Some(at) = rest.find("[[") {
        rest = &rest[at + 2..];
        if let Some(end) = rest.find("]]") {
            let inner = &rest[..end];
            let target = inner
                .split(['|', '#'])
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches('\\');
            if !target.contains('\n') {
                push(format!("[[{target}]]"));
            }
            rest = &rest[end + 2..];
        } else {
            break;
        }
    }
    for word in body
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '<' | '>' | '"' | '`' | '*'))
    {
        let word = word.trim_start_matches(['[', '\'']);
        if word.starts_with("https://") || word.starts_with("http://") {
            push(
                word.trim_end_matches(['.', ',', ';', ':', ']', '!', '?', '\''])
                    .to_string(),
            );
        }
    }
    out
}

/// The exported Markdown. Pure.
pub(crate) fn render(
    result: &WorkResult,
    workspace: &str,
    body: &str,
    sources: &[SourceLine],
) -> String {
    let ext = Path::new(&result.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut out = String::new();
    if ext == "md" || ext == "markdown" {
        out.push_str(body.trim_end());
    } else {
        out.push_str(&format!("# {}\n\n", result.title));
        let fence = fence_for(body);
        out.push_str(&format!("{fence}{ext}\n{}\n{fence}", body.trim_end()));
    }
    out.push_str("\n\n---\n\n## About this Result\n\n");
    out.push_str(&format!("- Workspace: `{workspace}`\n"));
    out.push_str(&format!("- File: `{}`", result.name));
    if let Some(sha) = &result.sha256 {
        out.push_str(&format!(" (sha256 `{}`)", head(sha, 12)));
    }
    out.push('\n');
    let review = match result.review.as_str() {
        "accepted" => format!(
            "accepted{}",
            result
                .accepted_at
                .as_deref()
                .map(|a| format!(" on {}", head(a, 10)))
                .unwrap_or_default()
        ),
        "changed" => "changed since it was accepted".to_string(),
        "ready" => "ready for review".to_string(),
        other => other.to_string(),
    };
    out.push_str(&format!("- Review: {review}\n"));
    match &result.origin {
        Some(o) => out.push_str(&format!(
            "- Written by run `{}` ({}, {})\n",
            o.run_id,
            o.status,
            head(&o.finished_at, 10)
        )),
        None => out.push_str("- Written by: no run receipt names this file\n"),
    }
    out.push_str("\n## Sources\n\n");
    let cited = cited(body);
    if sources.is_empty() && cited.is_empty() {
        out.push_str(
            "None recorded: the workspace has no sources and the document cites nothing.\n",
        );
    }
    for s in sources {
        match &s.origin {
            Some(origin) => out.push_str(&format!("- {}: {}\n", s.name, origin)),
            None => out.push_str(&format!("- {}\n", s.name)),
        }
    }
    if !cited.is_empty() {
        if !sources.is_empty() {
            out.push('\n');
        }
        out.push_str("Cited in the document:\n\n");
        for c in cited {
            out.push_str(&format!("- {c}\n"));
        }
    }
    out
}

/// Write `result` (vault-relative `path` in workspace `id`) to `target`.
/// `exclusive` publishes without ever replacing a file ([`publish`]).
fn export(
    vault: &Path,
    id: &str,
    path: &str,
    target: &Path,
    exclusive: bool,
) -> Result<Exported, String> {
    let md = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    if !target.is_absolute() || !md {
        return Err("Pick where to save it with the file picker (a .md file).".into());
    }
    // Inside the vault it would be indexed and synced, and bypass History.
    let real_vault = vault.canonicalize().unwrap_or_else(|_| vault.to_path_buf());
    let real_parent = target
        .parent()
        .map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
    if target.starts_with(vault) || real_parent.is_some_and(|p| p.starts_with(&real_vault)) {
        return Err("Save it outside the vault (a file there would be indexed and synced). Pick another folder.".into());
    }
    let result = workspace_results::list(vault, id)?
        .into_iter()
        .find(|r| r.path == path)
        .ok_or("That Result isn't in this workspace any more. Refresh, then try again.")?;
    let ext = Path::new(&result.name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !(ext == "md" || ext == "markdown" || FENCED.contains(&ext.as_str())) {
        return Err(format!(
            "Only text Results export as Markdown; this one is a {} file. Use Show in Finder to copy it.",
            if ext.is_empty() { "binary".to_string() } else { ext }
        ));
    }
    if result.size > MAX_TEXT {
        return Err("That Result is too large to export as Markdown.".into());
    }
    let source = vault.join(&result.path);
    let bytes = std::fs::read(&source).map_err(|e| format!("Couldn't read the Result: {e}"))?;
    let body = String::from_utf8_lossy(&bytes).into_owned();
    let dir = crate::workspaces::folder(vault, id)?;
    let sources = workspace_sources(&dir);
    let text = render(&result, id, &body, &sources);
    let stamp = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let tmp = target.with_file_name(format!(
        ".hollow-result-{}-{stamp}.partial",
        std::process::id()
    ));
    {
        use std::io::Write;
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
            .map_err(|e| format!("Couldn't write the file: {e}"))?;
        if let Err(e) = out.write_all(text.as_bytes()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Couldn't write the file: {e}"));
        }
    }
    publish(&tmp, target, exclusive)?;
    Ok(Exported {
        target: target.to_string_lossy().to_string(),
        sources: sources.len() + cited(&body).len(),
    })
}

/// Move the finished temp file `tmp` to `target`. The person's save dialog
/// asked about replacing, so theirs renames over it; an `exclusive` publish
/// (the COO's, L13 review) hard-links it, which fails when the name exists
/// at that instant, so a file made after the early check is never replaced.
/// The temp file is gone either way.
pub(crate) fn publish(tmp: &Path, target: &Path, exclusive: bool) -> Result<(), String> {
    if !exclusive {
        return std::fs::rename(tmp, target).map_err(|e| {
            let _ = std::fs::remove_file(tmp);
            format!("Couldn't write the file: {e}")
        });
    }
    let linked = std::fs::hard_link(tmp, target);
    let _ = std::fs::remove_file(tmp);
    linked.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            format!(
                "{} already exists. Choose a file name that is not taken; nothing was written.",
                target.display()
            )
        } else {
            format!("Couldn't write the file: {e}")
        }
    })
}

/// [`export`] for the COO's `workspaces.export_result_markdown`: the person
/// confirmed `target` on a card, and it must be a file that is not there yet,
/// so an agent can never replace a file on this Mac.
pub(crate) fn export_new(
    vault: &Path,
    id: &str,
    path: &str,
    target: &Path,
) -> Result<Exported, String> {
    if std::fs::symlink_metadata(target).is_ok() {
        return Err(format!(
            "{} already exists. Choose a file name that is not taken; nothing was written.",
            target.display()
        ));
    }
    export(vault, id, path, target, true)
}

/// Save a workspace Result as Markdown at `target`, a file the person picked.
#[tauri::command]
pub async fn result_export_markdown(
    id: String,
    path: String,
    target: String,
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<Exported, String> {
    let vault = state.config.vault_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        export(&vault, &id, &path, &PathBuf::from(target), false)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace_results::{ResultChecks, ResultOrigin};

    fn result(name: &str, review: &str) -> WorkResult {
        WorkResult {
            path: format!("workspaces/w1/{name}"),
            name: name.into(),
            title: "Q3 plan".into(),
            kind: "document".into(),
            size: 10,
            modified: None,
            sha256: Some("abcdef0123456789".into()),
            review: review.into(),
            accepted_sha256: None,
            accepted_at: Some("2026-10-09T15:00:00Z".into()),
            ready_at: None,
            has_snapshot: false,
            acceptance: None,
            history: vec![],
            origin: Some(ResultOrigin {
                session_key: Some("s-1".into()),
                run_id: "run-7".into(),
                finished_at: "2026-10-09T14:00:00Z".into(),
                status: "success".into(),
                verification: "checked".into(),
            }),
            checks: ResultChecks {
                total: None,
                failed: None,
                unknown: None,
            },
            canvas: None,
        }
    }

    #[test]
    fn a_markdown_result_keeps_its_text_and_lists_origin_and_sources() {
        let body = "# Q3 plan\n\nSee [[Budget 2026|budget]] and https://example.com/report.\n";
        let sources = vec![
            SourceLine {
                name: "board-deck.pdf".into(),
                origin: Some("Notion page \"Board\"".into()),
            },
            SourceLine {
                name: "sources/notes.md".into(),
                origin: None,
            },
        ];
        let out = render(&result("outputs/plan.md", "accepted"), "w1", body, &sources);
        assert!(
            out.starts_with("# Q3 plan\n\nSee [[Budget 2026|budget]]"),
            "{out}"
        );
        assert!(out.contains("- Workspace: `w1`\n"));
        assert!(out.contains("- File: `outputs/plan.md` (sha256 `abcdef012345`)\n"));
        assert!(out.contains("- Review: accepted on 2026-10-09\n"));
        assert!(out.contains("- Written by run `run-7` (success, 2026-10-09)\n"));
        assert!(out.contains("- board-deck.pdf: Notion page \"Board\"\n- sources/notes.md\n"));
        assert!(
            out.contains(
                "Cited in the document:\n\n- [[Budget 2026]]\n- https://example.com/report\n"
            ),
            "{out}"
        );
    }

    #[test]
    fn other_text_goes_in_a_fence_and_nothing_cited_says_so() {
        let mut r = result("outputs/data.csv", "draft");
        r.origin = None;
        let out = render(&r, "w1", "a,b\n1,2\n", &[]);
        assert!(
            out.starts_with("# Q3 plan\n\n```csv\na,b\n1,2\n```"),
            "{out}"
        );
        assert!(out.contains("- Review: draft\n- Written by: no run receipt names this file\n"));
        assert!(out.contains("None recorded"));
        let fenced = render(&r, "w1", "```\ncode\n```", &[]);
        assert!(fenced.contains("````csv\n```\ncode\n```\n````"), "{fenced}");
    }

    #[test]
    fn sources_come_from_sources_md_else_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("sources")).unwrap();
        std::fs::write(dir.path().join("sources/b.md"), "x").unwrap();
        std::fs::write(dir.path().join("sources/a.pdf"), "x").unwrap();
        std::fs::write(dir.path().join("sources/.hidden"), "x").unwrap();
        let names: Vec<String> = workspace_sources(dir.path())
            .into_iter()
            .map(|s| s.name)
            .collect();
        assert_eq!(names, ["sources/a.pdf", "sources/b.md"]);
        std::fs::write(
            dir.path().join("SOURCES.md"),
            "# Sources\n\nIntro.\n\n## deck.pdf\n\n- Origin: Notion page.\n- SHA-256: `x`\n\n## memo.md\n\nNo origin.\n",
        )
        .unwrap();
        assert_eq!(
            workspace_sources(dir.path()),
            vec![
                SourceLine {
                    name: "deck.pdf".into(),
                    origin: Some("Notion page.".into())
                },
                SourceLine {
                    name: "memo.md".into(),
                    origin: None
                },
            ]
        );
    }

    #[test]
    fn cited_links_are_unique_and_trimmed() {
        assert_eq!(
            cited("[[A]] [[A|alias]] [[B#h]] (https://x.io/a). <https://y.io> https://x.io/a,"),
            ["[[A]]", "[[B]]", "https://x.io/a", "https://y.io"]
        );
    }

    #[test]
    fn the_target_must_be_an_absolute_md_file() {
        let vault = tempfile::tempdir().unwrap();
        for bad in ["relative.md", "/tmp/x.txt"] {
            let err = export(
                vault.path(),
                "w1",
                "workspaces/w1/outputs/a.md",
                Path::new(bad),
                false,
            )
            .unwrap_err();
            assert!(err.contains("file picker"), "{bad}: {err}");
        }
    }

    #[test]
    fn dates_never_split_a_character_and_fences_outgrow_the_body() {
        let mut r = result("outputs/plan.md", "accepted");
        r.accepted_at = Some("2026-10-0é…".into());
        r.origin.as_mut().unwrap().finished_at = "ünicode-date-here".into();
        let out = render(&r, "w1", "x", &[]);
        assert!(out.contains("accepted on 2026-10-0é"), "{out}");
        assert_eq!(fence_for("a ```` b"), "`````");
        assert_eq!(fence_for("plain"), "```");
    }

    #[test]
    fn code_examples_are_not_citations_and_a_first_line_heading_counts() {
        let body = "See `https://a.io/x`!\n```\nhttps://example.com [[Not]]\n```\n| [[T\\|t]] | **https://b.io/y**? |\n";
        assert_eq!(cited(body), ["[[T]]", "https://a.io/x", "https://b.io/y"]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("SOURCES.md"),
            "## first.pdf\n- **Origin:** a scan\n",
        )
        .unwrap();
        assert_eq!(
            workspace_sources(dir.path()),
            vec![SourceLine {
                name: "first.pdf".into(),
                origin: Some("a scan".into())
            }]
        );
    }

    #[test]
    fn a_target_inside_the_vault_is_refused() {
        let vault = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(vault.path().join("notes")).unwrap();
        let inside = vault.path().join("notes/x.md");
        let err = export(
            vault.path(),
            "w1",
            "workspaces/w1/outputs/a.md",
            &inside,
            false,
        )
        .unwrap_err();
        assert!(err.contains("outside the vault"), "{err}");
    }
}
