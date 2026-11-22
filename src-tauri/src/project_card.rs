//! Project architecture cards (PC1, 2026-11-18; idea from reposwarm).
//!
//! A project's card is one Markdown file, `outputs/PROJECT_CARD.md` in its
//! workspace, that says what the project is for, its modules, entry points,
//! how to run it and its risks. An agent writes it by reading the real code;
//! Hollow never calls a model for it. Hollow owns the rest:
//!
//! - **The versioned prompt** ([`PROMPT`]): its first line is
//!   `prompt-version: N` ([`prompt_version`]). Changing what the prompt asks
//!   for means bumping N, which marks every card written under the old
//!   number stale.
//! - **The key**: the card's frontmatter records `commit:` (the repository
//!   commit the code was read at) and `prompt_version:`. Hollow compares
//!   them with the working folder's HEAD, read from the `.git` files without
//!   spawning git ([`head_commit`]), and with [`prompt_version`].
//! - **The section check** ([`check_text`]): every heading in [`REQUIRED`]
//!   must be a level-2 heading with text under it. A missing or empty one
//!   fails the check (status `invalid`), as does a missing or malformed key.
//! - **The stale flag**: a card whose commit or prompt version differs from
//!   now is `stale`; one that passes both is `fresh`. Only a card that isn't
//!   fresh carries the instruction to (re)write it, so a card regenerates
//!   only after a commit or a prompt bump (or when it is missing or fails
//!   the check).
//! - **Getting it read first**: the `<hollow-workspace>` envelope
//!   (`workspaces::context_with`) carries the card as its `projectCard`
//!   value, with a preamble note ([`ENVELOPE_NOTE`]) telling the agent to
//!   read it before the other documents. A workspace gets the value when it
//!   has a connected repository (a build binding) or a card; a durable child
//!   task's slim envelope does not. `workspaces.project_card` (a read action)
//!   runs the same check on demand, so an agent can check a card it just
//!   wrote.
//!
//! **The design facet** (SD1, 2026-11-18; idea from stitch-skills'
//! extract-design-md): a product project with UI source gets a design brief,
//! `outputs/DESIGN.md` by default, that an agent writes from the styling
//! source only (Tailwind config, theme files, CSS variables) under its own
//! versioned [`DESIGN_PROMPT`]: five sections ([`DESIGN_SECTIONS`]) pairing
//! natural names with exact values, at most [`DESIGN_MAX`] bytes in all. The
//! card links it with a `design:` frontmatter key (a path relative to the
//! card's folder). The facet is optional and checked only when linked:
//! the link must resolve to a regular file inside the workspace, the file
//! must fit in 2 KB, have the five sections and its own `commit:` and
//! `prompt_version:` key, and it goes stale on the same commit or prompt
//! bump as the card. A linked brief rides the card's value: one sentence on
//! the status line (so it survives the trim to that line) and, when fresh,
//! its text after the card. A card without the key renders exactly as
//! before. Detection only steers the action: when the code folder has a UI
//! marker file ([`UI_MARKERS`]) and the card links no brief,
//! `workspaces.project_card` returns the design prompt and frontmatter.
//!
//! **The first-connect draft** (BI7, 2026-11-18): `first_connect.rs` writes
//! `outputs/PROJECT_CARD_DRAFT.md` when a build workspace is created from an
//! existing repository. While the card isn't fresh, its status line points
//! at the draft ([`draft_line`]) and the action returns its path as `draft`.
//!
//! **Room for more facets** (later rows, not built). Each adds its own
//! frontmatter key or section; the check ignores keys and sections it
//! doesn't know, so a card with a facet still passes, and a facet that needs
//! checking adds its rule here.

use crate::workspaces::Workspace;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};

/// The card's path inside its workspace folder.
pub(crate) const CARD_FILE: &str = "outputs/PROJECT_CARD.md";
/// The envelope key that carries the card.
pub(crate) const ENVELOPE_KEY: &str = "projectCard";
/// Appended to the `<hollow-workspace>` preamble when the envelope carries
/// [`ENVELOPE_KEY`]. Fixed text: what changes (the status, the card) lives
/// in the value, so a resumed session gets it as a line delta.
pub(crate) const ENVELOPE_NOTE: &str = " projectCard is this project's architecture card with the status Hollow checked (its sections, and whether it matches the repository's commit and the card prompt's version): read it before the other documents. Only when it is not fresh does it say how to rewrite it.";

/// The headings every card must have, in order, each with text under it.
pub(crate) const REQUIRED: [&str; 5] =
    ["Purpose", "Modules", "Entry points", "How to run", "Risks"];

/// The card prompt. Line one is its version (reposwarm's convention): bump
/// it whenever the text below changes what a card should hold.
pub(crate) const PROMPT: &str = "prompt-version: 1
Write this project's architecture card from the code itself: open the files you describe and name real paths. A README alone is not enough. Keep the card under 6 KB so it fits in every turn. Use exactly these level-2 headings, in this order, none empty:
- `## Purpose`: what the project is for and who uses it, in two to four sentences.
- `## Modules`: the main folders and files, one line each: the path and what it owns.
- `## Entry points`: where execution starts (binaries, main functions, servers, command-line tools, scheduled jobs, exported APIs), with paths.
- `## How to run`: the commands to install, build, test and run it as the repository defines them (package scripts, Makefile, Cargo, CI files); say which you ran and which you only read.
- `## Risks`: what is fragile, surprising or easy to break (shared state, secrets, migrations, generated files, missing tests), each with where it is.
Write only what you verified in the code and mark a guess as a guess. Change no file in the repository: the card goes in the workspace's outputs/ folder.";

/// The card frontmatter key that links the design brief (SD1).
pub(crate) const DESIGN_KEY: &str = "design";
/// The link the action suggests: `outputs/DESIGN.md`, beside the card.
pub(crate) const DESIGN_LINK: &str = "DESIGN.md";
/// A design brief's size limit, frontmatter included, in bytes.
pub(crate) const DESIGN_MAX: u64 = 2048;
/// The headings every design brief must have, in order, each with text.
pub(crate) const DESIGN_SECTIONS: [&str; 5] =
    ["Theme", "Colors", "Typography", "Components", "Layout"];

/// The design prompt, versioned like [`PROMPT`] and keyed separately: a
/// bump stales every brief, not every card.
pub(crate) const DESIGN_PROMPT: &str = "prompt-version: 1
Write this project's design brief from its styling source only: the Tailwind config, theme files, CSS custom properties, design tokens and the shared components that use them. Open those files; do not work from screenshots, the README or the running app. The whole file, frontmatter included, must be at most 2 KB (2048 bytes). Pair a natural name with each exact value, for example `Deep forest green #1F3D2B: primary buttons, links`. Use exactly these level-2 headings, in this order, none empty:
- `## Theme`: the mood and density in a sentence or two, whether there is a dark mode, and the source files read.
- `## Colors`: each role (background, surface, text, primary, accent, border, states) with its natural name, exact value and token or class.
- `## Typography`: font families with fallbacks, the sizes and weights by use, and line heights.
- `## Components`: buttons, inputs, cards and other shared pieces: radius, borders, shadows and padding, with values.
- `## Layout`: the spacing scale, breakpoints, max widths and grid rules.
Write only values you found in the source. Change no file in the repository: the brief goes in the workspace's outputs/ folder.";

/// Files in a code folder that mean it has UI source worth a design brief.
/// Checked by existence only (no reads): Tailwind's config, or the usual
/// global stylesheet of a Vite, Next or Tailwind 4 app.
pub(crate) const UI_MARKERS: [&str; 10] = [
    "tailwind.config.ts",
    "tailwind.config.js",
    "tailwind.config.mjs",
    "tailwind.config.cjs",
    "src/index.css",
    "src/styles.css",
    "src/app/globals.css",
    "src/styles/globals.css",
    "app/globals.css",
    "styles/globals.css",
];

/// The card text the envelope carries at most, in bytes (whole lines).
const CARD_CAP: usize = 6_000;
/// A stale card's text the envelope carries beside the prompt, in bytes.
const STALE_CAP: usize = 2_500;
/// Largest card read; a bigger one fails the check.
const READ_MAX: u64 = 64 * 1024;
/// Largest `.git` file read (HEAD, a loose ref, `commondir`, `.git`).
const GIT_FILE_MAX: u64 = 4 * 1024;
const PACKED_REFS_MAX: u64 = 4 * 1024 * 1024;

/// The prompt's version, from its first line.
pub(crate) fn prompt_version() -> u32 {
    version_of(PROMPT).unwrap_or(0)
}

fn version_of(prompt: &str) -> Option<u32> {
    prompt
        .lines()
        .next()?
        .strip_prefix("prompt-version:")?
        .trim()
        .parse()
        .ok()
}

/// The design prompt's version, from its first line.
pub(crate) fn design_version() -> u32 {
    version_of(DESIGN_PROMPT).unwrap_or(0)
}

/// The prompt without its version line.
fn prompt_body() -> &'static str {
    body_of(PROMPT)
}

fn body_of(prompt: &'static str) -> &'static str {
    prompt.split_once('\n').map_or("", |(_, body)| body)
}

/// What a checked document is called in its reasons, and what it needs.
struct Doc {
    noun: &'static str,
    prompt: &'static str,
    required: &'static [&'static str],
}

const CARD_DOC: Doc = Doc {
    noun: "card",
    prompt: "card prompt",
    required: &REQUIRED,
};

const DESIGN_DOC: Doc = Doc {
    noun: "brief",
    prompt: "design prompt",
    required: &DESIGN_SECTIONS,
};

/// What a card is now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Status {
    /// No card yet.
    Missing,
    /// A section or the key is missing or malformed.
    Invalid,
    /// It passes the check but its commit or prompt version is not today's.
    Stale,
    /// It passes the check and matches the commit and prompt version.
    Fresh,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Missing => "missing",
            Status::Invalid => "invalid",
            Status::Stale => "stale",
            Status::Fresh => "fresh",
        }
    }
}

/// A card's key: the commit its code was read at and the prompt version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Key {
    pub commit: Option<String>,
    pub prompt_version: Option<u32>,
}

/// The check's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Check {
    pub status: Status,
    /// What the card's frontmatter records.
    pub recorded: Key,
    /// The working folder's HEAD (none when unreadable or no repository)
    /// and [`prompt_version`].
    pub current: Key,
    /// Required headings that are missing or empty.
    pub missing_sections: Vec<String>,
    /// Why it isn't fresh, one sentence each.
    pub reasons: Vec<String>,
    /// The card's body (after the frontmatter); empty when missing.
    #[serde(skip)]
    pub body: String,
    /// The card's `design:` link as written (SD1); `None` without one.
    #[serde(skip)]
    pub design_link: Option<String>,
    /// The linked design brief's check, filled by [`check_with`].
    #[serde(skip)]
    pub design: Option<Box<DesignCheck>>,
}

/// A linked design brief's check (SD1): the link, where it resolved, its
/// size and the same status, key and section check as a card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DesignCheck {
    /// The card's `design:` value.
    pub link: String,
    /// Where it resolved; `None` when it leaves the workspace.
    pub path: Option<String>,
    /// The file's size in bytes, when it is a regular file.
    pub bytes: Option<u64>,
    #[serde(flatten)]
    pub check: Check,
}

impl Check {
    /// Whether the card should be (re)written.
    pub(crate) fn needs_write(&self) -> bool {
        self.status != Status::Fresh
    }
}

fn is_commit(value: &str) -> bool {
    (7..=64).contains(&value.len()) && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// Whether recorded commit `card` names `head` (a short id is a prefix).
fn same_commit(card: &str, head: &str) -> bool {
    let (card, head) = (card.to_ascii_lowercase(), head.to_ascii_lowercase());
    head.starts_with(&card) || card.starts_with(&head)
}

/// The level-2 headings of `body` with whether each has non-blank text under
/// it (up to the next heading of level 1 or 2), by lowercased name.
fn sections(body: &str) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = Vec::new();
    let mut fence = false;
    for line in body.lines() {
        let trimmed = line.trim();
        let marker = trimmed.starts_with("```") || trimmed.starts_with("~~~");
        // A heading inside a code fence is the fence's text, not a heading.
        if !fence && !marker {
            if let Some(name) = line.strip_prefix("## ") {
                out.push((
                    name.trim().trim_end_matches(':').trim().to_lowercase(),
                    false,
                ));
                continue;
            }
            if line.starts_with("# ") {
                // A level-1 heading ends the section above it.
                out.push((String::new(), false));
                continue;
            }
        }
        if marker {
            fence = !fence;
        }
        if !trimmed.is_empty() {
            if let Some(last) = out.last_mut() {
                last.1 = true;
            }
        }
    }
    out
}

/// A frontmatter value without quotes; `None` when empty.
fn fm_value(fm: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fm.get(key)
        .map(|v| v.trim().trim_matches(['"', '\'']).trim().to_string())
        .filter(|v| !v.is_empty())
}

/// The check of card text `raw` against the current commit (`None` when it
/// can't be read) and prompt version. Pure: the file and HEAD reads are the
/// caller's.
pub(crate) fn check_text(raw: &str, head: Option<&str>, version: u32) -> Check {
    let (fm, body) = crate::frontmatter::parse_map(raw);
    let mut check = check_doc(&fm, body, head, version, &CARD_DOC);
    check.design_link = fm_value(&fm, DESIGN_KEY);
    check
}

/// The check of a parsed document (a card or a design brief) against its
/// required sections and its `commit:` and `prompt_version:` key.
fn check_doc(
    fm: &BTreeMap<String, String>,
    body: String,
    head: Option<&str>,
    version: u32,
    doc: &Doc,
) -> Check {
    let recorded_commit = fm_value(fm, "commit");
    let recorded_version = fm_value(fm, "prompt_version").and_then(|v| v.parse::<u32>().ok());
    let mut reasons = Vec::new();
    let found = sections(&body);
    let missing_sections: Vec<String> = doc
        .required
        .iter()
        .filter(|name| {
            !found
                .iter()
                .any(|(heading, filled)| *filled && heading == &name.to_lowercase())
        })
        .map(|name| name.to_string())
        .collect();
    if !missing_sections.is_empty() {
        reasons.push(format!(
            "Missing or empty sections: {}.",
            missing_sections.join(", ")
        ));
    }
    match recorded_commit.as_deref() {
        None => reasons.push("The frontmatter has no commit.".into()),
        Some(c) if c != "unknown" && !is_commit(c) => reasons.push(format!(
            "The frontmatter's commit {c:?} is not a commit id."
        )),
        _ => {}
    }
    if recorded_version.is_none() {
        reasons.push("The frontmatter has no prompt_version number.".into());
    }
    let invalid = !reasons.is_empty();
    let (noun, prompt) = (doc.noun, doc.prompt);
    if let (Some(written), Some(head)) = (recorded_commit.as_deref(), head) {
        if written == "unknown" || (is_commit(written) && !same_commit(written, head)) {
            reasons.push(format!(
                "The repository is at commit {}; the {noun} was written at {written}.",
                short(head)
            ));
        }
    }
    if let Some(written) = recorded_version {
        if written != version {
            reasons.push(format!(
                "The {prompt} is at version {version}; the {noun} was written for version {written}."
            ));
        }
    }
    let status = if invalid {
        Status::Invalid
    } else if reasons.is_empty() {
        Status::Fresh
    } else {
        Status::Stale
    };
    Check {
        status,
        recorded: Key {
            commit: recorded_commit,
            prompt_version: recorded_version,
        },
        current: Key {
            commit: head.map(str::to_string),
            prompt_version: Some(version),
        },
        missing_sections,
        reasons,
        body: body.trim().to_string(),
        design_link: None,
        design: None,
    }
}

/// The check for a project with no card yet.
fn missing(head: Option<&str>, version: u32) -> Check {
    missing_doc(head, version, &CARD_DOC, "This project has no card yet.")
}

fn missing_doc(head: Option<&str>, version: u32, doc: &Doc, reason: &str) -> Check {
    Check {
        status: Status::Missing,
        recorded: Key::default(),
        current: Key {
            commit: head.map(str::to_string),
            prompt_version: Some(version),
        },
        missing_sections: doc.required.iter().map(|s| s.to_string()).collect(),
        reasons: vec![reason.into()],
        body: String::new(),
        design_link: None,
        design: None,
    }
}

fn short(commit: &str) -> &str {
    &commit[..commit.len().min(12)]
}

/// Where workspace `w`'s code is: its worktree while it exists, else the
/// repository it came from; `None` without a build binding.
pub(crate) fn code_folder(w: &Workspace) -> Option<PathBuf> {
    let build = w.build.as_ref()?;
    Some(PathBuf::from(if build.exists {
        &build.worktree
    } else {
        &build.repo
    }))
}

/// A regular file of at most `max` bytes, never through a symlink.
fn read_small(path: &Path, max: u64) -> Option<String> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max {
        return None;
    }
    fs::read_to_string(path).ok()
}

/// The commit `dir`'s HEAD names, read from its `.git` files (a linked
/// worktree's `gitdir:` file and `commondir` followed; a loose ref, then
/// `packed-refs`). No process is spawned. `None` when anything is unreadable.
pub(crate) fn head_commit(dir: &Path) -> Option<String> {
    let dot = dir.join(".git");
    let meta = fs::symlink_metadata(&dot).ok()?;
    let git_dir = if meta.is_dir() {
        dot
    } else if meta.is_file() {
        let text = read_small(&dot, GIT_FILE_MAX)?;
        dir.join(text.trim().strip_prefix("gitdir:")?.trim())
    } else {
        return None;
    };
    let common = read_small(&git_dir.join("commondir"), GIT_FILE_MAX)
        .map(|c| git_dir.join(c.trim()))
        .unwrap_or_else(|| git_dir.clone());
    let sha = |s: &str| {
        let s = s.trim();
        (matches!(s.len(), 40 | 64) && s.chars().all(|c| c.is_ascii_hexdigit()))
            .then(|| s.to_ascii_lowercase())
    };
    let head = read_small(&git_dir.join("HEAD"), GIT_FILE_MAX)?;
    let Some(reference) = head.trim().strip_prefix("ref:").map(str::trim) else {
        return sha(&head);
    };
    if !reference.starts_with("refs/")
        || reference
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        return None;
    }
    for base in [&git_dir, &common] {
        if let Some(loose) = read_small(&base.join(reference), GIT_FILE_MAX) {
            return sha(&loose);
        }
    }
    let packed = read_small(&common.join("packed-refs"), PACKED_REFS_MAX)?;
    packed.lines().find_map(|line| {
        let (id, name) = line.split_once(' ')?;
        (name.trim() == reference).then(|| sha(id)).flatten()
    })
}

/// The card's absolute path for workspace `id` in vault `root`.
pub(crate) fn card_path(root: &Path, id: &str) -> Result<PathBuf, String> {
    crate::vault::safe_join(root, &format!("workspaces/{id}/{CARD_FILE}"))
}

/// The check of workspace `w`'s card now, with the prompt version `version`.
pub(crate) fn check_with(root: &Path, w: &Workspace, version: u32) -> Result<Check, String> {
    let head = code_folder(w).and_then(|dir| head_commit(&dir));
    let path = card_path(root, &w.id)?;
    let Ok(meta) = fs::symlink_metadata(&path) else {
        return Ok(missing(head.as_deref(), version));
    };
    let unreadable = |why: String| {
        let mut check = missing(head.as_deref(), version);
        check.status = Status::Invalid;
        check.reasons = vec![why];
        check
    };
    if !meta.is_file() {
        return Ok(unreadable(format!(
            "{CARD_FILE} is not a regular file (a link or a folder)."
        )));
    }
    if meta.len() > READ_MAX {
        return Ok(unreadable(format!(
            "{CARD_FILE} is over {} KB; a card fits in 6 KB.",
            READ_MAX / 1024
        )));
    }
    match read_no_follow(&path, READ_MAX) {
        Bounded::Text(raw) => {
            let mut check = check_text(&raw, head.as_deref(), version);
            if let Some(link) = check.design_link.clone() {
                check.design = Some(Box::new(check_design(
                    root,
                    &w.id,
                    &link,
                    head.as_deref(),
                    design_version(),
                )));
            }
            Ok(check)
        }
        Bounded::TooBig(_) => Ok(unreadable(format!(
            "{CARD_FILE} is over {} KB; a card fits in 6 KB.",
            READ_MAX / 1024
        ))),
        Bounded::Refused(why) => Ok(unreadable(format!("{CARD_FILE} {why}"))),
    }
}

/// What [`read_no_follow`] found.
enum Bounded {
    Text(String),
    /// Over the limit: its size, or the bytes read past the limit.
    TooBig(u64),
    /// Why it can't be read, as the end of a sentence.
    Refused(String),
}

/// A regular file's text, opened without following a symlink
/// (`O_NOFOLLOW`; `O_NONBLOCK` so a FIFO swapped in doesn't block), its size
/// taken from the open handle and at most `max + 1` bytes read, so a file
/// swapped after an earlier check is still refused or bounded (SD1 review).
fn read_no_follow(path: &Path, max: u64) -> Bounded {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let not_regular = || Bounded::Refused("is not a regular file (a link or a folder).".into());
    let file = match fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.raw_os_error() == Some(libc::ELOOP) => return not_regular(),
        Err(e) => return Bounded::Refused(format!("can't be read: {e}.")),
    };
    let meta = match file.metadata() {
        Ok(meta) => meta,
        Err(e) => return Bounded::Refused(format!("can't be read: {e}.")),
    };
    if !meta.is_file() {
        return not_regular();
    }
    if meta.len() > max {
        return Bounded::TooBig(meta.len());
    }
    let mut bytes = Vec::new();
    if let Err(e) = file.take(max + 1).read_to_end(&mut bytes) {
        return Bounded::Refused(format!("can't be read: {e}."));
    }
    if bytes.len() as u64 > max {
        return Bounded::TooBig(bytes.len() as u64);
    }
    match String::from_utf8(bytes) {
        Ok(text) => Bounded::Text(text),
        Err(_) => Bounded::Refused("is not UTF-8 text.".into()),
    }
}

/// Whether `path`'s deepest existing folder resolves inside workspace `id`,
/// so a path that doesn't exist yet can't be written through a folder link
/// that leads out (SD1 review).
fn folder_inside(root: &Path, id: &str, path: &Path) -> bool {
    let Ok(workspace) = crate::vault::safe_join(root, &format!("workspaces/{id}")) else {
        return false;
    };
    let mut probe = path.parent();
    while let Some(dir) = probe {
        if fs::symlink_metadata(dir).is_ok() {
            return dir
                .canonicalize()
                .is_ok_and(|real| real.starts_with(&workspace));
        }
        probe = dir.parent();
    }
    false
}

/// Where design link `link` (relative to the card's `outputs/` folder)
/// points in workspace `id`, lexically: the workspace folder joined with
/// the normalized path. Refused when it is absolute or climbs out of the
/// workspace.
fn design_path(root: &Path, id: &str, link: &str) -> Result<PathBuf, String> {
    let outside = || {
        format!(
            "The card's design link {link:?} leaves the workspace; link the brief relative to the card, normally `design: {DESIGN_LINK}`."
        )
    };
    let mut parts: Vec<String> = vec!["outputs".into()];
    for part in Path::new(link).components() {
        match part {
            Component::Normal(name) => parts.push(name.to_str().ok_or_else(outside)?.into()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop().ok_or_else(outside)?;
            }
            Component::RootDir | Component::Prefix(_) => return Err(outside()),
        }
    }
    if parts.is_empty() || link.contains('\\') {
        return Err(outside());
    }
    let workspace = crate::vault::safe_join(root, &format!("workspaces/{id}"))?;
    Ok(workspace.join(parts.join("/")))
}

/// The check of the design brief card link `link` points at in workspace
/// `id`: inside the workspace, a regular file of at most [`DESIGN_MAX`]
/// bytes, with [`DESIGN_SECTIONS`] and a key matching `head` and `version`.
fn check_design(
    root: &Path,
    id: &str,
    link: &str,
    head: Option<&str>,
    version: u32,
) -> DesignCheck {
    let mut out = DesignCheck {
        link: link.to_string(),
        path: None,
        bytes: None,
        check: missing_doc(
            head,
            version,
            &DESIGN_DOC,
            &format!("The card links {link}, which does not exist yet."),
        ),
    };
    let fail = |out: &mut DesignCheck, why: String| {
        out.check.status = Status::Invalid;
        out.check.reasons = vec![why];
    };
    let path = match design_path(root, id, link) {
        Ok(path) => path,
        Err(why) => {
            fail(&mut out, why);
            return out;
        }
    };
    out.path = Some(path.display().to_string());
    if !folder_inside(root, id, &path) {
        fail(
            &mut out,
            format!("{link} resolves outside the workspace through a folder link."),
        );
        return out;
    }
    if fs::symlink_metadata(&path).is_err() {
        return out;
    }
    match read_no_follow(&path, DESIGN_MAX) {
        Bounded::Text(raw) => {
            out.bytes = Some(raw.len() as u64);
            let (fm, body) = crate::frontmatter::parse_map(&raw);
            out.check = check_doc(&fm, body, head, version, &DESIGN_DOC);
        }
        Bounded::TooBig(bytes) => {
            out.bytes = Some(bytes);
            fail(
                &mut out,
                format!(
                    "{link} is over {DESIGN_MAX} bytes; a design brief fits in {DESIGN_MAX} bytes (2 KB), frontmatter included."
                ),
            );
        }
        Bounded::Refused(why) => fail(&mut out, format!("{link} {why}")),
    }
    out
}

/// The first [`UI_MARKERS`] file code folder `code` has, by existence only.
pub(crate) fn ui_marker(code: &Path) -> Option<&'static str> {
    UI_MARKERS
        .into_iter()
        .find(|marker| fs::symlink_metadata(code.join(marker)).is_ok_and(|meta| meta.is_file()))
}

/// How to (re)write the card: the path, the frontmatter to record and the
/// prompt.
fn write_instruction(check: &Check, path: &Path, code: Option<&Path>) -> String {
    let commit = check.current.commit.as_deref().unwrap_or("unknown");
    let folder = code.map_or_else(
        || "the project's code".to_string(),
        |dir| format!("the code in {}", dir.display()),
    );
    // Keep a linked design brief linked across a rewrite (SD1).
    let design = check
        .design_link
        .as_deref()
        .map_or_else(String::new, |link| format!("{DESIGN_KEY}: {link}\n"));
    format!(
        "To write it (when you can write files, and the task needs the project's architecture or the person asks; otherwise use the card as a lead and check its claims against the code): read {folder}, then write {} starting with this frontmatter:\n---\nkind: project-card\ncommit: {commit}\nprompt_version: {}\n{design}---\nThen follow the card prompt (v{}):\n{}\nAfterwards check it with Hollow's workspaces.project_card action.",
        path.display(),
        check.current.prompt_version.unwrap_or(0),
        check.current.prompt_version.unwrap_or(0),
        prompt_body(),
    )
}

/// The envelope value for a check: the status, then the card (fresh) or
/// what is wrong, the instruction and the old card cut short.
pub(crate) fn render(check: &Check, path: &Path, code: Option<&Path>) -> String {
    let design = check.design.as_deref();
    let mut text = format!("Status: {}", check.status.as_str());
    if check.status == Status::Fresh {
        text.push_str(&format!(" ({}).", key_text(&check.recorded)));
        if check.current.commit.is_none() {
            text.push_str(" The repository's commit could not be read, so only the prompt version was compared.");
        }
        text.push_str(&format!(
            " File: {}. Do not rewrite it until the commit or the prompt version changes.",
            path.display()
        ));
        text.push_str(&design.map_or_else(String::new, design_line));
        text.push_str(&format!(
            "\n\n{}",
            crate::context_layers::cut(&check.body, CARD_CAP)
        ));
        text.push_str(&design.map_or_else(String::new, |d| design_text(d, code)));
        return text;
    }
    text.push_str(&format!(
        ". {} Now: {}. File: {}.",
        check.reasons.join(" "),
        key_text(&check.current),
        path.display()
    ));
    text.push_str(&design.map_or_else(String::new, design_line));
    text.push_str(&draft_line(path));
    text.push_str(&format!("\n{}", write_instruction(check, path, code)));
    if !check.body.is_empty() {
        text.push_str(&format!(
            "\n\nThe card as it is ({}):\n{}",
            key_text(&check.recorded),
            crate::context_layers::cut(&check.body, STALE_CAP)
        ));
    }
    text.push_str(&design.map_or_else(String::new, |d| design_text(d, code)));
    text
}

/// The first-connect draft beside the card at `card`, when it is a regular
/// file (BI7). Never followed through a link.
pub(crate) fn draft_beside(card: &Path) -> Option<PathBuf> {
    let draft = card.with_file_name(crate::first_connect::DRAFT_NAME);
    fs::symlink_metadata(&draft)
        .is_ok_and(|m| m.is_file())
        .then_some(draft)
}

/// The status-line sentence pointing a card that isn't fresh at the
/// first-connect draft (BI7); empty without one, so the value is unchanged.
fn draft_line(card: &Path) -> String {
    draft_beside(card).map_or_else(String::new, |d| {
        format!(
            " A first-connect draft Hollow wrote by reading the repository (facts, run hints and what needs attention) is at {}: start from it and check each fact in the code.",
            d.display()
        )
    })
}

fn key_text(k: &Key) -> String {
    format!(
        "commit {}, prompt v{}",
        k.commit.as_deref().map_or("unknown", short),
        k.prompt_version
            .map_or_else(|| "?".to_string(), |v| v.to_string())
    )
}

/// The design brief's sentence on the card value's status line, so it
/// survives [`status_only`].
fn design_line(d: &DesignCheck) -> String {
    let file = d
        .path
        .as_deref()
        .map_or_else(String::new, |p| format!(" File: {p}."));
    if d.check.status == Status::Fresh {
        return format!(
            " Design brief: fresh ({}).{file} Read it before any UI work.",
            key_text(&d.check.recorded)
        );
    }
    format!(
        " Design brief: {}. {}{file}",
        d.check.status.as_str(),
        d.check.reasons.join(" ")
    )
}

/// The frontmatter a design brief starts with, at key `current`.
fn design_frontmatter(current: &Key) -> String {
    format!(
        "---\nkind: design-brief\ncommit: {}\nprompt_version: {}\n---",
        current.commit.as_deref().unwrap_or("unknown"),
        current.prompt_version.unwrap_or(0)
    )
}

/// After the card: a fresh brief's text, or how to (re)write it.
fn design_text(d: &DesignCheck, code: Option<&Path>) -> String {
    if d.check.status == Status::Fresh {
        return format!(
            "\n\nDesign brief:\n{}",
            crate::context_layers::cut(&d.check.body, DESIGN_MAX as usize)
        );
    }
    let Some(path) = d.path.as_deref() else {
        return format!(
            "\n\nFix the card's `{DESIGN_KEY}:` frontmatter to a path inside the workspace, normally `{DESIGN_KEY}: {DESIGN_LINK}`."
        );
    };
    let folder = code.map_or_else(
        || "the project's styling source".to_string(),
        |dir| format!("the styling source in {}", dir.display()),
    );
    format!(
        "\n\nTo write the design brief (when you can write files, and the task touches the UI or the person asks): read {folder}, then write {path} starting with this frontmatter:\n{}\nThen follow the design prompt (v{}):\n{}\nAfterwards check it with Hollow's workspaces.project_card action.",
        design_frontmatter(&d.check.current),
        d.check.current.prompt_version.unwrap_or(0),
        body_of(DESIGN_PROMPT),
    )
}

/// Said after a value cut to its status line.
pub(crate) const CUT_TO_STATUS: &str =
    " The rest was left out to fit this turn's context: read the file.";

/// A `projectCard` value cut to its first line (the status, the reasons and
/// the file) for an envelope over its limit; `None` when it is that already,
/// so the envelope's trim loop moves on to the documents.
pub(crate) fn status_only(value: &str) -> Option<String> {
    let first = value.lines().next().unwrap_or("").trim_end();
    if value.ends_with(CUT_TO_STATUS) || value.trim_end() == first {
        return None;
    }
    Some(format!("{first}{CUT_TO_STATUS}"))
}

/// Workspace `w`'s `projectCard` value for its envelope, or `None` when it
/// has neither a connected repository nor a card (its envelope then stays
/// as it was). Never fails a turn: an unreadable card is reported in the
/// value.
pub(crate) fn envelope_value(root: &Path, w: &Workspace) -> Option<String> {
    let path = card_path(root, &w.id).ok()?;
    let code = code_folder(w);
    if code.is_none() && fs::symlink_metadata(&path).is_err() {
        return None;
    }
    let check = check_with(root, w, prompt_version()).ok()?;
    Some(render(&check, &path, code.as_deref()))
}

/// `workspaces.project_card`: workspace `id`'s card check, with the prompt
/// and the frontmatter to write when it isn't fresh.
pub(crate) fn action(root: &Path, id: &str) -> Result<serde_json::Value, String> {
    if !crate::workspaces::valid_id(id) {
        return Err(format!(
            "INVALID_ARGUMENT: {id:?} is not a workspace id; use one from list_workspaces."
        ));
    }
    if crate::workspaces::is_builtin(id) {
        return Err(format!(
            "INVALID_ARGUMENT: {id} is built in and has no project card; name a project's workspace."
        ));
    }
    let w = crate::workspaces::summary(root, id)
        .map_err(|e| format!("NOT_FOUND: no workspace {id}: {e}"))?;
    let check = check_with(root, &w, prompt_version())?;
    let path = card_path(root, id)?;
    let code = code_folder(&w);
    let mut out = serde_json::json!({
        "workspace": id,
        "path": path.display().to_string(),
        "codeFolder": code.as_ref().map(|c| c.display().to_string()),
        "status": check.status,
        "recorded": check.recorded,
        "current": check.current,
        "missingSections": check.missing_sections,
        "reasons": check.reasons,
        "requiredSections": REQUIRED,
        "rewrite": check.needs_write(),
    });
    // BI7: the first-connect draft, when Hollow wrote one.
    if let Some(draft) = draft_beside(&path) {
        out["draft"] = serde_json::json!(draft.display().to_string());
    }
    // SD1: the linked brief's check, or, for a project with UI source and
    // no link, the brief it should get. Neither: no `design` key.
    let ui = code.as_deref().and_then(ui_marker);
    let design = match (check.design.as_deref(), ui) {
        (Some(d), _) => Some(d.clone()),
        (None, Some(marker)) => Some(DesignCheck {
            link: DESIGN_LINK.into(),
            path: design_path(root, id, DESIGN_LINK)
                .ok()
                .map(|p| p.display().to_string()),
            bytes: None,
            check: missing_doc(
                check.current.commit.as_deref(),
                design_version(),
                &DESIGN_DOC,
                &format!(
                    "This project has UI source ({marker}) and its card links no design brief yet."
                ),
            ),
        }),
        (None, None) => None,
    };
    let link = check
        .design_link
        .clone()
        .or_else(|| design.as_ref().map(|d| d.link.clone()));
    if check.needs_write() {
        let version = check.current.prompt_version.unwrap_or(0);
        out["frontmatter"] = serde_json::json!(format!(
            "---\nkind: project-card\ncommit: {}\nprompt_version: {version}\n{}---",
            check.current.commit.as_deref().unwrap_or("unknown"),
            link.as_deref()
                .map_or_else(String::new, |l| format!("{DESIGN_KEY}: {l}\n"))
        ));
        out["prompt"] = serde_json::json!(prompt_body());
        out["promptVersion"] = serde_json::json!(version);
    }
    if let Some(d) = design {
        let mut value = serde_json::to_value(&d).map_err(|e| e.to_string())?;
        value["linked"] = serde_json::json!(check.design.is_some());
        value["uiSource"] = serde_json::json!(ui);
        value["maxBytes"] = serde_json::json!(DESIGN_MAX);
        value["requiredSections"] = serde_json::json!(DESIGN_SECTIONS);
        value["rewrite"] = serde_json::json!(d.check.needs_write());
        if d.check.needs_write() && d.path.is_some() {
            value["frontmatter"] = serde_json::json!(design_frontmatter(&d.check.current));
            value["prompt"] = serde_json::json!(body_of(DESIGN_PROMPT));
            value["promptVersion"] = serde_json::json!(design_version());
            if check.design.is_none() {
                value["cardFrontmatterLine"] =
                    serde_json::json!(format!("{DESIGN_KEY}: {DESIGN_LINK}"));
            }
        }
        out["design"] = value;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";
    const NEXT: &str = "89abcdef0123456789abcdef0123456789abcdef";

    fn card(commit: &str, version: u32, skip: Option<&str>) -> String {
        let mut text = format!(
            "---\nkind: project-card\ncommit: {commit}\nprompt_version: {version}\n---\n# Card\n"
        );
        for name in REQUIRED {
            if Some(name) == skip {
                continue;
            }
            text.push_str(&format!("\n## {name}\n\nSomething true about {name}.\n"));
        }
        text
    }

    #[test]
    fn the_prompt_names_its_version_on_line_one_and_every_section() {
        assert!(
            version_of(PROMPT).is_some(),
            "line one is prompt-version: N"
        );
        assert!(prompt_version() >= 1);
        for name in REQUIRED {
            assert!(prompt_body().contains(&format!("`## {name}`")), "{name}");
        }
        assert!(!prompt_body().contains("prompt-version"));
    }

    #[test]
    fn a_matching_card_is_fresh_and_carries_no_rewrite() {
        let check = check_text(&card(HEAD, 1, None), Some(HEAD), 1);
        assert_eq!(check.status, Status::Fresh, "{:?}", check.reasons);
        assert!(!check.needs_write());
        let text = render(&check, Path::new("/v/card.md"), None);
        assert!(text.starts_with("Status: fresh"), "{text}");
        assert!(!text.contains("To write it"), "{text}");
        assert!(text.contains("Something true about Risks."));
        // A short commit id that prefixes HEAD names the same commit.
        let short = check_text(&card(&HEAD[..10], 1, None), Some(HEAD), 1);
        assert_eq!(short.status, Status::Fresh);
    }

    #[test]
    fn a_card_regenerates_only_after_a_commit_or_a_prompt_bump() {
        // Same commit, same prompt: nothing to write, however often checked.
        for _ in 0..3 {
            assert!(!check_text(&card(HEAD, 1, None), Some(HEAD), 1).needs_write());
        }
        // A new commit.
        let moved = check_text(&card(HEAD, 1, None), Some(NEXT), 1);
        assert_eq!(moved.status, Status::Stale);
        assert!(
            moved.reasons[0].contains("89abcdef0123"),
            "{:?}",
            moved.reasons
        );
        let text = render(&moved, Path::new("/v/card.md"), Some(Path::new("/repo")));
        assert!(text.contains("To write it"), "{text}");
        assert!(text.contains(&format!("commit: {NEXT}")), "{text}");
        assert!(text.contains("The card as it is"), "{text}");
        // A prompt bump.
        let bumped = check_text(&card(HEAD, 1, None), Some(HEAD), 2);
        assert_eq!(bumped.status, Status::Stale);
        assert!(render(&bumped, Path::new("/v/c.md"), None).contains("prompt_version: 2"));
        // Rewritten at the new key: fresh again.
        assert!(!check_text(&card(NEXT, 2, None), Some(NEXT), 2).needs_write());
    }

    #[test]
    fn a_card_missing_a_section_fails_the_check() {
        for name in REQUIRED {
            let check = check_text(&card(HEAD, 1, Some(name)), Some(HEAD), 1);
            assert_eq!(check.status, Status::Invalid, "{name}");
            assert_eq!(check.missing_sections, vec![name.to_string()]);
            assert!(check.needs_write());
        }
        // An empty section counts as missing, and so does one inside a fence.
        let empty = card(HEAD, 1, None).replace("Something true about Risks.", "");
        assert_eq!(
            check_text(&empty, Some(HEAD), 1).missing_sections,
            vec!["Risks"]
        );
        let fenced = card(HEAD, 1, Some("Risks")) + "\n```\n## Risks\nnone\n```\n";
        assert_eq!(
            check_text(&fenced, Some(HEAD), 1).missing_sections,
            vec!["Risks"]
        );
        // A missing key fails too.
        let bare = card(HEAD, 1, None).replace(&format!("commit: {HEAD}\n"), "");
        assert_eq!(check_text(&bare, Some(HEAD), 1).status, Status::Invalid);
        let unversioned = card(HEAD, 1, None).replace("prompt_version: 1\n", "");
        assert_eq!(
            check_text(&unversioned, Some(HEAD), 1).status,
            Status::Invalid
        );
        // Extra sections and keys (later facets) are ignored.
        let facet = card(HEAD, 1, None).replace("kind:", "design: DESIGN.md\nkind:")
            + "\n## Design\n\nSee DESIGN.md.\n";
        assert_eq!(check_text(&facet, Some(HEAD), 1).status, Status::Fresh);
    }

    #[test]
    fn an_unreadable_commit_compares_the_prompt_version_only() {
        let check = check_text(&card(HEAD, 1, None), None, 1);
        assert_eq!(check.status, Status::Fresh);
        assert!(render(&check, Path::new("/v/c.md"), None).contains("could not be read"));
        assert_eq!(
            check_text(&card(HEAD, 1, None), None, 2).status,
            Status::Stale
        );
    }

    #[test]
    fn head_is_read_from_git_files_for_a_checkout_and_a_linked_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let git = repo.join(".git");
        fs::create_dir_all(git.join("refs/heads")).unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(git.join("refs/heads/main"), format!("{HEAD}\n")).unwrap();
        assert_eq!(head_commit(&repo).as_deref(), Some(HEAD));
        // Packed only.
        fs::remove_file(git.join("refs/heads/main")).unwrap();
        fs::write(
            git.join("packed-refs"),
            format!("# pack-refs with: peeled\n{HEAD} refs/heads/main\n"),
        )
        .unwrap();
        assert_eq!(head_commit(&repo).as_deref(), Some(HEAD));
        // A linked worktree on another branch.
        let private = git.join("worktrees/wt");
        fs::create_dir_all(&private).unwrap();
        fs::write(private.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        fs::write(private.join("commondir"), "../..\n").unwrap();
        fs::write(git.join("refs/heads/feature"), format!("{NEXT}\n")).unwrap();
        let tree = dir.path().join("wt");
        fs::create_dir_all(&tree).unwrap();
        fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", private.display()),
        )
        .unwrap();
        assert_eq!(head_commit(&tree).as_deref(), Some(NEXT));
        // Detached.
        fs::write(private.join("HEAD"), format!("{HEAD}\n")).unwrap();
        assert_eq!(head_commit(&tree).as_deref(), Some(HEAD));
        // No repository, or a ref that climbs out.
        assert_eq!(head_commit(dir.path()), None);
        fs::write(private.join("HEAD"), "ref: refs/../../x\n").unwrap();
        assert_eq!(head_commit(&tree), None);
    }

    #[test]
    fn a_card_cut_to_its_status_line_keeps_the_status_and_the_file() {
        let fresh = check_text(&card(HEAD, 1, None), Some(HEAD), 1);
        let text = render(&fresh, Path::new("/v/card.md"), None);
        let short = status_only(&text).unwrap();
        assert!(short.starts_with("Status: fresh"), "{short}");
        assert!(short.contains("/v/card.md") && short.ends_with(CUT_TO_STATUS));
        assert!(!short.contains("Something true"));
        // Cut once: nothing left to cut, so the trim loop moves on.
        assert_eq!(status_only(&short), None);
        let stale = check_text(&card(HEAD, 1, None), Some(NEXT), 1);
        let short = status_only(&render(&stale, Path::new("/v/c.md"), None)).unwrap();
        assert!(short.starts_with("Status: stale") && short.contains("89abcdef0123"));
    }

    /// BI7: a card that isn't fresh points at the first-connect draft on
    /// its status line (so it survives the trim); a fresh one doesn't, and
    /// without a draft the value is as before.
    #[test]
    fn a_card_that_is_not_fresh_points_at_the_first_connect_draft() {
        let dir = tempfile::tempdir().unwrap();
        let card_file = dir.path().join("PROJECT_CARD.md");
        let missing = missing(Some(HEAD), 1);
        let without = render(&missing, &card_file, None);
        assert!(!without.contains("first-connect draft"));
        assert_eq!(draft_beside(&card_file), None);
        let draft = dir.path().join(crate::first_connect::DRAFT_NAME);
        fs::write(&draft, "draft").unwrap();
        let with = render(&missing, &card_file, None);
        let first = with.lines().next().unwrap();
        assert!(first.contains("first-connect draft"), "{first}");
        assert!(first.contains(&draft.display().to_string()));
        assert_eq!(with.replacen(&draft_line(&card_file), "", 1), without);
        let fresh = check_text(&card(HEAD, 1, None), Some(HEAD), 1);
        assert!(!render(&fresh, &card_file, None).contains("first-connect draft"));
        // A link is never followed.
        fs::remove_file(&draft).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(dir.path().join("elsewhere.md"), &draft).unwrap();
            fs::write(dir.path().join("elsewhere.md"), "x").unwrap();
            assert_eq!(draft_beside(&card_file), None);
        }
    }

    #[test]
    fn an_envelope_over_its_budget_cuts_the_card_before_halving_the_brief() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let w = crate::workspaces::create(root, "Shop", "Ship the shop").unwrap();
        let path = card_path(root, &w.id).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let long = card("unknown", prompt_version(), None).replace(
            "Something true about Modules.",
            &"src/x.rs owns x.\n".repeat(300),
        );
        fs::write(&path, long).unwrap();
        let request = crate::workspaces::SourceRequest {
            query: "",
            reader: crate::workspaces::SourceReader::OwnTools,
            child_packet: None,
        };
        let cwd = Path::new(&w.cwd);
        let full = crate::workspaces::context_for(root, cwd, &request)
            .unwrap()
            .unwrap();
        let brief = full.parts.as_ref().unwrap().json["WORKSPACE.md"].clone();
        assert!(full.text.contains("src/x.rs owns x."));
        // A budget the brief fits in but not the card: the card is cut to
        // its status line and the brief stays whole.
        let budget = full.text.len() - 2_000;
        let cut = crate::workspaces::context_for_within(root, cwd, &request, budget)
            .unwrap()
            .unwrap();
        let json = &cut.parts.as_ref().unwrap().json;
        let value = json[ENVELOPE_KEY].as_str().unwrap();
        assert!(
            value.starts_with("Status: fresh") && value.ends_with(CUT_TO_STATUS),
            "{value}"
        );
        assert_eq!(json["WORKSPACE.md"], brief);
        assert!(cut.text.len() <= budget, "{} > {budget}", cut.text.len());
    }

    #[test]
    fn a_workspace_card_on_disk_is_checked_and_reaches_the_envelope_only_when_relevant() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let w = crate::workspaces::create(root, "Shop", "Ship the shop").unwrap();
        // No repository and no card: no value, so its envelope is unchanged.
        assert_eq!(envelope_value(root, &w), None);
        let envelope = |root: &Path| {
            crate::workspaces::context(root, Path::new(&w.cwd))
                .unwrap()
                .unwrap()
        };
        let plain = envelope(root);
        assert!(!plain.contains(ENVELOPE_KEY) && !plain.contains(ENVELOPE_NOTE));
        let action = action(root, &w.id).unwrap();
        assert_eq!(action["status"], "missing");
        assert_eq!(action["rewrite"], true);
        assert!(action["frontmatter"]
            .as_str()
            .unwrap()
            .contains("commit: unknown"));
        // A card without a section: the envelope says invalid and how to fix it.
        let path = card_path(root, &w.id).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, card("unknown", prompt_version(), Some("Modules"))).unwrap();
        let text = envelope_value(root, &w).unwrap();
        assert!(text.starts_with("Status: invalid"), "{text}");
        assert!(
            text.contains("Missing or empty sections: Modules."),
            "{text}"
        );
        // Whole, with no repository to compare: fresh.
        fs::write(&path, card("unknown", prompt_version(), None)).unwrap();
        let text = envelope_value(root, &w).unwrap();
        assert!(text.starts_with("Status: fresh"), "{text}");
        let full = envelope(root);
        assert!(full.contains(ENVELOPE_NOTE), "{full}");
        assert!(full.contains("\"projectCard\":\"Status: fresh"), "{full}");
        let action = super::action(root, &w.id).unwrap();
        assert_eq!(action["rewrite"], false);
        assert!(action.get("prompt").is_none());
        // Refusals carry a code.
        assert!(super::action(root, "../x")
            .unwrap_err()
            .starts_with("INVALID_ARGUMENT:"));
        assert!(super::action(root, "nope")
            .unwrap_err()
            .starts_with("NOT_FOUND:"));
    }

    fn brief(commit: &str, version: u32, skip: Option<&str>) -> String {
        let mut text =
            format!("---\nkind: design-brief\ncommit: {commit}\nprompt_version: {version}\n---\n");
        for name in DESIGN_SECTIONS {
            if Some(name) == skip {
                continue;
            }
            text.push_str(&format!(
                "\n## {name}\n\nForest green #1F3D2B for {name}.\n"
            ));
        }
        text
    }

    #[test]
    fn the_design_prompt_names_its_version_and_five_sections() {
        assert!(design_version() >= 1);
        for name in DESIGN_SECTIONS {
            assert!(
                body_of(DESIGN_PROMPT).contains(&format!("`## {name}`")),
                "{name}"
            );
        }
        assert!(body_of(DESIGN_PROMPT).contains("2048 bytes"));
        assert!(brief(HEAD, 1, None).len() as u64 <= DESIGN_MAX);
    }

    #[test]
    fn a_card_without_a_design_link_renders_as_before() {
        let check = check_text(&card(HEAD, 1, None), Some(HEAD), 1);
        assert_eq!(check.design_link, None);
        let text = render(&check, Path::new("/v/card.md"), None);
        assert!(!text.contains("Design brief") && !text.contains("design:"));
        let stale = check_text(&card(HEAD, 1, None), Some(NEXT), 1);
        assert!(!render(&stale, Path::new("/v/c.md"), None).contains("design:"));
        // A linked card keeps its link across a rewrite.
        let linked = card(HEAD, 1, None).replace("kind:", "design: DESIGN.md\nkind:");
        let stale = check_text(&linked, Some(NEXT), 1);
        assert_eq!(stale.design_link.as_deref(), Some("DESIGN.md"));
        let text = render(&stale, Path::new("/v/c.md"), None);
        assert!(
            text.contains("prompt_version: 1\ndesign: DESIGN.md\n---"),
            "{text}"
        );
    }

    #[test]
    fn a_linked_design_brief_is_checked_for_place_size_sections_and_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let w = crate::workspaces::create(root, "Shop", "Ship the shop").unwrap();
        let card_file = card_path(root, &w.id).unwrap();
        fs::create_dir_all(card_file.parent().unwrap()).unwrap();
        let linked = |link: &str| {
            card("unknown", prompt_version(), None)
                .replace("kind:", &format!("design: {link}\nkind:"))
        };
        fs::write(&card_file, linked("DESIGN.md")).unwrap();
        let design_file = card_file.with_file_name("DESIGN.md");
        let design = |root: &Path| {
            *check_with(root, &w, prompt_version())
                .unwrap()
                .design
                .expect("linked")
        };
        // Linked but not written: missing, with the prompt in the value.
        let d = design(root);
        assert_eq!(d.check.status, Status::Missing);
        assert!(d.path.as_deref().unwrap().ends_with("outputs/DESIGN.md"));
        let text = envelope_value(root, &w).unwrap();
        let first = text.lines().next().unwrap();
        assert!(
            first.starts_with("Status: fresh") && first.contains("Design brief: missing"),
            "{first}"
        );
        assert!(
            text.contains("kind: design-brief") && text.contains("`## Typography`"),
            "{text}"
        );
        let out = action(root, &w.id).unwrap();
        assert_eq!(out["design"]["status"], "missing");
        assert_eq!(out["design"]["linked"], true);
        assert!(out["design"]["prompt"]
            .as_str()
            .unwrap()
            .contains("`## Layout`"));
        // Written whole: fresh, its text after the card, its line on the status line.
        fs::write(&design_file, brief("unknown", design_version(), None)).unwrap();
        assert_eq!(design(root).check.status, Status::Fresh);
        let text = envelope_value(root, &w).unwrap();
        assert!(
            text.lines().next().unwrap().contains("Design brief: fresh"),
            "{text}"
        );
        assert!(text.contains("Forest green #1F3D2B for Colors."), "{text}");
        assert!(!text.contains("To write the design brief"));
        let short = status_only(&text).unwrap();
        assert!(short.contains("Design brief: fresh") && !short.contains("Forest green"));
        let out = action(root, &w.id).unwrap();
        assert_eq!(out["design"]["rewrite"], false);
        assert!(out["design"].get("prompt").is_none());
        // A section missing, or over 2 KB: invalid.
        fs::write(
            &design_file,
            brief("unknown", design_version(), Some("Colors")),
        )
        .unwrap();
        let d = design(root);
        assert_eq!(d.check.status, Status::Invalid);
        assert_eq!(d.check.missing_sections, vec!["Colors"]);
        let big = brief("unknown", design_version(), None)
            .replace("for Layout.", &"x".repeat(DESIGN_MAX as usize));
        fs::write(&design_file, big).unwrap();
        let d = design(root);
        assert_eq!(d.check.status, Status::Invalid);
        assert!(
            d.check.reasons[0].contains("2048 bytes"),
            "{:?}",
            d.check.reasons
        );
        // A new commit or a design prompt bump: stale.
        let stale = check_design(root, &w.id, "DESIGN.md", Some(NEXT), 1);
        assert_eq!(stale.check.status, Status::Invalid, "still too big");
        fs::write(&design_file, brief(HEAD, 1, None)).unwrap();
        let stale = check_design(root, &w.id, "DESIGN.md", Some(NEXT), 1);
        assert_eq!(stale.check.status, Status::Stale);
        assert!(stale.check.reasons[0].contains("the brief was written at"));
        let bumped = check_design(root, &w.id, "DESIGN.md", Some(HEAD), 2);
        assert_eq!(bumped.check.status, Status::Stale);
        assert!(bumped.check.reasons[0].contains("design prompt is at version 2"));
        assert_eq!(
            check_design(root, &w.id, "./DESIGN.md", Some(HEAD), 1)
                .check
                .status,
            Status::Fresh
        );
        // Links that leave the workspace, and a symlink, are refused.
        for link in ["../../x.md", "/etc/hosts", "../../../outside.md"] {
            let d = check_design(root, &w.id, link, None, 1);
            assert_eq!(d.check.status, Status::Invalid, "{link}");
            assert_eq!(d.path, None, "{link}");
        }
        let elsewhere = dir.path().join("elsewhere.md");
        fs::write(&elsewhere, brief(HEAD, 1, None)).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&elsewhere, card_file.with_file_name("LINK.md")).unwrap();
            let d = check_design(root, &w.id, "LINK.md", Some(HEAD), 1);
            assert_eq!(d.check.status, Status::Invalid);
            assert!(d.check.reasons[0].contains("not a regular file"));
        }
        // A folder link that leads out (into a repository, say): refused
        // before the file exists, so the envelope never says to write there.
        #[cfg(unix)]
        {
            let repo = dir.path().join("repo");
            fs::create_dir_all(&repo).unwrap();
            std::os::unix::fs::symlink(&repo, card_file.with_file_name("site")).unwrap();
            let d = check_design(root, &w.id, "site/DESIGN.md", None, 1);
            assert_eq!(d.check.status, Status::Invalid);
            assert!(
                d.check.reasons[0].contains("outside the workspace"),
                "{:?}",
                d.check.reasons
            );
            let d = check_design(root, &w.id, "site/new/DESIGN.md", None, 1);
            assert_eq!(d.check.status, Status::Invalid);
            fs::write(repo.join("DESIGN.md"), brief(HEAD, 1, None)).unwrap();
            let d = check_design(root, &w.id, "site/DESIGN.md", Some(HEAD), 1);
            assert_eq!(d.check.status, Status::Invalid);
            assert_eq!(d.bytes, None);
        }
        // A link to the workspace root still resolves inside it.
        let up = check_design(root, &w.id, "../DESIGN.md", None, 1);
        assert_eq!(up.check.status, Status::Missing);
        assert!(!up.path.unwrap().contains("outputs"));
    }

    #[test]
    fn ui_source_is_detected_by_marker_files_only() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(ui_marker(dir.path()), None);
        fs::create_dir_all(dir.path().join("src/app")).unwrap();
        fs::write(dir.path().join("src/app/globals.css"), "@theme {}").unwrap();
        assert_eq!(ui_marker(dir.path()), Some("src/app/globals.css"));
        fs::write(dir.path().join("tailwind.config.ts"), "export default {}").unwrap();
        assert_eq!(ui_marker(dir.path()), Some("tailwind.config.ts"));
    }
}
