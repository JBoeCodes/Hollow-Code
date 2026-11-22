//! Write-once binaries between the Macs (two-Macs row V4, reworked to
//! Alex's N15 answer of 2026-11-13): screenshots, captures, PDFs,
//! recordings and other files the vault root's git keeps out (`sync_vault.rs`,
//! CX6b) **stay on the Mac that made them** and come over ssh only when asked.
//!
//! **On demand** ([`fetch_one`], `vault.fetch_peer_file`). One vault path at a
//! time, pulled from the peer when this Mac lacks it: never overwrites (a
//! path already here is left alone; the copy links in without clobbering),
//! size and SHA-256 checked, the peer's mtime kept, an interrupted fetch
//! resumes from its part. [`peer_missing`] (`vault.peer_files`) lists what
//! the peer has that this Mac lacks. Both only read on the peer.
//!
//! **What can come over.** A vault file [`copyable`] allows: one the vault
//! root leaves out because it is binary or big (a media folder, a binary or
//! data extension, over 1 MB). Never: Markdown, `.txt`, `.json`, `.jsonl` or
//! other live-state files, the personal folders (`me/`, `memory/`,
//! `learnings/`, `home/`), `tasks/`, any dot file or folder at any depth,
//! dependency and build folders, CX1's secret key file names, symlinks.
//!
//! **The peer.** The runner this Mac names in `~/.config/hollow/slot-remote`
//! (`remote_session::configured_runner`), reached with its own ssh key and
//! config: `BatchMode`, a 10 s connect timeout, no passwords, no agent or
//! port forwarding. Its vault is `remoteVault` (home-relative or absolute;
//! default: this vault's path below the home folder). Everything there runs
//! in one small Perl program ([`REMOTE_SCRIPT`]) whose arguments are quoted;
//! no secret is ever in argv.
//!
//! **The retired timed copy** ([`run_cycle`], [`plan`]). V4 first shipped
//! (#1454) as a two-way copy every 15 minutes, off by default and never run
//! on a real second Mac. N15 retired it: no timer starts, `enabled: true` is
//! refused and the code stays unused (tested) until Alex confirms or drops
//! it. Its rules: a file on one side only is copied to the other (never
//! deletes); newest wins only over the version both held at the last copy;
//! otherwise both are kept as `Name (from <machine>).ext`.
//!
//! **Resumable.** A transfer writes `.<name>.hollow-part` beside the target
//! (a dot name, so never listed or copied), checks size and SHA-256, sets the
//! mtime and only then links it in under its name (no clobber).
//!
//! **State.** App data only, never the vault: `file-copy.json` (the peer's
//! vault path; the retired switch, always saved off) and the timed copy's
//! old record `.hollow-files.json` (read for the status, never deleted).
//! `configure_file_copy` is user-only; agents read `settings.file_copy_status`.

use crate::context_roots as roots;
use crate::sync_engine::vault::{path_allowed, MANIFEST, MAX_BYTES};
use crate::sync_engine::PERSONAL_ALLOW;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The record of what each Mac has, in app data.
pub(crate) const RECORD_FILE: &str = ".hollow-files.json";
/// The person's switch and the peer's vault path, in app data.
const SETTINGS_FILE: &str = "file-copy.json";
/// A transfer in progress: `.<name>` plus this, beside the target.
pub(crate) const PART_SUFFIX: &str = ".hollow-part";
/// Folders never walked or copied at any depth (dependencies, builds,
/// caches, test output). The media folders of the vault manifest are copied.
pub(crate) const NEVER_DIRS: &[&str] = &[
    "node_modules",
    "__pycache__",
    "venv",
    "target",
    "test-results",
    "playwright-report",
];
/// Extensions never copied: text (git carries what it should) and live
/// state that is not write-once.
pub(crate) const NEVER_EXTS: &[&str] = &[
    "md",
    "markdown",
    "txt",
    "json",
    "jsonl",
    "lock",
    "tmp",
    "swp",
    "sqlite-wal",
    "sqlite-shm",
    "db-wal",
    "db-shm",
    "hollow-part",
    "yaml",
    "yml",
    "toml",
    "env",
    "sql",
];
/// One peer step's ceiling (a 512 MB copy at a slow 1 MB/s fits); a fetch
/// stopped by it keeps its part and the next try resumes.
pub(crate) const STEP_TIMEOUT: Duration = Duration::from_secs(20 * 60);
/// Bytes one cycle copies at most; the rest waits for the next cycle (the
/// first copy of a cycle always runs, so a bigger file still moves).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const CYCLE_BYTES: u64 = 512 * 1024 * 1024;
/// Bytes of a peer's error kept for the status.
const ERR_CAP: usize = 400;

/// Whether vault-relative `rel` (`/` separators) of `size` bytes is one V4
/// copies (see the module notes).
pub(crate) fn copyable(rel: &str, size: u64) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || p.starts_with('.')) {
        return false;
    }
    let top = parts[0];
    if PERSONAL_ALLOW.iter().any(|p| p.eq_ignore_ascii_case(top))
        || MANIFEST
            .top_dirs
            .iter()
            .any(|d| d.eq_ignore_ascii_case(top))
    {
        return false;
    }
    let (dirs, name) = parts.split_at(parts.len() - 1);
    let name = name[0];
    if dirs
        .iter()
        .any(|d| NEVER_DIRS.iter().any(|n| n.eq_ignore_ascii_case(d)))
    {
        return false;
    }
    if name.ends_with('~') {
        return false;
    }
    if let Some((_, ext)) = name.rsplit_once('.') {
        if NEVER_EXTS.contains(&ext.to_ascii_lowercase().as_str()) {
            return false;
        }
    }
    if crate::scan_rules::path_rule_matches("secret.key_file", rel) {
        return false;
    }
    !path_allowed(rel) || size > MAX_BYTES
}

// ── Listings and the planner ────────────────────────────────────────────

/// One file on one Mac: size, mtime (whole seconds) and its SHA-256 once
/// known.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub size: u64,
    pub mtime: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

impl Entry {
    fn same_stat(&self, other: &Entry) -> bool {
        self.size == other.size && self.mtime == other.mtime
    }
}

/// Vault-relative path to entry.
pub(crate) type Listing = BTreeMap<String, Entry>;

/// What one cycle does with one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Action {
    /// Only here: copy it to the peer.
    ToPeer(String),
    /// Only there: copy it here.
    FromPeer(String),
    /// The peer still holds the last copied version and this Mac's is newer.
    ReplacePeer(String),
    /// This Mac still holds the last copied version and the peer's is newer.
    ReplaceHere(String),
    /// Both changed: the peer's version lands here as `here`, this Mac's
    /// lands there as `there`; `path` is untouched on both sides.
    KeepBoth {
        path: String,
        here: String,
        there: String,
    },
    Skip(String, SkipWhy),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SkipWhy {
    /// Same size and mtime, or the same hash.
    Same,
    /// A hash the decision needs could not be read.
    Unhashed,
    /// This clash was already kept both.
    KeptBoth,
}

/// The paths whose hash a [`plan`] needs and lacks: (here, there). Present
/// on both with another size or mtime: both; one side only: that side's (a
/// copy is checked against it).
pub(crate) fn wanted_hashes(local: &Listing, peer: &Listing) -> (Vec<String>, Vec<String>) {
    let mut here = Vec::new();
    let mut there = Vec::new();
    for (path, l) in local {
        match peer.get(path) {
            Some(p) if l.same_stat(p) => {}
            Some(p) => {
                if l.hash.is_none() {
                    here.push(path.clone());
                }
                if p.hash.is_none() {
                    there.push(path.clone());
                }
            }
            None if l.hash.is_none() => here.push(path.clone()),
            None => {}
        }
    }
    for (path, p) in peer {
        if !local.contains_key(path) && p.hash.is_none() {
            there.push(path.clone());
        }
    }
    (here, there)
}

/// The cycle's actions, one per path in either listing (pure). `synced`:
/// the hash both sides held at the last copy; `clashes`: the (here, there)
/// hash pairs already kept both; `reserved`: copy names a past clash still
/// owes the peer; the machine names make the copy names. A copy name is
/// taken when any of these holds it in any case (APFS ignores case).
pub(crate) fn plan(
    local: &Listing,
    peer: &Listing,
    synced: &BTreeMap<String, String>,
    clashes: &BTreeMap<String, [String; 2]>,
    reserved: &BTreeSet<String>,
    machines: (&str, &str),
) -> Vec<Action> {
    let (this_machine, peer_machine) = machines;
    let paths: BTreeSet<&String> = local.keys().chain(peer.keys()).collect();
    let mut assigned: BTreeSet<String> = local
        .keys()
        .chain(peer.keys())
        .chain(reserved.iter())
        .map(|p| p.to_lowercase())
        .collect();
    let mut out = Vec::new();
    for path in paths {
        let action = match (local.get(path), peer.get(path)) {
            (Some(_), None) => Action::ToPeer(path.clone()),
            (None, Some(_)) => Action::FromPeer(path.clone()),
            (None, None) => continue,
            (Some(l), Some(p)) => {
                if l.same_stat(p) {
                    Action::Skip(path.clone(), SkipWhy::Same)
                } else {
                    match (&l.hash, &p.hash) {
                        (Some(lh), Some(ph)) => {
                            let base = synced.get(path);
                            if lh == ph {
                                Action::Skip(path.clone(), SkipWhy::Same)
                            } else if clashes
                                .get(path)
                                .is_some_and(|c| &c[0] == lh && &c[1] == ph)
                            {
                                Action::Skip(path.clone(), SkipWhy::KeptBoth)
                            } else if base == Some(ph) && l.mtime > p.mtime {
                                Action::ReplacePeer(path.clone())
                            } else if base == Some(lh) && p.mtime > l.mtime {
                                Action::ReplaceHere(path.clone())
                            } else {
                                let taken = |c: &str| assigned.contains(&c.to_lowercase());
                                let here =
                                    crate::sync_engine::copy_name(path, peer_machine, &taken);
                                let there = {
                                    let taken = |c: &str| {
                                        taken(c) || c.to_lowercase() == here.to_lowercase()
                                    };
                                    crate::sync_engine::copy_name(path, this_machine, &taken)
                                };
                                assigned.insert(here.to_lowercase());
                                assigned.insert(there.to_lowercase());
                                Action::KeepBoth {
                                    path: path.clone(),
                                    here,
                                    there,
                                }
                            }
                        }
                        _ => Action::Skip(path.clone(), SkipWhy::Unhashed),
                    }
                }
            }
        };
        out.push(action);
    }
    out
}

// ── The peer ────────────────────────────────────────────────────────────

/// Why the peer did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PeerError {
    /// Not reachable (ssh could not connect): the cycle stops quietly.
    Offline(String),
    /// Reached, and this one step failed (named in the status).
    Failed(String),
}

impl PeerError {
    fn text(&self) -> &str {
        match self {
            PeerError::Offline(t) | PeerError::Failed(t) => t,
        }
    }
}

/// The other Mac's vault, as V4 uses it. [`ShellPeer`] is the one
/// implementation (ssh in the app, `sh -c` over a temp folder in tests).
pub(crate) trait Peer {
    /// What the record is kept against (the ssh host).
    fn id(&self) -> String;
    /// The peer's machine name and its files (every regular file outside
    /// dot and [`NEVER_DIRS`] folders; [`copyable`] is applied here).
    fn list(&mut self) -> Result<(String, Listing), PeerError>;
    /// SHA-256 of each of `paths` there (a missing or unreadable one is left out).
    fn hashes(&mut self, paths: &[String]) -> Result<BTreeMap<String, String>, PeerError>;
    /// Bytes already in `rel`'s part file there (0 when none).
    fn partial(&mut self, rel: &str) -> Result<u64, PeerError>;
    /// Write `data` (the file from `offset` on) into `rel`'s part there,
    /// check it against `entry`, then link it in (no clobber) or, with
    /// `expect`, replace a file that still has that size and mtime and,
    /// hashed again just before the rename, `expect.hash` (else the part is
    /// dropped and nothing is replaced).
    fn receive(
        &mut self,
        rel: &str,
        offset: u64,
        entry: &Entry,
        expect: Option<&Entry>,
        data: &mut (dyn Read + Send),
    ) -> Result<(), PeerError>;
    /// Stream `rel` from `offset` on into `out`.
    fn send(&mut self, rel: &str, offset: u64, out: &mut dyn Write) -> Result<(), PeerError>;
    /// The peer's machine name and `rel` there when it is a regular file
    /// [`copyable`] allows (the on-demand fetch's one lookup).
    fn stat(&mut self, rel: &str) -> Result<(String, Option<Entry>), PeerError> {
        let (name, mut listing) = self.list()?;
        Ok((name, listing.remove(rel)))
    }
}

/// The program every peer step runs (`perl -e`, modes `list`, `stat`,
/// `hash`, `part`, `recv`, `send`). Paths are vault-relative and checked: no empty
/// or dot component, no absolute path, no symlink anywhere below the vault
/// (`O_NOFOLLOW` opens, `lstat` on every folder). Its own failures exit 3;
/// ssh's are 255. `__NEVER__` becomes [`NEVER_DIRS`].
pub(crate) const REMOTE_SCRIPT: &str = r#"use strict; use warnings;
use Fcntl qw(O_RDONLY O_WRONLY O_CREAT O_NOFOLLOW SEEK_SET);
use File::Find (); use Digest::SHA (); use Sys::Hostname ();
my ($mode, $root, @a) = @ARGV;
my %never = map { $_ => 1 } split /,/, '__NEVER__';
sub safe { my $r = shift; die "bad path\n" if !defined $r || $r eq '' || $r =~ m{^/} || grep { $_ eq '' || /^\./ } split(m{/}, $r, -1); $r }
sub dirs { my ($r, $make) = @_; my @p = split m{/}, $r; pop @p; my $d = $root;
  for my $c (@p) { $d .= "/$c";
    if (!lstat $d) { die "missing folder\n" unless $make; mkdir $d or die "mkdir: $!\n"; lstat $d or die "lstat: $!\n"; }
    die "not a folder: $c\n" if -l _ || !-d _; } $d }
sub part { my @p = split m{/}, shift; my $n = pop @p; join('/', $root, @p, ".$n.hollow-part") }
sub sha { my $p = shift; sysopen(my $h, $p, O_RDONLY|O_NOFOLLOW) or return undef; binmode $h; Digest::SHA->new(256)->addfile($h)->hexdigest }
my $ok = eval {
  $root = "$ENV{HOME}/$root" unless $root =~ m{^/};
  $root =~ s{/+$}{};
  die "no vault at $root\n" if -l $root || !-d $root;
  binmode STDOUT; binmode STDIN;
  if ($mode eq 'list') {
    my $h = Sys::Hostname::hostname(); $h =~ s/\..*//; print "H\t$h\0";
    File::Find::find({ no_chdir => 1, preprocess => sub { grep { !/^\./ && !$never{$_} } @_ }, wanted => sub {
      my $f = $File::Find::name; return if $f eq $root; my @s = lstat $f or return; return unless -f _;
      print "$s[7]\t$s[9]\t", substr($f, length($root) + 1), "\0"; } }, $root);
  } elsif ($mode eq 'stat') {
    my $r = safe($a[0]); my $h = Sys::Hostname::hostname(); $h =~ s/\..*//; print "H\t$h\0";
    if (eval { dirs($r, 0); 1 }) { my @s = lstat "$root/$r"; print "$s[7]\t$s[9]\t$r\0" if @s && -f _; }
  } elsif ($mode eq 'hash') {
    local $/ = "\0";
    while (my $r = <STDIN>) { chomp $r; next unless eval { safe($r); dirs($r, 0); 1 };
      my $d = sha("$root/$r"); print "$d\t$r\0" if defined $d; }
  } elsif ($mode eq 'part') {
    my @s = lstat part(safe($a[0])); print((@s && -f _) ? $s[7] : 0);
  } elsif ($mode eq 'recv') {
    my ($r, $off, $size, $mt, $want, $es, $em, $eh) = @a; safe($r); dirs($r, 1);
    my ($p, $f) = (part($r), "$root/$r");
    sysopen(my $fh, $p, O_WRONLY|O_CREAT|O_NOFOLLOW, 0644) or die "part: $!\n";
    my $have = (stat $fh)[7]; die "part is shorter than $off\n" if $have < $off;
    truncate($fh, $off) or die "truncate: $!\n"; sysseek($fh, $off, SEEK_SET) or die "seek: $!\n";
    my $buf; while (my $n = sysread(STDIN, $buf, 1 << 20)) { my $o = 0;
      while ($o < $n) { my $w = syswrite($fh, $buf, $n - $o, $o); die "write: $!\n" unless defined $w; $o += $w; } }
    close $fh or die "close: $!\n";
    my $got = (lstat $p)[7];
    if ($got > $size) { unlink $p; die "too long\n"; }
    die "stopped at $got of $size\n" if $got < $size;
    my $sum = sha($p) // ''; if ($sum ne $want) { unlink $p; die "content differs from what was sent\n"; }
    utime($mt, $mt, $p) or die "utime: $!\n";
    if (defined $es) { my @s = lstat $f;
      my $cur = (@s && -f _ && $s[7] == $es && $s[9] == $em) ? (sha($f) // '') : '';
      if ($cur eq '' || $cur ne ($eh // '')) { unlink $p; die "changed there since the plan\n"; }
      rename($p, $f) or die "rename: $!\n"; }
    else { link($p, $f) or die "already there: $r\n"; unlink $p; }
    print "ok";
  } elsif ($mode eq 'send') {
    my ($r, $off) = @a; safe($r); dirs($r, 0);
    sysopen(my $fh, "$root/$r", O_RDONLY|O_NOFOLLOW) or die "open: $!\n"; binmode $fh;
    sysseek($fh, $off, SEEK_SET) or die "seek: $!\n";
    my $buf; while (my $n = sysread($fh, $buf, 1 << 20)) { print STDOUT $buf or die "write: $!\n"; }
  } else { die "unknown mode\n"; }
  1 };
if (!$ok) { print STDERR $@; exit 3; }
"#;

/// The ssh argv that runs `remote_cmd` on `host`: the host's own key and
/// config, never a password or a forwarded agent, a 10 s connect timeout.
pub(crate) fn ssh_argv(host: &str) -> Vec<String> {
    [
        "ssh",
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=15",
        "-o",
        "ServerAliveCountMax=4",
        "-o",
        "PasswordAuthentication=no",
        "-o",
        "KbdInteractiveAuthentication=no",
        "-o",
        "ForwardAgent=no",
        "-o",
        "ClearAllForwardings=yes",
        "--",
        host,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// The one command string the peer's shell runs for `mode`: the script and
/// every argument single-quoted.
pub(crate) fn remote_command(mode: &str, root: &str, args: &[String]) -> String {
    let q = crate::remote_session::shell_quote;
    let script = REMOTE_SCRIPT.replace("__NEVER__", &NEVER_DIRS.join(","));
    let mut out = format!("perl -e {} -- {} {}", q(&script), q(mode), q(root));
    for a in args {
        out.push(' ');
        out.push_str(&q(a));
    }
    out
}

/// A peer reached by running [`remote_command`] after `prefix` (the
/// [`ssh_argv`] for a host; `sh -c` in tests).
pub(crate) struct ShellPeer {
    pub id: String,
    pub prefix: Vec<String>,
    /// The peer's vault: home-relative or absolute.
    pub root: String,
    /// A step that runs longer is killed (its whole process group).
    pub timeout: Duration,
}

impl ShellPeer {
    pub(crate) fn ssh(host: &str, root: &str) -> Self {
        ShellPeer {
            id: host.to_string(),
            prefix: ssh_argv(host),
            root: root.to_string(),
            timeout: STEP_TIMEOUT,
        }
    }

    fn run(
        &self,
        mode: &str,
        args: &[String],
        input: Option<&mut (dyn Read + Send)>,
        out: &mut dyn Write,
    ) -> Result<(), PeerError> {
        let Some((program, rest)) = self.prefix.split_first() else {
            return Err(PeerError::Failed("no peer command".into()));
        };
        let mut child = Command::new(program)
            .process_group(0)
            .args(rest)
            .arg(remote_command(mode, &self.root, args))
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PeerError::Failed(format!("{program} did not start: {e}")))?;
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let pid = child.id() as libc::pid_t;
        let timed_out = AtomicBool::new(false);
        let (done, watch) = std::sync::mpsc::channel::<()>();
        let limit = self.timeout;
        let copied = std::thread::scope(|s| {
            let flag = &timed_out;
            s.spawn(move || {
                if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) = watch.recv_timeout(limit) {
                    flag.store(true, Ordering::SeqCst);
                    // The whole group: ssh, or a shell and what it started.
                    // SAFETY: kill(2) on our own child's process group.
                    unsafe {
                        libc::kill(-pid, libc::SIGKILL);
                    }
                }
            });
            if let (Some(mut pipe), Some(src)) = (stdin, input) {
                s.spawn(move || {
                    let _ = std::io::copy(src, &mut pipe);
                });
            }
            let err = s.spawn(move || {
                let mut buf = Vec::new();
                let _ = stderr.read_to_end(&mut buf);
                buf
            });
            let copied = std::io::copy(&mut stdout, out);
            let err = err.join().unwrap_or_default();
            let _ = done.send(());
            (copied, err)
        });
        let status = child.wait().map_err(|e| PeerError::Failed(e.to_string()))?;
        if timed_out.load(Ordering::SeqCst) {
            return Err(PeerError::Offline(format!(
                "the peer step took over {} s and was stopped",
                limit.as_secs()
            )));
        }
        let (copied, err) = copied;
        let mut msg = String::from_utf8_lossy(&err).trim().to_string();
        if msg.len() > ERR_CAP {
            let mut at = ERR_CAP;
            while !msg.is_char_boundary(at) {
                at -= 1;
            }
            msg.truncate(at);
        }
        match status.code() {
            Some(0) => copied
                .map(drop)
                .map_err(|e| PeerError::Failed(format!("reading the peer: {e}"))),
            Some(255) => Err(PeerError::Offline(if msg.is_empty() {
                "the peer did not answer".into()
            } else {
                msg
            })),
            _ => Err(PeerError::Failed(if msg.is_empty() {
                format!("the peer step stopped ({status})")
            } else {
                msg
            })),
        }
    }

    /// The peer's machine name: its host name as a slug, else the ssh host's.
    fn name(&self, host: Option<String>) -> String {
        host.as_deref()
            .and_then(roots::suggest_slug)
            .or_else(|| roots::suggest_slug(&self.id))
            .unwrap_or_else(|| "other-mac".into())
    }

    fn run_bytes(
        &self,
        mode: &str,
        args: &[String],
        input: Option<&mut (dyn Read + Send)>,
    ) -> Result<Vec<u8>, PeerError> {
        let mut out = Vec::new();
        self.run(mode, args, input, &mut out)?;
        Ok(out)
    }
}

/// `size\tmtime\tpath` NUL-separated records after an `H\thost` record.
pub(crate) fn parse_listing(bytes: &[u8]) -> (Option<String>, Listing) {
    let mut host = None;
    let mut out = Listing::new();
    for rec in bytes.split(|b| *b == 0).filter(|r| !r.is_empty()) {
        let Ok(text) = std::str::from_utf8(rec) else {
            continue;
        };
        let mut it = text.splitn(3, '\t');
        match (it.next(), it.next(), it.next()) {
            (Some("H"), Some(h), None) => host = Some(h.to_string()),
            (Some(size), Some(mtime), Some(path)) => {
                if let (Ok(size), Ok(mtime)) = (size.parse(), mtime.parse()) {
                    if copyable(path, size) {
                        out.insert(
                            path.to_string(),
                            Entry {
                                size,
                                mtime,
                                hash: None,
                            },
                        );
                    }
                }
            }
            _ => {}
        }
    }
    (host, out)
}

impl Peer for ShellPeer {
    fn id(&self) -> String {
        self.id.clone()
    }

    fn list(&mut self) -> Result<(String, Listing), PeerError> {
        let bytes = self.run_bytes("list", &[], None)?;
        let (host, listing) = parse_listing(&bytes);
        Ok((self.name(host), listing))
    }

    fn stat(&mut self, rel: &str) -> Result<(String, Option<Entry>), PeerError> {
        let bytes = self.run_bytes("stat", &[rel.to_string()], None)?;
        let (host, mut listing) = parse_listing(&bytes);
        Ok((self.name(host), listing.remove(rel)))
    }

    fn hashes(&mut self, paths: &[String]) -> Result<BTreeMap<String, String>, PeerError> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let mut input = Vec::new();
        for p in paths {
            input.extend_from_slice(p.as_bytes());
            input.push(0);
        }
        let mut reader = std::io::Cursor::new(input);
        let bytes = self.run_bytes("hash", &[], Some(&mut reader))?;
        Ok(bytes
            .split(|b| *b == 0)
            .filter_map(|r| std::str::from_utf8(r).ok())
            .filter_map(|r| r.split_once('\t'))
            .filter(|(h, _)| h.len() == 64)
            .map(|(h, p)| (p.to_string(), h.to_string()))
            .collect())
    }

    fn partial(&mut self, rel: &str) -> Result<u64, PeerError> {
        let bytes = self.run_bytes("part", &[rel.to_string()], None)?;
        Ok(String::from_utf8_lossy(&bytes).trim().parse().unwrap_or(0))
    }

    fn receive(
        &mut self,
        rel: &str,
        offset: u64,
        entry: &Entry,
        expect: Option<&Entry>,
        data: &mut (dyn Read + Send),
    ) -> Result<(), PeerError> {
        let mut args = vec![
            rel.to_string(),
            offset.to_string(),
            entry.size.to_string(),
            entry.mtime.to_string(),
            entry.hash.clone().unwrap_or_default(),
        ];
        if let Some(e) = expect {
            args.push(e.size.to_string());
            args.push(e.mtime.to_string());
            args.push(e.hash.clone().unwrap_or_default());
        }
        let out = self.run_bytes("recv", &args, Some(data))?;
        if out == b"ok" {
            Ok(())
        } else {
            Err(PeerError::Failed(
                "the peer did not confirm the copy".into(),
            ))
        }
    }

    fn send(&mut self, rel: &str, offset: u64, out: &mut dyn Write) -> Result<(), PeerError> {
        self.run("send", &[rel.to_string(), offset.to_string()], None, out)
    }
}

// ── This Mac's side ─────────────────────────────────────────────────────

fn secs(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    }
}

/// Every [`copyable`] regular file in `vault` (no symlink followed, dot and
/// [`NEVER_DIRS`] folders not walked).
pub(crate) fn list_local(vault: &Path) -> Listing {
    let mut out = Listing::new();
    let mut stack = vec![(vault.to_path_buf(), String::new())];
    while let Some((dir, rel)) = stack.pop() {
        let Ok(read) = fs::read_dir(&dir) else {
            continue;
        };
        for e in read.flatten() {
            let Ok(name) = e.file_name().into_string() else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(meta) = fs::symlink_metadata(e.path()) else {
                continue;
            };
            let path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if meta.is_dir() {
                if !NEVER_DIRS.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
                    stack.push((e.path(), path));
                }
            } else if meta.is_file() && copyable(&path, meta.len()) {
                let mtime = meta.modified().map(secs).unwrap_or(0);
                out.insert(
                    path,
                    Entry {
                        size: meta.len(),
                        mtime,
                        hash: None,
                    },
                );
            }
        }
    }
    out
}

/// Whether `rel` is a path this side may touch: relative, no empty or dot
/// component.
fn safe_rel(rel: &str) -> bool {
    !rel.is_empty()
        && !rel.starts_with('/')
        && rel.split('/').all(|p| !p.is_empty() && !p.starts_with('.'))
}

/// `rel`'s folder under `vault`, every level a real folder (created when
/// `make`); a symlink anywhere is refused.
fn local_dir(vault: &Path, rel: &str, make: bool) -> Result<PathBuf, String> {
    if !safe_rel(rel) {
        return Err(format!("{rel}: not a vault path"));
    }
    let meta = fs::symlink_metadata(vault).map_err(|e| e.to_string())?;
    if !meta.is_dir() {
        return Err("the vault is not a folder".into());
    }
    let mut dir = vault.to_path_buf();
    let parts: Vec<&str> = rel.split('/').collect();
    for c in &parts[..parts.len() - 1] {
        dir.push(c);
        match fs::symlink_metadata(&dir) {
            Ok(m) if m.is_dir() => {}
            Ok(_) => return Err(format!("{rel}: {c} is not a folder")),
            Err(_) if make => {
                fs::create_dir(&dir).map_err(|e| format!("{rel}: {e}"))?;
            }
            Err(e) => return Err(format!("{rel}: {e}")),
        }
    }
    Ok(dir)
}

fn open_nofollow(path: &Path) -> std::io::Result<fs::File> {
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
}

/// SHA-256 (hex) of `vault/rel`, opened without following a link.
pub(crate) fn hash_local(vault: &Path, rel: &str) -> Option<String> {
    let dir = local_dir(vault, rel, false).ok()?;
    let name = rel.rsplit('/').next()?;
    let mut file = open_nofollow(&dir.join(name)).ok()?;
    hash_reader(&mut file).ok()
}

fn hash_reader(r: &mut dyn Read) -> std::io::Result<String> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn part_name(name: &str) -> String {
    format!(".{name}{PART_SUFFIX}")
}

/// Copy `src` (this Mac's file, as `entry`) to `dst` there, resuming a part.
fn push(
    vault: &Path,
    peer: &mut dyn Peer,
    src: &str,
    dst: &str,
    entry: &Entry,
    expect: Option<&Entry>,
) -> Result<u64, PeerError> {
    let fail = |e: String| PeerError::Failed(format!("{src}: {e}"));
    let dir = local_dir(vault, src, false).map_err(fail)?;
    let name = src.rsplit('/').next().unwrap_or(src);
    let mut file = open_nofollow(&dir.join(name)).map_err(|e| fail(e.to_string()))?;
    let meta = file.metadata().map_err(|e| fail(e.to_string()))?;
    if meta.len() != entry.size || meta.modified().map(secs).ok() != Some(entry.mtime) {
        return Err(fail("changed since it was listed".into()));
    }
    let have = peer.partial(dst)?;
    let offset = if have > entry.size { 0 } else { have };
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| fail(e.to_string()))?;
    peer.receive(dst, offset, entry, expect, &mut file)?;
    Ok(offset)
}

/// Copy the peer's `src` (as `entry`) here as `dst`, resuming a part.
fn pull(
    vault: &Path,
    peer: &mut dyn Peer,
    src: &str,
    dst: &str,
    entry: &Entry,
    expect: Option<&Entry>,
) -> Result<u64, PeerError> {
    let fail = |e: String| PeerError::Failed(format!("{dst}: {e}"));
    let want = entry.hash.clone().ok_or_else(|| fail("no hash".into()))?;
    let dir = local_dir(vault, dst, true).map_err(fail)?;
    let name = dst.rsplit('/').next().unwrap_or(dst);
    let target = dir.join(name);
    let part = dir.join(part_name(name));
    // A part that is also linked elsewhere (an app that died between the
    // link and the unlink) starts over, so a resume never appends to it.
    if fs::symlink_metadata(&part).is_ok_and(|m| m.nlink() > 1) {
        fs::remove_file(&part).map_err(|e| fail(e.to_string()))?;
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&part)
        .map_err(|e| fail(e.to_string()))?;
    let mut offset = file.metadata().map_err(|e| fail(e.to_string()))?.len();
    if offset > entry.size {
        file.set_len(0).map_err(|e| fail(e.to_string()))?;
        offset = 0;
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| fail(e.to_string()))?;
    let fetched = peer.send(src, offset, &mut file);
    file.flush().map_err(|e| fail(e.to_string()))?;
    drop(file);
    fetched?;
    let got = fs::symlink_metadata(&part).map(|m| m.len()).unwrap_or(0);
    if got > entry.size {
        let _ = fs::remove_file(&part);
        return Err(fail("longer than listed".into()));
    }
    if got < entry.size {
        return Err(fail(format!(
            "stopped at {got} of {} bytes; the next try resumes it",
            entry.size
        )));
    }
    let sum = open_nofollow(&part)
        .and_then(|mut f| hash_reader(&mut f))
        .map_err(|e| fail(e.to_string()))?;
    if sum != want {
        let _ = fs::remove_file(&part);
        return Err(fail("content differs from the peer's".into()));
    }
    let t = UNIX_EPOCH + Duration::from_secs(entry.mtime.max(0) as u64);
    fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&part)
        .and_then(|f| f.set_modified(t))
        .map_err(|e| fail(e.to_string()))?;
    match expect {
        Some(e) => {
            let ok = fs::symlink_metadata(&target).is_ok_and(|m| {
                m.is_file() && m.len() == e.size && m.modified().map(secs).ok() == Some(e.mtime)
            }) && e.hash.is_some()
                && open_nofollow(&target)
                    .and_then(|mut f| hash_reader(&mut f))
                    .ok()
                    == e.hash;
            if !ok {
                let _ = fs::remove_file(&part);
                return Err(fail("changed here since the plan".into()));
            }
            fs::rename(&part, &target).map_err(|e| fail(e.to_string()))?;
        }
        None => {
            if let Err(e) = fs::hard_link(&part, &target) {
                // Lost to a file that appeared meanwhile: it wins; drop ours.
                let _ = fs::remove_file(&part);
                return Err(fail(format!("already here ({e})")));
            }
            let _ = fs::remove_file(&part);
        }
    }
    Ok(offset)
}

// ── On demand (N15): one file from the other Mac when asked ─────────────

/// What [`fetch_one`] did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Fetched {
    pub path: String,
    /// `fetched`, or `alreadyHere` (this Mac has the path; nothing was asked
    /// of the other Mac and nothing was written).
    pub state: String,
    pub bytes: u64,
    /// Bytes an earlier interrupted fetch had already brought (0 when none).
    pub resumed_from: u64,
    /// The other Mac's name, when it was asked.
    pub from: Option<String>,
}

/// Why a path is never fetched (it syncs through git, or stays put).
const NOT_FETCHABLE: &str = "only files the vault's git keeps out are fetched (media, binaries, \
files over 1 MB); Markdown, text, JSON, the personal folders, dot files and key files are not";

/// Whether this Mac already has something at vault-relative `rel` (any kind
/// of entry, a symlink included).
fn here(vault: &Path, rel: &str) -> bool {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    local_dir(vault, rel, false).is_ok_and(|d| fs::symlink_metadata(d.join(name)).is_ok())
}

/// Bring the peer's `rel` here when this Mac lacks it (N15: binaries stay
/// on the Mac that made them and come over only when asked). Never
/// overwrites: a file already here is left alone and the copy links in
/// without clobbering. Checks size and SHA-256, keeps the peer's mtime and
/// resumes an interrupted part. Only reads on the peer.
pub(crate) fn fetch_one(vault: &Path, peer: &mut dyn Peer, rel: &str) -> Result<Fetched, String> {
    let rel = rel.trim();
    if !safe_rel(rel) {
        return Err(format!("{rel:?} is not a vault path"));
    }
    if !copyable(rel, u64::MAX) {
        return Err(format!("{rel}: {NOT_FETCHABLE}"));
    }
    let mut done = Fetched {
        path: rel.to_string(),
        state: "alreadyHere".into(),
        bytes: 0,
        resumed_from: 0,
        from: None,
    };
    if here(vault, rel) {
        return Ok(done);
    }
    let (name, entry) = peer.stat(rel).map_err(|e| e.text().to_string())?;
    let Some(mut entry) = entry else {
        return Err(format!("{rel}: not on {name} (or {NOT_FETCHABLE})"));
    };
    let hash = peer
        .hashes(&[rel.to_string()])
        .map_err(|e| e.text().to_string())?
        .remove(rel)
        .ok_or_else(|| format!("{rel}: {name} could not read it"))?;
    entry.hash = Some(hash);
    let offset = pull(vault, peer, rel, rel, &entry, None).map_err(|e| e.text().to_string())?;
    done.state = "fetched".into();
    done.bytes = entry.size;
    done.resumed_from = offset;
    done.from = Some(name);
    Ok(done)
}

/// What [`peer_missing`] found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerFiles {
    pub peer_name: String,
    /// Files there that this Mac lacks, by path (at most `limit`).
    pub files: Vec<PeerFile>,
    /// How many matched in all.
    pub total: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PeerFile {
    pub path: String,
    pub size: u64,
    pub mtime: i64,
}

/// The peer's fetchable files under `prefix` (a vault-relative folder or
/// path start; empty for all) that this Mac lacks. Reads only, both sides.
pub(crate) fn peer_missing(
    vault: &Path,
    peer: &mut dyn Peer,
    prefix: &str,
    limit: usize,
) -> Result<PeerFiles, String> {
    let prefix = prefix.trim().trim_start_matches("./");
    if prefix.starts_with('/') || prefix.split('/').any(|p| p == "..") {
        return Err(format!("{prefix:?} is not a vault path"));
    }
    let (peer_name, listing) = peer.list().map_err(|e| e.text().to_string())?;
    let mut files = Vec::new();
    let mut total = 0;
    for (path, e) in listing {
        if !path.starts_with(prefix) || here(vault, &path) {
            continue;
        }
        total += 1;
        if files.len() < limit {
            files.push(PeerFile {
                path,
                size: e.size,
                mtime: e.mtime,
            });
        }
    }
    Ok(PeerFiles {
        peer_name,
        files,
        total,
    })
}

// ── The record and one cycle ────────────────────────────────────────────

/// `.hollow-files.json`: what each Mac has and what was last copied.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Record {
    pub version: u32,
    /// The peer this record is kept against (its ssh host).
    pub peer: Option<String>,
    pub peer_name: Option<String>,
    /// This Mac's copyable files (a hash cache too).
    pub here: Listing,
    /// The peer's.
    pub there: Listing,
    /// Path to the hash both sides held after the last copy.
    pub synced: BTreeMap<String, String>,
    /// Path to the (here, there) hashes already kept both.
    pub clashes: BTreeMap<String, [String; 2]>,
    /// A clash's copy name the peer still lacks, to the path here whose
    /// content it gets (a push after a kept-both pull that failed).
    pub pending: BTreeMap<String, String>,
    pub last_run_at: Option<String>,
    /// `idle`, `offline` or `error`.
    pub last_state: Option<String>,
    pub last_text: Option<String>,
    pub last_errors: Vec<String>,
}

/// What one cycle did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Report {
    pub offline: bool,
    /// Copied here (path as it landed).
    pub here: Vec<String>,
    /// Copied there.
    pub there: Vec<String>,
    /// Paths kept both.
    pub kept_both: Vec<String>,
    pub errors: Vec<String>,
    /// Transfers that resumed a part (path, bytes already there).
    pub resumed: Vec<(String, u64)>,
    /// Copies left for the next cycle by the byte cap.
    pub deferred: usize,
}

impl Report {
    // Read by the retired timer's status line; only tests read it now.
    #[cfg_attr(not(test), allow(dead_code))]
    fn text(&self) -> String {
        if self.offline {
            return "The other Mac didn't answer; skipped.".into();
        }
        let mut t = format!(
            "{} copied here, {} copied there",
            self.here.len(),
            self.there.len()
        );
        if !self.kept_both.is_empty() {
            t.push_str(&format!(", {} kept both", self.kept_both.len()));
        }
        if !self.errors.is_empty() {
            t.push_str(&format!(", {} failed", self.errors.len()));
        }
        if self.deferred > 0 {
            t.push_str(&format!(", {} next cycle", self.deferred));
        }
        t
    }
}

/// Fill each entry's hash from `cache` where size and mtime still match.
fn fill_from(cache: &Listing, list: &mut Listing) {
    for (p, e) in list.iter_mut() {
        if let Some(c) = cache.get(p) {
            if c.same_stat(e) {
                e.hash = c.hash.clone();
            }
        }
    }
}

/// One cycle against `peer` (see the module notes); updates `record`.
/// Copies stop once `byte_cap` bytes moved (the first always runs).
/// Retired with the timer (N15, 2026-11-13): nothing in the app calls it;
/// it stays, tested, until Alex confirms or drops the timed copy.
#[allow(dead_code)]
pub(crate) fn run_cycle(
    vault: &Path,
    record: &mut Record,
    peer: &mut dyn Peer,
    this_machine: &str,
    byte_cap: u64,
) -> Report {
    let mut report = Report::default();
    if record.peer.as_deref() != Some(peer.id().as_str()) {
        *record = Record {
            peer: Some(peer.id()),
            ..Record::default()
        };
    }
    record.version = 1;
    let mut local = list_local(vault);
    fill_from(&record.here, &mut local);
    let (peer_name, mut remote) = match peer.list() {
        Ok(v) => v,
        Err(e) => {
            report.offline = matches!(e, PeerError::Offline(_));
            if !report.offline {
                report.errors.push(e.text().to_string());
            }
            record.here = local;
            return report;
        }
    };
    record.peer_name = Some(peer_name.clone());
    fill_from(&record.there, &mut remote);
    let (want_here, want_there) = wanted_hashes(&local, &remote);
    for p in want_here {
        let h = hash_local(vault, &p);
        if let Some(e) = local.get_mut(&p) {
            e.hash = h;
        }
    }
    match peer.hashes(&want_there) {
        Ok(found) => {
            for (p, h) in found {
                if let Some(e) = remote.get_mut(&p) {
                    e.hash = Some(h);
                }
            }
        }
        Err(e) => {
            report.offline = matches!(e, PeerError::Offline(_));
            if !report.offline {
                report.errors.push(e.text().to_string());
            }
            record.here = local;
            record.there = remote;
            return report;
        }
    }
    let reserved: BTreeSet<String> = record.pending.keys().cloned().collect();
    let machines = (this_machine, peer_name.as_str());
    let mut actions = plan(
        &local,
        &remote,
        &record.synced,
        &record.clashes,
        &reserved,
        machines,
    );
    // A replace is never decided on a cached hash: hash the side it would
    // overwrite again now, and plan once more if that changed anything.
    let mut fresh_here = Vec::new();
    let mut fresh_there = Vec::new();
    for a in &actions {
        match a {
            Action::ReplaceHere(p) => fresh_here.push(p.clone()),
            Action::ReplacePeer(p) => fresh_there.push(p.clone()),
            _ => {}
        }
    }
    if !fresh_here.is_empty() || !fresh_there.is_empty() {
        let mut changed = false;
        for p in fresh_here {
            let h = hash_local(vault, &p);
            if let Some(e) = local.get_mut(&p) {
                changed |= e.hash != h;
                e.hash = h;
            }
        }
        match peer.hashes(&fresh_there) {
            Ok(found) => {
                for p in &fresh_there {
                    if let Some(e) = remote.get_mut(p) {
                        let h = found.get(p).cloned();
                        changed |= e.hash != h;
                        e.hash = h;
                    }
                }
            }
            Err(e) => {
                report.offline = matches!(e, PeerError::Offline(_));
                if !report.offline {
                    report.errors.push(e.text().to_string());
                }
                record.here = local;
                record.there = remote;
                return report;
            }
        }
        if changed {
            actions = plan(
                &local,
                &remote,
                &record.synced,
                &record.clashes,
                &reserved,
                machines,
            );
        }
    }
    let mut moved: u64 = 0;
    // Copy names a kept-both clash still owes the peer, first.
    for (there, src) in record.pending.clone() {
        let ours = record
            .clashes
            .get(&src)
            .is_some_and(|c| local.get(&src).and_then(|e| e.hash.as_ref()) == Some(&c[0]));
        if remote.contains_key(&there) || !ours {
            record.pending.remove(&there);
            continue;
        }
        let entry = local[&src].clone();
        if moved > 0 && moved + entry.size > byte_cap {
            report.deferred += 1;
            continue;
        }
        match push(vault, peer, &src, &there, &entry, None) {
            Ok(off) => {
                moved += entry.size - off;
                note(&mut report, &there, off);
                record.pending.remove(&there);
                if let Some(h) = entry.hash.clone() {
                    record.synced.insert(there.clone(), h);
                }
                remote.insert(there.clone(), entry);
                report.there.push(there);
            }
            Err(PeerError::Offline(_)) => {
                report.offline = true;
                record.here = local;
                record.there = remote;
                return report;
            }
            Err(e) => report.errors.push(e.text().to_string()),
        }
    }
    for action in actions {
        let size = transfer_size(&action, &local, &remote);
        if size > 0 && moved > 0 && moved + size > byte_cap {
            report.deferred += 1;
            continue;
        }
        let step = do_action(
            vault,
            peer,
            &action,
            &mut local,
            &mut remote,
            record,
            &mut report,
        );
        match step {
            Ok(n) => moved += n,
            Err(e) => {
                if matches!(e, PeerError::Offline(_)) {
                    report.offline = true;
                    break;
                }
                report.errors.push(e.text().to_string());
            }
        }
    }
    record.here = local;
    record.there = remote;
    report
}

/// Bytes `action` would copy at most.
fn transfer_size(action: &Action, local: &Listing, remote: &Listing) -> u64 {
    let size = |l: &Listing, p: &str| l.get(p).map_or(0, |e| e.size);
    match action {
        Action::Skip(..) => 0,
        Action::ToPeer(p) | Action::ReplacePeer(p) => size(local, p),
        Action::FromPeer(p) | Action::ReplaceHere(p) => size(remote, p),
        Action::KeepBoth { path, .. } => size(local, path) + size(remote, path),
    }
}

/// A transfer that resumed a part.
fn note(report: &mut Report, path: &str, offset: u64) {
    if offset > 0 {
        report.resumed.push((path.to_string(), offset));
    }
}

/// One action; the bytes it copied.
fn do_action(
    vault: &Path,
    peer: &mut dyn Peer,
    action: &Action,
    local: &mut Listing,
    remote: &mut Listing,
    record: &mut Record,
    report: &mut Report,
) -> Result<u64, PeerError> {
    // A replace overwrites only what both held at the last copy.
    let expect = |l: &Listing, p: &str, record: &Record| {
        l.get(p).cloned().map(|mut e| {
            e.hash = record.synced.get(p).cloned();
            e
        })
    };
    match action {
        Action::Skip(path, SkipWhy::Same) => {
            if let Some(h) = local
                .get(path)
                .and_then(|e| e.hash.clone())
                .or_else(|| remote.get(path).and_then(|e| e.hash.clone()))
            {
                record.synced.insert(path.clone(), h);
            }
            Ok(0)
        }
        Action::Skip(..) => Ok(0),
        Action::ToPeer(path) | Action::ReplacePeer(path) => {
            let entry = local[path].clone();
            let expect = matches!(action, Action::ReplacePeer(_))
                .then(|| expect(remote, path, record))
                .flatten();
            let off = push(vault, peer, path, path, &entry, expect.as_ref())?;
            note(report, path, off);
            remote.insert(path.clone(), entry.clone());
            if let Some(h) = entry.hash {
                record.synced.insert(path.clone(), h);
            }
            report.there.push(path.clone());
            Ok(entry.size - off)
        }
        Action::FromPeer(path) | Action::ReplaceHere(path) => {
            let entry = remote[path].clone();
            let expect = matches!(action, Action::ReplaceHere(_))
                .then(|| expect(local, path, record))
                .flatten();
            let off = pull(vault, peer, path, path, &entry, expect.as_ref())?;
            note(report, path, off);
            local.insert(path.clone(), entry.clone());
            if let Some(h) = entry.hash {
                record.synced.insert(path.clone(), h);
            }
            report.here.push(path.clone());
            Ok(entry.size - off)
        }
        Action::KeepBoth { path, here, there } => {
            let theirs = remote[path].clone();
            let ours = local[path].clone();
            let (Some(lh), Some(rh)) = (ours.hash.clone(), theirs.hash.clone()) else {
                return Ok(0);
            };
            let off = pull(vault, peer, path, here, &theirs, None)?;
            note(report, here, off);
            local.insert(here.clone(), theirs.clone());
            report.here.push(here.clone());
            report.kept_both.push(path.clone());
            // Recorded now, so a failed push below is retried as a plain
            // push of `there` and never plans the clash again.
            record.synced.insert(here.clone(), rh.clone());
            record.clashes.insert(path.clone(), [lh.clone(), rh]);
            record.pending.insert(there.clone(), path.clone());
            let mut moved = theirs.size - off;
            let off = push(vault, peer, path, there, &ours, None)?;
            note(report, there, off);
            moved += ours.size - off;
            record.pending.remove(there);
            remote.insert(there.clone(), ours);
            record.synced.insert(there.clone(), lh);
            report.there.push(there.clone());
            Ok(moved)
        }
    }
}

// ── Settings, status, the commands ──────────────────────────────────────

/// `file-copy.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct Settings {
    /// The retired timed copy's switch: read from old files, always saved off.
    pub enabled: bool,
    /// The peer's vault: home-relative or absolute; default [`default_remote_vault`].
    pub remote_vault: Option<String>,
}

fn app_dir() -> Result<PathBuf, String> {
    crate::agent_settings::app_data_dir()
}

/// A saved JSON file, or the default when missing or unreadable.
pub(crate) fn load_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_json<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let text = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    crate::vault::atomic_write(path, &text)
}

/// The peer's vault when none is saved: this vault's path below the home
/// folder (the same layout on both Macs), else none.
pub(crate) fn default_remote_vault(vault: &Path, home: Option<&Path>) -> Option<String> {
    let rel = vault.strip_prefix(home?).ok()?.to_str()?.to_string();
    (!rel.is_empty()).then_some(rel)
}

/// A remote vault path as saved: trimmed, no trailing slash (`/` stays).
pub(crate) fn normalize_remote_vault(path: &str) -> String {
    let t = path.trim();
    let n = t.trim_end_matches('/');
    if n.is_empty() && !t.is_empty() {
        "/".into()
    } else {
        n.to_string()
    }
}

/// A remote vault path is one line, not `/`, with no `..` or empty-dot part.
pub(crate) fn valid_remote_vault(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    !trimmed.is_empty()
        && !path.chars().any(|c| c.is_control())
        && trimmed
            .trim_start_matches('/')
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
}

/// What Settings and agents read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FileCopyStatus {
    /// Always `fetchOnDemand` (N15): nothing copies on a timer.
    pub mode: String,
    /// The runner named in `slot-remote`, the peer.
    pub peer: Option<String>,
    pub peer_name: Option<String>,
    pub remote_vault: Option<String>,
    /// `noPeer` (no runner or no vault path for it) or `ready`.
    pub state: String,
    pub text: String,
    /// The retired timed copy's last run, kept as history (its record is
    /// never deleted): when, and files it saw on each side.
    pub last_run_at: Option<String>,
    pub files_here: usize,
    pub files_there: usize,
    pub kept_both: usize,
    pub last_errors: Vec<String>,
}

pub(crate) fn status_from(
    record: &Record,
    runner: Option<String>,
    remote_vault: Option<String>,
) -> FileCopyStatus {
    let (state, text) = if runner.is_none() {
        (
            "noPeer",
            "No other Mac: name the runner in ~/.config/hollow/slot-remote.",
        )
    } else if remote_vault.is_none() {
        ("noPeer", "No vault path for the other Mac.")
    } else {
        (
            "ready",
            "Binaries stay on the Mac that made them; vault.fetch_peer_file brings one here when asked.",
        )
    };
    FileCopyStatus {
        mode: "fetchOnDemand".into(),
        peer: runner,
        peer_name: record.peer_name.clone(),
        remote_vault,
        state: state.to_string(),
        text: text.to_string(),
        last_run_at: record.last_run_at.clone(),
        files_here: record.here.len(),
        files_there: record.there.len(),
        kept_both: record.clashes.len(),
        last_errors: record.last_errors.clone(),
    }
}

fn remote_vault_for(vault: &Path, settings: &Settings) -> Option<String> {
    settings
        .remote_vault
        .as_deref()
        .map(normalize_remote_vault)
        .filter(|p| valid_remote_vault(p))
        .or_else(|| default_remote_vault(vault, dirs::home_dir().as_deref()))
}

fn status_now(vault: &Path) -> Result<FileCopyStatus, String> {
    let dir = app_dir()?;
    let settings = load_json::<Settings>(&dir.join(SETTINGS_FILE));
    let record = load_json::<Record>(&dir.join(RECORD_FILE));
    Ok(status_from(
        &record,
        crate::remote_session::configured_runner(),
        remote_vault_for(vault, &settings),
    ))
}

/// The other Mac as the fetch reaches it: the runner over ssh, at its vault.
fn peer_now(vault: &Path) -> Result<ShellPeer, String> {
    let settings = load_json::<Settings>(&app_dir()?.join(SETTINGS_FILE));
    let host = crate::remote_session::configured_runner()
        .ok_or("No other Mac: name the runner in ~/.config/hollow/slot-remote.")?;
    let root = remote_vault_for(vault, &settings).ok_or("No vault path for the other Mac.")?;
    Ok(ShellPeer::ssh(&host, &root))
}

/// One fetch at a time, so two asks for one path never share its part.
static FETCH: Mutex<()> = Mutex::new(());

/// How the binaries reach the other Mac (agents: `settings.file_copy_status`). Reads only.
#[tauri::command]
pub async fn file_copy_status(
    state: tauri::State<'_, crate::commands::AppState>,
) -> Result<FileCopyStatus, String> {
    file_copy_status_at(state.config.vault_path.clone()).await
}

/// [`file_copy_status`] for `vault` (the action's read).
pub(crate) async fn file_copy_status_at(vault: PathBuf) -> Result<FileCopyStatus, String> {
    crate::vault::spawn_blocking_logged(move || status_now(&vault))
        .await
        .map_err(|e| e.to_string())?
}

/// The other Mac's binaries this Mac lacks, under `prefix` (agents:
/// `vault.peer_files`). Reads only, on both Macs.
#[tauri::command]
pub async fn peer_files(
    state: tauri::State<'_, crate::commands::AppState>,
    prefix: Option<String>,
    limit: Option<usize>,
) -> Result<PeerFiles, String> {
    peer_files_at(state.config.vault_path.clone(), prefix, limit).await
}

pub(crate) async fn peer_files_at(
    vault: PathBuf,
    prefix: Option<String>,
    limit: Option<usize>,
) -> Result<PeerFiles, String> {
    crate::vault::spawn_blocking_logged(move || {
        let mut peer = peer_now(&vault)?;
        peer_missing(
            &vault,
            &mut peer,
            prefix.as_deref().unwrap_or(""),
            limit.unwrap_or(200).clamp(1, 1000),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Bring one binary from the other Mac (agents: `vault.fetch_peer_file`):
/// never overwrites, checked by SHA-256, resumable.
#[tauri::command]
pub async fn fetch_peer_file(
    state: tauri::State<'_, crate::commands::AppState>,
    path: String,
) -> Result<Fetched, String> {
    fetch_peer_file_at(state.config.vault_path.clone(), path).await
}

pub(crate) async fn fetch_peer_file_at(vault: PathBuf, path: String) -> Result<Fetched, String> {
    crate::vault::spawn_blocking_logged(move || {
        let _one = match FETCH.try_lock() {
            Ok(held) => held,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(
                    "Another fetch from the other Mac is running; try again when it ends.".into(),
                )
            }
        };
        let mut peer = peer_now(&vault)?;
        fetch_one(&vault, &mut peer, &path)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Set the other Mac's vault path (the person's choice). The timed two-way
/// copy is retired (N15, 2026-11-13): `enabled: true` is refused, and any
/// save writes it off. No file is touched.
#[tauri::command]
pub async fn configure_file_copy(
    state: tauri::State<'_, crate::commands::AppState>,
    enabled: Option<bool>,
    remote_vault: Option<String>,
) -> Result<FileCopyStatus, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let path = app_dir()?.join(SETTINGS_FILE);
        let mut settings = load_json::<Settings>(&path);
        apply_switch(&mut settings, enabled, remote_vault)?;
        save_json(&path, &settings)?;
        status_now(&vault)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// What `configure_file_copy` saves: `enabled: true` is refused, the
/// switch is always left off, and `remote_vault` (empty clears it) is checked.
pub(crate) fn apply_switch(
    settings: &mut Settings,
    enabled: Option<bool>,
    remote_vault: Option<String>,
) -> Result<(), String> {
    if enabled == Some(true) {
        return Err(RETIRED.to_string());
    }
    if let Some(rv) = remote_vault.map(|s| normalize_remote_vault(&s)) {
        if rv.is_empty() {
            settings.remote_vault = None;
        } else if valid_remote_vault(&rv) {
            settings.remote_vault = Some(rv);
        } else {
            return Err(format!("{rv:?} is not a vault path for the other Mac."));
        }
    }
    settings.enabled = false;
    Ok(())
}

/// Why the timed copy can no longer be turned on.
pub(crate) const RETIRED: &str = "The timed copy between the Macs is retired (N15): binaries stay \
on the Mac that made them, and vault.fetch_peer_file brings one here when asked.";

#[cfg(test)]
#[path = "sync_files_tests.rs"]
mod tests;
