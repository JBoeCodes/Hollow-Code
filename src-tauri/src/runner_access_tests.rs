//! Remote runners (2026-11-03): runner access per workspace, the GitHub
//! credential's source, and what a turn's header and leader carry.

use super::*;

/// One test at a time touches the shared store file and the secret store
/// (`remote_session`'s leader test takes it too).
pub(crate) static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn vault_with(ids: &[&str]) -> tempfile::TempDir {
    let vault = tempfile::tempdir().unwrap();
    for id in ids {
        std::fs::create_dir_all(vault.path().join("workspaces").join(id)).unwrap();
    }
    vault
}

#[test]
fn github_repos_are_read_from_every_github_url_form_and_nothing_else() {
    for url in [
        "git@github.com:acme/widgets.git",
        "https://github.com/acme/widgets",
        "https://github.com/acme/widgets.git",
        "https://github.com/acme/widgets/",
        "ssh://git@github.com/acme/widgets.git",
        "ssh://git@github.com:22/acme/widgets",
    ] {
        assert_eq!(github_repo(url).as_deref(), Some("acme/widgets"), "{url}");
    }
    for url in [
        "git@gitlab.com:acme/widgets.git",
        "https://github.example.com/acme/widgets",
        "https://github.com/acme",
        "https://github.com/acme/../x",
        "https://github.com/a b/c",
        "",
    ] {
        assert_eq!(github_repo(url), None, "{url}");
    }
}

#[test]
fn service_accounts_and_tokens_are_checked_without_echoing_the_token() {
    assert!(service_account_ok(
        "deploy@example-project.iam.gserviceaccount.com"
    ));
    assert!(!service_account_ok("alex@example.com"));
    assert!(!service_account_ok("@x.iam.gserviceaccount.com"));
    assert!(!service_account_ok("a b@x.iam.gserviceaccount.com"));
    assert!(github_token_ok("github_pat_11ABCDEF0123456789_abcdefghijklmnop").is_ok());
    assert!(github_token_ok("ghp_abcdefghijklmnopqrstuvwxyz0123456789").is_ok());
    let secret = "gho_secretlooking0123456789";
    let err = github_token_ok(secret).unwrap_err();
    assert!(!err.contains(secret), "{err}");
    assert!(github_token_ok("github_pat_has space").is_err());
    assert!(github_token_ok("").is_err());
}

#[test]
fn a_workspaces_access_is_off_until_set_and_kept_only_for_real_workspaces() {
    let _held = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let vault = vault_with(&["w1"]);
    assert_eq!(access_for(vault.path(), "w1"), RunnerAccess::default());
    assert!(!access_for(vault.path(), "w1").github);
    assert_eq!(access_for(vault.path(), "w1").cloud, CloudAccess::Off);
    let on = RunnerAccess {
        github: true,
        cloud: CloudAccess::Limited,
        service_account: Some("ignored@x.iam.gserviceaccount.com".into()),
    };
    let saved = set_access(vault.path(), "w1", on).unwrap();
    assert!(saved.github);
    assert_eq!(saved.cloud, CloudAccess::Limited);
    assert_eq!(saved.service_account, None, "kept only while in use");
    assert_eq!(access_for(vault.path(), "w1"), saved);
    // A service account needs a real address.
    let sa = |s: Option<&str>| RunnerAccess {
        github: false,
        cloud: CloudAccess::ServiceAccount,
        service_account: s.map(str::to_string),
    };
    assert!(set_access(vault.path(), "w1", sa(None)).is_err());
    assert!(set_access(vault.path(), "w1", sa(Some("me@example.com"))).is_err());
    let kept = set_access(
        vault.path(),
        "w1",
        sa(Some(" deploy@p-1.iam.gserviceaccount.com ")),
    )
    .unwrap();
    assert_eq!(
        kept.service_account.as_deref(),
        Some("deploy@p-1.iam.gserviceaccount.com")
    );
    // Not a workspace, the Main agent, a path: refused.
    assert!(set_access(vault.path(), "nope", RunnerAccess::default()).is_err());
    assert!(set_access(vault.path(), "../w1", RunnerAccess::default()).is_err());
    assert!(set_access(vault.path(), "main", RunnerAccess::default()).is_err());
    // Back to all off: the entry goes.
    set_access(vault.path(), "w1", RunnerAccess::default()).unwrap();
    let text = std::fs::read_to_string(store_path().unwrap()).unwrap();
    assert!(!text.contains("w1"), "{text}");
}

#[test]
fn the_header_carries_flags_only_never_a_secret_or_a_key_named_variable() {
    let access = TurnAccess {
        github: true,
        cloud: CloudAccess::Limited,
        service_account: None,
        gh_repo: Some("acme/widgets".into()),
    };
    let mut env = BTreeMap::new();
    header_env(&access, &mut env);
    assert_eq!(env["HOLLOW_RUNNER_GITHUB"], "on");
    assert_eq!(env["HOLLOW_RUNNER_CLOUD"], "limited");
    assert_eq!(env["GH_REPO"], "acme/widgets");
    assert_eq!(
        env["HOLLOW_RUNNER_ORIGIN"],
        "https://github.com/acme/widgets.git"
    );
    assert_eq!(env["GIT_TERMINAL_PROMPT"], "0");
    for (key, value) in &env {
        assert!(!key.starts_with("HOLLOW_CRED"), "{key}");
        // Codex drops variables named like a secret from its commands.
        for word in ["KEY", "SECRET", "TOKEN"] {
            assert!(!key.contains(word), "{key}");
        }
        assert!(
            !value.contains("github_pat_") && !value.contains("ya29."),
            "{key}={value}"
        );
    }
    // A folder turn (no checkout of its own): no origin, no GH_REPO.
    let mut folder = BTreeMap::new();
    header_env(
        &TurnAccess {
            gh_repo: None,
            ..access.clone()
        },
        &mut folder,
    );
    assert!(!folder.contains_key("HOLLOW_RUNNER_ORIGIN") && !folder.contains_key("GH_REPO"));
    // Off: nothing at all.
    let mut none = BTreeMap::new();
    header_env(&TurnAccess::default(), &mut none);
    assert!(none.is_empty(), "{none:?}");
    // The Cloud level goes back with each token from the leader.
    let creds = credentials(&TurnAccess {
        cloud: CloudAccess::Full,
        ..Default::default()
    });
    assert!(leader_env(&creds).contains(&("HOLLOW_CRED_GCLOUD_LEVEL", "full".to_string())));
}

#[test]
fn credentials_come_from_the_keychain_or_say_why_not() {
    let _held = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let token = "github_pat_TEST0123456789abcdefghij";
    let _ = crate::secret_store::delete(GITHUB_TOKEN_KEY);
    set_github_source(GithubSource::RunnerToken).unwrap();
    let access = TurnAccess {
        github: true,
        ..Default::default()
    };
    let none = credentials(&access);
    let why = none.github.unwrap().unwrap_err();
    assert!(why.contains("no runner token is saved"), "{why}");
    save_github_token(&format!("  {token}\n")).unwrap();
    assert!(github_status().token_saved);
    let creds = credentials(&access);
    assert_eq!(creds.github.as_ref().unwrap().as_deref(), Ok(token));
    assert!(creds.gcloud_mint.is_none(), "Google Cloud off");
    let env = leader_env(&creds);
    assert_eq!(env, vec![("HOLLOW_CRED_GITHUB", token.to_string())]);
    // Nothing the person reads carries it.
    assert!(!serde_json::to_string(&github_status())
        .unwrap()
        .contains(token));
    let noticed = notice(&access).unwrap().to_string();
    assert!(
        noticed.contains("runner-token") && !noticed.contains(token),
        "{noticed}"
    );
    assert!(!agent_note(&access).contains(token));
    // Off: no credential asked for, none held.
    let off = credentials(&TurnAccess::default());
    assert!(off.github.is_none() && off.gcloud_mint.is_none());
    assert!(leader_env(&off).is_empty());
    clear_github_token().unwrap();
    assert!(!github_status().token_saved);
}

#[test]
fn a_service_account_turn_mints_by_impersonation() {
    let access = TurnAccess {
        cloud: CloudAccess::ServiceAccount,
        service_account: Some("deploy@p-1.iam.gserviceaccount.com".into()),
        ..Default::default()
    };
    match credentials(&access).gcloud_mint.unwrap() {
        Ok(argv) => {
            assert!(argv[0].ends_with("/gcloud"), "{argv:?}");
            assert_eq!(&argv[1..3], ["auth", "print-access-token"]);
            assert!(argv.contains(
                &"--impersonate-service-account=deploy@p-1.iam.gserviceaccount.com".to_string()
            ));
        }
        // A machine without gcloud says so instead.
        Err(why) => assert!(why.contains("gcloud isn't installed"), "{why}"),
    }
    let bad = TurnAccess {
        service_account: Some("nobody@example.com".into()),
        ..access
    };
    if gcloud_program().is_some() {
        assert!(credentials(&bad).gcloud_mint.unwrap().is_err());
    }
}
