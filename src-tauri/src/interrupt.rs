//! One interrupt record for everything that stops a run and asks the person
//! (AG3, 2026-11-17; Hollow's own design, after AG-UI's interrupt and resume
//! vocabulary). Spec and fixtures: `docs/reference/interrupts.md`,
//! `fixtures/interactions/interrupt-*.json`.
//!
//! Question cards (`interactions.rs`, `interaction_schema.rs`) and approval
//! cards (`approvals.rs`) keep their own stores and wire events; this module
//! is the shared shape both map to, so the phone approvals design, the
//! waiting list and any later answer route speak one language. Nothing here
//! stores, shows or answers a card: a pure mapping, so it cannot approve
//! anything. Reading an answer is deny-by-default ([`InterruptAnswer::approved`]).
//!
//! Hollow's rule differs from AG-UI's "pending interrupts block new input": a
//! reply supersedes an open question card (`supersedeOnUserMessage`, default
//! on), and an approval card is closed by its turn ending, never by a reply.

// No caller yet outside the tests: the phone approvals row wires `waiting`
// into `GET /v1/waiting` (wire.rs is in other PRs' hands).
#![allow(dead_code)]

use crate::approvals::PendingCard;
use crate::types::ChatInteraction;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Reason of a question card.
pub const REASON_QUESTION: &str = "hollow:question";
/// Reason of an approval card for a change inside Hollow or a computer verb.
pub const REASON_APPROVAL: &str = "hollow:approval";
/// Reason of an approval card for an action that leaves this Mac.
pub const REASON_EXTERNAL: &str = "hollow:external";

/// What is being asked and what answers it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interrupt {
    /// Unique across kinds and launches: a question card's id, or an
    /// approval's `apr-<launch>-<n>`.
    pub id: String,
    pub reason: String,
    /// What the person reads.
    pub message: String,
    /// The agent tool call it belongs to, when bound.
    pub tool_call_id: Option<String>,
    /// JSON Schema of the answer's `payload`.
    pub response_schema: Value,
    /// When it stops waiting (RFC 3339), if it does.
    pub expires_at: Option<String>,
    pub metadata: Value,
}

/// How an interrupt ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Resolved,
    Cancelled,
}

/// The next input's answer to one interrupt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterruptAnswer {
    pub interrupt_id: String,
    pub status: Status,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub metadata: Value,
}

impl InterruptAnswer {
    /// An approval answer: `resolved` with the verdict, plus the words typed
    /// on a Decline.
    pub fn approval(id: &str, approved: bool, reason: Option<&str>) -> Self {
        let mut payload = json!({ "approved": approved });
        if let (false, Some(why)) = (approved, reason) {
            payload["reason"] = json!(why);
        }
        Self {
            interrupt_id: id.to_string(),
            status: Status::Resolved,
            payload,
            metadata: Value::Null,
        }
    }

    /// An interrupt that ended with no answer (timed out, turn stopped).
    pub fn cancelled(id: &str) -> Self {
        Self {
            interrupt_id: id.to_string(),
            status: Status::Cancelled,
            payload: Value::Null,
            metadata: Value::Null,
        }
    }

    /// Whether this answers an approval with an Allow. Deny by default:
    /// only `resolved` with `payload.approved == true` is a yes; a cancelled
    /// answer, a missing or non-boolean field, a string `"true"` are all no.
    pub fn approved(&self) -> bool {
        self.status == Status::Resolved && self.payload.get("approved") == Some(&Value::Bool(true))
    }

    /// The typed Decline reason, cleaned, when there is one.
    pub fn decline_reason(&self) -> Option<String> {
        self.payload
            .get("reason")
            .and_then(Value::as_str)
            .and_then(crate::approvals::clean_reason)
    }
}

/// The answer schema of an approval card.
fn approval_schema() -> Value {
    json!({
        "type": "object",
        "required": ["approved"],
        "properties": {
            "approved": { "type": "boolean" },
            "reason": { "type": "string", "maxLength": 500,
                "description": "Words typed on a Decline; sent to the agent." }
        },
        "additionalProperties": false
    })
}

/// An approval card as an interrupt.
pub(crate) fn from_approval(card: &PendingCard) -> Interrupt {
    let external = card.kind == "app-external";
    let mut metadata = json!({
        "kind": card.kind,
        "title": card.title,
        "sessionKey": card.session_key,
        "shownAt": card.at,
    });
    if card.action.is_empty() {
        metadata["action"] = Value::Null;
    }
    Interrupt {
        id: card.id.clone(),
        reason: if external {
            REASON_EXTERNAL
        } else {
            REASON_APPROVAL
        }
        .into(),
        message: card.action.clone(),
        tool_call_id: card.tool_use_id.clone(),
        response_schema: approval_schema(),
        expires_at: Some(card.expires_at.clone()).filter(|e| !e.is_empty()),
        metadata,
    }
}

/// An open question card as an interrupt. Its request is the response
/// schema: the answer's `payload` is the card's `interaction_schema::Answer`.
pub fn from_interaction(card: &ChatInteraction) -> Interrupt {
    let text = |key: &str| {
        card.request
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Interrupt {
        id: card.id.clone(),
        reason: REASON_QUESTION.into(),
        message: text("title")
            .or_else(|| text("description"))
            .unwrap_or_else(|| "A question for you".into()),
        tool_call_id: Some(card.tool_use_id.clone()).filter(|t| !t.is_empty()),
        response_schema: card.request.clone(),
        expires_at: None,
        metadata: json!({
            "mode": card.mode,
            "sessionKey": card.session_key,
            "status": card.status,
            "createdAt": card.created_at,
            "supersedeOnUserMessage": card.supersede_on_user_message,
        }),
    }
}

/// Everything waiting on the person across both kinds, oldest first by the
/// time each was shown. The one list a phone `GET /v1/waiting` can serve
/// once approvals are answerable there.
pub fn waiting(vault: &std::path::Path) -> Result<Vec<Interrupt>, String> {
    let mut out: Vec<(String, Interrupt)> = crate::approvals::pending_cards()
        .iter()
        .map(|c| (c.at.clone(), from_approval(c)))
        .collect();
    for card in crate::interactions::list_open_every(vault)? {
        if card.status == "pending" {
            out.push((card.created_at.clone(), from_interaction(&card)));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Ok(out.into_iter().map(|(_, i)| i).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        let raw = match name {
            "question" => include_str!("../fixtures/interactions/interrupt-question.json"),
            "approval" => include_str!("../fixtures/interactions/interrupt-approval.json"),
            "answer-allow" => include_str!("../fixtures/interactions/interrupt-answer-allow.json"),
            "answer-decline" => {
                include_str!("../fixtures/interactions/interrupt-answer-decline.json")
            }
            "answer-cancelled" => {
                include_str!("../fixtures/interactions/interrupt-answer-cancelled.json")
            }
            other => panic!("no fixture {other}"),
        };
        serde_json::from_str(raw).unwrap()
    }

    fn approval_card() -> PendingCard {
        PendingCard {
            id: "apr-19a7c2f1b3e-1f4a-3".into(),
            title: "Hollow app".into(),
            action: "Publish the build to the beta channel".into(),
            kind: "app-external".into(),
            session_key: Some("s-20261117-abc12345".into()),
            tool_use_id: Some("toolu-007".into()),
            at: "2026-11-17T21:00:00+00:00".into(),
            expires_at: "2026-11-17T21:02:00+00:00".into(),
            routed_to_coo: false,
        }
    }

    fn question_card() -> ChatInteraction {
        ChatInteraction {
            id: "int-1".into(),
            session_key: "s-20261117-abc12345".into(),
            turn_ord: 2,
            run_id: "run-1".into(),
            agent: "claude".into(),
            native_session_id: None,
            tool_use_id: "toolu-009".into(),
            source: "mcp".into(),
            mode: "questions".into(),
            request: json!({
                "mode": "questions",
                "title": "Which channel?",
                "supersedeOnUserMessage": true,
                "questions": [{ "id": "channel", "prompt": "Ship to which channel?" }]
            }),
            target_path: None,
            supersede_on_user_message: true,
            status: "pending".into(),
            status_reason: None,
            superseded_by: None,
            answer: None,
            created_at: "2026-11-17T20:59:00Z".into(),
            resolved_at: None,
        }
    }

    #[test]
    fn an_approval_card_maps_to_the_fixture() {
        let got = serde_json::to_value(from_approval(&approval_card())).unwrap();
        assert_eq!(got, fixture("approval"));
    }

    #[test]
    fn a_question_card_maps_to_the_fixture() {
        let got = serde_json::to_value(from_interaction(&question_card())).unwrap();
        assert_eq!(got, fixture("question"));
    }

    #[test]
    fn answers_match_their_fixtures() {
        let id = "apr-19a7c2f1b3e-1f4a-3";
        let allow = InterruptAnswer::approval(id, true, Some("ignored on an allow"));
        assert_eq!(
            serde_json::to_value(&allow).unwrap(),
            fixture("answer-allow")
        );
        let decline = InterruptAnswer::approval(id, false, Some("Use the staging channel"));
        assert_eq!(
            serde_json::to_value(&decline).unwrap(),
            fixture("answer-decline")
        );
        assert_eq!(
            serde_json::to_value(InterruptAnswer::cancelled(id)).unwrap(),
            fixture("answer-cancelled")
        );
        // The fixtures read back as the same records.
        for name in ["answer-allow", "answer-decline", "answer-cancelled"] {
            let parsed: InterruptAnswer = serde_json::from_value(fixture(name)).unwrap();
            assert_eq!(serde_json::to_value(&parsed).unwrap(), fixture(name));
        }
    }

    #[test]
    fn only_a_resolved_boolean_true_is_a_yes() {
        let id = "apr-x-1";
        assert!(InterruptAnswer::approval(id, true, None).approved());
        assert!(!InterruptAnswer::approval(id, false, None).approved());
        assert!(!InterruptAnswer::cancelled(id).approved());
        for payload in [
            json!({}),
            json!({"approved": "true"}),
            json!({"approved": 1}),
            Value::Null,
        ] {
            let a = InterruptAnswer {
                interrupt_id: id.into(),
                status: Status::Resolved,
                payload,
                metadata: Value::Null,
            };
            assert!(!a.approved());
        }
        let cancelled_yes = InterruptAnswer {
            interrupt_id: id.into(),
            status: Status::Cancelled,
            payload: json!({"approved": true}),
            metadata: Value::Null,
        };
        assert!(
            !cancelled_yes.approved(),
            "a cancelled answer never approves"
        );
    }

    #[test]
    fn an_approval_from_a_card_with_no_expiry_has_none() {
        let mut card = approval_card();
        card.expires_at = String::new();
        card.kind = "computer".into();
        let i = from_approval(&card);
        assert_eq!(i.expires_at, None);
        assert_eq!(i.reason, REASON_APPROVAL);
    }
}
