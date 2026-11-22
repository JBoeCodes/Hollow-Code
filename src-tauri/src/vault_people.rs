//! People — the vault `people/` stakeholder CRM, surfaced as a first-class
//! tab. Reads `people/*.md` (skipping `_index.md`), pulling each person's
//! frontmatter snapshot (role, relationship, description). The working-notes
//! body is not summarized here — it is rendered on demand via the existing
//! `vault_note` command; the People page also runs a vault search on the
//! person's name to surface recent mentions across emails/meetings/chat.

use crate::frontmatter;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    /// Display name — frontmatter `title`, else the file stem.
    pub name: String,
    pub role: String,
    pub relationship: String,
    pub description: String,
    /// Vault-relative path, e.g. "people/Jordan Lee.md".
    pub path: String,
}

/// List every promoted person file under `people/`, sorted by name. Returns
/// an empty vec if the folder is absent.
pub fn list(vault: &Path) -> Vec<Person> {
    let dir = vault.join("people");
    let mut people = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return people;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let fname = match path.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };
        // Skip the directory index + any dotfiles.
        if fname.eq_ignore_ascii_case("_index.md") || fname.starts_with('.') {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let fm = frontmatter::parse(&raw);
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&fname)
            .to_string();
        let name = match fm.get("title") {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => stem,
        };
        people.push(Person {
            name,
            role: fm.get("role").unwrap_or("").to_string(),
            relationship: fm.get("relationship").unwrap_or("").to_string(),
            description: fm.get("description").unwrap_or("").to_string(),
            path: format!("people/{fname}"),
        });
    }
    people.sort_by_key(|a| a.name.to_lowercase());
    people
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn lists_people_sorted_skipping_index() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let people = dir.join("people");
        fs::create_dir_all(&people).unwrap();
        fs::write(
            people.join("Jordan Lee.md"),
            "---\ntitle: Jordan Lee\nrole: SVP Technology\nrelationship: Direct manager\n---\n## Snapshot\nStrategic thinker.",
        )
        .unwrap();
        fs::write(
            people.join("Pat Rivera.md"),
            "---\ntitle: Pat Rivera\nrole: CEO\n---\nExecutive champion.",
        )
        .unwrap();
        fs::write(people.join("_index.md"), "directory").unwrap();

        let out = list(&dir);
        assert_eq!(out.len(), 2, "index excluded");
        assert_eq!(out[0].name, "Jordan Lee", "sorted by name");
        assert_eq!(out[0].role, "SVP Technology");
        assert_eq!(out[0].relationship, "Direct manager");
        assert_eq!(out[0].path, "people/Jordan Lee.md");
        assert_eq!(out[1].name, "Pat Rivera");
        assert_eq!(out[1].role, "CEO");
    }

    #[test]
    fn empty_when_no_people_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        fs::create_dir_all(&dir).unwrap();
        assert!(list(&dir).is_empty());
    }
}
