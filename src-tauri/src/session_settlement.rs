//! Opt-in, one-shot settlement. A policy is tied to this revision and activity,
//! never carried into a newer turn. All checks run again inside the write.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    pub kind: String,
    pub activity: String,
    pub revision: i64,
    pub pr: Option<crate::build_changes::SettlementPr>,
}

pub(super) fn read(conn: &Connection, key: &str) -> Result<Option<Policy>, String> {
    let value: Option<String> = conn
        .query_row(
            "SELECT policy FROM session_settlement WHERE session_key = ?1",
            [key],
            |r| r.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    value
        .map(|s| serde_json::from_str(&s).map_err(|e| e.to_string()))
        .transpose()
}

pub async fn configure(vault: &Path, change: &Change) -> Result<Organization, String> {
    if !matches!(change.action.as_str(), "idle" | "merged" | "off") {
        return Err("Choose a settlement policy".into());
    }
    let session = session_by_key(vault, &change.key)?.ok_or("Unknown conversation")?;
    let pr = if change.action == "merged" {
        let (id, lane) = crate::build_workspaces::worktree_lane(vault, Path::new(&session.cwd))
            .ok_or("This conversation has no bound build worktree")?;
        // Pull requests are read for the primary's branch only (F6 part B).
        if lane != crate::build_workspaces::MAIN_LANE {
            return Err("Settling on a merged pull request follows the workspace's own worktree; this conversation is in another one".into());
        }
        let snapshot = crate::build_changes::settlement_pr(vault, &id, None)
            .await
            .ok_or("Could not verify this branch's pull request")?;
        if snapshot.state != "open" {
            return Err("Choose an open pull request before enabling this policy".into());
        }
        Some(snapshot)
    } else {
        None
    };
    with_write(vault, "conversation settlement policy", |tx| {
        let old = read_in(tx, &change.key, Utc::now())?;
        if old.revision != change.expected_revision
            || old.session.last_activity_at != change.expected_activity
            || old.session.cwd != session.cwd
        {
            return Err("CONVERSATION_CHANGED".into());
        }
        if change.action != "off"
            && (crate::agent_sessions::live_for_key(&change.key).is_some()
                || chat_turn_queued(tx, &change.key)?)
        {
            return Err(ARCHIVE_BUSY.into());
        }
        let archived: bool = tx
            .query_row(
                "SELECT archived FROM sessions WHERE key=?1",
                [&change.key],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if change.action != "off" && (archived || old.pinned || old.snoozed_until.is_some()) {
            return Err("Wake and unpin this conversation first".into());
        }
        let revision = old.revision + 1;
        tx.execute("INSERT INTO session_organization(session_key,pinned,snoozed_until,reason,activity_at,revision) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(session_key) DO UPDATE SET pinned=excluded.pinned,snoozed_until=excluded.snoozed_until,reason=excluded.reason,activity_at=excluded.activity_at,revision=excluded.revision", params![change.key, old.pinned, old.snoozed_until, old.reason, old.session.last_activity_at, revision]).map_err(|e| e.to_string())?;
        tx.execute(
            "DELETE FROM session_settlement WHERE session_key = ?1",
            [&change.key],
        )
        .map_err(|e| e.to_string())?;
        if change.action != "off" {
            let policy = Policy {
                kind: change.action.clone(),
                activity: old.session.last_activity_at,
                revision,
                pr: pr.clone(),
            };
            tx.execute(
                "INSERT INTO session_settlement(session_key,policy) VALUES(?1,?2)",
                params![
                    change.key,
                    serde_json::to_string(&policy).map_err(|e| e.to_string())?
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        read_in(tx, &change.key, Utc::now())
    })
}

fn eligible(
    conn: &Connection,
    row: &Organization,
    policy: &Policy,
    now: DateTime<Utc>,
) -> Result<bool, String> {
    if let Some(pr) = &policy.pr {
        let cwd = Path::new(&row.session.cwd).canonicalize().ok();
        if cwd.is_none() || cwd != Path::new(&pr.worktree).canonicalize().ok() {
            return Ok(false);
        }
    }
    if row.pinned
        || row.snoozed_until.is_some()
        || row.revision != policy.revision
        || row.session.last_activity_at != policy.activity
        || crate::agent_sessions::live_for_key(&row.key).is_some()
        || chat_turn_queued(conn, &row.key)?
    {
        return Ok(false);
    }
    let blocked: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE key=?1 AND archived=1) OR EXISTS(SELECT 1 FROM interactions WHERE session_key=?1 AND status IN ('pending','stale')) OR EXISTS(SELECT 1 FROM chat_pauses WHERE session_key=?1)", [&row.key], |r| r.get(0)).map_err(|e| e.to_string())?;
    let last: Option<(Option<String>, Option<String>)> = conn
        .query_row(
            "SELECT status, ended_at FROM turns WHERE session_key=?1 ORDER BY ord DESC LIMIT 1",
            [&row.key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if blocked || !matches!(last, Some((Some(ref status), Some(_))) if status == "success") {
        return Ok(false);
    }
    // A finished parent can still own a running or waiting child. UNION also
    // bounds malformed cycles, so that work remains visible with its parent.
    let mut stmt = conn.prepare("WITH RECURSIVE family(key) AS (SELECT key FROM sessions WHERE parent_key=?1 UNION SELECT s.key FROM sessions s JOIN family f ON s.parent_key=f.key) SELECT key FROM family").map_err(|e| e.to_string())?;
    let descendants = stmt
        .query_map([&row.key], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for key in descendants {
        if crate::agent_sessions::live_for_key(&key).is_some() || chat_turn_queued(conn, &key)? {
            return Ok(false);
        }
        let attention: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM interactions WHERE session_key=?1 AND status IN ('pending','stale')) OR EXISTS(SELECT 1 FROM chat_pauses WHERE session_key=?1) OR COALESCE((SELECT status IS NOT 'success' OR ended_at IS NULL FROM turns WHERE session_key=?1 ORDER BY ord DESC LIMIT 1),0)", [&key], |r| r.get(0)).map_err(|e| e.to_string())?;
        if attention {
            return Ok(false);
        }
    }
    Ok(match policy.kind.as_str() {
        "idle" => DateTime::parse_from_rfc3339(&policy.activity)
            .is_ok_and(|at| now.signed_duration_since(at) >= Duration::days(7)),
        "merged" => policy.pr.is_some(),
        _ => false,
    })
}

fn settle(
    vault: &Path,
    key: &str,
    expected: &Policy,
    now: DateTime<Utc>,
    merged: bool,
) -> Result<Option<Organization>, String> {
    with_write(vault, "automatic conversation settlement", |tx| {
        let Some(policy) = read(tx, key)? else {
            return Ok(None);
        };
        let row = read_in(tx, key, now)?;
        if policy != *expected
            || !eligible(tx, &row, &policy, now)?
            || (policy.kind == "merged" && !merged)
        {
            return Ok(None);
        }
        let reason = policy
            .pr
            .as_ref()
            .map(|pr| format!("Settled after PR #{} merged ({})", pr.number, pr.url))
            .unwrap_or_else(|| "Settled after seven idle days".into());
        tx.execute("UPDATE sessions SET archived=1 WHERE key=?1", [key])
            .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE session_organization SET reason=?2, revision=revision+1 WHERE session_key=?1",
            params![key, reason],
        )
        .map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM session_settlement WHERE session_key=?1", [key])
            .map_err(|e| e.to_string())?;
        // No pending cards exist here; automatic settlement never expires one.
        Ok(Some(read_in(tx, key, now)?))
    })
}

pub async fn sweep(vault: &Path, protected: &[String]) -> Result<Vec<Organization>, String> {
    // One sweep at a time, without a queue. The frontend also coalesces ticks.
    static SWEEP: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let Ok(_guard) = SWEEP.try_lock() else {
        return Ok(vec![]);
    };
    let candidates = with_read(vault, |conn| {
        let mut stmt = conn
            .prepare("SELECT session_key FROM session_settlement ORDER BY session_key")
            .map_err(|e| e.to_string())?;
        let keys = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        let mut candidates = vec![];
        for key in keys {
            if protected.contains(&key) {
                continue;
            }
            let row = read_in(conn, &key, Utc::now())?;
            if let Some(policy) = read(conn, &key)? {
                if eligible(conn, &row, &policy, Utc::now())? {
                    candidates.push((key, policy));
                }
            }
        }
        Ok(candidates)
    })?;
    let mut settled = vec![];
    for (key, policy) in candidates {
        let merged = if let Some(pr) = &policy.pr {
            crate::build_changes::settlement_pr(vault, &pr.workspace_id, Some(pr))
                .await
                .is_some_and(|fresh| fresh.state == "merged")
        } else {
            false
        };
        if let Some(row) = settle(vault, &key, &policy, Utc::now(), merged)? {
            settled.push(row);
        }
    }
    Ok(settled)
}

#[cfg(test)]
mod tests {
    use super::*;
    const KEY: &str = "s-20261116-0a0b0c02";
    fn fixture() -> (tempfile::TempDir, Policy, DateTime<Utc>) {
        let dir = tempfile::tempdir().unwrap();
        start_turn(dir.path(), KEY, "codex", "/fixture", "run", "hello").unwrap();
        let activity = "2030-01-01T12:00:00.000Z";
        let policy = Policy {
            kind: "idle".into(),
            activity: activity.into(),
            revision: 1,
            pr: None,
        };
        with_write(dir.path(), "fixture", |tx| {
            tx.execute("UPDATE sessions SET last_activity_at=?2 WHERE key=?1", params![KEY, activity]).unwrap();
            tx.execute("UPDATE turns SET status='success', ended_at=?2 WHERE session_key=?1", params![KEY, activity]).unwrap();
            tx.execute("INSERT INTO session_organization(session_key,activity_at,revision) VALUES(?1,?2,1)", params![KEY, activity]).unwrap();
            tx.execute("INSERT INTO session_settlement(session_key,policy) VALUES(?1,?2)", params![KEY, serde_json::to_string(&policy).unwrap()]).unwrap(); Ok(())
        }).unwrap();
        (
            dir,
            policy,
            DateTime::parse_from_rfc3339("2030-01-08T12:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
        )
    }
    #[test]
    fn idle_policy_is_one_shot_and_undo_stays_awake() {
        let (dir, policy, now) = fixture();
        assert!(
            settle(dir.path(), KEY, &policy, now - Duration::seconds(1), false)
                .unwrap()
                .is_none()
        );
        let result = settle(dir.path(), KEY, &policy, now, false)
            .unwrap()
            .unwrap();
        assert!(result.settled);
        assert_eq!(
            result.reason.as_deref(),
            Some("Settled after seven idle days")
        );
        change_at(
            dir.path(),
            &Change {
                key: KEY.into(),
                action: "wake".into(),
                until: None,
                expected_revision: result.revision,
                expected_activity: policy.activity.clone(),
            },
            now,
        )
        .unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, false)
            .unwrap()
            .is_none());
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
    }
    #[test]
    fn new_activity_revision_pins_failures_and_queue_block_automation() {
        for sql in ["UPDATE sessions SET last_activity_at='2030-01-02T12:00:00Z'", "UPDATE session_organization SET revision=2", "UPDATE session_organization SET pinned=1", "UPDATE turns SET status='error'", "UPDATE turns SET ended_at=NULL", "INSERT INTO run_queue(source,source_id,label,status,priority,requested_at) VALUES('chat','s-20261116-0a0b0c02','Work','queued',0,'2030-01-01')", "INSERT INTO interactions(id,session_key,turn_ord,run_id,agent,tool_use_id,source,mode,request_json,status,created_at) VALUES('card','s-20261116-0a0b0c02',1,'run','codex','question','test','default','{}','pending','2030-01-01')"] {
            let (dir, policy, now) = fixture();
            with_write(dir.path(), "block", |tx| { tx.execute(sql, []).unwrap(); Ok(()) }).unwrap();
            assert!(settle(dir.path(), KEY, &policy, now, false).unwrap().is_none(), "{sql}");
            assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
        }
    }

    #[test]
    fn bound_pr_requires_merge_proof_and_the_same_conversation_folder() {
        let (dir, mut policy, now) = fixture();
        let tree = dir.path().to_string_lossy().into_owned();
        policy.kind = "merged".into();
        policy.pr = Some(crate::build_changes::SettlementPr {
            workspace_id: "build".into(),
            repo: tree.clone(),
            worktree: tree.clone(),
            remote: "git@example/repo".into(),
            head: "a".repeat(40),
            branch: "feature".into(),
            url: "https://example/repo/pull/1".into(),
            number: 1,
            state: "open".into(),
        });
        with_write(dir.path(), "bind", |tx| {
            tx.execute("UPDATE sessions SET cwd=?1", [&tree]).unwrap();
            tx.execute(
                "UPDATE session_settlement SET policy=?1",
                [serde_json::to_string(&policy).unwrap()],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, false)
            .unwrap()
            .is_none());
        with_write(dir.path(), "move", |tx| {
            tx.execute("UPDATE sessions SET cwd='/different'", [])
                .unwrap();
            Ok(())
        })
        .unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, true)
            .unwrap()
            .is_none());
        with_write(dir.path(), "restore folder", |tx| {
            tx.execute("UPDATE sessions SET cwd=?1", [&tree]).unwrap();
            Ok(())
        })
        .unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, true)
            .unwrap()
            .unwrap()
            .reason
            .unwrap()
            .contains("PR #1 merged"));
    }

    #[test]
    fn paused_chat_is_never_settled() {
        let (dir, policy, now) = fixture();
        with_write(dir.path(), "pause", |tx| {
            tx.execute("INSERT INTO chat_pauses(session_key,native_session_id,agent,paused_at,note,reason) VALUES(?1,'native','codex',0,'Paused','user')", [KEY]).unwrap(); Ok(())
        }).unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, false)
            .unwrap()
            .is_none());
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
    }

    #[test]
    fn a_finished_parent_keeps_its_unfinished_child_visible() {
        let (dir, policy, now) = fixture();
        let child = "s-20261116-0a0b0c03";
        start_turn(dir.path(), child, "codex", "/fixture", "child", "work").unwrap();
        with_write(dir.path(), "child", |tx| {
            tx.execute(
                "UPDATE sessions SET parent_key=?1 WHERE key=?2",
                params![KEY, child],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        assert!(settle(dir.path(), KEY, &policy, now, false)
            .unwrap()
            .is_none());
        assert_eq!(is_archived(dir.path(), KEY).unwrap(), Some(false));
    }

    #[tokio::test]
    async fn configuring_after_new_activity_keeps_the_settlement_reason_visible() {
        let (dir, policy, now) = fixture();
        with_write(dir.path(), "older metadata", |tx| {
            tx.execute(
                "UPDATE session_organization SET activity_at='2029-12-01T00:00:00Z'",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();
        let configured = configure(
            dir.path(),
            &Change {
                key: KEY.into(),
                action: "idle".into(),
                until: None,
                expected_revision: 1,
                expected_activity: policy.activity,
            },
        )
        .await
        .unwrap();
        let saved = configured.auto_settle.unwrap();
        let settled = settle(dir.path(), KEY, &saved, now, false)
            .unwrap()
            .unwrap();
        assert!(settled.settled);
        assert_eq!(
            settled.reason.as_deref(),
            Some("Settled after seven idle days")
        );
    }
}
