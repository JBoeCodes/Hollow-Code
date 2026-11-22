//! The per-run context/decision manifest (agent effectiveness F11, wave 1 ·
//! lane R): what prepared one run's context, and why, keyed by the run's id
//! (the History run id its evidence receipt carries).
//!
//! It replaces the process-global "last report" the context prioritizer kept
//! (`context_rank`'s old `LAST`), which a second turn preparing at the same
//! time overwrote. Each run now builds its own record on its
//! [`crate::evidence::EvidenceCollector`], and `finish` stores it with the
//! receipt: the receipt carries a small reference ([`ManifestRef`]: schema,
//! byte count, SHA-256) and the manifest itself sits in a sidecar,
//! `{vault}/.evidence-manifests/{runId}.json`, written and pruned under the
//! receipt store's lock, so it is kept exactly as long as its receipt (the
//! newest `evidence::KEEP`). A reader checks the sidecar against the
//! receipt's hash and refuses one that differs (the receipt store is a
//! watched state file; the sidecar is not).
//!
//! **Hashes and metadata only.** Layer sizes and SHA-256s, vault-relative
//! paths, scores, Hollow's own reasons and notices: never the text of the
//! prompt, the request, a passage or a Jev request. There is no raw-text
//! switch: a diagnostic that needs the text can read the run's own
//! transcript, which already holds it, so a second copy here would only be
//! one more place for it to leak from.
//!
//! What it records (schema [`SCHEMA_VERSION`]):
//! - `layers`: each context layer the backend assembled or could see in the
//!   composed prompt, with its UTF-8 size and hash ([`Layer`]).
//! - `retrieval`: the candidates retrieval considered (at most 8) and which
//!   were kept, with the local score, Jev's score, the basis and a reason,
//!   and the coverage notices ([`RetrievalRecord`]).
//! - `attachedSources`: a workspace run's attached sources (agent
//!   effectiveness F3 part 2): the 24 KB budget and what was used, the
//!   reader the agent has, each file's coverage, size and included bytes
//!   (hashed), and one decision per kept or matching chunk: path, line
//!   range, score, kept or omitted, bytes and Hollow's reason (a matching
//!   chunk below OF06's relevance floor says so) ([`AttachedSourcesRecord`]).
//! - `lifecycle`: how the one lifecycle policy (agent effectiveness F7,
//!   `note_lifecycle`) treated remembered notes in recall and retrieval:
//!   whether the request asked about the past (and which of Hollow's cue
//!   words said so), and per note its route, path, status (`superseded`,
//!   `current`, `duplicate`, `unapproved`), action and reason
//!   ([`LifecycleRecord`]).
//! - `connectors`: the context prioritizer's decision for every "when
//!   relevant" connector, with its basis and reason
//!   ([`crate::context_rank::ConnectorSelection`], agent effectiveness F5).
//! - `jev`: each Jev use that ran or would have run, its outcome and latency
//!   and a cost that is always `null` (TypeSafe reports none: unknown, never
//!   0) ([`JevUse`]).
//! - `capabilities`: the Hollow-owned MCP servers and library connectors the
//!   run was given, and Claude's inline Bash output cap (OB23,
//!   `bashOutputMaxChars`) when its command carried one.
//! - `fallbacks`: every place preparation fell back (Jev failed, a fresh
//!   native session) ([`Fallback`]).
//! - `preparation`: how long retrieval and connector selection took, and
//!   (agent effectiveness F8, wave 3) the `timeline`: every preparation
//!   step's start offset from admission and its duration, `null` for a step
//!   that didn't run (`prep_timing`, [`PrepTimeline`]).
//! - `compiler`: a chat turn's host context compiler decisions (agent
//!   effectiveness F4, `context_compiler.rs`): the entry point, the native
//!   session's state, and whether the operator profile (its digest's size
//!   and hash) and the conversation bundle (entries and bytes) were sent,
//!   the task-state record's revision and size and whether it was sent
//!   ([`TaskStateRecord`]), and whether a resumed session got the profile's
//!   revision delta ([`ProfileDeltaRecord`]) ([`CompilerRecord`]).
//!   Its `visualGuide` says how the body got the visual guide (agent
//!   effectiveness F6, G1): in the prompt, in this turn, given earlier in
//!   the session or left to the file, the cue (Hollow's own word) and the
//!   guide's size and hash ([`VisualGuideRecord`]).
//! - `nativeMemory`: what Claude Code loads by itself, which Hollow neither
//!   assembles nor sees in the prompt (S13): the CLAUDE.md files, the
//!   auto-memory folder for the run's project and the user skill listing, by
//!   path and size ([`NativeMemoryRecord`]). Claude runs only.
//!
//! The manifest is diagnostic. Nothing reads it to decide anything.
use crate::context_rank::ConnectorSelection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// The manifest's schema. A reader refuses any other.
pub const SCHEMA_VERSION: u32 = 1;
/// The sidecar folder at the vault root (hidden, so History never tracks it).
pub(crate) const DIR_NAME: &str = ".evidence-manifests";
/// A sidecar larger than this is not read.
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Notices kept per retrieval, and characters per notice.
const MAX_NOTICES: usize = 12;
const NOTICE_CHARS: usize = 300;

/// See the module doc.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RunManifest {
    pub schema_version: u32,
    pub run_id: String,
    /// In the order the agent reads them.
    pub layers: Vec<Layer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub retrieval: Option<RetrievalRecord>,
    /// A workspace run's attached sources: the chunks the broker ranked
    /// against the request and which reached the envelope (agent
    /// effectiveness F3 part 2). Absent outside a workspace, for a quick
    /// chat, and in manifests written before 2026-10-06.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub attached_sources: Option<AttachedSourcesRecord>,
    /// How the one lifecycle policy treated remembered notes for this run,
    /// in recall and retrieval (agent effectiveness F7). Absent when no
    /// candidate had a lifecycle, and in manifests written before
    /// 2026-10-06.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub lifecycle: Option<LifecycleRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub connectors: Option<ConnectorSelection>,
    #[serde(default)]
    pub jev: Vec<JevUse>,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub fallbacks: Vec<Fallback>,
    #[serde(default)]
    pub preparation: Preparation,
    /// A chat turn's context compiler decisions (agent effectiveness F4);
    /// absent on workflow runs and on manifests from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub compiler: Option<CompilerRecord>,
    /// The agent's own reads through Hollow's vault server during the run
    /// (F3: agent rereads are measured): counted from the run's tool calls
    /// when the receipt is written. Present when `hollow_vault` was
    /// attached (zero included) or was called; absent otherwise and on
    /// manifests from before 2026-10-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub agent_reads: Option<AgentReads>,
    /// The Jev duplicate checks the run itself ran (F11, O3): a heartbeat's
    /// check of its task proposals, with its request also in `jev` (use
    /// `proposals`). Absent when none ran (a chat's learning check runs
    /// after its receipt and is in the view's `afterRun`), and on manifests
    /// from before 2026-10-07.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<crate::jev_decide::Report>>", optional))]
    pub dedup: Vec<crate::jev_decide::Report>,
    /// A chat turn's relevant skill index (F9, wave 4 · V2): which skills
    /// its `<hollow-skills>` block listed and why, which went inline, and
    /// which the agent read. Absent when no skill fit and none was
    /// selected, and on manifests from before 2026-10-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub skills: Option<SkillsRecord>,
    /// What Claude Code loads by itself into the run, outside Hollow's
    /// envelope (S13): its CLAUDE.md files, the auto-memory folder for the
    /// run's project (repository root) and its user skill listing. Paths
    /// and sizes only. Claude runs only; absent for every other agent and
    /// on manifests from before 2026-10-17.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub native_memory: Option<NativeMemoryRecord>,
    /// Whether the run's recall ranked global learnings by meaning (OF26):
    /// `used` (a query vector was read), `skipped` (a turn that retrieves
    /// nothing embeds nothing), `timedOut` (no answer within recall's
    /// 250 ms, or an earlier recall's still running; stems only) or
    /// `noVector` (nothing to embed, or the embedding failed). Absent when
    /// semantic search is off, the model isn't installed, and on manifests
    /// from before 2026-10-17 (`vault_learning::RecallMeaning`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub recall_meaning: Option<String>,
    /// What the CFO decided for the run (COO rebuild L7): agent, model,
    /// effort and who set it, the account for unattended work, the job kind
    /// and its reason. Absent on runs the CFO doesn't decide for and on
    /// manifests from before 2026-11-10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub cfo: Option<crate::cfo::Decision>,
    /// The preparation marks taken so far (F8), in memory only: the
    /// receipt turns them into `preparation.timeline`.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub(crate) timer: crate::prep_timing::InMemory<crate::prep_timing::PrepMarks>,
    /// The folder the agent ran in, in memory only: the receipt leaves the
    /// notes under it out of `agentReads.notesOpened` (S26).
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub(crate) run_cwd: Option<PathBuf>,
}

/// How one chat turn got the visual guide (`visual_prompt::delivery`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestVisualGuide"))]
#[serde(rename_all = "camelCase")]
pub struct VisualGuideRecord {
    /// `prompt` (Canvases: most replies; in the appended prompt), `turn`
    /// (in this turn's message), `given` (earlier in this native session)
    /// `onDemand` (not sent; the capability index names the file), or `off`
    /// (Canvases off: there is no guide).
    pub delivery: String,
    /// Why the request looked visual, in Hollow's own words (a cue word,
    /// `numbers` or `data file`); never the request's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub cue: Option<String>,
    /// The guide was written to the turn's scratch folder.
    pub file: bool,
    /// The guide's size and SHA-256 (its revision), sent or not.
    pub bytes: u64,
    pub sha256: String,
}

/// How often the agent asked `hollow_vault` for more during one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestAgentReads"))]
#[serde(rename_all = "camelCase")]
pub struct AgentReads {
    /// `read_source` calls (an attached or cited file's lines).
    pub read_source: u32,
    /// Of those, the ones that failed (a refusal, a missing file).
    pub read_source_failed: u32,
    /// Of those, the ones that asked by intent (a non-empty `query`, repo
    /// features R3). Zero on manifests from before 2026-10-18.
    #[serde(default)]
    pub read_source_queried: u32,
    /// `search_vault` calls.
    pub search_vault: u32,
    /// The vault notes the agent opened outside the run's folder (S26): the
    /// vault-relative `.md` paths a `Read`, a `read_source` or a shell
    /// `cat`/`sed`/`head`/`tail` read, in first-read order, at most
    /// [`MAX_NOTES_OPENED`]. Never a path in a restricted folder. Retrieval's
    /// need-recall label (`scripts/agent-eval/retrieval-labels.mjs`). Absent
    /// when none was, and on manifests from before 2026-10-17.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<String>>", optional))]
    pub notes_opened: Vec<String>,
}

/// [`AgentReads::notes_opened`]' cap.
pub const MAX_NOTES_OPENED: usize = 32;

/// The vault server's tool names as the run reports them (Claude's own,
/// and Codex's after `agent::parse_codex` normalises them).
pub const READ_SOURCE_TOOL: &str = "mcp__hollow_vault__read_source";
pub const SEARCH_VAULT_TOOL: &str = "mcp__hollow_vault__search_vault";
pub const READ_SKILL_TOOL: &str = "mcp__hollow_vault__read_skill";
/// The app server's catalog runner (`skills.read_skill` is one of its ids).
pub const RUN_ACTION_TOOL: &str = "mcp__hollow_app__run_action";

/// One chat turn's relevant skill index (`skill_index::for_turn`, F9 wave
/// 4 · V2). Names, sizes and reasons only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestSkills"))]
#[serde(rename_all = "camelCase")]
pub struct SkillsRecord {
    /// How the block told the agent to read a skill whole: `readSkill`
    /// (`hollow_vault`), `appAction` (`hollow_app` `skills.read_skill`) or
    /// `ownTools`.
    pub reader: String,
    /// Skills in the two roots (the vault's `.agents/skills`, then
    /// `~/.agents/skills`).
    pub available: u32,
    /// What the block listed, in its order.
    pub indexed: Vec<SkillIndexed>,
    /// Matching skills the index's cap (5) left out.
    pub left_out: u32,
    /// Selected skills Hollow found nowhere (named in the block, not here).
    pub selected_missing: u32,
    /// The agent's skill reads during the run, counted from its tool calls
    /// when the receipt is written.
    #[serde(default)]
    pub read: Vec<SkillRead>,
}

/// One skill a turn's index listed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestSkillIndexed"))]
#[serde(rename_all = "camelCase")]
pub struct SkillIndexed {
    pub name: String,
    /// `vault`, `shared` or `installed` (a selected provider or plugin skill).
    pub source: String,
    /// `selected`, `named` or `match`.
    pub reason: String,
    /// The local match score (0 for a selected skill).
    pub score: u32,
    /// Its `SKILL.md`'s size.
    pub bytes: u64,
    /// Its text went into the turn's message.
    pub inline: bool,
    /// Its `SKILL.md`, to match the agent's reads by path; not stored.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub path: String,
}

/// One skill read by the agent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestSkillRead"))]
#[serde(rename_all = "camelCase")]
pub struct SkillRead {
    /// The indexed skill it read; absent for one the index didn't list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub name: Option<String>,
    /// Proven indexed home, absent for legacy or ambiguous name-only reads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub source: Option<String>,
    /// `readSkill` or `appAction`.
    pub via: String,
    /// The read succeeded.
    pub ok: bool,
}

impl SkillsRecord {
    /// Record one of the run's tool calls if it read a skill.
    pub(crate) fn observe(&mut self, tool: &str, input: &serde_json::Value, failed: bool) {
        let (via, name, source) = if tool == READ_SKILL_TOOL {
            let wanted = input.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let requested_source = input
                .get("source")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let found = self.indexed.iter().find(|s| {
                s.name.eq_ignore_ascii_case(wanted.trim())
                    && requested_source.is_none_or(|source| s.source == source)
            });
            // A default name read can resolve to a shadowing vault skill
            // that the capped index did not list. Never infer its home.
            (
                "readSkill",
                found.map(|s| s.name.clone()),
                found
                    .filter(|_| requested_source.is_some())
                    .map(|s| s.source.clone()),
            )
        } else if tool == RUN_ACTION_TOOL
            && input.get("id").and_then(|v| v.as_str()) == Some("skills.read_skill")
        {
            let path = input
                .pointer("/args/path")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let real = std::fs::canonicalize(path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let found = self
                .indexed
                .iter()
                .find(|s| !s.path.is_empty() && (s.path == path || s.path == real));
            (
                "appAction",
                found.map(|s| s.name.clone()),
                found.map(|s| s.source.clone()),
            )
        } else {
            return;
        };
        self.read.push(SkillRead {
            name,
            source,
            via: via.to_string(),
            ok: !failed,
        });
    }
}

/// What Claude Code loads by itself into a Claude run (S13), found the way
/// it finds it. Hollow passes Claude no `--setting-sources` and never
/// overrides `HOME` or `CLAUDE_CONFIG_DIR` (`agent::isolate_claude` adds
/// only the MCP config and the appended prompt; `claude_accounts` keeps the
/// config folder shared across accounts), so the user file, every
/// ancestor's project files, the auto memory and the user skills all load.
/// Hollow's own `autoMemoryDirectory` (CX4b, a local turn with a personal
/// context root) is followed by `set_native_memory`. Not modelled: a
/// person's own `autoMemoryDirectory` and `CLAUDE_CODE_REMOTE_MEMORY_DIR`,
/// which move the auto-memory folder, and the settings that turn auto
/// memory off (the record still names the default folder). Paths and sizes only, never a file's text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestNativeMemory"))]
#[serde(rename_all = "camelCase")]
pub struct NativeMemoryRecord {
    /// Claude Code's config folder: `CLAUDE_CONFIG_DIR` when set, else
    /// `~/.claude`.
    pub config_dir: String,
    /// The CLAUDE.md files, in the order Claude Code loads them: the user
    /// file, then each folder's from `/` down to the cwd. `@` imports inside
    /// them and `.claude/rules/` are not followed.
    pub instructions: Vec<NativeFileRecord>,
    /// Their sizes, summed.
    pub instruction_bytes: u64,
    pub auto_memory: AutoMemoryRecord,
    pub skill_listing: SkillListingRecord,
}

/// One CLAUDE.md file Claude Code loads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestNativeFile"))]
#[serde(rename_all = "camelCase")]
pub struct NativeFileRecord {
    /// `user` (`<config>/CLAUDE.md`), `project` (`CLAUDE.md` or
    /// `.claude/CLAUDE.md` in the cwd or a folder above it) or `local`
    /// (`CLAUDE.local.md` there).
    pub scope: String,
    pub path: String,
    pub bytes: u64,
}

/// Claude Code's auto-memory folder for the run's project.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestAutoMemory"))]
#[serde(rename_all = "camelCase")]
pub struct AutoMemoryRecord {
    /// `<config>/projects/<slug>/memory`, the slug
    /// ([`claude_project_slug`]) of the project root
    /// ([`claude_memory_root`]): the git repository holding the cwd, the
    /// main working copy for a linked worktree, else the cwd itself; real
    /// paths. Named whether or not it exists.
    pub dir: String,
    pub exists: bool,
    /// `MEMORY.md`, the index Claude Code loads into every session: the
    /// whole file's size and line count (0 when there is none), though
    /// Claude Code loads only about its first 200 lines / 25 KB.
    pub memory_md_bytes: u64,
    pub memory_md_lines: u32,
    /// The other `.md` files directly in the folder: the notes the index
    /// points to, which the agent reads on demand.
    pub notes: u32,
}

/// The user skills in Claude Code's skill listing: those under
/// `<config>/skills` only. Project skills (`.claude/skills` in the
/// project), plugin skills and built-in ones are not counted.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestSkillListing"))]
#[serde(rename_all = "camelCase")]
pub struct SkillListingRecord {
    /// Folders under `<config>/skills` that hold a `SKILL.md`, symlinked
    /// ones included.
    pub count: u32,
    /// The user skills' frontmatter `name` and `description`, in bytes,
    /// summed (the folder's name when there is no `name`): what each
    /// listing entry is made of, without Claude Code's own framing. Only
    /// the first [`SKILL_READ_LIMIT`] skills are read; any past that are
    /// counted but add no bytes.
    pub bytes: u64,
}

impl NativeMemoryRecord {
    /// A `Write`, `Edit` or `MultiEdit` call whose `file_path` is inside the
    /// auto-memory folder (the receipt's `memoryWrites`, S13).
    pub(crate) fn is_memory_write(&self, tool: &str, input: &serde_json::Value) -> bool {
        if !matches!(tool, "Write" | "Edit" | "MultiEdit") || self.auto_memory.dir.is_empty() {
            return false;
        }
        let Some(path) = input.get("file_path").and_then(|v| v.as_str()) else {
            return false;
        };
        let (path, dir) = (Path::new(path), Path::new(&self.auto_memory.dir));
        if !path.is_absolute() {
            return false;
        }
        // Real paths when both exist (`/tmp` is `/private/tmp`), else the
        // path as written, with no `..` to climb out of the folder.
        match (fs::canonicalize(path), fs::canonicalize(dir)) {
            (Ok(path), Ok(dir)) => path.starts_with(dir),
            _ => {
                path.starts_with(dir)
                    && !path
                        .components()
                        .any(|c| c == std::path::Component::ParentDir)
            }
        }
    }
}

/// Bytes of a `SKILL.md` read for its frontmatter.
const SKILL_HEAD_BYTES: u64 = 16 * 1024;

/// User skills whose `SKILL.md` is read for the listing's bytes; past this
/// they are counted without being opened, so a huge skills folder costs
/// the command build a stat per entry, not a read.
const SKILL_READ_LIMIT: u32 = 500;

/// [`NativeMemoryRecord`] for a Claude run in `cwd`, with this process's
/// home and `CLAUDE_CONFIG_DIR` (the child inherits both). `None` with no
/// home.
pub(crate) fn native_memory_for_run(cwd: &Path) -> Option<NativeMemoryRecord> {
    let home = dirs::home_dir()?;
    let configured = std::env::var_os(crate::usage::claude_env::CONFIG_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    Some(native_memory_in(&home, configured.as_deref(), cwd))
}

/// What Claude Code loads by itself for a run in `cwd` under `home`, with
/// the default config folder `home/.claude`: the injectable form tests use.
#[cfg(test)]
pub(crate) fn native_memory(home: &Path, cwd: &Path) -> NativeMemoryRecord {
    native_memory_in(home, None, cwd)
}

fn native_memory_in(home: &Path, configured: Option<&Path>, cwd: &Path) -> NativeMemoryRecord {
    let config = configured
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".claude"));
    // Claude Code runs in the real path (getcwd) and names folders by it.
    let cwd = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let user = config.join("CLAUDE.md");
    let mut instructions = Vec::new();
    push_native_file(&mut instructions, "user", &user);
    let mut folders: Vec<&Path> = cwd.ancestors().collect();
    folders.reverse();
    for folder in folders {
        for (scope, file) in [
            ("project", folder.join("CLAUDE.md")),
            ("project", folder.join(".claude").join("CLAUDE.md")),
            ("local", folder.join("CLAUDE.local.md")),
        ] {
            // Home's `.claude/CLAUDE.md` is the user file, listed once.
            if file != user {
                push_native_file(&mut instructions, scope, &file);
            }
        }
    }
    let instruction_bytes = instructions.iter().map(|f| f.bytes).sum();
    let memory = config
        .join("projects")
        .join(claude_project_slug(&claude_memory_root(&cwd)))
        .join("memory");
    NativeMemoryRecord {
        config_dir: config.to_string_lossy().into_owned(),
        instructions,
        instruction_bytes,
        auto_memory: auto_memory(&memory),
        skill_listing: skill_listing(&config.join("skills")),
    }
}

/// The folder Claude Code keys a project's auto memory by (S13 review):
/// not the cwd but the canonical repository root. Walking up from `cwd` (a
/// real path) to the first folder holding a `.git`: a `.git` folder makes
/// that folder the root; a `.git` file (a linked worktree) names its
/// gitdir (`gitdir: <path>`, relative to that folder when relative), and
/// when that gitdir is a registered worktree of a common dir (see
/// [`worktree_main_root`]) the root is the main working copy, so every
/// worktree of a repository shares one memory folder. A `.git` file with
/// no `commondir` (a submodule), a stale or foreign back-pointer, or
/// anything that can't be followed leaves its own folder as the root.
/// `cwd` itself when no `.git` is found. The walk ends at `/`. Mirrors
/// Claude Code's canonical-root walk (CLI 2.1.301).
pub(crate) fn claude_memory_root(cwd: &Path) -> PathBuf {
    for folder in cwd.ancestors() {
        let dot_git = folder.join(".git");
        let Ok(meta) = fs::metadata(&dot_git) else {
            continue;
        };
        if meta.is_dir() {
            return folder.to_path_buf();
        }
        if meta.is_file() {
            return worktree_main_root(folder, &dot_git).unwrap_or_else(|| folder.to_path_buf());
        }
    }
    cwd.to_path_buf()
}

/// The main working copy of the linked worktree at `folder`, from its
/// `.git` file, as Claude Code finds it. The gitdir's `commondir`
/// (relative to the gitdir) names the common dir, which is followed only
/// when the gitdir sits in `<common>/worktrees` and its back-pointer
/// (`<gitdir>/gitdir`, relative to the gitdir) names this `.git` file; a
/// moved or copied worktree fails that and keys its own folder. A common
/// dir named `.git` gives its parent; any other name (a bare `repo.git`,
/// a `.bare` layout) gives the common dir itself, unless it holds a
/// `.git` of its own. Paths compare as real paths when they exist, else
/// lexically. `None` (key `folder`) when anything is missing, unreadable,
/// or doesn't match.
fn worktree_main_root(folder: &Path, dot_git: &Path) -> Option<PathBuf> {
    // Every file read is one line; read a bounded head of each.
    fn head(path: &Path) -> Option<String> {
        let mut text = String::new();
        fs::File::open(path)
            .ok()?
            .take(4096)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }
    // The trimmed text of the file at `path`, when not empty.
    fn line(path: &Path) -> Option<String> {
        let text = head(path)?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }
    let text = head(dot_git)?;
    let gitdir = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))?
        .trim();
    if gitdir.is_empty() {
        return None;
    }
    // `join` keeps an absolute gitdir as it is.
    let gitdir = real_or_lexical(&folder.join(gitdir));
    let common = real_or_lexical(&gitdir.join(line(&gitdir.join("commondir"))?));
    // Registered under the common dir's `worktrees`.
    if real_or_lexical(gitdir.parent()?) != real_or_lexical(&common.join("worktrees")) {
        return None;
    }
    // The back-pointer names this very `.git` file.
    let back = line(&gitdir.join("gitdir"))?;
    if real_or_lexical(&gitdir.join(back)) != real_or_lexical(folder).join(".git") {
        return None;
    }
    if common.file_name().is_some_and(|name| name == ".git") {
        return common.parent().map(Path::to_path_buf);
    }
    // A bare common dir is the project itself, unless it has a `.git`.
    if fs::metadata(common.join(".git")).is_ok_and(|m| m.is_dir() || m.is_file()) {
        return None;
    }
    Some(common)
}

/// `path` as a real path when it exists, else with `.` and `..` folded
/// lexically (`..` at the root stays at the root).
fn real_or_lexical(path: &Path) -> PathBuf {
    use std::path::Component;
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => match out.components().next_back() {
                Some(Component::Normal(_)) => {
                    out.pop();
                }
                Some(Component::RootDir | Component::Prefix(_)) => {}
                _ => out.push(part),
            },
            other => out.push(other),
        }
    }
    out
}

/// Claude Code's folder name for a project path under `<config>/projects`:
/// every character but an ASCII letter or digit becomes `-`, one per UTF-16
/// unit (its JavaScript replace counts those), so `/Users/a/Alex’s.x`
/// is `-Users-a-Alex-s-x`. Claude Code may shorten a very long slug;
/// that isn't followed (the record then says the folder doesn't exist).
pub(crate) fn claude_project_slug(path: &Path) -> String {
    let mut slug = String::new();
    for c in path.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else {
            for _ in 0..c.len_utf16() {
                slug.push('-');
            }
        }
    }
    slug
}

fn push_native_file(out: &mut Vec<NativeFileRecord>, scope: &str, path: &Path) {
    let Ok(meta) = fs::metadata(path) else {
        return;
    };
    if meta.is_file() {
        out.push(NativeFileRecord {
            scope: scope.into(),
            path: path.to_string_lossy().into_owned(),
            bytes: meta.len(),
        });
    }
}

fn auto_memory(dir: &Path) -> AutoMemoryRecord {
    let mut record = AutoMemoryRecord {
        dir: dir.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return record;
    };
    record.exists = true;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        if entry.file_name().to_str() == Some("MEMORY.md") {
            // Read only to count its lines; the text is dropped here.
            if let Ok(bytes) = fs::read(&path) {
                record.memory_md_bytes = bytes.len() as u64;
                let lines = String::from_utf8_lossy(&bytes).lines().count();
                record.memory_md_lines = u32::try_from(lines).unwrap_or(u32::MAX);
            }
        } else {
            record.notes += 1;
        }
    }
    record
}

fn skill_listing(root: &Path) -> SkillListingRecord {
    let mut record = SkillListingRecord::default();
    let Ok(entries) = fs::read_dir(root) else {
        return record;
    };
    for entry in entries.flatten() {
        // `is_file` follows a symlinked skill folder.
        let skill = entry.path().join("SKILL.md");
        if !skill.is_file() {
            continue;
        }
        record.count = record.count.saturating_add(1);
        if record.count > SKILL_READ_LIMIT {
            continue;
        }
        let mut head = Vec::new();
        if let Ok(file) = fs::File::open(&skill) {
            let _ = file.take(SKILL_HEAD_BYTES).read_to_end(&mut head);
        }
        let (description, _, name) =
            crate::skills::parse_skill_meta(&String::from_utf8_lossy(&head));
        let name = name.unwrap_or_else(|| entry.file_name().to_string_lossy().into_owned());
        record.bytes += (name.len() + description.len()) as u64;
    }
    record
}

/// What the host context compiler (`context_compiler.rs`) decided for one
/// chat turn's message body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestCompiler"))]
#[serde(rename_all = "camelCase")]
pub struct CompilerRecord {
    /// `composer` (a UI chat: vault, quick or workspace) or `appSession` (a
    /// conversation an agent started or messaged through `hollow_app`).
    pub entry: String,
    /// The native session the body was compiled for: `new`, `resume`,
    /// `reset` (Hollow moved the conversation to a fresh one) or `retry` (a
    /// failed resume retried fresh inside the turn).
    pub session: String,
    /// A reset's reason (`chat_resume::ResetReason::wire`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reset_reason: Option<String>,
    /// What a config-changed reset names as changed (OB03): fingerprint
    /// parts (`chat_resume::PARTS`), or `unknown` when the stint stored
    /// none. Absent on other turns and on manifests from before 2026-10-17.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<String>>", optional))]
    pub reset_parts: Vec<String>,
    pub profile: ProfileRecord,
    pub continuity: ContinuityRecord,
    /// The conversation's task-state record (F4, K2): its revision and size,
    /// and whether this body carries it.
    #[serde(default)]
    pub task_state: TaskStateRecord,
    /// The "updated since your session started" delta of the operator
    /// profile on a resumed session (F4, K2).
    #[serde(default)]
    pub profile_delta: ProfileDeltaRecord,
    /// How this body got the visual guide (agent effectiveness F6, wave
    /// 3 · G1; `visual_prompt::delivery`). Absent on manifests from before
    /// 2026-10-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub visual_guide: Option<VisualGuideRecord>,
    /// The turn's total context budget and what each layer got (agent
    /// effectiveness F6, wave 3 · G2; `context_budget.rs`). Absent on
    /// manifests from before 2026-10-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub budget: Option<BudgetRecord>,
    /// Whether the configured half and the workspace envelope went in full
    /// or as what changed since the native session was given them (G2;
    /// `stable_layers.rs`). Absent on manifests from before 2026-10-07.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub stable_layers: Option<StableLayersRecord>,
}

/// One chat turn's total context budget (`context_budget::allocate`).
/// Sizes only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestBudget"))]
#[serde(rename_all = "camelCase")]
pub struct BudgetRecord {
    /// The agent (`claude`, `codex`, `antigravity`, `grok`).
    pub provider: String,
    /// The provider's total, in UTF-8 bytes.
    pub total: u64,
    /// Where the configured half goes: `systemPrompt` (Claude) or `prompt`.
    pub configured_channel: String,
    /// What the layers were granted together.
    pub used: u64,
    /// The never-cut layers alone exceed the total (nothing settled is
    /// dropped to fit).
    pub over: bool,
    /// Each layer's ask and grant, in reading order; a grant below its ask
    /// is a cut.
    pub layers: Vec<BudgetLayer>,
}

/// One layer's ask and grant.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestBudgetLayer"))]
#[serde(rename_all = "camelCase")]
pub struct BudgetLayer {
    pub name: String,
    /// `fixed`, `request`, `taskState`, `reserved`, `evidence`,
    /// `profileProse` or `guidance`.
    pub tier: String,
    pub wanted: u64,
    pub granted: u64,
}

/// How a chat turn carried its stable layers (`stable_layers.rs`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestStableLayers"))]
#[serde(rename_all = "camelCase")]
pub struct StableLayersRecord {
    /// The configured half: `systemPrompt` (Claude: in every process),
    /// `full` (in the prompt) or `held` (a resumed Codex, Antigravity or
    /// Grok session holds it, so the prompt carries a one-line reference).
    pub configured: String,
    /// The workspace envelope: `none`, `full`, `delta` or `child-slim` (a
    /// durable child task's slim envelope with a packet, sent whole; S01).
    pub workspace: String,
    /// Why a resumed turn got them in full: `noBaseline`, `otherSession`,
    /// `refresh` or `budget`; absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
    /// The envelope in full and as sent, UTF-8 bytes.
    pub full_bytes: u64,
    pub sent_bytes: u64,
    /// In a delta: workspace documents unchanged, and changed (sent as
    /// their changed lines or whole); attached-source excerpts named by
    /// range because the session already has them.
    pub unchanged: u32,
    pub changed: u32,
    pub excerpts_referenced: u32,
    /// Resumed turns since the session was last given them in full.
    pub resumed_since_full: u32,
    /// The per-turn layers a resumed turn left out because its session was
    /// given each one as is (S02): `recall`, `chatIntent`, `skillIndex`,
    /// `permissionMode`. Absent when all were sent.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<String>>", optional))]
    pub held_layers: Vec<String>,
}

/// The task-state record (`task_state.rs`) at this turn. Counts and sizes
/// only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestTaskState"))]
#[serde(rename_all = "camelCase")]
pub struct TaskStateRecord {
    /// The record's revision (bumped when its content changes; 0 before
    /// any), as folded for this turn.
    pub revision: u64,
    /// The last turn folded in.
    pub through_turn: u64,
    /// Items in the record (the sent block's when sent).
    pub items: u32,
    /// The body carries the `<task-state>` block (a fresh session, or a
    /// resumed one it re-grounds: `reground`).
    pub sent: bool,
    /// The block's UTF-8 size; 0 when not sent.
    pub bytes: u64,
    /// Earlier conversations whose records were merged in (a continued
    /// conversation).
    #[serde(default)]
    pub merged: u32,
    /// Why a resumed session got the block again (repo features R1):
    /// `compacted` (its agent reported compacting its history since the
    /// last turn that finished) or `refresh` (the resumed turn after 8
    /// resumed turns of deltas, for an agent whose compaction Hollow can't
    /// see). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reground: Option<String>,
}

/// The operator-profile revision delta (`<operator-update>`) at this turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestProfileDelta"))]
#[serde(rename_all = "camelCase")]
pub struct ProfileDeltaRecord {
    /// What the native session was last given: `stored` (a baseline from
    /// this or an earlier turn), `adopted` (none was recorded, so the current
    /// profile was taken as what it holds, and nothing is sent), `fresh`
    /// (this turn starts the session and records one) or `off` (About you is
    /// off; nothing compared).
    pub baseline: String,
    /// The body carries the `<operator-update>` block.
    pub sent: bool,
    /// Changed lines: added and removed.
    pub added: u32,
    pub removed: u32,
    /// The `me/` files with changes.
    #[serde(default)]
    pub files: Vec<String>,
    /// The block's UTF-8 size; 0 when not sent.
    pub bytes: u64,
}

/// The operator profile (`operator::compile`, About you) at this turn.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestProfile"))]
#[serde(rename_all = "camelCase")]
pub struct ProfileRecord {
    /// About you is switched on (`operator::is_enabled`).
    pub enabled: bool,
    /// The body carries the `<operator-context>` block.
    pub sent: bool,
    /// The digest's UTF-8 size; 0 when off or empty.
    pub bytes: u64,
    /// The digest's SHA-256: its revision. Absent when off or empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub sha256: Option<String>,
    /// `me/` files in the digest.
    pub files: u32,
    /// A file or the whole digest hit its cap.
    pub truncated: bool,
}

/// The conversation bundle for a fresh native session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestContinuity"))]
#[serde(rename_all = "camelCase")]
pub struct ContinuityRecord {
    /// The body carries the `<conversation-context>` block.
    pub sent: bool,
    /// User and assistant entries before this turn (read only for a fresh
    /// session; 0 on a resume).
    pub available: u32,
    /// Entries kept in the bundle.
    pub entries: u32,
    /// The block's UTF-8 size.
    pub bytes: u64,
    /// Earlier conversations whose recorded turns lead the bundle (a
    /// Continue from summary or a folder-vanished reset; agent
    /// effectiveness F4); 0 for every other turn.
    #[serde(default)]
    pub continued_from: u32,
    /// User and assistant entries those conversations contributed to
    /// `available`.
    #[serde(default)]
    pub seeded: u32,
}

/// One context layer: its name, UTF-8 size and SHA-256 (hex). Layer names:
/// `visualPrompt`, `environmentBrief` and `policy` (the appended system
/// prompt, in that order), `workspace` (the workspace envelope: brief, plan
/// and attached sources), `operatorProfile`, `recall`, `clock`,
/// `contextNote` (the turn's `<hollow-context>` line, repo features R2b),
/// `permissionMode` (the turn's `<hollow-mode>` line, HB01; not `chatMode`,
/// the composer's intent), `connectorsNote`,
/// `retrievalEvidence`, and from the composed message `conversationContext`,
/// `taskState` and `operatorUpdate` (the context compiler's, F4 K2),
/// `pageContext`, `chatMode`, `chatIntent`, `request` (the user's own words),
/// `attachedImages` and `attachedFiles`. An empty layer is left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestLayer"))]
#[serde(rename_all = "camelCase")]
pub struct Layer {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}

/// What retrieval considered and kept.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RetrievalRecord {
    /// The connected project it was scoped to
    /// (`retrieval_broker::resolve_scope`); `None`: every one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub scope: Option<String>,
    /// How the scope was resolved: `workspace` (a connected project's
    /// workspace), `workspace-disconnected` (a project's workspace whose
    /// project left the index: scoped to its id, so no project evidence),
    /// `notion-twin` (a Notion project's workspace named like a connected
    /// project), `build-workspace` (the project its worktree was created from),
    /// `project-folder`, `git-worktree`, `explicit` or `none`. Absent in
    /// manifests written before 2026-10-06.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub scope_basis: Option<String>,
    /// The workspace the run's folder belongs to, when it is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub workspace: Option<String>,
    /// `local`, `jev` or `jev-fallback`.
    pub engine: String,
    /// The broker's ranked pool, in its order (lexical and, when available,
    /// semantic blended); `kept` marks what reached the prompt.
    pub candidates: Vec<RetrievalCandidate>,
    /// Coverage notices (connection state, scan budgets, what Jev left out,
    /// what the evidence budget left out), Hollow's own words, at most 12.
    pub notices: Vec<String>,
    /// What the broker scanned and how long it took (agent effectiveness
    /// F3). Absent in manifests written before 2026-10-06.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub stats: Option<RetrievalStats>,
    /// What a chat turn's query was read from (overnight OB18, OB25):
    /// `request` (the turn's own words), `hostSubject` (a wake's chats or a
    /// check-in's new signals), `followup+prior` (a short follow-up with its
    /// conversation's topic), or `skipped-host` / `skipped-followup`
    /// (nothing retrieved; the last connector decision kept). Absent on
    /// other runs, a bare turn and manifests written before 2026-10-17.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub query_basis: Option<String>,
    /// The run's final reply was read for the kept candidates it names
    /// (S26, [`RetrievalCandidate::cited`]), so a kept candidate without
    /// `cited` was not named. Absent when no reply was read (a run with no
    /// reply or output) and on manifests from before 2026-10-17.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub cited_checked: bool,
}

/// What one retrieval scanned, ranked and kept, and how long it took.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestRetrievalStats"))]
#[serde(rename_all = "camelCase")]
pub struct RetrievalStats {
    /// The vault scan's prefilter: `fts` (the index, in the app),
    /// `fts-read-only` (the app's index read from another process) or
    /// `linear` (none).
    pub prefilter: String,
    /// How the vault notes to read were chosen (OB12): `bm25`, the index's
    /// best 200 plus name matches, the 20 newest and any it can't vouch for;
    /// or `linear`, every eligible note. Absent when the vault wasn't asked
    /// and in manifests written before 2026-10-17.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub candidate_source: Option<String>,
    /// Vault notes chosen to be read (`candidateSource`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub candidates: Option<u64>,
    /// Vault notes eligible for retrieval.
    pub notes_eligible: u64,
    /// Vault notes read (the rest are skipped unread).
    pub notes_read: u64,
    /// Vault notes that matched and were scored.
    pub notes_matched: u64,
    pub bytes_read: u64,
    /// Bytes charged to the scoring budget: matching notes only.
    pub bytes_scored: u64,
    /// `budget` or `read-limit` when the vault scan stopped early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub stopped: Option<String>,
    /// Connected-project text files read.
    pub project_files_read: u64,
    /// Passages ranked into the pool the reranker sees.
    pub pool: u32,
    /// The final evidence budget (passages).
    pub budget: u32,
    /// `blended`, `off`, `unavailable` or (F8, O2) `deadline`: the
    /// preparation deadline passed before the semantic half answered.
    pub semantic: String,
    /// Why semantic candidates were not blended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub semantic_reason: Option<String>,
    /// Notes the semantic half contributed (above its floor).
    pub semantic_hits: u32,
    /// Query embedding and lookup time, when it ran.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub semantic_ms: Option<u64>,
    /// When the semantic half ran, on the monotonic clock (F8), for the
    /// timeline. In memory only.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub(crate) semantic_mark: crate::prep_timing::InMemory<Option<crate::prep_timing::Mark>>,
    /// The broker's wall time: candidates and ranking, reranking excluded.
    pub ms: u64,
}

/// One retrieved passage.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RetrievalCandidate {
    /// Vault-relative for a vault note; a connected project's file keeps its
    /// absolute path (it is outside the vault, and cited that way).
    pub path: String,
    pub line: u64,
    /// `connected_project` or the vault note's kind.
    pub source: String,
    /// `excerpt`, `too_large` or `requires_reader`.
    pub coverage: String,
    /// 1-based position in the broker's ranked pool.
    pub rank: u32,
    /// Distinct query terms the chosen line (or, unread, the file name)
    /// matched.
    pub local_score: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub jev_score: Option<f64>,
    /// Cosine similarity of the note's best semantic chunk, when the
    /// semantic half was blended and found it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub semantic_score: Option<f64>,
    pub kept: bool,
    /// `local`, `jev` or `jev-fallback`.
    pub basis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
    /// Of the excerpt.
    pub bytes: u64,
    pub sha256: String,
    /// A kept candidate the run's final reply names (S26): its path, or its
    /// file name with or without `.md` when that name is at least 4
    /// characters (only with it for a generic or date-shaped name,
    /// [`cites`]). Absent when it isn't, and on manifests from
    /// before 2026-10-17 (see [`RetrievalRecord::cited_checked`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[cfg_attr(test, ts(as = "Option<bool>", optional))]
    pub cited: bool,
}

/// The shortest file name (without `.md`) a reply can cite by name alone
/// (S26): a shorter one (`faq`, `ops`) is named by chance.
const MIN_CITED_NAME_CHARS: usize = 4;

/// File names (lowercase, without `.md`) so common a reply says them by
/// chance (S26): one cites only with its `.md` or by its path.
const GENERIC_NOTE_NAMES: &[&str] = &[
    "readme",
    "plan",
    "index",
    "notes",
    "workspace",
    "sources",
    "summary",
    "agents",
    "claude",
    "todo",
    "build_log",
];

/// A date-shaped file name (`2026-10-17`, `2026-10`, `20261017`): digits and
/// `-` only, four digits or more. Held like [`GENERIC_NOTE_NAMES`].
fn date_shaped(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_digit() || c == '-')
        && name.chars().filter(char::is_ascii_digit).count() >= 4
}

/// Whether `reply` names the note at `path` (S26): its path (without its
/// `.md`), or its file name without `.md` when that is at least
/// [`MIN_CITED_NAME_CHARS`] long (only with its `.md` when the name is
/// generic, [`GENERIC_NOTE_NAMES`], or date-shaped), compared
/// case-insensitively and only as a whole word (no letter, digit, `_` or
/// `-` on either side; a following `.md` is fine).
pub(crate) fn cites(reply: &str, path: &str) -> bool {
    let reply = reply.to_lowercase();
    let path = path.trim().to_lowercase();
    let separator = |c: char| c == '/' || c == '\\';
    let whole = path.strip_suffix(".md").unwrap_or(&path);
    let name = whole.rsplit(separator).next().unwrap_or("");
    let mut needles: Vec<String> = Vec::new();
    // A path with no folder is only its name, held to the name's length.
    if whole.contains(separator) {
        needles.push(whole.to_string());
    }
    if name.chars().count() >= MIN_CITED_NAME_CHARS {
        if GENERIC_NOTE_NAMES.contains(&name) || date_shaped(name) {
            needles.push(format!("{name}.md"));
        } else {
            needles.push(name.to_string());
        }
    }
    let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '-');
    needles.iter().filter(|n| !n.is_empty()).any(|needle| {
        reply.match_indices(needle.as_str()).any(|(at, _)| {
            !word(reply[..at].chars().next_back())
                && !word(reply[at + needle.len()..].chars().next())
        })
    })
}

/// What the broker did with a workspace's attached sources for one request
/// (`source_evidence::select`): the budget, what it read, and one decision
/// per recorded chunk. Paths, line ranges, scores and sizes; never text.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestAttachedSources"))]
#[serde(rename_all = "camelCase")]
pub struct AttachedSourcesRecord {
    /// Bytes of attached text the envelope may carry.
    pub budget: u64,
    /// Bytes of attached text it carried.
    pub used: u64,
    /// How the agent can read the rest: `read_source` (hollow_vault is
    /// attached) or `own-tools` (its own file tools, with the path).
    pub reader: String,
    /// Search terms the request gave (their count; never the words).
    pub terms: u32,
    /// Bytes of attached text read and chunked.
    pub bytes_searched: u64,
    /// Chunks made, relevant chunks (since OF06, 2026-10-17: those past the
    /// relevance floor, or the capped fallback's when none did; every
    /// matching chunk when the request asked for the whole document or all
    /// attached text fit), and matching or kept
    /// chunks not listed in `chunks` (past its cap).
    pub chunks_total: u32,
    pub chunks_relevant: u32,
    pub chunks_unrecorded: u32,
    /// The 128 KB envelope trim dropped every excerpt after selection.
    #[serde(default)]
    pub trimmed: bool,
    /// One per listed source, in listing order.
    pub files: Vec<AttachedFileRecord>,
    /// Kept chunks and matching chunks left out, in the order the broker
    /// considered them.
    pub chunks: Vec<ChunkDecision>,
    pub ms: u64,
}

/// One attached file's outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestAttachedFile"))]
#[serde(rename_all = "camelCase")]
pub struct AttachedFileRecord {
    /// Vault-relative.
    pub path: String,
    /// `complete`, `excerpt`, `not_selected`, `budget_exceeded` or
    /// `requires_reader`.
    pub coverage: String,
    pub lines: u64,
    pub total_bytes: u64,
    pub included_bytes: u64,
    /// SHA-256 of the included text (empty when none).
    pub sha256: String,
}

/// One chunk the broker considered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestChunkDecision"))]
#[serde(rename_all = "camelCase")]
pub struct ChunkDecision {
    /// Vault-relative.
    pub path: String,
    /// 1-based, inclusive: what was kept, or the whole chunk when omitted.
    pub from_line: u64,
    pub to_line: u64,
    /// Distinct request terms in the chunk plus those in the file's name;
    /// 0 for a chunk kept only to fill leftover budget or to send a
    /// one-chunk file whole (its `reason` says which).
    pub score: u32,
    pub kept: bool,
    /// Kept: bytes included. Omitted: the chunk's bytes.
    pub bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub reason: Option<String>,
}

/// The lifecycle policy's record for one run (`note_lifecycle`): whether the
/// request asked about the past, and one decision per note it touched.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestLifecycle"))]
#[serde(rename_all = "camelCase")]
pub struct LifecycleRecord {
    /// The request asks about the past: superseded notes were kept, labelled
    /// historical, behind their current versions.
    pub history: bool,
    /// What made it historical: a word from Hollow's fixed list (`before`,
    /// `used to`, `history`, `changed`, …) or `a past date`. Never the
    /// request's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub cue: Option<String>,
    /// At most [`MAX_LIFECYCLE`], recall's and retrieval's together.
    pub decisions: Vec<LifecycleDecision>,
    /// Decisions past the cap, counted.
    pub unrecorded: u32,
}

/// One lifecycle decision: which route, which note, its status, what was
/// done and Hollow's reason.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestLifecycleDecision"))]
#[serde(rename_all = "camelCase")]
pub struct LifecycleDecision {
    /// `recall` or `retrieval`.
    pub route: String,
    /// Vault-relative.
    pub path: String,
    /// `superseded`, `current`, `duplicate` or `unapproved`.
    pub status: String,
    /// `excluded` (superseded, current-state request), `historical` (kept
    /// and labelled), `promoted` (a current version ranked in place of what
    /// it replaced), `current` (it already ranked ahead), `collapsed` (an
    /// exact duplicate) or `labelled` (unapproved).
    pub action: String,
    /// The current version, for a superseded note or a duplicate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub current: Option<String>,
    pub reason: String,
}

/// Lifecycle decisions kept per run.
pub const MAX_LIFECYCLE: usize = 32;

/// One Jev use for this run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestJevUse"))]
#[serde(rename_all = "camelCase")]
pub struct JevUse {
    /// `passages` (evidence reranking) or `connectors` (relevance).
    #[serde(rename = "use")]
    #[cfg_attr(test, ts(rename = "use"))]
    pub kind: String,
    /// `ok`, `timeout`, `malformed`, `error` (unreachable, refused, rate
    /// limited) or `off` (switched off or no key: nothing was sent); since
    /// F8's O2 also `cached` (an identical request's ranking reused, or
    /// since S28 the conversation's last connector decision, nothing
    /// sent), `deadline` (the preparation deadline cut it, or had passed
    /// before it was sent) and `circuit-open` (Jev paused after repeated
    /// failures, nothing sent).
    pub outcome: String,
    /// Candidates sent (0 when off, paused or too late; for `cached`, the
    /// candidates the reused ranking or decision covered).
    pub items: u32,
    /// Wall time of the request; `None` when nothing was sent.
    #[serde(default)]
    pub latency_ms: Option<u64>,
    /// Always `None`: TypeSafe reports no cost per call, and an unknown cost
    /// is never written as 0. Serialized as `null`.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    /// When the request was sent, on the monotonic clock (F8), for the
    /// timeline; `None` when nothing was sent. In memory only.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub(crate) mark: crate::prep_timing::InMemory<Option<crate::prep_timing::Mark>>,
    /// Original score time, including an enqueue preparation's cache hit.
    /// Not a preparation mark and never serialized.
    #[serde(skip)]
    #[cfg_attr(test, ts(skip))]
    pub(crate) decision_at: crate::prep_timing::InMemory<Option<std::time::Instant>>,
}

impl JevUse {
    /// The record of one call: `outcome` is what the scorer returned (`None`:
    /// Jev is off), `items` how many candidates were sent.
    pub(crate) fn of<T>(
        kind: &str,
        outcome: Option<&Result<T, String>>,
        items: usize,
        latency_ms: u64,
    ) -> Self {
        let (outcome, items, latency_ms) = match outcome {
            None => ("off".to_string(), 0, None),
            Some(Ok(_)) => ("ok".to_string(), items, Some(latency_ms)),
            Some(Err(e)) => (
                crate::jev::outcome_of(e).to_string(),
                items,
                Some(latency_ms),
            ),
        };
        JevUse {
            kind: kind.into(),
            outcome,
            items: u32::try_from(items).unwrap_or(u32::MAX),
            latency_ms,
            cost_usd: None,
            mark: Default::default(),
            decision_at: Default::default(),
        }
    }

    /// The call started at `started` (F8): with its latency, the timeline's
    /// mark. Nothing for a use that sent nothing.
    pub(crate) fn sent_at(mut self, started: std::time::Instant) -> Self {
        self.mark = crate::prep_timing::InMemory(
            self.latency_ms
                .map(|ms| crate::prep_timing::Mark { started, ms }),
        );
        self
    }

    /// How the call was served (F8, `jev::Guard`): a reused ranking reads
    /// `cached` (`items` are the candidates it covered); a paused Jev
    /// (`circuit-open`) and a deadline that had already passed (`deadline`)
    /// sent nothing (0 items). None of the three has a latency or a mark.
    pub(crate) fn served(&mut self, how: crate::jev::Served) {
        use crate::jev::Served;
        let outcome = match how {
            Served::Sent => return,
            Served::Cached => "cached",
            Served::CachedAt(at) => {
                self.decision_at = crate::prep_timing::InMemory(Some(at));
                "cached"
            }
            Served::Paused => {
                self.items = 0;
                "circuit-open"
            }
            Served::Late => {
                self.items = 0;
                "deadline"
            }
        };
        self.outcome = outcome.into();
        self.latency_ms = None;
        self.mark = Default::default();
    }
}

/// The capability set the run was given.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestCapabilities"))]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    /// Hollow-owned MCP servers (`mcp_attach`), by name.
    pub owned_servers: Vec<String>,
    /// Library connectors attached ("always" and the relevant ones), by id.
    pub connectors: Vec<String>,
    /// The inline cap on one Bash tool result, in characters, the run's
    /// command carried (OB23, `agent::bash_output_cap`): Claude with tools.
    /// Absent for every other agent, a tool-free run and on manifests from
    /// before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub bash_output_max_chars: Option<u32>,
    /// The compaction window the run's command carried, in tokens (OB22,
    /// `agent::compact_window`): Claude and Codex when Settings sets one.
    /// Absent when it is off (the default), for other agents and on
    /// manifests from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub compact_window: Option<u32>,
    /// How many times the agent compacted its context during the run
    /// (OB22, the receipt's `compactions`), set when the receipt is
    /// written. Absent when it never did and on manifests from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub compactions: Option<u32>,
}

/// One place preparation fell back: `stage` (`passages`, `connectors`,
/// `session`) and Hollow's own description.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestFallback"))]
#[serde(rename_all = "camelCase")]
pub struct Fallback {
    pub stage: String,
    pub detail: String,
}

/// Preparation phases measured here (the receipt's `phaseTimings.spawnMs`
/// is their whole). Unmeasured is `None`, never 0.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestPreparation"))]
#[serde(rename_all = "camelCase")]
pub struct Preparation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub retrieval_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub connectors_ms: Option<u64>,
    /// Every preparation step's start and duration (agent effectiveness
    /// F8). Absent on a run that was never admitted and on manifests from
    /// before wave 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub timeline: Option<PrepTimeline>,
    /// The one preparation deadline retrieval and connector selection ran
    /// under, and the optional parts it cut (agent effectiveness F8, O2,
    /// `prep_deadline`). Absent on manifests from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub deadline: Option<PrepDeadlineRecord>,
    /// Codex's native MCP inventory (agent effectiveness F8, O3,
    /// `codex_inventory`): `listed` (`codex mcp list` ran) or `cached` (an
    /// identical listing for the same binary, config and folder was
    /// reused; nothing started). Absent for every other agent and on
    /// manifests from before it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub codex_inventory: Option<String>,
}

/// What the preparation deadline did on one run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestPrepDeadline"))]
#[serde(rename_all = "camelCase")]
pub struct PrepDeadlineRecord {
    /// The bound, from the start of retrieval and connector selection.
    pub budget_ms: u64,
    /// The optional parts that fell back because it passed, by timeline
    /// step: `retrievalJev`, `connectorsJev` (local decisions),
    /// `retrievalSemantic` (lexical only), `microsoftAuth` (the refresh
    /// finishes on its own). Empty when nothing was cut.
    pub cut: Vec<String>,
}

/// A run's preparation timeline (`prep_timing::timeline`): offsets from
/// admission's start (`origin`), in milliseconds on the monotonic clock.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestPrepTimeline"))]
#[serde(rename_all = "camelCase")]
pub struct PrepTimeline {
    /// Always `admission`: the clock's 0 is admission's start.
    pub origin: String,
    /// Admission's start → the agent's spawn; `null` when it never spawned.
    pub total_ms: Option<u64>,
    /// Every step (`prep_timing::Step`), those that ran by start offset,
    /// then those that didn't. Nested steps keep their parent's time.
    pub phases: Vec<PhaseTiming>,
}

/// One preparation step: both `null` when it didn't run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestPhaseTiming"))]
#[serde(rename_all = "camelCase")]
pub struct PhaseTiming {
    pub name: String,
    pub start_ms: Option<u64>,
    pub duration_ms: Option<u64>,
}

/// What a receipt holds about its manifest.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRef {
    pub schema_version: u32,
    pub bytes: u64,
    pub sha256: String,
}

/// What `run_context_manifest` returns: the run it belongs to, and the
/// manifest.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RunContextView {
    pub run_id: String,
    /// `chat`, `workflow` or (since O3) `heartbeat`.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub session_key: Option<String>,
    pub finished_at: String,
    pub manifest: RunManifest,
    /// Jev duplicate checks for this run that ran after its receipt (a
    /// chat turn's learning proposals; F11, O3), from this process's
    /// memory (`jev_decide::for_run`): lost on restart. Absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<crate::jev_decide::Report>>", optional))]
    pub after_run: Vec<crate::jev_decide::Report>,
    /// Every Jev call this run made, in one place (F11, O3): the manifest's
    /// `jev` uses and the after-run checks' requests.
    pub jev_calls: JevCalls,
    /// The delegations this run made (F10, D1), oldest first, from the
    /// session store (kept 90 days, at most 32 per run). Absent when none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[cfg_attr(test, ts(as = "Option<Vec<DelegationRecord>>", optional))]
    pub delegations: Vec<DelegationRecord>,
    /// The run's own cost and each descendant's (F10, D2), read now
    /// (`descendants::rollup`): a child task still running shows `null`
    /// until it ends. `stop` on a child comes from the receipt.
    pub cost_rollup: CostRollup,
}

/// A run's cost with its descendants' (agent effectiveness F10, wave 4 ·
/// D2): its own agent invocations, each bounded consultation
/// (`hollow_team`), each durable child task it started (that turn's whole
/// cost, its own descendants included) and each helper one-shot run for it
/// (the learning check). A cost nobody reported stays `null`, never 0, and
/// a sum with an unknown part is `null` too; `knownUsd` adds only what is
/// known. Every invocation is counted once: a child's descendants are in
/// the child's cost, not listed again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestCostRollup"))]
#[serde(rename_all = "camelCase")]
pub struct CostRollup {
    /// The run's own usage rows (every attempt); `null` when none was
    /// found or any is unknown.
    pub own_usd: Option<f64>,
    /// The descendants' costs summed; `null` when any is unknown (0 with
    /// none).
    pub descendants_usd: Option<f64>,
    /// `ownUsd` + `descendantsUsd`; `null` when either is.
    pub total_usd: Option<f64>,
    /// Every known cost summed (an unknown one adds nothing).
    pub known_usd: f64,
    /// How many costs are unknown: the run's own, and each descendant's.
    pub unknown: u32,
    /// How many costs are billed by GPU time (a self-hosted model, Q8): the
    /// run's own, and each descendant's. Never priced and never unknown
    /// either: no Anthropic spend, so they add nothing to `knownUsd` and a
    /// budget counts them as nothing, but a sum with one stays `null` (shown
    /// as billed by GPU time, never $0).
    #[serde(default, skip_serializing_if = "is_zero")]
    #[cfg_attr(test, ts(as = "Option<u32>", optional))]
    pub gpu_time: u32,
    pub descendants: Vec<Descendant>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

// Costs are finite, never NaN, so equality is total: a receipt (which is
// `Eq`) can carry a roll-up.
impl Eq for CostRollup {}
impl Eq for Descendant {}

/// One descendant of a run ([`CostRollup`]).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestDescendant"))]
#[serde(rename_all = "camelCase")]
pub struct Descendant {
    /// `consultation`, `childTask` or `helper`.
    #[cfg_attr(test, ts(type = "\"consultation\" | \"childTask\" | \"helper\""))]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub agent: Option<String>,
    /// A consultation's or helper's usage outcome (`success`, `cancelled`,
    /// `interrupted`, `running`, …); a child task's turn as its queue row
    /// says (`queued`, `running`, `done`, `failed`, `cancelled`, …, or
    /// `unknown` once the row is gone).
    pub status: String,
    pub cost_usd: Option<f64>,
    /// A helper's purpose (`learning_extract`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub purpose: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub child_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub child_queue_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub child_run_id: Option<String>,
    /// What the parent's stop did to a child task's turn: `cancelled` (it
    /// was queued), `stopRequested` (claimed, ended before it starts),
    /// `signalled` (running: stopped like the composer's Stop) or
    /// `finished` (it had already ended). Absent when the parent wasn't
    /// stopped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub stop: Option<String>,
}

/// One delegation a run made (agent effectiveness F10, wave 4 · D1): a
/// bounded consultation (`hollow_team` `ask_agent`) or a durable child
/// task (`hollow_app` `start_session` / `send_to_session`), with its task
/// packet's size and SHA-256 (never its text) and its budget. Kept in the
/// session store (`delegation::store`), outside the vault, and listed by
/// the run's manifest view.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestDelegation"))]
#[serde(rename_all = "camelCase")]
pub struct DelegationRecord {
    /// `consultation` or `childTask`.
    #[cfg_attr(test, ts(type = "\"consultation\" | \"childTask\""))]
    pub kind: String,
    /// `ask_agent`, `start_session`, `send_to_session`, `handover`, or
    /// `wake`: a woken turn of a child task (`wake_me::link_to_delegator`).
    pub tool: String,
    /// The agent asked (`claude`, `codex`, `antigravity`, `grok`).
    pub agent: String,
    /// A child task's conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub child_session: Option<String>,
    /// A child task's turn: its run-queue row (F10, D2), which says whether
    /// it is still queued or running, how it ended and which run it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub child_queue_id: Option<i64>,
    pub packet_bytes: u64,
    pub packet_sha256: String,
    /// Constraints and sources the packet holds, and how many of them the
    /// host added from the asking conversation (its task state, its
    /// request's named files).
    pub constraints: u32,
    pub sources: u32,
    pub from_task_state: u32,
    /// Items left out by the packet's caps.
    pub omitted: u32,
    /// The packet's budget: seconds and the cost ceiling asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub budget_seconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub budget_usd: Option<f64>,
    /// What Hollow enforces: a consultation's run deadline in seconds.
    /// Absent for a child task (Hollow doesn't stop it at its budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub deadline_seconds: Option<u64>,
    pub at: String,
    /// How a consultation ended (OF04): `ok`, `timeout` (its deadline),
    /// `error` (a failure or a budget stop) or `cancelled` (the run was
    /// stopped and the call dropped). Absent while it runs, for a child
    /// task, and on records written before OF04 (no migration). Absent on a
    /// consultation whose run has finished means the consultation process
    /// itself was killed (SIGKILL, a crash) before it could record any
    /// ending, its drop guard included: read it as interrupted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        test,
        ts(optional, type = "\"ok\" | \"timeout\" | \"error\" | \"cancelled\"")
    )]
    pub outcome: Option<String>,
    /// A consultation's wall time from its teammate's start to its end, in
    /// milliseconds (the permit wait excluded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub duration_ms: Option<u64>,
    /// Bytes of the teammate's text returned to the asking agent: its
    /// answer, or on a timeout what it had written so far (0 when nothing
    /// came back).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional, type = "number"))]
    pub returned_bytes: Option<u64>,
    /// The consultation was cut off: a timeout, whose returned text (if any)
    /// is partial, or a cancel, which drops the call with no reply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(test, ts(optional))]
    pub incomplete: Option<bool>,
}

/// A run's Jev calls, reconciled (F11, O3): what was sent, how long it
/// took, each outcome, and the cost, which TypeSafe never reports.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "ManifestJevCalls"))]
#[serde(rename_all = "camelCase")]
pub struct JevCalls {
    /// Requests sent (a use with a latency).
    pub sent: u32,
    /// Uses that sent nothing: `off`, `cached`, `circuit-open`, a deadline
    /// that had passed.
    pub not_sent: u32,
    /// The sent requests' wall time, summed.
    pub latency_ms: u64,
    /// Every use's outcome, counted.
    pub outcomes: std::collections::BTreeMap<String, u32>,
    /// Always `null`: unknown, never 0.
    pub cost_usd: Option<f64>,
}

impl JevCalls {
    /// The calls in `manifest.jev` and the `after_run` checks' requests.
    pub fn of(manifest: &RunManifest, after_run: &[crate::jev_decide::Report]) -> Self {
        let mut calls = JevCalls::default();
        let uses = manifest
            .jev
            .iter()
            .chain(after_run.iter().filter_map(|r| r.request.as_ref()));
        for u in uses {
            match u.latency_ms {
                Some(ms) => {
                    calls.sent += 1;
                    calls.latency_ms += ms;
                }
                None => calls.not_sent += 1,
            }
            *calls.outcomes.entry(u.outcome.clone()).or_default() += 1;
        }
        calls
    }
}

impl RunManifest {
    pub fn new() -> Self {
        RunManifest {
            schema_version: SCHEMA_VERSION,
            ..Default::default()
        }
    }

    /// Add layer `name` for `text`; an empty text adds nothing.
    pub fn layer(&mut self, name: &str, text: &str) {
        if let Some(layer) = layer(name, text) {
            self.layers.push(layer);
        }
    }

    pub fn fallback(&mut self, stage: &str, detail: impl Into<String>) {
        self.fallbacks.push(Fallback {
            stage: stage.into(),
            detail: cap(&detail.into(), NOTICE_CHARS),
        });
    }

    /// Record the connector selection: its decisions, its Jev use and, when
    /// Jev failed or scored only some, a fallback.
    pub fn set_connectors(&mut self, mut selection: ConnectorSelection) {
        if let Some(jev) = selection.jev.take() {
            self.timer
                .0
                .record_some(crate::prep_timing::Step::ConnectorsJev, jev.mark.0);
            self.jev.push(jev);
        }
        if selection.engine == "jev-fallback" {
            let why = selection
                .decisions
                .iter()
                .find_map(|d| d.reason.clone())
                .unwrap_or_else(|| "Jev failed.".into());
            self.fallback("connectors", format!("local relevance: {why}"));
        } else if let Some(note) = selection.note.clone() {
            self.fallback("connectors", note);
        }
        self.connectors = Some(selection);
    }

    /// Add lifecycle decisions from one route (`note_lifecycle`), and what
    /// made the request historical. Nothing is recorded for a route that
    /// decided nothing about a request that isn't historical.
    pub fn add_lifecycle(&mut self, cue: Option<&str>, decisions: Vec<LifecycleDecision>) {
        if cue.is_none() && decisions.is_empty() {
            return;
        }
        let record = self.lifecycle.get_or_insert_with(Default::default);
        if let Some(cue) = cue {
            record.history = true;
            record.cue.get_or_insert_with(|| cue.to_string());
        }
        for mut d in decisions {
            if record.decisions.len() >= MAX_LIFECYCLE {
                record.unrecorded += 1;
                continue;
            }
            d.reason = cap(&d.reason, NOTICE_CHARS);
            record.decisions.push(d);
        }
    }

    /// Record retrieval: its candidates and notices, its Jev use and, when
    /// Jev failed, a fallback.
    pub fn set_retrieval(&mut self, retrieval: RetrievalRecord, jev: Option<JevUse>) {
        if retrieval.engine == "jev-fallback" {
            let why = retrieval
                .candidates
                .iter()
                .find_map(|c| c.reason.clone())
                .unwrap_or_else(|| "Jev failed.".into());
            self.fallback("passages", format!("local order: {why}"));
        }
        let timer = &mut self.timer.0;
        timer.record_some(
            crate::prep_timing::Step::RetrievalSemantic,
            retrieval.stats.as_ref().and_then(|s| s.semantic_mark.0),
        );
        timer.record_some(
            crate::prep_timing::Step::RetrievalJev,
            jev.as_ref().and_then(|j| j.mark.0),
        );
        self.jev.extend(jev);
        self.retrieval = Some(retrieval);
    }

    /// Mark the kept retrieval candidates the run's final reply names (S26,
    /// [`cites`]). Nothing without a retrieval or a reply; the reply itself
    /// is never kept.
    pub(crate) fn mark_cited(&mut self, reply: &str) {
        let Some(retrieval) = self.retrieval.as_mut() else {
            return;
        };
        if reply.trim().is_empty() {
            return;
        }
        for candidate in retrieval.candidates.iter_mut().filter(|c| c.kept) {
            candidate.cited = cites(reply, &candidate.path);
        }
        retrieval.cited_checked = true;
    }

    /// Record the preparation deadline (F8, O2), after retrieval and the
    /// connectors are set: which optional parts it cut (read from the Jev
    /// outcomes and the semantic state already recorded, plus `auth_cut`, a
    /// Microsoft 365 refresh it stopped waiting for), with a fallback for
    /// each cut the Jev fallbacks don't already state.
    /// A Jev duplicate check the run itself ran (F11, O3): its report, and
    /// its request (when one was sent) among the run's Jev uses.
    pub(crate) fn add_dedup(&mut self, report: crate::jev_decide::Report) {
        if let Some(request) = &report.request {
            self.jev.push(request.clone());
        }
        self.dedup.push(report);
    }

    /// What Claude Code loads by itself for a run in `cwd` (S13): its
    /// CLAUDE.md files, auto memory and user skills. Nothing for another
    /// agent, which loads none of them.
    ///
    /// CX4b: a local turn's auto memory is the personal clone's
    /// `memory/<project>` while a personal root is set, so the record names
    /// that folder (and `memoryWrites` counts writes into it). A `remote`
    /// run keeps the default folder: its command never carries the setting.
    pub(crate) fn set_native_memory(
        &mut self,
        kind: crate::agent::AgentKind,
        cwd: &Path,
        remote: bool,
    ) {
        if kind == crate::agent::AgentKind::Claude {
            self.native_memory = native_memory_for_run(cwd);
            let personal = if remote {
                None
            } else {
                crate::context_layers::auto_memory_dir(cwd)
            };
            if let (Some(record), Some(dir)) = (self.native_memory.as_mut(), personal) {
                record.auto_memory = auto_memory(&dir);
            }
        }
    }

    /// The folder the run's agent works in (S26): notes the agent opens
    /// under it are its own workspace's, not `agentReads.notesOpened`.
    pub(crate) fn set_run_cwd(&mut self, cwd: &Path) {
        self.run_cwd = Some(cwd.to_path_buf());
    }

    /// Codex's inventory step of the command build (F8, O3): its mark on
    /// the timeline and how it was served. Nothing for another agent.
    pub(crate) fn set_codex_inventory(
        &mut self,
        inventory: Option<crate::codex_inventory::Inventory>,
    ) {
        if let Some(inventory) = inventory {
            self.timer
                .0
                .record(crate::prep_timing::Step::CodexInventory, inventory.mark);
            self.preparation.codex_inventory = Some(inventory.served.as_str().into());
        }
    }

    pub(crate) fn set_deadline(
        &mut self,
        deadline: &crate::prep_deadline::PrepDeadline,
        auth_cut: bool,
    ) {
        let mut cut = Vec::new();
        for (kind, step) in [
            ("passages", "retrievalJev"),
            ("connectors", "connectorsJev"),
        ] {
            if self
                .jev
                .iter()
                .any(|j| j.kind == kind && j.outcome == "deadline")
            {
                cut.push(step.to_string());
            }
        }
        if self
            .retrieval
            .as_ref()
            .and_then(|r| r.stats.as_ref())
            .is_some_and(|s| s.semantic == "deadline")
        {
            cut.push("retrievalSemantic".into());
            self.fallback(
                "semantic",
                "lexical only: the semantic half missed the preparation deadline",
            );
        }
        if auth_cut {
            cut.push("microsoftAuth".into());
            self.fallback(
                "microsoftAuth",
                "the Microsoft 365 refresh missed the preparation deadline; it finishes on its own, and the connector refreshes in memory if it must",
            );
        }
        self.preparation.deadline = Some(PrepDeadlineRecord {
            budget_ms: deadline.budget_ms(),
            cut,
        });
    }
}

/// SHA-256 of `bytes`, lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Layer `name` for `text`, or `None` when it is empty.
pub fn layer(name: &str, text: &str) -> Option<Layer> {
    (!text.is_empty()).then(|| Layer {
        name: name.into(),
        bytes: text.len() as u64,
        sha256: sha256_hex(text.as_bytes()),
    })
}

/// A notice, capped.
pub(crate) fn notice(text: &str) -> String {
    cap(text, NOTICE_CHARS)
}

/// At most [`MAX_NOTICES`] notices, each capped.
pub(crate) fn notices<'a>(all: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    all.into_iter()
        .take(MAX_NOTICES)
        .map(|n| notice(n))
        .collect()
}

fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// The composer's blocks, by layer name. The same walk as
/// `sessions::strip_injected_context` (leading blocks in any order, the
/// first close of each unless an operator or page body embeds its own close
/// tag, then trailing `<attached-images>` and `<attached-files>`); lane C's
/// `sessions::split_injected_context` can replace this once both lanes are
/// merged.
const COMPOSER_TAGS: [(&str, &str); 13] = [
    (crate::delegation::TAG, "taskPacket"),
    // A lead's arrival note rides the child notes' slot (quick chats, 11-14).
    (crate::chat_chain::ARRIVED_TAG, "childFinished"),
    (crate::descendants::NOTE_TAG, "childFinished"),
    (crate::context_compiler::SHARED_TAG, "workspaceShared"),
    ("conversation-context", "conversationContext"),
    ("task-state", "taskState"),
    ("operator-context", "operatorProfile"),
    ("operator-update", "operatorUpdate"),
    ("hollow-visual-guide", "visualGuide"),
    ("page-context", "pageContext"),
    ("shared-page", "sharedPage"),
    ("hollow-chat-mode", "chatMode"),
    ("hollow-chat-intent", "chatIntent"),
];

/// The layers of a composed chat message (`body`, what the turn wraps):
/// each injected block, the user's own words (`request`) and the attached
/// images block, in order. Sizes include the tags.
pub fn composer_layers(body: &str) -> Vec<Layer> {
    let opens_block = |rest: &str| {
        let rest = rest.trim_start();
        COMPOSER_TAGS
            .iter()
            .any(|(tag, _)| rest.starts_with(&format!("<{tag}>")))
    };
    let mut out = Vec::new();
    let mut s = body.trim_start();
    'blocks: loop {
        for (tag, name) in COMPOSER_TAGS {
            let open = format!("<{tag}>");
            let close = format!("</{tag}>");
            if !s.starts_with(&open) {
                continue;
            }
            let ends: Vec<usize> = s
                .match_indices(&close)
                .map(|(i, _)| i + close.len())
                .collect();
            let embeds_text =
                matches!(tag, "operator-context" | "operator-update" | "page-context");
            let end = ends
                .iter()
                .copied()
                .find(|&e| embeds_text && opens_block(&s[e..]))
                .or_else(|| ends.first().copied());
            if let Some(end) = end {
                out.extend(layer(name, &s[..end]));
                s = s[end..].trim_start();
                continue 'blocks;
            }
        }
        break;
    }
    let blocks = crate::sessions::trailing_attachment_blocks(s);
    let request = blocks.first().map_or(s, |&(start, _)| &s[..start]);
    out.extend(layer("request", request.trim()));
    // The attachment blocks, each its own layer in the order they come
    // (C5, 2026-11-04: `attachedFiles` beside `attachedImages`).
    for (k, &(start, kind)) in blocks.iter().enumerate() {
        let end = blocks.get(k + 1).map_or(s.len(), |next| next.0);
        let name = ["attachedImages", "attachedFiles"][kind];
        out.extend(layer(name, &s[start..end]));
    }
    out
}

// ── The sidecar ───────────────────────────────────────────────────────────

/// A run id that can name a sidecar: 1–128 ASCII letters, digits, `-`, `_`
/// or `.`, not starting with a dot.
fn valid_run_id(run_id: &str) -> bool {
    !run_id.is_empty()
        && run_id.len() <= 128
        && !run_id.starts_with('.')
        && run_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn dir(vault: &Path) -> PathBuf {
    vault.join(DIR_NAME)
}

fn sidecar_path(vault: &Path, run_id: &str) -> Result<PathBuf, String> {
    if !valid_run_id(run_id) {
        return Err(format!("not a run id: {run_id:?}"));
    }
    Ok(dir(vault).join(format!("{run_id}.json")))
}

/// The sidecar folder, made if missing. A symlink or a non-folder in its
/// place is refused, so a write never lands outside the vault.
fn real_dir(vault: &Path, create: bool) -> Result<Option<PathBuf>, String> {
    let path = dir(vault);
    match fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{DIR_NAME} is a symlink; context manifests are not written through it"
        )),
        Ok(meta) if !meta.is_dir() => Err(format!("{DIR_NAME} is not a folder")),
        Ok(_) => Ok(Some(path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            fs::create_dir(&path).map_err(|e| format!("create {DIR_NAME}: {e}"))?;
            Ok(Some(path))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("read {DIR_NAME}: {e}")),
    }
}

/// Encode `manifest` for run `run_id`: the bytes to store and the
/// receipt's reference to them.
pub(crate) fn encode(manifest: &RunManifest) -> Result<(Vec<u8>, ManifestRef), String> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| format!("encode the context manifest: {e}"))?;
    let reference = ManifestRef {
        schema_version: manifest.schema_version,
        bytes: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
    };
    Ok((bytes, reference))
}

/// Write run `run_id`'s sidecar (a new temp file, then a rename over the
/// final name, so neither a symlink nor a half-written file is followed).
/// Called by `evidence::persist` under the receipt store's lock.
pub(crate) fn store(vault: &Path, run_id: &str, bytes: &[u8]) -> Result<(), String> {
    let path = sidecar_path(vault, run_id)?;
    let folder = real_dir(vault, true)?.ok_or("no manifest folder")?;
    let temp = folder.join(format!(".{run_id}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temp);
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temp)
            .map_err(|e| format!("write the context manifest: {e}"))?
    };
    let written = file.write_all(bytes).and_then(|_| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|_| fs::rename(&temp, &path)) {
        let _ = fs::remove_file(&temp);
        return Err(format!("write the context manifest: {e}"));
    }
    Ok(())
}

/// Remove every sidecar whose run is not in `keep` (receipts the store no
/// longer holds, and strays), and leftover temp files. Only regular files
/// named like a sidecar are removed; a symlinked folder is never entered.
pub(crate) fn prune(vault: &Path, keep: &HashSet<&str>) {
    let Ok(Some(folder)) = real_dir(vault, false) else {
        return;
    };
    let Ok(entries) = fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if !kind.is_file() {
            continue;
        }
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let stale = match name.strip_suffix(".json") {
            Some(run) if !run.starts_with('.') => !keep.contains(run),
            _ => name.starts_with('.') && name.ends_with(".tmp"),
        };
        if stale {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Read run `run_id`'s manifest and check it against the receipt's
/// `reference`: the size, the hash, the schema and the run id must all
/// match, or it is refused (an agent can write the vault; the receipt it
/// would also have to change is watched).
pub(crate) fn load(
    vault: &Path,
    run_id: &str,
    reference: &ManifestRef,
) -> Result<RunManifest, String> {
    let path = sidecar_path(vault, run_id)?;
    if real_dir(vault, false)?.is_none() {
        return Err("this run's context manifest is missing".into());
    }
    let mut file = {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "this run's context manifest is missing".into(),
                _ => format!("read the context manifest: {e}"),
            })?
    };
    let meta = file
        .metadata()
        .map_err(|e| format!("read the context manifest: {e}"))?;
    if !meta.is_file() || meta.len() > MAX_FILE_BYTES {
        return Err("this run's context manifest is not a manifest".into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("read the context manifest: {e}"))?;
    if bytes.len() as u64 != reference.bytes || sha256_hex(&bytes) != reference.sha256 {
        return Err("this run's context list does not match its run record".into());
    }
    let manifest: RunManifest = serde_json::from_slice(&bytes)
        .map_err(|e| format!("this run's context list is unreadable: {e}"))?;
    if manifest.schema_version != SCHEMA_VERSION || manifest.run_id != run_id {
        return Err("this run's context manifest does not belong to it".into());
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_are_sizes_and_hashes_and_stable() {
        let a = layer("recall", "approved note").unwrap();
        let b = layer("recall", "approved note").unwrap();
        assert_eq!(a, b, "identical inputs, identical layer");
        assert_eq!(a.bytes, 13);
        assert_eq!(a.sha256.len(), 64);
        assert_ne!(a.sha256, layer("recall", "approved notes").unwrap().sha256);
        assert!(layer("recall", "").is_none(), "an empty layer is left out");
    }

    #[test]
    fn composer_layers_split_the_blocks_and_keep_no_text() {
        let body = "<operator-context>\nprofile, with </operator-context> inside\n</operator-context>\n\n<page-context>\npage\n</page-context>\n<hollow-chat-intent>\nintent\n</hollow-chat-intent>\n\nDraft the zircon kestrel memo\n<attached-images count=\"1\">\n/a.png\n</attached-images>";
        let layers = composer_layers(body);
        let names: Vec<&str> = layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "operatorProfile",
                "pageContext",
                "chatIntent",
                "request",
                "attachedImages"
            ]
        );
        let request = &layers[3];
        assert_eq!(request.bytes, "Draft the zircon kestrel memo".len() as u64);
        assert_eq!(
            request.sha256,
            sha256_hex(b"Draft the zircon kestrel memo"),
            "the request layer is the user's own words, as retrieval reads them"
        );
        assert_eq!(
            crate::sessions::strip_injected_context(body),
            "Draft the zircon kestrel memo",
            "the same walk as strip_injected_context"
        );
        let json = serde_json::to_string(&layers).unwrap();
        assert!(!json.contains("zircon"), "{json}");
        // A bare message is one request layer.
        let bare = composer_layers("just this");
        assert_eq!(bare.len(), 1);
        // C5: attached files are their own layer, after any images.
        let attached = "look\n\n<attached-images count=\"1\">\n1. /a.png\n</attached-images>\n\n<attached-files count=\"1\">\n1. a.pdf at /a.pdf\n</attached-files>";
        let both = composer_layers(attached);
        let names: Vec<&str> = both.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["request", "attachedImages", "attachedFiles"]);
        assert_eq!(bare[0].name, "request");
        // Repo ideas 2026-10-27 #3: a shared page is its own layer, by size.
        let shared = composer_layers("<shared-page>\n{\"text\":\"vendor totals\"}\n</shared-page>\n\n<hollow-chat-intent>\nintent\n</hollow-chat-intent>\n\nSum it");
        let names: Vec<&str> = shared.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["sharedPage", "chatIntent", "request"]);
        assert!(!serde_json::to_string(&shared).unwrap().contains("vendor"));
    }

    #[test]
    fn unknown_cost_is_null_never_zero() {
        let used = JevUse::of::<()>("connectors", Some(&Ok(())), 3, 12);
        let json = serde_json::to_value(&used).unwrap();
        assert_eq!(json["use"], "connectors");
        assert_eq!(json["outcome"], "ok");
        assert!(json["costUsd"].is_null(), "{json}");
        assert!(json.as_object().unwrap().contains_key("costUsd"));
        let off = JevUse::of::<()>("passages", None, 3, 12);
        assert_eq!(
            (off.outcome.as_str(), off.items, off.latency_ms),
            ("off", 0, None)
        );
        let timeout = JevUse::of::<()>("passages", Some(&Err("Jev timed out.".into())), 2, 4000);
        assert_eq!(timeout.outcome, "timeout");
        let malformed = JevUse::of::<()>(
            "passages",
            Some(&Err("Jev returned an invalid response.".into())),
            2,
            9,
        );
        assert_eq!(malformed.outcome, "malformed");
        assert!(serde_json::to_value(&malformed).unwrap()["costUsd"].is_null());
    }

    /// OB23: a Claude run with tools records its Bash output cap; any other
    /// run writes no key, and a manifest from before it reads as none.
    #[test]
    fn capabilities_record_the_bash_output_cap_only_when_set() {
        let capped = Capabilities {
            bash_output_max_chars: Some(crate::agent::CLAUDE_BASH_MAX_OUTPUT_CHARS),
            ..Default::default()
        };
        let json = serde_json::to_value(&capped).unwrap();
        assert_eq!(json["bashOutputMaxChars"], 16_000, "{json}");
        let none = serde_json::to_value(Capabilities::default()).unwrap();
        assert!(none.get("bashOutputMaxChars").is_none(), "{none}");
        let older = r#"{"ownedServers":["hollow_vault"],"connectors":[]}"#;
        let older: Capabilities = serde_json::from_str(older).unwrap();
        assert_eq!(older.bash_output_max_chars, None);
        assert_eq!(older.owned_servers, ["hollow_vault"]);
    }

    /// S26: a reply cites a note by its path or, at 4 characters or more,
    /// its name, case-insensitively and as a whole word; the new fields
    /// write no key until set and read as unset from older manifests.
    #[test]
    fn citations_match_whole_names_and_the_new_fields_stay_optional() {
        assert!(cites("See Weekly-Sync.md.", "notes/weekly-sync.md"));
        assert!(cites("see `notes/ops`", "notes/ops.md"));
        assert!(!cites("see ops", "notes/ops.md"), "a 3-character name");
        assert!(!cites("see ops", "ops.md"), "a bare short name is a name");
        assert!(!cites("the weekly-syncs", "notes/weekly-sync.md"));
        assert!(!cites("a_sync_b", "sync.md"));
        assert!(cites("(sync)", "sync.md"));
        // A generic or date-shaped name cites only with its `.md` or path.
        assert!(!cites("See the README first.", "projects/x/README.md"));
        assert!(cites("See the README.md first.", "projects/x/README.md"));
        assert!(cites("see projects/x/readme", "projects/x/README.md"));
        assert!(!cites("Plan: ship the summary", "notes/summary.md"));
        assert!(!cites("On 2026-10-17 we shipped.", "daily/2026-10-17.md"));
        assert!(cites("See 2026-10-17.md.", "daily/2026-10-17.md"));
        assert!(!cites("since 2026-10,", "journal/2026-10.md"));
        // Case folds beyond ASCII.
        assert!(cites("Per ÜBERSICHT, yes", "notes/übersicht.md"));
        let older = r#"{"path":"a.md","line":1,"source":"note","coverage":"excerpt","rank":1,"localScore":1,"kept":true,"basis":"jev","bytes":1,"sha256":""}"#;
        let older: RetrievalCandidate = serde_json::from_str(older).unwrap();
        assert!(!older.cited);
        assert!(serde_json::to_value(&older).unwrap().get("cited").is_none());
        let reads: AgentReads =
            serde_json::from_str(r#"{"readSource":1,"readSourceFailed":0,"searchVault":0}"#)
                .unwrap();
        assert!(reads.notes_opened.is_empty());
        assert_eq!(reads.read_source_queried, 0, "older manifests read as none");
        assert!(serde_json::to_value(&reads)
            .unwrap()
            .get("notesOpened")
            .is_none());
        let mut m = RunManifest::new();
        m.mark_cited("anything");
        assert!(m.retrieval.is_none(), "no retrieval, nothing to mark");
        m.retrieval = Some(RetrievalRecord::default());
        m.mark_cited("  ");
        assert!(!m.retrieval.as_ref().unwrap().cited_checked, "no reply");
        m.mark_cited("a reply");
        assert!(m.retrieval.as_ref().unwrap().cited_checked);
    }

    /// OB22: the compaction window and the count are written only when set;
    /// a manifest from before them reads as none.
    #[test]
    fn capabilities_record_the_compaction_window_and_count_only_when_set() {
        let compacting = Capabilities {
            compact_window: Some(400_000),
            compactions: Some(2),
            ..Default::default()
        };
        let json = serde_json::to_value(&compacting).unwrap();
        assert_eq!(json["compactWindow"], 400_000, "{json}");
        assert_eq!(json["compactions"], 2, "{json}");
        let none = serde_json::to_value(Capabilities::default()).unwrap();
        assert!(none.get("compactWindow").is_none(), "{none}");
        assert!(none.get("compactions").is_none(), "{none}");
        let older = r#"{"ownedServers":[],"connectors":[]}"#;
        let older: Capabilities = serde_json::from_str(older).unwrap();
        assert_eq!((older.compact_window, older.compactions), (None, None));
    }

    /// Agent effectiveness F8 (O2): how a ranking call was served, and the
    /// preparation deadline's record: the cut steps read from the outcomes
    /// already recorded, a fallback for each cut the Jev fallbacks don't
    /// state, the budget, and no text.
    #[test]
    fn the_deadline_and_the_guard_are_recorded() {
        use crate::jev::Served;
        let sent = || {
            JevUse::of::<()>("passages", Some(&Ok(())), 24, 12).sent_at(std::time::Instant::now())
        };
        let mut cached = sent();
        cached.served(Served::Cached);
        assert_eq!(
            (cached.outcome.as_str(), cached.items, cached.latency_ms),
            ("cached", 24, None)
        );
        assert!(cached.mark.0.is_none());
        let paused_error =
            "Jev is paused after 3 failed requests in a row; local ranking is used for 40 s more.";
        let mut paused = JevUse::of::<()>("connectors", Some(&Err(paused_error.into())), 24, 0);
        paused.served(Served::Paused);
        assert_eq!(
            (paused.outcome.as_str(), paused.items, paused.latency_ms),
            ("circuit-open", 0, None)
        );
        let mut late = JevUse::of::<()>(
            "connectors",
            Some(&Err(crate::prep_deadline::JEV_MISSED.into())),
            24,
            0,
        );
        late.served(Served::Late);
        assert_eq!((late.outcome.as_str(), late.items), ("deadline", 0));
        let mut kept = sent();
        kept.served(Served::Sent);
        assert_eq!((kept.outcome.as_str(), kept.latency_ms), ("ok", Some(12)));

        let deadline =
            crate::prep_deadline::PrepDeadline::with_budget(std::time::Duration::from_millis(3000));
        let mut m = RunManifest::new();
        m.set_retrieval(
            RetrievalRecord {
                stats: Some(RetrievalStats {
                    semantic: "deadline".into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            Some(JevUse::of::<()>(
                "passages",
                Some(&Err(crate::prep_deadline::JEV_MISSED.into())),
                24,
                3000,
            )),
        );
        m.jev.push(late);
        m.set_deadline(&deadline, true);
        let record = m.preparation.deadline.clone().unwrap();
        assert_eq!(record.budget_ms, 3000);
        assert_eq!(
            record.cut,
            [
                "retrievalJev",
                "connectorsJev",
                "retrievalSemantic",
                "microsoftAuth"
            ]
        );
        let stages: Vec<&str> = m.fallbacks.iter().map(|f| f.stage.as_str()).collect();
        assert_eq!(stages, ["semantic", "microsoftAuth"]);
        let json = serde_json::to_value(&m).unwrap();
        assert_eq!(json["preparation"]["deadline"]["budgetMs"], 3000);
        assert_eq!(json["preparation"]["deadline"]["cut"][3], "microsoftAuth");
        // Nothing cut: an empty list, no fallback.
        let mut quiet = RunManifest::new();
        quiet.set_deadline(&deadline, false);
        assert_eq!(
            quiet.preparation.deadline.unwrap().cut,
            Vec::<String>::new()
        );
        assert!(quiet.fallbacks.is_empty());
        // A manifest from before O2 reads back without it.
        let old: RunManifest = serde_json::from_value(
            serde_json::json!({"schemaVersion": 1, "runId": "r", "layers": []}),
        )
        .unwrap();
        assert!(old.preparation.deadline.is_none());
    }

    #[test]
    fn a_sidecar_round_trips_and_a_tampered_one_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        let mut m = RunManifest::new();
        m.run_id = "run-1".into();
        m.layer("request", "hello");
        let (bytes, reference) = encode(&m).unwrap();
        store(v, "run-1", &bytes).unwrap();
        assert_eq!(load(v, "run-1", &reference).unwrap(), m);
        fs::write(v.join(DIR_NAME).join("run-1.json"), b"{}").unwrap();
        let err = load(v, "run-1", &reference).unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        // A sidecar under another run's name is refused too.
        store(v, "run-2", &bytes).unwrap();
        let err = load(v, "run-2", &reference).unwrap_err();
        assert!(err.contains("does not belong"), "{err}");
        assert!(store(v, "../escape", &bytes).is_err());
        assert!(load(v, "run-9", &reference)
            .unwrap_err()
            .contains("missing"));
    }

    #[test]
    fn a_symlinked_folder_is_never_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join(DIR_NAME)).unwrap();
        let err = store(dir.path(), "run-1", b"{}").unwrap_err();
        assert!(err.contains("symlink"), "{err}");
        prune(dir.path(), &HashSet::new());
        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn prune_keeps_only_the_named_runs() {
        let dir = tempfile::tempdir().unwrap();
        let v = dir.path();
        for run in ["run-a", "run-b"] {
            store(v, run, b"{}").unwrap();
        }
        fs::write(v.join(DIR_NAME).join(".run-c.1.tmp"), b"x").unwrap();
        fs::write(v.join(DIR_NAME).join("notes.txt"), b"x").unwrap();
        prune(v, &HashSet::from(["run-a"]));
        let mut left: Vec<String> = fs::read_dir(v.join(DIR_NAME))
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["notes.txt", "run-a.json"]);
    }

    /// S13: a temp HOME with the user file, two ancestor CLAUDE.md files,
    /// an auto-memory folder with its index and two notes, and three skills
    /// (one symlinked): every one recorded, by path and size.
    #[test]
    fn native_memory_records_what_claude_loads_by_itself() {
        let tmp = tempfile::tempdir().unwrap();
        // Real paths, as Claude Code (getcwd) sees them.
        let root = fs::canonicalize(tmp.path()).unwrap();
        let home = root.join("home");
        let cwd = home.join("proj").join("sub");
        fs::create_dir_all(&cwd).unwrap();
        let config = home.join(".claude");
        fs::create_dir_all(&config).unwrap();
        fn path(p: &Path) -> String {
            p.to_string_lossy().into_owned()
        }
        let user = config.join("CLAUDE.md");
        fs::write(&user, "user rules\n").unwrap();
        fs::write(home.join("CLAUDE.md"), "home rules, longer\n").unwrap();
        fs::write(home.join("proj").join("CLAUDE.md"), "project\n").unwrap();
        let memory = config
            .join("projects")
            .join(claude_project_slug(&cwd))
            .join("memory");
        fs::create_dir_all(&memory).unwrap();
        let index = "- [a](a.md)\n- [b](b.md)\n";
        fs::write(memory.join("MEMORY.md"), index).unwrap();
        fs::write(memory.join("a.md"), "note a").unwrap();
        fs::write(memory.join("b.md"), "note b").unwrap();
        fs::write(memory.join("scratch.txt"), "not a note").unwrap();
        let skills = config.join("skills");
        for (dir, name, description) in [
            ("alpha", "alpha", "Does a."),
            ("beta", "beta-skill", "Does b, at length."),
        ] {
            fs::create_dir_all(skills.join(dir)).unwrap();
            fs::write(
                skills.join(dir).join("SKILL.md"),
                format!("---\nname: {name}\ndescription: {description}\n---\n\nBody.\n"),
            )
            .unwrap();
        }
        // A symlinked skill folder, as the Homebrew-managed ones are.
        let elsewhere = root.join("elsewhere").join("gamma");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(
            elsewhere.join("SKILL.md"),
            "---\nname: gamma\ndescription: \"Quoted c.\"\n---\n",
        )
        .unwrap();
        std::os::unix::fs::symlink(&elsewhere, skills.join("gamma")).unwrap();
        // A folder with no SKILL.md is no skill.
        fs::create_dir_all(skills.join("notes")).unwrap();

        let record = native_memory(&home, &cwd);
        assert_eq!(record.config_dir, path(&config));
        // The user file first, once (it is also home's `.claude/CLAUDE.md`),
        // then the ancestors from the top down.
        let ours: Vec<(&str, String, u64)> = record
            .instructions
            .iter()
            .filter(|f| Path::new(&f.path).starts_with(&root))
            .map(|f| (f.scope.as_str(), f.path.clone(), f.bytes))
            .collect();
        assert_eq!(
            ours,
            [
                ("user", path(&user), 11),
                ("project", path(&home.join("CLAUDE.md")), 19),
                ("project", path(&home.join("proj").join("CLAUDE.md")), 8),
            ]
        );
        assert_eq!(record.instructions[0].path, path(&user));
        assert_eq!(
            record.instruction_bytes,
            record.instructions.iter().map(|f| f.bytes).sum::<u64>()
        );
        assert_eq!(
            record.auto_memory,
            AutoMemoryRecord {
                dir: path(&memory),
                exists: true,
                memory_md_bytes: index.len() as u64,
                memory_md_lines: 2,
                notes: 2,
            }
        );
        let listed: u64 = [
            "alpha",
            "Does a.",
            "beta-skill",
            "Does b, at length.",
            "gamma",
            "Quoted c.",
        ]
        .iter()
        .map(|s| s.len() as u64)
        .sum();
        assert_eq!(
            record.skill_listing,
            SkillListingRecord {
                count: 3,
                bytes: listed,
            }
        );
        // Sizes and paths only: no file's text reaches the record.
        let json = serde_json::to_string(&record).unwrap();
        for text in ["user rules", "home rules", "note a", "Does a.", "Body."] {
            assert!(!json.contains(text), "{text} in {json}");
        }

        // A cwd with no memory folder: the folder is named, and absent.
        let bare = root.join("bare");
        fs::create_dir_all(&bare).unwrap();
        let record = native_memory(&home, &bare);
        assert!(!record.auto_memory.exists);
        assert_eq!(record.auto_memory.notes, 0);
        assert!(record.auto_memory.dir.ends_with("-bare/memory"));
    }

    /// CX4b: with a personal root a local Claude run's record names the
    /// clone's `memory/<slug>` folder and counts its notes; a runner run
    /// keeps the default folder.
    #[test]
    fn set_native_memory_follows_the_personal_memory_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        let (clone, work) = (root.join("clone"), root.join("work"));
        fs::create_dir_all(&clone).unwrap();
        fs::create_dir_all(work.join(".git")).unwrap();
        let claude = crate::agent::AgentKind::Claude;
        let dir = clone.join("memory").join(claude_project_slug(&work));
        let _root = crate::context_layers::test_root::set(clone.clone(), vec![]);
        let mut local = RunManifest::new();
        local.set_native_memory(claude, &work, false);
        let record = local.native_memory.unwrap().auto_memory;
        assert_eq!(record.dir, dir.to_string_lossy());
        assert!(record.exists, "the folder is created with the command");
        fs::write(dir.join("note.md"), "x").unwrap();
        let mut again = RunManifest::new();
        again.set_native_memory(claude, &work, false);
        assert_eq!(again.native_memory.unwrap().auto_memory.notes, 1);
        let mut remote = RunManifest::new();
        remote.set_native_memory(claude, &work, true);
        assert!(!remote
            .native_memory
            .unwrap()
            .auto_memory
            .dir
            .starts_with(clone.to_string_lossy().as_ref()));
    }

    /// S13: the slug is Claude Code's folder name; only Claude runs get
    /// the record, and a manifest from before it reads as absent.
    #[test]
    fn native_memory_is_claude_only_and_slugs_like_claude_code() {
        assert_eq!(
            claude_project_slug(Path::new("/Users/dev/Documents/projects/hollow-vault")),
            "-Users-dev-Documents-projects-hollow-vault"
        );
        assert_eq!(
            claude_project_slug(Path::new(
                "/Users/dev/Documents/Documents - Alex\u{2019}s MacBook Pro/projects"
            )),
            "-Users-dev-Documents-Documents---Alex-s-MacBook-Pro-projects"
        );
        assert_eq!(
            claude_project_slug(Path::new("/r/.claude/worktrees/a_b")),
            "-r--claude-worktrees-a-b"
        );
        let mut manifest = RunManifest::new();
        manifest.set_native_memory(crate::agent::AgentKind::Codex, Path::new("/"), false);
        assert_eq!(manifest.native_memory, None, "Codex loads none of it");
        let older: RunManifest =
            serde_json::from_str(r#"{"schemaVersion":1,"runId":"r","layers":[]}"#).unwrap();
        assert_eq!(older.native_memory, None);
        assert!(!serde_json::to_string(&older)
            .unwrap()
            .contains("nativeMemory"));
    }

    /// S13 review: auto memory is keyed by the repository root, not the
    /// cwd. No `.git` keeps the cwd; a repository's subfolder gives the
    /// repository; a linked worktree (`.git` file → gitdir → `commondir`,
    /// with a matching back-pointer) gives the main working copy, with an
    /// absolute or a relative gitdir; a bare common dir (`repo.git`,
    /// `.bare`) is keyed itself. A submodule, a stale back-pointer and a
    /// gitdir outside `<common>/worktrees` key the worktree's own folder.
    #[test]
    fn claude_memory_root_is_the_repo_root_and_the_main_copy_for_worktrees() {
        let tmp = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(tmp.path()).unwrap();
        // A worktree folder whose `.git` file names `gitdir_line`, and an
        // admin gitdir holding `commondir` and the `gitdir` back-pointer.
        fn link(worktree: &Path, gitdir_line: &str, admin: &Path, common: &str, back: &str) {
            fs::create_dir_all(worktree).unwrap();
            fs::create_dir_all(admin).unwrap();
            fs::write(worktree.join(".git"), format!("gitdir: {gitdir_line}\n")).unwrap();
            fs::write(admin.join("commondir"), format!("{common}\n")).unwrap();
            fs::write(admin.join("gitdir"), format!("{back}\n")).unwrap();
        }

        // Plain folder, no git anywhere below the temp root.
        let plain = root.join("plain").join("deep");
        fs::create_dir_all(&plain).unwrap();
        assert_eq!(claude_memory_root(&plain), plain);

        // A repository and a subfolder of it.
        let main = root.join("repos").join("main");
        let sub = main.join("src").join("lib");
        fs::create_dir_all(&sub).unwrap();
        fs::create_dir_all(main.join(".git").join("worktrees")).unwrap();
        assert_eq!(claude_memory_root(&sub), main);
        assert_eq!(claude_memory_root(&main), main);

        // A linked worktree elsewhere, gitdir absolute.
        let admin = main.join(".git").join("worktrees").join("wt");
        fs::create_dir_all(&admin).unwrap();
        fs::write(admin.join("commondir"), "../..\n").unwrap();
        let wt = root.join("worktrees").join("wt");
        let wt_cwd = wt.join("src");
        fs::create_dir_all(&wt_cwd).unwrap();
        fs::write(wt.join(".git"), format!("gitdir: {}\n", admin.display())).unwrap();
        fs::write(
            admin.join("gitdir"),
            format!("{}\n", wt.join(".git").display()),
        )
        .unwrap();
        assert_eq!(claude_memory_root(&wt_cwd), main);
        assert_eq!(claude_memory_root(&wt), main);

        // Gitdir relative to the worktree's folder, back-pointer relative
        // to the gitdir, `commondir` without a newline.
        let admin2 = main.join(".git").join("worktrees").join("wt2");
        fs::create_dir_all(&admin2).unwrap();
        fs::write(admin2.join("commondir"), "../..").unwrap();
        fs::write(admin2.join("gitdir"), "../../../../../worktrees/wt2/.git").unwrap();
        let wt2 = root.join("worktrees").join("wt2");
        fs::create_dir_all(&wt2).unwrap();
        fs::write(
            wt2.join(".git"),
            "gitdir: ../../repos/main/.git/worktrees/wt2\n",
        )
        .unwrap();
        assert_eq!(claude_memory_root(&wt2), main);

        // A submodule: its `.git` file names an existing gitdir under the
        // superproject's `.git/modules` with no `commondir`, so it keeps
        // its own folder.
        let superproject = root.join("super");
        let module_gitdir = superproject.join(".git").join("modules").join("lib");
        fs::create_dir_all(&module_gitdir).unwrap();
        fs::write(module_gitdir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let module = superproject.join("lib");
        fs::create_dir_all(&module).unwrap();
        fs::write(module.join(".git"), "gitdir: ../.git/modules/lib\n").unwrap();
        assert_eq!(claude_memory_root(&module), module);

        // A bare clone (`git clone --bare x repo.git`) with a worktree: the
        // common dir isn't named `.git`, so it is the key itself.
        let bare = root.join("bare").join("repo.git");
        let bare_wt = root.join("bare").join("wt3");
        let bare_admin = bare.join("worktrees").join("wt3");
        link(
            &bare_wt,
            &bare_admin.display().to_string(),
            &bare_admin,
            "../..",
            &bare_wt.join(".git").display().to_string(),
        );
        fs::create_dir_all(bare_wt.join("src")).unwrap();
        assert_eq!(claude_memory_root(&bare_wt.join("src")), bare);

        // The `.bare` layout: `proj/.bare` plus worktrees beside it.
        let proj = root.join("proj");
        let dot_bare = proj.join(".bare");
        let proj_main = proj.join("main");
        link(
            &proj_main,
            "../.bare/worktrees/main",
            &dot_bare.join("worktrees").join("main"),
            "../..",
            &proj_main.join(".git").display().to_string(),
        );
        assert_eq!(claude_memory_root(&proj_main), dot_bare);

        // A moved worktree: the back-pointer still names the old place, so
        // it keys its own folder.
        let moved = root.join("worktrees").join("moved");
        let stale_admin = main.join(".git").join("worktrees").join("old");
        link(
            &moved,
            &stale_admin.display().to_string(),
            &stale_admin,
            "../..",
            &root
                .join("worktrees")
                .join("old")
                .join(".git")
                .display()
                .to_string(),
        );
        assert_eq!(claude_memory_root(&moved), moved);

        // A gitdir that isn't under `<common>/worktrees`, even with a
        // matching back-pointer, keys its own folder.
        let stray = root.join("worktrees").join("stray");
        let stray_admin = root.join("elsewhere").join("stray");
        link(
            &stray,
            &stray_admin.display().to_string(),
            &stray_admin,
            &main.join(".git").display().to_string(),
            &stray.join(".git").display().to_string(),
        );
        assert_eq!(claude_memory_root(&stray), stray);

        // The record's memory folder follows: a worktree run reads the main
        // copy's memory.
        let home = root.join("home");
        let record = native_memory(&home, &wt_cwd);
        let expected = home
            .join(".claude")
            .join("projects")
            .join(claude_project_slug(&main))
            .join("memory");
        assert_eq!(record.auto_memory.dir, expected.to_string_lossy());
    }
}
