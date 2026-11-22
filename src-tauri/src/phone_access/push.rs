//! v1.1 push notifications (2026-11-05): Hollow sends straight to Apple's
//! push service (APNs), with no server in between.
//!
//! - **The key.** A `.p8` key from Alex's Apple developer account (team
//!   [`TEAM_ID`]), with its Key ID, imported by the person's own click in
//!   Settings into Hollow's one Keychain item (`secret_store`, key
//!   [`SECRET_KEY`]). Never the vault, a log or a command line.
//! - **Token auth.** An ES256 JWT (`ring`, P-256 with SHA-256) signed with
//!   that key, kept in memory and made again once it is [`JWT_FOR`] old
//!   (Apple takes one for an hour and refuses a new one more than every 20
//!   minutes). Its age is the wall clock's ([`Clock`]) since its `iat`, the
//!   age Apple checks, never an `Instant`'s, which on macOS stops while the
//!   Mac sleeps (review medium 2, 2026-11-05). When Apple says it expired
//!   anyway (403 `ExpiredProviderToken`), a fresh one is signed and that
//!   push sent once more.
//! - **The transport** is the system's `curl` over HTTP/2 (`--http2`, ALPN
//!   on TLS) to `api.push.apple.com`, or `api.sandbox.push.apple.com` for a
//!   phone's `development` token. Its URL, headers and body go on its
//!   standard input (`--config -`), so neither the JWT nor a device token is
//!   ever in an argument list; `-q` keeps the person's `.curlrc` out. A
//!   push is the occasional message, so a connection per push is fine.
//! - **When.** The live worker's observer ([`Pusher::observe`]) sees every
//!   chat event folded while a phone is paired: a turn whose `runStarted`
//!   it saw says `attended` (the person sent it, `chat::turn_attended`) ends
//!   (`done`) → `turnFinished`. Nothing for any other turn: a night item, a
//!   Main agent wake, check-in or swarm wake, an agent's chat (review
//!   medium 1, 2026-11-05: wakes say `unattended: false` too). A newly
//!   recorded pending card (`interaction`) → `question`, whatever started
//!   its turn: a card waits on the person, and the phone's Focus settings
//!   keep quiet hours. Each becomes a job for one task, which reads the
//!   chat and sends to every phone that asked for that kind and has no
//!   event stream open for that chat. A turn that opened a card is told as
//!   its `question` push alone to a phone that asked for both.
//! - **Failures** are logged (never with a token) and kept for Settings'
//!   status; nothing waits on a push, and none is sent again but the one
//!   whose JWT Apple called expired. A token Apple calls invalid (410, or
//!   400 `BadDeviceToken` / `DeviceTokenNotForTopic`) is dropped from its
//!   phone's record.
//! - **Buttons (v1.2, 2026-11-07).** What a push says gains what the app
//!   needs to put buttons on it (`docs/phone-api/notification-actions.md`):
//!   `turnFinished` names the category [`CATEGORY_TURN`]; a `question` for a
//!   card with one single-choice question of 2 to 4 options
//!   ([`card_actions`]) names [`CATEGORY_QUESTION`], is `mutable-content` and
//!   carries the options for the app's notification service extension to
//!   turn into buttons. Any other push is as in v1.1.
//! - **Live Activities (v1.2, 2026-11-07).** For a chat whose Live Activity
//!   token a phone registered (`POST /v1/live-activity`, kept in
//!   [`Activities`]), the observer also makes [`Job::LiveStep`] (a tool or
//!   the answer's text changed), [`Job::LiveWaiting`] (a card opened) and
//!   [`Job::LiveEnd`] (the turn's `done`, with how it ended): pushes with
//!   `apns-push-type: liveactivity` to topic [`LIVE_ACTIVITY_TOPIC`], whose
//!   body is only a phase and a step label from a tool's name
//!   (`live_activity.rs`). A phone's activity gets at most one `update` per
//!   [`LIVE_GAP`] seconds and the newest step wins; `waiting` and `end` go at
//!   once. A step that doesn't change is sent again every [`LIVE_REFRESH`]
//!   seconds ([`Job::LiveRefresh`]) so a healthy long step never reaches its
//!   `stale-date`. How a turn ended is what Hollow stored for it, not what
//!   events hinted. An end Apple didn't take (no answer, a 5xx or 429, a
//!   refused JWT) keeps its tokens and is sent again [`LIVE_END_TRIES`] more
//!   times, [`LIVE_END_WAIT`] seconds apart. An end is only for the activities
//!   registered when its turn ended ([`Activities::mark`]), so the chat's next
//!   turn's activity is never told the previous turn is over. Two pushes of
//!   one chat in the same second go a second apart, so ActivityKit (which
//!   drops a push not newer than the last) takes both. No key imported:
//!   nothing is sent and nothing is wrong.

use super::live::Live;
use super::live_activity::{self, Activities, Phase};
use super::store::{Device, Store};
use super::wire;
use base64::{engine::general_purpose, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(crate) const PRODUCTION: &str = "production";
pub(crate) const DEVELOPMENT: &str = "development";
pub(crate) const TURN_FINISHED: &str = "turnFinished";
pub(crate) const QUESTION: &str = "question";
/// B4: an approval card is waiting; sent only to a phone with approval keys.
pub(crate) const APPROVAL: &str = "approval";
/// Settings' Send test notification.
pub(crate) const TEST: &str = "test";
/// The pushes a phone may ask for, in the order the record keeps them.
pub(crate) const EVENTS: [&str; 3] = [TURN_FINISHED, QUESTION, APPROVAL];
/// The iPhone app's bundle id.
pub(crate) const TOPIC: &str = "com.hollow.app.companion";
/// v1.2 (`docs/phone-api/live-activity.md`): the topic of a Live Activity
/// push, the bundle id with Apple's suffix.
pub(crate) const LIVE_ACTIVITY_TOPIC: &str = "com.hollow.app.companion.push-type.liveactivity";
/// A phone's Live Activity gets at most one step `update` this often
/// (seconds); the newest step wins.
pub(crate) const LIVE_GAP: u64 = 10;
/// A running or waiting Live Activity that nothing changed for is told again
/// this often (seconds), half of its `stale-date` ([`live_activity::STALE_IN`]),
/// so the lock screen never reads "Open Hollow to refresh" under a step that
/// is simply long.
pub(crate) const LIVE_REFRESH: u64 = 300;
/// An end push Apple didn't take is sent again this many more times, so a
/// closed app's lock screen doesn't keep a running turn that ended.
pub(crate) const LIVE_END_TRIES: u8 = 2;
/// Seconds before the first re-send of an end; the second waits twice as long.
pub(crate) const LIVE_END_WAIT: u64 = 20;
/// A failing key read is noted for Settings at most this often (seconds): a
/// streamed answer would make one error per delta.
const LIVE_ERROR_EVERY: i64 = 60;
/// Chats whose tool calls are followed for a Live Activity, at most.
const LIVE_TOOL_CHATS: usize = 64;

/// How Apple is told to treat a push: its type, topic and priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PushClass {
    pub push_type: &'static str,
    pub topic: &'static str,
    pub priority: &'static str,
}

/// A notification (v1.1): shown, with a sound.
pub(crate) const ALERT: PushClass = PushClass {
    push_type: "alert",
    topic: TOPIC,
    priority: "10",
};
/// A Live Activity's step `update`: low priority, which Apple doesn't budget.
pub(crate) const LIVE_UPDATE: PushClass = PushClass {
    push_type: "liveactivity",
    topic: LIVE_ACTIVITY_TOPIC,
    priority: "5",
};
/// A Live Activity's `waiting` and `end`: at once.
pub(crate) const LIVE_URGENT: PushClass = PushClass {
    push_type: "liveactivity",
    topic: LIVE_ACTIVITY_TOPIC,
    priority: "10",
};
/// Alex's Apple developer team: the default for a key's team.
pub(crate) const TEAM_ID: &str = "TEAM000000";
/// The key's entry in Hollow's Keychain item (`secret_store`).
pub(crate) const SECRET_KEY: &str = "apns";
/// A JWT is made again once it is this old by the wall clock (Apple: at
/// most an hour old, and no new one more often than every 20 minutes).
pub(crate) const JWT_FOR: Duration = Duration::from_secs(50 * 60);
/// After Apple calls a JWT `InvalidProviderToken` (a wrong key, key id or
/// team id: a newer JWT from the same key fails the same way), no new JWT is
/// signed for this long, by the wall clock. Apple answers a provider that
/// makes JWTs faster than once in 20 minutes with 429
/// `TooManyProviderTokenUpdates`, and signing again for every push got there
/// (ENG-223). Importing or removing the key ends the wait.
pub(crate) const JWT_INVALID_WAIT: Duration = Duration::from_secs(20 * 60);
pub(crate) const PRODUCTION_URL: &str = "https://api.push.apple.com";
pub(crate) const SANDBOX_URL: &str = "https://api.sandbox.push.apple.com";
/// A push's title, at most this many characters.
pub(crate) const TITLE_MAX: usize = 80;
/// v1.2 (`docs/phone-api/notification-actions.md`): the notification
/// category of a `turnFinished` push (Reply, Open) and of a `question` push
/// that carries a [`wire::PushCard`] for buttons.
pub(crate) const CATEGORY_TURN: &str = "hollow.turn";
pub(crate) const CATEGORY_QUESTION: &str = "hollow.question";
/// A card gets one button per option when its one question has this many
/// options (iOS shows four actions on a banner).
pub(crate) const ACTION_OPTIONS: std::ops::RangeInclusive<usize> = 2..=4;
/// A button's label, at most this many characters (a longer one is cut with
/// an ellipsis).
pub(crate) const ACTION_LABEL_MAX: usize = 40;
/// An option id longer than this (Hollow's own cap is 40) keeps a card from
/// getting buttons.
const ACTION_ID_MAX: usize = 40;
/// One delivery, at most (curl's `--max-time`).
const SEND_SECONDS: u32 = 20;
/// Cards whose push was decided, remembered at most (oldest go first).
const SEEN_CARDS: usize = 500;
/// A card recorded longer ago than this is one coming back (an answer that
/// never went), not a new question: no push.
const NEW_CARD_FOR: chrono::Duration = chrono::Duration::minutes(10);
/// An error kept for Settings, at most this many characters.
const ERROR_MAX: usize = 300;

/// The wall clock, in Unix seconds: what a JWT's `iat` says and its age is
/// measured with. Apple checks that age by its own wall clock, and an
/// `Instant` on macOS doesn't count the time the Mac sleeps, so a JWT kept
/// by an `Instant` outlived its hour across a night's sleep (review medium
/// 2, 2026-11-05). [`wall_clock`] in the app; tests move their own.
pub(crate) type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The system's wall clock.
pub(crate) fn wall_clock() -> Clock {
    Arc::new(|| chrono::Utc::now().timestamp())
}

/// An APNs device token: hex, an even 32 to 400 characters (Apple's are 64
/// today; it says not to rely on the length).
pub(crate) fn valid_device_token(token: &str) -> bool {
    (32..=400).contains(&token.len())
        && token.len().is_multiple_of(2)
        && token.chars().all(|c| c.is_ascii_hexdigit())
}

/// The pushes a phone asked for that Hollow sends, in [`EVENTS`] order;
/// none named means all of them, and an unknown name is left out.
pub(crate) fn known_events(asked: Option<&[String]>) -> Vec<String> {
    match asked {
        None => EVENTS.iter().map(|e| e.to_string()).collect(),
        Some(asked) => EVENTS
            .iter()
            .filter(|e| asked.iter().any(|a| a.trim() == **e))
            .map(|e| e.to_string())
            .collect(),
    }
}

// ── The key ──────────────────────────────────────────────────────────────

/// The push key as Hollow keeps it in the Keychain.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PushKey {
    pub key_id: String,
    pub team_id: String,
    /// The `.p8` file's text (PKCS#8 PEM).
    pub key: String,
}

impl std::fmt::Debug for PushKey {
    // Never the key itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushKey")
            .field("key_id", &self.key_id)
            .field("team_id", &self.team_id)
            .finish_non_exhaustive()
    }
}

/// A Key ID or team id: ten upper-case letters and digits.
fn apple_id(raw: &str, what: &str) -> Result<String, String> {
    let id = raw.trim().to_ascii_uppercase();
    if id.len() == 10 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
        Ok(id)
    } else {
        Err(format!("The {what} is 10 letters and digits."))
    }
}

/// The Key ID in Apple's file name for a key, `AuthKey_<Key ID>.p8`.
pub(crate) fn key_id_from_file_name(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    apple_id(stem.strip_prefix("AuthKey_")?, "Key ID").ok()
}

/// The DER inside a PKCS#8 PEM (`BEGIN PRIVATE KEY`).
fn pem_der(pem: &str) -> Result<Vec<u8>, String> {
    let wrong = || {
        "That isn't an APNs key: the .p8 file from Apple starts with -----BEGIN PRIVATE KEY-----."
            .to_string()
    };
    let text = pem.trim();
    let body = text
        .strip_prefix("-----BEGIN PRIVATE KEY-----")
        .and_then(|rest| rest.trim_end().strip_suffix("-----END PRIVATE KEY-----"))
        .ok_or_else(wrong)?;
    let base64: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    general_purpose::STANDARD
        .decode(base64)
        .map_err(|_| wrong())
}

fn key_pair(der: &[u8]) -> Result<ring::signature::EcdsaKeyPair, String> {
    ring::signature::EcdsaKeyPair::from_pkcs8(
        &ring::signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        der,
        &ring::rand::SystemRandom::new(),
    )
    .map_err(|_| "That key isn't a P-256 key Apple's push service takes.".to_string())
}

impl PushKey {
    /// A key as imported: the `.p8` text, its Key ID and the team id,
    /// checked (the key must sign).
    pub(crate) fn parse(pem: &str, key_id: &str, team_id: &str) -> Result<Self, String> {
        let key_id = apple_id(key_id, "Key ID")?;
        let team_id = apple_id(team_id, "team ID")?;
        key_pair(&pem_der(pem)?)?;
        Ok(Self {
            key_id,
            team_id,
            key: pem.trim().to_string(),
        })
    }

    /// The JWT for `issued_at` (Unix seconds): `{"alg":"ES256","kid"}` and
    /// `{"iss":<team>,"iat"}`, signed (raw `r || s`, base64url).
    pub(crate) fn jwt(&self, issued_at: i64) -> Result<String, String> {
        let pair = key_pair(&pem_der(&self.key)?)?;
        let b64 = |bytes: &[u8]| general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        let header = serde_json::json!({ "alg": "ES256", "kid": self.key_id });
        let claims = serde_json::json!({ "iss": self.team_id, "iat": issued_at });
        let signing = format!(
            "{}.{}",
            b64(header.to_string().as_bytes()),
            b64(claims.to_string().as_bytes())
        );
        let signature = pair
            .sign(&ring::rand::SystemRandom::new(), signing.as_bytes())
            .map_err(|_| "Hollow couldn't sign with the push key.".to_string())?;
        Ok(format!("{signing}.{}", b64(signature.as_ref())))
    }
}

/// Where the key is kept: Hollow's Keychain item in the app, memory in
/// tests.
pub(crate) trait KeySource: Send + Sync + 'static {
    fn load(&self) -> Result<Option<PushKey>, String>;
    fn save(&self, key: &PushKey) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// The Keychain (`secret_store`, which never puts a value in an error).
pub(crate) struct KeychainKey;

impl KeySource for KeychainKey {
    fn load(&self) -> Result<Option<PushKey>, String> {
        let Some(raw) = crate::secret_store::get(SECRET_KEY).map_err(|e| e.to_string())? else {
            return Ok(None);
        };
        serde_json::from_str(&raw)
            .map(Some)
            .map_err(|_| "Hollow's saved push key can't be read. Import it again.".to_string())
    }
    fn save(&self, key: &PushKey) -> Result<(), String> {
        let raw = serde_json::to_string(key).map_err(|e| e.to_string())?;
        crate::secret_store::set(SECRET_KEY, &raw).map_err(|e| e.to_string())
    }
    fn delete(&self) -> Result<(), String> {
        crate::secret_store::delete(SECRET_KEY).map_err(|e| e.to_string())
    }
}

// ── Delivery ─────────────────────────────────────────────────────────────

/// Where pushes go: Apple's two services, or a fake one in tests.
#[derive(Clone, Debug)]
pub(crate) struct Endpoints {
    pub production: String,
    pub development: String,
    /// The `curl` that sends them.
    pub curl: PathBuf,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            production: PRODUCTION_URL.into(),
            development: SANDBOX_URL.into(),
            curl: PathBuf::from("/usr/bin/curl"),
        }
    }
}

/// Apple's answer to one push.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Delivery {
    pub status: u16,
    /// Apple's `reason`, when it refused.
    pub reason: Option<String>,
}

impl Delivery {
    /// The token will never work again: drop it.
    pub(crate) fn token_gone(&self) -> bool {
        self.status == 410
            || (self.status == 400
                && matches!(
                    self.reason.as_deref(),
                    Some("BadDeviceToken" | "DeviceTokenNotForTopic")
                ))
    }

    /// The JWT was refused: make a new one next time.
    pub(crate) fn jwt_refused(&self) -> bool {
        self.status == 403
            && matches!(
                self.reason.as_deref(),
                Some("ExpiredProviderToken" | "InvalidProviderToken")
            )
    }

    /// The JWT was refused as too old: a fresh one may carry the same push.
    pub(crate) fn jwt_expired(&self) -> bool {
        self.status == 403 && self.reason.as_deref() == Some("ExpiredProviderToken")
    }
}

/// A value inside a double-quoted `curl --config` string.
fn curl_quoted(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 8);
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A notification's [`curl_config_for`] (what v1.1's tests read).
#[cfg(test)]
pub(crate) fn curl_config(url: &str, jwt: &str, payload: &str) -> String {
    curl_config_for(url, jwt, payload, &ALERT)
}

/// What `curl` reads on its standard input for one push of `class` (a
/// notification, or a Live Activity with its own type, topic and priority):
/// the URL, the headers (the JWT among them) and the body. Nothing of it is
/// an argument.
pub(crate) fn curl_config_for(url: &str, jwt: &str, payload: &str, class: &PushClass) -> String {
    let mut config = format!("url = \"{}\"\nrequest = \"POST\"\n", curl_quoted(url));
    for (name, value) in [
        ("authorization", format!("bearer {jwt}")),
        ("apns-topic", class.topic.to_string()),
        ("apns-push-type", class.push_type.to_string()),
        ("apns-priority", class.priority.to_string()),
        ("content-type", "application/json".to_string()),
    ] {
        config.push_str(&format!("header = \"{name}: {}\"\n", curl_quoted(&value)));
    }
    // A payload is a JSON object, so it never starts with curl's `@`.
    config.push_str(&format!("data-binary = \"{}\"\n", curl_quoted(payload)));
    config
}

/// Send one push of `class`: `payload` to device `token` in `environment`,
/// signed by `jwt`. `Err` when it never reached Apple (no network, a
/// timeout).
pub(crate) async fn deliver_as(
    endpoints: &Endpoints,
    jwt: &str,
    token: &str,
    environment: &str,
    payload: &str,
    class: &PushClass,
) -> Result<Delivery, String> {
    use tokio::io::AsyncWriteExt;
    let base = if environment == DEVELOPMENT {
        &endpoints.development
    } else {
        &endpoints.production
    };
    let url = format!("{}/3/device/{token}", base.trim_end_matches('/'));
    // HTTP/2 to Apple; the tests' fake speaks plain HTTP/1.1.
    let http = if url.starts_with("https://") {
        "--http2"
    } else {
        "--http1.1"
    };
    let max_time = SEND_SECONDS.to_string();
    let mut child = tokio::process::Command::new(&endpoints.curl)
        .args([
            "-q",
            "--silent",
            "--show-error",
            http,
            "--max-time",
            &max_time,
            "--connect-timeout",
            "10",
            "--write-out",
            "\n%{http_code}",
            "--config",
            "-",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("Hollow couldn't start curl to send it ({e})"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(curl_config_for(&url, jwt, payload, class).as_bytes())
            .await
            .map_err(|e| format!("Hollow couldn't hand the push to curl ({e})"))?;
    }
    let output = tokio::time::timeout(
        Duration::from_secs(u64::from(SEND_SECONDS) + 10),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| "Apple's push service didn't answer in time".to_string())?
    .map_err(|e| format!("curl failed ({e})"))?;
    let out = String::from_utf8_lossy(&output.stdout);
    let (body, code) = out.rsplit_once('\n').unwrap_or(("", out.as_ref()));
    let status = code.trim().parse::<u16>().unwrap_or(0);
    if status == 0 {
        let said = String::from_utf8_lossy(&output.stderr);
        let said = said.trim().trim_start_matches("curl: ");
        return Err(if said.is_empty() {
            "Apple's push service couldn't be reached".to_string()
        } else {
            format!("Apple's push service couldn't be reached ({said})")
        });
    }
    let reason = serde_json::from_str::<Value>(body.trim())
        .ok()
        .and_then(|v| v.get("reason").and_then(Value::as_str).map(str::to_string));
    Ok(Delivery { status, reason })
}

// ── What a push says ─────────────────────────────────────────────────────

/// One notification to send, before it is addressed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Message {
    /// `None` for a test notification.
    pub key: Option<String>,
    pub kind: &'static str,
    pub title: String,
    pub body: String,
    /// A `turnFinished` for a turn that opened a card: a phone that asked
    /// for `question` pushes had that one instead.
    pub asked: bool,
    /// v1.2: a `question` for a card the phone can answer with one tap
    /// ([`card_actions`]).
    pub card: Option<wire::PushCard>,
}

impl Message {
    pub(crate) fn payload(&self) -> wire::PushPayload {
        // v1.2: the category the app registered buttons for. A question
        // gets one only with a card the extension can turn into buttons.
        let category = match (self.kind, &self.card) {
            (TURN_FINISHED, _) => Some(CATEGORY_TURN),
            (QUESTION, Some(_)) => Some(CATEGORY_QUESTION),
            _ => None,
        };
        wire::PushPayload {
            aps: wire::Aps {
                alert: wire::Alert {
                    title: self.title.clone(),
                    body: self.body.clone(),
                },
                sound: "default".into(),
                thread_id: self.key.clone(),
                category: category.map(str::to_string),
                mutable_content: self.card.as_ref().map(|_| 1),
            },
            session_key: self.key.clone(),
            kind: self.kind.to_string(),
            card: self.card.clone(),
        }
    }

    /// Whether `device` gets it: it registered for this kind (a test goes
    /// to every registered phone), and a `turnFinished` for a turn that
    /// asked goes only to a phone that didn't ask for questions.
    pub(crate) fn wanted_by(&self, device: &Device) -> bool {
        let Some(push) = &device.push else {
            return false;
        };
        if self.kind == TEST {
            return true;
        }
        let wants = |kind: &str| push.events.iter().any(|e| e == kind);
        // B4: an approval goes only to a phone that can answer it.
        if self.kind == APPROVAL && device.approval_keys.is_none() {
            return false;
        }
        wants(self.kind) && !(self.asked && wants(QUESTION))
    }
}

/// A chat's title as a push shows it: its own, else "New chat".
pub(crate) fn push_title(title: Option<&str>) -> String {
    let title = title
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "New chat".to_string());
    if title.chars().count() <= TITLE_MAX {
        return title;
    }
    let mut cut = super::live::cut_chars(&title, TITLE_MAX - 1);
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// A button's label: the option's label on one line, cut to
/// [`ACTION_LABEL_MAX`] characters with an ellipsis, else its id.
fn action_label(label: &str, id: &str) -> String {
    let label = label.split_whitespace().collect::<Vec<_>>().join(" ");
    let label = if label.is_empty() {
        id.to_string()
    } else {
        label
    };
    if label.chars().count() <= ACTION_LABEL_MAX {
        return label;
    }
    let mut cut = super::live::cut_chars(&label, ACTION_LABEL_MAX - 1);
    cut.truncate(cut.trim_end().len());
    cut.push('\u{2026}');
    cut
}

/// What a `question` push carries when its card can be answered with one tap
/// (v1.2, `docs/phone-api/notification-actions.md`): the card asks exactly
/// one question (a confirmation card is one, a verdicts card with one item
/// is one), that question takes one option (`single`), and it has 2 to 4
/// options whose ids are usable as they are (not empty, at most
/// [`ACTION_ID_MAX`] characters, none twice) and whose buttons read
/// differently once their labels are cut. Any other card is `None`: its push
/// is v1.1's, and a tap opens the chat.
pub(crate) fn card_actions(card: &crate::types::ChatInteraction) -> Option<wire::PushCard> {
    let questions = super::routes::questions_of(card)?;
    let [item] = questions.items.as_slice() else {
        return None;
    };
    // A verdict the card wants a reason for can't be sent by a button: the
    // answer would be refused for its missing reason, every time.
    let needs_reason: Vec<&str> = card
        .request
        .get("verdicts")
        .and_then(|v| {
            v.get("requireReasonOn")
                .or_else(|| v.get("require_reason_on"))
        })
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let offered: Vec<_> = item
        .options
        .iter()
        .filter(|option| !needs_reason.contains(&option.id.as_str()))
        .collect();
    if item.mode != "single"
        || item.id.is_empty()
        || item.id.chars().count() > ACTION_ID_MAX
        || !ACTION_OPTIONS.contains(&offered.len())
    {
        return None;
    }
    let mut seen = HashSet::new();
    let mut labels = HashSet::new();
    let mut options = Vec::with_capacity(offered.len());
    for option in offered {
        if option.id.is_empty()
            || option.id.chars().count() > ACTION_ID_MAX
            || !seen.insert(option.id.as_str())
        {
            return None;
        }
        // Two buttons must not read the same: a label cut to the button's width
        // can make two different options identical, and an answer can't be
        // taken back.
        let label = action_label(&option.label, &option.id);
        if !labels.insert(label.clone()) {
            return None;
        }
        options.push(wire::PushCardOption {
            id: option.id.clone(),
            label,
        });
    }
    Some(wire::PushCard {
        question_id: questions.id,
        item_id: item.id.clone(),
        options,
    })
}

/// The error a failed turn stopped with, from its trace.
fn turn_error(vault: &Path, key: &str, ord: u32) -> Option<String> {
    crate::session_store::with_read(vault, |conn| {
        use rusqlite::OptionalExtension;
        conn.query_row(
            "SELECT payload FROM events WHERE session_key = ?1 AND turn_ord = ?2 AND kind = 'error' \
             ORDER BY id DESC LIMIT 1",
            rusqlite::params![key, ord],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()
        .map(Option::flatten)
        .map_err(|e| e.to_string())
    })
    .ok()
    .flatten()
    .filter(|e| !e.trim().is_empty())
}

/// What a `turnFinished` push for `key` says, once its turn ended: `None`
/// for a chat the phone's list doesn't show, and for a turn that was
/// stopped or isn't over.
pub(crate) fn finished_message(vault: &Path, key: &str) -> Option<Message> {
    let summary = super::routes::live_chat(vault, key).ok()?;
    let turn = crate::session_store::latest_turn(vault, key).ok()??;
    let body = match turn.status.as_deref() {
        Some("success") => crate::session_store::newest_exchanges(vault, &[key.to_string()])
            .ok()
            .and_then(|mut rows| rows.remove(key))
            .and_then(|(_, _, answer)| answer.as_deref().and_then(super::routes::preview))
            .unwrap_or_else(|| "Finished.".to_string()),
        Some("error") => {
            let said = turn_error(vault, key, turn.ord)
                .map(|e| format!("Stopped with an error: {e}"))
                .unwrap_or_else(|| "Stopped with an error.".to_string());
            super::routes::preview(&said).unwrap_or(said)
        }
        _ => return None,
    };
    let asked = crate::interactions::list_pending(vault, key)
        .unwrap_or_default()
        .iter()
        .any(|card| card.status == "pending" && card.turn_ord == i64::from(turn.ord));
    Some(Message {
        key: Some(key.to_string()),
        kind: TURN_FINISHED,
        title: push_title(summary.title.as_deref()),
        body,
        asked,
        card: None,
    })
}

/// How chat `key`'s newest turn ended, by what Hollow stored for it (the
/// same status [`finished_message`] reads): `None` while the store can't say
/// (no turn, or one still marked running). The events a turn emits can
/// disagree with it: a refusal ends a turn as an error with no `error` event,
/// and an `error` event can come in a turn that went on to succeed.
pub(crate) fn stored_phase(vault: &Path, key: &str) -> Option<Phase> {
    let turn = crate::session_store::latest_turn(vault, key).ok()??;
    match turn.status.as_deref() {
        Some("success") => Some(Phase::Finished),
        Some("error") => Some(Phase::Failed),
        Some("cancelled" | "interrupted") => Some(Phase::Stopped),
        _ => None,
    }
}

/// What a `question` push for card `id` of chat `key` says: `None` once it
/// is no longer pending or for a chat the phone's list doesn't show.
pub(crate) fn question_message(vault: &Path, key: &str, id: &str) -> Option<Message> {
    let summary = super::routes::live_chat(vault, key).ok()?;
    let card = crate::interactions::list_pending(vault, key)
        .ok()?
        .into_iter()
        .find(|card| card.id == id && card.status == "pending")?;
    Some(Message {
        key: Some(key.to_string()),
        kind: QUESTION,
        title: push_title(summary.title.as_deref()),
        body: super::routes::card_preview(&card)
            .unwrap_or_else(|| "Hollow has a question for you.".to_string()),
        asked: false,
        card: card_actions(&card),
    })
}

/// B4: what an `approval` push for card `id` says ("<chat title>: <card
/// title>"): `None` once the card is resolved.
pub(crate) fn approval_message(vault: &Path, id: &str) -> Option<Message> {
    let card = super::approvals_api::open_cards()
        .into_iter()
        .find(|card| card.id == id)?;
    let chat = card
        .session_key
        .as_deref()
        .and_then(|key| super::routes::live_chat(vault, key).ok())
        .and_then(|summary| summary.title);
    Some(Message {
        key: card.session_key.clone(),
        kind: APPROVAL,
        title: push_title(chat.as_deref()),
        body: card.title.clone(),
        asked: false,
        card: None,
    })
}

// ── The sender ───────────────────────────────────────────────────────────

/// One job for the push task.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Job {
    /// A turn of chat `key` ended.
    TurnEnded { key: String },
    /// Card `id` of chat `key` opened.
    CardOpened { key: String, id: String },
    /// B4: approval card `id` was shown.
    ApprovalOpened { id: String },
    /// v1.2: a tool or the answer's text of chat `key`'s running turn
    /// changed, which may change its Live Activity's step.
    LiveStep { key: String },
    /// v1.2: a card opened in chat `key`: its Live Activity waits for the
    /// person.
    LiveWaiting { key: String },
    /// v1.2: chat `key`'s turn ended (`done`); `phase` is how the events
    /// hinted it ended, which the stored turn overrides. `tries` counts the
    /// re-sends of an end Apple didn't take (0 for the first). `mark` is
    /// [`Activities::mark`] when the turn ended: the end is for the activities
    /// registered by then, never for the next turn's.
    LiveEnd {
        key: String,
        phase: Phase,
        tries: u8,
        mark: u64,
    },
    /// v1.2: [`LIVE_REFRESH`] seconds after the last Live Activity push of
    /// chat `key`: tell its activities again when nothing else did.
    LiveRefresh { key: String },
}

/// Push as Settings shows it.
#[derive(Clone, Debug, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PhonePush {
    /// `ready` (a key is imported), `noKey`, or `error` (the saved key
    /// can't be read: `last_error` says why).
    pub state: String,
    pub key_id: Option<String>,
    pub team_id: Option<String>,
    /// Paired phones with notifications on.
    pub phones: usize,
    pub last_sent_at: Option<String>,
    pub last_error: Option<String>,
    pub last_error_at: Option<String>,
}

#[derive(Default)]
struct Outcomes {
    last_sent_at: Option<String>,
    last_error: Option<(String, String)>,
}

/// The JWT the pushes are signed with, kept.
struct KeptJwt {
    /// The key it was made with.
    key: PushKey,
    jwt: String,
    /// Its `iat`, by the [`Clock`].
    issued_at: i64,
}

/// The pushes Hollow sends: what the live worker sees, the jobs it makes,
/// and the key and JWT they're signed with.
pub(crate) struct Pusher {
    vault: PathBuf,
    store: Arc<Store>,
    live: Arc<Live>,
    keys: Arc<dyn KeySource>,
    endpoints: Endpoints,
    clock: Clock,
    jwt: Mutex<Option<KeptJwt>>,
    /// When a JWT may be signed again after an `InvalidProviderToken` (Unix
    /// seconds by the [`Clock`]; 0 none).
    jwt_blocked_until: AtomicI64,
    outcomes: Mutex<Outcomes>,
    /// Chats whose attended turn's `runStarted` the observer saw.
    running: Mutex<HashSet<String>>,
    /// Cards already decided, oldest first.
    cards: Mutex<VecDeque<String>>,
    jobs: tokio::sync::mpsc::UnboundedSender<Job>,
    /// v1.2: the Live Activity tokens phones registered, in memory only
    /// (`POST /v1/live-activity` writes them).
    activities: Arc<Activities>,
    /// v1.2: per chat, what its activities were last told and when (Unix
    /// seconds by the [`Clock`]): an unchanged state isn't sent again, and a
    /// step waits out [`LIVE_GAP`].
    live_sent: Mutex<HashMap<String, (live_activity::State, i64)>>,
    /// v1.2: chats with a [`Job::LiveStep`] queued or waiting out the gap, so
    /// a turn's burst of events makes one job.
    live_queued: Mutex<HashSet<String>>,
    /// v1.2: how chats whose turn was stopped or failed ended, until their
    /// `done` (which carries neither). Only the fallback: the stored turn
    /// says how it ended when it can.
    live_outcome: Mutex<HashMap<String, Phase>>,
    /// v1.2: per chat with a token, its running turn's tool calls by id, past
    /// the live buffer's cap on them ([`live_activity::ToolWatch`]).
    live_tools: Mutex<HashMap<String, live_activity::ToolWatch>>,
    /// v1.2: chats with a [`Job::LiveRefresh`] sleeping, so each has one.
    live_refreshing: Mutex<HashSet<String>>,
    /// v1.2: [`LIVE_END_WAIT`], seconds; tests shorten it.
    live_end_wait: AtomicU64,
    /// v1.2: [`LIVE_GAP`], seconds; tests shorten it.
    live_gap: AtomicU64,
    /// v1.2: [`LIVE_REFRESH`], seconds; tests shorten it.
    live_refresh: AtomicU64,
    /// v1.2: when a failing key read was last noted (Unix seconds by the
    /// [`Clock`]; 0 never).
    live_error_at: AtomicI64,
}

fn stamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

impl Pusher {
    /// The sender and the receiving end of its jobs, for [`Self::drain`].
    pub(crate) fn new(
        vault: PathBuf,
        store: Arc<Store>,
        live: Arc<Live>,
        keys: Arc<dyn KeySource>,
        endpoints: Endpoints,
        clock: Clock,
    ) -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<Job>) {
        let (jobs, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(Self {
                vault,
                store,
                live,
                keys,
                endpoints,
                clock,
                jwt: Mutex::new(None),
                jwt_blocked_until: AtomicI64::new(0),
                outcomes: Mutex::new(Outcomes::default()),
                running: Mutex::new(HashSet::new()),
                cards: Mutex::new(VecDeque::new()),
                jobs,
                activities: Arc::new(Activities::default()),
                live_sent: Mutex::new(HashMap::new()),
                live_queued: Mutex::new(HashSet::new()),
                live_outcome: Mutex::new(HashMap::new()),
                live_tools: Mutex::new(HashMap::new()),
                live_refreshing: Mutex::new(HashSet::new()),
                live_end_wait: AtomicU64::new(LIVE_END_WAIT),
                live_gap: AtomicU64::new(LIVE_GAP),
                live_refresh: AtomicU64::new(LIVE_REFRESH),
                live_error_at: AtomicI64::new(0),
            }),
            rx,
        )
    }

    /// v1.2: the Live Activity tokens, which the route `POST
    /// /v1/live-activity` shares.
    pub(crate) fn activities(&self) -> Arc<Activities> {
        self.activities.clone()
    }

    #[cfg(test)]
    pub(crate) fn set_live_gap(&self, seconds: u64) {
        self.live_gap.store(seconds, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn set_live_refresh(&self, seconds: u64) {
        self.live_refresh.store(seconds, Ordering::SeqCst);
    }

    #[cfg(test)]
    pub(crate) fn set_live_end_wait(&self, seconds: u64) {
        self.live_end_wait.store(seconds, Ordering::SeqCst);
    }

    /// Run jobs one after another, for as long as the sender lives.
    pub(crate) async fn drain(self: Arc<Self>, mut rx: tokio::sync::mpsc::UnboundedReceiver<Job>) {
        while let Some(job) = rx.recv().await {
            self.run(job).await;
        }
    }

    /// One chat event the live worker folded (`Live::observe`). Cheap: it
    /// only makes jobs.
    /// B4: an approval card was shown: push it to the phones that can answer.
    pub(crate) fn approval_opened(&self, id: &str) {
        let _ = self.jobs.send(Job::ApprovalOpened { id: id.to_string() });
    }

    pub(crate) fn observe(&self, event: &Value) {
        let kind = event.get("type").and_then(Value::as_str).unwrap_or("");
        let key = event
            .get("conversationKey")
            .and_then(Value::as_str)
            .filter(|k| crate::agent_sessions::is_safe_session_key(k));
        match (kind, key) {
            // Only a turn the person waits on (`attended`: they sent it)
            // pushes when it ends. Every other one, a night item, a Main
            // agent wake, check-in or swarm wake, an agent's chat, ends with
            // nobody waiting on it, whatever its `unattended` says (wakes
            // say false); no `attended` counts as not.
            ("runStarted", Some(key)) => {
                let mut running = self.running.lock().unwrap_or_else(|e| e.into_inner());
                if event.get("attended").and_then(Value::as_bool) == Some(true) {
                    running.insert(key.to_string());
                } else {
                    running.remove(key);
                }
                // v1.2: a new turn forgets how the last one ended, what its
                // activity was told (an entry that survived a token drop
                // would hide an equal first push) and its tool calls.
                self.live_outcome
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(key);
                self.live_sent
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(key);
                self.live_tools
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(key);
            }
            // Only a turn that started: a row taken out of the queue ends
            // with `done` too.
            ("done", Some(key)) => {
                let started = self
                    .running
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(key);
                if started {
                    let _ = self.jobs.send(Job::TurnEnded {
                        key: key.to_string(),
                    });
                }
                self.live_turn_ended(key);
            }
            // A new card pushes whatever started its turn, a night item's or
            // a check-in's too: it waits on the person (it is in Waiting on
            // you), and the phone's Focus settings keep quiet hours.
            ("interaction", _) => {
                if let Some((key, id)) = self.new_card(event) {
                    self.live_card_opened(&key);
                    let _ = self.jobs.send(Job::CardOpened { key, id });
                }
            }
            // v1.2: what a Live Activity's step is made of.
            ("toolUse" | "toolResult" | "assistantText" | "assistantDelta", Some(key)) => {
                self.live_changed(key, kind, event);
            }
            // v1.2: how an ending turn ended (its `done` says neither).
            ("cancelled", Some(key)) if self.activities.has_chat(key) => {
                self.live_outcome
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.to_string(), Phase::Stopped);
            }
            ("error", Some(key)) if self.activities.has_chat(key) => {
                self.live_outcome
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .entry(key.to_string())
                    .or_insert(Phase::Failed);
            }
            _ => {}
        }
    }

    /// v1.2: a tool or the answer's text of chat `key` changed. Only a chat
    /// a phone registered a Live Activity token for makes a job, and a
    /// burst of events one. Its tool calls are followed here by id, since the
    /// live buffer stops holding them at its cap.
    fn live_changed(&self, key: &str, kind: &str, event: &Value) {
        if !self.activities.has_chat(key) {
            return;
        }
        if kind == "toolUse" || kind == "toolResult" {
            let text = |name: &str| event.get(name).and_then(Value::as_str).unwrap_or("");
            // A watch that starts partway through a turn takes what the
            // buffer holds first (it already folded this event too, which
            // the watch counts once). Read before the lock: the buffer has
            // its own.
            let known = self
                .live_tools
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(key);
            let seeded = (!known).then(|| {
                self.live
                    .snapshot(key)
                    .map(|live| live_activity::ToolWatch::from_live(&live))
                    .unwrap_or_default()
            });
            let mut tools = self.live_tools.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(watch) = seeded {
                if tools.len() < LIVE_TOOL_CHATS {
                    tools.entry(key.to_string()).or_insert(watch);
                }
            }
            if let Some(watch) = tools.get_mut(key) {
                if kind == "toolUse" {
                    watch.used(text("id"), text("name"));
                } else {
                    watch.resulted(text("id"));
                }
            }
        }
        let first = self
            .live_queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string());
        if first {
            let _ = self.jobs.send(Job::LiveStep {
                key: key.to_string(),
            });
        }
    }

    /// v1.2: a card opened in chat `key`.
    fn live_card_opened(&self, key: &str) {
        if self.activities.has_chat(key) {
            let _ = self.jobs.send(Job::LiveWaiting {
                key: key.to_string(),
            });
        }
    }

    /// v1.2: chat `key`'s turn ended, whoever started it.
    fn live_turn_ended(&self, key: &str) {
        self.live_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        let phase = self
            .live_outcome
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key)
            .unwrap_or(Phase::Finished);
        if self.activities.has_chat(key) {
            // The end is for the activities registered now. The job may wait
            // behind a notification push to Apple, and the open app can end
            // the old activity and register the next turn's meanwhile.
            let _ = self.jobs.send(Job::LiveEnd {
                key: key.to_string(),
                phase,
                tries: 0,
                mark: self.activities.mark(),
            });
        }
    }

    /// A card the event says was just recorded, pending, and not decided
    /// before (a card reopened after an unsent answer is no new question).
    fn new_card(&self, event: &Value) -> Option<(String, String)> {
        let card = event.get("interaction")?;
        let text = |name: &str| card.get(name).and_then(Value::as_str);
        if text("status") != Some("pending") {
            return None;
        }
        let id = text("id")?.to_string();
        let key = text("sessionKey")
            .filter(|k| crate::agent_sessions::is_safe_session_key(k))?
            .to_string();
        let created = chrono::DateTime::parse_from_rfc3339(text("createdAt")?).ok()?;
        if chrono::Utc::now().signed_duration_since(created) > NEW_CARD_FOR {
            return None;
        }
        let mut cards = self.cards.lock().unwrap_or_else(|e| e.into_inner());
        if cards.contains(&id) {
            return None;
        }
        cards.push_back(id.clone());
        while cards.len() > SEEN_CARDS {
            cards.pop_front();
        }
        Some((key, id))
    }

    /// Do one job: read what it says, then send it.
    pub(crate) async fn run(&self, job: Job) {
        let job = match job {
            Job::LiveStep { key } => return self.run_live_step(&key).await,
            Job::LiveWaiting { key } => return self.run_live_waiting(&key).await,
            Job::LiveEnd {
                key,
                phase,
                tries,
                mark,
            } => return self.run_live_end(&key, phase, tries, mark).await,
            Job::LiveRefresh { key } => return self.run_live_refresh(&key).await,
            job => job,
        };
        let vault = self.vault.clone();
        let message = tokio::task::spawn_blocking(move || match job {
            Job::TurnEnded { key } => finished_message(&vault, &key),
            Job::CardOpened { key, id } => question_message(&vault, &key, &id),
            Job::ApprovalOpened { id } => approval_message(&vault, &id),
            // The Live Activity jobs went above.
            _ => None,
        })
        .await
        .ok()
        .flatten();
        if let Some(message) = message {
            self.send(&message).await;
        }
    }

    fn note_error(&self, error: &str) {
        let error = super::live::cut_chars(error.trim(), ERROR_MAX);
        eprintln!("[phone] push: {error}");
        self.outcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_error = Some((error, stamp()));
    }

    /// The JWT for `key`: the kept one while it was made with this key and
    /// its `iat` is less than [`JWT_FOR`] ago by the wall clock, else a new
    /// one. A clock set back before its `iat` makes a new one too.
    fn jwt_for(&self, key: &PushKey) -> Result<String, String> {
        let now = (self.clock)();
        let mut slot = self.jwt.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(kept) = slot.as_ref() {
            let age = now - kept.issued_at;
            if kept.key == *key && (0..JWT_FOR.as_secs() as i64).contains(&age) {
                return Ok(kept.jwt.clone());
            }
        }
        let until = self.jwt_blocked_until.load(Ordering::SeqCst);
        // A wait longer than the whole wait means the clock was set back since
        // it started: it has run out.
        if now < until && until - now <= JWT_INVALID_WAIT.as_secs() as i64 {
            return Err(format!(
                "Apple called the push key's token invalid; Hollow waits {} more minutes before signing another (check the key, key ID and team ID in Settings).",
                (until - now + 59) / 60
            ));
        }
        let jwt = key.jwt(now)?;
        *slot = Some(KeptJwt {
            key: key.clone(),
            jwt: jwt.clone(),
            issued_at: now,
        });
        Ok(jwt)
    }

    fn forget_jwt(&self) {
        *self.jwt.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// Apple refused `jwt`: drop it, if it is still the kept one. An invalid
    /// one also starts the wait before another is signed
    /// ([`JWT_INVALID_WAIT`]); an expired one doesn't (`deliver_signed` signs
    /// one fresh JWT for it, once per push). A late refusal of a JWT already
    /// replaced (an old key's, or one a newer push made again) changes
    /// nothing: it must not block a freshly imported key.
    fn jwt_was_refused(&self, jwt: &str, delivery: &Delivery) {
        let mut slot = self.jwt.lock().unwrap_or_else(|e| e.into_inner());
        if slot.as_ref().is_none_or(|kept| kept.jwt != jwt) {
            return;
        }
        *slot = None;
        if delivery.reason.as_deref() == Some("InvalidProviderToken") {
            self.jwt_blocked_until.store(
                (self.clock)() + JWT_INVALID_WAIT.as_secs() as i64,
                Ordering::SeqCst,
            );
        }
    }

    #[cfg(test)]
    pub(crate) fn jwt_for_test(&self, key: &PushKey) -> Result<String, String> {
        self.jwt_for(key)
    }

    #[cfg(test)]
    pub(crate) fn refused_for_test(&self, jwt: &str, delivery: &Delivery) {
        self.jwt_was_refused(jwt, delivery);
    }

    #[cfg(test)]
    pub(crate) fn import_for_test(&self) {
        self.forget_jwt_and_wait();
    }

    /// A new or removed key starts clean.
    fn forget_jwt_and_wait(&self) {
        self.forget_jwt();
        self.jwt_blocked_until.store(0, Ordering::SeqCst);
    }

    /// `payload` to one phone, signed with the kept JWT. When Apple calls
    /// that JWT expired (403 `ExpiredProviderToken`), it is dropped, a fresh
    /// one is signed and the push goes once more, and Apple's answer to that
    /// one is returned. `Err` when no JWT could be signed; the inner `Err`
    /// when the push never reached Apple.
    async fn deliver_signed(
        &self,
        key: &PushKey,
        token: &str,
        environment: &str,
        payload: &str,
        class: &PushClass,
    ) -> Result<Result<Delivery, String>, String> {
        let mut again = true;
        loop {
            let jwt = self.jwt_for(key)?;
            let delivered =
                deliver_as(&self.endpoints, &jwt, token, environment, payload, class).await;
            if let Ok(delivery) = &delivered {
                if delivery.jwt_refused() {
                    self.jwt_was_refused(&jwt, delivery);
                }
            }
            let expired = matches!(&delivered, Ok(delivery) if delivery.jwt_expired());
            if !(again && expired) {
                return Ok(delivered);
            }
            again = false;
            eprintln!("[phone] push: JWT expired; sending again with a fresh one");
        }
    }

    /// Send `message` to every phone that wants it and isn't watching its
    /// chat. Returns how many Apple took, and the errors (one per phone).
    pub(crate) async fn send(&self, message: &Message) -> (usize, Vec<String>) {
        let key = match self.keys.load() {
            Ok(Some(key)) => key,
            // No key yet: nothing to send, and nothing wrong.
            Ok(None) => return (0, Vec::new()),
            Err(error) => {
                self.note_error(&error);
                return (0, vec![error]);
            }
        };
        let devices = match self.store.read() {
            Ok(record) => record.devices,
            Err(error) => {
                self.note_error(&error);
                return (0, vec![error]);
            }
        };
        let watching = message
            .key
            .as_deref()
            .map(|key| self.live.watchers(key))
            .unwrap_or_default();
        let targets: Vec<Device> = devices
            .into_iter()
            .filter(|d| message.wanted_by(d) && !watching.contains(&d.device_id))
            .collect();
        if targets.is_empty() {
            return (0, Vec::new());
        }
        let payload = match serde_json::to_string(&message.payload()) {
            Ok(payload) => payload,
            Err(error) => return (0, vec![error.to_string()]),
        };
        let mut sent = 0;
        let mut errors = Vec::new();
        for device in targets {
            let Some(push) = device.push.as_ref() else {
                continue;
            };
            let delivered = match self
                .deliver_signed(&key, &push.token, &push.environment, &payload, &ALERT)
                .await
            {
                Ok(delivered) => delivered,
                Err(error) => {
                    self.note_error(&error);
                    errors.push(error);
                    break;
                }
            };
            match delivered {
                Ok(delivery) if delivery.status == 200 => {
                    sent += 1;
                    self.outcomes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .last_sent_at = Some(stamp());
                }
                Ok(delivery) => {
                    let why = format!(
                        "Apple refused the push to {} ({}{})",
                        device.name,
                        delivery.status,
                        delivery
                            .reason
                            .as_deref()
                            .map(|r| format!(" {r}"))
                            .unwrap_or_default()
                    );
                    if delivery.token_gone() {
                        let (store, token) = (self.store.clone(), push.token.clone());
                        let _ = tokio::task::spawn_blocking(move || store.drop_push_token(&token))
                            .await;
                    }
                    self.note_error(&why);
                    errors.push(why);
                }
                Err(error) => {
                    let why = format!("The push to {} didn't go: {error}", device.name);
                    self.note_error(&why);
                    errors.push(why);
                }
            }
        }
        (sent, errors)
    }

    // ── v1.2: Live Activities ────────────────────────────────────────────

    /// What chat `key`'s running turn `live` is doing: its tool calls as the
    /// watch followed them (the buffer stops holding them at its cap), else
    /// what the buffer holds.
    fn live_step_of(&self, key: &str, live: &wire::Live) -> String {
        match self
            .live_tools
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
        {
            Some(watch) => {
                live_activity::step_of(watch.newest_running(), !live.text.is_empty(), watch.seen())
            }
            None => live_activity::current_step(live),
        }
    }

    /// A card of chat `key` is still pending (the store says).
    async fn card_pending(&self, key: &str) -> bool {
        let (vault, key) = (self.vault.clone(), key.to_string());
        tokio::task::spawn_blocking(move || {
            crate::interactions::list_pending(&vault, &key)
                .unwrap_or_default()
                .iter()
                .any(|card| card.status == "pending")
        })
        .await
        .unwrap_or(false)
    }

    /// A step may have changed: tell the chat's activities when it did and
    /// the gap since the last push allows, else come back when it is over
    /// and read the newest step then.
    async fn run_live_step(&self, key: &str) {
        self.live_queued
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        let now = (self.clock)();
        let targets = self.activities.for_chat(key, now);
        if targets.is_empty() {
            return;
        }
        // Not running any more (its `done` folded it away): the `end` push
        // says so.
        let Some(live) = self.live.snapshot(key) else {
            return;
        };
        let state = live_activity::State::running(self.live_step_of(key, &live));
        let last = self
            .live_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned();
        if let Some((sent, at)) = last {
            // A question still waiting for its answer stays what the screen
            // says: the turn ends right after it, and a step would hide it.
            if sent.phase == Phase::Waiting && self.card_pending(key).await {
                return;
            }
            if sent == state {
                return;
            }
            let gap = self.live_gap.load(Ordering::SeqCst) as i64;
            let wait = at + gap - now;
            if wait > 0 {
                // A wall clock set back must not hold the jobs for longer
                // than the gap.
                let wait = wait.min(gap.max(1));
                let first = self
                    .live_queued
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(key.to_string());
                if first {
                    let jobs = self.jobs.clone();
                    let key = key.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(wait as u64)).await;
                        let _ = jobs.send(Job::LiveStep { key });
                    });
                }
                return;
            }
        }
        self.live_push(key, &state, &targets, &LIVE_UPDATE, now)
            .await;
    }

    /// A card opened: the chat's activities wait for the person, at once.
    async fn run_live_waiting(&self, key: &str) {
        let now = (self.clock)();
        let targets = self.activities.for_chat(key, now);
        if targets.is_empty() {
            return;
        }
        let state = live_activity::State::waiting();
        let told = self
            .live_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .is_some_and(|(sent, _)| *sent == state);
        if told {
            return;
        }
        self.live_push(key, &state, &targets, &LIVE_URGENT, now)
            .await;
    }

    /// The turn ended: the chat's activities end as Hollow stored the turn
    /// (`hinted`, from the events, only when the store can't say), and the
    /// tokens that were told go (they name one running activity).
    async fn run_live_end(&self, key: &str, hinted: Phase, tries: u8, mark: u64) {
        let now = (self.clock)();
        // Only the activities registered when the turn ended: while this job
        // waited (behind a notification push, or for a re-send) the app may
        // have ended the old activity and registered the chat's next turn's,
        // which must not be told the previous turn is over.
        let targets: Vec<_> = self
            .activities
            .for_chat(key, now)
            .into_iter()
            .filter(|entry| entry.seq <= mark)
            .collect();
        let mut retry = false;
        if !targets.is_empty() {
            let (vault, owned) = (self.vault.clone(), key.to_string());
            let stored = tokio::task::spawn_blocking(move || stored_phase(&vault, &owned))
                .await
                .ok()
                .flatten();
            retry = self
                .live_push(
                    key,
                    &live_activity::State::ended(stored.unwrap_or(hinted)),
                    &targets,
                    &LIVE_URGENT,
                    now,
                )
                .await;
        }
        if retry && tries < LIVE_END_TRIES {
            // Apple didn't take the end (or nobody could ask): the tokens stay,
            // and the end goes again, a little later each time, to the same
            // activities (`mark`), even once the chat's next turn has begun:
            // the old activity still needs its end.
            let wait = self.live_end_wait.load(Ordering::SeqCst) * u64::from(tries + 1);
            let jobs = self.jobs.clone();
            let key = key.to_string();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_secs(wait)).await;
                let _ = jobs.send(Job::LiveEnd {
                    key,
                    phase: hinted,
                    tries: tries + 1,
                    mark,
                });
            });
            return;
        }
        // Only the tokens that were told: while the push was away, the app
        // may have registered one for the chat's next turn.
        for entry in &targets {
            self.activities.drop_token(&entry.token);
        }
        // The end's own record goes; a next turn's push already made isn't it.
        let mut sent = self.live_sent.lock().unwrap_or_else(|e| e.into_inner());
        if sent
            .get(key)
            .is_some_and(|(state, _)| state.phase.is_final())
        {
            sent.remove(key);
        }
    }

    /// [`LIVE_REFRESH`] seconds after the last push: when nothing told the
    /// activities since (a changed step restarts the wait), tell them again,
    /// so a step that is only long never goes stale on the lock screen.
    async fn run_live_refresh(&self, key: &str) {
        self.live_refreshing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
        let now = (self.clock)();
        let targets = self.activities.for_chat(key, now);
        if targets.is_empty() {
            return;
        }
        let last = self
            .live_sent
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned();
        // Nothing told, or the end did: nothing to keep alive.
        let Some((sent, at)) = last else {
            return;
        };
        if sent.phase.is_final() {
            return;
        }
        let period = self.live_refresh.load(Ordering::SeqCst) as i64;
        let left = at + period - now;
        if left > 0 {
            // A later push already told them: wait out what is left of it
            // (never longer than a period, whatever the clock did).
            self.schedule_live_refresh(key, left.min(period.max(1)) as u64);
            return;
        }
        // A running turn that went away (`done` not seen yet) isn't kept
        // alive; its `end` is on its way.
        let state = match sent.phase {
            Phase::Running => match self.live.snapshot(key) {
                Some(live) => live_activity::State::running(self.live_step_of(key, &live)),
                None => return,
            },
            _ => sent,
        };
        self.live_push(key, &state, &targets, &LIVE_UPDATE, now)
            .await;
    }

    /// Send chat `key`'s activities a [`Job::LiveRefresh`] in `seconds`, once
    /// at a time per chat.
    fn schedule_live_refresh(&self, key: &str, seconds: u64) {
        let first = self
            .live_refreshing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string());
        if !first {
            return;
        }
        let jobs = self.jobs.clone();
        let key = key.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(seconds)).await;
            let _ = jobs.send(Job::LiveRefresh { key });
        });
    }

    /// Note a failing key read for Settings, at most once a minute.
    fn note_live_key_error(&self, error: &str, now: i64) {
        let last = self.live_error_at.load(Ordering::SeqCst);
        if last == 0 || !(0..LIVE_ERROR_EVERY).contains(&(now - last)) {
            self.live_error_at.store(now, Ordering::SeqCst);
            self.note_error(error);
        }
    }

    /// `state` to each of `targets` (the registered activities of chat
    /// `key`) whose phone is still paired. No key imported: nothing is sent
    /// and nothing is wrong; the tokens stay for when there is one. Returns
    /// whether a push should be tried again (no answer from Apple, a 5xx or
    /// 429, a refused JWT, a key or phone record that couldn't be read): a
    /// token Apple calls gone, or a refusal that waiting won't change, is not.
    async fn live_push(
        &self,
        key: &str,
        state: &live_activity::State,
        targets: &[live_activity::Entry],
        class: &PushClass,
        now: i64,
    ) -> bool {
        let signing = match self.keys.load() {
            Ok(Some(signing)) => signing,
            Ok(None) => return false,
            Err(error) => {
                self.note_live_key_error(&error, now);
                return true;
            }
        };
        let devices = match self.store.read() {
            Ok(record) => record.devices,
            Err(error) => {
                self.note_live_key_error(&error, now);
                return true;
            }
        };
        // A phone revoked since it registered is forgotten here.
        let paired: HashSet<String> = devices.iter().map(|d| d.device_id.clone()).collect();
        self.activities.retain_devices(&paired);
        // ActivityKit drops a push that isn't newer than the last one it took
        // (`aps.timestamp`), so a card right after a step, or the end right
        // after a card, within the same second goes a second later.
        let at = {
            let mut sent = self.live_sent.lock().unwrap_or_else(|e| e.into_inner());
            let at = sent
                .get(key)
                .map_or(now, |(_, last)| now.max(last.saturating_add(1)));
            sent.insert(key.to_string(), (state.clone(), at));
            at
        };
        let mut retry = false;
        let body = live_activity::payload(state, at).to_string();
        for entry in targets.iter().filter(|e| paired.contains(&e.device_id)) {
            let name = devices
                .iter()
                .find(|d| d.device_id == entry.device_id)
                .map(|d| d.name.as_str())
                .unwrap_or("a phone");
            let delivered = match self
                .deliver_signed(&signing, &entry.token, &entry.environment, &body, class)
                .await
            {
                Ok(delivered) => delivered,
                Err(error) => {
                    self.note_live_key_error(&error, now);
                    retry = true;
                    break;
                }
            };
            match delivered {
                Ok(delivery) if delivery.status == 200 => {
                    self.outcomes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .last_sent_at = Some(stamp());
                }
                Ok(delivery) => {
                    if delivery.token_gone() {
                        self.activities.drop_token(&entry.token);
                    } else if delivery.jwt_refused()
                        || delivery.status == 429
                        || delivery.status >= 500
                    {
                        retry = true;
                    }
                    // A 410 is the routine answer for an activity that
                    // already ended (the app ended it first, or the person
                    // swiped it away): the token goes, quietly, and Settings
                    // keeps showing the real errors.
                    if delivery.status != 410 {
                        self.note_error(&format!(
                            "Apple refused the Live Activity push to {name} ({}{})",
                            delivery.status,
                            delivery
                                .reason
                                .as_deref()
                                .map(|r| format!(" {r}"))
                                .unwrap_or_default()
                        ));
                    }
                }
                Err(error) => {
                    retry = true;
                    self.note_error(&format!(
                        "The Live Activity push to {name} didn't go: {error}"
                    ));
                }
            }
        }
        // A running or waiting activity is told again if nothing else does.
        if !state.phase.is_final() {
            self.schedule_live_refresh(key, self.live_refresh.load(Ordering::SeqCst));
        }
        retry
    }

    /// Settings' Send test notification: to every phone with notifications
    /// on, watching or not.
    pub(crate) async fn test(&self, mac_name: &str) -> Result<usize, String> {
        if self.keys.load()?.is_none() {
            return Err("Import the push key first.".into());
        }
        let phones = self
            .store
            .read()?
            .devices
            .iter()
            .filter(|d| d.push.is_some())
            .count();
        if phones == 0 {
            return Err(
                "No paired iPhone has turned on notifications yet. Open the Hollow app on the iPhone and allow them."
                    .into(),
            );
        }
        let message = Message {
            key: None,
            kind: TEST,
            title: "Hollow".into(),
            body: format!("Test notification from {mac_name}."),
            asked: false,
            card: None,
        };
        let (sent, errors) = self.send(&message).await;
        if sent == 0 {
            return Err(errors
                .into_iter()
                .next()
                .unwrap_or_else(|| "No notification went.".into()));
        }
        Ok(sent)
    }

    /// Import a key (Settings): checked, kept in the Keychain, and used for
    /// the next push.
    pub(crate) fn import(&self, pem: &str, key_id: &str, team_id: &str) -> Result<(), String> {
        let key = PushKey::parse(pem, key_id, team_id)?;
        self.keys.save(&key)?;
        self.forget_jwt_and_wait();
        self.outcomes
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_error = None;
        Ok(())
    }

    /// Remove the key (Settings): no more pushes until one is imported.
    pub(crate) fn remove_key(&self) -> Result<(), String> {
        self.keys.delete()?;
        self.forget_jwt_and_wait();
        Ok(())
    }

    /// Push as Settings shows it.
    pub(crate) fn status(&self) -> PhonePush {
        let phones = self
            .store
            .read()
            .map(|r| r.devices.iter().filter(|d| d.push.is_some()).count())
            .unwrap_or(0);
        let outcomes = self.outcomes.lock().unwrap_or_else(|e| e.into_inner());
        let (last_error, last_error_at) = match outcomes.last_error.clone() {
            Some((error, at)) => (Some(error), Some(at)),
            None => (None, None),
        };
        let mut status = PhonePush {
            state: "noKey".into(),
            key_id: None,
            team_id: None,
            phones,
            last_sent_at: outcomes.last_sent_at.clone(),
            last_error,
            last_error_at,
        };
        match self.keys.load() {
            Ok(Some(key)) => {
                status.state = "ready".into();
                status.key_id = Some(key.key_id);
                status.team_id = Some(key.team_id);
            }
            Ok(None) => {}
            Err(error) => {
                status.state = "error".into();
                status.last_error = Some(error);
                status.last_error_at = None;
            }
        }
        status
    }
}
