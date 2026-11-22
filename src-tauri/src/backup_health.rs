//! Conservative, read-only backup posture diagnostics.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupSignal {
    pub id: String,
    pub label: String,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BackupHealth {
    pub status: String,
    pub summary: String,
    pub signals: Vec<BackupSignal>,
}

pub fn inspect(vault: &Path, managed_elsewhere: bool) -> BackupHealth {
    let signals = vec![
        cloud_signal(vault),
        git_signal(vault),
        time_machine_signal(),
        BackupSignal {
            id: "managed".into(),
            label: "Managed elsewhere".into(),
            state: if managed_elsewhere {
                "confirmed"
            } else {
                "missing"
            }
            .into(),
            detail: if managed_elsewhere {
                "You marked this vault as protected by another backup system on this Mac."
            } else {
                "No machine-local acknowledgement of another backup system."
            }
            .into(),
        },
    ];
    classify(signals)
}

/// The advice a user can act on, not a description of the check. Each arm names
/// the winning signal so the sentence is about their machine, and every
/// protected arm still says plainly that no restore was tested — Hollow only
/// ever observes evidence, it never proves recoverability.
fn classify(signals: Vec<BackupSignal>) -> BackupHealth {
    let pick = |state: &str| -> Option<&BackupSignal> { signals.iter().find(|s| s.state == state) };
    let unavailable = signals.iter().filter(|s| s.state == "unavailable").count();

    const ADVICE: &str = "Turn on Time Machine, or mark it handled elsewhere.";
    let (status, summary): (&str, String) = if let Some(win) = pick("confirmed") {
        let subject = match win.id.as_str() {
            "time-machine" => "Time Machine is backing up your vault.",
            "git" => "A git remote holds your latest work.",
            "managed" => "You marked this vault as handled by another backup system.",
            _ => "A backup was found.",
        };
        (
            "protected",
            format!("{subject} Hollow has not tested a restore."),
        )
    } else if let Some(win) = pick("partial") {
        let subject = match win.id.as_str() {
            "cloud-path" => "Cloud sync is not a backup.",
            "git" => "A git remote exists but does not hold your latest work.",
            _ => "What Hollow found is not proof of a backup.",
        };
        ("partial", format!("{subject} {ADVICE}"))
    } else if unavailable == signals.len() {
        (
            "unavailable",
            "Could not check your backup. Retry.".to_string(),
        )
    } else {
        ("unknown", format!("No backup found. {ADVICE}"))
    };
    BackupHealth {
        status: status.into(),
        summary,
        signals,
    }
}

fn cloud_signal(vault: &Path) -> BackupSignal {
    let lower = vault.to_string_lossy().to_lowercase();
    let provider = if lower.contains("mobile documents") || lower.contains("icloud") {
        Some("iCloud Drive")
    } else if lower.contains("onedrive") {
        Some("OneDrive")
    } else if lower.contains("dropbox") {
        Some("Dropbox")
    } else {
        None
    };
    BackupSignal {
        id: "cloud-path".into(),
        label: "Cloud-synced location".into(),
        state: if provider.is_some() {
            "partial"
        } else {
            "missing"
        }
        .into(),
        detail: provider
            .map(|name| {
                format!("The vault is under a {name}-looking path; sync success is not verified.")
            })
            .unwrap_or_else(|| {
                "The vault is not under a recognized iCloud, OneDrive, or Dropbox root.".into()
            }),
    }
}

fn git_signal(vault: &Path) -> BackupSignal {
    let remote = Command::new("git")
        .args(["-C", &vault.to_string_lossy(), "remote"])
        .output();
    let Ok(remote) = remote else {
        return BackupSignal {
            id: "git".into(),
            label: "Git remote".into(),
            state: "unavailable".into(),
            detail: "Git is unavailable, so repository backup posture could not be checked.".into(),
        };
    };
    if !remote.status.success() || String::from_utf8_lossy(&remote.stdout).trim().is_empty() {
        return BackupSignal {
            id: "git".into(),
            label: "Git remote".into(),
            state: "missing".into(),
            detail: "No enclosing repository with a configured remote was found.".into(),
        };
    }
    let last = Command::new("git")
        .args(["-C", &vault.to_string_lossy(), "log", "-1", "--format=%cI"])
        .output();
    let recent = last.ok().and_then(|out| {
        if !out.status.success() {
            return None;
        }
        DateTime::parse_from_rfc3339(String::from_utf8_lossy(&out.stdout).trim())
            .ok()
            .map(|at| at.with_timezone(&Utc) >= Utc::now() - Duration::days(7))
    });
    let remote_contains_head = Command::new("git")
        .args([
            "-C",
            &vault.to_string_lossy(),
            "for-each-ref",
            "--contains",
            "HEAD",
            "--format=%(refname)",
            "refs/remotes",
        ])
        .output()
        .ok()
        .is_some_and(|out| {
            out.status.success() && !String::from_utf8_lossy(&out.stdout).trim().is_empty()
        });
    let confirmed = recent == Some(true) && remote_contains_head;
    BackupSignal {
        id: "git".into(),
        label: "Git remote".into(),
        state: if confirmed {
            "confirmed"
        } else {
            "partial"
        }
        .into(),
        detail: if confirmed {
            "A remote-tracking ref contains HEAD and the latest commit is less than seven days old."
        } else if recent == Some(true) {
            "A remote exists and the latest commit is recent, but no remote-tracking ref is known to contain HEAD."
        } else {
            "A remote exists, but the latest commit is not recent enough to count as current protection."
        }
        .into(),
    }
}

fn time_machine_signal() -> BackupSignal {
    match Command::new("tmutil").arg("destinationinfo").output() {
        Err(_) => BackupSignal {
            id: "time-machine".into(),
            label: "Time Machine".into(),
            state: "unavailable".into(),
            detail: "Time Machine configuration could not be inspected.".into(),
        },
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let configured = out.status.success()
                && !text.to_lowercase().contains("no destinations configured")
                && (text.contains("Name") || text.contains("ID"));
            BackupSignal {
                id: "time-machine".into(),
                label: "Time Machine".into(),
                state: if configured { "confirmed" } else { "missing" }.into(),
                detail: if configured {
                    "macOS reports a configured Time Machine destination. Destination details stay private."
                } else {
                    "macOS does not report a configured Time Machine destination."
                }
                .into(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signal(id: &str, state: &str) -> BackupSignal {
        BackupSignal {
            id: id.into(),
            label: id.into(),
            state: state.into(),
            detail: String::new(),
        }
    }

    #[test]
    fn cloud_path_alone_is_partial_never_protected() {
        let health = classify(vec![signal("cloud", "partial"), signal("git", "missing")]);
        assert_eq!(health.status, "partial");
    }

    #[test]
    fn one_explicit_signal_is_protected_but_restore_is_not_claimed() {
        let health = classify(vec![signal("tm", "confirmed"), signal("cloud", "partial")]);
        assert_eq!(health.status, "protected");
        // The wording changed to advice, but the honesty requirement did not:
        // no protected arm may imply Hollow verified recoverability.
        assert!(health.summary.contains("has not tested a restore"));
    }

    #[test]
    fn unprotected_states_tell_the_user_what_to_do() {
        let cloud = classify(vec![signal("cloud-path", "partial")]);
        assert_eq!(cloud.status, "partial");
        assert!(
            cloud.summary.contains("Cloud sync is not a backup"),
            "{}",
            cloud.summary
        );
        assert!(
            cloud.summary.contains("Turn on Time Machine"),
            "{}",
            cloud.summary
        );

        let none = classify(vec![signal("git", "missing")]);
        assert!(
            none.summary.contains("Turn on Time Machine"),
            "{}",
            none.summary
        );
    }

    #[test]
    fn no_signals_is_unknown_and_all_unavailable_is_unavailable() {
        assert_eq!(classify(vec![signal("git", "missing")]).status, "unknown");
        assert_eq!(
            classify(vec![signal("git", "unavailable")]).status,
            "unavailable"
        );
    }
}
