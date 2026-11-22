//! `hollow serve`: its command line, its host, and the one-Hollow guard.

use super::*;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn state_in(dir: &Path) -> AppState {
    let mut config = crate::config::Config::placeholder();
    config.vault_path = dir.to_path_buf();
    config.claude_projects_path = dir.join("no-claude-projects");
    AppState::new(config, dir.join("config.json"))
}

#[test]
fn the_command_line_takes_its_three_options_both_ways() {
    assert_eq!(parse(&[]).unwrap(), Options::default());
    let full = parse(&args(&[
        "--data-dir",
        "/tmp/hollow-data",
        "--config=/tmp/c.json",
        "--phone-port",
        "47690",
    ]))
    .unwrap();
    assert_eq!(full.data_dir, Some(PathBuf::from("/tmp/hollow-data")));
    assert_eq!(full.config, Some(PathBuf::from("/tmp/c.json")));
    assert_eq!(full.phone_port, Some(47690));
    assert!(parse(&args(&["--help"])).unwrap().help);
    assert!(parse(&args(&["-h"])).unwrap().help);
}

#[test]
fn a_bad_command_line_is_refused_with_a_reason() {
    for (line, says) in [
        (vec!["--data-dir", "relative/dir"], "absolute"),
        (vec!["--data-dir"], "needs a value"),
        (vec!["--config="], "needs a value"),
        (
            vec!["--data-dir", "--config", "/tmp/c.json"],
            "needs a value",
        ),
        (vec!["--config", "--phone-port"], "needs a value"),
        (vec!["--phone-port", "80"], "1024 to 65535"),
        (vec!["--phone-port", "lots"], "1024 to 65535"),
        (vec!["--hidden"], "unknown option `--hidden`"),
        (vec!["pair"], "unknown option `pair`"),
    ] {
        let error = parse(&args(&line)).unwrap_err();
        assert!(error.contains(says), "{line:?}: {error}");
    }
}

#[test]
fn help_names_every_option_and_exit_status() {
    let help = help_text();
    for needle in ["--data-dir", "--config", "--phone-port", " 75 ", " 78 "] {
        assert!(help.contains(needle), "{needle}");
    }
    assert_eq!(
        (EXIT_USAGE, EXIT_BUSY, EXIT_CONFIG, EXIT_RUNTIME),
        (2, 75, 78, 1)
    );
}

/// The serve host owns its state, has no window or GUI handle, and swallows
/// events and banners without failing.
#[test]
fn the_serve_host_holds_its_state_and_has_no_window() {
    let dir = tempfile::tempdir().unwrap();
    let host = ServeHost::new(state_in(dir.path()), false);
    let core = Core::new(host.clone());
    assert_eq!(
        core.try_state::<AppState>().unwrap().config.vault_path,
        dir.path()
    );
    assert_eq!(CoreHost::main_window(&core), None);
    assert!(core.gui().is_none());
    assert!(core.emit("chat-event", serde_json::json!({"k": 1})).is_ok());
    assert!(core.emit_to("main", "phone-modes-wanted", ()).is_ok());
    assert!(CoreHost::show_native(&core, "T", "B").is_ok());
    // core() hands back a core over the same state.
    let again = CoreHost::core(&host);
    assert!(std::ptr::eq(
        again.try_state::<AppState>().unwrap(),
        core.try_state::<AppState>().unwrap()
    ));
}

/// A notification made through serve's host is written like the app's
/// (the bell's JSONL), with no window to suppress the banner.
#[test]
fn a_notification_through_serve_is_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let core = Core::new(ServeHost::new(state_in(dir.path()), true));
    let made = crate::notifications::create(
        &core,
        dir.path(),
        crate::notifications::NotificationInput {
            title: "Serve".into(),
            body: "a banner becomes a log line".into(),
            kind: "info".into(),
            source: None,
            action_label: None,
            action_url: None,
            notify_native: true,
        },
    )
    .unwrap();
    assert_eq!(made.title, "Serve");
    let listed = crate::notifications::list(dir.path());
    assert!(listed.items.iter().any(|n| n.id == made.id));
}

/// Serve takes the app's launch lock and leaves its pid beside it; a second
/// taker is refused with 75; letting go removes the marker.
#[test]
fn serve_owns_the_data_folder_like_the_app_does() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Owner::take(dir.path()).expect("first");
    assert_eq!(running_in(dir.path()), Some(std::process::id()));
    match Owner::take(dir.path()) {
        Err((code, message)) => {
            assert_eq!(code, EXIT_BUSY);
            assert!(message.contains("another Hollow"), "{message}");
        }
        Ok(_) => panic!("a second serve took the lock"),
    }
    // The app's own launch sees the lock held.
    assert!(matches!(
        crate::launch::acquire(&dir.path().join(crate::launch::LOCK_FILE), Duration::ZERO),
        crate::launch::Acquire::Running
    ));
    drop(owner);
    assert_eq!(running_in(dir.path()), None);
    assert!(!dir.path().join(MARKER_FILE).exists());
    assert!(Owner::take(dir.path()).is_ok(), "free again");
}

/// A marker whose process is gone (a serve that was killed) names nobody,
/// so the app starts as before.
#[test]
fn a_dead_serves_marker_is_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    std::fs::write(
        dir.path().join(MARKER_FILE),
        serde_json::json!({ "pid": pid }).to_string(),
    )
    .unwrap();
    assert_eq!(running_in(dir.path()), None);
    std::fs::write(dir.path().join(MARKER_FILE), "not json").unwrap();
    assert_eq!(running_in(dir.path()), None);
}

/// A vault whose run queue another process owns is refused before serve
/// starts anything.
#[test]
fn a_vault_owned_elsewhere_is_refused() {
    let vault = tempfile::tempdir().unwrap();
    let Some(path) = crate::session_store::pump_lock_path(vault.path()) else {
        return;
    };
    assert!(queue_free(vault.path()).is_ok());
    let held = match crate::launch::acquire(&path, Duration::ZERO) {
        crate::launch::Acquire::First(lock) => lock,
        _ => panic!("lock"),
    };
    let error = queue_free(vault.path()).unwrap_err();
    assert!(error.contains("owns the run queue"), "{error}");
    drop(held);
    assert!(queue_free(vault.path()).is_ok());
    let _ = std::fs::remove_file(path);
}

/// A confirm-gated tool's approval request to a Hollow with no window is
/// refused at once with the reason, never a two-minute wait.
#[cfg(unix)]
#[test]
fn an_approval_request_to_serve_fails_closed_at_once() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let path = std::env::temp_dir().join(format!("hsv-apr-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let dir = tempfile::tempdir().unwrap();
        // The approvals socket's own handler, on serve's host.
        let core = Core::new(ServeHost::new(state_in(dir.path()), false));
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            crate::approvals::handle_conn(core, stream).await;
        });
        let started = std::time::Instant::now();
        let verdict = crate::approvals::request_via_app_at(
            path.to_str().unwrap(),
            Duration::from_secs(30),
            "Computer control",
            "open an app",
            "computer",
            None,
            None,
        )
        .await;
        server.await.unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            verdict,
            Some(Err(crate::approvals::HEADLESS_REFUSAL.to_string()))
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    });
}

/// The exit statuses a test can reach without starting a process: 2 for a
/// bad command line (from `main` itself), 75 for a held data folder or
/// queue (above), 78 for a config whose vault is not a folder. A missing
/// config.json (also 78) is `load_state` reading `HOLLOW_CONFIG`, which a
/// test can't set without changing it for every other test in the process.
#[test]
fn exit_statuses_without_a_process() {
    assert_eq!(main(args(&["--hidden"])), EXIT_USAGE);
    assert_eq!(main(args(&["--phone-port", "80"])), EXIT_USAGE);
    let dir = tempfile::tempdir().unwrap();
    let gone = state_in(&dir.path().join("no-such-vault"));
    let error = usable(gone).err().unwrap();
    assert!(error.contains("is not a folder"), "{error}");
    assert!(usable(state_in(dir.path())).is_ok());
    assert!(no_config("missing".into()).contains("no setup mode"));
    // A named config.json that isn't there is 78, never the usual lookup.
    let file = dir.path().join("config.json");
    std::fs::write(&file, "{}").unwrap();
    assert!(named_config(None).is_ok());
    assert!(named_config(Some(" ")).is_ok());
    assert!(named_config(file.to_str()).is_ok());
    let missing = dir.path().join("nope.json");
    let error = named_config(missing.to_str()).err().unwrap();
    assert!(error.contains("is not a file"), "{error}");
    assert!(
        named_config(dir.path().to_str()).is_err(),
        "a folder is not a config"
    );
}

/// Under `--data-dir` serve also holds the vault's queue lock where the
/// app keeps it: same file name as the app's own lock, in its Sessions.
#[test]
fn the_apps_queue_lock_is_named_like_its_own() {
    let vault = tempfile::tempdir().unwrap();
    let Some(own) = crate::session_store::pump_lock_path(vault.path()) else {
        return;
    };
    let support = tempfile::tempdir().unwrap();
    let app = crate::session_store::pump_lock_path_under(support.path(), vault.path());
    assert_eq!(app.file_name(), own.file_name());
    assert_eq!(app.parent().unwrap(), support.path().join("Sessions"));
}

/// `--data-dir`'s folder is made the user's alone.
#[test]
fn a_data_folder_of_serves_own_is_made_private() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    private_folder(dir.path());
    let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
}
