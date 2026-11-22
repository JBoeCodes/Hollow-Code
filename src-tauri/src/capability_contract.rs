//! The per-provider capability contract (agent effectiveness F9, wave 4 ·
//! V1): what each agent CLI gets from Hollow at runtime, as one table.
//!
//! This table is the source of truth for three readers, and tests pin all
//! three to it:
//! - `docs/AGENT_CONTEXT.md` "Tools per provider" holds [`markdown_table`]
//!   verbatim between its `capability-contract` markers
//!   (`the_doc_holds_the_contract_table`);
//! - each chat's capability index ends with [`guidance_line`] for its agent:
//!   what this agent doesn't have here and the fallback
//!   (`chat::capability_index`);
//! - the cells that restate a runtime decision are checked against the code
//!   that makes it (`agent::capabilities`, `agent::unavailable_mode`,
//!   `codex_probe::VERIFIED`), so the table can't claim more than a run gets.
//!
//! Change a capability, then its row here; the doc and the guidance follow
//! (the guidance is in the appended prompt, which fingerprint v3 doesn't
//! hash, so a change reaches resumed sessions; bump
//! `chat_resume::PROMPT_CONTRACT` when one must start them fresh, OB04).
use crate::agent::AgentKind;

/// The providers, in the table's column order.
pub(crate) const PROVIDERS: [AgentKind; 4] = [
    AgentKind::Claude,
    AgentKind::Codex,
    AgentKind::Antigravity,
    AgentKind::Grok,
];

/// Whether a provider has a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Has {
    Yes,
    /// Some of it, or a substitute (the cell says which).
    Partial,
    No,
}

/// One provider's cell.
#[derive(Debug, Clone)]
pub(crate) struct Cell {
    #[cfg_attr(not(test), allow(dead_code))]
    pub has: Has,
    /// The table's text (Markdown).
    #[cfg_attr(not(test), allow(dead_code))]
    pub text: String,
    /// What an agent of this provider is told about this row in its
    /// capability index: the gap and the fallback. `None`: nothing to say.
    pub guidance: Option<&'static str>,
}

/// One capability, across the providers.
#[derive(Debug, Clone)]
pub(crate) struct Row {
    #[cfg_attr(not(test), allow(dead_code))]
    pub key: &'static str,
    #[cfg_attr(not(test), allow(dead_code))]
    pub label: &'static str,
    /// In [`PROVIDERS`] order.
    pub cells: [Cell; 4],
}

impl Row {
    pub(crate) fn cell(&self, kind: AgentKind) -> &Cell {
        &self.cells[PROVIDERS.iter().position(|k| *k == kind).unwrap_or(0)]
    }
}

fn cell(has: Has, text: impl Into<String>) -> Cell {
    Cell {
        has,
        text: text.into(),
        guidance: None,
    }
}

fn told(has: Has, text: impl Into<String>, guidance: &'static str) -> Cell {
    Cell {
        has,
        text: text.into(),
        guidance: Some(guidance),
    }
}

/// What Antigravity and Grok are told about Hollow's own tools.
/// CC2: the code-context tools reach them through the CLI twin instead.
const NO_OWNED: &str = "Hollow can't attach its own tools here (no app, tasks, vault search, read_source, computer or teammates): use your own file tools in the folders named above; ask the person for the rest. Callers and impact in a git checkout: `hollow code-context <tool> '<json>'` (Hollow's own binary; `hollow code-context tools` lists them).";
/// What Antigravity and Grok are told about shared connectors.
const NO_CONNECTORS: &str = "No shared connectors either: for an external service, tell the person which connector it needs (Claude Code and Codex can take one).";

/// The contract, row by row (`docs/AGENT_CONTEXT.md` order).
pub(crate) fn contract() -> Vec<Row> {
    use Has::{No, Partial, Yes};
    let verified = crate::codex_probe::verified_list();
    vec![
        Row {
            key: "ownedMcp",
            label: "Owned MCP (`hollow_questions`, `hollow_tasks`, `hollow_app`, `hollow_vault`, `hollow_code_context`, `hollow_computer`, `hollow_team`, `hollow_services`), per surface as above",
            cells: [
                cell(Yes, "yes: private `--mcp-config` file, `--strict-mcp-config`"),
                cell(Yes, "yes: each stdio server through `hollow mcp-launch`, `-c mcp_servers` overrides"),
                cell(Yes, "yes in a chat, over `agy acp` (`session/new` `mcpServers`, per chat, never agy's own config); a one-shot runs `agy -p` and gets none"),
                told(No, "no (no per-run MCP flag, 1.0.46)", NO_OWNED),
            ],
        },
        Row {
            key: "preapproval",
            label: "Preapproval of owned tools",
            cells: [
                cell(Yes, "`--allowed-tools mcp__<server>` (`hollow_computer`, `hollow_services` only in auto and bypass)"),
                cell(Yes, "`default_tools_approval_mode=\"approve\"`, same rule; `hollow_app` gets a longer tool timeout"),
                cell(Yes, "none needed: each call asks (an approval card in a chat); bypass starts agy with `--dangerously-skip-permissions`"),
                cell(No, "n/a"),
            ],
        },
        Row {
            key: "appTools",
            label: "Hollow app tools (`hollow_app`: workspaces, sessions, automations, `list_actions` / `run_action`)",
            cells: [
                cell(Yes, "yes, while Settings → Connectors → Built into Hollow → Hollow app is on (read-only in plan)"),
                cell(Yes, "same"),
                cell(Yes, "yes in a chat, as Claude"),
                cell(No, "no: ask the person, or use its own file tools"),
            ],
        },
        Row {
            key: "reader",
            label: "On-demand reader for attached and retrieved files",
            cells: [
                cell(
                    Yes,
                    "`hollow_vault` `read_source` (while Ask your vault is on): by range or by `query`, and the run's scratch folder too",
                ),
                cell(Yes, "same"),
                cell(Yes, "same, in a chat"),
                cell(No, "no: the envelope names the workspace folder for its own file tools"),
            ],
        },
        Row {
            key: "connectors",
            label: "Library connectors",
            cells: [
                cell(Yes, "yes, claude.ai ones included"),
                cell(Yes, "yes, not claude.ai"),
                cell(Yes, "yes in a chat, not claude.ai; the person shares each connector with Antigravity"),
                told(No, "no", NO_CONNECTORS),
            ],
        },
        Row {
            key: "native",
            label: "The CLI's own MCP servers, apps and plugins",
            cells: [
                told(Partial, "its MCP servers excluded (strict config); its own skills and plugins are Claude Code's, not managed or verified by Hollow", "Your own MCP servers are off in this run: use Hollow's tools above, or ask the person."),
                told(Partial, "each configured MCP server disabled (named by `codex mcp list`, cached per binary, config and folder); `apps`, `plugins`, `remote_plugin` disabled", "Your own MCP servers, apps and plugins are off in this run: use Hollow's tools above, or ask the person."),
                told(Partial, "its own global configuration still loads, not managed by Hollow; Hollow's servers are per chat and never written there", "Your own agy MCP servers still load but Hollow doesn't manage them: use Hollow's tools above, or ask the person."),
                cell(Yes, "`~/.grok` servers still load"),
            ],
        },
        Row {
            key: "questions",
            label: "Question cards",
            cells: [
                cell(Yes, "`hollow_questions` in chats"),
                cell(Yes, "`hollow_questions` in chats"),
                cell(Yes, "`hollow_questions` in chats"),
                cell(Partial, "a fenced `hollow-questions` block at the end of a reply"),
            ],
        },
        Row {
            key: "confirmCards",
            label: "Hollow confirm cards (app, computer)",
            cells: [
                cell(Yes, "per mode"),
                cell(Yes, "per mode"),
                cell(Yes, "per mode, and an approval card for each request agy itself asks"),
                cell(No, "none"),
            ],
        },
        Row {
            key: "skills",
            label: "Skills",
            cells: [
                cell(Partial, "a turn's `<hollow-skills>` block lists up to 5 skills from the vault's `.agents/skills` and `~/.agents/skills` that fit its request (a skill chosen with \"Use in workspace\" first, its text inline up to 8 KB; `skill_index.rs`), and `hollow_vault` `read_skill` reads one whole (else `hollow_app` `skills.read_skill`). Claude Code may also load its own `~/.claude/skills` and plugin skills (not verified per run)"),
                cell(Partial, "as Claude; Codex may also load its own `~/.codex/skills` (not verified per run); plugin skills don't load (plugins off)"),
                cell(Partial, "the same block, so a selected skill's text arrives inline when small; any other it opens at the listed path with its own file tools, which its mode may refuse outside the working folder (then the person pastes it)"),
                cell(Partial, "same as Antigravity"),
            ],
        },
        Row {
            key: "visualGuide",
            label: "Visual guide on demand (G1)",
            cells: [
                cell(Yes, "`$HOLLOW_SCRATCH_DIR/hollow-visual-guide.md`, an `--add-dir` folder"),
                cell(Yes, "the same path (`HOLLOW_SCRATCH_DIR`), read with its own tools"),
                cell(Yes, "`--add-dir` folder, as Claude"),
                cell(Yes, "the same path, as Codex"),
            ],
        },
        Row {
            key: "toolRestrictions",
            label: "Workflow tool restrictions",
            cells: [
                cell(Yes, "yes"),
                cell(No, "refused"),
                cell(No, "refused"),
                cell(No, "refused (only the `osascript`/`open` pair, as `--deny`)"),
            ],
        },
        Row {
            key: "toolFree",
            label: "Tool-free mode (one-shot helpers)",
            cells: [
                cell(Yes, "`--tools \"\" --disable-slash-commands` (`claude_runner::NO_TOOLS`)"),
                cell(Partial, format!("`codex_transform.rs` on a verified codex-cli ({verified}), its version read once per binary (`codex_probe`); any other version doesn't run and says why (Settings → Models, the usage row's note)")),
                cell(No, "none verified"),
                cell(No, "none verified"),
            ],
        },
        Row {
            key: "heartbeat",
            label: "Heartbeat",
            cells: [
                cell(Yes, "tool pass"),
                cell(Partial, "tool-free snapshot"),
                cell(No, "refused"),
                cell(No, "refused"),
            ],
        },
        Row {
            key: "modes",
            label: "Permission modes",
            cells: [
                cell(Yes, "all"),
                cell(Yes, "all"),
                cell(Yes, "all; its own reviewer in auto"),
                cell(Yes, "all; its sandbox is the limit"),
            ],
        },
        Row {
            key: "readRoots",
            label: "Reads outside its folders in default and accept edits (OB02)",
            cells: [
                cell(No, "refused, not asked (headless `-p`): only the cwd and its `--add-dir` folders (scratch, a build workspace's vault folder), which the turn's `<hollow-mode>` line names"),
                cell(Yes, "yes: its sandbox limits writes, not reads"),
                cell(Partial, "not verified; its mode may refuse outside the working folder"),
                cell(Partial, "not verified; its sandbox is the limit"),
            ],
        },
        Row {
            key: "delegates",
            label: "Delegates through `hollow_team` (bounded consultation)",
            cells: [
                cell(Yes, "yes (not in plan)"),
                cell(
                    Yes,
                    "yes (not in plan); the tool waits 240 s (D1; was Codex's 60 s default)",
                ),
                cell(Yes, "yes in a chat (not in plan)"),
                cell(No, "no"),
            ],
        },
        Row {
            key: "childTasks",
            label: "Starts durable child tasks (`hollow_app` `start_session`)",
            cells: [
                cell(Yes, "yes (writable app tools, depth < 2)"),
                cell(Yes, "yes (same)"),
                cell(Yes, "yes in a chat (same)"),
                cell(No, "no"),
            ],
        },
        Row {
            key: "waits",
            label: "Waits for a child task (`hollow_app` `wait_for_session`, D2)",
            cells: [
                cell(Yes, "yes (≤ 300 s; no shorter client timeout)"),
                cell(
                    Yes,
                    "yes (≤ 300 s; the server waits 315 s, inside its 480 s tool timeout)",
                ),
                cell(Yes, "yes in a chat (same)"),
                cell(No, "no"),
            ],
        },
        Row {
            key: "budget",
            label: "Budget metering mid-run",
            cells: [
                cell(Yes, "yes"),
                cell(No, "no"),
                cell(No, "no"),
                cell(No, "no"),
            ],
        },
    ]
}

/// The contract as the Markdown table `docs/AGENT_CONTEXT.md` holds.
#[cfg(test)]
pub(crate) fn markdown_table() -> String {
    let mut out =
        String::from("| | Claude | Codex | Antigravity | Grok |\n|---|---|---|---|---|\n");
    for row in contract() {
        out.push_str("| ");
        out.push_str(row.label);
        for cell in &row.cells {
            out.push_str(" | ");
            out.push_str(&cell.text);
        }
        out.push_str(" |\n");
    }
    out
}

/// The capability index's line for `kind` (F9): what this agent doesn't
/// have in Hollow and what to do instead, from the cells' guidance in
/// row order.
pub(crate) fn guidance_line(kind: AgentKind) -> String {
    let told: Vec<&str> = contract()
        .iter()
        .filter_map(|row| row.cell(kind).guidance)
        .collect();
    if told.is_empty() {
        return String::new();
    }
    format!("- This agent ({}): {}", kind.label(), told.join(" "))
}

/// Whether a `kind` run's reads stop at its folders in default and accept
/// edits (the `readRoots` row says no reads elsewhere), so its turn's
/// `<hollow-mode>` line names them (OB02, `mode_contract::Reads`).
pub(crate) fn reads_only_its_roots(kind: AgentKind) -> bool {
    contract()
        .iter()
        .find(|row| row.key == "readRoots")
        .is_some_and(|row| row.cell(kind).has == Has::No)
}

/// The markers around the table in `docs/AGENT_CONTEXT.md`.
#[cfg(test)]
pub(crate) const DOC_BEGIN: &str = "<!-- capability-contract:begin (generated from src-tauri/src/capability_contract.rs; edit the table there) -->";
#[cfg(test)]
pub(crate) const DOC_END: &str = "<!-- capability-contract:end -->";

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str) -> Row {
        contract()
            .into_iter()
            .find(|r| r.key == key)
            .unwrap_or_else(|| panic!("row {key}"))
    }

    /// The doc's table is this table, verbatim. After changing a row, from
    /// `src-tauri/` run `HOLLOW_REGENERATE_CONTRACT=1 cargo test --lib
    /// capability_contract` to rewrite the block.
    #[test]
    fn the_doc_holds_the_contract_table() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs/AGENT_CONTEXT.md");
        let doc = std::fs::read_to_string(&path).unwrap();
        let start = doc.find(DOC_BEGIN).expect("begin marker") + DOC_BEGIN.len();
        let end = doc.find(DOC_END).expect("end marker");
        let want = format!("\n{}", markdown_table());
        if std::env::var_os("HOLLOW_REGENERATE_CONTRACT").is_some() {
            let mut rewritten = doc[..start].to_string();
            rewritten.push_str(&want);
            rewritten.push_str(&doc[end..]);
            std::fs::write(&path, rewritten).unwrap();
            return;
        }
        assert_eq!(
            &doc[start..end],
            want,
            "docs/AGENT_CONTEXT.md's contract table is stale; from src-tauri/ run HOLLOW_REGENERATE_CONTRACT=1 cargo test --lib capability_contract"
        );
    }

    /// The cells that restate a runtime decision agree with the code that
    /// makes it.
    #[test]
    fn the_table_matches_what_a_run_gets() {
        for kind in PROVIDERS {
            let caps = crate::agent::capabilities(kind);
            let owned = caps.owned_mcp;
            for key in [
                "ownedMcp",
                "preapproval",
                "appTools",
                "reader",
                "connectors",
                "delegates",
                "childTasks",
                "waits",
            ] {
                assert_eq!(row(key).cell(kind).has == Has::Yes, owned, "{key} {kind:?}");
            }
            assert_eq!(
                row("questions").cell(kind).has == Has::Yes,
                owned,
                "owned MCP: cards; else the fenced block"
            );
            assert_eq!(
                row("toolRestrictions").cell(kind).has == Has::Yes,
                caps.tool_restrictions
            );
            assert_eq!(
                row("modes").cell(kind).text.contains("no `auto`"),
                crate::agent::unavailable_mode(kind, "auto").is_some()
            );
            for perm in ["plan", "default", "acceptEdits", "bypassPermissions"] {
                assert!(crate::agent::unavailable_mode(kind, perm).is_none());
            }
            // An agent without owned MCP gets no library connector; one
            // with it gets an HTTP connector offered to every agent.
            let shared: crate::connectors::Connector = serde_json::from_value(serde_json::json!({
                "id": "notes", "name": "Notes", "transport": "http",
                "url": "https://example.invalid/mcp",
                "agents": ["claude", "codex", "antigravity", "grok"],
            }))
            .unwrap();
            assert_eq!(
                !crate::connectors::eligible(&[shared], kind, "acceptEdits", false).is_empty(),
                owned,
                "{kind:?}"
            );
        }
        // The delegation rows restate D1's limits (F10).
        assert!(row("delegates")
            .cell(AgentKind::Codex)
            .text
            .contains(&format!("{} s", crate::team_mcp::CODEX_TOOL_TIMEOUT_SECS)));
        assert!(row("childTasks")
            .cell(AgentKind::Claude)
            .text
            .contains(&format!("depth < {}", crate::app_mcp::MAX_START_DEPTH)));
        let waits = row("waits").cell(AgentKind::Codex).text.clone();
        for secs in [
            crate::delegation::WAIT_MAX_SECONDS,
            crate::delegation::WAIT_ANSWER_SECONDS,
            crate::app_mcp::CODEX_TOOL_TIMEOUT_SECS,
        ] {
            assert!(waits.contains(&format!("{secs} s")), "{secs}: {waits}");
        }
        let tool_free = row("toolFree").cell(AgentKind::Codex).text.clone();
        for version in crate::codex_probe::VERIFIED {
            assert!(
                tool_free.contains(version.trim_start_matches("codex-cli ")),
                "{version}"
            );
        }
    }

    /// OB02: only Claude's reads stop at its folders, so only its turn line
    /// names them; the row adds nothing to the guidance line, whose bytes
    /// are in the fingerprinted appended prompt.
    #[test]
    fn only_claudes_reads_stop_at_its_roots() {
        let limited: Vec<AgentKind> = PROVIDERS
            .into_iter()
            .filter(|kind| reads_only_its_roots(*kind))
            .collect();
        assert_eq!(limited, vec![AgentKind::Claude]);
        assert!(row("readRoots").cells.iter().all(|c| c.guidance.is_none()));
    }

    /// Every provider is told every gap that has a fallback, and only
    /// those: the guidance line is the cells' guidance, in row order, and
    /// an agent without owned MCP hears how to work without it.
    #[test]
    fn guidance_comes_from_the_table() {
        for kind in PROVIDERS {
            let line = guidance_line(kind);
            let expected: Vec<&str> = contract()
                .iter()
                .filter_map(|r| r.cell(kind).guidance)
                .collect();
            assert_eq!(
                line,
                format!("- This agent ({}): {}", kind.label(), expected.join(" "))
            );
            // 400 until CC2 (2026-11-18) named the code-context CLI twin
            // for the agents without owned MCP (Antigravity: 486).
            assert!(line.len() <= 500, "{kind:?}: {} bytes", line.len());
            for row in contract() {
                let cell = row.cell(kind);
                if cell.guidance.is_some() {
                    assert_ne!(
                        cell.has,
                        Has::Yes,
                        "{} {kind:?}: guidance is for a gap",
                        row.key
                    );
                }
            }
            let owned = crate::agent::capabilities(kind).owned_mcp;
            assert_eq!(line.contains("can't attach its own tools"), !owned);
            assert_eq!(line.contains("No shared connectors"), !owned);
            assert!(line.contains("ask the person"), "{kind:?}: a fallback");
        }
    }

    /// The skill actions say how a skill reaches an agent and no more: no
    /// agent loads Hollow's shared skills by itself (the Skills row).
    #[test]
    fn skill_actions_describe_delivery_truthfully() {
        let write = crate::app_actions::find("skills.write_user_skill").unwrap();
        assert!(
            !write.summary.contains("every agent loads"),
            "{}",
            write.summary
        );
        assert!(write.summary.contains("No agent loads it by itself"));
        assert!(write.summary.contains("skills.read_skill"));
        // The Skills row restates the index's bounds (wave 4 · V2).
        let claude = row("skills").cell(AgentKind::Claude).text.clone();
        assert!(claude.contains(&format!("up to {} skills", crate::skill_index::INDEX_MAX)));
        assert!(claude.contains(&format!(
            "inline up to {} KB",
            crate::skill_index::INLINE_BUDGET_BYTES / 1024
        )));
        assert!(claude.contains("`read_skill`"));
        for kind in PROVIDERS {
            let skills = row("skills");
            let text = &skills.cell(kind).text;
            assert_eq!(skills.cell(kind).has, Has::Partial, "{kind:?}");
            assert!(
                !text.contains("loads every") && !text.contains("loads them"),
                "{text}"
            );
        }
    }

    #[test]
    fn the_table_has_one_cell_per_provider_and_no_stray_pipes() {
        let table = markdown_table();
        for line in table.lines() {
            assert_eq!(line.matches('|').count(), 6, "{line}");
        }
        let rows = contract();
        let mut keys: Vec<_> = rows.iter().map(|r| r.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), rows.len(), "unique keys");
    }
}
