//! The v1.2 Calendar route over real HTTP, against a temporary vault
//! (`docs/phone-api/calendar.md`). The read itself is the Agenda's
//! (`agenda::events_with`, tested there); these cover the token, the
//! work-data rule that keeps the calendar unread, the window and the
//! read-only method.
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn calendar_needs_a_token_and_has_no_write_methods() {
    let rig = rig().await;
    let none = rig.call("GET", "/v1/calendar", None, None).await;
    assert_eq!(none.status, 401);
    let token = rig.token();
    for method in ["POST", "PUT", "DELETE"] {
        let resp = rig
            .call(method, "/v1/calendar", Some(&token), Some(json!({})))
            .await;
        assert_eq!(resp.status, 404, "{method}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_data_off_reads_no_calendar_and_says_so() {
    let rig = rig().await;
    let token = rig.token();
    let scope = rig.store.dir().join("scope.json");
    // No scope file, an explicit off and an unreadable file all fail closed.
    for setup in [
        None,
        Some(r#"{"includeWorkData": false}"#),
        Some("not json"),
    ] {
        let _ = std::fs::remove_file(&scope);
        if let Some(text) = setup {
            std::fs::write(&scope, text).unwrap();
        }
        let resp = rig.call("GET", "/v1/calendar", Some(&token), None).await;
        assert_eq!(resp.status, 200, "{setup:?}");
        assert_eq!(resp.json()["enabled"], false, "{setup:?}");
        assert_eq!(resp.json()["events"], json!([]), "{setup:?}");
        assert!(resp.json()["from"].is_string() && resp.json()["to"].is_string());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bad_window_is_refused_before_anything_is_read() {
    let rig = rig().await;
    let token = rig.token();
    for query in [
        "from=2026-11-19T00:00:00Z",
        "from=nope&to=nope",
        "from=2026-11-20T00:00:00Z&to=2026-11-19T00:00:00Z",
        "from=2026-11-19T00:00:00Z&to=2027-01-19T00:00:00Z",
    ] {
        let resp = rig
            .call("GET", &format!("/v1/calendar?{query}"), Some(&token), None)
            .await;
        assert_eq!(resp.status, 400, "{query}");
    }
    let ok = rig
        .call(
            "GET",
            "/v1/calendar?from=2026-11-19T00:00:00Z&to=2026-11-20T00:00:00Z",
            Some(&token),
            None,
        )
        .await;
    assert_eq!(ok.status, 200);
    assert_eq!(ok.json()["from"], "2026-11-19T00:00:00Z");
}
