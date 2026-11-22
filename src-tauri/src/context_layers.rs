//! Context layers, read side (CX4 home layer, CX5 org layer): what a fresh
//! chat session is told about this machine and the organization, on top of
//! the operator profile (`me/`).
//!
//! **Short core, the rest on demand** (card 4, 2026-11-14). A session starts
//! with two small sections appended to the `<operator-context>` digest by
//! the context compiler (`context_compiler::Profile::read_within`):
//! - `## home/<slug>.md`: this machine's home file from the personal clone
//!   (`context_roots::root(Layer::Personal)`, the slug from
//!   `~/.config/hollow/machine.json`), cut to whole lines within
//!   [`HOME_CAP`] bytes. Topic files it links are not loaded.
//! - `## personal/pinned.md` (PX3): the memories marked `pinned: true`, whole,
//!   within `context_pins::PINNED_CAP` bytes; see `context_pins.rs`.
//! - `## personal/INDEX.md` (PX2): an index of the personal clone's memories
//!   and learnings, generated from each file's `description:` (a learning's
//!   `title:`), one line per file grouped by type, within [`INDEX_CAP`]
//!   bytes; see [`personal_index`]. Built on each read, never written, so a
//!   new memory appears in it after the next sync.
//! - `## org/INDEX.md`: the org clone's `INDEX.md`, cut within [`ORG_CAP`]
//!   bytes, or, while the repo has none, a generated list of its top-level
//!   folders and Markdown files. A line names the clone's folder: the agent
//!   reads a file there when the task needs it, never the whole layer, and
//!   never writes in it (changes go by pull request, a later row).
//!
//! Both are read only, through `context_roots::read_in_root` (no symlink
//! followed). Nothing is sent while a root or the slug is unset, so Hollow
//! behaves as before until the person sets them. A file that can't be read
//! is skipped with a log line, never a failed turn.
//!
//! **Why inside the profile block.** The sections ride the profile's
//! existing paths: a fresh session gets them once, a resumed one gets a
//! changed line as the `<operator-update>` delta, the manifest counts them
//! with the profile, and the turn budget treats them as settled (fixed size,
//! never cut) like Decisions and Guardrails (`context_compiler::profile_split`).
//!
//! **Which machine.** The slug names the machine Hollow runs on. A turn
//! placed on a remote runner still gets this machine's file (named as such
//! in the section's first line); the runner's own file waits for CX12.
//!
//! **Freshness of the org clone.** [`spawn_pull_timer`] runs
//! `git pull --ff-only` in the org clone shortly after launch and every 30
//! minutes, on its own thread with a timeout, never on a turn's path. It
//! never clones, never pushes and never touches the personal clone.
//!
//! **The rest on demand** (CX5b). Every Markdown or text file of the org
//! clone is a retrieval source of kind [`ORG_KIND`] ([`org_passages`], called
//! by `retrieval_broker`): hidden entries (`.git`, `.github`), the top-level
//! `tools/` and symlinks are skipped, each file is read through
//! `read_in_root` (no symlink followed) within [`READ_MAX`], and a request
//! reads at most [`ORG_FILES`] files and [`ORG_BYTES`] bytes. A passage cites
//! the file's absolute path in the clone. Retrieval and the sections above
//! read the working tree as it is: neither waits on the pull, which takes no
//! lock a reader honours.
//!
//! **What refuses a write in the clone** (CX5b; [`ORG_BLOCK_LIMITS`] says
//! what doesn't). Claude, every mode: [`write_rules_for`]'s `Edit` rule
//! (Edit, Write, NotebookEdit) and `Bash` rules on command text naming the
//! clone. Codex auto in a checked linked worktree: its permission profile
//! names the clone `read` (`agent::codex_auto_profile_overrides`), which
//! holds even inside a writable folder; Codex auto never gets the clone, or
//! a folder in it, as a writable root.

use crate::context_roots as roots;
use std::path::{Path, PathBuf};

/// The home file's always-on cap, in bytes (plan section 3, rule 7).
pub(crate) const HOME_CAP: usize = 4_000;
/// The org index's always-on cap, in bytes.
pub(crate) const ORG_CAP: usize = 2_000;
/// The generated personal index's always-on cap, in bytes (PX2).
pub(crate) const INDEX_CAP: usize = 2_000;
/// The label the digest heading names for the personal index.
pub(crate) const INDEX_PATH: &str = "personal/INDEX.md";
/// A listed line's description is cut to this many characters.
const INDEX_LINE: usize = 110;
/// Files the index reads at most (memories and learnings together).
const INDEX_FILES: usize = 400;
/// Largest file read before cutting: a bigger one is skipped.
pub(crate) const READ_MAX: u64 = 256 * 1024;
/// The line that ends a section cut to its cap.
pub(crate) const CUT_MARK: &str = "…(cut: read the file for the rest)";
/// Generated index entries at most.
const MAX_LISTED: usize = 40;
/// The retrieval source kind of an org clone file.
pub(crate) const ORG_KIND: &str = "org";
/// Org files read per retrieval request at most.
pub(crate) const ORG_FILES: usize = 400;
/// Bytes of org files read per retrieval request at most.
pub(crate) const ORG_BYTES: u64 = 4 * 1024 * 1024;
/// Folders deep the org walk goes, and entries it looks at, at most.
const ORG_DEPTH: usize = 8;
const ORG_ENTRIES: usize = 20_000;

/// What the org write block can't refuse (CX5b), named so the docs and a
/// test say the same thing.
#[cfg_attr(not(test), allow(dead_code))] // the docs' text, pinned by tests
pub(crate) const ORG_BLOCK_LIMITS: &str = "Not refused: a Claude Bash command that reaches the clone without naming its path (a relative path, another spelling, a variable, a script); Codex in full access (no sandbox), and Codex default, Accept edits or auto outside a checked worktree when the clone sits inside the run's folder, a writable root, /tmp or $TMPDIR; Grok and Antigravity (no org rule). Over-refused: the Bash rules match a prefix, so a custom root also refuses commands naming a sibling that starts with its path (root ~/org refuses ~/organizer too); the default ~/Hollow/Context/org has no such sibling. The Bash rules are tested as generated text, not against a live Claude CLI.";

/// The org clone's root, when set and checked ([`roots::root`]); tests get
/// none unless they set [`set_test_org_root`].
pub(crate) fn org_root() -> Option<PathBuf> {
    #[cfg(test)]
    {
        TEST_ORG_ROOT.with(|r| r.borrow().clone())
    }
    #[cfg(not(test))]
    {
        roots::root(roots::Layer::Org)
    }
}

#[cfg(test)]
thread_local! {
    static TEST_ORG_ROOT: std::cell::RefCell<Option<PathBuf>> =
        const { std::cell::RefCell::new(None) };
}

/// Sets what [`org_root`] returns on this test thread.
#[cfg(test)]
pub(crate) fn set_test_org_root(root: Option<PathBuf>) {
    TEST_ORG_ROOT.with(|r| *r.borrow_mut() = root);
}

/// A layer section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Section {
    /// The label the digest heading names (`home/<slug>.md`, `org/INDEX.md`).
    pub path: String,
    /// What the digest holds: the lead line and the text cut to its cap.
    pub text: String,
    /// The lead line and the whole text, never cut: what still holds, so
    /// the revision delta never reports a line past the cap as removed.
    pub whole: String,
}

/// The layer sections for a fresh session now: the home file, the generated
/// personal index (PX2), then the org index, each only when its root (and for
/// home, the slug) is set and it reads. A stale home file or memory is
/// labelled and ranked last there, and offered once for recheck in `vault`'s
/// review queue (ME2, `context_stale`). Tests get none unless they set [`set_test_sections`].
pub(crate) fn sections(vault: &Path) -> Vec<Section> {
    #[cfg(test)]
    {
        let _ = vault;
        TEST_SECTIONS.with(|s| s.borrow().clone())
    }
    #[cfg(not(test))]
    {
        let mut out = Vec::new();
        let today = chrono::Local::now().date_naive();
        let mut stale = Vec::new();
        let personal = roots::root(roots::Layer::Personal);
        if let (Some(root), Some(slug)) = (personal.as_ref(), roots::machine_slug()) {
            if let Some(section) = home_section_on(root, &slug, today, &mut stale) {
                out.push(section);
            }
        }
        if let Some(root) = personal.as_ref() {
            // PX3: pinned memories ahead of the generated index.
            if let Some(section) = crate::context_pins::pinned_section(root) {
                out.push(section);
            }
            if let Some(section) = personal_index_on(root, today, &mut stale) {
                out.push(section);
            }
            crate::context_stale::offer_in_background(vault, root, stale);
        }
        if let Some(root) = org_root() {
            if let Some(section) = org_section(&root) {
                out.push(section);
            }
        }
        out
    }
}

#[cfg(test)]
thread_local! {
    static TEST_SECTIONS: std::cell::RefCell<Vec<Section>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets what [`sections`] returns on this test thread: `(path, text)`
/// pairs, each whole as given.
#[cfg(test)]
pub(crate) fn set_test_sections(sections: Vec<(String, String)>) {
    let sections = sections
        .into_iter()
        .map(|(path, text)| Section {
            path,
            whole: text.clone(),
            text,
        })
        .collect();
    TEST_SECTIONS.with(|s| *s.borrow_mut() = sections);
}

/// `text` trimmed and cut to whole lines within `cap` bytes, the cut mark
/// included; whole when it fits.
pub(crate) fn cut(text: &str, cap: usize) -> String {
    let text = text.trim();
    if text.len() <= cap {
        return text.to_string();
    }
    let room = cap.saturating_sub(CUT_MARK.len() + 1);
    let mut end = room.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let end = text[..end].rfind('\n').unwrap_or(0);
    let kept = text[..end].trim_end();
    if kept.is_empty() {
        CUT_MARK.to_string()
    } else {
        format!("{kept}\n{CUT_MARK}")
    }
}

/// Frontmatter dropped, the way the profile's `me/` files are read.
fn body_of(raw: &str) -> String {
    crate::frontmatter::parse(raw).body.trim().to_string()
}

/// The home section for `slug` from the personal clone at `root`, as of today.
#[cfg(test)]
pub(crate) fn home_section(root: &Path, slug: &str) -> Option<Section> {
    home_section_on(
        root,
        slug,
        chrono::Local::now().date_naive(),
        &mut Vec::new(),
    )
}

/// The home section for `slug` from the personal clone at `root` as of
/// `today`. A stale file (`context_stale::home_staleness`) says so in the
/// lead line and is added to `stale`.
pub(crate) fn home_section_on(
    root: &Path,
    slug: &str,
    today: chrono::NaiveDate,
    stale: &mut Vec<crate::context_stale::StaleFile>,
) -> Option<Section> {
    let rel = roots::home_file(slug).ok()?;
    let raw = match roots::read_in_root(root, &rel, READ_MAX) {
        Ok(Some(raw)) => raw,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("[context-layers] {rel} could not be read: {error}");
            return None;
        }
    };
    let body = body_of(&raw);
    if body.is_empty() {
        return None;
    }
    let mut lead = format!(
        "The machine Hollow runs on (`{slug}`). Its topic files sit beside it in {}; read one when the task needs it.",
        root.join("home").display()
    );
    if let Some(staleness) = crate::context_stale::home_staleness(&raw, today) {
        lead.push_str(&format!(
            " This file is {} (recheck a fact here before relying on it).",
            staleness.label()
        ));
        stale.push(crate::context_stale::StaleFile {
            rel: rel.clone(),
            staleness,
        });
    }
    let text = cut(&body, HOME_CAP.saturating_sub(lead.len() + 2));
    Some(Section {
        path: rel,
        text: format!("{lead}\n\n{text}"),
        whole: format!("{lead}\n\n{body}"),
    })
}

/// `key: value`'s value in the frontmatter of `raw` (the first line whose
/// trimmed key is `key`, top level or under `metadata:`), unquoted, or
/// `None`. Reads only the block between the opening and closing `---`.
pub(crate) fn fm_value(raw: &str, key: &str) -> Option<String> {
    let mut lines = raw.trim_start_matches('\u{feff}').lines();
    if lines.next()?.trim_end() != "---" {
        return None;
    }
    for line in lines {
        if line.trim_end() == "---" {
            break;
        }
        let Some((k, v)) = line.trim().split_once(':') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        // The writer escapes `\"`, `\\` and newlines in a quoted value: read it back.
        let v = crate::frontmatter::unquote(v.trim());
        return Some(v.trim().to_string()).filter(|v| !v.is_empty());
    }
    None
}

/// `text` on one line, cut to [`INDEX_LINE`] characters with an ellipsis.
fn one_line(text: &str) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= INDEX_LINE {
        return flat;
    }
    let kept: String = flat.chars().take(INDEX_LINE - 1).collect();
    format!("{}…", kept.trim_end())
}

/// The heading a memory's `type` is listed under.
fn index_group(kind: &str) -> &'static str {
    match kind {
        "feedback" => "Feedback",
        "project" => "Project",
        "reference" => "Reference",
        "user" => "About you",
        _ => "Other",
    }
}

/// The groups in the order the index lists them; learnings, then stale
/// memories (ME2), come last.
const INDEX_GROUPS: [&str; 7] = [
    "Feedback",
    "Project",
    "Reference",
    "About you",
    "Other",
    "Learnings",
    crate::context_stale::RECHECK_GROUP,
];

/// The generated index of the personal clone at `root` (PX2): one line per
/// Markdown file of `memory/` and `learnings/` (top level only: Claude's own
/// per-project folders under `memory/<project>/` are not read; no `README.md`;
/// a learning another replaced is left out),
/// `- <file name>: <description>`, grouped by the memory's `type`
/// (learnings, which carry a `title` and no description, under their own
/// heading), within [`INDEX_CAP`] bytes. The label is [`INDEX_PATH`]. Files
/// read through `read_in_root` (no symlink followed), at most [`INDEX_FILES`].
/// A file with no description is listed by name alone.
///
/// Nothing is cut silently: past the cap the last line says how many files
/// are not listed and that consolidating memories makes them fit, and the
/// section's `whole` text keeps every line (so the revision delta never
/// reports a line past the cap as removed). Nothing is written, so the index
/// can't drift from the files: it is rebuilt on each read. `None` when the
/// clone holds no such file.
///
/// A stale memory (ME2, `context_stale::memory_staleness` as of `today`) is
/// ranked last: listed under `context_stale::RECHECK_GROUP` after every
/// other group, its line labelled, so the cap cuts it first; it is added to
/// `stale`.
pub(crate) fn personal_index_on(
    root: &Path,
    today: chrono::NaiveDate,
    stale: &mut Vec<crate::context_stale::StaleFile>,
) -> Option<Section> {
    roots::check_root(root).ok()?;
    let mut groups: Vec<(&'static str, Vec<String>)> =
        INDEX_GROUPS.iter().map(|g| (*g, Vec::new())).collect();
    let mut total = 0usize;
    // Files past [`INDEX_FILES`], counted but not read.
    let mut unread = 0usize;
    for folder in ["memory", "learnings"] {
        let dir = root.join(folder);
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
            continue;
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            // A name with a control character could forge a line of the index.
            .filter(|n| n.ends_with(".md") && n != "README.md" && !n.chars().any(char::is_control))
            .collect();
        names.sort();
        for (i, name) in names.iter().enumerate() {
            if total >= INDEX_FILES {
                unread += names.len() - i;
                break;
            }
            let Ok(Some(raw)) = roots::read_in_root(root, &format!("{folder}/{name}"), READ_MAX)
            else {
                continue;
            };
            // A learning another one replaced is history, not an index line.
            if folder == "learnings"
                && (fm_value(&raw, "superseded_by").is_some()
                    || fm_value(&raw, "status").as_deref() == Some("superseded"))
            {
                continue;
            }
            total += 1;
            let stem = name.trim_end_matches(".md");
            let (group, about) = if folder == "learnings" {
                ("Learnings", fm_value(&raw, "title"))
            } else {
                let kind = fm_value(&raw, "type").unwrap_or_default();
                (index_group(&kind), fm_value(&raw, "description"))
            };
            let mut line = match about {
                Some(about) => format!("- {stem}: {}", one_line(&about)),
                None => format!("- {stem}"),
            };
            let staleness = (folder == "memory")
                .then(|| crate::context_stale::memory_staleness(&raw, today))
                .flatten();
            let group = match staleness {
                Some(staleness) => {
                    line.push_str(&format!(" ({})", staleness.label()));
                    stale.push(crate::context_stale::StaleFile {
                        rel: format!("{folder}/{name}"),
                        staleness,
                    });
                    crate::context_stale::RECHECK_GROUP
                }
                None => group,
            };
            if let Some(entry) = groups.iter_mut().find(|entry| entry.0 == group) {
                entry.1.push(line);
            }
        }
    }
    if total == 0 && unread == 0 {
        return None;
    }
    let lead = format!(
        "Your saved memories and learnings, in {}. Read a file there (memory/<name>.md or learnings/<name>.md) when its line fits the task; the index is generated, so don't edit it.",
        root.display()
    );
    // Heading lines and file lines in order; a heading is kept only with a
    // line under it.
    let mut all: Vec<String> = Vec::new();
    for (group, lines) in &groups {
        if lines.is_empty() {
            continue;
        }
        all.push(format!("### {group}"));
        all.extend(lines.iter().cloned());
    }
    let whole = format!("{lead}\n\n{}", all.join("\n"));
    if whole.len() <= INDEX_CAP && unread == 0 {
        return Some(Section {
            path: INDEX_PATH.to_string(),
            text: whole.clone(),
            whole,
        });
    }
    // Over the cap: whole lines that leave room for the closing note.
    let note = |left: usize| {
        format!("…({left} more file(s) not listed: the index holds {INDEX_CAP} bytes; merge or retire memories so it fits)")
    };
    let room = INDEX_CAP.saturating_sub(lead.len() + 2);
    let mut kept: Vec<&String> = Vec::new();
    let mut used = 0usize;
    let mut files_kept = 0usize;
    for line in &all {
        let reserve = note(total + unread).len() + 1;
        if used + line.len() + 1 + reserve > room {
            break;
        }
        used += line.len() + 1;
        if !line.starts_with("### ") {
            files_kept += 1;
        }
        kept.push(line);
    }
    // A heading with nothing under it is dropped.
    while kept.last().is_some_and(|l| l.starts_with("### ")) {
        kept.pop();
    }
    let body = kept
        .iter()
        .map(|l| l.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!("{lead}\n\n{body}\n{}", note(total - files_kept + unread));
    Some(Section {
        path: INDEX_PATH.to_string(),
        text,
        whole,
    })
}

/// [`personal_index_on`] as of today, for tests.
#[cfg(test)]
pub(crate) fn personal_index(root: &Path) -> Option<Section> {
    personal_index_on(root, chrono::Local::now().date_naive(), &mut Vec::new())
}

/// The org section from the org clone at `root`: its `INDEX.md`, or a
/// generated list of top-level entries while it has none.
pub(crate) fn org_section(root: &Path) -> Option<Section> {
    let lead = format!(
        "Org context, read-only clone at {}. Read files with Read, Grep or Glob (not Bash) as needed; never write in it.",
        root.display()
    );
    let room = ORG_CAP.saturating_sub(lead.len() + 2);
    let index = match roots::read_in_root(root, "INDEX.md", READ_MAX) {
        Ok(Some(raw)) => Some(body_of(&raw)).filter(|b| !b.is_empty()),
        Ok(None) => None,
        Err(error) => {
            eprintln!("[context-layers] the org INDEX.md could not be read: {error}");
            None
        }
    };
    let body = match index {
        Some(index) => index,
        None => listing(root, room)?,
    };
    Some(Section {
        path: "org/INDEX.md".to_string(),
        text: format!("{lead}\n\n{}", cut(&body, room)),
        whole: format!("{lead}\n\n{body}"),
    })
}

/// Top-level folders (with a trailing `/`) and Markdown files of `root`,
/// sorted, hidden entries, symlinks and names with control characters left
/// out, as many as fit `room` bytes; `None` when there are none.
fn listing(root: &Path, room: usize) -> Option<String> {
    roots::check_root(root).ok()?;
    let mut names: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_string();
            if name.starts_with('.') || name.chars().any(char::is_control) {
                return None;
            }
            let kind = entry.file_type().ok()?;
            if kind.is_dir() {
                Some(format!("{name}/"))
            } else if kind.is_file() && name.ends_with(".md") {
                Some(name)
            } else {
                None
            }
        })
        .collect();
    if names.is_empty() {
        return None;
    }
    names.sort();
    names.truncate(MAX_LISTED);
    let mut out = String::from("No INDEX.md yet. Top-level entries: ");
    for (i, name) in names.iter().enumerate() {
        let sep = if i == 0 { "" } else { ", " };
        if out.len() + sep.len() + name.len() > room {
            break;
        }
        out.push_str(sep);
        out.push_str(name);
    }
    Some(out)
}

/// Where a local Claude turn's auto memory goes (CX4b): `memory/<project>`
/// in the personal clone, the project named the way Claude Code names its
/// own folder (`run_manifest::claude_project_slug` of the repository root
/// holding `cwd`), so every worktree of a repository shares one folder (two
/// paths that differ only in punctuation share a slug too, as they do for
/// the CLI). The turn gets it as `autoMemoryDirectory`
/// (`agent::claude_settings`). `None` while no personal root is set or
/// usable, and for a per-run scratch working folder ([`is_scratch_cwd`]), so
/// the CLI keeps its default folder under its config dir. Once the key is
/// set the CLI no longer loads the old `~/.claude/projects/<slug>/memory`
/// notes (nothing is copied; see `docs/AGENT_CONTEXT.md`). Tests get a root
/// only through [`test_root::set`].
pub(crate) fn auto_memory_dir(cwd: &Path) -> Option<std::path::PathBuf> {
    #[cfg(test)]
    {
        let (root, scratch) = test_root::get()?;
        auto_memory_dir_with(&root, cwd, &scratch)
    }
    #[cfg(not(test))]
    {
        let root = roots::root(roots::Layer::Personal)?;
        auto_memory_dir_with(&root, cwd, &scratch_roots())
    }
}

/// [`auto_memory_dir_in`] unless `cwd` is under one of `scratch`.
pub(crate) fn auto_memory_dir_with(
    root: &Path,
    cwd: &Path,
    scratch: &[std::path::PathBuf],
) -> Option<std::path::PathBuf> {
    if is_scratch_cwd(cwd, scratch) {
        eprintln!(
            "[context-layers] auto memory stays default for the scratch folder {}",
            cwd.display()
        );
        return None;
    }
    auto_memory_dir_in(root, cwd)
}

/// The folders per-run scratch working folders live under: the OS temp dir
/// (`$TMPDIR`) and `/tmp` (`/private/tmp` on macOS, where Hollow's
/// `hollow-run-*` folders are), each also by its real path.
#[cfg(not(test))]
fn scratch_roots() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for p in [std::env::temp_dir(), "/tmp".into(), "/private/tmp".into()] {
        if let Ok(real) = std::fs::canonicalize(&p) {
            out.push(real);
        }
        out.push(p);
    }
    out
}

/// Whether `cwd` (by its real path too) is inside one of the `scratch`
/// roots: a scratch folder holds no project, so a memory folder named for it
/// would only litter the clone.
pub(crate) fn is_scratch_cwd(cwd: &Path, scratch: &[std::path::PathBuf]) -> bool {
    let real = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    scratch
        .iter()
        .any(|r| cwd.starts_with(r) || real.starts_with(r))
}

/// [`auto_memory_dir`] for the personal clone at `root`. The folder is
/// created, then resolved again so a symlink anywhere on the way (`memory`
/// itself included) refuses it; every failure is a log line and `None`,
/// never a failed turn. Existing files in `memory/` stay where they are.
pub(crate) fn auto_memory_dir_in(root: &Path, cwd: &Path) -> Option<std::path::PathBuf> {
    let cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let project =
        crate::run_manifest::claude_project_slug(&crate::run_manifest::claude_memory_root(&cwd));
    let skip = |why: String| {
        eprintln!(
            "[context-layers] auto memory stays default for {}: {why}",
            cwd.display()
        );
        None
    };
    let rel = match roots::memory_dir(&project) {
        Ok(rel) => rel,
        Err(e) => return skip(e.to_string()),
    };
    let dir = match roots::resolve_in_root(root, &rel) {
        Ok(dir) => dir,
        Err(e) => return skip(e.to_string()),
    };
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return skip(format!("can't create {}: {e}", dir.display()));
    }
    match roots::resolve_in_root(root, &rel) {
        Ok(d) if d.is_dir() => Some(d),
        Ok(d) => skip(format!("{} is not a folder", d.display())),
        Err(e) => skip(e.to_string()),
    }
}

/// Test-only personal root and scratch roots for [`auto_memory_dir`], per
/// thread (a test's `build_command` runs on its own thread).
#[cfg(test)]
pub(crate) mod test_root {
    use std::cell::RefCell;
    use std::path::PathBuf;
    type Slot = Option<(PathBuf, Vec<PathBuf>)>;
    thread_local!(static SLOT: RefCell<Slot> = const { RefCell::new(None) });
    pub(crate) fn get() -> Slot {
        SLOT.with(|s| s.borrow().clone())
    }
    /// Clears itself when dropped.
    pub(crate) struct Guard;
    pub(crate) fn set(root: PathBuf, scratch: Vec<PathBuf>) -> Guard {
        SLOT.with(|s| *s.borrow_mut() = Some((root, scratch)));
        Guard
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            SLOT.with(|s| *s.borrow_mut() = None);
        }
    }
}

/// Claude permission rules that refuse writes in the org clone, passed with
/// `--disallowed-tools` on every Claude run while the org root is set (CX5:
/// the clone is read only; changes go by pull request). Deny rules hold in
/// every permission mode. See [`write_rules_for`]; tests get none unless
/// they set [`set_test_org_root`].
pub(crate) fn org_write_rules() -> Vec<String> {
    let Some(root) = org_root() else {
        return Vec::new();
    };
    write_rules_for(&root, dirs::home_dir().as_deref()).unwrap_or_else(|| {
        eprintln!(
            "[context-layers] no edit block for the org clone at {}: its path has characters a Claude rule can't carry",
            root.display()
        );
        Vec::new()
    })
}

/// [`org_write_rules`] for `root`, `home` the home folder:
/// - `Edit(//<root>/**)` (a leading `//` is an absolute path in Claude's
///   rule syntax): Edit, Write and NotebookEdit anywhere in the clone;
/// - `Bash(*<root>*)`, and with `home` above the root `Bash(*~/<rel>*)` and
///   `Bash(*$HOME/<rel>*)`: any shell command whose text names the clone by
///   one of those spellings, reads included (the Read, Grep and Glob tools
///   still read it). Claude checks each part of a compound command, so a
///   `cd` there, a redirect, `tee`, `sed -i`, `mv`, `cp`, `rm` or
///   `git -C` naming it is refused. Matching is on command text only:
///   [`ORG_BLOCK_LIMITS`] says what gets past it.
///
/// `None` for a path a rule can't carry plainly: anything but letters,
/// digits and `/._-~` (the CLI splits its list at spaces and commas, and
/// `*` or parentheses would change the rule).
pub(crate) fn write_rules_for(root: &Path, home: Option<&Path>) -> Option<Vec<String>> {
    let path = root.to_str()?.trim_end_matches('/');
    let plain = |c: char| c.is_ascii_alphanumeric() || "/._-~".contains(c);
    if !path.starts_with('/') || path.len() < 2 || !path.chars().all(plain) {
        return None;
    }
    let mut rules = vec![format!("Edit(/{path}/**)"), format!("Bash(*{path}*)")];
    let rel = home
        .and_then(|h| Path::new(path).strip_prefix(h).ok())
        .and_then(Path::to_str)
        .filter(|rel| !rel.is_empty());
    if let Some(rel) = rel {
        rules.push(format!("Bash(*~/{rel}*)"));
        rules.push(format!("Bash(*$HOME/{rel}*)"));
    }
    Some(rules)
}

/// One org clone file a retrieval request may read.
struct OrgFile {
    /// Relative to the clone's root, `/`-separated.
    rel: String,
    size: u64,
}

/// The org clone's Markdown and text files (`.md`, `.markdown`, `.txt`),
/// sorted by relative path: hidden entries, the top-level `tools/`, symlinks,
/// other file kinds, names with control characters and the names retrieval
/// keeps out of connected projects too (a `restricted` folder, a likely
/// secret such as `credentials.md`: `project_text_index::private`) left out,
/// at most [`ORG_DEPTH`] folders deep and [`ORG_ENTRIES`] entries looked at.
/// The flag says the entry limit stopped the walk. Empty when the root fails
/// [`roots::check_root`].
fn org_files(root: &Path) -> (Vec<OrgFile>, bool) {
    let mut out = Vec::new();
    if roots::check_root(root).is_err() {
        return (out, false);
    }
    let mut seen = 0usize;
    let mut truncated = false;
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    'walk: while let Some((dir, prefix, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            seen += 1;
            if seen > ORG_ENTRIES {
                truncated = true;
                break 'walk;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if name.starts_with('.')
                || name.chars().any(char::is_control)
                || crate::project_text_index::private(&name)
            {
                continue;
            }
            // `DirEntry::file_type` doesn't follow a symlink: one is neither.
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                if depth + 1 < ORG_DEPTH && !(depth == 0 && name == "tools") {
                    stack.push((entry.path(), format!("{rel}/"), depth + 1));
                }
            } else if kind.is_file() {
                let lower = name.to_ascii_lowercase();
                if [".md", ".markdown", ".txt"]
                    .iter()
                    .any(|x| lower.ends_with(x))
                {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    out.push(OrgFile { rel, size });
                }
            }
        }
    }
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    (out, truncated)
}

/// The org clone's passages for one request (kind [`ORG_KIND`]), pushed to
/// `scored` the way connected project files are: files whose path names a
/// term first, then by path, within [`ORG_FILES`] files and [`ORG_BYTES`]
/// bytes (a warning says when files were left out: past the budget, over
/// the per-file size, or past the walk's entry limit), up to
/// `LINES_PER_NOTE` lines each. A passage's path is the file's absolute
/// path in the clone. Returns the files read.
pub(crate) fn org_passages(
    root: &Path,
    terms: &[crate::vault_search::Term],
    scored: &mut Vec<crate::retrieval_broker::Scored>,
    warnings: &mut Vec<String>,
) -> usize {
    use crate::retrieval_broker::{Scored, LINES_PER_NOTE};
    let (files, truncated) = org_files(root);
    let too_large = files.iter().filter(|f| f.size > READ_MAX).count();
    let mut ranked: Vec<(u32, &OrgFile)> = files
        .iter()
        .filter(|f| f.size <= READ_MAX)
        .map(|f| {
            let name = f.rel.to_lowercase();
            (crate::vault_search::name_mask(terms, &[name.as_str()]), f)
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.0.count_ones()
            .cmp(&a.0.count_ones())
            .then(a.1.rel.cmp(&b.1.rel))
    });
    let mut bytes = 0u64;
    let mut chosen: Vec<(u32, &OrgFile)> = Vec::new();
    for &(mask, f) in &ranked {
        if chosen.len() >= ORG_FILES {
            break;
        }
        if bytes + f.size > ORG_BYTES {
            continue;
        }
        bytes += f.size;
        chosen.push((mask, f));
    }
    if chosen.len() < ranked.len() || too_large > 0 || truncated {
        let mut warning = format!(
            "Retrieval read {} of {} organization context files ({ORG_FILES} files and {} MB per request; files whose path names a term first)",
            chosen.len(),
            files.len(),
            ORG_BYTES / (1024 * 1024)
        );
        if too_large > 0 {
            warning.push_str(&format!(
                "; {too_large} over {} KB not read",
                READ_MAX / 1024
            ));
        }
        if truncated {
            warning.push_str(&format!(
                "; the clone has more than {ORG_ENTRIES} entries, so some files were never listed"
            ));
        }
        warning.push('.');
        warnings.push(warning);
    }
    let scorer = crate::vault_search::LineScorer::new(terms);
    let mut read = 0;
    for (mask, f) in chosen {
        let text = match roots::read_in_root(root, &f.rel, READ_MAX) {
            Ok(Some(text)) => text,
            Ok(None) => continue,
            Err(error) => {
                eprintln!("[context-layers] org/{} not read: {error}", f.rel);
                continue;
            }
        };
        read += 1;
        let path = root.join(&f.rel).to_string_lossy().into_owned();
        for ev in scorer.best_lines(&text, mask, LINES_PER_NOTE) {
            scored.push(Scored {
                band: 0,
                coverage: ev.coverage,
                in_line: ev.in_line,
                vault: false,
                recency: String::new(),
                passage: crate::directory_connections::Passage {
                    path: path.clone(),
                    line: ev.line,
                    excerpt: ev.excerpt,
                    source: ORG_KIND.into(),
                    coverage: "excerpt".into(),
                    score: u32::try_from(ev.coverage).unwrap_or(u32::MAX),
                    semantic: None,
                    lifecycle: None,
                    note: None,
                    current: None,
                },
            });
        }
    }
    read
}

/// How long one pull may take before it is stopped.
const PULL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
/// Between pulls.
const PULL_EVERY: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// The first pull, after launch settles.
const FIRST_PULL: std::time::Duration = std::time::Duration::from_secs(30);

/// Starts the org clone's fast-forward pull timer (one thread for the app's
/// life). Each round re-reads the saved root, so setting it later works.
pub(crate) fn spawn_pull_timer() {
    spawn_pull_loop(FIRST_PULL, PULL_EVERY, || {
        if let Some(root) = roots::root(roots::Layer::Org) {
            if root.join(".git").is_dir() {
                pull(&root);
            }
        }
    })
    .map(drop)
    .unwrap_or_else(|error| eprintln!("[context-layers] no pull timer: {error}"));
}

/// [`spawn_pull_timer`]'s loop with its times and round given: returns once
/// the thread is started, so a round that hangs holds only that thread.
fn spawn_pull_loop(
    first: std::time::Duration,
    every: std::time::Duration,
    round: impl Fn() + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("org-context-pull".into())
        .spawn(move || {
            std::thread::sleep(first);
            loop {
                round();
                std::thread::sleep(every);
            }
        })
}

/// One `git pull --ff-only --quiet` in `root`, stopped at [`PULL_TIMEOUT`];
/// no prompt for credentials, failures only logged.
fn pull(root: &Path) {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    // No hook or fsmonitor of the clone's own config runs, no prompt of any
    // kind waits, and the pull is its own process group so a timeout stops
    // its transport helpers too.
    let child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "pull",
            "--ff-only",
            "--quiet",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            eprintln!("[context-layers] org pull did not start: {error}");
            return;
        }
    };
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    eprintln!("[context-layers] org pull ended with {status}");
                }
                return;
            }
            Ok(None) if started.elapsed() < PULL_TIMEOUT => {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            _ => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    // SAFETY: killpg only delivers a signal, to the group
                    // this pull leads (spawned with process_group(0)).
                    unsafe {
                        libc::killpg(pid, libc::SIGKILL);
                    }
                }
                let _ = child.kill();
                let _ = child.wait();
                eprintln!("[context-layers] org pull stopped after {PULL_TIMEOUT:?}");
                return;
            }
        }
    }
}

// ── Context health (PX7) ────────────────────────────────────────────────

/// The five profile files whose two copies the doctor compares. `me/README.md`
/// is left out: it is a note that differs on purpose.
pub(crate) const PROFILE_FILES: [&str; 5] = [
    "Identity.md",
    "Communication.md",
    "Anti-patterns.md",
    "Guardrails.md",
    "Decisions.md",
];
/// Days after which a memory's `verified_at` reads as stale.
pub(crate) const MEMORY_STALE_DAYS: i64 = 180;
/// Memory files the doctor reads at most.
const MEMORY_SCAN_MAX: usize = 500;

/// What the capability doctor shows about the personal context: whether the
/// two copies of `me/` agree and how fresh the home file and the memories
/// are. Read only, computed from the files as they are.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextHealth {
    /// The clone has a `me/` folder, so the two copies could be compared.
    pub me_compared: bool,
    /// Profile files that differ between the vault and the clone, or sit in
    /// only one of them, each with the reason in brackets.
    pub me_differs: Vec<String>,
    /// The home file's last `Last checked: YYYY-MM-DD` date.
    pub home_checked: Option<String>,
    /// That date's age in days (negative when it is in the future).
    pub home_age_days: Option<i64>,
    /// The home file's staleness label (`context_stale::home_staleness`, the
    /// rule recall uses), `None` when it is fresh or undated.
    pub home_stale: Option<String>,
    /// Memory files read (`memory/*.md`, not `README.md`).
    pub memory_files: usize,
    /// How many of them carry a readable `verified_at` or `valid_until`.
    pub memory_dated: usize,
    /// Memories past their `valid_until`.
    pub memory_expired: Vec<String>,
    /// Memories whose `verified_at` is older than [`MEMORY_STALE_DAYS`] and
    /// that are not already expired.
    pub memory_unverified: Vec<String>,
}

pub(crate) fn date_of(value: &str) -> Option<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(value.trim().get(..10)?, "%Y-%m-%d").ok()
}

/// The date of the last `Last checked: YYYY-MM-DD` line in `text` (leading
/// list or emphasis marks and a closing mark around the date are allowed).
pub(crate) fn last_checked(text: &str) -> Option<chrono::NaiveDate> {
    text.lines().rev().find_map(|line| {
        let line = line.trim().trim_start_matches(['*', '_', '-', ' ']);
        let head = line.get(..13)?;
        if !head.eq_ignore_ascii_case("last checked:") {
            return None;
        }
        date_of(line[13..].trim().trim_start_matches(['*', '_', '`']))
    })
}

/// A vault file's text, `None` when it is missing, not a plain file or too
/// large. The vault is the person's own folder, so no symlink rule applies.
fn read_plain(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > READ_MAX {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Compares the profile's two live copies by hash (trailing whitespace
/// ignored). `(false, [])` when the clone has no `me/` folder yet.
fn compare_me(vault: &Path, clone: &Path) -> (bool, Vec<String>) {
    let has_me = std::fs::symlink_metadata(clone.join("me")).is_ok_and(|m| m.is_dir());
    if !has_me {
        return (false, Vec::new());
    }
    let mut differs = Vec::new();
    for name in PROFILE_FILES {
        let rel = format!("me/{name}");
        let mine = read_plain(&vault.join(&rel));
        let theirs = match roots::read_in_root(clone, &rel, READ_MAX) {
            Ok(text) => text,
            Err(_) => {
                differs.push(format!("{name} (the clone's copy can't be read)"));
                continue;
            }
        };
        match (mine, theirs) {
            (None, None) => {}
            (Some(_), None) => differs.push(format!("{name} (only in the vault)")),
            (None, Some(_)) => differs.push(format!("{name} (only in the clone)")),
            (Some(a), Some(b)) => {
                let (a, b) = (
                    crate::memory_guard::text_hash(a.trim_end()),
                    crate::memory_guard::text_hash(b.trim_end()),
                );
                if a != b {
                    differs.push(format!("{name} (differs)"));
                }
            }
        }
    }
    (true, differs)
}

/// The context health for the personal clone at `personal` against the
/// vault at `vault`, as of `today`. `slug` names this machine's home file.
/// Nothing is written; a file that can't be read is left out, not an error.
pub(crate) fn health_in(
    vault: &Path,
    personal: &Path,
    slug: Option<&str>,
    today: chrono::NaiveDate,
) -> ContextHealth {
    let mut out = ContextHealth::default();
    if roots::check_root(personal).is_err() {
        return out;
    }
    (out.me_compared, out.me_differs) = compare_me(vault, personal);
    if let Some(rel) = slug.and_then(|s| roots::home_file(s).ok()) {
        if let Ok(Some(raw)) = roots::read_in_root(personal, &rel, READ_MAX) {
            if let Some(date) = last_checked(&raw) {
                out.home_checked = Some(date.format("%Y-%m-%d").to_string());
                out.home_age_days = Some((today - date).num_days());
            }
            out.home_stale = crate::context_stale::home_staleness(&raw, today).map(|s| s.label());
        }
    }
    let memory = personal.join("memory");
    if !std::fs::symlink_metadata(&memory).is_ok_and(|m| m.is_dir()) {
        return out;
    }
    let mut names: Vec<String> = std::fs::read_dir(&memory)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|n| n.ends_with(".md") && n != "README.md")
        .collect();
    names.sort();
    names.truncate(MEMORY_SCAN_MAX);
    for name in names {
        let Ok(Some(text)) = roots::read_in_root(personal, &format!("memory/{name}"), READ_MAX)
        else {
            continue;
        };
        out.memory_files += 1;
        let (verified, until) = crate::memory_guard::memory_dates(&text);
        let verified = verified.as_deref().and_then(date_of);
        let until = until.as_deref().and_then(date_of);
        if verified.is_some() || until.is_some() {
            out.memory_dated += 1;
        }
        let stem = name.trim_end_matches(".md").to_string();
        // One staleness rule for the doctor and recall (ME2).
        match crate::context_stale::memory_staleness(&text, today) {
            Some(crate::context_stale::Staleness::Expired(_)) => out.memory_expired.push(stem),
            Some(_) => out.memory_unverified.push(stem),
            None => {}
        }
    }
    out
}

/// [`health_in`] for this machine now: the configured personal clone, this
/// machine's slug and the local date. `None` while no personal clone is set.
pub(crate) fn health(vault: &Path) -> Option<ContextHealth> {
    let personal = roots::root(roots::Layer::Personal)?;
    Some(health_in(
        vault,
        &personal,
        roots::machine_slug().as_deref(),
        chrono::Local::now().date_naive(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A personal clone with memories and learnings written as Hollow writes them.
    fn index_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("personal");
        std::fs::create_dir_all(root.join("memory")).unwrap();
        std::fs::create_dir_all(root.join("learnings")).unwrap();
        (dir, root)
    }

    fn memory(root: &Path, name: &str, kind: &str, description: &str) {
        std::fs::write(
            root.join("memory").join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: \"{description}\"\nmetadata:\n  node_type: memory\n  type: {kind}\n---\n\nBody text.\n"),
        )
        .unwrap();
    }

    /// ME2: a stale memory is labelled and ranked after every fresh file,
    /// never dropped; a stale home file says so in its lead; both are
    /// collected for the recheck card.
    #[test]
    fn recall_labels_a_stale_memory_and_ranks_it_last() {
        use crate::context_stale::{StaleFile, Staleness, RECHECK_GROUP};
        let (_dir, root) = index_fixture();
        let today = chrono::NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        memory(&root, "fresh", "feedback", "Still true");
        let dated = |name: &str, dates: &str| {
            std::fs::write(
                root.join("memory").join(format!("{name}.md")),
                format!("---\nname: {name}\ndescription: About {name}\nmetadata:\n  type: feedback\n{dates}---\nBody\n"),
            )
            .unwrap();
        };
        dated("aaa-expired", "  valid_until: 2026-10-01\n");
        dated("bbb-current", "  verified_at: 2026-11-01\n");
        std::fs::write(
            root.join("learnings/lesson.md"),
            "---\ntitle: A lesson\n---\nBody\n",
        )
        .unwrap();
        let mut stale = Vec::new();
        let text = personal_index_on(&root, today, &mut stale).unwrap().text;
        let at = |needle: &str| {
            text.find(needle)
                .unwrap_or_else(|| panic!("{needle} in {text}"))
        };
        assert!(
            text.contains("- aaa-expired: About aaa-expired (stale: past valid_until 2026-10-01)")
        );
        assert!(
            text.contains("- bbb-current: About bbb-current\n"),
            "{text}"
        );
        // Alphabetically first, yet after every fresh file and the learnings.
        assert!(at("aaa-expired") > at("bbb-current"));
        assert!(at("aaa-expired") > at("- fresh"));
        assert!(at("aaa-expired") > at("- lesson"));
        assert!(at(&format!("### {RECHECK_GROUP}")) < at("aaa-expired"));
        assert_eq!(
            stale,
            vec![StaleFile {
                rel: "memory/aaa-expired.md".into(),
                staleness: Staleness::Expired(
                    chrono::NaiveDate::from_ymd_opt(2026, 10, 1).unwrap()
                ),
            }]
        );

        std::fs::create_dir_all(root.join("home")).unwrap();
        std::fs::write(
            root.join("home/mac.md"),
            "# Mac\n\nLast checked: 2026-09-01\n",
        )
        .unwrap();
        let mut stale = Vec::new();
        let home = home_section_on(&root, "mac", today, &mut stale).unwrap();
        assert!(
            home.text
                .contains("This file is stale: last checked 2026-09-01"),
            "{}",
            home.text
        );
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].rel, "home/mac.md");
        std::fs::write(
            root.join("home/mac.md"),
            "# Mac\n\nLast checked: 2026-11-10\n",
        )
        .unwrap();
        let mut stale = Vec::new();
        let home = home_section_on(&root, "mac", today, &mut stale).unwrap();
        assert!(!home.text.contains("stale"), "{}", home.text);
        assert!(stale.is_empty());
    }

    #[test]
    fn the_index_lists_every_file_by_type_and_ends_without_a_cut() {
        let (_dir, root) = index_fixture();
        memory(
            &root,
            "short-emails",
            "feedback",
            "Keep drafted emails very short",
        );
        memory(&root, "no-obsidian", "feedback", "Do not use Obsidian");
        memory(
            &root,
            "runner-hostner",
            "reference",
            "The Mac mini runs gates",
        );
        memory(&root, "who", "user", "Alex runs Hollow");
        std::fs::write(root.join("memory/README.md"), "# not a memory").unwrap();
        std::fs::write(root.join("memory/bare.md"), "no frontmatter at all\n").unwrap();
        std::fs::write(
            root.join("learnings/provider-agnostic-29.md"),
            "---\ntype: learned-context\ntitle: Development must not depend on one provider\ntrust: approved\n---\nbody\n",
        )
        .unwrap();
        let section = personal_index(&root).unwrap();
        assert_eq!(section.path, INDEX_PATH);
        assert_eq!(section.text, section.whole, "under the cap nothing is cut");
        assert!(section.text.len() <= INDEX_CAP);
        let body = section.text.split("\n\n").nth(1).unwrap();
        assert_eq!(
            body,
            "### Feedback\n- no-obsidian: Do not use Obsidian\n- short-emails: Keep drafted emails very short\n### Reference\n- runner-hostner: The Mac mini runs gates\n### About you\n- who: Alex runs Hollow\n### Other\n- bare\n### Learnings\n- provider-agnostic-29: Development must not depend on one provider"
        );
        assert!(!section.text.contains("README"));
        assert!(section.text.contains(&root.display().to_string()));
    }

    #[test]
    fn a_new_memory_appears_on_the_next_read_and_nothing_is_written() {
        let (_dir, root) = index_fixture();
        memory(&root, "first", "feedback", "One");
        assert!(!personal_index(&root).unwrap().text.contains("second"));
        memory(&root, "second", "feedback", "Two");
        assert!(personal_index(&root)
            .unwrap()
            .text
            .contains("- second: Two"));
        assert!(!root.join("INDEX.md").exists(), "generated, never written");
        assert!(!root.join("memory/INDEX.md").exists());
    }

    #[test]
    fn past_the_cap_the_index_says_how_many_files_are_left_out() {
        let (_dir, root) = index_fixture();
        for i in 0..60 {
            memory(
                &root,
                &format!("memory-number-{i:02}"),
                "feedback",
                "A rule with a description long enough that sixty of them cannot fit in two kilobytes",
            );
        }
        let section = personal_index(&root).unwrap();
        assert!(section.text.len() <= INDEX_CAP, "{}", section.text.len());
        let last = section.text.lines().last().unwrap();
        assert!(
            last.starts_with("…(") && last.contains("more file(s) not listed"),
            "{last}"
        );
        let left: usize = last
            .trim_start_matches("…(")
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        let listed = section.text.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(listed + left, 60, "every file is either listed or counted");
        assert!(
            section.whole.len() > INDEX_CAP,
            "the whole text keeps every line"
        );
        assert_eq!(
            section
                .whole
                .lines()
                .filter(|l| l.starts_with("- "))
                .count(),
            60
        );
        // Cut at a line end: no fragment of a line.
        assert!(
            section.text.lines().all(|l| l.is_empty()
                || l.starts_with("- ")
                || l.starts_with("### ")
                || l.starts_with("…(")
                || l.starts_with("Your saved")),
            "{}",
            section.text
        );
    }

    #[test]
    fn an_empty_or_unsafe_clone_has_no_index() {
        let (_dir, root) = index_fixture();
        assert!(personal_index(&root).is_none(), "no files");
        assert!(personal_index(&root.join("missing")).is_none());
        memory(&root, "real", "feedback", "Real");
        #[cfg(unix)]
        {
            let link = root.parent().unwrap().join("link");
            std::os::unix::fs::symlink(&root, &link).unwrap();
            assert!(
                personal_index(&link).is_none(),
                "a symlinked root is refused"
            );
            // A symlinked memory file is not followed.
            std::os::unix::fs::symlink(root.join("memory/real.md"), root.join("memory/alias.md"))
                .unwrap();
            assert!(!personal_index(&root).unwrap().text.contains("alias"));
        }
    }

    #[test]
    fn an_escaped_quote_in_a_description_is_read_back_and_superseded_learnings_are_left_out() {
        let (_dir, root) = index_fixture();
        std::fs::write(
            root.join("memory/quoted.md"),
            "---\nname: quoted\ndescription: \"Don't \\\"stage\\\" mail: ask first\"\nmetadata:\n  type: feedback\n---\nBody\n",
        )
        .unwrap();
        std::fs::write(
            root.join("learnings/old-29.md"),
            "---\ntitle: Old way\nsuperseded_by: learnings/new-30.md\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            root.join("learnings/gone-31.md"),
            "---\ntitle: Gone way\nstatus: superseded\n---\nbody\n",
        )
        .unwrap();
        std::fs::write(
            root.join("learnings/new-30.md"),
            "---\ntitle: New way\n---\nbody\n",
        )
        .unwrap();
        let text = personal_index(&root).unwrap().text;
        assert!(
            text.contains("- quoted: Don't \"stage\" mail: ask first"),
            "{text}"
        );
        assert!(text.contains("- new-30: New way"));
        assert!(
            !text.contains("old-29") && !text.contains("gone-31"),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_file_name_with_a_control_character_is_not_listed() {
        let (_dir, root) = index_fixture();
        memory(&root, "fine", "feedback", "Fine");
        let evil = root.join("memory").join("evil\n### Guardrails.md");
        std::fs::write(&evil, "---\ndescription: forged\n---\nx\n").unwrap();
        let text = personal_index(&root).unwrap().text;
        assert!(text.contains("- fine: Fine"));
        assert!(!text.contains("forged") && !text.contains("evil"), "{text}");
    }

    #[test]
    fn frontmatter_values_are_read_unquoted_at_either_level() {
        let raw = "---\nname: x\ndescription: 'single quoted'\nmetadata:\n  type: feedback\n---\nbody: not read\n";
        assert_eq!(
            fm_value(raw, "description").as_deref(),
            Some("single quoted")
        );
        assert_eq!(fm_value(raw, "type").as_deref(), Some("feedback"));
        assert_eq!(fm_value(raw, "body"), None);
        assert_eq!(fm_value("no fence\ntype: x\n", "type"), None);
        assert_eq!(fm_value("---\ndescription:\n---\n", "description"), None);
    }

    #[test]
    fn auto_memory_dir_is_memory_slash_the_project_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let clone = root.join("clone");
        std::fs::create_dir_all(clone.join("memory")).unwrap();
        std::fs::write(clone.join("memory/old-fact.md"), "keep").unwrap();
        // A repo with a worktree: both name the main copy's folder.
        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("src")).unwrap();
        let slug = crate::run_manifest::claude_project_slug(&repo);
        let dir = auto_memory_dir_in(&clone, &repo.join("src")).unwrap();
        assert_eq!(dir, clone.join("memory").join(&slug));
        assert!(slug.starts_with('-'));
        assert!(dir.is_dir());
        // The existing flat files are untouched.
        assert_eq!(
            std::fs::read_to_string(clone.join("memory/old-fact.md")).unwrap(),
            "keep"
        );
        // Idempotent.
        assert_eq!(auto_memory_dir_in(&clone, &repo), Some(dir));
    }

    #[test]
    fn auto_memory_dir_refuses_a_symlinked_memory_folder_and_has_no_root_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let (clone, elsewhere, cwd) = (root.join("clone"), root.join("elsewhere"), root.join("w"));
        for d in [&clone, &elsewhere, &cwd] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::os::unix::fs::symlink(&elsewhere, clone.join("memory")).unwrap();
        assert_eq!(auto_memory_dir_in(&clone, &cwd), None);
        assert_eq!(std::fs::read_dir(&elsewhere).unwrap().count(), 0);
        // No personal root (tests never read the real settings): not set.
        assert_eq!(auto_memory_dir(&cwd), None);
        // A scratch working folder gets the default folder, nothing created.
        let scratch = vec![cwd.clone()];
        assert!(is_scratch_cwd(&cwd.join("sub"), &scratch));
        assert!(!is_scratch_cwd(&root, &scratch));
        let plain = root.join("clone2");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(auto_memory_dir_with(&plain, &cwd, &scratch), None);
        assert!(!plain.join("memory").exists());
    }

    #[test]
    fn memory_folder_names_are_the_cli_slug_not_the_machine_slug() {
        assert_eq!(
            roots::memory_dir("-Users-a-b").unwrap(),
            "memory/-Users-a-b"
        );
        assert_eq!(
            roots::memory_dir("-Users-A-b9").unwrap(),
            "memory/-Users-A-b9"
        );
        for bad in ["", "..", "a/b", "a b", "-x/../y", "é", "a.b"] {
            assert!(roots::memory_dir(bad).is_err(), "{bad}");
        }
        assert!(roots::memory_dir(&"a".repeat(256)).is_err());
    }

    #[test]
    fn cut_keeps_whole_lines_and_the_cap() {
        let lines: Vec<String> = (0..200).map(|i| format!("line {i}")).collect();
        let text = lines.join("\n");
        let out = cut(&text, 300);
        assert!(out.len() <= 300, "{}", out.len());
        assert!(out.ends_with(CUT_MARK));
        assert!(out.starts_with("line 0\nline 1\n"));
        let before = out.strip_suffix(CUT_MARK).unwrap().trim_end();
        assert!(before.lines().all(|l| l.starts_with("line ")));
        assert_eq!(cut("short\n", 300), "short");
        // Multi-byte text never splits a character.
        let wide = "é".repeat(400);
        assert!(cut(&wide, 100).len() <= 100);
    }

    #[test]
    fn home_and_org_sections_read_their_clones_within_caps() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let personal = base.join("personal");
        std::fs::create_dir_all(personal.join("home")).unwrap();
        std::fs::write(
            personal.join("home/macbook-pro.md"),
            format!("---\ntitle: x\n---\n# MacBook\n{}", "fact\n".repeat(2_000)),
        )
        .unwrap();
        let home = home_section(&personal, "macbook-pro").unwrap();
        let (label, text) = (home.path, home.text);
        assert_eq!(label, "home/macbook-pro.md");
        assert!(home.whole.len() > HOME_CAP && home.whole.ends_with("fact"));
        assert!(text.len() <= HOME_CAP, "{}", text.len());
        assert!(!text.contains("title: x"));
        assert!(text.contains("# MacBook"));
        assert!(home_section(&personal, "mac-mini").is_none());
        assert!(home_section(&personal, "../x").is_none());

        let org = base.join("org");
        std::fs::create_dir_all(org.join("glossary")).unwrap();
        std::fs::create_dir_all(org.join(".github")).unwrap();
        std::fs::write(org.join("README.md"), "r").unwrap();
        std::fs::write(org.join("notes.txt"), "n").unwrap();
        let section = org_section(&org).unwrap();
        let (label, text) = (section.path, section.text);
        assert_eq!(label, "org/INDEX.md");
        assert!(text.contains("README.md, glossary/"), "{text}");
        assert!(!text.contains(".github") && !text.contains("notes.txt"));
        std::fs::write(org.join("INDEX.md"), "x\n".repeat(5_000)).unwrap();
        let text = org_section(&org).unwrap().text;
        assert!(text.len() <= ORG_CAP, "{}", text.len());
        assert!(text.contains("never write in it"));
    }

    #[test]
    fn a_long_listing_stays_within_its_room() {
        let dir = tempfile::tempdir().unwrap();
        let org = dir.path().canonicalize().unwrap().join("org");
        for i in 0..40 {
            std::fs::create_dir_all(org.join(format!("a-long-folder-name-{i:02}"))).unwrap();
        }
        let out = listing(&org, 300).unwrap();
        assert!(out.len() <= 300, "{}", out.len());
        assert!(out.ends_with("a-long-folder-name-00/") || out.contains("-01/"));
        let section = org_section(&org).unwrap();
        assert!(section.text.len() <= ORG_CAP);
        assert!(section.text.contains("a-long-folder-name-00/"));
    }

    #[test]
    fn the_org_write_rule_names_the_clone_absolutely() {
        let rules = |p: &str| write_rules_for(Path::new(p), None);
        assert_eq!(rules("/x/my org"), None);
        assert_eq!(
            write_rules_for(
                Path::new("/Users/a/Hollow/Context/org"),
                Some(Path::new("/Users/a"))
            ),
            Some(
                [
                    "Edit(//Users/a/Hollow/Context/org/**)",
                    "Bash(*/Users/a/Hollow/Context/org*)",
                    "Bash(*~/Hollow/Context/org*)",
                    "Bash(*$HOME/Hollow/Context/org*)",
                ]
                .map(String::from)
                .to_vec()
            )
        );
        // A root outside the home folder gets no home-relative spellings.
        assert_eq!(
            write_rules_for(Path::new("/x/org/"), Some(Path::new("/Users/a"))),
            Some(vec![
                "Edit(//x/org/**)".to_string(),
                "Bash(*/x/org*)".to_string()
            ])
        );
        // No rule carries a space or a comma (the CLI splits its list there).
        for rule in rules("/Users/a/Hollow/Context/org").unwrap() {
            assert!(!rule.contains([' ', ',']), "{rule}");
        }
        assert_eq!(rules("/x/o(rg)"), None);
        assert_eq!(rules("/x/o*rg"), None);
        assert_eq!(rules("/x/o,rg"), None);
        assert_eq!(rules("relative/org"), None);
        assert_eq!(rules("/"), None);
        assert!(org_write_rules().is_empty(), "tests read no saved root");
        set_test_org_root(Some(PathBuf::from("/x/org")));
        assert_eq!(
            org_write_rules(),
            vec!["Edit(//x/org/**)".to_string(), "Bash(*/x/org*)".to_string()]
        );
        set_test_org_root(None);
    }

    /// The write block's limits are named, not hidden (CX5b): Bash matches
    /// command text only, Codex full access has no sandbox, and Codex's
    /// workspace sandbox can't carve the clone out of a folder it may write.
    #[test]
    fn the_org_block_names_what_it_cannot_refuse() {
        for phrase in [
            "Bash command that reaches the clone without naming its path",
            "Codex in full access (no sandbox)",
            "outside a checked worktree",
            "Grok and Antigravity",
        ] {
            assert!(ORG_BLOCK_LIMITS.contains(phrase), "{phrase}");
        }
        // The Bash rules name the clone by its absolute, `~/` and `$HOME/`
        // spellings only: a relative path (`Context/org/x` from `~/Hollow`)
        // matches none of them, which is the first limit above.
        let rules = write_rules_for(
            Path::new("/Users/a/Hollow/Context/org"),
            Some(Path::new("/Users/a")),
        )
        .unwrap();
        let bash: Vec<&String> = rules.iter().filter(|r| r.starts_with("Bash(")).collect();
        assert_eq!(bash.len(), 3);
        assert!(bash.iter().all(|r| r.starts_with("Bash(*/")
            || r.starts_with("Bash(*~/")
            || r.starts_with("Bash(*$HOME/")));
    }

    fn org_fixture() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let org = dir.path().canonicalize().unwrap().join("org");
        std::fs::create_dir_all(org.join("glossary")).unwrap();
        std::fs::create_dir_all(org.join("tools")).unwrap();
        std::fs::create_dir_all(org.join(".git")).unwrap();
        std::fs::write(
            org.join("glossary/terms.md"),
            "# Glossary\n\nLPR: licence plate recognition at the gate.\n",
        )
        .unwrap();
        std::fs::write(org.join("INDEX.md"), "# Org\nglossary/ has the terms.\n").unwrap();
        std::fs::write(org.join("tools/scan.md"), "LPR scanner tool notes\n").unwrap();
        std::fs::write(org.join(".git/LPR.md"), "LPR inside git\n").unwrap();
        std::fs::write(org.join("glossary/LPR.csv"), "LPR,1\n").unwrap();
        (dir, org)
    }

    fn terms(query: &str) -> Vec<crate::vault_search::Term> {
        crate::directory_connections::retrieval_terms(query)
    }

    #[test]
    fn org_files_are_a_retrieval_source_citing_the_clone() {
        let (dir, org) = org_fixture();
        let (mut scored, mut warnings) = (Vec::new(), Vec::new());
        let read = org_passages(
            &org,
            &terms("what does LPR mean"),
            &mut scored,
            &mut warnings,
        );
        assert_eq!(read, 2, "INDEX.md and glossary/terms.md only");
        assert!(warnings.is_empty(), "{warnings:?}");
        let hit = scored
            .iter()
            .find(|s| s.passage.path.ends_with("org/glossary/terms.md"))
            .expect("the glossary hit");
        assert_eq!(hit.passage.source, ORG_KIND);
        assert_eq!(hit.passage.line, 3);
        assert!(hit.passage.excerpt.contains("licence plate recognition"));
        assert_eq!(
            Path::new(&hit.passage.path),
            org.join("glossary/terms.md"),
            "cites the clone path"
        );
        assert!(!hit.vault);
        for s in &scored {
            let p = &s.passage.path;
            assert!(!p.contains("/tools/") && !p.contains("/.git/") && !p.ends_with(".csv"));
        }
        // A symlinked file or folder in the clone is never followed.
        std::fs::write(dir.path().join("secret.md"), "LPR secret\n").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.md"), org.join("link.md")).unwrap();
        std::os::unix::fs::symlink(dir.path(), org.join("linked-dir")).unwrap();
        let mut scored = Vec::new();
        org_passages(&org, &terms("LPR"), &mut scored, &mut warnings);
        assert!(scored.iter().all(|s| !s.passage.excerpt.contains("secret")));
        // An oversized file is skipped, never cut and read.
        std::fs::write(
            org.join("big.md"),
            format!("LPR {}", "x".repeat(READ_MAX as usize)),
        )
        .unwrap();
        let (mut scored, mut warnings) = (Vec::new(), Vec::new());
        org_passages(&org, &terms("LPR"), &mut scored, &mut warnings);
        assert!(scored.iter().all(|s| !s.passage.path.ends_with("big.md")));
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(
            warnings[0].contains("1 over 256 KB not read"),
            "{warnings:?}"
        );
        // A root that fails its check reads nothing.
        assert!(org_files(Path::new("/no/such/org")).0.is_empty());
    }

    #[test]
    fn the_org_read_budget_names_files_first_and_warns() {
        let dir = tempfile::tempdir().unwrap();
        let org = dir.path().canonicalize().unwrap().join("org");
        std::fs::create_dir_all(&org).unwrap();
        for i in 0..(ORG_FILES + 5) {
            std::fs::write(org.join(format!("a{i:04}.md")), "plain line\n").unwrap();
        }
        std::fs::write(org.join("zz-cobalt.md"), "cobalt facts\n").unwrap();
        let (mut scored, mut warnings) = (Vec::new(), Vec::new());
        let read = org_passages(&org, &terms("cobalt"), &mut scored, &mut warnings);
        assert_eq!(read, ORG_FILES);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(scored
            .iter()
            .any(|s| s.passage.path.ends_with("zz-cobalt.md")));
    }

    /// Names retrieval keeps out of connected projects stay out of the org
    /// source too: a likely secret file or folder, a `restricted` folder.
    #[test]
    fn org_retrieval_skips_secret_and_restricted_names() {
        let (_dir, org) = org_fixture();
        std::fs::create_dir_all(org.join("restricted")).unwrap();
        std::fs::create_dir_all(org.join("secrets")).unwrap();
        for rel in [
            "credentials.md",
            "glossary/api-password.md",
            "glossary/secret-notes.txt",
            "restricted/LPR.md",
            "secrets/LPR.md",
            "token.md",
        ] {
            std::fs::write(org.join(rel), "LPR hidden value\n").unwrap();
        }
        let listed: Vec<String> = org_files(&org).0.into_iter().map(|f| f.rel).collect();
        assert_eq!(listed, ["INDEX.md", "glossary/terms.md"], "{listed:?}");
        let (mut scored, mut warnings) = (Vec::new(), Vec::new());
        org_passages(&org, &terms("LPR"), &mut scored, &mut warnings);
        assert!(!scored.is_empty());
        assert!(scored
            .iter()
            .all(|s| !s.passage.excerpt.contains("hidden value")));
    }

    /// A slow or hanging pull never holds a turn (CX5b): the timer returns as
    /// soon as its thread starts, and the section and retrieval read the
    /// working tree while a pull holds git's locks.
    #[test]
    fn a_hanging_pull_never_holds_retrieval_or_the_prompt() {
        let (_dir, org) = org_fixture();
        let (hold, held) = std::sync::mpsc::channel::<()>();
        let (started_tx, started) = std::sync::mpsc::channel::<()>();
        let held = std::sync::Mutex::new(held);
        let t = std::time::Instant::now();
        spawn_pull_loop(
            std::time::Duration::ZERO,
            std::time::Duration::from_secs(3600),
            move || {
                let _ = started_tx.send(());
                // Hangs until the test ends: a pull that never returns.
                let _ = held.lock().unwrap().recv();
            },
        )
        .unwrap();
        started
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        // What a running pull holds: git's own lock files.
        std::fs::write(org.join(".git/index.lock"), "").unwrap();
        std::fs::write(org.join(".git/HEAD.lock"), "").unwrap();
        let section = org_section(&org).unwrap();
        assert!(section.text.contains("glossary/ has the terms"));
        let (mut scored, mut warnings) = (Vec::new(), Vec::new());
        org_passages(&org, &terms("LPR"), &mut scored, &mut warnings);
        assert!(!scored.is_empty());
        assert!(
            t.elapsed() < std::time::Duration::from_secs(5),
            "{:?}",
            t.elapsed()
        );
        drop(hold);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_home_file_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let personal = base.join("personal");
        std::fs::create_dir_all(personal.join("home")).unwrap();
        std::fs::write(base.join("secret.md"), "secret").unwrap();
        let link = personal.join("home/mac.md");
        std::os::unix::fs::symlink(base.join("secret.md"), link).unwrap();
        assert!(home_section(&personal, "mac").is_none());
    }

    fn day(s: &str) -> chrono::NaiveDate {
        chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    /// A vault and a personal clone, each with the five profile files, the
    /// same text in both.
    fn me_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let vault = base.join("vault");
        let personal = base.join("personal");
        for side in [&vault, &personal] {
            std::fs::create_dir_all(side.join("me")).unwrap();
            for name in PROFILE_FILES {
                std::fs::write(side.join("me").join(name), format!("# {name}\nsame\n")).unwrap();
            }
        }
        (dir, vault, personal)
    }

    #[test]
    fn matching_profile_copies_raise_nothing() {
        let (_dir, vault, personal) = me_fixture();
        // A README that differs on purpose, and trailing whitespace, are not drift.
        std::fs::write(vault.join("me/README.md"), "vault note").unwrap();
        std::fs::write(personal.join("me/README.md"), "clone note").unwrap();
        std::fs::write(personal.join("me/Identity.md"), "# Identity.md\nsame\n\n\n").unwrap();
        let health = health_in(&vault, &personal, None, day("2026-11-16"));
        assert!(health.me_compared);
        assert!(health.me_differs.is_empty(), "{:?}", health.me_differs);
    }

    #[test]
    fn a_planted_edit_in_one_copy_is_named() {
        let (_dir, vault, personal) = me_fixture();
        std::fs::write(
            personal.join("me/Guardrails.md"),
            "# Guardrails.md\nedited\n",
        )
        .unwrap();
        std::fs::remove_file(vault.join("me/Decisions.md")).unwrap();
        std::fs::remove_file(personal.join("me/Identity.md")).unwrap();
        let health = health_in(&vault, &personal, None, day("2026-11-16"));
        assert!(health.me_compared);
        assert_eq!(
            health.me_differs,
            vec![
                "Identity.md (only in the vault)".to_string(),
                "Guardrails.md (differs)".to_string(),
                "Decisions.md (only in the clone)".to_string(),
            ]
        );
    }

    #[test]
    fn a_clone_without_me_is_not_compared() {
        let (_dir, vault, personal) = me_fixture();
        std::fs::remove_dir_all(personal.join("me")).unwrap();
        let health = health_in(&vault, &personal, None, day("2026-11-16"));
        assert!(!health.me_compared);
        assert!(health.me_differs.is_empty());
    }

    #[test]
    fn last_checked_reads_the_marked_date() {
        assert_eq!(
            last_checked("a\nLast checked: 2026-11-14\n"),
            Some(day("2026-11-14"))
        );
        assert_eq!(
            last_checked("- **Last checked:** 2026-11-14"),
            Some(day("2026-11-14"))
        );
        assert_eq!(last_checked("Last checked: someday"), None);
        assert_eq!(last_checked("no marker"), None);
        // The last marker wins.
        assert_eq!(
            last_checked("Last checked: 2026-01-01\nLast checked: 2026-02-02"),
            Some(day("2026-02-02"))
        );
    }

    #[test]
    fn freshness_counts_the_home_file_and_the_memories() {
        let (_dir, vault, personal) = me_fixture();
        std::fs::create_dir_all(personal.join("home")).unwrap();
        std::fs::create_dir_all(personal.join("memory")).unwrap();
        std::fs::write(
            personal.join("home/mac.md"),
            "Notes\n\nLast checked: 2026-09-01\n",
        )
        .unwrap();
        let memory = |name: &str, extra: &str| {
            std::fs::write(
                personal.join("memory").join(format!("{name}.md")),
                format!("---\nname: {name}\ndescription: d\nmetadata:\n  type: feedback\n{extra}---\nBody\n"),
            )
            .unwrap();
        };
        memory("expired", "  valid_until: 2026-10-01\n");
        memory("old", "  verified_at: 2026-01-01\n");
        memory("fresh", "  verified_at: 2026-11-01\n");
        memory(
            "both",
            "  verified_at: 2026-01-01\n  valid_until: 2026-02-01\n",
        );
        memory("plain", "");
        std::fs::write(personal.join("memory/README.md"), "# not a memory").unwrap();
        let health = health_in(&vault, &personal, Some("mac"), day("2026-11-16"));
        assert_eq!(health.home_checked.as_deref(), Some("2026-09-01"));
        assert_eq!(health.home_age_days, Some(76));
        assert_eq!(health.memory_files, 5);
        assert_eq!(health.memory_dated, 4);
        assert_eq!(health.memory_expired, vec!["both", "expired"]);
        assert_eq!(health.memory_unverified, vec!["old"]);
        assert_eq!(
            health.home_stale.as_deref(),
            Some("stale: last checked 2026-09-01")
        );
        // ME2 review: one home rule for recall and the doctor. A lapsed
        // valid_until is stale though Last checked is recent.
        std::fs::write(
            personal.join("home/mac.md"),
            "---\nvalid_until: 2026-11-01\n---\nNotes\n\nLast checked: 2026-11-15\n",
        )
        .unwrap();
        let health = health_in(&vault, &personal, Some("mac"), day("2026-11-16"));
        assert_eq!(
            health.home_stale.as_deref(),
            Some("stale: past valid_until 2026-11-01")
        );
        assert_eq!(health.home_age_days, Some(1));
    }

    #[test]
    fn health_ignores_a_missing_or_unsafe_clone() {
        let (_dir, vault, personal) = me_fixture();
        let gone = personal.join("nope");
        assert_eq!(
            health_in(&vault, &gone, Some("mac"), day("2026-11-16")),
            ContextHealth::default()
        );
        #[cfg(unix)]
        {
            let link = personal.parent().unwrap().join("link");
            std::os::unix::fs::symlink(&personal, &link).unwrap();
            assert_eq!(
                health_in(&vault, &link, Some("mac"), day("2026-11-16")),
                ContextHealth::default()
            );
        }
    }
}
