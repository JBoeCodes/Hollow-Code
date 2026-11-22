//! A canned GraphQL server for tracker tests (tests only).
//!
//! Unlike `test_http`, it records every request's headers (so a test can see
//! the `Authorization` header the adapter sent) and replies with headers of
//! its own (so a test can serve `X-RateLimit-*` readings and 429s). Replies
//! come from a handler; [`unscoped`] is the scope check the adapter tests use
//! to make the stub refuse any issue or comment query that doesn't name the
//! team, as the design asks.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One request: method, path, lower-cased headers, and the GraphQL body.
#[derive(Clone, Debug)]
pub struct StubRequest {
    pub method: String,
    pub path: String,
    pub headers: BTreeMap<String, String>,
    pub body: serde_json::Value,
}

impl StubRequest {
    pub fn query(&self) -> &str {
        self.body
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
    }

    pub fn variables(&self) -> &serde_json::Value {
        self.body
            .get("variables")
            .unwrap_or(&serde_json::Value::Null)
    }

    /// The operation name, from the body or the query text.
    pub fn operation(&self) -> Option<String> {
        if let Some(name) = self.body.get("operationName").and_then(|n| n.as_str()) {
            return Some(name.to_string());
        }
        let query = self.query().trim_start();
        let rest = query
            .strip_prefix("query")
            .or_else(|| query.strip_prefix("mutation"))?
            .trim_start();
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    }
}

/// A reply: status, extra headers, JSON body.
#[derive(Clone, Debug)]
pub struct StubReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl StubReply {
    /// `{"data": data}` with 200.
    pub fn data(data: serde_json::Value) -> StubReply {
        StubReply {
            status: 200,
            headers: Vec::new(),
            body: serde_json::json!({ "data": data }).to_string(),
        }
    }

    /// A GraphQL error with `extensions.code` (Linear's `RATELIMITED`,
    /// `AUTHENTICATION_ERROR`, `FORBIDDEN`…), 200 as Linear sends most.
    pub fn error(code: &str, message: &str) -> StubReply {
        StubReply {
            status: 200,
            headers: Vec::new(),
            body: serde_json::json!({
                "errors": [{ "message": message, "extensions": { "code": code } }]
            })
            .to_string(),
        }
    }

    /// A bare HTTP status (429, 500…) with an error body.
    pub fn status(status: u16) -> StubReply {
        StubReply {
            status,
            headers: Vec::new(),
            body: serde_json::json!({ "errors": [{ "message": format!("HTTP {status}") }] })
                .to_string(),
        }
    }

    pub fn with_header(mut self, name: &str, value: &str) -> StubReply {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Linear's rate-limit headers.
    pub fn with_rate(self, requests_left: u64, requests_limit: u64, reset_ms: u64) -> StubReply {
        self.with_header("X-RateLimit-Requests-Remaining", &requests_left.to_string())
            .with_header("X-RateLimit-Requests-Limit", &requests_limit.to_string())
            .with_header("X-RateLimit-Requests-Reset", &reset_ms.to_string())
    }
}

pub struct StubServer {
    /// `http://127.0.0.1:<port>/graphql`.
    pub url: String,
    pub seen: Arc<Mutex<Vec<StubRequest>>>,
}

impl StubServer {
    pub fn requests(&self) -> Vec<StubRequest> {
        self.seen.lock().unwrap().clone()
    }
}

/// Serve `reply` on a fresh port until the runtime ends.
pub async fn serve<F>(reply: F) -> StubServer
where
    F: Fn(&StubRequest) -> StubReply + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/graphql", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 8192];
            let (header_end, length) = loop {
                let n = stream.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break (0, 0);
                }
                raw.extend_from_slice(&chunk[..n]);
                if let Some(i) = raw.windows(4).position(|b| b == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..i]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    break (i + 4, length);
                }
            };
            if header_end == 0 {
                continue;
            }
            while raw.len() < header_end + length {
                let n = stream.read(&mut chunk).await.unwrap_or(0);
                if n == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..n]);
            }
            let head = String::from_utf8_lossy(&raw[..header_end - 4]).into_owned();
            let mut lines = head.lines();
            let mut first = lines.next().unwrap_or("").split_whitespace();
            let method = first.next().unwrap_or("").to_string();
            let path = first.next().unwrap_or("").to_string();
            let headers = lines
                .filter_map(|line| line.split_once(':'))
                .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
                .collect();
            let body_bytes = &raw[header_end..(header_end + length).min(raw.len())];
            let body = serde_json::from_slice(body_bytes).unwrap_or(serde_json::Value::Null);
            let req = StubRequest {
                method,
                path,
                headers,
                body,
            };
            log.lock().unwrap().push(req.clone());
            let out = reply(&req);
            let mut response = format!(
                "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
                out.status,
                out.body.len()
            );
            for (name, value) in &out.headers {
                response.push_str(&format!("{name}: {value}\r\n"));
            }
            response.push_str("\r\n");
            response.push_str(&out.body);
            let _ = stream.write_all(response.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    StubServer { url, seen }
}

/// The stub's scope check: an `issues` or `comments` query must name `team_id`
/// in its variables or its text. Returns the refusal to send when it doesn't.
pub fn unscoped(req: &StubRequest, team_id: &str) -> Option<StubReply> {
    // A create's read-back by Hollow's own id (Q-R22) names no team on
    // purpose and selects only the id and the team id.
    if req.operation().as_deref() == Some("IssueLanding") {
        return None;
    }
    let query = req.query();
    let reads_team_data = ["issues(", "comments(", "issueSearch(", "searchIssues("]
        .iter()
        .any(|field| query.contains(field));
    if !reads_team_data {
        return None;
    }
    let named = query.contains(team_id) || req.variables().to_string().contains(team_id);
    (!named).then(|| StubReply::error("FORBIDDEN", "stub: query without the team filter"))
}

/// An issue node in Linear's GraphQL shape, for fixtures.
pub fn issue_node(id: &str, key: &str, team_id: &str, updated_at: &str) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "identifier": key,
        "url": format!("https://linear.app/example/issue/{key}"),
        "title": format!("Fixture {key}"),
        "description": "",
        "priority": 0,
        "dueDate": null,
        "estimate": null,
        "createdAt": "2026-11-07T21:27:00.000Z",
        "updatedAt": updated_at,
        "archivedAt": null,
        "trashed": false,
        "team": { "id": team_id },
        "state": { "id": "state-todo", "name": "Todo", "type": "unstarted" },
        "assignee": null,
        "creator": null,
        "project": null,
        "parent": null,
        "cycle": null,
        "projectMilestone": null,
        "labels": { "nodes": [] },
        "children": { "nodes": [] },
        "relations": { "nodes": [] },
        "inverseRelations": { "nodes": [] }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAM: &str = "team-ai";

    #[tokio::test]
    async fn the_stub_records_headers_replies_with_rate_headers_and_refuses_unscoped_reads() {
        let server = serve(|req| {
            if let Some(refused) = unscoped(req, TEAM) {
                return refused;
            }
            match req.operation().as_deref() {
                Some("Issues") => StubReply::data(serde_json::json!({
                    "issues": { "nodes": [issue_node("i1", "ENG-1", TEAM, "2026-11-08T05:00:00.000Z")] }
                }))
                .with_rate(2400, 2500, 1_762_578_000_000),
                Some("Busy") => StubReply::status(429),
                _ => StubReply::error("GRAPHQL_VALIDATION_FAILED", "unknown operation"),
            }
        })
        .await;
        let client = reqwest::Client::new();
        let post = |body: serde_json::Value| {
            client
                .post(&server.url)
                .header("Authorization", "lin_api_test")
                .json(&body)
                .send()
        };

        let scoped = post(serde_json::json!({
            "query": "query Issues($team: ID!) { issues(filter: {team: {id: {eq: $team}}}) { nodes { id } } }",
            "variables": { "team": TEAM }
        }))
        .await
        .unwrap();
        assert_eq!(scoped.status(), 200);
        assert_eq!(
            scoped.headers()["x-ratelimit-requests-remaining"]
                .to_str()
                .unwrap(),
            "2400"
        );
        let body: serde_json::Value = scoped.json().await.unwrap();
        assert_eq!(body["data"]["issues"]["nodes"][0]["team"]["id"], TEAM);

        let unscoped_reply: serde_json::Value = post(serde_json::json!({
            "query": "query Issues { issues(first: 50) { nodes { id } } }"
        }))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
        assert_eq!(
            unscoped_reply["errors"][0]["extensions"]["code"],
            "FORBIDDEN"
        );

        let busy = post(serde_json::json!({ "query": "query Busy { viewer { id } }" }))
            .await
            .unwrap();
        assert_eq!(busy.status(), 429);

        let seen = server.requests();
        assert_eq!(seen.len(), 3);
        assert!(seen
            .iter()
            .all(|r| r.method == "POST" && r.path == "/graphql"));
        assert!(seen
            .iter()
            .all(|r| r.headers["authorization"] == "lin_api_test"));
        assert_eq!(seen[0].operation().as_deref(), Some("Issues"));
        assert_eq!(seen[0].variables()["team"], TEAM);
    }
}
