//! M2, the peer protocol's server side (`docs/phone-api/machines.md`):
//! another Mac's pairing window and route, its identity route, its route
//! allowlist (default deny) walked over real HTTP against the [`ROUTES`]
//! table, and the iPhone contract unchanged beside it. Every test runs on a
//! rig (a free port, a temporary vault, store and `machine.json`).
use super::*;
use crate::phone_access::routes::{Scope, ROUTES};
use crate::phone_access::store::DeviceKind;

const MACHINE_CODE: &str = "M4CH7NE2";
const CLIENT_ID: &str = "m-3f9c1a07e2b45d18";
const CHAT: &str = "s-20261115-ffffffff";

fn pair_body(code: &str, machine_id: &str) -> Value {
    json!({
        "code": code,
        "machine": {"machineId": machine_id, "slug": "macbook-pro", "name": "MacBook Pro"},
        "clientVersion": "0.5.17 (abc1234)",
    })
}

impl Rig {
    /// Pair a Mac through the route: `(token, deviceId)`.
    async fn machine_token(&self) -> (String, String) {
        self.machine_token_for(CLIENT_ID).await
    }

    async fn machine_token_for(&self, machine_id: &str) -> (String, String) {
        self.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
        let resp = self
            .call(
                "POST",
                "/v1/machines/pair",
                None,
                Some(pair_body(MACHINE_CODE, machine_id)),
            )
            .await;
        assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
        let body = resp.json();
        (
            body["token"].as_str().unwrap().to_string(),
            body["deviceId"].as_str().unwrap().to_string(),
        )
    }

    /// A request with extra headers and a Host of our own.
    async fn call_as(
        &self,
        method: &str,
        path: &str,
        host: &str,
        extra: &[(&str, &str)],
        token: Option<&str>,
        body: Option<Value>,
    ) -> Resp {
        let auth = token.map(|t| format!("Bearer {t}"));
        let mut headers = vec![("Host", host), ("Connection", "close")];
        headers.extend_from_slice(extra);
        if let Some(auth) = auth.as_deref() {
            headers.push(("Authorization", auth));
        }
        let body = body.map(|b| b.to_string());
        let mut http = Http::connect(self.port).await;
        http.send(method, path, &headers, body.as_deref().map(str::as_bytes))
            .await;
        http.response().await
    }

    /// The head of an event stream (it never ends on its own).
    async fn stream_status(&self, token: &str) -> u16 {
        let host = format!("127.0.0.1:{}", self.port);
        let auth = format!("Bearer {token}");
        let mut http = Http::connect(self.port).await;
        http.send(
            "GET",
            "/v1/events",
            &[("Host", host.as_str()), ("Authorization", auth.as_str())],
            None,
        )
        .await;
        http.head().await.0
    }

    fn quick_chat(&self) -> String {
        let quick = crate::workspaces::ensure_quick_chats(&self.vault).unwrap();
        let quick = quick.to_string_lossy().into_owned();
        seed_chat(
            &self.vault,
            CHAT,
            "claude",
            &quick,
            &[("Hi", "Hello")],
            "plan",
        );
        quick
    }
}

fn forbidden(resp: &Resp, what: &str) {
    assert_eq!(
        resp.status,
        403,
        "{what}: {}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(resp.json()["error"]["code"], "forbidden", "{what}");
}

// ── Fixtures ─────────────────────────────────────────────────────────────

#[test]
fn the_machine_fixtures_are_what_hollow_reads_and_writes() {
    let request: wire::MachinePairRequest =
        serde_json::from_value(fixture("machines/pair-request.json")).unwrap();
    assert_eq!(request.code.as_deref(), Some("7KQ2-M9XA"));
    let machine = request.machine.unwrap();
    assert_eq!(machine.machine_id.as_deref(), Some("m-3f9c1a07e2b45d18"));
    assert!(store::valid_machine_id(
        machine.machine_id.as_deref().unwrap()
    ));
    assert_eq!(machine.slug.as_deref(), Some("macbook-pro"));
    assert_eq!(machine.name.as_deref(), Some("MacBook Pro"));
    assert_eq!(request.client_version.as_deref(), Some("0.5.17 (abc1234)"));

    let info = wire::MachineInfo {
        machine_id: "m-8d21c4be90f17a3e".into(),
        slug: Some("mac-mini".into()),
        name: "Mac mini".into(),
        app: "hollow",
        api: wire::API,
        peer_api: wire::PEER_API,
        version: "0.5.17".into(),
        commit: "abc1234".into(),
        headless: true,
        capabilities: crate::runner_coo::capabilities(&wire::CAPABILITIES, None, false),
        coo: None,
    };
    assert_eq!(value(&info), fixture("machines/machine.json"));
    // M3a: the runner adds `coo` and its chat key; any other Mac sends
    // neither (the fixture above).
    let mut runner = fixture("machines/machine.json");
    runner["capabilities"]
        .as_array_mut()
        .unwrap()
        .push(json!("coo"));
    runner["coo"] = json!("s-20261115-0000c000");
    let as_runner = wire::MachineInfo {
        capabilities: crate::runner_coo::capabilities(
            &wire::CAPABILITIES,
            Some("s-20261115-0000c000"),
            true,
        ),
        coo: Some("s-20261115-0000c000".into()),
        ..info.clone()
    };
    assert_eq!(value(&as_runner), runner);
    let paired = wire::MachinePairResponse {
        token: "q3Zt8wQ0b1V4mN7xK2sY9cR5uE6aH0jL3pD8fG1iT4o".into(),
        device_id: "mc-1a2b3c4d".into(),
        machine: info,
        api: wire::API,
        peer_api: wire::PEER_API,
    };
    assert_eq!(value(&paired), fixture("machines/pair-response.json"));
    // A2: the account snapshot, exactly.
    let account = |key: &str, provider, label: &str| wire::AccountUsage {
        key: key.into(),
        provider,
        label: label.into(),
        billing: Some("Acme".into()),
        routed: true,
        load_percent: None,
        blocked: false,
        blocked_until: None,
        seven_day_resets_at: None,
        measured_at: None,
        recent_picks: 0,
        last_picked_at: None,
    };
    let usage = wire::MachineUsage {
        usage_api: wire::USAGE_API,
        machine_id: "m-8d21c4be90f17a3e".into(),
        observed_at: "2026-11-15T20:41:12Z".into(),
        accounts: vec![
            wire::AccountUsage {
                load_percent: Some(42.5),
                seven_day_resets_at: Some("2026-11-19T04:00:00Z".into()),
                measured_at: Some("2026-11-15T20:41:07Z".into()),
                recent_picks: 2,
                last_picked_at: Some("2026-11-15T20:40:55Z".into()),
                ..account("ak-5c0b8e21d47f9a36", "claude", "Claude Code login")
            },
            wire::AccountUsage {
                load_percent: Some(18.0),
                measured_at: Some("2026-11-15T20:12:40Z".into()),
                ..account("ak-91e4f0c2a8d35b77", "codex", "Codex (ChatGPT Team)")
            },
        ],
    };
    assert_eq!(value(&usage), fixture("machines/usage.json"));
    assert_eq!(
        value(&ApiError::not_for_machines().body()),
        fixture("machines/error-machine-forbidden.json")
    );
    // Every new refusal is one line ending in a full stop, as the contract
    // asks of every message.
    for error in [
        ApiError::not_for_machines(),
        ApiError::machines_only(),
        ApiError::loopback_only(),
        ApiError::machine_pairing_closed(),
        ApiError::phones_off(),
    ] {
        assert!(error.message.ends_with('.') && !error.message.contains('\n'));
    }
}

// ── The record ───────────────────────────────────────────────────────────

/// An old `phone.json` (no `kind`, no `machines`) reads as phones; fields a
/// later Hollow writes are kept on every write; a Mac goes in its own list,
/// which a reader from before M2 never sees, so it can't take a Mac for a
/// phone.
#[test]
fn an_old_record_reads_as_phones_and_unknown_fields_survive_a_write() {
    let dir = store_dir();
    let token = "an-old-phones-token";
    std::fs::write(
        dir.path().join(store::FILE),
        json!({
            "enabled": true,
            "devices": [{
                "deviceId": "ph-0000abcd",
                "name": "Old iPhone",
                "model": null,
                "tokenSha256": store::token_hash(token),
                "pairedAt": "2026-11-04T10:00:00.000000+00:00",
                "lastSeenAt": null,
                "laterDeviceField": 2,
            }],
            "laterField": {"x": 1},
        })
        .to_string(),
    )
    .unwrap();
    let store = Store::at(dir.path());
    let phone = store.device_for_token(token).unwrap().unwrap();
    assert_eq!(phone.kind, DeviceKind::Phone);
    assert_eq!(phone.machine_id, None);
    assert!(store.phone_ready() && store.viewers_ready() && !store.machines_paired());

    store.set_enabled(false).unwrap();
    let (mac, mac_token, replaced) = store
        .pair_machine("MacBook Pro", CLIENT_ID, Utc::now())
        .unwrap();
    assert!(replaced.is_none());
    assert!(mac.device_id.starts_with("mc-") && mac.device_id.len() == 11);
    let text = std::fs::read_to_string(store.path()).unwrap();
    assert!(!text.contains(&mac_token), "only the token's hash is kept");
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["laterField"], json!({"x": 1}));
    let device = &file["devices"][0];
    assert_eq!(device["laterDeviceField"], 2);
    assert!(device.get("kind").is_none() && device.get("machineId").is_none());
    assert_eq!(file["devices"].as_array().unwrap().len(), 1);
    assert_eq!(file["machines"][0]["kind"], "machine");
    assert_eq!(file["machines"][0]["machineId"], CLIENT_ID);
    assert_eq!(
        file["machines"][0]["tokenSha256"],
        store::token_hash(&mac_token)
    );
    // The switch off with a Mac paired: phones don't watch, the Mac does.
    assert!(!store.phone_ready() && store.viewers_ready() && store.machines_paired());

    // A Hollow from before M2 reads only `devices`: the Mac's token names
    // nothing there.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct OldDevice {
        token_sha256: String,
    }
    #[derive(serde::Deserialize)]
    struct OldRecord {
        devices: Vec<OldDevice>,
    }
    let old: OldRecord = serde_json::from_str(&text).unwrap();
    assert!(old
        .devices
        .iter()
        .all(|d| d.token_sha256 != store::token_hash(&mac_token)));

    // A fresh store reads the Mac as a Mac, also from `machines` alone.
    let again = Store::at(dir.path());
    let found = again.device_for_token(&mac_token).unwrap().unwrap();
    assert_eq!(found.kind, DeviceKind::Machine);
    assert_eq!(found.machine_id.as_deref(), Some(CLIENT_ID));
    // The same Mac pairing again replaces its device: the old token is dead.
    let (newer, newer_token, replaced) = again
        .pair_machine("MacBook Pro", CLIENT_ID, Utc::now())
        .unwrap();
    assert_eq!(replaced.as_deref(), Some(mac.device_id.as_str()));
    assert_ne!(newer.device_id, mac.device_id);
    assert!(again.device_for_token(&mac_token).unwrap().is_none());
    assert!(again.device_for_token(&newer_token).unwrap().is_some());
    assert_eq!(again.read().unwrap().machines.len(), 1);
    assert!(again.revoke(&newer.device_id).unwrap());
    assert!(!again.viewers_ready() && !again.machines_paired());
}

// ── Pairing ──────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_pairs_through_its_own_window_once_and_gets_a_machine_token() {
    let rig = rig().await;
    // No window: closed, and the phone's window doesn't count.
    let closed = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body(MACHINE_CODE, CLIENT_ID)),
        )
        .await;
    assert_eq!(closed.status, 403);
    assert_eq!(closed.json()["error"]["code"], "pairing_closed");
    assert_eq!(rig.live.watching(), 0, "nothing paired, nothing copied");

    rig.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
    let resp = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body("m4ch-7ne2", CLIENT_ID)),
        )
        .await;
    assert_eq!(resp.status, 200);
    let body = resp.json();
    same_shape(&body, &fixture("machines/pair-response.json"), "pair");
    let token = body["token"].as_str().unwrap().to_string();
    assert_eq!(token.len(), 43);
    let device_id = body["deviceId"].as_str().unwrap();
    assert!(device_id.starts_with("mc-"));
    assert_eq!(
        (body["api"].clone(), body["peerApi"].clone()),
        (json!(1), json!(1))
    );
    let me = &body["machine"];
    assert_eq!(me["name"], "Test Mac");
    assert_eq!(me["app"], "hollow");
    assert_eq!(me["headless"], false, "the rig has a window");
    assert!(store::valid_machine_id(me["machineId"].as_str().unwrap()));
    assert_eq!(
        me["capabilities"],
        json!(["chats", "send", "stop", "cards", "events", "usage", "moves", "appends"])
    );
    // One use.
    assert!(rig.ctx.machine_pairing.current(Utc::now()).is_none());
    // The record holds the Mac in its own list, hash only.
    let record = rig.store.read().unwrap();
    assert!(record.devices.is_empty());
    assert_eq!(record.machines.len(), 1);
    assert_eq!(record.machines[0].machine_id.as_deref(), Some(CLIENT_ID));
    assert_eq!(record.machines[0].token_sha256, store::token_hash(&token));
    assert!(rig.host.changed.load(Ordering::SeqCst) >= 1);

    // `GET /v1/machine` says the same, and the id is this Mac's stable one.
    let info = rig.call("GET", "/v1/machine", Some(&token), None).await;
    assert_eq!(info.status, 200);
    same_shape(&info.json(), &fixture("machines/machine.json"), "machine");
    assert_eq!(info.json(), body["machine"]);
    let again = rig.call("GET", "/v1/machine", Some(&token), None).await;
    assert_eq!(again.json()["machineId"], me["machineId"]);

    // Live follow with only a Mac paired (the iPhone switch is off).
    assert!(!rig.store.read().unwrap().enabled);
    assert_ne!(rig.live.watching(), 0, "a paired Mac watches chats live");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn five_wrong_machine_codes_close_its_window() {
    let rig = rig().await;
    rig.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
    for left in (0..store::MAX_WRONG_CODES).rev() {
        let resp = rig
            .call(
                "POST",
                "/v1/machines/pair",
                None,
                Some(pair_body("WRNGC0DE", CLIENT_ID)),
            )
            .await;
        assert_eq!(resp.status, 403);
        assert_eq!(resp.json()["error"]["code"], "wrong_code");
        assert_eq!(resp.json()["error"]["attemptsLeft"], left);
    }
    let resp = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body(MACHINE_CODE, CLIENT_ID)),
        )
        .await;
    assert_eq!(resp.json()["error"]["code"], "pairing_closed");
    assert!(rig.store.read().unwrap().machines.is_empty());
    // A bad request spends no try.
    rig.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
    for bad in [
        json!({"code": MACHINE_CODE}),
        pair_body(MACHINE_CODE, "m-NOTHEX"),
        json!({"code": MACHINE_CODE, "machine": {"machineId": CLIENT_ID, "name": ""}}),
        json!({"code": MACHINE_CODE, "machine": {"machineId": CLIENT_ID, "name": "Mac", "slug": "Not A Slug"}}),
    ] {
        let resp = rig.call("POST", "/v1/machines/pair", None, Some(bad)).await;
        assert_eq!(resp.status, 400, "{}", String::from_utf8_lossy(&resp.body));
    }
    let resp = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body(MACHINE_CODE, CLIENT_ID)),
        )
        .await;
    assert_eq!(resp.status, 200);
}

/// Two windows, two routes: a phone's code never pairs a Mac, nor a Mac's
/// code a phone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_phone_code_never_pairs_a_mac_nor_the_reverse() {
    let rig = rig().await;
    rig.pairing.open_with("PH0NE234", Utc::now());
    // The phone's code at the Mac route, with no Mac window: closed.
    let resp = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body("PH0NE234", CLIENT_ID)),
        )
        .await;
    assert_eq!(resp.json()["error"]["code"], "pairing_closed");
    // With both open, each code is wrong at the other's route.
    rig.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
    let resp = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body("PH0NE234", CLIENT_ID)),
        )
        .await;
    assert_eq!(resp.json()["error"]["code"], "wrong_code");
    let resp = rig
        .call(
            "POST",
            "/v1/pair",
            None,
            Some(json!({"code": MACHINE_CODE, "deviceName": "iPhone"})),
        )
        .await;
    assert_eq!(resp.json()["error"]["code"], "wrong_code");
    // Each still pairs its own kind.
    let phone = rig
        .call(
            "POST",
            "/v1/pair",
            None,
            Some(json!({"code": "PH0NE234", "deviceName": "iPhone"})),
        )
        .await;
    assert_eq!(phone.status, 200);
    assert!(phone.json()["deviceId"]
        .as_str()
        .unwrap()
        .starts_with("ph-"));
    same_shape(&phone.json(), &fixture("pair-response.json"), "phone pair");
    let mac = rig
        .call(
            "POST",
            "/v1/machines/pair",
            None,
            Some(pair_body(MACHINE_CODE, CLIENT_ID)),
        )
        .await;
    assert_eq!(mac.status, 200);
    let record = rig.store.read().unwrap();
    assert_eq!((record.devices.len(), record.machines.len()), (1, 1));
}

/// A request through `tailscale serve` (or any proxy) can't try a Mac code
/// or use a Mac's token: only a loopback request, as an SSH tunnel makes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_tailnet_request_never_tries_a_machine_code_or_uses_a_machine_token() {
    let rig = rig().await;
    rig.ctx.machine_pairing.open_with(MACHINE_CODE, Utc::now());
    let loopback = format!("127.0.0.1:{}", rig.port);
    let tailnet = format!("{DNS}:8443");
    let body = || Some(pair_body(MACHINE_CODE, CLIENT_ID));
    let resp = rig
        .call_as(
            "POST",
            "/v1/machines/pair",
            &tailnet,
            &[("Tailscale-User-Login", LOGIN)],
            None,
            body(),
        )
        .await;
    forbidden(&resp, "this Mac's own tailnet user");
    let resp = rig
        .call_as(
            "POST",
            "/v1/machines/pair",
            &loopback,
            &[("X-Forwarded-For", "100.64.0.9")],
            None,
            body(),
        )
        .await;
    forbidden(&resp, "a proxied loopback request");
    let resp = rig
        .call_as(
            "POST",
            "/v1/machines/pair",
            &loopback,
            &[("Tailscale-User-Login", LOGIN)],
            None,
            body(),
        )
        .await;
    forbidden(&resp, "a loopback Host with a tailnet user");
    // None of that spent a try: the window is open with every try left.
    let (token, _) = {
        let resp = rig.call("POST", "/v1/machines/pair", None, body()).await;
        assert_eq!(resp.status, 200);
        let body = resp.json();
        (body["token"].as_str().unwrap().to_string(), ())
    };
    // The Mac's token over the tailnet: refused; straight: taken.
    let resp = rig
        .call_as(
            "GET",
            "/v1/sessions",
            &tailnet,
            &[("Tailscale-User-Login", LOGIN)],
            Some(&token),
            None,
        )
        .await;
    forbidden(&resp, "a machine token over the tailnet");
    let resp = rig.call("GET", "/v1/sessions", Some(&token), None).await;
    assert_eq!(resp.status, 200);
    // A phone over the tailnet works as before.
    let phone = rig.token();
    let resp = rig
        .call_as(
            "GET",
            "/v1/me",
            &tailnet,
            &[("Tailscale-User-Login", LOGIN)],
            Some(&phone),
            None,
        )
        .await;
    assert_eq!(resp.status, 200);
}

// ── The allowlist ────────────────────────────────────────────────────────

/// Every arm of `dispatch` is in [`ROUTES`] and nothing else is, so a route
/// added later is refused to a Mac until someone gives it a scope here.
#[test]
fn the_routes_table_names_every_arm_of_dispatch() {
    let source = include_str!("routes.rs");
    let start = source
        .find("pub(crate) async fn dispatch(")
        .expect("dispatch");
    let end = start
        + source[start..]
            .find("_ => Err(ApiError::not_found(\"Hollow has no such route.\"))")
            .expect("dispatch's last arm");
    let body = &source[start..end];
    let mut arms = std::collections::BTreeSet::new();
    let mut rest = body;
    while let Some(at) = rest.find("(\"") {
        rest = &rest[at + 2..];
        let Some(close) = rest.find('"') else { break };
        let method = &rest[..close];
        let after = &rest[close + 1..];
        if !method.chars().all(|c| c.is_ascii_uppercase()) || !after.starts_with(", [") {
            continue;
        }
        let list = &after[3..after.find(']').unwrap()];
        let path: String = list
            .split(',')
            .map(str::trim)
            .map(|part| match part.strip_prefix('"') {
                Some(word) => word.trim_end_matches('"').to_string(),
                None => "{}".to_string(),
            })
            .map(|part| format!("/{part}"))
            .collect();
        arms.insert((method.to_string(), path));
    }
    let table: std::collections::BTreeSet<(String, String)> = ROUTES
        .iter()
        .map(|(method, pattern, _)| {
            let path = pattern
                .split('/')
                .skip(1)
                .map(|p| {
                    if p.starts_with('{') {
                        "/{}".to_string()
                    } else {
                        format!("/{p}")
                    }
                })
                .collect::<String>();
            (method.to_string(), path)
        })
        .collect();
    assert!(arms.len() > 30, "the parse found the arms: {arms:?}");
    assert_eq!(arms, table, "dispatch's arms and ROUTES differ");
    // The peer link's allowlist, exactly.
    let machine: Vec<(&str, &str)> = ROUTES
        .iter()
        .filter(|(_, _, scope)| matches!(scope, Scope::Both | Scope::Machine))
        .map(|(m, p, _)| (*m, *p))
        .collect();
    assert_eq!(
        machine,
        [
            ("GET", "/v1/machine"),
            ("GET", "/v1/machine/usage"),
            ("POST", "/v1/machine/coo"),
            ("DELETE", "/v1/machines/me"),
            ("POST", "/v1/machines/moves/{id}/parts/{part}"),
            ("POST", "/v1/machines/moves/{id}/commit"),
            ("GET", "/v1/machines/moves/{id}"),
            ("DELETE", "/v1/machines/moves/{id}"),
            ("GET", "/v1/sessions"),
            ("GET", "/v1/sessions/{key}"),
            ("POST", "/v1/sessions/{key}/messages"),
            ("POST", "/v1/sessions/{key}/questions/{question}/answer"),
            ("POST", "/v1/sessions/{key}/stop"),
            ("GET", "/v1/events"),
        ]
    );
}

/// Every route in [`ROUTES`] against a Mac's token over real HTTP: the ones
/// it may use answer (2xx, or a 4xx about the request itself), every other
/// is 403 `forbidden`; and a phone's token is refused the Mac's own routes
/// and takes the shared ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_route_is_walked_with_a_machine_token_and_a_phone_token() {
    let rig = rig().await;
    rig.quick_chat();
    let phone = rig.token();
    let (mac, _) = rig.machine_token().await;
    let concrete = |pattern: &str| {
        pattern
            .replace("{key}", CHAT)
            .replace("{question}", "qi-0000000000000000")
            .replace("{id}", "x")
            .replace("{canvas}", "x")
            .replace("{part}", "0")
    };
    // `DELETE /v1/machines/me` unpairs the caller, so it goes last.
    let mut routes: Vec<_> = ROUTES.iter().collect();
    routes.sort_by_key(|(m, p, _)| (*m, *p) == ("DELETE", "/v1/machines/me"));
    for (method, pattern, scope) in routes {
        let path = concrete(pattern);
        let what = format!("{method} {pattern}");
        if *scope == Scope::Open {
            continue;
        }
        // The phone first, while the Mac is still paired.
        match scope {
            Scope::Machine => forbidden(
                &rig.call(method, &path, Some(&phone), body_for(method))
                    .await,
                &what,
            ),
            Scope::Both if path == "/v1/events" => {
                assert_eq!(rig.stream_status(&phone).await, 200, "{what}")
            }
            Scope::Both => {
                let resp = rig
                    .call(method, &path, Some(&phone), body_for(method))
                    .await;
                assert!(
                    resp.status != 403 && resp.status != 401,
                    "{what}: {}",
                    resp.status
                );
            }
            _ => {}
        }
        match scope {
            Scope::Phone => forbidden(
                &rig.call(method, &path, Some(&mac), body_for(method)).await,
                &what,
            ),
            Scope::Both if path == "/v1/events" => {
                assert_eq!(rig.stream_status(&mac).await, 200, "{what}")
            }
            _ => {
                let resp = rig.call(method, &path, Some(&mac), body_for(method)).await;
                assert!(
                    resp.status < 300
                        || (resp.status == 400 || resp.status == 404 || resp.status == 409),
                    "{what}: {} {}",
                    resp.status,
                    String::from_utf8_lossy(&resp.body)
                );
            }
        }
    }
    // The last route unpaired the Mac.
    let gone = rig.call("GET", "/v1/sessions", Some(&mac), None).await;
    assert_eq!(gone.status, 401);
    // Default deny: a route nobody has is refused to a Mac before it is
    // looked up, and still a 404 to a phone.
    let (mac, _) = rig.machine_token().await;
    forbidden(
        &rig.call("GET", "/v1/nowhere", Some(&mac), None).await,
        "no route",
    );
    assert_eq!(
        rig.call("GET", "/v1/nowhere", Some(&phone), None)
            .await
            .status,
        404
    );
}

fn body_for(method: &str) -> Option<Value> {
    (method == "POST").then(|| json!({}))
}

// ── What a Mac does ──────────────────────────────────────────────────────

/// A2: `GET /v1/machine/usage` answers a Mac (the fixture's shape, this
/// Mac's id, at most the cap, never a token) and refuses a phone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_reads_the_account_snapshot_and_a_phone_cannot() {
    let rig = rig().await;
    let phone = rig.token();
    let (mac, _) = rig.machine_token().await;
    let empty = rig.call("GET", "/v1/machine/usage", Some(&mac), None).await;
    assert_eq!(empty.status, 200);
    assert_eq!(empty.json()["accounts"], json!([]));
    assert_eq!(empty.json()["usageApi"], 1);

    let fixture_body = fixture("machines/usage.json");
    let entry = wire::AccountUsage {
        key: crate::peer_usage::shared_key("claude-org", "1f0e8d3c-0000-4000-8000-00000000000a"),
        provider: "claude",
        label: "Claude Code login".into(),
        billing: Some("Acme".into()),
        routed: true,
        load_percent: Some(42.5),
        blocked: false,
        blocked_until: None,
        seven_day_resets_at: Some("2026-11-19T04:00:00Z".into()),
        measured_at: Some("2026-11-15T20:41:07Z".into()),
        recent_picks: 2,
        last_picked_at: Some("2026-11-15T20:40:55Z".into()),
    };
    *rig.host.usage.lock().unwrap() = vec![entry.clone(); wire::USAGE_ACCOUNTS_MAX + 3];
    let resp = rig.call("GET", "/v1/machine/usage", Some(&mac), None).await;
    assert_eq!(resp.status, 200);
    let body = resp.json();
    same_shape(&body, &fixture_body, "usage");
    same_shape(
        &body["accounts"][0],
        &fixture_body["accounts"][0],
        "usage.accounts",
    );
    assert_eq!(
        body["accounts"].as_array().unwrap().len(),
        wire::USAGE_ACCOUNTS_MAX
    );
    let info = rig
        .call("GET", "/v1/machine", Some(&mac), None)
        .await
        .json();
    assert_eq!(body["machineId"], info["machineId"]);
    assert!(info["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("usage")));
    // What the client reads back is what was served.
    let read: crate::peer_usage::PeerUsage = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(read.accounts[0].key, entry.key);
    assert_eq!(read.accounts[0].load_percent, Some(42.5));
    assert!(
        !String::from_utf8_lossy(&resp.body).contains(&mac),
        "no token in the snapshot"
    );

    forbidden(
        &rig.call("GET", "/v1/machine/usage", Some(&phone), None)
            .await,
        "a phone at the Mac's usage route",
    );
    assert_eq!(
        rig.call("GET", "/v1/machine/usage", None, None)
            .await
            .status,
        401
    );
}

/// A Mac sends text only, as this Mac keeps the chat: attachments and
/// controls are refused, a plain send is queued as the person's own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_macs_send_is_text_only_and_runs_as_the_chat_is_kept() {
    let rig = rig().await;
    let quick = rig.quick_chat();
    let (mac, _) = rig.machine_token().await;
    let path = format!("/v1/sessions/{CHAT}/messages");
    let id = "6f1c2d3e-4b5a-4c7d-9e8f-0a1b2c3d4e5f";
    for refused in [
        json!({"text": "Hi", "clientMessageId": id, "attachmentIds": []}),
        json!({"text": "Hi", "clientMessageId": id, "attachmentIds": ["at-1"]}),
        json!({"text": "Hi", "clientMessageId": id, "controls": {"effort": "high"}}),
        json!({"text": "Hi", "clientMessageId": id, "controls": {"permissionMode": "bypassPermissions"}}),
    ] {
        let resp = rig.call("POST", &path, Some(&mac), Some(refused)).await;
        assert_eq!(resp.status, 400, "{}", String::from_utf8_lossy(&resp.body));
        assert_eq!(resp.json()["error"]["code"], "bad_request");
    }
    assert!(rig.host.sent.lock().unwrap().is_empty());
    let resp = rig
        .call(
            "POST",
            &path,
            Some(&mac),
            Some(json!({"text": "Carry on.", "clientMessageId": id})),
        )
        .await;
    assert_eq!(resp.status, 202, "{}", String::from_utf8_lossy(&resp.body));
    same_shape(&resp.json(), &fixture("send-accepted.json"), "send");
    let sent = rig.host.sent.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].session_key.as_deref(), Some(CHAT));
    assert_eq!(sent[0].permission_mode.as_deref(), Some("plan"));
    assert_eq!(sent[0].cwd.as_deref(), Some(quick.as_str()));
    // The turn says it came from another Mac, not a phone.
    assert_eq!(
        sent[0].client_turn_id.as_deref(),
        Some(format!("mac-{id}").as_str())
    );
    assert_eq!(live_rows_of(&rig.vault, CHAT).len(), 1);
    // A new chat is not a Mac's to start (M3a).
    let resp = rig
        .call(
            "POST",
            "/v1/sessions",
            Some(&mac),
            Some(json!({"text": "Hi", "clientMessageId": id})),
        )
        .await;
    forbidden(&resp, "new chat");
}

/// `DELETE /v1/machines/me` unpairs the calling Mac: its stream closes, its
/// token is dead, and with nothing else paired the live copy stops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_unpairing_itself_closes_its_stream_and_its_token() {
    let rig = rig().await;
    let (mac, device_id) = rig.machine_token().await;
    assert_ne!(rig.live.watching(), 0);
    let host = format!("127.0.0.1:{}", rig.port);
    let auth = format!("Bearer {mac}");
    let mut stream = Http::connect(rig.port).await;
    stream
        .send(
            "GET",
            "/v1/events",
            &[("Host", host.as_str()), ("Authorization", auth.as_str())],
            None,
        )
        .await;
    assert_eq!(stream.head().await.0, 200);
    assert!(stream.chunk().await.is_some(), "hello");
    let resp = rig
        .call("DELETE", "/v1/machines/me", Some(&mac), None)
        .await;
    assert_eq!(resp.status, 204);
    while stream.chunk().await.is_some() {}
    assert_eq!(rig.live.stream_count(), 0);
    assert_eq!(
        rig.call("GET", "/v1/machine", Some(&mac), None)
            .await
            .status,
        401
    );
    assert!(!rig.store.has_device(&device_id).unwrap());
    assert_eq!(rig.live.watching(), 0, "nothing paired, nothing copied");
}

/// The iPhone contract is unchanged with a Mac paired: the phone's routes
/// and bodies are what they were, its record entry has no new field, and it
/// never reaches a Mac's route. While the server listens only for a Mac
/// (the iPhone switch off), a phone's token waits with 503.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_paired_phone_works_as_before_beside_a_mac() {
    let rig = rig().await;
    rig.quick_chat();
    let phone = rig.token();
    let (mac, _) = rig.machine_token().await;
    let me = rig.call("GET", "/v1/me", Some(&phone), None).await;
    assert_eq!(me.status, 200);
    same_shape(&me.json(), &fixture("me.json"), "me");
    let list = rig.call("GET", "/v1/sessions", Some(&phone), None).await;
    assert_eq!(list.status, 200);
    same_shape(
        &list.json()["sessions"][0],
        &fixture("sessions.json")["sessions"][0],
        "row",
    );
    assert_eq!(
        list.json(),
        rig.call("GET", "/v1/sessions", Some(&mac), None)
            .await
            .json(),
        "a Mac reads the list the phone reads, nothing added"
    );
    let workspaces = rig.call("GET", "/v1/workspaces", Some(&phone), None).await;
    assert_eq!(workspaces.status, 200);
    forbidden(
        &rig.call("GET", "/v1/machine", Some(&phone), None).await,
        "a phone at the Mac's identity route",
    );
    forbidden(
        &rig.call("DELETE", "/v1/machines/me", Some(&phone), None)
            .await,
        "a phone at the Mac's unpair route",
    );
    let file: Value =
        serde_json::from_str(&std::fs::read_to_string(rig.store.path()).unwrap()).unwrap();
    let mut keys: Vec<&String> = file["devices"][0].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "deviceId",
            "lastSeenAt",
            "model",
            "name",
            "pairedAt",
            "tokenSha256"
        ],
        "a phone's record keeps its shape"
    );

    // The switch off with a Mac paired: the phone waits, the Mac goes on.
    rig.ctx.phones_off.store(true, Ordering::SeqCst);
    let off = rig.call("GET", "/v1/me", Some(&phone), None).await;
    assert_eq!(off.status, 503);
    assert_eq!(off.json()["error"]["code"], "unavailable");
    assert_eq!(
        rig.call("GET", "/v1/sessions", Some(&mac), None)
            .await
            .status,
        200
    );
    rig.ctx.phones_off.store(false, Ordering::SeqCst);
    assert_eq!(
        rig.call("GET", "/v1/me", Some(&phone), None).await.status,
        200
    );
}

/// The same Mac pairing again replaces its device: one device per Mac, the
/// old token dead and its streams closed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_pairing_again_replaces_its_old_device() {
    let rig = rig().await;
    let (first, first_id) = rig.machine_token().await;
    assert_eq!(rig.stream_status(&first).await, 200);
    let (second, second_id) = rig.machine_token().await;
    assert_ne!(first_id, second_id);
    assert_eq!(
        rig.call("GET", "/v1/sessions", Some(&first), None)
            .await
            .status,
        401
    );
    assert_eq!(
        rig.call("GET", "/v1/sessions", Some(&second), None)
            .await
            .status,
        200
    );
    // Another Mac is its own device.
    let (_, other_id) = rig.machine_token_for("m-0123456789abcdef").await;
    assert_eq!(rig.store.read().unwrap().machines.len(), 2);
    assert_ne!(other_id, second_id);
}

// ── Review fixes (2026-11-15) ────────────────────────────────────────────

/// A Mac's POST to a route it may not use is refused before its body is
/// read: with every upload place taken, an upload from a Mac is 403 at once
/// (it never waits for a place), and the places stay as they were.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_macs_refused_upload_takes_no_place_and_buffers_nothing() {
    let rig = rig().await;
    rig.quick_chat();
    let (mac, _) = rig.machine_token().await;
    let places = rig.ctx.uploading.available_permits();
    let held = rig
        .ctx
        .uploading
        .clone()
        .acquire_many_owned(places as u32)
        .await
        .unwrap();
    let file = vec![b'x'; 2 * 1024 * 1024];
    let started = Instant::now();
    for path in [
        format!("/v1/sessions/{CHAT}/attachments"),
        "/v1/workspaces/x/attachments".to_string(),
    ] {
        let resp = rig.call_raw("POST", &path, Some(&mac), &file).await;
        forbidden(&resp, &path);
    }
    assert!(
        started.elapsed() < attachments_place_wait(),
        "refused before it waited for a place"
    );
    assert_eq!(rig.ctx.uploading.available_permits(), 0);
    drop(held);
    assert_eq!(rig.ctx.uploading.available_permits(), places);
}

fn attachments_place_wait() -> Duration {
    crate::phone_access::attachments::PLACE_WAIT
}

/// No two rows of [`ROUTES`] that can match one path have different
/// scopes, and of two that match, the more specific is the one read.
#[test]
fn overlapping_routes_agree_and_the_most_specific_wins() {
    let parts = |p: &str| p.split('/').skip(1).map(str::to_string).collect::<Vec<_>>();
    for (i, (m1, p1, s1)) in ROUTES.iter().enumerate() {
        for (m2, p2, s2) in ROUTES.iter().skip(i + 1) {
            let (a, b) = (parts(p1), parts(p2));
            let overlap = m1 == m2
                && a.len() == b.len()
                && a.iter()
                    .zip(&b)
                    .all(|(x, y)| x == y || routes::is_placeholder(x) || routes::is_placeholder(y));
            assert!(
                !overlap || s1 == s2,
                "{m1} {p1} and {p2} overlap with different scopes"
            );
        }
    }
    let scope = |m: &str, p: &str| {
        let parts: Vec<&str> = p.split('/').skip(1).collect();
        routes::scope_of(m, &parts)
    };
    assert_eq!(scope("GET", "/v1/workspaces/overview"), Some(Scope::Phone));
    assert_eq!(scope("GET", "/v1/machine"), Some(Scope::Machine));
    assert_eq!(scope("GET", "/v1/machine/usage"), Some(Scope::Machine));
    assert_eq!(scope("GET", "/v1/sessions/x"), Some(Scope::Both));
    assert_eq!(scope("HEAD", "/v1/sessions"), None);
}

/// Odd spellings of a path never reach a refused route with a Mac's token:
/// each is 403 (no scope, or a phone's) or 404, never 2xx.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn path_variants_never_open_a_refused_route_to_a_mac() {
    let rig = rig().await;
    rig.quick_chat();
    let (mac, _) = rig.machine_token().await;
    for (method, path) in [
        ("GET", "//v1/sessions"),
        ("GET", "/V1/sessions"),
        ("GET", "/v1/tasks/"),
        ("GET", "/v1/tasks//"),
        ("GET", "/v1/waiting/"),
        ("GET", "/v1//sessions"),
        ("GET", "/v1/sessions/%2e%2e"),
        ("GET", "/v1/sessions/%2e%2e/canvases"),
        ("GET", "/v1/me/"),
        ("OPTIONS", "/v1/sessions"),
        ("PUT", "/v1/sessions"),
    ] {
        let resp = rig.call(method, path, Some(&mac), None).await;
        assert!(
            resp.status == 403 || resp.status == 404,
            "{method} {path}: {}",
            resp.status
        );
    }
    // HEAD (its answer has no body to read).
    let host = format!("127.0.0.1:{}", rig.port);
    let auth = format!("Bearer {mac}");
    let mut http = Http::connect(rig.port).await;
    http.send(
        "HEAD",
        "/v1/sessions",
        &[("Host", host.as_str()), ("Authorization", auth.as_str())],
        None,
    )
    .await;
    assert_eq!(http.head().await.0, 403, "HEAD /v1/sessions");
    // The allowed list read with a trailing slash is the list itself.
    assert_eq!(
        rig.call("GET", "/v1/sessions/", Some(&mac), None)
            .await
            .status,
        200
    );
}

/// After a revoke the server stops only with nothing left to listen for:
/// the iPhone switch off, no Mac paired and no machine window open.
#[test]
fn a_revoke_stops_the_server_only_with_nobody_left_to_listen_for() {
    use crate::phone_access::nothing_to_listen_for as quiet;
    assert!(quiet(true, false, false));
    assert!(!quiet(true, false, true), "a Mac is about to pair");
    assert!(!quiet(true, true, false), "a Mac is paired");
    assert!(!quiet(false, false, false), "the iPhone switch is on");
}

// ── M5: a chat moving here ───────────────────────────────────────────────

/// A Mac moves one of its chats here over the routes: its parts, the
/// commit, a repeated commit that changes nothing, the status, and a phone
/// refused every move route.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_mac_moves_a_chat_here_once_over_the_routes() {
    use crate::chat_move::{new_move_id, part_bodies, sha256_hex};
    let rig = rig().await;
    let phone = rig.token();
    let (mac, _) = rig.machine_token().await;
    let id = new_move_id();
    let key = "s-20261115-m5000099";
    let bundle = json!({
        "version": 1, "moveId": id, "key": key, "from": CLIENT_ID,
        "session": {"title": "Trip", "agent": "claude", "createdAt": "2026-11-15T01:00:00Z",
                    "lastActivityAt": "2026-11-15T01:05:00Z"},
        "place": {},
        "native": {"agent": "claude", "id": "0b1c2d3e-4f50-4617-8899-aabbccddeeff"},
        "turns": [{"ord": 1, "runId": "run-1", "startedAt": "2026-11-15T01:00:00Z",
                   "endedAt": "2026-11-15T01:05:00Z", "status": "success", "asked": "Plan it",
                   "permissionMode": "plan",
                   "transcript": {"agent": "claude", "text": "Here is a plan."}}],
    });
    let bytes = serde_json::to_vec(&bundle).unwrap();
    let parts = part_bodies(&bytes);
    let base = format!("/v1/machines/moves/{id}");
    for (n, part) in parts.iter().enumerate() {
        let path = format!("{base}/parts/{n}");
        forbidden(
            &rig.call("POST", &path, Some(&phone), Some(part.clone()))
                .await,
            "a phone's part",
        );
        let resp = rig
            .call("POST", &path, Some(&mac), Some(part.clone()))
            .await;
        assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    }
    let status = rig.call("GET", &base, Some(&mac), None).await.json();
    assert_eq!(status["state"], "receiving");
    let commit = json!({"parts": parts.len(), "sha256": sha256_hex(&bytes)});
    let resp = rig
        .call(
            "POST",
            &format!("{base}/commit"),
            Some(&mac),
            Some(commit.clone()),
        )
        .await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    assert_eq!(resp.json()["already"], false);
    assert_eq!(resp.json()["turns"], 1);
    let again = rig
        .call("POST", &format!("{base}/commit"), Some(&mac), Some(commit))
        .await;
    assert_eq!(again.status, 200);
    assert_eq!(again.json()["already"], true);
    let status = rig.call("GET", &base, Some(&mac), None).await.json();
    assert_eq!(
        (status["state"].as_str(), status["key"].as_str()),
        (Some("imported"), Some(key))
    );
    // It is a chat here now, listed and readable like any other.
    let detail = rig
        .call("GET", &format!("/v1/sessions/{key}"), Some(&mac), None)
        .await;
    assert_eq!(
        detail.status,
        200,
        "{}",
        String::from_utf8_lossy(&detail.body)
    );
    let home = crate::chat_move::home_of(&rig.vault, key).unwrap().unwrap();
    assert_eq!(
        (home.state.as_str(), home.peer.as_str()),
        ("arrived", CLIENT_ID)
    );
    // M5 UI: its list row and its detail say where it came from.
    assert_eq!(detail.json()["session"]["movedFrom"], CLIENT_ID);
    let list = rig
        .call("GET", "/v1/sessions", Some(&mac), None)
        .await
        .json();
    let row = list["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == key)
        .unwrap()
        .clone();
    assert_eq!(row["movedFrom"], CLIENT_ID);
    assert!(row.get("movedTo").is_none(), "absent, not null");
    // A phone's rows never carry them: the iPhone contract has no such keys.
    let phone_list = rig.call("GET", "/v1/sessions", Some(&phone), None).await;
    assert_eq!(phone_list.status, 200);
    let phone_detail = rig
        .call("GET", &format!("/v1/sessions/{key}"), Some(&phone), None)
        .await;
    assert_eq!(phone_detail.status, 200);
    for body in [&phone_list.body, &phone_detail.body] {
        let text = String::from_utf8_lossy(body);
        assert!(text.contains(key), "the moved chat is listed: {text}");
        assert!(
            !text.contains("movedFrom") && !text.contains("movedTo"),
            "{text}"
        );
    }
    // A cancel after a turn ran here is refused; the move stands.
    let cwd = crate::session_store::session_by_key(&rig.vault, key)
        .unwrap()
        .unwrap()
        .cwd;
    crate::session_store::start_turn(&rig.vault, key, "claude", &cwd, "run-2", "More").unwrap();
    let cancel = rig.call("DELETE", &base, Some(&mac), None).await;
    assert_eq!(cancel.status, 409);
    assert_eq!(cancel.json()["error"]["code"], "started");
    forbidden(
        &rig.call("DELETE", &base, Some(&phone), None).await,
        "a phone's cancel",
    );
}

/// Only the runner takes `POST /v1/machine/coo`: any other Mac's Hollow
/// answers 404 and starts nothing; a phone may not call it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_runner_lets_a_mac_start_its_coo() {
    let rig = rig().await;
    let phone = rig.token();
    let (mac, _) = rig.machine_token().await;
    let body = json!({"text": "Hi", "clientMessageId": "coo-start-1"});
    let resp = rig
        .call("POST", "/v1/machine/coo", Some(&mac), Some(body.clone()))
        .await;
    assert_eq!(resp.status, 404, "{}", String::from_utf8_lossy(&resp.body));
    let resp = rig
        .call("POST", "/v1/machine/coo", Some(&phone), Some(body))
        .await;
    assert_eq!(resp.status, 403);
    assert!(rig.host.sent.lock().unwrap().is_empty());
    let info = rig
        .call("GET", "/v1/machine", Some(&mac), None)
        .await
        .json();
    assert!(!info["capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "coo-start" || c == "coo"));
}
