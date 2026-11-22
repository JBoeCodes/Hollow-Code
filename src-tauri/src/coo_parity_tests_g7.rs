//! The COO's queue for settings the window owns, while Hollow's window is
//! closed (`commands::app_bridge_api::{queue_in, pending_in, take_in,
//! ack_in}`). Tempdir-based: never touches the user's app-data folder.

use crate::commands::app_bridge_api::{ack_in, pending_in, queue_in, take_in, PENDING_FILE};
use serde_json::json;

fn queue_file() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(PENDING_FILE);
    (dir, path)
}

#[test]
fn the_last_write_per_key_wins_and_a_read_sees_it() {
    let (_dir, path) = queue_file();
    assert_eq!(pending_in(&path, "theme"), None);
    queue_in(&path, "theme", json!("dark")).unwrap();
    queue_in(&path, "notifyInApp", json!(false)).unwrap();
    queue_in(&path, "theme", json!("light")).unwrap();
    assert_eq!(pending_in(&path, "theme"), Some(json!("light")));
    assert_eq!(pending_in(&path, "notifyInApp"), Some(json!(false)));
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(file["settings"].as_object().unwrap().len(), 2);
}

/// L13 review: a take clears nothing; entries stay until the webview
/// acknowledges their version, so a reload mid-drain loses nothing, and a
/// value queued again meanwhile outlives the ack of the older one.
#[test]
fn the_queue_survives_until_the_webview_acknowledges_it() {
    let (_dir, path) = queue_file();
    queue_in(&path, "theme", json!("dark")).unwrap();
    queue_in(&path, "sidebarCollapsed", json!(true)).unwrap();
    let taken = take_in(&path);
    assert_eq!(taken["settings"]["theme"], json!("dark"));
    assert_eq!(taken["settings"]["sidebarCollapsed"], json!(true));
    assert_eq!(
        take_in(&path),
        taken,
        "taking again (a reload) sees the same"
    );
    assert!(path.exists(), "nothing is cleared by a take");
    let versions = taken["versions"].as_object().unwrap().clone();
    assert_eq!(versions.len(), 2);
    // A newer theme lands while the webview applies the older one.
    queue_in(&path, "theme", json!("light")).unwrap();
    assert_eq!(
        ack_in(&path, &versions).unwrap(),
        1,
        "only sidebarCollapsed"
    );
    assert_eq!(pending_in(&path, "theme"), Some(json!("light")));
    assert_eq!(pending_in(&path, "sidebarCollapsed"), None);
    // A stale or unknown ack removes nothing.
    assert_eq!(ack_in(&path, &versions).unwrap(), 0);
    let mut unknown = serde_json::Map::new();
    unknown.insert("notifyInApp".into(), json!(1));
    assert_eq!(ack_in(&path, &unknown).unwrap(), 0);
    let newer = take_in(&path)["versions"].as_object().unwrap().clone();
    assert_eq!(ack_in(&path, &newer).unwrap(), 1);
    assert_eq!(take_in(&path)["settings"], json!({}));
}

/// L13 delta review: emptying the queue keeps its counter, so a value queued
/// after a full drain gets a new version and a stale ack of the old one
/// (a reload that acks late) never deletes it.
#[test]
fn versions_never_repeat_after_the_queue_empties() {
    let (_dir, path) = queue_file();
    queue_in(&path, "theme", json!("dark")).unwrap();
    let first = take_in(&path)["versions"].as_object().unwrap().clone();
    assert_eq!(ack_in(&path, &first).unwrap(), 1);
    assert_eq!(take_in(&path)["settings"], json!({}));
    queue_in(&path, "theme", json!("light")).unwrap();
    let second = take_in(&path)["versions"].as_object().unwrap().clone();
    assert!(
        second["theme"].as_u64().unwrap() > first["theme"].as_u64().unwrap(),
        "{first:?} then {second:?}"
    );
    assert_eq!(ack_in(&path, &first).unwrap(), 0, "the stale ack misses");
    assert_eq!(pending_in(&path, "theme"), Some(json!("light")));
    // A damaged file restarts the counter from the clock, above the old one.
    std::fs::write(&path, "not json").unwrap();
    queue_in(&path, "theme", json!("dark")).unwrap();
    let third = take_in(&path)["versions"]["theme"].as_u64().unwrap();
    assert!(third > second["theme"].as_u64().unwrap());
}

/// A queue file from before versions still drains: its entries read as
/// version 0 and a later queue numbers above them.
#[test]
fn an_unversioned_queue_file_is_read_and_acknowledged() {
    let (_dir, path) = queue_file();
    std::fs::write(&path, r#"{"settings":{"theme":"dark"}}"#).unwrap();
    let taken = take_in(&path);
    assert_eq!(taken["versions"]["theme"], json!(0));
    queue_in(&path, "notifyInApp", json!(false)).unwrap();
    assert_eq!(take_in(&path)["versions"]["notifyInApp"], json!(1));
    assert_eq!(
        ack_in(&path, taken["versions"].as_object().unwrap()).unwrap(),
        1
    );
    assert_eq!(pending_in(&path, "theme"), None);
    assert_eq!(pending_in(&path, "notifyInApp"), Some(json!(false)));
}

#[test]
fn a_bad_value_an_unknown_key_a_backend_key_and_a_secret_are_refused() {
    let (_dir, path) = queue_file();
    assert!(queue_in(&path, "notifyInApp", json!("yes")).is_err());
    assert!(queue_in(&path, "no.such.key", json!(true)).is_err());
    let backend = crate::settings_keys::KEYS
        .iter()
        .find(|k| matches!(k.owner, crate::settings_keys::Owner::Backend { .. }))
        .unwrap();
    assert!(queue_in(&path, backend.key, json!(true))
        .unwrap_err()
        .contains("not a window setting"));
    // No UI row is a secret today; the guard still holds the Secret kind out.
    assert!(
        crate::settings_keys::KEYS
            .iter()
            .filter(|k| matches!(k.kind, crate::settings_keys::Type::Secret))
            .all(|k| matches!(k.owner, crate::settings_keys::Owner::Backend { .. })),
        "a secret key is backend-owned, so it can never reach the queue"
    );
    assert!(!path.exists(), "a refusal writes nothing");
}

#[test]
fn a_damaged_file_reads_as_empty_and_is_replaced_by_the_next_queue() {
    let (_dir, path) = queue_file();
    std::fs::write(&path, "not json").unwrap();
    assert_eq!(pending_in(&path, "theme"), None);
    queue_in(&path, "theme", json!("dark")).unwrap();
    assert_eq!(pending_in(&path, "theme"), Some(json!("dark")));
}
