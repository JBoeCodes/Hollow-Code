//! The phone API's Calendar route (v1.2, build run 11-17 row B3, ENG-219;
//! contract `docs/phone-api/calendar.md`, fixtures
//! `docs/phone-api-fixtures/calendar/`): read-only.
//!
//! `GET /v1/calendar?from=&to=` lists the meetings the Agenda reads from the
//! selected calendar source (Mac or Microsoft, [`crate::agenda::events_with`])
//! for `[from, to)`, by default the Mac's local day. Meetings are work data,
//! so the route follows the companion's work-data rule and fails closed: with
//! "include work data" off (or its scope file unreadable) it answers
//! `enabled: false` and no events, and never reads the calendar at all. It
//! writes nothing, asks for no calendar permission (a denied or unset source
//! comes back as the Agenda's own `error`) and sends only what the Today tab
//! draws: id, title, start, end, all-day, cancelled, a short location, the
//! organiser's name and an `https` join link. Attendees, bodies and web links
//! stay on the Mac.

use super::live::cut_chars;
use super::routes::{self, json, ApiError, Ctx, Reply};
use chrono::{DateTime, Duration, Local, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;
use std::sync::Arc;

/// The longest window one request may ask for.
const WINDOW_MAX_DAYS: i64 = 8;
/// Meetings a reply lists, at most (the Agenda's own read stops at 200).
const EVENTS_MAX: usize = 100;
const TITLE_MAX: usize = 200;
const LOCATION_MAX: usize = 200;
const ORGANIZER_MAX: usize = 120;
const JOIN_URL_MAX: usize = 2_000;

/// One meeting as the phone sees it.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meeting {
    pub id: String,
    pub title: String,
    /// As the source wrote it: an instant with an offset, or a civil time
    /// for an all-day event (the phone reads both the way the Mac's Agenda
    /// does).
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub cancelled: bool,
    pub location: Option<String>,
    pub organizer: Option<String>,
    pub join_url: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarBody {
    /// False when the companion's "include work data" is off: nothing was
    /// read and `events` is empty by design, not because the day is free.
    pub enabled: bool,
    /// Hollow has a calendar source it could read (a Microsoft sign-in, or
    /// the Mac's permission); false says to connect one on the Mac.
    pub connected: bool,
    /// The read covered the whole window; false, with the reason in
    /// `partial`, when it stopped early.
    pub complete: bool,
    pub partial: Option<String>,
    /// The Agenda's own words when the read failed.
    pub error: Option<String>,
    pub source: Option<String>,
    pub fetched_at: Option<String>,
    pub from: String,
    pub to: String,
    pub events: Vec<Meeting>,
}

/// An `https` link, at most [`JOIN_URL_MAX`] characters, no spaces or
/// control characters.
fn https_link(value: &Value) -> Option<String> {
    let url = value.as_str()?.trim();
    (url.len() <= JOIN_URL_MAX
        && url.to_ascii_lowercase().starts_with("https://")
        && !url.chars().any(|c| c.is_control() || c.is_whitespace()))
    .then(|| url.to_string())
}

fn text(value: &Value, max: usize) -> Option<String> {
    let t = value.as_str()?.trim();
    (!t.is_empty()).then(|| cut_chars(t, max))
}

/// The meetings in an Agenda read (`events_with`'s view), in the source's
/// order, an event without an id or a start dropped, at most
/// [`EVENTS_MAX`].
pub(crate) fn meetings(view: &Value) -> Vec<Meeting> {
    let Some(events) = view["events"].as_array() else {
        return Vec::new();
    };
    events
        .iter()
        .filter_map(|e| {
            let id = text(&e["id"], 400)?;
            let start = text(&e["start"], 40)?;
            let end = text(&e["end"], 40).unwrap_or_else(|| start.clone());
            Some(Meeting {
                id,
                title: text(&e["title"], TITLE_MAX)
                    .or_else(|| text(&e["subject"], TITLE_MAX))
                    .unwrap_or_else(|| "(No title)".to_string()),
                start,
                end,
                all_day: e["allDay"].as_bool().unwrap_or(false),
                cancelled: e["cancelled"].as_bool().unwrap_or(false),
                location: text(&e["location"], LOCATION_MAX),
                organizer: text(&e["organizer"], ORGANIZER_MAX),
                join_url: https_link(&e["joinUrl"]),
            })
        })
        .take(EVENTS_MAX)
        .collect()
}

/// The body for a window and an Agenda read.
pub(crate) fn body(from: &str, to: &str, view: &Value) -> CalendarBody {
    CalendarBody {
        enabled: true,
        connected: view["connected"].as_bool().unwrap_or(true),
        complete: view["complete"].as_bool().unwrap_or(true),
        partial: text(&view["partial"], 300),
        error: text(&view["error"], 300),
        source: text(&view["source"], 80),
        fetched_at: text(&view["fetchedAt"], 40),
        from: from.to_string(),
        to: to.to_string(),
        events: meetings(view),
    }
}

/// What a phone with work data off gets.
fn disabled(from: &str, to: &str) -> CalendarBody {
    CalendarBody {
        enabled: false,
        connected: false,
        complete: true,
        partial: None,
        error: None,
        source: None,
        fetched_at: None,
        from: from.to_string(),
        to: to.to_string(),
        events: Vec::new(),
    }
}

fn instant(raw: &str, name: &str) -> Result<DateTime<Utc>, ApiError> {
    // `query_param` reads a literal `+` as a space (form encoding), which would break an offset east
    // of UTC (`+05:30`) from a client that didn't percent-encode it: a space can't be part of an
    // RFC 3339 time, so read it as the plus it was meant to be.
    let raw = raw.replace(' ', "+");
    DateTime::parse_from_rfc3339(&raw)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|_| {
            ApiError::bad_request(format!(
                "{name} is an RFC 3339 time, like 2026-11-19T00:00:00-05:00."
            ))
        })
}

/// The window the query names: both of `from` and `to`, or neither (the
/// Mac's local day, midnight to midnight). `to` is after `from` and at most
/// [`WINDOW_MAX_DAYS`] later. Both come back in UTC RFC 3339, the form the
/// Agenda reads.
pub(crate) fn window(query: &str, now: DateTime<Local>) -> Result<(String, String), ApiError> {
    // A name that is there with nothing (or nothing readable) after it is a bad request, not "neither".
    let given = |name: &str| -> Result<Option<String>, ApiError> {
        let present = query
            .split('&')
            .any(|pair| pair.split_once('=').unwrap_or((pair, "")).0 == name);
        match (
            present,
            routes::query_param(query, name).filter(|v| !v.is_empty()),
        ) {
            (true, None) => Err(ApiError::bad_request(format!(
                "{name} is an RFC 3339 time."
            ))),
            (_, value) => Ok(value),
        }
    };
    let from = given("from")?;
    let to = given("to")?;
    let (a, b) = match (from, to) {
        (None, None) => {
            // A zone whose clock skips 00:00 on a change day starts its day at 01:00.
            let midnight = [0, 1]
                .into_iter()
                .find_map(|h| {
                    now.date_naive()
                        .and_hms_opt(h, 0, 0)
                        .and_then(|d| Local.from_local_datetime(&d).earliest())
                })
                .ok_or_else(|| ApiError::internal("the local day can't be read".to_string()))?;
            let start = midnight.with_timezone(&Utc);
            // The next midnight, not +24 hours: a day may be 23 or 25 hours.
            let next = (now.date_naive() + Duration::days(1))
                .and_hms_opt(0, 0, 0)
                .and_then(|d| Local.from_local_datetime(&d).earliest())
                .map_or(start + Duration::hours(24), |t| t.with_timezone(&Utc));
            (start, next)
        }
        (Some(f), Some(t)) => (instant(&f, "from")?, instant(&t, "to")?),
        _ => {
            return Err(ApiError::bad_request(
                "from and to come together: send both, or neither for today.".to_string(),
            ))
        }
    };
    if b <= a {
        return Err(ApiError::bad_request("to is after from.".to_string()));
    }
    if b - a > Duration::days(WINDOW_MAX_DAYS) {
        return Err(ApiError::bad_request(format!(
            "The window is at most {WINDOW_MAX_DAYS} days."
        )));
    }
    let rfc = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    Ok((rfc(a), rfc(b)))
}

pub(crate) async fn list(ctx: &Arc<Ctx>, query: &str) -> Result<Reply, ApiError> {
    let (from, to) = window(query, Local::now())?;
    // Fails closed: off, absent or unreadable scope reads nothing.
    if !crate::companion_docs::work_data_on(ctx.store.dir()) {
        return Ok(json(200, &disabled(&from, &to)));
    }
    let view = crate::agenda::events_with(&from, &to, crate::agenda::Reuse::Kept).await;
    Ok(json(200, &body(&from, &to, &view)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_mac_view_and_a_graph_view_both_become_meetings() {
        let view = json!({
            "connected": true, "complete": true, "partial": null, "error": null,
            "source": "Mac Calendar", "fetchedAt": "2026-11-19T09:00:00Z",
            "events": [
                {"id": "mac:1", "title": "Standup", "start": "2026-11-19T09:30:00-05:00",
                 "end": "2026-11-19T09:45:00-05:00", "allDay": false,
                 "attendees": [{"name": "Ana", "email": "ana@example.test"}],
                 "location": "Room 2", "joinUrl": "https://meet.example.test/x"},
                {"id": "g:2", "subject": "Board", "start": "2026-11-19T00:00:00",
                 "end": "2026-11-20T00:00:00", "allDay": true, "cancelled": true,
                 "organizer": "Dee", "webLink": "https://outlook.example.test/e",
                 "joinUrl": "http://insecure.example.test/j"},
                {"title": "No id", "start": "2026-11-19T10:00:00-05:00"},
                {"id": "x", "title": "No start"}
            ]
        });
        let b = body("a", "b", &view);
        assert!(b.enabled && b.connected && b.complete);
        assert_eq!(b.events.len(), 2);
        assert_eq!(b.events[0].title, "Standup");
        assert_eq!(b.events[0].location.as_deref(), Some("Room 2"));
        assert_eq!(
            b.events[0].join_url.as_deref(),
            Some("https://meet.example.test/x")
        );
        assert_eq!(b.events[1].title, "Board");
        assert!(b.events[1].all_day && b.events[1].cancelled);
        assert_eq!(b.events[1].join_url, None, "only https links go out");
        let wire = serde_json::to_string(&b).unwrap();
        assert!(
            !wire.contains("ana@example.test"),
            "attendees stay on the Mac"
        );
        assert!(
            !wire.contains("outlook.example.test"),
            "web links stay on the Mac"
        );
    }

    #[test]
    fn a_failed_or_partial_read_says_so() {
        let b = body(
            "a",
            "b",
            &json!({"connected": true, "events": [], "complete": false,
                    "partial": "page 2 failed", "error": null}),
        );
        assert!(!b.complete);
        assert_eq!(b.partial.as_deref(), Some("page 2 failed"));
        let b = body(
            "a",
            "b",
            &json!({"connected": false, "events": [], "complete": true,
                    "source": "Microsoft 365 (not signed in)"}),
        );
        assert!(!b.connected && b.events.is_empty());
    }

    #[test]
    fn work_data_off_is_an_empty_day_that_says_so() {
        let b = disabled("a", "b");
        assert!(!b.enabled && b.events.is_empty() && b.error.is_none());
        let wire = serde_json::to_value(&b).unwrap();
        assert_eq!(wire["enabled"], false);
        assert_eq!(wire["events"], json!([]));
    }

    #[test]
    fn long_text_is_cut_and_the_list_is_capped() {
        let events: Vec<Value> = (0..150)
            .map(|i| {
                json!({"id": format!("e{i}"), "title": "t".repeat(500),
                       "start": "2026-11-19T09:00:00Z", "end": "2026-11-19T10:00:00Z"})
            })
            .collect();
        let m = meetings(&json!({ "events": events }));
        assert_eq!(m.len(), EVENTS_MAX);
        assert!(m[0].title.chars().count() <= TITLE_MAX + 1);
    }

    #[test]
    fn the_window_is_today_or_two_times_at_most_eight_days_apart() {
        let now = Local.with_ymd_and_hms(2026, 11, 19, 15, 0, 0).unwrap();
        let (a, b) = window("", now).unwrap();
        let (a, b) = (
            DateTime::parse_from_rfc3339(&a).unwrap(),
            DateTime::parse_from_rfc3339(&b).unwrap(),
        );
        assert!(a <= now && now < b);
        assert!(b - a >= Duration::hours(23) && b - a <= Duration::hours(25));
        assert!(window("from=2026-11-19T00:00:00Z&to=2026-11-20T00:00:00Z", now).is_ok());
        // A literal plus in an offset (not percent-encoded) reads as the plus it was meant to be.
        let (a, b) = window(
            "from=2026-11-19T00:00:00+05:30&to=2026-11-19T12:00:00%2B05:30",
            now,
        )
        .unwrap();
        assert_eq!(
            (a.as_str(), b.as_str()),
            ("2026-11-18T18:30:00Z", "2026-11-19T06:30:00Z")
        );
        for bad in [
            "from=&to=",
            "from=%ZZ&to=%ZZ",
            "from=2026-11-19T00:00:00Z",
            "to=2026-11-19T00:00:00Z",
            "from=yesterday&to=today",
            "from=2026-11-20T00:00:00Z&to=2026-11-19T00:00:00Z",
            "from=2026-11-19T00:00:00Z&to=2026-12-19T00:00:00Z",
        ] {
            assert_eq!(window(bad, now).unwrap_err().status, 400, "{bad}");
        }
    }
}
