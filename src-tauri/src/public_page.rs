//! Link import for any agent (repo ideas 2026-10-27 #6): Hollow fetches a
//! public page itself, signed out, and hands its text to whichever agent
//! the person picked, instead of leaving Claude Code to fetch it with
//! WebFetch (no other agent could). YouTube links stay on their own path.
//!
//! Fetching on the person's behalf is a small SSRF surface, so the gate is
//! [`crate::remote_image`]'s, adapted to pages:
//! - `http` or `https` on the default port, with no user name or password;
//! - every host, and every redirect hop, must be a public name that resolves
//!   only to public addresses (no loopback, private, link-local and so cloud
//!   metadata, CGNAT, multicast or reserved space, in either family), and
//!   the connection is pinned to the addresses that were checked, so DNS
//!   can't rebind between check and connect; no proxy, cookies or referer;
//! - at most [`MAX_REDIRECTS`] redirects, [`MAX_BYTES`] of HTML or text,
//!   [`TOTAL_TIMEOUT`] in all; one URL per person's click, never a crawl.
//!
//! A sign-in wall or a browser check is refused as content (Agent-Reach's
//! rule): the person gets told, rather than notes about a login form.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use reqwest::Url;

use crate::remote_image::{check_resolved, is_local_name, is_public_ip};

/// The most HTML or text a page may send.
pub const MAX_BYTES: usize = 2 * 1024 * 1024;
/// Redirect hops followed (each one re-vetted) before giving up.
pub const MAX_REDIRECTS: usize = 5;
/// The whole fetch, DNS and every redirect included.
pub const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The most text kept from a page (characters).
pub const MAX_TEXT_CHARS: usize = 60_000;

/// A page as Hollow read it: where it ended up, its title and its text.
#[derive(Debug, Clone, PartialEq)]
pub struct Page {
    pub url: String,
    pub title: String,
    pub text: String,
}

/// Parse and vet a page address the person gave.
pub fn parse_page_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw.trim()).map_err(|_| "That isn't a valid web address.".to_string())?;
    vet_url(url)
}

fn vet_url(url: Url) -> Result<Url, String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("Only http and https pages can be imported.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("A page address can't carry a username or password.".into());
    }
    // `port()` is None for the scheme's default.
    if url.port().is_some() {
        return Err("Pages are imported only from the standard web ports.".into());
    }
    let host = url.host_str().unwrap_or_default();
    match ip_literal(host) {
        Some(ip) if !is_public_ip(ip) => Err("That page is on a private or local address.".into()),
        Some(_) => Ok(url),
        None if is_local_name(host) => Err("That page is on a local network name.".into()),
        None => Ok(url),
    }
}

fn ip_literal(host: &str) -> Option<IpAddr> {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .ok()
}

/// The next hop of a redirect, vetted like the first address.
fn redirect_target(current: &Url, location: Option<&str>) -> Result<Url, String> {
    let loc = location.ok_or_else(|| "The page redirected without saying where.".to_string())?;
    let next = current
        .join(loc)
        .map_err(|_| "The page redirected to an invalid address.".to_string())?;
    vet_url(next)
}

/// A response's media type, when it is a page Hollow reads.
fn page_kind(content_type: Option<&str>) -> Result<&'static str, String> {
    let essence = content_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    match essence.as_str() {
        "text/html" | "application/xhtml+xml" => Ok("html"),
        "text/plain" => Ok("text"),
        _ => {
            Err("That address isn't a web page Hollow can read (only HTML and plain text).".into())
        }
    }
}

async fn resolve_public(host: &str, port: u16) -> Result<Vec<SocketAddr>, String> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .map_err(|_| "That page's host didn't resolve.".to_string())?
        .collect();
    check_resolved(&addrs)
        .map_err(|_| "That page's host resolves to a private or local address.".to_string())?;
    Ok(addrs)
}

/// Fetch one public page and read its text: the gate above, then
/// [`read_page`]. Never more than this one address and its redirects.
pub async fn fetch(raw: &str) -> Result<Page, String> {
    let url = parse_page_url(raw)?;
    tokio::time::timeout(TOTAL_TIMEOUT, fetch_inner(url))
        .await
        .map_err(|_| {
            format!(
                "The page took over {} s to answer.",
                TOTAL_TIMEOUT.as_secs()
            )
        })?
}

async fn fetch_inner(mut url: Url) -> Result<Page, String> {
    for _ in 0..=MAX_REDIRECTS {
        let host = url.host_str().unwrap_or_default().to_string();
        let port = url.port_or_known_default().unwrap_or(443);
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!(
                "Hollow/",
                env!("CARGO_PKG_VERSION"),
                " (link import)"
            ));
        if ip_literal(&host).is_none() {
            // Pin the connection to the addresses just checked.
            let addrs = resolve_public(&host, port).await?;
            builder = builder.resolve_to_addrs(&host, &addrs);
        }
        let client = builder.build().map_err(|e| e.to_string())?;
        let mut resp = client
            .get(url.clone())
            .header(ACCEPT, "text/html,application/xhtml+xml,text/plain;q=0.9")
            .send()
            .await
            .map_err(|_| "The page didn't answer.".to_string())?;
        let status = resp.status();
        if status.is_redirection() {
            let location = resp.headers().get(LOCATION).and_then(|v| v.to_str().ok());
            url = redirect_target(&url, location)?;
            continue;
        }
        if matches!(status.as_u16(), 401 | 403 | 407) {
            return Err(SIGNED_IN.into());
        }
        if status.as_u16() == 429 {
            return Err("The site asked Hollow to slow down; try again later.".into());
        }
        if !status.is_success() {
            return Err(format!("The page answered {}.", status.as_u16()));
        }
        let kind = page_kind(
            resp.headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
        )?;
        if resp.content_length().is_some_and(|n| n > MAX_BYTES as u64) {
            return Err(TOO_LARGE.into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|_| "The page stopped answering.".to_string())?
        {
            if body.len() + chunk.len() > MAX_BYTES {
                return Err(TOO_LARGE.into());
            }
            body.extend_from_slice(&chunk);
        }
        return read_page(
            url.as_str(),
            &String::from_utf8_lossy(&body),
            kind == "html",
        );
    }
    Err("The page redirected too many times.".into())
}

const SIGNED_IN: &str =
    "That page needs a sign-in or a browser check; Hollow imports public pages only, signed out.";
const TOO_LARGE: &str = "That page is larger than 2 MB.";

/// A page's title and readable text: its `main` or `article` when one holds
/// a real amount of text, else its body; scripts, styles and other
/// non-content dropped; block elements as line breaks, list items as "- ",
/// table cells separated by " | "; entities decoded; at most
/// [`MAX_TEXT_CHARS`]. A sign-in wall or a browser check is refused.
pub fn read_page(url: &str, body: &str, html: bool) -> Result<Page, String> {
    let (title, text, password) = if html {
        html_text(body)
    } else {
        (String::new(), tidy(body), false)
    };
    let lower_title = title.to_ascii_lowercase();
    let lower = text.to_ascii_lowercase();
    let length = text.chars().count();
    // A browser check is a short page that says so; an article about
    // captchas is long (review of #1062: a title word alone refused it).
    let challenge = length < 2_000
        && [
            "just a moment",
            "attention required",
            "access denied",
            "are you a robot",
            "verify you are human",
            "captcha",
            "checking your browser",
        ]
        .iter()
        .any(|w| lower_title.contains(w) || lower.contains(w));
    // A sign-in wall: a password field on a page with little else, a sign-in
    // title, or a short page that asks for one.
    let signs_in = |s: &str| {
        [
            "sign in",
            "log in",
            "login",
            "subscribe to continue",
            "enable javascript",
        ]
        .iter()
        .any(|w| s.contains(w))
    };
    let wall = (password && length < 5_000)
        || (signs_in(&lower_title) && length < 3_000)
        || (length < 600 && signs_in(&lower));
    if challenge || wall {
        return Err(SIGNED_IN.into());
    }
    if text.trim().chars().count() < 40 {
        return Err(
            "That page has no readable text (it may need JavaScript, which Hollow doesn't run)."
                .into(),
        );
    }
    let text: String = text.chars().take(MAX_TEXT_CHARS).collect();
    Ok(Page {
        url: url.to_string(),
        title,
        text,
    })
}

/// A page's title, its readable text, and whether it holds a password field
/// (looked for before forms are dropped).
fn html_text(html: &str) -> (String, String, bool) {
    use regex::Regex;
    use std::sync::LazyLock;
    static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    // One pattern per element, so an element ends at its own closing tag
    // (review of #1062: one alternation let `</style>` end a template).
    static DROP: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            "script", "style", "noscript", "template", "svg", "iframe", "object", "canvas", "head",
            "nav", "footer", "form",
        ]
        .iter()
        .map(|t| Regex::new(&format!(r"(?is)<{t}\b[^>]*>.*?</{t}\s*>")).unwrap())
        .collect()
    });
    static TITLE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<title\b[^>]*>(.*?)</title\s*>").unwrap());
    static MAIN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?is)<(main|article)\b[^>]*>(.*?)</(main|article)\s*>").unwrap()
    });
    static BODY: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?is)<body\b[^>]*>(.*)</body\s*>").unwrap());
    static ITEM: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)<li\b[^>]*>").unwrap());
    static CELL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)</(td|th)\s*>").unwrap());
    static BLOCK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?i)</?(p|div|br|h[1-6]|tr|table|ul|ol|section|header|blockquote|pre|dl|dt|dd|figure|figcaption|hr|main|article|aside)\b[^>]*>").unwrap()
    });
    static TAG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
    let password = has_password_field(&COMMENT.replace_all(html, " "));
    let title = TITLE
        .captures(html)
        .map(|c| {
            decode_entities(&TAG.replace_all(&c[1], " "))
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect::<String>();
    let html = COMMENT.replace_all(html, " ");
    let html = DROP.iter().fold(html.into_owned(), |acc, re| {
        re.replace_all(&acc, " ").into_owned()
    });
    let main = MAIN
        .captures_iter(&html)
        .map(|c| c[2].to_string())
        .find(|m| TAG.replace_all(m, " ").split_whitespace().count() >= 40);
    let region = main.unwrap_or_else(|| {
        BODY.captures(&html)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| html.to_string())
    });
    let region = ITEM.replace_all(&region, "\n- ");
    let region = CELL.replace_all(&region, " | ");
    let region = BLOCK.replace_all(&region, "\n");
    let region = TAG.replace_all(&region, " ");
    (title, tidy(&decode_entities(&region)), password)
}

/// Whether a page holds a real password field: an `input` whose own `type`
/// attribute is `password`, read attribute by attribute (quoted values,
/// `>` inside them included), outside scripts, styles, templates, noscript
/// and textareas, each ending at its own closing tag (review of #1062:
/// `title="type=password"` and inert markup refused public pages). A
/// template nested in a template is a known limit.
fn has_password_field(html: &str) -> bool {
    use regex::Regex;
    use std::sync::LazyLock;
    static INERT: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        ["script", "style", "template", "noscript", "textarea"]
            .iter()
            .map(|t| Regex::new(&format!(r"(?is)<{t}\b[^>]*>.*?</{t}\s*>")).unwrap())
            .collect()
    });
    // Every tag, quote-aware, left to right: markup inside another tag's
    // quoted attribute is part of that tag, never an input of its own.
    static TAG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)<([A-Za-z][\w-]*)((?:[^>"']|"[^"]*"|'[^']*')*)>"#).unwrap()
    });
    static ATTR: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?s)([^\s"'=<>/]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'>]+)))?"#).unwrap()
    });
    let live = INERT.iter().fold(html.to_string(), |acc, re| {
        re.replace_all(&acc, " ").into_owned()
    });
    TAG.captures_iter(&live)
        .filter(|tag| tag[1].eq_ignore_ascii_case("input"))
        .any(|input| {
            ATTR.captures_iter(&input[2]).any(|a| {
                let value = a
                    .get(2)
                    .or_else(|| a.get(3))
                    .or_else(|| a.get(4))
                    .map_or("", |m| m.as_str());
                a[1].eq_ignore_ascii_case("type") && value.trim().eq_ignore_ascii_case("password")
            })
        })
}

/// Lines trimmed and their runs of spaces collapsed, at most one blank line
/// in a row, control characters dropped.
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text.lines() {
        let line: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect::<String>()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let line = line.trim_matches(|c: char| c == '|' || c.is_whitespace());
        if line.is_empty() {
            blank += 1;
            if blank == 1 && !out.is_empty() {
                out.push('\n');
            }
            continue;
        }
        blank = 0;
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

/// The entities pages use most, and numeric ones.
fn decode_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // Look at most 11 bytes ahead: a page of bare ampersands must not
        // make this quadratic (review of #1062).
        let end = tail.as_bytes().iter().take(11).position(|&b| b == b';');
        let decoded = end.and_then(|e| {
            let name = &tail[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "rsquo" | "lsquo" => Some('\''),
                "rdquo" | "ldquo" => Some('"'),
                _ => name
                    .strip_prefix("#x")
                    .or_else(|| name.strip_prefix("#X"))
                    .and_then(|h| u32::from_str_radix(h, 16).ok())
                    .or_else(|| name.strip_prefix('#').and_then(|d| d.parse().ok()))
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_web_addresses_pass_the_gate() {
        for good in [
            "https://example.com/a?b=1#c",
            "http://example.org/",
            "https://93.184.216.34/x",
        ] {
            assert!(parse_page_url(good).is_ok(), "{good}");
        }
        for bad in [
            "ftp://example.com/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://user:pw@example.com/",
            "https://user@example.com/",
            "https://example.com:8443/",
            "http://localhost/",
            "http://LOCALHOST./admin",
            "http://router/",
            "http://printer.local/",
            "http://service.internal/",
            "http://127.0.0.1/",
            "http://2130706433/",
            "http://0x7f.1/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://172.16.0.1/",
            "http://169.254.169.254/latest/meta-data/",
            "http://100.64.0.1/",
            "http://[::1]/",
            "http://[fd00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:127.0.0.1]/",
            "http://0.0.0.0/",
            "not a url",
        ] {
            assert!(parse_page_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn every_redirect_is_vetted_again() {
        let from = Url::parse("https://example.com/a").unwrap();
        assert_eq!(
            redirect_target(&from, Some("/b")).unwrap().as_str(),
            "https://example.com/b"
        );
        assert!(redirect_target(&from, Some("http://169.254.169.254/")).is_err());
        assert!(redirect_target(&from, Some("http://localhost:8080/")).is_err());
        assert!(redirect_target(&from, Some("file:///etc/passwd")).is_err());
        assert!(redirect_target(&from, None).is_err());
        assert!(check_resolved(&["10.0.0.1:443".parse().unwrap()]).is_err());
        assert!(
            check_resolved(&[
                "93.184.216.34:443".parse().unwrap(),
                "127.0.0.1:443".parse().unwrap()
            ])
            .is_err(),
            "one private answer fails it all"
        );
    }

    #[test]
    fn only_html_and_plain_text_are_read() {
        assert_eq!(page_kind(Some("text/html; charset=utf-8")), Ok("html"));
        assert_eq!(page_kind(Some("application/xhtml+xml")), Ok("html"));
        assert_eq!(page_kind(Some("text/plain")), Ok("text"));
        for bad in [
            None,
            Some("application/pdf"),
            Some("image/png"),
            Some("application/json"),
        ] {
            assert!(page_kind(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_page_reads_as_its_main_text() {
        let words = "Revenue grew in the third quarter across every region we serve. ".repeat(4);
        let html = format!(
            "<html><head><title>Q3 &amp; results</title><style>p{{color:red}}</style><script>alert('x')</script></head>\
             <body><nav>Home | About</nav><main><h1>Results</h1><p>{words}</p><ul><li>North</li><li>South</li></ul>\
             <table><tr><td>Plan</td><td>$12</td></tr></table><!-- hidden --></main><footer>© 2026</footer></body></html>"
        );
        let page = read_page("https://example.com/q3", &html, true).unwrap();
        assert_eq!(page.title, "Q3 & results");
        assert!(page.text.starts_with("Results\n"), "{}", page.text);
        assert!(page.text.contains("- North\n- South"), "{}", page.text);
        assert!(page.text.contains("Plan | $12"), "{}", page.text);
        for gone in ["alert", "color:red", "Home | About", "© 2026", "hidden"] {
            assert!(!page.text.contains(gone), "{gone} in {}", page.text);
        }
        assert_eq!(
            decode_entities("a&#38;b &#x41; &lt;c&gt; &bogus; & x"),
            "a&b A <c> &bogus; & x"
        );
    }

    #[test]
    fn sign_in_walls_and_browser_checks_are_refused() {
        let check = "<html><head><title>Just a moment...</title></head><body>Checking your browser before accessing.</body></html>";
        assert!(read_page("https://x.test/", check, true)
            .unwrap_err()
            .contains("sign-in or a browser check"));
        let wall = "<html><body><h1>Sign in to continue</h1><p>Log in with your account.</p></body></html>";
        assert!(read_page("https://x.test/", wall, true)
            .unwrap_err()
            .contains("public pages only"));
        // A long sign-in page is still a wall when it asks for a password.
        let long_wall = format!(
            "<html><head><title>Welcome</title></head><body><p>{}</p><form><input type=\"password\"></form></body></html>",
            "Our product helps teams. ".repeat(60)
        );
        assert!(read_page("https://x.test/", &long_wall, true).is_err());
        // Inert or look-alike password markup doesn't make a wall.
        let inert = format!(
            "<html><body><p>{}</p><!-- <input type=password> --><script>'<input type=\"password\">'</script><input data-type=\"password\"></body></html>",
            "A public article about account security. ".repeat(10)
        );
        assert!(read_page("https://x.test/", &inert, true).is_ok());
        assert!(!has_password_field(
            r#"<input type="text" title="Use type=password">"#
        ));
        assert!(!has_password_field(
            "<template><style>p{}</style><input type=password></template>"
        ));
        assert!(has_password_field(
            r#"<input class="a>b" TYPE = 'Password' name="pw">"#
        ));
        assert!(has_password_field(
            "<form><input id=p type=password></form>"
        ));
        assert!(!has_password_field(
            r#"<div data-content="<input type='password'>">Text</div>"#
        ));
        // An article about captchas is not a browser check.
        let article = format!(
            "<html><head><title>How captcha systems work</title></head><body><article><p>{}</p></article></body></html>",
            "Captcha tests tell people and programs apart in many ways. ".repeat(60)
        );
        assert!(read_page("https://x.test/captcha", &article, true).is_ok());
        // Bare ampersands stay linear and literal.
        assert_eq!(decode_entities(&"&".repeat(5)), "&&&&&");
        let many = "&".repeat(1_000_000);
        let started = std::time::Instant::now();
        assert_eq!(decode_entities(&many).len(), many.len());
        // timing: a quadratic decoder takes minutes on this input; 2 s is generous.
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        assert!(read_page(
            "https://x.test/",
            "<html><body><div id=app></div></body></html>",
            true
        )
        .unwrap_err()
        .contains("no readable text"));
        let plain = "Plain text notes that are long enough to read as a page of their own words.";
        assert_eq!(
            read_page("https://x.test/a.txt", plain, false)
                .unwrap()
                .text,
            plain
        );
        let long = format!(
            "<html><body><p>{}</p></body></html>",
            "word ".repeat(30_000)
        );
        assert_eq!(
            read_page("https://x.test/", &long, true)
                .unwrap()
                .text
                .chars()
                .count(),
            MAX_TEXT_CHARS
        );
    }
}
