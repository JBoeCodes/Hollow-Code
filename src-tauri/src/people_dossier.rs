//! People dossiers (master plan 2026-10, #3): who a name in a recording
//! means, and what each person's note shows below the person's own text.
//! See `docs/RECORDS.md#people-dossiers-3`.
//!
//! **Identity is never guessed** (Guardrails). A name matches a person only
//! when exactly one `people/` note has it as its title, file name or one of
//! its `aliases:`. A name from an extracted commitment or fact that matches
//! no one becomes a `person_alias` review item: "Is 'Ana' Ana Ruiz?" when
//! notes share its first name, else "Add a page for 'Raj'?" (pages grow by
//! suggestion, the plan's recommendation). Only the person's answer links an
//! alias (frontmatter `aliases`, body untouched) or creates the page.
//!
//! **The generated block** (`records::generated_for`) gains Last contact
//! (the newest kept recording they speak in, or record with them) and Open
//! threads (open tasks and active workspaces naming them). Facts come only
//! from what the person holds: no HR data, nothing inferred about
//! performance or personal life.

use crate::frontmatter::{self, KeyEdit};
use crate::review_queue::{self, NewReviewItem, ReviewItem};
use serde_json::{json, Value};
use std::path::Path;

/// The card's "Add a new page" choice.
pub const NEW_PAGE: &str = "__new__";
/// The review kind for "who is this?" questions.
pub const ALIAS_KIND: &str = "person_alias";
const THREADS_MAX: usize = 5;

/// A note's `aliases:` (a YAML list or comma-separated), trimmed.
pub fn aliases(vault: &Path, person: &str) -> Vec<String> {
    let Ok(raw) = std::fs::read_to_string(vault.join(person)) else {
        return Vec::new();
    };
    if let Some(items) = frontmatter::list_items(&raw, "aliases") {
        return items
            .into_iter()
            .map(|a| frontmatter::unquote(a.trim()))
            .filter(|a| !a.is_empty())
            .collect();
    }
    let (fm, _) = frontmatter::parse_map(&raw);
    fm.get("aliases")
        .map(|v| {
            v.trim_matches(|c| c == '[' || c == ']')
                .split(',')
                .map(|a| frontmatter::unquote(a.trim()))
                .filter(|a| !a.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Whether `name` (lowercase) is one of `person`'s aliases.
pub fn matches(vault: &Path, person: &str, name: &str) -> bool {
    aliases(vault, person)
        .iter()
        .any(|a| a.to_lowercase() == name)
}

/// People notes sharing `name`'s first word with their title, file name or
/// an alias: the candidates a question offers. Never an answer.
pub fn candidates(vault: &Path, name: &str) -> Vec<String> {
    // Only a bare first name can be someone's: "Ana Garcia" is not a
    // candidate for Ana Ruiz (review fix).
    let mut words = name.split_whitespace();
    let first = words.next().unwrap_or("").to_lowercase();
    if first.chars().count() < 2 || words.next().is_some() {
        return Vec::new();
    }
    crate::vault_people::list(vault)
        .into_iter()
        .filter(|p| {
            let stem = p
                .path
                .trim_start_matches("people/")
                .trim_end_matches(".md")
                .to_string();
            std::iter::once(p.name.clone())
                .chain(std::iter::once(stem))
                .chain(aliases(vault, &p.path))
                .any(|n| {
                    n.split_whitespace()
                        .next()
                        .is_some_and(|w| w.to_lowercase() == first)
                })
        })
        .map(|p| p.path)
        .collect()
}

fn fingerprint(name: &str) -> String {
    format!("alias:{}", name.trim().to_lowercase())
}

/// A file name for a new person's page, or `None` when the name can't be one.
fn page_for(name: &str) -> Option<String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let ok = !name.is_empty()
        && name.chars().count() <= 80
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '\'' | '.'))
        && !name.starts_with('.');
    ok.then(|| format!("people/{name}.md"))
}

/// Ask the person who `name` is, unless a note already means it, or a
/// question about it exists (sticky: a dismissed one isn't asked again).
/// Returns the review item when one was queued.
pub fn ask_identity(
    vault: &Path,
    name: &str,
    source: &str,
    source_title: &str,
    anchor: &str,
) -> Result<Option<ReviewItem>, String> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty()
        || crate::records::person_for(vault, &name).is_some()
        || page_for(&name).is_none()
    {
        return Ok(None);
    }
    let fp = fingerprint(&name);
    if review_queue::list(vault, Some(ALIAS_KIND))?
        .iter()
        .any(|i| i.fingerprint == fp)
    {
        return Ok(None);
    }
    let candidates = candidates(vault, &name);
    let title = match candidates.as_slice() {
        [] => format!("Add a page for '{name}'?"),
        [one] => format!(
            "Is '{name}' {}?",
            one.trim_start_matches("people/").trim_end_matches(".md")
        ),
        many => format!("Who is '{name}'? {} people could be", many.len()),
    };
    review_queue::enqueue(
        vault,
        NewReviewItem {
            kind: ALIAS_KIND.into(),
            fingerprint: fp,
            payload: json!({
                "title": title,
                "name": name,
                "candidates": candidates,
                "sourceTitle": source_title,
                "anchor": anchor,
            }),
            risk: "local_write".into(),
            source: source.into(),
            source_path: None,
        },
    )
    .map(Some)
}

/// The person's answer (user-only, through `resolve_review_item`): `person`
/// (a `people/` path) links the name as its alias; with no candidates and no
/// `person`, accepting creates the page. Returns the person's path.
pub fn resolve(vault: &Path, item: &ReviewItem, edited: Option<&Value>) -> Result<String, String> {
    if item.kind != ALIAS_KIND {
        return Err(format!("review item {} is not a person question", item.id));
    }
    let name = item
        .payload
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let picked = edited
        .and_then(|e| e.get("person"))
        .and_then(Value::as_str)
        .map(|p| p.trim().trim_start_matches("vault:").to_string())
        .filter(|p| !p.is_empty());
    let candidates: Vec<String> = item
        .payload
        .get("candidates")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let target = match (picked, candidates.as_slice()) {
        // "Add a new page" on the card, even when candidates exist.
        (Some(p), _) if p == NEW_PAGE => Link::Create,
        (Some(p), _) => Link::Alias(p),
        (None, [one]) => Link::Alias(one.clone()),
        (None, []) => Link::Create,
        (None, _) => return Err("Pick which person this is (or reject it).".into()),
    };
    review_queue::claim(vault, &item.id, edited.cloned())?;
    let out = match target {
        Link::Alias(person) => link_alias(vault, &person, &name).map(|_| person),
        Link::Create => create_page(vault, &name),
    };
    match out {
        Ok(person) => {
            review_queue::accept(vault, &item.id)?;
            let _ = crate::records::regenerate_person(vault, &person);
            Ok(person)
        }
        Err(e) => {
            let _ = review_queue::fail(vault, &item.id, &e);
            Err(e)
        }
    }
}

enum Link {
    Alias(String),
    Create,
}

/// Add `name` to `person`'s `aliases:` (frontmatter only; the body stays
/// byte-for-byte). A name another note already means is refused.
pub fn link_alias(vault: &Path, person: &str, name: &str) -> Result<(), String> {
    if !person.starts_with("people/") || !person.ends_with(".md") || !vault.join(person).is_file() {
        return Err(format!("{person} isn't a person's note in people/."));
    }
    if let Some(other) = crate::records::person_for(vault, name).filter(|p| p != person) {
        return Err(format!("'{name}' already means {other}."));
    }
    let mut all = aliases(vault, person);
    if all.iter().any(|a| a.eq_ignore_ascii_case(name)) {
        return Ok(());
    }
    all.push(name.to_string());
    // A YAML list, one alias per line: commas inside a name survive and an
    // Obsidian list property stays a list (review fix).
    let mut lines = vec!["aliases:".to_string()];
    lines.extend(all.iter().map(|a| {
        if frontmatter::needs_quoting(a) {
            format!("  - {}", frontmatter::quote(a))
        } else {
            format!("  - {a}")
        }
    }));
    crate::vault::mutate_note(vault, person, |raw| {
        if frontmatter::parse(raw).pairs.is_empty() {
            return Ok(format!("---\n{}\n---\n{raw}", lines.join("\n")));
        }
        frontmatter::patch(raw, &[("aliases", KeyEdit::Block(lines.clone()))], None)
    })?;
    Ok(())
}

fn create_page(vault: &Path, name: &str) -> Result<String, String> {
    let rel = page_for(name).ok_or_else(|| format!("'{name}' can't be a page name."))?;
    let mut lines = vec!["---".to_string()];
    frontmatter::push_kv(&mut lines, "title", name);
    lines.push("---".into());
    crate::vault::create_note(vault, &rel, &format!("{}\n\n# {name}\n", lines.join("\n")))?;
    Ok(rel)
}

// ── The dossier's extra sections ──────────────────────────────────────────

/// Kept recording transcripts (`workspaces/*/sources/recording-*.md`) where
/// one of `names` speaks: (date kept, path), newest first.
fn recordings_with(vault: &Path, names: &[String]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Ok(workspaces) = std::fs::read_dir(vault.join("workspaces")) else {
        return out;
    };
    for ws in workspaces.flatten() {
        let Ok(files) = std::fs::read_dir(ws.path().join("sources")) else {
            continue;
        };
        for f in files.flatten() {
            let file = f.file_name().to_string_lossy().to_string();
            if !file.starts_with("recording-") || !file.ends_with(".md") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(f.path()) else {
                continue;
            };
            let speaks = crate::recordings::parse_transcript(&raw)
                .unwrap_or_default()
                .iter()
                .filter_map(|s| s.speaker.as_deref())
                .any(|s| names.iter().any(|n| n.eq_ignore_ascii_case(s.trim())));
            if !speaks {
                continue;
            }
            let (fm, _) = frontmatter::parse_map(&raw);
            let date = fm
                .get("retrieved_at")
                .map(|d| d.chars().take(10).collect::<String>())
                .unwrap_or_default();
            out.push((
                date,
                format!(
                    "workspaces/{}/sources/{file}",
                    ws.file_name().to_string_lossy()
                ),
            ));
        }
    }
    out.sort_by(|a, b| b.cmp(a));
    out
}

/// The person's names: note title, file name and aliases.
fn names_of(vault: &Path, person: &str, name: &str) -> Vec<String> {
    let stem = person
        .trim_start_matches("people/")
        .trim_end_matches(".md")
        .to_string();
    let mut out = vec![name.to_string()];
    if !stem.eq_ignore_ascii_case(name) {
        out.push(stem);
    }
    out.extend(aliases(vault, person));
    out
}

/// "Last contact" and "Open threads" for `records::generated_for`.
pub fn extra_sections(vault: &Path, person: &str, name: &str) -> Vec<String> {
    let names = names_of(vault, person, name);
    let lower: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    let link = |target: &str| crate::records::link(person, target);
    // Last contact: the newest kept recording they speak in, or the newest
    // record with them.
    let mut contacts: Vec<(String, String, String)> = recordings_with(vault, &names)
        .into_iter()
        .map(|(date, path)| (date, "recording kept".to_string(), path))
        .collect();
    for c in crate::vault_commitments::list(vault) {
        if c.person_path.as_deref() == Some(person)
            || lower.contains(&c.counterparty.to_lowercase())
        {
            contacts.push((
                c.created.chars().take(10).collect(),
                "commitment".into(),
                c.vault_path.clone(),
            ));
        }
    }
    for f in crate::records::facts(vault)
        .into_iter()
        .filter(|f| f.person == person)
    {
        contacts.push((f.stated_at.clone(), "fact".into(), f.path.clone()));
    }
    contacts.sort_by(|a, b| b.cmp(a));
    let last = match contacts.first() {
        Some((date, what, path)) if !date.is_empty() => {
            format!("### Last contact\n\n{date}: [{what}]({})", link(path))
        }
        _ => "### Last contact\n\nNothing recorded.".into(),
    };
    // Open threads: open tasks and active workspaces that name them.
    let mentions = |text: &str| {
        let text = text.to_lowercase();
        lower.iter().any(|n| {
            text.match_indices(n.as_str()).any(|(i, _)| {
                let before = text[..i].chars().next_back();
                let after = text[i + n.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
        })
    };
    let mut threads: Vec<String> = crate::vault_tasks::list_parsed(vault)
        .into_iter()
        .filter(|t| {
            !crate::vault_tasks::is_terminal_status(&t.status)
                && !crate::vault_tasks::is_cancelled_status(&t.status)
        })
        .filter(|t| mentions(&t.title))
        .take(THREADS_MAX)
        .map(|t| {
            format!(
                "- Task: [{}]({})",
                t.title,
                link(t.url.trim_start_matches("vault:"))
            )
        })
        .collect();
    for ws in crate::workspaces::list(vault).unwrap_or_default() {
        if threads.len() >= THREADS_MAX * 2
            || ws.status != "active"
            || crate::workspaces::is_main_agent(&ws.id)
        {
            continue;
        }
        let brief =
            std::fs::read_to_string(vault.join(&ws.path).join("WORKSPACE.md")).unwrap_or_default();
        if mentions(&brief) {
            threads.push(format!(
                "- Workspace: [{}]({})",
                ws.title,
                link(&format!("{}/WORKSPACE.md", ws.path))
            ));
        }
    }
    let open = if threads.is_empty() {
        "### Open threads\n\nNothing recorded.".to_string()
    } else {
        format!("### Open threads\n\n{}", threads.join("\n"))
    };
    vec![last, open]
}

/// Regenerate every person's block; how many changed.
pub fn refresh_all(vault: &Path) -> usize {
    crate::vault_people::list(vault)
        .into_iter()
        .filter(|p| crate::records::regenerate_person(vault, &p.path).unwrap_or(false))
        .count()
}

/// Regenerate every person's generated block (only those blocks change).
#[tauri::command]
pub fn records_refresh_people(state: tauri::State<'_, crate::commands::AppState>) -> usize {
    refresh_all(&state.config.vault_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extract::tests::{block_on, source, vault_with_transcript, Fake, ANSWER};
    use crate::records::tests::person;

    #[test]
    fn a_name_is_never_guessed_and_the_answer_links_an_alias() {
        let dir = vault_with_transcript();
        let v = dir.path();
        let ruiz = person(v, "Ana Ruiz", "## My notes\nLikes tables.\n");
        let body_before = std::fs::read_to_string(v.join(&ruiz)).unwrap();
        // "Ana" isn't exactly anyone: a question, not a match.
        assert!(crate::records::person_for(v, "Ana").is_none());
        let q = ask_identity(v, "Ana", "meeting", "Ops sync", "rec.md@00:05")
            .unwrap()
            .unwrap();
        assert_eq!(q.payload["title"], "Is 'Ana' Ana Ruiz?");
        assert!(
            ask_identity(v, "Ana", "meeting", "Ops sync", "x")
                .unwrap()
                .is_none(),
            "asked once"
        );
        resolve(v, &q, None).unwrap();
        assert_eq!(
            crate::records::person_for(v, "ana").as_deref(),
            Some(ruiz.as_str())
        );
        let after = std::fs::read_to_string(v.join(&ruiz)).unwrap();
        let body = |t: &str| {
            t.split_once("\n---\n")
                .map(|(_, b)| b.to_string())
                .unwrap_or_default()
        };
        assert!(
            body(&after).starts_with(&body(&body_before)),
            "body untouched: {after}"
        );
        // Two candidates: the person must pick.
        person(v, "Raj Patel", "");
        person(v, "Raj Singh", "");
        let q = ask_identity(v, "Raj", "meeting", "Ops sync", "x")
            .unwrap()
            .unwrap();
        assert!(resolve(v, &q, None)
            .unwrap_err()
            .contains("Pick which person"));
        let picked = json!({"person": "people/Raj Singh.md"});
        assert_eq!(
            resolve(v, &q, Some(&picked)).unwrap(),
            "people/Raj Singh.md"
        );
        // No candidate: accepting adds a page.
        let q = ask_identity(v, "Dana Wu", "meeting", "Ops sync", "x")
            .unwrap()
            .unwrap();
        assert_eq!(q.payload["title"], "Add a page for 'Dana Wu'?");
        assert_eq!(resolve(v, &q, None).unwrap(), "people/Dana Wu.md");
        assert!(v.join("people/Dana Wu.md").is_file());
        // Review fixes: a full name isn't asked as someone else; "add a new
        // page" works beside candidates; aliases are a list and keep commas.
        let q = ask_identity(v, "Ana Garcia", "meeting", "Ops sync", "x")
            .unwrap()
            .unwrap();
        assert_eq!(q.payload["title"], "Add a page for 'Ana Garcia'?");
        person(v, "Sam Lee", "");
        person(v, "Sam Roy", "");
        let q = ask_identity(v, "Sam", "meeting", "Ops sync", "x")
            .unwrap()
            .unwrap();
        assert_eq!(
            resolve(v, &q, Some(&json!({"person": NEW_PAGE}))).unwrap(),
            "people/Sam.md"
        );
        link_alias(v, &ruiz, "Ruiz, Ana").unwrap();
        assert_eq!(
            aliases(v, &ruiz),
            vec!["Ana".to_string(), "Ruiz, Ana".to_string()]
        );
        let raw = std::fs::read_to_string(v.join(&ruiz)).unwrap();
        assert!(raw.contains("aliases:\n  - Ana\n"), "{raw}");
        // A name another note means can't be linked elsewhere.
        assert!(link_alias(v, &ruiz, "Dana Wu")
            .unwrap_err()
            .contains("already means"));
        assert!(ask_identity(v, "../etc", "meeting", "x", "x")
            .unwrap()
            .is_none());
    }

    #[test]
    fn extraction_asks_about_unmatched_names_and_the_block_shows_contact_and_threads() {
        let dir = vault_with_transcript();
        let v = dir.path();
        std::fs::write(
            v.join("workspaces/ops/WORKSPACE.md"),
            "---\nhollow_workspace: 1\ntitle: Ops\nstatus: active\n---\n# Ops\n\nWorking with Ana on the Q3 numbers.\n",
        )
        .unwrap();
        let ana = person(v, "Ana", "Mine.\n");
        block_on(crate::extract::run(
            v,
            &source(),
            &Fake::new(ANSWER),
            "Alex Morgan",
            chrono::Utc::now(),
        ))
        .unwrap();
        let asked: Vec<String> = review_queue::list(v, Some(ALIAS_KIND))
            .unwrap()
            .into_iter()
            .map(|i| i.payload["name"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(
            asked,
            vec!["Raj".to_string()],
            "Ana has a page; Raj doesn't"
        );
        crate::vault_tasks::create_idempotent(
            v,
            crate::vault_tasks::TaskFrontmatter {
                title: "Send Ana the deck".into(),
                status: "Not started".into(),
                ..Default::default()
            },
            "",
            true,
            crate::vault_tasks::ExistingPolicy::Return,
        )
        .unwrap();
        refresh_all(v);
        let text = std::fs::read_to_string(v.join(&ana)).unwrap();
        assert!(
            text.starts_with("---\ntitle: Ana\nrole: Ops\n---\nMine.\n"),
            "{text}"
        );
        assert!(text.contains("### Last contact"), "{text}");
        assert!(
            text.contains("### Last contact\n\n2026-10-06: [recording kept](<../workspaces/ops/sources/recording-ops-sync-abc.md>)"),
            "{text}"
        );
        assert!(text.contains("- Task: [Send Ana the deck]"), "{text}");
        assert!(text.contains("- Workspace: [Ops]"), "{text}");
        assert!(!text.contains("Banana"), "word boundaries");
    }
}
