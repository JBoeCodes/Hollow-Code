//! The effort each job kind runs at (K2, approved 11-10) and where a turn's
//! effort comes from. The level is a request: `effort::resolve` maps it per
//! model downstream as it always has, so an agent or model without that
//! level gets the nearest one it has.

use super::job::JobKind;
use super::settings::EffortMode;
use serde::{Deserialize, Serialize};

/// The level the CFO picks for a job kind.
pub fn level_for(kind: JobKind) -> &'static str {
    match kind {
        JobKind::Conversation | JobKind::Question => "low",
        JobKind::Report | JobKind::Review => "medium",
        JobKind::Research | JobKind::Plan | JobKind::Code => "high",
    }
}

/// Who set a turn's effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[cfg_attr(test, ts(rename = "CfoEffortSource"))]
#[serde(rename_all = "camelCase")]
pub enum EffortSource {
    /// The person's per-chat choice.
    You,
    /// An agent's explicit `start_session` / `send_to_session` effort.
    Agent,
    /// The CFO, by job kind (Automatic).
    Cfo,
    /// The agent's saved level (Fixed).
    Fixed,
}

/// The effort for one turn: an explicit request wins (the person's or an
/// agent's); else Automatic picks by job kind and Fixed keeps the saved
/// per-agent level (which may be none: the runtime default).
pub fn choose(
    requested: Option<String>,
    requested_by_person: bool,
    mode: EffortMode,
    kind: JobKind,
    saved: Option<String>,
) -> (Option<String>, EffortSource) {
    if let Some(level) = requested.filter(|l| !l.trim().is_empty()) {
        let source = if requested_by_person {
            EffortSource::You
        } else {
            EffortSource::Agent
        };
        return (Some(level), source);
    }
    match mode {
        EffortMode::Automatic => (Some(level_for(kind).to_string()), EffortSource::Cfo),
        EffortMode::Fixed => (saved, EffortSource::Fixed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_kind_has_its_level() {
        assert_eq!(level_for(JobKind::Conversation), "low");
        assert_eq!(level_for(JobKind::Question), "low");
        assert_eq!(level_for(JobKind::Report), "medium");
        assert_eq!(level_for(JobKind::Review), "medium");
        assert_eq!(level_for(JobKind::Research), "high");
        assert_eq!(level_for(JobKind::Plan), "high");
        assert_eq!(level_for(JobKind::Code), "high");
        for kind in [JobKind::Code, JobKind::Question] {
            assert!(crate::effort::LEVELS.contains(&level_for(kind)));
        }
    }

    #[test]
    fn the_persons_choice_wins_then_the_mode_decides() {
        let saved = Some("xhigh".to_string());
        assert_eq!(
            choose(
                Some("max".into()),
                true,
                EffortMode::Automatic,
                JobKind::Question,
                saved.clone()
            ),
            (Some("max".into()), EffortSource::You)
        );
        assert_eq!(
            choose(
                Some("low".into()),
                false,
                EffortMode::Fixed,
                JobKind::Code,
                saved.clone()
            ),
            (Some("low".into()), EffortSource::Agent)
        );
        assert_eq!(
            choose(
                None,
                true,
                EffortMode::Automatic,
                JobKind::Code,
                saved.clone()
            ),
            (Some("high".into()), EffortSource::Cfo)
        );
        assert_eq!(
            choose(None, true, EffortMode::Fixed, JobKind::Code, saved.clone()),
            (saved, EffortSource::Fixed)
        );
        assert_eq!(
            choose(None, true, EffortMode::Fixed, JobKind::Code, None),
            (None, EffortSource::Fixed)
        );
    }
}
