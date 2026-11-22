//! Recordings (next-level P11, 2026-10-01): a reader over the user's Plaud
//! recordings, on demand, through the Plaud sign-in Hollow already holds
//! (`plaud.rs`, the `plaud` CLI). Recordings stay in Plaud: listing and
//! reading copy nothing into the vault. This replaces nothing removed on
//! 2026-09-26: there is still no background sync and no meeting archive.
//!
//! What Hollow keeps locally is small and explicit:
//! - links: `{vault}/.hollow/recording-links.json` (recording id → workspace
//!   and/or calendar event, chosen by the user, never guessed);
//! - an opt-in transcript copy, written only when the user asks, into one
//!   workspace's `sources/` with its source id and retrieval time.
//!
//! The CLI prints human text, so the parsers here are strict: a line that
//! doesn't match is skipped and counted, never guessed at, and "unavailable"
//! (no transcript, no summary, no audio) is a state, not empty content.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::state_file::OnDamage;

static LOCK: Mutex<()> = Mutex::new(());

/// How long Plaud's recent list is reused (overnight S7: the Inbox and the
/// Recordings view read Plaud on every open, so a slow Plaud filled them in
/// late). The links file is local and always read fresh; Refresh skips this.
const RECENT_TTL: std::time::Duration = std::time::Duration::from_secs(120);

/// The last successful `plaud recent` output.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Recent {
    days: String,
    at: std::time::Instant,
    fetched_at: String,
    text: String,
}

static RECENT: Mutex<Option<Recent>> = Mutex::new(None);
/// Bumped by [`forget_recent`]: a read that started before a sign-in or
/// sign-out isn't kept when it finishes after it.
static RECENT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Keep `read` unless a forget came since `generation` was taken or a newer
/// read (started later) is already kept.
fn keep_recent(slot: &mut Option<Recent>, read: Recent, generation: u64, now_generation: u64) {
    if generation != now_generation || slot.as_ref().is_some_and(|k| k.at > read.at) {
        return;
    }
    *slot = Some(read);
}

/// The kept list for `days`, while younger than [`RECENT_TTL`] at `now`.
fn kept_recent(slot: &Option<Recent>, days: &str, now: std::time::Instant) -> Option<Recent> {
    slot.as_ref()
        .filter(|r| r.days == days && now.saturating_duration_since(r.at) < RECENT_TTL)
        .cloned()
}

/// Forget the kept list (a Plaud sign-in or sign-out).
pub(crate) fn forget_recent() {
    RECENT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *RECENT.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingRow {
    pub id: String,
    pub name: String,
    pub date: Option<String>,
    pub duration: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub start: String,
    pub end: String,
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingList {
    /// `ok` | `notConnected` | `unavailable`.
    pub state: String,
    pub fetched_at: String,
    pub source: String,
    pub recordings: Vec<RecordingRow>,
    /// Lines the parser didn't recognise (shown, never silently dropped).
    pub unparsed: usize,
    pub error: Option<String>,
    pub links: BTreeMap<String, RecordingLink>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingDetail {
    pub id: String,
    pub name: Option<String>,
    pub fetched_at: String,
    /// `null` with `transcriptState: "unavailable"` when Plaud has none.
    pub transcript: Option<Vec<Segment>>,
    pub transcript_state: String,
    pub summary: Option<String>,
    pub summary_state: String,
    /// `available` | `unavailable` | `unknown`. The audio URL (which expires)
    /// is fetched only when the user asks for it.
    pub audio: String,
    pub link: Option<RecordingLink>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordingLink {
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub event_title: Option<String>,
    /// Known Calendar start; older links omit it and use the recording day to search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_start: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub linked_at: String,
    /// Vault path of an opt-in transcript copy, when the user kept one.
    #[serde(default)]
    pub kept_copy: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkStore {
    #[serde(default)]
    schema_version: u32,
    #[serde(default)]
    links: BTreeMap<String, RecordingLink>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

fn links_path(vault: &Path) -> PathBuf {
    vault.join(".hollow").join("recording-links.json")
}
/// The recording links. Only a missing file is empty: an unreadable store is
/// an error and stays put, and a damaged one is moved aside
/// (`recording-links.json.corrupt-<time>`) before the error (B·F08).
fn read_links(vault: &Path) -> Result<LinkStore, String> {
    read_links_or(vault, OnDamage::SetAside)
}

/// [`read_links`], choosing what happens to a damaged store: the listing
/// and reader an agent calls leave it in place (B·F08).
fn read_links_or(vault: &Path, on_damage: OnDamage) -> Result<LinkStore, String> {
    let store =
        crate::state_file::read_json_or(&links_path(vault), "Recording link store", on_damage)?;
    Ok(store.unwrap_or_default())
}

/// Explicit calendar links only; unreadable state is never an empty list.
pub(crate) fn linked_to_event(
    vault: &Path,
    event_id: &str,
) -> Result<Vec<(String, RecordingLink)>, String> {
    let mut links: Vec<_> = read_links_or(vault, OnDamage::Leave)?
        .links
        .into_iter()
        .filter(|(_, link)| link.event_id.as_deref() == Some(event_id))
        .collect();
    links.sort_by(|a, b| b.1.linked_at.cmp(&a.1.linked_at).then(a.0.cmp(&b.0)));
    Ok(links)
}

/// Picker selection changes only the event; retain the workspace and kept copy
/// under the same lock as the link write, even if another UI linked it meanwhile.
pub(crate) fn link_event(
    vault: &Path,
    id: &str,
    event: &str,
    title: &str,
    start: Option<&str>,
) -> Result<RecordingLink, String> {
    valid_id(id)?;
    let _g = LOCK.lock().map_err(|_| "recordings lock poisoned")?;
    let mut store = read_links_or(vault, OnDamage::Leave)?;
    let link = store.links.entry(id.into()).or_default();
    if link.event_id.as_deref() != Some(event) {
        link.event_start = None;
    }
    link.event_id = Some(event.into());
    link.event_title = Some(title.into());
    if let Some(start) = start.filter(|s| !s.is_empty()) {
        link.event_start = Some(start.into());
    }
    link.linked_at = chrono::Utc::now().to_rfc3339();
    let result = link.clone();
    write_links(vault, &store)?;
    Ok(result)
}
fn write_links(vault: &Path, s: &LinkStore) -> Result<(), String> {
    let mut s2 = LinkStore {
        schema_version: 1,
        links: s.links.clone(),
        extra: s.extra.clone(),
    };
    s2.schema_version = 1;
    crate::vault::atomic_write(
        &links_path(vault),
        serde_json::to_string_pretty(&s2)
            .map_err(|e| e.to_string())?
            .as_bytes(),
    )
}

/// `  <id>  <name>  <YYYY-MM-DD|->  <duration>` (`plaud recent`). The name may
/// contain double spaces; the date and duration are the last two fields.
pub(crate) fn parse_recent(text: &str) -> (Vec<RecordingRow>, usize) {
    let mut out = Vec::new();
    let mut unparsed = 0;
    for raw in text.lines() {
        let line = raw.trim_end();
        let t = line.trim();
        if t.is_empty() || t.starts_with("Recordings in the last") || t.starts_with("No recordings")
        {
            continue;
        }
        let parts: Vec<&str> = t
            .split("  ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() < 4 || !raw.starts_with("  ") {
            unparsed += 1;
            continue;
        }
        let (id, duration, date) = (parts[0], parts[parts.len() - 1], parts[parts.len() - 2]);
        let date_ok = date == "-"
            || (date.len() == 10 && date.as_bytes()[4] == b'-' && date.as_bytes()[7] == b'-');
        if id.contains(' ') || id.len() < 6 || !date_ok {
            unparsed += 1;
            continue;
        }
        let name = parts[1..parts.len() - 2].join("  ");
        out.push(RecordingRow {
            id: id.into(),
            name,
            date: (date != "-").then(|| date.to_string()),
            duration: (duration != "-").then(|| duration.to_string()),
        });
    }
    (out, unparsed)
}

/// `[MM:SS - MM:SS] Speaker: text` (`plaud transcript`). A speaker is the
/// text before the first ": " only when it is short and has no sentence
/// punctuation; otherwise the whole line is text with no speaker.
pub(crate) fn parse_transcript(text: &str) -> Option<Vec<Segment>> {
    if text.contains("Transcript not available") {
        return None;
    }
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix('[') else {
            continue;
        };
        let Some((times, body)) = rest.split_once("] ") else {
            continue;
        };
        let Some((start, end)) = times.split_once(" - ") else {
            continue;
        };
        let (speaker, text) = match body.split_once(": ") {
            Some((s, t)) if s.len() <= 40 && !s.contains(['.', '?', '!']) => {
                (Some(s.to_string()), t.to_string())
            }
            _ => (None, body.to_string()),
        };
        out.push(Segment {
            start: start.into(),
            end: end.into(),
            speaker,
            text,
        });
    }
    Some(out)
}

pub(crate) fn parse_summary(text: &str) -> Option<String> {
    if text.contains("Summary not available") {
        return None;
    }
    let body = text
        .lines()
        .skip_while(|l| l.trim().is_empty() || l.trim_start().starts_with("Summary:"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = body.trim();
    (!body.is_empty()).then(|| body.to_string())
}

pub(crate) fn parse_file_flags(text: &str) -> (Option<String>, String) {
    let mut name = None;
    let mut audio = "unknown".to_string();
    for l in text.lines() {
        let t = l.trim();
        if let Some(v) = t.strip_prefix("name:") {
            name = Some(v.trim().to_string());
        }
        if let Some(v) = t.strip_prefix("audio:") {
            audio = if v.trim() == "available" {
                "available".into()
            } else {
                "unavailable".into()
            };
        }
    }
    (name, audio)
}

fn not_connected(e: &str) -> bool {
    e.contains("not authenticated") || e.contains("AUTH_FAILED")
}

/// The recent recordings and their links. `on_damage` is
/// [`OnDamage::Leave`] for an agent action, so a damaged links file stays
/// for the Recordings view to move aside and report.
///
/// `fresh` asks Plaud again; otherwise a list read in the last
/// [`RECENT_TTL`] is reused (its `fetched_at` says when it was read).
pub async fn list(
    vault: &Path,
    days: u32,
    on_damage: OnDamage,
    fresh: bool,
) -> Result<RecordingList, String> {
    let mut fetched_at = chrono::Utc::now().to_rfc3339();
    let links = read_links_or(vault, on_damage)?.links;
    let source = "Plaud · read only".to_string();
    if !crate::plaud::is_connected() {
        return Ok(RecordingList {
            state: "notConnected".into(),
            fetched_at,
            source,
            recordings: vec![],
            unparsed: 0,
            error: None,
            links,
        });
    }
    let days = days.clamp(1, 365).to_string();
    let kept = (!fresh)
        .then(|| {
            kept_recent(
                &RECENT.lock().unwrap_or_else(|e| e.into_inner()),
                &days,
                std::time::Instant::now(),
            )
        })
        .flatten();
    let recent = match kept {
        Some(k) => {
            fetched_at = k.fetched_at;
            Ok(k.text)
        }
        None => {
            let generation = RECENT_GENERATION.load(std::sync::atomic::Ordering::SeqCst);
            let started = std::time::Instant::now();
            let read =
                crate::plaud::run_cli(&["recent", "--days", &days], crate::plaud::CLI_TIMEOUT)
                    .await;
            if let Ok(text) = &read {
                keep_recent(
                    &mut RECENT.lock().unwrap_or_else(|e| e.into_inner()),
                    Recent {
                        days: days.clone(),
                        at: started,
                        fetched_at: fetched_at.clone(),
                        text: text.clone(),
                    },
                    generation,
                    RECENT_GENERATION.load(std::sync::atomic::Ordering::SeqCst),
                );
            }
            read
        }
    };
    let list = match recent {
        Ok(text) => {
            let (recordings, unparsed) = parse_recent(&text);
            RecordingList {
                state: "ok".into(),
                fetched_at,
                source,
                recordings,
                unparsed,
                error: None,
                links,
            }
        }
        Err(e) if not_connected(&e) => RecordingList {
            state: "notConnected".into(),
            fetched_at,
            source,
            recordings: vec![],
            unparsed: 0,
            error: Some(e),
            links,
        },
        Err(e) => RecordingList {
            state: "unavailable".into(),
            fetched_at,
            source,
            recordings: vec![],
            unparsed: 0,
            error: Some(e),
            links,
        },
    };
    Ok(list)
}

fn valid_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!("not a Plaud recording id: {id}"));
    }
    Ok(())
}

/// One recording. `on_damage` as for [`list`].
pub async fn read(vault: &Path, id: &str, on_damage: OnDamage) -> Result<RecordingDetail, String> {
    valid_id(id)?;
    // The link first: a damaged or unreadable store fails before Plaud is asked.
    let link = read_links_or(vault, on_damage)?.links.get(id).cloned();
    let fetched_at = chrono::Utc::now().to_rfc3339();
    let mut errors = vec![];
    let to = crate::plaud::CLI_TIMEOUT;
    let (fa, ta, sa) = (["file", id], ["transcript", id], ["summary", id]);
    let (file, transcript, summary) = tokio::join!(
        crate::plaud::run_cli(&fa, to),
        crate::plaud::run_cli(&ta, to),
        crate::plaud::run_cli(&sa, to)
    );
    let (name, audio) = match file {
        Ok(t) => parse_file_flags(&t),
        Err(e) => {
            errors.push(e);
            (None, "unknown".into())
        }
    };
    let (transcript, transcript_state) = match transcript {
        Ok(t) => match parse_transcript(&t) {
            Some(s) => (Some(s), "available".to_string()),
            None => (None, "unavailable".into()),
        },
        Err(e) => {
            errors.push(e);
            (None, "error".into())
        }
    };
    let (summary, summary_state) = match summary {
        Ok(t) => match parse_summary(&t) {
            Some(s) => (Some(s), "available".to_string()),
            None => (None, "unavailable".into()),
        },
        Err(e) => {
            errors.push(e);
            (None, "error".into())
        }
    };
    Ok(RecordingDetail {
        id: id.into(),
        name,
        fetched_at,
        transcript,
        transcript_state,
        summary,
        summary_state,
        audio,
        link,
        errors,
    })
}

/// The audio download URL, on request (it expires after about a day).
pub async fn audio_url(id: &str) -> Result<String, String> {
    valid_id(id)?;
    let text = crate::plaud::run_cli(&["audio", id], crate::plaud::CLI_TIMEOUT).await?;
    text.lines()
        .map(str::trim)
        .find(|l| l.starts_with("https://"))
        .map(str::to_string)
        .ok_or_else(|| "Audio isn’t available for this recording.".into())
}

/// Link (or unlink, with both fields empty) a recording to a workspace and/or
/// a calendar event. Explicit only: nothing calls this by guessing.
pub fn set_link(
    vault: &Path,
    id: &str,
    workspace_id: Option<String>,
    event_id: Option<String>,
    event_title: Option<String>,
    name: Option<String>,
) -> Result<Option<RecordingLink>, String> {
    valid_id(id)?;
    if let Some(w) = workspace_id.as_deref() {
        crate::workspaces::summary(vault, w)?;
    }
    let _g = LOCK.lock().map_err(|_| "recordings lock poisoned")?;
    let mut store = read_links(vault)?;
    if workspace_id.is_none() && event_id.is_none() {
        store.links.remove(id);
        write_links(vault, &store)?;
        return Ok(None);
    }
    let prev = store.links.get(id).cloned().unwrap_or_default();
    let event_start = if event_id == prev.event_id {
        prev.event_start
    } else {
        None
    };
    let link = RecordingLink {
        workspace_id,
        event_id,
        event_title,
        event_start,
        name: name.or(prev.name),
        linked_at: chrono::Utc::now().to_rfc3339(),
        kept_copy: prev.kept_copy,
    };
    store.links.insert(id.into(), link.clone());
    write_links(vault, &store)?;
    Ok(Some(link))
}

/// Opt-in local retention: write this recording's transcript into one
/// workspace's sources/, with its provenance. Refuses to overwrite.
pub async fn keep_transcript(vault: &Path, id: &str, workspace_id: &str) -> Result<String, String> {
    let detail = read(vault, id, OnDamage::SetAside).await?;
    let Some(segments) = detail.transcript else {
        return Err(
            "Plaud has no transcript for this recording, so there is nothing to keep.".into(),
        );
    };
    let w = crate::workspaces::summary(vault, workspace_id)?;
    let dir = crate::workspaces::folder(vault, workspace_id)?;
    let slug: String = detail
        .name
        .clone()
        .unwrap_or_else(|| id.into())
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    let rel = format!(
        "sources/recording-{}-{}.md",
        &slug.chars().take(60).collect::<String>(),
        &id.chars().take(8).collect::<String>()
    );
    let full = dir.join(&rel);
    if full.exists() {
        return Err(format!("{rel} already exists; nothing was overwritten"));
    }
    let mut body = format!("---\nsource: plaud\nrecording_id: {id}\nretrieved_at: {}\nretention: kept by the user (opt-in)\n---\n\n# {}\n\n", detail.fetched_at, detail.name.clone().unwrap_or_else(|| id.into()));
    for s in &segments {
        body.push_str(&format!(
            "[{} - {}] {}{}\n",
            s.start,
            s.end,
            s.speaker
                .as_deref()
                .map(|x| format!("{x}: "))
                .unwrap_or_default(),
            s.text
        ));
    }
    crate::vault::atomic_write(&full, body.as_bytes())?;
    let path = format!("{}/{rel}", w.path);
    let _g = LOCK.lock().map_err(|_| "recordings lock poisoned")?;
    let mut store = read_links(vault)?;
    let link = store
        .links
        .entry(id.into())
        .or_insert_with(|| RecordingLink {
            workspace_id: Some(workspace_id.into()),
            linked_at: chrono::Utc::now().to_rfc3339(),
            ..Default::default()
        });
    link.kept_copy = Some(path.clone());
    write_links(vault, &store)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_links_are_newest_first_and_picker_preserves_metadata() {
        let v = tempfile::tempdir().unwrap();
        let mut s = LinkStore::default();
        for (id, at) in [("a", "2026-10-17"), ("b", "2026-10-18")] {
            s.links.insert(
                id.into(),
                RecordingLink {
                    event_id: Some("event".into()),
                    linked_at: at.into(),
                    kept_copy: Some("sources/kept.md".into()),
                    workspace_id: Some("existing".into()),
                    ..Default::default()
                },
            );
        }
        write_links(v.path(), &s).unwrap();
        assert_eq!(
            linked_to_event(v.path(), "event")
                .unwrap()
                .iter()
                .map(|(id, _)| id.as_str())
                .collect::<Vec<_>>(),
            vec!["b", "a"]
        );
        let linked = link_event(
            v.path(),
            "a",
            "new",
            "New title",
            Some("2026-10-17T12:00:00Z"),
        )
        .unwrap();
        assert_eq!(linked.workspace_id.as_deref(), Some("existing"));
        assert_eq!(linked.kept_copy.as_deref(), Some("sources/kept.md"));
        assert_eq!(linked.event_start.as_deref(), Some("2026-10-17T12:00:00Z"));
        std::fs::write(links_path(v.path()), "broken").unwrap();
        assert!(linked_to_event(v.path(), "new").is_err());
        assert_eq!(
            std::fs::read_to_string(links_path(v.path())).unwrap(),
            "broken"
        );
    }

    #[test]
    fn recent_lines_parse_strictly() {
        let text = "\nRecordings in the last 7 days: 3\n\n  a1b2c3d4e5f6a7b8  Weekly ops sync  2026-09-30  42m10s\n  ffeeddccbbaa0011  Vendor call  with  two spaces  2026-09-29  1h02m\n  zz  bad  row\n  0123456789abcdef  No date  -  -\n";
        let (rows, unparsed) = parse_recent(text);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1].name, "Vendor call  with  two spaces");
        assert_eq!(rows[0].duration.as_deref(), Some("42m10s"));
        assert_eq!(
            (rows[2].date.clone(), rows[2].duration.clone()),
            (None, None)
        );
        assert_eq!(unparsed, 1);
        assert_eq!(parse_recent("No recordings in the last 7 days.").0.len(), 0);
    }

    #[test]
    fn transcripts_keep_timestamps_and_unknown_speakers() {
        let t = "\nTranscript: Ops sync\n\n[00:05 - 00:12] Dana: We need two float supervisors.\n[00:12 - 00:20] Okay. So: the budget is next week.\nnoise line\n";
        let s = parse_transcript(t).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].start.as_str(), s[0].speaker.as_deref()),
            ("00:05", Some("Dana"))
        );
        assert_eq!(s[1].speaker, None);
        assert!(parse_transcript("Transcript not available for this recording.").is_none());
        assert!(parse_summary("Summary not available for this recording.").is_none());
        assert_eq!(
            parse_summary("\nSummary: Ops\n\n## Action items\n- Hire").as_deref(),
            Some("## Action items\n- Hire")
        );
        assert_eq!(
            parse_file_flags("  name:         Ops sync\n  audio:        unavailable").1,
            "unavailable"
        );
    }

    #[test]
    fn links_are_explicit_and_removable() {
        let dir = tempfile::tempdir().unwrap();
        let w = crate::workspaces::create(dir.path(), "Hiring", "Plan").unwrap();
        assert!(set_link(dir.path(), "bad id", Some(w.id.clone()), None, None, None).is_err());
        assert!(set_link(
            dir.path(),
            "abc123",
            Some("missing".into()),
            None,
            None,
            None
        )
        .is_err());
        let l = set_link(
            dir.path(),
            "abc123",
            Some(w.id.clone()),
            Some("ev1".into()),
            Some("Ops".into()),
            Some("Ops sync".into()),
        )
        .unwrap()
        .unwrap();
        assert_eq!(l.workspace_id.as_deref(), Some(w.id.as_str()));
        assert_eq!(read_links(dir.path()).unwrap().links.len(), 1);
        assert!(set_link(dir.path(), "abc123", None, None, None, None)
            .unwrap()
            .is_none());
        assert!(read_links(dir.path()).unwrap().links.is_empty());
    }

    #[test]
    fn a_damaged_link_store_is_moved_aside_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let path = links_path(v);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        // Not UTF-8: before B·F08 this read as no links and the next link
        // replaced the file.
        let damaged = b"{\"links\": \xff".to_vec();
        std::fs::write(&path, &damaged).unwrap();
        let ev = || Some("ev1".to_string());
        let err = set_link(v, "abc123", None, ev(), None, None).unwrap_err();
        assert!(err.contains("Recording link store was damaged"), "{err}");
        let kept = crate::state_file::corrupt_copies(&path);
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
        // Set aside, the next link starts a fresh store.
        assert!(set_link(v, "abc123", None, ev(), None, None)
            .unwrap()
            .is_some());
        assert_eq!(std::fs::read(&kept[0]).unwrap(), damaged);
    }

    #[tokio::test]
    async fn an_agent_listing_leaves_a_damaged_link_store_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let path = links_path(v);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"nope").unwrap();
        // The links are read before Plaud is asked, so no CLI runs here.
        let err = list(v, 14, OnDamage::Leave, false).await.unwrap_err();
        assert!(err.contains("Recording link store is damaged"), "{err}");
        let err = read(v, "abc123", OnDamage::Leave).await.unwrap_err();
        assert!(err.contains("Recording link store is damaged"), "{err}");
        assert_eq!(std::fs::read(&path).unwrap(), b"nope");
        assert!(crate::state_file::corrupt_copies(&path).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_link_store_is_surfaced_and_left_alone() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let ev = || Some("ev1".to_string());
        set_link(v, "abc123", None, ev(), None, None).unwrap();
        let path = links_path(v);
        let before = std::fs::read(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&path).is_ok() {
            return; // running as root: permissions don't stop the read
        }
        let err = set_link(v, "def456", None, ev(), None, None).unwrap_err();
        assert!(err.contains("could not be read"), "{err}");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before, "never replaced");
        assert!(crate::state_file::corrupt_copies(&path).is_empty());
    }
}

#[cfg(test)]
mod recent_cache_tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn plauds_list_is_reused_briefly_for_the_same_window_only() {
        let t0 = Instant::now();
        let slot = Some(Recent {
            days: "14".into(),
            at: t0,
            fetched_at: "2026-10-13T07:00:00Z".into(),
            text: "rec".into(),
        });
        assert_eq!(
            kept_recent(&slot, "14", t0 + Duration::from_secs(60))
                .unwrap()
                .text,
            "rec"
        );
        assert!(
            kept_recent(&slot, "30", t0).is_none(),
            "another window asks again"
        );
        assert!(kept_recent(&slot, "14", t0 + RECENT_TTL).is_none(), "stale");
        assert!(kept_recent(&None, "14", t0).is_none());
    }

    #[test]
    fn a_read_from_before_a_sign_in_or_an_older_read_is_never_kept() {
        let t0 = Instant::now();
        let read = |at: Instant, text: &str| Recent {
            days: "14".into(),
            at,
            fetched_at: String::new(),
            text: text.into(),
        };
        let mut slot = None;
        keep_recent(&mut slot, read(t0, "old account"), 3, 4);
        assert!(slot.is_none(), "a forget came while it ran");
        keep_recent(&mut slot, read(t0 + Duration::from_secs(5), "fresh"), 4, 4);
        keep_recent(&mut slot, read(t0, "slow older read"), 4, 4);
        assert_eq!(slot.unwrap().text, "fresh");
    }
}
