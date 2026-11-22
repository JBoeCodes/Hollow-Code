//! The install card shows what a build changes and its approval binds the
//! build's receipt; it shows in Bypass too (P0-27), and an allowed install
//! takes only the receipt its card recorded (merged plan slice 6, review
//! A·S4/B·F12). The receipt itself, the
//! re-hash and the deferred install's check are tested in `self_install.rs`.

use crate::app_actions::{self, Caller};
use crate::app_broker::{execute, Asker};
use crate::app_mcp::AppAccess;
use crate::approvals::Verdict;
use crate::self_install::{self, BuildReceipt};
use serde_json::json;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=t", "-c", "user.email=t@t"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn receipt(worktree: &Path, base: &str, commit: &str) -> BuildReceipt {
    BuildReceipt {
        build_id: "20261003-101500-0badf00d".into(),
        workspace_id: "ws-card".into(),
        worktree: worktree.to_string_lossy().into(),
        bundle: "/nonexistent/Hollow.app".into(),
        bundle_digest: "d".repeat(64),
        source_fingerprint: "e".repeat(64),
        commit: commit.into(),
        dirty: false,
        base_commit: base.into(),
        recorded_at: "2026-10-03T14:15:00Z".into(),
    }
}

#[test]
fn the_install_card_shows_the_commit_the_diffstat_and_the_changed_files() {
    let tmp = tempfile::Builder::new()
        .prefix("install-card-")
        .tempdir()
        .unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "--template="]);
    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "installed"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("a.txt"), "one\ntwo\n").unwrap();
    for i in 0..14 {
        std::fs::write(repo.join(format!("f{i:02}.txt")), "new\n").unwrap();
    }
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "built"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);

    let diff = self_install::build_diff(&repo, &base, &head);
    assert_eq!(diff.error, None);
    assert!(diff.stat.starts_with("15 files changed"), "{}", diff.stat);
    assert_eq!(diff.files.len(), 15);
    assert_eq!(diff.files[0], "M a.txt");

    let r = receipt(&repo, &base, &head);
    let card = app_actions::install_card_for("ws-card", Some("feat/x"), &r, &diff).unwrap();
    let text = card.text();
    assert!(
        text.contains(&format!("Commit {} on feat/x.", &head[..12])),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "Changes against the installed {}: 15 files changed, 15 insertions(+)",
            &base[..12]
        )),
        "{text}"
    );
    assert!(
        text.contains("  M a.txt") && text.contains("  A f10.txt"),
        "{text}"
    );
    assert!(!text.contains("f11.txt"), "the list is capped: {text}");
    assert!(text.contains("  and 3 more"), "{text}");
    assert!(
        text.contains("Receipt 20261003-101500-0badf00d: bundle fingerprint dddddddddddddddd, source fingerprint eeeeeeeeeeeeeeee"),
        "{text}"
    );
    assert!(text.contains("Destination: replaces the running Hollow"));
    assert_eq!(card.bind, Some(serde_json::to_value(&r).unwrap()));

    // No installed commit, or one this repository doesn't have: the card
    // says so instead of showing an empty diff.
    let unknown = self_install::build_diff(&repo, "", &head);
    assert!(unknown.error.is_some() && unknown.files.is_empty());
    let card = app_actions::install_card_for("ws-card", None, &r, &unknown).unwrap();
    assert!(
        card.text()
            .contains("Changes against the installed app: unavailable, the installed app's commit is unknown."),
        "{}",
        card.text()
    );
    let foreign = self_install::build_diff(&repo, &"0".repeat(40), &head);
    assert!(foreign.error.is_some(), "{foreign:?}");
    // A dirty build says its uncommitted changes are not in the diff.
    let dirty = BuildReceipt {
        dirty: true,
        ..r.clone()
    };
    let card = app_actions::install_card_for("ws-card", None, &dirty, &diff).unwrap();
    assert!(
        card.text().contains("built with uncommitted changes"),
        "{}",
        card.text()
    );
}

/// Records every card it is shown and denies it.
struct Recorder(RefCell<Vec<String>>);

impl Asker for Recorder {
    fn ask(&self, card: &app_actions::Card, _caller: &Caller) -> Verdict {
        self.0.borrow_mut().push(card.text());
        Verdict::Denied
    }
}

/// Records each card and allows it, as the person pressing Allow.
struct Allower(RefCell<Vec<String>>);

impl Asker for Allower {
    fn ask(&self, card: &app_actions::Card, _caller: &Caller) -> Verdict {
        self.0.borrow_mut().push(card.text());
        Verdict::Allowed
    }
}

fn write_exec(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fake_bundle(root: &Path) -> PathBuf {
    let bundle = root.join("target/release/bundle/macos/Hollow.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
    write_exec(
        &bundle.join("Contents/MacOS/hollow"),
        "#!/bin/sh\necho built\n",
    );
    std::fs::write(
        bundle.join("Contents/Resources/build-provenance.json"),
        r#"{"commit":"abc123def4567890abc123def4567890abc12345","dirty":false,"sourceFingerprint":"0123456789abcdef"}"#,
    )
    .unwrap();
    bundle
}

/// Installing Hollow asks in every mode, Bypass included (P0-27, D12): a
/// Bypass chat's call shows the card, and a declined card installs nothing.
/// Once allowed, the install still takes only the receipt the card
/// recorded: a bundle swapped after the build finished is refused, the same
/// bytes pass the check. Outside Bypass the same call shows the same card.
#[test]
fn bypass_shows_the_install_card_and_still_refuses_a_mismatched_bundle() {
    let _guard = self_install::TEST_JOB_GUARD
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = tempfile::Builder::new()
        .prefix("install-bypass-")
        .tempdir()
        .unwrap();
    let vault = tmp.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let bundle = fake_bundle(tmp.path());
    let recorded = self_install::record_receipt(
        "ws-bypass",
        Path::new("/nonexistent/worktree"),
        &bundle,
        "0011223344",
    )
    .unwrap();
    self_install::set_test_job(Some(recorded), "ws-bypass");

    let mut config = crate::config::Config::placeholder();
    config.vault_path = vault.clone();
    config.claude_projects_path = vault.join("no-claude-projects");
    let app = tauri::test::mock_builder()
        .manage(crate::commands::AppState::new(
            config,
            vault.join("config.json"),
        ))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .unwrap();
    let handle = crate::automated_history::try_begin_run(&vault, "chat", None).unwrap();
    let mut caller = Caller {
        run_id: handle.id.clone(),
        surface: "chat".into(),
        scope: "s-20261003-bypass".into(),
        access: AppAccess::Write,
        depth: 0,
        perm: "bypassPermissions".into(),
    };
    let asker = Recorder(RefCell::new(Vec::new()));
    let host = || -> Box<dyn app_actions::Host> { Box::new(app.handle().clone()) };
    let action = app_actions::find("build.install_built_app").unwrap();
    let args = || app_actions::validate(action, &json!({ "id": "ws-bypass" })).unwrap();

    // Bypass shows the card; declining it installs nothing.
    let why = execute(&host, &vault, action, args(), caller.clone(), &asker).unwrap_err();
    assert!(why.contains("declined"), "{why}");
    assert!(!why.contains("needs the running app"), "{why}");
    assert_eq!(asker.0.borrow().len(), 1, "bypass shows the install card");
    asker.0.borrow_mut().clear();

    // Allowed: the binary swapped after the build recorded its receipt.
    let allower = Allower(RefCell::new(Vec::new()));
    let exe = bundle.join("Contents/MacOS/hollow");
    write_exec(&exe, "#!/bin/sh\necho swapped\n");
    let why = execute(&host, &vault, action, args(), caller.clone(), &allower).unwrap_err();
    assert!(why.contains("changed after it was approved"), "{why}");
    assert_eq!(allower.0.borrow().len(), 1, "bypass shows the install card");

    // The recorded bytes pass the check; only the missing app stops it here.
    write_exec(&exe, "#!/bin/sh\necho built\n");
    let why = execute(&host, &vault, action, args(), caller.clone(), &allower).unwrap_err();
    assert!(why.contains("needs the running app"), "{why}");
    assert_eq!(allower.0.borrow().len(), 2, "bypass shows the install card");

    // Outside Bypass the same call asks, and the card names the commit.
    caller.perm = "acceptEdits".into();
    let why = execute(&host, &vault, action, args(), caller, &asker).unwrap_err();
    assert!(!why.contains("needs the running app"), "{why}");
    let cards = asker.0.borrow();
    assert_eq!(cards.len(), 1);
    assert!(
        cards[0].contains("Commit abc123def456 on b."),
        "{}",
        cards[0]
    );
    assert!(cards[0].contains("bundle fingerprint"), "{}", cards[0]);
    drop(cards);

    self_install::clear_test_job();
    crate::automated_history::finish_run(&vault, handle, "success").unwrap();
}
