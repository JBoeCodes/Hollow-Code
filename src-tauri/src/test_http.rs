//! A tiny canned HTTP server for connector tests (merged plan slice 5): it
//! answers each request from a handler and records what was asked, so a test
//! can serve paged API replies without the network.
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// One request the server saw: method, path with query, body. The method and
/// body are recorded for a test to check; no current test reads them since
/// the Notion connector's tests left (2026-11-10).
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub body: String,
}

pub struct MockServer {
    /// `http://127.0.0.1:<port>`, no trailing slash.
    pub base: String,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

/// Serve `reply(&seen) -> (status, json body)` on a fresh port until the
/// runtime ends.
pub async fn serve<F>(reply: F) -> MockServer
where
    F: Fn(&Seen) -> (u16, String) + Send + Sync + 'static,
{
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
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
            let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
            let mut first = head.lines().next().unwrap_or("").split_whitespace();
            let req = Seen {
                method: first.next().unwrap_or("").to_string(),
                path: first.next().unwrap_or("").to_string(),
                body: String::from_utf8_lossy(
                    &raw[header_end..(header_end + length).min(raw.len())],
                )
                .into_owned(),
            };
            log.lock().unwrap().push(req.clone());
            let (status, body) = reply(&req);
            let out = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(out.as_bytes()).await;
            let _ = stream.shutdown().await;
        }
    });
    MockServer { base, seen }
}
