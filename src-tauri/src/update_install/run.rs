//! One install or go back, start to finish (the order is in the module doc
//! of `update_install`). Each function returns an [`Outcome`]; nothing here
//! exits the process.

use super::effects::Deps;
use super::health::{self, Start, Timing};
use super::journal::{self, Attempt, Journal, Kind, Record, Snapshot};
use super::lock::{self, LockError};
use super::verify::{self, Team};
use super::{iso, restore, snapshot, swap, Exit, Layout, Outcome};
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) const FAILURE_FILE: &str = "last-install-failure.json";
/// Reopen attempts before the automatic go back.
pub(crate) const ATTEMPTS: u32 = 2;

#[derive(Debug, Clone)]
pub(crate) struct InstallArgs {
    pub(crate) staged: PathBuf,
    pub(crate) expected_version: String,
    pub(crate) team: Team,
    pub(crate) app: PathBuf,
    pub(crate) state_dir: PathBuf,
    /// `None`: `--no-snapshot`. `Some((path, explicit))` otherwise.
    pub(crate) snapshot_source: Option<(PathBuf, bool)>,
    pub(crate) relaunch: bool,
    pub(crate) allow_downgrade: bool,
    pub(crate) install_id: String,
    pub(crate) timing: Timing,
}

#[derive(Debug, Clone)]
pub(crate) struct GoBackArgs {
    pub(crate) app: PathBuf,
    pub(crate) state_dir: PathBuf,
    pub(crate) relaunch: bool,
    pub(crate) install_id: String,
    pub(crate) timing: Timing,
    /// `--restore-snapshot`: put the Sessions databases back from this
    /// snapshot folder before the swap (row U6, [`super::restore`]).
    pub(crate) restore: Option<PathBuf>,
}

/// `20261115-093012-1a2b3c4d`: UTC date and time, then 4 random bytes.
pub(crate) fn new_install_id(now_ms: u64) -> String {
    let stamp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms as i64)
        .map(|t| t.format("%Y%m%d-%H%M%S").to_string())
        .unwrap_or_else(|| "00000000-000000".into());
    format!("{stamp}-{:08x}", rand::random::<u32>())
}

fn guard_test_paths(app: &Path, state_dir: &Path) {
    // Tests never touch the real app or the real app data.
    if cfg!(test) {
        assert!(
            !app.starts_with("/Applications"),
            "a test reached /Applications"
        );
        assert!(
            !state_dir.to_string_lossy().contains("Application Support"),
            "a test reached the real app data"
        );
    }
}

/// Can this account create and remove a file in `dir`? (`access(2)` is
/// not enough: ACLs and the read-only system volume answer differently.)
pub(crate) fn writable(dir: &Path, pid: i32) -> Result<(), String> {
    let probe = dir.join(format!(".Hollow.update-write-probe-{pid}"));
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
    {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(format!(
            "Hollow can't update itself here: this account can't write to {} ({e}). Updating needs an account that can (an administrator account, the macOS default). Nothing was changed.",
            dir.display()
        )),
    }
}

/// Settle or undo whatever earlier runs left: their journals, and refuse to
/// guess about leftovers no journal explains.
pub(crate) fn recover_all(
    layout: &Layout,
    state_dir: &Path,
    deps: &Deps,
) -> Result<Vec<String>, String> {
    let mut notes = Vec::new();
    let (journals, unreadable) = journal::list(state_dir);
    // An unreadable journal (damaged, missing a required field, or another
    // schema) is moved aside, not obeyed. One whose text still names a state
    // from which apps may have moved blocks; any other one blocks only the
    // leftover retry below, and only while it is newer than the newest
    // settled journal.
    for (path, why) in unreadable {
        let aside = path.with_extension("json.bad");
        std::fs::rename(&path, &aside).map_err(|e| {
            format!(
                "the install journal {} can't be read ({why}) or moved aside ({e}); resolve by hand",
                path.display()
            )
        })?;
        notes.push(format!(
            "moved the unreadable journal {} aside ({why})",
            path.display()
        ));
    }
    let set_aside = journal::recent_set_aside(state_dir);
    journal::prune_set_aside(state_dir, 5);
    if let Some((path, _)) = set_aside.iter().find(|(_, moving)| *moving) {
        return Err(format!(
            "the install journal {} could not be read and was in the middle of moving apps, so nothing will be installed or put back until a person checks. Check that {} opens and is the version you want; if not, swap it with {} by hand (or run `node scripts/install-macos-app.mjs --rollback` from a Hollow checkout), since --go-back is blocked too. Then delete that journal file.",
            path.display(),
            layout.installed.display(),
            layout.previous.display()
        ));
    }
    for mut j in journals.into_iter().filter(|j| !j.record.settled()) {
        let now = iso(deps.clock.now_ms());
        let jl = Layout::for_app(&j.record.app.clone());
        let how = swap::recover(deps.fs, deps.signer, &jl, &mut j, &now)
            .map_err(|why| format!("install {} could not be recovered: {why}", j.record.id))?;
        let failed_starts = j
            .record
            .attempts
            .iter()
            .filter(|a| a.outcome != "healthy")
            .count();
        if how == swap::Recovered::Undone && swap::last_attempt_failed(&j.record) {
            let reason = format!(
                "Hollow {} did not start ({failed_starts} failed tries); the installer stopped before going back, and the next run finished it",
                j.record.to_version.as_deref().unwrap_or("?")
            );
            write_failure(state_dir, &j, deps, &reason, true);
        }
        restore::remove_temps(&j.record);
        finish(&j, deps);
        notes.push(format!("recovered install {}: {how:?}", j.record.id));
    }
    // A removal that failed earlier is retried while the newest journal says
    // nothing there is needed: any settled install leaves no app in
    // `staging`, and `discard` only holds a kept app until `healthy`,
    // `unchecked` or `rolled-back`.
    // Never while a journal was set aside: it may be the one that explains
    // the leftover.
    let (settled, _) = journal::list(state_dir);
    let newest = settled.last().filter(|j| j.record.settled());
    if let Some(newest) = newest.filter(|_| set_aside.is_empty()) {
        let mut retry = vec![&layout.staging];
        if ["healthy", "rolled-back", "unchecked"].contains(&newest.record.state.as_str()) {
            retry.push(&layout.discard);
        }
        for leftover in retry.into_iter().filter(|p| p.exists()) {
            deps.fs.remove_all(leftover).map_err(|e| {
                format!(
                    "{} is left over and could not be removed ({e}); resolve by hand",
                    leftover.display()
                )
            })?;
            notes.push(format!("removed the leftover {}", leftover.display()));
        }
    }
    restore::sweep_orphans(&settled);
    for leftover in [&layout.staging, &layout.discard] {
        if leftover.exists() {
            return Err(format!(
                "{} is left over and no install journal explains it; resolve by hand",
                leftover.display()
            ));
        }
    }
    if let Some(foreign) = layout.foreign.iter().find(|p| p.exists()) {
        return Err(format!(
            "{} is left from an install by scripts/install-macos-app.mjs that did not finish; run its recovery (an install or --rollback) first",
            foreign.display()
        ));
    }
    Ok(notes)
}

/// Lock, wait for the app to quit, recover. Returns the lock.
fn begin(
    layout: &Layout,
    state_dir: &Path,
    install_id: &str,
    what: &str,
    deps: &Deps,
    timing: &Timing,
) -> Result<lock::InstallerLock, Outcome> {
    let pid = deps.procs.own_pid();
    let held = lock::acquire(
        state_dir,
        install_id,
        what,
        pid,
        &iso(deps.clock.now_ms()),
        deps.procs,
    )
    .map_err(|e| {
        let exit = match e {
            LockError::Busy(_) => Exit::Busy,
            LockError::Io(_) => Exit::Failed,
        };
        Outcome::new(exit, e.message())
    })?;
    let mut exes = Vec::new();
    for bundle in [&layout.installed, &layout.previous] {
        if let Ok(info) = verify::read_info(bundle) {
            exes.push(layout.exe(bundle, &info.executable));
        }
    }
    health::wait_until_quit(&exes, deps, timing).map_err(|m| Outcome::new(Exit::AppRunning, m))?;
    recover_all(layout, state_dir, deps).map_err(|m| Outcome::new(Exit::NeedsHand, m))?;
    Ok(held)
}

fn step(
    journal: &mut Journal,
    deps: &Deps,
    state: &str,
    detail: serde_json::Value,
) -> Result<(), String> {
    journal
        .step(state, &iso(deps.clock.now_ms()), detail)
        .map_err(|e| format!("could not write the journal: {e}"))?;
    (deps.checkpoint)(state);
    Ok(())
}

/// Settle a journal as `failed` (nothing moved) after removing the staging
/// copy, and return the outcome.
fn fail_unmoved(
    journal: &mut Journal,
    layout: &Layout,
    deps: &Deps,
    exit: Exit,
    reason: String,
) -> Outcome {
    let _ = deps.fs.remove_all(&layout.staging);
    let _ = step(
        journal,
        deps,
        "failed",
        json!({ "reason": reason, "moved": false }),
    );
    finish(journal, deps);
    with_journal(Outcome::new(exit, reason), journal)
}

/// After a journal settles: a failed or rolled-back install's snapshot is
/// removed (the data it copied is still the live data), then old snapshot
/// folders and journals are pruned.
fn finish(journal: &Journal, deps: &Deps) {
    let Some(state_dir) = journal.path.parent().and_then(Path::parent) else {
        return;
    };
    let keep = journal
        .record
        .snapshot
        .as_ref()
        .and_then(|s| s.folder.clone());
    // The pre-update data is still the live data when nothing was launched
    // (`failed`, or rolled back before a start). After a go back from failed
    // starts the new version may have migrated the stores, so the snapshot
    // stays for U6 to restore from; the keep-3 prune ages it out.
    let state = journal.record.state.as_str();
    let unused =
        state == "failed" || (state == "rolled-back" && journal.record.attempts.is_empty());
    let mut keep = keep;
    if unused {
        if let Some(folder) = keep.take() {
            let _ = deps.fs.remove_all(&folder);
        }
    }
    snapshot::prune(
        &state_dir.join(snapshot::DIR),
        snapshot::KEEP,
        keep.as_deref(),
    );
    journal::prune(state_dir, 20);
}

/// Remove `path` and say how it went, for the journal.
fn removed(deps: &Deps, path: &Path) -> serde_json::Value {
    match deps.fs.remove_all(path) {
        Ok(()) => json!(true),
        Err(e) => json!(format!("could not remove {}: {e}", path.display())),
    }
}

fn with_journal(mut outcome: Outcome, journal: &Journal) -> Outcome {
    outcome.journal = Some(journal.path.clone());
    outcome
}

/// Install `args.staged` as the app at `args.app`.
pub(crate) fn install(args: &InstallArgs, deps: &Deps) -> Outcome {
    guard_test_paths(&args.app, &args.state_dir);
    let layout = Layout::for_app(&args.app);
    let dir = super::install_dir(&args.app);
    if let Err(message) = writable(&dir, deps.procs.own_pid()) {
        return Outcome::new(Exit::NotWritable, message);
    }
    let _lock = match begin(
        &layout,
        &args.state_dir,
        &args.install_id,
        "install",
        deps,
        &args.timing,
    ) {
        Ok(lock) => lock,
        Err(outcome) => return outcome,
    };

    let now = iso(deps.clock.now_ms());
    let mut record = Record::new(
        Kind::Install,
        &args.install_id,
        deps.procs.own_pid(),
        &args.app,
        &now,
    );
    record.staged = Some(args.staged.clone());
    record.expected_version = Some(args.expected_version.clone());
    record.expected_team = Some(args.team.label());
    let mut journal = match Journal::create(&args.state_dir, record) {
        Ok(j) => j,
        Err(e) => {
            return Outcome::new(
                Exit::Failed,
                format!("could not start the install journal: {e}"),
            )
        }
    };
    if let Err(m) = step(&mut journal, deps, "prepared", json!({})) {
        return with_journal(Outcome::new(Exit::Failed, m), &journal);
    }

    // The staged bundle: where it is, what it is, who signed it.
    let staged = match verify::check_location(&args.staged, &layout) {
        Ok(p) => p,
        Err(m) => return fail_unmoved(&mut journal, &layout, deps, Exit::Refused, m),
    };
    let info = match verify::check_bundle(
        &staged,
        deps.signer,
        Some(&args.team),
        Some(&args.expected_version),
    ) {
        Ok(info) => info,
        Err(m) => {
            let m = format!("the staged app was refused: {m}");
            return fail_unmoved(&mut journal, &layout, deps, Exit::Refused, m);
        }
    };
    let current = verify::read_info(&layout.installed).ok();
    if let Some(current) = &current {
        if let Err(m) =
            verify::check_downgrade(&current.version, &info.version, args.allow_downgrade)
        {
            return fail_unmoved(&mut journal, &layout, deps, Exit::Refused, m);
        }
    }
    // Kept as previous only when it verifies now (signature and bundle id;
    // an older build may have no team).
    let keep_current = layout.installed.exists()
        && verify::check_bundle(&layout.installed, deps.signer, None, None).is_ok();

    // The copy beside the app, verified again, byte for byte.
    if let Err(m) = deps.fs.copy_bundle(&staged, &layout.staging) {
        return fail_unmoved(&mut journal, &layout, deps, Exit::Failed, m);
    }
    deps.fs.clear_quarantine(&layout.staging);
    let copied = verify::check_bundle(
        &layout.staging,
        deps.signer,
        Some(&args.team),
        Some(&args.expected_version),
    )
    .and_then(|_| {
        let (a, b) = (
            layout.exe(&staged, &info.executable),
            layout.exe(&layout.staging, &info.executable),
        );
        match verify::same_bytes(&a, &b)? {
            true => Ok(()),
            false => Err("the copied executable differs from the staged one".into()),
        }
    });
    if let Err(m) = copied {
        let m = format!("the copy beside the app did not verify: {m}");
        return fail_unmoved(&mut journal, &layout, deps, Exit::Refused, m);
    }
    let Some(new_ino) = swap::ino(&layout.staging) else {
        return fail_unmoved(
            &mut journal,
            &layout,
            deps,
            Exit::Failed,
            "the copy beside the app vanished".into(),
        );
    };
    journal.record.from_version = current.as_ref().map(|c| c.version.clone());
    journal.record.to_version = Some(info.version.clone());
    journal.record.keep_current = Some(keep_current);
    journal.record.new_ino = Some(new_ino);
    if let Err(m) = step(
        &mut journal,
        deps,
        "staged-verified",
        json!({ "staging": layout.staging }),
    ) {
        return fail_unmoved(&mut journal, &layout, deps, Exit::Failed, m);
    }

    // The Sessions snapshot, before anything moves.
    let from = current
        .as_ref()
        .map(|c| c.version.as_str())
        .unwrap_or("none");
    let folder = args.state_dir.join(snapshot::DIR).join(format!(
        "{}-from-{}",
        journal::safe_id(&args.install_id),
        journal::safe_id(from)
    ));
    let snap = match &args.snapshot_source {
        None => Ok(Snapshot {
            skipped: true,
            folder: None,
            files: Vec::new(),
        }),
        Some((source, explicit)) => snapshot::sources(source, *explicit)
            .and_then(|sources| snapshot::take(&sources, &folder)),
    };
    let snap = match snap {
        Ok(s) => s,
        Err(m) => {
            let _ = deps.fs.remove_all(&folder);
            let m = format!("the Sessions snapshot failed, so nothing was swapped: {m}");
            return fail_unmoved(&mut journal, &layout, deps, Exit::SnapshotFailed, m);
        }
    };
    journal.record.snapshot = Some(snap.clone());
    if let Err(m) = step(
        &mut journal,
        deps,
        "snapshot-written",
        json!({ "files": snap.files.len(), "skipped": snap.skipped }),
    ) {
        return fail_unmoved(&mut journal, &layout, deps, Exit::Failed, m);
    }

    // Opened meanwhile? Then nothing moves.
    let exe = layout.exe(&layout.installed, &info.executable);
    let mut exes = vec![exe.clone()];
    if let Some(c) = &current {
        exes.push(layout.exe(&layout.installed, &c.executable));
    }
    match health::running(&exes, deps) {
        Ok(list) if list.is_empty() => {}
        Ok(list) => {
            let m = format!(
                "Hollow was opened during the install (pid {}), so nothing was swapped",
                list[0].pid
            );
            return fail_unmoved(&mut journal, &layout, deps, Exit::AppRunning, m);
        }
        Err(m) => return fail_unmoved(&mut journal, &layout, deps, Exit::Failed, m),
    }

    // The signature once more, right before the swap.
    if let Err(m) = deps.signer.verify(&layout.staging) {
        let m = format!("the copy beside the app no longer verifies: {m}");
        return fail_unmoved(&mut journal, &layout, deps, Exit::Refused, m);
    }

    // The swap.
    let plan = match swap::plan_install(&layout, new_ino, keep_current) {
        Ok(plan) => plan,
        Err(m) => return fail_unmoved(&mut journal, &layout, deps, Exit::NeedsHand, m),
    };
    journal.record.plan = Some(plan);
    journal.record.moves_done = 0;
    if let Err(e) = journal.save() {
        return fail_unmoved(
            &mut journal,
            &layout,
            deps,
            Exit::Failed,
            format!("could not write the journal: {e}"),
        );
    }
    if let Err(m) = swap::run_moves(deps.fs, &mut journal) {
        return undo_after(&mut journal, &layout, deps, m);
    }
    let swapped_at = deps.clock.now_ms();
    if let Err(m) = step(
        &mut journal,
        deps,
        "promoted",
        json!({ "swappedAt": iso(swapped_at) }),
    ) {
        return undo_after(&mut journal, &layout, deps, m);
    }

    if !args.relaunch {
        let discard = removed(deps, &layout.discard);
        let _ = step(
            &mut journal,
            deps,
            "unchecked",
            json!({ "reason": "--no-relaunch: the new app was not opened", "discardRemoved": discard }),
        );
        finish(&journal, deps);
        return with_journal(
            Outcome::new(
                Exit::Ok,
                format!("installed Hollow {} (not reopened)", info.version),
            ),
            &journal,
        );
    }

    // The reopen check: two tries, then go back.
    for n in 1..=ATTEMPTS {
        let launched_at = iso(deps.clock.now_ms());
        let attempted = health::attempt(&layout.installed, &exe, deps, &args.timing);
        let start = attempted.start.clone();
        journal.record.attempts.push(Attempt {
            attempt: n,
            launched_at,
            outcome: start.name().into(),
            pid: start.pid(),
            detail: start.detail(&args.timing),
        });
        if let Err(m) = step(
            &mut journal,
            deps,
            "started",
            json!({ "attempt": n, "outcome": start.name() }),
        ) {
            // The swap stands; the next run's recovery verifies it.
            return with_journal(Outcome::new(Exit::NeedsHand, m), &journal);
        }
        if let Start::Healthy { pid } = start {
            // Healthy is recorded first, so a crash during the removal
            // leaves a settled journal (the next run retries the removal).
            let _ = step(&mut journal, deps, "healthy", json!({ "pid": pid }));
            let discard = removed(deps, &layout.discard);
            if let Some(last) = journal.record.steps.last_mut() {
                last.detail.insert("discardRemoved".into(), discard);
            }
            let _ = journal.save();
            finish(&journal, deps);
            let from = current
                .map(|c| c.version)
                .unwrap_or_else(|| "nothing".into());
            return with_journal(
                Outcome::new(
                    Exit::Ok,
                    format!("installed Hollow {} (was {from}); it started", info.version),
                ),
                &journal,
            );
        }
        // Whatever is left of this try must be gone before the next one
        // (or before going back).
        if let Err(m) = health::stop(
            &exe,
            &attempted.ours,
            attempted.launched_ms,
            deps,
            &args.timing,
        ) {
            let reason =
                format!("the new version did not start, and going back is not safe yet: {m}");
            write_failure(&args.state_dir, &journal, deps, &reason, false);
            let _ = step(&mut journal, deps, "started", json!({ "stuck": reason }));
            return with_journal(Outcome::new(Exit::NeedsHand, reason), &journal);
        }
    }
    go_back_after_failed_starts(&mut journal, &layout, deps, &args.state_dir, &info.version)
}

/// Undo the moves after a failure during the swap.
fn undo_after(journal: &mut Journal, layout: &Layout, deps: &Deps, reason: String) -> Outcome {
    journal.record.state = "rolling-back".into();
    let _ = journal.save();
    match swap::undo_moves(deps.fs, journal) {
        Ok(()) => {
            let _ = deps.fs.remove_all(&layout.staging);
            let _ = step(
                journal,
                deps,
                "failed",
                json!({ "reason": reason, "moved": true, "undone": true }),
            );
            finish(journal, deps);
            with_journal(
                Outcome::new(Exit::Failed, format!("{reason}; every move was undone")),
                journal,
            )
        }
        Err(undo) => with_journal(
            Outcome::new(
                Exit::NeedsHand,
                format!("{reason}; putting the apps back failed too: {undo}"),
            ),
            journal,
        ),
    }
}

fn go_back_after_failed_starts(
    journal: &mut Journal,
    layout: &Layout,
    deps: &Deps,
    state_dir: &Path,
    version: &str,
) -> Outcome {
    let reason = format!(
        "Hollow {version} did not start in {ATTEMPTS} tries ({})",
        journal
            .record
            .attempts
            .iter()
            .map(|a| a.detail.clone())
            .collect::<Vec<_>>()
            .join("; ")
    );
    journal.record.state = "rolling-back".into();
    let _ = journal.save();
    (deps.checkpoint)("rolling-back");
    if let Err(undo) = swap::undo_moves(deps.fs, journal) {
        let reason = format!("{reason}; going back failed: {undo}");
        write_failure(state_dir, journal, deps, &reason, false);
        return with_journal(Outcome::new(Exit::NeedsHand, reason), journal);
    }
    let _ = deps.fs.remove_all(&layout.staging);
    // Written before the previous app opens, so it can say why.
    write_failure(state_dir, journal, deps, &reason, true);
    let relaunched = deps.launcher.launch(&layout.installed);
    let _ = step(
        journal,
        deps,
        "rolled-back",
        json!({ "reason": reason, "reopened": relaunched.as_ref().map(|_| "ok".to_string()).unwrap_or_else(|e| e.clone()) }),
    );
    finish(journal, deps);
    with_journal(
        Outcome::new(
            Exit::WentBack,
            format!("{reason}; the previous version is back"),
        ),
        journal,
    )
}

/// A go back failed part way: undo every move (with a restore, the restored
/// databases go back to their temporary names and the live ones come back),
/// remove the restore copies if there are any, settle `failed`.
pub(super) fn undo_go_back(
    journal: &mut Journal,
    deps: &Deps,
    prepared: Option<&restore::Prepared>,
    exit: Exit,
    reason: String,
) -> Outcome {
    journal.record.state = "rolling-back".into();
    let _ = journal.save();
    match swap::undo_moves(deps.fs, journal) {
        Ok(()) => {
            if let Some(prepared) = prepared {
                restore::discard(prepared);
            }
            let _ = step(
                journal,
                deps,
                "failed",
                json!({ "reason": reason, "moved": true, "undone": true }),
            );
            with_journal(
                Outcome::new(exit, format!("{reason}; every move was undone")),
                journal,
            )
        }
        Err(undo) => with_journal(
            Outcome::new(
                Exit::NeedsHand,
                format!("{reason}; putting the files back failed too: {undo}"),
            ),
            journal,
        ),
    }
}

/// `<state>/last-install-failure.json`, for Hollow to read at its next start
/// (U5/U6 read it; schema in build-and-ship.md).
pub(crate) fn write_failure(
    state_dir: &Path,
    journal: &Journal,
    deps: &Deps,
    reason: &str,
    went_back: bool,
) {
    let r = &journal.record;
    let restored = verify::read_info(&r.app).ok().map(|i| i.version);
    let body = json!({
        "schema": 1,
        "installId": r.id,
        "at": iso(deps.clock.now_ms()),
        "attemptedVersion": r.to_version,
        "restoredVersion": if went_back { restored } else { None },
        "wentBack": went_back,
        "reason": reason,
        "attempts": r.attempts,
        "journal": journal.path,
    });
    let text = serde_json::to_string_pretty(&body).unwrap_or_default();
    let _ = journal::write_durably(&state_dir.join(FAILURE_FILE), &format!("{text}\n"));
}

/// Swap `.previous` and the installed app (the `.mjs`'s `--rollback`),
/// journaled; then reopen and check once (no automatic reversal). With
/// `restore`, the Sessions databases are put back from that snapshot first,
/// in the same plan ([`restore`]); any failure there undoes every move.
pub(crate) fn go_back(args: &GoBackArgs, deps: &Deps) -> Outcome {
    guard_test_paths(&args.app, &args.state_dir);
    let layout = Layout::for_app(&args.app);
    if let Err(message) = writable(&super::install_dir(&args.app), deps.procs.own_pid()) {
        return Outcome::new(Exit::NotWritable, message);
    }
    let _lock = match begin(
        &layout,
        &args.state_dir,
        &args.install_id,
        "go back",
        deps,
        &args.timing,
    ) {
        Ok(lock) => lock,
        Err(outcome) => return outcome,
    };
    let outcome = go_back_locked(args, &layout, deps);
    reopen_after_failure(args, &layout, deps, outcome)
}

/// Hollow quit for the go back, so a go back that changed nothing (it
/// failed before any move, or every move was undone) opens the installed
/// app again, which is still the version it was (with `--relaunch`). Not
/// after `needsHand`: what is where then needs a person.
fn reopen_after_failure(
    args: &GoBackArgs,
    layout: &Layout,
    deps: &Deps,
    mut outcome: Outcome,
) -> Outcome {
    let unchanged = matches!(
        outcome.exit,
        Exit::Failed | Exit::RestoreFailed | Exit::Refused | Exit::NothingToGoBack
    );
    if !args.relaunch || !unchanged || !layout.installed.exists() {
        return outcome;
    }
    match deps.launcher.launch(&layout.installed) {
        Ok(()) => outcome.message.push_str("; Hollow was opened again"),
        Err(e) => outcome
            .message
            .push_str(&format!("; opening Hollow again failed: {e}")),
    }
    outcome
}

/// The go back once the lock is held and Hollow has quit.
fn go_back_locked(args: &GoBackArgs, layout: &Layout, deps: &Deps) -> Outcome {
    if !layout.previous.exists() {
        return Outcome::new(
            Exit::NothingToGoBack,
            format!(
                "there is no previous Hollow to go back to ({})",
                layout.previous.display()
            ),
        );
    }
    if !layout.installed.exists() {
        return Outcome::new(
            Exit::NeedsHand,
            format!("{} is missing", layout.installed.display()),
        );
    }
    let previous = match verify::check_bundle(&layout.previous, deps.signer, None, None) {
        Ok(info) => info,
        Err(m) => {
            return Outcome::new(
                Exit::Refused,
                format!("the previous app does not verify, so it was not put back: {m}"),
            )
        }
    };
    let current = verify::read_info(&layout.installed).ok();
    let now = iso(deps.clock.now_ms());
    let mut record = Record::new(
        Kind::GoBack,
        &args.install_id,
        deps.procs.own_pid(),
        &args.app,
        &now,
    );
    record.from_version = current.as_ref().map(|c| c.version.clone());
    record.to_version = Some(previous.version.clone());
    let mut journal = match Journal::create(&args.state_dir, record) {
        Ok(j) => j,
        Err(e) => return Outcome::new(Exit::Failed, format!("could not start the journal: {e}")),
    };
    if let Err(m) = step(&mut journal, deps, "prepared", json!({})) {
        return with_journal(Outcome::new(Exit::Failed, m), &journal);
    }
    let swap_plan = match swap::plan_go_back(layout) {
        Ok(p) => p,
        Err(m) => {
            let _ = step(
                &mut journal,
                deps,
                "failed",
                json!({ "reason": m, "moved": false }),
            );
            return with_journal(Outcome::new(Exit::NothingToGoBack, m), &journal);
        }
    };
    // The Sessions restore (row U6): checked and laid beside the live
    // databases before anything moves; its renames come first in the plan.
    let prepared = match &args.restore {
        None => None,
        Some(folder) => {
            match restore::prepare(&args.state_dir, folder, &args.install_id) {
                Ok(p) => Some(p),
                Err(m) => {
                    let m = format!("the data from before the update was not restored, so nothing was changed: {m}");
                    let _ = step(
                        &mut journal,
                        deps,
                        "failed",
                        json!({ "reason": m, "moved": false }),
                    );
                    return with_journal(Outcome::new(Exit::RestoreFailed, m), &journal);
                }
            }
        }
    };
    let mut plan = Vec::new();
    if let Some(p) = &prepared {
        plan.extend(p.moves.iter().cloned());
        journal.record.restore = Some(p.record.clone());
    }
    let restore_moves = plan.len();
    plan.extend(swap_plan);
    journal.record.plan = Some(plan);
    if let Err(e) = journal.save() {
        if let Some(p) = &prepared {
            restore::discard(p);
        }
        let m = format!("could not write the journal: {e}");
        return with_journal(Outcome::new(Exit::Failed, m), &journal);
    }
    if let Some(p) = &prepared {
        let restored = swap::run_moves_until(deps.fs, &mut journal, restore_moves)
            .and_then(|()| restore::verify_restored(p));
        if let Err(m) = restored {
            let m = format!("the data from before the update could not be restored: {m}");
            return undo_go_back(&mut journal, deps, Some(p), Exit::RestoreFailed, m);
        }
        if let Err(m) = step(
            &mut journal,
            deps,
            "data-restored",
            json!({ "databases": p.record.databases.len() }),
        ) {
            return undo_go_back(&mut journal, deps, Some(p), Exit::RestoreFailed, m);
        }
    }
    if let Err(m) = swap::run_moves(deps.fs, &mut journal) {
        // With a restore, or when any move already happened, every move is
        // undone first: `Exit::Failed` reopens the installed app, so it must
        // be the one that was installed (U6 follow-up: a failure part way
        // with no restore used to reopen whatever the moves had left).
        if prepared.is_some() || journal.record.moves_done > 0 {
            return undo_go_back(&mut journal, deps, prepared.as_ref(), Exit::Failed, m);
        }
        let _ = step(
            &mut journal,
            deps,
            "failed",
            json!({ "reason": m, "moved": false }),
        );
        return with_journal(Outcome::new(Exit::Failed, m), &journal);
    }
    let _ = step(&mut journal, deps, "promoted", json!({}));
    let back_to = previous.version.clone();
    if !args.relaunch {
        let _ = step(&mut journal, deps, "unchecked", json!({}));
        return with_journal(
            Outcome::new(
                Exit::Ok,
                format!("went back to Hollow {back_to} (not reopened)"),
            ),
            &journal,
        );
    }
    let exe = layout.exe(&layout.installed, &previous.executable);
    let start = health::attempt(&layout.installed, &exe, deps, &args.timing).start;
    journal.record.attempts.push(Attempt {
        attempt: 1,
        launched_at: iso(deps.clock.now_ms()),
        outcome: start.name().into(),
        pid: start.pid(),
        detail: start.detail(&args.timing),
    });
    let _ = step(
        &mut journal,
        deps,
        "started",
        json!({ "attempt": 1, "outcome": start.name() }),
    );
    if matches!(start, Start::Healthy { .. }) {
        let _ = step(&mut journal, deps, "healthy", json!({}));
        with_journal(
            Outcome::new(
                Exit::Ok,
                format!("went back to Hollow {back_to}; it started"),
            ),
            &journal,
        )
    } else {
        let detail = start.detail(&args.timing);
        let _ = step(
            &mut journal,
            deps,
            "start-failed",
            json!({ "reason": detail }),
        );
        with_journal(
            Outcome::new(
                Exit::StartFailed,
                format!("went back to Hollow {back_to}, but it did not start: {detail}"),
            ),
            &journal,
        )
    }
}
