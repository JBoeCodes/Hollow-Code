//! L13 COO parity, group g1: the self-hosted model actions.

use crate::app_actions::{find, validate, Kind, Tier};
use serde_json::json;

const ALL: &[(&str, &str, Tier)] = &[
    (
        "settings.agents.self_hosted_models",
        "self_hosted_models",
        Tier::Read,
    ),
    (
        "settings.agents.save_self_hosted_model",
        "save_self_hosted_model",
        Tier::Confirm,
    ),
    (
        "settings.agents.remove_self_hosted_model",
        "remove_self_hosted_model",
        Tier::Confirm,
    ),
    (
        "settings.agents.set_self_hosted_model_enabled",
        "set_self_hosted_model_enabled",
        Tier::Write,
    ),
    (
        "settings.agents.test_self_hosted_model",
        "test_self_hosted_model",
        Tier::External,
    ),
    (
        "settings.agents.wake_self_hosted_model",
        "wake_self_hosted_model",
        Tier::External,
    ),
    (
        "settings.agents.self_hosted_wake_status",
        "self_hosted_wake_status",
        Tier::Read,
    ),
    (
        "settings.agents.self_hosted_jobs",
        "self_hosted_jobs",
        Tier::Read,
    ),
    (
        "settings.agents.set_self_hosted_job",
        "set_self_hosted_job",
        Tier::Confirm,
    ),
    (
        "settings.agents.run_self_hosted_jobs_now",
        "run_self_hosted_jobs_now",
        Tier::External,
    ),
    (
        "settings.agents.run_self_hosted_job_as_now",
        "run_self_hosted_job_as_now",
        Tier::Confirm,
    ),
    (
        "settings.agents.retry_self_hosted_job",
        "retry_self_hosted_job",
        Tier::External,
    ),
    (
        "settings.agents.discard_self_hosted_job",
        "discard_self_hosted_job",
        Tier::Confirm,
    ),
];

#[test]
fn every_self_hosted_action_resolves_with_its_tier_and_command() {
    for (id, command, tier) in ALL {
        let action = find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert_eq!(action.command, Some(*command), "{id}");
        assert!(action.tier == *tier, "{id}");
        assert_eq!(action.area, "settings.agents", "{id}");
    }
}

#[test]
fn the_token_command_is_a_secret_param_and_nothing_else_is() {
    let save = find("settings.agents.save_self_hosted_model").unwrap();
    let token = save
        .params
        .iter()
        .find(|p| p.name == "tokenCommand")
        .unwrap();
    assert!(matches!(token.kind, Kind::Secret) && !token.required);
    for (id, _, _) in ALL {
        let action = find(id).unwrap();
        for p in action.params.iter().filter(|p| p.name != "tokenCommand") {
            assert!(!matches!(p.kind, Kind::Secret), "{id}: {}", p.name);
        }
    }
}

#[test]
fn save_validates_required_params() {
    let save = find("settings.agents.save_self_hosted_model").unwrap();
    assert!(validate(save, &json!({ "id": "a" })).is_err());
    assert!(validate(
        save,
        &json!({ "id": "a", "name": "A", "baseUrl": "https://a.example", "servedModel": "m", "contextWindow": 131072, "tokenCommand": "tok --x" })
    )
    .is_ok());
}

#[test]
fn job_kind_choices_all_parse() {
    let set = find("settings.agents.set_self_hosted_job").unwrap();
    let kind = set.params.iter().find(|p| p.name == "kind").unwrap();
    let Kind::OneOf(values) = kind.kind else {
        panic!("kind is a choice");
    };
    for id in crate::self_hosted_jobs::JobKind::ALL.map(|k| k.id()) {
        assert!(values.contains(&id), "{id} missing");
    }
    assert_eq!(values.len(), crate::self_hosted_jobs::JobKind::ALL.len());
}

#[test]
fn results_carry_no_token_command() {
    use crate::commands::self_hosted_api::{mask_models, scrub};
    use crate::self_hosted_registry::{preset, SelfHostedAuth, SelfHostedEntry, SelfHostedModels};
    let entry = SelfHostedEntry {
        auth: SelfHostedAuth::TokenCommand {
            program: "/usr/local/bin/mint-token".into(),
            args: vec!["--audience".into(), "private-aud-9".into()],
        },
        ..preset()
    };
    let models = SelfHostedModels {
        entries: vec![entry.clone()],
        preset: preset(),
        levels: vec![],
        min_context_window: 1,
        max_context_window: 2,
        error: None,
        invalid: Default::default(),
    };
    let text = mask_models(models).unwrap().to_string();
    for leak in [
        "mint-token",
        "private-aud-9",
        "gcloud",
        "print-identity-token",
    ] {
        assert!(!text.contains(leak), "{leak}: {text}");
    }
    let scrubbed = scrub(
        json!({ "error": "/usr/local/bin/mint-token --audience private-aud-9 exited 1" }),
        &[
            "/usr/local/bin/mint-token --audience private-aud-9".into(),
            "/usr/local/bin/mint-token".into(),
        ],
    )
    .to_string();
    assert!(
        !scrubbed.contains("private-aud-9") && !scrubbed.contains("mint-token"),
        "{scrubbed}"
    );
}

// ── L13 review fixes (set A) ─────────────────────────────────────────────

/// A Test's and a wake's errors reach an agent in Hollow's words: the
/// model's answer can echo the bearer token, the token command's stderr can
/// quote its arguments.
#[test]
fn test_and_wake_errors_an_agent_reads_quote_neither_model_nor_command() {
    use crate::commands::self_hosted_api::{agent_test_result, agent_wake_result};
    use crate::self_hosted_registry::SelfHostedTest;
    use crate::self_hosted_wake::SelfHostedWake;
    let bearer = "eyJhbGciOiJSUzI1NiJ9.SECRET-BEARER";
    let echoed = SelfHostedTest {
        health_status: Some(200),
        status: Some(401),
        error: Some(format!("The model answered 401: bad token Bearer {bearer}")),
        ..SelfHostedTest::default()
    };
    let out = agent_test_result(echoed);
    let error = out.error.unwrap();
    assert!(!error.contains(bearer) && error.contains("401"), "{error}");
    let command = SelfHostedTest {
        error: Some("token command gcloud --impersonate SECRET-SA failed: SECRET-STDERR".into()),
        ..SelfHostedTest::default()
    };
    let error = agent_test_result(command).error.unwrap();
    assert!(!error.contains("SECRET"), "{error}");
    assert!(agent_test_result(SelfHostedTest::default()).error.is_none());

    let woke = agent_wake_result(SelfHostedWake {
        error: Some(format!("token command printed {bearer}")),
        ..SelfHostedWake::default()
    });
    let error = woke.error.unwrap();
    assert!(
        !error.contains(bearer) && error.contains("self-hosted-wake"),
        "{error}"
    );
}

/// Kept job errors (a drain's, a rerun's, from before a token command
/// changed) and unreadable files reach an agent as fixed sentences.
#[test]
fn kept_job_errors_an_agent_reads_are_fixed_sentences() {
    use crate::commands::self_hosted_api::agent_jobs_view;
    use crate::self_hosted_jobs::{
        JobKind, SelfHostedHeldJob, SelfHostedJobRow, SelfHostedJobs, SelfHostedWaiting,
    };
    let old = "old-mint --audience SECRET-OLD-AUD";
    let kind = JobKind::ALL[0];
    let view = SelfHostedJobs {
        kinds: vec![SelfHostedJobRow {
            kind,
            label: "k".into(),
            about: "a".into(),
            entry: Some("qwen".into()),
            problem: Some(format!(
                "Skipped while the self-hosted models list can't be read (invalid type: string \"{old}\")"
            )),
        }],
        waiting: vec![SelfHostedWaiting {
            entry: "qwen".into(),
            name: None,
            count: 1,
            oldest_at: "2026-11-10T00:00:00Z".into(),
            draining: false,
            last_error: Some(format!("{old} exited 1")),
        }],
        held: vec![SelfHostedHeldJob {
            id: "j1".into(),
            kind,
            label: "k".into(),
            entry: "qwen".into(),
            name: None,
            queued_at: "2026-11-10T00:00:00Z".into(),
            attempts: 3,
            last_error: Some(format!("{old}: permission denied")),
            gave_up: true,
            running: false,
            can_run_as_now: false,
            can_try_again: true,
            only_on: None,
        }],
        error: Some(format!("self-hosted-jobs.json isn't valid JSON ({old})")),
        queue_error: Some(format!("queue: {old}")),
    };
    let shown = agent_jobs_view(view);
    let text = serde_json::to_string(&shown).unwrap();
    assert!(
        !text.contains("SECRET-OLD-AUD") && !text.contains("old-mint"),
        "{text}"
    );
    assert!(
        shown.held[0].last_error.is_some(),
        "it still says one failed"
    );
    assert!(shown.waiting[0].last_error.is_some());
}

/// A registry file that can't be read reaches an agent without its serde
/// text (which quotes the file), on every path: the list, a save, a
/// removal and a switch.
#[test]
fn a_malformed_registry_is_reported_without_its_text() {
    use crate::commands::self_hosted_api::{agent_error, mask_models, AGENT_REGISTRY_ERROR};
    use crate::self_hosted_registry::{list_at, FILE};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE);
    let secret = "SECRET-IN-THE-FILE";
    std::fs::write(
        &path,
        format!(
            "{{\"version\": 1, \"entries\": [{{\"id\": \"a\", \"contextWindow\": \"{secret}\"}}]}}"
        ),
    )
    .unwrap();
    let listed = mask_models(list_at(&path)).unwrap().to_string();
    assert!(!listed.contains(secret), "{listed}");
    assert!(listed.contains("self-hosted-registry"), "{listed}");
    for why in [
        crate::self_hosted_registry::remove_at(&path, "a").unwrap_err(),
        crate::self_hosted_registry::set_enabled_at(&path, "a", true).unwrap_err(),
        crate::self_hosted_registry::read_at(&path).unwrap_err(),
    ] {
        assert_eq!(agent_error(&why, &[]), AGENT_REGISTRY_ERROR);
    }
}

/// The COO's save reads the saved entry, merges what it left out and writes,
/// all under the registry's one lock; a file that can't be read refuses the
/// save instead of replacing the saved token command with none.
#[test]
fn the_agents_save_merges_under_one_lock_and_refuses_an_unreadable_file() {
    use crate::commands::self_hosted_api::{agent_error, entry_from_args, AGENT_REGISTRY_ERROR};
    use crate::self_hosted_registry::{
        preset, save_at, save_with_at, SelfHostedAuth, SelfHostedEntry, FILE,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(FILE);
    let saved = SelfHostedEntry {
        auth: SelfHostedAuth::TokenCommand {
            program: "mint-tool".into(),
            args: vec!["--aud".into(), "private-aud-42".into()],
        },
        ..preset()
    };
    save_at(&path, saved.clone(), false).unwrap();
    let args = json!({
        "id": saved.id, "name": "Renamed", "baseUrl": saved.base_url,
        "servedModel": saved.served_model, "contextWindow": saved.context_window
    })
    .as_object()
    .cloned()
    .unwrap();
    let models = save_with_at(&path, &saved.id, true, |s| entry_from_args(&args, s)).unwrap();
    assert_eq!(models.entries[0].name, "Renamed");
    assert_eq!(
        models.entries[0].auth, saved.auth,
        "the saved token command stays"
    );

    let broken = b"{\"version\": 1, \"entries\": [";
    std::fs::write(&path, broken).unwrap();
    let mut built = false;
    let why = save_with_at(&path, &saved.id, true, |s| {
        built = true;
        entry_from_args(&args, s)
    })
    .unwrap_err();
    assert!(!built, "nothing is built on an unreadable file");
    assert_eq!(std::fs::read(&path).unwrap(), broken, "nothing is written");
    assert_eq!(agent_error(&why, &[]), AGENT_REGISTRY_ERROR);
}

/// A token command's text (the whole line, and its program) is hidden
/// wherever it turns up, as an error is kept.
#[test]
fn token_command_text_is_hidden_before_an_error_is_kept() {
    use crate::self_hosted_registry::{
        hide_token_commands, preset, token_command_texts, SelfHostedAuth, SelfHostedEntry,
        HIDDEN_TOKEN_COMMAND,
    };
    let entry = SelfHostedEntry {
        auth: SelfHostedAuth::TokenCommand {
            program: "/opt/bin/mint".into(),
            args: vec!["--audience".into(), "SECRET-AUD-77".into()],
        },
        ..preset()
    };
    let secrets = token_command_texts(&[entry]);
    let kept = hide_token_commands(
        "/opt/bin/mint --audience SECRET-AUD-77 exited 1 (/opt/bin/mint)",
        &secrets,
    );
    assert!(!kept.contains("SECRET-AUD-77"), "{kept}");
    assert!(kept.contains(HIDDEN_TOKEN_COMMAND), "{kept}");
}
