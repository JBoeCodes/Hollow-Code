//! The importer's own Notion client: read only, by construction.
//!
//! Every request it can send is one [`Request`] variant, and the variants
//! are closed: three GET reads and the one data-source query POST
//! (`/v1/data_sources/{id}/query`). There is no method, path or body
//! parameter anywhere, so no other request can be built (the test
//! `the_notion_client_can_only_read` checks both the variants and this
//! file's text). Working Files are fetched by [`NotionClient::file`], a plain
//! GET with no Authorization header, capped while reading.
//!
//! The token lives in one sensitive header value and nowhere else; every
//! error message goes through [`NotionClient::scrub`] first, and error
//! bodies are cut to Notion's `code` and a short message.

use reqwest::header::{HeaderValue, AUTHORIZATION};
use reqwest::Method;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub(crate) const API: &str = "https://api.notion.com/v1";
/// Sequential requests, one every 350 ms, as Notion sync always paced them.
pub(crate) const PACE: Duration = Duration::from_millis(350);
const TIMEOUT: Duration = Duration::from_secs(60);
/// Pages of 100 times this many stays far above any data source here.
const MAX_PAGES: usize = 200;

/// Every request the client can make. Adding a write means adding a variant
/// here, which the read-only test refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    /// `GET /v1/pages/{page}/properties/{property}`: a relation longer than
    /// the 25 items a page object carries.
    PageProperty {
        page: String,
        property: String,
        cursor: Option<String>,
    },
    /// `GET /v1/blocks/{id}/children`: page content.
    BlockChildren { id: String, cursor: Option<String> },
    /// `GET /v1/data_sources/{id}`: the source's title and schema.
    DataSource { id: String },
    /// `POST /v1/data_sources/{id}/query`: one page of rows. The only POST.
    Query { id: String, cursor: Option<String> },
}

/// A Notion id as dashed or plain hex; anything else is refused.
fn id(raw: &str) -> Result<String, String> {
    let plain: String = raw.chars().filter(|c| *c != '-').collect();
    if plain.len() == 32 && plain.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(plain.to_ascii_lowercase())
    } else {
        Err("A Notion id isn't 32 hex digits.".into())
    }
}

fn cursor(raw: &str) -> Result<&str, String> {
    if !raw.is_empty()
        && raw.len() <= 64
        && raw.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    {
        Ok(raw)
    } else {
        Err("A Notion cursor has unexpected characters.".into())
    }
}

/// Property ids are short and URL-encoded already; encode anything else.
fn property(raw: &str) -> Result<String, String> {
    if raw.is_empty() || raw.len() > 64 {
        return Err("A Notion property id is empty or too long.".into());
    }
    Ok(raw
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'%' | b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect())
}

impl Request {
    pub(crate) fn method(&self) -> Method {
        match self {
            Request::Query { .. } => Method::POST,
            Request::PageProperty { .. }
            | Request::BlockChildren { .. }
            | Request::DataSource { .. } => Method::GET,
        }
    }

    /// The path under `/v1`, with its query string.
    pub(crate) fn path(&self) -> Result<String, String> {
        let page_size = |cursor_value: &Option<String>| -> Result<String, String> {
            Ok(match cursor_value {
                Some(c) => format!("?page_size=100&start_cursor={}", cursor(c)?),
                None => "?page_size=100".into(),
            })
        };
        Ok(match self {
            Request::PageProperty {
                page,
                property: prop,
                cursor: c,
            } => format!(
                "/pages/{}/properties/{}{}",
                id(page)?,
                property(prop)?,
                page_size(c)?
            ),
            Request::BlockChildren {
                id: block,
                cursor: c,
            } => {
                format!("/blocks/{}/children{}", id(block)?, page_size(c)?)
            }
            Request::DataSource { id: source } => format!("/data_sources/{}", id(source)?),
            Request::Query { id: source, .. } => format!("/data_sources/{}/query", id(source)?),
        })
    }

    fn body(&self) -> Option<Value> {
        match self {
            Request::Query { cursor: c, .. } => {
                let mut body = json!({ "page_size": 100 });
                if let Some(c) = c {
                    body["start_cursor"] = json!(c);
                }
                Some(body)
            }
            _ => None,
        }
    }
}

/// A Working File: its bytes, or its size when it is over the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Fetched {
    Bytes(Vec<u8>),
    TooLarge(u64),
}

pub(crate) struct NotionClient {
    /// The API client: redirects off, so no reply can send the token (or the
    /// query POST) anywhere else.
    http: reqwest::Client,
    /// Working File downloads only: a few redirects, never an Authorization.
    files: reqwest::Client,
    base: String,
    auth: HeaderValue,
    /// Kept only so [`NotionClient::scrub`] can remove it from messages.
    secret: String,
    version: String,
    pace: Duration,
    last: Mutex<Option<Instant>>,
    /// Loopback file URLs (the tests' stub) are allowed only in tests.
    allow_loopback: bool,
}

impl std::fmt::Debug for NotionClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionClient")
            .field("base", &self.base)
            .finish_non_exhaustive()
    }
}

impl NotionClient {
    pub(crate) fn new(token: &str, version: &str) -> Result<Self, String> {
        Self::build(API, token, version, PACE, false)
    }

    #[cfg(test)]
    pub(crate) fn with_base(base: &str, token: &str) -> Result<Self, String> {
        Self::build(base, token, "2026-03-11", Duration::ZERO, true)
    }

    fn build(
        base: &str,
        token: &str,
        version: &str,
        pace: Duration,
        allow_loopback: bool,
    ) -> Result<Self, String> {
        let token = token.trim();
        if token.is_empty() {
            return Err("No Notion token is saved in the config.".into());
        }
        let mut auth = HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "The saved Notion token has characters a token can't have.".to_string())?;
        auth.set_sensitive(true);
        let client = |redirects: reqwest::redirect::Policy| {
            reqwest::Client::builder()
                .timeout(TIMEOUT)
                .redirect(redirects)
                .build()
                .map_err(|_| "Couldn't start the Notion client.".to_string())
        };
        Ok(NotionClient {
            http: client(reqwest::redirect::Policy::none())?,
            files: client(reqwest::redirect::Policy::limited(5))?,
            base: base.trim_end_matches('/').to_string(),
            auth,
            secret: token.to_string(),
            version: version.to_string(),
            pace,
            last: Mutex::new(None),
            allow_loopback,
        })
    }

    /// The text with the token (and anything shaped like a Notion token)
    /// replaced. Every error this client returns has passed through here.
    pub(crate) fn scrub(&self, text: &str) -> String {
        super::scrub(text, &[self.secret.as_str()])
    }

    async fn wait_turn(&self) {
        let wait = {
            let mut last = self.last.lock().unwrap_or_else(|p| p.into_inner());
            let now = Instant::now();
            let wait = last
                .map(|at| self.pace.saturating_sub(now.duration_since(at)))
                .unwrap_or_default();
            *last = Some(now + wait);
            wait
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }

    /// Send one request; JSON back, or a scrubbed error.
    pub(crate) async fn send(&self, request: &Request) -> Result<Value, String> {
        let path = request.path()?;
        self.wait_turn().await;
        let url = format!("{}{path}", self.base);
        let builder = match request.method() {
            Method::POST => self.http.post(&url),
            _ => self.http.get(&url),
        };
        let mut builder = builder
            .header(AUTHORIZATION, self.auth.clone())
            .header("Notion-Version", &self.version);
        if let Some(body) = request.body() {
            builder = builder.json(&body);
        }
        let response = builder
            .send()
            .await
            .map_err(|e| self.scrub(&format!("Couldn't reach Notion: {}", e.without_url())))?;
        let status = response.status();
        let text = response.text().await.map_err(|e| {
            self.scrub(&format!(
                "Notion's reply didn't arrive: {}",
                e.without_url()
            ))
        })?;
        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        if !status.is_success() {
            let code = value["code"].as_str().unwrap_or("error");
            // Redact the whole message first, so a cut can't split a secret.
            let message: String = self
                .scrub(value["message"].as_str().unwrap_or("request failed"))
                .chars()
                .take(200)
                .collect();
            let code = self.scrub(code);
            return Err(format!(
                "Notion refused a read ({}, {code}): {message}",
                status.as_u16()
            ));
        }
        if value.is_null() {
            return Err("Notion's reply wasn't JSON.".into());
        }
        Ok(value)
    }

    async fn paged(
        &self,
        mut next: impl FnMut(Option<String>) -> Request,
    ) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        let mut at = None;
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_PAGES {
            let value = self.send(&next(at.clone())).await?;
            out.extend(
                value["results"]
                    .as_array()
                    .ok_or("A Notion list reply has no results.")?
                    .iter()
                    .cloned(),
            );
            if value["has_more"] != true {
                return Ok(out);
            }
            let cursor = value["next_cursor"]
                .as_str()
                .ok_or("A Notion list reply has no cursor.")?
                .to_string();
            if !seen.insert(cursor.clone()) {
                return Err("Notion repeated a page cursor.".into());
            }
            at = Some(cursor);
        }
        Err("A Notion list is longer than the importer reads.".into())
    }

    /// Every row of a data source.
    pub(crate) async fn query_all(&self, source: &str) -> Result<Vec<Value>, String> {
        self.paged(|cursor| Request::Query {
            id: source.to_string(),
            cursor,
        })
        .await
    }

    pub(crate) async fn children_all(&self, block: &str) -> Result<Vec<Value>, String> {
        self.paged(|cursor| Request::BlockChildren {
            id: block.to_string(),
            cursor,
        })
        .await
    }

    pub(crate) async fn property_all(
        &self,
        page: &str,
        property: &str,
    ) -> Result<Vec<Value>, String> {
        self.paged(|cursor| Request::PageProperty {
            page: page.to_string(),
            property: property.to_string(),
            cursor,
        })
        .await
    }

    /// A Working File's bytes, read up to `cap`. Signed file links carry
    /// their own authority, so no Authorization header is sent.
    pub(crate) async fn file(&self, url: &str, cap: u64) -> Result<Fetched, String> {
        let parsed = reqwest::Url::parse(url)
            .map_err(|_| "A Working File link is malformed.".to_string())?;
        let loopback = parsed.host_str() == Some("127.0.0.1");
        if parsed.scheme() != "https" && !(self.allow_loopback && loopback) {
            return Err("A Working File link isn't https.".into());
        }
        self.wait_turn().await;
        let mut response = self.files.get(parsed).send().await.map_err(|e| {
            self.scrub(&format!(
                "Couldn't download a Working File: {}",
                e.without_url()
            ))
        })?;
        if !response.status().is_success() {
            return Err(format!(
                "A Working File download failed ({}).",
                response.status().as_u16()
            ));
        }
        if let Some(size) = response.content_length().filter(|size| *size > cap) {
            return Ok(Fetched::TooLarge(size));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|e| {
            self.scrub(&format!(
                "A Working File download broke off: {}",
                e.without_url()
            ))
        })? {
            bytes.extend_from_slice(&chunk);
            if bytes.len() as u64 > cap {
                return Ok(Fetched::TooLarge(bytes.len() as u64));
            }
        }
        Ok(Fetched::Bytes(bytes))
    }
}
