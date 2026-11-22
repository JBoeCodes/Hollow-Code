//! L13 COO parity, group g6: custom icons, settings export and import, quit
//! and running work, dictation status, diagnostics, chat limits, the COO's
//! own settings, schedule windows, tab grants and canvas export. Each of
//! these was a user-only Tauri command; each now has an agent action with the
//! tier below. The integrator adds `#[cfg(test)] mod coo_parity_tests_g6;` to
//! lib.rs.

use crate::app_actions::{find, validate, Cx, Kind, Tier};
use crate::commands::AppState;
use serde_json::{json, Value};

/// (action id, command it mirrors, tier, area).
const ACTIONS_G6: &[(&str, &str, Tier, &str)] = &[
    (
        "settings.read_custom_icon",
        "read_custom_icon",
        Tier::Read,
        "settings",
    ),
    (
        "settings.prepare_custom_icon",
        "prepare_custom_icon",
        Tier::Read,
        "settings",
    ),
    (
        "settings.save_custom_icon",
        "save_custom_icon",
        Tier::Write,
        "settings",
    ),
    (
        "settings.reset_custom_icon",
        "reset_custom_icon",
        Tier::Confirm,
        "settings",
    ),
    (
        "settings.select_app_icon_preset",
        "select_app_icon_preset",
        Tier::Write,
        "settings",
    ),
    (
        "system.exit_running_work",
        "exit_running_work",
        Tier::Read,
        "system",
    ),
    (
        "system.quit_hollow",
        "resolve_quit",
        Tier::Confirm,
        "system",
    ),
    (
        "settings.set_main_agent_settings",
        "set_main_agent_settings",
        Tier::Confirm,
        "settings",
    ),
    (
        "settings.set_chat_limits",
        "set_chat_limits",
        Tier::Confirm,
        "settings",
    ),
    (
        "system.save_diagnostics",
        "save_diagnostics",
        Tier::Write,
        "system",
    ),
    (
        "settings.settings_export",
        "settings_export",
        Tier::Write,
        "settings",
    ),
    (
        "settings.settings_import_preview",
        "settings_import_preview",
        Tier::Read,
        "settings",
    ),
    (
        "settings.settings_import_apply",
        "settings_import_apply",
        Tier::Confirm,
        "settings",
    ),
    (
        "system.dictation_status",
        "dictation_status",
        Tier::Read,
        "system",
    ),
    (
        "canvases.export_canvas",
        "canvas_export",
        Tier::Write,
        "canvases",
    ),
    ("ui.revoke_tab", "browser_revoke_tab", Tier::Write, "ui"),
    ("ui.list_tab_grants", "browser_tab_grants", Tier::Read, "ui"),
    (
        "automations.set_automation_windows",
        "set_automation_windows",
        Tier::Confirm,
        "automations",
    ),
];

fn bed() -> (tempfile::TempDir, tauri::App<tauri::test::MockRuntime>) {
    let dir = tempfile::Builder::new()
        .prefix("coo-g6-")
        .tempdir()
        .unwrap();
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.path().to_path_buf();
    config.claude_projects_path = dir.path().join("no-claude-projects");
    let app = tauri::test::mock_builder()
        .manage(AppState::new(config, dir.path().join("config.json")))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    (dir, app)
}

fn cx(app: &tauri::App<tauri::test::MockRuntime>) -> Cx {
    Cx::for_test(Box::new(app.handle().clone()))
}

#[test]
fn every_g6_action_resolves_with_its_tier_command_and_area() {
    for (id, command, tier, area) in ACTIONS_G6 {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(action.tier, *tier, "{id}");
        assert_eq!(action.command, Some(*command), "{id}");
        assert_eq!(action.area, *area, "{id}");
        assert!(action.id.starts_with(&format!("{area}.")), "{id}");
        assert!(!action.summary.contains("Main agent"), "{id}: say COO");
        for param in action.params {
            assert!(!param.about.is_empty(), "{id}.{}", param.name);
        }
    }
}

#[test]
fn the_g6_actions_take_no_secret_and_the_icon_read_cannot_ask_for_the_original() {
    for (id, ..) in ACTIONS_G6 {
        let action = find(id).unwrap();
        assert!(
            action
                .params
                .iter()
                .all(|p| !matches!(p.kind, Kind::Secret)),
            "{id} takes a secret param but none of these should"
        );
    }
    let read = find("settings.read_custom_icon").unwrap();
    assert!(read.params.iter().all(|p| p.name == "target"));
    assert!(validate(read, &json!({ "target": "app", "includeOriginal": true })).is_err());
    assert!(validate(read, &json!({ "target": "dock" })).is_err());
}

#[test]
fn a_call_with_wrong_arguments_is_refused_by_validation() {
    let quit = find("system.quit_hollow").unwrap();
    assert!(validate(quit, &json!({ "choice": "cancel" })).is_err());
    assert!(validate(quit, &json!({})).is_err());
    assert!(validate(quit, &json!({ "choice": "tray" })).is_ok());
    let preset = find("settings.select_app_icon_preset").unwrap();
    assert!(validate(preset, &json!({ "preset": "fold" })).is_ok());
    assert!(validate(preset, &json!({ "preset": "neon" })).is_err());
    let apply = find("settings.settings_import_apply").unwrap();
    assert!(validate(apply, &json!({ "path": "/tmp/s.json" })).is_err());
    let export = find("canvases.export_canvas").unwrap();
    assert!(validate(
        export,
        &json!({ "workspaceId": "w", "source": "x", "format": "gif" })
    )
    .is_err());
    let limits = find("settings.set_chat_limits").unwrap();
    assert!(validate(limits, &json!({ "chatsAtOnce": 61 })).is_err());
    // L13 (2026-11-12): the COO reaches the per-chat subagent cap here too.
    assert!(validate(limits, &json!({ "subagentsPerChat": 2 })).is_ok());
    assert!(validate(limits, &json!({ "subagentsPerChat": 21 })).is_err());
}

#[test]
fn an_icon_summary_and_a_prepare_check_carry_no_picture_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("private.png");
    image::RgbaImage::from_pixel(64, 32, image::Rgba([9, 99, 199, 255]))
        .save(&path)
        .unwrap();
    let record = crate::custom_icons::record_from_file(&path).unwrap();
    let shown = crate::custom_icons::summary("mainAgent", Some(&record)).to_string();
    let prepared = crate::custom_icons::inspect_file(&path)
        .unwrap()
        .to_string();
    for text in [&shown, &prepared] {
        assert!(!text.contains(&record.original.bytes));
        assert!(!text.contains("data:image"));
        for png in record.pngs.values() {
            assert!(!text.contains(png));
        }
    }
    assert!(shown.contains(&record.revision));
    assert!(prepared.contains("\"width\":64"));
}

#[test]
fn settings_export_never_carries_a_secret_key_and_its_result_has_no_values() {
    use crate::settings_keys::{Type, KEYS};
    let secrets: Vec<&str> = KEYS
        .iter()
        .filter(|k| matches!(k.kind, Type::Secret))
        .map(|k| k.key)
        .collect();
    assert!(!secrets.is_empty(), "the registry has secret keys");
    for key in KEYS.iter().filter(|k| matches!(k.kind, Type::Secret)) {
        assert!(!crate::settings_transfer::exportable(key), "{}", key.key);
    }
    // The exported result names a path, a count and unreadable keys: no value.
    let result = serde_json::to_value(crate::settings_transfer::Exported {
        path: "/tmp/s.json".into(),
        count: 3,
        skipped: vec!["theme".into()],
    })
    .unwrap();
    let mut fields: Vec<&str> = result
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort();
    assert_eq!(fields, ["count", "path", "skipped"]);
    // A file naming a secret key never imports it.
    let secret = secrets[0];
    let mut settings = serde_json::Map::new();
    settings.insert(secret.to_string(), json!("sk-live-123"));
    let file = json!({ "hollowSettings": 1, "settings": settings });
    let preview = crate::settings_transfer::plan(&file, &serde_json::Map::new(), &[]).unwrap();
    assert!(preview.changes.is_empty());
    assert_eq!(preview.ignored.len(), 1);
    assert!(!serde_json::to_string(&preview)
        .unwrap()
        .contains("sk-live-123"));
}

#[test]
fn an_import_card_lists_every_confirm_change_and_caps_the_quiet_ones() {
    use crate::settings_transfer::{card_lines, Change, Preview};
    let change = |key: String, asks: bool| Change {
        key,
        summary: String::new(),
        old: "off".into(),
        new: "on".into(),
        asks,
        value: json!(true),
    };
    let mut changes = vec![
        change("agentTools.app".into(), true),
        change("agentTools.tasks".into(), true),
    ];
    changes.extend((0..50).map(|n| change(format!("quiet.{n:02}"), false)));
    let preview = Preview {
        changes,
        unchanged: 7,
        ignored: vec![],
        exported_at: None,
        hash: "h".into(),
    };
    let lines = card_lines(&preview);
    assert_eq!(lines[0], "52 setting(s) change, 7 already match.");
    assert!(lines.contains(&"agentTools.app: off → on".to_string()));
    assert!(lines.contains(&"agentTools.tasks: off → on".to_string()));
    assert!(lines.contains(&"quiet.39: off → on".to_string()));
    assert!(!lines.contains(&"quiet.40: off → on".to_string()));
    assert!(lines.contains(&"and 10 more low-risk setting(s)".to_string()));
    assert!(lines.iter().any(|l| l.contains("what agents may do")));
}

#[test]
fn the_import_card_needs_the_running_app_and_refuses_before_asking() {
    let (_dir, app) = bed();
    let action = find("settings.settings_import_apply").unwrap();
    let args = validate(
        action,
        &json!({ "path": "/tmp/none.json", "expectedHash": "x" }),
    )
    .unwrap();
    let card = crate::app_actions::card_for(action, &cx(&app), &args);
    assert!(card.is_err(), "{card:?}");
}

#[test]
fn the_cards_of_the_limit_changes_show_old_to_new_and_refuse_a_no_op() {
    let (_dir, app) = bed();
    let cx = cx(&app);
    let card_text = |id: &str, args: Value| -> Result<String, String> {
        let action = find(id).unwrap();
        let args = validate(action, &args)?;
        crate::app_actions::card_for(action, &cx, &args)
            .map(|card| card.expect("a confirm action has a card").text())
    };
    let limits = card_text("settings.set_chat_limits", json!({ "chatsAtOnce": 20 })).unwrap();
    assert!(
        limits.contains("Chats at once across Hollow: 10 → 20"),
        "{limits}"
    );
    assert!(limits.contains("more memory"), "{limits}");
    let subagents =
        card_text("settings.set_chat_limits", json!({ "subagentsPerChat": 2 })).unwrap();
    assert!(
        subagents.contains("Subagents at once from one chat: 6 → 2"),
        "{subagents}"
    );
    assert!(
        !subagents.contains("more memory"),
        "a lowered cap warns of nothing: {subagents}"
    );
    let raised = card_text("settings.set_chat_limits", json!({ "subagentsPerChat": 8 })).unwrap();
    assert!(
        raised.contains("6 → 8") && raised.contains("more memory"),
        "{raised}"
    );
    assert!(card_text("settings.set_chat_limits", json!({ "subagentsPerChat": 6 })).is_err());
    assert!(card_text(
        "settings.set_chat_limits",
        json!({ "subagentsPerChat": 21 })
    )
    .is_err());
    assert!(card_text("settings.set_chat_limits", json!({})).is_err());
    assert!(card_text(
        "settings.set_chat_limits",
        json!({ "chatsPerWorkspace": 26 })
    )
    .is_err());

    let coo = card_text(
        "settings.set_main_agent_settings",
        json!({ "dailyCapUsd": 500.0 }),
    )
    .unwrap();
    assert!(coo.contains("Daily cap: $100 → $500"), "{coo}");
    assert!(coo.contains("COO"), "{coo}");
    assert!(card_text("settings.set_main_agent_settings", json!({})).is_err());
    assert!(card_text(
        "settings.set_main_agent_settings",
        json!({ "intervalMin": 1 })
    )
    .is_err());

    let windows = card_text(
        "automations.set_automation_windows",
        json!({ "overnightCapUsd": 90.0 }),
    )
    .unwrap();
    assert!(windows.contains("Overnight cap: $40 → $90"), "{windows}");
    assert!(windows.contains("higher cap"), "{windows}");
    assert!(card_text("automations.set_automation_windows", json!({})).is_err());
    assert!(card_text(
        "automations.set_automation_windows",
        json!({ "windows": [{ "name": "Bad", "label": "x", "days": ["mon"], "start": "18:00", "end": "07:00" }] })
    )
    .is_err());
}

#[test]
fn the_quit_card_says_it_ends_every_turn_and_a_tray_choice_says_it_does_not() {
    let (_dir, app) = bed();
    let cx = cx(&app);
    let action = find("system.quit_hollow").unwrap();
    let quit = validate(action, &json!({ "choice": "quit" })).unwrap();
    let card = crate::app_actions::card_for(action, &cx, &quit)
        .unwrap()
        .unwrap()
        .text();
    assert!(
        card.contains("ends every running turn, including this chat's"),
        "{card}"
    );
    let tray = validate(action, &json!({ "choice": "tray" })).unwrap();
    let card = crate::app_actions::card_for(action, &cx, &tray)
        .unwrap()
        .unwrap()
        .text();
    assert!(card.contains("Running turns keep going"), "{card}");
}

#[test]
fn replacing_an_uploaded_icon_asks_but_a_fresh_install_does_not() {
    // The test process's app-data folder has no saved icon: nothing to lose.
    let (_dir, app) = bed();
    let action = find("settings.save_custom_icon").unwrap();
    let args = validate(action, &json!({ "target": "app", "path": "/tmp/x.png" })).unwrap();
    let card = crate::app_actions::card_for(action, &cx(&app), &args).unwrap();
    assert!(card.is_none(), "{card:?}");
}

#[test]
fn a_tab_grant_is_listed_and_revoking_it_removes_it_from_the_list() {
    let (_dir, app) = bed();
    let run = |id: &str, args: Value| -> Value {
        let action = find(id).unwrap();
        let args = validate(action, &args).unwrap();
        tauri::async_runtime::block_on((action.run)(cx(&app), args)).unwrap()
    };
    crate::tab_grants::grant("g6-tab-a", "s-20261110-g6-a", "Chat A").unwrap();
    crate::tab_grants::grant("g6-tab-b", "s-20261110-g6-b", "Chat B").unwrap();
    let listed = run("ui.list_tab_grants", json!({}));
    let ids: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|g| g["tabId"].as_str())
        .collect();
    assert!(
        ids.contains(&"g6-tab-a") && ids.contains(&"g6-tab-b"),
        "{listed}"
    );
    let first = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["tabId"] == "g6-tab-a")
        .unwrap();
    assert_eq!(first["title"], "Chat A");
    let revoked = run("ui.revoke_tab", json!({ "tabId": "g6-tab-a" }));
    assert_eq!(revoked["revoked"], "g6-tab-a");
    let after = run("ui.list_tab_grants", json!({}));
    let after: Vec<&str> = after
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|g| g["tabId"].as_str())
        .collect();
    assert!(!after.contains(&"g6-tab-a"), "{after:?}");
    assert!(after.contains(&"g6-tab-b"), "{after:?}");
    run("ui.revoke_tab", json!({ "tabId": "g6-tab-b" }));
}

#[test]
fn a_diagnostics_save_goes_outside_the_vault_and_returns_only_its_path_size_and_count() {
    let vault = tempfile::tempdir().unwrap();
    let home = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let sources = crate::diagnostics_bundle::Sources {
        vault: vault.path().to_path_buf(),
        home: home.path().to_path_buf(),
        debug_dir: home.path().join("cache"),
        logs_dir: home.path().join("logs"),
    };
    let saved =
        crate::diagnostics_bundle::save(&sources, out.path(), chrono::Local::now()).unwrap();
    let value = serde_json::to_value(&saved).unwrap();
    let mut fields: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    fields.sort();
    assert_eq!(fields, ["bytes", "entries", "path"]);
    assert!(std::path::Path::new(&saved.path).is_file());
    // A folder inside the vault is refused: its files would be indexed and synced.
    let inside = vault.path().join("exports");
    std::fs::create_dir(&inside).unwrap();
    let refused = crate::diagnostics_bundle::save(&sources, &inside, chrono::Local::now());
    assert!(refused.unwrap_err().contains("outside the vault"));
}

#[test]
fn a_coo_settings_export_never_replaces_a_file() {
    // The same hard-link rule the action relies on: a name that exists fails.
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("settings.json");
    std::fs::write(&target, "mine").unwrap();
    let tmp = dir.path().join(".partial");
    std::fs::write(&tmp, "new").unwrap();
    let linked = std::fs::hard_link(&tmp, &target);
    assert_eq!(
        linked.unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "mine");
}

/// L13 review: the COO's settings export never lands inside the vault (its
/// write would bypass History), however the path is spelled; outside is fine.
#[test]
fn a_coo_settings_export_inside_the_vault_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path().join("vault");
    std::fs::create_dir_all(vault.join("notes")).unwrap();
    let refused = |target: std::path::PathBuf| {
        crate::settings_transfer::outside_vault(&vault, &target).unwrap_err()
    };
    assert!(refused(vault.join("settings.json")).contains("outside the vault"));
    assert!(refused(vault.join("notes/settings.json")).contains("outside the vault"));
    // Through a `..` that walks back in, and through a link to a vault folder.
    let dotted = dir.path().join("vault/notes/../settings.json");
    assert!(refused(dotted).contains("outside the vault"));
    #[cfg(unix)]
    {
        let link = dir.path().join("linked");
        std::os::unix::fs::symlink(vault.join("notes"), &link).unwrap();
        assert!(refused(link.join("settings.json")).contains("outside the vault"));
    }
    assert!(crate::settings_transfer::outside_vault(&vault, &dir.path().join("s.json")).is_ok());
}

/// L13 review: an agent's import preview and its card mask a secret nested
/// in an object setting (`heartbeat.config.apiKey`) on both sides, while the
/// hash and the value applied stay exact.
#[test]
fn an_agents_import_preview_redacts_a_nested_api_key() {
    use crate::settings_transfer::{card_lines, for_agent, plan};
    let new_key = "sk-ant-api03-NewSecretValue0123456789";
    let old_key = "sk-old-OldSecretValue0123456789";
    let file = json!({
        "hollowSettings": 1,
        "settings": { "heartbeat.config": { "workHoursStart": 9, "apiKey": new_key } },
    });
    let mut current = serde_json::Map::new();
    current.insert(
        "heartbeat.config".into(),
        json!({ "workHoursStart": 8, "apiKey": old_key }),
    );
    let exact = plan(&file, &current, &[]).unwrap();
    assert_eq!(exact.changes.len(), 1);
    let hash = exact.hash.clone();
    let shown = for_agent(exact, &current);
    assert_eq!(shown.hash, hash, "the hash binds the exact values");
    assert_eq!(
        shown.changes[0].value["apiKey"],
        json!(new_key),
        "applied exactly"
    );
    let text = serde_json::to_string(&shown).unwrap();
    let card = card_lines(&shown).join("\n");
    for out in [&text, &card] {
        assert!(!out.contains("NewSecretValue"), "{out}");
        assert!(!out.contains("OldSecretValue"), "{out}");
        assert!(out.contains("••••"), "{out}");
    }
    assert!(
        card.contains("workHoursStart"),
        "the rest of the change still shows: {card}"
    );
}

/// L13 merge: the CFO's policy (COO rebuild L7) is the COO's too, through
/// two reads and two confirm actions that ask in `auto`.
const CFO_ACTIONS: &[(&str, &str, Tier)] = &[
    ("settings.cfo_settings", "cfo_settings", Tier::Read),
    ("settings.cfo_view", "cfo_view", Tier::Read),
    (
        "settings.cfo_save_settings",
        "cfo_save_settings",
        Tier::Confirm,
    ),
    (
        "settings.cfo_approve_extra_usage",
        "cfo_approve_extra_usage",
        Tier::Confirm,
    ),
];

#[test]
fn the_cfo_actions_resolve_with_their_tier_and_command_and_ask_in_auto() {
    for (id, command, tier) in CFO_ACTIONS {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(action.tier, *tier, "{id}");
        assert_eq!(action.command, Some(*command), "{id}");
        assert_eq!(action.area, "settings", "{id}");
        assert!(!action.summary.contains("Main agent"), "{id}: say COO");
        assert!(
            !crate::app_actions::USER_ONLY.contains(command),
            "{command} has an action now"
        );
        if *tier == Tier::Confirm {
            assert!(crate::app_actions::auto_asks(id), "{id} must ask in auto");
            assert!(action.card.is_some(), "{id} shows old to new");
            assert_eq!(action.lease, crate::app_actions::Lease::Outside, "{id}");
        }
    }
}

#[test]
fn approving_extra_usage_stays_within_0_to_72_hours() {
    let approve = find("settings.cfo_approve_extra_usage").unwrap();
    assert!(validate(approve, &json!({ "hours": 73 })).is_err());
    assert!(validate(approve, &json!({})).is_err());
    assert!(validate(approve, &json!({ "hours": 72 })).is_ok());
    assert!(validate(approve, &json!({ "hours": 0 })).is_ok());
    assert!(crate::cfo::approve_card_line(73).is_err());
    assert!(crate::cfo::approve_card_line(0)
        .unwrap()
        .contains("Withdraw"));
    assert!(crate::cfo::approve_card_line(5)
        .unwrap()
        .contains("next 5 hours"));
    let dir = tempfile::tempdir().unwrap();
    let err = tauri::async_runtime::block_on(crate::cfo::approve_extra_usage_in(
        dir.path().to_path_buf(),
        73,
    ))
    .unwrap_err();
    assert!(err.contains("at most 72"), "{err}");
    let saved = crate::cfo::settings::peek(dir.path()).unwrap();
    assert_eq!(saved.extra_usage_until, None, "nothing approved");
    tauri::async_runtime::block_on(crate::cfo::approve_extra_usage_in(
        dir.path().to_path_buf(),
        2,
    ))
    .unwrap();
    let saved = crate::cfo::settings::peek(dir.path()).unwrap();
    assert!(saved.extra_usage_open(chrono::Utc::now()));
}

#[test]
fn saving_the_cfo_policy_refuses_a_model_that_is_not_its_agents() {
    use crate::cfo::settings::{AllowEntry, Patch};
    use std::collections::{BTreeMap, BTreeSet};
    let save = find("settings.cfo_save_settings").unwrap();
    assert!(validate(save, &json!({ "reservePercent": 91 })).is_err());
    assert!(validate(save, &json!({ "effortMode": "cheap" })).is_err());
    let dir = tempfile::tempdir().unwrap();
    let catalogs = BTreeMap::from([(
        "claude".to_string(),
        BTreeSet::from(["claude-sonnet-5-5".to_string()]),
    )]);
    let patch = |model: &str| Patch {
        allow: Some(vec![AllowEntry {
            agent: "claude".into(),
            model: Some(model.into()),
        }]),
        ..Patch::default()
    };
    let err =
        crate::cfo::save_checked(dir.path(), patch("claude-imaginary-9"), &catalogs).unwrap_err();
    assert!(err.contains("is not one of"), "{err}");
    assert!(
        crate::cfo::settings::peek(dir.path())
            .unwrap()
            .allow
            .is_empty(),
        "nothing saved"
    );
    crate::cfo::save_checked(dir.path(), patch("claude-sonnet-5-5"), &catalogs).unwrap();
    assert_eq!(
        crate::cfo::settings::peek(dir.path()).unwrap().allow.len(),
        1
    );
}

#[test]
fn the_cfo_save_card_shows_old_to_new_and_refuses_no_change() {
    use crate::cfo::settings::Patch;
    let old = crate::cfo::CfoSettings::default();
    let lines = crate::cfo::save_card_lines(
        &old,
        &Patch {
            reserve_percent: Some(30),
            ..Patch::default()
        },
    )
    .unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("20% to 30%"), "{lines:?}");
    assert!(crate::cfo::save_card_lines(&old, &Patch::default()).is_err());
    assert!(crate::cfo::save_card_lines(
        &old,
        &Patch {
            reserve_percent: Some(20),
            ..Patch::default()
        }
    )
    .is_err());
    // Built the way the broker builds it, from the action's own arguments.
    let (_dir, app) = bed();
    let save = find("settings.cfo_save_settings").unwrap();
    let args = validate(save, &json!({ "effortMode": "fixed" })).unwrap();
    let card = crate::app_actions::card_for(save, &cx(&app), &args)
        .unwrap()
        .unwrap()
        .text();
    assert!(card.contains("Automatic"), "{card}");
    assert!(card.contains("Fixed"), "{card}");
    let approve = find("settings.cfo_approve_extra_usage").unwrap();
    let args = validate(approve, &json!({ "hours": 0 })).unwrap();
    let card = crate::app_actions::card_for(approve, &cx(&app), &args)
        .unwrap()
        .unwrap()
        .text();
    assert!(card.contains("Withdraw"), "{card}");
}
