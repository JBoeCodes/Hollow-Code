//! The person's updates token: 32 random bytes as base64url without padding
//! (43 characters, `infra/updates-worker/README.md`), sent only as
//! `Authorization: Bearer <token>` to the update host.
//!
//! It lives in the Keychain through `secret_store` under [`KEY`], so it is in
//! no settings file or export. Commands take it once and answer only
//! `connected`; it is never logged, put in an error, a state or an event.

/// The `secret_store` key.
pub(crate) const KEY: &str = "updates:token";

/// Exactly 43 base64url characters.
pub(crate) fn valid_shape(token: &str) -> bool {
    token.len() == 43
        && token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Where the token is kept: the Keychain in the app, memory in tests.
pub(crate) trait TokenStore: Send + Sync {
    fn get(&self) -> Result<Option<String>, String>;
    fn set(&self, token: &str) -> Result<(), String>;
    fn delete(&self) -> Result<(), String>;
}

/// The Keychain, through Hollow's one secrets item (`secret_store`, which
/// itself uses memory in test builds).
pub(crate) struct Keychain;

impl TokenStore for Keychain {
    fn get(&self) -> Result<Option<String>, String> {
        crate::secret_store::get(KEY).map_err(|e| e.to_string())
    }
    fn set(&self, token: &str) -> Result<(), String> {
        crate::secret_store::set(KEY, token).map_err(|e| e.to_string())
    }
    fn delete(&self) -> Result<(), String> {
        crate::secret_store::delete(KEY).map_err(|e| e.to_string())
    }
}

/// A token from a person or a link, trimmed and checked. The error never
/// repeats what was given.
pub(crate) fn accept(given: &str) -> Result<String, String> {
    let token = given.trim();
    if valid_shape(token) {
        Ok(token.to_string())
    } else {
        Err("That isn't an updates token: a token is 43 letters, digits, `-` and `_`. Copy it again from your install page.".into())
    }
}
