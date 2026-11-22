//! The v1.2 task routes over real HTTP, against a temporary vault
//! (`docs/phone-api/tasks.md`).
use super::*;
use crate::phone_access::tasks_api::{is_finished, split_due};

fn put(vault: &Path, relative: &str, text: &str) {
    let path = vault.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// `tasks/<name>.md` with `front` as its front matter (every line ends in a
/// newline) and `body` below it.
fn task(vault: &Path, name: &str, front: &str, body: &str) {
    put(
        vault,
        &format!("tasks/{name}.md"),
        &format!("---\n{front}---\n\n{body}\n"),
    );
}

fn file(vault: &Path, name: &str) -> String {
    std::fs::read_to_string(vault.join(format!("tasks/{name}.md"))).unwrap()
}

/// The task file's title line, replaced: an edit made on the Mac.
fn retitle(vault: &Path, name: &str, title: &str) {
    let path = vault.join(format!("tasks/{name}.md"));
    let text = std::fs::read_to_string(&path).unwrap();
    let edited: Vec<String> = text
        .lines()
        .map(|line| {
            if line.starts_with("title:") {
                format!("title: {title}")
            } else {
                line.to_string()
            }
        })
        .collect();
    std::fs::write(&path, edited.join("\n") + "\n").unwrap();
}

/// How many task files `tasks/` holds (none before it exists).
fn task_count(vault: &Path) -> usize {
    walkdir::WalkDir::new(vault.join("tasks"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .count()
}

/// The date `offset` days from the Mac's today.
fn day(offset: i64) -> String {
    (chrono::Local::now().date_naive() + chrono::Duration::days(offset)).to_string()
}

fn hash_of(vault: &Path, name: &str) -> String {
    crate::vault_tasks::get(vault, &format!("tasks/{name}.md"))
        .unwrap()
        .content_hash
}

fn the_ref(name: &str) -> String {
    format!("vault:tasks/{name}.md")
}

/// The names (`tasks/<name>.md` without the folder and extension) of a
/// list's rows, in order.
fn names(body: &Value) -> Vec<String> {
    body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            str_of(&t["ref"])
                .trim_start_matches("vault:tasks/")
                .trim_end_matches(".md")
                .to_string()
        })
        .collect()
}

fn row<'a>(body: &'a Value, name: &str) -> &'a Value {
    body["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["ref"] == the_ref(name))
        .unwrap_or_else(|| panic!("no {name} in {body}"))
}

async fn list(rig: &Rig, token: &str, query: &str) -> Resp {
    rig.call("GET", &format!("/v1/tasks{query}"), Some(token), None)
        .await
}

async fn post(rig: &Rig, token: &str, path: &str, body: Value) -> Resp {
    rig.call("POST", path, Some(token), Some(body)).await
}

fn refused(resp: &Resp, status: u16, code: &str, what: &str) {
    assert_eq!(
        resp.status,
        status,
        "{what}: {}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(resp.json()["error"]["code"], code, "{what}");
}

#[test]
fn a_finished_status_is_the_macs_done_or_cancelled_in_any_casing() {
    for status in [
        "Done",
        "done",
        "DELIVERED",
        " Complete ",
        "completed",
        "Cancelled",
        "canceled",
        "Won't do",
        "WONT DO",
        "Won\u{2019}t do",
    ] {
        assert!(is_finished(status), "{status}");
    }
    for status in [
        "Not started",
        "In progress",
        "Blocked",
        "",
        "Doing it",
        "Done?",
    ] {
        assert!(!is_finished(status), "{status}");
    }
}

#[test]
fn a_due_with_a_time_splits_into_a_date_and_a_clock() {
    let split = |due: Option<&str>, time: Option<&str>| split_due(due, time);
    let some = |a: &str, b: &str| (Some(a.to_string()), Some(b.to_string()));
    // Notion keeps the time in the date.
    assert_eq!(
        split(Some("2026-11-09T14:30"), None),
        some("2026-11-09", "14:30")
    );
    assert_eq!(
        split(Some("2026-11-09T14:30:00.000Z"), None),
        some("2026-11-09", "14:30")
    );
    // Hollow's own tasks keep it beside the date, and it wins over a time in the date.
    assert_eq!(
        split(Some("2026-11-09"), Some("09:15")),
        some("2026-11-09", "09:15")
    );
    assert_eq!(
        split(Some("2026-11-09T14:30"), Some("08:00")),
        some("2026-11-09", "08:00")
    );
    // No time, a time that is no clock, or a half-written one.
    let date_only = (Some("2026-11-09".to_string()), None);
    assert_eq!(split(Some("2026-11-09"), None), date_only);
    assert_eq!(split(Some("2026-11-09"), Some("25:00")), date_only);
    assert_eq!(split(Some("2026-11-09T14:3"), None), date_only);
    assert_eq!(split(Some("2026-11-09 14:30"), None), date_only);
    // No date means no time, and nothing here can panic on odd text.
    assert_eq!(split(None, Some("09:00")), (None, None));
    assert_eq!(split(Some("soon"), Some("09:00")), (None, None));
    assert_eq!(split(Some("2026-13-45"), None), (None, None));
    assert_eq!(split(Some("  "), None), (None, None));
    assert_eq!(
        split(Some("日本語日本語日本語日本語"), Some("09:00")),
        (None, None)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_task_routes_need_a_token_and_nothing_else_finds_a_route() {
    let rig = rig().await;
    task(
        &rig.vault,
        "quiet",
        "title: Quiet\nstatus: Not started\n",
        "",
    );
    let before = file(&rig.vault, "quiet");
    for (method, path) in [
        ("GET", "/v1/tasks"),
        ("GET", "/v1/tasks/statuses"),
        ("POST", "/v1/tasks"),
        ("POST", "/v1/tasks/status"),
        ("POST", "/v1/tasks/update"),
    ] {
        let body = (method == "POST").then(|| json!({}));
        let none = rig.call(method, path, None, body).await;
        refused(&none, 401, "unauthorized", &format!("{method} {path}"));
    }
    let token = rig.token();
    // There is no delete, no other verb, and no route that names a task in its path.
    for (method, path) in [
        ("DELETE", "/v1/tasks"),
        ("PUT", "/v1/tasks"),
        ("GET", "/v1/tasks/status"),
        ("GET", "/v1/tasks/update"),
        ("DELETE", "/v1/tasks/status"),
        ("POST", "/v1/tasks/delete"),
        ("DELETE", "/v1/tasks/quiet"),
        ("GET", "/v1/tasks/vault:tasks/quiet.md"),
    ] {
        let body = (method != "GET").then(|| json!({"ref": the_ref("quiet")}));
        let resp = rig.call(method, path, Some(&token), body).await;
        assert_eq!(resp.status, 404, "{method} {path}");
    }
    assert_eq!(file(&rig.vault, "quiet"), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_open_list_is_grouped_and_ordered_the_way_the_tasks_page_does() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let open = |title: &str, extra: &str| format!("title: {title}\nstatus: Not started\n{extra}");
    task(
        v,
        "late",
        &open("Late one", &format!("due: {}\npriority: P3\n", day(-1))),
        "",
    );
    task(
        v,
        "now-low",
        &open("Today low", &format!("due: {}\n", day(0))),
        "",
    );
    task(
        v,
        "now-high",
        &format!(
            "title: Today high\nstatus: In progress\ndue: {}\npriority: P1\n",
            day(0)
        ),
        "",
    );
    task(
        v,
        "now-timed",
        &open(
            "Today timed",
            &format!("due: {}\ndue_time: 09:00\npriority: P4\n", day(0)),
        ),
        "",
    );
    task(v, "soon", &open("Soon", &format!("due: {}\n", day(3))), "");
    task(
        v,
        "edge",
        &open("Edge of the week", &format!("due: {}\n", day(7))),
        "",
    );
    task(
        v,
        "far",
        &open("Far off", &format!("due: {}\n", day(30))),
        "",
    );
    task(v, "whenever", &open("Whenever", ""), "");
    task(v, "finished", "title: Finished\nstatus: Done\n", "");
    task(v, "dropped", "title: Dropped\nstatus: Cancelled\n", "");

    let resp = list(&rig, &token, "").await;
    assert_eq!(resp.status, 200);
    let body = resp.json();
    assert_eq!(body["today"], chrono::Local::now().date_naive().to_string());
    assert_eq!(
        names(&body),
        [
            "late",
            "now-timed",
            "now-high",
            "now-low",
            "soon",
            "edge",
            "far",
            "whenever"
        ]
    );
    assert_eq!(body["total"], 8);
    assert_eq!(body["hasMore"], false);
    for (name, bucket, days) in [
        ("late", "overdue", json!(-1)),
        ("now-timed", "today", json!(0)),
        ("now-high", "today", json!(0)),
        ("now-low", "today", json!(0)),
        ("soon", "thisweek", json!(3)),
        ("edge", "thisweek", json!(7)),
        ("far", "later", json!(30)),
        ("whenever", "undated", Value::Null),
    ] {
        let r = row(&body, name);
        assert_eq!(r["bucket"], bucket, "{name}");
        assert_eq!(r["daysUntilDue"], days, "{name}");
        assert_eq!(r["done"], false, "{name}");
    }
    assert_eq!(row(&body, "now-timed")["dueTime"], "09:00");
    assert_eq!(row(&body, "now-high")["status"], "In progress");
    assert_eq!(row(&body, "now-high")["priority"], "P1");
    assert_eq!(row(&body, "whenever")["priority"], Value::Null);
    assert_eq!(row(&body, "whenever")["due"], Value::Null);

    // `limit` cuts the list and says there is more.
    let few = list(&rig, &token, "?limit=3").await.json();
    assert_eq!(names(&few), ["late", "now-timed", "now-high"]);
    assert_eq!(few["total"], 8);
    assert_eq!(few["hasMore"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_and_dropped_tasks_list_newest_change_first_and_stay_out_of_the_open_list() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "open-one", "title: Open one\nstatus: Not started\n", "");
    for (name, status, hours) in [
        ("done-old", "Done", 5u64),
        ("dropped-mid", "Cancelled", 3),
        ("done-new", "Complete", 1),
        ("delivered", "delivered", 2),
    ] {
        task(v, name, &format!("title: {name}\nstatus: {status}\n"), "");
        let at = std::time::SystemTime::now() - Duration::from_secs(hours * 3600);
        std::fs::File::options()
            .write(true)
            .open(v.join(format!("tasks/{name}.md")))
            .unwrap()
            .set_modified(at)
            .unwrap();
    }
    let done = list(&rig, &token, "?state=done").await.json();
    assert_eq!(
        names(&done),
        ["done-new", "delivered", "dropped-mid", "done-old"]
    );
    assert_eq!(done["total"], 4);
    assert_eq!(done["hasMore"], false);
    for r in done["tasks"].as_array().unwrap() {
        assert_eq!(r["done"], true, "{r}");
        assert_eq!(r["bucket"], "done", "{r}");
    }
    assert_eq!(row(&done, "dropped-mid")["status"], "Cancelled");

    let open = list(&rig, &token, "").await.json();
    assert_eq!(names(&open), ["open-one"]);
    assert_eq!(open["total"], 1);
    assert_eq!(list(&rig, &token, "?state=open").await.json(), open);

    let all = list(&rig, &token, "?state=all").await.json();
    assert_eq!(
        names(&all),
        [
            "open-one",
            "done-new",
            "delivered",
            "dropped-mid",
            "done-old"
        ]
    );
    assert_eq!(all["total"], 5);
    let two = list(&rig, &token, "?state=all&limit=2").await.json();
    assert_eq!(names(&two), ["open-one", "done-new"]);
    assert_eq!(two["total"], 5);
    assert_eq!(two["hasMore"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_finished_list_sends_thirty_by_default_and_never_more_than_a_hundred() {
    let rig = rig().await;
    let token = rig.token();
    for i in 0..105 {
        task(
            &rig.vault,
            &format!("old-{i:03}"),
            &format!("title: Old {i:03}\nstatus: Done\n"),
            "",
        );
    }
    let default = list(&rig, &token, "?state=done").await.json();
    assert_eq!(default["tasks"].as_array().unwrap().len(), 30);
    assert_eq!(default["total"], 105);
    assert_eq!(default["hasMore"], true);
    let most = list(&rig, &token, "?state=done&limit=100").await.json();
    assert_eq!(most["tasks"].as_array().unwrap().len(), 100);
    refused(
        &list(&rig, &token, "?state=done&limit=101").await,
        400,
        "bad_request",
        "101 finished rows",
    );
    // The open list takes up to 500; the finished tasks are not in it.
    let open = list(&rig, &token, "?limit=500").await;
    assert_eq!(open.status, 200);
    assert_eq!(open.json()["tasks"].as_array().unwrap().len(), 0);
    refused(
        &list(&rig, &token, "?limit=501").await,
        400,
        "bad_request",
        "501 rows",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_list_with_a_bad_state_or_limit_is_a_400() {
    let rig = rig().await;
    let token = rig.token();
    for query in [
        "?state=weird",
        "?state=",
        "?state=OPEN",
        "?limit=0",
        "?limit=-1",
        "?limit=abc",
        "?limit=",
        "?limit=1.5",
        "?state=all&limit=0",
    ] {
        refused(&list(&rig, &token, query).await, 400, "bad_request", query);
    }
    // Unknown parameters are ignored, as on the other routes.
    assert_eq!(list(&rig, &token, "?cursor=x").await.status, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tracker_fields_use_state_kind_and_the_status_route_keeps_a_local_fallback() {
    let rig = rig().await;
    let token = rig.token();
    task(&rig.vault,"tracker-row","title: A tracker fixture\nstatus: Shipped\nstatus_type: completed\npriority: High\ntracker: linear\ntracker_id: fixture-issue\ntracker_key: ENG-99\ntracker_url: https://example.invalid/ENG-99\n",
        "<!-- tracker:description -->\nProvider description\n<!-- /tracker:description -->\n<!-- tracker:comments -->\n- [ ] Provider checklist\n<!-- /tracker:comments -->\n");
    let done = list(&rig, &token, "?state=done").await.json();
    let tracked = row(&done, "tracker-row");
    assert_eq!(tracked["statusType"], "completed");
    assert_eq!(tracked["trackerLinked"], true);
    assert_eq!(tracked["notionLinked"], true);
    assert_eq!(tracked["trackerKey"], "ENG-99");
    assert_eq!(tracked["priority"], "P2");
    assert_eq!(tracked["subtasksTotal"], 0);
    assert_eq!(tracked["description"], "Provider description");
    let response = rig
        .call("GET", "/v1/tasks/statuses", Some(&token), None)
        .await;
    assert_eq!(response.status, 200);
    let statuses = response.json();
    assert_eq!(statuses["tracker"], false);
    assert_eq!(statuses["statuses"].as_array().unwrap().len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_row_carries_what_the_phone_shows_and_matches_the_fixtures() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "send-the-report",
        &format!(
            "title: Send the report\nstatus: Not started\npriority: P2\ndue: {}\ndue_time: 14:30\nrecurrence: every week\nlabels: work, finance\n",
            day(2)
        ),
        "The quarterly numbers for Jordan.\n\n- [x] Draft\n- [ ] Send\n",
    );
    task(
        v,
        "call-the-vendor",
        "title: Call the vendor\nstatus: In progress\n",
        "",
    );
    task(
        v,
        "waiting",
        "title: Waiting on the report\nstatus: Not started\nblocked_by: vault:tasks/send-the-report.md\n",
        "",
    );
    task(
        v,
        "book-the-flights",
        &format!(
            "title: Book the flights\nstatus: Done\npriority: P1\ndue: {}\n",
            day(-2)
        ),
        "",
    );
    let open = list(&rig, &token, "").await.json();
    let fixture_open = fixture("tasks/list-open.json");
    same_shape(&open, &fixture_open, "list-open");
    same_shape(
        row(&open, "call-the-vendor"),
        &fixture_open["tasks"][1],
        "bare row",
    );
    let r = row(&open, "send-the-report");
    assert_eq!(r["title"], "Send the report");
    assert_eq!(r["priority"], "P2");
    assert_eq!(r["due"], day(2));
    assert_eq!(r["dueTime"], "14:30");
    assert_eq!(r["bucket"], "thisweek");
    assert_eq!(r["daysUntilDue"], 2);
    assert_eq!(r["labels"], json!(["work", "finance"]));
    assert_eq!(r["recurrence"], "every week");
    assert!(str_of(&r["description"]).starts_with("The quarterly numbers for Jordan."));
    assert_eq!(r["descriptionTruncated"], false);
    assert_eq!(
        r["subtasks"],
        json!([{"text": "Draft", "done": true}, {"text": "Send", "done": false}])
    );
    assert_eq!(r["subtasksTotal"], 2);
    assert_eq!(r["blocked"], false);
    assert_eq!(str_of(&r["contentHash"]).len(), 64);
    assert_eq!(r["contentHash"], hash_of(v, "send-the-report"));
    assert_eq!(row(&open, "waiting")["blocked"], true);
    let bare = row(&open, "call-the-vendor");
    assert_eq!(bare["description"], Value::Null);
    assert_eq!(bare["labels"], json!([]));
    assert_eq!(bare["subtasks"], json!([]));

    let done = list(&rig, &token, "?state=done").await.json();
    same_shape(&done, &fixture("tasks/list-done.json"), "list-done");
    let flights = row(&done, "book-the-flights");
    assert_eq!(flights["done"], true);
    assert_eq!(flights["bucket"], "done");
    assert_eq!(flights["daysUntilDue"], -2);
    assert_eq!(flights["status"], "Done");
    assert_eq!(flights["priority"], "P1");

    // A long description and a long checklist are cut, and say so.
    let checklist: String = (0..60).map(|i| format!("- [ ] item {i}\n")).collect();
    task(
        v,
        "long",
        "title: Long one\nstatus: Not started\n",
        &format!("{}\n\n{checklist}", "word ".repeat(400)),
    );
    let open = list(&rig, &token, "").await.json();
    let long = row(&open, "long");
    assert_eq!(long["descriptionTruncated"], true);
    assert_eq!(str_of(&long["description"]).chars().count(), 1_000);
    assert_eq!(long["subtasks"].as_array().unwrap().len(), 50);
    assert_eq!(long["subtasksTotal"], 60);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_real_visible_task_files_are_listed() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "real", "title: Real\nstatus: Not started\n", "");
    task(
        v,
        "nested/deeper",
        "title: Nested\nstatus: Not started\n",
        "",
    );
    task(v, ".hidden", "title: Hidden\nstatus: Not started\n", "");
    task(
        v,
        ".archive/old",
        "title: Archived\nstatus: Not started\n",
        "",
    );
    put(
        v,
        "tasks/notes.txt",
        "---\ntitle: Not markdown\nstatus: Not started\n---\n",
    );
    put(
        v,
        "me/secret.md",
        "---\ntitle: Secret\nstatus: Not started\n---\n\nPrivate.\n",
    );
    std::os::unix::fs::symlink(v.join("me/secret.md"), v.join("tasks/linked.md")).unwrap();
    let body = list(&rig, &token, "?state=all").await.json();
    assert_eq!(names(&body), ["nested/deeper", "real"]);
    assert_eq!(body["total"], 2);
    let text = body.to_string();
    assert!(!text.contains("Private."), "{text}");
    assert!(!text.contains("Secret"), "{text}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_made_on_the_phone_is_one_file_however_often_the_key_is_sent() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let request = fixture("tasks/create-request.json");
    let first = post(&rig, &token, "/v1/tasks", request.clone()).await;
    assert_eq!(
        first.status,
        200,
        "{}",
        String::from_utf8_lossy(&first.body)
    );
    let made = first.json();
    same_shape(&made, &fixture("tasks/created.json"), "created");
    assert_eq!(made["disposition"], "applied");
    let t = &made["task"];
    let reference = str_of(&t["ref"]);
    assert!(
        reference.starts_with("vault:tasks/") && reference.ends_with(".md"),
        "{reference}"
    );
    let name = reference
        .trim_start_matches("vault:tasks/")
        .trim_end_matches(".md")
        .to_string();
    for (field, expected) in [
        ("title", json!("Send the report")),
        ("status", json!("Not started")),
        ("due", json!("2026-11-09")),
        ("dueTime", json!("14:30")),
        ("priority", json!("P2")),
        ("recurrence", json!("every week")),
        ("labels", json!(["work"])),
    ] {
        assert_eq!(t[field], expected, "{field}");
    }
    assert!(str_of(&t["description"]).starts_with("The quarterly numbers for Jordan."));
    assert_eq!(task_count(v), 1);
    // It is the iPhone's task, and the vault is free again.
    let stored = crate::vault_tasks::get(v, &format!("tasks/{name}.md")).unwrap();
    assert!(
        stored
            .origin
            .as_deref()
            .is_some_and(|o| o.starts_with("companion:ph-")),
        "{:?}",
        stored.origin
    );
    assert_eq!(t["contentHash"], stored.content_hash);
    assert!(crate::automated_history::lease_holder(v).is_none());

    // The same key again (a retry after a lost answer) makes no second file.
    let again = post(&rig, &token, "/v1/tasks", request.clone())
        .await
        .json();
    assert_eq!(again["disposition"], "duplicate");
    assert_eq!(again["task"]["ref"], t["ref"]);
    assert_eq!(task_count(v), 1);
    // The answer is the task as it is now, even after it was renamed on the Mac.
    retitle(v, &name, "Renamed on the Mac");
    let renamed = post(&rig, &token, "/v1/tasks", request.clone())
        .await
        .json();
    assert_eq!(renamed["disposition"], "duplicate");
    assert_eq!(renamed["task"]["title"], "Renamed on the Mac");
    assert_eq!(task_count(v), 1);
    // The same key with another title is still a retry (the contract says so):
    // the first task as it is now, and no second file.
    let mut retitled = request.clone();
    retitled["title"] = json!("A different title");
    let same_key = post(&rig, &token, "/v1/tasks", retitled).await.json();
    assert_eq!(same_key["disposition"], "duplicate");
    assert_eq!(same_key["task"]["title"], "Renamed on the Mac");
    assert_eq!(task_count(v), 1);
    // Another iPhone with the same key makes a task of its own.
    let other = rig.token();
    let second = post(&rig, &other, "/v1/tasks", request).await.json();
    assert_eq!(second["disposition"], "applied");
    assert_ne!(second["task"]["ref"], t["ref"]);
    assert_eq!(task_count(v), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_create_that_is_not_valid_is_a_400_and_writes_nothing() {
    let rig = rig().await;
    let token = rig.token();
    let long_title = "t".repeat(241);
    let long_description = "d".repeat(20_001);
    let many_labels: Vec<String> = (0..21).map(|i| format!("label-{i}")).collect();
    for (what, body) in [
        ("no key", json!({"title": "X"})),
        ("short key", json!({"idempotencyKey": "abc", "title": "X"})),
        (
            "a slash in the key",
            json!({"idempotencyKey": "abcdefgh/ijk", "title": "X"}),
        ),
        (
            "a space in the key",
            json!({"idempotencyKey": "abcdefgh ijk", "title": "X"}),
        ),
        (
            "long key",
            json!({"idempotencyKey": "k".repeat(65), "title": "X"}),
        ),
        (
            "key that is a number",
            json!({"idempotencyKey": 12345678, "title": "X"}),
        ),
        ("no title", json!({"idempotencyKey": "key-00000001"})),
        (
            "blank title",
            json!({"idempotencyKey": "key-00000002", "title": "   "}),
        ),
        (
            "two-line title",
            json!({"idempotencyKey": "key-00000003", "title": "one\ntwo"}),
        ),
        (
            "long title",
            json!({"idempotencyKey": "key-00000004", "title": long_title}),
        ),
        (
            "bad date",
            json!({"idempotencyKey": "key-00000005", "title": "X", "due": "next friday"}),
        ),
        (
            "impossible date",
            json!({"idempotencyKey": "key-00000006", "title": "X", "due": "2026-02-30"}),
        ),
        (
            "bad time",
            json!({"idempotencyKey": "key-00000007", "title": "X", "dueTime": "25:00"}),
        ),
        (
            "a date that is not written YYYY-MM-DD",
            json!({"idempotencyKey": "key-00000011", "title": "X", "due": "2026-1-5"}),
        ),
        (
            "a due that carries a time",
            json!({"idempotencyKey": "key-00000012", "title": "X", "due": "2026-11-09T14:30"}),
        ),
        (
            "a ten-character due with a leading space and no zero",
            json!({"idempotencyKey": "key-00000019", "title": "X", "due": " 2026-1-05"}),
        ),
        (
            "a due with a trailing space",
            json!({"idempotencyKey": "key-0000001a", "title": "X", "due": "2026-11-09 "}),
        ),
        (
            "a title that ends in a line break",
            json!({"idempotencyKey": "key-0000001b", "title": "X\n"}),
        ),
        (
            "a time with one-digit hours",
            json!({"idempotencyKey": "key-00000013", "title": "X", "dueTime": "9:30"}),
        ),
        (
            "a bell in the title",
            json!({"idempotencyKey": "key-00000014", "title": "a\u{7}b"}),
        ),
        (
            "a line separator in the title",
            json!({"idempotencyKey": "key-00000015", "title": "a\u{2028}b"}),
        ),
        (
            "a C1 control in a label",
            json!({"idempotencyKey": "key-00000016", "title": "X", "labels": ["a\u{85}b"]}),
        ),
        (
            "a control character in the recurrence",
            json!({"idempotencyKey": "key-00000017", "title": "X", "recurrence": "every\u{7} week"}),
        ),
        (
            "a recurrence over sixty characters",
            json!({"idempotencyKey": "key-00000018", "title": "X", "recurrence": format!("every week {}", "x".repeat(60))}),
        ),
        (
            "bad priority",
            json!({"idempotencyKey": "key-00000008", "title": "X", "priority": "P9"}),
        ),
        (
            "bad recurrence",
            json!({"idempotencyKey": "key-00000009", "title": "X", "recurrence": "now and then"}),
        ),
        (
            "long description",
            json!({"idempotencyKey": "key-0000000a", "title": "X", "description": long_description}),
        ),
        (
            "labels as text",
            json!({"idempotencyKey": "key-0000000b", "title": "X", "labels": "work"}),
        ),
        (
            "too many labels",
            json!({"idempotencyKey": "key-0000000c", "title": "X", "labels": many_labels}),
        ),
        (
            "a label with a comma",
            json!({"idempotencyKey": "key-0000000d", "title": "X", "labels": ["a,b"]}),
        ),
        (
            "a status",
            json!({"idempotencyKey": "key-0000000e", "title": "X", "status": "Done"}),
        ),
        (
            "a project",
            json!({"idempotencyKey": "key-0000000f", "title": "X", "projectUrl": "vault:projects/a.md"}),
        ),
        (
            "a blocker",
            json!({"idempotencyKey": "key-00000010", "title": "X", "blockedBy": ["vault:tasks/a.md"]}),
        ),
        ("not an object", json!(["idempotencyKey", "title"])),
    ] {
        let resp = post(&rig, &token, "/v1/tasks", body).await;
        refused(&resp, 400, "bad_request", what);
    }
    let resp = rig
        .call_raw("POST", "/v1/tasks", Some(&token), b"this is not json")
        .await;
    refused(&resp, 400, "bad_request", "not JSON");
    assert_eq!(task_count(&rig.vault), 0, "a refused create wrote a file");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn completing_and_reopening_a_task_changes_only_its_status() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "plain",
        "title: Plain\nstatus: Not started\npriority: P2\nlabels: home\n",
        "Body stays.\n\n- [ ] one\n",
    );
    let hash = hash_of(v, "plain");
    let resp = post(
        &rig,
        &token,
        "/v1/tasks/status",
        json!({"ref": the_ref("plain"), "status": "Done", "expectedHash": hash}),
    )
    .await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let done = resp.json();
    assert_eq!(done["disposition"], "applied");
    assert_eq!(done["rolledForwardTo"], Value::Null);
    assert_eq!(done["task"]["status"], "Done");
    assert_eq!(done["task"]["done"], true);
    assert_eq!(done["task"]["bucket"], "done");
    let new_hash = str_of(&done["task"]["contentHash"]);
    assert_ne!(new_hash, hash);
    assert_eq!(new_hash, hash_of(v, "plain"));
    let text = file(v, "plain");
    assert!(
        text.contains("status: Done") && !text.contains("status: Not started"),
        "{text}"
    );
    for kept in ["title: Plain", "priority: P2", "Body stays.", "- [ ] one"] {
        assert!(text.contains(kept), "{kept} in {text}");
    }
    assert!(crate::automated_history::lease_holder(v).is_none());

    // Undo: the old status sent back with the answer's hash.
    let reopened = post(
        &rig,
        &token,
        "/v1/tasks/status",
        json!({"ref": the_ref("plain"), "status": "Not started", "expectedHash": new_hash}),
    )
    .await
    .json();
    assert_eq!(reopened["task"]["status"], "Not started");
    assert_eq!(reopened["task"]["done"], false);
    assert_eq!(reopened["task"]["bucket"], "undated");
    assert_eq!(reopened["rolledForwardTo"], Value::Null);
    assert!(file(v, "plain").contains("status: Not started"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finishing_a_repeating_task_moves_its_date_and_says_so() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "weekly",
        &format!(
            "title: Weekly report\nstatus: In progress\ndue: {}\ndue_time: 10:00\nrecurrence: every week\n",
            day(-1)
        ),
        "",
    );
    let hash = hash_of(v, "weekly");
    let request = json!({"ref": the_ref("weekly"), "status": "Done", "expectedHash": hash});
    let resp = post(&rig, &token, "/v1/tasks/status", request.clone()).await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let body = resp.json();
    same_shape(
        &body,
        &fixture("tasks/status-changed.json"),
        "status-changed",
    );
    assert_eq!(body["rolledForwardTo"], day(7));
    assert_eq!(body["task"]["status"], "Not started");
    assert_eq!(body["task"]["done"], false);
    assert_eq!(body["task"]["due"], day(7));
    assert_eq!(body["task"]["dueTime"], "10:00");
    assert_eq!(body["task"]["bucket"], "thisweek");
    let after = file(v, "weekly");
    // A retry of the same call is refused, so a repeat can't roll it forward twice.
    let retry = post(&rig, &token, "/v1/tasks/status", request).await;
    refused(&retry, 409, "task_changed", "a retry of a rolled task");
    assert_eq!(file(v, "weekly"), after);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stale_hash_is_a_409_and_the_file_is_untouched() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "stale", "title: Stale\nstatus: Not started\n", "Text.\n");
    let before = file(v, "stale");
    let wrong = "0".repeat(64);
    for (path, body) in [
        (
            "/v1/tasks/status",
            json!({"ref": the_ref("stale"), "status": "Done", "expectedHash": wrong}),
        ),
        (
            "/v1/tasks/update",
            json!({"ref": the_ref("stale"), "expectedHash": wrong, "fields": {"title": "New"}}),
        ),
    ] {
        let resp = post(&rig, &token, path, body).await;
        assert_eq!(resp.status, 409, "{path}");
        assert_eq!(
            resp.json(),
            fixture("tasks/error-task-changed.json"),
            "{path}"
        );
    }
    assert_eq!(file(v, "stale"), before);

    // A change made on the Mac after the list was read makes that list's hash stale.
    let read = hash_of(v, "stale");
    retitle(v, "stale", "Edited on the Mac");
    let resp = post(
        &rig,
        &token,
        "/v1/tasks/status",
        json!({"ref": the_ref("stale"), "status": "Done", "expectedHash": read}),
    )
    .await;
    refused(&resp, 409, "task_changed", "a task edited after the list");
    assert!(file(v, "stale").contains("title: Edited on the Mac"));
    assert!(file(v, "stale").contains("status: Not started"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_changes_the_title_date_time_and_priority_and_null_clears() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "move-me",
        &format!(
            "title: Move me\nstatus: Not started\npriority: P3\ndue: {}\ndue_time: 09:30\n",
            day(1)
        ),
        "Keep this text.\n",
    );
    let hash = hash_of(v, "move-me");
    let resp = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("move-me"),
            "expectedHash": hash,
            "fields": {"title": "Moved", "due": day(5), "priority": "P1"}
        }),
    )
    .await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let body = resp.json();
    same_shape(&body, &fixture("tasks/updated.json"), "updated");
    assert_eq!(body["disposition"], "applied");
    let t = &body["task"];
    assert_eq!(t["title"], "Moved");
    assert_eq!(t["due"], day(5));
    assert_eq!(t["priority"], "P1");
    assert_eq!(t["dueTime"], "09:30");
    assert_eq!(t["bucket"], "thisweek");
    assert_eq!(t["daysUntilDue"], 5);
    assert_eq!(t["status"], "Not started");
    assert!(file(v, "move-me").contains("Keep this text."));
    assert_eq!(t["contentHash"], hash_of(v, "move-me"));

    let time = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("move-me"),
            "expectedHash": t["contentHash"],
            "fields": {"dueTime": "16:45"}
        }),
    )
    .await
    .json();
    assert_eq!(time["task"]["dueTime"], "16:45");
    assert_eq!(time["task"]["due"], day(5));

    // Null and an empty string clear a date, a time and a priority.
    let cleared = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("move-me"),
            "expectedHash": time["task"]["contentHash"],
            "fields": {"due": null, "dueTime": "", "priority": null}
        }),
    )
    .await;
    assert_eq!(
        cleared.status,
        200,
        "{}",
        String::from_utf8_lossy(&cleared.body)
    );
    let cleared = cleared.json();
    let t = &cleared["task"];
    assert_eq!(t["due"], Value::Null);
    assert_eq!(t["dueTime"], Value::Null);
    assert_eq!(t["priority"], Value::Null);
    assert_eq!(t["bucket"], "undated");
    assert_eq!(t["daysUntilDue"], Value::Null);
    assert_eq!(t["title"], "Moved");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_update_or_status_call_that_is_not_valid_is_a_400_and_writes_nothing() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "target",
        "title: Target\nstatus: Not started\n",
        "Text.\n",
    );
    let before = file(v, "target");
    let hash = hash_of(v, "target");
    let r = the_ref("target");
    let update = |fields: Value| json!({"ref": r, "expectedHash": hash, "fields": fields});
    for (what, body) in [
        ("a status field", update(json!({"status": "Done"}))),
        ("a description field", update(json!({"description": "x"}))),
        ("a labels field", update(json!({"labels": ["a"]}))),
        (
            "a project field",
            update(json!({"projectUrl": "vault:projects/a.md"})),
        ),
        (
            "a field and an unknown one",
            update(json!({"title": "Ok", "nope": 1})),
        ),
        ("no fields", update(json!({}))),
        ("fields that are text", update(json!("title"))),
        ("a null title", update(json!({"title": null}))),
        ("a number title", update(json!({"title": 5}))),
        ("a blank title", update(json!({"title": "  "}))),
        ("a two-line title", update(json!({"title": "a\nb"}))),
        ("a bad date", update(json!({"due": "friday"}))),
        (
            "a date that is not written YYYY-MM-DD",
            update(json!({"due": "2026-1-5"})),
        ),
        (
            "a due that carries a time",
            update(json!({"due": "2026-11-09T14:30"})),
        ),
        (
            "a ten-character due with a leading space and no zero",
            update(json!({"due": " 2026-1-05"})),
        ),
        (
            "a time with one-digit hours",
            update(json!({"dueTime": "9:30"})),
        ),
        ("a bell in the title", update(json!({"title": "a\u{7}b"}))),
        ("a bad priority", update(json!({"priority": "P7"}))),
        ("a bad time", update(json!({"dueTime": "9am"}))),
        ("a number date", update(json!({"due": 20261109}))),
        ("no fields key", json!({"ref": r, "expectedHash": hash})),
        ("no hash", json!({"ref": r, "fields": {"title": "X"}})),
        (
            "a short hash",
            json!({"ref": r, "expectedHash": "abc", "fields": {"title": "X"}}),
        ),
        (
            "a non-hex hash",
            json!({"ref": r, "expectedHash": "z".repeat(64), "fields": {"title": "X"}}),
        ),
        (
            "no ref",
            json!({"expectedHash": hash, "fields": {"title": "X"}}),
        ),
        (
            "a blank ref",
            json!({"ref": "  ", "expectedHash": hash, "fields": {"title": "X"}}),
        ),
        (
            "an unknown key",
            json!({"ref": r, "expectedHash": hash, "fields": {"title": "X"}, "force": true}),
        ),
    ] {
        let resp = post(&rig, &token, "/v1/tasks/update", body).await;
        refused(&resp, 400, "bad_request", what);
    }
    let status = |extra: Value| {
        let mut body = json!({"ref": r, "status": "Done", "expectedHash": hash});
        for (key, value) in extra.as_object().unwrap() {
            if value.is_null() {
                body.as_object_mut().unwrap().remove(key);
            } else {
                body[key.as_str()] = value.clone();
            }
        }
        body
    };
    for (what, body) in [
        ("no ref", status(json!({"ref": null}))),
        ("a blank ref", status(json!({"ref": ""}))),
        ("no hash", status(json!({"expectedHash": null}))),
        ("a short hash", status(json!({"expectedHash": "abc"}))),
        (
            "a non-hex hash",
            status(json!({"expectedHash": "z".repeat(64)})),
        ),
        ("no status", status(json!({"status": null}))),
        ("a blank status", status(json!({"status": "  "}))),
        ("a long status", status(json!({"status": "s".repeat(61)}))),
        (
            "a two-line status",
            status(json!({"status": "Done\nand more"})),
        ),
        (
            "a bell in the status",
            status(json!({"status": "Do\u{7}ne"})),
        ),
        (
            "a status that ends in a line separator",
            status(json!({"status": "Done\u{2028}"})),
        ),
        (
            "a ref that ends in a line break",
            status(json!({"ref": format!("{r}\n")})),
        ),
        ("an unknown key", status(json!({"force": true}))),
    ] {
        let resp = post(&rig, &token, "/v1/tasks/status", body).await;
        refused(&resp, 400, "bad_request", what);
    }
    let resp = rig
        .call_raw("POST", "/v1/tasks/status", Some(&token), b"[]")
        .await;
    refused(&resp, 400, "bad_request", "not an object");
    assert_eq!(file(v, "target"), before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refs_name_only_real_visible_task_files_under_tasks() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "real", "title: Real\nstatus: Not started\n", "");
    task(v, ".hidden", "title: Hidden\nstatus: Not started\n", "");
    let secret = "---\ntitle: Secret\nstatus: Not started\n---\n\nPrivate.\n";
    put(v, "me/secret.md", secret);
    std::os::unix::fs::symlink(v.join("me/secret.md"), v.join("tasks/linked.md")).unwrap();
    // A task file inside a linked folder resolves outside `tasks/`.
    put(v, "me/dir/inner.md", secret);
    std::os::unix::fs::symlink(v.join("me/dir"), v.join("tasks/dirlink")).unwrap();
    task(
        v,
        ".archive/old",
        "title: Archived\nstatus: Not started\n",
        "",
    );
    let long_ref = format!("vault:tasks/{}.md", "a".repeat(400));
    let hash = "a".repeat(64);
    let sent = |path: &str, reference: &str| {
        let body = if path.ends_with("status") {
            json!({"ref": reference, "status": "Done", "expectedHash": hash})
        } else {
            json!({"ref": reference, "expectedHash": hash, "fields": {"title": "Taken"}})
        };
        (path.to_string(), body)
    };
    for path in ["/v1/tasks/status", "/v1/tasks/update"] {
        // Malformed or outside tasks/ (400), whatever the route.
        for reference in [
            "../me/secret.md",
            "vault:me/secret.md",
            "me/secret.md",
            "vault:tasks/../me/secret.md",
            "tasks/.hidden.md",
            "vault:tasks/linked.md",
            "vault:tasks/",
            "vault:tasks/real.txt",
            "tasks\\real.md",
            "/tasks/real.md",
            "vault:tasks/.archive/old.md",
            "vault:tasks/dirlink/inner.md",
            "vault:tasks/real.md\u{7}",
            long_ref.as_str(),
        ] {
            let (p, body) = sent(path, reference);
            refused(
                &post(&rig, &token, &p, body).await,
                400,
                "bad_request",
                &format!("{path} {reference}"),
            );
        }
        // A task that is not there (404), so the app lists again.
        for reference in [
            "vault:tasks/missing.md",
            "tasks/missing.md",
            "vault:tasks/nested/missing.md",
        ] {
            let (p, body) = sent(path, reference);
            refused(
                &post(&rig, &token, &p, body).await,
                404,
                "not_found",
                &format!("{path} {reference}"),
            );
        }
    }
    for outside in ["me/secret.md", "me/dir/inner.md"] {
        assert_eq!(
            std::fs::read_to_string(v.join(outside)).unwrap(),
            secret,
            "{outside}"
        );
    }
    assert!(file(v, "real").contains("title: Real"));
    assert!(file(v, ".hidden").contains("title: Hidden"));
    assert!(file(v, ".archive/old").contains("title: Archived"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_busy_vault_is_a_409_busy_and_the_same_body_works_when_it_is_free() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "waits", "title: Waits\nstatus: Not started\n", "");
    put(v, "notes/seed.md", "seed");
    let status = json!({
        "ref": the_ref("waits"),
        "status": "Done",
        "expectedHash": hash_of(v, "waits")
    });
    let create = json!({"idempotencyKey": "retry-key-0001", "title": "Later"});
    // An automated run holds the vault: nothing is written, and the answer says to retry.
    let run = crate::automated_history::try_begin_run(v, "workflow", Some("held")).unwrap();
    refused(
        &post(&rig, &token, "/v1/tasks/status", status.clone()).await,
        409,
        "busy",
        "a status call while a run holds the vault",
    );
    refused(
        &post(&rig, &token, "/v1/tasks", create.clone()).await,
        409,
        "busy",
        "a create while a run holds the vault",
    );
    assert!(file(v, "waits").contains("status: Not started"));
    assert_eq!(task_count(v), 1);
    let _ = crate::automated_history::finish_run(v, run, "success");
    // Free again: the same bodies go through.
    assert_eq!(
        post(&rig, &token, "/v1/tasks/status", status).await.status,
        200
    );
    assert_eq!(post(&rig, &token, "/v1/tasks", create).await.status, 200);
    assert_eq!(task_count(v), 2);
}

/// A Notion-style due that carries its time lives in `due`, so clearing the
/// time has to trim the due to its date, or the time would come back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_a_time_that_lives_inside_due_trims_the_due_to_its_date() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "inline",
        "title: Inline\nstatus: Not started\ndue: 2026-11-09T14:30\n",
        "",
    );
    let before = list(&rig, &token, "").await.json();
    assert_eq!(row(&before, "inline")["due"], "2026-11-09");
    assert_eq!(row(&before, "inline")["dueTime"], "14:30");
    let resp = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("inline"),
            "expectedHash": hash_of(v, "inline"),
            "fields": {"dueTime": null}
        }),
    )
    .await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    let body = resp.json();
    assert_eq!(body["task"]["due"], "2026-11-09");
    assert_eq!(body["task"]["dueTime"], Value::Null);
    let stored = crate::vault_tasks::get(v, "tasks/inline.md").unwrap();
    assert_eq!(stored.due.as_deref(), Some("2026-11-09"));
    assert_eq!(stored.due_time, None);
    let after = list(&rig, &token, "").await.json();
    assert_eq!(row(&after, "inline")["dueTime"], Value::Null);
    assert_eq!(row(&after, "inline")["due"], "2026-11-09");
}

/// Clearing the date of a task that has its own `due_time` clears the time
/// too: a row that looks cleared must not show the old time with a later date.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clearing_the_date_clears_a_stored_time_too() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "timed",
        &format!(
            "title: Timed\nstatus: Not started\ndue: {}\ndue_time: 09:15\n",
            day(3)
        ),
        "",
    );
    let cleared = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("timed"),
            "expectedHash": hash_of(v, "timed"),
            "fields": {"due": null}
        }),
    )
    .await;
    assert_eq!(
        cleared.status,
        200,
        "{}",
        String::from_utf8_lossy(&cleared.body)
    );
    let cleared = cleared.json();
    assert_eq!(cleared["task"]["due"], Value::Null);
    assert_eq!(cleared["task"]["dueTime"], Value::Null);
    let stored = crate::vault_tasks::get(v, "tasks/timed.md").unwrap();
    assert_eq!(stored.due_time, None, "the stored time stayed behind");
    // A date set later brings no old time back.
    let dated = post(
        &rig,
        &token,
        "/v1/tasks/update",
        json!({
            "ref": the_ref("timed"),
            "expectedHash": cleared["task"]["contentHash"],
            "fields": {"due": day(6)}
        }),
    )
    .await
    .json();
    assert_eq!(dated["task"]["due"], day(6));
    assert_eq!(dated["task"]["dueTime"], Value::Null);
}

/// A runaway app stops at the daily count, but a retry of a key that already
/// made its task still gets its answer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_iphone_may_make_only_so_many_tasks_a_day_but_a_retry_still_answers() {
    let rig = rig().await;
    let (device, token) = rig
        .store
        .pair("Counted iPhone", None, chrono::Utc::now())
        .unwrap();
    let v = &rig.vault;
    let first = json!({"idempotencyKey": "count-key-0001", "title": "Made before the limit"});
    assert_eq!(
        post(&rig, &token, "/v1/tasks", first.clone()).await.status,
        200
    );
    assert_eq!(task_count(v), 1);
    // The count is in memory: seed what a day of making tasks would leave
    // (one of the 200 slots is taken already, by the first task).
    for _ in 0..199 {
        crate::phone_access::tasks_api::reserve_create(
            &device.device_id,
            std::time::Instant::now(),
            usize::MAX,
        )
        .unwrap();
    }
    let next = json!({"idempotencyKey": "count-key-0002", "title": "One too many"});
    let resp = post(&rig, &token, "/v1/tasks", next.clone()).await;
    refused(&resp, 400, "bad_request", "past the daily count");
    assert!(
        String::from_utf8_lossy(&resp.body).contains("200 tasks"),
        "{}",
        String::from_utf8_lossy(&resp.body)
    );
    assert_eq!(task_count(v), 1, "a refused create wrote a file");
    // A retry of the key that did land is no new task: it answers.
    let retry = post(&rig, &token, "/v1/tasks", first).await;
    assert_eq!(
        retry.status,
        200,
        "{}",
        String::from_utf8_lossy(&retry.body)
    );
    assert_eq!(retry.json()["disposition"], "duplicate");
    // Another iPhone has its own count.
    let other = rig.token();
    assert_eq!(post(&rig, &other, "/v1/tasks", next).await.status, 200);
    assert_eq!(task_count(v), 2);
}

/// A write that landed is never a 500: when the list does not hold the task
/// (a hidden file it skips), the one file is read.
#[test]
fn the_answer_after_a_write_falls_back_to_the_file_when_the_list_skips_it() {
    let dir = tempfile::tempdir().unwrap();
    let v = dir.path();
    task(v, ".quiet", "title: Quiet\nstatus: Not started\n", "");
    assert!(crate::vault_tasks::list(v)
        .iter()
        .all(|t| t.url != "vault:tasks/.quiet.md"));
    let row = crate::phone_access::tasks_api::task_after(
        v,
        &json!({"ref": "vault:tasks/.quiet.md", "disposition": "applied"}),
    )
    .unwrap();
    assert_eq!(row.title, "Quiet");
    assert!(!row.blocked);
    // A ref that names nothing at all is still the one error.
    let missing =
        crate::phone_access::tasks_api::task_after(v, &json!({"ref": "vault:tasks/nothing.md"}));
    assert!(missing.is_err());
}

/// A link at `tasks/` would list, and take writes for, another part of the
/// vault: the phone lists nothing and refuses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_tasks_folder_lists_nothing_and_takes_no_writes() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    // `tasks/` is a link to another folder of the vault, which holds task-shaped files.
    let secret = "---\ntitle: Elsewhere\nstatus: Not started\n---\n\nPrivate.\n";
    put(v, "me/area/one.md", secret);
    std::os::unix::fs::symlink(v.join("me/area"), v.join("tasks")).unwrap();
    let body = list(&rig, &token, "?state=all").await.json();
    assert_eq!(body["tasks"], json!([]));
    assert_eq!(body["total"], 0);
    assert!(!body.to_string().contains("Elsewhere"));
    let hash = "a".repeat(64);
    for (path, request) in [
        (
            "/v1/tasks",
            json!({"idempotencyKey": "link-key-0001", "title": "Nope"}),
        ),
        (
            "/v1/tasks/status",
            json!({"ref": "vault:tasks/one.md", "status": "Done", "expectedHash": hash}),
        ),
        (
            "/v1/tasks/update",
            json!({"ref": "vault:tasks/one.md", "expectedHash": hash, "fields": {"title": "Nope"}}),
        ),
    ] {
        refused(
            &post(&rig, &token, path, request).await,
            400,
            "bad_request",
            path,
        );
    }
    assert_eq!(
        std::fs::read_to_string(v.join("me/area/one.md")).unwrap(),
        secret
    );
    assert_eq!(
        std::fs::read_dir(v.join("me/area")).unwrap().count(),
        1,
        "a create wrote through the link"
    );
}

/// The contract's order: `P1` first, `P4` before no priority.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_priority_sorts_after_p4_and_a_backslash_name_is_not_listed() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    let due = day(2);
    for (name, front) in [
        ("apple", "title: Apple\nstatus: Not started\n".to_string()),
        (
            "zebra",
            "title: Zebra\nstatus: Not started\npriority: P4\n".to_string(),
        ),
        (
            "mango",
            "title: Mango\nstatus: Not started\npriority: P1\n".to_string(),
        ),
    ] {
        task(v, name, &format!("{front}due: {due}\n"), "");
    }
    task(v, "back\\slash", "title: Back\nstatus: Not started\n", "");
    let body = list(&rig, &token, "").await.json();
    assert_eq!(names(&body), ["mango", "zebra", "apple"]);
}

/// The request fixtures are real requests: sent with the task's real hash,
/// the routes accept them and answer in the fixtures' shapes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_status_and_update_request_fixtures_are_accepted_with_a_real_hash() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(
        v,
        "send-the-report",
        &format!(
            "title: Send the report\nstatus: Not started\ndue: {}\n",
            day(2)
        ),
        "",
    );
    let mut update = fixture("tasks/update-request.json");
    update["expectedHash"] = json!(hash_of(v, "send-the-report"));
    let updated = post(&rig, &token, "/v1/tasks/update", update).await;
    assert_eq!(
        updated.status,
        200,
        "{}",
        String::from_utf8_lossy(&updated.body)
    );
    let updated = updated.json();
    same_shape(&updated, &fixture("tasks/updated.json"), "updated");
    assert_eq!(updated["task"]["due"], "2026-11-10");
    assert_eq!(updated["task"]["priority"], "P1");
    let mut status = fixture("tasks/status-request.json");
    status["expectedHash"] = updated["task"]["contentHash"].clone();
    let done = post(&rig, &token, "/v1/tasks/status", status).await;
    assert_eq!(done.status, 200, "{}", String::from_utf8_lossy(&done.body));
    let done = done.json();
    same_shape(
        &done,
        &fixture("tasks/status-changed.json"),
        "status-changed",
    );
    assert_eq!(done["task"]["done"], true);
}

/// A date and its time go together on an update, a task imported from Notion
/// included (Notion two-way sync was removed on 2026-11-10).
#[test]
fn a_time_and_a_date_are_tied_on_update() {
    use crate::phone_access::tasks_api::tie_time_to_date;
    let parse = |front: &str| {
        crate::vault_tasks::parse(
            "tasks/t.md",
            &format!("---\ntitle: T\nstatus: Not started\n{front}---\n"),
            Path::new("/unused"),
        )
        .unwrap()
    };
    let clearing = |fields: Value| fields.as_object().unwrap().clone();
    let inline = parse("due: 2026-11-09T14:30\n");
    let timed = parse("due: 2026-11-09\ndue_time: 09:15\n");
    let plain = parse("due: 2026-11-09\n");
    let imported = parse(
        "due: 2026-11-09T14:30\ndue_time: 09:00\nnotion_id: bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\n",
    );

    // Clearing the time of a due that carries one trims the due to its date.
    let mut changes = clearing(json!({"dueTime": ""}));
    tie_time_to_date(&inline, &mut changes);
    assert_eq!(changes["due"], "2026-11-09");
    // Clearing the date clears a stored time.
    let mut changes = clearing(json!({"due": ""}));
    tie_time_to_date(&timed, &mut changes);
    assert_eq!(changes["dueTime"], "");
    // Nothing is added when there is nothing to tie.
    for (task, fields) in [
        (&plain, json!({"dueTime": ""})),
        (&plain, json!({"due": ""})),
        (&inline, json!({"due": "2026-11-12", "dueTime": ""})),
        (&timed, json!({"due": "", "dueTime": ""})),
        (&timed, json!({"title": "New"})),
    ] {
        let mut changes = clearing(fields.clone());
        tie_time_to_date(task, &mut changes);
        assert_eq!(changes, clearing(fields.clone()), "{fields}");
    }
    // An imported task is tied the same way.
    let mut changes = clearing(json!({"dueTime": ""}));
    tie_time_to_date(&imported, &mut changes);
    assert_eq!(changes["due"], "2026-11-09");
    let mut changes = clearing(json!({"due": ""}));
    tie_time_to_date(&imported, &mut changes);
    assert_eq!(changes["dueTime"], "");
}

/// A task file whose name holds a line separator is listed by the Mac but
/// could not be named by a write, so the phone does not offer it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_task_file_with_a_control_character_in_its_name_is_not_listed() {
    let rig = rig().await;
    let token = rig.token();
    let v = &rig.vault;
    task(v, "plain", "title: Plain\nstatus: Not started\n", "");
    task(v, "a\u{2028}b", "title: Split\nstatus: Not started\n", "");
    task(v, "bell\u{7}", "title: Bell\nstatus: Not started\n", "");
    let body = list(&rig, &token, "").await.json();
    assert_eq!(names(&body), ["plain"]);
    assert_eq!(body["total"], 1);
}

/// R2 (ENG-117): the day's count keeps no entry for a phone that has made
/// nothing in the last day: a refused or repeated write gives its slot back
/// and the entry goes with it, a lapsed one is swept by the next reservation
/// from any phone, and a revoked phone's is dropped.
#[test]
fn the_day_count_keeps_no_empty_entry_per_device() {
    use crate::phone_access::tasks_api::{created_has, forget_device, reserve_create};
    let now = std::time::Instant::now();
    let (a, b, c) = ("ph-r2-count-a", "ph-r2-count-b", "ph-r2-count-c");
    // A reservation handed back leaves nothing behind.
    let slot = reserve_create(a, now, 5).unwrap();
    assert!(created_has(a));
    crate::phone_access::tasks_api::release_create_for_test(a, slot);
    assert!(!created_has(a), "an empty list was kept");
    // A cap of none makes no entry either.
    assert!(reserve_create(b, now, 0).is_none());
    assert!(!created_has(b));
    reserve_create(c, now, 5).unwrap();
    assert!(created_has(c));
    let later = now;
    // A revoked phone's counts (tasks and notes) are dropped.
    reserve_create(&crate::phone_access::notes_api::count_key(c), later, 5).unwrap();
    forget_device(c);
    assert!(!created_has(c));
    assert!(!created_has(&crate::phone_access::notes_api::count_key(c)));
}

/// The sweep behind [`reserve_create`], on a map of its own (the real one is
/// shared by every test, and a time a day ahead would sweep theirs): a list
/// older than a day goes, and a device left with none is dropped.
#[test]
fn the_sweep_drops_lapsed_lists_and_keeps_live_ones() {
    use crate::phone_access::tasks_api::prune_created;
    use std::collections::{HashMap, VecDeque};
    let now = std::time::Instant::now();
    let day = std::time::Duration::from_secs(24 * 60 * 60);
    let mut all: HashMap<String, VecDeque<std::time::Instant>> = HashMap::new();
    all.insert("lapsed".into(), VecDeque::from([now]));
    all.insert("mixed".into(), VecDeque::from([now, now + day]));
    all.insert("empty".into(), VecDeque::new());
    prune_created(&mut all, now + day);
    assert_eq!(all.keys().collect::<Vec<_>>(), ["mixed"]);
    assert_eq!(all["mixed"].len(), 1);
}

/// H4b: a labeled task's row carries `sourceLabel` (the desktop's words);
/// an unlabeled one leaves the key out, so older phones see no change.
#[test]
fn a_labeled_task_row_carries_its_source_label() {
    use crate::phone_access::tasks_api::{list_body, Which};
    let dir = tempfile::tempdir().unwrap();
    task(
        dir.path(),
        "local",
        "title: Local\nstatus: Not started\n",
        "",
    );
    let today = chrono::Local::now().date_naive();
    let plain =
        serde_json::to_value(list_body(dir.path(), Which::Open, 10, today, |_| {})).unwrap();
    assert!(plain["tasks"][0].get("sourceLabel").is_none());
    let labeled = list_body(dir.path(), Which::Open, 10, today, |tasks| {
        for t in tasks {
            t.source_label = Some(crate::sources::SourceLabel {
                kind: crate::sources::LabelKind::Stored,
                text: "Stored in Hollow".into(),
                provider_name: None,
                url: None,
                open_label: None,
                since: None,
                was_tracked: None,
                was_tracked_url: None,
            });
        }
    });
    let row = &serde_json::to_value(labeled).unwrap()["tasks"][0];
    assert_eq!(row["sourceLabel"]["kind"], "stored");
    assert_eq!(row["sourceLabel"]["text"], "Stored in Hollow");
}
