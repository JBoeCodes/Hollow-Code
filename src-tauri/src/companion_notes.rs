//! Companion op kinds beyond tasks (overnight 10-12/13, lane H, H5): notes
//! and append-only workspace lines, applied with the same ledger, lease and
//! receipts as the task kinds (a child module of [`super`], the inbox).
//! Contract: `docs/COMPANION_PROTOCOL.md` → "Kinds (overnight 10-12/13)".
//!
//! - `note.create` `{title, body?, folder?}`: a new file under `notes/`
//!   (default) or `library/notes/`; never overwrites (a taken name gets
//!   ` 2`, ` 3`…).
//! - `note.edit` `{ref, baseHash, body}`: replaces the body, keeps the
//!   front matter. When the file changed since the device read it (its hash
//!   isn't `baseHash`), nothing is overwritten: the device's version is saved
//!   beside it as `<name> (from <device> <date>).md` and the receipt says
//!   `conflict`.
//! - `note.append` `{ref, text}`: adds a paragraph at the end.
//! - `workspace.append` `{workspace, file: "plan" | "brief", text}`: one
//!   line: an open item under `## From your devices` in PLAN.md, or a dated
//!   line at the end of the brief's `## Current context`.
//!
//! The phone API's quick note (`POST /v1/notes`, `docs/phone-api/notes.md`)
//! makes the same file as `note.create` through [`create_for_phone`], which
//! shares [`note_text`], [`created_before`], [`write_new`] and the lease with
//! it but has no ledger: no scope file, and the phone API's own in-memory
//! day count in place of [`within_daily_files`].
//!
//! Every target must be a path the backend mirror may hold in the person's
//! scope ([`companion_docs::classify`] + `scope.json`), with no symlink
//! anywhere on it. Anything else is refused.
//!
//! **Busy vs refused.** Only a held lease and transient I/O are `Busy` (the
//! op waits, and in the backend road everything after it waits too); a
//! problem that can't clear by itself (not UTF-8, a folder, a locked file, a
//! bad path) is refused, so one bad op never stalls the queue.
//!
//! **Once.** The ledger records each op after it applies; if Hollow stops in
//! between, the op comes again. Each kind can tell it already happened: a
//! created note carries `companion-op: <opId>` in its front matter, appended
//! lines end with an invisible `<!-- op:<opId> -->`, and an edit whose result
//! is already on disk counts as applied.

use super::{reject_unknown, required_str, Device, Outcome, Receipt, Step, Store};
use crate::companion_docs;
use crate::task_broker::CompanionRefusal;
use chrono::{DateTime, Local};
use serde_json::{Map, Value};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const TITLE_MAX: usize = 200;
/// A note body in UTF-8 bytes: the whole op must fit the 64 KiB envelope,
/// and JSON escapes the body twice.
const BODY_MAX: usize = 48 * 1024;
const APPEND_MAX: usize = 10_000;
const LINE_MAX: usize = 2_000;
/// A file name stays well under APFS's 255 bytes, with room for the
/// atomic-write temp name around it.
const STEM_MAX_BYTES: usize = 100;
/// New files (notes and conflict copies) one device may make in 24 hours: a
/// stolen or broken key can't flood the vault (and everything it syncs to).
const FILES_PER_DAY: i64 = 200;
/// Where a phone's quick note goes: `note.create`'s default folder.
const PHONE_FOLDER: &str = "notes";
const PLAN_SECTION: &str = "## From your devices";
const BRIEF_SECTION: &str = "## Current context";

fn refused(m: impl Into<String>) -> Step {
    Step::Refused(m.into())
}

/// An I/O error that won't clear by itself is a refusal; anything else
/// (a full disk, a sync client holding the file) may, so the op waits.
fn io_step(rel: &str, what: &str, e: &std::io::Error) -> Step {
    match e.kind() {
        ErrorKind::NotFound => refused(format!("`{rel}` doesn't exist")),
        ErrorKind::InvalidData => refused(format!("`{rel}` isn't UTF-8 text; edit it on the Mac")),
        ErrorKind::IsADirectory | ErrorKind::NotADirectory | ErrorKind::InvalidFilename => {
            refused(format!("`{rel}` isn't a note file"))
        }
        ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
            refused(format!("`{rel}` is locked or read-only on the Mac"))
        }
        _ => Step::Busy(format!("{what} {rel}: {e}")),
    }
}

/// The same split for the vault's string errors (`mutate_note`,
/// `write_note_expecting`).
fn msg_step(rel: &str, e: String) -> Step {
    let lower = e.to_lowercase();
    let permanent = [
        "not found",
        "no such file",
        "not utf-8",
        "invalid utf-8",
        "stream did not contain valid utf-8",
        "is a directory",
        "not a directory",
        "permission denied",
        "operation not permitted",
        "read-only",
        "file name too long",
        "operation not supported",
        "path escape",
        "absolute path",
    ];
    if permanent.iter().any(|p| lower.contains(p)) {
        refused(format!("`{rel}` can't be changed: {e}"))
    } else {
        Step::Busy(e)
    }
}

/// The vault path of `rel` with no symlink anywhere on it (a link inside the
/// vault, `notes/x.md` → `me/Identity.md`, would pass `safe_join`), every
/// existing parent a folder, and its letter case exactly as on disk.
fn exact(vault: &Path, rel: &str) -> Result<PathBuf, Step> {
    let canon = vault
        .canonicalize()
        .map_err(|e| Step::Busy(format!("the vault folder: {e}")))?;
    crate::vault::safe_join(vault, rel).map_err(Step::Refused)?;
    let want = canon.join(rel);
    let real = match std::fs::symlink_metadata(&want) {
        Ok(_) => want.canonicalize().ok(),
        // A new file (maybe in new folders): the deepest folder that exists
        // must be real, and a folder.
        Err(_) => {
            let mut existing = want.as_path();
            let mut rest: Vec<&std::ffi::OsStr> = Vec::new();
            while std::fs::symlink_metadata(existing).is_err() {
                rest.push(existing.file_name().unwrap_or_default());
                existing = existing.parent().unwrap_or(&canon);
            }
            if !std::fs::metadata(existing).is_ok_and(|m| m.is_dir()) {
                return Err(refused(format!("`{rel}` isn't a note file")));
            }
            existing.canonicalize().ok().map(|mut p| {
                for part in rest.iter().rev() {
                    p.push(part);
                }
                p
            })
        }
    };
    if real.as_deref() != Some(want.as_path()) {
        return Err(refused(format!(
            "`{rel}` isn't a plain file at that path (a symlink, or a name that differs only in case); companion devices can't change it"
        )));
    }
    Ok(want)
}

/// A target the person's scope lets devices change: the backend mirror's
/// rules for one of `kinds`, work data only with work data on, and a
/// workspace only when it is on the personal list.
fn in_scope(vault: &Path, store: &Store, rel: &str, kinds: &[&str]) -> Result<(), Step> {
    let (kind, workspace, work_only) = companion_docs::classify(rel).ok_or_else(|| {
        refused(format!(
            "`{rel}` isn't something companion devices may change"
        ))
    })?;
    if !kinds.contains(&kind) {
        return Err(refused(format!(
            "`{rel}` is a {kind}, not a {}",
            kinds.join(" or ")
        )));
    }
    let (scope, error) = companion_docs::load_scope_checked(store, vault);
    if let Some(error) = error {
        return Err(refused(error));
    }
    if work_only && !scope.include_work_data {
        return Err(refused(format!(
            "`{rel}` is work data, which this Hollow keeps off its devices"
        )));
    }
    if let Some(ws) = workspace {
        let shared = (scope.include_work_data || scope.personal_workspaces.contains(&ws))
            && companion_docs::workspaces(vault, &scope)
                .into_iter()
                .any(|w| w.id == ws && (w.personal || scope.include_work_data));
        if !shared {
            return Err(refused(format!(
                "workspace `{ws}` isn't one this Hollow shares with its devices"
            )));
        }
    }
    Ok(())
}

fn lease(vault: &Path) -> Result<crate::automated_history::BackgroundWriteLease, Step> {
    crate::automated_history::companion_write_lease(vault).map_err(|e| match e {
        crate::automated_history::BeginRunError::Busy(holder) => Step::Busy(holder.busy_message()),
        crate::automated_history::BeginRunError::Failed(error) => Step::Busy(error),
    })
}

fn done(
    op_id: &str,
    device: &Device,
    now: DateTime<Local>,
    status: &str,
    rel: &str,
    content: &str,
    step: &str,
) -> Outcome {
    let mut receipt = Receipt::new(op_id, &device.device_id, now, status);
    receipt.reference = Some(format!("vault:{rel}"));
    receipt.content_hash = Some(crate::vault::revision(content.as_bytes()));
    receipt.applied = vec![step.into()];
    Outcome::Done {
        receipt,
        review_changed: 0,
    }
}

/// Cut `s` to at most `max` UTF-8 bytes on a character boundary.
fn cut_bytes(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Names an agent loads as instructions when it reads under that folder
/// (`CLAUDE.md`, `AGENTS.md` and their local or override variants, `GEMINI.md`).
/// Matched in lower case, as APFS compares names.
const INSTRUCTION_STEMS: [&str; 5] = [
    "claude",
    "claude.local",
    "agents",
    "agents.override",
    "gemini",
];

/// Whether `stem` is an [`INSTRUCTION_STEMS`] name as APFS compares names:
/// by Unicode case folding, so the long s (U+017F) is the same letter as `s`
/// and `AGENTſ.md` is `AGENTS.md`. Going up to upper case and back down to
/// lower case folds those (and the Kelvin sign) onto the ASCII letters; it
/// also catches a few look-alikes no disk would fold, which only costs them a
/// suffix.
fn names_an_instruction_file(stem: &str) -> bool {
    let folded: String = stem
        .chars()
        .flat_map(char::to_uppercase)
        .flat_map(char::to_lowercase)
        .collect();
    INSTRUCTION_STEMS.contains(&folded.as_str())
}

/// A file name from a title: letters, digits, spaces and `-_.,()&'` kept,
/// the rest dropped; at most [`STEM_MAX_BYTES`]; "note" when nothing is left.
/// A name an agent would load as instructions ([`INSTRUCTION_STEMS`]) gets
/// " note" after it: a note called "Claude" is a note, never persistent
/// instructions for the next agent that reads the folder (from a phone, a
/// first line is all it takes).
fn file_stem(title: &str) -> String {
    let kept: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || " -_.,()&'".contains(*c))
        .collect();
    let stem: String = kept.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem =
        cut_bytes(stem.trim_matches(['.', ' ']), STEM_MAX_BYTES).trim_end_matches(['.', ' ']);
    if stem.is_empty() {
        "note".into()
    } else if names_an_instruction_file(stem) {
        format!("{stem} note")
    } else {
        stem.to_string()
    }
}

/// One line: every run of whitespace (CR, LS, NEL included) becomes a space.
fn one_line(text: &str) -> String {
    text.split(char::is_whitespace)
        .map(|w| w.chars().filter(|c| !c.is_control()).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// (BOM, front matter block with its fences, body) of `content`.
fn split(content: &str) -> (&str, &str, &str) {
    let (bom, text) = match content.strip_prefix('\u{feff}') {
        Some(rest) => ("\u{feff}", rest),
        None => ("", content),
    };
    for start in ["---\n", "---\r\n"] {
        if let Some(rest) = text.strip_prefix(start) {
            let mut offset = start.len();
            for line in rest.split_inclusive('\n') {
                offset += line.len();
                if line.trim_end_matches(['\r', '\n']) == "---" {
                    return (bom, &text[..offset], &text[offset..]);
                }
            }
        }
    }
    (bom, "", text)
}

/// The note as it becomes with `body`, which is exactly what the read API
/// serves as `body` (the text after the front matter's closing line, its
/// blank line included): the BOM and front matter kept, the body verbatim,
/// so an unchanged body round-trips byte for byte. A body that would read as
/// front matter on a note without any gets a blank line first.
fn with_body(current: &str, body: &str) -> String {
    let (bom, front, _) = split(current);
    let sep = if front.is_empty() && body.starts_with("---") {
        "\n"
    } else {
        ""
    };
    format!("{bom}{front}{sep}{body}")
}

fn op_marker(op_id: &str) -> String {
    format!("<!-- op:{op_id} -->")
}

/// A created note's own record of the op that made it.
fn creates_op(text: &str, op_id: &str) -> bool {
    let (_, front, _) = split(text);
    front
        .lines()
        .any(|l| l.trim() == format!("companion-op: {op_id}"))
}

/// A note this op already created (a repeat after a lost ledger write): it
/// can only be `<stem>.md` or `<stem> 2..50.md` in `dir`, and says so in its
/// front matter.
fn created_before(vault: &Path, dir: &str, stem: &str, op_id: &str) -> Option<(String, String)> {
    (1..=50).find_map(|n| {
        let name = if n == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} {n}.md")
        };
        let rel = format!("{dir}/{name}");
        let full = vault.join(&rel);
        if !std::fs::symlink_metadata(&full).is_ok_and(|m| m.is_file()) {
            return None;
        }
        let text = std::fs::read_to_string(&full).ok()?;
        creates_op(&text, op_id).then_some((rel, text))
    })
}

/// Refuse when this device already made [`FILES_PER_DAY`] files (tasks,
/// notes, conflict copies) in the last 24 hours.
pub(super) fn within_daily_files(
    store: &Store,
    device: &Device,
    now: DateTime<Local>,
) -> Result<(), Step> {
    let since = (now.with_timezone(&chrono::Utc) - chrono::Duration::hours(24))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let made = store
        .files_made_since(&device.device_id, &since)
        .map_err(Step::Busy)?;
    if made >= FILES_PER_DAY {
        return Err(refused(format!(
            "{} made {made} new tasks, notes or copies in the last 24 hours, the most a device may; try again later",
            one_line(&device.name)
        )));
    }
    Ok(())
}

/// Write `content` to a new file named after `stem` in `dir`, with ` 2`,
/// ` 3`… when taken (in any letter case). Returns the path written.
fn write_new(vault: &Path, dir: &str, stem: &str, content: &str) -> Result<String, Step> {
    for n in 1..=50 {
        let name = if n == 1 {
            format!("{stem}.md")
        } else {
            format!("{stem} {n}.md")
        };
        let rel = format!("{dir}/{name}");
        if std::fs::symlink_metadata(vault.join(&rel)).is_ok() {
            continue;
        }
        exact(vault, &rel)?;
        match crate::vault::write_note_expecting(
            vault,
            &rel,
            content,
            crate::vault::Expected::Absent,
        )
        .map_err(|e| msg_step(&rel, e))?
        {
            crate::vault::CasOutcome::Written(_) => return Ok(rel),
            crate::vault::CasOutcome::Conflict(_) => continue,
        }
    }
    Err(refused(format!("too many notes named `{stem}` already")))
}

/// Read a target as text, telling a permanent problem from a passing one.
fn read_text(full: &Path, rel: &str) -> Result<String, Step> {
    std::fs::read_to_string(full).map_err(|e| io_step(rel, "read", &e))
}

fn optional_str<'a>(payload: &'a Map<String, Value>, key: &str) -> Result<Option<&'a str>, Step> {
    match payload.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s)),
        Some(_) => Err(refused(format!("`{key}` must be text"))),
    }
}

pub(super) fn note_create(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    reject_unknown(payload, &["title", "body", "folder"]).map_err(Step::Refused)?;
    let title = one_line(required_str(payload, "title").map_err(Step::Refused)?);
    if title.chars().count() > TITLE_MAX {
        return Err(refused(format!(
            "a title is at most {TITLE_MAX} characters"
        )));
    }
    let body = optional_str(payload, "body")?.unwrap_or("");
    if body.len() > BODY_MAX {
        return Err(refused("a note body is at most 48 KiB"));
    }
    let folder = match optional_str(payload, "folder")?.unwrap_or("notes") {
        f @ ("notes" | "library/notes") => f,
        other => {
            return Err(refused(format!(
                "notes go in `notes` or `library/notes`, not `{other}`"
            )))
        }
    };
    in_scope(
        vault,
        store,
        &format!("{folder}/{}.md", file_stem(&title)),
        &["note"],
    )?;
    let content = note_text(&title, body, &device.name, op_id, now);
    let _lease = lease(vault)?;
    if let Some((rel, text)) = created_before(vault, folder, &file_stem(&title), op_id) {
        return Ok(done(op_id, device, now, "applied", &rel, &text, "create"));
    }
    within_daily_files(store, device, now)?;
    let rel = write_new(vault, folder, &file_stem(&title), &content)?;
    Ok(done(
        op_id, device, now, "applied", &rel, &content, "create",
    ))
}

/// When a note was made, as its front matter keeps it: RFC 3339, seconds.
fn created_stamp(now: DateTime<Local>) -> String {
    now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// A new note's file: front matter (the title, when it was made, which device
/// made it and the op that did, so a repeat finds it), a blank line, then the
/// body without its trailing line breaks. What `note.create` and the phone
/// API's quick note both write.
fn note_text(
    title: &str,
    body: &str,
    device_name: &str,
    op_id: &str,
    now: DateTime<Local>,
) -> String {
    let mut lines = vec!["---".to_string()];
    crate::frontmatter::push_kv(&mut lines, "title", title);
    crate::frontmatter::push_kv(&mut lines, "created", &created_stamp(now));
    crate::frontmatter::push_kv(
        &mut lines,
        "source",
        &format!("companion: {}", one_line(device_name)),
    );
    crate::frontmatter::push_kv(&mut lines, "companion-op", op_id);
    lines.push("---".into());
    format!("{}\n\n{}\n", lines.join("\n"), body.trim_end_matches('\n'))
}

/// A quick note the phone API made, or found it had made already.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PhoneNote {
    /// Vault-relative, under `notes/`.
    pub(crate) path: String,
    /// The title as the file keeps it (one line).
    pub(crate) title: String,
    /// The front matter's `created`, RFC 3339.
    pub(crate) created: String,
    /// The op had made this file before: nothing was written.
    pub(crate) duplicate: bool,
}

/// The iPhone app's quick note (`POST /v1/notes`): the file [`note_create`]
/// makes for a `note.create` op with the default folder, written the same
/// way (front matter, file name, a taken name gets ` 2`, `write_new` never
/// overwrites, the short companion write lease, no link on the path), but not
/// through the companion ledger, which the phone API doesn't have: there is
/// no scope file to read, and the day's limit is the caller's own, asked in
/// `before_write`.
///
/// `before_write` runs once, only for a note that is about to be written: a
/// repeat of an `op_id` that already made its file is found first and
/// answered as it is, so a repeat never counts against the day. It refuses
/// with its reason, which is a [`CompanionRefusal::Refused`] here.
///
/// `body` is kept as it comes (the caller trims it); the title is the title
/// as one line. A held lease is [`CompanionRefusal::Busy`], and so is a
/// passing I/O error; a problem that can't clear by itself is refused.
pub(crate) fn create_for_phone(
    vault: &Path,
    device_name: &str,
    op_id: &str,
    title: &str,
    body: &str,
    now: DateTime<Local>,
    before_write: impl FnOnce() -> Result<(), String>,
) -> Result<PhoneNote, CompanionRefusal> {
    // The limits `note.create` has, so no file is made that it would refuse.
    let title = one_line(title);
    if title.is_empty() {
        return Err(CompanionRefusal::Refused("A note needs a title.".into()));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(CompanionRefusal::Refused(format!(
            "A title is at most {TITLE_MAX} characters."
        )));
    }
    if body.len() > BODY_MAX {
        return Err(CompanionRefusal::Refused(
            "A note is at most 48 KiB.".into(),
        ));
    }
    let stem = file_stem(&title);
    let content = note_text(&title, body, device_name, op_id, now);
    let _lease = lease(vault).map_err(refusal_of)?;
    if let Some((path, text)) = created_before(vault, PHONE_FOLDER, &stem, op_id) {
        // The file as it is now. A line the person took out of it on the Mac
        // falls back to what this request says: the answer is never an error.
        let front = crate::frontmatter::parse(&text);
        let kept = |key: &str| {
            front
                .get(key)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        return Ok(PhoneNote {
            path,
            title: kept("title").unwrap_or(title),
            created: kept("created").unwrap_or_else(|| created_stamp(now)),
            duplicate: true,
        });
    }
    before_write().map_err(CompanionRefusal::Refused)?;
    let path = write_new(vault, PHONE_FOLDER, &stem, &content).map_err(refusal_of)?;
    Ok(PhoneNote {
        path,
        title,
        created: created_stamp(now),
        duplicate: false,
    })
}

/// A step's error as the companion writers' refusal the phone API maps.
fn refusal_of(step: Step) -> CompanionRefusal {
    match step {
        Step::Busy(reason) => CompanionRefusal::Busy(reason),
        Step::Refused(reason) => CompanionRefusal::Refused(reason),
    }
}

pub(super) fn note_edit(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    reject_unknown(payload, &["ref", "baseHash", "body"]).map_err(Step::Refused)?;
    let rel = super::rel_of(required_str(payload, "ref").map_err(Step::Refused)?);
    let base = required_str(payload, "baseHash").map_err(Step::Refused)?;
    let body = optional_str(payload, "body")?.ok_or_else(|| refused("`body` is required"))?;
    if body.len() > BODY_MAX {
        return Err(refused("a note body is at most 48 KiB"));
    }
    in_scope(vault, store, &rel, &["note"])?;
    let full = exact(vault, &rel)?;
    let _lease = lease(vault)?;
    let current = read_text(&full, &rel)?;
    let next = with_body(&current, body);
    if current == next {
        // Already so (a repeat whose ledger write was lost, or no change).
        return Ok(done(op_id, device, now, "applied", &rel, &current, "edit"));
    }
    if crate::vault::revision(current.as_bytes()) == base {
        match crate::vault::write_note_expecting(
            vault,
            &rel,
            &next,
            crate::vault::Expected::Content(&current),
        )
        .map_err(|e| msg_step(&rel, e))?
        {
            crate::vault::CasOutcome::Written(_) => {
                return Ok(done(op_id, device, now, "applied", &rel, &next, "edit"))
            }
            crate::vault::CasOutcome::Conflict(_) => {}
        }
    }
    // Changed on the Mac since the device read it: keep both. A repeat finds
    // its copy by the marker instead of making another.
    let dir = rel.rsplit_once('/').map(|(d, _)| d).unwrap_or("notes");
    let marker = op_marker(op_id);
    let next_without_op: String = next
        .split_inclusive('\n')
        .filter(|l| !l.trim_start().starts_with("companion-op:"))
        .collect();
    let copy_text = format!("{}\n\n{marker}\n", next_without_op.trim_end_matches('\n'));
    let stem = rel
        .rsplit('/')
        .next()
        .unwrap_or("note")
        .trim_end_matches(".md");
    let copy_prefix = format!("{} (from ", cut_bytes(&file_stem(stem), 20).trim_end());
    let existing = std::fs::read_dir(vault.join(dir)).ok().and_then(|entries| {
        entries.flatten().find_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            (name.ends_with(".md") && name.starts_with(&copy_prefix))
                .then(|| std::fs::read_to_string(e.path()).ok())
                .flatten()
                .filter(|t| t.contains(&marker))
                .map(|t| (format!("{dir}/{name}"), t))
        })
    });
    let (copy, written) = match existing {
        Some(found) => found,
        None => {
            within_daily_files(store, device, now)?;
            let from = format!(
                "(from {} {})",
                one_line(&device.name),
                now.format("%Y-%m-%d %H%M")
            );
            let suffix = file_stem(&format!(" {from}"));
            let room = STEM_MAX_BYTES.saturating_sub(suffix.len() + 1).max(20);
            let copy_stem = format!("{} {suffix}", cut_bytes(&file_stem(stem), room).trim_end());
            // Its own title too, so two notes don't share one (H5 review). A
            // block-scalar title (`title: |`) stays as it is: rewriting it
            // would lose its text.
            let copy_text = match crate::frontmatter::parse(&copy_text).get("title") {
                Some(title)
                    if !title.trim().is_empty()
                        && !crate::frontmatter::is_block_scalar(&copy_text, "title") =>
                {
                    crate::frontmatter::patch(
                        &copy_text,
                        &[(
                            "title",
                            crate::frontmatter::KeyEdit::Set(format!("{} {from}", title.trim())),
                        )],
                        None,
                    )
                    .unwrap_or(copy_text)
                }
                _ => copy_text,
            };
            (write_new(vault, dir, &copy_stem, &copy_text)?, copy_text)
        }
    };
    let mut outcome = done(
        op_id,
        device,
        now,
        "conflict",
        &copy,
        &written,
        "conflict-copy",
    );
    if let Outcome::Done { receipt, .. } = &mut outcome {
        receipt.reason = Some(format!(
            "`{rel}` changed on the Mac since you opened it; your version is saved as `{copy}`"
        ));
    }
    Ok(outcome)
}

pub(super) fn note_append(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    reject_unknown(payload, &["ref", "text"]).map_err(Step::Refused)?;
    let rel = super::rel_of(required_str(payload, "ref").map_err(Step::Refused)?);
    let text = required_str(payload, "text").map_err(Step::Refused)?;
    if text.chars().count() > APPEND_MAX {
        return Err(refused(format!(
            "an addition is at most {APPEND_MAX} characters"
        )));
    }
    in_scope(vault, store, &rel, &["note"])?;
    let full = exact(vault, &rel)?;
    let _lease = lease(vault)?;
    let marker = op_marker(op_id);
    let current = read_text(&full, &rel)?;
    if current.contains(&marker) {
        return Ok(done(
            op_id, device, now, "applied", &rel, &current, "append",
        ));
    }
    let note = crate::vault::mutate_note(vault, &rel, |cur| {
        let nl = if cur.contains("\r\n") { "\r\n" } else { "\n" };
        let paragraph = text
            .replace("\r\n", "\n")
            .trim_end_matches('\n')
            .replace('\n', nl);
        let before = cur.trim_end_matches(['\r', '\n']);
        let gap = if before.is_empty() {
            String::new()
        } else {
            format!("{nl}{nl}")
        };
        // The marker on its own line: a hidden HTML block that can't break a
        // closing code fence the paragraph ends with.
        Ok(format!("{before}{gap}{paragraph}{nl}{nl}{marker}{nl}"))
    })
    .map_err(|e| msg_step(&rel, e))?;
    Ok(done(
        op_id,
        device,
        now,
        "applied",
        &rel,
        &note.content,
        "append",
    ))
}

/// Insert `line` at the end of the section headed `heading` (matched
/// ignoring case, outside fenced code), before the next `## `; the section
/// is added at the end when it's missing. Keeps CRLF files CRLF.
fn add_to_section(content: &str, heading: &str, line: &str) -> String {
    let nl = if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let lines: Vec<&str> = content.lines().collect();
    let mut fenced = false;
    let mut start = None;
    let mut end = lines.len();
    for (i, l) in lines.iter().enumerate() {
        if l.trim_start().starts_with("```") || l.trim_start().starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        match start {
            None if l.trim_end().eq_ignore_ascii_case(heading) => start = Some(i),
            Some(_) if l.starts_with("## ") || l.starts_with("# ") => {
                end = i;
                break;
            }
            _ => {}
        }
    }
    let Some(start) = start else {
        let body = content.trim_end_matches(['\r', '\n']);
        return if body.is_empty() {
            format!("{heading}{nl}{nl}{line}{nl}")
        } else {
            format!("{body}{nl}{nl}{heading}{nl}{nl}{line}{nl}")
        };
    };
    // After the section's last non-blank line.
    let mut at = end;
    while at > start + 1 && lines[at - 1].trim().is_empty() {
        at -= 1;
    }
    let mut out: Vec<&str> = lines[..at].to_vec();
    if at == start + 1 {
        out.push("");
    }
    out.push(line);
    if end < lines.len() {
        out.push("");
        out.extend(lines[end..].iter().copied());
    }
    let mut text = out.join(nl);
    text.push_str(nl);
    text
}

pub(super) fn workspace_append(
    vault: &Path,
    store: &Store,
    device: &Device,
    op_id: &str,
    payload: &Map<String, Value>,
    now: DateTime<Local>,
) -> Result<Outcome, Step> {
    reject_unknown(payload, &["workspace", "file", "text"]).map_err(Step::Refused)?;
    let workspace = required_str(payload, "workspace").map_err(Step::Refused)?;
    let text = one_line(required_str(payload, "text").map_err(Step::Refused)?);
    if text.chars().count() > LINE_MAX {
        return Err(refused(format!("a line is at most {LINE_MAX} characters")));
    }
    let who = one_line(&device.name);
    let marker = op_marker(op_id);
    let (file, heading, line) = match required_str(payload, "file").map_err(Step::Refused)? {
        "plan" => (
            "PLAN.md",
            PLAN_SECTION,
            format!("- [ ] {text} ({who}, {}) {marker}", now.format("%Y-%m-%d")),
        ),
        "brief" => (
            "WORKSPACE.md",
            BRIEF_SECTION,
            format!(
                "- {} · {who}: {text} {marker}",
                now.format("%Y-%m-%d %H:%M")
            ),
        ),
        other => return Err(refused(format!("`file` is plan or brief, not `{other}`"))),
    };
    let rel = format!("workspaces/{workspace}/{file}");
    in_scope(vault, store, &rel, &["workspace"])?;
    let full = exact(vault, &rel)?;
    let _lease = lease(vault)?;
    let current = read_text(&full, &rel)?;
    if current.contains(&marker) {
        return Ok(done(
            op_id, device, now, "applied", &rel, &current, "append",
        ));
    }
    let note =
        crate::vault::mutate_note(vault, &rel, |cur| Ok(add_to_section(cur, heading, &line)))
            .map_err(|e| msg_step(&rel, e))?;
    Ok(done(
        op_id,
        device,
        now,
        "applied",
        &rel,
        &note.content,
        "append",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Setup {
        vault: tempfile::TempDir,
        _store_dir: tempfile::TempDir,
        store: Store,
        device: Device,
    }

    fn setup() -> Setup {
        let vault = tempfile::tempdir().unwrap();
        let v = vault.path();
        let write = |rel: &str, text: &str| {
            let p = v.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("notes/idea.md", "---\ntitle: Idea\n---\n\nFirst.\n");
        write("library/work/secret.md", "work");
        write("me/Identity.md", "me");
        write("workspaces/hollow-1/WORKSPACE.md", "---\ntitle: hollow\n---\n\n# hollow\n\n## Current context\n\n- earlier\n\n## Decisions\n\n- d\n");
        write("workspaces/hollow-1/PLAN.md", "# Plan\n\n- [ ] one\n");
        write("workspaces/w-2/WORKSPACE.md", "---\ntitle: atlas2.0\n---\n");
        let store_dir = tempfile::tempdir().unwrap();
        let store = Store::at(store_dir.path().to_path_buf());
        companion_docs::save_scope(
            &store,
            &companion_docs::Scope {
                include_work_data: false,
                personal_workspaces: vec!["hollow-1".into()],
                seeded_at: Some("x".into()),
            },
        )
        .unwrap();
        Setup {
            vault,
            _store_dir: store_dir,
            store,
            device: Device {
                device_id: "web-11223344".into(),
                name: "Safari".into(),
                platform: "web".into(),
                public_key: String::new(),
                paired_at: String::new(),
                last_op_at: None,
            },
        }
    }

    fn payload(v: Value) -> Map<String, Value> {
        serde_json::from_value(v).unwrap()
    }

    fn receipt(r: Result<Outcome, Step>) -> Receipt {
        match r {
            Ok(Outcome::Done { receipt, .. }) => receipt,
            Ok(Outcome::Waiting) => panic!("waiting"),
            Ok(Outcome::Held { .. }) => panic!("held"),
            Err(Step::Refused(m)) => panic!("refused: {m}"),
            Err(Step::Busy(m)) => panic!("busy: {m}"),
        }
    }

    fn refusal(r: Result<Outcome, Step>) -> String {
        match r {
            Err(Step::Refused(m)) => m,
            Ok(_) => panic!("applied"),
            Err(Step::Busy(m)) => panic!("busy: {m}"),
        }
    }

    const OP: &str = "01JA9M3Q8Z5X7Y2W4V6T8S0R1P";

    #[test]
    fn a_new_note_never_overwrites() {
        let s = setup();
        let now = Local::now();
        let p = payload(json!({ "title": "Brainstorm", "body": "From the phone" }));
        let r = receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &p,
            now,
        ));
        assert_eq!(r.reference.as_deref(), Some("vault:notes/Brainstorm.md"));
        let r2 = receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R2P",
            &p,
            now,
        ));
        assert_eq!(r2.reference.as_deref(), Some("vault:notes/Brainstorm 2.md"));
        let text = std::fs::read_to_string(s.vault.path().join("notes/Brainstorm.md")).unwrap();
        assert!(text.starts_with("---\ntitle: Brainstorm\n"), "{text}");
        assert!(text.contains("companion: Safari"), "{text}");
        assert!(text.ends_with("From the phone\n"));
        let lib = payload(json!({ "title": "x/../../me", "folder": "library/notes" }));
        let r3 = receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &lib,
            now,
        ));
        assert_eq!(
            r3.reference.as_deref(),
            Some("vault:library/notes/x....me.md")
        );
        let bad = payload(json!({ "title": "x", "folder": "me" }));
        assert!(refusal(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &bad,
            now
        ))
        .contains("notes go in"));
    }

    #[test]
    fn an_edit_keeps_front_matter_and_a_stale_one_saves_a_copy() {
        let s = setup();
        let now = Local::now();
        let current = std::fs::read_to_string(s.vault.path().join("notes/idea.md")).unwrap();
        let base = crate::vault::revision(current.as_bytes());
        let p = payload(
            json!({ "ref": "vault:notes/idea.md", "baseHash": base, "body": "\nSecond.\n" }),
        );
        let r = receipt(note_edit(s.vault.path(), &s.store, &s.device, OP, &p, now));
        assert_eq!(r.status, "applied");
        let text = std::fs::read_to_string(s.vault.path().join("notes/idea.md")).unwrap();
        assert_eq!(text, "---\ntitle: Idea\n---\n\nSecond.\n");
        // The same base again is now stale: the Mac's text stays, the device's is a copy.
        let r = receipt(note_edit(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/idea.md", "baseHash": base, "body": "Third." })),
            now,
        ));
        assert_eq!(r.status, "conflict");
        assert!(
            std::fs::read_to_string(s.vault.path().join("notes/idea.md"))
                .unwrap()
                .contains("Second.")
        );
        let copy = r.reference.unwrap();
        assert!(copy.starts_with("vault:notes/idea (from Safari "), "{copy}");
        let copied =
            std::fs::read_to_string(s.vault.path().join(copy.trim_start_matches("vault:")))
                .unwrap();
        assert!(copied.contains("Third."));
        // Its own title, so two notes don't share one (H5 review).
        let title = crate::frontmatter::parse(&copied)
            .get("title")
            .unwrap_or_default()
            .to_string();
        assert!(title.starts_with("Idea (from Safari "), "{title}");
        assert!(r.reason.unwrap().contains("changed on the Mac"));
    }

    #[test]
    fn a_copy_of_an_untitled_or_block_titled_note_keeps_its_front_matter() {
        let s = setup();
        let now = Local::now();
        let v = s.vault.path();
        for (name, text) in [
            ("bare.md", "---\ntags: [a]\n---\n\nOne.\n"),
            ("block.md", "---\ntitle: |\n  Long title\n---\n\nOne.\n"),
        ] {
            std::fs::write(v.join("notes").join(name), text).unwrap();
            let stale =
                payload(json!({ "ref": format!("notes/{name}"), "baseHash": "0", "body": "Two." }));
            let r = receipt(note_edit(v, &s.store, &s.device, OP, &stale, now));
            assert_eq!(r.status, "conflict");
            let copy =
                std::fs::read_to_string(v.join(r.reference.unwrap().trim_start_matches("vault:")))
                    .unwrap();
            let head = text.split("\n---\n").next().unwrap();
            assert!(copy.starts_with(head), "{name}: {copy}");
        }
    }

    #[test]
    fn appends_add_a_paragraph_and_workspace_lines_land_in_their_section() {
        let s = setup();
        let now = Local::now();
        receipt(note_append(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/idea.md", "text": "More." })),
            now,
        ));
        assert!(
            std::fs::read_to_string(s.vault.path().join("notes/idea.md"))
                .unwrap()
                .contains("First.\n\nMore.\n\n<!-- op:")
        );
        receipt(workspace_append(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "workspace": "hollow-1", "file": "brief", "text": "Called\nJordan" })),
            now,
        ));
        let brief =
            std::fs::read_to_string(s.vault.path().join("workspaces/hollow-1/WORKSPACE.md"))
                .unwrap();
        let ctx = brief.find("## Current context").unwrap();
        let dec = brief.find("## Decisions").unwrap();
        let line = brief.find("Safari: Called Jordan").unwrap();
        assert!(ctx < line && line < dec, "{brief}");
        assert!(brief.contains("- earlier\n- "), "{brief}");
        receipt(workspace_append(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "workspace": "hollow-1", "file": "plan", "text": "Buy milk" })),
            now,
        ));
        let plan =
            std::fs::read_to_string(s.vault.path().join("workspaces/hollow-1/PLAN.md")).unwrap();
        assert!(
            plan.contains("## From your devices\n\n- [ ] Buy milk (Safari, "),
            "{plan}"
        );
        assert!(plan.contains("- [ ] one"), "{plan}");
    }

    #[test]
    fn targets_outside_the_scope_or_through_symlinks_are_refused() {
        let s = setup();
        let now = Local::now();
        let v = s.vault.path();
        for (p, why) in [
            (
                json!({ "ref": "me/Identity.md", "text": "x" }),
                "isn't something",
            ),
            (
                json!({ "ref": "library/work/secret.md", "text": "x" }),
                "not a note",
            ),
            (
                json!({ "ref": "notes/../me/Identity.md", "text": "x" }),
                "isn't something",
            ),
            (json!({ "ref": "tasks/a.md", "text": "x" }), "is a task"),
            (
                json!({ "ref": "workspaces/hollow-1/PLAN.md", "text": "x" }),
                "is a workspace",
            ),
        ] {
            let m = refusal(note_append(
                v,
                &s.store,
                &s.device,
                OP,
                &payload(p.clone()),
                now,
            ));
            assert!(m.contains(why), "{p}: {m}");
        }
        let m = refusal(workspace_append(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "workspace": "w-2", "file": "plan", "text": "x" })),
            now,
        ));
        assert!(m.contains("isn't one this Hollow shares"), "{m}");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(v.join("me/Identity.md"), v.join("notes/me.md")).unwrap();
            let m = refusal(note_append(
                v,
                &s.store,
                &s.device,
                OP,
                &payload(json!({ "ref": "notes/me.md", "text": "x" })),
                now,
            ));
            assert!(m.contains("a symlink"), "{m}");
            assert_eq!(
                std::fs::read_to_string(v.join("me/Identity.md")).unwrap(),
                "me"
            );
        }
        // An unknown key burns nothing new: it's refused like the task kinds do.
        let m = refusal(note_append(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/idea.md", "text": "x", "extra": 1 })),
            now,
        ));
        assert!(m.contains("unknown key"), "{m}");
    }

    #[test]
    fn a_repeat_after_a_lost_ledger_write_applies_nothing_twice() {
        let s = setup();
        let v = s.vault.path();
        let now = Local::now();
        let create = payload(json!({ "title": "Once", "body": "b" }));
        let a = receipt(note_create(v, &s.store, &s.device, OP, &create, now));
        let b = receipt(note_create(v, &s.store, &s.device, OP, &create, now));
        assert_eq!(a.reference, b.reference);
        assert!(!v.join("notes/Once 2.md").exists());
        let append = payload(json!({ "ref": "notes/idea.md", "text": "Again" }));
        receipt(note_append(v, &s.store, &s.device, OP, &append, now));
        receipt(note_append(v, &s.store, &s.device, OP, &append, now));
        let text = std::fs::read_to_string(v.join("notes/idea.md")).unwrap();
        assert_eq!(text.matches("Again").count(), 1, "{text}");
        let line = payload(json!({ "workspace": "hollow-1", "file": "plan", "text": "Milk" }));
        receipt(workspace_append(v, &s.store, &s.device, OP, &line, now));
        receipt(workspace_append(v, &s.store, &s.device, OP, &line, now));
        let plan = std::fs::read_to_string(v.join("workspaces/hollow-1/PLAN.md")).unwrap();
        assert_eq!(plan.matches("Milk").count(), 1, "{plan}");
        // An edit already on disk counts as applied, not as a conflict.
        let cur = std::fs::read_to_string(v.join("notes/idea.md")).unwrap();
        let base = crate::vault::revision(cur.as_bytes());
        let edit = payload(json!({ "ref": "notes/idea.md", "baseHash": base, "body": "New" }));
        assert_eq!(
            receipt(note_edit(v, &s.store, &s.device, OP, &edit, now)).status,
            "applied"
        );
        assert_eq!(
            receipt(note_edit(v, &s.store, &s.device, OP, &edit, now)).status,
            "applied"
        );
    }

    #[test]
    fn problems_that_never_clear_are_refused_not_busy() {
        let s = setup();
        let v = s.vault.path();
        let now = Local::now();
        std::fs::write(v.join("notes/latin1.md"), [0x63u8, 0x61, 0x66, 0xe9]).unwrap();
        let m = refusal(note_append(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/latin1.md", "text": "x" })),
            now,
        ));
        assert!(m.contains("UTF-8"), "{m}");
        std::fs::create_dir_all(v.join("notes/folder.md")).unwrap();
        refusal(note_append(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/folder.md", "text": "x" })),
            now,
        ));
        let m = refusal(note_create(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "title": "x", "body": 5 })),
            now,
        ));
        assert!(m.contains("must be text"), "{m}");
        // A path through a file is refused, not retried forever.
        let edit = payload(json!({ "ref": "notes/idea.md/x.md", "baseHash": "0", "body": "x" }));
        refusal(note_edit(v, &s.store, &s.device, OP, &edit, now));
    }

    #[test]
    fn a_held_lease_makes_the_op_wait() {
        let s = setup();
        let v = s.vault.path();
        let _held = crate::automated_history::companion_write_lease(v).unwrap();
        let r = note_append(
            v,
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "ref": "notes/idea.md", "text": "x" })),
            Local::now(),
        );
        assert!(matches!(r, Err(Step::Busy(_))));
    }

    #[test]
    fn names_bodies_and_sections_keep_their_shape() {
        // Byte-capped names (a CJK title can't make an unwritable file).
        let stem = file_stem(&"漢".repeat(200));
        assert!(stem.len() <= STEM_MAX_BYTES && !stem.is_empty());
        assert_eq!(one_line("a\rb\u{2028}c\u{85}d"), "a b c d");
        // BOM and front matter survive an edit; a body like front matter on a
        // note without any doesn't become front matter.
        assert_eq!(
            with_body("\u{feff}---\ntitle: x\n---\n\nold\n", "\nnew\n"),
            "\u{feff}---\ntitle: x\n---\n\nnew\n"
        );
        // An unchanged body, as the read API serves it, round-trips byte for byte.
        for c in [
            "---\ntitle: a\ncompanion-op: X\n---\n\nbody\n",
            "---\r\ntitle: a\r\n---\r\n\r\nbody\r\n",
            "\u{feff}---\nt: 1\n---\nx",
            "plain\n",
        ] {
            assert_eq!(with_body(c, split(c).2), c);
        }
        assert_eq!(
            with_body("plain\n", "---\nscope: work\n---\nx\n"),
            "\n---\nscope: work\n---\nx\n"
        );
        assert_eq!(with_body("plain\n", "new\n"), "new\n");
        // Sections: CRLF kept, heading case ignored, fenced headings skipped.
        let crlf = add_to_section(
            "# A\r\n\r\n## current context\r\n\r\n- one\r\n",
            BRIEF_SECTION,
            "- two",
        );
        assert_eq!(
            crlf,
            "# A\r\n\r\n## current context\r\n\r\n- one\r\n- two\r\n"
        );
        let fenced = add_to_section(
            "## Current context\n\n- one\n\n```\n## not a heading\n```\n\n## Next\n\nx\n",
            BRIEF_SECTION,
            "- two",
        );
        assert!(fenced.contains("```\n- two\n\n## Next"), "{fenced}");
        assert_eq!(
            add_to_section("", PLAN_SECTION, "- [ ] x"),
            "## From your devices\n\n- [ ] x\n"
        );
    }

    #[test]
    fn a_device_makes_at_most_200_files_a_day() {
        let s = setup();
        let now = Local::now();
        for n in 0..FILES_PER_DAY {
            let mut r = Receipt::new(
                &format!("01JA9M3Q8Z5X7Y2W4V6T8S{n:04}"),
                &s.device.device_id,
                now,
                "applied",
            );
            r.reference = Some("vault:notes/x.md".into());
            s.store.record("note.create", &r).unwrap();
        }
        let m = refusal(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S9999",
            &payload(json!({ "title": "One too many" })),
            now,
        ));
        assert!(m.contains("the most a device may"), "{m}");
        // Appends make no files: still fine.
        receipt(note_append(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S9998",
            &payload(json!({ "ref": "notes/idea.md", "text": "ok" })),
            now,
        ));
    }

    /// The ledger rows of [`FILES_PER_DAY`] files made at `at`.
    fn fill_files(s: &Setup, at: DateTime<Local>) {
        for n in 0..FILES_PER_DAY {
            let mut r = Receipt::new(
                &format!("01JA9M3Q8Z5X7Y2W4V6T8F{n:04}"),
                &s.device.device_id,
                at,
                "applied",
            );
            r.reference = Some("vault:notes/x.md".into());
            s.store.record("note.create", &r).unwrap();
        }
    }

    #[test]
    fn a_full_day_still_answers_repeats() {
        let s = setup();
        let now = Local::now();
        let create = payload(json!({ "title": "Kept" }));
        let first = receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &create,
            now,
        ));
        // A stale edit saves a copy before the day fills up.
        let current = std::fs::read_to_string(s.vault.path().join("notes/idea.md")).unwrap();
        let base = crate::vault::revision(current.as_bytes());
        let ok = payload(json!({ "ref": "notes/idea.md", "baseHash": base, "body": "Two" }));
        receipt(note_edit(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R2A",
            &ok,
            now,
        ));
        let stale = payload(json!({ "ref": "notes/idea.md", "baseHash": base, "body": "Three" }));
        let copy = receipt(note_edit(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R2B",
            &stale,
            now,
        ));
        assert_eq!(copy.status, "conflict");
        fill_files(&s, now);
        // Repeats make no new file, so the full day doesn't refuse them.
        let again = receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &create,
            now,
        ));
        assert_eq!(again.reference, first.reference);
        let copy_again = receipt(note_edit(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R2B",
            &stale,
            now,
        ));
        assert_eq!(copy_again.reference, copy.reference);
        // A new stale edit would make a new copy: refused, and the Mac's text stays.
        let m = refusal(note_edit(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R2C",
            &stale,
            now,
        ));
        assert!(m.contains("the most a device may"), "{m}");
        assert!(
            std::fs::read_to_string(s.vault.path().join("notes/idea.md"))
                .unwrap()
                .contains("Two")
        );
    }

    #[test]
    fn files_older_than_a_day_dont_count() {
        let s = setup();
        let now = Local::now();
        fill_files(&s, now - chrono::Duration::hours(25));
        receipt(note_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &payload(json!({ "title": "Tomorrow" })),
            now,
        ));
    }

    #[test]
    fn task_creates_count_toward_the_day() {
        let s = setup();
        let now = Local::now();
        let create = payload(json!({ "kind": "task.create", "title": "Buy milk" }));
        let first = receipt(super::super::apply_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &create,
            now,
        ));
        assert_eq!(first.status, "applied");
        fill_files(&s, now);
        let again = receipt(super::super::apply_create(
            s.vault.path(),
            &s.store,
            &s.device,
            OP,
            &create,
            now,
        ));
        assert_eq!(again.reference, first.reference);
        let m = refusal(super::super::apply_create(
            s.vault.path(),
            &s.store,
            &s.device,
            "01JA9M3Q8Z5X7Y2W4V6T8S0R3A",
            &payload(json!({ "kind": "task.create", "title": "Buy bread" })),
            now,
        ));
        assert!(m.contains("the most a device may"), "{m}");
    }
}
