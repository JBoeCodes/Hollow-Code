//! L13 COO parity, group g2: companion devices, the companion backend and
//! the iPhone app. Each Settings command has a `companion.*` action of the
//! right tier that calls a plain inner function; no result carries a
//! pairing code, a pairing link or QR code, the backend's applier token, a
//! claim code or the push key's text.

use crate::app_actions::{card_for, find, validate, Cx, Kind, Tier, ACTIONS};
use serde_json::{json, Map, Value};

/// (action id, Tauri command, tier) for every g2 action.
const G2: [(&str, &str, Tier); 23] = [
    ("companion.status", "companion_status", Tier::Read),
    (
        "companion.pairing_start",
        "companion_pairing_start",
        Tier::Confirm,
    ),
    (
        "companion.pairing_cancel",
        "companion_pairing_cancel",
        Tier::Write,
    ),
    (
        "companion.revoke_device",
        "companion_revoke_device",
        Tier::Confirm,
    ),
    (
        "companion.backend_status",
        "companion_backend_status",
        Tier::Read,
    ),
    (
        "companion.backend_set",
        "companion_backend_set",
        Tier::External,
    ),
    (
        "companion.backend_claim",
        "companion_backend_claim",
        Tier::External,
    ),
    (
        "companion.backend_repush",
        "companion_backend_repush",
        Tier::External,
    ),
    (
        "companion.backend_test",
        "companion_backend_test",
        Tier::External,
    ),
    ("companion.scope_get", "companion_scope_get", Tier::Read),
    ("companion.scope_set", "companion_scope_set", Tier::Confirm),
    ("companion.phone_status", "phone_access_status", Tier::Read),
    (
        "companion.phone_set_enabled",
        "phone_access_set_enabled",
        Tier::Confirm,
    ),
    (
        "companion.phone_pairing_start",
        "phone_access_pairing_start",
        Tier::Confirm,
    ),
    (
        "companion.phone_pairing_cancel",
        "phone_access_pairing_cancel",
        Tier::Write,
    ),
    (
        "companion.phone_revoke",
        "phone_access_revoke",
        Tier::Confirm,
    ),
    (
        "companion.phone_publish",
        "phone_access_publish",
        Tier::External,
    ),
    (
        "companion.phone_unpublish",
        "phone_access_unpublish",
        Tier::Confirm,
    ),
    ("companion.phone_lid_status", "phone_lid_status", Tier::Read),
    ("companion.phone_lid_set", "phone_lid_set", Tier::Confirm),
    (
        "companion.phone_push_import_key",
        "phone_push_import_key",
        Tier::Confirm,
    ),
    (
        "companion.phone_push_remove_key",
        "phone_push_remove_key",
        Tier::Confirm,
    ),
    (
        "companion.phone_push_test",
        "phone_push_test",
        Tier::External,
    ),
];

const TOKEN: &str = "SECRET-applier-token-0123456789";

/// A mock app managing an `AppState` on a temp vault, and a test context.
fn bed() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let dir = tempfile::Builder::new()
        .prefix("coo-g2-")
        .tempdir()
        .unwrap();
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.path().to_path_buf();
    let app = tauri::test::mock_builder()
        .manage(crate::commands::AppState::new(
            config,
            dir.path().join("config.json"),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    (dir, app)
}

/// Every required parameter, typed (as the tier audit's sweep builds them).
fn dummy_args(id: &str) -> Value {
    let action = find(id).unwrap();
    let mut args = Map::new();
    for p in action.params.iter().filter(|p| p.required) {
        let value = match p.kind {
            Kind::Str | Kind::Text | Kind::Secret | Kind::Any => json!("x"),
            Kind::Int(_) | Kind::Num => json!(1),
            Kind::Bool => json!(false),
            Kind::StrList => json!([]),
            Kind::OneOf(values) => json!(values[0]),
            Kind::Object => json!({}),
        };
        args.insert(p.name.to_string(), value);
    }
    Value::Object(args)
}

#[test]
fn every_g2_command_has_its_action_tier_and_area() {
    for (id, command, tier) in G2 {
        let action = find(id).unwrap_or_else(|| panic!("{id} is no action"));
        assert_eq!(action.command, Some(command), "{id}");
        assert_eq!(action.tier, tier, "{id}");
        assert_eq!(action.area, "companion", "{id}");
        assert!(
            !crate::app_actions::USER_ONLY.contains(&command),
            "{command} is still user-only"
        );
        assert_eq!(
            ACTIONS
                .iter()
                .filter(|a| a.command == Some(command))
                .count(),
            1,
            "{command} has one action"
        );
        validate(action, &dummy_args(id)).unwrap_or_else(|e| panic!("{id}: {e}"));
    }
    // The window's own reports stay the window's.
    for command in ["phone_access_new_chat_mode", "phone_access_chat_modes"] {
        assert!(crate::app_actions::USER_ONLY.contains(&command));
        assert!(ACTIONS.iter().all(|a| a.command != Some(command)));
    }
}

/// The handlers call the inner functions, never a `#[tauri::command]`
/// (several of which check for the main window, which an agent isn't).
#[test]
fn no_g2_handler_calls_its_tauri_command() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/app_actions.rs");
    let text = std::fs::read_to_string(src).unwrap();
    for (_, command, _) in G2 {
        assert!(
            !text.contains(&format!("::{command}(")),
            "app_actions.rs calls the command {command}"
        );
    }
}

/// A secret goes in only as a `Kind::Secret` parameter (a card shows it as
/// ••••, the audit never logs it); the push key goes in as a file path.
#[test]
fn g2_secrets_go_in_only_as_secret_params() {
    for (id, name) in [
        ("companion.backend_set", "token"),
        ("companion.backend_claim", "code"),
        ("companion.backend_test", "token"),
    ] {
        let action = find(id).unwrap();
        let param = action.params.iter().find(|p| p.name == name).unwrap();
        assert!(matches!(param.kind, Kind::Secret), "{id}.{name}");
    }
    for (id, _, _) in G2 {
        for p in find(id).unwrap().params {
            let secretish = ["token", "code", "pem", "key", "password", "secret"]
                .iter()
                .any(|word| p.name.eq_ignore_ascii_case(word));
            assert!(
                !secretish || matches!(p.kind, Kind::Secret),
                "{id}.{} must be Kind::Secret",
                p.name
            );
        }
    }
    let import = find("companion.phone_push_import_key").unwrap();
    let names: Vec<&str> = import.params.iter().map(|p| p.name).collect();
    assert_eq!(names, ["path", "keyId", "teamId"]);
}

/// Every confirm and external g2 action builds its card from the sweep's
/// arguments; a secret shows as ••••; an external one names where it goes.
#[test]
fn g2_cards_mask_secrets_and_external_ones_name_their_destination() {
    let (_dir, app) = bed();
    let cx = Cx::for_test(Box::new(app.handle().clone()));
    for (id, _, tier) in G2 {
        if !matches!(tier, Tier::Confirm | Tier::External) {
            continue;
        }
        let action = find(id).unwrap();
        // A typed backend address must be one: its card validates it first.
        let mut raw = dummy_args(id);
        if raw.get("url").is_some() {
            raw["url"] = json!("https://backend.example.ts.net");
        }
        let args = validate(action, &raw).unwrap();
        let card = card_for(action, &cx, &args)
            .unwrap_or_else(|e| panic!("{id}: {e}"))
            .unwrap_or_else(|| panic!("{id} has no card"));
        if tier == Tier::External {
            assert!(card.destination.is_some(), "{id} names no destination");
        }
    }
    for (id, args) in [
        (
            "companion.backend_set",
            json!({ "url": "https://backend.example.ts.net", "token": TOKEN }),
        ),
        (
            "companion.backend_claim",
            json!({ "url": "https://backend.example.ts.net", "code": TOKEN }),
        ),
        (
            "companion.backend_test",
            json!({ "url": "https://backend.example.ts.net", "token": TOKEN }),
        ),
    ] {
        let action = find(id).unwrap();
        let args = validate(action, &args).unwrap();
        let card = card_for(action, &cx, &args).unwrap().unwrap();
        let text = card.text();
        assert!(!text.contains(TOKEN), "{id}: {text}");
        assert!(text.contains("backend.example.ts.net"), "{id}: {text}");
    }
    // The lid's card says the person finishes it at macOS's prompt.
    let action = find("companion.phone_lid_set").unwrap();
    let args = validate(action, &json!({ "enabled": true, "onBattery": true })).unwrap();
    let text = card_for(action, &cx, &args).unwrap().unwrap().text();
    assert!(text.contains("password prompt"), "{text}");
}

/// What the COO reads of Companion devices never holds the open code.
#[test]
fn the_companion_status_an_agent_reads_holds_no_pairing_code() {
    use crate::companion_inbox::{for_agent, CompanionStatus, PairingCode};
    let status = CompanionStatus {
        devices: Vec::new(),
        pairing: Some(PairingCode {
            code: "K7M3-Q9TX".into(),
            expires_at: "2026-11-10T12:10:00Z".into(),
        }),
        waiting: 2,
    };
    let view = serde_json::to_value(for_agent(status)).unwrap();
    let text = view.to_string();
    assert!(!text.contains("K7M3") && !text.contains("Q9TX"), "{text}");
    assert_eq!(view["pairingOpen"], true);
    assert_eq!(view["pairingExpiresAt"], "2026-11-10T12:10:00Z");
    assert_eq!(view["waiting"], 2);
    assert!(view.get("pairing").is_none());
}

/// The COO's pairing start opens a code (Settings shows it) and returns
/// only that it is open and until when.
#[test]
fn the_coo_opens_a_companion_pairing_code_without_seeing_it() {
    let _serial = crate::companion_inbox::pairing_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let started = crate::companion_inbox::agent_pairing_start(dir.path(), None).unwrap();
    let open = crate::companion_inbox::open_code(chrono::Utc::now()).expect("a code is open");
    let text = serde_json::to_string(&started).unwrap();
    let bare = open.code.replace('-', "");
    assert!(
        !text.contains(&open.code) && !text.contains(&bare),
        "{text}"
    );
    assert!(started.started);
    assert_eq!(started.expires_at, open.expires_at);
    assert!(started.note.contains("Settings"), "{}", started.note);
    assert!(dir.path().join(".hollow-sync/pair").is_dir());
    crate::companion_inbox::pairing_cancel();
    assert!(crate::companion_inbox::open_code(chrono::Utc::now()).is_none());
}

/// What the COO reads of the iPhone app never holds the code, the pairing
/// link or its QR code.
#[test]
fn the_phone_status_an_agent_reads_holds_no_code_link_or_qr() {
    use crate::phone_access::{for_agent, MachinePairing, PhoneAccessStatus, PhonePairing};
    let status = PhoneAccessStatus {
        // M2: the machine window's code is never an agent's either.
        machine_pairing: Some(MachinePairing {
            code: "M4CH-7NE2".into(),
            expires_at: "2026-11-10T12:20:00Z".into(),
        }),
        available: true,
        enabled: true,
        mac_name: "Test Mac".into(),
        pairing: Some(PhonePairing {
            code: "ABCD-EFGH".into(),
            expires_at: "2026-11-10T12:10:00Z".into(),
            link: Some(
                "hollow-companion://pair?url=https%3A%2F%2Fmac.ts.net%3A8443&code=ABCDEFGH&mac=Test"
                    .into(),
            ),
            qr_svg: Some("<svg>QR-MODULES</svg>".into()),
        }),
        ..PhoneAccessStatus::default()
    };
    let view = for_agent(status);
    let text = view.to_string();
    for leak in [
        "ABCD-EFGH",
        "ABCDEFGH",
        "hollow-companion://",
        "<svg",
        "QR-MODULES",
        "M4CH-7NE2",
        "M4CH7NE2",
    ] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
    assert!(view.get("pairing").is_none(), "{text}");
    assert!(view.get("machinePairing").is_none(), "{text}");
    assert_eq!(view["machinePairingOpen"], true);
    assert_eq!(view["machinePairingExpiresAt"], "2026-11-10T12:20:00Z");
    assert_eq!(view["pairingOpen"], true);
    assert_eq!(view["pairingExpiresAt"], "2026-11-10T12:10:00Z");
    assert_eq!(view["macName"], "Test Mac");
    let closed = for_agent(PhoneAccessStatus::default());
    assert_eq!(closed["pairingOpen"], false);
    assert!(closed["pairingExpiresAt"].is_null());
    assert_eq!(closed["machinePairingOpen"], false);
}

/// Outside the Hollow that serves the iPhone app (every unit test), the
/// COO's phone actions refuse in words and the status says unavailable.
#[test]
fn the_phone_actions_refuse_where_the_iphone_app_isnt_served() {
    let status = tauri::async_runtime::block_on(crate::phone_access::agent_status(false));
    assert!(status.get("pairing").is_none());
    assert!(status.get("pairingOpen").is_some());
    let error = tauri::async_runtime::block_on(crate::phone_access::agent_pairing_start());
    if let Err(error) = error {
        assert!(!error.is_empty());
    }
    assert!(
        tauri::async_runtime::block_on(crate::phone_access::lid::agent_lid_set(true, false))
            .is_err()
    );
}

/// The backend's status never carries the applier token, and the COO's
/// copies mask it even where an error or a test's sentence quoted it.
#[test]
fn the_backend_results_an_agent_reads_never_carry_the_token() {
    use crate::companion_bridge::{
        load_config, masked, masked_test, save_config, scrubbed, status, BackendStatus, BackendTest,
    };
    let dir = tempfile::tempdir().unwrap();
    let store = crate::companion_inbox::Store::at(dir.path().to_path_buf());
    save_config(&store, "https://backend.example.ts.net", TOKEN).unwrap();
    assert_eq!(load_config(&store).unwrap().token, TOKEN, "it is saved");
    let saved = serde_json::to_string(&status(&store)).unwrap();
    assert!(!saved.contains(TOKEN), "{saved}");
    assert!(saved.contains("https://backend.example.ts.net"), "{saved}");

    let quoted = BackendStatus {
        last_error: Some(format!("The backend answered 401: bad token {TOKEN}")),
        mirror_error: Some(format!("{TOKEN} refused")),
        docs_error: Some("no documents".into()),
        scope_error: None,
        ..BackendStatus::default()
    };
    let clean = serde_json::to_string(&scrubbed(quoted, TOKEN)).unwrap();
    assert!(!clean.contains(TOKEN), "{clean}");
    assert!(clean.contains("bad token ••••"), "{clean}");
    assert!(clean.contains("no documents"), "{clean}");

    let test = masked_test(
        BackendTest {
            ok: false,
            message: format!("The backend at h answered 409: token {TOKEN} is in use"),
            version: None,
        },
        TOKEN,
    );
    assert!(!test.message.contains(TOKEN), "{}", test.message);
    // A blank secret masks nothing; any other token is masked, however short.
    assert_eq!(masked("abc".into(), ""), "abc");
    assert_eq!(masked("abc".into(), "  "), "abc");
    assert_eq!(masked("xabcx".into(), "abc"), "x••••x");
}

/// A 3-byte token is masked in a test's sentence, and a version that holds
/// it is dropped (L13 delta review: the old floor let tokens under 4 bytes through).
#[test]
fn a_three_byte_token_is_masked_and_never_returned_as_a_version() {
    use crate::companion_bridge::{masked_test, BackendTest};
    const SHORT: &str = "q7z";
    let test = masked_test(
        BackendTest {
            ok: true,
            message: format!("Connected with token {SHORT}."),
            version: Some(format!("1.{SHORT}.0")),
        },
        SHORT,
    );
    assert!(!test.message.contains(SHORT), "{}", test.message);
    assert_eq!(test.version, None);
    let kept = masked_test(
        BackendTest {
            ok: true,
            message: "Connected.".into(),
            version: Some("0.4.2".into()),
        },
        SHORT,
    );
    assert_eq!(kept.version.as_deref(), Some("0.4.2"));
}

/// The scope the COO sets: work data and the personal workspaces (an id
/// that isn't one is dropped); omitting the list keeps the saved one.
#[test]
fn the_coo_sets_the_backend_scope_and_keeps_the_list_when_omitted() {
    use crate::companion_bridge::scope_set_in;
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let store = crate::companion_inbox::Store::at(dir.path().join("store"));
    let view = scope_set_in(
        &vault,
        &store,
        true,
        Some(vec!["personal-ws".into(), "../outside".into()]),
    )
    .unwrap();
    let view = serde_json::to_value(view).unwrap();
    assert_eq!(view["includeWorkData"], true);
    let (scope, error) = crate::companion_docs::load_scope_checked(&store, &vault);
    assert!(error.is_none(), "{error:?}");
    assert!(scope.include_work_data);
    assert_eq!(scope.personal_workspaces, ["personal-ws"]);
    assert!(scope.seeded_at.is_some());
    scope_set_in(&vault, &store, false, None).unwrap();
    let (scope, _) = crate::companion_docs::load_scope_checked(&store, &vault);
    assert!(!scope.include_work_data);
    assert_eq!(scope.personal_workspaces, ["personal-ws"]);
}

/// The scope card says old and new, and warns when the change widens it.
#[test]
fn the_scope_card_warns_when_it_widens() {
    let new = vec!["zz-coo-l13-not-a-workspace".to_string()];
    let lines = crate::companion_bridge::scope_change_lines(true, Some(new.as_slice()));
    let text = lines.join("\n");
    assert!(text.contains("Work data"), "{text}");
    assert!(text.contains("zz-coo-l13-not-a-workspace"), "{text}");
    assert!(text.contains("widens"), "{text}");
}

/// The lid switch asks macOS for nothing when it is already set that way.
#[test]
fn the_lid_switch_knows_when_nothing_would_change() {
    use crate::phone_access::lid::{already_set, PhoneLidStatus, PROMPT_NOTE};
    let off = PhoneLidStatus::default();
    assert!(already_set(&off, false, false));
    assert!(!already_set(&off, true, false));
    let on = PhoneLidStatus {
        enabled: true,
        on_battery: false,
        ..PhoneLidStatus::default()
    };
    assert!(already_set(&on, true, false));
    assert!(!already_set(&on, true, true), "On battery too changes it");
    assert!(!already_set(&on, false, false), "off removes it");
    let outdated = PhoneLidStatus {
        outdated: true,
        ..on
    };
    assert!(
        !already_set(&outdated, true, false),
        "an outdated helper updates"
    );
    assert!(PROMPT_NOTE.contains("password"));
}

// ── L13 review fixes (set A) ─────────────────────────────────────────────

/// A backend's own words are masked with the token in use before they are
/// kept, so a status read after the token changed can't show the old one.
#[test]
fn a_kept_backend_error_never_carries_the_token_it_was_made_with() {
    use crate::companion_bridge::{backend_answered, scrubbed, BackendStatus};
    let old = "OLD-applier-token-abcdef0123";
    let kept = backend_answered(500, &format!("bearer {old} is not allowed"), old);
    assert!(!kept.contains(old), "{kept}");
    assert!(kept.contains("500"), "{kept}");
    // Read later, with another token saved: still nothing of the old one.
    let status = BackendStatus {
        docs_error: Some(kept.clone()),
        last_error: Some(kept),
        ..BackendStatus::default()
    };
    let text = serde_json::to_string(&scrubbed(status, TOKEN)).unwrap();
    assert!(!text.contains(old), "{text}");
}

/// A test's version is the backend's own: kept only when it looks like a
/// version and doesn't hold the token.
#[test]
fn a_backend_tests_version_is_sanitized_too() {
    use crate::companion_bridge::{masked_test, BackendTest};
    let test = |version: &str| {
        masked_test(
            BackendTest {
                ok: true,
                message: "Connected.".into(),
                version: Some(version.into()),
            },
            TOKEN,
        )
        .version
    };
    assert_eq!(test("1.4.2+build.7"), Some("1.4.2+build.7".into()));
    assert_eq!(test(TOKEN), None);
    assert_eq!(test(&format!("v1 ({TOKEN})")), None);
    assert_eq!(test("<script>alert(1)</script>"), None);
    assert_eq!(test(&"9".repeat(41)), None);
}

/// A failed claim reads in fixed words whatever the backend said, in any
/// form of the code.
#[test]
fn a_failed_claim_never_quotes_the_code_in_any_form() {
    use crate::companion_bridge::agent_claim_error;
    for said in [
        "The backend answered 403: code k7m3q9tx is not valid",
        "The backend answered 403: K7M3-Q9TX expired",
        "The backend answered 400: K7M3Q9TX",
    ] {
        let error = agent_claim_error(said);
        for form in ["k7m3q9tx", "K7M3-Q9TX", "K7M3Q9TX", "k7m3", "K7M3"] {
            assert!(!error.contains(form), "{form} in {error}");
        }
        assert!(error.contains("claim-failed"), "{error}");
    }
    let safe = agent_claim_error("The backend's answer had no usable token.");
    assert!(
        safe.starts_with("The backend's answer had no usable token."),
        "{safe}"
    );
}

/// A backend card validates the typed address first and shows only
/// scheme://host[:port]/path: a user name, password, query or fragment
/// (where a secret can hide) is refused without being quoted.
#[test]
fn backend_cards_validate_the_address_and_never_show_its_secret_parts() {
    use crate::companion_bridge::card_backend_url;
    assert_eq!(
        card_backend_url("https://backend.example.ts.net:8443/hollow/").unwrap(),
        "https://backend.example.ts.net:8443/hollow"
    );
    let (_dir, app) = bed();
    let cx = Cx::for_test(Box::new(app.handle().clone()));
    for url in [
        "https://user:SECRET-PW@backend.example.ts.net",
        "https://backend.example.ts.net/?key=SECRET-PW",
        "https://backend.example.ts.net/#SECRET-PW",
    ] {
        let refused = card_backend_url(url).unwrap_err();
        assert!(!refused.contains("SECRET-PW"), "{refused}");
        for (id, args) in [
            (
                "companion.backend_set",
                json!({ "url": url, "token": TOKEN }),
            ),
            (
                "companion.backend_claim",
                json!({ "url": url, "code": "K7M3-Q9TX" }),
            ),
            (
                "companion.backend_test",
                json!({ "url": url, "token": TOKEN }),
            ),
        ] {
            let action = find(id).unwrap();
            let args = validate(action, &args).unwrap();
            let error = card_for(action, &cx, &args).unwrap_err();
            assert!(!error.contains("SECRET-PW"), "{id}: {error}");
        }
    }
}

/// Revoking takes the pass lock first and runs its change (bound to the
/// caller's History lease by the COO's action) with it held: the order a
/// pass takes them in, so the two can't deadlock.
#[test]
fn revoking_holds_the_pass_lock_while_its_bound_change_runs() {
    use crate::companion_inbox::{pass_lock, revoke_bound, Store};
    use std::sync::mpsc;
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let store = Store::at(dir.path().join("store"));
    let mut bound = false;
    let result = revoke_bound(&vault, &store, "no-such-device", |change| {
        bound = true;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _pass = pass_lock();
            let _ = tx.send(());
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(200))
                .is_err(),
            "the pass lock is held while the bound change runs"
        );
        change()
    });
    assert!(bound);
    assert!(result.unwrap_err().contains("no paired device"));
    assert_eq!(
        find("companion.revoke_device").unwrap().lease,
        crate::app_actions::Lease::Own
    );
}

/// What an agent reads of the iPhone app never holds the record's or a
/// push's own error text (either can quote a phone's push token), and a
/// failed change reads in fixed words.
#[test]
fn the_phone_results_an_agent_reads_carry_no_record_or_push_error_text() {
    use crate::phone_access::{agent_error, for_agent, PhoneAccessStatus};
    let apns = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
    let status = PhoneAccessStatus {
        available: true,
        record_error: Some(format!(
            "phone.json is damaged: invalid type: string \"{apns}\""
        )),
        push: Some(Default::default()),
        ..PhoneAccessStatus::default()
    };
    let mut status = status;
    if let Some(push) = status.push.as_mut() {
        push.last_error = Some(format!("Apple refused {apns}: BadDeviceToken"));
    }
    let text = for_agent(status).to_string();
    assert!(!text.contains(apns), "{text}");
    assert!(
        text.contains("phone-record") && text.contains("phone-push"),
        "{text}"
    );
    let error = agent_error(
        "revoke that iPhone",
        format!("phone.json: expected `,` at {apns}"),
    );
    assert!(
        !error.contains(apns) && error.contains("phone-error"),
        "{error}"
    );
    let fixed = "Turn on the iPhone app first, so the iPhone can reach this Mac.";
    assert_eq!(agent_error("x", fixed.into()), fixed);
}

/// A helper installed for another copy of Hollow is never "already set":
/// the COO's switch rebinds it, as the window's does.
#[test]
fn the_lid_switch_rebinds_a_helper_of_another_copy() {
    use crate::phone_access::lid::{already_set, PhoneLidStatus};
    let other = PhoneLidStatus {
        enabled: true,
        other_copy: Some("/Applications/Hollow Canary.app/Contents/MacOS/hollow".into()),
        ..PhoneLidStatus::default()
    };
    assert!(!already_set(&other, true, false));
    assert!(!already_set(&other, false, false), "off removes it");
}
