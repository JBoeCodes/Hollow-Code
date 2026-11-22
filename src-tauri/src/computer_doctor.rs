//! Computer control's Doctor (Cua CU10): the checks behind the Settings
//! sheet's list, each with a state, a line and a copyable fix. It reads and
//! installs nothing: a fix is words for the person (or for a build run's
//! Needs Owner list), never a command Hollow runs.
//!
//! What it can check today is what lives on this Mac: the switch, the broker,
//! the engine pin, the Driver binary against its pinned SHA-256 (This Mac), the
//! sandbox's key and whether its host answers (sandbox), the deny-list file and
//! free disk space. What needs the Driver running or the VM (the Driver's own
//! permissions and telemetry, Lume, the VM volume) is reported `unknown` and
//! named, never guessed: it is checked once CU0 and CU5 exist.

use crate::computer_broker::{self, EngineConfig, TargetConfig};
use serde::Serialize;
use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: &'static str,
    pub label: &'static str,
    /// `ok`, `warn`, `fail` or `unknown` (not checkable until the Driver or
    /// the VM exists).
    pub state: &'static str,
    pub line: String,
    /// Copyable steps for a person; never run by Hollow.
    pub fix: Option<String>,
}

fn check(
    id: &'static str,
    label: &'static str,
    state: &'static str,
    line: impl Into<String>,
    fix: Option<&str>,
) -> Check {
    Check {
        id,
        label,
        state,
        line: line.into(),
        fix: fix.map(str::to_string),
    }
}

/// What the checks read, so a test can supply it.
pub(crate) struct Inputs<'a> {
    pub enabled: bool,
    pub listening: bool,
    /// Hollow's support folder (holds the pin and the deny-list file).
    pub support_dir: Option<&'a Path>,
    pub free_bytes: Option<u64>,
    /// Whether a host answers on its ssh port.
    pub reach: &'a dyn Fn(&str) -> bool,
    /// What the connector hygiene table (CD1) found missing in the library.
    pub connector_findings: &'a [String],
}

const GIB: u64 = 1 << 30;

pub(crate) fn checks(i: &Inputs<'_>) -> Vec<Check> {
    let mut out = vec![
        if i.enabled {
            check("switch", "Computer control", "ok", "On.", None)
        } else {
            check(
                "switch",
                "Computer control",
                "fail",
                "Off: no desktop tool runs.",
                Some("Turn on Computer control in Settings, Connectors."),
            )
        },
        if i.listening {
            check("broker", "Hollow's desktop link", "ok", "Listening.", None)
        } else {
            check("broker", "Hollow's desktop link", "fail", "Not listening: chats get only the basic verbs.", Some("Restart Hollow. If it stays off, the app log names the socket it could not bind."))
        },
    ];
    out.extend(engine_checks(i));
    out.push(match i.free_bytes {
        None => check(
            "space",
            "Free disk space",
            "unknown",
            "Could not be read.",
            None,
        ),
        Some(b) if b < 2 * GIB => check(
            "space",
            "Free disk space",
            "fail",
            format!(
                "{:.1} GB free: screenshots and the Driver need room.",
                b as f64 / GIB as f64
            ),
            Some(
                "Free some space: Settings, Advanced, Disk space shows what can go, and asks first.",
            ),
        ),
        Some(b) if b < 5 * GIB => check(
            "space",
            "Free disk space",
            "warn",
            format!("{:.1} GB free.", b as f64 / GIB as f64),
            Some(
                "Free some space: Settings, Advanced, Disk space shows what can go, and asks first.",
            ),
        ),
        Some(b) => check(
            "space",
            "Free disk space",
            "ok",
            format!("{:.0} GB free.", b as f64 / GIB as f64),
            None,
        ),
    });
    out.push(policy_check(i.support_dir));
    out.push(if i.connector_findings.is_empty() {
        check("connectors", "Connector hygiene", "ok", "No known connector has telemetry on or runs an unpinned version.", None)
    } else {
        check(
            "connectors",
            "Connector hygiene",
            "warn",
            i.connector_findings.join("; ") + ".",
            Some("Edit the connector in Settings, Connectors: pin its version (name@x.y.z) and add the opt-outs named above."),
        )
    });
    out.push(check("driver-permissions", "Driver permissions", "unknown", "Not checked: Hollow can read what the Driver was granted only once it runs.", Some("In System Settings, Privacy and Security, give CuaDriver (not Hollow) Accessibility and Screen Recording.")));
    out.push(check("telemetry", "Driver telemetry", "unknown", "Not checked until the Driver is installed.", Some("Turn the Driver's telemetry off after installing it (its docs name the setting); Hollow's design keeps it off.")));
    out.push(check("vm", "Sandbox VM", "unknown", "Not checked: the VM, its storage volume and Lume are set up on the Mac mini by a later step.", Some("See the build run's Needs Owner list: the licence call, the consent click in the VM and the storage folder.")));
    out
}

fn engine_checks(i: &Inputs<'_>) -> Vec<Check> {
    let Some(dir) = i.support_dir else {
        return vec![check(
            "pin",
            "Cua Driver",
            "fail",
            "Hollow's support folder isn't available.",
            None,
        )];
    };
    let path = dir.join(computer_broker::ENGINE_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![check(
            "pin",
            "Cua Driver",
            "fail",
            "Not set up: no pinned version.",
            Some("Hollow never installs the Driver itself. Put computer-engine.json (the exact version and SHA-256, and where it runs) in Hollow's support folder; the build run's Needs Owner list has the steps."),
        )];
    };
    let cfg = match EngineConfig::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            return vec![check("pin", "Cua Driver", "fail", format!("The pin file is not valid: {e}."), Some("Fix computer-engine.json: an exact version, a 64 character lowercase SHA-256 and an absolute path or host."))];
        }
    };
    let mut out = vec![check(
        "pin",
        "Cua Driver",
        "ok",
        format!(
            "Pinned {} ({}).",
            cfg.version,
            if matches!(cfg.target, TargetConfig::ThisMac { .. }) {
                "this Mac"
            } else {
                "sandbox Mac"
            }
        ),
        None,
    )];
    match &cfg.target {
        TargetConfig::ThisMac { driver } => out.push(driver_check(driver, &cfg.sha256)),
        TargetConfig::SandboxVm { host, key, .. } => {
            out.push(key_check(key));
            out.push(if (i.reach)(host) {
                check("host", "Sandbox host", "ok", "Answers on its ssh port.", None)
            } else {
                check("host", "Sandbox host", "fail", "Does not answer: the Mac mini or the VM is off or on another network.", Some("Check that the Mac mini is on and reachable from this Mac, and that the sandbox VM is running."))
            });
        }
    }
    out
}

fn driver_check(driver: &Path, pinned_sha: &str) -> Check {
    let fix = "Install CuaDriver.app from the pinned release, then check its hash against computer-engine.json; a new Driver version needs a new pin.";
    if !driver.is_file() {
        return check(
            "driver",
            "Driver on this Mac",
            "fail",
            "The pinned Driver isn't at its path.",
            Some(fix),
        );
    }
    match computer_broker::file_sha256(driver) {
        Ok(sha) if sha == pinned_sha => check("driver", "Driver on this Mac", "ok", "Present and matches the pin.", None),
        Ok(_) => check("driver", "Driver on this Mac", "fail", "Its SHA-256 doesn't match the pin, so Hollow refuses it.", Some("The Driver changed outside Hollow. Reinstall the pinned release, or update the pin in a deliberate step.")),
        Err(e) => check("driver", "Driver on this Mac", "fail", format!("Could not be read: {e}."), Some(fix)),
    }
}

fn key_check(key: &Path) -> Check {
    use std::os::unix::fs::PermissionsExt;
    let fix = "Create the sandbox's own key pair (named for the VM), keep the private key at the pinned path with mode 0600, and bake the public key into the VM image.";
    match std::fs::metadata(key) {
        Err(_) => check(
            "key",
            "Sandbox key",
            "fail",
            "The pinned key file isn't there.",
            Some(fix),
        ),
        Ok(m) if !m.is_file() => check(
            "key",
            "Sandbox key",
            "fail",
            "The pinned key path isn't a file.",
            Some(fix),
        ),
        Ok(m) if m.permissions().mode() & 0o077 != 0 => check(
            "key",
            "Sandbox key",
            "fail",
            "The key is readable by others (ssh refuses it).",
            Some("Run chmod 600 on the key file."),
        ),
        Ok(_) => check(
            "key",
            "Sandbox key",
            "ok",
            "Present, readable only by you.",
            None,
        ),
    }
}

fn policy_check(dir: Option<&Path>) -> Check {
    let Some(dir) = dir else {
        return check(
            "policy",
            "Apps allowed on this Mac",
            "unknown",
            "Could not be read.",
            None,
        );
    };
    let path = dir.join(crate::computer_thismac::POLICY_FILE);
    match std::fs::read_to_string(&path) {
        Err(_) => check("policy", "Apps allowed on this Mac", "ok", "No changes: every default-denied app stays blocked.", None),
        Ok(text) if serde_json::from_str::<serde_json::Value>(&text).is_ok() => {
            check("policy", "Apps allowed on this Mac", "ok", "Your allowances are read.", None)
        }
        Ok(_) => check("policy", "Apps allowed on this Mac", "warn", "The file is unreadable, so nothing extra is allowed.", Some("Open Settings, Connectors, Computer control and allow or block the apps again; that rewrites the file.")),
    }
}

/// A host answers on its ssh port within a second, name lookup included: the
/// lookup and the connect run on a worker thread the caller stops waiting for
/// after the deadline (a resolver can take far longer than a connect, and the
/// Doctor is for when the network is broken).
pub(crate) fn ssh_reachable(host: &str) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let host = host.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("computer-doctor-reach".into())
        .spawn(move || {
            let ok = (host.as_str(), 22u16)
                .to_socket_addrs()
                .ok()
                .and_then(|mut a| a.next())
                .is_some_and(|addr| {
                    TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok()
                });
            let _ = tx.send(ok);
        });
    spawned.is_ok() && rx.recv_timeout(Duration::from_secs(1)).unwrap_or(false)
}

#[cfg(test)]
#[path = "computer_doctor_tests.rs"]
mod tests;
