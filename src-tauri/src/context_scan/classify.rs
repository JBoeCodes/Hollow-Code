//! Which files are instruction and memory files (CX19), by path alone.
//!
//! The classifier never opens a file: it reads the path below a scan root
//! (split into components) and answers the agent the file belongs to and
//! what kind of file it is. A root may carry an [`Agent`] hint for a folder
//! like `~/.cursor` whose own name is not part of the relative path.

use serde::{Deserialize, Serialize};

/// The tool a file configures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Agent {
    ClaudeCode,
    Codex,
    Cursor,
    Gemini,
    Windsurf,
    Copilot,
    Cline,
    /// `AGENTS.md` and `.agents/`: read by several tools.
    Generic,
}

/// What a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum FileKind {
    /// `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, `copilot-instructions.md`.
    Instructions,
    /// A rule file (`.cursorrules`, `.cursor/rules/*`, `.windsurfrules`, `.clinerules`).
    Rule,
    /// A saved prompt or slash command (`commands/*.md`, `.codex/prompts/*.md`).
    Command,
    /// `skills/<name>/SKILL.md`.
    Skill,
    /// A file in a memory folder.
    Memory,
}

fn agent_of_component(c: &str) -> Option<Agent> {
    Some(match c {
        ".claude" => Agent::ClaudeCode,
        ".codex" => Agent::Codex,
        ".cursor" => Agent::Cursor,
        ".gemini" => Agent::Gemini,
        ".windsurf" => Agent::Windsurf,
        ".github" => Agent::Copilot,
        ".clinerules" => Agent::Cline,
        ".agents" => Agent::Generic,
        _ => return None,
    })
}

fn is_text(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with(".md") || n.ends_with(".mdc") || n.ends_with(".markdown") || n.ends_with(".txt")
}

/// Classify the file at `rel` (components below the scan root). `hint` is the
/// root's own agent, used when no component names one.
pub(crate) fn classify(rel: &[String], hint: Option<Agent>) -> Option<(Agent, FileKind)> {
    let name = rel.last()?;
    let lower = name.to_ascii_lowercase();
    let dirs: Vec<String> = rel[..rel.len() - 1]
        .iter()
        .map(|c| c.to_ascii_lowercase())
        .collect();
    let has_dir = |d: &str| dirs.iter().any(|c| c == d);
    let component_agent = dirs.iter().find_map(|c| agent_of_component(c));
    let agent = component_agent.or(hint);

    // Whole-file names, wherever they sit.
    match lower.as_str() {
        "claude.md" | "claude.local.md" => {
            return Some((Agent::ClaudeCode, FileKind::Instructions));
        }
        "gemini.md" => return Some((Agent::Gemini, FileKind::Instructions)),
        "agents.md" | "agents.override.md" => {
            let a = match agent {
                Some(Agent::Codex) => Agent::Codex,
                _ => Agent::Generic,
            };
            return Some((a, FileKind::Instructions));
        }
        ".cursorrules" => return Some((Agent::Cursor, FileKind::Rule)),
        ".windsurfrules" => return Some((Agent::Windsurf, FileKind::Rule)),
        ".clinerules" => return Some((Agent::Cline, FileKind::Rule)),
        "copilot-instructions.md"
            if dirs.last().map(String::as_str) == Some(".github") || dirs.is_empty() =>
        {
            return Some((Agent::Copilot, FileKind::Instructions));
        }
        _ => {}
    }
    if lower.ends_with(".instructions.md")
        && dirs.last().map(String::as_str) == Some("instructions")
    {
        return Some((Agent::Copilot, FileKind::Instructions));
    }
    // Folders of rules, commands, skills and memories.
    if dirs.last().map(String::as_str) == Some(".clinerules") && is_text(name) {
        return Some((Agent::Cline, FileKind::Rule));
    }
    if (has_dir("rules") || has_dir("rule")) && is_text(name) {
        let a = match agent {
            Some(a @ (Agent::Cursor | Agent::Windsurf | Agent::Cline)) => a,
            _ if lower.ends_with(".mdc") => Agent::Cursor,
            _ => return None,
        };
        return Some((a, FileKind::Rule));
    }
    if lower == "skill.md"
        && rel.len() >= 3
        && dirs.get(dirs.len().wrapping_sub(2)).map(String::as_str) == Some("skills")
    {
        return Some((agent.unwrap_or(Agent::Generic), FileKind::Skill));
    }
    if (dirs.last().map(String::as_str) == Some("commands")
        || dirs.last().map(String::as_str) == Some("prompts"))
        && lower.ends_with(".md")
    {
        let a = agent?;
        return Some((a, FileKind::Command));
    }
    if (has_dir("memory") || has_dir("memories")) && is_text(name) {
        return Some((agent.unwrap_or(Agent::Generic), FileKind::Memory));
    }
    None
}

/// Folder names the scan does not enter: build output, dependencies, and the
/// agents' caches and plugin trees (a scan of one plugin folder is still
/// possible by naming it as a root).
pub(crate) fn skip_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | ".git"
            | "target"
            | "dist"
            | "build"
            | ".next"
            | ".venv"
            | "venv"
            | "__pycache__"
            | "Pods"
            | "plugins"
            | "cache"
            | "paste-cache"
            | "file-history"
            | "shell-snapshots"
            | "telemetry"
            | "statsig"
            | "todos"
            | "debug"
            | "ide"
            | "Library"
            | ".Trash"
    )
}
