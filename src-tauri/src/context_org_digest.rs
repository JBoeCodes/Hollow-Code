//! The daily digest of org changes merged as the owner (context layers CX8b).
//!
//! **Input.** `context-org-prs.jsonl`, the record `context_propose.rs` (CX8)
//! appends to: one `opened` line per pull request and a later `merged` line
//! when `context.merge_org_change` merges one that waited. A PR is listed when
//! its opened line says `identity: owner` and `digest: true`, and it merged:
//! at the opened line's time when that line already says `merged`, else at
//! its first `merged` line's time. So a PR that waited is listed on the day it
//! merged, not the day it opened. Teammates' PRs (`digest: false`) are never
//! listed, and owner PRs still waiting on checks are left out until they merge
//! (listing them every day would make the digest never quiet).
//!
//! **Watermark.** `context-org-digest.json` beside the record holds the urls
//! already listed and when the last digest ran. A url is listed once, ever,
//! so running twice in a day lists nothing new, and a line appended while a
//! digest read the record is picked up by the next one.
//!
//! **Output.** One note per day in the Main workspace's outputs,
//! `workspaces/main/outputs/org-changes/<YYYY-MM-DD>.md` (beside the weekly
//! update and the gardener's reports, so it is where the COO and the person
//! already look): one line per PR with its title (or branch), url, files,
//! merge time and chat. Never the reason, the scan or any file's contents. A
//! second run the same day appends only urls the note does not hold yet. Then
//! one in-app notification, "Org changes merged as you", that opens the note.
//! Nothing to report writes no note, no item and no state.
//!
//! **When.** The `org-digest` window job (`schedule_windows::JOBS`) on the
//! `nightly` window (02:00 to 05:00, run once on wake), so the digest waits in
//! Recent activity by morning without a native pop at night. [`run`] takes
//! the clock, the zone and every path, so tests use none of the app's data.

use crate::notifications::NotificationInput;
use chrono::{DateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// The watermark's file name, beside the record in app data.
pub(crate) const STATE_FILE: &str = "context-org-digest.json";
/// The notification's source and the window job's id.
pub(crate) const SOURCE: &str = "org-digest";
const NOTE_DIR: &str = "workspaces/main/outputs/org-changes";

/// One PR merged as the owner.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Merged {
    pub url: String,
    pub title: String,
    pub branch: String,
    pub files: Vec<String>,
    pub chat: Option<String>,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    through: Option<String>,
    #[serde(default)]
    listed: BTreeSet<String>,
}

/// What one run wrote.
#[derive(Debug)]
pub(crate) struct Digest {
    /// Vault-relative path of the note.
    pub path: String,
    pub listed: Vec<Merged>,
    pub item: NotificationInput,
}

/// `s` without control characters, at most `n` characters.
fn clean(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

fn text(v: &Value, key: &str) -> String {
    clean(v[key].as_str().unwrap_or(""), 200)
}

/// Link text that markdown shows as typed: every character that could
/// open a code span, emphasis, HTML, an entity, an image or a link is
/// backslash-escaped (CommonMark escapes any ASCII punctuation).
pub(crate) fn md_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '`' | '*' | '_' | '<' | '>' | '&' | '!' | '[' | ']'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// A code span's contents: a backtick would end the span early.
fn code(s: &str) -> String {
    format!("`{}`", s.replace('`', "'"))
}

fn time(v: &Value) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(v["at"].as_str()?)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Owner PRs in the record that merged at or before `now`, oldest first.
pub(crate) fn merged_owner_prs(record: &str, now: DateTime<Utc>) -> Vec<Merged> {
    let mut opened: Vec<(String, Value)> = vec![];
    let mut merged_at: HashMap<String, DateTime<Utc>> = HashMap::new();
    for line in record.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(url) = v["url"].as_str().map(str::to_string) else {
            continue;
        };
        match v["event"].as_str() {
            Some("merged") => {
                if let Some(t) = time(&v) {
                    merged_at.entry(url).or_insert(t);
                }
            }
            Some("opened") if !opened.iter().any(|(u, _)| *u == url) => opened.push((url, v)),
            _ => {}
        }
    }
    let mut out: Vec<Merged> = opened
        .into_iter()
        .filter(|(_, v)| v["digest"] == Value::Bool(true) && v["identity"] == "owner")
        .filter_map(|(url, v)| {
            let at = if v["status"] == "merged" {
                time(&v)?
            } else {
                *merged_at.get(&url)?
            };
            Some(Merged {
                title: text(&v, "title"),
                branch: text(&v, "branch"),
                files: v["files"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|f| f.as_str().map(|s| clean(s, 200)))
                            .collect()
                    })
                    .unwrap_or_default(),
                chat: v["chat"].as_str().map(|c| clean(c, 80)),
                // An angle-bracket link destination can't hold these.
                url: clean(&url, 300).replace(['<', '>', ' '], ""),
                at,
            })
        })
        .filter(|m| m.at <= now)
        .collect();
    out.sort_by_key(|m| m.at);
    out
}

/// One note line. The title is escaped ([`md_text`]), the url is an
/// angle-bracket destination and the rest are code spans, so nothing in the
/// record can change the note's markup.
pub(crate) fn line<Tz: TimeZone>(m: &Merged, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let name = md_text(if m.title.is_empty() {
        &m.branch
    } else {
        &m.title
    });
    let files = if m.files.is_empty() {
        "no files named".to_string()
    } else {
        m.files
            .iter()
            .map(|f| code(f))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let chat = m
        .chat
        .as_deref()
        .map(|c| format!("chat {}", code(c)))
        .unwrap_or_else(|| "no chat".into());
    format!(
        "- {} [{name}](<{}>): {files}; branch {}; {chat}\n",
        m.at.with_timezone(tz).format("%b %-d %H:%M"),
        m.url,
        code(&m.branch)
    )
}

fn header<Tz: TimeZone>(now: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    format!(
        "# Org changes merged as you, {}\n\nPull requests Hollow merged into the organization layer under your login since the last digest (`context.propose_org_change`, `context.merge_org_change`). Teammates' pull requests and yours still waiting on checks are not listed; a waiting one appears on the day it merges.\n\n",
        now.with_timezone(tz).format("%a %b %-d, %Y")
    )
}

/// The watermark: a missing file is a first run; an unreadable or broken
/// one stops the run (the job retries) instead of listing everything again.
fn read_state(path: &Path) -> Result<State, String> {
    match std::fs::read_to_string(path) {
        Ok(t) => serde_json::from_str(&t)
            .map_err(|e| format!("the org digest watermark is not valid JSON: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(format!("reading the org digest watermark: {e}")),
    }
}

/// Build today's digest: read the record, list owner PRs merged since the
/// last digest, write or extend the day's note, move the watermark, and
/// return the notification to post. `Ok(None)`: nothing to report, nothing
/// written.
pub(crate) fn run<Tz: TimeZone>(
    vault: &Path,
    ledger: &Path,
    state_path: &Path,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Result<Option<Digest>, String>
where
    Tz::Offset: std::fmt::Display,
{
    // Lossy: one bad byte must not block every later digest.
    let record = match std::fs::read(ledger) {
        Ok(b) => String::from_utf8_lossy(&b).into_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("reading the org PR record: {e}")),
    };
    let mut state = read_state(state_path)?;
    let fresh: Vec<Merged> = merged_owner_prs(&record, now)
        .into_iter()
        .filter(|m| !state.listed.contains(&m.url))
        .collect();
    if fresh.is_empty() {
        return Ok(None);
    }
    let rel = format!("{NOTE_DIR}/{}.md", now.with_timezone(tz).format("%Y-%m-%d"));
    let path = vault.join(&rel);
    let existing = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        // Never replace a note that is there but unreadable.
        Err(e) => return Err(format!("reading {rel}: {e}")),
    };
    let mut body = if existing.is_empty() {
        header(now, tz)
    } else {
        existing.clone()
    };
    if !body.ends_with('\n') {
        body.push('\n');
    }
    for m in &fresh {
        // A run whose watermark write failed already wrote this line.
        if !existing.contains(&format!("](<{}>)", m.url)) {
            body.push_str(&line(m, tz));
        }
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("org changes folder: {e}"))?;
    }
    crate::vault::atomic_write(&path, body.as_bytes())?;
    state.listed.extend(fresh.iter().map(|m| m.url.clone()));
    state.through = Some(now.to_rfc3339());
    if let Some(dir) = state_path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("app data folder: {e}"))?;
    }
    let json = serde_json::to_vec_pretty(&state).map_err(|e| e.to_string())?;
    crate::vault::atomic_write(state_path, &json)?;
    let n = fresh.len();
    let item = NotificationInput {
        title: "Org changes merged as you".into(),
        body: format!(
            "{n} pull request{} merged into the organization layer under your login since the last digest.",
            if n == 1 { "" } else { "s" }
        ),
        kind: "info".into(),
        source: Some(SOURCE.into()),
        action_label: Some("Open".into()),
        action_url: Some(format!("vault:{rel}")),
        notify_native: false,
    };
    Ok(Some(Digest {
        path: rel,
        listed: fresh,
        item,
    }))
}

/// The watermark's path in Hollow's app data.
pub(crate) fn state_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join(STATE_FILE))
}

async fn run_nightly(
    app: tauri::AppHandle,
    config: crate::config::Config,
    _key: String,
    _late: bool,
) -> Result<(), String> {
    let vault = config.vault_path.clone();
    if crate::workspaces::main_agent_cwd(&vault).is_none() {
        eprintln!("[org digest] no COO workspace; no digest written");
        return Ok(());
    }
    let ledger = crate::context_propose::ledger_path()?;
    let state = state_path()?;
    let digest = {
        let vault = vault.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let guard = crate::automated_history::scheduler_write_lease(&vault)
                .map_err(|_| "the vault is busy".to_string())?;
            let out = run(&vault, &ledger, &state, Utc::now(), &chrono::Local);
            drop(guard);
            out
        })
        .await
        .map_err(|e| e.to_string())??
    };
    if let Some(d) = digest {
        eprintln!("[org digest] {} PR(s) listed in {}", d.listed.len(), d.path);
        if let Err(e) = crate::notifications::create(&app, &vault, d.item) {
            eprintln!("[org digest] the notification was not posted: {e}");
        }
    }
    Ok(())
}

/// The `nightly` window job (`schedule_windows::JOBS`, id `org-digest`).
pub(crate) fn nightly_job(
    app: tauri::AppHandle,
    config: crate::config::Config,
    key: String,
    late: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>> {
    Box::pin(run_nightly(app, config, key, late))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_propose::{record, record_merged, Expect, Outcome};
    use chrono::{Duration, FixedOffset};

    fn outcome(n: u32, identity: &str, status: &str) -> Outcome {
        Outcome {
            url: format!("https://github.com/example/org-context/pull/{n}"),
            branch: format!("hollow/change-{n}"),
            login: if identity == "owner" {
                "theowner"
            } else {
                "someone"
            }
            .into(),
            identity: identity.into(),
            status: status.into(),
            reason: if status == "merged" {
                String::new()
            } else {
                "waiting for checks: SECRET-REASON".into()
            },
            warnings: vec![],
        }
    }

    fn open(ledger: &Path, n: u32, identity: &str, status: &str, title: &str) -> String {
        let o = outcome(n, identity, status);
        let e = Expect {
            sha: "abc123".into(),
            branch: o.branch.clone(),
            base: "main".into(),
            repo: "example/org-context".into(),
        };
        record(
            ledger,
            &o,
            &e,
            Some("s-20261115-abc"),
            title,
            &["glossary.md", "systems/parking.md"],
        );
        o.url
    }

    fn eastern() -> FixedOffset {
        FixedOffset::west_opt(5 * 3600).unwrap()
    }

    #[test]
    fn lists_only_the_merged_owner_pr_once_and_a_waiting_one_the_day_it_merges() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let ledger = dir.path().join("data/context-org-prs.jsonl");
        let state = dir.path().join("data").join(STATE_FILE);
        let tz = eastern();

        // Nothing recorded: no note, no item, no state.
        assert!(run(&vault, &ledger, &state, Utc::now(), &tz)
            .unwrap()
            .is_none());

        let merged = open(
            &ledger,
            7,
            "owner",
            "merged",
            "Add the [parking] glossary entry",
        );
        let teammate = open(&ledger, 8, "teammate", "waiting", "A teammate's change");
        let waiting = open(&ledger, 9, "owner", "waiting", "Waits on checks");

        let day1 = Utc::now() + Duration::minutes(1);
        let d = run(&vault, &ledger, &state, day1, &tz).unwrap().unwrap();
        let urls: Vec<&str> = d.listed.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(urls, [merged.as_str()]);
        assert_eq!(
            d.path,
            format!(
                "workspaces/main/outputs/org-changes/{}.md",
                day1.with_timezone(&tz).format("%Y-%m-%d")
            )
        );
        let note = std::fs::read_to_string(vault.join(&d.path)).unwrap();
        assert!(note.starts_with("# Org changes merged as you, "), "{note}");
        assert!(note.contains(&format!(
            "[Add the \\[parking\\] glossary entry](<{merged}>)"
        )));
        assert!(note.contains("`glossary.md`, `systems/parking.md`"));
        assert!(note.contains("chat `s-20261115-abc`"));
        assert!(note.contains("branch `hollow/change-7`"));
        assert!(
            !note.contains(&teammate) && !note.contains(&waiting),
            "{note}"
        );
        assert!(!note.contains("SECRET-REASON"));
        // The item links the note.
        assert_eq!(d.item.title, "Org changes merged as you");
        assert_eq!(d.item.source.as_deref(), Some(SOURCE));
        assert_eq!(d.item.action_url, Some(format!("vault:{}", d.path)));
        assert!(d.item.body.starts_with("1 pull request merged"));
        assert!(state.is_file());

        // The second run lists nothing and leaves the note alone.
        assert!(
            run(&vault, &ledger, &state, day1 + Duration::minutes(5), &tz)
                .unwrap()
                .is_none()
        );
        assert_eq!(std::fs::read_to_string(vault.join(&d.path)).unwrap(), note);

        // The waiting owner PR merges later: the next day's digest lists it.
        record_merged(&ledger, &waiting, "theowner");
        let day2 = day1 + Duration::days(1);
        let d2 = run(&vault, &ledger, &state, day2, &tz).unwrap().unwrap();
        let urls: Vec<&str> = d2.listed.iter().map(|m| m.url.as_str()).collect();
        assert_eq!(urls, [waiting.as_str()]);
        assert_ne!(d2.path, d.path);
        let note2 = std::fs::read_to_string(vault.join(&d2.path)).unwrap();
        assert!(note2.contains("[Waits on checks]"));
        assert!(!note2.contains(&merged) && !note2.contains(&teammate));
        assert!(run(&vault, &ledger, &state, day2, &tz).unwrap().is_none());
    }

    #[test]
    fn a_merge_after_the_clock_waits_and_a_lost_watermark_never_doubles_a_line() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let ledger = dir.path().join("context-org-prs.jsonl");
        let state = dir.path().join(STATE_FILE);
        let tz = eastern();
        let url = open(&ledger, 3, "owner", "merged", "");
        // A clock before the merge lists nothing yet.
        let before = Utc::now() - Duration::hours(1);
        assert!(run(&vault, &ledger, &state, before, &tz).unwrap().is_none());
        let now = Utc::now() + Duration::minutes(1);
        let d = run(&vault, &ledger, &state, now, &tz).unwrap().unwrap();
        let note = std::fs::read_to_string(vault.join(&d.path)).unwrap();
        // No title: the branch names it.
        assert!(
            note.contains(&format!("[hollow/change-3](<{url}>)")),
            "{note}"
        );
        // The watermark is lost: the same day's note still holds one line.
        std::fs::remove_file(&state).unwrap();
        assert!(run(&vault, &ledger, &state, now, &tz).unwrap().is_some());
        let again = std::fs::read_to_string(vault.join(&d.path)).unwrap();
        assert_eq!(again.matches(&url).count(), 1, "{again}");
    }

    #[test]
    fn titles_cannot_change_the_note_markup() {
        let tz = eastern();
        let m = |title: &str| Merged {
            url: "https://github.com/example/org-context/pull/4".into(),
            title: title.into(),
            branch: "hollow/x-1".into(),
            files: vec!["glossary.md".into()],
            chat: Some("s-1".into()),
            at: Utc::now(),
        };
        let l = line(&m("Fix `foo"), &tz);
        assert!(l.contains(r"[Fix \`foo]("), "{l}");
        let l = line(&m(r"ends in a backslash\"), &tz);
        assert!(l.contains(r"[ends in a backslash\\]("), "{l}");
        let l = line(&m("<b>bold</b> *a* _b_ &amp; ![i](x) and ](http://x)"), &tz);
        assert!(
            l.contains(r"[\<b\>bold\</b\> \*a\* \_b\_ \&amp; \!\[i\](x) and \](http://x)]("),
            "{l}"
        );
        assert_eq!(l.matches("](<").count(), 1, "{l}");
        assert!(l.contains("(<https://github.com/example/org-context/pull/4>)"));
    }

    #[test]
    fn a_broken_watermark_or_an_unreadable_note_stops_the_run() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let ledger = dir.path().join("context-org-prs.jsonl");
        let state = dir.path().join(STATE_FILE);
        let tz = eastern();
        open(&ledger, 5, "owner", "merged", "A change");
        let now = Utc::now() + Duration::minutes(1);
        std::fs::write(&state, "{not json").unwrap();
        let err = run(&vault, &ledger, &state, now, &tz).unwrap_err();
        assert!(err.contains("watermark"), "{err}");
        assert_eq!(std::fs::read_to_string(&state).unwrap(), "{not json");
        assert!(!vault.join(NOTE_DIR).exists());
        // The day's note exists but can't be read (a folder here): never
        // overwritten as if empty, and the watermark doesn't move.
        std::fs::remove_file(&state).unwrap();
        let note = vault.join(format!(
            "{NOTE_DIR}/{}.md",
            now.with_timezone(&tz).format("%Y-%m-%d")
        ));
        std::fs::create_dir_all(&note).unwrap();
        assert!(run(&vault, &ledger, &state, now, &tz).is_err());
        assert!(note.is_dir() && !state.exists());
    }

    #[test]
    fn a_bad_byte_in_the_record_does_not_block_the_digest() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("vault");
        let ledger = dir.path().join("context-org-prs.jsonl");
        let state = dir.path().join(STATE_FILE);
        std::fs::write(&ledger, b"\xff\xfe not a line\n").unwrap();
        let url = open(&ledger, 6, "owner", "merged", "After a bad line");
        let d = run(
            &vault,
            &ledger,
            &state,
            Utc::now() + Duration::minutes(1),
            &eastern(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(d.listed[0].url, url);
    }
}
