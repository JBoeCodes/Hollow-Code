//! Read-only operating-system probes (Paperclip PRs 18 and 20): is a process
//! alive, when did it start, how many `node` processes are running, and how
//! much memory pressure macOS reports.
//!
//! Used by the lease liveness check (`automated_history`, PCLIP-018) and the
//! run-permit pressure gate (`run_permit`, PCLIP-020). Safe in MCP
//! subprocesses (`hollow team-mcp` takes run permits): nothing here creates a
//! process group or delivers a signal; `kill(pid, 0)` only probes.
//!
//! Every probe returns `None` when the answer cannot be read (an unsupported
//! platform, a permission error, a vanished process). Callers treat `None` as
//! "unknown" and must fail safe: the lease is not reclaimed, the permit is
//! not gated.
//!
//! Platforms:
//! - macOS: `proc_pidinfo(PROC_PIDTBSDINFO)` for a process's start (µs since
//!   the epoch), `proc_listallpids` + `proc_name` for the node count, and
//!   `sysctlbyname` for `kern.memorystatus_vm_pressure_level` (1 normal,
//!   2 warning, 4 critical), `kern.memorystatus_level` (percent free) and
//!   `kern.boottime` (when the machine booted).
//! - Linux: `/proc/<pid>/stat` field 22 (start, clock ticks after boot,
//!   converted to µs after boot) and `/proc/*/comm`. No pressure probes.
//! - Elsewhere: every probe is `None`.
//!
//! Start stamps are only ever compared for equality with another stamp read
//! the same way on the same machine, so the Linux boot-relative value is as
//! good as the macOS epoch one.

use std::sync::OnceLock;

/// Whether `pid` exists: `Some(true)` when `kill(pid, 0)` succeeds,
/// `Some(false)` on `ESRCH` (no such process), `None` otherwise (`EPERM`: it
/// exists but belongs to another user; any other error). Pids ≤ 0 are `None`:
/// they name process groups, never one process.
pub(crate) fn pid_alive(pid: i32) -> Option<bool> {
    if pid <= 0 {
        return None;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 delivers nothing; it only checks existence.
        let rc = unsafe { libc::kill(pid, 0) };
        if rc == 0 {
            return Some(true);
        }
        match std::io::Error::last_os_error().raw_os_error() {
            Some(code) if code == libc::ESRCH => Some(false),
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// When `pid` started, as a platform start stamp (see the module doc).
/// `None` for a vanished process, a zombie macOS will not describe, or an
/// unsupported platform.
pub(crate) fn process_start_micros(pid: i32) -> Option<u64> {
    if pid <= 0 {
        return None;
    }
    platform::process_start_micros(pid)
}

/// `pid`'s parent pid, for walking a process up to the run that started it
/// (`run_auth`, merged plan slice 6). `None` for a vanished process, pid
/// 0 or 1 (nothing above them is a run), or an unsupported platform.
pub(crate) fn parent_pid(pid: i32) -> Option<i32> {
    if pid <= 1 {
        return None;
    }
    platform::parent_pid(pid).filter(|parent| *parent > 0)
}

/// This process's own start stamp, read once.
pub(crate) fn own_start_micros() -> Option<u64> {
    static OWN: OnceLock<Option<u64>> = OnceLock::new();
    *OWN.get_or_init(|| process_start_micros(std::process::id() as i32))
}

/// When this machine booted, in µs since the epoch (macOS `kern.boottime`,
/// Linux `btime` in `/proc/stat`). `None` when unreadable or elsewhere.
pub(crate) fn boot_unix_micros() -> Option<u64> {
    platform::boot_unix_micros()
}

/// A start stamp from [`process_start_micros`] as µs since the epoch: the
/// stamp itself on macOS, boot time plus the stamp on Linux (where it is
/// boot-relative, so a stamp from an earlier boot maps into this one).
/// `None` elsewhere.
pub(crate) fn stamp_unix_micros(stamp: u64) -> Option<u64> {
    platform::stamp_unix_micros(stamp)
}

/// How many processes named exactly `node` are running (every user's this
/// process can see). `None` when the process table cannot be read.
pub(crate) fn node_process_count() -> Option<usize> {
    platform::count_processes_named("node")
}

/// macOS's memory pressure level (`kern.memorystatus_vm_pressure_level`):
/// 1 normal, 2 warning, 4 critical. `None` elsewhere or when unreadable.
pub(crate) fn vm_pressure_level() -> Option<i32> {
    platform::sysctl_int("kern.memorystatus_vm_pressure_level")
}

/// macOS's free-memory percentage (`kern.memorystatus_level`, the number
/// `memory_pressure` prints), clamped to 0-100. Quoted in messages only.
pub(crate) fn free_memory_pct() -> Option<u8> {
    platform::sysctl_int("kern.memorystatus_level").map(|value| value.clamp(0, 100) as u8)
}

/// Available physical headroom on macOS: free plus inactive pages. This is
/// distinct from the diagnostic free percentage and excludes compressed RAM.
pub(crate) fn available_memory_bytes() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        static HOST: OnceLock<libc::host_t> = OnceLock::new();
        // Retain one host send right for this process, not one per sample.
        // libc recommends another crate for Mach bindings; the system API
        // itself remains supported. Keep this allowance on that binding only.
        #[allow(deprecated)]
        let host = *HOST.get_or_init(|| unsafe { libc::mach_host_self() });
        // SAFETY: the plain C output structure is zero-initialized and count
        // describes exactly its capacity in integer units.
        let mut stats: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_VM_INFO64_COUNT;
        let result = unsafe {
            libc::host_statistics64(
                host,
                libc::HOST_VM_INFO64,
                (&mut stats as *mut libc::vm_statistics64).cast(),
                &mut count,
            )
        };
        if result != libc::KERN_SUCCESS || count != libc::HOST_VM_INFO64_COUNT {
            return None;
        }
        // SAFETY: sysconf has no caller preconditions.
        let page_size = u64::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
        if page_size == 0 {
            return None;
        }
        (u64::from(stats.free_count) + u64::from(stats.inactive_count)).checked_mul(page_size)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::CString;

    pub(super) fn process_start_micros(pid: i32) -> Option<u64> {
        // SAFETY: `proc_bsdinfo` is a plain C struct; all-zero is valid.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: the buffer is exactly `size` bytes of a `proc_bsdinfo`.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
                size,
            )
        };
        if written != size {
            return None;
        }
        info.pbi_start_tvsec
            .checked_mul(1_000_000)?
            .checked_add(info.pbi_start_tvusec)
    }

    pub(super) fn parent_pid(pid: i32) -> Option<i32> {
        // SAFETY: `proc_bsdinfo` is a plain C struct; all-zero is valid.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
        // SAFETY: the buffer is exactly `size` bytes of a `proc_bsdinfo`.
        let written = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut libc::proc_bsdinfo as *mut libc::c_void,
                size,
            )
        };
        if written != size {
            return None;
        }
        i32::try_from(info.pbi_ppid).ok()
    }

    pub(super) fn count_processes_named(name: &str) -> Option<usize> {
        // SAFETY: a null buffer asks only for the current count estimate.
        let estimate = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if estimate <= 0 {
            return None;
        }
        // Headroom for processes started between the two calls.
        let capacity = estimate as usize + 256;
        let mut pids = vec![0 as libc::pid_t; capacity];
        let bytes = (capacity * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
        // SAFETY: the buffer holds `capacity` pids, `bytes` long.
        let count =
            unsafe { libc::proc_listallpids(pids.as_mut_ptr() as *mut libc::c_void, bytes) };
        if count <= 0 {
            return None;
        }
        let listed = (count as usize).min(capacity);
        let wanted = name.as_bytes();
        let mut matches = 0;
        let mut buf = [0u8; 256];
        for &pid in &pids[..listed] {
            if pid <= 0 {
                continue;
            }
            // SAFETY: `buf` is 256 writable bytes.
            let len = unsafe {
                libc::proc_name(pid, buf.as_mut_ptr() as *mut libc::c_void, buf.len() as u32)
            };
            if len > 0 && &buf[..(len as usize).min(buf.len())] == wanted {
                matches += 1;
            }
        }
        Some(matches)
    }

    pub(super) fn boot_unix_micros() -> Option<u64> {
        let key = CString::new("kern.boottime").ok()?;
        // SAFETY: `timeval` is a plain C struct; all-zero is valid.
        let mut value: libc::timeval = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::timeval>() as libc::size_t;
        // SAFETY: `value` is a timeval-sized buffer and `len` says so.
        let rc = unsafe {
            libc::sysctlbyname(
                key.as_ptr(),
                &mut value as *mut libc::timeval as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if rc != 0 || len != std::mem::size_of::<libc::timeval>() || value.tv_sec <= 0 {
            return None;
        }
        (value.tv_sec as u64)
            .checked_mul(1_000_000)?
            .checked_add(u64::try_from(value.tv_usec).ok()?)
    }

    pub(super) fn stamp_unix_micros(stamp: u64) -> Option<u64> {
        Some(stamp)
    }

    pub(super) fn sysctl_int(name: &str) -> Option<i32> {
        let key = CString::new(name).ok()?;
        let mut value: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::size_t;
        // SAFETY: `value` is an int-sized buffer and `len` says so.
        let rc = unsafe {
            libc::sysctlbyname(
                key.as_ptr(),
                &mut value as *mut libc::c_int as *mut libc::c_void,
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (rc == 0 && len == std::mem::size_of::<libc::c_int>()).then_some(value)
    }
}

#[cfg(target_os = "linux")]
mod platform {
    pub(super) fn process_start_micros(pid: i32) -> Option<u64> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // The command name (field 2) is parenthesized and may hold spaces or
        // parentheses; the fields after the LAST `)` start at field 3.
        let rest = &stat[stat.rfind(')')? + 1..];
        let ticks: u64 = rest.split_whitespace().nth(19)?.parse().ok()?;
        // SAFETY: sysconf has no preconditions.
        let hz = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        if hz <= 0 {
            return None;
        }
        ticks
            .checked_mul(1_000_000)
            .map(|micros| micros / hz as u64)
    }

    pub(super) fn parent_pid(pid: i32) -> Option<i32> {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // Field 4, the second after the parenthesized command name.
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_whitespace().nth(1)?.parse().ok()
    }

    pub(super) fn count_processes_named(name: &str) -> Option<usize> {
        let entries = std::fs::read_dir("/proc").ok()?;
        let mut matches = 0;
        for entry in entries.flatten() {
            let file_name = entry.file_name();
            let Some(pid) = file_name.to_str() else {
                continue;
            };
            if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            if let Ok(comm) = std::fs::read_to_string(entry.path().join("comm")) {
                if comm.trim_end_matches('\n') == name {
                    matches += 1;
                }
            }
        }
        Some(matches)
    }

    pub(super) fn boot_unix_micros() -> Option<u64> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        let secs: u64 = stat
            .lines()
            .find_map(|line| line.strip_prefix("btime "))?
            .trim()
            .parse()
            .ok()?;
        secs.checked_mul(1_000_000)
    }

    pub(super) fn stamp_unix_micros(stamp: u64) -> Option<u64> {
        boot_unix_micros()?.checked_add(stamp)
    }

    pub(super) fn sysctl_int(_name: &str) -> Option<i32> {
        None
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod platform {
    pub(super) fn process_start_micros(_pid: i32) -> Option<u64> {
        None
    }

    pub(super) fn parent_pid(_pid: i32) -> Option<i32> {
        None
    }

    pub(super) fn count_processes_named(_name: &str) -> Option<usize> {
        None
    }

    pub(super) fn boot_unix_micros() -> Option<u64> {
        None
    }

    pub(super) fn stamp_unix_micros(_stamp: u64) -> Option<u64> {
        None
    }

    pub(super) fn sysctl_int(_name: &str) -> Option<i32> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn available_memory_reads_free_plus_inactive_pages() {
        let bytes = available_memory_bytes().expect("host VM statistics are readable");
        assert_eq!(
            bytes % 4096,
            0,
            "physical pages are whole multiples of 4 KiB"
        );
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn the_boot_precedes_this_process_and_stamps_map_to_the_epoch() {
        let boot = boot_unix_micros().expect("boot time");
        let own = stamp_unix_micros(own_start_micros().expect("own start")).expect("epoch");
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        // 2020-01-01: a sanity floor for a real boot time.
        assert!(boot > 1_577_836_800_000_000, "boot {boot}");
        assert!(boot < own, "booted {boot}, this process started {own}");
        assert!(own <= now + 1_000_000, "started {own}, now {now}");
    }

    #[cfg(unix)]
    #[test]
    fn a_reaped_child_reads_as_dead_and_this_process_as_alive() {
        let own = std::process::id() as i32;
        assert_eq!(pid_alive(own), Some(true));
        assert_eq!(pid_alive(0), None, "pid 0 names a group, never a process");
        assert_eq!(pid_alive(-5), None);
        let mut child = std::process::Command::new("/usr/bin/true")
            .spawn()
            .expect("spawn /usr/bin/true");
        let pid = child.id() as i32;
        child.wait().expect("reap it");
        // A pid can be reused right away in theory; in practice a freshly
        // reaped pid stays free for a while. Either way it is never us.
        assert_ne!(pid, own);
        if pid_alive(pid) == Some(true) {
            assert_ne!(
                process_start_micros(pid),
                None,
                "a reused pid is a live process with its own start"
            );
        } else {
            assert_eq!(pid_alive(pid), Some(false));
        }
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_child_names_this_process_as_its_parent() {
        let own = std::process::id() as i32;
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .expect("spawn /bin/sleep");
        let pid = child.id() as i32;
        let parent = parent_pid(pid);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(parent, Some(own));
        assert_eq!(parent_pid(1), None, "nothing above launchd/init is a run");
        assert_eq!(parent_pid(0), None);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn own_start_is_stable_and_matches_the_per_pid_read() {
        let own = own_start_micros().expect("this platform reports a start");
        assert_eq!(own_start_micros(), Some(own), "read once and cached");
        assert_eq!(process_start_micros(std::process::id() as i32), Some(own));
        assert_eq!(process_start_micros(0), None);
    }
}
