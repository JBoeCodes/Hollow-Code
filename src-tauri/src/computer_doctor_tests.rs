//! The Doctor's checks (CU10): each failure case shows its fix, and what
//! needs the Driver or the VM is named, not guessed.

use super::*;
use std::os::unix::fs::PermissionsExt;

const GOOD: u64 = 100 * GIB;

fn run(dir: &Path, enabled: bool, listening: bool, free: Option<u64>, reach: bool) -> Vec<Check> {
    let reach_fn = move |_: &str| reach;
    checks(&Inputs {
        enabled,
        listening,
        support_dir: Some(dir),
        free_bytes: free,
        reach: &reach_fn,
        connector_findings: &[],
    })
}

fn get<'a>(list: &'a [Check], id: &str) -> &'a Check {
    list.iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no {id} in {list:?}"))
}

fn sandbox_pin(dir: &Path, key: &Path) {
    std::fs::write(
        dir.join(computer_broker::ENGINE_FILE),
        serde_json::json!({
            "version": "0.52.1",
            "sha256": "0".repeat(64),
            "target": { "kind": "sandboxVm", "host": "192.168.64.5", "user": "lume", "key": key },
        })
        .to_string(),
    )
    .unwrap();
}

#[test]
fn nothing_is_set_up_on_a_fresh_hollow_and_each_failure_has_its_fix() {
    let dir = tempfile::tempdir().unwrap();
    let list = run(dir.path(), false, false, Some(GOOD), true);
    for id in ["switch", "broker", "pin"] {
        let c = get(&list, id);
        assert_eq!(c.state, "fail", "{id}");
        assert!(
            c.fix.as_deref().is_some_and(|f| f.len() > 20),
            "{id} needs a fix"
        );
    }
    assert!(get(&list, "pin")
        .fix
        .as_deref()
        .unwrap()
        .contains("never installs"));
    assert_eq!(get(&list, "space").state, "ok");
    // What needs the Driver or the VM is named as not checked.
    for id in ["driver-permissions", "telemetry", "vm"] {
        assert_eq!(get(&list, id).state, "unknown", "{id}");
        assert!(get(&list, id).line.starts_with("Not checked"), "{id}");
    }
    let on = run(dir.path(), true, true, Some(GOOD), true);
    assert_eq!(
        (get(&on, "switch").state, get(&on, "broker").state),
        ("ok", "ok")
    );
}

#[test]
fn a_bad_pin_says_why_and_a_good_sandbox_pin_checks_its_key_and_host() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(computer_broker::ENGINE_FILE), "{ not json").unwrap();
    let c = run(dir.path(), true, true, Some(GOOD), true);
    assert!(get(&c, "pin").line.contains("not valid"));

    let key = dir.path().join("vm_key");
    sandbox_pin(dir.path(), &key);
    let missing = run(dir.path(), true, true, Some(GOOD), true);
    assert_eq!(get(&missing, "pin").state, "ok");
    assert!(get(&missing, "pin").line.contains("sandbox Mac"));
    assert_eq!(get(&missing, "key").state, "fail");
    assert!(get(&missing, "key").fix.is_some());

    std::fs::write(&key, "k").unwrap();
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
    let open = run(dir.path(), true, true, Some(GOOD), true);
    assert!(get(&open, "key").line.contains("readable by others"));
    assert!(get(&open, "key")
        .fix
        .as_deref()
        .unwrap()
        .contains("chmod 600"));
    std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o600)).unwrap();
    let ok = run(dir.path(), true, true, Some(GOOD), true);
    assert_eq!(
        (get(&ok, "key").state, get(&ok, "host").state),
        ("ok", "ok")
    );
    let down = run(dir.path(), true, true, Some(GOOD), false);
    assert_eq!(get(&down, "host").state, "fail");
    assert!(get(&down, "host").fix.is_some());
}

#[test]
fn this_macs_driver_is_checked_against_the_pin() {
    let dir = tempfile::tempdir().unwrap();
    let driver = dir.path().join("cua-driver");
    let pin = |sha: &str| {
        std::fs::write(
            dir.path().join(computer_broker::ENGINE_FILE),
            serde_json::json!({
                "version": "0.52.1", "sha256": sha,
                "target": { "kind": "thisMac", "driver": &driver },
            })
            .to_string(),
        )
        .unwrap();
    };
    pin(&"0".repeat(64));
    let none = run(dir.path(), true, true, Some(GOOD), true);
    assert!(get(&none, "driver").line.contains("isn't at its path"));
    std::fs::write(&driver, "binary").unwrap();
    let wrong = run(dir.path(), true, true, Some(GOOD), true);
    assert!(get(&wrong, "driver").line.contains("doesn't match"));
    pin(&computer_broker::file_sha256(&driver).unwrap());
    let ok = run(dir.path(), true, true, Some(GOOD), true);
    assert_eq!(get(&ok, "driver").state, "ok");
    assert!(get(&ok, "pin").line.contains("this Mac"));
}

#[test]
fn low_space_and_an_unreadable_allow_file_warn_with_a_fix() {
    let dir = tempfile::tempdir().unwrap();
    for (free, state) in [
        (Some(GIB), "fail"),
        (Some(3 * GIB), "warn"),
        (Some(50 * GIB), "ok"),
        (None, "unknown"),
    ] {
        let c = run(dir.path(), true, true, free, true);
        assert_eq!(get(&c, "space").state, state, "{free:?}");
        if state == "fail" || state == "warn" {
            assert!(get(&c, "space")
                .fix
                .as_deref()
                .unwrap()
                .contains("Disk space"));
        }
    }
    assert_eq!(
        get(&run(dir.path(), true, true, Some(GOOD), true), "policy").state,
        "ok"
    );
    std::fs::write(
        dir.path().join(crate::computer_thismac::POLICY_FILE),
        "nope",
    )
    .unwrap();
    let c = run(dir.path(), true, true, Some(GOOD), true);
    assert_eq!(get(&c, "policy").state, "warn");
    assert!(get(&c, "policy").fix.is_some());
    // The Doctor reads and writes nothing.
    assert_eq!(
        std::fs::read_to_string(dir.path().join(crate::computer_thismac::POLICY_FILE)).unwrap(),
        "nope"
    );
}

#[test]
fn every_failing_or_warning_check_carries_a_fix_and_the_ids_are_unique() {
    let dir = tempfile::tempdir().unwrap();
    sandbox_pin(dir.path(), &dir.path().join("k"));
    let list = run(dir.path(), false, false, Some(GIB), false);
    let mut ids: Vec<_> = list.iter().map(|c| c.id).collect();
    for c in &list {
        if c.state == "fail" || c.state == "warn" {
            assert!(c.fix.is_some(), "{} has no fix", c.id);
        }
        assert!(!c.line.contains('\u{2014}') && !c.line.contains('\u{2013}'));
    }
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), list.len());
    let json = serde_json::to_value(&list[0]).unwrap();
    assert!(json.get("state").is_some() && json.get("fix").is_some());
}

#[test]
fn nothing_listens_on_a_closed_port_and_a_bad_host_does_not_panic() {
    assert!(!ssh_reachable("256.256.256.256"));
    assert!(!ssh_reachable(""));
}
