//! Everything waiting on the person, each with an exact id (merged plan
//! slice 4 item 7, review finding B·F15).
//!
//! One backend query over every store that holds a decision only the user
//! can make: approval cards on screen ([`crate::approvals::pending_cards`]),
//! open question cards (`interactions`), the review inbox's pending items
//! (every kind the five review panels show), results marked ready for review
//! or changed since accepted, unread failure notifications (Activity's
//! "Needs you"), two-way sync conflicts with no choice yet, and ask-canvases
//! whose answer isn't sent (Canvas C4 A3, [`waiting_canvas`]). Each item
//! carries the id its own store knows it by (`source_id`) plus the exact
//! conversation, workspace or file to open: nothing is resolved through a
//! title, a recency guess or the rail's capped lists.
//!
//! The list is paged, never capped: `total` counts everything waiting and
//! `next_cursor` continues where a page ended. The cursor is a keyset
//! ([`crate::page_cursor`]): it names the last item's `(at, id)`, so an item
//! resolving or arriving between two page reads never makes the next page
//! skip or repeat one. A source that cannot be read
//! is named in `unavailable`, so a partial list never reads as complete.
//! The single "waiting on you" surface (wave 3, decision 7: the top-bar bell,
//! `src/components/WaitingOnYouPanel.tsx`) reads it, every kind, paged.

use crate::approvals::PendingCard;
use crate::page_cursor::{newest_first, Keyset};
use crate::workspace_canvases::AskHead;
use crate::{interactions, review_queue, session_store, workspaces};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

/// The kind of the one "Resume N interrupted chats" item (overnight A1).
pub(crate) const INTERRUPTED_KIND: &str = "interrupted";
/// The kind of the one "N conflict copies in the vault" item (overnight A10).
pub(crate) const COPIES_KIND: &str = "copies";
/// The kind of one context sync clash card (CX6c), one item per file.
pub(crate) const CLASH_KIND: &str = "clash";

/// Items per page when the caller does not say.
const PAGE_DEFAULT: usize = 50;
/// Most items one page carries.
const PAGE_MAX: usize = 200;

/// The review inbox kinds the waiting-on-you surface shows, with their
/// labels. Mirrors `WORKSPACE_REVIEW_KINDS` in `src/workspaceContent.ts`
/// (lockstep test `review_kinds_match_the_frontend`).
const REVIEW_KINDS: [(&str, &str); 17] = [
    ("learning", "Learning"),
    ("commitment", "Commitment"),
    ("project_activation", "Project brief"),
    ("goal", "Goal"),
    ("heartbeat_task", "Suggested task"),
    ("meeting_task", "Meeting task"),
    ("workflow_permissions", "Automation permissions"),
    ("workflow_failure_streak", "Failing automation"),
    ("budget_pause", "Budget pause"),
    ("agent_task", "Agent-proposed task"),
    ("gardener", "Tidy up"),
    ("record", "Decision or fact"),
    ("person_alias", "Who is this?"),
    ("companion_conflict", "Changed on two devices"),
    ("companion_old_op", "Old change from a device"),
    ("memory_recheck", "Recheck memory"),
    ("eval_candidate", "Eval candidate"),
];
/// Where a review item's title comes from, first present wins (the review
/// card's `itemTitle`, `src/components/ReviewCard.tsx`).
const REVIEW_TITLE_KEYS: [&str; 4] = ["subject", "title", "projectName", "text"];
/// Mirrors `WORKSPACE_REVIEW_SOURCES` in `src/workspaceContent.ts`.
const REVIEW_SOURCES: [&str; 10] = [
    "activation",
    "heartbeat",
    "meeting",
    "chat",
    "agent",
    "workflow",
    "budget",
    "gardener",
    "companion",
    "memory",
];

/// One thing waiting on the person.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NeedsYouItem {
    /// Unique across kinds: `<kind>:<source id>` (a conflict adds `:<field>`;
    /// a canvas is `canvas:<session key>/<source id>`).
    pub id: String,
    /// `approval` | `question` | `review` | `result` | `failure` | `conflict`
    /// | `canvas` (an ask-canvas, Canvas C4 A3) | `interrupted` (this
    /// launch's "Resume N interrupted chats", overnight A1) | `copies` (the
    /// vault's conflict copies, overnight A10) | `clash` (a context sync
    /// clash card, CX6c: `sourceId` is `<root>:<path>`) | `lesson_memory`
    /// ("Delete the memory X?", build run L2; `category` is its HOL issue key).
    #[cfg_attr(test, ts(as = "crate::ts_bindings::NeedsYouKind"))]
    pub kind: String,
    /// The id the item's own store knows it by: the approval id, the
    /// question card id, the review item id, the result's vault path, the
    /// notification id, the tracker record id; for an ask-canvas
    /// `<canvas id>@<revision>`, the key its answer locks (`answerKey`).
    pub source_id: String,
    pub title: String,
    pub detail: Option<String>,
    /// `pending` (an approval, a question, an ask-canvas) | `stale` (a
    /// question that can no longer be answered) |
    /// `applying` (a review item) | `ready` | `changed` (a result) | `unread`
    /// (a failure) | `conflict`.
    pub state: String,
    /// When it started waiting (RFC 3339), when known.
    pub at: Option<String>,
    pub workspace_id: Option<String>,
    /// The exact conversation it belongs to.
    pub session_key: Option<String>,
    /// The vault-relative file it is about (an ask-canvas in a workspace:
    /// its visible copy, `outputs/canvases/<id>.canvas.html`).
    pub path: Option<String>,
    /// Approval kind, question mode, review kind, result kind, notification
    /// source or sync record kind.
    pub category: Option<String>,
    /// A failure notification's own target (`vault:…`, `workflow:<slug>`,
    /// `settings:<section>`, `https://…`).
    pub link: Option<String>,
}

/// One page of [`NeedsYouItem`]s.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct NeedsYouPage {
    pub items: Vec<NeedsYouItem>,
    /// Everything waiting now, across all pages.
    pub total: usize,
    /// Pass back as `cursor` for the next page; `None` on the last page.
    /// Opaque: it names the last item's `(at, id)`, not a position.
    pub next_cursor: Option<String>,
    /// Sources that could not be read; their items are missing from `total`.
    pub unavailable: Vec<String>,
}

fn item(kind: &str, source_id: &str, title: String, state: &str) -> NeedsYouItem {
    NeedsYouItem {
        id: format!("{kind}:{source_id}"),
        kind: kind.into(),
        source_id: source_id.into(),
        title,
        detail: None,
        state: state.into(),
        at: None,
        workspace_id: None,
        session_key: None,
        path: None,
        category: None,
        link: None,
    }
}

/// A conversation's workspace by its exact key, looked up once per key.
struct Homes<'a> {
    root: &'a Path,
    dirs: Option<workspaces::SessionHomes>,
    by_key: HashMap<String, Option<String>>,
}

impl Homes<'_> {
    fn workspace_of(&mut self, key: &str) -> Option<String> {
        if let Some(found) = self.by_key.get(key) {
            return found.clone();
        }
        let cwd = session_store::session_by_key(self.root, key)
            .ok()
            .flatten()
            .map(|s| s.cwd);
        let found = match (&self.dirs, cwd) {
            (Some(dirs), Some(cwd)) => dirs.workspace_for(&cwd),
            _ => None,
        };
        self.by_key.insert(key.to_string(), found.clone());
        found
    }
}

/// Each conversation's stored messages that may answer an ask-canvas, read
/// once per conversation per query.
#[derive(Default)]
struct Asks {
    by_key: HashMap<String, Option<session_store::SessionAsks>>,
    /// A conversation couldn't be read: "conversations" is unavailable.
    failed: bool,
}

/// An ask-canvas still waiting on the person (Canvas C4 A3), or `None`.
/// `home` is the workspace whose store holds it (`None`: the conversation's
/// own app-data store); `file` its visible copy. It waits while the
/// conversation that emitted its current revision exists, isn't archived,
/// still lives in that store's workspace (the chat reads a conversation's
/// canvases from its own workspace's store, so one moved elsewhere shows
/// this canvas unsaved and never asks), and has sent no message answering
/// that revision ([`crate::canvas_answers::answered`], the chat's lock rule).
/// Its exact target is that conversation and that canvas revision.
fn waiting_canvas(
    root: &Path,
    homes: &Homes,
    asks: &mut Asks,
    head: AskHead,
    home: Option<&str>,
    file: Option<String>,
) -> Option<NeedsYouItem> {
    let key = head.session.clone()?;
    if !asks.by_key.contains_key(&key) {
        let read = session_store::session_asks(root, &key, crate::next_step::CANVAS_ANSWER_LANG);
        if read.is_err() {
            asks.failed = true;
        }
        asks.by_key.insert(key.clone(), read.ok().flatten());
    }
    let session = asks.by_key.get(&key)?.as_ref()?;
    let lives = homes.dirs.as_ref()?.workspace_for(&session.cwd);
    if session.archived || lives.as_deref() != home {
        return None;
    }
    let answer = (head.id.clone(), head.revision);
    if session
        .asked
        .iter()
        .any(|text| crate::canvas_answers::answered(text).contains(&answer))
    {
        return None;
    }
    let source = format!("{}@{}", head.id, head.revision);
    let mut it = item("canvas", &source, head.title, "pending");
    it.id = format!("canvas:{key}/{source}");
    it.at = Some(head.saved_at);
    it.workspace_id = home.map(str::to_string);
    it.session_key = Some(key);
    it.path = file;
    Some(it)
}

/// Legacy message routes never surface (mirrors `isRetiredMessageSource` in
/// `src/workspaceContent.ts`, which Activity applies to notifications).
fn retired_message_source(source: &str) -> bool {
    let value = source.replace('\\', "/").to_ascii_lowercase();
    let value = value.strip_prefix("vault:").unwrap_or(&value);
    let head = value.strip_prefix("page:").unwrap_or(value);
    for word in ["email", "emails", "teams", "slack", "inbox"] {
        let rest = head.strip_prefix(word);
        if rest.is_some_and(|r| r.is_empty() || r.starts_with(['-', '/', ':'])) {
            return true;
        }
    }
    let folder = |seg: &str| matches!(seg, "emails" | "teams" | "slack");
    value.split('/').any(folder) || value.starts_with("mailto:")
}

/// A review kind's label, when a review panel shows that kind.
fn review_label(kind: &str) -> Option<&'static str> {
    REVIEW_KINDS
        .iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, label)| *label)
}

/// A notification's `vault:` target as a vault-relative path.
fn vault_target(link: Option<&str>) -> Option<String> {
    Some(link?.strip_prefix("vault:")?.to_string())
}

fn first_text(payload: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|k| payload.get(*k).and_then(|v| v.as_str()))
        .map(str::trim)
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

/// Everything waiting now of the asked-for `kinds` (every kind when
/// `None`), unpaged, newest first. A source no asked-for kind comes from is
/// never read, so it is never named in `unavailable` either.
fn collect(
    root: &Path,
    app_data: Option<&Path>,
    approvals: Vec<PendingCard>,
    kinds: Option<&[String]>,
) -> (Vec<NeedsYouItem>, Vec<String>) {
    let wants = |kind: &str| kinds.is_none_or(|kinds| kinds.iter().any(|k| k == kind));
    let mut out = Vec::new();
    let mut unavailable = Vec::new();
    // Only approvals, questions and ask-canvases look up a conversation's
    // workspace.
    let dirs = if wants("approval") || wants("question") || wants("canvas") {
        match workspaces::SessionHomes::read(root) {
            Ok(dirs) => Some(dirs),
            Err(_) => {
                unavailable.push("workspaces".to_string());
                None
            }
        }
    } else {
        None
    };
    let mut homes = Homes {
        root,
        dirs,
        by_key: HashMap::new(),
    };

    for card in approvals.into_iter().filter(|_| wants("approval")) {
        let mut it = item("approval", &card.id, card.action, "pending");
        it.detail = Some(card.title).filter(|t| !t.trim().is_empty());
        it.at = Some(card.at);
        it.category = Some(card.kind);
        if let Some(key) = card.session_key.as_deref() {
            it.workspace_id = homes.workspace_of(key);
        }
        it.session_key = card.session_key;
        out.push(it);
    }

    match wants("question").then(|| interactions::list_open_every(root)) {
        None => {}
        Some(Ok(cards)) => {
            for card in cards {
                let fallback = if card.mode == "confirmation" {
                    "A confirmation"
                } else {
                    "A question"
                };
                let title =
                    first_text(&card.request, &["title"]).unwrap_or_else(|| fallback.to_string());
                let mut it = item("question", &card.id, title, &card.status);
                it.detail = card.status_reason;
                it.at = Some(card.created_at);
                it.category = Some(card.mode);
                it.path = card.target_path;
                it.workspace_id = homes.workspace_of(&card.session_key);
                it.session_key = Some(card.session_key);
                out.push(it);
            }
        }
        Some(Err(_)) => unavailable.push("questions".into()),
    }

    match wants("review").then(|| review_queue::list(root, None)) {
        None => {}
        Some(Ok(items)) => {
            for review in items {
                let Some(label) = review_label(&review.kind) else {
                    continue;
                };
                // A pending learning is listed and counted like any review
                // item; its row opens Library → Learnings, where it is
                // decided (the bell is its signal, not a second home).
                if !matches!(review.status.as_str(), "pending" | "applying")
                    || !REVIEW_SOURCES.contains(&review.source.as_str())
                    || review.risk != "local_write"
                    || review
                        .source_path
                        .as_deref()
                        .is_some_and(retired_message_source)
                {
                    continue;
                }
                let title = first_text(&review.payload, &REVIEW_TITLE_KEYS)
                    .unwrap_or_else(|| label.to_string());
                let mut it = item("review", &review.id, title, &review.status);
                it.detail = Some(label.to_string());
                it.at = Some(review.created_at);
                it.category = Some(review.kind);
                it.path = review.source_path;
                out.push(it);
            }
        }
        Some(Err(_)) => unavailable.push("review inbox".into()),
    }

    match wants("result").then(|| workspaces::list(root)) {
        None => {}
        Some(Ok(all)) => {
            for w in all {
                match crate::workspace_results::awaiting_review(root, &w) {
                    Ok(results) => {
                        for r in results {
                            let mut it = item("result", &r.path, r.title, &r.review);
                            it.detail = Some(w.title.clone());
                            it.at = r.since;
                            it.category = Some(r.kind);
                            it.workspace_id = Some(w.id.clone());
                            it.path = Some(r.path);
                            out.push(it);
                        }
                    }
                    Err(_) => unavailable.push(format!("results in {}", w.title)),
                }
            }
        }
        Some(Err(_)) => unavailable.push("results".into()),
    }

    let failures = match wants("failure").then(|| crate::notifications::unread_errors(root)) {
        None => Vec::new(),
        Some(Ok(failures)) => failures,
        Some(Err(_)) => {
            unavailable.push("failures".into());
            Vec::new()
        }
    };
    for n in failures {
        if n.source.as_deref().is_some_and(retired_message_source)
            || n.action_url.as_deref().is_some_and(retired_message_source)
        {
            continue;
        }
        let mut it = item("failure", &n.id, n.title, "unread");
        it.detail = Some(n.body).filter(|b| !b.trim().is_empty());
        it.at = Some(n.created);
        it.path = vault_target(n.action_url.as_deref());
        it.category = n.source;
        it.link = n.action_url;
        out.push(it);
    }

    match wants("conflict").then(|| crate::tracker_sync::open_conflicts(root)) {
        None => {}
        Some(Ok(conflicts)) => {
            for c in conflicts {
                let source = format!("tracker:{}:{}", c.record_id, c.field);
                let mut it = item("conflict", &source, c.title, "conflict");
                it.source_id = c.record_id;
                it.detail = Some(format!(
                    "The tracker kept its value for {}; your edit is noted in the task.",
                    c.field
                ));
                it.at = Some(c.at);
                it.category = Some(c.kind);
                it.workspace_id = c.workspace_id;
                it.path = Some(c.path);
                out.push(it);
            }
        }
        Some(Err(_)) => unavailable.push("Tracker sync".into()),
    }

    if wants("canvas") {
        let mut asks = Asks::default();
        match workspaces::list(root) {
            Ok(all) => {
                for w in all {
                    let heads = workspaces::folder(root, &w.id)
                        .and_then(|folder| crate::workspace_canvases::workspace_ask_heads(&folder));
                    match heads {
                        Ok(heads) => {
                            for head in heads {
                                let file =
                                    format!("{}/outputs/canvases/{}.canvas.html", w.path, head.id);
                                let home = Some(w.id.as_str());
                                if let Some(it) =
                                    waiting_canvas(root, &homes, &mut asks, head, home, Some(file))
                                {
                                    out.push(it);
                                }
                            }
                        }
                        Err(_) => unavailable.push(format!("canvases in {}", w.title)),
                    }
                }
            }
            Err(_) => unavailable.push("canvases".into()),
        }
        // Conversations outside every workspace keep theirs in app data.
        match app_data.map(crate::workspace_canvases::app_data_ask_heads) {
            None => {}
            Some(Ok(stores)) => {
                for (key, heads) in stores {
                    for head in heads
                        .into_iter()
                        .filter(|h| h.session.as_deref() == Some(key.as_str()))
                    {
                        if let Some(it) = waiting_canvas(root, &homes, &mut asks, head, None, None)
                        {
                            out.push(it);
                        }
                    }
                }
            }
            Some(Err(_)) => unavailable.push("canvases outside workspaces".into()),
        }
        if asks.failed {
            unavailable.push("conversations".into());
        }
    }

    // This launch's offer to resume the chats its quit interrupted
    // (overnight A1): one item, whatever the count.
    match wants(INTERRUPTED_KIND).then(|| crate::restart_resume::waiting(root, chrono::Utc::now()))
    {
        None | Some(Ok(None)) => {}
        Some(Ok(Some((source_id, title, detail, at)))) => {
            let mut it = item(INTERRUPTED_KIND, &source_id, title, "pending");
            it.detail = Some(detail);
            it.at = Some(at);
            out.push(it);
        }
        Some(Err(_)) => unavailable.push("interrupted chats".into()),
    }

    // Context sync clashes (CX6c): one card per file, every root, under the
    // engine's `context-sync` source. Read from the roots' state files only;
    // no vault file is read here.
    if wants(CLASH_KIND) {
        match crate::sync_engine::clash_list(root, false) {
            Ok(list) => {
                for c in list {
                    let name = c.path.rsplit('/').next().unwrap_or(&c.path).to_string();
                    let source = format!("{}:{}", c.root, c.path);
                    let state = if c.editing { "editing" } else { "pending" };
                    let mut it = item(
                        CLASH_KIND,
                        &source,
                        format!("Two Macs changed {name}"),
                        state,
                    );
                    it.detail = Some(format!(
                        "This Mac kept its version of {}; {}'s is beside it as {}.",
                        c.path, c.theirs_machine, c.copy
                    ));
                    it.at = Some(c.at);
                    it.path = Some(c.path);
                    it.category = Some("context-sync".into());
                    out.push(it);
                }
            }
            Err(_) => unavailable.push("context sync clashes".into()),
        }
    }

    // iCloud/Finder conflict copies in the vault (overnight A10): one item,
    // whatever the count; the person keeps one of each pair. The scan never
    // reads the files here (this query runs often). A context sync clash's
    // copy is its own card above, and one the person kept (Keep both, Done)
    // is never offered again.
    if wants(COPIES_KIND) {
        let skip = crate::sync_engine::clash_skip_copies(root).unwrap_or_default();
        if let Some((title, detail)) = crate::conflict_copies::waiting(root, &skip) {
            let mut it = item(COPIES_KIND, "vault", title, "pending");
            it.detail = Some(detail);
            out.push(it);
        }
    }

    // "Delete the memory X?" (build run L2): a lesson's fix is in the
    // installed build and its workaround memory is still there. At most
    // `lesson_memories::OFFER_CAP` at once; never copied to Linear.
    match wants(crate::lesson_memories::KIND).then(|| crate::lesson_memories::waiting(root)) {
        None => {}
        Some(Ok(offers)) => {
            for o in offers {
                let mut it = item(
                    crate::lesson_memories::KIND,
                    &o.source_id,
                    o.title,
                    "pending",
                );
                it.detail = Some(o.detail);
                it.at = Some(o.at).filter(|at| !at.is_empty());
                it.category = Some(o.issue);
                it.workspace_id = o.workspace_id;
                out.push(it);
            }
        }
        Some(Err(_)) => unavailable.push("lesson memories".into()),
    }

    // Newest first; an item with no time sorts last; ties by id, so a page
    // boundary is stable between two reads of the same state.
    out.sort_by(order);
    (out, unavailable)
}

/// Everything of `kinds` waiting now, unpaged, with the sources that could
/// not be read (T3b's Linear copies; no approval cards: those are on screen
/// for minutes only).
pub(crate) fn waiting_now(
    root: &Path,
    app_data: Option<&Path>,
    kinds: &[String],
) -> (Vec<NeedsYouItem>, Vec<String>) {
    collect(root, app_data, Vec::new(), Some(kinds))
}

/// The list's order: newest first, an item with no time last, ties by id.
fn order(a: &NeedsYouItem, b: &NeedsYouItem) -> std::cmp::Ordering {
    newest_first(a.at.as_deref(), &a.id, b.at.as_deref(), &b.id)
}

/// One page of everything waiting on the person: the items strictly after
/// `cursor` (a previous page's `next_cursor`, naming its last item's
/// `(at, id)`) in the list's order, at most `limit` (default 50, at most
/// 200). `kinds`, when given, keeps only those kinds, before paging, so
/// `total` counts the same list; sources no asked-for kind comes from are
/// not read and never named in `unavailable`. `approvals` are the cards on screen now.
/// A cursor this query did not give out is an error, never a silent restart.
/// Ask-canvases are read from workspaces only here; [`list_matching`] with
/// the app data folder adds those of conversations outside every workspace.
pub fn list(
    root: &Path,
    approvals: Vec<PendingCard>,
    kinds: Option<&[String]>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<NeedsYouPage, String> {
    list_matching(root, None, approvals, kinds, None, cursor, limit)
}

/// `list`, also keeping only items whose `category` is one of `categories`
/// (a review item's kind, say), before paging like `kinds`: the waiting-on-you
/// surface filtered to one page's review kinds asks for exactly those, so a
/// page of other review items can never read as "none of these waiting".
/// `app_data` is the app data folder, whose canvas store holds the
/// ask-canvases of conversations outside every workspace.
pub fn list_matching(
    root: &Path,
    app_data: Option<&Path>,
    approvals: Vec<PendingCard>,
    kinds: Option<&[String]>,
    categories: Option<&[String]>,
    cursor: Option<&str>,
    limit: Option<usize>,
) -> Result<NeedsYouPage, String> {
    let after = cursor.map(Keyset::decode).transpose()?;
    let (mut all, unavailable) = collect(root, app_data, approvals, kinds);
    if let Some(categories) = categories {
        all.retain(|item| {
            item.category
                .as_ref()
                .is_some_and(|c| categories.contains(c))
        });
    }
    let total = all.len();
    let limit = limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    // Everything strictly after the cursor's key. The key need not still be
    // in the list (its item may have resolved since), so this compares by
    // order rather than looking the item up.
    let start = match &after {
        Some(key) => all.partition_point(|item| {
            key.order(&Keyset::new(item.at.as_deref(), &item.id)) != std::cmp::Ordering::Less
        }),
        None => 0,
    };
    let rest = all.len() - start;
    let items: Vec<NeedsYouItem> = all.into_iter().skip(start).take(limit).collect();
    let next_cursor = (rest > items.len())
        .then(|| items.last())
        .flatten()
        .map(|last| Keyset::new(last.at.as_deref(), &last.id).encode());
    Ok(NeedsYouPage {
        items,
        total,
        next_cursor,
        unavailable,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_learnings_stay_listed_so_the_bell_counts_them() {
        let root = vault();
        let learning = propose(root.path(), "learning", 'a', "A grounded lesson");
        let page = list(root.path(), vec![], None, None, None).unwrap();
        let row = page
            .items
            .iter()
            .find(|item| item.source_id == learning.id)
            .expect("a pending learning is listed");
        assert_eq!(row.kind, "review");
        assert_eq!(row.category.as_deref(), Some("learning"));
        assert_eq!(row.title, "A grounded lesson");
    }

    fn vault() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vault-").tempdir().unwrap()
    }

    fn propose(root: &Path, kind: &str, fp: char, title: &str) -> review_queue::ReviewItem {
        review_queue::enqueue(
            root,
            review_queue::NewReviewItem {
                kind: kind.into(),
                fingerprint: fp.to_string().repeat(64),
                payload: json!({ "title": title }),
                risk: "local_write".into(),
                source: "chat".into(),
                source_path: None,
            },
        )
        .unwrap()
    }

    fn of_kind<'a>(page: &'a NeedsYouPage, kind: &str) -> Vec<&'a NeedsYouItem> {
        page.items.iter().filter(|i| i.kind == kind).collect()
    }

    /// Every page, followed through `next_cursor`.
    fn every_page(root: &Path, approvals: &[PendingCard], limit: usize) -> Vec<NeedsYouItem> {
        let mut out = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = list(
                root,
                approvals.to_vec(),
                None,
                cursor.as_deref(),
                Some(limit),
            )
            .unwrap();
            out.extend(page.items);
            match page.next_cursor {
                Some(next) => cursor = Some(next),
                None => return out,
            }
        }
    }

    /// B·F15: more than 25 conversations' worth of waiting items, in one
    /// workspace, are all reachable through pages, each with its exact
    /// conversation and workspace; nothing is capped.
    #[test]
    fn every_waiting_item_is_reachable_beyond_twenty_five() {
        let root = vault();
        let w = workspaces::create(root.path(), "Busy", "Many asks").unwrap();
        let mut approvals = Vec::new();
        for i in 0..40 {
            let key = format!("s-20261002-needs{i:03}");
            session_store::start_turn(root.path(), &key, "claude", &w.cwd, "run-n", "Go").unwrap();
            approvals.push(PendingCard {
                id: format!("apr-{}", 1000 + i),
                title: "Computer control".into(),
                action: format!("Quit app {i}"),
                kind: "computer".into(),
                session_key: Some(key),
                at: format!("2026-10-02T10:{i:02}:00Z"),
                ..Default::default()
            });
        }
        // The rail lists a workspace's 25 newest conversations; the oldest
        // ones are exactly what a lookup through it used to miss.
        let rail = workspaces::sessions_by_workspace(root.path()).unwrap();
        let listed = rail
            .iter()
            .flat_map(|g| &g.sessions)
            .any(|s| s.key == "s-20261002-needs000");
        assert!(!listed, "the rail leaves the oldest conversation out");

        let first = list(root.path(), approvals.clone(), None, None, Some(25)).unwrap();
        assert_eq!(first.total, 40);
        assert_eq!(first.items.len(), 25);
        assert!(first.next_cursor.is_some(), "a second page follows");
        assert!(first.unavailable.is_empty(), "{:?}", first.unavailable);

        let all = every_page(root.path(), &approvals, 25);
        assert_eq!(all.len(), 40, "every item, across pages");
        let ids: std::collections::BTreeSet<_> = all.iter().map(|i| i.id.clone()).collect();
        assert_eq!(ids.len(), 40, "no item twice");
        // Newest first; the oldest (the one a cap of 25 dropped) is on the last page.
        assert_eq!(all[0].source_id, "apr-1039");
        let oldest = all.last().unwrap();
        assert_eq!(oldest.id, "approval:apr-1000");
        assert_eq!(oldest.session_key.as_deref(), Some("s-20261002-needs000"));
        assert_eq!(oldest.workspace_id.as_deref(), Some(w.id.as_str()));
        assert_eq!(oldest.title, "Quit app 0");

        // A cursor past the last item is an empty last page, not an error;
        // a cursor the query never gave out (an old offset) is refused.
        let end = Keyset::new(Some("2000-01-01T00:00:00Z"), "approval:zzz").encode();
        let past = list(root.path(), approvals.clone(), None, Some(&end), None).unwrap();
        assert!(past.items.is_empty());
        assert_eq!((past.total, past.next_cursor), (40, None));
        assert!(list(root.path(), approvals, None, Some("25"), None).is_err());
    }

    fn card(i: usize) -> PendingCard {
        PendingCard {
            id: format!("apr-{}", 1000 + i),
            title: "Computer control".into(),
            action: format!("Quit app {i}"),
            kind: "computer".into(),
            session_key: None,
            at: format!("2026-10-02T10:{i:02}:00Z"),
            ..Default::default()
        }
    }

    /// B·F15 review: the cursor names the last item's key, not a position.
    /// An item on page 1 resolving before page 2 is read must not shift the
    /// first unread item off the boundary (an offset cursor skipped it), and
    /// an item arriving must not repeat the boundary item.
    #[test]
    fn resolving_or_adding_an_item_between_pages_skips_and_repeats_nothing() {
        let root = vault();
        let mut cards: Vec<PendingCard> = (0..10).map(card).collect();
        let first = list(root.path(), cards.clone(), None, None, Some(4)).unwrap();
        let seen: Vec<String> = first.items.iter().map(|i| i.source_id.clone()).collect();
        assert_eq!(seen, ["apr-1009", "apr-1008", "apr-1007", "apr-1006"]);
        let cursor = first.next_cursor.unwrap();

        // The person answers one approval they saw on page 1.
        cards.retain(|c| c.id != "apr-1008");
        let second = list(root.path(), cards.clone(), None, Some(&cursor), Some(4)).unwrap();
        let next: Vec<&str> = second.items.iter().map(|i| i.source_id.as_str()).collect();
        assert_eq!(next, ["apr-1005", "apr-1004", "apr-1003", "apr-1002"]);
        assert_eq!(second.total, 9);

        // A new approval arrives (newest of all) before page 3.
        cards.push(card(30));
        let third = list(
            root.path(),
            cards,
            None,
            second.next_cursor.as_deref(),
            Some(4),
        )
        .unwrap();
        let last: Vec<&str> = third.items.iter().map(|i| i.source_id.as_str()).collect();
        assert_eq!(last, ["apr-1001", "apr-1000"], "nothing older was skipped");
        assert_eq!(third.next_cursor, None);
    }

    /// Every source lands in the one list, each item naming its own exact
    /// target: the result's path (not the workspace's latest result), the
    /// question's conversation, the review item's id.
    #[test]
    fn each_source_contributes_items_with_exact_targets() {
        let root = vault();
        let w = workspaces::create(root.path(), "Report", "Write it").unwrap();
        let outputs = Path::new(&w.cwd).join("outputs");
        std::fs::create_dir_all(&outputs).unwrap();
        std::fs::write(outputs.join("ready.md"), "# Ready one\n").unwrap();
        std::fs::write(outputs.join("newer.md"), "# Newer draft\n").unwrap();
        let ready_rel = "outputs/ready.md";
        crate::workspace_results::set_review(
            root.path(),
            &w.id,
            ready_rel,
            "ready",
            None,
            None,
            &crate::workspace_results::Reviewer::Person,
        )
        .unwrap();

        let key = "s-20261002-question1";
        let ord =
            session_store::start_turn(root.path(), key, "claude", &w.cwd, "run-q", "Ask").unwrap();
        let mut request = crate::interaction_schema::tests::layout_request();
        request["title"] = json!("Which layout?");
        let arguments: Result<serde_json::Value, String> = Ok(request);
        interactions::record(
            root.path(),
            &interactions::NewInteraction {
                session_key: key,
                turn_ord: ord,
                run_id: "run-q",
                agent: crate::agent::AgentKind::Claude,
                native_session_id: Some("native-q"),
                tool_use_id: "toolu-q",
                source: interactions::InteractionSource::Mcp,
                arguments: &arguments,
                cwd: Path::new(&w.cwd),
                vault_root: root.path(),
            },
        )
        .unwrap();

        let review = propose(root.path(), "commitment", 'f', "Prefers tables");

        let unread = json!({ "id": "n-fail", "title": "Weekly digest failed", "body": "Exit 1",
            "kind": "error", "source": "weekly-digest", "actionUrl": "workflow:weekly-digest",
            "created": "2026-10-02T09:00:00Z", "read": false });
        let read = json!({ "id": "n-old", "title": "Old failure", "body": "", "kind": "error",
            "created": "2026-10-01T09:00:00Z", "read": true });
        let retired = json!({ "id": "n-mail", "title": "Reply failed", "body": "", "kind": "error",
            "source": "emails/inbox", "created": "2026-10-02T09:30:00Z", "read": false });
        std::fs::write(
            root.path().join(".notifications.jsonl"),
            format!("{read}\n{unread}\n{retired}\n"),
        )
        .unwrap();

        let page = list(root.path(), vec![], None, None, None).unwrap();
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);
        let results = of_kind(&page, "result");
        assert_eq!(results.len(), 1, "only the result marked ready waits");
        let ready = format!("{}/outputs/ready.md", w.path);
        assert_eq!(results[0].source_id, ready);
        assert_eq!(results[0].path.as_deref(), Some(ready.as_str()));
        assert_eq!(results[0].workspace_id.as_deref(), Some(w.id.as_str()));
        assert_eq!(results[0].title, "Ready one");

        let questions = of_kind(&page, "question");
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].title, "Which layout?");
        assert_eq!(questions[0].session_key.as_deref(), Some(key));
        assert_eq!(questions[0].workspace_id.as_deref(), Some(w.id.as_str()));

        let reviews = of_kind(&page, "review");
        assert_eq!(reviews.len(), 1);
        assert_eq!(reviews[0].source_id, review.id);
        assert_eq!(reviews[0].title, "Prefers tables");
        assert_eq!(reviews[0].category.as_deref(), Some("commitment"));

        let failures = of_kind(&page, "failure");
        assert_eq!(
            failures.len(),
            1,
            "read and retired-message failures stay out"
        );
        assert_eq!(failures[0].source_id, "n-fail");
        assert_eq!(failures[0].link.as_deref(), Some("workflow:weekly-digest"));

        assert_eq!(page.total, 4);

        // A kinds filter applies before paging: its total is that list's.
        let only = ["result".to_string(), "failure".to_string()];
        let filtered = list(root.path(), vec![], Some(&only[..]), None, Some(1)).unwrap();
        assert_eq!(filtered.total, 2);
        assert!(filtered.next_cursor.is_some());
        assert_eq!(filtered.items.len(), 1);

        // A category filter applies before paging too: only commitment reviews.
        let reviews_only = ["review".to_string()];
        let learning = ["commitment".to_string()];
        let page = list_matching(
            root.path(),
            None,
            vec![],
            Some(&reviews_only[..]),
            Some(&learning[..]),
            None,
            Some(1),
        )
        .unwrap();
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].source_id, review.id);
        assert!(page.next_cursor.is_none());
        let none = ["budget_pause".to_string()];
        let page = list_matching(
            root.path(),
            None,
            vec![],
            Some(&reviews_only[..]),
            Some(&none[..]),
            None,
            None,
        )
        .unwrap();
        assert_eq!(page.total, 0);
        assert!(page.items.is_empty());
    }

    /// A notifications file that exists but can't be read (here: bytes that
    /// aren't UTF-8) names "failures" as unavailable rather than reading as
    /// "no failures waiting"; a missing file is simply none.
    #[test]
    fn unreadable_failures_file_is_named_unavailable() {
        let root = vault();
        let page = list(root.path(), vec![], None, None, None).unwrap();
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);

        let unread = json!({ "id": "n-fail", "title": "Digest failed", "body": "",
            "kind": "error", "created": "2026-10-02T09:00:00Z", "read": false });
        let mut bytes = format!("{unread}\n").into_bytes();
        bytes.extend_from_slice(&[0xff, 0xfe, b'\n']);
        std::fs::write(root.path().join(".notifications.jsonl"), bytes).unwrap();

        let page = list(root.path(), vec![], None, None, None).unwrap();
        assert!(
            page.unavailable.iter().any(|u| u == "failures"),
            "{:?}",
            page.unavailable
        );
        assert!(of_kind(&page, "failure").is_empty());
    }

    /// A results.json that is corrupt or can't be read names its workspace in
    /// `unavailable` instead of reading as "no results waiting", and the
    /// read-only query writes nothing: no `results.corrupt-*` backup per run.
    #[test]
    fn unreadable_results_store_is_named_unavailable_and_writes_nothing() {
        let root = vault();
        let corrupt = workspaces::create(root.path(), "Corrupt", "x").unwrap();
        let unreadable = workspaces::create(root.path(), "Unreadable", "y").unwrap();
        let hollow = Path::new(&corrupt.cwd).join(".hollow");
        std::fs::create_dir_all(&hollow).unwrap();
        std::fs::write(hollow.join("results.json"), "{not json").unwrap();
        // A directory where the file should be: exists, but can't be read.
        std::fs::create_dir_all(Path::new(&unreadable.cwd).join(".hollow/results.json")).unwrap();

        for _ in 0..2 {
            let page = list(root.path(), vec![], None, None, None).unwrap();
            for title in ["Corrupt", "Unreadable"] {
                let want = format!("results in {title}");
                assert!(
                    page.unavailable.contains(&want),
                    "{want}: {:?}",
                    page.unavailable
                );
            }
        }
        let backups: Vec<_> = std::fs::read_dir(&hollow)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("results.corrupt-"))
            .collect();
        assert!(backups.is_empty(), "{backups:?}");
        assert_eq!(
            std::fs::read_to_string(hollow.join("results.json")).unwrap(),
            "{not json"
        );
    }

    /// Wave 2 review nit: a `kinds` filter reads only the sources those
    /// kinds come from, so a broken source nobody asked for is not named in
    /// `unavailable` (and a broken one that was asked for still is).
    #[test]
    fn unavailable_names_only_the_sources_asked_for() {
        let root = vault();
        let corrupt = workspaces::create(root.path(), "Corrupt", "x").unwrap();
        let hollow = Path::new(&corrupt.cwd).join(".hollow");
        std::fs::create_dir_all(&hollow).unwrap();
        std::fs::write(hollow.join("results.json"), "{not json").unwrap();
        std::fs::write(
            root.path().join(".notifications.jsonl"),
            [0xff, 0xfe, b'\n'],
        )
        .unwrap();
        let review = propose(root.path(), "goal", 'c', "Ship it");

        let everything = list(root.path(), vec![], None, None, None).unwrap();
        assert!(everything.unavailable.contains(&"failures".to_string()));
        assert!(everything
            .unavailable
            .contains(&"results in Corrupt".to_string()));

        let reviews = ["review".to_string()];
        let page = list(root.path(), vec![], Some(&reviews[..]), None, None).unwrap();
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);
        let ids: Vec<&str> = page.items.iter().map(|i| i.source_id.as_str()).collect();
        assert_eq!(ids, vec![review.id.as_str()]);

        let failures = ["failure".to_string(), "approval".to_string()];
        let page = list(root.path(), vec![card(1)], Some(&failures[..]), None, None).unwrap();
        assert_eq!(page.unavailable, vec!["failures".to_string()]);
        assert_eq!(page.total, 1, "the approval still lists");
    }

    const ASK: &str = "<hc-canvas id=\"release-choices\" title=\"Release choices\" ask>\n<hc-ask title=\"Release choices\">\n<hc-choice name=\"day\" label=\"Ship day\">\nFriday\nMonday\n</hc-choice>\n</hc-ask>\n</hc-canvas>";

    fn record_canvas(
        root: &Path,
        app: &Path,
        workspace: Option<&str>,
        key: &str,
        body: &str,
        now: &str,
    ) {
        let scope = crate::workspace_canvases::Scope {
            workspace_id: workspace,
            session_key: Some(key),
        };
        let seen = crate::workspace_canvases::read(root, app, scope, "release-choices")
            .unwrap()
            .map(|c| c.sha256);
        crate::workspace_canvases::record(root, app, scope, body, None, None, seen.as_deref(), now)
            .unwrap();
    }

    fn answer(rev: u64) -> String {
        format!("Answered canvas \"release-choices\":\n- Ship day: Friday\n\n```hollow-canvas-answer\n{{\"canvasId\":\"release-choices\",\"revision\":{rev},\"answers\":{{\"day\":\"Friday\"}}}}\n```")
    }

    fn canvases(root: &Path, app: Option<&Path>) -> NeedsYouPage {
        let only = ["canvas".to_string()];
        list_matching(root, app, vec![], Some(&only[..]), None, None, None).unwrap()
    }

    /// Canvas C4 A3: an ask-canvas waits on the person, with its exact
    /// conversation and revision, until a message answering that revision
    /// is sent; something else sent first, or an answer to another
    /// revision, leaves it waiting; a new ask revision is a new question and
    /// a plain revision asks nothing.
    #[test]
    fn an_ask_canvas_waits_until_its_answer_is_sent() {
        let root = vault();
        let app = vault();
        let w = workspaces::create(root.path(), "Release", "Ship it").unwrap();
        let key = "s-20261005-ask001";
        session_store::start_turn(
            root.path(),
            key,
            "claude",
            &w.cwd,
            "run-1",
            "Help me choose",
        )
        .unwrap();
        let t0 = "2026-10-05T09:00:00+00:00";
        record_canvas(root.path(), app.path(), Some(&w.id), key, ASK, t0);
        let plain = "<hc-canvas id=\"status\" title=\"Status\">\n<p>Fine</p>\n</hc-canvas>";
        let scope = crate::workspace_canvases::Scope {
            workspace_id: Some(&w.id),
            session_key: Some(key),
        };
        crate::workspace_canvases::record(
            root.path(),
            app.path(),
            scope,
            plain,
            None,
            None,
            None,
            t0,
        )
        .unwrap();

        let page = canvases(root.path(), Some(app.path()));
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);
        assert_eq!(page.total, 1, "the plain canvas asks nothing");
        let it = &page.items[0];
        assert_eq!(it.kind, "canvas");
        assert_eq!(it.id, format!("canvas:{key}/release-choices@1"));
        assert_eq!(it.source_id, "release-choices@1");
        assert_eq!(it.title, "Release choices");
        assert_eq!(it.state, "pending");
        assert_eq!(it.at.as_deref(), Some(t0));
        assert_eq!(it.session_key.as_deref(), Some(key));
        assert_eq!(it.workspace_id.as_deref(), Some(w.id.as_str()));
        let file = format!("{}/outputs/canvases/release-choices.canvas.html", w.path);
        assert_eq!(it.path.as_deref(), Some(file.as_str()));
        // Every kind together lists it too.
        let all = list(root.path(), vec![], None, None, None).unwrap();
        assert_eq!(of_kind(&all, "canvas").len(), 1);

        // Something else first, or an answer to another revision: still waiting.
        session_store::start_turn(
            root.path(),
            key,
            "claude",
            &w.cwd,
            "run-2",
            "Actually, wait.",
        )
        .unwrap();
        session_store::start_turn(root.path(), key, "claude", &w.cwd, "run-3", &answer(9)).unwrap();
        assert_eq!(canvases(root.path(), Some(app.path())).total, 1);

        // The person sends the answer: it leaves the list.
        session_store::start_turn(root.path(), key, "claude", &w.cwd, "run-4", &answer(1)).unwrap();
        assert_eq!(canvases(root.path(), Some(app.path())).total, 0);

        // The agent asks again (v2): a new question, until it is answered.
        let v2 = ASK.replace("Monday", "Monday\nTuesday");
        record_canvas(
            root.path(),
            app.path(),
            Some(&w.id),
            key,
            &v2,
            "2026-10-05T09:05:00+00:00",
        );
        let page = canvases(root.path(), Some(app.path()));
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].source_id, "release-choices@2");
        // A plain v3 asks nothing.
        let v3 = v2.replace(" ask>", ">");
        record_canvas(
            root.path(),
            app.path(),
            Some(&w.id),
            key,
            &v3,
            "2026-10-05T09:06:00+00:00",
        );
        assert_eq!(canvases(root.path(), Some(app.path())).total, 0);
    }

    /// An ask-canvas outside every workspace (the app-data store) waits too,
    /// but only when the query is given the app data folder. One whose
    /// conversation is archived, gone, or now lives in another workspace
    /// (the chat would render it unsaved there) never waits.
    #[test]
    fn ask_canvases_outside_workspaces_and_conversations_that_moved() {
        let root = vault();
        let app = vault();
        let a = workspaces::create(root.path(), "Alpha", "One").unwrap();
        let b = workspaces::create(root.path(), "Beta", "Two").unwrap();
        let t0 = "2026-10-05T09:00:00+00:00";

        // A vault-root chat: its canvases live in app data.
        let loose = "s-20261005-loose1";
        let cwd = root.path().to_string_lossy().to_string();
        session_store::start_turn(root.path(), loose, "claude", &cwd, "run-l", "Pick").unwrap();
        record_canvas(root.path(), app.path(), None, loose, ASK, t0);
        assert_eq!(
            canvases(root.path(), None).total,
            0,
            "no app data, no app-data canvases"
        );
        let page = canvases(root.path(), Some(app.path()));
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].session_key.as_deref(), Some(loose));
        assert_eq!(page.items[0].workspace_id, None);
        assert_eq!(page.items[0].path, None);

        // Recorded in Alpha by a conversation that now lives in Beta.
        let moved = "s-20261005-moved1";
        session_store::start_turn(root.path(), moved, "claude", &b.cwd, "run-m", "Pick").unwrap();
        record_canvas(root.path(), app.path(), Some(&a.id), moved, ASK, t0);
        // Recorded by a conversation the store doesn't know.
        record_canvas(
            root.path(),
            app.path(),
            Some(&b.id),
            "s-20261005-gone01",
            ASK,
            t0,
        );
        let page = canvases(root.path(), Some(app.path()));
        let keys: Vec<_> = page
            .items
            .iter()
            .filter_map(|i| i.session_key.as_deref())
            .collect();
        assert_eq!(keys, vec![loose]);

        // Archiving the conversation ends its waiting.
        session_store::archive_session(root.path(), loose, true).unwrap();
        assert_eq!(canvases(root.path(), Some(app.path())).total, 0);
    }

    /// A canvas folder that can't be read names its workspace unavailable;
    /// a query for other kinds never reads canvases at all.
    #[test]
    fn an_unreadable_canvas_folder_is_named_and_other_kinds_skip_canvases() {
        let root = vault();
        let w = workspaces::create(root.path(), "Linked", "x").unwrap();
        let outputs = Path::new(&w.cwd).join("outputs");
        std::fs::create_dir_all(&outputs).unwrap();
        let elsewhere = vault();
        std::os::unix::fs::symlink(elsewhere.path(), outputs.join("canvases")).unwrap();
        let page = canvases(root.path(), None);
        assert_eq!(page.unavailable, vec!["canvases in Linked".to_string()]);
        let questions = ["question".to_string()];
        let page = list(root.path(), vec![], Some(&questions[..]), None, None).unwrap();
        assert!(page.unavailable.is_empty(), "{:?}", page.unavailable);
    }

    /// A review kind no review panel shows, or a closed item, never waits.
    #[test]
    fn only_open_items_of_reviewed_kinds_wait() {
        let root = vault();
        let open = propose(root.path(), "goal", 'a', "Ship the beta");
        let closed = propose(root.path(), "commitment", 'b', "Send the deck");
        review_queue::reject(root.path(), &closed.id).unwrap();
        let page = list(root.path(), vec![], None, None, None).unwrap();
        let ids: Vec<&str> = page.items.iter().map(|i| i.source_id.as_str()).collect();
        assert_eq!(ids, vec![open.id.as_str()]);
    }

    #[test]
    fn retired_message_routes_match_the_frontend_rule() {
        for source in [
            "emails/inbox/a.md",
            "vault:Emails/a.md",
            "teams",
            "slack:general",
            "page:email-reply",
            "inbox-sync",
            "notes/slack/x.md",
            "mailto:a@b.c",
        ] {
            assert!(retired_message_source(source), "{source}");
        }
        for source in [
            "weekly-digest",
            "nudge",
            "workflow:emailer",
            "notes/inboxes.md",
        ] {
            assert!(!retired_message_source(source), "{source}");
        }
    }

    /// `REVIEW_KINDS` and `REVIEW_SOURCES` are the frontend's lists exactly.
    #[test]
    fn review_kinds_match_the_frontend() {
        let source = include_str!("../../src/workspaceContent.ts");
        let pair = regex::Regex::new(r"(\w+):\s*'([^']*)'").unwrap();
        let block = |name: &str| -> Vec<(String, String)> {
            let head = format!("export const {name} = {{");
            let start = source
                .find(head.as_str())
                .unwrap_or_else(|| panic!("workspaceContent.ts defines {name}"));
            let end = start + source[start..].find("} as const").expect("block end");
            pair.captures_iter(&source[start..end])
                .map(|c| (c[1].to_string(), c[2].to_string()))
                .collect()
        };
        let kinds: Vec<(String, String)> = REVIEW_KINDS
            .iter()
            .map(|(k, l)| (k.to_string(), l.to_string()))
            .collect();
        assert_eq!(block("WORKSPACE_REVIEW_KINDS"), kinds);
        let sources: Vec<String> = block("WORKSPACE_REVIEW_SOURCES")
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        assert_eq!(sources, REVIEW_SOURCES.map(str::to_string).to_vec());
    }
}
