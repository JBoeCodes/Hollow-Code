//! CX19 tests: the classifier by path, the scan over sanitized fixtures in
//! temp dirs (no test reads `$HOME`), links, budgets, secrets never copied,
//! duplicates, cancel, staging outside the folder scanned.

use super::*;
use std::sync::atomic::AtomicBool;

fn v(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

fn write(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A secret-shaped token built at run time, so no secret-looking text sits in
/// the source.
fn fake_key() -> String {
    format!("{}{}", "sk-ant-", "a1B2c3D4e5F6g7H8i9J0k1L2m3N4")
}

struct Rig {
    tmp: tempfile::TempDir,
    staging: Staging,
}

impl Rig {
    fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let staging = Staging::new(tmp.path().join("app-support").join("Imports"));
        Rig { tmp, staging }
    }
    fn dir(&self, name: &str) -> PathBuf {
        let d = self.tmp.path().join(name);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn run(&self, roots: &[ScanRoot]) -> Result<ScanManifest, String> {
        self.run_with(roots, &ScanBudget::default(), &AtomicBool::new(false))
    }
    fn run_with(
        &self,
        roots: &[ScanRoot],
        budget: &ScanBudget,
        cancel: &AtomicBool,
    ) -> Result<ScanManifest, String> {
        scan(
            &self.staging,
            "imp-20261119-00000001",
            roots,
            &[],
            budget,
            cancel,
        )
    }
}

fn project(path: PathBuf) -> ScanRoot {
    ScanRoot {
        path,
        scope: Scope::Project,
        label: "demo".into(),
        agent: None,
    }
}

#[test]
fn classifier_knows_the_instruction_and_memory_files() {
    let c = |p: &[&str], hint| classify(&v(p), hint);
    assert_eq!(
        c(&["CLAUDE.md"], None),
        Some((Agent::ClaudeCode, FileKind::Instructions))
    );
    assert_eq!(
        c(&["sub", "AGENTS.md"], None),
        Some((Agent::Generic, FileKind::Instructions))
    );
    assert_eq!(
        c(&["AGENTS.md"], Some(Agent::Codex)),
        Some((Agent::Codex, FileKind::Instructions))
    );
    assert_eq!(
        c(&["GEMINI.md"], None),
        Some((Agent::Gemini, FileKind::Instructions))
    );
    assert_eq!(
        c(&[".cursorrules"], None),
        Some((Agent::Cursor, FileKind::Rule))
    );
    assert_eq!(
        c(&[".cursor", "rules", "a.mdc"], None),
        Some((Agent::Cursor, FileKind::Rule))
    );
    assert_eq!(
        c(&["rules", "a.mdc"], Some(Agent::Cursor)),
        Some((Agent::Cursor, FileKind::Rule))
    );
    assert_eq!(
        c(&[".windsurfrules"], None),
        Some((Agent::Windsurf, FileKind::Rule))
    );
    assert_eq!(
        c(&[".github", "copilot-instructions.md"], None),
        Some((Agent::Copilot, FileKind::Instructions))
    );
    assert_eq!(
        c(&[".github", "instructions", "ts.instructions.md"], None),
        Some((Agent::Copilot, FileKind::Instructions))
    );
    assert_eq!(
        c(&[".claude", "commands", "ship.md"], None),
        Some((Agent::ClaudeCode, FileKind::Command))
    );
    assert_eq!(
        c(&["prompts", "review.md"], Some(Agent::Codex)),
        Some((Agent::Codex, FileKind::Command))
    );
    assert_eq!(
        c(&["skills", "pdf", "SKILL.md"], Some(Agent::ClaudeCode)),
        Some((Agent::ClaudeCode, FileKind::Skill))
    );
    assert_eq!(
        c(
            &["projects", "-Users-x-demo", "memory", "MEMORY.md"],
            Some(Agent::ClaudeCode)
        ),
        Some((Agent::ClaudeCode, FileKind::Memory))
    );
    // Not instruction files.
    assert_eq!(c(&["README.md"], None), None);
    assert_eq!(c(&["commands", "x.md"], None), None);
    assert_eq!(
        c(&["skills", "pdf", "notes.md"], Some(Agent::ClaudeCode)),
        None
    );
    assert_eq!(c(&["projects", "s.jsonl"], Some(Agent::ClaudeCode)), None);
    assert_eq!(c(&["rules", "a.md"], None), None);
}

#[test]
fn scan_proposes_files_with_kind_hash_excerpt_and_scope() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(
        &root,
        "CLAUDE.md",
        "# Demo\n\nUse tabs. Run the tests before a commit.\n",
    );
    write(&root, "pkg/AGENTS.md", "Agents read this.\n");
    write(
        &root,
        ".cursor/rules/style.mdc",
        "Prefer small functions.\n",
    );
    write(&root, "README.md", "not an instruction file\n");
    let m = rig.run(&[project(root.clone())]).unwrap();
    let rels: Vec<&str> = m.proposals.iter().map(|p| p.rel_path.as_str()).collect();
    assert_eq!(
        rels,
        [".cursor/rules/style.mdc", "CLAUDE.md", "pkg/AGENTS.md"]
    );
    let claude = m
        .proposals
        .iter()
        .find(|p| p.rel_path == "CLAUDE.md")
        .unwrap();
    assert_eq!(claude.kind, FileKind::Instructions);
    assert_eq!(claude.agent, Agent::ClaudeCode);
    assert_eq!(claude.scope, Scope::Project);
    assert_eq!(claude.hash.as_ref().unwrap().len(), 64);
    assert!(claude
        .excerpt
        .as_ref()
        .unwrap()
        .starts_with("# Demo Use tabs."));
    assert!(claude.id.starts_with("ctx-"));
    // Same content, same id; staged outside the scanned folder.
    let again = rig.run(&[project(root.clone())]).unwrap();
    assert_eq!(again.proposals[1].id, claude.id);
    let staged: ScanManifest = manifest(&rig.staging, "imp-20261119-00000001").unwrap();
    assert_eq!(staged.proposals.len(), 3);
    assert!(!root.join("imp-20261119-00000001").exists());
}

#[test]
fn a_secret_marks_the_file_and_is_never_copied() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    let body = format!("# Notes\nkey = {}\n", fake_key());
    write(&root, "CLAUDE.md", &body);
    write(&root, "AGENTS.md", "Clean file about tests.\n");
    let m = rig.run(&[project(root)]).unwrap();
    assert_eq!(m.secret_files, 1);
    let bad = m
        .proposals
        .iter()
        .find(|p| p.rel_path == "CLAUDE.md")
        .unwrap();
    assert!(bad.secret_like);
    assert_eq!(bad.excerpt, None);
    assert_eq!(bad.withheld.as_deref(), Some("secret"));
    assert!(bad
        .flags
        .iter()
        .any(|f| f.rule == "secret.anthropic" && f.line == 2));
    // The key text is nowhere in what the scan returns or stages.
    let json = serde_json::to_string(&m).unwrap();
    assert!(!json.contains(&fake_key()));
    assert!(!json.contains("a1B2c3D4"));
    let staged = std::fs::read_to_string(
        rig.tmp
            .path()
            .join("app-support/Imports/imp-20261119-00000001")
            .join(MANIFEST),
    )
    .unwrap();
    assert!(!staged.contains("a1B2c3D4"));
    let clean = m
        .proposals
        .iter()
        .find(|p| p.rel_path == "AGENTS.md")
        .unwrap();
    assert!(!clean.secret_like && clean.excerpt.is_some());
}

#[test]
fn an_allow_line_cannot_hide_a_secret() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    let body = format!(
        "# context-scan: allow secret.anthropic reason here\nk: {}\n",
        fake_key()
    );
    write(&root, "CLAUDE.md", &body);
    let m = rig.run(&[project(root)]).unwrap();
    assert!(m.proposals[0].secret_like);
}

#[test]
fn links_are_never_followed_and_a_link_root_is_refused() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    let outside = rig.dir("outside");
    write(&outside, "CLAUDE.md", "outside the root\n");
    write(&root, "CLAUDE.md", "inside\n");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("linked")).unwrap();
        std::os::unix::fs::symlink(outside.join("CLAUDE.md"), root.join("AGENTS.md")).unwrap();
        let linkroot = rig.tmp.path().join("linkroot");
        std::os::unix::fs::symlink(&root, &linkroot).unwrap();
        let m = rig
            .run(&[project(root.clone()), project(linkroot)])
            .unwrap();
        assert_eq!(m.proposals.len(), 1);
        assert_eq!(m.roots[0].symlinks_skipped, 2);
        assert!(m.roots[1].note.as_ref().unwrap().contains("link"));
        assert_eq!(m.roots[1].found, 0);
    }
}

#[test]
fn dependency_and_cache_folders_are_not_entered() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(&root, "node_modules/x/CLAUDE.md", "dep\n");
    write(&root, ".git/AGENTS.md", "git\n");
    write(&root, "plugins/p/skills/a/SKILL.md", "plugin\n");
    write(&root, "AGENTS.md", "mine\n");
    let m = rig.run(&[project(root)]).unwrap();
    assert_eq!(m.proposals.len(), 1);
}

#[test]
fn user_roots_find_memory_commands_and_skills_by_the_root_hint() {
    let rig = Rig::new();
    let home = rig.dir("home");
    write(&home, ".claude/CLAUDE.md", "global rules\n");
    write(&home, ".claude/commands/ship.md", "Ship it\n");
    write(
        &home,
        ".claude/skills/pdf/SKILL.md",
        "---\nname: pdf\n---\n",
    );
    write(
        &home,
        ".claude/projects/-Users-x-demo/memory/MEMORY.md",
        "- a memory\n",
    );
    write(&home, ".claude/projects/-Users-x-demo/s.jsonl", "{}\n");
    write(&home, ".codex/AGENTS.md", "codex rules\n");
    write(&home, ".codex/prompts/r.md", "review\n");
    write(&home, ".cursor/rules/a.mdc", "rule\n");
    write(&home, "elsewhere/CLAUDE.md", "not under an agent folder\n");
    let roots = default_roots(&home, None);
    assert_eq!(roots.len(), 5);
    let m = rig.run(&roots).unwrap();
    let kinds: Vec<(String, Agent, FileKind)> = m
        .proposals
        .iter()
        .map(|p| (p.rel_path.clone(), p.agent, p.kind))
        .collect();
    assert_eq!(kinds.len(), 8 - 1);
    assert!(kinds.contains(&(
        "CLAUDE.md".into(),
        Agent::ClaudeCode,
        FileKind::Instructions
    )));
    assert!(kinds.contains(&(
        "commands/ship.md".into(),
        Agent::ClaudeCode,
        FileKind::Command
    )));
    assert!(kinds.contains(&(
        "skills/pdf/SKILL.md".into(),
        Agent::ClaudeCode,
        FileKind::Skill
    )));
    assert!(kinds.contains(&(
        "projects/-Users-x-demo/memory/MEMORY.md".into(),
        Agent::ClaudeCode,
        FileKind::Memory
    )));
    assert!(kinds.contains(&("AGENTS.md".into(), Agent::Codex, FileKind::Instructions)));
    assert!(kinds.contains(&("prompts/r.md".into(), Agent::Codex, FileKind::Command)));
    assert!(kinds.contains(&("rules/a.mdc".into(), Agent::Cursor, FileKind::Rule)));
    assert!(m.proposals.iter().all(|p| p.scope == Scope::User));
    // A missing root (Gemini, Windsurf) is a note, not a failure.
    assert!(m
        .roots
        .iter()
        .any(|r| r.label == "Gemini" && r.note.is_some()));
}

#[test]
fn codex_home_overrides_the_default_codex_folder() {
    let home = Path::new("/h");
    let roots = default_roots(home, Some(Path::new("/elsewhere/codex")));
    assert_eq!(roots[1].path, Path::new("/elsewhere/codex"));
    assert_eq!(default_roots(home, None)[1].path, Path::new("/h/.codex"));
}

#[test]
fn duplicates_large_files_and_the_read_budget() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(&root, "a/CLAUDE.md", "same text\n");
    write(&root, "b/CLAUDE.md", "same text\n");
    write(&root, "big/CLAUDE.md", &"x ".repeat(FILE_MAX as usize));
    let m = rig.run(&[project(root.clone())]).unwrap();
    let a = &m.proposals[0];
    let b = &m.proposals[1];
    assert_eq!(a.duplicate_of, None);
    assert_eq!(b.duplicate_of.as_ref(), Some(&a.id));
    let big = m
        .proposals
        .iter()
        .find(|p| p.rel_path == "big/CLAUDE.md")
        .unwrap();
    assert_eq!(big.withheld.as_deref(), Some("too-large"));
    assert!(big.hash.is_none() && big.excerpt.is_none());
    // A tiny read budget leaves the later files listed but unread.
    let tight = ScanBudget {
        read_bytes: 12,
        ..ScanBudget::default()
    };
    let m = rig
        .run_with(&[project(root)], &tight, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(m.proposals[0].withheld, None);
    assert_eq!(m.proposals[1].withheld.as_deref(), Some("read-budget"));
}

#[test]
fn the_file_budget_stops_the_walk_and_says_so() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    for i in 0..5 {
        write(&root, &format!("d{i}/AGENTS.md"), "x\n");
    }
    let mut budget = ScanBudget::default();
    budget.walk.max_files = 2;
    let m = rig
        .run_with(&[project(root)], &budget, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(m.proposals.len(), 2);
    assert!(m.roots[0]
        .note
        .as_ref()
        .unwrap()
        .contains("Stopped after 2 files"));
}

#[test]
fn cancel_discards_and_an_empty_root_list_is_refused() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(&root, "AGENTS.md", "x\n");
    let cancel = AtomicBool::new(true);
    let r = rig.run_with(&[project(root)], &ScanBudget::default(), &cancel);
    assert_eq!(r.unwrap_err(), CANCELLED);
    assert!(rig.staging.ids().is_empty());
    assert!(rig.run(&[]).is_err());
}

#[test]
fn nothing_is_written_to_the_scanned_folder() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(&root, "CLAUDE.md", "x\n");
    let before: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    rig.run(&[project(root.clone())]).unwrap();
    let after: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(before, after);
}

#[test]
fn job_status_follows_the_run() {
    let rig = Rig::new();
    let id = "imp-20261119-00000002";
    let cancel = begin_job(id);
    assert_eq!(status(&rig.staging, id).unwrap().unwrap().state, "scanning");
    let root = rig.dir("proj");
    write(&root, "AGENTS.md", "x\n");
    let result = scan(
        &rig.staging,
        id,
        &[project(root)],
        &[],
        &ScanBudget::default(),
        &cancel,
    );
    end_job(id, &result);
    let s = status(&rig.staging, id).unwrap().unwrap();
    assert_eq!((s.state.as_str(), s.found), ("ready", 1));
    assert!(cancel_staged(&rig, id));
    assert!(status(&rig.staging, id)
        .unwrap()
        .is_some_and(|s| s.state == "cancelled"));
}

fn cancel_staged(rig: &Rig, id: &str) -> bool {
    cancel(&rig.staging, id).unwrap()
}

#[test]
fn a_file_cannot_waive_its_own_blocking_finding() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    let ssn = format!("{}-{}-{}", "123", "45", "6789");
    write(
        &root,
        "CLAUDE.md",
        &format!("SSN {ssn}  # context-scan: allow pii.ssn reason here\n"),
    );
    let m = rig.run(&[project(root)]).unwrap();
    let p = &m.proposals[0];
    assert!(p.flags.iter().any(|f| f.rule == "pii.ssn"), "{:?}", p.flags);
    assert_eq!(p.excerpt, None);
    assert!(!serde_json::to_string(&m).unwrap().contains(&ssn));
}

#[test]
fn the_vault_guard_refuses_dotdot_links_and_ancestors() {
    let rig = Rig::new();
    let home = rig.dir("home");
    let vault = home.join("Hollow").join("Vault");
    std::fs::create_dir_all(vault.join("notes")).unwrap();
    write(&vault, "notes/CLAUDE.md", "vault file\n");
    let r = |p: PathBuf| vec![project(p)];
    assert!(resolve_roots(r(vault.join("notes")), &vault).is_err());
    std::fs::create_dir_all(home.join("elsewhere")).unwrap();
    let dotdot = home
        .join("elsewhere")
        .join("..")
        .join("Hollow")
        .join("Vault")
        .join("notes");
    assert!(resolve_roots(r(dotdot), &vault).is_err());
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&vault, home.join("elsewhere").join("v")).unwrap();
        let via = home.join("elsewhere").join("v").join("notes");
        assert!(resolve_roots(r(via), &vault).is_err());
    }
    assert!(resolve_roots(r(home.clone()), &vault).is_err());
    assert!(resolve_roots(r(home.join("Hollow")), &vault).is_err());
    let ok = rig.dir("other");
    let (roots, avoid) = resolve_roots(r(ok.clone()), &vault).unwrap();
    assert_eq!(roots[0].path, std::fs::canonicalize(&ok).unwrap());
    assert!(!avoid.is_empty());
}

#[test]
fn the_walk_skips_an_avoided_folder() {
    let rig = Rig::new();
    let root = rig.dir("proj");
    write(&root, "keep/CLAUDE.md", "keep\n");
    write(&root, "vault/CLAUDE.md", "private\n");
    let m = scan(
        &rig.staging,
        "imp-20261119-00000003",
        &[project(root.clone())],
        &[root.join("vault")],
        &ScanBudget::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(m.proposals.len(), 1);
    assert_eq!(m.proposals[0].rel_path, "keep/CLAUDE.md");
}
