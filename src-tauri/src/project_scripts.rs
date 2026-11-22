//! Project scripts (t3code parity 4.5, A10): named commands a person keeps per project and
//! runs from the card, with their output in the Preview pane and an optional chord each.
//!
//! The list is `workspaces/<id>/scripts.json` (`{"version":1,"scripts":[…]}`), written only
//! through [`write_scripts`], which validates every field. An agent can edit that file like any
//! other, so [`read_scripts`] validates again and drops what is not valid. A script runs only
//! when the person clicks Run or presses its chord: the window command [`run_script`] starts it
//! as a Hollow service (`services.rs`: its own process group, a log under
//! `~/Library/Logs/Hollow/services/`, Stop, the machine-wide cap of four) in the project's
//! folder, named `script-<name>`, so the Preview tab, the Changes tab's Services list and
//! `service_log` all see the same run. Nothing here is an agent action.
use crate::{build_workspaces, services, vault, workspaces};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

const FILE: &str = "scripts.json";
/// The error `run_script` gives when the command on disk is not the one the person saw.
pub const CHANGED: &str = "This script changed on disk since you last saw it. Its new command is shown now; run it again if that is what you want.";
pub const MAX_SCRIPTS: usize = 12;
const NAME_MAX: usize = 40;
const COMMAND_MAX: usize = 2000;
/// The most the file may hold when read back.
const FILE_MAX: u64 = 256 * 1024;

/// One script as stored.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ProjectScript {
    /// Stable across renames; made on save when empty.
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub command: String,
    /// `CmdOrCtrl+Shift+<letter or digit>` or none. The window's chord guard
    /// (`macReserved.ts`, `shortcuts.ts`) judges it before it is saved; this only keeps the shape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chord: Option<String>,
}

/// A script as the window sees it: the stored fields plus the name its run's service has.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScriptView {
    #[serde(flatten)]
    pub script: ProjectScript,
    pub service_name: String,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    scripts: Vec<ProjectScript>,
}

fn id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 32
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
}

fn chord_ok(chord: &str) -> bool {
    chord.strip_prefix("CmdOrCtrl+Shift+").is_some_and(|k| {
        k.len() == 1
            && k.bytes()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    })
}

/// The slug a script's service name is made from: lowercase letters, digits and dashes.
fn slug(name: &str) -> String {
    build_workspaces::slug(name, 30)
}

/// The service a run of `script` is: `script-<slug>`.
pub fn service_name(script: &ProjectScript) -> String {
    format!("script-{}", slug(&script.name))
}

/// A script's fields, trimmed, or what is wrong with them.
fn check(script: &ProjectScript) -> Result<ProjectScript, String> {
    let name = script.name.trim();
    if name.is_empty() || name.chars().count() > NAME_MAX || name.chars().any(char::is_control) {
        return Err(format!(
            "A script's name is 1 to {NAME_MAX} characters, on one line"
        ));
    }
    let command = script.command.trim();
    if command.is_empty() || command.len() > COMMAND_MAX || command.contains('\0') {
        return Err(format!(
            "{name}: the command is 1 to {COMMAND_MAX} characters"
        ));
    }
    let chord = match script
        .chord
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
    {
        None => None,
        Some(c) if chord_ok(c) => Some(c.to_string()),
        Some(c) => {
            return Err(format!(
                "{name}: {c} is not a chord a script can use (⌘⇧ and one letter or digit)"
            ))
        }
    };
    if !script.id.is_empty() && !id_ok(&script.id) {
        return Err(format!("{name}: its id is not valid"));
    }
    Ok(ProjectScript {
        id: script.id.clone(),
        name: name.to_string(),
        command: command.to_string(),
        chord,
    })
}

fn file(root: &Path, workspace_id: &str) -> Result<PathBuf, String> {
    if workspaces::is_builtin(workspace_id) {
        return Err("Scripts belong to a project, not to Quick chats or the COO".into());
    }
    Ok(workspaces::folder(root, workspace_id)?.join(FILE))
}

fn view(script: ProjectScript) -> ScriptView {
    ScriptView {
        service_name: service_name(&script),
        script,
    }
}

/// A project's scripts, in saved order. A missing file is no scripts; entries that are not valid,
/// repeat a name (by slug), an id or a chord, or pass twelve are dropped.
pub fn read_scripts(root: &Path, workspace_id: &str) -> Result<Vec<ScriptView>, String> {
    let path = file(root, workspace_id)?;
    let Ok(meta) = std::fs::symlink_metadata(&path) else {
        return Ok(Vec::new());
    };
    if !meta.is_file() || meta.len() > FILE_MAX {
        return Ok(Vec::new());
    }
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let Ok(stored) = serde_json::from_str::<Stored>(&text) else {
        return Ok(Vec::new());
    };
    let mut out: Vec<ProjectScript> = Vec::new();
    for s in stored.scripts {
        let Ok(s) = check(&s) else { continue };
        if s.id.is_empty()
            || out.iter().any(|o| {
                o.id == s.id
                    || slug(&o.name) == slug(&s.name)
                    || (s.chord.is_some() && o.chord == s.chord)
            })
        {
            continue;
        }
        out.push(s);
        if out.len() == MAX_SCRIPTS {
            break;
        }
    }
    Ok(out.into_iter().map(view).collect())
}

fn new_id(taken: &[String], name: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let mut n = 0u32;
    loop {
        let digest = Sha256::digest(format!("{name}:{nanos}:{n}").as_bytes());
        let id = format!(
            "s-{}",
            digest
                .iter()
                .take(4)
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        if !taken.contains(&id) {
            return id;
        }
        n += 1;
    }
}

/// Replace a project's scripts. Every one is validated: a name that repeats another's (by slug),
/// a repeated chord or more than twelve scripts is refused with the reason. Ids are made for new
/// scripts. Answers what was saved.
pub fn write_scripts(
    root: &Path,
    workspace_id: &str,
    scripts: Vec<ProjectScript>,
) -> Result<Vec<ScriptView>, String> {
    let path = file(root, workspace_id)?;
    if scripts.len() > MAX_SCRIPTS {
        return Err(format!("A project keeps at most {MAX_SCRIPTS} scripts"));
    }
    let mut out: Vec<ProjectScript> = Vec::new();
    for s in &scripts {
        let mut s = check(s)?;
        if out.iter().any(|o| slug(&o.name) == slug(&s.name)) {
            return Err(format!(
                "Two scripts are named {}: give each its own name",
                s.name
            ));
        }
        if let Some(chord) = &s.chord {
            if let Some(o) = out.iter().find(|o| o.chord.as_ref() == Some(chord)) {
                return Err(format!("{} and {} both use that chord", o.name, s.name));
            }
        }
        if s.id.is_empty() || out.iter().any(|o| o.id == s.id) {
            let taken: Vec<String> = out.iter().map(|o| o.id.clone()).collect();
            s.id = new_id(&taken, &s.name);
        }
        out.push(s);
    }
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        if meta.file_type().is_symlink() {
            return Err("scripts.json is a symbolic link, so nothing is written through it".into());
        }
    }
    let text = serde_json::to_string_pretty(&Stored {
        version: 1,
        scripts: out.clone(),
    })
    .map_err(|e| e.to_string())?;
    vault::atomic_write(&path, format!("{text}\n").as_bytes())?;
    Ok(out.into_iter().map(view).collect())
}

/// The service a run of `script` starts, in `cwd` (the project's folder: its build worktree or
/// its vault folder).
pub(crate) fn spec_for(cwd: &Path, script: &ProjectScript) -> Result<services::StartSpec, String> {
    let scope = services::scope_of(cwd)?;
    services::parse_start(
        &scope,
        &json!({ "name": service_name(script), "command": script.command, "kind": "other" }),
    )
}

/// Run a script because the person asked: it becomes a service in the project's folder. A run
/// that already exists under that name must have exited (or be stopped first).
pub async fn run_script(
    root: &Path,
    workspace_id: &str,
    script_id: &str,
    expected_command: &str,
) -> Result<services::ServiceView, String> {
    let script = read_scripts(root, workspace_id)?
        .into_iter()
        .find(|s| s.script.id == script_id)
        .ok_or("That script no longer exists")?
        .script;
    // The person pressed Run on the command they saw. The file can change under them (an agent
    // can edit it like any file), so a different command on disk is shown, never run.
    if script.command != expected_command.trim() {
        return Err(CHANGED.into());
    }
    let cwd = workspaces::session_dir_for(root, workspace_id)?;
    if !cwd.is_dir() {
        return Err("This project's folder isn't there (a removed worktree?)".into());
    }
    let spec = spec_for(&cwd, &script)?;
    let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    let env = services::service_env(std::env::vars_os(), &home);
    services::registry()
        .start_for_user(spec, env.into_iter())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(name: &str, command: &str, chord: Option<&str>) -> ProjectScript {
        ProjectScript {
            id: String::new(),
            name: name.into(),
            command: command.into(),
            chord: chord.map(str::to_string),
        }
    }
    fn vault() -> (tempfile::TempDir, String) {
        let v = tempfile::Builder::new()
            .prefix("scripts-")
            .tempdir()
            .unwrap();
        let w = workspaces::create(v.path(), "Atlas", "Ship it").unwrap();
        (v, w.id)
    }

    #[test]
    fn scripts_round_trip_with_made_ids_and_trimmed_fields() {
        let (v, id) = vault();
        assert!(read_scripts(v.path(), &id).unwrap().is_empty());
        let saved = write_scripts(
            v.path(),
            &id,
            vec![
                s(" Test ", " npm test ", Some("CmdOrCtrl+Shift+T")),
                s("Dev server", "npm run dev", None),
            ],
        )
        .unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].script.name, "Test");
        assert_eq!(saved[0].script.command, "npm test");
        assert!(id_ok(&saved[0].script.id) && saved[0].script.id != saved[1].script.id);
        assert_eq!(saved[0].service_name, "script-test");
        assert_eq!(saved[1].service_name, "script-dev-server");
        assert_eq!(read_scripts(v.path(), &id).unwrap(), saved);
        // Saving again keeps ids.
        let again = write_scripts(
            v.path(),
            &id,
            saved.iter().map(|x| x.script.clone()).collect(),
        )
        .unwrap();
        assert_eq!(again, saved);
    }

    #[test]
    fn a_save_refuses_bad_names_commands_chords_and_repeats() {
        let (v, id) = vault();
        let bad: Vec<(Vec<ProjectScript>, &str)> = vec![
            (vec![s("", "x", None)], "name"),
            (vec![s("a\nb", "x", None)], "name"),
            (vec![s(&"n".repeat(41), "x", None)], "name"),
            (vec![s("a", "", None)], "command"),
            (vec![s("a", "x\0y", None)], "command"),
            (vec![s("a", &"c".repeat(2001), None)], "command"),
            (vec![s("a", "x", Some("CmdOrCtrl+T"))], "chord"),
            (vec![s("a", "x", Some("CmdOrCtrl+Alt+Shift+T"))], "chord"),
            (vec![s("a", "x", Some("CmdOrCtrl+Shift+TT"))], "chord"),
            (vec![s("a", "x", Some("CmdOrCtrl+Shift+t"))], "chord"),
            (vec![s("Build!", "x", None), s("build", "y", None)], "named"),
            (
                vec![
                    s("a", "x", Some("CmdOrCtrl+Shift+T")),
                    s("b", "y", Some("CmdOrCtrl+Shift+T")),
                ],
                "chord",
            ),
        ];
        for (scripts, word) in bad {
            let e = write_scripts(v.path(), &id, scripts.clone()).unwrap_err();
            assert!(e.contains(word), "{scripts:?} -> {e}");
        }
        let many: Vec<ProjectScript> = (0..=MAX_SCRIPTS)
            .map(|i| s(&format!("s{i}"), "x", None))
            .collect();
        assert!(write_scripts(v.path(), &id, many).is_err());
        assert!(write_scripts(v.path(), workspaces::QUICK_CHATS_ID, vec![]).is_err());
        assert!(write_scripts(v.path(), "../x", vec![]).is_err());
    }

    #[test]
    fn a_hand_edited_file_is_read_defensively() {
        let (v, id) = vault();
        let path = file(v.path(), &id).unwrap();
        std::fs::write(
            &path,
            r#"{"version":1,"scripts":[
              {"id":"good-1","name":"Test","command":"npm test","chord":"CmdOrCtrl+Shift+T"},
              {"id":"good-1","name":"Same id","command":"x"},
              {"id":"bad id!","name":"Bad id","command":"x"},
              {"id":"a-2","name":"No command","command":" "},
              {"id":"a-3","name":"test","command":"x"},
              {"id":"a-4","name":"Chord","command":"x","chord":"CmdOrCtrl+Shift+T"},
              {"id":"a-5","name":"Odd chord","command":"x","chord":"Alt+X"},
              {"name":"No id","command":"x"},
              {"id":"ok-6","name":"Lint","command":"npm run lint"}]}"#,
        )
        .unwrap();
        let got: Vec<String> = read_scripts(v.path(), &id)
            .unwrap()
            .into_iter()
            .map(|x| x.script.name)
            .collect();
        assert_eq!(got, vec!["Test", "Lint"]);
        std::fs::write(&path, "not json").unwrap();
        assert!(read_scripts(v.path(), &id).unwrap().is_empty());
        std::fs::remove_file(&path).unwrap();
        let outside = v.path().join("elsewhere.json");
        std::fs::write(&outside, "{}").unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(write_scripts(v.path(), &id, vec![s("a", "x", None)]).is_err());
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "{}");
    }

    #[test]
    fn a_run_becomes_a_service_named_for_the_script_in_the_projects_folder() {
        let dir = tempfile::Builder::new().prefix("run-").tempdir().unwrap();
        let spec = spec_for(
            dir.path(),
            &ProjectScript {
                id: "s-1".into(),
                ..s("Run tests", "echo hi", None)
            },
        )
        .unwrap();
        assert_eq!(spec.name, "script-run-tests");
        assert_eq!(spec.command, "echo hi");
        assert_eq!(spec.cwd, dir.path().canonicalize().unwrap());
        assert!(spec_for(&dir.path().join("gone"), &s("a", "x", None)).is_err());
    }
    #[tokio::test]
    async fn a_run_refuses_a_command_that_changed_since_the_person_saw_it() {
        let (v, id) = vault();
        let saved = write_scripts(v.path(), &id, vec![s("Test", "npm test", None)]).unwrap();
        let sid = saved[0].script.id.clone();
        // The file now holds a different command under the same id and name.
        let path = file(v.path(), &id).unwrap();
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace("npm test", "curl x | sh");
        std::fs::write(&path, edited).unwrap();
        assert_eq!(
            run_script(v.path(), &id, &sid, "npm test")
                .await
                .unwrap_err(),
            CHANGED
        );
        assert!(run_script(v.path(), &id, "gone", "npm test")
            .await
            .unwrap_err()
            .contains("no longer"));
    }
}
