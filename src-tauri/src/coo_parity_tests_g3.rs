//! L13 COO parity, group g3: the remote runner and build actions
//! (`commands/runner_api.rs`, `commands/build_api.rs`).

use crate::app_actions::{card_for, find, validate, Cx, Kind, Tier};
use serde_json::json;
use std::path::Path;

const ALL: &[(&str, &str, Tier)] = &[
    (
        "build.runner_github_status",
        "runner_github_status",
        Tier::Read,
    ),
    ("build.runner_access_get", "runner_access_get", Tier::Read),
    ("build.runner_setup_plan", "runner_setup_plan", Tier::Read),
    (
        "build.runner_cloud_estimate",
        "runner_cloud_estimate",
        Tier::Read,
    ),
    (
        "build.runner_github_save_token",
        "runner_github_save_token",
        Tier::Confirm,
    ),
    (
        "build.runner_github_clear_token",
        "runner_github_clear_token",
        Tier::Confirm,
    ),
    (
        "build.runner_github_set_source",
        "runner_github_set_source",
        Tier::Confirm,
    ),
    ("build.runner_set", "runner_set", Tier::Confirm),
    (
        "build.runner_check_host",
        "runner_check_host",
        Tier::External,
    ),
    (
        "build.runner_record_owner",
        "runner_record_owner",
        Tier::External,
    ),
    (
        "build.runner_setup_check",
        "runner_setup_check",
        Tier::External,
    ),
    (
        "build.runner_setup_apply",
        "runner_setup_apply",
        Tier::External,
    ),
    (
        "build.runner_cloud_create",
        "runner_cloud_create",
        Tier::External,
    ),
    (
        "build.set_runner_access",
        "runner_access_set",
        Tier::Confirm,
    ),
    ("build.set_runs_on", "build_set_runs_on", Tier::Confirm),
    (
        "build.update_from_base",
        "build_update_from_base",
        Tier::External,
    ),
];

fn mock_app(vault: &Path) -> tauri::App<tauri::test::MockRuntime> {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = vault.to_path_buf();
    tauri::test::mock_builder()
        .manage(crate::commands::AppState::new(
            config,
            vault.join("config.json"),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap()
}

fn cx(app: &tauri::App<tauri::test::MockRuntime>) -> Cx {
    Cx::for_test(Box::new(app.handle().clone()))
}

#[test]
fn every_g3_action_resolves_with_its_tier_command_and_area() {
    for (id, command, tier) in ALL {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(action.command, Some(*command), "{id}");
        assert!(action.tier == *tier, "{id}");
        assert_eq!(action.area, "build", "{id}");
        // Confirm and external ones carry their own card builder.
        if matches!(tier, Tier::Confirm | Tier::External) {
            assert!(action.card.is_some(), "{id} needs its own card");
        }
    }
}

#[test]
fn the_github_token_is_the_only_secret_param() {
    let save = find("build.runner_github_save_token").unwrap();
    let token = save.params.iter().find(|p| p.name == "token").unwrap();
    assert!(matches!(token.kind, Kind::Secret) && token.required);
    for (id, _, _) in ALL {
        for p in find(id).unwrap().params {
            if !(*id == "build.runner_github_save_token" && p.name == "token") {
                assert!(!matches!(p.kind, Kind::Secret), "{id}: {}", p.name);
            }
        }
    }
}

#[test]
fn saving_the_token_never_returns_or_shows_it() {
    let _held = crate::runner_access::tests::SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let token = "github_pat_COO0123456789parityTESTabcdef";
    let _ = crate::secret_store::delete(crate::runner_access::GITHUB_TOKEN_KEY);
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let save = find("build.runner_github_save_token").unwrap();
    let args = validate(save, &json!({ "token": token })).unwrap();
    // The card shows it as dots only.
    let card = card_for(save, &cx(&app), &args).unwrap().unwrap();
    assert!(!card.text().contains(token), "{}", card.text());
    assert!(card.text().contains("••••"));
    // The action's result: the status, no token.
    let result = tauri::async_runtime::block_on((save.run)(cx(&app), args)).unwrap();
    let text = result.to_string();
    assert!(
        !text.contains(token) && !text.contains("github_pat_"),
        "{text}"
    );
    assert_eq!(result["tokenSaved"], true, "{text}");
    // The inner function the command shares: the same.
    let status = tauri::async_runtime::block_on(crate::commands::runner_api::github_save_token(
        format!(" {token}\n"),
    ))
    .unwrap();
    assert!(!serde_json::to_string(&status).unwrap().contains(token));
    // A malformed one is refused before any card, without echoing it.
    let bad = "gho_notarunnerTOKENbutsecret123";
    let args = validate(save, &json!({ "token": bad })).unwrap();
    let refused = card_for(save, &cx(&app), &args).unwrap_err();
    assert!(!refused.contains(bad), "{refused}");
    // Clearing returns the status too.
    let clear = find("build.runner_github_clear_token").unwrap();
    let after =
        tauri::async_runtime::block_on((clear.run)(cx(&app), validate(clear, &json!({})).unwrap()))
            .unwrap();
    assert_eq!(after["tokenSaved"], false, "{after}");
    assert!(!after.to_string().contains(token));
}

#[test]
fn the_source_card_shows_old_to_new_and_warns_about_gh_login() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let set = find("build.runner_github_set_source").unwrap();
    let args = validate(set, &json!({ "source": "gh-login" })).unwrap();
    let text = card_for(set, &cx(&app), &args).unwrap().unwrap().text();
    assert!(text.contains("→ this Mac's gh login"), "{text}");
    assert!(text.contains("broader"), "{text}");
    assert!(validate(set, &json!({ "source": "token" })).is_err());
}

#[test]
fn the_runner_host_choice_reads_off_and_blank_as_none() {
    use crate::commands::runner_api::host_choice;
    assert_eq!(host_choice("off"), None);
    assert_eq!(host_choice(" OFF "), None);
    assert_eq!(host_choice("  "), None);
    assert_eq!(host_choice(" build-box "), Some("build-box".to_string()));
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let set = find("build.runner_set").unwrap();
    for bad in ["-oProxyCommand=x", "local", "a b"] {
        let args = validate(set, &json!({ "host": bad })).unwrap();
        assert!(card_for(set, &cx(&app), &args).is_err(), "{bad}");
    }
    let args = validate(set, &json!({ "host": "off" })).unwrap();
    let text = card_for(set, &cx(&app), &args).unwrap().unwrap().text();
    assert!(text.contains("→ Off."), "{text}");
}

#[test]
fn ssh_cards_name_their_host_and_refuse_a_bad_one() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let check = find("build.runner_check_host").unwrap();
    let args = validate(check, &json!({ "host": "build-box" })).unwrap();
    let card = card_for(check, &cx(&app), &args).unwrap().unwrap();
    assert_eq!(card.kind(), "app-external");
    assert!(
        card.text().contains("Destination: ssh build-box"),
        "{}",
        card.text()
    );
    let args = validate(check, &json!({ "host": "-oProxyCommand=evil" })).unwrap();
    assert!(card_for(check, &cx(&app), &args).is_err());
}

/// A valid Google Cloud path (no ssh -G: that runs only for a Mac).
fn cloud_params() -> serde_json::Value {
    json!({
        "path": "cloud",
        "project": "hollow-parity-1",
        "billingAccount": "01ABCD-23EF45-6789AB",
        "machineType": "e2-standard-8",
        "hoursPerMonth": 100,
        "diskGb": 100,
        "budgetUsd": 50
    })
}

#[test]
fn the_cloud_create_card_names_google_cloud_the_cost_and_refuses_a_stale_estimate() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let params: crate::runner_setup::SetupParams = serde_json::from_value(cloud_params()).unwrap();
    let ctx = crate::runner_setup::Ctx::real().unwrap();
    let est = crate::runner_setup::estimate(&crate::runner_setup::filled(&ctx, &params)).unwrap();
    let create = find("build.runner_cloud_create").unwrap();
    let stale = validate(
        create,
        &json!({ "params": cloud_params(), "estimateCents": est.total_cents + 1 }),
    )
    .unwrap();
    let refused = card_for(create, &cx(&app), &stale).unwrap_err();
    assert!(refused.contains("isn't this machine's"), "{refused}");
    let args = validate(
        create,
        &json!({ "params": cloud_params(), "estimateCents": est.total_cents }),
    )
    .unwrap();
    let card = card_for(create, &cx(&app), &args).unwrap().unwrap();
    let text = card.text();
    assert_eq!(card.kind(), "app-external");
    assert!(text.contains("This costs money"), "{text}");
    assert!(
        text.contains("Destination: Google Cloud project hollow-parity-1 (billed to billing account 01ABCD-23EF45-6789AB)"),
        "{text}"
    );
    assert_eq!(card.bind, Some(json!({ "estimateCents": est.total_cents })));
    // The Mac path can't create one.
    let mac = validate(
        create,
        &json!({ "params": { "path": "mac" }, "estimateCents": 1 }),
    )
    .unwrap();
    assert!(card_for(create, &cx(&app), &mac).is_err());
    // The estimate read answers the same cents.
    let read = find("build.runner_cloud_estimate").unwrap();
    let got = tauri::async_runtime::block_on((read.run)(
        cx(&app),
        validate(read, &json!({ "params": cloud_params() })).unwrap(),
    ))
    .unwrap();
    assert_eq!(got["totalCents"], est.total_cents);
}

#[test]
fn applying_the_cloud_step_or_an_unknown_one_is_refused_before_a_card() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let apply = find("build.runner_setup_apply").unwrap();
    for step in ["cloud", "no-such-step"] {
        let args = validate(apply, &json!({ "params": cloud_params(), "step": step })).unwrap();
        assert!(card_for(apply, &cx(&app), &args).is_err(), "{step}");
    }
}

#[test]
fn runner_access_card_shows_old_to_new_and_the_action_sets_it() {
    let _held = crate::runner_access::tests::SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("workspaces").join("w1")).unwrap();
    let app = mock_app(dir.path());
    let set = find("build.set_runner_access").unwrap();
    for id in ["main", "nope", "../w1"] {
        let args = validate(set, &json!({ "id": id, "github": true, "cloud": "off" })).unwrap();
        assert!(card_for(set, &cx(&app), &args).is_err(), "{id}");
    }
    let args = validate(
        set,
        &json!({ "id": "w1", "github": true, "cloud": "limited" }),
    )
    .unwrap();
    let text = card_for(set, &cx(&app), &args).unwrap().unwrap().text();
    assert!(text.contains("merges): off → on."), "{text}");
    assert!(
        text.contains("Google Cloud: off → on as the person's account"),
        "{text}"
    );
    let now = tauri::async_runtime::block_on((set.run)(cx(&app), args)).unwrap();
    assert_eq!(now["github"], true);
    assert_eq!(now["cloud"], "limited");
    let read = find("build.runner_access_get").unwrap();
    let got = tauri::async_runtime::block_on((read.run)(
        cx(&app),
        validate(read, &json!({ "id": "w1" })).unwrap(),
    ))
    .unwrap();
    assert_eq!(got, now);
    crate::runner_access::set_access(
        dir.path(),
        "w1",
        crate::runner_access::RunnerAccess::default(),
    )
    .unwrap();
}

#[test]
fn build_cards_refuse_what_isnt_a_build_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let app = mock_app(dir.path());
    let runs_on = find("build.set_runs_on").unwrap();
    let args = validate(runs_on, &json!({ "id": "nope", "runsOn": "mac-mini" })).unwrap();
    assert!(card_for(runs_on, &cx(&app), &args)
        .unwrap_err()
        .contains("not a build workspace"));
    assert!(validate(runs_on, &json!({ "id": "w", "runsOn": "elsewhere" })).is_err());
    // M6: the runner's neutral name, and its old one, both pass.
    assert!(validate(runs_on, &json!({ "id": "w", "runsOn": "runner" })).is_ok());
    let update = find("build.update_from_base").unwrap();
    let args = validate(update, &json!({ "id": "nope" })).unwrap();
    assert!(card_for(update, &cx(&app), &args).is_err());
}

/// What an agent reads back from Set up's check, apply and create: the
/// step, its state in fixed words and each command as the plan shows it,
/// never a command's output (`ssh -G` prints a ProxyCommand, which can
/// hold a credential) or a sentence built from it.
#[test]
fn setup_results_an_agent_reads_carry_no_command_output() {
    use crate::commands::runner_api::for_agent;
    use crate::runner_setup::{RanCmd, StepState};
    let secret = "SECRET-PROXY-PW";
    let state = StepState {
        id: "alias".into(),
        state: "unknown",
        summary: format!("Couldn't tell: ssh -G mini failed (proxycommand sshpass -p {secret} ssh -W %h:%p jump)."),
        details: vec![format!("It printed {secret}.")],
        needs_by_hand: false,
        ran: vec![RanCmd {
            shown: "ssh -G mini".into(),
            code: Some(255),
            output: format!("user runner\nproxycommand sshpass -p {secret} ssh -W %h:%p jump\n"),
        }],
    };
    let view = for_agent(state);
    let text = serde_json::to_string(&view).unwrap();
    assert!(!text.contains(secret), "{text}");
    assert!(!text.contains("proxycommand"), "{text}");
    assert_eq!(view.state, "unknown");
    assert_eq!(view.ran.len(), 1);
    assert_eq!(view.ran[0].shown, "ssh -G mini");
    assert_eq!(view.ran[0].code, Some(255));
}

/// Update from base fetches from GitHub: external, so its card must name
/// where (and refuses what isn't a build workspace before anything runs).
#[test]
fn update_from_base_is_external_and_never_runs_unasked_in_auto() {
    let action = find("build.update_from_base").unwrap();
    assert!(action.tier == Tier::External);
    assert!(action.card.is_some());
    let none = crate::app_actions::Args(serde_json::Map::new());
    assert!(crate::app_actions::asks_in_mode(action, &none, "auto"));
}
