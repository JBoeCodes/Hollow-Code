//! The relevant skill index and its reader (agent effectiveness F9, wave 4 ·
//! V2).
//!
//! Before V2 a skill reached an agent only as "read this file": Skills → Use
//! in workspace put a path in the composer, and an accept-edits agent whose
//! working folder was a workspace could not read a `SKILL.md` outside it (the
//! paid pilot's selected-skill task failed 0/3 in both rounds for exactly
//! that). Now every chat turn gets, in its own message (never the
//! appended prompt, so a different index never restarts a native session
//! or rewrites its cached prefix):
//!
//! - **a small index** ([`for_turn`]): up to [`INDEX_MAX`] skills relevant to
//!   the request, one line each (name, where it lives, why, its description
//!   cut to [`DESCRIPTION_CHARS`]). A skill the person selected with Use in
//!   workspace (the composer text `useSkillPrompt` writes, parsed by
//!   [`selections`]) always comes first; the rest are a deterministic local
//!   match of the request's words against each skill's name and description
//!   ([`rank`]): no model, no Jev, and nothing leaves the Mac;
//! - **a selected skill's text inline** when it is small: the selected skills'
//!   `SKILL.md` files, in order, while they fit [`INLINE_BUDGET_BYTES`]
//!   together, so an agent with no owned MCP (Antigravity, Grok) gets the
//!   instructions too;
//! - **an on-demand reader**: `hollow_vault` `read_skill {name}` ([`read`],
//!   served by `vault_mcp`) returns one skill's whole `SKILL.md`, at most
//!   [`READ_MAX_BYTES`]; OC8: a skill whose folder has `references/*.md`
//!   is a kernel plus named references, the answer names them and
//!   `reference` adds one ([`with_reference`], [`app_answer`] for
//!   `skills.read_skill`); a skill with none answers exactly as before.
//!
//! **Roots.** Only two folders are ever read ([`Roots`]): the vault's
//! `.agents/skills` and the shared library `~/.agents/skills`, in that order
//! (a vault skill wins a name clash; `read_skill`'s `source` picks the other).
//! Nothing else: not the provider roots, not plugins, not any other vault
//! file. `.agents` and `.agents/skills` must be real folders (a symlink there
//! drops the root), and a skill's `SKILL.md` must resolve inside its root's
//! real path, so a link can't lead out. A selected skill from a provider root
//! (Skills lists those too) is matched against the registered skill set by
//! `skills::read_skill_in`, the Skills page's own reader, and only inlined.
//!
//! The run manifest records which skills were indexed and why, which went
//! inline and which the agent read (`run_manifest::SkillsRecord`); names and
//! sizes only.

use crate::run_manifest::{SkillIndexed, SkillsRecord};
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

/// At most this many skills in a turn's index, the selected ones included.
pub(crate) const INDEX_MAX: usize = 5;
/// A listed skill's description is cut to this many characters.
pub(crate) const DESCRIPTION_CHARS: usize = 160;
/// The selected skills' text goes inline while it fits this many bytes in
/// total; a larger one is read on demand.
pub(crate) const INLINE_BUDGET_BYTES: usize = 8_192;
/// `read_skill` returns at most this many bytes of a `SKILL.md`.
pub(crate) const READ_MAX_BYTES: usize = 64 * 1024;
/// At most this many skill folders are looked at per root.
const ROOT_MAX_SKILLS: usize = 256;
/// Of each `SKILL.md`, the listing reads only this much (the frontmatter).
pub(crate) const META_READ_BYTES: u64 = 16 * 1024;
/// At most this many selected skills per request.
const MAX_SELECTED: usize = 3;
/// A skill folder under the vault and under the home folder.
pub(crate) const SKILLS_DIR: &str = ".agents/skills";
/// The block's tags.
pub(crate) const OPEN: &str = "<hollow-skills>";
pub(crate) const CLOSE: &str = "</hollow-skills>";
/// The block a resumed turn with no fitting skill sends once, when its
/// session still holds an earlier turn's picks (overnight A15): the latest
/// block applies, so this one says none do.
pub(crate) const NONE_FIT: &str = "<hollow-skills></hollow-skills>\n\n";

/// Where a skill lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    /// The vault's `.agents/skills`.
    Vault,
    /// The shared library, `~/.agents/skills`.
    Shared,
    /// A provider or plugin installation Skills lists (selected only).
    Installed,
}

impl Source {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Source::Vault => "vault",
            Source::Shared => "shared",
            Source::Installed => "installed",
        }
    }
}

/// The two folders skills are read from (see the module doc).
#[derive(Debug, Clone, Default)]
pub(crate) struct Roots {
    /// `{vault}/.agents/skills`.
    vault: Option<PathBuf>,
    /// `~/.agents/skills`.
    shared: Option<PathBuf>,
    /// The home folder, for a selected installed skill.
    home: Option<PathBuf>,
}

impl Roots {
    pub(crate) fn new(vault: Option<&Path>, home: Option<&Path>) -> Self {
        Roots {
            vault: vault.map(|v| v.join(SKILLS_DIR)),
            shared: home.map(|h| h.join(SKILLS_DIR)),
            home: home.map(Path::to_path_buf),
        }
    }

    /// The vault's and the person's home folder's.
    pub(crate) fn for_vault(vault: &Path) -> Self {
        Self::new(Some(vault), dirs::home_dir().as_deref())
    }

    /// The vault's root and a shared root given as its skills folder (the
    /// vault server's `HOLLOW_VAULT_SKILLS`); no installed skills.
    pub(crate) fn with_shared_dir(vault: Option<&Path>, shared: Option<&Path>) -> Self {
        Roots {
            vault: vault.map(|v| v.join(SKILLS_DIR)),
            shared: shared.map(Path::to_path_buf),
            home: None,
        }
    }

    fn dirs(&self) -> impl Iterator<Item = (Source, &Path)> {
        [
            (Source::Vault, self.vault.as_deref()),
            (Source::Shared, self.shared.as_deref()),
        ]
        .into_iter()
        .filter_map(|(source, dir)| dir.map(|d| (source, d)))
    }
}

/// The shared library's folder, `~/.agents/skills`.
pub(crate) fn shared_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(SKILLS_DIR))
}

/// One skill found in a root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    /// Its frontmatter `name`, else its folder's.
    pub name: String,
    pub folder: String,
    /// One line.
    pub description: String,
    pub source: Source,
    /// The `SKILL.md`'s real path.
    pub path: PathBuf,
    pub bytes: u64,
}

/// `dir` (`<base>/.agents/skills`) as a real folder, or `None` when it is
/// missing or `.agents` or `skills` is a symlink.
fn checked_root(dir: &Path) -> Option<PathBuf> {
    let agents = dir.parent()?;
    for path in [agents, dir] {
        let meta = fs::symlink_metadata(path).ok()?;
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return None;
        }
    }
    fs::canonicalize(dir).ok()
}

/// `file`'s real path when it is a `SKILL.md` inside `root` (a real path).
fn contained(root: &Path, file: &Path) -> Option<PathBuf> {
    let real = fs::canonicalize(file).ok()?;
    (real.starts_with(root) && real.file_name().and_then(|n| n.to_str()) == Some("SKILL.md"))
        .then_some(real)
}

fn read_head(path: &Path, max: u64) -> String {
    let mut buf = Vec::new();
    if let Ok(file) = fs::File::open(path) {
        let _ = file.take(max).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf).into_owned()
}

/// `text` on one line, cut to `max` characters with "…".
fn one_line(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut cut: String = flat.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// A frontmatter name fit to show and to look up.
pub(crate) fn fit_name(name: &str) -> bool {
    let name = name.trim();
    !name.is_empty()
        && name.chars().count() <= 64
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '/' | '\\'))
}

/// Every skill in the roots, vault first, each folder in name order.
pub(crate) fn list(roots: &Roots) -> Vec<Entry> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (source, dir) in roots.dirs() {
        let Some(root) = checked_root(dir) else {
            continue;
        };
        let mut folders: Vec<String> = fs::read_dir(&root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().to_str().map(str::to_string))
            .filter(|name| !name.starts_with('.'))
            .collect();
        folders.sort();
        folders.truncate(ROOT_MAX_SKILLS);
        for folder in folders {
            let Some(path) = contained(&root, &root.join(&folder).join("SKILL.md")) else {
                continue;
            };
            let Ok(meta) = fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || !seen.insert(path.clone()) {
                continue;
            }
            let (description, _, name) =
                crate::skills::parse_skill_meta(&read_head(&path, META_READ_BYTES));
            let name = name
                .filter(|n| fit_name(n))
                .map(|n| n.trim().to_string())
                .unwrap_or_else(|| folder.clone());
            out.push(Entry {
                name,
                folder,
                description: one_line(&description, DESCRIPTION_CHARS),
                source,
                path,
                bytes: meta.len(),
            });
        }
    }
    out
}

/// One skill's text, at most [`READ_MAX_BYTES`].
#[derive(Debug, Clone)]
pub(crate) struct SkillText {
    pub entry: Entry,
    pub text: String,
    /// The file was longer than the bound.
    pub cut: bool,
    /// The skill's named references (OC8): the `.md` files in its
    /// `references/` folder, by name without `.md`; empty for none.
    pub references: Vec<String>,
    /// The one reference asked for with the kernel, if any.
    pub reference: Option<Reference>,
}

/// One reference read with a skill's kernel (OC8).
#[derive(Debug, Clone)]
pub(crate) struct Reference {
    pub name: String,
    pub text: String,
    /// The file's size on disk.
    pub bytes: u64,
    /// The file was longer than [`READ_MAX_BYTES`].
    pub cut: bool,
}

/// The skill named `name` in the roots (its frontmatter name or its folder;
/// exact first, then ignoring case), from `source` (`vault` or `shared`)
/// when given. Never anything but a listed `SKILL.md`.
pub(crate) fn read(roots: &Roots, name: &str, source: Option<&str>) -> Result<SkillText, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(
            "read_skill: `name` is required (a skill name from the latest <hollow-skills> block)."
                .into(),
        );
    }
    let wanted = match source.map(str::trim).filter(|s| !s.is_empty()) {
        None => None,
        Some("vault") => Some(Source::Vault),
        Some("shared") => Some(Source::Shared),
        Some(other) => {
            return Err(format!(
                "read_skill: `source` must be vault or shared (got `{other}`)."
            ))
        }
    };
    let entries: Vec<Entry> = list(roots)
        .into_iter()
        .filter(|e| wanted.is_none_or(|s| e.source == s))
        .collect();
    let found = entries
        .iter()
        .find(|e| e.name == name || e.folder == name)
        .or_else(|| {
            entries
                .iter()
                .find(|e| e.name.eq_ignore_ascii_case(name) || e.folder.eq_ignore_ascii_case(name))
        });
    let Some(entry) = found else {
        let mut names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        names.truncate(20);
        let known = if names.is_empty() {
            "there are none".to_string()
        } else {
            format!("skills: {}", names.join(", "))
        };
        return Err(format!(
            "read_skill: no skill named `{}` in the vault's .agents/skills or ~/.agents/skills ({known}). Nothing else is readable here.",
            one_line(name, 64)
        ));
    };
    read_entry(roots, entry)
}

/// The listed skill whose `SKILL.md` is `path`: `hollow_app`'s
/// `skills.read_skill` for a skill in the vault's `.agents/skills`, which
/// the Skills page's reader doesn't list.
pub(crate) fn read_path(roots: &Roots, path: &str) -> Result<SkillText, String> {
    let real = fs::canonicalize(path.trim())
        .map_err(|_| format!("no skill at {}", one_line(path, 200)))?;
    let entries = list(roots);
    let entry = entries
        .iter()
        .find(|e| e.path == real)
        .ok_or_else(|| format!("path not registered as a skill: {}", one_line(path, 200)))?;
    read_entry(roots, entry)
}

/// Read a listed entry again, re-checking that it still resolves inside
/// its root (a component may have been swapped since the listing).
fn read_entry(roots: &Roots, entry: &Entry) -> Result<SkillText, String> {
    let root = roots
        .dirs()
        .find(|(source, _)| *source == entry.source)
        .and_then(|(_, dir)| checked_root(dir))
        .ok_or("read_skill: the skill's folder is no longer readable.")?;
    let real = contained(&root, &entry.path)
        .filter(|real| *real == entry.path)
        .ok_or("read_skill: the skill's file moved out of its folder; nothing was read.")?;
    let mut buf = Vec::new();
    fs::File::open(&real)
        .and_then(|f| f.take(READ_MAX_BYTES as u64 + 1).read_to_end(&mut buf))
        .map_err(|e| format!("read_skill: {e}"))?;
    let cut = buf.len() > READ_MAX_BYTES;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(SkillText {
        entry: entry.clone(),
        text: if cut {
            cut_at(&text, READ_MAX_BYTES)
        } else {
            text
        },
        cut,
        references: reference_names(&real),
        reference: None,
    })
}

// ── References (OC8) ─────────────────────────────────────────────────────

/// A skill's references live in this folder beside its `SKILL.md`.
pub(crate) const REFERENCES_DIR: &str = "references";
/// At most this many references are named per skill.
const REFERENCES_MAX: usize = 32;

/// The references folder of the skill whose real `SKILL.md` is `skill_md`,
/// as a real path, or `None` when it is missing or a symlink.
fn references_dir(skill_md: &Path) -> Option<PathBuf> {
    let dir = skill_md.parent()?.join(REFERENCES_DIR);
    let meta = fs::symlink_metadata(&dir).ok()?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return None;
    }
    fs::canonicalize(dir).ok()
}

/// A reference name: a plain file stem, no path parts.
fn fit_reference(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The named references of the skill whose real `SKILL.md` is `skill_md`:
/// the regular `.md` files (never a link) directly in its `references/`
/// folder, by name without `.md`, sorted, at most [`REFERENCES_MAX`].
pub(crate) fn reference_names(skill_md: &Path) -> Vec<String> {
    let Some(dir) = references_dir(skill_md) else {
        return Vec::new();
    };
    let Ok(read) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let file = e.file_name().to_str()?.to_string();
            let stem = file.strip_suffix(".md")?;
            fit_reference(stem).then(|| stem.to_string())
        })
        .collect();
    names.sort();
    names.truncate(REFERENCES_MAX);
    names
}

/// Read the reference `name` of the skill whose real `SKILL.md` is
/// `skill_md`: only a name [`reference_names`] lists, re-checked to resolve
/// inside the skill's own folder, at most [`READ_MAX_BYTES`].
pub(crate) fn read_reference(
    skill_md: &Path,
    skill: &str,
    name: &str,
) -> Result<Reference, String> {
    let name = name.trim();
    let names = reference_names(skill_md);
    if !names.iter().any(|n| n == name) {
        let known = if names.is_empty() {
            "it has none".to_string()
        } else {
            format!("its references: {}", names.join(", "))
        };
        return Err(format!(
            "read_skill: no reference named `{}` in skill `{skill}` ({known}).",
            one_line(name, 64)
        ));
    }
    let moved = "read_skill: the reference moved out of its skill's folder; nothing was read.";
    let dir = references_dir(skill_md).ok_or(moved)?;
    let real = fs::canonicalize(dir.join(format!("{name}.md"))).map_err(|_| moved)?;
    if real.parent() != Some(dir.as_path()) {
        return Err(moved.into());
    }
    let mut buf = Vec::new();
    let file = fs::File::open(&real).map_err(|e| format!("read_skill: {e}"))?;
    let bytes = file.metadata().map(|m| m.len()).unwrap_or(0);
    file.take(READ_MAX_BYTES as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("read_skill: {e}"))?;
    let cut = buf.len() > READ_MAX_BYTES;
    let text = String::from_utf8_lossy(&buf).into_owned();
    Ok(Reference {
        name: name.to_string(),
        text: if cut {
            cut_at(&text, READ_MAX_BYTES)
        } else {
            text
        },
        bytes,
        cut,
    })
}

/// `skill` with its reference `name` attached, when one is asked for.
pub(crate) fn with_reference(
    mut skill: SkillText,
    name: Option<&str>,
) -> Result<SkillText, String> {
    if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
        skill.reference = Some(read_reference(&skill.entry.path, &skill.entry.name, name)?);
    }
    Ok(skill)
}

/// What follows a kernel: the named references, then the one asked for.
/// Empty for a skill with no references, so its answer is as before.
fn references_tail(
    skill_md: &Path,
    references: &[String],
    reference: Option<&Reference>,
) -> String {
    let mut out = String::new();
    if references.is_empty() {
        return out;
    }
    out.push_str(&format!(
        "\n\n[This skill has references: {}. read_skill with `reference` set to one returns this kernel plus that reference.]",
        references.join(", ")
    ));
    if let Some(r) = reference {
        out.push_str(&format!(
            "\n\nReference \"{}\" ({} bytes):\n\n{}",
            r.name, r.bytes, r.text
        ));
        if r.cut {
            out.push_str(&format!(
                "\n\n[read_skill returned the first {READ_MAX_BYTES} bytes of this reference; the rest is in {}]",
                skill_md
                    .parent()
                    .unwrap_or(skill_md)
                    .join(REFERENCES_DIR)
                    .join(format!("{}.md", r.name))
                    .display()
            ));
        }
    }
    out
}

/// `hollow_app` `skills.read_skill`'s answer: the kernel `text` read from
/// `skill_md` (already checked to be a registered skill), plus its named
/// references and the one asked for. A skill with no references and no
/// `reference` asked answers exactly `text`, as before.
pub(crate) fn app_answer(
    skill_md: &str,
    text: String,
    reference: Option<&str>,
) -> Result<String, String> {
    let Ok(real) = fs::canonicalize(skill_md.trim()) else {
        return Ok(text);
    };
    let skill = real
        .parent()
        .and_then(|d| d.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("skill")
        .to_string();
    let references = reference_names(&real);
    let reference = match reference.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => Some(read_reference(&real, &skill, name)?),
        None => None,
    };
    Ok(format!(
        "{text}{}",
        references_tail(&real, &references, reference.as_ref())
    ))
}

/// `text` cut to at most `max` bytes, on a character boundary.
fn cut_at(text: &str, max: usize) -> String {
    let mut end = max.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// `read_skill`'s answer.
pub(crate) fn render(skill: &SkillText) -> String {
    let e = &skill.entry;
    let mut out = format!(
        "Skill \"{}\" ({}, {} bytes): the person's own instructions for this kind of task. Follow them where they fit the request.\n\n{}",
        e.name,
        e.source.as_str(),
        e.bytes,
        skill.text
    );
    if skill.cut {
        out.push_str(&format!(
            "\n\n[read_skill returned the first {READ_MAX_BYTES} bytes of {}; the rest is in {}]",
            e.bytes,
            e.path.display()
        ));
    }
    out.push_str(&references_tail(
        &e.path,
        &skill.references,
        skill.reference.as_ref(),
    ));
    out
}

// ── Selection ────────────────────────────────────────────────────────────

/// A skill the person chose with Use in workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Selection {
    pub name: String,
    pub path: Option<String>,
}

/// How `useSkillPrompt` (`skillCatalog.ts`) starts and ends.
const SELECT_LEAD: &str = "Use the skill ";
const SELECT_NAMED: &str = " for this task.";
const SELECT_END: &str = "Then follow them for this task:";
const PATH_KEY: &str = "{\"path\": ";
/// The longest selection text looked at.
const SELECT_MAX_CHARS: usize = 4_000;

/// The JSON string literal at the start of `s`, and the bytes it took.
fn json_string(s: &str) -> Option<(String, usize)> {
    if !s.starts_with('"') {
        return None;
    }
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => {
                let value: String = serde_json::from_str(&s[..=i]).ok()?;
                return Some((value, i + 1));
            }
            _ => {}
        }
    }
    None
}

/// The skills a request selected (Use in workspace's text), and the request
/// without that text, which the index then matches against.
pub(crate) fn selections(request: &str) -> (Vec<Selection>, String) {
    let mut found = Vec::new();
    let mut rest = String::new();
    let mut at = 0;
    while let Some(offset) = request[at..].find(SELECT_LEAD) {
        let start = at + offset;
        let after = &request[start + SELECT_LEAD.len()..];
        let parsed =
            json_string(after).filter(|(_, used)| after[*used..].starts_with(SELECT_NAMED));
        let Some((name, used)) = parsed else {
            rest.push_str(&request[at..start + SELECT_LEAD.len()]);
            at = start + SELECT_LEAD.len();
            continue;
        };
        let named_end = start + SELECT_LEAD.len() + used + SELECT_NAMED.len();
        let window_end = request[named_end..]
            .char_indices()
            .nth(SELECT_MAX_CHARS)
            .map_or(request.len(), |(i, _)| named_end + i);
        let end = request[named_end..window_end]
            .find(SELECT_END)
            .map_or(named_end, |i| named_end + i + SELECT_END.len());
        let path = request[named_end..end]
            .find(PATH_KEY)
            .and_then(|i| json_string(&request[named_end + i + PATH_KEY.len()..end]))
            .map(|(p, _)| p);
        if found.len() < MAX_SELECTED && !name.trim().is_empty() {
            found.push(Selection {
                name: name.trim().to_string(),
                path,
            });
        }
        rest.push_str(&request[at..start]);
        at = end;
    }
    rest.push_str(&request[at..]);
    (found, rest)
}

// ── Relevance ────────────────────────────────────────────────────────────

/// Words that say nothing about which skill fits.
const STOP: &[&str] = &[
    "about", "also", "and", "any", "anything", "are", "been", "but", "can", "could", "did", "does",
    "each", "for", "from", "get", "had", "has", "have", "here", "how", "into", "its", "just",
    "let", "like", "make", "may", "might", "more", "most", "must", "need", "not", "only", "other",
    "our", "please", "should", "skill", "skills", "some", "such", "task", "tasks", "than", "that",
    "the", "their", "them", "then", "there", "they", "this", "use", "used", "user", "using",
    "very", "want", "wants", "was", "were", "what", "when", "whenever", "which", "will", "with",
    "would", "you", "your",
];

/// A word's comparable form: lower case, a plural `s` dropped.
fn norm(word: &str) -> String {
    let w = word.to_lowercase();
    if w.len() >= 5 && w.ends_with('s') && !w.ends_with("ss") {
        w[..w.len() - 1].to_string()
    } else {
        w
    }
}

/// The distinct meaningful words of `text` (at least 3 letters, not
/// [`STOP`]), at most 48.
fn words(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_alphanumeric()) {
        if raw.chars().count() < 3 || STOP.contains(&raw.to_lowercase().as_str()) {
            continue;
        }
        let w = norm(raw);
        if !out.contains(&w) {
            out.push(w);
            if out.len() == 48 {
                break;
            }
        }
    }
    out
}

/// Whether `lower` (a lower-cased request) names `name` as a whole token.
fn names(lower: &str, name: &str) -> bool {
    let name = name.to_lowercase();
    if name.chars().count() < 3 {
        return false;
    }
    let token = |c: char| c.is_alphanumeric() || c == '-' || c == '_';
    lower.match_indices(&name).any(|(i, _)| {
        let before = lower[..i].chars().next_back();
        let after = lower[i + name.len()..].chars().next();
        !before.is_some_and(token) && !after.is_some_and(token)
    })
}

/// Why a skill is in the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reason {
    Selected,
    /// The request names it.
    Named,
    /// Its name or description shares the request's words.
    Match,
}

impl Reason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reason::Selected => "selected",
            Reason::Named => "named",
            Reason::Match => "match",
        }
    }
}

/// One indexed skill.
#[derive(Debug, Clone)]
pub(crate) struct Pick {
    pub entry: Entry,
    pub reason: Reason,
    pub score: u32,
}

/// The lowest score a matched skill needs: one word of its name, or three
/// of its description.
const MATCH_MIN: u32 = 3;

/// The skills `text` matches, best first: named (10) plus 3 per request
/// word in the name plus 1 per word only in the description, at least
/// [`MATCH_MIN`]; ties by source (vault first) then name.
pub(crate) fn rank(entries: &[Entry], text: &str) -> Vec<Pick> {
    let lower = text.to_lowercase();
    let wanted = words(text);
    let mut picks: Vec<Pick> = entries
        .iter()
        .filter_map(|entry| {
            let named = names(&lower, &entry.name) || names(&lower, &entry.folder);
            let in_name = words(&format!("{} {}", entry.name, entry.folder));
            let in_description = words(&entry.description);
            let mut score = if named { 10 } else { 0 };
            for word in &wanted {
                if in_name.contains(word) {
                    score += 3;
                } else if in_description.contains(word) {
                    score += 1;
                }
            }
            (named || score >= MATCH_MIN).then(|| Pick {
                entry: entry.clone(),
                reason: if named { Reason::Named } else { Reason::Match },
                score,
            })
        })
        .collect();
    let order = |s: Source| match s {
        Source::Vault => 0,
        Source::Shared => 1,
        Source::Installed => 2,
    };
    picks.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(order(a.entry.source).cmp(&order(b.entry.source)))
            .then(a.entry.name.cmp(&b.entry.name))
    });
    picks
}

// ── A turn's block ───────────────────────────────────────────────────────

/// How the agent reads a skill whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Reader {
    /// `hollow_vault` `read_skill` (Ask your vault is attached).
    VaultTool,
    /// `hollow_app` `run_action skills.read_skill`.
    AppAction,
    /// The agent's own file tools.
    OwnTools,
}

impl Reader {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Reader::VaultTool => "readSkill",
            Reader::AppAction => "appAction",
            Reader::OwnTools => "ownTools",
        }
    }

    /// From what the run attached: the vault server, else the app server.
    pub(crate) fn of(vault: bool, app: bool) -> Self {
        if vault {
            Reader::VaultTool
        } else if app {
            Reader::AppAction
        } else {
            Reader::OwnTools
        }
    }

    fn sentence(self) -> &'static str {
        match self {
            Reader::VaultTool => "hollow_vault read_skill with its name",
            Reader::AppAction => "hollow_app run_action skills.read_skill with its path",
            Reader::OwnTools => "open its path with your own file tools; if your mode refuses a file outside the working folder, ask the person to paste it",
        }
    }
}

/// A chat turn's `<hollow-skills>` block (empty when no skill fits and
/// none was selected) and its manifest record (`None` likewise).
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnSkills {
    pub block: String,
    pub record: Option<SkillsRecord>,
}

/// Keep a skill's text from closing the block early.
fn fenced(text: &str) -> String {
    text.replace("</hollow-skills", "<\\/hollow-skills")
        .replace("</skill", "<\\/skill")
}

/// A selected skill outside the roots that Skills lists (a provider or
/// plugin installation), as an entry, with its text.
fn installed(roots: &Roots, selection: &Selection) -> Option<(Entry, String)> {
    let home = roots.home.as_deref()?;
    let path = selection.path.as_deref()?;
    let text = crate::skills::read_skill_in(home, path).ok()?;
    let real = fs::canonicalize(path).ok()?;
    let (description, _, _) = crate::skills::parse_skill_meta(&text);
    let folder = real
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Some((
        Entry {
            name: selection.name.clone(),
            folder,
            description: one_line(&description, DESCRIPTION_CHARS),
            source: Source::Installed,
            bytes: text.len() as u64,
            path: real,
        },
        text,
    ))
}

/// The index for one chat turn's `request` (the person's own words; see the
/// module doc).
pub(crate) fn for_turn(roots: &Roots, request: &str, reader: Reader) -> TurnSkills {
    let entries = list(roots);
    let (selected, rest) = selections(request);
    let mut picks: Vec<Pick> = Vec::new();
    let mut texts: Vec<Option<String>> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for selection in &selected {
        let by_path = selection.path.as_deref().and_then(|p| {
            let real = fs::canonicalize(p).ok()?;
            entries.iter().find(|e| e.path == real)
        });
        let by_name = || {
            entries
                .iter()
                .find(|e| e.name == selection.name || e.folder == selection.name)
        };
        let (entry, text) = match by_path.or_else(by_name) {
            Some(entry) => (entry.clone(), None),
            None => match installed(roots, selection) {
                Some((entry, text)) => (entry, Some(text)),
                None => {
                    missing.push(selection.name.clone());
                    continue;
                }
            },
        };
        if picks.iter().any(|p| p.entry.path == entry.path) {
            continue;
        }
        picks.push(Pick {
            entry,
            reason: Reason::Selected,
            score: 0,
        });
        texts.push(text);
    }
    let matched: Vec<Pick> = rank(&entries, &rest)
        .into_iter()
        .filter(|p| !picks.iter().any(|s| s.entry.path == p.entry.path))
        .collect();
    let room = INDEX_MAX.saturating_sub(picks.len());
    let left_out = matched.len().saturating_sub(room);
    for pick in matched.into_iter().take(room) {
        picks.push(pick);
        texts.push(None);
    }
    if picks.is_empty() && missing.is_empty() {
        return TurnSkills::default();
    }
    // The selected skills' text, inline while it fits.
    let mut budget = INLINE_BUDGET_BYTES;
    let mut inline: Vec<Option<String>> = Vec::with_capacity(picks.len());
    for (pick, text) in picks.iter().zip(texts) {
        let text = match (pick.reason, text) {
            (Reason::Selected, Some(text)) => Some(text),
            (Reason::Selected, None) => read_entry(roots, &pick.entry)
                .ok()
                .filter(|t| !t.cut)
                .map(|t| t.text),
            _ => None,
        };
        let fits = text.filter(|t| t.len() <= budget);
        if let Some(t) = &fits {
            budget -= t.len();
        }
        inline.push(fits);
    }
    let block = render_block(&picks, &inline, &missing, reader);
    let record = SkillsRecord {
        reader: reader.as_str().to_string(),
        available: u32::try_from(entries.len()).unwrap_or(u32::MAX),
        indexed: picks
            .iter()
            .zip(&inline)
            .map(|(p, text)| SkillIndexed {
                name: p.entry.name.clone(),
                source: p.entry.source.as_str().to_string(),
                reason: p.reason.as_str().to_string(),
                score: p.score,
                bytes: p.entry.bytes,
                inline: text.is_some(),
                path: p.entry.path.to_string_lossy().into_owned(),
            })
            .collect(),
        left_out: u32::try_from(left_out).unwrap_or(u32::MAX),
        selected_missing: u32::try_from(missing.len()).unwrap_or(u32::MAX),
        read: Vec::new(),
    };
    TurnSkills {
        block,
        record: Some(record),
    }
}

fn render_block(
    picks: &[Pick],
    inline: &[Option<String>],
    missing: &[String],
    reader: Reader,
) -> String {
    let mut s = String::from(OPEN);
    s.push('\n');
    s.push_str(if picks.iter().any(|p| p.reason == Reason::Selected) {
        "Skills from the person's library for this request, the one they selected first."
    } else {
        "Skills from the person's library that may fit this request, best first."
    });
    s.push_str(&format!(
        " A skill is the person's own instructions for a kind of task: before you follow one, read it whole ({}). Use one only where it fits.\n",
        reader.sentence()
    ));
    for (pick, text) in picks.iter().zip(inline) {
        let e = &pick.entry;
        let mut tags = vec![e.source.as_str()];
        if pick.reason == Reason::Selected {
            tags.insert(0, "selected");
        }
        let path = if reader != Reader::VaultTool || e.source == Source::Installed {
            format!("; {}", e.path.display())
        } else {
            String::new()
        };
        let description = if e.description.is_empty() {
            "(no description)".to_string()
        } else {
            fenced(&e.description)
        };
        let tail = if text.is_some() {
            " Its SKILL.md is below."
        } else if e.source == Source::Installed {
            " Not in Hollow's skill folders: read it with hollow_app run_action skills.read_skill, or your own file tools."
        } else {
            ""
        };
        s.push_str(&format!(
            "- {} ({}{path}): {description}{tail}\n",
            fenced(&e.name),
            tags.join(", ")
        ));
    }
    for name in missing {
        s.push_str(&format!(
            "- \"{}\" was selected, but no such skill is in the vault's .agents/skills, ~/.agents/skills or the installed skills: say so and ask the person for it.\n",
            fenced(&one_line(name, 64))
        ));
    }
    for (pick, text) in picks.iter().zip(inline) {
        if let Some(text) = text {
            s.push_str(&format!(
                "<skill name=\"{}\" source=\"{}\">\n{}\n</skill>\n",
                fenced(&pick.entry.name),
                pick.entry.source.as_str(),
                fenced(text.trim_end())
            ));
        }
    }
    s.push_str(CLOSE);
    s.push_str("\n\n");
    s
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The bed's selected-skill fixture (`scripts/test-bed-vault.mjs`).
    pub(crate) const KESTREL: &str = "---\nname: kestrel-status-report\ndescription: Format a Kestrel pilot status report the way the steering group reads it.\n---\n\n# Kestrel status report\n\nWhen you write a Kestrel status report:\n\n1. The first line is exactly `KESTREL STATUS · <date>`, with today's date as YYYY-MM-DD.\n2. Then three sections, in this order: `## Done`, `## Next`, `## Risks`.\n3. The last line is exactly: Prepared for the Kestrel steering group.\n";

    pub(crate) fn write_skill(base: &Path, folder: &str, text: &str) -> PathBuf {
        let path = base.join(SKILLS_DIR).join(folder).join("SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, text).unwrap();
        path
    }

    pub(crate) fn meta(name: &str, description: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n\nBody of {name}.\n")
    }

    /// What Use in workspace writes (`skillCatalog.ts` `useSkillPrompt`).
    pub(crate) fn use_skill_prompt(name: &str, path: &str, task: &str) -> String {
        let path = serde_json::to_string(path).unwrap();
        format!("Use the skill {} for this task. First read its instructions: with Hollow’s app tools, run_action skills.read_skill with {{\"path\": {path}}}; without them, open {path} with your own file tools (if that file is outside what you may read, say so and ask me to paste it). Then follow them for this task:\n\n{task}", serde_json::to_string(name).unwrap())
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        vault: PathBuf,
        home: PathBuf,
    }

    fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let vault = dir.path().join("Vault");
        let home = dir.path().join("home");
        fs::create_dir_all(&vault).unwrap();
        fs::create_dir_all(&home).unwrap();
        write_skill(&vault, "kestrel-status-report", KESTREL);
        write_skill(
            &home,
            "pdf",
            &meta("pdf", "Read, fill and merge PDF files."),
        );
        write_skill(
            &home,
            "release-notes",
            &meta(
                "release-notes",
                "Draft release notes from merged pull requests.",
            ),
        );
        Fixture {
            _dir: dir,
            vault,
            home,
        }
    }

    fn roots(f: &Fixture) -> Roots {
        Roots::new(Some(&f.vault), Some(&f.home))
    }

    #[test]
    fn both_roots_are_listed_vault_first_and_nothing_else() {
        let f = fixture();
        // A provider root and a vault note are never skills here.
        let provider = f.home.join(".claude/skills/provider-only/SKILL.md");
        fs::create_dir_all(provider.parent().unwrap()).unwrap();
        fs::write(&provider, meta("provider-only", "x")).unwrap();
        fs::create_dir_all(f.vault.join("notes")).unwrap();
        fs::write(f.vault.join("notes/SKILL.md"), meta("note", "x")).unwrap();
        let listed = list(&roots(&f));
        let names: Vec<(&str, Source)> =
            listed.iter().map(|e| (e.name.as_str(), e.source)).collect();
        assert_eq!(
            names,
            [
                ("kestrel-status-report", Source::Vault),
                ("pdf", Source::Shared),
                ("release-notes", Source::Shared)
            ]
        );
        assert_eq!(
            listed[0].description,
            "Format a Kestrel pilot status report the way the steering group reads it."
        );
    }

    /// Overnight 9 A5: block-scalar descriptions reach the index as their
    /// text, on one line, never as "> …" or "| …".
    #[test]
    fn no_listed_description_starts_with_a_block_indicator() {
        let f = fixture();
        write_skill(
            &f.home,
            "hyperframes",
            "---\nname: hyperframes\ndescription: >\n  Mandatory entry point: read this first\n  for any video.\n---\n\nBody.\n",
        );
        write_skill(
            &f.home,
            "humanizer",
            "---\nname: humanizer\ndescription: |\n  Remove signs of AI writing.\n  Based on a guide.\n---\n\nBody.\n",
        );
        let listed = list(&roots(&f));
        let described = |name: &str| {
            listed
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.description.clone())
                .unwrap()
        };
        assert_eq!(
            described("hyperframes"),
            "Mandatory entry point: read this first for any video."
        );
        assert_eq!(
            described("humanizer"),
            "Remove signs of AI writing. Based on a guide."
        );
        assert!(listed
            .iter()
            .all(|e| !e.description.starts_with('>') && !e.description.starts_with('|')));
    }

    #[test]
    fn read_returns_a_skill_whole_and_refuses_everything_else() {
        let f = fixture();
        let r = roots(&f);
        let skill = read(&r, "kestrel-status-report", None).unwrap();
        assert_eq!(skill.text, KESTREL);
        assert!(render(&skill).contains("KESTREL STATUS · <date>"));
        assert!(read(&r, "PDF", None).is_ok(), "case doesn't matter");
        assert!(read(&r, "pdf", Some("vault")).is_err());
        assert!(read(&r, "pdf", Some("shared")).is_ok());
        assert!(read(&r, "pdf", Some("plugins")).is_err());
        for name in [
            "",
            "../notes",
            "../../home/.claude/skills/provider-only",
            "me/x",
        ] {
            assert!(read(&r, name, None).is_err(), "{name:?}");
        }
        let unknown = read(&r, "nope", None).unwrap_err();
        assert!(unknown.contains("kestrel-status-report"), "{unknown}");
    }

    #[test]
    fn a_vault_skill_is_read_by_its_path_and_nothing_else_is() {
        let f = fixture();
        let r = roots(&f);
        let path = f
            .vault
            .join(SKILLS_DIR)
            .join("kestrel-status-report/SKILL.md");
        assert_eq!(
            read_path(&r, &path.to_string_lossy()).unwrap().text,
            KESTREL
        );
        fs::create_dir_all(f.vault.join("me")).unwrap();
        fs::write(f.vault.join("me/SKILL.md"), "private").unwrap();
        for bad in [
            f.vault.join("me/SKILL.md"),
            f.vault
                .join(SKILLS_DIR)
                .join("kestrel-status-report/../../../me/SKILL.md"),
            f.vault.join("nope/SKILL.md"),
        ] {
            assert!(
                read_path(&r, &bad.to_string_lossy()).is_err(),
                "{}",
                bad.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_link_never_leads_out_of_a_root() {
        use std::os::unix::fs::symlink;
        let f = fixture();
        let outside = f.vault.parent().unwrap().join("outside");
        fs::create_dir_all(outside.join("evil")).unwrap();
        fs::write(outside.join("evil/SKILL.md"), meta("evil", "x")).unwrap();
        fs::write(outside.join("secret.md"), "secret").unwrap();
        let skills = f.vault.join(SKILLS_DIR);
        symlink(outside.join("evil"), skills.join("evil")).unwrap();
        fs::create_dir_all(skills.join("linked")).unwrap();
        symlink(outside.join("secret.md"), skills.join("linked/SKILL.md")).unwrap();
        // A link inside the root is fine.
        symlink(skills.join("kestrel-status-report"), skills.join("alias")).unwrap();
        let r = roots(&f);
        let names: Vec<String> = list(&r).into_iter().map(|e| e.folder).collect();
        assert!(!names.contains(&"evil".to_string()) && !names.contains(&"linked".to_string()));
        assert!(read(&r, "evil", None).is_err());
        assert!(read(&r, "linked", None).is_err());
        assert_eq!(
            names
                .iter()
                .filter(|n| n.starts_with("alias") || n.starts_with("kestrel"))
                .count(),
            1,
            "one physical file once"
        );
        // A symlinked .agents drops the root.
        let other = tempfile::tempdir().unwrap();
        write_skill(other.path(), "x", &meta("x", "y"));
        let vault2 = other.path().join("Vault2");
        fs::create_dir_all(&vault2).unwrap();
        symlink(other.path().join(".agents"), vault2.join(".agents")).unwrap();
        assert!(list(&Roots::new(Some(&vault2), None)).is_empty());
    }

    /// OC8: a skill with references answers its kernel plus their names,
    /// and `reference` adds exactly one; a skill without them is unchanged.
    #[test]
    fn a_skill_with_references_returns_its_kernel_plus_one_reference() {
        let f = fixture();
        let r = roots(&f);
        let before = render(&read(&r, "pdf", None).unwrap());
        let refs = f.home.join(SKILLS_DIR).join("release-notes/references");
        fs::create_dir_all(&refs).unwrap();
        fs::write(refs.join("scripts.md"), "Run scripts/notes.sh.\n").unwrap();
        fs::write(refs.join("style.md"), "Short lines.\n").unwrap();
        fs::write(refs.join("notes.txt"), "not a reference").unwrap();
        fs::create_dir_all(refs.join("nested.md")).unwrap();

        // A skill without references: byte for byte as before.
        let pdf = read(&r, "pdf", None).unwrap();
        assert!(pdf.references.is_empty());
        assert_eq!(render(&pdf), before);
        let none = with_reference(read(&r, "pdf", None).unwrap(), Some("scripts")).unwrap_err();
        assert!(
            none.contains("no reference named") && none.contains("it has none"),
            "{none}"
        );

        // The kernel alone names the references.
        let kernel = read(&r, "release-notes", None).unwrap();
        assert_eq!(kernel.references, ["scripts", "style"]);
        let text = render(&kernel);
        assert!(text.contains("Body of release-notes."), "{text}");
        assert!(text.contains("references: scripts, style."), "{text}");
        assert!(!text.contains("Run scripts/notes.sh."), "{text}");

        // Kernel plus one reference, and only that one.
        let one =
            with_reference(read(&r, "release-notes", None).unwrap(), Some("scripts")).unwrap();
        let text = render(&one);
        assert!(text.contains("Body of release-notes."), "{text}");
        assert!(
            text.contains("Reference \"scripts\" (22 bytes):\n\nRun scripts/notes.sh."),
            "{text}"
        );
        assert!(!text.contains("Short lines."), "{text}");
        // An empty reference is the kernel alone.
        let empty = with_reference(read(&r, "release-notes", None).unwrap(), Some(" ")).unwrap();
        assert!(empty.reference.is_none());

        // Only a listed name: never a path, a hidden file or another file.
        for bad in [
            "notes",
            "nested",
            "../SKILL",
            "scripts.md",
            "../../pdf/SKILL",
            "x/y",
        ] {
            let e =
                with_reference(read(&r, "release-notes", None).unwrap(), Some(bad)).unwrap_err();
            assert!(
                e.contains("no reference named") && e.contains("scripts, style"),
                "{bad}: {e}"
            );
        }

        // The app action's answer: as before without references, the
        // same tail as the vault tool's with them.
        let pdf_path = f.home.join(SKILLS_DIR).join("pdf/SKILL.md");
        let pdf_path = pdf_path.to_string_lossy();
        assert_eq!(app_answer(&pdf_path, "K".into(), None).unwrap(), "K");
        assert!(app_answer(&pdf_path, "K".into(), Some("scripts")).is_err());
        let notes = f.home.join(SKILLS_DIR).join("release-notes/SKILL.md");
        let answer = app_answer(&notes.to_string_lossy(), "K".into(), Some("style")).unwrap();
        assert!(
            answer.starts_with("K\n\n[This skill has references: scripts, style."),
            "{answer}"
        );
        assert!(
            answer.ends_with("Reference \"style\" (13 bytes):\n\nShort lines.\n"),
            "{answer}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_reference_link_never_leads_out_of_its_skill() {
        use std::os::unix::fs::symlink;
        let f = fixture();
        let r = roots(&f);
        let outside = f.home.join("secret.md");
        fs::write(&outside, "secret").unwrap();
        let refs = f.home.join(SKILLS_DIR).join("release-notes/references");
        fs::create_dir_all(&refs).unwrap();
        symlink(&outside, refs.join("leak.md")).unwrap();
        let skill = read(&r, "release-notes", None).unwrap();
        assert!(skill.references.is_empty(), "{:?}", skill.references);
        assert!(with_reference(skill, Some("leak")).is_err());
        // A linked references folder is no folder at all.
        fs::remove_dir_all(&refs).unwrap();
        let elsewhere = f.home.join("elsewhere");
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(elsewhere.join("scripts.md"), "secret").unwrap();
        symlink(&elsewhere, &refs).unwrap();
        assert!(read(&r, "release-notes", None)
            .unwrap()
            .references
            .is_empty());
    }

    #[test]
    fn a_long_skill_is_read_to_the_bound() {
        let f = fixture();
        let body = "x".repeat(READ_MAX_BYTES + 500);
        write_skill(&f.home, "huge", &format!("{}{body}", meta("huge", "big")));
        let skill = read(&roots(&f), "huge", None).unwrap();
        assert!(skill.cut && skill.text.len() == READ_MAX_BYTES);
        assert!(render(&skill).contains("the rest is in"));
    }

    #[test]
    fn use_in_workspace_text_is_parsed_and_left_out_of_the_match() {
        let text = use_skill_prompt(
            "kestrel-status-report",
            "/v/.agents/skills/kestrel-status-report/SKILL.md",
            "Write this week's report.",
        );
        let (found, rest) = selections(&text);
        assert_eq!(
            found,
            [Selection {
                name: "kestrel-status-report".into(),
                path: Some("/v/.agents/skills/kestrel-status-report/SKILL.md".into())
            }]
        );
        assert_eq!(rest, "\n\nWrite this week's report.");
        // Not the template: nothing selected.
        let (none, same) = selections("Use the skill of patience.");
        assert!(none.is_empty());
        assert_eq!(same, "Use the skill of patience.");
    }

    #[test]
    fn relevant_skills_are_ranked_locally_and_capped() {
        let f = fixture();
        for i in 0..8 {
            write_skill(
                &f.home,
                &format!("report-{i}"),
                &meta(&format!("report-{i}"), "Write a report."),
            );
        }
        let r = roots(&f);
        let turn = for_turn(
            &r,
            "Write this week's Kestrel status report.",
            Reader::VaultTool,
        );
        let record = turn.record.unwrap();
        assert_eq!(record.indexed.len(), INDEX_MAX);
        assert_eq!(record.indexed[0].name, "kestrel-status-report");
        assert_eq!(record.indexed[0].reason, "match");
        assert!(record.left_out > 0);
        assert!(turn.block.lines().filter(|l| l.starts_with("- ")).count() == INDEX_MAX);
        // Named outright.
        let named = for_turn(&r, "Use pdf here.", Reader::VaultTool);
        assert_eq!(named.record.unwrap().indexed[0].reason, "named");
        // Nothing fits: no block at all.
        let none = for_turn(&r, "What's the weather?", Reader::VaultTool);
        assert!(none.block.is_empty() && none.record.is_none());
        // Deterministic.
        assert_eq!(
            for_turn(&r, "release notes please", Reader::OwnTools).block,
            for_turn(&r, "release notes please", Reader::OwnTools).block
        );
    }

    #[test]
    fn a_selected_skill_comes_first_with_its_text_inline() {
        let f = fixture();
        let r = roots(&f);
        let path = f
            .vault
            .join(SKILLS_DIR)
            .join("kestrel-status-report/SKILL.md");
        let request = use_skill_prompt(
            "kestrel-status-report",
            &path.to_string_lossy(),
            "Draft the release notes too.",
        );
        let turn = for_turn(&r, &request, Reader::VaultTool);
        let record = turn.record.unwrap();
        assert_eq!(record.indexed[0].reason, "selected");
        assert!(record.indexed[0].inline);
        assert_eq!(record.indexed[1].name, "release-notes");
        assert!(
            !record.indexed[1].inline,
            "only a selected skill goes inline"
        );
        assert!(turn
            .block
            .contains("<skill name=\"kestrel-status-report\" source=\"vault\">"));
        assert!(turn
            .block
            .contains("The first line is exactly `KESTREL STATUS"));
        assert!(turn.block.contains("hollow_vault read_skill"));
        assert!(turn.block.ends_with("</hollow-skills>\n\n"));
        // Without the vault reader each line names its path.
        let own = for_turn(&r, &request, Reader::OwnTools);
        // The reader resolves symlinks; TMPDIR may itself be a short alias
        // so macOS test socket paths fit. Assert the emitted canonical path.
        let expected = path.canonicalize().unwrap();
        assert!(own.block.contains(&expected.to_string_lossy().to_string()));
        // A large selected skill is listed, not inlined.
        write_skill(
            &f.home,
            "big",
            &format!(
                "{}{}",
                meta("big", "Large."),
                "y".repeat(INLINE_BUDGET_BYTES)
            ),
        );
        let big = for_turn(
            &r,
            &use_skill_prompt("big", "/nowhere", "go"),
            Reader::VaultTool,
        );
        let rec = big.record.unwrap();
        assert_eq!(rec.indexed[0].name, "big");
        assert!(!rec.indexed[0].inline);
        assert!(!big.block.contains("yyyy"));
        // A selected skill that isn't anywhere is said so.
        let gone = for_turn(
            &r,
            &use_skill_prompt("gone", "/nowhere", "go"),
            Reader::VaultTool,
        );
        assert_eq!(gone.record.unwrap().selected_missing, 1);
        assert!(gone.block.contains("\"gone\" was selected"));
    }

    #[test]
    fn a_selected_installed_skill_is_inlined_from_the_registered_set() {
        let f = fixture();
        let provider = f.home.join(".claude/skills/house-style/SKILL.md");
        fs::create_dir_all(provider.parent().unwrap()).unwrap();
        fs::write(&provider, meta("house-style", "The house style.")).unwrap();
        let r = roots(&f);
        let turn = for_turn(
            &r,
            &use_skill_prompt("house-style", &provider.to_string_lossy(), "go"),
            Reader::VaultTool,
        );
        let record = turn.record.unwrap();
        assert_eq!(record.indexed[0].source, "installed");
        assert!(record.indexed[0].inline);
        assert!(turn.block.contains("Body of house-style."));
        // An unregistered SKILL.md elsewhere is never read.
        let stray = f.home.join("elsewhere/SKILL.md");
        fs::create_dir_all(stray.parent().unwrap()).unwrap();
        fs::write(&stray, "TOP SECRET").unwrap();
        let refused = for_turn(
            &r,
            &use_skill_prompt("stray", &stray.to_string_lossy(), "go"),
            Reader::VaultTool,
        );
        assert!(!refused.block.contains("TOP SECRET"));
        assert_eq!(refused.record.unwrap().selected_missing, 1);
    }
}
