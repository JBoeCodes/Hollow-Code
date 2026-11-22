//! Row U10: Local builds in the Update button, on the client's fakes (temp
//! folders, a fake codesign, a recording installer launcher). No network,
//! Keychain, `/Applications` or real sleep.

use super::client_tests::{info_plist, rig, Setup};
use super::config;
use super::install::UpdateInstallOutcome;
use super::last::{self, UpdateLastResult};
use super::local::{self, LocalOffer, LocalRecord};
use super::state::UpdatePhase;
use crate::install_brings::{Commit, PullRequest, Report};
use crate::self_install::{self, BuildReceipt, InstallState};
use crate::update_install::verify::Team;
use std::path::{Path, PathBuf};

const COMMIT: &str = "abc123def4567890abc123def4567890abc12345";

/// A built `Hollow.app` (Info.plist at `version`, provenance naming
/// `COMMIT`) under `root`, and its receipt.
fn built(root: &Path, version: &str) -> (PathBuf, BuildReceipt) {
    use std::os::unix::fs::PermissionsExt;
    let bundle = root
        .join("hollow-install-target-ws")
        .join("release/bundle/macos/Hollow.app");
    std::fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    std::fs::create_dir_all(bundle.join("Contents/Resources")).unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), info_plist(version)).unwrap();
    let exe = bundle.join("Contents/MacOS/hollow");
    std::fs::write(&exe, "#!/bin/sh\necho built\n").unwrap();
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(
        bundle.join("Contents/Resources/build-provenance.json"),
        format!(
            r#"{{"commit":"{COMMIT}","dirty":false,"sourceFingerprint":"{}","version":"{version}"}}"#,
            "f".repeat(64)
        ),
    )
    .unwrap();
    let receipt =
        self_install::record_receipt("ws", &root.join("worktree"), &bundle, "0011223344").unwrap();
    (bundle, receipt)
}

fn offer(receipt: &BuildReceipt, asked: bool) -> LocalOffer {
    LocalOffer {
        receipt: receipt.clone(),
        branch: "feat/x".into(),
        notes: "Brings 1 pull request".into(),
        asked,
    }
}

fn local_branch(phase: &UpdatePhase) -> Option<String> {
    match phase {
        UpdatePhase::Ready { release } => release.local.as_ref().map(|l| l.branch.clone()),
        _ => None,
    }
}

#[test]
fn the_team_is_the_bundles_own_only_when_it_is_the_configured_one() {
    let ours = Team::Id("TEAM000000".into());
    // A Developer ID local build of our team: checked as that team (and its
    // Developer ID requirement).
    assert_eq!(local::expected_team(Some("TEAM000000"), Some(&ours)), ours);
    // "Hollow Dev Signing" (no team) or ad hoc: ad hoc.
    assert_eq!(local::expected_team(None, Some(&ours)), Team::AdHoc);
    // Another team: ad hoc is expected, which that signature then fails.
    assert_eq!(
        local::expected_team(Some("ZZZZZZZZZZ"), Some(&ours)),
        Team::AdHoc
    );
    // No configured team (a test or override set to ad hoc): ad hoc.
    assert_eq!(local::expected_team(Some("TEAM000000"), None), Team::AdHoc);
    assert_eq!(
        local::expected_team(Some("TEAM000000"), Some(&Team::AdHoc)),
        Team::AdHoc
    );
}

#[test]
fn the_notes_list_what_the_build_brings() {
    let report = Report {
        missing: 1,
        build_id: Some("b".into()),
        installed: None,
        installed_from: "binary".into(),
        repo: None,
        repo_source: None,
        target: "feat/x".into(),
        target_commit: None,
        fetched_at: None,
        pull_requests: vec![PullRequest {
            number: 1469,
            title: "U5: the Update pill".into(),
            merge: "m".into(),
            merged_at: "t".into(),
            untested: vec![],
            entry_found: true,
        }],
        commits: vec![Commit {
            sha: "0123456789abcdef".into(),
            subject: "fix the thing".into(),
            at: "t".into(),
        }],
        note: Some("This build also includes uncommitted changes that aren't listed.".into()),
    };
    let notes = local::notes(&report);
    assert_eq!(
        notes,
        "Brings 1 pull request and 1 direct commit since the installed build.\n#1469 U5: the Update pill\n01234567 fix the thing\nThis build drops 1 commit from the installed build.\nThis build also includes uncommitted changes that aren't listed."
    );
}

/// The Local build's whole path: offered (in the Update button, channel
/// checks paused), Restart to update hands a copy of the built bundle to the
/// installer with the bundle's own version, `--expected-team -` and
/// `--allow-downgrade` (the installer's own parser takes them), and the
/// start record carries the receipt so the notice and the build receipt
/// after the restart say what went in.
#[test]
fn a_local_build_is_offered_installed_by_the_helper_and_recorded() {
    let r = rig(Setup::default());
    let (bundle, receipt) = built(&r.root, "0.5.15");
    let state = r.client.offer_local(offer(&receipt, false)).unwrap();
    assert_eq!(local_branch(&state.phase).as_deref(), Some("feat/x"));
    let UpdatePhase::Ready { release } = &state.phase else {
        panic!("{:?}", state.phase)
    };
    assert_eq!(release.version, "0.5.15");
    assert_eq!(release.commit.as_deref(), Some(COMMIT));
    let view = release.local.as_ref().unwrap();
    assert_eq!(
        (
            view.workspace_id.as_str(),
            view.build_id.as_str(),
            view.dirty
        ),
        ("ws", receipt.build_id.as_str(), false)
    );
    assert_eq!(r.client.local_build_id(), Some(receipt.build_id.clone()));

    // A channel check while it waits does nothing: no request, still ready.
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.downloads(), 0);
    assert_eq!(
        local_branch(&r.client.state().phase).as_deref(),
        Some("feat/x")
    );

    // Work runs: an answer, not an install.
    r.run(&["chat-1"]);
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::WorkRunning { .. }
    ));
    r.run(&[]);
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { ref version } if version == "0.5.15"
    ));
    let args = r.args();
    let flag = |name: &str| {
        let at = args.iter().position(|a| a == name).unwrap();
        args[at + 1].clone()
    };
    // The installer gets the copy under updates/local, not the build target.
    let copy = r
        .root
        .join("data/updates/local")
        .join(&receipt.build_id)
        .join("Hollow.app");
    assert_eq!(flag("--staged"), copy.to_string_lossy());
    assert_ne!(copy, bundle);
    assert_eq!(flag("--expected-version"), "0.5.15");
    assert_eq!(flag("--expected-team"), "-");
    assert!(args.iter().any(|a| a == "--allow-downgrade"));
    assert!(args.iter().any(|a| a == "--no-snapshot"));
    let parsed =
        crate::update_install::cli::parse(&args, None, Some("/x".into()), None, 0).unwrap();
    let crate::update_install::cli::Mode::Install(install) = parsed else {
        panic!("not an install")
    };
    assert_eq!(install.staged, copy);
    assert_eq!(install.team, Team::AdHoc);
    assert!(install.allow_downgrade);
    assert_eq!(install.expected_version, "0.5.15");

    // The start record carries the receipt; after the restart (another
    // pid, the new version running) the notice names the local build.
    let last_path = r.root.join("data/updates").join(last::FILE);
    let started = last::read_started(&last_path).unwrap();
    assert_eq!(started.build_id.as_deref(), Some(receipt.build_id.as_str()));
    let record = started.local.clone().unwrap();
    assert_eq!(record.receipt, receipt);
    let running = crate::app_menu::parse_identity(
        &format!(
            r#"{{"commit":"{COMMIT}","sourceFingerprint":"{}"}}"#,
            "f".repeat(64)
        ),
        "0.5.15",
    );
    let after = last::read(&last_path, None, "0.5.15", Some(&running), 2222).unwrap();
    let UpdateLastResult::Updated { local, version, .. } = &after else {
        panic!("{after:?}")
    };
    assert_eq!(version, "0.5.15");
    assert_eq!(local.as_ref().unwrap().branch, "feat/x");

    // The build receipt is settled once, acknowledged (the Update button's
    // notice is the one shown), and reads as installed when the running
    // app reports the build's commit.
    let dir = r.root.join("self-install");
    let log = r.root.join("install-update.log");
    assert!(local::settle_receipt(&dir, &started.install_id, &record, &after, &log).unwrap());
    assert!(!local::settle_receipt(&dir, &started.install_id, &record, &after, &log).unwrap());
    assert!(self_install::unacknowledged_in(&dir).is_none());
    let receipt_now = self_install::receipt_in(&dir, &running).unwrap();
    assert_eq!(receipt_now.state, InstallState::Installed);
    assert!(receipt_now.result.ok);
    assert_eq!(receipt_now.result.build.as_ref(), Some(&receipt));
}

#[test]
fn a_local_build_that_went_back_is_recorded_as_not_installed() {
    let dir = tempfile::tempdir().unwrap();
    let (_bundle, receipt) = built(dir.path(), "0.5.15");
    let record = LocalRecord {
        receipt,
        branch: "feat/x".into(),
    };
    let went_back = UpdateLastResult::WentBack {
        attempted: "0.5.15".into(),
        restored: Some("0.6.10".into()),
        reason: "it did not start twice".into(),
        at: "2026-11-15T10:00:00Z".into(),
        manual: false,
        data_restored: false,
    };
    let state = dir.path().join("state");
    assert!(local::settle_receipt(&state, "id-1", &record, &went_back, Path::new("/l")).unwrap());
    let running = crate::app_menu::parse_identity(r#"{"commit":"0011223344"}"#, "0");
    let receipt = self_install::receipt_in(&state, &running).unwrap();
    assert_eq!(receipt.state, InstallState::Failed);
    assert_eq!(receipt.result.message, "it did not start twice");
}

/// Not now on a Local build takes it out of the button, keeps it out at
/// the next launch's offer, and the person's Show in Update button brings
/// it back. A new build withdraws it. Channel checks resume after.
#[test]
fn not_now_and_a_new_build_withdraw_the_local_build() {
    let r = rig(Setup::default());
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, false)).unwrap();
    let state = r.client.dismiss("0.5.15").unwrap();
    assert!(local_branch(&state.phase).is_none(), "{:?}", state.phase);
    assert_eq!(state.phase, UpdatePhase::Idle);
    assert_eq!(
        r.client.settings().dismissed_local_build.as_deref(),
        Some(receipt.build_id.as_str())
    );
    // The launch's offer of the same build is skipped.
    let again = r.client.offer_local(offer(&receipt, false)).unwrap();
    assert!(local_branch(&again.phase).is_none());
    // Asked for: offered, and the Not now is forgotten.
    let asked = r.client.offer_local(offer(&receipt, true)).unwrap();
    assert_eq!(local_branch(&asked.phase).as_deref(), Some("feat/x"));
    assert_eq!(r.client.settings().dismissed_local_build, None);

    // Another build's id doesn't withdraw it; a new build (any) does.
    assert!(!r.client.withdraw_local(Some("other-build")));
    assert!(r.client.withdraw_local(None));
    assert_eq!(r.client.local_build_id(), None);
    // The channel is checked again.
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.downloads(), 1);
}

#[test]
fn update_when_chats_finish_waits_for_a_local_build_and_a_withdraw_ends_it() {
    let r = rig(Setup::default());
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, true)).unwrap();
    r.run(&["chat-1"]);
    // As the agent's approved install does: wait (no expiry).
    let state = r
        .client
        .install_when_idle(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(state.waiting);
    assert!(r.client.withdraw_local(None));
    assert!(!r.client.state().waiting, "the wait ended with the build");
}

#[test]
fn a_local_build_whose_bytes_changed_is_refused_before_and_at_install() {
    let r = rig(Setup::default());
    let (bundle, receipt) = built(&r.root, "0.5.15");
    let exe = bundle.join("Contents/MacOS/hollow");
    std::fs::write(&exe, "#!/bin/sh\necho swapped\n").unwrap();
    assert_eq!(
        r.client.offer_local(offer(&receipt, true)).unwrap_err(),
        local::CHANGED
    );
    // The refused copy isn't kept.
    let local_root = r.root.join("data/updates/local");
    assert!(!local_root.join(&receipt.build_id).exists());
    std::fs::write(&exe, "#!/bin/sh\necho built\n").unwrap();
    r.client.offer_local(offer(&receipt, true)).unwrap();
    // The copy itself swapped while it waited: the install refuses, starts
    // nothing and drops the copy.
    let copy = local_root.join(&receipt.build_id).join("Hollow.app");
    std::fs::write(
        copy.join("Contents/MacOS/hollow"),
        "#!/bin/sh\necho swapped\n",
    )
    .unwrap();
    assert_eq!(r.client.install_now(None).unwrap_err(), local::CHANGED);
    assert!(matches!(r.client.state().phase, UpdatePhase::Failed { .. }));
    assert_eq!(r.client.local_build_id(), None);
    assert!(!copy.exists());
    assert_eq!(r.spawns(), 0);
}

/// Review LOW: what is hashed is what is installed. The bundle is copied
/// when offered; the build target changing afterwards (a rebuild) doesn't
/// change the copy, which still matches the receipt and is installed.
#[test]
fn the_installer_gets_the_copy_made_when_the_build_was_offered() {
    let r = rig(Setup::default());
    let (bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, true)).unwrap();
    std::fs::write(
        bundle.join("Contents/MacOS/hollow"),
        "#!/bin/sh\necho rebuilt\n",
    )
    .unwrap();
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
    let copy = r
        .root
        .join("data/updates/local")
        .join(&receipt.build_id)
        .join("Hollow.app");
    assert_eq!(
        std::fs::read_to_string(copy.join("Contents/MacOS/hollow")).unwrap(),
        "#!/bin/sh\necho built\n"
    );
    assert_eq!(
        self_install::bundle_digest(&copy).unwrap(),
        receipt.bundle_digest
    );
    let args = r.args();
    let at = args.iter().position(|a| a == "--staged").unwrap();
    assert_eq!(args[at + 1], copy.to_string_lossy());
}

/// A Mac not connected to updates (U0 not done) still installs Local
/// builds; withdrawing one goes back to "not connected".
#[test]
fn a_local_build_needs_no_updates_token() {
    let r = rig(Setup {
        token: None,
        ..Setup::default()
    });
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    let state = r.client.offer_local(offer(&receipt, true)).unwrap();
    assert_eq!(local_branch(&state.phase).as_deref(), Some("feat/x"));
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
    let r = rig(Setup {
        token: None,
        ..Setup::default()
    });
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, true)).unwrap();
    r.client.withdraw_local(None);
    assert_eq!(
        r.client.state().phase,
        UpdatePhase::NotSetUp {
            reason: config::NOT_CONNECTED.into()
        }
    );
}

#[test]
fn a_review_copy_never_offers_a_local_build() {
    let r = rig(Setup {
        review_copy: true,
        ..Setup::default()
    });
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    assert_eq!(
        r.client.offer_local(offer(&receipt, true)).unwrap_err(),
        local::REVIEW_COPY
    );
}

/// The consolidation's grep test: the install card's IPC command, the
/// 30-minute waiter, its banner and their commands are gone from the
/// source; the agent's install action stays, always asks, and an
/// unattended run or automation never runs it.
#[test]
fn the_old_install_card_and_its_waiter_are_gone() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("src/self_install/scheduled.rs").exists());
    let read = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
    let lib = read("src/lib.rs");
    for gone in [
        "build_api::install_built_app",
        "pending_self_install",
        "cancel_scheduled_install",
    ] {
        assert!(!lib.contains(gone), "lib.rs still registers {gone}");
    }
    let actions = read("src/app_actions.rs");
    let api = read("src/commands/build_api.rs");
    let si = read("src/self_install.rs");
    for (name, text) in [
        ("app_actions", &actions),
        ("build_api", &api),
        ("self_install", &si),
    ] {
        for gone in [
            "INSTALL_WAIT",
            "self_install::scheduled",
            "scheduled-install-event",
            "install_and_quit",
            "InstallScript",
        ] {
            assert!(!text.contains(gone), "{name} still has {gone}");
        }
    }
    let front = root.join("../src");
    assert!(!front.join("store/scheduledInstall.ts").exists());
    for f in [
        "api.ts",
        "components/AppBuildPanel.tsx",
        "components/SelfInstallNotice.tsx",
    ] {
        let text = std::fs::read_to_string(front.join(f)).unwrap();
        for gone in [
            "installBuiltApp",
            "pendingSelfInstall",
            "cancelScheduledInstall",
            "scheduled-install-event",
            "Install and restart",
        ] {
            assert!(!text.contains(gone), "{f} still has {gone}");
        }
    }

    // The agent's way to propose a branch build stays, always asks, and no
    // automation or unattended (overnight) run may run it.
    let install = crate::app_actions::find("build.install_built_app").unwrap();
    assert_eq!(install.command, None);
    assert!(crate::app_actions::always_asks("build.install_built_app"));
    let dir = tempfile::tempdir().unwrap();
    let args = serde_json::Map::new();
    assert!(crate::main_agent::unattended::allows(
        dir.path(),
        "build.install_built_app",
        false,
        &args,
        true
    )
    .is_err());
    // No agent action installs a channel update or offers a Local build:
    // no action mirrors a command of the updater's IPC.
    let ipc = read("src/update_client/ipc.rs");
    for action in crate::app_actions::ACTIONS {
        if let Some(command) = action.command {
            assert!(
                !ipc.contains(&format!("pub async fn {command}(")),
                "{} reaches the updater's {command}",
                action.id
            );
        }
    }
    assert!(crate::app_actions::USER_ONLY.contains(&"update_offer_local_build"));
}

/// Review B1: "Update when chats finish" armed for a channel update never
/// installs a Local build offered meanwhile: the offer ends the wait, and
/// the wait's own looks refuse a build of another kind.
#[test]
fn a_wait_armed_for_a_channel_update_never_installs_a_local_build() {
    use std::sync::atomic::AtomicU8;
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.run(&["chat-1"]);
    let state = r
        .client
        .install_when_idle(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(state.waiting);
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    let state = r.client.offer_local(offer(&receipt, false)).unwrap();
    assert!(!state.waiting, "the offer ended the channel update's wait");
    // Everything goes idle: nothing installs.
    r.run(&[]);
    assert_eq!(r.spawns(), 0);
    // A wait armed for the channel (None) that is still looking when the
    // Local build arrives ends without installing it.
    let ticket = AtomicU8::new(super::install::WAITING);
    assert_eq!(
        r.client.wait_loop_for(
            &ticket,
            super::install::WaitAction::Update,
            super::install::POLL,
            None
        ),
        super::install::WaitEnd::Cancelled
    );
    assert_eq!(r.spawns(), 0);
    assert_eq!(r.client.local_build_id(), Some(receipt.build_id.clone()));
    // Armed for another Local build: the same.
    let ticket = AtomicU8::new(super::install::WAITING);
    assert_eq!(
        r.client.wait_loop_for(
            &ticket,
            super::install::WaitAction::Update,
            super::install::POLL,
            Some("other".into())
        ),
        super::install::WaitEnd::Cancelled
    );
    assert_eq!(r.spawns(), 0);
    // The person's own wait on the Local build installs it.
    let ticket = AtomicU8::new(super::install::WAITING);
    assert_eq!(
        r.client.wait_loop_for(
            &ticket,
            super::install::WaitAction::Update,
            super::install::POLL,
            Some(receipt.build_id.clone())
        ),
        super::install::WaitEnd::Installed
    );
    assert_eq!(r.spawns(), 1);
}

/// The agent's approved install (the same build offered again with
/// `asked`, then its own wait) keeps a wait already armed for that build.
#[test]
fn offering_the_same_local_build_again_keeps_its_wait() {
    let r = rig(Setup::default());
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, false)).unwrap();
    r.run(&["chat-1"]);
    assert!(
        r.client
            .install_when_idle(std::time::Duration::from_secs(5))
            .unwrap()
            .waiting
    );
    assert!(r.client.offer_local(offer(&receipt, true)).unwrap().waiting);
    assert!(!r.client.cancel_wait().waiting);
}

/// Review M2: a Local build carrying the running version reads as updated
/// only when the running app is that build (its source fingerprint), and
/// its receipt is then settled as installed; otherwise "not installed".
#[test]
fn a_local_build_of_the_running_version_is_updated_only_when_it_runs() {
    let r = rig(Setup::default());
    let (_bundle, receipt) = built(&r.root, "0.6.10");
    r.client.offer_local(offer(&receipt, true)).unwrap();
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
    let path = r.root.join("data/updates").join(last::FILE);
    let started = last::read_started(&path).unwrap();
    let record = started.local.clone().unwrap();
    let identity = |fp: &str, commit: &str| {
        crate::app_menu::parse_identity(
            &format!(r#"{{"commit":"{commit}","sourceFingerprint":"{fp}"}}"#),
            "0.6.10",
        )
    };
    // The old app still runs (another launch, same version): not updated.
    let old = identity(&"e".repeat(64), "0011223344556677");
    let result = last::read(&path, None, "0.6.10", Some(&old), 2222).unwrap();
    assert!(
        matches!(result, UpdateLastResult::NotInstalled { ref attempted, .. } if attempted == "0.6.10"),
        "{result:?}"
    );
    // No identity at all: never "Updated" for a Local build.
    assert!(matches!(
        last::read(&path, None, "0.6.10", None, 2222),
        Some(UpdateLastResult::NotInstalled { .. })
    ));
    let dir = r.root.join("self-install");
    let log = r.root.join("install-update.log");
    assert!(local::settle_receipt(&dir, &started.install_id, &record, &result, &log).unwrap());
    let receipt_now = self_install::receipt_in(&dir, &old).unwrap();
    assert!(!receipt_now.result.ok);
    assert_eq!(receipt_now.state, InstallState::Failed);
    // The build runs: updated.
    let new = identity(&"f".repeat(64), COMMIT);
    assert!(matches!(
        last::read(&path, None, "0.6.10", Some(&new), 2222),
        Some(UpdateLastResult::Updated { .. })
    ));
}

#[test]
fn a_running_build_is_matched_by_fingerprint_else_by_clean_commit() {
    let (_d, receipt) = {
        let d = tempfile::tempdir().unwrap();
        let (_b, r) = built(d.path(), "0.5.15");
        (d, r)
    };
    let id = |json: &str| crate::app_menu::parse_identity(json, "0");
    let fp = "f".repeat(64);
    assert!(local::is_running(
        &receipt,
        &id(&format!(r#"{{"sourceFingerprint":"{fp}"}}"#))
    ));
    assert!(!local::is_running(
        &receipt,
        &id(&format!(
            r#"{{"commit":"{COMMIT}","sourceFingerprint":"{}"}}"#,
            "e".repeat(64)
        ))
    ));
    // No fingerprint: the commit (a prefix of 7 or more), never a dirty one.
    assert!(local::is_running(
        &receipt,
        &id(r#"{"commit":"abc123def456"}"#)
    ));
    assert!(!local::is_running(&receipt, &id(r#"{"commit":"abc123"}"#)));
    assert!(!local::is_running(
        &receipt,
        &id(r#"{"commit":"abc123def456","dirty":true}"#)
    ));
    assert!(!local::is_running(&receipt, &id("{}")));
}

/// Review M3: a channel update that was ready is kept aside while a Local
/// build waits and is back in the button (no new download) when the Local
/// build leaves; a channel switch meanwhile drops it.
#[test]
fn the_channel_update_a_local_build_replaced_comes_back() {
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.downloads(), 1);
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, false)).unwrap();
    let state = r.client.dismiss("0.5.15").unwrap();
    let UpdatePhase::Ready { release } = &state.phase else {
        panic!("{:?}", state.phase)
    };
    assert_eq!(release.version, "0.6.11");
    assert!(release.local.is_none());
    // The next check confirms it without downloading again; it installs.
    r.publish("0.6.11", "main");
    r.client.check();
    assert_eq!(r.downloads(), 1);
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { ref version } if version == "0.6.11"
    ));

    // A channel switch while the Local build waits: nothing comes back.
    let r = rig(Setup::default());
    r.publish("0.6.11", "main");
    r.client.check();
    r.client
        .offer_local(offer(&receipt_for(&r), false))
        .unwrap();
    r.client
        .set_channel(super::state::Channel::Nightly)
        .unwrap();
    assert!(r.client.withdraw_local(None));
    assert!(!matches!(r.client.state().phase, UpdatePhase::Ready { .. }));
}

fn receipt_for(r: &super::client_tests::Rig) -> BuildReceipt {
    built(&r.root, "0.5.15").1
}

/// Review LOW: while a Local build's install starts, the build can't be
/// withdrawn or replaced (a new app build is refused: `installing_local`
/// is what `self_install::start_build` asks), and its copy stays.
#[test]
fn a_local_build_being_installed_stays_put() {
    let r = rig(Setup::default());
    let (_bundle, receipt) = built(&r.root, "0.5.15");
    r.client.offer_local(offer(&receipt, true)).unwrap();
    assert!(!r.client.installing_local());
    assert!(matches!(
        r.client.install_now(None).unwrap(),
        UpdateInstallOutcome::Started { .. }
    ));
    assert!(r.client.installing_local());
    assert!(!r.client.withdraw_local(None));
    assert!(r.client.offer_local(offer(&receipt, true)).is_err());
    assert!(r
        .root
        .join("data/updates/local")
        .join(&receipt.build_id)
        .join("Hollow.app")
        .is_dir());
    let source = include_str!("../self_install.rs");
    let guard = source.find("installing_local_build()").unwrap();
    let forget = source
        .find("forget_finished_in(&dir);\n        }\n        *job")
        .unwrap();
    assert!(
        guard < forget,
        "start_build checks before it forgets the saved build"
    );
}
