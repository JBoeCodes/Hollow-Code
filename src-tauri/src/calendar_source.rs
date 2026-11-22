//! Read-only Agenda source selection. Native access lives behind `Reader`;
//! tests supply fakes and never ask macOS for permission or calendar data.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Mutex;

static SETTINGS_LOCK: Mutex<()> = Mutex::new(());
static SOURCE_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn generation() -> u64 {
    SOURCE_GENERATION.load(std::sync::atomic::Ordering::SeqCst)
}

#[cfg(target_os = "macos")]
#[cfg_attr(test, allow(dead_code))] // Compiled for API checking; never invoked by tests.
mod native;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Mac,
    #[default]
    Microsoft,
    Off,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub source: Source,
    /// None means all calendars; Some([]) means none, never all.
    pub calendar_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Calendar {
    pub id: String,
    pub title: String,
    pub account: String,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub permission: String,
    pub calendars: Vec<Calendar>,
    pub events: Vec<Value>,
    pub skipped: usize,
}

/// One malformed occurrence must not hide readable events from other calendars.
#[cfg(any(target_os = "macos", test))]
fn usable_events<T>(events: impl Iterator<Item = Result<T, String>>) -> (Vec<T>, usize) {
    let mut skipped = 0;
    let valid = events
        .filter_map(|e| match e {
            Ok(e) => Some(e),
            Err(_) => {
                skipped += 1;
                None
            }
        })
        .collect();
    (valid, skipped)
}

pub trait Reader {
    fn read(&self, from: &str, to: &str, selected: Option<&[String]>) -> Result<Snapshot, String>;
}

fn read_at(path: &Path) -> Result<Settings, String> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|_| {
            "Calendar settings are damaged. Choose a source again in Settings → Connectors.".into()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
        Err(e) => Err(format!("Calendar settings could not be read: {e}")),
    }
}

pub fn settings() -> Result<Settings, String> {
    let _lock = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_at(&crate::agent_settings::app_data_dir()?.join("agenda-calendar.json"))
}

pub fn read_with(settings: &Settings, from: &str, to: &str, reader: &impl Reader) -> Value {
    if settings.source == Source::Off {
        return json!({"provider": "off", "source": "Off", "status": "Calendar is off", "connected": false, "complete": true, "events": [], "fetchedAt": chrono::Utc::now().to_rfc3339(), "error": null, "partial": null});
    }
    match reader.read(from, to, settings.calendar_ids.as_deref()) {
        Err(e) => unavailable(&e),
        Ok(snapshot) => {
            let granted = snapshot.permission == "granted";
            let missing = granted
                && settings.calendar_ids.as_ref().is_some_and(|ids| {
                    ids.iter()
                        .any(|id| !snapshot.calendars.iter().any(|c| &c.id == id))
                });
            let mut reasons = Vec::new();
            if missing {
                reasons.push("A selected calendar is unavailable. Review Calendars to include in Settings → Connectors.".to_string());
            }
            if granted && snapshot.skipped > 0 {
                reasons.push(format!(
                    "{} calendar {} could not be read. Other events are shown; refresh to retry.",
                    snapshot.skipped,
                    if snapshot.skipped == 1 {
                        "event"
                    } else {
                        "events"
                    }
                ));
            }
            json!({"provider": "mac", "source": "Calendars on this Mac · read only", "permission": snapshot.permission,
                "status": if granted { "Connected" } else { "Calendar access is not granted" },
                "connected": granted, "complete": granted && reasons.is_empty(),
                "partial": if reasons.is_empty() { None } else { Some(reasons.join(" ")) },
                "error": null, "events": if granted { snapshot.events } else { vec![] }, "fetchedAt": chrono::Utc::now().to_rfc3339()})
        }
    }
}

pub fn unavailable(error: &str) -> Value {
    json!({"connected": false, "complete": false, "events": [], "error": error, "status": error, "source": "Calendar unavailable", "provider": "unavailable", "partial": null, "fetchedAt": chrono::Utc::now().to_rfc3339()})
}

/// Civil ends are exclusive. Midnight stays unchanged; an inclusive final-day
/// time becomes the following civil midnight, never a fixed 24-hour duration.
#[cfg(any(target_os = "macos", test))]
fn civil_bounds(
    start: chrono::NaiveDateTime,
    end: chrono::NaiveDateTime,
) -> Result<(String, String), String> {
    let end_day = if end.time() == chrono::NaiveTime::MIN {
        end.date()
    } else {
        end.date().succ_opt().ok_or("Invalid all-day end")?
    };
    if end_day <= start.date() {
        return Err("Invalid all-day range".into());
    }
    Ok((
        format!("{}T00:00:00", start.date()),
        format!("{end_day}T00:00:00"),
    ))
}

#[cfg(any(target_os = "macos", test))]
fn participant_email(url: &str) -> String {
    url.get(..7)
        .filter(|s| s.eq_ignore_ascii_case("mailto:"))
        .map(|_| {
            urlencoding::decode(url[7..].split('?').next().unwrap_or_default())
                .map(|s| s.into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

#[cfg(any(target_os = "macos", test))]
fn occurrence_id(calendar: &str, id: &str, occurrence: i64) -> String {
    use sha2::{Digest, Sha256};
    // Tuple encoding avoids delimiter collisions. Graph IDs remain unchanged.
    let encoded = serde_json::to_vec(&(calendar, id, occurrence)).unwrap_or_default();
    format!("mac:{:x}", Sha256::digest(encoded))
}

#[cfg(any(target_os = "macos", test))]
fn participant(name: String, url: &str, current_user: bool) -> Option<Value> {
    (!current_user).then(|| json!({"name": name, "email": participant_email(url)}))
}

struct Platform;
impl Reader for Platform {
    fn read(&self, from: &str, to: &str, selected: Option<&[String]>) -> Result<Snapshot, String> {
        // Native entry points are impossible in the Rust test binary.
        #[cfg(all(target_os = "macos", not(test)))]
        {
            native::Native.read(from, to, selected)
        }
        #[cfg(any(not(target_os = "macos"), test))]
        {
            let _ = (from, to, selected);
            Err("Calendars on this Mac is unavailable in this runtime".into())
        }
    }
}

pub fn native_view(settings: &Settings, from: &str, to: &str) -> Value {
    read_with(settings, from, to, &Platform)
}

pub fn status() -> Value {
    match settings() {
        Err(e) => {
            json!({"settings": {"source": "off", "calendarIds": null}, "settingsValid": false, "permission": "notRequested", "calendars": [], "error": e})
        }
        Ok(settings) => {
            let snapshot = if settings.source == Source::Mac {
                Platform.read("", "", None)
            } else {
                Ok(Snapshot {
                    permission: "notRequested".into(),
                    calendars: vec![],
                    events: vec![],
                    skipped: 0,
                })
            };
            match snapshot {
                Ok(s) => {
                    json!({"settings": settings, "permission": s.permission, "calendars": s.calendars, "error": null})
                }
                Err(e) => {
                    json!({"settings": settings, "permission": "unavailable", "calendars": [], "error": e})
                }
            }
        }
    }
}

fn save_at(path: &Path, settings: &Settings) -> Result<(), String> {
    if settings.calendar_ids.as_ref().is_some_and(|ids| {
        ids.len() > 1000 || ids.iter().any(|id| id.is_empty() || id.len() > 2048)
    }) {
        return Err("Invalid calendar selection".into());
    }
    crate::vault::atomic_write(
        path,
        &serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?,
    )
}

pub fn save(settings: Settings, request_access: bool) -> Result<Value, String> {
    let request = {
        let _lock = SETTINGS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = crate::agent_settings::app_data_dir()?.join("agenda-calendar.json");
        save_at(&path, &settings)?;
        SOURCE_GENERATION.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::agenda::forget_events();
        settings.source == Source::Mac && request_access
    };
    if request {
        #[cfg(all(target_os = "macos", not(test)))]
        if let Err(e) = native::request() {
            let mut view = status();
            view["error"] = json!(e);
            return Ok(view);
        }
    }
    Ok(status())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn malformed_occurrences_leave_good_events_visible_and_partial() {
        let (events, skipped) = usable_events(
            [
                Ok(json!({"id": "good", "title": "Readable"})),
                Err("missing identifier".into()),
                Err("invalid civil range".into()),
            ]
            .into_iter(),
        );
        assert_eq!(events.len(), 1);
        assert_eq!(skipped, 2);
        struct Partial(Snapshot);
        impl Reader for Partial {
            fn read(&self, _: &str, _: &str, _: Option<&[String]>) -> Result<Snapshot, String> {
                Ok(self.0.clone())
            }
        }
        let view = read_with(
            &Settings {
                source: Source::Mac,
                calendar_ids: None,
            },
            "a",
            "b",
            &Partial(Snapshot {
                permission: "granted".into(),
                calendars: vec![],
                events,
                skipped,
            }),
        );
        assert_eq!(view["connected"], true);
        assert_eq!(view["complete"], false);
        assert_eq!(view["events"][0]["id"], "good");
        assert!(view["partial"]
            .as_str()
            .unwrap()
            .contains("2 calendar events"));
    }

    #[test]
    fn sources_share_local_date_and_rfc3339_window_contract() {
        let (a, b) = crate::m365_mcp::window(Some("2026-11-01"), Some("2026-11-02")).unwrap();
        let a = chrono::DateTime::parse_from_rfc3339(&a)
            .unwrap()
            .with_timezone(&chrono::Local);
        let b = chrono::DateTime::parse_from_rfc3339(&b)
            .unwrap()
            .with_timezone(&chrono::Local);
        assert_eq!(a.date_naive().to_string(), "2026-11-01");
        assert_eq!(b.date_naive().to_string(), "2026-11-02");
        assert_eq!(a.time(), chrono::NaiveTime::MIN);
        assert!(crate::m365_mcp::window(Some("2026-01-01"), Some("2026-05-01")).is_err());
        assert!(crate::m365_mcp::window(
            Some("2026-11-01T01:30:00-04:00"),
            Some("2026-11-01T01:30:00-05:00")
        )
        .is_ok());
    }

    #[test]
    fn civil_ends_are_exclusive_across_dst_and_occurrences_do_not_collide() {
        let start = "2026-10-31T00:00:00".parse().unwrap();
        let midnight = "2026-11-02T00:00:00".parse().unwrap();
        let inclusive = "2026-11-01T23:59:59".parse().unwrap();
        let expected = ("2026-10-31T00:00:00".into(), "2026-11-02T00:00:00".into());
        assert_eq!(civil_bounds(start, midnight).unwrap(), expected);
        assert_eq!(civil_bounds(start, inclusive).unwrap(), expected);
        assert_ne!(
            occurrence_id("work", "series", 1),
            occurrence_id("work", "series", 2)
        );
        assert_ne!(
            occurrence_id("work", "series", 1),
            occurrence_id("personal", "series", 1)
        );
        assert_eq!(
            participant_email("MAILTO:ana%2Bwork@example.test?subject=x"),
            "ana+work@example.test"
        );
        assert_eq!(participant_email("https://example.test/person"), "");
        assert!(participant("Me".into(), "mailto:me@example.test", true).is_none());
        assert_eq!(
            participant("Ana".into(), "mailto:ana@example.test", false).unwrap()["email"],
            "ana@example.test"
        );
    }

    #[test]
    fn saving_none_and_all_calendars_is_distinct_and_invalid_saves_preserve_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("source.json");
        let mut s = Settings {
            source: Source::Mac,
            calendar_ids: Some(vec![]),
        };
        save_at(&path, &s).unwrap();
        assert_eq!(read_at(&path).unwrap(), s);
        s.calendar_ids = None;
        save_at(&path, &s).unwrap();
        assert_eq!(read_at(&path).unwrap(), s);
        let prior = std::fs::read(&path).unwrap();
        s.calendar_ids = Some(vec![String::new()]);
        assert!(save_at(&path, &s).is_err());
        assert_eq!(std::fs::read(path).unwrap(), prior);
    }

    struct Fake {
        calls: Cell<usize>,
        permission: &'static str,
    }
    impl Reader for Fake {
        fn read(&self, _: &str, _: &str, _: Option<&[String]>) -> Result<Snapshot, String> {
            self.calls.set(self.calls.get() + 1);
            Ok(Snapshot {
                permission: self.permission.into(),
                calendars: vec![Calendar {
                    id: "work".into(),
                    title: "Work".into(),
                    account: "Account".into(),
                }],
                events: vec![
                    json!({"id": "mac:series:1", "title": "DST meeting", "start": "2026-11-01T01:30:00-04:00", "end": "2026-11-01T01:30:00-05:00", "attendees": [{"name": "Ana", "email": "ana@example.test"}]}),
                    json!({"id": "mac:series:2", "title": "Two days", "start": "2026-10-24T00:00:00", "end": "2026-10-26T00:00:00", "allDay": true, "attendees": null}),
                ],
                skipped: 0,
            })
        }
    }

    #[test]
    fn off_never_reads_and_mac_denial_is_a_state() {
        let fake = Fake {
            calls: Cell::new(0),
            permission: "denied",
        };
        let off = read_with(
            &Settings {
                source: Source::Off,
                calendar_ids: None,
            },
            "a",
            "b",
            &fake,
        );
        assert_eq!(fake.calls.get(), 0);
        assert_eq!(off["provider"], "off");
        assert_eq!(off["connected"], false);
        let denied = read_with(
            &Settings {
                source: Source::Mac,
                calendar_ids: None,
            },
            "a",
            "b",
            &fake,
        );
        assert_eq!(denied["permission"], "denied");
        assert_eq!(denied["complete"], false);
        assert_eq!(denied["connected"], false);
        assert!(denied["status"].as_str().unwrap().contains("Calendar"));
    }

    #[test]
    fn native_view_keeps_instants_civil_ends_and_unknown_attendees() {
        let fake = Fake {
            calls: Cell::new(0),
            permission: "granted",
        };
        let view = read_with(
            &Settings {
                source: Source::Mac,
                calendar_ids: Some(vec!["work".into()]),
            },
            "a",
            "b",
            &fake,
        );
        assert_eq!(view["provider"], "mac");
        assert_eq!(view["complete"], true);
        assert_eq!(view["events"][0]["start"], "2026-11-01T01:30:00-04:00");
        assert_eq!(view["events"][0]["end"], "2026-11-01T01:30:00-05:00");
        assert_eq!(view["events"][1]["end"], "2026-10-26T00:00:00");
        assert!(view["events"][1]["attendees"].is_null());
        assert_eq!(view["events"][0]["attendees"][0]["name"], "Ana");
        let missing = read_with(
            &Settings {
                source: Source::Mac,
                calendar_ids: Some(vec!["vanished".into()]),
            },
            "a",
            "b",
            &fake,
        );
        assert_eq!(missing["complete"], false);
        assert!(missing["partial"].as_str().unwrap().contains("unavailable"));
    }

    #[test]
    fn settings_default_to_microsoft_but_damage_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("calendar.json");
        assert_eq!(read_at(&path).unwrap().source, Source::Microsoft);
        std::fs::write(&path, "broken").unwrap();
        assert!(read_at(&path).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }
}
