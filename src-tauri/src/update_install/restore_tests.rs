//! `--go-back --restore-snapshot` (row U6) on the U4 harness: fake apps,
//! real SQLite files in temp folders, injected failures and crashes.

use super::health::Timing;
use super::journal;
use super::run::{self, GoBackArgs};
use super::tests::{H, TEAM_OK};
use super::{cli, verify, Exit};
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn live_db(sessions: &Path) -> PathBuf {
    std::fs::create_dir_all(sessions).unwrap();
    let path = sessions.join("vault-aaaa.db");
    Connection::open(&path)
        .unwrap()
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE turns(id INTEGER PRIMARY KEY, body TEXT);
             CREATE VIRTUAL TABLE chat_search_fts USING fts5(body);
             INSERT INTO chat_search_fts(body) VALUES ('before');
             INSERT INTO turns(body) VALUES ('before');",
        )
        .unwrap();
    path
}

fn bodies(db: &Path) -> Vec<String> {
    let conn = Connection::open(db).unwrap();
    let mut stmt = conn.prepare("SELECT body FROM turns ORDER BY id").unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    rows
}

fn version_of(bundle: &Path) -> Option<String> {
    verify::read_info(bundle).ok().map(|i| i.version)
}

/// Installed 0.6.2 over 0.6.1 with a Sessions snapshot, then a chat the
/// newer version wrote. Returns the harness, the live database and the
/// snapshot folder.
fn updated_with_snapshot() -> (H, PathBuf, PathBuf) {
    let h = H::new().with_installed();
    let sessions = h.root.join("support").join("Sessions");
    let live = live_db(&sessions);
    let staged = h.staged("0.6.2", TEAM_OK);
    let mut args = h.args(&staged, "0.6.2");
    args.snapshot_source = Some((sessions, true));
    let out = h.install(&args);
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    let folder = h.journal(&out).snapshot.unwrap().folder.unwrap();
    Connection::open(&live)
        .unwrap()
        .execute("INSERT INTO turns(body) VALUES ('after')", [])
        .unwrap();
    h.world.borrow_mut().procs.clear();
    (h, live, folder)
}

fn args(h: &H, restore: Option<PathBuf>) -> GoBackArgs {
    GoBackArgs {
        app: h.app(),
        state_dir: h.state.clone(),
        relaunch: true,
        install_id: format!("20261115-130000-{:08x}", rand::random::<u32>()),
        timing: Timing::default(),
        restore,
    }
}

fn temps(live: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(live.parent().unwrap())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.to_string_lossy().contains(super::restore::TEMP_SUFFIX))
        .collect()
}

#[test]
fn go_back_with_restore_puts_the_chats_back_and_keeps_the_newer_ones_aside() {
    let (h, live, folder) = updated_with_snapshot();
    let out = run::go_back(&args(&h, Some(folder.clone())), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.1"));
    assert_eq!(bodies(&live), ["before"], "the data from before the update");
    let aside = PathBuf::from(format!("{}.before-go-back", live.display()));
    assert_eq!(
        bodies(&aside),
        ["before", "after"],
        "the newer chat is kept"
    );
    assert!(temps(&live).is_empty());
    let record = h.journal(&out);
    let steps: Vec<&str> = record.steps.iter().map(|s| s.step.as_str()).collect();
    assert_eq!(
        steps,
        [
            "prepared",
            "data-restored",
            "promoted",
            "started",
            "healthy"
        ]
    );
    let restore = record.restore.expect("the journal names the restore");
    assert_eq!(restore.databases, std::slice::from_ref(&live));
    assert_eq!(
        std::fs::canonicalize(&restore.folder).unwrap(),
        std::fs::canonicalize(&folder).unwrap()
    );
    // A second go back with a restore finds the first aside name taken.
    assert_eq!(
        super::restore::aside_suffix(&live, "x/y"),
        ".before-go-back-x_y"
    );
}

fn launches(h: &H) -> usize {
    h.world.borrow().launches.len()
}

/// Review LOW: one aside suffix per database. Only the `-wal`'s plain aside
/// name is taken, yet every file of the database goes aside with the
/// install id, so the set stays together.
#[test]
fn every_file_of_a_database_goes_aside_under_one_suffix() {
    let (h, live, folder) = updated_with_snapshot();
    let wal_taken = PathBuf::from(format!("{}-wal.before-go-back", live.display()));
    std::fs::write(&wal_taken, b"an earlier go back's").unwrap();
    let a = args(&h, Some(folder));
    let out = run::go_back(&a, &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    let suffix = format!(".before-go-back-{}", journal::safe_id(&a.install_id));
    let restore = h.journal(&out).restore.unwrap();
    assert!(!restore.aside.is_empty());
    for (from, to) in &restore.aside {
        assert_eq!(
            to,
            &PathBuf::from(format!("{}{suffix}", from.display())),
            "{from:?}"
        );
    }
    assert!(!Path::new(&format!("{}.before-go-back", live.display())).exists());
    assert_eq!(std::fs::read(&wal_taken).unwrap(), b"an earlier go back's");
    assert_eq!(
        bodies(&PathBuf::from(format!("{}{suffix}", live.display()))),
        ["before", "after"]
    );
}

/// Review M2: Hollow quit for the go back, so one that changed nothing
/// opens the installed app again (with `--relaunch`), and says so.
#[test]
fn a_go_back_that_changed_nothing_opens_hollow_again() {
    let (h, live, folder) = updated_with_snapshot();
    std::fs::write(folder.join("vault-aaaa.db"), b"not a database at all").unwrap();
    let before = launches(&h);
    let out = run::go_back(&args(&h, Some(folder.clone())), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::RestoreFailed, "{}", out.message);
    assert!(
        out.message.ends_with("Hollow was opened again"),
        "{}",
        out.message
    );
    assert_eq!(h.world.borrow().launches[before..], [h.app()]);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(bodies(&live), ["before", "after"]);
    // `--no-relaunch` leaves it closed (the fake app opened above quits).
    h.world.borrow_mut().procs.clear();
    let mut quiet = args(&h, Some(folder));
    quiet.relaunch = false;
    let before = launches(&h);
    let out = run::go_back(&quiet, &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::RestoreFailed, "{}", out.message);
    assert_eq!(launches(&h), before);
}

/// Review M3: the restore copy and its folder are flushed before the
/// renames; a flush that can't happen is an error, not a silent pass.
#[test]
fn the_restore_copy_is_flushed_before_it_moves() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("copy.db");
    std::fs::write(&file, b"bytes").unwrap();
    assert_eq!(super::restore::sync_file_and_folder(&file), Ok(()));
    let missing = dir.path().join("gone.db");
    let err = super::restore::sync_file_and_folder(&missing).unwrap_err();
    assert!(err.contains("could not flush"), "{err}");
}

/// Review LOW: restore copies no plan names (a crash after they were laid
/// and before the plan was saved), with any `-wal` or `-shm` a check left,
/// are swept by the next run's recovery.
#[test]
fn orphan_restore_copies_are_swept_by_the_next_run() {
    let (h, live, _) = updated_with_snapshot();
    for suffix in ["", "-wal", "-shm"] {
        std::fs::write(
            format!("{}{}{suffix}", live.display(), super::restore::TEMP_SUFFIX),
            b"left",
        )
        .unwrap();
    }
    assert_eq!(temps(&live).len(), 3);
    run::recover_all(&h.layout(), &h.state, &h.deps(&|_| {})).unwrap();
    assert!(temps(&live).is_empty());
    assert_eq!(
        bodies(&live),
        ["before", "after"],
        "the live data is untouched"
    );
}

/// Review LOW: settling a journal with a restore removes the copy's `-wal`
/// and `-shm` too, not only the copy.
#[test]
fn remove_temps_takes_the_copys_wal_and_shm_too() {
    let dir = tempfile::tempdir().unwrap();
    let live = dir.path().join("vault.db");
    let temp = PathBuf::from(format!("{}{}", live.display(), super::restore::TEMP_SUFFIX));
    for suffix in ["", "-wal", "-shm"] {
        std::fs::write(format!("{}{suffix}", temp.display()), b"x").unwrap();
    }
    let ino = {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(&temp).unwrap().ino()
    };
    let mut record = journal::Record::new(
        journal::Kind::GoBack,
        "20261115-130000-cccc",
        1,
        &dir.path().join("Hollow.app"),
        "2026-11-15T13:00:00Z",
    );
    record.restore = Some(super::restore::RestoreRecord::default());
    record.plan = Some(vec![journal::Move {
        op: journal::Op::Rename,
        from: temp.clone(),
        to: live.clone(),
        ino,
    }]);
    super::restore::remove_temps(&record);
    assert!(temps(&live).is_empty());
}

#[test]
fn a_broken_snapshot_changes_nothing() {
    let (h, live, folder) = updated_with_snapshot();
    std::fs::write(folder.join("vault-aaaa.db"), b"not a database at all").unwrap();
    let out = run::go_back(&args(&h, Some(folder)), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::RestoreFailed, "{}", out.message);
    assert!(
        out.message.contains("nothing was changed"),
        "{}",
        out.message
    );
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(bodies(&live), ["before", "after"]);
    assert!(temps(&live).is_empty());
    assert_eq!(h.journal(&out).state, "failed");
}

#[test]
fn a_folder_that_is_not_an_installer_snapshot_is_refused() {
    let (h, live, _) = updated_with_snapshot();
    let elsewhere = h.root.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let out = run::go_back(&args(&h, Some(elsewhere)), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::RestoreFailed, "{}", out.message);
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(bodies(&live), ["before", "after"]);
}

#[test]
fn a_failed_swap_after_the_restore_undoes_the_restore_too() {
    let (h, live, folder) = updated_with_snapshot();
    // The plan: live .db aside, -wal aside, -shm aside (when present), the
    // restore copy in, then the swap: fail the last op.
    let moves = {
        let (_, snap) = super::restore::find(&h.state, &folder).unwrap();
        snap.files.len()
    };
    assert_eq!(moves, 1);
    let before = h.fs.ops.get();
    let companions = ["", "-wal", "-shm", "-journal"]
        .iter()
        .filter(|s| Path::new(&format!("{}{s}", live.display())).exists())
        .count();
    h.fs.fail_at.set(Some(before + companions + 1));
    let launched = launches(&h);
    let out = run::go_back(&args(&h, Some(folder)), &h.deps(&|_| {}));
    assert_eq!(out.exit, Exit::Failed, "{}", out.message);
    assert!(
        out.message.contains("every move was undone"),
        "{}",
        out.message
    );
    assert_eq!(launches(&h), launched + 1, "Hollow is opened again");
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(bodies(&live), ["before", "after"]);
    assert!(temps(&live).is_empty());
    assert!(!Path::new(&format!("{}.before-go-back", live.display())).exists());
}

#[test]
fn a_crash_between_the_restore_and_the_swap_is_undone_by_the_next_run() {
    let (h, live, folder) = updated_with_snapshot();
    let hook = |s: &str| {
        if s == "data-restored" {
            panic!("injected crash at {s}");
        }
    };
    let a = args(&h, Some(folder));
    let deps = h.deps(&hook);
    let crashed =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run::go_back(&a, &deps)));
    assert!(crashed.is_err());
    assert_eq!(bodies(&live), ["before"], "restored at the crash");
    // The next run (any install or go back) recovers first.
    let notes = run::recover_all(&h.layout(), &h.state, &h.deps(&|_| {})).unwrap();
    assert!(notes.iter().any(|n| n.contains("Undone")), "{notes:?}");
    assert_eq!(version_of(&h.app()).as_deref(), Some("0.6.2"));
    assert_eq!(bodies(&live), ["before", "after"], "the live data is back");
    assert!(temps(&live).is_empty(), "the restore copy is removed");
    let (all, _) = journal::list(&h.state);
    assert_eq!(all.last().unwrap().record.state, "rolled-back");
}

#[test]
fn restore_snapshot_goes_only_with_go_back() {
    let parse = |a: &[&str]| {
        let args: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        cli::parse(&args, None, Some("/s".into()), Some("/x".into()), 0)
    };
    match parse(&[
        "--go-back",
        "--restore-snapshot",
        "/s/snapshots/a-from-0.6.1",
    ]) {
        Ok(cli::Mode::GoBack(g)) => {
            assert_eq!(g.restore, Some(PathBuf::from("/s/snapshots/a-from-0.6.1")))
        }
        other => panic!("{other:?}"),
    }
    assert!(parse(&["--status", "--restore-snapshot", "/s/x"]).is_err());
    assert!(parse(&["--go-back", "--restore-snapshot", "relative"]).is_err());
    assert!(cli::USAGE.contains("--restore-snapshot") && cli::USAGE.contains("12 the snapshot"));
    assert_eq!(Exit::RestoreFailed.code(), 12);
    assert_eq!(Exit::RestoreFailed.name(), "restore-failed");
}

/// Found by row U6: a read-only `quick_check` can't validate an FTS5 index
/// (it needs to write), and the Sessions database has one
/// (`chat_search_fts`), so every real snapshot used to fail. That one line
/// is not damage; real damage still fails.
#[test]
fn a_database_with_an_fts5_table_snapshots_and_restores() {
    let dir = tempfile::tempdir().unwrap();
    let live = live_db(&dir.path().join("Sessions"));
    let snap =
        super::snapshot::take(std::slice::from_ref(&live), &dir.path().join("snap")).unwrap();
    super::restore::check(&snap.files[0].copy, 0).unwrap();
    // A damaged page still fails.
    let broken = dir.path().join("broken.db");
    let mut bytes = std::fs::read(&snap.files[0].copy).unwrap();
    let page = 4096.min(bytes.len() / 2);
    for b in bytes.iter_mut().skip(page).take(64) {
        *b = 0xA5;
    }
    std::fs::write(&broken, &bytes).unwrap();
    assert!(super::restore::check(&broken, 0).is_err());
}
