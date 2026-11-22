//! Claude plan-usage command adapter (Paperclip PR 8c). Thin: the sources,
//! the Keychain read and the hide rule live in `claude_quota`.

use crate::claude_quota::ClaudeQuota;

/// Settings, Usage "Claude plan" card. Always `Ok`: `None` hides the card
/// (not a subscription login, any Keychain or endpoint failure, or nothing
/// observed yet off macOS). `force` is the card's Refresh button, which may
/// ask the usage endpoint at most once a minute.
#[tauri::command]
pub async fn claude_quota(force: bool) -> Result<Option<ClaudeQuota>, String> {
    Ok(crate::claude_quota::get(force).await)
}
