//! Compact a workspace brief's "Current context" (overnight C4): move its
//! older dated entries into a dated archive note and keep the newest, so the
//! brief stays readable and under its 48 KB limit (the Main agent's had grown
//! past 40 lines).
//!
//! **Entries.** The section runs from its `## Current context` heading to the
//! next `## ` (or `# `) heading. It is read as blank-line-separated blocks: a
//! block whose first line is a top-level bullet (`- …`) holds one entry per
//! top-level bullet (with its indented lines); any other block is one entry
//! (a bold-date paragraph, bullet lines inside it included). An entry is
//! dated only by a date at its very start, after `- ` and/or `**`: a
//! `YYYY-MM-DD`, or `MM-DD` (the year taken from `today`, or the year before
//! when that would be in the future). A block starting `As of`, an entry
//! with no leading date, and the archive's own pointer line never move.
//!
//! **What moves.** Dated entries outside the newest `keep_newest` and older
//! than `keep_days` days (default 1: today's and yesterday's always stay).
//! They are appended, in order and byte-for-byte, to
//! `outputs/workspace-archive/<today>-current-context.md` in the workspace
//! (created, or added to under a new heading). The brief is spliced by line
//! ranges, each line keeping its own ending: only the moved entries' lines
//! go (and a blank line a drop leaves doubled), and one pointer line goes at
//! the section's end, replacing every earlier pointer; every other byte
//! stays. Archive older entries… first lists, by day, the entries that would
//! move (a count and the first headlines). Dating is the only rule, so a
//! standing instruction written as a dated entry moves too: untick its day
//! (which keeps that whole day), or move such a line to Decisions first.
//!
//! **Safe writes.** Refused, with nothing written, while a run is live in
//! that workspace; the move is its own History run, so Undo puts every file
//! back. The archive is written first, by
//! compare-and-swap (created only if absent, appended only to the revision
//! read), then the brief only if unchanged since it was read; if the brief
//! write fails or conflicts, the archive is put back. So entries are never
//! only in History, and a concurrent edit loses nothing.
//!
//! **Who runs it.** Since V5a (2026-11-15) the hollow_app action
//! `workspaces.archive_shared_docs` (the `archive_shared_docs` command) and
//! the workspace menu's "Archive older entries…". One doc at a time,
//! WORKSPACE.md (older dated Current context entries) or PLAN.md (sections
//! with every row ticked; open work stays): a dry run returns the sizes
//! before and after, what moves and the doc's hash; an apply sends that hash
//! back as `expectedHash` and moves nothing when the doc changed
//! ([`dry_run_shared_doc`], [`apply_shared_doc_in_run`] for an agent inside
//! its own run, [`apply_shared_doc`] for the app's own History run). Every
//! apply first checks the move is lossless ([`is_lossless`]: kept plus
//! archived lines are the original's). The older per-day and per-section
//! picker (`archive_preview`, `archive_older_entries`) and the agent action
//! `workspaces.compact_context` (removed in P0-24) stay gone; `compact`
//! and the preview types are test-only. [`ARCHIVE_DIR`] is left out of
//! Results, and [`ARCHIVE_SOURCE`] labels the History run.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

pub const DEFAULT_KEEP_DAYS: u32 = 1;
pub const DEFAULT_KEEP_NEWEST: u32 = 10;
/// What the pointer line an archive writes says (it replaces earlier ones
/// by matching it). `workspaces::brief_selection` keeps any line holding it
/// in a cut brief; the replay probes look for it.
pub(crate) const POINTER: &str = "Earlier entries were moved to";

/// One entry: its line range `[start, end)` in the brief and its date.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub start: usize,
    pub end: usize,
    pub date: Option<NaiveDate>,
    /// A paragraph entry (its block is one entry), not a bullet.
    pub paragraph: bool,
}

/// The `## Current context` section's line range `[start, end)`.
pub(crate) fn section(lines: &[&str]) -> Option<(usize, usize)> {
    let head = lines.iter().position(|l| {
        l.starts_with("## ")
            && l.trim_end()
                .trim_start_matches('#')
                .trim()
                .eq_ignore_ascii_case("current context")
    })?;
    let end = lines[head + 1..]
        .iter()
        .position(|l| l.starts_with("## ") || l.starts_with("# "))
        .map_or(lines.len(), |i| head + 1 + i);
    Some((head + 1, end))
}

/// A strict `YYYY-MM-DD` or `MM-DD` at the start of `rest`.
fn leading_date(rest: &str, today: NaiveDate) -> Option<NaiveDate> {
    let c: Vec<char> = rest.chars().collect();
    let digits = |r: std::ops::Range<usize>| {
        r.clone()
            .all(|i| c.get(i).is_some_and(char::is_ascii_digit))
    };
    let not_digit = |i: usize| c.get(i).is_none_or(|ch| !ch.is_ascii_digit());
    if digits(0..4)
        && c.get(4) == Some(&'-')
        && digits(5..7)
        && c.get(7) == Some(&'-')
        && digits(8..10)
        && not_digit(10)
    {
        let s: String = c[..10].iter().collect();
        return NaiveDate::parse_from_str(&s, "%Y-%m-%d").ok();
    }
    if digits(0..2) && c.get(2) == Some(&'-') && digits(3..5) && not_digit(5) {
        let s: String = c[..5].iter().collect();
        let (m, d) = s.split_once('-')?;
        let (m, d) = (m.parse::<u32>().ok()?, d.parse::<u32>().ok()?);
        let this_year = NaiveDate::from_ymd_opt(today.year(), m, d)?;
        return if this_year > today + chrono::Duration::days(1) {
            NaiveDate::from_ymd_opt(today.year() - 1, m, d)
        } else {
            Some(this_year)
        };
    }
    None
}

/// An entry's date: only a leading one, after `- ` and/or `**`.
fn date_of(first_line: &str, today: NaiveDate) -> Option<NaiveDate> {
    if first_line.contains(POINTER) {
        return None;
    }
    let mut rest = first_line.trim_start();
    rest = rest.strip_prefix("- ").unwrap_or(rest).trim_start();
    rest = rest.strip_prefix("**").unwrap_or(rest).trim_start();
    leading_date(rest, today)
}

/// The entries of section `[start, end)` of `lines`.
pub fn entries(lines: &[&str], start: usize, end: usize, today: NaiveDate) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut i = start;
    while i < end {
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let block_start = i;
        let mut block_end = i;
        while block_end < end && !lines[block_end].trim().is_empty() {
            block_end += 1;
        }
        let first = lines[block_start];
        if first.trim_start().starts_with("As of") {
            // A header line, never an entry; what follows it in the same
            // block is read on its own.
            i = block_start + 1;
            continue;
        }
        if first.starts_with("- ") {
            let mut s = block_start;
            while s < block_end {
                let mut e = s + 1;
                while e < block_end && !lines[e].starts_with("- ") {
                    e += 1;
                }
                out.push(Entry {
                    start: s,
                    end: e,
                    date: date_of(lines[s], today),
                    paragraph: false,
                });
                s = e;
            }
        } else {
            out.push(Entry {
                start: block_start,
                end: block_end,
                date: date_of(first, today),
                paragraph: true,
            });
        }
        i = block_end;
    }
    out
}

/// The part of the Current context section that may move: it stops at the
/// first heading of any level, so entries under a `###` sub-heading inside
/// the section stay where they are (readers of the section, such as the
/// envelope cut, still see the whole of it through [`section`]).
fn move_section(lines: &[&str]) -> Option<(usize, usize)> {
    let (start, end) = section(lines)?;
    let end = (start..end)
        .find(|&i| atx(lines[i]).is_some())
        .unwrap_or(end);
    Some((start, end))
}

/// Which entries move: dated, outside the newest `keep_newest` dated ones,
/// and older than `keep_days`. Indexes into `entries`.
pub fn to_move(
    entries: &[Entry],
    today: NaiveDate,
    keep_days: u32,
    keep_newest: u32,
) -> Vec<usize> {
    let cutoff = today - chrono::Duration::days(i64::from(keep_days));
    let mut dated: Vec<(usize, NaiveDate)> = entries
        .iter()
        .enumerate()
        .filter_map(|(i, e)| e.date.map(|d| (i, d)))
        .collect();
    // Newest first; later entries win ties (they were written later).
    dated.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.cmp(&a.0)));
    let kept: std::collections::HashSet<usize> = dated
        .iter()
        .take(keep_newest as usize)
        .map(|(i, _)| *i)
        .collect();
    let mut out: Vec<usize> = dated
        .into_iter()
        .filter(|(i, d)| *d < cutoff && !kept.contains(i))
        .map(|(i, _)| i)
        .collect();
    out.sort_unstable();
    out
}

/// What a compaction would do.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    pub brief: String,
    pub archive: String,
    /// `(date, first line)` per moved entry.
    pub moved: Vec<(NaiveDate, String)>,
}

/// The lines of `text` without their endings, and each line's ending
/// (`"\r\n"`, `"\n"`, or `""` for a last line without one).
fn split_lines(text: &str) -> (Vec<&str>, Vec<&'static str>) {
    let mut lines: Vec<&str> = Vec::new();
    let mut ends: Vec<&'static str> = Vec::new();
    for piece in text.split_inclusive('\n') {
        let body = piece.strip_suffix('\n').unwrap_or(piece);
        let body_cr = body.strip_suffix('\r');
        lines.push(body_cr.unwrap_or(body));
        ends.push(match (piece.ends_with('\n'), body_cr.is_some()) {
            (true, true) => "\r\n",
            (true, false) => "\n",
            (false, _) => "",
        });
    }
    (lines, ends)
}

/// A pointer line this module wrote, by its whole shape: an optional `- `,
/// `phrase` ([`POINTER`] or [`PLAN_POINTER`]), a backticked path under
/// [`ARCHIVE_DIR`], ` on <date>`, an optional ` (through <date>)`, the
/// optional "earlier moves" clause and the final period. Any other line
/// holding the phrase is the person's text and is never replaced.
pub(crate) fn is_pointer_line(line: &str, phrase: &str) -> bool {
    static SHAPE: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(
            r"^(?:- )?(Earlier entries were moved to|Finished sections were moved to) `outputs/workspace-archive/[^`]+` on \d{4}-\d{2}-\d{2}(?: \(through \d{4}-\d{2}-\d{2}\))?(?:; earlier moves are beside it in `outputs/workspace-archive/`)?\.$",
        )
        .expect("pointer shape")
    });
    // A file's BOM may sit on a pointer written as its first line.
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    SHAPE.captures(line).is_some_and(|c| &c[1] == phrase)
}

/// The compacted brief and the archive text for `brief`, or `None` when
/// nothing moves. Splices by line ranges, each line keeping its own ending,
/// so untouched bytes stay. Tests only since P0-24 removed the action that
/// called it; the button goes through [`compact_days`].
#[cfg(test)]
pub fn compact(
    brief: &str,
    today: NaiveDate,
    keep_days: u32,
    keep_newest: u32,
    archive_rel: &str,
) -> Option<Plan> {
    compact_days(brief, today, keep_days, keep_newest, archive_rel, None)
}

/// `compact`, moving only entries dated one of `days` when given (the
/// days the person ticked in Archive older entries).
pub fn compact_days(
    brief: &str,
    today: NaiveDate,
    keep_days: u32,
    keep_newest: u32,
    archive_rel: &str,
    days: Option<&BTreeSet<NaiveDate>>,
) -> Option<Plan> {
    let (lines, ends) = split_lines(brief);
    // New lines (the pointer, a blank before it) take the brief's first ending.
    let nl = ends.iter().copied().find(|e| !e.is_empty()).unwrap_or("\n");
    let (start, end) = move_section(&lines)?;
    let all = entries(&lines, start, end, today);
    let mut moving = to_move(&all, today, keep_days, keep_newest);
    if let Some(days) = days {
        moving.retain(|&i| all[i].date.is_some_and(|d| days.contains(&d)));
    }
    if moving.is_empty() {
        return None;
    }
    let mut drop = vec![false; lines.len()];
    let mut archive_parts = Vec::new();
    let mut moved = Vec::new();
    for &i in &moving {
        let e = &all[i];
        for flag in drop.iter_mut().take(e.end).skip(e.start) {
            *flag = true;
        }
        let mut text: String = (e.start..e.end)
            .map(|j| format!("{}{}", lines[j], ends[j]))
            .collect();
        text.truncate(text.trim_end_matches(['\r', '\n']).len());
        archive_parts.push(text);
        moved.push((e.date.unwrap_or(today), lines[e.start].to_string()));
    }
    let through = moved.iter().map(|(d, _)| *d).max()?;
    let bullets = !all.iter().any(|e| e.paragraph);
    // Every earlier pointer is replaced, so a brief compacted on many days
    // still carries one line; it names the folder when earlier moves went
    // to other archives there. Only lines of the generated shape: a line of
    // the person's that quotes the phrase stays.
    let mut elsewhere = false;
    for i in start..end {
        if is_pointer_line(lines[i], POINTER) {
            drop[i] = true;
            elsewhere |= !lines[i].contains(archive_rel);
        }
    }
    let dir = archive_rel.rsplit_once('/').map_or("", |(d, _)| d);
    let pointer = format!(
        "{}{POINTER} `{archive_rel}` on {today} (through {through}){}.",
        if bullets { "- " } else { "" },
        if elsewhere && !dir.is_empty() {
            format!("; earlier moves are beside it in `{dir}/`")
        } else {
            String::new()
        }
    );
    // The pointer goes after the section's last kept non-blank line.
    let last_kept = (start..end)
        .rev()
        .find(|&i| !drop[i] && !lines[i].trim().is_empty());
    let mut out: Vec<(String, &str)> = Vec::with_capacity(lines.len() + 2);
    let mut placed = false;
    let mut dropped_since = false;
    let push_pointer = |out: &mut Vec<(String, &str)>| {
        if let Some(last) = out.last_mut() {
            if last.1.is_empty() {
                last.1 = nl;
            }
        }
        if !bullets {
            out.push((String::new(), nl));
        }
        out.push((pointer.clone(), nl));
    };
    for (i, line) in lines.iter().enumerate() {
        let in_section = (start..end).contains(&i);
        if i == start && last_kept.is_none() {
            push_pointer(&mut out);
            placed = true;
        }
        if drop[i] {
            dropped_since = true;
            continue;
        }
        // A blank line that a drop left next to another blank goes too; the
        // brief's own blank runs stay.
        if in_section
            && dropped_since
            && line.trim().is_empty()
            && out.last().is_some_and(|(l, _)| l.trim().is_empty())
        {
            continue;
        }
        out.push(((*line).to_string(), ends[i]));
        dropped_since = false;
        if Some(i) == last_kept {
            let was_last_line = i + 1 == lines.len();
            push_pointer(&mut out);
            if was_last_line && ends[i].is_empty() {
                // The brief ended without a newline; so does the result.
                if let Some(last) = out.last_mut() {
                    last.1 = "";
                }
            }
            placed = true;
        }
    }
    if !placed {
        return None;
    }
    let sep = if bullets {
        nl.to_string()
    } else {
        format!("{nl}{nl}")
    };
    Some(Plan {
        brief: out.iter().map(|(l, e)| format!("{l}{e}")).collect(),
        archive: archive_parts.join(&sep),
        moved,
    })
}

/// Where compacted entries go, under a workspace. Not a deliverable:
/// [`crate::workspace_results::is_listed`] leaves it out of Results.
pub const ARCHIVE_DIR: &str = "outputs/workspace-archive/";

/// Move text into an archive note, then shorten the file it came from.
/// The archive first, so the text is never only in History: created as
/// `header` + `section` if absent, else `section` appended to the revision
/// just read (an unreadable existing archive stops here, before anything is
/// written). Then `target_rel` becomes `target_text`, only if still at
/// `revision`; if it changed (or the write fails), the archive goes back to
/// what it was. `what` ("brief", "plan") and `done` ("compacted",
/// "archived") word the errors, which all say nothing moved.
#[allow(clippy::too_many_arguments)]
fn move_into_archive(
    vault: &Path,
    archive_path: &str,
    header: &str,
    section: &str,
    target_rel: &str,
    target_text: &str,
    revision: &str,
    what: &str,
    done: &str,
) -> Result<(), String> {
    use crate::vault::{CasOutcome, Expected};
    let existing = if crate::vault::safe_join(vault, archive_path)?.exists() {
        Some(crate::workspaces::read_file(vault, archive_path)?)
    } else {
        None
    };
    let archive_text = match &existing {
        Some(old) => {
            let mut text = old.content.clone();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            text.push('\n');
            text.push_str(section);
            text
        }
        None => format!("{header}{section}"),
    };
    let expected = existing
        .as_ref()
        .map_or(Expected::Absent, |old| Expected::Revision(&old.revision));
    let written =
        match crate::vault::write_note_expecting(vault, archive_path, &archive_text, expected)? {
            CasOutcome::Written(n) => n,
            CasOutcome::Conflict(_) => {
                return Err(
                "The archive note changed while it was being written; nothing moved. Try again."
                    .into(),
            );
            }
        };
    #[cfg(test)]
    let failing = tests::FAIL_WRITE.with(|f| f.borrow().as_deref() == Some(target_rel));
    #[cfg(not(test))]
    let failing = false;
    let target = if failing {
        Err("a test made this write fail".to_string())
    } else {
        crate::vault::write_note_expecting(
            vault,
            target_rel,
            target_text,
            Expected::Revision(revision),
        )
    };
    if matches!(target, Ok(CasOutcome::Written(_))) {
        return Ok(());
    }
    let undone = match &existing {
        Some(old) => crate::vault::write_note_expecting(
            vault,
            archive_path,
            &old.content,
            Expected::Revision(&written.revision),
        )
        .map(|o| matches!(o, CasOutcome::Written(_))),
        None => crate::vault::delete_note(vault, archive_path).map(|()| true),
    };
    let undo = match undone {
        Ok(true) => String::new(),
        _ => format!(" The archive {archive_path} could not be put back; check it."),
    };
    Err(match target {
        Err(e) => format!("Writing the {what} failed ({e}); nothing moved.{undo}"),
        _ => {
            format!("The {what} changed while it was being {done}; nothing moved. Try again.{undo}")
        }
    })
}

// ── Archive older entries (overnight 10 A9): the person's button ────────
//
// Brief and Plan → Section options → Archive older entries… previews what
// can move ([`preview_workspace`]) and moves only what the person ticked
// (`archive_workspace`): brief days (the dated "Current context" entries
// `compact` would move by default, grouped by day) and PLAN.md `##`
// sections. Finished sections (at least one row, none open) are offered
// ticked; older dated sections with open rows, and "Waiting on you"
// sections, unticked. Text moves word for word to
// `outputs/workspace-archive/`, each file gets one pointer line, and the
// move is its own History run, so Undo puts every file back. Only the
// user-only `archive_older_entries` command applied it until the COO
// rebuild removed the button; it is test-only now (Alex, 10-19 21:59: no
// action, automation or agent may archive).

/// What the plan's pointer line says. `workspaces::plan_selection` never
/// cuts it.
pub(crate) const PLAN_POINTER: &str = "Finished sections were moved to";

/// The History run source of an archive (`historyRun.ts`, holder label
/// "Archive older entries").
pub const ARCHIVE_SOURCE: &str = "workspaceArchive";

/// The error when the files moved on since the preview.
const CHANGED_SINCE_PREVIEW: &str =
    "The brief or plan changed since the preview; nothing moved. Open Archive older entries again.";

/// One PLAN.md `##` section the person can archive.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct PlanCandidate {
    /// The heading's line, 1-based (as `planModel` counts lines).
    pub line: u32,
    /// The section's last non-blank line, 1-based.
    pub end_line: u32,
    /// The heading text, without the `##`.
    pub heading: String,
    /// SHA-256 of the heading line's text without its ending (its identity
    /// for the apply, like `planRowId`).
    pub heading_sha256: String,
    /// Checklist rows in the section, `###` subsections and nested rows
    /// included.
    pub open: u32,
    pub done: u32,
    /// The first date in the heading (`YYYY-MM-DD` or `MM-DD`), if any.
    pub date: Option<String>,
    /// A "Waiting on you" heading (`planModel.WAITING_HEADING`).
    pub waiting: bool,
    /// `finished` (ticked by default) or `older` (offered unticked).
    #[cfg_attr(test, ts(type = "\"finished\" | \"older\""))]
    pub class: String,
}

/// A `##` section's line range `[start, end)` (0-based, trailing blank
/// lines included) and its candidate, if it is one.
struct PlanSection {
    start: usize,
    end: usize,
    candidate: Option<PlanCandidate>,
}

/// "Waiting on you", "Waiting on the user", "Needs you", "Blocked on you",
/// "For you" (`planModel.ts` `WAITING_HEADING`).
fn waiting_heading(heading: &str) -> bool {
    static WAITING: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(
            r"(?i)\b(waiting (on|for) (you|the user|me|the owner)|needs? you|blocked on (you|the user|me)|for you)\b",
        )
        .expect("waiting heading")
    });
    WAITING.is_match(heading)
}

/// An ATX heading's level and text (`sectionMarkdown.ts` `ATX`).
fn atx(line: &str) -> Option<(usize, &str)> {
    static ATX: once_cell::sync::Lazy<regex::Regex> = once_cell::sync::Lazy::new(|| {
        regex::Regex::new(r"^ {0,3}(#{1,6})(?:[ \t]+(.*?))?(?:[ \t]+#+)?[ \t]*$").expect("atx")
    });
    let c = ATX.captures(line)?;
    let level = c.get(1)?.as_str().len();
    Some((level, c.get(2).map_or("", |m| m.as_str().trim())))
}

/// The first `YYYY-MM-DD` or `MM-DD` in `text` not glued to a digit before
/// it, with `leading_date`'s year rule.
fn first_date(text: &str, today: NaiveDate) -> Option<NaiveDate> {
    let mut prev: Option<char> = None;
    for (at, ch) in text.char_indices() {
        if ch.is_ascii_digit() && !prev.is_some_and(|p| p.is_ascii_digit()) {
            if let Some(date) = leading_date(&text[at..], today) {
                return Some(date);
            }
        }
        prev = Some(ch);
    }
    None
}

/// The plan's lines and endings, its `##` sections, and the line its lead
/// starts at (after the frontmatter). Headings and rows as `planModel`
/// reads them: outside fences and the frontmatter.
fn read_plan(
    text: &str,
    today: NaiveDate,
    keep_days: u32,
) -> (Vec<&str>, Vec<&'static str>, Vec<PlanSection>, usize) {
    let (lines, ends) = split_lines(text);
    // Per line (0-based): code, and the row state (Some(done)).
    let mut code = vec![false; lines.len()];
    let mut row: Vec<Option<bool>> = vec![None; lines.len()];
    let bytes = text.as_bytes();
    crate::plan_rows::visit_rows(text, |n, r| {
        let i = n as usize - 1;
        if i < lines.len() {
            code[i] = r.code;
            row[i] = r.box_at.map(|at| bytes.get(at).is_some_and(|b| *b != b' '));
        }
        true
    });
    let content_start = code.iter().take_while(|c| **c).count();
    let heading_at = |i: usize| -> Option<(usize, &str)> {
        if code[i] {
            return None;
        }
        let line = if i == 0 {
            lines[0].strip_prefix('\u{feff}').unwrap_or(lines[0])
        } else {
            lines[i]
        };
        atx(line).filter(|(level, _)| *level <= 2)
    };
    let heads: Vec<(usize, usize, &str)> = (0..lines.len())
        .filter_map(|i| heading_at(i).map(|(level, h)| (i, level, h)))
        .collect();
    let cutoff = today - chrono::Duration::days(i64::from(keep_days));
    let mut sections = Vec::new();
    for (k, &(start, level, heading)) in heads.iter().enumerate() {
        let end = heads.get(k + 1).map_or(lines.len(), |h| h.0);
        if level != 2 {
            continue;
        }
        let (mut open, mut done) = (0u32, 0u32);
        for state in row[start + 1..end].iter().flatten() {
            if *state {
                done += 1;
            } else {
                open += 1;
            }
        }
        let date = first_date(heading, today);
        let waiting = waiting_heading(heading);
        let finished = open == 0 && done > 0;
        let class = if finished && !waiting {
            Some("finished")
        } else if date.is_some_and(|d| d < cutoff) || (finished && waiting) {
            Some("older")
        } else {
            None
        };
        let last = (start..end)
            .rev()
            .find(|&i| !lines[i].trim().is_empty())
            .unwrap_or(start);
        let candidate = class.map(|class| PlanCandidate {
            line: (start + 1) as u32,
            end_line: (last + 1) as u32,
            heading: heading.to_string(),
            heading_sha256: crate::vault::revision(lines[start].as_bytes()),
            open,
            done,
            date: date.map(|d| d.to_string()),
            waiting,
            class: class.to_string(),
        });
        sections.push(PlanSection {
            start,
            end,
            candidate,
        });
    }
    (lines, ends, sections, content_start)
}

/// The `##` sections of `plan` the person may archive, in file order.
pub fn plan_sections(plan: &str, today: NaiveDate, keep_days: u32) -> Vec<PlanCandidate> {
    read_plan(plan, today, keep_days)
        .2
        .into_iter()
        .filter_map(|s| s.candidate)
        .collect()
}

/// What archiving plan sections does.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanMove {
    /// PLAN.md afterwards.
    pub plan: String,
    /// The moved sections, byte for byte, a blank line between them.
    pub archive: String,
    /// The moved headings.
    pub moved: Vec<String>,
}

/// PLAN.md without the candidate sections whose heading lines (1-based) are
/// `chosen`, and one [`PLAN_POINTER`] line at the end of the lead (before
/// the first `##`) naming `archive_rel`, replacing earlier pointer lines of
/// the generated shape there. Every other byte stays. `None` when none of
/// `chosen` is a candidate.
pub fn compact_plan_text(
    plan: &str,
    today: NaiveDate,
    chosen: &[u32],
    archive_rel: &str,
) -> Option<PlanMove> {
    let (lines, ends, sections, content_start) = read_plan(plan, today, DEFAULT_KEEP_DAYS);
    let nl = ends.iter().copied().find(|e| !e.is_empty()).unwrap_or("\n");
    let mut drop = vec![false; lines.len()];
    let mut parts = Vec::new();
    let mut moved = Vec::new();
    for s in &sections {
        let Some(c) = &s.candidate else { continue };
        if !chosen.contains(&c.line) {
            continue;
        }
        for flag in &mut drop[s.start..s.end] {
            *flag = true;
        }
        // Up to the end of the last non-blank line: its trailing spaces (a
        // Markdown hard break) stay.
        let mut text = String::new();
        let last = (s.start..s.end)
            .rev()
            .find(|&j| !lines[j].trim().is_empty())
            .unwrap_or(s.start);
        for j in s.start..=last {
            // A BOM stays at the top of PLAN.md, never mid-archive.
            text.push_str(if j == 0 {
                lines[0].trim_start_matches('\u{feff}')
            } else {
                lines[j]
            });
            if j < last {
                text.push_str(ends[j]);
            }
        }
        parts.push(text);
        moved.push(c.heading.clone());
    }
    if moved.is_empty() {
        return None;
    }
    let lead_end = sections.first().map_or(lines.len(), |s| s.start);
    let mut elsewhere = false;
    for i in content_start..lead_end {
        if is_pointer_line(lines[i], PLAN_POINTER) {
            drop[i] = true;
            elsewhere |= !lines[i].contains(archive_rel);
        }
    }
    let pointer = format!(
        "{PLAN_POINTER} `{archive_rel}` on {today}{}.",
        if elsewhere {
            format!("; earlier moves are beside it in `{ARCHIVE_DIR}`")
        } else {
            String::new()
        }
    );
    let last_lead = (content_start..lead_end)
        .rev()
        .find(|&i| !drop[i] && !lines[i].trim().is_empty());
    // The kept lines after the last dropped one, when the drop reached the
    // end: blank lines left at the end of the file go too.
    let tail_dropped = drop.last().copied().unwrap_or(false);
    // When line 0 goes (moved, or an old pointer) or the pointer goes above
    // it, its BOM moves to whatever line comes first.
    let bom_top = content_start == 0
        && lines.first().is_some_and(|l| l.starts_with('\u{feff}'))
        && (last_lead.is_none() || drop[0]);
    let mut out: Vec<(String, &str)> = Vec::with_capacity(lines.len() + 2);
    let mut dropped_since = false;
    for (i, line) in lines.iter().enumerate() {
        let line: &str = if bom_top && i == 0 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if i == content_start && last_lead.is_none() {
            out.push((pointer.clone(), nl));
            out.push((String::new(), nl));
        }
        if drop[i] {
            dropped_since = true;
            continue;
        }
        // A blank line a drop left at the top or next to another blank goes.
        if dropped_since
            && line.trim().is_empty()
            && out.last().is_none_or(|(l, _)| l.trim().is_empty())
        {
            continue;
        }
        out.push(((*line).to_string(), ends[i]));
        dropped_since = false;
        if Some(i) == last_lead {
            if out.last().is_some_and(|(_, e)| e.is_empty()) {
                out.last_mut().expect("just pushed").1 = nl;
            }
            out.push((String::new(), nl));
            out.push((pointer.clone(), nl));
        }
    }
    if tail_dropped {
        while out.len() > 1 && out.last().is_some_and(|(l, _)| l.trim().is_empty()) {
            out.pop();
        }
        if ends.last().is_some_and(|e| e.is_empty()) {
            // The plan ended without a newline; so does the result.
            if let Some(last) = out.last_mut() {
                last.1 = "";
            }
        }
    }
    if bom_top {
        if let Some(first) = out.first_mut() {
            first.0.insert(0, '\u{feff}');
        }
    }
    Some(PlanMove {
        plan: out.iter().map(|(l, e)| format!("{l}{e}")).collect(),
        archive: parts.join(&format!("{nl}{nl}")),
        moved,
    })
}

/// One brief day that can move: its date, how many entries, and up to three
/// of their first lines (at most 100 characters each).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ArchiveDay {
    pub date: String,
    pub count: u32,
    pub headlines: Vec<String>,
}

/// What Archive older entries offers (`archive_preview`).
#[derive(Serialize, Clone, Debug, PartialEq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg(test)]
pub struct ArchivePreview {
    /// The day the preview was made for: the apply uses it, so it moves what
    /// was shown even after midnight.
    pub today: String,
    pub brief_revision: String,
    /// `None` when the workspace has no PLAN.md.
    pub plan_revision: Option<String>,
    /// Brief days, oldest first: every one is offered ticked.
    pub days: Vec<ArchiveDay>,
    /// Plan sections, in file order.
    pub plan: Vec<PlanCandidate>,
    pub brief_bytes: u32,
    pub plan_bytes: u32,
    /// A run is live in the workspace now: the apply would be refused.
    pub busy: bool,
}

/// A plan section the person ticked: its heading line and that line's
/// SHA-256, from the preview.
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlanChoice {
    pub line: u32,
    pub sha256: String,
}

/// What the person ticked (`archive_older_entries`).
#[derive(Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
// `today` is read only by the test-only `archive_workspace`; the shared-docs
// apply passes its own day to `apply_archive`.
#[cfg_attr(not(test), allow(dead_code))]
pub struct ArchiveArgs {
    /// The preview's `today`.
    pub today: String,
    pub brief_revision: String,
    pub plan_revision: Option<String>,
    pub brief_days: Vec<String>,
    pub plan_sections: Vec<PlanChoice>,
}

/// What an archive did. Anything written comes with `runId`, so Undo
/// (History & recovery) can put it back.
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct ArchiveDone {
    pub run_id: String,
    pub brief_moved: u32,
    pub plan_moved: u32,
    pub brief_archive: Option<String>,
    pub plan_archive: Option<String>,
    pub brief_before: u32,
    pub brief_after: u32,
    pub plan_before: u32,
    pub plan_after: u32,
    /// For example "brief archived; plan unchanged: …".
    pub warnings: Vec<String>,
}

fn len32(text: &str) -> u32 {
    u32::try_from(text.len()).unwrap_or(u32::MAX)
}

/// The workspace's brief and plan as the archive reads them.
struct Files {
    folder: String,
    title: String,
    brief_rel: String,
    brief: crate::types::NoteContent,
    plan_rel: String,
    plan: Option<crate::types::NoteContent>,
}

fn read_files(vault: &Path, workspace: &str) -> Result<Files, String> {
    let summary = crate::workspaces::summary(vault, workspace)?;
    let brief_rel = format!("{}/WORKSPACE.md", summary.path);
    let plan_rel = format!("{}/PLAN.md", summary.path);
    let brief = crate::workspaces::read_file(vault, &brief_rel)?;
    let plan = if crate::vault::safe_join(vault, &plan_rel)?.exists() {
        Some(crate::workspaces::read_file(vault, &plan_rel)?)
    } else {
        None
    };
    Ok(Files {
        folder: summary.path,
        title: summary.title,
        brief_rel,
        brief,
        plan_rel,
        plan,
    })
}

/// The brief entries `compact` would move by default, by day, oldest first.
fn brief_days(brief: &str, today: NaiveDate) -> Vec<ArchiveDay> {
    let lines: Vec<&str> = split_lines(brief).0;
    let Some((start, end)) = move_section(&lines) else {
        return Vec::new();
    };
    let all = entries(&lines, start, end, today);
    let mut days: std::collections::BTreeMap<NaiveDate, ArchiveDay> = Default::default();
    for i in to_move(&all, today, DEFAULT_KEEP_DAYS, DEFAULT_KEEP_NEWEST) {
        let Some(date) = all[i].date else { continue };
        let day = days.entry(date).or_insert_with(|| ArchiveDay {
            date: date.to_string(),
            count: 0,
            headlines: Vec::new(),
        });
        day.count += 1;
        if day.headlines.len() < 3 {
            day.headlines
                .push(lines[all[i].start].trim().chars().take(100).collect());
        }
    }
    days.into_values().collect()
}

/// `archive_preview`: what Archive older entries would offer in
/// `workspace` today. Writes nothing.
#[cfg(test)]
pub fn preview_workspace(
    vault: &Path,
    workspace: &str,
    today: NaiveDate,
) -> Result<ArchivePreview, String> {
    let files = read_files(vault, workspace)?;
    let plan_text = files.plan.as_ref().map_or("", |p| p.content.as_str());
    Ok(ArchivePreview {
        today: today.to_string(),
        brief_revision: files.brief.revision.clone(),
        plan_revision: files.plan.as_ref().map(|p| p.revision.clone()),
        days: brief_days(&files.brief.content, today),
        plan: plan_sections(plan_text, today, DEFAULT_KEEP_DAYS),
        brief_bytes: len32(&files.brief.content),
        plan_bytes: len32(plan_text),
        busy: held_by_another(vault, &files.folder, None),
    })
}

/// `archive_older_entries`: move what the person ticked, as its own History
/// run. Refused, with nothing written, while a run is live in the
/// workspace or when either file changed since the preview (or a ticked
/// day or section is no longer offered). The brief half goes first, then
/// the plan half, each archive-first with rollback; a plan half that fails
/// after the brief moved is a warning, never an error, so the returned
/// `runId` always covers what moved.
#[cfg(test)]
pub fn archive_workspace(
    vault: &Path,
    workspace: &str,
    args: &ArchiveArgs,
) -> Result<ArchiveDone, String> {
    use crate::automated_history::BeginRunError;
    let today = NaiveDate::parse_from_str(&args.today, "%Y-%m-%d")
        .map_err(|_| format!("Not a date: {}; nothing moved.", args.today))?;
    if args.brief_days.is_empty() && args.plan_sections.is_empty() {
        return Err("Nothing was ticked; nothing moved.".into());
    }
    let summary = crate::workspaces::summary(vault, workspace)?;
    // A full-vault run (an automation, a vault-root chat) would lose its own
    // changes in this folder from its History once this run released it, so
    // the archive waits for it too (A9 review).
    if let Some(holder) = crate::automated_history::lease_holder(vault) {
        return Err(format!("{} Nothing moved.", holder.busy_message()));
    }
    let lease = match crate::automated_history::try_begin_scoped(
        vault,
        &summary.path,
        ARCHIVE_SOURCE,
        None,
    ) {
        Ok(lease) => lease,
        Err(BeginRunError::Busy(_)) => return Err(
            "A conversation is running in this workspace; archive once it finishes. Nothing moved."
                .into(),
        ),
        Err(BeginRunError::Failed(e)) => return Err(format!("{e}; nothing moved.")),
    };
    let result = apply_archive(vault, workspace, args, today);
    match result {
        Ok(mut done) => {
            done.run_id = lease.id().to_string();
            if let Err(e) = lease.finish("success") {
                done.warnings.push(format!(
                    "History could not record this archive ({e}); the archive notes hold the moved text."
                ));
            }
            Ok(done)
        }
        Err(e) => {
            let _ = lease.finish("cancelled");
            Err(e)
        }
    }
}

/// The checks and both halves of `archive_workspace`, inside its run.
fn apply_archive(
    vault: &Path,
    workspace: &str,
    args: &ArchiveArgs,
    today: NaiveDate,
) -> Result<ArchiveDone, String> {
    let files = read_files(vault, workspace)?;
    let plan_text = files.plan.as_ref().map_or("", |p| p.content.as_str());
    if files.brief.revision != args.brief_revision
        || files.plan.as_ref().map(|p| &p.revision) != args.plan_revision.as_ref()
    {
        return Err(CHANGED_SINCE_PREVIEW.into());
    }
    // Every ticked day and section must still be offered.
    let mut days = BTreeSet::new();
    for day in &args.brief_days {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|_| format!("Not a date: {day}; nothing moved."))?;
        days.insert(date);
    }
    let offered: BTreeSet<NaiveDate> = brief_days(&files.brief.content, today)
        .iter()
        .filter_map(|d| NaiveDate::parse_from_str(&d.date, "%Y-%m-%d").ok())
        .collect();
    let candidates = plan_sections(plan_text, today, DEFAULT_KEEP_DAYS);
    let chosen_ok = args.plan_sections.iter().all(|choice| {
        candidates
            .iter()
            .any(|c| c.line == choice.line && c.heading_sha256 == choice.sha256)
    });
    if !days.is_subset(&offered) || !chosen_ok {
        return Err(CHANGED_SINCE_PREVIEW.into());
    }
    let mut done = ArchiveDone {
        brief_before: len32(&files.brief.content),
        brief_after: len32(&files.brief.content),
        plan_before: len32(plan_text),
        plan_after: len32(plan_text),
        ..ArchiveDone::default()
    };
    if !days.is_empty() {
        let archive_rel = format!("{ARCHIVE_DIR}{today}-current-context.md");
        let plan = compact_days(
            &files.brief.content,
            today,
            DEFAULT_KEEP_DAYS,
            DEFAULT_KEEP_NEWEST,
            &archive_rel,
            Some(&days),
        )
        .ok_or_else(|| CHANGED_SINCE_PREVIEW.to_string())?;
        let archive_path = format!("{}/{archive_rel}", files.folder);
        let header = format!(
            "# Current context archive ({})\n\nOlder entries moved out of the brief's \"Current context\" by Archive older entries, word for word.\n\n",
            files.title
        );
        let section = format!(
            "## Moved from WORKSPACE.md \"Current context\" on {today}\n\n{}\n",
            plan.archive
        );
        move_into_archive(
            vault,
            &archive_path,
            &header,
            &section,
            &files.brief_rel,
            &plan.brief,
            &files.brief.revision,
            "brief",
            "archived",
        )?;
        done.brief_moved = plan.moved.len() as u32;
        done.brief_after = len32(&plan.brief);
        done.brief_archive = Some(archive_path);
    }
    if !args.plan_sections.is_empty() {
        let archive_rel = format!("{ARCHIVE_DIR}{today}-plan.md");
        let lines: Vec<u32> = args.plan_sections.iter().map(|c| c.line).collect();
        let half = compact_plan_text(plan_text, today, &lines, &archive_rel)
            .ok_or_else(|| CHANGED_SINCE_PREVIEW.to_string())
            .and_then(|moved| {
                let archive_path = format!("{}/{archive_rel}", files.folder);
                let header = format!(
                    "# Plan archive ({})\n\nSections moved out of PLAN.md by Archive older entries, word for word.\n\n",
                    files.title
                );
                let section = format!("## Moved from PLAN.md on {today}\n\n{}\n", moved.archive);
                let revision = files.plan.as_ref().map_or("", |p| p.revision.as_str());
                move_into_archive(
                    vault,
                    &archive_path,
                    &header,
                    &section,
                    &files.plan_rel,
                    &moved.plan,
                    revision,
                    "plan",
                    "archived",
                )
                .map(|()| (moved, archive_path))
            });
        match half {
            Ok((moved, archive_path)) => {
                done.plan_moved = moved.moved.len() as u32;
                done.plan_after = len32(&moved.plan);
                done.plan_archive = Some(archive_path);
            }
            Err(why) if done.brief_moved > 0 => {
                done.warnings
                    .push(format!("Brief archived; plan unchanged: {why}"));
            }
            Err(why) => return Err(why),
        }
    }
    Ok(done)
}

// ── `workspaces.archive_shared_docs` (V5a): one shared doc at a time ─────

/// Which shared doc to archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedDoc {
    /// WORKSPACE.md: older dated "Current context" entries move; Outcome,
    /// the rest of the context and Decisions stay.
    Workspace,
    /// PLAN.md: sections with every row ticked move; open work stays.
    Plan,
}

impl SharedDoc {
    pub fn parse(name: &str) -> Result<Self, String> {
        match name.trim() {
            "WORKSPACE.md" => Ok(Self::Workspace),
            "PLAN.md" => Ok(Self::Plan),
            other => Err(format!(
                "doc must be WORKSPACE.md or PLAN.md, not {other:?}; nothing moved."
            )),
        }
    }

    pub fn file(self) -> &'static str {
        match self {
            Self::Workspace => "WORKSPACE.md",
            Self::Plan => "PLAN.md",
        }
    }
}

/// What a dry run shows and what an apply did (`workspaces.archive_shared_docs`).
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SharedDocArchive {
    pub workspace: String,
    pub doc: String,
    /// `dry_run` or `apply`.
    pub mode: String,
    /// The day the archive note is named for.
    pub today: String,
    /// SHA-256 of the doc's bytes now: the compare-and-swap `expectedHash`
    /// an apply must send back.
    pub hash: String,
    /// The doc's hash after an apply (the same as `hash` when nothing moved).
    pub new_hash: Option<String>,
    pub bytes_before: u32,
    pub bytes_after: u32,
    /// Bytes of text that move (or moved) into the archive note.
    pub archive_bytes: u32,
    /// The archive note's vault path, when something moves.
    pub archive_path: Option<String>,
    /// First lines of the entries (or headings of the sections) that move.
    pub moved: Vec<String>,
    pub moved_count: u32,
    /// Kept text plus archived text reproduces the original, line for line.
    pub lossless: bool,
    pub applied: bool,
    /// A run is live in the workspace now (an apply from the app would wait).
    pub busy: bool,
    pub run_id: Option<String>,
    pub warnings: Vec<String>,
}

/// The doc after the move, and the text that goes to the archive.
struct Prepared {
    new_text: String,
    archive: String,
    moved: Vec<String>,
    archive_rel: String,
    /// The plan sections to tick, or the brief's days.
    args: ArchiveArgs,
}

/// Non-blank lines (CR stripped) of `text` counted, leaving out this
/// module's own pointer lines, which an archive adds and replaces.
fn line_counts(text: &str) -> std::collections::BTreeMap<String, u32> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let line = line.strip_prefix('\u{feff}').unwrap_or(line);
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty()
            || is_pointer_line(line, POINTER)
            || is_pointer_line(line, PLAN_POINTER)
        {
            continue;
        }
        *out.entry(line.to_string()).or_insert(0u32) += 1;
    }
    out
}

/// Whether `kept` and `archived` together are exactly `original`: no line
/// lost, none invented (blank lines and pointer lines aside).
#[allow(clippy::let_and_return)]
pub(crate) fn is_lossless(original: &str, kept: &str, archived: &str) -> bool {
    let (orig, kept, arch) = (
        line_counts(original),
        line_counts(kept),
        line_counts(archived),
    );
    let mut keys: BTreeSet<&String> = orig.keys().collect();
    keys.extend(kept.keys());
    keys.extend(arch.keys());
    let ok = keys.into_iter().all(|k| {
        let n = |m: &std::collections::BTreeMap<String, u32>| m.get(k).copied().unwrap_or(0);
        n(&orig) == n(&kept) + n(&arch)
    });
    ok
}

fn prepare(files: &Files, doc: SharedDoc, today: NaiveDate) -> Result<Option<Prepared>, String> {
    let mut args = ArchiveArgs {
        today: today.to_string(),
        brief_revision: files.brief.revision.clone(),
        plan_revision: files.plan.as_ref().map(|p| p.revision.clone()),
        brief_days: Vec::new(),
        plan_sections: Vec::new(),
    };
    match doc {
        SharedDoc::Workspace => {
            let archive_rel = format!("{ARCHIVE_DIR}{today}-current-context.md");
            let Some(plan) = compact_days(
                &files.brief.content,
                today,
                DEFAULT_KEEP_DAYS,
                DEFAULT_KEEP_NEWEST,
                &archive_rel,
                None,
            ) else {
                return Ok(None);
            };
            args.brief_days = brief_days(&files.brief.content, today)
                .into_iter()
                .map(|d| d.date)
                .collect();
            let moved: Vec<String> = plan
                .moved
                .iter()
                .map(|(_, line)| line.trim().chars().take(100).collect())
                .collect();
            Ok(Some(Prepared {
                new_text: plan.brief,
                archive: plan.archive,
                moved,
                archive_rel,
                args,
            }))
        }
        SharedDoc::Plan => {
            let Some(plan) = &files.plan else {
                return Err("This workspace has no PLAN.md; nothing moved.".into());
            };
            let chosen: Vec<PlanCandidate> = plan_sections(&plan.content, today, DEFAULT_KEEP_DAYS)
                .into_iter()
                .filter(|c| c.class == "finished")
                .collect();
            if chosen.is_empty() {
                return Ok(None);
            }
            let archive_rel = format!("{ARCHIVE_DIR}{today}-plan.md");
            let lines: Vec<u32> = chosen.iter().map(|c| c.line).collect();
            let Some(moved) = compact_plan_text(&plan.content, today, &lines, &archive_rel) else {
                return Ok(None);
            };
            args.plan_sections = chosen
                .iter()
                .map(|c| PlanChoice {
                    line: c.line,
                    sha256: c.heading_sha256.clone(),
                })
                .collect();
            Ok(Some(Prepared {
                new_text: moved.plan,
                archive: moved.archive,
                moved: moved.moved,
                archive_rel,
                args,
            }))
        }
    }
}

/// The day a dry run or apply works for: the `today` a dry run returned
/// (so an apply moves exactly what was previewed, even after midnight), or
/// today's date when none is given.
pub fn parse_today(today: Option<&str>) -> Result<NaiveDate, String> {
    parse_today_at(today, chrono::Local::now().date_naive())
}

/// [`parse_today`] with `now` given. A day other than `now` or the day
/// before is refused: an agent must not pick a far-off day and so move
/// almost everything (an apply that waited past midnight is the case this
/// allows).
pub fn parse_today_at(today: Option<&str>, now: NaiveDate) -> Result<NaiveDate, String> {
    let Some(t) = today.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(now);
    };
    let day = NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .map_err(|_| format!("today must be YYYY-MM-DD, not {t:?}; nothing moved."))?;
    if day != now && day != now - chrono::Duration::days(1) {
        return Err(format!(
            "today must be {now} or the day before ({}), not {day}; nothing moved. Run the dry run again.",
            now - chrono::Duration::days(1)
        ));
    }
    Ok(day)
}

/// Whether another run is live where the doc lives. `own_run` is the
/// caller's own run (an agent's, or the app's archive run), which never
/// counts; without one, a full-vault run counts too.
fn held_by_another(vault: &Path, folder: &str, own_run: Option<&str>) -> bool {
    let others = crate::automated_history::scope_holders(vault, folder)
        .into_iter()
        .any(|id| Some(id.as_str()) != own_run);
    others || (own_run.is_none() && crate::automated_history::lease_holder(vault).is_some())
}

fn shared_doc_report(
    vault: &Path,
    workspace: &str,
    doc: SharedDoc,
    today: NaiveDate,
    own_run: Option<&str>,
    mode: &str,
) -> Result<(Files, Option<Prepared>, SharedDocArchive), String> {
    let files = read_files(vault, workspace)?;
    let current = match doc {
        SharedDoc::Workspace => &files.brief,
        SharedDoc::Plan => files
            .plan
            .as_ref()
            .ok_or_else(|| "This workspace has no PLAN.md; nothing moved.".to_string())?,
    };
    let prepared = prepare(&files, doc, today)?;
    let before = len32(&current.content);
    let mut report = SharedDocArchive {
        workspace: workspace.to_string(),
        doc: doc.file().to_string(),
        mode: mode.to_string(),
        today: today.to_string(),
        hash: current.revision.clone(),
        bytes_before: before,
        bytes_after: before,
        lossless: true,
        busy: held_by_another(vault, &files.folder, own_run),
        ..SharedDocArchive::default()
    };
    if let Some(p) = &prepared {
        report.bytes_after = len32(&p.new_text);
        report.archive_bytes = len32(&p.archive);
        report.archive_path = Some(format!("{}/{}", files.folder, p.archive_rel));
        report.moved = p.moved.clone();
        report.moved_count = p.moved.len() as u32;
        report.lossless = is_lossless(&current.content, &p.new_text, &p.archive);
    }
    Ok((files, prepared, report))
}

/// The dry run: sizes before and after, what would move, and the hash an
/// apply must send back. Writes nothing.
pub fn dry_run_shared_doc(
    vault: &Path,
    workspace: &str,
    doc: SharedDoc,
    today: NaiveDate,
    own_run: Option<&str>,
) -> Result<SharedDocArchive, String> {
    shared_doc_report(vault, workspace, doc, today, own_run, "dry_run").map(|(_, _, r)| r)
}

/// The apply, inside a run the caller already holds (an agent's own run:
/// History records the change there). Refused, with nothing written, when
/// another run is live in the workspace (`own_run` is the caller's own and
/// never counts), the doc's hash is no longer `expected_hash`, the move would not be
/// lossless, or when the archive's own checks fail (archive note first,
/// then the doc, each by compare-and-swap, the note put back if the doc
/// write fails). Nothing older to move is a success that writes nothing, so
/// a second apply is a no-op.
pub fn apply_shared_doc_in_run(
    vault: &Path,
    workspace: &str,
    doc: SharedDoc,
    expected_hash: &str,
    today: NaiveDate,
    own_run: Option<&str>,
) -> Result<SharedDocArchive, String> {
    let (files, prepared, mut report) =
        shared_doc_report(vault, workspace, doc, today, own_run, "apply")?;
    if report.busy {
        return Err(
            "Another run is working in this workspace; archive once it finishes. Nothing moved."
                .into(),
        );
    }
    if report.hash != expected_hash.trim() {
        return Err(format!(
            "{} changed since the dry run; nothing moved. Run the dry run again.",
            doc.file()
        ));
    }
    report.new_hash = Some(report.hash.clone());
    let Some(prepared) = prepared else {
        return Ok(report);
    };
    if !report.lossless {
        return Err(format!(
            "The move would not reproduce {} line for line; nothing moved.",
            doc.file()
        ));
    }
    let mut args = prepared.args;
    // Only the chosen doc is held to its hash; the other is read just now.
    args.brief_revision = files.brief.revision.clone();
    args.plan_revision = files.plan.as_ref().map(|p| p.revision.clone());
    let done = apply_archive(vault, workspace, &args, today)?;
    let after = read_files(vault, workspace)?;
    let text = match doc {
        SharedDoc::Workspace => Some(&after.brief),
        SharedDoc::Plan => after.plan.as_ref(),
    };
    if let Some(note) = text {
        report.new_hash = Some(note.revision.clone());
        report.bytes_after = len32(&note.content);
        if note.content != prepared.new_text {
            report.warnings.push(format!(
                "{} was written, but not exactly as the dry run showed.",
                doc.file()
            ));
        }
    }
    report.applied = true;
    report.warnings.extend(done.warnings);
    Ok(report)
}

/// The apply from the app: its own History run ("Archive older entries"),
/// refused while a run is live in the workspace.
pub fn apply_shared_doc(
    vault: &Path,
    workspace: &str,
    doc: SharedDoc,
    expected_hash: &str,
    today: NaiveDate,
) -> Result<SharedDocArchive, String> {
    use crate::automated_history::BeginRunError;
    let summary = crate::workspaces::summary(vault, workspace)?;
    if let Some(holder) = crate::automated_history::lease_holder(vault) {
        return Err(format!("{} Nothing moved.", holder.busy_message()));
    }
    let lease = match crate::automated_history::try_begin_scoped(
        vault,
        &summary.path,
        ARCHIVE_SOURCE,
        None,
    ) {
        Ok(lease) => lease,
        Err(BeginRunError::Busy(_)) => return Err(
            "A conversation is running in this workspace; archive once it finishes. Nothing moved."
                .into(),
        ),
        Err(BeginRunError::Failed(e)) => return Err(format!("{e}; nothing moved.")),
    };
    let own = lease.id().to_string();
    match apply_shared_doc_in_run(vault, workspace, doc, expected_hash, today, Some(&own)) {
        Ok(mut report) => {
            report.run_id = Some(lease.id().to_string());
            if let Err(e) = lease.finish("success") {
                report.warnings.push(format!(
                    "History could not record this archive ({e}); the archive notes hold the moved text."
                ));
            }
            Ok(report)
        }
        Err(e) => {
            let _ = lease.finish("cancelled");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    const MAIN: &str = "---\ntitle: Main\n---\n\n# Main\n\n## Current context\n\nAs of 2026-10-08, 15:15 ET:\n- Undated: always here.\n- Notion connector now shared (2026-10-08): a standing fact.\n- 10-01 09:00 ET: oldest.\n  continued line\n- 10-02 10:00 ET: old.\n- **10-08 21:25 ET OVERNIGHT** recent, see outputs/2026-10-10/x.\n- 10-09 13:35 ET: newer.\n- 2026-10-10 00:30: newest.\n";

    #[test]
    fn dated_bullets_move_byte_for_byte_and_standing_facts_stay() {
        let today = d("2026-10-10");
        let plan = compact(MAIN, today, 1, 2, "outputs/workspace-archive/x.md").unwrap();
        assert_eq!(plan.moved.len(), 3, "{:?}", plan.moved);
        assert_eq!(
            plan.archive,
            "- 10-01 09:00 ET: oldest.\n  continued line\n- 10-02 10:00 ET: old.\n- **10-08 21:25 ET OVERNIGHT** recent, see outputs/2026-10-10/x."
        );
        assert_eq!(
            plan.brief,
            "---\ntitle: Main\n---\n\n# Main\n\n## Current context\n\nAs of 2026-10-08, 15:15 ET:\n- Undated: always here.\n- Notion connector now shared (2026-10-08): a standing fact.\n- 10-09 13:35 ET: newer.\n- 2026-10-10 00:30: newest.\n- Earlier entries were moved to `outputs/workspace-archive/x.md` on 2026-10-10 (through 2026-10-08).\n"
        );
        assert!(
            compact(&plan.brief, today, 1, 2, "outputs/workspace-archive/x.md").is_none(),
            "nothing left to move"
        );
    }

    /// The hollow brief's shape: bold-date paragraphs (some with bullet
    /// lines inside), a `**Next:**` paragraph with bullets, and a closing
    /// paragraph naming dates mid-line.
    const HOLLOW: &str = "# h\n\n## Outcome\n\nX.\n\n## Current context\n\n**2026-10-10 overnight: B1 merged.** Text.\n- a sub-point\n\n**2026-10-09 19:22 EDT: built.** More.\n\n**2026-10-05: old one.** Older.\n- its own bullet\n\n**2026-10-04: older.** Text.\n\n**Next:**\n- do this\n- and that\n\nCI is advisory since #329. Older context entries (2026-10-01 to 10-04) are in [archive](outputs/a.md).\n\n## Decisions\n\n- keep\n";

    #[test]
    fn bold_paragraphs_move_whole_and_the_rest_stays_byte_for_byte() {
        let today = d("2026-10-10");
        let plan = compact(HOLLOW, today, 1, 2, "outputs/workspace-archive/y.md").unwrap();
        assert_eq!(plan.moved.len(), 2, "{:?}", plan.moved);
        assert_eq!(
            plan.archive,
            "**2026-10-05: old one.** Older.\n- its own bullet\n\n**2026-10-04: older.** Text."
        );
        assert_eq!(
            plan.brief,
            "# h\n\n## Outcome\n\nX.\n\n## Current context\n\n**2026-10-10 overnight: B1 merged.** Text.\n- a sub-point\n\n**2026-10-09 19:22 EDT: built.** More.\n\n**Next:**\n- do this\n- and that\n\nCI is advisory since #329. Older context entries (2026-10-01 to 10-04) are in [archive](outputs/a.md).\n\nEarlier entries were moved to `outputs/workspace-archive/y.md` on 2026-10-10 (through 2026-10-05).\n\n## Decisions\n\n- keep\n"
        );
        // A second run the same day replaces the pointer rather than adding one.
        let more = plan
            .brief
            .replace("**Next:**", "**2026-10-03: found later.** X.\n\n**Next:**");
        let again = compact(&more, today, 1, 2, "outputs/workspace-archive/y.md").unwrap();
        assert_eq!(again.brief.matches(POINTER).count(), 1, "{}", again.brief);
        assert!(!again.brief.contains("\n\n\n"), "{}", again.brief);
    }

    #[test]
    fn dates_are_leading_only_and_strict() {
        let t = d("2027-01-02");
        assert_eq!(date_of("- 12-30 09:00: x", t), Some(d("2026-12-30")));
        assert_eq!(date_of("- 01-02: x", t), Some(d("2027-01-02")));
        assert_eq!(
            date_of("**2026-10-09 19:22 EDT: x**", t),
            Some(d("2026-10-09"))
        );
        assert_eq!(date_of("- **10-09 21:25 ET** y", t), Some(d("2026-10-09")));
        assert_eq!(date_of("- Notion (2026-10-08) z", t), None);
        assert_eq!(date_of(" 2026-10-1 x", t), None);
        assert_eq!(date_of("- 100-200 things", t), None);
        assert_eq!(date_of("- 10-099 x", t), None);
        assert_eq!(date_of("As of 2026-10-08", t), None);
        assert!(compact("# W\n\nNo section.\n", t, 1, 0, "a.md").is_none());
        // CRLF briefs keep their line endings.
        let crlf = MAIN.replace('\n', "\r\n");
        let plan = compact(&crlf, d("2026-10-10"), 1, 2, "a.md").unwrap();
        assert!(
            !plan.brief.replace("\r\n", "").contains('\n'),
            "only CRLF line endings"
        );
    }

    #[test]
    fn each_line_keeps_its_own_ending_and_no_newline_is_invented() {
        let t = d("2026-10-10");
        let mixed =
            "# W\r\n\n## Current context\n\n- 10-01: old\r\n- 10-09: new\n## Decisions\r\n- keep";
        let plan = compact(mixed, t, 1, 1, "a.md").unwrap();
        assert_eq!(
            plan.brief,
            "# W\r\n\n## Current context\n\n- 10-09: new\n- Earlier entries were moved to `a.md` on 2026-10-10 (through 2026-10-01).\r\n## Decisions\r\n- keep"
        );
        // The section running to an end with no final newline keeps none.
        let plan = compact(
            "## Current context\n- 10-01: old\n- 10-09: new",
            t,
            1,
            1,
            "a.md",
        )
        .unwrap();
        assert!(
            plan.brief.ends_with("(through 2026-10-01)."),
            "{:?}",
            plan.brief
        );
        assert!(plan.brief.contains("- 10-09: new\n- Earlier"));
    }

    #[test]
    fn drops_leave_single_blank_lines_and_one_pointer_however_many_days() {
        let t = d("2026-10-10");
        // (a) A bullet block that moves whole, among paragraphs.
        let a = "## Current context\n\nP undated.\n\n- 10-01: x\n- 10-02: y\n\nQ undated.\n";
        let plan = compact(a, t, 1, 0, "w/a.md").unwrap();
        assert_eq!(
            plan.brief,
            "## Current context\n\nP undated.\n\nQ undated.\n\nEarlier entries were moved to `w/a.md` on 2026-10-10 (through 2026-10-02).\n"
        );
        // (b) A moved paragraph right before an earlier pointer, which is
        // replaced; an earlier day's pointer names the folder.
        let b = "## Current context\n\nP undated.\n\n**10-03: m.** x\n\nEarlier entries were moved to `outputs/workspace-archive/old.md` on 2026-10-04 (through 2026-10-02).\n\n## Next\n";
        let plan = compact(b, t, 1, 0, "outputs/workspace-archive/a.md").unwrap();
        assert_eq!(
            plan.brief,
            "## Current context\n\nP undated.\n\nEarlier entries were moved to `outputs/workspace-archive/a.md` on 2026-10-10 (through 2026-10-03); earlier moves are beside it in `outputs/workspace-archive/`.\n\n## Next\n"
        );
        // The brief's own blank runs stay where nothing was dropped.
        let c = "## Current context\n\nP.\n\n\nQ.\n\n**10-01: m.** x\n";
        let plan = compact(c, t, 1, 0, "a.md").unwrap();
        assert!(
            plan.brief.contains("P.\n\n\nQ.\n\nEarlier"),
            "{:?}",
            plan.brief
        );
    }

    /// A real workspace whose brief body is [`MAIN`]: (id, folder, brief path, brief).
    fn seeded(v: &Path) -> (String, std::path::PathBuf, std::path::PathBuf, String) {
        let w = crate::workspaces::create(v, "Main-ish", "x").unwrap();
        let brief_path = v.join(&w.path).join("WORKSPACE.md");
        let real = std::fs::read_to_string(&brief_path).unwrap();
        let fm_end = real[3..].find("\n---").unwrap() + 3 + 4;
        let body_at = MAIN[3..].find("\n---").unwrap() + 3 + 4;
        let brief = format!("{}{}", &real[..fm_end], &MAIN[body_at..]);
        std::fs::write(&brief_path, &brief).unwrap();
        (w.id, v.join(&w.path), brief_path, brief)
    }

    // ── Archive older entries (overnight 10 A9) ─────────────────────────

    thread_local! {
        /// A vault-relative path whose target write fails (the archive
        /// write before it goes through).
        pub(super) static FAIL_WRITE: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    const PLAN: &str =
        "# Plan\n\nLead.\n\n## Done (10-01)\n- [x] a\n- [x] b\n\n## Open\n- [ ] c\n- [x] d\n";
    const ARCH: &str = "outputs/workspace-archive/2026-10-20-plan.md";

    fn lines_of(plan: &[PlanCandidate]) -> Vec<u32> {
        plan.iter().map(|c| c.line).collect()
    }

    #[test]
    fn a_finished_plan_section_moves_and_an_open_one_stays_byte_for_byte() {
        let today = d("2026-10-20");
        let offered = plan_sections(PLAN, today, 1);
        assert_eq!(offered.len(), 1, "{offered:?}");
        assert_eq!(
            (offered[0].line, offered[0].class.as_str(), offered[0].done),
            (5, "finished", 2)
        );
        let moved = compact_plan_text(PLAN, today, &lines_of(&offered), ARCH).unwrap();
        assert_eq!(moved.archive, "## Done (10-01)\n- [x] a\n- [x] b");
        assert_eq!(
            moved.plan,
            format!("# Plan\n\nLead.\n\n{PLAN_POINTER} `{ARCH}` on 2026-10-20.\n\n## Open\n- [ ] c\n- [x] d\n")
        );
        let again = plan_sections(&moved.plan, today, 1);
        assert!(again.is_empty(), "{again:?}");
        assert!(compact_plan_text(&moved.plan, today, &[5], ARCH).is_none());
    }

    #[test]
    fn older_sections_with_open_rows_are_offered_never_chosen() {
        let plan = "# Plan\n\n## Overnight 2026-10-11/12 · Lane V\n- [ ] left\n- [x] done\n\n## Notes (10-15)\nNo rows.\n\n## Today (10-19)\n- [ ] new\n";
        let offered = plan_sections(plan, d("2026-10-20"), 1);
        let got: Vec<(&str, &str, u32)> = offered
            .iter()
            .map(|c| (c.heading.as_str(), c.class.as_str(), c.open))
            .collect();
        assert_eq!(
            got,
            [
                ("Overnight 2026-10-11/12 · Lane V", "older", 1),
                ("Notes (10-15)", "older", 0)
            ]
        );
        assert_eq!(offered[0].date.as_deref(), Some("2026-10-11"));
    }

    #[test]
    fn waiting_on_you_and_dateless_rowless_sections_are_not_preselected() {
        let plan = "# Plan\n\nLead.\n\n## Waiting on you\n- [x] answered\n\n## Ideas\nSome text.\n\n## Done\n- [x] shipped\n";
        let offered = plan_sections(plan, d("2026-10-20"), 1);
        let got: Vec<(&str, &str, bool)> = offered
            .iter()
            .map(|c| (c.heading.as_str(), c.class.as_str(), c.waiting))
            .collect();
        assert_eq!(
            got,
            [
                ("Waiting on you", "older", true),
                ("Done", "finished", false)
            ]
        );
        // The lead and the title are never offered.
        assert!(offered.iter().all(|c| c.line > 3));
    }

    #[test]
    fn fenced_rows_and_headings_are_text() {
        let plan = "---\ntitle: x\n## not a heading\n---\n# Plan\n\n## Done\n- [x] a\n```md\n## Also not a heading\n- [ ] fenced row\n```\n\n## Next\n- [ ] b\n";
        let offered = plan_sections(plan, d("2026-10-20"), 1);
        assert_eq!(offered.len(), 1, "{offered:?}");
        assert_eq!((offered[0].heading.as_str(), offered[0].open), ("Done", 0));
        let moved = compact_plan_text(plan, d("2026-10-20"), &[7], ARCH).unwrap();
        assert!(
            moved.archive.contains("- [ ] fenced row\n```"),
            "{}",
            moved.archive
        );
        assert!(moved
            .plan
            .starts_with("---\ntitle: x\n## not a heading\n---\n# Plan\n\n"));
    }

    #[test]
    fn crlf_bom_and_no_final_newline_keep_their_bytes() {
        let plan =
            "\u{feff}# Plan\r\n\r\nLead.\r\n\r\n## Open\r\n- [ ] c\r\n\r\n## Done\r\n- [x] a";
        let offered = plan_sections(plan, d("2026-10-20"), 1);
        let moved = compact_plan_text(plan, d("2026-10-20"), &lines_of(&offered), ARCH).unwrap();
        assert_eq!(
            moved.plan,
            format!("\u{feff}# Plan\r\n\r\nLead.\r\n\r\n{PLAN_POINTER} `{ARCH}` on 2026-10-20.\r\n\r\n## Open\r\n- [ ] c")
        );
        assert_eq!(moved.archive, "## Done\r\n- [x] a");
        // A plan that starts with its first `##`: the pointer goes on top.
        let bare = "## Done\n- [x] a\n\n## Open\n- [ ] b\n";
        let moved = compact_plan_text(bare, d("2026-10-20"), &[1], ARCH).unwrap();
        assert_eq!(
            moved.plan,
            format!("{PLAN_POINTER} `{ARCH}` on 2026-10-20.\n\n## Open\n- [ ] b\n")
        );
    }

    #[test]
    fn a_section_whose_subsection_has_an_open_row_is_not_finished() {
        let plan = "# Plan\n\n## Done\n- [x] a\n- [x] b\n\n### Follow-ups\n- [ ] x\n\n## Nested\n- [x] parent\n  - [ ] child\n";
        assert!(plan_sections(plan, d("2026-10-20"), 1).is_empty());
    }

    #[test]
    fn a_line_quoting_the_pointer_phrase_is_kept() {
        let mine = format!("Note: {POINTER} the old doc, see there.");
        let brief = format!("## Current context\n\n- {mine}\n- 10-01: old\n- 10-09: new\n");
        let plan = compact(
            &brief,
            d("2026-10-10"),
            1,
            1,
            "outputs/workspace-archive/a.md",
        )
        .unwrap();
        assert!(plan.brief.contains(&mine), "{}", plan.brief);
        assert_eq!(plan.brief.matches(POINTER).count(), 2, "{}", plan.brief);
        let lead = format!("# Plan\n\n{PLAN_POINTER} the team on Friday.\n\n## Done\n- [x] a\n");
        let moved = compact_plan_text(&lead, d("2026-10-20"), &[5], ARCH).unwrap();
        assert!(moved
            .plan
            .contains(&format!("{PLAN_POINTER} the team on Friday.")));
        // A generated one is replaced, and names the folder for earlier moves.
        let old = format!("# Plan\n\n{PLAN_POINTER} `outputs/workspace-archive/2026-10-01-plan.md` on 2026-10-01.\n\n## Done\n- [x] a\n");
        let moved = compact_plan_text(&old, d("2026-10-20"), &[5], ARCH).unwrap();
        assert_eq!(
            moved.plan,
            format!("# Plan\n\n{PLAN_POINTER} `{ARCH}` on 2026-10-20; earlier moves are beside it in `outputs/workspace-archive/`.\n")
        );
        assert!(is_pointer_line(
            &format!(
                "- {POINTER} `outputs/workspace-archive/x.md` on 2026-10-10 (through 2026-10-08)."
            ),
            POINTER
        ));
        assert!(!is_pointer_line(
            &format!("{POINTER} `elsewhere/x.md` on 2026-10-10."),
            POINTER
        ));
        assert!(!is_pointer_line(
            &format!("{POINTER} `outputs/workspace-archive/x.md` on 2026-10-10. More."),
            POINTER
        ));
    }

    // Round trips: every non-blank line before is, as a multiset, the
    // non-blank lines after plus the moved text (generated pointer lines
    // aside), and both keep the original order.

    fn kept(text: &str) -> Vec<String> {
        text.lines()
            .map(|l| l.trim_end_matches('\r').to_string())
            .filter(|l| {
                !l.trim().is_empty()
                    && !is_pointer_line(l, POINTER)
                    && !is_pointer_line(l, PLAN_POINTER)
            })
            .collect()
    }

    fn in_order(sub: &[String], of: &[String]) -> bool {
        let mut it = of.iter();
        sub.iter().all(|s| it.any(|o| o == s))
    }

    fn assert_round_trip(before: &str, after: &str, moved: &str) {
        let (b, a, m) = (kept(before), kept(after), kept(moved));
        assert!(!m.is_empty(), "something moved");
        let mut whole = b.clone();
        whole.sort();
        let mut parts: Vec<String> = a.iter().chain(m.iter()).cloned().collect();
        parts.sort();
        assert_eq!(whole, parts, "no line lost or invented");
        assert!(in_order(&a, &b) && in_order(&m, &b), "order kept");
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../tests/fixtures/workspace-sections")
                .join(name),
        )
        .unwrap()
    }

    /// A brief whose defaults move something: 11 entries from 09-20 to
    /// 09-30, then `recent` entries dated 10-09.
    fn long_brief(recent: usize) -> String {
        let mut text = String::from("# W\n\n## Current context\n\nAs of now:\n");
        for day in 20..=30 {
            text.push_str(&format!(
                "- 09-{day} 10:00 ET: entry from the {day}th.\n  continued.\n"
            ));
        }
        for n in 0..recent {
            text.push_str(&format!("- 10-09 0{}:00 ET: recent {n}.\n", n % 10));
        }
        text.push_str("\n## Decisions\n\n- Keep it.\n");
        text
    }

    #[test]
    fn round_trip_loses_no_line() {
        let today = d("2026-10-20");
        let briefs = [
            MAIN.to_string(),
            HOLLOW.to_string(),
            fixture("WORKSPACE.md"),
            long_brief(12),
        ];
        for crlf in [false, true] {
            let conv = |s: &str| {
                if crlf {
                    s.replace('\n', "\r\n")
                } else {
                    s.to_string()
                }
            };
            for brief in &briefs {
                let brief = conv(brief);
                let Some(plan) = compact(&brief, today, 1, 2, "outputs/workspace-archive/x.md")
                else {
                    continue;
                };
                assert_round_trip(&brief, &plan.brief, &plan.archive);
            }
            for plan in [fixture("PLAN.md"), PLAN.to_string()] {
                let plan = conv(&plan);
                let offered = plan_sections(&plan, today, 1);
                assert!(!offered.is_empty());
                let moved = compact_plan_text(&plan, today, &lines_of(&offered), ARCH).unwrap();
                assert_round_trip(&plan, &moved.plan, &moved.archive);
            }
        }
        // Through the apply, on a temp vault.
        for crlf in [false, true] {
            let conv = |s: &str| {
                if crlf {
                    s.replace('\n', "\r\n")
                } else {
                    s.to_string()
                }
            };
            let dir = tempfile::tempdir().unwrap();
            let v = dir.path();
            let (id, ws, brief_path, _) = seeded(v);
            let real = std::fs::read_to_string(&brief_path).unwrap();
            let fm_end = real[3..].find("\n---").unwrap() + 3 + 4;
            let brief = conv(&format!("{}{}", &real[..fm_end], long_brief(12)));
            std::fs::write(&brief_path, &brief).unwrap();
            let plan = conv(&fixture("PLAN.md"));
            std::fs::write(ws.join("PLAN.md"), &plan).unwrap();
            let preview = preview_workspace(v, &id, today).unwrap();
            let done = archive_workspace(v, &id, &args_for(&preview, true)).unwrap();
            let after_brief = std::fs::read_to_string(&brief_path).unwrap();
            let after_plan = std::fs::read_to_string(ws.join("PLAN.md")).unwrap();
            let brief_archive =
                std::fs::read_to_string(v.join(done.brief_archive.as_ref().unwrap())).unwrap();
            let plan_archive =
                std::fs::read_to_string(v.join(done.plan_archive.as_ref().unwrap())).unwrap();
            let moved_brief = brief_archive
                .split_once("\n\n## Moved from")
                .unwrap()
                .1
                .split_once("\n\n")
                .unwrap()
                .1;
            let moved_plan = plan_archive
                .split_once("\n\n## Moved from")
                .unwrap()
                .1
                .split_once("\n\n")
                .unwrap()
                .1;
            assert_round_trip(&brief, &after_brief, moved_brief);
            assert_round_trip(&plan, &after_plan, moved_plan);
            // 11 September entries, and the two 10-09 ones outside the newest ten.
            assert_eq!(done.brief_moved, 13);
            assert_eq!(done.plan_moved as usize, preview.plan.len());
        }
    }

    /// The apply's arguments for everything `preview` offers (`all`) or only
    /// what it ticks by default.
    fn args_for(preview: &ArchivePreview, all: bool) -> ArchiveArgs {
        ArchiveArgs {
            today: preview.today.clone(),
            brief_revision: preview.brief_revision.clone(),
            plan_revision: preview.plan_revision.clone(),
            brief_days: preview.days.iter().map(|d| d.date.clone()).collect(),
            plan_sections: preview
                .plan
                .iter()
                .filter(|c| all || c.class == "finished")
                .map(|c| PlanChoice {
                    line: c.line,
                    sha256: c.heading_sha256.clone(),
                })
                .collect(),
        }
    }

    /// A workspace with [`long_brief`] and [`PLAN`]: (id, folder, brief path).
    fn archived_bed(v: &Path) -> (String, std::path::PathBuf, std::path::PathBuf) {
        let (id, ws, brief_path, _) = seeded(v);
        let real = std::fs::read_to_string(&brief_path).unwrap();
        let fm_end = real[3..].find("\n---").unwrap() + 3 + 4;
        std::fs::write(
            &brief_path,
            format!("{}{}", &real[..fm_end], long_brief(15)),
        )
        .unwrap();
        std::fs::write(ws.join("PLAN.md"), PLAN).unwrap();
        (id, ws, brief_path)
    }

    #[test]
    fn the_preview_offers_days_and_sections_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let brief = std::fs::read_to_string(&brief_path).unwrap();
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        assert_eq!(preview.days.len(), 11);
        assert_eq!(preview.days[0].date, "2026-09-20");
        assert_eq!(
            (preview.days[0].count, preview.days[0].headlines.len()),
            (1, 1)
        );
        assert_eq!(lines_of(&preview.plan), [5]);
        assert!(!preview.busy);
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), brief);
        assert!(!ws.join(ARCHIVE_DIR).exists());
    }

    #[test]
    fn apply_refuses_when_the_brief_changed_since_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        let edited = std::fs::read_to_string(&brief_path).unwrap() + "\nMore.\n";
        std::fs::write(&brief_path, &edited).unwrap();
        let err = archive_workspace(v, &id, &args_for(&preview, true)).unwrap_err();
        assert_eq!(err, CHANGED_SINCE_PREVIEW);
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), edited);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
        assert!(!ws.join(ARCHIVE_DIR).exists());
        // A ticked section that isn't offered is refused the same way.
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        let mut args = args_for(&preview, true);
        args.plan_sections.push(PlanChoice {
            line: 9,
            sha256: crate::vault::revision(b"## Open"),
        });
        assert_eq!(
            archive_workspace(v, &id, &args).unwrap_err(),
            CHANGED_SINCE_PREVIEW
        );
    }

    #[test]
    fn apply_with_another_days_today_moves_only_what_was_shown() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        // On 10-11 the extra 10-09 entries would be old enough too.
        let later = preview_workspace(v, &id, d("2026-10-11")).unwrap();
        assert!(later.days.iter().any(|d| d.date == "2026-10-09"));
        let done = archive_workspace(v, &id, &args_for(&preview, false)).unwrap();
        assert_eq!(done.brief_moved, 11);
        let after = std::fs::read_to_string(&brief_path).unwrap();
        assert_eq!(after.matches("- 10-09 ").count(), 15, "{after}");
        assert!(ws
            .join(format!("{ARCHIVE_DIR}2026-10-10-current-context.md"))
            .exists());
        assert!(ws.join(format!("{ARCHIVE_DIR}2026-10-10-plan.md")).exists());
    }

    #[test]
    fn apply_refuses_while_a_chat_runs_there() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let brief = std::fs::read_to_string(&brief_path).unwrap();
        let folder = crate::workspaces::summary(v, &id).unwrap().path;
        let chat = crate::automated_history::try_begin_scoped(v, &folder, "chat", None).unwrap();
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        assert!(preview.busy);
        let err = archive_workspace(v, &id, &args_for(&preview, true)).unwrap_err();
        assert!(err.contains("conversation is running"), "{err}");
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), brief);
        assert!(!ws.join(ARCHIVE_DIR).exists());
        chat.finish("success").unwrap();
        assert!(archive_workspace(v, &id, &args_for(&preview, true)).is_ok());
    }

    /// A9 review: a full-vault run would lose its own changes in the folder
    /// from its History, so the archive waits for it too.
    #[test]
    fn apply_refuses_during_a_full_vault_run() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, _) = archived_bed(v);
        let run =
            crate::automated_history::try_begin_owned(v, "workflow", Some("nightly")).unwrap();
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        assert!(preview.busy);
        let err = archive_workspace(v, &id, &args_for(&preview, true)).unwrap_err();
        assert!(err.ends_with("Nothing moved."), "{err}");
        assert!(!ws.join(ARCHIVE_DIR).exists());
        run.finish("success").unwrap();
        assert!(archive_workspace(v, &id, &args_for(&preview, true)).is_ok());
    }

    #[test]
    fn a_bom_stays_on_top_and_a_hard_break_keeps_its_spaces() {
        let plan = "\u{feff}## Done\n- [x] a  \n\n## Open\n- [ ] b\n";
        let moved = compact_plan_text(plan, d("2026-10-20"), &[1], ARCH).unwrap();
        assert_eq!(
            moved.plan,
            format!("\u{feff}{PLAN_POINTER} `{ARCH}` on 2026-10-20.\n\n## Open\n- [ ] b\n")
        );
        assert_eq!(moved.archive, "## Done\n- [x] a  ");
        assert_eq!(plan_sections(&moved.plan, d("2026-10-20"), 1), []);
        // A second archive finds that pointer again: one line, BOM on top.
        let plan = "\u{feff}## A\n- [x] a\n\n## B\n- [x] b\n\n## Open\n- [ ] c\n";
        let once = compact_plan_text(plan, d("2026-10-20"), &[1], ARCH).unwrap();
        let twice = compact_plan_text(&once.plan, d("2026-10-20"), &[3], ARCH).unwrap();
        assert_eq!(
            twice.plan,
            format!("\u{feff}{PLAN_POINTER} `{ARCH}` on 2026-10-20.\n\n## Open\n- [ ] c\n")
        );
        // An old pointer on line 0 above lead text: the BOM moves to the lead.
        let old = format!("\u{feff}{PLAN_POINTER} `outputs/workspace-archive/2026-10-01-plan.md` on 2026-10-01.\n\nNotes.\n\n## X\n- [x] a\n");
        let moved = compact_plan_text(&old, d("2026-10-20"), &[5], ARCH).unwrap();
        assert_eq!(
            moved.plan,
            format!("\u{feff}Notes.\n\n{PLAN_POINTER} `{ARCH}` on 2026-10-20; earlier moves are beside it in `outputs/workspace-archive/`.\n")
        );
    }

    #[test]
    fn a_failed_plan_write_puts_its_archive_back_and_keeps_the_brief_half() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let brief = std::fs::read_to_string(&brief_path).unwrap();
        let folder = crate::workspaces::summary(v, &id).unwrap().path;
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        FAIL_WRITE.with(|f| *f.borrow_mut() = Some(format!("{folder}/PLAN.md")));
        let done = archive_workspace(v, &id, &args_for(&preview, true));
        FAIL_WRITE.with(|f| *f.borrow_mut() = None);
        let done = done.unwrap();
        assert!(!done.run_id.is_empty());
        assert_eq!((done.brief_moved, done.plan_moved), (11, 0));
        assert!(
            done.warnings[0].starts_with("Brief archived; plan unchanged: Writing the plan failed"),
            "{:?}",
            done.warnings
        );
        assert_ne!(std::fs::read_to_string(&brief_path).unwrap(), brief);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
        assert!(!ws.join(format!("{ARCHIVE_DIR}2026-10-10-plan.md")).exists());
        assert!(ws
            .join(format!("{ARCHIVE_DIR}2026-10-10-current-context.md"))
            .exists());
    }

    #[test]
    fn restoring_the_archive_run_puts_every_file_back() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let brief = std::fs::read(&brief_path).unwrap();
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        let done = archive_workspace(v, &id, &args_for(&preview, true)).unwrap();
        assert_eq!((done.brief_moved, done.plan_moved), (11, 1));
        crate::automated_history::restore_run(v, &done.run_id, false).unwrap();
        assert_eq!(std::fs::read(&brief_path).unwrap(), brief);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
        assert!(!ws
            .join(format!("{ARCHIVE_DIR}2026-10-10-current-context.md"))
            .exists());
        assert!(!ws.join(format!("{ARCHIVE_DIR}2026-10-10-plan.md")).exists());
    }

    #[test]
    fn restoring_an_appended_archive_restores_its_earlier_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, _) = archived_bed(v);
        let earlier = ws.join(format!("{ARCHIVE_DIR}2026-10-10-plan.md"));
        std::fs::create_dir_all(earlier.parent().unwrap()).unwrap();
        std::fs::write(&earlier, "# Plan archive (Main-ish)\n\nAn earlier move.").unwrap();
        let preview = preview_workspace(v, &id, d("2026-10-10")).unwrap();
        let done = archive_workspace(v, &id, &args_for(&preview, false)).unwrap();
        let text = std::fs::read_to_string(&earlier).unwrap();
        assert!(text.starts_with("# Plan archive (Main-ish)\n\nAn earlier move.\n\n## Moved from PLAN.md on 2026-10-10\n\n## Done (10-01)"), "{text}");
        crate::automated_history::restore_run(v, &done.run_id, false).unwrap();
        assert_eq!(
            std::fs::read_to_string(&earlier).unwrap(),
            "# Plan archive (Main-ish)\n\nAn earlier move."
        );
    }

    /// Alex, 10-19: no action reaches the archive but one. Since V5a that
    /// is `workspaces.archive_shared_docs`, which calls only the shared-docs
    /// entry points; the old picker, the engine's internals and the removed
    /// commands stay out of the actions and the commands.
    #[test]
    fn only_the_shared_docs_entry_points_reach_the_archive() {
        let allowed = [
            "SharedDoc",
            "dry_run_shared_doc",
            "apply_shared_doc_in_run",
            "apply_shared_doc",
            "parse_today",
        ];
        for (file, text) in [
            ("app_actions.rs", include_str!("app_actions.rs")),
            (
                "commands/workspaces_api.rs",
                include_str!("commands/workspaces_api.rs"),
            ),
        ] {
            for (at, _) in text.match_indices("context_compact::") {
                let rest = &text[at + "context_compact::".len()..];
                assert!(
                    allowed.iter().any(|a| rest.starts_with(a)),
                    "{file} reaches into context_compact: {}",
                    &rest[..rest.len().min(40)]
                );
            }
            for name in [
                "archive_workspace",
                "compact_plan_text",
                "compact_days",
                "move_into_archive",
                "archive_older_entries",
                "compact_context",
            ] {
                // The removed action's id may be named once, in the test that
                // proves it is gone.
                let count = text.matches(name).count();
                let ok = name == "compact_context" && file == "app_actions.rs";
                assert!(count == 0 || ok, "{file} names {name}");
            }
        }
    }

    // ── `workspaces.archive_shared_docs` (V5a) ──────────────────────────

    const SHARED_TODAY: &str = "2026-10-10";

    fn shared_dry(v: &Path, id: &str, doc: SharedDoc) -> SharedDocArchive {
        dry_run_shared_doc(v, id, doc, d(SHARED_TODAY), None).unwrap()
    }

    #[test]
    fn shared_doc_dry_run_shows_sizes_and_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let brief = std::fs::read_to_string(&brief_path).unwrap();
        let r = shared_dry(v, &id, SharedDoc::Workspace);
        assert_eq!(
            (r.mode.as_str(), r.doc.as_str()),
            ("dry_run", "WORKSPACE.md")
        );
        assert!(!r.applied && r.lossless);
        assert_eq!(r.hash, crate::vault::revision(brief.as_bytes()));
        assert_eq!(r.bytes_before as usize, brief.len());
        assert!(r.bytes_after < r.bytes_before, "{r:?}");
        assert!(r.archive_bytes > 0);
        assert_eq!(r.moved_count, 11);
        assert!(r.moved[0].contains("09-20"), "{:?}", r.moved);
        assert!(r.archive_path.as_deref().is_some_and(
            |p| p.ends_with("outputs/workspace-archive/2026-10-10-current-context.md")
        ));
        let p = shared_dry(v, &id, SharedDoc::Plan);
        assert_eq!(p.doc, "PLAN.md");
        assert_eq!(p.moved_count, 1, "{p:?}");
        assert!(p.moved[0].contains("Done"), "{:?}", p.moved);
        // A tiny plan can grow: the pointer line is longer than one short section.
        assert_ne!(p.bytes_after, p.bytes_before);
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), brief);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
        assert!(!ws.join(ARCHIVE_DIR).exists());
    }

    #[test]
    fn shared_doc_apply_moves_older_entries_losslessly() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let before = std::fs::read_to_string(&brief_path).unwrap();
        let dry = shared_dry(v, &id, SharedDoc::Workspace);
        let out = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            None,
        )
        .unwrap();
        assert!(out.applied && out.lossless);
        assert_eq!(out.moved_count, 11);
        let after = std::fs::read_to_string(&brief_path).unwrap();
        assert_eq!(out.bytes_after as usize, after.len());
        assert_eq!(out.bytes_after, dry.bytes_after);
        assert_eq!(
            out.new_hash.as_deref(),
            Some(crate::vault::revision(after.as_bytes()).as_str())
        );
        assert!(after.contains(POINTER));
        assert!(!after.contains("entry from the 20th"));
        // The archive note holds the moved text word for word, and the two
        // together are the original.
        let archive = std::fs::read_to_string(v.join(out.archive_path.as_ref().unwrap())).unwrap();
        let moved = archive
            .split_once("\n\n## Moved from")
            .unwrap()
            .1
            .split_once("\n\n")
            .unwrap()
            .1;
        assert!(moved.contains("- 09-20 10:00 ET: entry from the 20th.\n  continued."));
        assert!(is_lossless(&before, &after, moved));
        assert_round_trip(&before, &after, moved);
        // PLAN.md was not touched by a WORKSPACE.md apply.
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
    }

    #[test]
    fn shared_doc_apply_refuses_a_stale_hash_and_moves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let dry = shared_dry(v, &id, SharedDoc::Workspace);
        let edited = std::fs::read_to_string(&brief_path).unwrap() + "\nMore.\n";
        std::fs::write(&brief_path, &edited).unwrap();
        let err = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            None,
        )
        .unwrap_err();
        assert!(err.contains("changed since the dry run"), "{err}");
        let err = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            "deadbeef",
            d(SHARED_TODAY),
            None,
        )
        .unwrap_err();
        assert!(err.contains("nothing moved"), "{err}");
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), edited);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), PLAN);
        assert!(!ws.join(ARCHIVE_DIR).exists());
    }

    #[test]
    fn shared_doc_plan_apply_keeps_open_work_and_a_second_apply_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, _) = archived_bed(v);
        let dry = shared_dry(v, &id, SharedDoc::Plan);
        // From the app: its own History run.
        let out = apply_shared_doc(v, &id, SharedDoc::Plan, &dry.hash, d(SHARED_TODAY)).unwrap();
        assert!(out.applied && out.run_id.is_some(), "{out:?}");
        let after = std::fs::read_to_string(ws.join("PLAN.md")).unwrap();
        assert!(
            after.contains("## Open") && after.contains("- [ ] c"),
            "{after}"
        );
        assert!(!after.contains("Done (10-01)"));
        assert!(after.contains(PLAN_POINTER));
        let archive = std::fs::read_to_string(v.join(out.archive_path.as_ref().unwrap())).unwrap();
        assert!(
            archive.contains("## Done (10-01)\n- [x] a\n- [x] b"),
            "{archive}"
        );
        assert_round_trip(
            PLAN,
            &after,
            archive
                .split_once("\n\n## Moved from")
                .unwrap()
                .1
                .split_once("\n\n")
                .unwrap()
                .1,
        );
        // Idempotent: nothing is left to move, so nothing is written.
        let again_dry = shared_dry(v, &id, SharedDoc::Plan);
        assert_eq!(
            (again_dry.moved_count, again_dry.archive_path.clone()),
            (0, None)
        );
        assert_eq!(again_dry.bytes_before, again_dry.bytes_after);
        let again = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Plan,
            &again_dry.hash,
            d(SHARED_TODAY),
            None,
        )
        .unwrap();
        assert!(!again.applied && again.moved_count == 0);
        assert_eq!(std::fs::read_to_string(ws.join("PLAN.md")).unwrap(), after);
        // The earlier dry run's hash is stale now.
        assert!(
            apply_shared_doc_in_run(v, &id, SharedDoc::Plan, &dry.hash, d(SHARED_TODAY), None)
                .is_err()
        );
    }

    #[test]
    fn shared_doc_apply_moves_what_the_dry_run_showed_after_midnight() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, _ws, brief_path) = archived_bed(v);
        let dry = shared_dry(v, &id, SharedDoc::Workspace);
        // The next day more entries are old enough, and the hash is the same.
        let later =
            dry_run_shared_doc(v, &id, SharedDoc::Workspace, d("2026-10-11"), None).unwrap();
        assert_eq!(later.hash, dry.hash);
        assert!(later.moved_count > dry.moved_count, "{later:?}");
        assert_eq!(later.today, "2026-10-11");
        // An apply given the dry run's own day moves exactly that.
        let day = parse_today_at(Some(&dry.today), d("2026-10-11")).unwrap();
        let out =
            apply_shared_doc_in_run(v, &id, SharedDoc::Workspace, &dry.hash, day, None).unwrap();
        assert_eq!(out.moved_count, dry.moved_count);
        assert_eq!(out.today, dry.today);
        assert!(out
            .archive_path
            .unwrap()
            .contains("2026-10-10-current-context"));
        let after = std::fs::read_to_string(&brief_path).unwrap();
        assert_eq!(after.matches("- 10-09 ").count(), 15, "{after}");
        assert!(parse_today(Some("10/10/2026")).is_err());
        assert!(parse_today(None).is_ok());
        // Only today or yesterday: a far-off day would move almost everything.
        let now = d("2026-10-11");
        assert_eq!(parse_today_at(None, now), Ok(now));
        assert_eq!(parse_today_at(Some("2026-10-11"), now), Ok(now));
        assert_eq!(parse_today_at(Some("2026-10-10"), now), Ok(d("2026-10-10")));
        for bad in ["2099-01-01", "2026-10-09", "2026-10-12"] {
            let err = parse_today_at(Some(bad), now).unwrap_err();
            assert!(err.contains("nothing moved"), "{bad}: {err}");
        }
    }

    #[test]
    fn shared_doc_apply_puts_the_archive_back_when_the_doc_write_fails() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let before = std::fs::read_to_string(&brief_path).unwrap();
        let folder = crate::workspaces::summary(v, &id).unwrap().path;
        let dry = shared_dry(v, &id, SharedDoc::Workspace);
        FAIL_WRITE.with(|f| *f.borrow_mut() = Some(format!("{folder}/WORKSPACE.md")));
        let err = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            None,
        );
        FAIL_WRITE.with(|f| *f.borrow_mut() = None);
        let err = err.unwrap_err();
        assert!(err.contains("nothing moved"), "{err}");
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), before);
        assert!(!ws
            .join(format!("{ARCHIVE_DIR}2026-10-10-current-context.md"))
            .exists());
        // And the same dry run still applies once the failure is gone.
        assert!(apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            None
        )
        .is_ok());
    }

    #[test]
    fn shared_doc_apply_refuses_while_another_run_holds_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let before = std::fs::read_to_string(&brief_path).unwrap();
        let folder = crate::workspaces::summary(v, &id).unwrap().path;
        let chat =
            crate::automated_history::try_begin_scoped_shared(v, &folder, "chat", None).unwrap();
        let other =
            crate::automated_history::try_begin_scoped_shared(v, &folder, "chat", None).unwrap();
        let (mine, theirs) = (chat.id().to_string(), other.id().to_string());
        assert_ne!(mine, theirs);
        let dry =
            dry_run_shared_doc(v, &id, SharedDoc::Workspace, d(SHARED_TODAY), Some(&mine)).unwrap();
        assert!(dry.busy, "another run is live");
        let err = apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            Some(&mine),
        )
        .unwrap_err();
        assert!(err.contains("Another run"), "{err}");
        assert_eq!(std::fs::read_to_string(&brief_path).unwrap(), before);
        assert!(!ws.join(ARCHIVE_DIR).exists());
        // Without the other run, the caller's own does not count.
        other.finish("cancelled").unwrap();
        let dry =
            dry_run_shared_doc(v, &id, SharedDoc::Workspace, d(SHARED_TODAY), Some(&mine)).unwrap();
        assert!(!dry.busy);
        assert!(apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            Some(&mine)
        )
        .is_ok());
        // The app's own apply refuses while a chat runs there.
        let again = shared_dry(v, &id, SharedDoc::Plan);
        let err =
            apply_shared_doc(v, &id, SharedDoc::Plan, &again.hash, d(SHARED_TODAY)).unwrap_err();
        assert!(
            err.contains("Nothing moved") || err.contains("nothing moved"),
            "{err}"
        );
        chat.finish("cancelled").unwrap();
    }

    #[test]
    fn shared_doc_works_on_crlf_files_and_a_bom_plan() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, ws, brief_path) = archived_bed(v);
        let crlf = |s: String| s.replace('\n', "\r\n");
        let brief = crlf(std::fs::read_to_string(&brief_path).unwrap());
        std::fs::write(&brief_path, &brief).unwrap();
        let plan = format!("\u{feff}{}", crlf(PLAN.to_string()));
        std::fs::write(ws.join("PLAN.md"), &plan).unwrap();
        for (doc, text) in [(SharedDoc::Workspace, &brief), (SharedDoc::Plan, &plan)] {
            let dry = shared_dry(v, &id, doc);
            assert!(dry.lossless && dry.moved_count > 0, "{doc:?} {dry:?}");
            let out =
                apply_shared_doc_in_run(v, &id, doc, &dry.hash, d(SHARED_TODAY), None).unwrap();
            assert!(out.applied, "{out:?}");
            let after = std::fs::read_to_string(ws.join(doc.file())).unwrap();
            assert!(
                after.contains("\r\n") && !after.replace("\r\n", "").contains('\n'),
                "line endings kept"
            );
            let note = std::fs::read_to_string(v.join(out.archive_path.unwrap())).unwrap();
            let moved = note
                .split_once("\n\n## Moved from")
                .unwrap()
                .1
                .split_once("\n\n")
                .unwrap()
                .1;
            assert!(is_lossless(text, &after, moved));
        }
        assert!(std::fs::read_to_string(ws.join("PLAN.md"))
            .unwrap()
            .starts_with('\u{feff}'));
        assert!(is_lossless("\u{feff}a\nb\n", "a\n", "b\n"));
    }

    #[test]
    fn shared_doc_leaves_entries_under_a_sub_heading_alone() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let (id, _ws, brief_path) = archived_bed(v);
        let text = std::fs::read_to_string(&brief_path).unwrap().replace(
            "\n## Decisions",
            "\n### Older notes\n\n- 09-01 10:00 ET: kept under a sub-heading.\n\n## Decisions",
        );
        std::fs::write(&brief_path, &text).unwrap();
        let dry = shared_dry(v, &id, SharedDoc::Workspace);
        assert_eq!(dry.moved_count, 11, "{:?}", dry.moved);
        assert!(dry.moved.iter().all(|m| !m.contains("sub-heading")));
        apply_shared_doc_in_run(
            v,
            &id,
            SharedDoc::Workspace,
            &dry.hash,
            d(SHARED_TODAY),
            None,
        )
        .unwrap();
        assert!(std::fs::read_to_string(&brief_path)
            .unwrap()
            .contains("kept under a sub-heading."));
    }

    #[test]
    fn shared_doc_names_are_checked_and_lossless_catches_a_lost_line() {
        assert_eq!(SharedDoc::parse("PLAN.md"), Ok(SharedDoc::Plan));
        assert_eq!(SharedDoc::parse("WORKSPACE.md"), Ok(SharedDoc::Workspace));
        assert!(SharedDoc::parse("SOURCES.md").is_err());
        assert!(is_lossless("a\nb\n", "a\n", "b\n"));
        assert!(!is_lossless("a\nb\n", "a\n", ""), "a lost line");
        assert!(!is_lossless("a\nb\n", "a\nx\n", "b\n"), "an invented line");
        assert!(!is_lossless("a\n", "a\n", "a\n"), "a duplicated line");
    }

    /// Dry numbers for the PR, from copies only: `HOLLOW_ARCHIVE_DRY=<dir>`
    /// holding `<ws>/WORKSPACE.md` and `<ws>/PLAN.md` copies.
    #[test]
    #[ignore = "a dry-run report, not a test: HOLLOW_ARCHIVE_DRY=<folder of copied briefs and plans> prints what Archive older entries would offer"]
    fn dry_numbers_from_copies() {
        let Some(root) = std::env::var_os("HOLLOW_ARCHIVE_DRY") else {
            return;
        };
        let today = chrono::Local::now().date_naive();
        for entry in std::fs::read_dir(root).unwrap().flatten() {
            let read = |f: &str| std::fs::read_to_string(entry.path().join(f)).unwrap_or_default();
            let (brief, plan) = (read("WORKSPACE.md"), read("PLAN.md"));
            let days = brief_days(&brief, today);
            let sections = plan_sections(&plan, today, DEFAULT_KEEP_DAYS);
            let entries: u32 = days.iter().map(|d| d.count).sum();
            let finished: Vec<u32> = sections
                .iter()
                .filter(|c| c.class == "finished")
                .map(|c| c.line)
                .collect();
            let moved = compact_days(
                &brief,
                today,
                DEFAULT_KEEP_DAYS,
                DEFAULT_KEEP_NEWEST,
                "outputs/workspace-archive/x.md",
                None,
            );
            let plan_moved = if finished.is_empty() {
                None
            } else {
                compact_plan_text(
                    &plan,
                    today,
                    &finished,
                    "outputs/workspace-archive/x-plan.md",
                )
            };
            println!(
                "DRY {:?}: WORKSPACE.md {} B -> {} B ({} days / {} entries move, lossless {}); PLAN.md {} B -> {} B ({} of {} sections move, lossless {})",
                entry.file_name(),
                brief.len(),
                moved.as_ref().map_or(brief.len(), |m| m.brief.len()),
                days.len(),
                entries,
                moved
                    .as_ref()
                    .is_none_or(|m| is_lossless(&brief, &m.brief, &m.archive)),
                plan.len(),
                plan_moved.as_ref().map_or(plan.len(), |m| m.plan.len()),
                finished.len(),
                sections.len(),
                plan_moved
                    .as_ref()
                    .is_none_or(|m| is_lossless(&plan, &m.plan, &m.archive)),
            );
        }
    }
}
