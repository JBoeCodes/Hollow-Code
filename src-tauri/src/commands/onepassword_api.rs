//! 1Password as an optional second secret path (t3code parity row 7.5,
//! 2026-11-20; `onepassword.rs`): Settings, Accounts, 1Password.
//!
//! Each command's body is a plain `pub(crate)` function the Hollow app
//! actions call too (`settings.accounts.onepassword_*`), behind the card
//! builders below. Turning the path on and pointing a secret at a reference
//! are confirm actions that still ask in `auto` (secrets class). Nothing here
//! returns a secret: a reference names an item, and the status carries only
//! references, states and fixed messages. `onepassword_check` runs `op`
//! (the person's Check button) and has no agent action.

use crate::app_actions::{Args, Card, Cx};
use crate::onepassword::{self, Status};
use serde_json::Value;

fn arg(a: &Args, key: &str) -> String {
    a.0.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| e.to_string())
}

/// The switch, whether `op` and the desktop app are there, and each slot's
/// reference. Runs nothing.
pub(crate) async fn status() -> Result<Status, String> {
    blocking(onepassword::status).await
}

/// 1Password's switch, whether the command line tool is there, and each
/// secret's reference. Runs nothing.
#[tauri::command]
pub async fn onepassword_status() -> Result<Status, String> {
    status().await
}

/// Ask 1Password whether it answers (`op --version`, `op whoami`); reads no
/// secret. Does nothing while the switch is off.
#[tauri::command]
pub async fn onepassword_check() -> Result<Status, String> {
    blocking(onepassword::check).await
}

/// Turn the 1Password path on or off; the status after.
pub(crate) async fn set_enabled(on: bool) -> Result<Status, String> {
    blocking(move || onepassword::set_enabled(on)).await??;
    status().await
}

/// Turn the 1Password secret path on or off (off by default).
#[tauri::command]
pub async fn onepassword_set_enabled(enabled: bool) -> Result<Status, String> {
    set_enabled(enabled).await
}

/// The slot named `key`, or why it isn't one.
fn slot_label(key: &str) -> Result<String, String> {
    onepassword::slots()
        .into_iter()
        .find(|s| s.key == key)
        .map(|s| s.label)
        .ok_or_else(|| "That isn't a secret 1Password can stand in for. Nothing changed.".into())
}

/// Point secret `key` at `reference`; the status after.
pub(crate) async fn set_reference(key: String, reference: String) -> Result<Status, String> {
    blocking(move || {
        slot_label(&key)?;
        onepassword::set_reference(&key, &reference)
    })
    .await??;
    status().await
}

/// Point a secret at an `op://Vault/Item/field` reference.
#[tauri::command]
pub async fn onepassword_set_reference(key: String, reference: String) -> Result<Status, String> {
    set_reference(key, reference).await
}

/// Forget a secret's reference (its Keychain value is used again); the
/// status after.
pub(crate) async fn clear_reference(key: String) -> Result<Status, String> {
    blocking(move || onepassword::clear_reference(&key)).await??;
    status().await
}

/// Forget a secret's 1Password reference.
#[tauri::command]
pub async fn onepassword_clear_reference(key: String) -> Result<Status, String> {
    clear_reference(key).await
}

/// `settings.accounts.onepassword_set_enabled`.
pub(crate) fn set_enabled_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let on = a.0.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let card = if on {
        Card::new("Hollow app tools: turn on 1Password secrets")
            .line("Hollow will run the 1Password command line tool (op read) when it needs a secret you point at an op:// reference, instead of reading the Keychain.")
            .line("Nothing is saved from 1Password. If 1Password can't answer, the Keychain value is used when there is one.")
            .line("No secret has a reference yet unless you set one.")
    } else {
        Card::new("Hollow app tools: turn off 1Password secrets").line(
            "Hollow goes back to the Keychain for every secret. Saved references are kept, unused.",
        )
    };
    Ok(Some(card))
}

/// `settings.accounts.onepassword_set_reference`: refused before any card
/// for a secret that isn't a slot or a reference that isn't one.
pub(crate) fn set_reference_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let (key, reference) = (arg(a, "key"), arg(a, "reference"));
    let label = slot_label(&key)?;
    onepassword::reference_ok(&reference)
        .map_err(|e| format!("{} Nothing changed.", e.message()))?;
    let on = onepassword::enabled();
    Ok(Some(
        Card::new("Hollow app tools: read a secret from 1Password")
            .line(format!("Secret: {label}"))
            .line(format!("Reference: {reference}"))
            .line(if on {
                "1Password secrets are on: Hollow will read this at use time with op read."
            } else {
                "1Password secrets are off, so this has no effect until they are turned on."
            })
            .line("The Keychain value, if there is one, stays as the fallback."),
    ))
}

/// `settings.accounts.onepassword_clear_reference`.
pub(crate) fn clear_reference_card(_cx: &Cx, a: &Args) -> Result<Option<Card>, String> {
    let label = slot_label(&arg(a, "key"))?;
    Ok(Some(
        Card::new("Hollow app tools: stop reading a secret from 1Password")
            .line(format!("Secret: {label}"))
            .line("Its reference is forgotten and the Keychain value, if there is one, is used again."),
    ))
}
