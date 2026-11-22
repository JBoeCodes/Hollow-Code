//! Workflow permission stamps: an automated run must not escalate its own
//! automation (Paperclip PR 5, bug 0.6).
//!
//! A workflow file in `vault/workflows/*.md` decides what an unattended agent
//! may do: its permission mode, tool allow/deny lists, working folder, agent,
//! model, triggers, schedule and prompt body. Every agent that runs with the
//! vault as its working folder can write that file, so without a guard an
//! automated run could rewrite `permission_mode: bypassPermissions` (or its
//! prompt) and the next scheduled, triggered or Run-now run would execute it.
//!
//! **Stamps.** For each workflow Hollow records the exact [`PermissionFields`]
//! it last approved, built from [`crate::vault_workflows::parse_content`] —
//! the same parser the runner executes from, so no parser differential can
//! separate what was stamped from what runs. The store lives OUTSIDE the vault
//! (`{data_dir}/Hollow/WorkflowPermissions/{vault}-{hash16}.json`, 0600, keyed
//! by canonical vault path) and keeps the approved prompt text (≤ 64 KiB) so a
//! review can show a real before/after.
//!
//! **Trust on first use.** [`initialize`] runs once at startup, before
//! reconciliation, the scheduler, the watcher and IPC. The first launch of this
//! build (or of a new vault) trusts every workflow already on disk as-is and
//! records `trustedAt`. [`verify`] never trusts on first use: a deleted store
//! refuses every automation until the next launch re-trusts them, and a corrupt
//! store fails closed and is never overwritten.
//!
//! **Owned writers.** The automation editor (`vault_workflows::upsert_input`),
//! `delete` and `set_schedule` re-stamp by
//! the rules in the `*_locked` hooks below: a create stamps; a save re-stamps
//! only when the file it replaced still matched its stamp; a routine toggle
//! moves only the schedule of an otherwise-matching stamp. Status and run-log
//! writes never re-stamp, and cannot change a stamped field: since Paperclip
//! PR 9 they are verified line patches that never edit a stamped key or the
//! prompt bytes, and their fallback for a refused patch (the canonical
//! render) re-emits every parsed value. The Library
//! editor, Obsidian, `move_note`, History restore and agents are not owned
//! writers: their changes need an explicit approval.
//!
//! **Deletes.** A file that goes missing outside the editor (Obsidian, the
//! Library, `move_note`, an agent) has its stamp tombstoned the first time a
//! check sees it gone (the watcher's sweep, usually within a second): a
//! tombstone approves nothing, so a file recreated at that path, even
//! byte-for-byte the approved one, needs a new approval. A run that deleted a
//! workflow is still flagged. A cloud-evicted file (an iCloud `.name.icloud`
//! placeholder) or a missing `workflows/` folder keeps its approval.
//!
//! **Refusal and review.** [`verify`] compares the file with its stamp. A
//! mismatch is refused and keeps exactly one live `workflow_permissions` review
//! (fingerprint = the approved→observed digest pair) in the review queue.
//! Accepting it ([`approve`]) re-derives everything from disk and re-stamps;
//! the approval is bound to the fingerprint of the card the user confirmed,
//! and duplicate ids in the queue are refused. A review the watcher's quiet
//! sweep opened is announced by the next notify-capable check, so a
//! hand-edited scheduled automation never pauses silently; one a flagged run
//! opened is covered by that run's own notification. [`permission_holds`]
//! answers "would this be refused now?" read-only, for the Automations list.
//! Every run path reaches `verify` before the history lease, through
//! `workflows::precheck`: Run now checks it up front (`workflows::run`, so a
//! refusal returns at once even while the vault is busy), and the run-queue
//! pump's claim (`run_pump::claim_verdict`, phase 2) checks every cron,
//! trigger, manual and retry row again before `workflows::admit_runnable`,
//! which accepts only `precheck`'s output (Paperclip PR 7). The refusal is
//! typed ([`VerifyError`]): a held row is deferred with its `held_reason`
//! (never a lease wait), a trigger row keeps its fire (its watermark was
//! committed at enqueue), and past grace a cron row writes its own
//! `not run: …` row.
//!
//! **Trust boundary.** The store is outside the vault, so acceptEdits and
//! workspace-write agents can't reach it. It is not a sandbox: an agent with
//! Bash or bypassPermissions can edit it (or the queue, or history).
//!
//! **Lock order:** `automated_history` HISTORY_LOCK → [`write_guard`] →
//! {`vault::MUTATE_LOCK`, STORE_LOCK, review-queue QUEUE_LOCK} →
//! `run_guard` TRACKED. STORE_LOCK is never held while calling the review
//! queue. Nothing under the write guard may take HISTORY_LOCK, and the guard is
//! not reentrant: a `*_locked` hook must never call [`verify`] or [`approve`].

use crate::notifications::{self, NotificationInput};
use crate::review_queue::{self, NewReviewItem, ReviewItem};
use crate::types::Workflow;
use crate::vault;
use crate::vault_workflows;
use chrono::Utc;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// Review-queue kind for a pending permission change.
pub const REVIEW_KIND: &str = "workflow_permissions";
/// Review-queue source for a pending permission change.
pub const REVIEW_SOURCE: &str = "workflow";
/// Every refusal message starts with this (mirrored in
/// `src/workflowPermissions.ts` as `WORKFLOW_REVIEW_REFUSAL_PREFIX`).
pub const REFUSAL_PREFIX: &str = "Automation paused for review:";
/// The approved prompt text is kept for review diffs up to this size.
const PROMPT_TEXT_CAP: usize = 64 * 1024;
const SCHEMA_VERSION: u32 = 1;
/// One refusal notification per workflow per this interval.
const NOTICE_INTERVAL: Duration = Duration::from_secs(10 * 60);
const SCALAR_CAP: usize = 300;
const PROMPT_EXCERPT_CAP: usize = 2_000;
const PROMPT_CONTEXT_LINES: usize = 2;
const SUMMARY_CAP: usize = 600;
const SUMMARY_LIST_ITEMS: usize = 4;

static STORE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
static WRITE_GUARD: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
/// Last notice per (canonical vault, slug); the store-unavailable notice uses
/// [`STORE_NOTICE_KEY`] as its slug, so it is rate-limited per vault.
static LAST_NOTICE: Lazy<Mutex<HashMap<(PathBuf, String), Instant>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
/// Reviews opened quietly (the watcher's sweep) and not yet announced, keyed
/// by (canonical vault, fingerprint). The next notify-capable check (the
/// scheduler's admission when the automation comes due, Run now, the startup
/// sweep) sends the one rate-limited warning; a post-run check removes the
/// entry because that run's own notification already covers it.
static UNANNOUNCED: Lazy<Mutex<BTreeSet<(PathBuf, String)>>> =
    Lazy::new(|| Mutex::new(BTreeSet::new()));
const STORE_NOTICE_KEY: &str = "\u{0}permission-store";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Held by every Hollow-owned workflow writer across its file write and
/// re-stamp, and by verify/approve and the post-run workflow flag check, so
/// none of them can observe a half-finished owned write. Poison-tolerant.
pub(crate) fn write_guard() -> MutexGuard<'static, ()> {
    lock(&WRITE_GUARD)
}

// ── Permission fields ────────────────────────────────────────────────────────

/// Everything about a workflow that changes what an automated run may do.
/// Title, description, output_glob, routine, misfire_grace_min and the
/// last_*/next_run_at bookkeeping are deliberately excluded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionFields {
    pub permission_mode: String,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub triggers: Vec<String>,
    pub schedule: String,
    /// The conversation each run is a turn in (overhaul F1). Omitted when
    /// None, so a stamp from before F1 keeps its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_chat: Option<String>,
    /// The owner Mac (M4, `runs_on`): a change moves the automation to a Mac
    /// with other logins, so it is reviewed like any stamped change. Omitted
    /// when None, so a stamp from before M4 keeps its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_on: Option<String>,
    /// The precheck command (OC13): it runs on this Mac as the person before
    /// each scheduled occurrence, so setting or changing it is reviewed like
    /// a new permission. Omitted when None, so a stamp from before OC13 keeps
    /// its digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub precheck: Option<String>,
    /// sha256 hex of the trimmed prompt body.
    pub prompt_sha256: String,
}

impl PermissionFields {
    /// Fields of an already-parsed workflow (the exact values the runner uses).
    pub fn from_workflow(w: &Workflow, prompt: &str) -> Self {
        let mode = w.permission_mode.trim();
        Self {
            permission_mode: if mode.is_empty() {
                "acceptEdits".into()
            } else {
                mode.to_string()
            },
            allowed_tools: w.allowed_tools.clone(),
            disallowed_tools: w.disallowed_tools.clone(),
            cwd: w.cwd.clone(),
            agent: w.agent.clone(),
            model: w.model.clone(),
            triggers: w.triggers.clone(),
            schedule: w.schedule.clone(),
            in_chat: w.in_chat.clone(),
            runs_on: w.runs_on.clone(),
            precheck: w.precheck.clone(),
            prompt_sha256: sha256_hex(prompt.trim().as_bytes()),
        }
    }

    /// Parse file text and return (fields, trimmed prompt). None when the
    /// file can't be parsed as a workflow (e.g. it has no title).
    pub fn from_content(slug: &str, content: &str) -> Option<(Self, String)> {
        let (w, prompt, _) = vault_workflows::parse_content(slug, content).ok()?;
        let fields = Self::from_workflow(&w, &prompt);
        Some((fields, prompt.trim().to_string()))
    }

    /// Stable content digest: sha256 hex of `v1:` + the JSON encoding.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        h.update(b"v1:");
        h.update(serde_json::to_vec(self).unwrap_or_default());
        format!("{:x}", h.finalize())
    }

    fn same_except_schedule(&self, other: &Self) -> bool {
        Self {
            schedule: other.schedule.clone(),
            ..self.clone()
        } == *other
    }

    /// Human labels of the fields that differ. `None` approved means the file
    /// has no approval at all.
    pub fn changed_fields(approved: Option<&Self>, observed: &Self) -> Vec<&'static str> {
        let Some(a) = approved else {
            return vec!["new automation file"];
        };
        let mut out = Vec::new();
        if a.permission_mode != observed.permission_mode {
            out.push("permission mode");
        }
        if a.allowed_tools != observed.allowed_tools {
            out.push("allowed tools");
        }
        if a.disallowed_tools != observed.disallowed_tools {
            out.push("disallowed tools");
        }
        if a.cwd != observed.cwd {
            out.push("working folder");
        }
        if a.agent != observed.agent {
            out.push("agent");
        }
        if a.model != observed.model {
            out.push("model");
        }
        if a.triggers != observed.triggers {
            out.push("triggers");
        }
        if a.schedule != observed.schedule {
            out.push("schedule");
        }
        if a.in_chat != observed.in_chat {
            out.push("runs in");
        }
        if a.runs_on != observed.runs_on {
            out.push("runs on");
        }
        if a.precheck != observed.precheck {
            out.push("precheck");
        }
        if a.prompt_sha256 != observed.prompt_sha256 {
            out.push("prompt");
        }
        out
    }

    /// True when the change grants the automation more power: it is new, its
    /// permission mode rises, its tool allowlist widens (an EMPTY allowlist
    /// means every tool, so adding the first entries is a restriction but
    /// clearing a non-empty list is an escalation), its denylist loses
    /// entries, its working folder or agent changes, or it gains triggers.
    pub fn escalation(approved: Option<&Self>, observed: &Self) -> bool {
        let Some(a) = approved else {
            return true;
        };
        let gained =
            |before: &[String], after: &[String]| after.iter().any(|t| !before.contains(t));
        mode_rank(&observed.permission_mode) > mode_rank(&a.permission_mode)
            || (!a.allowed_tools.is_empty()
                && (observed.allowed_tools.is_empty()
                    || gained(&a.allowed_tools, &observed.allowed_tools)))
            || a.disallowed_tools
                .iter()
                .any(|t| !observed.disallowed_tools.contains(t))
            || a.cwd != observed.cwd
            || a.agent != observed.agent
            || gained(&a.triggers, &observed.triggers)
            || (observed.in_chat.is_some() && a.in_chat != observed.in_chat)
            || (observed.runs_on.is_some() && a.runs_on != observed.runs_on)
            || (observed.precheck.is_some() && a.precheck != observed.precheck)
    }

    /// Equal but for `runs_on` (M4's owner rewrite after a slug rename).
    fn same_except_runs_on(&self, other: &Self) -> bool {
        Self {
            runs_on: other.runs_on.clone(),
            ..self.clone()
        } == *other
    }
}

/// plan < default < acceptEdits < auto < bypassPermissions; unknown modes
/// rank highest so an unrecognized value is never treated as a restriction.
fn mode_rank(mode: &str) -> u8 {
    match mode.trim() {
        "plan" => 0,
        "default" => 1,
        "acceptEdits" => 2,
        "auto" => 3,
        "bypassPermissions" => 4,
        _ => 5,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn workflow_rel(slug: &str) -> String {
    format!("{}/{slug}.md", vault_workflows::WORKFLOWS_DIR)
}

/// A slug Hollow could have produced: one path component, not hidden.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty() && !slug.starts_with('.') && !slug.contains('/') && !slug.contains('\\')
}

// ── Stamp store ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StampFile {
    schema_version: u32,
    #[serde(default)]
    vault: String,
    /// Set once trust-on-first-use has run for this vault. File existence is
    /// not enough: setup may create the store before the vault's first launch.
    #[serde(default)]
    trusted_at: Option<String>,
    #[serde(default)]
    workflows: BTreeMap<String, Stamp>,
}

impl Default for StampFile {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            vault: String::new(),
            trusted_at: None,
            workflows: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stamp {
    fields: PermissionFields,
    /// The approved prompt text (trimmed); None when it exceeded the cap.
    #[serde(default)]
    prompt: Option<String>,
    stamped_at: String,
    /// `tofu` | `create` | `save` | `rename` | `schedule` | `review`.
    by: String,
    /// Set when Hollow saw the file missing outside the automation editor
    /// (Obsidian, the Library, `move_note`, an agent). A tombstoned stamp
    /// approves nothing: a file recreated at that path, even byte-for-byte the
    /// approved one (an agent can recover it from `.hollow-history/objects`),
    /// needs a new approval. The editor's own delete forgets the stamp instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    deleted_at: Option<String>,
    /// The script file the approved `precheck` command names (a path inside
    /// the working folder) and the sha256 of its bytes when it was approved
    /// (OC13). The command text alone would let an edit to the script change
    /// what runs without a review; the gate refuses a script that differs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    precheck_script: Option<crate::workflow_precheck::ScriptPin>,
}

impl Stamp {
    fn new(fields: PermissionFields, prompt: &str, by: &str) -> Self {
        let prompt = prompt.trim();
        Self {
            fields,
            prompt: (prompt.len() <= PROMPT_TEXT_CAP).then(|| prompt.to_string()),
            stamped_at: Utc::now().to_rfc3339(),
            by: by.to_string(),
            deleted_at: None,
            precheck_script: None,
        }
    }

    /// The approval this stamp grants: none once tombstoned.
    fn live(&self) -> Option<&Stamp> {
        self.deleted_at.is_none().then_some(self)
    }
}

/// Every stamp's state, for the post-run flag check.
#[derive(Debug, Default)]
pub(crate) struct Approvals {
    /// slug → approved fields, for stamps that still approve something.
    pub live: BTreeMap<String, PermissionFields>,
    /// Slugs whose stamp was tombstoned (the file went missing outside the
    /// editor). Hollow still "knows" them, so a run's deletion is flagged.
    pub tombstoned: BTreeSet<String>,
}

impl Approvals {
    /// True while Hollow holds any stamp for `slug`, live or tombstoned.
    pub fn knows(&self, slug: &str) -> bool {
        self.live.contains_key(slug) || self.tombstoned.contains(slug)
    }
}

/// Where the stamp stores live. Test builds get a per-process temp dir so a
/// test can never write into the user's app data dir (the same swap as
/// `session_store`).
#[cfg(not(test))]
fn store_root() -> Option<PathBuf> {
    Some(crate::instance::support_dir()?.join("WorkflowPermissions"))
}

#[cfg(test)]
fn store_root() -> Option<PathBuf> {
    Some(crate::test_temp::process_dir(
        "hollow-test-workflow-permissions-",
    ))
}

fn canonical_vault(vault: &Path) -> PathBuf {
    fs::canonicalize(vault).unwrap_or_else(|_| vault.to_path_buf())
}

/// Named exactly like `session_store::db_path_for`, with `.json`.
fn store_path(vault: &Path) -> Option<PathBuf> {
    let canon = canonical_vault(vault);
    let hash = sha256_hex(canon.to_string_lossy().as_bytes());
    let name: String = canon
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "vault".to_string())
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    Some(store_root()?.join(format!("{name}-{}.json", &hash[..16])))
}

fn load(vault: &Path) -> Result<StampFile, String> {
    let path = store_path(vault)
        .ok_or_else(|| "this system has no application data directory".to_string())?;
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(StampFile::default()),
        Err(e) => return Err(format!("read failed: {e}")),
    };
    let file: StampFile =
        serde_json::from_slice(&bytes).map_err(|e| format!("invalid JSON: {e}"))?;
    if file.schema_version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported schema version {} (expected {SCHEMA_VERSION})",
            file.schema_version
        ));
    }
    Ok(file)
}

/// Owner-only atomic write ([`vault::atomic_write_private`]: synced 0600
/// temp file → rename → directory sync), in a 0700 folder.
fn save(vault: &Path, file: &StampFile) -> Result<(), String> {
    let path = store_path(vault)
        .ok_or_else(|| "this system has no application data directory".to_string())?;
    let dir = path
        .parent()
        .ok_or_else(|| "permission store path has no parent".to_string())?;
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
    }
    let body =
        serde_json::to_vec_pretty(file).map_err(|e| format!("encode permission store: {e}"))?;
    vault::atomic_write_private(&path, &body)
        .map_err(|e| format!("replace {}: {e}", path.display()))
}

/// Serialized load → modify → save. A load error (corrupt store) aborts
/// before `f` runs, so a corrupt store is never overwritten.
fn with_store<T>(
    vault: &Path,
    f: impl FnOnce(&mut StampFile) -> Result<(T, bool), String>,
) -> Result<T, String> {
    let _guard = lock(&STORE_LOCK);
    let mut file = load(vault)?;
    let (value, changed) = f(&mut file)?;
    if changed {
        if file.vault.is_empty() {
            file.vault = canonical_vault(vault).to_string_lossy().to_string();
        }
        save(vault, &file)?;
    }
    Ok(value)
}

/// The stamp that still approves `slug` (None when absent or tombstoned).
fn stamp_for(vault: &Path, slug: &str) -> Result<Option<Stamp>, String> {
    with_store(vault, |file| {
        Ok((
            file.workflows.get(slug).and_then(Stamp::live).cloned(),
            false,
        ))
    })
}

/// The script pinned by `slug`'s live stamp (OC13): what its approved
/// `precheck` command named and hashed. None when it named no script, the
/// script could not be hashed, or there is no live stamp or store.
pub(crate) fn precheck_pin(
    vault: &Path,
    slug: &str,
) -> Option<crate::workflow_precheck::ScriptPin> {
    stamp_for(vault, slug)
        .ok()
        .flatten()
        .and_then(|stamp| stamp.precheck_script)
}

/// Approved fields for every workflow with a live stamp.
pub(crate) fn stamps(vault: &Path) -> Result<BTreeMap<String, PermissionFields>, String> {
    Ok(approvals(vault)?.live)
}

/// Live approvals and tombstoned slugs.
pub(crate) fn approvals(vault: &Path) -> Result<Approvals, String> {
    with_store(vault, |file| {
        let mut out = Approvals::default();
        for (slug, stamp) in &file.workflows {
            match stamp.live() {
                Some(stamp) => {
                    out.live.insert(slug.clone(), stamp.fields.clone());
                }
                None => {
                    out.tombstoned.insert(slug.clone());
                }
            }
        }
        Ok((out, false))
    })
}

/// Tombstone a live stamp whose file is gone. Returns whether it changed.
fn tombstone(vault: &Path, slug: &str) -> Result<bool, String> {
    with_store(vault, |file| match file.workflows.get_mut(slug) {
        Some(stamp) if stamp.deleted_at.is_none() => {
            stamp.deleted_at = Some(Utc::now().to_rfc3339());
            Ok((true, true))
        }
        _ => Ok((false, false)),
    })
}

/// A file that is missing only because a cloud provider evicted it (iCloud
/// leaves a `.{name}.icloud` placeholder beside it) or because the whole
/// `workflows/` folder is unavailable (a partial sync, an unmounted vault) is
/// not a deletion: keep its approval so re-downloading it needs no review.
fn missing_is_transient(full: &Path) -> bool {
    let (Some(parent), Some(name)) = (full.parent(), full.file_name()) else {
        return true;
    };
    !parent.is_dir()
        || parent
            .join(format!(".{}.icloud", name.to_string_lossy()))
            .exists()
}

/// The pin of the script `fields.precheck` names, as the disk has it now.
fn pin_now(vault: &Path, fields: &PermissionFields) -> Option<crate::workflow_precheck::ScriptPin> {
    let command = fields.precheck.as_deref()?;
    let cwd = fields
        .cwd
        .as_deref()
        .filter(|cwd| !cwd.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| vault.to_path_buf());
    crate::workflow_precheck::script_pin(command, &cwd)
        .ok()
        .flatten()
}

fn put_stamp(
    vault: &Path,
    slug: &str,
    fields: PermissionFields,
    prompt: &str,
    by: &str,
    agent: bool,
) -> Result<(), String> {
    let mut stamp = Stamp::new(fields, prompt, by);
    // The person's own editor save (and a new file they make) pins the script
    // as it is now. A save an AGENT drives, and any approval that is not about
    // the script (a review card, an app card, a rename), never hashes the
    // script: it keeps the OLD pin only while the command and folder are
    // unchanged, otherwise the pin is left empty. Either way the gate refuses
    // an edited or unpinned script until the person saves the automation in
    // the editor, so an agent cannot get a script approved by a card that
    // shows no script bytes.
    let person_pins = !agent && matches!(by, "save" | "create");
    if stamp.fields.precheck.is_some() && !person_pins {
        stamp.precheck_script = stamp_for(vault, slug)
            .ok()
            .flatten()
            .filter(|old| {
                old.fields.precheck == stamp.fields.precheck && old.fields.cwd == stamp.fields.cwd
            })
            .and_then(|old| old.precheck_script)
            .or_else(|| {
                // A person's review approval of a changed command pins; an
                // agent-driven one never does.
                (!agent).then(|| pin_now(vault, &stamp.fields)).flatten()
            });
    } else if stamp.fields.precheck.is_some() {
        stamp.precheck_script = pin_now(vault, &stamp.fields);
    }
    with_store(vault, |file| {
        file.workflows.insert(slug.to_string(), stamp);
        Ok(((), true))
    })
}

/// Move a stamp with its file. When `from` has no stamp, any stale stamp at
/// `to` is dropped so a rename can never inherit an unrelated approval.
fn move_stamp(vault: &Path, from: &str, to: &str) -> Result<(), String> {
    with_store(vault, |file| {
        let moved = file.workflows.remove(from);
        let had_target = match moved {
            Some(stamp) => {
                file.workflows.insert(to.to_string(), stamp);
                true
            }
            None => file.workflows.remove(to).is_some(),
        };
        Ok(((), had_target))
    })
}

fn remove_stamp(vault: &Path, slug: &str) -> Result<(), String> {
    with_store(vault, |file| {
        let removed = file.workflows.remove(slug).is_some();
        Ok(((), removed))
    })
}

fn unavailable_message(vault: &Path, detail: &str) -> String {
    let path = store_path(vault)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "the automation permission store".into());
    format!(
        "Automation permission records unavailable: Hollow could not read {path} ({detail}). \
         Nothing was overwritten; repair or move that file, then restart Hollow."
    )
}

// ── Startup ──────────────────────────────────────────────────────────────────

/// Trust on first use. Runs synchronously at startup before reconciliation,
/// the scheduler, the watcher and IPC. The first time for a vault it stamps
/// every workflow that has no stamp yet (`by: tofu`) and records `trustedAt`;
/// afterwards it is a no-op. Returns how many workflows were trusted. A
/// corrupt store is an error and is left untouched.
pub fn initialize(vault: &Path) -> Result<usize, String> {
    let _guard = write_guard();
    with_store(vault, |file| {
        if file.trusted_at.is_some() {
            return Ok((0, false));
        }
        let mut trusted = 0;
        for w in vault_workflows::list(vault) {
            if file.workflows.contains_key(&w.slug) {
                continue;
            }
            let Ok(full) = vault::safe_join(vault, &workflow_rel(&w.slug)) else {
                continue;
            };
            let Ok(content) = fs::read_to_string(full) else {
                continue;
            };
            let Some((fields, prompt)) = PermissionFields::from_content(&w.slug, &content) else {
                continue;
            };
            file.workflows
                .insert(w.slug.clone(), Stamp::new(fields, &prompt, "tofu"));
            trusted += 1;
        }
        file.trusted_at = Some(Utc::now().to_rfc3339());
        file.vault = canonical_vault(vault).to_string_lossy().to_string();
        Ok((trusted, true))
    })
    .map_err(|e| unavailable_message(vault, &e))
}

/// Check every workflow (and every live-stamped slug, so a deleted file's
/// stamp is tombstoned) so reviews exist, are refreshed or superseded, and
/// retire when a file matches again. `notify` is true only for the startup
/// sweep. The watcher's sweep is quiet so an Obsidian autosave never
/// notifies, but a review it opens is remembered and announced by the next
/// notify-capable check (typically the scheduler's, when the automation comes
/// due), so a hand-edited scheduled automation never stops silently.
pub fn sweep(vault: &Path, notify: bool) {
    let mut slugs: BTreeSet<String> = vault_workflows::list(vault)
        .into_iter()
        .map(|w| w.slug)
        .collect();
    if let Ok(stamped) = stamps(vault) {
        slugs.extend(stamped.into_keys());
    }
    let mode = if notify { Notice::Send } else { Notice::Defer };
    for slug in slugs {
        let _ = check(vault, &slug, mode);
    }
}

/// Which workflows [`verify`] would refuse right now, and why: `review` (its
/// settings or prompt differ from the approved stamp, it has none, or the
/// stamp was tombstoned) or `unavailable` (the stamp store can't be read, so
/// every automation is paused). Read-only: no review is created, refreshed or
/// retired, and nothing is announced. Files that can't be parsed are left out
/// (they fail as configuration errors, not refusals). Feeds
/// `Workflow.permissionHold` in `list_workflows` / `read_workflow`, so the
/// Automations page labels a paused automation from the backend's own rule
/// rather than guessing from the review queue (a user rejection stays in the
/// queue after the file is restored).
pub fn permission_holds(vault: &Path) -> BTreeMap<String, &'static str> {
    let _guard = write_guard();
    let approvals = approvals(vault);
    let mut out = BTreeMap::new();
    for w in vault_workflows::list(vault) {
        let Ok(full) = vault::safe_join(vault, &workflow_rel(&w.slug)) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(full) else {
            continue;
        };
        let Some((observed, _)) = PermissionFields::from_content(&w.slug, &content) else {
            continue;
        };
        let hold = match &approvals {
            Err(_) => Some(HOLD_UNAVAILABLE),
            Ok(approvals) => {
                (approvals.live.get(&w.slug) != Some(&observed)).then_some(HOLD_REVIEW)
            }
        };
        if let Some(hold) = hold {
            out.insert(w.slug, hold);
        }
    }
    out
}

/// `Workflow.permissionHold` values (mirrored in `src/types.ts`).
pub const HOLD_REVIEW: &str = "review";
pub const HOLD_UNAVAILABLE: &str = "unavailable";

// ── Verify ───────────────────────────────────────────────────────────────────

/// The workflow exactly as verified: run it from this value, never from a
/// second read of the file.
#[derive(Debug, Clone)]
pub struct VerifiedWorkflow {
    pub workflow: Workflow,
    pub prompt: String,
}

/// Why a workflow may not run. Typed so a scheduler never has to parse text:
/// [`VerifyError::holds_fire`] tells it to keep a trigger fire / retry within
/// grace rather than consume it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The file differs from its approved stamp. A review exists (or the user
    /// rejected this exact change). Starts with [`REFUSAL_PREFIX`].
    Refused(String),
    /// The stamp store can't be read; nothing was overwritten or queued.
    Unavailable(String),
    /// The workflow file does not exist.
    Missing(String),
    /// The file exists but can't be read or parsed, or the slug is invalid.
    Parse(String),
}

impl VerifyError {
    pub fn message(&self) -> &str {
        match self {
            Self::Refused(m) | Self::Unavailable(m) | Self::Missing(m) | Self::Parse(m) => m,
        }
    }

    /// True for a permission refusal (a review exists or was rejected).
    pub fn is_refusal(&self) -> bool {
        matches!(self, Self::Refused(_))
    }

    /// True when the workflow itself is intact but may not run yet: refused,
    /// or its permission records are unreadable. The run-queue claim holds
    /// the row for these (`workflows::PrecheckFailure::Refused`): deferred
    /// with its `held_reason`, keeping the occurrence and the fire, never
    /// labelled a misfire or a wait.
    pub fn holds_fire(&self) -> bool {
        matches!(self, Self::Refused(_) | Self::Unavailable(_))
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl From<VerifyError> for String {
    fn from(error: VerifyError) -> Self {
        error.message().to_string()
    }
}

/// Verify a workflow against its stamp before running it. On a mismatch the
/// single live review is created, refreshed or reopened, and one refusal
/// notification (rate-limited per workflow) is sent when this call opened the
/// review or the review was opened quietly and never announced. Unreadable
/// permission records send one rate-limited notice per vault. Called by
/// `workflows::precheck` for every admission. The write guard is released
/// before this returns, so a caller may take the history lease next.
pub fn verify(vault: &Path, slug: &str) -> Result<VerifiedWorkflow, VerifyError> {
    check(vault, slug, Notice::Send)
}

/// [`verify`] without a notification, for a check whose refusal someone is
/// already being told about: the post-run flag check (that run's own
/// notification covers it) and a stale approval (the user is looking at the
/// panel). The review then counts as announced.
pub fn verify_quiet(vault: &Path, slug: &str) -> Result<VerifiedWorkflow, VerifyError> {
    check(vault, slug, Notice::Covered)
}

/// What a check does about telling the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Notice {
    /// Announce a review this call opened, or one opened quietly earlier.
    Send,
    /// Stay quiet, but remember a review this call opened for the next
    /// [`Notice::Send`] (the watcher's sweep).
    Defer,
    /// Stay quiet and mark the review announced ([`verify_quiet`]).
    Covered,
}

struct RefusalNotice {
    title: String,
    summary: String,
    fingerprint: String,
    opened: bool,
}

struct Refusal {
    error: VerifyError,
    notice: Option<RefusalNotice>,
}

fn check(vault: &Path, slug: &str, mode: Notice) -> Result<VerifiedWorkflow, VerifyError> {
    match check_inner(vault, slug) {
        Ok(verified) => Ok(verified),
        Err(Refusal { error, notice }) => {
            match notice {
                Some(notice) => {
                    let key = (canonical_vault(vault), notice.fingerprint.clone());
                    match mode {
                        Notice::Send => {
                            let deferred = lock(&UNANNOUNCED).remove(&key);
                            if (notice.opened || deferred) && notice_due(vault, slug) {
                                send(
                                    vault,
                                    refusal_notification(&notice.title, slug, &notice.summary),
                                );
                            }
                        }
                        Notice::Defer => {
                            if notice.opened {
                                lock(&UNANNOUNCED).insert(key);
                            }
                        }
                        Notice::Covered => {
                            lock(&UNANNOUNCED).remove(&key);
                        }
                    }
                }
                None => {
                    if let (Notice::Send, VerifyError::Unavailable(message)) = (mode, &error) {
                        report_unavailable(vault, message);
                    }
                }
            }
            Err(error)
        }
    }
}

/// Tell the user that every automation is paused because the stamp store
/// can't be read: one warning per vault per [`NOTICE_INTERVAL`] (the store
/// path and repair steps are in `message`), and a health record. Called by a
/// notify-capable [`verify`] and by startup when [`initialize`] fails.
pub(crate) fn report_unavailable(vault: &Path, message: &str) {
    if !notice_due(vault, STORE_NOTICE_KEY) {
        return;
    }
    crate::health::record(
        vault,
        "workflows",
        crate::health::Outcome::Failed,
        message.to_string(),
    );
    send(
        vault,
        NotificationInput {
            title: "Automations are paused: permission records unavailable".into(),
            body: message.to_string(),
            kind: "error".into(),
            source: Some("workflow-permissions".into()),
            action_label: None,
            action_url: None,
            notify_native: true,
        },
    );
}

/// Send a notification through the registered app handle (none under test,
/// where the notice is recorded for assertions instead).
fn send(vault: &Path, input: NotificationInput) {
    #[cfg(test)]
    lock(&SENT_FOR_TEST).push((canonical_vault(vault), input.title.clone()));
    let _ = notifications::create_registered(vault, input);
}

#[cfg(test)]
static SENT_FOR_TEST: Lazy<Mutex<Vec<(PathBuf, String)>>> = Lazy::new(|| Mutex::new(Vec::new()));

/// Titles of the notifications this module sent for `vault` (tests only).
#[cfg(test)]
pub(crate) fn sent_for_test(vault: &Path) -> Vec<String> {
    let root = canonical_vault(vault);
    lock(&SENT_FOR_TEST)
        .iter()
        .filter(|(v, _)| *v == root)
        .map(|(_, title)| title.clone())
        .collect()
}

/// The guarded read → compare → enqueue. Notifications are sent by the
/// caller after the write guard is released.
fn check_inner(vault: &Path, slug: &str) -> Result<VerifiedWorkflow, Refusal> {
    let plain = |error: VerifyError| Refusal {
        error,
        notice: None,
    };
    if !valid_slug(slug) {
        return Err(plain(VerifyError::Parse(format!(
            "invalid workflow name: {slug}"
        ))));
    }
    let rel = workflow_rel(slug);
    let _guard = write_guard();
    let full = vault::safe_join(vault, &rel).map_err(|e| plain(VerifyError::Parse(e)))?;
    let content = match fs::read_to_string(&full) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Deleted outside the editor (the editor's delete forgets the
            // stamp itself): its approval must not carry over to a file
            // recreated at this path later. A cloud-evicted file keeps it.
            if !missing_is_transient(&full) {
                if let Err(error) = tombstone(vault, slug) {
                    eprintln!("[workflows] could not retire the stamp of deleted {slug}: {error}");
                }
            }
            let _ = review_queue::retire_live(
                vault,
                REVIEW_KIND,
                &rel,
                "Superseded: the automation file no longer exists.",
            );
            return Err(plain(VerifyError::Missing(format!("read {rel}: {e}"))));
        }
        Err(e) => return Err(plain(VerifyError::Parse(format!("read {rel}: {e}")))),
    };
    let (workflow, prompt, _) =
        vault_workflows::parse_content(slug, &content).map_err(|e| plain(VerifyError::Parse(e)))?;
    let observed = PermissionFields::from_workflow(&workflow, &prompt);
    let stamp = stamp_for(vault, slug)
        .map_err(|e| plain(VerifyError::Unavailable(unavailable_message(vault, &e))))?;
    if stamp.as_ref().map(|s| &s.fields) == Some(&observed) {
        let _ = review_queue::retire_live(
            vault,
            REVIEW_KIND,
            &rel,
            "Superseded: this automation matches its approved settings again.",
        );
        return Ok(VerifiedWorkflow { workflow, prompt });
    }

    let input = review_input(slug, &workflow.title, stamp.as_ref(), &observed, &prompt);
    let summary = input
        .payload
        .get("summary")
        .and_then(Value::as_str)
        .unwrap_or("settings changed")
        .to_string();
    let new_file = stamp.is_none();
    let title = workflow.title;
    match review_queue::enqueue_exclusive(vault, input, "Superseded: the automation changed again.")
    {
        Ok(outcome) => {
            let message = if is_user_rejection(&outcome.item) {
                rejected_message(&title)
            } else {
                pending_message(&title, &summary, new_file)
            };
            Err(Refusal {
                error: VerifyError::Refused(message),
                notice: Some(RefusalNotice {
                    title,
                    summary,
                    fingerprint: outcome.item.fingerprint.clone(),
                    opened: outcome.opened,
                }),
            })
        }
        Err(e) => Err(plain(VerifyError::Refused(format!(
            "{} (Hollow could not record the review request: {e})",
            pending_message(&title, &summary, new_file)
        )))),
    }
}

fn is_user_rejection(item: &ReviewItem) -> bool {
    item.status == "rejected"
        && !item
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with(review_queue::SUPERSEDED_PREFIX))
}

fn pending_message(title: &str, summary: &str, new_file: bool) -> String {
    let what = if new_file {
        "was created outside Hollow's automation editor"
    } else {
        "was changed outside Hollow's automation editor"
    };
    format!(
        "{REFUSAL_PREFIX} “{title}” {what} ({summary}). It will not run until you approve \
         the change in Automations → Permission changes."
    )
}

fn rejected_message(title: &str) -> String {
    format!(
        "{REFUSAL_PREFIX} you rejected the change to “{title}”. Restore the approved version \
         in Library → History, or save a corrected version from the automation \
         editor to request a new review."
    )
}

fn notice_due(vault: &Path, slug: &str) -> bool {
    let key = (canonical_vault(vault), slug.to_string());
    let mut last = lock(&LAST_NOTICE);
    let now = Instant::now();
    match last.get(&key) {
        Some(at) if now.duration_since(*at) < NOTICE_INTERVAL => false,
        _ => {
            last.insert(key, now);
            true
        }
    }
}

/// The notification a newly opened review sends.
pub(crate) fn refusal_notification(title: &str, slug: &str, summary: &str) -> NotificationInput {
    NotificationInput {
        title: format!("Automation “{title}” is paused for review"),
        body: format!("{summary}. Approve or reject it in Automations → Permission changes."),
        kind: "warning".into(),
        source: Some("workflow-permissions".into()),
        action_label: Some("Review".into()),
        action_url: Some(format!("workflow-permissions:{slug}")),
        notify_native: true,
    }
}

// ── Approve ──────────────────────────────────────────────────────────────────

/// Accept a permission review. Everything is re-derived from disk: the queue
/// lives in the agent-writable vault, so a forged or stale card can never be
/// approved. Payload edits are refused for this kind.
///
/// `expected_fingerprint` is the fingerprint of the card the user confirmed
/// (required). The approval is bound to it, not just to the id: the queue
/// does not enforce unique ids, so an agent could plant a forged item for
/// another automation under the id of a real card. More than one item with
/// the id (of any kind) is refused outright, and so is a card whose
/// fingerprint changed after the user saw it.
pub fn approve(
    vault: &Path,
    item: &ReviewItem,
    edited: Option<&Value>,
    expected_fingerprint: Option<&str>,
) -> Result<(), String> {
    if edited.is_some() {
        return Err("Permission reviews cannot be edited.".into());
    }
    if item.kind != REVIEW_KIND {
        return Err(format!(
            "review item {} is not an automation permission review",
            item.id
        ));
    }
    let expected = expected_fingerprint
        .map(str::trim)
        .filter(|fp| !fp.is_empty())
        .ok_or_else(|| {
            "This approval did not say which change it approves. Refresh Automations and try again."
                .to_string()
        })?;
    let (slug, changed_again) = {
        let _guard = write_guard();
        let mut same_id: Vec<ReviewItem> = review_queue::list(vault, None)?
            .into_iter()
            .filter(|candidate| candidate.id == item.id)
            .collect();
        if same_id.len() > 1 {
            return Err(format!(
                "{} review items share the id {}: the review queue was edited outside Hollow. \
                 Reject them and let Hollow re-create the request.",
                same_id.len(),
                item.id
            ));
        }
        let current = same_id
            .pop()
            .ok_or_else(|| format!("review item not found: {}", item.id))?;
        if current.kind != REVIEW_KIND {
            return Err(format!(
                "review item {} is not an automation permission review",
                item.id
            ));
        }
        if !current.fingerprint.eq_ignore_ascii_case(expected) {
            return Err(
                "This review changed after you opened it. Review the current request.".into(),
            );
        }
        if current.status != "pending" {
            return Err(format!(
                "This review is {}; only a pending review can be approved.",
                current.status
            ));
        }
        let slug = current
            .payload
            .get("slug")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|s| valid_slug(s))
            .ok_or_else(|| "This permission review has no valid automation name.".to_string())?;
        let rel = workflow_rel(&slug);
        if current.source_path.as_deref() != Some(rel.as_str()) {
            return Err("This permission review does not match its automation file.".into());
        }
        let full = vault::safe_join(vault, &rel)?;
        let content = match fs::read_to_string(&full) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let _ = review_queue::retire_live(
                    vault,
                    REVIEW_KIND,
                    &rel,
                    "Superseded: the automation file no longer exists.",
                );
                return Err("This automation no longer exists.".into());
            }
            Err(e) => return Err(format!("read {rel}: {e}")),
        };
        let (workflow, prompt, _) = vault_workflows::parse_content(&slug, &content)?;
        let observed = PermissionFields::from_workflow(&workflow, &prompt);
        let stamp = stamp_for(vault, &slug).map_err(|e| unavailable_message(vault, &e))?;
        if stamp.as_ref().map(|s| &s.fields) == Some(&observed) {
            let _ = review_queue::retire_live(
                vault,
                REVIEW_KIND,
                &rel,
                "Superseded: this automation already matches its approved settings.",
            );
            return Err(
                "This automation already matches its approved settings; nothing to approve.".into(),
            );
        }
        let canonical = review_input(&slug, &workflow.title, stamp.as_ref(), &observed, &prompt);
        let changed_again = canonical.fingerprint.to_lowercase() != current.fingerprint
            || without_title(&canonical.payload) != without_title(&current.payload);
        if changed_again {
            let _ = review_queue::retire_live(
                vault,
                REVIEW_KIND,
                &rel,
                "Superseded: the automation changed again before approval.",
            );
        } else {
            review_queue::claim(vault, &current.id, None)?;
            if let Err(e) = put_stamp(vault, &slug, observed, &prompt, "review", false) {
                let _ = review_queue::fail(vault, &current.id, &e);
                return Err(e);
            }
            review_queue::accept(vault, &current.id)?;
        }
        (slug, changed_again)
    };
    if changed_again {
        // Recreate the canonical review for what is on disk now.
        let _ = verify_quiet(vault, &slug);
        return Err(
            "This automation changed again after the request was created. Review the newer request."
                .into(),
        );
    }
    Ok(())
}

/// An automation saved through Hollow app tools after the user allowed a
/// card showing `shown` (`vault_workflows::preview_input`'s `after`; PR A3):
/// stamp exactly those fields the way [`approve`] does (`"review"`), when
/// the file on disk carries them, and retire the path's live reviews. When
/// the file differs from what the card showed (it changed in between), no
/// stamp is written: `verify_quiet` queues the ordinary review and the
/// error says so.
pub(crate) fn approve_app_save(
    vault: &Path,
    slug: &str,
    shown: &PermissionFields,
) -> Result<(), String> {
    let rel = workflow_rel(slug);
    {
        let _guard = write_guard();
        let content = fs::read_to_string(vault::safe_join(vault, &rel)?)
            .map_err(|e| format!("read {rel}: {e}"))?;
        if let Some((observed, prompt)) = PermissionFields::from_content(slug, &content) {
            if observed == *shown {
                put_stamp(vault, slug, observed, &prompt, "review", true)?;
                let _ = review_queue::retire_live(
                    vault,
                    REVIEW_KIND,
                    &rel,
                    "Superseded: approved on a Hollow app card.",
                );
                return Ok(());
            }
        }
    }
    let _ = verify_quiet(vault, slug);
    Err("Saved, but the automation changed after the card was shown, so its settings wait for approval in Automations → Permission changes.".into())
}

fn without_title(value: &Value) -> Value {
    let mut value = value.clone();
    if let Some(object) = value.as_object_mut() {
        object.remove("title");
    }
    value
}

// ── Owned-writer hooks (the caller holds `write_guard`) ─────────────────────

/// A new automation file written by Hollow (editor create, setup scaffold):
/// stamp it, overwriting any stale stamp at that slug, and retire its live
/// reviews. Failures are logged; the workflow is then simply refused.
pub(crate) fn stamp_created_locked(vault: &Path, slug: &str, content: &str, agent: bool) {
    match PermissionFields::from_content(slug, content) {
        Some((fields, prompt)) => {
            if let Err(e) = put_stamp(vault, slug, fields, &prompt, "create", agent) {
                eprintln!("[workflows] could not stamp new automation {slug}: {e}");
            }
        }
        None => eprintln!("[workflows] could not stamp new automation {slug}: unparseable"),
    }
    let _ = review_queue::retire_live(
        vault,
        REVIEW_KIND,
        &workflow_rel(slug),
        "Superseded: a new automation was created at this path.",
    );
}

/// The automation editor's save (`vault_workflows::upsert_input`), called
/// with the prior bytes of the attempt that actually wrote (`before`).
/// - create (no previous slug): stamp.
/// - rename: move the stamp and retire the old path's live reviews. The old
///   key is removed, so a run tracked across the rename does not see the old
///   path's disappearance as a delete.
/// - save: re-stamp only when the replaced file (`before`) still matched its
///   stamp; with no stamp and no prior file, treat it as a create. Anything
///   else (an agent-created unstamped file, an unparseable prior, a mismatch)
///   leaves the stamps alone, so a UI save never approves a tamper.
pub(crate) fn after_upsert_locked(
    vault: &Path,
    prev_slug: Option<&str>,
    new_slug: &str,
    before: Option<&str>,
    after: &str,
    by_agent: bool,
) {
    let Some(prev) = prev_slug else {
        stamp_created_locked(vault, new_slug, after, by_agent);
        return;
    };
    let renamed = prev != new_slug;
    if renamed {
        if let Err(e) = move_stamp(vault, prev, new_slug) {
            eprintln!("[workflows] could not move the stamp of {prev} to {new_slug}: {e}");
        }
        let _ = review_queue::retire_live(
            vault,
            REVIEW_KIND,
            &workflow_rel(prev),
            "Superseded: the automation was renamed.",
        );
    }
    let stamp = match stamp_for(vault, new_slug) {
        Ok(stamp) => stamp,
        Err(e) => {
            eprintln!("[workflows] permission stamps unavailable while saving {new_slug}: {e}");
            return;
        }
    };
    let restamp = match (&stamp, before) {
        (Some(stamp), Some(before)) => PermissionFields::from_content(new_slug, before)
            .is_some_and(|(pre, _)| pre == stamp.fields),
        (None, None) => true,
        _ => false,
    };
    if !restamp {
        return;
    }
    let Some((fields, prompt)) = PermissionFields::from_content(new_slug, after) else {
        eprintln!("[workflows] could not stamp saved automation {new_slug}: unparseable");
        return;
    };
    let by = match (&stamp, renamed) {
        (None, _) => "create",
        (Some(_), true) => "rename",
        (Some(_), false) => "save",
    };
    if let Err(e) = put_stamp(vault, new_slug, fields, &prompt, by, by_agent) {
        eprintln!("[workflows] could not re-stamp {new_slug}: {e}");
        return;
    }
    let _ = review_queue::retire_live(
        vault,
        REVIEW_KIND,
        &workflow_rel(new_slug),
        "Superseded: the automation was saved from the automation editor.",
    );
}

/// A routine toggle (`set_schedule`): when the stamp equals
/// the pre-write fields except schedule, adopt the post-write schedule. Never
/// approves any other change.
pub(crate) fn after_schedule_write_locked(vault: &Path, slug: &str, before: &str, after: &str) {
    let (Some((pre, _)), Some((post, _))) = (
        PermissionFields::from_content(slug, before),
        PermissionFields::from_content(slug, after),
    ) else {
        return;
    };
    if !post.same_except_schedule(&pre) {
        return;
    }
    let healed = with_store(vault, |file| {
        // A tombstoned stamp approves nothing, so a toggle can't revive it.
        let Some(stamp) = file
            .workflows
            .get_mut(slug)
            .filter(|stamp| stamp.deleted_at.is_none())
        else {
            return Ok((false, false));
        };
        if !stamp.fields.same_except_schedule(&pre) {
            return Ok((false, false));
        }
        if stamp.fields.schedule == post.schedule {
            return Ok((true, false));
        }
        stamp.fields.schedule = post.schedule.clone();
        stamp.stamped_at = Utc::now().to_rfc3339();
        stamp.by = "schedule".into();
        Ok((true, true))
    });
    match healed {
        Ok(true) => {
            let _ = review_queue::retire_live(
                vault,
                REVIEW_KIND,
                &workflow_rel(slug),
                "Superseded: the schedule was set from Hollow.",
            );
        }
        Ok(false) => {}
        Err(e) => eprintln!("[workflows] could not update the schedule stamp of {slug}: {e}"),
    }
}

/// M4: `vault_workflows::set_owner` (this Mac's slug renamed): move only
/// the `runs_on` of an otherwise-matching, live stamp, as
/// [`after_schedule_write_locked`] does for a schedule. A stamp that
/// already differs elsewhere is left for review.
pub(crate) fn after_owner_write_locked(vault: &Path, slug: &str, before: &str, after: &str) {
    let (Some((pre, _)), Some((post, _))) = (
        PermissionFields::from_content(slug, before),
        PermissionFields::from_content(slug, after),
    ) else {
        return;
    };
    if !post.same_except_runs_on(&pre) {
        eprintln!(
            "[workflows] {slug}: its owner moved with other changes; its stamp is left for review"
        );
        return;
    }
    let moved = with_store(vault, |file| {
        let Some(stamp) = file
            .workflows
            .get_mut(slug)
            .filter(|stamp| stamp.deleted_at.is_none())
        else {
            eprintln!("[workflows] {slug}: no live stamp, so its owner move is left for review");
            return Ok((false, false));
        };
        if stamp.fields != pre {
            eprintln!(
                "[workflows] {slug}: its stamp already differed, so the owner move is left for review"
            );
            return Ok((false, false));
        }
        stamp.fields.runs_on = post.runs_on.clone();
        stamp.stamped_at = Utc::now().to_rfc3339();
        stamp.by = "owner".into();
        Ok((true, true))
    });
    if let Err(e) = moved {
        eprintln!("[workflows] could not update the owner stamp of {slug}: {e}");
    }
}

/// `vault_workflows::delete`: drop the stamp and retire live reviews.
pub(crate) fn forget_locked(vault: &Path, slug: &str) {
    if let Err(e) = remove_stamp(vault, slug) {
        eprintln!("[workflows] could not remove the stamp of {slug}: {e}");
    }
    let _ = review_queue::retire_live(
        vault,
        REVIEW_KIND,
        &workflow_rel(slug),
        "Superseded: the automation was deleted.",
    );
}

// ── Review payload ───────────────────────────────────────────────────────────

fn review_input(
    slug: &str,
    title: &str,
    stamp: Option<&Stamp>,
    observed: &PermissionFields,
    observed_prompt: &str,
) -> NewReviewItem {
    let approved_digest = stamp.map(|s| s.fields.digest());
    NewReviewItem {
        kind: REVIEW_KIND.into(),
        fingerprint: format!(
            "workflow-permissions:{slug}:{}:{}",
            approved_digest.as_deref().unwrap_or("none"),
            observed.digest()
        )
        .to_lowercase(),
        payload: review_payload(slug, title, stamp, observed, observed_prompt),
        risk: "local_write".into(),
        source: REVIEW_SOURCE.into(),
        source_path: Some(workflow_rel(slug)),
    }
}

/// Deterministic (no timestamps) review payload:
/// `{slug, title, path, observedDigest, approvedDigest, observed, approved,
/// changedFields, escalation, summary, diff}`.
fn review_payload(
    slug: &str,
    title: &str,
    stamp: Option<&Stamp>,
    observed: &PermissionFields,
    observed_prompt: &str,
) -> Value {
    let approved = stamp.map(|s| &s.fields);
    json!({
        "slug": slug,
        "title": title,
        "path": workflow_rel(slug),
        "observedDigest": observed.digest(),
        "approvedDigest": approved.map(PermissionFields::digest),
        "observed": observed,
        "approved": approved,
        "changedFields": PermissionFields::changed_fields(approved, observed),
        "escalation": PermissionFields::escalation(approved, observed),
        "summary": summary_line(approved, observed),
        "diff": diff_entries(stamp, observed, observed_prompt),
    })
}

fn cap(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('…');
    out
}

fn list_diff(before: &[String], after: &[String]) -> (Vec<String>, Vec<String>, bool) {
    let added: Vec<String> = after
        .iter()
        .filter(|entry| !before.contains(entry))
        .cloned()
        .collect();
    let removed: Vec<String> = before
        .iter()
        .filter(|entry| !after.contains(entry))
        .cloned()
        .collect();
    let reordered = added.is_empty() && removed.is_empty() && before != after;
    (added, removed, reordered)
}

fn diff_entries(
    stamp: Option<&Stamp>,
    observed: &PermissionFields,
    observed_prompt: &str,
) -> Vec<Value> {
    let approved = stamp.map(|s| &s.fields);
    let mut out = Vec::new();
    let scalars: [(&str, Option<&str>, Option<&str>); 8] = [
        (
            "permission mode",
            approved.map(|a| a.permission_mode.as_str()),
            Some(observed.permission_mode.as_str()),
        ),
        (
            "working folder",
            approved.and_then(|a| a.cwd.as_deref()),
            observed.cwd.as_deref(),
        ),
        (
            "agent",
            approved.and_then(|a| a.agent.as_deref()),
            observed.agent.as_deref(),
        ),
        (
            "model",
            approved.and_then(|a| a.model.as_deref()),
            observed.model.as_deref(),
        ),
        (
            "schedule",
            approved.map(|a| a.schedule.as_str()),
            Some(observed.schedule.as_str()),
        ),
        (
            "runs in",
            approved.and_then(|a| a.in_chat.as_deref()),
            observed.in_chat.as_deref(),
        ),
        (
            "runs on",
            approved.and_then(|a| a.runs_on.as_deref()),
            observed.runs_on.as_deref(),
        ),
        (
            "precheck",
            approved.and_then(|a| a.precheck.as_deref()),
            observed.precheck.as_deref(),
        ),
    ];
    for (field, before, after) in scalars {
        let changed = match approved {
            None => after.is_some(),
            Some(_) => before != after,
        };
        if changed {
            out.push(json!({
                "field": field,
                "kind": "scalar",
                "before": before.map(|v| cap(v, SCALAR_CAP)),
                "after": after.map(|v| cap(v, SCALAR_CAP)),
            }));
        }
    }
    let empty: Vec<String> = Vec::new();
    let lists: [(&str, Option<&Vec<String>>, &Vec<String>); 3] = [
        (
            "allowed tools",
            approved.map(|a| &a.allowed_tools),
            &observed.allowed_tools,
        ),
        (
            "disallowed tools",
            approved.map(|a| &a.disallowed_tools),
            &observed.disallowed_tools,
        ),
        (
            "triggers",
            approved.map(|a| &a.triggers),
            &observed.triggers,
        ),
    ];
    for (field, before, after) in lists {
        let before = before.unwrap_or(&empty);
        if before == after {
            continue;
        }
        let (added, removed, reordered) = list_diff(before, after);
        let capped = |entries: Vec<String>| -> Vec<String> {
            entries.iter().map(|e| cap(e, SCALAR_CAP)).collect()
        };
        out.push(json!({
            "field": field,
            "kind": "list",
            "added": capped(added),
            "removed": capped(removed),
            "reordered": reordered,
        }));
    }
    let prompt_changed = approved
        .map(|a| a.prompt_sha256 != observed.prompt_sha256)
        .unwrap_or(true);
    if prompt_changed {
        let before_text = if approved.is_some() {
            stamp.and_then(|s| s.prompt.as_deref())
        } else {
            None
        };
        let (before, after, truncated) = prompt_excerpt(before_text, observed_prompt.trim());
        out.push(json!({
            "field": "prompt",
            "kind": "text",
            "before": before,
            "after": after,
            "truncated": truncated,
        }));
    }
    out
}

/// Bounded before/after excerpt around the changed lines: the common line
/// prefix and suffix are cut to two context lines each, and each side is
/// capped at 2,000 characters. `before` is None when the approved text is
/// unavailable (a new automation, or a stamp whose prompt exceeded the cap).
fn prompt_excerpt(before: Option<&str>, after: &str) -> (Option<String>, String, bool) {
    let Some(before) = before else {
        let (text, capped) = cap_block(after);
        return (None, text, capped);
    };
    let b: Vec<&str> = before.lines().collect();
    let a: Vec<&str> = after.lines().collect();
    let mut prefix = 0;
    while prefix < b.len() && prefix < a.len() && b[prefix] == a[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < b.len() - prefix
        && suffix < a.len() - prefix
        && b[b.len() - 1 - suffix] == a[a.len() - 1 - suffix]
    {
        suffix += 1;
    }
    let start = prefix.saturating_sub(PROMPT_CONTEXT_LINES);
    let b_end = (b.len() - suffix + PROMPT_CONTEXT_LINES).min(b.len());
    let a_end = (a.len() - suffix + PROMPT_CONTEXT_LINES).min(a.len());
    let (before_text, before_cut) = excerpt(&b, start, b_end);
    let (after_text, after_cut) = excerpt(&a, start, a_end);
    (Some(before_text), after_text, before_cut || after_cut)
}

fn excerpt(lines: &[&str], start: usize, end: usize) -> (String, bool) {
    let mut text = lines[start..end].join("\n");
    let mut cut = false;
    if start > 0 {
        text = format!("…\n{text}");
        cut = true;
    }
    if end < lines.len() {
        text = format!("{text}\n…");
        cut = true;
    }
    let (text, capped) = cap_block(&text);
    (text, cut || capped)
}

fn cap_block(text: &str) -> (String, bool) {
    if text.chars().count() <= PROMPT_EXCERPT_CAP {
        (text.to_string(), false)
    } else {
        (cap(text, PROMPT_EXCERPT_CAP), true)
    }
}

fn bounded_entries(entries: &[String], sign: &str) -> String {
    let mut parts: Vec<String> = entries
        .iter()
        .take(SUMMARY_LIST_ITEMS)
        .map(|e| format!("{sign}{}", cap(e, 80)))
        .collect();
    if entries.len() > SUMMARY_LIST_ITEMS {
        parts.push(format!("(+{} more)", entries.len() - SUMMARY_LIST_ITEMS));
    }
    parts.join(" ")
}

fn summary_line(approved: Option<&PermissionFields>, observed: &PermissionFields) -> String {
    let mut parts: Vec<String> = Vec::new();
    let show = |v: Option<&str>| v.map(|s| cap(s, 80)).unwrap_or_else(|| "(none)".into());
    match approved {
        None => {
            parts.push("New automation file".into());
            parts.push(format!("Permission mode: {}", observed.permission_mode));
            for (label, entries) in [
                ("Allowed tools", &observed.allowed_tools),
                ("Disallowed tools", &observed.disallowed_tools),
                ("Triggers", &observed.triggers),
            ] {
                if !entries.is_empty() {
                    parts.push(format!("{label} {}", bounded_entries(entries, "")));
                }
            }
            if observed.schedule != "manual" {
                parts.push(format!("Schedule: {}", observed.schedule));
            }
            if let Some(cwd) = observed.cwd.as_deref() {
                parts.push(format!("Working folder: {}", cap(cwd, 80)));
            }
            if let Some(agent) = observed.agent.as_deref() {
                parts.push(format!("Agent: {agent}"));
            }
            if let Some(model) = observed.model.as_deref() {
                parts.push(format!("Model: {model}"));
            }
            if let Some(key) = observed.in_chat.as_deref() {
                parts.push(format!("Runs in chat: {key}"));
            }
            if let Some(owner) = observed.runs_on.as_deref() {
                parts.push(format!("Runs on: {owner}"));
            }
            if let Some(command) = observed.precheck.as_deref() {
                parts.push(format!("Precheck: {}", cap(command, 80)));
                if let Some(note) = crate::workflow_precheck::uncovered_note(
                    command,
                    observed
                        .cwd
                        .as_deref()
                        .filter(|cwd| !cwd.is_empty())
                        .map(Path::new),
                ) {
                    parts.push(format!("Precheck {note}"));
                }
            }
        }
        Some(a) => {
            for (label, before, after) in [
                (
                    "Permission mode",
                    Some(a.permission_mode.as_str()),
                    Some(observed.permission_mode.as_str()),
                ),
                ("Working folder", a.cwd.as_deref(), observed.cwd.as_deref()),
                ("Agent", a.agent.as_deref(), observed.agent.as_deref()),
                ("Model", a.model.as_deref(), observed.model.as_deref()),
                (
                    "Schedule",
                    Some(a.schedule.as_str()),
                    Some(observed.schedule.as_str()),
                ),
                (
                    "Runs in chat",
                    a.in_chat.as_deref(),
                    observed.in_chat.as_deref(),
                ),
                ("Runs on", a.runs_on.as_deref(), observed.runs_on.as_deref()),
                (
                    "Precheck",
                    a.precheck.as_deref(),
                    observed.precheck.as_deref(),
                ),
            ] {
                if before != after {
                    parts.push(format!("{label}: {} → {}", show(before), show(after)));
                }
            }
            for (label, before, after) in [
                ("Allowed tools", &a.allowed_tools, &observed.allowed_tools),
                (
                    "Disallowed tools",
                    &a.disallowed_tools,
                    &observed.disallowed_tools,
                ),
                ("Triggers", &a.triggers, &observed.triggers),
            ] {
                if before == after {
                    continue;
                }
                let (added, removed, reordered) = list_diff(before, after);
                if reordered {
                    parts.push(format!("{label} reordered"));
                    continue;
                }
                let mut clause = label.to_string();
                if !added.is_empty() {
                    clause.push(' ');
                    clause.push_str(&bounded_entries(&added, "+"));
                }
                if !removed.is_empty() {
                    clause.push(' ');
                    clause.push_str(&bounded_entries(&removed, "−"));
                }
                // An empty allowlist means every tool is allowed.
                if label == "Allowed tools" {
                    if before.is_empty() {
                        clause.push_str(" (was unrestricted)");
                    } else if after.is_empty() {
                        clause.push_str(" (now unrestricted)");
                    }
                }
                parts.push(clause);
            }
            if a.prompt_sha256 != observed.prompt_sha256 {
                parts.push("Prompt changed".into());
            }
        }
    }
    cap(&parts.join(" · "), SUMMARY_CAP)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::RunLogEntry;
    use tempfile::TempDir;

    const CRON: &str = "0 0 7 * * Mon-Fri *";

    fn vault() -> TempDir {
        tempfile::Builder::new()
            .prefix("hollow-wfperm-")
            .tempdir()
            .unwrap()
    }

    fn create(v: &Path, title: &str, prompt: &str) -> Workflow {
        vault_workflows::upsert(
            v,
            None,
            title,
            "desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            prompt,
        )
        .unwrap()
    }

    /// The automation editor saving `w` in place with the values it would
    /// show after loading the current file (i.e. "save without changes").
    fn ui_save_as_loaded(v: &Path, slug: &str) -> Workflow {
        let (w, prompt, _) = vault_workflows::read_full(v, slug).unwrap();
        vault_workflows::upsert(
            v,
            Some(slug),
            &w.title,
            &w.description,
            &w.schedule,
            w.agent.as_deref(),
            w.model.as_deref(),
            w.cwd.as_deref(),
            &w.permission_mode,
            &w.allowed_tools,
            &w.disallowed_tools,
            w.output_glob.as_deref(),
            &prompt,
        )
        .unwrap()
    }

    fn path_of(v: &Path, slug: &str) -> PathBuf {
        v.join(format!("workflows/{slug}.md"))
    }

    fn read(v: &Path, slug: &str) -> String {
        fs::read_to_string(path_of(v, slug)).unwrap()
    }

    fn write(v: &Path, slug: &str, content: &str) {
        fs::create_dir_all(v.join("workflows")).unwrap();
        fs::write(path_of(v, slug), content).unwrap();
    }

    fn tamper(v: &Path, slug: &str, from: &str, to: &str) {
        let content = read(v, slug);
        assert!(
            content.contains(from),
            "tamper anchor {from:?} missing in {content}"
        );
        write(v, slug, &content.replacen(from, to, 1));
    }

    fn items(v: &Path) -> Vec<ReviewItem> {
        review_queue::list(v, Some(REVIEW_KIND)).unwrap()
    }

    fn pending(v: &Path) -> Vec<ReviewItem> {
        items(v)
            .into_iter()
            .filter(|i| i.status == "pending")
            .collect()
    }

    fn superseded(v: &Path) -> Vec<ReviewItem> {
        items(v)
            .into_iter()
            .filter(|i| {
                i.status == "rejected"
                    && i.error
                        .as_deref()
                        .is_some_and(|e| e.starts_with(review_queue::SUPERSEDED_PREFIX))
            })
            .collect()
    }

    fn only_pending(v: &Path) -> ReviewItem {
        let p = pending(v);
        assert_eq!(
            p.len(),
            1,
            "expected exactly one pending review, got {p:#?}"
        );
        p.into_iter().next().unwrap()
    }

    fn refused(result: Result<VerifiedWorkflow, VerifyError>) -> String {
        match result {
            Ok(_) => panic!("expected a refusal"),
            Err(VerifyError::Refused(message)) => {
                assert!(message.starts_with(REFUSAL_PREFIX), "{message}");
                message
            }
            Err(other) => panic!("expected Refused, got {other:?}"),
        }
    }

    const BASE: &str = "---\ntitle: T\ndescription: D\nschedule: manual\npermission_mode: acceptEdits\noutput_glob: out/*.md\nroutine: plan-day\nmisfire_grace_min: 30\nlast_run_at: \"2026-01-01T00:00:00Z\"\nlast_status: success\nnext_run_at: \"2026-01-02T00:00:00Z\"\n---\n\nBODY\n\n## Run log\n\n_no runs yet_\n";

    fn fields(content: &str) -> PermissionFields {
        PermissionFields::from_content("t", content).unwrap().0
    }

    #[test]
    fn store_path_stays_out_of_the_real_app_data_dir_and_the_vault() {
        let dir = vault();
        let path = store_path(dir.path()).expect("a store path");
        assert!(
            path.starts_with(std::env::temp_dir()),
            "test store should live under the temp dir, got {path:?}"
        );
        if let Some(real) = dirs::data_dir() {
            assert!(
                !path.starts_with(real.join("Hollow")),
                "test store must not sit under the live app data dir, got {path:?}"
            );
        }
        assert!(!path.starts_with(dir.path()));
        assert!(!path.starts_with(canonical_vault(dir.path())));
    }

    #[cfg(unix)]
    #[test]
    fn stamp_store_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = vault();
        create(dir.path(), "Private", "BODY");
        let meta = fs::metadata(store_path(dir.path()).unwrap()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn permission_fields_cover_every_stamped_field_and_ignore_the_rest() {
        let base = fields(BASE);
        let stamped = [
            (
                "permission_mode: acceptEdits",
                "permission_mode: bypassPermissions",
                "permission mode",
            ),
            (
                "schedule: manual",
                "schedule: manual\nallowed_tools: Read, Bash(*)",
                "allowed tools",
            ),
            (
                "schedule: manual",
                "schedule: manual\ndisallowed_tools: Bash",
                "disallowed tools",
            ),
            (
                "schedule: manual",
                "schedule: manual\ncwd: /tmp/elsewhere",
                "working folder",
            ),
            (
                "schedule: manual",
                "schedule: manual\nagent: codex",
                "agent",
            ),
            (
                "schedule: manual",
                "schedule: manual\nmodel: gpt-x",
                "model",
            ),
            (
                "schedule: manual",
                "schedule: manual\ntriggers: file-changed:**/*.md",
                "triggers",
            ),
            ("schedule: manual", "schedule: 0 0 7 * * * *", "schedule"),
            ("\nBODY\n", "\nOTHER BODY\n", "prompt"),
        ];
        for (from, to, label) in stamped {
            let changed = fields(&BASE.replacen(from, to, 1));
            assert_ne!(changed, base, "{label} must be stamped");
            assert_eq!(
                PermissionFields::changed_fields(Some(&base), &changed),
                vec![label],
                "{label}"
            );
        }
        let ignored = [
            ("title: T", "title: Renamed"),
            ("description: D", "description: Other"),
            ("output_glob: out/*.md", "output_glob: elsewhere/*.html"),
            ("routine: plan-day", "routine: close-day"),
            ("misfire_grace_min: 30", "misfire_grace_min: 5"),
            ("last_status: success", "last_status: error"),
            (
                "next_run_at: \"2026-01-02T00:00:00Z\"",
                "next_run_at: \"2027-01-02T00:00:00Z\"",
            ),
            (
                "_no runs yet_",
                "- 2026-01-01T00:00:00Z — success (3s) — ran",
            ),
        ];
        for (from, to) in ignored {
            assert_eq!(
                fields(&BASE.replacen(from, to, 1)),
                base,
                "{to} must not be stamped"
            );
        }
        // An empty or absent permission_mode is the acceptEdits the runner uses.
        assert_eq!(
            fields(&BASE.replacen("permission_mode: acceptEdits", "permission_mode: ", 1)),
            base
        );
        assert_eq!(
            fields(&BASE.replacen("permission_mode: acceptEdits\n", "", 1)),
            base
        );
        // A fake second run-log header yields exactly the prompt the runner reads.
        let dir = vault();
        let fake = BASE.replacen(
            "\nBODY\n",
            "\nBODY\n\n## Run log\n\n- smuggled\n\nMORE PROMPT\n",
            1,
        );
        write(dir.path(), "t", &fake);
        let (_, runner_prompt, _) = vault_workflows::read_full(dir.path(), "t").unwrap();
        let (_, stamped_prompt) = PermissionFields::from_content("t", &fake).unwrap();
        assert_eq!(stamped_prompt, runner_prompt.trim());
        assert_eq!(
            fields(&fake).prompt_sha256,
            sha256_hex(runner_prompt.trim().as_bytes())
        );
    }

    #[test]
    fn status_and_run_log_writers_never_change_permission_fields() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Bookkeeping", "PROMPT BODY");
        let slug = w.slug.clone();
        let original = fields(&read(v, &slug));
        let entry = || RunLogEntry {
            at: Utc::now().to_rfc3339(),
            status: "success".into(),
            duration_sec: 2,
            note: "ran".into(),
            run_id: Some("run-1".into()),
            error_tail: None,
            exit_kind: None,
        };
        let intact = |name: &str| {
            assert_eq!(
                fields(&read(v, &slug)),
                original,
                "{name} changed stamped fields"
            );
            assert!(verify(v, &slug).is_ok(), "{name} broke verification");
        };
        vault_workflows::mark_status(v, &slug, "running", None, None, None).unwrap();
        intact("mark_status");
        vault_workflows::finalize_run(v, &slug, "success", None, 2, None, entry()).unwrap();
        intact("finalize_run");
        // Paperclip PR 8b: a budget-ended run's row (error, stderr tail, the
        // hidden exit marker) is run-log bookkeeping too, never stamped.
        let budget_stop = "Budget limit reached: the heartbeat hit its limit";
        vault_workflows::finalize_run(
            v,
            &slug,
            "error",
            Some(budget_stop),
            2,
            None,
            RunLogEntry {
                status: "error".into(),
                note: budget_stop.into(),
                error_tail: Some("last stderr line".into()),
                exit_kind: Some(vault_workflows::EXIT_KIND_BUDGET.into()),
                ..entry()
            },
        )
        .unwrap();
        intact("finalize_run (budget exit)");
        assert!(read(v, &slug).contains("<!-- hollow-exit: budget -->"));
        vault_workflows::append_run_log(v, &slug, entry()).unwrap();
        intact("append_run_log");
        vault_workflows::refresh_next_run(v, &slug).unwrap();
        intact("refresh_next_run");
        let now = Utc::now();
        vault_workflows::skip_scheduled_run(
            v,
            &slug,
            vault_workflows::missed_while_closed_entry(now),
            now,
        )
        .unwrap();
        intact("skip_scheduled_run (misfire)");
        vault_workflows::skip_scheduled_run(
            v,
            &slug,
            vault_workflows::refused_skip_entry(vault_workflows::PERMISSION_REFUSED_REASON, now),
            now,
        )
        .unwrap();
        intact("skip_scheduled_run (refusal)");
        // Leave a stale `running` status so the heal actually rewrites the file.
        vault_workflows::mark_status(v, &slug, "running", None, None, None).unwrap();
        vault_workflows::heal_orphan_running(v, &[]);
        intact("heal_orphan_running");
        assert!(items(v).is_empty());
    }

    /// Paperclip PR 9 (9a-1): the status and run-log writers are line
    /// patches, so a hand-written file keeps its duplicate keys, its YAML-list
    /// tool allowlist (which the parser reads as empty) and its indented
    /// prompt byte-for-byte, and the stamp keeps matching. A routine toggle
    /// moves only the schedule.
    #[test]
    fn bookkeeping_writes_keep_hand_written_duplicates_lists_and_prompt_indent() {
        let dir = vault();
        let v = dir.path();
        let slug = "dupes";
        let hand_keys = "permission_mode: plan\npermission_mode: acceptEdits\nallowed_tools:\n  - Read\n  - Grep\nowner: alex\n";
        let prompt = "\n---\n\n  Indented first line.\nSecond line.\n\n## Run log";
        write(
            v,
            slug,
            &format!("---\ntitle: Dupes\nschedule: manual\n{hand_keys}---\n\n  Indented first line.\nSecond line.\n\n## Run log\n\n_no runs yet_\n"),
        );
        // Trusted as found, the way an existing vault is stamped at startup.
        initialize(v).unwrap();
        let original = fields(&read(v, slug));
        assert_eq!(
            original.permission_mode, "acceptEdits",
            "the last duplicate wins"
        );
        assert!(
            original.allowed_tools.is_empty(),
            "a YAML list reads as empty"
        );
        let entry = || RunLogEntry {
            at: Utc::now().to_rfc3339(),
            status: "success".into(),
            duration_sec: 2,
            note: "ran".into(),
            run_id: Some("run-1".into()),
            error_tail: None,
            exit_kind: None,
        };
        let intact = |name: &str| {
            let content = read(v, slug);
            assert_eq!(fields(&content), original, "{name} changed stamped fields");
            assert!(
                content.contains(hand_keys),
                "{name} rewrote hand-written keys:\n{content}"
            );
            assert!(
                content.contains(prompt),
                "{name} moved the prompt:\n{content}"
            );
            assert!(verify(v, slug).is_ok(), "{name} broke verification");
        };
        vault_workflows::mark_status(v, slug, "running", None, None, None).unwrap();
        intact("mark_status");
        vault_workflows::finalize_run(v, slug, "success", None, 2, None, entry()).unwrap();
        intact("finalize_run");
        vault_workflows::append_run_log(v, slug, entry()).unwrap();
        intact("append_run_log");
        vault_workflows::refresh_next_run(v, slug).unwrap();
        intact("refresh_next_run");
        let now = Utc::now();
        vault_workflows::skip_scheduled_run(
            v,
            slug,
            vault_workflows::missed_while_closed_entry(now),
            now,
        )
        .unwrap();
        intact("skip_scheduled_run");
        vault_workflows::mark_status(v, slug, "running", None, None, None).unwrap();
        vault_workflows::heal_orphan_running(v, &[]);
        intact("heal_orphan_running");

        vault_workflows::set_schedule(v, slug, CRON).unwrap();
        let content = read(v, slug);
        assert_eq!(
            fields(&content),
            PermissionFields {
                schedule: CRON.into(),
                ..original.clone()
            },
            "a routine toggle changes only the schedule"
        );
        assert!(
            content.contains(hand_keys),
            "set_schedule rewrote hand-written keys:\n{content}"
        );
        assert!(
            content.contains(prompt),
            "set_schedule moved the prompt:\n{content}"
        );
        assert!(
            verify(v, slug).is_ok(),
            "the toggle re-stamped the schedule"
        );
        assert!(items(v).is_empty());
    }

    #[test]
    fn initialize_trusts_existing_workflows_exactly_once() {
        let dir = vault();
        let v = dir.path();
        write(
            v,
            "one",
            "---\ntitle: One\nschedule: manual\n---\n\nFIRST\n",
        );
        write(
            v,
            "two",
            "---\ntitle: Two\nschedule: manual\npermission_mode: plan\n---\n\nSECOND\n",
        );
        assert_eq!(initialize(v).unwrap(), 2);
        assert!(verify(v, "one").is_ok());
        assert!(verify(v, "two").is_ok());

        write(
            v,
            "three",
            "---\ntitle: Three\nschedule: manual\n---\n\nTHIRD\n",
        );
        assert_eq!(initialize(v).unwrap(), 0, "trust on first use runs once");
        let message = refused(verify(v, "three"));
        assert!(message.contains("created outside"), "{message}");
        let item = only_pending(v);
        assert!(item.payload["approvedDigest"].is_null());
        assert_eq!(item.payload["escalation"], true);
        assert_eq!(item.source_path.as_deref(), Some("workflows/three.md"));
    }

    #[test]
    fn owned_writers_restamp_an_untampered_workflow() {
        let dir = vault();
        let v = dir.path();
        initialize(v).unwrap();
        let w = create(v, "Owned", "PROMPT BODY");
        assert!(verify(v, &w.slug).is_ok());

        vault_workflows::set_schedule(v, &w.slug, CRON).unwrap();
        assert!(
            verify(v, &w.slug).is_ok(),
            "routine toggle re-stamps the schedule"
        );

        vault_workflows::upsert(
            v,
            Some(&w.slug),
            "Owned",
            "desc",
            CRON,
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "A NEW PROMPT",
        )
        .unwrap();
        assert!(verify(v, &w.slug).is_ok(), "a UI save re-stamps");

        let renamed = vault_workflows::upsert(
            v,
            Some(&w.slug),
            "Owned Renamed",
            "desc",
            CRON,
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "A NEW PROMPT",
        )
        .unwrap();
        assert_ne!(renamed.slug, w.slug);
        assert!(verify(v, &renamed.slug).is_ok(), "a rename moves the stamp");
        let stamped = stamps(v).unwrap();
        assert!(
            !stamped.contains_key(&w.slug),
            "the old slug's stamp is gone"
        );
        assert!(stamped.contains_key(&renamed.slug));
        assert!(items(v).is_empty(), "no review was ever needed");
    }

    #[test]
    fn tampered_workflow_stays_refused_after_routine_toggle_and_ui_save_until_approved() {
        let tampers: [(&str, &str, &str); 3] = [
            (
                "mode",
                "permission_mode: acceptEdits",
                "permission_mode: bypassPermissions",
            ),
            ("prompt", "PROMPT BODY", "EXFILTRATE EVERYTHING"),
            (
                "triggers",
                "permission_mode: acceptEdits",
                "permission_mode: acceptEdits\ntriggers: file-changed:**/*.md",
            ),
        ];
        for (name, from, to) in tampers {
            let dir = vault();
            let v = dir.path();
            initialize(v).unwrap();
            let w = create(v, "Guarded", "PROMPT BODY");
            tamper(v, &w.slug, from, to);

            vault_workflows::set_schedule(v, &w.slug, CRON).unwrap();
            refused(verify(v, &w.slug));

            let saved = ui_save_as_loaded(v, &w.slug);
            refused(verify(v, &w.slug));
            if name == "triggers" {
                assert_eq!(
                    saved.triggers,
                    vec!["file-changed:**/*.md"],
                    "tampered triggers survive"
                );
            }
            let item = only_pending(v);
            approve(v, &item, None, Some(&item.fingerprint))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let verified = verify(v, &w.slug).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(verified.workflow.slug, w.slug);
            assert!(pending(v).is_empty());
        }
    }

    #[test]
    fn ui_save_never_approves_an_unstamped_or_unparseable_prior() {
        let dir = vault();
        let v = dir.path();
        initialize(v).unwrap();
        // 1. An agent-created, never-stamped automation saved from the editor.
        write(v, "sneaky", "---\ntitle: Sneaky\nschedule: manual\npermission_mode: bypassPermissions\n---\n\nBODY\n");
        ui_save_as_loaded(v, "sneaky");
        refused(verify(v, "sneaky"));
        assert_eq!(pending(v).len(), 1);

        // 2. A stamped automation whose file lost its title, then saved with
        //    bypassPermissions from a stale editor.
        let w = create(v, "Stamped", "BODY");
        let approved = stamps(v).unwrap()[&w.slug].clone();
        tamper(v, &w.slug, "title: Stamped\n", "");
        vault_workflows::upsert(
            v,
            Some(&w.slug),
            "Stamped",
            "desc",
            "manual",
            None,
            None,
            None,
            "bypassPermissions",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        refused(verify(v, &w.slug));
        assert_eq!(
            stamps(v).unwrap()[&w.slug],
            approved,
            "the stamp still holds the old fields"
        );
    }

    #[test]
    fn routine_toggle_heals_a_schedule_only_tamper() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Routine", "BODY");
        tamper(v, &w.slug, "schedule: manual", "schedule: 0 0 3 * * * *");
        refused(verify(v, &w.slug));
        vault_workflows::set_schedule(v, &w.slug, CRON).unwrap();
        assert!(verify(v, &w.slug).is_ok());
        assert!(pending(v).is_empty());
        assert_eq!(superseded(v).len(), 1);
        assert_eq!(stamps(v).unwrap()[&w.slug].schedule, CRON);
    }

    #[test]
    fn approve_rejects_stale_or_forged_reviews() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Forge", "BODY");
        let original = read(v, &w.slug);
        write(
            v,
            &w.slug,
            &original.replacen("permission_mode: acceptEdits", "permission_mode: auto", 1),
        );
        refused(verify(v, &w.slug));
        let a = only_pending(v);
        write(
            v,
            &w.slug,
            &original.replacen(
                "permission_mode: acceptEdits",
                "permission_mode: bypassPermissions",
                1,
            ),
        );
        refused(verify(v, &w.slug));
        let b = only_pending(v);
        assert_ne!(a.id, b.id);
        assert_eq!(superseded(v).len(), 1, "a was superseded by b");

        // 2. The stale card can't be approved.
        assert!(approve(v, &a, None, Some(&a.fingerprint)).is_err());
        refused(verify(v, &w.slug));

        // 3. A forged payload on the live card can't be approved, and the
        //    canonical payload comes back on the same fingerprint.
        let queue_path = v.join(".review-queue.json");
        let mut queue: Value = serde_json::from_slice(&fs::read(&queue_path).unwrap()).unwrap();
        for item in queue["items"].as_array_mut().unwrap() {
            if item["id"] == json!(b.id) {
                item["payload"]["summary"] = json!("Nothing to see here");
                item["payload"]["observed"]["permissionMode"] = json!("plan");
            }
        }
        fs::write(&queue_path, serde_json::to_vec_pretty(&queue).unwrap()).unwrap();
        let forged = pending(v).into_iter().find(|i| i.id == b.id).unwrap();
        assert!(approve(v, &forged, None, Some(&forged.fingerprint))
            .unwrap_err()
            .contains("changed again"));
        let refreshed = only_pending(v);
        assert_eq!(refreshed.fingerprint, b.fingerprint);
        assert_eq!(refreshed.payload, b.payload, "canonical payload restored");
        assert!(refused(verify(v, &w.slug)).contains("bypassPermissions"));

        // 5. Edited payloads are refused outright.
        assert!(approve(
            v,
            &refreshed,
            Some(&json!({"summary": "x"})),
            Some(&refreshed.fingerprint)
        )
        .is_err());

        // 4. The refreshed canonical card approves.
        approve(v, &refreshed, None, Some(&refreshed.fingerprint)).unwrap();
        assert!(verify(v, &w.slug).is_ok());
    }

    #[test]
    fn approve_tolerates_a_title_only_edit_after_enqueue() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Titled", "BODY");
        tamper(
            v,
            &w.slug,
            "permission_mode: acceptEdits",
            "permission_mode: plan",
        );
        refused(verify(v, &w.slug));
        let item = only_pending(v);
        tamper(v, &w.slug, "title: Titled", "title: Titled Again");
        approve(v, &item, None, Some(&item.fingerprint)).unwrap();
        assert!(verify(v, &w.slug).is_ok());
    }

    #[test]
    fn a_recurring_approved_transition_reopens_its_review() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Recurring", "BODY");
        let s = read(v, &w.slug);
        let t = s.replacen("permission_mode: acceptEdits", "permission_mode: auto", 1);

        // 1. Approve S → T.
        write(v, &w.slug, &t);
        refused(verify(v, &w.slug));
        let first = only_pending(v);
        approve(v, &first, None, Some(&first.fingerprint)).unwrap();
        // 2. Save back to S through the editor (the prior T matched its stamp).
        vault_workflows::upsert(
            v,
            Some(&w.slug),
            "Recurring",
            "desc",
            "manual",
            None,
            None,
            None,
            "acceptEdits",
            &[],
            &[],
            None,
            "BODY",
        )
        .unwrap();
        assert!(verify(v, &w.slug).is_ok());
        // 3. The same S → T transition again reopens the accepted review.
        write(v, &w.slug, &t);
        let outcome = check_inner(v, &w.slug).expect_err("refused");
        assert!(outcome.notice.as_ref().unwrap().opened);
        let reopened = only_pending(v);
        assert_eq!(reopened.id, first.id);
        refused(verify(v, &w.slug));

        // 4. Approve A → C, then write A back: a new (C, A) pair.
        let a = s.replacen("permission_mode: acceptEdits", "permission_mode: plan", 1);
        let c = s.replacen(
            "permission_mode: acceptEdits",
            "permission_mode: default",
            1,
        );
        write(v, &w.slug, &a);
        refused(verify(v, &w.slug));
        {
            let card = only_pending(v);
            approve(v, &card, None, Some(&card.fingerprint)).unwrap();
        }
        write(v, &w.slug, &c);
        refused(verify(v, &w.slug));
        {
            let card = only_pending(v);
            approve(v, &card, None, Some(&card.fingerprint)).unwrap();
        }
        write(v, &w.slug, &a);
        refused(verify(v, &w.slug));
        let back = only_pending(v);
        assert_eq!(back.payload["approvedDigest"], json!(fields(&c).digest()));
        assert_eq!(back.payload["observedDigest"], json!(fields(&a).digest()));
    }

    #[test]
    fn returning_to_an_earlier_tamper_reopens_its_review() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Flip", "BODY");
        let s = read(v, &w.slug);
        let a = s.replacen("BODY", "BODY A", 1);
        let b = s.replacen("BODY", "BODY B", 1);
        write(v, &w.slug, &a);
        refused(verify(v, &w.slug));
        let first = only_pending(v);
        write(v, &w.slug, &b);
        refused(verify(v, &w.slug));
        write(v, &w.slug, &a);
        refused(verify(v, &w.slug));
        let live = only_pending(v);
        assert_eq!(live.id, first.id, "A's review is reopened");
        assert_eq!(superseded(v).len(), 1, "B's review is superseded");
    }

    #[test]
    fn restoring_approved_content_retires_the_pending_review() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Restore", "BODY");
        let original = read(v, &w.slug);
        tamper(v, &w.slug, "BODY", "OTHER");
        refused(verify(v, &w.slug));
        write(v, &w.slug, &original);
        assert!(verify(v, &w.slug).is_ok());
        assert!(pending(v).is_empty());
        assert_eq!(superseded(v).len(), 1);
    }

    #[test]
    fn rejected_review_is_sticky_and_keeps_refusing() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Sticky", "BODY");
        tamper(
            v,
            &w.slug,
            "permission_mode: acceptEdits",
            "permission_mode: bypassPermissions",
        );
        refused(verify(v, &w.slug));
        let item = only_pending(v);
        review_queue::reject(v, &item.id).unwrap();

        let again = check_inner(v, &w.slug).expect_err("still refused");
        assert!(
            again.error.message().contains("you rejected"),
            "{}",
            again.error
        );
        assert!(!again.notice.unwrap().opened);
        assert!(pending(v).is_empty());

        // A later editor save that changes the prompt asks again.
        let (loaded, _, _) = vault_workflows::read_full(v, &w.slug).unwrap();
        vault_workflows::upsert(
            v,
            Some(&w.slug),
            "Sticky",
            "desc",
            "manual",
            None,
            None,
            None,
            &loaded.permission_mode,
            &[],
            &[],
            None,
            "A CORRECTED BODY",
        )
        .unwrap();
        refused(verify(v, &w.slug));
        assert_ne!(only_pending(v).fingerprint, item.fingerprint);
    }

    #[test]
    fn delete_removes_the_stamp_and_a_recreated_file_needs_review() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Doomed", "BODY");
        let content = read(v, &w.slug);
        tamper(v, &w.slug, "BODY", "OTHER");
        refused(verify(v, &w.slug));
        vault_workflows::delete(v, &w.slug).unwrap();
        assert!(!stamps(v).unwrap().contains_key(&w.slug));
        assert!(pending(v).is_empty(), "the live review was retired");

        write(v, &w.slug, &content);
        refused(verify(v, &w.slug));
        let item = only_pending(v);
        assert!(item.payload["approvedDigest"].is_null());

        fs::remove_file(path_of(v, &w.slug)).unwrap();
        assert!(matches!(verify(v, &w.slug), Err(VerifyError::Missing(_))));
        assert!(pending(v).is_empty(), "a missing file retires its review");
    }

    #[test]
    fn corrupt_stamp_store_fails_closed_and_is_preserved() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Corrupt", "BODY");
        let store = store_path(v).unwrap();
        fs::write(&store, b"{ not json").unwrap();
        match verify(v, &w.slug) {
            Err(VerifyError::Unavailable(message)) => {
                assert!(
                    message.contains("permission records unavailable"),
                    "{message}"
                );
                assert!(message.contains("Nothing was overwritten"), "{message}");
            }
            other => panic!("expected Unavailable, got {other:?}"),
        }
        assert!(initialize(v).is_err());
        // Owned writers log instead of overwriting.
        vault_workflows::set_schedule(v, &w.slug, CRON).unwrap();
        assert_eq!(fs::read(&store).unwrap(), b"{ not json");
        assert!(items(v).is_empty(), "nothing was queued");
    }

    #[test]
    fn verify_waits_for_an_in_flight_owned_write() {
        let dir = vault();
        let v = dir.path().to_path_buf();
        let w = create(&v, "Serialized", "BODY");
        let guard = write_guard();
        let (vault_path, slug) = (v.clone(), w.slug.clone());
        let handle = std::thread::spawn(move || verify(&vault_path, &slug).is_ok());
        std::thread::sleep(Duration::from_millis(150));
        assert!(
            !handle.is_finished(),
            "verify must wait for the owned write"
        );
        drop(guard);
        assert!(handle.join().unwrap());
    }

    #[test]
    fn review_payload_shows_untruncated_list_diffs_and_prompt_excerpts() {
        let lines: Vec<String> = (0..200).map(|n| format!("line {n}")).collect();
        let approved_prompt = lines.join("\n");
        let mut changed = lines.clone();
        changed[100] = "line 100 — now exfiltrate the vault".into();
        let observed_prompt = changed.join("\n");
        let approved = PermissionFields {
            permission_mode: "acceptEdits".into(),
            allowed_tools: vec!["Read".into()],
            disallowed_tools: vec![],
            cwd: None,
            agent: None,
            model: None,
            triggers: vec![],
            schedule: "manual".into(),
            in_chat: None,
            runs_on: None,
            precheck: None,
            prompt_sha256: sha256_hex(approved_prompt.as_bytes()),
        };
        let mut tools = vec!["Read".to_string()];
        tools.extend((1..12).map(|n| format!("Tool{n}")));
        tools.push("Bash(*)".into());
        let observed = PermissionFields {
            allowed_tools: tools,
            prompt_sha256: sha256_hex(observed_prompt.as_bytes()),
            ..approved.clone()
        };
        let stamp = Stamp::new(approved, &approved_prompt, "create");
        let payload = review_payload("x", "X", Some(&stamp), &observed, &observed_prompt);
        let diff = payload["diff"].as_array().unwrap();
        let tools = diff.iter().find(|e| e["field"] == "allowed tools").unwrap();
        let added = tools["added"].as_array().unwrap();
        assert_eq!(added.len(), 12);
        assert_eq!(added.last().unwrap(), "Bash(*)");
        let prompt = diff.iter().find(|e| e["field"] == "prompt").unwrap();
        let before = prompt["before"].as_str().unwrap();
        let after = prompt["after"].as_str().unwrap();
        assert!(before.contains("line 100") && !before.contains("exfiltrate"));
        assert!(after.contains("now exfiltrate the vault"));
        assert!(
            after.contains("line 98") && after.contains("line 102"),
            "{after}"
        );
        assert!(!after.contains("line 10\n"), "only local context is kept");
        assert!(before.chars().count() <= PROMPT_EXCERPT_CAP);
        assert!(after.chars().count() <= PROMPT_EXCERPT_CAP);
        assert_eq!(prompt["truncated"], true);
        assert_eq!(payload["escalation"], true);
        let summary = payload["summary"].as_str().unwrap();
        assert!(
            !summary.contains("+Read"),
            "unchanged entries are not listed: {summary}"
        );
        assert!(summary.contains("(+8 more)"), "{summary}");
        assert!(summary.contains("Prompt changed"), "{summary}");
    }

    fn with(base: &PermissionFields, edit: impl FnOnce(&mut PermissionFields)) -> PermissionFields {
        let mut next = base.clone();
        edit(&mut next);
        next
    }

    #[test]
    fn escalation_follows_what_the_runner_would_allow() {
        let base = fields(BASE);
        let esc = PermissionFields::escalation;
        assert!(esc(None, &base));
        assert!(!esc(Some(&base), &base));
        assert!(esc(
            Some(&base),
            &with(&base, |f| f.permission_mode = "bypassPermissions".into())
        ));
        assert!(!esc(
            Some(&base),
            &with(&base, |f| f.permission_mode = "plan".into())
        ));
        assert!(esc(
            Some(&base),
            &with(&base, |f| f.permission_mode = "unheard-of".into())
        ));
        // An empty allowlist is unrestricted: adding entries restricts.
        let restricted = with(&base, |f| f.allowed_tools = vec!["Read".into()]);
        assert!(!esc(Some(&base), &restricted));
        assert!(
            esc(Some(&restricted), &base),
            "clearing the allowlist widens it"
        );
        let widened = with(&base, |f| {
            f.allowed_tools = vec!["Read".into(), "Bash".into()]
        });
        assert!(esc(Some(&restricted), &widened));
        let denied = with(&base, |f| f.disallowed_tools = vec!["Bash".into()]);
        assert!(esc(Some(&denied), &base));
        assert!(!esc(Some(&base), &denied));
        assert!(esc(Some(&base), &with(&base, |f| f.cwd = Some("/".into()))));
        assert!(esc(
            Some(&base),
            &with(&base, |f| f.agent = Some("codex".into()))
        ));
        assert!(esc(
            Some(&base),
            &with(&base, |f| f.triggers = vec!["task-overdue".into()])
        ));
        assert!(!esc(
            Some(&base),
            &with(&base, |f| f.prompt_sha256 = "x".into())
        ));
    }

    #[test]
    fn refusal_notification_is_opened_once_per_pair() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Notify", "BODY");
        tamper(
            v,
            &w.slug,
            "permission_mode: acceptEdits",
            "permission_mode: bypassPermissions",
        );
        let first = check_inner(v, &w.slug).expect_err("refused");
        assert!(first.notice.as_ref().unwrap().opened);
        for _ in 0..3 {
            let again = check_inner(v, &w.slug).expect_err("refused");
            assert!(!again.notice.unwrap().opened);
        }
        let input = refusal_notification(
            "Notify",
            &w.slug,
            "Permission mode: acceptEdits → bypassPermissions",
        );
        assert_eq!(input.kind, "warning");
        assert_eq!(
            input.action_url.as_deref(),
            Some(format!("workflow-permissions:{}", w.slug).as_str())
        );
        assert_eq!(input.source.as_deref(), Some("workflow-permissions"));
        assert!(input.notify_native);
        assert!(input.title.contains("Notify"));
        // The rate limit allows one notice per workflow per interval.
        assert!(notice_due(v, "rate-limited"));
        assert!(!notice_due(v, "rate-limited"));
    }

    #[test]
    fn a_quietly_opened_review_is_announced_once_by_the_next_notify_capable_check() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Hand edited", "BODY");
        tamper(
            v,
            &w.slug,
            "permission_mode: acceptEdits",
            "permission_mode: bypassPermissions",
        );
        // The watcher's sweep opens the review but stays quiet (an Obsidian
        // autosave must never pop a notification).
        sweep(v, false);
        assert_eq!(pending(v).len(), 1);
        assert!(sent_for_test(v).is_empty());
        // The scheduler's admission when it comes due (or Run now) announces
        // it, although it didn't open the review itself...
        refused(verify(v, &w.slug));
        assert_eq!(sent_for_test(v).len(), 1, "{:?}", sent_for_test(v));
        assert!(sent_for_test(v)[0].contains("Hand edited"));
        // ...exactly once.
        refused(verify(v, &w.slug));
        sweep(v, true);
        assert_eq!(sent_for_test(v).len(), 1);

        // A review a flagged run opened is covered by that run's notification.
        let other = create(v, "Run edited", "BODY");
        tamper(v, &other.slug, "BODY", "EXFILTRATE");
        sweep(v, false);
        refused(verify_quiet(v, &other.slug));
        refused(verify(v, &other.slug));
        assert_eq!(
            sent_for_test(v).len(),
            1,
            "no second notice: {:?}",
            sent_for_test(v)
        );
    }

    #[test]
    fn an_unreadable_stamp_store_notifies_once_per_vault() {
        let dir = vault();
        let v = dir.path();
        let a = create(v, "First", "BODY");
        let b = create(v, "Second", "BODY");
        fs::write(store_path(v).unwrap(), b"{ not json").unwrap();
        for slug in [&a.slug, &b.slug, &a.slug] {
            assert!(matches!(verify(v, slug), Err(VerifyError::Unavailable(_))));
        }
        let sent = sent_for_test(v);
        assert_eq!(
            sent,
            vec!["Automations are paused: permission records unavailable".to_string()]
        );
        let health = fs::read_to_string(v.join(crate::health::FILE)).unwrap();
        assert!(
            health.contains("permission records unavailable"),
            "{health}"
        );
        // Quiet checks never notify.
        assert!(matches!(
            verify_quiet(v, &a.slug),
            Err(VerifyError::Unavailable(_))
        ));
        assert_eq!(sent_for_test(v).len(), 1);
        // Every automation reads as paused in the Automations list.
        let holds = permission_holds(v);
        assert_eq!(holds.get(&a.slug).copied(), Some(HOLD_UNAVAILABLE));
        assert_eq!(holds.get(&b.slug).copied(), Some(HOLD_UNAVAILABLE));
    }

    #[test]
    fn a_workflow_deleted_outside_the_editor_loses_its_approval() {
        let dir = vault();
        let v = dir.path();
        initialize(v).unwrap();
        let w = create(v, "Resurrected", "BODY");
        let approved = read(v, &w.slug);
        fs::remove_file(path_of(v, &w.slug)).unwrap();
        sweep(v, false);
        assert!(
            !stamps(v).unwrap().contains_key(&w.slug),
            "the approval is retired"
        );
        assert!(
            approvals(v).unwrap().knows(&w.slug),
            "but Hollow still knows it"
        );

        // Byte-for-byte the approved file (an agent can recover it from
        // .hollow-history/objects): refused as a new automation.
        write(v, &w.slug, &approved);
        let message = refused(verify(v, &w.slug));
        assert!(message.contains("created outside"), "{message}");
        let item = only_pending(v);
        assert!(item.payload["approvedDigest"].is_null());
        // Neither a routine toggle nor a UI save revives the tombstone...
        vault_workflows::set_schedule(v, &w.slug, CRON).unwrap();
        refused(verify(v, &w.slug));
        ui_save_as_loaded(v, &w.slug);
        refused(verify(v, &w.slug));
        // ...only an approval does.
        let card = only_pending(v);
        approve(v, &card, None, Some(&card.fingerprint)).unwrap();
        assert!(verify(v, &w.slug).is_ok());
        assert!(!approvals(v).unwrap().tombstoned.contains(&w.slug));

        // A cloud-evicted file (iCloud placeholder) keeps its approval.
        let evicted = create(v, "Evicted", "BODY");
        let content = read(v, &evicted.slug);
        fs::remove_file(path_of(v, &evicted.slug)).unwrap();
        fs::write(
            v.join(format!("workflows/.{}.md.icloud", evicted.slug)),
            b"",
        )
        .unwrap();
        sweep(v, false);
        write(v, &evicted.slug, &content);
        assert!(
            verify(v, &evicted.slug).is_ok(),
            "a re-downloaded file needs no review"
        );
    }

    #[test]
    fn approval_is_bound_to_the_card_the_user_confirmed() {
        let dir = vault();
        let v = dir.path();
        let x = create(v, "Target", "BODY");
        let y = create(v, "Benign", "BODY");
        tamper(
            v,
            &x.slug,
            "permission_mode: acceptEdits",
            "permission_mode: bypassPermissions",
        );
        tamper(
            v,
            &y.slug,
            "permission_mode: acceptEdits",
            "permission_mode: plan",
        );
        refused(verify(v, &x.slug));
        refused(verify(v, &y.slug));
        let find = |slug: &str| {
            pending(v)
                .into_iter()
                .find(|i| i.payload["slug"] == json!(slug))
                .unwrap()
        };
        let (real_x, real_y) = (find(&x.slug), find(&y.slug));

        // No fingerprint, or another card's: refused.
        assert!(approve(v, &real_y, None, None)
            .unwrap_err()
            .contains("did not say"));
        assert!(approve(v, &real_y, None, Some(&real_x.fingerprint))
            .unwrap_err()
            .contains("changed after you opened it"));

        // An agent plants X's canonical item under Y's id (newest first, so
        // it's the one a lookup by id would find). The user approves Y's card.
        let queue_path = v.join(".review-queue.json");
        let mut queue: Value = serde_json::from_slice(&fs::read(&queue_path).unwrap()).unwrap();
        let items = queue["items"].as_array_mut().unwrap();
        let mut forged = items
            .iter()
            .find(|i| i["id"] == json!(real_x.id))
            .unwrap()
            .clone();
        forged["id"] = json!(real_y.id);
        forged["createdAt"] = json!("2099-01-01T00:00:00Z");
        items.push(forged);
        fs::write(&queue_path, serde_json::to_vec_pretty(&queue).unwrap()).unwrap();
        let error = approve(v, &real_y, None, Some(&real_y.fingerprint)).unwrap_err();
        assert!(error.contains("share the id"), "{error}");
        assert!(
            stamps(v).unwrap()[&x.slug].permission_mode == "acceptEdits",
            "X was never approved"
        );
        refused(verify(v, &x.slug));
    }

    #[test]
    fn permission_holds_report_what_verify_would_refuse() {
        let dir = vault();
        let v = dir.path();
        let ok = create(v, "Fine", "BODY");
        let edited = create(v, "Edited", "BODY");
        let rejected = create(v, "Rejected", "BODY");
        write(
            v,
            "handmade",
            "---\ntitle: Handmade\nschedule: manual\n---\n\nBODY\n",
        );
        write(v, "broken", "---\ndescription: no title\n---\n\nBODY\n");
        tamper(v, &edited.slug, "BODY", "OTHER");
        tamper(v, &rejected.slug, "BODY", "OTHER");
        refused(verify(v, &rejected.slug));
        review_queue::reject(v, &only_pending(v).id).unwrap();
        let queue_before = fs::read(v.join(".review-queue.json")).unwrap();
        let sent_before = sent_for_test(v).len();

        let holds = permission_holds(v);
        assert_eq!(holds.get(&ok.slug), None);
        assert_eq!(holds.get(&edited.slug).copied(), Some(HOLD_REVIEW));
        assert_eq!(
            holds.get(&rejected.slug).copied(),
            Some(HOLD_REVIEW),
            "a rejection still holds it"
        );
        assert_eq!(holds.get("handmade").copied(), Some(HOLD_REVIEW));
        assert_eq!(
            holds.get("broken"),
            None,
            "unparseable files are configuration errors"
        );
        // Read-only: no review was opened for the edited automation.
        assert_eq!(
            fs::read(v.join(".review-queue.json")).unwrap(),
            queue_before
        );
        assert_eq!(sent_for_test(v).len(), sent_before, "nothing is announced");

        // Restoring the approved version clears the hold even though the
        // user's rejection stays in the queue.
        tamper(v, &rejected.slug, "OTHER", "BODY");
        assert_eq!(permission_holds(v).get(&rejected.slug), None);
        assert!(items(v)
            .iter()
            .any(|i| i.status == "rejected" && i.error.is_none()));
    }

    #[test]
    fn sweep_creates_and_retires_reviews_without_running_anything() {
        let dir = vault();
        let v = dir.path();
        let w = create(v, "Swept", "BODY");
        let original = read(v, &w.slug);
        tamper(v, &w.slug, "BODY", "OTHER");
        sweep(v, false);
        assert_eq!(pending(v).len(), 1);
        write(v, &w.slug, &original);
        sweep(v, false);
        assert!(pending(v).is_empty());
    }
}
