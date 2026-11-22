//! Retention for native install build folders (overnight A5). Each Hollow
//! app build gets its own cargo target, `~/Library/Caches/<hollow>-install-
//! target-<workspace>` (`self_install::target_dir`), and nothing removed
//! them: 13 had piled up by 2026-10-09, several GB each.
//!
//! [`plan`] lists them and says which stay: the folder that built the
//! installed app and the one that built its rollback copy (found through the
//! installer's journals, whose `source` is the installed bundle's path), a
//! running or finished-but-not-installed build's, and the newest `keep`
//! others. [`prune`] deletes the rest, only through the confirm-tier
//! `build.prune_install_targets` action, bound to the exact list its card
//! showed, or the person's Settings, Advanced, Disk space, Delete old
//! install builds… (overnight H20; `prune_install_targets` with the hash of
//! the list its confirm dialog showed). Never automatic. The apps in
//! /Applications are never touched: these folders are rebuildable caches.
//! The same two apps' commits and checkouts also keep the worktrees that
//! built them ([`app_commits`], [`app_checkouts`], read by
//! `worktree_hygiene`).

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many other folders stay when the caller doesn't say.
pub const DEFAULT_KEEP: usize = 2;
/// The most `keep` may ask for.
pub const MAX_KEEP: usize = 20;

/// The everyday app's installer journals (`scripts/install-macos-app.mjs`,
/// `PROFILES.everyday.stateDir`).
const JOURNALS: &str = "com.hollow.app/installer/journal";
const INSTALLED_PROVENANCE: &str =
    "/Applications/Hollow.app/Contents/Resources/build-provenance.json";
const PREVIOUS_PROVENANCE: &str =
    "/Applications/.Hollow.app.previous/Contents/Resources/build-provenance.json";

/// One install build folder and whether it stays.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    pub path: String,
    pub name: String,
    /// When cargo last used it (RFC 3339), if known.
    pub last_used: Option<String>,
    /// Why it stays; `None` means [`prune`] would delete it.
    pub keep: Option<String>,
}

/// What [`prune`] would do now.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Newest first.
    pub targets: Vec<Target>,
    /// The paths [`prune`] deletes, newest first.
    pub remove: Vec<String>,
    pub keep_newest: usize,
    /// Binds a confirm card to exactly `remove`.
    pub hash: String,
    /// What couldn't be told (e.g. no journal names the installed build).
    pub notes: Vec<String>,
    /// Folders an interrupted delete left aside; also in `remove`.
    pub leftovers: Vec<String>,
}

/// What [`prune`] did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Pruned {
    pub removed: Vec<String>,
    /// (path, error) for each folder that couldn't be removed.
    pub failed: Vec<(String, String)>,
    pub kept: usize,
}

/// Everything [`plan_in`] reads, so tests can point it anywhere.
#[derive(Debug, Clone)]
pub struct Facts {
    /// The folder holding the targets (`~/Library/Caches`).
    pub caches: PathBuf,
    /// `<hollow>-install-target-`.
    pub prefix: String,
    pub journals: PathBuf,
    /// The installed and rollback apps' commits, when readable.
    pub installed_commit: Option<String>,
    pub previous_commit: Option<String>,
    /// Targets a build holds, with why (running, or finished and waiting).
    pub held: Vec<(PathBuf, String)>,
}

impl Facts {
    /// This Mac's state.
    pub fn live() -> Result<Facts, String> {
        let caches = dirs::cache_dir().ok_or("No Caches folder")?;
        let journals = dirs::data_dir()
            .ok_or("No Application Support folder")?
            .join(JOURNALS);
        Ok(Facts {
            caches,
            prefix: format!(
                "{}-install-target-",
                crate::instance::current().cache_name()
            ),
            journals,
            installed_commit: provenance_commit(Path::new(INSTALLED_PROVENANCE)),
            previous_commit: provenance_commit(Path::new(PREVIOUS_PROVENANCE)),
            held: crate::self_install::held_targets(),
        })
    }
}

/// The commit a `build-provenance.json` names, when readable.
pub(crate) fn provenance_commit(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let commit = crate::app_menu::parse_identity(&text, "").commit;
    (!commit.is_empty()).then_some(commit)
}

/// The installed app's and its rollback copy's commits, each with what it
/// built (the same words as a kept folder's reason), when readable.
/// `worktree_hygiene` keeps a worktree whose HEAD is one of them
/// (overnight H20). An app that is there but whose provenance can't be read
/// gives [`ANY_COMMIT`]: every worktree is kept rather than guessing, as an
/// unreadable process list keeps everything.
pub(crate) fn app_commits() -> Vec<(String, String)> {
    [
        (INSTALLED_PROVENANCE, "built the installed app"),
        (PREVIOUS_PROVENANCE, "built the rollback copy"),
    ]
    .into_iter()
    .filter_map(|(path, what)| match provenance_commit(Path::new(path)) {
        Some(commit) => Some((commit, what.to_string())),
        // …/Hollow.app/Contents/Resources/build-provenance.json → the bundle.
        None => Path::new(path)
            .ancestors()
            .nth(3)
            .filter(|app| app.exists())
            .map(|_| {
                let which = what.trim_start_matches("built the ");
                (
                    ANY_COMMIT.to_string(),
                    format!("couldn't read which commit built {which}"),
                )
            }),
    })
    .collect()
}

/// The folder a `build-provenance.json` says the app was built in (its
/// `checkout`, `scripts/source-provenance.mjs`), when readable.
pub(crate) fn provenance_checkout(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let checkout = v.get("checkout")?.as_str()?.trim();
    (!checkout.is_empty()).then(|| PathBuf::from(checkout))
}

/// The folders the installed app and its rollback copy were built in, each
/// with what it built (the same words as [`app_commits`]), when readable.
/// `worktree_hygiene` keeps those worktrees too, wherever their HEAD has
/// moved since (overnight S01a: the release checkout moves on to the next
/// release and stops matching the installed commit).
pub(crate) fn app_checkouts() -> Vec<(PathBuf, String)> {
    [
        (INSTALLED_PROVENANCE, "built the installed app"),
        (PREVIOUS_PROVENANCE, "built the rollback copy"),
    ]
    .into_iter()
    .filter_map(|(path, what)| {
        provenance_checkout(Path::new(path)).map(|dir| (dir, what.to_string()))
    })
    .collect()
}

/// Stands for every commit in [`app_commits`]: the app is there, its
/// provenance unreadable.
pub(crate) const ANY_COMMIT: &str = "*";

/// Same commit, allowing one side to be abbreviated (7+ characters).
pub(crate) fn same_commit(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim(), b.trim());
    a.len().min(b.len()) >= 7 && (a.starts_with(b) || b.starts_with(a))
}

/// True when `path` is a folder [`prune_in`] moved aside to delete and
/// didn't finish (Hollow quit mid-delete): `.<prefix><slug>.deleting-<pid>`,
/// a real folder directly in `caches`. Listed for deletion with the rest.
fn is_leftover(caches: &Path, prefix: &str, path: &Path) -> bool {
    let named = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(|n| n.strip_prefix('.'))
        .is_some_and(|n| n.starts_with(prefix) && n.contains(".deleting-"));
    named
        && path.parent() == Some(caches)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// True when `path` is one of the folders this module manages: a real
/// folder (not a link) directly in `caches` named `<prefix><something>`.
pub fn is_install_target(caches: &Path, prefix: &str, path: &Path) -> bool {
    let named = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.len() > prefix.len() && n.starts_with(prefix));
    named
        && path.parent() == Some(caches)
        && std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir())
}

/// When cargo last used `dir`: it rewrites `.rustc_info.json` on every run,
/// and the profile folders change with each build.
fn last_used(dir: &Path) -> Option<SystemTime> {
    [".rustc_info.json", "release", "debug", ""]
        .iter()
        .filter_map(|p| std::fs::metadata(dir.join(p)).ok()?.modified().ok())
        .max()
}

/// Journal states of an install that put its build in /Applications (the
/// installer's `SETTLED` minus `failed` and `undone`).
const INSTALLED_STATES: &[&str] = &["verified", "healthy", "running-mismatch"];

/// (commit, source) of each installer journal whose install went in,
/// newest first by `startedAt` (as the installer's own `listJournals`).
/// A failed or undone install's journal names a folder that built nothing
/// in /Applications, so it never claims the installed app's keep reason.
fn journal_sources(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found: Vec<(String, String, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .filter_map(|p| {
            let v: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            let state = v.get("state")?.as_str()?;
            if !INSTALLED_STATES.contains(&state) {
                return None;
            }
            let started = v.get("startedAt").and_then(|s| s.as_str()).unwrap_or("");
            let commit = v.get("commit")?.as_str()?.to_string();
            let source = PathBuf::from(v.get("source")?.as_str()?);
            Some((started.to_string(), commit, source))
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, c, s)| (c, s)).collect()
}

fn hash_of(remove: &[String]) -> String {
    let mut hash = Sha256::new();
    for path in remove {
        hash.update(path.as_bytes());
        hash.update(b"\n");
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// What [`prune`] would delete given `facts`, keeping `keep_newest` others.
pub fn plan_in(facts: &Facts, keep_newest: usize) -> Plan {
    let keep_newest = keep_newest.min(MAX_KEEP);
    let mut found: Vec<(PathBuf, Option<SystemTime>)> = std::fs::read_dir(&facts.caches)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_install_target(&facts.caches, &facts.prefix, p))
        .map(|p| {
            let used = last_used(&p);
            (p, used)
        })
        .collect();
    // Newest first; unknown times count as newest (never pruned for age first).
    found.sort_by(|a, b| match (a.1, b.1) {
        (Some(x), Some(y)) => y.cmp(&x).then_with(|| a.0.cmp(&b.0)),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => a.0.cmp(&b.0),
    });

    let journals = journal_sources(&facts.journals);
    let mut notes = Vec::new();
    let mut reasons: Vec<Option<String>> = vec![None; found.len()];
    for (commit, what) in [
        (&facts.installed_commit, "built the installed app"),
        (&facts.previous_commit, "built the rollback copy"),
    ] {
        let Some(commit) = commit else { continue };
        let short: String = commit.chars().take(8).collect();
        let source = journals
            .iter()
            .find(|(c, _)| same_commit(c, commit))
            .map(|(_, s)| s);
        match source.and_then(|s| found.iter().position(|(t, _)| s.starts_with(t))) {
            Some(i) => {
                let why = format!("{what} ({short})");
                reasons[i] = Some(match reasons[i].take() {
                    Some(first) => format!("{first}; {why}"),
                    None => why,
                });
            }
            None if source.is_some() => {}
            None => notes.push(format!(
                "No installer journal names the folder that {what} ({short}); none is kept for it."
            )),
        }
    }
    for (held, why) in &facts.held {
        if let Some(i) = found.iter().position(|(t, _)| t == held) {
            reasons[i].get_or_insert_with(|| why.clone());
        }
    }
    let mut newest = 0;
    for reason in reasons.iter_mut() {
        if reason.is_none() && newest < keep_newest {
            newest += 1;
            *reason = Some(format!("one of the newest {keep_newest}"));
        }
    }

    let targets: Vec<Target> = found
        .iter()
        .zip(reasons)
        .map(|((path, used), keep)| Target {
            path: path.to_string_lossy().into_owned(),
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            last_used: used.map(|t| chrono::DateTime::<chrono::Local>::from(t).to_rfc3339()),
            keep,
        })
        .collect();
    let mut leftovers: Vec<String> = std::fs::read_dir(&facts.caches)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_leftover(&facts.caches, &facts.prefix, p))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    leftovers.sort();
    let mut remove: Vec<String> = targets
        .iter()
        .filter(|t| t.keep.is_none())
        .map(|t| t.path.clone())
        .collect();
    remove.extend(leftovers.iter().cloned());
    Plan {
        hash: hash_of(&remove),
        targets,
        remove,
        keep_newest,
        notes,
        leftovers,
    }
}

/// The plan for this Mac.
pub fn plan(keep_newest: usize) -> Result<Plan, String> {
    Ok(plan_in(&Facts::live()?, keep_newest))
}

/// Card lines: every folder that goes, then every one that stays and why.
/// The agent's card; the person's confirm dialog says the same in the
/// panel's words (`src/diskGuard.ts`, `pruneConfirmBody`): change both.
pub fn describe(plan: &Plan) -> Vec<String> {
    let day = |t: &Target| {
        t.last_used
            .as_deref()
            .map(|d| format!(", last used {}", &d[..d.len().min(10)]))
            .unwrap_or_default()
    };
    let mut lines = Vec::new();
    if plan.remove.is_empty() {
        lines.push("Nothing to delete: every install build folder is kept.".to_string());
    } else {
        lines.push(format!(
            "Delete {} install build folder{} in ~/Library/Caches (rebuildable cargo output; the apps in /Applications are not touched):",
            plan.remove.len(),
            if plan.remove.len() == 1 { "" } else { "s" }
        ));
        for t in plan.targets.iter().filter(|t| t.keep.is_none()) {
            lines.push(format!("- {}{}", t.name, day(t)));
        }
        for path in &plan.leftovers {
            let name = Path::new(path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy();
            lines.push(format!("- {name} (left by an interrupted delete)"));
        }
    }
    for t in plan.targets.iter().filter(|t| t.keep.is_some()) {
        lines.push(format!(
            "Kept: {} ({}{})",
            t.name,
            t.keep.as_deref().unwrap_or(""),
            day(t)
        ));
    }
    lines.extend(plan.notes.iter().cloned());
    lines
}

/// Delete what [`plan_in`] lists now. With `expected_hash` (the card's), a
/// list that changed since refuses and deletes nothing.
pub fn prune_in(
    facts: &Facts,
    keep_newest: usize,
    expected_hash: Option<&str>,
    held_now: &dyn Fn() -> Vec<PathBuf>,
) -> Result<Pruned, String> {
    let plan = plan_in(facts, keep_newest);
    if let Some(expected) = expected_hash {
        if expected != plan.hash {
            return Err("The folders to delete changed since the card was shown. Nothing was deleted; ask again to see the new list.".into());
        }
    }
    let mut removed = Vec::new();
    let mut failed = Vec::new();
    for path in &plan.remove {
        let p = Path::new(path);
        if plan.leftovers.contains(path) {
            match std::fs::remove_dir_all(p) {
                Ok(()) => removed.push(path.clone()),
                Err(e) => failed.push((path.clone(), e.to_string())),
            }
            continue;
        }
        if !is_install_target(&facts.caches, &facts.prefix, p) {
            failed.push((
                path.clone(),
                "no longer an install build folder".to_string(),
            ));
            continue;
        }
        // A build may have started since the plan (deleting takes a while).
        if held_now().iter().any(|h| h == p) {
            failed.push((path.clone(), "a build started using it; kept".to_string()));
            continue;
        }
        // Move it aside first, so a build starting from here on gets a fresh
        // folder instead of one being emptied underneath it.
        let name = p.file_name().unwrap_or_default().to_string_lossy();
        let aside = facts
            .caches
            .join(format!(".{name}.deleting-{}", std::process::id()));
        if let Err(e) = std::fs::rename(p, &aside) {
            failed.push((path.clone(), e.to_string()));
            continue;
        }
        match std::fs::remove_dir_all(&aside) {
            Ok(()) => removed.push(path.clone()),
            Err(e) => {
                // Put what's left back where cargo looks, rather than leave a
                // hidden folder nothing lists.
                if !p.exists() {
                    let _ = std::fs::rename(&aside, p);
                }
                failed.push((path.clone(), e.to_string()));
            }
        }
    }
    Ok(Pruned {
        kept: plan.targets.iter().filter(|t| t.keep.is_some()).count(),
        removed,
        failed,
    })
}

/// [`prune_in`] for this Mac.
pub fn prune(keep_newest: usize, expected_hash: Option<&str>) -> Result<Pruned, String> {
    let held_now = || {
        crate::self_install::held_targets()
            .into_iter()
            .map(|(p, _)| p)
            .collect()
    };
    prune_in(&Facts::live()?, keep_newest, expected_hash, &held_now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    const PREFIX: &str = "hollow-install-target-";

    struct Bed {
        _tmp: tempfile::TempDir,
        caches: PathBuf,
        journals: PathBuf,
    }

    fn bed() -> Bed {
        let tmp = tempfile::tempdir().unwrap();
        let caches = tmp.path().join("Caches");
        let journals = tmp.path().join("journal");
        fs::create_dir_all(&caches).unwrap();
        fs::create_dir_all(&journals).unwrap();
        Bed {
            _tmp: tmp,
            caches,
            journals,
        }
    }

    /// A target last used `age` days ago.
    fn target(bed: &Bed, slug: &str, age: u64) -> PathBuf {
        let dir = bed.caches.join(format!("{PREFIX}{slug}"));
        fs::create_dir_all(dir.join("release")).unwrap();
        let info = dir.join(".rustc_info.json");
        fs::write(&info, "{}").unwrap();
        let when = SystemTime::now() - Duration::from_secs(age * 86_400);
        for p in [&info, &dir.join("release"), &dir] {
            set_times(p, when);
        }
        dir
    }

    /// Folders can't be opened for writing, so set times through utimes.
    fn set_times(path: &Path, when: SystemTime) {
        let secs = when
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let c = std::ffi::CString::new(path.to_str().unwrap()).unwrap();
        let times = [
            libc::timeval {
                tv_sec: secs,
                tv_usec: 0,
            },
            libc::timeval {
                tv_sec: secs,
                tv_usec: 0,
            },
        ];
        assert_eq!(unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) }, 0);
    }

    fn journal(bed: &Bed, id: &str, commit: &str, target: &Path) {
        journal_in(bed, id, commit, target, "healthy");
    }

    /// `id` is `YYYYMMDD-HHMMSS-x`; `startedAt` follows it.
    fn journal_in(bed: &Bed, id: &str, commit: &str, target: &Path, state: &str) {
        let source = target.join("release/bundle/macos/Hollow.app");
        let started = format!(
            "{}-{}-{}T{}:{}:{}.000Z",
            &id[0..4],
            &id[4..6],
            &id[6..8],
            &id[9..11],
            &id[11..13],
            &id[13..15]
        );
        fs::write(
            bed.journals.join(format!("{id}.json")),
            serde_json::json!({ "id": id, "commit": commit, "source": source, "state": state, "startedAt": started })
                .to_string(),
        )
        .unwrap();
    }

    fn facts(bed: &Bed) -> Facts {
        Facts {
            caches: bed.caches.clone(),
            prefix: PREFIX.into(),
            journals: bed.journals.clone(),
            installed_commit: None,
            previous_commit: None,
            held: vec![],
        }
    }

    fn kept(plan: &Plan) -> Vec<(String, String)> {
        plan.targets
            .iter()
            .filter_map(|t| {
                Some((
                    t.name.trim_start_matches(PREFIX).to_string(),
                    t.keep.clone()?,
                ))
            })
            .collect()
    }

    fn removed(plan: &Plan) -> Vec<String> {
        plan.targets
            .iter()
            .filter(|t| t.keep.is_none())
            .map(|t| t.name.trim_start_matches(PREFIX).to_string())
            .collect()
    }

    #[test]
    fn keeps_installed_rollback_held_and_newest_and_lists_the_rest() {
        let bed = bed();
        let installed = target(&bed, "installed", 30);
        let previous = target(&bed, "previous", 40);
        let finished = target(&bed, "finished", 50);
        for (slug, age) in [("new-1", 1), ("new-2", 2), ("old-3", 3), ("old-4", 60)] {
            target(&bed, slug, age);
        }
        journal(&bed, "20261001-100000-aaaa", "1111111111111111", &previous);
        journal(&bed, "20261002-100000-bbbb", "2222222222222222", &installed);
        let mut f = facts(&bed);
        f.installed_commit = Some("2222222222222222".into());
        f.previous_commit = Some("1111111".into());
        f.held = vec![(finished, "a finished build waiting to install".into())];

        let plan = plan_in(&f, 2);
        assert_eq!(
            kept(&plan),
            [
                ("new-1".to_string(), "one of the newest 2".to_string()),
                ("new-2".into(), "one of the newest 2".into()),
                (
                    "installed".into(),
                    "built the installed app (22222222)".into()
                ),
                (
                    "previous".into(),
                    "built the rollback copy (1111111)".into()
                ),
                (
                    "finished".into(),
                    "a finished build waiting to install".into()
                ),
            ]
        );
        assert_eq!(removed(&plan), ["old-3", "old-4"]);
        assert!(plan.notes.is_empty(), "{:?}", plan.notes);
        assert_eq!(plan.hash, hash_of(&plan.remove));
        let text = describe(&plan).join("\n");
        assert!(text.contains("Delete 2 install build folders"), "{text}");
        assert!(
            text.contains(&format!("- {PREFIX}old-3, last used ")),
            "{text}"
        );
        assert!(
            text.contains(&format!("Kept: {PREFIX}installed (built the installed app")),
            "{text}"
        );
    }

    #[test]
    fn one_folder_that_built_both_apps_says_so() {
        let bed = bed();
        let both = target(&bed, "both", 5);
        journal(&bed, "20261001-100000-aaaa", "1111111111", &both);
        journal(&bed, "20261002-100000-bbbb", "2222222222", &both);
        let mut f = facts(&bed);
        f.installed_commit = Some("2222222222".into());
        f.previous_commit = Some("1111111111".into());
        assert_eq!(
            kept(&plan_in(&f, 0)),
            [(
                "both".to_string(),
                "built the installed app (22222222); built the rollback copy (11111111)"
                    .to_string()
            )]
        );
    }

    #[test]
    fn the_newest_journal_for_a_commit_names_its_folder() {
        let bed = bed();
        let first = target(&bed, "first", 10);
        let second = target(&bed, "second", 11);
        journal(&bed, "20261001-100000-aaaa", "abcdef1234", &first);
        journal(&bed, "20261005-100000-bbbb", "abcdef1234", &second);
        let mut f = facts(&bed);
        f.installed_commit = Some("abcdef1234".into());
        let plan = plan_in(&f, 0);
        assert_eq!(
            kept(&plan),
            [(
                "second".to_string(),
                "built the installed app (abcdef12)".to_string()
            )]
        );
        assert_eq!(removed(&plan), ["first"]);
    }

    #[test]
    fn an_unknown_installed_build_is_noted_not_guessed() {
        let bed = bed();
        target(&bed, "a", 1);
        let mut f = facts(&bed);
        f.installed_commit = Some("feedfacecafe".into());
        let plan = plan_in(&f, 0);
        assert_eq!(removed(&plan), ["a"]);
        assert_eq!(plan.notes.len(), 1);
        assert!(
            plan.notes[0].contains("installed app (feedface)"),
            "{:?}",
            plan.notes
        );
        // A journal whose folder is already gone is not a gap worth noting.
        journal(
            &bed,
            "20261001-100000-aaaa",
            "feedfacecafe",
            &bed.caches.join(format!("{PREFIX}gone")),
        );
        assert!(plan_in(&f, 0).notes.is_empty());
    }

    #[test]
    fn only_real_prefixed_folders_directly_in_caches_count() {
        let bed = bed();
        target(&bed, "real", 1);
        fs::create_dir_all(bed.caches.join("hollow-build-targets")).unwrap();
        fs::create_dir_all(bed.caches.join("other-install-target-x")).unwrap();
        fs::create_dir_all(bed.caches.join(PREFIX)).unwrap(); // no slug
        fs::write(bed.caches.join(format!("{PREFIX}file")), "x").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(elsewhere.path(), bed.caches.join(format!("{PREFIX}link")))
            .unwrap();
        let plan = plan_in(&facts(&bed), 0);
        assert_eq!(removed(&plan), ["real"]);
        assert!(!is_install_target(
            &bed.caches,
            PREFIX,
            &bed.caches.join(format!("{PREFIX}link"))
        ));
    }

    #[test]
    fn prune_deletes_exactly_the_bound_list_and_refuses_a_changed_one() {
        let bed = bed();
        let keep = target(&bed, "keep", 1);
        let old = target(&bed, "old", 9);
        fs::write(old.join("release/big"), "x").unwrap();
        target(&bed, "old", 9); // the write made it look new again
        let f = facts(&bed);
        let plan = plan_in(&f, 1);
        assert_eq!(removed(&plan), ["old"]);

        // A new folder since the card: the bound list no longer matches.
        let newer = target(&bed, "newer", 0);
        let err = prune_in(&f, 1, Some(&plan.hash), &|| vec![]).unwrap_err();
        assert!(err.contains("Nothing was deleted"), "{err}");
        assert!(old.exists() && keep.exists() && newer.exists());

        fs::remove_dir_all(&newer).unwrap();
        let done = prune_in(&f, 1, Some(&plan.hash), &|| vec![]).unwrap();
        assert_eq!(done.removed, [old.to_string_lossy().into_owned()]);
        assert!(done.failed.is_empty());
        assert_eq!(done.kept, 1);
        assert!(!old.exists() && keep.exists());
    }

    #[test]
    fn a_failed_or_undone_install_never_names_the_installed_folder() {
        let bed = bed();
        let real = target(&bed, "real", 9);
        let failed = target(&bed, "failed", 8);
        let undone = target(&bed, "undone", 7);
        journal(&bed, "20261001-100000-aaaa", "abcdef1234", &real);
        journal_in(
            &bed,
            "20261002-100000-bbbb",
            "abcdef1234",
            &failed,
            "failed",
        );
        journal_in(
            &bed,
            "20261003-100000-cccc",
            "abcdef1234",
            &undone,
            "undone",
        );
        let mut f = facts(&bed);
        f.installed_commit = Some("abcdef1234".into());
        let plan = plan_in(&f, 0);
        assert_eq!(
            kept(&plan),
            [(
                "real".to_string(),
                "built the installed app (abcdef12)".to_string()
            )]
        );
        assert_eq!(removed(&plan), ["undone", "failed"]);
    }

    #[test]
    fn a_build_that_starts_mid_prune_keeps_its_folder() {
        let bed = bed();
        let a = target(&bed, "a", 5);
        let b = target(&bed, "b", 6);
        let f = facts(&bed);
        let plan = plan_in(&f, 0);
        let busy = b.clone();
        let done = prune_in(&f, 0, Some(&plan.hash), &move || vec![busy.clone()]).unwrap();
        assert_eq!(done.removed, [a.to_string_lossy().into_owned()]);
        assert_eq!(done.failed.len(), 1);
        assert!(
            done.failed[0].1.contains("a build started"),
            "{:?}",
            done.failed
        );
        assert!(!a.exists() && b.exists());
        // Nothing is left aside in Caches.
        let left: Vec<_> = fs::read_dir(&bed.caches)
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(left.len(), 1, "{left:?}");
    }

    #[test]
    fn an_interrupted_deletes_leftover_is_listed_and_removed() {
        let bed = bed();
        target(&bed, "kept", 1);
        let aside = bed.caches.join(format!(".{PREFIX}old.deleting-123"));
        fs::create_dir_all(aside.join("release")).unwrap();
        fs::create_dir_all(bed.caches.join(".unrelated.deleting-1")).unwrap();
        let f = facts(&bed);
        let plan = plan_in(&f, 1);
        assert_eq!(plan.leftovers, [aside.to_string_lossy().into_owned()]);
        assert_eq!(plan.remove, plan.leftovers, "it doesn't take a newest slot");
        let text = describe(&plan).join("\n");
        assert!(text.contains("Delete 1 install build folder "), "{text}");
        assert!(
            text.contains(&format!(
                "- .{PREFIX}old.deleting-123 (left by an interrupted delete)"
            )),
            "{text}"
        );
        let done = prune_in(&f, 1, Some(&plan.hash), &|| vec![]).unwrap();
        assert_eq!(done.removed, plan.leftovers);
        assert_eq!(done.kept, 1);
        assert!(!aside.exists() && bed.caches.join(".unrelated.deleting-1").exists());
    }

    #[test]
    fn keep_is_capped_and_an_empty_plan_says_so() {
        let bed = bed();
        target(&bed, "a", 1);
        let plan = plan_in(&facts(&bed), 500);
        assert_eq!(plan.keep_newest, MAX_KEEP);
        assert!(plan.remove.is_empty());
        assert_eq!(
            describe(&plan)[0],
            "Nothing to delete: every install build folder is kept."
        );
    }

    /// Prints this Mac's plan; reads only. `cargo test --lib live_plan -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads this Mac's caches and journals; run by hand"]
    fn live_plan_reads_only() {
        let plan = plan(DEFAULT_KEEP).unwrap();
        println!("{}", describe(&plan).join("\n"));
    }

    #[test]
    fn abbreviated_commits_match_but_short_ones_do_not() {
        assert!(same_commit("abcdef1234567", "abcdef1"));
        assert!(!same_commit("abcdef1234567", "abcdef"));
        assert!(!same_commit("abcdef1", "abcdef2"));
    }

    /// The folder an app was built in comes from its provenance's
    /// `checkout` (overnight S01a); none when absent, empty or unreadable.
    #[test]
    fn a_provenance_names_the_folder_it_was_built_in() {
        let bed = bed();
        let file = bed.caches.join("build-provenance.json");
        fs::write(
            &file,
            r#"{"commit":"abcdef1234567890","checkout":"/w/release","dirty":false}"#,
        )
        .unwrap();
        assert_eq!(
            provenance_checkout(&file),
            Some(PathBuf::from("/w/release"))
        );
        assert_eq!(
            provenance_commit(&file).as_deref(),
            Some("abcdef1234567890")
        );
        for text in [
            r#"{"commit":"abcdef1234567890"}"#,
            r#"{"checkout":""}"#,
            "not json",
        ] {
            fs::write(&file, text).unwrap();
            assert_eq!(provenance_checkout(&file), None, "{text}");
        }
        assert_eq!(provenance_checkout(&bed.caches.join("missing.json")), None);
    }
}
