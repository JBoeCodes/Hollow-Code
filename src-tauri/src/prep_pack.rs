//! Meeting prep packs (overnight 10-11 A4; see `docs/PREP_PACKS.md`): one
//! page per meeting with what the person needs walking in.
//!
//! **The dossier** ([`dossier`], deterministic, from the vault): who's coming
//! (each attendee's `people/` note and role, and the open commitments with
//! them both ways), the related workspaces `meeting_prep` matched (next step,
//! open items, newest Result), and earlier notes and recordings that share
//! the meeting's distinctive words. Every item is a numbered source.
//!
//! **The agent's part**: the person's default agent (any provider, tool-free,
//! metered `prep_pack`) reads the numbered dossier and writes open questions
//! and talking points, each talking point citing source numbers; a point
//! that cites nothing real is dropped ([`parse`]).
//!
//! **The page** ([`markdown`]) is saved to the best-matching workspace's
//! `outputs/prep/<date>-<slug>.md` (else the Main agent's), and kept: asking
//! again returns it unless `regenerate`. [`prepare`] builds and saves it;
//! [`prep_pack`] (the Prepare button) and the `agenda.prepare_meeting` action
//! (the Main agent, including a check-in an hour before) find the meeting,
//! under way or in the next 24 hours, by its calendar id first.

use crate::meeting_prep::Meeting;
use chrono::{DateTime, Local};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, Weak};

/// How far ahead Prepare looks for the meeting (a meeting under way counts
/// too: `meeting_prep::PREP_LOOKBACK_HOURS`).
pub(crate) const WINDOW_HOURS: u32 = 24;
const EARLIER_NOTES: usize = 3;
const MAX_POINTS: usize = 6;
const MAX_QUESTIONS: usize = 5;

/// A tool-free completion (a fake in tests).
pub(crate) trait Writer {
    async fn complete(&self, prompt: &str) -> Result<String, String>;
    /// Self-hosted models Q10: the last completion ran on a self-hosted
    /// model, so what came of it stays off Jev.
    fn private(&self) -> bool {
        false
    }
}

/// The person's default agent, tool-free, metered `prep_pack` (to the
/// calling run when an agent asked, so a check-in's packs count in its day),
/// or the self-hosted model Settings sends Meeting recaps and prep packs to
/// (Q10), where `job` waits when it is asleep.
pub(crate) struct AgentWriter {
    pub run_id: Option<String>,
    pub label: &'static str,
    pub timeout: std::time::Duration,
    /// Self-hosted models Q10: this Prepare or Recap as a queued job.
    pub job: Option<crate::self_hosted_jobs::Job>,
    /// Self-hosted models Q10: set when a completion ran on a self-hosted
    /// model ([`Writer::private`]).
    pub routed: std::sync::atomic::AtomicBool,
}

impl AgentWriter {
    pub(crate) fn new(
        run_id: Option<String>,
        label: &'static str,
        timeout: std::time::Duration,
        job: Option<crate::self_hosted_jobs::Job>,
    ) -> Self {
        AgentWriter {
            run_id,
            label,
            timeout,
            job,
            routed: Default::default(),
        }
    }
}

impl Writer for AgentWriter {
    async fn complete(&self, prompt: &str) -> Result<String, String> {
        use crate::self_hosted_jobs::{JobKind, Routed};
        let cwd = std::env::temp_dir();
        let opts = crate::claude_runner::RunOpts {
            cwd: &cwd,
            timeout: self.timeout,
            allowed_tools: &[],
            disallowed_tools: crate::claude_runner::NO_TOOLS,
            model: None,
            debug_log: None,
            meter: match self.run_id.as_deref().filter(|r| !r.is_empty()) {
                Some(run) => crate::usage_ledger::Meter::oneshot_for_run(self.label, run),
                None => crate::usage_ledger::Meter::oneshot(self.label),
            },
            self_hosted: None,
        };
        // Review round 4 (R1): Prepare and Recap are the person's; a
        // check-in's `agenda.prepare_meeting` (with its run) is automatic.
        let asked = match self.run_id.as_deref().filter(|r| !r.is_empty()) {
            Some(_) => crate::self_hosted_jobs::Asked::Auto,
            None => crate::self_hosted_jobs::Asked::Person,
        };
        match crate::self_hosted_jobs::route(JobKind::Meetings, None, asked, || self.job.clone())
            .await
        {
            Routed::AsNow => crate::claude_runner::run_oneshot(prompt, opts).await,
            Routed::Run(run) => {
                self.routed
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                crate::self_hosted_jobs::run_on(&run, prompt, opts).await
            }
            Routed::Held(held) => Err(held.into_message()),
        }
    }

    fn private(&self) -> bool {
        self.routed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Self-hosted models Q10: a Prepare that waited for its self-hosted model
/// (the button, or a check-in's `agenda.prepare_meeting`), run once it woke.
pub(crate) async fn run_queued(
    vault: PathBuf,
    meeting_id: String,
    regenerate: bool,
    expected_hash: Option<String>,
    run_id: Option<String>,
) -> Result<(), String> {
    let writer = AgentWriter::new(
        run_id.clone(),
        "prep_pack",
        std::time::Duration::from_secs(150),
        Some(crate::self_hosted_jobs::Job::PrepPack {
            vault: vault.clone(),
            meeting_id: meeting_id.clone(),
            regenerate,
            expected_hash: expected_hash.clone(),
            run_id,
        }),
    );
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(async {
            let meeting = find_meeting(&vault, &meeting_id).await?;
            prepare_expected(
                &vault,
                &meeting,
                regenerate,
                expected_hash.as_deref(),
                &writer,
            )
            .await
            .map(drop)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// One numbered source in the dossier.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub n: usize,
    pub label: String,
    /// Vault-relative path.
    pub path: String,
    /// What the dossier says from it, one line.
    pub fact: String,
}

/// An attendee, as the pack shows them.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    pub name: String,
    pub person: Option<String>,
    pub role: Option<String>,
    /// "You owe Ana: Q3 numbers (due 2026-10-11)".
    pub open: Vec<String>,
}

/// What the vault knows for one meeting.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Dossier {
    pub attendees: Vec<Attendee>,
    pub sources: Vec<Source>,
    pub related: Vec<String>,
    pub earlier: Vec<String>,
}

/// A talking point with its sources.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Point {
    pub point: String,
    #[serde(default)]
    pub cites: Vec<usize>,
}

/// A saved pack.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrepPack {
    pub meeting_id: String,
    pub subject: String,
    pub start: String,
    /// Vault-relative path of the page.
    pub path: String,
    pub workspace_id: Option<String>,
    pub markdown: String,
    /// It was already there (not written now).
    pub kept: bool,
    pub content_hash: String,
    pub edited: bool,
}

/// Hash exactly the generated bytes; never normalize the person's whitespace.
pub(crate) fn mark_written(text: &str) -> Result<String, String> {
    let body = text
        .strip_prefix("---\n")
        .ok_or("Generated note has no frontmatter; the saved note was not replaced.")?;
    let hash = crate::vault::revision(text.as_bytes());
    Ok(format!("---\nhollow-written-sha256: {hash}\n{body}"))
}

pub(crate) fn note_edited(text: &str) -> bool {
    let Some(rest) = text.strip_prefix("---\nhollow-written-sha256: ") else {
        return true;
    };
    let Some((hash, rest)) = rest.split_once('\n') else {
        return true;
    };
    hash != crate::vault::revision(format!("---\n{rest}").as_bytes())
}

pub(crate) fn check_rewrite(text: Option<&str>, expected: Option<&str>) -> Result<(), String> {
    match (text, expected) {
        (Some(text), Some(hash)) if crate::vault::revision(text.as_bytes()) == hash => Ok(()),
        (Some(text), None) if !note_edited(text) => Ok(()),
        (None, None) => Ok(()),
        _ => Err("This note changed or has no saved writing fingerprint. Open it again and confirm replacing your edits before writing it again.".into()),
    }
}

pub(crate) fn save_generated(
    vault: &Path,
    rel: &str,
    text: &str,
    previous: Option<&str>,
) -> Result<(), String> {
    let expected = previous
        .map(crate::vault::Expected::Content)
        .unwrap_or(crate::vault::Expected::Absent);
    match crate::vault::write_note_expecting(vault, rel, text, expected)? {
        crate::vault::CasOutcome::Written(_) => Ok(()),
        crate::vault::CasOutcome::Conflict(_) => Err("The note changed while it was being written. Your current note was kept. Open it again before retrying.".into()),
    }
}

fn one_line(text: &str, max: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(max)
        .collect()
}

/// The `people/` note for an attendee: the full name (or the file stem), else
/// the one person whose first name it is.
fn person_for<'a>(
    people: &'a [crate::vault_people::Person],
    name: &str,
) -> Option<&'a crate::vault_people::Person> {
    let want = name.trim().to_lowercase();
    if want.is_empty() {
        return None;
    }
    let stem = |p: &crate::vault_people::Person| {
        Path::new(&p.path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default()
    };
    if let Some(p) = people
        .iter()
        .find(|p| p.name.to_lowercase() == want || stem(p) == want)
    {
        return Some(p);
    }
    // A one-word name ("Pat") may be the one person with that first name;
    // "Pat Jones" with no note of their own is nobody's.
    let mut words = want.split_whitespace();
    let first = words.next()?;
    if words.next().is_some() {
        return None;
    }
    let mut by_first = people
        .iter()
        .filter(|p| p.name.to_lowercase().split_whitespace().next() == Some(first));
    match (by_first.next(), by_first.next()) {
        (Some(one), None) => Some(one),
        _ => None,
    }
}

/// Whether a commitment is with this attendee (their note, or their name).
fn with(c: &crate::vault_commitments::Commitment, name: &str, person: Option<&str>) -> bool {
    if let (Some(p), Some(cp)) = (person, c.person_path.as_deref()) {
        if p == cp {
            return true;
        }
    }
    // By full name only: a counterparty "Pat" isn't every Pat.
    let who = c.counterparty.trim().to_lowercase();
    !who.is_empty() && who == name.trim().to_lowercase()
}

/// Words shorter than this must match whole in a note (ENG-39): the vault
/// search lets a 1 to 3 letter term begin a word, so `nat` found "native".
const SHORT_WORD: usize = 4;
/// Invite-body words searched, at most (the longest first).
const BODY_WORDS: usize = 3;

/// The part of an invite body worth searching: cut at the first line of
/// underscores, "Microsoft Teams meeting" or "Join Zoom" (what follows is the
/// dial-in block and links), and without URL tokens.
fn invite_text(body: &str) -> String {
    let mut kept = String::new();
    for line in body.lines() {
        let low = line.trim().to_lowercase();
        if (low.len() >= 3 && low.chars().all(|c| c == '_'))
            || low.contains("microsoft teams meeting")
            || low.contains("join zoom")
        {
            break;
        }
        kept.push_str(line);
        kept.push('\n');
    }
    kept.split_whitespace()
        .filter(|t| {
            let t = t.to_lowercase();
            !(t.contains("://") || t.starts_with("www.") || t.contains('@'))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One search for earlier notes: the vault-search text, and the short words in
/// it that a hit must contain as whole words.
struct NoteQuery {
    text: String,
    whole: Vec<String>,
}

/// The searches that find a meeting's earlier notes, best first (ENG-39): the
/// title's distinctive words; each attendee's full name as an exact phrase (a
/// first name alone says nothing); and the invite body's longest distinctive
/// words. Every query is its own search, so a body word never makes a title
/// search miss. Short words stay in a query, but a hit must hold them whole.
fn note_queries(meeting: &Meeting, me: &str) -> Vec<NoteQuery> {
    let query_of = |words: Vec<String>| {
        (!words.is_empty()).then(|| NoteQuery {
            whole: words
                .iter()
                .filter(|w| w.chars().count() < SHORT_WORD)
                .cloned()
                .collect(),
            text: words.join(" "),
        })
    };
    let mut out = Vec::new();
    out.extend(query_of(
        crate::meeting_prep::words(&meeting.subject)
            .into_iter()
            .collect(),
    ));
    let mut names: Vec<&String> = meeting.attendees.iter().flatten().collect();
    names.extend(meeting.organizer.as_ref());
    let mut done: Vec<String> = Vec::new();
    for name in names {
        let plain: String = name
            .split('(')
            .next()
            .unwrap_or("")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        // "Ruiz, Ana" reads as "Ana Ruiz"; a name of one word is not searched.
        let plain = match plain.split_once(',') {
            Some((last, first)) if !first.trim().is_empty() => {
                format!("{} {}", first.trim(), last.trim())
            }
            _ => plain,
        };
        if plain.split(' ').filter(|p| p.chars().count() >= 2).count() < 2
            || crate::extract::is_me(&plain, me)
            || plain.contains('@')
            || plain.starts_with('+')
            || plain.contains('"')
            || done.iter().any(|d| d.eq_ignore_ascii_case(&plain))
        {
            continue;
        }
        done.push(plain.clone());
        out.push(NoteQuery {
            text: format!("\"{plain}\""),
            whole: Vec::new(),
        });
    }
    if let Some(body) = &meeting.body {
        let mut words: Vec<String> = crate::meeting_prep::words(&invite_text(body))
            .into_iter()
            .collect();
        words.sort_by_key(|w| std::cmp::Reverse(w.chars().count()));
        words.retain(|w| w.chars().count() > SHORT_WORD);
        words.truncate(BODY_WORDS);
        // One search per word: the invite's words need not all be in one note,
        // and each must stand as a whole word in the notes it pulls.
        out.extend(words.into_iter().map(|w| NoteQuery {
            whole: vec![w.clone()],
            text: w,
        }));
    }
    out
}

/// Whether the note at `rel` holds `word` as a whole word in its name or body.
fn names_word(vault: &Path, rel: &str, word: &str) -> bool {
    let Ok(full) = crate::vault::safe_join(vault, rel) else {
        return false;
    };
    let name = Path::new(rel)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase().replace(['-', '_'], " "))
        .unwrap_or_default();
    if crate::meeting_prep::bounded(&name, word) {
        return true;
    }
    std::fs::read_to_string(full)
        .is_ok_and(|text| crate::meeting_prep::bounded(&text.to_lowercase(), word))
}

/// The dossier for `meeting` (see the module doc).
pub(crate) fn dossier(vault: &Path, meeting: &Meeting) -> Dossier {
    let people = crate::vault_people::list(vault);
    let commitments: Vec<_> = crate::vault_commitments::list(vault)
        .into_iter()
        .filter(|c| c.status == "open")
        .collect();
    let mut sources: Vec<Source> = Vec::new();
    let add = |label: String, path: String, fact: String, sources: &mut Vec<Source>| -> usize {
        if let Some(s) = sources.iter().find(|s| s.path == path && s.fact == fact) {
            return s.n;
        }
        let n = sources.len() + 1;
        sources.push(Source {
            n,
            label,
            path,
            fact: one_line(&fact, 240),
        });
        n
    };
    let mut names: Vec<String> = meeting.attendees.clone().unwrap_or_default();
    if let Some(o) = &meeting.organizer {
        if !names.iter().any(|n| n.eq_ignore_ascii_case(o)) {
            names.insert(0, o.clone());
        }
    }
    names.retain(|n| !(n.starts_with('+') && n.ends_with(" more")));
    let mut attendees = Vec::new();
    for name in names {
        let person = person_for(&people, &name);
        if let Some(p) = person {
            let fact = [
                p.role.as_str(),
                p.relationship.as_str(),
                p.description.as_str(),
            ]
            .iter()
            .filter(|s| !s.trim().is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join("; ");
            add(
                format!("{} (people note)", p.name),
                p.path.clone(),
                if fact.is_empty() {
                    p.name.clone()
                } else {
                    fact
                },
                &mut sources,
            );
        }
        let mut open = Vec::new();
        for c in commitments
            .iter()
            .filter(|c| with(c, &name, person.map(|p| p.path.as_str())))
        {
            let line = if c.direction == "owed_by_me" {
                format!("You owe {}: {}", c.counterparty, c.title)
            } else {
                format!("{} owes you: {}", c.counterparty, c.title)
            };
            let line = match &c.due {
                Some(due) => format!("{line} (due {})", due.chars().take(10).collect::<String>()),
                None => line,
            };
            add(
                "Commitment".into(),
                c.vault_path.clone(),
                line.clone(),
                &mut sources,
            );
            open.push(line);
        }
        attendees.push(Attendee {
            name,
            person: person.map(|p| p.path.clone()),
            role: person
                .map(|p| p.role.clone())
                .filter(|r| !r.trim().is_empty()),
            open,
        });
    }
    let mut related = Vec::new();
    for m in &meeting.matches {
        let brief = format!("workspaces/{}/WORKSPACE.md", m.workspace_id);
        let mut fact = format!("Workspace “{}”", m.title);
        if let Some(step) = &m.next_step {
            fact.push_str(&format!("; next step: {step}"));
        }
        if let Some(n) = m.open_items {
            fact.push_str(&format!("; {n} open plan items"));
        }
        let n = add(m.title.clone(), brief, fact.clone(), &mut sources);
        related.push(format!("{fact} [{n}]"));
        if let Some(result) = &m.latest_result {
            let n = add(
                format!("Newest Result in {}", m.title),
                result.clone(),
                format!("Newest Result: {result}"),
                &mut sources,
            );
            related.push(format!("Newest Result in “{}”: `{result}` [{n}]", m.title));
        }
    }
    let mut earlier = Vec::new();
    let mut seen_paths: Vec<String> = Vec::new();
    let me = crate::extract::me(vault);
    for query in note_queries(meeting, &me) {
        if earlier.len() >= EARLIER_NOTES {
            break;
        }
        for hit in crate::vault::search(vault, &query.text, 12, None)
            .into_iter()
            .filter(|h| !h.path.contains("/outputs/prep/") && !h.path.starts_with("people/"))
            .filter(|h| query.whole.iter().all(|w| names_word(vault, &h.path, w)))
        {
            if earlier.len() >= EARLIER_NOTES {
                break;
            }
            if seen_paths.contains(&hit.path) {
                continue;
            }
            seen_paths.push(hit.path.clone());
            let n = add(
                hit.name.trim_end_matches(".md").to_string(),
                hit.path.clone(),
                if hit.snippet.trim().is_empty() {
                    hit.name.clone()
                } else {
                    hit.snippet.clone()
                },
                &mut sources,
            );
            earlier.push(format!(
                "`{}`: {} [{n}]",
                hit.path,
                one_line(&hit.snippet, 160)
            ));
        }
    }
    Dossier {
        attendees,
        sources,
        related,
        earlier,
    }
}

/// The agent's prompt: the meeting and the numbered dossier.
pub(crate) fn prompt(meeting: &Meeting, d: &Dossier) -> String {
    let sources: Vec<String> = d
        .sources
        .iter()
        .map(|s| format!("[{}] {} ({}): {}", s.n, s.label, s.path, s.fact))
        .collect();
    format!(
        "You prepare the person for a meeting in Hollow, their local workspace app. Reply with ONE JSON object and nothing else:\n\
         {{\"openQuestions\":[\"<a question the person should get answered in the meeting>\"],\"talkingPoints\":[{{\"point\":\"<one thing to raise, plain and specific>\",\"cites\":[<source numbers>]}}]}}\n\
         At most {MAX_QUESTIONS} questions and {MAX_POINTS} talking points. Every talking point cites at least one source number below; use only what the sources say, never invent names, numbers or history. Never suggest sending anything outside the meeting.\n\n\
         Meeting and numbered sources:\n{dossier}",
        dossier = crate::untrusted::wrap(
            crate::untrusted::Origin::Meeting,
            "meeting dossier",
            &format!(
                "Meeting: {subject}, {start}{organizer}.\n{sources}",
                subject = meeting.subject,
                start = meeting.start,
                organizer = meeting
                    .organizer
                    .as_deref()
                    .map(|o| format!(", organized by {o}"))
                    .unwrap_or_default(),
                sources = sources.join("\n"),
            ),
        ),
    )
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Reply {
    #[serde(default)]
    open_questions: Vec<String>,
    #[serde(default)]
    talking_points: Vec<Point>,
}

/// The agent's questions and points; a point citing no real source is
/// dropped, and cites outside `1..=sources` are removed.
pub(crate) fn parse(raw: &str, sources: usize) -> Result<(Vec<String>, Vec<Point>), String> {
    let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) else {
        return Err("The agent didn't write talking points.".into());
    };
    if end < start {
        return Err("The agent didn't write talking points.".into());
    }
    let reply: Reply = serde_json::from_str(&raw[start..=end])
        .map_err(|e| format!("The agent's talking points weren't readable ({e})."))?;
    let questions = reply
        .open_questions
        .iter()
        .map(|q| one_line(q, 240))
        .filter(|q| !q.is_empty())
        .take(MAX_QUESTIONS)
        .collect();
    let points = reply
        .talking_points
        .into_iter()
        .map(|p| Point {
            point: one_line(&p.point, 300),
            cites: {
                let mut seen = std::collections::HashSet::new();
                p.cites
                    .into_iter()
                    .filter(|n| (1..=sources).contains(n) && seen.insert(*n))
                    .collect()
            },
        })
        .filter(|p| !p.point.is_empty() && !p.cites.is_empty())
        .take(MAX_POINTS)
        .collect();
    Ok((questions, points))
}

/// The pack's page.
pub(crate) fn markdown(
    meeting: &Meeting,
    d: &Dossier,
    questions: &[String],
    points: &[Point],
    workspace: Option<&str>,
    now: DateTime<Local>,
) -> String {
    let when = DateTime::parse_from_rfc3339(&meeting.start)
        .map(|t| {
            t.with_timezone(&Local)
                .format("%A %-d %B, %-I:%M %p")
                .to_string()
        })
        .unwrap_or_else(|_| meeting.start.clone());
    let mut out = format!(
        "---\ntype: prep-pack\nmeeting: {}\nmeeting-id: {}\nstart: {}\nworkspace: {}\ngenerated: {}\n---\n\n# Prep: {}\n\n{}",
        serde_json::to_string(&meeting.subject).unwrap_or_default(),
        serde_json::to_string(&meeting.id).unwrap_or_default(),
        meeting.start,
        workspace.unwrap_or("none"),
        now.to_rfc3339(),
        meeting.subject,
        when
    );
    if let Some(o) = &meeting.organizer {
        out.push_str(&format!(" · organized by {o}"));
    }
    if let Some(l) = meeting.location.as_deref().filter(|l| !l.trim().is_empty()) {
        out.push_str(&format!(" · {l}"));
    }
    out.push_str("\n\n## Who's coming\n\n");
    if d.attendees.is_empty() {
        out.push_str(if meeting.attendees.is_none() {
            "Attendees couldn't be read from the calendar.\n"
        } else {
            "No one else is listed.\n"
        });
    }
    for a in &d.attendees {
        out.push_str(&format!("- **{}**", a.name));
        if let Some(role) = &a.role {
            out.push_str(&format!(", {role}"));
        }
        if let Some(p) = &a.person {
            out.push_str(&format!(" · [[{}]]", p.trim_end_matches(".md")));
        }
        out.push('\n');
        for line in &a.open {
            out.push_str(&format!("  - {line}\n"));
        }
    }
    out.push_str("\n## Related work\n\n");
    if d.related.is_empty() {
        out.push_str("No workspace matched this meeting.\n");
    }
    for r in &d.related {
        out.push_str(&format!("- {r}\n"));
    }
    if !d.earlier.is_empty() {
        out.push_str("\n## Earlier notes\n\n");
        for e in &d.earlier {
            out.push_str(&format!("- {e}\n"));
        }
    }
    out.push_str("\n## Open questions\n\n");
    if questions.is_empty() {
        out.push_str("None suggested.\n");
    }
    for q in questions {
        out.push_str(&format!("- {q}\n"));
    }
    out.push_str("\n## Talking points\n\n");
    if points.is_empty() {
        out.push_str("None the sources support.\n");
    }
    for p in points {
        let cites: Vec<String> = p.cites.iter().map(|n| format!("[{n}]")).collect();
        out.push_str(&format!("- {} {}\n", p.point, cites.join("")));
    }
    if !d.sources.is_empty() {
        out.push_str("\n## Sources\n\n");
        for s in &d.sources {
            out.push_str(&format!("{}. {} · `{}`\n", s.n, s.label, s.path));
        }
    }
    out.push_str("\n_Talking points and questions are written by your agent from the sources above; check them before you rely on them._\n");
    out
}

/// `2026-10-11-atlas-pricing-review`.
pub(crate) fn file_stem(meeting: &Meeting) -> String {
    let day = DateTime::parse_from_rfc3339(&meeting.start)
        .map(|t| t.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| Local::now().format("%Y-%m-%d").to_string());
    let mut slug: String = meeting
        .subject
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug: String = slug.trim_matches('-').chars().take(60).collect();
    let slug = slug.trim_end_matches('-');
    // Two meetings with one subject on one day (two 1:1s) are two packs.
    let id = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(meeting.id.as_bytes()))[..6].to_string()
    };
    format!(
        "{day}-{}-{id}",
        if slug.is_empty() { "meeting" } else { slug }
    )
}

/// Self-hosted models Q10: whether a Prepare (`folder` `prep`) or Recap
/// (`recaps`) that waited for its self-hosted model would still write
/// anything: a rewrite always does, otherwise only while none is kept.
pub(crate) fn still_wanted(vault: &Path, id: &str, folder: &str, regenerate: bool) -> bool {
    regenerate || find_kept_in(vault, id, folder).is_none()
}

/// A kept pack for calendar id `id`, found by its frontmatter in any
/// workspace's `outputs/prep/` (so a meeting that has started, which the
/// calendar read no longer lists, still opens). `(vault path, text,
/// workspace id)`.
pub(crate) fn find_kept(vault: &Path, id: &str) -> Option<(String, String, Option<String>)> {
    find_kept_in(vault, id, "prep")
}

pub(crate) fn find_kept_in(
    vault: &Path,
    id: &str,
    kind: &str,
) -> Option<(String, String, Option<String>)> {
    if id.trim().is_empty() {
        return None;
    }
    let want = format!("meeting-id: {}", serde_json::to_string(id).ok()?);
    let mut folders: Vec<(PathBuf, Option<String>)> = crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .map(|w| (vault.join(&w.path), Some(w.id)))
        .collect();
    if let Ok(main) = crate::workspaces::summary(vault, crate::workspaces::MAIN_AGENT_ID) {
        folders.push((vault.join(&main.path), Some(main.id)));
    }
    for (folder, workspace) in folders {
        let Ok(entries) = std::fs::read_dir(folder.join("outputs").join(kind)) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let Ok(rel) = path.strip_prefix(vault) else {
                continue;
            };
            let rel = rel.to_string_lossy().to_string();
            let Ok(safe) = crate::vault::safe_join(vault, &rel) else {
                continue;
            };
            let Ok(text) = std::fs::read_to_string(safe) else {
                continue;
            };
            if text.lines().take(8).any(|l| l.trim() == want) {
                return Some((rel, text, workspace));
            }
        }
    }
    None
}

/// The workspace, of any status, whose whole title is in `subject` as whole
/// words. Built-in workspaces and titles with no distinctive word (`Team
/// sync`) never count. Deterministic: a current workspace beats a finished or
/// archived one, then the longer title, then the lower id.
fn workspace_named_in(vault: &Path, subject: &str) -> Option<String> {
    let subject = subject.to_lowercase();
    crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|w| !crate::workspaces::is_builtin(&w.id))
        .filter(|w| !crate::meeting_prep::words(&w.title).is_empty())
        .filter(|w| crate::meeting_prep::bounded(&subject, &w.title.trim().to_lowercase()))
        .min_by(|a, b| {
            let key = |w: &crate::workspaces::Workspace| {
                (
                    !crate::workspaces::is_current_status(&w.status),
                    std::cmp::Reverse(w.title.trim().chars().count()),
                    w.id.clone(),
                )
            };
            key(a).cmp(&key(b))
        })
        .map(|w| w.id)
}

/// Where the pack goes: the best-ranked workspace when it is current, else
/// (none ranked, or the ranked one is not current) the workspace the meeting
/// title names, whatever its status (ENG-39), else the ranked one, else the
/// Main agent's. `(folder, workspace id)`.
pub(crate) fn home_for(
    vault: &Path,
    meeting: &Meeting,
) -> Result<(PathBuf, Option<String>), String> {
    let ranked = meeting
        .matches
        .iter()
        .find_map(|m| crate::workspaces::summary(vault, &m.workspace_id).ok());
    if let Some(w) = &ranked {
        if crate::workspaces::is_current_status(&w.status) {
            return Ok((vault.join(&w.path), Some(w.id.clone())));
        }
    }
    if let Some(id) = workspace_named_in(vault, &meeting.subject) {
        if let Ok(w) = crate::workspaces::summary(vault, &id) {
            return Ok((vault.join(w.path), Some(w.id)));
        }
    }
    if let Some(w) = ranked {
        return Ok((vault.join(w.path), Some(w.id)));
    }
    let main = crate::workspaces::main_agent_workspace(vault)?;
    Ok((vault.join(main.path), Some(main.id)))
}

type PrepKey = (PathBuf, String);
type PrepResult = Result<PrepPack, String>;

struct PrepOutcome {
    snapshot: Meeting,
    regenerate: bool,
    expected: Option<String>,
    result: PrepResult,
}

/// One app-process flight per canonical vault and meeting. Weak entries
/// retain no pack after the callers finish; the next call prunes dead keys.
#[derive(Default)]
struct PrepFlight {
    finished: AtomicU64,
    result: tokio::sync::Mutex<Option<PrepOutcome>>,
}
static PREP_FLIGHTS: LazyLock<Mutex<HashMap<PrepKey, Weak<PrepFlight>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn prep_flight(vault: &Path, meeting: &str) -> Result<Arc<PrepFlight>, String> {
    let key = (
        vault
            .canonicalize()
            .map_err(|e| format!("prep pack: {e}"))?,
        meeting.to_string(),
    );
    let mut entries = PREP_FLIGHTS.lock().unwrap_or_else(|e| e.into_inner());
    entries.retain(|_, v| v.strong_count() > 0);
    if let Some(flight) = entries.get(&key).and_then(Weak::upgrade) {
        return Ok(flight);
    }
    let flight = Arc::new(PrepFlight::default());
    entries.insert(key, Arc::downgrade(&flight));
    Ok(flight)
}

/// Build and save `meeting`'s pack, or return the saved one (see the module
/// doc).
pub(crate) async fn prepare(
    vault: &Path,
    meeting: &Meeting,
    regenerate: bool,
    writer: &impl Writer,
) -> Result<PrepPack, String> {
    prepare_expected(vault, meeting, regenerate, None, writer).await
}

async fn prepare_expected(
    vault: &Path,
    meeting: &Meeting,
    regenerate: bool,
    expected: Option<&str>,
    writer: &impl Writer,
) -> Result<PrepPack, String> {
    let flight = prep_flight(vault, &meeting.id)?;
    let started = flight.finished.load(Ordering::SeqCst);
    let mut snapshot = meeting.clone();
    snapshot.minutes_until = 0; // a ticking countdown is not changed source data
    let mut previous = flight.result.lock().await;
    if flight.finished.load(Ordering::SeqCst) != started {
        if let Some(done) = previous.as_ref() {
            let freshly_written = done.result.as_ref().is_ok_and(|p| !p.kept);
            if done.snapshot == snapshot
                && done.expected.as_deref() == expected
                && (!regenerate || done.regenerate || freshly_written)
            {
                // A failed rewrite must not hide an already saved pack from Prepare.
                let kept_fallback =
                    !regenerate && done.result.is_err() && find_kept(vault, &meeting.id).is_some();
                if !kept_fallback {
                    return done.result.clone();
                }
            }
        }
    }
    // Cancellation drops this guard without completing the generation, so
    // a waiting caller can take over. Concurrent failures share one error;
    // a later independent request may retry or deliberately regenerate.
    let result = prepare_one(vault, meeting, regenerate, expected, writer).await;
    *previous = Some(PrepOutcome {
        snapshot,
        regenerate,
        expected: expected.map(str::to_owned),
        result: result.clone(),
    });
    flight.finished.fetch_add(1, Ordering::SeqCst);
    result
}

async fn prepare_one(
    vault: &Path,
    meeting: &Meeting,
    regenerate: bool,
    expected: Option<&str>,
    writer: &impl Writer,
) -> PrepResult {
    let kept = find_kept(vault, &meeting.id);
    if !regenerate {
        if let Some((path, markdown, workspace_id)) = kept.as_ref() {
            return Ok(PrepPack {
                meeting_id: meeting.id.clone(),
                subject: meeting.subject.clone(),
                start: meeting.start.clone(),
                path: path.clone(),
                workspace_id: workspace_id.clone(),
                markdown: markdown.clone(),
                kept: true,
                content_hash: crate::vault::revision(markdown.as_bytes()),
                edited: note_edited(markdown),
            });
        }
    }
    let previous = kept.as_ref().map(|(_, text, _)| text.clone());
    check_rewrite(previous.as_deref(), expected)?;
    // find_meeting already awaits the bounded Calendar attendee read. Do
    // not spend or save a thin pack if coverage is still unavailable.
    // Some([]) is known empty and remains valid; existing saved packs stay.
    if meeting.attendees.is_none() {
        return Err(
            "Couldn't read attendee information. Check Calendar access, then try again.".into(),
        );
    }
    // An explicit rewrite replaces the existing artifact in place. A new
    // workspace match or event date must not leave a second stale copy.
    let (path, rel, workspace_id) = if let Some((rel, _, workspace_id)) = kept {
        (crate::vault::safe_join(vault, &rel)?, rel, workspace_id)
    } else {
        let (folder, workspace_id) = home_for(vault, meeting)?;
        let path = folder
            .join("outputs")
            .join("prep")
            .join(format!("{}.md", file_stem(meeting)));
        let rel = path
            .strip_prefix(vault)
            .map_err(|e| format!("prep pack: {e}"))?
            .to_string_lossy()
            .to_string();
        (path, rel, workspace_id)
    };
    let pack = |markdown: String, kept: bool| PrepPack {
        meeting_id: meeting.id.clone(),
        subject: meeting.subject.clone(),
        start: meeting.start.clone(),
        path: rel.clone(),
        workspace_id: workspace_id.clone(),
        content_hash: crate::vault::revision(markdown.as_bytes()),
        edited: note_edited(&markdown),
        markdown,
        kept,
    };
    let d = dossier(vault, meeting);
    let raw = writer.complete(&prompt(meeting, &d)).await?;
    let (questions, points) = parse(&raw, d.sources.len())?;
    let text = mark_written(&markdown(
        meeting,
        &d,
        &questions,
        &points,
        workspace_id.as_deref(),
        Local::now(),
    ))?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("prep pack: {e}"))?;
    }
    save_generated(vault, &rel, &text, previous.as_deref())?;
    Ok(pack(text, false))
}

/// The meeting with calendar id `id` that hasn't ended: under way now, or in
/// the next [`WINDOW_HOURS`] (P0-26).
pub(crate) async fn find_meeting(vault: &Path, id: &str) -> Result<Meeting, String> {
    crate::meeting_prep::for_prep(vault, id, WINDOW_HOURS)
        .await
        .map_err(|calendar| format!("The calendar is {calendar}."))?
        .ok_or_else(|| {
            "That meeting has ended, isn't in the next 24 hours, or was cancelled.".into()
        })
}

/// Whether a pack exists for `meeting` (the check-in's "no prep pack yet").
pub(crate) fn exists(vault: &Path, meeting: &Meeting) -> bool {
    find_kept(vault, &meeting.id).is_some()
}

/// Prepare: the meeting's pack, written now or kept from before.
#[tauri::command]
pub async fn prep_pack(
    state: tauri::State<'_, crate::commands::AppState>,
    meeting_id: String,
    regenerate: bool,
    expected_hash: Option<String>,
) -> Result<PrepPack, String> {
    let vault = state.config.vault_path.clone();
    if !regenerate {
        let (v, id) = (vault.clone(), meeting_id.clone());
        let kept = tauri::async_runtime::spawn_blocking(move || find_kept(&v, &id))
            .await
            .map_err(|e| e.to_string())?;
        if let Some((path, markdown, workspace_id)) = kept {
            return Ok(PrepPack {
                meeting_id,
                subject: String::new(),
                start: String::new(),
                path,
                workspace_id,
                content_hash: crate::vault::revision(markdown.as_bytes()),
                edited: note_edited(&markdown),
                markdown,
                kept: true,
            });
        }
    }
    let meeting = find_meeting(&vault, &meeting_id).await?;
    let job = crate::self_hosted_jobs::Job::PrepPack {
        vault: vault.clone(),
        meeting_id: meeting_id.clone(),
        regenerate,
        expected_hash: expected_hash.clone(),
        run_id: None,
    };
    tauri::async_runtime::spawn_blocking(move || {
        tauri::async_runtime::block_on(prepare_expected(
            &vault,
            &meeting,
            regenerate,
            expected_hash.as_deref(),
            &AgentWriter::new(
                None,
                "prep_pack",
                std::time::Duration::from_secs(150),
                Some(job),
            ),
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meeting_prep::Match;
    use std::cell::RefCell;

    struct Fake {
        reply: String,
        prompts: RefCell<Vec<String>>,
    }

    impl Writer for Fake {
        async fn complete(&self, prompt: &str) -> Result<String, String> {
            self.prompts.borrow_mut().push(prompt.to_string());
            Ok(self.reply.clone())
        }
    }

    fn meeting(workspace: Option<&str>) -> Meeting {
        Meeting {
            id: "evt-1".into(),
            subject: "Atlas pricing review".into(),
            start: "2026-10-11T18:00:00+00:00".into(),
            end: "2026-10-11T19:00:00+00:00".into(),
            minutes_until: 45,
            organizer: Some("Pat Smith".into()),
            location: Some("Teams".into()),
            join_url: None,
            person: None,
            attendees: Some(vec!["Ana Ruiz".into()]),
            body: None,
            matches: workspace
                .map(|id| {
                    vec![Match {
                        workspace_id: id.into(),
                        title: "Atlas".into(),
                        because: vec!["atlas".into()],
                        project: None,
                        people: vec![],
                        next_step: Some("Finalize Q3 pricing".into()),
                        open_items: Some(3),
                        latest_result: None,
                        last_activity_at: None,
                    }]
                })
                .unwrap_or_default(),
        }
    }

    fn vault_with_people() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("people")).unwrap();
        std::fs::write(
            v.join("people/Pat Smith.md"),
            "---\ntitle: Pat Smith\nrole: CFO\n---\n\nDave runs finance.\n",
        )
        .unwrap();
        dir
    }

    const REPLY: &str = r#"{"openQuestions":["Is the Q3 floor price final?"],"talkingPoints":[{"point":"Confirm the Q3 pricing next step","cites":[2]},{"point":"Unsupported claim","cites":[]},{"point":"Out of range","cites":[99]}]}"#;

    struct YieldWriter {
        calls: std::cell::Cell<usize>,
        fail: bool,
    }
    impl Writer for YieldWriter {
        async fn complete(&self, _prompt: &str) -> Result<String, String> {
            self.calls.set(self.calls.get() + 1);
            tokio::task::yield_now().await;
            if self.fail {
                Err("synthetic provider failure".into())
            } else {
                Ok(REPLY.into())
            }
        }
    }

    #[test]
    fn a8_cancellation_releases_the_flight_for_a_retry() {
        use std::future::Future;
        struct PendingWriter;
        impl Writer for PendingWriter {
            async fn complete(&self, _: &str) -> Result<String, String> {
                std::future::pending().await
            }
        }
        let dir = vault_with_people();
        let m = meeting(None);
        tauri::async_runtime::block_on(async {
            let mut pending = Box::pin(prepare(dir.path(), &m, false, &PendingWriter));
            std::future::poll_fn(|cx| {
                assert!(pending.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            drop(pending);
            let writer = YieldWriter {
                calls: Default::default(),
                fail: false,
            };
            let retried = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                prepare(dir.path(), &m, false, &writer),
            )
            .await;
            assert!(retried.unwrap().is_ok());
            assert_eq!(writer.calls.get(), 1);
        });
    }

    #[test]
    fn a8_other_meetings_and_vaults_do_not_wait_on_each_other() {
        struct BarrierWriter {
            barrier: tokio::sync::Barrier,
        }
        impl Writer for BarrierWriter {
            async fn complete(&self, _: &str) -> Result<String, String> {
                self.barrier.wait().await;
                Ok(REPLY.into())
            }
        }
        let a = vault_with_people();
        let b = vault_with_people();
        let m = meeting(None);
        let other = Meeting {
            id: "evt-other".into(),
            ..m.clone()
        };
        let writer = BarrierWriter {
            barrier: tokio::sync::Barrier::new(3),
        };
        tauri::async_runtime::block_on(async {
            let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), async {
                tokio::join!(
                    prepare(a.path(), &m, false, &writer),
                    prepare(a.path(), &other, false, &writer),
                    prepare(b.path(), &m, false, &writer)
                )
            })
            .await
            .expect("unrelated flights must all reach the writer together");
            assert!(outcome.0.is_ok() && outcome.1.is_ok() && outcome.2.is_ok());
        });
    }

    #[test]
    fn a8review_waiting_rewrite_does_not_inherit_a_kept_result() {
        use std::future::Future;
        let dir = vault_with_people();
        let v = dir.path();
        let m = meeting(None);
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        tauri::async_runtime::block_on(async {
            let mut kept = prepare(v, &m, false, &writer).await.unwrap();
            kept.kept = true;
            let flight = prep_flight(v, &m.id).unwrap();
            let mut result = flight.result.lock().await;
            let mut waiting = Box::pin(prepare(v, &m, true, &writer));
            std::future::poll_fn(|cx| {
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let mut snapshot = m.clone();
            snapshot.minutes_until = 0;
            *result = Some(PrepOutcome {
                snapshot,
                regenerate: false,
                expected: None,
                result: Ok(kept),
            });
            flight.finished.fetch_add(1, Ordering::SeqCst);
            drop(result);
            assert!(!waiting.await.unwrap().kept);
            assert_eq!(writer.calls.get(), 2);
        });
    }

    #[test]
    fn a8review_new_attendees_do_not_inherit_an_older_snapshot_error() {
        use std::future::Future;
        let dir = vault_with_people();
        let v = dir.path();
        let m = meeting(None);
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        tauri::async_runtime::block_on(async {
            let flight = prep_flight(v, &m.id).unwrap();
            let mut result = flight.result.lock().await;
            let mut waiting = Box::pin(prepare(v, &m, false, &writer));
            std::future::poll_fn(|cx| {
                assert!(waiting.as_mut().poll(cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            let mut snapshot = m.clone();
            snapshot.minutes_until = 0;
            snapshot.attendees = None;
            *result = Some(PrepOutcome {
                snapshot,
                regenerate: false,
                expected: None,
                result: Err("Attendee information unavailable".into()),
            });
            flight.finished.fetch_add(1, Ordering::SeqCst);
            drop(result);
            assert!(waiting.await.is_ok());
            assert_eq!(writer.calls.get(), 1);
        });
    }

    #[test]
    fn rewrite_refuses_an_edited_pack_before_paid_work() {
        let dir = vault_with_people();
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let m = meeting(None);
        let pack = tauri::async_runtime::block_on(prepare(dir.path(), &m, false, &writer)).unwrap();
        let edited = format!("{}\nMy own follow-up\n", pack.markdown);
        std::fs::write(dir.path().join(&pack.path), &edited).unwrap();
        let result = tauri::async_runtime::block_on(prepare(dir.path(), &m, true, &writer));
        assert!(
            result.is_err(),
            "rewriting edited notes requires explicit approval"
        );
        assert_eq!(writer.calls.get(), 1, "refuse before a second model call");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&pack.path)).unwrap(),
            edited
        );
        let hash = crate::vault::revision(edited.as_bytes());
        let rewritten = tauri::async_runtime::block_on(prepare_expected(
            dir.path(),
            &m,
            true,
            Some(&hash),
            &writer,
        ))
        .unwrap();
        assert!(!rewritten.edited);
        assert_eq!(writer.calls.get(), 2);
        assert!(tauri::async_runtime::block_on(prepare_expected(
            dir.path(),
            &m,
            true,
            Some(&hash),
            &writer
        ))
        .is_err());
        assert_eq!(writer.calls.get(), 2, "a stale confirmation spends nothing");
    }

    #[test]
    fn edits_during_generation_survive_the_final_write() {
        struct EditingWriter {
            path: std::path::PathBuf,
        }
        impl Writer for EditingWriter {
            async fn complete(&self, _: &str) -> Result<String, String> {
                std::fs::write(&self.path, "A concurrent edit").unwrap();
                Ok(REPLY.into())
            }
        }
        let v = vault_with_people();
        let m = meeting(None);
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let pack = tauri::async_runtime::block_on(prepare(v.path(), &m, false, &writer)).unwrap();
        let path = v.path().join(&pack.path);
        let result = tauri::async_runtime::block_on(prepare(
            v.path(),
            &m,
            true,
            &EditingWriter { path: path.clone() },
        ));
        assert!(result.unwrap_err().contains("current note was kept"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "A concurrent edit");
    }

    #[test]
    fn malformed_generated_frontmatter_does_not_panic() {
        assert!(std::panic::catch_unwind(|| mark_written("# Missing frontmatter")).is_ok());
        assert!(mark_written("# Missing frontmatter")
            .unwrap_err()
            .contains("not replaced"));
    }

    #[test]
    fn unknown_provenance_and_whitespace_edits_need_confirmation() {
        let old = "---\nmeeting-id: old\n---\n\nKeep me\n";
        assert!(note_edited(old));
        assert!(check_rewrite(Some(old), None).is_err());
        let written = mark_written(old).unwrap();
        assert!(!note_edited(&written));
        assert!(note_edited(&format!("{written} ")));
        assert!(note_edited(&written.replace('\n', "\r\n")));
        assert!(check_rewrite(None, Some(&crate::vault::revision(old.as_bytes()))).is_err());
    }

    #[test]
    fn a8review_rewrite_retains_the_saved_path_after_a_date_or_match_change() {
        let dir = vault_with_people();
        let v = dir.path();
        let a = crate::workspaces::create(v, "First", "First").unwrap();
        let b = crate::workspaces::create(v, "Second", "Second").unwrap();
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let first =
            tauri::async_runtime::block_on(prepare(v, &meeting(Some(&a.id)), false, &writer))
                .unwrap();
        let mut moved = meeting(Some(&b.id));
        moved.start = "2026-10-12T18:00:00Z".into();
        let rewritten = tauri::async_runtime::block_on(prepare(v, &moved, true, &writer)).unwrap();
        assert_eq!(rewritten.path, first.path);
        assert_eq!(rewritten.workspace_id, first.workspace_id);
        assert_eq!(
            tauri::async_runtime::block_on(prepare(v, &moved, false, &writer))
                .unwrap()
                .markdown,
            rewritten.markdown
        );
        assert!(!v.join(&b.path).join("outputs/prep").exists());
    }

    #[test]
    fn a8review_failed_rewrite_does_not_hide_a_kept_pack_from_prepare() {
        let dir = vault_with_people();
        let v = dir.path();
        let m = meeting(None);
        let ok = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let fail = YieldWriter {
            calls: Default::default(),
            fail: true,
        };
        tauri::async_runtime::block_on(async {
            let saved = prepare(v, &m, false, &ok).await.unwrap();
            let (rewrite, ordinary) =
                tokio::join!(prepare(v, &m, true, &fail), prepare(v, &m, false, &ok));
            assert!(rewrite.is_err());
            let kept = ordinary.unwrap();
            assert!(kept.kept);
            assert_eq!(kept.markdown, saved.markdown);
            assert_eq!(fail.calls.get(), 1);
            assert_eq!(ok.calls.get(), 1);
        });
    }

    #[test]
    fn a8_concurrent_prepares_and_rewrites_share_one_paid_completion() {
        let dir = vault_with_people();
        let v = dir.path();
        let m = meeting(None);
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        tauri::async_runtime::block_on(async {
            let (a, b) = tokio::join!(
                prepare(v, &m, false, &writer),
                prepare(v, &m, false, &writer)
            );
            assert_eq!(a.unwrap().markdown, b.unwrap().markdown);
            assert_eq!(writer.calls.get(), 1);
            let (a, b) = tokio::join!(prepare(v, &m, true, &writer), prepare(v, &m, true, &writer));
            assert_eq!(a.unwrap().markdown, b.unwrap().markdown);
            assert_eq!(writer.calls.get(), 2, "one further paid rewrite, not two");
        });
    }

    #[test]
    fn a8_concurrent_failures_share_the_error_and_a_later_retry_can_run() {
        let dir = vault_with_people();
        let v = dir.path();
        let m = meeting(None);
        let fail = YieldWriter {
            calls: Default::default(),
            fail: true,
        };
        tauri::async_runtime::block_on(async {
            let (a, b) = tokio::join!(prepare(v, &m, false, &fail), prepare(v, &m, false, &fail));
            assert_eq!(a.unwrap_err(), b.unwrap_err());
            assert_eq!(fail.calls.get(), 1);
            let ok = YieldWriter {
                calls: Default::default(),
                fail: false,
            };
            assert!(prepare(v, &m, false, &ok).await.is_ok());
            assert_eq!(ok.calls.get(), 1);
        });
    }

    #[test]
    fn a8_unknown_attendees_cost_nothing_and_create_no_pack() {
        let dir = vault_with_people();
        let v = dir.path();
        let mut m = meeting(None);
        m.attendees = None;
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let result = tauri::async_runtime::block_on(prepare(v, &m, false, &writer));
        assert!(
            result.is_err(),
            "unknown attendees must not become a paid thin pack"
        );
        assert_eq!(writer.calls.get(), 0);
        assert!(find_kept(v, &m.id).is_none());
        // Known empty is different from unread; it is a valid meeting.
        m.attendees = Some(vec![]);
        assert!(tauri::async_runtime::block_on(prepare(v, &m, false, &writer)).is_ok());
        assert_eq!(writer.calls.get(), 1);
    }

    #[test]
    fn a8_kept_pack_keeps_its_real_path_when_workspace_matches_change() {
        let dir = vault_with_people();
        let v = dir.path();
        let a = crate::workspaces::create(v, "First", "First").unwrap();
        let b = crate::workspaces::create(v, "Second", "Second").unwrap();
        let writer = YieldWriter {
            calls: Default::default(),
            fail: false,
        };
        let first =
            tauri::async_runtime::block_on(prepare(v, &meeting(Some(&a.id)), false, &writer))
                .unwrap();
        let kept =
            tauri::async_runtime::block_on(prepare(v, &meeting(Some(&b.id)), false, &writer))
                .unwrap();
        assert_eq!(kept.path, first.path);
        assert_eq!(kept.workspace_id, first.workspace_id);
        assert_eq!(writer.calls.get(), 1);
    }

    #[test]
    fn a_pack_cites_its_sources_and_lands_in_the_matched_workspace() {
        let dir = vault_with_people();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Atlas", "Pricing").unwrap();
        let writer = Fake {
            reply: REPLY.into(),
            prompts: RefCell::new(Vec::new()),
        };
        let m = meeting(Some(&w.id));
        let pack = tauri::async_runtime::block_on(prepare(v, &m, false, &writer)).unwrap();
        assert!(!pack.kept);
        assert_eq!(pack.workspace_id.as_deref(), Some(w.id.as_str()));
        assert!(
            pack.path.starts_with(&format!(
                "workspaces/{}/outputs/prep/2026-10-11-atlas-pricing-review-",
                w.id
            )),
            "{}",
            pack.path
        );
        assert!(v.join(&pack.path).is_file());
        let md = &pack.markdown;
        assert!(
            md.contains("**Pat Smith**, CFO · [[people/Pat Smith]]"),
            "{md}"
        );
        assert!(md.contains("Finalize Q3 pricing"));
        assert!(md.contains("Confirm the Q3 pricing next step [2]"));
        assert!(
            !md.contains("Unsupported claim") && !md.contains("Out of range"),
            "uncited points are dropped"
        );
        assert!(md.contains("Is the Q3 floor price final?"));
        let prompt = writer.prompts.borrow()[0].clone();
        assert!(prompt.contains("not instructions") && prompt.contains("[1] Pat Smith"));
        // Kept: asking again doesn't call the agent.
        let again = tauri::async_runtime::block_on(prepare(v, &m, false, &writer)).unwrap();
        assert!(again.kept);
        assert_eq!(writer.prompts.borrow().len(), 1);
        assert!(exists(v, &m));
        tauri::async_runtime::block_on(prepare(v, &m, true, &writer)).unwrap();
        assert_eq!(writer.prompts.borrow().len(), 2, "regenerate writes again");
    }

    #[test]
    fn a_check_in_hears_of_a_meeting_within_the_hour_until_it_has_a_pack() {
        let dir = vault_with_people();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Atlas", "Pricing").unwrap();
        let m = meeting(Some(&w.id));
        let later = Meeting {
            minutes_until: 90,
            id: "evt-2".into(),
            ..m.clone()
        };
        let signals = crate::main_agent::meeting_signals(v, &[m.clone(), later]);
        assert_eq!(signals.len(), 1, "only within the hour");
        assert_eq!(signals[0].id, "meeting-prep:evt-1");
        assert!(
            signals[0].line.contains("agenda.prepare_meeting") && signals[0].line.contains("evt-1")
        );
        let writer = Fake {
            reply: REPLY.into(),
            prompts: RefCell::new(Vec::new()),
        };
        tauri::async_runtime::block_on(prepare(v, &m, false, &writer)).unwrap();
        assert!(
            crate::main_agent::meeting_signals(v, &[m]).is_empty(),
            "a pack exists"
        );
    }

    #[test]
    fn a_kept_pack_is_found_by_its_meeting_id_without_the_calendar() {
        let dir = vault_with_people();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Atlas", "Pricing").unwrap();
        let writer = Fake {
            reply: REPLY.into(),
            prompts: RefCell::new(Vec::new()),
        };
        let pack =
            tauri::async_runtime::block_on(prepare(v, &meeting(Some(&w.id)), false, &writer))
                .unwrap();
        let (path, text, workspace) = find_kept(v, "evt-1").expect("found by id");
        assert_eq!(
            (path.as_str(), workspace.as_deref()),
            (pack.path.as_str(), Some(w.id.as_str()))
        );
        assert!(text.contains("# Prep: Atlas pricing review"));
        assert!(find_kept(v, "evt-2").is_none());
        assert!(find_kept(v, "").is_none());
    }

    #[test]
    fn a_check_in_may_write_a_pack_but_never_rewrite_one() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let allows = |args: serde_json::Value| {
            crate::main_agent::unattended::allows(
                v,
                "agenda.prepare_meeting",
                false,
                args.as_object().unwrap(),
                true,
            )
        };
        assert!(allows(serde_json::json!({"meetingId": "evt-1"})).is_ok());
        assert!(
            allows(serde_json::json!({"meetingId": "evt-1", "regenerate": true}))
                .unwrap_err()
                .contains("never rewrites")
        );
    }

    /// ENG-39: a short attendee or title word never pulls notes that only share
    /// a word prefix; the full attendee name and the invite body feed the
    /// search; a title naming a workspace files the pack there at any status.
    #[test]
    fn prep_notes_match_whole_short_words_names_and_the_invite_body() {
        // A visible folder name: the vault walk skips dot-folders, and tempdir()'s is one.
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let note = |name: &str, text: &str| {
            crate::vault::write_note(v, &format!("notes/{name}"), text).unwrap();
        };
        note("native-plans.md", "The native pricing rollout.\n");
        note("nat-call.md", "Nat agreed the pricing floor.\n");
        note("ruiz-visit.md", "Visit report: Ana Ruiz asked for terms.\n");
        note("renewal-risk.md", "Zebracorp renewal risk is high.\n");
        let mut m = meeting(None);
        m.subject = "Nat pricing".into();
        let d = dossier(v, &m);
        let earlier = d.earlier.join("\n");
        assert!(earlier.contains("nat-call.md"), "{earlier}");
        assert!(
            !earlier.contains("native-plans.md"),
            "a 3-letter word is not a prefix: {earlier}"
        );
        // Full attendee name as a phrase: found though the title says nothing of it.
        let mut m = meeting(None);
        m.subject = "Weekly sync".into();
        let d = dossier(v, &m);
        assert!(
            d.earlier.join("\n").contains("ruiz-visit.md"),
            "{:?}",
            d.earlier
        );
        // The invite body feeds the search.
        assert!(!dossier(v, &m)
            .earlier
            .join("\n")
            .contains("renewal-risk.md"));
        m.body = Some("Agenda: zebracorp renewal".into());
        let d = dossier(v, &m);
        assert!(
            d.earlier.join("\n").contains("renewal-risk.md"),
            "{:?}",
            d.earlier
        );
    }

    #[test]
    fn a_title_naming_a_workspace_files_the_pack_there_whatever_its_status() {
        let dir = vault_with_people();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Orion Launch", "Launch").unwrap();
        let brief = v.join(&w.path).join("WORKSPACE.md");
        let text = std::fs::read_to_string(&brief).unwrap();
        assert!(text.contains("status: active"), "{text}");
        std::fs::write(&brief, text.replace("status: active", "status: archived")).unwrap();
        let mut m = meeting(None);
        m.subject = "Orion Launch go/no-go".into();
        let (_, id) = home_for(v, &m).unwrap();
        assert_eq!(id.as_deref(), Some(w.id.as_str()));
        // Words of a title that only share letters never name it.
        m.subject = "Orionic launching".into();
        let (_, id) = home_for(v, &m).unwrap();
        assert_ne!(id.as_deref(), Some(w.id.as_str()));
    }

    /// ENG-39 review: a real Teams invite body yields no searches (its words and
    /// links are boilerplate), the signed-in person's own name is not searched,
    /// and a title tie prefers a current workspace.
    #[test]
    fn invite_boilerplate_and_own_name_make_no_searches_and_title_ties_prefer_current() {
        let teams =
            "________________________________________________________________________________\n\
Microsoft Teams meeting\n\
Join: https://teams.microsoft.com/l/meetup-join/19%3ameeting_abc\n\
Meeting ID: 123 456 789 012\nPasscode: aBcDeF\n\
Download Teams | Join on the web\nLearn More | Meeting options\n\
________________________________________________________________________________\n";
        let mut m = meeting(None);
        m.subject = "Sync".into();
        m.attendees = None;
        m.organizer = None;
        m.body = Some(teams.into());
        assert!(note_queries(&m, "").is_empty(), "boilerplate only");
        // The same boilerplate after real words keeps only the real words.
        m.body = Some(format!("Agenda: zebracorp renewal terms\n{teams}"));
        let texts: Vec<String> = note_queries(&m, "").into_iter().map(|q| q.text).collect();
        assert!(texts.contains(&"zebracorp".to_string()), "{texts:?}");
        assert!(
            texts.iter().all(|t| !t.contains("microsoft")
                && !t.contains("passcode")
                && !t.contains("teams")),
            "{texts:?}"
        );
        // Zoom: "Join Zoom" starts the cut; a bare link token is dropped.
        assert_eq!(
            invite_text("Plan the audit https://x.zoom.us/j/1\nJoin Zoom Meeting\nPasscode 1"),
            "Plan the audit"
        );
        // Own name: the organizer who is the signed-in person is not searched; others are.
        let mut m = meeting(None);
        m.subject = "Sync".into();
        m.organizer = Some("Jamie Rivers".into());
        m.attendees = Some(vec!["Ana Ruiz".into()]);
        let own: Vec<String> = note_queries(&m, "Jamie Rivers")
            .into_iter()
            .map(|q| q.text)
            .collect();
        assert_eq!(own, vec!["\"Ana Ruiz\"".to_string()]);
        let all: Vec<String> = note_queries(&m, "").into_iter().map(|q| q.text).collect();
        assert!(all.contains(&"\"Jamie Rivers\"".to_string()));
    }

    #[test]
    fn a_title_naming_workspaces_prefers_a_current_one_and_ranking_wins_when_current() {
        let dir = vault_with_people();
        let v = dir.path();
        let old = crate::workspaces::create(v, "Orion Launch", "Launch").unwrap();
        let cur = crate::workspaces::create(v, "Orion Launch Plan", "Plan").unwrap();
        let set = |id: &str, from: &str, to: &str| {
            let brief = v.join(format!("workspaces/{id}/WORKSPACE.md"));
            let text = std::fs::read_to_string(&brief).unwrap();
            std::fs::write(&brief, text.replace(from, to)).unwrap();
        };
        set(&old.id, "status: active", "status: archived");
        // Both titles are in the subject: the current one wins though its title is shorter or longer.
        let mut m = meeting(None);
        m.subject = "Orion Launch Plan review".into();
        assert_eq!(home_for(v, &m).unwrap().1.as_deref(), Some(cur.id.as_str()));
        // A tie of equal titles: the current one, then the same answer every time.
        let twin = crate::workspaces::create(v, "Orion Launch", "Twin").unwrap();
        m.subject = "Orion Launch go/no-go".into();
        for _ in 0..3 {
            assert_eq!(
                home_for(v, &m).unwrap().1.as_deref(),
                Some(twin.id.as_str())
            );
        }
        // A current ranked match is kept over a title that names another workspace.
        let mut m = meeting(Some(&cur.id));
        m.subject = "Orion Launch go/no-go".into();
        assert_eq!(home_for(v, &m).unwrap().1.as_deref(), Some(cur.id.as_str()));
        // A ranked match that is not current yields to the title's workspace.
        let mut m = meeting(Some(&old.id));
        m.subject = "Orion Launch go/no-go".into();
        assert_eq!(
            home_for(v, &m).unwrap().1.as_deref(),
            Some(twin.id.as_str())
        );
        // Ranked and not current, with no title naming anything: still filed there.
        let mut m = meeting(Some(&old.id));
        m.subject = "Unrelated words".into();
        assert_eq!(home_for(v, &m).unwrap().1.as_deref(), Some(old.id.as_str()));
    }

    #[test]
    fn a_meeting_no_workspace_matches_goes_to_the_main_agent() {
        let dir = vault_with_people();
        let v = dir.path();
        let writer = Fake {
            reply: r#"{"openQuestions":[],"talkingPoints":[]}"#.into(),
            prompts: RefCell::new(Vec::new()),
        };
        let pack =
            tauri::async_runtime::block_on(prepare(v, &meeting(None), false, &writer)).unwrap();
        assert!(crate::workspaces::is_main_agent(
            pack.workspace_id.as_deref().unwrap()
        ));
        assert!(pack.markdown.contains("No workspace matched this meeting."));
        assert!(pack.markdown.contains("None the sources support."));
    }

    #[test]
    fn commitments_with_attendees_show_both_ways_and_people_match_by_name() {
        let dir = vault_with_people();
        let v = dir.path();
        let people = crate::vault_people::list(v);
        assert!(person_for(&people, "pat smith").is_some());
        assert!(person_for(&people, "Pat").is_some(), "a unique first name");
        assert!(person_for(&people, "Ana Ruiz").is_none());
        assert!(
            person_for(&people, "Pat Jones").is_none(),
            "another Pat is nobody's note"
        );
        let c = crate::vault_commitments::Commitment {
            url: "c".into(),
            vault_path: "commitments/q3.md".into(),
            title: "Q3 numbers".into(),
            direction: "owed_by_me".into(),
            counterparty: "Pat".into(),
            person_path: None,
            project_url: None,
            due: Some("2026-10-11".into()),
            status: "open".into(),
            channel: "meeting".into(),
            source_path: None,
            created: String::new(),
            fulfilled_at: None,
            fingerprint: String::new(),
            context: String::new(),
            snoozed_until: None,
            evidence: None,
        };
        assert!(
            !with(&c, "Pat Smith", None),
            "a first name is not a full name"
        );
        assert!(with(&c, "Pat", None));
        assert!(with(
            &crate::vault_commitments::Commitment {
                person_path: Some("people/Pat Smith.md".into()),
                ..c.clone()
            },
            "Pat Smith",
            Some("people/Pat Smith.md")
        ));
        assert!(!with(&c, "Ana Ruiz", None));
    }

    #[test]
    fn replies_are_parsed_leniently_and_cut() {
        let (q, p) = parse(&format!("Sure:\n```json\n{REPLY}\n```"), 3).unwrap();
        assert_eq!(q.len(), 1);
        assert_eq!(p.len(), 1);
        assert!(parse("nothing", 3).is_err());
        let (_, p) = parse(r#"{"talkingPoints":[{"point":"x","cites":[2,1,2]}]}"#, 3).unwrap();
        assert_eq!(p[0].cites, vec![2, 1], "repeats go, order stays");
        let stem = file_stem(&Meeting {
            subject: "  Q3 / Pricing!!  ".into(),
            ..meeting(None)
        });
        assert!(
            stem.starts_with("2026-10-11-q3-pricing-")
                && stem.len() == "2026-10-11-q3-pricing-".len() + 6,
            "{stem}"
        );
        assert!(file_stem(&Meeting {
            subject: "!!!".into(),
            ..meeting(None)
        })
        .contains("-meeting-"));
        // Two meetings with one subject on one day are two files.
        let other = Meeting {
            id: "evt-other".into(),
            ..meeting(None)
        };
        assert_ne!(file_stem(&meeting(None)), file_stem(&other));
    }
}
