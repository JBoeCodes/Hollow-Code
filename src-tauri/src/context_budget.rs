//! One total budget for the context Hollow injects into a chat turn, per
//! provider (agent effectiveness F6, wave 3 · G2).
//!
//! Before G2 every layer had its own cap (the workspace envelope 128 KB,
//! the conversation bundle 32,000 characters, the profile 16,000 bytes, the
//! visual guide about 13.4 KB, …) and nothing bounded their sum: a fresh
//! turn of a heavy workspace chat carried about 150 KB of Hollow's text.
//!
//! **The budget.** [`for_agent`]: [`TOTAL_BYTES`] for every provider. What
//! differs per provider is where the configured half goes and so what each
//! turn is charged ([`Channel`]): Claude gets it as `--append-system-prompt`
//! in every process, so every turn is charged for it; Codex, Antigravity
//! and Grok get it at the top of the prompt, so a turn is charged for what
//! it actually carries (the whole configured half, or on a resumed turn
//! whose native session holds it, the one-line reference,
//! `stable_layers.rs`). The numbers are equal because nothing measured
//! justifies a difference yet: the pilot's real token counts can set them
//! per provider later. Every provider gets its prompt in argv, whose macOS
//! limit (1 MiB with the environment) is far above.
//!
//! **Allocation** ([`allocate`]), by priority:
//! 1. never cut: the configured half and the connectors note (permission
//!    wording), the request (the caller's composed text), the task-state
//!    record, and the reserved settled context: approved recall, the
//!    profile's settled files (`me/Decisions.md`, `me/Guardrails.md`) and a
//!    profile revision delta;
//! 2. evidence: the retrieved evidence whole, then the workspace envelope
//!    and the conversation bundle sharing what is left (the smaller kept
//!    whole, the larger cut to an equal share; both can be read again: the
//!    workspace files with the agent's tools, the transcript through
//!    `read_session`);
//! 3. profile prose (Identity, Communication, Anti-patterns), with what
//!    evidence left;
//! 4. optional guidance: the visual guide block, whole or not at all (its
//!    file in the scratch folder still has it).
//!
//! When the never-cut tiers alone exceed the total, the turn still goes,
//! over budget, and the record says so: nothing settled is ever dropped to
//! fit. The run manifest records each layer's ask and grant
//! (`compiler.budget`).

use crate::agent::AgentKind;

/// The total for one turn's injected context, in UTF-8 bytes: the workspace
/// envelope's own ceiling, so a turn never carries more of Hollow's text
/// than that one layer alone was allowed (about 32,000 tokens at four bytes
/// a token).
pub(crate) const TOTAL_BYTES: usize = 128_000;

/// Where a provider gets Hollow's configured half.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Channel {
    /// Claude's `--append-system-prompt`, in every process.
    SystemPrompt,
    /// The top of the prompt (Codex, Antigravity, Grok).
    Prompt,
}

impl Channel {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Channel::SystemPrompt => "systemPrompt",
            Channel::Prompt => "prompt",
        }
    }
}

/// One provider's budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProviderBudget {
    pub total: usize,
    pub configured: Channel,
}

/// The budget for a chat turn with `agent`.
pub(crate) fn for_agent(agent: AgentKind) -> ProviderBudget {
    match agent {
        AgentKind::Claude => ProviderBudget {
            total: TOTAL_BYTES,
            configured: Channel::SystemPrompt,
        },
        AgentKind::Codex | AgentKind::Antigravity | AgentKind::Grok => ProviderBudget {
            total: TOTAL_BYTES,
            configured: Channel::Prompt,
        },
    }
}

/// A layer's priority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tier {
    /// Never cut: the configured half and the connectors note.
    Fixed,
    /// Never cut: the caller's composed text.
    Request,
    /// Never cut: the `<task-state>` record.
    TaskState,
    /// Never cut: settled context (recall, the profile's Decisions and
    /// Guardrails, the profile delta).
    Reserved,
    /// The retrieved evidence (granted whole), the workspace envelope and
    /// the conversation bundle (shared).
    Evidence,
    /// Identity, Communication, Anti-patterns.
    ProfileProse,
    /// The visual guide block.
    Guidance,
}

impl Tier {
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Tier::Fixed => "fixed",
            Tier::Request => "request",
            Tier::TaskState => "taskState",
            Tier::Reserved => "reserved",
            Tier::Evidence => "evidence",
            Tier::ProfileProse => "profileProse",
            Tier::Guidance => "guidance",
        }
    }
}

/// What one layer asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    pub name: &'static str,
    pub tier: Tier,
    pub bytes: usize,
    /// An evidence layer that can be cut (the envelope, the bundle); the
    /// retrieved evidence is granted whole.
    pub cuttable: bool,
}

impl Ask {
    pub(crate) fn whole(name: &'static str, tier: Tier, bytes: usize) -> Self {
        Ask {
            name,
            tier,
            bytes,
            cuttable: false,
        }
    }

    pub(crate) fn cuttable(name: &'static str, tier: Tier, bytes: usize) -> Self {
        Ask {
            name,
            tier,
            bytes,
            cuttable: true,
        }
    }
}

/// What one layer got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Grant {
    pub name: &'static str,
    pub tier: Tier,
    pub wanted: usize,
    pub granted: usize,
}

/// A turn's allocation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Allocation {
    pub total: usize,
    pub grants: Vec<Grant>,
}

impl Allocation {
    /// What `name` was granted; what it asked for when it isn't listed.
    pub(crate) fn granted(&self, name: &str) -> Option<usize> {
        self.grants
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.granted)
    }

    pub(crate) fn used(&self) -> usize {
        self.grants.iter().map(|g| g.granted).sum()
    }

    /// Whether `name` got less than it asked for.
    pub(crate) fn cut(&self, name: &str) -> bool {
        self.grants
            .iter()
            .any(|g| g.name == name && g.granted < g.wanted)
    }

    /// The manifest's record of it.
    pub(crate) fn record(
        &self,
        agent: AgentKind,
        channel: Channel,
    ) -> crate::run_manifest::BudgetRecord {
        crate::run_manifest::BudgetRecord {
            provider: agent.as_str().to_string(),
            total: self.total as u64,
            configured_channel: channel.wire().to_string(),
            used: self.used() as u64,
            over: self.used() > self.total,
            layers: self
                .grants
                .iter()
                .map(|g| crate::run_manifest::BudgetLayer {
                    name: g.name.to_string(),
                    tier: g.tier.wire().to_string(),
                    wanted: g.wanted as u64,
                    granted: g.granted as u64,
                })
                .collect(),
        }
    }
}

/// Allocate `total` bytes to `asks` by priority (the module doc). Grants
/// keep the asks' order.
pub(crate) fn allocate(total: usize, asks: &[Ask]) -> Allocation {
    let mut granted = vec![0usize; asks.len()];
    // Never cut, and the whole evidence layers.
    let mut left = total;
    for (i, ask) in asks.iter().enumerate() {
        let shared = ask.tier == Tier::Evidence && ask.cuttable;
        let whole = !shared && !matches!(ask.tier, Tier::ProfileProse | Tier::Guidance);
        if whole {
            granted[i] = ask.bytes;
            left = left.saturating_sub(ask.bytes);
        }
    }
    // The cuttable evidence shares what is left: the smaller whole, the
    // larger cut to an equal share.
    let mut shared: Vec<usize> = (0..asks.len())
        .filter(|&i| asks[i].tier == Tier::Evidence && asks[i].cuttable)
        .collect();
    shared.sort_by_key(|&i| asks[i].bytes);
    let count = shared.len();
    for (done, &i) in shared.iter().enumerate() {
        let fair = left / (count - done);
        granted[i] = asks[i].bytes.min(fair);
        left -= granted[i];
    }
    for (i, ask) in asks.iter().enumerate() {
        match ask.tier {
            Tier::ProfileProse => {
                granted[i] = ask.bytes.min(left);
                left -= granted[i];
            }
            Tier::Guidance if ask.bytes <= left => {
                granted[i] = ask.bytes;
                left -= ask.bytes;
            }
            _ => {}
        }
    }
    Allocation {
        total,
        grants: asks
            .iter()
            .zip(granted)
            .map(|(ask, granted)| Grant {
                name: ask.name,
                tier: ask.tier,
                wanted: ask.bytes,
                granted,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asks(envelope: usize, bundle: usize, prose: usize, guide: usize) -> Vec<Ask> {
        vec![
            Ask::whole("configured", Tier::Fixed, 9_000),
            Ask::whole("request", Tier::Request, 500),
            Ask::whole("taskState", Tier::TaskState, 2_000),
            Ask::whole("profileSettled", Tier::Reserved, 1_000),
            Ask::whole("retrievalEvidence", Tier::Evidence, 8_000),
            Ask::cuttable("workspace", Tier::Evidence, envelope),
            Ask::cuttable("conversationContext", Tier::Evidence, bundle),
            Ask::whole("profileProse", Tier::ProfileProse, prose),
            Ask::whole("visualGuide", Tier::Guidance, guide),
        ]
    }

    #[test]
    fn a_turn_that_fits_gets_everything() {
        let a = allocate(TOTAL_BYTES, &asks(30_000, 10_000, 5_000, 13_000));
        assert!(a.grants.iter().all(|g| g.granted == g.wanted), "{a:?}");
        assert!(a.used() <= TOTAL_BYTES);
    }

    /// Guidance goes first, then prose, then evidence; the never-cut tiers
    /// stay whole.
    #[test]
    fn an_overflow_is_cut_from_the_lowest_priority() {
        // 20.5 KB fixed + 90 + 32 + 15 + 13 = 170.5 KB: over by 42.5 KB.
        let a = allocate(TOTAL_BYTES, &asks(90_000, 32_000, 15_000, 13_000));
        assert_eq!(a.granted("visualGuide"), Some(0));
        assert_eq!(a.granted("profileProse"), Some(0));
        for name in [
            "configured",
            "request",
            "taskState",
            "profileSettled",
            "retrievalEvidence",
        ] {
            assert!(!a.cut(name), "{name}");
        }
        // 128 - 20.5 = 107.5 KB shared: the bundle whole, the envelope the rest.
        assert_eq!(a.granted("conversationContext"), Some(32_000));
        assert_eq!(a.granted("workspace"), Some(TOTAL_BYTES - 20_500 - 32_000));
        assert_eq!(a.used(), TOTAL_BYTES);

        // A smaller overflow cuts only the guide.
        let a = allocate(TOTAL_BYTES, &asks(70_000, 20_000, 12_000, 13_000));
        assert_eq!(a.granted("visualGuide"), Some(0));
        assert_eq!(a.granted("profileProse"), Some(12_000));
        assert!(!a.cut("workspace") && !a.cut("conversationContext"));
    }

    #[test]
    fn two_large_evidence_layers_share_equally() {
        let a = allocate(TOTAL_BYTES, &asks(120_000, 120_000, 0, 0));
        let left = TOTAL_BYTES - 20_500;
        assert_eq!(a.granted("workspace"), Some(left / 2));
        assert_eq!(a.granted("conversationContext"), Some(left - left / 2));
    }

    /// Nothing settled is dropped to fit: the turn goes over and says so.
    #[test]
    fn the_never_cut_tiers_can_go_over() {
        let mut big = asks(10_000, 10_000, 5_000, 0);
        big[1].bytes = 150_000;
        let a = allocate(TOTAL_BYTES, &big);
        assert_eq!(a.granted("request"), Some(150_000));
        assert_eq!(a.granted("profileSettled"), Some(1_000));
        assert_eq!(a.granted("workspace"), Some(0));
        let record = a.record(AgentKind::Codex, Channel::Prompt);
        assert!(record.over);
        assert_eq!(record.configured_channel, "prompt");
    }

    #[test]
    fn every_provider_has_a_documented_budget() {
        for agent in [
            AgentKind::Claude,
            AgentKind::Codex,
            AgentKind::Antigravity,
            AgentKind::Grok,
        ] {
            let budget = for_agent(agent);
            assert_eq!(budget.total, TOTAL_BYTES);
            assert_eq!(
                budget.configured == Channel::SystemPrompt,
                agent == AgentKind::Claude
            );
        }
    }
}
