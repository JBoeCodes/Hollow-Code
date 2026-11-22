//! Keep Hollow running with the lid closed: the helper's pass and the
//! restore check's against fake `pmset`, `ioreg`, `lsof` and `ps`, the
//! install and uninstall commands against fake `stat`, `chown`, `chmod`,
//! `launchctl` and `pmset` in a temp folder, and the lid against fake power
//! reads, a fake password prompt and a fake assertion. Nothing here runs the
//! real `pmset`, `launchctl`, `sudo`, `osascript` or `caffeinate`; one test
//! reads this test process's own executable with the real `lsof`.

use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

const EXE: &str = "/Applications/Hollow.app/Contents/MacOS/hollow";
const ON_POWER: &str = "Now drawing from 'AC Power'\n -InternalBattery-0 (id=23461987)\t100%; charged; 0:00 remaining present: true\n";
const ON_BATTERY: &str = "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=23461987)\t87%; discharging; 9:12 remaining present: true\n";

fn temp(name: &str) -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix(&format!("lid-{name}-"))
        .tempdir()
        .unwrap()
}

fn write_tool(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\n{body}")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn link(target: &Path, at: &Path) {
    std::os::unix::fs::symlink(target, at).unwrap();
}

// ── The helper's pass and the restore check's ────────────────────────────

/// What the fake `ioreg` prints for the root domain.
#[derive(Clone, Copy, Debug)]
enum Root {
    /// `"SleepDisabled" = Yes/No` and `"AppleClamshellState" = Yes/No`.
    Reads,
    /// No top-level `"SleepDisabled" = ` line (only a nested key).
    NoSleepLine,
    /// `ioreg` succeeds and prints nothing.
    Empty,
    /// `ioreg` fails and prints nothing.
    Fails,
}

/// What happens while a pid is looked up.
#[derive(Clone, Copy)]
enum During {
    /// The lookup doesn't answer until the test releases it: a file the pid
    /// maps sits on a file system a process of the person's keeps waiting.
    Hang,
    /// The Mac is unplugged meanwhile.
    Unplug,
    /// Someone else turns system sleep off meanwhile.
    SleepTurnedOff,
}

/// Which script a pass runs.
#[derive(Clone, Copy)]
enum Script {
    Helper,
    Restore,
}

struct Pass {
    /// Armed file names in the folder.
    armed: Vec<String>,
    source: &'static str,
    battery: bool,
    disabled: bool,
    lid_closed: bool,
    marker: bool,
    root: Root,
    /// Each pid's executable as the kernel names it (what `lsof` prints).
    exes: Vec<(u32, &'static str)>,
    /// Each pid's name as it shows it (`ps -o comm=`), which a process sets
    /// itself (`exec -a`): the helper must never trust it.
    names: Vec<(u32, &'static str)>,
    /// What happens while each of these pids is looked up.
    during: Vec<(u32, During)>,
    /// `pmset -a` fails.
    pmset_fails: bool,
    /// The other pass removes the marker while this one turns sleep back on.
    other_restores: bool,
}

impl Default for Pass {
    /// On power, sleep on, nothing armed; the pid 4242 is this Hollow.
    fn default() -> Self {
        Self {
            armed: Vec::new(),
            source: ON_POWER,
            battery: false,
            disabled: false,
            lid_closed: false,
            marker: false,
            root: Root::Reads,
            exes: vec![(4242, EXE)],
            names: vec![(4242, EXE)],
            during: Vec::new(),
            pmset_fails: false,
            other_restores: false,
        }
    }
}

fn armed(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}

struct After {
    /// What `pmset` changed.
    log: String,
    /// Each pid looked up and by what (`lsof 4242`, `ps 4242`).
    looked: Vec<String>,
    disabled: bool,
    marker: bool,
    /// The helper's last-pass file exists.
    beat: bool,
    /// The restore check's last-pass file exists.
    restore_beat: bool,
    /// The pass exited 0.
    ok: bool,
}

/// Both scripts' files and fake tools in a temp folder, shared, so a test
/// can run a restore pass while a helper pass waits.
struct PassRig {
    _dir: tempfile::TempDir,
    bin: PathBuf,
    state: PathBuf,
    armed_dir: PathBuf,
    mark: PathBuf,
    beat: PathBuf,
    restore_beat: PathBuf,
    battery: bool,
}

impl PassRig {
    fn new(case: &Pass) -> Self {
        let dir = temp("pass");
        let root = dir.path();
        let (bin, state, armed_dir, sys) = (
            root.join("bin"),
            root.join("state"),
            root.join("armed"),
            root.join("root"),
        );
        for d in [&bin, &state, &armed_dir, &sys] {
            std::fs::create_dir_all(d).unwrap();
        }
        for name in &case.armed {
            std::fs::write(armed_dir.join(name), b"").unwrap();
        }
        let yes = |b: bool| if b { "Yes\n" } else { "No\n" };
        let put = |name: &str, text: &str| std::fs::write(state.join(name), text).unwrap();
        put("source", case.source);
        put("battery-source", ON_BATTERY);
        put("disabled", yes(case.disabled));
        put("lid", yes(case.lid_closed));
        put("log", "");
        put("looked", "");
        match case.root {
            Root::Reads => {}
            Root::NoSleepLine => put("no-sleep-line", ""),
            Root::Empty => put("ioreg-empty", ""),
            Root::Fails => put("ioreg-fails", ""),
        }
        if case.pmset_fails {
            put("pmset-fails", "");
        }
        if case.other_restores {
            put("other-restores", "");
        }
        for (pid, exe) in &case.exes {
            put(&format!("exe.{pid}"), exe);
        }
        for (pid, name) in &case.names {
            put(&format!("name.{pid}"), &format!("{name}\n"));
        }
        for (pid, during) in &case.during {
            let what = match during {
                During::Hang => "hang",
                During::Unplug => "unplug",
                During::SleepTurnedOff => "sleep-off",
            };
            put(&format!("{what}.{pid}"), "");
        }
        let mark = sys.join("lid-closed-sleep-disabled");
        let beat = sys.join("lid-closed-last-pass");
        let restore_beat = sys.join("lid-closed-restore-last-pass");
        if case.marker {
            std::fs::write(&mark, b"").unwrap();
        }
        put("marker-path", &mark.to_string_lossy());
        let s = sh_quote(&state);
        write_tool(
            &bin.join("pmset"),
            &format!(
                "s={s}\ncase \"$1\" in\n  -g) cat \"$s/source\" ;;\n  -a) [ -f \"$s/pmset-fails\" ] && exit 1; echo \"disablesleep $3\" >> \"$s/log\"; if [ \"$3\" = 1 ]; then echo Yes > \"$s/disabled\"; else echo No > \"$s/disabled\"; if [ -f \"$s/other-restores\" ]; then rm -f \"$(cat \"$s/marker-path\")\"; fi; fi ;;\n  sleepnow) echo sleepnow >> \"$s/log\" ;;\nesac\n"
            ),
        );
        write_tool(
            &bin.join("ioreg"),
            &format!(
                "s={s}\n[ -f \"$s/ioreg-fails\" ] && exit 1\n[ -f \"$s/ioreg-empty\" ] && exit 0\nif [ -f \"$s/no-sleep-line\" ]; then printf '  \"Supported\" = {{\"SleepDisabled\"=Yes}}\\n  \"AppleClamshellState\" = %s\\n' \"$(cat \"$s/lid\")\"; exit 0; fi\nprintf '  \"SleepDisabled\" = %s\\n  \"AppleClamshellState\" = %s\\n' \"$(cat \"$s/disabled\")\" \"$(cat \"$s/lid\")\"\n"
            ),
        );
        // `lsof -b -w -a -p <pid> -d txt -Fn`, in lsof's field form: the
        // executable first, then dyld. A hung lookup waits (for a minute at
        // most, and no longer than its folder) for the test's release.
        write_tool(
            &bin.join("lsof"),
            &format!(
                "s={s}\npid=''\nwhile [ $# -gt 0 ]; do [ \"$1\" = -p ] && pid=\"$2\"; shift; done\necho \"lsof $pid\" >> \"$s/looked\"\nif [ -f \"$s/hang.$pid\" ]; then n=0; while [ -d \"$s\" ] && [ ! -f \"$s/release\" ] && [ \"$n\" -lt 600 ]; do /bin/sleep 0.1; n=$((n+1)); done; fi\n[ -f \"$s/unplug.$pid\" ] && cat \"$s/battery-source\" > \"$s/source\"\n[ -f \"$s/sleep-off.$pid\" ] && echo Yes > \"$s/disabled\"\n[ -f \"$s/exe.$pid\" ] || exit 1\nprintf 'p%s\\nftxt\\nn%s\\nftxt\\nn/usr/lib/dyld\\n' \"$pid\" \"$(cat \"$s/exe.$pid\")\"\n"
            ),
        );
        // `ps -o comm= -p <pid>`: the name a process shows for itself.
        write_tool(
            &bin.join("ps"),
            &format!("s={s}\necho \"ps $4\" >> \"$s/looked\"\n[ -f \"$s/name.$4\" ] && cat \"$s/name.$4\"\nexit 0\n"),
        );
        Self {
            _dir: dir,
            bin,
            state,
            armed_dir,
            mark,
            beat,
            restore_beat,
            battery: case.battery,
        }
    }

    /// One pass of `script` as launchd starts it, with every tool faked.
    fn command(&self, script: Script) -> std::process::Command {
        let text = match script {
            Script::Helper => HELPER,
            Script::Restore => RESTORE,
        };
        let fake = text
            .replace("/usr/bin/pmset", &sh_quote(&self.bin.join("pmset")))
            .replace("/usr/sbin/ioreg", &sh_quote(&self.bin.join("ioreg")))
            .replace("/usr/sbin/lsof", &sh_quote(&self.bin.join("lsof")))
            .replace("/bin/ps", &sh_quote(&self.bin.join("ps")));
        assert!(
            !fake.contains("/usr/bin/pmset") && !fake.contains("/usr/sbin/lsof"),
            "every pmset and lsof is the fake"
        );
        let battery = if self.battery { "1" } else { "0" };
        let mut command = std::process::Command::new("/bin/sh");
        command.arg("-c").arg(fake);
        match script {
            Script::Helper => command
                .arg("hollow-lid-closed")
                .arg(&self.armed_dir)
                .arg(EXE)
                .arg(battery)
                .arg(&self.mark)
                .arg(&self.beat),
            Script::Restore => command
                .arg("hollow-lid-closed-restore")
                .arg(battery)
                .arg(&self.mark)
                .arg(&self.restore_beat),
        };
        command
    }

    fn run(&self, script: Script) -> After {
        let ok = self.command(script).status().unwrap().success();
        self.after(ok)
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.state.join(name)).unwrap()
    }

    fn looked(&self) -> Vec<String> {
        self.read("looked").lines().map(str::to_string).collect()
    }

    fn after(&self, ok: bool) -> After {
        After {
            log: self.read("log"),
            looked: self.looked(),
            disabled: self.read("disabled").trim() == "Yes",
            marker: self.mark.exists(),
            beat: self.beat.exists(),
            restore_beat: self.restore_beat.exists(),
            ok,
        }
    }

    /// Alex plugs in or unplugs.
    fn set_source(&self, source: &str) {
        std::fs::write(self.state.join("source"), source).unwrap();
    }

    /// The hung lookup answers.
    fn release(&self) {
        std::fs::write(self.state.join("release"), b"").unwrap();
    }
}

/// One helper pass with fake tools.
fn pass(case: Pass) -> After {
    PassRig::new(&case).run(Script::Helper)
}

/// One restore check pass with fake tools.
fn restore(case: Pass) -> After {
    PassRig::new(&case).run(Script::Restore)
}

/// A pass started in the background, stopped if the test ends first.
struct Running(std::process::Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn helper_disables_sleep_on_power_while_this_hollow_has_armed_it() {
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 1\n");
    assert!(after.disabled);
    assert!(after.marker, "it remembers that it turned sleep off");
    assert_eq!(after.looked, vec!["lsof 4242"], "the kernel's path, once");
    assert!(after.ok && after.beat, "a finished pass records itself");
    assert!(!after.restore_beat, "and only its own");
}

#[test]
fn helper_trusts_the_kernels_executable_path_not_the_name_a_process_shows() {
    // 4243 runs /bin/sleep but shows Hollow's path as its name (`exec -a`),
    // and 4244 has gone: neither keeps the Mac awake.
    let after = pass(Pass {
        armed: armed(&["armed.4243", "armed.4244"]),
        exes: vec![(4242, EXE), (4243, "/bin/sleep")],
        names: vec![(4242, EXE), (4243, EXE), (4244, EXE)],
        ..Pass::default()
    });
    assert_eq!(after.log, "", "sleep stays on");
    assert!(!after.disabled && !after.marker);
    assert_eq!(after.looked, vec!["lsof 4243", "lsof 4244"]);
    assert!(after.ok && after.beat);
    assert!(
        HELPER.contains(r#"/usr/sbin/lsof -b -w -a -p "$pid" -d txt -Fn"#),
        "lsof's own stat and readlink off"
    );
}

#[test]
fn helper_unplugged_without_on_battery_too_restores_without_reading_the_folder() {
    // Lid closed on battery, with Hollow's file and 300 others there.
    let mut names: Vec<String> = (1000..1300).map(|p| format!("armed.{p}")).collect();
    names.push("armed.4242".into());
    let after = pass(Pass {
        armed: names,
        source: ON_BATTERY,
        disabled: true,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled && !after.marker);
    assert!(
        after.looked.is_empty(),
        "no name looked up: {:?}",
        after.looked
    );
    assert!(after.ok && after.beat);
}

#[test]
fn helper_looks_at_no_more_than_four_armed_names() {
    // A flood sorted before Hollow's own name: the pass stops after four
    // and counts the folder as not armed, so sleep comes back.
    let mut names: Vec<String> = (1000..1050).map(|p| format!("armed.{p}")).collect();
    names.push("armed.4242".into());
    let after = pass(Pass {
        armed: names,
        disabled: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.looked.len(), 4, "{:?}", after.looked);
    assert_eq!(after.log, "disablesleep 0\n");
    assert!(!after.disabled && !after.marker);
    // Names that aren't pids count toward the four too (these sort before
    // Hollow's).
    let after = pass(Pass {
        armed: armed(&["armed.", "armed.1a", "armed.2b", "armed.3c", "armed.4242"]),
        ..Pass::default()
    });
    assert!(after.looked.is_empty() && after.log.is_empty());
}

#[test]
fn helper_acts_on_the_power_source_after_its_lookups_not_before() {
    // Unplugged while the pass looks Hollow up: no sleep off on battery.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        lid_closed: true,
        during: vec![(4242, During::Unplug)],
        ..Pass::default()
    });
    assert_eq!(after.looked, vec!["lsof 4242"]);
    assert_eq!(after.log, "", "no disablesleep 1 after the unplug");
    assert!(!after.disabled && !after.marker && after.ok);
    // With sleep off on Hollow's marker, the same unplug turns it back on
    // and sleeps the closed Mac.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        disabled: true,
        lid_closed: true,
        marker: true,
        during: vec![(4242, During::Unplug)],
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled && !after.marker);
    // With On battery too the unplug changes nothing.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        battery: true,
        during: vec![(4242, During::Unplug)],
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 1\n");
}

#[test]
fn helper_reads_sleep_disabled_after_its_lookups_not_before() {
    // Someone else turns system sleep off while the pass looks Hollow up:
    // the pass leaves it theirs.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        during: vec![(4242, During::SleepTurnedOff)],
        ..Pass::default()
    });
    assert_eq!(after.log, "", "no disablesleep 1 over theirs");
    assert!(after.disabled);
    assert!(!after.marker, "never claims sleep it didn't turn off");
}

#[test]
fn helper_on_battery_turns_sleep_back_on_and_sleeps_a_closed_mac() {
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: ON_BATTERY,
        disabled: true,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled);
    assert!(!after.marker);
}

#[test]
fn helper_with_on_battery_too_keeps_sleep_off_on_battery() {
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: ON_BATTERY,
        battery: true,
        lid_closed: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 1\n");
    assert!(after.disabled && after.marker);
}

#[test]
fn helper_with_on_battery_too_stops_keeping_sleep_off_at_ten_percent() {
    let low = |pct: u8| {
        format!(
            "Now drawing from 'Battery Power'\n -InternalBattery-0 (id=23461987)\t{pct}%; discharging; 0:40 remaining present: true\n"
        )
    };
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    // 11%: still kept awake.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: leak(low(11)),
        battery: true,
        lid_closed: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 1\n");
    // 10%, sleep off with the marker: sleep comes back and the closed Mac sleeps.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: leak(low(10)),
        battery: true,
        disabled: true,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled && !after.marker);
    // 5% with nothing off yet: it never turns sleep off.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: leak(low(5)),
        battery: true,
        lid_closed: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "");
    assert!(!after.disabled);
    // Power at any charge is unaffected.
    let after = pass(Pass {
        armed: armed(&["armed.4242"]),
        source: "Now drawing from 'AC Power'\n -InternalBattery-0 (id=1)\t4%; charging; 2:00 remaining present: true\n",
        battery: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 1\n");
}

#[test]
fn helper_turns_sleep_back_on_when_hollow_is_gone_and_leaves_a_mac_on_power_awake() {
    // armed.999 is no Hollow (quit, or another program's pid).
    let after = pass(Pass {
        armed: armed(&["armed.999", "armed.12ab", "armed."]),
        disabled: true,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\n", "no sleepnow on power");
    assert!(!after.disabled && !after.marker);
}

#[test]
fn helper_never_touches_sleep_someone_else_disabled() {
    for names in [&[][..], &["armed.4242"][..]] {
        let after = pass(Pass {
            armed: armed(names),
            disabled: true,
            ..Pass::default()
        });
        assert_eq!(after.log, "", "nothing changed with {names:?} armed");
        assert!(after.disabled);
        assert!(!after.marker, "never claims sleep it didn't turn off");
    }
}

#[test]
fn helper_restores_with_its_marker_even_when_the_reading_says_on() {
    // The root domain reads No (someone turned sleep back on): the marker
    // still means turn it on, once, and no sleepnow.
    let after = pass(Pass {
        source: ON_BATTERY,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\n");
    assert!(!after.marker);
}

#[test]
fn helper_that_cant_read_sleep_disabled_never_turns_it_off_but_restores_with_its_marker() {
    for root in [Root::NoSleepLine, Root::Empty, Root::Fails] {
        // Armed on power: no marker over sleep someone else may have
        // disabled, and no disablesleep 1.
        let after = pass(Pass {
            armed: armed(&["armed.4242"]),
            root,
            ..Pass::default()
        });
        assert_eq!(after.log, "", "{root:?}");
        assert!(!after.marker, "{root:?}: claims nothing it can't read");
        assert!(after.ok && after.beat, "{root:?}: the pass finished");
        // Its marker, unplugged: sleep comes back on the marker alone, and
        // the Mac isn't put to sleep on a reading it doesn't have.
        let after = pass(Pass {
            armed: armed(&["armed.4242"]),
            source: ON_BATTERY,
            disabled: true,
            lid_closed: true,
            marker: true,
            root,
            ..Pass::default()
        });
        assert_eq!(after.log, "disablesleep 0\n", "{root:?}");
        assert!(!after.disabled && !after.marker && after.ok, "{root:?}");
        // Its marker on power with Hollow gone: the same.
        let after = pass(Pass {
            disabled: true,
            lid_closed: true,
            marker: true,
            root,
            ..Pass::default()
        });
        assert_eq!(after.log, "disablesleep 0\n", "{root:?}");
        assert!(!after.marker, "{root:?}");
        // Its marker while Hollow still asks: sleep stays off.
        let after = pass(Pass {
            armed: armed(&["armed.4242"]),
            disabled: true,
            marker: true,
            root,
            ..Pass::default()
        });
        assert_eq!(after.log, "", "{root:?}");
        assert!(after.marker && after.disabled, "{root:?}");
    }
}

#[test]
fn helper_records_only_the_passes_that_finished() {
    // `pmset` refuses: the pass fails before its last-pass file.
    let off = pass(Pass {
        armed: armed(&["armed.4242"]),
        pmset_fails: true,
        ..Pass::default()
    });
    assert!(!off.ok && !off.beat && !off.disabled);
    let on = pass(Pass {
        disabled: true,
        marker: true,
        pmset_fails: true,
        ..Pass::default()
    });
    assert!(!on.ok && !on.beat);
    assert!(on.marker, "the marker stays until sleep is back on");
    // A pass with nothing to change still records itself.
    let idle = pass(Pass::default());
    assert!(idle.ok && idle.beat && idle.log.is_empty());
}

#[test]
fn helper_reads_no_folder_through_a_link_and_writes_through_none() {
    let keep = |rig: &PassRig, name: &str| {
        let target = rig.state.join(name);
        std::fs::write(&target, "keep").unwrap();
        target
    };
    // The armed folder is a link to a folder holding Hollow's armed file:
    // no lookup through it, so with the marker sleep comes back; nothing
    // is written.
    let rig = PassRig::new(&Pass {
        disabled: true,
        marker: true,
        ..Pass::default()
    });
    let elsewhere = rig.state.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(elsewhere.join("armed.4242"), b"").unwrap();
    std::fs::remove_dir(&rig.armed_dir).unwrap();
    link(&elsewhere, &rig.armed_dir);
    let after = rig.run(Script::Helper);
    assert!(after.looked.is_empty(), "{:?}", after.looked);
    assert_eq!(after.log, "disablesleep 0\n");
    assert!(!after.marker && !after.ok && !after.beat);
    // The marker is a link: armed on power, the pass looks nothing up and
    // turns nothing off; it counts the link as its marker, turns sleep back
    // on and removes the link, never what it points at.
    let rig = PassRig::new(&Pass {
        armed: armed(&["armed.4242"]),
        ..Pass::default()
    });
    let target = keep(&rig, "marker-target");
    link(&target, &rig.mark);
    let after = rig.run(Script::Helper);
    assert!(after.looked.is_empty(), "{:?}", after.looked);
    assert_eq!(after.log, "disablesleep 0\n");
    assert!(!after.disabled && after.ok);
    assert!(
        std::fs::symlink_metadata(&rig.mark).is_err(),
        "the link went"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
    // The last-pass files are links: neither pass writes through its own.
    let rig = PassRig::new(&Pass::default());
    let (helper_target, restore_target) = (keep(&rig, "beat-target"), keep(&rig, "restore-target"));
    link(&helper_target, &rig.beat);
    link(&restore_target, &rig.restore_beat);
    assert!(!rig.run(Script::Helper).ok);
    assert!(!rig.run(Script::Restore).ok);
    assert_eq!(std::fs::read_to_string(&helper_target).unwrap(), "keep");
    assert_eq!(std::fs::read_to_string(&restore_target).unwrap(), "keep");

    // Parent directory replaced by a symlink: neither helper nor restore may
    // truncate or delete files inside the symlink target directory.
    let rig = PassRig::new(&Pass {
        armed: armed(&["armed.4242"]),
        source: ON_BATTERY,
        disabled: true,
        marker: false,
        ..Pass::default()
    });
    let victim = rig.state.join("victim-dir");
    std::fs::create_dir(&victim).unwrap();
    let victim_mark = victim.join("lid-closed-sleep-disabled");
    let victim_beat = victim.join("lid-closed-last-pass");
    let victim_rbeat = victim.join("lid-closed-restore-last-pass");
    std::fs::write(&victim_mark, "preserve-mark").unwrap();
    std::fs::write(&victim_beat, "preserve-beat").unwrap();
    std::fs::write(&victim_rbeat, "preserve-rbeat").unwrap();
    let sys_dir = rig.mark.parent().unwrap();
    std::fs::remove_dir_all(sys_dir).unwrap();
    link(&victim, sys_dir);
    assert!(!rig.run(Script::Helper).ok);
    assert!(!rig.run(Script::Restore).ok);
    assert_eq!(
        std::fs::read_to_string(&victim_mark).unwrap(),
        "preserve-mark"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_beat).unwrap(),
        "preserve-beat"
    );
    assert_eq!(
        std::fs::read_to_string(&victim_rbeat).unwrap(),
        "preserve-rbeat"
    );
}

#[test]
fn restore_turns_sleep_back_on_unplugged_with_the_marker_and_sleeps_a_closed_mac() {
    // Hollow's armed file is there and the Mac is closed on battery: the
    // restore check doesn't ask who armed it.
    let after = restore(Pass {
        armed: armed(&["armed.4242"]),
        source: ON_BATTERY,
        disabled: true,
        lid_closed: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled && !after.marker);
    assert!(after.looked.is_empty(), "no pid looked up");
    assert!(after.ok && after.restore_beat && !after.beat);
    // An open Mac: sleep back on, no sleepnow.
    let after = restore(Pass {
        source: ON_BATTERY,
        disabled: true,
        marker: true,
        ..Pass::default()
    });
    assert_eq!(after.log, "disablesleep 0\n");
    // A UPS, or no reading at all, is off power, as for the helper.
    for source in ["Now drawing from 'UPS Power'\n", ""] {
        let after = restore(Pass {
            source,
            disabled: true,
            marker: true,
            ..Pass::default()
        });
        assert_eq!(after.log, "disablesleep 0\n", "{source:?}");
    }
}

#[test]
fn restore_changes_nothing_on_power_with_on_battery_too_or_without_the_marker() {
    let cases = [
        // On power, sleep off on Hollow's marker: the helper's to decide.
        Pass {
            disabled: true,
            lid_closed: true,
            marker: true,
            ..Pass::default()
        },
        // On battery with On battery too.
        Pass {
            source: ON_BATTERY,
            battery: true,
            disabled: true,
            lid_closed: true,
            marker: true,
            ..Pass::default()
        },
        // On battery, sleep off by someone else.
        Pass {
            source: ON_BATTERY,
            disabled: true,
            lid_closed: true,
            ..Pass::default()
        },
    ];
    for case in cases {
        let marker = case.marker;
        let after = restore(case);
        assert_eq!(after.log, "");
        assert!(after.disabled);
        assert_eq!(after.marker, marker);
        assert!(after.ok && after.restore_beat, "every pass records itself");
    }
}

#[test]
fn restore_without_a_sleep_disabled_line_still_restores_but_never_sleeps_the_mac() {
    for root in [Root::NoSleepLine, Root::Empty, Root::Fails] {
        let after = restore(Pass {
            source: ON_BATTERY,
            disabled: true,
            lid_closed: true,
            marker: true,
            root,
            ..Pass::default()
        });
        assert_eq!(after.log, "disablesleep 0\n", "{root:?}");
        assert!(!after.marker && after.ok && after.restore_beat, "{root:?}");
    }
    // A pmset that refuses keeps the marker and records nothing.
    let after = restore(Pass {
        source: ON_BATTERY,
        disabled: true,
        marker: true,
        pmset_fails: true,
        ..Pass::default()
    });
    assert!(!after.ok && after.marker && !after.restore_beat);
}

#[test]
fn only_the_pass_that_removes_the_marker_sleeps_the_mac() {
    // Unplugged with the lid shut, the helper and the restore check both
    // find the marker; the other one removes it first, so this one turns
    // sleep back on but doesn't sleepnow as well (a second one could land
    // after the Mac wakes and put it straight back to sleep).
    for script in [Script::Helper, Script::Restore] {
        let rig = PassRig::new(&Pass {
            source: ON_BATTERY,
            disabled: true,
            lid_closed: true,
            marker: true,
            other_restores: true,
            ..Pass::default()
        });
        let after = rig.run(script);
        assert_eq!(after.log, "disablesleep 0\n");
        assert!(!after.disabled && !after.marker && after.ok);
    }
}

#[test]
fn restore_turns_sleep_back_on_unplugged_while_a_helper_pass_waits_on_a_lookup() {
    // The review's case: on power, lid shut, sleep off on Hollow's marker.
    // A process of the person's runs a program from a share it serves,
    // names its pid before Hollow's, and the share stops answering.
    let rig = PassRig::new(&Pass {
        armed: armed(&["armed.1000", "armed.4242"]),
        disabled: true,
        lid_closed: true,
        marker: true,
        exes: vec![(1000, "/Volumes/share/tool"), (4242, EXE)],
        during: vec![(1000, During::Hang)],
        ..Pass::default()
    });
    let mut helper = Running(rig.command(Script::Helper).spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(20);
    while !rig.looked().iter().any(|l| l == "lsof 1000") {
        assert!(Instant::now() < deadline, "the helper pass never looked");
        std::thread::sleep(Duration::from_millis(20));
    }
    // Alex unplugs and bags the Mac; launchd starts no other helper
    // pass while this one waits, but the restore check is its own job.
    rig.set_source(ON_BATTERY);
    let started = Instant::now();
    let after = rig.run(Script::Restore);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the restore check waits on no lookup"
    );
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n");
    assert!(!after.disabled && !after.marker);
    assert!(after.ok && after.restore_beat);
    assert_eq!(after.looked, vec!["lsof 1000"], "it looked nothing up");
    assert!(
        matches!(helper.0.try_wait(), Ok(None)),
        "the helper pass still waits"
    );
    // The share answers at last: the helper pass finds Hollow, reads the
    // power source again and leaves sleep on.
    rig.release();
    let ok = helper.0.wait().unwrap().success();
    let after = rig.after(ok);
    assert_eq!(after.looked, vec!["lsof 1000", "lsof 4242"]);
    assert_eq!(after.log, "disablesleep 0\nsleepnow\n", "nothing more");
    assert!(!after.disabled && !after.marker && after.ok && after.beat);
}

#[cfg(target_os = "macos")]
#[test]
fn own_exe_is_the_path_the_helpers_lsof_reads_for_this_process() {
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-b", "-w", "-a", "-p"])
        .arg(std::process::id().to_string())
        .args(["-d", "txt", "-Fn"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let first = text.lines().find_map(|line| line.strip_prefix('n'));
    assert_eq!(first, Some(own_exe().as_str()));
}

// ── The plists ───────────────────────────────────────────────────────────

fn rig_paths(root: &Path) -> Paths {
    let support = root.join("Root's & <Support>");
    let daemons = root.join("LaunchDaemons");
    Paths {
        plist: daemons.join(format!("{LABEL}.plist")),
        restore_plist: daemons.join(format!("{RESTORE_LABEL}.plist")),
        marker: support.join("lid-closed-sleep-disabled"),
        beat: support.join("lid-closed-last-pass"),
        restore_beat: support.join("lid-closed-restore-last-pass"),
        armed: support.join("lid-closed-armed"),
    }
}

fn strings(value: &plist::Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_string().unwrap().to_string())
        .collect()
}

#[test]
fn plist_round_trips_its_arguments_and_runs_every_ten_seconds() {
    let dir = temp("plist");
    let paths = rig_paths(dir.path());
    let xml = helper_plist(&paths, EXE, true).unwrap();
    let text = String::from_utf8(xml.clone()).unwrap();
    assert!(text.contains("<string>com.hollow.app.lid-closed</string>"));
    assert!(text.contains("&amp; &lt;Support&gt;"), "XML-escaped");
    std::fs::create_dir_all(paths.plist.parent().unwrap()).unwrap();
    std::fs::write(&paths.plist, &xml).unwrap();
    let installed = read_installed(&paths.plist).unwrap();
    assert_eq!(installed.exe, EXE);
    assert!(installed.on_battery);
    assert_eq!(Path::new(&installed.armed), paths.armed.as_path());
    let value = plist::Value::from_file(&paths.plist).unwrap();
    let dict = value.as_dictionary().unwrap();
    let args = dict.get("ProgramArguments").unwrap().as_array().unwrap();
    assert_eq!(args[2].as_string(), Some(HELPER));
    assert_eq!(
        args[7].as_string().map(PathBuf::from),
        Some(paths.marker.clone())
    );
    assert_eq!(
        args[8].as_string().map(PathBuf::from),
        Some(paths.beat.clone())
    );
    assert_eq!(
        dict.get("RunAtLoad").and_then(|v| v.as_boolean()),
        Some(true)
    );
    assert_eq!(
        dict.get("StartInterval")
            .and_then(|v| v.as_signed_integer()),
        Some(TICK_SECS)
    );
    let watch = dict.get("WatchPaths").unwrap().as_array().unwrap();
    assert_eq!(
        watch[0].as_string().map(PathBuf::from),
        Some(paths.armed.clone())
    );
    assert!(
        same_plist(&paths.plist, &xml),
        "the same plist reads the same"
    );
    assert!(!same_plist(
        &paths.plist,
        &helper_plist(&paths, EXE, false).unwrap()
    ));
    // Not Hollow's label: not Hollow's helper.
    std::fs::write(&paths.plist, text.replace(LABEL, "com.example.other")).unwrap();
    assert_eq!(read_installed(&paths.plist), None);
}

#[test]
fn the_restore_checks_plist_names_no_folder_and_no_hollow_and_runs_at_load_and_every_ten_seconds() {
    let dir = temp("restore-plist");
    let paths = rig_paths(dir.path());
    for on_battery in [false, true] {
        let xml = restore_plist(&paths, on_battery).unwrap();
        let value = plist::Value::from_reader(std::io::Cursor::new(&xml)).unwrap();
        let dict = value.as_dictionary().unwrap();
        assert_eq!(
            dict.get("Label").and_then(|v| v.as_string()),
            Some(RESTORE_LABEL)
        );
        let marker = paths.marker.to_string_lossy().into_owned();
        let beat = paths.restore_beat.to_string_lossy().into_owned();
        assert_eq!(
            strings(dict.get("ProgramArguments").unwrap()),
            vec![
                "/bin/sh",
                "-c",
                RESTORE,
                "hollow-lid-closed-restore",
                if on_battery { "1" } else { "0" },
                marker.as_str(),
                beat.as_str(),
            ]
        );
        assert_eq!(
            dict.get("RunAtLoad").and_then(|v| v.as_boolean()),
            Some(true)
        );
        assert_eq!(
            dict.get("StartInterval")
                .and_then(|v| v.as_signed_integer()),
            Some(TICK_SECS)
        );
        assert!(dict.get("WatchPaths").is_none(), "it watches no path");
        assert_eq!(
            strings(dict.get("AssociatedBundleIdentifiers").unwrap()),
            vec!["com.hollow.app"]
        );
    }
    assert!(
        !RESTORE.contains("lsof") && !RESTORE.contains("armed") && !RESTORE.contains("/bin/ps"),
        "no folder and no pid"
    );
}

#[test]
fn the_system_paths_keep_the_armed_folder_in_roots_folder() {
    let paths = Paths::system();
    let root = paths.root_dir();
    assert_eq!(root, Path::new("/var/db/hollow"));
    assert_eq!(paths.marker.parent(), Some(root));
    assert_eq!(paths.armed.parent(), Some(root));
    assert_eq!(paths.beat.parent(), Some(root));
    assert_eq!(paths.restore_beat.parent(), Some(root));
    assert_eq!(
        paths.restore_plist,
        Path::new("/Library/LaunchDaemons").join(format!("{RESTORE_LABEL}.plist"))
    );
    assert_eq!(
        paths.plist,
        Path::new("/Library/LaunchDaemons").join(format!("{LABEL}.plist"))
    );
}

// ── The root commands, run against fake root tools ───────────────────────

/// The install and uninstall commands run for real against a temp folder,
/// with fake `stat` (it prints the owner in `owner`: root's 0 unless a test
/// says otherwise) and fake `chown`, `chmod`, `launchctl` and `pmset`,
/// which only log what they were asked (a bootstrap needs its plist there).
struct RootTools {
    _dir: tempfile::TempDir,
    fake: PathBuf,
    state: PathBuf,
}

impl RootTools {
    fn new() -> Self {
        let dir = temp("root");
        let (fake, state) = (dir.path().join("fake"), dir.path().join("state"));
        for d in [&fake, &state] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(state.join("owner"), "0\n").unwrap();
        std::fs::write(state.join("log"), "").unwrap();
        let s = sh_quote(&state);
        write_tool(&fake.join("stat"), &format!("s={s}\ncat \"$s/owner\"\n"));
        write_tool(
            &fake.join("launchctl"),
            &format!("s={s}\necho \"launchctl $*\" >> \"$s/log\"\n[ \"$1\" != bootstrap ] || [ -f \"$3\" ]\n"),
        );
        for tool in ["chown", "chmod", "pmset"] {
            write_tool(
                &fake.join(tool),
                &format!("s={s}\necho \"{tool} $*\" >> \"$s/log\"\n"),
            );
        }
        Self {
            _dir: dir,
            fake,
            state,
        }
    }

    /// The owner `stat` gives Hollow's folder.
    fn owner(&self, uid: u32) {
        std::fs::write(self.state.join("owner"), format!("{uid}\n")).unwrap();
    }

    /// Run a root command as `osascript` hands it to `sh`: whether it
    /// exited 0, and what it said on stderr.
    fn run(&self, command: &str) -> (bool, String) {
        let mut script = command.to_string();
        for (real, name) in [
            ("/usr/bin/stat", "stat"),
            ("/usr/sbin/chown", "chown"),
            ("/bin/chmod", "chmod"),
            ("/bin/launchctl", "launchctl"),
            ("/usr/bin/pmset", "pmset"),
        ] {
            script = script.replace(real, &sh_quote(&self.fake.join(name)));
            assert!(!script.contains(real), "every {real} is the fake");
        }
        let out = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg(&script)
            .output()
            .unwrap();
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.state.join("log"))
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect()
    }
}

/// The base64 plists an install command carries, in order.
fn carried_plists(command: &str) -> Vec<Vec<u8>> {
    command
        .split("printf %s '")
        .skip(1)
        .map(|rest| STANDARD.decode(&rest[..rest.find('\'').unwrap()]).unwrap())
        .collect()
}

/// This Hollow's install for `paths`: Hollow at [`EXE`], On battery too off.
fn install(paths: &Paths) -> String {
    install_command(
        paths,
        &helper_plist(paths, EXE, false).unwrap(),
        &restore_plist(paths, false).unwrap(),
    )
}

/// A temp `/Library` (its LaunchDaemons folder only) and fake root tools.
fn root_rig() -> (tempfile::TempDir, Paths, RootTools) {
    let dir = temp("library");
    let paths = rig_paths(dir.path());
    std::fs::create_dir_all(paths.plist.parent().unwrap()).unwrap();
    (dir, paths, RootTools::new())
}

#[test]
fn install_command_carries_both_plists_and_keeps_every_path_quoted() {
    let (_dir, paths, _) = root_rig();
    let helper = helper_plist(&paths, EXE, false).unwrap();
    let restore = restore_plist(&paths, false).unwrap();
    let command = install_command(&paths, &helper, &restore);
    assert_eq!(carried_plists(&command), vec![helper, restore]);
    assert!(
        !command.contains("add_subdirectory"),
        "no subfolders, so nothing can be mounted in it"
    );
    assert!(
        command.contains(r"Root'\''s & <Support>"),
        "paths stay quoted for sh: {command}"
    );
    assert!(!command.contains('\n'));
    assert!(
        !command.contains("disablesleep"),
        "installing never touches sleep"
    );
    // The folder check comes first, before anything is made.
    assert!(command.starts_with(&format!("if [ -L {} ]", sh_quote(paths.root_dir()))));
}

#[test]
fn install_makes_hollows_folder_roots_then_loads_the_restore_check_and_the_helper() {
    let (_dir, paths, tools) = root_rig();
    let (ok, said) = tools.run(&install(&paths));
    assert!(ok, "{said}");
    assert_eq!(
        std::fs::read(&paths.plist).unwrap(),
        helper_plist(&paths, EXE, false).unwrap()
    );
    assert_eq!(
        std::fs::read(&paths.restore_plist).unwrap(),
        restore_plist(&paths, false).unwrap()
    );
    assert!(paths.armed.is_dir() && paths.beat.is_file() && paths.restore_beat.is_file());
    let (root_dir, armed) = (paths.root_dir().display(), paths.armed.display());
    let (target, restore_target) = (paths.plist.display(), paths.restore_plist.display());
    assert_eq!(
        tools.log(),
        vec![
            format!("chown root:wheel {root_dir}"),
            format!("chmod 755 {root_dir}"),
            format!("chmod -N {root_dir}"),
            format!("chown root:wheel {armed}"),
            format!("chmod 755 {armed}"),
            format!("chmod -N {armed}"),
            format!(
                "chmod +a user:{} allow add_file,delete_child {armed}",
                String::from_utf8(
                    std::process::Command::new("/usr/bin/id")
                        .args(["-un", &current_uid().to_string()])
                        .output()
                        .unwrap()
                        .stdout
                )
                .unwrap()
                .trim()
            ),
            format!("chown root:wheel {target}.new {restore_target}.new"),
            format!("chmod 644 {target}.new {restore_target}.new"),
            "launchctl bootout system/com.hollow.app.lid-closed".to_string(),
            "launchctl bootout system/com.hollow.app.lid-closed-restore".to_string(),
            format!("launchctl bootstrap system {restore_target}"),
            format!("launchctl bootstrap system {target}"),
        ],
        "the restore check loads before the helper"
    );
    // A root-owned folder that is already there is fine too (a reinstall).
    let (ok, said) = tools.run(&install(&paths));
    assert!(ok, "{said}");
}

#[test]
fn install_goes_ahead_only_when_hollows_folder_is_absent_or_a_real_folder_of_roots() {
    let refused = |paths: &Paths, tools: &RootTools| {
        let (ok, said) = tools.run(&install(paths));
        assert!(!ok, "refused");
        assert!(said.contains(NOT_ROOTS), "{said}");
        assert_eq!(
            tools.log(),
            Vec::<String>::new(),
            "nothing changed or loaded"
        );
        assert!(!paths.plist.exists() && !paths.restore_plist.exists());
    };
    // Someone else's folder.
    let (_dir, paths, tools) = root_rig();
    std::fs::create_dir_all(paths.root_dir()).unwrap();
    tools.owner(501);
    refused(&paths, &tools);
    assert!(!paths.armed.exists());
    // A link to a folder (root's or not).
    let (dir, paths, tools) = root_rig();
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    link(&elsewhere, paths.root_dir());
    refused(&paths, &tools);
    assert_eq!(
        std::fs::read_dir(&elsewhere).unwrap().count(),
        0,
        "nothing made through it"
    );
    // A link to nowhere, and a file.
    let (dir, paths, tools) = root_rig();
    link(&dir.path().join("nowhere"), paths.root_dir());
    refused(&paths, &tools);
    let (_dir, paths, tools) = root_rig();
    std::fs::write(paths.root_dir(), b"").unwrap();
    refused(&paths, &tools);
}

#[test]
fn install_refuses_a_link_where_the_helper_keeps_its_files() {
    for which in ["armed folder", "marker", "last pass", "restore's last pass"] {
        let (dir, paths, tools) = root_rig();
        std::fs::create_dir_all(paths.root_dir()).unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, "keep").unwrap();
        let at = match which {
            "armed folder" => &paths.armed,
            "marker" => &paths.marker,
            "last pass" => &paths.beat,
            _ => &paths.restore_beat,
        };
        link(&target, at);
        let (ok, said) = tools.run(&install(&paths));
        assert!(!ok, "{which}: refused");
        assert!(said.contains(A_LINK), "{which}: {said}");
        assert!(
            !tools
                .log()
                .iter()
                .any(|l| l.starts_with("launchctl") || l.contains("lid-closed-armed")),
            "{which}: nothing loaded, and the armed folder untouched: {:?}",
            tools.log()
        );
        assert!(
            !paths.plist.exists() && !paths.restore_plist.exists(),
            "{which}"
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep", "{which}");
    }
}

#[test]
fn uninstall_unloads_both_before_anything_goes_and_turns_sleep_on_only_with_the_marker() {
    let (_dir, paths, tools) = root_rig();
    let command = uninstall_command(&paths);
    let mark = sh_quote(&paths.marker);
    let at = |text: &str| {
        command
            .find(text)
            .unwrap_or_else(|| panic!("{text} in {command}"))
    };
    let helper_out = at("/bin/launchctl bootout system/com.hollow.app.lid-closed 2>");
    let restore_out = at("/bin/launchctl bootout system/com.hollow.app.lid-closed-restore 2>");
    let plists = at(&format!(
        "/bin/rm -f {} {} {} {}",
        sh_quote(&paths.plist),
        fresh(&paths.plist),
        sh_quote(&paths.restore_plist),
        fresh(&paths.restore_plist)
    ));
    let folder = at(&format!("/bin/rm -rf {}", sh_quote(&paths.armed)));
    let beats = at(&format!(
        "/bin/rm -f {} {}",
        sh_quote(&paths.beat),
        sh_quote(&paths.restore_beat)
    ));
    let restore = at(&format!(
        "if [ -f {mark} ]; then /usr/bin/pmset -a disablesleep 0 && /bin/rm -f {mark} || bad=1; fi"
    ));
    assert!(
        helper_out < restore_out
            && restore_out < plists
            && plists < restore
            && restore < folder
            && folder < beats,
        "both stop, sleep comes back, then the folder and the beats go"
    );
    assert_eq!(command.matches("disablesleep").count(), 1);
    // Run for real after an install: with the marker, sleep comes back.
    assert!(tools.run(&install(&paths)).0);
    std::fs::write(&paths.marker, b"").unwrap();
    let before = tools.log().len();
    let (ok, said) = tools.run(&command);
    assert!(ok, "{said}");
    assert_eq!(
        tools.log()[before..].to_vec(),
        vec![
            "launchctl bootout system/com.hollow.app.lid-closed".to_string(),
            "launchctl bootout system/com.hollow.app.lid-closed-restore".to_string(),
            "pmset -a disablesleep 0".to_string(),
        ]
    );
    for gone in [
        &paths.plist,
        &paths.restore_plist,
        &paths.armed,
        &paths.beat,
        &paths.restore_beat,
        &paths.marker,
    ] {
        assert!(!gone.exists(), "{} is gone", gone.display());
    }
    assert!(!paths.root_dir().exists(), "and Hollow's folder with them");
    // Without the marker, sleep is left as it is.
    assert!(tools.run(&install(&paths)).0);
    let before = tools.log().len();
    assert!(tools.run(&command).0);
    assert!(!tools.log()[before..].iter().any(|l| l.starts_with("pmset")));
}

#[test]
fn uninstall_removes_the_folder_and_beats_even_when_pmset_fails_and_says_so() {
    let (_dir, paths, tools) = root_rig();
    assert!(tools.run(&install(&paths)).0);
    std::fs::write(&paths.marker, b"").unwrap();
    write_tool(&tools.fake.join("pmset"), "exit 1\n");
    let (ok, _) = tools.run(&uninstall_command(&paths));
    assert!(!ok, "a failed restore is reported");
    for gone in [&paths.armed, &paths.beat, &paths.restore_beat, &paths.plist] {
        assert!(!gone.exists(), "{} is gone", gone.display());
    }
    assert!(
        paths.marker.exists(),
        "the marker stays so the restore can be retried"
    );
}

#[test]
fn uninstall_refuses_symlinked_or_non_root_folders_without_deleting_targets() {
    // Symlinked root_dir must never be traversed or deleted.
    let (dir, paths, tools) = root_rig();
    let victim = dir.path().join("victim");
    std::fs::create_dir_all(victim.join("lid-closed-armed")).unwrap();
    let keep_file = victim.join("lid-closed-armed/important.txt");
    std::fs::write(&keep_file, "keep").unwrap();
    link(&victim, paths.root_dir());
    let (ok, said) = tools.run(&uninstall_command(&paths));
    assert!(!ok, "symlinked root_dir must fail");
    assert!(said.contains(NOT_ROOTS), "{said}");
    assert_eq!(std::fs::read_to_string(&keep_file).unwrap(), "keep");

    // Non-root-owned root_dir must be refused untouched.
    let (_dir, paths, tools) = root_rig();
    std::fs::create_dir_all(&paths.armed).unwrap();
    let keep_file = paths.armed.join("important.txt");
    std::fs::write(&keep_file, "keep").unwrap();
    tools.owner(501);
    let (ok, said) = tools.run(&uninstall_command(&paths));
    assert!(!ok, "non-root root_dir must fail");
    assert!(said.contains(NOT_ROOTS), "{said}");
    assert_eq!(std::fs::read_to_string(&keep_file).unwrap(), "keep");

    // Symlinked armed dir inside a root-owned root_dir must never be rm -rf'd.
    let (dir, paths, tools) = root_rig();
    std::fs::create_dir_all(paths.root_dir()).unwrap();
    let victim_armed = dir.path().join("victim-armed");
    std::fs::create_dir_all(&victim_armed).unwrap();
    let keep_armed = victim_armed.join("keep.txt");
    std::fs::write(&keep_armed, "keep").unwrap();
    link(&victim_armed, &paths.armed);
    let (ok, said) = tools.run(&uninstall_command(&paths));
    assert!(!ok, "symlinked armed dir must fail");
    assert!(said.contains(A_LINK), "{said}");
    assert_eq!(std::fs::read_to_string(&keep_armed).unwrap(), "keep");
}

#[test]
fn applescript_escapes_quotes_and_backslashes() {
    let script = applescript(r#"echo "a\b""#, "Say \"hi\"");
    assert_eq!(
        script,
        r#"do shell script "echo \"a\\b\"" with administrator privileges with prompt "Say \"hi\"""#
    );
}

#[test]
fn a_cancelled_prompt_reads_as_nothing_changed() {
    assert_eq!(
        admin_error("0:131: execution error: User canceled. (-128)\n"),
        "You cancelled macOS’s password prompt, so nothing changed."
    );
    assert_eq!(
        admin_error("execution error: launchctl failed (1)"),
        "macOS didn’t make the change: execution error: launchctl failed (1)"
    );
    assert_eq!(admin_error(""), "macOS didn’t make the change.");
}

// ── Readings (this Mac's own output, 2026-11-05) ─────────────────────────

#[test]
fn reads_the_power_source_and_charge() {
    assert_eq!(parse_source(ON_POWER), (Source::Power, Some(100)));
    assert_eq!(parse_source(ON_BATTERY), (Source::Battery, Some(87)));
    assert_eq!(
        parse_source("Now drawing from 'AC Power'\n"),
        (Source::Power, None),
        "a desktop Mac has no battery"
    );
    assert_eq!(
        parse_source("Now drawing from 'UPS Power'\n"),
        (Source::Unknown, None)
    );
    assert_eq!(parse_source(""), (Source::Unknown, None));
}

#[test]
fn reads_the_root_domain_flags() {
    let out = "      \"AppleClamshellCausesSleep\" = Yes\n      \"AppleClamshellState\" = No\n      \"SleepDisabled\" = No\n";
    assert_eq!(flag(out, "SleepDisabled"), Some(false));
    assert_eq!(flag(out, "AppleClamshellState"), Some(false));
    assert_eq!(flag(out, "AppleClamshellCausesSleep"), Some(true));
    assert_eq!(flag("\"SleepDisabled\" = Yes", "SleepDisabled"), Some(true));
    assert_eq!(flag(out, "Missing"), None);
    assert_eq!(flag("\"SleepDisabled\" = 1", "SleepDisabled"), None);
}

// ── The lid with fakes ───────────────────────────────────────────────────

#[derive(Default)]
struct FakePower {
    root: Mutex<Option<String>>,
    source: Mutex<Option<String>>,
    /// How many times the power source was read (`pmset -g ps` started).
    reads: AtomicUsize,
}

impl FakePower {
    fn set(&self, root: Option<&str>, source: Option<&str>) {
        *self.root.lock().unwrap() = root.map(str::to_string);
        *self.source.lock().unwrap() = source.map(str::to_string);
    }
}

impl Power for Arc<FakePower> {
    fn root_domain(&self) -> Option<String> {
        self.root.lock().unwrap().clone()
    }

    fn source(&self) -> Option<String> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.source.lock().unwrap().clone()
    }
}

/// Records each prompt, then runs the root command for real against the
/// rig's temp folder with fake root tools ([`RootTools`]): an install makes
/// the armed folder and both last-pass files and writes both plists; an
/// uninstall removes them and, with the marker, turns sleep back on.
struct FakeAdmin {
    tools: RootTools,
    runs: Mutex<Vec<(String, String)>>,
    fail: Mutex<Option<String>>,
}

impl Admin for Arc<FakeAdmin> {
    fn run(&self, command: &str, why: &str) -> Result<(), String> {
        self.runs
            .lock()
            .unwrap()
            .push((command.to_string(), why.to_string()));
        if let Some(error) = self.fail.lock().unwrap().take() {
            return Err(error);
        }
        match self.tools.run(command) {
            (true, _) => Ok(()),
            (false, said) => Err(admin_error(&said)),
        }
    }
}

#[derive(Default)]
struct FakeHold(AtomicBool);

impl Hold for Arc<FakeHold> {
    fn set(&self, on: bool) -> bool {
        self.0.store(on, Ordering::SeqCst);
        on
    }
}

struct Rig {
    _dir: tempfile::TempDir,
    paths: Paths,
    power: Arc<FakePower>,
    admin: Arc<FakeAdmin>,
    hold: Arc<FakeHold>,
    /// (the iPhone app is on, a phone is paired too)
    phone: Arc<Mutex<(bool, bool)>>,
    lid: Lid,
}

fn rig() -> Rig {
    let dir = temp("rig");
    let paths = rig_paths(dir.path());
    // /Library/LaunchDaemons is there; Hollow's folder isn't.
    std::fs::create_dir_all(paths.plist.parent().unwrap()).unwrap();
    let power = Arc::new(FakePower::default());
    power.set(
        Some("\"SleepDisabled\" = No\n\"AppleClamshellState\" = No\n"),
        Some(ON_POWER),
    );
    let admin = Arc::new(FakeAdmin {
        tools: RootTools::new(),
        runs: Mutex::new(Vec::new()),
        fail: Mutex::new(None),
    });
    let hold = Arc::new(FakeHold::default());
    let phone = Arc::new(Mutex::new((true, true)));
    let state = phone.clone();
    let lid = Lid::new(
        paths.clone(),
        Box::new(power.clone()),
        Box::new(admin.clone()),
        Box::new(hold.clone()),
        4242,
        EXE.to_string(),
        Box::new(move || *state.lock().unwrap()),
    );
    Rig {
        _dir: dir,
        paths,
        power,
        admin,
        hold,
        phone,
        lid,
    }
}

impl Rig {
    fn armed_files(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.paths.armed)
            .map(|entries| {
                entries
                    .flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn prompts(&self) -> Vec<String> {
        self.admin
            .runs
            .lock()
            .unwrap()
            .iter()
            .map(|(_, why)| why.clone())
            .collect()
    }

    /// Set a last-pass file's time `ago` before now.
    fn passed(&self, file: &Path, ago: Duration) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"").unwrap();
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(SystemTime::now() - ago)
            .unwrap();
    }

    /// Both plists are the two this Hollow writes for Hollow at [`EXE`].
    fn current(&self, on_battery: bool) -> bool {
        same_plist(
            &self.paths.plist,
            &helper_plist(&self.paths, EXE, on_battery).unwrap(),
        ) && same_plist(
            &self.paths.restore_plist,
            &restore_plist(&self.paths, on_battery).unwrap(),
        )
    }
}

#[test]
fn off_by_default_nothing_is_armed_or_held() {
    let r = rig();
    let status = r.lid.status();
    assert!(status.available && !status.enabled && !status.on_battery);
    assert!(!status.armed && !status.holding && !status.outdated);
    assert!(r.armed_files().is_empty());
    assert!(!r.hold.0.load(Ordering::SeqCst));
    assert_eq!(status.source, Source::Power);
    assert_eq!(status.sleep_disabled, Some(false));
    assert_eq!(status.last_pass_secs, None, "no helper, no pass");
    assert_eq!(status.last_restore_secs, None);
    assert!(
        !r.paths.armed.exists(),
        "Hollow never makes the armed folder"
    );
    assert!(r.prompts().is_empty(), "reading never prompts");
}

#[test]
fn turning_it_on_needs_the_iphone_app() {
    let r = rig();
    *r.phone.lock().unwrap() = (false, false);
    let error = r.lid.set(true, false).unwrap_err();
    assert!(error.starts_with("Turn on the iPhone app first"), "{error}");
    assert!(r.prompts().is_empty(), "no prompt for a refused click");
}

#[test]
fn with_the_setting_off_the_sync_starts_no_process() {
    let r = rig();
    for _ in 0..3 {
        assert_eq!(r.lid.sync(), Synced::default());
    }
    assert_eq!(r.power.reads.load(Ordering::SeqCst), 0, "no pmset -g ps");
    r.lid.set(true, false).unwrap();
    let before = r.power.reads.load(Ordering::SeqCst);
    assert!(r.lid.sync().holding);
    assert_eq!(
        r.power.reads.load(Ordering::SeqCst),
        before + 1,
        "armed: one read"
    );
    // Settings' reading reads it once, not once for the sync and again.
    let before = r.power.reads.load(Ordering::SeqCst);
    r.lid.status();
    assert_eq!(r.power.reads.load(Ordering::SeqCst), before + 1);
}

#[test]
fn turned_on_before_a_phone_pairs_the_watched_folder_exists_and_nothing_is_armed() {
    let r = rig();
    *r.phone.lock().unwrap() = (true, false);
    r.lid.set(true, false).unwrap();
    assert!(
        r.paths.armed.is_dir(),
        "the install made the folder launchd's WatchPaths watches"
    );
    assert!(r.armed_files().is_empty());
    let status = r.lid.status();
    assert!(status.enabled && !status.ready && !status.armed && !status.holding);
    *r.phone.lock().unwrap() = (true, true);
    assert!(r.lid.sync().armed, "the phone pairs: the next sync arms it");
    assert_eq!(r.prompts().len(), 1);
}

#[test]
fn on_installs_both_with_one_prompt_arms_the_helper_and_holds_idle_sleep_off() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    assert_eq!(r.prompts(), vec![WHY_ON.to_string()]);
    assert!(r.current(false), "the helper and its restore check");
    let status = r.lid.status();
    assert!(status.enabled && !status.on_battery && !status.outdated);
    assert!(status.armed && status.holding);
    assert!(status.last_pass_secs.is_some() && status.last_restore_secs.is_some());
    assert_eq!(r.armed_files(), vec!["armed.4242".to_string()]);
    assert_eq!(status.other_copy, None);
    // The same choice again changes nothing and asks nothing.
    r.lid.set(true, false).unwrap();
    assert_eq!(r.prompts().len(), 1);
}

#[test]
fn on_battery_the_assertion_goes_unless_on_battery_too() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    r.power
        .set(Some("\"SleepDisabled\" = Yes\n"), Some(ON_BATTERY));
    let status = r.lid.status();
    assert!(status.armed, "the helper decides about the lid on battery");
    assert!(!status.holding, "no idle-sleep hold on battery");
    assert_eq!(status.source, Source::Battery);
    assert_eq!(status.battery_percent, Some(87));
    // Ticking On battery too reinstalls both, with one prompt of its own.
    r.lid.set(true, true).unwrap();
    assert_eq!(
        r.prompts(),
        vec![WHY_ON.to_string(), WHY_CHANGE.to_string()]
    );
    assert!(r.current(true), "the restore check knows On battery too");
    let status = r.lid.status();
    assert!(status.on_battery && status.holding);
}

#[test]
fn the_iphone_app_off_or_the_last_phone_revoked_disarms_without_a_prompt() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    assert!(r.lid.sync().armed);
    *r.phone.lock().unwrap() = (true, false);
    assert_eq!(r.lid.sync(), Synced::default(), "the last phone revoked");
    assert!(r.armed_files().is_empty());
    *r.phone.lock().unwrap() = (true, true);
    assert!(r.lid.sync().armed, "a phone pairs again: armed again");
    *r.phone.lock().unwrap() = (false, false);
    assert_eq!(r.lid.sync(), Synced::default(), "the iPhone app off");
    assert!(!r.hold.0.load(Ordering::SeqCst));
    assert_eq!(r.prompts().len(), 1, "only the first click prompted");
    // The setting stays on, and turning it off still works.
    assert!(r.lid.status().enabled);
    r.lid.set(false, false).unwrap();
    assert!(!r.lid.status().enabled);
}

#[test]
fn off_removes_both_with_one_prompt_turns_sleep_back_on_and_disarms() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    std::fs::write(&r.paths.marker, b"").unwrap();
    let before = r.admin.tools.log().len();
    r.lid.set(false, false).unwrap();
    let runs = r.admin.runs.lock().unwrap().clone();
    assert_eq!(runs.len(), 2);
    assert_eq!(runs[1], (uninstall_command(&r.paths), WHY_OFF.to_string()));
    assert_eq!(
        r.admin.tools.log()[before..].to_vec(),
        vec![
            "launchctl bootout system/com.hollow.app.lid-closed".to_string(),
            "launchctl bootout system/com.hollow.app.lid-closed-restore".to_string(),
            "pmset -a disablesleep 0".to_string(),
        ]
    );
    let status = r.lid.status();
    assert!(!status.enabled && !status.armed && !status.holding);
    assert!(!status.disabled_by_hollow);
    assert_eq!(status.last_pass_secs, None);
    assert_eq!(status.last_restore_secs, None);
    assert!(!r.paths.plist.exists() && !r.paths.restore_plist.exists());
    assert!(!r.paths.armed.exists(), "its folder went with it");
}

#[test]
fn a_cancelled_prompt_changes_nothing() {
    let r = rig();
    *r.admin.fail.lock().unwrap() = Some(admin_error("User canceled. (-128)"));
    let error = r.lid.set(true, false).unwrap_err();
    assert_eq!(
        error,
        "You cancelled macOS’s password prompt, so nothing changed."
    );
    let status = r.lid.status();
    assert!(!status.enabled && !status.armed && !status.holding);
    assert!(r.armed_files().is_empty());
}

#[test]
fn an_install_refused_as_root_says_why_and_changes_nothing() {
    let r = rig();
    // Hollow's folder is someone else's.
    std::fs::create_dir_all(r.paths.root_dir()).unwrap();
    r.admin.tools.owner(501);
    let error = r.lid.set(true, false).unwrap_err();
    assert!(error.contains(NOT_ROOTS), "{error}");
    let status = r.lid.status();
    assert!(!status.enabled && !status.armed);
}

#[test]
fn one_change_at_a_time() {
    let r = rig();
    let _open = r.lid.changing.lock().unwrap();
    let error = r.lid.set(true, false).unwrap_err();
    assert_eq!(
        error,
        "macOS is already asking for your password. Answer that prompt first."
    );
    assert!(r.prompts().is_empty());
}

#[test]
fn stale_armed_files_from_an_earlier_launch_go() {
    let r = rig();
    *r.phone.lock().unwrap() = (true, false);
    r.lid.set(true, false).unwrap();
    std::fs::write(r.paths.armed.join("armed.17"), b"").unwrap();
    std::fs::write(r.paths.armed.join("notes.txt"), b"").unwrap();
    r.lid.sync();
    assert_eq!(
        r.armed_files(),
        vec!["notes.txt".to_string()],
        "no phone: nothing armed"
    );
    std::fs::write(r.paths.armed.join("armed.17"), b"").unwrap();
    *r.phone.lock().unwrap() = (true, true);
    r.lid.sync();
    assert_eq!(
        r.armed_files(),
        vec!["armed.4242".to_string(), "notes.txt".to_string()]
    );
}

#[test]
fn says_whose_sleep_setting_it_is() {
    let r = rig();
    // Someone else turned system sleep off: Hollow didn't.
    r.power.set(
        Some("\"SleepDisabled\" = Yes\n\"AppleClamshellState\" = Yes\n"),
        Some(ON_POWER),
    );
    let status = r.lid.status();
    assert_eq!(status.sleep_disabled, Some(true));
    assert_eq!(status.lid_closed, Some(true));
    assert!(!status.disabled_by_hollow);
    // The helper's marker: Hollow did.
    std::fs::create_dir_all(r.paths.marker.parent().unwrap()).unwrap();
    std::fs::write(&r.paths.marker, b"").unwrap();
    assert!(r.lid.status().disabled_by_hollow);
    // Unreadable readings stay unknown, never "on".
    r.power.set(None, None);
    let status = r.lid.status();
    assert_eq!(status.sleep_disabled, None);
    assert_eq!(status.source, Source::Unknown);
}

#[test]
fn a_helper_set_up_for_another_copy_says_so_and_a_click_rearms_it_here() {
    let r = rig();
    std::fs::create_dir_all(&r.paths.armed).unwrap();
    std::fs::write(
        &r.paths.plist,
        helper_plist(&r.paths, "/Users/dev/dev/target/debug/hollow", false).unwrap(),
    )
    .unwrap();
    std::fs::write(
        &r.paths.restore_plist,
        restore_plist(&r.paths, false).unwrap(),
    )
    .unwrap();
    // That copy's own armed file.
    std::fs::write(r.paths.armed.join("armed.777"), b"").unwrap();
    let status = r.lid.status();
    assert!(status.enabled && !status.outdated);
    assert_eq!(
        status.other_copy.as_deref(),
        Some("/Users/dev/dev/target/debug/hollow")
    );
    assert!(!status.armed, "the helper wouldn't follow this copy");
    assert_eq!(
        r.armed_files(),
        vec!["armed.777".to_string()],
        "the other copy's file isn't this copy's to remove"
    );
    r.lid.set(true, false).unwrap();
    assert_eq!(r.prompts(), vec![WHY_CHANGE.to_string()]);
    let status = r.lid.status();
    assert_eq!(status.other_copy, None);
    assert!(status.armed);
    assert_eq!(r.armed_files(), vec!["armed.4242".to_string()]);
}

#[test]
fn an_out_of_date_helper_says_so_and_the_next_change_updates_both_with_one_prompt() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    // An older Hollow's helper: the same Hollow and choices, another script.
    let mut older = plist::Value::from_file(&r.paths.plist).unwrap();
    older
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("ProgramArguments"))
        .and_then(|a| a.as_array_mut())
        .unwrap()[2] = plist::Value::String("echo an older helper".into());
    older.to_file_xml(&r.paths.plist).unwrap();
    let status = r.lid.status();
    assert!(status.enabled && status.outdated);
    assert_eq!(status.other_copy, None);
    assert!(status.armed, "it still serves this Hollow meanwhile");
    // The next change (Update the helper sends the same choice) replaces it.
    r.lid.set(true, false).unwrap();
    assert_eq!(
        r.prompts(),
        vec![WHY_ON.to_string(), WHY_UPDATE.to_string()]
    );
    let status = r.lid.status();
    assert!(!status.outdated);
    assert!(r.current(false));
    // Up to date, the same click asks nothing.
    r.lid.set(true, false).unwrap();
    assert_eq!(r.prompts().len(), 2);
}

#[test]
fn a_missing_or_older_restore_check_reads_as_out_of_date_and_update_puts_both_back() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    // An older restore check: another script.
    let mut older = plist::Value::from_file(&r.paths.restore_plist).unwrap();
    older
        .as_dictionary_mut()
        .and_then(|d| d.get_mut("ProgramArguments"))
        .and_then(|a| a.as_array_mut())
        .unwrap()[2] = plist::Value::String("echo an older restore check".into());
    older.to_file_xml(&r.paths.restore_plist).unwrap();
    assert!(r.lid.status().outdated);
    r.lid.set(true, false).unwrap();
    assert_eq!(
        r.prompts(),
        vec![WHY_ON.to_string(), WHY_UPDATE.to_string()]
    );
    assert!(r.current(false) && !r.lid.status().outdated);
    // The restore check gone (a helper from before it existed).
    std::fs::remove_file(&r.paths.restore_plist).unwrap();
    let status = r.lid.status();
    assert!(status.enabled && status.outdated && status.armed);
    r.lid.set(true, false).unwrap();
    assert_eq!(r.prompts().len(), 3, "one prompt for both");
    assert!(r.current(false) && !r.lid.status().outdated);
}

#[test]
fn a_lone_restore_check_reads_as_on_and_out_of_date_and_off_removes_it() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    std::fs::remove_file(&r.paths.plist).unwrap();
    let status = r.lid.status();
    assert!(status.enabled && status.outdated);
    assert!(!status.armed, "no helper to arm");
    r.lid.set(false, false).unwrap();
    assert_eq!(r.prompts(), vec![WHY_ON.to_string(), WHY_OFF.to_string()]);
    let status = r.lid.status();
    assert!(!status.enabled && !status.outdated);
    assert!(!r.paths.restore_plist.exists());
}

#[test]
fn status_says_how_long_ago_each_daemon_last_passed() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    let status = r.lid.status();
    for fresh in [status.last_pass_secs, status.last_restore_secs] {
        assert!(fresh.is_some_and(|s| s < 60), "the install's: {fresh:?}");
    }
    // The helper's passes stopped two minutes ago (Login Items switched
    // off, or held up), the restore check's ten minutes ago.
    r.passed(&r.paths.beat, Duration::from_secs(120));
    r.passed(&r.paths.restore_beat, Duration::from_secs(600));
    let status = r.lid.status();
    assert!(status
        .last_pass_secs
        .is_some_and(|s| (120..600).contains(&s)));
    assert!(status.last_restore_secs.is_some_and(|s| s >= 600));
    // A time ahead of the clock reads as just now.
    std::fs::File::options()
        .write(true)
        .open(&r.paths.beat)
        .unwrap()
        .set_modified(SystemTime::now() + Duration::from_secs(3600))
        .unwrap();
    assert_eq!(r.lid.status().last_pass_secs, Some(0));
    std::fs::remove_file(&r.paths.beat).unwrap();
    std::fs::remove_file(&r.paths.restore_beat).unwrap();
    let status = r.lid.status();
    assert_eq!(
        (status.last_pass_secs, status.last_restore_secs),
        (None, None)
    );
}

#[test]
fn status_says_how_long_ago_hollow_stopped_asking() {
    let r = rig();
    r.lid.set(true, false).unwrap();
    assert_eq!(r.lid.status().stopped_asking_secs, None, "asking on power");
    // Unplugged without On battery too: armed, but not asking.
    r.power
        .set(Some("\"SleepDisabled\" = Yes\n"), Some(ON_BATTERY));
    let status = r.lid.status();
    assert!(status.armed);
    assert!(status.stopped_asking_secs.is_some_and(|s| s < 60));
    // The iPhone app off keeps the first time it stopped.
    *r.phone.lock().unwrap() = (false, false);
    assert!(r.lid.status().stopped_asking_secs.is_some());
    // Asking again clears it.
    *r.phone.lock().unwrap() = (true, true);
    r.power
        .set(Some("\"SleepDisabled\" = Yes\n"), Some(ON_POWER));
    assert_eq!(r.lid.status().stopped_asking_secs, None);
}
