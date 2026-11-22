//! A stable, provider-neutral conversation for each project's main agent.
//! Keys are derived from project identity; opening a project never calls a model.
use crate::{session_store, workspaces};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Shared by global and project main agents; factual questions stay in place.
pub(crate) const WORKER_GUIDANCE: &str = "Create Hollow workers only when the person requests workers/subagents or asks you to execute a batch of tasks. 'Do 5 Portal tasks' means choose five distinct authorized tasks and call hollow_app run_action sessions.start_workers with count 5 and five task entries in Portal's workspace. Use one worker per task, with a clear title, scope and done condition; do not substitute provider-native subagents or invisible consultations. If the tasks are ambiguous, read the project's task list first and ask only for missing scope. 'Where are my projects?', status questions and ordinary conversation need a direct answer, no workers. Report the returned worker chats and any failures, distinguish queued from running, then end your turn. Read their results with read_session on follow-up; never claim a worker exists before the tool returns its session key.";

pub(crate) fn key(id: &str) -> Option<String> {
    if workspaces::is_builtin(id) {
        return None;
    }
    Some(format!(
        "s-project-{}",
        &format!("{:x}", Sha256::digest(id.as_bytes()))[..48]
    ))
}

/// Every lead key project `id` has had: its current id's first, then one
/// per older id a Move Hollow home re-keyed it from (`workspace_aliases`).
/// A lead's key is a hash of the id its project had when the lead started,
/// and the move rewrites ids, not session keys, so a moved project's lead
/// keeps its old key (2026-11-14: atlas2.0's lead after the home move).
pub(crate) fn keys_for(aliases: &crate::workspace_aliases::Aliases, id: &str) -> Vec<String> {
    if aliases.is_empty() {
        return key(id).into_iter().collect();
    }
    let mut out: Vec<String> = Vec::new();
    for id in aliases.ids_for(id) {
        if let Some(k) = key(&id) {
            if !out.contains(&k) {
                out.push(k);
            }
        }
    }
    out
}

/// Project `id`'s lead conversation key, across moves: of
/// [`keys_for`]'s keys, the one whose conversation was active last; the
/// current id's when none has a conversation yet. None for a built-in
/// workspace.
pub(crate) fn lead_key(vault: &Path, id: &str) -> Option<String> {
    LeadKeys::read(vault, &[id]).lead_key(id)
}

/// [`lead_key`] for many projects with one aliases read and at most one
/// sessions read (the rail lists every project: a store open per row once
/// broke its `perf_hot_paths` ceiling).
pub(crate) struct LeadKeys {
    aliases: crate::workspace_aliases::Aliases,
    /// Last activity of each candidate key that has a conversation, read
    /// only for projects with more than one candidate.
    activity: std::collections::HashMap<String, String>,
}

impl LeadKeys {
    pub(crate) fn read(vault: &Path, ids: &[&str]) -> LeadKeys {
        let aliases = crate::workspace_aliases::Aliases::read(vault);
        let mut wanted: Vec<String> = Vec::new();
        if !aliases.is_empty() {
            for id in ids {
                let keys = keys_for(&aliases, id);
                if keys.len() > 1 {
                    wanted.extend(keys);
                }
            }
        }
        let activity = if wanted.is_empty() {
            Default::default()
        } else {
            session_store::last_activity_by_key(vault, &wanted).unwrap_or_default()
        };
        LeadKeys { aliases, activity }
    }

    pub(crate) fn lead_key(&self, id: &str) -> Option<String> {
        let keys = keys_for(&self.aliases, id);
        // The newest wins; a tie keeps the earlier candidate (the current
        // id's key comes first).
        let mut best: Option<(&String, &String)> = None;
        for k in &keys {
            if let Some(at) = self.activity.get(k) {
                let newer = match best {
                    Some((b, _)) => at > b,
                    None => true,
                };
                if newer {
                    best = Some((at, k));
                }
            }
        }
        best.map(|(_, k)| k.clone())
            .or_else(|| keys.into_iter().next())
    }
}

/// The project whose lead conversation is `session` (a fixed lead key, by
/// [`keys_for`]), among `all`.
pub(crate) fn project_of<'a>(
    vault: &Path,
    all: &'a [workspaces::Workspace],
    session: &str,
) -> Option<&'a workspaces::Workspace> {
    if !session.starts_with("s-project-") {
        return None;
    }
    let aliases = crate::workspace_aliases::Aliases::read(vault);
    all.iter()
        .find(|w| keys_for(&aliases, &w.id).iter().any(|k| k == session))
}

/// Recheck at both queue acceptance and admission. A moved conversation must
/// not silently become another project's agent or run under a different cwd.
pub(crate) fn validate(vault: &Path, cwd: &Path, session: &str) -> Result<(), String> {
    if !session.starts_with("s-project-") {
        return Ok(());
    }
    let all = workspaces::list(vault)?;
    // A project re-keyed by Move Hollow home still owns the lead it started
    // under its old id ([`keys_for`]).
    let project = project_of(vault, &all, session)
        .filter(|w| Path::new(&w.cwd) == cwd)
        .ok_or("This lead conversation belongs to another project. Nothing was queued.")?;
    let Some(s) = session_store::session_by_key(vault, session)? else {
        return Ok(());
    };
    if s.cwd != project.cwd {
        // After Remove worktree the project runs in its vault folder while
        // its main conversation keeps the old path. That path still counts
        // as the project's own, unless another workspace runs there or once
        // did (a reused path is ambiguous, so it is refused).
        let own_removed = project
            .build
            .as_ref()
            .is_some_and(|b| !b.exists && b.worktree == s.cwd);
        let claimed = all.iter().any(|w| w.id != project.id && w.cwd == s.cwd)
            || crate::build_workspaces::worktree_of_another(vault, &project.id, &s.cwd);
        if claimed || !own_removed {
            return Err("This lead conversation was moved. Move it back from Conversation history before continuing.".into());
        }
    }
    Ok(())
}

/// The notes a project's lead writes (L8, 2026-11-10): its project
/// folder's own files. It is their one writer; everything else it hands to
/// subagents.
pub(crate) const LEAD_NOTES: &[&str] = &["WORKSPACE.md", "PLAN.md", "SOURCES.md", "AGENTS.md"];

/// The project a lead conversation belongs to (its fixed one or a
/// successor of its handovers, `chat_chain::lead_root`).
pub(crate) fn lead_project(vault: &Path, session: &str) -> Option<workspaces::Workspace> {
    let lead = crate::chat_chain::lead_root(vault, session)?;
    let all = workspaces::list(vault).ok()?;
    project_of(vault, &all, &lead).cloned()
}

/// The lead's role (L8), added to every turn of a lead conversation. The
/// tool policy it states is enforced on agents that hold deny rules
/// ([`lead_denied_tools`]); on the others this text is the policy.
pub(crate) fn context(vault: &Path, session: &str) -> Option<String> {
    crate::chat_chain::is_lead_conversation(vault, session).then(|| format!(
        "\n\nYou are this project's lead: the person's one conversation for it, available for prompts, conversations and questions. Answer directly what a few reads answer. Building, research, planning and deliverables go to subagents, split by real dependencies with independent parts started at once: hollow_app start_session in this project's workspace with a task packet, then end your turn; never wait in it. {FOLLOW_UP_RULE} Overnight: the orchestrator is your subagent, its lanes its subagents. Hollow wakes you once when every subagent you started in a turn has ended, or one fails. You are the one writer of this project's notes (WORKSPACE.md Decisions, AGENTS.md, PLAN.md, SOURCES.md): when a subagent ends, read its result, record its decisions and outcomes there, and say so in one line starting \"Updated my notes\". Keep no task history in them: search your subagents' chats instead. Your own tools: reads, every Hollow action, questions and tasks; no shell, and edits only to those notes. {WORKER_GUIDANCE} Preserve the person's chosen model and permissions.",
        FOLLOW_UP_RULE = crate::fresh_brief::FOLLOW_UP_RULE,
    ))
}

/// A Claude path rule's absolute form (`//abs/path`), or None for a path
/// its rule syntax can't hold safely (spaces, commas, parentheses or glob
/// characters: the CLI splits and globs rule text).
fn abs_rule(path: &Path, suffix: &str) -> Option<String> {
    let text = path.to_str()?;
    let plain = text.starts_with('/')
        && !text.chars().any(|c| {
            c.is_whitespace() || matches!(c, ',' | '(' | ')' | '*' | '?' | '[' | ']' | '{' | '}')
        });
    plain.then(|| format!("Edit(/{text}{suffix})"))
}

/// Edit rules for every entry of folder `dir` except `keep` (a folder's
/// whole tree, a file itself). False when one couldn't be written
/// ([`abs_rule`]).
fn deny_entries_except(dir: &Path, keep: &[&str], out: &mut Vec<String>) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return true;
    };
    let mut names: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            let is_dir = e.file_type().ok()?.is_dir();
            Some((name, is_dir))
        })
        .filter(|(name, _)| !keep.contains(&name.as_str()))
        .collect();
    names.sort();
    let mut held = true;
    for (name, is_dir) in names {
        match abs_rule(&dir.join(&name), if is_dir { "/**" } else { "" }) {
            Some(rule) => out.push(rule),
            None => held = false,
        }
    }
    held
}

/// The delegate-only tool policy of a lead turn (L8) as Claude deny rules,
/// for project folder `notes` (`<vault>/workspaces/<id>`) and turn folder
/// `cwd`. Claude's rules can't say "edits except these four files" (a deny
/// beats any allow), so they deny as tightly as plain paths allow: the shell
/// (`Bash`), new or rewritten files (`Write`), notebooks, and `Edit` of the
/// turn folder when it isn't the project folder (a build worktree), of the
/// project folder's subfolders and its other top-level files, of every other
/// workspace folder, and of the vault's other top-level entries. Left open:
/// `Edit` of files outside the vault and the turn folder, and files created
/// after the turn started. A path the rule syntax can't hold ([`abs_rule`])
/// denies every edit instead, the notes included (logged).
pub(crate) fn lead_deny_rules(vault: &Path, notes: &Path, cwd: &Path) -> Vec<String> {
    let mut out: Vec<String> = ["Bash", "Write", "NotebookEdit"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    // The same folder written another way (`/var` and `/private/var`) is
    // the project folder: denying it would deny the notes.
    let same = cwd == notes
        || matches!((cwd.canonicalize(), notes.canonicalize()), (Ok(c), Ok(n)) if c == n);
    let own = notes
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let mut own_files: Vec<String> = std::fs::read_dir(notes)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| !t.is_dir()))
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|name| !LEAD_NOTES.contains(&name.as_str()))
                .collect()
        })
        .unwrap_or_default();
    own_files.sort();
    // Each path as given and resolved, so a rule holds whichever form the
    // agent names.
    let mut held = true;
    let mut push = |out: &mut Vec<String>, rule: Option<String>| match rule {
        Some(rule) => out.push(rule),
        None => held = false,
    };
    for cwd in forms(cwd).iter().filter(|_| !same) {
        push(&mut out, abs_rule(cwd, "/**"));
    }
    for notes in forms(notes) {
        push(&mut out, abs_rule(&notes, "/*/**"));
        for name in &own_files {
            push(&mut out, abs_rule(&notes.join(name), ""));
        }
    }
    for vault in forms(vault) {
        held &= deny_entries_except(&vault.join("workspaces"), &[own], &mut out);
        held &= deny_entries_except(&vault, &["workspaces"], &mut out);
    }
    if !held {
        // A path the rule syntax can't hold would leave its edits open:
        // deny every edit instead (the lead delegates them).
        eprintln!(
            "[project_agents] a path under {} can't be a Claude rule (space, comma, parenthesis or glob character): this lead turn gets no file edits",
            vault.display()
        );
        return lead_no_edits();
    }
    out
}

/// The fail-closed policy: no shell, no file edits at all.
fn lead_no_edits() -> Vec<String> {
    ["Bash", "Write", "NotebookEdit", "Edit"]
        .iter()
        .map(|t| t.to_string())
        .collect()
}

/// `path` as given, and resolved when that differs.
fn forms(path: &Path) -> Vec<std::path::PathBuf> {
    let mut out = vec![path.to_path_buf()];
    if let Ok(real) = path.canonicalize() {
        if real != path {
            out.push(real);
        }
    }
    out
}

/// The tool rules a lead conversation's turn adds (L8), on agents that hold
/// deny rules (`agent::holds_deny`, Claude today); empty for every other
/// conversation, the COO's included. Agents that can't hold them get the
/// policy from the prompt ([`context`]), so their lead turns are admitted.
pub(crate) fn lead_denied_tools(
    vault: &Path,
    session: &str,
    cwd: &Path,
    agent: crate::agent::AgentKind,
) -> Vec<String> {
    if !crate::chat_chain::is_lead_conversation(vault, session) {
        return Vec::new();
    }
    let notes = lead_project(vault, session).and_then(|p| workspaces::folder(vault, &p.id).ok());
    let rules = match notes {
        Some(notes) => lead_deny_rules(vault, &notes, cwd),
        // Its project can't be read (an unreadable brief): fail closed, no
        // shell and no file edits at all this turn.
        None => lead_no_edits(),
    };
    rules
        .into_iter()
        .filter(|rule| crate::agent::holds_deny(agent, rule))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_are_stable_distinct_and_exclude_builtins() {
        assert_eq!(key("a"), key("a"));
        assert_ne!(key("a"), key("b"));
        assert!(crate::agent_sessions::is_safe_session_key(
            &key("a").unwrap()
        ));
        assert!(key("main").is_none());
        assert!(key("quick-chats").is_none());
        assert!(context(Path::new("/nowhere"), "s-ordinary").is_none());
    }
    #[test]
    fn wrong_project_and_moved_coordinators_are_refused_without_changing_history() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let a = workspaces::create(vault, "First", "Work").unwrap();
        let b = workspaces::create(vault, "Second", "Work").unwrap();
        let main = key(&a.id).unwrap();
        assert!(validate(vault, Path::new(&a.cwd), &main).is_ok());
        assert!(validate(vault, Path::new(&b.cwd), &main).is_err());
        session_store::start_turn(
            vault,
            &main,
            "codex",
            &b.cwd,
            "r-moved",
            "Keep this history",
        )
        .unwrap();
        assert!(validate(vault, Path::new(&a.cwd), &main).is_err());
        let kept = session_store::session_by_key(vault, &main)
            .unwrap()
            .unwrap();
        assert_eq!(kept.agent, "codex");
        assert_eq!(kept.cwd, b.cwd);
    }

    /// 2026-11-14: Move Hollow home re-keyed atlas2.0 from one `local-` id
    /// to another (`workspace_aliases`) but kept its lead's session key, a
    /// hash of the old id, so Main's send was refused as "another project"
    /// and the project page showed no lead.
    #[test]
    fn a_lead_started_before_a_home_move_stays_its_projects_lead() {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path();
        let a = workspaces::create(vault, "atlas2.0", "Work").unwrap();
        let b = workspaces::create(vault, "Other", "Work").unwrap();
        let old_id = "local-33c6f11285f1b1637f2779b6";
        let old_lead = key(old_id).unwrap();
        // The lead's history, its folder already rewritten by the move.
        session_store::start_turn(
            vault,
            &old_lead,
            "claude",
            &a.cwd,
            "r-lead",
            "Before the move",
        )
        .unwrap();
        // Without the alias the old key belongs to no project.
        assert!(validate(vault, Path::new(&a.cwd), &old_lead).is_err());
        assert_eq!(lead_key(vault, &a.id), key(&a.id));
        let pairs = std::collections::BTreeMap::from([(old_id.to_string(), a.id.clone())]);
        session_store::with_write(vault, "aliases", |tx| {
            crate::workspace_aliases::record(tx, &pairs, "move-1", "now")
        })
        .unwrap();
        assert!(validate(vault, Path::new(&a.cwd), &old_lead).is_ok());
        assert!(validate(vault, Path::new(&b.cwd), &old_lead).is_err());
        assert_eq!(lead_key(vault, &a.id).as_deref(), Some(old_lead.as_str()));
        assert_eq!(
            crate::chat_chain::lead_current(vault, &a.id).as_deref(),
            Some(old_lead.as_str())
        );
        assert_eq!(
            lead_project(vault, &old_lead).map(|w| w.id),
            Some(a.id.clone())
        );
        assert!(
            keys_for(&crate::workspace_aliases::Aliases::read(vault), &b.id)
                .iter()
                .all(|k| k != &old_lead)
        );
        let rail = workspaces::sessions_by_workspace(vault).unwrap();
        let row = rail.iter().find(|g| g.workspace_id == a.id).unwrap();
        assert_eq!(row.project_agent_key.as_deref(), Some(old_lead.as_str()));
        // A lead started fresh under the new id after the move: the one
        // active last is the lead, and the old one still validates.
        let new_lead = key(&a.id).unwrap();
        session_store::start_turn(
            vault,
            &new_lead,
            "claude",
            &a.cwd,
            "r-new",
            "After the move",
        )
        .unwrap();
        assert_eq!(lead_key(vault, &a.id).as_deref(), Some(new_lead.as_str()));
        assert!(validate(vault, Path::new(&a.cwd), &old_lead).is_ok());
    }

    /// The lead prompt's byte budget (L8): it rides every lead turn.
    const LEAD_PROMPT_BUDGET: usize = 2_000;

    /// L8: the lead's role is short and says what it does and doesn't do.
    #[test]
    fn the_lead_prompt_delegates_and_keeps_the_notes_within_its_budget() {
        let dir = tempfile::tempdir().unwrap();
        let p = context(dir.path(), &key("proj").unwrap()).unwrap();
        assert!(
            p.len() <= LEAD_PROMPT_BUDGET,
            "the lead prompt is {} bytes",
            p.len()
        );
        for needle in [
            "available for prompts, conversations and questions",
            "Building, research, planning and deliverables go to subagents",
            "start_session in this project's workspace",
            "then end your turn; never wait in it",
            "the orchestrator is your subagent, its lanes its subagents",
            "go to subagents, split by real dependencies with independent parts started at once",
            "every subagent you started in a turn has ended, or one fails",
            "one writer of this project's notes (WORKSPACE.md Decisions, AGENTS.md",
            "one line starting \"Updated my notes\"",
            "search your subagents' chats",
            "no shell, and edits only to those notes",
            crate::fresh_brief::FOLLOW_UP_RULE,
        ] {
            assert!(p.contains(needle), "{needle}: {p}");
        }
        assert!(!p.contains('\u{2014}') && !p.contains('\u{2013}'));
        // The rule itself rides the Delegating block every lead turn gets
        // (delegation::guidance); the role only points at it.
        assert!(!p.contains(crate::delegation::DEPENDENCY_RULE));
    }

    /// Whether Claude rule list `rules` denies an edit of `path` (the shapes
    /// [`lead_deny_rules`] writes: a file, `dir/**`, `dir/*/**`).
    fn denies(rules: &[String], path: &Path) -> bool {
        let path = path.to_str().unwrap();
        rules.iter().any(|rule| {
            let Some(pattern) = rule
                .strip_prefix("Edit(/")
                .and_then(|r| r.strip_suffix(')'))
            else {
                return false;
            };
            if let Some(dir) = pattern.strip_suffix("/*/**") {
                return path
                    .strip_prefix(&format!("{dir}/"))
                    .is_some_and(|rest| rest.contains('/'));
            }
            if let Some(dir) = pattern.strip_suffix("/**") {
                return path.starts_with(&format!("{dir}/"));
            }
            pattern == path
        })
    }

    /// L8: a Claude lead turn denies the shell, new files and edits outside
    /// its project's four notes, and allows those; a successor of its
    /// handovers gets the same; a Codex lead gets no rule it can't hold (so
    /// its turn is admitted, the policy in its prompt); the COO and any
    /// other chat get none of it.
    #[test]
    fn a_lead_turn_denies_the_shell_and_outside_edits_and_allows_its_notes() {
        use crate::agent::AgentKind;
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let vault = dir.path();
        let a = workspaces::create(vault, "First", "Work").unwrap();
        let b = workspaces::create(vault, "Second", "Work").unwrap();
        let own = workspaces::folder(vault, &a.id).unwrap();
        std::fs::create_dir_all(own.join("outputs")).unwrap();
        std::fs::write(own.join("outputs/report.md"), "x").unwrap();
        std::fs::write(own.join("README.md"), "x").unwrap();
        for note in LEAD_NOTES {
            if !own.join(note).exists() {
                std::fs::write(own.join(note), "x").unwrap();
            }
        }
        std::fs::create_dir_all(vault.join("inbox")).unwrap();
        std::fs::write(vault.join("notes.md"), "x").unwrap();
        let lead = key(&a.id).unwrap();
        let rules = lead_denied_tools(vault, &lead, Path::new(&a.cwd), AgentKind::Claude);
        for tool in ["Bash", "Write", "NotebookEdit"] {
            assert!(rules.iter().any(|r| r == tool), "{tool}: {rules:?}");
        }
        for note in LEAD_NOTES {
            assert!(
                !denies(&rules, &own.join(note)),
                "{note} stays editable: {rules:?}"
            );
        }
        let other = workspaces::folder(vault, &b.id).unwrap();
        for outside in [
            own.join("outputs/report.md"),
            own.join("README.md"),
            other.join("WORKSPACE.md"),
            vault.join("inbox/today.md"),
            vault.join("notes.md"),
        ] {
            assert!(denies(&rules, &outside), "{outside:?}: {rules:?}");
        }
        assert!(!rules
            .iter()
            .any(|r| r == "Edit" || r == "Read" || r.starts_with("Read(")));
        // A build worktree's tree is denied too.
        let tree = tempfile::Builder::new().prefix("tree-").tempdir().unwrap();
        let in_tree = lead_deny_rules(vault, &own, tree.path());
        assert!(denies(&in_tree, &tree.path().join("src/main.rs")));
        assert!(!denies(&in_tree, &own.join("PLAN.md")));
        // A successor of the lead's handover is the lead.
        crate::app_sessions::record_handover(vault, &lead, "s-20261110-l8pol-b", 1).unwrap();
        assert_eq!(
            lead_denied_tools(
                vault,
                "s-20261110-l8pol-b",
                Path::new(&a.cwd),
                AgentKind::Claude
            ),
            rules
        );
        // Codex holds no deny rule: none, and its run validates.
        for agent in [AgentKind::Codex, AgentKind::Antigravity, AgentKind::Grok] {
            let held = lead_denied_tools(vault, &lead, Path::new(&a.cwd), agent);
            assert!(held.is_empty(), "{agent:?}: {held:?}");
        }
        let spec = |disallowed: Vec<String>| crate::agent::RunSpec {
            prompt: "hi".into(),
            resume: None,
            model: None,
            perm: "acceptEdits".into(),
            cwd: std::path::PathBuf::from(&a.cwd),
            allowed_tools: vec![],
            disallowed_tools: disallowed,
            interactive: false,
            mcp_config: None,
            append_system_prompt: None,
            process_scope: crate::agent::ProcessScope::Inherited,
            usage_stream: false,
            max_budget_usd: None,
            extras: crate::agent::RunExtras::default(),
        };
        assert!(
            crate::agent::validate_run_spec(AgentKind::Codex, &spec(rules.clone())).is_err(),
            "unfiltered rules would refuse it"
        );
        assert!(crate::agent::validate_run_spec(
            AgentKind::Codex,
            &spec(lead_denied_tools(
                vault,
                &lead,
                Path::new(&a.cwd),
                AgentKind::Codex
            ))
        )
        .is_ok());
        assert!(crate::agent::validate_run_spec(AgentKind::Claude, &spec(rules)).is_ok());
        // The COO and an ordinary chat: nothing added.
        let main = workspaces::ensure_main_agent(vault).unwrap();
        session_store::start_turn(
            vault,
            "s-20261110-l8pol-coo",
            "claude",
            &main.to_string_lossy(),
            "R-l8pol",
            "Hi",
        )
        .unwrap();
        assert!(
            lead_denied_tools(vault, "s-20261110-l8pol-coo", &main, AgentKind::Claude).is_empty()
        );
        assert!(lead_denied_tools(
            vault,
            "s-20261110-l8pol-any",
            Path::new(&a.cwd),
            AgentKind::Claude
        )
        .is_empty());
    }

    /// L8 review: a vault entry whose path the rule syntax can't hold would
    /// leave its edits open, so the lead gets no file edits at all.
    #[test]
    fn a_path_a_rule_cant_hold_denies_every_edit() {
        let dir = tempfile::Builder::new().prefix("vault-").tempdir().unwrap();
        let vault = dir.path();
        let a = workspaces::create(vault, "First", "Work").unwrap();
        let own = workspaces::folder(vault, &a.id).unwrap();
        let fine = lead_deny_rules(vault, &own, &own);
        assert!(!fine.iter().any(|r| r == "Edit"));
        std::fs::create_dir_all(vault.join("My Notes")).unwrap();
        let rules = lead_deny_rules(vault, &own, &own);
        assert_eq!(rules, lead_no_edits());
        assert!(rules.iter().any(|r| r == "Edit") && rules.iter().any(|r| r == "Bash"));
    }
}
