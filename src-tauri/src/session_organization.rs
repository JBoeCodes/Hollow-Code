//! Conversation organization shares the archive bit with the existing lifecycle.
//! Compare both revision and activity before writes so stale menus cannot hide
//! new work. A new turn or elapsed snooze makes the row active without a writer.
use super::*;

#[path = "session_settlement.rs"]
pub mod settlement;

pub(super) const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS session_organization (
    session_key TEXT PRIMARY KEY, pinned INTEGER NOT NULL DEFAULT 0,
    snoozed_until TEXT, reason TEXT, activity_at TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0
); CREATE TABLE IF NOT EXISTS session_settlement(session_key TEXT PRIMARY KEY, policy TEXT NOT NULL);";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub key: String,
    pub pinned: bool,
    pub snoozed_until: Option<String>,
    pub settled: bool,
    pub reason: Option<String>,
    pub revision: i64,
    pub auto_settle: Option<settlement::Policy>,
    pub session: AgentSessionSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub key: String,
    pub action: String,
    pub until: Option<String>,
    pub expected_revision: i64,
    pub expected_activity: String,
}

type SavedOrganization = (bool, Option<String>, Option<String>, String, i64);

fn read_in(conn: &Connection, key: &str, _now: DateTime<Utc>) -> Result<Organization, String> {
    let session = summary_for(conn, key)?;
    let saved: Option<SavedOrganization> = conn
        .query_row("SELECT pinned, snoozed_until, reason, activity_at, revision FROM session_organization WHERE session_key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional().map_err(|e| format!("conversation organization: {e}"))?;
    let (pinned, snooze, reason, activity, revision) =
        saved.unwrap_or((false, None, None, session.last_activity_at.clone(), 0));
    let unchanged = activity == session.last_activity_at;
    let archived: bool = conn
        .query_row("SELECT archived FROM sessions WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .map_err(|e| e.to_string())?;
    let snoozed_until =
        snooze.filter(|s| unchanged && !archived && DateTime::parse_from_rfc3339(s).is_ok());
    let settled = archived && unchanged && reason.is_some();
    Ok(Organization {
        key: key.into(),
        pinned,
        snoozed_until,
        settled,
        reason: reason.filter(|_| settled),
        revision,
        auto_settle: settlement::read(conn, key)?
            .filter(|p| p.revision == revision && p.activity == session.last_activity_at),
        session,
    })
}

pub fn list(vault: &Path) -> Result<Vec<Organization>, String> {
    with_read(vault, |conn| {
        let now = Utc::now();
        let mut stmt = conn.prepare("SELECT o.session_key FROM session_organization o JOIN sessions s ON s.key = o.session_key ORDER BY s.last_activity_at DESC, o.session_key").map_err(|e| e.to_string())?;
        let keys = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        keys.into_iter()
            .map(|key| read_in(conn, &key, now))
            .collect()
    })
}

pub fn change(vault: &Path, change: &Change) -> Result<Organization, String> {
    change_at(vault, change, Utc::now())
}

fn change_at(vault: &Path, change: &Change, now: DateTime<Utc>) -> Result<Organization, String> {
    let key = change.key.as_str();
    if !is_safe_session_key(key) {
        return Err("Invalid conversation ID".into());
    }
    let hiding = matches!(change.action.as_str(), "snooze" | "settle");
    if hiding && crate::agent_sessions::live_for_key(key).is_some() {
        return Err(ARCHIVE_BUSY.into());
    }
    with_write(vault, "conversation organization", |tx| {
        let old = read_in(tx, key, now)?;
        if old.revision != change.expected_revision
            || old.session.last_activity_at != change.expected_activity
        {
            return Err("CONVERSATION_CHANGED: refresh the conversation and try again".into());
        }
        if hiding {
            if old.pinned {
                return Err("Unpin this conversation first".into());
            }
            if chat_turn_queued(tx, key)? {
                return Err(ARCHIVE_BUSY.into());
            }
            let attention: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM interactions WHERE session_key = ?1 AND status IN ('pending','stale'))", [key], |r| r.get(0)).map_err(|e| e.to_string())?;
            // Explicit settlement keeps Archive's card-expiry behavior.
            // Snooze cannot conceal an unanswered card.
            if attention && change.action == "snooze" {
                return Err("This conversation needs your answer".into());
            }
        }
        let (mut pin, mut snooze, mut reason) = (old.pinned, old.snoozed_until, old.reason);
        tx.execute("DELETE FROM session_settlement WHERE session_key=?1", [key])
            .map_err(|e| e.to_string())?;
        match change.action.as_str() {
            "pin" => {
                pin = true;
                snooze = None;
                reason = None;
                tx.execute("UPDATE sessions SET archived = 0 WHERE key = ?1", [key])
                    .map_err(|e| e.to_string())?;
            }
            "unpin" => pin = false,
            "snooze" => {
                let at = change.until.as_deref().ok_or("Choose a snooze time")?;
                let at = DateTime::parse_from_rfc3339(at)
                    .map_err(|_| "Choose a valid snooze time")?
                    .with_timezone(&Utc);
                if at <= now {
                    return Err("Choose a future snooze time".into());
                }
                snooze = Some(db_time(at));
                reason = None;
                tx.execute("UPDATE sessions SET archived = 0 WHERE key = ?1", [key])
                    .map_err(|e| e.to_string())?;
            }
            "wake" => {
                snooze = None;
                reason = None;
                tx.execute("UPDATE sessions SET archived = 0 WHERE key = ?1", [key])
                    .map_err(|e| e.to_string())?;
            }
            "settle" => {
                snooze = None;
                reason = Some("Settled by you".into());
                tx.execute("UPDATE sessions SET archived = 1 WHERE key = ?1", [key])
                    .map_err(|e| e.to_string())?;
                crate::interactions::expire_pending_in(tx, key, &db_time(now), "archived")?;
            }
            _ => return Err("Unknown conversation organization action".into()),
        }
        tx.execute("INSERT INTO session_organization(session_key,pinned,snoozed_until,reason,activity_at,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_key) DO UPDATE SET pinned=excluded.pinned,snoozed_until=excluded.snoozed_until,reason=excluded.reason,activity_at=excluded.activity_at,revision=excluded.revision", params![key,pin,snooze,reason,old.session.last_activity_at,old.revision + 1]).map_err(|e| e.to_string())?;
        read_in(tx, key, now)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str = "s-20261116-0a0b0c01";
    fn setup() -> (tempfile::TempDir, Change, DateTime<Utc>) {
        let dir = tempfile::tempdir().unwrap();
        start_turn(
            dir.path(),
            KEY,
            "codex",
            "/fixture/quick-chats",
            "run-org",
            "hello",
        )
        .unwrap();
        let session = session_by_key(dir.path(), KEY).unwrap().unwrap();
        let now = DateTime::parse_from_rfc3339("2030-01-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        (
            dir,
            Change {
                key: KEY.into(),
                action: "snooze".into(),
                until: Some("2030-01-01T13:00:00Z".into()),
                expected_revision: 0,
                expected_activity: session.last_activity_at,
            },
            now,
        )
    }
    #[test]
    fn legacy_archive_respects_pins_and_clears_old_organization() {
        let (dir, mut c, now) = setup();
        c.action = "pin".into();
        c.expected_revision = change_at(dir.path(), &c, now).unwrap().revision;
        assert!(archive_session(dir.path(), KEY, true)
            .unwrap_err()
            .contains("Unpin"));
        c.action = "unpin".into();
        c.expected_revision = change_at(dir.path(), &c, now).unwrap().revision;
        c.action = "snooze".into();
        let snoozed = change_at(dir.path(), &c, now).unwrap();
        archive_session(dir.path(), KEY, true).unwrap();
        archive_session(dir.path(), KEY, false).unwrap();
        let awake = list(dir.path()).unwrap().remove(0);
        assert!(awake.snoozed_until.is_none());
        assert!(!awake.settled);
        assert!(awake.reason.is_none());
        assert!(awake.revision > snoozed.revision);
    }

    #[test]
    fn queued_turn_refuses_settlement_without_writing_organization() {
        let (dir, mut c, now) = setup();
        c.action = "settle".into();
        with_write(dir.path(), "queue", |tx| {
            tx.execute("INSERT INTO run_queue(source,source_id,label,status,priority,requested_at) VALUES('chat',?1,'Work','queued',0,?2)", params![KEY, db_time(now)]).unwrap();
            Ok(())
        }).unwrap();
        assert!(change_at(dir.path(), &c, now)
            .unwrap_err()
            .contains(ARCHIVE_BUSY));
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
        assert!(list(dir.path()).unwrap().is_empty());
    }

    #[test]
    fn snooze_keeps_question_cards_and_explicit_settlement_expires_them() {
        let (dir, mut c, now) = setup();
        with_write(dir.path(), "question", |tx| {
            tx.execute("INSERT INTO interactions(id,session_key,turn_ord,run_id,agent,tool_use_id,source,mode,request_json,status,created_at) VALUES('org-card',?1,1,'run-org','codex','question','test','default','{}','pending',?2)", params![KEY, db_time(now)]).unwrap();
            Ok(())
        }).unwrap();
        assert!(change_at(dir.path(), &c, now)
            .unwrap_err()
            .contains("needs your answer"));
        c.action = "settle".into();
        assert!(change_at(dir.path(), &c, now).unwrap().settled);
        with_read(dir.path(), |conn| {
            let status: String = conn
                .query_row(
                    "SELECT status FROM interactions WHERE id = 'org-card'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(status, "expired");
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn snooze_expires_and_new_activity_wakes_it() {
        let (dir, c, now) = setup();
        assert!(change_at(dir.path(), &c, now)
            .unwrap()
            .snoozed_until
            .is_some());
        // Keep the deadline for the sidebar's clock, so an expired old chat
        // stays reachable even when it is beyond the ordinary recent cap.
        with_read(dir.path(), |conn| {
            assert!(read_in(conn, KEY, now + Duration::hours(1))?
                .snoozed_until
                .is_some());
            Ok(())
        })
        .unwrap();
        with_write(dir.path(), "new activity", |tx| {
            tx.execute(
                "UPDATE sessions SET last_activity_at = '2030-01-01T12:30:00.000Z' WHERE key = ?1",
                [KEY],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        with_read(dir.path(), |conn| {
            assert!(read_in(conn, KEY, now)?.snoozed_until.is_none());
            Ok(())
        })
        .unwrap();
        assert!(change_at(dir.path(), &c, now)
            .unwrap_err()
            .contains("CONVERSATION_CHANGED"));
    }
    #[test]
    fn settlement_uses_archive_and_revision_guarded_wake() {
        let (dir, mut c, now) = setup();
        c.action = "settle".into();
        let settled = change_at(dir.path(), &c, now).unwrap();
        assert!(settled.settled);
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(true));
        c.action = "wake".into();
        assert!(change_at(dir.path(), &c, now)
            .unwrap_err()
            .contains("CONVERSATION_CHANGED"));
        c.expected_revision = settled.revision;
        assert!(!change_at(dir.path(), &c, now).unwrap().settled);
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
    }
    #[test]
    fn pins_prevent_hiding_and_invalid_times_leave_state_unchanged() {
        let (dir, mut c, now) = setup();
        c.action = "pin".into();
        let pin = change_at(dir.path(), &c, now).unwrap();
        c.expected_revision = pin.revision;
        c.action = "settle".into();
        assert!(change_at(dir.path(), &c, now)
            .unwrap_err()
            .contains("Unpin"));
        c.action = "unpin".into();
        c.expected_revision = change_at(dir.path(), &c, now).unwrap().revision;
        c.action = "snooze".into();
        c.until = Some("2020-01-01T00:00:00Z".into());
        assert!(change_at(dir.path(), &c, now).is_err());
        assert_eq!(list(dir.path()).unwrap()[0].revision, c.expected_revision);
    }
}
