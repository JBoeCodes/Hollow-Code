//! The built-in Microsoft 365 connector: an MCP stdio server (hidden
//! `hollow m365-mcp` subcommand) on Hollow's own Microsoft sign-in
//! (`m365_auth.rs`), so it works for every Claude account and for Codex.
//! Attached through the connectors library like any connector
//! (`builtin_connectors.rs`).
//!
//! Read-only, and calendar and files only: `/me`, the calendar, OneDrive and
//! SharePoint (search, recent, folders, file text). Hollow has no messaging,
//! so there is no mail or Teams tool; [`allowed_path`] refuses those routes
//! before any token is read. Tokens are refreshed in memory and never saved
//! from this process (see `m365_auth`).
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const DEFAULT_PROTOCOL: &str = "2024-11-05";
const GRAPH: &str = "https://graph.microsoft.com/v1.0";
/// Largest file `read_file` downloads.
const MAX_DOWNLOAD: u64 = 15 * 1024 * 1024;
/// Most text `read_file` returns.
const MAX_TEXT: usize = 60_000;
/// The Office reader's caps for a OneDrive file: [`MAX_TEXT`] characters are
/// kept, so extracting more than about that many bytes (4 per character at
/// most) is wasted work, as before the reader moved to `doc_text`.
const ONEDRIVE_LIMITS: crate::doc_text::Limits = crate::doc_text::Limits {
    text_bytes: MAX_TEXT * 4,
    ..crate::doc_text::LIMITS
};
/// Most pages a list read follows through `@odata.nextLink` before it stops
/// and says the result is partial.
const MAX_PAGES: usize = 10;
/// Longest a list read keeps following pages.
const LIST_DEADLINE: Duration = Duration::from_secs(60);

pub fn serve() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio current-thread runtime");
    rt.block_on(serve_async());
}

struct Ctx {
    http: reqwest::Client,
    /// The Graph root ([`GRAPH`]; a local mock in tests). Only links under it
    /// are followed, so the bearer never leaves Graph.
    base: String,
    tokens: tokio::sync::Mutex<Option<crate::m365_auth::Tokens>>,
}

async fn serve_async() {
    let ctx = Ctx {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(45))
            .build()
            .expect("http client"),
        base: GRAPH.to_string(),
        tokens: tokio::sync::Mutex::new(None),
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = tokio::io::stdout();
    while let Ok(Some(raw)) = lines.next_line().await {
        let Ok(req) = serde_json::from_str::<Value>(raw.trim()) else {
            continue;
        };
        if let Some(resp) = handle(&req, &ctx).await {
            if stdout
                .write_all(format!("{resp}\n").as_bytes())
                .await
                .is_err()
            {
                break;
            }
            let _ = stdout.flush().await;
        }
    }
}

async fn handle(req: &Value, ctx: &Ctx) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let res = match method {
        "initialize" => json!({
            "protocolVersion": req.pointer("/params/protocolVersion").and_then(Value::as_str).unwrap_or(DEFAULT_PROTOCOL),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "hollow-m365", "version": env!("CARGO_PKG_VERSION")},
        }),
        "tools/list" => json!({"tools": tool_specs()}),
        "tools/call" => {
            let name = req.pointer("/params/name").and_then(Value::as_str).unwrap_or("");
            let args = req.pointer("/params/arguments").cloned().unwrap_or(Value::Null);
            match call(name, &args, ctx).await {
                Ok(text) => json!({"content": [{"type": "text", "text": text}]}),
                Err(e) => json!({"isError": true, "content": [{"type": "text", "text": e}]}),
            }
        }
        "ping" => json!({}),
        _ => return id.map(|id| json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("method not found: {method}")}})),
    };
    id.map(|id| json!({"jsonrpc": "2.0", "id": id, "result": res}))
}

fn read_only() -> Value {
    json!({"readOnlyHint": true, "openWorldHint": true})
}

fn tool_specs() -> Vec<Value> {
    vec![
        json!({"name": "whoami", "description": "The signed-in Microsoft 365 user (name, email, job title).", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {}}}),
        json!({"name": "list_events", "description": "Calendar events between two times, in the Mac's local time zone. Defaults: from the start of today for 7 days. Dates (YYYY-MM-DD) mean local midnight.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {
            "start": {"type": "string", "description": "Start, YYYY-MM-DD or an RFC 3339 time."},
            "end": {"type": "string", "description": "End (exclusive), YYYY-MM-DD or an RFC 3339 time."},
            "limit": {"type": "integer", "description": "Most events (default 50, max 200)."}}}}),
        json!({"name": "get_event", "description": "One calendar event in full: attendees and their responses, body preview, join link.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}}, "required": ["id"]}}),
        json!({"name": "search_files", "description": "Search OneDrive and SharePoint files the user can open (Microsoft Search; keywords, KQL allowed). Returns drive and item ids for read_file.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {
            "query": {"type": "string"}, "limit": {"type": "integer", "description": "Most results (default 15, max 50)."}}, "required": ["query"]}}),
        json!({"name": "recent_files", "description": "Files the user opened or edited recently in OneDrive and SharePoint.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {"limit": {"type": "integer"}}}}),
        json!({"name": "list_folder", "description": "Items in a OneDrive or SharePoint folder. Without ids, the root of the user's OneDrive.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {
            "drive_id": {"type": "string"}, "item_id": {"type": "string", "description": "A folder's item id (needs drive_id)."}}}}),
        json!({"name": "read_file", "description": "A file's details and its text: plain text, Markdown, CSV and JSON as is; Word and PowerPoint as extracted text; Excel as typed cells by sheet and row (text quoted, numbers, booleans, errors, dates, formulas with their cached values), capped and marked [truncated] when cut. Other types (PDF, images) return details and a link only. Up to 15 MB.", "annotations": read_only(), "inputSchema": {"type": "object", "properties": {
            "drive_id": {"type": "string"}, "item_id": {"type": "string"}}, "required": ["drive_id", "item_id"]}}),
    ]
}

// ── Graph ──────────────────────────────────────────────────────────────────

/// Graph routes this server may call. Everything else, mail and Teams above
/// all, is refused before a token is read.
fn allowed_path(path: &str) -> bool {
    let p = path.split('?').next().unwrap_or("");
    let lower = p.to_ascii_lowercase();
    if lower.contains("..") || lower.contains("%2e") || lower.contains("//") {
        return false;
    }
    let banned = [
        "/messages",
        "/mailfolders",
        "/sendmail",
        "/chats",
        "/teams",
        "/channels",
        "/onlinemeetings",
        "/contacts",
    ];
    if banned.iter().any(|b| lower.contains(b)) {
        return false;
    }
    lower == "/me"
        || lower == "/me/calendarview"
        || (lower.starts_with("/me/events/") && lower.matches('/').count() == 3)
        || lower.starts_with("/me/drive/")
        || lower.starts_with("/drives/")
        || lower == "/search/query"
}

async fn token(ctx: &Ctx) -> Result<String, String> {
    Ok(crate::m365_auth::access_token_readonly(&ctx.tokens)
        .await?
        .access_token)
}

async fn graph(
    ctx: &Ctx,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    if !allowed_path(path) {
        return Err(format!(
            "Hollow's Microsoft 365 connector doesn't open {path}"
        ));
    }
    let bearer = token(ctx).await?;
    let mut req = ctx
        .http
        .request(method, format!("{}{path}", ctx.base))
        .bearer_auth(bearer);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let res = req
        .send()
        .await
        .map_err(|e| format!("couldn't reach Microsoft Graph: {e}"))?;
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| {
                v.pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .unwrap_or_else(|| crate::claude_runner::truncate(&text, 200).to_string());
        let hint = if status.as_u16() == 403 {
            " (the sign-in may lack this permission: sign in again in Hollow → Settings → Connectors, or ask IT to approve it)"
        } else {
            ""
        };
        return Err(format!("Microsoft Graph {status}: {message}{hint}"));
    }
    serde_json::from_str(&text).map_err(|e| format!("unreadable Graph reply: {e}"))
}

/// A Graph collection read across `@odata.nextLink` pages.
struct Paged {
    items: Vec<Value>,
    /// Why the read stopped before Graph's last page; `None` when complete.
    stopped: Option<String>,
}

impl Paged {
    /// `complete` and `partial` (the reason), for the reply.
    fn coverage(&self, out: &mut Value) {
        out["complete"] = json!(self.stopped.is_none());
        out["partial"] = json!(self.stopped);
    }
}

/// GET `path` and follow `@odata.nextLink` until Graph's last page,
/// `max_items`, [`MAX_PAGES`] pages or [`LIST_DEADLINE`]. A next link is
/// followed only under this connector's Graph root and still passes
/// [`allowed_path`]. A failing first page is the caller's error; a failing
/// later page keeps what was read and says so.
async fn graph_list(ctx: &Ctx, path: &str, max_items: usize) -> Result<Paged, String> {
    let began = std::time::Instant::now();
    let mut items: Vec<Value> = Vec::new();
    let mut path = path.to_string();
    for page in 0..MAX_PAGES {
        if page > 0 && began.elapsed() >= LIST_DEADLINE {
            return Ok(Paged {
                items,
                stopped: Some(format!("stopped after {} s", LIST_DEADLINE.as_secs())),
            });
        }
        let v = match graph(ctx, reqwest::Method::GET, &path, None).await {
            Ok(v) => v,
            Err(e) if page == 0 => return Err(e),
            Err(e) => {
                return Ok(Paged {
                    items,
                    stopped: Some(format!("page {} failed: {e}", page + 1)),
                })
            }
        };
        items.extend(v["value"].as_array().into_iter().flatten().cloned());
        let Some(next) = v["@odata.nextLink"].as_str() else {
            if items.len() > max_items {
                items.truncate(max_items);
                return Ok(Paged {
                    items,
                    stopped: Some(format!(
                        "stopped at {max_items} items; Microsoft 365 has more"
                    )),
                });
            }
            return Ok(Paged {
                items,
                stopped: None,
            });
        };
        if items.len() >= max_items {
            items.truncate(max_items);
            return Ok(Paged {
                items,
                stopped: Some(format!(
                    "stopped at {max_items} items; Microsoft 365 has more"
                )),
            });
        }
        match next
            .strip_prefix(ctx.base.as_str())
            .filter(|rest| rest.starts_with('/'))
        {
            Some(rest) => path = rest.to_string(),
            None => {
                return Ok(Paged {
                    items,
                    stopped: Some(
                        "Graph's next page is outside Microsoft Graph, so Hollow didn't follow it"
                            .into(),
                    ),
                })
            }
        }
    }
    Ok(Paged {
        items,
        stopped: Some(format!("stopped after {MAX_PAGES} pages")),
    })
}

fn arg<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn limit(args: &Value, default: u64, max: u64) -> u64 {
    args.get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .clamp(1, max)
}

/// A Graph id segment: letters, digits and the few marks Graph ids use.
fn id_segment(s: &str) -> Result<String, String> {
    if s.is_empty()
        || s.len() > 512
        || !s
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_!=.+".contains(&b))
    {
        return Err(format!("not a Microsoft 365 id: {s}"));
    }
    Ok(urlencoding::encode(s).into_owned())
}

async fn call(name: &str, args: &Value, ctx: &Ctx) -> Result<String, String> {
    let out = match name {
        "whoami" => {
            let me = graph(
                ctx,
                reqwest::Method::GET,
                "/me?$select=displayName,mail,userPrincipalName,jobTitle,officeLocation",
                None,
            )
            .await?;
            json!({"name": me["displayName"], "email": me.get("mail").filter(|m| !m.is_null()).unwrap_or(&me["userPrincipalName"]), "jobTitle": me["jobTitle"], "office": me["officeLocation"]})
        }
        "list_events" => {
            let (start, end) = window(arg(args, "start"), arg(args, "end"))?;
            calendar(ctx, &start, &end, limit(args, 50, 200)).await?
        }
        "get_event" => {
            let id = id_segment(arg(args, "id").ok_or("`id` is required")?)?;
            let path = format!("/me/events/{id}?$select=id,subject,start,end,location,isAllDay,isOnlineMeeting,onlineMeeting,organizer,attendees,bodyPreview,showAs,isCancelled,webLink");
            let e = graph(ctx, reqwest::Method::GET, &path, None).await?;
            let mut s = event_summary(&e);
            s["bodyPreview"] = e["bodyPreview"].clone();
            s["attendees"] = json!(e["attendees"].as_array().into_iter().flatten().take(50).map(|a| json!({
                "name": a.pointer("/emailAddress/name"), "email": a.pointer("/emailAddress/address"),
                "type": a["type"], "response": a.pointer("/status/response")})).collect::<Vec<_>>());
            s
        }
        "search_files" => {
            let query = arg(args, "query").ok_or("`query` is required")?;
            let body = json!({"requests": [{"entityTypes": ["driveItem"], "query": {"queryString": query}, "from": 0, "size": limit(args, 15, 50)}]});
            let v = graph(ctx, reqwest::Method::POST, "/search/query", Some(body)).await?;
            let hits: Vec<Value> = v
                .pointer("/value/0/hitsContainers/0/hits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .map(|h| {
                    let mut f = file_summary(&h["resource"]);
                    f["summary"] = h["summary"].clone();
                    f
                })
                .collect();
            let more =
                v.pointer("/value/0/hitsContainers/0/moreResultsAvailable") == Some(&json!(true));
            let partial =
                more.then_some("more matches than `limit`; narrow the query or raise limit");
            json!({"query": query, "files": hits, "complete": !more, "partial": partial})
        }
        "recent_files" => {
            let n = limit(args, 20, 100);
            let p = graph_list(ctx, &format!("/me/drive/recent?$top={n}"), n as usize).await?;
            let mut out = json!({"files": p.items.iter().map(|f| file_summary(f.get("remoteItem").unwrap_or(f))).collect::<Vec<_>>()});
            p.coverage(&mut out);
            out
        }
        "list_folder" => {
            let path = match (arg(args, "drive_id"), arg(args, "item_id")) {
                (Some(d), Some(i)) => format!(
                    "/drives/{}/items/{}/children?$top=200",
                    id_segment(d)?,
                    id_segment(i)?
                ),
                (Some(d), None) => format!("/drives/{}/root/children?$top=200", id_segment(d)?),
                (None, None) => "/me/drive/root/children?$top=200".to_string(),
                (None, Some(_)) => return Err("`item_id` needs `drive_id`".into()),
            };
            let p = graph_list(ctx, &path, 1000).await?;
            let mut out = json!({"items": p.items.iter().map(file_summary).collect::<Vec<_>>()});
            p.coverage(&mut out);
            out
        }
        "read_file" => return read_file(ctx, args).await,
        _ => return Err(format!("unknown tool: {name}")),
    };
    Ok(serde_json::to_string_pretty(&out).unwrap_or_default())
}

fn event_summary(e: &Value) -> Value {
    json!({
        "id": e["id"], "subject": e["subject"],
        "start": local_time(&e["start"]), "end": local_time(&e["end"]),
        "allDay": e["isAllDay"], "cancelled": e["isCancelled"], "showAs": e["showAs"],
        "location": e.pointer("/location/displayName"),
        "organizer": e.pointer("/organizer/emailAddress/name"),
        "joinUrl": e.pointer("/onlineMeeting/joinUrl"),
        "bodyPreview": e["bodyPreview"],
        "webLink": e["webLink"],
    })
}

fn file_summary(f: &Value) -> Value {
    json!({
        "name": f["name"], "driveId": f.pointer("/parentReference/driveId"), "itemId": f["id"],
        "folder": f.get("folder").is_some(), "size": f["size"], "modified": f["lastModifiedDateTime"],
        "modifiedBy": f.pointer("/lastModifiedBy/user/displayName"), "webUrl": f["webUrl"],
    })
}

/// `calendarView` for `[start, end)` (UTC RFC 3339), across Graph's pages,
/// at most `limit_n` events: `{from, to, timeZone, events, complete, partial}`.
async fn calendar(ctx: &Ctx, start: &str, end: &str, limit_n: u64) -> Result<Value, String> {
    let n = limit_n.clamp(1, 200);
    let path = format!(
        "/me/calendarView?startDateTime={}&endDateTime={}&$top={n}&$orderby=start/dateTime&$select=id,subject,start,end,location,isAllDay,isOnlineMeeting,onlineMeeting,organizer,bodyPreview,showAs,isCancelled,webLink",
        urlencoding::encode(start), urlencoding::encode(end)
    );
    let p = graph_list(ctx, &path, n as usize).await?;
    let mut out = json!({"from": start, "to": end, "timeZone": local_zone_name(), "events": p.items.iter().map(event_summary).collect::<Vec<_>>()});
    p.coverage(&mut out);
    Ok(out)
}

/// Calendar events for Hollow's own Agenda (next-level P10): the same
/// read-only `calendarView` route, paging and summary the agent tool uses, on
/// Hollow's own sign-in. Never writes; the route allowlist above still applies.
pub(crate) async fn agenda_events(start: &str, end: &str, limit_n: u64) -> Result<Value, String> {
    let ctx = Ctx {
        http: reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?,
        base: GRAPH.to_string(),
        tokens: tokio::sync::Mutex::new(None),
    };
    let (start, end) = window(Some(start), Some(end))?;
    calendar(&ctx, &start, &end, limit_n).await
}

/// Each event's attendees for Hollow's meeting prep (morning N4): one
/// read-only `GET /me/events/{id}?$select=id,attendees` per event, on
/// Hollow's own sign-in, as (name, email) pairs (at most 50 an event; rooms
/// and equipment left out). Five at a time, 5 s each; after the first failed
/// read (a refresh or the network) the rest aren't tried. An event that
/// wasn't read is missing; the caller keeps going.
pub(crate) async fn agenda_attendees(
    ids: &[String],
) -> std::collections::BTreeMap<String, Result<Vec<(String, String)>, String>> {
    let mut out = std::collections::BTreeMap::new();
    let ctx = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(http) => Ctx {
            http,
            base: GRAPH.to_string(),
            tokens: tokio::sync::Mutex::new(None),
        },
        Err(e) => {
            for id in ids {
                out.insert(id.clone(), Err(e.to_string()));
            }
            return out;
        }
    };
    let ctx = std::sync::Arc::new(ctx);
    for batch in ids.chunks(5) {
        let mut set = tokio::task::JoinSet::new();
        for id in batch.iter().cloned() {
            let ctx = ctx.clone();
            set.spawn(async move {
                let got = async {
                    let seg = id_segment(&id)?;
                    let e = graph(
                        &ctx,
                        reqwest::Method::GET,
                        &format!("/me/events/{seg}?$select=id,attendees"),
                        None,
                    )
                    .await?;
                    Ok::<_, String>(attendee_pairs(&e))
                }
                .await;
                (id, got)
            });
        }
        let mut failed = false;
        while let Some(done) = set.join_next().await {
            if let Ok((id, got)) = done {
                failed |= got.is_err();
                out.insert(id, got);
            }
        }
        if failed {
            break;
        }
    }
    out
}

/// An event's attendees as (name, email), at most 50.
fn attendee_pairs(e: &Value) -> Vec<(String, String)> {
    e["attendees"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["type"].as_str() != Some("resource"))
        .take(50)
        .map(|a| {
            let get = |p: &str| {
                a.pointer(p)
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string()
            };
            (get("/emailAddress/name"), get("/emailAddress/address"))
        })
        .filter(|(n, e)| !n.is_empty() || !e.is_empty())
        .collect()
}

// ── time ───────────────────────────────────────────────────────────────────

/// The Mac's zone name (`America/New_York`), for the reader.
fn local_zone_name() -> String {
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            p.to_string_lossy()
                .split("zoneinfo/")
                .nth(1)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "local".into())
}

/// A Graph `{dateTime, timeZone: "UTC"}` as local RFC 3339.
fn local_time(v: &Value) -> Value {
    let Some(dt) = v.get("dateTime").and_then(Value::as_str) else {
        return Value::Null;
    };
    let trimmed = dt.split('.').next().unwrap_or(dt);
    match chrono::NaiveDateTime::parse_from_str(trimmed, "%Y-%m-%dT%H:%M:%S") {
        Ok(naive) if v.get("timeZone").and_then(Value::as_str).unwrap_or("UTC") == "UTC" => {
            json!(
                chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(naive, chrono::Utc)
                    .with_timezone(&chrono::Local)
                    .to_rfc3339()
            )
        }
        _ => json!(dt),
    }
}

/// `start`/`end` as UTC RFC 3339, local dates meaning local midnight.
pub(crate) fn window(start: Option<&str>, end: Option<&str>) -> Result<(String, String), String> {
    fn parse(s: &str) -> Result<chrono::DateTime<chrono::Utc>, String> {
        if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
            return Ok(t.with_timezone(&chrono::Utc));
        }
        let date = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .map_err(|_| format!("use YYYY-MM-DD or an RFC 3339 time, not {s}"))?;
        local_midnight(date)
    }
    let start = match start {
        Some(s) => parse(s)?,
        None => local_midnight(chrono::Local::now().date_naive())?,
    };
    let end = match end {
        Some(s) => parse(s)?,
        None => start + chrono::Duration::days(7),
    };
    if end <= start || end - start > chrono::Duration::days(93) {
        return Err("`end` must be after `start`, at most 93 days later".into());
    }
    Ok((
        start.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        end.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    ))
}

fn local_midnight(date: chrono::NaiveDate) -> Result<chrono::DateTime<chrono::Utc>, String> {
    use chrono::TimeZone;
    chrono::Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight"))
        .earliest()
        .map(|t| t.with_timezone(&chrono::Utc))
        .ok_or_else(|| format!("{date} has no local midnight"))
}

// ── file text ──────────────────────────────────────────────────────────────

async fn read_file(ctx: &Ctx, args: &Value) -> Result<String, String> {
    let drive = id_segment(arg(args, "drive_id").ok_or("`drive_id` is required")?)?;
    let item = id_segment(arg(args, "item_id").ok_or("`item_id` is required")?)?;
    let meta = graph(
        ctx,
        reqwest::Method::GET,
        &format!("/drives/{drive}/items/{item}"),
        None,
    )
    .await?;
    let mut out = file_summary(&meta);
    let name = meta["name"].as_str().unwrap_or("").to_ascii_lowercase();
    let size = meta["size"].as_u64().unwrap_or(0);
    let ext = name.rsplit('.').next().unwrap_or("");
    let kind = match ext {
        "txt" | "md" | "markdown" | "csv" | "tsv" | "json" | "log" | "xml" | "html" | "htm"
        | "yaml" | "yml" => "text",
        "docx" | "pptx" | "xlsx" => ext,
        _ => "",
    };
    if meta.get("folder").is_some() {
        out["note"] = json!("This is a folder: use list_folder.");
    } else if kind.is_empty() {
        out["note"] = json!("Hollow can't read text from this file type; open it from webUrl.");
    } else if size > MAX_DOWNLOAD {
        out["note"] = json!(format!(
            "Larger than {} MB; open it from webUrl.",
            MAX_DOWNLOAD / 1024 / 1024
        ));
    } else {
        // The pre-authenticated download URL takes no bearer, so the token
        // never goes to the storage host.
        let url = meta
            .get("@microsoft.graph.downloadUrl")
            .and_then(Value::as_str)
            .ok_or("Microsoft gave no download link for this file")?;
        let bytes = ctx
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| e.to_string())?
            .bytes()
            .await
            .map_err(|e| e.to_string())?;
        let text = match kind {
            "text" => String::from_utf8_lossy(&bytes).into_owned(),
            office => crate::doc_text::office_text(&bytes, office, &ONEDRIVE_LIMITS)?,
        };
        let total = text.chars().count();
        out["text"] = json!(text.chars().take(MAX_TEXT).collect::<String>());
        if total > MAX_TEXT {
            out["truncated"] = json!(format!("first {MAX_TEXT} of {total} characters"));
        }
    }
    Ok(serde_json::to_string_pretty(&out).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_calendar_files_and_identity_routes_are_open() {
        for ok in [
            "/me",
            "/me?$select=x",
            "/me/calendarView?startDateTime=a",
            "/me/events/AAMk",
            "/me/drive/recent",
            "/drives/b!x/items/01/children",
            "/search/query",
        ] {
            assert!(allowed_path(ok), "{ok}");
        }
        for no in [
            "/me/messages",
            "/me/mailFolders/inbox",
            "/me/sendMail",
            "/me/chats",
            "/teams/x/channels",
            "/me/events/a/attachments",
            "/me/events/../messages",
            "/me/events/%2e%2e",
            "/users/x",
            "/me/drive/../messages",
            "/me/onlineMeetings",
            "/me/contacts",
        ] {
            assert!(!allowed_path(no), "{no}");
        }
    }

    #[test]
    fn ids_and_windows_are_validated() {
        assert!(id_segment("b!abc-DEF_12").is_ok());
        assert!(
            id_segment("../x").is_err() && id_segment("a/b").is_err() && id_segment("").is_err()
        );
        let (s, e) = window(Some("2026-10-01T00:00:00Z"), Some("2026-10-02T00:00:00Z")).unwrap();
        assert_eq!(
            (s.as_str(), e.as_str()),
            ("2026-10-01T00:00:00Z", "2026-10-02T00:00:00Z")
        );
        assert!(window(Some("2026-10-02T00:00:00Z"), Some("2026-10-01T00:00:00Z")).is_err());
        assert!(
            window(Some("2026-01-01T00:00:00Z"), Some("2026-12-01T00:00:00Z")).is_err(),
            "too long"
        );
        assert!(window(Some("next tuesday"), None).is_err());
        assert!(window(None, None).is_ok());
    }

    #[test]
    fn utc_event_times_become_local() {
        let v = local_time(&json!({"dateTime": "2026-10-01T14:00:00.0000000", "timeZone": "UTC"}));
        let t = chrono::DateTime::parse_from_rfc3339(v.as_str().unwrap()).unwrap();
        assert_eq!(
            t.with_timezone(&chrono::Utc).to_rfc3339(),
            "2026-10-01T14:00:00+00:00"
        );
    }

    #[tokio::test]
    async fn lists_read_only_tools_and_refuses_unknown_ones() {
        let ctx = Ctx {
            http: reqwest::Client::new(),
            base: GRAPH.into(),
            tokens: tokio::sync::Mutex::new(None),
        };
        let list = handle(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
            &ctx,
        )
        .await
        .unwrap();
        let tools = list
            .pointer("/result/tools")
            .and_then(Value::as_array)
            .unwrap();
        assert_eq!(tools.len(), 7);
        assert!(tools
            .iter()
            .all(|t| t.pointer("/annotations/readOnlyHint") == Some(&json!(true))));
        assert!(!tools
            .iter()
            .any(|t| t["name"].as_str().unwrap().contains("mail")));
        let r = handle(&json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "send_mail"}}), &ctx).await.unwrap();
        assert_eq!(r.pointer("/result/isError"), Some(&json!(true)));
        assert!(handle(
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            &ctx
        )
        .await
        .is_none());
    }

    /// A Graph `calendarView` over `total` events, at most `per` a page,
    /// continued by `@odata.nextLink` (`$skiptoken`). `fail_at` makes that
    /// token's page a 503; `foreign` points the first next link off Graph.
    async fn graph_mock(
        total: usize,
        per: usize,
        fail_at: Option<usize>,
        foreign: bool,
    ) -> (Ctx, crate::test_http::MockServer) {
        let base = std::sync::Arc::new(std::sync::OnceLock::<String>::new());
        let b = base.clone();
        let server = crate::test_http::serve(move |req| {
            let q = |k: &str| req.path.split(&format!("{k}=")).nth(1).and_then(|v| v.split('&').next()).and_then(|v| v.parse::<usize>().ok());
            let start = q("$skiptoken").unwrap_or(0);
            if Some(start) == fail_at {
                return (503, json!({"error": {"message": "try later"}}).to_string());
            }
            let end = (start + per.min(q("$top").unwrap_or(10))).min(total);
            let value: Vec<Value> = (start..end).map(|i| json!({"id": format!("e{i}"), "subject": format!("Meeting {i}"),
                "start": {"dateTime": "2026-10-01T14:00:00.0000000", "timeZone": "UTC"}, "end": {"dateTime": "2026-10-01T15:00:00.0000000", "timeZone": "UTC"}})).collect();
            let mut out = json!({"value": value});
            if end < total {
                let root = if foreign { "https://graph.example.com/v1.0".to_string() } else { b.get().unwrap().clone() };
                out["@odata.nextLink"] = json!(format!("{root}/me/calendarView?startDateTime=a&$skiptoken={end}"));
            }
            (200, out.to_string())
        })
        .await;
        base.set(server.base.clone()).unwrap();
        let tokens = crate::m365_auth::Tokens {
            access_token: "t".into(),
            refresh_token: None,
            expires_at: u64::MAX / 2,
            scope: String::new(),
            account: String::new(),
            problem: None,
        };
        (
            Ctx {
                http: reqwest::Client::new(),
                base: server.base.clone(),
                tokens: tokio::sync::Mutex::new(Some(tokens)),
            },
            server,
        )
    }

    const FROM: &str = "2026-10-01T00:00:00Z";
    const TO: &str = "2026-10-08T00:00:00Z";

    #[tokio::test]
    async fn calendar_reads_follow_next_links() {
        let (ctx, server) = graph_mock(5, 2, None, false).await;
        let v = calendar(&ctx, FROM, TO, 200).await.unwrap();
        let ids: Vec<&str> = v["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["e0", "e1", "e2", "e3", "e4"], "{v}");
        assert_eq!(
            (v["complete"].clone(), v["partial"].clone()),
            (json!(true), Value::Null)
        );
        let seen = server.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 3);
        assert!(
            seen[1].path.starts_with("/me/calendarView?") && seen[1].path.contains("$skiptoken=2"),
            "{:?}",
            seen[1]
        );
        // The agent tool uses the same paging.
        let t: Value = serde_json::from_str(
            &call("list_events", &json!({"start": FROM, "end": TO}), &ctx)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            (t["events"].as_array().unwrap().len(), t["complete"].clone()),
            (5, json!(true))
        );
    }

    #[tokio::test]
    async fn a_cap_a_failing_page_or_a_foreign_link_is_partial() {
        let (ctx, _s) = graph_mock(5, 2, None, false).await;
        let v = calendar(&ctx, FROM, TO, 3).await.unwrap();
        assert_eq!(
            (v["events"].as_array().unwrap().len(), v["complete"].clone()),
            (3, json!(false))
        );
        assert!(
            v["partial"].as_str().unwrap().contains("stopped at 3"),
            "{v}"
        );

        let (ctx, _s) = graph_mock(5, 2, Some(2), false).await;
        let v = calendar(&ctx, FROM, TO, 200).await.unwrap();
        assert_eq!(
            (v["events"].as_array().unwrap().len(), v["complete"].clone()),
            (2, json!(false))
        );
        assert!(
            v["partial"].as_str().unwrap().contains("page 2 failed"),
            "{v}"
        );

        let (ctx, s) = graph_mock(5, 2, None, true).await;
        let v = calendar(&ctx, FROM, TO, 200).await.unwrap();
        assert_eq!(
            (v["events"].as_array().unwrap().len(), v["complete"].clone()),
            (2, json!(false))
        );
        assert!(
            v["partial"]
                .as_str()
                .unwrap()
                .contains("outside Microsoft Graph"),
            "{v}"
        );
        assert_eq!(
            s.seen.lock().unwrap().len(),
            1,
            "the bearer never went to the foreign host"
        );

        // A failing first page is still an error, not an empty calendar.
        let (ctx, _s) = graph_mock(5, 2, Some(0), false).await;
        assert!(calendar(&ctx, FROM, TO, 200)
            .await
            .unwrap_err()
            .contains("try later"));
    }
}
