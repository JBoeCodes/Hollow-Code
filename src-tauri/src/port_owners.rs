//! Listening ports attributed to worktrees (OC14, from orca's `ports/`).
//!
//! `list_services` knows only what Hollow started. A dev server a stopped
//! turn left behind (a `vite` that escaped the sweep and still holds a port)
//! is invisible there. [`report`] lists this user's listening TCP ports
//! (`lsof -nP -iTCP -sTCP:LISTEN -F...`), reads each process's working
//! folder (`lsof -a -p <pids> -d cwd -Fpn`) and attributes it to a build
//! worktree when that folder is inside one. It changes nothing.
//!
//! What it claims, and what it refuses to guess ([`Owner`]):
//! - `hollow-service`: its process group is one `start_service` made (stop it
//!   with `stop_service`);
//! - `live-turn`: a live scoped run of THIS Hollow is in that workspace
//!   (scopes are vault-relative, `workspaces/<id>`, so the match is by the
//!   bound workspace's id, never by the absolute worktree path);
//! - `orphan`: the worktree is bound to a build workspace, this Hollow has
//!   no live scoped run in it, no other Hollow holds the vault's full-vault
//!   lease, and the process's parent is launchd (pid 1). That is evidence
//!   the starter is gone, not proof: another Hollow instance's scoped turns
//!   need no lease and are NOT visible here, so a listed orphan is a
//!   candidate a person confirms;
//! - `unknown-owner`: anything else inside a worktree: a folder under the
//!   worktrees root that no workspace is bound to, a process whose parent is
//!   alive (it may be your own terminal), or another Hollow holding the
//!   full-vault lease. Never reported as orphan.
//!
//! Processes whose folder is in no worktree (or can't be read) are only
//! counted, never described.
//!
//! **Stopping.** [`check_stop_target`] and [`stop_orphan`] signal one PID
//! (SIGTERM, never a group, never by name) and only a PID this module
//! currently reports as `orphan` on that port, re-read at the moment of the
//! call, and [`stop_orphan`] checks the process again right before the
//! signal (parent, working folder and start time), so a reused PID is
//! refused. They are NOT wired to any action: an
//! agent-callable stop needs its own confirm-tier action id (and a row in
//! `docs/HOLLOW_APP_COMMAND_MAP.tsv`), which this change does not add.

use crate::services::ServiceView;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const LSOF_LIMIT: Duration = Duration::from_secs(8);

/// One listening socket of one process, from lsof's field output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listener {
    pub pid: i32,
    pub pgid: Option<i32>,
    pub ppid: Option<i32>,
    pub command: String,
    pub address: String,
    pub port: u16,
}

/// Parse `lsof -nP -iTCP -sTCP:LISTEN -FpgRcn` output. Each process set
/// starts with `p<pid>` and may carry `g` (process group), `R` (parent) and
/// `c` (command); each socket is an `n<address>:<port>` line. One entry per
/// (pid, port): a process listening on `*:80` and `[::]:80` is one listener.
/// Lines that are not a plain `address:port` (established sockets have
/// `->`) are ignored.
pub(crate) fn parse_listeners(text: &str) -> Vec<Listener> {
    let mut out: Vec<Listener> = Vec::new();
    let mut seen: HashSet<(i32, u16)> = HashSet::new();
    let mut pids: HashSet<i32> = HashSet::new();
    let mut current: Option<Listener> = None;
    for line in text.lines() {
        let Some(tag) = line.chars().next() else {
            continue;
        };
        let value = &line[tag.len_utf8()..];
        match tag {
            'p' => {
                // One process set per pid: a repeated one is ignored (first wins).
                current = value
                    .trim()
                    .parse::<i32>()
                    .ok()
                    .filter(|pid| pids.insert(*pid))
                    .map(|pid| Listener {
                        pid,
                        pgid: None,
                        ppid: None,
                        command: String::new(),
                        address: String::new(),
                        port: 0,
                    });
            }
            'g' => {
                if let Some(c) = current.as_mut() {
                    c.pgid = value.trim().parse().ok();
                }
            }
            'R' => {
                if let Some(c) = current.as_mut() {
                    c.ppid = value.trim().parse().ok();
                }
            }
            'c' => {
                if let Some(c) = current.as_mut() {
                    c.command = crate::worktree_hygiene::lsof_unescape(value);
                }
            }
            'n' => {
                let Some(c) = current.as_ref() else { continue };
                if value.contains("->") {
                    continue;
                }
                let Some((address, port)) = value.rsplit_once(':') else {
                    continue;
                };
                let Ok(port) = port.parse::<u16>() else {
                    continue;
                };
                if !seen.insert((c.pid, port)) {
                    continue;
                }
                out.push(Listener {
                    address: address.to_string(),
                    port,
                    ..c.clone()
                });
            }
            _ => {}
        }
    }
    out
}

/// Parse `lsof -a -p <pids> -d cwd -Fpn` output: pid to working folder.
pub(crate) fn parse_cwds(text: &str) -> HashMap<i32, String> {
    let mut out = HashMap::new();
    let mut pid: Option<i32> = None;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('p') {
            pid = rest.trim().parse().ok();
        } else if let (Some(rest), Some(pid)) = (line.strip_prefix('n'), pid) {
            out.entry(pid)
                .or_insert_with(|| crate::worktree_hygiene::lsof_unescape(rest));
        }
    }
    out
}

/// A worktree a listener can be attributed to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct WorktreeRef {
    /// The build workspace bound to it; `None` for a folder under the
    /// worktrees root that no workspace is bound to.
    pub workspace_id: Option<String>,
    /// Other workspaces bound to the very same path (a removed workspace's
    /// leaf name can be reused), so a live turn in any of them counts.
    pub also_ids: Vec<String>,
    pub path: PathBuf,
    pub branch: Option<String>,
    /// The folder is still there (false after "Remove worktree").
    pub exists: bool,
}

/// A working folder deeper than this many components is not attributed: the
/// walk stays bounded whatever a process reports.
const MAX_CWD_DEPTH: usize = 64;

/// What this Hollow can say about who owns a listening process.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    HollowService,
    LiveTurn,
    Orphan,
    #[serde(rename = "unknown-owner")]
    Unknown,
}

/// What the attribution read, separate from the processes: everything
/// [`attribute`] needs, so tests plant it without a running Hollow.
#[derive(Clone, Debug, Default)]
pub(crate) struct Facts {
    /// Ids of the workspaces this Hollow has a live scoped run in (the
    /// vault-relative scope `workspaces/<id>`, prefix stripped).
    pub live_workspaces: HashSet<String>,
    /// Pids of the services Hollow started (each its own process group).
    pub service_pids: Vec<i32>,
    /// ANY live run holds the vault's full-vault lease, this Hollow's own
    /// (a chat turn, a project chat, an overnight run: it may have started a
    /// server from a subfolder) or another's. Scoped runs of another
    /// instance take no lease and are not detected.
    pub lease_held: bool,
}

/// One attributed listener.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortEntry {
    pub port: u16,
    pub address: String,
    pub pid: i32,
    pub ppid: Option<i32>,
    pub command: String,
    pub cwd: String,
    pub workspace_id: Option<String>,
    pub worktree: String,
    pub branch: Option<String>,
    pub owner: Owner,
    pub reason: String,
    /// The process's start time when listed. A stop card carries it with the
    /// pid, and [`stop_orphan`] refuses when the process started at another time.
    pub start_micros: Option<u64>,
}

/// The listing: what is attributed, and how many listeners were not.
#[derive(Clone, Debug, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PortReport {
    pub listeners: Vec<PortEntry>,
    /// Listening sockets whose process is in no worktree, or whose working
    /// folder couldn't be read. Not described.
    pub unattributed: usize,
    /// Why the scan could not run (lsof missing or timed out), else absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unavailable: Option<String>,
}

/// The worktree `cwd` is inside: the longest bound or existing worktree
/// that is a path-component prefix of it (so `/w/wt-10` never matches
/// `/w/wt`), else, for a folder under `root` (`<root>/<repo>/<name>/...`),
/// that unbound worktree.
fn worktree_of(cwd: &Path, worktrees: &[WorktreeRef], root: Option<&Path>) -> Option<WorktreeRef> {
    if cwd.components().count() > MAX_CWD_DEPTH {
        return None;
    }
    let candidates: Vec<&WorktreeRef> = worktrees
        .iter()
        // A recorded path that is relative, empty, `/` or only a top folder
        // would claim every process under it: never attribute through one.
        .filter(|w| w.path.is_absolute() && w.path.components().count() >= 3)
        .filter(|w| cwd.starts_with(&w.path))
        .collect();
    if let Some(depth) = candidates.iter().map(|w| w.path.components().count()).max() {
        // Bindings at the longest path: prefer the one whose folder exists
        // (the live binding), and carry every other id so none is lost.
        let candidates_all = candidates.clone();
        let same: Vec<&WorktreeRef> = candidates
            .into_iter()
            .filter(|w| w.path.components().count() == depth)
            .collect();
        let mut primary = (*same.iter().find(|w| w.exists).unwrap_or(&same[0])).clone();
        // Every ENCLOSING binding's ids too (a live turn in an outer
        // workspace may have started a server inside a nested one).
        let mut ids: Vec<String> = Vec::new();
        for w in &candidates_all {
            ids.extend(w.workspace_id.iter().cloned());
            ids.extend(w.also_ids.iter().cloned());
        }
        ids.retain(|id| Some(id) != primary.workspace_id.as_ref());
        ids.sort();
        ids.dedup();
        primary.also_ids = ids;
        return Some(primary);
    }
    let rest = cwd.strip_prefix(root?).ok()?;
    let mut parts = rest.components();
    let (repo, name) = (parts.next()?, parts.next()?);
    Some(WorktreeRef {
        workspace_id: None,
        also_ids: Vec::new(),
        path: root?.join(repo).join(name),
        branch: None,
        exists: true,
    })
}

/// Attribute `listeners` (with their working folders) to worktrees and say
/// who owns each. Pure: no process, file or registry is read here.
pub(crate) fn attribute(
    listeners: &[Listener],
    cwds: &HashMap<i32, String>,
    worktrees: &[WorktreeRef],
    root: Option<&Path>,
    facts: &Facts,
) -> PortReport {
    let mut report = PortReport::default();
    for l in listeners {
        let Some(cwd) = cwds.get(&l.pid) else {
            report.unattributed += 1;
            continue;
        };
        let Some(w) = worktree_of(Path::new(cwd), worktrees, root) else {
            report.unattributed += 1;
            continue;
        };
        let (owner, reason) = classify(l, &w, facts);
        report.listeners.push(PortEntry {
            port: l.port,
            address: l.address.clone(),
            pid: l.pid,
            ppid: l.ppid,
            command: l.command.clone(),
            cwd: cwd.clone(),
            workspace_id: w.workspace_id.clone(),
            worktree: w.path.to_string_lossy().into_owned(),
            branch: w.branch.clone(),
            owner,
            reason,
            start_micros: None,
        });
    }
    report.listeners.sort_by_key(|e| (e.port, e.pid));
    report
}

fn classify(l: &Listener, w: &WorktreeRef, facts: &Facts) -> (Owner, String) {
    let managed = facts
        .service_pids
        .iter()
        .any(|&s| s == l.pid || Some(s) == l.pgid);
    if managed {
        return (
            Owner::HollowService,
            "Hollow started it as a service; stop_service stops it.".into(),
        );
    }
    if w.workspace_id.is_none() {
        return (
            Owner::Unknown,
            "Inside a worktrees folder no build workspace is bound to; Hollow can't see who owns it."
                .into(),
        );
    }
    if facts.lease_held {
        return (
            Owner::Unknown,
            "A live full-vault run (this Hollow's own, or another Hollow's) holds the vault lease, so it may have started this; Hollow can't tell."
                .into(),
        );
    }
    if w.workspace_id
        .iter()
        .chain(w.also_ids.iter())
        .any(|id| facts.live_workspaces.contains(id))
    {
        return (
            Owner::LiveTurn,
            "A turn of this Hollow is running in this workspace now.".into(),
        );
    }
    match l.ppid {
        Some(1) => {
            let removed = if w.exists {
                ""
            } else {
                " The worktree itself was removed."
            };
            (
                Owner::Orphan,
                format!("No turn of this Hollow is running in this workspace and the process's parent is launchd (pid 1).{removed}"),
            )
        }
        Some(parent) => (
            Owner::Unknown,
            format!(
                "No turn is running in this worktree, but its parent process {parent} is alive; it may be a terminal you started."
            ),
        ),
        None => (
            Owner::Unknown,
            "No turn is running in this worktree, but its parent process couldn't be read.".into(),
        ),
    }
}

/// Did lsof answer? Exit 0 yes; exit 1 is "nothing matched" only with no
/// error message; anything else, or a wait error, is a failure.
fn lsof_succeeded(code: Option<i32>, stdout_empty: bool, stderr: &str, wait_error: bool) -> bool {
    if wait_error {
        return false;
    }
    match code {
        Some(0) => true,
        Some(1) => !stdout_empty || stderr.trim().is_empty(),
        _ => false,
    }
}

/// The same folder spelled differently (trailing slash, `/tmp` vs
/// `/private/tmp`): the deepest existing ancestor canonicalised, the rest kept.
fn same_path(a: &str, b: &str) -> bool {
    fn norm(s: &str) -> PathBuf {
        let p = Path::new(s);
        let mut rest = Vec::new();
        let mut cur = p;
        loop {
            if let Ok(c) = cur.canonicalize() {
                let mut out = c;
                for part in rest.iter().rev() {
                    out.push(part);
                }
                return out;
            }
            match (cur.parent(), cur.file_name()) {
                (Some(parent), Some(name)) => {
                    rest.push(name.to_os_string());
                    cur = parent;
                }
                _ => return p.components().collect(),
            }
        }
    }
    norm(a) == norm(b)
}

async fn run_lsof(args: &[&str]) -> Option<String> {
    let mut cmd = tokio::process::Command::new("lsof");
    cmd.args(args).env("LC_ALL", "en_US.UTF-8");
    let out =
        crate::process_group::run_to_completion_capped(&mut cmd, None, LSOF_LIMIT, None, 8 << 20)
            .await
            .ok()?;
    // A cut-off listing would read as "fewer listeners": unavailable instead.
    if out.timed_out || out.stdout_truncated {
        return None;
    }
    // A failed lsof must read as unknown, never as "no ports". It exits 1
    // when nothing matches (no rows, no message); above 1, no exit status, a
    // wait error, or exit 1 with only an error message are failures.
    if !lsof_succeeded(
        out.status.and_then(|s| s.code()),
        out.stdout.is_empty(),
        &out.stderr,
        out.wait_error.is_some(),
    ) {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Every listening TCP socket of this user's processes.
async fn scan_listeners() -> Option<Vec<Listener>> {
    let text = run_lsof(&["-nP", "-w", "-iTCP", "-sTCP:LISTEN", "-FpgRcn"]).await?;
    Some(parse_listeners(&text))
}

/// The working folder of each of `pids`, canonical when it resolves.
async fn scan_cwds(pids: &[i32]) -> Option<HashMap<i32, String>> {
    if pids.is_empty() {
        return Some(HashMap::new());
    }
    let list = pids
        .iter()
        .map(i32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let text = run_lsof(&["-nP", "-w", "-a", "-p", &list, "-d", "cwd", "-Fpn"]).await?;
    Some(
        parse_cwds(&text)
            .into_iter()
            .map(|(pid, cwd)| {
                let real = Path::new(&cwd)
                    .canonicalize()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(cwd);
                (pid, real)
            })
            .collect(),
    )
}

/// The worktrees this vault's build workspaces name, paths canonical when
/// they resolve (a removed worktree keeps its recorded path).
fn bound_worktrees(vault: &Path) -> Vec<WorktreeRef> {
    crate::build_workspaces::all_worktrees(vault)
        .into_iter()
        .map(|(id, _, info)| WorktreeRef {
            workspace_id: Some(id),
            also_ids: Vec::new(),
            path: Path::new(&info.worktree)
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from(&info.worktree)),
            branch: Some(info.branch),
            exists: info.exists,
        })
        .collect()
}

fn gather_facts(vault: &Path, services: &[ServiceView]) -> Facts {
    // A live run's scope is the vault-relative `workspaces/<id>`
    // (`automated_history::validate_scope`), never the absolute worktree.
    let live_workspaces = crate::automated_history::scoped_run_ids(vault)
        .iter()
        .filter_map(|id| crate::automated_history::live_scope(vault, id))
        .filter_map(|(scope, _)| scope.strip_prefix("workspaces/").map(str::to_string))
        .collect();
    Facts {
        live_workspaces,
        service_pids: services
            .iter()
            .filter(|s| s.status == "running")
            .filter_map(|s| i32::try_from(s.pid).ok())
            .collect(),
        lease_held: crate::automated_history::lease_holder(vault).is_some(),
    }
}

/// Scan this Mac's listening ports and attribute them. `root` is the
/// worktrees folder (`disk_guard::worktrees_root`), for worktrees no
/// workspace is bound to.
pub(crate) async fn report(vault: &Path, root: Option<&Path>) -> PortReport {
    let services = crate::services::registry().list(None);
    let Some(listeners) = scan_listeners().await else {
        return PortReport {
            unavailable: Some("lsof did not answer in time or could not start".into()),
            ..PortReport::default()
        };
    };
    let mut pids: Vec<i32> = listeners.iter().map(|l| l.pid).collect();
    pids.sort_unstable();
    pids.dedup();
    let Some(cwds) = scan_cwds(&pids).await else {
        return PortReport {
            unavailable: Some("lsof could not read the processes' folders".into()),
            ..PortReport::default()
        };
    };
    let (vault, root) = (vault.to_path_buf(), root.map(Path::to_path_buf));
    let services_for_facts = services.clone();
    let (worktrees, facts) = crate::vault::spawn_blocking_logged({
        let vault = vault.clone();
        move || {
            (
                bound_worktrees(&vault),
                gather_facts(&vault, &services_for_facts),
            )
        }
    })
    .await
    .unwrap_or_default();
    let mut found = attribute(&listeners, &cwds, &worktrees, root.as_deref(), &facts);
    for e in &mut found.listeners {
        e.start_micros = crate::proc_info::process_start_micros(e.pid);
    }
    found
}

/// `list_services`' answer. Without `ports` it is exactly the services array
/// it always was; with it, an object `{ "services": [...], "listeningPorts":
/// {...} }` (additive: nothing existing reads the object form).
pub(crate) fn services_value(services: &[ServiceView], ports: Option<&PortReport>) -> Value {
    match ports {
        None => json!(services),
        Some(ports) => json!({ "services": services, "listeningPorts": ports }),
    }
}

/// The `build.list_services` action: services of `cwd` (or all), and with
/// `ports` the listening ports attributed to worktrees (of `cwd` only when
/// given).
pub(crate) async fn list_services_value(
    vault: &Path,
    root: Option<&Path>,
    cwd: Option<String>,
    ports: bool,
) -> Result<Value, String> {
    // With `ports`, a worktree that no longer exists is still a fair filter
    // (its orphans are the point): no service can be bound to it, so none are
    // listed, and the raw folder is compared. Without `ports` it stays an error.
    let scope = cwd
        .as_deref()
        .map(|c| crate::services::scope_of(Path::new(c)).unwrap_or_else(|_| c.to_string()));
    let missing = cwd
        .as_deref()
        .is_some_and(|c| crate::services::scope_of(Path::new(c)).is_err());
    let services = if ports && missing {
        Vec::new()
    } else {
        crate::commands::build_api::list_services(cwd.clone()).await?
    };
    if !ports {
        return Ok(services_value(&services, None));
    }
    let mut found = report(vault, root).await;
    if let Some(scope) = scope {
        let before = found.listeners.len();
        found.listeners.retain(|e| same_path(&e.worktree, &scope));
        found.unattributed += before - found.listeners.len();
    }
    Ok(services_value(&services, Some(&found)))
}

/// May `pid` on `port` be stopped? Only when the listing names it as an
/// orphan on that port. A pid that is gone, was reused by another process,
/// or is anything else the listing may report is refused, with the reason.
pub(crate) fn check_stop_target(
    report: &PortReport,
    pid: i32,
    port: u16,
    confirmed_by_person: bool,
) -> Result<&PortEntry, String> {
    // Orphan is evidence, not proof: another Hollow instance's scoped turns
    // are invisible here. A person must have seen the pid, command, port,
    // parent and folder and said yes.
    if !confirmed_by_person {
        return Err(
            "A person must confirm this stop after seeing the process; nothing was stopped.".into(),
        );
    }
    if pid <= 1 || pid == std::process::id() as i32 {
        return Err(format!("Refusing to signal pid {pid}."));
    }
    let entry = report
        .listeners
        .iter()
        .find(|e| e.pid == pid && e.port == port)
        .ok_or_else(|| {
            format!(
                "pid {pid} is not listening on port {port} in a worktree now; nothing was stopped."
            )
        })?;
    if entry.owner != Owner::Orphan {
        return Err(format!(
            "pid {pid} on port {port} is not a known orphan ({}); nothing was stopped.",
            entry.reason
        ));
    }
    Ok(entry)
}

/// The last look before a signal: `listen_text` and `cwd_text` are lsof
/// field output for this one pid (`-a -p <pid> -iTCP:<port> -sTCP:LISTEN
/// -FpgRcn` and `-a -p <pid> -d cwd -Fpn`), `start_now` its start time now.
/// Refuses unless it still listens on the port, its parent is still pid 1,
/// its working folder is the listed one and it started when the listing says
/// (a reused pid starts later), so a process that replaced the orphan between
/// the listing and the signal is never hit.
pub(crate) fn verify_fresh(
    entry: &PortEntry,
    listen_text: &str,
    cwd_text: &str,
    start_now: Option<u64>,
) -> Result<(), String> {
    let pid = entry.pid;
    let listens = parse_listeners(listen_text)
        .into_iter()
        .find(|l| l.pid == pid && l.port == entry.port)
        .ok_or_else(|| format!("pid {pid} no longer listens on port {}", entry.port))?;
    if listens.ppid != Some(1) {
        return Err(format!("pid {pid} no longer has launchd as its parent"));
    }
    let cwd = parse_cwds(cwd_text)
        .remove(&pid)
        .ok_or_else(|| format!("pid {pid}'s working folder can't be read now"))?;
    let real = |p: &str| {
        Path::new(p)
            .canonicalize()
            .unwrap_or_else(|_| PathBuf::from(p))
    };
    if real(&cwd) != real(&entry.cwd) {
        return Err(format!("pid {pid} now works in {cwd}, not {}", entry.cwd));
    }
    match (entry.start_micros, start_now) {
        (Some(listed), Some(now)) if listed == now => Ok(()),
        _ => Err(format!(
            "pid {pid} is not the process that was listed (start time differs or is unknown)"
        )),
    }
}

/// The last gate before kill(2): the fresh listing's entry AND the process
/// right now must both carry the start time the card showed.
fn final_check(fresh: &PortEntry, expected: u64, start_now: Option<u64>) -> Result<(), String> {
    if fresh.start_micros != Some(expected) {
        return Err(format!(
            "pid {} in the fresh listing is not the process the card showed",
            fresh.pid
        ));
    }
    if start_now != Some(expected) {
        return Err(format!(
            "pid {} changed or exited just before the signal",
            fresh.pid
        ));
    }
    Ok(())
}

/// Stop one orphan: re-read the ports now, check the target, look at the
/// process once more ([`verify_fresh`]), then SIGTERM that PID only. Not wired
/// to an action (see the module doc): the confirm-tier action that calls it
/// shows the entry's pid, command, port, parent and working folder on its
/// card first. `confirmed_by_person` must come from that card's approval by
/// the person, never from an argument an agent supplies, when this is wired.
#[allow(dead_code)]
pub(crate) async fn stop_orphan(
    vault: &Path,
    root: Option<&Path>,
    pid: i32,
    port: u16,
    expected_start_micros: u64,
    confirmed_by_person: bool,
) -> Result<String, String> {
    let now = report(vault, root).await;
    let entry = check_stop_target(&now, pid, port, confirmed_by_person)?.clone();
    // The card named this process by pid AND start time: a pid reused since
    // the card was shown is a different process.
    if entry.start_micros != Some(expected_start_micros) {
        return Err(format!(
            "pid {pid} is not the process the card showed (start time differs); nothing was stopped."
        ));
    }
    let list = pid.to_string();
    let port_spec = format!("TCP:{port}");
    let listen = run_lsof(&[
        "-nP",
        "-w",
        "-a",
        "-p",
        &list,
        "-i",
        &port_spec,
        "-sTCP:LISTEN",
        "-FpgRcn",
    ])
    .await
    .ok_or("lsof did not answer; nothing was stopped.")?;
    let cwd = run_lsof(&["-nP", "-w", "-a", "-p", &list, "-d", "cwd", "-Fpn"])
        .await
        .ok_or("lsof did not answer; nothing was stopped.")?;
    verify_fresh(
        &entry,
        &listen,
        &cwd,
        crate::proc_info::process_start_micros(pid),
    )
    .map_err(|why| format!("{why}; nothing was stopped."))?;
    // Live turns once more, right before the signal: a turn that began since
    // the first look makes the listener a live turn, not an orphan.
    let again = report(vault, root).await;
    let fresh = check_stop_target(&again, pid, port, confirmed_by_person)
        .map_err(|why| format!("{why} (rechecked just before signalling)"))?;
    // The scan above was slow: the orphan may have exited and its pid been
    // reused. Bind the fresh entry and the process as it is NOW to the card's
    // start time, as the last step before the signal.
    final_check(
        fresh,
        expected_start_micros,
        crate::proc_info::process_start_micros(pid),
    )
    .map_err(|why| format!("{why}; nothing was stopped."))?;
    // SAFETY: a plain kill(2) of one positive pid the check above accepted.
    let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
    if rc != 0 {
        return Err(format!(
            "Could not signal pid {pid}: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(format!(
        "Sent SIGTERM to pid {pid} ({}) on port {port} in {}.",
        entry.command, entry.worktree
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lsof -nP -iTCP -sTCP:LISTEN -FpgRcn` as macOS prints it.
    const LISTEN: &str = "\
p101\ng101\nR1\ncnode\nn*:5173\nn[::]:5173\n\
p202\ng202\nR1\ncControlCe\nn*:7000\n\
p303\ng303\nR55\ncnode\nn127.0.0.1:3000\nn127.0.0.1:3000->127.0.0.1:51234\n\
p404\ng404\nR1\ncpostgres\nn[::1]:5432\n\
p505\ng500\nR500\ncnode\nn*:8080\n";

    const CWDS: &str = "p101\nfcwd\nn/work/Worktrees/hollow/o17-x/web\n\
p202\nfcwd\nn/System/Library/CoreServices\n\
p303\nfcwd\nn/work/Worktrees/hollow/o17-x\n\
p404\nfcwd\nn/work/Worktrees/hollow/o17-x-extra\n\
p505\nfcwd\nn/work/Worktrees/hollow/loose-wt/app\n";

    fn bound(id: &str, path: &str) -> WorktreeRef {
        WorktreeRef {
            workspace_id: Some(id.into()),
            also_ids: Vec::new(),
            path: PathBuf::from(path),
            branch: Some("o17/x".into()),
            exists: true,
        }
    }

    fn plan() -> (Vec<Listener>, HashMap<i32, String>) {
        (parse_listeners(LISTEN), parse_cwds(CWDS))
    }

    #[test]
    fn lsof_field_output_parses_into_one_listener_per_pid_and_port() {
        let listeners = parse_listeners(LISTEN);
        let seen: Vec<(i32, u16)> = listeners.iter().map(|l| (l.pid, l.port)).collect();
        // 5173 listed for IPv4 and IPv6 is one; the established socket is skipped.
        assert_eq!(
            seen,
            vec![
                (101, 5173),
                (202, 7000),
                (303, 3000),
                (404, 5432),
                (505, 8080)
            ]
        );
        let first = &listeners[0];
        assert_eq!(first.command, "node");
        assert_eq!(first.pgid, Some(101));
        assert_eq!(first.ppid, Some(1));
        assert_eq!(listeners[3].address, "[::1]");
        assert_eq!(parse_listeners("").len(), 0);
        assert_eq!(parse_listeners("garbage\nnx:notaport\n").len(), 0);
    }

    #[test]
    fn cwd_output_maps_pid_to_folder_and_undoes_lsof_escapes() {
        let cwds = parse_cwds(CWDS);
        assert_eq!(cwds[&101], "/work/Worktrees/hollow/o17-x/web");
        let escaped = parse_cwds("p9\nfcwd\nn/Users/j/Alex\\xe2\\x80\\x99s\n");
        assert_eq!(escaped[&9], "/Users/j/Alex\u{2019}s");
    }

    #[test]
    fn a_planted_orphan_is_listed_with_its_worktree() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        let orphan = report.listeners.iter().find(|e| e.port == 5173).unwrap();
        assert_eq!(orphan.owner, Owner::Orphan);
        assert_eq!(orphan.pid, 101);
        assert_eq!(orphan.workspace_id.as_deref(), Some("ws-1"));
        assert_eq!(orphan.worktree, "/work/Worktrees/hollow/o17-x");
        assert_eq!(orphan.cwd, "/work/Worktrees/hollow/o17-x/web");
        assert_eq!(orphan.branch.as_deref(), Some("o17/x"));
        assert!(orphan.reason.contains("launchd (pid 1)"));
        // 7000 (system folder), 5432 (sibling worktree o17-x-extra: not a
        // prefix match) and 8080 (unbound, no root given) are counted only.
        assert_eq!(report.unattributed, 3);
    }

    #[test]
    fn a_live_turn_in_the_worktree_is_not_an_orphan() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let facts = Facts {
            live_workspaces: HashSet::from(["ws-1".to_string()]),
            ..Facts::default()
        };
        let report = attribute(&listeners, &cwds, &worktrees, None, &facts);
        assert!(report
            .listeners
            .iter()
            .all(|e| e.owner == Owner::LiveTurn || e.owner == Owner::Unknown));
        assert_eq!(
            report
                .listeners
                .iter()
                .find(|e| e.port == 5173)
                .unwrap()
                .owner,
            Owner::LiveTurn
        );
    }

    #[test]
    fn a_live_parent_or_a_foreign_lease_reads_unknown_never_orphan() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        // 3000 has parent 55, alive: a terminal may own it.
        let parented = report.listeners.iter().find(|e| e.port == 3000).unwrap();
        assert_eq!(parented.owner, Owner::Unknown);
        assert!(parented.reason.contains("parent process 55"));

        let foreign = Facts {
            lease_held: true,
            ..Facts::default()
        };
        let report = attribute(&listeners, &cwds, &worktrees, None, &foreign);
        assert!(report.listeners.iter().all(|e| e.owner == Owner::Unknown));
    }

    #[test]
    fn a_hollow_service_is_its_own_owner_by_pid_or_group() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        // 101 is the service's own pid; 303's group is 303.
        let facts = Facts {
            service_pids: vec![101, 303],
            ..Facts::default()
        };
        let report = attribute(&listeners, &cwds, &worktrees, None, &facts);
        for port in [5173, 3000] {
            let e = report.listeners.iter().find(|e| e.port == port).unwrap();
            assert_eq!(e.owner, Owner::HollowService, "port {port}");
        }
    }

    #[test]
    fn a_folder_under_the_worktrees_root_with_no_workspace_is_unknown_owner() {
        let (listeners, cwds) = plan();
        let root = PathBuf::from("/work/Worktrees");
        let report = attribute(&listeners, &cwds, &[], Some(&root), &Facts::default());
        let loose = report.listeners.iter().find(|e| e.port == 8080).unwrap();
        assert_eq!(loose.owner, Owner::Unknown);
        assert_eq!(loose.workspace_id, None);
        assert_eq!(loose.worktree, "/work/Worktrees/hollow/loose-wt");
        // pid 101 sits in hollow/o17-x: unbound too, so never an orphan.
        assert!(report.listeners.iter().all(|e| e.owner == Owner::Unknown));
    }

    #[test]
    fn the_longest_worktree_wins_and_a_removed_one_says_so() {
        let (listeners, cwds) = plan();
        let mut removed = bound("ws-2", "/work/Worktrees/hollow/o17-x");
        removed.exists = false;
        let outer = bound("ws-1", "/work/Worktrees/hollow");
        let report = attribute(
            &listeners,
            &cwds,
            &[outer, removed],
            None,
            &Facts::default(),
        );
        let e = report.listeners.iter().find(|e| e.port == 5173).unwrap();
        assert_eq!(e.workspace_id.as_deref(), Some("ws-2"));
        assert_eq!(e.owner, Owner::Orphan);
        assert!(e.reason.contains("worktree itself was removed"));
    }

    #[test]
    fn only_a_listed_orphan_on_that_port_may_be_stopped() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        assert_eq!(
            check_stop_target(&report, 101, 5173, true).unwrap().pid,
            101
        );
        // Wrong port, a pid that isn't listening, an unknown owner, an
        // unattributed process, pid 1 and this process are all refused.
        assert!(check_stop_target(&report, 101, 5174, true).is_err());
        assert!(check_stop_target(&report, 999, 5173, true).is_err());
        assert!(check_stop_target(&report, 303, 3000, true)
            .unwrap_err()
            .contains("not a known orphan"));
        assert!(check_stop_target(&report, 202, 7000, true).is_err());
        assert!(check_stop_target(&report, 1, 5173, true).is_err());
        assert!(check_stop_target(&report, std::process::id() as i32, 5173, true).is_err());
    }

    #[test]
    fn two_bindings_at_one_path_never_hide_a_live_turn() {
        let (listeners, cwds) = plan();
        let mut gone = bound("ws-old", "/work/Worktrees/hollow/o17-x");
        gone.exists = false;
        let live = bound("ws-new", "/work/Worktrees/hollow/o17-x");
        for order in [[gone.clone(), live.clone()], [live.clone(), gone.clone()]] {
            let facts = Facts {
                live_workspaces: ["ws-new".to_string()].into(),
                ..Facts::default()
            };
            let r = attribute(&listeners, &cwds, &order, None, &facts);
            let e = r.listeners.iter().find(|e| e.port == 5173).unwrap();
            assert_eq!(e.workspace_id.as_deref(), Some("ws-new"));
            assert_eq!(e.owner, Owner::LiveTurn);
            // A live turn in the removed binding still counts, either order.
            let facts = Facts {
                live_workspaces: ["ws-old".to_string()].into(),
                ..Facts::default()
            };
            let r = attribute(&listeners, &cwds, &order, None, &facts);
            let e = r.listeners.iter().find(|e| e.port == 5173).unwrap();
            assert_eq!(e.owner, Owner::LiveTurn);
        }
    }

    #[test]
    fn an_unconfirmed_stop_is_refused_even_for_an_orphan() {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        assert!(check_stop_target(&report, 101, 5173, false)
            .unwrap_err()
            .contains("confirm"));
        assert!(check_stop_target(&report, 101, 5173, true).is_ok());
    }

    #[test]
    fn an_own_full_vault_lease_makes_every_listener_unknown_never_orphan() {
        let dir = tempfile::tempdir().unwrap();
        let _lease = crate::automated_history::try_begin_owned(dir.path(), "chat", None).unwrap();
        let facts = gather_facts(dir.path(), &[]);
        assert!(facts.lease_held, "own lease must be seen");
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &facts);
        assert!(!report.listeners.is_empty());
        assert!(report.listeners.iter().all(|e| e.owner == Owner::Unknown));
    }

    #[test]
    fn a_live_turn_in_an_enclosing_binding_is_not_lost_to_a_nested_one() {
        let (listeners, cwds) = plan();
        let outer = bound("ws-outer", "/work/Worktrees/hollow");
        let inner = bound("ws-inner", "/work/Worktrees/hollow/o17-x");
        let facts = Facts {
            live_workspaces: ["ws-outer".to_string()].into(),
            ..Facts::default()
        };
        let r = attribute(&listeners, &cwds, &[outer, inner], None, &facts);
        let e = r.listeners.iter().find(|e| e.port == 5173).unwrap();
        assert_eq!(e.workspace_id.as_deref(), Some("ws-inner"));
        assert_eq!(e.owner, Owner::LiveTurn);
    }

    #[test]
    fn the_removed_worktree_filter_compares_canonical_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().canonicalize().unwrap();
        let gone = real.join("gone");
        let spelled = format!("{}/gone/", tmp.path().display());
        assert!(same_path(&spelled, &gone.to_string_lossy()));
        assert!(!same_path(&spelled, &real.join("other").to_string_lossy()));
    }

    #[test]
    fn lsof_exit_one_with_no_output_is_empty_but_with_a_message_is_a_failure() {
        assert!(lsof_succeeded(Some(0), false, "", false));
        assert!(lsof_succeeded(Some(1), true, "", false));
        assert!(!lsof_succeeded(Some(1), true, "lsof: bad option", false));
        assert!(!lsof_succeeded(Some(2), true, "", false));
        assert!(!lsof_succeeded(None, true, "", false));
        assert!(!lsof_succeeded(Some(0), false, "", true));
    }

    #[test]
    fn the_final_check_binds_fresh_entry_and_live_process_to_the_card_time() {
        let e = listed_orphan(); // start_micros Some(1_000)
        assert!(final_check(&e, 1_000, Some(1_000)).is_ok());
        // Fresh listing shows a different (reused) process.
        assert!(final_check(&e, 999, Some(999)).is_err());
        let mut unlisted = e.clone();
        unlisted.start_micros = None;
        assert!(final_check(&unlisted, 1_000, Some(1_000)).is_err());
        // Process replaced or gone between the listing and the signal.
        assert!(final_check(&e, 1_000, Some(2_000)).is_err());
        assert!(final_check(&e, 1_000, None).is_err());
    }

    #[test]
    fn a_absurdly_deep_folder_is_not_attributed() {
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let deep = format!("/work/Worktrees/hollow/o17-x{}", "/d".repeat(80));
        assert!(worktree_of(Path::new(&deep), &worktrees, None).is_none());
        assert!(worktree_of(
            Path::new("/work/Worktrees/hollow/o17-x/d"),
            &worktrees,
            None
        )
        .is_some());
    }

    #[test]
    fn list_services_keeps_its_array_shape_unless_ports_are_asked_for() {
        let service = ServiceView {
            scope: "/w".into(),
            name: "web".into(),
            command: "npm run dev".into(),
            cwd: "/w".into(),
            port: Some(5173),
            kind: crate::services::Kind::DevServer,
            status: "running".into(),
            exit_code: None,
            pid: 9,
            started_at: "t".into(),
            log: "/l".into(),
        };
        let old = services_value(std::slice::from_ref(&service), None);
        assert!(old.is_array(), "the old answer is still a bare array");
        assert_eq!(old, json!([service.clone()]));
        let keys: Vec<&str> = old[0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(!keys.contains(&"listeningPorts") && !keys.contains(&"owner"));

        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        let new = services_value(&[service], Some(&report));
        assert_eq!(new["services"][0]["name"], "web");
        let rows = new["listeningPorts"]["listeners"].as_array().unwrap();
        let orphan = rows.iter().find(|r| r["port"] == 5173).unwrap();
        assert_eq!(orphan["owner"], "orphan");
        assert_eq!(orphan["workspaceId"], "ws-1");
        let unknown = rows.iter().find(|r| r["port"] == 3000).unwrap();
        assert_eq!(unknown["owner"], "unknown-owner");
        assert!(new["listeningPorts"].get("unavailable").is_none());
    }

    #[test]
    fn a_real_scoped_run_makes_its_workspace_live_turn_and_its_end_makes_it_an_orphan() {
        // Production scopes are `workspaces/<id>` (never the absolute worktree).
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let v = dir.path();
        std::fs::create_dir_all(v.join("workspaces/ws-live-oc14")).unwrap();
        std::fs::write(v.join("workspaces/ws-live-oc14/WORKSPACE.md"), "brief").unwrap();
        let lease = crate::automated_history::try_begin_scoped(
            v,
            "workspaces/ws-live-oc14",
            "chat",
            Some("s-20261117-portowners"),
        )
        .unwrap();
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-live-oc14", "/work/Worktrees/hollow/o17-x")];
        let live = gather_facts(v, &[]);
        assert!(live.live_workspaces.contains("ws-live-oc14"));
        let report = attribute(&listeners, &cwds, &worktrees, None, &live);
        let e = report.listeners.iter().find(|e| e.port == 5173).unwrap();
        assert_eq!(e.owner, Owner::LiveTurn, "{}", e.reason);

        drop(lease);
        let after = gather_facts(v, &[]);
        assert!(!after.live_workspaces.contains("ws-live-oc14"));
        let report = attribute(&listeners, &cwds, &worktrees, None, &after);
        let e = report.listeners.iter().find(|e| e.port == 5173).unwrap();
        assert_eq!(e.owner, Owner::Orphan);
        assert!(e.reason.contains("launchd (pid 1)"));
    }

    #[test]
    fn a_bound_path_that_is_too_short_or_relative_attributes_nothing() {
        let (listeners, cwds) = plan();
        for bad in ["", "/", "/work", "work/Worktrees/hollow", "relative"] {
            let report = attribute(
                &listeners,
                &cwds,
                &[bound("ws-bad", bad)],
                None,
                &Facts::default(),
            );
            assert!(report.listeners.is_empty(), "{bad:?} claimed a process");
            assert_eq!(report.unattributed, 5);
        }
    }

    #[test]
    fn a_forged_record_inside_a_name_or_a_repeated_pid_forges_nothing() {
        // lsof writes a newline in a name as the two characters `\n`, so a
        // folder named "a<NL>p1<NL>n/etc" can't start a record of its own.
        let cwds = parse_cwds("p9\nfcwd\nn/w/a\\np1\\nfcwd\\nn/etc\n");
        assert_eq!(cwds.len(), 1);
        assert_eq!(cwds[&9], "/w/a\np1\nfcwd\nn/etc");
        // A command name with an escaped record stays one command.
        let listeners = parse_listeners("p7\ng7\nR1\ncevil\\np1\\nR1\nn*:80\n");
        assert_eq!(listeners.len(), 1);
        assert_eq!(listeners[0].pid, 7);
        assert_eq!(listeners[0].command, "evil\np1\nR1");
        // A second `p7` set (a raw forged record) is ignored: first wins.
        let twice = parse_listeners("p7\nR55\nn*:80\np7\nR1\nn*:81\n");
        assert_eq!(twice.len(), 1);
        assert_eq!(twice[0].ppid, Some(55));
    }

    fn listed_orphan() -> PortEntry {
        let (listeners, cwds) = plan();
        let worktrees = [bound("ws-1", "/work/Worktrees/hollow/o17-x")];
        let report = attribute(&listeners, &cwds, &worktrees, None, &Facts::default());
        let mut e = report
            .listeners
            .iter()
            .find(|e| e.port == 5173)
            .unwrap()
            .clone();
        e.start_micros = Some(1_000);
        e
    }

    #[test]
    fn the_last_look_refuses_a_process_that_changed_since_the_listing() {
        let e = listed_orphan();
        let listen = "p101\ng101\nR1\ncnode\nn*:5173\n";
        let cwd = "p101\nfcwd\nn/work/Worktrees/hollow/o17-x/web\n";
        assert_eq!(verify_fresh(&e, listen, cwd, Some(1_000)), Ok(()));
        // Reused pid: a later start time, or none readable, or none listed.
        assert!(verify_fresh(&e, listen, cwd, Some(2_000)).is_err());
        assert!(verify_fresh(&e, listen, cwd, None).is_err());
        let mut unlisted = e.clone();
        unlisted.start_micros = None;
        assert!(verify_fresh(&unlisted, listen, cwd, Some(1_000)).is_err());
        // No longer listening, a live parent now, another port, another folder.
        assert!(verify_fresh(&e, "", cwd, Some(1_000)).is_err());
        assert!(verify_fresh(&e, "p101\nR55\nn*:5173\n", cwd, Some(1_000))
            .unwrap_err()
            .contains("launchd"));
        assert!(verify_fresh(&e, "p101\nR1\nn*:5999\n", cwd, Some(1_000)).is_err());
        assert!(
            verify_fresh(&e, listen, "p101\nfcwd\nn/Users/j/other\n", Some(1_000))
                .unwrap_err()
                .contains("now works in")
        );
        assert!(verify_fresh(&e, listen, "", Some(1_000)).is_err());
    }
}
