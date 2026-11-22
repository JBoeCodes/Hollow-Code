use super::*;
use std::process::{Command, Stdio};

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_plain_launch_and_the_flags_the_app_and_macos_pass_start_the_app() {
    assert_eq!(classify(&[]), Launch::Gui);
    assert_eq!(classify(&args(&["--hidden"])), Launch::Gui);
    assert_eq!(classify(&args(&["-psn_0_12345"])), Launch::Gui);
    assert_eq!(
        classify(&args(&["-NSDocumentRevisionsDebugMode", "YES", "--hidden"])),
        Launch::Gui
    );
}

#[test]
fn version_and_help_answer_instead_of_starting_the_app() {
    for v in [
        &["--version"][..],
        &["-V"],
        &["version"],
        &["--hidden", "--version"],
    ] {
        assert_eq!(classify(&args(v)), Launch::Version, "{v:?}");
    }
    for h in [&["--help"][..], &["-h"], &["help"], &["--hidden", "--help"]] {
        assert_eq!(classify(&args(h)), Launch::Help, "{h:?}");
    }
    // Help wins when both are asked for.
    assert_eq!(classify(&args(&["--version", "--help"])), Launch::Help);
}

#[test]
fn an_unknown_command_or_option_is_refused_with_its_name_and_never_starts_the_app() {
    for bad in [
        &["bogus"][..],
        &["--bogus"],
        &["--hidden", "bogus"],
        &["/Users/x/folder"],
        &["--versoin"],
    ] {
        match classify(&args(bad)) {
            Launch::Reject(message) => {
                assert!(message.contains("unknown command or option"), "{message}");
                assert!(message.contains("hollow --help"), "{message}");
                assert!(message.contains(bad.iter().find(|a| !a.starts_with("--hidden")).unwrap()));
            }
            other => panic!("{bad:?} gave {other:?}"),
        }
        assert_eq!(gate(&args(bad)), Some(2), "{bad:?}");
    }
}

#[test]
fn a_listed_command_is_a_command_whatever_follows_it() {
    for (name, _) in COMMANDS {
        assert_eq!(classify(&args(&[name])), Launch::Command, "{name}");
        assert_eq!(
            classify(&args(&[name, "--version", "x"])),
            Launch::Command,
            "{name} passes its own arguments through"
        );
    }
}

/// `main.rs` dispatches exactly the commands `--help` lists: a command added
/// there without a row here would be refused by `gate` as unwired, and a row
/// without an arm would claim a command that starts the app.
#[test]
fn the_command_list_matches_the_arms_of_main() {
    let main = include_str!("main.rs");
    let mut arms: Vec<String> = Vec::new();
    let mut rest = main;
    while let Some(at) = rest.find("Some(\"") {
        rest = &rest[at + 6..];
        let end = rest.find('"').unwrap();
        if rest[end..].starts_with("\") =>") {
            arms.push(rest[..end].to_string());
        }
    }
    arms.sort();
    let mut listed: Vec<String> = COMMANDS.iter().map(|(n, _)| n.to_string()).collect();
    listed.sort();
    assert_eq!(arms, listed);
    assert!(
        crate::source_pins::contains(main, "hollow_lib::gate_gui_launch()"),
        "main.rs must gate the app start"
    );
}

#[test]
fn help_lists_the_usage_and_every_command_and_version_names_the_build() {
    let help = help_text();
    for needle in ["--version", "--help", "exit status 2", "app is not started"] {
        assert!(help.contains(needle), "{needle}");
    }
    for (name, purpose) in COMMANDS {
        assert!(help.contains(name) && help.contains(purpose), "{name}");
    }
    assert!(version_text().starts_with("Hollow "), "{}", version_text());
    assert!(!version_text().contains('\n'));
}

#[test]
fn a_second_launch_finds_the_lock_held_and_a_released_lock_is_free_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join(LOCK_FILE);
    let Acquire::First(first) = acquire(&path, Duration::ZERO) else {
        panic!("the first launch takes it");
    };
    assert!(matches!(acquire(&path, Duration::ZERO), Acquire::Running));
    drop(first);
    assert!(matches!(acquire(&path, Duration::ZERO), Acquire::First(_)));
}

#[test]
fn a_launch_during_a_relaunch_waits_for_the_quitting_app_to_let_go() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOCK_FILE);
    let Acquire::First(first) = acquire(&path, Duration::ZERO) else {
        panic!("free");
    };
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        drop(first);
    });
    let started = Instant::now();
    assert!(matches!(
        acquire(&path, Duration::from_secs(10)),
        Acquire::First(_)
    ));
    assert!(started.elapsed() >= Duration::from_millis(300));
    assert!(started.elapsed() < Duration::from_secs(5));
    releaser.join().unwrap();
}

#[test]
fn an_unwritable_lock_folder_never_stops_the_app_from_starting() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a-file");
    std::fs::write(&file, "x").unwrap();
    assert!(matches!(
        acquire(&file.join(LOCK_FILE), Duration::ZERO),
        Acquire::Unavailable(_)
    ));
}

/// Run this test binary again on one named child test with `mode`, stderr
/// piped; the caller decides what to do with the pipe.
fn child(test: &str, mode: &str, extra: &[(&str, &Path)]) -> std::process::Child {
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env("HOLLOW_LAUNCH_TEST_CHILD", mode)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    for (key, value) in extra {
        cmd.env(key, value);
    }
    cmd.spawn().unwrap()
}

/// The crash itself (13:55 and 12:33 PM): stderr is a pipe whose reader has
/// gone when a background thread logs. `eprintln!` panics there; `elog`
/// doesn't. The panicking child is the premise of the fix, kept as a test
/// so a toolchain that stops panicking says so.
#[test]
fn elog_survives_a_stderr_pipe_nobody_reads_where_eprintln_panics() {
    let mut safe = child("launch::tests::stderr_child", "elog", &[]);
    drop(safe.stderr.take());
    let mut unsafe_child = child("launch::tests::stderr_child", "eprintln", &[]);
    drop(unsafe_child.stderr.take());
    assert!(safe.wait().unwrap().success(), "elog must not panic");
    assert!(
        !unsafe_child.wait().unwrap().success(),
        "eprintln! on a dead stderr pipe panics (an abort in a release build)"
    );
}

#[test]
fn stderr_child() {
    let Ok(mode) = std::env::var("HOLLOW_LAUNCH_TEST_CHILD") else {
        return;
    };
    // Let the parent close its end first.
    std::thread::sleep(Duration::from_millis(600));
    let writer = std::thread::Builder::new()
        .name("shell-path".into())
        .spawn(move || {
            for _ in 0..4 {
                match mode.as_str() {
                    "elog" => elog("[shell-path] read 3 folders from /bin/zsh in 12 ms"),
                    "eprintln" => eprintln!("[shell-path] read 3 folders from /bin/zsh in 12 ms"),
                    _ => {}
                }
            }
        })
        .unwrap();
    writer.join().unwrap();
}

/// A second launch in another process, while this one holds the lock: the
/// real cross-process case, not two descriptors in one process.
#[test]
fn a_launch_in_another_process_sees_the_lock_this_process_holds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(LOCK_FILE);
    let Acquire::First(mine) = acquire(&path, Duration::ZERO) else {
        panic!("free");
    };
    let out = dir.path().join("answer");
    let mut other = child(
        "launch::tests::second_launch_child",
        "second",
        &[
            ("HOLLOW_LAUNCH_TEST_LOCK", &path),
            ("HOLLOW_LAUNCH_TEST_OUT", &out),
        ],
    );
    assert!(other.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "running");
    drop(mine);
    let mut later = child(
        "launch::tests::second_launch_child",
        "second",
        &[
            ("HOLLOW_LAUNCH_TEST_LOCK", &path),
            ("HOLLOW_LAUNCH_TEST_OUT", &out),
        ],
    );
    assert!(later.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "first");
}

#[test]
fn second_launch_child() {
    if std::env::var("HOLLOW_LAUNCH_TEST_CHILD").is_err() {
        return;
    }
    let path = std::path::PathBuf::from(std::env::var("HOLLOW_LAUNCH_TEST_LOCK").unwrap());
    let out = std::env::var("HOLLOW_LAUNCH_TEST_OUT").unwrap();
    let answer = match acquire(&path, Duration::ZERO) {
        Acquire::First(_) => "first",
        Acquire::Running => "running",
        Acquire::Unavailable(_) => "unavailable",
    };
    std::fs::write(out, answer).unwrap();
}

/// The `shell-path` thread logs through `elog`, never `eprintln!` (crash 1).
#[test]
fn the_shell_path_thread_never_uses_eprintln() {
    let source = include_str!("shell_path.rs");
    let code: String = source
        .split("#[cfg(test)]")
        .next()
        .unwrap()
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!code.contains("eprintln!"), "use crate::launch::elog");
    assert!(code.contains("crate::launch::elog"));
}
