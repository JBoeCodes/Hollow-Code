//! `hollow://updates/connect?token=<token>`: the install page's **Connect
//! Hollow** button (`infra/updates-worker/`). It stores the token in the
//! Keychain, starts one check and opens Settings, Updates.
//!
//! The URL carries a secret, so nothing here logs it, records it in
//! History, or puts it (or any part of it) in an error, an event or the
//! window. Only `token` is read; every other parameter, a fragment, a port
//! or credentials make the link refused.

use super::token;
use std::sync::Mutex;

/// The host of every updates link.
pub(crate) const HOST: &str = "updates";
/// Past this many bytes the link is refused outright.
const MAX_URL_BYTES: usize = 512;

/// Whether `url` is a `hollow://updates…` link (handled here, not by the
/// Linear link handler).
pub(crate) fn is_updates_link(url: &str) -> bool {
    let Some((scheme, rest)) = url.trim().split_once("://") else {
        return false;
    };
    let host_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    scheme.eq_ignore_ascii_case(crate::linear_link::SCHEME)
        && rest[..host_end].eq_ignore_ascii_case(HOST)
}

/// The token of a `hollow://updates/connect?token=…` link. The error is a
/// fixed sentence.
pub(crate) fn parse_connect(url: &str) -> Result<String, &'static str> {
    const REFUSED: &str = "Hollow ignored an updates link it couldn't use. Open the Connect Hollow link from your install page again.";
    if url.len() > MAX_URL_BYTES || !is_updates_link(url) {
        return Err(REFUSED);
    }
    let rest = url.trim().split_once("://").map(|(_, r)| r).unwrap_or("");
    let rest = &rest[HOST.len()..];
    if rest.contains('#') {
        return Err(REFUSED);
    }
    let (path, query) = rest.split_once('?').unwrap_or((rest, ""));
    if path.trim_end_matches('/') != "/connect" {
        return Err(REFUSED);
    }
    let mut found = None;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        match pair.split_once('=') {
            Some(("token", value)) if found.is_none() => found = Some(value),
            _ => return Err(REFUSED),
        }
    }
    // base64url needs no percent-decoding; an escaped token is refused.
    let value = found.ok_or(REFUSED)?;
    token::accept(value).map_err(|_| REFUSED)
}

/// The same link again within this window is one open, not two.
const REPEAT_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

/// The last link's hash (never the link) and when it came.
static LAST: Mutex<Option<([u8; 32], std::time::Instant)>> = Mutex::new(None);

/// A link that arrived before the client existed (memory only).
static EARLY: Mutex<Option<String>> = Mutex::new(None);

pub(crate) fn is_repeat(url: &str) -> bool {
    use sha2::Digest;
    let hash: [u8; 32] = sha2::Sha256::digest(url.as_bytes()).into();
    let now = std::time::Instant::now();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    let repeat = last
        .as_ref()
        .is_some_and(|(h, at)| *h == hash && now.duration_since(*at) < REPEAT_WINDOW);
    *last = Some((hash, now));
    repeat
}

/// The token of a link that came before the client existed.
pub(crate) fn take_early() -> Option<String> {
    EARLY.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// A Connect Hollow link: off the event loop, never logged. A refused link
/// leaves Hollow as it was (hidden stays hidden) and only sets a note. A
/// good one shows the window; its token is stored when none is, else held
/// for the person's confirmation; then Settings, Updates opens.
pub(crate) fn handle(app: &tauri::AppHandle, url: &str) {
    if is_repeat(url) {
        return;
    }
    let parsed = parse_connect(url);
    let app = app.clone();
    std::thread::spawn(move || {
        let token = match parsed {
            Ok(token) => token,
            Err(words) => {
                if let Some(client) = super::client() {
                    client.set_note(words.to_string());
                }
                return;
            }
        };
        show_window(&app);
        let Some(client) = super::client() else {
            *EARLY.lock().unwrap_or_else(|e| e.into_inner()) = Some(token);
            return;
        };
        match client.connect_from_link(token) {
            Ok(super::client::LinkConnect::Stored) => super::kick(&client),
            Ok(_) => {}
            Err(_) => client
                .set_note("Hollow couldn't save the updates token in the Keychain.".to_string()),
        }
        open_settings(&app);
    });
}

fn show_window(app: &tauri::AppHandle) {
    crate::lifecycle::bring_main_forward(app);
}

/// Settings, Updates, once the window answers (a cold launch takes a few
/// seconds): up to 20 tries a second apart. Settings, Updates is row U5's
/// second PR; until then this opens Settings.
fn open_settings(app: &tauri::AppHandle) {
    let args = serde_json::json!({ "page": "settings", "record": "updates" });
    for _ in 0..20 {
        if crate::commands::app_bridge_api::window_open(app)
            && crate::commands::app_bridge_api::request(app, "navigate", args.clone()).is_ok()
        {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
