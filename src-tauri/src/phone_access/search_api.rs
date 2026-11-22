//! The phone API's search route (v1.2, overnight 11-07 lane I3; contract
//! `docs/phone-api/search.md`, fixtures `docs/phone-api-fixtures/search/`):
//! read-only.
//!
//! `GET /v1/search?q=` finds chats (every word in the title or in one of the
//! chat's scrubbed turns: the person's messages, replies, errors and short
//! tool summaries, the Mac's shared chat search index,
//! [`crate::workspaces::search_sessions_for_device`]) and workspaces (every
//! word in the title, status, brief Outcome or Next step:
//! [`super::workspaces_api::find`]). A chat is listed only when its
//! workspace is on the allow-list [`crate::companion_docs::shown_on_devices`]
//! builds: a workspace classified as personal, or with the companion's
//! "include work data" on any but a `linear-*` one, whose chats never leave
//! the Mac. A merged workspace, or one whose brief can't be read or
//! classified, counts as work. It writes no file; the search brings the
//! derived index up to date first, a bounded batch.

use super::routes::{self, json, ApiError, Ctx, Reply};
use super::wire;
use super::workspaces_api::{self, Card};
use crate::workspaces;
use serde::Serialize;
use std::sync::{Arc, OnceLock};
use tokio::sync::Semaphore;

/// The query, at most this many characters (its whitespace folded to single
/// spaces).
const QUERY_MAX: usize = 200;
/// Distinct words of the query that are read, as in the chat search
/// (`session_store::SEARCH_MAX_WORDS`): any after them are ignored for the
/// workspaces too, so both halves answer the same question.
const WORDS_MAX: usize = 32;
/// Searches that run at the same moment, at most. A search first reindexes
/// what changed (a bounded batch) and keeps going after the phone hangs up,
/// and the app asks again at each pause in typing: the rest wait their turn
/// here (a request whose phone has gone is dropped while it waits, and costs
/// nothing).
const SCANS_AT_ONCE: usize = 2;

fn scan_slots() -> &'static Semaphore {
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    SLOTS.get_or_init(|| Semaphore::new(SCANS_AT_ONCE))
}
/// Chats a page lists by default.
const CHATS_DEFAULT: usize = 20;
/// Chats a page lists, at most.
const CHATS_MAX: usize = 50;
/// Workspaces the first page lists, at most.
const WORKSPACES_MAX: usize = 10;
/// A snippet, at most this many characters.
const SNIPPET_MAX: usize = 240;

/// One chat that matched.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChatHit {
    pub session: wire::Session,
    /// The workspace the chat opens in on the Mac; null for a chat outside
    /// every workspace (the search leaves those out, so null is not sent
    /// today).
    pub workspace_id: Option<String>,
    /// The part of the earliest indexed turn (the person's message, the
    /// reply, an error or a tool summary, secrets redacted) that holds the
    /// first word the title lacks; null when the title holds every word.
    pub snippet: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchBody {
    pub query: String,
    pub workspaces: Vec<Card>,
    pub workspaces_total: usize,
    pub chats: Vec<ChatHit>,
    pub next_cursor: Option<String>,
}

pub(crate) async fn search(ctx: &Arc<Ctx>, query: &str) -> Result<Reply, ApiError> {
    let typed = routes::query_param(query, "q").unwrap_or_default();
    let q = typed.split_whitespace().collect::<Vec<_>>().join(" ");
    if q.is_empty() {
        return Err(ApiError::bad_request(
            "q is required: the words to search for.".to_string(),
        ));
    }
    if q.chars().count() > QUERY_MAX {
        return Err(ApiError::bad_request(format!(
            "q is at most {QUERY_MAX} characters."
        )));
    }
    let limit = routes::bounded(query, "limit", CHATS_DEFAULT, CHATS_MAX)?;
    let cursor = routes::query_param(query, "cursor").filter(|c| !c.is_empty());
    if let Some(cursor) = &cursor {
        crate::page_cursor::Keyset::decode(cursor)
            .map_err(|_| ApiError::bad_request("cursor is not one Hollow gave.".to_string()))?;
    }
    let ctx = ctx.clone();
    // The permit travels into the scan, so it is held until the scan ends,
    // not until the phone stops listening.
    let slot = scan_slots()
        .acquire()
        .await
        .map_err(|_| ApiError::internal("search is not available".to_string()))?;
    let body = routes::blocking(move || {
        let _slot = slot;
        // The chat search reads the first 32 distinct words (case folded): so do the workspaces.
        let mut words: Vec<String> = Vec::new();
        for word in q.split(' ') {
            let lower = word.to_lowercase();
            if words.len() < WORDS_MAX && !words.iter().any(|w| w.to_lowercase() == lower) {
                words.push(word.to_string());
            }
        }
        // The workspaces ride on the first page only.
        let (workspaces, workspaces_total) = if cursor.is_none() {
            workspaces_api::find(&ctx.vault, &words, WORKSPACES_MAX).map_err(ApiError::internal)?
        } else {
            (Vec::new(), 0)
        };
        // The companion's work-data rule, failing closed: the phone sees a
        // chat only when its workspace is classified as personal, or any
        // but a `linear-*` one when the person shares work data.
        let shown = crate::companion_docs::shown_on_devices(
            &ctx.vault,
            crate::companion_docs::work_data_on(ctx.store.dir()),
        );
        let page = workspaces::search_sessions_for_device(
            &ctx.vault,
            &q,
            cursor.as_deref(),
            Some(limit),
            &shown,
        )
        .map_err(ApiError::internal)?;
        crate::session_store::drain_search_index_soon(&ctx.vault);
        let summaries: Vec<_> = page.items.iter().map(|hit| hit.session.clone()).collect();
        let rows = routes::session_rows(&ctx, &summaries).map_err(ApiError::internal)?;
        let chats = page
            .items
            .into_iter()
            .zip(rows)
            .map(|(hit, session)| ChatHit {
                session,
                workspace_id: hit.workspace_id,
                snippet: hit
                    .snippet
                    .map(|snippet| super::live::cut_chars(&snippet, SNIPPET_MAX)),
            })
            .collect();
        Ok(SearchBody {
            query: q,
            workspaces,
            workspaces_total,
            chats,
            next_cursor: page.next_cursor,
        })
    })
    .await?;
    Ok(json(200, &body))
}
