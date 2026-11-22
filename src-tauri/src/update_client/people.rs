//! Row U9, the teammate side: who this Mac's token belongs to and which
//! channels it may use (updates design section 11).
//!
//! After a connect and on every check the client asks the update host's
//! `GET /me` (`infra/updates-worker/`, bearer token as for the manifests)
//! for `{name, channels}`, keeps it in `update-settings.json`
//! (`personName`, `allowedChannels`), and shows it in `UpdateState`. The
//! channel picker offers only allowed channels; `update_set_channel` refuses
//! any other; a channel that stops being allowed falls back to the first
//! allowed one (Main before Nightly) with a note. A 401 from `/me` (or from
//! the channel) is a refused token: the Worker never says why, so the words
//! are [`REVOKED`]. A 404 means a Worker without `/me`: nothing changes.
//!
//! Nothing here logs, stores or shows the token.

use super::settings::UpdateSettings;
use super::state::Channel;
use reqwest::Url;
use serde::Deserialize;
use std::time::Duration;

/// The words for a refused token: the update host answers one identical 401
/// for an unknown, malformed or revoked token, so the app can't tell which.
pub(crate) const REVOKED: &str = "The update server did not accept this token (it may have been revoked). Connect again with the link from your install page, or ask the person who added you for a new one.";
/// No channel is allowed for this token.
pub(crate) const NO_CHANNEL: &str = "Your updates access doesn't include any channel right now, so Hollow can't check for updates. Ask the person who added you.";

/// Every channel, in the Worker's order (Main first).
pub(crate) const ALL: [Channel; 2] = [Channel::Main, Channel::Nightly];

/// How long `/me` may take.
const TIMEOUT: Duration = Duration::from_secs(30);
/// A `/me` body past this is refused (it is a name and two words).
pub(crate) const MAX_BODY: usize = 16 * 1024;
/// A name past this many characters is cut (it is only shown).
pub(crate) const MAX_NAME_CHARS: usize = 80;

/// What `/me` says about this token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Me {
    pub(crate) name: String,
    /// Allowed channels, known ones only, in [`ALL`] order; empty means none.
    pub(crate) channels: Vec<Channel>,
}

/// The answer to one `/me` request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MeAnswer {
    Known(Me),
    /// 401: the token was refused ([`REVOKED`]).
    Refused,
    /// 404 (a Worker from before `/me`), or a fetcher that doesn't ask:
    /// keep what is stored.
    Unsupported,
    /// Offline, a timeout, a server error or a body that doesn't read: keep
    /// what is stored; the channel check says what is wrong.
    Network(String),
}

/// `<host>/me`.
pub(crate) fn me_url(host: &Url) -> Url {
    let mut url = host.clone();
    url.set_path("/me");
    url.set_query(None);
    url.set_fragment(None);
    url
}

/// Only `name` and `channels` are read. Other fields (a newer Worker's) are
/// ignored and never kept, as unknown channel names are: an old client keeps
/// working against a newer Worker. Both fields are required.
#[derive(Deserialize)]
struct Wire {
    name: String,
    channels: Vec<String>,
}

/// A `/me` body: `{name, channels}` (other fields ignored). Unknown channel
/// names (a newer Worker's) are dropped, repeats too; control, format
/// (`\p{Cf}`: bidi overrides, zero widths, the BOM) and separator characters
/// leave the name, which is cut to [`MAX_NAME_CHARS`].
pub(crate) fn parse_me(body: &[u8]) -> Result<Me, String> {
    if body.len() > MAX_BODY {
        return Err("the update server's answer about this token is too long".into());
    }
    let wire: Wire = serde_json::from_slice(body)
        .map_err(|_| "the update server's answer about this token doesn't read".to_string())?;
    let name: String = wire
        .name
        .chars()
        .filter(|&c| !hidden_char(c))
        .take(MAX_NAME_CHARS)
        .collect::<String>()
        .trim()
        .to_string();
    let channels = ALL
        .into_iter()
        .filter(|c| wire.channels.iter().any(|w| w == c.as_str()))
        .collect();
    Ok(Me { name, channels })
}

/// A character a name may not hold: control (`Cc`), format (`Cf`, which
/// includes the bidi overrides, zero-width characters and U+FEFF) or a line
/// or paragraph separator. The same set as `people.mjs`'s
/// `[\p{Cc}\p{Cf}\u2028\u2029]`, so the two sides agree on every name (and
/// Rust's `trim` and JS's `trim` then strip the same characters: they differ
/// only on U+0085 and U+FEFF, both refused here).
pub(crate) fn hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{2028}' | '\u{2029}')
        || matches!(
            c as u32,
            0x00AD
                | 0x0600..=0x0605
                | 0x061C
                | 0x06DD
                | 0x070F
                | 0x0890..=0x0891
                | 0x08E2
                | 0x180E
                | 0x200B..=0x200F
                | 0x202A..=0x202E
                | 0x2060..=0x2064
                | 0x2066..=0x206F
                | 0xFEFF
                | 0xFFF9..=0xFFFB
                | 0x110BD
                | 0x110CD
                | 0x13430..=0x1343F
                | 0x1BCA0..=0x1BCA3
                | 0x1D173..=0x1D17A
                | 0xE0001
                | 0xE0020..=0xE007F
        )
}

/// What a status and body mean.
pub(crate) fn classify(status: u16, body: &[u8]) -> MeAnswer {
    match status {
        200 => match parse_me(body) {
            Ok(me) => MeAnswer::Known(me),
            Err(why) => MeAnswer::Network(why),
        },
        401 => MeAnswer::Refused,
        404 => MeAnswer::Unsupported,
        code => MeAnswer::Network(format!("the update server answered {code}")),
    }
}

/// Ask `<host>/me` with the bearer token: no redirects (the token goes only
/// to the host), a time limit, a capped body. Blocking (the schedule
/// thread); the error never carries the token or the request.
pub(crate) fn fetch_me(host: &Url, token: &str) -> MeAnswer {
    let url = me_url(host);
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    tauri::async_runtime::block_on(async move {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT);
        if loopback {
            builder = builder.no_proxy();
        }
        let Ok(client) = builder.build() else {
            return MeAnswer::Network("couldn't start a request".into());
        };
        let mut response = match client.get(url).bearer_auth(token).send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => {
                return MeAnswer::Network("the update server didn't answer in time".into())
            }
            Err(_) => return MeAnswer::Network("the update server didn't answer".into()),
        };
        let status = response.status().as_u16();
        if response
            .content_length()
            .is_some_and(|n| n > MAX_BODY as u64)
        {
            return classify(status, &[0; MAX_BODY + 1]);
        }
        let mut body = Vec::new();
        loop {
            match response.chunk().await {
                Ok(Some(chunk)) => {
                    body.extend_from_slice(&chunk);
                    if body.len() > MAX_BODY {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => return MeAnswer::Network("the update server's answer broke off".into()),
            }
        }
        classify(status, &body)
    })
}

/// What a `/me` answer did to the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Access {
    /// The channel is still allowed.
    Same,
    /// The channel stopped being allowed: moved to the first allowed one.
    Moved { from: Channel, to: Channel },
    /// No channel is allowed.
    NoChannel,
}

/// Keep `me` in the settings and move off a channel that is no longer
/// allowed. A forced move sets the channel and leaves a note. Unlike a move
/// the person makes (`update_set_channel`), it never arms
/// `allow_downgrade_once`, so a `/me` answer (the server's word, not the
/// person's click) can't get an older build offered. A downgrade the person
/// armed for the channel being left is disarmed too, so it can't carry over
/// to the channel moved to.
pub(crate) fn reconcile(s: &mut UpdateSettings, me: &Me) -> Access {
    s.person_name = Some(me.name.clone()).filter(|n| !n.is_empty());
    s.allowed_channels = Some(me.channels.clone());
    let Some(&first) = me.channels.first() else {
        return Access::NoChannel;
    };
    if me.channels.contains(&s.channel) {
        return Access::Same;
    }
    let from = s.channel;
    s.channel = first;
    s.allow_downgrade_once = false;
    s.allow_downgrade_since_ms = None;
    Access::Moved { from, to: first }
}

/// The note an [`Access`] leaves beside the phase (kept until the person
/// picks a channel, disconnects, or access comes back).
pub(crate) fn access_note(access: &Access) -> Option<String> {
    match access {
        Access::Same => None,
        Access::Moved { from, to } => Some(format!(
            "Your updates access no longer includes {}, so Hollow switched to {}. A build older than this one isn't offered.",
            from.label(),
            to.label()
        )),
        Access::NoChannel => Some(NO_CHANNEL.into()),
    }
}

/// The channels the picker offers: what `/me` last said, or both while it
/// hasn't answered (a Worker without `/me`, or not connected yet).
pub(crate) fn allowed(s: &UpdateSettings) -> Vec<Channel> {
    s.allowed_channels.clone().unwrap_or_else(|| ALL.to_vec())
}

/// `update_set_channel`'s refusal for a channel the token may not use.
pub(crate) fn refuse_channel(s: &UpdateSettings, channel: Channel) -> Result<(), String> {
    if allowed(s).contains(&channel) {
        Ok(())
    } else {
        Err(format!(
            "CHANNEL_NOT_ALLOWED: Your updates access doesn't include the {} channel.",
            channel.label()
        ))
    }
}

/// Forget whose token this was (a new token or a disconnect).
pub(crate) fn forget(s: &mut UpdateSettings) {
    s.person_name = None;
    s.allowed_channels = None;
}
