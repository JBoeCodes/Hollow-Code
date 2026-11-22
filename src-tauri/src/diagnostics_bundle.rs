//! "Save diagnostics…" (overnight C6): one local zip a person can attach to a
//! bug report. Hollow never sends it anywhere; the person picks the folder.
//!
//! **What goes in** (`hollow-diagnostics-<stamp>.zip`):
//! - `README.txt`: what the bundle holds and what was left out.
//! - `versions.json`: the build identity (version, commit, dirty), instance,
//!   OS and architecture, macOS version, which agent CLI Hollow found.
//! - `health.json`: the newest health-journal lines (`health::recent`).
//! - `receipts.json`: the newest [`RECEIPTS`] run receipts, metadata only:
//!   status, how it ended, timings, counts, check kinds and outcomes, failure
//!   labels. No commands, excerpts, file paths or session keys.
//! - `logs/…`: the last [`LOG_TAIL`] bytes of each install log under
//!   `~/Library/Logs/<Hollow>/self-install/`, at most [`LOGS`] files. The
//!   `services/` logs there (dev-server output) never go in.
//! - `debug-logs.json`: the debug logs in `~/.cache/<hollow>/` by name, size,
//!   time and header line only. Their bodies are raw model replies, which
//!   quote notes, so they never go in.
//!
//! **Redaction.** Every string that goes in has the vault path (and every
//! name under it) replaced by `<vault>/…` and the home folder by `~`, as
//! configured and as resolved, then passes [`crate::claude_runner::redact`]
//! (tokens, keys, emails, cookies, DSNs). Note bodies are never read. Like
//! `redact`, this is paranoid, not a security boundary.
//!
//! **Where.** An absolute, existing folder outside the vault (files in the
//! vault would be indexed and synced). Never overwrites: a name that exists
//! gets `-2`, `-3`, …

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Receipts kept in the bundle (newest first).
pub const RECEIPTS: usize = 200;
/// Log files kept, newest first.
pub const LOGS: usize = 30;
/// Bytes kept from the end of each log.
pub const LOG_TAIL: u64 = 256 * 1024;

/// Where the bundle reads from, so tests can point it at temp folders.
pub struct Sources {
    pub vault: PathBuf,
    pub home: PathBuf,
    /// `~/.cache/<hollow>`: debug logs, listed only.
    pub debug_dir: PathBuf,
    /// `~/Library/Logs/<Hollow>`: only its `self-install/` logs go in (its
    /// `services/` folder holds arbitrary dev-server output and never does).
    pub logs_dir: PathBuf,
}

/// The folder under [`Sources::logs_dir`] whose logs go in.
const INSTALL_LOGS: &str = "self-install";

impl Sources {
    pub fn for_this_mac(vault: &Path) -> Result<Self, String> {
        let home = dirs::home_dir().ok_or("Hollow can't find your home folder.")?;
        Ok(Self {
            vault: vault.to_path_buf(),
            debug_dir: crate::instance::dot_cache_dir(&home),
            logs_dir: crate::instance::logs_dir(&home),
            home,
        })
    }

    /// Hide the vault (and every name under it) and the home folder, then
    /// redact secrets. Paths go first: `redact` would otherwise mask an email
    /// inside a path (a Google Drive vault) and the path would no longer
    /// match.
    fn scrub(&self, text: &str) -> String {
        use once_cell::sync::Lazy;
        use regex::Regex;
        static UNDER_VAULT: Lazy<Regex> =
            Lazy::new(|| Regex::new(r#"<vault>/[^\s"'<>()\[\]`]+"#).unwrap());
        let mut paths: Vec<(String, &str)> = Vec::new();
        for (path, label) in [(&self.vault, "<vault>"), (&self.home, "~")] {
            paths.push((path.to_string_lossy().to_string(), label));
            if let Ok(real) = std::fs::canonicalize(path) {
                paths.push((real.to_string_lossy().to_string(), label));
            }
        }
        // Longest first, so the vault (usually under home) wins.
        paths.sort_by_key(|(p, _)| std::cmp::Reverse(p.len()));
        let mut s = text.to_string();
        for (p, label) in paths {
            let p = p.trim_end_matches('/');
            if p.len() > 1 {
                s = replace_path(&s, p, label);
            }
        }
        let s = UNDER_VAULT.replace_all(&s, "<vault>/…");
        crate::claude_runner::redact(&s)
    }
}

/// Replace `path` where it is a whole path prefix: followed by `/`, the end,
/// or a character that can't continue a name (so `/Users/ann` leaves
/// `/Users/anna` alone).
fn replace_path(text: &str, path: &str, label: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(path) {
        let after = &rest[at + path.len()..];
        let whole = after
            .chars()
            .next()
            .is_none_or(|c| !(c.is_alphanumeric() || matches!(c, '-' | '_' | '.')));
        out.push_str(&rest[..at]);
        out.push_str(if whole { label } else { path });
        rest = after;
    }
    out.push_str(rest);
    out
}

/// What was saved.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Saved {
    pub path: String,
    pub bytes: u64,
    /// Files inside the zip.
    pub entries: u32,
}

/// Write the bundle into `dir`.
pub fn save(
    sources: &Sources,
    dir: &Path,
    now: chrono::DateTime<chrono::Local>,
) -> Result<Saved, String> {
    if !dir.is_absolute() {
        return Err("Choose a folder (an absolute path) to save diagnostics in.".into());
    }
    let target = std::fs::canonicalize(dir)
        .map_err(|_| "That folder doesn't exist; choose another.".to_string())?;
    if !target.is_dir() {
        return Err("Choose a folder, not a file.".into());
    }
    let vault = std::fs::canonicalize(&sources.vault).unwrap_or_else(|_| sources.vault.clone());
    if target.starts_with(&vault) {
        return Err(
            "Choose a folder outside the vault: files there are indexed and synced.".into(),
        );
    }
    let files = contents(sources, now);
    let stem = format!("hollow-diagnostics-{}", now.format("%Y%m%d-%H%M%S"));
    let (path, file) = create_new(&target, &stem)?;
    let written = write_zip(file, &files);
    if let Err(e) = written {
        // Only the file this call just created.
        let _ = std::fs::remove_file(&path);
        return Err(format!(
            "Saving diagnostics failed ({e}); nothing was kept."
        ));
    }
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(Saved {
        path: path.to_string_lossy().to_string(),
        bytes,
        entries: files.len() as u32,
    })
}

fn create_new(dir: &Path, stem: &str) -> Result<(PathBuf, std::fs::File), String> {
    for n in 1..100 {
        let name = if n == 1 {
            format!("{stem}.zip")
        } else {
            format!("{stem}-{n}.zip")
        };
        let path = dir.join(name);
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(f) => return Ok((path, f)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Hollow couldn't create a file there ({e}).")),
        }
    }
    Err("Too many diagnostics files with that name in that folder.".into())
}

fn write_zip(file: std::fs::File, files: &[(String, Vec<u8>)]) -> Result<(), String> {
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o600);
    for (name, body) in files {
        zip.start_file(name.as_str(), options)
            .map_err(|e| e.to_string())?;
        zip.write_all(body).map_err(|e| e.to_string())?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn pretty(value: &Value) -> Vec<u8> {
    let mut v = serde_json::to_vec_pretty(value).unwrap_or_default();
    v.push(b'\n');
    v
}

/// The bundle's files, every string already scrubbed.
pub fn contents(sources: &Sources, now: chrono::DateTime<chrono::Local>) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let logs = log_tails(sources);
    let debug = debug_listing(sources);
    let readme = format!(
        "Hollow diagnostics, saved {}.\n\n\
         Saved on this Mac only; Hollow never sends it. Attach it to a bug report if you choose.\n\n\
         versions.json    build, instance, OS, agent CLI found\n\
         health.json      newest health-journal lines\n\
         receipts.json    newest {RECEIPTS} run receipts: status, timings, counts (no commands, excerpts or file paths)\n\
         logs/            last {} KB of each install log ({} files)\n\
         debug-logs.json  debug logs by name and size only (their bodies are model replies that quote notes)\n\n\
         Secrets (tokens, keys, cookies), email addresses, your home folder, the vault path and the\n\
         names of files in it are masked. Note bodies are never read.\n",
        now.to_rfc3339(),
        LOG_TAIL / 1024,
        logs.len(),
    );
    files.push(("README.txt".to_string(), readme.into_bytes()));
    files.push((
        "versions.json".to_string(),
        pretty(&scrub_value(sources, versions())),
    ));
    let health: Vec<Value> = crate::health::recent(&sources.vault)
        .into_iter()
        .map(|e| serde_json::to_value(e).unwrap_or(Value::Null))
        .collect();
    files.push((
        "health.json".to_string(),
        pretty(&scrub_value(sources, json!(health))),
    ));
    files.push((
        "receipts.json".to_string(),
        pretty(&scrub_value(sources, receipts(&sources.vault))),
    ));
    files.push((
        "debug-logs.json".to_string(),
        pretty(&scrub_value(sources, debug)),
    ));
    files.extend(logs);
    files
}

/// Scrub every string in a JSON value.
fn scrub_value(sources: &Sources, value: Value) -> Value {
    match value {
        Value::String(s) => Value::String(sources.scrub(&s)),
        Value::Array(a) => Value::Array(a.into_iter().map(|v| scrub_value(sources, v)).collect()),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, v)| (k, scrub_value(sources, v)))
                .collect(),
        ),
        other => other,
    }
}

fn versions() -> Value {
    let instance = crate::instance::current();
    json!({
        "build": crate::app_menu::build_identity(),
        "instance": { "identifier": instance.identifier, "label": instance.label },
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "macos": macos_version(),
        "claudeCli": crate::chat::find_claude_binary().map(|p| p.to_string_lossy().to_string()),
        "debugLogEnabled": matches!(
            std::env::var("HOLLOW_DEBUG_LOG").ok().as_deref(),
            Some("1" | "true" | "yes" | "on")
        ),
    })
}

/// `ProductVersion` from the system's version plist, read as text.
fn macos_version() -> Option<String> {
    let text = std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()?;
    let at = text.find("<key>ProductVersion</key>")?;
    let rest = &text[at..];
    let open = rest.find("<string>")? + "<string>".len();
    let close = rest[open..].find("</string>")?;
    Some(rest[open..open + close].to_string())
}

/// Receipts as metadata: nothing a person wrote or a command printed.
pub fn receipts(vault: &Path) -> Value {
    let list = crate::evidence::list(vault, None, None, RECEIPTS).unwrap_or_default();
    let rows: Vec<Value> = list
        .into_iter()
        .map(|r| {
            json!({
                "id": r.id,
                "source": r.source,
                "status": r.status,
                "resultKind": r.result_kind,
                "verificationState": r.verification_state,
                "startedAt": r.started_at,
                "finishedAt": r.finished_at,
                "processStop": r.process_stop,
                "files": r.files.len(),
                "repoChanges": r.repo_changes.len(),
                "checks": r.checks.iter().map(|c| json!({ "kind": c.kind, "outcome": c.outcome })).collect::<Vec<_>>(),
                "failures": r.failures.iter().map(|f| f.label.clone()).collect::<Vec<_>>(),
                "phaseTimings": r.phase_timings,
            })
        })
        .collect();
    json!(rows)
}

/// Text files under `dir`, one level of subfolders deep, newest first.
fn text_files(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    let mut out = Vec::new();
    let walk =
        |d: &Path, out: &mut Vec<(PathBuf, std::time::SystemTime)>, deeper: &mut Vec<PathBuf>| {
            let Ok(read) = std::fs::read_dir(d) else {
                return;
            };
            for entry in read.flatten() {
                let path = entry.path();
                let Ok(meta) = entry.metadata() else { continue };
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if meta.is_dir() {
                    deeper.push(path);
                } else if meta.is_file()
                    && matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("log" | "txt" | "jsonl")
                    )
                {
                    out.push((path, meta.modified().unwrap_or(std::time::UNIX_EPOCH)));
                }
            }
        };
    let mut deeper = Vec::new();
    walk(dir, &mut out, &mut deeper);
    for d in deeper {
        walk(&d, &mut out, &mut Vec::new());
    }
    out.sort_by_key(|f| std::cmp::Reverse(f.1));
    out
}

fn log_tails(sources: &Sources) -> Vec<(String, Vec<u8>)> {
    use std::io::{Read, Seek, SeekFrom};
    text_files(&sources.logs_dir.join(INSTALL_LOGS))
        .into_iter()
        .take(LOGS)
        .filter_map(|(path, _)| {
            let rel = path.strip_prefix(&sources.logs_dir).ok()?;
            let mut f = std::fs::File::open(&path).ok()?;
            let len = f.metadata().ok()?.len();
            let from = len.saturating_sub(LOG_TAIL);
            f.seek(SeekFrom::Start(from)).ok()?;
            let mut raw = Vec::new();
            f.take(LOG_TAIL).read_to_end(&mut raw).ok()?;
            let text = if from > 0 {
                // Start at a whole line (cut on the raw bytes, so the count
                // is exact), and say what was cut.
                let cut = raw.iter().position(|b| *b == b'\n').map_or(0, |i| i + 1);
                format!(
                    "[… first {} bytes left out]\n{}",
                    from + cut as u64,
                    String::from_utf8_lossy(&raw[cut..])
                )
            } else {
                String::from_utf8_lossy(&raw).to_string()
            };
            Some((
                format!("logs/{}", rel.to_string_lossy()),
                sources.scrub(&text).into_bytes(),
            ))
        })
        .collect()
}

fn debug_listing(sources: &Sources) -> Value {
    let rows: Vec<Value> = text_files(&sources.debug_dir)
        .into_iter()
        .filter(|(p, _)| p.parent() == Some(sources.debug_dir.as_path()))
        .filter(|(p, _)| p.extension().and_then(|e| e.to_str()) == Some("log"))
        .map(|(path, modified)| {
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            // Only the `=== tag @ time ===` header, never the body.
            let header = std::fs::File::open(&path).ok().and_then(|f| {
                use std::io::{BufRead, Read};
                let mut line = String::new();
                std::io::BufReader::new(f.take(512))
                    .read_line(&mut line)
                    .ok()?;
                let line = line.trim_end().to_string();
                (line.starts_with("=== ") && line.ends_with(" ===")).then_some(line)
            });
            json!({
                "name": path.file_name().map(|n| n.to_string_lossy().to_string()),
                "bytes": bytes,
                "modified": chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339(),
                "header": header,
            })
        })
        .collect();
    json!(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn now() -> chrono::DateTime<chrono::Local> {
        use chrono::TimeZone;
        chrono::Local
            .with_ymd_and_hms(2026, 10, 10, 3, 4, 5)
            .unwrap()
    }

    /// A home with a vault, an install log, a debug log and a receipt.
    fn fixture() -> (tempfile::TempDir, Sources) {
        let dir = tempfile::Builder::new().prefix("home-").tempdir().unwrap();
        let home = dir.path().to_path_buf();
        let vault = home.join("vault");
        let logs_dir = home.join("Library/Logs/Hollow");
        let debug_dir = home.join(".cache/hollow");
        std::fs::create_dir_all(vault.join("notes")).unwrap();
        std::fs::create_dir_all(logs_dir.join("self-install")).unwrap();
        std::fs::create_dir_all(logs_dir.join("services/abc")).unwrap();
        std::fs::write(
            logs_dir.join("services/abc/dev.log"),
            "SERVICE OUTPUT from a dev server\n",
        )
        .unwrap();
        std::fs::create_dir_all(&debug_dir).unwrap();
        std::fs::write(
            vault.join("notes/secret-plan.md"),
            "NOTE BODY: the acquisition closes Friday.",
        )
        .unwrap();
        std::fs::write(
            logs_dir.join("self-install/install.log"),
            format!(
                "copying {}/vault/notes/secret-plan.md\nusing key sk-ant-abcdefghijklmnopqrstuv\nmail ann@example.com\ndone\n",
                home.display()
            ),
        )
        .unwrap();
        std::fs::write(
            debug_dir.join("calendar.log"),
            "=== calendar @ 2026-10-10T01:00:00-04:00 ===\nNOTE BODY: the acquisition closes Friday.\n",
        )
        .unwrap();
        crate::evidence::persist_for_test(
            &vault,
            crate::evidence::EvidenceReceipt {
                id: "r1".into(),
                run_id: "run-1".into(),
                source: "chat".into(),
                status: "failed".into(),
                finished_at: "2026-10-10T02:00:00Z".into(),
                session_key: Some("sess-secret".into()),
                checks: vec![crate::evidence::ReceiptCheck {
                    command: "cat notes/secret-plan.md".into(),
                    kind: "test".into(),
                    outcome: "failed".into(),
                    sequence: 1,
                    excerpt: Some("NOTE BODY".into()),
                }],
                failures: vec![crate::evidence::ReceiptFailure {
                    label: "result".into(),
                    sequence: 2,
                    excerpt: "NOTE BODY: the acquisition".into(),
                }],
                ..Default::default()
            },
        )
        .unwrap();
        let sources = Sources {
            vault,
            home,
            debug_dir,
            logs_dir,
        };
        (dir, sources)
    }

    fn unzip(path: &str) -> Vec<(String, String)> {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        (0..zip.len())
            .map(|i| {
                let mut f = zip.by_index(i).unwrap();
                let mut text = String::new();
                f.read_to_string(&mut text).unwrap();
                (f.name().to_string(), text)
            })
            .collect()
    }

    #[test]
    fn the_bundle_holds_versions_receipts_and_log_tails_with_nothing_private() {
        let (dir, sources) = fixture();
        let out = dir.path().join("Desktop");
        std::fs::create_dir_all(&out).unwrap();
        let saved = save(&sources, &out, now()).unwrap();
        assert!(saved
            .path
            .ends_with("hollow-diagnostics-20261010-030405.zip"));
        let files = unzip(&saved.path);
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "README.txt",
                "versions.json",
                "health.json",
                "receipts.json",
                "debug-logs.json",
                "logs/self-install/install.log"
            ]
        );
        assert_eq!(saved.entries, 6);
        let all: String = files.iter().map(|(_, t)| t.as_str()).collect();
        // No note body, secret, email, session key, command or path.
        for private in [
            "NOTE BODY",
            "acquisition",
            "sk-ant-abcdefghijklmnopqrstuv",
            "ann@example.com",
            "sess-secret",
            "secret-plan",
            "SERVICE OUTPUT",
            sources.home.to_string_lossy().as_ref(),
        ] {
            assert!(!all.contains(private), "{private} leaked:\n{all}");
        }
        let receipts: Value =
            serde_json::from_str(&files.iter().find(|(n, _)| n == "receipts.json").unwrap().1)
                .unwrap();
        assert_eq!(receipts[0]["status"], "failed");
        assert_eq!(receipts[0]["checks"][0]["kind"], "test");
        assert_eq!(receipts[0]["failures"][0], "result");
        let debug: Value = serde_json::from_str(
            &files
                .iter()
                .find(|(n, _)| n == "debug-logs.json")
                .unwrap()
                .1,
        )
        .unwrap();
        assert_eq!(debug[0]["name"], "calendar.log");
        assert_eq!(
            debug[0]["header"],
            "=== calendar @ 2026-10-10T01:00:00-04:00 ==="
        );
        let log = &files
            .iter()
            .find(|(n, _)| n.starts_with("logs/"))
            .unwrap()
            .1;
        assert!(log.contains("copying <vault>/…\n"), "{log}");
        assert!(log.contains("done"));
        let versions = &files.iter().find(|(n, _)| n == "versions.json").unwrap().1;
        assert!(versions.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn paths_are_masked_before_redaction_and_only_as_whole_paths() {
        let sources = Sources {
            vault: PathBuf::from(
                "/Users/ann/Library/CloudStorage/GoogleDrive-ann@gmail.com/My Drive/Vault",
            ),
            home: PathBuf::from("/Users/ann"),
            debug_dir: PathBuf::from("/nowhere"),
            logs_dir: PathBuf::from("/nowhere"),
        };
        assert_eq!(
            sources.scrub(
                "read /Users/ann/Library/CloudStorage/GoogleDrive-ann@gmail.com/My Drive/Vault/notes/plan.md, \
                 then /Users/ann/.zshrc and /Users/anna/x"
            ),
            "read <vault>/… then ~/.zshrc and /Users/anna/x"
        );
        assert_eq!(
            sources.scrub("<vault> alone and /Users/ann"),
            "<vault> alone and ~"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_zip_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, sources) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let saved = save(&sources, &out, now()).unwrap();
        let mode = std::fs::metadata(&saved.path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn it_never_overwrites_and_refuses_the_vault_and_relative_folders() {
        let (dir, sources) = fixture();
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        let first = save(&sources, &out, now()).unwrap();
        let second = save(&sources, &out, now()).unwrap();
        assert!(second
            .path
            .ends_with("hollow-diagnostics-20261010-030405-2.zip"));
        assert_ne!(first.path, second.path);
        assert!(save(&sources, &sources.vault.join("notes"), now())
            .unwrap_err()
            .contains("outside the vault"));
        assert!(save(&sources, Path::new("relative"), now())
            .unwrap_err()
            .contains("absolute"));
        assert!(save(&sources, &dir.path().join("missing"), now())
            .unwrap_err()
            .contains("doesn't exist"));
    }

    #[test]
    fn a_long_log_keeps_its_tail_from_a_whole_line() {
        let (dir, sources) = fixture();
        let line = "x".repeat(99) + "\n";
        let big = line.repeat((LOG_TAIL as usize / 100) + 50) + "LAST LINE\n";
        std::fs::write(sources.logs_dir.join("self-install/big.log"), &big).unwrap();
        let tails = log_tails(&sources);
        let (_, body) = tails
            .iter()
            .find(|(n, _)| n == "logs/self-install/big.log")
            .unwrap();
        let text = String::from_utf8(body.clone()).unwrap();
        assert!(text.starts_with("[… first "), "{}", &text[..40]);
        assert!(text.ends_with("LAST LINE\n"));
        assert!(text.len() as u64 <= LOG_TAIL + 64);
        let second = text.lines().nth(1).unwrap();
        assert_eq!(second.len(), 99, "starts at a whole line");
        drop(dir);
    }
}
