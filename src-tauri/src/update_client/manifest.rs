//! The channel manifest the Worker serves (`infra/updates-worker/README.md`,
//! "Channel manifest format"): Tauri's static updater JSON plus Hollow's
//! `channel`, `commit`, `buildId` and `schema`.
//!
//! The updater plugin parses `version`, `notes`, `pub_date` and the
//! platform entry for itself; this reads the same JSON again for Hollow's
//! own checks before anything downloads: the channel is the one asked for,
//! and the download URL is on the update host (the bearer token goes with
//! the download, so a manifest must never point it elsewhere) at exactly
//! `/builds/<version>/Hollow.app.tar.gz`.

use super::config::{same_origin, PLATFORM};
use super::state::{valid_version, Channel, UpdateRelease};
use reqwest::Url;
use serde_json::Value;

/// The checked manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Manifest {
    pub(crate) version: String,
    pub(crate) notes: String,
    pub(crate) pub_date: Option<String>,
    pub(crate) channel: Channel,
    pub(crate) commit: Option<String>,
    pub(crate) build_id: Option<String>,
    pub(crate) url: Url,
}

impl Manifest {
    pub(crate) fn release(&self, downgrade: bool) -> UpdateRelease {
        UpdateRelease {
            version: self.version.clone(),
            notes: self.notes.clone(),
            pub_date: self.pub_date.clone(),
            channel: self.channel,
            commit: self.commit.clone(),
            downgrade,
            local: None,
        }
    }
}

/// Notes are cut to this many characters for the window.
pub(crate) const MAX_NOTES_CHARS: usize = 20_000;

/// Check `raw` as the manifest of `channel` served from `host`.
pub(crate) fn parse(raw: &Value, channel: Channel, host: &Url) -> Result<Manifest, String> {
    let bad = |why: &str| format!("The update server sent a manifest Hollow can't use: {why}.");
    let obj = raw.as_object().ok_or_else(|| bad("not a JSON object"))?;
    let text = |key: &str| obj.get(key).and_then(Value::as_str);
    let version = text("version").ok_or_else(|| bad("no version"))?;
    let version = version.trim_start_matches('v').to_string();
    if !valid_version(&version) {
        return Err(bad("the version isn't dotted numbers"));
    }
    let sent = text("channel").ok_or_else(|| bad("no channel"))?;
    if sent != channel.as_str() {
        return Err(bad(&format!(
            "it is for the {sent} channel, not {}",
            channel.as_str()
        )));
    }
    let platform = obj
        .get("platforms")
        .and_then(|p| p.get(PLATFORM))
        .ok_or_else(|| bad(&format!("no {PLATFORM} build")))?;
    let signature = platform
        .get("signature")
        .and_then(Value::as_str)
        .unwrap_or("");
    if signature.trim().is_empty() {
        return Err(bad("the build has no signature"));
    }
    let url = platform
        .get("url")
        .and_then(Value::as_str)
        .and_then(|u| Url::parse(u).ok())
        .ok_or_else(|| bad("the build has no download address"))?;
    if !same_origin(&url, host) {
        return Err(bad("the download is not on the update server"));
    }
    let expected_path = format!("/builds/{version}/Hollow.app.tar.gz");
    if url.path() != expected_path || url.query().is_some() || url.fragment().is_some() {
        return Err(bad("the download address is not this version's build"));
    }
    let commit = text("commit")
        .filter(|c| c.len() == 40 && c.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
        .map(str::to_string);
    let notes: String = text("notes")
        .unwrap_or("")
        .chars()
        .take(MAX_NOTES_CHARS)
        .collect();
    Ok(Manifest {
        version,
        notes,
        pub_date: text("pub_date").map(str::to_string),
        channel,
        commit,
        build_id: text("buildId")
            .filter(|b| !b.trim().is_empty())
            .map(str::to_string),
        url,
    })
}
