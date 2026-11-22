//! Real HTTP and queue coverage for per-send settings, with synthetic agents.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_and_attachments_survive_existing_and_new_chat_sends() {
    for existing in [false, true] {
        let rig = rig().await;
        let token = rig.token();
        let cwd = crate::workspaces::ensure_quick_chats(&rig.vault).unwrap();
        let key = "s-20261107-cccc9999";
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &cwd.to_string_lossy(),
            &[("Hi", "Hello")],
            "acceptEdits",
        );
        let upload = rig
            .call_raw(
                "POST",
                "/v1/workspaces/quick-chats/attachments?name=note.txt",
                Some(&token),
                b"A note",
            )
            .await;
        assert_eq!(upload.status, 201);
        let id = upload.json()["attachment"]["id"].clone();
        let request = json!({
            "workspaceId": "quick-chats", "text": "", "clientMessageId": "controls-with-file",
            "attachmentIds": [id],
            "controls": {"agent": "codex", "model": "test-model", "permissionMode": "plan", "effort": "high"}
        });
        let path = if existing {
            format!("/v1/sessions/{key}/messages")
        } else {
            "/v1/sessions".into()
        };
        let response = rig
            .call("POST", &path, Some(&token), Some(request.clone()))
            .await;
        assert_eq!(response.status, 202, "{}", response.json());
        let sent = rig.host.sent.lock().unwrap()[0].clone();
        assert_eq!(sent.agent.as_deref(), Some("codex"));
        assert_eq!(sent.model.as_deref(), Some("test-model"));
        assert_eq!(sent.permission_mode.as_deref(), Some("plan"));
        assert_eq!(sent.session_id, None);
        assert!(sent.prompt.contains("<attached-files count=\"1\">"));
        let saved = std::fs::read_dir(cwd.join(".hollow-attachments"))
            .unwrap()
            .flatten()
            .find(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .unwrap()
            .path()
            .canonicalize()
            .unwrap();
        assert!(
            sent.prompt.contains(saved.to_string_lossy().as_ref()),
            "{}",
            sent.prompt
        );
        let queued =
            crate::run_queue::get(&rig.vault, response.json()["queueId"].as_i64().unwrap())
                .unwrap()
                .unwrap();
        assert_eq!(queued.payload["effort"], "high");
        let retry = rig.call("POST", &path, Some(&token), Some(request)).await;
        assert_eq!(retry.json(), response.json());
        assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_precheck_preserves_sends_without_a_workspace_brief() {
    let rig = rig().await;
    let token = rig.token();
    let cwd = rig.vault.join("workspaces/legacy");
    std::fs::create_dir_all(&cwd).unwrap();
    assert!(crate::workspaces::workspace_for_cwd(&rig.vault, &cwd).is_err());
    let key = "s-20261106-cccc8888";
    seed_chat(
        &rig.vault,
        key,
        "codex",
        &cwd.to_string_lossy(),
        &[("Hi", "Hello")],
        "plan",
    );
    let response = rig
        .call(
            "POST",
            &format!("/v1/sessions/{key}/messages"),
            Some(&token),
            Some(json!({"text": "Continue", "clientMessageId": "legacy-workspace"})),
        )
        .await;
    assert_eq!(response.status, 202, "{}", response.json());
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_inherit_the_started_turn_but_keep_the_macs_saved_permission_choice() {
    for saved in [None, Some("bypassPermissions")] {
        let rig = rig().await;
        let token = rig.token();
        let key = "s-20261106-cccc7777";
        let cwd = rig.vault.to_string_lossy();
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &cwd,
            &[("Hi", "Hello")],
            "bypassPermissions",
        );
        if let Some(mode) = saved {
            rig.store
                .set_chat_modes(&[(key.into(), mode.into())], true, false)
                .unwrap();
        }
        let path = format!("/v1/sessions/{key}/messages");
        let first = rig
            .call(
                "POST",
                &path,
                Some(&token),
                Some(json!({
                    "text": "Run it", "clientMessageId": "inherit-first",
                    "controls": {"agent": "codex", "permissionMode": "acceptEdits", "effort": "high"}
                })),
            )
            .await;
        assert_eq!(first.status, 202, "{}", first.json());
        let input = rig.host.sent.lock().unwrap()[0].clone();
        // Exercise the session writes that the pump makes when this accepted turn starts.
        let ord = session_store::start_turn(
            &rig.vault,
            key,
            input.agent.as_deref().unwrap(),
            &cwd,
            "inherit-run",
            "Run it",
        )
        .unwrap();
        session_store::record_turn_permission(
            &rig.vault,
            key,
            ord,
            input.permission_mode.as_deref().unwrap(),
        )
        .unwrap();
        session_store::finish_turn(&rig.vault, key, ord, "success").unwrap();
        crate::run_queue::cancel(
            &rig.vault,
            first.json()["queueId"].as_i64().unwrap(),
            Utc::now(),
        )
        .unwrap();
        let next = rig
            .call(
                "POST",
                &path,
                Some(&token),
                Some(json!({
                    "text": "Continue", "clientMessageId": "inherit-next"
                })),
            )
            .await;
        assert_eq!(next.status, 202, "{}", next.json());
        let input = rig.host.sent.lock().unwrap()[1].clone();
        assert_eq!(input.agent.as_deref(), Some("codex"));
        assert_eq!(
            input.permission_mode.as_deref(),
            Some(saved.unwrap_or("acceptEdits"))
        );
        let row = crate::run_queue::get(&rig.vault, next.json()["queueId"].as_i64().unwrap())
            .unwrap()
            .unwrap();
        assert!(row.payload.get("effort").is_none_or(Value::is_null));
        assert_eq!(rig.store.chat_mode(key).unwrap().as_deref(), saved);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_reset_native_resume_only_for_an_agent_model_or_permission_class_change() {
    for (index, (controls, resume)) in [
        (json!({}), true),
        (json!({"agent": "claude"}), true),
        (json!({"permissionMode": "acceptEdits"}), true),
        (json!({"permissionMode": "plan"}), false),
        (json!({"model": "other-model"}), false),
        (json!({"agent": "codex"}), false),
    ]
    .into_iter()
    .enumerate()
    {
        let rig = rig().await;
        let token = rig.token();
        let key = "s-20261106-cccc8888";
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &rig.vault.to_string_lossy(),
            &[("Hi", "Hello")],
            "bypassPermissions",
        );
        session_store::record_harness_id(&rig.vault, key, "native-controls").unwrap();
        let response = rig
            .call(
                "POST",
                &format!("/v1/sessions/{key}/messages"),
                Some(&token),
                Some(json!({
                    "text": "Hi", "clientMessageId": format!("resume-{index}"), "controls": controls
                })),
            )
            .await;
        assert_eq!(response.status, 202, "{}", response.json());
        assert_eq!(
            rig.host.sent.lock().unwrap()[0].session_id.as_deref(),
            resume.then_some("native-controls")
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_catalog_requires_pairing_and_matches_the_contract() {
    let rig = rig().await;
    assert_eq!(
        rig.call("GET", "/v1/chat-controls", None, None)
            .await
            .status,
        401
    );
    let token = rig.token();
    let response = rig
        .call("GET", "/v1/chat-controls?agent=codex", Some(&token), None)
        .await;
    assert_eq!(response.status, 200);
    assert_eq!(response.json(), fixture("chat-controls/options.json"));
    assert_eq!(
        rig.call("GET", "/v1/chat-controls?agent=unknown", Some(&token), None)
            .await
            .status,
        400
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_switch_agent_without_resuming_and_keep_effort_in_the_queue() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261106-cccc5555";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &rig.vault.to_string_lossy(),
        &[("Hi", "Hello")],
        "bypassPermissions",
    );
    let body = fixture("chat-controls/send-request.json");
    let route = format!("/v1/sessions/{key}/messages");
    let response = rig
        .call("POST", &route, Some(&token), Some(body.clone()))
        .await;
    assert_eq!(response.status, 202, "{}", response.json());
    let input = rig.host.sent.lock().unwrap()[0].clone();
    assert_eq!(input.agent.as_deref(), Some("codex"));
    assert_eq!(input.model.as_deref(), Some("test-model"));
    assert_eq!(input.permission_mode.as_deref(), Some("plan"));
    assert_eq!(input.session_id, None);
    assert_eq!(input.session_key.as_deref(), Some(key));
    let row = crate::run_queue::get(&rig.vault, response.json()["queueId"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    let payload = row.payload;
    assert_eq!(payload["effort"], "high");
    assert_eq!(
        rig.store.chat_mode(key).unwrap(),
        None,
        "no Mac picker preference changed"
    );
    let retry = rig.call("POST", &route, Some(&token), Some(body)).await;
    assert_eq!(retry.json(), response.json());
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_refuse_bad_choices_before_queueing_and_support_new_chats() {
    let rig = rig().await;
    let token = rig.token();
    for controls in [
        json!({"agent": "unknown"}),
        json!({"effort": "ultracode"}),
        json!({"permissionMode": "default"}),
        json!({"model": "two words"}),
        json!({"model": "--help"}),
        json!({"model": "model\u{202e}"}),
    ] {
        let request = json!({"workspaceId": "quick-chats", "text": "Hello", "clientMessageId": "invalid-controls", "controls": controls});
        let response = rig
            .call("POST", "/v1/sessions", Some(&token), Some(request))
            .await;
        assert_eq!(response.status, 400, "{}", response.json());
        assert!(rig.host.sent.lock().unwrap().is_empty());
    }
    let request = json!({"workspaceId": "quick-chats", "text": "Hello", "clientMessageId": "new-controls",
        "controls": {"agent": "codex", "model": "", "effort": "low", "permissionMode": "plan"}});
    let response = rig
        .call("POST", "/v1/sessions", Some(&token), Some(request))
        .await;
    assert_eq!(response.status, 202, "{}", response.json());
    let input = rig.host.sent.lock().unwrap()[0].clone();
    assert_eq!(input.agent.as_deref(), Some("codex"));
    assert_eq!(input.model.as_deref(), Some(""));
    assert_eq!(input.permission_mode.as_deref(), Some("plan"));
}

/// L6 (11-10): the COO (`main`) takes any installed agent and its models,
/// for a new chat and for an existing conversation, like every workspace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coo_controls_take_another_provider_and_its_models() {
    let rig = rig().await;
    let token = rig.token();
    let response = rig
        .call(
            "POST",
            "/v1/sessions",
            Some(&token),
            Some(json!({
                "workspaceId": "main", "text": "Hi", "agent": "codex",
                "clientMessageId": "main-foreign-model", "controls": {"model": "codex-model"}
            })),
        )
        .await;
    assert_eq!(response.status, 202, "{}", response.json());
    let input = rig.host.sent.lock().unwrap()[0].clone();
    assert_eq!(input.agent.as_deref(), Some("codex"));
    assert_eq!(input.model.as_deref(), Some("codex-model"));
    let cwd = crate::workspaces::session_dir_for(&rig.vault, "main").unwrap();
    let key = "s-20261106-cccc6666";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &cwd.to_string_lossy(),
        &[("Hi", "Hello")],
        "plan",
    );
    let response = rig
        .call(
            "POST",
            &format!("/v1/sessions/{key}/messages"),
            Some(&token),
            Some(json!({
                "text": "Hi", "clientMessageId": "main-agent-switch", "controls": {"agent": "codex"}
            })),
        )
        .await;
    assert_eq!(response.status, 202, "{}", response.json());
    let sent = rig.host.sent.lock().unwrap();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].agent.as_deref(), Some("codex"));
}

/// Reliability R2 (2026-11-13): a phone send that names no effort queues
/// none of its own, so the chat's own level (kept in the session store since
/// #1358) is what the turn runs at; a send that names one wins over it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_send_without_an_effort_runs_at_the_chats_own_level() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261113-effr0001";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &rig.vault.to_string_lossy(),
        &[("Hi", "Hello")],
        "acceptEdits",
    );
    session_store::set_chat_effort(&rig.vault, key, Some("max")).unwrap();
    let route = format!("/v1/sessions/{key}/messages");
    let plain = rig
        .call(
            "POST",
            &route,
            Some(&token),
            Some(json!({"text": "Continue", "clientMessageId": "own-effort-plain"})),
        )
        .await;
    assert_eq!(plain.status, 202, "{}", plain.json());
    let queue_id = plain.json()["queueId"].as_i64().unwrap();
    let row = crate::run_queue::get(&rig.vault, queue_id)
        .unwrap()
        .unwrap();
    let named = row
        .payload
        .get("effort")
        .and_then(Value::as_str)
        .map(String::from);
    assert_eq!(named, None, "the phone names none");
    assert_eq!(
        crate::chat::resolve_requested_effort(&rig.vault, key, named).as_deref(),
        Some("max"),
        "the turn runs at the chat's own level"
    );
    crate::run_queue::cancel(&rig.vault, queue_id, Utc::now()).unwrap();

    let chosen = rig
        .call(
            "POST",
            &route,
            Some(&token),
            Some(json!({
                "text": "Continue", "clientMessageId": "own-effort-named",
                "controls": {"effort": "low"}
            })),
        )
        .await;
    assert_eq!(chosen.status, 202, "{}", chosen.json());
    let row = crate::run_queue::get(&rig.vault, chosen.json()["queueId"].as_i64().unwrap())
        .unwrap()
        .unwrap();
    let named = row
        .payload
        .get("effort")
        .and_then(Value::as_str)
        .map(String::from);
    assert_eq!(named.as_deref(), Some("low"));
    assert_eq!(
        crate::chat::resolve_requested_effort(&rig.vault, key, named).as_deref(),
        Some("low"),
        "a level the phone names wins"
    );
    assert_eq!(
        session_store::chat_effort(&rig.vault, key)
            .unwrap()
            .as_deref(),
        Some("max"),
        "a phone's level is that turn's only, never the chat's own"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controls_refuse_permission_escalation_on_existing_and_new_chats() {
    let rig = rig().await;
    let token = rig.token();
    let cwd = rig.vault.to_string_lossy().to_string();

    // Existing chat in acceptEdits cannot escalate to auto or bypassPermissions.
    let key = "s-20261120-esca0001";
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &cwd,
        &[("Hi", "Hello")],
        "acceptEdits",
    );
    let route = format!("/v1/sessions/{key}/messages");
    for (idx, mode) in ["bypassPermissions", "auto"].into_iter().enumerate() {
        let resp = rig
            .call(
                "POST",
                &route,
                Some(&token),
                Some(json!({
                    "text": "Escalate",
                    "clientMessageId": format!("esc-existing-{idx}"),
                    "controls": {"permissionMode": mode}
                })),
            )
            .await;
        assert_eq!(resp.status, 403, "{mode}: {}", resp.json());
        assert_eq!(resp.json()["error"]["code"], "forbidden");
    }
    assert!(rig.host.sent.lock().unwrap().is_empty());

    // Matching or lowering an existing chat's permission mode is still allowed.
    let same = rig
        .call(
            "POST",
            &route,
            Some(&token),
            Some(json!({
                "text": "Same mode",
                "clientMessageId": "same-existing",
                "controls": {"permissionMode": "acceptEdits"}
            })),
        )
        .await;
    assert_eq!(same.status, 202, "{}", same.json());
    assert_eq!(
        rig.host.sent.lock().unwrap()[0].permission_mode.as_deref(),
        Some("acceptEdits")
    );
    crate::run_queue::cancel(
        &rig.vault,
        same.json()["queueId"].as_i64().unwrap(),
        Utc::now(),
    )
    .unwrap();

    // New chat defaults to acceptEdits (or window/headless mode): cannot escalate
    // into bypassPermissions or auto.
    for (idx, mode) in ["bypassPermissions", "auto"].into_iter().enumerate() {
        let resp = rig
            .call(
                "POST",
                "/v1/sessions",
                Some(&token),
                Some(json!({
                    "workspaceId": "quick-chats",
                    "text": "Escalate new",
                    "clientMessageId": format!("esc-new-{idx}"),
                    "controls": {"permissionMode": mode}
                })),
            )
            .await;
        assert_eq!(resp.status, 403, "{mode}: {}", resp.json());
        assert_eq!(resp.json()["error"]["code"], "forbidden");
    }

    // Matching acceptEdits or lowering to plan on a new chat still succeeds.
    let lower = rig
        .call(
            "POST",
            "/v1/sessions",
            Some(&token),
            Some(json!({
                "workspaceId": "quick-chats",
                "text": "Lower new",
                "clientMessageId": "lower-new",
                "controls": {"permissionMode": "plan"}
            })),
        )
        .await;
    assert_eq!(lower.status, 202, "{}", lower.json());
    assert_eq!(
        rig.host.sent.lock().unwrap()[1].permission_mode.as_deref(),
        Some("plan")
    );
}
