//! Owner-only runners (remote runners item 2, 2026-11-03). A remote runner
//! belongs to one person. Hollow ships no host or account name: a runner
//! exists only once this Mac names one in `~/.config/hollow/slot-remote`
//! (Settings, Devices, Remote runner, [`set_runner`]), and
//! `scripts/check-no-host-names.test.mjs` keeps such names out of shipped
//! code.
//!
//! The runner records its owner in `~/hollow-remote/owner.json` there
//! ([`record_owner`]): the SHA256 fingerprint of the SSH key this Mac
//! connects with, this Hollow's install id ([`install_id`]) and when. The
//! runner status check (`remote_status`) reads that record and the
//! fingerprints of every key the runner account lets in
//! (`~/.ssh/authorized_keys` and `authorized_keys2`), and [`judge`] turns
//! them into warnings: a key that is neither the owner's nor this Mac's, an
//! owner record another Hollow wrote, entries that aren't readable keys.
//! `hollow-slot --remote` warns about extra keys from its capability line
//! (`keys=`, `others=`, `owner=`; `build_slots::SLOT_TOOL`'s `r_prep`).
//!
//! Recording and naming the runner are the person's clicks in Settings
//! (Record me as owner, Set up, Turn off), or the COO's `build.runner_set` and
//! `build.runner_record_owner` on their cards (COO parity L13). Recording
//! writes that one file there and nothing else: never sshd, the firewall or
//! accounts.

use serde::Serialize;
use serde_json::Value;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The owner record's path under the runner account's home.
pub(crate) const OWNER_FILE: &str = "hollow-remote/owner.json";
/// How long recording the owner may take (ssh's own connect timeout is 10 s).
const RECORD_TIMEOUT: Duration = Duration::from_secs(30);

/// This Hollow's install id: 32 lowercase hex characters in app data
/// `install-id`, made on first use. It names this Hollow in a runner's
/// owner record; it is no secret and grants nothing.
pub(crate) fn install_id() -> Result<String, String> {
    let path = crate::agent_settings::app_data_dir()?.join("install-id");
    if let Some(id) = std::fs::read_to_string(&path)
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| valid_install_id(t))
    {
        return Ok(id);
    }
    let bytes: [u8; 16] = rand::random();
    let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    }
    crate::vault::atomic_write(&path, format!("{id}\n").as_bytes())?;
    Ok(id)
}

fn valid_install_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A `SHA256:<base64>` key fingerprint, as `ssh-keygen -l -E sha256` prints it.
pub(crate) fn valid_fingerprint(fp: &str) -> bool {
    fp.strip_prefix("SHA256:").is_some_and(|b| {
        (40..=50).contains(&b.len())
            && b.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'+' || c == b'/' || c == b'=')
    })
}

/// `~/.config/hollow/slot-remote` (`XDG_CONFIG_HOME` when set), the file
/// `hollow-slot --remote` and remote sessions read their runner from.
pub(crate) fn slot_remote_path() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .map(|base| base.join("hollow").join("slot-remote"))
}

/// Name `host` as this Mac's runner (`None`: turn it off), writing
/// `slot-remote`; returns the runner now in use. Turning off keeps the old
/// host as a comment. Refused while `HOLLOW_SLOT_REMOTE` is set, which wins
/// over the file.
pub(crate) fn set_runner(host: Option<&str>) -> Result<Option<String>, String> {
    if std::env::var_os("HOLLOW_SLOT_REMOTE").is_some() {
        return Err("HOLLOW_SLOT_REMOTE is set in Hollow's environment, and it wins over ~/.config/hollow/slot-remote. Unset it and restart Hollow to choose the runner here.".into());
    }
    let path = slot_remote_path().ok_or("Couldn't find the home folder.")?;
    set_runner_at(&path, host, crate::remote_session::configured_runner())?;
    let now = crate::remote_session::configured_runner();
    if now.is_some() {
        // F3: a runner is another Mac to offer "Connect this Mac's context" for.
        crate::mac_context::after_change();
    }
    Ok(now)
}

/// [`set_runner`]'s file write: `host` on its own line under a header, or
/// `off` with the runner it replaces (`was`) as a comment.
pub(crate) fn set_runner_at(
    path: &Path,
    host: Option<&str>,
    was: Option<String>,
) -> Result<(), String> {
    let header = "# Hollow's remote runner: the SSH host on the first line that isn't a comment\n# (Settings, Devices, Remote runner). off means none.\n";
    let text = match host.map(str::trim) {
        Some(h)
            if crate::remote_session::valid_host(h) && !matches!(h, "off" | "local" | "none") =>
        {
            format!("{header}{h}\n")
        }
        Some(h) => return Err(format!("{h} is not an SSH host name.")),
        None => {
            format!(
                "{header}off\n{}",
                was.map(|w| format!("# was {w}\n")).unwrap_or_default()
            )
        }
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("couldn't make {}: {e}", dir.display()))?;
    }
    crate::vault::atomic_write(path, text.as_bytes())
}

/// What this Hollow is to a runner: its install id (`None` when it can't
/// be read) and the fingerprints of every key this Mac may offer it.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Mine {
    pub install_id: Option<String>,
    pub keys: Vec<String>,
}

impl Mine {
    /// This Hollow as `host` sees it, through `ssh -G` (`ssh` is the ssh
    /// binary to ask; tests pass a stand-in).
    pub(crate) fn for_host(ssh: &str, host: &str) -> Mine {
        Mine {
            install_id: install_id().ok(),
            keys: key_fingerprints(ssh, host),
        }
    }
}

/// The SHA256 fingerprints of every key this Mac may offer `host`, in
/// ssh's order: each `IdentityFile` in `ssh -G host` (the `.pub` beside it,
/// or the file itself when it is a `.pub`, as a key held by an agent such
/// as 1Password is named). Empty when none can be read (a certificate
/// setup, a key with no public half here).
pub(crate) fn key_fingerprints(ssh: &str, host: &str) -> Vec<String> {
    if !crate::remote_session::valid_host(host) {
        return Vec::new();
    }
    let Ok(out) = std::process::Command::new(ssh)
        .args(["-G", host])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let home = dirs::home_dir();
    let mut found: Vec<String> = Vec::new();
    for file in String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.strip_prefix("identityfile "))
        .filter(|f| !f.contains('%'))
    {
        let path = match (file.strip_prefix("~/"), &home) {
            (Some(rest), Some(h)) => h.join(rest),
            _ => PathBuf::from(file),
        };
        let public = if file.ends_with(".pub") {
            path
        } else {
            let mut p = path.into_os_string();
            p.push(".pub");
            PathBuf::from(p)
        };
        if let Some(fp) = pub_fingerprint(&public) {
            if !found.contains(&fp) {
                found.push(fp);
            }
        }
    }
    found
}

/// `ssh-keygen -l -E sha256 -f <pub>`'s fingerprint, when it is one.
fn pub_fingerprint(public: &Path) -> Option<String> {
    if !public.is_file() {
        return None;
    }
    let out = std::process::Command::new("ssh-keygen")
        .args(["-l", "-E", "sha256", "-f"])
        .arg(public)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let fp = text.split_whitespace().nth(1)?.to_string();
    valid_fingerprint(&fp).then_some(fp)
}

/// What [`record_owner`] wrote there.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OwnerRecord {
    pub v: u32,
    pub install_id: String,
    pub key: String,
    pub at: String,
    pub by: String,
}

/// `ssh <SSH_OPTS> host command`, with `input` on its stdin, killed after
/// [`RECORD_TIMEOUT`]: (exit code, stdout, stderr), both read as they come
/// so a chatty login can't fill a pipe and stall it.
fn ssh_bounded(
    ssh: &str,
    host: &str,
    command: &str,
    input: &[u8],
) -> Result<(Option<i32>, String, String), String> {
    use std::io::Read;
    let mut child = std::process::Command::new(ssh)
        .args(crate::remote_session::SSH_OPTS)
        .args([host, command])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("ssh couldn't start ({e})."))?;
    let reader = |pipe: Option<Box<dyn Read + Send>>| {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.by_ref().take(1 << 20).read_to_end(&mut bytes);
            }
            let _ = tx.send(bytes);
        });
        rx
    };
    let out = reader(
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    let err = reader(
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn Read + Send>),
    );
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < RECORD_TIMEOUT => {
                std::thread::sleep(Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{host} didn't finish in {} s; nothing may have been recorded.",
                    RECORD_TIMEOUT.as_secs()
                ));
            }
        }
    };
    let grace = Duration::from_secs(2);
    let text = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        String::from_utf8_lossy(&rx.recv_timeout(grace).unwrap_or_default()).into_owned()
    };
    Ok((status.code(), text(out), text(err)))
}

/// What a failed [`ssh_bounded`] call means, in words.
fn ssh_failure(host: &str, code: Option<i32>, stderr: &str, doing: &str) -> String {
    let last = stderr.lines().last().unwrap_or("").trim().to_string();
    match code {
        Some(255) => format!(
            "{host} didn't answer{}. {}",
            if last.is_empty() {
                String::new()
            } else {
                format!(": {last}")
            },
            crate::remote_status::likely_causes(&last)
        ),
        _ => format!(
            "{host} couldn't {doing}{}.",
            if last.is_empty() {
                String::new()
            } else {
                format!(" ({last})")
            }
        ),
    }
}

/// Wraps a remote `/bin/sh` script so exit 255 means only ssh's own failure.
fn remote_sh(script: &str) -> String {
    format!(
        "/bin/sh -c {}; s=$?; [ \"$s\" -ne 255 ] || s=3; exit $s",
        crate::remote_session::shell_quote(script)
    )
}

/// The fingerprints the runner account's authorized_keys files hold there
/// (`/usr/bin/ssh-keygen` first), read before recording so the record
/// names the key that actually opens it.
const AUTHORIZED_FPS_SH: &str = "kg=/usr/bin/ssh-keygen; [ -x \"$kg\" ] || kg=ssh-keygen; for f in \"$HOME/.ssh/authorized_keys\" \"$HOME/.ssh/authorized_keys2\"; do [ -f \"$f\" ] && \"$kg\" -l -E sha256 -f \"$f\" 2>/dev/null; done; exit 0";

/// Record this Hollow as `host`'s owner: `~/hollow-remote/owner.json` there
/// (0600, replaced whole), through `ssh` (a stand-in in tests). The key it
/// names is the first one this Mac may offer that the runner account's
/// authorized_keys holds (else the first one this Mac offers). The person's
/// own click; it changes nothing else there.
pub(crate) fn record_owner(ssh: &str, host: &str) -> Result<OwnerRecord, String> {
    if !crate::remote_session::valid_host(host) {
        return Err(format!("{host} is not an SSH host name."));
    }
    let candidates = key_fingerprints(ssh, host);
    if candidates.is_empty() {
        return Err(format!("Couldn't tell which key this Mac uses for {host}: ssh -G {host} names no IdentityFile with a public key file here. Add an IdentityFile line for it in ~/.ssh/config."));
    }
    let (code, listed, err) = ssh_bounded(ssh, host, &remote_sh(AUTHORIZED_FPS_SH), b"")?;
    if code != Some(0) {
        return Err(ssh_failure(host, code, &err, "read its authorized_keys"));
    }
    let authorized: Vec<&str> = listed
        .lines()
        .filter_map(|l| l.split_whitespace().nth(1))
        .collect();
    let key = candidates
        .iter()
        .find(|k| authorized.contains(&k.as_str()))
        .unwrap_or(&candidates[0])
        .clone();
    let record = OwnerRecord {
        v: 1,
        install_id: install_id()?,
        key,
        at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        by: format!("Hollow {}", env!("CARGO_PKG_VERSION")),
    };
    let body = serde_json::to_string(&record).map_err(|e| e.to_string())? + "\n";
    let dir = OWNER_FILE
        .rsplit_once('/')
        .map(|(d, _)| d)
        .unwrap_or("hollow-remote");
    let write = format!(
        "umask 077; d=\"$HOME/{dir}\"; t=\"$d/.owner.json.$$\"; mkdir -p \"$d\" || exit 1; if cat > \"$t\" && mv -f \"$t\" \"$HOME/{OWNER_FILE}\"; then exit 0; fi; rm -f \"$t\"; exit 1"
    );
    let (code, _, err) = ssh_bounded(ssh, host, &remote_sh(&write), body.as_bytes())?;
    if code == Some(0) {
        Ok(record)
    } else {
        Err(ssh_failure(
            host,
            code,
            &err,
            &format!("write ~/{OWNER_FILE}"),
        ))
    }
}

/// Who a runner belongs to, as its status check found it (Settings shows
/// it under Remote runner; agents read it with `build.runner_status`).
#[derive(Debug, Clone, Serialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunnerOwner {
    /// `you` (this Hollow, or another Hollow on this Mac using the same
    /// key, recorded it), `other` (another Hollow did) or `none` (no
    /// readable record there).
    pub state: &'static str,
    /// When the owner was recorded (the record's own `at`).
    pub recorded_at: Option<String>,
    /// SSH keys its runner account lets in (each counted once); `None` when
    /// they couldn't be read or none were found.
    pub keys: Option<u64>,
    /// Of those, how many are neither the owner's nor this Mac's; `None`
    /// when that can't be told.
    pub other_keys: Option<u64>,
    /// This Hollow is the owner, but the record names a key this Mac no
    /// longer offers (record again).
    pub key_changed: bool,
    /// What to look at, worst first.
    pub warnings: Vec<String>,
}

/// How many keys are listed at most (the probe sends the first 50).
const KEYS_LISTED: usize = 50;

/// The status probe's answer (`owner`: the record's installId, key and at;
/// `keys`: unique fingerprints, at most 50; `keyCount`: how many unique;
/// `keyLines`: key lines in the files; `keysRead`: fingerprints read,
/// repeats included), judged against this Hollow (`mine`). Fields of the
/// wrong type count as absent.
pub(crate) fn judge(probe: &Value, mine: &Mine) -> RunnerOwner {
    let owner = probe.get("owner");
    let field = |k: &str| {
        owner
            .and_then(|o| o.get(k))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
    };
    let count = |k: &str| probe.get(k).and_then(Value::as_u64);
    let owner_id = field("installId").filter(|id| valid_install_id(id));
    let owner_key = field("key").filter(|k| valid_fingerprint(k));
    let key_is_mine = owner_key.is_some_and(|k| mine.keys.iter().any(|m| m == k));
    let mut out = RunnerOwner {
        state: match (owner_id, mine.install_id.as_deref()) {
            (None, _) => "none",
            (Some(theirs), Some(ours)) if theirs == ours => "you",
            // Another instance of Hollow on this Mac (a candidate or review
            // build keeps its own app data) connects with the same key.
            _ if key_is_mine => "you",
            _ => "other",
        },
        recorded_at: owner_id
            .and(field("at"))
            .map(|at| at.chars().take(40).collect()),
        ..RunnerOwner::default()
    };
    if out.state == "other" {
        out.warnings.push(format!(
            "Another Hollow recorded itself as this runner's owner{}. A runner should serve one person: if that wasn't you from another Mac, stop using it here and check who can log in there.",
            out.recorded_at
                .as_deref()
                .map(|at| format!(" ({})", at.get(..10).unwrap_or(at)))
                .unwrap_or_default()
        ));
    }
    if out.state == "you" && owner_key.is_some() && !mine.keys.is_empty() && !key_is_mine {
        out.key_changed = true;
        out.warnings.push("The owner record names an SSH key this Mac no longer offers. Record yourself as owner again.".into());
    }
    let list: Option<Vec<&str>> = probe.get("keys").and_then(Value::as_array).map(|a| {
        let mut seen: Vec<&str> = Vec::new();
        for k in a
            .iter()
            .filter_map(Value::as_str)
            .filter(|k| valid_fingerprint(k))
        {
            if !seen.contains(&k) {
                seen.push(k);
            }
        }
        seen
    });
    let Some(list) = list else {
        return out;
    };
    let total = count("keyCount")
        .unwrap_or(list.len() as u64)
        .max(list.len() as u64);
    if total == 0 {
        out.warnings.push("No key could be read from its ~/.ssh/authorized_keys (sshd may keep keys elsewhere there, or the file isn't readable), so other keys can't be counted.".into());
        return out;
    }
    out.keys = Some(total);
    let allowed: Vec<&str> = owner_key
        .into_iter()
        .chain(mine.keys.iter().map(String::as_str))
        .collect();
    if !allowed.is_empty() {
        let listed_others = list.iter().filter(|k| !allowed.contains(k)).count() as u64;
        // Keys past the first 50 weren't listed: none of them is told apart.
        let unlisted = if list.len() >= KEYS_LISTED {
            total - list.len() as u64
        } else {
            0
        };
        let others = listed_others + unlisted;
        out.other_keys = Some(others);
        if others > 0 {
            out.warnings.insert(
                0,
                format!(
                    "{others} other SSH key{} can log in as its runner account (~/.ssh/authorized_keys there): whoever holds {} can run as that account and read what runs there, your turns' files and credentials included. Remove any you don't recognise.",
                    if others == 1 { "" } else { "s" },
                    if others == 1 { "it" } else { "one" }
                ),
            );
        }
    }
    if let (Some(lines), Some(read)) = (count("keyLines"), count("keysRead")) {
        if lines > read {
            let n = lines - read;
            out.warnings.push(format!(
                "{n} line{} of its authorized_keys couldn't be read as a key; check {} there.",
                if n == 1 { "" } else { "s" },
                if n == 1 { "it" } else { "them" }
            ));
        }
    }
    out
}

#[cfg(test)]
#[path = "runner_owner_tests.rs"]
mod tests;
