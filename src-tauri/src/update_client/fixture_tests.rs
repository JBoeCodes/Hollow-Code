//! The real updater plugin against a local HTTP server: a channel manifest
//! and a tiny `Hollow.app.tar.gz` signed here with a throwaway minisign key
//! (Ed25519 through `ring`, in minisign's legacy `Ed` format, which the
//! plugin's `minisign-verify` accepts). No network beyond 127.0.0.1, no CLI.

use super::client_tests::{app_archive, ready_version, rig, Setup, TOKEN};
use super::fetch::{CheckError, Fetcher, PluginFetcher};
use super::state::UpdatePhase;
use base64::Engine as _;
use reqwest::Url;
use ring::signature::{Ed25519KeyPair, KeyPair};
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;
const KEY_ID: [u8; 8] = [1, 2, 3, 4, 5, 6, 7, 8];

struct Key(Ed25519KeyPair);

impl Key {
    fn new(seed: u8) -> Self {
        Self(Ed25519KeyPair::from_seed_unchecked(&[seed; 32]).unwrap())
    }

    /// What `plugins.updater.pubkey` holds: base64 of the public key file.
    fn pubkey(&self) -> String {
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&KEY_ID);
        raw.extend_from_slice(self.0.public_key().as_ref());
        B64.encode(format!(
            "untrusted comment: minisign public key 0807060504030201\n{}\n",
            B64.encode(raw)
        ))
    }

    /// What the manifest's `signature` holds: base64 of the `.sig` file.
    fn sign(&self, data: &[u8]) -> String {
        let sig = self.0.sign(data);
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&KEY_ID);
        raw.extend_from_slice(sig.as_ref());
        let trusted = "timestamp:1763208000\tfile:Hollow.app.tar.gz";
        let mut global = sig.as_ref().to_vec();
        global.extend_from_slice(trusted.as_bytes());
        let global = self.0.sign(&global);
        B64.encode(format!(
            "untrusted comment: signature from tauri secret key\n{}\ntrusted comment: {trusted}\n{}\n",
            B64.encode(raw),
            B64.encode(global.as_ref())
        ))
    }
}

/// One served file: path, then body (status 200).
type Routes = Vec<(String, Vec<u8>)>;

struct Server {
    origin: String,
    seen: Arc<Mutex<Vec<(String, Option<String>)>>>,
}

impl Server {
    /// Serves `routes` to requests that carry `Authorization: Bearer TOKEN`
    /// (else 401); anything else is 404. Each answer closes its connection.
    fn start(routes: impl Fn(&str) -> Routes + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (log, base) = (seen.clone(), origin.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                if !answer(stream, &routes(&base), &log) {
                    return;
                }
            }
        });
        Self { origin, seen }
    }

    fn stop(&self) {
        let mut s = TcpStream::connect(self.origin.trim_start_matches("http://")).unwrap();
        let _ = s.write_all(b"GET /__stop HTTP/1.1\r\n\r\n");
    }
}

/// `false` to stop serving.
fn answer(stream: TcpStream, routes: &Routes, log: &Mutex<Vec<(String, Option<String>)>>) -> bool {
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap_or(0);
    let path = line.split_whitespace().nth(1).unwrap_or("").to_string();
    let mut auth = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header == "\r\n" {
            break;
        }
        if let Some((k, v)) = header.split_once(':') {
            if k.eq_ignore_ascii_case("authorization") {
                auth = Some(v.trim().to_string());
            }
        }
    }
    if path == "/__stop" {
        return false;
    }
    log.lock().unwrap().push((path.clone(), auth.clone()));
    let (status, body) = if auth.as_deref() != Some(&format!("Bearer {TOKEN}")) {
        ("401 Unauthorized", Vec::new())
    } else {
        match routes.iter().find(|(p, _)| *p == path) {
            Some((_, body)) => ("200 OK", body.clone()),
            None => ("404 Not Found", Vec::new()),
        }
    };
    let mut out = stream;
    // Two test bodies misbehave on purpose: `!BIG` announces 500 MB and
    // sends 4 bytes, `!STALL` sends one byte of 100; both then send nothing
    // more (the connection is kept open, never closed).
    let (length, body, stall) = match body.as_slice() {
        b"!BIG" => (500_000_000usize, b"xxxx".to_vec(), true),
        b"!STALL" => (100, b"x".to_vec(), true),
        _ => (body.len(), body, false),
    };
    let _ = write!(
        out,
        "HTTP/1.1 {status}\r\nContent-Length: {length}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n"
    );
    let _ = out.write_all(&body);
    let _ = out.flush();
    if stall {
        STALLED.lock().unwrap().push(out);
    }
    true
}

static STALLED: Mutex<Vec<TcpStream>> = Mutex::new(Vec::new());

/// A mock app with the plugin registered as `lib.rs` does (empty pubkey in
/// the config, the real one given at runtime).
fn app() -> tauri::App<tauri::test::MockRuntime> {
    let mut ctx = tauri::test::mock_context(tauri::test::noop_assets());
    ctx.config_mut()
        .plugins
        .0
        .insert("updater".into(), serde_json::json!({ "pubkey": "" }));
    tauri::test::mock_builder()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .build(ctx)
        .unwrap()
}

fn manifest(origin: &str, version: &str, signature: &str) -> Vec<u8> {
    serde_json::json!({
        "version": version,
        "notes": "Signed here",
        "pub_date": "2026-11-15T12:00:00Z",
        "platforms": { "darwin-aarch64": {
            "signature": signature,
            "url": format!("{origin}/builds/{version}/Hollow.app.tar.gz"),
        }},
        "channel": "nightly",
    })
    .to_string()
    .into_bytes()
}

/// A nightly channel serving `served` as 0.6.11's archive, signed over
/// `signed`.
fn channel(key: &Key, signed: Vec<u8>, served: Vec<u8>) -> Server {
    let signature = key.sign(&signed);
    Server::start(move |origin| {
        vec![
            (
                "/channels/nightly.json".into(),
                manifest(origin, "0.6.11", &signature),
            ),
            ("/builds/0.6.11/Hollow.app.tar.gz".into(), served.clone()),
        ]
    })
}

fn endpoint(server: &Server, channel: &str) -> Url {
    Url::parse(&format!("{}/channels/{channel}.json", server.origin)).unwrap()
}

#[test]
fn the_plugin_checks_downloads_and_verifies_a_real_minisign_signature() {
    let key = Key::new(7);
    let archive = app_archive("0.6.11");
    let server = channel(&key, archive.clone(), archive.clone());
    let app = app();
    let fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    let pending = fetcher
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .expect("an update");
    assert_eq!(pending.manifest()["version"], "0.6.11");
    assert_eq!(
        pending.download_url().as_str(),
        format!("{}/builds/0.6.11/Hollow.app.tar.gz", server.origin)
    );
    let mut seen = Vec::new();
    let bytes = pending
        .download(&mut |got, total| {
            seen.push((got, total));
            true
        })
        .unwrap();
    assert_eq!(bytes, archive);
    assert_eq!(seen.last().map(|(got, _)| *got), Some(archive.len() as u64));
    // Both requests carried the bearer token, and only to this server.
    let log = server.seen.lock().unwrap().clone();
    assert_eq!(
        log.iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(),
        ["/channels/nightly.json", "/builds/0.6.11/Hollow.app.tar.gz"]
    );
    assert!(log
        .iter()
        .all(|(_, auth)| auth.as_deref() == Some(&format!("Bearer {TOKEN}"))));
    server.stop();
}

#[test]
fn a_tampered_archive_or_another_key_fails_the_signature_check() {
    let key = Key::new(7);
    let archive = app_archive("0.6.11");
    let mut tampered = archive.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let server = channel(&key, archive.clone(), tampered);
    let app = app();
    let fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    let pending = fetcher
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .unwrap();
    let err = pending.download(&mut |_, _| true).unwrap_err();
    assert!(err.contains("signature doesn't match"), "{err}");
    server.stop();
    // The right bytes, but this build trusts another key.
    let server = channel(&key, archive.clone(), archive);
    let other = PluginFetcher::new(app.handle().clone(), Key::new(9).pubkey());
    let pending = other
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .unwrap();
    assert!(pending
        .download(&mut |_, _| true)
        .unwrap_err()
        .contains("signature doesn't match"));
    server.stop();
}

#[test]
fn a_refused_token_and_an_empty_channel_are_told_apart() {
    let key = Key::new(7);
    let archive = app_archive("0.6.11");
    let server = channel(&key, archive.clone(), archive);
    let app = app();
    let fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    let wrong = "W".repeat(43);
    assert_eq!(
        fetcher.check(&endpoint(&server, "nightly"), &wrong).err(),
        Some(CheckError::Unauthorized)
    );
    assert_eq!(
        fetcher.check(&endpoint(&server, "main"), TOKEN).err(),
        Some(CheckError::NothingPublished)
    );
    server.stop();
}

#[test]
fn the_client_reaches_ready_from_the_local_server() {
    let key = Key::new(7);
    let archive = app_archive("0.6.11");
    let server = channel(&key, archive.clone(), archive);
    let app = app();
    let r = rig(Setup {
        host: Some(server.origin.clone()),
        fetcher: Some(Box::new(PluginFetcher::new(
            app.handle().clone(),
            key.pubkey(),
        ))),
        ..Setup::default()
    });
    r.client
        .set_channel(super::state::Channel::Nightly)
        .unwrap();
    r.client.check();
    assert_eq!(ready_version(&r.phase_now()).as_deref(), Some("0.6.11"));
    assert!(matches!(r.phase_now(), UpdatePhase::Ready { .. }));
    server.stop();
}

#[test]
fn a_download_past_the_size_cap_or_the_time_limit_is_stopped() {
    let key = Key::new(7);
    let app = app();
    // Announces 500 MB: stopped at the first chunk.
    let server = channel(&key, b"!BIG".to_vec(), b"!BIG".to_vec());
    let fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    let pending = fetcher
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .unwrap();
    let err = pending.download(&mut |_, _| true).unwrap_err();
    assert!(err.contains("larger than 400 MB"), "{err}");
    server.stop();
    // Sends one byte and stalls: the whole-download limit ends it (a short
    // one here; the app's is 25 minutes).
    let server = channel(&key, b"!STALL".to_vec(), b"!STALL".to_vec());
    let mut fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    fetcher.download_timeout = std::time::Duration::from_millis(300);
    let pending = fetcher
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .unwrap();
    let err = pending.download(&mut |_, _| true).unwrap_err();
    assert!(err.contains("didn't finish downloading"), "{err}");
    server.stop();
    assert_eq!(super::fetch::DOWNLOAD_TIMEOUT.as_secs(), 25 * 60);
    assert_eq!(super::fetch::MAX_DOWNLOAD_BYTES, 400 * 1024 * 1024);
}

#[test]
fn the_client_can_stop_a_download_it_no_longer_wants() {
    let key = Key::new(7);
    let app = app();
    // A stalled download (one byte of 100): the client's `false` ends it at
    // the first chunk instead of after the whole-download limit.
    let server = channel(&key, b"!STALL".to_vec(), b"!STALL".to_vec());
    let fetcher = PluginFetcher::new(app.handle().clone(), key.pubkey());
    let pending = fetcher
        .check(&endpoint(&server, "nightly"), TOKEN)
        .unwrap()
        .unwrap();
    let err = pending.download(&mut |_, _| false).unwrap_err();
    assert_eq!(err, super::fetch::STOPPED);
    server.stop();
}
