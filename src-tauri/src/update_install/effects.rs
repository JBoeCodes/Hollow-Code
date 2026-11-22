//! The installer's effects on the machine, behind small traits so tests use
//! fakes and temp folders: renames and copies ([`Fs`]), signatures
//! ([`Signer`]), processes ([`Procs`]), reopening the app ([`Launcher`]) and
//! time ([`Clock`]). The real ones are at the bottom; nothing in a test
//! reaches `codesign` on `/Applications`, `open` or a real sleep.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

pub(crate) trait Fs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Exchange two existing paths in one step (`renamex_np` with
    /// `RENAME_SWAP`): neither path is ever absent.
    fn swap(&self, a: &Path, b: &Path) -> io::Result<()>;
    fn remove_all(&self, path: &Path) -> io::Result<()>;
    /// Copy a bundle (clones on APFS), keeping signatures and attributes.
    fn copy_bundle(&self, from: &Path, to: &Path) -> Result<(), String>;
    /// Drop the download's `com.apple.quarantine` flag from a copied bundle
    /// (`xattr -d -r -s`), best effort: `ditto --noqtn` was seen keeping it on
    /// macOS 27 (review round 2 probe).
    fn clear_quarantine(&self, path: &Path);
}

pub(crate) trait Signer {
    /// `codesign --verify --deep --strict`; `Err` carries codesign's reason.
    fn verify(&self, bundle: &Path) -> Result<(), String>;
    /// The signature's TeamIdentifier; `None` for an ad hoc signature.
    fn team(&self, bundle: &Path) -> Result<Option<String>, String>;
    /// `codesign --verify --deep --strict -R <requirement>`: the signature
    /// chains to Apple and is a Developer ID for that team
    /// ([`team_requirement`]). Only asked for a real team id.
    fn verify_requirement(&self, bundle: &Path, requirement: &str) -> Result<(), String>;
}

/// One running app process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppProc {
    pub(crate) pid: i32,
    /// When it started, milliseconds since the epoch (0 when unknown).
    pub(crate) started_ms: u64,
}

pub(crate) trait Procs {
    /// The app processes running `exe` itself (no subcommand such as
    /// `team-mcp` or this `install-update`), never this process.
    fn app_processes(&self, exe: &Path) -> Result<Vec<AppProc>, String>;
    fn alive(&self, pid: i32) -> Option<bool>;
    /// A process start stamp, compared only for equality (pid reuse).
    fn started_micros(&self, pid: i32) -> Option<u64>;
    /// A normal TERM, never KILL.
    fn terminate(&self, pid: i32);
    fn own_pid(&self) -> i32;
}

pub(crate) trait Launcher {
    fn launch(&self, app: &Path) -> Result<(), String>;
}

pub(crate) trait Clock {
    fn now_ms(&self) -> u64;
    fn sleep(&self, duration: Duration);
}

/// Everything one run uses. `checkpoint` is called with each journal state
/// right after it is durably written (tests stop a run there to act out a
/// crash; the real one does nothing).
pub(crate) struct Deps<'a> {
    pub(crate) fs: &'a dyn Fs,
    pub(crate) signer: &'a dyn Signer,
    pub(crate) procs: &'a dyn Procs,
    pub(crate) launcher: &'a dyn Launcher,
    pub(crate) clock: &'a dyn Clock,
    pub(crate) checkpoint: &'a dyn Fn(&str),
}

// ------------------------------------------------------------------ real

pub(crate) struct RealFs;

impl Fs for RealFs {
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }

    fn swap(&self, a: &Path, b: &Path) -> io::Result<()> {
        #[cfg(target_os = "macos")]
        {
            use std::ffi::CString;
            use std::os::unix::ffi::OsStrExt;
            let a = CString::new(a.as_os_str().as_bytes())?;
            let b = CString::new(b.as_os_str().as_bytes())?;
            // SAFETY: two valid NUL-terminated paths; no memory is shared.
            let rc = unsafe { libc::renamex_np(a.as_ptr(), b.as_ptr(), libc::RENAME_SWAP) };
            if rc == 0 {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (a, b);
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "atomic swap needs macOS",
            ))
        }
    }

    fn remove_all(&self, path: &Path) -> io::Result<()> {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
            Ok(_) => std::fs::remove_file(path),
        }
    }

    fn copy_bundle(&self, from: &Path, to: &Path) -> Result<(), String> {
        // An older ditto without `--clone` gets a plain copy.
        for clone in [true, false] {
            let _ = self.remove_all(to);
            let out = Command::new("/usr/bin/ditto")
                .args(ditto_args(clone))
                .arg(from)
                .arg(to)
                .stdin(Stdio::null())
                .output()
                .map_err(|e| format!("could not run ditto: {e}"))?;
            if out.status.success() {
                return Ok(());
            }
            if !clone {
                return Err(format!(
                    "ditto could not copy {}: {}",
                    from.display(),
                    String::from_utf8_lossy(&out.stderr).trim()
                ));
            }
        }
        Err(format!("ditto could not copy {}", from.display()))
    }

    fn clear_quarantine(&self, path: &Path) {
        // Best effort: a copy with no quarantine flag makes xattr exit 1.
        let _ = Command::new("/usr/bin/xattr")
            .args(xattr_args())
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// `xattr`'s options for clearing quarantine: delete, recursively, acting on
/// a symlink itself (`-s`) so a link never leads the walk out of the bundle.
pub(crate) fn xattr_args() -> [&'static str; 4] {
    ["-d", "-r", "-s", "com.apple.quarantine"]
}

/// `ditto`'s options, as the `.mjs` passes them: resource forks, extended
/// attributes (a script's signature lives in them) and ACLs kept
/// explicitly; `--noqtn` asks ditto to leave the quarantine attribute out
/// (it did not always, so [`Fs::clear_quarantine`] follows); dropping it is
/// safe because the staged bundle's signature and team were verified first
/// and the copy is verified again; `--clone` makes an APFS clone.
pub(crate) fn ditto_args(clone: bool) -> Vec<&'static str> {
    let mut args = vec!["--rsrc", "--extattr", "--acl", "--noqtn"];
    if clone {
        args.push("--clone");
    }
    args
}

pub(crate) struct Codesign;

fn codesign(args: &[&str], bundle: &Path) -> Result<String, String> {
    let out = Command::new("/usr/bin/codesign")
        .args(args)
        .arg(bundle)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run codesign: {e}"))?;
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        Err(text.lines().next().unwrap_or("codesign failed").to_string())
    }
}

impl Signer for Codesign {
    fn verify(&self, bundle: &Path) -> Result<(), String> {
        codesign(&["--verify", "--deep", "--strict"], bundle).map(|_| ())
    }

    fn team(&self, bundle: &Path) -> Result<Option<String>, String> {
        let text = codesign(&["-dv", "--verbose=2"], bundle)?;
        team_from_codesign(&text)
    }

    fn verify_requirement(&self, bundle: &Path, requirement: &str) -> Result<(), String> {
        // Two arguments: `-R` and the text, whose leading `=` marks it as
        // requirement source rather than a file.
        codesign(
            &["--verify", "--deep", "--strict", "-R", requirement],
            bundle,
        )
        .map(|_| ())
    }
}

/// The `TeamIdentifier=` line of `codesign -dv`: exactly one such line
/// (anywhere else in the output, such as inside an `Identifier=` value on
/// its own line, the text is not trusted), `not set` (ad hoc) is none.
pub(crate) fn team_from_codesign(text: &str) -> Result<Option<String>, String> {
    let found: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("TeamIdentifier="))
        .collect();
    let [team] = found.as_slice() else {
        return Err(format!(
            "codesign printed {} TeamIdentifier lines, not one",
            found.len()
        ));
    };
    let team = team.trim();
    Ok((!team.is_empty() && team != "not set").then(|| team.to_string()))
}

/// Whether `team` is an Apple team id: ten capital letters or digits.
pub(crate) fn is_team_id(team: &str) -> bool {
    team.len() == 10
        && team
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

/// The designated-requirement text a Developer ID Application signature of
/// `team` satisfies: Apple's anchor, the leaf's OU is the team, the
/// Developer ID intermediate (1.2.840.113635.100.6.2.6) and the Developer ID
/// Application leaf (1.2.840.113635.100.6.1.13). The team is checked before
/// it goes into the text.
pub(crate) fn team_requirement(team: &str) -> Result<String, String> {
    if !is_team_id(team) {
        return Err(format!(
            "`{team}` is not a team id (ten capitals or digits)"
        ));
    }
    Ok(format!(
        "=anchor apple generic and certificate leaf[subject.OU] = \"{team}\" and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    ))
}

pub(crate) struct MacProcs;

impl Procs for MacProcs {
    fn app_processes(&self, exe: &Path) -> Result<Vec<AppProc>, String> {
        let out = Command::new("/bin/ps")
            .args(["-axww", "-o", "pid=,args="])
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run ps: {e}"))?;
        if !out.status.success() {
            return Err("ps could not list the processes".into());
        }
        let me = self.own_pid();
        Ok(app_pids(&String::from_utf8_lossy(&out.stdout), exe)
            .into_iter()
            .filter(|pid| *pid != me)
            .map(|pid| AppProc {
                pid,
                started_ms: crate::proc_info::process_start_micros(pid).unwrap_or(0) / 1000,
            })
            .collect())
    }

    fn alive(&self, pid: i32) -> Option<bool> {
        crate::proc_info::pid_alive(pid)
    }

    fn started_micros(&self, pid: i32) -> Option<u64> {
        crate::proc_info::process_start_micros(pid)
    }

    fn terminate(&self, pid: i32) {
        if pid > 1 {
            // SAFETY: a plain signal to one pid.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
    }

    fn own_pid(&self) -> i32 {
        std::process::id() as i32
    }
}

/// `ps -o pid=,args=` lines whose command is `exe` alone or `exe -flag…`:
/// the app, not a subcommand child (`exe team-mcp`) or this installer.
pub(crate) fn app_pids(ps: &str, exe: &Path) -> Vec<i32> {
    let exe = exe.to_string_lossy();
    let with_flag = format!("{exe} -");
    ps.lines()
        .filter_map(|line| {
            let line = line.trim_start();
            let (pid, args) = line.split_once(char::is_whitespace)?;
            let args = args.trim();
            (args == exe || args.starts_with(&with_flag))
                .then(|| pid.parse().ok())
                .flatten()
        })
        .collect()
}

pub(crate) struct OpenLauncher;

impl Launcher for OpenLauncher {
    fn launch(&self, app: &Path) -> Result<(), String> {
        let status = Command::new("/usr/bin/open")
            .arg("-n")
            .arg(app)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|e| format!("could not run open: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("open exited {status}"))
        }
    }
}

pub(crate) struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}
