//! CC2: parse goldens per language, the byte-cap skip and the file filter,
//! and one resolution test per CC1 design 2.3 rule plus the cases that must
//! stay unresolved.

use super::parse::{parse_file, Lang};
use super::resolve::{resolve, Res};
use super::*;

fn parsed(path: &str, src: &str) -> FileParse {
    let lang = Lang::of(path).unwrap();
    FileParse {
        path: path.into(),
        lang,
        sha: short_sha(src.as_bytes()),
        bytes: src.len() as u64,
        test: parse::test_path(path),
        parsed: parse_file(path, lang, src.as_bytes()),
    }
}

/// `(kind, name, container, modpath, test, command)` of every definition.
fn defs(f: &FileParse) -> Vec<(String, String, String, String, bool, bool)> {
    f.parsed
        .defs
        .iter()
        .map(|d| {
            (
                d.kind.clone(),
                d.name.clone(),
                d.container.clone(),
                d.modpath.clone(),
                d.test,
                d.command,
            )
        })
        .collect()
}

/// `(line, kind, name, qual, macro_args)` of every edge named one of `names`.
fn edges(f: &FileParse, names: &[&str]) -> Vec<(u32, String, String, String, bool)> {
    f.parsed
        .edges
        .iter()
        .filter(|e| names.contains(&e.name.as_str()))
        .map(|e| {
            (
                e.line,
                e.kind.clone(),
                e.name.clone(),
                e.qual.clone(),
                e.macro_args,
            )
        })
        .collect()
}

fn e(
    line: u32,
    kind: &str,
    name: &str,
    qual: &str,
    macro_args: bool,
) -> (u32, String, String, String, bool) {
    (line, kind.into(), name.into(), qual.into(), macro_args)
}

fn d(
    kind: &str,
    name: &str,
    container: &str,
    modpath: &str,
    test: bool,
    command: bool,
) -> (String, String, String, String, bool, bool) {
    (
        kind.into(),
        name.into(),
        container.into(),
        modpath.into(),
        test,
        command,
    )
}

const RUST: &str = r#"use crate::skills::{valid_skill_name, other};
mod inner {
    pub fn helper() {}
}
struct Caps;
impl Caps {
    fn teamwork(&self) -> bool { true }
    fn check(&self) { Self::teamwork(self); self.teamwork(); }
}
#[tauri::command]
fn set_flag(caps: Caps) {
    inner::helper();
    let v = vec![crate::skills::valid_skill_name("x")];
    assert!(caps.teamwork(), "{}", format!("{}", helper()));
    let s = "helper()";
}
#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
"#;

#[test]
fn parse_rust_goldens_with_macro_argument_calls() {
    let f = parsed("src-tauri/src/a.rs", RUST);
    assert_eq!(
        defs(&f),
        vec![
            d("mod", "inner", "", "a", false, false),
            d("fn", "helper", "", "a::inner", false, false),
            d("struct", "Caps", "", "a", false, false),
            d("method", "teamwork", "Caps", "a", false, false),
            d("method", "check", "Caps", "a", false, false),
            d("fn", "set_flag", "", "a", false, true),
            d("mod", "tests", "", "a", true, false),
            d("fn", "t", "", "a::tests", true, false),
        ]
    );
    assert_eq!(
        edges(
            &f,
            &["valid_skill_name", "helper", "teamwork", "vec", "assert"]
        ),
        vec![
            e(1, "import", "valid_skill_name", "crate::skills", false),
            e(8, "call", "teamwork", "Self", false),
            e(8, "method", "teamwork", ".self", false),
            e(12, "call", "helper", "inner", false),
            e(13, "macro", "vec", "", false),
            e(13, "call", "valid_skill_name", "crate::skills", true),
            e(14, "macro", "assert", "", false),
            e(14, "method", "teamwork", ".", true),
            e(14, "call", "helper", "", true),
        ],
        "string literals never yield edges; macro arguments do"
    );
    let src = f.parsed.edges.iter().find(|e| e.line == 12).unwrap().src;
    assert_eq!(f.parsed.defs[src.unwrap() as usize].name, "set_flag");
}

const TS: &str = r#"import { decisionRows, other as renamed } from '../lib/decisionRows';
import Panel from './Panel';
export interface Props { a: Item }
export type Item = { id: string };
export const LIMIT = 3;
export function RecordDecisions(p: Props) {
  const rows = decisionRows(p);
  invoke('set_vault_mcp_enabled', { v: true });
  const m = new Map();
  rows.forEach(r => r.id);
  return <Panel rows={rows} />;
}
class Store { load() { return this.save(); } save() {} }
"#;

#[test]
fn parse_typescript_goldens() {
    let f = parsed("src/pages/Records.tsx", TS);
    assert_eq!(
        defs(&f),
        vec![
            d("iface", "Props", "", "pages::Records", false, false),
            d("type", "Item", "", "pages::Records", false, false),
            d("const", "LIMIT", "", "pages::Records", false, false),
            d("fn", "RecordDecisions", "", "pages::Records", false, false),
            d("class", "Store", "", "pages::Records", false, false),
            d("method", "load", "Store", "pages::Records", false, false),
            d("method", "save", "Store", "pages::Records", false, false),
        ]
    );
    assert_eq!(
        edges(
            &f,
            &[
                "decisionRows",
                "other",
                "Panel",
                "set_vault_mcp_enabled",
                "Map",
                "save",
                "forEach",
                "Item"
            ]
        ),
        vec![
            e(1, "import", "decisionRows", "../lib/decisionRows", false),
            e(1, "import", "other", "../lib/decisionRows", false),
            e(2, "import", "Panel", "./Panel", false),
            e(3, "ref", "Item", "", false),
            e(7, "call", "decisionRows", "", false),
            e(8, "invoke", "set_vault_mcp_enabled", "", false),
            e(9, "new", "Map", "", false),
            e(10, "method", "forEach", ".rows", false),
            e(11, "jsx", "Panel", "", false),
            e(13, "method", "save", ".this", false),
        ]
    );
}

const PY: &str = r#"from .cases import case_text, other as o
import os

LIMIT = 3

class Runner:
    def run(self):
        return self.step(case_text(LIMIT))
    def step(self, x):
        return os.path.join(x)

def test_runner():
    Runner().run()
"#;

#[test]
fn parse_python_goldens() {
    let f = parsed("eval/runner.py", PY);
    assert_eq!(
        defs(&f),
        vec![
            d("var", "LIMIT", "", "eval::runner", false, false),
            d("class", "Runner", "", "eval::runner", false, false),
            d("method", "run", "Runner", "eval::runner", false, false),
            d("method", "step", "Runner", "eval::runner", false, false),
            d("fn", "test_runner", "", "eval::runner", true, false),
        ]
    );
    assert_eq!(
        edges(
            &f,
            &[
                "case_text",
                "other",
                "step",
                "LIMIT",
                "Runner",
                "run",
                "join"
            ]
        ),
        vec![
            e(1, "import", "case_text", ".cases", false),
            e(1, "import", "other", ".cases", false),
            e(8, "method", "step", ".self", false),
            e(8, "call", "case_text", "", false),
            e(8, "ref", "LIMIT", "", false),
            e(10, "method", "join", ".os.path", false),
            e(13, "call", "Runner", "", false),
            e(13, "method", "run", ".Runner()", false),
        ]
    );
}

/// The index's file filter: listed code files only; never hidden, secret
/// looking, symlinked, ignored or over the byte cap (that one reported).
#[test]
fn list_files_skips_over_cap_hidden_secret_ignored_and_links() {
    let repo = crate::code_context::tests::repo(&[
        ("src/a.rs", "fn a() {}\n"),
        ("src/.hidden/b.rs", "fn b() {}\n"),
        (".github/c.ts", "export const c = 1;\n"),
        ("src/secret_store.rs", "fn s() {}\n"),
        ("target/gen.rs", "fn g() {}\n"),
        (".gitignore", "target/\n"),
        ("README.md", "# x\n"),
        ("web/app.mjs", "export const m = 1;\n"),
    ]);
    let big = "// x\n".repeat((MAX_FILE_BYTES as usize / 5) + 1);
    std::fs::write(repo.path().join("src/big.rs"), big).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/hosts", repo.path().join("src/link.rs")).unwrap();
    let listing = list_files(repo.path()).unwrap();
    let paths: Vec<&str> = listing.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["src/a.rs"]);
    assert_eq!(listing.skipped, vec!["src/big.rs".to_string()]);
}

/// The annotation of the edge at `line` named `name` in file `file`.
fn res_at<'a>(
    files: &[FileParse],
    all: &'a [Vec<Res>],
    file: &str,
    line: u32,
    name: &str,
    kind: &str,
) -> &'a Res {
    let fi = files.iter().position(|f| f.path == file).unwrap();
    let ei = files[fi]
        .parsed
        .edges
        .iter()
        .position(|e| e.line == line && e.name == name && e.kind == kind)
        .unwrap_or_else(|| panic!("no {kind} edge {name} at {file}:{line}"));
    &all[fi][ei]
}

/// The `(path, name, container)` a resolved edge points at.
fn target_of(files: &[FileParse], res: &Res) -> Option<(String, String, String)> {
    let firsts = resolve::node_ids(files);
    let id = res.target?;
    let fi = firsts.iter().rposition(|f| *f <= id)?;
    let def = &files[fi].parsed.defs[(id - firsts[fi]) as usize];
    Some((
        files[fi].path.clone(),
        def.name.clone(),
        def.container.clone(),
    ))
}

#[test]
fn resolve_rules_each_annotate_and_ambiguous_cases_stay_unresolved() {
    let files = vec![
        parsed(
            "src/records.rs",
            "pub fn decisions() {}\npub fn check() {}\nfn local() { decisions(); check(); solo(); }\n",
        ),
        parsed("src/solo.rs", "pub fn solo() {}\n"
        ),
        parsed("src/memory_guard.rs", "pub fn check() {}\n"),
        parsed(
            "src/caps.rs",
            "pub struct Caps;\nimpl Caps {\n    pub fn teamwork(&self) -> bool { true }\n    pub fn run(&self) { self.teamwork(); Self::teamwork(self); }\n}\npub struct Other;\nimpl Other { pub fn go(&self) {} }\nimpl Caps { pub fn go(&self) {} }\n",
        ),
        parsed(
            "src/user.rs",
            "fn u(x: Caps, teamwork: bool) {\n    crate::records::decisions();\n    crate::memory_guard::check();\n    x.teamwork();\n    x.go();\n    serde_json::to_string(&teamwork);\n    vec![crate::records::decisions()];\n    let t = teamwork;\n    unique_free();\n    check();\n}\nfn unique_free() {}\n",
        ),
        parsed("src/cmd_a.rs", "#[tauri::command]\npub fn set_flag() {}\n"),
        parsed("src/cmd_b.rs", "pub fn set_flag() {}\n"),
        parsed("ui/rows.ts", "export function decisionRows() {}\nexport function check() {}\n"),
        parsed(
            "ui/page.ts",
            "import { decisionRows } from './rows';\ninvoke('set_flag');\ndecisionRows();\n",
        ),
    ];
    let all = resolve(&files);
    let at = |file: &str, line: u32, name: &str, kind: &str| {
        let r = res_at(&files, &all, file, line, name, kind);
        (r.how.clone(), target_of(&files, r))
    };
    let t = |p: &str, n: &str, c: &str| Some((p.to_string(), n.to_string(), c.to_string()));
    // qualifier: the last path segment names one module or impl type.
    assert_eq!(
        at("src/user.rs", 2, "decisions", "call"),
        ("qualifier".into(), t("src/records.rs", "decisions", ""))
    );
    assert_eq!(
        at("src/user.rs", 3, "check", "call"),
        ("qualifier".into(), t("src/memory_guard.rs", "check", ""))
    );
    assert_eq!(
        at("src/caps.rs", 4, "teamwork", "call"),
        ("qualifier".into(), t("src/caps.rs", "teamwork", "Caps"))
    );
    // self-type: `self.N()` in the impl that has exactly one N.
    assert_eq!(
        at("src/caps.rs", 4, "teamwork", "method"),
        ("self-type".into(), t("src/caps.rs", "teamwork", "Caps"))
    );
    // same-file wins for a bare call; unique-name across the repo.
    assert_eq!(
        at("src/records.rs", 3, "decisions", "call"),
        ("same-file".into(), t("src/records.rs", "decisions", ""))
    );
    assert_eq!(
        at("src/records.rs", 3, "check", "call"),
        ("same-file".into(), t("src/records.rs", "check", ""))
    );
    assert_eq!(
        at("src/user.rs", 9, "unique_free", "call"),
        ("same-file".into(), t("src/user.rs", "unique_free", ""))
    );
    assert_eq!(
        at("src/records.rs", 3, "solo", "call"),
        ("unique-name".into(), t("src/solo.rs", "solo", ""))
    );
    // unique-method on an unknown receiver; ambiguous methods stay open.
    assert_eq!(
        at("src/user.rs", 4, "teamwork", "method"),
        ("unique-method".into(), t("src/caps.rs", "teamwork", "Caps"))
    );
    assert_eq!(
        at("src/user.rs", 5, "go", "method"),
        ("ambiguous-method(2)".into(), None)
    );
    // An external crate's path and a bare local never resolve.
    assert_eq!(
        at("src/user.rs", 6, "to_string", "call"),
        ("no-def".into(), None)
    );
    assert_eq!(
        at("src/user.rs", 8, "teamwork", "ref"),
        ("bare-ref-may-be-local".into(), None)
    );
    assert_eq!(
        at("src/user.rs", 6, "teamwork", "ref"),
        ("bare-ref-may-be-local".into(), None)
    );
    // A bare call to a name defined in two other files.
    assert_eq!(
        at("src/user.rs", 10, "check", "call"),
        ("ambiguous(2)".into(), None)
    );
    // Macro arguments carry the prefix and still resolve by rule.
    assert_eq!(
        at("src/user.rs", 7, "decisions", "call"),
        (
            "macro-args:qualifier".into(),
            t("src/records.rs", "decisions", "")
        )
    );
    // TS: the relative import, a bare use of the imported name, and the
    // Tauri invoke preferring the #[tauri::command] fn.
    assert_eq!(
        at("ui/page.ts", 1, "decisionRows", "import"),
        ("import".into(), t("ui/rows.ts", "decisionRows", ""))
    );
    assert_eq!(
        at("ui/page.ts", 3, "decisionRows", "call"),
        ("import".into(), t("ui/rows.ts", "decisionRows", ""))
    );
    assert_eq!(
        at("ui/page.ts", 2, "set_flag", "invoke"),
        ("invoke-string".into(), t("src/cmd_a.rs", "set_flag", ""))
    );
}

#[test]
fn resolve_qualifier_with_no_match_is_external() {
    let files = vec![
        parsed("src/a.rs", "pub fn parse() {}\n"),
        parsed("src/b.rs", "fn b() { toml::parse(); }\n"),
    ];
    let all = resolve(&files);
    let r = res_at(&files, &all, "src/b.rs", 1, "parse", "call");
    assert_eq!(
        (r.how.as_str(), r.target),
        ("qualifier-no-match(external?)", None)
    );
}

#[test]
fn split_identifiers_for_symbol_search() {
    assert_eq!(
        store::split_ident("valid_skill_name"),
        "valid skill name validskillname"
    );
    assert_eq!(
        store::split_ident("AttachedCaps"),
        "attached caps attachedcaps"
    );
    assert_eq!(store::split_ident("check"), "check");
    assert_eq!(store::split_ident("HTTPServer"), "http server httpserver");
}

/// The reusable build step CC3 ships a base index with: one file, its
/// counts, and the meta every result carries.
#[test]
fn build_into_writes_one_index_file_with_its_meta() {
    let repo = crate::code_context::tests::repo(&[
        ("src/a.rs", "pub fn a() { b(); }\npub fn b() {}\n"),
        ("ui/c.ts", "export function c() {}\n"),
    ]);
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("base.sqlite");
    let built = build_into(repo.path(), &file).unwrap();
    assert_eq!((built.files, built.nodes, built.edges), (2, 3, 1));
    assert_eq!(store::read_meta(&file, "fingerprint").unwrap().len(), 12);
    assert_eq!(store::read_meta(&file, "parsers").unwrap(), parse::PARSERS);
}

/// Must-fix 3: a build over the file or byte cap, or past its deadline, is
/// refused with the limit named (a leading REFUSED code), and leaves no
/// index, temp or lock file behind.
#[test]
fn build_limits_refuse_naming_the_limit_and_leave_nothing() {
    let repo = crate::code_context::tests::repo(&[
        ("src/a.rs", "pub fn a() {}\n"),
        ("src/b.rs", "pub fn b() { a(); }\n"),
    ]);
    let cache = tempfile::tempdir().unwrap();
    let cases = [
        (
            Limits {
                max_files: 1,
                ..LIMITS
            },
            "limit of 1 files",
        ),
        (
            Limits {
                max_bytes: 5,
                ..LIMITS
            },
            "limit of 5 bytes",
        ),
        (
            Limits {
                deadline: Duration::ZERO,
                ..LIMITS
            },
            "ran past its 0s code index limit",
        ),
    ];
    for (limits, named) in cases {
        let err = ensure_with(repo.path(), Some(cache.path()), &limits).unwrap_err();
        assert!(err.starts_with(LIMIT_REFUSAL), "{err}");
        assert!(err.contains(named), "{err}");
        let code = crate::mcp_envelope::classify(
            crate::mcp_envelope::Server::Code,
            &format!("callers: {err}"),
        )
        .code;
        assert_eq!(code, "REFUSED");
        assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
    }
    assert!(ensure_with(repo.path(), Some(cache.path()), &LIMITS).is_ok());
}

/// Must-fix 2 and should-fix d: a live lock from another process holds a
/// build off until its deadline; a stale one is broken; a successful build
/// prunes this root's dead `.building` files older than an hour.
#[test]
fn build_lock_waits_breaks_stale_and_prunes_dead_temp_files() {
    let repo = crate::code_context::tests::repo(&[("src/a.rs", "pub fn a() {}\n")]);
    let cache = tempfile::tempdir().unwrap();
    let path = index_path(
        &std::fs::canonicalize(repo.path()).unwrap(),
        Some(cache.path()),
    )
    .unwrap();
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    let lock = cache.path().join(format!(".{name}.lock"));
    std::fs::write(&lock, "1\n").unwrap();
    let short = Limits {
        deadline: Duration::from_millis(200),
        ..LIMITS
    };
    let err = ensure_with(repo.path(), Some(cache.path()), &short).unwrap_err();
    assert!(err.contains("another Hollow process"), "{err}");
    assert!(!path.exists());
    let old = SystemTime::now() - Duration::from_secs(2 * 3600);
    std::fs::File::options()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    let dead = cache.path().join(format!(".{name}.999.0.building"));
    let live = cache.path().join(format!(".{name}.998.0.building"));
    std::fs::write(&dead, "x").unwrap();
    std::fs::write(&live, "x").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&dead)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert_eq!(
        ensure_with(repo.path(), Some(cache.path()), &LIMITS).unwrap(),
        path
    );
    assert!(!lock.exists(), "the stale lock was broken and released");
    assert!(!dead.exists(), "a dead build's temp file is pruned");
    assert!(live.exists(), "a recent one is left alone");
}
