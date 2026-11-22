//! CC4: `slice` cards and handles. A first call returns cards and a
//! handle; the same call with it and no edit is `notModified`; an edit of
//! one body (through CC3's overlay) returns only that card; a deleted
//! caller is `removed`; a shifted definition is `moved`, not resent;
//! expired, forged and foreign handles are refused; a large slice pages
//! every card once; strict arguments.

use super::*;
use crate::code_context::tests::{repo, target};
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .args(["-c", "user.name=t", "-c", "user.email=t@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

const A: &str =
    "pub fn alpha() -> u32 {\n    beta()\n}\npub fn beta() -> u32 {\n    1\n}\npub fn after() {}\n";
const B: &str =
    "pub fn gamma() {\n    crate::a::alpha();\n}\npub fn delta() {\n    crate::a::alpha();\n}\n";

/// A committed repo, so calls read CC3's base plus overlay.
fn fixture() -> tempfile::TempDir {
    let r = repo(&[
        ("src/a.rs", A),
        ("src/b.rs", B),
        ("src/c.rs", "pub fn far() { crate::b::gamma(); }\n"),
    ]);
    git(r.path(), &["add", "-A"]);
    git(r.path(), &["commit", "-qm", "base"]);
    r
}

fn first(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn ok(t: &Target, args: Value) -> Value {
    let r = call("slice", &args, t);
    assert_eq!(r["isError"], false, "{r}");
    first(&r)
}

fn refused(t: &Target, args: Value) -> (String, Value) {
    let r = call("slice", &args, t);
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap().to_string();
    let block: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    (text, block["error"].clone())
}

fn ids(v: &Value) -> Vec<String> {
    v["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn a_first_slice_returns_ordered_cards_and_a_handle() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let v = ok(&t, json!({ "symbol": "alpha" }));
    assert!(
        v["repo"]["base"].is_string(),
        "read through the overlay: {v}"
    );
    assert_eq!(
        ids(&v),
        [
            "src/a.rs:alpha",
            "src/a.rs:beta",
            "src/b.rs:gamma",
            "src/b.rs:delta"
        ]
    );
    let rels: Vec<&str> = v["cards"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["rel"].as_str().unwrap())
        .collect();
    assert_eq!(rels, ["target", "callee", "caller", "caller"]);
    let alpha = &v["cards"][0];
    assert_eq!(alpha["signature"], "pub fn alpha() -> u32");
    assert_eq!(alpha["lines"], json!([1, 3]));
    for key in ["cardHash", "bodyHash", "fileHash"] {
        assert_eq!(alpha[key].as_str().unwrap().len(), 12, "{key}");
    }
    let handle = v["handle"].as_str().unwrap();
    assert!(handle.starts_with("h1.") && handle.len() == 19, "{handle}");
    assert_eq!(v["totals"], json!({ "cards": 4, "targets": 1 }));
    // Depth 2 reaches the caller's caller; the same call gives the same
    // handle (it hashes the arguments and the cards).
    let deep = ok(&t, json!({ "symbol": "alpha", "depth": 2 }));
    assert_eq!(ids(&deep).last().unwrap(), "src/c.rs:far");
    assert_eq!(ok(&t, json!({ "symbol": "alpha" }))["handle"], v["handle"]);
    // A task's words pick the definitions they name.
    let task = ok(&t, json!({ "task": "fix the gamma path" }));
    assert_eq!(task["target"]["matched"], json!(["gamma"]));
    assert_eq!(ids(&task)[0], "src/b.rs:gamma");
}

#[test]
fn a_handle_returns_not_modified_then_only_the_edited_card() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let v = ok(&t, json!({ "symbol": "alpha" }));
    let h0 = v["handle"].as_str().unwrap().to_string();
    // Nothing changed: one small block.
    let same = call("slice", &json!({ "symbol": "alpha", "handle": h0 }), &t);
    let blocks = same["content"].as_array().unwrap();
    assert_eq!(blocks.len(), 1, "{same}");
    let text = blocks[0]["text"].as_str().unwrap();
    assert_eq!(
        text,
        json!({ "notModified": true, "handle": h0 }).to_string()
    );
    assert!(text.len() < 60, "{text}");
    // Edit beta's body (same line count): only beta comes back, though
    // alpha shares its file.
    std::fs::write(r.path().join("src/a.rs"), A.replace("    1\n", "    42\n")).unwrap();
    let d = ok(&t, json!({ "symbol": "alpha", "handle": h0 }));
    assert_eq!(d["repo"]["overlayFiles"], 1, "{d}");
    assert_eq!(ids(&d), ["src/a.rs:beta"]);
    assert_eq!(d["cards"][0]["change"], "changed");
    assert_eq!(d["since"], json!(h0));
    assert_eq!(d["moved"], json!([]));
    assert_eq!(d["removed"], json!([]));
    assert_eq!(
        d["totals"],
        json!({ "cards": 4, "added": 0, "changed": 1, "moved": 0, "removed": 0 })
    );
    let h1 = d["handle"].as_str().unwrap().to_string();
    assert_ne!(h1, h0);
    let again = first(&call(
        "slice",
        &json!({ "symbol": "alpha", "handle": h1 }),
        &t,
    ));
    assert_eq!(again, json!({ "notModified": true, "handle": h1 }));
    // A blank line above alpha moves both cards in a.rs: they are listed
    // in `moved` with their new lines, not resent.
    std::fs::write(
        r.path().join("src/a.rs"),
        format!("\n{}", A.replace("    1\n", "    42\n")),
    )
    .unwrap();
    let m = ok(&t, json!({ "symbol": "alpha", "handle": h1 }));
    assert_eq!(ids(&m), Vec::<String>::new());
    assert_eq!(
        m["moved"],
        json!([{ "id": "src/a.rs:alpha", "lines": [2, 4] }, { "id": "src/a.rs:beta", "lines": [5, 7] }])
    );
    // Deleting a caller: it is removed, nothing else is sent.
    let h2 = m["handle"].as_str().unwrap().to_string();
    std::fs::write(
        r.path().join("src/b.rs"),
        "pub fn gamma() {\n    crate::a::alpha();\n}\n",
    )
    .unwrap();
    let gone = ok(&t, json!({ "symbol": "alpha", "handle": h2 }));
    assert_eq!(ids(&gone), Vec::<String>::new());
    assert_eq!(gone["removed"], json!(["src/b.rs:delta"]));
    assert_eq!(gone["totals"]["cards"], 3);
}

#[test]
fn expired_forged_and_foreign_handles_are_refused() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let v = ok(&t, json!({ "symbol": "alpha" }));
    let h = v["handle"].as_str().unwrap().to_string();
    for forged in [
        "h1.0123456789abcdef",
        "h1.../../x",
        "c1.0.abc",
        "h2.0123456789abcdef",
    ] {
        let (text, error) = refused(&t, json!({ "symbol": "alpha", "handle": forged }));
        assert!(
            text.starts_with("slice: this handle") && text.contains("drop handle"),
            "{text}"
        );
        assert_eq!(error["code"], "CURSOR_INVALID", "{forged}");
    }
    // Another slice's handle.
    let (text, error) = refused(&t, json!({ "symbol": "beta", "handle": h }));
    assert!(text.contains("another slice"), "{text}");
    assert_eq!(error["code"], "CURSOR_INVALID");
    let (text, _) = refused(&t, json!({ "symbol": "alpha", "depth": 2, "handle": h }));
    assert!(text.contains("another slice"), "{text}");
    // Past its lease: refused, and its state is gone.
    let file = cache
        .path()
        .join("slices")
        .join(format!("{}.json", &h[3..]));
    assert!(file.is_file());
    // sleep-ok: no sleep; the file's time is set back past the lease.
    let old = SystemTime::now() - HANDLE_LEASE - Duration::from_secs(60);
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let (text, error) = refused(&t, json!({ "symbol": "alpha", "handle": h }));
    assert!(text.contains("lease"), "{text}");
    assert_eq!(error["code"], "CURSOR_INVALID");
    assert!(!file.exists());
    // Without the handle the slice starts over (and the handle works again).
    assert_eq!(ok(&t, json!({ "symbol": "alpha" }))["handle"], json!(h));
    assert_eq!(
        ok(&t, json!({ "symbol": "alpha", "handle": h }))["notModified"],
        true
    );
}

#[test]
fn a_large_slice_pages_every_card_once() {
    let callers: String = (0..90)
        .map(|i| format!("pub fn caller_number_{i:03}() {{\n    crate::hot::hot();\n}}\n"))
        .collect();
    let (r, cache) = (
        repo(&[("src/hot.rs", "pub fn hot() {}\n"), ("src/m.rs", &callers)]),
        tempfile::tempdir().unwrap(),
    );
    let t = target(&r, &cache);
    let args = json!({ "symbol": "hot", "maxBytes": env::MIN_MAX_BYTES });
    let mut all: Vec<String> = Vec::new();
    let mut next = args.clone();
    let mut pages = 0;
    loop {
        let res = call("slice", &next, &t);
        assert_eq!(res["isError"], false, "{res}");
        let text = res["content"][0]["text"].as_str().unwrap();
        assert!(
            text.len() <= env::MIN_MAX_BYTES,
            "page {pages}: {}",
            text.len()
        );
        all.extend(ids(&first(&res)));
        pages += 1;
        let envelope: Value =
            serde_json::from_str(res["content"][1]["text"].as_str().unwrap()).unwrap();
        match envelope["envelope"]["nextCursor"].as_str() {
            Some(c) => next["cursor"] = json!(c),
            None => break,
        }
    }
    assert!(pages > 2, "{pages} pages");
    assert_eq!(all.len(), 91);
    assert_eq!(all[0], "src/hot.rs:hot");
    let unique: BTreeSet<&String> = all.iter().collect();
    assert_eq!(unique.len(), 91, "every card once");
    let mut sorted = all[1..].to_vec();
    sorted.sort();
    assert_eq!(
        sorted,
        all[1..].to_vec(),
        "callers by path and line (names sort alike)"
    );
}

#[test]
fn slice_arguments_are_strict() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let (text, error) = refused(&t, json!({ "symbl": "alpha" }));
    assert!(text.contains("did you mean \"symbol\""), "{text}");
    assert_eq!(error["code"], "INVALID_ARGUMENT");
    assert_eq!(error["didYouMean"], json!(["symbol"]));
    let (text, _) = refused(&t, json!({ "symbol": "alpha", "hanlde": "x" }));
    assert!(text.contains("did you mean \"handle\""), "{text}");
    for (args, want) in [
        (json!({}), "is required"),
        (
            json!({ "symbol": "alpha", "task": "x" }),
            "must be left out",
        ),
        (
            json!({ "task": "alpha", "qualifier": "a" }),
            "must be left out",
        ),
        (
            json!({ "symbol": "alpha", "depth": 3 }),
            "must be a whole number from 1 to 2",
        ),
        (json!({ "task": "the and of" }), "needs a word"),
    ] {
        let (text, error) = refused(&t, args.clone());
        assert!(text.contains(want), "{args}: {text}");
        assert_eq!(error["code"], "INVALID_ARGUMENT", "{args}");
    }
    let (text, error) = refused(&t, json!({ "symbol": "alpah" }));
    assert!(text.contains("did you mean `alpha`"), "{text}");
    assert_eq!(error["code"], "NOT_FOUND");
}

#[test]
fn two_threads_saving_one_handle_leave_it_loadable() {
    // Review should-fix 1: one temp name per process let one save truncate
    // the file another was renaming.
    let dir = tempfile::tempdir().unwrap();
    let key = "slice\n/x\n{}";
    let cards: Vec<Card> = (0..50)
        .map(|i| Card {
            id: format!("src/a.rs:f{i}"),
            hash: format!("{i:012}"),
            l0: i,
            l1: i + 1,
            json: json!({}),
        })
        .collect();
    let handle = handle_of(key, &cards);
    let file = handle_file(dir.path(), &handle).unwrap();
    for _ in 0..40 {
        let _ = std::fs::remove_file(&file);
        let gate = std::sync::Barrier::new(2);
        std::thread::scope(|s| {
            for _ in 0..2 {
                s.spawn(|| {
                    gate.wait();
                    save(Some(dir.path()), &handle, key, &cards);
                });
            }
        });
        let state = load(Some(dir.path()), &handle, key).unwrap();
        assert_eq!(state.cards.len(), 50);
    }
    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn a_delta_with_many_removals_stays_within_max_bytes() {
    // Review should-fix 2: `moved` and `removed` were outside the budget.
    let callers: String = (0..150)
        .map(|i| format!("pub fn caller_number_{i:03}() {{\n    crate::hot::hot();\n}}\n"))
        .collect();
    let r = repo(&[("src/hot.rs", "pub fn hot() {}\n"), ("src/m.rs", &callers)]);
    git(r.path(), &["add", "-A"]);
    git(r.path(), &["commit", "-qm", "base"]);
    let cache = tempfile::tempdir().unwrap();
    let t = target(&r, &cache);
    let small = env::MIN_MAX_BYTES;
    let v = first(&call(
        "slice",
        &json!({ "symbol": "hot", "maxBytes": small }),
        &t,
    ));
    let h = v["handle"].as_str().unwrap().to_string();
    std::fs::write(r.path().join("src/m.rs"), "pub fn other() {}\n").unwrap();
    let res = call(
        "slice",
        &json!({ "symbol": "hot", "handle": h, "maxBytes": small }),
        &t,
    );
    assert_eq!(res["isError"], false, "{res}");
    let text = res["content"][0]["text"].as_str().unwrap();
    assert!(text.len() <= small, "{}", text.len());
    let d = first(&res);
    assert_eq!(d["totals"]["removed"], 150);
    let listed = d["removed"].as_array().unwrap().len();
    assert!(listed > 0 && listed < 150, "{listed}");
    assert_eq!(d["removed"][0], "src/m.rs:caller_number_000");
    let cut = d["listsCut"].as_str().unwrap();
    assert!(
        cut.contains(&format!("removed {listed} of 150")) && cut.contains("drop handle"),
        "{cut}"
    );
    // A short list is kept whole; a long one stops at DELTA_LIST_MAX.
    assert_eq!(listed_in(&[1, 2, 3], 4096), 3);
    assert_eq!(listed_in(&vec![0u8; 500], 1 << 20), DELTA_LIST_MAX);
}

fn listed_in(items: &[u8], bytes: usize) -> usize {
    listed(items, bytes)
}

#[test]
fn a_repeated_task_word_weighs_once() {
    // Review should-fix 3: `dedup` dropped only adjacent repeats.
    let (r, cache) = (
        repo(&[
            ("src/a.rs", "pub fn check_all_items() {}\n"),
            ("src/b.rs", "pub fn memory() {}\n"),
        ]),
        tempfile::tempdir().unwrap(),
    );
    let t = target(&r, &cache);
    let once = ok(&t, json!({ "task": "check memory" }));
    let twice = ok(&t, json!({ "task": "check memory check" }));
    assert_eq!(
        once["target"]["matched"],
        json!(["memory", "check_all_items"])
    );
    assert_eq!(twice["target"]["matched"], once["target"]["matched"]);
    assert_eq!(ids(&twice), ids(&once));
}

#[test]
fn equivalent_arguments_share_a_handle() {
    // Review should-fix 4: the handle hashed the raw arguments.
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let h = ok(&t, json!({ "symbol": "alpha" }))["handle"].clone();
    for same in [
        json!({ "symbol": "alpha", "depth": 1 }),
        json!({ "symbol": " alpha " }),
        json!({ "symbol": "alpha", "qualifier": "" }),
    ] {
        assert_eq!(ok(&t, same.clone())["handle"], h, "{same}");
    }
    let mut with = json!({ "symbol": "alpha", "depth": 1, "handle": h });
    assert_eq!(ok(&t, with.clone())["notModified"], true);
    with["symbol"] = json!("alpha ");
    assert_eq!(ok(&t, with)["notModified"], true);
    let langs = ok(
        &t,
        json!({ "symbol": "alpha", "languages": ["rust", "ts", "rust"] }),
    )["handle"]
        .clone();
    assert_eq!(
        ok(
            &t,
            json!({ "symbol": "alpha", "languages": ["ts", "rust"] })
        )["handle"],
        langs
    );
    assert_ne!(langs, h, "languages given is another slice than none");
}

#[test]
fn not_modified_does_not_refresh_file_hashes() {
    // Review should-fix 5, as documented: an edit elsewhere in a file
    // changes no card, so the answer is notModified and a held card's
    // fileHash is older than the file's; a call without handle has it.
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let v = ok(&t, json!({ "symbol": "alpha" }));
    let h = v["handle"].as_str().unwrap().to_string();
    let held = v["cards"][0]["fileHash"].clone();
    std::fs::write(
        r.path().join("src/a.rs"),
        A.replace("pub fn after() {}", "pub fn after() { let _ = 7; }"),
    )
    .unwrap();
    let same = call("slice", &json!({ "symbol": "alpha", "handle": h }), &t);
    assert_eq!(
        first(&same),
        json!({ "notModified": true, "handle": h }),
        "{same}"
    );
    let now = ok(&t, json!({ "symbol": "alpha" }));
    assert_eq!(now["handle"], json!(h));
    assert_ne!(now["cards"][0]["fileHash"], held);
}
