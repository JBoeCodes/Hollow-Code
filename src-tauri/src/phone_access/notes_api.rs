//! The phone API's quick note (`POST /v1/notes`, v1.2, overnight 11-07 lane
//! I3; contract `docs/phone-api/notes.md`, fixtures
//! `docs/phone-api-fixtures/notes/`).
//!
//! A few words typed on the iPhone become a new Markdown file in `notes/`: the
//! file a paired companion device's `note.create` makes, through
//! [`crate::companion_inbox::notes::create_for_phone`] (the same front matter
//! and file name, a taken name gets ` 2`, never an overwrite, the short
//! companion write lease, no link on the path). It does not use the companion
//! ledger, which the phone API has none of: a repeat is found the way that
//! writer finds its own file (its name and its `companion-op` line), and the
//! day's limit is counted here, in memory, like the task routes'. Nothing
//! here edits, appends to, renames or deletes a note.

use super::live::cut_chars;
use super::routes::{self, json, ApiError, Ctx, Reply};
use super::store::Device;
use super::tasks_api::{parse_body, plain_line, reserve_create, valid_key, Reserved};
use crate::companion_inbox::notes;
use crate::task_broker::CompanionRefusal;
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;

/// A title is at most this many characters, given or made from the text (the
/// writer's own limit is the same).
const TITLE_MAX: usize = 200;
/// A title made from the text is its first line cut to this many characters.
const DERIVED_TITLE_CHARS: usize = 80;
/// The text, trimmed, is at most this many bytes (the writer's limit too).
const TEXT_MAX: usize = 48 * 1024;
/// New notes one iPhone may make in 24 hours: a runaway app can't flood the
/// vault. The Mac's own companion ops stop at 200 files a day too.
const NOTES_PER_DAY: usize = 200;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateRequest {
    idempotency_key: Option<String>,
    text: Option<String>,
    title: Option<String>,
}

/// The note as the phone is told of it.
#[derive(Debug, Serialize)]
pub(crate) struct NoteRow {
    pub path: String,
    pub title: String,
    pub created: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Written {
    pub note: NoteRow,
    pub disposition: &'static str,
}

/// This iPhone's key in the day's count ([`reserve_create`]), apart from the
/// one its tasks use (the bare device id): making tasks never uses up notes.
pub(crate) fn count_key(device_id: &str) -> String {
    format!("notes:{device_id}")
}

/// A real folder at `notes/` (or none yet). A link there would take the
/// phone's notes to another part of the vault, or out of it, so the route
/// leaves it alone.
fn notes_dir_is_real(vault: &Path) -> bool {
    std::fs::symlink_metadata(vault.join("notes")).map_or(true, |m| !m.file_type().is_symlink())
}

fn ensure_notes_dir(vault: &Path) -> Result<(), ApiError> {
    if notes_dir_is_real(vault) {
        Ok(())
    } else {
        Err(ApiError::bad_request(
            "The notes folder is a link, so Hollow leaves it alone on the iPhone.".to_string(),
        ))
    }
}

/// The text to keep, trimmed: 400 when it is missing, empty, over [`TEXT_MAX`]
/// bytes, or holds a control character other than a line break or a tab.
fn text_of(text: Option<&str>) -> Result<&str, ApiError> {
    let sent = text.unwrap_or("");
    // Checked as sent: trimming would hide a vertical tab or a next-line mark
    // at either end.
    if sent
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ApiError::bad_request(
            "text can't hold control characters, only line breaks and tabs.".to_string(),
        ));
    }
    let text = sent.trim();
    if text.is_empty() {
        return Err(ApiError::bad_request(
            "text is required: the words to keep.".to_string(),
        ));
    }
    if text.len() > TEXT_MAX {
        return Err(ApiError::bad_request("text is at most 48 KiB.".to_string()));
    }
    Ok(text)
}

/// The note's title: the one the request names (one plain line, at most
/// [`TITLE_MAX`] characters), else the first non-empty line of `text` cut to
/// [`DERIVED_TITLE_CHARS`] characters. `text` is trimmed and not empty, so a
/// first line with a word on it always exists.
fn title_of(given: Option<&str>, text: &str) -> Result<String, ApiError> {
    let Some(title) = given else {
        let line = text
            .split(['\n', '\r'])
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default();
        return Ok(cut_chars(line, DERIVED_TITLE_CHARS));
    };
    plain_line("title", title)?;
    let title = title.trim();
    if title.is_empty() {
        return Err(ApiError::bad_request(
            "title can't be empty: leave it out and the first line of the text is used."
                .to_string(),
        ));
    }
    if title.chars().count() > TITLE_MAX {
        return Err(ApiError::bad_request(format!(
            "title is at most {TITLE_MAX} characters."
        )));
    }
    Ok(title.to_string())
}

/// A write that did not land, as the contract's error.
fn refusal(error: CompanionRefusal) -> ApiError {
    match error {
        CompanionRefusal::Busy(reason) => {
            eprintln!("[phone] a note write waits: {reason}");
            ApiError::busy("Hollow is busy for a moment. Try again.".to_string())
        }
        CompanionRefusal::Refused(reason) => ApiError::bad_request(reason),
    }
}

pub(crate) async fn create(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: CreateRequest = parse_body(body)?;
    let key = request
        .idempotency_key
        .as_deref()
        .map(str::trim)
        .unwrap_or("");
    if !valid_key(key) {
        return Err(ApiError::bad_request(
            "idempotencyKey must be 8 to 64 letters, digits, dashes, underscores or dots."
                .to_string(),
        ));
    }
    let text = text_of(request.text.as_deref())?.to_string();
    let title = title_of(request.title.as_deref(), &text)?;
    // Scoped to this iPhone, so two phones never share a key.
    let op_id = format!("{}.{key}", device.device_id);
    let (vault, device, store) = (ctx.vault.clone(), device.clone(), ctx.store.clone());
    let written = routes::blocking(move || {
        ensure_notes_dir(&vault)?;
        // One of the day's slots is taken just before the file is written, and
        // only then: a retry of a key that already made its note is found
        // first and answers `duplicate`, so it never counts, and the limit
        // never stops it. Taken in one step under the counter's lock, so two
        // requests at the limit can't both pass.
        let mut reserved: Option<Reserved> = None;
        let revoked = std::cell::Cell::new(None);
        let note = notes::create_for_phone(
            &vault,
            &device.name,
            &op_id,
            &title,
            &text,
            Local::now(),
            || {
                // The note writer's lease can wait; the phone must still be
                // paired right before the file is written (R1).
                routes::commit_point(&store, &device.device_id, "note-write").map_err(|error| {
                    let text = error.message.clone();
                    revoked.set(Some(error));
                    text
                })?;
                let key = count_key(&device.device_id);
                match reserve_create(&key, Instant::now(), NOTES_PER_DAY) {
                    Some(slot) => {
                        reserved = Some(Reserved {
                            device_id: key,
                            slot,
                            keep: false,
                        });
                        Ok(())
                    }
                    None => Err(format!(
                        "This iPhone has made {NOTES_PER_DAY} notes in the last 24 hours, the most it may. Try again later."
                    )),
                }
            },
        )
        .map_err(|error| revoked.take().unwrap_or_else(|| refusal(error)))?;
        // The slot stays taken because this request made a file.
        if let Some(reserved) = reserved.as_mut() {
            reserved.keep = true;
        }
        Ok(Written {
            note: NoteRow {
                path: note.path,
                title: note.title,
                created: note.created,
            },
            disposition: if note.duplicate {
                "duplicate"
            } else {
                "applied"
            },
        })
    })
    .await?;
    Ok(json(200, &written))
}
