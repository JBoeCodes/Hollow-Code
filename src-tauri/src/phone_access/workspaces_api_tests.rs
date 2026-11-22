//! The v1.2 workspace routes over real HTTP, against a temporary vault
//! (`docs/phone-api/workspaces.md`).
use super::*;
use crate::phone_access::workspaces_api::{mime_of, section};
use base64::Engine as _;

fn write(vault: &Path, relative: &str, bytes: &[u8]) {
    let path = vault.join(relative);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

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

#[test]
fn a_brief_section_is_its_markdown_up_to_the_next_heading() {
    let brief = "---\ntitle: X\nstatus: active\n---\n\n# X\n\n## Outcome\n\nShip it.\n\n- one\n- two\n\n```md\n## Not a heading\n```\n\n## Next step\n\nDo the thing.\n";
    assert_eq!(
        section(brief, "outcome", 4_000).as_deref(),
        Some("Ship it.\n\n- one\n- two\n\n```md\n## Not a heading\n```")
    );
    assert_eq!(
        section(brief, "Next step", 4_000).as_deref(),
        Some("Do the thing.")
    );
    assert_eq!(section(brief, "decisions", 4_000), None);
    assert_eq!(
        section("## Outcome\n\n\n## Next step\nx", "outcome", 10),
        None
    );
    assert_eq!(
        section("## Outcome\nabcdefghij", "outcome", 4).as_deref(),
        Some("abcd")
    );
    // A front matter line that looks like a heading is not one.
    assert_eq!(section("---\n## Outcome\n---\nbody", "outcome", 10), None);
}

#[test]
fn a_files_type_comes_from_its_extension() {
    assert_eq!(mime_of("notes.MD"), ("text/markdown", true));
    assert_eq!(mime_of("a.csv"), ("text/csv", true));
    assert_eq!(mime_of("plan.pdf"), ("application/pdf", false));
    assert_eq!(mime_of("shot.JPEG"), ("image/jpeg", false));
    assert_eq!(mime_of("no-extension"), ("application/octet-stream", false));
    assert_eq!(
        mime_of("archive.tar.gz"),
        ("application/octet-stream", false)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_workspace_routes_need_a_token_and_never_write() {
    let rig = rig().await;
    let w = crate::workspaces::create(&rig.vault, "Quiet project", "Goal").unwrap();
    for path in [
        "/v1/workspaces/overview".to_string(),
        format!("/v1/workspaces/{}", w.id),
        format!("/v1/workspaces/{}/file?path=WORKSPACE.md", w.id),
    ] {
        let none = rig.call("GET", &path, None, None).await;
        assert_eq!(none.status, 401, "{path}");
    }
    let token = rig.token();
    let before = tree(&rig.vault.join("workspaces"));
    for path in [
        "/v1/workspaces/overview".to_string(),
        "/v1/workspaces/overview?scope=all".to_string(),
        format!("/v1/workspaces/{}", w.id),
        format!("/v1/workspaces/{}/file?path=WORKSPACE.md", w.id),
    ] {
        assert_eq!(get(&rig, &token, &path).await.status, 200, "{path}");
    }
    assert_eq!(
        tree(&rig.vault.join("workspaces")),
        before,
        "reads changed a file"
    );
    // Nothing writes: other methods find no route.
    for (method, path) in [
        ("POST", "/v1/workspaces/overview".to_string()),
        ("POST", format!("/v1/workspaces/{}", w.id)),
        ("PUT", format!("/v1/workspaces/{}/file?path=PLAN.md", w.id)),
        ("DELETE", format!("/v1/workspaces/{}", w.id)),
    ] {
        let resp = rig
            .call(method, &path, Some(&token), Some(json!({"text": "x"})))
            .await;
        assert_eq!(resp.status, 404, "{method} {path}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_overview_lists_pinned_first_then_the_newest_and_leaves_out_chats_and_the_main_agent() {
    let rig = rig().await;
    let token = rig.token();
    let alpha = crate::workspaces::create(&rig.vault, "Alpha project", "Goal").unwrap();
    let bravo = crate::workspaces::create(&rig.vault, "Bravo project", "Goal").unwrap();
    let charlie = crate::workspaces::create(&rig.vault, "Charlie project", "Goal").unwrap();
    let delta = crate::workspaces::create(&rig.vault, "Delta project", "Goal").unwrap();
    crate::workspaces::update_meta(&rig.vault, &charlie.id, None, Some("done")).unwrap();
    crate::workspace_pins::set_pin(&rig.vault, &delta.id, crate::workspace_pins::PinChange::Pin)
        .unwrap();
    // Bravo has the newest conversation.
    seed_chat(
        &rig.vault,
        "s-20261107-0000a001",
        "claude",
        &bravo.cwd,
        &[("Hi", "Hello")],
        "acceptEdits",
    );
    let body = get(&rig, &token, "/v1/workspaces/overview").await.json();
    same_shape(&body, &fixture("workspaces/overview.json"), "overview");
    let ids: Vec<&str> = body["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [delta.id.as_str(), bravo.id.as_str(), alpha.id.as_str()]
    );
    assert_eq!(body["total"], 3);
    let delta_card = &body["workspaces"][0];
    assert_eq!(delta_card["pinned"], 1);
    assert_eq!(delta_card["chatCount"], 0);
    assert_eq!(delta_card["lastActivityAt"], Value::Null);
    assert_eq!(delta_card["kind"], "vault");
    assert_eq!(
        delta_card["nextStep"],
        "Start with the request and attached sources."
    );
    let bravo_card = &body["workspaces"][1];
    assert_eq!(bravo_card["pinned"], Value::Null);
    assert_eq!(bravo_card["chatCount"], 1);
    assert_eq!(bravo_card["runningCount"], 0);
    assert_eq!(bravo_card["waitingCount"], 0);
    assert!(bravo_card["lastActivityAt"].is_string());

    // `all` adds the finished workspace; chats and the main agent never show.
    let all = get(&rig, &token, "/v1/workspaces/overview?scope=all")
        .await
        .json();
    let all_ids: Vec<&str> = all["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap())
        .collect();
    assert!(all_ids.contains(&charlie.id.as_str()));
    assert!(!all_ids.contains(&"quick-chats") && !all_ids.contains(&"main"));
    assert_eq!(all["total"], 4);
    let done = all["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == charlie.id.as_str())
        .unwrap();
    assert_eq!(done["status"], "done");

    let bad = get(&rig, &token, "/v1/workspaces/overview?scope=everything").await;
    assert_eq!(bad.status, 400);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_detail_has_the_outcome_the_plan_the_chats_and_the_results() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Release work", "Ship the release.").unwrap();
    let other = crate::workspaces::create(&rig.vault, "Other work", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/PLAN.md", w.path),
        b"# Plan\n\n- [x] Done thing\n- [ ] Open one\n- [ ] Open two\n",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/notes.md", w.path),
        b"# Release notes\n\nHello.\n",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/data.csv", w.path),
        b"a,b\n1,2\n",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/deep/shot.png", w.path),
        b"\x89PNG",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/.hidden/secret.md", w.path),
        b"# Secret\n",
    );
    write(&rig.vault, &format!("{}/outputs/.dotfile", w.path), b"x");
    seed_chat(
        &rig.vault,
        "s-20261107-0000b001",
        "claude",
        &w.cwd,
        &[("Ship?", "Shipping.")],
        "acceptEdits",
    );
    seed_chat(
        &rig.vault,
        "s-20261107-0000b002",
        "claude",
        &other.cwd,
        &[("Elsewhere", "Yes.")],
        "acceptEdits",
    );

    let resp = get(&rig, &token, &format!("/v1/workspaces/{}", w.id)).await;
    assert_eq!(resp.status, 200);
    let body = resp.json();
    same_shape(&body, &fixture("workspaces/detail.json"), "detail");
    assert_eq!(body["workspace"]["id"], w.id.as_str());
    assert_eq!(body["workspace"]["chatCount"], 1);
    assert_eq!(body["outcome"], "Ship the release.");
    assert_eq!(body["plan"]["openItems"], 2);
    assert_eq!(body["plan"]["truncated"], false);
    assert!(body["plan"]["text"].as_str().unwrap().starts_with("# Plan"));
    let documents: Vec<&str> = body["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    // A new workspace starts with all four context documents.
    assert_eq!(
        documents,
        ["WORKSPACE.md", "PLAN.md", "SOURCES.md", "AGENTS.md"]
    );
    let chats = body["chats"].as_array().unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0]["key"], "s-20261107-0000b001");
    assert_eq!(chats[0]["workspace"]["id"], w.id.as_str());
    assert_eq!(body["chatsHasMore"], false);
    let results = body["results"].as_array().unwrap();
    let mut paths: Vec<&str> = results
        .iter()
        .map(|r| r["path"].as_str().unwrap())
        .collect();
    paths.sort();
    assert_eq!(
        paths,
        [
            "outputs/data.csv",
            "outputs/deep/shot.png",
            "outputs/notes.md"
        ]
    );
    let notes = results.iter().find(|r| r["name"] == "notes.md").unwrap();
    assert_eq!(notes["title"], "Release notes");
    assert_eq!(notes["kind"], "document");
    assert_eq!(notes["size"], 24);
    let csv = results.iter().find(|r| r["name"] == "data.csv").unwrap();
    assert_eq!(csv["kind"], "table");
    assert_eq!(csv["title"], "Data");
    assert!(results.iter().all(|r| r.get("review").is_none()));
    assert_eq!(body["resultsHasMore"], false);

    // A new workspace starts with Hollow's own plan (two open items)...
    let fresh = get(&rig, &token, &format!("/v1/workspaces/{}", other.id))
        .await
        .json();
    assert_eq!(fresh["plan"]["openItems"], 2);
    assert_eq!(fresh["chats"].as_array().unwrap().len(), 1);
    assert_eq!(fresh["results"], json!([]));
    // ...and one without a plan says so.
    std::fs::remove_file(rig.vault.join(format!("{}/PLAN.md", other.path))).unwrap();
    let bare = get(&rig, &token, &format!("/v1/workspaces/{}", other.id))
        .await
        .json();
    assert_eq!(bare["plan"], Value::Null);
    let names: Vec<&str> = bare["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["WORKSPACE.md", "SOURCES.md", "AGENTS.md"]);

    // The built-ins have a detail too.
    crate::workspaces::ensure_quick_chats(&rig.vault).unwrap();
    let quick = get(&rig, &token, "/v1/workspaces/quick-chats").await;
    assert_eq!(quick.status, 200);
    assert_eq!(quick.json()["workspace"]["kind"], "quick");

    for missing in ["/v1/workspaces/no-such-workspace", "/v1/workspaces/Bad_Id"] {
        let resp = get(&rig, &token, missing).await;
        assert_eq!(resp.status, 404, "{missing}");
        assert_eq!(resp.json()["error"]["code"], "not_found");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_detail_lists_at_most_fifty_chats_and_a_hundred_results() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Busy work", "Goal").unwrap();
    for i in 0..52 {
        seed_chat(
            &rig.vault,
            &format!("s-20261107-{i:08x}"),
            "claude",
            &w.cwd,
            &[("Hi", "Hello")],
            "acceptEdits",
        );
    }
    for i in 0..102 {
        write(&rig.vault, &format!("{}/outputs/r{i:03}.txt", w.path), b"x");
    }
    let body = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(body["chats"].as_array().unwrap().len(), 50);
    assert_eq!(body["chatsHasMore"], true);
    assert_eq!(body["results"].as_array().unwrap().len(), 100);
    assert_eq!(body["resultsHasMore"], true);
    assert_eq!(body["workspace"]["chatCount"], 52);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_file_route_sends_text_as_text_and_the_rest_as_base64() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Files work", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/outputs/README.md", w.path),
        b"# Hollow Companion App\n\nHello.\n",
    );
    let png = base64::engine::general_purpose::STANDARD
        .decode(
            fixture("workspaces/file-binary.json")["content"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
    write(&rig.vault, &format!("{}/outputs/dot.png", w.path), &png);

    let path = |rel: &str| {
        format!(
            "/v1/workspaces/{}/file?path={}",
            w.id,
            urlencoding::encode(rel)
        )
    };
    let text = get(&rig, &token, &path("outputs/README.md")).await;
    assert_eq!(text.status, 200);
    let text = text.json();
    same_shape(&text, &fixture("workspaces/file-text.json"), "file-text");
    assert_eq!(text["encoding"], "utf8");
    assert_eq!(text["mime"], "text/markdown");
    assert_eq!(text["kind"], "document");
    assert_eq!(text["name"], "README.md");
    assert_eq!(text["content"], "# Hollow Companion App\n\nHello.\n");
    assert_eq!(text["size"], 31);
    assert_eq!(text["truncated"], false);

    let image = get(&rig, &token, &path("outputs/dot.png")).await.json();
    same_shape(
        &image,
        &fixture("workspaces/file-binary.json"),
        "file-binary",
    );
    assert_eq!(image["encoding"], "base64");
    assert_eq!(image["mime"], "image/png");
    assert_eq!(image["kind"], "image");
    assert_eq!(
        image["content"],
        fixture("workspaces/file-binary.json")["content"]
    );
    assert_eq!(image["size"], 70);

    // The brief and the plan open the same way.
    let brief = get(&rig, &token, &path("WORKSPACE.md")).await.json();
    assert_eq!(brief["encoding"], "utf8");
    assert!(brief["content"].as_str().unwrap().contains("## Outcome"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_large_text_file_is_cut_on_a_character_boundary_and_a_huge_one_is_refused() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Big files", "Goal").unwrap();
    // Each "é" is two bytes, so the 1 MiB cut falls inside a character
    // when the file starts with one extra byte.
    let mut text = String::from("a");
    text.push_str(&"é".repeat(600_000));
    write(
        &rig.vault,
        &format!("{}/outputs/long.txt", w.path),
        text.as_bytes(),
    );
    let path = |rel: &str| {
        format!(
            "/v1/workspaces/{}/file?path={}",
            w.id,
            urlencoding::encode(rel)
        )
    };
    let body = get(&rig, &token, &path("outputs/long.txt")).await.json();
    assert_eq!(body["truncated"], true);
    assert_eq!(body["size"], text.len());
    let cut = body["content"].as_str().unwrap();
    assert!(
        cut.len() <= 1024 * 1024 && cut.len() > 1024 * 1024 - 4,
        "{}",
        cut.len()
    );
    assert!(text.starts_with(cut));

    // A binary file over 10 MiB is too large.
    let big = rig.vault.join(format!("{}/outputs/big.bin", w.path));
    std::fs::create_dir_all(big.parent().unwrap()).unwrap();
    std::fs::File::create(&big)
        .unwrap()
        .set_len(10 * 1024 * 1024 + 1)
        .unwrap();
    let resp = get(&rig, &token, &path("outputs/big.bin")).await;
    assert_eq!(resp.status, 413);
    assert_eq!(resp.json(), fixture("workspaces/error-file-too-large.json"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_text_name_with_bytes_that_are_not_text_travels_as_bytes() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Odd files", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/outputs/odd.md", w.path),
        &[0xff, 0xfe, 0x41],
    );
    let path = format!("/v1/workspaces/{}/file?path=outputs%2Fodd.md", w.id);
    let body = get(&rig, &token, &path).await.json();
    assert_eq!(body["encoding"], "base64");
    assert_eq!(body["mime"], "application/octet-stream");
    assert_eq!(body["content"], "//5B");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_file_route_stays_inside_the_workspace_folder() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Mine", "Goal").unwrap();
    let other = crate::workspaces::create(&rig.vault, "Theirs", "Goal").unwrap();
    write(&rig.vault, &format!("{}/outputs/ok.md", w.path), b"# ok\n");
    write(
        &rig.vault,
        &format!("{}/.hollow/results.json", w.path),
        b"{}",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/theirs.md", other.path),
        b"# theirs\n",
    );
    let outside = tempfile::Builder::new()
        .prefix("outside-")
        .tempdir()
        .unwrap();
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            rig.vault.join(format!("{}/outputs/leak.txt", w.path)),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            rig.vault.join(format!("{}/outputs/theirs.md", other.path)),
            rig.vault.join(format!("{}/outputs/sibling.md", w.path)),
        )
        .unwrap();
    }
    let path = |rel: &str| {
        format!(
            "/v1/workspaces/{}/file?path={}",
            w.id,
            urlencoding::encode(rel)
        )
    };
    for refused in [
        "",
        "   ",
        "..",
        "../theirs.md",
        "outputs/../../x",
        "/etc/passwd",
        "outputs//ok.md",
        "outputs/",
        "outputs\\ok.md",
        ".hollow/results.json",
        "outputs/.hidden",
        "./outputs/ok.md",
        "outputs/\u{0}ok.md",
    ] {
        let resp = get(&rig, &token, &path(refused)).await;
        assert_eq!(resp.status, 400, "{refused:?}");
        assert_eq!(resp.json()["error"]["code"], "bad_request");
    }
    let long = "a/".repeat(300);
    assert_eq!(get(&rig, &token, &path(&long)).await.status, 400);
    assert_eq!(get(&rig, &token, &path("outputs/ok.md")).await.status, 200);
    // A folder, a missing file and an unknown workspace are 404.
    assert_eq!(get(&rig, &token, &path("outputs")).await.status, 404);
    assert_eq!(
        get(&rig, &token, &path("outputs/none.md")).await.status,
        404
    );
    let elsewhere = format!(
        "/v1/workspaces/no-such-one/file?path={}",
        urlencoding::encode("outputs/ok.md")
    );
    assert_eq!(get(&rig, &token, &elsewhere).await.status, 404);
    #[cfg(unix)]
    {
        // A link out of the vault, and a link into another workspace.
        assert_eq!(
            get(&rig, &token, &path("outputs/leak.txt")).await.status,
            404
        );
        assert_eq!(
            get(&rig, &token, &path("outputs/sibling.md")).await.status,
            404
        );
    }
}

// ── The independent review's findings (Claude, PR #1189) ─────────────────────

/// A symbolic link `at` that points to `target`.
#[cfg(unix)]
fn link(target: &Path, at: &Path) {
    std::fs::create_dir_all(at.parent().unwrap()).unwrap();
    let _ = std::fs::remove_file(at);
    std::os::unix::fs::symlink(target, at).unwrap();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_inside_the_workspace_never_reaches_hollows_state_or_another_workspace() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Mine", "Goal").unwrap();
    let other = crate::workspaces::create(&rig.vault, "Theirs", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/.hollow/results.json", w.path),
        b"{\"state\":1}",
    );
    write(
        &rig.vault,
        &format!("{}/outputs/theirs.md", other.path),
        b"# theirs\n",
    );
    let mine = rig.vault.join(&w.path);
    // `outputs/state` lands in this workspace's own `.hollow/`: the rule
    // about hidden parts applies to where a path ends up, not to its text.
    link(&mine.join(".hollow"), &mine.join("outputs/state"));
    link(
        &rig.vault.join(&other.path).join("outputs"),
        &mine.join("outputs/team"),
    );
    let path = |rel: &str| {
        format!(
            "/v1/workspaces/{}/file?path={}",
            w.id,
            urlencoding::encode(rel)
        )
    };
    for rel in ["outputs/state/results.json", "outputs/team/theirs.md"] {
        assert_eq!(get(&rig, &token, &path(rel)).await.status, 404, "{rel}");
    }
    // Neither link is listed as a Result.
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["results"], json!([]));
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plan_or_document_that_is_a_link_out_of_the_workspace_is_missing() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Mine", "Goal").unwrap();
    let other = crate::workspaces::create(&rig.vault, "Theirs", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/PLAN.md", other.path),
        b"# Plan\n\n- [ ] Their secret step\n",
    );
    let outside = tempfile::Builder::new()
        .prefix("outside-")
        .tempdir()
        .unwrap();
    std::fs::write(outside.path().join("key"), "private").unwrap();
    let mine = rig.vault.join(&w.path);
    // The plan is another workspace's; SOURCES.md is a file outside the
    // vault; AGENTS.md is a link to a file of this workspace, which is fine.
    link(
        &rig.vault.join(&other.path).join("PLAN.md"),
        &mine.join("PLAN.md"),
    );
    link(&outside.path().join("key"), &mine.join("SOURCES.md"));
    write(
        &rig.vault,
        &format!("{}/outputs/agents.md", w.path),
        b"# Agents\n",
    );
    link(&mine.join("outputs/agents.md"), &mine.join("AGENTS.md"));
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["plan"], Value::Null);
    let names: Vec<&str> = detail["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["WORKSPACE.md", "AGENTS.md"]);
    // The file route agrees about the plan.
    let plan = get(
        &rig,
        &token,
        &format!("/v1/workspaces/{}/file?path=PLAN.md", w.id),
    )
    .await;
    assert_eq!(plan.status, 404);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_linked_outputs_folder_lists_nothing() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Linked", "Goal").unwrap();
    let outside = tempfile::Builder::new()
        .prefix("outside-")
        .tempdir()
        .unwrap();
    std::fs::write(outside.path().join("leak.md"), "# leak\n").unwrap();
    let outputs = rig.vault.join(&w.path).join("outputs");
    let _ = std::fs::remove_dir_all(&outputs);
    link(outside.path(), &outputs);
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["results"], json!([]));
    assert_eq!(detail["resultsHasMore"], false);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_named_pipe_where_a_plan_should_be_never_hangs_a_route() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Piped", "Goal").unwrap();
    // A new workspace has a plan: put the pipe where it is.
    let plan = rig.vault.join(&w.path).join("PLAN.md");
    let _ = std::fs::remove_file(&plan);
    let fifo = std::ffi::CString::new(plan.to_str().unwrap().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    let wait = std::time::Duration::from_secs(10);
    let detail = tokio::time::timeout(wait, get(&rig, &token, &format!("/v1/workspaces/{}", w.id)))
        .await
        .expect("the detail hung on a named pipe");
    assert_eq!(detail.status, 200);
    assert_eq!(detail.json()["plan"], Value::Null);
    let file = tokio::time::timeout(
        wait,
        get(
            &rig,
            &token,
            &format!("/v1/workspaces/{}/file?path=PLAN.md", w.id),
        ),
    )
    .await
    .expect("the file route hung on a named pipe");
    assert_eq!(file.status, 404);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_plus_in_a_file_name_is_a_plus_and_a_space_travels_as_percent_20() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Names", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/outputs/C++ notes.md", w.path),
        b"# C++\n",
    );
    for query in [
        "outputs/C%2B%2B%20notes.md",
        // A bare plus is a plus on this route (not a space).
        "outputs/C++%20notes.md",
    ] {
        let resp = get(
            &rig,
            &token,
            &format!("/v1/workspaces/{}/file?path={query}", w.id),
        )
        .await;
        assert_eq!(resp.status, 200, "{query}");
        assert_eq!(resp.json()["name"], "C++ notes.md");
    }
    // A plus read as a space finds no such file.
    let spaced = get(
        &rig,
        &token,
        &format!("/v1/workspaces/{}/file?path=outputs/C%2B%2B+notes.md", w.id),
    )
    .await;
    assert_eq!(spaced.status, 404);
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["results"][0]["path"], "outputs/C++ notes.md");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn contract_edges_a_leading_control_character_an_empty_scope_and_unopenable_results() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Edges", "Goal").unwrap();
    write(&rig.vault, &format!("{}/outputs/ok.md", w.path), b"# ok\n");
    // A name with a backslash is legal on disk, but the file route can't
    // be asked for it, so it isn't listed.
    write(
        &rig.vault,
        &format!("{}/outputs/odd\\name.md", w.path),
        b"# odd\n",
    );
    // A control character is refused even at the very start (no trimming).
    let resp = get(
        &rig,
        &token,
        &format!("/v1/workspaces/{}/file?path=%0Aoutputs/ok.md", w.id),
    )
    .await;
    assert_eq!(resp.status, 400);
    // An empty or undecodable scope is a 400, an absent one the default.
    assert_eq!(
        get(&rig, &token, "/v1/workspaces/overview?scope=")
            .await
            .status,
        400
    );
    assert_eq!(
        get(&rig, &token, "/v1/workspaces/overview").await.status,
        200
    );
    let paths: Vec<String> = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json()["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, ["outputs/ok.md"]);
}

#[test]
fn the_results_walk_stops_at_its_limit_and_says_more_exist() {
    let dir = tempfile::Builder::new().prefix("walk-").tempdir().unwrap();
    for n in 0..8 {
        write(dir.path(), &format!("outputs/note-{n}.md"), b"# n\n");
    }
    let (rows, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 5);
    assert!(more);
    assert!(rows.len() <= 5, "{}", rows.len());
    let (rows, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 8);
    assert_eq!(rows.len(), 8);
    assert!(!more);
}

#[test]
fn folders_count_toward_the_results_walk_limit_too() {
    let dir = tempfile::Builder::new().prefix("walk-").tempdir().unwrap();
    for n in 0..8 {
        std::fs::create_dir_all(dir.path().join(format!("outputs/empty-{n}"))).unwrap();
    }
    // No file anywhere, yet the walk stops at its limit and says so.
    let (rows, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 5);
    assert!(rows.is_empty());
    assert!(more);
    let (_, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 8);
    assert!(!more);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_link_that_points_at_a_link_ends_where_the_last_one_ends() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Chained", "Goal").unwrap();
    let other = crate::workspaces::create(&rig.vault, "Theirs", "Goal").unwrap();
    write(
        &rig.vault,
        &format!("{}/PLAN.md", other.path),
        b"# Plan\n\n- [ ] Their secret step\n",
    );
    write(
        &rig.vault,
        &format!("{}/.hollow/results.json", w.path),
        b"{\"state\":1}",
    );
    let mine = rig.vault.join(&w.path);
    std::fs::create_dir_all(mine.join("outputs")).unwrap();
    // PLAN.md -> outputs/hop-plan -> another workspace's plan.
    link(
        &rig.vault.join(&other.path).join("PLAN.md"),
        &mine.join("outputs/hop-plan"),
    );
    let _ = std::fs::remove_file(mine.join("PLAN.md"));
    link(&mine.join("outputs/hop-plan"), &mine.join("PLAN.md"));
    // outputs/state-2 -> outputs/state-1 -> .hollow
    link(&mine.join(".hollow"), &mine.join("outputs/state-1"));
    link(&mine.join("outputs/state-1"), &mine.join("outputs/state-2"));
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["plan"], Value::Null);
    for rel in [
        "PLAN.md",
        "outputs/hop-plan",
        "outputs/state-2/results.json",
    ] {
        let path = format!(
            "/v1/workspaces/{}/file?path={}",
            w.id,
            urlencoding::encode(rel)
        );
        assert_eq!(get(&rig, &token, &path).await.status, 404, "{rel}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_outcome_the_plan_and_the_next_step_are_cut_at_their_limits() {
    let rig = rig().await;
    let token = rig.token();
    let w = crate::workspaces::create(&rig.vault, "Long text", "Goal").unwrap();
    let brief_file = rig.vault.join(&w.path).join("WORKSPACE.md");
    let old = std::fs::read_to_string(&brief_file).unwrap();
    let front: String = old.split_inclusive("---\n").take(2).collect();
    std::fs::write(
        &brief_file,
        format!(
            "{front}\n# Long text\n\n## Outcome\n\n{}\n\n## Next step\n\n{}\n",
            "o".repeat(5_000),
            "n".repeat(400)
        ),
    )
    .unwrap();
    write(
        &rig.vault,
        &format!("{}/PLAN.md", w.path),
        format!("# Plan\n\n{}\n", "p".repeat(70_000)).as_bytes(),
    );
    let detail = get(&rig, &token, &format!("/v1/workspaces/{}", w.id))
        .await
        .json();
    assert_eq!(detail["outcome"].as_str().unwrap().chars().count(), 4_000);
    assert_eq!(
        detail["plan"]["text"].as_str().unwrap().chars().count(),
        60_000
    );
    assert_eq!(detail["plan"]["truncated"], true);
    assert_eq!(
        detail["workspace"]["nextStep"]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        240
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_trashed_workspace_is_gone_and_an_archived_one_still_opens() {
    let rig = rig().await;
    let token = rig.token();
    let gone = crate::workspaces::create(&rig.vault, "Gone soon", "Goal").unwrap();
    let old = crate::workspaces::create(&rig.vault, "Old one", "Goal").unwrap();
    crate::workspaces::update_meta(&rig.vault, &old.id, None, Some("archived")).unwrap();
    crate::workspaces::trash(&rig.vault, &gone.id).unwrap();
    for path in [
        format!("/v1/workspaces/{}", gone.id),
        format!("/v1/workspaces/{}/file?path=WORKSPACE.md", gone.id),
    ] {
        assert_eq!(get(&rig, &token, &path).await.status, 404, "{path}");
    }
    let archived = get(&rig, &token, &format!("/v1/workspaces/{}", old.id)).await;
    assert_eq!(archived.status, 200);
    assert_eq!(archived.json()["workspace"]["status"], "archived");
}

/// Q-R21: a resolved path's file opens with no link anywhere in its path on
/// macOS, so a workspace folder swapped for a link after the resolve is
/// refused; the last-step-only open (`open_regular`, for the results walk)
/// still follows a link higher up.
#[cfg(target_os = "macos")]
#[test]
fn a_resolved_workspace_file_with_a_link_in_its_path_is_refused() {
    use crate::phone_access::workspaces_api::{open_regular, open_resolved};
    let root = tempfile::Builder::new()
        .prefix("nofollow-any-")
        .tempdir()
        .unwrap();
    let root = root.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("real")).unwrap();
    std::fs::write(root.join("real/PLAN.md"), "plan").unwrap();
    std::os::unix::fs::symlink(root.join("real"), root.join("swapped")).unwrap();
    assert!(open_resolved(&root.join("real/PLAN.md")).is_ok());
    assert!(open_resolved(&root.join("swapped/PLAN.md")).is_err());
    assert!(open_regular(&root.join("swapped/PLAN.md")).is_ok());
}

/// R2 (ENG-117): hidden entries count toward the results walk's limit. They
/// are filtered out of the listing, but the walk still reads them, so a
/// folder of dotfiles was scanned in full without ever reaching the cap.
#[test]
fn hidden_entries_count_toward_the_results_walk_limit() {
    let dir = tempfile::Builder::new().prefix("walk-").tempdir().unwrap();
    for n in 0..20 {
        write(dir.path(), &format!("outputs/.hidden-{n}"), b"x");
    }
    let (rows, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 5);
    assert!(rows.is_empty());
    assert!(more, "twenty dotfiles went uncounted");
    let (_, more) = crate::phone_access::workspaces_api::results_of(dir.path(), 20);
    assert!(!more);
}
