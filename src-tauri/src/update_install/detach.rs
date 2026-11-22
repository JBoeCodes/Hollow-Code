//! Starting `hollow install-update` so it outlives the app that asks for it
//! (`update_client`'s Restart to update calls this). The binary is the staged new
//! bundle's own `Contents/MacOS/hollow`; the child gets its own session and
//! process group (`setsid`, so a quit's group signal or a closed terminal
//! never reaches it), stdin closed, output appended to `log`, and the
//! environment cut to [`ENV_ALLOWED`] (so no `HOLLOW_*` variable). The log is created 0600. It is never waited for.

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

/// The only variables the installer inherits: no `HOLLOW_*` (a run's finish
/// sweep matches on `HOLLOW_RUN_TAG`; `HOLLOW_APP_DATA_DIR` would point the
/// reopened app at other data) and nothing else a parent happened to set.
pub(crate) const ENV_ALLOWED: &[&str] = &["PATH", "HOME", "TMPDIR", "LANG"];

/// The command, built but not started (tests read it).
pub(crate) fn detached_command(
    binary: &Path,
    args: &[OsString],
    log: &Path,
    env: impl Iterator<Item = (OsString, OsString)>,
) -> Result<Command, String> {
    use std::os::unix::fs::OpenOptionsExt;
    let out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(log)
        .map_err(|e| format!("could not open the installer log {}: {e}", log.display()))?;
    let err = out.try_clone().map_err(|e| e.to_string())?;
    let mut cmd = Command::new(binary);
    cmd.arg("install-update").args(args).env_clear();
    cmd.envs(env.filter(|(k, _)| ENV_ALLOWED.contains(&k.to_string_lossy().as_ref())));
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: setsid is async-signal-safe and touches no memory of ours.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    Ok(cmd)
}

/// Start the installer detached and return its `Child`, so the caller can
/// watch it briefly (`try_wait`) and stop it if Hollow decides not to quit.
/// Dropping the `Child` neither kills nor reaps it.
pub(crate) fn spawn_detached(
    binary: &Path,
    args: &[OsString],
    log: &Path,
) -> Result<std::process::Child, String> {
    let mut cmd = detached_command(binary, args, log, std::env::vars_os())?;
    cmd.spawn()
        .map_err(|e| format!("could not start the installer: {e}"))
}
