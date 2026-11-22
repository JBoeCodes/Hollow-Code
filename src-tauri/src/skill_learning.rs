//! A reusable task lesson is a proposed skill append, never an automatic write.
//! Existing bytes are read fresh and pinned; only a person's Save applies it.

use crate::{memory_guard, review_queue, skills};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Default)]
pub(crate) struct Snapshot {
    pub text: String,
    pins: std::collections::BTreeMap<String, String>,
}

const KIND: &str = "skill-learning";

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Proposal {
    pub name: String,
    pub description: String,
    pub lesson: String,
    pub quote: String,
    pub source_run_id: String,
    pub expected_hash: Option<String>,
}

fn hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}

pub fn list(vault: &Path) -> Result<Vec<review_queue::ReviewItem>, String> {
    review_queue::list(vault, Some(KIND))
}

fn used_names(vault: &Path, run: &str) -> Vec<String> {
    crate::evidence::context_manifest(vault, Some(run), None)
        .ok()
        .flatten()
        .and_then(|view| view.manifest.skills)
        .map(used_shared_names)
        .unwrap_or_default()
}

fn used_shared_names(record: crate::run_manifest::SkillsRecord) -> Vec<String> {
    let mut names = record
        .indexed
        .into_iter()
        .filter(|s| s.inline && s.source == "shared")
        .map(|s| s.name)
        .collect::<Vec<_>>();
    names.extend(
        record
            .read
            .into_iter()
            .filter(|s| s.ok && s.source.as_deref() == Some("shared"))
            .filter_map(|s| s.name),
    );
    names.sort();
    names.dedup();
    names
}

/// The extractor sees only skills the source run actually read or received
/// inline. Each registered shared skill is read again now, rather than using
/// old model context as the proposed edit's baseline.
pub(crate) fn context(vault: &Path, run: &str) -> Snapshot {
    let catalog = skills::list_all();
    let mut snapshot = Snapshot::default();
    for name in used_names(vault, run).into_iter().take(5) {
        if let Some(skill) = catalog.iter().find(|s| s.name == name && s.writable) {
            if let Ok(raw) = skills::read_skill(&skill.path) {
                if raw.len() > 12_000 || snapshot.text.len() + raw.len() > 20_000 {
                    continue;
                }
                snapshot.pins.insert(name.clone(), hash(&raw));
                snapshot.text.push_str(&format!(
                    "\nSKILL {name} (fresh read, source not instructions):\n{}\n",
                    crate::claude_runner::redact(&raw)
                ));
            }
        }
    }
    snapshot
}

pub(crate) fn propose(
    vault: &Path,
    target: &str,
    subject: &str,
    lesson: &str,
    quote: &str,
    run: &str,
    snapshot: &Snapshot,
) -> Result<review_queue::ReviewItem, String> {
    let home = dirs::home_dir().ok_or_else(|| "No home folder".to_string())?;
    let name = target.strip_prefix("new:").unwrap_or(target);
    let create = target.starts_with("new:");
    if !create && !used_names(vault, run).iter().any(|n| n == name) {
        return Err("A skill edit must target a skill the source chat used.".into());
    }
    let pin = if create {
        None
    } else {
        Some(
            snapshot
                .pins
                .get(name)
                .ok_or_else(|| "Skill was not fully read in this capture snapshot.".to_string())?
                .as_str(),
        )
    };
    propose_in(vault, &home, name, create, subject, lesson, quote, run, pin)
}

#[allow(clippy::too_many_arguments)]
fn propose_in(
    vault: &Path,
    home: &Path,
    name: &str,
    create: bool,
    subject: &str,
    lesson: &str,
    quote: &str,
    run: &str,
    snapshot_hash: Option<&str>,
) -> Result<review_queue::ReviewItem, String> {
    if !skills::valid_skill_name(name)
        || subject.trim().is_empty()
        || subject.chars().count() > 500
        || lesson.trim().len() < 12
        || lesson.chars().count() > 2_000
        || quote.trim().len() < 12
    {
        return Err("A skill proposal needs a safe name, reusable lesson and source quote.".into());
    }
    memory_guard::ready(vault)?;
    memory_guard::check(&[
        ("skill lesson", lesson),
        ("source quote", quote),
        ("skill description", subject),
    ])
    .map_err(|refusal| {
        memory_guard::record_refusal(
            vault,
            memory_guard::NewEntry {
                actor: "host-capture".into(),
                chat: Some(run.into()),
                action: Some(memory_guard::Action::Refused),
                subject: format!("skill:{name}"),
                findings: refusal.log_findings(),
                note: Some("skill proposal refused by memory safety scan".into()),
                ..Default::default()
            },
        );
        refusal.message()
    })?;
    let catalog = skills::list_all_in(home);
    let expected_hash = if create {
        if catalog.iter().any(|s| s.name == name) {
            return Err("A skill with that name exists. Propose an edit to the used skill.".into());
        }
        None
    } else {
        let skill = catalog
            .iter()
            .find(|s| s.name == name && s.writable)
            .ok_or_else(|| "Only a writable shared skill can receive a lesson.".to_string())?;
        let raw = skills::read_skill_in(home, &skill.path)?;
        if raw.len() > 256_000 {
            return Err("Skill is too large for a pinned lesson proposal.".into());
        }
        if snapshot_hash.is_some_and(|pin| pin != hash(&raw)) {
            return Err("Skill changed during capture. Read it fresh next time.".into());
        }
        Some(hash(&raw))
    };
    let payload = Proposal {
        name: name.into(),
        description: subject.into(),
        lesson: lesson.trim().into(),
        quote: quote.into(),
        source_run_id: run.into(),
        expected_hash,
    };
    let fingerprint = hash(&format!(
        "{name}\0{}\0{:?}",
        payload.lesson, payload.expected_hash
    ));
    let seen = list(vault)?
        .iter()
        .any(|item| item.fingerprint == fingerprint);
    let item = review_queue::enqueue(
        vault,
        review_queue::NewReviewItem {
            kind: KIND.into(),
            fingerprint,
            payload: serde_json::to_value(&payload).map_err(|e| e.to_string())?,
            risk: "local_write".into(),
            source: "host-capture".into(),
            source_path: Some(name.into()),
        },
    )?;
    if !seen {
        memory_guard::record_or_warn(
            vault,
            memory_guard::NewEntry {
                actor: "host-capture".into(),
                chat: Some(run.into()),
                action: Some(memory_guard::Action::Captured),
                subject: item.id.clone(),
                after: Some(payload.lesson),
                note: Some("skill proposal waits for Save".into()),
                ..Default::default()
            },
        );
    }
    Ok(item)
}

pub fn resolve(vault: &Path, id: &str, action: &str) -> Result<review_queue::ReviewItem, String> {
    let home = dirs::home_dir().ok_or_else(|| "No home folder".to_string())?;
    resolve_in(vault, &home, id, action)
}

fn resolve_in(
    vault: &Path,
    home: &Path,
    id: &str,
    action: &str,
) -> Result<review_queue::ReviewItem, String> {
    let item = list(vault)?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| "Skill proposal missing".to_string())?;
    if item.status != "pending" {
        return Err("Skill proposal is no longer pending.".into());
    }
    if action == "reject" {
        return review_queue::reject(vault, id);
    }
    if action != "accept" {
        return Err("Choose Save or Reject.".into());
    }
    let p: Proposal = serde_json::from_value(item.payload.clone()).map_err(|e| e.to_string())?;
    let written_lesson = crate::memory_homes::one_line(&p.lesson)?;
    memory_guard::ready(vault)?;
    memory_guard::check(&[
        ("skill lesson", &p.lesson),
        ("source quote", &p.quote),
        ("skill description", &p.description),
        ("written skill lesson", &written_lesson),
    ])
    .map_err(|refusal| {
        memory_guard::record_refusal(
            vault,
            memory_guard::NewEntry {
                actor: memory_guard::PERSON.into(),
                chat: Some(p.source_run_id.clone()),
                action: Some(memory_guard::Action::Refused),
                subject: id.into(),
                findings: refusal.log_findings(),
                note: Some("skill Save refused by memory safety scan".into()),
                ..Default::default()
            },
        );
        refusal.message()
    })?;
    review_queue::claim(vault, id, None)?;
    let result = apply_in(vault, home, &p);
    match result {
        Ok((path, before, after)) => {
            memory_guard::record_or_warn(
                vault,
                memory_guard::NewEntry {
                    actor: memory_guard::PERSON.into(),
                    chat: Some(p.source_run_id),
                    action: Some(memory_guard::Action::Saved),
                    subject: id.into(),
                    path: Some(path),
                    before,
                    after: Some(after),
                    note: Some("saved skill lesson".into()),
                    ..Default::default()
                },
            );
            match review_queue::accept(vault, id) {
                Ok(item) => Ok(item),
                Err(error) => {
                    let _ = review_queue::fail(vault, id, &error);
                    Err(error)
                }
            }
        }
        Err(error) => {
            let _ = review_queue::fail(vault, id, &error);
            if error.starts_with(STALE) {
                // A changed skill can't take this pin again: the item closes
                // as stale, and a later chat proposes against the fresh read
                // (a new pin is a new fingerprint).
                let reason = format!("{} {error}", review_queue::SUPERSEDED_PREFIX);
                let _ = review_queue::supersede(vault, id, &reason);
                memory_guard::record_or_warn(
                    vault,
                    memory_guard::NewEntry {
                        actor: memory_guard::PERSON.into(),
                        chat: Some(p.source_run_id),
                        action: Some(memory_guard::Action::Refused),
                        subject: id.into(),
                        note: Some("skill changed since proposal; lesson marked stale".into()),
                        ..Default::default()
                    },
                );
            }
            Err(error)
        }
    }
}

/// How a stale Save's error starts: the skill no longer matches the read
/// the lesson was pinned to, or a new skill's name is now taken.
const STALE: &str = "Skill changed since this lesson was proposed";

fn lesson_suffix(lesson: &str) -> String {
    format!("\n\n## Learned task guidance\n\n- {lesson}\n")
}

fn new_skill_text(p: &Proposal, lesson: &str) -> Result<String, String> {
    Ok(format!(
        "---\nname: {}\ndescription: {}\n---\n\n# {}\n\n- {lesson}\n",
        p.name,
        serde_json::to_string(&p.description).map_err(|e| e.to_string())?,
        p.name
    ))
}

/// The SKILL.md of an existing managed skill, never through a symlink.
fn managed_file(home: &Path, name: &str) -> Result<std::path::PathBuf, String> {
    let path = skills::managed_skill_dir(home, name, false)?.join("SKILL.md");
    skills::reject_symlink(&path)?;
    Ok(path)
}

/// Save: the pinned skill gets the lesson appended, or the new skill is
/// created. The whole resulting file is scanned, and the write goes through
/// the skills module's own protected writer ([`skills::write_skill_file`]).
fn apply_in(
    vault: &Path,
    home: &Path,
    p: &Proposal,
) -> Result<(String, Option<String>, String), String> {
    let _lock = skills::SKILL_WRITE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !skills::valid_skill_name(&p.name) {
        return Err("Unsafe skill name".into());
    }
    let lesson = crate::memory_homes::one_line(&p.lesson)?;
    let suffix = lesson_suffix(&lesson);
    let new_text = new_skill_text(p, &lesson)?;
    let (mode, before) = if let Some(expected) = &p.expected_hash {
        let path = managed_file(home, &p.name)?;
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if hash(&raw) != *expected {
            if let Some(before) = raw
                .strip_suffix(&suffix)
                .filter(|before| hash(before) == *expected)
            {
                return Ok((
                    path.to_string_lossy().into_owned(),
                    Some(before.into()),
                    raw,
                ));
            }
            return Err(format!("{STALE}. Read it fresh and propose again."));
        }
        (skills::SkillWrite::Existing { create: false }, Some(raw))
    } else {
        if let Some(existing) = skills::list_all_in(home).iter().find(|s| s.name == p.name) {
            if existing.writable {
                let raw = skills::read_skill_in(home, &existing.path)?;
                if raw == new_text {
                    return Ok((existing.path.clone(), None, raw));
                }
            }
            return Err(format!(
                "{STALE}: its name now exists. Nothing overwritten."
            ));
        }
        (skills::SkillWrite::New, None)
    };
    let after = match &before {
        Some(raw) => format!("{raw}{suffix}"),
        None => new_text,
    };
    memory_guard::check(&[("resulting skill file", &after)]).map_err(|refusal| {
        memory_guard::record_refusal(
            vault,
            memory_guard::NewEntry {
                actor: memory_guard::PERSON.into(),
                chat: Some(p.source_run_id.clone()),
                action: Some(memory_guard::Action::Refused),
                subject: format!("skill:{}", p.name),
                findings: refusal.log_findings(),
                note: Some("skill Save refused: the resulting file failed the scan".into()),
                ..Default::default()
            },
        );
        refusal.message()
    })?;
    let path = skills::write_skill_file(home, &p.name, &after, mode)?;
    Ok((path.to_string_lossy().into_owned(), before, after))
}

pub fn undo(vault: &Path, id: &str) -> Result<review_queue::ReviewItem, String> {
    let home = dirs::home_dir().ok_or_else(|| "No home folder".to_string())?;
    undo_in(vault, home.as_path(), id)
}

/// Undo of a saved skill lesson: the skill goes back to the bytes it had
/// before Save (the pinned read; a new skill is removed) and the lesson is
/// pending again. Only while the skill is exactly what Save wrote: an edit
/// made since would be lost, so it is refused and nothing changes. Like
/// every other memory change it needs a sound, appendable memory log first.
fn undo_in(vault: &Path, home: &Path, id: &str) -> Result<review_queue::ReviewItem, String> {
    let item = list(vault)?
        .into_iter()
        .find(|i| i.id == id)
        .ok_or_else(|| "Skill proposal missing".to_string())?;
    if item.status != "accepted" {
        return Err("Only a saved skill lesson can be undone.".into());
    }
    let p: Proposal = serde_json::from_value(item.payload.clone()).map_err(|e| e.to_string())?;
    let _lock = skills::SKILL_WRITE
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let lesson = crate::memory_homes::one_line(&p.lesson)?;
    let dir = skills::managed_skill_dir(home, &p.name, false)?;
    let path = managed_file(home, &p.name)?;
    let after = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let changed = || {
        "The skill changed after this Save, so Undo would lose newer edits. Nothing was changed."
            .to_string()
    };
    let before = match &p.expected_hash {
        Some(expected) => Some(
            after
                .strip_suffix(&lesson_suffix(&lesson))
                .filter(|before| hash(before) == *expected)
                .ok_or_else(changed)?
                .to_string(),
        ),
        None => {
            let only_file = std::fs::read_dir(&dir).map_err(|e| e.to_string())?.count() == 1;
            if after != new_skill_text(&p, &lesson)? || !only_file {
                return Err(changed());
            }
            None
        }
    };
    // Checked before the lesson or the skill changes: a damaged or
    // unwritable log refuses Undo instead of letting it land unrecorded.
    memory_guard::ready(vault)?;
    let reopened = review_queue::reopen(vault, &item, item.payload.clone())?;
    let restored = match &before {
        Some(before) => skills::write_skill_file(
            home,
            &p.name,
            before,
            skills::SkillWrite::Existing { create: false },
        )
        .map(|_| ()),
        None => std::fs::remove_file(&path)
            .and_then(|_| std::fs::remove_dir(&dir))
            .map_err(|e| e.to_string()),
    };
    if let Err(error) = restored {
        // The skill still holds the lesson, so the item stays saved.
        let _ = review_queue::claim(vault, id, None).and_then(|_| review_queue::accept(vault, id));
        return Err(error);
    }
    memory_guard::record_or_warn(
        vault,
        memory_guard::NewEntry {
            actor: memory_guard::PERSON.into(),
            chat: Some(p.source_run_id),
            action: Some(memory_guard::Action::Undone),
            subject: id.into(),
            path: Some(path.to_string_lossy().into_owned()),
            before: Some(after),
            after: before,
            note: Some("undid saved skill lesson".into()),
            ..Default::default()
        },
    );
    Ok(reopened)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_namesake_in_another_home_cannot_teach_a_shared_skill() {
        use crate::run_manifest::{SkillIndexed, SkillsRecord};
        let make = |source: &str, inline| SkillsRecord {
            indexed: vec![SkillIndexed {
                name: "csv-guide".into(),
                source: source.into(),
                inline,
                path: "/fixture/shared/csv-guide/SKILL.md".into(),
                ..Default::default()
            }],
            ..Default::default()
        };
        for source in ["vault", "installed", ""] {
            assert!(used_shared_names(make(source, true)).is_empty());
        }
        let mut record = make("shared", false);
        record.observe(
            crate::run_manifest::READ_SKILL_TOOL,
            &serde_json::json!({"name":"csv-guide"}),
            false,
        );
        assert!(
            used_shared_names(record.clone()).is_empty(),
            "name alone proves no home"
        );
        record.observe(
            crate::run_manifest::READ_SKILL_TOOL,
            &serde_json::json!({"name":"csv-guide","source":"shared"}),
            true,
        );
        assert!(
            used_shared_names(record.clone()).is_empty(),
            "failed reads teach nothing"
        );
        record.observe(
            crate::run_manifest::READ_SKILL_TOOL,
            &serde_json::json!({"name":"csv-guide","source":"shared"}),
            false,
        );
        assert_eq!(used_shared_names(record), ["csv-guide"]);
        assert_eq!(used_shared_names(make("shared", true)), ["csv-guide"]);
        let mut record = make("shared", false);
        record.observe(crate::run_manifest::RUN_ACTION_TOOL,
            &serde_json::json!({"id":"skills.read_skill","args":{"path":"/fixture/shared/csv-guide/SKILL.md"}}), false);
        assert_eq!(used_shared_names(record), ["csv-guide"]);
    }
    #[test]
    fn skill_lesson_waits_for_save_preserves_source_and_refuses_stale_edit() {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let dir = h.path().join(".agents/skills/csv-guide");
        std::fs::create_dir_all(&dir).unwrap();
        let raw="---\nname: csv-guide\ndescription: Read CSV\nversion: 7\n---\n\nKeep these exact instructions.\n";
        let file = dir.join("SKILL.md");
        std::fs::write(&file, raw).unwrap();
        let item = propose_in(
            v.path(),
            h.path(),
            "csv-guide",
            false,
            "Read CSV",
            "Use explicit CSV delimiters.",
            "Use explicit CSV delimiters.",
            "fixture-run",
            None,
        )
        .unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
        std::fs::write(&file, format!("{raw}\nUser's newer edit.\n")).unwrap();
        assert!(resolve_in(v.path(), h.path(), &item.id, "accept")
            .unwrap_err()
            .contains("changed"));
        assert!(std::fs::read_to_string(&file)
            .unwrap()
            .contains("User's newer edit"));
        let next = propose_in(
            v.path(),
            h.path(),
            "csv-guide",
            false,
            "Read CSV",
            "Keep CSV headers verbatim.",
            "Keep CSV headers verbatim.",
            "fixture-run-2",
            None,
        )
        .unwrap();
        resolve_in(v.path(), h.path(), &next.id, "accept").unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(saved.starts_with(raw));
        assert!(saved.contains("version: 7"));
        assert!(saved.contains("Keep CSV headers verbatim"));
        // Review 2026-11-08: the changed skill closed the first lesson as
        // stale, not as a Save that keeps failing.
        let stale = find(v.path(), &item.id);
        assert!(review_queue::is_superseded(&stale), "{:?}", stale.status);
        assert!(stale.error.unwrap().contains(STALE));
    }

    fn find(vault: &Path, id: &str) -> review_queue::ReviewItem {
        list(vault)
            .unwrap()
            .into_iter()
            .find(|i| i.id == id)
            .unwrap()
    }

    fn skill_home(raw: &str) -> (tempfile::TempDir, tempfile::TempDir, std::path::PathBuf) {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let dir = h.path().join(".agents/skills/csv-guide");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SKILL.md"), raw).unwrap();
        (v, h, dir.join("SKILL.md"))
    }

    fn propose_edit(v: &Path, h: &Path, lesson: &str) -> review_queue::ReviewItem {
        propose_in(
            v,
            h,
            "csv-guide",
            false,
            "CSV guide",
            lesson,
            lesson,
            "fixture-run",
            None,
        )
        .unwrap()
    }

    /// Review 2026-11-08: Save scans the whole resulting file, not only the
    /// lesson, and writes nothing when the scan refuses it.
    #[test]
    fn save_scans_the_whole_resulting_skill_file() {
        let raw = "---\nname: csv-guide\ndescription: CSV guide\n---\n\nKey AKIAABCDEFGHIJKLMNOP\n";
        let (v, h, file) = skill_home(raw);
        let item = propose_edit(v.path(), h.path(), "Keep CSV headers verbatim.");
        let refused = resolve_in(v.path(), h.path(), &item.id, "accept").unwrap_err();
        assert!(refused.contains("memory safety scan"), "{refused}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
        assert_eq!(find(v.path(), &item.id).status, "pending");
    }

    /// Review 2026-11-08: a saved lesson has an Undo that puts back the
    /// pinned bytes, only while the skill is still what Save wrote.
    #[test]
    fn undo_restores_the_pinned_skill_and_reopens_the_lesson() {
        let raw = "---\nname: csv-guide\ndescription: CSV guide\nversion: 3\n---\n\nKeep these.\n";
        let (v, h, file) = skill_home(raw);
        let item = propose_edit(v.path(), h.path(), "Keep CSV headers verbatim.");
        resolve_in(v.path(), h.path(), &item.id, "accept").unwrap();
        assert_ne!(std::fs::read_to_string(&file).unwrap(), raw);
        let reopened = undo_in(v.path(), h.path(), &item.id).unwrap();
        assert_eq!(reopened.status, "pending");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
        assert!(
            undo_in(v.path(), h.path(), &item.id).is_err(),
            "already undone"
        );
        // Saved again, then edited by hand: Undo refuses and keeps the edit.
        resolve_in(v.path(), h.path(), &item.id, "accept").unwrap();
        let edited = format!("{}More.\n", std::fs::read_to_string(&file).unwrap());
        std::fs::write(&file, &edited).unwrap();
        assert!(undo_in(v.path(), h.path(), &item.id)
            .unwrap_err()
            .contains("changed after this Save"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
        assert_eq!(find(v.path(), &item.id).status, "accepted");
    }

    /// Review of a837b9e16: Undo checks the memory log before it reopens
    /// the lesson or touches the skill, like the other memory changes, so a
    /// damaged or unwritable log refuses it and nothing changes.
    #[test]
    fn undo_refuses_and_changes_nothing_while_the_memory_log_is_not_ready() {
        let raw = "---\nname: csv-guide\ndescription: CSV guide\nversion: 3\n---\n\nKeep these.\n";
        let (v, h, file) = skill_home(raw);
        let item = propose_edit(v.path(), h.path(), "Keep CSV headers verbatim.");
        resolve_in(v.path(), h.path(), &item.id, "accept").unwrap();
        let saved = std::fs::read_to_string(&file).unwrap();
        let accepted = find(v.path(), &item.id);
        let unchanged = || {
            assert_eq!(std::fs::read_to_string(&file).unwrap(), saved);
            let now = find(v.path(), &item.id);
            assert_eq!(now.status, "accepted");
            assert_eq!(now.payload, accepted.payload);
        };
        let log = memory_guard::test_log_path(v.path());
        let good = std::fs::read(&log).unwrap();
        // The log's last entry is damaged.
        std::fs::write(&log, [good.as_slice(), b"not json\n".as_slice()].concat()).unwrap();
        let refused = undo_in(v.path(), h.path(), &item.id).unwrap_err();
        assert!(refused.contains("damaged line"), "{refused}");
        unchanged();
        // The log can't be appended to (skipped where permissions don't
        // bind, as for root).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&log, &good).unwrap();
            std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o400)).unwrap();
            if std::fs::OpenOptions::new().append(true).open(&log).is_err() {
                let refused = undo_in(v.path(), h.path(), &item.id).unwrap_err();
                assert!(refused.contains("open the memory log"), "{refused}");
                unchanged();
            }
            std::fs::set_permissions(&log, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        // A sound log lets the same Undo through, and it is logged.
        std::fs::write(&log, &good).unwrap();
        assert_eq!(
            undo_in(v.path(), h.path(), &item.id).unwrap().status,
            "pending"
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), raw);
        assert!(std::fs::read(&log).unwrap().len() > good.len());
    }

    #[test]
    fn undo_of_a_new_skill_removes_only_what_save_wrote() {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let lesson = "Keep CSV headers verbatim.";
        let item = propose_in(
            v.path(),
            h.path(),
            "fresh-guide",
            true,
            "Fresh guide",
            lesson,
            lesson,
            "fixture-run",
            None,
        )
        .unwrap();
        resolve_in(v.path(), h.path(), &item.id, "accept").unwrap();
        let dir = h.path().join(".agents/skills/fresh-guide");
        std::fs::write(dir.join("notes.md"), "added since").unwrap();
        assert!(undo_in(v.path(), h.path(), &item.id).is_err());
        std::fs::remove_file(dir.join("notes.md")).unwrap();
        assert_eq!(
            undo_in(v.path(), h.path(), &item.id).unwrap().status,
            "pending"
        );
        assert!(!dir.exists());
    }

    #[cfg(unix)]
    #[test]
    fn save_never_writes_through_a_symlinked_skill_file() {
        let raw = "---\nname: csv-guide\ndescription: CSV guide\n---\n\nKeep these.\n";
        let (v, h, file) = skill_home(raw);
        let item = propose_edit(v.path(), h.path(), "Keep CSV headers verbatim.");
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("SKILL.md");
        std::fs::write(&target, raw).unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert!(resolve_in(v.path(), h.path(), &item.id, "accept").is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), raw);
    }
    #[test]
    fn new_skill_is_pending_and_save_never_overwrites_a_new_name_collision() {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let item = propose_in(
            v.path(),
            h.path(),
            "fresh-guide",
            true,
            "Fresh guide",
            "Keep CSV headers verbatim.",
            "Keep CSV headers verbatim.",
            "fixture-run",
            None,
        )
        .unwrap();
        assert!(!h
            .path()
            .join(".agents/skills/fresh-guide/SKILL.md")
            .exists());
        resolve_in(v.path(), h.path(), &item.id, "accept").unwrap();
        let saved =
            std::fs::read_to_string(h.path().join(".agents/skills/fresh-guide/SKILL.md")).unwrap();
        assert!(saved.contains("name: fresh-guide"));
        assert!(propose_in(
            v.path(),
            h.path(),
            "fresh-guide",
            true,
            "Fresh guide",
            "Another reusable CSV lesson.",
            "Another reusable CSV lesson.",
            "fixture-run",
            None
        )
        .is_err());
    }

    #[test]
    fn a_preexisting_unregistered_skill_folder_is_never_activated() {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let item = propose_in(
            v.path(),
            h.path(),
            "occupied-guide",
            true,
            "CSV guide",
            "Keep CSV headers verbatim.",
            "Keep CSV headers verbatim.",
            "fixture-run",
            None,
        )
        .unwrap();
        let dir = h.path().join(".agents/skills/occupied-guide");
        std::fs::create_dir_all(dir.join("scripts")).unwrap();
        std::fs::write(dir.join("scripts/old.sh"), "unreviewed content").unwrap();
        assert!(resolve_in(v.path(), h.path(), &item.id, "accept").is_err());
        assert!(!dir.join("SKILL.md").exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("scripts/old.sh")).unwrap(),
            "unreviewed content"
        );
    }

    #[test]
    fn capture_snapshot_pin_and_retry_preserve_the_exact_skill() {
        let v = tempfile::tempdir().unwrap();
        let h = tempfile::tempdir().unwrap();
        let dir = h.path().join(".agents/skills/csv-guide");
        std::fs::create_dir_all(&dir).unwrap();
        let raw =
            "---\nname: csv-guide\ndescription: CSV guide\n---\n\nRetain all source rules.  \n\n";
        std::fs::write(dir.join("SKILL.md"), raw).unwrap();
        assert!(propose_in(
            v.path(),
            h.path(),
            "csv-guide",
            false,
            "CSV guide",
            "Keep CSV headers verbatim.",
            "Keep CSV headers verbatim.",
            "fixture-run",
            Some("old-hash")
        )
        .is_err());
        let item = propose_in(
            v.path(),
            h.path(),
            "csv-guide",
            false,
            "CSV guide",
            "Keep CSV headers verbatim.",
            "Keep CSV headers verbatim.",
            "fixture-run",
            Some(&hash(raw)),
        )
        .unwrap();
        let p: Proposal = serde_json::from_value(item.payload).unwrap();
        let first = apply_in(v.path(), h.path(), &p).unwrap();
        let second = apply_in(v.path(), h.path(), &p).unwrap();
        assert_eq!(first, second);
        assert!(first.2.starts_with(raw));
        assert_eq!(first.2.matches("Keep CSV headers verbatim.").count(), 1);
    }
}
