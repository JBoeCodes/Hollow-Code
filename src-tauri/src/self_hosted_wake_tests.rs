//! Tests for `self_hosted_wake` (attached as `#[path] mod tests`, so
//! `super::` is the wake): the throttle and join, resolution, and wakes
//! against a stub endpoint on loopback, in the style of
//! `self_hosted_registry_tests.rs`. No call leaves this Mac.
use super::*;
use crate::self_hosted_registry::{preset, SelfHostedAuth};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// A forwarder entry id unique to this test process and `name`.
fn unique(name: &str) -> String {
    format!("w{}-{name}", std::process::id())
}

fn answered(tx: &watch::Sender<Option<SelfHostedWake>>) {
    tx.send_replace(Some(SelfHostedWake {
        status: Some(200),
        ms: 40,
        ..SelfHostedWake::default()
    }));
}

fn leads(turn: &Turn) -> bool {
    matches!(turn, Turn::Lead(_))
}

/// Q6's verify: one wake per entry per 2 minutes, and a wake that is still
/// running is joined however long it takes.
#[test]
fn a_running_wake_is_joined_and_a_new_one_waits_two_minutes() {
    let mut map = HashMap::new();
    let t0 = Instant::now();
    let Turn::Lead(first) = take_turn(&mut map, "qwen", t0, WAKE_EVERY) else {
        panic!("the first call leads");
    };
    // Still running: joined, a second later and five minutes later.
    assert!(!leads(&take_turn(
        &mut map,
        "qwen",
        t0 + Duration::from_secs(1),
        WAKE_EVERY
    )));
    assert!(!leads(&take_turn(
        &mut map,
        "qwen",
        t0 + Duration::from_secs(5 * 60),
        WAKE_EVERY
    )));
    // Another entry has its own wake.
    assert!(leads(&take_turn(&mut map, "other", t0, WAKE_EVERY)));
    // Answered in under 2 minutes: shared until 2 minutes after its start.
    answered(&first);
    assert!(!leads(&take_turn(
        &mut map,
        "qwen",
        t0 + Duration::from_secs(119),
        WAKE_EVERY
    )));
    let Turn::Lead(second) = take_turn(&mut map, "qwen", t0 + WAKE_EVERY, WAKE_EVERY) else {
        panic!("2 minutes after the last start, a call leads again");
    };
    // The new wake is the one joined now, with its own start.
    let Turn::Join(joined, started) = take_turn(
        &mut map,
        "qwen",
        t0 + WAKE_EVERY + Duration::from_secs(9),
        WAKE_EVERY,
    ) else {
        panic!("joined");
    };
    assert_eq!(started, t0 + WAKE_EVERY);
    assert!(joined.borrow().is_none(), "the new wake is still running");
    // Status reads its age and no answer yet, without sending anything.
    let running = status_in(&map, "qwen", t0 + WAKE_EVERY + Duration::from_secs(5)).unwrap();
    assert_eq!(running.started_ms_ago, 5_000);
    assert_eq!(running.result, None);
    // A wake whose task ended without an answer is never joined or reported.
    drop(second);
    drop(joined);
    assert_eq!(status_in(&map, "qwen", t0 + WAKE_EVERY), None);
    assert!(leads(&take_turn(
        &mut map,
        "qwen",
        t0 + WAKE_EVERY + Duration::from_secs(1),
        WAKE_EVERY
    )));
    assert_eq!(status_in(&map, "never", t0), None);
}

/// Q6 review: an answered wake reports its age and answer; Settings'
/// save, switch and remove forget an entry's last wake.
#[test]
fn status_reads_the_last_wake_and_settings_changes_forget_it() {
    let mut map = HashMap::new();
    let t0 = Instant::now();
    let Turn::Lead(tx) = take_turn(&mut map, "qwen", t0, WAKE_EVERY) else {
        panic!("leads");
    };
    answered(&tx);
    let done = status_in(&map, "qwen", t0 + Duration::from_secs(90)).unwrap();
    assert_eq!(done.started_ms_ago, 90_000);
    assert_eq!(done.result.map(|r| r.status), Some(Some(200)));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app-data").join(registry::FILE);
    let id = unique("forget");
    let wake = |id: &str| {
        let mut map = slots().lock().unwrap();
        let Turn::Lead(tx) = take_turn(&mut map, id, Instant::now(), WAKE_EVERY) else {
            panic!("leads");
        };
        answered(&tx);
    };
    let e = SelfHostedEntry {
        id: id.clone(),
        ..preset()
    };
    wake(&id);
    assert!(status(&id).is_some());
    registry::save_at(&path, e.clone(), false).unwrap();
    assert_eq!(status(&id), None, "saved: forgotten");
    wake(&id);
    registry::set_enabled_at(&path, &id, true).unwrap();
    assert_eq!(status(&id), None, "switched: forgotten");
    wake(&id);
    registry::remove_at(&path, &id).unwrap();
    assert_eq!(status(&id), None, "removed: forgotten");
}

#[test]
fn resolution_names_settings_for_a_missing_switched_off_or_broken_entry() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app-data").join(registry::FILE);
    let missing = resolve_in(registry::list_at(&path), "qwen").unwrap_err();
    assert_eq!(
        missing,
        "There is no self-hosted model \"qwen\". Add it in Settings → Models → Self-hosted models."
    );
    registry::save_at(&path, preset(), false).unwrap();
    let entry = resolve_in(registry::list_at(&path), "qwen").unwrap();
    assert_eq!(entry, preset());
    registry::set_enabled_at(&path, "qwen", false).unwrap();
    assert_eq!(
        resolve_in(registry::list_at(&path), "qwen").unwrap_err(),
        "Qwen3.8 27B is switched off. Switch it on in Settings → Models → Self-hosted models."
    );
    // A file Hollow can't read names itself and where to fix it.
    std::fs::write(&path, b"{not json").unwrap();
    let broken = resolve_in(registry::list_at(&path), "qwen").unwrap_err();
    assert!(
        broken.contains("isn't valid JSON")
            && broken.ends_with("(Settings → Models → Self-hosted models)"),
        "{broken}"
    );
    // An entry edited by hand into something invalid isn't woken.
    let mut bad = preset();
    bad.health_path = "health".into();
    let view = SelfHostedModels {
        entries: vec![bad],
        ..registry::list_at(&dir.path().join("none.json"))
    };
    let why = resolve_in(view, "qwen").unwrap_err();
    assert!(
        why.starts_with(
            "Qwen3.8 27B can't start until it is fixed in Settings → Models → Self-hosted models: "
        ),
        "{why}"
    );
}

// ── Against a stub endpoint ─────────────────────────────────────────────

#[derive(Clone, Debug)]
struct Seen {
    path: String,
    headers: Vec<(String, String)>,
}

struct Stub {
    base: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    /// Hold `/health` answers until this is released (a cold start).
    gate: Arc<tokio::sync::Notify>,
}

impl Stub {
    fn health_calls(&self) -> usize {
        self.seen
            .lock()
            .unwrap()
            .iter()
            .filter(|s| s.path == "/health")
            .count()
    }
}

/// A stub endpoint answering `/health` with `status`. With `held`, each
/// health answer waits for `gate` (released with `notify_waiters`); with
/// `status` 0 it never answers.
async fn stub(status: u16, held: bool) -> Stub {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let gate = Arc::new(tokio::sync::Notify::new());
    let (log, wait) = (seen.clone(), gate.clone());
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let (log, wait) = (log.clone(), wait.clone());
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
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
                let headers = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_string()))
                    .collect();
                // Registered before the request is logged, so a test that
                // sees the request can release it.
                let released = wait.notified();
                log.lock().unwrap().push(Seen {
                    path: path.clone(),
                    headers,
                });
                if status == 0 {
                    std::future::pending::<()>().await;
                }
                if held {
                    released.await;
                }
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{{}}"
                );
                let _ = stream.write_all(reply.as_bytes()).await;
                let _ = stream.flush().await;
            });
        }
    });
    Stub { base, seen, gate }
}

/// An entry on `stub` whose token command prints `tok-wake`.
fn stub_entry(id: &str, stub: &Stub) -> SelfHostedEntry {
    SelfHostedEntry {
        id: id.into(),
        base_url: stub.base.clone(),
        auth: SelfHostedAuth::TokenCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "echo tok-wake".into()],
        },
        ..preset()
    }
}

/// Q6's verify: three chats waking one cold model at once send one health
/// request; a fourth within 2 minutes shares its answer; only past the
/// throttle does another go out.
#[tokio::test]
async fn calls_during_a_cold_start_join_it_and_send_nothing() {
    let up = stub(200, true).await;
    let id = unique("join");
    let e = stub_entry(&id, &up);
    let three = tokio::spawn({
        let e = e.clone();
        async move {
            tokio::join!(
                wake_entry(e.clone(), WAKE_EVERY, WAKE_TIMEOUT),
                wake_entry(e.clone(), WAKE_EVERY, WAKE_TIMEOUT),
                wake_entry(e, WAKE_EVERY, WAKE_TIMEOUT),
            )
        }
    });
    // The endpoint is "starting": hold its answer until the request is in.
    while up.health_calls() == 0 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    up.gate.notify_waiters();
    let (a, b, c) = three.await.unwrap();
    for r in [&a, &b, &c] {
        assert_eq!(r.status, Some(200), "{r:?}");
        assert!(r.ok(), "{r:?}");
    }
    assert_eq!(
        [a.shared, b.shared, c.shared]
            .iter()
            .filter(|s| !**s)
            .count(),
        1,
        "exactly one call sent the wake"
    );
    assert_eq!(up.health_calls(), 1);
    // Joined calls get the wake's own answer and time.
    assert!(a.ms == b.ms && b.ms == c.ms, "{a:?} {b:?} {c:?}");
    // Under 2 minutes later: the same answer, nothing sent, and its age
    // counts from the wake's start, not this call's.
    let again = wake_entry(e.clone(), WAKE_EVERY, WAKE_TIMEOUT).await;
    assert!(again.shared && again.ok(), "{again:?}");
    assert_eq!(again.ms, a.ms);
    assert!(again.started_ms_ago >= again.ms, "{again:?}");
    assert_eq!(up.health_calls(), 1);
    // Settings changed the entry: the next call wakes it again at once.
    forget(&id);
    assert_eq!(status(&id), None);
    let held = tokio::spawn(wake_entry(e.clone(), WAKE_EVERY, WAKE_TIMEOUT));
    while up.health_calls() < 2 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    up.gate.notify_waiters();
    let after = held.await.unwrap();
    assert!(!after.shared && after.ok(), "{after:?}");
    assert_eq!(up.health_calls(), 2);
    // Past the throttle (zero here), a new wake goes out.
    let held = tokio::spawn(wake_entry(e, Duration::ZERO, WAKE_TIMEOUT));
    while up.health_calls() < 3 {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    up.gate.notify_waiters();
    let fresh = held.await.unwrap();
    assert!(!fresh.shared && fresh.ok(), "{fresh:?}");
    assert_eq!(up.health_calls(), 3);
    forwarder::stop(&id);
}

/// The endpoint gets its own token through the forwarder; the result and
/// the endpoint never see a run key, and the result holds no token.
#[tokio::test]
async fn a_wake_sends_the_endpoint_token_and_returns_no_secret() {
    let up = stub(200, false).await;
    let id = unique("secret");
    let result = wake_entry(stub_entry(&id, &up), WAKE_EVERY, WAKE_TIMEOUT).await;
    assert!(result.ok(), "{result:?}");
    assert!(!result.shared);
    let seen = up.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].path, "/health");
    let auth = seen[0].headers.iter().find(|(n, _)| n == "authorization");
    assert_eq!(auth.map(|(_, v)| v.as_str()), Some("Bearer tok-wake"));
    assert!(
        !seen[0].headers.iter().any(|(_, v)| v.contains("hsr_")),
        "no run key goes to the endpoint: {:?}",
        seen[0].headers
    );
    // A wake mints no run key.
    assert_eq!(forwarder::get(&id).unwrap().live_keys(), 0);
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(
        wire,
        serde_json::json!({"status": 200, "ms": result.ms, "error": null, "shared": false, "startedMsAgo": result.started_ms_ago})
    );
    let text = wire.to_string();
    assert!(
        !text.contains("tok-wake") && !text.contains("hsr_"),
        "{text}"
    );
    forwarder::stop(&id);
}

#[tokio::test]
async fn a_failing_health_check_is_an_error_in_plain_words() {
    let up = stub(503, false).await;
    let id = unique("503");
    let result = wake_entry(stub_entry(&id, &up), WAKE_EVERY, WAKE_TIMEOUT).await;
    assert!(!result.ok());
    assert_eq!(result.status, Some(503));
    assert_eq!(
        result.error.as_deref(),
        Some("The health check (/health) answered 503.")
    );
    // A failed wake is throttled too: nothing loops on an error.
    let again = wake_entry(stub_entry(&id, &up), WAKE_EVERY, WAKE_TIMEOUT).await;
    assert!(again.shared);
    assert_eq!(again.error, result.error);
    assert_eq!(up.health_calls(), 1);
    forwarder::stop(&id);
}

#[tokio::test]
async fn an_unreachable_endpoint_or_one_that_never_answers_is_an_error_not_a_hang() {
    // Port 9 (discard) has nothing listening on loopback.
    let id = unique("down");
    let e = SelfHostedEntry {
        id: id.clone(),
        base_url: "http://127.0.0.1:9".into(),
        auth: SelfHostedAuth::None,
        ..preset()
    };
    let result = wake_entry(e, WAKE_EVERY, WAKE_TIMEOUT).await;
    assert_eq!(result.status, None);
    let why = result.error.unwrap_or_default();
    assert!(why.contains("couldn't be reached"), "{why}");
    forwarder::stop(&id);

    let up = stub(0, false).await;
    let id = unique("silent");
    let result = wake_once(&stub_entry(&id, &up), Duration::from_millis(300)).await;
    assert_eq!(result.status, None);
    assert_eq!(
        result.error.as_deref(),
        Some("The model didn't answer within 300 ms.")
    );
    forwarder::stop(&id);
}
