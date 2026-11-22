//! Tests for `self_hosted_forwarder` against a stub upstream on loopback
//! (attached as `#[path] mod tests`, so `super::` is the forwarder).
use super::*;
use tokio::sync::Notify;

/// One request the stub saw.
#[derive(Clone, Debug)]
struct Seen {
    path: String,
    authorization: Option<String>,
    api_key: Option<String>,
    /// Every header name that went up (Q9).
    names: Vec<String>,
    body: Vec<u8>,
}

/// What the stub answers with.
#[derive(Clone)]
enum Reply {
    /// Status and a whole body.
    Whole(u16, &'static str),
    /// 401 while the bearer is `bad`, else a 200 body.
    RejectBearer(&'static str),
    /// Two chunks with the second held until `release` is notified.
    TwoChunks(Arc<Notify>),
    /// No answer at all (a cold start that never ends).
    Hang,
    /// A redirect to `location`.
    Redirect(String),
    /// Q9: OpenAI Responses events, the last held until `release`.
    Responses(Arc<Notify>),
}

struct Stub {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

async fn stub(reply: Reply) -> Stub {
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
            let reply = reply.clone();
            tokio::spawn(async move {
                let Ok(mut head) = read_head(&mut stream).await else {
                    return;
                };
                let Ok(body) = read_body(&mut stream, &mut head).await else {
                    return;
                };
                let authorization = header(&head.headers, "authorization").map(String::from);
                log.lock().unwrap().push(Seen {
                    path: head.path.clone(),
                    authorization: authorization.clone(),
                    api_key: header(&head.headers, "x-api-key").map(String::from),
                    names: head.headers.iter().map(|(n, _)| n.clone()).collect(),
                    body,
                });
                let whole = |status: u16, body: &str| {
                    format!(
                        "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                };
                match reply {
                    Reply::Whole(status, body) => {
                        let _ = stream.write_all(whole(status, body).as_bytes()).await;
                    }
                    Reply::RejectBearer(bad) => {
                        let text = if authorization.as_deref() == Some(&format!("Bearer {bad}")) {
                            whole(401, r#"{"error":"expired"}"#)
                        } else {
                            whole(200, r#"{"ok":true}"#)
                        };
                        let _ = stream.write_all(text.as_bytes()).await;
                    }
                    Reply::Hang => {
                        std::future::pending::<()>().await;
                    }
                    Reply::Redirect(location) => {
                        let text = format!(
                            "HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        );
                        let _ = stream.write_all(text.as_bytes()).await;
                    }
                    Reply::Responses(release) => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
                        let _ = stream.write_all(head.as_bytes()).await;
                        let _ = stream
                            .write_all(
                                format!("{:x}\r\n{RESPONSES_ONE}\r\n", RESPONSES_ONE.len())
                                    .as_bytes(),
                            )
                            .await;
                        let _ = stream.flush().await;
                        release.notified().await;
                        let _ = stream
                            .write_all(
                                format!(
                                    "{:x}\r\n{RESPONSES_TWO}\r\n0\r\n\r\n",
                                    RESPONSES_TWO.len()
                                )
                                .as_bytes(),
                            )
                            .await;
                    }
                    Reply::TwoChunks(release) => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n";
                        let _ = stream.write_all(head.as_bytes()).await;
                        let one = "event: one\ndata: {}\n\n";
                        let _ = stream
                            .write_all(format!("{:x}\r\n{one}\r\n", one.len()).as_bytes())
                            .await;
                        let _ = stream.flush().await;
                        release.notified().await;
                        let two = "event: two\ndata: {}\n\n";
                        let _ = stream
                            .write_all(format!("{:x}\r\n{two}\r\n0\r\n\r\n", two.len()).as_bytes())
                            .await;
                    }
                }
                let _ = stream.flush().await;
            });
        }
    });
    Stub { base, seen }
}

/// Q9: the Responses stream's first events (the reply's start and a delta).
const RESPONSES_ONE: &str = concat!(
    "event: response.created\n",
    r#"data: {"type":"response.created","response":{"id":"resp_q9"}}"#,
    "\n\nevent: response.output_text.delta\n",
    r#"data: {"type":"response.output_text.delta","delta":"Two tasks"}"#,
    "\n\n",
);
/// Q9: and its end.
const RESPONSES_TWO: &str = concat!(
    "event: response.completed\n",
    r#"data: {"type":"response.completed","response":{"id":"resp_q9","usage":{"input_tokens":40,"output_tokens":3,"total_tokens":43}}}"#,
    "\n\n",
);

/// A token command that prints `tok1`, `tok2`, … counting its runs in a
/// file, and the counter's path.
fn counting_token(dir: &std::path::Path) -> (Auth, std::path::PathBuf) {
    let counter = dir.join("count");
    let script = r#"n=$(cat "$0" 2>/dev/null || echo 0); n=$((n+1)); echo $n > "$0"; echo "tok$n""#;
    (
        Auth::TokenCommand {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                script.into(),
                counter.to_string_lossy().into_owned(),
            ],
        },
        counter,
    )
}

fn runs(counter: &std::path::Path) -> usize {
    std::fs::read_to_string(counter)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn capture() -> (Log, Arc<Mutex<Vec<String>>>) {
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    (
        Arc::new(move |line: &str| sink.lock().unwrap().push(line.to_string())),
        lines,
    )
}

async fn forwarder(base: &str, auth: Auth) -> (Forwarder, Arc<Mutex<Vec<String>>>) {
    forwarder_with(base, auth, Limits::default()).await
}

async fn forwarder_with(
    base: &str,
    auth: Auth,
    limits: Limits,
) -> (Forwarder, Arc<Mutex<Vec<String>>>) {
    let (log, lines) = capture();
    let fwd = Forwarder::start_with(
        Upstream {
            entry: "qwen".into(),
            base_url: base.into(),
            auth,
        },
        log,
        limits,
    )
    .await
    .unwrap();
    (fwd, lines)
}

/// Write `raw` to the forwarder and read whatever comes back until it closes.
async fn raw_exchange(port: u16, raw: &[u8]) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(raw).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(20), stream.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

async fn wait_for_log(lines: &Arc<Mutex<Vec<String>>>, needle: &str) -> bool {
    for _ in 0..200 {
        if lines.lock().unwrap().iter().any(|l| l.contains(needle)) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

fn client() -> reqwest::Client {
    reqwest::Client::new()
}

#[test]
fn upstream_urls_must_be_https_or_this_macs_loopback() {
    assert!(upstream_url_problem("https://qwen-27b-x6jmsblvea-uc.a.run.app").is_none());
    assert!(upstream_url_problem("http://127.0.0.1:8080").is_none());
    assert!(upstream_url_problem("http://localhost:11434").is_none());
    assert!(upstream_url_problem("https://gpu.example.com/qwen").is_none());
    // Q4 delta review: the loopback host comes from the parsed URL, so
    // `[::1]` with no port and a capitalised `localhost` are this Mac too.
    for good in [
        "http://[::1]",
        "http://[::1]/v1",
        "http://[::1]:8000",
        "http://LOCALHOST:8000",
    ] {
        assert!(upstream_url_problem(good).is_none(), "{good}");
    }
    for bad in [
        "http://127.0.0.1.nip.io:8000",
        "http://localhost.example.com",
        "http://[::2]:8000",
        "http://qwen.example.com",
        "http://10.0.0.2:8000",
        "ftp://x",
        "https://",
        "",
        // Q4 review: a host, and nothing the appended path would break.
        "https:///v1",
        "https://me:pw@qwen.example.com",
        "https://me@qwen.example.com",
        "https://qwen.example.com?",
        "https://qwen.example.com/v1?key=1",
        "https://qwen.example.com#v1",
        "https://qwen ex.com",
        "https://qwen.ex\nample.com",
        "https://qwen.example.com:99999",
        "http://127.0.0.1:8000?x",
        "http://127.0.0.1@qwen.example.com",
    ] {
        assert!(upstream_url_problem(bad).is_some(), "{bad}");
    }
}

#[tokio::test]
async fn it_listens_on_loopback_and_stops_at_shutdown() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    assert!(fwd.base_url().starts_with("http://127.0.0.1:"));
    let port = fwd.port();
    assert!(TcpStream::connect(("127.0.0.1", port)).await.is_ok());
    fwd.shutdown();
    // The aborted accept loop drops its listener on its next poll.
    let mut refused = false;
    for _ in 0..50 {
        tokio::task::yield_now().await;
        if TcpStream::connect(("127.0.0.1", port)).await.is_err() {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(refused, "nothing listens after shutdown");
    assert_eq!(fwd.live_keys(), 0);
}

/// Q2's verify: a missing or wrong key is refused, and so are expired and
/// revoked ones, all without reaching the endpoint.
#[tokio::test]
async fn requests_without_a_live_run_key_never_reach_the_endpoint() {
    let up = stub(Reply::Whole(200, r#"{"ok":true}"#)).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    let url = format!("{}/v1/messages", fwd.base_url());
    let none = client().post(&url).body("{}").send().await.unwrap();
    assert_eq!(none.status().as_u16(), 401);
    let body = none.text().await.unwrap();
    assert!(body.contains("authentication_error"), "{body}");
    let wrong = client()
        .post(&url)
        .bearer_auth("hsr_not-a-key")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status().as_u16(), 401);

    let short = fwd.mint(Duration::from_millis(1)).as_str().to_string();
    tokio::time::sleep(Duration::from_millis(20)).await;
    let expired = client()
        .post(&url)
        .bearer_auth(&short)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status().as_u16(), 401);

    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let ok = client()
        .post(&url)
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    // x-api-key works too (Claude Code sends one when only an API key is set).
    let ok = client()
        .post(&url)
        .header("x-api-key", &key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    fwd.revoke(&key);
    let revoked = client()
        .post(&url)
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status().as_u16(), 401);
    assert_eq!(
        up.seen.lock().unwrap().len(),
        2,
        "only the two keyed requests went up"
    );
}

#[tokio::test]
async fn the_endpoint_gets_its_token_and_never_the_run_key() {
    let dir = tempfile::tempdir().unwrap();
    let (auth, counter) = counting_token(dir.path());
    let up = stub(Reply::Whole(200, r#"{"ok":true}"#)).await;
    let (fwd, lines) = forwarder(&up.base, auth).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let body = r#"{"model":"qwen3.8-27b","messages":[]}"#;
    let resp = client()
        .post(format!("{}/v1/messages?beta=true", fwd.base_url()))
        .bearer_auth(&key)
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(resp.text().await.unwrap(), r#"{"ok":true}"#);
    let seen = up.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/messages?beta=true");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer tok1"));
    assert_eq!(seen[0].api_key, None, "the client's key stays here");
    assert_eq!(seen[0].body, body.as_bytes());
    // A second request reuses the cached token.
    let again = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(again.status().as_u16(), 200);
    assert_eq!(runs(&counter), 1, "the token command ran once");
    // Logs: status and timing, never the token, the key or the body.
    let logged = lines.lock().unwrap().join("\n");
    assert!(logged.contains("POST /v1/messages 200"), "{logged}");
    for secret in ["tok1", key.as_str(), "qwen3.8-27b"] {
        assert!(!logged.contains(secret), "{secret} in {logged}");
    }
}

/// Q2's verify: chunks reach the client as the endpoint sends them.
#[tokio::test]
async fn each_chunk_streams_through_as_it_arrives() {
    let release = Arc::new(Notify::new());
    let up = stub(Reply::TwoChunks(release.clone())).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let mut resp = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    // The first event arrives while the endpoint still holds the second.
    let first = tokio::time::timeout(Duration::from_secs(10), resp.chunk())
        .await
        .expect("the first chunk wasn't held back")
        .unwrap()
        .unwrap();
    assert_eq!(&first[..], b"event: one\ndata: {}\n\n");
    release.notify_one();
    let mut rest = Vec::new();
    while let Some(chunk) = resp.chunk().await.unwrap() {
        rest.extend_from_slice(&chunk);
    }
    assert_eq!(rest, b"event: two\ndata: {}\n\n");
}

/// Q9's verify: Codex's `POST /v1/responses` with the run key as its bearer
/// streams through event by event, unchanged; the endpoint gets its own
/// token and none of Codex's own headers; a request with no key, or a
/// wrong one, is refused with a message where OpenAI's clients read one
/// (`error.message`) and never reaches the endpoint.
#[tokio::test]
async fn codexs_responses_stream_passes_through_with_the_run_key() {
    let release = Arc::new(Notify::new());
    let up = stub(Reply::Responses(release.clone())).await;
    let (fwd, lines) = forwarder(
        &up.base,
        Auth::TokenCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo tok-q9".into()],
        },
    )
    .await;
    let url = format!("{}/v1/responses", fwd.base_url());
    let body = r#"{"model":"qwen3.8-27b","stream":true,"store":false,"input":[{"type":"message","role":"user","content":[{"type":"input_text","text":"Which tasks are open?"}]}],"tools":[]}"#;
    for key in [None, Some("hsr_00000000000000000000000000000000")] {
        let mut request = client().post(&url).body(body);
        if let Some(key) = key {
            request = request.bearer_auth(key);
        }
        let refused = request.send().await.unwrap();
        assert_eq!(refused.status().as_u16(), 401);
        let value: serde_json::Value =
            serde_json::from_str(&refused.text().await.unwrap()).unwrap();
        assert_eq!(
            value["error"]["message"],
            "Hollow's forwarder needs this run's key."
        );
    }
    assert!(up.seen.lock().unwrap().is_empty(), "nothing went up");

    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let mut resp = client()
        .post(&url)
        .bearer_auth(&key)
        .header("content-type", "application/json")
        .header("accept", "text/event-stream")
        .header("openai-beta", "responses=experimental")
        .header("originator", "codex_exec")
        .header("session_id", "0199-q9")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "text/event-stream"
    );
    let first = tokio::time::timeout(Duration::from_secs(10), resp.chunk())
        .await
        .expect("the first events weren't held back")
        .unwrap()
        .unwrap();
    assert_eq!(&first[..], RESPONSES_ONE.as_bytes());
    release.notify_one();
    let mut rest = Vec::new();
    while let Some(chunk) = resp.chunk().await.unwrap() {
        rest.extend_from_slice(&chunk);
    }
    assert_eq!(rest, RESPONSES_TWO.as_bytes());

    let seen = up.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/v1/responses");
    assert_eq!(
        seen[0].body,
        body.as_bytes(),
        "the request goes up unchanged"
    );
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer tok-q9"));
    for name in ["openai-beta", "originator", "session_id"] {
        assert!(!seen[0].names.iter().any(|n| n == name), "{name} went up");
    }
    assert!(seen[0].names.iter().any(|n| n == "accept"));
    let logged = lines.lock().unwrap().join("\n");
    assert!(logged.contains("POST /v1/responses 200"), "{logged}");
    for secret in ["tok-q9", key.as_str()] {
        assert!(!logged.contains(secret), "{secret} in {logged}");
    }
}

/// Q2's verify: a 401 from the endpoint fetches a fresh token and retries
/// once; a second 401 goes back to the client.
#[tokio::test]
async fn an_endpoint_401_refreshes_the_token_once() {
    let dir = tempfile::tempdir().unwrap();
    let (auth, counter) = counting_token(dir.path());
    let up = stub(Reply::RejectBearer("tok1")).await;
    let (fwd, _) = forwarder(&up.base, auth).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let resp = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 200);
    let auths: Vec<_> = up
        .seen
        .lock()
        .unwrap()
        .iter()
        .map(|s| s.authorization.clone().unwrap_or_default())
        .collect();
    assert_eq!(auths, vec!["Bearer tok1", "Bearer tok2"]);
    assert_eq!(runs(&counter), 2);

    let always = stub(Reply::Whole(401, r#"{"error":"denied"}"#)).await;
    let dir = tempfile::tempdir().unwrap();
    let (auth, counter) = counting_token(dir.path());
    let (fwd, _) = forwarder(&always.base, auth).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let resp = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
    assert_eq!(
        always.seen.lock().unwrap().len(),
        2,
        "one retry, not a loop"
    );
    assert_eq!(runs(&counter), 2);
}

#[tokio::test]
async fn a_failing_token_command_is_a_502_naming_the_problem() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let auth = Auth::TokenCommand {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "echo 'ERROR: You do not currently have an active account selected.' >&2; exit 1"
                .into(),
        ],
    };
    let (fwd, _) = forwarder(&up.base, auth).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let resp = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 502);
    let body = resp.text().await.unwrap();
    assert!(body.contains("active account"), "{body}");
    assert!(up.seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn hollows_own_get_needs_no_run_key() {
    let dir = tempfile::tempdir().unwrap();
    let (auth, _) = counting_token(dir.path());
    let up = stub(Reply::Whole(200, r#"{"status":"ok"}"#)).await;
    let (fwd, _) = forwarder(&up.base, auth).await;
    let (status, body) = fwd.get("/health", 1024).await.unwrap();
    assert_eq!((status, body.as_str()), (200, r#"{"status":"ok"}"#));
    assert_eq!(
        up.seen.lock().unwrap()[0].authorization.as_deref(),
        Some("Bearer tok1")
    );
}

#[tokio::test]
async fn a_chunked_request_body_is_read_whole() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", fwd.port())).await.unwrap();
    let req = format!(
        "POST /v1/messages HTTP/1.1\r\nhost: x\r\nauthorization: Bearer {key}\r\ntransfer-encoding: chunked\r\n\r\n5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = stream.read_to_end(&mut out).await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 200"));
    assert_eq!(up.seen.lock().unwrap()[0].body, b"hello world");
}

#[tokio::test]
async fn ensure_reuses_a_forwarder_until_its_upstream_changes() {
    let entry = format!("test-ensure-{}", std::process::id());
    let upstream = |base: &str| Upstream {
        entry: entry.clone(),
        base_url: base.into(),
        auth: Auth::None,
    };
    let a = ensure(upstream("http://127.0.0.1:9")).await.unwrap();
    let again = ensure(upstream("http://127.0.0.1:9")).await.unwrap();
    assert_eq!(a.port(), again.port());
    let b = ensure(upstream("http://127.0.0.1:10")).await.unwrap();
    assert_ne!(a.port(), b.port());
    assert_eq!(get(&entry).unwrap().port(), b.port());
    stop(&entry);
    assert!(get(&entry).is_none());
}

/// The token lives in memory only: the forwarder never writes a file.
#[test]
fn the_forwarder_writes_no_files() {
    let source = include_str!("self_hosted_forwarder.rs");
    for write in ["std::fs::", "File::create", "OpenOptions", "tokio::fs"] {
        assert!(!source.contains(write), "{write}");
    }
}

/// Review blocking finding: a crafted chunk size must be refused, never
/// wrap or panic (the release build aborts on panic).
#[tokio::test]
async fn a_crafted_chunk_size_is_refused_without_a_panic() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let raw = format!(
        "POST /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\ntransfer-encoding: chunked\r\n\r\n1\r\na\r\nffffffffffffffff\r\nx"
    );
    let out = raw_exchange(fwd.port(), raw.as_bytes()).await;
    assert!(out.starts_with("HTTP/1.1 413"), "{out}");
    // A size line that never ends is refused too.
    let raw = format!(
        "POST /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\ntransfer-encoding: chunked\r\n\r\n{}",
        "1".repeat(4096)
    );
    let out = raw_exchange(fwd.port(), raw.as_bytes()).await;
    assert!(out.starts_with("HTTP/1.1 400"), "{out}");
    // Still serving.
    let ok = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status().as_u16(), 200);
    assert!(up.seen.lock().unwrap().len() == 1);
}

/// Review: the key is checked right after the headers, so a request
/// without one is refused before any body is read.
#[tokio::test]
async fn the_key_is_checked_before_the_body() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    // Claims 60 MB and sends none of it: answered at once.
    let out = raw_exchange(
        fwd.port(),
        b"POST /v1/messages HTTP/1.1\r\ncontent-length: 60000000\r\n\r\n",
    )
    .await;
    assert!(out.starts_with("HTTP/1.1 401"), "{out}");
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let bad_length = format!(
        "POST /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\ncontent-length: lots\r\n\r\n"
    );
    assert!(raw_exchange(fwd.port(), bad_length.as_bytes())
        .await
        .starts_with("HTTP/1.1 400"));
    let too_big = format!(
        "POST /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\ncontent-length: 70000000\r\n\r\n"
    );
    assert!(raw_exchange(fwd.port(), too_big.as_bytes())
        .await
        .starts_with("HTTP/1.1 413"));
    let put = format!("PUT /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\n\r\n");
    assert!(raw_exchange(fwd.port(), put.as_bytes())
        .await
        .starts_with("HTTP/1.1 405"));
    // A wrong Bearer with the right x-api-key still matches.
    let either = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth("hsr_wrong")
        .header("x-api-key", &key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(either.status().as_u16(), 200);
    assert_eq!(up.seen.lock().unwrap().len(), 1);
}

/// Review: a client that sends nothing is cut off, and the connection cap
/// answers 503 rather than queueing.
#[tokio::test]
async fn silent_clients_time_out_and_connections_are_capped() {
    let up = stub(Reply::Whole(200, "{}")).await;
    let limits = Limits {
        client_headers: Duration::from_secs(3),
        max_connections: 1,
        ..Limits::default()
    };
    let (fwd, _) = forwarder_with(&up.base, Auth::None, limits).await;
    let mut silent = TcpStream::connect(("127.0.0.1", fwd.port())).await.unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let crowded = raw_exchange(fwd.port(), b"GET /health HTTP/1.1\r\n\r\n").await;
    assert!(crowded.starts_with("HTTP/1.1 503"), "{crowded}");
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(10), silent.read_to_end(&mut out)).await;
    assert!(String::from_utf8_lossy(&out).starts_with("HTTP/1.1 408"));
}

/// Review: shutdown ends a connection in the middle of a stream, not only
/// the listener.
#[tokio::test]
async fn shutdown_ends_a_stream_in_flight() {
    let release = Arc::new(Notify::new());
    let up = stub(Reply::TwoChunks(release.clone())).await;
    let (fwd, _) = forwarder(&up.base, Auth::None).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let mut resp = client()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    let first = resp.chunk().await.unwrap();
    assert!(first.is_some());
    fwd.shutdown();
    let next = tokio::time::timeout(Duration::from_secs(10), resp.chunk())
        .await
        .expect("the stream ended at shutdown");
    assert!(!matches!(next, Ok(Some(_))), "no more data after shutdown");
}

/// Review: a client that leaves during a long wait ends the upstream call.
#[tokio::test]
async fn a_client_that_leaves_ends_the_upstream_wait() {
    let up = stub(Reply::Hang).await;
    let (fwd, lines) = forwarder(&up.base, Auth::None).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let mut stream = TcpStream::connect(("127.0.0.1", fwd.port())).await.unwrap();
    let raw = format!(
        "POST /v1/messages HTTP/1.1\r\nauthorization: Bearer {key}\r\ncontent-length: 2\r\n\r\n{{}}"
    );
    stream.write_all(raw.as_bytes()).await.unwrap();
    for _ in 0..200 {
        if !up.seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    drop(stream);
    assert!(
        wait_for_log(&lines, "the client left").await,
        "{:?}",
        lines.lock().unwrap()
    );
}

/// Review: a redirect is passed back, never followed with the token.
#[tokio::test]
async fn redirects_are_not_followed() {
    let elsewhere = stub(Reply::Whole(200, "{}")).await;
    let up = stub(Reply::Redirect(format!("{}/v1/messages", elsewhere.base))).await;
    let dir = tempfile::tempdir().unwrap();
    let (auth, _) = counting_token(dir.path());
    let (fwd, _) = forwarder(&up.base, auth).await;
    let key = fwd.mint(Duration::from_secs(60)).as_str().to_string();
    let resp = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
        .post(format!("{}/v1/messages", fwd.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status().as_u16(), 302);
    assert!(
        elsewhere.seen.lock().unwrap().is_empty(),
        "the token stayed put"
    );
}

/// Review: an edited entry's old forwarder keeps serving its live keys, and
/// two spellings of one address are one upstream.
#[tokio::test]
async fn ensure_keeps_a_replaced_forwarder_for_its_live_runs() {
    let entry = format!("test-retire-{}", std::process::id());
    let upstream = |base: &str| Upstream {
        entry: entry.clone(),
        base_url: base.into(),
        auth: Auth::None,
    };
    let up = stub(Reply::Whole(200, "{}")).await;
    let a = ensure(upstream(&format!("{}/", up.base))).await.unwrap();
    let same = ensure(upstream(&up.base)).await.unwrap();
    assert_eq!(
        a.port(),
        same.port(),
        "a trailing slash is the same upstream"
    );
    let key = a.mint(Duration::from_secs(60)).as_str().to_string();
    let other = stub(Reply::Whole(200, "{}")).await;
    let b = ensure(upstream(&other.base)).await.unwrap();
    assert_ne!(a.port(), b.port());
    // Q4 review: retiring is not a stop, so the run's calls keep going.
    assert!(!a.is_closed() && !same.is_closed());
    let still = client()
        .post(format!("{}/v1/messages", a.base_url()))
        .bearer_auth(&key)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(
        still.status().as_u16(),
        200,
        "the old forwarder still serves its run"
    );
    stop(&entry);
    assert!(a.is_closed() && b.is_closed(), "a stop closes both");
}

/// Q4 review: a shutdown ends Hollow's own call in flight (a Test's health
/// check waiting on a cold start), and a closed forwarder starts no other
/// call and no token command.
#[tokio::test]
async fn shutdown_ends_hollows_own_get_in_flight() {
    let up = stub(Reply::Hang).await;
    let dir = tempfile::tempdir().unwrap();
    let (auth, counter) = counting_token(dir.path());
    let (fwd, _) = forwarder(&up.base, auth).await;
    let fwd = Arc::new(fwd);
    let pending = {
        let fwd = fwd.clone();
        tokio::spawn(async move { fwd.get("/health", 1024).await })
    };
    for _ in 0..200 {
        if !up.seen.lock().unwrap().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(up.seen.lock().unwrap().len(), 1);
    assert!(!fwd.is_closed());
    fwd.shutdown();
    let got = tokio::time::timeout(Duration::from_secs(10), pending)
        .await
        .expect("the call ended at shutdown")
        .unwrap();
    assert_eq!(got.unwrap_err(), STOPPED);
    assert!(fwd.is_closed() && !fwd.is_serving());
    assert_eq!(fwd.get("/v1/models", 1024).await.unwrap_err(), STOPPED);
    assert_eq!(up.seen.lock().unwrap().len(), 1, "nothing more went up");
    assert_eq!(runs(&counter), 1, "no token command after the stop");
}

/// Q4 review: a stop after the caller's reading (an entry removed or
/// switched off while a Test or run was starting its forwarder) leaves no
/// forwarder registered.
#[tokio::test]
async fn a_stop_after_the_reading_leaves_no_forwarder() {
    let entry = format!("test-stop-race-{}", std::process::id());
    let upstream = Upstream {
        entry: entry.clone(),
        base_url: "http://127.0.0.1:9".into(),
        auth: Auth::None,
    };
    let since = stops(&entry);
    stop(&entry);
    assert_eq!(stops(&entry), since + 1);
    let refused = ensure_since(upstream.clone(), since).await;
    assert_eq!(refused.err().as_deref(), Some(STOPPED));
    assert!(get(&entry).is_none());
    // A reading taken after the stop starts one, and the next stop closes it.
    let fwd = ensure(upstream.clone()).await.unwrap();
    assert!(get(&entry).is_some() && !fwd.is_closed());
    stop(&entry);
    assert!(fwd.is_closed() && get(&entry).is_none());
}

/// Q10: the entry's current forwarder notes when its endpoint last answered
/// 2xx (Hollow's own GET here; a run's call the same way) and wakes the
/// waiter; a failure doesn't count, and a stop or a new address forgets it.
#[tokio::test]
async fn the_current_forwarder_notes_when_its_endpoint_last_answered() {
    let entry = format!("test-answered-{}", std::process::id());
    let ok = stub(Reply::Whole(200, r#"{"ok":true}"#)).await;
    let down = stub(Reply::Whole(503, r#"{"error":"starting"}"#)).await;
    let upstream = |base: &str| Upstream {
        entry: entry.clone(),
        base_url: base.into(),
        auth: Auth::None,
    };
    let fwd = ensure(upstream(&down.base)).await.unwrap();
    assert_eq!(fwd.get("/health", 64).await.unwrap().0, 503);
    assert_eq!(last_answered(&entry), None, "a 503 is not an answer");
    let fwd = ensure(upstream(&ok.base)).await.unwrap();
    let before = SystemTime::now();
    let signal = answered_signal().notified();
    assert_eq!(fwd.get("/health", 64).await.unwrap().0, 200);
    let at = last_answered(&entry).expect("the 200 is noted");
    assert!(at >= before && at <= SystemTime::now());
    tokio::time::timeout(Duration::from_secs(5), signal)
        .await
        .expect("the waiter is woken");
    // The same address again keeps it; another address forgets it.
    ensure(upstream(&ok.base)).await.unwrap();
    assert_eq!(last_answered(&entry), Some(at));
    ensure(upstream(&down.base)).await.unwrap();
    assert_eq!(last_answered(&entry), None);
    set_last_answered(&entry, Some(at));
    stop(&entry);
    assert_eq!(last_answered(&entry), None, "a stop forgets it");
}
