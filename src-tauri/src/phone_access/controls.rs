//! Per-send phone choices. No app-wide preference writes.
use super::routes::{self, ApiError};
use crate::agent::AgentKind;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Selection {
    pub agent: Option<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub permission_mode: Option<String>,
}

impl Selection {
    pub(crate) fn validate(&self) -> Result<(), ApiError> {
        self.agent_kind()?;
        if self.model.as_deref().is_some_and(|m| {
            m.len() > 200 || m.starts_with('-')
                || m.chars().any(|c| c.is_whitespace() || c.is_control()
                    || matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'))
        }) {
            return Err(ApiError::bad_request(
                "Model IDs must be at most 200 bytes, without spaces, control characters or a leading dash.",
            ));
        }
        if self
            .effort
            .as_deref()
            .is_some_and(|e| !crate::effort::LEVELS.contains(&e))
        {
            return Err(ApiError::bad_request(
                "Choose low, medium, high, xhigh or max effort.",
            ));
        }
        if self
            .permission_mode
            .as_deref()
            .is_some_and(|m| !routes::WINDOW_MODES.contains(&m))
        {
            return Err(ApiError::bad_request(
                "Choose a permission mode offered by Hollow.",
            ));
        }
        Ok(())
    }

    pub(crate) fn agent_kind(&self) -> Result<Option<AgentKind>, ApiError> {
        self.agent
            .as_deref()
            .map(|id| {
                AgentKind::from_opt(id)
                    .ok_or_else(|| ApiError::bad_request("That agent isn't supported by Hollow."))
            })
            .transpose()
    }

    pub(crate) fn permission(
        &self,
        kind: AgentKind,
        inherited: &str,
        ceiling: &str,
    ) -> Result<String, ApiError> {
        let mode = self.permission_mode.as_deref().unwrap_or(inherited);
        if let Some(reason) = crate::agent::unavailable_mode(kind, mode) {
            return Err(ApiError::bad_request(reason));
        }
        if self.permission_mode.is_some() && permission_rank(mode) > permission_rank(ceiling) {
            return Err(ApiError::permission_escalation());
        }
        Ok(mode.to_string())
    }
}

fn permission_rank(mode: &str) -> u8 {
    match crate::app_mcp::normalize_perm(mode) {
        "plan" => 0,
        "auto" => 2,
        "bypassPermissions" => 3,
        _ => 1,
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct AgentOption {
    pub id: String,
    pub label: String,
    pub available: bool,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct PermissionOption {
    pub id: String,
    pub label: String,
    pub available: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Options {
    pub agent: String,
    pub agents: Vec<AgentOption>,
    pub models: Vec<crate::agent_models::ModelOption>,
    pub source: String,
    pub warning: Option<String>,
    pub default_model: Option<String>,
    pub default_effort: Option<String>,
    pub effort_levels: Vec<String>,
    pub permission_modes: Vec<PermissionOption>,
}

pub(crate) fn permissions(kind: AgentKind) -> Vec<PermissionOption> {
    [
        ("plan", "Plan"),
        ("acceptEdits", "Edits only"),
        ("auto", "Auto"),
        ("bypassPermissions", "Bypass all (unrestricted)"),
    ]
    .into_iter()
    .map(|(id, label)| {
        let reason = crate::agent::unavailable_mode(kind, id).map(str::to_string);
        PermissionOption {
            id: id.into(),
            label: label.into(),
            available: reason.is_none(),
            reason,
        }
    })
    .collect()
}

pub(crate) async fn catalog(asked: Option<AgentKind>) -> Result<Options, ApiError> {
    let (kind, settings, agents) = routes::blocking(move || {
        let (kind, _) = crate::agent_settings::resolve(asked.map(|k| k.as_str()), None)
            .map_err(ApiError::bad_request)?;
        let settings = crate::agent_settings::get_settings().map_err(ApiError::unavailable)?;
        let agents = crate::agent::descriptors()
            .into_iter()
            .map(|a| AgentOption {
                id: a.id,
                label: a.label,
                available: a.available,
            })
            .collect();
        Ok((kind, settings, agents))
    })
    .await?;
    let catalog = crate::agent_models::catalog(kind.as_str(), false).await;
    Ok(Options {
        agent: kind.as_str().into(),
        agents,
        models: catalog.models,
        source: catalog.source,
        warning: catalog.warning,
        default_model: settings.models.get(kind.as_str()).cloned(),
        default_effort: settings.efforts.get(kind.as_str()).cloned(),
        effort_levels: crate::effort::LEVELS.iter().map(|s| (*s).into()).collect(),
        permission_modes: permissions(kind),
    })
}
