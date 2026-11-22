//! Agent effectiveness F6, wave 3 · G2: what a chat turn carries, first and
//! resumed, per provider, through the real composition (`TurnComposer`),
//! the real workspace envelope and the real session store. Generates
//! `tests/fixtures/prompts/turn-budget.json`.

use super::*;
use crate::context_compiler::Session;
use crate::visual_prompt::CanvasFrequency;
use std::path::PathBuf;

pub(super) const AGENTS: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Antigravity,
    AgentKind::Grok,
];

/// What the measured paths are printed as, so the bytes don't depend on
/// where the temporary vault is.
const FIXED_VAULT: &str = "/Users/you/Hollow";
/// The environment brief names Downloads twice. Like the vault path, its
/// bytes must not depend on the account running these budget fixtures.
pub(super) fn normalize_downloads(text: &str) -> String {
    text.replace(&downloads_folder(), "/Users/you/Downloads")
}
/// The native session id the measured agent announces.
const NATIVE: &str = "native-1";

/// What a chat with `agent` typically has attached (as G1 measured):
/// Claude and Codex question cards, task, app and vault tools; the others
/// the fenced question block.
pub(super) fn caps_for(agent: AgentKind) -> AttachedCaps {
    if crate::agent::capabilities(agent).owned_mcp {
        AttachedCaps {
            questions: true,
            tasks: true,
            app: true,
            vault: true,
            agent: Some(agent),
            ..Default::default()
        }
    } else {
        AttachedCaps {
            questions_fenced: true,
            agent: Some(agent),
            ..Default::default()
        }
    }
}

/// `n` numbered lines about `topic`, deterministic.
pub(super) fn lines(topic: &str, n: usize) -> String {
    (1..=n)
        .map(|i| {
            format!(
                "- {topic} note {i:03}: {} in cohort {i} moved by {} points after the March release, per the weekly export.\n",
                topic.to_lowercase(),
                i % 7
            )
        })
        .collect()
}

/// A vault with a profile and one workspace (`release-review`): a brief,
/// a plan, a sources list and three attached sources. Returns the
/// workspace folder.
pub(super) fn workspace_vault(vault: &Path) -> PathBuf {
    let me = vault.join("me");
    std::fs::create_dir_all(&me).unwrap();
    std::fs::write(
        me.join("Identity.md"),
        "I run product research for a small studio.\n",
    )
    .unwrap();
    std::fs::write(
        me.join("Communication.md"),
        "Lead with the answer. Cite the file and lines.\n",
    )
    .unwrap();
    std::fs::write(
        me.join("Decisions.md"),
        "- Churn is measured on paid seats, not accounts.\n",
    )
    .unwrap();
    let cwd = vault.join("workspaces/release-review");
    std::fs::create_dir_all(cwd.join("sources")).unwrap();
    std::fs::write(
        cwd.join("WORKSPACE.md"),
        format!(
            "---\nhollow_workspace: 1\ntitle: Release review\nstatus: active\n---\n# Release review\n\nOutcome: a churn and retention readout for the release.\n\n{}",
            lines("Brief", 20)
        ),
    )
    .unwrap();
    std::fs::write(
        cwd.join("PLAN.md"),
        format!("# Plan\n\n{}", lines("Plan", 40)),
    )
    .unwrap();
    std::fs::write(
        cwd.join("SOURCES.md"),
        "# Sources\n\n- sources/churn.md\n- sources/retention.md\n- sources/pricing.md\n",
    )
    .unwrap();
    let fixed = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_790_000_000);
    for (name, topic) in [
        ("churn.md", "Churn"),
        ("retention.md", "Retention"),
        ("pricing.md", "Pricing"),
    ] {
        let path = cwd.join("sources").join(name);
        std::fs::write(&path, lines(topic, 160)).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(fixed)
            .unwrap();
    }
    cwd
}

/// One measured turn: bytes in Claude's system channel and in the prompt,
/// and the parts.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Bytes {
    /// `--append-system-prompt` (Claude only).
    pub system: usize,
    /// The prompt the agent's CLI gets.
    pub prompt: usize,
    /// Hollow's configured half as sent this turn (appended prompt and the
    /// workspace policy), in whichever channel.
    pub configured: usize,
    /// The workspace envelope as sent.
    pub workspace: usize,
}

impl Bytes {
    pub fn total(&self) -> usize {
        self.system + self.prompt
    }
}

/// A chat in `cwd` with `agent`: the configured half, and turn by turn
/// what is sent.
pub(super) struct Chat<'a> {
    pub agent: AgentKind,
    pub vault: &'a Path,
    pub cwd: PathBuf,
    pub key: &'a str,
    pub scratch: PathBuf,
    /// Record what each turn's native session holds, as `execute_turn`
    /// does once the agent ran. Off, every resumed turn finds no baseline
    /// and sends everything in full: what a turn carried before G2.
    pub baseline: bool,
}

impl Chat<'_> {
    fn normalize(&self, text: &str) -> String {
        let mut text = text.to_string();
        if let Ok(canonical) = self.vault.canonicalize() {
            text = text.replace(canonical.to_string_lossy().as_ref(), FIXED_VAULT);
        }
        normalize_downloads(&text.replace(self.vault.to_string_lossy().as_ref(), FIXED_VAULT))
    }

    /// Send `request` as a turn on `session`, as `execute_turn` composes
    /// it, and finish it with `reply`. Returns the bytes, the prompt the
    /// agent's CLI gets (paths normalized) and the compiler's record.
    pub fn turn(&self, request: &str, session: Session, reply: &str) -> Sent {
        self.turn_ending(request, session, reply, true, Some(NATIVE))
    }

    /// [`Self::turn`], ending as the agent did: `finished` (a success) or
    /// not, and the native id its stream `announced`, if any.
    pub fn turn_ending(
        &self,
        request: &str,
        session: Session,
        reply: &str,
        finished: bool,
        announced: Option<&str>,
    ) -> Sent {
        let ending = Ending {
            finished,
            announced,
            compacted: false,
        };
        self.turn_with(request, session, reply, ending, PerTurn::default())
    }

    /// [`Self::turn_ending`] with the per-turn layers `per` (S02).
    pub fn turn_with(
        &self,
        request: &str,
        session: Session,
        reply: &str,
        ending: Ending<'_>,
        per: PerTurn<'_>,
    ) -> Sent {
        let composed = format!("<hollow-chat-intent>\nanswer\n</hollow-chat-intent>\n\n{request}");
        let queued = Queued {
            entry: crate::context_compiler::Entry::Composer,
            request,
            composed: &composed,
            asker: None,
        };
        self.queued_with(queued, session, reply, ending, per)
    }

    /// A turn queued as `queued` says (OB15: a check-in, a packet child),
    /// otherwise as [`Self::turn_ending`].
    pub fn queued(
        &self,
        queued: Queued<'_>,
        session: Session,
        reply: &str,
        finished: bool,
        announced: Option<&str>,
    ) -> Sent {
        let ending = Ending {
            finished,
            announced,
            compacted: false,
        };
        self.queued_with(queued, session, reply, ending, PerTurn::default())
    }

    /// [`Self::queued`] with the per-turn layers `per` (S02).
    pub fn queued_with(
        &self,
        queued: Queued<'_>,
        session: Session,
        reply: &str,
        ending: Ending<'_>,
        per: PerTurn<'_>,
    ) -> Sent {
        let (finished, announced) = (ending.finished, ending.announced);
        let request = queued.request;
        let workspace = !self.cwd.as_os_str().is_empty() && self.cwd != self.vault;
        let caps = caps_for(self.agent);
        let perm = per.perm;
        let mut append_layers = chat_append_layers(
            workspace,
            self.vault,
            &self.cwd,
            caps,
            CanvasFrequency::Helps,
            perm,
        );
        // L10: an app-session child is a subagent: the lean profile, as
        // execute_turn gives it.
        if queued.entry == crate::context_compiler::Entry::AppSession {
            append_layers = lean_subagent(append_layers, caps);
        }
        let append = append_layers.joined();
        let policy = crate::agent::chat_policy(self.agent, perm);
        // As execute_turn: an app-session turn runs one level deeper
        // (`Entry::for_depth`), so a packet child gets the slim envelope (S01).
        let depth = u32::from(queued.entry == crate::context_compiler::Entry::AppSession);
        let child_packet = crate::delegation::child_envelope(depth, queued.composed);
        let source_request = crate::workspaces::SourceRequest {
            query: request,
            reader: if crate::mcp_attach::chat_has_vault_reader(self.agent) {
                crate::workspaces::SourceReader::ReadSource
            } else {
                crate::workspaces::SourceReader::OwnTools
            },
            child_packet: child_packet.as_deref(),
        };
        let envelope =
            crate::workspaces::context_for(self.vault, &self.cwd, &source_request).unwrap();
        let rebuild = |max: usize| {
            crate::workspaces::context_for_within(self.vault, &self.cwd, &source_request, max)
                .ok()
                .flatten()
        };
        let ord = crate::session_store::start_turn(
            self.vault,
            self.key,
            self.agent.as_str(),
            &self.cwd.to_string_lossy(),
            &format!("run-{}-{}", self.key, request.len()),
            request,
        )
        .unwrap();
        let guide = TurnGuide::new(
            CanvasFrequency::Helps,
            crate::visual_prompt::Asking::of(caps.questions, caps.questions_fenced),
            super::chat_queue::turn_visual_cue(request, &[], queued.asker),
            Some(&self.scratch),
        );
        let composed = queued.composed;
        // A real turn's clock line (overnight A2), so its bytes count.
        let clock = {
            use chrono::TimeZone;
            let edt = chrono::FixedOffset::west_opt(4 * 3600).unwrap();
            crate::chat::clock_line(&edt.with_ymd_and_hms(2026, 10, 9, 19, 16, 0).unwrap())
        };
        // The turn's mode line (HB01), as execute_turn builds it, with the
        // folders a Claude turn reads (OB02).
        let reads = crate::mode_contract::Reads {
            scratch: true,
            extra: &[],
            app: caps.app,
            vault: caps.vault,
        };
        let mode = crate::mode_contract::turn_line(
            perm,
            crate::agent::capabilities(self.agent).owned_mcp,
            caps.app && !caps.app_read_only,
            caps.computer,
            crate::capability_contract::reads_only_its_roots(self.agent).then_some(&reads),
        );
        let composer = TurnComposer {
            vault: self.vault,
            session_key: self.key,
            continued_from: &[],
            host: false,
            turn_ord: ord,
            entry: queued.entry,
            operator_enabled: true,
            reader: crate::task_state::Reader::AppTools,
            composed,
            guide: &guide,
            agent: self.agent,
            append: &append,
            policy: &policy,
            workspace: envelope.as_ref(),
            rebuild: &rebuild,
            native: (session == Session::Resume).then_some(NATIVE),
            recall: per.recall,
            connectors_note: "",
            skills: per.skills,
            evidence: "",
            clock: &clock,
            context_note: "",
            mode: &mode,
        };
        let turn = composer.compose(session);
        // The context layers execute_turn records on the run's manifest
        // (F11), paths normalized (OB15's golden envelope).
        let layers = {
            let n = |text: &str| self.normalize(text);
            let append = AppendLayers {
                visual: n(&append_layers.visual),
                index: n(&append_layers.index),
                brief: n(&append_layers.brief),
                policy: n(&append_layers.policy),
            };
            let workspace = turn.workspace.as_deref().map(n);
            let left_out = turn.left_out;
            let mut manifest = crate::run_manifest::RunManifest::new();
            record_turn_layers(
                &mut manifest,
                &TurnLayers {
                    append: &append,
                    configured_held: turn.configured_held,
                    workspace: workspace.as_deref(),
                    recall: &n(left_out.sent(LeftOut::RECALL, per.recall)),
                    clock: &n(&clock),
                    context_note: "",
                    mode: &n(left_out.sent(LeftOut::MODE, &mode)),
                    connectors_note: "",
                    skills: &n(left_out.sent(LeftOut::SKILLS, per.skills)),
                    evidence: "",
                    wrapped: &n(&composer.wrap(left_out, turn.workspace.as_deref(), "")),
                },
                &n(&turn.prompt),
            );
            manifest
                .layers
                .into_iter()
                .map(|layer| (layer.name, layer.bytes))
                .collect()
        };
        // Every simulated turn reached its agent: the guide its prompt
        // carried is recorded, as execute_turn does after the spawn (G1).
        if let Some(given) = &turn.guide_given {
            given.record(self.vault, self.key, ord);
        }
        // What the native session now holds, decided and written as
        // execute_turn does once the agent ran.
        if self.baseline {
            let mut held = turn.pending.clone();
            let compacted = ending.compacted;
            let reached = [announced, (session == Session::Resume).then_some(NATIVE)];
            crate::stable_layers::unsettle(self.vault, self.key, ord, &held, &reached, compacted);
            if let Some(native) = stable_layers_holder(true, finished, announced, &held.native) {
                held.native = native.to_string();
                if compacted {
                    crate::stable_layers::forget_layers(&mut held);
                    held.compacted_turn = ord;
                }
                crate::stable_layers::write(self.vault, self.key, ord, &held);
            }
        }
        let resumed = session == Session::Resume;
        let spec = crate::agent::RunSpec {
            prompt: turn.prompt.clone(),
            resume: resumed.then(|| NATIVE.to_string()),
            model: None,
            perm: perm.to_string(),
            cwd: self.cwd.clone(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            interactive: true,
            mcp_config: None,
            append_system_prompt: Some(append.clone()),
            process_scope: crate::agent::ProcessScope::GuiOwned,
            usage_stream: false,
            max_budget_usd: None,
            extras: crate::agent::RunExtras {
                configured_held: turn.configured_held,
                // As execute_turn: the message names the mode (HB01).
                mode_per_turn: true,
                ..Default::default()
            },
        };
        let (system, prompt) = if self.agent == AgentKind::Claude {
            (format!("{append}\n\n{policy}"), turn.prompt.clone())
        } else {
            (
                String::new(),
                crate::agent::contextual_prompt(self.agent, &spec),
            )
        };
        let configured = if self.agent == AgentKind::Claude {
            self.normalize(&system).len()
        } else {
            self.normalize(&prompt).len() - self.normalize(&turn.prompt).len()
        };
        crate::session_store::append_event(self.vault, self.key, ord, "assistant", reply).unwrap();
        crate::session_store::finish_turn(
            self.vault,
            self.key,
            ord,
            if finished { "success" } else { "error" },
        )
        .unwrap();
        let bytes = Bytes {
            system: self.normalize(&system).len(),
            prompt: self.normalize(&prompt).len(),
            configured,
            workspace: turn
                .workspace
                .as_deref()
                .map_or(0, |w| self.normalize(w).len()),
        };
        Sent {
            bytes,
            prompt: self.normalize(&prompt),
            record: turn.record,
            layers,
            parts: vec![
                ("visualPrompt".into(), self.normalize(&append_layers.visual)),
                (
                    "capabilityIndex".into(),
                    self.normalize(&append_layers.index),
                ),
                (
                    "environmentBrief".into(),
                    self.normalize(&append_layers.brief),
                ),
                (
                    "policyGuidance".into(),
                    self.normalize(&append_layers.policy),
                ),
                ("chatPolicy".into(), self.normalize(&policy)),
            ],
        }
    }
}

/// Who queued a measured turn and its text as queued (OB15).
#[derive(Clone, Copy)]
pub(super) struct Queued<'a> {
    pub entry: crate::context_compiler::Entry,
    /// What the turn records as asked, and what retrieval and the visual
    /// cue read.
    pub request: &'a str,
    /// What the context compiler gets.
    pub composed: &'a str,
    /// Hollow's tag for its own turn (`ChatSendRequest::host_asker`): a
    /// check-in's never cues the visual guide (`chat_queue::turn_visual_cue`).
    pub asker: Option<&'static str>,
}

/// How a measured turn's agent ended: `finished` (a success) or not, the
/// native id its stream `announced`, if any, and whether it `compacted`
/// the session's history (S02 review).
#[derive(Clone, Copy)]
pub(super) struct Ending<'a> {
    pub finished: bool,
    pub announced: Option<&'a str>,
    pub compacted: bool,
}

impl Ending<'_> {
    /// A finished reply that announced the measured native id.
    pub fn finished() -> Self {
        Ending {
            finished: true,
            announced: Some(NATIVE),
            compacted: false,
        }
    }

    /// A reply that announced the measured native id and then stopped (a
    /// Stop, an API error).
    pub fn stopped() -> Self {
        Ending {
            finished: false,
            ..Self::finished()
        }
    }
}

/// A measured turn's per-turn layers (S02): its approved recall, the
/// permission mode its `<hollow-mode>` line names and its `<hollow-skills>`
/// block (A15). By default no recall and no skills, in accept edits.
#[derive(Clone, Copy)]
pub(super) struct PerTurn<'a> {
    pub recall: &'a str,
    pub perm: &'a str,
    pub skills: &'a str,
}

impl Default for PerTurn<'_> {
    fn default() -> Self {
        PerTurn {
            recall: "",
            perm: "acceptEdits",
            skills: "",
        }
    }
}

/// What one turn sent.
pub(super) struct Sent {
    pub bytes: Bytes,
    pub prompt: String,
    pub record: crate::run_manifest::CompilerRecord,
    /// Each context layer's name and bytes, as the run's manifest records
    /// them (`record_turn_layers`), paths normalized.
    pub layers: Vec<(String, u64)>,
    /// The appended system prompt's parts as text (COO rebuild L10: the
    /// overhead ablation reads them through `HOLLOW_DUMP_LAYERS`).
    pub parts: Vec<(String, String)>,
}

/// The measured conversations: a workspace research chat (a first turn,
/// a resumed follow-up on the same sources, then a resumed one on a new
/// topic, whose excerpts are new) and a vault chat (first, resumed).
pub(super) const SURFACES: [(&str, bool, &[(&str, &str)]); 2] = [
    (
        "workspaceResearch",
        true,
        &[
            (
                "first",
                "Summarize what the attached sources say about churn, with citations.",
            ),
            (
                "resumedSameSources",
                "Again: summarize what the attached sources say about churn, with citations, shorter.",
            ),
            (
                "resumedNewTopic",
                "And what do they say about retention in cohort 12?",
            ),
        ],
    ),
    (
        "vaultChat",
        false,
        &[
            ("first", "What time zone is the Lisbon office in?"),
            ("resumed", "And the Porto office?"),
        ],
    ),
];

/// Rows for every provider and surface, turn by turn (the first on a new
/// native session, the rest resumed): `before`, as every turn was sent
/// before G2 (no baseline, so everything in full), and `after`.
pub(super) fn rows() -> Vec<serde_json::Value> {
    let measure = |agent: AgentKind, workspace: bool, turns: &[(&str, &str)], baseline: bool| {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let ws = workspace_vault(vault);
        let chat = Chat {
            agent,
            vault,
            cwd: if workspace { ws } else { vault.to_path_buf() },
            key: "s-20261007-g2000001",
            scratch: scratch.path().to_path_buf(),
            baseline,
        };
        turns
            .iter()
            .enumerate()
            .map(|(i, (_, request))| {
                let session = if i == 0 {
                    Session::New
                } else {
                    Session::Resume
                };
                chat.turn(request, session, "Here is the answer.").bytes
            })
            .collect::<Vec<_>>()
    };
    let mut rows = Vec::new();
    for agent in AGENTS {
        for (name, workspace, turns) in SURFACES {
            let before = measure(agent, workspace, turns, false);
            let after = measure(agent, workspace, turns, true);
            for (((turn, _), before), after) in turns.iter().zip(before).zip(after) {
                rows.push(serde_json::json!({
                    "agent": agent.as_str(),
                    "surface": name,
                    "turn": turn,
                    "before": {
                        "systemPrompt": before.system,
                        "prompt": before.prompt,
                        "configured": before.configured,
                        "workspace": before.workspace,
                        "total": before.total(),
                    },
                    "after": {
                        "systemPrompt": after.system,
                        "prompt": after.prompt,
                        "configured": after.configured,
                        "workspace": after.workspace,
                        "total": after.total(),
                    },
                }));
            }
        }
    }
    rows
}

#[test]
fn turn_budget_fixture_is_current() {
    let fixture = serde_json::json!({
        "_doc": "Generated by chat_g2_budget_tests::turn_budget_fixture_is_current: bytes one chat turn carries per provider (Claude: --append-system-prompt and the -p prompt; Codex, Antigravity, Grok: the one prompt), turn by turn (first: a new native session; the rest resumed), through TurnComposer with a real workspace envelope and session store. before: every turn in full, as before G2 (no stable-layer baseline); after: with G2's deltas. No recall, connectors or retrieved evidence. Do not edit; regenerate with HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib turn_budget.",
        "rows": rows(),
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/prompts/turn-budget.json");
    if std::env::var_os("HOLLOW_REGENERATE_PROMPTS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == text,
        "tests/fixtures/prompts/turn-budget.json is stale; from src-tauri/ run HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib turn_budget"
    );
}

/// OB15 · the golden envelope's turns: the day's first Main agent check-in
/// (a new thread), a later one resuming it, a resumed composer turn, an
/// app-session packet child and a reset after a config change.
pub(super) const ENVELOPE_SCENARIOS: [&str; 5] = [
    "checkInWake",
    "checkInResume",
    "composerResume",
    "packetChild",
    "configReset",
];

/// [`workspace_vault`] plus the Main agent's workspace with a 51 KB
/// sectioned brief, shaped like the real one in HB02: frontmatter, Outcome,
/// a `## Current context` of 340 dated entries oldest first, then
/// `## Decisions` from about byte 46,000. Returns the Main agent's folder.
fn main_agent_vault(vault: &Path) -> PathBuf {
    workspace_vault(vault);
    let cwd = crate::workspaces::ensure_main_agent(vault).unwrap();
    let entries: String = (0..340)
        .map(|i| {
            format!(
                "- 2026-09-{:02} {:02}:{:02}: started a chat in release-review on cohort {i} churn (run {i:04}); waiting on the pricing export before the readout.\n",
                1 + i / 12,
                8 + i % 12,
                (i * 7) % 60
            )
        })
        .collect();
    std::fs::write(
        cwd.join("WORKSPACE.md"),
        format!(
            "---\nhollow_workspace: 1\nkind: main\ntitle: \"COO\"\nstatus: active\n---\n\n# COO\n\n## Outcome\n\nThe always-on COO: what is running across workspaces, what needs the person, and what to do next.\n\n## Current context\n\n{entries}\n## Decisions\n\n{}",
            lines("Decision", 50)
        ),
    )
    .unwrap();
    cwd
}

/// [`workspace_vault`] with 75 KB of workspace documents (a 30 KB brief and
/// a 45 KB plan) beside its three attached sources: a packet child's
/// workspace.
fn packet_vault(vault: &Path) -> PathBuf {
    let cwd = workspace_vault(vault);
    std::fs::write(
        cwd.join("WORKSPACE.md"),
        format!(
            "---\nhollow_workspace: 1\ntitle: Release review\nstatus: active\n---\n# Release review\n\nOutcome: a churn and retention readout for the release.\n\n{}",
            lines("Brief", 290)
        ),
    )
    .unwrap();
    std::fs::write(
        cwd.join("PLAN.md"),
        format!("# Plan\n\n{}", lines("Plan", 440)),
    )
    .unwrap();
    cwd
}

/// A check-in's text as `main_agent::decide_pulse` builds it, with two new
/// signals and nothing declined: the day's first (`first_today`) or a later
/// one.
fn check_in_text(first_today: bool) -> String {
    let signals = [
        "- A run failed: Nightly digest exit status 1: usage limit reached",
        "- Waiting on you (question): Release review asks which cohorts to chart",
    ];
    let mut text = format!(
        "{} {}.",
        crate::main_agent::CHECKIN_OPEN,
        if first_today { "09:00" } else { "09:30" }
    );
    if first_today {
        text.push_str(
            " This is the first check-in today: open with a brief of the day in three lines.",
        );
    }
    text.push_str(crate::main_agent::CHECKIN_NEW);
    text.push_str(&signals.join("\n"));
    text.push_str("\n\n");
    text.push_str(crate::main_agent::PULSE_GUIDANCE);
    text
}

/// A packet child's task: what `TurnRequest::admit` leaves as its request
/// once the `<hollow-task-packet>` block is split off (app_sessions.rs
/// asserts it), so its numbers don't read as a visual cue.
const PACKET_TASK: &str = "Draft the churn readout for the release from the attached sources.";

/// A durable child task's first text as `app_sessions::child_prompt` builds
/// it for a caller with no conversation: the packet the caller passed, then
/// its task.
fn packet_child_text() -> String {
    let request = crate::delegation::parse(Some(&serde_json::json!({
        "constraints": [
            "Cite the file and lines for every number.",
            "Churn is measured on paid seats, not accounts."
        ],
        "sources": [
            {"path": "workspaces/release-review/sources/churn.md"},
            {
                "path": "workspaces/release-review/sources/retention.md",
                "excerpt": "- Retention note 012: retention in cohort 12 moved by 5 points after the March release, per the weekly export."
            }
        ],
        "expected": "A Markdown table of churn by cohort, then three findings.",
        "budget": {"seconds": 1800, "usd": 2.0}
    })))
    .unwrap();
    let packet = crate::delegation::build(
        crate::delegation::Contract::ChildTask,
        request,
        crate::delegation::Parent::default(),
    );
    crate::delegation::prepend(&packet.render().text, PACKET_TASK)
}

/// The measured turn of `scenario` for `agent`, on its own vault, after the
/// turns that lead to it.
fn envelope_turn(agent: AgentKind, scenario: &str) -> Sent {
    use crate::context_compiler::Entry;
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let vault = dir.path();
    let cwd = match scenario {
        "checkInWake" | "checkInResume" => main_agent_vault(vault),
        "packetChild" => packet_vault(vault),
        _ => workspace_vault(vault),
    };
    let chat = Chat {
        agent,
        vault,
        cwd: cwd.clone(),
        key: "s-20261017-ob150001",
        scratch: scratch.path().to_path_buf(),
        baseline: true,
    };
    let first = "Summarize what the attached sources say about churn, with citations.";
    let check_in = |first_today: bool, session: Session| {
        let text = check_in_text(first_today);
        let queued = Queued {
            entry: Entry::Composer,
            request: &text,
            composed: &text,
            asker: Some(crate::main_agent::ASKED_BY_CHECKIN),
        };
        chat.queued(queued, session, "PULSE_OK", true, Some(NATIVE))
    };
    match scenario {
        // The day's first check-in opens today's thread: `today_thread`
        // finds none, so `thread_basis` names no native session (acceptEdits).
        "checkInWake" => check_in(true, Session::New),
        // A later check-in resumes it, after the agent added an entry to its
        // notepad.
        "checkInResume" => {
            check_in(true, Session::New);
            let brief = cwd.join("WORKSPACE.md");
            let text = std::fs::read_to_string(&brief).unwrap();
            let text = text.replace(
                "\n## Decisions",
                "- 2026-10-17 09:05: the churn readout chat finished; the pricing export is still missing.\n\n## Decisions",
            );
            std::fs::write(&brief, text).unwrap();
            check_in(false, Session::Resume)
        }
        "composerResume" => {
            chat.turn(first, Session::New, "Churn rose in cohorts 3 and 5.");
            chat.turn(
                "And what do they say about retention in cohort 12?",
                Session::Resume,
                "It held.",
            )
        }
        // A `start_session` child: a new conversation, one level deeper,
        // whose request is its task (`TurnRequest::admit` splits the packet
        // off).
        "packetChild" => {
            let text = packet_child_text();
            let admitted = crate::turn_request::TurnRequest::admit(vault, &text, None);
            let queued = Queued {
                entry: Entry::AppSession,
                request: &admitted.request,
                composed: &text,
                asker: None,
            };
            chat.queued(
                queued,
                Session::New,
                "Here is the readout.",
                true,
                Some(NATIVE),
            )
        }
        "configReset" => {
            chat.turn(first, Session::New, "Churn rose.");
            chat.turn("And retention in cohort 12?", Session::Resume, "It held.");
            chat.turn(
                "Start over: what about pricing?",
                Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
                "Pricing moved.",
            )
        }
        other => panic!("no envelope scenario {other}"),
    }
}

/// OB15 · the golden envelope (a 100-byte change to one guidance string
/// fails it until regenerated): per provider, each
/// scenario's measured turn with every context layer's bytes as the run's
/// manifest records them. Sequential, like [`rows`] (OB13: unit tests skip
/// `F_FULLFSYNC`, so no fan-out). `scripts/agent-eval/envelope-diff.mjs`
/// prints two versions' per-layer deltas.
/// COO rebuild L10, the layer manifest: a subagent's turn carries no visual
/// guide, no question-card policy and no next-step block (the COO's and a
/// lead's do), for every provider; its brief is unchanged.
#[test]
fn a_subagent_turn_carries_no_visual_guide_question_policy_or_next_step() {
    let dir = tempfile::tempdir().unwrap();
    let vault = dir.path();
    let cwd = workspace_vault(vault);
    for agent in AGENTS {
        let caps = caps_for(agent);
        let full = chat_append_layers(true, vault, &cwd, caps, CanvasFrequency::Helps, "default");
        let question = crate::interaction_schema::guidance(caps.questions, caps.questions_fenced)
            .expect("these caps ask");
        assert!(
            full.policy.contains(crate::next_step::GUIDANCE),
            "{agent:?}"
        );
        assert!(full.policy.contains(question), "{agent:?}");
        let brief = full.brief.clone();
        let lean = lean_subagent(full, caps);
        assert_eq!(lean.visual, LEAN_VISUAL, "{agent:?}: no visual guide");
        assert_eq!(lean.index, LEAN_INDEX, "{agent:?}: the short index");
        assert!(
            !lean.policy.contains(crate::next_step::GUIDANCE),
            "{agent:?}"
        );
        assert!(!lean.policy.contains(question), "{agent:?}");
        assert!(
            lean.policy.contains(BACKGROUND_GUIDANCE),
            "{agent:?}: the rest stays"
        );
        assert_eq!(lean.brief, brief, "{agent:?}");
        // The fixture's packet child (a subagent's first turn) records it.
        let sent = envelope_turn(agent, "packetChild");
        let part = |name: &str| {
            sent.parts
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, t)| t.clone())
                .unwrap()
        };
        assert_eq!(part("visualPrompt"), LEAN_VISUAL, "{agent:?}");
        assert!(!part("policyGuidance").contains(crate::next_step::GUIDANCE));
    }
}

#[test]
fn envelope_fixture_is_current() {
    let mut scenarios = Vec::new();
    for agent in AGENTS {
        for scenario in ENVELOPE_SCENARIOS {
            let sent = envelope_turn(agent, scenario);
            // L10: the ablation's layer texts, only when asked for.
            if let Some(dir) = std::env::var_os("HOLLOW_DUMP_LAYERS") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                let stem = format!("{}-{scenario}", agent.as_str());
                std::fs::write(dir.join(format!("{stem}.prompt.txt")), &sent.prompt).unwrap();
                for (name, text) in &sent.parts {
                    std::fs::write(dir.join(format!("{stem}.{name}.txt")), text).unwrap();
                }
            }
            scenarios.push(serde_json::json!({
                "scenario": scenario,
                "agent": agent.as_str(),
                "entry": sent.record.entry,
                "session": sent.record.session,
                "workspace": sent.record.stable_layers.as_ref().map(|s| s.workspace.clone()),
                "systemPrompt": sent.bytes.system,
                "prompt": sent.bytes.prompt,
                "total": sent.bytes.total(),
                "layers": sent
                    .layers
                    .iter()
                    .map(|(name, bytes)| serde_json::json!({ "name": name, "bytes": bytes }))
                    .collect::<Vec<_>>(),
            }));
        }
    }
    let fixture = serde_json::json!({
        "_doc": "Generated by chat_g2_budget_tests::envelope_fixture_is_current (OB15, the golden envelope): per provider, five real-shaped chat turns through TurnComposer with a real workspace envelope and session store. checkInWake: the day's first COO check-in, opening today's thread (a new native session) on a 51 KB sectioned brief; checkInResume: a later check-in resuming that thread after the brief gained an entry; composerResume: a resumed workspace turn; packetChild: an app-session child's first turn, its packet first and its task as the request, in a 75 KB workspace (it gets the slim child envelope, S01); configReset: a reset after a config change on the third turn. layers: each context layer's UTF-8 bytes as the run's manifest records them (record_turn_layers), paths normalized; systemPrompt, prompt and total as in turn-budget.json. No recall, connectors or retrieved evidence. Compare two versions with node scripts/agent-eval/envelope-diff.mjs. Do not edit; regenerate with HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib envelope_fixture.",
        "scenarios": scenarios,
    });
    let text = format!("{}\n", serde_json::to_string_pretty(&fixture).unwrap());
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/prompts/envelope.json");
    if std::env::var_os("HOLLOW_REGENERATE_PROMPTS").is_some() {
        std::fs::write(&path, &text).unwrap();
    }
    let current = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        current == text,
        "tests/fixtures/prompts/envelope.json is stale; from src-tauri/ run HOLLOW_REGENERATE_PROMPTS=1 cargo test --lib envelope_fixture"
    );
}

/// The total budget the red-first tests hold every provider to (G2).
const TOTAL_BUDGET: usize = crate::context_budget::TOTAL_BYTES;

/// A heavy workspace chat: a 70 KB plan, a 10 KB brief, 63 attached
/// sources, a 30-turn conversation with long replies and a profile over its
/// 16,000-byte total whose settled decision and guardrail are the last two
/// files. Returns the workspace folder.
fn heavy_vault(vault: &Path, key: &str) -> PathBuf {
    let cwd = workspace_vault(vault);
    std::fs::write(
        cwd.join("PLAN.md"),
        format!("# Plan\n\n{}", lines("Plan", 600)),
    )
    .unwrap();
    std::fs::write(
        cwd.join("WORKSPACE.md"),
        format!(
            "---\nhollow_workspace: 1\ntitle: Release review\nstatus: active\n---\n# Release review\n\n{}",
            lines("Brief", 80)
        ),
    )
    .unwrap();
    for i in 0..60 {
        std::fs::write(
            cwd.join("sources").join(format!("export-{i:02}.csv")),
            format!("cohort,week,value\n{i},1,{i}\n"),
        )
        .unwrap();
    }
    let me = vault.join("me");
    let prose = |name: &str| -> String {
        (1..=100)
            .map(|i| format!("{name} line {i:03} {}\n", "p".repeat(40)))
            .collect()
    };
    std::fs::write(me.join("Identity.md"), prose("Identity")).unwrap();
    std::fs::write(me.join("Communication.md"), prose("Comms")).unwrap();
    std::fs::write(me.join("Anti-patterns.md"), prose("Anti")).unwrap();
    std::fs::write(
        me.join("Guardrails.md"),
        "- Never send email without asking (guardrail amber-fox).\n",
    )
    .unwrap();
    std::fs::write(
        me.join("Decisions.md"),
        "- Churn is measured on paid seats (decision quartz-heron).\n",
    )
    .unwrap();
    for i in 1..=30 {
        let asked = if i == 1 {
            "Draft the readout. Always cite file and line (constraint cobalt-wren).".to_string()
        } else {
            format!("Next section {i}, please.")
        };
        let ord = crate::session_store::start_turn(
            vault,
            key,
            "claude",
            &cwd.to_string_lossy(),
            &format!("run-heavy-{i}"),
            &asked,
        )
        .unwrap();
        crate::session_store::append_event(
            vault,
            key,
            ord,
            "assistant",
            &format!("Section {i}. {}", "r".repeat(2_000)),
        )
        .unwrap();
        crate::session_store::finish_turn(vault, key, ord, "success").unwrap();
    }
    cwd
}

/// F6 acceptance (G2, red first): one total budget per provider. A fresh
/// turn of a heavy workspace chat (reset, so the conversation, task state
/// and profile all go in, plus a visual request's guide) stays within it,
/// with the request, the task state's constraint and the settled decision
/// and guardrail kept.
#[test]
fn every_provider_stays_within_its_total_budget() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261007-g2000002";
        let cwd = heavy_vault(vault, key);
        let chat = Chat {
            agent,
            vault,
            cwd,
            key,
            scratch: scratch.path().to_path_buf(),
            baseline: true,
        };
        let request = "Compare churn and retention across the cohorts in a chart.";
        let sent = chat.turn(
            request,
            Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
            "Done.",
        );
        let budget = sent.record.budget.clone().expect("budget");
        eprintln!(
            "{}: {} bytes (system {}, prompt {}); {}",
            agent.as_str(),
            sent.bytes.total(),
            sent.bytes.system,
            sent.bytes.prompt,
            budget
                .layers
                .iter()
                .filter(|l| l.wanted > 0)
                .map(|l| format!("{} {}/{}", l.name, l.granted, l.wanted))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert!(!budget.over, "{budget:?}");
        // Lowest priority first: the guide and the prose are cut before
        // any evidence, and the settled layers never.
        let layer = |name: &str| budget.layers.iter().find(|l| l.name == name).unwrap();
        assert_eq!(layer("visualGuide").granted, 0, "{budget:?}");
        assert!(
            layer("profileProse").granted < layer("profileProse").wanted,
            "{budget:?}"
        );
        for name in [
            "taskState",
            "profileSettled",
            "request",
            "configured",
            "workspace",
            "conversationContext",
        ] {
            let got = layer(name);
            if name == "workspace" {
                // The plan is cut to its share, so a shorter file does not
                // shrink this layer. SCREEN_JOBS sits in the never-cut
                // configured half and takes the last of the slack from the
                // workspace, and so does the dependency rule (11-10).
                // Anything past the two allowances still fails.
                let short = got.wanted.saturating_sub(got.granted);
                assert!(
                    short
                        <= (super::SCREEN_JOBS_ALLOWANCE + super::DEPENDENCY_RULE_ALLOWANCE) as u64,
                    "{name} lost {short} bytes"
                );
            } else {
                assert_eq!(got.granted, got.wanted, "{name}");
            }
        }
        assert_eq!(
            sent.record.visual_guide.as_ref().unwrap().delivery,
            "onDemand",
            "the guide stays in its file"
        );
        assert!(
            sent.bytes.total() <= TOTAL_BUDGET,
            "{}: {} bytes over {TOTAL_BUDGET}",
            agent.as_str(),
            sent.bytes.total()
        );
        for kept in [
            request,
            "constraint cobalt-wren",
            "decision quartz-heron",
            "guardrail amber-fox",
        ] {
            assert!(sent.prompt.contains(kept), "{}: {kept}", agent.as_str());
        }
    }
}

/// F6 acceptance (G2, red first): where the native session holds the
/// earlier turns (an exact resume), a resumed turn carries the workspace
/// envelope as a delta, not again in full, and Codex, Antigravity and Grok
/// don't repeat the configured half; a reset gets everything in full.
#[test]
fn a_resumed_turn_sends_a_delta_and_a_reset_sends_everything() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let cwd = workspace_vault(vault);
        let chat = Chat {
            agent,
            vault,
            cwd,
            key: "s-20261007-g2000003",
            scratch: scratch.path().to_path_buf(),
            baseline: true,
        };
        let first = chat.turn(
            "Summarize what the attached sources say about churn, with citations.",
            Session::New,
            "Churn rose.",
        );
        assert!(
            first.prompt.contains("Plan note 040"),
            "a first turn is full"
        );
        let resumed = chat.turn(
            "And what do they say about churn in cohort 12?",
            Session::Resume,
            "It held.",
        );
        let name = agent.as_str();
        assert!(
            !resumed.prompt.contains("Plan note 040"),
            "{name}: the unchanged plan was sent again"
        );
        let stable = resumed.record.stable_layers.clone().unwrap();
        assert_eq!(stable.workspace, "delta", "{name}");
        assert_eq!(stable.unchanged, 3, "{name}: brief, plan and sources list");
        assert!(
            resumed.bytes.workspace < first.bytes.workspace,
            "{name}: envelope {} bytes resumed, {} first",
            resumed.bytes.workspace,
            first.bytes.workspace
        );
        // The same request again: every excerpt it gets was given, so
        // they are named by range and nothing is resent.
        let again = chat.turn(
            "Summarize what the attached sources say about churn, with citations.",
            Session::Resume,
            "Same.",
        );
        let stable = again.record.stable_layers.clone().unwrap();
        assert!(stable.excerpts_referenced > 0, "{name}: {stable:?}");
        assert!(
            again.bytes.workspace * 10 < first.bytes.workspace,
            "{name}: envelope {} bytes for the same request, {} first",
            again.bytes.workspace,
            first.bytes.workspace
        );
        assert_eq!(
            stable.configured,
            if agent == AgentKind::Claude {
                "systemPrompt"
            } else {
                "held"
            }
        );
        if agent != AgentKind::Claude {
            assert!(
                !resumed.prompt.contains(INDEX_HEADING),
                "{name}: the configured half was repeated"
            );
            assert!(
                resumed.prompt.contains(crate::agent::WORKSPACE_POLICY),
                "{name}: the permission paragraph goes every turn"
            );
        }
        let reset = chat.turn(
            "Start over: what about retention?",
            Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
            "Retention held.",
        );
        assert!(
            reset.prompt.contains("Plan note 040"),
            "{name}: a reset is full"
        );
        let stable = reset.record.stable_layers.clone().unwrap();
        assert_eq!(stable.workspace, "full", "{name}");
        if agent != AgentKind::Claude {
            assert!(reset.prompt.contains(INDEX_HEADING), "{name}");
            assert_eq!(stable.configured, "full", "{name}");
        }
    }
}

/// S02: approved recall as a measured turn's [`PerTurn`].
const RECALL: &str = "Approved memory: churn is measured on paid seats (recall ochre-finch).";

/// A turn with approved `recall`, in accept edits (S02).
fn recalled(recall: &str) -> PerTurn<'_> {
    PerTurn {
        recall,
        ..Default::default()
    }
}

/// A turn in permission mode `perm`, with no recall (S02).
fn in_mode(perm: &str) -> PerTurn<'_> {
    PerTurn {
        perm,
        ..Default::default()
    }
}

/// A workspace chat with `agent` whose turns record what the session holds
/// (S02). Keep the two folders alive with it.
fn per_turn_chat<'a>(
    agent: AgentKind,
    vault: &'a tempfile::TempDir,
    scratch: &tempfile::TempDir,
    key: &'a str,
) -> Chat<'a> {
    Chat {
        agent,
        vault: vault.path(),
        cwd: workspace_vault(vault.path()),
        key,
        scratch: scratch.path().to_path_buf(),
        baseline: true,
    }
}

/// The per-turn layers a turn left out as given (S02).
fn held_layers(sent: &Sent) -> Vec<String> {
    sent.record
        .stable_layers
        .as_ref()
        .map_or_else(Vec::new, |s| s.held_layers.clone())
}

/// Whether a turn's manifest records layer `name`, so its prompt carried it.
fn carried(sent: &Sent, name: &str) -> bool {
    sent.layers.iter().any(|(layer, _)| layer == name)
}

/// S02 (red first): a resumed turn whose session was given the same
/// approved recall leaves it out. Two resumed turns with identical recall
/// send it on the first only; changed recall is sent. The composer's chat
/// intent, unchanged since the opening turn, goes on neither.
#[test]
fn identical_recall_and_chat_intent_are_sent_once_per_session() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200001");
        let name = agent.as_str();
        let first = chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        assert!(first.prompt.contains("<hollow-chat-intent>"), "{name}");
        assert!(carried(&first, "chatIntent"), "{name}");
        let ending = Ending::finished();
        let resumed = chat.turn_with(
            "And retention?",
            Session::Resume,
            "It held.",
            ending,
            recalled(RECALL),
        );
        assert!(
            resumed.prompt.contains("recall ochre-finch"),
            "{name}: recall the session never had is sent"
        );
        assert!(
            !resumed.prompt.contains("<hollow-chat-intent>"),
            "{name}: the intent was given on the first turn"
        );
        assert!(!carried(&resumed, "chatIntent"), "{name}");
        assert_eq!(
            held_layers(&resumed),
            ["chatIntent", "permissionMode"],
            "{name}"
        );
        let again = chat.turn_with(
            "And pricing?",
            Session::Resume,
            "Flat.",
            ending,
            recalled(RECALL),
        );
        assert!(
            !again.prompt.contains("recall ochre-finch"),
            "{name}: the same recall was sent again"
        );
        assert!(!carried(&again, "recall"), "{name}");
        assert!(
            again.prompt.contains("And pricing?"),
            "{name}: the request stays"
        );
        assert_eq!(
            held_layers(&again),
            ["recall", "chatIntent", "permissionMode"],
            "{name}"
        );
        let changed = chat.turn_with(
            "And churn?",
            Session::Resume,
            "Up.",
            ending,
            recalled("Approved memory: pricing is reviewed monthly (recall teal-heron)."),
        );
        assert!(
            changed.prompt.contains("recall teal-heron"),
            "{name}: changed recall is sent"
        );
    }
}

/// S02 (red first): the `<hollow-mode>` line goes once per native session
/// and again whenever the mode changes (a permission flip, and back), so
/// the latest line in the conversation is always the mode that applies.
#[test]
fn the_mode_line_is_sent_again_only_when_the_mode_changes() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200002");
        let name = agent.as_str();
        let owned_mcp = crate::agent::capabilities(agent).owned_mcp;
        let accept_line = crate::mode_contract::policy("acceptEdits", owned_mcp);
        let bypass_line = crate::mode_contract::policy("bypassPermissions", owned_mcp);
        let ending = Ending::finished();
        let first = chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        assert!(first.prompt.contains(&accept_line), "{name}");
        let same = chat.turn("And retention?", Session::Resume, "It held.");
        assert!(
            !same.prompt.contains("</hollow-mode>"),
            "{name}: the same mode line was sent again"
        );
        assert!(!carried(&same, "permissionMode"), "{name}");
        let bypass = in_mode("bypassPermissions");
        let flipped = chat.turn_with("Fix it.", Session::Resume, "Fixed.", ending, bypass);
        assert!(
            flipped.prompt.contains(&bypass_line),
            "{name}: a changed mode's line is sent"
        );
        assert!(carried(&flipped, "permissionMode"), "{name}");
        let still = chat.turn_with("And now?", Session::Resume, "Done.", ending, bypass);
        assert!(!still.prompt.contains("</hollow-mode>"), "{name}");
        let back = chat.turn("Back to review.", Session::Resume, "Reviewed.");
        assert!(
            back.prompt.contains(&accept_line),
            "{name}: the earlier mode, back, is sent again"
        );
    }
}

/// S02 (red first): a turn the agent never finished (a failed spawn, a
/// crash; nothing announced) records nothing, so the per-turn layers it
/// carried are not counted as given and the next turn sends them again.
#[test]
fn a_failed_turn_does_not_count_its_per_turn_layers_as_given() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200003");
        let name = agent.as_str();
        chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        let per = PerTurn {
            recall: RECALL,
            perm: "bypassPermissions",
            ..Default::default()
        };
        let failed_ending = Ending {
            finished: false,
            announced: None,
            compacted: false,
        };
        let failed = chat.turn_with("And retention?", Session::Resume, "", failed_ending, per);
        assert!(failed.prompt.contains("recall ochre-finch"), "{name}");
        assert!(failed.prompt.contains("</hollow-mode>"), "{name}");
        let ending = Ending::finished();
        let next = chat.turn_with("And retention?", Session::Resume, "It held.", ending, per);
        assert!(
            next.prompt.contains("recall ochre-finch"),
            "{name}: the recall the agent never got is sent again"
        );
        assert!(
            next.prompt.contains("</hollow-mode>"),
            "{name}: so is the mode line"
        );
        let after = chat.turn_with("And pricing?", Session::Resume, "Flat.", ending, per);
        assert!(!after.prompt.contains("recall ochre-finch"), "{name}");
        assert!(!after.prompt.contains("</hollow-mode>"), "{name}");
    }
}

/// S02 review (B1, red first): a turn that announced the session's id and
/// then stopped (a Stop, an API error) may have left its message in that
/// session's history, so a mode line it carried in another text may be the
/// latest there. Back in the earlier mode, the next turn sends that mode's
/// line again. A layer the stopped turn carried as given stays given, and
/// the rest of the baseline (the envelope's) is as it was.
#[test]
fn a_stopped_turn_unsettles_the_layers_it_changed() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200005");
        let name = agent.as_str();
        let owned_mcp = crate::agent::capabilities(agent).owned_mcp;
        let accept_line = crate::mode_contract::policy("acceptEdits", owned_mcp);
        let vault = chat.vault;
        chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        let (_, before) = crate::stable_layers::read(vault, chat.key).expect("baseline");
        let bypass = in_mode("bypassPermissions");
        let stopped = chat.turn_with("Fix it.", Session::Resume, "", Ending::stopped(), bypass);
        assert!(stopped.prompt.contains("</hollow-mode>"), "{name}");
        let (_, after) = crate::stable_layers::read(vault, chat.key).expect("baseline");
        // Only the chat intent, carried as given, stays given.
        assert_eq!(after.layers.len(), 1, "{name}: {after:?}");
        let mut envelope = after.clone();
        envelope.layers = before.layers.clone();
        assert_eq!(envelope, before, "{name}: the rest is as it was");
        let back = chat.turn("Back to review.", Session::Resume, "Reviewed.");
        assert!(
            back.prompt.contains(&accept_line),
            "{name}: the stopped turn's bypass line may be the latest"
        );
        assert!(
            !back.prompt.contains("<hollow-chat-intent>"),
            "{name}: the intent it carried as given stays given"
        );
    }
}

/// S02 review (M1): a turn in which the agent compacted its session's
/// history counts no per-turn layer as given, so the next turn sends them
/// all (the compacting turn itself runs without them); the envelope still
/// goes as a delta, and the refresh count is unchanged.
#[test]
fn after_a_compaction_the_next_turn_resends_every_per_turn_layer() {
    for agent in [AgentKind::Claude, AgentKind::Grok] {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200006");
        let name = agent.as_str();
        chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        let ending = Ending {
            compacted: true,
            ..Ending::finished()
        };
        let compacting = chat.turn_with(
            "Next section, please.",
            Session::Resume,
            "Next.",
            ending,
            PerTurn::default(),
        );
        assert!(!compacting.prompt.contains("</hollow-mode>"), "{name}");
        let next = chat.turn("And the one after.", Session::Resume, "Next.");
        assert!(next.prompt.contains("</hollow-mode>"), "{name}");
        assert!(next.prompt.contains("<hollow-chat-intent>"), "{name}");
        let stable = next.record.stable_layers.clone().unwrap();
        assert!(stable.held_layers.is_empty(), "{name}: {stable:?}");
        assert_eq!(stable.workspace, "delta", "{name}");
        assert_eq!(stable.resumed_since_full, 2, "{name}");
        let after = chat.turn("And after that.", Session::Resume, "Next.");
        assert!(!after.prompt.contains("</hollow-mode>"), "{name}");
    }
}

/// Repo features R1 (red first): a turn whose agent reported compacting its
/// session's history (Claude's `compact_boundary`; Grok shares the parser)
/// makes exactly the next resumed turn carry the `<task-state>` record
/// again, with the compacting turn named, and the manifest says
/// `compacted`; the turn after carries neither. A stopped re-grounding turn
/// leaves the mark, so the next one sends the record again.
#[test]
fn a_compaction_re_grounds_the_next_resumed_turn_once() {
    for agent in [AgentKind::Claude, AgentKind::Grok] {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261018-0e010002");
        let name = agent.as_str();
        chat.turn(
            "Summarize the churn sources. Never cite the March export.",
            Session::New,
            "Churn rose.",
        );
        let held = chat.turn("And retention?", Session::Resume, "It held.");
        assert!(!held.prompt.contains("<task-state>"), "{name}");
        let compacted = Ending {
            compacted: true,
            ..Ending::finished()
        };
        let compacting = chat.turn_with(
            "Next section, please.",
            Session::Resume,
            "Next.",
            compacted,
            PerTurn::default(),
        );
        assert!(!compacting.prompt.contains("<task-state>"), "{name}");
        // The compacting turn is the conversation's newest: its ord is the
        // turn count.
        let turn = crate::session_store::session_by_key(chat.vault, chat.key)
            .unwrap()
            .map(|session| i64::from(session.turn_count))
            .unwrap();
        assert_eq!(turn, 3, "{name}");
        let (_, marked) = crate::stable_layers::read(chat.vault, chat.key).expect("baseline");
        assert_eq!(marked.compacted_turn, turn, "{name}: {marked:?}");
        // A stopped re-grounding turn: the record goes, the mark stays.
        let stopped = chat.turn_with(
            "And the one after.",
            Session::Resume,
            "",
            Ending::stopped(),
            PerTurn::default(),
        );
        assert!(stopped.prompt.contains("<task-state>"), "{name}");
        let next = chat.turn("And the one after.", Session::Resume, "Next.");
        let note =
            crate::context_compiler::reground_note(crate::stable_layers::Reground::Compacted(turn));
        assert!(next.prompt.contains("<task-state>"), "{name}");
        assert!(next.prompt.contains(&note), "{name}");
        assert!(
            next.prompt.contains("Never cite the March export."),
            "{name}: the rule a summary may have dropped"
        );
        assert!(carried(&next, "taskState"), "{name}");
        let state = &next.record.task_state;
        assert!(state.sent, "{name}");
        assert_eq!(state.reground.as_deref(), Some("compacted"), "{name}");
        let after = chat.turn("And after that.", Session::Resume, "Next.");
        assert!(!after.prompt.contains("<task-state>"), "{name}: only once");
        assert_eq!(after.record.task_state.reground, None, "{name}");
    }
}

/// R1 review (red first): a session's first turn compacts and is stopped,
/// so no finished turn ever recorded its baseline; the next resumed turn
/// still carries the task-state record, naming that turn, and sends
/// everything else in full (nothing was recorded as held).
#[test]
fn a_stopped_first_turn_that_compacted_re_grounds_the_next_one() {
    for agent in [AgentKind::Claude, AgentKind::Grok] {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261018-0e010003");
        let name = agent.as_str();
        let stopped = Ending {
            compacted: true,
            ..Ending::stopped()
        };
        chat.turn_with(
            "Summarize the churn sources. Never cite the March export.",
            Session::New,
            "",
            stopped,
            PerTurn::default(),
        );
        let (_, marked) = crate::stable_layers::read(chat.vault, chat.key).expect("baseline");
        assert_eq!(
            (marked.native.as_str(), marked.compacted_turn),
            (NATIVE, 1),
            "{name}"
        );
        let next = chat.turn("Go on.", Session::Resume, "Next.");
        let note =
            crate::context_compiler::reground_note(crate::stable_layers::Reground::Compacted(1));
        assert!(next.prompt.contains(&note), "{name}: {}", next.prompt);
        assert!(
            next.prompt.contains("Never cite the March export."),
            "{name}"
        );
        assert_eq!(
            next.record.task_state.reground.as_deref(),
            Some("compacted"),
            "{name}"
        );
        let stable = next.record.stable_layers.clone().unwrap();
        assert_eq!(stable.workspace, "full", "{name}: {stable:?}");
        assert!(stable.held_layers.is_empty(), "{name}: {stable:?}");
        assert!(next.prompt.contains("</hollow-mode>"), "{name}");
        let after = chat.turn("And then?", Session::Resume, "Next.");
        assert!(!after.prompt.contains("<task-state>"), "{name}: only once");
    }
}

/// S02 (red first): every [`crate::stable_layers::REFRESH_TURNS`] resumed
/// turns the session gets its per-turn layers again with the stable ones,
/// so what a provider's own compaction summarized away comes back; for
/// every agent but Claude, the task-state record too (R1).
#[test]
fn the_refresh_resends_the_per_turn_layers() {
    for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Grok] {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261017-s0200004");
        let name = agent.as_str();
        chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        for i in 1..=crate::stable_layers::REFRESH_TURNS {
            let held = chat.turn("Next section, please.", Session::Resume, "Next.");
            assert!(!held.prompt.contains("</hollow-mode>"), "{name}: turn {i}");
            assert!(!held.prompt.contains("<hollow-chat-intent>"), "{name}: {i}");
        }
        let refreshed = chat.turn("Next section, please.", Session::Resume, "Next.");
        let stable = refreshed.record.stable_layers.clone().unwrap();
        assert_eq!(stable.reason.as_deref(), Some("refresh"), "{name}");
        assert!(stable.held_layers.is_empty(), "{name}: {stable:?}");
        assert!(refreshed.prompt.contains("</hollow-mode>"), "{name}");
        assert!(refreshed.prompt.contains("<hollow-chat-intent>"), "{name}");
        // R1: the refresh re-grounds an agent whose compaction Hollow can't
        // see with the task-state record; Claude's compaction is seen.
        let note = crate::context_compiler::reground_note(crate::stable_layers::Reground::Refresh);
        let regrounds = agent != AgentKind::Claude;
        assert_eq!(
            refreshed.prompt.contains("<task-state>"),
            regrounds,
            "{name}"
        );
        assert_eq!(refreshed.prompt.contains(&note), regrounds, "{name}");
        let reground = refreshed.record.task_state.reground.as_deref();
        assert_eq!(reground, regrounds.then_some("refresh"), "{name}");
        let after = chat.turn("Next section, please.", Session::Resume, "Next.");
        assert!(!after.prompt.contains("</hollow-mode>"), "{name}");
        assert!(!after.prompt.contains("<task-state>"), "{name}");
    }
}

/// A15: a `<hollow-skills>` block with one pick, as a turn after a
/// HyperFrames request carries it.
const PICKED: &str = "<hollow-skills>\n- hyperframes: make a video.\n</hollow-skills>\n\n";

/// A turn with skill block `skills`, in accept edits (A15).
fn with_skills(skills: &str) -> PerTurn<'_> {
    PerTurn {
        skills,
        ..Default::default()
    }
}

/// A15 (red first): after a turn whose block listed picks, a resumed turn
/// with no fitting skill carries the empty block, so the stale picks are no
/// longer the latest; the manifest records the layer it carried.
#[test]
fn a_resumed_turn_after_a_skills_turn_with_no_fit_carries_the_empty_block_once() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261019-a1500001");
        let name = agent.as_str();
        let ending = Ending::finished();
        let first = chat.turn_with(
            "Make a promo video.",
            Session::New,
            "Made.",
            ending,
            with_skills(PICKED),
        );
        assert!(first.prompt.contains("- hyperframes"), "{name}");
        let unrelated = chat.turn("And the churn numbers?", Session::Resume, "Up.");
        assert!(
            unrelated
                .prompt
                .contains(crate::skill_index::NONE_FIT.trim_end()),
            "{name}: the empty block says no skill fits"
        );
        assert!(carried(&unrelated, "skillIndex"), "{name}");
        assert!(!unrelated.prompt.contains("- hyperframes"), "{name}");
    }
}

/// A15: the empty block goes once; the next turn with no fitting skill
/// leaves it out as given, and a later pick is sent again in full.
#[test]
fn not_on_the_next_turn() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261019-a1500002");
        let name = agent.as_str();
        let ending = Ending::finished();
        let none = crate::skill_index::NONE_FIT.trim_end();
        chat.turn_with(
            "Make a promo video.",
            Session::New,
            "Made.",
            ending,
            with_skills(PICKED),
        );
        let once = chat.turn("And the churn numbers?", Session::Resume, "Up.");
        assert!(once.prompt.contains(none), "{name}");
        let next = chat.turn("And retention?", Session::Resume, "Held.");
        assert!(!next.prompt.contains(none), "{name}: sent again");
        assert!(!carried(&next, "skillIndex"), "{name}");
        assert!(
            held_layers(&next).contains(&"skillIndex".to_string()),
            "{name}: the empty block is held"
        );
        let again = chat.turn_with(
            "Cut the video shorter.",
            Session::Resume,
            "Cut.",
            ending,
            with_skills(PICKED),
        );
        assert!(again.prompt.contains("- hyperframes"), "{name}");
    }
}

/// A15 review: a turn carrying the empty block that stops (its message may
/// not be in the session), or a compaction, doesn't forget that the session
/// may still hold the picks: the next turn with no fit carries it again.
#[test]
fn a_stopped_or_compacted_turn_keeps_the_empty_block_due() {
    for agent in AGENTS {
        for compacted in [false, true] {
            let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
            let scratch = tempfile::tempdir().unwrap();
            let chat = per_turn_chat(agent, &dir, &scratch, "s-20261019-a1500004");
            let name = agent.as_str();
            let none = crate::skill_index::NONE_FIT.trim_end();
            let skills = with_skills(PICKED);
            let done = Ending::finished();
            chat.turn_with("Make a promo video.", Session::New, "Made.", done, skills);
            let ending = Ending {
                finished: compacted,
                compacted,
                ..Ending::stopped()
            };
            let per = PerTurn::default();
            let first = chat.turn_with("And churn?", Session::Resume, "Up.", ending, per);
            assert!(first.prompt.contains(none), "{name} {compacted}");
            let next = chat.turn("And retention?", Session::Resume, "Held.");
            assert!(next.prompt.contains(none), "{name} {compacted}: still due");
            let after = chat.turn("And pricing?", Session::Resume, "Flat.");
            assert!(!after.prompt.contains(none), "{name} {compacted}: once");
        }
    }
}

/// A15: a new native session, and a resumed one never given a block,
/// never carry the empty block.
#[test]
fn a_fresh_session_never_carries_it() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let chat = per_turn_chat(agent, &dir, &scratch, "s-20261019-a1500003");
        let name = agent.as_str();
        let none = crate::skill_index::NONE_FIT.trim_end();
        let first = chat.turn("Summarize the churn sources.", Session::New, "Rose.");
        assert!(!first.prompt.contains(none), "{name}");
        let resumed = chat.turn("And retention?", Session::Resume, "Held.");
        assert!(!resumed.prompt.contains(none), "{name}");
        assert!(!carried(&resumed, "skillIndex"), "{name}");
        // A skills turn, then a new session (a reset): its first turn
        // starts clean, so nothing needs replacing.
        let ending = Ending::finished();
        let skills = with_skills(PICKED);
        chat.turn_with("Make a video.", Session::Resume, "Made.", ending, skills);
        let reset = chat.turn("And the churn numbers?", Session::New, "Up.");
        assert!(!reset.prompt.contains(none), "{name}: a new session");
    }
}

/// F6 acceptance (G2): the manifest shows the turn's allocation, per layer.
#[test]
fn the_manifest_shows_the_allocation() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let vault = dir.path();
    let cwd = workspace_vault(vault);
    let chat = Chat {
        agent: AgentKind::Codex,
        vault,
        cwd,
        key: "s-20261007-g2000004",
        scratch: scratch.path().to_path_buf(),
        baseline: true,
    };
    let sent = chat.turn("Summarize the churn sources.", Session::New, "Done.");
    let record = serde_json::to_value(&sent.record).unwrap();
    let budget = &record["budget"];
    assert_eq!(budget["provider"], "codex", "{record}");
    assert!(budget["total"].as_u64().unwrap() > 0);
    let layers = budget["layers"].as_array().expect("layers");
    for name in ["configured", "workspace", "request"] {
        assert!(
            layers.iter().any(|l| l["name"] == name),
            "{name} missing: {budget}"
        );
    }
}

/// A pasted request leaves less room: the evidence shares what is left,
/// the smaller layer whole, and the workspace envelope is rebuilt within
/// its share (excerpts first, as at the 128 KB limit), so the turn stays
/// within the budget with the request whole.
#[test]
fn a_long_request_cuts_the_envelope_to_its_share() {
    let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
    let scratch = tempfile::tempdir().unwrap();
    let vault = dir.path();
    let key = "s-20261007-g2000005";
    let cwd = heavy_vault(vault, key);
    let chat = Chat {
        agent: AgentKind::Codex,
        vault,
        cwd,
        key,
        scratch: scratch.path().to_path_buf(),
        baseline: true,
    };
    let pasted = lines("Pasted", 500);
    let request = format!("Check these notes against the churn sources:\n{pasted}");
    let sent = chat.turn(
        &request,
        Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
        "Done.",
    );
    let budget = sent.record.budget.clone().unwrap();
    let layer = |name: &str| budget.layers.iter().find(|l| l.name == name).unwrap();
    assert!(sent.bytes.total() <= TOTAL_BUDGET, "{}", sent.bytes.total());
    assert!(sent.prompt.contains(&pasted), "the request is whole");
    let workspace = layer("workspace");
    assert!(workspace.granted < workspace.wanted, "{budget:?}");
    assert!(sent.bytes.workspace as u64 <= workspace.granted);
    assert_eq!(
        layer("conversationContext").granted,
        layer("conversationContext").wanted,
        "the smaller evidence layer stays whole"
    );
    assert!(
        sent.prompt.contains(crate::workspaces::CUT_NOTE)
            || sent.prompt.contains("this turn's context budget"),
        "the envelope says what it left out"
    );
    for kept in ["constraint cobalt-wren", "decision quartz-heron"] {
        assert!(sent.prompt.contains(kept), "{kept}");
    }
}

/// G2 review: prose the budget cut on a fresh turn was never given, and
/// it is not news either. A resumed turn with no `me/` edit carries no
/// `<operator-update>` (before the fix, every cut prose line came back as
/// "added"); a real edit afterwards is still reported, and only it.
#[test]
fn prose_the_budget_cut_does_not_return_as_a_profile_change() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let key = "s-20261007-g2000006";
        let cwd = heavy_vault(vault, key);
        let chat = Chat {
            agent,
            vault,
            cwd,
            key,
            scratch: scratch.path().to_path_buf(),
            baseline: true,
        };
        let name = agent.as_str();
        let fresh = chat.turn(
            "Compare churn and retention across the cohorts in a chart.",
            Session::Reset(crate::chat_resume::ResetReason::ConfigChanged),
            "Done.",
        );
        let budget = fresh.record.budget.clone().expect("budget");
        let prose = budget
            .layers
            .iter()
            .find(|l| l.name == "profileProse")
            .unwrap();
        assert!(prose.granted < prose.wanted, "{name}: the prose was cut");

        let resumed = chat.turn("Next section, please.", Session::Resume, "Next.");
        assert!(
            !resumed
                .prompt
                .contains(crate::context_compiler::UPDATE_OPEN),
            "{name}: an unedited profile was reported changed"
        );
        let delta = &resumed.record.profile_delta;
        assert_eq!((delta.added, delta.removed), (0, 0), "{name}: {delta:?}");

        // A real edit is still news, and only it.
        let decisions = vault.join("me/Decisions.md");
        let mut text = std::fs::read_to_string(&decisions).unwrap();
        text.push_str("- Retention is reported weekly (decision slate-owl).\n");
        std::fs::write(&decisions, text).unwrap();
        let edited = chat.turn("And the next one.", Session::Resume, "Next.");
        let delta = &edited.record.profile_delta;
        assert_eq!((delta.added, delta.removed), (1, 0), "{name}: {delta:?}");
        assert_eq!(delta.files, vec!["me/Decisions.md".to_string()], "{name}");
        assert!(edited.prompt.contains("+ - Retention is reported weekly"));
    }
}

/// G2 review (red first): a resumed turn the agent never finished (a usage
/// limit, a crash; nothing announced) leaves the stable-layer baseline as
/// it was, so the next resumed turn still carries the plan edited before
/// it. Before the fix the failed turn's prompt was recorded as held and
/// the next turn marked the edited plan unchanged.
#[test]
fn a_failed_resumed_turn_leaves_the_baseline_as_it_was() {
    for agent in AGENTS {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let scratch = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let cwd = workspace_vault(vault);
        let chat = Chat {
            agent,
            vault,
            cwd: cwd.clone(),
            key: "s-20261007-g2000007",
            scratch: scratch.path().to_path_buf(),
            baseline: true,
        };
        let name = agent.as_str();
        chat.turn("Summarize the churn sources.", Session::New, "Churn rose.");
        let before = crate::stable_layers::read(vault, chat.key).expect("baseline");
        let plan = cwd.join("PLAN.md");
        let mut text = std::fs::read_to_string(&plan).unwrap();
        text.push_str("- Ship the readout on Friday (plan teal-lynx).\n");
        std::fs::write(&plan, text).unwrap();
        let failed = chat.turn_ending("And retention?", Session::Resume, "", false, None);
        assert!(
            failed.prompt.contains("plan teal-lynx"),
            "{name}: the failed turn carried the edit"
        );
        assert_eq!(
            crate::stable_layers::read(vault, chat.key).expect("baseline"),
            before,
            "{name}: the failed turn recorded nothing"
        );
        let next = chat.turn("And retention?", Session::Resume, "It held.");
        assert!(
            next.prompt.contains("plan teal-lynx"),
            "{name}: the edit the agent never got is sent again"
        );
        let stable = next.record.stable_layers.clone().unwrap();
        assert_eq!(stable.workspace, "delta", "{name}: {stable:?}");
        assert!(stable.changed >= 1, "{name}: {stable:?}");
    }
}
