//! Where updates come from and whether this build can take them: the host
//! (`https://updates.example.com`, or `HOLLOW_UPDATES_URL`), the channel
//! manifest URL, the updater public key and the signing team.
//!
//! The client is inert, with a plain-words reason, while the public key, the
//! host or the person's token is missing ([`resolve`]); nothing here panics.

use super::state::Channel;
use crate::update_install::verify::Team;
use reqwest::Url;

/// The updates Worker (design section 4, `infra/updates-worker/`).
pub(crate) const DEFAULT_HOST: &str = "https://updates.example.com";
/// Overrides [`DEFAULT_HOST`] (tests point it at a local server). A release
/// build still refuses plain `http` (the updater plugin's own rule).
pub(crate) const HOST_ENV: &str = "HOLLOW_UPDATES_URL";
/// Overrides the signing team the staged bundle must carry (the same
/// variable the installer reads, `update_install::cli`).
pub(crate) const TEAM_ENV: &str = "HOLLOW_UPDATE_TEAM";
/// The one platform the release builder publishes (design section 13 item 10).
pub(crate) const PLATFORM: &str = "darwin-aarch64";
/// The words the client shows while this build has no updater public key
/// (`tauri.conf.json` `plugins.updater.pubkey`, empty until U11).
pub(crate) const NOT_SET_UP: &str = "Updates are not set up on this build.";
/// The words while no token is stored.
pub(crate) const NOT_CONNECTED: &str = "Hollow isn't connected to updates yet. Open the Connect Hollow link from your install page, or paste your token in Settings, Updates.";
/// The words in a review copy or a candidate build, which never update.
pub(crate) const REVIEW_COPY: &str =
    "This is a review copy of Hollow. Only the everyday Hollow updates itself.";

/// `HOLLOW_UPDATES_URL`, read only in tests and debug builds: a release
/// build always uses [`DEFAULT_HOST`].
pub(crate) fn host_override() -> Option<String> {
    #[cfg(any(test, debug_assertions))]
    {
        std::env::var(HOST_ENV).ok()
    }
    #[cfg(not(any(test, debug_assertions)))]
    {
        None
    }
}

/// `HOLLOW_UPDATE_TEAM`, read only in tests and debug builds.
pub(crate) fn team_override() -> Option<String> {
    #[cfg(any(test, debug_assertions))]
    {
        std::env::var(TEAM_ENV).ok()
    }
    #[cfg(not(any(test, debug_assertions)))]
    {
        None
    }
}

/// The host from the override (when set and not empty) or the default: no
/// credentials, query, fragment or path; `https` when `require_https` (a
/// release build), else `http` too.
pub(crate) fn host_from(env: Option<String>, require_https: bool) -> Result<Url, String> {
    let text = env
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_HOST.to_string());
    let url = Url::parse(text.trim_end_matches('/'))
        .map_err(|_| format!("{HOST_ENV} is not a web address."))?;
    let schemes: &[&str] = if require_https {
        &["https"]
    } else {
        &["http", "https"]
    };
    if !schemes.contains(&url.scheme())
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(format!(
            "{HOST_ENV} must be a plain http(s) origin such as {DEFAULT_HOST}."
        ));
    }
    Ok(url)
}

/// `<host>/channels/<channel>.json`.
pub(crate) fn endpoint(host: &Url, channel: Channel) -> Url {
    let mut url = host.clone();
    url.set_path(&format!("/channels/{}.json", channel.as_str()));
    url
}

/// Whether `url` has the same scheme, host and port as `host`: the bearer
/// token is only ever sent there.
pub(crate) fn same_origin(url: &Url, host: &Url) -> bool {
    url.username().is_empty()
        && url.password().is_none()
        && url.scheme() == host.scheme()
        && url.host_str() == host.host_str()
        && url.port_or_known_default() == host.port_or_known_default()
}

/// The updater public key from the app's `plugins` config
/// (`plugins.updater.pubkey`): the base64 text of a minisign public key file,
/// as `tauri signer generate` prints it. Empty when absent.
pub(crate) fn pubkey_from_plugins(plugins: &serde_json::Value) -> String {
    plugins
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .map(|k| k.trim().to_string())
        .unwrap_or_default()
}

/// The team the staged bundle must be signed by: `HOLLOW_UPDATE_TEAM`, else
/// the Developer ID team.
pub(crate) fn team_from(env: Option<String>) -> Result<Team, String> {
    Team::parse(
        &env.filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| crate::update_install::DEFAULT_TEAM.to_string()),
    )
}

/// What this build was configured with, before the token.
#[derive(Debug, Clone)]
pub(crate) struct Config {
    /// `None` with the reason when the host is unusable.
    pub(crate) host: Result<Url, String>,
    pub(crate) pubkey: String,
    pub(crate) team: Result<Team, String>,
    /// The running version (`build-provenance.json`, never `CARGO_PKG_VERSION`).
    pub(crate) running_version: String,
    /// A review copy or candidate: always inert.
    pub(crate) review_copy: bool,
}

/// Everything a check needs, or why there is no check. Its `Debug` never
/// prints the token.
#[derive(Clone)]
pub(crate) struct Ready {
    pub(crate) host: Url,
    pub(crate) endpoint: Url,
    pub(crate) token: String,
}

impl Config {
    /// Whether the build itself can update (pubkey, host, team), whatever
    /// the token.
    pub(crate) fn configured(&self) -> bool {
        self.blocker().is_none()
    }

    fn blocker(&self) -> Option<String> {
        if self.review_copy {
            return Some(REVIEW_COPY.into());
        }
        if self.pubkey.is_empty() {
            return Some(NOT_SET_UP.into());
        }
        if let Err(why) = &self.host {
            return Some(format!("{NOT_SET_UP} {why}"));
        }
        if let Err(why) = &self.team {
            return Some(format!("{NOT_SET_UP} {TEAM_ENV}: {why}"));
        }
        None
    }

    /// The check's inputs, or the plain-words reason the client is inert.
    pub(crate) fn resolve(&self, channel: Channel, token: Option<String>) -> Result<Ready, String> {
        if let Some(why) = self.blocker() {
            return Err(why);
        }
        let host = self.host.clone()?;
        let token = token
            .filter(|t| super::token::valid_shape(t))
            .ok_or_else(|| NOT_CONNECTED.to_string())?;
        Ok(Ready {
            endpoint: endpoint(&host, channel),
            host,
            token,
        })
    }
}

impl std::fmt::Debug for Ready {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ready")
            .field("host", &self.host.as_str())
            .field("endpoint", &self.endpoint.as_str())
            .field("token", &"<redacted>")
            .finish()
    }
}
