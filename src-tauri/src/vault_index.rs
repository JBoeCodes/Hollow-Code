//! The semantic vault index — Hollow's "what does this file mean" layer.
//!
//! The vault is the single source of truth for every tab, but on disk it is
//! one flat folder world: durable human knowledge (`me/`, `domains/`,
//! `people/`, `daily/`), app-generated sync mirrors (`calendar/`, `emails/`,
//! `teams/`, `slack/`, `recordings/`), and generated artifacts (`briefings/`)
//! all sit side by side. The old Vault tab treated them with equal visual
//! weight, so a handful of high-signal notes drowned under hundreds of synced
//! substrate files.
//!
//! This module classifies each file — by **folder first, frontmatter second** —
//! into a [`VaultItem`] carrying:
//! - `kind`     — the semantic type (task, meeting, email, person, daily, …).
//! - `surface`  — where the item is *best* opened. This is the redesign's
//!   unifying primitive: a `task` opens the TaskModal, an `email`
//!   opens as a local vault note, everything else opens in the Vault
//!   editor. The UI routes on this instead of duplicating editors.
//! - `editable` — whether direct editing is safe (synced mirrors / generated
//!   artifacts are read-only).
//! - `hidden`   — whether the item should be hidden from the default browse
//!   experience (the high-volume mirrors), reachable via a lens.
//! - `trust`    — provenance: authored | synced | generated | imported.
//! - `drift`    — an optional cleanup reason (unfiled, missing frontmatter).
//!
//! The classifier is deliberately a *pure* function of `(rel, frontmatter)` so
//! it is exhaustively unit-tested below. The folder→kind core is also exposed
//! as [`kind_from_folder`] for the cheap paths (the wikilink graph) that only
//! know a node's top-level folder, not its frontmatter.

use crate::frontmatter;
use crate::vault::walk_notes;
use once_cell::sync::Lazy;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Mutex;

/// One classified vault file. `serde(camelCase)` to match the frontend mirror
/// in `src/types.ts`.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VaultItem {
    /// Vault-relative path, e.g. `daily/2026/2026-06-09.md`. The stable id.
    pub path: String,
    /// Display title — frontmatter `title`/`subject`/`conversation`, else the
    /// filename stem.
    pub title: String,
    /// Semantic kind (see module docs). One of the `KIND_*` strings.
    pub kind: String,
    /// Canonical surface to open this item on. One of: `tasks`, `projects`,
    /// `vault`.
    pub surface: String,
    /// Top-level folder (or `""` for a vault-root file).
    pub folder: String,
    /// True when the file is safe to edit directly in the Vault editor.
    pub editable: bool,
    /// True when the item should be hidden from the default browse lens (the
    /// high-volume synced mirrors + generated artifacts).
    pub hidden: bool,
    /// Provenance: `authored` | `synced` | `generated` | `imported`.
    pub trust: String,
    /// ISO-8601 mtime (mirrors `Note.mtime`).
    pub mtime: String,
    pub size: u64,
    /// A one-line body excerpt for the lens list preview (may be empty).
    pub excerpt: String,
    /// Set when the item is a cleanup candidate: the human-readable reason.
    /// `None` for healthy files.
    pub drift: Option<String>,
}

// Kind constants — single source of the kind vocabulary (mirrored in
// `src/vaultKinds.ts`).
pub const KIND_TASK: &str = "task";
pub const KIND_PROJECT: &str = "project";
pub const KIND_GOAL: &str = "goal";
pub const KIND_COMMITMENT: &str = "commitment";
pub const KIND_MEETING: &str = "meeting";
pub const KIND_RECORDING: &str = "recording";
pub const KIND_EMAIL: &str = "email";
pub const KIND_CHAT: &str = "chat";
pub const KIND_WORKFLOW: &str = "workflow";
pub const KIND_BRIEFING: &str = "briefing";
pub const KIND_PERSON: &str = "person";
pub const KIND_DOMAIN: &str = "domain";
pub const KIND_DAILY: &str = "daily";
pub const KIND_WEEKLY: &str = "weekly";
pub const KIND_NOTE: &str = "note";
pub const KIND_SOURCE: &str = "source";
pub const KIND_OPERATOR: &str = "operator";
pub const KIND_CAPTURE: &str = "capture";
pub const KIND_LEARNING: &str = "learning";
pub const KIND_UNFILED: &str = "unfiled";

/// The folder→kind core. Maps a **top-level** folder name to its semantic kind.
/// Returns `None` for an unrecognized folder so callers can fall back to
/// frontmatter or the unfiled bucket. Exposed for the graph, which only knows a
/// node's top-level folder.
pub fn kind_from_folder(folder: &str) -> Option<&'static str> {
    Some(match folder {
        "tasks" => KIND_TASK,
        "projects" => KIND_PROJECT,
        "goals" => KIND_GOAL,
        "commitments" => KIND_COMMITMENT,
        // The Outlook mirror and user-authored meeting notes are both meetings;
        // editability/visibility differ (handled in `classify`), kind is shared.
        "calendar" | "meetings" => KIND_MEETING,
        "recordings" => KIND_RECORDING,
        "emails" => KIND_EMAIL,
        "teams" | "slack" => KIND_CHAT,
        "workflows" => KIND_WORKFLOW,
        "briefings" => KIND_BRIEFING,
        "people" => KIND_PERSON,
        "domains" => KIND_DOMAIN,
        "daily" => KIND_DAILY,
        "weekly" => KIND_WEEKLY,
        "me" => KIND_OPERATOR,
        "notes" => KIND_NOTE,
        "sources" | "ingested" | "imported" => KIND_SOURCE,
        "captures" => KIND_CAPTURE,
        "learnings" => KIND_LEARNING,
        _ => return None,
    })
}

/// Canonical surface for a kind. The redesign's routing primitive: the UI opens
/// an item on this surface instead of always loading the raw markdown editor.
fn surface_for(kind: &str) -> &'static str {
    match kind {
        KIND_TASK => "tasks",
        KIND_PROJECT => "projects",
        KIND_EMAIL | KIND_CHAT => "vault",
        // meeting/recording/briefing/person/note/daily/… all read or edit in
        // the Vault pane itself (type-aware header + editor or sandboxed HTML).
        _ => "vault",
    }
}

/// The top-level folder of a vault-relative path (`""` for a root file).
pub fn top_folder(rel: &str) -> &str {
    match rel.find('/') {
        Some(i) => &rel[..i],
        None => "",
    }
}

/// Classify a vault file from its relative path and parsed frontmatter. Pure
/// and total — never reads the filesystem. `fm` is the last-wins scalar map
/// from [`frontmatter::parse_map`].
pub fn classify(rel: &str, fm: &BTreeMap<String, String>) -> VaultItem {
    let folder = top_folder(rel).to_string();

    // Folder is the primary signal; frontmatter is the tiebreaker for files
    // outside a recognized folder (e.g. a root-level note carrying `type:`).
    let kind: String = match kind_from_folder(&folder) {
        Some(k) => k.to_string(),
        None => kind_from_frontmatter(fm)
            .unwrap_or(KIND_UNFILED)
            .to_string(),
    };

    // Mirror folders are synced and hidden from the default browse; user
    // meeting notes (`meetings/`) are visible. Both `meeting` kinds stay
    // EDITABLE though — only a calendar note's *frontmatter* is synced from
    // Outlook (reconcile never touches the body), and the Agenda / Notes /
    // Action-items sections are exactly what the user (and Plaud enrichment)
    // fill in. Truly verbatim/generated kinds are the read-only ones.
    let is_calendar_mirror = folder == "calendar";
    let editable = !matches!(kind.as_str(), KIND_RECORDING | KIND_BRIEFING);
    let hidden = matches!(
        kind.as_str(),
        KIND_EMAIL | KIND_CHAT | KIND_RECORDING | KIND_BRIEFING
    ) || is_calendar_mirror;
    let trust = match kind.as_str() {
        KIND_MEETING if is_calendar_mirror => "synced",
        KIND_RECORDING => "synced",
        KIND_EMAIL | KIND_CHAT => "imported",
        KIND_BRIEFING => "generated",
        KIND_SOURCE => "imported",
        _ => "authored",
    }
    .to_string();

    let title = derive_title(rel, fm);
    let drift = drift_reason(&kind, &folder, fm);

    VaultItem {
        path: rel.to_string(),
        title,
        surface: surface_for(&kind).to_string(),
        kind,
        folder,
        editable,
        hidden,
        trust,
        mtime: String::new(),
        size: 0,
        excerpt: String::new(),
        drift,
    }
}

/// Secondary classification from frontmatter alone, for files outside a known
/// folder. `type: meeting` / `type: recording` / `type: note` / `type: content`
/// and the chat `service:` marker are honored.
fn kind_from_frontmatter(fm: &BTreeMap<String, String>) -> Option<&'static str> {
    if fm.contains_key("service") {
        return Some(KIND_CHAT);
    }
    match fm.get("type").map(|s| s.as_str()) {
        Some("meeting") => Some(KIND_MEETING),
        Some("goal") => Some(KIND_GOAL),
        Some("commitment") => Some(KIND_COMMITMENT),
        Some("learning") => Some(KIND_LEARNING),
        Some("recording") => Some(KIND_RECORDING),
        Some("note") => Some(KIND_NOTE),
        Some("content") => Some(KIND_SOURCE),
        _ => None,
    }
}

/// Best display title: per-kind frontmatter field, then the filename stem.
fn derive_title(rel: &str, fm: &BTreeMap<String, String>) -> String {
    for key in ["title", "subject", "conversation"] {
        if let Some(v) = fm.get(key) {
            let v = v.trim();
            if !v.is_empty() {
                return v.to_string();
            }
        }
    }
    Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| rel.to_string())
}

/// Flag cleanup candidates. Conservative: only the unambiguous cases so the
/// cleanup lens stays trustworthy.
fn drift_reason(kind: &str, folder: &str, fm: &BTreeMap<String, String>) -> Option<String> {
    if kind == KIND_UNFILED {
        return Some(if folder.is_empty() {
            "Sitting at the vault root — no home folder".to_string()
        } else {
            format!("In an unrecognized folder `{folder}/`")
        });
    }
    // Authored knowledge notes with no frontmatter at all read as drafts that
    // never got typed — worth surfacing for a quick title/frontmatter pass.
    let authored_knowledge = matches!(kind, KIND_NOTE | KIND_DOMAIN | KIND_PERSON | KIND_SOURCE);
    if authored_knowledge && fm.is_empty() {
        return Some("No frontmatter (untyped knowledge note)".to_string());
    }
    None
}

/// A short, single-line body excerpt for the lens list preview. Skips the H1,
/// blank lines, and standalone markdown-link lines; caps at ~140 chars.
fn body_excerpt(body: &str) -> String {
    for line in body.lines() {
        let t = line.trim();
        if t.is_empty() || t.starts_with('#') {
            continue;
        }
        // Skip a bare image/link line.
        if (t.starts_with('[') || t.starts_with('!')) && t.contains("](") {
            continue;
        }
        let clean = t.trim_start_matches(['-', '*', '>', ' ']).trim();
        if clean.is_empty() {
            continue;
        }
        if clean.chars().count() > 140 {
            let cut: String = clean.chars().take(140).collect();
            return format!("{cut}…");
        }
        return clean.to_string();
    }
    String::new()
}

/// Per-path classification cache, revalidated by the (mtime, size) the walk
/// already carries. `list_items` is refetched on every vault-changed burst,
/// and classification reads every file's content for frontmatter + excerpt —
/// with the cache, a burst that touched 3 files re-reads 3 files instead of
/// the whole vault. Stale entries for deleted paths are pruned when the map
/// outgrows the live walk.
struct CachedItem {
    mtime: String,
    size: u64,
    item: VaultItem,
}
static ITEM_CACHE: Lazy<Mutex<HashMap<String, CachedItem>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

/// Build the full semantic index over the vault: classify every walked note,
/// reading changed files once for frontmatter + excerpt (unchanged files come
/// from the per-path cache). Newest first. Files whose basename starts with
/// `_` (tool internals like `people/_index.md`) are skipped, mirroring the
/// watcher's ignore rules.
pub fn list_items(vault: &Path) -> Vec<VaultItem> {
    let notes = walk_notes(vault);
    let mut out: Vec<VaultItem> = Vec::new();
    let mut guard = ITEM_CACHE.lock().ok();
    for note in &notes {
        let basename = note.name.as_str();
        if basename.starts_with('_') {
            continue;
        }
        if let Some(c) = guard.as_ref().and_then(|g| g.get(&note.path)) {
            if c.mtime == note.mtime && c.size == note.size {
                out.push(c.item.clone());
                continue;
            }
        }
        let content = std::fs::read_to_string(vault.join(&note.path)).unwrap_or_default();
        let (fm, body) = frontmatter::parse_map(&content);
        let mut item = classify(&note.path, &fm);
        item.mtime = note.mtime.clone();
        item.size = note.size;
        item.excerpt = body_excerpt(&body);
        if let Some(g) = guard.as_mut() {
            g.insert(
                note.path.clone(),
                CachedItem {
                    mtime: note.mtime.clone(),
                    size: note.size,
                    item: item.clone(),
                },
            );
        }
        out.push(item);
    }
    // Deleted/renamed paths leave dead entries behind; prune once the map
    // visibly outgrows the live vault rather than on every call.
    if let Some(g) = guard.as_mut() {
        if g.len() > notes.len() + 64 {
            let live: std::collections::HashSet<&str> =
                notes.iter().map(|n| n.path.as_str()).collect();
            g.retain(|k, _| live.contains(k.as_str()));
        }
    }
    out.retain(|item| item.kind != KIND_EMAIL && item.kind != KIND_CHAT);
    out.sort_by(|a, b| b.mtime.cmp(&a.mtime));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fm(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn tasks_route_to_task_modal() {
        let it = classify(
            "tasks/claims-dashboard.md",
            &fm(&[("title", "Claims Dashboard")]),
        );
        assert_eq!(it.kind, KIND_TASK);
        assert_eq!(it.surface, "tasks");
        assert_eq!(it.title, "Claims Dashboard");
        assert!(it.editable);
        assert!(!it.hidden);
        assert_eq!(it.trust, "authored");
        assert!(it.drift.is_none());
    }

    #[test]
    fn projects_carry_status_folder_depth() {
        let it = classify(
            "projects/superseded/HR Analytics Dashboard.md",
            &fm(&[
                ("title", "HR Analytics Dashboard"),
                ("status", "superseded"),
            ]),
        );
        assert_eq!(it.kind, KIND_PROJECT);
        assert_eq!(it.surface, "projects");
        assert_eq!(it.folder, "projects");
        assert!(it.editable);
    }

    #[test]
    fn calendar_is_a_hidden_synced_but_annotatable_mirror() {
        let it = classify(
            "calendar/2026-05-26-1100-touch-base.md",
            &fm(&[("type", "meeting"), ("subject", "Touch Base")]),
        );
        assert_eq!(it.kind, KIND_MEETING);
        assert_eq!(it.surface, "vault");
        assert_eq!(it.title, "Touch Base");
        // Frontmatter is synced from Outlook, but the body (Agenda / Notes /
        // Action items) is the user's to annotate — so it's editable.
        assert!(it.editable, "calendar note body is annotatable");
        assert!(it.hidden, "calendar floods browse — hidden by default");
        assert_eq!(it.trust, "synced");
    }

    #[test]
    fn user_meeting_notes_are_editable_and_visible() {
        let it = classify("meetings/kickoff.md", &fm(&[("title", "Kickoff")]));
        assert_eq!(it.kind, KIND_MEETING);
        assert!(it.editable, "user-authored meeting notes are editable");
        assert!(!it.hidden);
        assert_eq!(it.trust, "authored");
    }

    #[test]
    fn retired_emails_open_as_local_notes_and_are_hidden() {
        let it = classify(
            "emails/inbox/2026-05-06-foo.md",
            &fm(&[("subject", "FW: Loss Run"), ("from", "Alex")]),
        );
        assert_eq!(it.kind, KIND_EMAIL);
        assert_eq!(it.surface, "vault");
        assert_eq!(it.title, "FW: Loss Run");
        assert!(it.editable);
        assert!(it.hidden);
    }

    #[test]
    fn retired_chat_detected_by_service_marker_and_opens_locally() {
        let it = classify(
            "slack/dms/jake.md",
            &fm(&[("service", "slack"), ("conversation", "Riley Shaw")]),
        );
        assert_eq!(it.kind, KIND_CHAT);
        assert_eq!(it.surface, "vault");
        assert_eq!(it.title, "Riley Shaw");
        assert!(it.hidden);
    }

    #[test]
    fn recordings_are_readonly_synced_and_hidden() {
        let it = classify(
            "recordings/2026-05-31-sermon.md",
            &fm(&[
                ("type", "recording"),
                ("title", "Sermon"),
                ("plaud_id", "abc"),
            ]),
        );
        assert_eq!(it.kind, KIND_RECORDING);
        assert_eq!(it.surface, "vault");
        assert!(!it.editable);
        assert!(it.hidden);
        assert_eq!(it.trust, "synced");
    }

    #[test]
    fn briefings_are_generated_readonly_hidden() {
        let it = classify("briefings/2026-05-28.html", &fm(&[]));
        assert_eq!(it.kind, KIND_BRIEFING);
        assert!(!it.editable);
        assert!(it.hidden);
        assert_eq!(it.trust, "generated");
    }

    #[test]
    fn authored_knowledge_is_visible_and_editable() {
        for (path, props) in [
            ("daily/2026/2026-04-09.md", vec![]),
            ("domains/Engineering.md", vec![("title", "Engineering")]),
            (
                "people/Pat Rivera.md",
                vec![("title", "Pat Rivera"), ("role", "CEO")],
            ),
            ("me/Anti-patterns.md", vec![("title", "Anti-patterns")]),
            ("acme/Business.md", vec![("title", "Business")]),
        ] {
            let it = classify(path, &fm(&props));
            assert!(it.editable, "{path} should be editable");
            assert!(!it.hidden, "{path} should be visible");
            assert_eq!(it.surface, "vault", "{path} opens in the vault editor");
            assert_eq!(it.trust, "authored", "{path} is authored");
        }
    }

    #[test]
    fn goals_commitments_and_approved_learnings_have_first_class_kinds() {
        for (path, expected) in [
            ("goals/Improve reliability.md", KIND_GOAL),
            ("commitments/send-renewal.md", KIND_COMMITMENT),
            ("learnings/operator-preference.md", KIND_LEARNING),
        ] {
            let item = classify(path, &fm(&[("title", "Typed record")]));
            assert_eq!(item.kind, expected, "{path}");
            assert_eq!(item.surface, "vault");
            assert!(item.editable);
            assert!(!item.hidden);
            assert_eq!(item.trust, "authored");
        }
    }

    #[test]
    fn daily_falls_back_to_filename_when_untitled() {
        let it = classify("daily/2026/2026-04-09.md", &fm(&[("tags", "daily")]));
        assert_eq!(it.kind, KIND_DAILY);
        assert_eq!(it.title, "2026-04-09");
        assert!(it.drift.is_none(), "daily without a title is not drift");
    }

    #[test]
    fn sources_are_imported_trust() {
        let it = classify(
            "sources/2026-04-14 - Talk.md",
            &fm(&[
                ("title", "Talk"),
                ("type", "content"),
                ("source", "youtube"),
            ]),
        );
        assert_eq!(it.kind, KIND_SOURCE);
        assert_eq!(it.trust, "imported");
    }

    #[test]
    fn ingested_and_imported_map_to_source() {
        assert_eq!(classify("ingested/x/foo.md", &fm(&[])).kind, KIND_SOURCE);
        assert_eq!(
            classify("imported/3 Pillars.md", &fm(&[])).kind,
            KIND_SOURCE
        );
    }

    #[test]
    fn root_level_file_is_unfiled_and_flagged() {
        let it = classify("scratch.md", &fm(&[]));
        assert_eq!(it.kind, KIND_UNFILED);
        assert_eq!(it.folder, "");
        assert!(it.drift.is_some());
        assert!(it.drift.unwrap().contains("root"));
    }

    #[test]
    fn unknown_folder_is_unfiled_with_folder_named_in_drift() {
        let it = classify("misc/whatever.md", &fm(&[("title", "Whatever")]));
        assert_eq!(it.kind, KIND_UNFILED);
        let drift = it.drift.expect("flagged");
        assert!(
            drift.contains("misc"),
            "names the offending folder: {drift}"
        );
    }

    #[test]
    fn root_file_with_type_frontmatter_is_classified_not_unfiled() {
        let it = classify(
            "Claude Access Roster.md",
            &fm(&[("title", "Roster"), ("type", "note")]),
        );
        assert_eq!(it.kind, KIND_NOTE);
        assert!(it.drift.is_none());
    }

    #[test]
    fn empty_knowledge_note_is_flagged_for_cleanup() {
        let it = classify("notes/orphan.md", &fm(&[]));
        assert_eq!(it.kind, KIND_NOTE);
        assert!(it.drift.is_some(), "untyped knowledge note flagged");
    }

    #[test]
    fn kind_from_folder_covers_the_vocabulary() {
        assert_eq!(kind_from_folder("tasks"), Some(KIND_TASK));
        assert_eq!(kind_from_folder("emails"), Some(KIND_EMAIL));
        assert_eq!(kind_from_folder("teams"), Some(KIND_CHAT));
        assert_eq!(kind_from_folder("slack"), Some(KIND_CHAT));
        assert_eq!(kind_from_folder("goals"), Some(KIND_GOAL));
        assert_eq!(kind_from_folder("commitments"), Some(KIND_COMMITMENT));
        assert_eq!(kind_from_folder("learnings"), Some(KIND_LEARNING));
        assert_eq!(kind_from_folder("nonsense"), None);
        assert_eq!(kind_from_folder(""), None);
    }

    #[test]
    fn body_excerpt_skips_heading_and_links() {
        let body = "# Title\n\n[link](http://x)\n\nThe real first sentence here.";
        assert_eq!(body_excerpt(body), "The real first sentence here.");
    }
}
