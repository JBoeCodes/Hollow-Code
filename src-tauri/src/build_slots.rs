//! Build slots and per-worktree build settings (Hollow build programme PR
//! B7; see docs/history/HOLLOW_BUILD_PLAN.md).
//!
//! This Mac runs at most TWO toolchain commands at once, each capped
//! (`~/.claude/CLAUDE.md`): uncapped parallel builds froze it twice. Since
//! B4 several build workspaces' turns run in parallel, so each one gets:
//!
//! - **its own output folders**: `CARGO_TARGET_DIR` under
//!   `~/Library/Caches/hollow-build-targets/` (outside iCloud-synced
//!   `~/Documents`), and its own `PORT` / `PW_PORT`, so two worktrees never
//!   share a `target/` or a dev-server port;
//! - **the caps**: `CARGO_BUILD_JOBS=8`, `VITEST_MAX_*=4` (and the minimums
//!   vitest needs on an 18-core Mac), which Codex and other agents do not get
//!   from Claude's settings;
//! - **`hollow-slot <command>`** on its `PATH`: a small shell tool that runs
//!   the command while holding one of two machine-wide slot locks
//!   (`/usr/bin/lockf`, or perl's `flock` where macOS has no `lockf`, on
//!   `slot-0.lock` / `slot-1.lock`), waiting (and saying
//!   so once) while both are taken. The kernel frees a slot when its holder
//!   exits, so a crash never leaks one. It is cooperative: the toolchain guard
//!   hook still protects against anything that does not use it (another
//!   session in Conductor, a command run without the wrapper).
//!
//! Since OF07 (2026-10-17) every chat turn that can run Bash (any mode but
//! plan; Antigravity only in bypass, [`can_take_slot`]) gets the caps and
//! `hollow-slot` too ([`chat_env`]), not only a build workspace's: the
//! overnight lanes had built their own `mkdir` semaphore and a separate
//! Playwright lock, which the slots couldn't see. Workflow steps, heartbeat
//! passes and `ask_agent` delegates don't get them yet. The locks moved to
//! `/tmp/hollow-build-slots-<uid>` ([`slot_dir`]), which sandboxed Codex and
//! Grok turns can write.
//! `hollow-slot --dev-server <command>` holds a third lock,
//! `dev-server.lock`: one dev server at a time on this Mac, with `PORT` /
//! `PW_PORT` set (the turn's own `PORT`, else [`DEV_SERVER_PORT`]); it
//! refuses with exit 75 and the holder while another holds it. Whoever takes
//! a lock writes `<lock>.holder` (pid, since, port, command line), which
//! `build.slots_status` ([`status`]) reports beside whether each is held.
//!
//! One Rust build at a time (2026-10-20): the two slots stay, but at most one
//! of them compiles Rust, because two Hollow compiles together peak around
//! 13 GB (one `hollow_lib` rustc alone reached 6.36 GB). A command the tool
//! takes for a Rust build (an argument, or a word of the string after a shell
//! option cluster with `c` such as `sh -ce`, names `cargo`, `rustc`,
//! `cargo-*` or `tauri`, or it runs a repo npm script that runs cargo;
//! `HOLLOW_SLOT_CARGO=1`/`0` overrides) holds `cargo.lock` ([`CARGO`]) as
//! well as a slot, taking both in one non-blocking try per slot: nobody
//! ever waits while holding a lock, so no wait can close a cycle. Lock files
//! are close-on-exec (set and checked), so a lock lives exactly as long as
//! its holder. A slot's command gets `HOLLOW_SLOT_HOLDER_PID` (a Rust build's
//! also `HOLLOW_SLOT_CARGO_PID`); a `hollow-slot` called from inside it runs
//! in its caller's slot, a Rust build there waiting for `cargo.lock` alone,
//! and one inside a Rust build is refused with exit 2. The hints can move a
//! call out of a slot or refuse it, never skip `cargo.lock`. (Reviews of PR
//! #859: three exemptions for nested builds and a take-the-cargo-lock-then-
//! wait-for-a-slot order were each unsound.)
//!
//! Two Rust lanes on a runner (R0, 2026-11-14): the Mac mini has 10 cores and
//! 32 GB, so its runs get two cargo locks, lane 0 (`cargo.lock`, as above)
//! and lane 1 ([`CARGO_LANE_1`], `cargo-1.lock`), each tried in the same
//! one-try, no-wait way (a Rust build takes a slot and then a lane, lane 0
//! first), so two gates compile at once there. Each lane's jobs are capped at
//! 5 (`CARGO_BUILD_JOBS`), and the build folder is the checkout's own
//! (`targets/<worktree id>`, a run holding its checkout's folder lock), so
//! two lanes never share a target. A memory watch (`kern.memorystatus_level`,
//! the toolchain guard's number) turns lane 1 off while less than 30% is
//! free, checked at every try; the one lane left keeps its 5 jobs (low memory
//! lowers the work, never raises it; 8 jobs only where lanes are off).
//! `HOLLOW_SLOT_RUST_LANES=1` on the runner keeps it to one. This Mac's own
//! calls don't set the variable, so they have no lane 1: one Rust build at a
//! time here, as before (cooperative: exporting `HOLLOW_SLOT_RUST_LANES=2`
//! here would open it).
//!
//! Since F6 part A (2026-10-20) every other chat turn that gets the caps,
//! and a build workspace's vault-folder conversation, also leases a `PORT` /
//! `PW_PORT` of its own from [`TURN_PORTS`] for the length of the turn
//! ([`lease_turn_port`]): several conversations run at once in one
//! workspace's folder, and they used to share 1499 and no `PW_PORT`. They
//! get no `CARGO_TARGET_DIR` of their own: a workspace folder holds no code,
//! and a per-conversation target would be a cold multi-gigabyte build each
//! time (a build workspace's vault folder keeps its worktree's warm one).

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// The tool every build workspace's turn can call. Written to Hollow's app
/// data `bin/` folder and put first on the turn's `PATH`.
pub(crate) const SLOT_TOOL: &str = r#"#!/bin/sh
# hollow-slot: run a build or test command in one of this Mac's two build
# slots (Hollow build programme PR B7). Waits while both are taken.
# hollow-slot --dev-server <command>: run a dev server holding this Mac's one
# dev-server lock (OF07), with PORT and PW_PORT set; refuses while another
# holds it. Either way the lock is taken, the command runs and the lock is
# freed in one call: never hold one across commands.
# The locks live in /tmp/hollow-build-slots-<uid> (HOLLOW_BUILD_SLOT_DIR
# overrides it): a temp folder, which every agent's sandbox can write, made
# 0700. Exit 75 (EX_TEMPFAIL) is also how a lock says it is taken, so a
# command that itself exits 75 can't be told apart from one: a build runs
# again (in the other slot, then after each poll, so one that always exits
# 75 never ends), and a dev server is retried and then reported as refused.
# One Rust build at a time (2026-10-20): two Hollow compiles together peak
# around 13 GB, so a command that compiles Rust holds cargo.lock as well as a
# slot. It takes both in one try per slot, never one while waiting for the
# other: nobody ever waits holding a lock, so no wait can close a cycle. It is
# a Rust build when an argument (or a word of the string after a shell option
# cluster with c: -c, -ce, -lc, -xec, as in sh -c '...') is cargo, rustc,
# cargo-*, tauri or a path ending in one, or it runs one of the repo's npm
# scripts that run cargo (npm's options before or after run skipped, an
# option's value being one whole argument): gate, gate:rust, perf, tauri,
# tauri:dev, tauri:bundle, tauri:build, tauri:rebuild. Other wrappers (perl
# -e, a script file) aren't looked into: HOLLOW_SLOT_CARGO=1 makes any
# command one, =0 none. Lock files are close-on-exec (lockf(1) keeps its own
# from the command too), so a lock lives exactly as long as its holder, a
# kill -9 included. A slot's command gets HOLLOW_SLOT_HOLDER_PID (and a Rust
# build's HOLLOW_SLOT_CARGO_PID): a hollow-slot called from inside it (that
# pid a live ancestor) runs in its caller's slot instead of taking another;
# a Rust build there still waits for cargo.lock alone, holding nothing, and
# one inside a Rust build is refused with exit 2 (it would wait for itself).
# Two Rust lanes on a runner (R0, 2026-11-14): its runs (remote_host_run)
# have HOLLOW_SLOT_RUST_LANES=2, so a Rust build there tries cargo.lock (lane
# 0) and then cargo-1.lock (lane 1), in the same one-try way, taking either
# with a slot; two Rust builds then run at once, each with
# CARGO_BUILD_JOBS=5 (HOLLOW_SLOT_LANE_JOBS=1; 5 for lane 0 under low memory
# too, 8 only where lanes are off). Lane 1 is
# off while free memory is under HOLLOW_SLOT_MIN_FREE_PCT (30; the guard's
# kern.memorystatus_level, tests give another sysctl in HOLLOW_SLOT_SYSCTL),
# read again at every try. HOLLOW_SLOT_RUST_LANES=1 in the runner's login
# keeps it to one. A call on this Mac never sets it, so here it is one.
# The hints are only that: a wrong one can make a call run in no slot or be
# refused, never skip a lane lock. (A same-user process that forges them on
# purpose is outside this tool's reach; it can opt out with
# HOLLOW_SLOT_CARGO=0 anyway.) Two known limits: --dev-server takes only the
# dev-server lock and never looks at the command, so a Tauri dev server
# (npm run tauri:dev) compiles Rust without cargo.lock (this Mac's rule
# already keeps a dev server from running beside a build); and a Rust build
# that itself exits 75 looks like a taken lock, so it is retried forever,
# like any build that exits 75.
# Remote runs (2026-11-01): hollow-slot --remote <command> runs it on this
# Mac's remote runner instead, the SSH host in HOLLOW_SLOT_REMOTE or on the
# first line of ~/.config/hollow/slot-remote (--on <host> names one). The
# checkout's exact state goes there with git push to a bare repository in
# the host's ~/hollow-remote: HEAD when the tree is clean, else a commit of
# HEAD plus every uncommitted and untracked change, made in a private object
# folder so this checkout's own repository gets nothing. The host checks it
# out in a folder of its own per local worktree (ignored files such as
# node_modules stay there between runs; npm ci runs when package-lock.json
# changes) and runs the command in one of its own slots, through this same
# tool, with that worktree's own CARGO_TARGET_DIR (a new one starts as a
# copy-on-write clone of the newest one of the same repository's there, when
# the checkout tracks a Cargo.toml), a short TMPDIR and a PORT/PW_PORT of
# its own. Output streams back; the exit code is the
# command's (ssh's own failures are 255 too, so a command that exits 255
# reads as a lost connection and nothing is copied back). When the host
# can't be reached, its hollow-run-preflight says no, git status fails here
# or the checkout can't be sent, it says why (for ssh's own failure, also
# what to check: r_why) and runs the command here instead, so nothing ran
# there. --fetch <path> copies a file or folder the
# command wrote there back over this worktree's (2026-11-03: only one in
# which something changed after the command started, since an ignored one an
# earlier run left stays there, so a tool that wrote nothing, a no-op make,
# brings nothing back; never a symlink, nor a folder holding one
# that could point out of it; a path inside another fetched one goes with
# it, paths compared with repeated slashes, a leading ./ and a trailing /
# dropped; the old copy here waits beside it under a dot name and is deleted
# only once the new one is in place, else put back); a
# gate record of a clean tree comes back to this checkout's gate records by
# itself. None of this
# Mac's environment goes along: put a variable in the command (env NAME=value
# <command>); only HOLLOW_SLOT_CARGO and the setup timings go, and (11-02)
# the git identity this Mac uses for the checkout (r_identity: includes and
# includeIf honoured), which is the command's
# GIT_AUTHOR_* and GIT_COMMITTER_* there, so a commit it makes is this Mac's,
# not a guess from the runner account; it travels on the preparation's stdin
# (never a command line) and waits for this call alone in a file only that
# account reads (work/<id>.<token>.ident), removed as the run starts; with
# none configured here, git's own default there stays.
# Stopping the call stops the remote command: ssh reads its stdin
# from a pipe only this process holds, and on the host a watchdog kills the
# command's process group when that stdin ends. A host keeps
# HOLLOW_REMOTE_KEEP (8) worktree folders and their build folders, and drops
# any unused for HOLLOW_REMOTE_DAYS (14) days, and while its disk has less
# than HOLLOW_REMOTE_MIN_FREE_GB (40) free, the least recently used ones
# unused for an hour, saying so (remote sessions' folders are theirs); its
# preflight refusing prunes once and asks again. Each worktree folder there
# keeps one port of its own while it exists (work/<id>.port). A gate's run
# record (hollow-gate-runs) comes back too, clean tree or not, so gate-stats
# here counts gates run there. A sandboxed Codex turn has no network
# (CODEX_SANDBOX_NETWORK_DISABLED=1), so --remote says so and runs here.
# Jobs (overnight N4, 2026-11-02): with --send <path> (repeatable), the
# command needs no checkout: those files or folders under this folder go to
# a folder of the job's own on the host (tar over ssh), the command runs
# there, one job at a time on the host, with at most 2 render workers
# (PRODUCER_MAX_WORKERS) and HyperFrames' pins unless it sets them, the
# --fetch paths come back, and the folder is deleted. For a transcription or
# a video render that shouldn't hold this Mac.
# Each setup step is timed (overnight N3, 2026-11-02): when getting to the
# command took HOLLOW_SLOT_SETUP_SAY (5) seconds or more, the host says one
# line naming the steps that took a second or more (r_note); so does
# wrapping up after it, and this side when copying results back is slow.
# Memory floor (2026-11-01): a run on this machine, a fallback included,
# doesn't start a build or dev server while less than
# HOLLOW_SLOT_MIN_FREE_PCT (30) percent of memory is free
# (kern.memorystatus_level, what memory_pressure reports; the toolchain
# guard's floor, which only Claude's hook applied before). Once it holds its
# slot (or the dev-server lock) it waits, saying so once, and after more than
# HOLLOW_SLOT_MEMORY_WAIT (300) seconds under the floor refuses with exit 69,
# freeing the slot. A build call inside a slot's own command doesn't wait:
# its caller already did (a --dev-server call always does). HOLLOW_SLOT_MIN_FREE_PCT=0 turns it off; like the
# cargo hint, it is cooperative, not a security boundary.
# Routing (2026-11-03, smart routing): with a remote runner set up, a plain
# hollow-slot <command> decides where it runs (--auto, the default;
# HOLLOW_SLOT_ROUTE=auto, runner (or mac-mini, the name Hollow still sets
# so older copies of this tool read it) or this-mac changes the default: Hollow
# sets it from Settings -> Devices -> Builds and tests, smart
# routing 3, and Settings shows route-log's latest lines) and says
# so in one line ("on the Mac mini (gate, usually 3 min; this Mac 72%
# free)"). --here keeps it on this Mac; --remote and --on send it as before.
# In order:
# 1. requirements: a command that needs this Mac stays here (route_pl: its
#    database variables, gcloud, gh, git, ssh, docker and deploy tools, the
#    keychain or signing, package installs, anything that writes into the
#    checkout (a > redirect included, unless --fetch brings it back), app
#    builds, benchmarks, cargo run, dev servers, media work without --send,
#    Python tests outside uv, this Mac's own path into the checkout, a file
#    outside the checkout);
# 2. its class: gates, Rust builds, whole test suites, typechecks, lints,
#    builds, Playwright and Python tests go there; one test file, a lint of
#    named files and the like stay here;
# 3. history (route-history in the slot folder: the last runs of the same
#    command in the same repository): a usual time (passing runs only) of
#    HOLLOW_SLOT_LONG_SECS (10) or more sends a quick one, or one of no
#    known kind with two passing runs, there; a heavy one under
#    HOLLOW_SLOT_QUICK_SECS (5) stays here; one that changed the checkout,
#    here or there, stays here, and so does one of no known kind that failed
#    there and passes here, and for a day one that couldn't start there (126
#    or 127; the host's own setup failures exit 247 and run it here);
# 4. pressure: this Mac under its memory floor, both of its slots taken or
#    on battery sends a command that may go; a runner that didn't answer in
#    the last 5 minutes (runner-down) keeps work here.
# The runner's preparation reports its tools and load (caps, kept here in
# runner-caps): a routed command needing a tool it lacks runs here, and an
# automatic one comes back here when both its slots, or its Rust build
# lock for a Rust build, are taken while this Mac is free. Each decision is
# a line of route-log in the slot folder. A run there that changed files in
# its copy of the checkout, or wrote ignored output, says which: they don't
# come back.
# Linux runners (2026-11-03): the host side also runs on Linux. hs_os, read
# once, picks util-linux flock for its locks (lockf is macOS's), a reflink
# (or none) for a build folder's clone and /proc for the parent walk. Where
# the macOS source gives nothing, the memory floor reads MemAvailable in
# /proc/meminfo and a machine's name is uname -n (on any system; a Mac never
# gets that far). Its preparation reports os= in its caps, and a command that
# needs macOS (codesign, xcodebuild, xcrun, swift, MLX, a webkit browser for
# Playwright or vitest, a Rust build or gate of a Tauri checkout) stays here
# when the runner is Linux. HOLLOW_SLOT_FLOCK and HOLLOW_SLOT_MEMINFO stand in
# for /usr/bin/flock and /proc/meminfo in this tool's tests.
dir="${HOLLOW_BUILD_SLOT_DIR:-/tmp/hollow-build-slots-$(id -u)}"
hs_os=$(uname -s 2>/dev/null)
hs_flock="${HOLLOW_SLOT_FLOCK:-/usr/bin/flock}"
usage_text() {
  echo "usage: hollow-slot [--auto | --here] <command> [args...]"
  echo "       hollow-slot --dev-server <command> [args...]"
  echo "       hollow-slot [--postgres] <command> [args...]"
  echo "       hollow-slot --remote [--fetch <path>]... <command> [args...]"
  echo "       hollow-slot --on <ssh host> [--fetch <path>]... <command> [args...]"
  echo "       hollow-slot --remote --send <path>... [--fetch <path>]... <command> [args...]"
  echo "One call per heavy build or test. With a remote runner set up (~/.config/hollow/slot-remote), it decides where the command runs and says why in one line: gates, Rust builds, whole test suites and other long work go to the runner; quick checks, anything needing this Mac (its database, gcloud, GitHub, the keychain, files outside the checkout) and anything that writes into the checkout stay here. --here keeps it here; --remote sends it there (else here). A dev server runs with --dev-server (one on this Mac at a time). --send runs a job there (a transcription, a render) in a folder holding only what it sends, no checkout needed; --fetch copies a path the command wrote there back here. --postgres gives the command a throwaway Postgres wherever it runs (DATABASE_URL set, database hollow_test), removed when it ends."
}
usage() {
  usage_text >&2
  exit 2
}
say() { printf 'hollow-slot: %s\n' "$*" >&2; }
# The memory floor, as text: it runs in a lock's own shell (record, below),
# so a call waits for memory only once it holds its slot or the dev-server
# lock. A call queued behind both slots never times out on memory, and one
# that waits holding a slot starts nothing that uses memory.
# mem_free: the percentage of memory free, from kern.memorystatus_level by
# sysctl's full path (a PATH without /usr/sbin must not turn the floor off),
# else from memory_pressure -Q, which Codex's sandbox allows where it refuses
# that sysctl, else (a Linux runner) from /proc/meminfo, MemAvailable as a
# percentage of MemTotal; fails with no reading. HOLLOW_SLOT_SYSCTL,
# HOLLOW_SLOT_MEMORY_PRESSURE and HOLLOW_SLOT_MEMINFO are for this tool's
# tests.
# memory_wait: wait while less than the floor is free, saying so once; fail
# once it has been under the floor for more than HOLLOW_SLOT_MEMORY_WAIT
# seconds. No reading, or a floor of 0, never waits. HOLLOW_SLOT_MEM=remote
# also names the remote runner in the refusal.
mem_lib='
mem_free() {
  m_level=$("${HOLLOW_SLOT_SYSCTL:-/usr/sbin/sysctl}" -n kern.memorystatus_level 2>/dev/null)
  case "$m_level" in
    "" | *[!0-9]*) m_level=$("${HOLLOW_SLOT_MEMORY_PRESSURE:-/usr/bin/memory_pressure}" -Q 2>/dev/null | sed -n "s/.*free percentage: *\([0-9][0-9]*\)%.*/\1/p" | head -n 1) ;;
  esac
  case "$m_level" in
    "" | *[!0-9]*) m_level=$(awk "/^MemTotal:/ { t = \$2 } /^MemAvailable:/ { a = \$2; s = 1 } END { if (t > 0 && s) printf \"%d\", a * 100 / t }" "${HOLLOW_SLOT_MEMINFO:-/proc/meminfo}" 2>/dev/null) ;;
  esac
  case "$m_level" in "" | *[!0-9]*) return 1 ;; esac
  printf "%s" "$m_level"
}
memory_wait() {
  m_floor="${HOLLOW_SLOT_MIN_FREE_PCT:-30}"
  case "$m_floor" in "" | *[!0-9]*) m_floor=30 ;; esac
  [ "$m_floor" -gt 0 ] || return 0
  m_limit="${HOLLOW_SLOT_MEMORY_WAIT:-300}"
  case "$m_limit" in "" | *[!0-9]*) m_limit=300 ;; esac
  m_since=$(date +%s)
  m_said=0
  m_mac="this Mac ($(hostname -s 2>/dev/null))"
  [ "$(uname -s 2>/dev/null)" != Linux ] || m_mac="this Linux runner ($(uname -n 2>/dev/null))"
  m_hint="Let the running build or dev server finish, then try again."
  [ "$1" != remote ] || m_hint="Let the running build or dev server finish and try again, or run tests and gates on the remote runner: hollow-slot --remote <command>."
  while m_now=$(mem_free) && [ "$m_now" -lt "$m_floor" ]; do
    if [ $(($(date +%s) - m_since)) -gt "$m_limit" ]; then
      printf "hollow-slot: %s\n" "not starting it: $m_mac has had under ${m_floor}% of its memory free for over ${m_limit} s (now ${m_now}%), the floor that keeps parallel builds from freezing it. $m_hint" >&2
      return 1
    fi
    if [ "$m_said" = 0 ]; then
      printf "hollow-slot: %s\n" "waiting: $m_mac has ${m_now}% of its memory free, under its ${m_floor}% floor for builds; starting once it is back over it (up to ${m_limit} s)" >&2
      m_said=1
    fi
    sleep "${HOLLOW_SLOT_POLL_SECS:-5}"
  done
  return 0
}
'
# Only the main script sets these, for its own slot or dev-server call (and
# HOLLOW_SLOT_PG only for --postgres, smart routing 6).
unset HOLLOW_SLOT_MEM HOLLOW_SLOT_TIMER HOLLOW_SLOT_PG
# Only lock_wait sets this, for its own perl; inherited, it would make every
# try-once lock below wait instead.
unset HOLLOW_SLOT_LOCK_WAIT
# Run "$@" holding lock file $1, or exit 75 at once if it is held: perl's
# flock(2), on a file it marks close-on-exec itself (perl does so only above
# fd 2, and with a standard fd closed the lock could land on 0-2), checked.
# With HOLLOW_SLOT_LOCK_WAIT=1 it waits for the lock instead (lock_wait).
lock_pl='
  use Fcntl qw(:flock F_GETFD F_SETFD FD_CLOEXEC);
  my $lock = shift;
  my $wait = delete $ENV{HOLLOW_SLOT_LOCK_WAIT};
  open(my $fh, ">>", $lock)
    or do { print STDERR "hollow-slot: cannot open the lock file $lock ($!)\n"; exit 1 };
  (fcntl($fh, F_SETFD, FD_CLOEXEC) && (fcntl($fh, F_GETFD, 0) & FD_CLOEXEC))
    or do { print STDERR "hollow-slot: cannot keep the lock file $lock from the command ($!)\n"; exit 1 };
  flock($fh, $wait ? LOCK_EX : LOCK_EX | LOCK_NB) or exit 75;
  system { $ARGV[0] } @ARGV;
  exit($? == -1 ? 127 : $? & 127 ? 128 + ($? & 127) : $? >> 8);
'
# A slot lock: lockf(1) where there is one (it closes the lock in the
# command it runs), else the perl above; the two see each other. lockf isn't
# on every macOS Hollow supports (macOS 14 has none). On a Linux runner,
# util-linux flock(1) (2026-11-03): -o closes the lock in the command, as
# lockf does; -n gives up at once like lockf -t 0, -E 75 exits as lockf does
# then (flock's own conflict code is 1); it says nothing, as lockf -s. Both
# take flock(2) locks, as perl does, so all three see each other.
# It is tried first there (hs_os, so a Mac never runs it), and
# HOLLOW_SLOT_NO_LOCKF picks the perl everywhere.
in_slot() {
  lock="$1"
  shift
  if [ "$hs_os" = Linux ] && [ -x "$hs_flock" ] && [ -z "$HOLLOW_SLOT_NO_LOCKF" ]; then
    "$hs_flock" -o -n -E 75 "$lock" "$@"
  elif [ -x /usr/bin/lockf ] && [ -z "$HOLLOW_SLOT_NO_LOCKF" ]; then
    /usr/bin/lockf -s -t 0 "$lock" "$@"
  else
    /usr/bin/perl -e "$lock_pl" "$lock" "$@"
  fi
}
# Run inside the lock: write who holds it to <lock>.holder (pid, since,
# port, label; build.slots_status reads it), then become the command, so
# the pid is the command's own. The file is written aside and moved into
# place, so a reader never sees half of it; printf, not echo, because
# macOS sh's echo expands backslashes in a command line. A slot's or the
# cargo lock's command gets its pid as a hint for hollow-slot calls inside
# it. The message is said at the lock HOLLOW_SLOT_SAY_ON names (the last
# one a Rust build takes), else at the first; there, with HOLLOW_SLOT_MEM
# set (a call from this script's own slot or dev-server path, never a call
# inside a slot's command), it first waits for memory, and exits 69 (not 75,
# which means a lock is taken) when the floor holds past its wait.
record="$mem_lib"'
lock="$1"; shift
tmp="$lock.holder.$$"
{
  printf "pid=%s\n" "$$"
  printf "since=%s\n" "$(date +%s)"
  [ -z "$HOLLOW_SLOT_PORT" ] || printf "port=%s\n" "$HOLLOW_SLOT_PORT"
  printf "label=%s\n" "$(printf %s "${HOLLOW_SLOT_LABEL:-$*}" | tr "\n\r" "  ")"
} 2>/dev/null > "$tmp" && mv -f "$tmp" "$lock.holder" 2>/dev/null || rm -f "$tmp" 2>/dev/null
if [ "${HOLLOW_SLOT_SAY_ON:-$lock}" = "$lock" ]; then
  if [ -n "$HOLLOW_SLOT_MEM" ]; then
    m_mode="$HOLLOW_SLOT_MEM"
    unset HOLLOW_SLOT_MEM
    memory_wait "$m_mode" || exit 69
  fi
  [ -z "$HOLLOW_SLOT_SAY" ] || printf "%s\n" "$HOLLOW_SLOT_SAY" >&2
  [ -z "$HOLLOW_SLOT_TIMER" ] || date +%s > "$HOLLOW_SLOT_TIMER" 2>/dev/null
  unset HOLLOW_SLOT_SAY HOLLOW_SLOT_SAY_ON HOLLOW_SLOT_TIMER
fi
unset HOLLOW_SLOT_PORT
case "$lock" in
  */cargo.lock | */cargo-1.lock) HOLLOW_SLOT_HOLDER_PID=$$ HOLLOW_SLOT_CARGO_PID=$$; export HOLLOW_SLOT_HOLDER_PID HOLLOW_SLOT_CARGO_PID ;;
  */slot-*.lock) HOLLOW_SLOT_HOLDER_PID=$$; export HOLLOW_SLOT_HOLDER_PID ;;
esac
exec "$@"'
# Run "$@" holding lock file $1, waiting for it as long as it takes.
lock_wait() {
  lock="$1"
  shift
  if [ "$hs_os" = Linux ] && [ -x "$hs_flock" ] && [ -z "$HOLLOW_SLOT_NO_LOCKF" ]; then
    "$hs_flock" -o "$lock" "$@"
  elif [ -x /usr/bin/lockf ] && [ -z "$HOLLOW_SLOT_NO_LOCKF" ]; then
    /usr/bin/lockf -s "$lock" "$@"
  else
    HOLLOW_SLOT_LOCK_WAIT=1 /usr/bin/perl -e "$lock_pl" "$lock" "$@"
  fi
}
# $1 single-quoted for the remote shell (sh or zsh), newlines and all.
q() {
  set -- "$(printf '%sx' "$1" | LC_ALL=C sed "s/'/'\\\\''/g")"
  printf "'%s'" "${1%x}"
}
# The first $2 (12) hex digits of $1's SHA-256; a folder name's safe part.
hash_of() { printf '%s' "$1" | /usr/bin/shasum -a 256 | cut -c1-"${2:-12}"; }
# A leading dot becomes _ (2026-11-03): ls and the *.used globs skip a
# dot-named folder, which was then never pruned or seeded from.
# remote_session.rs slug_of matches it.
slug_of() { printf '%s' "${1##*/}" | LC_ALL=C tr -c 'A-Za-z0-9._-' '-' | cut -c1-40 | sed 's/^\./_/'; }
# A path inside the checkout: relative, one line, no . or .. part, and no
# .git part (bringing a host .git back would replace this repository).
valid_rel() {
  case "$1" in '' | /* | -* | *'
'*) return 1 ;; esac
  case "/$1/" in */../* | */./* | */.[Gg][Ii][Tt]/*) return 1 ;; esac
  return 0
}
# $1 in one spelling (2026-11-03): repeated slashes squeezed, no leading ./
# and no trailing slash. fetch_tidy compares paths as typed, so a//b/c beside
# a/b used to stay in the list and the folder lost c when it came back.
norm_rel() {
  r_p=$(printf '%sx' "$1" | LC_ALL=C tr -s /)
  r_p="${r_p%x}"
  while :; do case "$r_p" in ./*) r_p="${r_p#./}" ;; *) break ;; esac; done
  while [ "$r_p" != "${r_p%/}" ]; do r_p="${r_p%/}"; done
  printf '%s' "$r_p"
}
# Whether a folder on the way from $1 to $1/$2 (not $2 itself) is a symlink.
link_on_way() {
  r_base="$1"
  r_rest="$2"
  while :; do
    case "$r_rest" in */*) ;; *) return 1 ;; esac
    r_base="$r_base/${r_rest%%/*}"
    r_rest="${r_rest#*/}"
    [ -L "$r_base" ] && return 0
  done
}
# Whether every symlink in $1 (a fetched file or folder) stays inside it:
# relative with no .. part, so no chain of them can leave it either. Fails
# on one that doesn't, or a folder find can't read.
links_inside() {
  find "$1" -type l -exec /bin/sh -c 'for l do t=$(readlink "$l") || exit 1; case "/$t/" in //* | */../*) exit 1 ;; esac; done' hollow-slot {} + 2>/dev/null
}
# The --fetch list ($1, a path a line) without a path that another one in it
# holds, or a second copy of one: each comes back by replacing what is here,
# so a folder copied back after a path inside it deleted that path.
fetch_tidy() {
  r_out=
  while IFS= read -r r_a; do
    [ -n "$r_a" ] || continue
    r_in=
    while IFS= read -r r_b; do
      [ -n "$r_b" ] && [ "$r_b" != "$r_a" ] || continue
      case "$r_a/" in "$r_b"/*) r_in=1 ;; esac
    done <<EOF2
$1
EOF2
    case "
$r_out" in *"
$r_a
"*) r_in=1 ;; esac
    [ -n "$r_in" ] || r_out="$r_out$r_a
"
  done <<EOF
$1
EOF
  printf '%s' "$r_out"
}
# The remote runner --remote uses: HOLLOW_SLOT_REMOTE when it is set (even
# empty), else the first line of ~/.config/hollow/slot-remote that isn't
# blank or a # comment. off, local or none turn it off.
configured_remote() {
  if [ -n "${HOLLOW_SLOT_REMOTE+set}" ]; then
    printf '%s' "$HOLLOW_SLOT_REMOTE"
    return 0
  fi
  r_conf="${XDG_CONFIG_HOME:-$HOME/.config}/hollow/slot-remote"
  [ -r "$r_conf" ] || return 0
  sed -e 's/#.*//' -e 's/^[[:space:]]*//' -e 's/[[:space:]]*$//' "$r_conf" 2>/dev/null | grep -v '^$' | head -n 1
}
r_ssh() { "${HOLLOW_SLOT_SSH:-ssh}" -o BatchMode=yes -o ConnectTimeout=10 "$@"; }
# Step timing (overnight N3, 2026-11-02; one Phase 1 gate call spent 20 s
# before its command with nothing said). r_step <name> adds the whole seconds
# since the last step (r_t) to r_setup as "<seconds> <name>", steps split by
# ;. This side's steps go to the host in HOLLOW_SLOT_SETUP, set in front of
# the command there (a host tool without this ignores it), the host adds its
# own, and r_note <what> <when> says one line naming the steps of a second or
# more when they add up to HOLLOW_SLOT_SETUP_SAY (5) seconds or more. Each
# side's steps add up to its own elapsed time, so rounding is under a second
# a side. The run's own ssh login is the one step timed across the two
# clocks (r_login), so it is left out when they disagree.
r_step() {
  r_now=$(date +%s)
  r_setup="${r_setup:+$r_setup;}$((r_now - r_t)) $1"
  r_t="$r_now"
}
# The run's own ssh login, from the caller's clock when it started ssh ($1)
# to this host's now: added only when the two plainly agree (0 to 600 s).
r_login() {
  case "$1" in '' | *[!0-9]* | ???????????*) return 0 ;; esac
  [ "$r_t" -ge "$1" ] && [ $((r_t - $1)) -le 600 ] || return 0
  r_setup="${r_setup:+$r_setup;}$((r_t - $1)) ssh login"
}
r_note() {
  r_min="${r_say:-${HOLLOW_SLOT_SETUP_SAY:-5}}"
  case "$r_min" in '' | *[!0-9]* | ??????*) r_min=5 ;; esac
  printf '%s' "$r_setup" | awk -v what="$1" -v when="$2" -v min="$r_min" 'BEGIN { RS = ";" }
    $1 ~ /^[0-9]+$/ { n = $1 + 0; sub(/^[0-9]+ +/, ""); total += n; if (n >= 1) list = list (list == "" ? "" : ", ") $0 " " n " s" }
    END { if (total > 0 && total >= min) printf "hollow-slot: %s took %d s %s: %s\n", what, total, when, list }' >&2
}
# What to check when ssh couldn't reach the host (overnight N2, 2026-11-02),
# from ssh's last error line: the same errors Hollow's own message for a
# turn matches (remote_status.rs, likely_causes), in shorter words.
r_why() {
  case "$1" in
    *"Could not resolve hostname"* | *"nodename nor servname"* | *"Name or service not known"*) echo "this Mac can't look up its address: if it is on a tailnet, check that Tailscale is running and connected here" ;;
    *"Connection refused"*) echo "it is on but not taking SSH connections: Remote Login is off there, or it is still starting up" ;;
    *"Permission denied"* | *"Too many authentication failures"*) echo "it refused this Mac's key: check the IdentityFile for it in ~/.ssh/config and that key in its ~/.ssh/authorized_keys" ;;
    *"Host key verification failed"* | *"IDENTIFICATION HAS CHANGED"*) echo "its host key isn't the one this Mac knows: check ~/.ssh/known_hosts" ;;
    *"Connection closed"* | *"Connection reset"* | *kex_exchange_identification* | *"Broken pipe"*) echo "it dropped the connection while logging in (after several failed logins it refuses for a while)" ;;
    *) echo "it may be off, asleep or restarting; after a restart it waits at the FileVault unlock screen until someone signs in on it" ;;
  esac
}
# Routing (2026-11-03; the rules are in the header). /usr/bin/perl -e
# "$route_pl" <checkout top, or empty> <folder> <command...> prints, split by
# \037: need (why it must run here; empty when it can go), class (heavy,
# quick, or empty when unknown), label (what the line calls it), key (its
# history key: the words that gave its kind, else its first words, with
# " +files" when it names files), named (1 when it names files) and needs
# (the runner tools it needs: rust, node, playwright, uv, python3,
# postgres, and macos for a Mac runner). The string after a shell option
# cluster with c (sh -c '...') is read as words, its ; & | ( ) kept as
# separators. HOLLOW_SLOT_JOB=1 (a
# --send call) lets a media job go; HOLLOW_SLOT_PG=1 (a --postgres call)
# brings its own database. A heavy kind found anywhere in the command wins
# over a quick one.
route_pl='use strict; use warnings;
my ($top, $pwd, @args) = @ARGV;
my $home = $ENV{HOME} // "";
my $canon = sub { my $p = shift; $p =~ s{\A/(var|tmp|etc)(?=/|\z)}{/private/$1}; $p };
$top = $canon->($top) if length $top;
my @fetched = grep { length } map { s{/+\z}{}r } split /\n/, ($ENV{HOLLOW_SLOT_FETCHED} // "");
my @w; my $prev = ""; my $redirect;
for my $a (@args) {
  if ($prev =~ /\A-[A-Za-z]*c[A-Za-z]*\z/) {
    (my $t = $a) =~ s/\$\{(\w+)\}/\$$1/g;
    # A file the command writes with > or >> stays where it runs, unless
    # --fetch brings it back. Quoted text (an awk program, a grep pattern)
    # and [[ ]] or (( )) tests are no redirects (review r3).
    (my $r = $t) =~ s/\x27[^\x27]*\x27|"(?:[^"\\]|\\.)*"/ /g;
    $r =~ s/\[\[.*?\]\]|\(\(.*?\)\)/ /g;
    while ($r =~ /(?<![<>])(?:&|\d*)>(?:>|\|)?\s*([^\s&|;<>()]+)/g) {
      my $f = $1;
      next if $f =~ m{\A(?:/dev/(?:null|stdout|stderr|fd/\d+)|&\d*|-)\z} || !length $f;
      (my $rel = $f) =~ s{\A\./}{};
      next if grep { $rel eq $_ || index($rel, "$_/") == 0 } @fetched;
      $redirect //= $f;
    }
    push @w, grep { defined && length } split /([;&|()]+)|[\s<>`"\x27{}]+/, $t;
  }
  else { push @w, $a }
  $prev = $a;
}
my ($need, $class, $label, $named) = ("", "", "", 0);
my $must = sub { $need = $_[0] unless length $need };
$must->("it writes $redirect with a > redirect, which would stay where it runs") if defined $redirect;
my $ckey = "";
my $i_now = 0;
my %heavy_at;
my $is = sub { $heavy_at{$i_now} = 1 if $_[0] eq "heavy"; ($class, $label, $ckey, $named) = (@_[0 .. 2], $_[3] // 0) if !length($class) || ($class eq "quick" && $_[0] eq "heavy") };
my $sep = sub { $_[0] =~ /\A[;&|()]+\z/ };
my $pg = length($ENV{HOLLOW_SLOT_PG} // "");
my $job = length($ENV{HOLLOW_SLOT_JOB} // "");
my $dbvar = qr/\A(?:DATABASE_URL|DIRECT_URL|SHADOW_DATABASE_URL|PG(?:HOST|PORT|DATABASE|USER|PASSWORD|SERVICE)|POSTGRES_[A-Z_]*URL[A-Z_]*)\z/;
for my $n (sort keys %ENV) {
  $must->("it uses $n from this Mac\x27s environment, a database this Mac reaches") if $n =~ $dbvar && length($ENV{$n}) && !$pg;
}
$must->("it uses gcloud credentials from this Mac\x27s environment") if length($ENV{GOOGLE_APPLICATION_CREDENTIALS} // "");
my %value_opt = map { $_ => 1 } qw(--config -c --project --root -r --dir --reporter --outputFile --pool --shard -t --testNamePattern --environment --mode --maxWorkers --minWorkers --exclude --workers -j --grep -g --retries --timeout --repeat-each --output -o --max-failures --features -p --package --bin --test --target --manifest-path --prefix -C --workspace -w --loglevel --cwd --filter -F);
my $short = sub { my $t = shift; length($t) > 80 ? substr($t, 0, 77) . "..." : $t };
my $outside = sub {
  my $p = shift;
  $p =~ s/\A[^=\/]*=(?=[\/~.\$])//;
  $p =~ s{\A~(?=/|\z)}{$home};
  $p =~ s{\A\$(?:\{HOME\}|HOME)(?=/|\z)}{$home};
  my $abs = $p =~ m{\A/};
  return 1 if $p =~ m{\A\$(?:\{TMPDIR\}|TMPDIR|\{HOLLOW_SCRATCH_DIR\}|HOLLOW_SCRATCH_DIR)(?:/|\z)};
  if ($p !~ m{\A/}) {
    return 0 unless $p =~ m{(?:\A|/)\.\.(?:/|\z)};
    $p = "$pwd/$p";
  }
  my @parts;
  for (split m{/}, $p) { if ($_ eq "..") { pop @parts } elsif (length && $_ ne ".") { push @parts, $_ } }
  $p = $canon->("/" . join("/", @parts));
  my $h = length($home) ? $canon->($home) : "";
  # This Mac\x27s own path to the checkout: the runner\x27s copy is elsewhere.
  return 2 if $abs && length($top) && index("$p/", "$top/") == 0;
  return 0 unless (length($h) && index("$p/", "$h/") == 0) || $p =~ m{\A/(?:private/(?:tmp|var)|Volumes|Applications)(?:/|\z)};
  return 0 if length($top) && index("$p/", "$top/") == 0;
  return 1;
};
my $names = sub {
  my ($p, $shown) = @_;
  my $o = $outside->($p);
  $must->("it names " . $short->($shown) . ", outside this checkout") if $o == 1;
  $must->("it names " . $short->($shown) . ", this Mac\x27s own path into the checkout (the runner\x27s copy is elsewhere; write it relative)") if $o == 2;
};
# Whether the words from $j on name particular files (not ., a folder or a
# glob, which mean a whole suite): 1, else 0.
my $positional = sub {
  my $j = shift;
  for (my $k = $j; $k < @w; $k++) {
    my $y = $w[$k];
    return 0 if $sep->($y);
    next if $k == $j && $y =~ /\A(?:run|watch|related|bench)\z/;
    if ($y =~ /\A-/) { $k++ if $value_opt{$y}; next }
    next if $y eq "." || $y =~ m{/\z} || $y =~ /[*?\[]/ || -d "$pwd/$y" || -d $y;
    return 1;
  }
  return 0;
};
my %needs;
$needs{postgres} = 1 if $pg;
# A step that readies the rest (npm ci, a frozen-lockfile install, prisma
# generate; smart routing 6): wherever the command runs, its output serves
# the next steps, so it doesn\x27t keep the command here when a heavy step
# follows it (after && or ;). Alone, or before nothing heavy, it stays a need.
my @setup;
my $setup = sub { push @setup, [$_[0], $i_now] };
my $heavy_after = sub {
  my $i = shift;
  my $after = 0;
  for my $k ($i + 1 .. $#w) { $after = 1 if $w[$k] =~ /(?:&&|;)/ && $sep->($w[$k]); return 1 if $after && $heavy_at{$k} }
  return 0;
};
# Whether a test runner (vitest, jest, playwright, npm test or npm run test*)
# comes before word $i in its own part of the command (review r3: -u for
# sort or set is no snapshot update).
my $runner_before = sub {
  my $i = shift;
  for (my $k = $i - 1; $k >= 0 && !$sep->($w[$k]); $k--) {
    return 1 if $w[$k] =~ m{(?:\A|/)(?:vitest|jest|playwright)\z};
    return 1 if $w[$k] =~ /\A(?:test|t|test:.*)\z/ && $k > 0 && $w[$k - 1] =~ /\A(?:npm|pnpm|yarn|run|run-script)\z/;
  }
  return 0;
};
# npm test or npm run test*: files after a -- make it one test file\x27s run.
my $test_run = sub {
  my ($j, $key) = @_;
  for (my $k = $j; $k < @w && !$sep->($w[$k]); $k++) {
    next unless $w[$k] eq "--";
    return $is->("quick", "tests on named files", $key, 1) if $positional->($k + 1);
    last;
  }
  $is->("heavy", "tests", $key);
};
# Whether word $i is in command position: first in its part, after VAR=
# words, or the program a wrapper runs (uvx, env, exec, time, nice, nohup,
# command, uv run, pipx run).
my $cmd_word = sub {
  my $i = shift; my $k = $i - 1;
  $k-- while $k >= 0 && $w[$k] =~ /\A[A-Za-z_][A-Za-z0-9_]*=/;
  return 1 if $k < 0 || $sep->($w[$k]);
  return 1 if $w[$k] =~ m{(?:\A|/)(?:uvx|env|exec|time|nice|nohup|command)\z};
  return $w[$k] eq "run" && $k > 0 && $w[$k - 1] =~ m{(?:\A|/)(?:uv|pipx)\z} ? 1 : 0;
};
my $writer_script = qr/(?:\A|:)(?:fix|format|fmt|write|update|regen|regenerate|generate|gen|codegen|snapshots?|bless)(?::|\z)/;
for (my $i = 0; $i < @w; $i++) {
  $i_now = $i;
  my $x = $w[$i];
  my $n1 = $w[$i + 1] // "";
  (my $b = $x) =~ s{.*/}{};
  if ($x =~ /\A([A-Za-z_][A-Za-z0-9_]*)=(.*)\z/s) {
    my ($n, $v) = ($1, $2);
    $must->("it gives the command $n, a database this Mac reaches") if $n =~ $dbvar && (!$pg || $v !~ /\$/);
    $must->("it writes files into this checkout ($n)") if $n !~ /(?:\A|_)NO_/ && $n =~ /(?:\A|_)(?:REGENERATE|REGEN|UPDATE|BLESS|SNAPSHOTS?)(?:_|\z)/;
    $must->("it uses gcloud credentials ($n)") if $n =~ /\A(?:GOOGLE_APPLICATION_CREDENTIALS|CLOUDSDK_)/;
    $names->($v, $v);
    next;
  }
  $names->($x, $x);
  $needs{rust} = 1 if $b =~ /\A(?:cargo|rustc|tauri|cargo-.*)\z/ || ($x =~ /\Arun(?:-script)?\z/ && $n1 =~ /\A(?:gate|gate:rust|perf|tauri|tauri:.*)\z/);
  $needs{node} = 1 if $b =~ /\A(?:npm|npx|node|pnpm|yarn|vitest|jest|tsc|vue-tsc|eslint|next|vite|playwright|hyperframes|prettier)\z/;
  $needs{playwright} = 1 if $b eq "playwright";
  $needs{uv} = 1 if $b =~ /\A(?:uv|uvx|mlx_whisper|mlx-whisper)\z/;
  $needs{python3} = 1 if $b =~ /\A(?:python|python3|pytest|tox|nox)\z/;
  $needs{postgres} = 1 if $b =~ /\A(?:psql|pg_ctl|postgres|createdb|dropdb|pg_dump|pg_restore|initdb)\z/;
  # Only a Mac runner can take these (a Linux runner, 2026-11-03): Apple\x27s
  # tools, and Playwright\x27s webkit project (WKWebView is what it stands in for).
  # MLX (mlx_whisper, mlx-lm) runs on Apple silicon alone: an mlx- or mlx_
  # name as the command (first in its part, after VAR= words, or run by
  # uvx, env, exec, time, nice, nohup, command, uv run or pipx run) or as
  # uvx\x27s --from (review: not python3 tools/mlx_bench.py, nor rg mlx_ src).
  # A webkit browser:
  # --project=webkit, --browser webkit, vitest\x27s --browser.name=webkit, and
  # --project="webkit" inside sh -c, whose quote split leaves --project= and
  # the name as the next word.
  $needs{macos} = 1 if $b =~ /\A(?:codesign|xcodebuild|xcrun|swift|swiftc)\z/ || ($b =~ /\Amlx[-_]/ && ($cmd_word->($i) || ($i > 0 && $w[$i - 1] eq "--from"))) || $x =~ /\A--from=mlx[-_]/ || ($b =~ /\A(?:tauri|cargo-tauri)\z/ && $n1 =~ /\A(?:build|bundle)\z/);
  if ($x =~ /\A--(?:project|browser(?:\.name)?)(?:=(.*))?\z/s) { my $v = defined($1) && length($1) ? $1 : $n1; $needs{macos} = 1 if $v =~ /webkit/i }
  if ($b =~ /\A(?:gcloud|gsutil|bq|cloud-sql-proxy|cloud_sql_proxy)\z/) { $must->("it runs $b, which is signed in only on this Mac") }
  elsif ($b eq "gh") { $must->("it runs gh, and only this Mac is signed in to GitHub") }
  elsif ($b eq "git") { $must->("it runs git, which works on this checkout here") }
  elsif ($b =~ /\A(?:ssh|scp|sftp|rsync|mosh)\z/) { $must->("it runs $b with this Mac\x27s keys") }
  elsif ($b =~ /\A(?:docker|podman|colima|vercel|firebase|netlify|fly|flyctl|wrangler|heroku|railway|aws|az)\z/) { $must->("it runs $b, which is set up only on this Mac") }
  elsif ($b =~ /\A(?:security|codesign|notarytool|spctl|productsign|altool|xcodebuild)\z/) { $must->("it runs $b, which needs this Mac\x27s keychain, signing or Xcode") }
  elsif ($b =~ /\A(?:osascript|screencapture|open)\z/) { $must->("it runs $b, which needs this Mac\x27s screen") }
  elsif (($b eq "npm" && $n1 eq "ci") || ($b =~ /\A(?:pnpm|yarn|bun)\z/ && $n1 eq "install" && grep { /\A--(?:frozen-lockfile|immutable)\z/ } @w)) { $setup->("$b $n1 installs packages into this checkout") }
  elsif ($b =~ /\A(?:npm|pnpm|yarn|bun)\z/ && $n1 =~ /\A(?:ci|install|i|add|remove|rm|uninstall|update|up|upgrade|link|rebuild|dedupe|prune)\z/) { $must->("$b $n1 installs packages into this checkout") }
  elsif ($b eq "yarn" && ($n1 eq "" || $n1 =~ /\A-/ || $sep->($n1))) { $must->("yarn installs packages into this checkout") }
  elsif ($b =~ /\A(?:npm|pnpm|yarn)\z/ && $n1 eq "publish") { $must->("$b publish publishes from this Mac") }
  elsif ($b =~ /\A(?:pip|pip3)\z/ && $n1 =~ /\A(?:install|uninstall)\z/) { $must->("$b $n1 installs packages") }
  elsif ($b eq "uv" && $n1 =~ /\A(?:pip|sync|add|remove|lock|tool|venv)\z/) { $must->("uv $n1 installs packages or writes this checkout") }
  elsif ($b =~ /\A(?:poetry|bundle|brew)\z/ && $n1 =~ /\A(?:install|add|remove|lock|update|upgrade|uninstall|reinstall)\z/) { $must->("$b $n1 installs packages") }
  elsif ($b eq "prisma" && (($n1 eq "migrate" && ($w[$i + 2] // "") =~ /\A(?:dev|diff)\z/) || ($n1 eq "db" && ($w[$i + 2] // "") eq "pull"))) { $must->("prisma $n1 " . $w[$i + 2] . " writes into this checkout") }
  elsif ($b eq "prisma" && $n1 =~ /\A(?:migrate|db)\z/ && !$pg) { $must->("prisma $n1 writes into a database this Mac reaches") }
  elsif ($b eq "prisma" && $n1 eq "generate") { $setup->("prisma generate writes into this checkout") }
  elsif ($b eq "cargo" && $n1 =~ /\A(?:fmt|fix|update|add|remove|install)\z/ && !($n1 eq "fmt" && grep { $_ eq "--check" } @w)) { $must->("cargo $n1 writes files here") }
  elsif ($b eq "cargo" && $n1 =~ /\A(?:run|r)\z/) { $must->("cargo run runs a program whose effects belong here") }
  elsif (($b =~ /\A(?:pytest|tox|nox)\z/ || ($x eq "-m" && $n1 eq "pytest")) && !grep { $_ eq "uvx" || m{(?:\A|/)uv\z} } @w) { $must->("Python tests use this Mac\x27s environment; uv run pytest brings its own there") }
  elsif ($b =~ /\A(?:vitest|jest)\z/ && $n1 eq "bench") { $must->("a benchmark measures the machine it runs on") }
  elsif (($b eq "cargo" && $n1 eq "bench") || ($x =~ /\Arun(?:-script)?\z/ && $n1 =~ /\A(?:perf|bench)(?::|\z)/)) { $must->("a benchmark measures the machine it runs on") }
  elsif ($x =~ /\A(?:--fix|--write|--update-snapshots|--update|--bless)\z/ || ($x eq "-u" && $runner_before->($i)) || ($x eq "-w" && grep { m{(?:\A|/)prettier\z} } @w)) { $must->("it writes files into this checkout ($x)") }
  elsif ($x =~ /\Arun(?:-script)?\z/ && $n1 =~ $writer_script) { $must->("run $n1 writes files into this checkout") }
  elsif (($b =~ /\A(?:tauri|cargo-tauri)\z/ && $n1 =~ /\A(?:build|bundle)\z/) || $x =~ /\Atauri:(?:build|bundle|rebuild)\z/) { $must->("an app build, which stays on this Mac") }
  elsif (($b =~ /\A(?:next|tauri|astro|nuxt|remix|vite)\z/ && $n1 =~ /\A(?:dev|start|preview|serve)\z/)
    || ($b =~ /\A(?:npm|pnpm|yarn|bun)\z/ && $n1 =~ /\A(?:dev|start|serve)\z/)
    || ($x =~ /\Arun(?:-script)?\z/ && $n1 =~ /\A(?:dev|start|serve|preview|storybook)(?::|\z)/)
    || $x eq "tauri:dev" || $b =~ /\A(?:storybook|http-server|serve)\z/ || ($x eq "-m" && $n1 eq "http.server")
    || ($b eq "vite" && ($n1 eq "" || $n1 =~ /\A-/ || $sep->($n1)))) { $must->("a dev server, which runs here: hollow-slot --dev-server <command>") }
  if ($x =~ /\Arun(?:-script)?\z/) {
    if ($n1 =~ /\Agate(?::|\z)/) { $is->("heavy", "gate", "run $n1") }
    elsif ($n1 eq "test:scripts") { $is->("heavy", "script tests", "run $n1") }
    elsif ($n1 =~ /\A(?:test|test:.*|e2e|e2e:.*)\z/) { $test_run->($i + 2, "run $n1") }
    elsif ($n1 =~ /\A(?:typecheck|type-check|tsc)\z/) { $is->("heavy", "typecheck", "run $n1") }
    elsif ($n1 =~ /\Alint(?::|\z)/) { $is->("heavy", "lint", "run $n1") }
    elsif ($n1 =~ /\Abuild(?::|\z)/) { $is->("heavy", "build", "run $n1") }
  }
  elsif ($b =~ /\A(?:npm|pnpm|yarn)\z/ && $n1 =~ /\A(?:test|t)\z/) { $test_run->($i + 2, "run test") }
  elsif ($b eq "cargo" && $n1 =~ /\A(?:build|b|test|t|check|c|clippy|nextest|doc)\z/) { $is->("heavy", "cargo $n1", "cargo $n1") }
  elsif ($b eq "cargo" && $n1 eq "fmt") { $is->("quick", "cargo fmt", "cargo fmt") }
  elsif ($b =~ /\A(?:vitest|jest)\z/) { my $f = $positional->($i + 1); $is->($f ? ("quick", "$b on named files", $b, 1) : ("heavy", "the whole $b suite", $b)) }
  elsif ($b eq "playwright" && $n1 eq "test") { $is->("heavy", "Playwright", "playwright test") }
  elsif ($b eq "node" && $n1 eq "--test") {
    my $n = 0;
    for (my $k = $i + 2; $k < @w && !$sep->($w[$k]); $k++) { next if $w[$k] =~ /\A-/; $n += ($w[$k] =~ /[*?\[]/ || -d "$pwd/$w[$k]") ? 2 : 1 }
    $is->($n == 1 ? ("quick", "one script test file", "node --test", 1) : ("heavy", "script tests", "node --test"));
  }
  elsif ($b =~ /\A(?:tsc|vue-tsc)\z/) { $is->("heavy", "typecheck", $b) }
  elsif ($b =~ /\A(?:eslint|biome|oxlint)\z/ || ($b eq "next" && $n1 eq "lint")) { my $f = $positional->($i + ($b eq "next" ? 2 : 1)); $is->($f ? ("quick", "lint on named files", $b, 1) : ("heavy", "lint", $b)) }
  elsif (($b =~ /\A(?:next|vite|turbo|nx)\z/ && $n1 eq "build") || $b =~ /\A(?:webpack|rollup|tsup)\z/) { $is->("heavy", "build", "$b build") }
  elsif ($b =~ /\A(?:pytest|tox|nox)\z/ || ($x eq "-m" && $n1 eq "pytest")) { $is->("heavy", "Python tests", "pytest") }
  elsif ($b =~ /\A(?:mlx_whisper|mlx-whisper|whisper|whisper-cli)\z/ || ($b eq "hyperframes" && $n1 eq "render") || $b eq "ffmpeg") {
    my $what = $b eq "ffmpeg" || $b eq "hyperframes" ? "render" : "transcription";
    if ($job) { $is->("heavy", $what, $b) }
    else { $must->("a $what reads and writes media here; hollow-slot --send <its files> --fetch <its output> runs it on the runner") }
  }
  elsif ($b eq "swift" && $n1 =~ /\A(?:build|test)\z/) { $is->("heavy", "swift $n1", "swift $n1") }
}
# A Rust build or gate of a Tauri checkout builds a WebKit app (Hollow\x27s own
# links AppKit and WebKit): a Mac runner only.
$needs{macos} = 1 if $needs{rust} && length($top) && -f "$top/src-tauri/tauri.conf.json";
for my $s (@setup) { $must->($s->[0]) unless $heavy_after->($s->[1]) }
my @k;
for (my $i = 0; $i < @w && @k < 3; $i++) {
  my $y = $w[$i];
  if (!@k) {
    next if $y =~ /\A[A-Za-z_][A-Za-z0-9_]*=/ || $y =~ /\A-/ || $sep->($y) || $y =~ m{(?:\A|/)(?:env|exec|time|nice|nohup|command|sh|bash|zsh|dash)\z};
    if ($y eq "cd") { $i++; next }
  }
  last if $sep->($y) || ($y =~ /\A-/ && !(@k == 1 && $k[0] =~ m{(?:\A|/)node\z} && $y eq "--test"));
  push @k, $y;
}
my $key = (length $ckey ? $ckey : join(" ", @k)) . ($named ? " +files" : "") . ($pg ? " +postgres" : "");
$key = $short->($key);
$label = $key unless length $label;
for ($need, $label, $key) { s/[\x00-\x1f]+/ /g }
print join("\x1f", $need, $class, $label, $key, $named, join(" ", sort keys %needs)), "\n";'
# How a duration of $1 seconds reads in a line.
r_dur() {
  if [ "$1" -lt 90 ]; then
    printf '%s s' "$1"
  elif [ "$1" -lt 5400 ]; then
    printf '%s min' "$((($1 + 30) / 60))"
  else
    printf '%s h %s min' "$(($1 / 3600))" "$((($1 % 3600 + 30) / 60))"
  fi
}
# Keep file $1 under $2 lines: past it, its last $3 stay (a line another
# call appends meanwhile can be lost; it is a record, not a ledger).
r_trim() {
  [ -f "$1" ] || return 0
  [ "$(wc -l < "$1" 2>/dev/null | tr -d ' ')" -gt "$2" ] 2>/dev/null || return 0
  tail -n "$3" "$1" > "$1.$$" 2>/dev/null && mv -f "$1.$$" "$1" 2>/dev/null
  rm -f "$1.$$" 2>/dev/null
}
# route-log, a line per decision: time, where (here or the host), why,
# label, repository and what chose it (auto, --auto, --here, --remote, --on,
# Builds and tests, which is HOLLOW_SLOT_ROUTE, --send), split by tabs.
route_log() {
  printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "$1" "$(printf '%s' "$2" | tr '\t\n' '  ')" "$rt_label" "$rt_repo" "$rt_source" >> "$dir/route-log" 2>/dev/null
  r_trim "$dir/route-log" 400 200
}
# route-history, a line per finished run: time, repository, key, where,
# seconds, exit code, and 1 when it changed the checkout.
route_record() {
  [ -n "$rt_key" ] || return 0
  printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$(date +%s)" "$rt_repo" "$rt_key" "$1" "$2" "$3" "${4:-0}" >> "$dir/route-history" 2>/dev/null
  r_trim "$dir/route-history" 2000 1000
}
# History of key $2 in repository $1, as "<usual> <passes> <wrote>
# <there> <here passed>": the median seconds of the last 9 runs that passed
# (- when none did) and how many those are; 1 when any kept run changed the
# checkout (review r2: within the last 9 only, a writer went back there
# every tenth run); the exit code of the latest run on a runner (- when
# none ran there); 1 when one of the last 9 passed here; when that latest
# run there was (seconds since 1970, - for none). Runs that didn't
# pass never count toward a usual time (a dev server stopped after an hour,
# a build that failed at once).
route_median() {
  [ -f "$dir/route-history" ] || return 0
  R_REPO="$1" R_KEY="$2" awk -F '\t' '$2 == ENVIRON["R_REPO"] && $3 == ENVIRON["R_KEY"] && $5 ~ /^[0-9]+$/ { n++; s[n] = $5; e[n] = $6; w[n] = $7; h[n] = ($4 == "here"); t[n] = $1 }
    END {
      if (n == 0) exit
      m = 0; wrote = 0; there = "-"; hp = 0
      for (i = n; i >= 1; i--) if (w[i] == 1) wrote = 1
      for (i = n; i >= 1 && i > n - 9; i--) if (h[i] && e[i] == 0) hp = 1
      tat = "-"
      for (i = n; i >= 1; i--) if (!h[i]) { there = e[i]; tat = t[i]; break }
      for (i = n; i >= 1 && i > n - 9; i--) if (e[i] == 0) { m++; v[m] = s[i] }
      med = "-"
      if (m > 0) {
        for (i = 1; i <= m; i++) for (j = i + 1; j <= m; j++) if (v[j] + 0 < v[i] + 0) { x = v[i]; v[i] = v[j]; v[j] = x }
        med = (m % 2) ? v[(m + 1) / 2] : int((v[m / 2] + v[m / 2 + 1]) / 2)
      }
      print med " " m " " wrote " " there " " hp " " tat
    }' "$dir/route-history" 2>/dev/null
}
# Whether runner $rt_host didn't answer in the last HOLLOW_SLOT_DOWN_SECS
# (300) seconds (runner-down; a time ahead of this clock doesn't count):
# sets rt_why.
route_down() {
  [ -r "$dir/runner-down" ] || return 1
  d_at= d_host= d_msg=
  read -r d_at d_host d_msg < "$dir/runner-down" 2>/dev/null
  case "$d_at" in '' | *[!0-9]*) return 1 ;; esac
  d_age=$(($(date +%s) - d_at))
  d_for="${HOLLOW_SLOT_DOWN_SECS:-300}"
  case "$d_for" in '' | *[!0-9]*) d_for=300 ;; esac
  [ "$d_host" = "$rt_host" ] && [ "$d_age" -ge 0 ] && [ "$d_age" -lt "$d_for" ] || return 1
  rt_why="$rt_name didn't answer $(r_dur "$d_age") ago ($d_msg); asking it again $(r_dur "$d_for") after that"
}
# This side's free memory, slots and power, for route_auto: rt_mem (empty
# with no reading), rt_floor, rt_busy (1 when both slots are taken) and
# rt_batt (1 on battery). HOLLOW_SLOT_PMSET is for this tool's tests.
route_pressure() {
  rt_floor="${HOLLOW_SLOT_MIN_FREE_PCT:-30}"
  case "$rt_floor" in "" | *[!0-9]*) rt_floor=30 ;; esac
  rt_mem=$(mem_free) || rt_mem=
  rt_busy=1
  for n in 0 1; do
    in_slot "$dir/slot-$n.lock" /usr/bin/true
    [ "$?" -eq 75 ] || rt_busy=0
  done
  rt_batt=0
  case "$("${HOLLOW_SLOT_PMSET:-/usr/bin/pmset}" -g batt 2>/dev/null | head -n 1)" in *"Battery Power"*) rt_batt=1 ;; esac
}
# How a line names runner tool $1.
cap_name() {
  case "$1" in
    rust) printf 'Rust' ;;
    node) printf 'Node.js' ;;
    python3) printf 'Python 3' ;;
    postgres) printf 'Postgres' ;;
    playwright) printf 'a Playwright browser' ;;
    macos) printf 'macOS' ;;
    *) printf '%s' "$1" ;;
  esac
}
# The routing line's reason when runner $rt_name lacks what the command needs
# (route_lacks set rt_lack, rt_lack_os and rt_caps_age).
lack_why() {
  if [ -n "$rt_lack_os" ]; then
    rt_why="$rt_name runs $rt_lack_os and $rt_label needs macOS (it said so $(r_dur "$rt_caps_age") ago)"
  else
    rt_why="$rt_label needs $rt_lack, which $rt_name doesn't have (it said so $(r_dur "$rt_caps_age") ago)"
  fi
}
# Whether runner $1, by its last report (runner-caps: what a preparation
# found, at most HOLLOW_SLOT_CAPS_SECS (600) old), lacks a tool the command
# needs ($rt_needs). Sets rt_lack (the missing ones, readable; rt_lack_os,
# caps_lack) and rt_caps_age (seconds). Right after this call's own preparation,
# route_after_prepare reads that call's report instead (caps_lack).
route_lacks() {
  rt_lack=
  [ -n "$rt_needs" ] && [ -r "$dir/runner-caps" ] || return 1
  c_at= c_host= c_caps=
  read -r c_at c_host c_caps < "$dir/runner-caps" 2>/dev/null
  case "$c_at" in '' | *[!0-9]*) return 1 ;; esac
  [ "$c_host" = "$1" ] || return 1
  rt_caps_age=$(($(date +%s) - c_at))
  [ "$rt_caps_age" -ge 0 ] && [ "$rt_caps_age" -lt "${HOLLOW_SLOT_CAPS_SECS:-600}" ] || return 1
  caps_lack "$c_caps"
}
# Whether caps line $1 lacks a tool the command needs ($rt_needs): sets
# rt_lack (the missing ones, readable). macos (a Linux runner, 2026-11-03)
# is the runner's system, its os=: one that names none predates the Linux
# port, so it is a Mac. rt_lack_os is then the system it runs (Linux).
caps_lack() {
  rt_lack=
  rt_lack_os=
  for c_need in $rt_needs; do
    if [ "$c_need" = macos ]; then
      case " $1 " in
        *" os=darwin "* | *" os= "*) ;;
        *" os=linux "*) rt_lack_os=Linux ;;
        *" os="*) c_os=" $1 "; c_os="${c_os##* os=}"; rt_lack_os="${c_os%% *}" ;;
      esac
      [ -z "$rt_lack_os" ] || rt_lack="${rt_lack:+$rt_lack, }$(cap_name macos)"
      continue
    fi
    case " $1 " in
      *" $c_need="[0-9A-Za-z]*) ;;
      *) rt_lack="${rt_lack:+$rt_lack, }$(cap_name "$c_need")" ;;
    esac
  done
  [ -n "$rt_lack" ]
}
# After the runner's preparation (run_remote, run_job): what it reports now
# against the command. A routed run (not --remote or --on) that needs a tool
# it lacks comes back here; one routed automatically comes back too when the
# runner is busy (both its slots, or its Rust build lock for a Rust build)
# while this Mac has a free slot, memory and power. --remote and --on only
# warn. Returns 1 with fallback set when it comes back.
route_after_prepare() {
  [ "$rt_where" = there ] || return 0
  if [ -n "$r_caps" ] && caps_lack "$r_caps"; then
    # A Linux runner and a command that needs macOS: said as such.
    case "$rt_source:$rt_lack_os" in
      --remote:?* | --on:?*) say "$host runs $rt_lack_os and this command needs macOS; running it there anyway, as --remote asked" ;;
      --remote: | --on:) say "$host reports no $rt_lack, which this command needs; running it there anyway, as --remote asked" ;;
      *:?*) fallback="$rt_name runs $rt_lack_os and this command needs macOS"; return 1 ;;
      *) fallback="$rt_name has no $rt_lack, which this command needs"; return 1 ;;
    esac
  fi
  [ "$route" = auto ] || return 0
  m_busy=
  case " $r_caps " in *" slots=2/2 "*) m_busy="both its build slots are taken" ;; esac
  case " $rt_needs " in *" rust "*) case " $r_caps " in *" cargo=busy "*) m_busy="${m_busy:-its Rust build lock is taken}" ;; esac ;; esac
  [ -n "$m_busy" ] || return 0
  route_pressure
  [ "$rt_busy" = 0 ] && [ "$rt_batt" = 0 ] || return 0
  [ -z "$rt_mem" ] || [ "$rt_floor" -eq 0 ] || [ "$rt_mem" -ge "$rt_floor" ] || return 0
  case " $rt_needs " in *" rust "*) ! cargo_busy || return 0 ;; esac
  fallback="$rt_name is busy ($m_busy) and this Mac has a free build slot"
  return 1
}
# Where an automatic run goes (route.pl's rules, then history, then
# pressure, in that order): sets rt_where (here or there) and rt_why.
# $rt_host is the runner, $rt_name how the line calls it.
route_auto() {
  rt_where=here
  rt_why="$rt_need"
  [ -z "$rt_need" ] || return 0
  if [ -z "$rt_top" ] && [ -z "$send" ]; then
    rt_why="this folder isn't in a git checkout, which is how files get to $rt_name"
    return 0
  fi
  route_down && return 0
  rt_quick="${HOLLOW_SLOT_QUICK_SECS:-5}"
  rt_long="${HOLLOW_SLOT_LONG_SECS:-10}"
  case "$rt_quick" in '' | *[!0-9]*) rt_quick=5 ;; esac
  case "$rt_long" in '' | *[!0-9]*) rt_long=10 ;; esac
  rt_med= rt_npass=0 rt_wrote=0 rt_tfail=- rt_hpass=0 rt_tat=-
  set -- $(route_median "$rt_repo" "$rt_key")
  [ "$#" -lt 6 ] || { rt_med="$1"; rt_npass="$2"; rt_wrote="$3"; rt_tfail="$4"; rt_hpass="$5"; rt_tat="$6"; }
  [ "$rt_med" != - ] || rt_med=
  rt_usual="${rt_med:+, usually $(r_dur "$rt_med")}"
  # A command that changed the checkout, whatever its kind, runs where its
  # changes land (review of #1113: a heavy one used to keep going there).
  if [ "$rt_wrote" = 1 ]; then
    rt_why="$rt_label changed this checkout when it ran, so it runs where its changes land"
    return 0
  fi
  # One that couldn't start there (126, 127: a tool it lacks), whatever its
  # kind, or one of no known kind that failed there and passes here: there
  # lacks what it needs. A known kind's other failures there are its own.
  case "$rt_tfail" in
    - | 0) ;;
    126 | 127)
      # For a day (HOLLOW_SLOT_START_FAIL_SECS): the runner may get the tool
      # (review r3: it used to stay here for good). --remote tries it now.
      case "$rt_tat" in '' | *[!0-9]*) rt_tat=0 ;; esac
      if [ $(($(date +%s) - rt_tat)) -lt "${HOLLOW_SLOT_START_FAIL_SECS:-86400}" ]; then
        rt_why="$rt_label couldn't start on $rt_name last time (exit $rt_tfail), so it runs here; hollow-slot --remote tries it there again"
        return 0
      fi
      ;;
    *)
      if [ -z "$rt_class" ] && [ "$rt_hpass" = 1 ]; then
        rt_why="$rt_label failed on $rt_name last time (exit $rt_tfail) and passes here, so it runs here"
        return 0
      fi
      ;;
  esac
  # Whether it may go when this side is under pressure: a known kind, or a
  # command of no known kind with two passing runs timed and no changes.
  rt_can=1
  case "$rt_class" in
    heavy)
      if [ -n "$rt_med" ] && [ "$rt_med" -lt "$rt_quick" ]; then
        rt_why="$rt_label$rt_usual, less than sending it to $rt_name takes"
      else
        rt_where=there
        rt_why="$rt_label$rt_usual"
      fi
      ;;
    quick)
      if [ -n "$rt_med" ] && [ "$rt_med" -ge "$rt_long" ]; then
        rt_where=there
        rt_why="$rt_label$rt_usual"
      else
        rt_why="$rt_label$rt_usual; sending it to $rt_name would take about as long"
      fi
      ;;
    *)
      # No known kind: it may need something only here that its words don't
      # show, so it goes only on two passing runs' evidence.
      if [ "$rt_npass" -eq 0 ]; then
        rt_can=0
        rt_why="$rt_label: no timing for it yet, so it runs here this time"
      elif [ "$rt_npass" -lt 2 ]; then
        rt_can=0
        rt_why="$rt_label$rt_usual, timed once so far, so it runs here again"
      elif [ "$rt_med" -ge "$rt_long" ]; then
        rt_where=there
        rt_why="$rt_label$rt_usual"
      else
        rt_why="$rt_label$rt_usual; sending it to $rt_name would take about as long"
      fi
      ;;
  esac
  route_pressure
  if [ "$rt_where" = there ]; then
    if route_lacks "$rt_host"; then
      rt_where=here
      lack_why
      return 0
    fi
    [ -z "$rt_mem" ] || rt_why="$rt_why; this Mac ${rt_mem}% free"
    return 0
  fi
  [ "$rt_can" = 1 ] || return 0
  if [ -n "$rt_mem" ] && [ "$rt_floor" -gt 0 ] && [ "$rt_mem" -lt "$rt_floor" ]; then
    rt_where=there
    rt_why="$rt_label; this Mac has ${rt_mem}% of its memory free, under its ${rt_floor}% floor"
  elif [ "$rt_busy" = 1 ]; then
    rt_where=there
    rt_why="$rt_label; both of this Mac's build slots are in use"
  elif [ "$rt_batt" = 1 ]; then
    rt_where=there
    rt_why="$rt_label; this Mac is on battery"
  fi
  if [ "$rt_where" = there ] && route_lacks "$rt_host"; then
    rt_where=here
    lack_why
  fi
}
# Set up the host (stdin: this script), run there by its own /bin/sh: the
# folders, this tool as ~/hollow-remote/bin/hollow-slot (moved into place,
# so a run reading the old one keeps it), the host's own
# hollow-run-preflight (exit 4 when it says no, after one prune that never
# takes worktree folder $2's: a disk under the preflight's floor recovers)
# and the bare repository $1. Says the preflight's status line.
r_prep='root="$HOME/hollow-remote"
mkdir -p "$root/bin" "$root/repos" "$root/work" "$root/targets" || exit 3
IFS= read -r ident || exit 3
tmp="$root/bin/.hollow-slot.$$"
cat > "$tmp" || { rm -f "$tmp"; exit 3; }
if cmp -s "$tmp" "$root/bin/hollow-slot"; then
  rm -f "$tmp"
else
  chmod 755 "$tmp" && mv -f "$tmp" "$root/bin/hollow-slot" || exit 3
fi
status="ready; no hollow-run-preflight there"
preflight=$(command -v hollow-run-preflight 2>/dev/null)
[ -n "$preflight" ] || [ ! -x "$HOME/.local/bin/hollow-run-preflight" ] || preflight="$HOME/.local/bin/hollow-run-preflight"
if [ -n "$preflight" ]; then
  if ! status=$("$preflight" 2>&1 </dev/null); then
    /bin/sh "$root/bin/hollow-slot" --remote-prune "$2" < /dev/null >&2
    if ! status=$("$preflight" 2>&1 </dev/null); then
      printf "%s\n" "$status" | tail -n 1 >&2
      exit 4
    fi
  fi
fi
# Made aside and renamed into place (a perl rename fails on a repository that
# is already there), because two gates of one repository on its first run (R0:
# two lanes) prepare at once, and two git init into one folder collide.
if [ -n "$1" ] && [ ! -f "$root/repos/$1.git/HEAD" ]; then
  init="$root/repos/.init-$1.$$"
  git init -q --bare "$init" >/dev/null || { rm -rf "$init"; exit 3; }
  /usr/bin/perl -e "rename(\$ARGV[0], \$ARGV[1]) or exit 1" "$init" "$root/repos/$1.git" 2>/dev/null || rm -rf "$init"
  [ -f "$root/repos/$1.git/HEAD" ] || exit 3
fi
case "$2.$3" in *[!A-Za-z0-9._-]*) ident= ;; esac
if [ -n "$ident" ] && [ -n "$3" ]; then
  (umask 077 && printf "%s\n" "$ident" > "$root/work/$2.$3.ident") || exit 3
fi
tools="$root/caps-tools"
# os= (a Linux runner, 2026-11-03) names the system; a list kept from before
# it had none, so it is read again at once.
if [ -z "$(find "$tools" -mmin -10 2>/dev/null)" ] || ! grep -q " os=" "$tools" 2>/dev/null; then
  os=$(uname -s 2>/dev/null | LC_ALL=C tr A-Z a-z)
  ver() { "$@" 2>/dev/null < /dev/null | head -n 1 | sed -n "s/[^0-9]*\([0-9][0-9.]*\).*/\1/p"; }
  pg=$(command -v pg_ctl 2>/dev/null)
  if [ -z "$pg" ] && [ -n "${HOLLOW_REMOTE_PG_DIRS+set}" ]; then
    for d in $HOLLOW_REMOTE_PG_DIRS; do [ -n "$pg" ] || [ ! -x "$d/pg_ctl" ] || pg="$d/pg_ctl"; done
  elif [ -z "$pg" ]; then
    pg=$(for d in /opt/homebrew/opt/postgresql@*/bin /usr/local/opt/postgresql@*/bin; do [ ! -x "$d/pg_ctl" ] || printf "%s\n" "$d/pg_ctl"; done | sed -n "s/.*@\([0-9][0-9]*\)\/bin\/pg_ctl$/\1 &/p" | sort -n | tail -n 1 | cut -d " " -f 2-)
    [ -n "$pg" ] || pg=$(for d in /usr/lib/postgresql/*/bin; do [ ! -x "$d/pg_ctl" ] || printf "%s\n" "$d/pg_ctl"; done | sed -n "s/.*postgresql\/\([0-9][0-9]*\)\/bin\/pg_ctl$/\1 &/p" | sort -n | tail -n 1 | cut -d " " -f 2-)
  fi
  py=$(command -v python3 2>/dev/null)
  [ "$py" != /usr/bin/python3 ] || [ "$os" != darwin ] || xcode-select -p > /dev/null 2>&1 || py=
  pw=$(ls -d "$HOME/Library/Caches/ms-playwright"/chromium-* "${XDG_CACHE_HOME:-$HOME/.cache}/ms-playwright"/chromium-* 2>/dev/null | sed -n "s/.*chromium-\([0-9][0-9]*\)$/\1/p" | sort -n | tail -n 1)
  printf "rust=%s node=%s uv=%s python3=%s postgres=%s playwright=%s gcloud=%s os=%s\n" "$(ver rustc --version)" "$(ver node --version)" "$(ver uv --version)" "${py:+$(ver "$py" --version)}" "${pg:+$(ver "$pg" --version)}" "${pw:+chromium-$pw}" "$(command -v gcloud > /dev/null 2>&1 && echo yes)" "$os" > "$tools.$$" && mv -f "$tools.$$" "$tools"
fi
held=0
d="${HOLLOW_BUILD_SLOT_DIR:-/tmp/hollow-build-slots-$(id -u)}"
free() { [ ! -f "$1" ] || /usr/bin/perl -e "use Fcntl qw(:flock); open(my \$f, q(>>), \$ARGV[0]) or exit 0; exit(flock(\$f, LOCK_EX | LOCK_NB) ? 0 : 1)" "$1"; }
for n in 0 1; do free "$d/slot-$n.lock" || held=$((held + 1)); done
# cargo=busy only when every Rust lane this runner may use now is taken (a
# lane 1 needs HOLLOW_SLOT_RUST_LANES not 1 and the memory floor met: the
# tool says, --remote-lanes), so a Rust build is sent here while one is free.
cargo=busy
lanes=$(/bin/sh "$root/bin/hollow-slot" --remote-lanes 2>/dev/null < /dev/null)
case "$lanes" in 1 | 2) ;; *) lanes=1 ;; esac
if free "$d/cargo.lock"; then
  cargo=free
elif [ "$lanes" = 2 ] && free "$d/cargo-1.lock"; then
  cargo=free
fi
# Who can log in (remote runners 2): the keys in its authorized_keys files
# (each once), how many are not the owner key ~/hollow-remote/owner.json
# records, and whether one is recorded.
okey=
[ ! -f "$root/owner.json" ] || okey=$(head -c 4096 "$root/owner.json" | sed -n "s|.*\"key\" *: *\"\(SHA256:[A-Za-z0-9+/=]*\)\".*|\1|p" | head -n 1)
kg=/usr/bin/ssh-keygen
[ -x "$kg" ] || kg=ssh-keygen
keys=0 others=0
for fp in $(for f in "$HOME/.ssh/authorized_keys" "$HOME/.ssh/authorized_keys2"; do [ ! -f "$f" ] || "$kg" -l -E sha256 -f "$f" 2>/dev/null; done | cut -d " " -f 2 | sort -u); do
  keys=$((keys + 1))
  [ "$fp" = "$okey" ] || others=$((others + 1))
done
owner=none
[ -z "$okey" ] || owner=yes
printf "caps %s slots=%s/2 cargo=%s lanes=%s keys=%s others=%s owner=%s\n" "$(cat "$tools" 2>/dev/null)" "$held" "$cargo" "$lanes" "$keys" "$others" "$owner"
printf "%s\n" "$status" | tail -n 1'
# Owner-only runners (remote runners 2, 2026-11-03): warn when the
# runner account lets in an SSH key that isn't its recorded owner's, or
# several keys with no owner recorded (caps line $1: keys=, others=,
# owner=). Settings, Devices, Remote runner shows which.
r_keys_note() {
  r_k=$(printf '%s\n' "$1" | sed -n 's/.* keys=\([0-9][0-9]*\).*/\1/p')
  r_o=$(printf '%s\n' "$1" | sed -n 's/.* others=\([0-9][0-9]*\).*/\1/p')
  r_w=$(printf '%s\n' "$1" | sed -n 's/.* owner=\([a-z]*\).*/\1/p')
  if [ "$r_w" = yes ] && [ "${r_o:-0}" -gt 0 ]; then
    say "warning: $host lets $r_o SSH key(s) in besides its owner's (~/.ssh/authorized_keys there): whoever holds one can run as its runner account and read what runs there. Settings, Devices, Remote runner shows it."
  elif [ "$r_w" = none ] && [ "${r_k:-0}" -gt 1 ]; then
    say "warning: $host lets $r_k SSH keys in and has no owner recorded: whoever holds one can run as its runner account. Record yourself as owner in Settings, Devices, Remote runner, and remove keys you don't recognise there."
  fi
}
# Prepare $host for a run (r_prep, there: its folders, this tool, its
# preflight, bare repository $1 unless empty) with folder $2 kept from any
# prune; status is the preflight's line. With call token $3, the checkout's
# git identity ($r_ident, the first line of r_prep's stdin, before this
# tool) waits there for that call. Returns 1 with fallback set when it can't
# take the run.
r_prepare() {
  status=$({ printf '%s\n' "${3:+${r_ident:-}}"; cat "$0"; } | r_ssh "$host" "/bin/sh -c $(q "$r_prep") hollow-slot $(q "$1") $(q "$2") $(q "${3:-}")" 2> "$scratch/err")
  case "$?" in
    0)
      r_step "ssh and preflight"
      grep '^hollow-slot: ' "$scratch/err" >&2
      # The runner's tools and load (2026-11-03, smart routing 2), kept here
      # for routing (route_caps); the status is the preflight's line.
      r_caps=$(printf '%s\n' "$status" | sed -n 's/^caps //p' | head -n 1)
      status=$(printf '%s\n' "$status" | grep -v '^caps ' | tail -n 1)
      [ -z "$r_caps" ] || printf '%s %s %s\n' "$(date +%s)" "$host" "$r_caps" > "$dir/runner-caps.$$" 2>/dev/null && mv -f "$dir/runner-caps.$$" "$dir/runner-caps" 2>/dev/null
      rm -f "$dir/runner-caps.$$" 2>/dev/null
      r_keys_note "$r_caps"
      return 0
      ;;
    255) fallback="$host can't be reached ($(tail -n 1 "$scratch/err"))"; fallback_why=$(r_why "$(tail -n 1 "$scratch/err")"); r_unreached=$(tail -n 1 "$scratch/err") ;;
    4) fallback="$host isn't taking work now ($(tail -n 1 "$scratch/err"))" ;;
    *) fallback="$host couldn't be set up ($(tail -n 1 "$scratch/err"))" ;;
  esac
  return 1
}
# The git identity this Mac uses for checkout $1, as git var gives it (its
# includes and includeIf honoured, GIT_AUTHOR_* here winning as they would
# for a commit here): author name, author email, committer name, committer
# email, separated by \037. Empty when none is configured here
# (user.useConfigOnly: git's guess from this account is never sent).
r_identity() {
  r_ia=$(git -C "$1" -c user.useConfigOnly=true var GIT_AUTHOR_IDENT 2>/dev/null) || return 0
  r_ic=$(git -C "$1" -c user.useConfigOnly=true var GIT_COMMITTER_IDENT 2>/dev/null) || return 0
  printf '%s\n%s\n' "$r_ia" "$r_ic" | /usr/bin/perl -ne 'chomp; (/\A(.*?) *<([^<>\037]+)> \d+ [-+]\d{4}\z/ && length($1) && $1 !~ /\037/) or exit 0; push @f, $1, $2; END { print join("\037", @f) if @f == 4 }'
}
# Outside the run itself, a signal removes the scratch folder (and an old
# copy a copy back set aside, r_del) and ends the call (128 + the signal);
# the caller resets the traps before a fallback.
r_trap() {
  trap 'rm -rf ${r_del:+"$r_del"} "${scratch:?}"; exit 130' INT
  trap 'rm -rf ${r_del:+"$r_del"} "${scratch:?}"; exit 143' TERM
  trap 'rm -rf ${r_del:+"$r_del"} "${scratch:?}"; exit 129' HUP
}
# Nothing is set aside yet (and never a path from the environment).
r_del=
# Run "$@" on SSH host $1. Sets rc to the command's exit code, or fallback
# to why it didn't run there (nothing ran on the host then).
run_remote() {
  host="$1"
  shift
  r_t=$(date +%s)
  r_setup=
  case "$host" in
    -* | *[!A-Za-z0-9._@-]*) fallback="$host is not an SSH host name"; return ;;
  esac
  top=$(git rev-parse --show-toplevel 2>/dev/null) || { fallback="this folder isn't in a git checkout, which is how the files get there"; return; }
  prefix=$(git rev-parse --show-prefix 2>/dev/null)
  common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || { fallback="git couldn't name the repository"; return; }
  head=$(git rev-parse --verify -q 'HEAD^{commit}' 2>/dev/null) || { fallback="the checkout has no commit yet"; return; }
  [ -r "$0" ] || { fallback="its own script can't be read to send"; return; }
  scratch=$(mktemp -d "${TMPDIR:-/tmp}/hollow-slot.XXXXXX") || { fallback="no temp folder here"; return; }
  r_trap
  repo_id="$(slug_of "${common%/.git}")-$(hash_of "$common")"
  wt_hash=$(hash_of "$top")
  wt_id="$(slug_of "$top")-$wt_hash"
  # targets/session-* are remote sessions' build folders there.
  case "$wt_id" in session-*) wt_id="w-$wt_id" ;; esac
  sha="$head"
  clean=1
  # A git status that fails is no clean tree (2026-11-03): read as one, HEAD
  # went alone and the uncommitted changes were silently left out.
  git --no-optional-locks -C "$top" status --porcelain --untracked-files=all > "$scratch/status" 2> "$scratch/err" ||
    { fallback="git status failed here, so the uncommitted changes can't be told ($(tail -n 1 "$scratch/err"))"; return; }
  if [ -s "$scratch/status" ]; then
    clean=0
    mkdir "$scratch/objects" || { fallback="no temp folder here"; return; }
    cp "$(git -C "$top" rev-parse --path-format=absolute --git-path index)" "$scratch/index" 2>/dev/null || rm -f "$scratch/index"
    sha=$(
      cd "$top" || exit 1
      GIT_INDEX_FILE="$scratch/index" GIT_OBJECT_DIRECTORY="$scratch/objects" GIT_ALTERNATE_OBJECT_DIRECTORIES="$common/objects"
      export GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
      git add -A . >/dev/null 2>&1 || exit 1
      tree=$(git write-tree) || exit 1
      GIT_AUTHOR_NAME=hollow-slot GIT_AUTHOR_EMAIL=hollow-slot@localhost GIT_COMMITTER_NAME=hollow-slot GIT_COMMITTER_EMAIL=hollow-slot@localhost \
        git -c commit.gpgSign=false commit-tree "$tree" -p "$head" -m "hollow-slot: uncommitted changes on $head"
    ) || { fallback="the uncommitted changes couldn't be gathered"; return; }
  fi
  r_ident=$(r_identity "$top")
  token="$$-$(date +%s)"
  r_step "reading the checkout"
  r_prepare "$repo_id" "$wt_id" "$token" || return
  if ! route_after_prepare; then
    # The identity r_prepare left there for this call: nothing will use it.
    (r_ssh "$host" "rm -f \"\$HOME/hollow-remote/work/\"$(q "$wt_id.$token.ident")" < /dev/null > /dev/null 2>&1 &)
    return
  fi
  if ! (
    if [ "$clean" = 0 ]; then
      GIT_OBJECT_DIRECTORY="$scratch/objects" GIT_ALTERNATE_OBJECT_DIRECTORIES="$common/objects"
      export GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
    fi
    GIT_SSH_COMMAND="$(q "${HOLLOW_SLOT_SSH:-ssh}") -o BatchMode=yes -o ConnectTimeout=10" GIT_SSH_VARIANT=ssh
    export GIT_SSH_COMMAND GIT_SSH_VARIANT
    # HEAD goes along as <hash>-base: git sends only what the host lacks,
    # judged by the host's refs this side also has, and a snapshot commit
    # exists only there. Without a base, every push sent the whole history.
    git -C "$top" push -q --no-verify --force "$host:hollow-remote/repos/$repo_id.git" "$head:refs/hollow-slot/$wt_hash-base" "$sha:refs/hollow-slot/$wt_hash"
  ) < /dev/null 2> "$scratch/err"; then
    fallback="the checkout couldn't be sent to $host ($(tail -n 1 "$scratch/err"))"
    (r_ssh "$host" "rm -f \"\$HOME/hollow-remote/work/\"$(q "$wt_id.$token.ident")" < /dev/null > /dev/null 2>&1 &)
    return
  fi
  r_step push
  # --fetch paths are the command's folder's, like its own arguments; the
  # host packs them while it still holds the folder.
  r_fetch=
  while IFS= read -r p; do
    [ -z "$p" ] || r_fetch="$r_fetch$prefix$p
"
  done <<EOF
$fetch
EOF
  r_cmd="HOLLOW_SLOT_SETUP=$(q "$r_setup")${HOLLOW_SLOT_SETUP_SAY:+ HOLLOW_SLOT_SETUP_SAY=$(q "$HOLLOW_SLOT_SETUP_SAY")} \"\$HOME/hollow-remote/bin/hollow-slot\" --remote-run $(q "$repo_id") $(q "$wt_id") $sha $(q "$prefix") $clean $token $(q "${HOLLOW_SLOT_CARGO:-}") $(q "$r_fetch") --"
  for arg in "$@"; do
    r_cmd="$r_cmd $(q "$arg")"
  done
  if [ "$clean" = 1 ]; then
    say "running on $host, not this Mac ($status), at $(printf %.8s "$head")"
  else
    say "running on $host, not this Mac ($status), at $(printf %.8s "$head") plus this checkout's uncommitted changes"
  fi
  r_go
}
# Run $r_cmd on $host and bring back what it packed as $wt_id.$token: the
# --fetch paths ($r_fetch) into folder $top here, and, for a checkout's run
# ($common set), the gate record of commit $sha and the gate's run records.
# Sets rc; Stop ends the command there.
r_go() {
  mkfifo "$scratch/alive" || { fallback="no pipe for the connection here"; return; }
  # This process alone holds the pipe's writing end: when it ends, however
  # it ends, ssh's stdin ends, and so does the command on the host.
  exec 9<> "$scratch/alive"
  started=$(date +%s)
  stopped=
  trap 'stopped=130; exec 9>&-' INT
  trap 'stopped=143; exec 9>&-' TERM
  trap 'stopped=129; exec 9>&-' HUP
  "${HOLLOW_SLOT_SSH:-ssh}" -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 "$host" "HOLLOW_SLOT_SETUP_SENT=$(date +%s) $r_cmd" < "$scratch/alive" 9>&- &
  r_pid=$!
  wait "$r_pid"
  rc=$?
  if [ -n "$stopped" ]; then
    n=0
    while kill -0 "$r_pid" 2>/dev/null && [ "$n" -lt 60 ]; do
      sleep 0.25
      n=$((n + 1))
    done
    kill "$r_pid" 2>/dev/null
    wait "$r_pid" 2>/dev/null
    rc="$stopped"
    say "stopped; the command on $host was stopped too"
    return
  fi
  exec 9>&-
  r_trap
  r_back=$(date +%s)
  # Collected after every run (a gate's run record comes back whatever the
  # tree), except after a dropped connection: another try would only wait
  # out the connect timeout.
  if [ "$rc" -eq 255 ]; then
    say "ssh exited 255: the connection to $host failed, or the command did; nothing was copied back"
  elif ! r_ssh "$host" "\"\$HOME/hollow-remote/bin/hollow-slot\" --remote-collect $(q "$wt_id") $token" < /dev/null > "$scratch/back.tar"; then
    say "the results couldn't be copied back from $host"
  elif [ "$rc" -eq 247 ] && [ ! -s "$scratch/back.tar" ]; then
    # The host's own setup failed (exit 247 and, since its command never
    # ran, no results pack): nothing ran there. A command's own 247 leaves
    # its pack, so it reads as its exit code (review r3).
    fallback="nothing ran on $host: it couldn't get the checkout ready (its line above says why)"
    return
  elif [ -s "$scratch/back.tar" ]; then
    if ! { mkdir "$scratch/back" && tar -xf "$scratch/back.tar" -C "$scratch/back"; }; then
      say "the results from $host couldn't be unpacked here"
    elif [ -L "$scratch/back/records" ] || [ -L "$scratch/back/runs" ] || [ -L "$scratch/back/wt" ] || [ -L "$scratch/back/changed" ] || [ -L "$scratch/back/ignored" ]; then
      # The host runs branch code: a pack whose top folder is a symlink would
      # move files from wherever it points into this checkout.
      say "not copying anything back from $host: its results pack holds a symlinked folder"
    else
      r_n=0
      # What the command changed in the checkout there (2026-11-03), other
      # than what --fetch brings back: it stays there, so say so, and an
      # automatic run of it stays here from then on (route-history).
      if [ -n "$common" ] && [ -f "$scratch/back/changed" ] && [ -s "$scratch/back/changed" ]; then
        r_wrote=1
        r_nch=$(wc -l < "$scratch/back/changed" | tr -d ' ')
        [ "$r_nch" -lt 50 ] || r_nch="50 or more"
        r_list=$(head -n 5 "$scratch/back/changed" | LC_ALL=C tr -cd '[:print:]\n' | tr '\n' ' ')
        say "the command changed $r_nch path(s) in the checkout on $host, which don't come back here (${r_list% }); to change them here, run it with hollow-slot --here"
      fi
      # What it wrote that git ignores, for a build, Playwright or a command
      # of no known kind (whose output may be the point): said, not
      # remembered. Not for tests, gates and checks, whose ignored output
      # (Tauri's gen/, a provenance file) is incidental.
      case "$rt_class:$rt_label" in
        :* | *:build | *:"cargo build" | *:"cargo b" | *:Playwright | heavy:"swift build") r_say_ignored=1 ;;
        *) r_say_ignored= ;;
      esac
      if [ -n "$r_say_ignored" ] && [ -n "$common" ] && [ -f "$scratch/back/ignored" ] && [ -s "$scratch/back/ignored" ]; then
        r_list=$(head -n 5 "$scratch/back/ignored" | LC_ALL=C tr -cd '[:print:]\n' | tr '\n' ' ')
        say "it also wrote ${r_list% } on $host, which stay there (ignored files); hollow-slot --fetch <path> brings one back, --here builds it here"
      fi
      # A job (--send) has no checkout here: no gate records go anywhere.
      [ -n "$common" ] || rm -rf "${scratch:?}/back/records" "${scratch:?}/back/runs"
      for f in "$scratch/back/records/"*.json; do
        [ -f "$f" ] && [ ! -L "$f" ] || continue
        name="${f##*/}"
        printf '%s\n' "$name" | grep -Eq '^[0-9a-f]{64}\.json$' || continue
        grep -q "\"commit\": \"$sha\"" "$f" || continue
        mkdir -p "$common/hollow-gate-records" && mv -f "$f" "$common/hollow-gate-records/$name" &&
          say "the gate record for this tree came back to this checkout's gate records"
      done
      for f in "$scratch/back/runs/"*.json; do
        [ -f "$f" ] && [ ! -L "$f" ] || continue
        name="${f##*/}"
        printf '%s\n' "$name" | grep -Eq '^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}-[0-9]{2}-[0-9]{2}-[0-9]{3}Z-([0-9a-f]{8}|unknown0)\.json$' || continue
        mkdir -p "$common/hollow-gate-runs" && mv -f "$f" "$common/hollow-gate-runs/$name" &&
          say "the gate's run record came back to this checkout's gate history (scripts/gate-stats.mjs counts it)"
      done
      while IFS= read -r p; do
        [ -n "$p" ] || continue
        [ -e "$scratch/back/wt/$p" ] || [ -L "$scratch/back/wt/$p" ] || continue
        if link_on_way "$top" "$p" || link_on_way "$scratch/back/wt" "$p"; then
          say "not copying $p back: a folder on its way here is a symlink"
          continue
        fi
        # A symlink coming back could point anywhere here (out.json ->
        # ~/.zshrc): the host runs branch code.
        if [ -L "$scratch/back/wt/$p" ]; then
          say "not copying $p back: it is a symlink on $host"
          continue
        fi
        if ! links_inside "$scratch/back/wt/$p"; then
          say "not copying $p back: it holds a symlink that points out of it (an absolute one, or one with a .. part)"
          continue
        fi
        # A .git in it would be a repository here, whose config git (and a
        # prompt that runs git) would obey: the host runs branch code.
        if [ -n "$(find "$scratch/back/wt/$p" -iname .git -print 2>/dev/null | head -n 1)" ]; then
          say "not copying $p back: it holds a .git"
          continue
        fi
        # The swap ignores INT, TERM and HUP (so do the mv's and rm's it
        # runs; a stop's trap runs once it ends): the old copy goes aside, the
        # new one in, and only then is the old one deleted, so a stop or a
        # failed move always leaves one of them here. Before, a stop between
        # rm -rf and mv lost both (the trap deletes the scratch folder). The
        # old copy waits beside the path under a dot name (2026-11-03), not in
        # the scratch folder: TMPDIR can be another volume, where moving it
        # aside was a whole copy with signals ignored, and a failed restore
        # left it inside a partial new copy, or lost it with the scratch
        # folder. A failed move in (one from another volume copies) can leave
        # part of the new copy, so that goes before the old one comes back.
        # A name already taken there is skipped, not emptied: it can be an
        # old copy an earlier run said it couldn't put back.
        r_try=0
        while :; do
          r_n=$((r_n + 1))
          r_old="$(dirname "$top/$p")/.${p##*/}.hollow-slot-old.$$.$r_n"
          [ -e "$r_old" ] || [ -L "$r_old" ] || break
          r_try=$((r_try + 1))
          [ "$r_try" -lt 100 ] || break
        done
        if [ -e "$r_old" ] || [ -L "$r_old" ]; then
          say "not copying $p back: the names beside it that the old copy would wait under are all taken"
          continue
        fi
        # The old copy is deleted after the swap, outside it (2026-11-03), so
        # signals are ignored only for the two renames. Stop signals the
        # whole group and sends SIGKILL 3 s after its SIGTERM, so a big old
        # copy can still be left half deleted beside the path (the next run
        # sends it as a change); renaming it out of the work tree first is
        # the open fix. A stop that comes during the swap waits for it
        # (r_sig), then deletes it like any other stop.
        r_sig=
        trap 'r_sig=130' INT
        trap 'r_sig=143' TERM
        trap 'r_sig=129' HUP
        (
          trap '' INT TERM HUP
          mkdir -p "$(dirname "$top/$p")" || exit 1
          if [ -e "$top/$p" ] || [ -L "$top/$p" ]; then
            mv "$top/$p" "$r_old" || exit 1
          fi
          mv "$scratch/back/wt/$p" "$top/$p" && exit 0
          rm -rf "$top/$p"
          [ -e "$r_old" ] || [ -L "$r_old" ] || exit 1
          mv "$r_old" "$top/$p" && exit 1
          exit 3
        )
        r_st=$?
        [ "$r_st" -ne 0 ] || r_del="$r_old"
        r_trap
        if [ -n "$r_sig" ]; then
          [ "$r_st" -ne 3 ] || say "couldn't copy $p back from $host, nor put the old copy back: it is at $r_old"
          rm -rf ${r_del:+"$r_del"} "${scratch:?}"
          exit "$r_sig"
        fi
        case $r_st in
          0) say "copied $p back from $host" ;;
          3) say "couldn't copy $p back from $host, nor put the old copy back: it is at $r_old" ;;
          *) say "couldn't copy $p back from $host" ;;
        esac
        [ -z "$r_del" ] || rm -rf "$r_del"
        r_del=
      done <<EOF
$r_fetch
EOF
    fi
  fi
  r_now=$(date +%s)
  r_min="${HOLLOW_SLOT_SETUP_SAY:-5}"
  case "$r_min" in '' | *[!0-9]* | ??????*) r_min=5 ;; esac
  [ "$r_now" -eq "$r_back" ] || [ $((r_now - r_back)) -lt "$r_min" ] || say "copying the results back from $host took $((r_now - r_back)) s"
  say "finished on $host: exit $rc after $((r_now - started)) s"
}
# Run "$@" on SSH host $1 as a job (--send): no checkout, so it works in any
# folder. The --send paths (files or folders under this one) go to a folder
# of the job's own there, the command runs in it, and the --fetch paths come
# back here. Sets rc, or fallback like run_remote.
run_job() {
  host="$1"
  shift
  r_t=$(date +%s)
  r_setup=
  case "$host" in
    -* | *[!A-Za-z0-9._@-]*) fallback="$host is not an SSH host name"; return ;;
  esac
  top=$(pwd -P) || { fallback="this folder can't be read"; return; }
  # Mistakes in what it names stop the call (exit 2) rather than run a long
  # job here instead.
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    [ -e "$top/$p" ] || { say "--send $p: it isn't in this folder"; exit 2; }
    if link_on_way "$top" "$p"; then
      say "--send $p: a folder on the way to it is a symlink"
      exit 2
    fi
  done <<EOF
$send
EOF
  # A fetched path replaces this folder's: a folder here the job didn't get
  # whole (not sent, nor inside what was sent) would lose what wasn't sent.
  while IFS= read -r p; do
    [ -n "$p" ] || continue
    if link_on_way "$top" "$p" || [ -L "$top/$p" ]; then
      say "--fetch $p: it, or a folder on the way to it, is a symlink here, so nothing could come back there"
      exit 2
    fi
    [ -d "$top/$p" ] || continue
    r_cov=
    while IFS= read -r r_s; do
      [ -n "$r_s" ] || continue
      case "$p/" in "$r_s"/*) r_cov=1 ;; esac
    done <<EOF2
$send
EOF2
    if [ -z "$r_cov" ]; then
      say "--fetch $p would replace the folder $p here with the job's copy, which holds only what --send sent; send $p too, or fetch a path the job writes inside it"
      exit 2
    fi
  done <<EOF
$fetch
EOF
  [ -r "$0" ] || { fallback="its own script can't be read to send"; return; }
  scratch=$(mktemp -d "${TMPDIR:-/tmp}/hollow-slot.XXXXXX") || { fallback="no temp folder here"; return; }
  r_trap
  token="$$-$(date +%s)"
  wt_id="job-$token"
  common=
  sha=
  r_prepare "" "$wt_id" || return
  # Nothing was sent yet, so nothing there to drop.
  route_after_prepare || return
  mkfifo "$scratch/up" || { fallback="no pipe for the upload here"; return; }
  # Both ends of the upload run in the background so a Stop (a trap runs only
  # between commands) ends it at once, with what reached the host dropped.
  # ./ keeps a name starting with @ a path for tar; -H sends what a symlink
  # named here points to.
  (
    cd "$top" || exit 1
    set --
    while IFS= read -r p; do
      [ -z "$p" ] || set -- "$@" "./$p"
    done <<EOF
$send
EOF
    exec tar -H -cf - "$@"
  ) > "$scratch/up" 2> "$scratch/tar-err" < /dev/null &
  r_tar=$!
  "${HOLLOW_SLOT_SSH:-ssh}" -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=15 -o ServerAliveCountMax=4 "$host" "\"\$HOME/hollow-remote/bin/hollow-slot\" --job-receive $(q "$wt_id")" < "$scratch/up" > /dev/null 2> "$scratch/err" &
  r_up=$!
  trap 'kill "$r_tar" "$r_up" 2>/dev/null; r_drop; rm -rf "${scratch:?}"; exit 130' INT
  trap 'kill "$r_tar" "$r_up" 2>/dev/null; r_drop; rm -rf "${scratch:?}"; exit 143' TERM
  trap 'kill "$r_tar" "$r_up" 2>/dev/null; r_drop; rm -rf "${scratch:?}"; exit 129' HUP
  wait "$r_up"
  r_sent=$?
  wait "$r_tar"
  r_tared=$?
  r_trap
  if [ "$r_sent" -eq 255 ]; then
    fallback="$host can't be reached ($(tail -n 1 "$scratch/err"))"
    fallback_why=$(r_why "$(tail -n 1 "$scratch/err")")
    r_unreached=$(tail -n 1 "$scratch/err")
    r_drop
    return
  fi
  if [ "$r_tared" -ne 0 ] && ! grep -q "Write error" "$scratch/tar-err" 2>/dev/null; then
    fallback="what it sends couldn't be packed here ($(tail -n 1 "$scratch/tar-err"))"
    r_drop
    return
  fi
  if [ "$r_sent" -ne 0 ] || [ "$r_tared" -ne 0 ]; then
    fallback="what it sends couldn't be sent to $host ($(tail -n 1 "$scratch/err"))"
    r_drop
    return
  fi
  r_step "sending its files"
  r_fetch="$fetch"
  r_cmd="HOLLOW_SLOT_SETUP=$(q "$r_setup")${HOLLOW_SLOT_SETUP_SAY:+ HOLLOW_SLOT_SETUP_SAY=$(q "$HOLLOW_SLOT_SETUP_SAY")} \"\$HOME/hollow-remote/bin/hollow-slot\" --job-run $(q "$wt_id") $token $(q "$r_fetch") --"
  for arg in "$@"; do
    r_cmd="$r_cmd $(q "$arg")"
  done
  say "running on $host, not this Mac ($status), as a job with the files it sends"
  stopped=
  r_go
  # Stopped, or the run never reached the host (or the connection went):
  # drop what was sent, unless the run there claimed it (it deletes its own
  # folder).
  if [ "$rc" -eq 255 ] || [ "$rc" -eq 247 ] || [ -n "$stopped" ]; then
    r_drop
  fi
}
# Best effort, in the background: delete job $wt_id's folder on $host unless
# a --job-run there has claimed it.
r_drop() {
  (r_ssh "$host" "\"\$HOME/hollow-remote/bin/hollow-slot\" --job-drop $(q "$wt_id")" < /dev/null > /dev/null 2>&1 &)
}
# On the host: drop worktree folders past the newest HOLLOW_REMOTE_KEEP, or
# unused for HOLLOW_REMOTE_DAYS days, with their build folders and TMPDIR,
# each only while no run holds its lock, and build folders left without a
# worktree; never $1's. Moved aside at once, deleted in the background. A
# build folder being cloned is named .seed-*, which ls doesn't list; those
# and a run's leftovers (packed results nobody collected) go after a day.
# A remote session's build folder (targets/session-*) is that session's:
# hollow-remote-run's host preparation keeps and prunes those. Free space
# (overnight N5, 2026-11-02): then, while the volume has less than
# HOLLOW_REMOTE_MIN_FREE_GB (40, twice the preflight's usual 20 GB floor) free,
# the least recently used worktree folder goes with its build folder, never
# $1's, deleted before the next is weighed (a clone frees only its own
# blocks, so the volume is asked again each time), and only one unused for
# an hour (a run holds its folder's lock, so this is against clearing a
# whole host each call when something else fills its disk). What the count
# and age rules set aside is deleted first, then; trash an earlier call
# left (a restart cut its deletion short) goes after half an hour.
# HOLLOW_REMOTE_DF stands in for /bin/df in this tool's tests.
r_free_gb() {
  r_df=/bin/df
  case "${HOLLOW_REMOTE_DF:-}" in /*) r_df="$HOLLOW_REMOTE_DF" ;; esac
  "$r_df" -Pk "$root" 2>/dev/null | awk 'NR == 2 && $4 ~ /^[0-9]+$/ { print int($4 / 1048576) }'
}
remote_prune() {
  r_keep="${HOLLOW_REMOTE_KEEP:-8}"
  r_days="${HOLLOW_REMOTE_DAYS:-14}"
  r_trash="$root/trash/$$"
  n=0
  # .?*.used: a folder named before slug_of replaced a leading dot.
  for used in $(cd "$root/work" 2>/dev/null && ls -t -- *.used .?*.used 2>/dev/null); do
    id="${used%.used}"
    n=$((n + 1))
    [ "$id" != "$1" ] || continue
    if [ "$n" -gt "$r_keep" ] || [ -n "$(find "$root/work/$used" -mtime +"$r_days" 2>/dev/null)" ]; then
      mkdir -p "$r_trash" && in_slot "$root/work/$id.lock" /bin/sh -c 'mv "$1/work/$2" "$3/work-$2" 2>/dev/null; mv "$1/targets/$2" "$3/target-$2" 2>/dev/null; rm -rf "$4"; rm -f "$1/work/$2.used" "$1/work/$2.port"' prune "$root" "$id" "$r_trash" "${HOLLOW_REMOTE_TMP:-/tmp}/hs-$(hash_of "$id" 6)"
    fi
  done
  for id in $(ls "$root/targets" 2>/dev/null); do
    case "$id" in session-*) continue ;; esac
    [ "$id" != "$1" ] && [ ! -e "$root/work/$id.used" ] || continue
    mkdir -p "$r_trash" && in_slot "$root/work/$id.lock" mv "$root/targets/$id" "$r_trash/target-$id"
  done
  find "$root/work" -maxdepth 1 \( -name '*.tar' -o -name '*.start' -o -name '*.ident' -o -name '.back.*' \) -mtime +0 -exec rm -rf {} + 2>/dev/null
  find "$root/targets" -maxdepth 1 -name '.seed-*' -mtime +0 -exec rm -rf {} + 2>/dev/null
  find "$root/trash" -mindepth 1 -maxdepth 1 -mmin +30 -exec rm -rf {} + 2>/dev/null
  # A job's folder (and its .run claim) a lost connection left (jobs delete
  # their own) goes after a day, while no job runs.
  [ ! -d "$root/jobs" ] || in_slot "$root/jobs/jobs.lock" find "$root/jobs" -mindepth 1 -maxdepth 1 -type d -name 'job-*' -mtime +0 -exec rm -rf {} + 2>/dev/null
  r_moved=
  [ ! -d "$r_trash" ] || r_moved=1
  r_pruned=
  r_floor="${HOLLOW_REMOTE_MIN_FREE_GB:-40}"
  case "$r_floor" in '' | *[!0-9]*) r_floor=40 ;; esac
  r_free=$(r_free_gb)
  if [ -n "$r_free" ] && [ "$r_free" -lt "$r_floor" ] && [ -d "$r_trash" ]; then
    rm -rf "${r_trash:?}"
  fi
  for used in $(cd "$root/work" 2>/dev/null && ls -tr -- *.used .?*.used 2>/dev/null); do
    r_free=$(r_free_gb)
    [ -n "$r_free" ] && [ "$r_free" -lt "$r_floor" ] || break
    id="${used%.used}"
    [ "$id" != "$1" ] && [ -e "$root/work/$used" ] || continue
    [ -n "$(find "$root/work/$used" -mmin +60 2>/dev/null)" ] || continue
    r_gone="$root/trash/$$-free"
    # A folder another run holds stays (lockf -t 0 gives up at once).
    if mkdir -p "$r_gone" && in_slot "$root/work/$id.lock" /bin/sh -c 'mv "$1/work/$2" "$3/work-$2" 2>/dev/null; mv "$1/targets/$2" "$3/target-$2" 2>/dev/null; rm -rf "$4"; rm -f "$1/work/$2.used" "$1/work/$2.port"' prune "$root" "$id" "$r_gone" "${HOLLOW_REMOTE_TMP:-/tmp}/hs-$(hash_of "$id" 6)"; then
      say "$here had $r_free GB free, under its $r_floor GB floor: removed the least recently used worktree folder there ($id) and its build folder"
      r_pruned=1
    fi
    rm -rf "$r_gone"
  done
  [ -n "$r_moved" ] || [ -n "$r_pruned" ] || return 0
  for repo in "$root/repos/"*.git; do
    [ ! -d "$repo" ] || git -C "$repo" worktree prune < /dev/null 2>/dev/null
  done
  [ -d "$r_trash" ] || return 0
  (cd / && exec nohup rm -rf "$r_trash" < /dev/null > /dev/null 2>&1) &
}
# Whether the caller is gone: ssh's stdin has ended. The caller never writes
# to it, so stdin being readable at all means its end.
caller_gone() { /usr/bin/perl -e 'my $r = ""; vec($r, 0, 1) = 1; exit(select($r, undef, undef, 0) > 0 ? 0 : 1)'; }
# This machine's short name for the lines: hostname -s, or uname -n where a
# Linux runner has no hostname command (2026-11-03).
host_name() {
  h_n=$(hostname -s 2>/dev/null) || h_n=
  [ -n "$h_n" ] || h_n=$(uname -n 2>/dev/null)
  printf '%s' "${h_n%%.*}"
}
# On the host, run by /bin/sh -c under work/ports.lock (text, so a newer or
# older copy of this tool installed meanwhile can't change it): worktree $2's
# port, printed. A worktree folder keeps the one it first got
# (work/<id>.port, dropped with the folder): 2300 + $3 (its id's hash), else
# the next one no other folder in $1 holds. A host keeps HOLLOW_REMOTE_KEEP
# folders, far fewer than 400 ports, so two never share one (by hash alone,
# about once in 400 pairs did).
# On the host, by /usr/bin/perl: point build folder $1's build-script
# outputs at itself ($2 is the folder holding every build folder). Cargo
# build scripts write absolute paths into their outputs (tauri's permission
# lists name files in its own OUT_DIR), so a folder cloned from another kept
# reading that one's and broke once it was pruned (2026-11-02, after the
# first folder there went). Only text files naming another build folder
# change, written aside and moved into place with their old times, so cargo
# sees nothing new. Cheap (a grep), so every run does it: that also mends
# folders cloned before this. Same code: remote_session.rs REMOTE_RUN_PL.
r_repath='use strict; use warnings;
my ($t, $base) = @ARGV; my ($name) = ($t // "") =~ m{/([^/]+)\z}; exit 0 unless defined $name && defined $base;
opendir(my $d, $t) or exit 0;
my @dirs = grep { -d $_ } map { "$t/$_/build" } grep { !/\A\./ } readdir $d;
exit 0 unless @dirs;
my $pid = open(my $g, "-|") // exit 0;
if (!$pid) { open(STDERR, ">", "/dev/null"); exec { "/usr/bin/grep" } "grep", "-rlIF", "--", "$base/", @dirs or exit 127 }
my @files = <$g>; close $g; chomp @files;
for my $f (@files) {
  next if -l $f || !-f $f;
  my @st = stat $f; open(my $i, "<:raw", $f) or next; my $c = do { local $/; <$i> }; close $i; next if index($c, "\0") >= 0;
  (my $n = $c) =~ s{\Q$base/\E(?!\Q$name/\E)[A-Za-z0-9_-][A-Za-z0-9._-]*/}{$base/$name/}g;
  next if $n eq $c;
  my $tmp = "$f.repath.$$"; open(my $o, ">:raw", $tmp) or next; print $o $n; close $o or do { unlink $tmp; next };
  chmod($st[2] & 07777, $tmp); rename($tmp, $f) or do { unlink $tmp; next }; utime($st[8], $st[9], $f);
}'
r_port='work="$1" id="$2" start="$3"
own=$(cat "$work/$id.port" 2>/dev/null)
case "$own" in
  23[0-9][0-9] | 24[0-9][0-9] | 25[0-9][0-9] | 26[0-9][0-9]) printf "%s\n" "$own"; exit 0 ;;
esac
taken=$(for f in "$work/"*.port; do [ "$f" = "$work/$id.port" ] || cat "$f" 2>/dev/null; done)
i=0
while [ "$i" -lt 400 ]; do
  try=$((2300 + (start + i) % 400))
  if ! printf "%s\n" "$taken" | grep -qx "$try"; then
    printf "%s\n" "$try" > "$work/$id.port" || exit 1
    printf "%s\n" "$try"
    exit 0
  fi
  i=$((i + 1))
done
exit 1'
# A host's own failure before the command starts (a checkout, npm ci, its
# temp folder or port, the job's folder) exits 247 (smart routing, review
# r2: it used to be 70, which read as the command failing there): the caller
# then knows nothing ran there and runs the command here.
# On the host, by ssh from run_remote: --remote-run <repo id> <worktree id>
# <commit> <subfolder> <clean 0|1> <token> <HOLLOW_SLOT_CARGO> <fetch paths,
# one a line> -- <command>. It waits for the worktree's lock (one run per
# local worktree at a time), then, holding it (--remote-locked): gives up if
# the caller is already gone, checks the commit out (making the folder again
# when it is broken or another repository's), starts a missing build folder
# from the newest free one of the same repository's (for a checkout that
# tracks a Cargo.toml), runs npm ci when the lock file changed, runs the
# command through this tool in its own process group (a watchdog kills the
# group when stdin ends; what it leaves running is ended after it), and packs
# the fetch paths it wrote and new gate records before letting the folder go.
remote_host_run() {
  mode="$1"
  shift
  [ "$#" -ge 10 ] && [ "$9" = -- ] || { say "$mode: wrong arguments"; exit 2; }
  repo_id="$1" wt_id="$2" sha="$3" prefix="$4" clean="$5" token="$6" cargo="$7" r_fetch="$8"
  shift 9
  for v in "$repo_id" "$wt_id" "$token"; do
    case "$v" in '' | . | .. | *[!A-Za-z0-9._-]*) say "$mode: bad name $v"; exit 2 ;; esac
  done
  case "$sha" in '' | *[!0-9a-f]*) say "$mode: bad commit $sha"; exit 2 ;; esac
  [ -z "$prefix" ] || valid_rel "${prefix%/}" || { say "$mode: bad folder $prefix"; exit 2; }
  root="$HOME/hollow-remote"
  here=$(host_name)
  if [ "$mode" = --remote-run ]; then
    r_t=$(date +%s)
    r_setup="${HOLLOW_SLOT_SETUP:-}"
    r_login "${HOLLOW_SLOT_SETUP_SENT:-}"
    unset HOLLOW_SLOT_SETUP_SENT
    # The calling Mac's git identity for its checkout, which r_prep left for
    # this call alone: the command's commits carry it (exported, so the run
    # that holds the folder has it). Removed at once; none, and git's own
    # default here stays.
    r_identf="$root/work/$wt_id.$token.ident"
    if [ -f "$r_identf" ] && [ ! -L "$r_identf" ]; then
      r_an= r_ae= r_cn= r_ce=
      IFS="$(printf '\037')" read -r r_an r_ae r_cn r_ce < "$r_identf"
      if [ -n "$r_an" ] && [ -n "$r_ae" ] && [ -n "$r_cn" ] && [ -n "$r_ce" ]; then
        GIT_AUTHOR_NAME="$r_an" GIT_AUTHOR_EMAIL="$r_ae" GIT_COMMITTER_NAME="$r_cn" GIT_COMMITTER_EMAIL="$r_ce"
        export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
      fi
    fi
    rm -f "$r_identf"
    remote_prune "$wt_id"
    r_step prune
    # For the run that holds the folder: the steps so far and when this one
    # began waiting for it.
    HOLLOW_SLOT_SETUP="$r_setup" HOLLOW_SLOT_SETUP_AT="$r_t"
    export HOLLOW_SLOT_SETUP HOLLOW_SLOT_SETUP_AT
    in_slot "$root/work/$wt_id.lock" /usr/bin/true
    [ "$?" -ne 75 ] || say "another run from this checkout is using its folder on $here; waiting for it"
    lock_wait "$root/work/$wt_id.lock" /bin/sh "$0" --remote-locked "$repo_id" "$wt_id" "$sha" "$prefix" "$clean" "$token" "$cargo" "$r_fetch" -- "$@"
    exit "$?"
  fi
  r_setup="${HOLLOW_SLOT_SETUP:-}"
  r_t="${HOLLOW_SLOT_SETUP_AT:-}"
  case "$r_t" in '' | *[!0-9]*) r_t=$(date +%s) ;; esac
  r_say="${HOLLOW_SLOT_SETUP_SAY:-}"
  unset HOLLOW_SLOT_SETUP HOLLOW_SLOT_SETUP_AT HOLLOW_SLOT_SETUP_SAY
  r_step "waiting for its folder"
  if caller_gone; then
    say "the call was stopped before the command started on $here (while it waited for its folder); not running it"
    exit 143
  fi
  wt="$root/work/$wt_id"
  repo="$root/repos/$repo_id.git"
  target="$root/targets/$wt_id"
  # Its repository's id, which a new build folder's seed must match.
  printf '%s\n' "$repo_id" > "$root/work/$wt_id.used"
  r_common=
  [ ! -e "$wt/.git" ] || r_common=$(git -C "$wt" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
  if ! {
    [ -n "$r_common" ] &&
      [ "$(cd "$r_common" 2>/dev/null && pwd -P)" = "$(cd "$repo" && pwd -P)" ] &&
      git -C "$wt" checkout -q -f --detach "$sha" < /dev/null && git -C "$wt" clean -q -f -d < /dev/null
  }; then
    [ ! -e "$wt" ] || say "this checkout's folder on $here was broken or another repository's; making it again"
    rm -rf "${wt:?}"
    git -C "$repo" worktree prune < /dev/null
    git -C "$repo" worktree add -q -f --detach "$wt" "$sha" < /dev/null || { say "couldn't check out $sha in $wt on $here"; exit 247; }
  fi
  r_step checkout
  # Seeded only from the same repository's folders (its id in <id>.used;
  # one written before 2026-11-03 is empty, so never a seed) and only for a
  # checkout that tracks a Cargo.toml: another repository's build folder, or
  # any for a repository with no Rust, is gigabytes it can't use.
  if [ ! -d "$target" ] && [ -n "$(git -C "$wt" ls-files -- '*Cargo.toml' 2>/dev/null | head -n 1)" ]; then
    seed="$root/targets/.seed-$wt_id.$$"
    for used in $(cd "$root/work" && ls -t -- *.used 2>/dev/null); do
      other="${used%.used}"
      [ "$other" != "$wt_id" ] && [ -d "$root/targets/$other" ] || continue
      [ "$(head -n 1 "$root/work/$used" 2>/dev/null)" = "$repo_id" ] || continue
      # An APFS clone (cp -c). On a Linux runner only a reflink one (btrfs,
      # XFS; times kept, as a clone keeps them): a whole copy of a build
      # folder takes minutes holding that folder's lock, so where the file
      # system can't clone (ext4: a one-byte file's reflink fails, named so
      # the .seed-* prune takes a stray one) the folder starts empty.
      r_cp="/bin/cp -cR"
      if [ "$hs_os" = Linux ]; then
        r_cp="/bin/cp -a --reflink=always"
        r_probe="$root/targets/.seed-reflink.$$"
        if ! { printf x > "$r_probe" && /bin/cp --reflink=always "$r_probe" "$r_probe.c"; } 2>/dev/null; then
          rm -f "$r_probe" "$r_probe.c"
          say "started this checkout's Rust build folder on $here empty: its file system can't clone $other's (cp --reflink), and a whole copy would hold that folder for minutes"
          break
        fi
        rm -f "$r_probe" "$r_probe.c"
      fi
      if in_slot "$root/work/$other.lock" $r_cp "$root/targets/$other" "$seed" 2>/dev/null && mv "$seed" "$target"; then
        # The clone's build records are newer than these files: touch them,
        # or cargo would take stale test binaries for fresh ones.
        git -C "$wt" ls-files -z | (cd "$wt" && xargs -0 touch)
        say "started this checkout's Rust build folder on $here as a clone of $other's"
        break
      fi
      rm -rf "${seed:?}"
    done
  fi
  [ ! -d "$target" ] || /usr/bin/perl -e "$r_repath" "$target" "$root/targets"
  r_step "build folder"
  cd "$wt" || exit 247
  if [ -f package-lock.json ]; then
    want=$(/usr/bin/shasum -a 256 < package-lock.json | cut -c1-64)
    if [ "$want" != "$(cat node_modules/.hollow-remote-lock 2>/dev/null)" ]; then
      say "npm ci on $here: package-lock.json is new there"
      npm ci --no-audit --no-fund --loglevel=error < /dev/null >&2 || { say "npm ci failed on $here"; exit 247; }
      printf '%s\n' "$want" > node_modules/.hollow-remote-lock
      if [ -x node_modules/.bin/playwright ]; then
        node_modules/.bin/playwright install chromium < /dev/null >&2 || say "Playwright couldn't install its browser on $here; browser tests may fail"
      fi
      r_step "npm ci"
    fi
  fi
  tmp="${HOLLOW_REMOTE_TMP:-/tmp}/hs-$(hash_of "$wt_id" 6)"
  mkdir -p -m 700 "$tmp" 2>/dev/null
  if [ ! -d "$tmp" ] || [ -L "$tmp" ] || [ ! -O "$tmp" ]; then
    say "refusing the temp folder $tmp on $here: a symlink, not a folder, or another user's"
    exit 247
  fi
  CARGO_TARGET_DIR="$target"
  TMPDIR="$tmp"
  PORT=$(lock_wait "$root/work/ports.lock" /bin/sh -c "$r_port" hollow-slot "$root/work" "$wt_id" "$(($(printf '%d' "0x$(hash_of "$wt_id" 4)") % 400))") || { say "couldn't give this checkout a port on $here"; exit 247; }
  r_step "temp folder and port"
  PW_PORT="$PORT"
  # What gate-run.mjs records for a run here: this checkout's own HEAD (a
  # snapshot of uncommitted changes is a commit only this host has).
  HOLLOW_REMOTE_HEAD="$sha"
  HOLLOW_REMOTE_DIRTY=0
  if [ "$clean" != 1 ]; then
    HOLLOW_REMOTE_HEAD=$(git -C "$wt" rev-parse -q --verify "$sha^" 2>/dev/null) || HOLLOW_REMOTE_HEAD="$sha"
    HOLLOW_REMOTE_DIRTY=1
  fi
  : "${CARGO_PROFILE_DEV_DEBUG:=0}" "${CARGO_PROFILE_TEST_DEBUG:=0}"
  : "${CARGO_BUILD_JOBS:=$((($(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 2) + 1) / 2))}"
  : "${VITEST_MAX_WORKERS:=4}" "${VITEST_MAX_THREADS:=4}" "${VITEST_MAX_FORKS:=4}" "${VITEST_MIN_THREADS:=1}" "${VITEST_MIN_FORKS:=1}"
  # Two Rust lanes here (R0, 2026-11-14), unless the runner's own login says
  # HOLLOW_SLOT_RUST_LANES=1; the tool then sets cargo's jobs per lane
  # (HOLLOW_SLOT_LANE_JOBS=0 in the login keeps CARGO_BUILD_JOBS as it is).
  : "${HOLLOW_SLOT_RUST_LANES:=2}" "${HOLLOW_SLOT_LANE_JOBS:=1}"
  HOLLOW_SLOT_REMOTE=off
  PATH="$root/bin:$PATH"
  export HOLLOW_SLOT_RUST_LANES HOLLOW_SLOT_LANE_JOBS
  export CARGO_TARGET_DIR TMPDIR PORT PW_PORT CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG CARGO_BUILD_JOBS VITEST_MAX_WORKERS VITEST_MAX_THREADS VITEST_MAX_FORKS VITEST_MIN_THREADS VITEST_MIN_FORKS HOLLOW_SLOT_REMOTE PATH HOLLOW_REMOTE_HEAD HOLLOW_REMOTE_DIRTY
  unset HOLLOW_SLOT_HOLDER_PID HOLLOW_SLOT_CARGO_PID HOLLOW_SLOT_SAY HOLLOW_SLOT_SAY_ON HOLLOW_SLOT_CARGO
  [ -z "$cargo" ] || { HOLLOW_SLOT_CARGO="$cargo"; export HOLLOW_SLOT_CARGO; }
  cd "$wt/$prefix" || exit 247
  # Again: getting the folder ready (a checkout, a build folder's clone, npm
  # ci) can take minutes, and a call stopped meanwhile must not start.
  if caller_gone; then
    say "the call was stopped before the command started on $here (while its folder was made ready); not running it"
    exit 143
  fi
  r_note setup "before the command on $here"
  marker="$root/work/$wt_id.$token.start"
  host_exec "$@"
  # Nobody is left to collect a stopped call's results.
  caller_gone || host_pack 1
  r_note "wrapping up" "after the command on $here"
  rm -f "$marker"
  exit "$rc"
}
# On the host: run "$@" through this tool (in one of the host's own slots)
# in a process group of its own, $marker there meanwhile (its time is what
# "new" means for gate records); a watchdog kills the group when stdin
# (ssh's) ends, and what the command leaves running is ended after it, since
# it would hold the session's output. Sets rc.
host_exec() {
  : > "$marker"
  exec 3<&0
  /usr/bin/perl -e 'setpgrp(0, 0); exec { $ARGV[0] } @ARGV or exit 127' /bin/sh "$root/bin/hollow-slot" -- "$@" < /dev/null 3<&- &
  r_pid=$!
  (
    while IFS= read -r line; do :; done
    # The group exists once perl has called setpgrp: wait for it (or for the
    # command to be gone), or a TERM sent first would reach nothing.
    n=0
    while ! kill -0 "-$r_pid" 2>/dev/null && kill -0 "$r_pid" 2>/dev/null && [ "$n" -lt 100 ]; do
      sleep 0.05
      n=$((n + 1))
    done
    kill -TERM "-$r_pid" 2>/dev/null
    sleep 10
    kill -KILL "-$r_pid" 2>/dev/null
  ) <&3 > /dev/null 2>&1 &
  r_watch=$!
  exec 3<&-
  wait "$r_pid" 2>/dev/null
  rc=$?
  r_t=$(date +%s)
  r_setup=
  { kill "$r_watch" && wait "$r_watch"; } 2>/dev/null
  # Anything the command left running in its group still holds this
  # session's output, so ssh would wait for it: end it, as gate-run.mjs does.
  if kill -0 "-$r_pid" 2>/dev/null; then
    kill -TERM "-$r_pid" 2>/dev/null
    n=0
    while kill -0 "-$r_pid" 2>/dev/null && [ "$n" -lt 20 ]; do
      sleep 0.25
      n=$((n + 1))
    done
    kill -KILL "-$r_pid" 2>/dev/null
  fi
  r_step "ending what it left running"
}
# On the host: pack the --fetch paths ($r_fetch, in folder $wt) that this run
# wrote (something in each changed after $marker) and, when $1 is 1 (a
# checkout's run), gate records newer than $marker into
# work/$wt_id.$token.tar for --remote-collect.
host_pack() {
  r_git="$1"
  set --
  stage=$(mktemp -d "$root/work/.back.XXXXXX") && ln -s "$wt" "$stage/wt" && mkdir "$stage/records" "$stage/runs" || stage=
  if [ -n "$stage" ]; then
    common=
    [ "$r_git" != 1 ] || common=$(git -C "$wt" rev-parse --path-format=absolute --git-common-dir 2>/dev/null)
    if [ -n "$common" ]; then
      # A passing gate's record only for a clean tree (the commit is this
      # checkout's own); a gate's run record (gate-run.mjs) always.
      [ "$clean" != 1 ] || [ ! -d "$common/hollow-gate-records" ] || find "$common/hollow-gate-records" -maxdepth 1 -type f -name '*.json' -newer "$marker" -exec cp {} "$stage/records/" ';' 2>/dev/null
      # Only this folder's (node names it by its real path): every worktree
      # here shares the repository's.
      wt_real=$(cd "$wt" 2>/dev/null && pwd -P)
      for f in $(find "$common/hollow-gate-runs" -maxdepth 1 -type f -name '*.json' -newer "$marker" 2>/dev/null); do
        if grep -qF "\"checkout\": \"$wt\"" "$f" || grep -qF "\"checkout\": \"$wt_real\"" "$f"; then
          cp "$f" "$stage/runs/" 2>/dev/null
        fi
      done
    fi
    while IFS= read -r p; do
      p="${p%/}"
      [ -n "$p" ] || continue
      if ! valid_rel "$p" || { [ ! -e "$wt/$p" ] && [ ! -L "$wt/$p" ]; }; then
        say "$p isn't on $here, so it wasn't copied back"
      elif [ -z "$(find "$wt/$p" -cnewer "$marker" -print 2>/dev/null | head -n 1)" ]; then
        # Nothing in it changed since the command started: an ignored path
        # stays between runs (git clean -d keeps it), so this is an earlier
        # run's (the command failed before writing it, say). ctime, not
        # mtime: tar -x and cp -p set old mtimes on what they write.
        say "$p on $here is older than this run (the command didn't write it), so it wasn't copied back"
      else
        set -- "$@" "wt/$p"
      fi
    done <<EOF
$r_fetch
EOF
    # The checkout's changes the command made (it started clean), less the
    # --fetch paths, which come back (changed, a path a line), and the
    # ignored paths it wrote (ignored: a build's dist/ or .next/,
    # test-results/), less node_modules and the fetched ones: neither list
    # comes back, so the call says what stayed here (2026-11-03, review of
    # #1113). git status -z, so a path with a space or a quote reads as is.
    : > "$stage/changed"
    : > "$stage/ignored"
    if [ -n "$common" ]; then
      git --no-optional-locks -C "$wt" status --porcelain -z --untracked-files=all 2>/dev/null |
        R_FETCH="$r_fetch" /usr/bin/perl -0 -ne 'BEGIN { @f = grep { length } map { s{/+\z}{}r } split /\n/, $ENV{R_FETCH} } chomp; if ($skip) { $skip = 0; next } next if length($_) < 4; $skip = 1 if substr($_, 0, 2) =~ /[RC]/; my $p = substr($_, 3); next if grep { $p eq $_ || index($p, "$_/") == 0 } @f; print "$p\n" if $n++ < 50' > "$stage/changed"
      git --no-optional-locks -C "$wt" status --porcelain -z --ignored=matching --untracked-files=normal 2>/dev/null |
        R_FETCH="$r_fetch" R_WT="$wt" R_MARK="$marker" /usr/bin/perl -0 -ne 'BEGIN { @f = grep { length } map { s{/+\z}{}r } split /\n/, $ENV{R_FETCH} } chomp; next unless s/\A!! //; (my $p = $_) =~ s{/+\z}{}; next if $p =~ m{(?:\A|/)node_modules\z} || grep { $p eq $_ || index($p, "$_/") == 0 || index($_, "$p/") == 0 } @f; my $t = "$ENV{R_WT}/$p"; open(my $h, "-|", "/usr/bin/find", $t, "-cnewer", $ENV{R_MARK}, "-print", "-quit") or next; my $new = defined(scalar <$h>); close $h; print "$p\n" if $new && $n++ < 5' > "$stage/ignored"
    fi
    tar -C "$stage" -cf "$root/work/$wt_id.$token.tar" records runs changed ignored "$@" || say "couldn't pack the results on $here"
    rm -rf "${stage:?}"
  fi
  r_step "packing the results"
}
# Jobs (--send, overnight N4, 2026-11-02). On the host, by ssh from run_job:
# --job-receive <job> unpacks the tar on stdin into jobs/<job>, a new 0700
# folder (tar without -P refuses absolute paths, .. and writing through a
# symlink).
remote_job_receive() {
  [ "$#" -eq 2 ] || { say "--job-receive: wrong arguments"; exit 2; }
  job="$2"
  case "$job" in job-[0-9]*-[0-9]*) ;; *) say "--job-receive: bad name $job"; exit 2 ;; esac
  case "$job" in *[!A-Za-z0-9-]*) say "--job-receive: bad name $job"; exit 2 ;; esac
  root="$HOME/hollow-remote"
  j="$root/jobs/$job"
  # It holds the job's claim (jobs/<job>.run) while it unpacks, and a drop
  # (the call was stopped) leaves jobs/<job>.drop: seen before or after, the
  # upload is deleted here (tar accepts a stream cut between two files).
  mkdir -p "$root/jobs" && [ ! -d "$j.drop" ] && mkdir "$j.run" || { say "--job-receive: $job was dropped or is taken"; exit 3; }
  mkdir -m 700 "$j" || { rmdir "$j.run"; say "--job-receive: no new folder for $job"; exit 3; }
  if ! tar -xf - -C "$j" || [ -d "$j.drop" ]; then
    rm -rf "${j:?}" "${j:?}.run" "${j:?}.drop"
    say "--job-receive: what was sent couldn't be unpacked, or its call dropped it"
    exit 3
  fi
  rmdir "$j.run"
  exit 0
}
# On the host, by ssh from run_job when the run can't go ahead: --job-drop
# <job> leaves jobs/<job>.drop (a receive still unpacking deletes its upload
# when it sees it, a --job-run refuses to start; pruned after a day) and
# deletes the folder unless something holds its claim (jobs/<job>.run: the
# receive, or a run that started, which deletes its own folder).
remote_job_drop() {
  [ "$#" -eq 2 ] || { say "--job-drop: wrong arguments"; exit 2; }
  job="$2"
  case "$job" in job-[0-9]*-[0-9]*) ;; *) say "--job-drop: bad name $job"; exit 2 ;; esac
  case "$job" in *[!A-Za-z0-9-]*) say "--job-drop: bad name $job"; exit 2 ;; esac
  root="$HOME/hollow-remote"
  j="$root/jobs/$job"
  mkdir -p "$root/jobs" && mkdir "$j.drop" 2>/dev/null
  mkdir "$j.run" 2>/dev/null || exit 0
  rm -rf "${j:?}" "${j:?}.run"
  exit 0
}
# On the host, by ssh from run_job: --job-run <job> <token> <fetch paths> --
# <command>. One job at a time on the host (jobs/jobs.lock: a render or a
# transcription takes the machine's GPU and memory), then, holding it
# (--job-locked): gives up if the caller is gone, runs the command in the
# job's folder through this tool (one of the host's slots) with a short
# TMPDIR, at most 2 render workers and HyperFrames' pins as on this Mac,
# packs the --fetch paths, and deletes the folder. A folder a lost
# connection left behind goes with the next prune after a day.
remote_job_run() {
  mode="$1"
  shift
  [ "$#" -ge 5 ] && [ "$4" = -- ] || { say "$mode: wrong arguments"; exit 2; }
  job="$1" token="$2" r_fetch="$3"
  shift 4
  for v in "$job" "$token"; do
    case "$v" in '' | . | .. | *[!A-Za-z0-9._-]*) say "$mode: bad name $v"; exit 2 ;; esac
  done
  case "$job" in job-*) ;; *) say "$mode: bad name $job"; exit 2 ;; esac
  root="$HOME/hollow-remote"
  here=$(host_name)
  wt="$root/jobs/$job"
  wt_id="$job"
  tmp="${HOLLOW_REMOTE_TMP:-/tmp}/hj-$(hash_of "$job" 6)"
  if [ "$mode" = --job-run ]; then
    r_t=$(date +%s)
    r_setup="${HOLLOW_SLOT_SETUP:-}"
    r_login "${HOLLOW_SLOT_SETUP_SENT:-}"
    unset HOLLOW_SLOT_SETUP_SENT
    if [ ! -d "$wt" ] || [ -L "$wt" ] || [ -d "$wt.drop" ] || ! mkdir "$wt.run" 2>/dev/null; then
      say "$job isn't on $here to run (its call dropped it, or it was never sent)"
      exit 247
    fi
    remote_prune ""
    r_step prune
    HOLLOW_SLOT_SETUP="$r_setup" HOLLOW_SLOT_SETUP_AT="$r_t"
    export HOLLOW_SLOT_SETUP HOLLOW_SLOT_SETUP_AT
    in_slot "$root/jobs/jobs.lock" /usr/bin/true
    [ "$?" -ne 75 ] || say "another job is running on $here (one at a time); waiting for it"
    lock_wait "$root/jobs/jobs.lock" /bin/sh "$0" --job-locked "$job" "$token" "$r_fetch" -- "$@"
    rc=$?
    rm -rf "${wt:?}" "${wt:?}.run" "${wt:?}.drop" "${tmp:?}"
    exit "$rc"
  fi
  r_setup="${HOLLOW_SLOT_SETUP:-}"
  r_t="${HOLLOW_SLOT_SETUP_AT:-}"
  case "$r_t" in '' | *[!0-9]*) r_t=$(date +%s) ;; esac
  r_say="${HOLLOW_SLOT_SETUP_SAY:-}"
  unset HOLLOW_SLOT_SETUP HOLLOW_SLOT_SETUP_AT HOLLOW_SLOT_SETUP_SAY
  r_step "waiting for another job"
  if caller_gone; then
    say "the call was stopped before the command started on $here (while it waited for another job); not running it"
    rm -rf "${wt:?}" "${wt:?}.run" "${wt:?}.drop"
    exit 143
  fi
  if [ ! -d "$wt" ] || [ -L "$wt" ]; then
    say "$job has no folder on $here"
    exit 247
  fi
  mkdir -p -m 700 "$tmp" 2>/dev/null
  if [ ! -d "$tmp" ] || [ -L "$tmp" ] || [ ! -O "$tmp" ]; then
    say "refusing the temp folder $tmp on $here: a symlink, not a folder, or another user's"
    exit 247
  fi
  TMPDIR="$tmp"
  # At most 2 render workers whatever the login says (a --workers flag in the
  # command itself still wins), and the same caps as a checkout run.
  case "${PRODUCER_MAX_WORKERS:-}" in 1 | 2) ;; *) PRODUCER_MAX_WORKERS=2 ;; esac
  : "${CARGO_PROFILE_DEV_DEBUG:=0}" "${CARGO_PROFILE_TEST_DEBUG:=0}"
  : "${CARGO_BUILD_JOBS:=$((($(sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 2) + 1) / 2))}"
  : "${VITEST_MAX_WORKERS:=4}" "${VITEST_MAX_THREADS:=4}" "${VITEST_MAX_FORKS:=4}" "${VITEST_MIN_THREADS:=1}" "${VITEST_MIN_FORKS:=1}"
  : "${HYPERFRAMES_NO_UPDATE_CHECK:=1}" "${HYPERFRAMES_NO_AUTO_INSTALL:=1}" "${HYPERFRAMES_SKIP_SKILLS:=1}" "${HYPERFRAMES_NO_TELEMETRY:=1}" "${HF_HUB_DISABLE_TELEMETRY:=1}"
  HOLLOW_SLOT_REMOTE=off
  PATH="$root/bin:$PATH"
  export CARGO_PROFILE_DEV_DEBUG CARGO_PROFILE_TEST_DEBUG CARGO_BUILD_JOBS VITEST_MAX_WORKERS VITEST_MAX_THREADS VITEST_MAX_FORKS VITEST_MIN_THREADS VITEST_MIN_FORKS
  export TMPDIR PRODUCER_MAX_WORKERS HYPERFRAMES_NO_UPDATE_CHECK HYPERFRAMES_NO_AUTO_INSTALL HYPERFRAMES_SKIP_SKILLS HYPERFRAMES_NO_TELEMETRY HF_HUB_DISABLE_TELEMETRY HOLLOW_SLOT_REMOTE PATH
  unset HOLLOW_SLOT_HOLDER_PID HOLLOW_SLOT_CARGO_PID HOLLOW_SLOT_SAY HOLLOW_SLOT_SAY_ON HOLLOW_SLOT_CARGO
  cd "$wt" || exit 247
  r_step "job folder"
  r_note setup "before the command on $here"
  mkdir -p "$root/work" || exit 247
  marker="$root/work/$job.$token.start"
  host_exec "$@"
  # Nobody is left to collect a stopped call's results.
  caller_gone || host_pack 0
  r_note "wrapping up" "after the command on $here"
  rm -f "$marker"
  cd / && rm -rf "${wt:?}" "${wt:?}.run" "${wt:?}.drop" "${tmp:?}"
  exit "$rc"
}
# On the host, by ssh after a run: the results that run packed (--fetch
# paths under wt/, new gate records under records/, gate run records under
# runs/) on stdout, then gone; nothing when it packed none.
remote_host_collect() {
  [ "$#" -eq 3 ] || { say "--remote-collect: wrong arguments"; exit 2; }
  wt_id="$2" token="$3"
  for v in "$wt_id" "$token"; do
    case "$v" in '' | . | .. | *[!A-Za-z0-9._-]*) say "--remote-collect: bad name $v"; exit 2 ;; esac
  done
  r_pack="$HOME/hollow-remote/work/$wt_id.$token.tar"
  [ -f "$r_pack" ] || exit 0
  cat "$r_pack" || exit 1
  rm -f "$r_pack"
  exit 0
}
# How many Rust lanes may be taken now (R0, 2026-11-14), printed: 1 unless
# HOLLOW_SLOT_RUST_LANES=2. A runner's runs have it (remote_host_run), and
# --remote-lanes defaults it to 2 wherever it runs (it is the runner's
# preparation asking). This Mac's own calls don't set it, so they get 1, but
# that is cooperative: an exported HOLLOW_SLOT_RUST_LANES=2 here opens lane 1.
# With 2 enabled, 1 still while free memory is under
# HOLLOW_SLOT_MIN_FREE_PCT (30; 0 turns the watch off) or can't be read: a
# second compile only starts with room for it. Needs mem_free.
rust_lanes() {
  [ "${HOLLOW_SLOT_RUST_LANES:-1}" = 2 ] || { printf 1; return 0; }
  l_floor="${HOLLOW_SLOT_MIN_FREE_PCT:-30}"
  case "$l_floor" in "" | *[!0-9]*) l_floor=30 ;; esac
  [ "$l_floor" -gt 0 ] || { printf 2; return 0; }
  l_now=$(mem_free) || { printf 1; return 0; }
  if [ "$l_now" -ge "$l_floor" ]; then printf 2; else printf 1; fi
}
case "${1:-}" in
  --remote-lanes)
    # On the host, by the preparation: how many Rust lanes its runs may use
    # now (1 or 2), a runner's default being 2.
    eval "$mem_lib"
    HOLLOW_SLOT_RUST_LANES="${HOLLOW_SLOT_RUST_LANES:-2}"
    rust_lanes
    printf '\n'
    exit 0
    ;;
  --remote-run | --remote-locked) remote_host_run "$@" ;;
  --job-run | --job-locked) remote_job_run "$@" ;;
  --job-receive) remote_job_receive "$@" ;;
  --job-drop) remote_job_drop "$@" ;;
  --remote-collect) remote_host_collect "$@" ;;
  --remote-prune)
    # On the host, by the preparation when its preflight refused: make room
    # (never worktree folder $2's), then the preflight asks again.
    root="$HOME/hollow-remote"
    here=$(host_name)
    remote_prune "${2:-}"
    exit 0
    ;;
esac
# hollow-slot --postgres <command> (smart routing 6, 2026-11-03): run by
# /bin/sh -c in front of the command, here or on the runner. It starts a
# throwaway Postgres (pg_ctl on PATH, else the highest Homebrew
# postgresql@<n>; HOLLOW_SLOT_PG_DIRS overrides those folders) in a new
# folder under /tmp (short: its socket path has a 103-byte limit), on a free
# port on 127.0.0.1 only, as a direct child (review of #1121: pg_ctl start
# detaches the server into its own session, so a stopped or killed wrapper
# left it running; now a group stop reaches it), with a random password
# over TCP, fsync off, and empty databases hollow_test (test setups such as
# Portal's ask for "test" in the name) and hollow_test_shadow. The command
# gets DATABASE_URL, DIRECT_URL, the POSTGRES_*URL names and PG* pointed at
# it (SHADOW_DATABASE_URL at the shadow; PGPASSFILE, PGSERVICE and the like
# cleared, so no real database is reached), and vitest at one worker (one
# database). The server is stopped and the folder deleted when the command
# ends or is stopped; a folder an earlier wrapper left (its owner gone) is
# swept first. Exit 69 when there is no Postgres there, 70 when it can't
# start. HOLLOW_SLOT_PG_TMP stands in for /tmp in this tool's tests.
pg_wrap='here=$(hostname -s 2>/dev/null)
# Every PG* setting this side has goes first (PGSERVICE, PGSSLMODE, PGHOSTADDR
# would steer pg_isready and createdb away from the throwaway server; review
# r2 of #1121); the ones the command gets are set again below.
for v in $(env | sed -n "s/^\(PG[A-Z_]*\)=.*/\1/p"); do unset "$v"; done
pg=$(command -v pg_ctl 2>/dev/null)
if [ -z "$pg" ] && [ -n "${HOLLOW_SLOT_PG_DIRS+set}" ]; then
  for d in $HOLLOW_SLOT_PG_DIRS; do [ -n "$pg" ] || [ ! -x "$d/pg_ctl" ] || pg="$d/pg_ctl"; done
elif [ -z "$pg" ]; then
  pg=$(for d in /opt/homebrew/opt/postgresql@*/bin /usr/local/opt/postgresql@*/bin; do [ ! -x "$d/pg_ctl" ] || printf "%s\n" "$d/pg_ctl"; done | sed -n "s/.*@\([0-9][0-9]*\)\/bin\/pg_ctl$/\1 &/p" | sort -n | tail -n 1 | cut -d " " -f 2-)
fi
if [ -z "$pg" ]; then
  printf "hollow-slot: --postgres: no Postgres on %s (no pg_ctl on PATH or in Homebrew)\n" "$here" >&2
  exit 69
fi
bin="${pg%/*}"
# HOLLOW_SLOT_PG_TMP stands in for /tmp in the tests of this tool: a test with a
# folder of its own never sweeps, or loses, the folders of another test or
# gate running beside it (2026-11-04: a sweep from another call took a
# killed wrapper folder before that test could see it).
pg_tmp="${HOLLOW_SLOT_PG_TMP:-/tmp}"
case "$pg_tmp" in /*) ;; *) pg_tmp=/tmp ;; esac
# A folder an earlier wrapper left: its owner is gone (a pid this side may
# not signal, as from a sandbox, counts as alive: review r2 of #1121), or it
# never got an owner and is a day old. Its server, if one still runs with
# that very data folder, is stopped first.
for old in "$pg_tmp"/hs-pg.*; do
  [ -d "$old" ] && [ ! -L "$old" ] && [ -O "$old" ] || continue
  o=$(cat "$old/owner" 2>/dev/null)
  case "$o" in
    "" | *[!0-9]*) [ -n "$(find "$old" -maxdepth 0 -mmin +1440 2>/dev/null)" ] || continue ;;
    *) ! /usr/bin/perl -e "exit((kill(0, \$ARGV[0]) || \$!{EPERM}) ? 0 : 1)" "$o" || continue ;;
  esac
  m=$(head -n 1 "$old/data/postmaster.pid" 2>/dev/null)
  case "$m" in "" | *[!0-9]*) ;; *) case "$(ps -o args= -p "$m" 2>/dev/null)" in *"$old/data"*) kill -QUIT "$m" 2>/dev/null ;; esac ;; esac
  rm -rf "$old"
done
d=$(mktemp -d "$pg_tmp/hs-pg.XXXXXX") || exit 70
printf "%s\n" "$$" > "$d/owner"
server=
stop() {
  if [ -n "$server" ]; then
    kill -INT "$server" 2>/dev/null
    n=0
    while kill -0 "$server" 2>/dev/null && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
    kill -KILL "$server" 2>/dev/null
    wait "$server" 2>/dev/null
  fi
  rm -rf "$d"
}
trap "stop; exit 130" INT
trap "stop; exit 143" TERM
trap "stop; exit 129" HUP
pw=$(od -An -tx1 -N16 /dev/urandom | tr -d " \n")
printf "%s\n" "$pw" > "$d/pw"
if ! LC_ALL=en_US.UTF-8 "$bin/initdb" -D "$d/data" --auth-local=trust --auth-host=scram-sha-256 --pwfile="$d/pw" -U postgres -E UTF8 > "$d/initdb.log" 2>&1; then
  printf "hollow-slot: --postgres: initdb failed on %s: %s\n" "$here" "$(tail -n 1 "$d/initdb.log")" >&2
  stop
  exit 70
fi
port=
bound=0
n=0
while [ "$n" -lt 20 ]; do
  try=$((55000 + ($$ * 17 + n * 211) % 3000))
  n=$((n + 1))
  /usr/bin/perl -MIO::Socket::INET -e "exit(IO::Socket::INET->new(Listen => 1, LocalAddr => q(127.0.0.1), LocalPort => \$ARGV[0]) ? 0 : 1)" "$try" 2> /dev/null || continue
  bound=1
  LC_ALL=en_US.UTF-8 "$bin/postgres" -D "$d/data" -p "$try" -k "$d" -c listen_addresses=127.0.0.1 -c fsync=off -c synchronous_commit=off -c full_page_writes=off < /dev/null > "$d/log" 2>&1 &
  server=$!
  # Ready within 60 s by the clock; only a server that died moves on to
  # another port (a live one that never answers is a failure, not a retry).
  since=$(date +%s)
  while kill -0 "$server" 2>/dev/null && [ $(($(date +%s) - since)) -lt 60 ]; do
    if [ -S "$d/.s.PGSQL.$try" ] && PGPASSWORD="$pw" "$bin/pg_isready" -q -t 1 -h 127.0.0.1 -p "$try" -U postgres 2> /dev/null && kill -0 "$server" 2>/dev/null; then
      port="$try"
      break
    fi
    sleep 0.1
  done
  [ -z "$port" ] || break
  if kill -0 "$server" 2>/dev/null; then
    kill -KILL "$server" 2>/dev/null
    wait "$server" 2>/dev/null
    server=
    break
  fi
  wait "$server" 2>/dev/null
  server=
done
if [ -z "$port" ]; then
  if [ "$bound" = 0 ]; then
    printf "hollow-slot: --postgres: no port on 127.0.0.1 could be bound on %s (a sandbox without network?)\n" "$here" >&2
  else
    printf "hollow-slot: --postgres: Postgres would not start on %s: %s\n" "$here" "$(tail -n 2 "$d/log" 2> /dev/null | tr "\n" " ")" >&2
  fi
  stop
  exit 70
fi
for db in hollow_test hollow_test_shadow; do
  if ! PGPASSWORD="$pw" "$bin/createdb" -h 127.0.0.1 -p "$port" -U postgres "$db" > /dev/null 2> "$d/createdb.log"; then
    printf "hollow-slot: --postgres: createdb failed on %s: %s\n" "$here" "$(tail -n 1 "$d/createdb.log")" >&2
    stop
    exit 70
  fi
done
DATABASE_URL="postgresql://postgres:$pw@127.0.0.1:$port/hollow_test"
DIRECT_URL="$DATABASE_URL"
POSTGRES_URL="$DATABASE_URL"
POSTGRES_PRISMA_URL="$DATABASE_URL"
POSTGRES_URL_NON_POOLING="$DATABASE_URL"
SHADOW_DATABASE_URL="postgresql://postgres:$pw@127.0.0.1:$port/hollow_test_shadow"
PGHOST=127.0.0.1
PGPORT="$port"
PGUSER=postgres
PGPASSWORD="$pw"
PGDATABASE=hollow_test
# One database, so vitest runs one test file at a time: the runner caps it
# at 4 workers, which overrides a config'\''s fileParallelism: false (Portal'\''s
# integration suite deadlocked that way, 2026-11-03). A command that sets
# its own (env VITEST_MAX_WORKERS=2 ...) still wins.
VITEST_MAX_WORKERS=1
VITEST_MAX_FORKS=1
VITEST_MAX_THREADS=1
VITEST_MIN_FORKS=1
VITEST_MIN_THREADS=1
export DATABASE_URL DIRECT_URL POSTGRES_URL POSTGRES_PRISMA_URL POSTGRES_URL_NON_POOLING SHADOW_DATABASE_URL PGHOST PGPORT PGUSER PGPASSWORD PGDATABASE VITEST_MAX_WORKERS VITEST_MAX_FORKS VITEST_MAX_THREADS VITEST_MIN_FORKS VITEST_MIN_THREADS
printf "hollow-slot: --postgres: a throwaway Postgres %s on %s at 127.0.0.1:%s, database hollow_test (DATABASE_URL set); it goes when the command ends\n" "$("$bin/postgres" --version 2> /dev/null | awk "{ print \$3 }")" "$here" "$port" >&2
"$@"
rc=$?
stop
exit "$rc"'
# Whether another process holds cargo.lock now (tried for an instant).
cargo_busy() {
  /usr/bin/perl -e "$lock_pl" "$dir/cargo.lock" /usr/bin/true
  [ "$?" -eq 75 ]
}
# Sets lane_files (the lock files a Rust build may take now, lane 0 first:
# cargo.lock, then cargo-1.lock while a second lane is on) and rust_wait,
# and with HOLLOW_SLOT_LANE_JOBS=1 (a runner's run) cargo's jobs for it: 5
# whenever two lanes are enabled (HOLLOW_SLOT_RUST_LANES=2), lane 0 under low
# memory included, so a lane never adds up with the other to more than the
# runner's 10 cores (low memory lowers the work, never raises it); 8 only
# where lanes are not enabled at all.
lane_set() {
  lane_files=cargo.lock
  lane_jobs=8
  rust_wait="hollow-slot: waiting for the other Rust build to finish (one at a time on this Mac)"
  if [ "${HOLLOW_SLOT_RUST_LANES:-1}" = 2 ]; then
    lane_jobs=5
    if [ "$(rust_lanes)" = 2 ]; then
      lane_files="cargo.lock cargo-1.lock"
      rust_wait="hollow-slot: both Rust lanes are in use; waiting for one to free up (two at a time on this runner)"
    else
      rust_wait="hollow-slot: waiting for the other Rust build to finish (one at a time while this runner's memory is under its floor)"
    fi
  fi
  if [ "${HOLLOW_SLOT_LANE_JOBS:-}" = 1 ]; then
    CARGO_BUILD_JOBS="$lane_jobs"
    export CARGO_BUILD_JOBS
  fi
}
# Whether every lane in lane_files is taken now (each tried for an instant).
lanes_busy() {
  for l_f in $lane_files; do
    /usr/bin/perl -e "$lock_pl" "$dir/$l_f" /usr/bin/true
    [ "$?" -eq 75 ] || return 1
  done
  return 0
}
# Whether pid $1 is this process or one of its ancestors (64 deep at most).
is_ancestor() {
  case "$1" in '' | *[!0-9]*) return 1 ;; esac
  p="$$"
  depth=0
  while [ "$depth" -lt 64 ] && [ -n "$p" ] && [ "$p" -gt 1 ]; do
    [ "$p" = "$1" ] && return 0
    # Linux (2026-11-03): /proc/<pid>/stat is "pid (name) state ppid ...",
    # and procps may be missing there.
    if [ "$hs_os" = Linux ] && [ -r "/proc/$p/stat" ]; then
      p=$(sed -n 's/.*) [^ ]* \([0-9][0-9]*\) .*/\1/p' "/proc/$p/stat" 2>/dev/null)
    else
      p=$(ps -o ppid= -p "$p" 2>/dev/null | tr -d ' ')
    fi
    depth=$((depth + 1))
  done
  return 1
}
dev=0
pg=
remote=
route=
rt_source=
fetch=
send=
# Where to run, named once: --here beside --remote or --on is a mistake.
r_route() {
  if [ -n "$route" ] && [ "$route" != "$1" ]; then
    say "$2 doesn't go with $rt_source"
    exit 2
  fi
  route="$1"
  rt_source="$2"
}
while [ "$#" -gt 0 ]; do
  case "$1" in
    -h | --help)
      usage_text
      exit 0
      ;;
    --dev-server)
      dev=1
      shift
      ;;
    --auto)
      r_route auto --auto
      shift
      ;;
    --here)
      r_route here --here
      shift
      ;;
    --remote)
      r_route remote --remote
      remote="${remote:-configured}"
      shift
      ;;
    --on)
      [ "$#" -ge 2 ] || usage
      r_route remote --on
      remote="host:$2"
      shift 2
      ;;
    --fetch)
      [ "$#" -ge 2 ] || usage
      r_p=$(norm_rel "$2")
      valid_rel "$r_p" || { say "--fetch takes a path inside the checkout, not $2"; exit 2; }
      fetch="$fetch$r_p
"
      shift 2
      ;;
    --postgres)
      pg=1
      shift
      ;;
    --send)
      [ "$#" -ge 2 ] || usage
      r_p=$(norm_rel "$2")
      valid_rel "$r_p" || { say "--send takes a path inside this folder, not $2"; exit 2; }
      send="$send$r_p
"
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      # Before, an unknown option such as --help took a slot and ran as the
      # command ("exec: --: invalid option").
      say "unknown option $1 (hollow-slot --help lists them)"
      exit 2
      ;;
    *) break ;;
  esac
done
fetch=$(fetch_tidy "$fetch")
if [ "$#" -eq 0 ]; then
  usage
fi
if [ "$dev" -eq 1 ] && [ -n "$pg" ]; then
  say "--postgres gives a command a throwaway database; a dev server doesn't go with it"
  exit 2
fi
if [ "$dev" -eq 1 ] && [ -n "$remote$fetch$send" ]; then
  say "a dev server runs on this Mac only: --dev-server doesn't go with --remote, --on, --fetch or --send"
  exit 2
fi
if [ "$route" = here ] && [ -n "$fetch$send" ]; then
  say "--fetch and --send copy files to and from the remote runner, so they don't go with --here"
  exit 2
fi
# With a runner set up, routing decides where a --fetch or --send call runs
# (a --send one goes there unless it needs this Mac); without one, they need
# --remote or --on, as before.
rt_host=$(configured_remote)
case "$rt_host" in off | local | none) rt_host= ;; esac
if [ -n "$fetch$send" ] && [ -z "$remote" ] && [ -z "$rt_host" ]; then
  say "--fetch and --send go with --remote or --on"
  exit 2
fi
mkdir -p -m 700 "$dir" || exit 1
# /tmp is shared: use the folder only when it is a real folder of this
# user's, not a symlink or one another user made first (mkdir -p leaves an
# existing one as it is), then make it 0700 whatever mode it had.
if [ ! -d "$dir" ] || [ -L "$dir" ] || [ ! -O "$dir" ]; then
  printf '%s\n' "hollow-slot: refusing the slot folder $dir: it is a symlink, not a folder, or owned by another user. Remove it, or set HOLLOW_BUILD_SLOT_DIR to a folder of your own." >&2
  exit 1
fi
chmod 700 "$dir" || exit 1
eval "$mem_lib"
# Routing (2026-11-03; the header has the rules). Not for a dev server, nor
# a call from inside a slot's own command (it runs in its caller's slot,
# wherever that is), nor on a host (its runs set HOLLOW_SLOT_REMOTE=off).
# Without a runner set up and no --remote or --on, nothing changes: it runs
# here and says nothing new.
rt_key=
rt_repo=
rt_label=
rt_where=
rt_class=
# How the lines name the runner: Hollow's own name for it (Settings, Remote
# runner), unless HOLLOW_SLOT_REMOTE_NAME says otherwise.
rt_name="${HOLLOW_SLOT_REMOTE_NAME:-the runner}"
# A killed call leaves its timer (below); a day later it goes.
find "$dir" -maxdepth 1 -name '.timer.*' -mmin +1440 -exec rm -f {} + 2>/dev/null
if [ "$dev" -eq 0 ] && ! is_ancestor "${HOLLOW_SLOT_HOLDER_PID:-}" && [ -n "$rt_host$remote" ]; then
  if [ -z "$route" ]; then
    case "${HOLLOW_SLOT_ROUTE:-}" in
      remote | runner | mac-mini) route=prefer rt_source="Builds and tests" rt_set="Remote runner" ;;
      here | this-mac) route=here rt_source="Builds and tests" rt_set="This Mac" ;;
      *) route=auto rt_source=auto ;;
    esac
  fi
  # A job asks for the runner: it can't be run anywhere else as asked.
  if [ -n "$send" ] && [ "$route" = auto ]; then
    route=prefer
    rt_source=--send
  fi
  rt_top=$(git rev-parse --show-toplevel 2>/dev/null) || rt_top=
  rt_repo=-
  if [ -n "$rt_top" ]; then
    rt_common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) &&
      rt_repo="$(slug_of "${rt_common%/.git}")-$(hash_of "$rt_common")"
  fi
  rt_cls=$(HOLLOW_SLOT_JOB="${send:+1}" HOLLOW_SLOT_PG="$pg" HOLLOW_SLOT_FETCHED="$fetch" /usr/bin/perl -e "$route_pl" "$rt_top" "$(pwd -P)" "$@" 2>/dev/null)
  IFS="$(printf '\037')" read -r rt_need rt_class rt_label rt_key rt_named rt_needs <<EOF
$rt_cls
EOF
  if [ -z "$remote" ] && [ -n "$rt_host" ]; then
    if [ "${CODEX_SANDBOX_NETWORK_DISABLED:-}" = 1 ]; then
      rt_where=here
      rt_why="this turn's sandbox has no network (a Codex turn in a mode other than bypass), so it can't reach $rt_name"
    elif [ "$route" = here ]; then
      rt_where=here
      if [ "$rt_source" = --here ]; then rt_why="asked for with --here"; else rt_why="Builds and tests is set to $rt_set"; fi
    elif [ "$route" = prefer ]; then
      rt_where=here
      if [ -n "$rt_need" ]; then
        rt_why="$rt_need"
      elif route_down; then
        :
      elif [ "$(route_median "$rt_repo" "$rt_key" | cut -d ' ' -f 3)" = 1 ]; then
        rt_why="$rt_label changed this checkout when it ran, so it runs where its changes land"
      elif route_lacks "$rt_host"; then
        rt_where=here
        lack_why
      else
        rt_where=there
        if [ "$rt_source" = --send ]; then rt_why="a job with the files it sends"; else rt_why="Builds and tests is set to $rt_set"; fi
      fi
    else
      route_auto
    fi
    if [ "$rt_where" = there ]; then
      say "on $rt_name ($rt_why)"
      route_log "$rt_host" "$rt_why"
      remote=configured
    else
      say "on this Mac ($rt_why)"
      route_log here "$rt_why"
    fi
  elif [ -n "$remote" ]; then
    rt_where=there
    rt_target="${remote#host:}"
    [ "$rt_target" != configured ] || rt_target="$rt_host"
    if [ -n "$rt_target" ]; then
      route_log "$rt_target" "asked for with $rt_source"
    else
      route_log here "$rt_source asked for the runner, but none is set up"
    fi
  fi
fi
# --postgres (smart routing 6): the command runs inside pg_wrap, wherever it
# runs, which starts its throwaway database first.
[ -z "$pg" ] || set -- /bin/sh -c "$pg_wrap" hollow-slot-postgres "$@"
if [ -n "$remote" ]; then
  case "$remote" in
    host:*) host="${remote#host:}" ;;
    *) host=$(configured_remote) ;;
  esac
  case "$host" in
    '') say "no remote runner is set up (HOLLOW_SLOT_REMOTE, or an SSH host in ${XDG_CONFIG_HOME:-$HOME/.config}/hollow/slot-remote); running on this Mac" ;;
    off | local | none) say "the remote runner is turned off ($host); running on this Mac" ;;
    *)
      # Codex's workspace-write sandbox (any Codex mode but bypass) has no
      # network, and ssh would only time out: say so at once.
      if [ "${CODEX_SANDBOX_NETWORK_DISABLED:-}" = 1 ]; then
        say "this turn's sandbox has no network (a Codex turn in a mode other than bypass), so it can't reach $host; running on this Mac instead"
        [ -z "$rt_where" ] || route_log here "no network in this turn's sandbox"
      else
        scratch=
        fallback=
        fallback_why=
        r_unreached=
        r_wrote=0
        rc=0
        rt_t0=$(date +%s)
        if [ -n "$send" ]; then
          run_job "$host" "$@"
        else
          run_remote "$host" "$@"
        fi
        trap - INT TERM HUP
        [ -z "$scratch" ] || rm -rf "$scratch"
        if [ -z "$fallback" ]; then
          # It answered: a runner-down note of its own is out of date.
          if [ -r "$dir/runner-down" ] && [ "$(cut -d ' ' -f 2 "$dir/runner-down" 2>/dev/null)" = "$host" ]; then
            rm -f "$dir/runner-down"
          fi
          [ "$rc" -eq 255 ] || route_record "$host" "$(($(date +%s) - rt_t0))" "$rc" "$r_wrote"
          exit "$rc"
        fi
        say "$fallback; running on this Mac instead"
        [ -z "$fallback_why" ] || say "$fallback_why"
        [ -z "$rt_where" ] || route_log here "$fallback"
        # Automatic runs don't wait for it again for a while (route_auto).
        [ -z "$r_unreached" ] || printf '%s %s %s\n' "$(date +%s)" "$host" "$r_unreached" > "$dir/runner-down" 2>/dev/null
      fi
      ;;
  esac
fi
unset HOLLOW_SLOT_SAY_ON
if [ "$dev" -eq 1 ]; then
  # One dev server at a time on this Mac, on the turn's own PORT when it has
  # one (every chat turn that can run it does), else HOLLOW_DEV_SERVER_PORT,
  # else 1499.
  port="${PORT:-${HOLLOW_DEV_SERVER_PORT:-1499}}"
  PORT="$port"
  PW_PORT="$port"
  HOLLOW_SLOT_PORT="$port"
  HOLLOW_SLOT_SAY="hollow-slot: got the dev-server lock; starting it on PORT=$port"
  HOLLOW_SLOT_MEM=1
  export PORT PW_PORT HOLLOW_SLOT_PORT HOLLOW_SLOT_SAY HOLLOW_SLOT_MEM
  lock="$dir/dev-server.lock"
  # A status read takes the lock for an instant, so try three times.
  for try in 1 2 3; do
    in_slot "$lock" /bin/sh -c "$record" hollow-slot "$lock" "$@"
    status=$?
    if [ "$status" -ne 75 ]; then
      exit "$status"
    fi
    if [ "$try" -lt 3 ]; then
      sleep 1
    fi
  done
  holder=$(tr '\n\r' '  ' 2>/dev/null < "$lock.holder")
  printf '%s\n' "hollow-slot: another dev server holds this Mac's dev-server lock (${holder:-holder unknown}); one runs at a time. Stop it, or use the one already running." >&2
  exit 75
fi
# Is it a Rust build? set -f, so a word like * isn't matched against files.
# Arguments keep their boundaries: after npm's run or run-script, options are
# skipped, and the value of one that takes one is a whole argument (spaces
# and all). Only the string after a shell option cluster holding c is split
# into words; in it, a cargo script anywhere after run counts (quoting can't
# be followed).
is_tool() { case "${1##*/}" in cargo | rustc | tauri | cargo-*) return 0 ;; esac; return 1; }
is_script() { case "$1" in gate | gate:rust | perf | tauri | tauri:dev | tauri:bundle | tauri:build | tauri:rebuild) return 0 ;; esac; return 1; }
rust=0
case "${HOLLOW_SLOT_CARGO:-}" in
  1) rust=1 ;;
  0) ;;
  *)
    set -f
    script=0
    prev=
    for arg in "$@"; do
      cmd_string=0
      case "$prev" in
        --* | -*[!a-zA-Z]*) ;;
        -*c*) cmd_string=1 ;;
      esac
      if [ "$cmd_string" = 1 ]; then
        after_run=0
        for word in $(printf '%s\n' "$arg" | tr ';&|()<>`"\047{}' '            '); do
          is_tool "$word" && rust=1
          [ "$after_run" = 1 ] && is_script "$word" && rust=1
          case "$word" in run | run-script) after_run=1 ;; esac
        done
      elif [ "$script" = value ]; then
        script=1
      else
        is_tool "$arg" && rust=1
        if [ "$script" = 1 ]; then
          case "$arg" in
            --prefix | -C | --workspace | -w | --loglevel | --userconfig | --cache | --registry) script=value ;;
            -*) ;;
            *) is_script "$arg" && rust=1; script=0 ;;
          esac
        fi
        case "$arg" in run | run-script) script=1 ;; esac
      fi
      prev="$arg"
    done
    set +f
    ;;
esac
if is_ancestor "${HOLLOW_SLOT_HOLDER_PID:-}"; then
  # Called from inside a command that holds a slot: run in that slot.
  if [ "$rust" -eq 0 ]; then
    echo "hollow-slot: running in the build slot of the command that called it" >&2
    exec "$@"
  fi
  if is_ancestor "${HOLLOW_SLOT_CARGO_PID:-}"; then
    echo "hollow-slot: already inside a Rust build that holds the cargo lock; run cargo directly instead of through hollow-slot" >&2
    exit 2
  fi
  # A Rust build in its caller's slot: cargo.lock alone, holding nothing
  # while it waits (the slot is its caller's).
  said=0
  while :; do
    lane_set
    for lane in $lane_files; do
      /usr/bin/perl -e "$lock_pl" "$dir/$lane" /bin/sh -c "$record" hollow-slot "$dir/$lane" "$@"
      status=$?
      if [ "$status" -ne 75 ]; then
        exit "$status"
      fi
    done
    if [ "$said" -eq 0 ]; then
      echo "$rust_wait" >&2
      said=1
    fi
    sleep "${HOLLOW_SLOT_POLL_SECS:-5}"
  done
fi
# The capability index names only `hollow-slot <cmd>` (32 bytes, the
# budgets' limit), so a build call says the dev-server form as it starts:
# a dev server run as a build would hold one of the two slots while it lives.
# Where a remote runner is set up, the routing line above already said where
# it runs and why (before 2026-11-03 this line named --remote instead); a
# wait for memory here then names the runner as the way out.
HOLLOW_SLOT_SAY="hollow-slot: got a build slot; running it now (a dev server runs as hollow-slot --dev-server <command>, not in a build slot)"
HOLLOW_SLOT_MEM=1
if [ "${CODEX_SANDBOX_NETWORK_DISABLED:-}" != 1 ] && [ -n "$rt_host" ] && [ -z "$remote" ]; then
  HOLLOW_SLOT_MEM=remote
fi
export HOLLOW_SLOT_SAY HOLLOW_SLOT_MEM
# A routed run here is timed from when it holds its slot (record writes the
# start to HOLLOW_SLOT_TIMER) for route-history; one of no known kind also
# notes whether it changed the checkout, which keeps it here from then on.
rt_timer=
rt_before=
# The tree of checkout $1's files as they are now (tracked and untracked,
# not ignored), hashed with a copy of its index in a private object folder
# so the repository gets nothing (as run_remote's snapshot): unlike git
# status lines, it changes when a file that is already modified is written
# again (review r2). Empty when git can't say.
tree_of() {
  t_dir=$(mktemp -d "${TMPDIR:-/tmp}/hollow-slot-tree.XXXXXX" 2>/dev/null) || return 0
  (
    cd "$1" 2>/dev/null || exit 1
    t_common=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 1
    cp "$(git rev-parse --path-format=absolute --git-path index 2>/dev/null)" "$t_dir/index" 2>/dev/null
    mkdir "$t_dir/objects" || exit 1
    GIT_INDEX_FILE="$t_dir/index" GIT_OBJECT_DIRECTORY="$t_dir/objects" GIT_ALTERNATE_OBJECT_DIRECTORIES="$t_common/objects"
    export GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
    git --no-optional-locks add -A . > /dev/null 2>&1 || exit 1
    git write-tree 2>/dev/null
  )
  rm -rf "$t_dir"
}
if [ -n "$rt_key" ]; then
  rt_timer="$dir/.timer.$$"
  rm -f "$rt_timer"
  HOLLOW_SLOT_TIMER="$rt_timer"
  export HOLLOW_SLOT_TIMER
  # Not for a heavy kind: kind alone sends it, and a long gate here would
  # count an edit made beside it as its own; a heavy writer is caught when
  # the runner reports what it changed.
  if [ -n "$rt_top" ] && [ "$rt_class" != heavy ]; then
    rt_before=$(tree_of "$rt_top")
  fi
fi
# Record a routed run here that finished with status $1.
route_done() {
  [ -n "$rt_timer" ] || return 0
  r_start=$(cat "$rt_timer" 2>/dev/null)
  rm -f "$rt_timer"
  case "$r_start" in '' | *[!0-9]*) return 0 ;; esac
  r_changed=0
  r_after=$(tree_of "$rt_top")
  if [ -n "$rt_before" ] && [ -n "$r_after" ] && [ "$rt_before" != "$r_after" ]; then
    r_changed=1
  fi
  route_record here "$(($(date +%s) - r_start))" "$1" "$r_changed"
}
if [ "$rust" -eq 1 ]; then
  # Said once the cargo lock is taken too, not at the slot.
  HOLLOW_SLOT_SAY_ON="$dir/cargo.lock"
  export HOLLOW_SLOT_SAY_ON
fi
said_rust=0
said_slots=0
while :; do
  # The lanes open now (memory checked here, at every try), and jobs for them.
  [ "$rust" -eq 0 ] || lane_set
  # A Rust build skips the slots while every lane is plainly taken, so a
  # waiting one doesn't keep touching a slot other work could use.
  if [ "$rust" -eq 0 ] || ! lanes_busy; then
    for n in 0 1; do
      lock="$dir/slot-$n.lock"
      if [ "$rust" -eq 1 ]; then
        # Both locks in one try: the slot, then (no wait) a lane's cargo
        # lock, lane 0 first. Said once the lane is held, not at the slot.
        for lane in $lane_files; do
          HOLLOW_SLOT_SAY_ON="$dir/$lane"
          in_slot "$lock" /bin/sh -c "$record" hollow-slot "$lock" /usr/bin/perl -e "$lock_pl" "$dir/$lane" /bin/sh -c "$record" hollow-slot "$dir/$lane" "$@"
          status=$?
          # 75 (EX_TEMPFAIL) means a lock is taken; anything else is the command's own status.
          if [ "$status" -ne 75 ]; then
            route_done "$status"
            exit "$status"
          fi
        done
      else
        in_slot "$lock" /bin/sh -c "$record" hollow-slot "$lock" "$@"
        status=$?
        if [ "$status" -ne 75 ]; then
          route_done "$status"
          exit "$status"
        fi
      fi
    done
  fi
  if [ "$rust" -eq 1 ] && lanes_busy; then
    if [ "$said_rust" -eq 0 ]; then
      echo "$rust_wait" >&2
      said_rust=1
    fi
  elif [ "$said_slots" -eq 0 ]; then
    echo "hollow-slot: both build slots are in use; waiting for one to free up" >&2
    said_slots=1
  fi
  sleep "${HOLLOW_SLOT_POLL_SECS:-5}"
done
"#;

/// Where the slot locks live: [`default_slot_dir`], which is also the
/// script's default.
pub(crate) fn slot_dir() -> PathBuf {
    #[cfg(test)]
    {
        crate::test_temp::process_dir("hollow-build-slots-tests-")
    }
    #[cfg(not(test))]
    {
        default_slot_dir()
    }
}

/// `/tmp/hollow-build-slots-<uid>`, which `hollow-slot` makes 0700. A temp
/// folder because every agent's sandbox can write one (OF07 review): Codex's
/// `workspace-write` writes only the working folder and the temp folders,
/// and Grok's `read-only` and `workspace` profiles `~/.grok` and `/tmp`, so
/// the B7 folder, `~/Library/Application Support/Hollow/BuildSlots`, failed
/// `hollow-slot` in every sandboxed Codex or Grok turn. Not `$TMPDIR`: that
/// differs between processes, and the slots must be one set per user.
fn default_slot_dir() -> PathBuf {
    PathBuf::from(format!("/tmp/hollow-build-slots-{}", current_uid()))
}

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

/// Whether a turn of `agent` in `perm` can take a slot: it runs commands,
/// and its sandbox can write the slot folder in `/tmp` (OF07 review). Plan
/// runs no commands in any agent, and Antigravity's `--sandbox` modes
/// (accept-edits; it has no auto) deny every command, so only its bypass
/// can. Codex's `workspace-write` and Grok's `read-only` (its `default`) and
/// `workspace` profiles all write `/tmp`.
pub(crate) fn can_take_slot(agent: crate::agent::AgentKind, perm: &str) -> bool {
    perm != "plan" && (agent != crate::agent::AgentKind::Antigravity || perm == "bypassPermissions")
}

/// Write `hollow-slot` (when missing or changed) and return its folder.
pub(crate) fn ensure_tool() -> Result<PathBuf, String> {
    let bin = crate::agent_settings::app_data_dir()?.join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| format!("create {}: {e}", bin.display()))?;
    let path = bin.join("hollow-slot");
    if std::fs::read_to_string(&path).ok().as_deref() != Some(SLOT_TOOL) {
        crate::vault::atomic_write(&path, SLOT_TOOL.as_bytes())?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
    }
    // A machine-note failure must not take away the slot tool or its caps.
    // It removes the old note, so turns get no HOLLOW_THIS_MAC until a
    // generation succeeds ([`turn_env`]).
    if let Err(error) = crate::this_mac::ensure_file(&bin, &crate::this_mac::Facts::detect()) {
        eprintln!("[machine note] {error}; no note this time, hollow-slot remains available");
    }
    Ok(bin)
}

/// A stable short name for a worktree: its repo folder slug and a hash of
/// its path.
fn worktree_id(build: &crate::build_workspaces::BuildInfo) -> String {
    worktree_id_of(&build.repo, &build.worktree)
}
/// The per-worktree target folder's name, from the binding's repo and worktree.
fn worktree_id_of(repo_path: &str, worktree: &str) -> String {
    let build = crate::build_workspaces::BuildInfo {
        repo: repo_path.to_string(),
        worktree: worktree.to_string(),
        branch: String::new(),
        base: String::new(),
        exists: false,
        runs_on: None,
        port: None,
    };
    let repo = Path::new(&build.repo)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let hash = format!("{:x}", Sha256::digest(build.worktree.as_bytes()));
    format!(
        "{}-{}",
        crate::build_workspaces::slug(&repo, 40),
        &hash[..12]
    )
}

/// Delete a removed worktree's build target (night ST7, 2026-10-14): each
/// build workspace's `hollow-build-targets/<repo>-<hash>` stayed behind after
/// its worktree was removed, with nothing left to build from it.
pub(crate) fn forget_target(repo: &str, worktree: &str) {
    // Tests never touch this Mac's real caches.
    if cfg!(test) {
        return;
    }
    let root = crate::instance::cache_sibling("build-targets");
    crate::self_install::remove_cache_dir(&root.join(worktree_id_of(repo, worktree)), &root);
}

/// A port for the worktree's dev server and browser tests, stable per
/// worktree, in 1500-1899 (clear of Vite's 1420): the one assigned when it
/// was bound (F6 part B, unique in its vault), else [`hashed_port`].
pub(crate) fn port_for(build: &crate::build_workspaces::BuildInfo) -> u16 {
    build.port.unwrap_or_else(|| hashed_port(&build.worktree))
}

/// The port a worktree path hashes to, in 1500-1899: every binding's before
/// F6 part B, and where a new binding's search for a free one starts.
pub(crate) fn hashed_port(worktree: &str) -> u16 {
    let hash = Sha256::digest(worktree.as_bytes());
    1500 + (u16::from_be_bytes([hash[0], hash[1]]) % 400)
}

/// The toolchain caps every slot runs with (`~/.claude/CLAUDE.md`): rustc
/// jobs, vitest's workers, and the minimums vitest needs under that cap on
/// an 18-core Mac (it aborts on a minimum above the maximum).
pub(crate) const CAPS: [(&str, &str); 6] = [
    ("CARGO_BUILD_JOBS", "8"),
    ("VITEST_MAX_WORKERS", "4"),
    ("VITEST_MAX_THREADS", "4"),
    ("VITEST_MAX_FORKS", "4"),
    ("VITEST_MIN_THREADS", "1"),
    ("VITEST_MIN_FORKS", "1"),
];

/// The build slot lock files, `slot-0.lock` and `slot-1.lock`.
pub(crate) const SLOTS: usize = 2;

/// The machine-wide dev-server lock, `dev-server.lock` (`hollow-slot
/// --dev-server`, OF07).
pub(crate) const DEV_SERVER: &str = "dev-server";

/// The machine-wide Rust build lock, `cargo.lock` (2026-10-20): a command
/// `hollow-slot` takes for a Rust build holds it beside its slot, so one
/// compiles at a time.
pub(crate) const CARGO: &str = "cargo";

/// A runner's second Rust lane, `cargo-1.lock` (R0, 2026-11-14): its runs
/// may hold it beside a slot while memory allows, so two compile at once
/// there. This Mac's own calls don't (cooperative); it reads free here.
pub(crate) const CARGO_LANE_1: &str = "cargo-1";

/// The dev-server port when the turn has no `PORT` of its own (since F6
/// part A every chat turn that can take a slot has one; workflow steps and
/// commands run outside Hollow don't) and `HOLLOW_DEV_SERVER_PORT` isn't
/// set: just under the build workspaces' 1500-1899, clear of Vite's 1420.
pub(crate) const DEV_SERVER_PORT: u16 = 1499;

/// Every other chat turn's own `PORT` / `PW_PORT` (F6 part A, 2026-10-20):
/// 1900-2299, just above the build workspaces' 1500-1899, so a turn's port
/// is never a worktree's, [`DEV_SERVER_PORT`] or Vite's 1420. Before this a
/// workspace folder's conversations, several of which run at once since
/// 2026-10-07, all fell back to 1499 for a dev server and had no `PW_PORT`,
/// so two Playwright runs in the two slots fought over one port.
pub(crate) const TURN_PORTS: std::ops::Range<u16> = 1900..2300;

/// Leased turn ports: port -> lease number.
type PortLeases = Mutex<BTreeMap<u16, u64>>;

/// The turn ports leased now. In this process only, like the scoped-run
/// registry: one Hollow process runs a vault's turns.
static TURN_PORT_LEASES: Lazy<PortLeases> = Lazy::new(|| Mutex::new(BTreeMap::new()));
static NEXT_TURN_PORT_LEASE: AtomicU64 = AtomicU64::new(1);

/// One live turn's port from [`TURN_PORTS`]; dropping it frees the port.
/// Held for the whole turn (`chat::execute_turn`), so no two live turns
/// share one.
#[derive(Debug)]
pub(crate) struct TurnPort {
    port: u16,
    lease: u64,
    table: &'static PortLeases,
}

impl TurnPort {
    pub(crate) fn port(&self) -> u16 {
        self.port
    }
}

impl Drop for TurnPort {
    fn drop(&mut self) {
        let mut leases = self.table.lock().unwrap_or_else(|e| e.into_inner());
        if leases.get(&self.port) == Some(&self.lease) {
            leases.remove(&self.port);
        }
    }
}

/// Lease a port for a turn of conversation `owner` (its session key). It
/// starts from a port picked by hashing `owner`, so a conversation usually
/// gets the same port turn after turn, and takes the next one no live turn
/// holds and nothing on this Mac is listening on. `None` only when all 400
/// are taken.
pub(crate) fn lease_turn_port(owner: &str) -> Option<TurnPort> {
    lease_turn_port_in(&TURN_PORT_LEASES, owner, port_is_free)
}

fn lease_turn_port_in(
    table: &'static PortLeases,
    owner: &str,
    free: impl Fn(u16) -> bool,
) -> Option<TurnPort> {
    let hash = Sha256::digest(owner.as_bytes());
    let len = TURN_PORTS.end - TURN_PORTS.start;
    let start = u16::from_be_bytes([hash[0], hash[1]]) % len;
    let mut leases = table.lock().unwrap_or_else(|e| e.into_inner());
    let port = (0..len)
        .map(|i| TURN_PORTS.start + (start + i) % len)
        .find(|port| !leases.contains_key(port) && free(*port))?;
    let lease = NEXT_TURN_PORT_LEASE.fetch_add(1, Ordering::Relaxed);
    leases.insert(port, lease);
    Some(TurnPort { port, lease, table })
}

/// Best effort: whether nothing listens on `port` on loopback or every
/// address, IPv4 or IPv6 (a leftover dev server from before Hollow started;
/// Vite's `localhost` is often `::1` only, review 2026-10-20). The lease
/// table, not this, is what keeps two live turns apart.
fn port_is_free(port: u16) -> bool {
    let v4 = ["127.0.0.1", "0.0.0.0"]
        .iter()
        .all(|host| std::net::TcpListener::bind((*host, port)).is_ok());
    v4 && ["::1", "::"]
        .iter()
        .all(|host| !ipv6_busy(std::net::TcpListener::bind((*host, port)).err()))
}

/// Whether an IPv6 probe's bind error says the port is taken. Only "address
/// in use" does: a Mac with IPv6 off answers "address not available" or
/// "family not supported", which says nothing about the port.
fn ipv6_busy(error: Option<std::io::Error>) -> bool {
    error.is_some_and(|e| e.kind() == std::io::ErrorKind::AddrInUse)
}

/// The environment a build workspace's turn runs with. `path` is the PATH it
/// would otherwise inherit.
pub(crate) fn run_env(
    build: &crate::build_workspaces::BuildInfo,
    tool_dir: Option<&Path>,
    path: Option<&str>,
) -> Vec<(String, String)> {
    let target = crate::instance::cache_sibling("build-targets").join(worktree_id(build));
    let port = port_for(build).to_string();
    let mut env = vec![
        (
            "CARGO_TARGET_DIR".to_string(),
            target.to_string_lossy().into_owned(),
        ),
        ("PORT".into(), port.clone()),
        ("PW_PORT".into(), port),
    ];
    env.extend(turn_env(tool_dir, path));
    env
}

/// What every chat turn that can run Bash gets (OF07): the caps, the slot
/// folder, and `hollow-slot`'s folder first on `PATH`. Before OF07 only a
/// build workspace's turns had them, so the overnight lanes built their own
/// semaphores, which the slots couldn't see.
pub(crate) fn turn_env(tool_dir: Option<&Path>, path: Option<&str>) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = CAPS
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    env.push((
        "HOLLOW_BUILD_SLOT_DIR".into(),
        slot_dir().to_string_lossy().into_owned(),
    ));
    if let Some(dir) = tool_dir {
        // Only a note the last generation wrote or confirmed: a failed one
        // removes the old note, or leaves it unexported when it can't
        // (`this_mac::ensure_file`), and the turn starts without it.
        let note = dir.join(crate::this_mac::FILE_NAME);
        if crate::this_mac::is_current(&note) {
            env.push((
                "HOLLOW_THIS_MAC".into(),
                note.to_string_lossy().into_owned(),
            ));
        }
        let dir = dir.to_string_lossy();
        let joined = match path.filter(|p| !p.is_empty()) {
            Some(path) => format!("{dir}:{path}"),
            None => format!("{dir}:/usr/bin:/bin:/usr/sbin:/sbin"),
        };
        env.push(("PATH".into(), joined));
    }
    env
}

/// A chat turn's toolchain environment (OF07): a build workspace's whole
/// [`run_env`] in every mode, as before OF07; any other turn gets
/// [`turn_env`] when it can take a slot ([`can_take_slot`]: not in plan
/// mode, which runs no Bash, nor Antigravity's sandboxed modes).
/// `turn_port` (F6 part A, 2026-10-20) is the turn's port ([`TurnPortEnv`]):
/// a build worktree's own turn keeps its stable [`port_for`]; every other
/// turn gets its leased port as `PORT` and `PW_PORT`, or no Hollow-assigned
/// port when every turn port was taken. A build workspace's vault-folder conversation
/// keeps the worktree's `CARGO_TARGET_DIR` (a warm target; cargo's own lock
/// serializes two builds in it) but never its port.
pub(crate) fn chat_env(
    build: Option<&crate::build_workspaces::BuildInfo>,
    agent: crate::agent::AgentKind,
    perm: &str,
    tool_dir: Option<&Path>,
    path: Option<&str>,
    turn_port: TurnPortEnv,
) -> Vec<(String, String)> {
    let mut env = match build {
        Some(build) => run_env(build, tool_dir, path),
        None if !can_take_slot(agent, perm) => return Vec::new(),
        None => turn_env(tool_dir, path),
    };
    if turn_port != TurnPortEnv::Worktree {
        env.retain(|(key, _)| key != "PORT" && key != "PW_PORT");
    }
    if let TurnPortEnv::Leased(port) = turn_port {
        let port = port.to_string();
        env.push(("PORT".into(), port.clone()));
        env.push(("PW_PORT".into(), port));
    }
    env
}

/// Which port a chat turn runs with ([`chat_env`], F6 part A).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnPortEnv {
    /// A build worktree's own turn: its stable [`port_for`].
    Worktree,
    /// The port this turn leased ([`lease_turn_port`]).
    Leased(u16),
    /// Every turn port was taken: no Hollow-assigned port, never a build
    /// worktree's (review 2026-10-20). Hollow sets no `PORT` / `PW_PORT`, so
    /// one inherited from Hollow's own environment stays (the agent's
    /// process gets its overrides on top, nothing cleared); otherwise
    /// `hollow-slot --dev-server` falls back to [`DEV_SERVER_PORT`] under the
    /// one dev-server lock, and a Playwright config to its own default, as
    /// before part A. Safer than
    /// failing the turn: a missing port costs at worst a clash the command
    /// reports; a failed turn costs the person's request.
    Unavailable,
}

/// One lock as `build.slots_status` reports it (OF07).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LockState {
    /// `slot-0`, `slot-1`, `cargo`, `cargo-1` or `dev-server`.
    pub name: String,
    /// Whether a process holds it now (the kernel's answer, by trying it).
    pub held: bool,
    /// While held, from `<lock>.holder`, which `hollow-slot` writes once it
    /// has the lock: the command's pid, when it took the lock (RFC 3339),
    /// the dev server's port and the command line (or `HOLLOW_SLOT_LABEL`).
    /// The cargo lock's pid is its command's own, like a slot's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The build slots, the cargo lock and the dev-server lock
/// (`build.slots_status`, OF07; the cargo lock since 2026-10-20).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SlotsStatus {
    /// The folder the locks live in.
    pub dir: String,
    /// `slot-0` and `slot-1`.
    pub slots: Vec<LockState>,
    /// The one-Rust-build-at-a-time lock (a runner's lane 0), held beside a
    /// slot.
    pub cargo: LockState,
    /// A runner's second Rust lane (R0); always free on this Mac.
    pub cargo_lane_1: LockState,
    pub dev_server: LockState,
    /// The dev server's port when its turn has no `PORT` of its own.
    pub default_dev_server_port: u16,
    /// Why the folder can't be trusted, when it is a symlink, not a folder,
    /// or another user's (`hollow-slot` refuses it too); its locks then
    /// read as free and are never opened.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub untrusted: Option<String>,
}

/// A turn's `HOLLOW_SLOT_ROUTE` from Settings, Devices, Builds and tests
/// (smart routing 3, 2026-11-03): `mac-mini` for the runner, `auto`, or
/// `this-mac`, which is also the default since M7 (PER-21, 2026-11-15; it
/// was `auto`), set too so a value Hollow itself inherited never reaches a
/// turn as if it were the setting (review of #1119).
///
/// M6 (2026-11-15): the setting's runner is `runner` now, but the variable
/// keeps saying `mac-mini`. A `hollow-slot` installed before M6 (on this Mac
/// until the next install, and on the runner, which updates on its own
/// schedule) reads only `remote | mac-mini`, and would take `runner` as
/// auto. This tool reads all three.
pub(crate) fn route_env(builds_on: Option<&str>) -> (String, String) {
    let choice = match crate::agent_settings::builds_on_choice(builds_on) {
        Some(c) if c == crate::agent_settings::RUNNER => {
            crate::agent_settings::LEGACY_RUNNER.to_string()
        }
        Some(c) => c,
        None => "this-mac".into(),
    };
    ("HOLLOW_SLOT_ROUTE".to_string(), choice)
}

/// One line of `route-log` in the slot folder: where `hollow-slot` ran a
/// command and why (smart routing; Settings, Devices, Builds and tests
/// shows the latest).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RouteDecision {
    /// When it decided (RFC 3339).
    pub at: Option<String>,
    /// `here`, or the runner's SSH host.
    #[serde(rename = "where")]
    pub where_to: String,
    /// The reason it gave in its line.
    pub why: String,
    /// What it called the command (`gate`, `cargo test`, …).
    pub label: String,
    /// What chose it: `auto`, `--auto`, `--here`, `--remote`, `--on`,
    /// `--send` or `Builds and tests`.
    pub source: String,
}

/// What the runner said it has at its last preparation (`runner-caps`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunnerCaps {
    pub host: String,
    /// When (RFC 3339).
    pub at: Option<String>,
    /// `rust=1.96.0 node=26.3.1 … postgres= … slots=0/2 cargo=free lanes=2`
    /// (`cargo=busy` once every Rust lane it may use now is taken); an empty
    /// value means it has none.
    pub line: String,
}

/// The latest routing decisions (newest first) and the runner's tools.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RouteStatus {
    pub decisions: Vec<RouteDecision>,
    pub caps: Option<RunnerCaps>,
}

/// How many decisions Settings shows.
pub(crate) const ROUTE_DECISIONS_SHOWN: usize = 6;

/// [`RouteStatus`] from this Mac's slot folder.
pub(crate) fn route_status() -> RouteStatus {
    route_status_in(&slot_dir(), ROUTE_DECISIONS_SHOWN)
}

fn rfc3339_of(secs: &str) -> Option<String> {
    let secs: i64 = secs.trim().parse().ok()?;
    chrono::DateTime::from_timestamp(secs, 0).map(|at| at.to_rfc3339())
}

/// The last `limit` decisions of `dir/route-log` (newest first; lines that
/// don't parse skipped) and `dir/runner-caps`. Reads only, and nothing in a
/// folder `hollow-slot` would refuse ([`untrusted_dir`]).
pub(crate) fn route_status_in(dir: &Path, limit: usize) -> RouteStatus {
    if untrusted_dir(dir).is_some() {
        return RouteStatus {
            decisions: Vec::new(),
            caps: None,
        };
    }
    // Lossy: a label is the command's own words, any bytes at all (review
    // of #1119: one bad byte emptied the list).
    let read = |name: &str| {
        std::fs::read(dir.join(name))
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .unwrap_or_default()
    };
    // Text from the runner (a reason quoting its stderr, its tool report)
    // is cut to one line of 300 characters, like the runner status's own.
    let clip = crate::remote_status::clip;
    let log = read("route-log");
    let decisions = log
        .lines()
        .rev()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            let [at, where_to, why, label, _repo, source] = fields[..] else {
                return None;
            };
            Some(RouteDecision {
                at: rfc3339_of(at),
                where_to: clip(where_to),
                why: clip(why),
                label: clip(label),
                source: clip(source),
            })
        })
        .take(limit)
        .collect();
    let caps = Some(read("runner-caps"))
        .filter(|text| !text.is_empty())
        .and_then(|text| {
            let line = text.lines().next()?.to_string();
            let mut parts = line.splitn(3, ' ');
            let (at, host, rest) = (parts.next()?, parts.next()?, parts.next().unwrap_or(""));
            Some(RunnerCaps {
                host: clip(host),
                at: rfc3339_of(at),
                line: clip(rest),
            })
        });
    RouteStatus { decisions, caps }
}

/// This Mac's slots, cargo lock and dev-server lock, now.
pub(crate) fn status() -> SlotsStatus {
    status_in(&slot_dir())
}

/// The slots, cargo lock and dev-server lock in `dir`. Reads only: a missing lock file
/// is a free lock and is never created. A folder `hollow-slot` would refuse
/// ([`untrusted_dir`]) is reported as untrusted, its locks unopened.
pub(crate) fn status_in(dir: &Path) -> SlotsStatus {
    let untrusted = untrusted_dir(dir);
    let trusted = untrusted.is_none();
    let state = |name: &str| {
        if trusted {
            return lock_state(dir, name);
        }
        LockState {
            name: name.to_string(),
            held: false,
            pid: None,
            since: None,
            port: None,
            label: None,
        }
    };
    SlotsStatus {
        dir: dir.to_string_lossy().into_owned(),
        slots: (0..SLOTS).map(|n| state(&format!("slot-{n}"))).collect(),
        cargo: state(CARGO),
        cargo_lane_1: state(CARGO_LANE_1),
        dev_server: state(DEV_SERVER),
        default_dev_server_port: DEV_SERVER_PORT,
        untrusted,
    }
}

/// Why the slot folder can't be trusted: a symlink, not a folder, or owned
/// by another user, the same checks `hollow-slot` makes before using it. A
/// missing folder is fine (nothing holds a lock in it).
fn untrusted_dir(dir: &Path) -> Option<String> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if meta.file_type().is_symlink() {
        return Some("the slot folder is a symlink".into());
    }
    if !meta.is_dir() {
        return Some("the slot folder is not a folder".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.uid() != current_uid() {
            return Some(format!(
                "the slot folder is owned by another user (uid {})",
                meta.uid()
            ));
        }
    }
    None
}

fn lock_state(dir: &Path, name: &str) -> LockState {
    let held = is_held(&dir.join(format!("{name}.lock")));
    let mut state = LockState {
        name: name.to_string(),
        held,
        pid: None,
        since: None,
        port: None,
        label: None,
    };
    if !held {
        return state;
    }
    let Ok(text) = std::fs::read_to_string(dir.join(format!("{name}.lock.holder"))) else {
        return state;
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key {
            "pid" => state.pid = value.parse().ok(),
            "since" => {
                state.since = value
                    .parse::<i64>()
                    .ok()
                    .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
                    .map(|at| at.to_rfc3339())
            }
            "port" => state.port = value.parse().ok(),
            "label" if !value.is_empty() => state.label = Some(value.chars().take(200).collect()),
            _ => {}
        }
    }
    state
}

/// Whether a process holds `lock` now: take it without waiting and let go
/// at once. A `hollow-slot` trying at that same instant sees it taken: a
/// build tries the other slot or waits its poll, and `--dev-server` tries
/// three times a second apart, so the read never refuses one.
fn is_held(lock: &Path) -> bool {
    // A missing lock file is free; never create one.
    let Ok(file) = std::fs::File::open(lock) else {
        return false;
    };
    matches!(
        crate::flock::HeldLock::try_take(file),
        Err(std::fs::TryLockError::WouldBlock)
    )
}

#[cfg(test)]
#[path = "build_slots_remote_tests.rs"]
mod remote_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::build_workspaces::BuildInfo;
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn info(worktree: &str) -> BuildInfo {
        BuildInfo {
            repo: "/Users/x/Projects/My App".into(),
            worktree: worktree.into(),
            branch: "hollow/x".into(),
            base: "origin/main".into(),
            exists: true,
            runs_on: None,
            port: None,
        }
    }

    #[test]
    fn each_worktree_gets_its_own_target_and_port() {
        let a = run_env(&info("/w/a"), Some(Path::new("/tools")), Some("/usr/bin"));
        let b = run_env(&info("/w/b"), None, None);
        let get = |env: &[(String, String)], key: &str| {
            env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        };
        assert_ne!(get(&a, "CARGO_TARGET_DIR"), get(&b, "CARGO_TARGET_DIR"));
        assert!(get(&a, "CARGO_TARGET_DIR")
            .unwrap()
            .contains("hollow-build-targets/my-app-"));
        assert!(!get(&a, "CARGO_TARGET_DIR").unwrap().contains("Documents"));
        assert_eq!(get(&a, "CARGO_BUILD_JOBS").as_deref(), Some("8"));
        assert_eq!(get(&a, "VITEST_MAX_WORKERS").as_deref(), Some("4"));
        assert_eq!(get(&a, "PATH").as_deref(), Some("/tools:/usr/bin"));
        assert!(get(&b, "PATH").is_none());
        let port: u16 = get(&a, "PORT").unwrap().parse().unwrap();
        assert!((1500..1900).contains(&port));
        assert_eq!(get(&a, "PW_PORT"), get(&a, "PORT"));
        assert_eq!(
            port_for(&info("/w/a")),
            port_for(&info("/w/a")),
            "stable per worktree"
        );
    }

    /// B02: a Dock-launched Hollow inherits only the system folders; the
    /// build PATH still has Homebrew and cargo after the tool folder.
    #[test]
    fn a_bare_inherited_path_still_gets_the_tool_folders() {
        let home = Path::new("/Users/x");
        let base = crate::self_install::tool_path(Some("/usr/bin:/bin:/usr/sbin:/sbin"), home);
        let env = run_env(&info("/w/a"), Some(Path::new("/slot/bin")), Some(&base));
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert!(path.starts_with("/slot/bin:"), "{path}");
        let dirs: Vec<&str> = path.split(':').collect();
        assert!(dirs.contains(&"/opt/homebrew/bin"), "{path}");
        assert!(dirs.contains(&"/Users/x/.cargo/bin"), "{path}");
    }

    /// OF07: every chat turn that can run Bash gets the caps and the slot
    /// tool first on PATH; plan mode runs no Bash and gets none; a build
    /// workspace keeps its whole build env in every mode, as before.
    #[test]
    fn a_chat_turn_that_can_run_bash_gets_the_caps_and_the_slot_tool() {
        assert!(!turn_env(None, None)
            .iter()
            .any(|(key, _)| key == "HOLLOW_THIS_MAC"));
        // The machine note's path, while the note is there (J4).
        let temp = tempfile::Builder::new()
            .prefix("turn-env-this-mac-")
            .tempdir()
            .unwrap();
        let note = temp.path().join(crate::this_mac::FILE_NAME);
        std::fs::write(&note, "# This Mac\n").unwrap();
        // A file Hollow didn't generate is never exported (#1234 review).
        assert!(!turn_env(Some(temp.path()), None)
            .iter()
            .any(|(key, _)| key == "HOLLOW_THIS_MAC"));
        crate::this_mac::mark_current_for_test(&note);
        let with_note = turn_env(Some(temp.path()), None);
        assert_eq!(
            with_note
                .iter()
                .find(|(key, _)| key == "HOLLOW_THIS_MAC")
                .map(|(_, value)| value.as_str()),
            Some(note.to_string_lossy().as_ref())
        );
        let get = |env: &[(String, String)], key: &str| {
            env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
        };
        let tool = Some(Path::new("/slot/bin"));
        let claude = crate::agent::AgentKind::Claude;
        for perm in ["default", "acceptEdits", "auto", "bypassPermissions"] {
            let env = chat_env(
                None,
                claude,
                perm,
                tool,
                Some("/opt/homebrew/bin:/usr/bin"),
                TurnPortEnv::Unavailable,
            );
            assert_eq!(
                get(&env, "PATH").as_deref(),
                Some("/slot/bin:/opt/homebrew/bin:/usr/bin"),
                "{perm}"
            );
            for (key, value) in CAPS {
                assert_eq!(get(&env, key).as_deref(), Some(value), "{perm} {key}");
            }
            assert_eq!(get(&env, "CARGO_BUILD_JOBS").as_deref(), Some("8"));
            assert_eq!(get(&env, "VITEST_MAX_FORKS").as_deref(), Some("4"));
            assert!(get(&env, "HOLLOW_BUILD_SLOT_DIR").is_some(), "{perm}");
            // No note in /slot/bin, so no variable naming one.
            assert!(get(&env, "HOLLOW_THIS_MAC").is_none(), "{perm}");
            // Only a build workspace has its own target folder; a port
            // only when the turn leased one (F6 part A).
            assert!(get(&env, "CARGO_TARGET_DIR").is_none(), "{perm}");
            assert!(get(&env, "PORT").is_none(), "{perm}");
        }
        assert!(chat_env(
            None,
            claude,
            "plan",
            tool,
            Some("/usr/bin"),
            TurnPortEnv::Leased(1900)
        )
        .is_empty());
        // Antigravity's sandboxed accept-edits runs no commands.
        let agy = crate::agent::AgentKind::Antigravity;
        assert!(chat_env(
            None,
            agy,
            "acceptEdits",
            tool,
            Some("/usr/bin"),
            TurnPortEnv::Unavailable
        )
        .is_empty());
        assert!(!chat_env(
            None,
            agy,
            "bypassPermissions",
            tool,
            Some("/usr/bin"),
            TurnPortEnv::Unavailable
        )
        .is_empty());
        let build = chat_env(
            Some(&info("/w/a")),
            claude,
            "plan",
            tool,
            Some("/usr/bin"),
            TurnPortEnv::Worktree,
        );
        assert_eq!(build, run_env(&info("/w/a"), tool, Some("/usr/bin")));
        assert!(get(&build, "CARGO_TARGET_DIR").is_some());
        assert_eq!(get(&build, "VITEST_MAX_WORKERS").as_deref(), Some("4"));
    }

    /// F6 part A (2026-10-20): a turn's leased port is its `PORT` and
    /// `PW_PORT`; on a build workspace's env it replaces the worktree's.
    #[test]
    fn a_turn_port_is_the_turns_port_and_pw_port() {
        let get = |env: &[(String, String)], key: &str| {
            env.iter()
                .filter(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .collect::<Vec<_>>()
        };
        let claude = crate::agent::AgentKind::Claude;
        let env = chat_env(
            None,
            claude,
            "default",
            None,
            None,
            TurnPortEnv::Leased(1907),
        );
        assert_eq!(get(&env, "PORT"), ["1907"]);
        assert_eq!(get(&env, "PW_PORT"), ["1907"]);
        let env = chat_env(
            Some(&info("/w/a")),
            claude,
            "default",
            None,
            None,
            TurnPortEnv::Leased(1907),
        );
        assert_eq!(
            get(&env, "PORT"),
            ["1907"],
            "once, never the worktree's too"
        );
        assert_eq!(get(&env, "PW_PORT"), ["1907"]);
        assert_eq!(get(&env, "CARGO_TARGET_DIR").len(), 1);
        // Review 2026-10-20: none left is no port, never the worktree's.
        let none = TurnPortEnv::Unavailable;
        let env = chat_env(Some(&info("/w/a")), claude, "default", None, None, none);
        assert!(get(&env, "PORT").is_empty() && get(&env, "PW_PORT").is_empty());
        assert_eq!(get(&env, "CARGO_TARGET_DIR").len(), 1);
        let env = chat_env(None, claude, "default", None, None, none);
        assert!(get(&env, "PORT").is_empty() && get(&env, "PW_PORT").is_empty());
    }

    /// F6 part A: turn ports never land on a build workspace's port range,
    /// the default dev-server port or Vite's.
    #[test]
    fn turn_ports_stay_clear_of_build_workspace_ports() {
        const { assert!(TURN_PORTS.start >= 1900, "build workspaces use 1500-1899") };
        assert!(!TURN_PORTS.contains(&DEV_SERVER_PORT));
        assert!(!TURN_PORTS.contains(&1420));
        for path in ["/w/a", "/w/b", "/Users/x/Hollow/Worktrees/app/lane-1"] {
            assert!(!TURN_PORTS.contains(&port_for(&info(path))), "{path}");
        }
    }

    /// F6 part A: no two live leases share a port, a busy port is skipped,
    /// one owner gets the same port again once its last turn freed it, and
    /// the search wraps around the range.
    #[test]
    fn turn_port_leases_are_unique_and_freed_on_drop() {
        // A table of its own: other tests lease real turn ports meanwhile.
        static TABLE: Lazy<PortLeases> = Lazy::new(|| Mutex::new(BTreeMap::new()));
        let lease_turn_port_with =
            |owner: &str, free: &dyn Fn(u16) -> bool| lease_turn_port_in(&TABLE, owner, free);
        let owner = "s-turn-port-unique";
        let first = lease_turn_port_with(owner, &|_| true).unwrap();
        let again = lease_turn_port_with(owner, &|_| true).unwrap();
        assert_ne!(first.port(), again.port(), "one port per live turn");
        let port = first.port();
        drop(first);
        drop(again);
        let back = lease_turn_port_with(owner, &|_| true).unwrap();
        assert_eq!(back.port(), port, "stable per conversation");
        drop(back);
        let skipped = lease_turn_port_with(owner, &|p| p != port).unwrap();
        assert_ne!(
            skipped.port(),
            port,
            "a port something listens on is skipped"
        );
        assert!(TURN_PORTS.contains(&skipped.port()));
        drop(skipped);
        let last = TURN_PORTS.end - 1;
        let wrapped = lease_turn_port_with(owner, &|p| p == last || p == TURN_PORTS.start);
        assert!(wrapped.is_some_and(|l| l.port() == last || l.port() == TURN_PORTS.start));
        assert!(
            lease_turn_port_with(owner, &|_| false).is_none(),
            "all taken"
        );
    }

    /// F6 part A review (2026-10-20): a server listening only on IPv6
    /// loopback or the IPv6 wildcard makes its port busy too (Vite's
    /// `localhost` is often `::1`). Skipped where this Mac has no IPv6.
    #[test]
    fn a_port_something_listens_on_over_ipv6_is_busy() {
        for host in ["::1", "::"] {
            let Ok(listener) = std::net::TcpListener::bind((host, 0)) else {
                eprintln!("no IPv6 here ({host}); skipped");
                continue;
            };
            let port = listener.local_addr().unwrap().port();
            assert!(!port_is_free(port), "[{host}]:{port} is taken");
        }
        let v4 = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        assert!(!port_is_free(v4.local_addr().unwrap().port()));
        // IPv6 switched off says nothing about the port.
        use std::io::{Error, ErrorKind};
        assert!(ipv6_busy(Some(Error::from(ErrorKind::AddrInUse))));
        assert!(!ipv6_busy(Some(Error::from(ErrorKind::AddrNotAvailable))));
        assert!(!ipv6_busy(Some(Error::from(ErrorKind::Unsupported))));
        assert!(!ipv6_busy(Some(Error::from_raw_os_error(
            libc::EAFNOSUPPORT
        ))));
        assert!(!ipv6_busy(None));
    }

    /// OF07 review: which turns can take a slot. Plan runs no commands;
    /// Antigravity's `--sandbox` modes deny every command, so only its
    /// bypass; Codex's `workspace-write` and Grok's `read-only` (its
    /// `default`) and `workspace` profiles all write `/tmp`, where the slots
    /// are.
    #[test]
    fn only_a_turn_that_runs_commands_and_writes_tmp_takes_a_slot() {
        use crate::agent::AgentKind;
        for agent in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Antigravity,
            AgentKind::Grok,
        ] {
            assert!(!can_take_slot(agent, "plan"), "{agent:?}");
            assert!(can_take_slot(agent, "bypassPermissions"), "{agent:?}");
            for perm in ["default", "acceptEdits", "auto"] {
                assert_eq!(
                    can_take_slot(agent, perm),
                    agent != AgentKind::Antigravity,
                    "{agent:?} {perm}"
                );
            }
        }
    }

    /// OF07 review (B1): the slots live in `/tmp/hollow-build-slots-<uid>`,
    /// which every agent's sandbox can write, and the script's own default
    /// is that same folder (Hollow passes `HOLLOW_BUILD_SLOT_DIR` too, but a
    /// stale environment must still meet the same locks).
    #[test]
    fn the_slot_folder_is_a_per_user_temp_folder_the_script_agrees_on() {
        let uid = current_uid();
        assert_eq!(
            default_slot_dir(),
            PathBuf::from(format!("/tmp/hollow-build-slots-{uid}"))
        );
        // Only the script's `dir=` line, so no lock is taken.
        let line = SLOT_TOOL
            .lines()
            .find(|line| line.starts_with("dir="))
            .expect("the dir line");
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{line}\nprintf %s \"$dir\""))
            .env_remove("HOLLOW_BUILD_SLOT_DIR")
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            default_slot_dir().to_string_lossy()
        );
        let set = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("{line}\nprintf %s \"$dir\""))
            .env("HOLLOW_BUILD_SLOT_DIR", "/x/slots")
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&set.stdout), "/x/slots");
    }

    /// OF07 review: the tool makes the slot folder 0700, and the perl
    /// fallback says why when it can't open a lock instead of failing with
    /// a bare 1.
    #[test]
    fn the_slot_folder_is_private_and_an_unopenable_lock_says_why() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::Builder::new().prefix("mode-").tempdir().unwrap();
        let tool = dir.path().join("hollow-slot");
        std::fs::write(&tool, SLOT_TOOL).unwrap();
        let slots = dir.path().join("locks");
        let out = Command::new("/bin/sh")
            .arg(&tool)
            .args(["true"])
            .env("HOLLOW_BUILD_SLOT_DIR", &slots)
            .env("HOLLOW_SLOT_REMOTE", "")
            .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
            .env_remove("HOLLOW_SLOT_HOLDER_PID")
            .env_remove("HOLLOW_SLOT_CARGO_PID")
            .env_remove("HOLLOW_SLOT_CARGO")
            .output()
            .unwrap();
        assert!(out.status.success());
        let mode = std::fs::metadata(&slots).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
        // A lock path that is a folder can't be opened for appending.
        std::fs::create_dir(slots.join("dev-server.lock")).unwrap();
        let out = Command::new("/bin/sh")
            .arg(&tool)
            .args(["--dev-server", "true"])
            .env("HOLLOW_BUILD_SLOT_DIR", &slots)
            .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
            .env_remove("HOLLOW_SLOT_HOLDER_PID")
            .env_remove("HOLLOW_SLOT_CARGO_PID")
            .env_remove("HOLLOW_SLOT_CARGO")
            .env("HOLLOW_SLOT_NO_LOCKF", "1")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(
            said.contains("hollow-slot: cannot open the lock file"),
            "{said}"
        );
    }

    /// OF07: `hollow-slot --dev-server` is exclusive and hands out a port.
    /// The holder gets PORT/PW_PORT (its turn's own PORT when it has one)
    /// and writes who it is; a second refuses at once (after its three
    /// tries) with 75, naming the holder; a build slot still runs beside it;
    /// and the status read sees each lock held, then free. With `lockf` and
    /// with the perl fallback. The holders run until a stop file appears
    /// (made after the asserts, or when a failed assert unwinds), so the
    /// test takes only as long as its asserts.
    #[test]
    fn the_dev_server_lock_is_exclusive_and_hands_out_a_port() {
        /// Makes the stop file when dropped, so the holders always end.
        struct Stop(std::path::PathBuf);
        impl Drop for Stop {
            fn drop(&mut self) {
                let _ = std::fs::write(&self.0, "");
            }
        }
        // Also ends when the temp folder is gone (on a failed assert it is
        // removed right after the guard makes the stop file), and after 60 s
        // at most, so a test binary killed outright leaves no holder behind.
        const HOLD: &str = r#"n=0; while [ "$n" -lt 600 ] && [ -d "${STOP%/*}" ] && [ ! -e "$STOP" ]; do n=$((n + 1)); sleep 0.1; done"#;
        // A backslash, which macOS sh's echo would expand.
        const BUILD_LABEL: &str = r"lane OB \c tests";
        let has_lockf = Path::new("/usr/bin/lockf").exists();
        for no_lockf in [false, true] {
            if !no_lockf && !has_lockf {
                continue;
            }
            let dir = tempfile::Builder::new().prefix("dev-").tempdir().unwrap();
            let stop_file = dir.path().join("stop");
            let tool = dir.path().join("hollow-slot");
            std::fs::write(&tool, SLOT_TOOL).unwrap();
            let slots = dir.path().join("locks");
            let cmd = |args: &[&str]| {
                let mut cmd = Command::new("/bin/sh");
                cmd.arg(&tool)
                    .args(args)
                    .env("HOLLOW_BUILD_SLOT_DIR", &slots)
                    .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
                    .env_remove("HOLLOW_SLOT_HOLDER_PID")
                    .env_remove("HOLLOW_SLOT_CARGO_PID")
                    .env_remove("HOLLOW_SLOT_CARGO")
                    .env_remove("PORT")
                    .env_remove("HOLLOW_DEV_SERVER_PORT")
                    .env_remove("HOLLOW_SLOT_LABEL")
                    .env("HOLLOW_SLOT_POLL_SECS", "1")
                    .env("HOLLOW_SLOT_REMOTE", "")
                    .env("STOP", &stop_file);
                if no_lockf {
                    cmd.env("HOLLOW_SLOT_NO_LOCKF", "1");
                }
                cmd
            };
            let text = |bytes: &[u8]| String::from_utf8_lossy(bytes).trim().to_string();

            let out = cmd(&["--dev-server", "sh", "-c", "echo \"$PORT $PW_PORT\""])
                .output()
                .unwrap();
            assert!(out.status.success(), "no_lockf {no_lockf}");
            assert_eq!(
                text(&out.stdout),
                format!("{DEV_SERVER_PORT} {DEV_SERVER_PORT}")
            );
            assert!(
                text(&out.stderr).contains(&format!("PORT={DEV_SERVER_PORT}")),
                "it says the port"
            );
            let own = cmd(&["--dev-server", "sh", "-c", "echo $PORT"])
                .env("PORT", "1612")
                .output()
                .unwrap();
            assert_eq!(text(&own.stdout), "1612", "a build workspace's own PORT");
            let usage = cmd(&["--dev-server"]).output().unwrap();
            assert_eq!(usage.status.code(), Some(2));
            assert!(!status_in(&slots).dev_server.held, "freed when it exits");

            let stop = Stop(stop_file.clone());
            let mut server = cmd(&["--dev-server", "sh", "-c", HOLD]).spawn().unwrap();
            let mut build = cmd(&["sh", "-c", HOLD])
                .env("HOLLOW_SLOT_LABEL", BUILD_LABEL)
                .spawn()
                .unwrap();
            // Until both have taken their lock and written who they are
            // (the label is the last line of the holder file).
            let label = format!("sh -c {HOLD}");
            let started = Instant::now();
            let status = loop {
                let status = status_in(&slots);
                let named = |s: &LockState, w: &str| s.held && s.label.as_deref() == Some(w);
                if (named(&status.dev_server, label.as_str())
                    && status.slots.iter().any(|s| named(s, BUILD_LABEL)))
                    || started.elapsed() > Duration::from_secs(10)
                {
                    break status;
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            let dev = &status.dev_server;
            assert!(dev.held, "no_lockf {no_lockf}: {status:?}");
            assert_eq!(dev.name, DEV_SERVER);
            assert_eq!(dev.port, Some(DEV_SERVER_PORT));
            assert_eq!(dev.label.as_deref(), Some(label.as_str()));
            assert!(dev.pid.is_some() && dev.since.is_some(), "{status:?}");
            assert_eq!(status.slots.len(), SLOTS);
            let held: Vec<&LockState> = status.slots.iter().filter(|s| s.held).collect();
            assert_eq!(held.len(), 1, "{status:?}");
            // Kept whole: printf, not echo, whose `\c` would cut it short.
            assert_eq!(held[0].label.as_deref(), Some(BUILD_LABEL));
            assert_eq!(held[0].port, None);

            let second = cmd(&["--dev-server", "true"]).output().unwrap();
            assert_eq!(second.status.code(), Some(75), "no_lockf {no_lockf}");
            let refused = text(&second.stderr);
            assert!(
                refused.contains("another dev server holds")
                    && refused.contains(&format!("label={label}")),
                "{refused}"
            );
            assert!(
                cmd(&["true"]).output().unwrap().status.success(),
                "a build slot is a separate lock"
            );
            drop(stop);
            server.wait().unwrap();
            build.wait().unwrap();
            let after = status_in(&slots);
            assert!(!after.dev_server.held, "{after:?}");
            assert!(after.slots.iter().all(|s| !s.held), "{after:?}");
            assert_eq!(after.dev_server.pid, None, "a free lock names no holder");
        }
    }

    /// Smart routing 3: Builds and tests becomes the turn's
    /// `HOLLOW_SLOT_ROUTE`, `this-mac` for the default or anything unknown
    /// (M7; set, so a value Hollow inherited never stands in for the
    /// setting), and `auto` only when chosen.
    #[test]
    fn builds_and_tests_becomes_the_turns_route() {
        let route = |v: &str| ("HOLLOW_SLOT_ROUTE".to_string(), v.to_string());
        assert_eq!(route_env(Some("mac-mini")), route("mac-mini"));
        // M6: the runner's new name still reaches the turn as `mac-mini`,
        // the value an older installed hollow-slot understands.
        assert_eq!(route_env(Some("runner")), route("mac-mini"));
        assert_eq!(route_env(Some(" runner ")), route("mac-mini"));
        assert_eq!(route_env(Some(" this-mac ")), route("this-mac"));
        assert_eq!(route_env(Some(" auto ")), route("auto"));
        for here in [None, Some("this-mac"), Some(""), Some("elsewhere")] {
            assert_eq!(route_env(here), route("this-mac"), "{here:?}");
        }
        assert!(SLOT_TOOL.contains("*) route=auto rt_source=auto ;;"));
        // The script reads the same values.
        assert!(SLOT_TOOL.contains("remote | runner | mac-mini) route=prefer"));
        assert!(SLOT_TOOL.contains("here | this-mac) route=here"));
    }

    /// Settings reads the newest decisions of `route-log` (lines that don't
    /// parse skipped) and the runner's last report from `runner-caps`, in
    /// the formats the script writes.
    #[test]
    fn route_status_reads_the_scripts_log_and_the_runners_report() {
        let dir = tempfile::Builder::new().prefix("route-").tempdir().unwrap();
        assert_eq!(
            route_status_in(dir.path(), 6),
            RouteStatus {
                decisions: Vec::new(),
                caps: None
            }
        );
        let mut log = String::new();
        for n in 0..8 {
            log.push_str(&format!(
                "{}\there\twhy {n}\tlabel {n}\trepo-abc\tauto\n",
                1_793_729_000 + n
            ));
        }
        log.push_str("not a decision\n");
        log.push_str(
            "1793729100\trunner-host\tgate, usually 3 min; this Mac 72% free\tgate\trepo-abc\tauto\n",
        );
        std::fs::write(dir.path().join("route-log"), log).unwrap();
        std::fs::write(
            dir.path().join("runner-caps"),
            "1793729565 runner-host rust=1.96.0 node=26.3.1 postgres= slots=0/2 cargo=free\n",
        )
        .unwrap();
        let status = route_status_in(dir.path(), 3);
        assert_eq!(status.decisions.len(), 3);
        let newest = &status.decisions[0];
        assert_eq!(newest.where_to, "runner-host");
        assert_eq!(newest.why, "gate, usually 3 min; this Mac 72% free");
        assert_eq!(newest.label, "gate");
        assert_eq!(newest.source, "auto");
        assert!(newest
            .at
            .as_deref()
            .is_some_and(|at| at.starts_with("2026-11-0")));
        assert_eq!(status.decisions[1].why, "why 7");
        let caps = status.caps.unwrap();
        assert_eq!(caps.host, "runner-host");
        assert_eq!(
            caps.line,
            "rust=1.96.0 node=26.3.1 postgres= slots=0/2 cargo=free"
        );
        let json = serde_json::to_value(&status.decisions[0]).unwrap();
        assert_eq!(json["where"], "runner-host", "{json}");
        // Runner text is cut to one line, and a bad byte loses nothing else.
        let mut long =
            format!("1793729200\there\t{}\tlabel\trepo\tauto\n", "x".repeat(900)).into_bytes();
        long.extend_from_slice(b"1793729300\there\tok \xff byte\tl\trepo\tauto\n");
        std::fs::write(dir.path().join("route-log"), long).unwrap();
        let status = route_status_in(dir.path(), 6);
        assert_eq!(status.decisions.len(), 2);
        assert!(
            status.decisions[0].why.starts_with("ok "),
            "{:?}",
            status.decisions[0]
        );
        assert!(
            status.decisions[1].why.chars().count() <= 301,
            "{}",
            status.decisions[1].why.len()
        );
        // The script's own formats.
        assert!(SLOT_TOOL.contains("printf '%s\\t%s\\t%s\\t%s\\t%s\\t%s\\n' \"$(date +%s)\" \"$1\" \"$(printf '%s' \"$2\" | tr '\\t\\n' '  ')\" \"$rt_label\" \"$rt_repo\" \"$rt_source\" >> \"$dir/route-log\""));
        assert!(SLOT_TOOL.contains(
            "printf '%s %s %s\\n' \"$(date +%s)\" \"$host\" \"$r_caps\" > \"$dir/runner-caps.$$\""
        ));
    }

    /// A status read creates nothing: a missing folder is all free.
    #[test]
    fn slots_status_reads_a_missing_folder_as_free() {
        let dir = tempfile::Builder::new().prefix("free-").tempdir().unwrap();
        let missing = dir.path().join("none");
        let status = status_in(&missing);
        assert_eq!(
            status
                .slots
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["slot-0", "slot-1"]
        );
        assert!(status.slots.iter().all(|s| !s.held));
        assert!(!status.dev_server.held);
        assert!(!status.cargo.held);
        assert!(!missing.exists());
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["devServer"]["name"], "dev-server");
        assert_eq!(json["cargo"]["name"], "cargo");
        assert_eq!(json["defaultDevServerPort"], DEV_SERVER_PORT);
        // The script uses the same names and port as the status read.
        assert!(SLOT_TOOL.contains(&format!("$dir/{DEV_SERVER}.lock")));
        assert!(SLOT_TOOL.contains(&format!("$dir/{CARGO}.lock")));
        assert!(SLOT_TOOL.contains(&format!(":-{DEV_SERVER_PORT}}}")));
    }

    /// The real tool, against real locks: it runs a command in a free slot
    /// with the command's own status, and waits while both are held. Run
    /// with `lockf` (where this Mac has it) and with the perl fallback, and
    /// the locks are held by the other one, so each is seen by the other.
    #[test]
    fn hollow_slot_runs_in_a_free_slot_and_waits_while_both_are_held() {
        let has_lockf = Path::new("/usr/bin/lockf").exists();
        for no_lockf in [false, true] {
            if !no_lockf && !has_lockf {
                continue;
            }
            let dir = tempfile::Builder::new().prefix("slots-").tempdir().unwrap();
            let tool = dir.path().join("hollow-slot");
            std::fs::write(&tool, SLOT_TOOL).unwrap();
            let slots = dir.path().join("locks");
            let run = |args: &[&str]| {
                let mut cmd = Command::new("/bin/sh");
                cmd.arg(&tool)
                    .args(args)
                    .env("HOLLOW_BUILD_SLOT_DIR", &slots)
                    .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
                    .env_remove("HOLLOW_SLOT_HOLDER_PID")
                    .env_remove("HOLLOW_SLOT_CARGO_PID")
                    .env_remove("HOLLOW_SLOT_CARGO")
                    .env("HOLLOW_SLOT_POLL_SECS", "1")
                    .env("HOLLOW_SLOT_REMOTE", "");
                if no_lockf {
                    cmd.env("HOLLOW_SLOT_NO_LOCKF", "1");
                }
                cmd.output().unwrap()
            };
            let out = run(&["sh", "-c", "exit 7"]);
            assert_eq!(
                out.status.code(),
                Some(7),
                "the command's own status (no_lockf {no_lockf})"
            );
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("hollow-slot --dev-server <command>"),
                "a build call names the dev-server form (the index line doesn't)"
            );
            assert_eq!(run(&[]).status.code(), Some(2), "usage");

            std::fs::create_dir_all(&slots).unwrap();
            let hold = |n: u8| {
                let lock = slots.join(format!("slot-{n}.lock"));
                if no_lockf && has_lockf {
                    Command::new("/usr/bin/lockf")
                        .arg(&lock)
                        .args(["sleep", "2"])
                        .spawn()
                        .unwrap()
                } else {
                    Command::new("/usr/bin/perl")
                        .args(["-MFcntl=:flock", "-e", "open(my $f, '>>', $ARGV[0]) or die; flock($f, LOCK_EX) or die; sleep 2"])
                        .arg(&lock)
                        .spawn()
                        .unwrap()
                }
            };
            let mut holders = vec![hold(0), hold(1)];
            std::thread::sleep(Duration::from_millis(300));
            let started = Instant::now();
            let out = run(&["true"]);
            assert!(out.status.success(), "no_lockf {no_lockf}");
            assert!(
                started.elapsed() >= Duration::from_millis(1200),
                "it waited for a slot (no_lockf {no_lockf})"
            );
            assert!(
                String::from_utf8_lossy(&out.stderr).contains("waiting"),
                "and said so once"
            );
            for holder in &mut holders {
                holder.wait().unwrap();
            }
        }
    }

    /// One Rust build at a time (2026-10-20): the tool, a lock folder and a
    /// `bin/` of fake toolchain commands, put first on a bare `PATH`. The fake
    /// `cargo` (also linked as `cargo-clippy`, `rustc` and `npm`) prints
    /// whether it ran inside the cargo lock (saving its parent's pid, the
    /// lock's holder, to `RIG_SAVE`; with `RIG_DETACH`, it also
    /// leaves two children behind that run `hollow-slot cargo test` once the
    /// `go` file appears), then, with `RIG_LOG` set, logs
    /// its start and end, takes `RIG_LOG/running` with an atomic `mkdir` (a
    /// second build running at the same time can't, and logs `overlap`) and
    /// holds it until the stop file appears (with `RIG_HOLD`) or for
    /// `RIG_SLEEP` seconds. `other` does the same under another name, so the
    /// tool can't tell it is a build: it logs `beside` when a build holds
    /// `running` as it starts.
    struct Rig {
        dir: tempfile::TempDir,
        no_lockf: bool,
    }

    /// Makes the stop file when dropped, so the holders always end.
    struct StopOnDrop(std::path::PathBuf);
    impl Drop for StopOnDrop {
        fn drop(&mut self) {
            let _ = std::fs::write(&self.0, "");
        }
    }

    const FAKE_BUILD: &str = r#"#!/bin/sh
printf 'held=%s\n' "${HOLLOW_SLOT_CARGO_PID:-no}"
[ -z "$RIG_SAVE" ] || printf %s "$PPID" > "$RIG_SAVE"
printf 'internal=%s%s\n' "$HOLLOW_SLOT_SAY" "$HOLLOW_SLOT_SAY_ON"
if [ -n "$RIG_DETACH" ]; then
  unset RIG_DETACH
  for i in 1 2; do
    (
      n=0; while [ "$n" -lt 600 ] && [ -d "$RIG_LOG" ] && [ ! -e "$RIG_LOG/go" ]; do n=$((n + 1)); sleep 0.1; done
      exec /bin/sh "$RIG_TOOL" cargo test
    ) > /dev/null 2>> "$RIG_LOG/detached-$i.err" &
  done
fi
[ -n "$RIG_LOG" ] || exit 0
name="${0##*/}"
kind=build
[ "$name" = other ] && [ -z "$RIG_FORCED" ] && kind=other
[ "$name" = other ] || name=build
if [ "$kind" = other ]; then
  [ -d "$RIG_LOG/running" ] && echo beside >> "$RIG_LOG/log"
else
  mkdir "$RIG_LOG/running" 2>/dev/null || echo overlap >> "$RIG_LOG/log"
fi
echo "start $name" >> "$RIG_LOG/log"
if [ -n "$RIG_HOLD" ]; then
  n=0; while [ "$n" -lt 600 ] && [ -d "$RIG_LOG" ] && [ ! -e "$RIG_LOG/stop" ]; do n=$((n + 1)); sleep 0.1; done
else
  sleep "${RIG_SLEEP:-1}"
fi
echo "end $name" >> "$RIG_LOG/log"
[ "$kind" = other ] || rmdir "$RIG_LOG/running" 2>/dev/null
exit 0
"#;

    impl Rig {
        fn new(no_lockf: bool) -> Rig {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::Builder::new().prefix("cargo-").tempdir().unwrap();
            std::fs::write(dir.path().join("hollow-slot"), SLOT_TOOL).unwrap();
            let bin = dir.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            for name in ["cargo", "cargo-clippy", "rustc", "npm", "npx", "other"] {
                let path = bin.join(name);
                std::fs::write(&path, FAKE_BUILD).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            std::fs::create_dir(dir.path().join("log")).unwrap();
            std::fs::create_dir(dir.path().join("src-tauri")).unwrap();
            Rig { dir, no_lockf }
        }
        fn slots(&self) -> PathBuf {
            self.dir.path().join("locks")
        }
        fn log_dir(&self) -> PathBuf {
            self.dir.path().join("log")
        }
        fn stop(&self) -> StopOnDrop {
            StopOnDrop(self.log_dir().join("stop"))
        }
        /// Start a new phase: no stop file, an empty log.
        fn reset(&self) {
            let _ = std::fs::remove_file(self.log_dir().join("stop"));
            let _ = std::fs::remove_file(self.log_dir().join("log"));
        }
        /// `cmd` held until the stop file appears.
        fn held(&self, mut cmd: Command, name: &str) -> std::process::Child {
            cmd.env("RIG_HOLD", "1");
            self.spawn(cmd, name)
        }
        fn log(&self) -> String {
            std::fs::read_to_string(self.log_dir().join("log")).unwrap_or_default()
        }
        /// `hollow-slot <args>` with the rig's PATH, locks and log.
        fn cmd(&self, args: &[&str]) -> Command {
            let mut cmd = self.sh();
            cmd.arg(self.dir.path().join("hollow-slot")).args(args);
            cmd
        }
        /// `/bin/sh` with the rig's PATH, locks and log, and no arguments.
        fn sh(&self) -> Command {
            let mut cmd = Command::new("/bin/sh");
            cmd.current_dir(self.dir.path())
                .env(
                    "PATH",
                    format!("{}:/usr/bin:/bin", self.dir.path().join("bin").display()),
                )
                .env("HOLLOW_BUILD_SLOT_DIR", self.slots())
                .env("HOLLOW_SLOT_MIN_FREE_PCT", "0")
                .env("HOLLOW_SLOT_POLL_SECS", "1")
                .env("HOLLOW_SLOT_REMOTE", "")
                .env("RIG_LOG", self.log_dir())
                .env_remove("HOLLOW_SLOT_CARGO")
                .env_remove("HOLLOW_SLOT_CARGO_PID")
                .env_remove("HOLLOW_SLOT_LABEL")
                .env_remove("HOLLOW_SLOT_HOLDER_PID")
                .env_remove("HOLLOW_SLOT_SAY_ON")
                .env_remove("HOLLOW_SLOT_RUST_LANES")
                .env_remove("HOLLOW_SLOT_LANE_JOBS")
                .env_remove("HOLLOW_SLOT_SYSCTL")
                .env_remove("CARGO_BUILD_JOBS")
                .env_remove("RIG_HOLD")
                .env_remove("RIG_FORCED")
                .env_remove("RIG_DETACH")
                .env_remove("RIG_SAVE")
                .env("RIG_TOOL", self.dir.path().join("hollow-slot"));
            if self.no_lockf {
                cmd.env("HOLLOW_SLOT_NO_LOCKF", "1");
            }
            cmd
        }
        /// Spawn with stderr to `<name>.err` in the log folder.
        fn spawn(&self, mut cmd: Command, name: &str) -> std::process::Child {
            let err = std::fs::File::create(self.log_dir().join(format!("{name}.err"))).unwrap();
            cmd.stdout(std::process::Stdio::null())
                .stderr(err)
                .spawn()
                .unwrap()
        }
        fn stderr_of(&self, name: &str) -> String {
            std::fs::read_to_string(self.log_dir().join(format!("{name}.err"))).unwrap_or_default()
        }
        /// Wait (60 s at most, so a hang fails instead of blocking) until `done`.
        fn until(&self, what: &str, mut done: impl FnMut() -> bool) {
            let started = Instant::now();
            while !done() {
                assert!(
                    started.elapsed() < Duration::from_secs(60),
                    "no hang: {what} (no_lockf {}); log: {}",
                    self.no_lockf,
                    self.log()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    pub(super) fn lock_modes() -> Vec<bool> {
        let has_lockf = Path::new("/usr/bin/lockf").exists();
        [false, true]
            .into_iter()
            .filter(|no_lockf| *no_lockf || has_lockf)
            .collect()
    }

    /// 2026-10-20: two Rust builds never run at once (two Hollow compiles
    /// together peak around 13 GB), even when one hides in a `sh -c` string,
    /// while a command that isn't one runs beside a build in the other slot;
    /// the second build says once why it waits.
    #[test]
    fn two_rust_builds_never_overlap_while_other_work_runs_beside_one() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            let stop = rig.stop();
            // The first build runs until the stop file, so everything below
            // happens while it holds the cargo lock.
            let first = rig.held(rig.cmd(&["cargo", "build"]), "first");
            rig.until("the first build starts", || {
                rig.log().contains("start build")
            });
            // The status read names the cargo lock's holder.
            let status = status_in(&rig.slots());
            assert!(status.cargo.held, "{status:?}");
            assert_eq!(status.cargo.label.as_deref(), Some("cargo build"));
            assert!(status.cargo.pid.is_some(), "{status:?}");
            let second = rig.spawn(rig.cmd(&["sh", "-c", "cd . && cargo test"]), "second");
            rig.until("the second build says it waits", || {
                rig.stderr_of("second")
                    .contains("waiting for the other Rust build")
            });
            let other = rig.cmd(&["other"]).output().unwrap();
            assert!(other.status.success(), "no_lockf {no_lockf}");
            assert!(
                !String::from_utf8_lossy(&other.stderr).contains("waiting"),
                "{other:?}"
            );
            drop(stop);
            for mut child in [first, second] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }
            let log = rig.log();
            assert!(!log.contains("overlap"), "no_lockf {no_lockf}: {log}");
            assert_eq!(log.matches("end build").count(), 2, "{log}");
            assert!(
                log.contains("beside"),
                "the other command ran beside a build (no_lockf {no_lockf}): {log}"
            );
            assert!(
                rig.stderr_of("second").contains(
                    "waiting for the other Rust build to finish (one at a time on this Mac)"
                ),
                "{}",
                rig.stderr_of("second")
            );
        }
    }

    /// A fake `cargo` for the lane tests: logs its pid and jobs when it
    /// starts, holds until the stop file appears, then logs its end.
    const FAKE_LANE_BUILD: &str = r#"#!/bin/sh
echo "start $$ jobs=${CARGO_BUILD_JOBS:-unset}" >> "$RIG_LOG/log"
n=0; while [ "$n" -lt 600 ] && [ ! -e "$RIG_LOG/stop" ]; do n=$((n + 1)); sleep 0.1; done
echo "end $$" >> "$RIG_LOG/log"
"#;

    /// R0 (2026-11-14): with `HOLLOW_SLOT_RUST_LANES=2` (a runner's runs have
    /// it) two Rust builds run at once, each with 5 jobs, and a third waits;
    /// the memory watch turns lane 1 off under the floor (the second waits,
    /// saying why, although lane 1 is free), and the one lane left keeps its 5
    /// jobs (never 8 beside lane 1's 5); lane 1 starts as memory comes back.
    /// This Mac's own calls (no variable) don't use lane 1.
    #[test]
    fn two_rust_lanes_run_two_builds_at_once_and_low_memory_falls_back_to_one() {
        use std::os::unix::fs::PermissionsExt;
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            let build = rig.dir.path().join("bin/cargo");
            std::fs::write(&build, FAKE_LANE_BUILD).unwrap();
            std::fs::set_permissions(&build, std::fs::Permissions::from_mode(0o755)).unwrap();
            let level = rig.dir.path().join("level");
            std::fs::write(&level, "80\n").unwrap();
            let sysctl = rig.dir.path().join("bin/fake-sysctl");
            std::fs::write(&sysctl, "#!/bin/sh\ncat \"$(dirname \"$0\")/../level\"\n").unwrap();
            std::fs::set_permissions(&sysctl, std::fs::Permissions::from_mode(0o755)).unwrap();
            let lanes = |mut cmd: Command| {
                cmd.env("HOLLOW_SLOT_RUST_LANES", "2")
                    .env("HOLLOW_SLOT_LANE_JOBS", "1")
                    .env("HOLLOW_SLOT_MIN_FREE_PCT", "30")
                    .env("HOLLOW_SLOT_SYSCTL", &sysctl);
                cmd
            };
            let starts = |rig: &Rig| rig.log().matches("start ").count();
            let stop = rig.stop();

            // Memory 80%: two builds at once, 5 jobs each, a third waits.
            let first = rig.spawn(lanes(rig.cmd(&["cargo", "build"])), "first");
            rig.until("the first starts", || starts(&rig) == 1);
            let second = rig.spawn(lanes(rig.cmd(&["cargo", "test"])), "second");
            rig.until("the second starts beside it", || starts(&rig) == 2);
            let status = status_in(&rig.slots());
            assert!(
                status.cargo.held && status.cargo_lane_1.held,
                "both lanes taken: {status:?}"
            );
            assert!(status.slots.iter().all(|slot| slot.held), "{status:?}");
            let log = rig.log();
            assert_eq!(log.matches("jobs=5").count(), 2, "{log}");
            let third = rig.spawn(lanes(rig.cmd(&["cargo", "clippy"])), "third");
            rig.until("the third says it waits", || {
                rig.stderr_of("third").contains(
                    "both Rust lanes are in use; waiting for one to free up (two at a time on this runner)",
                )
            });
            assert_eq!(starts(&rig), 2, "a third never ran beside two");
            drop(stop);
            for mut child in [first, second, third] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }
            assert_eq!(starts(&rig), 3, "{}", rig.log());

            // Memory 12%: the first build waits for memory holding lane 0
            // (5 jobs still); the second can't have lane 1.
            rig.reset();
            let stop = rig.stop();
            std::fs::write(&level, "12\n").unwrap();
            let first = rig.spawn(lanes(rig.cmd(&["cargo", "build"])), "first");
            rig.until("the first holds lane 0 while it waits for memory", || {
                status_in(&rig.slots()).cargo.held
            });
            let second = rig.spawn(lanes(rig.cmd(&["cargo", "test"])), "second");
            rig.until("the second says why it waits", || {
                rig.stderr_of("second")
                    .contains("one at a time while this runner's memory is under its floor")
            });
            std::thread::sleep(Duration::from_millis(2500));
            assert_eq!(
                starts(&rig),
                0,
                "nothing starts under the floor: {}",
                rig.log()
            );
            assert!(
                !status_in(&rig.slots()).cargo_lane_1.held,
                "lane 1 stays off with low memory"
            );

            // Memory is back: the first runs with its 5 jobs, the second
            // takes lane 1 with 5.
            std::fs::write(&level, "80\n").unwrap();
            rig.until("both run", || starts(&rig) == 2);
            let log = rig.log();
            assert_eq!(log.matches("jobs=5").count(), 2, "{log}");
            assert_eq!(log.matches("jobs=8").count(), 0, "{log}");
            drop(stop);
            for mut child in [first, second] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }

            // This Mac's own call: no variable, one lane, whatever memory.
            rig.reset();
            let _ = std::fs::remove_file(rig.slots().join("cargo-1.lock"));
            let stop = rig.stop();
            let first = rig.spawn(rig.cmd(&["cargo", "build"]), "first");
            rig.until("the first starts", || starts(&rig) == 1);
            let second = rig.spawn(rig.cmd(&["cargo", "test"]), "second");
            rig.until("the second waits", || {
                rig.stderr_of("second").contains(
                    "waiting for the other Rust build to finish (one at a time on this Mac)",
                )
            });
            std::thread::sleep(Duration::from_millis(1500));
            assert_eq!(starts(&rig), 1, "{}", rig.log());
            assert!(!status_in(&rig.slots()).cargo_lane_1.held);
            drop(stop);
            for mut child in [first, second] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }
            assert!(
                !rig.slots().join("cargo-1.lock").exists(),
                "this Mac never creates lane 1's lock"
            );
        }
    }

    /// 2026-10-20: `HOLLOW_SLOT_CARGO=1` makes any command a Rust build (a
    /// wrapper script the argv can't show), and `=0` lets a cargo command
    /// run beside another.
    #[test]
    fn the_cargo_override_works_both_ways() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            // Forced: the first holds until the stop file; the second must
            // say it waits for it.
            let stop = rig.stop();
            let forced = || {
                let mut cmd = rig.cmd(&["other"]);
                cmd.env("HOLLOW_SLOT_CARGO", "1").env("RIG_FORCED", "1");
                cmd
            };
            let a = rig.held(forced(), "a");
            rig.until("the forced first starts", || {
                rig.log().contains("start other")
            });
            let b = rig.spawn(forced(), "b");
            rig.until("the forced second says it waits", || {
                rig.stderr_of("b")
                    .contains("waiting for the other Rust build")
            });
            assert!(!rig.log().contains("overlap"), "{}", rig.log());
            drop(stop);
            for mut child in [a, b] {
                assert!(child.wait().unwrap().success());
            }
            let log = rig.log();
            assert_eq!(log.matches("end other").count(), 2, "{log}");
            assert!(
                !log.contains("overlap"),
                "forced builds take turns (no_lockf {no_lockf}): {log}"
            );

            // Opted out: the second runs while the first still holds, in
            // the other slot (until the stop file, so they surely meet).
            rig.reset();
            let stop = rig.stop();
            let opted_out = || {
                let mut cmd = rig.cmd(&["cargo", "build"]);
                cmd.env("HOLLOW_SLOT_CARGO", "0");
                cmd
            };
            let c = rig.held(opted_out(), "c");
            rig.until("the opted-out first starts", || {
                rig.log().contains("start build")
            });
            let d = rig.held(opted_out(), "d");
            rig.until("the opted-out second starts or waits", || {
                rig.log().matches("start build").count() == 2
                    || rig.stderr_of("d").contains("waiting")
            });
            let log = rig.log();
            assert!(
                log.contains("overlap"),
                "opted out, they run together (no_lockf {no_lockf}): {log} {}",
                rig.stderr_of("d")
            );
            drop(stop);
            for mut child in [c, d] {
                assert!(child.wait().unwrap().success());
            }
        }
    }

    /// 2026-10-20: which commands count as a Rust build, by their words (npm
    /// options before or after `run` included, review of PR #859): the
    /// command sees `HOLLOW_SLOT_CARGO_PID` only inside the cargo lock, and
    /// never the re-run's private markers. An npm option's value is one
    /// argument, spaces and all, and only a string after `-c` is split into
    /// words (delta reviews of PR #859).
    #[test]
    fn a_rust_build_is_told_from_its_words() {
        let rig = Rig::new(false);
        let cargo_path = rig.dir.path().join("bin/cargo");
        let cargo_path = cargo_path.to_str().unwrap();
        let cases: &[(&[&str], bool)] = &[
            (&["cargo", "test", "--lib"], true),
            (&[cargo_path, "build"], true),
            (&["cargo-clippy"], true),
            (&["rustc", "--version"], true),
            (&["npx", "tauri", "build"], true),
            (&["npm", "run", "gate"], true),
            (&["npm", "run", "gate:rust"], true),
            (&["npm", "run", "perf"], true),
            (&["npm", "run", "tauri:bundle"], true),
            (&["npm", "run", "tauri:build"], true),
            (&["npm", "run", "--silent", "gate"], true),
            (&["npm", "run", "-s", "gate:rust"], true),
            (&["npm", "--prefix", "x", "run", "gate"], true),
            (&["npm", "run", "--prefix", "x", "gate"], true),
            (&["npm", "run-script", "--loglevel=warn", "perf"], true),
            (&["sh", "-c", "npm run --silent gate:rust"], true),
            (&["npm", "run", "--prefix", "/tmp/my project", "gate"], true),
            (&["npm", "run", "--workspace", "my app", "gate:rust"], true),
            (&["npm", "run", "-w", "a b c", "perf"], true),
            (
                &["npm", "run", "--prefix=/tmp/my project", "tauri:build"],
                true,
            ),
            (&["npm", "--prefix", "/tmp/my project", "run", "gate"], true),
            (&["sh", "-c", "cd src-tauri && cargo clippy --lib"], true),
            (&["sh", "-c", "cd src-tauri;cargo test"], true),
            (&["sh", "-ce", "cargo build"], true),
            (&["sh", "-cxe", "cargo build"], true),
            (&["bash", "-xc", "cargo build"], true),
            (&["sh", "-xec", "npm run gate"], true),
            (&["sh", "-ec", "cd src-tauri && cargo test"], true),
            (&["npm", "test"], false),
            (&["npm", "run", "typecheck"], false),
            (&["npm", "run", "gate:frontend"], false),
            (&["npm", "run", "tauri:install"], false),
            (&["npm", "run", "--silent", "typecheck"], false),
            (&["npm", "--prefix", "gate", "run", "test"], false),
            (
                &["npm", "--prefix", "/tmp/my project", "run", "test"],
                false,
            ),
            (
                &["npm", "run", "--prefix", "/tmp/gate dir", "typecheck"],
                false,
            ),
            (&["npm", "run", "--workspace", "gate", "test"], false),
            (&["npx", "vitest", "run", "src/a.test.ts"], false),
            (&["sh", "-c", "cd src-tauri && npm test"], false),
            (&["other", "--cargo-ish"], false),
            (&["npx", "echo cargo build"], false),
            (&["npx", "-x", "echo cargo build"], false),
            (&["npx", "-C", "echo cargo build"], false),
            (&["npx", "--rc", "echo cargo build"], false),
        ];
        for (args, cargo) in cases {
            // The fake command is what prints; `sh -c` reaches one at its end.
            let mut args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            if args[0] == "sh" || args[0] == "bash" {
                args[2].push_str(" && npm x");
            }
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut cmd = rig.cmd(&args);
            cmd.env_remove("RIG_LOG");
            let out = cmd.output().unwrap();
            assert!(out.status.success(), "{args:?}: {out:?}");
            let stdout = String::from_utf8_lossy(&out.stdout);
            assert_eq!(!stdout.contains("held=no"), *cargo, "{args:?}: {out:?}");
            // The re-run's private markers never reach the command.
            assert!(stdout.contains("internal=\n"), "{args:?}: {out:?}");
        }
        // The override.
        let mut forced = rig.cmd(&["npm", "test"]);
        forced.env_remove("RIG_LOG").env("HOLLOW_SLOT_CARGO", "1");
        assert!(!String::from_utf8_lossy(&forced.output().unwrap().stdout).contains("held=no"));
        let mut off = rig.cmd(&["cargo", "build"]);
        off.env_remove("RIG_LOG").env("HOLLOW_SLOT_CARGO", "0");
        assert!(String::from_utf8_lossy(&off.output().unwrap().stdout).contains("held=no"));
    }

    /// 2026-10-20: no deadlock when both slots and the cargo lock are
    /// contended. A build waiting for the cargo lock holds no slot (it takes
    /// the cargo lock first), so other work still gets one; once the first
    /// build ends, everything that waited runs and ends.
    #[test]
    fn no_deadlock_when_both_slots_and_the_cargo_lock_are_contended() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            let stop = rig.stop();
            let held = |name: &str, args: &[&str]| {
                let mut cmd = rig.cmd(args);
                cmd.env("RIG_HOLD", "1");
                rig.spawn(cmd, name)
            };
            // A build holds the cargo lock and one slot until the stop file.
            let first = held("first", &["cargo", "build"]);
            rig.until("the first build starts", || {
                rig.log().contains("start build")
            });
            // A second build waits for the cargo lock...
            let second = rig.spawn(rig.cmd(&["cargo", "test"]), "second");
            rig.until("the second build says it waits", || {
                rig.stderr_of("second")
                    .contains("waiting for the other Rust build")
            });
            // ...holding no slot: other work gets the second one at once.
            let quick = rig.cmd(&["other"]).output().unwrap();
            assert!(quick.status.success(), "no_lockf {no_lockf}");
            assert!(
                !String::from_utf8_lossy(&quick.stderr).contains("both build slots are in use"),
                "a build waiting for the cargo lock held a slot (no_lockf {no_lockf}): {}",
                String::from_utf8_lossy(&quick.stderr)
            );
            // Now both slots are taken (the first build and a held other)
            // and the cargo lock too; one more command waits for a slot.
            let busy = held("busy", &["other"]);
            rig.until("the held other starts", || {
                rig.log().matches("start other").count() == 2
            });
            let queued = rig.spawn(rig.cmd(&["other"]), "queued");
            rig.until("the queued command says it waits", || {
                rig.stderr_of("queued")
                    .contains("both build slots are in use")
            });
            drop(stop);
            for mut child in [first, second, busy, queued] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }
            let log = rig.log();
            assert_eq!(log.matches("end build").count(), 2, "{log}");
            assert!(!log.contains("overlap"), "{log}");
        }
    }

    /// Delta reviews of PR #859: a Rust build that asks `hollow-slot` for
    /// another Rust build from inside itself is refused at once with 2 (it
    /// would wait for its own lock), not let through; a non-Rust command
    /// inside a Rust build runs in its caller's slot.
    #[test]
    fn a_rust_build_inside_a_rust_build_is_refused_at_once() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            let mut nested = rig.spawn(
                rig.cmd(&["sh", "-c", "/bin/sh \"$RIG_TOOL\" cargo test"]),
                "nested",
            );
            let mut code = None;
            rig.until("the nested build is refused", || {
                code = nested.try_wait().unwrap().map(|status| status.code());
                code.is_some()
            });
            assert_eq!(
                code,
                Some(Some(2)),
                "no_lockf {no_lockf}: {}",
                rig.stderr_of("nested")
            );
            assert!(
                rig.stderr_of("nested")
                    .contains("already inside a Rust build that holds the cargo lock"),
                "{}",
                rig.stderr_of("nested")
            );
            assert!(!rig.log().contains("start build"), "{}", rig.log());

            // A non-Rust command inside a (forced) Rust build runs in its
            // caller's slot: the second stays free for other work.
            let stop = rig.stop();
            let mut outer = rig.cmd(&[
                "sh",
                "-c",
                "unset HOLLOW_SLOT_CARGO; /bin/sh \"$RIG_TOOL\" other",
            ]);
            outer.env("HOLLOW_SLOT_CARGO", "1");
            let outer = rig.held(outer, "outer");
            rig.until("the nested command runs", || {
                rig.log().contains("start other")
            });
            let status = status_in(&rig.slots());
            assert!(status.cargo.held, "{status:?}");
            assert_eq!(
                status.slots.iter().filter(|s| s.held).count(),
                1,
                "{status:?}"
            );
            assert!(
                rig.stderr_of("outer")
                    .contains("running in the build slot of the command that called it"),
                "{}",
                rig.stderr_of("outer")
            );
            let free = rig.cmd(&["true"]).output().unwrap();
            assert!(
                !String::from_utf8_lossy(&free.stderr).contains("waiting"),
                "{free:?}"
            );
            drop(stop);
            let mut outer = outer;
            assert!(outer.wait().unwrap().success(), "no_lockf {no_lockf}");
        }
    }

    /// Delta reviews of PR #859: no process skips the cargo lock. Forged
    /// hints only move a Rust build out of a slot or refuse it; two callers
    /// forked from one shell that opened cargo.lock on fd 9 while it was free
    /// take turns; two children a finished build left
    /// behind (they carry its variables) take turns when let go at once
    /// with the lock free, and both wait while another build holds it.
    #[test]
    fn no_process_skips_the_cargo_lock() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            // A pid that has ended: a hint that names no ancestor.
            let mut gone = Command::new("/usr/bin/true").spawn().unwrap();
            let gone_pid = gone.id().to_string();
            gone.wait().unwrap();

            // Forged hints while another build holds the lock: a holder
            // pid that is a live ancestor (this test) makes a Rust build wait
            // for cargo.lock in no slot; with a dead cargo pid as well, the
            // same; a cargo pid that is a live ancestor refuses it. None
            // builds unlocked.
            let me = std::process::id().to_string();
            let stop = rig.stop();
            let holder = rig.held(rig.cmd(&["cargo", "build"]), "holder");
            rig.until("the holder starts", || rig.log().contains("start build"));
            let mut a = rig.cmd(&["cargo", "test"]);
            a.env("HOLLOW_SLOT_HOLDER_PID", &me);
            let a = rig.spawn(a, "a");
            let mut b = rig.cmd(&["cargo", "test"]);
            b.env("HOLLOW_SLOT_HOLDER_PID", &me)
                .env("HOLLOW_SLOT_CARGO_PID", &gone_pid);
            let b = rig.spawn(b, "b");
            // The old re-run marker with its parent named: no bypass either.
            let mut d = rig.cmd(&["cargo", "test"]);
            d.env("HOLLOW_SLOT_REEXEC", "1")
                .env("HOLLOW_SLOT_CARGO_PID", &me);
            let d = rig.spawn(d, "d");
            rig.until("the forged callers wait", || {
                ["a", "b", "d"].iter().all(|name| {
                    rig.stderr_of(name)
                        .contains("waiting for the other Rust build")
                })
            });
            let mut c = rig.cmd(&["cargo", "test"]);
            c.env("HOLLOW_SLOT_HOLDER_PID", &me)
                .env("HOLLOW_SLOT_CARGO_PID", &me);
            let refused = c.output().unwrap();
            assert_eq!(refused.status.code(), Some(2), "{refused:?}");
            assert!(!rig.log().contains("overlap"), "{}", rig.log());
            assert_eq!(rig.log().matches("start build").count(), 1, "{}", rig.log());
            drop(stop);
            for mut child in [holder, a, b, d] {
                assert!(child.wait().unwrap().success(), "no_lockf {no_lockf}");
            }
            assert_eq!(rig.log().matches("end build").count(), 4, "{}", rig.log());
            assert!(!rig.log().contains("overlap"), "{}", rig.log());

            // Two callers forked from one shell that opened cargo.lock on
            // fd 9 (one open file between them), with a live holder pid that
            // isn't their ancestor, let go at once while the lock is free.
            rig.reset();
            let mut live = Command::new("/bin/cat")
                .stdin(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            let stop = rig.stop();
            let mut pair = rig.sh();
            pair.args([
                "-c",
                "exec 9>>\"$0\"; /bin/sh \"$1\" cargo build & /bin/sh \"$1\" cargo test; wait",
            ])
            .arg(rig.slots().join("cargo.lock"))
            .arg(rig.dir.path().join("hollow-slot"))
            .env("HOLLOW_SLOT_HOLDER_PID", live.id().to_string())
            .env("HOLLOW_SLOT_CARGO_PID", live.id().to_string());
            std::fs::create_dir_all(rig.slots()).unwrap();
            let pair = rig.held(pair, "pair");
            rig.until("one runs and the other waits (or both run)", || {
                let started = rig.log().matches("start build").count();
                started == 2
                    || (started == 1
                        && rig
                            .stderr_of("pair")
                            .contains("waiting for the other Rust build"))
            });
            assert!(
                !rig.log().contains("overlap") && rig.log().matches("start build").count() == 1,
                "two callers sharing one open cargo.lock both built (no_lockf {no_lockf}): {}",
                rig.log()
            );
            drop(stop);
            let mut pair = pair;
            assert!(pair.wait().unwrap().success(), "no_lockf {no_lockf}");
            assert!(!rig.log().contains("overlap"), "{}", rig.log());
            live.kill().unwrap();
            live.wait().unwrap();

            // A build that leaves two children waiting for `go`.
            let leave_two = || {
                rig.reset();
                for i in 1..=2 {
                    let _ = std::fs::remove_file(rig.log_dir().join(format!("detached-{i}.err")));
                }
                let mut build = rig.cmd(&["cargo", "build"]);
                build.env("RIG_DETACH", "1").env("RIG_SLEEP", "0");
                let out = build.output().unwrap();
                assert!(out.status.success(), "{out:?}");
                rig.reset();
            };
            leave_two();
            std::fs::write(rig.log_dir().join("go"), "").unwrap();
            rig.until("the two left behind run", || {
                rig.log().matches("end build").count() == 2
            });
            std::fs::remove_file(rig.log_dir().join("go")).unwrap();
            assert!(
                !rig.log().contains("overlap"),
                "no_lockf {no_lockf}: {}",
                rig.log()
            );

            leave_two();
            let stop = rig.stop();
            let holder = rig.held(rig.cmd(&["cargo", "build"]), "holder");
            rig.until("the holder starts", || rig.log().contains("start build"));
            std::fs::write(rig.log_dir().join("go"), "").unwrap();
            rig.until("both left behind wait", || {
                (1..=2).all(|i| {
                    rig.stderr_of(&format!("detached-{i}"))
                        .contains("waiting for the other Rust build")
                })
            });
            assert!(!rig.log().contains("overlap"), "{}", rig.log());
            drop(stop);
            let mut holder = holder;
            assert!(holder.wait().unwrap().success(), "no_lockf {no_lockf}");
            rig.until("the two left behind run after the holder", || {
                rig.log().matches("end build").count() == 3
            });
            assert!(!rig.log().contains("overlap"), "{}", rig.log());
        }
    }

    /// Delta reviews of PR #859: the lock lives exactly as long as its
    /// holder. Killed with SIGKILL, the holder frees it at once, though the
    /// build it started (which never had the lock's file) still runs.
    #[test]
    fn killing_the_holder_frees_the_cargo_lock_at_once() {
        let rig = Rig::new(false);
        let saved = rig.dir.path().join("holder-pid");
        let stop = rig.stop();
        let mut build = rig.cmd(&["cargo", "build"]);
        build.env("RIG_SAVE", &saved);
        let mut build = rig.held(build, "build");
        rig.until("the build starts", || rig.log().contains("start build"));
        let pid = std::fs::read_to_string(&saved).unwrap();
        assert!(status_in(&rig.slots()).cargo.held);
        let killed = Command::new("/bin/kill")
            .args(["-9", pid.trim()])
            .status()
            .unwrap();
        assert!(killed.success(), "kill -9 {pid}");
        rig.until("the cargo lock is free", || {
            !status_in(&rig.slots()).cargo.held
        });
        assert!(rig.log().contains("start build") && !rig.log().contains("end build"));
        drop(stop);
        assert!(!build.wait().unwrap().success(), "the holder was killed");
        rig.until("the orphaned build ends", || {
            rig.log().contains("end build")
        });
    }

    /// Delta review of 86fd3f5f: the lock-order cycle can't form. Two
    /// non-Rust wrappers hold both slots and each runs a nested
    /// `hollow-slot cargo build`, while an independent Rust caller wants a
    /// slot and the cargo lock: the caller holds neither while it waits (it
    /// takes both in one try), the nested builds take cargo.lock in turn
    /// inside their callers' slots, and everything ends.
    #[test]
    fn no_lock_order_cycle_between_slots_and_the_cargo_lock() {
        for no_lockf in lock_modes() {
            let rig = Rig::new(no_lockf);
            let stop = rig.stop();
            let wrapper = |name: &str| {
                let mut cmd = rig.cmd(&[
                    "sh",
                    "-c",
                    "unset HOLLOW_SLOT_CARGO; n=0; while [ \"$n\" -lt 600 ] && [ ! -e \"$RIG_LOG/go\" ]; do n=$((n + 1)); sleep 0.1; done; /bin/sh \"$RIG_TOOL\" cargo build",
                ]);
                cmd.env("HOLLOW_SLOT_CARGO", "0");
                rig.spawn(cmd, name)
            };
            let w1 = wrapper("w1");
            let w2 = wrapper("w2");
            rig.until("the wrappers hold both slots", || {
                status_in(&rig.slots()).slots.iter().all(|s| s.held)
            });
            let caller = rig.spawn(rig.cmd(&["cargo", "test"]), "caller");
            rig.until("the caller waits", || {
                rig.stderr_of("caller").contains("waiting")
            });
            assert!(
                !status_in(&rig.slots()).cargo.held,
                "the waiting caller holds no lock"
            );
            std::fs::write(rig.log_dir().join("go"), "").unwrap();
            for mut child in [w1, w2, caller] {
                let mut done = None;
                rig.until("everything ends", || {
                    done = child.try_wait().unwrap();
                    done.is_some()
                });
                assert!(
                    done.unwrap().success(),
                    "no_lockf {no_lockf}: {}",
                    rig.log()
                );
            }
            drop(stop);
            let log = rig.log();
            assert_eq!(log.matches("end build").count(), 3, "{log}");
            assert!(!log.contains("overlap"), "{log}");
        }
    }

    /// Delta review of 86fd3f5f: a shell option cluster holding c (`sh -ce`)
    /// is a command string like `-c`: two such builds take turns.
    #[test]
    fn a_sh_ce_build_waits_its_turn() {
        let rig = Rig::new(false);
        let stop = rig.stop();
        let first = rig.held(rig.cmd(&["sh", "-ce", "cargo build"]), "first");
        rig.until("the first starts", || rig.log().contains("start build"));
        let second = rig.held(rig.cmd(&["sh", "-cxe", "cargo build"]), "second");
        rig.until("the second waits or runs", || {
            rig.stderr_of("second")
                .contains("waiting for the other Rust build")
                || rig.log().matches("start build").count() == 2
        });
        assert!(!rig.log().contains("overlap"), "{}", rig.log());
        assert_eq!(rig.log().matches("start build").count(), 1, "{}", rig.log());
        drop(stop);
        for mut child in [first, second] {
            assert!(child.wait().unwrap().success());
        }
        assert!(!rig.log().contains("overlap"), "{}", rig.log());
    }
}
