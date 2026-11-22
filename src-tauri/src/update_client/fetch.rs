//! Checking a channel and downloading its build, through
//! `tauri-plugin-updater` (its `check` and `download` only, which verifies
//! the minisign signature against the build's public key; its `install` is
//! never called). Behind [`Fetcher`] so the client's tests use a fake.
//!
//! The plugin keeps HTTP status codes to itself (an error status reads as
//! "no release found"), so when it finds nothing this asks the endpoint once
//! more for the status alone, to tell an empty channel (404) from a refused
//! token (401) or a channel the person may not use (403).

use super::config::PLATFORM;
use reqwest::Url;
use std::time::Duration;

/// Why a check found nothing to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CheckError {
    /// The channel has no complete build yet (404).
    NothingPublished,
    /// The token was refused (401).
    Unauthorized,
    /// The token may not use this channel (403).
    Forbidden,
    /// Offline, a timeout, a server error: try again later.
    Network(String),
}

/// A build the endpoint offered, not yet downloaded.
pub(crate) trait Pending: Send {
    /// The manifest exactly as served (for Hollow's own checks).
    fn manifest(&self) -> &serde_json::Value;
    /// Where the download goes (the plugin's own reading of the manifest).
    fn download_url(&self) -> Url;
    /// Download it and check its signature; `progress(received, total)`
    /// returns `false` to stop it (the caller's check went stale).
    fn download(
        &self,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, String>;
}

pub(crate) trait Fetcher: Send + Sync {
    /// `Ok(None)`: the endpoint answered 204 (nothing to offer).
    fn check(&self, endpoint: &Url, token: &str) -> Result<Option<Box<dyn Pending>>, CheckError>;

    /// Row U9: the host's `/me` for this token (`people.rs`). A fetcher
    /// that doesn't ask changes nothing.
    fn me(&self, _host: &Url, _token: &str) -> super::people::MeAnswer {
        super::people::MeAnswer::Unsupported
    }
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// A download that stalls this long between bytes fails.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// A whole download may take this long (a build is 110 to 130 MB), so a
/// trickle can't hold the client busy forever.
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(25 * 60);
/// The error of a download its caller stopped.
pub(crate) const STOPPED: &str = "The download was stopped.";
/// A download announcing or delivering more than this is stopped.
pub(crate) const MAX_DOWNLOAD_BYTES: u64 = 400 * 1024 * 1024;

/// The real one: the plugin, on the app's handle.
pub(crate) struct PluginFetcher<R: tauri::Runtime> {
    pub(crate) app: tauri::AppHandle<R>,
    pub(crate) pubkey: String,
    /// [`DOWNLOAD_TIMEOUT`] and [`MAX_DOWNLOAD_BYTES`] in the app; tests
    /// set smaller ones.
    pub(crate) download_timeout: Duration,
    pub(crate) max_bytes: u64,
}

impl<R: tauri::Runtime> PluginFetcher<R> {
    pub(crate) fn new(app: tauri::AppHandle<R>, pubkey: String) -> Self {
        Self {
            app,
            pubkey,
            download_timeout: DOWNLOAD_TIMEOUT,
            max_bytes: MAX_DOWNLOAD_BYTES,
        }
    }
}

struct PluginPending {
    update: tauri_plugin_updater::Update,
    timeout: Duration,
    max_bytes: u64,
}

impl Pending for PluginPending {
    fn manifest(&self) -> &serde_json::Value {
        &self.update.raw_json
    }

    fn download_url(&self) -> Url {
        self.update.download_url.clone()
    }

    fn download(
        &self,
        progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
    ) -> Result<Vec<u8>, String> {
        let max = self.max_bytes;
        let stopped = std::sync::atomic::AtomicBool::new(false);
        let too_big = tokio::sync::Notify::new();
        let over = std::sync::atomic::AtomicBool::new(false);
        let big = || {
            format!(
                "The update is larger than {} MB, so it wasn't downloaded.",
                max / (1024 * 1024)
            )
        };
        let mut received = 0u64;
        let download = self.update.download(
            |chunk, total| {
                received += chunk as u64;
                if received > max || total.is_some_and(|t| t > max) {
                    over.store(true, std::sync::atomic::Ordering::SeqCst);
                    too_big.notify_one();
                }
                if !progress(received, total) {
                    stopped.store(true, std::sync::atomic::Ordering::SeqCst);
                    too_big.notify_one();
                }
            },
            || {},
        );
        // Dropping the download future (on either stop) aborts the request.
        tauri::async_runtime::block_on(async {
            tokio::select! {
                result = download => match result {
                    _ if stopped.load(std::sync::atomic::Ordering::SeqCst) => Err(STOPPED.into()),
                    _ if over.load(std::sync::atomic::Ordering::SeqCst) => Err(big()),
                    result => result.map_err(|e| plain_download_error(&e)),
                },
                () = too_big.notified() => {
                    if stopped.load(std::sync::atomic::Ordering::SeqCst) {
                        Err(STOPPED.into())
                    } else {
                        Err(big())
                    }
                }
                () = tokio::time::sleep(self.timeout) => Err(format!(
                    "The update didn't finish downloading within {} minutes.",
                    self.timeout.as_secs() / 60
                )),
            }
        })
    }
}

/// A download error for a person (never the request's headers).
fn plain_download_error(error: &tauri_plugin_updater::Error) -> String {
    use tauri_plugin_updater::Error as E;
    match error {
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => {
            "The downloaded update's signature doesn't match Hollow's update key, so it was thrown away.".into()
        }
        other => format!("The update didn't download: {other}"),
    }
}

impl<R: tauri::Runtime> Fetcher for PluginFetcher<R> {
    fn me(&self, host: &Url, token: &str) -> super::people::MeAnswer {
        super::people::fetch_me(host, token)
    }

    fn check(&self, endpoint: &Url, token: &str) -> Result<Option<Box<dyn Pending>>, CheckError> {
        use tauri_plugin_updater::UpdaterExt;
        let other = |e: tauri_plugin_updater::Error| CheckError::Network(e.to_string());
        let mut builder = self
            .app
            .updater_builder()
            .pubkey(self.pubkey.clone())
            .target(PLATFORM)
            // Hollow decides what is offered (`state::decide`): the plugin
            // reports every published build.
            .version_comparator(|_, _| true)
            .configure_client({
                let host = endpoint.clone();
                move |client| {
                    client
                        .connect_timeout(CONNECT_TIMEOUT)
                        .read_timeout(READ_TIMEOUT)
                        .redirect(same_host_redirects(host.clone()))
                }
            })
            .endpoints(vec![endpoint.clone()])
            .map_err(other)?
            .header("Authorization", format!("Bearer {token}"))
            .map_err(|_| CheckError::Unauthorized)?;
        if is_loopback(endpoint) {
            builder = builder.no_proxy();
        }
        let updater = builder.build().map_err(other)?;
        match tauri::async_runtime::block_on(updater.check()) {
            Ok(Some(update)) => Ok(Some(Box::new(PluginPending {
                update,
                timeout: self.download_timeout,
                max_bytes: self.max_bytes,
            }))),
            Ok(None) => Ok(None),
            Err(tauri_plugin_updater::Error::ReleaseNotFound) => {
                Err(classify(probe_status(endpoint, token)))
            }
            Err(e) => Err(other(e)),
        }
    }
}

/// Follow a redirect (at most 5) only to the same scheme, host and port,
/// never with credentials in the address: the bearer token goes with it.
fn same_host_redirects(host: Url) -> reqwest_updater::redirect::Policy {
    reqwest_updater::redirect::Policy::custom(move |attempt| {
        let to = attempt.url();
        if attempt.previous().len() < 5 && super::config::same_origin(to, &host) {
            attempt.follow()
        } else {
            attempt.stop()
        }
    })
}

fn is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))
}

/// The endpoint's status code alone (one GET, redirects not followed).
fn probe_status(endpoint: &Url, token: &str) -> Option<u16> {
    // Built and sent inside the runtime: reqwest's client needs its reactor.
    tauri::async_runtime::block_on(async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(CONNECT_TIMEOUT)
            .no_proxy_if(is_loopback(endpoint))
            .build()
            .ok()?;
        let response = client
            .get(endpoint.clone())
            .bearer_auth(token)
            .send()
            .await
            .ok()?;
        Some(response.status().as_u16())
    })
}

trait NoProxyIf {
    fn no_proxy_if(self, yes: bool) -> Self;
}

impl NoProxyIf for reqwest::ClientBuilder {
    fn no_proxy_if(self, yes: bool) -> Self {
        if yes {
            self.no_proxy()
        } else {
            self
        }
    }
}

/// The plugin found nothing: what the status says.
pub(crate) fn classify(status: Option<u16>) -> CheckError {
    match status {
        Some(404) | Some(204) => CheckError::NothingPublished,
        Some(401) => CheckError::Unauthorized,
        Some(403) => CheckError::Forbidden,
        Some(code) => CheckError::Network(format!("the update server answered {code}")),
        None => CheckError::Network("the update server didn't answer".into()),
    }
}
