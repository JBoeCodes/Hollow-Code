//! Passive, memory-only plan readings, owned by the login that launched
//! the run. No credentials or native/provider requests leave this module.
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanWindow {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanLimits {
    pub observed_at: String,
    pub windows: Vec<PlanWindow>,
}

/// A file identity, never its contents. Replacement/log-out invalidates a
/// previous login's reading, including changes made outside Hollow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginStamp {
    path: PathBuf,
    modified: SystemTime,
    len: u64,
}

fn stamp(home: &Path) -> Option<LoginStamp> {
    let path = home.join("auth.json");
    let meta = std::fs::metadata(&path).ok()?;
    Some(LoginStamp {
        path,
        modified: meta.modified().ok()?,
        len: meta.len(),
    })
}

pub fn for_command(command: &tokio::process::Command) -> Option<LoginStamp> {
    if let Some((_, home)) = command
        .as_std()
        .get_envs()
        .find(|(key, _)| *key == "CODEX_HOME")
    {
        return home.and_then(|home| stamp(Path::new(home)));
    }
    super::codex_home().and_then(|home| stamp(&home))
}

#[derive(Default)]
struct Cache {
    codex: Option<(LoginStamp, PlanLimits)>,
}
static CACHE: Mutex<Cache> = Mutex::new(Cache { codex: None });

impl Cache {
    fn read(&self, owner: &LoginStamp, now: DateTime<Utc>) -> Option<PlanLimits> {
        let (saved_owner, limits) = self.codex.as_ref()?;
        if saved_owner != owner {
            return None;
        }
        let observed = DateTime::parse_from_rfc3339(&limits.observed_at).ok()?;
        let age = now.signed_duration_since(observed).num_seconds();
        if !(0..=900).contains(&age) {
            return None;
        }
        let mut current = limits.clone();
        current.windows.retain(|window| {
            window
                .resets_at
                .as_ref()
                .is_none_or(|reset| DateTime::parse_from_rfc3339(reset).is_ok_and(|at| at > now))
        });
        (!current.windows.is_empty()).then_some(current)
    }
}

pub fn observe(owner: &LoginStamp, limits: &PlanLimits) {
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .codex = Some((owner.clone(), limits.clone()));
}

pub fn latest() -> Option<PlanLimits> {
    let owner = super::codex_home().and_then(|home| stamp(&home))?;
    CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .read(&owner, Utc::now())
}

/// The optional account/rateLimits/updated payload. Missing/invalid windows
/// stay unknown; counters are never inferred from plan names or token use.
pub fn parse(value: &Value) -> Option<PlanLimits> {
    let windows: Vec<_> = ["primary", "secondary"]
        .into_iter()
        .filter_map(|id| {
            let window = value.get(id)?;
            let used = window.get("usedPercent")?.as_f64()?;
            if !used.is_finite() || used < 0.0 {
                return None;
            }
            let minutes = window.get("windowDurationMins").and_then(Value::as_u64);
            let label = match minutes {
                Some(300) => "5h limit".to_string(),
                Some(10080) => "7d limit".to_string(),
                Some(n) if n > 0 => format!("{n} min limit"),
                _ => format!(
                    "{} limit",
                    if id == "primary" {
                        "Primary"
                    } else {
                        "Secondary"
                    }
                ),
            };
            let resets_at = window
                .get("resetsAt")
                .and_then(Value::as_i64)
                .and_then(|seconds| DateTime::<Utc>::from_timestamp(seconds, 0))
                .map(|at| at.to_rfc3339());
            Some(PlanWindow {
                id: id.into(),
                label,
                used_percent: used,
                resets_at,
            })
        })
        .collect();
    (!windows.is_empty()).then(|| PlanLimits {
        observed_at: Utc::now().to_rfc3339(),
        windows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn unknown_invalid_and_over_limit_readings_stay_honest() {
        assert!(parse(&json!({})).is_none());
        assert!(parse(&json!({"primary": {"usedPercent": -1}})).is_none());
        assert!(parse(&json!({"primary": {"usedPercent": "42"}})).is_none());
        let reading = parse(
            &json!({"primary": {"usedPercent": 120, "windowDurationMins": 300},
            "secondary": {"usedPercent": 0, "windowDurationMins": 10080, "resetsAt": 1800000000}}),
        )
        .unwrap();
        assert_eq!(reading.windows[0].used_percent, 120.0);
        assert_eq!(reading.windows[0].label, "5h limit");
        assert_eq!(reading.windows[1].used_percent, 0.0);
        assert_eq!(reading.windows[1].label, "7d limit");
        assert!(reading.windows[0].resets_at.is_none());
    }
    #[test]
    fn readings_expire_and_never_cross_a_login_or_a_reset() {
        let owner = LoginStamp {
            path: "fixture".into(),
            modified: SystemTime::UNIX_EPOCH,
            len: 12,
        };
        let other = LoginStamp {
            len: 13,
            ..owner.clone()
        };
        let mut limits = parse(&json!({"primary": {"usedPercent": 23}})).unwrap();
        let now = Utc::now();
        limits.observed_at = now.to_rfc3339();
        let mut cache = Cache {
            codex: Some((owner.clone(), limits.clone())),
        };
        assert!(cache.read(&owner, now).is_some());
        assert!(cache.read(&other, now).is_none());
        assert!(cache
            .read(&owner, now + chrono::Duration::minutes(16))
            .is_none());
        limits.windows[0].resets_at = Some((now - chrono::Duration::seconds(1)).to_rfc3339());
        cache.codex = Some((owner.clone(), limits));
        assert!(cache.read(&owner, now).is_none());
    }
}
