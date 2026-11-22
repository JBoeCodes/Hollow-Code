//! The phone API's approval routes (build run 11-17, row B4, ENG-172; design
//! `docs/phone-api/approvals-design.md`, contract `docs/phone-api/approvals.md`,
//! fixtures `docs/phone-api-fixtures/approvals/`).
//!
//! This file holds the enrollment of a phone's approval keys and the read of
//! pending cards (PR 1), and (PR 2) the challenge and decision routes that
//! answer a card with a signature.
//!
//! **Enrollment** is the one step that needs the Mac. The person opens an
//! approvals code window for one phone ([`Enroll::open`]: 8 characters from the
//! pairing alphabet, 10 minutes, one at a time, in memory); the phone then asks
//! for a challenge, builds the enrollment message
//! (`hollow-approval-key-v1`, device id, approve key, deny key, challenge, one
//! per line), proves it knows the code with an HMAC over that message, and
//! proves it is the genuine app with an App Attest attestation over it
//! ([`super::app_attest`]). A stolen bearer token has none of that, so it
//! can't enroll a key of its own.

use super::app_attest::{self, Expected};
use super::routes::{self, json, ApiError, Ctx, Reply};
use super::store::{
    display, normalize_code, random_code, same_bytes, ApprovalKeys, Device, MAX_WRONG_CODES,
    PAIRING_MINUTES,
};
use crate::approvals::AnsweredBy;
use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine as _,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rand::RngCore;
use ring::hmac;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The App ID an attestation must be for: `<team>.<bundle id>`.
pub(crate) const APP_ID: &str = "TEAM000000.com.hollow.app.companion";
/// How long an enrollment challenge lives.
const CHALLENGE_SECS: i64 = 60;
/// The enrollment message's first line.
const ENROLL_PREFIX: &str = "hollow-approval-key-v1";
/// The message a decision signs, first line.
const DECISION_PREFIX: &str = "hollow-approval-v1";
/// Open decision challenges kept per phone.
const CHALLENGES_PER_PHONE: usize = 16;
/// How long a decision challenge lives.
const DECISION_CHALLENGE_SECS: i64 = 60;
/// Bad signatures from one phone, within [`BAD_SIGNATURE_MINUTES`], that drop its keys.
const BAD_SIGNATURES_MAX: usize = 5;
const BAD_SIGNATURE_MINUTES: i64 = 10;
/// The longest signature taken (a P-256 DER signature is at most 72 bytes).
const SIGNATURE_MAX: usize = 128;
/// The longest attestation object taken (Apple's are about 5 KB).
const ATTESTATION_MAX: usize = 32 * 1024;

// ── The approvals code window ────────────────────────────────────────────

struct Window {
    device_id: String,
    /// The code as the Mac displays it, normalized (upper case, no dash).
    code: String,
    expires: DateTime<Utc>,
    failures: u32,
    /// The open challenge (base64url) and when it ends; one use.
    challenge: Option<(String, DateTime<Utc>)>,
}

/// An open approvals code window, as the Mac shows it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenWindow {
    pub device_id: String,
    pub display: String,
    pub expires: DateTime<Utc>,
}

/// What a proof attempt came to.
#[derive(Debug, PartialEq)]
pub(crate) enum Proof {
    /// No window for this phone (never opened, expired, used up or closed by
    /// five wrong proofs).
    Closed,
    /// The challenge isn't this phone's open one: expired, used or unknown.
    ChallengeGone,
    Wrong {
        attempts_left: u32,
    },
    Right,
}

/// The approvals code window: one at a time, for one phone, in memory.
#[derive(Default)]
pub(crate) struct Enroll {
    window: Mutex<Option<Window>>,
}

impl Enroll {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Window>> {
        self.window.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Open a fresh window for `device_id`, closing any open one. PR 2's
    /// Settings row calls it; until then only tests do.
    #[allow(dead_code)]
    pub(crate) fn open(&self, device_id: &str, now: DateTime<Utc>) -> (OpenWindow, String) {
        self.open_with(device_id, &random_code(), now)
    }

    /// [`Self::open`] with a known code (tests). Returns the window and the code.
    pub(crate) fn open_with(
        &self,
        device_id: &str,
        code: &str,
        now: DateTime<Utc>,
    ) -> (OpenWindow, String) {
        let expires = now + Duration::minutes(PAIRING_MINUTES);
        *self.lock() = Some(Window {
            device_id: device_id.to_string(),
            code: normalize_code(code),
            expires,
            failures: 0,
            challenge: None,
        });
        (
            OpenWindow {
                device_id: device_id.to_string(),
                display: display(&normalize_code(code)),
                expires,
            },
            code.to_string(),
        )
    }

    /// Close the window (the Mac's Cancel).
    pub(crate) fn close(&self) {
        *self.lock() = None;
    }

    /// Close the window only if it is `device_id`'s (an enrollment that
    /// finishes must not close a window the Mac opened for another phone
    /// meanwhile).
    pub(crate) fn close_for(&self, device_id: &str) {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|w| w.device_id == device_id) {
            *guard = None;
        }
    }

    /// The open window, unless it ended (an ended one is dropped). PR 2's
    /// Settings row reads it to show the code.
    #[allow(dead_code)]
    pub(crate) fn current(&self, now: DateTime<Utc>) -> Option<(OpenWindow, String)> {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|w| w.expires <= now) {
            *guard = None;
        }
        guard.as_ref().map(|w| {
            (
                OpenWindow {
                    device_id: w.device_id.clone(),
                    display: display(&w.code),
                    expires: w.expires,
                },
                w.code.clone(),
            )
        })
    }

    /// A one-use challenge for `device_id`, or `None` with no window for it.
    pub(crate) fn challenge(
        &self,
        device_id: &str,
        now: DateTime<Utc>,
    ) -> Option<(String, DateTime<Utc>)> {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|w| w.expires <= now) {
            *guard = None;
        }
        let window = guard.as_mut().filter(|w| w.device_id == device_id)?;
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let challenge = URL_SAFE_NO_PAD.encode(bytes);
        let ends = (now + Duration::seconds(CHALLENGE_SECS)).min(window.expires);
        window.challenge = Some((challenge.clone(), ends));
        Some((challenge, ends))
    }

    /// An attestation was refused after a right proof: it counts toward the
    /// window's five like a wrong proof, so a holder of the code can't probe
    /// attestations for the whole ten minutes. Closes the window at five.
    pub(crate) fn attestation_failed(&self, device_id: &str) {
        let mut guard = self.lock();
        let Some(window) = guard.as_mut().filter(|w| w.device_id == device_id) else {
            return;
        };
        window.failures += 1;
        if window.failures >= MAX_WRONG_CODES {
            *guard = None;
        }
    }

    /// Check `proof` (lower-case hex of HMAC-SHA256 over `message`, keyed by
    /// the code). The challenge is used up whatever the outcome, so a wrong
    /// proof can't be retried against the same one; a wrong one counts toward
    /// the window's five.
    pub(crate) fn prove(
        &self,
        device_id: &str,
        challenge: &str,
        message: &str,
        proof: &str,
        now: DateTime<Utc>,
    ) -> Proof {
        let mut guard = self.lock();
        if guard.as_ref().is_some_and(|w| w.expires <= now) {
            *guard = None;
        }
        let Some(window) = guard.as_mut().filter(|w| w.device_id == device_id) else {
            return Proof::Closed;
        };
        let open = window.challenge.take();
        let fresh = open
            .is_some_and(|(c, ends)| ends > now && same_bytes(c.as_bytes(), challenge.as_bytes()));
        if !fresh {
            return Proof::ChallengeGone;
        }
        let key = hmac::Key::new(hmac::HMAC_SHA256, window.code.as_bytes());
        let right = hex_decode(proof)
            .is_some_and(|tag| hmac::verify(&key, message.as_bytes(), &tag).is_ok());
        if right {
            return Proof::Right;
        }
        window.failures += 1;
        let attempts_left = MAX_WRONG_CODES.saturating_sub(window.failures);
        if attempts_left == 0 {
            *guard = None;
        }
        Proof::Wrong { attempts_left }
    }
}

fn hex_decode(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.is_ascii() {
        return None;
    }
    // Hex digits only: `from_str_radix` alone would take a leading `+`.
    if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The enrollment message the phone builds, as Hollow rebuilds it.
pub(crate) fn enroll_message(
    device_id: &str,
    approve_key: &str,
    deny_key: &str,
    challenge: &str,
) -> String {
    [ENROLL_PREFIX, device_id, approve_key, deny_key, challenge].join("\n")
}

/// The first 8 lower-case hex digits of SHA-256(approve point || deny point),
/// as two groups of four (`a1b2 c3d4`).
pub(crate) fn fingerprint(approve: &[u8], deny: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(approve);
    hasher.update(deny);
    let digits = hex(&hasher.finalize()[..4]);
    format!("{} {}", &digits[..4], &digits[4..])
}

// ── Wire bodies ──────────────────────────────────────────────────────────

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChallengeBody {
    pub(crate) challenge: String,
    pub(crate) expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EnrollRequest {
    #[serde(default)]
    approve_key: Option<String>,
    #[serde(default)]
    deny_key: Option<String>,
    #[serde(default)]
    challenge: Option<String>,
    #[serde(default)]
    key_id: Option<String>,
    #[serde(default)]
    attestation: Option<String>,
    #[serde(default)]
    proof: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EnrolledBody {
    pub(crate) fingerprint: String,
    pub(crate) enrolled_at: String,
}

/// One pending card as the phone reads it.
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalItem {
    pub id: String,
    pub session_key: Option<String>,
    pub chat_title: Option<String>,
    pub title: String,
    pub action: String,
    pub kind: String,
    pub created_at: String,
    pub expires_at: String,
    /// SHA-256 (lower-case hex) of the card's canonical text.
    pub digest: String,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ApprovalsBody {
    /// This phone has approval keys, so it may answer (reading needs none).
    pub(crate) enrolled: bool,
    pub(crate) items: Vec<ApprovalItem>,
}

/// SHA-256 of a card's canonical text, `title + "\n\n" + action`, lower-case hex.
pub(crate) fn card_digest(title: &str, action: &str) -> String {
    hex(&Sha256::digest(format!("{title}\n\n{action}").as_bytes()))
}

// ── Where the pending cards come from ────────────────────────────────────

/// Every card waiting for an answer, oldest first. Tests read a list they set
/// instead of the process-wide one, so a card another test shows never leaks in.
#[cfg(not(test))]
pub(crate) fn open_cards() -> Vec<crate::approvals::PendingCard> {
    crate::approvals::pending_cards()
}

#[cfg(test)]
pub(crate) fn open_cards() -> Vec<crate::approvals::PendingCard> {
    TEST_CARDS.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

#[cfg(test)]
pub(crate) static TEST_CARDS: Mutex<Vec<crate::approvals::PendingCard>> = Mutex::new(Vec::new());

/// Tests that set [`TEST_CARDS`] hold this, so they don't see each other's.
#[cfg(test)]
pub(crate) static TEST_CARDS_LOCK: Mutex<()> = Mutex::new(());

fn item_of(ctx: &Ctx, card: crate::approvals::PendingCard) -> ApprovalItem {
    let chat_title = card
        .session_key
        .as_deref()
        .and_then(|key| routes::live_chat(&ctx.vault, key).ok())
        .and_then(|summary| summary.title)
        .filter(|t| !t.trim().is_empty());
    ApprovalItem {
        digest: card_digest(&card.title, &card.action),
        id: card.id,
        session_key: card.session_key,
        chat_title,
        title: card.title,
        action: card.action,
        kind: card.kind,
        created_at: card.at,
        expires_at: card.expires_at,
    }
}

/// `GET /v1/approvals`: the pending cards, oldest first, for every phone;
/// answering needs a key, reading doesn't.
pub(crate) async fn list(ctx: &Arc<Ctx>, device: &Device) -> Result<Reply, ApiError> {
    let (ctx, device_id) = (ctx.clone(), device.device_id.clone());
    let body = routes::blocking(move || {
        let enrolled = ctx
            .store
            .read()
            .map_err(ApiError::internal)?
            .devices
            .iter()
            .any(|d| d.device_id == device_id && d.approval_keys.is_some());
        let items = open_cards()
            .into_iter()
            .map(|card| item_of(&ctx, card))
            .collect();
        Ok(ApprovalsBody { enrolled, items })
    })
    .await?;
    Ok(json(200, &body))
}

// ── Enrollment ───────────────────────────────────────────────────────────

fn rfc3339(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// `POST /v1/approvals/key/challenge`: a one-use challenge, while the Mac has
/// an approvals code window open for this phone.
pub(crate) async fn key_challenge(ctx: &Arc<Ctx>, device: &Device) -> Result<Reply, ApiError> {
    let now = Utc::now();
    let (challenge, ends) = ctx
        .approval_enroll
        .challenge(&device.device_id, now)
        .ok_or_else(ApiError::approvals_closed)?;
    Ok(json(
        200,
        &ChallengeBody {
            challenge,
            expires_at: rfc3339(ends),
        },
    ))
}

fn base64url_point(text: &str, what: &str) -> Result<Vec<u8>, ApiError> {
    // Canonical base64url only (no padding, no stray trailing bits): PR 2
    // compares keys by their text, so two spellings of one key must not exist.
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .ok()
        .filter(|b| URL_SAFE_NO_PAD.encode(b) == text && app_attest::p256_point_valid(b))
        .ok_or_else(|| ApiError::bad_request(format!("{what} isn't a P-256 key.")))?;
    Ok(bytes)
}

/// `POST /v1/approvals/key`: enroll the phone's two keys.
pub(crate) async fn key_enroll(
    ctx: &Arc<Ctx>,
    device: &Device,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: EnrollRequest = routes::parse_body(body)?;
    let field = |value: &Option<String>, what: &str| -> Result<String, ApiError> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .ok_or_else(|| ApiError::bad_request(format!("{what} is missing.")))
    };
    let approve_text = field(&request.approve_key, "approveKey")?;
    let deny_text = field(&request.deny_key, "denyKey")?;
    let challenge = field(&request.challenge, "challenge")?;
    let key_id = field(&request.key_id, "keyId")?;
    let attestation = field(&request.attestation, "attestation")?;
    let proof = field(&request.proof, "proof")?;
    if attestation.len() > ATTESTATION_MAX * 2 {
        return Err(ApiError::too_large("The attestation is too large."));
    }

    // 1 and 2: the code window, the challenge, the proof.
    let message = enroll_message(&device.device_id, &approve_text, &deny_text, &challenge);
    let now = Utc::now();
    match ctx
        .approval_enroll
        .prove(&device.device_id, &challenge, &message, &proof, now)
    {
        Proof::Closed => return Err(ApiError::approvals_closed()),
        Proof::ChallengeGone => return Err(ApiError::challenge_expired()),
        Proof::Wrong { attempts_left } => return Err(ApiError::wrong_code(attempts_left)),
        Proof::Right => {}
    }

    // 3: both keys are valid P-256 points, and two different keys.
    let approve = base64url_point(&approve_text, "approveKey")?;
    let deny = base64url_point(&deny_text, "denyKey")?;
    if approve == deny {
        return Err(ApiError::bad_request(
            "The approve key and the deny key must be different keys.",
        ));
    }

    // 4: the attestation. The decoded key id is a 32-byte SHA-256.
    let key_id_bytes = STANDARD
        .decode(&key_id)
        .ok()
        .filter(|k| k.len() == 32)
        .ok_or_else(|| ApiError::bad_request("keyId isn't an App Attest key id."))?;
    let object = URL_SAFE_NO_PAD
        .decode(attestation.trim_end_matches('='))
        .ok()
        .filter(|o| o.len() <= ATTESTATION_MAX)
        .ok_or_else(|| ApiError::bad_request("attestation isn't base64url."))?;
    let expected = Expected {
        app_id: APP_ID,
        key_id: &key_id_bytes,
        client_data_hash: Sha256::digest(message.as_bytes()).into(),
        now: now.timestamp(),
        root: ctx.attest_root,
    };
    if let Err(why) = app_attest::verify_attestation(&object, &expected) {
        ctx.approval_enroll.attestation_failed(&device.device_id);
        return Err(ApiError::bad_attestation(why.0));
    }

    // 5: keep the keys, close the window (one use).
    let keys = ApprovalKeys {
        approve_key: approve_text,
        deny_key: deny_text,
        fingerprint: fingerprint(&approve, &deny),
        enrolled_at: rfc3339(now),
    };
    let (store, device_id, kept) = (ctx.store.clone(), device.device_id.clone(), keys.clone());
    let live = routes::blocking(move || {
        store
            .set_approval_keys(&device_id, Some(kept))
            .map_err(ApiError::internal)
    })
    .await?;
    if !live {
        return Err(ApiError::unauthorized());
    }
    ctx.approval_enroll.close_for(&device.device_id);
    ctx.approval_decisions.clear_dropped(&device.device_id);
    super::approval_keys_changed(ctx);
    ctx.host.changed();
    Ok(json(
        200,
        &EnrolledBody {
            fingerprint: keys.fingerprint,
            enrolled_at: keys.enrolled_at,
        },
    ))
}

/// `DELETE /v1/approvals/key`: this phone turns approvals off.
pub(crate) async fn key_forget(ctx: &Arc<Ctx>, device: &Device) -> Result<Reply, ApiError> {
    let (store, device_id) = (ctx.store.clone(), device.device_id.clone());
    let live = routes::blocking(move || {
        store
            .set_approval_keys(&device_id, None)
            .map_err(ApiError::internal)
    })
    .await?;
    if !live {
        return Err(ApiError::unauthorized());
    }
    ctx.approval_decisions.forget(&device.device_id);
    super::approval_keys_changed(ctx);
    ctx.host.changed();
    Ok(Reply::Empty(204))
}

// ── Decisions (PR 2) ─────────────────────────────────────────────────────

struct OpenChallenge {
    nonce: String,
    card_id: String,
    expires: DateTime<Utc>,
    /// The `expiresAt` text handed out, which the signature covers as sent.
    expires_text: String,
}

/// A phone's open decision challenges and its bad signatures, in memory.
#[derive(Default)]
pub(crate) struct Decisions {
    open: Mutex<HashMap<String, Vec<OpenChallenge>>>,
    bad: Mutex<HashMap<String, Vec<DateTime<Utc>>>>,
    /// Phones whose keys were dropped for bad signatures, for the Mac's row.
    dropped: Mutex<HashMap<String, DateTime<Utc>>>,
}

impl Decisions {
    /// A challenge for `card_id` for `device_id`: at most 16 open per phone
    /// (the oldest goes), each used once, 60 seconds.
    pub(crate) fn issue(
        &self,
        device_id: &str,
        card_id: &str,
        now: DateTime<Utc>,
    ) -> (String, String) {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let nonce = URL_SAFE_NO_PAD.encode(bytes);
        let expires = now + Duration::seconds(DECISION_CHALLENGE_SECS);
        let expires_text = rfc3339(expires);
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let list = open.entry(device_id.to_string()).or_default();
        list.retain(|c| c.expires > now);
        while list.len() >= CHALLENGES_PER_PHONE {
            list.remove(0);
        }
        list.push(OpenChallenge {
            nonce: nonce.clone(),
            card_id: card_id.to_string(),
            expires,
            expires_text: expires_text.clone(),
        });
        (nonce, expires_text)
    }

    /// The `expiresAt` of `device_id`'s open challenge `nonce` for `card_id`
    /// (without using it), or `None`: unknown, another card's, used or ended.
    fn peek(
        &self,
        device_id: &str,
        nonce: &str,
        card_id: &str,
        now: DateTime<Utc>,
    ) -> Option<String> {
        let open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        open.get(device_id)?
            .iter()
            .find(|c| {
                c.expires > now
                    && c.card_id == card_id
                    && same_bytes(c.nonce.as_bytes(), nonce.as_bytes())
            })
            .map(|c| c.expires_text.clone())
    }

    /// Use the challenge up (once): `false` when it was already used.
    fn consume(&self, device_id: &str, nonce: &str, card_id: &str, now: DateTime<Utc>) -> bool {
        let mut open = self.open.lock().unwrap_or_else(|e| e.into_inner());
        let Some(list) = open.get_mut(device_id) else {
            return false;
        };
        let before = list.len();
        list.retain(|c| {
            !(c.expires > now
                && c.card_id == card_id
                && same_bytes(c.nonce.as_bytes(), nonce.as_bytes()))
        });
        list.len() < before
    }

    /// Tests: [`Self::consume`] at a chosen time.
    #[cfg(test)]
    pub(crate) fn consume_for_test(
        &self,
        device_id: &str,
        nonce: &str,
        card_id: &str,
        now: DateTime<Utc>,
    ) -> bool {
        self.consume(device_id, nonce, card_id, now)
    }

    /// A bad signature from `device_id`; `true` once it is the fifth within
    /// ten minutes (the caller then drops the phone's keys).
    fn note_bad(&self, device_id: &str, now: DateTime<Utc>) -> bool {
        let mut bad = self.bad.lock().unwrap_or_else(|e| e.into_inner());
        let list = bad.entry(device_id.to_string()).or_default();
        list.retain(|at| now - *at < Duration::minutes(BAD_SIGNATURE_MINUTES));
        list.push(now);
        if list.len() >= BAD_SIGNATURES_MAX {
            list.clear();
            self.dropped
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(device_id.to_string(), now);
            return true;
        }
        false
    }

    /// When `device_id`'s keys were dropped for bad signatures (until it
    /// enrolls again or Hollow restarts), for the Companion devices row.
    #[cfg(test)]
    pub(crate) fn dropped_at(&self, device_id: &str) -> Option<DateTime<Utc>> {
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(device_id)
            .copied()
    }

    /// Every phone whose keys were dropped for bad signatures: `(device id, RFC 3339)`.
    pub(crate) fn dropped_list(&self) -> Vec<(String, String)> {
        let mut list: Vec<(String, String)> = self
            .dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(id, at)| (id.clone(), rfc3339(*at)))
            .collect();
        list.sort();
        list
    }

    /// Forget a drop note (the phone enrolled again, or the Mac turned approvals off).
    pub(crate) fn clear_dropped(&self, device_id: &str) {
        self.dropped
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(device_id);
    }

    /// Forget everything about a phone (revoked).
    pub(crate) fn forget(&self, device_id: &str) {
        self.open
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(device_id);
        self.bad
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(device_id);
        self.clear_dropped(device_id);
    }
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DecisionChallengeBody {
    pub(crate) nonce: String,
    pub(crate) expires_at: String,
}

#[derive(Debug, Deserialize)]
struct DecisionRequest {
    #[serde(default)]
    decision: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    signature: Option<String>,
}

#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct ResolvedBody {
    pub(crate) resolved: &'static str,
}

/// The message a decision signs (UTF-8, lines joined by `\n`, no trailing
/// newline); `expires_at` is the challenge's text exactly as handed out.
pub(crate) fn decision_message(
    device_id: &str,
    card_id: &str,
    nonce: &str,
    digest: &str,
    decision: &str,
    expires_at: &str,
) -> String {
    [
        DECISION_PREFIX,
        device_id,
        card_id,
        nonce,
        digest,
        decision,
        expires_at,
    ]
    .join("\n")
}

fn keys_of(ctx: &Ctx, device_id: &str) -> Result<Option<ApprovalKeys>, ApiError> {
    Ok(ctx
        .store
        .read()
        .map_err(ApiError::internal)?
        .devices
        .into_iter()
        .find(|d| d.device_id == device_id)
        .and_then(|d| d.approval_keys))
}

fn card_by_id(id: &str) -> Option<crate::approvals::PendingCard> {
    open_cards().into_iter().find(|card| card.id == id)
}

/// `POST /v1/approvals/{id}/challenge`: a one-use, 60-second nonce for
/// answering this card. Needs the phone's approval keys.
pub(crate) async fn decision_challenge(
    ctx: &Arc<Ctx>,
    device: &Device,
    id: &str,
) -> Result<Reply, ApiError> {
    let (ctx, device_id, id) = (ctx.clone(), device.device_id.clone(), id.to_string());
    let body = routes::blocking(move || {
        if keys_of(&ctx, &device_id)?.is_none() {
            return Err(ApiError::approval_key_required());
        }
        if card_by_id(&id).is_none() {
            return Err(ApiError::approval_closed());
        }
        let (nonce, expires_at) = ctx.approval_decisions.issue(&device_id, &id, Utc::now());
        Ok(DecisionChallengeBody { nonce, expires_at })
    })
    .await?;
    Ok(json(200, &body))
}

/// `POST /v1/approvals/{id}/decision`: answer the card with a signature.
///
/// Checked in this order, stopping at the first failure: the phone has keys
/// (403 `approval_key_required`); the nonce is its own open challenge for this
/// card (409 `approval_expired`); the card is still pending (409
/// `approval_closed`); the sent digest is the card's current one (409
/// `approval_changed`); then the signature over the message Hollow rebuilds
/// from its own copy of the challenge (400 `bad_signature`, five in ten minutes
/// drop the keys). The nonce is used up before the card resolves, and the
/// first answer from the window or any phone wins.
pub(crate) async fn decide(
    ctx: &Arc<Ctx>,
    device: &Device,
    id: &str,
    body: &[u8],
) -> Result<Reply, ApiError> {
    let request: DecisionRequest = routes::parse_body(body)?;
    let allow = match request.decision.as_deref() {
        Some("allow") => true,
        Some("deny") => false,
        _ => return Err(ApiError::bad_request("decision must be allow or deny.")),
    };
    let word = if allow { "allow" } else { "deny" };
    let text = |value: &Option<String>, what: &str| -> Result<String, ApiError> {
        value
            .clone()
            .filter(|v| !v.is_empty())
            .ok_or_else(|| ApiError::bad_request(format!("{what} is missing.")))
    };
    let nonce = text(&request.nonce, "nonce")?;
    let digest = text(&request.digest, "digest")?;
    let signature = text(&request.signature, "signature")?;
    let (inner, device, id) = (ctx.clone(), device.clone(), id.to_string());
    let resolved = routes::blocking(move || {
        let ctx = inner;
        let now = Utc::now();
        let device_id = device.device_id.as_str();
        let keys = keys_of(&ctx, device_id)?.ok_or_else(ApiError::approval_key_required)?;
        let expires_at = ctx
            .approval_decisions
            .peek(device_id, &nonce, &id, now)
            .ok_or_else(ApiError::approval_expired)?;
        let card = card_by_id(&id).ok_or_else(ApiError::approval_closed)?;
        if !same_bytes(
            card_digest(&card.title, &card.action).as_bytes(),
            digest.as_bytes(),
        ) {
            return Err(ApiError::approval_changed());
        }
        let message = decision_message(device_id, &id, &nonce, &digest, word, &expires_at);
        let key_text = if allow {
            &keys.approve_key
        } else {
            &keys.deny_key
        };
        let valid = URL_SAFE_NO_PAD
            .decode(key_text)
            .ok()
            .zip(
                URL_SAFE_NO_PAD
                    .decode(&signature)
                    .ok()
                    .filter(|s| s.len() <= SIGNATURE_MAX),
            )
            .is_some_and(|(key, sig)| {
                ring::signature::UnparsedPublicKey::new(
                    &ring::signature::ECDSA_P256_SHA256_ASN1,
                    key,
                )
                .verify(message.as_bytes(), &sig)
                .is_ok()
            });
        if !valid {
            // A bad signature burns its challenge and counts toward the five.
            ctx.approval_decisions.consume(device_id, &nonce, &id, now);
            if ctx.approval_decisions.note_bad(device_id, now) {
                ctx.store
                    .set_approval_keys(device_id, None)
                    .map_err(ApiError::internal)?;
                super::approval_keys_changed(&ctx);
                // The Mac's row says the keys were dropped.
                ctx.host.changed();
            }
            return Err(ApiError::bad_signature());
        }
        if !ctx.approval_decisions.consume(device_id, &nonce, &id, now) {
            return Err(ApiError::approval_expired());
        }
        // The commit point: keys dropped (the Mac's Turn off, five bad
        // signatures) or a phone revoked while this ran get nothing through.
        if keys_of(&ctx, device_id)?.is_none() {
            return Err(ApiError::approval_key_required());
        }
        ctx.store
            .ensure_live(device_id)
            .map_err(|_| ApiError::unauthorized())?;
        let by = AnsweredBy::device(device_id, &device.name, allow, now);
        crate::approvals::respond_from(&id, allow, by).map_err(|_| ApiError::approval_closed())?;
        Ok(if allow { "allowed" } else { "denied" })
    })
    .await?;
    ctx.host.changed();
    Ok(json(200, &ResolvedBody { resolved }))
}
