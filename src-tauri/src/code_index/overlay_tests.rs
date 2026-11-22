//! CC3: base plus overlay. The overlay sees an uncommitted edit, an
//! untracked file and a deletion; overlay definitions win and base edges
//! re-resolve; it answers exactly as a full index of the same work tree
//! does; one base serves two worktrees; a kept base stands in for a missing
//! one; build-base is byte-stable and import-base refuses what it can't
//! read; the cursor binds to base plus overlay; no commit falls back.

use super::*;
use crate::code_context::{call, Target};
use serde_json::Value;
use std::process::{Command, Stdio};

/// `git <args>` in `dir` (no signing, no hooks, stdin closed): stdout.
fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repo holding `files`, committed.
fn committed(files: &[(&str, &str)]) -> tempfile::TempDir {
    let repo = crate::code_context::tests::repo(files);
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "base"]);
    repo
}

fn target(root: &Path, cache: &Path) -> Target {
    Target {
        root: std::fs::canonicalize(root).unwrap(),
        index_dir: Some(cache.to_path_buf()),
        vault: None,
    }
}

fn run(t: &Target, tool: &str, args: Value) -> Value {
    let r = call(tool, &args, t);
    assert_eq!(r["isError"], false, "{tool}: {r}");
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

/// Files in `cache` whose names start with `prefix` and end with `suffix`.
fn named(cache: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(cache)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(prefix) && n.ends_with(suffix))
        .collect();
    out.sort();
    out
}

/// `(path, line, resolved, how)` of every callers hit.
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

fn h(path: &str, line: i64, resolved: bool, how: &str) -> (String, i64, bool, String) {
    (path.into(), line, resolved, how.into())
}

fn fixture() -> tempfile::TempDir {
    committed(&[
        (
            "src/a.rs",
            "pub fn alpha() -> u32 { beta() }\npub fn beta() -> u32 { 1 }\npub fn moving() {}\n",
        ),
        (
            "src/b.rs",
            "pub fn gamma() { crate::a::alpha(); }\npub fn delta() {}\npub fn calls_moving() { moving(); }\n",
        ),
        ("src/c.rs", "pub fn only_c() { crate::b::delta(); }\n"),
        ("ui/x.ts", "export function ex() { return 1; }\n"),
    ])
}

/// Done-when 2: an uncommitted edit, an untracked new file and a deletion
/// (a tombstone) all show, over one base file; `repo` names the base and
/// the overlay's file count.
#[test]
fn overlay_sees_an_edit_an_untracked_file_and_a_deletion() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(repo.path(), cache.path());
    // Before any change: the base answers, the overlay is empty.
    let v = run(&t, "callers", json!({ "name": "alpha" }));
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    assert_eq!(v["repo"]["base"], json!(head[..9]));
    assert_eq!(v["repo"]["overlayFiles"], 0);
    assert_eq!(hits(&v), vec![h("src/b.rs", 1, true, "qualifier")]);

    std::fs::write(
        repo.path().join("src/a.rs"),
        "pub fn alpha() -> u32 { beta() }\npub fn beta() -> u32 { 1 }\npub fn moving() {}\npub fn added_here() {}\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/new.rs"),
        "pub fn fresh() { crate::a::alpha(); }\n",
    )
    .unwrap();
    std::fs::remove_file(repo.path().join("src/c.rs")).unwrap();

    let v = run(&t, "symbols", json!({ "query": "added_here" }));
    assert_eq!(v["symbols"][0]["path"], "src/a.rs", "{v}");
    let v = run(&t, "callers", json!({ "name": "alpha" }));
    assert_eq!(
        hits(&v),
        vec![
            h("src/b.rs", 1, true, "qualifier"),
            h("src/new.rs", 1, true, "qualifier")
        ]
    );
    assert_eq!(v["repo"]["overlayFiles"], 3, "{v}");
    let v = run(&t, "symbols", json!({ "query": "only_c" }));
    assert_eq!(v["symbols"], json!([]), "the deleted file is a tombstone");
    let v = run(&t, "callers", json!({ "name": "delta" }));
    assert_eq!(v["totals"]["nameScanHits"], 0, "{v}");
    assert_eq!(named(cache.path(), "base-", ".sqlite").len(), 1);
    assert_eq!(named(cache.path(), "", ".overlay.sqlite").len(), 1);
    assert!(
        named(cache.path(), "", ".sqlite")
            .iter()
            .all(|n| n.starts_with("base-") || n.ends_with(".overlay.sqlite")),
        "no full index was built"
    );
}

/// A readable digest of an index's answers for `names`, with node ids
/// replaced by what they name, so two indexes compare.
fn digest(ix: &store::Index, names: &[&str]) -> Vec<String> {
    let at = |id: Option<i64>| {
        id.and_then(|id| ix.node(id).unwrap())
            .map_or("-".to_string(), |n| format!("{}:{}", n.path, n.l0))
    };
    let mut out = Vec::new();
    for f in ix.files_under("").unwrap() {
        out.push(format!("file {} {} {}", f.path, f.sha, f.defs));
    }
    let mut all = ix.def_names().unwrap();
    all.sort();
    out.push(format!("names {all:?}"));
    for name in names {
        for d in ix.defs_named(name).unwrap() {
            out.push(format!(
                "def {name} {}:{}-{} {} {}",
                d.path, d.l0, d.l1, d.kind, d.container
            ));
            for e in ix.edges_to(d.id).unwrap() {
                out.push(format!("  to {}:{}", e.path, e.line));
            }
            for e in ix.edges_from(d.id).unwrap() {
                out.push(format!("  from {} {}", e.name, at(e.target)));
            }
        }
        for e in ix.edges_named(name).unwrap() {
            let src = e.src.as_ref().map_or("-".into(), |s| s.name.clone());
            out.push(format!(
                "edge {name} {}:{} {} {} in {src} -> {}",
                e.path,
                e.line,
                e.kind,
                e.how,
                at(e.target)
            ));
        }
        for s in ix.search(name, 5).unwrap() {
            out.push(format!("search {name} {}:{}", s.path, s.l0));
        }
    }
    out
}

/// Overlay definitions win: a definition moved to a new file and one made
/// ambiguous re-resolve the base's edges to them (a name lookup), and the
/// overlaid index answers exactly as a full index of the same work tree,
/// fingerprint included.
#[test]
fn overlay_definitions_win_and_answers_match_a_full_index() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(repo.path(), cache.path());
    let v = run(&t, "callers", json!({ "name": "moving" }));
    assert_eq!(hits(&v), vec![h("src/b.rs", 3, true, "unique-name")]);
    assert_eq!(v["definitions"][0]["path"], "src/a.rs");

    // `moving` leaves a.rs for a new file; `beta` gets a second definition.
    std::fs::write(
        repo.path().join("src/a.rs"),
        "pub fn alpha() -> u32 { beta() }\npub fn beta() -> u32 { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/moved.rs"),
        "// moved here\n\npub fn moving() {}\npub fn beta() -> u32 { 2 }\n",
    )
    .unwrap();
    let v = run(&t, "callers", json!({ "name": "moving" }));
    assert_eq!(v["definitions"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["definitions"][0]["path"], "src/moved.rs");
    assert_eq!(v["definitions"][0]["lines"], json!([3, 3]));
    assert_eq!(hits(&v), vec![h("src/b.rs", 3, true, "unique-name")]);
    let v = run(&t, "callers", json!({ "name": "beta" }));
    assert_eq!(v["definitionsTotal"], 2, "{v}");
    assert_eq!(hits(&v), vec![h("src/a.rs", 1, true, "same-file")]);

    let root = std::fs::canonicalize(repo.path()).unwrap();
    let overlaid = open(&root, Some(cache.path())).unwrap();
    assert!(overlaid.overlaid());
    let other = tempfile::tempdir().unwrap();
    let full =
        store::Index::open(&ensure_with(&root, Some(other.path()), &LIMITS).unwrap()).unwrap();
    assert!(!full.overlaid());
    let names = ["alpha", "beta", "moving", "delta", "gamma", "only_c", "ex"];
    assert_eq!(digest(&overlaid, &names), digest(&full, &names));
    assert_eq!(overlaid.meta("fingerprint"), full.meta("fingerprint"));
}

/// Done-when 3: two worktrees of one repository read one base file, each
/// with its own overlay, and each sees only its own uncommitted change.
#[test]
fn two_worktrees_share_one_base_file() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let wt = tempfile::tempdir().unwrap();
    let second = wt.path().join("second");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            second.to_str().unwrap(),
        ],
    );
    std::fs::write(repo.path().join("src/one.rs"), "pub fn in_one() {}\n").unwrap();
    std::fs::write(second.join("src/two.rs"), "pub fn in_two() {}\n").unwrap();
    let (t1, t2) = (
        target(repo.path(), cache.path()),
        target(&second, cache.path()),
    );
    for (t, mine, theirs) in [(&t1, "in_one", "in_two"), (&t2, "in_two", "in_one")] {
        let v = run(t, "symbols", json!({ "query": mine }));
        assert_eq!(v["symbols"].as_array().unwrap().len(), 1, "{v}");
        let v = run(t, "symbols", json!({ "query": theirs }));
        assert_eq!(v["symbols"], json!([]));
        assert_eq!(v["repo"]["overlayFiles"], 1);
    }
    assert_eq!(named(cache.path(), "base-", ".sqlite").len(), 1);
    assert_eq!(named(cache.path(), "", ".overlay.sqlite").len(), 2);
}

/// A fresh commit's base is missing, but a kept base whose diff from the
/// work tree is small stands in for it: no second base is built.
#[test]
fn a_kept_base_stands_in_for_a_missing_one() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(repo.path(), cache.path());
    let first = git(repo.path(), &["rev-parse", "HEAD"]);
    run(&t, "map", json!({}));
    std::fs::write(repo.path().join("src/d.rs"), "pub fn later() {}\n").unwrap();
    git(repo.path(), &["add", "-A"]);
    git(repo.path(), &["commit", "-qm", "next"]);
    let v = run(&t, "symbols", json!({ "query": "later" }));
    assert_eq!(v["symbols"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["repo"]["base"], json!(first[..9]));
    assert_eq!(v["repo"]["overlayFiles"], 1);
    assert_eq!(named(cache.path(), "base-", ".sqlite").len(), 1);
}

/// The cursor binds to base plus overlay: an uncommitted edit changes the
/// fingerprint, so page 2 of the old answer is refused.
#[test]
fn a_cursor_is_refused_after_the_overlay_changes() {
    let body: String = (0..200).map(|_| "    crate::hot::hot();\n").collect();
    let repo = committed(&[
        ("src/hot.rs", "pub fn hot() {}\n"),
        ("src/m.rs", &format!("pub fn f() {{\n{body}}}\n")),
    ]);
    let cache = tempfile::tempdir().unwrap();
    let t = target(repo.path(), cache.path());
    let args = json!({ "name": "hot", "maxBytes": crate::mcp_envelope::MIN_MAX_BYTES });
    let first = call("callers", &args, &t);
    let page: Value = serde_json::from_str(first["content"][1]["text"].as_str().unwrap()).unwrap();
    let cursor = page["envelope"]["nextCursor"].clone();
    assert!(cursor.is_string(), "{first}");
    let before = run(&t, "map", json!({}))["repo"]["fingerprint"].clone();
    let mut next = args.clone();
    next["cursor"] = cursor;
    assert_eq!(call("callers", &next, &t)["isError"], false);
    std::fs::write(
        repo.path().join("src/m.rs"),
        format!("pub fn f() {{\n    crate::hot::hot();\n{body}}}\n"),
    )
    .unwrap();
    let v = run(&t, "map", json!({}));
    assert_ne!(v["repo"]["fingerprint"], before);
    assert_eq!(v["repo"]["overlayFiles"], 1);
    let r = call("callers", &next, &t);
    assert_eq!(r["isError"], true, "{r}");
    assert!(r["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("code index changed"));
}

/// No commit, no base: CC2's full index answers, and `repo` names no base.
#[test]
fn a_checkout_without_a_commit_falls_back_to_the_full_index() {
    let repo = crate::code_context::tests::repo(&[("src/a.rs", "pub fn a() {}\n")]);
    let cache = tempfile::tempdir().unwrap();
    let t = target(repo.path(), cache.path());
    let v = run(&t, "symbols", json!({ "query": "a" }));
    assert_eq!(v["symbols"][0]["path"], "src/a.rs");
    assert!(v["repo"].get("base").is_none(), "{v}");
    assert!(v["repo"].get("overlayFiles").is_none());
    assert!(named(cache.path(), "base-", ".sqlite").is_empty());
    assert_eq!(named(cache.path(), "", ".sqlite").len(), 1);
}

/// build-base: a pure function of the commit (twice, the same bytes, even
/// with the work tree edited in between) with a receipt naming it.
#[test]
fn build_base_is_byte_stable_for_one_commit() {
    let repo = fixture();
    let out = tempfile::tempdir().unwrap();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let build = |file: &str| {
        let args: Vec<String> = [
            "--repo",
            repo.path().to_str().unwrap(),
            "--commit",
            &head[..12],
            "--out",
            out.path().join(file).to_str().unwrap(),
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let (text, code) = cli_build_base(&args, None);
        assert_eq!(code, 0, "{text}");
        serde_json::from_str::<Value>(&text).unwrap()
    };
    let one = build("one.sqlite");
    std::fs::write(repo.path().join("src/a.rs"), "pub fn changed() {}\n").unwrap();
    let two = build("two.sqlite");
    assert_eq!(one["commit"], json!(head));
    assert_eq!(
        (one["files"].clone(), one["nodes"].clone()),
        (json!(4), json!(8))
    );
    assert_eq!(one["sha256"], two["sha256"]);
    assert_eq!(
        std::fs::read(out.path().join("one.sqlite")).unwrap(),
        std::fs::read(out.path().join("two.sqlite")).unwrap()
    );
    assert!(named(out.path(), ".", ".building").is_empty());
    let (text, code) = cli_build_base(
        &["--repo".into(), repo.path().to_string_lossy().into()],
        None,
    );
    assert_eq!(code, 2, "{text}");
    let bad: Vec<String> = ["--repo", repo.path().to_str().unwrap(), "--commit", "--out"]
        .iter()
        .map(|s| s.to_string())
        .chain([
            "--out".into(),
            out.path().join("x").to_string_lossy().into(),
        ])
        .collect();
    let (text, code) = cli_build_base(&bad, None);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("is not a commit"), "{text}");
}

/// import-base takes a good base in (and the overlay path then reads it
/// without building one) and refuses a different schema, other parsers,
/// a commit this repository lacks, or a file that is not a base.
#[test]
fn import_base_checks_schema_parsers_and_commit() {
    let repo = fixture();
    let root = std::fs::canonicalize(repo.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let good = out.path().join("good.sqlite");
    build_base_cli(repo.path(), &head, &good, None).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let edited = |name: &str, key: &str, value: &str| {
        let file = out.path().join(name);
        std::fs::copy(&good, &file).unwrap();
        let conn = rusqlite::Connection::open(&file).unwrap();
        conn.execute("UPDATE meta SET value = ?1 WHERE key = ?2", [value, key])
            .unwrap();
        file
    };
    let cases = [
        (edited("schema.sqlite", "schema", "999"), "schema `999`"),
        (edited("parsers.sqlite", "parsers", "old"), "parsers `old`"),
        (
            edited("commit.sqlite", "base", &"0".repeat(40)),
            "is not in the repository",
        ),
        (edited("kind.sqlite", "kind", "overlay"), "not a base index"),
    ];
    for (file, why) in cases {
        let (text, code) = cli_import_base(
            &[file.to_string_lossy().into_owned()],
            &root,
            Some(cache.path()),
            None,
        );
        assert_eq!(code, 1, "{text}");
        assert!(text.contains(why), "{text}");
        assert!(text.contains("nothing was imported"));
        assert!(std::fs::read_dir(cache.path()).unwrap().next().is_none());
    }
    let (text, code) = cli_import_base(
        &[good.to_string_lossy().into_owned()],
        &root,
        Some(cache.path()),
        None,
    );
    assert_eq!(code, 0, "{text}");
    let receipt: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(receipt["commit"], json!(head));
    assert_eq!(named(cache.path(), "base-", ".sqlite").len(), 1);
    // The overlay path reads the imported base as is: the same bytes after.
    let imported = PathBuf::from(receipt["imported"].as_str().unwrap());
    let before = std::fs::read(&imported).unwrap();
    let t = target(repo.path(), cache.path());
    let v = run(&t, "callers", json!({ "name": "alpha" }));
    assert_eq!(v["repo"]["base"], json!(head[..9]));
    assert_eq!(std::fs::read(&imported).unwrap(), before);
    assert_eq!(cli_import_base(&[], &root, None, None).1, 2);
}

/// Generations: the base in use and the BASE_KEEP - 1 most recently used
/// others stay; older ones go, except one used in the last 30 minutes.
#[test]
fn old_base_generations_are_pruned() {
    let dir = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let mut paths = Vec::new();
    for i in 0..5u64 {
        let p = base_path(dir.path(), "k", &format!("{i:040}"));
        std::fs::write(&p, "x").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(now - Duration::from_secs(3600 * (5 - i)))
            .unwrap();
        paths.push(p);
    }
    std::fs::write(base_path(dir.path(), "other", &"9".repeat(40)), "x").unwrap();
    prune_bases(dir.path(), "k", &paths[0]);
    let left: Vec<bool> = paths.iter().map(|p| p.exists()).collect();
    assert_eq!(left, vec![true, false, false, true, true]);
    assert!(base_path(dir.path(), "other", &"9".repeat(40)).exists());
    // Within the grace a base is kept even past BASE_KEEP: a fourth recent
    // one leaves all four.
    let recent = base_path(dir.path(), "k", &"7".repeat(40));
    std::fs::write(&recent, "x").unwrap();
    for (p, ago) in [(&paths[3], 60), (&paths[4], 120)] {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(now - Duration::from_secs(ago))
            .unwrap();
    }
    prune_bases(dir.path(), "k", &paths[0]);
    assert!(paths[3].exists() && paths[4].exists() && recent.exists());
}

/// CC3 review: import-base trusts nothing in the file. A path outside the
/// checkout, an absolute or hidden path, a file the commit lacks, a view (a
/// recursive one would hang a reader), a trigger and a huge definition id
/// are each refused with nothing imported; a FIFO and a symlink are refused
/// without blocking; a root that holds the vault is refused; a read-only
/// (0444) good file is taken in as 0644.
#[test]
fn import_base_refuses_a_crafted_file() {
    let repo = fixture();
    let root = std::fs::canonicalize(repo.path()).unwrap();
    let out = tempfile::tempdir().unwrap();
    let head = git(repo.path(), &["rev-parse", "HEAD"]);
    let good = out.path().join("good.sqlite");
    build_base_cli(repo.path(), &head, &good, None).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let crafted = |name: &str, sql: &str| {
        let file = out.path().join(name);
        std::fs::copy(&good, &file).unwrap();
        rusqlite::Connection::open(&file)
            .unwrap()
            .execute_batch(sql)
            .unwrap();
        file
    };
    let moved = |to: &str| format!("UPDATE files SET path = '{to}' WHERE path = 'src/c.rs'");
    let fifo = out.path().join("fifo.sqlite");
    let c = std::ffi::CString::new(fifo.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let link = out.path().join("link.sqlite");
    std::os::unix::fs::symlink(&good, &link).unwrap();
    let cases = [
        (crafted("up.sqlite", &moved("../../../../dev/zero.rs")), "would never index"),
        (crafted("abs.sqlite", &moved("/etc/zero.rs")), "would never index"),
        (crafted("ssh.sqlite", &moved("src/.ssh/id.rs")), "would never index"),
        (crafted("extra.sqlite", &moved("src/zz.rs")), "not commit"),
        (
            crafted(
                "view.sqlite",
                "CREATE VIEW spin AS WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) SELECT x FROM c",
            ),
            "view `spin`",
        ),
        (
            crafted(
                "trigger.sqlite",
                "CREATE TRIGGER t AFTER INSERT ON meta BEGIN DELETE FROM files; END",
            ),
            "trigger `t`",
        ),
        (
            crafted(
                "huge.sqlite",
                "UPDATE nodes SET id = 9223372036854775807 WHERE id = (SELECT max(id) FROM nodes)",
            ),
            "definition ids",
        ),
        (fifo, "not a regular file"),
        (link, "could not read"),
    ];
    for (file, why) in cases {
        let (text, code) = cli_import_base(
            &[file.to_string_lossy().into_owned()],
            &root,
            Some(cache.path()),
            None,
        );
        assert_eq!(code, 1, "{text}");
        assert!(text.contains(why), "{why}: {text}");
        assert!(text.contains("nothing was imported"));
        assert!(std::fs::read_dir(cache.path()).unwrap().next().is_none());
    }
    let arg = [good.to_string_lossy().into_owned()];
    let (text, code) = cli_import_base(&arg, &root, Some(cache.path()), Some(&root));
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("vault"), "{text}");
    let build = [
        "--repo".to_string(),
        root.to_string_lossy().into(),
        "--commit".into(),
        head.clone(),
        "--out".into(),
        out.path().join("v.sqlite").to_string_lossy().into(),
    ];
    let (text, code) = cli_build_base(&build, Some(&root));
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("vault"), "{text}");
    use std::os::unix::fs::PermissionsExt;
    let ro = out.path().join("ro.sqlite");
    std::fs::copy(&good, &ro).unwrap();
    std::fs::set_permissions(&ro, std::fs::Permissions::from_mode(0o444)).unwrap();
    let (text, code) = cli_import_base(
        &[ro.to_string_lossy().into_owned()],
        &root,
        Some(cache.path()),
        None,
    );
    assert_eq!(code, 0, "{text}");
    let receipt: Value = serde_json::from_str(&text).unwrap();
    let imported = PathBuf::from(receipt["imported"].as_str().unwrap());
    let mode = std::fs::metadata(&imported).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o644);
}

/// CC3 review: an overlay is bound to the base file it was built on. A
/// base replaced under it (here: one more definition, so other offsets) is
/// refused on attach, and the next call rebuilds the overlay on it.
#[test]
fn an_overlay_is_rebuilt_when_its_base_changes_under_it() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(repo.path(), cache.path());
    std::fs::write(repo.path().join("src/d.rs"), "pub fn later() {}\n").unwrap();
    let v = run(&t, "symbols", json!({ "query": "later" }));
    assert_eq!(v["symbols"].as_array().unwrap().len(), 1, "{v}");
    let base = cache
        .path()
        .join(&named(cache.path(), "base-", ".sqlite")[0]);
    let overlay = cache
        .path()
        .join(&named(cache.path(), "", ".overlay.sqlite")[0]);
    assert!(store::Index::open_overlay(&base, &overlay).is_ok());
    rusqlite::Connection::open(&base)
        .unwrap()
        .execute_batch(
            "INSERT INTO nodes(id, file_id, kind, name_id, container, modpath, l0, l1, is_test, command)
             SELECT id + 1, file_id, kind, name_id, container, modpath, l0, l1, is_test, command
             FROM nodes WHERE id = (SELECT max(id) FROM nodes)",
        )
        .unwrap();
    let err = store::Index::open_overlay(&base, &overlay).err();
    assert_eq!(err.as_deref(), Some(store::OVERLAY_MISMATCH));
    let v = run(&t, "symbols", json!({ "query": "later" }));
    assert_eq!(v["symbols"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(v["repo"]["overlayFiles"], 1);
    let identity = store::Index::open(&base).unwrap().base_identity().unwrap();
    assert_eq!(store::read_meta(&overlay, "baseIdentity"), Some(identity));
    assert!(store::Index::open_overlay(&base, &overlay).is_ok());
}

/// CC3 review: in-place edits to assume-unchanged and skip-worktree files,
/// which `git diff` does not report, still reach the overlay.
#[test]
fn assume_unchanged_and_skip_worktree_edits_are_seen() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(repo.path(), cache.path());
    run(&t, "map", json!({}));
    git(
        repo.path(),
        &["update-index", "--assume-unchanged", "src/c.rs"],
    );
    git(
        repo.path(),
        &["update-index", "--skip-worktree", "src/b.rs"],
    );
    std::fs::write(
        repo.path().join("src/c.rs"),
        "pub fn only_c() {}\npub fn hid_edit() {}\n",
    )
    .unwrap();
    std::fs::write(
        repo.path().join("src/b.rs"),
        "pub fn gamma() { crate::a::alpha(); }\npub fn delta() {}\npub fn calls_moving() { moving(); }\npub fn skip_edit() {}\n",
    )
    .unwrap();
    for q in ["hid_edit", "skip_edit"] {
        let v = run(&t, "symbols", json!({ "query": q }));
        assert_eq!(v["symbols"].as_array().unwrap().len(), 1, "{q}: {v}");
        assert_eq!(v["repo"]["overlayFiles"], 2);
    }
}

/// The measurement in the CC3 PR and build log (not part of the suite):
/// `HOLLOW_CC3_MEASURE=<a hollow checkout> cargo test --lib
/// code_index::overlay::tests::measure -- --ignored --nocapture`. A shared
/// clone of it gets five worktrees, each with 20 code files changed; prints
/// the base build (through build-base and import-base), each overlay, and
/// one full index, with sizes.
#[test]
#[ignore = "measurement, run by hand: base and overlay times and sizes on this repo (docs/CODE_CONTEXT.md)"]
fn measure() {
    let Ok(src) = std::env::var("HOLLOW_CC3_MEASURE") else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let clone = tmp.path().join("clone");
    git(
        tmp.path(),
        &["clone", "-q", "--shared", "--no-checkout", &src, "clone"],
    );
    let head = git(&clone, &["rev-parse", "HEAD"]);
    let cache = tmp.path().join("cache");
    let size = |p: &Path| std::fs::metadata(p).map_or(0, |m| m.len());
    let started = Instant::now(); // sleep-ok: measures build time, asserts nothing on it
    let base_file = tmp.path().join("base.sqlite");
    let receipt = build_base_cli(&clone, &head, &base_file, None).unwrap();
    println!(
        "build-base: {:.1}s, {} bytes, {} files, {} nodes, {} edges",
        started.elapsed().as_secs_f64(),
        size(&base_file),
        receipt["files"],
        receipt["nodes"],
        receipt["edges"]
    );
    let mut overlay_bytes = 0u64;
    let mut roots = Vec::new();
    for w in 0..5 {
        let wt = tmp.path().join(format!("wt{w}"));
        git(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                wt.to_str().unwrap(),
                &head,
            ],
        );
        let wt = std::fs::canonicalize(&wt).unwrap();
        if w == 0 {
            let r = import_base(&base_file, &wt, Some(&cache), None).unwrap();
            println!("import-base: {}", r["imported"]);
        }
        let files = git(&wt, &["ls-files", "src-tauri/src/*.rs"]);
        for (i, path) in files.lines().skip(w * 40).step_by(2).take(20).enumerate() {
            let full = wt.join(path);
            let mut text = std::fs::read_to_string(&full).unwrap();
            text.push_str(&format!("\npub fn cc3_measure_{w}_{i}() {{}}\n"));
            std::fs::write(&full, text).unwrap();
        }
        let started = Instant::now(); // sleep-ok: measures build time, asserts nothing on it
        let index = open(&wt, Some(&cache)).unwrap();
        let took = started.elapsed().as_secs_f64();
        assert!(index.overlaid());
        let started = Instant::now(); // sleep-ok: measures query time, asserts nothing on it
        let n = index.defs_named("ensure").unwrap().len();
        let q = started.elapsed().as_secs_f64();
        let key = short_sha(wt.to_string_lossy().as_bytes());
        let ov = cache.join(format!("{key}.overlay.sqlite"));
        overlay_bytes += size(&ov);
        println!(
            "worktree {w}: overlay {took:.2}s, {} bytes, overlayFiles {}, a query {q:.3}s ({n} defs)",
            size(&ov),
            index.meta("overlayFiles").unwrap_or_default()
        );
        roots.push(wt);
    }
    let full_cache = tmp.path().join("full");
    let started = Instant::now(); // sleep-ok: measures build time, asserts nothing on it
    let full = ensure_with(&roots[0], Some(&full_cache), &LIMITS).unwrap();
    println!(
        "full index of worktree 0: {:.1}s, {} bytes",
        started.elapsed().as_secs_f64(),
        size(&full)
    );
    println!(
        "5 worktrees: base + 5 overlays = {} bytes; 5 full indexes = {} bytes (5 x worktree 0's)",
        size(&base_file) + overlay_bytes,
        5 * size(&full)
    );
}
