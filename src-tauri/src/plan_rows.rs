//! Ticking one PLAN.md row from the panel's Plan view (Canvas C5a, plan §6).
//!
//! The only write the panel sections make, and only on a person's click
//! (`toggle_plan_row` is user-only in `docs/HOLLOW_APP_COMMAND_MAP.tsv`, so
//! it is never a `hollow_app` action). Markdown stays the source of truth:
//! the command flips exactly one `[ ]`/`[x]` box and leaves every other byte
//! of the file as it was, line endings included.
//!
//! **Compare-and-swap.** The caller names what it saw: the file's revision
//! (SHA-256 of its bytes, [`crate::vault::revision`]) and the row's identity
//! from `src/planModel.ts` (`planRowId`: the 1-based line number and the
//! SHA-256 of that line's text without its line ending). The file must still
//! have that revision, and that line must still hash the same and be a
//! checklist row outside a code fence and the frontmatter; otherwise nothing
//! is written and the error starts with [`CHANGED`]. Lines are counted as
//! `str::lines()` and `sectionMarkdown.sourceLines` count them (LF split, one
//! trailing CR dropped, a BOM on line 1 ignored), so both sides agree.
//!
//! **Locks and paths.** The read, the checks and the write run under the
//! vault mutate lock (lock order tier 5a) and write through
//! [`crate::vault::write_note_expecting_locked`] with the same revision
//! (temp file, full sync, rename, folder sync; the derived indexes are
//! refreshed). The workspace id goes through `workspaces::folder` (no
//! symlinked workspace), PLAN.md must be a regular file (never a symbolic
//! link), and `run_guard::is_protected_target` must not match it. Asking for
//! the state a row already has writes nothing (`unchanged`).

use crate::vault::{self, CasOutcome, Expected};
use serde::Serialize;
use std::fs;
use std::io::Read;
use std::path::Path;

/// Error prefix when the file or the row is no longer what the caller saw:
/// nothing was written, re-read PLAN.md.
pub const CHANGED: &str = "PLAN_CHANGED";

/// PLAN.md larger than this isn't read (the panel's reader limit).
const LIMIT: u64 = 1024 * 1024;

/// What a toggle did.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanRowToggled {
    /// `written`, or `unchanged` when the row already had that state.
    #[cfg_attr(test, ts(type = "\"written\" | \"unchanged\""))]
    pub outcome: String,
    /// PLAN.md's revision now (SHA-256 of its bytes).
    pub revision: String,
    /// The row's line, 1-based (unchanged by a toggle).
    pub line: u32,
    /// The SHA-256 of the row's line text now.
    pub line_sha256: String,
    /// The row's state now.
    pub done: bool,
}

/// Set the checklist row at `line` of `workspaces/<id>/PLAN.md` to `done`.
pub fn toggle(
    root: &Path,
    workspace_id: &str,
    line: u32,
    line_sha256: &str,
    expected_revision: &str,
    done: bool,
) -> Result<PlanRowToggled, String> {
    let dir = crate::workspaces::folder(root, workspace_id)?;
    let rel = format!("workspaces/{workspace_id}/PLAN.md");
    let guard = vault::lock_mutate();
    match fs::symlink_metadata(dir.join("PLAN.md")) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(
                "PLAN.md is a symbolic link; Hollow ticks rows only in the file itself".into(),
            )
        }
        Ok(meta) if meta.is_file() => {}
        Ok(_) => return Err("PLAN.md is not a file".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err("This workspace has no PLAN.md".into())
        }
        Err(e) => return Err(format!("PLAN.md: {e}")),
    }
    if crate::run_guard::is_protected_target(root, &rel) {
        return Err(format!("{rel} is a protected path"));
    }
    let text = read_bounded(&vault::safe_join(root, &rel)?)?;
    if vault::revision(text.as_bytes()) != expected_revision {
        return Err(format!(
            "{CHANGED}: PLAN.md changed since it was read; nothing was ticked. Read it again."
        ));
    }
    let moved = || {
        format!(
            "{CHANGED}: line {line} of PLAN.md is no longer the row you ticked; nothing was ticked. Read it again."
        )
    };
    let row = find_row(&text, line).ok_or_else(moved)?;
    if sha256_hex(row.hashed) != line_sha256 {
        return Err(moved());
    }
    let Some(box_at) = row.box_at else {
        return Err(format!(
            "line {line} of PLAN.md is not a checklist row; nothing was ticked"
        ));
    };
    let is_done = text.as_bytes()[box_at] != b' ';
    if is_done == done {
        return Ok(PlanRowToggled {
            outcome: "unchanged".into(),
            revision: expected_revision.into(),
            line,
            line_sha256: line_sha256.into(),
            done,
        });
    }
    let mut next = text.clone();
    next.replace_range(box_at..box_at + 1, if done { "x" } else { " " });
    let written = match vault::write_note_expecting_locked(
        &guard,
        root,
        &rel,
        &next,
        Expected::Revision(expected_revision),
    )? {
        CasOutcome::Written(note) => note,
        // Something outside Hollow's lock (Obsidian, an agent's own file
        // tools) wrote between the read above and the write.
        CasOutcome::Conflict(_) => {
            return Err(format!(
                "{CHANGED}: PLAN.md changed while it was being ticked; nothing was ticked. Read it again."
            ))
        }
    };
    let after = find_row(&next, line).map(|r| sha256_hex(r.hashed));
    Ok(PlanRowToggled {
        outcome: "written".into(),
        revision: written.revision,
        line,
        line_sha256: after.unwrap_or_default(),
        done,
    })
}

fn read_bounded(full: &Path) -> Result<String, String> {
    let file = fs::File::open(full).map_err(|e| format!("read PLAN.md: {e}"))?;
    let mut bytes = Vec::new();
    file.take(LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("read PLAN.md: {e}"))?;
    if bytes.len() as u64 > LIMIT {
        return Err("PLAN.md is larger than the 1 MB reader limit".into());
    }
    String::from_utf8(bytes).map_err(|_| "PLAN.md is not UTF-8 text".into())
}

/// One line of the file, located for the toggle (and for
/// `context_compact::plan_sections`, which counts rows the same way).
pub(crate) struct Row<'a> {
    /// The text the row identity hashes: no line ending, no BOM on line 1.
    pub(crate) hashed: &'a str,
    /// The byte offset in the whole text of the box's state character
    /// (` `, `x` or `X`), when the line is a checklist row outside a fence
    /// and the frontmatter.
    pub(crate) box_at: Option<usize>,
    /// Inside a code fence or the frontmatter (delimiters included): text,
    /// never a row or a heading.
    pub(crate) code: bool,
}

/// Line `n` (1-based) the way `str::lines()` counts, or None past the end.
fn find_row(text: &str, n: u32) -> Option<Row<'_>> {
    if n == 0 {
        return None;
    }
    let mut found = None;
    visit_rows(text, |line, row| {
        if line == n {
            found = Some(row);
            false
        } else {
            true
        }
    });
    found
}

/// One pass over the file's rows for the row editor; `visit` returns false
/// to stop.
pub(crate) fn visit_rows<'a>(text: &'a str, mut visit: impl FnMut(u32, Row<'a>) -> bool) {
    let mut offset = 0;
    let mut fence: Option<(char, usize)> = None;
    let mut frontmatter = false;
    for (i, raw) in text.split_inclusive('\n').enumerate() {
        let start = offset;
        offset += raw.len();
        let mut line = raw.strip_suffix('\n').unwrap_or(raw);
        line = line.strip_suffix('\r').unwrap_or(line);
        let mut lead = 0;
        if i == 0 {
            if let Some(rest) = line.strip_prefix('\u{feff}') {
                lead = '\u{feff}'.len_utf8();
                line = rest;
            }
            // `readSections`: frontmatter only when a closing line follows.
            frontmatter = line == "---" && text.lines().skip(1).any(|l| l == "---" || l == "...");
        } else if frontmatter && (line == "---" || line == "...") {
            frontmatter = false;
            if !visit(
                (i + 1) as u32,
                Row {
                    hashed: line,
                    box_at: None,
                    code: true,
                },
            ) {
                return;
            }
            continue;
        }
        // CommonMark fences, as `sectionMarkdown.sourceLines` marks them.
        let fenced = match fence {
            Some((ch, len)) => {
                if let Some((c, l, rest)) = fence_run(line) {
                    if c == ch && l >= len && rest.trim().is_empty() {
                        fence = None;
                    }
                }
                true
            }
            None => match fence_run(line) {
                Some((c, l, rest)) if !(c == '`' && rest.contains('`')) => {
                    fence = Some((c, l));
                    true
                }
                _ => false,
            },
        };
        {
            let box_at = if fenced || frontmatter {
                None
            } else {
                box_offset(line).map(|at| start + lead + at)
            };
            if !visit(
                (i + 1) as u32,
                Row {
                    hashed: line,
                    box_at,
                    code: fenced || frontmatter,
                },
            ) {
                return;
            }
        }
    }
}

/// A line opening or closing a fence: its character, run length and the text
/// after the run (`^\s*(`{3,}|~{3,})(.*)$`). Also `workspaces::heading_lines`.
pub(crate) fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let body = line.trim_start();
    let ch = body.chars().next().filter(|c| *c == '`' || *c == '~')?;
    let len = body.chars().take_while(|c| *c == ch).count();
    (len >= 3).then(|| (ch, len, &body[len..]))
}

/// The byte offset in `line` of a checklist row's box state, as
/// `planModel.readPlan` reads a row: a list item (`-`, `*`, `+`, or one to
/// nine digits and `.` or `)`) after spaces or tabs, then `[ ]`, `[x]` or
/// `[X]` ending the line or followed by a space or tab.
fn box_offset(line: &str) -> Option<usize> {
    // A JavaScript `.` stops at these, so the model never reads such a line as a row.
    if line.contains(['\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let bytes = line.as_bytes();
    let mut i = bytes
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    match bytes.get(i)? {
        b'-' | b'*' | b'+' => i += 1,
        b'0'..=b'9' => {
            let digits = bytes[i..].iter().take_while(|b| b.is_ascii_digit()).count();
            if digits > 9 || !matches!(bytes.get(i + digits), Some(b'.' | b')')) {
                return None;
            }
            i += digits + 1;
        }
        _ => return None,
    }
    let gap = bytes[i..]
        .iter()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    if gap == 0 {
        return None;
    }
    i += gap;
    if bytes.get(i) != Some(&b'[')
        || !matches!(bytes.get(i + 1), Some(b' ' | b'x' | b'X'))
        || bytes.get(i + 2) != Some(&b']')
        || !matches!(bytes.get(i + 3), None | Some(b' ' | b'\t'))
    {
        return None;
    }
    Some(i + 1)
}

fn sha256_hex(text: &str) -> String {
    vault::revision(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(plan: &str) -> (tempfile::TempDir, String) {
        let root = tempfile::tempdir().unwrap();
        let w = crate::workspaces::create(root.path(), "Launch plan", "Ship it").unwrap();
        fs::write(plan_path(root.path(), &w.id), plan).unwrap();
        (root, w.id)
    }

    fn plan_path(root: &Path, id: &str) -> std::path::PathBuf {
        root.join("workspaces").join(id).join("PLAN.md")
    }

    fn read(root: &Path, id: &str) -> String {
        fs::read_to_string(plan_path(root, id)).unwrap()
    }

    /// Tick line `n` as the panel would: the file's revision and the line's hash as read.
    fn tick(root: &Path, id: &str, n: u32, done: bool) -> Result<PlanRowToggled, String> {
        let text = read(root, id);
        let hashed = find_row(&text, n)
            .map(|r| sha256_hex(r.hashed))
            .unwrap_or_default();
        toggle(
            root,
            id,
            n,
            &hashed,
            &vault::revision(text.as_bytes()),
            done,
        )
    }

    #[test]
    fn a_tick_flips_exactly_that_box_and_nothing_else() {
        let plan = "# Plan\n\n- [ ] Draft the brief\n- [ ] Book the room\n";
        let (root, id) = workspace(plan);
        let out = tick(root.path(), &id, 4, true).unwrap();
        assert_eq!(out.outcome, "written");
        assert!(out.done);
        let now = read(root.path(), &id);
        assert_eq!(
            now,
            "# Plan\n\n- [ ] Draft the brief\n- [x] Book the room\n"
        );
        assert_eq!(out.revision, vault::revision(now.as_bytes()));
        assert_eq!(out.line, 4);
        assert_eq!(out.line_sha256, sha256_hex("- [x] Book the room"));
        // And back: the original bytes exactly.
        let back = tick(root.path(), &id, 4, false).unwrap();
        assert_eq!(back.outcome, "written");
        assert_eq!(read(root.path(), &id), plan);
    }

    #[test]
    fn asking_for_the_state_a_row_has_writes_nothing() {
        let plan = "- [x] Done already\n- [X] Shouted\n- [ ] Open\n";
        let (root, id) = workspace(plan);
        let full = plan_path(root.path(), &id);
        let before = fs::metadata(&full).unwrap().modified().unwrap();
        for (n, done) in [(1, true), (2, true), (3, false)] {
            let out = tick(root.path(), &id, n, done).unwrap();
            assert_eq!(out.outcome, "unchanged", "line {n}");
            assert_eq!(out.revision, vault::revision(plan.as_bytes()));
        }
        assert_eq!(read(root.path(), &id), plan);
        assert_eq!(fs::metadata(&full).unwrap().modified().unwrap(), before);
        // A re-flip after a flip, with the identity the re-read gives, is also a no-op.
        assert_eq!(tick(root.path(), &id, 3, true).unwrap().outcome, "written");
        let once = read(root.path(), &id);
        assert_eq!(
            tick(root.path(), &id, 3, true).unwrap().outcome,
            "unchanged"
        );
        assert_eq!(read(root.path(), &id), once);
        // An uppercase X unticks like a lowercase one.
        tick(root.path(), &id, 2, false).unwrap();
        assert_eq!(
            read(root.path(), &id),
            "- [x] Done already\n- [ ] Shouted\n- [x] Open\n"
        );
    }

    #[test]
    fn a_stale_file_revision_is_a_conflict_and_writes_nothing() {
        let plan = "- [ ] One\n- [ ] Two\n";
        let (root, id) = workspace(plan);
        let seen = vault::revision(plan.as_bytes());
        let row = sha256_hex("- [ ] Two");
        // Someone edits another line after the panel read the file.
        let edited = "- [ ] One, reworded\n- [ ] Two\n";
        fs::write(plan_path(root.path(), &id), edited).unwrap();
        let err = toggle(root.path(), &id, 2, &row, &seen, true).unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        assert!(err.contains("changed since it was read"), "{err}");
        assert_eq!(read(root.path(), &id), edited);
        // The same request twice (a double click): the second one is stale.
        fs::write(plan_path(root.path(), &id), plan).unwrap();
        toggle(root.path(), &id, 2, &row, &seen, true).unwrap();
        let err = toggle(root.path(), &id, 2, &row, &seen, true).unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        assert_eq!(read(root.path(), &id), "- [ ] One\n- [x] Two\n");
    }

    #[test]
    fn a_line_that_moved_or_changed_is_a_conflict() {
        let plan = "- [ ] One\n- [ ] Two\n- [ ] Three\n";
        let (root, id) = workspace(plan);
        let rev = vault::revision(plan.as_bytes());
        // The panel's row identity for "Two" is line 2; a caller naming the
        // right file but the wrong line (the row moved) is refused.
        let two = sha256_hex("- [ ] Two");
        let lower = sha256_hex("- [ ] two");
        for (line, hash) in [(3, &two), (1, &two), (2, &lower), (4, &two), (0, &two)] {
            let err = toggle(root.path(), &id, line, hash, &rev, true).unwrap_err();
            assert!(err.starts_with(CHANGED), "line {line}: {err}");
            assert!(err.contains("no longer the row"), "line {line}: {err}");
        }
        assert_eq!(read(root.path(), &id), plan);
        // A line inserted above shifts the row: the old identity no longer matches.
        let shifted = "- [ ] Zero\n- [ ] One\n- [ ] Two\n- [ ] Three\n";
        fs::write(plan_path(root.path(), &id), shifted).unwrap();
        let err = toggle(
            root.path(),
            &id,
            2,
            &two,
            &vault::revision(shifted.as_bytes()),
            true,
        )
        .unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        assert_eq!(read(root.path(), &id), shifted);
    }

    #[test]
    fn nested_and_numbered_rows_flip_in_place() {
        let plan = "## Ship\n\n- [ ] Parent\n  - plain bullet\n    - [ ] Grandchild\n\t- [x] Tabbed child\n1. [ ] Numbered\n2) [x] Paren\n+ [ ] Plus\n* [ ]\n";
        let (root, id) = workspace(plan);
        tick(root.path(), &id, 5, true).unwrap();
        tick(root.path(), &id, 6, false).unwrap();
        tick(root.path(), &id, 7, true).unwrap();
        tick(root.path(), &id, 8, false).unwrap();
        tick(root.path(), &id, 9, true).unwrap();
        tick(root.path(), &id, 10, true).unwrap();
        assert_eq!(
            read(root.path(), &id),
            "## Ship\n\n- [ ] Parent\n  - plain bullet\n    - [x] Grandchild\n\t- [ ] Tabbed child\n1. [x] Numbered\n2) [ ] Paren\n+ [x] Plus\n* [x]\n"
        );
    }

    #[test]
    fn crlf_and_a_bom_keep_their_bytes() {
        let plan = "\u{feff}- [ ] First\r\n- [ ] Second\r\n\r\nlast line without an ending - [ ]";
        let (root, id) = workspace(plan);
        // Line 1 hashes without its BOM, like sectionMarkdown.sourceLines.
        let text = read(root.path(), &id);
        let out = toggle(
            root.path(),
            &id,
            1,
            &sha256_hex("- [ ] First"),
            &vault::revision(text.as_bytes()),
            true,
        )
        .unwrap();
        assert_eq!(out.line_sha256, sha256_hex("- [x] First"));
        tick(root.path(), &id, 2, true).unwrap();
        assert_eq!(
            read(root.path(), &id),
            "\u{feff}- [x] First\r\n- [x] Second\r\n\r\nlast line without an ending - [ ]"
        );
        // A CRLF line hashes without its CR.
        let text = read(root.path(), &id);
        let err = toggle(
            root.path(),
            &id,
            2,
            &sha256_hex("- [x] Second\r"),
            &vault::revision(text.as_bytes()),
            false,
        )
        .unwrap_err();
        assert!(err.starts_with(CHANGED), "{err}");
        assert!(tick(root.path(), &id, 4, true)
            .unwrap_err()
            .contains("not a checklist row"));
    }

    #[test]
    fn only_real_rows_flip_never_code_frontmatter_or_lookalikes() {
        let plan = "---\ntags: [x]\n- [ ] in frontmatter\n---\n# Plan\n```md\n- [ ] fenced\n```\n~~~~\n- [ ] tilde fenced\n~~~\n~~~~\n- [x]no space\n-[ ] no gap\n- [y] other\n- [ ] row\n";
        let (root, id) = workspace(plan);
        for n in [2, 3, 4, 7, 10, 13, 14, 15] {
            let err = tick(root.path(), &id, n, true).unwrap_err();
            assert!(err.contains("not a checklist row"), "line {n}: {err}");
        }
        assert_eq!(read(root.path(), &id), plan);
        tick(root.path(), &id, 16, true).unwrap();
        assert!(read(root.path(), &id).ends_with("- [x] row\n"));
        // An opening run with a backtick in its info string is not a fence.
        let inline = "``` not `a` fence\n- [ ] row\n";
        fs::write(plan_path(root.path(), &id), inline).unwrap();
        assert_eq!(tick(root.path(), &id, 2, true).unwrap().outcome, "written");
    }

    #[test]
    fn the_typescript_row_identity_opens_the_real_plan() {
        // planModel.test.ts pins this row of the sanitized real PLAN.md.
        let fixture = include_str!("../../tests/fixtures/workspace-sections/PLAN.md");
        let (root, id) = workspace(fixture);
        let out = toggle(
            root.path(),
            &id,
            13,
            "1851dc1df2e57e23f3c9e03a169d43f3e177df7f5e4aeabecba7efaf391e0d56",
            &vault::revision(fixture.as_bytes()),
            false,
        )
        .unwrap();
        assert_eq!(out.outcome, "written");
        let now = read(root.path(), &id);
        let (a, b) = (
            fixture.lines().collect::<Vec<_>>(),
            now.lines().collect::<Vec<_>>(),
        );
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            if i == 12 {
                assert_eq!(x.replacen("- [x]", "- [ ]", 1), *y);
            } else {
                assert_eq!(x, y, "line {}", i + 1);
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn protected_and_linked_paths_are_refused() {
        let (root, id) = workspace("- [ ] row\n");
        let rev = vault::revision(b"- [ ] row\n");
        let row = sha256_hex("- [ ] row");
        for bad in ["../me", "", "a/b", "me/../x"] {
            assert!(
                toggle(root.path(), bad, 1, &row, &rev, true).is_err(),
                "{bad}"
            );
        }
        // PLAN.md as a link to a protected file, or out of the vault, is never written through.
        let me = root.path().join("me");
        fs::create_dir_all(&me).unwrap();
        fs::write(me.join("Guardrails.md"), "- [ ] row\n").unwrap();
        let plan = plan_path(root.path(), &id);
        fs::remove_file(&plan).unwrap();
        std::os::unix::fs::symlink(me.join("Guardrails.md"), &plan).unwrap();
        let err = toggle(root.path(), &id, 1, &row, &rev, true).unwrap_err();
        assert!(err.contains("symbolic link"), "{err}");
        assert_eq!(
            fs::read_to_string(me.join("Guardrails.md")).unwrap(),
            "- [ ] row\n"
        );
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("PLAN.md"), "- [ ] row\n").unwrap();
        fs::remove_file(&plan).unwrap();
        std::os::unix::fs::symlink(outside.path().join("PLAN.md"), &plan).unwrap();
        assert!(toggle(root.path(), &id, 1, &row, &rev, true).is_err());
        assert_eq!(
            fs::read_to_string(outside.path().join("PLAN.md")).unwrap(),
            "- [ ] row\n"
        );
        // A symlinked workspace folder is refused before anything is read.
        let real = root.path().join("elsewhere");
        fs::create_dir_all(&real).unwrap();
        fs::write(real.join("PLAN.md"), "- [ ] row\n").unwrap();
        std::os::unix::fs::symlink(&real, root.path().join("workspaces").join("linked")).unwrap();
        assert!(toggle(root.path(), "linked", 1, &row, &rev, true).is_err());
        assert_eq!(
            fs::read_to_string(real.join("PLAN.md")).unwrap(),
            "- [ ] row\n"
        );
        // No PLAN.md at all.
        fs::remove_file(&plan).unwrap();
        assert!(toggle(root.path(), &id, 1, &row, &rev, true)
            .unwrap_err()
            .contains("no PLAN.md"));
        assert!(!plan.exists());
    }

    #[test]
    fn a_file_that_is_not_utf8_or_too_large_is_refused() {
        let (root, id) = workspace("- [ ] row\n");
        let plan = plan_path(root.path(), &id);
        let bad = b"- [ ] row\n\xff\n";
        fs::write(&plan, bad).unwrap();
        let err = toggle(
            root.path(),
            &id,
            1,
            &sha256_hex("- [ ] row"),
            &vault::revision(bad),
            true,
        )
        .unwrap_err();
        assert!(err.contains("not UTF-8"), "{err}");
        assert_eq!(fs::read(&plan).unwrap(), bad);
        let big = format!("- [ ] row\n{}", "x".repeat(LIMIT as usize));
        fs::write(&plan, &big).unwrap();
        let err = toggle(
            root.path(),
            &id,
            1,
            &sha256_hex("- [ ] row"),
            &vault::revision(big.as_bytes()),
            true,
        )
        .unwrap_err();
        assert!(err.contains("1 MB"), "{err}");
    }
}
