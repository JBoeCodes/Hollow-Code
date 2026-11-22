//! Explicitly opened provider scratch memories, scoped to one Hollow folder.
//! These files are never prompt context and are never written by this module.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const MAX_BYTES: u64 = 2_000_000;
/// Excerpts shown per provider (Claude and Codex each).
const MAX_ITEMS: usize = 80;
/// Longest passage one promotion takes, the Library textarea's `maxlength`.
const MAX_STATEMENT_CHARS: usize = 2_000;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: String,
    pub provider: String,
    pub path: String,
    pub text: String,
    pub expected_hash: String,
}

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

fn read(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let mut checked = root.to_path_buf();
    for part in rel.components() {
        if !matches!(part, std::path::Component::Normal(_)) {
            return None;
        }
        checked.push(part);
        if checked.symlink_metadata().ok()?.file_type().is_symlink() {
            return None;
        }
    }
    let meta = path.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_BYTES {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

fn entry(provider: &str, path: &Path, key: &str, raw: &str) -> Entry {
    let path = path.to_string_lossy().into_owned();
    Entry {
        id: hash(&format!("{provider}\0{path}\0{key}")),
        provider: provider.into(),
        path,
        text: crate::claude_runner::redact(raw)
            .chars()
            .take(6_000)
            .collect(),
        expected_hash: hash(raw),
    }
}

/// Every folder a Codex task group names with `cwd=`: in its rollout
/// references (`cwd=A,` or `cwd=A)`) and its `applies_to` line, which can
/// list several (`cwd=A; cwd=B`).
fn named_cwds(group: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = group;
    while let Some(at) = rest.find("cwd=") {
        let word_start = !matches!(
            rest[..at].chars().next_back(),
            Some(c) if c.is_alphanumeric() || c == '_'
        );
        let value = &rest[at + 4..];
        let end = value.find([',', ')', ';', '\n']).unwrap_or(value.len());
        if word_start {
            out.push(value[..end].trim());
        }
        rest = &value[end..];
    }
    out
}

fn same_folder(cwd: &str, folder: &Path) -> bool {
    let folder = folder.to_string_lossy();
    !cwd.is_empty() && cwd.trim_end_matches('/') == folder.trim_end_matches('/')
}

fn list_in(home: &Path, folders: &[PathBuf]) -> Vec<Entry> {
    // Each provider has its own cap, so a full Claude folder never hides
    // the Codex groups.
    let mut claude = Vec::new();
    // Claude encodes the exact working folder in its project directory name.
    for folder in folders {
        let encoded: String = folder
            .to_string_lossy()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let root = home.join(".claude/projects").join(encoded).join("memory");
        let mut paths = std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("md"))
            .collect::<Vec<_>>();
        paths.sort();
        for path in paths {
            if claude.len() >= MAX_ITEMS {
                break;
            }
            if let Some(raw) = read(home, &path) {
                claude.push(entry("Claude", &path, "", &raw));
            }
        }
    }
    // Codex's registry mixes folders. Return only complete task groups whose
    // every named cwd is one of this Hollow project's folders: a group that
    // also names another folder holds that folder's notes too.
    let mut codex = Vec::new();
    let path = home.join(".codex/memories/MEMORY.md");
    if let Some(raw) = read(home, &path) {
        for (index, group) in raw.split("\n# Task Group:").enumerate() {
            let cwds = named_cwds(group);
            let belongs = !cwds.is_empty()
                && cwds
                    .iter()
                    .all(|cwd| folders.iter().any(|folder| same_folder(cwd, folder)));
            if belongs && codex.len() < MAX_ITEMS {
                codex.push(entry("Codex", &path, &index.to_string(), group));
            }
        }
    }
    claude.extend(codex);
    claude
}

fn folders(vault: &Path, workspace_id: &str) -> Result<Vec<PathBuf>, String> {
    let workspace = crate::workspaces::list(vault)?
        .into_iter()
        .find(|w| w.id == workspace_id)
        .ok_or_else(|| "Choose a Hollow project to read its private memories.".to_string())?;
    let vault_folder = crate::vault::safe_join(vault, &workspace.path)?;
    let named = std::iter::once(workspace.cwd).chain(workspace.build.map(|build| build.repo));
    Ok(roots(vault_folder, named))
}

/// The project's vault folder plus every absolute folder it names. An empty
/// or relative cwd names no folder of its own, so it is skipped.
fn roots(vault_folder: PathBuf, named: impl IntoIterator<Item = String>) -> Vec<PathBuf> {
    let mut folders = vec![vault_folder];
    folders.extend(
        named
            .into_iter()
            .map(PathBuf::from)
            .filter(|path| path.is_absolute()),
    );
    folders.sort();
    folders.dedup();
    folders
}

pub fn list(vault: &Path, workspace_id: &str) -> Result<Vec<Entry>, String> {
    let home = dirs::home_dir().ok_or_else(|| "Home folder unavailable".to_string())?;
    let mut entries = list_in(&home, &folders(vault, workspace_id)?);
    let mut seen = std::collections::HashSet::new();
    entries.retain(|e| seen.insert(e.id.clone()));
    Ok(entries)
}

pub fn promote(
    vault: &Path,
    workspace: &str,
    id: &str,
    expected: &str,
    scope: &str,
    statement: &str,
) -> Result<crate::vault_learning::LearningProposal, String> {
    let item = list(vault, workspace)?
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| "Private memory is no longer available.".to_string())?;
    promote_entry(vault, workspace, item, expected, scope, statement)
}

/// One promotion an agent asks for (the COO's
/// `learning.promote_private_memory`, L13): [`promote`]'s arguments.
pub(crate) struct AgentPromotion<'a> {
    pub workspace: &'a str,
    pub id: &'a str,
    pub expected: &'a str,
    pub scope: &'a str,
    pub statement: &'a str,
}

/// [`promote`] for an agent (the COO's `learning.promote_private_memory`,
/// L13): the same pending proposal, which only the person accepts, under
/// the rules agents propose learnings by (`submit_learning_proposal`): none
/// while Learning suggestions is off, none from a chat another agent started
/// under Important (unless the person asked in their own turn), and one per
/// run. The proposal carries the run, so it counts toward that one.
pub(crate) fn promote_for_agent(
    vault: &Path,
    ask: &AgentPromotion<'_>,
    run_id: &str,
    asker: crate::vault_learning::Asker,
    mode: crate::vault_learning::Suggestions,
) -> Result<crate::vault_learning::LearningProposal, String> {
    use crate::vault_learning::Suggestions;
    let bar = if asker.person && asker.asked {
        Suggestions::All
    } else {
        mode
    };
    match bar {
        Suggestions::Off => {
            return Err(
                "Learning suggestions are off (Settings → Personalization). Nothing queued.".into(),
            )
        }
        Suggestions::Important if asker.depth > 0 => {
            return Err("Under Settings → Personalization → How many learnings to suggest: Important only, a chat another agent started doesn't propose learnings. Mention it in your report instead. Nothing queued.".into())
        }
        _ => {}
    }
    let proposed = crate::vault_learning::list_proposals(vault)?
        .into_iter()
        .any(|p| p.payload.source_run_id.as_deref() == Some(run_id));
    if proposed {
        return Err(
            "This run already proposed a learning; Hollow takes one per run. Nothing queued."
                .into(),
        );
    }
    let item = list(vault, ask.workspace)?
        .into_iter()
        .find(|e| e.id == ask.id)
        .ok_or_else(|| "Private memory is no longer available.".to_string())?;
    promote_entry_by(
        vault,
        ask.workspace,
        item,
        ask.expected,
        ask.scope,
        ask.statement,
        Some(run_id),
    )
}

fn promote_entry(
    vault: &Path,
    workspace: &str,
    item: Entry,
    expected: &str,
    scope: &str,
    statement: &str,
) -> Result<crate::vault_learning::LearningProposal, String> {
    promote_entry_by(vault, workspace, item, expected, scope, statement, None)
}

/// [`promote_entry`], stamped with the agent run that asked (`None` for the
/// person's Promote in Library).
fn promote_entry_by(
    vault: &Path,
    workspace: &str,
    item: Entry,
    expected: &str,
    scope: &str,
    statement: &str,
    run: Option<&str>,
) -> Result<crate::vault_learning::LearningProposal, String> {
    if item.expected_hash != expected {
        return Err("Private memory changed. Refresh before promoting.".into());
    }
    let statement = statement.trim();
    if statement.chars().count() > MAX_STATEMENT_CHARS {
        return Err(format!(
            "Choose a passage of at most {MAX_STATEMENT_CHARS} characters to promote."
        ));
    }
    if statement.len() < 12 || !item.text.contains(statement) {
        return Err("Choose an exact passage from the private memory (at least 12 bytes).".into());
    }
    let (kind, path) = match scope {
        "you" => ("operator", "me/Decisions.md".to_string()),
        "knowhow" => ("knowhow", "learnings/".to_string()),
        "project" => ("workspace", format!("workspaces/{workspace}/WORKSPACE.md")),
        _ => return Err("Choose You, Know-how or Project.".into()),
    };
    crate::vault_learning::submit_proposal_from(
        vault,
        crate::vault_learning::LearningProposalPayload {
            kind: "private-memory-promotion".into(),
            target_kind: kind.into(),
            target_path: path,
            subject: format!("Promote {} memory", item.provider),
            statement: statement.into(),
            evidence_excerpt: statement.chars().take(600).collect(),
            rationale: "Provider scratch memory, proposed for review. Save is required.".into(),
            source_run_id: run.map(str::to_string),
            ..Default::default()
        },
        "private-memory",
        Some(item.path),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_memory_roots_are_absolute_and_claude_reads_only_the_exact_folder() {
        let vault = tempfile::tempdir().unwrap();
        let w = crate::workspaces::create(vault.path(), "Memory fixture", "Read exact sources.")
            .unwrap();
        let roots = folders(vault.path(), &w.id).unwrap();
        assert!(roots.iter().all(|p| p.is_absolute()));
        let home = tempfile::tempdir().unwrap();
        let matching = home.path().join(".claude/projects/-work-hollow/memory");
        let other = home.path().join(".claude/projects/-work-other/memory");
        std::fs::create_dir_all(&matching).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(matching.join("lesson.md"), "Keep CSV headers verbatim.").unwrap();
        std::fs::write(other.join("lesson.md"), "Unrelated private source.").unwrap();
        let entries = list_in(home.path(), &["/work/hollow".into()]);
        assert_eq!(entries.len(), 1);
        assert!(!entries[0].text.contains("Unrelated"));
    }
    #[test]
    fn promotion_is_a_pending_scanned_proposal_with_a_fresh_pin() {
        let vault = tempfile::tempdir().unwrap();
        let source = entry(
            "Codex",
            Path::new("/fixture/MEMORY.md"),
            "0",
            "Keep CSV headers verbatim.",
        );
        assert!(promote_entry(
            vault.path(),
            "fixture",
            source.clone(),
            "changed",
            "knowhow",
            "Keep CSV headers verbatim."
        )
        .is_err());
        let proposed = promote_entry(
            vault.path(),
            "fixture",
            source.clone(),
            &source.expected_hash,
            "knowhow",
            "Keep CSV headers verbatim.",
        )
        .unwrap();
        assert_eq!(proposed.status, "pending");
        assert_eq!(proposed.payload.target_kind, "knowhow");
        assert!(crate::vault_learning::list_context(vault.path(), true).is_empty());
        assert!(!vault.path().join("learnings").exists());
    }
    /// L13: the COO's promotion is the same pending proposal, stamped with
    /// its run, so a second one in that run is refused like any agent
    /// learning; the person's Promote carries no run.
    #[test]
    fn an_agent_promotion_carries_its_run_and_counts_as_its_one_learning() {
        use crate::vault_learning::{Asker, Suggestions};
        let vault = tempfile::tempdir().unwrap();
        let source = entry(
            "Claude",
            Path::new("/fixture/memory/lesson.md"),
            "",
            "Always reconcile tips per company code.",
        );
        let proposed = promote_entry_by(
            vault.path(),
            "fixture",
            source.clone(),
            &source.expected_hash,
            "knowhow",
            "Always reconcile tips per company code.",
            Some("run-coo-1"),
        )
        .unwrap();
        assert_eq!(proposed.status, "pending", "only the person accepts it");
        assert_eq!(proposed.payload.source_run_id.as_deref(), Some("run-coo-1"));
        let ask = AgentPromotion {
            workspace: "fixture",
            id: &source.id,
            expected: &source.expected_hash,
            scope: "knowhow",
            statement: "Always reconcile tips per company code.",
        };
        let asker = Asker::default();
        let again = promote_for_agent(vault.path(), &ask, "run-coo-1", asker, Suggestions::All)
            .unwrap_err();
        assert!(again.contains("one per run"), "{again}");
        let off = promote_for_agent(vault.path(), &ask, "run-coo-2", asker, Suggestions::Off)
            .unwrap_err();
        assert!(off.contains("are off"), "{off}");
        let child = Asker {
            depth: 1,
            ..Asker::default()
        };
        let deep = promote_for_agent(
            vault.path(),
            &ask,
            "run-coo-2",
            child,
            Suggestions::Important,
        )
        .unwrap_err();
        assert!(deep.contains("another agent started"), "{deep}");
        assert_eq!(
            crate::vault_learning::list_proposals(vault.path())
                .unwrap()
                .len(),
            1,
            "a refusal queues nothing"
        );
    }
    #[test]
    fn registry_groups_are_scoped_and_sources_stay_unchanged() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".codex/memories/MEMORY.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = "# Task Group: Other\napplies_to: cwd=/work/other; rule=x\nprivate other\n\n# Task Group: Hollow\napplies_to: cwd=/work/hollow; rule=x\nUse exact sources.\n\n# Task Group: Similar\napplies_to: cwd=/work/hollow-extra; rule=x\nprivate similar";
        std::fs::write(&path, raw).unwrap();
        let entries = list_in(home.path(), &["/work/hollow".into()]);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("Use exact sources"));
        assert!(!entries[0].text.contains("private other"));
        assert!(!entries[0].text.contains("private similar"));
        assert_eq!(std::fs::read_to_string(path).unwrap(), raw);
    }
    /// Review 2026-11-08: a group that also names another folder holds that
    /// folder's notes, so it is returned to neither project.
    #[test]
    fn a_codex_group_naming_another_folder_is_not_returned() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join(".codex/memories/MEMORY.md");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let raw = "# Task Group: Mixed\napplies_to: cwd=/work/other; cwd=/work/hollow\nprivate other text\n\n# Task Group: Rollouts\n(rollout=1, cwd=/work/hollow, at=x)\n(rollout=2, cwd=/work/elsewhere)\nprivate elsewhere text\n\n# Task Group: Both mine\napplies_to: cwd=/work/hollow; cwd=/work/hollow-repo\nUse exact sources.\n\n# Task Group: Unnamed\nNo folder named here.";
        std::fs::write(&path, raw).unwrap();
        let mine = ["/work/hollow".into(), "/work/hollow-repo".into()];
        let entries = list_in(home.path(), &mine);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].text.contains("Use exact sources"));
        let other = list_in(home.path(), &["/work/other".into()]);
        assert!(other.is_empty());
        assert_eq!(
            named_cwds("applies_to: cwd=/a; cwd=/b\n(x=1, cwd=/c)\nmycwd=/d"),
            ["/a", "/b", "/c"]
        );
    }
    #[test]
    fn empty_or_relative_cwds_are_skipped_and_each_provider_has_its_own_cap() {
        let roots = roots(
            "/vault/workspaces/w".into(),
            ["".into(), "relative/dir".into(), "/work/hollow".into()],
        );
        assert_eq!(
            roots,
            [PathBuf::from("/vault/workspaces/w"), "/work/hollow".into()]
        );
        let home = tempfile::tempdir().unwrap();
        let claude = home.path().join(".claude/projects/-work-hollow/memory");
        std::fs::create_dir_all(&claude).unwrap();
        for n in 0..MAX_ITEMS + 5 {
            std::fs::write(claude.join(format!("note-{n:03}.md")), format!("Note {n}")).unwrap();
        }
        let codex = home.path().join(".codex/memories/MEMORY.md");
        std::fs::create_dir_all(codex.parent().unwrap()).unwrap();
        std::fs::write(
            &codex,
            "# Task Group: A\napplies_to: cwd=/work/hollow\nFirst.\n\n# Task Group: B\napplies_to: cwd=/work/hollow\nSecond.",
        )
        .unwrap();
        let entries = list_in(home.path(), &["/work/hollow".into()]);
        let count = |provider: &str| entries.iter().filter(|e| e.provider == provider).count();
        assert_eq!(count("Claude"), MAX_ITEMS);
        assert_eq!(count("Codex"), 2);
    }
    #[test]
    fn a_promoted_passage_is_capped_like_the_library_field() {
        let vault = tempfile::tempdir().unwrap();
        let long = "a".repeat(MAX_STATEMENT_CHARS + 1);
        let source = entry("Claude", Path::new("/fixture/note.md"), "", &long);
        let refused = promote_entry(
            vault.path(),
            "fixture",
            source.clone(),
            &source.expected_hash,
            "knowhow",
            &long,
        )
        .err()
        .unwrap();
        assert!(refused.contains("at most 2000 characters"), "{refused}");
    }
    #[cfg(unix)]
    #[test]
    fn symlinked_provider_memory_is_not_read() {
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("MEMORY.md"), "other project").unwrap();
        std::fs::create_dir_all(home.path().join(".codex")).unwrap();
        std::os::unix::fs::symlink(outside.path(), home.path().join(".codex/memories")).unwrap();
        assert!(list_in(home.path(), &["/work/hollow".into()]).is_empty());
    }
}
