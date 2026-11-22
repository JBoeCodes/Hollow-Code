//! CC5: `changes` maps hunks to definitions (edit, add, delete, a rename
//! as a delete plus an add) and lists what it can't map; `affected_tests`
//! finds a direct Rust test, a transitive one through a `#[path]` test
//! module, a TypeScript test file through an import (one level) and a
//! Playwright spec, reports a macro-only
//! reference in `cannotMap`, and stops at `depth`; strict arguments. The
//! evaluation on merged Hollow PRs is `evaluate_on_merged_prs` (ignored).

use super::*;
use crate::code_context::tests::{repo, target};
use std::process::{Command, Stdio};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
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
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

const A: &str = "pub fn alpha() -> u32 {\n    beta()\n}\npub fn beta() -> u32 {\n    1\n}\npub fn gone() {}\npub fn after() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn alpha_is_one() {\n        assert_eq!(super::alpha(), 1);\n    }\n}\n";
const B: &str = "pub fn gamma() -> u32 {\n    crate::a::alpha() + 1\n}\n#[cfg(test)]\n#[path = \"b_tests.rs\"]\nmod tests;\n";
const B_TESTS: &str = "use super::*;\nfn helper() -> u32 {\n    gamma()\n}\n#[test]\nfn gamma_is_two() {\n    assert_eq!(helper(), 2);\n}\n";
const C: &str = "pub const SCALE: u32 = 2;\n";
const D: &str = "#[cfg(test)]\nmod tests {\n    use crate::c::*;\n    #[test]\n    fn scale_is_two() {\n        assert_eq!(SCALE, 2);\n    }\n}\n";
const ROWS: &str = "export function decisionRows() {\n  return helperRows();\n}\nfunction helperRows() {\n  return 1;\n}\nexport const ROW_LIMIT = 5;\n";
const PAGE: &str = "import { decisionRows } from './rows';\nexport const PAGE_SIZE = 10;\nexport function Page() {\n  return decisionRows();\n}\n";
const PAGE_TEST: &str = "import { PAGE_SIZE } from './page';\ntest('size', () => {\n  expect(PAGE_SIZE).toBe(10);\n});\n";
const ROWS_TEST: &str = "import { decisionRows } from './rows';\ntest('rows', () => {\n  expect(decisionRows()).toBe(1);\n});\n";

/// A committed repo, so calls read CC3's base plus overlay.
fn fixture() -> tempfile::TempDir {
    let r = repo(&[
        (
            "src/lib.rs",
            "pub mod a;\npub mod b;\npub mod c;\npub mod d;\n",
        ),
        ("src/a.rs", A),
        ("src/b.rs", B),
        ("src/b_tests.rs", B_TESTS),
        ("src/c.rs", C),
        ("src/d.rs", D),
        ("ui/rows.ts", ROWS),
        ("ui/page.ts", PAGE),
        ("ui/page.test.ts", PAGE_TEST),
        ("ui/rows.test.ts", ROWS_TEST),
        (
            "ui/limit.test.ts",
            "import { ROW_LIMIT } from './rows';\ntest('limit', () => {\n  expect(ROW_LIMIT).toBe(5);\n});\n",
        ),
        (
            "tests/smoke/rows.spec.ts",
            "import { decisionRows } from '../../ui/rows';\ntest('rows', () => {\n  decisionRows();\n});\n",
        ),
        ("Cargo.toml", "[package]\nname = \"x\"\n"),
        ("package.json", "{}\n"),
        ("scripts/gate.mjs", "export const x = 1;\n"),
    ]);
    git(r.path(), &["add", "-A"]);
    git(r.path(), &["commit", "-qm", "base"]);
    r
}

fn first(r: &Value) -> Value {
    serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn ok(t: &Target, tool: &str, args: Value) -> Value {
    let r = call(tool, &args, t);
    assert_eq!(r["isError"], false, "{r}");
    first(&r)
}

fn refused(t: &Target, tool: &str, args: Value) -> (String, Value) {
    let r = call(tool, &args, t);
    assert_eq!(r["isError"], true, "{r}");
    let text = r["content"][0]["text"].as_str().unwrap().to_string();
    let block: Value = serde_json::from_str(r["content"][1]["text"].as_str().unwrap()).unwrap();
    (text, block["error"].clone())
}

fn write(r: &tempfile::TempDir, path: &str, text: &str) {
    std::fs::write(r.path().join(path), text).unwrap();
}

/// `(name, change)` of one file's symbols in a `changes` answer.
fn symbols_of(v: &Value, path: &str) -> BTreeSet<(String, String)> {
    v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .unwrap_or_else(|| panic!("{path} in {v}"))["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap().to_string(),
                s["change"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn set(items: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    items
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect()
}

#[test]
fn changes_maps_edits_adds_deletes_and_a_rename_to_symbols() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    assert_eq!(ok(&t, "changes", json!({}))["totals"]["files"], 0);
    // beta's body edited, gone deleted, after renamed to later, one added.
    let edited = A
        .replace("    1\n", "    2\n")
        .replace("pub fn gone() {}\n", "")
        .replace(
            "pub fn after() {}\n",
            "pub fn later() {}\npub fn added_fn() -> u32 {\n    7\n}\n",
        );
    write(&r, "src/a.rs", &edited);
    let v = ok(&t, "changes", json!({}));
    assert_eq!(v["base"].as_str().unwrap().len(), 9, "{v}");
    assert_eq!(
        symbols_of(&v, "src/a.rs"),
        set(&[
            ("beta", "edited"),
            ("gone", "deleted"),
            ("after", "deleted"),
            ("later", "added"),
            ("added_fn", "added")
        ]),
        "{v}"
    );
    // A deleted symbol names its old lines; a current one its lines now.
    let a = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == "src/a.rs")
        .unwrap()
        .clone();
    let gone = a["symbols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "gone")
        .unwrap();
    assert_eq!(gone["oldLines"], json!([7, 7]));
    assert_eq!(a["status"], "modified");
    // A deletion inside a body edits it (old side only).
    write(&r, "src/a.rs", &A.replace("    beta()\n", ""));
    let v = ok(&t, "changes", json!({}));
    assert_eq!(
        symbols_of(&v, "src/a.rs"),
        set(&[("alpha", "edited")]),
        "{v}"
    );
    // An edited test fn inside the cfg(test) module is that fn, not the module.
    write(
        &r,
        "src/a.rs",
        &A.replace("super::alpha(), 1", "super::alpha(), 1 + 0"),
    );
    let v = ok(&t, "changes", json!({}));
    assert_eq!(
        symbols_of(&v, "src/a.rs"),
        set(&[("alpha_is_one", "edited")]),
        "{v}"
    );
    // A deleted file lists its old definitions; an untracked one its new.
    write(&r, "src/a.rs", A);
    std::fs::remove_file(r.path().join("src/c.rs")).unwrap();
    write(&r, "src/e.rs", "pub fn epsilon() {}\n");
    let v = ok(&t, "changes", json!({}));
    assert_eq!(
        symbols_of(&v, "src/c.rs"),
        set(&[("SCALE", "deleted")]),
        "{v}"
    );
    assert_eq!(
        symbols_of(&v, "src/e.rs"),
        set(&[("epsilon", "added")]),
        "{v}"
    );
    // paths narrows it.
    let v = ok(&t, "changes", json!({ "paths": ["src/e.rs"] }));
    assert_eq!(v["totals"]["files"], 1, "{v}");
}

#[test]
fn changes_lists_config_and_non_indexed_files_as_unmapped() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    write(&r, "Cargo.toml", "[package]\nname = \"y\"\n");
    write(&r, "package.json", "{ \"a\": 1 }\n");
    write(&r, "scripts/gate.mjs", "export const x = 2;\n");
    std::fs::create_dir_all(r.path().join("tests/fixtures")).unwrap();
    write(&r, "tests/fixtures/case.json", "{}\n");
    let v = ok(&t, "changes", json!({}));
    let reasons: BTreeMap<String, String> = v["unmapped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| {
            (
                u["path"].as_str().unwrap().to_string(),
                u["reason"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(reasons["Cargo.toml"].starts_with("config"), "{v}");
    assert!(reasons["package.json"].starts_with("config"), "{v}");
    assert!(reasons["scripts/gate.mjs"].contains(".mjs"), "{v}");
    assert!(
        reasons["tests/fixtures/case.json"].starts_with("fixture"),
        "{v}"
    );
    assert_eq!(v["totals"]["unmapped"], 4, "{v}");
    assert_eq!(v["totals"]["mapped"], 0, "{v}");
    // affected_tests carries them into cannotMap.
    let v = ok(&t, "affected_tests", json!({}));
    let paths: Vec<&str> = v["cannotMap"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["path"].as_str())
        .collect();
    assert!(
        paths.contains(&"Cargo.toml") && paths.contains(&"scripts/gate.mjs"),
        "{v}"
    );
    assert_eq!(v["fullGateStillRequired"], true);
    assert!(v["missRate"].as_str().is_some_and(|m| !m.is_empty()));
}

/// Each listed row: `(command, confidence)`.
fn rows(v: &Value) -> Vec<(String, String)> {
    v["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            (
                t["command"].as_str().unwrap().to_string(),
                t["confidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn listed(v: &Value, command: &str) -> Value {
    v["tests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["command"] == command)
        .cloned()
        .unwrap_or_else(|| panic!("`{command}` not in {:#?}", rows(v)))
}

#[test]
fn affected_tests_finds_direct_transitive_and_import_tests() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    // Edit alpha: its own module's test calls it (direct), gamma calls it
    // and b_tests' helper calls gamma (transitive, through `#[path]`).
    write(
        &r,
        "src/a.rs",
        &A.replace("    beta()\n", "    beta() + 0\n"),
    );
    let v = ok(&t, "affected_tests", json!({}));
    let a = listed(&v, "cargo test --lib a::tests::");
    assert_eq!(a["confidence"], "direct", "{v}");
    assert_eq!(a["tests"][0]["name"], "a::tests::alpha_is_one", "{v}");
    assert_eq!(a["wholeModule"], true, "same-file adds the module: {v}");
    assert_eq!(
        v["run"],
        json!(["cargo test --lib -- a::tests:: b::tests::gamma_is_two"]),
        "{v}"
    );
    let b = listed(&v, "cargo test --lib b::tests::gamma_is_two");
    assert_eq!(b["confidence"], "transitive", "{v}");
    assert_eq!(
        b["tests"][0]["why"], "edited alpha (src/a.rs:1) <- gamma <- helper <- gamma_is_two",
        "{v}"
    );
    // Edit helperRows in TS: rows.test.ts and the Playwright spec call
    // decisionRows (transitive); limit.test.ts only imports rows.ts
    // (import); page.test.ts imports page.ts, which imports rows.ts: the
    // import walk is one level, so it is not listed.
    write(&r, "src/a.rs", A);
    write(&r, "ui/rows.ts", &ROWS.replace("return 1;", "return 2;"));
    let v = ok(&t, "affected_tests", json!({}));
    assert_eq!(
        listed(&v, "npx vitest run ui/rows.test.ts")["confidence"],
        "transitive",
        "{v}"
    );
    let limit = listed(&v, "npx vitest run ui/limit.test.ts");
    assert_eq!(limit["confidence"], "import", "{v}");
    assert_eq!(limit["why"], "ui/rows.ts <- imported by ui/limit.test.ts");
    assert!(!v.to_string().contains("page.test.ts"), "{v}");
    // A `.spec.` file is Playwright's.
    let spec = listed(&v, "npx playwright test tests/smoke/rows.spec.ts");
    assert_eq!(
        (spec["runner"].as_str(), spec["confidence"].as_str()),
        (Some("playwright"), Some("transitive")),
        "{v}"
    );
    assert_eq!(
        v["run"],
        json!([
            "npx vitest run ui/rows.test.ts ui/limit.test.ts",
            "npx playwright test tests/smoke/rows.spec.ts"
        ]),
        "{v}"
    );
}

#[test]
fn affected_tests_reports_a_macro_only_reference_and_holds_its_depth() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    // SCALE is used only inside assert_eq! in d.rs: unresolved, so no test
    // is listed and cannotMap says why.
    let v = ok(&t, "affected_tests", json!({ "symbols": ["SCALE"] }));
    assert!(v["tests"].as_array().unwrap().is_empty(), "{v}");
    let reasons: Vec<&str> = v["cannotMap"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|c| c["reason"].as_str())
        .collect();
    assert!(reasons.iter().any(|r| r.contains("macro arguments")), "{v}");
    // beta at depth 1 reaches alpha's test, not gamma's (two levels up);
    // at depth 2 both, and the cut is reported.
    let v1 = ok(
        &t,
        "affected_tests",
        json!({ "symbols": ["beta"], "depth": 1 }),
    );
    let cmds: Vec<String> = rows(&v1).into_iter().map(|r| r.0).collect();
    // a.rs holds beta, so its test module is listed whole (same-file).
    assert_eq!(cmds, ["cargo test --lib a::tests::"], "{v1}");
    assert_eq!(
        v1["tests"][0]["tests"][0]["name"], "a::tests::alpha_is_one",
        "{v1}"
    );
    assert!(v1["cannotMap"].to_string().contains("past depth 1"), "{v1}");
    let v2 = ok(&t, "affected_tests", json!({ "symbols": ["beta"] }));
    let cmds: Vec<String> = rows(&v2).into_iter().map(|r| r.0).collect();
    assert!(
        cmds.contains(&"cargo test --lib b::tests::gamma_is_two".to_string()),
        "{v2}"
    );
    assert_eq!(
        listed(&v2, "cargo test --lib a::tests::")["confidence"],
        "transitive"
    );
    // A name that isn't defined is in cannotMap with a did-you-mean.
    let v = ok(&t, "affected_tests", json!({ "symbols": ["gama"] }));
    assert!(
        v["cannotMap"].to_string().contains("did you mean `gamma`"),
        "{v}"
    );
}

#[test]
fn strict_arguments_and_refusals() {
    let (r, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&r, &cache);
    let (text, err) = refused(&t, "changes", json!({ "bse": "HEAD" }));
    assert!(text.contains("did you mean \"base\""), "{text}");
    assert_eq!(err["code"], "INVALID_ARGUMENT");
    let (text, err) = refused(&t, "changes", json!({ "base": "no-such-branch" }));
    assert!(text.contains("no such commit `no-such-branch`"), "{text}");
    assert_eq!(err["code"], "NOT_FOUND");
    let (text, err) = refused(
        &t,
        "affected_tests",
        json!({ "symbols": ["a"], "base": "HEAD" }),
    );
    assert!(text.contains("must be given alone"), "{text}");
    assert_eq!(err["code"], "INVALID_ARGUMENT");
    let (text, _) = refused(&t, "affected_tests", json!({ "depth": 5 }));
    assert!(text.contains("from 1 to 4"), "{text}");
    let (text, _) = refused(&t, "changes", json!({ "paths": ["../x"] }));
    assert!(text.contains("repo-relative"), "{text}");
    let (text, _) = refused(&t, "changes", json!({ "base": "--output=/tmp/x" }));
    assert!(text.contains("no such commit"), "{text}");
}

#[test]
fn patch_paths_and_ranges_parse_including_quoted_names() {
    let text = "diff --git a/src/x.rs b/src/x.rs\nindex 1..2 100644\n--- a/src/x.rs\n+++ b/src/x.rs\n@@ -3,0 +4,2 @@ fn a\n+x\n+y\n@@ -9 +11,0 @@\n-z\ndiff --git \"a/sp ace\\tx.rs\" \"b/sp ace\\tx.rs\"\n--- \"a/sp ace\\tx.rs\"\n+++ \"b/sp ace\\tx.rs\"\n@@ -1 +1 @@\n-a\n+b\ndiff --git a/img.png b/img.png\nBinary files a/img.png and b/img.png differ\ndiff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n";
    let p = parse_patch(text);
    assert_eq!(
        p["src/x.rs"].0,
        vec![
            Hunk {
                old: (3, 0),
                new: (4, 2)
            },
            Hunk {
                old: (9, 1),
                new: (11, 0)
            }
        ]
    );
    assert_eq!(
        p["sp ace\tx.rs"].0,
        vec![Hunk {
            old: (1, 1),
            new: (1, 1)
        }]
    );
    assert!(p["img.png"].1);
    assert_eq!(
        p["gone.rs"].0,
        vec![Hunk {
            old: (1, 2),
            new: (0, 0)
        }]
    );
}

#[test]
fn rust_modules_follow_path_attributes_and_nest() {
    let (r, cache) = (
        repo(&[
            ("crate/src/lib.rs", "mod outer;\n"),
            (
                "crate/src/outer.rs",
                "pub fn f() {}\n#[cfg(test)]\n#[path = \"outer_tests.rs\"]\nmod tests;\n",
            ),
            (
                "crate/src/outer_tests.rs",
                "#[path = \"outer/deep_tests.rs\"]\nmod deep;\n#[test]\nfn t() { super::f(); }\n",
            ),
            (
                "crate/src/outer/deep_tests.rs",
                "#[test]\nfn d() { crate::outer::f(); }\nfn not_a_test() {}\n",
            ),
        ]),
        tempfile::tempdir().unwrap(),
    );
    let t = target(&r, &cache);
    let v = ok(&t, "affected_tests", json!({ "symbols": ["f"] }));
    let cmds: Vec<String> = rows(&v).into_iter().map(|r| r.0).collect();
    assert!(
        cmds.contains(&"cd crate && cargo test --lib outer::tests::".to_string()),
        "{v}"
    );
    assert!(
        cmds.contains(&"cd crate && cargo test --lib outer::tests::deep::d".to_string()),
        "{v}"
    );
    assert!(!v.to_string().contains("not_a_test"), "{v}");
}

// ── The evaluation (CC5's done-when) ────────────────────────────────────

/// One merged PR's result.
struct Scored {
    pr: String,
    listed: usize,
    /// Rows listed by `runner:confidence`.
    rows: BTreeMap<String, usize>,
    /// Truth tests found by a call-graph rule (direct, transitive, import,
    /// name-match), not only by a same-file or same-module filter.
    graph_hits: usize,
    truth: Vec<String>,
    misses: Vec<String>,
    unmapped: usize,
}

/// Score `affected_tests` on merge commit `merge` of the repo at `git`:
/// the index of the merge, the diff from its first parent, test files
/// and test functions left out of the input, and the tests the PR added
/// or changed as the truth.
fn score(git: &Git, pr: &str, merge: &str, cache: &Path, tuning: Tuning) -> Scored {
    let parent = code_index::overlay::full_commit(git, &format!("{merge}^1")).unwrap();
    let merge = code_index::overlay::full_commit(git, merge).unwrap();
    let out = cache.join(format!("{}.sqlite", &merge[..12]));
    if !out.exists() {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600); // sleep-ok: a build deadline, asserts nothing on time
        code_index::overlay::build_base(git, &merge, &out, &code_index::LIMITS, deadline).unwrap();
    }
    let index = Index::open(&out).unwrap();
    let files = diff(git, &index, &parent, Some(&merge), &[]).unwrap();
    let read = |p: &str| -> Option<Vec<u8>> {
        git.run_bytes(&["cat-file", "blob", &format!("{merge}:{p}")], Vec::new())
            .ok()
    };
    let rust = Rust::new(&index, &read);
    let mut truth = Vec::new();
    for f in &files {
        if f.unmapped.is_some() || f.status == "deleted" {
            continue;
        }
        if f.test && !f.path.ends_with(".rs") {
            truth.push(f.path.clone());
            continue;
        }
        for s in &f.symbols {
            if let Some(n) = s.node.as_ref().filter(|n| rust.is_test_fn(n)) {
                truth.push(format!("{}::{}", rust.def_module(n), n.name));
            }
        }
    }
    let input: Vec<ChangedFile> = files
        .iter()
        .filter(|f| !f.test)
        .map(|f| {
            let mut f = f.clone();
            f.symbols.retain(|s| !s.test());
            f
        })
        .collect();
    let roots: Vec<ChangedSymbol> = input.iter().flat_map(|f| f.symbols.clone()).collect();
    let found = affected(&index, &read, &roots, &input, 2, tuning).unwrap();
    let (filters, test_files) = found.listed();
    let mut rows: BTreeMap<String, usize> = BTreeMap::new();
    for r in found.rows() {
        let key = format!(
            "{}:{}",
            r["runner"].as_str().unwrap_or(""),
            r["confidence"].as_str().unwrap_or("")
        );
        *rows.entry(key).or_default() += 1;
    }
    let graph_hits = truth
        .iter()
        .filter(|t| {
            found
                .confidence_for(t)
                .is_some_and(|c| !matches!(c, "same-file" | "same-module"))
        })
        .count();
    let misses: Vec<String> = truth
        .iter()
        .filter(|t| !(test_files.contains(t) || filters.iter().any(|f| t.contains(f.as_str()))))
        .cloned()
        .collect();
    Scored {
        pr: pr.to_string(),
        listed: filters.len() + test_files.len(),
        rows,
        graph_hits,
        truth,
        misses,
        unmapped: files.iter().filter(|f| f.unmapped.is_some()).count(),
    }
}

/// The evaluation in docs/CODE_CONTEXT.md (not part of the suite):
/// `HOLLOW_CC5_EVAL=<a hollow checkout> cargo test --lib
/// code_context::changes::tests::evaluate_on_merged_prs -- --ignored
/// --nocapture`. Reads only git objects (no work tree is touched).
#[test]
#[ignore = "evaluation, run by hand: affected_tests on 10 merged Hollow PRs (docs/CODE_CONTEXT.md)"]
fn evaluate_on_merged_prs() {
    let Ok(src) = std::env::var("HOLLOW_CC5_EVAL") else {
        return;
    };
    let git = Git::open(Path::new(&src)).unwrap();
    // HOLLOW_CC5_EVAL_CACHE keeps the base indexes between runs.
    let tmp = tempfile::tempdir().unwrap();
    let cache =
        std::env::var("HOLLOW_CC5_EVAL_CACHE").map_or(tmp.path().to_path_buf(), PathBuf::from);
    std::fs::create_dir_all(&cache).unwrap();
    let prs = [
        ("#1584", "583e9f6cd"),
        ("#1563", "163f694bf"),
        ("#1564", "23e67aee5"),
        ("#1554", "f7340bdd0"),
        ("#1537", "fe0552b57"),
        ("#1573", "0ac2af94f"),
        ("#1557", "65503a49a"),
        ("#1555", "c64fbdd16"),
        ("#1575", "642f6b1d9"),
        ("#1585", "3a752b9de"),
    ];
    let (mut truth, mut misses, mut graph) = (0usize, 0usize, 0usize);
    println!("| PR | listed | ground truth | hits | by call graph | misses | unmapped files |");
    println!("|---|---|---|---|---|---|---|");
    for (pr, merge) in prs {
        let s = score(&git, pr, merge, &cache, Tuning::TOOL);
        truth += s.truth.len();
        misses += s.misses.len();
        graph += s.graph_hits;
        println!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            s.pr,
            s.listed,
            s.truth.len(),
            s.truth.len() - s.misses.len(),
            s.graph_hits,
            if s.misses.is_empty() {
                "0".to_string()
            } else {
                format!("{} ({})", s.misses.len(), s.misses.join(", "))
            },
            s.unmapped
        );
        println!("  rows: {:?}", s.rows);
    }
    // The rules beyond resolved calls, compared: listed rows and misses.
    for (label, tuning) in [
        (
            "name-match everywhere, import walk to depth",
            Tuning {
                name_match_ts: true,
                import_depth: 4,
            },
        ),
        (
            "name-match Rust only, import walk to depth",
            Tuning {
                name_match_ts: false,
                import_depth: 4,
            },
        ),
        (
            "name-match everywhere, import walk 1 level",
            Tuning {
                name_match_ts: true,
                import_depth: 1,
            },
        ),
        (
            "name-match Rust only, import walk 1 level",
            Tuning {
                name_match_ts: false,
                import_depth: 1,
            },
        ),
    ] {
        let (mut listed, mut missed, mut per) = (0usize, 0usize, Vec::new());
        for (pr, merge) in prs {
            let s = score(&git, pr, merge, &cache, tuning);
            listed += s.listed;
            missed += s.misses.len();
            per.push(format!("{pr} {}/{}", s.listed, s.misses.len()));
        }
        println!(
            "variant {label}: {listed} rows, {missed} missed; {}",
            per.join(", ")
        );
    }
    println!(
        "overall: {misses} of {truth} missed ({:.1}%); {graph} found by the call graph alone, so {} of {truth} ({:.1}%) would be missed without the same-file and same-module filters",
        100.0 * misses as f64 / truth.max(1) as f64,
        truth - graph,
        100.0 * (truth - graph) as f64 / truth.max(1) as f64
    );
}

/// The listing for a break-on-purpose check (docs/CODE_CONTEXT.md): with
/// the break committed as a scratch commit in the checkout
/// `HOLLOW_CC5_LIST`, prints what `affected_tests` lists against `HEAD~1`.
#[test]
#[ignore = "evaluation, run by hand: the listing for a break-on-purpose check (docs/CODE_CONTEXT.md)"]
fn list_for_worktree() {
    let Ok(root) = std::env::var("HOLLOW_CC5_LIST") else {
        return;
    };
    let cache = tempfile::tempdir().unwrap();
    let t = Target {
        root: std::fs::canonicalize(&root).unwrap(),
        index_dir: Some(cache.path().to_path_buf()),
        vault: None,
    };
    let v = ok(
        &t,
        "affected_tests",
        json!({ "base": "HEAD~1", "maxBytes": 80_000 }),
    );
    println!("from: {}", v["from"]);
    for row in v["tests"].as_array().unwrap() {
        println!(
            "{} [{}]",
            row["command"].as_str().unwrap(),
            row["confidence"].as_str().unwrap()
        );
    }
    println!("cannotMap: {}", v["cannotMap"]);
}
