//! R1: a phone revoked while one of its requests is in flight commits nothing.
//! Each test revokes the phone in the gap a wait leaves (a real wait where the
//! rig has one: the held-send order; else at the commit point itself, through
//! `routes::CHECKPOINTS`, which runs just before the one check) and asserts the
//! 401 and that nothing landed.
use super::*;

/// Revoke `device_id` the next time its request reaches `point`.
fn revoke_at(rig: &Rig, device_id: &str, point: &'static str) {
    let (store, id) = (rig.store.clone(), device_id.to_string());
    routes::CHECKPOINTS.lock().unwrap().insert(
        (device_id.to_string(), point),
        Box::new(move || {
            assert!(store.revoke(&id).unwrap());
        }),
    );
}

fn paired(rig: &Rig) -> (store::Device, String) {
    rig.store.pair("Test iPhone", None, Utc::now()).unwrap()
}

fn unauthorized(resp: &Resp, what: &str) {
    assert_eq!(
        resp.status,
        401,
        "{what}: {}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(resp.json()["error"]["code"], "unauthorized", "{what}");
}

fn quick_chat(rig: &Rig, key: &str) -> PathBuf {
    let quick = crate::workspaces::ensure_quick_chats(&rig.vault).unwrap();
    let cwd = quick.to_string_lossy().into_owned();
    seed_chat(&rig.vault, key, "codex", &cwd, &[("Hi", "Hello")], "auto");
    quick
}

/// The held-send order: a send (and a card's answer, which sends) waits for
/// it, the phone is revoked during the wait, and nothing is queued.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_while_a_send_waits_for_the_held_order_nothing_is_queued() {
    let rig = Arc::new(rig().await);
    quick_chat(&rig, "s-20261114-0000b001");
    let key = "s-20261114-0000b001";
    let (device, token) = paired(&rig);
    let order = rig.host.order.lock().await;
    let call = {
        let (rig, token) = (rig.clone(), token.clone());
        tokio::spawn(async move {
            rig.call(
                "POST",
                &format!("/v1/sessions/{key}/messages"),
                Some(&token),
                Some(fixture("send-request.json")),
            )
            .await
        })
    };
    eventually("the send waits for the order", || {
        rig.host.order_waiting.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(rig.store.revoke(&device.device_id).unwrap());
    drop(order);
    unauthorized(&call.await.unwrap(), "send");
    assert!(rig.host.sent.lock().unwrap().is_empty());

    // The same wait for a card's answer: the card comes back open.
    let vault = rig.vault.to_string_lossy().into_owned();
    let ask = "s-20261114-0000b002";
    seed_chat(
        &rig.vault,
        ask,
        "claude",
        &vault,
        &[("Start", "Before:")],
        "acceptEdits",
    );
    let card = seed_card(&rig.vault, ask, 1, "toolu-r1", &three_questions());
    let (device, token) = paired(&rig);
    let order = rig.host.order.lock().await;
    let call = {
        let (rig, token) = (rig.clone(), token.clone());
        let path = format!("/v1/sessions/{ask}/questions/{}/answer", card.id);
        tokio::spawn(async move {
            rig.call(
                "POST",
                &path,
                Some(&token),
                Some(fixture("answer-request.json")),
            )
            .await
        })
    };
    eventually("the answer waits for the order", || {
        rig.host.order_waiting.load(Ordering::SeqCst) == 2
    })
    .await;
    assert!(rig.store.revoke(&device.device_id).unwrap());
    drop(order);
    unauthorized(&call.await.unwrap(), "answer at the order");
    assert!(rig.host.sent.lock().unwrap().is_empty());
    let cards = crate::interactions::list_for_session(&rig.vault, ask).unwrap();
    assert!(
        cards.iter().all(|c| c.status == "pending"),
        "the card stayed answered"
    );
}

/// The real order in `commands`: a gate that fails once the order is won
/// refuses, and the order is free again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_real_held_order_runs_the_gate_after_the_wait() {
    let rig = rig().await;
    let (device, _) = paired(&rig);
    let store = rig.store.clone();
    let order = crate::commands::held_order_for_test().lock().await;
    let gate = routes::gate_for(&store, &device.device_id, "send-held-order");
    let waiting =
        tokio::spawn(async move { crate::commands::held_order(|| gate()).await.map(|_| ()) });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(store.revoke(&device.device_id).unwrap());
    drop(order);
    assert_eq!(waiting.await.unwrap(), Err(store::REVOKED_TEXT.to_string()));
    assert!(crate::commands::held_order_for_test().try_lock().is_ok());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_after_stop_validated_the_chat_nothing_is_stopped() {
    let rig = rig().await;
    let key = "s-20261114-0000b003";
    quick_chat(&rig, key);
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "stop");
    let resp = rig
        .call(
            "POST",
            &format!("/v1/sessions/{key}/stop"),
            Some(&token),
            None,
        )
        .await;
    unauthorized(&resp, "stop");
    assert!(rig.host.stopped.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_in_the_task_writers_lock_wait_no_task_is_written() {
    let rig = rig().await;
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "task-write");
    let resp = rig
        .call(
            "POST",
            "/v1/tasks",
            Some(&token),
            Some(json!({"idempotencyKey": "revoke-task-0001", "title": "Nope"})),
        )
        .await;
    unauthorized(&resp, "task create");
    assert!(
        crate::vault_tasks::list(&rig.vault).is_empty(),
        "a task was written"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_in_the_note_writers_lease_wait_no_note_is_written() {
    let rig = rig().await;
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "note-write");
    let resp = rig
        .call(
            "POST",
            "/v1/notes",
            Some(&token),
            Some(json!({"idempotencyKey": "revoke-note-0001", "text": "Nope"})),
        )
        .await;
    unauthorized(&resp, "note");
    assert!(
        !rig.vault.join("notes").exists() || notes_api_tests::note_files(&rig.vault).is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_before_a_new_chat_is_placed_the_workspace_stays_where_it_was() {
    let rig = rig().await;
    let workspace = crate::workspaces::create(&rig.vault, "Mini work", "Goal").unwrap();
    *rig.host.runner.lock().unwrap() = Some(wire::Runner {
        name: "build-mac".into(),
        available: true,
    });
    *rig.host.place.lock().unwrap() = WITH_RUNNER;
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "placement");
    let resp = rig
        .call(
            "POST",
            "/v1/sessions",
            Some(&token),
            Some(
                json!({"workspaceId": workspace.id, "text": "Go", "runsOn": "mac-mini",
                "clientMessageId": "00000001-3333-4333-8444-555555555555"}),
            ),
        )
        .await;
    unauthorized(&resp, "placement");
    assert_eq!(
        crate::remote_folder::runs_on_for(&rig.vault, &workspace.id),
        None
    );
    assert!(rig.host.sent.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_before_a_card_is_resolved_it_stays_open_and_nothing_is_sent() {
    let rig = rig().await;
    let vault = rig.vault.to_string_lossy().into_owned();
    let key = "s-20261114-0000b004";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &vault,
        &[("Start", "Before:")],
        "acceptEdits",
    );
    let card = seed_card(&rig.vault, key, 1, "toolu-r2", &three_questions());
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "answer");
    let path = format!("/v1/sessions/{key}/questions/{}/answer", card.id);
    let resp = rig
        .call(
            "POST",
            &path,
            Some(&token),
            Some(fixture("answer-request.json")),
        )
        .await;
    unauthorized(&resp, "answer");
    assert!(rig.host.sent.lock().unwrap().is_empty());
    let cards = crate::interactions::list_for_session(&rig.vault, key).unwrap();
    assert!(cards.iter().all(|c| c.status == "pending"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_before_an_upload_is_saved_no_file_is_written() {
    let rig = rig().await;
    let key = "s-20261114-0000b005";
    let quick = quick_chat(&rig, key);
    let (device, token) = paired(&rig);
    revoke_at(&rig, &device.device_id, "upload-save");
    let resp = rig
        .call_raw(
            "POST",
            &format!("/v1/sessions/{key}/attachments?name=a.txt"),
            Some(&token),
            b"hello",
        )
        .await;
    unauthorized(&resp, "upload");
    assert!(!quick.join(".hollow-attachments").exists());
}

/// The low findings: after a revoke, DELETE push and the Live Activity routes
/// are refused instead of answering 204, and a registration leaves nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_phones_push_and_live_activity_calls_are_refused_and_leave_nothing() {
    let rig = rig().await;
    let key = "s-20261114-0000b006";
    quick_chat(&rig, key);
    let (device, _) = paired(&rig);
    // The request passed the token check; the revoke lands after it.
    assert!(rig.store.revoke(&device.device_id).unwrap());
    let call = |method: &'static str, path: &'static str, body: Value| {
        let (ctx, device) = (rig.ctx.clone(), device.clone());
        async move {
            let body = hyper::body::Bytes::from(body.to_string());
            match routes::dispatch(&ctx, Some(&device), method, path, "", body).await {
                routes::Reply::Json { status, .. } => status,
                routes::Reply::Empty(status) => status,
                _ => 0,
            }
        }
    };
    assert_eq!(call("DELETE", "/v1/push", json!({})).await, 401);
    assert_eq!(call("DELETE", "/v1/live-activity", json!({})).await, 401);
    let register =
        json!({"sessionKey": key, "token": "ab".repeat(32), "environment": "production"});
    assert_eq!(call("POST", "/v1/live-activity", register).await, 401);
    assert_eq!(
        rig.ctx.activities.len(),
        0,
        "a revoked phone registered an activity"
    );
}

/// A revoked id is never live again: its token no longer names a phone, the
/// commit check refuses it, and a pairing never hands the id out.
#[test]
fn a_revoked_id_is_never_live_again() {
    let dir = store_dir();
    let store = Store::at(dir.path());
    let (device, token) = store.pair("One", None, Utc::now()).unwrap();
    assert_eq!(store.ensure_live(&device.device_id), Ok(()));
    assert!(store.revoke(&device.device_id).unwrap());
    assert_eq!(
        store.ensure_live(&device.device_id),
        Err(store::NotLive::Revoked)
    );
    assert!(store.device_for_token(&token).unwrap().is_none());
}

/// Fail closed: a revoke whose file write fails leaves the phone in the file,
/// yet only the in-memory revoked set refuses it (token and commit check).
/// Retrying the revoke once the write works removes it from the file.
#[cfg(unix)]
#[test]
fn a_revoke_whose_write_fails_still_refuses_the_phone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = store_dir();
    let store = Store::at(dir.path());
    let (device, token) = store.pair("One", None, Utc::now()).unwrap();
    let id = device.device_id.clone();
    let lock = |mode| std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(mode));
    lock(0o555).unwrap();
    let writable = std::fs::write(dir.path().join("probe"), b"x").is_ok();
    let failed = store.revoke(&id).is_err();
    lock(0o755).unwrap();
    if writable {
        // Running as a user the mode does not bind (root): nothing to test.
        return;
    }
    assert!(failed, "the write was expected to fail");
    assert!(store
        .read()
        .unwrap()
        .devices
        .iter()
        .any(|d| d.device_id == id));
    assert_eq!(store.ensure_live(&id), Err(store::NotLive::Revoked));
    assert!(store.device_for_token(&token).unwrap().is_none());
    // The id is never handed out again either.
    assert!(store.revoke(&id).unwrap());
}

/// A new chat with no effort set (the path that skipped the held-send order)
/// waits for the order, the phone is revoked during the wait, nothing queues.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_while_a_new_chat_without_effort_waits_for_the_order_nothing_is_queued() {
    let rig = Arc::new(rig().await);
    let workspace = crate::workspaces::create(&rig.vault, "Mini work", "Goal").unwrap();
    let (device, token) = paired(&rig);
    let order = rig.host.order.lock().await;
    let call = {
        let (rig, token, id) = (rig.clone(), token.clone(), workspace.id.clone());
        tokio::spawn(async move {
            rig.call(
                "POST",
                "/v1/sessions",
                Some(&token),
                Some(json!({"workspaceId": id, "text": "Go",
                    "clientMessageId": "00000002-3333-4333-8444-555555555555"})),
            )
            .await
        })
    };
    eventually("the new chat waits for the order", || {
        rig.host.order_waiting.load(Ordering::SeqCst) == 1
    })
    .await;
    assert!(rig.store.revoke(&device.device_id).unwrap());
    drop(order);
    unauthorized(&call.await.unwrap(), "new chat");
    assert!(rig.host.sent.lock().unwrap().is_empty());
}

/// R2 (ENG-223): the phone's `runsOn` lands before the send so the chat's
/// first turn is placed by it, but a send that is then refused (here: the
/// phone is revoked at the held-send order, after the placement) puts the
/// workspace's Runs on back. With `meanwhile` set, the person changes the
/// setting in that gap too, and the undo must leave their choice alone.
/// Returns the setting as the gap saw it and as the request left it.
async fn failed_new_chat_runs_on(
    key: &str,
    meanwhile: Option<&'static str>,
) -> (Option<String>, Option<String>) {
    let rig = rig().await;
    let workspace = crate::workspaces::create(&rig.vault, "Mini work two", "Goal").unwrap();
    *rig.host.runner.lock().unwrap() = Some(wire::Runner {
        name: "build-mac".into(),
        available: true,
    });
    *rig.host.place.lock().unwrap() = WITH_RUNNER;
    assert_eq!(
        crate::remote_folder::runs_on_for(&rig.vault, &workspace.id),
        None
    );
    let (device, token) = paired(&rig);
    let seen = std::sync::Arc::new(Mutex::new(None));
    let (store, id, vault, ws, mid) = (
        rig.store.clone(),
        device.device_id.clone(),
        rig.vault.clone(),
        workspace.id.clone(),
        seen.clone(),
    );
    routes::CHECKPOINTS.lock().unwrap().insert(
        (device.device_id.clone(), "new-chat-held-order"),
        Box::new(move || {
            // What the placement left, before anything undoes it.
            *mid.lock().unwrap() = Some(crate::remote_folder::runs_on_for(&vault, &ws));
            if let Some(value) = meanwhile {
                crate::remote_folder::set_runs_on(&vault, &ws, Some(value)).unwrap();
            }
            assert!(store.revoke(&id).unwrap());
        }),
    );
    let resp = rig
        .call(
            "POST",
            "/v1/sessions",
            Some(&token),
            Some(
                json!({"workspaceId": workspace.id, "text": "Go", "runsOn": "mac-mini",
                "clientMessageId": key}),
            ),
        )
        .await;
    unauthorized(&resp, "new-chat-held-order");
    assert!(rig.host.sent.lock().unwrap().is_empty());
    let mid = seen.lock().unwrap().take().expect("the gap ran");
    (
        mid,
        crate::remote_folder::runs_on_for(&rig.vault, &workspace.id),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_chat_whose_send_fails_leaves_the_workspaces_runs_on_as_it_was() {
    let (mid, after) = failed_new_chat_runs_on("00000002-3333-4333-8444-555555555555", None).await;
    assert_eq!(
        mid.as_deref(),
        Some("runner"),
        "the placement must have landed before the send failed"
    );
    assert_eq!(
        after, None,
        "the refused chat left the phone's runsOn behind"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_new_chats_undo_never_overwrites_a_choice_made_meanwhile() {
    let (mid, after) =
        failed_new_chat_runs_on("00000003-3333-4333-8444-555555555555", Some("this-mac")).await;
    assert_eq!(mid.as_deref(), Some("runner"));
    assert_eq!(
        after.as_deref(),
        Some("this-mac"),
        "the person's change was undone"
    );
}
