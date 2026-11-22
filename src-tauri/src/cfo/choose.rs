//! Which agent and model a subagent runs on (COO rebuild L7), moved from the
//! company code's `routing.rs` (decision 1 of 11-09; removed 11-10). Pure: the
//! settings, the installed agents, the saved models, the app's default agent,
//! the current load, the model catalogs and the alias map come in as
//! [`Inputs`].
//!
//! - A named agent (and model) must be installed and inside a non-empty
//!   allowlist.
//! - A model named with no agent goes to an agent whose catalog (or a
//!   non-empty allowlist entry) has that model; else the app's default agent.
//! - Nothing named (or `auto`) with an EMPTY allowlist runs on the app's
//!   default agent as before; a review goes to another agent than its
//!   producer's.
//! - Nothing named with a non-empty allowlist picks from it: a review on
//!   another agent than the producer's, else the app's default agent when it
//!   is among the least busy, else the least busy allowed agent, ties in
//!   allowlist order.

use super::job::JobKind;
use super::settings::CfoSettings;
use crate::agent::AgentKind;
use std::collections::{BTreeMap, BTreeSet};

/// Where Settings keeps the allowlist, for the errors.
pub(crate) const MODELS_HOME: &str = "Settings → Models → Models the CFO may use";

/// What a placement is decided from.
pub(crate) struct Inputs<'a> {
    pub settings: &'a CfoSettings,
    pub installed: &'a [AgentKind],
    /// Each agent's saved model (`agent_settings.models`).
    pub saved: &'a BTreeMap<String, String>,
    /// The agent a start with nothing named ran on before the CFO (the
    /// saved default agent, else the first installed one).
    pub default_agent: Option<AgentKind>,
    /// Chat turns queued or running per agent.
    pub load: &'a BTreeMap<String, usize>,
    /// Each agent's catalog model ids (read only when a model is named with
    /// no agent).
    pub catalogs: &'a BTreeMap<String, BTreeSet<String>>,
    /// A model id in its pinned form (an alias such as `sonnet` through the
    /// agent's catalog alias map); identity when the agent has none.
    pub canon: &'a dyn Fn(AgentKind, &str) -> String,
}

/// The agent and model a task runs on, with the plain reason. `agent: None`
/// keeps the old default (the app's default agent on its saved model).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    pub agent: Option<AgentKind>,
    pub model: Option<String>,
    pub reason: String,
}

fn saved_model(saved: &BTreeMap<String, String>, kind: AgentKind) -> Option<String> {
    saved
        .get(kind.as_str())
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
}

/// Allowed (agent, model) pairs among the installed agents, in allowlist
/// order (or the app's agent order for an empty allowlist).
pub(crate) fn candidates(
    settings: &CfoSettings,
    installed: &[AgentKind],
    saved: &BTreeMap<String, String>,
) -> Vec<(AgentKind, Option<String>)> {
    if settings.allow.is_empty() {
        return installed
            .iter()
            .map(|k| (*k, saved_model(saved, *k)))
            .collect();
    }
    settings
        .allow
        .iter()
        .filter_map(|entry| {
            let kind = AgentKind::from_opt(&entry.agent)?;
            installed.contains(&kind).then(|| {
                (
                    kind,
                    entry.model.clone().or_else(|| saved_model(saved, kind)),
                )
            })
        })
        .collect()
}

/// Whether the allowlist permits this exact agent and model (aliases and
/// pinned ids match through `canon`).
pub(crate) fn allows(
    settings: &CfoSettings,
    agent: AgentKind,
    model: Option<&str>,
    canon: &dyn Fn(AgentKind, &str) -> String,
) -> bool {
    settings.allow.is_empty()
        || settings.allow.iter().any(|entry| {
            entry.agent == agent.as_str()
                && match (entry.model.as_deref(), model) {
                    (None, _) => true,
                    (Some(allowed), Some(asked)) => canon(agent, allowed) == canon(agent, asked),
                    (Some(_), None) => false,
                }
        })
}

/// The allowed models as one line, for an error.
pub(crate) fn allowed_line(settings: &CfoSettings) -> String {
    settings
        .allow
        .iter()
        .map(|e| {
            let label = AgentKind::from_opt(&e.agent)
                .map(|k| k.label().to_string())
                .unwrap_or_else(|| e.agent.clone());
            match &e.model {
                Some(model) => format!("{label} ({model})"),
                None => format!("{label} (any model)"),
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The refusal for an agent and model outside a non-empty allowlist.
pub(crate) fn refusal(settings: &CfoSettings, agent: AgentKind, model: Option<&str>) -> String {
    format!(
        "CFO: {}{} is not among the models the CFO may use ({MODELS_HOME}). Allowed: {}. Nothing started.",
        agent.label(),
        model.map(|m| format!(" ({m})")).unwrap_or_default(),
        allowed_line(settings)
    )
}

/// The model a named agent runs on: the one asked for, else its allowlist
/// model, else its saved one.
pub(crate) fn named_model(
    settings: &CfoSettings,
    saved: &BTreeMap<String, String>,
    agent: AgentKind,
    requested: Option<&str>,
) -> Option<String> {
    requested.map(str::to_string).or_else(|| {
        settings
            .allow
            .iter()
            .find(|e| e.agent == agent.as_str() && e.model.is_some())
            .and_then(|e| e.model.clone())
            .or_else(|| saved_model(saved, agent))
    })
}

/// Among `pool`: a review on another agent than its producer's, else the
/// default agent when it is among the least busy, else the least busy.
fn pick_from(
    inp: &Inputs<'_>,
    pool: &[(AgentKind, Option<String>)],
    kind: JobKind,
    producer: Option<AgentKind>,
) -> Placement {
    if let (JobKind::Review, Some(producer)) = (kind, producer) {
        if let Some(pick) = pool.iter().find(|(agent, _)| *agent != producer) {
            return Placement {
                agent: Some(pick.0),
                model: pick.1.clone(),
                reason: format!("independent of {}", producer.label()),
            };
        }
    }
    let busy = |agent: AgentKind| inp.load.get(agent.as_str()).copied().unwrap_or(0);
    let least = pool.iter().map(|(a, _)| busy(*a)).min().unwrap_or(0);
    if let Some(default) = inp.default_agent {
        if let Some(pick) = pool
            .iter()
            .find(|(agent, _)| *agent == default && busy(*agent) == least)
        {
            return Placement {
                agent: Some(pick.0),
                model: pick.1.clone(),
                reason: "the default agent".into(),
            };
        }
    }
    let pick = pool
        .iter()
        .find(|(agent, _)| busy(*agent) == least)
        .unwrap_or(&pool[0]);
    let reason = if inp.load.values().any(|n| *n > 0) {
        "least busy allowed agent"
    } else {
        "first allowed agent"
    };
    Placement {
        agent: Some(pick.0),
        model: pick.1.clone(),
        reason: reason.into(),
    }
}

/// Pick the agent and model for one task. `producer` is the agent whose work
/// a review checks.
pub(crate) fn choose(
    inp: &Inputs<'_>,
    requested_agent: Option<&str>,
    requested_model: Option<&str>,
    kind: JobKind,
    producer: Option<AgentKind>,
) -> Result<Placement, String> {
    let settings = inp.settings;
    let requested_model = requested_model
        .map(str::trim)
        .filter(|m| !m.is_empty() && *m != "auto")
        .map(str::to_string);
    let named = requested_agent
        .map(str::trim)
        .filter(|a| !a.is_empty() && *a != "auto");
    if let Some(name) = named {
        let agent =
            AgentKind::from_opt(name).ok_or_else(|| format!("CFO: unknown agent `{name}`"))?;
        if !inp.installed.contains(&agent) {
            return Err(format!(
                "CFO: {} is not installed on this Mac. Nothing started.",
                agent.label()
            ));
        }
        let model = named_model(settings, inp.saved, agent, requested_model.as_deref());
        if !allows(settings, agent, model.as_deref(), inp.canon) {
            return Err(refusal(settings, agent, model.as_deref()));
        }
        return Ok(Placement {
            agent: Some(agent),
            model,
            reason: "named for this task".into(),
        });
    }
    if let Some(model) = &requested_model {
        // Only agents that have the model: its catalog lists it, or a
        // non-empty allowlist names it for that agent.
        let has = |agent: AgentKind| {
            let wanted = (inp.canon)(agent, model);
            inp.catalogs.get(agent.as_str()).is_some_and(|ids| {
                ids.contains(model) || ids.iter().any(|id| (inp.canon)(agent, id) == wanted)
            }) || settings.allow.iter().any(|e| {
                e.agent == agent.as_str()
                    && e.model
                        .as_deref()
                        .is_some_and(|m| (inp.canon)(agent, m) == wanted)
            })
        };
        let owners: Vec<(AgentKind, Option<String>)> =
            candidates(settings, inp.installed, inp.saved)
                .into_iter()
                .filter(|(agent, _)| {
                    has(*agent) && allows(settings, *agent, Some(model), inp.canon)
                })
                .map(|(agent, _)| (agent, Some(model.clone())))
                .collect();
        if !owners.is_empty() {
            let mut placed = pick_from(inp, &owners, kind, producer);
            if !placed.reason.starts_with("independent") {
                let label = placed.agent.map(|a| a.label()).unwrap_or("its agent");
                placed.reason = format!("{label} has {model}");
            }
            return Ok(placed);
        }
        let Some(default) = inp.default_agent else {
            return Err(format!(
                "CFO: no installed agent has the model `{model}`. Nothing started."
            ));
        };
        if !allows(settings, default, Some(model), inp.canon) {
            return Err(refusal(settings, default, Some(model)));
        }
        return Ok(Placement {
            agent: Some(default),
            model: Some(model.clone()),
            reason: "the default agent".into(),
        });
    }
    if settings.allow.is_empty() {
        // As before the CFO: the app's default agent. A review still goes to
        // another agent than its producer's.
        if let (JobKind::Review, Some(producer)) = (kind, producer) {
            if inp.default_agent.is_none_or(|d| d == producer) {
                if let Some(other) = inp.installed.iter().find(|a| **a != producer) {
                    return Ok(Placement {
                        agent: Some(*other),
                        model: saved_model(inp.saved, *other),
                        reason: format!("independent of {}", producer.label()),
                    });
                }
            }
        }
        return Ok(Placement {
            agent: None,
            model: None,
            reason: "the default agent".into(),
        });
    }
    let pool = candidates(settings, inp.installed, inp.saved);
    if pool.is_empty() {
        return Err(format!(
            "CFO: no allowed model is installed; check {MODELS_HOME}. Nothing started."
        ));
    }
    Ok(pick_from(inp, &pool, kind, producer))
}

#[cfg(test)]
mod tests {
    use super::super::settings::AllowEntry;
    use super::*;

    fn saved() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("claude".to_string(), "claude-opus-5-5".to_string()),
            ("codex".to_string(), "gpt-5.5".to_string()),
        ])
    }
    const BOTH: [AgentKind; 2] = [AgentKind::Claude, AgentKind::Codex];

    /// Claude's `sonnet` alias is `claude-sonnet-5-5`; nothing else has one.
    fn canon(agent: AgentKind, model: &str) -> String {
        match (agent, model) {
            (AgentKind::Claude, "sonnet") => "claude-sonnet-5-5".into(),
            _ => model.to_string(),
        }
    }

    fn only(agent: &str, model: Option<&str>) -> CfoSettings {
        CfoSettings {
            allow: vec![AllowEntry {
                agent: agent.into(),
                model: model.map(str::to_string),
            }],
            ..CfoSettings::default()
        }
    }

    struct Fixture {
        saved: BTreeMap<String, String>,
        load: BTreeMap<String, usize>,
        catalogs: BTreeMap<String, BTreeSet<String>>,
    }

    fn fixture() -> Fixture {
        Fixture {
            saved: saved(),
            load: BTreeMap::new(),
            catalogs: BTreeMap::from([
                (
                    "claude".to_string(),
                    BTreeSet::from(["sonnet".to_string(), "claude-opus-5-5".to_string()]),
                ),
                ("codex".to_string(), BTreeSet::from(["gpt-5.5".to_string()])),
            ]),
        }
    }

    fn inputs<'a>(
        f: &'a Fixture,
        settings: &'a CfoSettings,
        installed: &'a [AgentKind],
        default_agent: Option<AgentKind>,
    ) -> Inputs<'a> {
        Inputs {
            settings,
            installed,
            saved: &f.saved,
            default_agent,
            load: &f.load,
            catalogs: &f.catalogs,
            canon: &canon,
        }
    }

    #[test]
    fn empty_allowlist_means_every_installed_agent_on_its_saved_model() {
        let pool = candidates(&CfoSettings::default(), &BOTH, &saved());
        assert_eq!(
            pool,
            vec![
                (AgentKind::Claude, Some("claude-opus-5-5".into())),
                (AgentKind::Codex, Some("gpt-5.5".into()))
            ]
        );
    }

    #[test]
    fn the_allowlist_refuses_a_named_model_outside_it_and_picks_inside_it() {
        let f = fixture();
        let settings = only("claude", Some("claude-sonnet-5-5"));
        let inp = inputs(&f, &settings, &BOTH, Some(AgentKind::Claude));
        let err = choose(
            &inp,
            Some("claude"),
            Some("claude-opus-5-5"),
            JobKind::Conversation,
            None,
        )
        .unwrap_err();
        assert!(
            err.contains("not among the models the CFO may use"),
            "{err}"
        );
        assert!(err.contains("Allowed: Claude (claude-sonnet-5-5)"), "{err}");
        let err = choose(&inp, Some("codex"), None, JobKind::Conversation, None).unwrap_err();
        assert!(err.contains("Nothing started"), "{err}");
        let ok = choose(&inp, Some("claude"), None, JobKind::Conversation, None).unwrap();
        assert_eq!(ok.model.as_deref(), Some("claude-sonnet-5-5"));
        // The alias names the same model as the pinned id in the list.
        let alias = choose(
            &inp,
            Some("claude"),
            Some("sonnet"),
            JobKind::Conversation,
            None,
        );
        assert!(alias.is_ok(), "{alias:?}");
        // Nothing named: the CFO picks inside the list.
        let auto = choose(&inp, Some("auto"), None, JobKind::Conversation, None).unwrap();
        assert_eq!(
            (auto.agent, auto.model.as_deref()),
            (Some(AgentKind::Claude), Some("claude-sonnet-5-5"))
        );
        let codex_only = inputs(&f, &settings, &[AgentKind::Codex], None);
        let err = choose(&codex_only, None, None, JobKind::Conversation, None).unwrap_err();
        assert!(err.contains("no allowed model is installed"), "{err}");
        let claude_only = inputs(&f, &settings, &[AgentKind::Claude], None);
        let err = choose(
            &claude_only,
            Some("codex"),
            None,
            JobKind::Conversation,
            None,
        )
        .unwrap_err();
        assert!(err.contains("not installed"), "{err}");
    }

    #[test]
    fn any_model_entry_takes_a_named_model() {
        let f = fixture();
        let settings = only("codex", None);
        let inp = inputs(&f, &settings, &BOTH, None);
        let ok = choose(
            &inp,
            Some("codex"),
            Some("gpt-5.5-mini"),
            JobKind::Conversation,
            None,
        )
        .unwrap();
        assert_eq!(
            (ok.agent, ok.model.as_deref()),
            (Some(AgentKind::Codex), Some("gpt-5.5-mini"))
        );
    }

    #[test]
    fn a_model_named_alone_goes_to_an_agent_that_has_it_else_the_default() {
        let f = fixture();
        let open = CfoSettings::default();
        let inp = inputs(&f, &open, &BOTH, Some(AgentKind::Claude));
        // Only Codex lists gpt-5.5: never Claude with a Codex model.
        let codex = choose(&inp, None, Some("gpt-5.5"), JobKind::Conversation, None).unwrap();
        assert_eq!(
            (codex.agent, codex.model.as_deref()),
            (Some(AgentKind::Codex), Some("gpt-5.5"))
        );
        // A model no catalog lists (a custom id) runs on the default agent.
        let custom = choose(
            &inp,
            None,
            Some("my-proxy-model"),
            JobKind::Conversation,
            None,
        )
        .unwrap();
        assert_eq!(custom.agent, Some(AgentKind::Claude));
        assert_eq!(custom.reason, "the default agent");
    }

    #[test]
    fn an_empty_allowlist_keeps_the_default_agent_and_reviews_still_move() {
        let mut f = fixture();
        f.load = BTreeMap::from([("claude".to_string(), 5usize)]);
        let open = CfoSettings::default();
        let inp = inputs(&f, &open, &BOTH, Some(AgentKind::Claude));
        // No least-busy spreading without a list: the old default applies.
        let plain = choose(&inp, None, None, JobKind::Code, None).unwrap();
        assert_eq!((plain.agent, plain.model), (None, None));
        let review = choose(&inp, None, None, JobKind::Review, Some(AgentKind::Claude)).unwrap();
        assert_eq!(review.agent, Some(AgentKind::Codex));
        assert!(
            review.reason.contains("independent of Claude"),
            "{}",
            review.reason
        );
        // The default isn't the producer: it reviews as before.
        let other = choose(&inp, None, None, JobKind::Review, Some(AgentKind::Codex)).unwrap();
        assert_eq!(other.agent, None);
    }

    #[test]
    fn a_reviewer_is_placed_on_another_agent_than_its_producer() {
        let f = fixture();
        let settings = CfoSettings {
            allow: vec![
                AllowEntry {
                    agent: "claude".into(),
                    model: None,
                },
                AllowEntry {
                    agent: "codex".into(),
                    model: None,
                },
            ],
            ..CfoSettings::default()
        };
        let inp = inputs(&f, &settings, &BOTH, Some(AgentKind::Claude));
        let review = choose(&inp, None, None, JobKind::Review, Some(AgentKind::Claude)).unwrap();
        assert_eq!(review.agent, Some(AgentKind::Codex));
        assert!(review.reason.contains("independent"), "{}", review.reason);
        // With one agent installed, it reviews its own work rather than none.
        let alone = inputs(&f, &settings, &[AgentKind::Claude], None);
        let own = choose(&alone, None, None, JobKind::Review, Some(AgentKind::Claude)).unwrap();
        assert_eq!(own.agent, Some(AgentKind::Claude));
    }

    #[test]
    fn a_list_prefers_the_default_then_spreads_by_load() {
        let mut f = fixture();
        let settings = CfoSettings {
            allow: vec![
                AllowEntry {
                    agent: "claude".into(),
                    model: None,
                },
                AllowEntry {
                    agent: "codex".into(),
                    model: None,
                },
            ],
            ..CfoSettings::default()
        };
        {
            let inp = inputs(&f, &settings, &BOTH, Some(AgentKind::Codex));
            let first = choose(&inp, None, None, JobKind::Code, None).unwrap();
            assert_eq!(first.agent, Some(AgentKind::Codex));
        }
        f.load = BTreeMap::from([("codex".to_string(), 2usize)]);
        let inp = inputs(&f, &settings, &BOTH, Some(AgentKind::Codex));
        let spread = choose(&inp, Some("auto"), None, JobKind::Code, None).unwrap();
        assert_eq!(spread.agent, Some(AgentKind::Claude));
        assert_eq!(spread.reason, "least busy allowed agent");
    }
}
