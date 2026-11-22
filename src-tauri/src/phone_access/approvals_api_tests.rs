//! The v1.2 approval routes over real HTTP, against a temporary vault
//! (`docs/phone-api/approvals.md`): enrollment of a phone's keys behind the
//! Mac's code and an App Attest attestation, and the read of pending cards.
//! The attestation check itself is `app_attest_tests.rs`'s.
use super::app_attest_tests::{attest, pki, Opts};
use super::*;
use crate::approvals::PendingCard;
use crate::phone_access::approvals_api::{self, card_digest, enroll_message, Enroll, Proof};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use ring::hmac;
use ring::rand::SystemRandom;
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
use sha2::{Digest, Sha256};

const CODE: &str = "ABCD2345";

fn point() -> String {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    URL_SAFE_NO_PAD.encode(pair.public_key().as_ref())
}

fn proof_of(code: &str, message: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, code.as_bytes());
    hmac::sign(&key, message.as_bytes())
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

struct Phone {
    device_id: String,
    token: String,
}

fn pair(rig: &Rig) -> Phone {
    let (device, token) = rig.store.pair("Test iPhone", None, Utc::now()).unwrap();
    Phone {
        device_id: device.device_id,
        token,
    }
}

fn open(rig: &Rig, phone: &Phone) {
    rig.ctx
        .approval_enroll
        .open_with(&phone.device_id, CODE, Utc::now());
}

async fn challenge(rig: &Rig, phone: &Phone) -> String {
    let resp = rig
        .call(
            "POST",
            "/v1/approvals/key/challenge",
            Some(&phone.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    assert!(resp.json()["expiresAt"].as_str().unwrap().ends_with('Z'));
    resp.json()["challenge"].as_str().unwrap().to_string()
}

/// A full enrollment body for `challenge`, with `opts` shaping the attestation.
fn body(phone: &Phone, challenge: &str, code: &str, opts: &Opts) -> Value {
    let (approve, deny) = (point(), point());
    let message = enroll_message(&phone.device_id, &approve, &deny, challenge);
    let (object, key_id) = attest(pki(), &message, opts);
    json!({
        "approveKey": approve,
        "denyKey": deny,
        "challenge": challenge,
        "keyId": key_id,
        "attestation": URL_SAFE_NO_PAD.encode(object),
        "proof": proof_of(code, &message),
    })
}

async fn enroll(rig: &Rig, phone: &Phone, body: Value) -> Resp {
    rig.call("POST", "/v1/approvals/key", Some(&phone.token), Some(body))
        .await
}

fn keys_of(rig: &Rig, phone: &Phone) -> Option<store::ApprovalKeys> {
    rig.store
        .read()
        .unwrap()
        .devices
        .into_iter()
        .find(|d| d.device_id == phone.device_id)
        .unwrap()
        .approval_keys
}

fn error_code(resp: &Resp) -> String {
    resp.json()["error"]["code"].as_str().unwrap().to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_enrolls_with_the_code_and_a_genuine_attestation() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let challenge = challenge(&rig, &phone).await;
    let sent = body(&phone, &challenge, CODE, &Opts::default());
    let resp = enroll(&rig, &phone, sent.clone()).await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let keys = keys_of(&rig, &phone).expect("the keys are kept");
    assert_eq!(keys.approve_key, sent["approveKey"]);
    assert_eq!(keys.deny_key, sent["denyKey"]);
    // The fingerprint: 8 hex digits of SHA-256(approve point || deny point).
    let raw = |k: &str| URL_SAFE_NO_PAD.decode(sent[k].as_str().unwrap()).unwrap();
    let digest = Sha256::digest([raw("approveKey"), raw("denyKey")].concat());
    let hex: String = digest[..4].iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(keys.fingerprint, format!("{} {}", &hex[..4], &hex[4..]));
    assert_eq!(resp.json()["fingerprint"], keys.fingerprint);
    // One use: the window closed, so the same body, or a new challenge, is refused.
    let again = enroll(&rig, &phone, sent).await;
    assert_eq!(
        (again.status, error_code(&again).as_str()),
        (403, "approvals_closed")
    );
    let more = rig
        .call(
            "POST",
            "/v1/approvals/key/challenge",
            Some(&phone.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(more.status, 403);
    // The list says this phone may answer.
    let list = rig
        .call("GET", "/v1/approvals", Some(&phone.token), None)
        .await;
    assert_eq!(list.json()["enrolled"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bearer_token_alone_enrolls_nothing() {
    let rig = rig().await;
    let phone = pair(&rig);
    // No window at all.
    for path in ["/v1/approvals/key/challenge", "/v1/approvals/key"] {
        let resp = rig
            .call(
                "POST",
                path,
                Some(&phone.token),
                Some(body(&phone, "x", CODE, &Opts::default())),
            )
            .await;
        assert_eq!(resp.status, 403, "{path}");
    }
    // A window for another phone is no window for this one.
    let other = pair(&rig);
    open(&rig, &other);
    let resp = rig
        .call(
            "POST",
            "/v1/approvals/key/challenge",
            Some(&phone.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(
        (resp.status, error_code(&resp).as_str()),
        (403, "approvals_closed")
    );
    assert!(keys_of(&rig, &phone).is_none());
    // No token at all.
    let none = rig
        .call("POST", "/v1/approvals/key/challenge", None, Some(json!({})))
        .await;
    assert_eq!(none.status, 401);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_wrong_proofs_close_the_window() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    for left in [4, 3, 2, 1, 0] {
        let challenge = challenge(&rig, &phone).await;
        let resp = enroll(
            &rig,
            &phone,
            body(&phone, &challenge, "WRONG222", &Opts::default()),
        )
        .await;
        assert_eq!(
            (resp.status, error_code(&resp).as_str()),
            (403, "wrong_code")
        );
        assert_eq!(resp.json()["error"]["attemptsLeft"], left);
    }
    let closed = rig
        .call(
            "POST",
            "/v1/approvals/key/challenge",
            Some(&phone.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(error_code(&closed), "approvals_closed");
    assert!(keys_of(&rig, &phone).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_challenge_is_one_use_and_must_be_the_one_handed_out() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let first = challenge(&rig, &phone).await;
    // A made-up challenge.
    let made_up = enroll(
        &rig,
        &phone,
        body(&phone, "bm90LW1pbmU", CODE, &Opts::default()),
    )
    .await;
    assert_eq!(
        (made_up.status, error_code(&made_up).as_str()),
        (409, "challenge_expired")
    );
    // That attempt used the open challenge up, so the real one is gone too.
    let late = enroll(&rig, &phone, body(&phone, &first, CODE, &Opts::default())).await;
    assert_eq!(error_code(&late), "challenge_expired");
    // A new one works; a second challenge replaces the first.
    let stale = challenge(&rig, &phone).await;
    let fresh = challenge(&rig, &phone).await;
    assert_ne!(stale, fresh);
    let replaced = enroll(&rig, &phone, body(&phone, &stale, CODE, &Opts::default())).await;
    assert_eq!(error_code(&replaced), "challenge_expired");
    let fresh = challenge(&rig, &phone).await;
    let ok = enroll(&rig, &phone, body(&phone, &fresh, CODE, &Opts::default())).await;
    assert_eq!(ok.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_window_and_challenge_expire() {
    let enroll = Enroll::default();
    let t0 = Utc::now();
    enroll.open_with("ph-00000001", CODE, t0);
    let (challenge, ends) = enroll.challenge("ph-00000001", t0).unwrap();
    assert_eq!(ends, t0 + chrono::Duration::seconds(60));
    let message = "m";
    let proof = proof_of(CODE, message);
    // 61 seconds on: the challenge is gone, the window still open.
    let late = t0 + chrono::Duration::seconds(61);
    assert_eq!(
        enroll.prove("ph-00000001", &challenge, message, &proof, late),
        Proof::ChallengeGone
    );
    // 11 minutes on: the window is gone.
    let (challenge, _) = enroll.challenge("ph-00000001", t0).unwrap();
    let much_later = t0 + chrono::Duration::minutes(11);
    assert_eq!(
        enroll.prove("ph-00000001", &challenge, message, &proof, much_later),
        Proof::Closed
    );
    assert!(enroll.current(much_later).is_none());
    // And a typed code in either case or with a dash proves the same.
    enroll.open_with("ph-00000001", "abcd-2345", t0);
    let (challenge, _) = enroll.challenge("ph-00000001", t0).unwrap();
    assert_eq!(
        enroll.prove("ph-00000001", &challenge, message, &proof, t0),
        Proof::Right
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_attestation_enrolls_nothing_and_leaves_the_window_open() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let cases = [
        Opts {
            app_id: "ABCDE12345.com.example.other".into(),
            ..Opts::default()
        },
        Opts {
            nonce_for: Some("another message".into()),
            ..Opts::default()
        },
        Opts {
            category: Some(3),
            ..Opts::default()
        },
        Opts {
            counter: 7,
            ..Opts::default()
        },
    ];
    for opts in cases {
        let challenge = challenge(&rig, &phone).await;
        let resp = enroll(&rig, &phone, body(&phone, &challenge, CODE, &opts)).await;
        assert_eq!(
            (resp.status, error_code(&resp).as_str()),
            (400, "bad_attestation")
        );
        assert!(keys_of(&rig, &phone).is_none());
    }
    // Four refusals so far: a fifth chance is left.
    let challenge = challenge(&rig, &phone).await;
    let ok = enroll(
        &rig,
        &phone,
        body(&phone, &challenge, CODE, &Opts::default()),
    )
    .await;
    assert_eq!(ok.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_keys_and_missing_fields_are_refused() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let mut mutations: Vec<Box<dyn Fn(&mut Value)>> = vec![
        Box::new(|b| b["approveKey"] = json!("AAAA")),
        Box::new(|b| b["denyKey"] = b["approveKey"].clone()),
        Box::new(|b| {
            // A point that was valid with one coordinate nudged.
            let mut raw = URL_SAFE_NO_PAD
                .decode(b["approveKey"].as_str().unwrap())
                .unwrap();
            raw[64] ^= 1;
            b["approveKey"] = json!(URL_SAFE_NO_PAD.encode(raw));
        }),
        Box::new(|b| b["keyId"] = json!("c2hvcnQ=")),
        Box::new(|b| b["attestation"] = json!("!!!")),
    ];
    for field in [
        "approveKey",
        "denyKey",
        "challenge",
        "keyId",
        "attestation",
        "proof",
    ] {
        mutations.push(Box::new(move |b| {
            b.as_object_mut().unwrap().remove(field);
        }));
    }
    for (i, mutate) in mutations.iter().enumerate() {
        let challenge = challenge(&rig, &phone).await;
        let mut sent = body(&phone, &challenge, CODE, &Opts::default());
        // Re-prove after a key edit so the failure is the key's, not the proof's.
        mutate(&mut sent);
        if let (Some(a), Some(d), Some(c)) = (
            sent["approveKey"].as_str(),
            sent["denyKey"].as_str(),
            sent["challenge"].as_str(),
        ) {
            let message = enroll_message(&phone.device_id, a, d, c);
            let (object, key_id) = attest(pki(), &message, &Opts::default());
            // Fix up what the edit didn't mean to break (and leave a removed or
            // deliberately bad field as it is).
            let fix = |sent: &mut Value, field: &str, bad: Option<&str>, to: Value| {
                if sent.get(field).is_some() && Some(sent[field].as_str().unwrap_or("")) != bad {
                    sent[field] = to;
                }
            };
            fix(&mut sent, "keyId", Some("c2hvcnQ="), json!(key_id));
            fix(
                &mut sent,
                "attestation",
                Some("!!!"),
                json!(URL_SAFE_NO_PAD.encode(object)),
            );
            fix(&mut sent, "proof", None, json!(proof_of(CODE, &message)));
        }
        let resp = enroll(&rig, &phone, sent).await;
        assert_eq!(
            resp.status,
            400,
            "case {i}: {}",
            String::from_utf8_lossy(&resp.body)
        );
        assert!(keys_of(&rig, &phone).is_none(), "case {i}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn turning_off_drops_the_keys_and_a_revoked_phone_is_refused() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let challenge = challenge(&rig, &phone).await;
    assert_eq!(
        enroll(
            &rig,
            &phone,
            body(&phone, &challenge, CODE, &Opts::default())
        )
        .await
        .status,
        200
    );
    assert!(keys_of(&rig, &phone).is_some());
    let off = rig
        .call("DELETE", "/v1/approvals/key", Some(&phone.token), None)
        .await;
    assert_eq!(off.status, 204);
    assert!(keys_of(&rig, &phone).is_none());
    // Turning off twice is fine; a revoked phone gets 401 on every route.
    assert_eq!(
        rig.call("DELETE", "/v1/approvals/key", Some(&phone.token), None)
            .await
            .status,
        204
    );
    rig.store.revoke(&phone.device_id).unwrap();
    for (method, path) in [
        ("GET", "/v1/approvals"),
        ("POST", "/v1/approvals/key/challenge"),
        ("DELETE", "/v1/approvals/key"),
    ] {
        let resp = rig
            .call(method, path, Some(&phone.token), Some(json!({})))
            .await;
        assert_eq!(resp.status, 401, "{method} {path}");
    }
}

#[test]
fn an_older_phone_file_without_the_field_reads() {
    let old = json!({
        "deviceId": "ph-0a1b2c3d",
        "name": "iPhone",
        "tokenSha256": "00",
        "pairedAt": "2026-11-01T00:00:00Z",
        "someLaterField": 7,
    });
    let device: store::Device = serde_json::from_value(old).unwrap();
    assert!(device.approval_keys.is_none());
    assert_eq!(device.extra["someLaterField"], 7);
    // Written back, the empty field is absent.
    assert!(serde_json::to_value(&device)
        .unwrap()
        .get("approvalKeys")
        .is_none());
}

fn card(id: &str, title: &str, action: &str, key: Option<&str>) -> PendingCard {
    PendingCard {
        id: id.into(),
        title: title.into(),
        action: action.into(),
        kind: "app".into(),
        session_key: key.map(str::to_string),
        tool_use_id: None,
        at: "2026-11-19T09:00:00.000Z".into(),
        expires_at: "2026-11-19T09:02:00.000Z".into(),
        routed_to_coo: false,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_cards_are_listed_with_a_digest_and_wait_in_the_waiting_list() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let phone = pair(&rig);
    let empty = rig
        .call("GET", "/v1/approvals", Some(&phone.token), None)
        .await;
    assert_eq!(empty.json(), json!({"enrolled": false, "items": []}));

    *approvals_api::TEST_CARDS.lock().unwrap() = vec![
        card(
            "apr-1a2b3c4d-1",
            "Install Hollow",
            "Commit abc123\n3 files changed",
            None,
        ),
        card("apr-1a2b3c4d-2", "Quit Safari", "Quit an app: Safari", None),
    ];
    let list = rig
        .call("GET", "/v1/approvals", Some(&phone.token), None)
        .await;
    assert_eq!(list.status, 200);
    let items = list.json()["items"].as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["id"], "apr-1a2b3c4d-1");
    assert_eq!(items[0]["sessionKey"], Value::Null);
    assert_eq!(items[0]["kind"], "app");
    assert_eq!(items[0]["createdAt"], "2026-11-19T09:00:00.000Z");
    // The digest is SHA-256 of `title + "\n\n" + action`.
    let expect = Sha256::digest("Install Hollow\n\nCommit abc123\n3 files changed".as_bytes());
    let hex: String = expect.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(items[0]["digest"], hex);
    assert_eq!(
        items[0]["digest"],
        card_digest("Install Hollow", "Commit abc123\n3 files changed")
    );

    // Waiting on you carries one `approval` item per card.
    let waiting = rig
        .call("GET", "/v1/waiting", Some(&phone.token), None)
        .await;
    let waiting = waiting.json();
    let kinds: Vec<&str> = waiting["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["approval", "approval"]);
    assert_eq!(waiting["items"][0]["questionId"], "apr-1a2b3c4d-1");
    assert_eq!(waiting["items"][0]["preview"], "Install Hollow");
    *approvals_api::TEST_CARDS.lock().unwrap() = Vec::new();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_approval_push_goes_only_to_a_phone_that_can_answer() {
    let rig = rig().await;
    let (with, without) = (pair(&rig), pair(&rig));
    for phone in [&with, &without] {
        rig.store
            .set_push(
                &phone.device_id,
                Some(store::PushTarget {
                    token: format!("{:0>64}", phone.device_id.replace("ph-", "")),
                    environment: "production".into(),
                    events: push::known_events(None),
                    registered_at: "2026-11-19T00:00:00Z".into(),
                }),
            )
            .unwrap();
    }
    rig.store
        .set_approval_keys(
            &with.device_id,
            Some(store::ApprovalKeys {
                approve_key: "a".into(),
                deny_key: "d".into(),
                fingerprint: "0000 0000".into(),
                enrolled_at: "2026-11-19T00:00:00Z".into(),
            }),
        )
        .unwrap();
    let message = push::Message {
        key: None,
        kind: push::APPROVAL,
        title: "Chat".into(),
        body: "Install Hollow".into(),
        asked: false,
        card: None,
    };
    let devices = rig.store.read().unwrap().devices;
    let wanted: Vec<&str> = devices
        .iter()
        .filter(|d| message.wanted_by(d))
        .map(|d| d.device_id.as_str())
        .collect();
    assert_eq!(wanted, [with.device_id.as_str()]);
    assert!(push::known_events(None).contains(&"approval".to_string()));
    // A phone that registered for the older two events never gets one.
    let older = push::known_events(Some(&["turnFinished".to_string(), "question".to_string()]));
    assert!(!older.contains(&"approval".to_string()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_card_appearing_is_told_to_every_stream() {
    let rig = rig().await;
    let mut rx = rig.live.open("ph-00000001", None, Instant::now());
    while rx.try_recv().is_some() {}
    rig.live.approvals_changed();
    let frame = rx.try_recv().expect("an approvals frame");
    let text = String::from_utf8_lossy(&frame).to_string();
    assert!(text.contains("event: approvals"), "{text}");
}

#[test]
fn the_bodies_are_the_fixtures() {
    let item = approvals_api::ApprovalItem {
        id: "apr-1a2b3c4d-1".into(),
        session_key: Some("s-20261104-e2582579".into()),
        chat_title: Some("Ship the release".into()),
        title: "Install Hollow 5.2.1?".into(),
        action: "Install the built app, commit 9f3c2ab.\n12 files changed against the installed build.\nsrc-tauri/src/approvals.rs\nand 11 more".into(),
        kind: "app".into(),
        created_at: "2026-11-19T09:00:00.000Z".into(),
        expires_at: "2026-11-19T09:02:00.000Z".into(),
        digest: "7d0c5f2b8e5a1c3d94a6b7e8f0123456789abcdef0123456789abcdef0123456".into(),
    };
    let list = approvals_api::ApprovalsBody {
        enrolled: true,
        items: vec![item],
    };
    assert_eq!(value(&list), fixture("approvals/list.json"));
    let challenge = approvals_api::ChallengeBody {
        challenge: "q3Vn0y8mK2dTq1xv7oWb4Lr9sPzE5cHhJaGfUiYtNkA".into(),
        expires_at: "2026-11-19T09:01:00Z".into(),
    };
    assert_eq!(value(&challenge), fixture("approvals/challenge.json"));
    let enrolled = approvals_api::EnrolledBody {
        fingerprint: "a1b2 c3d4".into(),
        enrolled_at: "2026-11-19T09:00:30Z".into(),
    };
    assert_eq!(value(&enrolled), fixture("approvals/enroll-response.json"));
    for (error, file) in [
        (
            routes::ApiError::approvals_closed(),
            "error-approvals-closed.json",
        ),
        (
            routes::ApiError::challenge_expired(),
            "error-challenge-expired.json",
        ),
        (
            routes::ApiError::bad_attestation("the attestation is for another app"),
            "error-bad-attestation.json",
        ),
    ] {
        assert_eq!(value(&error.body()), fixture(&format!("approvals/{file}")));
    }
    // The request fixture parses as the request the route reads.
    let request = fixture("approvals/enroll-request.json");
    for field in [
        "approveKey",
        "denyKey",
        "challenge",
        "keyId",
        "attestation",
        "proof",
    ] {
        assert!(request[field].is_string(), "{field}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_refused_attestations_close_the_window() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let bad = Opts {
        category: Some(3),
        ..Opts::default()
    };
    for _ in 0..5 {
        let challenge = challenge(&rig, &phone).await;
        let resp = enroll(&rig, &phone, body(&phone, &challenge, CODE, &bad)).await;
        assert_eq!(error_code(&resp), "bad_attestation");
    }
    let closed = rig
        .call(
            "POST",
            "/v1/approvals/key/challenge",
            Some(&phone.token),
            Some(json!({})),
        )
        .await;
    assert_eq!(error_code(&closed), "approvals_closed");
    assert!(keys_of(&rig, &phone).is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_must_be_spelled_canonically() {
    let rig = rig().await;
    let phone = pair(&rig);
    open(&rig, &phone);
    let challenge = challenge(&rig, &phone).await;
    let mut sent = body(&phone, &challenge, CODE, &Opts::default());
    // The same key with padding added: a different spelling, so refused.
    let padded = format!("{}=", sent["approveKey"].as_str().unwrap());
    sent["approveKey"] = json!(padded);
    let message = enroll_message(
        &phone.device_id,
        sent["approveKey"].as_str().unwrap(),
        sent["denyKey"].as_str().unwrap(),
        &challenge,
    );
    let (object, key_id) = attest(pki(), &message, &Opts::default());
    sent["keyId"] = json!(key_id);
    sent["attestation"] = json!(URL_SAFE_NO_PAD.encode(object));
    sent["proof"] = json!(proof_of(CODE, &message));
    let resp = enroll(&rig, &phone, sent).await;
    assert_eq!(resp.status, 400);
    assert!(keys_of(&rig, &phone).is_none());
}

// ── Decisions (PR 2) ─────────────────────────────────────────────────────

struct Keys {
    approve: EcdsaKeyPair,
    deny: EcdsaKeyPair,
}

fn sw_key() -> (EcdsaKeyPair, String) {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let text = URL_SAFE_NO_PAD.encode(pair.public_key().as_ref());
    (pair, text)
}

/// A paired phone with approval keys stored (a software P-256 key pair per
/// key, which signs the DER the Secure Enclave returns).
fn enrolled(rig: &Rig) -> (Phone, Keys) {
    let phone = pair(rig);
    let ((approve, a_text), (deny, d_text)) = (sw_key(), sw_key());
    rig.store
        .set_approval_keys(
            &phone.device_id,
            Some(store::ApprovalKeys {
                approve_key: a_text,
                deny_key: d_text,
                fingerprint: "0000 0000".into(),
                enrolled_at: "2026-11-19T00:00:00Z".into(),
            }),
        )
        .unwrap();
    (phone, Keys { approve, deny })
}

fn sign(key: &EcdsaKeyPair, message: &str) -> String {
    let rng = SystemRandom::new();
    URL_SAFE_NO_PAD.encode(key.sign(&rng, message.as_bytes()).unwrap().as_ref())
}

fn set_cards(cards: Vec<PendingCard>) {
    *approvals_api::TEST_CARDS.lock().unwrap() = cards;
}

async fn decision_challenge(rig: &Rig, phone: &Phone, id: &str) -> Resp {
    rig.call(
        "POST",
        &format!("/v1/approvals/{id}/challenge"),
        Some(&phone.token),
        Some(json!({})),
    )
    .await
}

/// A signed decision for `card`, with `key` signing and `decision` the word sent.
async fn decide(
    rig: &Rig,
    phone: &Phone,
    card: &PendingCard,
    nonce_from: &Resp,
    decision: &str,
    key: &EcdsaKeyPair,
) -> Resp {
    let digest = card_digest(&card.title, &card.action);
    let nonce = nonce_from.json()["nonce"].as_str().unwrap().to_string();
    let expires = nonce_from.json()["expiresAt"].as_str().unwrap().to_string();
    let message = approvals_api::decision_message(
        &phone.device_id,
        &card.id,
        &nonce,
        &digest,
        decision,
        &expires,
    );
    rig.call(
        "POST",
        &format!("/v1/approvals/{}/decision", card.id),
        Some(&phone.token),
        Some(json!({"decision": decision, "nonce": nonce, "digest": digest, "signature": sign(key, &message)})),
    )
    .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_allow_signed_with_the_approve_key_and_a_deny_with_the_deny_key_resolve_the_card() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    let allow_card = card("apr-dec-allow", "Install Hollow", "Commit abc", None);
    let deny_card = card("apr-dec-deny", "Quit Safari", "Quit Safari", None);
    set_cards(vec![allow_card.clone(), deny_card.clone()]);
    let mut allow_rx = crate::approvals::test_register_pending(&allow_card.id);
    let mut deny_rx = crate::approvals::test_register_pending(&deny_card.id);

    let ch = decision_challenge(&rig, &phone, &allow_card.id).await;
    assert_eq!(ch.status, 200);
    assert!(ch.json()["expiresAt"].as_str().unwrap().ends_with('Z'));
    let resp = decide(&rig, &phone, &allow_card, &ch, "allow", &keys.approve).await;
    assert_eq!(
        (resp.status, resp.json()),
        (200, json!({"resolved": "allowed"}))
    );
    assert!(allow_rx.try_recv().unwrap().approved);
    let by = crate::approvals::test_take_answered_by(&allow_card.id).unwrap();
    assert_eq!(
        (by.kind, by.device_id.as_str(), by.face_id),
        ("device", phone.device_id.as_str(), true)
    );

    let ch = decision_challenge(&rig, &phone, &deny_card.id).await;
    let resp = decide(&rig, &phone, &deny_card, &ch, "deny", &keys.deny).await;
    assert_eq!(
        (resp.status, resp.json()),
        (200, json!({"resolved": "denied"}))
    );
    assert!(!deny_rx.try_recv().unwrap().approved);
    crate::approvals::test_take_answered_by(&deny_card.id);
    set_cards(vec![]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_wrong_key_for_the_word_is_refused_and_a_copied_token_alone_answers_nothing() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    let target = card("apr-dec-wrong", "Install", "Commit", None);
    set_cards(vec![target.clone()]);
    let mut rx = crate::approvals::test_register_pending(&target.id);
    // An allow signed with the deny key, a deny signed with the approve key.
    for (word, key) in [("allow", &keys.deny), ("deny", &keys.approve)] {
        let ch = decision_challenge(&rig, &phone, &target.id).await;
        let resp = decide(&rig, &phone, &target, &ch, word, key).await;
        assert_eq!(
            (resp.status, error_code(&resp).as_str()),
            (400, "bad_signature"),
            "{word}"
        );
    }
    // A deny with no signature, a garbage one, and a word that isn't one.
    let ch = decision_challenge(&rig, &phone, &target.id).await;
    let nonce = ch.json()["nonce"].as_str().unwrap().to_string();
    let digest = card_digest(&target.title, &target.action);
    for body in [
        json!({"decision": "deny", "nonce": nonce, "digest": digest}),
        json!({"decision": "deny", "nonce": nonce, "digest": digest, "signature": ""}),
        json!({"decision": "maybe", "nonce": nonce, "digest": digest, "signature": "AAAA"}),
    ] {
        let resp = rig
            .call(
                "POST",
                &format!("/v1/approvals/{}/decision", target.id),
                Some(&phone.token),
                Some(body),
            )
            .await;
        assert_eq!(resp.status, 400);
    }
    let junk = rig
        .call(
            "POST",
            &format!("/v1/approvals/{}/decision", target.id),
            Some(&phone.token),
            Some(json!({"decision": "allow", "nonce": nonce, "digest": digest, "signature": "AAAAAAAA"})),
        )
        .await;
    assert_eq!(error_code(&junk), "bad_signature");
    assert!(rx.try_recv().is_err(), "nothing answered the card");
    // A phone with a token and no keys.
    let bare = pair(&rig);
    let resp = decision_challenge(&rig, &bare, &target.id).await;
    assert_eq!(
        (resp.status, error_code(&resp).as_str()),
        (403, "approval_key_required")
    );
    let resp = rig
        .call(
            "POST",
            &format!("/v1/approvals/{}/decision", target.id),
            Some(&bare.token),
            Some(json!({"decision": "allow", "nonce": "x", "digest": digest, "signature": "AAAA"})),
        )
        .await;
    assert_eq!(
        (resp.status, error_code(&resp).as_str()),
        (403, "approval_key_required")
    );
    crate::approvals::respond(&target.id, false).ok();
    set_cards(vec![]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_nonce_is_one_use_and_belongs_to_one_phone_and_one_card() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    let (other, other_keys) = enrolled(&rig);
    let (one, two) = (
        card("apr-dec-n1", "One", "a", None),
        card("apr-dec-n2", "Two", "b", None),
    );
    set_cards(vec![one.clone(), two.clone()]);
    let _rx1 = crate::approvals::test_register_pending(&one.id);
    let _rx2 = crate::approvals::test_register_pending(&two.id);
    let ch = decision_challenge(&rig, &phone, &one.id).await;
    // Another phone can't use it, nor can it answer the other card.
    let resp = decide(&rig, &other, &one, &ch, "allow", &other_keys.approve).await;
    assert_eq!(error_code(&resp), "approval_expired");
    let resp = decide(&rig, &phone, &two, &ch, "allow", &keys.approve).await;
    assert_eq!(error_code(&resp), "approval_expired");
    // Used once, it is gone: a replay is refused.
    let resp = decide(&rig, &phone, &one, &ch, "allow", &keys.approve).await;
    assert_eq!(resp.status, 200);
    let replay = decide(&rig, &phone, &one, &ch, "allow", &keys.approve).await;
    assert_eq!(
        (replay.status, error_code(&replay).as_str()),
        (409, "approval_expired")
    );
    crate::approvals::test_take_answered_by(&one.id);
    crate::approvals::respond(&two.id, false).ok();
    set_cards(vec![]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_changed_card_is_refused_without_counting_as_a_bad_signature() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    let original = card("apr-dec-chg", "Install", "Commit abc", None);
    set_cards(vec![original.clone()]);
    let _rx = crate::approvals::test_register_pending(&original.id);
    let ch = decision_challenge(&rig, &phone, &original.id).await;
    // The card's text changes under the phone: its digest no longer matches.
    let mut changed = original.clone();
    changed.action = "Commit def, 40 files".into();
    set_cards(vec![changed.clone()]);
    for _ in 0..8 {
        let resp = decide(&rig, &phone, &original, &ch, "allow", &keys.approve).await;
        assert_eq!(
            (resp.status, error_code(&resp).as_str()),
            (409, "approval_changed")
        );
    }
    assert!(
        keys_of(&rig, &phone).is_some(),
        "eight changed cards dropped no keys"
    );
    // The phone reads the new text and signs that, with the same challenge.
    let resp = decide(&rig, &phone, &changed, &ch, "allow", &keys.approve).await;
    assert_eq!(resp.status, 200);
    crate::approvals::test_take_answered_by(&original.id);
    set_cards(vec![]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_card_and_the_first_answer_winning() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    // No such card: no challenge.
    set_cards(vec![]);
    let resp = decision_challenge(&rig, &phone, "apr-dec-none").await;
    assert_eq!(
        (resp.status, error_code(&resp).as_str()),
        (409, "approval_closed")
    );
    // The Mac's window answers while the phone holds a challenge.
    let target = card("apr-dec-first", "Install", "Commit", None);
    set_cards(vec![target.clone()]);
    let mut rx = crate::approvals::test_register_pending(&target.id);
    let ch = decision_challenge(&rig, &phone, &target.id).await;
    crate::approvals::respond(&target.id, false).unwrap();
    let resp = decide(&rig, &phone, &target, &ch, "allow", &keys.approve).await;
    assert_eq!(
        (resp.status, error_code(&resp).as_str()),
        (409, "approval_closed")
    );
    assert!(
        !rx.try_recv().unwrap().approved,
        "the window's answer stood"
    );
    assert!(crate::approvals::test_take_answered_by(&target.id).is_none());
    // The card still listed but no longer pending (answered a moment ago): closed too.
    let ch = decision_challenge(&rig, &phone, &target.id).await;
    let resp = decide(&rig, &phone, &target, &ch, "allow", &keys.approve).await;
    assert_eq!(error_code(&resp), "approval_closed");
    set_cards(vec![]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_bad_signatures_drop_the_keys_and_the_mac_can_say_so() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, keys) = enrolled(&rig);
    let target = card("apr-dec-bad", "Install", "Commit", None);
    set_cards(vec![target.clone()]);
    let _rx = crate::approvals::test_register_pending(&target.id);
    for _ in 0..5 {
        let ch = decision_challenge(&rig, &phone, &target.id).await;
        // Signed with the deny key for an allow.
        let resp = decide(&rig, &phone, &target, &ch, "allow", &keys.deny).await;
        assert_eq!(error_code(&resp), "bad_signature");
    }
    assert!(keys_of(&rig, &phone).is_none(), "the keys were dropped");
    assert!(rig
        .ctx
        .approval_decisions
        .dropped_at(&phone.device_id)
        .is_some());
    let resp = decision_challenge(&rig, &phone, &target.id).await;
    assert_eq!(error_code(&resp), "approval_key_required");
    crate::approvals::respond(&target.id, false).ok();
    set_cards(vec![]);
}

#[test]
fn challenges_expire_and_a_phone_keeps_sixteen() {
    let decisions = approvals_api::Decisions::default();
    let t0 = Utc::now();
    let (first, expires) = decisions.issue("ph-00000001", "apr-1", t0);
    assert_eq!(
        expires,
        (t0 + chrono::Duration::seconds(60)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
    );
    for i in 0..16 {
        decisions.issue("ph-00000001", &format!("apr-{}", i + 2), t0);
    }
    // The oldest was dropped to make room for the seventeenth.
    assert!(!decisions.consume_for_test("ph-00000001", &first, "apr-1", t0));
    // A challenge is gone after 60 seconds.
    let (late, _) = decisions.issue("ph-00000002", "apr-9", t0);
    assert!(!decisions.consume_for_test(
        "ph-00000002",
        &late,
        "apr-9",
        t0 + chrono::Duration::seconds(61)
    ));
    let (live, _) = decisions.issue("ph-00000002", "apr-9", t0);
    assert!(decisions.consume_for_test(
        "ph-00000002",
        &live,
        "apr-9",
        t0 + chrono::Duration::seconds(59)
    ));
    assert!(!decisions.consume_for_test("ph-00000002", &live, "apr-9", t0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_revoked_phone_answers_nothing() {
    let _held = approvals_api::TEST_CARDS_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let rig = rig().await;
    let (phone, _keys) = enrolled(&rig);
    let target = card("apr-dec-rev", "Install", "Commit", None);
    set_cards(vec![target.clone()]);
    rig.store.revoke(&phone.device_id).unwrap();
    for path in ["challenge", "decision"] {
        let resp = rig
            .call(
                "POST",
                &format!("/v1/approvals/{}/{path}", target.id),
                Some(&phone.token),
                Some(json!({})),
            )
            .await;
        assert_eq!(resp.status, 401, "{path}");
    }
    set_cards(vec![]);
}

#[test]
fn the_wait_follows_who_has_keys() {
    let mut record = store::Record {
        enabled: true,
        ..store::Record::default()
    };
    assert_eq!(crate::phone_access::phone_wait_of(&record), None);
    let mut device: store::Device = serde_json::from_value(json!({
        "deviceId": "ph-0a1b2c3d", "name": "iPhone", "tokenSha256": "00", "pairedAt": "2026-11-01T00:00:00Z"
    }))
    .unwrap();
    record.devices.push(device.clone());
    assert_eq!(crate::phone_access::phone_wait_of(&record), None);
    device.approval_keys = Some(store::ApprovalKeys {
        approve_key: "a".into(),
        deny_key: "d".into(),
        fingerprint: "0000 0000".into(),
        enrolled_at: "2026-11-19T00:00:00Z".into(),
    });
    record.devices = vec![device];
    assert_eq!(crate::phone_access::phone_wait_of(&record), Some(300));
    record.approval_wait_secs = Some(600);
    assert_eq!(crate::phone_access::phone_wait_of(&record), Some(600));
    // The iPhone switch off: no phone can answer, so cards keep their 120 seconds.
    record.enabled = false;
    assert_eq!(crate::phone_access::phone_wait_of(&record), None);
}

#[test]
fn the_decision_bodies_are_the_fixtures() {
    assert_eq!(
        value(&approvals_api::DecisionChallengeBody {
            nonce: "Tn3kQ0v1bY8cWm5xLr2pEa9dHsJf7uZtGiOyNwKqXMA".into(),
            expires_at: "2026-11-19T09:01:00Z".into(),
        }),
        fixture("approvals/decision-challenge.json")
    );
    assert_eq!(
        value(&approvals_api::ResolvedBody {
            resolved: "allowed"
        }),
        fixture("approvals/decision-response.json")
    );
    for (error, file) in [
        (
            routes::ApiError::approval_key_required(),
            "error-approval-key-required.json",
        ),
        (
            routes::ApiError::approval_expired(),
            "error-approval-expired.json",
        ),
        (
            routes::ApiError::approval_closed(),
            "error-approval-closed.json",
        ),
        (
            routes::ApiError::approval_changed(),
            "error-approval-changed.json",
        ),
        (
            routes::ApiError::bad_signature(),
            "error-bad-signature.json",
        ),
    ] {
        assert_eq!(value(&error.body()), fixture(&format!("approvals/{file}")));
    }
    let request = fixture("approvals/decision-request.json");
    for field in ["decision", "nonce", "digest", "signature"] {
        assert!(request[field].is_string(), "{field}");
    }
}

/// Signatures made by Apple's Security framework (`SecKeyCreateSignature`,
/// `.ecdsaSignatureMessageX962SHA256`), so Hollow's check is tested against the
/// encoding the phone produces and not only against `ring`'s own.
#[test]
fn signatures_made_by_apples_security_framework_verify() {
    let v = fixture("approvals/decision-vector.json");
    let text = |k: &str| v[k].as_str().unwrap().to_string();
    let verify = |key: &str, decision: &str, signature: &str, device: &str| {
        let message = approvals_api::decision_message(
            device,
            &text("approvalId"),
            &text("nonce"),
            &text("digest"),
            decision,
            &text("expiresAt"),
        );
        ring::signature::UnparsedPublicKey::new(
            &ring::signature::ECDSA_P256_SHA256_ASN1,
            URL_SAFE_NO_PAD.decode(key).unwrap(),
        )
        .verify(
            message.as_bytes(),
            &URL_SAFE_NO_PAD.decode(signature).unwrap(),
        )
        .is_ok()
    };
    let device = text("deviceId");
    assert!(crate::phone_access::app_attest::p256_point_valid(
        &URL_SAFE_NO_PAD.decode(text("approveKey")).unwrap()
    ));
    assert!(verify(
        &text("approveKey"),
        "allow",
        &text("allowSignature"),
        &device
    ));
    assert!(verify(
        &text("denyKey"),
        "deny",
        &text("denySignature"),
        &device
    ));
    // Each key refuses the other's word, and the message binds the device.
    assert!(!verify(
        &text("approveKey"),
        "deny",
        &text("allowSignature"),
        &device
    ));
    assert!(!verify(
        &text("denyKey"),
        "allow",
        &text("denySignature"),
        &device
    ));
    assert!(!verify(
        &text("approveKey"),
        "allow",
        &text("allowSignature"),
        "ph-ffffffff"
    ));
    assert!(!verify(
        &text("denyKey"),
        "deny",
        &text("allowSignature"),
        &device
    ));
}
