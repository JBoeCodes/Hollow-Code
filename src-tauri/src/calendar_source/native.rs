//! EventKit objects remain on one blocking thread and within an autorelease
//! pool. Only owned Rust data crosses the boundary. No event writer exists.
use super::{Calendar, Reader, Snapshot};
use block2::RcBlock;
use objc2::{rc::autoreleasepool, runtime::Bool, sel};
use objc2_event_kit::{
    EKAuthorizationStatus, EKEntityType, EKEventAvailability, EKEventStatus, EKEventStore,
};
use objc2_foundation::{NSArray, NSDate, NSError, NSObjectProtocol};
use serde_json::json;

pub struct Native;

fn permission() -> &'static str {
    // Querying authorization does not request it or instantiate an event store.
    match unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Event) } {
        EKAuthorizationStatus::FullAccess => "granted",
        EKAuthorizationStatus::NotDetermined => "notDetermined",
        EKAuthorizationStatus::Restricted => "restricted",
        EKAuthorizationStatus::WriteOnly => "writeOnly",
        _ => "denied",
    }
}

/// Called only by the explicit source-choice command, never status or reads.
pub fn request() -> Result<(), String> {
    if matches!(permission(), "granted" | "denied" | "restricted") {
        return Ok(());
    }
    autoreleasepool(|_| unsafe {
        let store = EKEventStore::new();
        let (tx, rx) = std::sync::mpsc::channel();
        let block = RcBlock::new(move |_granted: Bool, error: *mut NSError| {
            let message = error.as_ref().map(|e| e.localizedDescription().to_string());
            let _ = tx.send(message);
        });
        // macOS 14 added full-access. Retain the macOS 11 deployment target.
        if store.respondsToSelector(sel!(requestFullAccessToEventsWithCompletion:)) {
            store.requestFullAccessToEventsWithCompletion(RcBlock::as_ptr(&block));
        } else {
            #[allow(deprecated)]
            store
                .requestAccessToEntityType_completion(EKEntityType::Event, RcBlock::as_ptr(&block));
        }
        match rx.recv_timeout(std::time::Duration::from_secs(120)) {
            Ok(None) => Ok(()),
            Ok(Some(e)) => Err(e),
            Err(_) => Err("Calendar access has not been answered. Refresh calendars after answering the macOS prompt.".into()),
        }
    })
}

fn timestamp(date: &NSDate) -> Result<chrono::DateTime<chrono::Utc>, String> {
    chrono::DateTime::from_timestamp_millis((date.timeIntervalSince1970() * 1000.0) as i64)
        .ok_or_else(|| "Calendar returned an invalid date".into())
}

impl Reader for Native {
    fn read(&self, from: &str, to: &str, selected: Option<&[String]>) -> Result<Snapshot, String> {
        let access = permission();
        let mut out = Snapshot {
            permission: access.into(),
            calendars: vec![],
            events: vec![],
            skipped: 0,
        };
        if access != "granted" {
            return Ok(out);
        }
        autoreleasepool(|_| unsafe {
            let store = EKEventStore::new();
            let calendars = store.calendarsForEntityType(EKEntityType::Event);
            for calendar in &calendars {
                out.calendars.push(Calendar {
                    id: calendar.calendarIdentifier().to_string(),
                    title: calendar.title().to_string(),
                    account: calendar
                        .source()
                        .map(|s| s.title().to_string())
                        .unwrap_or_default(),
                });
            }
            // Empty window is metadata only (Settings). No event read.
            if from.is_empty() && to.is_empty() {
                return Ok(out);
            }
            let (a, b) = crate::m365_mcp::window(Some(from), Some(to))?;
            let a = chrono::DateTime::parse_from_rfc3339(&a).map_err(|e| e.to_string())?;
            let b = chrono::DateTime::parse_from_rfc3339(&b).map_err(|e| e.to_string())?;
            let chosen: Vec<_> = calendars
                .iter()
                .filter(|c| {
                    selected.is_none_or(|ids| ids.contains(&c.calendarIdentifier().to_string()))
                })
                .collect();
            // EventKit treats nil as all. Never send nil for an empty selection.
            if chosen.is_empty() {
                return Ok(out);
            }
            let chosen = NSArray::from_retained_slice(&chosen);
            let predicate = store.predicateForEventsWithStartDate_endDate_calendars(
                &NSDate::dateWithTimeIntervalSince1970(a.timestamp_millis() as f64 / 1000.0),
                &NSDate::dateWithTimeIntervalSince1970(b.timestamp_millis() as f64 / 1000.0),
                Some(&chosen),
            );
            let events = store.eventsMatchingPredicate(&predicate);
            let (mut valid, skipped) = super::usable_events(events.iter().map(|event| {
                let Some(identifier) = event.eventIdentifier() else {
                    return Err(
                        "Calendar returned an event without an identifier. Refresh calendars."
                            .into(),
                    );
                };
                let start = timestamp(&event.startDate())?;
                let end = timestamp(&event.endDate())?;
                if end < start { return Err("Calendar returned an invalid range".into()); }
                let occurrence = event
                    .occurrenceDate()
                    .map(|d| timestamp(&d))
                    .transpose()?
                    .unwrap_or(start);
                let calendar_id = event
                    .calendar()
                    .map(|c| c.calendarIdentifier().to_string())
                    .unwrap_or_default();
                let all_day = event.isAllDay();
                let (start_text, end_text) = if all_day {
                    super::civil_bounds(
                        start.with_timezone(&chrono::Local).naive_local(),
                        end.with_timezone(&chrono::Local).naive_local(),
                    )?
                } else {
                    (start.to_rfc3339(), end.to_rfc3339())
                };
                let attendees = event.attendees().map(|list| {
                    list.iter()
                        .filter_map(|p| {
                            let url = p
                                .URL()
                                .absoluteString()
                                .map(|s| s.to_string())
                                .unwrap_or_default();
                            super::participant(
                                p.name().map(|s| s.to_string()).unwrap_or_default(),
                                &url,
                                p.isCurrentUser(),
                            )
                        })
                        .collect::<Vec<_>>()
                });
                Ok((start, json!({
                    "id": super::occurrence_id(&calendar_id, &identifier.to_string(), occurrence.timestamp_millis()),
                    "title": event.title().to_string(), "start": start_text, "end": end_text,
                    "allDay": all_day, "cancelled": event.status() == EKEventStatus::Canceled,
                    "location": event.location().map(|s| s.to_string()),
                    "organizer": event.organizer().and_then(|p| p.name()).map(|s| s.to_string()),
                    "showAs": if event.availability() == EKEventAvailability::Free { "free" } else { "busy" },
                    "attendees": attendees,
                })))
            }));
            valid.sort_by_key(|(start, _)| *start);
            out.events = valid.into_iter().map(|(_, event)| event).collect();
            out.skipped = skipped;
            // Permission can be revoked during the read: don't publish a complete empty day.
            if permission() != "granted" {
                out.permission = permission().into();
                out.events.clear();
            }
            Ok(out)
        })
    }
}
