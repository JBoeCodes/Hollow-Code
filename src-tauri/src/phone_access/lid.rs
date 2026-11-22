//! Keep Hollow running with the lid closed (2026-11-05): Settings,
//! Connections → Companion devices → iPhone app → Keep Hollow running with
//! the lid closed. Off by default; turning it on is offered only while the
//! iPhone app is on.
//!
//! **What macOS allows** (read on Alex's MacBook Pro, macOS 27, with
//! `pmset -g`, `pmset -g assertions` and `ioreg -r -n IOPMrootDomain`):
//! - A power assertion (`PreventUserIdleSystemSleep`, what `caffeinate -i`
//!   holds) keeps idle sleep away with no special rights, but closing the
//!   lid still sleeps the Mac (`AppleClamshellCausesSleep = Yes`) unless an
//!   external display and power keep it in clamshell mode.
//! - Only system sleep disabled (`pmset -a disablesleep 1`, the root
//!   domain's `SleepDisabled`, root only, kept across restarts) keeps a
//!   closed lid awake. While it is set the kernel refuses every sleep,
//!   `pmset sleepnow` and the Apple menu's included, so Hollow can't put the
//!   Mac to sleep on battery by itself.
//!
//! **So the switch installs a helper** with one administrator prompt (the
//! person's own click; macOS's standard password dialog through
//! `osascript`): two LaunchDaemons, each a few lines of `sh` held in its
//! plist. The first, [`LABEL`] ([`HELPER`]), runs every [`TICK_SECS`]
//! seconds and whenever its armed folder changes. It disables system sleep
//! only while
//! - an empty file `armed.<pid>` in the armed folder ([`ARMED_DIR`]) names a
//!   process running the Hollow executable the helper was installed for, by
//!   the kernel's path for that pid (`lsof`'s first text file, which a
//!   process can't change, unlike the name `ps` shows). Hollow keeps one for
//!   itself only while the iPhone app is on and a phone is paired
//!   ([`Lid::sync`]);
//! - and the Mac is on power, or the person ticked On battery too (baked
//!   into both plists, so only another administrator prompt changes it).
//!   **Battery floor (ENG-139):** On battery too stops at 10% charge: below it
//!   the helper no longer wants sleep off, so it turns sleep back on and
//!   sleeps a closed Mac instead of running it down until it shuts off.
//!   A charge it can't read leaves the rule as it was.
//!
//! Otherwise it turns system sleep back on, but only when its marker
//! ([`MARKER`]) says it turned it off, and a Mac on battery whose lid is
//! closed then sleeps (`pmset sleepnow`). The second, the restore check
//! [`RESTORE_LABEL`] ([`RESTORE`]), runs on its own every [`TICK_SECS`]
//! seconds and at load, and does only the unplugged half of that: off power
//! without On battery too, with the marker, it turns sleep back on and
//! sleeps a closed Mac. It reads no folder and looks up no pid.
//!
//! **What that check doesn't stop:** the armed folder's files are the
//! person's, and Hollow's own MCP servers and reaper run its executable, so
//! while the helper is installed any process of theirs can hold it armed
//! (for instance by starting Hollow's executable and naming that pid). Only
//! the plists' power rule limits what that gets: sleep off on power, and on
//! battery too only with On battery too ticked. The folder itself is root's,
//! made by the install with an ACL that lets the person add and remove files
//! in it and nothing else (no subfolders), so no process of theirs can swap
//! it for a link or mount something on it. **A pass of the first daemon has
//! no time bound:** it looks at four names at most, but the shell reads and
//! sorts every name first, and one `lsof` lookup can wait on a file system
//! a process of the person's controls (the kernel stats each file the pid
//! maps; `-b` stops only lsof's own calls), while launchd starts no other
//! pass of a job until that one ends. So a process of theirs can hold that
//! pass, and with it sleep off on power, for as long as its file system
//! keeps it waiting; the restore check, which waits on none of that, is
//! what turns sleep back on when the Mac is unplugged. After its lookups a
//! pass reads the power source and `SleepDisabled` again and acts on those,
//! so one that was held up never turns sleep off after an unplug. Without
//! a `"SleepDisabled" = ` line from `ioreg` neither turns sleep off or puts
//! the Mac to sleep, but both still turn sleep back on with the marker when
//! sleep shouldn't be off. Neither follows a link: the install goes ahead
//! only when `/var/db/hollow` is absent or a real
//! folder of root's (then made root:wheel 755 with no ACL) and no link
//! stands at the armed folder, the marker or a last-pass file, and a pass
//! writes nothing while one does.
//!
//! Each finished pass touches its last-pass file ([`BEAT`], [`RESTORE_BEAT`];
//! so does the install), so Settings tells a helper that has stopped,
//! switched off in Login Items or held up, from one that is about to turn
//! sleep back on, and says Off turns it back on. Settings also compares
//! both installed plists with the ones this Hollow would write: a helper
//! from another version, or missing one of its daemons, says so, and Update
//! the helper (or any change) replaces both with one prompt. Turning the
//! switch off (a second prompt) removes both, the armed folder and the
//! last-pass files, and turns sleep back on when the marker says the helper
//! turned it off. Sleep disabled by anyone else is left alone, and Settings
//! says so. Beside the helper, Hollow holds an idle-sleep assertion
//! ([`Caffeinate`]: `caffeinate -i -w <pid>`, gone with Hollow) under the
//! same conditions.
//!
//! **Without Hollow**, an administrator removes it in Terminal with
//! `sudo launchctl bootout system/com.hollow.app.lid-closed`,
//! `sudo launchctl bootout system/com.hollow.app.lid-closed-restore` and
//! `sudo rm /Library/LaunchDaemons/com.hollow.app.lid-closed.plist
//! /Library/LaunchDaemons/com.hollow.app.lid-closed-restore.plist`, then,
//! when `/var/db/hollow/lid-closed-sleep-disabled`
//! exists, `sudo pmset -a disablesleep 0`, and last
//! `sudo rm -rf "/var/db/hollow"` (only this helper
//! keeps files there).
//!
//! The commands are the person's own clicks in the main window. Since L13
//! (2026-11-10) the COO reaches them too (`companion.phone_lid_status`,
//! `companion.phone_lid_set`), through the inner functions without the
//! window check: its switch only starts the change, after the person's
//! card, and the person still types the Mac's password at macOS's own
//! prompt ([`agent_lid_set`]).
//! Tests fake every power read, the prompt and the assertion ([`Power`],
//! [`Admin`], [`Hold`]), and run both passes and the install and uninstall
//! commands against fake tools in a temp folder.

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use super::store::Store;

/// The helper's launchd label.
pub(crate) const LABEL: &str = "com.hollow.app.lid-closed";
/// The restore check's launchd label.
pub(crate) const RESTORE_LABEL: &str = "com.hollow.app.lid-closed-restore";
/// Where the helper's plist goes.
const PLIST: &str = "/Library/LaunchDaemons/com.hollow.app.lid-closed.plist";
/// Where the restore check's plist goes.
const RESTORE_PLIST: &str = "/Library/LaunchDaemons/com.hollow.app.lid-closed-restore.plist";
/// The helper's marker: it turned system sleep off, so it may turn it back
/// on. Root's folder under `/var/db` (root:wheel 0755, not admin-writable),
/// kept across restarts as the setting itself is.
const MARKER: &str = "/var/db/hollow/lid-closed-sleep-disabled";
/// The helper's last pass: root touches it at the end of every pass that
/// finished (and the install, before the first), so its age tells Settings
/// whether the helper still runs.
const BEAT: &str = "/var/db/hollow/lid-closed-last-pass";
/// The restore check's last pass, as [`BEAT`] is the helper's.
const RESTORE_BEAT: &str = "/var/db/hollow/lid-closed-restore-last-pass";
/// The armed folder: root's (755), made by the install with one ACL entry
/// that lets the person add and remove files in it, and no subfolders.
pub(crate) const ARMED_DIR: &str = "/var/db/hollow/lid-closed-armed";
/// How often the helper looks, in seconds.
pub(crate) const TICK_SECS: i64 = 10;
/// How often Hollow brings its armed file and assertion in line (a phone
/// pairing, a change of power source).
const SYNC_EVERY: Duration = Duration::from_secs(15);
/// The armed file's name before Hollow's pid.
const ARMED: &str = "armed.";
/// How long macOS's password prompt may stay open.
const PROMPT_WAIT: Duration = Duration::from_secs(600);

/// The helper, one pass (`sh -c`, as root): `$1` the armed folder, `$2` the
/// Hollow executable it serves, `$3` `1` for On battery too, `$4` its
/// marker, `$5` its last-pass file. Absolute paths only. Unplugged without
/// On battery too it never reads the armed folder; otherwise it reads no
/// more than four names there, and nothing but names. A pid counts only when
/// the kernel's path for it (`lsof`'s first text file) is `$2`. A lookup has
/// no time bound (`-b` stops lsof's own `stat` and `readlink`, not the
/// kernel's stat of each file the pid maps, which can wait on a file system
/// the person controls), so only after its lookups does the pass read the
/// power source again and `SleepDisabled`, and it acts on those. Without a
/// `"SleepDisabled" = ` line it never turns sleep off or sleeps the Mac, but
/// with its marker it still turns sleep back on when sleep shouldn't be off.
/// It reads the armed folder only while none of the folder, the marker and
/// the last-pass file is a link, and writes nothing (exit 1) while one is;
/// a link at the marker goes as the marker would (`rm`, never what it
/// points at). Only the pass whose `rm` removes the marker may sleep the
/// Mac: with the restore check running beside it, the other may have
/// removed it first, and a second `sleepnow` could land after the Mac wakes.
pub(crate) const HELPER: &str = r#"dir="$1"; exe="$2"; battery="$3"; mark="$4"; beat="$5"
pdir="${dir%/*}"; pmark="${mark%/*}"; pbeat="${beat%/*}"
on_power() { /usr/bin/pmset -g ps | /usr/bin/head -n 1 | /usr/bin/grep -q "'AC Power'"; }
is_root() { [ "$(/usr/bin/id -u)" = 0 ]; }
safe_dir() { [ -n "$1" ] && [ ! -L "$1" ] && [ -d "$1" ] && { ! is_root || [ "$(/usr/bin/stat -f %u "$1")" = 0 ]; }; }
bad_parents() { ! safe_dir "$pdir" || ! safe_dir "$pmark" || ! safe_dir "$pbeat"; }
links() { bad_parents || [ -L "$dir" ] || [ -L "$mark" ] || [ -L "$beat" ] || { [ -e "$dir" ] && ! safe_dir "$dir"; }; }
! bad_parents || exit 1
want=0
if { [ "$battery" = 1 ] || on_power; } && ! links; then
  n=0
  for f in "$dir"/armed.*; do
    n=$((n+1)); [ "$n" -le 4 ] || break
    pid="${f##*/armed.}"
    case "$pid" in ''|*[!0-9]*) continue ;; esac
    [ "$(/usr/sbin/lsof -b -w -a -p "$pid" -d txt -Fn 2>/dev/null | /usr/bin/sed -n 's/^n//p' | /usr/bin/head -n 1)" = "$exe" ] || continue
    want=1; break
  done
fi
if [ "$want" = 1 ] && [ "$battery" != 1 ] && ! on_power; then want=0; fi
if [ "$want" = 1 ] && ! on_power; then
  pct="$(/usr/bin/pmset -g ps | /usr/bin/sed -n 's/.*[^0-9]\([0-9][0-9]*\)%.*/\1/p' | /usr/bin/head -n 1)"
  case "$pct" in ''|*[!0-9]*) ;; *) [ "$pct" -gt 10 ] || want=0 ;; esac
fi
root="$(/usr/sbin/ioreg -r -n IOPMrootDomain -d 1)" || root=
case "$root" in
  *'"SleepDisabled" = Yes'*) now=1 ;;
  *'"SleepDisabled" = No'*) now=0 ;;
  *) now= ;;
esac
if [ "$want" = 1 ]; then
  if [ "$now" = 0 ]; then ! links && : > "$mark" && /usr/bin/pmset -a disablesleep 1 || exit 1; fi
elif [ -f "$mark" ]; then
  ! bad_parents || exit 1
  /usr/bin/pmset -a disablesleep 0 || exit 1
  if ! bad_parents && /bin/rm "$mark" 2>/dev/null && [ "$now" = 1 ]; then
    case "$root" in *'"AppleClamshellState" = Yes'*) on_power || /usr/bin/pmset sleepnow ;; esac
  fi
fi
! links || exit 1
: > "$beat"
"#;

/// The restore check, one pass (`sh -c`, as root, at load and every
/// [`TICK_SECS`] seconds, as its own launchd job): `$1` `1` for On battery
/// too, `$2` the helper's marker, `$3` its own last-pass file. Off power
/// without On battery too and with the marker, it turns system sleep back
/// on, removes the marker and sleeps a closed Mac whose sleep was off, as
/// the helper does; otherwise it changes nothing. It reads no folder and
/// looks up no pid, so nothing a helper pass waits on holds it up. As in the
/// helper, only the pass whose `rm` removes the marker sleeps the Mac.
/// Without a `"SleepDisabled" = ` line it still turns sleep back on but
/// doesn't sleep the Mac. It writes nothing (exit 1) while its parent folder
/// or last-pass file is a link or not root-owned.
pub(crate) const RESTORE: &str = r#"battery="$1"; mark="$2"; beat="$3"
pmark="${mark%/*}"; pbeat="${beat%/*}"
on_power() { /usr/bin/pmset -g ps | /usr/bin/head -n 1 | /usr/bin/grep -q "'AC Power'"; }
is_root() { [ "$(/usr/bin/id -u)" = 0 ]; }
safe_dir() { [ -n "$1" ] && [ ! -L "$1" ] && [ -d "$1" ] && { ! is_root || [ "$(/usr/bin/stat -f %u "$1")" = 0 ]; }; }
safe_dir "$pmark" && safe_dir "$pbeat" || exit 1
if [ "$battery" != 1 ] && [ -f "$mark" ] && ! on_power; then
  root="$(/usr/sbin/ioreg -r -n IOPMrootDomain -d 1)" || root=
  /usr/bin/pmset -a disablesleep 0 || exit 1
  if safe_dir "$pmark" && /bin/rm "$mark" 2>/dev/null; then
    case "$root" in
      *'"SleepDisabled" = Yes'*)
        case "$root" in *'"AppleClamshellState" = Yes'*) on_power || /usr/bin/pmset sleepnow ;; esac ;;
    esac
  fi
fi
safe_dir "$pbeat" && [ ! -L "$beat" ] || exit 1
: > "$beat"
"#;

/// What the password prompt says, after macOS's own words.
const WHY_ON: &str = "Hollow wants to keep this Mac awake with its lid closed while your iPhone is paired. It installs a small helper that turns system sleep off only while Hollow asks for it.";
const WHY_CHANGE: &str = "Hollow wants to change when this Mac stays awake with its lid closed.";
const WHY_UPDATE: &str = "Hollow wants to update its lid-closed helper to this version.";
const WHY_OFF: &str =
    "Hollow wants to remove its lid-closed helper and turn system sleep back on if it turned it off.";

// ── Paths ────────────────────────────────────────────────────────────────

/// Where the helper's files are: the system's in the app, a temp folder's
/// in tests.
#[derive(Clone, Debug)]
pub(crate) struct Paths {
    pub plist: PathBuf,
    /// The restore check's plist.
    pub restore_plist: PathBuf,
    pub marker: PathBuf,
    /// The helper's last-pass file (root's).
    pub beat: PathBuf,
    /// The restore check's last-pass file (root's).
    pub restore_beat: PathBuf,
    /// The armed folder (root's; the person adds and removes files in it).
    pub armed: PathBuf,
}

impl Paths {
    fn system() -> Self {
        Self {
            plist: PathBuf::from(PLIST),
            restore_plist: PathBuf::from(RESTORE_PLIST),
            marker: PathBuf::from(MARKER),
            beat: PathBuf::from(BEAT),
            restore_beat: PathBuf::from(RESTORE_BEAT),
            armed: PathBuf::from(ARMED_DIR),
        }
    }

    /// Hollow's root-owned folder (`/var/db/hollow`): the marker's, the
    /// armed folder's and the last-pass files'.
    fn root_dir(&self) -> &Path {
        self.marker.parent().unwrap_or(Path::new("/"))
    }
}

/// `'…'` for /bin/sh.
fn sh_quote(path: &Path) -> String {
    crate::self_install::sh_quote(&path.to_string_lossy())
}

/// The person's uid, which the install names (`id -un`, as root) in the
/// armed folder's ACL.
fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

/// This process's executable as the kernel names it (`proc_pidpath`), which
/// is what the helper's `lsof` reads for its pid. `current_exe` keeps the
/// path as launched, so a symlink (or `/tmp`) in it would never match.
pub(crate) fn own_exe() -> String {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: `buf` is as long as the size passed, and proc_pidpath
        // writes no more than that.
        let n = unsafe {
            libc::proc_pidpath(
                std::process::id() as i32,
                buf.as_mut_ptr().cast(),
                buf.len() as u32,
            )
        };
        if n > 0 {
            buf.truncate(n as usize);
            if let Ok(path) = String::from_utf8(buf) {
                return path;
            }
        }
    }
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A LaunchDaemon of Hollow's: `label` runs `args` at load, every
/// [`TICK_SECS`] seconds and, with `watch`, whenever that path changes.
fn daemon_plist(label: &str, args: &[&str], watch: Option<&Path>) -> Result<Vec<u8>, String> {
    use plist::{Dictionary, Value};
    let mut dict = Dictionary::new();
    dict.insert("Label".into(), Value::String(label.into()));
    dict.insert(
        "ProgramArguments".into(),
        Value::Array(args.iter().map(|a| Value::String(a.to_string())).collect()),
    );
    dict.insert("RunAtLoad".into(), Value::Boolean(true));
    dict.insert("StartInterval".into(), Value::Integer(TICK_SECS.into()));
    if let Some(watch) = watch {
        dict.insert(
            "WatchPaths".into(),
            Value::Array(vec![Value::String(watch.to_string_lossy().into_owned())]),
        );
    }
    dict.insert(
        "AssociatedBundleIdentifiers".into(),
        Value::Array(vec![Value::String("com.hollow.app".into())]),
    );
    let mut out = Vec::new();
    plist::to_writer_xml(&mut out, &Value::Dictionary(dict)).map_err(|e| e.to_string())?;
    Ok(out)
}

/// The helper's plist: [`HELPER`] with its five arguments, every
/// [`TICK_SECS`] seconds and whenever the armed folder changes.
pub(crate) fn helper_plist(paths: &Paths, exe: &str, on_battery: bool) -> Result<Vec<u8>, String> {
    daemon_plist(
        LABEL,
        &[
            "/bin/sh",
            "-c",
            HELPER,
            "hollow-lid-closed",
            &paths.armed.to_string_lossy(),
            exe,
            if on_battery { "1" } else { "0" },
            &paths.marker.to_string_lossy(),
            &paths.beat.to_string_lossy(),
        ],
        Some(&paths.armed),
    )
}

/// The restore check's plist: [`RESTORE`] with its three arguments (no
/// armed folder, no Hollow), every [`TICK_SECS`] seconds and at load.
pub(crate) fn restore_plist(paths: &Paths, on_battery: bool) -> Result<Vec<u8>, String> {
    daemon_plist(
        RESTORE_LABEL,
        &[
            "/bin/sh",
            "-c",
            RESTORE,
            "hollow-lid-closed-restore",
            if on_battery { "1" } else { "0" },
            &paths.marker.to_string_lossy(),
            &paths.restore_beat.to_string_lossy(),
        ],
        None,
    )
}

/// Whether the plist at `path` is `xml`, compared as plists rather than
/// bytes: the script, every argument and launchd's keys.
fn same_plist(path: &Path, xml: &[u8]) -> bool {
    match (
        plist::Value::from_file(path),
        plist::Value::from_reader(std::io::Cursor::new(xml)),
    ) {
        (Ok(installed), Ok(wanted)) => installed == wanted,
        _ => false,
    }
}

/// The helper as installed: its armed folder, the Hollow it serves and its
/// On battery too.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Installed {
    pub armed: String,
    pub exe: String,
    pub on_battery: bool,
}

/// Read an installed helper's plist (world-readable); `None` when there is
/// none or it isn't Hollow's.
pub(crate) fn read_installed(path: &Path) -> Option<Installed> {
    let value = plist::Value::from_file(path).ok()?;
    let dict = value.as_dictionary()?;
    if dict.get("Label")?.as_string()? != LABEL {
        return None;
    }
    let args = dict.get("ProgramArguments")?.as_array()?;
    let arg = |i: usize| args.get(i).and_then(|v| v.as_string()).map(str::to_string);
    Some(Installed {
        armed: arg(4)?,
        exe: arg(5)?,
        on_battery: arg(6)? == "1",
    })
}

/// `<plist>.new`, quoted: where the install writes a plist before moving it
/// into place.
fn fresh(plist: &Path) -> String {
    sh_quote(&PathBuf::from(format!("{}.new", plist.display())))
}

/// What the install or uninstall says on stderr (macOS's error, then Settings)
/// when it refuses Hollow's root folder.
const NOT_ROOTS: &str =
    "Hollow's folder in /var/db is a link, or not a folder of root's, so Hollow changed nothing.";
/// What it says when a link stands where the helper keeps its files.
const A_LINK: &str = "A link stands where Hollow's helper keeps its files, so Hollow didn't install or change the helper.";

/// The root command that installs (or replaces) the helper and its restore
/// check. It goes ahead only when Hollow's folder is absent or a real folder
/// owned by root (no link), makes it root:wheel 755 with no ACL, and then
/// only when no link stands at the armed folder, the marker or a last-pass
/// file (nobody but root can add one after that). Then the armed folder
/// made root's, with one ACL entry that lets the person add and remove
/// files in it (`id -un` names them; no subfolders), both last-pass files
/// made anew, both plists written as root and loaded into launchd, the
/// restore check first, each running its first pass now.
pub(crate) fn install_command(paths: &Paths, helper: &[u8], restore: &[u8]) -> String {
    let (encoded, restore_encoded) = (STANDARD.encode(helper), STANDARD.encode(restore));
    let (target, fresh_helper) = (sh_quote(&paths.plist), fresh(&paths.plist));
    let (restore_target, fresh_restore) =
        (sh_quote(&paths.restore_plist), fresh(&paths.restore_plist));
    let root_dir = sh_quote(paths.root_dir());
    let armed = sh_quote(&paths.armed);
    let mark = sh_quote(&paths.marker);
    let beat = sh_quote(&paths.beat);
    let restore_beat = sh_quote(&paths.restore_beat);
    let uid = current_uid();
    format!(
        "if [ -L {root_dir} ] || {{ [ -e {root_dir} ] && {{ [ ! -d {root_dir} ] \
         || [ \"$(/usr/bin/stat -f %u {root_dir})\" != 0 ]; }}; }}; then \
         echo \"{NOT_ROOTS}\" >&2; exit 1; fi; \
         /bin/mkdir -p {root_dir} && /usr/sbin/chown root:wheel {root_dir} \
         && /bin/chmod 755 {root_dir} && /bin/chmod -N {root_dir} || exit 1; \
         for p in {armed} {mark} {beat} {restore_beat}; do \
         if [ -L \"$p\" ]; then echo \"{A_LINK}\" >&2; exit 1; fi; done; \
         if [ -e {armed} ] && {{ [ ! -d {armed} ] \
         || [ \"$(/usr/bin/stat -f %u {armed})\" != 0 ]; }}; then \
         echo \"{NOT_ROOTS}\" >&2; exit 1; fi; \
         /bin/mkdir -p {armed} && /usr/sbin/chown root:wheel {armed} && /bin/chmod 755 {armed} \
         && /bin/chmod -N {armed} \
         && /bin/chmod +a \"user:$(/usr/bin/id -un {uid}) allow add_file,delete_child\" {armed} \
         && [ ! -L {root_dir} ] && [ ! -L {beat} ] && [ ! -L {restore_beat} ] \
         && /bin/rm -f {beat} {restore_beat} && : > {beat} && : > {restore_beat} \
         && /usr/bin/printf %s '{encoded}' | /usr/bin/base64 -D -o {fresh_helper} \
         && /usr/bin/printf %s '{restore_encoded}' | /usr/bin/base64 -D -o {fresh_restore} \
         && /usr/sbin/chown root:wheel {fresh_helper} {fresh_restore} \
         && /bin/chmod 644 {fresh_helper} {fresh_restore} \
         && {{ /bin/launchctl bootout system/{LABEL} 2>/dev/null; \
         /bin/launchctl bootout system/{RESTORE_LABEL} 2>/dev/null; true; }} \
         && /bin/mv -f {fresh_restore} {restore_target} && /bin/mv -f {fresh_helper} {target} \
         && n=0 && until /bin/launchctl bootstrap system {restore_target}; do \
         n=$((n+1)); [ $n -lt 5 ] || exit 1; /bin/sleep 1; done \
         && n=0 && until /bin/launchctl bootstrap system {target}; do \
         n=$((n+1)); [ $n -lt 5 ] || exit 1; /bin/sleep 1; done"
    )
}

/// The root command that removes the helper and its restore check (both
/// unloaded first), verifies Hollow's root folder is a real root-owned
/// directory (never a symlink), turns system sleep back on when the marker
/// says the helper turned it off (only then, and before the armed folder
/// goes: a folder a process of the person's makes slow to empty must not
/// delay it, ENG-139), verifies the armed folder is not a symlink and is
/// root-owned before removing it, then removes both last-pass files, which go
/// even when `pmset` fails (the marker stays so the restore can be retried,
/// and the command exits 1).
pub(crate) fn uninstall_command(paths: &Paths) -> String {
    let (target, fresh_helper) = (sh_quote(&paths.plist), fresh(&paths.plist));
    let (restore_target, fresh_restore) =
        (sh_quote(&paths.restore_plist), fresh(&paths.restore_plist));
    let mark = sh_quote(&paths.marker);
    let armed = sh_quote(&paths.armed);
    let beat = sh_quote(&paths.beat);
    let restore_beat = sh_quote(&paths.restore_beat);
    let root_dir = sh_quote(paths.root_dir());
    format!(
        "/bin/launchctl bootout system/{LABEL} 2>/dev/null; \
         /bin/launchctl bootout system/{RESTORE_LABEL} 2>/dev/null; \
         /bin/rm -f {target} {fresh_helper} {restore_target} {fresh_restore}; \
         bad=0; \
         if [ -L {root_dir} ] || {{ [ -e {root_dir} ] && {{ [ ! -d {root_dir} ] \
         || [ \"$(/usr/bin/stat -f %u {root_dir})\" != 0 ]; }}; }}; then \
         echo \"{NOT_ROOTS}\" >&2; exit 1; fi; \
         if [ -e {root_dir} ]; then \
         if [ -f {mark} ]; then /usr/bin/pmset -a disablesleep 0 && /bin/rm -f {mark} || bad=1; fi; \
         if [ -L {armed} ] || {{ [ -e {armed} ] && {{ [ ! -d {armed} ] \
         || [ \"$(/usr/bin/stat -f %u {armed})\" != 0 ]; }}; }}; then \
         echo \"{A_LINK}\" >&2; bad=1; else /bin/rm -rf {armed}; fi; \
         /bin/rm -f {beat} {restore_beat}; \
         /bin/rmdir {root_dir} 2>/dev/null; \
         fi; [ \"$bad\" = 0 ]"
    )
}

/// The AppleScript that runs `command` as root after macOS's password
/// prompt, which shows `why`.
pub(crate) fn applescript(command: &str, why: &str) -> String {
    let escape = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "do shell script \"{}\" with administrator privileges with prompt \"{}\"",
        escape(command),
        escape(why)
    )
}

/// osascript's error in words: a cancelled prompt changed nothing.
fn admin_error(stderr: &str) -> String {
    if stderr.contains("-128") || stderr.contains("User canceled") {
        return "You cancelled macOS’s password prompt, so nothing changed.".into();
    }
    let said = stderr.trim();
    if said.is_empty() {
        "macOS didn’t make the change.".into()
    } else {
        format!("macOS didn’t make the change: {said}")
    }
}

// ── What Hollow reads and holds (faked in tests) ─────────────────────────

/// This Mac's power state, read without rights.
pub(crate) trait Power: Send + Sync {
    /// `ioreg -r -n IOPMrootDomain -d 1`: the root domain's properties.
    fn root_domain(&self) -> Option<String>;
    /// `pmset -g ps`: the power source.
    fn source(&self) -> Option<String>;
}

/// Runs a root command after macOS's administrator prompt.
pub(crate) trait Admin: Send + Sync {
    fn run(&self, command: &str, why: &str) -> Result<(), String>;
}

/// The idle-sleep assertion.
pub(crate) trait Hold: Send + Sync {
    /// Hold it or let it go; whether it is held now.
    fn set(&self, on: bool) -> bool;
}

fn read_command(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

struct MacPower;

impl Power for MacPower {
    fn root_domain(&self) -> Option<String> {
        read_command(
            "/usr/sbin/ioreg",
            &["-r", "-n", "IOPMrootDomain", "-d", "1"],
        )
    }

    fn source(&self) -> Option<String> {
        read_command("/usr/bin/pmset", &["-g", "ps"])
    }
}

struct MacAdmin;

impl Admin for MacAdmin {
    fn run(&self, command: &str, why: &str) -> Result<(), String> {
        let mut child = Command::new("/usr/bin/osascript")
            .arg("-e")
            .arg(applescript(command, why))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Hollow couldn’t ask macOS for your password: {e}"))?;
        let started = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let mut said = String::new();
                    if let Some(mut stderr) = child.stderr.take() {
                        let _ = stderr.read_to_string(&mut said);
                    }
                    return if status.success() {
                        Ok(())
                    } else {
                        Err(admin_error(&said))
                    };
                }
                Ok(None) if started.elapsed() > PROMPT_WAIT => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("macOS’s password prompt stayed open for 10 minutes, so Hollow closed it and changed nothing.".into());
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(200)),
                Err(e) => return Err(format!("Hollow lost macOS’s password prompt: {e}")),
            }
        }
    }
}

/// `caffeinate -i -w <Hollow's pid>`: the assertion lives as long as the
/// child, and the child no longer than Hollow.
pub(crate) struct Caffeinate {
    pid: u32,
    child: Mutex<Option<Child>>,
}

impl Hold for Caffeinate {
    fn set(&self, on: bool) -> bool {
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(running) = child.as_mut() {
            if matches!(running.try_wait(), Ok(None)) {
                if on {
                    return true;
                }
                let _ = running.kill();
            }
            let _ = running.wait();
            *child = None;
        }
        if !on {
            return false;
        }
        *child = Command::new("/usr/bin/caffeinate")
            .args(["-i", "-w", &self.pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| eprintln!("[phone] lid closed: caffeinate: {e}"))
            .ok();
        child.is_some()
    }
}

// ── Readings ─────────────────────────────────────────────────────────────

/// Where the Mac draws power from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Power,
    Battery,
    #[default]
    Unknown,
}

/// `pmset -g ps`: the source and the internal battery's charge.
pub(crate) fn parse_source(out: &str) -> (Source, Option<u8>) {
    let first = out.lines().next().unwrap_or("");
    let source = if first.contains("'AC Power'") {
        Source::Power
    } else if first.contains("'Battery Power'") {
        Source::Battery
    } else {
        Source::Unknown
    };
    let percent = out
        .lines()
        .find(|line| line.contains("InternalBattery"))
        .and_then(|line| {
            let end = line.find('%')?;
            let digits: String = line[..end]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_digit())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            digits.parse::<u8>().ok()
        });
    (source, percent)
}

/// A Yes/No property of `ioreg`'s output (`"SleepDisabled" = Yes`).
pub(crate) fn flag(out: &str, key: &str) -> Option<bool> {
    let needle = format!("\"{key}\" = ");
    let rest = &out[out.find(&needle)? + needle.len()..];
    if rest.starts_with("Yes") {
        Some(true)
    } else if rest.starts_with("No") {
        Some(false)
    } else {
        None
    }
}

// ── The lid ──────────────────────────────────────────────────────────────

/// Settings, ... , Keep Hollow running with the lid closed, as Settings
/// reads it. The words are the window's (`src/phoneLid.ts`).
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneLidStatus {
    /// This Hollow offers it (macOS, the Hollow that serves the iPhone app).
    pub available: bool,
    pub unavailable_reason: Option<String>,
    /// The helper is installed: the setting is on.
    pub enabled: bool,
    /// The installed helper's On battery too.
    pub on_battery: bool,
    /// The iPhone app is on.
    pub iphone_on: bool,
    /// The iPhone app is on and a phone is paired.
    pub ready: bool,
    /// Hollow has armed the helper now.
    pub armed: bool,
    /// Hollow holds its idle-sleep assertion now.
    pub holding: bool,
    pub source: Source,
    pub battery_percent: Option<u8>,
    /// System sleep is disabled now; `None` when it couldn't be read.
    pub sleep_disabled: Option<bool>,
    /// The helper's marker: Hollow's helper turned system sleep off.
    pub disabled_by_hollow: bool,
    /// The lid is closed now.
    pub lid_closed: Option<bool>,
    /// The helper serves another copy of Hollow (that copy's path).
    pub other_copy: Option<String>,
    /// Seconds since the helper's last finished pass ([`BEAT`]'s age);
    /// `None` when there is none.
    pub last_pass_secs: Option<u64>,
    /// Seconds since the restore check's last finished pass
    /// ([`RESTORE_BEAT`]'s age); `None` when there is none.
    pub last_restore_secs: Option<u64>,
    /// Seconds since this Hollow stopped asking for sleep off (its first
    /// sync that didn't ask); `None` while it asks.
    pub stopped_asking_secs: Option<u64>,
    /// The installed plists aren't the two this Hollow writes for them
    /// (another version's scripts or launchd keys, or one of them missing).
    pub outdated: bool,
    /// A change the COO started (`companion.phone_lid_set`) is waiting at
    /// macOS's password prompt or running.
    pub agent_change_pending: bool,
    /// Why the last change the COO started didn't finish, in Hollow's own
    /// words (never the prompt's output); cleared when the next one starts.
    pub agent_change_error: Option<String>,
}

/// The phone record's state the lid follows: (the iPhone app is on, a
/// phone is paired too).
type PhoneState = Box<dyn Fn() -> (bool, bool) + Send + Sync>;

pub(crate) struct Lid {
    paths: Paths,
    power: Box<dyn Power>,
    admin: Box<dyn Admin>,
    hold: Box<dyn Hold>,
    pid: u32,
    exe: String,
    phone: PhoneState,
    /// One change at a time: a prompt can stay open for minutes.
    changing: Mutex<()>,
    /// A change the COO started is reserved ([`Lid::reserve`]) until its
    /// background task finishes: the window's switch refuses meanwhile.
    reserved: AtomicBool,
    /// Why the last change the COO started didn't finish (fixed words).
    agent_error: Mutex<Option<String>>,
    /// When this Hollow stopped asking for sleep off: a helper pass that
    /// finished after it and left sleep off isn't following this Hollow.
    quiet_since: Mutex<Option<SystemTime>>,
}

/// What [`Lid::sync`] left in place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Synced {
    pub armed: bool,
    pub holding: bool,
    /// Armed, and on power or with On battery too: the helper's own rule
    /// for keeping sleep off.
    pub asking: bool,
}

impl Lid {
    pub(crate) fn new(
        paths: Paths,
        power: Box<dyn Power>,
        admin: Box<dyn Admin>,
        hold: Box<dyn Hold>,
        pid: u32,
        exe: String,
        phone: PhoneState,
    ) -> Self {
        Self {
            paths,
            power,
            admin,
            hold,
            pid,
            exe,
            phone,
            changing: Mutex::new(()),
            reserved: AtomicBool::new(false),
            agent_error: Mutex::new(None),
            quiet_since: Mutex::new(None),
        }
    }

    fn mine(&self) -> String {
        format!("{ARMED}{}", self.pid)
    }

    /// Keep `armed.<pid>` (and no other armed file) when `on`, else none.
    /// The folder is the install's: Hollow never makes it.
    fn arm(&self, on: bool) -> bool {
        let mine = self.mine();
        if let Ok(entries) = std::fs::read_dir(&self.paths.armed) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with(ARMED) && (!on || name != mine) {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        if !on {
            return false;
        }
        let file = self.paths.armed.join(&mine);
        if file.is_file() {
            return true;
        }
        match std::fs::write(&file, b"") {
            Ok(()) => true,
            Err(error) => {
                eprintln!("[phone] lid closed: can't arm the helper: {error}");
                false
            }
        }
    }

    fn source(&self) -> (Source, Option<u8>) {
        self.power
            .source()
            .map(|out| parse_source(&out))
            .unwrap_or_default()
    }

    /// Bring the armed file and the assertion in line: armed while the
    /// helper serves this Hollow, the iPhone app is on and a phone is
    /// paired; the assertion held while armed and on power, or with On
    /// battery too. The power source is read only while armed, so with the
    /// setting off the 15-second sync starts no process.
    pub(crate) fn sync(&self) -> Synced {
        self.sync_at(None)
    }

    fn sync_at(&self, source: Option<Source>) -> Synced {
        let synced = self.bring_in_line(source);
        let mut quiet = self.quiet_since.lock().unwrap_or_else(|e| e.into_inner());
        if synced.asking {
            *quiet = None;
        } else if quiet.is_none() {
            *quiet = Some(SystemTime::now());
        }
        synced
    }

    fn bring_in_line(&self, source: Option<Source>) -> Synced {
        let (_, ready) = (self.phone)();
        let installed = read_installed(&self.paths.plist).filter(|i| i.exe == self.exe);
        let Some(installed) = installed else {
            // Not installed, or the helper serves another copy of Hollow,
            // whose armed files the rest are: only this copy's own goes.
            let _ = std::fs::remove_file(self.paths.armed.join(self.mine()));
            self.hold.set(false);
            return Synced::default();
        };
        if !self.arm(ready) {
            self.hold.set(false);
            return Synced::default();
        }
        let source = source.unwrap_or_else(|| self.source().0);
        let asking = source == Source::Power || installed.on_battery;
        let holding = self.hold.set(asking);
        Synced {
            armed: true,
            holding,
            asking,
        }
    }

    /// The installed helper and restore check are the two this Hollow
    /// writes for the helper's Hollow and On battery too.
    fn current(&self, installed: &Installed) -> bool {
        helper_plist(&self.paths, &installed.exe, installed.on_battery)
            .is_ok_and(|xml| same_plist(&self.paths.plist, &xml))
            && restore_plist(&self.paths, installed.on_battery)
                .is_ok_and(|xml| same_plist(&self.paths.restore_plist, &xml))
    }

    /// Settings' reading (after a sync, so it is current).
    pub(crate) fn status(&self) -> PhoneLidStatus {
        let (source, battery_percent) = self.source();
        let synced = self.sync_at(Some(source));
        let (iphone_on, ready) = (self.phone)();
        let root = self.power.root_domain();
        let installed = read_installed(&self.paths.plist);
        // Either daemon installed is the setting on, so Off removes a lone
        // restore check too.
        let enabled = self.paths.plist.exists() || self.paths.restore_plist.exists();
        let now = SystemTime::now();
        // A time ahead of the clock (it was set back) reads as just now.
        let age = |at: SystemTime| now.duration_since(at).map_or(0, |d| d.as_secs());
        let last_pass = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let quiet = *self.quiet_since.lock().unwrap_or_else(|e| e.into_inner());
        PhoneLidStatus {
            available: true,
            unavailable_reason: None,
            enabled,
            on_battery: installed.as_ref().is_some_and(|i| i.on_battery),
            iphone_on,
            ready,
            armed: synced.armed,
            holding: synced.holding,
            source,
            battery_percent,
            sleep_disabled: root.as_deref().and_then(|r| flag(r, "SleepDisabled")),
            disabled_by_hollow: self.paths.marker.exists(),
            lid_closed: root.as_deref().and_then(|r| flag(r, "AppleClamshellState")),
            last_pass_secs: last_pass(&self.paths.beat).map(age),
            last_restore_secs: last_pass(&self.paths.restore_beat).map(age),
            stopped_asking_secs: quiet.map(age),
            outdated: enabled && !installed.as_ref().is_some_and(|i| self.current(i)),
            other_copy: installed.filter(|i| i.exe != self.exe).map(|i| i.exe),
            agent_change_pending: self.reserved.load(Ordering::SeqCst),
            agent_change_error: self
                .agent_error
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone(),
        }
    }

    /// The person's click: on (a new On battery too, or Update the helper)
    /// installs the helper and restore check this Hollow writes, off
    /// removes both, each after one macOS password prompt. Turning it on
    /// needs the iPhone app on; off always works.
    pub(crate) fn set(&self, enabled: bool, on_battery: bool) -> Result<(), String> {
        let _changing = self.changing.try_lock().map_err(|_| BUSY.to_string())?;
        // A change the COO started and the person hasn't answered yet.
        if self.reserved.load(Ordering::SeqCst) {
            return Err(BUSY.into());
        }
        self.set_locked(enabled, on_battery)
    }

    /// Reserve the next change for the COO's switch, synchronously, so its
    /// background task can't find one already running: refused while a
    /// prompt is open or another reserved change hasn't finished. The
    /// reservation clears the last one's failure.
    pub(crate) fn reserve(&self) -> Result<(), String> {
        let _changing = self.changing.try_lock().map_err(|_| BUSY.to_string())?;
        self.reserved
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map_err(|_| BUSY.to_string())?;
        *self.agent_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
        Ok(())
    }

    /// The change [`Lid::reserve`] reserved (the COO's background task):
    /// waits for the lock (a window click that came between refuses at
    /// once, seeing the reservation), makes the change, records why it
    /// failed where [`Lid::status`] shows it, and frees the reservation.
    pub(crate) fn set_reserved(&self, enabled: bool, on_battery: bool) -> Result<(), String> {
        struct Release<'a>(&'a AtomicBool);
        impl Drop for Release<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _release = Release(&self.reserved);
        let result = {
            let _changing = self.changing.lock().unwrap_or_else(|e| e.into_inner());
            self.set_locked(enabled, on_battery)
        };
        if let Err(error) = &result {
            *self.agent_error.lock().unwrap_or_else(|e| e.into_inner()) =
                Some(agent_lid_error(error));
        }
        result
    }

    /// [`Lid::set`]'s change, with `changing` held by the caller.
    fn set_locked(&self, enabled: bool, on_battery: bool) -> Result<(), String> {
        if !enabled {
            self.admin.run(&uninstall_command(&self.paths), WHY_OFF)?;
            self.sync();
            return Ok(());
        }
        let (iphone_on, _) = (self.phone)();
        if !iphone_on {
            return Err(
                "Turn on the iPhone app first: this keeps the Mac awake only for a paired iPhone."
                    .into(),
            );
        }
        let helper = helper_plist(&self.paths, &self.exe, on_battery)?;
        let restore = restore_plist(&self.paths, on_battery)?;
        // Already these two (the same scripts, Hollow, folder and On
        // battery too): nothing to ask.
        if same_plist(&self.paths.plist, &helper) && same_plist(&self.paths.restore_plist, &restore)
        {
            self.sync();
            return Ok(());
        }
        let why = match read_installed(&self.paths.plist) {
            None => WHY_ON,
            Some(i)
                if i.exe == self.exe
                    && i.on_battery == on_battery
                    && Path::new(&i.armed) == self.paths.armed =>
            {
                WHY_UPDATE
            }
            Some(_) => WHY_CHANGE,
        };
        // The install makes the armed folder; the sync after it arms, and
        // that change starts the helper's next pass (WatchPaths).
        let result = self
            .admin
            .run(&install_command(&self.paths, &helper, &restore), why);
        self.sync();
        result
    }
}

// ── The app's lid ────────────────────────────────────────────────────────

static LID: OnceLock<Arc<Lid>> = OnceLock::new();

/// Start the lid's side of phone access (`phone_access::start`): sync now
/// and every [`SYNC_EVERY`]. A stale armed file from an earlier launch goes
/// at once.
pub(crate) fn start(store: Arc<Store>) {
    if !cfg!(target_os = "macos") {
        return;
    }
    let exe = own_exe();
    let pid = std::process::id();
    let phone: PhoneState = Box::new(move || {
        let enabled = store.read().is_ok_and(|record| record.enabled);
        (enabled, store.phone_ready())
    });
    let lid = Arc::new(Lid::new(
        Paths::system(),
        Box::new(MacPower),
        Box::new(MacAdmin),
        Box::new(Caffeinate {
            pid,
            child: Mutex::new(None),
        }),
        pid,
        exe,
        phone,
    ));
    if LID.set(lid.clone()).is_err() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        loop {
            let each = lid.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || each.sync()).await;
            tokio::time::sleep(SYNC_EVERY).await;
        }
    });
}

/// Bring the armed file and the assertion in line now (the iPhone app
/// switch, a revoke).
pub(crate) fn sync_now() {
    if let Some(lid) = LID.get().cloned() {
        tauri::async_runtime::spawn_blocking(move || {
            lid.sync();
        });
    }
}

fn unavailable() -> PhoneLidStatus {
    PhoneLidStatus {
        available: false,
        unavailable_reason: Some(if cfg!(target_os = "macos") {
            super::app().err().unwrap_or_else(|| {
                "Keeping the Mac awake isn’t available in this copy of Hollow.".into()
            })
        } else {
            "Keeping the lid closed is for macOS only.".into()
        }),
        ..PhoneLidStatus::default()
    }
}

/// Keep Hollow running with the lid closed, for Settings (the person's main
/// window only).
#[tauri::command]
pub async fn phone_lid_status(webview: tauri::Webview) -> Result<PhoneLidStatus, String> {
    super::main_window(&webview)?;
    lid_status().await
}

/// The section's reading (after the same sync the 60-second loop runs).
pub(crate) async fn lid_status() -> Result<PhoneLidStatus, String> {
    let Some(lid) = LID.get().cloned() else {
        return Ok(unavailable());
    };
    tauri::async_runtime::spawn_blocking(move || lid.status())
        .await
        .map_err(|e| e.to_string())
}

fn lid() -> Result<Arc<Lid>, String> {
    LID.get()
        .cloned()
        .ok_or_else(|| unavailable().unavailable_reason.unwrap_or_default())
}

/// The switch, On battery too and Update the helper (the person's click):
/// installs, changes, updates or removes the helper and its restore check
/// together after one macOS password prompt.
#[tauri::command]
pub async fn phone_lid_set(
    webview: tauri::Webview,
    enabled: bool,
    on_battery: bool,
) -> Result<PhoneLidStatus, String> {
    super::main_window(&webview)?;
    let lid = lid()?;
    tauri::async_runtime::spawn_blocking(move || {
        lid.set(enabled, on_battery)?;
        Ok(lid.status())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The refusal while a change is under way.
const BUSY: &str = "macOS is already asking for your password. Answer that prompt first.";

/// A failed change's words for an agent: Hollow's own fixed sentences as
/// they are, never what macOS's prompt or the install printed.
pub(crate) fn agent_lid_error(error: &str) -> String {
    const FIXED: [&str; 5] = [
        BUSY,
        "You cancelled macOS’s password prompt, so nothing changed.",
        "macOS didn’t make the change.",
        "macOS’s password prompt stayed open for 10 minutes, so Hollow closed it and changed nothing.",
        "Turn on the iPhone app first: this keeps the Mac awake only for a paired iPhone.",
    ];
    if FIXED.contains(&error) {
        return error.to_string();
    }
    "The change didn't finish (lid-failed): macOS or the helper's install refused it. Turning it on or off in Hollow's Settings (Devices, Companion devices, iPhone app) shows the reason.".into()
}

/// What the COO's switch says once it has started the change.
pub(crate) const PROMPT_NOTE: &str = "macOS is showing its password prompt on this Mac now. The person types the Mac's password there (Hollow and agents never see it); nothing changes until they do, and the prompt closes after 10 minutes, changing nothing. Read companion.phone_lid_status afterwards.";

/// Whether `now` already is `enabled` (and, on, `on_battery`, this
/// version's daemons): then there is nothing to ask macOS.
pub(crate) fn already_set(now: &PhoneLidStatus, enabled: bool, on_battery: bool) -> bool {
    if enabled {
        // A helper serving another copy of Hollow is rebound to this one,
        // as the window's switch does.
        now.enabled && now.on_battery == on_battery && !now.outdated && now.other_copy.is_none()
    } else {
        !now.enabled
    }
}

/// The COO's switch (after the person's card): checks what it can at once
/// (the iPhone app on; already set), then starts the change and returns
/// without waiting, because only the person can finish it, at macOS's
/// password prompt. Settings reads the section again when it is done.
pub(crate) async fn agent_lid_set(
    enabled: bool,
    on_battery: bool,
) -> Result<serde_json::Value, String> {
    let lid = lid()?;
    let now = {
        let lid = lid.clone();
        tauri::async_runtime::spawn_blocking(move || lid.status())
            .await
            .map_err(|e| e.to_string())?
    };
    if enabled && !now.iphone_on {
        return Err(
            "Turn on the iPhone app first: this keeps the Mac awake only for a paired iPhone."
                .into(),
        );
    }
    if already_set(&now, enabled, on_battery) {
        return Ok(serde_json::json!({
            "started": false,
            "note": "Already set that way, so there is nothing to change and macOS asks for nothing.",
            "status": now,
        }));
    }
    // Reserved here, before the reply says it started: a change already
    // under way refuses now instead of failing unseen in the task.
    lid.reserve()?;
    // The reply carries the status read after reserving (agentChangePending
    // on, the last failure cleared), read first in the task that then makes
    // the change, so the task always runs and always frees the reservation.
    let (tx, rx) = tokio::sync::oneshot::channel();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = tx.send(lid.status());
        if let Err(error) = lid.set_reserved(enabled, on_battery) {
            eprintln!("[phone] keep running with the lid closed: {error}");
        }
        if let Some(state) = super::APP.get() {
            let _ = state.app.emit(super::EVENT, ());
        }
    });
    let reserved = rx.await.unwrap_or(now);
    Ok(serde_json::json!({
        "started": true,
        "note": PROMPT_NOTE,
        "status": reserved,
    }))
}

/// L13 review: the COO's switch reserves the change before it answers, and
/// the eventual failure is kept, in fixed words, where the status shows it.
#[cfg(test)]
mod reserve_tests {
    use super::*;

    struct NoPower;
    impl Power for NoPower {
        fn root_domain(&self) -> Option<String> {
            None
        }
        fn source(&self) -> Option<String> {
            None
        }
    }

    /// macOS refuses, printing something that must never reach an agent.
    struct Refuses;
    impl Admin for Refuses {
        fn run(&self, _command: &str, _why: &str) -> Result<(), String> {
            Err(admin_error("osascript: SECRET-ARG-123 went wrong"))
        }
    }

    struct NoHold;
    impl Hold for NoHold {
        fn set(&self, _on: bool) -> bool {
            false
        }
    }

    fn lid(root: &Path) -> Lid {
        let support = root.join("Support");
        let daemons = root.join("LaunchDaemons");
        std::fs::create_dir_all(&daemons).unwrap();
        Lid::new(
            Paths {
                plist: daemons.join(format!("{LABEL}.plist")),
                restore_plist: daemons.join(format!("{RESTORE_LABEL}.plist")),
                marker: support.join("lid-closed-sleep-disabled"),
                beat: support.join("lid-closed-last-pass"),
                restore_beat: support.join("lid-closed-restore-last-pass"),
                armed: support.join("lid-closed-armed"),
            },
            Box::new(NoPower),
            Box::new(Refuses),
            Box::new(NoHold),
            4242,
            "/Applications/Hollow.app/Contents/MacOS/hollow".into(),
            Box::new(|| (true, true)),
        )
    }

    #[test]
    fn a_reserved_change_refuses_the_next_and_keeps_its_failure_in_fixed_words() {
        let dir = tempfile::tempdir().unwrap();
        let lid = lid(dir.path());
        lid.reserve().unwrap();
        assert_eq!(lid.reserve().unwrap_err(), BUSY, "a second reservation");
        assert_eq!(
            lid.set(true, false).unwrap_err(),
            BUSY,
            "the window's switch"
        );
        assert!(lid.status().agent_change_pending);

        let error = lid.set_reserved(true, false).unwrap_err();
        assert!(
            error.contains("SECRET-ARG-123"),
            "the window path keeps it: {error}"
        );
        let status = lid.status();
        assert!(!status.agent_change_pending, "released");
        let kept = status
            .agent_change_error
            .clone()
            .expect("kept for the status");
        assert!(!kept.contains("SECRET-ARG-123"), "{kept}");
        assert!(kept.contains("lid-failed"), "{kept}");
        let agent = serde_json::to_string(&status).unwrap();
        assert!(!agent.contains("SECRET-ARG-123"), "{agent}");

        // Free again; the next reservation clears the last failure.
        lid.reserve().unwrap();
        assert!(lid.status().agent_change_error.is_none());
        let _ = lid.set_reserved(false, false);
        assert!(!lid.status().agent_change_pending);
    }

    #[test]
    fn hollows_own_refusals_pass_and_anything_else_is_fixed() {
        assert_eq!(agent_lid_error(BUSY), BUSY);
        let cancelled = "You cancelled macOS’s password prompt, so nothing changed.";
        assert_eq!(agent_lid_error(cancelled), cancelled);
        let said = agent_lid_error("macOS didn’t make the change: token=abc123");
        assert!(!said.contains("abc123"), "{said}");
    }
}

#[cfg(test)]
#[path = "lid_tests.rs"]
mod tests;
