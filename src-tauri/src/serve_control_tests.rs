//! `hollow serve`'s commands and control socket.

use super::*;
use std::sync::Mutex;

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn every_command_parses_with_its_arguments() {
    let set = parse("secret-set", &args(&["claude:acct-1", "--data-dir=/tmp/d"])).unwrap();
    assert_eq!(set.what, What::SecretSet("claude:acct-1".into()));
    assert_eq!(set.data_dir, Some(PathBuf::from("/tmp/d")));
    assert_eq!(parse("secret-list", &[]).unwrap().what, What::SecretList);
    assert_eq!(
        parse("secret-delete", &args(&["tracker:linear"]))
            .unwrap()
            .what,
        What::SecretDelete("tracker:linear".into())
    );
    assert_eq!(
        parse("pair", &[]).unwrap().what,
        What::Phone(Request::Pair { machine: false })
    );
    assert_eq!(
        parse("pair", &args(&["--machine"])).unwrap().what,
        What::Phone(Request::Pair { machine: true })
    );
    assert_eq!(
        parse("enable-phone", &[]).unwrap().what,
        What::Phone(Request::EnablePhone)
    );
    assert_eq!(
        parse("disable-phone", &[]).unwrap().what,
        What::Phone(Request::DisablePhone)
    );
    assert_eq!(
        parse(
            "set-default-mode",
            &args(&["acceptEdits", "--agent", "Codex"])
        )
        .unwrap()
        .what,
        What::Phone(Request::SetDefaultMode {
            mode: "acceptEdits".into(),
            agent: Some("codex".into())
        })
    );
    // Every command `serve::main` sends here parses.
    for command in COMMANDS {
        let line: &[&str] = match *command {
            "secret-set" | "secret-delete" => &["k"],
            "set-default-mode" => &["plan"],
            "runner-coo" => &["status"],
            _ => &[],
        };
        assert!(parse(command, &args(line)).is_ok(), "{command}");
    }
}

/// A secret's value can't be put on the command line, where `ps` shows it.
#[test]
fn a_secret_value_is_never_taken_from_argv() {
    let error = parse("secret-set", &args(&["runner.github.token", "ghp_secret"])).unwrap_err();
    assert!(error.contains("stdin"), "{error}");
    assert!(!error.contains("ghp_secret"), "the value is not echoed");
    for (command, line, says) in [
        ("secret-set", vec![], "name the key"),
        ("secret-set", vec!["has space"], "no spaces"),
        ("set-default-mode", vec!["yolo"], "not a permission mode"),
        (
            "set-default-mode",
            vec!["plan", "--agent", "gpt"],
            "not an agent",
        ),
        ("pair", vec!["now"], "takes no arguments"),
        (
            "enable-phone",
            vec!["--machine"],
            "unknown option `--machine`",
        ),
        ("secret-list", vec!["--data-dir", "rel"], "absolute"),
        (
            "secret-list",
            vec!["--data-dir", "--agent"],
            "needs a value",
        ),
        (
            "set-default-mode",
            vec!["plan", "--agent", "--data-dir"],
            "needs a value",
        ),
    ] {
        let error = parse(command, &args(&line)).unwrap_err();
        assert!(error.contains(says), "{command} {line:?}: {error}");
    }
}

/// `secret-set`'s stdin path: the value read from a reader, one trailing
/// line break dropped, saved in the file backend; the output names the key
/// only.
#[test]
fn secret_set_reads_stdin_and_prints_only_the_key() {
    // Its own key: the test store is shared with `runner_access`'s tests.
    let value = read_value(&b"ghp_from_stdin_0123\n"[..]).unwrap();
    let mut out = Vec::new();
    save_secret("serve-test.token", value, &mut out).unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert_eq!(printed, "Saved serve-test.token.\n");
    assert_eq!(
        crate::secret_store::get("serve-test.token")
            .unwrap()
            .as_deref(),
        Some("ghp_from_stdin_0123")
    );
    let mut out = Vec::new();
    assert!(save_secret("jev.empty", "\n".into(), &mut out).is_err());
    assert!(
        read_value(&vec![b'x'; 70 * 1024][..]).is_err(),
        "64 KiB at most"
    );
    assert!(read_value(&[0xff, 0xfe][..]).is_err(), "text only");
    // A Claude token is shape-checked as Settings checks one.
    assert!(save_secret("claude:x", "sk-ant-api03-nope".into(), &mut Vec::new()).is_err());
}

/// The pairing code reaches stdout exactly once, and only stdout.
#[test]
fn the_pairing_code_is_printed_once() {
    let answer = Answer {
        code: Some("K7PM-4QXD".into()),
        expires_at: Some("2026-11-15T12:10:00Z".into()),
        ..Answer::done()
    };
    let mut out = Vec::new();
    print_answer(&Request::Pair { machine: false }, &answer, &mut out).unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert_eq!(printed.matches("K7PM-4QXD").count(), 1, "{printed}");
    assert!(printed.starts_with("Pairing code: K7PM-4QXD\n"));
    let refused = print_answer(
        &Request::Pair { machine: false },
        &Answer::failed("off"),
        &mut Vec::new(),
    );
    assert_eq!(refused, Err(Failure::Failed("off".into())));
    // M2: a serve that opened a phone window for `--machine` (one older than
    // the peer protocol) is an error, never a code that pairs a phone.
    let refused = print_answer(&Request::Pair { machine: true }, &answer, &mut Vec::new());
    assert!(matches!(refused, Err(Failure::Failed(_))));
}

/// M2: `pair --machine` prints the machine window's code once, and `--json`
/// one line the other Mac reads over SSH.
#[test]
fn a_machine_pairing_code_is_printed_once_and_as_one_json_line() {
    assert!(parse("pair", &args(&["--machine", "--json"])).unwrap().json);
    assert!(!parse("pair", &args(&["--machine"])).unwrap().json);
    assert!(parse("enable-phone", &args(&["--json"])).is_err());
    let answer = Answer {
        code: Some("M4CH-7NE2".into()),
        expires_at: Some("2026-11-15T12:10:00Z".into()),
        machine: true,
        machine_id: Some("m-0123456789abcdef".into()),
        slug: None,
        name: Some("Mac mini".into()),
        ..Answer::done()
    };
    let mut out = Vec::new();
    print_answer(&Request::Pair { machine: true }, &answer, &mut out).unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert_eq!(printed.matches("M4CH-7NE2").count(), 1, "{printed}");
    assert!(printed.starts_with("Machine pairing code for Mac mini: M4CH-7NE2\n"));
    let mut out = Vec::new();
    print_answer_as(&Request::Pair { machine: true }, &answer, true, &mut out).unwrap();
    let printed = String::from_utf8(out).unwrap();
    assert_eq!(printed.lines().count(), 1, "{printed}");
    let line: serde_json::Value = serde_json::from_str(printed.trim()).unwrap();
    assert_eq!(
        line,
        serde_json::json!({
            "code": "M4CH-7NE2",
            "expiresAt": "2026-11-15T12:10:00Z",
            "machine": true,
            "machineId": "m-0123456789abcdef",
            "slug": null,
            "name": "Mac mini",
        })
    );
    // A phone code's line has no machine fields.
    let phone = Answer {
        code: Some("K7PM-4QXD".into()),
        expires_at: Some("2026-11-15T12:10:00Z".into()),
        ..Answer::done()
    };
    let mut out = Vec::new();
    print_answer_as(&Request::Pair { machine: false }, &phone, true, &mut out).unwrap();
    let line: serde_json::Value =
        serde_json::from_str(String::from_utf8(out).unwrap().trim()).unwrap();
    assert_eq!(
        line,
        serde_json::json!({"code": "K7PM-4QXD", "expiresAt": "2026-11-15T12:10:00Z"})
    );
}

fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The socket is the owner's only (0600; a second user can't be tested
/// here, so the mode bits and the peer check's own answer are what's
/// asserted), takes one JSON line, answers one, and goes when it drops.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_control_socket_is_owner_only_and_answers_one_line() {
    let dir = tempfile::Builder::new()
        .prefix("sc-")
        .tempdir_in("/tmp")
        .unwrap();
    // A stale socket file from a crashed serve is replaced, and every
    // staging folder a killed serve left is removed.
    std::fs::write(socket_path(dir.path()), "stale").unwrap();
    std::fs::create_dir(dir.path().join(".ss99999")).unwrap();
    std::fs::write(dir.path().join(".ss99999").join("serve.sock"), "").unwrap();
    let (listener, control) = bind(dir.path()).unwrap();
    let path = socket_path(dir.path());
    assert_eq!(mode_of(&path), 0o600);
    assert!(!dir.path().join(".ss99999").exists());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handler: Handler = {
        let seen = seen.clone();
        Arc::new(move |request| {
            seen.lock().unwrap().push(request.clone());
            Box::pin(async move {
                match request {
                    Request::Pair { .. } => Answer {
                        code: Some("ABCD-2345".into()),
                        ..Answer::done()
                    },
                    _ => Answer::failed("no"),
                }
            })
        })
    };
    start(listener, handler).unwrap();
    let asked = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || {
            (
                ask(&path, &Request::Pair { machine: false }),
                ask(&path, &Request::EnablePhone),
            )
        })
        .await
        .unwrap()
    };
    assert_eq!(asked.0.unwrap().code.as_deref(), Some("ABCD-2345"));
    assert_eq!(asked.1.unwrap().error.as_deref(), Some("no"));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        [Request::Pair { machine: false }, Request::EnablePhone]
    );
    // The peer check passes for this process's own user.
    let client = tokio::net::UnixStream::connect(&path).await.unwrap();
    assert!(same_user(&client));
    // Garbage gets a refusal, not a hang.
    let raw = {
        let path = path.clone();
        tokio::task::spawn_blocking(move || {
            let mut stream = std::os::unix::net::UnixStream::connect(&path).unwrap();
            stream.write_all(b"not json\n").unwrap();
            let mut reply = String::new();
            std::io::BufReader::new(stream)
                .read_line(&mut reply)
                .unwrap();
            reply
        })
        .await
        .unwrap()
    };
    assert!(raw.contains("\"ok\":false"), "{raw}");
    drop(control);
    assert!(!path.exists(), "removed when serve stops");
}

#[test]
fn a_socket_path_too_long_for_unix_is_refused_up_front() {
    let long = PathBuf::from(format!("/tmp/{}", "d".repeat(120)));
    let error = bind(&long).err().unwrap();
    assert!(error.contains("too long"), "{error}");
    assert!(
        error.contains(&format!("{}/serve.sock", long.display())),
        "the real path: {error}"
    );
}

/// The wire shape both ends rely on.
#[test]
fn requests_and_answers_keep_their_wire_names() {
    assert_eq!(
        serde_json::to_string(&Request::SetDefaultMode {
            mode: "plan".into(),
            agent: None
        })
        .unwrap(),
        r#"{"cmd":"set-default-mode","mode":"plan","agent":null}"#
    );
    assert_eq!(
        serde_json::from_str::<Request>(r#"{"cmd":"pair"}"#).unwrap(),
        Request::Pair { machine: false }
    );
    assert_eq!(
        serde_json::to_string(&Answer::done()).unwrap(),
        r#"{"ok":true}"#
    );
}

/// With serve stopped, `set-default-mode` and `enable-phone` refuse while
/// the app holds the data folder, and `pair` needs a running serve.
#[test]
fn phone_commands_without_a_running_serve() {
    let dir = tempfile::tempdir().unwrap();
    let pair = phone(
        dir.path(),
        &Request::Pair { machine: false },
        false,
        &mut Vec::new(),
    );
    assert!(matches!(pair, Err(Failure::Failed(ref m)) if m.contains("not running")));
    let held =
        match crate::launch::acquire(&dir.path().join(crate::launch::LOCK_FILE), Duration::ZERO) {
            crate::launch::Acquire::First(lock) => lock,
            _ => panic!("lock"),
        };
    let busy = phone(dir.path(), &Request::EnablePhone, false, &mut Vec::new());
    assert!(
        matches!(busy, Err(Failure::Busy(ref m)) if m.contains("another Hollow process holds"))
    );
    drop(held);
}

/// `secret-set` into serve's real backend: a 0600 file a second store reads.
#[test]
fn secret_set_saves_through_the_secrets_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(crate::secret_store::FILE_NAME);
    let mut file = crate::secret_store::tests::TestFile::at(path.clone());
    let value = read_value(&b"tracker-key-0123\r\n"[..]).unwrap();
    save_secret_with("tracker:linear", value, &mut Vec::new(), |k, v| {
        file.set(k, v)
    })
    .unwrap();
    let mut reader = crate::secret_store::tests::TestFile::at(path.clone());
    assert_eq!(
        reader.get("tracker:linear").unwrap().as_deref(),
        Some("tracker-key-0123")
    );
    assert_eq!(mode_of(&path), 0o600);
}

/// A connection that never finishes its line is dropped after the read
/// wait, and a request over the limit is refused, never read whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_or_oversized_request_gets_nowhere() {
    let dir = tempfile::Builder::new()
        .prefix("sc-")
        .tempdir_in("/tmp")
        .unwrap();
    let (listener, _control) = bind(dir.path()).unwrap();
    let handler: Handler = Arc::new(|_| Box::pin(async { panic!("no request gets through") }));
    start(listener, handler).unwrap();
    let path = socket_path(dir.path());
    let (silent, oversized) = tokio::task::spawn_blocking(move || {
        let read_all = |mut stream: std::os::unix::net::UnixStream| {
            stream
                .set_read_timeout(Some(Duration::from_secs(20)))
                .unwrap();
            let mut reply = String::new();
            let _ = stream.read_to_string(&mut reply);
            reply
        };
        let mut slow = std::os::unix::net::UnixStream::connect(&path).unwrap();
        slow.write_all(b"{\"cmd\":\"pair\"").unwrap();
        let silent = read_all(slow);
        let mut big = std::os::unix::net::UnixStream::connect(&path).unwrap();
        let mut line = vec![b' '; 5000];
        line.extend_from_slice(b"{\"cmd\":\"pair\"}\n");
        big.write_all(&line).unwrap();
        (silent, read_all(big))
    })
    .await
    .unwrap();
    assert_eq!(silent, "", "closed without an answer");
    assert!(oversized.contains("\"ok\":false"), "{oversized}");
}

/// The data folder's lock decides, not a marker: with the lock free a
/// live pid in `serve.json` (this test's own) is a stale marker.
#[test]
fn a_stale_marker_with_a_free_lock_is_not_a_running_serve() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(crate::serve::MARKER_FILE),
        format!("{{\"pid\":{}}}", std::process::id()),
    )
    .unwrap();
    let pair = phone(
        dir.path(),
        &Request::Pair { machine: false },
        false,
        &mut Vec::new(),
    );
    assert!(
        matches!(pair, Err(Failure::Failed(ref m)) if m.contains("not running")),
        "{pair:?}"
    );
}

/// What `set-default-mode` prints: the stored mode and each agent's.
#[test]
fn set_default_mode_prints_what_each_agent_gets() {
    let without = default_mode_line("plan", None);
    assert!(
        without.contains("no --agent") && without.contains("claude plan"),
        "{without}"
    );
    let with = default_mode_line("acceptEdits", Some("claude"));
    assert!(
        with.contains("as set for claude") && with.contains("claude acceptEdits"),
        "{with}"
    );
}
