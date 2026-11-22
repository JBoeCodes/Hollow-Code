//! Meetings needing prep (overnight E3): calendar events in the next N hours,
//! each matched to the workspaces that could prepare for it, with what those
//! already have. A read action for the Main agent; it changes nothing.
//!
//! - **Events** come from the Agenda's own read (`agenda::events_with`, never kept: Hollow's
//!   Microsoft 365 sign-in, read-only `calendarView`). Not signed in is a
//!   state, said in `calendar`, not an error. Cancelled and all-day events
//!   are left out.
//! - **Matching is local**: the meeting's subject against each current
//!   workspace's title and the name of the project it follows (its tracker
//!   mirror or project note, as the workspace map resolves it), by
//!   distinctive shared words ([`words`] drops the words every meeting has:
//!   "sync", "weekly", "call"…); and its attendees against the brief, by
//!   full name (two words or more; "Last, First" and a "(…)" suffix handled)
//!   or email address, both on boundaries (morning N4). The signed-in person
//!   is left out of the attendees. A word or a person in two or more
//!   workspaces and a fifth or more of them, once there are five, says
//!   nothing; a project's words count once per project, not per workspace
//!   following it. Words and up to two people add up. At most [`MATCHES`]
//!   per meeting, best first; each says which words, project and people
//!   matched.
//! - **Attendees** need one read-only request per meeting (the list view has
//!   none), made only for meetings in the window, 15 s in all, and cached
//!   for the day ([`attendees_for`]); a meeting whose attendees weren't read
//!   has `attendees: null`. At most [`ATTENDEES_SHOWN`] are listed.
//! - **What exists** for each match: the brief's next step, the open plan
//!   items, the newest Result and the last activity (`workspace_results::
//!   digests`), and the organizer's `people/` note when there is one.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

pub const DEFAULT_HOURS: u32 = 12;
const MATCHES: usize = 3;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Match {
    pub workspace_id: String,
    pub title: String,
    /// The distinctive words the meeting and the workspace share.
    pub because: Vec<String>,
    /// The project it follows, when one of those words came from its name.
    pub project: Option<String>,
    /// Attendees the brief names (full name or email).
    pub people: Vec<String>,
    pub next_step: Option<String>,
    pub open_items: Option<usize>,
    pub latest_result: Option<String>,
    pub last_activity_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Meeting {
    /// The calendar event's id (overnight 10-11 A4: Prepare finds it by this).
    pub id: String,
    pub subject: String,
    pub start: String,
    pub end: String,
    pub minutes_until: i64,
    pub organizer: Option<String>,
    pub location: Option<String>,
    pub join_url: Option<String>,
    /// The organizer's `people/` note, when the vault has one.
    pub person: Option<String>,
    /// Its attendees' names (or emails); `None` when they couldn't be read.
    pub attendees: Option<Vec<String>>,
    /// The invite's body (Graph's `bodyPreview`, a native event's notes), when
    /// the calendar gave one: feeds the pack's note search (ENG-39), never shown.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub matches: Vec<Match>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub hours: u32,
    /// The calendar's state: `connected`, `not signed in`, or the error.
    pub calendar: String,
    pub complete: bool,
    pub meetings: Vec<Meeting>,
}

/// Words every meeting title has, which say nothing about the work.
const COMMON: &[&str] = &[
    "the",
    "and",
    "for",
    "with",
    "from",
    "this",
    "your",
    "our",
    "new",
    "next",
    "meeting",
    "meetings",
    "meet",
    "sync",
    "syncs",
    "call",
    "calls",
    "weekly",
    "biweekly",
    "daily",
    "monthly",
    "quarterly",
    "review",
    "reviews",
    "update",
    "updates",
    "status",
    "check",
    "chat",
    "catch",
    "touch",
    "base",
    "team",
    "one",
    "session",
    "discussion",
    "discuss",
    "intro",
    "follow",
    "followup",
    "prep",
    "about",
    "teams",
    "zoom",
    "office",
    "hours",
    "standup",
    "stand",
    "kickoff",
    "planning",
    "huddle",
    "project",
    // Teams and Zoom invite boilerplate (ENG-39).
    "microsoft",
    "computer",
    "passcode",
    "download",
    "options",
    "scheduled",
    "inviting",
];

/// The distinctive lowercase words of a text: 3+ letters, not all digits,
/// not [`COMMON`].
pub fn words(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '&'))
        .map(|w| w.trim_matches('&').to_lowercase())
        .filter(|w| {
            w.chars().count() >= 3
                && !w.chars().all(|c| c.is_ascii_digit())
                && !COMMON.contains(&w.as_str())
        })
        .collect()
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// An event's start as an instant (`local_time` gives RFC 3339 or a local naive time).
fn instant(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Some(t.with_timezone(&Utc));
    }
    let naive =
        chrono::NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S%.f")
            .ok()?;
    use chrono::TimeZone;
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|t| t.with_timezone(&Utc))
}

/// A workspace that could prepare: current, not built in, with the name of
/// the project it follows and its brief (lowercased, first 64 KB) for
/// attendee names.
#[derive(Debug, Clone, Default)]
pub struct Candidate {
    pub id: String,
    pub title: String,
    pub project: Option<String>,
    pub brief: String,
}

pub fn candidates(vault: &Path) -> Vec<Candidate> {
    crate::workspaces::list(vault)
        .unwrap_or_default()
        .into_iter()
        .filter(|w| {
            !crate::workspaces::is_builtin(&w.id) && crate::workspaces::is_current_status(&w.status)
        })
        .map(|w| {
            let brief = crate::workspaces::folder(vault, &w.id)
                .ok()
                .and_then(|d| {
                    crate::brief_read::read_prefix(
                        &d.join("WORKSPACE.md"),
                        crate::brief_read::BRIEF_PREFIX,
                    )
                })
                .unwrap_or_default();
            let (fm, _) = crate::frontmatter::parse_map(&brief);
            Candidate {
                project: crate::brief_read::project_of(vault, &fm).map(|(_, name)| name),
                brief: brief.to_lowercase(),
                id: w.id,
                title: w.title,
            }
        })
        .collect()
}

/// One attendee as the calendar gives them.
#[derive(Debug, Clone, PartialEq)]
pub struct Attendee {
    pub name: String,
    pub email: String,
}

/// A display name as a brief would write it: "Ruiz, Ana (Contractor)" is
/// "ana ruiz"; whitespace collapsed, lowercase.
fn plain_name(name: &str) -> String {
    let name = name.split('(').next().unwrap_or("").trim();
    let name = match name.split_once(',') {
        Some((last, first)) if !first.trim().is_empty() => {
            format!("{} {}", first.trim(), last.trim())
        }
        _ => name.to_string(),
    };
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Whether `needle` occurs in `text` as a whole word (no letter or digit
/// either side).
pub(crate) fn bounded(text: &str, needle: &str) -> bool {
    !needle.is_empty()
        && text.match_indices(needle).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + needle.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
}

/// Whether `text` (lowercase) names `who`: a full name of two words or more,
/// or the email address, each on boundaries.
fn names(text: &str, who: &Attendee) -> bool {
    let name = plain_name(&who.name);
    let full = name.split(' ').filter(|p| p.chars().count() >= 2).count() >= 2;
    let email = who.email.trim().to_lowercase();
    // An address runs on through letters, digits and . _ % + - (so ana@x.com
    // isn't inside joanna@x.com); a sentence's full stop after it is fine.
    let in_address = |c: char| c.is_alphanumeric() || "_%+-".contains(c);
    (full && bounded(text, &name))
        || (email.contains('@')
            && text.match_indices(&email).any(|(i, _)| {
                let before = text[..i].chars().next_back();
                let mut after = text[i + email.len()..].chars();
                let next = after.next();
                // ".au" continues the address; "." then a space ends a sentence.
                let runs_on = next.is_some_and(in_address)
                    || (next == Some('.') && after.next().is_some_and(char::is_alphanumeric));
                !before.is_some_and(|c| in_address(c) || c == '.') && !runs_on
            }))
}

/// What one workspace matched for a meeting.
#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub id: String,
    pub title: String,
    pub because: Vec<String>,
    pub project: Option<String>,
    pub people: Vec<String>,
}

/// Rank `workspaces` for a meeting: shared distinctive subject words (from
/// the title or the project's name) plus up to two attendees the brief names
/// (see the module doc for what says nothing); on a tie the subject's own
/// words, then the organizer's words as a tie-break only (a first
/// name is often an ordinary word), then fewer words in the title; ties keep
/// list order.
pub fn rank(
    subject: &str,
    organizer: Option<&str>,
    attendees: &[Attendee],
    workspaces: &[Candidate],
) -> Vec<Ranked> {
    let titles: Vec<BTreeSet<String>> = workspaces.iter().map(|c| words(&c.title)).collect();
    let projects: Vec<BTreeSet<String>> = workspaces
        .iter()
        .map(|c| c.project.as_deref().map(words).unwrap_or_default())
        .collect();
    // Says nothing: in two or more and a fifth or more of five or more.
    // Words count by group: the workspaces following one project are one
    // group (their titles and the project's name), every other workspace its
    // own, so a project several workspaces follow doesn't make its name
    // common.
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (i, c) in workspaces.iter().enumerate() {
        let key = c
            .project
            .as_ref()
            .map(|p| format!("project:{p}"))
            .unwrap_or_else(|| format!("workspace:{i}"));
        let group = groups.entry(key).or_default();
        group.extend(titles[i].iter().cloned());
        group.extend(projects[i].iter().cloned());
    }
    let common_in = |n: usize, len: usize| len >= 5 && n >= 2 && n * 5 >= len;
    let widespread: BTreeSet<String> = {
        let mut count = BTreeMap::<&String, usize>::new();
        for g in groups.values() {
            for w in g {
                *count.entry(w).or_default() += 1;
            }
        }
        count
            .into_iter()
            .filter(|(_, n)| common_in(*n, groups.len()))
            .map(|(w, _)| w.clone())
            .collect()
    };
    let common = |n: usize| common_in(n, workspaces.len());
    // Which workspaces each attendee's name or address is in, worked out once.
    let named: Vec<(&Attendee, Vec<bool>)> = attendees
        .iter()
        .map(|a| (a, workspaces.iter().map(|c| names(&c.brief, a)).collect()))
        .filter(|(_, hits): &(&Attendee, Vec<bool>)| {
            let n = hits.iter().filter(|h| **h).count();
            n > 0 && !common(n)
        })
        .collect();
    let want: BTreeSet<String> = words(subject).difference(&widespread).cloned().collect();
    let by: BTreeSet<String> = organizer.map(words).unwrap_or_default();
    let mut scored: Vec<(usize, usize, usize, usize, usize, Ranked)> = workspaces
        .iter()
        .zip(titles.iter().zip(&projects))
        .enumerate()
        .filter_map(|(i, (c, (title, project)))| {
            let have: BTreeSet<String> = title.union(project).cloned().collect();
            let shared: Vec<String> = want.intersection(&have).cloned().collect();
            let met: Vec<String> = named
                .iter()
                .filter(|(_, hits)| hits[i])
                .map(|(a, _)| {
                    if a.name.trim().is_empty() {
                        a.email.clone()
                    } else {
                        a.name.clone()
                    }
                })
                .collect();
            if shared.is_empty() && met.is_empty() {
                return None;
            }
            let via_project = shared
                .iter()
                .any(|w| project.contains(w) && !title.contains(w));
            // A roster brief naming everyone mustn't outrank the subject.
            Some((
                shared.len() + met.len().min(2),
                // On a tie, the subject's own words first.
                shared.len(),
                by.intersection(&have).count(),
                title.len(),
                i,
                Ranked {
                    id: c.id.clone(),
                    title: c.title.clone(),
                    because: shared,
                    project: c.project.clone().filter(|_| via_project),
                    people: met,
                },
            ))
        })
        .collect();
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then(b.1.cmp(&a.1))
            .then(b.2.cmp(&a.2))
            .then(a.3.cmp(&b.3))
            .then(a.4.cmp(&b.4))
    });
    scored.into_iter().take(MATCHES).map(|s| s.5).collect()
}

/// The organizer's `people/<Name>.md`, when it exists (the name comes from
/// whoever sent the invite, so it goes through the vault's path check).
fn person_note(vault: &Path, organizer: &str) -> Option<String> {
    let rel = format!("people/{}.md", organizer.trim());
    crate::vault::safe_join(vault, &rel)
        .ok()
        .filter(|p| p.is_file())
        .map(|_| rel)
}

/// The meetings of `view` (the Agenda's view JSON) that start in
/// `[now, now + hours)`, not cancelled or all-day: their event ids.
pub fn upcoming_ids(view: &Value, now: DateTime<Utc>, hours: u32) -> Vec<String> {
    let until = now + Duration::hours(i64::from(hours));
    view["events"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            e["cancelled"] != true
                && e["allDay"] != true
                && text(e, "subject").or_else(|| text(e, "title")).is_some()
        })
        .filter(|e| {
            text(e, "start")
                .as_deref()
                .and_then(instant)
                .is_some_and(|t| t >= now && t < until)
        })
        .filter_map(|e| text(e, "id"))
        .collect()
}

/// The report for `events` (the Agenda's view JSON) at `now`, with each
/// event's attendees by event id (an id missing: they couldn't be read).
pub fn report(
    vault: &Path,
    view: &Value,
    now: DateTime<Utc>,
    hours: u32,
    attendees: &BTreeMap<String, Vec<Attendee>>,
) -> Report {
    report_between(
        vault,
        view,
        now,
        now + Duration::hours(i64::from(hours)),
        now,
        attendees,
    )
}

pub(crate) fn report_between(
    vault: &Path,
    view: &Value,
    from: DateTime<Utc>,
    until: DateTime<Utc>,
    now: DateTime<Utc>,
    attendees: &BTreeMap<String, Vec<Attendee>>,
) -> Report {
    let hours = (until - from).num_hours().max(0) as u32;
    let calendar = if view["connected"] == false {
        disconnected_reason(view)
    } else if let Some(e) = view["error"].as_str() {
        format!("couldn't be read: {e}")
    } else {
        "connected".to_string()
    };
    let workspaces = candidates(vault);
    let mut meetings = Vec::new();
    let mut pending: BTreeSet<String> = BTreeSet::new();
    for e in view["events"].as_array().into_iter().flatten() {
        if e["cancelled"] == true || e["allDay"] == true {
            continue;
        }
        let Some(subject) = text(e, "subject").or_else(|| text(e, "title")) else {
            continue;
        };
        let start_s = text(e, "start").unwrap_or_default();
        let Some(start) = instant(&start_s) else {
            continue;
        };
        if start < from || start >= until {
            continue;
        }
        let organizer = text(e, "organizer");
        let people = text(e, "id").and_then(|id| attendees.get(&id));
        let ranked = rank(
            &subject,
            organizer.as_deref(),
            people.map(Vec::as_slice).unwrap_or_default(),
            &workspaces,
        );
        pending.extend(ranked.iter().map(|r| r.id.clone()));
        let matches = ranked
            .into_iter()
            .map(|r| Match {
                next_step: None,
                open_items: None,
                latest_result: None,
                last_activity_at: None,
                workspace_id: r.id,
                title: r.title,
                because: r.because,
                project: r.project,
                people: r.people,
            })
            .collect();
        meetings.push(Meeting {
            id: text(e, "id").unwrap_or_default(),
            attendees: people.map(|p| {
                let mut shown: Vec<String> = p
                    .iter()
                    .take(ATTENDEES_SHOWN)
                    .map(|a| {
                        if a.name.is_empty() {
                            a.email.clone()
                        } else {
                            a.name.clone()
                        }
                    })
                    .collect();
                if p.len() > ATTENDEES_SHOWN {
                    shown.push(format!("+{} more", p.len() - ATTENDEES_SHOWN));
                }
                shown
            }),
            person: organizer.as_deref().and_then(|o| person_note(vault, o)),
            minutes_until: (start - now).num_minutes(),
            end: text(e, "end").unwrap_or_default(),
            start: start_s,
            location: text(e, "location"),
            join_url: text(e, "joinUrl"),
            body: text(e, "bodyPreview")
                .or_else(|| text(e, "body"))
                .or_else(|| text(e, "notes")),
            organizer,
            subject,
            matches,
        });
    }
    // What exists, read once for every matched workspace (24 at a time).
    let ids: Vec<String> = pending.into_iter().collect();
    let digests: Vec<_> = ids
        .chunks(24)
        .flat_map(|c| crate::workspace_results::digests(vault, c).unwrap_or_default())
        .collect();
    for m in meetings.iter_mut().flat_map(|m| m.matches.iter_mut()) {
        if let Some(d) = digests.iter().find(|d| d.workspace_id == m.workspace_id) {
            m.next_step = d.reentry.next_step.clone();
            m.open_items = d.reentry.open_item_count;
            m.latest_result = d.latest_result.as_ref().map(|r| r.path.clone());
            m.last_activity_at = d.last_activity_at.clone();
        }
    }
    meetings.sort_by_key(|m| m.minutes_until);
    Report {
        hours,
        calendar,
        complete: view["complete"] != false,
        meetings,
    }
}

/// Attendees read today: (day, event id → attendees).
pub type AttendeeCache = Mutex<Option<(chrono::NaiveDate, BTreeMap<String, Vec<Attendee>>)>>;
/// The app's cache (morning N4): a meeting's attendees are read once a day,
/// not on every call.
static ATTENDEES: AttendeeCache = Mutex::new(None);
/// Attendees listed per meeting, at most (all of them still match).
pub const ATTENDEES_SHOWN: usize = 15;

fn disconnected_reason(view: &Value) -> String {
    view["status"]
        .as_str()
        .unwrap_or("not signed in to Microsoft 365")
        .to_string()
}

fn local_attendees(view: &Value, ids: &[String]) -> Option<BTreeMap<String, Vec<Attendee>>> {
    match view["provider"].as_str() {
        None | Some("microsoft") => None,
        Some("mac") => Some(
            view["events"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|event| {
                    let id = event["id"].as_str()?;
                    if !ids.iter().any(|i| i == id) {
                        return None;
                    }
                    // A missing/null list is unknown; an explicitly empty list is known.
                    let list = event["attendees"].as_array()?;
                    Some((
                        id.to_string(),
                        list.iter()
                            .map(|a| Attendee {
                                name: a["name"].as_str().unwrap_or_default().to_string(),
                                email: a["email"].as_str().unwrap_or_default().to_string(),
                            })
                            .collect(),
                    ))
                })
                .collect(),
        ),
        _ => Some(BTreeMap::new()),
    }
}

async fn view_attendees(view: &Value, ids: &[String]) -> BTreeMap<String, Vec<Attendee>> {
    if let Some(local) = local_attendees(view, ids) {
        return local;
    }
    let mut attendees = attendees_for(
        &ATTENDEES,
        ids,
        chrono::Local::now().date_naive(),
        |ids| async move {
            tokio::time::timeout(
                std::time::Duration::from_secs(15),
                crate::m365_mcp::agenda_attendees(&ids),
            )
            .await
            .unwrap_or_default()
        },
    )
    .await;
    if let Some(me) = crate::m365_auth::account().map(|a| a.trim().to_lowercase()) {
        for list in attendees.values_mut() {
            list.retain(|a| a.email.trim().to_lowercase() != me);
        }
    }
    attendees
}
/// At most this many attendee reads per call (one request each).
const ATTENDEE_READS: usize = 25;

/// The attendees of `ids`: today's cache, else one read-only request per
/// event (`fetch`), keeping the ones that came back. A failed read is left
/// out, so its meeting says it has no attendee list.
pub async fn attendees_for<F, Fut>(
    cache: &AttendeeCache,
    ids: &[String],
    today: chrono::NaiveDate,
    fetch: F,
) -> BTreeMap<String, Vec<Attendee>>
where
    F: FnOnce(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = BTreeMap<String, Result<Vec<(String, String)>, String>>>,
{
    let (mut out, missing) = {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.as_ref().is_none_or(|(day, _)| *day != today) {
            *cache = Some((today, BTreeMap::new()));
        }
        let known = &cache.as_ref().unwrap().1;
        let out: BTreeMap<String, Vec<Attendee>> = ids
            .iter()
            .filter_map(|id| known.get(id).map(|a| (id.clone(), a.clone())))
            .collect();
        let missing: Vec<String> = ids
            .iter()
            .filter(|id| !known.contains_key(*id))
            .take(ATTENDEE_READS)
            .cloned()
            .collect();
        (out, missing)
    };
    if missing.is_empty() {
        return out;
    }
    let read = fetch(missing).await;
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    for (id, got) in read {
        let Ok(pairs) = got else { continue };
        let list: Vec<Attendee> = pairs
            .into_iter()
            .map(|(name, email)| Attendee { name, email })
            .collect();
        if let Some((day, known)) = cache.as_mut() {
            if *day == today {
                known.insert(id.clone(), list.clone());
            }
        }
        out.insert(id, list);
    }
    out
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum FinishedError {
    Unavailable,
    Missing,
    Ineligible,
}
impl std::fmt::Display for FinishedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Unavailable => "Calendar unavailable; attendees couldn't be read.",
            Self::Missing => "The linked meeting isn't in this calendar window.",
            Self::Ineligible => "That timed meeting hasn't ended, is all-day, or was cancelled.",
        })
    }
}

/// How far back Prepare looks for a meeting already under way (P0-26).
pub(crate) const PREP_LOOKBACK_HOURS: i64 = 12;

/// The meeting `id` that Prepare can still use (P0-26): one that started up
/// to [`PREP_LOOKBACK_HOURS`] ago or starts in the next `hours`, and hasn't
/// ended. The read used to start at now, so a meeting in progress was "not
/// in the next 24 hours".
pub(crate) fn preparable(
    vault: &Path,
    view: &Value,
    id: &str,
    now: DateTime<Utc>,
    hours: u32,
    attendees: &BTreeMap<String, Vec<Attendee>>,
) -> Option<Meeting> {
    let from = now - Duration::hours(PREP_LOOKBACK_HOURS);
    let until = now + Duration::hours(i64::from(hours));
    report_between(vault, view, from, until, now, attendees)
        .meetings
        .into_iter()
        .find(|m| m.id == id && instant(&m.end).is_none_or(|end| end > now))
}

/// Read the calendar for Prepare: the window [`preparable`] looks in, with
/// the asked meeting's attendees. `Err` says what state the calendar is in.
pub(crate) async fn for_prep(
    vault: &Path,
    id: &str,
    hours: u32,
) -> Result<Option<Meeting>, String> {
    let now = Utc::now();
    let view = crate::agenda::events_with(
        &(now - Duration::hours(PREP_LOOKBACK_HOURS)).to_rfc3339(),
        &(now + Duration::hours(i64::from(hours))).to_rfc3339(),
        crate::agenda::Reuse::Once,
    )
    .await;
    if view["connected"] == false {
        return Err(disconnected_reason(&view));
    }
    if let Some(e) = view["error"].as_str() {
        return Err(format!("couldn't be read: {e}"));
    }
    // An unknown id needs no 15 s attendee read to be told so.
    let listed = view["events"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|e| text(e, "id").as_deref() == Some(id));
    if !listed {
        return Ok(None);
    }
    let attendees = view_attendees(&view, &[id.to_string()]).await;
    if !crate::agenda::source_is_current(&view) {
        return Err("Calendar source changed. Refresh before preparing.".into());
    }
    let (vault, id) = (vault.to_path_buf(), id.to_string());
    tauri::async_runtime::spawn_blocking(move || {
        preparable(&vault, &view, &id, now, hours, &attendees)
    })
    .await
    .map_err(|_| "couldn't be read".to_string())
}

/// Read a finished timed meeting around a known start or recording-day centre.
pub(crate) async fn finished(
    vault: &Path,
    id: &str,
    around: DateTime<Utc>,
) -> Result<Meeting, FinishedError> {
    let now = Utc::now();
    let from = around - Duration::hours(12);
    let until = around + Duration::hours(12);
    let view = crate::agenda::events_with(
        &from.to_rfc3339(),
        &until.to_rfc3339(),
        crate::agenda::Reuse::Once,
    )
    .await;
    if view["connected"] != true || !view["error"].is_null() {
        return Err(FinishedError::Unavailable);
    }
    let event = view["events"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| e["id"].as_str() == Some(id))
        .ok_or(FinishedError::Missing)?;
    if event["cancelled"] == true
        || event["allDay"] == true
        || text(event, "end")
            .and_then(|s| instant(&s))
            .is_none_or(|end| end > now)
    {
        return Err(FinishedError::Ineligible);
    }
    let attendees = view_attendees(&view, &[id.to_string()]).await;
    if !crate::agenda::source_is_current(&view) {
        return Err(FinishedError::Unavailable);
    }
    report_between(vault, &view, from, until, now, &attendees)
        .meetings
        .into_iter()
        .find(|m| m.id == id && instant(&m.end).is_some_and(|end| end <= now))
        .ok_or(FinishedError::Missing)
}

/// `agenda.meetings_needing_prep`.
pub async fn for_this_mac(vault: &Path, hours: u32) -> Report {
    let now = Utc::now();
    // A from-now window is never asked for twice: read it, keep nothing.
    let view = crate::agenda::events_with(
        &now.to_rfc3339(),
        &(now + Duration::hours(i64::from(hours))).to_rfc3339(),
        crate::agenda::Reuse::Once,
    )
    .await;
    let ids = if view["connected"] == true && view["error"].is_null() {
        upcoming_ids(&view, now, hours)
    } else {
        Vec::new()
    };
    let attendees = view_attendees(&view, &ids).await;
    if !crate::agenda::source_is_current(&view) {
        return Report {
            hours,
            calendar: "Calendar source changed. Refresh before preparing.".into(),
            complete: false,
            meetings: vec![],
        };
    }
    let vault = vault.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || report(&vault, &view, now, hours, &attendees))
        .await
        .unwrap_or(Report {
            hours,
            calendar: "couldn't be read".into(),
            complete: false,
            meetings: Vec::new(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_title_and_attendees_reach_meeting_preparation() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let view = json!({"connected": true, "complete": true, "provider": "mac", "events": [{
            "id": "mac:occurrence", "title": "Planning",
            "start": (now + Duration::minutes(30)).to_rfc3339(),
            "end": (now + Duration::minutes(90)).to_rfc3339(),
            "attendees": [{"name": "Ana", "email": "ana@example.test"}]
        }]});
        let ids = upcoming_ids(&view, now, 24);
        assert_eq!(ids, vec!["mac:occurrence"]);
        let people = local_attendees(&view, &ids).unwrap();
        let meetings = report(dir.path(), &view, now, 24, &people);
        assert_eq!(meetings.meetings.len(), 1);
        assert!(preparable(dir.path(), &view, "mac:occurrence", now, 24, &people).is_some());
        assert_eq!(people["mac:occurrence"][0].name, "Ana");
    }

    #[test]
    fn local_attendees_bypass_graph_cache_caps_and_preserve_unknown_coverage() {
        let mut events: Vec<_> = (0..30).map(|n| json!({"id": format!("mac:{n}"), "attendees": [{"name": "Ana", "email": "ana@example.test"}]})).collect();
        events.push(json!({"id": "unknown", "attendees": null}));
        events.push(json!({"id": "empty", "attendees": []}));
        let ids: Vec<_> = events
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect();
        let native = local_attendees(&json!({"provider": "mac", "events": events}), &ids)
            .expect("native must bypass Graph");
        assert_eq!(native.len(), 31);
        assert!(!native.contains_key("unknown"));
        assert_eq!(native["empty"], vec![]);
        assert_eq!(native["mac:29"][0].name, "Ana");
        assert!(local_attendees(&json!({"provider": "off"}), &ids)
            .unwrap()
            .is_empty());
        assert!(local_attendees(&json!({"source": "Microsoft 365"}), &ids).is_none());
    }

    #[test]
    fn prepare_finds_a_meeting_in_progress_and_not_one_that_ended() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let event = |id: &str, from: i64, to: i64| json!({"id":id,"subject":"Planning","start":(now + Duration::minutes(from)).to_rfc3339(),"end":(now + Duration::minutes(to)).to_rfc3339()});
        let view = json!({"connected":true,"events":[event("now",-10,50),event("over",-90,-30),event("next",60,90)]});
        let people = BTreeMap::new();
        let found = |id| preparable(dir.path(), &view, id, now, 24, &people).map(|m| m.id);
        assert_eq!(found("now").as_deref(), Some("now"));
        assert_eq!(found("next").as_deref(), Some("next"));
        assert_eq!(found("over"), None);
    }

    #[test]
    fn past_window_preserves_upcoming_report_contract() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let event = |id: &str, offset| json!({"id":id,"subject":"Planning","start":(now + Duration::hours(offset)).to_rfc3339(),"end":(now + Duration::hours(offset) + Duration::minutes(30)).to_rfc3339()});
        let view = json!({"connected":true,"events":[event("past",-2),event("next",2)]});
        let people = BTreeMap::new();
        let upcoming = report(dir.path(), &view, now, 24, &people);
        assert_eq!(
            upcoming
                .meetings
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["next"]
        );
        assert_eq!(
            upcoming,
            report_between(
                dir.path(),
                &view,
                now,
                now + Duration::hours(24),
                now,
                &people
            )
        );
        let both = report_between(
            dir.path(),
            &view,
            now - Duration::hours(12),
            now + Duration::hours(12),
            now,
            &people,
        );
        assert_eq!(both.meetings.len(), 2);
        assert_eq!(both.meetings[0].minutes_until, -120);
    }

    fn cand(id: &str, title: &str) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            ..Default::default()
        }
    }

    fn who(name: &str, email: &str) -> Attendee {
        Attendee {
            name: name.into(),
            email: email.into(),
        }
    }

    #[test]
    fn distinctive_words_only() {
        let w = words("Weekly sync: TipFlow tip reconciliation & ADP");
        assert_eq!(
            w.into_iter().collect::<Vec<_>>(),
            ["adp", "reconciliation", "tip", "tipflow"]
        );
    }

    #[test]
    fn workspaces_rank_by_shared_words_then_the_closer_title() {
        let ws = vec![
            cand("a", "Payroll union reconciliation and ADP"),
            cand("b", "tipflow-workflow"),
            cand("c", "TipFlow tip reconciliation"),
            cand("d", "Lot optimizer"),
        ];
        let r = rank("TipFlow tip run review", None, &[], &ws);
        let ids: Vec<&str> = r.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            ids,
            ["c", "b"],
            "two shared words beat one; no shared word, no match"
        );
        assert_eq!(r[0].because, ["tip", "tipflow"]);
        assert!(
            rank("Weekly team sync", Some("Ann Lee"), &[], &ws).is_empty(),
            "common words never match"
        );
    }

    #[test]
    fn upcoming_meetings_get_their_workspaces_and_people_and_the_rest_are_left_out() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "TipFlow tip reconciliation", "x").unwrap();
        std::fs::create_dir_all(v.join("people")).unwrap();
        std::fs::write(v.join("people/Morgan Reyes.md"), "# Morgan").unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let view = json!({
            "connected": true, "complete": true, "error": null,
            "events": [
                { "subject": "TipFlow tip run", "start": "2026-10-10T14:00:00Z", "end": "2026-10-10T14:30:00Z", "organizer": "Morgan Reyes", "joinUrl": "https://teams/x" },
                { "subject": "TipFlow tip run (moved)", "start": "2026-10-10T13:00:00Z", "end": "2026-10-10T13:30:00Z", "cancelled": true },
                { "subject": "Company holiday", "start": "2026-10-10T00:00:00Z", "end": "2026-10-11T00:00:00Z", "allDay": true },
                { "subject": "Already started", "start": "2026-10-10T11:30:00Z", "end": "2026-10-10T12:30:00Z" },
                { "subject": "Far away", "start": "2026-10-11T12:00:00Z", "end": "2026-10-11T13:00:00Z" },
                { "subject": "Weekly team sync", "start": "2026-10-10T12:30:00Z", "end": "2026-10-10T13:00:00Z" },
            ]
        });
        let r = report(v, &view, now, 12, &BTreeMap::new());
        assert_eq!(r.calendar, "connected");
        let subjects: Vec<&str> = r.meetings.iter().map(|m| m.subject.as_str()).collect();
        assert_eq!(
            subjects,
            ["Weekly team sync", "TipFlow tip run"],
            "soonest first; cancelled, all-day, started and far ones out"
        );
        assert!(r.meetings[0].matches.is_empty());
        let m = &r.meetings[1];
        assert_eq!(m.minutes_until, 120);
        assert_eq!(m.person.as_deref(), Some("people/Morgan Reyes.md"));
        assert_eq!(m.matches.len(), 1);
        assert_eq!(m.matches[0].workspace_id, w.id);
        assert_eq!(m.matches[0].because, ["tip", "tipflow"]);
    }

    #[test]
    fn a_calendar_not_signed_in_or_failing_is_a_state() {
        let dir = tempfile::tempdir().unwrap();
        let now = Utc::now();
        let off = report(
            dir.path(),
            &json!({ "connected": false, "events": [], "complete": true }),
            now,
            12,
            &BTreeMap::new(),
        );
        assert_eq!(off.calendar, "not signed in to Microsoft 365");
        let err = report(
            dir.path(),
            &json!({ "connected": true, "events": [], "complete": false, "error": "401" }),
            now,
            12,
            &BTreeMap::new(),
        );
        assert_eq!(err.calendar, "couldn't be read: 401");
        assert!(!err.complete);
    }

    #[test]
    fn attendees_the_brief_names_and_project_names_match_too() {
        let mut ws = vec![
            Candidate {
                project: Some("Atlas 2.0 pricing".into()),
                ..cand("a", "Garage rates")
            },
            Candidate {
                brief: "owner: ana ruiz (ana@example.com) and bob".into(),
                ..cand("b", "Lot survey")
            },
            cand("c", "Payroll audit"),
        ];
        // The project's name carries the word the title lacks, and says so.
        let r = rank("Atlas check-in", None, &[], &ws);
        assert_eq!(r.len(), 1);
        assert_eq!(
            (r[0].id.as_str(), r[0].project.as_deref()),
            ("a", Some("Atlas 2.0 pricing"))
        );
        assert_eq!(r[0].because, ["atlas"]);
        // A full name or an email in the brief matches; a first name alone doesn't.
        let r = rank("Quick question", None, &[who("Ana Ruiz", "")], &ws);
        assert_eq!(
            (r[0].id.as_str(), r[0].people.clone()),
            ("b", vec!["Ana Ruiz".to_string()])
        );
        let r = rank("Quick question", None, &[who("", "ANA@example.com")], &ws);
        assert_eq!(r[0].people, ["ANA@example.com"]);
        assert!(rank("Quick question", None, &[who("Bob", "")], &ws).is_empty());
        assert!(
            rank("Quick question", None, &[who("Ana Ruizz", "")], &ws).is_empty(),
            "word boundaries"
        );
        // Words and people add up: a person plus a word beats a word.
        ws.push(Candidate {
            brief: "ana ruiz".into(),
            ..cand("d", "Garage rates")
        });
        let r = rank("Garage rates", None, &[who("Ana Ruiz", "")], &ws);
        assert_eq!(r[0].id, "d");
        // A person in a fifth or more of five briefs or more (you) says nothing.
        let everyone: Vec<Candidate> = (0..5)
            .map(|i| Candidate {
                brief: "alex morgan".into(),
                ..cand(&format!("w{i}"), &format!("Thing {i}"))
            })
            .collect();
        assert!(rank("Hello", None, &[who("Alex Morgan", "")], &everyone).is_empty());
    }

    #[test]
    fn a_meeting_lists_its_attendees_and_their_reads_are_cached_for_the_day() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        let w = crate::workspaces::create(v, "Lot survey", "x").unwrap();
        let brief = crate::workspaces::folder(v, &w.id)
            .unwrap()
            .join("WORKSPACE.md");
        let text = std::fs::read_to_string(&brief).unwrap();
        std::fs::write(&brief, format!("{text}\nOwner: Ana Ruiz\n")).unwrap();
        let now = DateTime::parse_from_rfc3339("2026-10-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let view = json!({ "connected": true, "complete": true, "error": null, "events": [
            { "id": "e1", "subject": "Quick question", "start": "2026-10-10T13:00:00Z", "end": "2026-10-10T13:30:00Z" },
            { "id": "e2", "subject": "Other", "start": "2026-10-10T14:00:00Z", "end": "2026-10-10T14:30:00Z" },
            { "id": "e3", "subject": "Tomorrow", "start": "2026-10-11T14:00:00Z", "end": "2026-10-11T14:30:00Z" },
        ]});
        assert_eq!(upcoming_ids(&view, now, 12), ["e1", "e2"]);
        let day = chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap();
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fetch = |calls: std::sync::Arc<std::sync::atomic::AtomicUsize>| {
            move |ids: Vec<String>| async move {
                calls.fetch_add(ids.len(), std::sync::atomic::Ordering::SeqCst);
                ids.into_iter()
                    .map(|id| {
                        let got = if id == "e1" {
                            Ok(vec![("Ana Ruiz".to_string(), "ana@x.com".to_string())])
                        } else {
                            Err("Microsoft Graph 503".to_string())
                        };
                        (id, got)
                    })
                    .collect::<BTreeMap<_, _>>()
            }
        };
        let ids: Vec<String> = vec!["e1".into(), "e2".into()];
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let cache: AttendeeCache = Mutex::new(None);
        let got = rt.block_on(attendees_for(&cache, &ids, day, fetch(calls.clone())));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
        let r = report(v, &view, now, 12, &got);
        assert_eq!(
            r.meetings[0].attendees.as_deref(),
            Some(&["Ana Ruiz".to_string()][..])
        );
        assert_eq!(r.meetings[0].matches[0].workspace_id, w.id);
        assert_eq!(r.meetings[0].matches[0].people, ["Ana Ruiz"]);
        assert_eq!(r.meetings[1].attendees, None, "a failed read says so");
        // Again today: e1 comes from the cache, only the failed e2 is asked again.
        let again = rt.block_on(attendees_for(&cache, &ids, day, fetch(calls.clone())));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 3);
        assert!(again.contains_key("e1"));
        // Tomorrow the cache starts over.
        rt.block_on(attendees_for(
            &cache,
            &ids,
            day.succ_opt().unwrap(),
            fetch(calls.clone()),
        ));
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 5);
    }

    #[test]
    fn review_fixes_for_names_addresses_projects_and_five_workspaces() {
        // "Last, First (…)" and spacing still name the person.
        assert_eq!(plain_name("Ruiz,  Ana (Contractor)"), "ana ruiz");
        assert!(names("owner: ana ruiz", &who("Ruiz, Ana", "")));
        // An address is bounded: ana@ isn't inside joanna@; a full stop after is fine.
        assert!(!names("cc joanna@example.com", &who("", "ana@example.com")));
        assert!(!names(
            "cc ana@acme.example.au",
            &who("", "ana@example.com")
        ));
        assert!(names("ask ana@example.com.", &who("", "ana@example.com")));
        // With exactly five workspaces a word in one of them still matches.
        let five: Vec<Candidate> = [
            "TipFlow tips",
            "Lot survey",
            "Payroll",
            "Garage rates",
            "Fleet",
        ]
        .iter()
        .enumerate()
        .map(|(i, t)| cand(&format!("w{i}"), t))
        .collect();
        assert_eq!(rank("TipFlow check-in", None, &[], &five)[0].id, "w0");
        // Two of ten workspaces following one project don't make its name common.
        let mut ten: Vec<Candidate> = (0..8)
            .map(|i| cand(&format!("x{i}"), &format!("Thing {i}")))
            .collect();
        ten.push(Candidate {
            project: Some("Atlas pricing".into()),
            ..cand("r1", "Atlas rates")
        });
        ten.push(Candidate {
            project: Some("Atlas pricing".into()),
            ..cand("r2", "Lot list")
        });
        let r = rank("Atlas check-in", None, &[], &ten);
        assert_eq!(
            r.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["r1", "r2"]
        );
        // A roster brief counts at most two people against two subject words.
        let roster = vec![
            Candidate {
                brief: "a b, c d, e f, g h".into(),
                ..cand("roster", "Team list")
            },
            cand("subject", "TipFlow tip"),
            cand("other", "Fleet"),
        ];
        let people: Vec<Attendee> = ["Aa Bb", "Cc Dd", "Ee Ff", "Gg Hh"]
            .iter()
            .map(|n| who(n, ""))
            .collect();
        let roster: Vec<Candidate> = roster
            .into_iter()
            .map(|c| Candidate {
                brief: c
                    .brief
                    .replace("a b, c d, e f, g h", "aa bb, cc dd, ee ff, gg hh"),
                ..c
            })
            .collect();
        assert_eq!(rank("TipFlow tip", None, &people, &roster)[0].id, "subject");
    }
}
