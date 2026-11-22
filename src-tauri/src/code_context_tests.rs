//! CC2: the contract (name-scan floor, precision, resolves-elsewhere,
//! stale), the budget and cursor, strict schemas, and each tool on a
//! fixture repo.

use super::*;
use std::process::{Command, Stdio};
use tempfile::TempDir;

/// A git work tree in a temp folder holding `files` (untracked; git lists
/// them with `--others`). Every git call has stdin closed.
pub(crate) fn repo(files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let ok = Command::new("git")
        .args(["init", "-q"])
        .arg(dir.path())
        .stdin(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git init");
    for (path, text) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, text).unwrap();
    }
    dir
}

/// A target on `repo` whose index lives in `cache`.
pub(crate) fn target(repo: &TempDir, cache: &TempDir) -> Target {
    Target {
        root: std::fs::canonicalize(repo.path()).unwrap(),
        index_dir: Some(cache.path().to_path_buf()),
        vault: None,
    }
}

/// The fixture repo the contract and parity tests share.
pub(crate) fn fixture() -> TempDir {
    repo(&[
        ("src/records.rs", "pub fn decisions() -> u32 { helper() }\nfn helper() -> u32 { 1 }\npub fn check() {}\n"),
        ("src/memory_guard.rs", "pub fn check() {}\npub fn guard() { check(); }\n"),
        (
            "src/user.rs",
            "pub fn a() {\n    crate::records::decisions();\n    crate::memory_guard::check();\n    check();\n    vec![crate::records::decisions()];\n}\npub fn b() { a(); }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { super::a(); }\n}\n",
        ),
        ("src/cmd.rs", "#[tauri::command]\npub fn set_flag() { crate::user::b(); }\n"),
        ("ui/rows.ts", "export function decisionRows() { return 1; }\n"),
        ("ui/page.tsx", "import { decisionRows } from './rows';\nexport function Page() {\n  invoke('set_flag');\n  return decisionRows();\n}\n"),
        ("py/cases.py", "def case_text():\n    return 1\n"),
    ])
}

fn text(result: &Value, block: usize) -> Value {
    serde_json::from_str(result["content"][block]["text"].as_str().unwrap()).unwrap()
}

fn run(t: &Target, tool: &str, args: Value) -> Value {
    let r = call(tool, &args, t);
    assert_eq!(r["isError"], false, "{tool}: {r}");
    r
}

/// Every hit of a callers result: `(path, line, resolved, how)`.
fn hits(v: &Value) -> Vec<(String, i64, bool, String)> {
    let mut out = Vec::new();
    for f in v["files"].as_array().unwrap() {
        for h in f["hits"].as_array().unwrap() {
            out.push((
                f["path"].as_str().unwrap().to_string(),
                h["line"].as_i64().unwrap(),
                h["resolved"].as_bool().unwrap(),
                h["how"].as_str().unwrap().to_string(),
            ));
        }
    }
    out
}

#[test]
fn contract_name_scan_is_the_floor_and_resolution_only_annotates() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let r = run(
        &t,
        "callers",
        json!({ "name": "check", "qualifier": "memory_guard" }),
    );
    let v = text(&r, 0);
    assert_eq!(v["definitions"].as_array().unwrap().len(), 1);
    // Every textual call of `check` in Rust is listed (the floor): the
    // qualified call, the same-file call and the bare one in user.rs.
    let all = hits(&v);
    assert_eq!(
        all,
        vec![
            ("src/memory_guard.rs".into(), 2, true, "same-file".into()),
            ("src/user.rs".into(), 3, true, "qualifier".into()),
            ("src/user.rs".into(), 4, false, "ambiguous(2)".into()),
        ]
    );
    assert_eq!(
        v["totals"],
        json!({ "nameScanHits": 3, "resolved": 2, "unresolved": 1, "resolvesElsewhere": 0, "otherLanguages": 0 })
    );
    for key in ["head", "fingerprint", "indexedAt", "dirtyFiles"] {
        assert!(v["repo"].get(key).is_some(), "{key}");
    }
    assert_eq!(v["files"][0]["fileHash"].as_str().unwrap().len(), 12);
    assert_eq!(
        v["languages"],
        json!(["rust"]),
        "the target's language by default"
    );
    // `precision:"resolved"` drops only the unresolved hits.
    let p = text(
        &run(
            &t,
            "callers",
            json!({ "name": "check", "qualifier": "memory_guard", "precision": "resolved" }),
        ),
        0,
    );
    assert_eq!(hits(&p).len(), 2);
    assert_eq!(
        p["totals"], v["totals"],
        "totals always count the full scan"
    );
    // Asked about records' check, the memory_guard hits resolve elsewhere.
    let e = text(
        &run(
            &t,
            "callers",
            json!({ "name": "check", "qualifier": "records" }),
        ),
        0,
    );
    assert_eq!(e["totals"]["resolvesElsewhere"], 2);
    assert!(hits(&e).iter().all(|h| !h.2));
    // Macro arguments are found; a Tauri invoke reaches the Rust command.
    let d = text(&run(&t, "callers", json!({ "name": "decisions" })), 0);
    assert!(hits(&d).contains(&("src/user.rs".into(), 5, true, "macro-args:qualifier".into())));
    let c = text(&run(&t, "callers", json!({ "name": "set_flag" })), 0);
    assert_eq!(
        hits(&c),
        vec![("ui/page.tsx".into(), 3, true, "invoke-string".into())]
    );
    // includeTests:false filters test code, which is otherwise flagged.
    let a = text(&run(&t, "callers", json!({ "name": "a" })), 0);
    assert!(a["files"].as_array().unwrap().iter().any(|f| f["hits"]
        .as_array()
        .unwrap()
        .iter()
        .any(|h| h["caller"]["name"] == "t")));
    let a = text(
        &run(&t, "callers", json!({ "name": "a", "includeTests": false })),
        0,
    );
    assert_eq!(hits(&a).len(), 1);
}

#[test]
fn contract_an_edited_file_is_flagged_stale() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let path = crate::code_index::ensure(&t.root, t.index_dir.as_deref()).unwrap();
    std::fs::write(repo.path().join("src/user.rs"), "pub fn a() {}\n").unwrap();
    // The open index predates the edit (no re-index between).
    let index = Index::open(&path).unwrap();
    let args = json!({ "name": "check", "qualifier": "memory_guard" });
    let mut a = args.clone();
    a["maxBytes"] = json!(DEFAULT_MAX_BYTES);
    let req = Request::take("callers", &mut a).unwrap();
    let blocks = answer("callers", a.as_object().unwrap(), &Some(req), &index, &t).unwrap();
    let v: Value = serde_json::from_str(blocks[0]["text"].as_str().unwrap()).unwrap();
    let user = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/user.rs")
        .unwrap();
    assert_eq!(user["stale"], true);
    let memory = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/memory_guard.rs")
        .unwrap();
    assert!(memory.get("stale").is_none());
    let envelope: Value = serde_json::from_str(blocks[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(envelope["envelope"]["complete"], false);
    assert!(envelope["envelope"]["capReasons"]
        .as_array()
        .unwrap()
        .contains(&json!("stale_files")));
    // A fresh call re-indexes: the edit is seen and nothing is stale.
    let fresh = text(&run(&t, "callers", args), 0);
    assert!(hits(&fresh).iter().all(|h| h.0 != "src/user.rs"));
}

#[test]
fn budget_cuts_500_hits_at_16_kb_and_the_cursor_pages_the_rest() {
    let mut files: Vec<(String, String)> = vec![("src/hot.rs".into(), "pub fn hot() {}\n".into())];
    for i in 0..50 {
        let body: String = (0..10).map(|_| "    crate::hot::hot();\n").collect();
        files.push((
            format!("src/m{i:02}.rs"),
            format!("pub fn f{i}() {{\n{body}}}\n"),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (repo, cache) = (repo(&refs), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let mut seen: Vec<(String, i64)> = Vec::new();
    let mut args = json!({ "name": "hot" });
    let mut pages = 0;
    loop {
        let r = run(&t, "callers", args.clone());
        let body = r["content"][0]["text"].as_str().unwrap();
        assert!(body.len() <= DEFAULT_MAX_BYTES, "{} bytes", body.len());
        let v = text(&r, 0);
        assert_eq!(v["totals"]["nameScanHits"], 500);
        seen.extend(hits(&v).into_iter().map(|h| (h.0, h.1)));
        let env = text(&r, 1)["envelope"].clone();
        pages += 1;
        match env.get("nextCursor") {
            Some(c) => {
                assert_eq!(env["budgeted"], true);
                assert_eq!(env["capReasons"], json!(["byte_budget"]));
                args["cursor"] = c.clone();
            }
            None => break,
        }
    }
    assert!(pages > 1, "500 hits do not fit 16 KB");
    let unique: BTreeSet<(String, i64)> = seen.iter().cloned().collect();
    assert_eq!(seen.len(), 500, "no hit lost or repeated");
    assert_eq!(unique.len(), 500);
}

#[test]
fn strict_schemas_refuse_unknown_arguments_with_a_did_you_mean() {
    for spec in specs() {
        assert_eq!(
            spec["inputSchema"]["additionalProperties"], false,
            "{}",
            spec["name"]
        );
    }
    let all = specs();
    let names: Vec<&str> = all.iter().map(|s| s["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "map",
            "symbols",
            "callers",
            "callees",
            "impact",
            "read",
            "slice",
            "changes",
            "affected_tests"
        ]
    );
    let t = Target {
        root: PathBuf::from("/nonexistent"),
        index_dir: None,
        vault: None,
    };
    let r = call("callers", &json!({ "name": "x", "qualifer": "y" }), &t);
    assert_eq!(r["isError"], true);
    assert_eq!(
        r["content"][0]["text"],
        "callers: unknown argument \"qualifer\"; did you mean \"qualifier\"?"
    );
    assert_eq!(
        text(&r, 1),
        json!({ "error": { "code": "INVALID_ARGUMENT", "next": "retry", "didYouMean": ["qualifier"] } })
    );
    let r = call("callers", &json!({ "name": "x", "precision": "some" }), &t);
    assert_eq!(
        r["content"][0]["text"],
        "callers: `precision` must be one of all, resolved."
    );
    let r = call("impact", &json!({ "name": "x", "depth": 9 }), &t);
    assert_eq!(
        r["content"][0]["text"],
        "impact: `depth` must be a whole number from 1 to 4."
    );
    let r = call("callers", &json!({}), &t);
    assert_eq!(text(&r, 1)["error"]["code"], "INVALID_ARGUMENT");
    let r = call("nope", &json!({}), &t);
    assert_eq!(
        text(&r, 1)["error"],
        json!({ "code": "UNKNOWN_TOOL", "next": "tools/list" })
    );
    let r = call("callers", &json!({ "name": "x", "max_bytes": 5000 }), &t);
    assert_eq!(text(&r, 1)["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn map_symbols_callees_impact_and_read_answer_on_the_fixture() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let m = text(&run(&t, "map", json!({})), 0);
    assert_eq!(
        m["entries"],
        json!([
            { "name": "py/", "files": 1, "defs": 1 },
            { "name": "src/", "files": 4, "defs": 10 },
            { "name": "ui/", "files": 2, "defs": 2 },
        ])
    );
    let f = text(&run(&t, "map", json!({ "path": "src/records.rs" })), 0);
    assert_eq!(
        f["defs"][1],
        json!({ "name": "helper", "kind": "fn", "lines": [2, 2] })
    );
    let s = text(&run(&t, "symbols", json!({ "query": "decision rows" })), 0);
    assert_eq!(s["symbols"][0]["name"], "decisionRows");
    let s = text(&run(&t, "symbols", json!({ "query": "decisoins" })), 0);
    assert_eq!(s["didYouMean"], json!(["decisions"]));
    let c = text(&run(&t, "callees", json!({ "name": "decisions" })), 0);
    let h = &c["files"][0]["hits"][0];
    assert_eq!(
        (h["name"].as_str(), h["how"].as_str()),
        (Some("helper"), Some("same-file"))
    );
    assert_eq!(h["to"]["lines"], json!([2, 2]));
    let i = text(
        &run(&t, "impact", json!({ "name": "decisions", "depth": 3 })),
        0,
    );
    let reached: Vec<(i64, String)> = i["files"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|f| f["hits"].as_array().unwrap().clone())
        .map(|h| {
            (
                h["depth"].as_i64().unwrap(),
                h["name"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        reached,
        vec![
            (1, "a".into()),
            (2, "b".into()),
            (2, "t".into()),
            (3, "set_flag".into())
        ]
    );
    let r = text(
        &run(
            &t,
            "read",
            json!({ "path": "src/records.rs", "symbol": "helper" }),
        ),
        0,
    );
    assert_eq!(r["text"], "2\tfn helper() -> u32 { 1 }\n");
    assert_eq!(r["lines"], json!([2, 2]));
    let r = call("read", &json!({ "path": ".git/config" }), &t);
    assert_eq!(text(&r, 1)["error"]["code"], "NOT_FOUND");
    let r = call("callees", &json!({ "name": "decisoins" }), &t);
    assert_eq!(
        text(&r, 1)["error"],
        json!({ "code": "NOT_FOUND", "next": "symbols", "didYouMean": ["decisions"] })
    );
}

#[test]
fn impact_over_its_cap_refuses_and_names_the_cheaper_call() {
    let mut files: Vec<(String, String)> = vec![("src/hub.rs".into(), "pub fn hub() {}\n".into())];
    for i in 0..(IMPACT_MAX_NODES + 5) {
        files.push((
            format!("src/c{i:03}.rs"),
            format!("pub fn c{i}() {{ crate::hub::hub(); }}\n"),
        ));
    }
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (repo, cache) = (repo(&refs), tempfile::tempdir().unwrap());
    let r = call("impact", &json!({ "name": "hub" }), &target(&repo, &cache));
    assert_eq!(r["isError"], true);
    let says = r["content"][0]["text"].as_str().unwrap();
    assert!(says.contains("callers {name:\"hub\"} (205)"), "{says}");
    assert_eq!(
        text(&r, 1)["error"],
        json!({ "code": "INVALID_ARGUMENT", "next": "callers" })
    );
}

// ── Review fixes (PR #1592) ─────────────────────────────────────────────

/// `git <args>` in `dir` for a fixture (no signing, no hooks, stdin closed).
fn git_in(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// Must-fix 1: the repository's own `core.fsmonitor` program never runs
/// (git is hardened like repo_diff's), through map, callers, the dirty
/// count and the attach root.
#[test]
fn hardened_git_never_runs_the_repos_fsmonitor_program() {
    let repo = repo(&[("src/lib.rs", "pub fn a() {}\npub fn b() { a(); }\n")]);
    git_in(repo.path(), &["add", "-A"]);
    git_in(repo.path(), &["commit", "-qm", "x"]);
    let outside = tempfile::tempdir().unwrap();
    let marker = outside.path().join("MARK");
    let script = outside.path().join("fsmonitor.sh");
    std::fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\nexit 1\n", marker.display()),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    git_in(
        repo.path(),
        &["config", "core.fsmonitor", script.to_str().unwrap()],
    );
    // The hook is live for a plain git (the test would prove nothing else).
    Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["status", "--porcelain"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(marker.exists(), "plain git runs core.fsmonitor");
    std::fs::remove_file(&marker).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let t = target(&repo, &cache);
    run(&t, "map", json!({}));
    run(&t, "callers", json!({ "name": "a" }));
    assert!(crate::code_index::checkout_root(repo.path()).is_some());
    assert_eq!(crate::code_index::dirty_files(&t.root), 0);
    assert!(!marker.exists(), "Hollow's git ran core.fsmonitor");
    assert!(!repo.path().join(".git/index.lock").exists());
}

/// Must-fix 2: two first calls at once both get the full, identical answer
/// (one build; the second waits for it), and no temp or lock file is left.
#[test]
fn parallel_first_builds_coalesce_and_both_get_full_identical_results() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let args = json!({ "name": "decisions" });
    let answers: Vec<Value> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| scope.spawn(|| call("callers", &args, &t)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert_eq!(answers[0]["isError"], false, "{}", answers[0]);
    assert_eq!(answers[0], answers[1]);
    let v = text(&answers[0], 0);
    assert_eq!(v["totals"]["resolved"], 2, "{v}");
    let left: Vec<String> = std::fs::read_dir(cache.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".building") || n.ends_with(".lock"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

/// Must-fix 4: a tracked `src/lib.rs` whose `src` became a symlink to a
/// folder outside the checkout is never read: no outside name in map,
/// symbols or callers; the path is reported as skipped.
#[cfg(unix)]
#[test]
fn a_symlinked_parent_folder_is_never_followed_out_of_the_root() {
    let repo = repo(&[
        ("main.rs", "pub fn inside() {}\n"),
        ("src/lib.rs", "pub fn tracked() {}\n"),
    ]);
    git_in(repo.path(), &["add", "-A"]);
    git_in(repo.path(), &["commit", "-qm", "x"]);
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("lib.rs"),
        "pub fn outside_only_name() {}\npub fn caller_out() { outside_only_name(); }\n",
    )
    .unwrap();
    std::fs::remove_dir_all(repo.path().join("src")).unwrap();
    std::os::unix::fs::symlink(outside.path(), repo.path().join("src")).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let t = target(&repo, &cache);
    let map = text(&run(&t, "map", json!({})), 0);
    let names: Vec<&str> = map["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["main.rs"]);
    assert_eq!(map["repo"]["skippedOutsideRoot"], json!(["src/lib.rs"]));
    let symbols = run(&t, "symbols", json!({ "query": "outside_only_name" }));
    assert_eq!(text(&symbols, 0)["symbols"], json!([]));
    let callers = text(
        &run(&t, "callers", json!({ "name": "outside_only_name" })),
        0,
    );
    assert_eq!(callers["totals"]["nameScanHits"], 0);
    assert_eq!(callers["definitionsTotal"], 0);
    assert!(!callers.to_string().contains("caller_out"));
}

/// Must-fix 5: 200 same-named functions: `definitions` shows the first 20
/// (with the total and a hint), every page stays under its maxBytes, and
/// paging returns every hit once.
#[test]
fn definitions_are_capped_so_every_callers_page_holds_its_budget() {
    let mut files: Vec<(String, String)> = Vec::new();
    for i in 0..200 {
        files.push((format!("src/d{i:03}.rs"), "pub fn dup() {}\n".into()));
    }
    let body: String = (0..300).map(|_| "    dup();\n").collect();
    files.push(("src/user.rs".into(), format!("pub fn u() {{\n{body}}}\n")));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (repo, cache) = (repo(&refs), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let max = env::MIN_MAX_BYTES;
    let mut args = json!({ "name": "dup", "maxBytes": max });
    let mut seen: Vec<(String, i64)> = Vec::new();
    let mut pages = 0;
    loop {
        let r = run(&t, "callers", args.clone());
        let body = r["content"][0]["text"].as_str().unwrap();
        assert!(body.len() <= max, "page {pages}: {} bytes", body.len());
        let v = text(&r, 0);
        assert_eq!(v["definitionsTotal"], 200);
        assert_eq!(
            v["definitions"].as_array().unwrap().len(),
            DEFINITIONS_SHOWN
        );
        assert_eq!(v["definitions"][0]["path"], "src/d000.rs");
        assert!(v["definitionsNote"].as_str().unwrap().contains("qualifier"));
        seen.extend(hits(&v).into_iter().map(|h| (h.0, h.1)));
        pages += 1;
        match text(&r, 1)["envelope"].get("nextCursor") {
            Some(c) => args["cursor"] = c.clone(),
            None => break,
        }
    }
    assert!(pages > 1);
    let unique: BTreeSet<(String, i64)> = seen.iter().cloned().collect();
    assert_eq!((seen.len(), unique.len()), (300, 300));
    // callees and impact cap the same way.
    let callees = text(&run(&t, "callees", json!({ "name": "dup" })), 0);
    assert_eq!(
        callees["definitions"].as_array().unwrap().len(),
        DEFINITIONS_SHOWN
    );
    let impact = text(&run(&t, "impact", json!({ "name": "dup" })), 0);
    assert_eq!(impact["definitionsTotal"], 200);
}

/// Must-fix 6: a cursor binds to the index it paged; after an edit (and
/// so a re-index) page 2 is refused with CURSOR_INVALID, not re-sliced.
#[test]
fn a_cursor_is_refused_after_the_index_changes() {
    let mut files: Vec<(String, String)> = vec![("src/hot.rs".into(), "pub fn hot() {}\n".into())];
    let body: String = (0..200).map(|_| "    crate::hot::hot();\n").collect();
    files.push(("src/m.rs".into(), format!("pub fn f() {{\n{body}}}\n")));
    let refs: Vec<(&str, &str)> = files
        .iter()
        .map(|(p, t)| (p.as_str(), t.as_str()))
        .collect();
    let (repo, cache) = (repo(&refs), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let args = json!({ "name": "hot", "maxBytes": env::MIN_MAX_BYTES });
    let first = run(&t, "callers", args.clone());
    let cursor = text(&first, 1)["envelope"]["nextCursor"].clone();
    assert!(cursor.is_string(), "{first}");
    let mut next = args.clone();
    next["cursor"] = cursor;
    // Unchanged: the cursor pages on.
    assert_eq!(call("callers", &next, &t)["isError"], false);
    std::fs::write(
        repo.path().join("src/m.rs"),
        format!("pub fn f() {{\n    crate::hot::hot();\n{body}}}\n"),
    )
    .unwrap();
    let r = call("callers", &next, &t);
    assert_eq!(r["isError"], true, "{r}");
    let why = r["content"][0]["text"].as_str().unwrap();
    assert!(why.contains("code index changed"), "{why}");
    assert_eq!(text(&r, 1)["error"]["code"], "CURSOR_INVALID");
}

/// Should-fix a: one line over the budget comes back cut and flagged, and
/// `more` moves on; read stays near 16 KB; `maxBytes: null` is 16384.
#[test]
fn read_cuts_an_over_budget_line_and_moves_on() {
    let long = "x".repeat(40_000);
    let src = format!("pub const A: &str = \"{long}\";\npub fn b() {{}}\n");
    let (repo, cache) = (
        repo(&[("src/gen.rs", src.as_str())]),
        tempfile::tempdir().unwrap(),
    );
    let t = target(&repo, &cache);
    let r = run(&t, "read", json!({ "path": "src/gen.rs" }));
    let raw = r["content"][0]["text"].as_str().unwrap();
    assert!(raw.len() < DEFAULT_MAX_BYTES + 2_048, "{} bytes", raw.len());
    assert!(raw.len() < env::HARD_MAX_BYTES);
    let v = text(&r, 0);
    assert_eq!(v["truncatedLine"], 1);
    assert_eq!(v["lines"], json!([1, 1]));
    assert!(v["text"].as_str().unwrap().starts_with("1\tpub const A"));
    assert!(v["more"].as_str().unwrap().contains("fromLine 2"), "{v}");
    let rest = text(
        &run(&t, "read", json!({ "path": "src/gen.rs", "fromLine": 2 })),
        0,
    );
    assert_eq!(rest["text"], "2\tpub fn b() {}\n");
    assert!(rest.get("more").is_none());
    // An explicit null budget is this server's default, not OB1's.
    let mut big = vec![("src/hot.rs".to_string(), "pub fn hot() {}\n".to_string())];
    let body: String = (0..400).map(|_| "    crate::hot::hot();\n").collect();
    big.push(("src/m.rs".into(), format!("pub fn f() {{\n{body}}}\n")));
    let refs: Vec<(&str, &str)> = big.iter().map(|(p, t)| (p.as_str(), t.as_str())).collect();
    let (repo2, cache2) = (super::tests::repo(&refs), tempfile::tempdir().unwrap());
    let r = run(
        &target(&repo2, &cache2),
        "callers",
        json!({ "name": "hot", "maxBytes": null }),
    );
    assert_eq!(text(&r, 1)["envelope"]["maxBytes"], DEFAULT_MAX_BYTES);
}

/// Should-fix b and c: a root that holds the vault, sits in it, or is the
/// home folder is refused by every transport (the CLI's target too).
#[test]
fn a_root_holding_the_vault_or_the_home_folder_is_refused() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let vault = repo.path().join("vault");
    std::fs::create_dir_all(&vault).unwrap();
    let mut t = target(&repo, &cache);
    t.vault = Some(vault.clone());
    let r = call("map", &json!({}), &t);
    assert_eq!(r["isError"], true);
    assert!(r["content"][0]["text"].as_str().unwrap().contains("vault"));
    assert_eq!(text(&r, 1)["error"]["code"], "REFUSED");
    assert!(root_refusal(&t.root, Some(&t.root.join("vault"))).is_some());
    assert!(root_refusal(&t.root.join("src"), Some(&t.root)).is_some());
    assert!(root_refusal(&t.root, None).is_none());
    if let Some(home) = dirs::home_dir() {
        assert!(root_refusal(&home, None).is_some());
    }
}

/// CC3 review: the freshness check reads only through the confined reader,
/// so a path an index file names never steers a read: one climbing out of
/// the root, an absolute one, a symlink out and a FIFO each read nothing
/// (stale, at once), though the file they reach holds the indexed bytes.
#[test]
fn the_stale_check_never_reads_outside_the_root() {
    let body = "pub fn a() {}\n";
    let repo = repo(&[("src/a.rs", body)]);
    let outside = tempfile::tempdir().unwrap();
    let out = std::fs::canonicalize(outside.path()).unwrap().join("o.rs");
    std::fs::write(&out, body).unwrap();
    let root = std::fs::canonicalize(repo.path()).unwrap();
    std::os::unix::fs::symlink(&out, root.join("src/link.rs")).unwrap();
    let fifo = std::ffi::CString::new(root.join("src/fifo.rs").to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    let up = "../".repeat(root.components().count() - 1);
    let climb = format!("{up}{}", out.to_str().unwrap().trim_start_matches('/'));
    assert_eq!(std::fs::read_to_string(root.join(&climb)).unwrap(), body);
    let sha = code_index::short_sha(body.as_bytes());
    let fresh = Freshness::new(&root);
    assert!(!fresh.stale("src/a.rs", &sha));
    for path in [
        climb.as_str(),
        out.to_str().unwrap(),
        "src/link.rs",
        "src/fifo.rs",
    ] {
        assert!(fresh.stale(path, &sha), "{path} was read");
    }
}
