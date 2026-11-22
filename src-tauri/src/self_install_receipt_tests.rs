//! The install-result read follows `last-result.json` (merged plan slice 6,
//! review B·F02). The helper-script tests went with the script (row U10). Kept apart from `self_install.rs` so the fix there
//! can be reverted without losing the tests; new fields are read through
//! JSON so this file compiles against the old code too.

use crate::self_install::{acknowledge_result, pending_result, receipt_in};

fn running(commit: &str) -> crate::app_menu::BuildIdentity {
    crate::app_menu::parse_identity(&format!(r#"{{"commit":"{commit}"}}"#), "0.0.0")
}

#[test]
fn the_install_result_read_follows_the_file_not_a_launch_snapshot() {
    // The open row "install-result action returns null despite a matching
    // success receipt": the read was a snapshot taken at launch, so a
    // result written after the app started read as null.
    let dir = crate::agent_settings::app_data_dir()
        .unwrap()
        .join("self-install");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("last-result.json"), r#"{"id":"r1","ok":true,"finishedAt":"t","commit":"63c79a88","installer":{"ok":true,"installed":"yes"}}"#).unwrap();
    assert_eq!(pending_result().map(|r| r.id).as_deref(), Some("r1"));
    // The helper rewrites it after the reopen check: the read sees the rewrite.
    std::fs::write(dir.join("last-result.json"), r#"{"id":"r1","ok":true,"finishedAt":"t","commit":"63c79a88","message":"","installer":{"ok":true,"installed":"yes","runningMatches":false,"runningReason":"still the old app"}}"#).unwrap();
    assert_eq!(
        pending_result().map(|r| r.message).as_deref(),
        Some("still the old app")
    );
    // Dismissed: null, and the receipt still holds it.
    acknowledge_result().unwrap();
    assert!(pending_result().is_none());
    let receipt = receipt_in(&dir, &running("63c79a889a21")).unwrap();
    assert!(
        receipt.acknowledged && receipt.matches_running,
        "{receipt:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
