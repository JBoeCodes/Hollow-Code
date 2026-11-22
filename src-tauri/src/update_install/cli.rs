//! The `hollow install-update` command line: parse, run with the real
//! effects, print a sentence on stderr and one JSON result line on stdout
//! (`{"ok","exit","code","message","journal"}`), exit with [`Exit`].

use super::effects::{Codesign, Deps, MacProcs, OpenLauncher, RealFs, SystemClock};
use super::health::Timing;
use super::run::{self, GoBackArgs, InstallArgs};
use super::verify::Team;
use super::{journal, lock, verify, Exit, Layout, Outcome, DEFAULT_APP, DEFAULT_TEAM};
use crate::launch::elog;
use std::io::Write as _;
use std::path::PathBuf;

pub(crate) const USAGE: &str = "\
Usage:
  hollow install-update --staged <new Hollow.app> --expected-version <v> [options]
  hollow install-update --go-back [--restore-snapshot <dir>] [--app <path>] [--journal-dir <dir>] [--no-relaunch]
  hollow install-update --status [--app <path>] [--journal-dir <dir>]

Install options:
  --expected-team <id>       signing team (default TEAM000000, or HOLLOW_UPDATE_TEAM; `-` = ad hoc)
  --app <path>               the installed app (default /Applications/Hollow.app)
  --journal-dir <dir>        lock, journals, snapshots, failure file
                             (default ~/Library/Application Support/com.hollow.app/installer)
  --snapshot-source <path>   a Sessions database, or a folder of them (default the app's Sessions)
  --no-snapshot              skip the Sessions snapshot
  --relaunch | --no-relaunch reopen and check the new app (default --relaunch)
  --allow-downgrade          install an older version (channel switch, go back)
  --install-id <id>          name the journal (default: date, time, random)
  --quit-timeout-secs <n>    wait for Hollow to quit (default 60)
  --start-timeout-secs <n>   wait for the new app to appear (default 60)
  --settle-secs <n>          it must still run this long after (default 20)

Go back options:
  --restore-snapshot <dir>   first put the Sessions databases back from this install
                             snapshot (<journal dir>/snapshots/...); the live ones are kept
                             as <file>.before-go-back

Exit status: 0 ok, 1 failed (nothing changed), 2 usage, 3 another installer runs,
4 refused, 5 install folder not writable, 6 snapshot failed, 7 Hollow still running,
8 new version did not start twice and the previous one is back, 9 needs a person,
10 nothing to go back to, 11 went back but the app did not start,
12 the snapshot could not be restored (nothing changed).";

/// The longest wait a flag may ask for (an hour).
pub(crate) const MAX_WAIT_SECS: u64 = 3600;

#[derive(Debug)]
pub(crate) enum Mode {
    Install(InstallArgs),
    GoBack(GoBackArgs),
    Status { app: PathBuf, state_dir: PathBuf },
    Help,
}

/// Parse the arguments after `install-update`. `env_team` is
/// `HOLLOW_UPDATE_TEAM`; `default_state` the default journal folder.
pub(crate) fn parse(
    args: &[String],
    env_team: Option<String>,
    default_state: Option<PathBuf>,
    default_sessions: Option<PathBuf>,
    now_ms: u64,
) -> Result<Mode, String> {
    let mut staged = None;
    let mut version = None;
    let mut team = None;
    let mut app = PathBuf::from(DEFAULT_APP);
    let mut state_dir = default_state;
    let mut snapshot_source: Option<PathBuf> = None;
    let mut no_snapshot = false;
    let mut relaunch = true;
    let mut allow_downgrade = false;
    let mut install_id = None;
    let mut restore = None;
    let mut timing = Timing::default();
    let (mut go_back, mut status) = (false, false);

    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |name: &str| -> Result<String, String> {
            rest.next()
                .filter(|v| !v.starts_with("--"))
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        let secs = |text: String, name: &str| -> Result<u64, String> {
            text.parse::<u64>()
                .ok()
                .filter(|s| *s <= MAX_WAIT_SECS)
                .and_then(|s| s.checked_mul(1000))
                .ok_or_else(|| format!("{name} takes whole seconds, at most {MAX_WAIT_SECS}"))
        };
        match arg.as_str() {
            "--help" | "-h" => return Ok(Mode::Help),
            "--staged" => staged = Some(PathBuf::from(value(arg)?)),
            "--expected-version" => version = Some(value(arg)?),
            "--expected-team" => team = Some(value(arg)?),
            "--app" => app = PathBuf::from(value(arg)?),
            "--journal-dir" => state_dir = Some(PathBuf::from(value(arg)?)),
            "--snapshot-source" => snapshot_source = Some(PathBuf::from(value(arg)?)),
            "--no-snapshot" => no_snapshot = true,
            "--relaunch" => relaunch = true,
            "--no-relaunch" => relaunch = false,
            "--allow-downgrade" => allow_downgrade = true,
            "--install-id" => install_id = Some(journal::safe_id(&value(arg)?)),
            "--quit-timeout-secs" => timing.quit_ms = secs(value(arg)?, arg)?,
            "--start-timeout-secs" => timing.appear_ms = secs(value(arg)?, arg)?,
            "--settle-secs" => timing.settle_ms = secs(value(arg)?, arg)?,
            "--go-back" => go_back = true,
            "--restore-snapshot" => restore = Some(PathBuf::from(value(arg)?)),
            "--status" => status = true,
            other => return Err(format!("unknown option `{other}`")),
        }
    }
    if !app.is_absolute() || app.extension().is_none_or(|e| e != "app") {
        return Err(format!(
            "--app must be an absolute path to a .app, not {}",
            app.display()
        ));
    }
    let state_dir = state_dir.ok_or("no journal folder: pass --journal-dir")?;
    if !state_dir.is_absolute() {
        return Err("--journal-dir must be an absolute path".into());
    }
    let install_id = install_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| run::new_install_id(now_ms));
    if [go_back, status, staged.is_some()]
        .iter()
        .filter(|x| **x)
        .count()
        > 1
    {
        return Err("choose one of --staged, --go-back and --status".into());
    }
    if restore.is_some() && !go_back {
        return Err("--restore-snapshot goes with --go-back".into());
    }
    if restore.as_ref().is_some_and(|r| !r.is_absolute()) {
        return Err("--restore-snapshot must be an absolute path".into());
    }
    if status {
        return Ok(Mode::Status { app, state_dir });
    }
    if go_back {
        return Ok(Mode::GoBack(GoBackArgs {
            app,
            state_dir,
            relaunch,
            install_id,
            timing,
            restore,
        }));
    }
    let staged = staged.ok_or("--staged is required (or --go-back, --status)")?;
    let expected_version = version.ok_or("--expected-version is required with --staged")?;
    let team = Team::parse(
        &team
            .or(env_team)
            .unwrap_or_else(|| DEFAULT_TEAM.to_string()),
    )?;
    let snapshot_source = if no_snapshot {
        None
    } else {
        match snapshot_source {
            Some(path) => Some((path, true)),
            None => Some((
                default_sessions
                    .ok_or("no Sessions folder: pass --snapshot-source or --no-snapshot")?,
                false,
            )),
        }
    };
    Ok(Mode::Install(InstallArgs {
        staged,
        expected_version,
        team,
        app,
        state_dir,
        snapshot_source,
        relaunch,
        allow_downgrade,
        install_id,
        timing,
    }))
}

/// `hollow install-update ...`: returns the exit status.
pub(crate) fn main(args: Vec<String>) -> i32 {
    let clock = SystemClock;
    let mode = parse(
        &args,
        std::env::var("HOLLOW_UPDATE_TEAM")
            .ok()
            .filter(|t| !t.is_empty()),
        super::default_state_dir(),
        super::default_sessions_dir(),
        super::effects::Clock::now_ms(&clock),
    );
    let mode = match mode {
        Ok(mode) => mode,
        Err(why) => {
            elog(&format!("hollow install-update: {why}\n\n{USAGE}"));
            return Exit::Usage.code();
        }
    };
    let deps = Deps {
        fs: &RealFs,
        signer: &Codesign,
        procs: &MacProcs,
        launcher: &OpenLauncher,
        clock: &clock,
        checkpoint: &|_| {},
    };
    let outcome = match mode {
        Mode::Help => {
            say(USAGE);
            return 0;
        }
        Mode::Status { app, state_dir } => {
            say(&status(&app, &state_dir, &deps).to_string());
            return 0;
        }
        Mode::Install(args) => run::install(&args, &deps),
        Mode::GoBack(args) => run::go_back(&args, &deps),
    };
    report(&outcome);
    outcome.exit.code()
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{}", text.trim_end());
}

/// The result line on stdout, the sentence on stderr.
pub(crate) fn result_line(outcome: &Outcome) -> serde_json::Value {
    serde_json::json!({
        "ok": outcome.exit == Exit::Ok,
        "exit": outcome.exit.code(),
        "code": outcome.exit.name(),
        "message": outcome.message,
        "journal": outcome.journal,
    })
}

fn report(outcome: &Outcome) {
    elog(&format!("[install-update] {}", outcome.message));
    say(&result_line(outcome).to_string());
}

/// What is installed and kept, who holds the lock, unsettled journals and
/// the last failure. Reads only.
pub(crate) fn status(
    app: &std::path::Path,
    state_dir: &std::path::Path,
    deps: &Deps,
) -> serde_json::Value {
    let layout = Layout::for_app(app);
    let describe = |bundle: &std::path::Path| {
        verify::read_info(bundle).ok().map(|info| {
            serde_json::json!({
                "path": bundle,
                "version": info.version,
                "bundleId": info.bundle_id,
                "team": deps.signer.team(bundle).ok().flatten(),
            })
        })
    };
    let (journals, unreadable) = journal::list(state_dir);
    let unsettled: Vec<_> = journals
        .iter()
        .filter(|j| !j.record.settled())
        .map(|j| serde_json::json!({ "id": j.record.id, "state": j.record.state, "journal": j.path }))
        .collect();
    let failure = std::fs::read_to_string(state_dir.join(run::FAILURE_FILE))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
    serde_json::json!({
        "installed": describe(&layout.installed),
        "previous": describe(&layout.previous),
        "lock": lock::holder(state_dir, deps.procs).map(|(who, live)| serde_json::json!({ "holder": who, "live": live })),
        "unsettled": unsettled,
        "unreadableJournals": unreadable.iter().map(|(p, _)| p).collect::<Vec<_>>(),
        "lastFailure": failure,
    })
}
