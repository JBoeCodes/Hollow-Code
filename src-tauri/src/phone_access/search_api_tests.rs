//! The v1.2 search route over real HTTP, against a temporary vault
//! (`docs/phone-api/search.md`).
use super::*;

fn get<'a>(
    rig: &'a Rig,
    token: &'a str,
    path: &'a str,
) -> impl std::future::Future<Output = Resp> + 'a {
    rig.call("GET", path, Some(token), None)
}

/// Every file under `dir` with its size and modified time, for a before and
/// after comparison.
fn tree(dir: &Path) -> Vec<(String, u64, std::time::SystemTime)> {
    let mut rows: Vec<_> = walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| {
            let meta = e.metadata().unwrap();
            (
                e.path().to_string_lossy().into_owned(),
                meta.len(),
                meta.modified().unwrap(),
            )
        })
        .collect();
    rows.sort();
    rows
}

fn strings(body: &Value, field: &str, key: &str) -> Vec<String> {
    body[field]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            let row = if field == "chats" {
                &item["session"]
            } else {
                item
            };
            row[key].as_str().unwrap().to_string()
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn search_needs_a_token_and_never_writes() {
    let rig = rig().await;
    let w = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    seed_chat(
        &rig.vault,
        "s-20261107-5ea00001",
        "claude",
        &w.cwd,
        &[("Plan the garage lease", "Ok")],
        "acceptEdits",
    );
    let none = rig.call("GET", "/v1/search?q=garage", None, None).await;
    assert_eq!(none.status, 401);
    let token = rig.token();
    let before = tree(&rig.vault.join("workspaces"));
    let found = get(&rig, &token, "/v1/search?q=garage").await;
    assert_eq!(found.status, 200);
    assert_eq!(
        tree(&rig.vault.join("workspaces")),
        before,
        "a search changed a file"
    );
    // Nothing writes: other methods find no route.
    for method in ["POST", "PUT", "DELETE"] {
        let resp = rig
            .call(
                method,
                "/v1/search?q=garage",
                Some(&token),
                Some(json!({"q": "x"})),
            )
            .await;
        assert_eq!(resp.status, 404, "{method}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_query_is_required_and_bounded() {
    let rig = rig().await;
    let token = rig.token();
    let too_long = format!("/v1/search?q={}", "x".repeat(201));
    let longest = format!("/v1/search?q={}", "x".repeat(200));
    for path in [
        "/v1/search",
        "/v1/search?q=",
        "/v1/search?q=%20%20+",
        too_long.as_str(),
        "/v1/search?q=a&limit=0",
        "/v1/search?q=a&limit=51",
        "/v1/search?q=a&limit=many",
        "/v1/search?q=a&cursor=not-a-cursor",
    ] {
        let resp = get(&rig, &token, path).await;
        assert_eq!(resp.status, 400, "{path}");
        assert_eq!(resp.json()["error"]["code"], "bad_request", "{path}");
    }
    assert_eq!(get(&rig, &token, &longest).await.status, 200);
    assert_eq!(
        get(&rig, &token, "/v1/search?q=a&limit=50").await.status,
        200
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn workspaces_match_by_title_status_outcome_and_next_step_and_title_hits_come_first() {
    let rig = rig().await;
    let token = rig.token();
    let lease =
        crate::workspaces::create(&rig.vault, "Garage lease", "Renew the east lease.").unwrap();
    let notes =
        crate::workspaces::create(&rig.vault, "Meeting notes", "Notes on the garage.").unwrap();
    let old = crate::workspaces::create(&rig.vault, "Old project", "Goal").unwrap();
    crate::workspaces::update_meta(&rig.vault, &old.id, None, Some("archived")).unwrap();

    // The title first, then the Outcome; case is ignored.
    let body = get(&rig, &token, "/v1/search?q=GARAGE").await.json();
    assert_eq!(
        strings(&body, "workspaces", "id"),
        [lease.id.as_str(), notes.id.as_str()]
    );
    assert_eq!(body["workspacesTotal"], 2);
    assert_eq!(body["query"], "GARAGE");
    assert_eq!(body["chats"], json!([]));
    assert_eq!(body["nextCursor"], Value::Null);

    // Each word may sit in a different field: "east" in the Outcome, "garage" in the title.
    let both = get(&rig, &token, "/v1/search?q=east+garage").await.json();
    assert_eq!(strings(&both, "workspaces", "id"), [lease.id.as_str()]);
    // The status counts, and archived workspaces are found.
    let status = get(&rig, &token, "/v1/search?q=archived").await.json();
    assert_eq!(strings(&status, "workspaces", "id"), [old.id.as_str()]);
    assert_eq!(status["workspaces"][0]["status"], "archived");
    // The next step counts: a new brief's reads "Start with the request and
    // attached sources.", and no title or Outcome here holds those words.
    let step = get(&rig, &token, "/v1/search?q=attached+sources")
        .await
        .json();
    assert_eq!(step["workspacesTotal"], 3);
    // Nothing matches a word no field holds; the chat folders and the main
    // agent are never listed.
    let none = get(&rig, &token, "/v1/search?q=zebra").await.json();
    assert_eq!(none["workspaces"], json!([]));
    assert_eq!(none["workspacesTotal"], 0);
    // The built-ins exist now, and "a" is in their titles: still not listed.
    crate::workspaces::ensure_quick_chats(&rig.vault).unwrap();
    crate::workspaces::ensure_main_agent(&rig.vault).unwrap();
    let any = get(&rig, &token, "/v1/search?q=a").await.json();
    let ids = strings(&any, "workspaces", "id");
    assert!(
        ids.contains(&lease.id),
        "the test's own workspaces are found: {ids:?}"
    );
    assert!(
        !ids.contains(&"quick-chats".to_string()) && !ids.contains(&"main".to_string()),
        "{ids:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn at_most_ten_workspaces_ride_the_first_page_and_the_total_counts_them_all() {
    let rig = rig().await;
    let token = rig.token();
    for n in 0..12 {
        crate::workspaces::create(&rig.vault, &format!("Bulk project {n}"), "Goal").unwrap();
    }
    let body = get(&rig, &token, "/v1/search?q=bulk").await.json();
    assert_eq!(body["workspaces"].as_array().unwrap().len(), 10);
    assert_eq!(body["workspacesTotal"], 12);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chats_match_by_title_or_by_the_persons_messages_with_a_snippet() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let (by_message, by_title, by_answer, archived, outside) = (
        "s-20261107-5ea00011",
        "s-20261107-5ea00012",
        "s-20261107-5ea00013",
        "s-20261107-5ea00014",
        "s-20261107-5ea00015",
    );
    seed_chat(
        &rig.vault,
        by_message,
        "claude",
        &w.cwd,
        &[
            ("Weekly sync", "Hi"),
            ("Send the garage lease to Dana", "Sent"),
        ],
        "acceptEdits",
    );
    crate::session_store::rename_session(&rig.vault, by_message, "Weekly sync").unwrap();
    seed_chat(
        &rig.vault,
        by_title,
        "codex",
        &w.cwd,
        &[("Hello", "Hi")],
        "acceptEdits",
    );
    crate::session_store::rename_session(&rig.vault, by_title, "Garage notes").unwrap();
    // The phone shares the scrubbed index, including replies.
    seed_chat(
        &rig.vault,
        by_answer,
        "claude",
        &w.cwd,
        &[("Hello", "The garage is booked")],
        "acceptEdits",
    );
    crate::session_store::rename_session(&rig.vault, by_answer, "Greeting").unwrap();
    seed_chat(
        &rig.vault,
        archived,
        "claude",
        &w.cwd,
        &[("garage", "Ok")],
        "acceptEdits",
    );
    crate::session_store::archive_session(&rig.vault, archived, true).unwrap();
    // A chat outside every workspace is left out (the chat list has it).
    seed_chat(
        &rig.vault,
        outside,
        "claude",
        rig.vault.to_str().unwrap(),
        &[("garage", "Ok")],
        "acceptEdits",
    );

    let body = get(&rig, &token, "/v1/search?q=garage").await.json();
    let keys = strings(&body, "chats", "key");
    assert!(keys.contains(&by_message.to_string()), "{keys:?}");
    assert!(keys.contains(&by_title.to_string()), "{keys:?}");
    assert!(keys.contains(&by_answer.to_string()), "{keys:?}");
    assert!(!keys.contains(&archived.to_string()), "{keys:?}");
    assert!(!keys.contains(&outside.to_string()), "{keys:?}");
    let hit = |key: &str| {
        body["chats"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["session"]["key"] == key)
            .unwrap()
            .clone()
    };
    let message = hit(by_message);
    assert_eq!(message["workspaceId"], w.id.as_str());
    assert!(
        message["snippet"]
            .as_str()
            .unwrap()
            .contains("garage lease"),
        "{message}"
    );
    // The title holds the word: no snippet.
    assert_eq!(hit(by_title)["snippet"], Value::Null);
    // The row is the v1 Session, with its workspace.
    assert_eq!(message["session"]["workspace"]["id"], w.id.as_str());
    assert_eq!(message["session"]["title"], "Weekly sync");
    same_shape(
        &message["session"],
        &fixture("search/results.json")["chats"][0]["session"],
        "search chat session",
    );
    assert_eq!(body["workspaces"][0]["id"], w.id.as_str());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chats_page_by_cursor_and_later_pages_leave_the_workspaces_out() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    for n in 1..=3 {
        let key = format!("s-20261107-5ea0002{n}");
        seed_chat(
            &rig.vault,
            &key,
            "claude",
            &w.cwd,
            &[("Plan the garage lease", "Ok")],
            "acceptEdits",
        );
    }
    let first = get(&rig, &token, "/v1/search?q=garage&limit=2")
        .await
        .json();
    let first_keys = strings(&first, "chats", "key");
    assert_eq!(first_keys.len(), 2);
    assert_eq!(first["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(first["workspacesTotal"], 1);
    let cursor = first["nextCursor"].as_str().unwrap().to_string();
    same_shape(&first, &fixture("search/results.json"), "search first page");

    let path = format!(
        "/v1/search?q=garage&limit=2&cursor={}",
        urlencoding::encode(&cursor)
    );
    let second = get(&rig, &token, &path).await.json();
    let second_keys = strings(&second, "chats", "key");
    assert_eq!(second_keys.len(), 1);
    assert!(!first_keys.contains(&second_keys[0]));
    assert_eq!(second["workspaces"], json!([]));
    assert_eq!(second["workspacesTotal"], 0);
    assert_eq!(second["nextCursor"], Value::Null);
    same_shape(
        &second,
        &fixture("search/results-more.json"),
        "search second page",
    );

    // The default page is 20: all three fit.
    let all = get(&rig, &token, "/v1/search?q=garage").await.json();
    assert_eq!(all["chats"].as_array().unwrap().len(), 3);
    assert_eq!(all["nextCursor"], Value::Null);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_query_is_percent_decoded_and_its_whitespace_folded() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    seed_chat(
        &rig.vault,
        "s-20261107-5ea00031",
        "claude",
        &w.cwd,
        &[("Plan the garage lease renewal", "Ok")],
        "acceptEdits",
    );
    for path in [
        "/v1/search?q=garage+lease",
        "/v1/search?q=garage%20lease",
        "/v1/search?q=%20%20garage%09%20lease%20",
    ] {
        let body = get(&rig, &token, path).await.json();
        assert_eq!(body["query"], "garage lease", "{path}");
        assert_eq!(body["chats"].as_array().unwrap().len(), 1, "{path}");
        assert_eq!(body["workspacesTotal"], 1, "{path}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_token_that_is_not_one_hollow_gave_is_a_401() {
    let rig = rig().await;
    for token in ["not-a-token", "ph-00000000", ""] {
        let resp = rig
            .call("GET", "/v1/search?q=garage", Some(token), None)
            .await;
        assert_eq!(resp.status, 401, "{token:?}");
        assert_eq!(resp.json()["error"]["code"], "unauthorized");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chats_come_newest_first_and_only_the_persons_own_messages_are_searched() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let keys = [
        "s-20261107-5ea00041",
        "s-20261107-5ea00042",
        "s-20261107-5ea00043",
    ];
    for key in keys {
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &w.cwd,
            &[("Plan the garage lease", "Ok")],
            "acceptEdits",
        );
        // Distinct activity times, oldest first.
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
    // A message another agent sent, and one Hollow wrote for the person.
    let (by_agent, by_hollow) = ("s-20261107-5ea00044", "s-20261107-5ea00045");
    seed_chat(
        &rig.vault,
        by_agent,
        "claude",
        &w.cwd,
        &[("Check the garage lease", "Ok")],
        "acceptEdits",
    );
    crate::session_store::record_turn_asker(&rig.vault, by_agent, 1, "agent").unwrap();
    crate::session_store::rename_session(&rig.vault, by_agent, "Weekly").unwrap();
    seed_chat(
        &rig.vault,
        by_hollow,
        "claude",
        &w.cwd,
        &[("\u{27f2} Hollow check-in about the garage lease", "Ok")],
        "acceptEdits",
    );
    crate::session_store::rename_session(&rig.vault, by_hollow, "Check-in").unwrap();

    let body = get(&rig, &token, "/v1/search?q=garage").await.json();
    let found = strings(&body, "chats", "key");
    assert_eq!(found, [keys[2], keys[1], keys[0]], "newest activity first");
    // A page boundary keeps the order.
    let first = get(&rig, &token, "/v1/search?q=garage&limit=2")
        .await
        .json();
    assert_eq!(strings(&first, "chats", "key"), [keys[2], keys[1]]);
    let path = format!(
        "/v1/search?q=garage&limit=2&cursor={}",
        urlencoding::encode(first["nextCursor"].as_str().unwrap())
    );
    let second = get(&rig, &token, &path).await.json();
    assert_eq!(strings(&second, "chats", "key"), [keys[0]]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trashed_workspace_is_not_found() {
    let rig = rig().await;
    let token = rig.token();
    let kept = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let gone = crate::workspaces::create(&rig.vault, "Garage sale", "Sell it all.").unwrap();
    crate::workspaces::trash(&rig.vault, &gone.id).unwrap();
    let body = get(&rig, &token, "/v1/search?q=garage").await.json();
    assert_eq!(strings(&body, "workspaces", "id"), [kept.id.as_str()]);
    assert_eq!(body["workspacesTotal"], 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_example_cursor_in_the_fixture_is_one_hollow_accepts() {
    let rig = rig().await;
    let token = rig.token();
    let cursor = fixture("search/results.json")["nextCursor"]
        .as_str()
        .unwrap()
        .to_string();
    let path = format!(
        "/v1/search?q=garage&cursor={}",
        urlencoding::encode(&cursor)
    );
    let resp = get(&rig, &token, &path).await;
    assert_eq!(resp.status, 200, "{}", String::from_utf8_lossy(&resp.body));
    assert_eq!(resp.json()["workspaces"], json!([]));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_the_first_thirty_two_distinct_words_are_read_for_chats_and_workspaces_alike() {
    let rig = rig().await;
    let token = rig.token();
    // The workspace's Outcome and a chat's message hold w01 to w32, and nothing holds w33.
    let listed: Vec<String> = (1..=32).map(|n| format!("w{n:02}")).collect();
    let all = listed.join(" ");
    let w = crate::workspaces::create(&rig.vault, "Word list", &all).unwrap();
    let plan = format!("Plan {all}");
    seed_chat(
        &rig.vault,
        "s-20261107-5ea00051",
        "claude",
        &w.cwd,
        &[(plan.as_str(), "Ok")],
        "acceptEdits",
    );
    let url = |words: &[String]| format!("/v1/search?q={}", words.join("%20"));
    // 33 distinct words: the 33rd is not read, by the chats or by the workspaces.
    let mut words = listed.clone();
    words.push("w33".to_string());
    let body = get(&rig, &token, &url(&words)).await.json();
    assert_eq!(body["workspacesTotal"], 1, "{body}");
    assert_eq!(body["chats"].as_array().unwrap().len(), 1, "{body}");
    // A repeat is not a new word, in any case: w01, W01, w02 to w31 are 31 distinct words, so a
    // word nothing holds after them is the 32nd distinct word and IS read (32 raw words would
    // have cut it off), and nothing matches.
    let mut again = vec!["w01".to_string(), "W01".to_string()];
    again.extend(listed[1..31].iter().cloned());
    again.push("zebra".to_string());
    let repeated = get(&rig, &token, &url(&again)).await.json();
    assert_eq!(repeated["workspacesTotal"], 0, "{repeated}");
    assert_eq!(repeated["chats"], json!([]), "{repeated}");
    // A word among the first 32 still counts: one nothing holds, then 31 of the others.
    let mut early = vec!["zebra".to_string()];
    early.extend(listed[..31].iter().cloned());
    let none = get(&rig, &token, &url(&early)).await.json();
    assert_eq!(none["workspacesTotal"], 0, "{none}");
    assert_eq!(none["chats"], json!([]), "{none}");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_named_pipe_where_a_brief_should_be_never_hangs_a_search() {
    let rig = rig().await;
    let token = rig.token();
    let piped = crate::workspaces::create(&rig.vault, "Piped garage", "Goal").unwrap();
    let fine = crate::workspaces::create(&rig.vault, "Fine garage", "Goal").unwrap();
    let brief = rig.vault.join(&piped.path).join("WORKSPACE.md");
    std::fs::remove_file(&brief).unwrap();
    let fifo = std::ffi::CString::new(brief.to_str().unwrap().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let waited = tokio::time::timeout(
        Duration::from_secs(10),
        get(&rig, &token, "/v1/search?q=garage"),
    )
    .await;
    let Ok(resp) = waited else {
        // Let a reader that is stuck opening the pipe go, so the test run can end, then fail.
        use std::os::unix::fs::OpenOptionsExt;
        let _ = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&brief);
        panic!("the search hung on a named pipe");
    };
    assert_eq!(resp.status, 200);
    // The workspace whose brief is no file is skipped; the others are still found.
    assert_eq!(
        strings(&resp.json(), "workspaces", "id"),
        [fine.id.as_str()]
    );
}

/// 2026-11-08 review: the phone search follows the desktop's work-data rule
/// (`companion_docs::shown_on_devices`): a work workspace's chats only with
/// "include work data" on, a `linear-*` workspace's never.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn work_chats_stay_off_the_phone_unless_work_data_is_on() {
    let rig = rig().await;
    let token = rig.token();
    let personal =
        crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let work = crate::workspaces::create(&rig.vault, "Payroll garage", "Fix the export.").unwrap();
    let linear = crate::workspaces::create(&rig.vault, "Linear acme", "Ship the rollout.").unwrap();
    assert!(linear.id.starts_with("linear-"), "{}", linear.id);
    let chats = [
        ("s-20261108-5ea00051", &personal),
        ("s-20261108-5ea00052", &work),
        ("s-20261108-5ea00053", &linear),
    ];
    for (key, w) in chats {
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &w.cwd,
            &[("Plan the garage lease", "Ok")],
            "acceptEdits",
        );
    }
    let keys = |body: &Value| {
        let mut keys = strings(body, "chats", "key");
        keys.sort();
        keys
    };
    let off = get(&rig, &token, "/v1/search?q=garage").await.json();
    assert_eq!(keys(&off), [chats[0].0], "work data is off by default");
    std::fs::write(
        rig.store.dir().join("scope.json"),
        r#"{"includeWorkData": true}"#,
    )
    .unwrap();
    let on = get(&rig, &token, "/v1/search?q=garage").await.json();
    assert_eq!(keys(&on), [chats[0].0, chats[1].0], "Linear never");
    std::fs::write(rig.store.dir().join("scope.json"), "not json").unwrap();
    let broken = get(&rig, &token, "/v1/search?q=garage").await.json();
    assert_eq!(keys(&broken), [chats[0].0], "an unreadable scope is off");
}

/// Set front matter `key` to `value` in workspace `w`'s brief, and append
/// `tail` to it.
fn edit_brief(vault: &Path, w: &crate::workspaces::Workspace, key: &str, value: &str, tail: &str) {
    let brief = vault.join(&w.path).join("WORKSPACE.md");
    let text = std::fs::read_to_string(&brief).unwrap();
    let edited = crate::frontmatter::patch(
        &text,
        &[(key, crate::frontmatter::KeyEdit::Set(value.to_string()))],
        None,
    )
    .unwrap();
    std::fs::write(&brief, format!("{edited}{tail}")).unwrap();
}

/// Seeds one matching chat per workspace and returns the search's chat keys,
/// sorted, with "include work data" off and then on.
async fn keys_off_and_on(
    rig: &Rig,
    token: &str,
    chats: &[(&str, &crate::workspaces::Workspace)],
) -> (Vec<String>, Vec<String>) {
    for (key, w) in chats {
        seed_chat(
            &rig.vault,
            key,
            "claude",
            &w.cwd,
            &[("Plan the garage lease", "Ok")],
            "acceptEdits",
        );
    }
    let keys = |body: &Value| {
        let mut keys = strings(body, "chats", "key");
        keys.sort();
        keys
    };
    let scope = rig.store.dir().join("scope.json");
    let _ = std::fs::remove_file(&scope);
    let off = keys(&get(rig, token, "/v1/search?q=garage").await.json());
    std::fs::write(&scope, r#"{"includeWorkData": true}"#).unwrap();
    let on = keys(&get(rig, token, "/v1/search?q=garage").await.json());
    (off, on)
}

/// Second 2026-11-08 review: the phone's rule is an allow-list. A merge
/// marks its source `merged_into` before it moves the chats, and the deny
/// list before it skipped merged folders, so an interrupted merge showed a
/// work or `linear-*` workspace's chats. A merged workspace's chats now stay
/// off unless work data is on (even a personal one, until the merge ends),
/// and a `linear-*` one's always.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_interrupted_merge_keeps_work_and_linear_chats_off_the_phone() {
    let rig = rig().await;
    let token = rig.token();
    let target = crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let work = crate::workspaces::create(&rig.vault, "Payroll garage", "Fix the export.").unwrap();
    let linear = crate::workspaces::create(&rig.vault, "Linear acme", "Ship the rollout.").unwrap();
    let personal = crate::workspaces::create(&rig.vault, "Garden plans", "Plant it.").unwrap();
    assert!(linear.id.starts_with("linear-"), "{}", linear.id);
    for w in [&work, &linear, &personal] {
        edit_brief(&rig.vault, w, "merged_into", &target.id, "");
    }
    let (into, from_work, from_linear, from_personal) = (
        "s-20261108-5ea00061",
        "s-20261108-5ea00062",
        "s-20261108-5ea00063",
        "s-20261108-5ea00064",
    );
    let (off, on) = keys_off_and_on(
        &rig,
        &token,
        &[
            (into, &target),
            (from_work, &work),
            (from_linear, &linear),
            (from_personal, &personal),
        ],
    )
    .await;
    assert_eq!(off, [into], "work data off: merged folders stay off");
    assert_eq!(on, [into, from_work, from_personal], "Linear never");
}

/// Second 2026-11-08 review: a brief longer than the companion's document
/// limit keeps its front matter's classification, so a `scope: work`
/// workspace with a long brief stays off with work data off.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_long_brief_marked_scope_work_keeps_its_chats_off_the_phone() {
    let rig = rig().await;
    let token = rig.token();
    let personal =
        crate::workspaces::create(&rig.vault, "Garage lease", "Renew the lease.").unwrap();
    let long = crate::workspaces::create(&rig.vault, "Garden plans", "Plant it.").unwrap();
    let body = "\nGrow tomatoes.\n".repeat(30_000);
    assert!(body.len() as u64 > crate::companion_docs::MAX_DOC_BYTES);
    edit_brief(&rig.vault, &long, "scope", "work", &body);
    let (mine, theirs) = ("s-20261108-5ea00071", "s-20261108-5ea00072");
    let (off, on) = keys_off_and_on(&rig, &token, &[(mine, &personal), (theirs, &long)]).await;
    assert_eq!(off, [mine], "a long scope: work brief is work");
    assert_eq!(on, [mine, theirs], "work data on shows it");
}
