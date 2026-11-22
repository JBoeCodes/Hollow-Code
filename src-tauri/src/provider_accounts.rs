//! Settings, Accounts: the sign-in state of each agent CLI besides Claude
//! (whose several accounts live in `claude_accounts`), and each CLI's own
//! sign-in and sign-out commands.
//!
//! One account per provider: Hollow never stores these credentials. It reads
//! only what it shows (the signed-in email and plan) from each CLI's own
//! files, and runs the CLI's own `login` / `logout`:
//!
//! - **ChatGPT (Codex)**: `~/.codex/auth.json` (or `$CODEX_HOME`). A ChatGPT
//!   login carries an `id_token` whose claims name the email and
//!   `chatgpt_plan_type`; the token is decoded, never verified or sent
//!   anywhere. Sign in: `codex login` (it opens the browser and waits for the
//!   callback). Sign out: `codex logout`.
//! - **Grok**: `~/.grok/auth.json`, entries with an `auth_mode` and a refresh
//!   token (the SuperGrok OAuth login) carry the email. Sign in: `grok login
//!   --oauth`. Sign out: `grok logout`.
//! - **Antigravity**: no login command and no reliable account file, so it
//!   counts as signed in when `agy models` answers (what Hollow's model list
//!   already relies on). Signing in happens by running `agy` in Terminal.
//!
//! Only emails, plan names and booleans leave this module; tokens are read
//! into memory to decode a claim and dropped.

use crate::agent::{self, AgentKind};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Long enough to finish a browser sign-in; the CLI's own server waits.
const SIGN_IN_LIMIT: Duration = Duration::from_secs(5 * 60);
const QUICK_LIMIT: Duration = Duration::from_secs(20);
const STDOUT_CAP: usize = 256 * 1024;
static SIGNING_IN: AtomicBool = AtomicBool::new(false);
pub(crate) mod limits;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SignInMethod {
    /// The CLI's own `login` opens the browser.
    Browser,
    /// Sign in by running the CLI in Terminal.
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccount {
    /// `codex` | `antigravity` | `grok`.
    pub id: String,
    pub label: String,
    pub installed: bool,
    /// `None` when it could not be told.
    pub signed_in: Option<bool>,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub note: Option<String>,
    pub sign_in: SignInMethod,
    pub can_sign_out: bool,
    pub limits: Option<limits::PlanLimits>,
}

// ── Reading the CLIs' own files ─────────────────────────────────────────────

fn read_json(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// The claims of a JWT, decoded without verification (display only).
fn jwt_claims(token: &str) -> Option<Value> {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `(signed_in, email, plan, note)` from Codex's auth file.
fn codex_state(auth: &Path) -> (bool, Option<String>, Option<String>, Option<String>) {
    let Some(doc) = read_json(auth) else {
        return (false, None, None, None);
    };
    let mode = doc
        .get("auth_mode")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let api_key = doc
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_some_and(|k| !k.is_empty());
    let id_token = doc.pointer("/tokens/id_token").and_then(Value::as_str);
    if mode.eq_ignore_ascii_case("apikey") || (id_token.is_none() && api_key) {
        return (true, None, None, Some("Signed in with an API key, which bills per token. Sign out and sign in with ChatGPT to use your plan.".into()));
    }
    let Some(claims) = id_token.and_then(jwt_claims) else {
        return (false, None, None, None);
    };
    let email = claims
        .get("email")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let plan = claims
        .pointer("/https:~1~1api.openai.com~1auth/chatgpt_plan_type")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
        .map(|p| format!("ChatGPT {}", title_case(p)));
    (true, email, plan, None)
}

/// A2 / N17: the Codex sign-in as another Mac matches it, without a
/// secret: the shared key (`peer_usage::shared_key` over the ChatGPT user
/// and workspace ids in the `id_token` claims), the plan and the workspace
/// that pays, when the claims name one.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CodexIdentity {
    pub key: String,
    pub plan: Option<String>,
    pub billing: Option<String>,
}

/// [`CodexIdentity`] from this Mac's Codex auth file; `None` when not
/// signed in with ChatGPT (an API key bills per token and has no plan).
pub(crate) fn codex_identity() -> Option<CodexIdentity> {
    codex_identity_at(&codex_home()?.join("auth.json"))
}

pub(crate) fn codex_identity_at(auth: &Path) -> Option<CodexIdentity> {
    let doc = read_json(auth)?;
    let mode = doc
        .get("auth_mode")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if mode.eq_ignore_ascii_case("apikey") {
        return None;
    }
    let claims = doc
        .pointer("/tokens/id_token")
        .and_then(Value::as_str)
        .and_then(jwt_claims)?;
    codex_identity_of(&claims)
}

pub(crate) fn codex_identity_of(claims: &Value) -> Option<CodexIdentity> {
    let auth = claims.get("https://api.openai.com/auth");
    let text = |v: Option<&Value>| {
        v.and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let user = text(auth.and_then(|a| a.get("chatgpt_user_id")))
        .or_else(|| text(auth.and_then(|a| a.get("user_id"))))
        .or_else(|| text(claims.get("sub")))?;
    let workspace = text(auth.and_then(|a| a.get("chatgpt_account_id"))).unwrap_or_default();
    let plan = text(auth.and_then(|a| a.get("chatgpt_plan_type")))
        .map(|p| format!("ChatGPT {}", title_case(&p)));
    let billing = auth
        .and_then(|a| a.get("organizations"))
        .and_then(Value::as_array)
        .and_then(|orgs| {
            orgs.iter()
                .find(|o| o.get("is_default").and_then(Value::as_bool) == Some(true))
                .or_else(|| (orgs.len() == 1).then(|| &orgs[0]))
        })
        .and_then(|o| text(o.get("title")));
    Some(CodexIdentity {
        key: crate::peer_usage::shared_key("codex-user", &format!("{user}\n{workspace}")),
        plan,
        billing,
    })
}

/// `(signed_in, email)` from Grok's auth file.
fn grok_state(auth: &Path) -> (bool, Option<String>) {
    let Some(entries) = read_json(auth).and_then(|v| v.as_object().cloned()) else {
        return (false, None);
    };
    entries
        .values()
        .find(|e| {
            e.get("auth_mode").and_then(Value::as_str).is_some()
                && e.get("refresh_token")
                    .and_then(Value::as_str)
                    .is_some_and(|t| !t.is_empty())
        })
        .map(|e| {
            (
                true,
                e.get("email").and_then(Value::as_str).map(str::to_owned),
            )
        })
        .unwrap_or((false, None))
}

fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

async fn agy_signed_in() -> Option<bool> {
    let binary = agent::find_binary(AgentKind::Antigravity)?;
    let mut cmd = tokio::process::Command::new(binary);
    cmd.arg("models").current_dir(std::env::temp_dir());
    let out = crate::process_group::run_to_completion_capped(
        &mut cmd,
        None,
        QUICK_LIMIT,
        None,
        STDOUT_CAP,
    )
    .await
    .ok()?;
    if out.timed_out {
        return None;
    }
    Some(out.status.is_some_and(|s| s.success()))
}

pub async fn list() -> Vec<ProviderAccount> {
    let codex_installed = agent::is_available(AgentKind::Codex);
    let (codex_in, codex_email, codex_plan, codex_note) = codex_home()
        .map(|h| codex_state(&h.join("auth.json")))
        .unwrap_or((false, None, None, None));

    let agy_installed = agent::is_available(AgentKind::Antigravity);
    let agy_in = if agy_installed {
        agy_signed_in().await
    } else {
        Some(false)
    };

    let grok_installed = agent::is_available(AgentKind::Grok);
    let (grok_in, grok_email) = dirs::home_dir()
        .map(|h| grok_state(&h.join(".grok").join("auth.json")))
        .unwrap_or((false, None));

    vec![
        ProviderAccount {
            id: "codex".into(),
            label: "ChatGPT (Codex)".into(),
            installed: codex_installed,
            signed_in: Some(codex_in),
            email: codex_email,
            plan: codex_plan,
            note: codex_note,
            sign_in: SignInMethod::Browser,
            can_sign_out: codex_installed && codex_in,
            limits: limits::latest(),
        },
        ProviderAccount {
            id: "antigravity".into(),
            label: "Antigravity".into(),
            installed: agy_installed,
            signed_in: agy_in,
            email: None,
            plan: None,
            note: Some("Antigravity signs in inside its own app: open Terminal, run agy, and choose your Google account.".into()),
            sign_in: SignInMethod::Terminal,
            can_sign_out: false,
            limits: None,
        },
        ProviderAccount {
            id: "grok".into(),
            label: "Grok".into(),
            installed: grok_installed,
            signed_in: Some(grok_in),
            email: grok_email,
            plan: None,
            note: None,
            sign_in: SignInMethod::Browser,
            can_sign_out: grok_installed && grok_in,
            limits: None,
        },
    ]
}

// ── Sign in / out ───────────────────────────────────────────────────────────

fn login_args(id: &str, sign_out: bool) -> Result<(AgentKind, &'static [&'static str]), String> {
    match (id, sign_out) {
        ("codex", false) => Ok((AgentKind::Codex, &["login"])),
        ("codex", true) => Ok((AgentKind::Codex, &["logout"])),
        ("grok", false) => Ok((AgentKind::Grok, &["login", "--oauth"])),
        ("grok", true) => Ok((AgentKind::Grok, &["logout"])),
        ("antigravity", _) => {
            Err("Antigravity signs in inside its own app: open Terminal and run agy.".into())
        }
        _ => Err("Unknown provider.".into()),
    }
}

async fn run_cli(id: &str, sign_out: bool) -> Result<(), String> {
    let (kind, args) = login_args(id, sign_out)?;
    if !agent::is_available(kind) {
        return Err(format!("{} is not installed.", kind.label()));
    }
    let binary =
        agent::find_binary(kind).ok_or_else(|| format!("{} is not installed.", kind.label()))?;
    let mut cmd = tokio::process::Command::new(binary);
    cmd.args(args)
        .current_dir(std::env::temp_dir())
        .env("PATH", agent::cli_path());
    let limit = if sign_out { QUICK_LIMIT } else { SIGN_IN_LIMIT };
    let out =
        crate::process_group::run_to_completion_capped(&mut cmd, None, limit, None, STDOUT_CAP)
            .await
            .map_err(|e| format!("Could not start {}: {e}", kind.label()))?;
    if out.timed_out {
        return Err(format!(
            "{} sign-in timed out. Try again and finish in the browser.",
            kind.label()
        ));
    }
    if !out.status.is_some_and(|s| s.success()) {
        let tail = crate::claude_runner::redact(out.stderr.trim());
        let tail = crate::claude_runner::truncate(&tail, 300);
        return Err(if tail.is_empty() {
            format!(
                "{} {} failed.",
                kind.label(),
                if sign_out { "sign-out" } else { "sign-in" }
            )
        } else {
            format!("{}: {tail}", kind.label())
        });
    }
    Ok(())
}

#[tauri::command]
pub async fn provider_accounts() -> Result<Vec<ProviderAccount>, String> {
    Ok(list().await)
}

/// Run the provider's own sign-in (a browser opens) and return the refreshed
/// list once it finishes. One sign-in at a time.
#[tauri::command]
pub async fn provider_sign_in(id: String) -> Result<Vec<ProviderAccount>, String> {
    if SIGNING_IN.swap(true, Ordering::SeqCst) {
        return Err("Another sign-in is still waiting for the browser.".into());
    }
    let result = run_cli(&id, false).await;
    SIGNING_IN.store(false, Ordering::SeqCst);
    result?;
    Ok(list().await)
}

#[tauri::command]
pub async fn provider_sign_out(id: String) -> Result<Vec<ProviderAccount>, String> {
    run_cli(&id, true).await?;
    Ok(list().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

    fn jwt(claims: &Value) -> String {
        format!(
            "h.{}.s",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims).unwrap())
        )
    }

    fn write(dir: &Path, name: &str, value: &Value) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, serde_json::to_string(value).unwrap()).unwrap();
        path
    }

    #[test]
    fn codex_chatgpt_login_shows_email_and_plan() {
        let dir = tempfile::tempdir().unwrap();
        let token = jwt(&serde_json::json!({
            "email": "me@example.com",
            "https://api.openai.com/auth": {"chatgpt_plan_type": "pro"}
        }));
        let path = write(
            dir.path(),
            "auth.json",
            &serde_json::json!({
                "auth_mode": "chatgpt", "OPENAI_API_KEY": null, "tokens": {"id_token": token}
            }),
        );
        assert_eq!(
            codex_state(&path),
            (
                true,
                Some("me@example.com".into()),
                Some("ChatGPT Pro".into()),
                None
            )
        );
    }

    #[test]
    fn codex_api_key_login_is_flagged_and_missing_is_signed_out() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "auth.json",
            &serde_json::json!({"auth_mode": "apikey", "OPENAI_API_KEY": "sk-x"}),
        );
        let (signed_in, email, _, note) = codex_state(&path);
        assert!(signed_in && email.is_none());
        assert!(note.unwrap().contains("API key"));
        assert_eq!(
            codex_state(&dir.path().join("none.json")),
            (false, None, None, None)
        );
    }

    #[test]
    fn grok_oauth_entry_shows_its_email() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "auth.json",
            &serde_json::json!({
                "https://auth.x.ai::id": {"auth_mode": "oidc", "refresh_token": "r", "email": "me@x.com"}
            }),
        );
        assert_eq!(grok_state(&path), (true, Some("me@x.com".into())));
        let empty = write(
            dir.path(),
            "empty.json",
            &serde_json::json!({"x": {"auth_mode": "oidc", "refresh_token": ""}}),
        );
        assert_eq!(grok_state(&empty), (false, None));
    }

    #[test]
    fn only_codex_and_grok_have_login_commands() {
        assert!(login_args("codex", false).is_ok());
        assert!(login_args("grok", true).is_ok());
        assert!(login_args("antigravity", false).is_err());
        assert!(login_args("gemini", false).is_err());
        assert!(login_args("claude", false).is_err());
    }
}
