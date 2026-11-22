//! Workspace-shape helpers the gardener reads (overnight B2, overnight
//! 10-11 B1): the real-open-plan-row count ([`open_plan_rows`], the two seed
//! rows every new plan starts with ([`SEED_ROWS`]) don't count), and the
//! **never used** test ([`never_used`]: still Hollow's blank template, no
//! conversations, no files, no plan rows of its own; overnight 10-11 B1: the
//! 09-24 import left ~60 of them), proposed at any age from
//! [`NEVER_USED_MIN_DAYS`] on with [`never_used_reason`].
//!
//! They lived in `archive_suggestions.rs` until the 11-10 sweep. The
//! "Suggest workspaces to archive…" flow that module was named for (its
//! command, the `workspaces.archive_suggestions` action, `suggest` and
//! `scan`) was removed 11-10 (S6) with its screen.
//!
//! Nothing here writes: archiving stays the person's click
//! (`update_workspace` with status `archived`, or the gardener's card).

use std::path::Path;

/// A never-used workspace is proposed once it is this many days old, so one
/// made a moment ago isn't.
pub(crate) const NEVER_USED_MIN_DAYS: u32 = 1;
/// The files `workspaces::create` writes; anything else in the folder is use.
const TEMPLATE_FILES: [&str; 4] = ["WORKSPACE.md", "PLAN.md", "SOURCES.md", "AGENTS.md"];
/// The rows `workspaces::create` seeds every PLAN.md with.
pub(crate) const SEED_ROWS: [&str; 2] = [
    "Define what a complete result needs to contain",
    "Produce and check the result",
];

fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .to_lowercase()
}

/// Real open rows in `text` (a PLAN.md), in plan order: unchecked, outside
/// code fences, not one of the [`SEED_ROWS`].
pub(crate) fn open_row_texts(text: &str) -> Vec<String> {
    let seeds: Vec<String> = SEED_ROWS.iter().map(|s| normalize(s)).collect();
    crate::vault_tasks::open_checklist_items(text)
        .into_iter()
        .filter(|row| {
            let n = normalize(row);
            !n.is_empty() && !seeds.contains(&n)
        })
        .collect()
}

/// How many [`open_row_texts`] workspace `id`'s PLAN.md has (0 when it has none or it
/// can't be read).
pub(crate) fn open_plan_rows(vault: &Path, id: &str) -> usize {
    open_plan_row_texts(vault, id).len()
}

/// [`open_row_texts`] of workspace `id`'s PLAN.md (none when it has none or
/// it can't be read).
pub(crate) fn open_plan_row_texts(vault: &Path, id: &str) -> Vec<String> {
    let Ok(dir) = crate::workspaces::folder(vault, id) else {
        return Vec::new();
    };
    // Bounded like every brief read: a huge plan counts from its first 1 MB.
    match std::fs::File::open(dir.join("PLAN.md")) {
        Ok(file) => {
            use std::io::Read;
            let mut text = String::new();
            let _ = file.take(1024 * 1024).read_to_string(&mut text);
            open_row_texts(&text)
        }
        Err(_) => Vec::new(),
    }
}

/// A file's text, bounded like every brief read (first 1 MB); `None` when
/// it can't be read.
fn read_bounded(path: &Path) -> Option<String> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let mut text = String::new();
    file.take(1024 * 1024).read_to_string(&mut text).ok()?;
    Some(text)
}

/// A brief still reading as Hollow's template: a `# title` line, then only
/// the template's sections, each but Outcome (the goal it was made with)
/// holding exactly its template text.
pub(crate) fn brief_is_template(text: &str) -> bool {
    let body = crate::frontmatter::parse(text).body;
    let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if let Some(head) = line.strip_prefix("## ") {
            sections.push((head.trim().to_string(), Vec::new()));
        } else if !line.is_empty() {
            match sections.last_mut() {
                Some((_, lines)) => lines.push(line),
                // Before the first section only the title may stand.
                None if line.starts_with("# ") => {}
                None => return false,
            }
        }
    }
    let mut seen = Vec::new();
    for (head, lines) in &sections {
        if seen.contains(head) {
            return false;
        }
        seen.push(head.clone());
        if head == "Outcome" {
            continue;
        }
        let Some((_, template)) = crate::workspaces::TEMPLATE_SECTIONS
            .iter()
            .find(|(h, _)| h == head)
        else {
            return false;
        };
        if normalize(&lines.join(" ")) != normalize(template) {
            return false;
        }
    }
    true
}

/// A plan holding only its title and the two [`SEED_ROWS`], unticked.
pub(crate) fn plan_is_template(text: &str) -> bool {
    let seeds: Vec<String> = SEED_ROWS.iter().map(|s| normalize(s)).collect();
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .all(|line| {
            line.starts_with("# ")
                || line
                    .strip_prefix("- [ ] ")
                    .is_some_and(|row| seeds.contains(&normalize(row)))
        })
}

/// A SOURCES.md with no entry: its title and at most the one template
/// sentence (whose wording changed over versions, so it isn't compared).
fn sources_is_template(text: &str) -> bool {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    lines.len() <= 2
        && lines.iter().enumerate().all(|(i, l)| {
            if i == 0 {
                l.starts_with("# ")
            } else {
                !l.starts_with('#')
                    && !l.starts_with('-')
                    && !l.starts_with('*')
                    && !l.contains("](")
            }
        })
}

/// Whether any conversation, archived or not and however old, ran in one of
/// `dirs` (a workspace's session folders). A store that can't be read counts
/// as yes: nothing is proposed on a guess.
pub(crate) fn ever_had_a_conversation(vault: &Path, dirs: &[std::path::PathBuf]) -> bool {
    crate::session_store::with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT DISTINCT cwd FROM sessions")
            .map_err(|e| e.to_string())?;
        let cwds = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(cwds
            .iter()
            .any(|cwd| dirs.contains(&crate::workspaces::session_dir(cwd))))
    })
    .unwrap_or(true)
}

/// Workspace `id` was never used: its folder holds only the template files
/// (hidden files aside; `sources/`, `outputs/`, `sessions/` empty), and its
/// brief, plan and sources still read as the template. Conversations are
/// the caller's to count (`WorkspaceFacts::conversations`). Unreadable means
/// used: nothing is proposed on a guess.
pub(crate) fn never_used(vault: &Path, id: &str) -> bool {
    let Ok(dir) = crate::workspaces::folder(vault, id) else {
        return false;
    };
    // No depth limit: `all` stops at the first file below the top level.
    let mut walk = walkdir::WalkDir::new(&dir)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
    let only_template = walk.all(|entry| match entry {
        Ok(e) if e.file_type().is_dir() => true,
        Ok(e) => {
            e.depth() == 1
                && TEMPLATE_FILES
                    .iter()
                    .any(|f| e.file_name().to_string_lossy() == *f)
        }
        Err(_) => false,
    });
    only_template
        && read_bounded(&dir.join("WORKSPACE.md")).is_some_and(|t| brief_is_template(&t))
        && read_bounded(&dir.join("PLAN.md")).is_none_or(|t| plan_is_template(&t))
        && read_bounded(&dir.join("SOURCES.md")).is_none_or(|t| sources_is_template(&t))
}

/// Why a never-used workspace is proposed (the gardener says the same).
pub(crate) fn never_used_reason(idle: u32) -> String {
    format!(
        "Never used: still Hollow's blank template (no conversations, files or plan rows of its own), {idle} day{} since it was last touched.",
        if idle == 1 { "" } else { "s" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_used_means_still_the_template_and_nothing_else() {
        let v = tempfile::tempdir().unwrap();
        let w = crate::workspaces::create(v.path(), "Shell", "A goal").unwrap();
        assert!(
            never_used(v.path(), &w.id),
            "a fresh workspace is the template"
        );
        let dir = crate::workspaces::folder(v.path(), &w.id).unwrap();
        // A source file is use.
        std::fs::write(dir.join("sources").join("a.md"), "x").unwrap();
        assert!(!never_used(v.path(), &w.id));
        std::fs::remove_file(dir.join("sources").join("a.md")).unwrap();
        assert!(never_used(v.path(), &w.id));
        // An edited brief section is use; the Outcome (its goal) isn't compared.
        let brief = std::fs::read_to_string(dir.join("WORKSPACE.md")).unwrap();
        std::fs::write(
            dir.join("WORKSPACE.md"),
            brief.replace("A goal", "Another goal"),
        )
        .unwrap();
        assert!(never_used(v.path(), &w.id));
        std::fs::write(
            dir.join("WORKSPACE.md"),
            brief.replace("Start with the request and attached sources.", "Call Dana."),
        )
        .unwrap();
        assert!(!never_used(v.path(), &w.id));
        std::fs::write(dir.join("WORKSPACE.md"), &brief).unwrap();
        // A real plan row is use.
        std::fs::write(dir.join("PLAN.md"), "# Plan\n\n- [ ] Ship the deck\n").unwrap();
        assert!(!never_used(v.path(), &w.id));
        assert!(!never_used(v.path(), "no-such-workspace"));
    }

    #[test]
    fn seed_rows_and_fenced_rows_are_not_open_work() {
        let seeded = "# Plan\n\n- [ ] Define what a complete result needs to contain\n- [ ] Produce and check the result\n";
        assert_eq!(open_row_texts(seeded).len(), 0);
        let real = "# Plan\n\n- [x] Done thing\n- [ ] Ship the deck\n```\n- [ ] not a row\n```\n* [ ] Call Sam\n";
        assert_eq!(open_row_texts(real).len(), 2);
    }
}
