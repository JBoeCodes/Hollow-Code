use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) static SKILL_WRITE: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub name: String,
    /// "shared" | "provider:<provider>" | provider-qualified plugin/marketplace.
    pub source: String,
    pub source_kind: String, // "user" (shared or provider) | "plugin" | "marketplace"
    pub plugin: Option<String>,
    pub path: String, // absolute path to SKILL.md
    pub description: String,
    pub version: Option<String>,
    pub writable: bool,
    /// The static health check (NV1), for the shared library's skills only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lint: Option<crate::skill_lint::SkillLint>,
}

/// Shared skills are portable and editable; provider installations are discovery
/// sources only. A duplicate name never hides a skill from another source.
pub fn list_all() -> Vec<Skill> {
    dirs::home_dir()
        .map(|home| list_all_in(&home))
        .unwrap_or_default()
}

pub(crate) fn list_all_in(home: &Path) -> Vec<Skill> {
    let mut out = Vec::new();
    for (relative, source, writable) in [
        (".agents/skills", "shared", true),
        (".codex/skills", "provider:codex", false),
        (".codex/skills/.system", "provider:codex-system", false),
        (".claude/skills", "provider:claude", false),
    ] {
        scan_skills_dir(
            &home.join(relative),
            |path, name| Skill {
                name: name.to_string(),
                source: source.to_string(),
                source_kind: "user".into(),
                plugin: None,
                path: path.to_string_lossy().to_string(),
                description: String::new(),
                version: None,
                writable: writable
                    && managed_skill_dir(home, name, false).is_ok()
                    && !path
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(true),
                lint: None,
            },
            &mut out,
        );
    }
    for provider in ["codex", "claude"] {
        scan_plugin_sources(home, provider, &mut out);
    }

    for skill in &mut out {
        if let Ok(content) = fs::read_to_string(&skill.path) {
            let (description, version, name) = parse_skill_meta(&content);
            // NV1: the shared library's own skills get a static health check
            // (its folder name is the skill's name until the frontmatter's
            // overrides it just below).
            if skill.source == "shared" {
                skill.lint = Some(crate::skill_lint::lint(
                    &skill.name,
                    &content,
                    Path::new(&skill.path).parent(),
                ));
            }
            skill.description = description;
            skill.version = version.or_else(|| skill.version.clone());
            if let Some(name) = name.filter(|n| !n.is_empty()) {
                // The write/delete IPC takes a name. A foreign skill whose
                // invocation differs from its folder cannot be edited by that
                // interface without risking a different directory.
                if name != skill.name {
                    skill.writable = false;
                }
                skill.name = name;
            }
        }
    }
    // A provider root may symlink the shared directory. List that physical
    // file once, preserving the shared source's first-place precedence.
    let mut seen = std::collections::HashSet::new();
    out.retain(|skill| {
        fs::canonicalize(&skill.path)
            .ok()
            .map(|path| seen.insert(path))
            .unwrap_or(false)
    });
    out.sort_by(|a, b| {
        let order = |s: &Skill| {
            if s.source == "shared" {
                0
            } else if s.source_kind == "user" {
                1
            } else if s.source_kind == "plugin" {
                2
            } else {
                3
            }
        };
        order(a)
            .cmp(&order(b))
            .then(a.name.cmp(&b.name))
            .then(a.source.cmp(&b.source))
            .then(a.path.cmp(&b.path))
    });
    out
}

fn directories(path: &Path) -> Vec<PathBuf> {
    let mut entries = fs::read_dir(path)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn leaf(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn scan_plugin_sources(home: &Path, provider: &str, out: &mut Vec<Skill>) {
    let plugins_root = home.join(format!(".{provider}/plugins"));
    for market in directories(&plugins_root.join("cache")) {
        for plugin in directories(&market) {
            for version in directories(&plugin) {
                let plugin_name = leaf(&plugin);
                let source = format!(
                    "plugin:{provider}/{}/{}@{}",
                    leaf(&market),
                    plugin_name,
                    leaf(&version)
                );
                scan_skills_dir(
                    &version.join("skills"),
                    |path, name| Skill {
                        name: name.to_string(),
                        source: source.clone(),
                        source_kind: "plugin".into(),
                        plugin: Some(plugin_name.clone()),
                        path: path.to_string_lossy().to_string(),
                        description: String::new(),
                        version: Some(leaf(&version)),
                        writable: false,
                        lint: None,
                    },
                    out,
                );
            }
        }
    }
    for market in directories(&plugins_root.join("marketplaces")) {
        for subdir in ["plugins", "external_plugins"] {
            for plugin in directories(&market.join(subdir)) {
                let plugin_name = leaf(&plugin);
                let source = format!("marketplace:{provider}/{}/{}", leaf(&market), plugin_name);
                scan_skills_dir(
                    &plugin.join("skills"),
                    |path, name| Skill {
                        name: name.to_string(),
                        source: source.clone(),
                        source_kind: "marketplace".into(),
                        plugin: Some(plugin_name.clone()),
                        path: path.to_string_lossy().to_string(),
                        description: String::new(),
                        version: None,
                        writable: false,
                        lint: None,
                    },
                    out,
                );
            }
        }
    }
}

fn scan_skills_dir<F>(skills_dir: &Path, mut mk: F, out: &mut Vec<Skill>)
where
    F: FnMut(&Path, &str) -> Skill,
{
    let Ok(entries) = fs::read_dir(skills_dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_file = path.join("SKILL.md");
        if !skill_file.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        out.push(mk(&skill_file, &name));
    }
}

/// Parse `name`, `description`, `version` out of a SKILL.md YAML frontmatter
/// block. Returns (description, version, name).
pub(crate) fn parse_skill_meta(content: &str) -> (String, Option<String>, Option<String>) {
    if !content.starts_with("---") {
        return (String::new(), None, None);
    }
    let after_open = match content.find('\n') {
        Some(i) => &content[i + 1..],
        None => return (String::new(), None, None),
    };
    let end = match after_open.find("\n---") {
        Some(i) => i,
        None => return (String::new(), None, None),
    };
    let yaml = &after_open[..end];
    let mut desc = String::new();
    let mut version: Option<String> = None;
    let mut name: Option<String> = None;
    // Naive YAML: key: value pairs, indented continuation lines, and block
    // scalars (`description: >` folds its lines, `|` keeps their newlines;
    // overnight 9 A5). Not full YAML.
    let mut current_key: Option<String> = None;
    let mut current_val = String::new();
    let mut block: Option<u8> = None;
    for line in yaml.lines() {
        // A blank line inside a block scalar is a line break in both styles.
        if line.trim().is_empty() && current_key.is_some() && block.is_some() {
            current_val.push('\n');
            continue;
        }
        // Continuation lines (indented) extend the previous value.
        if line.starts_with(' ') || line.starts_with('\t') {
            if current_key.is_some() {
                // `|` keeps every break, a blank line's too ("a\n\nb");
                // `>` and plain values join with a space.
                let after_break = current_val.is_empty() || current_val.ends_with('\n');
                if block == Some(b'|') && !current_val.is_empty() {
                    current_val.push('\n');
                } else if !after_break {
                    current_val.push(' ');
                }
                current_val.push_str(line.trim());
            }
            continue;
        }
        // Commit previous key.
        if let Some(k) = current_key.take() {
            commit_yaml_kv(&k, current_val.trim(), &mut desc, &mut version, &mut name);
            current_val.clear();
        }
        block = None;
        if let Some(idx) = line.find(':') {
            let k = line[..idx].trim().to_string();
            let v = line[idx + 1..].trim().to_string();
            current_key = Some(k);
            block = block_indicator(&v);
            current_val = if block.is_some() { String::new() } else { v };
        }
    }
    if let Some(k) = current_key {
        commit_yaml_kv(&k, current_val.trim(), &mut desc, &mut version, &mut name);
    }
    (desc, version, name)
}

/// `Some(b'>')` or `Some(b'|')` when a value is only a YAML block-scalar
/// header: the indicator, then at most a chomping sign (`+`/`-`) and an
/// indentation digit in either order, optionally followed by a comment.
fn block_indicator(value: &str) -> Option<u8> {
    let head = value.split(" #").next().unwrap_or("").trim_end();
    let bytes = head.as_bytes();
    let (&style, rest) = bytes.split_first()?;
    if style != b'>' && style != b'|' || rest.len() > 2 {
        return None;
    }
    let signs = rest.iter().filter(|b| matches!(b, b'+' | b'-')).count();
    let digits = rest.iter().filter(|b| matches!(b, b'1'..=b'9')).count();
    (signs <= 1 && digits <= 1 && signs + digits == rest.len()).then_some(style)
}

fn commit_yaml_kv(
    key: &str,
    val: &str,
    desc: &mut String,
    version: &mut Option<String>,
    name: &mut Option<String>,
) {
    let v = unquote(val);
    match key {
        "description" => *desc = v,
        "version" => *version = Some(v),
        "name" => *name = Some(v),
        _ => {}
    }
}

fn unquote(s: &str) -> String {
    // Our writer emits JSON strings, a YAML-compatible double-quoted subset.
    // Decode once so literal backslashes and escaped newlines round-trip.
    if let Ok(value) = serde_json::from_str::<String>(s) {
        return value;
    }
    let b = s.as_bytes();
    if b.len() >= 2
        && ((b[0] == b'"' && b[b.len() - 1] == b'"') || (b[0] == b'\'' && b[b.len() - 1] == b'\''))
    {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Read the raw SKILL.md content at an absolute path. Two-layer
/// defense against a compromised renderer asking for arbitrary files
/// under provider configuration roots (settings, MCP config, plugin secrets):
///
///   1. The path must be `SKILL.md` — by filename only.
///   2. The path must match a `SKILL.md` returned by `list_all()`.
///      That bounds reads to the exact files discovered as skills
///      under the user, plugin-cache, and marketplace roots.
pub fn read_skill(path: &str) -> Result<String, String> {
    let home = dirs::home_dir().ok_or_else(|| "no home dir".to_string())?;
    read_skill_in(&home, path)
}

pub(crate) fn read_skill_in(home: &Path, path: &str) -> Result<String, String> {
    let pb = PathBuf::from(path);
    let canon = pb
        .canonicalize()
        .map_err(|e| format!("canonicalize: {e}"))?;
    if canon.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Err(format!("only SKILL.md is readable: {path}"));
    }
    // Cross-check against the discovered skill set. Slightly more
    // expensive than a static prefix check (list_all walks the tree),
    // but it's a once-per-skill-open user action, and it's the only
    // way to validate that this SKILL.md is a real skill and not just
    // an unrelated file someone named SKILL.md.
    let known: Vec<PathBuf> = list_all_in(home)
        .iter()
        .filter_map(|s| PathBuf::from(&s.path).canonicalize().ok())
        .collect();
    if !known.iter().any(|p| p == &canon) {
        return Err(format!("path not registered as a skill: {path}"));
    }
    fs::read_to_string(&canon).map_err(|e| format!("read: {e}"))
}

/// Write a shared skill. Creates `~/.agents/skills/<name>/SKILL.md`,
/// formatting frontmatter + body. Refuses an empty or unsafe name.
pub fn write_user_skill(
    name: &str,
    description: &str,
    body: &str,
    create_only: bool,
) -> Result<Skill, String> {
    let _held = SKILL_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let home = dirs::home_dir().ok_or_else(|| "no home dir".to_string())?;
    write_user_skill_with_mode(&home, name, description, body, create_only)
}

#[cfg(test)]
fn write_user_skill_in(
    home: &Path,
    name: &str,
    description: &str,
    body: &str,
) -> Result<Skill, String> {
    write_user_skill_with_mode(home, name, description, body, true)
}

fn write_user_skill_with_mode(
    home: &Path,
    name: &str,
    description: &str,
    body: &str,
    create_only: bool,
) -> Result<Skill, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("name is required".into());
    }
    if !valid_skill_name(trimmed) {
        return Err("name must be alphanumeric, dashes, or underscores".into());
    }
    let saved_description = if description.trim().is_empty() {
        trimmed
    } else {
        description
    };
    let desc_line = format!(
        "description: {}",
        serde_json::to_string(saved_description).map_err(|e| format!("description: {e}"))?
    );
    let content = format!(
        "---\nname: {trimmed}\n{desc_line}\n---\n\n{body}\n",
        trimmed = trimmed,
        desc_line = desc_line,
        body = body.trim_end()
    );
    let mode = if create_only {
        SkillWrite::New
    } else {
        SkillWrite::Existing { create: true }
    };
    let file = write_skill_file(home, trimmed, &content, mode)?;

    Ok(Skill {
        name: trimmed.to_string(),
        source: "shared".into(),
        source_kind: "user".into(),
        plugin: None,
        path: file.to_string_lossy().to_string(),
        description: saved_description.to_string(),
        version: None,
        writable: true,
        lint: None,
    })
}

/// How [`write_skill_file`] may place a shared skill's SKILL.md.
#[derive(Clone, Copy)]
pub(crate) enum SkillWrite {
    /// A new skill. Its folder is created exclusively, so any existing
    /// folder, even an empty one, is refused with `SKILL_EXISTS`.
    New,
    /// A managed skill folder that exists; `create` makes a missing one.
    Existing { create: bool },
}

/// The one protected writer for a shared skill's SKILL.md, used by New
/// skill, edits and Saved skill lessons: a safe name, no symlinked
/// `.agents`, `skills`, skill folder or SKILL.md, and an atomic write.
/// Callers hold [`SKILL_WRITE`].
pub(crate) fn write_skill_file(
    home: &Path,
    name: &str,
    content: &str,
    mode: SkillWrite,
) -> Result<PathBuf, String> {
    if !valid_skill_name(name) {
        return Err("name must be alphanumeric, dashes, or underscores".into());
    }
    let skill_dir = match mode {
        SkillWrite::New => {
            let root = home.join(".agents");
            let skills = root.join("skills");
            for path in [&root, &skills] {
                reject_symlink(path)?;
                fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
            }
            let dir = skills.join(name);
            // Exclusive creation also refuses an empty directory; no check/write race.
            fs::create_dir(&dir).map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("SKILL_EXISTS: /{name} already exists")
                } else {
                    format!("mkdir: {e}")
                }
            })?;
            dir
        }
        SkillWrite::Existing { create } => managed_skill_dir(home, name, create)?,
    };
    let file = skill_dir.join("SKILL.md");
    reject_symlink(&file)?;
    crate::vault::atomic_write(&file, content.as_bytes()).map_err(|e| format!("write: {e}"))?;
    Ok(file)
}

pub fn delete_user_skill(name: &str) -> Result<(), String> {
    let _held = SKILL_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    let home = dirs::home_dir().ok_or_else(|| "no home dir".to_string())?;
    delete_user_skill_in(&home, name)
}

fn delete_user_skill_in(home: &Path, name: &str) -> Result<(), String> {
    let dir = managed_skill_dir(home, name.trim(), false)?;
    fs::remove_dir_all(&dir).map_err(|e| format!("rmdir: {e}"))
}

pub(crate) fn reject_symlink(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "shared skill path must not be a symlink: {}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("skill path: {error}")),
    }
}

/// A shared skill's name, which is also its folder: ASCII letters, digits,
/// `-` and `_`. New skill and Import both hold names to it.
pub(crate) fn valid_skill_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Longest name [`suggest_skill_name`] returns (the cap other agents put on a
/// skill name).
pub(crate) const MAX_SUGGESTED_NAME: usize = 64;

/// SD2: a legal name close to `raw`, for an import whose SKILL.md names a skill
/// something [`valid_skill_name`] refuses (`stitch::generate-design`). Pure:
/// lowercase kebab, every run of characters that isn't an ASCII letter, digit
/// or `_` (`::`, `/`, spaces, dots, accents, emoji, an existing `-`) becomes
/// one `-`, trimmed of `-` at both ends and cut at [`MAX_SUGGESTED_NAME`].
/// `None` when no letter or digit remains, so there is nothing to suggest.
pub(crate) fn suggest_skill_name(raw: &str) -> Option<String> {
    let mut out = String::new();
    let mut gap = false;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    out.truncate(MAX_SUGGESTED_NAME);
    let out = out.trim_end_matches('-').to_string();
    out.chars()
        .any(|c| c.is_ascii_alphanumeric())
        .then_some(out)
        .filter(|name| valid_skill_name(name))
}

/// Scope writes to physical shared-skill directories. A symlink from a shared
/// path into a provider's installation must not turn that provider writable.
pub(crate) fn managed_skill_dir(home: &Path, name: &str, create: bool) -> Result<PathBuf, String> {
    if !valid_skill_name(name) {
        return Err("name must be alphanumeric, dashes, or underscores".into());
    }
    let root = home.join(".agents");
    let skills = root.join("skills");
    let dir = skills.join(name);
    for path in [&root, &skills, &dir] {
        reject_symlink(path)?;
        if create {
            fs::create_dir_all(path).map_err(|e| format!("mkdir: {e}"))?;
        }
        if !path.is_dir() {
            return Err(format!("skill directory not found: {}", path.display()));
        }
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_suggested_skill_name_is_lowercase_kebab_and_legal() {
        for (raw, want) in [
            ("stitch::generate-design", "stitch-generate-design"),
            ("Stitch::Generate Design", "stitch-generate-design"),
            ("a/b.c d", "a-b-c-d"),
            ("  --Foo__Bar--  ", "foo__bar"),
            ("x:::y///z", "x-y-z"),
            ("caf\u{e9} au lait", "caf-au-lait"),
            ("v1.2", "v1-2"),
            ("a_-_b", "a_-_b"),
        ] {
            let got = suggest_skill_name(raw);
            assert_eq!(got.as_deref(), Some(want), "{raw:?}");
            assert!(valid_skill_name(&got.unwrap()), "{raw:?}");
        }
        // Nothing legal to build from.
        for raw in ["", ":::", "  ", "\u{1F600}", "-_-"] {
            assert_eq!(suggest_skill_name(raw), None, "{raw:?}");
        }
        // Cut at the cap without a trailing dash.
        let long = format!("{}::{}", "a".repeat(63), "b".repeat(10));
        let got = suggest_skill_name(&long).unwrap();
        assert_eq!(got, "a".repeat(63));
        assert!(got.len() <= MAX_SUGGESTED_NAME);
    }

    fn description_of(frontmatter: &str) -> String {
        parse_skill_meta(&format!("---\n{frontmatter}\n---\n\nBody.\n")).0
    }

    /// Overnight 9 A5 (red first): a block-scalar description read as "> …"
    /// in the Skills library and in every agent's skill list.
    #[test]
    fn block_scalar_descriptions_fold_or_keep_their_lines() {
        let folded = "name: hyperframes\ndescription: >\n  Mandatory entry point: read this\n  first for any video.\nlicense: MIT";
        assert_eq!(
            description_of(folded),
            "Mandatory entry point: read this first for any video."
        );
        assert_eq!(
            description_of("description: >-\n  One\n  two"),
            "One two",
            "folded, strip"
        );
        assert_eq!(
            description_of("description: >\n  First paragraph\n  goes on.\n\n  Second."),
            "First paragraph goes on.\nSecond.",
            "a blank line is a line break"
        );
        let literal = "name: humanizer\nversion: 2.7.0\ndescription: |\n  Remove signs of AI writing.\n  Based on a guide.\nlicense: MIT";
        let (desc, version, name) = parse_skill_meta(&format!("---\n{literal}\n---\n"));
        assert_eq!(desc, "Remove signs of AI writing.\nBased on a guide.");
        assert_eq!(version.as_deref(), Some("2.7.0"));
        assert_eq!(name.as_deref(), Some("humanizer"));
        assert_eq!(
            description_of("description: |\n  One\n\n  Two"),
            "One\n\nTwo",
            "literal keeps a blank line"
        );
        assert_eq!(
            parse_skill_meta("---\r\ndescription: |\r\n  Kept\r\n  CRLF\r\n---\r\n").0,
            "Kept\nCRLF"
        );
        assert_eq!(
            description_of("description: |+\n  Kept\n  lines\n"),
            "Kept\nlines",
            "literal, keep (then trimmed)"
        );
        assert_eq!(description_of("description: >2\n    Indented"), "Indented");
        assert_eq!(
            description_of("description: > # a comment\n  Commented"),
            "Commented"
        );
    }

    #[test]
    fn plain_and_quoted_descriptions_read_as_before() {
        assert_eq!(description_of("description: One line."), "One line.");
        assert_eq!(
            description_of("description: \"Quoted: with a colon\""),
            "Quoted: with a colon"
        );
        assert_eq!(
            description_of("description: Starts here\n  and goes on"),
            "Starts here and goes on"
        );
        // Not a block header: text that merely starts with the indicator.
        assert_eq!(
            description_of("description: > not a header"),
            "> not a header"
        );
        assert_eq!(description_of("description: |pipe"), "|pipe");
        assert_eq!(
            parse_skill_meta(
                "---\r\nname: crlf\r\ndescription: >\r\n  Folded\r\n  CRLF\r\n---\r\n"
            )
            .0,
            "Folded CRLF"
        );
    }

    fn skill(home: &Path, relative: &str, name: &str) -> PathBuf {
        let path = home.join(relative).join("SKILL.md");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!("---\nname: {name}\ndescription: Test skill\n---\n\nUse this skill.\n"),
        )
        .unwrap();
        path
    }

    #[test]
    fn shared_skills_are_first_and_provider_name_collisions_remain_visible() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        skill(home, ".agents/skills/shared-name", "shared-name");
        skill(home, ".codex/skills/shared-name", "shared-name");
        skill(home, ".claude/skills/shared-name", "shared-name");
        let listed = list_all_in(home);
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].source, "shared");
        assert!(listed[0].writable);
        assert!(listed
            .iter()
            .filter(|s| s.source.starts_with("provider:"))
            .all(|s| !s.writable));
        assert!(listed.iter().all(|s| s.name == "shared-name"));
        for item in listed {
            assert!(read_skill_in(home, &item.path)
                .unwrap()
                .contains("Use this skill."));
        }
    }

    #[test]
    fn provider_plugin_caches_and_system_skills_are_read_only_sources() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        skill(
            home,
            ".codex/plugins/cache/bundled/demo/1.0.0/skills/inspect",
            "inspect",
        );
        skill(
            home,
            ".claude/plugins/cache/market/demo/2.0.0/skills/inspect",
            "inspect",
        );
        skill(home, ".codex/skills/.system/inspect", "inspect");
        let listed = list_all_in(home);
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().all(|s| !s.writable));
        assert!(listed
            .iter()
            .any(|s| s.source == "plugin:codex/bundled/demo@1.0.0"));
        assert!(listed
            .iter()
            .any(|s| s.source == "plugin:claude/market/demo@2.0.0"));
        assert!(listed.iter().any(|s| s.source == "provider:codex-system"));
    }

    #[test]
    fn saved_descriptions_round_trip_yaml_punctuation_and_line_breaks() {
        let temp = tempfile::tempdir().unwrap();
        for (index, description) in [
            "Use when: reviewing # notes",
            "\"Quoted\" and \\paths",
            "First line\n\n  Indented second\r\nLast line",
            "---\nname: different\n---",
            "true",
        ]
        .iter()
        .enumerate()
        {
            let name = format!("example-{index}");
            let created = write_user_skill_in(temp.path(), &name, description, "Body").unwrap();
            let saved = fs::read_to_string(&created.path).unwrap();
            let encoded = saved
                .lines()
                .find_map(|line| line.strip_prefix("description: "))
                .unwrap();
            // JSON strings are YAML double-quoted scalars, so another YAML
            // reader can consume the same escaped description safely.
            assert_eq!(
                serde_json::from_str::<String>(encoded).unwrap(),
                *description
            );
            assert_eq!(parse_skill_meta(&saved).0, *description);
            write_user_skill_with_mode(temp.path(), &name, description, "Edited", false).unwrap();
            let listed = list_all_in(temp.path());
            assert_eq!(
                listed.iter().find(|s| s.name == name).unwrap().description,
                *description
            );
        }
    }

    #[test]
    fn create_only_preserves_an_existing_skill() {
        let temp = tempfile::tempdir().unwrap();
        let original = write_user_skill_in(temp.path(), "foo", "Original", "Keep my work").unwrap();
        let before = fs::read(&original.path).unwrap();
        assert!(write_user_skill_in(temp.path(), "foo", "Replacement", "New content").is_err());
        assert_eq!(fs::read(&original.path).unwrap(), before);
        write_user_skill_with_mode(temp.path(), "foo", "Replacement", "Approved", false).unwrap();
        assert!(fs::read_to_string(&original.path)
            .unwrap()
            .contains("Approved"));
        fs::create_dir(temp.path().join(".agents/skills/empty")).unwrap();
        assert!(write_user_skill_in(temp.path(), "empty", "", "No").is_err());
    }

    #[test]
    fn write_and_delete_only_change_shared_skills() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let legacy = skill(home, ".claude/skills/example", "example");
        let codex = skill(home, ".codex/skills/example", "example");
        let before = fs::read_to_string(&legacy).unwrap();
        let created =
            write_user_skill_in(home, "example", "Shared description", "Shared body").unwrap();
        assert_eq!(created.source, "shared");
        assert!(Path::new(&created.path).starts_with(home.join(".agents/skills")));
        assert_eq!(fs::read_to_string(&legacy).unwrap(), before);
        assert_eq!(fs::read_to_string(&codex).unwrap(), before);
        delete_user_skill_in(home, "example").unwrap();
        assert!(!Path::new(&created.path).exists());
        assert_eq!(fs::read_to_string(&legacy).unwrap(), before);
        assert_eq!(fs::read_to_string(&codex).unwrap(), before);
        assert!(write_user_skill_in(home, "../escape", "", "").is_err());
        assert!(delete_user_skill_in(home, "../escape").is_err());
    }

    #[test]
    fn reading_requires_an_exact_registered_skill_file() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let known = skill(home, ".agents/skills/known", "known");
        let unknown = skill(home, "elsewhere/unknown", "unknown");
        let config = home.join(".codex/settings.json");
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, "private settings").unwrap();
        assert!(read_skill_in(home, known.to_str().unwrap()).is_ok());
        assert!(read_skill_in(home, unknown.to_str().unwrap())
            .unwrap_err()
            .contains("not registered"));
        assert!(read_skill_in(home, config.to_str().unwrap())
            .unwrap_err()
            .contains("only SKILL.md"));
    }

    #[cfg(unix)]
    #[test]
    fn shared_symlinks_cannot_make_a_provider_installation_writable() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path();
        let legacy = skill(home, ".claude/skills/example", "example");
        fs::create_dir_all(home.join(".agents/skills")).unwrap();
        symlink(
            legacy.parent().unwrap(),
            home.join(".agents/skills/example"),
        )
        .unwrap();
        let listed = list_all_in(home);
        assert_eq!(listed.len(), 1, "the same physical skill is listed once");
        assert!(!listed[0].writable);
        assert!(write_user_skill_in(home, "example", "", "overwrite").is_err());
        assert!(delete_user_skill_in(home, "example").is_err());
        assert!(fs::read_to_string(legacy)
            .unwrap()
            .contains("Use this skill."));
    }
}
