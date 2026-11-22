//! The phone API's workspace routes (v1.2, overnight 11-07 lane I3;
//! contract `docs/phone-api/workspaces.md`, fixtures
//! `docs/phone-api-fixtures/workspaces/`): read-only.
//!
//! - `GET /v1/workspaces/overview`: the cards the app's Workspaces tab lists,
//!   pinned ones first.
//! - `GET /v1/workspaces/{id}`: one workspace's outcome, plan, chats and
//!   Results.
//! - `GET /v1/workspaces/{id}/file?path=`: one file of the workspace's folder
//!   for the app's preview, as text or base64.
//!
//! Nothing here writes: no route touches a brief, a plan, a Result's review
//! state or a file. Every path is confined to the workspace's own folder:
//! no `..`, no hidden segment (`.hollow/` holds Hollow's own state), no
//! symbolic link out ([`crate::vault::safe_join`] and a folder check).

use super::routes::{self, json, ApiError, Ctx, Reply};
use super::wire;
use crate::workspaces::{self, Workspace};
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Cards `GET /v1/workspaces/overview` lists, at most.
pub(crate) const OVERVIEW_MAX: usize = 200;
/// A card's next step, at most this many characters.
const NEXT_STEP_MAX: usize = 240;
/// The brief's Outcome section, at most this many characters.
const OUTCOME_MAX: usize = 4_000;
/// The plan's text, at most this many characters.
const PLAN_MAX: usize = 60_000;
/// Chats a detail lists, at most.
const CHATS_MAX: usize = 50;
/// Results a detail lists, at most.
const RESULTS_MAX: usize = 100;
/// Entries (files and folders) the Results walk looks at, at most.
const RESULTS_WALK_MAX: usize = 5_000;
/// A text file's content, at most this many bytes (`truncated` past it).
const TEXT_FILE_MAX: usize = 1024 * 1024;
/// Any other file, at most this many bytes (413 past it).
const BINARY_FILE_MAX: u64 = 10 * 1024 * 1024;
/// How much of a brief or plan is read.
const DOC_READ_MAX: u64 = 512 * 1024;
/// A Result's title comes from this much of its start.
const TITLE_HEAD: usize = 16 * 1024;
/// A file path, at most this many characters.
const PATH_MAX: usize = 400;
/// The context documents a detail names, in this order.
const CONTEXT_DOCS: [&str; 4] = ["WORKSPACE.md", "PLAN.md", "SOURCES.md", "AGENTS.md"];

// ── Wire shapes ──────────────────────────────────────────────────────

/// A workspace as the Workspaces tab lists it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Card {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub pinned: Option<u32>,
    pub modified: String,
    pub last_activity_at: Option<String>,
    pub chat_count: usize,
    pub running_count: usize,
    pub waiting_count: usize,
    pub next_step: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Overview {
    pub workspaces: Vec<Card>,
    pub total: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Plan {
    pub text: String,
    pub truncated: bool,
    pub open_items: usize,
    pub modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct Document {
    pub name: String,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct ResultRow {
    pub path: String,
    pub name: String,
    pub title: String,
    pub kind: String,
    pub size: u64,
    pub modified: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Detail {
    pub workspace: Card,
    pub outcome: Option<String>,
    pub plan: Option<Plan>,
    pub documents: Vec<Document>,
    pub chats: Vec<wire::Session>,
    pub chats_has_more: bool,
    pub results: Vec<ResultRow>,
    pub results_has_more: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct FileBody {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub mime: String,
    pub size: u64,
    pub modified: Option<String>,
    pub encoding: String,
    pub content: String,
    pub truncated: bool,
}

// ── Overview ───────────────────────────────────────────────────────────

/// What the chats of one workspace add up to.
#[derive(Default)]
struct Counts {
    chats: usize,
    running: usize,
    waiting: usize,
    newest: Option<String>,
}

/// Each workspace's chats, counted from the newest 1,000 unarchived
/// conversations (the list's own horizon).
fn counts(vault: &Path) -> Result<HashMap<String, Counts>, String> {
    let homes = workspaces::SessionHomes::read(vault)?;
    let queue = routes::QueueView::read(vault);
    let waiting = routes::waiting_cards(vault);
    let mut by_workspace: HashMap<String, Counts> = HashMap::new();
    for row in crate::session_store::list_digest(vault, 1_000)? {
        let Some(id) = homes.workspace_for(&row.cwd) else {
            continue;
        };
        let slot = by_workspace.entry(id).or_default();
        slot.chats += 1;
        if queue.state(&row.key) != "idle" {
            slot.running += 1;
        }
        if waiting.contains(&row.key) {
            slot.waiting += 1;
        }
        if slot
            .newest
            .as_deref()
            .is_none_or(|n| row.last_activity_at.as_str() > n)
        {
            slot.newest = Some(row.last_activity_at.clone());
        }
    }
    Ok(by_workspace)
}

/// Opens a regular file for reading and returns it with its metadata, read
/// from the open handle: the check and the read are about the same file. It
/// never blocks on what is there (a named pipe or device that replaced the
/// file opens at once and fails the type check) and refuses a link at the
/// last step.
pub(crate) fn open_regular(path: &Path) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    #[cfg(unix)]
    let flags = libc::O_NONBLOCK | libc::O_NOFOLLOW;
    #[cfg(not(unix))]
    let flags = 0;
    open_with(path, flags)
}

/// [`open_regular`] for a path already resolved (`vault::safe_join`
/// canonicalizes one that exists): on macOS no link anywhere in it
/// (`O_NOFOLLOW_ANY`, as attachment downloads open theirs), so a parent
/// folder swapped for a link between the resolve and the open is refused
/// rather than followed out of the workspace (Q-R21).
pub(crate) fn open_resolved(path: &Path) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    #[cfg(unix)]
    let flags = super::attachments::OPEN_NO_LINKS;
    #[cfg(not(unix))]
    let flags = 0;
    open_with(path, flags)
}

fn open_with(path: &Path, flags: i32) -> std::io::Result<(std::fs::File, std::fs::Metadata)> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(flags);
    }
    #[cfg(not(unix))]
    let _ = flags;
    let file = options.open(path)?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::Error::other("not a regular file"));
    }
    Ok((file, meta))
}

/// Up to `cap` bytes of a file's start as text (cut at a character
/// boundary), the file's whole size and its modified time. `None` when it
/// can't be read.
fn read_prefix(path: &Path, cap: u64) -> Option<(String, u64, Option<String>)> {
    let (file, meta) = open_resolved(path).ok()?;
    let mut bytes = Vec::new();
    file.take(cap).read_to_end(&mut bytes).ok()?;
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) => {
            let valid = e.utf8_error().valid_up_to();
            let mut bytes = e.into_bytes();
            bytes.truncate(valid);
            String::from_utf8(bytes).ok()?
        }
    };
    Some((text, meta.len(), modified_of(&meta)))
}

fn modified_of(meta: &std::fs::Metadata) -> Option<String> {
    meta.modified()
        .ok()
        .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339())
}

/// Whether `path` (resolved, links followed) sits inside `dir` (resolved)
/// with no hidden part below it: `.hollow/` holds Hollow's own state, and a
/// link inside the workspace must not reach it.
fn inside(dir: &Path, path: &Path) -> bool {
    path.strip_prefix(dir).is_ok_and(|rest| {
        rest.components()
            .all(|part| !part.as_os_str().to_string_lossy().starts_with('.'))
    })
}

/// The file `name` of a workspace's folder, resolved through the vault's own
/// path check and kept only when it still sits inside the workspace's folder:
/// a brief, plan or other document that is a link to another workspace's
/// file, or to anything else, is as good as missing.
fn document_path(vault: &Path, w: &Workspace, name: &str) -> Option<PathBuf> {
    let dir = crate::vault::safe_join(vault, &w.path).ok()?;
    let path = crate::vault::safe_join(vault, &format!("{}/{name}", w.path)).ok()?;
    inside(&dir, &path).then_some(path)
}

/// A workspace document's text (see [`document_path`]).
fn document(vault: &Path, w: &Workspace, name: &str) -> Option<(String, u64, Option<String>)> {
    read_prefix(&document_path(vault, w, name)?, DOC_READ_MAX)
}

/// The text of `text` without its front matter.
fn body_of(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("---\n") else {
        return text;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start_matches(['\r', '\n']),
        None => text,
    }
}

/// The Markdown under the `## <name>` heading of a brief, up to the next
/// heading of its level or above, outside code fences; `None` when the
/// section is missing or empty. At most `max` characters.
pub(crate) fn section(text: &str, name: &str, max: usize) -> Option<String> {
    let mut found = false;
    let mut fence: Option<char> = None;
    let mut lines: Vec<&str> = Vec::new();
    for line in body_of(text).lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap_or('`');
            if fence == Some(marker) {
                fence = None;
            } else if fence.is_none() {
                fence = Some(marker);
            }
            if found {
                lines.push(line);
            }
            continue;
        }
        if fence.is_none() {
            if let Some(heading) = trimmed.strip_prefix("## ") {
                if found {
                    break;
                }
                found = heading
                    .trim()
                    .trim_end_matches('#')
                    .trim()
                    .eq_ignore_ascii_case(name);
                continue;
            }
            if found && trimmed.starts_with("# ") {
                break;
            }
        }
        if found {
            lines.push(line);
        }
    }
    let joined = lines.join("\n");
    let value = joined.trim();
    if value.is_empty() {
        None
    } else {
        Some(super::live::cut_chars(value, max))
    }
}

/// A brief's Next step as one plain line, at most [`NEXT_STEP_MAX`].
fn next_step_of(vault: &Path, w: &Workspace) -> Option<String> {
    let (text, _, _) = document(vault, w, "WORKSPACE.md")?;
    let step = crate::workspace_reentry::next_step(&text)?;
    Some(super::live::cut_chars(&step, NEXT_STEP_MAX))
}

fn card(vault: &Path, w: &Workspace, counts: Option<&Counts>) -> Card {
    card_with(w, counts, next_step_of(vault, w))
}

/// A card whose next step the caller has already read.
fn card_with(w: &Workspace, counts: Option<&Counts>, next_step: Option<String>) -> Card {
    Card {
        id: w.id.clone(),
        title: w.title.clone(),
        kind: workspaces::origin_kind(w).to_string(),
        status: w.status.clone(),
        pinned: w.pinned,
        modified: w.modified.clone(),
        last_activity_at: counts.and_then(|c| c.newest.clone()),
        chat_count: counts.map_or(0, |c| c.chats),
        running_count: counts.map_or(0, |c| c.running),
        waiting_count: counts.map_or(0, |c| c.waiting),
        next_step,
    }
}

/// The workspaces whose title, status, brief Outcome or Next step together hold
/// every one of `words` (compared lowercased; each word may sit in a different
/// field): every workspace that is not in the trash, archived ones included,
/// the chat folders and the Main agent left out. The ones whose title holds
/// every word come first; each group is in the overview's order. Returns at
/// most `max` cards and how many matched in all.
pub(super) fn find(
    vault: &Path,
    words: &[String],
    max: usize,
) -> Result<(Vec<Card>, usize), String> {
    let words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
    let holds = |text: &str| words.iter().all(|word| text.contains(word.as_str()));
    let listed = workspaces::list(vault)?;
    let counted = counts(vault)?;
    let (mut by_title, mut by_text) = (Vec::new(), Vec::new());
    for w in listed.iter().filter(|w| !workspaces::is_builtin(&w.id)) {
        let brief = document(vault, w, "WORKSPACE.md").map(|(text, _, _)| text);
        let outcome = brief
            .as_deref()
            .and_then(|text| section(text, "outcome", OUTCOME_MAX));
        let next_step = brief
            .as_deref()
            .and_then(crate::workspace_reentry::next_step)
            .map(|step| super::live::cut_chars(&step, NEXT_STEP_MAX));
        let fields = [
            w.title.as_str(),
            w.status.as_str(),
            outcome.as_deref().unwrap_or(""),
            next_step.as_deref().unwrap_or(""),
        ]
        .join("\n")
        .to_lowercase();
        if !holds(&fields) {
            continue;
        }
        let hit = card_with(w, counted.get(&w.id), next_step);
        if holds(&w.title.to_lowercase()) {
            by_title.push(hit);
        } else {
            by_text.push(hit);
        }
    }
    sort_cards(&mut by_title);
    sort_cards(&mut by_text);
    by_title.extend(by_text);
    let total = by_title.len();
    by_title.truncate(max);
    Ok((by_title, total))
}

/// Whether the Work page lists the workspace as current (the rule
/// `GET /v1/workspaces` uses).
fn is_current(w: &Workspace) -> bool {
    workspaces::CURRENT_STATUSES
        .iter()
        .any(|s| w.status.to_lowercase() == *s)
}

/// The overview's order: pinned workspaces by rank, then the newest
/// conversation (else the brief's change), then the title.
fn sort_cards(cards: &mut [Card]) {
    cards.sort_by(|a, b| {
        let rank = |c: &Card| c.pinned.unwrap_or(u32::MAX);
        let recent = |c: &Card| {
            c.last_activity_at
                .clone()
                .unwrap_or_else(|| c.modified.clone())
        };
        rank(a)
            .cmp(&rank(b))
            .then_with(|| recent(b).cmp(&recent(a)))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
    });
}

pub(crate) async fn overview(ctx: &Arc<Ctx>, query: &str) -> Result<Reply, ApiError> {
    // Absent is the default; present and anything but these two is a 400.
    let all = match routes::query_param(query, "scope").as_deref() {
        None | Some("current") => false,
        Some("all") => true,
        Some(_) => {
            return Err(ApiError::bad_request(
                "scope must be current or all.".to_string(),
            ))
        }
    };
    let ctx = ctx.clone();
    let body = routes::blocking(move || {
        let listed = workspaces::list(&ctx.vault).map_err(ApiError::internal)?;
        let counted = counts(&ctx.vault).map_err(ApiError::internal)?;
        // The order doesn't depend on the next step, so sort and cut first and
        // read the briefs of the cards that stay only.
        let mut cards: Vec<Card> = listed
            .iter()
            .filter(|w| !workspaces::is_builtin(&w.id))
            .filter(|w| all || is_current(w))
            .map(|w| card_with(w, counted.get(&w.id), None))
            .collect();
        sort_cards(&mut cards);
        let total = cards.len();
        cards.truncate(OVERVIEW_MAX);
        for kept in &mut cards {
            if let Some(w) = listed.iter().find(|w| w.id == kept.id) {
                kept.next_step = next_step_of(&ctx.vault, w);
            }
        }
        Ok(Overview {
            workspaces: cards,
            total,
        })
    })
    .await?;
    Ok(json(200, &body))
}

// ── Detail ─────────────────────────────────────────────────────────────

fn missing() -> ApiError {
    ApiError::not_found("Hollow has no such workspace.")
}

/// The workspace `id`, or 404 for an unknown, invalid or trashed one.
fn workspace_of(vault: &Path, id: &str) -> Result<Workspace, ApiError> {
    workspaces::summary(vault, id).map_err(|_| missing())
}

/// Whether `rel` (relative to a workspace folder) is a Result: not hidden,
/// not Hollow's own listing exclusions.
fn is_result(rel: &str) -> bool {
    crate::workspace_results::is_listed(rel)
}

/// The newest files under `outputs/`, as Results lists them (dot folders, a
/// page review's captures, a canvas's sidecars and archived brief entries
/// left out), and whether more exist. No review state and no hashing: the
/// phone never accepts a Result.
///
/// The walk looks at `walk_max` entries at most, files and folders alike (a
/// built site under `outputs/` can hold 100,000, and every refetch would stat
/// them all): past it the newest of those reached first are listed and the
/// second value is true.
/// `outputs/` itself must be a real folder: a link there would list files
/// outside the workspace.
pub(crate) fn results_of(dir: &Path, walk_max: usize) -> (Vec<ResultRow>, bool) {
    let outputs = dir.join("outputs");
    if !std::fs::symlink_metadata(&outputs).is_ok_and(|meta| meta.file_type().is_dir()) {
        return (Vec::new(), false);
    }
    let mut found: Vec<(String, u64, Option<String>)> = Vec::new();
    let mut cut = false;
    // Every entry the walk visits counts, hidden ones too: the filter below
    // drops them before the loop sees them, so a folder of millions of
    // dotfiles was read in full without ever reaching the cap (R2). Past the
    // cap the filter lets one entry through so the loop stops there.
    let visited = std::cell::Cell::new(0usize);
    for entry in walkdir::WalkDir::new(&outputs)
        .max_depth(6)
        .follow_links(false)
        .follow_root_links(false)
        .into_iter()
        .filter_entry(|e| {
            if e.depth() > 0 {
                visited.set(visited.get() + 1);
                if visited.get() > walk_max {
                    return true;
                }
            }
            !e.file_name().to_string_lossy().starts_with('.')
                && !crate::workspace_results::is_review_capture_dir(e.path())
        })
    {
        let Ok(entry) = entry else { continue };
        if entry.depth() == 0 {
            continue;
        }
        // Folders and links count as well as files, so a wide tree of them
        // is cut too (the first delta review's low 1).
        if visited.get() > walk_max {
            cut = true;
            break;
        }
        if !entry.file_type().is_file() {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(dir) else {
            continue;
        };
        // Only a path the file route can open is listed: UTF-8, no
        // backslash or control character, within the length limit.
        let Some(rel) = rel.to_str() else { continue };
        if clean_path(rel).is_err() || !is_result(rel) {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        found.push((rel.to_string(), meta.len(), modified_of(&meta)));
    }
    found.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0)));
    let more = cut || found.len() > RESULTS_MAX;
    found.truncate(RESULTS_MAX);
    let rows = found
        .into_iter()
        .map(|(rel, size, modified)| {
            let head = head_text(&dir.join(&rel));
            ResultRow {
                title: crate::workspace_results::title_of(&rel, &head),
                kind: crate::workspace_results::kind_of(&rel).to_string(),
                name: rel.rsplit('/').next().unwrap_or(&rel).to_string(),
                path: rel,
                size,
                modified,
            }
        })
        .collect();
    (rows, more)
}

fn head_text(path: &Path) -> String {
    let mut buf = vec![0u8; TITLE_HEAD];
    let n = open_regular(path)
        .and_then(|(mut file, _)| file.read(&mut buf))
        .unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).into_owned()
}

/// The context documents that exist inside the workspace (a link out is not
/// one), in [`CONTEXT_DOCS`] order.
fn documents_of(vault: &Path, w: &Workspace) -> Vec<Document> {
    CONTEXT_DOCS
        .iter()
        .filter_map(|name| {
            let path = document_path(vault, w, name)?;
            let meta = std::fs::metadata(path).ok()?;
            meta.is_file().then(|| Document {
                name: (*name).to_string(),
                size: meta.len(),
                modified: modified_of(&meta),
            })
        })
        .collect()
}

pub(crate) async fn detail(ctx: &Arc<Ctx>, id: &str) -> Result<Reply, ApiError> {
    let ctx = ctx.clone();
    let id = id.to_string();
    let body = routes::blocking(move || {
        let w = workspace_of(&ctx.vault, &id)?;
        let dir = workspaces::folder(&ctx.vault, &id).map_err(|_| missing())?;
        let counted = counts(&ctx.vault).map_err(ApiError::internal)?;
        let outcome = document(&ctx.vault, &w, "WORKSPACE.md")
            .and_then(|(t, _, _)| section(&t, "outcome", OUTCOME_MAX));
        let plan = document(&ctx.vault, &w, "PLAN.md").map(|(text, size, modified)| Plan {
            open_items: crate::vault_tasks::open_checklist_items(&text).len(),
            truncated: size > DOC_READ_MAX || text.chars().count() > PLAN_MAX,
            text: super::live::cut_chars(&text, PLAN_MAX),
            modified,
        });
        let sessions = workspaces::sessions_in(&ctx.vault, &dir, &w).map_err(ApiError::internal)?;
        let chats_has_more = sessions.len() > CHATS_MAX;
        let shown = &sessions[..sessions.len().min(CHATS_MAX)];
        let chats = routes::session_rows(&ctx, shown).map_err(ApiError::internal)?;
        let (results, results_has_more) = results_of(&dir, RESULTS_WALK_MAX);
        Ok(Detail {
            workspace: card(&ctx.vault, &w, counted.get(&w.id)),
            outcome,
            plan,
            documents: documents_of(&ctx.vault, &w),
            chats,
            chats_has_more,
            results,
            results_has_more,
        })
    })
    .await?;
    Ok(json(200, &body))
}

// ── File ──────────────────────────────────────────────────────────────────

/// A relative path the file route accepts: `/`-separated, no empty, `.`,
/// `..` or hidden segment, no backslash or control character. It is used as
/// given (a file name may start or end with a space), never trimmed.
fn clean_path(raw: &str) -> Result<String, ApiError> {
    let bad = || {
        ApiError::bad_request("path must be a file inside the workspace, like outputs/notes.md.")
    };
    let path = raw;
    if path.trim().is_empty() || path.chars().count() > PATH_MAX {
        return Err(bad());
    }
    if path.starts_with('/') || path.contains('\\') || path.chars().any(char::is_control) {
        return Err(bad());
    }
    if path
        .split('/')
        .any(|segment| segment.is_empty() || segment.starts_with('.'))
    {
        return Err(bad());
    }
    Ok(path.to_string())
}

/// The media type for a file name and whether it travels as text.
pub(crate) fn mime_of(name: &str) -> (&'static str, bool) {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "md" | "markdown" => ("text/markdown", true),
        "txt" | "log" | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "sql" | "sh" | "swift"
        | "css" | "patch" | "diff" | "ini" | "conf" => ("text/plain", true),
        "csv" => ("text/csv", true),
        "tsv" => ("text/tab-separated-values", true),
        "json" => ("application/json", true),
        "yaml" | "yml" => ("application/yaml", true),
        "toml" => ("application/toml", true),
        "xml" => ("application/xml", true),
        "html" | "htm" => ("text/html", true),
        "pdf" => ("application/pdf", false),
        "png" => ("image/png", false),
        "jpg" | "jpeg" => ("image/jpeg", false),
        "gif" => ("image/gif", false),
        "webp" => ("image/webp", false),
        "heic" => ("image/heic", false),
        "svg" => ("image/svg+xml", false),
        "mp4" | "m4v" => ("video/mp4", false),
        "mov" => ("video/quicktime", false),
        "docx" => (
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            false,
        ),
        "xlsx" => (
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            false,
        ),
        "pptx" => (
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            false,
        ),
        "rtf" => ("application/rtf", false),
        _ => ("application/octet-stream", false),
    }
}

/// Standard base64 with padding.
fn base64(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn read_file(vault: &Path, id: &str, raw: &str) -> Result<FileBody, ApiError> {
    let rel = clean_path(raw)?;
    let w = workspace_of(vault, id)?;
    let dir = workspaces::folder(vault, id)
        .and_then(|d| d.canonicalize().map_err(|e| e.to_string()))
        .map_err(|_| missing())?;
    let gone = || ApiError::not_found("Hollow has no such file in this workspace.");
    let target: PathBuf =
        crate::vault::safe_join(vault, &format!("{}/{rel}", w.path)).map_err(|_| gone())?;
    // A file that resolves outside this workspace's folder (a link to
    // another workspace), or into a hidden folder of it (a link to
    // `.hollow/`), is as good as missing.
    if !inside(&dir, &target) {
        return Err(gone());
    }
    // One handle for the type check, the size and every read.
    let (file, meta) = open_resolved(&target).map_err(|_| gone())?;
    let size = meta.len();
    let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
    let (mime, text_type) = mime_of(&name);
    let kind = crate::workspace_results::kind_of(&rel).to_string();
    let modified = modified_of(&meta);
    let too_large =
        || ApiError::too_large("This file is too large to open on the phone. Open it on your Mac.");
    let read = |cap: u64| -> Result<Vec<u8>, ApiError> {
        use std::io::{Seek, SeekFrom};
        let mut handle = &file;
        handle.seek(SeekFrom::Start(0)).map_err(|_| gone())?;
        let mut bytes = Vec::new();
        handle
            .take(cap)
            .read_to_end(&mut bytes)
            .map_err(|_| gone())?;
        Ok(bytes)
    };
    let (mime, encoding, content, truncated) = if text_type {
        let bytes = read(TEXT_FILE_MAX as u64)?;
        let cut = size > TEXT_FILE_MAX as u64;
        match String::from_utf8(bytes) {
            Ok(text) => (mime, "utf8", text, cut),
            // A cut can land inside a character; a file that isn't text at
            // all travels as bytes.
            Err(e) if cut && e.utf8_error().error_len().is_none() => {
                let valid = e.utf8_error().valid_up_to();
                let mut bytes = e.into_bytes();
                bytes.truncate(valid);
                let text = String::from_utf8(bytes).map_err(|_| gone())?;
                (mime, "utf8", text, true)
            }
            Err(_) => {
                if size > BINARY_FILE_MAX {
                    return Err(too_large());
                }
                let all = read(BINARY_FILE_MAX)?;
                ("application/octet-stream", "base64", base64(&all), false)
            }
        }
    } else {
        if size > BINARY_FILE_MAX {
            return Err(too_large());
        }
        let all = read(BINARY_FILE_MAX)?;
        (mime, "base64", base64(&all), false)
    };
    Ok(FileBody {
        path: rel,
        name,
        kind,
        mime: mime.to_string(),
        size,
        modified,
        encoding: encoding.to_string(),
        content,
        truncated,
    })
}

/// `name`'s value in a query string, percent-decoded once. Unlike
/// [`routes::query_param`] a `+` stays a plus: a file name may hold one, and
/// a space travels as `%20`.
fn literal_query_param(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == name).then(|| {
            urlencoding::decode(v)
                .map(|v| v.into_owned())
                .unwrap_or_default()
        })
    })
}

pub(crate) async fn file(ctx: &Arc<Ctx>, id: &str, query: &str) -> Result<Reply, ApiError> {
    let raw = literal_query_param(query, "path").unwrap_or_default();
    let vault = ctx.vault.clone();
    let id = id.to_string();
    // The body can be 13 MB of base64: build the reply off the async workers too.
    routes::blocking(move || read_file(&vault, &id, &raw).map(|body| json(200, &body))).await
}
