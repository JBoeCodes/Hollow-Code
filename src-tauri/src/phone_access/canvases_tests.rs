//! v1.2 canvases (`docs/phone-api/canvases.md`): the kit, the frame body,
//! the answer text and the routes, against `docs/phone-api-fixtures/canvases/`.

use super::super::canvases::{self, THEME_TOKENS};
use super::*;

const SANDBOX_TS: &str = include_str!("../../../src/components/htmlSandbox.ts");
const CSP_GOLDEN: &str = include_str!("../../../src/components/artifactCsp.golden.json");

#[test]
fn the_kit_is_the_macs_own_frame_parts() {
    let kit = canvases::kit();
    assert_eq!(kit.kit, "kit@1");
    assert_eq!(kit.sha256.len(), 64);
    assert!(kit.sha256.bytes().all(|b| b.is_ascii_hexdigit()));
    // The Canvas mode's own policy, as the Mac serves it.
    let golden: Value = serde_json::from_str(CSP_GOLDEN).unwrap();
    let policy = golden["canvas"].as_str().unwrap();
    assert!(kit.head.contains(&format!(
        "<meta http-equiv=\"Content-Security-Policy\" content=\"{policy}\">"
    )));
    // Runtime first, then the kit, byte for byte; the ask kit apart.
    let runtime = include_str!("../../../src/components/sandbox-doc.js");
    let kit_js = include_str!("../../../src/canvas/kit/generated/kit.min.js");
    let at_runtime = kit.head.find(runtime).expect("runtime");
    let at_kit = kit.head.find(kit_js).expect("kit");
    assert!(at_runtime < at_kit);
    assert!(kit.head.contains(include_str!(
        "../../../src/canvas/kit/generated/kit.min.css"
    )));
    assert!(kit.ask_head.contains(include_str!(
        "../../../src/canvas/kit/generated/kit-ask.min.js"
    )));
    assert!(!kit.head.contains("kit-ask"));
    assert!(kit.head.contains("data-hc-texture-root"), "artifact styles");
    // Light and dark: every frame token, stock values, each one the
    // runtime's theme message takes.
    assert_eq!(kit.themes.light.len(), THEME_TOKENS.len());
    assert_eq!(kit.themes.dark.len(), THEME_TOKENS.len());
    assert_eq!(kit.themes.light["--app-ink"], "#1a1a1a");
    assert_eq!(kit.themes.dark["--app-ink"], "#f9f9f9");
    assert_eq!(kit.themes.light["--app-canvas"], "#ffffff");
    assert_eq!(kit.themes.dark["--app-canvas"], "#1a1a1a");
    assert_eq!(kit.themes.dark["--app-primary"], "#eb847d");
    assert_eq!(kit.themes.light["--app-text-scale"], "1");
    for (name, value) in kit.themes.light.iter().chain(kit.themes.dark.iter()) {
        assert!(
            !value.is_empty()
                && value.len() <= 400
                && !value.contains([';', '{', '}', '<', '>', '\\'])
                && !value.contains("var("),
            "{name}: {value}"
        );
    }
    assert!(kit
        .head
        .contains("@media (prefers-color-scheme: dark){:root{"));
    // The fixture's keys (its theme maps are cut short).
    let ours = value(kit);
    let theirs = fixture("canvases/canvas-kit.json");
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&ours), keys(&theirs));
    assert_eq!(keys(&ours["themes"]), keys(&theirs["themes"]));
    for (name, value) in theirs["themes"]["light"].as_object().unwrap() {
        assert_eq!(&ours["themes"]["light"][name], value, "{name}");
    }
    for (name, value) in theirs["themes"]["dark"].as_object().unwrap() {
        assert_eq!(&ours["themes"]["dark"][name], value, "{name}");
    }
}

#[test]
fn the_theme_tokens_are_the_frames_theme_tokens() {
    let start = SANDBOX_TS
        .find("export const THEME_TOKENS")
        .expect("THEME_TOKENS");
    let block = &SANDBOX_TS[start..];
    let block = &block[..block.find("];").expect("end")];
    assert_eq!(block.matches("['--app-").count(), THEME_TOKENS.len());
    for (name, token, fallback) in THEME_TOKENS {
        let row = format!("['{name}', '{token}', '{fallback}'],");
        assert!(block.contains(&row), "{row}");
    }
}

#[test]
fn a_frame_body_is_canvas_frame_body_for_the_fixture_pairs() {
    for name in ["canvases/canvas.json", "canvases/canvas-ask.json"] {
        let f = fixture(name);
        let source = f["source"].as_str().unwrap();
        let valid = crate::canvas_fence::validate(source).unwrap();
        assert_eq!(
            canvases::frame_body(source, valid.inner),
            f["body"].as_str().unwrap(),
            "{name}"
        );
    }
    // A root that asks for nothing, or for plain, is dithered.
    for (attr, want) in [
        ("", "dither"),
        (" texture=\"plain\"", "dither"),
        (" TEXTURE=\" ASCII \"", "ascii"),
        (" texture=\"&#97;scii\"", "ascii"),
        (" texture=\"dither\"", "dither"),
    ] {
        let source = format!("<hc-canvas id=\"a\" title=\"A\"{attr}><p>x</p></hc-canvas>");
        let valid = crate::canvas_fence::validate(&source).unwrap();
        assert_eq!(
            canvases::frame_body(&source, valid.inner),
            format!(
                "<div data-hc-texture-root texture=\"{want}\" style=\"display:contents\"><p>x</p></div>"
            ),
            "{attr}"
        );
    }
}

#[test]
fn the_answer_text_is_what_the_macs_send_writes() {
    let request = fixture("canvases/answer-request.json");
    let text = canvases::answer_text(
        "launch-order",
        request["revision"].as_u64().unwrap(),
        &request["answers"],
        request["summary"].as_str().unwrap(),
    );
    assert_eq!(text, fixture_text("canvases/answer-message.txt").trim_end());
    // The lock reads it back, and the person sees the lines.
    assert_eq!(
        crate::canvas_answers::answered(&text),
        vec![("launch-order".to_string(), 3)]
    );
    assert_eq!(
        canvases::sent_lines(&text),
        "- What goes out first?: Docs, Beta, Pricing page\n- Launch budget: 20 k$\n- When?: This Friday"
    );
    // A line without "label: " is one plain line; blank lines go.
    let text = canvases::answer_text("a", 1, &json!({"x": null}), "Just this\n\n  \nx: \t y ");
    assert!(text
        .starts_with("Answered canvas \"a\":\n- Just this\n- x: y\n\n```hollow-canvas-answer\n"));
    // Behind the composer's intent line, the sent lines are the same.
    assert_eq!(
        canvases::sent_lines(&format!("{}{text}", routes::CHAT_INTENT)),
        "- Just this\n- x: y"
    );
}

#[test]
fn the_canvas_closed_error_matches_its_fixture() {
    let error = ApiError::canvas_closed("This canvas was already answered.");
    assert_eq!(error.status, 409);
    assert_eq!(
        value(&error.body()),
        fixture("canvases/error-canvas-closed.json")
    );
}

const STATUS: &str = "<hc-canvas id=\"release-status\" title=\"Release status\" height=\"480\">\n  <hc-stats>\n    <hc-stat label=\"Checks passed\" value=\"41\" tone=\"success\"></hc-stat>\n  </hc-stats>\n</hc-canvas>";
const STATUS_2: &str = "<hc-canvas id=\"release-status\" title=\"Release status\" height=\"480\">\n  <hc-stats>\n    <hc-stat label=\"Checks passed\" value=\"42\" tone=\"success\"></hc-stat>\n  </hc-stats>\n</hc-canvas>";
const ASK: &str = "<hc-canvas id=\"launch-order\" title=\"Launch order\" ask texture=\"ascii\">\n  <hc-ask title=\"Launch order\">\n    <hc-choice name=\"day\" label=\"When?\" required>\n    * This Friday\n    Next week\n    </hc-choice>\n  </hc-ask>\n</hc-canvas>";

fn fenced(body: &str) -> String {
    format!("```hollow-canvas\n{body}\n```")
}

/// A workspace chat whose replies drew `release-status` twice (two
/// revisions) and the ask-canvas `launch-order`, all saved as a turn's end
/// saves them.
fn canvas_chat(rig: &Rig, key: &str) -> crate::workspaces::Workspace {
    let ws = crate::workspaces::create(&rig.vault, "Canvas project", "Goal").unwrap();
    let first = format!(
        "Here it is.\n\n{}\n\nAnd a question:\n\n{}",
        fenced(STATUS),
        fenced(ASK)
    );
    let second = format!("Updated.\n\n{}", fenced(STATUS_2));
    seed_chat(
        &rig.vault,
        key,
        "claude",
        &ws.cwd,
        &[
            ("Show the release", first.as_str()),
            ("Again", second.as_str()),
        ],
        "acceptEdits",
    );
    let app = crate::agent_settings::app_data_dir().unwrap();
    let scope = crate::workspace_canvases::Scope {
        workspace_id: Some(&ws.id),
        session_key: Some(key),
    };
    let now = Utc::now().to_rfc3339();
    crate::workspace_canvases::record_reply(
        &rig.vault,
        &app,
        scope,
        &crate::canvas_fence::reply_canvas_bodies(&first),
        "run-1",
        &now,
    );
    crate::workspace_canvases::record_reply(
        &rig.vault,
        &app,
        scope,
        &crate::canvas_fence::reply_canvas_bodies(&second),
        "run-2",
        &now,
    );
    ws
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_chats_canvases_list_read_and_answer_as_the_window_does() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261107-ca1a0001";
    canvas_chat(&rig, key);

    // The kit, behind the token.
    let resp = rig.call("GET", "/v1/canvas-kit", None, None).await;
    assert_eq!(resp.status, 401);
    let resp = rig.call("GET", "/v1/canvas-kit", Some(&token), None).await;
    assert_eq!(resp.status, 200);
    assert_eq!(resp.json(), value(canvases::kit()));
    let sha = canvases::kit().sha256.clone();

    // The list: every fence in transcript order with its own revision.
    let path = format!("/v1/sessions/{key}/canvases");
    let resp = rig.call("GET", &path, Some(&token), None).await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let list = resp.json();
    same_shape(&list, &fixture("canvases/session-canvases.json"), "list");
    assert_eq!(list["kitSha256"], json!(sha));
    let rows = list["canvases"].as_array().unwrap();
    let got: Vec<(String, u64, String, Value, Value)> = rows
        .iter()
        .map(|r| {
            (
                str_of(&r["turnId"]),
                r["index"].as_u64().unwrap(),
                str_of(&r["id"]),
                r["revision"].clone(),
                r["latestRevision"].clone(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            (
                format!("{key}-1-assistant"),
                0,
                "release-status".into(),
                json!(1),
                json!(2)
            ),
            (
                format!("{key}-1-assistant"),
                0,
                "launch-order".into(),
                json!(1),
                json!(1)
            ),
            (
                format!("{key}-2-assistant"),
                0,
                "release-status".into(),
                json!(2),
                json!(2)
            ),
        ]
    );
    assert_eq!(rows[1]["ask"], json!(true));
    assert_eq!(rows[1]["answered"], Value::Null);
    assert_eq!(rows[0]["height"], json!(480));

    // One canvas: the current revision, or a kept one.
    let resp = rig
        .call("GET", &format!("{path}/release-status"), Some(&token), None)
        .await;
    assert_eq!(resp.status, 200);
    let detail = resp.json();
    same_shape(&detail, &fixture("canvases/canvas.json"), "detail");
    assert_eq!(detail["revision"], json!(2));
    assert_eq!(detail["source"], json!(STATUS_2));
    assert!(str_of(&detail["body"]).starts_with("<div data-hc-texture-root texture=\"dither\""));
    let resp = rig
        .call(
            "GET",
            &format!("{path}/release-status?revision=1"),
            Some(&token),
            None,
        )
        .await;
    assert_eq!(resp.json()["source"], json!(STATUS));
    assert_eq!(resp.json()["latestRevision"], json!(2));
    for (query, status) in [
        ("/release-status?revision=9", 404),
        ("/release-status?revision=0", 400),
        ("/nope", 404),
        ("/Bad_Id", 400),
    ] {
        let resp = rig
            .call("GET", &format!("{path}{query}"), Some(&token), None)
            .await;
        assert_eq!(resp.status, status, "{query}");
    }

    // Answering: the window's Send text, as the person's composer send.
    let answer_path = format!("{path}/launch-order/answer");
    let mut request = fixture("canvases/answer-request.json");
    request["revision"] = json!(1);
    let resp = rig
        .call("POST", &answer_path, Some(&token), Some(request.clone()))
        .await;
    assert_eq!(resp.status, 202, "{}", String::from_utf8_lossy(&resp.body));
    let accepted = resp.json();
    same_shape(&accepted, &fixture("send-accepted.json"), "accepted");
    let sent = rig.host.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    let want = canvases::answer_text(
        "launch-order",
        1,
        &request["answers"],
        request["summary"].as_str().unwrap(),
    );
    assert_eq!(sent[0].prompt, format!("{}{want}", routes::CHAT_INTENT));
    assert_eq!(sent[0].session_key.as_deref(), Some(key));
    assert_eq!(sent[0].permission_mode.as_deref(), Some("acceptEdits"));
    assert!(sent[0].typed && sent[0].app_depth == 0);
    // A retry gets the first answer and sends nothing.
    let again = rig
        .call("POST", &answer_path, Some(&token), Some(request.clone()))
        .await;
    assert_eq!((again.status, again.json()), (202, accepted));
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
    // The list and the read now show it answered, with the sent lines.
    let lines = "- What goes out first?: Docs, Beta, Pricing page\n- Launch budget: 20 k$\n- When?: This Friday";
    let resp = rig.call("GET", &path, Some(&token), None).await;
    assert_eq!(
        resp.json()["canvases"][1]["answered"],
        json!({"revision": 1, "summary": lines})
    );
    let resp = rig
        .call("GET", &format!("{path}/launch-order"), Some(&token), None)
        .await;
    let detail = resp.json();
    same_shape(&detail, &fixture("canvases/canvas-ask.json"), "ask detail");
    assert_eq!(detail["answered"], json!({"revision": 1, "summary": lines}));
    assert!(str_of(&detail["body"]).starts_with("<div data-hc-texture-root texture=\"ascii\""));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_answer_the_window_would_refuse_is_refused() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261107-ca1a0002";
    canvas_chat(&rig, key);
    let base = format!("/v1/sessions/{key}/canvases");
    let mut request = fixture("canvases/answer-request.json");
    let post = |path: String, body: Value| {
        let rig = &rig;
        let token = token.clone();
        async move { rig.call("POST", &path, Some(&token), Some(body)).await }
    };
    // Not an ask-canvas; not its current revision; no such canvas.
    request["revision"] = json!(2);
    let resp = post(format!("{base}/release-status/answer"), request.clone()).await;
    assert_eq!(
        (resp.status, str_of(&resp.json()["error"]["code"])),
        (409, "canvas_closed".into())
    );
    request["revision"] = json!(2);
    let resp = post(format!("{base}/launch-order/answer"), request.clone()).await;
    assert_eq!(
        (resp.status, str_of(&resp.json()["error"]["code"])),
        (409, "canvas_closed".into())
    );
    request["revision"] = json!(1);
    let resp = post(format!("{base}/missing/answer"), request.clone()).await;
    assert_eq!(resp.status, 404);
    // Malformed bodies.
    for (field, bad) in [
        ("answers", json!({"bad key": "x"})),
        ("answers", json!({"deep": {"a": {"b": "c"}}})),
        ("answers", json!(["not", "a", "map"])),
        ("summary", json!("  ")),
        ("summary", json!("x".repeat(501))),
        ("summary", json!("bell \u{7}")),
        ("revision", json!(0)),
        ("clientMessageId", json!("not a uuid!")),
    ] {
        let mut body = request.clone();
        body[field] = bad;
        let resp = post(format!("{base}/launch-order/answer"), body).await;
        assert_eq!(resp.status, 400, "{field}");
    }
    assert!(rig.host.sent.lock().unwrap().is_empty());
    // Answered once, a second answer (another message) is closed.
    let resp = post(format!("{base}/launch-order/answer"), request.clone()).await;
    assert_eq!(resp.status, 202);
    request["clientMessageId"] = json!("7a2b3c4d-5e6f-4a1b-8c9d-0e1f2a3b4c5d");
    let resp = post(format!("{base}/launch-order/answer"), request.clone()).await;
    assert_eq!(resp.status, 409);
    assert_eq!(resp.json(), fixture("canvases/error-canvas-closed.json"));
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_overlapping_retry_gets_the_first_answer() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261107-ca1a0004";
    canvas_chat(&rig, key);
    let path = format!("/v1/sessions/{key}/canvases/launch-order/answer");
    let mut request = fixture("canvases/answer-request.json");
    request["revision"] = json!(1);
    // Another answer holds the lock while the phone's send and its retry
    // (the same clientMessageId) both pass the first repeat check, as when
    // the retry goes before the first reply arrives (review of PR #1185).
    let held = canvases::answering().lock().await;
    let release = async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        drop(held);
    };
    let (first, retry, ()) = tokio::join!(
        rig.call("POST", &path, Some(&token), Some(request.clone())),
        rig.call("POST", &path, Some(&token), Some(request.clone())),
        release,
    );
    assert_eq!(
        first.status,
        202,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    assert_eq!(
        (retry.status, retry.json()),
        (202, first.json()),
        "{}",
        String::from_utf8_lossy(&retry.body)
    );
    assert_eq!(rig.host.sent.lock().unwrap().len(), 1);
}

#[test]
fn a_one_line_theme_block_ends_at_its_brace() {
    // The glass rule after the one-line block isn't the dark theme's.
    let css = ":root {\n  --ink: #111;\n}\n[data-theme=\"dark\"] { --ambient-tint: 8%; }\n.glass {\n  --blur: 4px;\n  --ink: red;\n}\n";
    let dark = canvases::declarations(css, "[data-theme=\"dark\"]");
    assert_eq!(
        dark,
        HashMap::from([("--ambient-tint".to_string(), "8%".to_string())])
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_workspaces_canvases_list_and_read_without_answers() {
    let rig = rig().await;
    let token = rig.token();
    let key = "s-20261107-ca1a0003";
    let ws = canvas_chat(&rig, key);
    let path = format!("/v1/workspaces/{}/canvases", ws.id);
    let resp = rig.call("GET", &path, Some(&token), None).await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let list = resp.json();
    same_shape(&list, &fixture("canvases/workspace-canvases.json"), "list");
    let mut ids: Vec<String> = list["canvases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| str_of(&c["id"]))
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["launch-order", "release-status"]);
    let resp = rig
        .call("GET", &format!("{path}/launch-order"), Some(&token), None)
        .await;
    assert_eq!(resp.status, 200);
    let detail = resp.json();
    same_shape(&detail, &fixture("canvases/canvas.json"), "detail");
    assert_eq!(detail["answered"], Value::Null);
    assert_eq!(detail["ask"], json!(true));
    for bad in [
        "/v1/workspaces/nope/canvases",
        "/v1/workspaces/..%2F..%2Fx/canvases",
    ] {
        let resp = rig.call("GET", bad, Some(&token), None).await;
        assert_eq!(resp.status, 404, "{bad}");
    }
    let resp = rig
        .call("GET", &format!("{path}/missing"), Some(&token), None)
        .await;
    assert_eq!(resp.status, 404);
}
