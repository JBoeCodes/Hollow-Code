//! Skill portability per agent (NV3, 2026-11-17): for one skill, whether each
//! agent loads it, loads it degraded or does not load it, with the rule ids
//! and the CLI version each rule was checked against.
//!
//! One table, [`PROFILES`], names the agents: a const slice keyed by
//! [`AgentKind`], one profile each, and nowhere else in this file branches on
//! an agent. A profile is a list of [`Rule`]s. A rule has an id, a [`Check`]
//! (a pure test of the skill's cheaply known [`SkillFacts`]), the [`Effect`]
//! when the check hits, its [`Evidence`] and two plain sentences.
//!
//! **Two channels.** A skill reaches an agent two ways, and a rule belongs to
//! one: [`Channel::Hollow`] is what Hollow itself delivers (the per-turn
//! `<hollow-skills>` index, the inline text, `read_skill`; `skill_index.rs`),
//! and [`Channel::Native`] is whatever the CLI's own loader does with its own
//! skill folders. The agent's summary effect is the worst effect of the
//! Hollow channel only: a native rule that is unknown must not make a skill
//! look broken, and it must not make it look fine either, so native findings
//! are listed beside the summary, never folded into it.
//!
//! **Evidence, and no invented rules.** [`Evidence::Hollow`] is a fact about
//! Hollow's own code, tested in this repo, so it has no CLI version.
//! [`Evidence::Cli`] names the CLI version a fact was checked on (the only two
//! today are in the capability table of `docs/reference/agents.md`: Grok
//! 1.0.46 and Antigravity 1.2.15 take no per-run MCP, so they open a skill
//! with their own file tools). [`Evidence::None`] says why nothing is
//! verified. Whether a Grok or Antigravity skill loader exists is not
//! verified anywhere in this repo, so those two have an `Unknown` native
//! rule and nothing else; no profile guesses a cap a CLI was never seen to
//! apply. A rule's version goes stale when the CLI updates:
//! [`version_status`] compares it with the installed version (read once per
//! binary by `agent_cli::steering_version`, outside every lock) and says "not
//! verified on this version" when they differ. Read-only: nothing here runs
//! an agent, writes a file or sends anything.

use crate::agent::AgentKind;
use crate::skill_index::{DESCRIPTION_CHARS, INLINE_BUDGET_BYTES, READ_MAX_BYTES};
use serde::Serialize;

/// The frontmatter bytes `skill_index` reads for the index.
pub(crate) const INDEX_FRONTMATTER_BYTES: usize = crate::skill_index::META_READ_BYTES as usize;

/// What a skill does for an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    /// Reaches the agent whole.
    Loads,
    /// Not checked, so no claim either way.
    Unknown,
    /// Reaches the agent, with something cut or one step further away.
    Degraded,
    /// Does not reach the agent this way. No rule is verified to say this
    /// yet (the Codex plugin rule is `Unknown`), but the wire and the page know it.
    #[allow(dead_code)]
    NoLoad,
}

/// How a skill reaches an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Channel {
    /// Hollow's own delivery (the index, the inline text, `read_skill`).
    Hollow,
    /// The CLI's own skill loader.
    Native,
}

/// What a rule rests on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Hollow's own code, tested here: no CLI version applies.
    Hollow,
    /// Checked against this CLI version.
    Cli(&'static str),
    /// Not verified; the reason.
    None(&'static str),
}

/// A test of a skill's facts. Pure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Check {
    /// Every skill.
    Always,
    /// The (whitespace-flattened) description is longer than this many characters.
    DescriptionOver(usize),
    /// The frontmatter name is one the index can't use.
    NameUnfit,
    /// The frontmatter runs past this many bytes.
    FrontmatterOver(usize),
    /// `SKILL.md` is larger than this many bytes.
    FileOver(usize),
    /// A copy outside the shared library (the index lists only that and the vault).
    NotLibrary,
    /// A plugin or marketplace skill of this provider.
    PluginOf(&'static str),
}

/// One rule of a profile.
#[derive(Debug, Clone, Copy)]
pub struct Rule {
    pub id: &'static str,
    pub channel: Channel,
    pub check: Check,
    /// The effect when the check hits.
    pub effect: Effect,
    pub evidence: Evidence,
    /// A few words: what the rule is about.
    pub what: &'static str,
    /// One plain sentence, shown when the rule hits.
    pub sentence: &'static str,
}

/// One agent's rules.
#[derive(Debug, Clone, Copy)]
pub struct Profile {
    pub kind: AgentKind,
    pub rules: &'static [Rule],
}

// ── Hollow's delivery, the same for every agent ─────────────────────────────

const DESC_CAP_160: Rule = Rule {
    id: "desc-cap-160",
    channel: Channel::Hollow,
    check: Check::DescriptionOver(DESCRIPTION_CHARS),
    effect: Effect::Degraded,
    evidence: Evidence::Hollow,
    what: "Index description cut at 160 characters",
    sentence: "The turn's skill index cuts the description to 160 characters, so the agent sees only its start.",
};
const NAME_UNFIT: Rule = Rule {
    id: "name-unfit",
    channel: Channel::Hollow,
    check: Check::NameUnfit,
    effect: Effect::Degraded,
    evidence: Evidence::Hollow,
    what: "Index name",
    sentence: "The skill's name is over 64 characters or holds a quote, slash, angle bracket or control character, so the index lists it by its folder name.",
};
const FRONTMATTER_16K: Rule = Rule {
    id: "frontmatter-16k",
    channel: Channel::Hollow,
    check: Check::FrontmatterOver(INDEX_FRONTMATTER_BYTES),
    effect: Effect::Degraded,
    evidence: Evidence::Hollow,
    what: "Index reads the first 16 KB",
    sentence: "Hollow reads only the first 16 KB of SKILL.md to build the index, so a name or description past it is missed.",
};
const NOT_LIBRARY: Rule = Rule {
    id: "not-indexed-copy",
    channel: Channel::Hollow,
    check: Check::NotLibrary,
    effect: Effect::Degraded,
    evidence: Evidence::Hollow,
    what: "Index lists only the library",
    sentence: "Hollow indexes only the shared library and the vault, so this copy reaches an agent only when selected with Use in workspace, and then its text goes inline only while it fits the 8 KB budget shared by the selected skills; otherwise the agent reads it by path.",
};

// ── Whole text of a large skill: how each agent gets it ─────────────────────

/// Claude and Codex have the owned `hollow_vault` `read_skill`.
const INLINE_8K_READ_SKILL: Rule = Rule {
    id: "inline-8k-read-skill",
    channel: Channel::Hollow,
    check: Check::FileOver(INLINE_BUDGET_BYTES),
    effect: Effect::Loads,
    evidence: Evidence::Hollow,
    what: "Over 8 KB: read on demand",
    sentence: "Over 8 KB the skill's text does not go inline when selected; the agent reads it whole through its attached skills reader (read_skill, else the app's skills.read_skill) or with its own file tools. read_skill reaches only the vault and the shared library, so a provider or plugin copy is read by path.",
};
/// Grok takes no per-run MCP (1.0.46), so it opens a large skill by path.
const INLINE_8K_OWN_TOOLS_GROK: Rule = Rule {
    id: "inline-8k-own-tools",
    channel: Channel::Hollow,
    check: Check::FileOver(INLINE_BUDGET_BYTES),
    effect: Effect::Degraded,
    evidence: Evidence::Cli("1.0.46"),
    what: "Over 8 KB: opened by path",
    sentence: "Over 8 KB the skill's text does not go inline; this agent has no read_skill and opens it by path with its own file tools, which its mode may refuse outside the working folder.",
};
/// Antigravity takes no per-run MCP (agy 1.2.15), likewise.
const INLINE_8K_OWN_TOOLS_AGY: Rule = Rule {
    id: "inline-8k-own-tools",
    channel: Channel::Hollow,
    check: Check::FileOver(INLINE_BUDGET_BYTES),
    effect: Effect::Degraded,
    evidence: Evidence::Cli("1.2.15"),
    what: "Over 8 KB: opened by path",
    sentence: "Over 8 KB the skill's text does not go inline; this agent has no read_skill and opens it by path with its own file tools, which its mode may refuse outside the working folder.",
};
const READ_CAP_64K: Rule = Rule {
    id: "read-cap-64k",
    channel: Channel::Hollow,
    check: Check::FileOver(READ_MAX_BYTES),
    effect: Effect::Degraded,
    evidence: Evidence::Hollow,
    what: "read_skill returns 64 KB",
    sentence: "read_skill returns at most 64 KB, so the end of this file is cut.",
};

// ── The CLI's own loader ────────────────────────────────────────────────────

const NATIVE_CLAUDE: Rule = Rule {
    id: "native-claude",
    channel: Channel::Native,
    check: Check::Always,
    effect: Effect::Unknown,
    evidence: Evidence::None(
        "the capability contract says Claude Code may load its own ~/.claude/skills and plugin skills, not verified per run",
    ),
    what: "Claude Code's own loader",
    sentence: "Claude Code may also load this skill from its own skills folder or a plugin; Hollow has not verified that for this version.",
};
const NATIVE_CODEX: Rule = Rule {
    id: "native-codex",
    channel: Channel::Native,
    check: Check::Always,
    effect: Effect::Unknown,
    evidence: Evidence::None(
        "the capability contract says Codex may load its own ~/.codex/skills, not verified per run",
    ),
    what: "Codex's own loader",
    sentence: "Codex may also load this skill from ~/.codex/skills; Hollow has not verified that for this version.",
};
const NATIVE_CODEX_PLUGIN: Rule = Rule {
    id: "native-codex-plugin-off",
    channel: Channel::Native,
    check: Check::PluginOf("codex"),
    effect: Effect::Unknown,
    evidence: Evidence::None(
        "Hollow passes --disable plugins to Codex, but what that does to a plugin's skills is not verified on any Codex version",
    ),
    what: "Codex plugins are off",
    sentence: "Hollow starts Codex with plugins disabled, so a Codex plugin's skills are expected not to load by themselves; that is not verified on a Codex version.",
};
const NATIVE_GROK: Rule = Rule {
    id: "native-grok",
    channel: Channel::Native,
    check: Check::Always,
    effect: Effect::Unknown,
    evidence: Evidence::None("no native skill loader for Grok is recorded or verified in this repo"),
    what: "Grok's own loader",
    sentence: "Whether Grok loads skills by itself is not verified; Hollow's index and file tools are the only route known.",
};
const NATIVE_ANTIGRAVITY: Rule = Rule {
    id: "native-antigravity",
    channel: Channel::Native,
    check: Check::Always,
    effect: Effect::Unknown,
    evidence: Evidence::None(
        "no native skill loader for Antigravity is recorded or verified in this repo",
    ),
    what: "Antigravity's own loader",
    sentence: "Whether Antigravity loads skills by itself is not verified; Hollow's index and file tools are the only route known.",
};

/// The one table: the only place this file names an agent.
pub const PROFILES: &[Profile] = &[
    Profile {
        kind: AgentKind::Claude,
        rules: &[
            DESC_CAP_160,
            NAME_UNFIT,
            FRONTMATTER_16K,
            NOT_LIBRARY,
            INLINE_8K_READ_SKILL,
            READ_CAP_64K,
            NATIVE_CLAUDE,
        ],
    },
    Profile {
        kind: AgentKind::Codex,
        rules: &[
            DESC_CAP_160,
            NAME_UNFIT,
            FRONTMATTER_16K,
            NOT_LIBRARY,
            INLINE_8K_READ_SKILL,
            READ_CAP_64K,
            NATIVE_CODEX,
            NATIVE_CODEX_PLUGIN,
        ],
    },
    Profile {
        kind: AgentKind::Grok,
        rules: &[
            DESC_CAP_160,
            NAME_UNFIT,
            FRONTMATTER_16K,
            NOT_LIBRARY,
            INLINE_8K_OWN_TOOLS_GROK,
            NATIVE_GROK,
        ],
    },
    Profile {
        kind: AgentKind::Antigravity,
        rules: &[
            DESC_CAP_160,
            NAME_UNFIT,
            FRONTMATTER_16K,
            NOT_LIBRARY,
            INLINE_8K_OWN_TOOLS_AGY,
            NATIVE_ANTIGRAVITY,
        ],
    },
];

/// The profile of `kind`.
pub fn profile(kind: AgentKind) -> Option<&'static Profile> {
    PROFILES.iter().find(|p| p.kind == kind)
}

// ── What is known about a skill ─────────────────────────────────────────────

/// Where a copy of a skill lives, from `skills::Skill::source`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Origin {
    /// The shared library (`~/.agents/skills`).
    pub library: bool,
    /// A plugin or marketplace skill, and its provider.
    pub plugin_of: Option<String>,
}

impl Origin {
    /// `shared`, `provider:<p>`, `plugin:<p>/...` or `marketplace:<p>/...`.
    pub fn of(source: &str) -> Self {
        let plugin_of = source
            .strip_prefix("plugin:")
            .or_else(|| source.strip_prefix("marketplace:"))
            .map(|rest| rest.split('/').next().unwrap_or_default().to_string())
            .filter(|p| !p.is_empty());
        Origin {
            library: source == "shared",
            plugin_of,
        }
    }
}

/// What is cheaply known about one `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillFacts {
    pub name_fit: bool,
    /// The description with whitespace flattened to single spaces, in characters.
    pub description_chars: usize,
    pub frontmatter_bytes: usize,
    pub file_bytes: usize,
    pub origin: Origin,
}

impl SkillFacts {
    /// From the file's text and where its copy lives.
    pub fn from_text(text: &str, origin: Origin) -> Self {
        let (description, _version, name) = crate::skills::parse_skill_meta(text);
        let flat: usize = description
            .split_whitespace()
            .map(|w| w.chars().count())
            .sum::<usize>()
            + description.split_whitespace().count().saturating_sub(1);
        // A name that is missing falls back to the folder in the index, which
        // is fine: only a present name the index can't use is a finding.
        let name_fit = name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .is_none_or(crate::skill_index::fit_name);
        let frontmatter_bytes = text
            .strip_prefix("---")
            .map_or(0, |rest| rest.find("\n---").map_or(0, |at| at + 3 + 4));
        SkillFacts {
            name_fit,
            description_chars: flat,
            frontmatter_bytes,
            file_bytes: text.len(),
            origin,
        }
    }
}

impl Check {
    /// Whether the check hits `facts`.
    pub fn hits(self, facts: &SkillFacts) -> bool {
        match self {
            Check::Always => true,
            Check::DescriptionOver(n) => facts.description_chars > n,
            Check::NameUnfit => !facts.name_fit,
            Check::FrontmatterOver(n) => facts.frontmatter_bytes > n,
            Check::FileOver(n) => facts.file_bytes > n,
            Check::NotLibrary => !facts.origin.library,
            Check::PluginOf(p) => facts.origin.plugin_of.as_deref() == Some(p),
        }
    }
}

// ── Versions ────────────────────────────────────────────────────────────────

/// How a rule's evidence stands against the installed CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VersionStatus {
    /// Hollow's own code: no CLI version applies.
    Hollow,
    /// Checked on the installed version.
    Verified,
    /// Checked on another version than the installed one.
    NotVerifiedOnThisVersion,
    /// Checked on a version, and the installed one couldn't be read.
    InstalledUnknown,
    /// Never verified.
    Unverified,
}

/// Compares a rule's evidence with the installed version string. Pure.
pub fn version_status(evidence: Evidence, installed: Option<&str>) -> VersionStatus {
    match (evidence, installed.map(str::trim).filter(|v| !v.is_empty())) {
        (Evidence::Hollow, _) => VersionStatus::Hollow,
        (Evidence::None(_), _) => VersionStatus::Unverified,
        (Evidence::Cli(_), None) => VersionStatus::InstalledUnknown,
        (Evidence::Cli(v), Some(now)) if v == now => VersionStatus::Verified,
        (Evidence::Cli(_), Some(_)) => VersionStatus::NotVerifiedOnThisVersion,
    }
}

// ── Assessment ──────────────────────────────────────────────────────────────

/// One rule that hit a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    pub rule: String,
    pub channel: Channel,
    pub effect: Effect,
    pub sentence: String,
    pub verified_on: Option<String>,
    pub status: VersionStatus,
}

/// `profile`'s rules that hit `facts`, in the profile's order. Pure.
pub fn assess(profile: &Profile, facts: &SkillFacts, installed: Option<&str>) -> Vec<Finding> {
    profile
        .rules
        .iter()
        .filter(|r| r.check.hits(facts))
        .map(|r| Finding {
            rule: r.id.to_string(),
            channel: r.channel,
            effect: r.effect,
            sentence: r.sentence.to_string(),
            verified_on: verified_on(r.evidence).map(str::to_string),
            status: version_status(r.evidence, installed),
        })
        .collect()
}

/// The agent's summary effect for a skill: the worst effect among the Hollow
/// channel's findings, `Loads` when none. Native findings never move it.
pub fn summary(findings: &[Finding]) -> Effect {
    findings
        .iter()
        .filter(|f| f.channel == Channel::Hollow)
        .map(|f| f.effect)
        .max()
        .unwrap_or(Effect::Loads)
}

fn verified_on(evidence: Evidence) -> Option<&'static str> {
    match evidence {
        Evidence::Cli(v) => Some(v),
        _ => None,
    }
}

// ── The wire ────────────────────────────────────────────────────────────────

/// A rule as the table shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRow {
    pub id: String,
    pub channel: Channel,
    pub effect: Effect,
    pub what: String,
    pub sentence: String,
    pub verified_on: Option<String>,
    /// Why nothing is verified, when [`Evidence::None`].
    pub unverified_reason: Option<String>,
    pub status: VersionStatus,
}

/// One agent's row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRow {
    pub agent: String,
    pub label: String,
    /// The installed CLI version, when it could be read.
    pub installed: Option<String>,
    pub rules: Vec<RuleRow>,
    /// Present only when a skill was assessed.
    pub summary: Option<Effect>,
    pub findings: Vec<Finding>,
    /// True when some rule was checked on a CLI version other than the
    /// installed one, or the installed one couldn't be read: the table says
    /// "not verified on this version".
    pub stale: bool,
}

/// The table, for every agent, with one skill's assessment when given.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortabilityTable {
    pub agents: Vec<AgentRow>,
    /// The assessed skill's name, when one was.
    pub skill: Option<String>,
}

/// Builds the table from the installed versions (read by the caller, outside
/// locks) and an optional skill's facts. Pure.
pub fn table(
    installed: &[(AgentKind, Option<String>)],
    facts: Option<(&str, &SkillFacts)>,
) -> PortabilityTable {
    let order = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Grok,
        AgentKind::Antigravity,
    ];
    let agents = order
        .into_iter()
        .filter_map(profile)
        .map(|p| {
            let version = installed
                .iter()
                .find(|(k, _)| *k == p.kind)
                .and_then(|(_, v)| v.clone());
            let now = version.as_deref();
            let rules: Vec<RuleRow> = p
                .rules
                .iter()
                .map(|r| RuleRow {
                    id: r.id.to_string(),
                    channel: r.channel,
                    effect: r.effect,
                    what: r.what.to_string(),
                    sentence: r.sentence.to_string(),
                    verified_on: verified_on(r.evidence).map(str::to_string),
                    unverified_reason: match r.evidence {
                        Evidence::None(why) => Some(why.to_string()),
                        _ => None,
                    },
                    status: version_status(r.evidence, now),
                })
                .collect();
            let findings = facts.map(|(_, f)| assess(p, f, now)).unwrap_or_default();
            AgentRow {
                agent: p.kind.as_str().to_string(),
                label: p.kind.label().to_string(),
                installed: version.clone(),
                stale: rules.iter().any(|r| {
                    matches!(
                        r.status,
                        VersionStatus::NotVerifiedOnThisVersion | VersionStatus::InstalledUnknown
                    )
                }),
                summary: facts.map(|_| summary(&findings)),
                findings,
                rules,
            }
        })
        .collect();
    PortabilityTable {
        agents,
        skill: facts.map(|(name, _)| name.to_string()),
    }
}

/// Reads each agent's installed version (the shared per-binary cache, once
/// per agent), then builds the table. Call from async code holding no lock.
pub async fn resolve(facts: Option<(&str, &SkillFacts)>) -> PortabilityTable {
    let mut installed = Vec::new();
    for kind in AgentKind::all() {
        installed.push((kind, crate::agent_cli::steering_version(kind).await));
    }
    table(&installed, facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> SkillFacts {
        SkillFacts {
            name_fit: true,
            description_chars: 80,
            frontmatter_bytes: 200,
            file_bytes: 1_000,
            origin: Origin {
                library: true,
                plugin_of: None,
            },
        }
    }

    fn of(kind: AgentKind, f: &SkillFacts) -> Vec<Finding> {
        assess(profile(kind).unwrap(), f, None)
    }

    fn rule_ids(found: &[Finding]) -> Vec<&str> {
        found.iter().map(|f| f.rule.as_str()).collect()
    }

    fn hollow_ids(found: &[Finding]) -> Vec<&str> {
        found
            .iter()
            .filter(|f| f.channel == Channel::Hollow)
            .map(|f| f.rule.as_str())
            .collect()
    }

    #[test]
    fn every_agent_has_exactly_one_profile_with_unique_rule_ids() {
        for kind in AgentKind::all() {
            assert_eq!(
                PROFILES.iter().filter(|p| p.kind == kind).count(),
                1,
                "{kind:?}"
            );
            let p = profile(kind).unwrap();
            let mut ids: Vec<&str> = p.rules.iter().map(|r| r.id).collect();
            ids.sort_unstable();
            let n = ids.len();
            ids.dedup();
            assert_eq!(ids.len(), n, "{kind:?} repeats a rule id");
            assert!(
                p.rules.iter().any(|r| r.channel == Channel::Native),
                "{kind:?} states its own loader, even as unknown"
            );
        }
    }

    #[test]
    fn a_plain_skill_loads_everywhere_and_the_native_loader_stays_unknown() {
        for kind in AgentKind::all() {
            let found = of(kind, &facts());
            assert_eq!(summary(&found), Effect::Loads, "{kind:?}");
            assert!(hollow_ids(&found).is_empty(), "{kind:?}");
            let native: Vec<_> = found
                .iter()
                .filter(|f| f.channel == Channel::Native)
                .collect();
            assert_eq!(native.len(), 1, "{kind:?}");
            assert_eq!(native[0].effect, Effect::Unknown, "{kind:?}");
            assert_eq!(native[0].status, VersionStatus::Unverified, "{kind:?}");
        }
    }

    #[test]
    fn a_300_character_description_is_degraded_for_every_agent() {
        let mut f = facts();
        f.description_chars = 300;
        for kind in AgentKind::all() {
            let found = of(kind, &f);
            assert!(rule_ids(&found).contains(&"desc-cap-160"), "{kind:?}");
            assert_eq!(summary(&found), Effect::Degraded, "{kind:?}");
        }
        // The edge: 160 fits, 161 does not.
        f.description_chars = DESCRIPTION_CHARS;
        assert!(!rule_ids(&of(AgentKind::Grok, &f)).contains(&"desc-cap-160"));
        f.description_chars = DESCRIPTION_CHARS + 1;
        assert!(rule_ids(&of(AgentKind::Grok, &f)).contains(&"desc-cap-160"));
    }

    #[test]
    fn the_description_rule_counts_what_the_index_counts() {
        // The index flattens whitespace, then cuts at 160 characters.
        let long = "word ".repeat(40);
        let text = format!("---\nname: x\ndescription: |\n  {long}\n  {long}\n---\nbody\n");
        let f = SkillFacts::from_text(&text, Origin::of("shared"));
        // 80 words of 4 letters, 79 single spaces.
        assert_eq!(f.description_chars, 80 * 4 + 79);
        let short = SkillFacts::from_text(
            "---\nname: x\ndescription: hello   there\n---\n",
            Origin::of("shared"),
        );
        assert_eq!(short.description_chars, "hello there".len());
        assert!(f.frontmatter_bytes > 0 && f.frontmatter_bytes < f.file_bytes);
    }

    #[test]
    fn an_unfit_name_is_a_finding_and_a_missing_one_is_not() {
        let mut f = facts();
        f.name_fit = false;
        for kind in AgentKind::all() {
            assert!(rule_ids(&of(kind, &f)).contains(&"name-unfit"), "{kind:?}");
        }
        let bad = format!("---\nname: {}\ndescription: d\n---\n", "n".repeat(65));
        assert!(!SkillFacts::from_text(&bad, Origin::of("shared")).name_fit);
        let slash = "---\nname: a/b\ndescription: d\n---\n";
        assert!(!SkillFacts::from_text(slash, Origin::of("shared")).name_fit);
        let none = "---\ndescription: d\n---\n";
        assert!(SkillFacts::from_text(none, Origin::of("shared")).name_fit);
    }

    #[test]
    fn frontmatter_past_16_kb_is_degraded_for_every_agent() {
        let mut f = facts();
        f.frontmatter_bytes = INDEX_FRONTMATTER_BYTES + 1;
        for kind in AgentKind::all() {
            assert!(
                rule_ids(&of(kind, &f)).contains(&"frontmatter-16k"),
                "{kind:?}"
            );
        }
        f.frontmatter_bytes = INDEX_FRONTMATTER_BYTES;
        assert!(!rule_ids(&of(AgentKind::Claude, &f)).contains(&"frontmatter-16k"));
        // The constant is the index's own, and the rule's words say 16 KB.
        assert_eq!(INDEX_FRONTMATTER_BYTES, 16 * 1024);
    }

    #[test]
    fn a_copy_outside_the_library_is_degraded_for_every_agent() {
        let mut f = facts();
        f.origin = Origin::of("provider:claude");
        for kind in AgentKind::all() {
            let found = of(kind, &f);
            assert!(rule_ids(&found).contains(&"not-indexed-copy"), "{kind:?}");
            assert_eq!(summary(&found), Effect::Degraded, "{kind:?}");
        }
    }

    #[test]
    fn a_skill_over_8_kb_is_read_on_demand_by_claude_and_codex_and_opened_by_path_by_the_others() {
        let mut f = facts();
        f.file_bytes = INLINE_BUDGET_BYTES + 1;
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let found = of(kind, &f);
            assert_eq!(rule_ids(&found)[..1], ["inline-8k-read-skill"], "{kind:?}");
            assert_eq!(summary(&found), Effect::Loads, "{kind:?}");
        }
        for kind in [AgentKind::Grok, AgentKind::Antigravity] {
            let found = of(kind, &f);
            assert!(
                rule_ids(&found).contains(&"inline-8k-own-tools"),
                "{kind:?}"
            );
            assert_eq!(summary(&found), Effect::Degraded, "{kind:?}");
        }
        f.file_bytes = INLINE_BUDGET_BYTES;
        for kind in AgentKind::all() {
            assert!(hollow_ids(&of(kind, &f)).is_empty(), "{kind:?}");
        }
    }

    #[test]
    fn the_read_route_follows_the_owned_mcp_capability() {
        // The Hollow reader rules are keyed by the same fact the capability
        // table states, so a profile can't drift from it.
        let mut f = facts();
        f.file_bytes = INLINE_BUDGET_BYTES + 1;
        // Antigravity is left out: its profile is the one-shot route (`agy -p`,
        // no servers); its ACP chats also have the owned reader (2026-11-20),
        // which this per-agent table can't say.
        for kind in AgentKind::all()
            .into_iter()
            .filter(|k| *k != AgentKind::Antigravity)
        {
            let own_tools = rule_ids(&of(kind, &f)).contains(&"inline-8k-own-tools");
            assert_eq!(
                own_tools,
                !crate::agent::capabilities(kind).owned_mcp,
                "{kind:?}"
            );
        }
    }

    #[test]
    fn read_skill_cuts_at_64_kb_for_the_agents_that_have_it() {
        let mut f = facts();
        f.file_bytes = READ_MAX_BYTES + 1;
        for kind in [AgentKind::Claude, AgentKind::Codex] {
            let found = of(kind, &f);
            assert!(rule_ids(&found).contains(&"read-cap-64k"), "{kind:?}");
            assert_eq!(summary(&found), Effect::Degraded, "{kind:?}");
        }
        // Grok and Antigravity have no read_skill, so Hollow cuts nothing for them.
        for kind in [AgentKind::Grok, AgentKind::Antigravity] {
            assert!(
                !rule_ids(&of(kind, &f)).contains(&"read-cap-64k"),
                "{kind:?}"
            );
        }
        f.file_bytes = READ_MAX_BYTES;
        assert!(!rule_ids(&of(AgentKind::Claude, &f)).contains(&"read-cap-64k"));
    }

    #[test]
    fn a_codex_plugin_skill_is_expected_not_to_load_natively_and_never_moves_the_summary() {
        let mut f = facts();
        f.origin = Origin::of("plugin:codex/market/tool@1.0");
        assert_eq!(f.origin.plugin_of.as_deref(), Some("codex"));
        let codex = of(AgentKind::Codex, &f);
        let off = codex
            .iter()
            .find(|x| x.rule == "native-codex-plugin-off")
            .expect("plugins are off for Codex");
        // Hollow's flag is real, but what it does to a plugin's skills isn't verified.
        assert_eq!(off.effect, Effect::Unknown);
        assert_eq!(off.status, VersionStatus::Unverified);
        // Only the Hollow channel sets the summary: not-in-the-library does.
        assert_eq!(summary(&codex), Effect::Degraded);
        // Another provider's plugin has no such rule.
        for kind in [AgentKind::Claude, AgentKind::Grok, AgentKind::Antigravity] {
            assert!(
                !rule_ids(&of(kind, &f)).contains(&"native-codex-plugin-off"),
                "{kind:?}"
            );
        }
        // The flag is Hollow's: the Codex builders pass it.
        let agent = include_str!("agent.rs");
        assert!(agent.matches("\"plugins\",").count() >= 2);
    }

    #[test]
    fn origin_reads_every_source_the_skills_page_lists() {
        assert!(Origin::of("shared").library);
        assert_eq!(Origin::of("provider:claude"), Origin::default());
        assert_eq!(Origin::of("provider:codex-system"), Origin::default());
        assert_eq!(
            Origin::of("marketplace:claude/market/tool")
                .plugin_of
                .as_deref(),
            Some("claude")
        );
        assert_eq!(Origin::of("plugin:").plugin_of, None);
    }

    #[test]
    fn evidence_is_recorded_for_what_it_is() {
        for p in PROFILES {
            for r in p.rules {
                match r.evidence {
                    Evidence::None(why) => {
                        assert!(why.len() > 20, "{} says why", r.id);
                        assert_eq!(
                            r.channel,
                            Channel::Native,
                            "{}: only a native rule is unverified",
                            r.id
                        );
                        assert_eq!(
                            r.effect,
                            Effect::Unknown,
                            "{}: unverified means unknown",
                            r.id
                        );
                    }
                    Evidence::Cli(v) => {
                        assert!(v.split('.').count() == 3, "{} names a version", r.id)
                    }
                    Evidence::Hollow => {}
                }
                assert_eq!(
                    r.effect == Effect::Unknown,
                    matches!(r.evidence, Evidence::None(_)),
                    "{}",
                    r.id
                );
                for text in [r.what, r.sentence] {
                    assert!(
                        !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                        "{}",
                        r.id
                    );
                    assert!(!text.is_empty());
                }
            }
        }
        // Grok and Antigravity claim nothing about a native loader.
        for kind in [AgentKind::Grok, AgentKind::Antigravity] {
            let native: Vec<_> = profile(kind)
                .unwrap()
                .rules
                .iter()
                .filter(|r| r.channel == Channel::Native)
                .collect();
            assert_eq!(native.len(), 1);
            assert_eq!(native[0].effect, Effect::Unknown);
        }
    }

    #[test]
    fn a_recorded_version_goes_stale_when_the_cli_updates() {
        assert_eq!(
            version_status(Evidence::Hollow, Some("9.9.9")),
            VersionStatus::Hollow
        );
        assert_eq!(
            version_status(Evidence::None("x"), Some("1.0.46")),
            VersionStatus::Unverified
        );
        assert_eq!(
            version_status(Evidence::Cli("1.0.46"), Some("1.0.46")),
            VersionStatus::Verified
        );
        assert_eq!(
            version_status(Evidence::Cli("1.0.46"), Some(" 1.0.46 ")),
            VersionStatus::Verified
        );
        assert_eq!(
            version_status(Evidence::Cli("1.0.46"), Some("1.0.79")),
            VersionStatus::NotVerifiedOnThisVersion
        );
        assert_eq!(
            version_status(Evidence::Cli("1.0.46"), None),
            VersionStatus::InstalledUnknown
        );
        assert_eq!(
            version_status(Evidence::Cli("1.0.46"), Some("")),
            VersionStatus::InstalledUnknown
        );
        // An unreadable installed version marks the row stale, so its badge shows.
        let unreadable = table(&[(AgentKind::Grok, None)], None);
        let grok = unreadable
            .agents
            .iter()
            .find(|a| a.agent == "grok")
            .unwrap();
        assert!(grok.stale && grok.installed.is_none());
        let claude = unreadable
            .agents
            .iter()
            .find(|a| a.agent == "claude")
            .unwrap();
        assert!(
            !claude.stale,
            "a Hollow-only profile has nothing to confirm"
        );
    }

    #[test]
    fn the_table_marks_a_row_stale_only_when_a_recorded_version_differs() {
        let mut f = facts();
        f.file_bytes = INLINE_BUDGET_BYTES + 1;
        let installed = |grok: &str, agy: &str| {
            vec![
                (AgentKind::Claude, Some("2.1.325".to_string())),
                (AgentKind::Codex, Some("0.169.0".to_string())),
                (AgentKind::Grok, Some(grok.to_string())),
                (AgentKind::Antigravity, Some(agy.to_string())),
            ]
        };
        let now = table(&installed("1.0.79", "1.5.9"), Some(("big", &f)));
        let by = |t: &PortabilityTable, agent: &str| {
            t.agents.iter().find(|a| a.agent == agent).unwrap().clone()
        };
        assert_eq!(now.skill.as_deref(), Some("big"));
        assert_eq!(
            now.agents
                .iter()
                .map(|a| a.agent.as_str())
                .collect::<Vec<_>>(),
            ["claude", "codex", "grok", "antigravity"]
        );
        assert!(by(&now, "grok").stale && by(&now, "antigravity").stale);
        assert!(!by(&now, "claude").stale && !by(&now, "codex").stale);
        let grok = by(&now, "grok");
        let hit = grok
            .findings
            .iter()
            .find(|x| x.rule == "inline-8k-own-tools")
            .unwrap();
        assert_eq!(hit.verified_on.as_deref(), Some("1.0.46"));
        assert_eq!(hit.status, VersionStatus::NotVerifiedOnThisVersion);
        assert_eq!(grok.summary, Some(Effect::Degraded));
        // On the version it was checked on, the row is not stale.
        let same = table(&installed("1.0.46", "1.2.15"), Some(("big", &f)));
        assert!(!by(&same, "grok").stale && !by(&same, "antigravity").stale);
        // With no skill, the rules are listed and nothing is assessed.
        let bare = table(&installed("1.0.46", "1.2.15"), None);
        assert!(bare.skill.is_none());
        assert!(bare
            .agents
            .iter()
            .all(|a| a.summary.is_none() && a.findings.is_empty() && !a.rules.is_empty()));
        // The unverified reason travels with its rule.
        let claude = by(&bare, "claude");
        let native = claude
            .rules
            .iter()
            .find(|r| r.id == "native-claude")
            .unwrap();
        assert!(native.unverified_reason.is_some() && native.verified_on.is_none());
        assert_eq!(native.status, VersionStatus::Unverified);
    }

    #[test]
    fn the_table_serializes_in_the_shape_the_page_reads() {
        let t = table(
            &[(AgentKind::Grok, Some("1.0.46".into()))],
            Some(("n", &facts())),
        );
        let json = serde_json::to_value(&t).unwrap();
        let grok = &json["agents"][2];
        assert_eq!(grok["agent"], "grok");
        assert_eq!(grok["installed"], "1.0.46");
        assert_eq!(grok["summary"], "loads");
        assert_eq!(grok["rules"][4]["status"], "verified");
        assert_eq!(grok["rules"][4]["effect"], "degraded");
        assert_eq!(grok["rules"][5]["status"], "unverified");
        assert_eq!(grok["rules"][5]["channel"], "native");
        assert_eq!(grok["findings"][0]["status"], "unverified");
    }
}
