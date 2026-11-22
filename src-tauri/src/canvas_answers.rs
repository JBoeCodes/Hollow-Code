//! Which ask-canvas revisions a user message answers (Canvas C4 A3).
//!
//! An ask-canvas's answer reaches the conversation as the person's own
//! message: the readable `Answered canvas "<id>":` lines, then one fenced
//! [`crate::next_step::CANVAS_ANSWER_LANG`] block holding
//! `{"canvasId", "revision", "answers"}` as one line of JSON (A2,
//! `src/canvas/askAnswer.ts`). The chat reads every user message for those
//! blocks to lock the canvas (`answeredCanvases`); the "needs you" query
//! ([`crate::needs_you`]) reads the conversation's stored messages the same
//! way to drop an answered ask-canvas from "waiting on you". So the two never
//! disagree, [`answered`] mirrors `readCanvasAnswers` exactly: the fence rules
//! of `splitHiddenFence` (`src/chatSegments.ts`) and the shape rules of
//! `parseAnswer` / `draftAnswers` (`src/canvasBridge.ts`). Both are tested
//! against `src/canvas/answerBlocks.golden.json`.

use crate::next_step::CANVAS_ANSWER_LANG;
use serde_json::{Map, Value};

/// `LIMITS.draftBytes`: the answers, as compact UTF-8 JSON.
pub(crate) const MAX_ANSWER_BYTES: usize = 16 * 1024;

/// The lines inside each `lang` block of `text` (closed, or left open at the
/// end), joined by `\n`. Exactly `splitHiddenFence`'s blocks: the fence
/// rules are [`crate::next_step::split`]'s, which `strip` shares, so what the
/// lock reads and what a session record drops never disagree.
pub(crate) fn hidden_blocks(text: &str, lang: &str) -> Vec<String> {
    crate::next_step::split(text, &[lang]).1
}

/// `CANVAS_ID`: `^[a-z0-9-]{1,64}$`.
fn canvas_id(id: &str) -> bool {
    (1..=64).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// `DRAFT_FIELD_NAME`: `^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$`.
fn field_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphanumeric())
        && name.len() <= 64
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

/// One ask kit answer value: text, a number, a list of texts, a `{row: text}`
/// map, or null; nothing deeper.
fn answer_value(value: &Value) -> bool {
    match value {
        Value::Null | Value::String(_) | Value::Number(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        Value::Object(rows) => rows.values().all(Value::is_string),
        Value::Bool(_) => false,
    }
}

/// `draftAnswers`: a map of field names to answer values, at most
/// [`MAX_ANSWER_BYTES`] as compact JSON.
pub(crate) fn answers(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    map.iter().all(|(k, v)| field_name(k) && answer_value(v))
        && serde_json::to_vec(map).is_ok_and(|bytes| bytes.len() <= MAX_ANSWER_BYTES)
}

/// A whole number from 1, however JSON wrote it (`2.0` is 2, as in JS).
fn revision(value: &Value) -> Option<u64> {
    if let Some(n) = value.as_u64() {
        return (n >= 1).then_some(n);
    }
    let f = value.as_f64()?;
    (f >= 1.0 && f.fract() == 0.0 && f <= u64::MAX as f64).then_some(f as u64)
}

/// `parseAnswer`: exactly `{canvasId, revision, answers}`.
fn parse_answer(json: &str) -> Option<(String, u64)> {
    let value: Value = serde_json::from_str(json.trim()).ok()?;
    let map: &Map<String, Value> = value.as_object()?;
    if map.len() != 3 {
        return None;
    }
    let id = map.get("canvasId")?.as_str()?;
    let rev = revision(map.get("revision")?)?;
    (canvas_id(id) && answers(map.get("answers")?)).then(|| (id.to_string(), rev))
}

/// Every canvas revision `text` answers, as `(canvas id, revision)`, in
/// order: each valid [`CANVAS_ANSWER_LANG`] block.
pub(crate) fn answered(text: &str) -> Vec<(String, u64)> {
    hidden_blocks(text, CANVAS_ANSWER_LANG)
        .iter()
        .filter_map(|json| parse_answer(json))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOLDEN: &str = include_str!("../../src/canvas/answerBlocks.golden.json");
    const BRIDGE_TS: &str = include_str!("../../src/canvasBridge.ts");

    /// The same golden messages the chat's `readCanvasAnswers` is tested
    /// against: the lock and "waiting on you" read answers identically.
    #[test]
    fn every_golden_message_answers_what_the_chat_reads() {
        let golden: Value = serde_json::from_str(GOLDEN).unwrap();
        let cases = golden["cases"].as_array().unwrap();
        assert!(cases.len() > 15);
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let want: Vec<String> = case["answered"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap().to_string())
                .collect();
            let got: Vec<String> = answered(case["text"].as_str().unwrap())
                .into_iter()
                .map(|(id, rev)| format!("{id}@{rev}"))
                .collect();
            assert_eq!(got, want, "{name}");
        }
    }

    #[test]
    fn the_limits_are_the_bridge_limits() {
        assert!(crate::source_pins::contains(
            BRIDGE_TS,
            "draftBytes: 16 * 1024,"
        ));
        assert_eq!(MAX_ANSWER_BYTES, 16 * 1024);
        assert!(crate::source_pins::contains(
            BRIDGE_TS,
            r"export const CANVAS_ID = /^[a-z0-9-]{1,64}$/;"
        ));
        assert!(crate::source_pins::contains(
            BRIDGE_TS,
            r"export const DRAFT_FIELD_NAME = /^[A-Za-z0-9][A-Za-z0-9_.-]{0,63}$/;"
        ));
    }

    #[test]
    fn a_hidden_block_needs_its_own_language_and_fence_rules() {
        assert_eq!(hidden_blocks("plain", "x"), Vec::<String>::new());
        assert_eq!(hidden_blocks("```x\na\nb\n```", "x"), vec!["a\nb"]);
        assert_eq!(hidden_blocks("``x\na\n``", "x"), Vec::<String>::new());
        assert_eq!(hidden_blocks("```x \na", "x"), vec!["a"]);
        assert_eq!(hidden_blocks("```x\n~~~\na\n```", "x"), vec!["~~~\na"]);
        assert_eq!(
            hidden_blocks("```y\n```x\n```\n```x\nb\n```", "x"),
            vec!["b"]
        );
    }
}
