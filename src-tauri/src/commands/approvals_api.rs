//! The inline chat approval card's answer (see `approvals.rs`).

/// `reason` is the optional words typed on a Decline (SA1); it goes back to
/// the agent with the refusal and is ignored on an Allow.
#[tauri::command]
pub fn approval_respond(id: String, approved: bool, reason: Option<String>) -> Result<(), String> {
    crate::approvals::respond_with(&id, approved, reason.as_deref())
}
