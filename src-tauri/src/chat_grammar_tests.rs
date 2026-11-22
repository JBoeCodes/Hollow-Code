//! Runs the grammar checker over the corpus in `tests/fixtures/chat-streams/`.

use super::*;
use serde_json::json;
use std::path::PathBuf;

fn corpus() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/chat-streams")
}

#[test]
fn every_fixture_has_the_manifest_verdict() {
    let dir = corpus();
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    let streams = manifest["streams"].as_array().unwrap();
    let mut listed = HashSet::new();
    let mut adapters = HashSet::new();
    for entry in streams {
        let file = entry["file"].as_str().unwrap();
        listed.insert(file.to_string());
        adapters.insert(entry["adapter"].as_str().unwrap().to_string());
        let text = std::fs::read_to_string(dir.join(file)).unwrap();
        let verdict = check_jsonl(&text);
        match entry["expect"].as_str().unwrap() {
            "pass" => assert_eq!(verdict, Ok(()), "{file} should pass"),
            _ => {
                let v = verdict.expect_err(file);
                assert_eq!(v.rule, entry["rule"].as_str().unwrap(), "{file}: {v}");
                assert_eq!(
                    v.line as u64,
                    entry["line"].as_u64().unwrap(),
                    "{file}: {v}"
                );
            }
        }
    }
    for adapter in ["claude", "codex", "antigravity", "grok"] {
        assert!(adapters.contains(adapter), "no fixture for {adapter}");
    }
    // A fixture left out of the manifest would be an unchecked stream.
    for file in std::fs::read_dir(&dir).unwrap() {
        let name = file.unwrap().file_name().to_string_lossy().to_string();
        if name.ends_with(".jsonl") {
            assert!(listed.contains(&name), "{name} is not in manifest.json");
        }
    }
}

#[test]
fn planted_orphan_result_names_the_rule_and_line() {
    let text = std::fs::read_to_string(corpus().join("bad-tool-result-no-use.jsonl")).unwrap();
    let v = check_jsonl(&text).unwrap_err();
    assert_eq!((v.rule, v.line), ("G06", 3));
    assert!(v.to_string().contains("G06 at line 3"), "{v}");
}

#[test]
fn empty_stream_and_mixed_conversation_keys_fail() {
    assert_eq!(check_events(&[]).unwrap_err().rule, "G02");
    let events = vec![
        (1, json!({"type":"runStarted","conversationKey":"a"})),
        (
            2,
            json!({"type":"assistantText","text":"x","conversationKey":"b"}),
        ),
        (3, json!({"type":"done"})),
    ];
    assert_eq!(check_events(&events).unwrap_err().rule, "G09");
}
