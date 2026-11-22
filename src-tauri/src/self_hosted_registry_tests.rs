//! Tests for `self_hosted_registry` (attached as `#[path] mod tests`, so
//! `super::` is the registry): the file, every validation rule, and Test
//! against a stub upstream on loopback, in the style of
//! `self_hosted_forwarder_tests.rs`.
use super::*;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;

fn entry(id: &str) -> SelfHostedEntry {
    SelfHostedEntry {
        id: id.into(),
        ..preset()
    }
}

/// A forwarder entry id unique to this test process and `name`.
fn unique(name: &str) -> String {
    format!("t{}-{name}", std::process::id())
}

fn registry() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app-data").join(FILE);
    (dir, path)
}

#[test]
fn the_schema_round_trips_in_camel_case_with_its_version() {
    let (_dir, path) = registry();
    let open = SelfHostedEntry {
        id: "local".into(),
        name: "Local vLLM".into(),
        base_url: "http://127.0.0.1:8000/".into(),
        auth: SelfHostedAuth::None,
        enabled: false,
        ..preset()
    };
    save_at(&path, preset(), false).unwrap();
    save_at(&path, open, false).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(raw["version"], 1);
    assert_eq!(
        raw["entries"][0],
        serde_json::json!({
            "id": "qwen",
            "name": "Qwen3.8 27B",
            "baseUrl": "https://qwen-27b-x6jmsblvea-uc.a.run.app",
            "apiStyle": "anthropic",
            "servedModel": "qwen3.8-27b",
            "contextWindow": 131072,
            "effortLevels": ["low", "medium", "xhigh"],
            "auth": {"kind": "tokenCommand", "program": "gcloud", "args": ["auth", "print-identity-token"]},
            "healthPath": "/health",
            "enabled": true,
        })
    );
    assert_eq!(
        raw["entries"][1]["auth"],
        serde_json::json!({"kind": "none"})
    );
    // The trailing slash is gone once saved.
    assert_eq!(raw["entries"][1]["baseUrl"], "http://127.0.0.1:8000");
    let read = read_at(&path).unwrap();
    assert_eq!(read.len(), 2);
    assert_eq!(read[0], preset());
    assert_eq!(read[1].auth, SelfHostedAuth::None);
    assert!(!read[1].enabled);
    // A hand-written entry without the defaulted fields reads with them.
    std::fs::write(
        &path,
        r#"{"version":1,"entries":[{"id":"x","name":"X","baseUrl":"https://x.example","servedModel":"m","contextWindow":200000,"effortLevels":["high"],"auth":{"kind":"tokenCommand","program":"tok"}}]}"#,
    )
    .unwrap();
    let read = read_at(&path).unwrap();
    assert_eq!(read[0].api_style, "anthropic");
    assert_eq!(read[0].health_path, "/health");
    assert!(read[0].enabled);
    assert_eq!(
        read[0].auth,
        SelfHostedAuth::TokenCommand {
            program: "tok".into(),
            args: vec![]
        }
    );
}

#[test]
fn a_missing_file_is_an_empty_list_and_the_preset_is_not_saved() {
    let (_dir, path) = registry();
    let listed = list_at(&path);
    assert!(listed.entries.is_empty());
    assert_eq!(listed.error, None);
    assert_eq!(listed.preset, preset());
    assert_eq!(listed.levels, crate::effort::LEVELS.to_vec());
    assert_eq!(
        (listed.min_context_window, listed.max_context_window),
        (110_000, 10_000_000)
    );
    assert!(!path.exists(), "listing writes nothing");
    // The preset passes every rule as it stands.
    assert_eq!(normalize(preset()).unwrap(), preset());
}

#[test]
fn an_unknown_version_is_refused_and_never_overwritten() {
    let (_dir, path) = registry();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let newer = r#"{"version":2,"entries":[{"id":"later"}]}"#;
    std::fs::write(&path, newer).unwrap();
    let listed = list_at(&path);
    let why = listed.error.unwrap();
    assert!(
        why.contains("version 2") && why.contains("version 1"),
        "{why}"
    );
    assert!(listed.entries.is_empty());
    for refused in [
        save_at(&path, preset(), false).map(|_| ()),
        remove_at(&path, "later").map(|_| ()),
        set_enabled_at(&path, "later", false).map(|_| ()),
        find_at(&path, "later").map(|_| ()),
    ] {
        assert!(refused.unwrap_err().contains("version 2"));
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), newer);
}

#[test]
fn an_unreadable_file_is_reported_and_kept() {
    let (_dir, path) = registry();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    for (bad, says) in [
        ("{not json", "isn't valid JSON"),
        (r#"{"entries":[]}"#, "no version"),
        (
            r#"{"version":1,"entries":[{"id":3}]}"#,
            "doesn't match version 1",
        ),
    ] {
        std::fs::write(&path, bad).unwrap();
        let why = list_at(&path).error.unwrap();
        assert!(why.contains(says), "{bad}: {why}");
        assert!(save_at(&path, preset(), false).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), bad);
    }
}

type Edit = fn(&mut SelfHostedEntry);

/// A refused edit and what its message says (a call, so each closure
/// becomes an `Edit`).
fn refused(
    what: &'static str,
    edit: Edit,
    says: &'static str,
) -> (&'static str, Edit, &'static str) {
    (what, edit, says)
}

fn passes(what: &'static str, edit: Edit) -> (&'static str, Edit) {
    (what, edit)
}

fn token_command(program: &str, args: Vec<String>) -> SelfHostedAuth {
    SelfHostedAuth::TokenCommand {
        program: program.into(),
        args,
    }
}

/// Every rule refuses with its own plain message.
#[test]
fn every_validation_rule_has_a_specific_message() {
    let cases = [
        refused(
            "empty id",
            |e| e.id = String::new(),
            "The id must be 1 to 32",
        ),
        refused("upper-case id", |e| e.id = "Qwen".into(), "The id must be"),
        refused(
            "id starting with a hyphen",
            |e| e.id = "-q".into(),
            "The id must be",
        ),
        refused("id with a dot", |e| e.id = "q.w".into(), "The id must be"),
        refused(
            "33-character id",
            |e| e.id = "a".repeat(33),
            "The id must be",
        ),
        refused(
            "empty name",
            |e| e.name = "  ".into(),
            "The name must be 1 to 80",
        ),
        refused(
            "long name",
            |e| e.name = "n".repeat(81),
            "The name must be 1 to 80",
        ),
        refused(
            "plain http",
            |e| e.base_url = "http://qwen.example.com".into(),
            "https://",
        ),
        refused(
            "http to the LAN",
            |e| e.base_url = "http://10.0.0.2:8000".into(),
            "https://",
        ),
        refused(
            "no scheme",
            |e| e.base_url = "qwen.example.com".into(),
            "https://",
        ),
        refused(
            "no host",
            |e| e.base_url = "https:///v1".into(),
            "needs a host name",
        ),
        refused(
            "a user name and password",
            |e| e.base_url = "https://me:pw@qwen.example.com".into(),
            "user name or password",
        ),
        refused(
            "a query",
            |e| e.base_url = "https://qwen.example.com/?key=1".into(),
            "query (?) or a fragment (#)",
        ),
        refused(
            "an empty query",
            |e| e.base_url = "https://qwen.example.com?".into(),
            "query (?) or a fragment (#)",
        ),
        refused(
            "a fragment",
            |e| e.base_url = "https://qwen.example.com#v1".into(),
            "query (?) or a fragment (#)",
        ),
        refused(
            "a space in the address",
            |e| e.base_url = "https://qwen example.com".into(),
            "spaces or control characters",
        ),
        refused(
            "a bad port",
            |e| e.base_url = "https://qwen.example.com:99999".into(),
            "isn't a valid URL",
        ),
        refused(
            "a control character in the name",
            |e| e.name = "Qwen\u{7}".into(),
            "The name can't contain control characters",
        ),
        refused(
            "a control character in the served model",
            |e| e.served_model = "qwen\u{1b}[0m".into(),
            "can't contain control characters",
        ),
        refused(
            "a served model starting with a hyphen",
            |e| e.served_model = "--model".into(),
            "can't start with a hyphen",
        ),
        refused(
            "brackets in the served model",
            |e| e.served_model = "qwen3.8-27b[1m]".into(),
            "can't contain [ or ]",
        ),
        refused(
            "an unknown API style",
            |e| e.api_style = "gemini".into(),
            "OpenAI's Responses API, run on Codex",
        ),
        refused(
            "max on an openai entry (Codex has no max)",
            |e| {
                e.api_style = "openai".into();
                e.effort_levels = vec!["medium".into(), "max".into()];
            },
            "isn't a level Codex sends",
        ),
        refused(
            "empty served model",
            |e| e.served_model = String::new(),
            "The served model must be",
        ),
        refused(
            "served model with a space",
            |e| e.served_model = "qwen 27b".into(),
            "no spaces",
        ),
        refused(
            "long served model",
            |e| e.served_model = "m".repeat(129),
            "1 to 128",
        ),
        refused(
            "a claude id",
            |e| e.served_model = "claude-sonnet-4-5".into(),
            "one Claude Code uses",
        ),
        refused(
            "a Claude id in capitals",
            |e| e.served_model = "Claude-Opus".into(),
            "one Claude Code uses",
        ),
        refused(
            "the opus alias",
            |e| e.served_model = "opus".into(),
            "one Claude Code uses",
        ),
        refused(
            "small window",
            |e| e.context_window = 109_999,
            "compacts at 100,000",
        ),
        refused(
            "huge window",
            |e| e.context_window = 10_000_001,
            "between 110,000 and 10,000,000",
        ),
        refused(
            "no levels",
            |e| e.effort_levels = vec![],
            "at least one effort level",
        ),
        refused(
            "unknown level",
            |e| e.effort_levels = vec!["ultracode".into()],
            "\"ultracode\" is not an effort level",
        ),
        refused(
            "a level twice",
            |e| e.effort_levels = vec!["low".into(), "low".into()],
            "\"low\" is listed twice",
        ),
        refused(
            "no program",
            |e| e.auth = token_command(" ", vec![]),
            "needs a program",
        ),
        refused(
            "17 arguments",
            |e| e.auth = token_command("tok", vec!["a".into(); 17]),
            "at most 16 arguments",
        ),
        refused(
            "a NUL in an argument",
            |e| e.auth = token_command("tok", vec!["a\0b".into()]),
            "NUL",
        ),
        refused(
            "a NUL in the program",
            |e| e.auth = token_command("to\0k", vec![]),
            "NUL",
        ),
        refused(
            "relative health path",
            |e| e.health_path = "health".into(),
            "must start with /",
        ),
        refused(
            "health path with a space",
            |e| e.health_path = "/he alth".into(),
            "no spaces",
        ),
    ];
    for (what, edit, says) in cases {
        let mut e = preset();
        edit(&mut e);
        let why = normalize(e).unwrap_err();
        assert!(why.contains(says), "{what}: {why}");
    }
    // The edges pass.
    for (what, edit) in [
        passes("32-character id", |e| e.id = format!("q{}", "-".repeat(31))),
        passes("digit first", |e| e.id = "27b".into()),
        passes("80-character name", |e| e.name = "n".repeat(80)),
        passes("localhost over http", |e| {
            e.base_url = "http://localhost:11434".into()
        }),
        passes("loopback over http", |e| {
            e.base_url = "http://127.0.0.1:8000".into()
        }),
        passes("a path after the host", |e| {
            e.base_url = "https://gpu.example.com/qwen".into()
        }),
        passes("a hyphen inside the served model", |e| {
            e.served_model = "qwen3.8-27b-awq".into()
        }),
        passes("smallest window", |e| e.context_window = 110_000),
        passes("largest window", |e| e.context_window = 10_000_000),
        passes("16 arguments", |e| {
            e.auth = token_command("tok", vec!["a".into(); 16])
        }),
        passes("an open endpoint", |e| e.auth = SelfHostedAuth::None),
        passes("an empty health path", |e| e.health_path = String::new()),
        // Q9: OpenAI's Responses API, for Codex, with Codex's levels.
        passes("openai style", |e| e.api_style = "openai".into()),
        passes("openai style with every Codex level", |e| {
            e.api_style = "openai".into();
            e.effort_levels = vec!["xhigh".into(), "high".into(), "medium".into(), "low".into()];
        }),
    ] {
        let mut e = preset();
        edit(&mut e);
        assert!(normalize(e).is_ok(), "{what}");
    }
    let mut blank = preset();
    blank.health_path = " ".into();
    assert_eq!(normalize(blank).unwrap().health_path, "/health");
}

/// Q9 review: an `openai` entry may not serve an id shaped like one of
/// OpenAI's own (it would refuse every ordinary Codex run on that id once
/// remembered), nor one Codex's own catalog lists; an `anthropic` entry may
/// (`gpt-oss-120b` on Claude Code is fine, Codex never refuses it).
#[test]
fn an_openai_entry_never_serves_openais_own_ids() {
    let openai = |served: &str| SelfHostedEntry {
        api_style: API_OPENAI.into(),
        served_model: served.into(),
        ..preset()
    };
    for id in [
        "gpt-5.5",
        "GPT-5-codex",
        "gpt-oss-120b",
        "o3",
        "o4-mini",
        "codex-mini-latest",
        "chatgpt-4o-latest",
    ] {
        let why = normalize(openai(id)).unwrap_err();
        assert!(
            why.contains("one OpenAI uses for its own models"),
            "{id}: {why}"
        );
        assert!(why.contains("--served-model-name"), "{id}: {why}");
    }
    for id in [
        "qwen3.8-27b",
        "omni-27b",
        "local-gpt-oss-120b",
        "o",
        "openai-like",
    ] {
        assert!(normalize(openai(id)).is_ok(), "{id}");
    }
    // Anthropic-style entries keep their old rule only.
    let anthropic = SelfHostedEntry {
        served_model: "gpt-oss-120b".into(),
        ..preset()
    };
    assert!(normalize(anthropic).is_ok());

    // Codex's own catalog, read on save.
    let catalog = br#"{"models":[{"slug":"gpt-6-luna","display_name":"GPT-6 Luna"},{"slug":"house-model","visibility":"hide"},{"nope":1}]}"#;
    let ids = catalog_ids(catalog);
    assert_eq!(ids, ["gpt-6-luna", "house-model"]);
    let why = catalog_problem("House-Model", &ids).unwrap();
    assert!(why.contains("Codex's own catalog"), "{why}");
    assert_eq!(catalog_problem("qwen3.8-27b", &ids), None);
    assert!(catalog_ids(b"not json").is_empty());
}

/// Q3 review: Hollow remembers an entry's window and effort levels by served
/// id, so an id Claude Code uses for its own models would change ordinary
/// Claude runs.
#[test]
fn a_served_model_named_like_claudes_own_is_refused() {
    let says =
        "This model id is one Claude Code uses for its own models; use the endpoint's own id.";
    for id in [
        "claude",
        "claude-sonnet-4-5",
        "CLAUDE-opus-4",
        "Claude3",
        "default",
        "Best",
        "opus",
        "SONNET",
        "haiku",
        "Fable",
        "opusplan",
        "OpusPlan",
        // Q4 review: Claude Code's 1M-window aliases, any case.
        "opus[1m]",
        "Sonnet[1M]",
        "fable[1m]",
        "claude-opus-4-5[1m]",
    ] {
        let mut e = preset();
        e.served_model = id.into();
        assert_eq!(normalize(e).unwrap_err(), says, "{id}");
    }
    for id in [
        "qwen3.8-27b",
        "my-claude",
        "opus-like",
        "sonnet2",
        "llama-haiku",
        "defaults",
    ] {
        let mut e = preset();
        e.served_model = id.into();
        assert!(normalize(e).is_ok(), "{id}");
    }
    // The window floor stays as briefed.
    let mut e = preset();
    e.context_window = 110_000;
    assert!(normalize(e).is_ok());
}

#[test]
fn effort_levels_are_stored_lowest_first() {
    let (_dir, path) = registry();
    let mut e = entry("order");
    e.effort_levels = vec!["max".into(), " low".into(), "high".into()];
    let saved = save_at(&path, e, false).unwrap();
    assert_eq!(saved.entries[0].effort_levels, vec!["low", "high", "max"]);
    assert_eq!(
        read_at(&path).unwrap()[0].effort_levels,
        vec!["low", "high", "max"]
    );
}

#[test]
fn ids_are_unique_and_an_edit_replaces_by_id() {
    let (_dir, path) = registry();
    save_at(&path, entry("qwen"), false).unwrap();
    let twice = save_at(&path, entry("qwen"), false).unwrap_err();
    assert!(twice.contains("already uses the id \"qwen\""), "{twice}");
    let missing = save_at(&path, entry("other"), true).unwrap_err();
    assert!(
        missing.contains("no self-hosted model with the id \"other\""),
        "{missing}"
    );
    let mut renamed = entry("qwen");
    renamed.name = "Qwen on the L4".into();
    let saved = save_at(&path, renamed, true).unwrap();
    assert_eq!(saved.entries.len(), 1);
    assert_eq!(saved.entries[0].name, "Qwen on the L4");
    save_at(&path, entry("second"), false).unwrap();
    let ids: Vec<String> = read_at(&path).unwrap().into_iter().map(|e| e.id).collect();
    assert_eq!(ids, vec!["qwen", "second"], "kept in the order added");
    // An invalid entry changes nothing.
    let mut bad = entry("qwen");
    bad.context_window = 1;
    assert!(save_at(&path, bad, true).is_err());
    assert_eq!(read_at(&path).unwrap()[0].context_window, 131_072);
}

/// Q4's verify: removing or switching off an entry stops its forwarder.
#[tokio::test]
async fn remove_and_switch_off_stop_the_forwarder() {
    let (_dir, path) = registry();
    let id = unique("stop");
    let mut e = entry(&id);
    e.base_url = "http://127.0.0.1:9".into();
    e.auth = SelfHostedAuth::None;
    save_at(&path, e.clone(), false).unwrap();
    forwarder::ensure(e.upstream()).await.unwrap();
    assert!(forwarder::get(&id).is_some());
    let off = set_enabled_at(&path, &id, false).unwrap();
    assert!(!off.entries[0].enabled);
    assert!(forwarder::get(&id).is_none(), "switched off: stopped");
    assert!(!read_at(&path).unwrap()[0].enabled);
    set_enabled_at(&path, &id, true).unwrap();
    forwarder::ensure(e.upstream()).await.unwrap();
    // Saving it switched off stops it too.
    let mut saved_off = e.clone();
    saved_off.enabled = false;
    save_at(&path, saved_off, true).unwrap();
    assert!(forwarder::get(&id).is_none());
    forwarder::ensure(e.upstream()).await.unwrap();
    let gone = remove_at(&path, &id).unwrap();
    assert!(gone.entries.is_empty());
    assert!(forwarder::get(&id).is_none(), "removed: stopped");
    assert!(remove_at(&path, &id)
        .unwrap_err()
        .contains("no self-hosted model"));
    assert!(set_enabled_at(&path, &id, true).is_err());
}

// ── Test, against a stub upstream ───────────────────────────────────────

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: String,
}

#[derive(Clone, Copy)]
struct StubReplies {
    health: u16,
    /// `/v1/models`' status.
    models: u16,
    /// `max_model_len` in `/v1/models`; `None` lists another model only.
    window: Option<u32>,
    messages: u16,
}

struct Stub {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

/// A stub endpoint: `/health`, `/v1/models` (vLLM's shape) and
/// `/v1/messages`, one request per connection.
async fn stub(replies: StubReplies) -> Stub {
    stub_gated(replies, None).await
}

/// [`stub`], whose `/health` answers only once `gate` is notified (a cold
/// start in progress).
async fn stub_gated(replies: StubReplies, gate: Option<Arc<Notify>>) -> Stub {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let log = log.clone();
            let gate = gate.clone();
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let head_end = loop {
                    if let Some(at) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        break at;
                    }
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&chunk[..n]),
                    }
                };
                let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
                let mut lines = head.split("\r\n");
                let path = lines
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("")
                    .to_string();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                let length: usize = headers
                    .iter()
                    .find(|(n, _)| n == "content-length")
                    .and_then(|(_, v)| v.parse().ok())
                    .unwrap_or(0);
                let mut body = buf[head_end + 4..].to_vec();
                while body.len() < length {
                    match stream.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => body.extend_from_slice(&chunk[..n]),
                    }
                }
                log.lock().unwrap().push(Seen {
                    path: path.clone(),
                    headers,
                    body: String::from_utf8_lossy(&body).into_owned(),
                });
                if let (Some(gate), "/health") = (&gate, path.as_str()) {
                    gate.notified().await;
                }
                let (status, text) = match path.as_str() {
                    "/health" => (replies.health, String::new()),
                    "/v1/models" if replies.models != 200 => {
                        (replies.models, r#"{"detail":"Not Found"}"#.to_string())
                    }
                    "/v1/models" => {
                        let model = match replies.window {
                            Some(w) => format!(r#"{{"id":"qwen3.8-27b","object":"model","max_model_len":{w}}}"#),
                            None => r#"{"id":"another-model","object":"model"}"#.to_string(),
                        };
                        (200, format!(r#"{{"object":"list","data":[{model}]}}"#))
                    }
                    "/v1/messages" if replies.messages == 200 => (
                        200,
                        r#"{"type":"message","content":[{"type":"text","text":"ok"}]}"#.to_string(),
                    ),
                    "/v1/messages" => (
                        replies.messages,
                        r#"{"object":"error","message":"The model `qwen3.8-27b` does not exist.","type":"NotFoundError","code":404}"#.to_string(),
                    ),
                    // Q9: OpenAI's Responses API, as vLLM serves it.
                    "/v1/responses" if replies.messages == 200 => (
                        200,
                        r#"{"id":"resp_q9","object":"response","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}"#.to_string(),
                    ),
                    "/v1/responses" => (
                        replies.messages,
                        r#"{"error":{"message":"The model `qwen3.8-27b` does not exist.","type":"NotFoundError","code":404}}"#.to_string(),
                    ),
                    _ => (404, String::new()),
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    Stub { base, seen }
}

const HEALTHY: StubReplies = StubReplies {
    health: 200,
    models: 200,
    window: Some(131_072),
    messages: 200,
};

/// The paths `stub` has seen, in order.
fn paths(stub: &Stub) -> Vec<String> {
    stub.seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.path.clone())
        .collect()
}

/// Waits until `stub` has seen a request for `path`.
async fn wait_for_path(stub: &Stub, path: &str) {
    for _ in 0..400 {
        if paths(stub).iter().any(|p| p == path) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the stub never saw {path}: {:?}", paths(stub));
}

/// An entry on `stub` whose token command prints `tok-registry`.
fn stub_entry(id: &str, stub: &Stub) -> SelfHostedEntry {
    SelfHostedEntry {
        id: id.into(),
        base_url: stub.base.clone(),
        auth: SelfHostedAuth::TokenCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo tok-registry".into()],
        },
        ..preset()
    }
}

/// Q4's verify: Test goes through the forwarder the way a run does, and
/// neither the token nor the run key comes back.
#[tokio::test]
async fn test_checks_health_the_model_list_and_one_completion() {
    let up = stub(HEALTHY).await;
    let id = unique("test-ok");
    let e = stub_entry(&id, &up);
    let result = test_entry(&e, forwarder::stops(&id)).await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.health_status, Some(200));
    assert!(result.health_ms.is_some());
    assert_eq!(result.models_status, Some(200));
    assert_eq!(result.model_listed, Some(true));
    assert_eq!(result.reported_window, Some(131_072));
    assert!(!result.window_mismatch);
    assert_eq!(result.status, Some(200));
    assert!(result.first_byte_ms.is_some());
    assert_eq!(result.error, None);

    let seen = up.seen.lock().unwrap().clone();
    let paths: Vec<&str> = seen.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, vec!["/health", "/v1/models", "/v1/messages"]);
    for s in &seen {
        let auth = s.headers.iter().find(|(n, _)| n == "authorization");
        assert_eq!(
            auth.map(|(_, v)| v.as_str()),
            Some("Bearer tok-registry"),
            "{}",
            s.path
        );
        assert!(
            !s.headers.iter().any(|(_, v)| v.contains("hsr_")),
            "the run key stays on this Mac: {:?}",
            s.headers
        );
    }
    let completion: serde_json::Value = serde_json::from_str(&seen[2].body).unwrap();
    assert_eq!(
        completion,
        serde_json::json!({"model": "qwen3.8-27b", "max_tokens": 1, "messages": [{"role": "user", "content": "ok"}]})
    );
    let version = seen[2]
        .headers
        .iter()
        .find(|(n, _)| n == "anthropic-version");
    assert_eq!(version.map(|(_, v)| v.as_str()), Some("2023-06-01"));
    // The key was revoked as soon as the call ended.
    assert_eq!(forwarder::get(&id).unwrap().live_keys(), 0);
    // Nothing returned carries the token or a key.
    let wire = serde_json::to_string(&result).unwrap();
    assert!(
        !wire.contains("tok-registry") && !wire.contains("hsr_"),
        "{wire}"
    );
    forwarder::stop(&id);
}

/// Q9: an `openai` entry's Test sends its one token the way Codex does,
/// `POST /v1/responses` with the run key as a bearer (no
/// `anthropic-version`), and a refusal's message comes back in OpenAI's
/// shape.
#[tokio::test]
async fn an_openai_entrys_test_asks_the_responses_api() {
    let up = stub(HEALTHY).await;
    let id = unique("test-openai");
    let e = SelfHostedEntry {
        api_style: API_OPENAI.into(),
        ..stub_entry(&id, &up)
    };
    let result = test_entry(&e, forwarder::stops(&id)).await;
    assert!(result.ok, "{result:?}");
    assert_eq!(result.status, Some(200));
    assert!(result.first_byte_ms.is_some());
    assert_eq!(result.model_listed, Some(true));
    let seen = up.seen.lock().unwrap().clone();
    let paths: Vec<&str> = seen.iter().map(|s| s.path.as_str()).collect();
    assert_eq!(paths, vec!["/health", "/v1/models", "/v1/responses"]);
    let call = &seen[2];
    let body: serde_json::Value = serde_json::from_str(&call.body).unwrap();
    assert_eq!(
        body,
        serde_json::json!({"model": "qwen3.8-27b", "input": "ok", "max_output_tokens": 1, "stream": false, "store": false})
    );
    assert!(!call.headers.iter().any(|(n, _)| n == "anthropic-version"));
    assert_eq!(
        call.headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .map(|(_, v)| v.as_str()),
        Some("Bearer tok-registry"),
        "the endpoint gets its own token, never the run key"
    );
    assert_eq!(forwarder::get(&id).unwrap().live_keys(), 0);
    forwarder::stop(&id);

    // A refusal names the endpoint's own message.
    let down = stub(StubReplies {
        messages: 404,
        ..HEALTHY
    })
    .await;
    let id = unique("test-openai-404");
    let e = SelfHostedEntry {
        api_style: API_OPENAI.into(),
        ..stub_entry(&id, &down)
    };
    let result = test_entry(&e, forwarder::stops(&id)).await;
    assert!(!result.ok);
    assert_eq!(result.status, Some(404));
    assert_eq!(
        result.error.as_deref(),
        Some("The model answered 404: The model `qwen3.8-27b` does not exist.")
    );
    forwarder::stop(&id);
}

#[tokio::test]
async fn test_flags_a_window_the_endpoint_disagrees_with() {
    let up = stub(HEALTHY).await;
    let id = unique("test-window");
    let mut e = stub_entry(&id, &up);
    e.context_window = 200_000;
    let result = test_entry(&e, forwarder::stops(&id)).await;
    assert!(result.ok, "a mismatch is a warning: {result:?}");
    assert_eq!(result.reported_window, Some(131_072));
    assert!(result.window_mismatch);
    forwarder::stop(&id);
}

#[tokio::test]
async fn a_failing_health_check_ends_the_test_before_any_completion() {
    let up = stub(StubReplies {
        health: 503,
        ..HEALTHY
    })
    .await;
    let id = unique("test-health");
    let result = test_entry(&stub_entry(&id, &up), forwarder::stops(&id)).await;
    assert!(!result.ok);
    assert_eq!(result.health_status, Some(503));
    let why = result.error.unwrap();
    assert!(why.contains("/health") && why.contains("503"), "{why}");
    assert_eq!(result.status, None);
    assert_eq!(result.first_byte_ms, None);
    let paths: Vec<String> = up
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.path.clone())
        .collect();
    assert_eq!(paths, vec!["/health"]);
    forwarder::stop(&id);
}

#[tokio::test]
async fn a_refused_completion_names_the_endpoints_message() {
    let up = stub(StubReplies {
        window: None,
        messages: 404,
        ..HEALTHY
    })
    .await;
    let id = unique("test-404");
    let result = test_entry(&stub_entry(&id, &up), forwarder::stops(&id)).await;
    assert!(!result.ok);
    assert_eq!(
        result.model_listed,
        Some(false),
        "only another model is listed"
    );
    assert_eq!(result.reported_window, None);
    assert!(!result.window_mismatch);
    assert_eq!(result.status, Some(404));
    let why = result.error.unwrap();
    assert!(
        why.contains("404") && why.contains("does not exist"),
        "{why}"
    );
    assert_eq!(forwarder::get(&id).unwrap().live_keys(), 0);
    forwarder::stop(&id);
}

#[tokio::test]
async fn an_unreachable_endpoint_is_an_error_not_a_hang() {
    // Port 9 (discard) has nothing listening on loopback.
    let id = unique("test-down");
    let e = SelfHostedEntry {
        id: id.clone(),
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        ..preset()
    };
    let result = test_entry(&e, forwarder::stops(&id)).await;
    assert!(!result.ok);
    assert_eq!(result.health_status, None);
    assert!(
        result
            .error
            .as_deref()
            .unwrap_or("")
            .contains("couldn't be reached"),
        "{result:?}"
    );
    forwarder::stop(&id);
}

/// Q4 review: a model list that can't be read is not "doesn't name it".
#[tokio::test]
async fn an_unreadable_model_list_is_reported_as_unread() {
    let up = stub(StubReplies {
        models: 404,
        ..HEALTHY
    })
    .await;
    let id = unique("test-models-404");
    let result = test_entry(&stub_entry(&id, &up), forwarder::stops(&id)).await;
    assert!(result.ok, "the completion still passed: {result:?}");
    assert_eq!(result.models_status, Some(404));
    assert_eq!(result.model_listed, None);
    assert_eq!(result.reported_window, None);
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["modelsStatus"], 404);
    assert_eq!(wire["modelListed"], serde_json::Value::Null);
    forwarder::stop(&id);
}

/// Q4 review: Settings' Test reads the saved entry, and an entry that was
/// off when it began has its forwarder stopped afterwards; one that is on
/// keeps it.
#[tokio::test]
async fn settings_test_reads_the_saved_entry_and_stops_an_off_forwarder() {
    let up = stub(HEALTHY).await;
    let (_dir, path) = registry();
    let id = unique("test-at");
    let mut e = stub_entry(&id, &up);
    e.enabled = false;
    save_at(&path, e, false).unwrap();
    let result = test_at(path.clone(), id.clone()).await.unwrap();
    assert!(result.ok, "{result:?}");
    assert_eq!(paths(&up), vec!["/health", "/v1/models", "/v1/messages"]);
    assert!(
        forwarder::get(&id).is_none(),
        "an entry still off has its forwarder stopped after the Test"
    );
    set_enabled_at(&path, &id, true).unwrap();
    assert!(test_at(path.clone(), id.clone()).await.unwrap().ok);
    assert!(
        forwarder::get(&id).is_some(),
        "an entry that is on keeps it"
    );
    forwarder::stop(&id);
    let missing = test_at(path.clone(), unique("nobody")).await.unwrap_err();
    assert!(missing.contains("no self-hosted model"), "{missing}");
}

/// Q4 review: a hand-edited entry that breaks a rule (plain http to the
/// LAN) is refused before any call: no forwarder, no token command.
#[tokio::test]
async fn settings_test_refuses_a_hand_edited_entry_before_any_call() {
    let (dir, path) = registry();
    let id = unique("test-plain");
    let ran = dir.path().join("token-ran");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let raw = serde_json::json!({"version": 1, "entries": [{
        "id": id,
        "name": "Plain",
        "baseUrl": "http://10.0.0.2:8000",
        "servedModel": "m",
        "contextWindow": 131_072,
        "effortLevels": ["low"],
        "auth": {"kind": "tokenCommand", "program": "/bin/sh",
                 "args": ["-c", "touch \"$0\"; echo tok", ran.to_string_lossy()]},
    }]});
    std::fs::write(&path, raw.to_string()).unwrap();
    let why = test_at(path.clone(), id.clone()).await.unwrap_err();
    assert!(
        why.starts_with("This entry can't be tested until it is fixed:")
            && why.contains("https://"),
        "{why}"
    );
    assert!(forwarder::get(&id).is_none(), "no forwarder started");
    assert!(!ran.exists(), "the token command never ran");
}

/// Q4 review: switching an off entry on while its Test runs keeps the
/// forwarder the Test started (a run may be using it by then).
#[tokio::test]
async fn switching_an_off_entry_on_during_its_test_keeps_the_forwarder() {
    let gate = Arc::new(Notify::new());
    let up = stub_gated(HEALTHY, Some(gate.clone())).await;
    let (_dir, path) = registry();
    let id = unique("test-reread");
    let mut e = stub_entry(&id, &up);
    e.enabled = false;
    save_at(&path, e, false).unwrap();
    let running = tokio::spawn(test_at(path.clone(), id.clone()));
    wait_for_path(&up, "/health").await;
    set_enabled_at(&path, &id, true).unwrap();
    gate.notify_one();
    let result = running.await.unwrap().unwrap();
    assert!(result.ok, "{result:?}");
    let fwd = forwarder::get(&id).expect("switched on meanwhile: the forwarder stays");
    assert!(!fwd.is_closed());
    forwarder::stop(&id);
}

/// Q4 review: switching an entry off or removing it while its Test waits
/// on a cold start ends the Test at once: no model list, no completion.
#[tokio::test]
async fn switching_off_or_removing_during_a_test_ends_it() {
    for remove in [false, true] {
        // Never released: the model is still starting.
        let up = stub_gated(HEALTHY, Some(Arc::new(Notify::new()))).await;
        let (_dir, path) = registry();
        let id = unique(if remove { "test-gone" } else { "test-off" });
        save_at(&path, stub_entry(&id, &up), false).unwrap();
        let running = tokio::spawn(test_at(path.clone(), id.clone()));
        wait_for_path(&up, "/health").await;
        if remove {
            remove_at(&path, &id).unwrap();
        } else {
            set_enabled_at(&path, &id, false).unwrap();
        }
        let result = tokio::time::timeout(Duration::from_secs(10), running)
            .await
            .expect("the Test ended at the stop")
            .unwrap()
            .unwrap();
        assert!(!result.ok);
        assert_eq!(
            result.error.as_deref(),
            Some(forwarder::STOPPED),
            "{result:?}"
        );
        assert_eq!(result.health_status, None);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(paths(&up), vec!["/health"], "removed {remove}");
        assert!(forwarder::get(&id).is_none());
    }
}

/// Q4 review: every change and Test answers Hollow's main window only.
#[test]
fn changes_and_test_answer_the_main_window_only() {
    let source = include_str!("commands/self_hosted_api.rs");
    assert!(source.contains(r#"if webview.label() != "main" {"#));
    for command in [
        "save_self_hosted_model",
        "remove_self_hosted_model",
        "set_self_hosted_model_enabled",
        "test_self_hosted_model",
    ] {
        let body = source
            .split(&format!("pub async fn {command}("))
            .nth(1)
            .unwrap_or_else(|| panic!("{command}"));
        let body = body.split("#[tauri::command]").next().unwrap();
        assert!(
            body.contains("webview: tauri::Webview") && body.contains("main_window(&webview)?;"),
            "{command}"
        );
    }
}

#[test]
fn model_lists_and_error_bodies_are_read_in_their_common_shapes() {
    let list =
        r#"{"object":"list","data":[{"id":"a"},{"id":"qwen3.8-27b","max_model_len":131072}]}"#;
    assert_eq!(
        served_model_in(list, "qwen3.8-27b"),
        Some((true, Some(131_072)))
    );
    assert_eq!(served_model_in(list, "a"), Some((true, None)));
    assert_eq!(served_model_in(list, "b"), Some((false, None)));
    assert_eq!(served_model_in("<html>", "a"), None);
    assert_eq!(served_model_in(r#"{"detail":"Not Found"}"#, "a"), None);
    assert_eq!(
        error_message(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"bad"}}"#
        ),
        "The model answered 400: bad"
    );
    assert_eq!(
        error_message(404, r#"{"object":"error","message":"no model"}"#),
        "The model answered 404: no model"
    );
    assert_eq!(
        error_message(422, r#"{"detail":"wrong"}"#),
        "The model answered 422: wrong"
    );
    assert_eq!(
        error_message(502, "Bad gateway\nmore"),
        "The model answered 502: Bad gateway"
    );
    assert_eq!(error_message(500, ""), "The model answered 500.");
    assert!(error_message(500, &"x".repeat(1000)).chars().count() < 340);
}
