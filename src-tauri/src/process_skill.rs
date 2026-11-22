//! Process recorder → skill, stage 2 (master plan #10; morning N1): from a
//! capture's step list (stage 1, [`crate::process_capture`]) to a tested
//! skill. Hollow itself calls no model here: it drafts a skeleton from the
//! steps, the chat's own agent (whichever provider) fills it in and may add a
//! script, Hollow tests it, and the person saves it from a card.
//!
//! Everything lives in the capture's folder,
//! `outputs/process-captures/<capture>/`:
//! - `skill/SKILL.md` ([`draft`]): frontmatter `name`/`description`, the
//!   inputs, the steps and the outputs from `capture.md`. Its `[fill in]`
//!   and `_Describe this step._` markers fail the test until replaced.
//! - `fixtures/run-1/input/` and `fixtures/run-1/expected/` ([`draft`]):
//!   copies of the files `capture.md` lists under Inputs and Outputs that
//!   exist in the workspace. They may hold real data (payroll): they stay in
//!   the workspace and are never installed with the skill. Use synthetic
//!   recordings and files for anything that leaves the Mac.
//! - `test-results.md` ([`prepare_test`] + [`commit_test`]): the pass/fail
//!   table. The skill's `scripts/run.sh`, `run.py` or executable `run` runs
//!   as `run <input dir> <output dir>` once per fixture case, on copies in a
//!   temp folder outside the vault, and every `expected/` file must come out
//!   the same (text compared without line-ending or trailing-space
//!   differences). A skill with no script is instructions only: its fixture
//!   rows are skipped, not passed, and so is a case with no `expected/`.
//!   The note is for reading; the record Save trusts (a digest of `skill/`
//!   and `fixtures/` as tested, and the counts) is kept outside the vault in
//!   app data, so editing the note can't unlock Save.
//! - Saving ([`save_check`], then `skill_import`): refused unless the last
//!   test had no failure and the digest still matches, then staged and
//!   installed into `~/.agents/skills` through the import path's card.
//!
//! No transcription either: a transcript must come with the recording
//! (stage 1). Drafting the automation or runbook from the steps isn't built.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::process_capture::CAPTURE_DIR;

/// Markers the draft leaves for the agent or person to replace.
pub const PLACEHOLDERS: &[&str] = &["[fill in]", "_Describe this step._", "_What happens here?_"];
const FIXTURE_FILE_MAX: u64 = 20 * 1024 * 1024;
const FIXTURES_MAX: u64 = 50 * 1024 * 1024;
const MAX_CASES: usize = 20;
const CASE_TIMEOUT: Duration = Duration::from_secs(120);
const ENTRY_POINTS: &[&str] = &["scripts/run.sh", "scripts/run.py", "scripts/run"];

/// The capture's folder in the workspace: (workspace vault path, capture
/// folder relative to the workspace, its name). `capture` is the folder's
/// name, its path under `outputs/process-captures/`, or its `capture.md`.
fn locate(
    vault: &Path,
    workspace: &str,
    capture: &str,
) -> Result<(String, String, String), String> {
    let ws = crate::workspaces::summary(vault, workspace)?;
    let rel = capture.trim().trim_start_matches('/');
    let rel = rel.strip_prefix(&format!("{}/", ws.path)).unwrap_or(rel);
    let rel = rel.strip_prefix(CAPTURE_DIR).unwrap_or(rel);
    let name = rel.split('/').next().unwrap_or("");
    if name.is_empty() || name == "." || name == ".." || name.starts_with('.') {
        return Err(format!(
            "{capture} isn't a capture: give its folder name under {CAPTURE_DIR}."
        ));
    }
    let folder_rel = format!("{CAPTURE_DIR}{name}");
    let abs = crate::vault::safe_join(vault, &format!("{}/{folder_rel}", ws.path))?;
    if !abs.join("capture.md").is_file() {
        return Err(format!(
            "{folder_rel}/capture.md doesn't exist in workspace {workspace}; run process.capture first."
        ));
    }
    Ok((ws.path, folder_rel, name.to_string()))
}

/// The steps and the Inputs/Outputs lines of a `capture.md`, as the person
/// left it after editing.
#[derive(Debug, Default, PartialEq)]
pub struct CaptureNote {
    pub title: String,
    pub steps: Vec<String>,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
}

pub fn parse_capture(note: &str) -> CaptureNote {
    let mut out = CaptureNote::default();
    let mut section = "";
    // Which list a `- Inputs:` / `- Outputs:` line opened, for its sub-bullets.
    let mut list: Option<bool> = None;
    for line in note.lines() {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("# ") {
            out.title = h.trim_start_matches("Process capture:").trim().to_string();
            continue;
        }
        if let Some(h) = t.strip_prefix("## ") {
            section = if h.starts_with("Steps") {
                "steps"
            } else if h.starts_with("Inputs") {
                "io"
            } else {
                ""
            };
            list = None;
            continue;
        }
        match section {
            "steps" => {
                // `1. **Step 1 (0:05).** Open the push file.`; pictures skipped.
                let Some((num, rest)) = t.split_once(". ") else {
                    continue;
                };
                if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
                    continue;
                }
                let mut text = rest.trim();
                if text.starts_with("**Step ") {
                    if let Some(i) = text.find(".**") {
                        text = text[i + 3..].trim();
                    }
                }
                out.steps.push(text.to_string());
            }
            "io" => {
                let indented = line.starts_with([' ', '\t']);
                let item = t.trim_start_matches(['-', '*']).trim();
                let (into_inputs, rest) = if !indented && item.starts_with("Inputs:") {
                    (Some(true), &item["Inputs:".len()..])
                } else if !indented && item.starts_with("Outputs:") {
                    (Some(false), &item["Outputs:".len()..])
                } else if indented && list.is_some() {
                    (list, item)
                } else {
                    (None, "")
                };
                if into_inputs.is_some() {
                    list = into_inputs;
                }
                let target = match into_inputs {
                    Some(true) => &mut out.inputs,
                    Some(false) => &mut out.outputs,
                    None => continue,
                };
                target.extend(
                    rest.split(',')
                        .map(|s| s.trim().trim_matches('`').trim().to_string())
                        .filter(|s| !s.is_empty()),
                );
            }
            _ => {}
        }
    }
    out
}

/// A skill name (letters, digits, `-`) from a capture's folder name.
fn skill_name(capture: &str) -> String {
    let s: String = capture
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let s = s
        .split('-')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if s.is_empty() {
        "recorded-process".into()
    } else {
        s
    }
}

/// A workspace path a fixture can be copied from: `sources/…` or `outputs/…`.
fn copyable(item: &str) -> bool {
    (item.starts_with("sources/") || item.starts_with("outputs/"))
        && !item
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

/// Draft `skill/SKILL.md` and `fixtures/run-1/` from the capture's note.
/// Refuses when the skill is already drafted (an agent's edits stay).
pub fn draft(vault: &Path, workspace: &str, capture: &str) -> Result<Value, String> {
    let (ws_path, folder_rel, name) = locate(vault, workspace, capture)?;
    let ws_abs = crate::workspaces::folder(vault, workspace)?;
    let folder = ws_abs.join(&folder_rel);
    let skill_rel = format!("{ws_path}/{folder_rel}/skill/SKILL.md");
    if crate::vault::safe_join(vault, &skill_rel)?.exists() {
        return Err(format!(
            "{folder_rel}/skill/SKILL.md is already drafted: edit it, or remove skill/ to draft again."
        ));
    }
    let note = std::fs::read_to_string(folder.join("capture.md"))
        .map_err(|e| format!("Couldn't read capture.md: {e}"))?;
    let parsed = parse_capture(&note);
    if parsed.steps.is_empty() {
        return Err(format!(
            "{folder_rel}/capture.md has no steps under \"## Steps\"; nothing to draft from."
        ));
    }
    let skill = skill_name(&name);
    let title = if parsed.title.is_empty() {
        name.clone()
    } else {
        parsed.title.clone()
    };

    // Fixtures first: only files that exist, inside the workspace, not links.
    let mut copied: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    let mut total = 0u64;
    let fixtures_exist = folder.join("fixtures").exists();
    if !fixtures_exist {
        for (items, dir) in [(&parsed.inputs, "input"), (&parsed.outputs, "expected")] {
            for item in items.iter().filter(|i| copyable(i)) {
                // Resolved, it must still be in this workspace: a link in
                // sources/ to another workspace's file isn't this run's.
                let src =
                    crate::vault::safe_join(vault, &format!("{ws_path}/{item}")).and_then(|p| {
                        let inside = p.canonicalize().ok().zip(ws_abs.canonicalize().ok());
                        match inside {
                            Some((c, w))
                                if c.starts_with(&w) && !ws_abs.join(item).is_symlink() =>
                            {
                                Ok(p)
                            }
                            _ => Err("outside the workspace".to_string()),
                        }
                    });
                let meta = src
                    .as_ref()
                    .ok()
                    .and_then(|p| std::fs::symlink_metadata(p).ok());
                let (Ok(src), Some(meta)) = (src, meta) else {
                    missing.push(item.clone());
                    continue;
                };
                if !meta.is_file() {
                    missing.push(format!("{item} (not a file)"));
                    continue;
                }
                if meta.len() > FIXTURE_FILE_MAX || total + meta.len() > FIXTURES_MAX {
                    missing.push(format!(
                        "{item} (too large: fixtures stop at 20 MB a file, 50 MB in all)"
                    ));
                    continue;
                }
                let file = Path::new(item)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let rel = format!("{ws_path}/{folder_rel}/fixtures/run-1/{dir}/{file}");
                let dest = crate::vault::safe_join(vault, &rel)?;
                if dest.exists() {
                    missing.push(format!("{item} (another file is already named {file})"));
                    continue;
                }
                let bytes = std::fs::read(&src).map_err(|e| format!("read {item}: {e}"))?;
                std::fs::create_dir_all(dest.parent().unwrap())
                    .map_err(|e| format!("fixtures folder: {e}"))?;
                crate::vault::atomic_write(&dest, &bytes)?;
                total += meta.len();
                copied.push(format!("fixtures/run-1/{dir}/{file}"));
            }
        }
    }

    let list = |items: &[String]| -> String {
        if items.is_empty() {
            "- [fill in]\n".to_string()
        } else {
            items.iter().map(|i| format!("- {i}\n")).collect()
        }
    };
    let mut body = format!(
        "---\nname: {skill}\ndescription: {description}\n---\n\n\
         # {title}\n\n\
         Drafted by Hollow from `{folder_rel}/capture.md` (no model). Replace every [fill in] and \
         _Describe this step._ line, add `scripts/run.sh` or `scripts/run.py` (called as \
         `run <input folder> <output folder>`) if the work can run as code, then test it with \
         `process.test_skill`.\n\n## Inputs\n\n{inputs}\n## Steps\n\n",
        inputs = list(&parsed.inputs),
        description = crate::frontmatter::quote(&format!(
            "{title} (a recorded process). Use it when [fill in]."
        )),
    );
    for (i, step) in parsed.steps.iter().enumerate() {
        let text = if step.is_empty() || PLACEHOLDERS.iter().any(|p| step.contains(p)) {
            "_Describe this step._"
        } else {
            step.as_str()
        };
        body.push_str(&format!("{}. {text}\n", i + 1));
    }
    body.push_str(&format!("\n## Outputs\n\n{}", list(&parsed.outputs)));
    crate::vault::create_note(vault, &skill_rel, &body)?;
    Ok(json!({
        "skill": skill_rel,
        "name": skill,
        "steps": parsed.steps.len(),
        "fixtures": copied,
        "notCopied": missing,
        "fixturesKept": fixtures_exist,
        "note": "Drafted without a model. Fill in the [fill in] markers and the steps (and a script, if it can run as code), then process.test_skill. Fixtures are copies of this run's files: they stay in the workspace and are never installed with the skill.",
    }))
}

/// Every file under `dirs` (relative to `root`), sorted; refuses links, so
/// a copy into scratch can't follow one out of the vault.
fn files_under(root: &Path, dirs: &[&str]) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for d in dirs {
        let base = root.join(d);
        if !base.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&base).follow_links(false) {
            let entry = entry.map_err(|e| format!("read {d}: {e}"))?;
            let ft = entry.file_type();
            let rel = entry
                .path()
                .strip_prefix(root)
                .unwrap_or(entry.path())
                .to_string_lossy()
                .replace('\\', "/");
            // Finder and git leftovers: staging skips them too, so opening
            // the folder in Finder doesn't make a test stale.
            if rel
                .split('/')
                .any(|p| p == "__MACOSX" || p == ".git" || p == ".DS_Store")
            {
                continue;
            }
            if ft.is_symlink() {
                return Err(format!("{rel} is a link; a tested skill can't hold links."));
            }
            if ft.is_file() {
                out.push(rel);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// SHA-256 over `skill/` and `fixtures/` (each file's path and bytes).
pub fn digest(folder: &Path) -> Result<String, String> {
    let mut h = Sha256::new();
    for rel in files_under(folder, &["skill", "fixtures"])? {
        let bytes = std::fs::read(folder.join(&rel)).map_err(|e| format!("read {rel}: {e}"))?;
        h.update(rel.as_bytes());
        h.update([0]);
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(&bytes);
    }
    Ok(format!("{:x}", h.finalize()))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outcome {
    Pass,
    Fail,
    Skip,
}

impl Outcome {
    fn word(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::Fail => "fail",
            Outcome::Skip => "skipped",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Row {
    pub case: String,
    pub check: String,
    pub outcome: Outcome,
    pub detail: String,
}

fn row(case: &str, check: &str, outcome: Outcome, detail: impl Into<String>) -> Row {
    Row {
        case: case.into(),
        check: check.into(),
        outcome,
        detail: detail.into(),
    }
}

/// The checks on `SKILL.md` itself.
fn skill_rows(skill_md: Option<&str>) -> Vec<Row> {
    let Some(text) = skill_md else {
        return vec![row(
            "skill",
            "SKILL.md",
            Outcome::Fail,
            "skill/SKILL.md is missing; run process.draft_skill",
        )];
    };
    let (description, _, name) = crate::skills::parse_skill_meta(text);
    let name = name.unwrap_or_default();
    let mut rows = vec![
        if !crate::skills::valid_skill_name(&name) || description.trim().is_empty() {
            row(
                "skill",
                "frontmatter",
                Outcome::Fail,
                "needs a name (letters, digits, - and _) and a description",
            )
        } else {
            row(
                "skill",
                "frontmatter",
                Outcome::Pass,
                format!("name {name}"),
            )
        },
    ];
    let left: Vec<&str> = PLACEHOLDERS
        .iter()
        .copied()
        .filter(|p| text.contains(p))
        .collect();
    rows.push(if left.is_empty() {
        row("skill", "no placeholders", Outcome::Pass, "")
    } else {
        row(
            "skill",
            "no placeholders",
            Outcome::Fail,
            format!("still has {}", left.join(", ")),
        )
    });
    rows
}

/// The skill's entry point, if it has one.
fn entry_point(skill: &Path) -> Option<&'static str> {
    ENTRY_POINTS
        .iter()
        .copied()
        .find(|e| skill.join(e).is_file())
}

fn python() -> Option<PathBuf> {
    [
        // Homebrew's first: /usr/bin's is the Command Line Tools stub.
        "/opt/homebrew/bin/python3",
        "/usr/local/bin/python3",
        "/usr/bin/python3",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
    for rel in files_under(from, &[""])? {
        let dest = to.join(&rel);
        std::fs::create_dir_all(dest.parent().unwrap()).map_err(|e| format!("scratch: {e}"))?;
        std::fs::copy(from.join(&rel), &dest).map_err(|e| format!("copy {rel}: {e}"))?;
    }
    Ok(())
}

/// Text compares without CRLF or trailing-space differences (per line, and
/// trailing blank lines); anything
/// else byte for byte. The first differing line, when text.
fn differs(expected: &[u8], got: &[u8]) -> Option<String> {
    match (std::str::from_utf8(expected), std::str::from_utf8(got)) {
        (Ok(e), Ok(g)) => {
            let norm = |s: &str| {
                s.lines()
                    .map(str::trim_end)
                    .collect::<Vec<_>>()
                    .join("\n")
                    .trim_end()
                    .to_string()
            };
            let (e, g) = (norm(e), norm(g));
            if e == g {
                return None;
            }
            let line = e
                .lines()
                .zip(g.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| e.lines().count().min(g.lines().count()));
            Some(format!("differs at line {}", line + 1))
        }
        _ => (expected != got).then(|| "differs (binary)".to_string()),
    }
}

/// Run one fixture case in `scratch`: the skill copied in, its input copied
/// in, the entry point called with the input and an empty output folder.
fn run_case(
    skill: &Path,
    case_dir: &Path,
    case: &str,
    entry: &str,
    scratch: &Path,
    stop: &dyn Fn() -> bool,
) -> Row {
    let check = format!("runs {entry}");
    let work = scratch.join(case);
    let (skill_copy, input, output, tmp) = (
        work.join("skill"),
        work.join("input"),
        work.join("output"),
        work.join("tmp"),
    );
    let setup = (|| -> Result<(), String> {
        copy_tree(skill, &skill_copy)?;
        std::fs::create_dir_all(&input).map_err(|e| e.to_string())?;
        if case_dir.join("input").is_dir() {
            copy_tree(&case_dir.join("input"), &input)?;
        }
        std::fs::create_dir_all(&output).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())
    })();
    if let Err(e) = setup {
        return row(
            case,
            &check,
            Outcome::Fail,
            format!("couldn't set up the scratch copy: {e}"),
        );
    }
    let script = skill_copy.join(entry);
    let mut cmd = match entry.rsplit('.').next() {
        Some("sh") => {
            let mut c = std::process::Command::new("/bin/sh");
            c.arg(&script);
            c
        }
        Some("py") => match python() {
            Some(p) => {
                let mut c = std::process::Command::new(p);
                c.arg(&script);
                c
            }
            None => return row(case, &check, Outcome::Fail, "python3 isn't installed"),
        },
        _ => std::process::Command::new(&script),
    };
    cmd.arg(&input)
        .arg(&output)
        .current_dir(&skill_copy)
        .env_clear()
        .env(
            "PATH",
            "/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/usr/local/bin",
        )
        .env("HOME", &work)
        .env("TMPDIR", &tmp)
        .env("LANG", "en_US.UTF-8");
    let started = Instant::now();
    let (ok, _, err) = match crate::process_capture::run_bounded_until(&mut cmd, CASE_TIMEOUT, stop)
    {
        Ok(r) => r,
        Err(e) => return row(case, &check, Outcome::Fail, format!("the script {e}")),
    };
    let secs = started.elapsed().as_secs_f64();
    if !ok {
        let tail: String = err
            .lines()
            .rev()
            .take(3)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" ");
        return row(
            case,
            &check,
            Outcome::Fail,
            format!("exited with an error: {}", tail.trim()),
        );
    }
    let expected_dir = case_dir.join("expected");
    let expected = match files_under(&expected_dir, &[""]) {
        Ok(f) if expected_dir.is_dir() => f,
        Ok(_) => Vec::new(),
        Err(e) => return row(case, &check, Outcome::Fail, e),
    };
    if expected.is_empty() {
        return row(
            case,
            &check,
            Outcome::Skip,
            format!("ran in {secs:.1} s, but it has no expected/ files to compare"),
        );
    }
    let mut problems = Vec::new();
    for rel in &expected {
        let want = std::fs::read(expected_dir.join(rel)).unwrap_or_default();
        match std::fs::read(output.join(rel)) {
            Ok(got) => {
                if let Some(d) = differs(&want, &got) {
                    problems.push(format!("{rel} {d}"));
                }
            }
            Err(_) => problems.push(format!("{rel} wasn't written")),
        }
    }
    if problems.is_empty() {
        row(
            case,
            &check,
            Outcome::Pass,
            format!(
                "ran in {secs:.1} s; {} expected file(s) match",
                expected.len()
            ),
        )
    } else {
        row(case, &check, Outcome::Fail, problems.join("; "))
    }
}

/// The fixture cases: `fixtures/<case>/` folders, sorted.
fn cases_in(folder: &Path) -> Vec<String> {
    let mut cases: Vec<String> = std::fs::read_dir(folder.join("fixtures"))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .filter(|n| !n.starts_with('.'))
                .collect()
        })
        .unwrap_or_default();
    cases.sort();
    cases
}

/// What [`prepare_test`] would run, for its card.
pub struct TestPlan {
    pub capture: String,
    pub has_skill_md: bool,
    pub entry: Option<&'static str>,
    /// The entry point's size and the start of its SHA-256.
    pub script: Option<(u64, String)>,
    pub cases: Vec<String>,
}

pub fn test_plan(vault: &Path, workspace: &str, capture: &str) -> Result<TestPlan, String> {
    let (_, folder_rel, name) = locate(vault, workspace, capture)?;
    let folder = crate::workspaces::folder(vault, workspace)?.join(&folder_rel);
    let skill = folder.join("skill");
    let entry = entry_point(&skill);
    let script = entry
        .and_then(|e| std::fs::read(skill.join(e)).ok())
        .map(|b| {
            let hex = format!("{:x}", Sha256::digest(&b));
            (b.len() as u64, hex[..12].to_string())
        });
    Ok(TestPlan {
        capture: name,
        has_skill_md: skill.join("SKILL.md").is_file(),
        entry,
        script,
        cases: cases_in(&folder).into_iter().take(MAX_CASES).collect(),
    })
}

/// A test ready to write: the cases already ran in a temp folder outside
/// the vault with no lock held; [`commit_test`] writes the table.
pub struct PreparedTest {
    note_rel: String,
    note: String,
    key: String,
    record: Record,
    summary: Value,
}

/// What Save trusts about a test, kept in app data (not the vault).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Record {
    digest: String,
    passed: u64,
    failed: u64,
    skipped: u64,
    tested: String,
}

const RECORDS_KEPT: usize = 500;
static RECORDS_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
/// Tests run in this process so far: part of each one's scratch name.
static SCRATCH_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A test run's scratch folder name, unique in this process (overnight 9
/// A4). Pid plus nanoseconds alone repeats when two runs start on one clock
/// tick, and the first to finish removes the folder the other still uses:
/// the suspected cause of the "tips.csv wasn't written" load flake (fix-up
/// 10-17 item 8e; a hypothesis, never reproduced).
fn scratch_name() -> String {
    format!(
        "hollow-skill-test-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
        SCRATCH_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

fn records_path() -> Result<PathBuf, String> {
    Ok(crate::agent_settings::app_data_dir()?.join("process-skill-tests.json"))
}

fn record_key(vault: &Path, ws_path: &str, folder_rel: &str) -> String {
    format!("{}::{ws_path}/{folder_rel}", vault.display())
}

fn read_records(path: &Path) -> BTreeMap<String, Record> {
    // Unreadable means no record: the cost is a retest, never a Save.
    std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

fn save_record(key: &str, record: &Record) -> Result<(), String> {
    let _held = RECORDS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let path = records_path()?;
    let mut all = read_records(&path);
    all.insert(key.to_string(), record.clone());
    while all.len() > RECORDS_KEPT {
        let oldest = all
            .iter()
            .min_by(|a, b| a.1.tested.cmp(&b.1.tested))
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => all.remove(&k),
            None => break,
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("app data: {e}"))?;
    }
    let bytes = serde_json::to_vec(&all).map_err(|e| e.to_string())?;
    crate::vault::atomic_write_private(&path, &bytes)
}

fn load_record(key: &str) -> Option<Record> {
    let _held = RECORDS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    read_records(&records_path().ok()?).remove(key)
}

/// Check `SKILL.md`, run every fixture case in scratch, and draft the
/// results note. Nothing is written in the vault. `stop()` turning true (the
/// run was stopped or its turn ended) ends the case running and records
/// nothing.
pub fn prepare_test(
    vault: &Path,
    workspace: &str,
    capture: &str,
    stop: &dyn Fn() -> bool,
) -> Result<PreparedTest, String> {
    let (ws_path, folder_rel, name) = locate(vault, workspace, capture)?;
    let folder = crate::workspaces::folder(vault, workspace)?.join(&folder_rel);
    let before = digest(&folder)?;
    let skill = folder.join("skill");
    let skill_md = std::fs::read_to_string(skill.join("SKILL.md")).ok();
    let mut rows = skill_rows(skill_md.as_deref());
    let entry = entry_point(&skill);
    let mut cases = cases_in(&folder);
    if cases.len() > MAX_CASES {
        rows.push(row(
            "fixtures",
            "case count",
            Outcome::Fail,
            format!(
                "{} cases; at most {MAX_CASES} run, so this fails until there are fewer",
                cases.len()
            ),
        ));
        cases.truncate(MAX_CASES);
    }
    match (entry, skill_md.is_some()) {
        (_, false) => {}
        (None, _) => {
            rows.push(row(
                "skill",
                "script",
                Outcome::Skip,
                "instructions only: no scripts/run.sh, run.py or run",
            ));
            for c in &cases {
                rows.push(row(
                    c,
                    "runs",
                    Outcome::Skip,
                    "not run: the skill has no script",
                ));
            }
        }
        (Some(e), _) if cases.is_empty() => {
            rows.push(row(
                "fixtures",
                &format!("runs {e}"),
                Outcome::Fail,
                "a skill with a script needs at least one fixtures/<case>/ to run against",
            ));
        }
        (Some(e), _) => {
            let scratch = std::env::temp_dir().join(scratch_name());
            for c in &cases {
                if stop() {
                    break;
                }
                rows.push(run_case(
                    &skill,
                    &folder.join("fixtures").join(c),
                    c,
                    e,
                    &scratch,
                    stop,
                ));
            }
            let _ = std::fs::remove_dir_all(&scratch);
            if stop() {
                return Err("The test was stopped; nothing was recorded.".into());
            }
        }
    }
    // The script ran on copies, but tested bytes must be the ones recorded.
    if digest(&folder)? != before {
        return Err("The skill or its fixtures changed while the test ran; nothing was recorded. Run it again.".into());
    }
    let count = |o: Outcome| rows.iter().filter(|r| r.outcome == o).count();
    let (passed, failed, skipped) = (
        count(Outcome::Pass),
        count(Outcome::Fail),
        count(Outcome::Skip),
    );
    let tested = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let mut note = format!(
        "---\npassed: {passed}\nfailed: {failed}\nskipped: {skipped}\ntested: {tested}\n---\n\n\
         # Skill test: {name}\n\n\
         {verdict}\n\n| Case | Check | Result | Detail |\n|---|---|---|---|\n",
        verdict = if failed > 0 {
            format!("**{failed} failed.** A failure blocks Save; fix the skill or its fixtures and test again.")
        } else {
            format!("No failures ({passed} passed, {skipped} skipped). `process.save_skill` can save these exact files.")
        },
    );
    for r in &rows {
        note.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            cell(&r.case),
            cell(&r.check),
            r.outcome.word(),
            cell(&r.detail)
        ));
    }
    note.push_str(
        "\nThe script ran on copies in a temp folder outside the vault, with this Mac's permissions (the network \
         isn't blocked). Editing anything under skill/ or fixtures/ makes this result stale. This note is for reading: \
         Save checks Hollow's own record of the test, kept outside the vault.\n",
    );
    let note_rel = format!("{ws_path}/{folder_rel}/test-results.md");
    let summary = json!({
        "results": note_rel,
        "passed": passed,
        "failed": failed,
        "skipped": skipped,
        "rows": rows.iter().map(|r| json!({"case": r.case, "check": r.check, "result": r.outcome.word(), "detail": r.detail})).collect::<Vec<_>>(),
        "canSave": failed == 0,
    });
    Ok(PreparedTest {
        note_rel,
        note,
        key: record_key(vault, &ws_path, &folder_rel),
        record: Record {
            digest: before,
            passed: passed as u64,
            failed: failed as u64,
            skipped: skipped as u64,
            tested,
        },
        summary,
    })
}

/// Write the results note (inside the caller's History lease) and Hollow's
/// record of the test.
pub fn commit_test(vault: &Path, prepared: &PreparedTest) -> Result<Value, String> {
    crate::vault::write_note(vault, &prepared.note_rel, &prepared.note)?;
    save_record(&prepared.key, &prepared.record)?;
    Ok(prepared.summary.clone())
}

/// What the save card shows, once the last test allows Save.
#[derive(Debug)]
pub struct SaveReady {
    pub skill_dir: PathBuf,
    pub digest: String,
    pub passed: u64,
    pub skipped: u64,
    pub tested: String,
    pub folder_rel: String,
}

/// Save only what was tested: Hollow's record of the last test (app data,
/// not the vault note) has no failure and its digest matches `skill/` and
/// `fixtures/` now.
pub fn save_check(vault: &Path, workspace: &str, capture: &str) -> Result<SaveReady, String> {
    let (ws_path, folder_rel, _) = locate(vault, workspace, capture)?;
    let folder = crate::workspaces::folder(vault, workspace)?.join(&folder_rel);
    let Some(record) = load_record(&record_key(vault, &ws_path, &folder_rel)) else {
        return Err(format!(
            "{folder_rel} hasn't been tested: run process.test_skill first. Nothing changed."
        ));
    };
    if record.failed > 0 {
        return Err(format!("{} check(s) failed in the last test ({folder_rel}/test-results.md); a failure blocks Save. Nothing changed.", record.failed));
    }
    let now = digest(&folder)?;
    if now != record.digest {
        return Err("skill/ or fixtures/ changed since the last test; run process.test_skill again. Nothing changed.".into());
    }
    Ok(SaveReady {
        skill_dir: folder.join("skill"),
        digest: now,
        passed: record.passed,
        skipped: record.skipped,
        tested: record.tested,
        folder_rel,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Overnight 9 A4: two runs that start on one clock tick still get
    /// folders of their own.
    #[test]
    fn two_scratch_names_minted_back_to_back_differ() {
        let names: Vec<String> = (0..64).map(|_| scratch_name()).collect();
        let distinct: std::collections::BTreeSet<&String> = names.iter().collect();
        assert_eq!(distinct.len(), names.len());
        let pid = format!("hollow-skill-test-{}-", std::process::id());
        assert!(names.iter().all(|n| n.starts_with(&pid)));
    }

    fn setup(capture_md: &str) -> (tempfile::TempDir, String, PathBuf) {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let w = crate::workspaces::create(dir.path(), "TipFlow", "x").unwrap();
        let ws = crate::workspaces::folder(dir.path(), &w.id).unwrap();
        let cap = ws.join("outputs/process-captures/tip-run");
        std::fs::create_dir_all(&cap).unwrap();
        std::fs::write(cap.join("capture.md"), capture_md).unwrap();
        std::fs::create_dir_all(ws.join("sources")).unwrap();
        (dir, w.id, cap)
    }

    const NOTE: &str = "# Process capture: tip run\n\nRecording: x\n\n## Steps (draft)\n\n\
        1. **Step 1 (0:01).** Open the push file.\n   ![step 1](frames/frame-001.jpg)\n\
        2. **Step 2 (0:25).** _What happens here?_\n3. Then total the tips.\n\n\
        ## Inputs and outputs (to fill in)\n\n- Inputs: `sources/push.csv`, sources/missing.csv, the HRIS report\n\
        - Outputs:\n  - outputs/tips.csv\n";

    #[test]
    fn the_capture_note_parses_as_the_person_edited_it() {
        let p = parse_capture(NOTE);
        assert_eq!(p.title, "tip run");
        assert_eq!(
            p.steps,
            [
                "Open the push file.",
                "_What happens here?_",
                "Then total the tips."
            ]
        );
        assert_eq!(
            p.inputs,
            ["sources/push.csv", "sources/missing.csv", "the HRIS report"]
        );
        assert_eq!(p.outputs, ["outputs/tips.csv"]);
        assert_eq!(skill_name("Tip run 2!"), "tip-run-2");
        assert_eq!(skill_name("ü—ï"), "recorded-process");
        assert!(!copyable("sources/../../x"));
        assert!(!copyable("/etc/passwd"));
    }

    #[test]
    fn a_draft_writes_a_skeleton_and_copies_only_existing_fixtures() {
        let (dir, id, cap) = setup(NOTE);
        let ws = cap
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        std::fs::write(ws.join("sources/push.csv"), "a,1\n").unwrap();
        std::fs::create_dir_all(ws.join("outputs")).unwrap();
        std::fs::write(ws.join("outputs/tips.csv"), "total,1\n").unwrap();
        let out = draft(dir.path(), &id, "tip-run").unwrap();
        assert_eq!(out["name"], "tip-run");
        assert_eq!(
            out["fixtures"],
            json!([
                "fixtures/run-1/input/push.csv",
                "fixtures/run-1/expected/tips.csv"
            ])
        );
        assert_eq!(out["notCopied"], json!(["sources/missing.csv"]));
        let md = std::fs::read_to_string(cap.join("skill/SKILL.md")).unwrap();
        assert!(md.starts_with("---\nname: tip-run\n"), "{md}");
        assert!(
            md.contains(
                "1. Open the push file.\n2. _Describe this step._\n3. Then total the tips.\n"
            ),
            "{md}"
        );
        assert!(md.contains("- the HRIS report\n"));
        // An agent's edits are never overwritten by a second draft.
        let err = draft(
            dir.path(),
            &id,
            "outputs/process-captures/tip-run/capture.md",
        )
        .unwrap_err();
        assert!(err.contains("already drafted"), "{err}");
        assert!(draft(dir.path(), &id, "../x")
            .unwrap_err()
            .contains("isn't a capture"));
        // A title with a colon still makes valid frontmatter (quoted).
        assert!(
            md.contains("description: \"tip run (a recorded process). Use it when [fill in].\""),
            "{md}"
        );
        assert!(draft(dir.path(), &id, "nope")
            .unwrap_err()
            .contains("process.capture first"));
    }

    #[test]
    fn a_fresh_draft_fails_its_test_and_cant_be_saved() {
        let (dir, id, _) = setup(NOTE);
        draft(dir.path(), &id, "tip-run").unwrap();
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        commit_test(dir.path(), &t).unwrap();
        assert_eq!(t.summary["canSave"], false);
        assert!(
            t.note.contains("| skill | no placeholders | fail |"),
            "{}",
            t.note
        );
        let err = save_check(dir.path(), &id, "tip-run").unwrap_err();
        assert!(err.contains("blocks Save"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn scripts_run_on_fixture_copies_and_a_failure_or_an_edit_blocks_save() {
        let (dir, id, cap) = setup(NOTE);
        let skill = cap.join("skill");
        std::fs::create_dir_all(skill.join("scripts")).unwrap();
        std::fs::write(
            skill.join("SKILL.md"),
            "---\nname: tip-run\ndescription: Total TipFlow tips.\n---\n\n1. Run it.\n",
        )
        .unwrap();
        // Totals the second column of every input CSV into output/tips.csv.
        std::fs::write(
            skill.join("scripts/run.sh"),
            "#!/bin/sh\nset -e\nsum=0\nfor f in \"$1\"/*.csv; do while IFS=, read a b; do sum=$((sum+b)); done < \"$f\"; done\nprintf 'total,%s\\r\\n' \"$sum\" > \"$2/tips.csv\"\n",
        )
        .unwrap();
        for (case, input, want) in [
            ("a", "x,1\ny,2\n", "total,3\n"),
            ("b", "x,5\n", "total,5\n"),
        ] {
            std::fs::create_dir_all(cap.join(format!("fixtures/{case}/input"))).unwrap();
            std::fs::create_dir_all(cap.join(format!("fixtures/{case}/expected"))).unwrap();
            std::fs::write(cap.join(format!("fixtures/{case}/input/push.csv")), input).unwrap();
            std::fs::write(cap.join(format!("fixtures/{case}/expected/tips.csv")), want).unwrap();
        }
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        assert_eq!(
            (t.summary["passed"].as_u64(), t.summary["failed"].as_u64()),
            (Some(4), Some(0)),
            "{}",
            t.note
        );
        commit_test(dir.path(), &t).unwrap();
        let ready = save_check(dir.path(), &id, "tip-run").unwrap();
        assert_eq!(ready.passed, 4);
        assert!(ready.skill_dir.ends_with("tip-run/skill"));
        // The script's writes stayed in scratch: input untouched, no output beside it.
        assert_eq!(
            std::fs::read_to_string(cap.join("fixtures/a/input/push.csv")).unwrap(),
            "x,1\ny,2\n"
        );
        assert!(!cap.join("fixtures/a/output").exists());

        // Any edit after the test makes it stale.
        std::fs::write(cap.join("fixtures/b/expected/tips.csv"), "total,6\n").unwrap();
        assert!(save_check(dir.path(), &id, "tip-run")
            .unwrap_err()
            .contains("changed since the last test"));
        // Retesting shows the failing case, and the failure blocks Save.
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        commit_test(dir.path(), &t).unwrap();
        assert!(
            t.note
                .contains("| b | runs scripts/run.sh | fail | tips.csv differs at line 1 |"),
            "{}",
            t.note
        );
        assert!(save_check(dir.path(), &id, "tip-run")
            .unwrap_err()
            .contains("1 check(s) failed"));
        // Editing the note can't unlock Save: Hollow's own record decides.
        let note = std::fs::read_to_string(cap.join("test-results.md")).unwrap();
        std::fs::write(
            cap.join("test-results.md"),
            note.replace("failed: 1", "failed: 0"),
        )
        .unwrap();
        assert!(save_check(dir.path(), &id, "tip-run")
            .unwrap_err()
            .contains("1 check(s) failed"));
        // Stop (or the turn ending) ends the test and records nothing.
        let err = prepare_test(dir.path(), &id, "tip-run", &|| true)
            .err()
            .unwrap();
        assert!(err.contains("stopped"), "{err}");
        // A script that errors says so.
        std::fs::write(
            skill.join("scripts/run.sh"),
            "#!/bin/sh\necho 'no such column' >&2\nexit 3\n",
        )
        .unwrap();
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        assert!(
            t.note.contains("exited with an error: no such column"),
            "{}",
            t.note
        );
    }

    #[cfg(unix)]
    #[test]
    fn instructions_only_skills_skip_their_cases_and_links_are_refused() {
        let (dir, id, cap) = setup(NOTE);
        std::fs::create_dir_all(cap.join("skill")).unwrap();
        std::fs::write(
            cap.join("skill/SKILL.md"),
            "---\nname: tip-run\ndescription: Total tips by hand.\n---\n\n1. Do it.\n",
        )
        .unwrap();
        std::fs::create_dir_all(cap.join("fixtures/run-1/input")).unwrap();
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        assert_eq!(
            (t.summary["failed"].as_u64(), t.summary["skipped"].as_u64()),
            (Some(0), Some(2)),
            "{}",
            t.note
        );
        assert!(t
            .note
            .contains("| run-1 | runs | skipped | not run: the skill has no script |"));
        // A script with no fixture case to run against fails.
        std::fs::create_dir_all(cap.join("skill/scripts")).unwrap();
        std::fs::write(cap.join("skill/scripts/run.py"), "print(1)\n").unwrap();
        std::fs::remove_dir_all(cap.join("fixtures")).unwrap();
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        assert!(
            t.note.contains("needs at least one fixtures/<case>/"),
            "{}",
            t.note
        );
        // A link in the skill could carry a file from outside the vault.
        std::os::unix::fs::symlink("/etc/hosts", cap.join("skill/hosts")).unwrap();
        assert!(prepare_test(dir.path(), &id, "tip-run", &|| false)
            .err()
            .unwrap()
            .contains("is a link"));
        assert!(differs(b"a\r\nb\n", b"a\nb").is_none());
        assert!(
            differs(b"a  \nb\n\n", b"a\nb").is_none(),
            "trailing spaces per line"
        );
        assert_eq!(
            differs(b"a\nb\n", b"a\nc\n").as_deref(),
            Some("differs at line 2")
        );
        assert_eq!(
            differs(&[0xff, 1], &[0xff, 2]).as_deref(),
            Some("differs (binary)")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_sources_to_another_workspace_isnt_copied_and_no_expected_is_skipped() {
        let (dir, id, cap) = setup(NOTE);
        let ws = cap
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let other = crate::workspaces::create(dir.path(), "Payroll", "x").unwrap();
        let other_ws = crate::workspaces::folder(dir.path(), &other.id).unwrap();
        std::fs::write(other_ws.join("secret.csv"), "ssn,1\n").unwrap();
        std::os::unix::fs::symlink(other_ws.join("secret.csv"), ws.join("sources/push.csv"))
            .unwrap();
        let out = draft(dir.path(), &id, "tip-run").unwrap();
        assert_eq!(out["fixtures"], json!([]));
        assert!(!cap.join("fixtures/run-1/input/push.csv").exists());
        // Finder's leftovers don't change the digest.
        let before = digest(&cap).unwrap();
        std::fs::write(cap.join("skill/.DS_Store"), "x").unwrap();
        assert_eq!(digest(&cap).unwrap(), before);
        // A script whose case has no expected/ ran, but nothing was compared.
        std::fs::write(
            cap.join("skill/SKILL.md"),
            "---\nname: tip-run\ndescription: x\n---\n\n1. Go.\n",
        )
        .unwrap();
        std::fs::create_dir_all(cap.join("skill/scripts")).unwrap();
        std::fs::write(cap.join("skill/scripts/run.sh"), "exit 0\n").unwrap();
        std::fs::create_dir_all(cap.join("fixtures/c/input")).unwrap();
        let t = prepare_test(dir.path(), &id, "tip-run", &|| false).unwrap();
        assert!(
            t.note.contains("| c | runs scripts/run.sh | skipped |"),
            "{}",
            t.note
        );
        let plan = test_plan(dir.path(), &id, "tip-run").unwrap();
        assert_eq!(plan.entry, Some("scripts/run.sh"));
        assert_eq!(plan.script.as_ref().map(|s| s.0), Some(7));
        assert!(plan.has_skill_md);
    }
}
