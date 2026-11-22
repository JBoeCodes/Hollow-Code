//! Is this bundle the update we were told to install? Read its Info.plist,
//! refuse a bundle that is a symlink, has a symlink pointing outside itself,
//! or sits inside the installed app; then `codesign --verify --deep
//! --strict`, the expected TeamIdentifier, version and bundle id, and no
//! downgrade unless allowed.

use super::effects::Signer;
use super::{Layout, BUNDLE_ID};
use std::cmp::Ordering;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// What the bundle's Info.plist says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BundleInfo {
    pub(crate) version: String,
    pub(crate) bundle_id: String,
    pub(crate) executable: String,
}

pub(crate) fn read_info(bundle: &Path) -> Result<BundleInfo, String> {
    let plist = bundle.join("Contents").join("Info.plist");
    let value = plist::Value::from_file(&plist)
        .map_err(|e| format!("could not read {}: {e}", plist.display()))?;
    let dict = value
        .as_dictionary()
        .ok_or_else(|| format!("{} is not a dictionary", plist.display()))?;
    let get = |key: &str| {
        dict.get(key)
            .and_then(|v| v.as_string())
            .map(str::to_string)
            .ok_or_else(|| format!("{} has no {key}", plist.display()))
    };
    Ok(BundleInfo {
        version: get("CFBundleShortVersionString")?,
        bundle_id: get("CFBundleIdentifier")?,
        executable: get("CFBundleExecutable")?,
    })
}

/// The team a bundle must be signed by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Team {
    Id(String),
    /// Ad hoc (`codesign -s -`): no team. Accepted only when asked for
    /// explicitly (`--expected-team -` or `adhoc`), which tests do.
    AdHoc,
}

impl Team {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        match text {
            "-" | "adhoc" | "ad-hoc" => Ok(Team::AdHoc),
            t if super::effects::is_team_id(t) => Ok(Team::Id(t.to_string())),
            _ => Err(format!("`{text}` is not a team id (or `-` for ad hoc)")),
        }
    }

    pub(crate) fn label(&self) -> String {
        match self {
            Team::Id(id) => id.clone(),
            Team::AdHoc => "-".into(),
        }
    }
}

/// The staged bundle's place and shape, before any signature check.
pub(crate) fn check_location(staged: &Path, layout: &Layout) -> Result<PathBuf, String> {
    let meta = fs::symlink_metadata(staged)
        .map_err(|e| format!("the staged app {} can't be read: {e}", staged.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "the staged app {} is a symlink; pass the bundle itself",
            staged.display()
        ));
    }
    if !meta.is_dir() || staged.extension().is_none_or(|e| e != "app") {
        return Err(format!("{} is not an app bundle", staged.display()));
    }
    let staged = staged
        .canonicalize()
        .map_err(|e| format!("could not resolve {}: {e}", staged.display()))?;
    for ours in [
        &layout.installed,
        &layout.previous,
        &layout.staging,
        &layout.discard,
    ] {
        let ours = ours.canonicalize().unwrap_or_else(|_| ours.clone());
        if staged.starts_with(&ours) {
            return Err(format!(
                "the staged app {} is inside {}, which the install replaces",
                staged.display(),
                ours.display()
            ));
        }
        if ours.starts_with(&staged) {
            return Err(format!(
                "the install path {} is inside the staged app {}",
                ours.display(),
                staged.display()
            ));
        }
    }
    if let Some(link) = escaping_symlinks(&staged).into_iter().next() {
        return Err(format!(
            "the staged app has a symlink that points outside it: {}",
            link.display()
        ));
    }
    Ok(staged)
}

/// Every symlink under `root` whose target resolves outside `root`
/// (absolute targets, or `..` past the bundle). Framework links such as
/// `Versions/Current -> A` stay inside and are fine.
pub(crate) fn escaping_symlinks(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                // Resolve the whole chain (a link to a link that leaves);
                // a dangling link is judged by its text alone.
                let inside = match fs::canonicalize(&path) {
                    Ok(resolved) => resolved.starts_with(root),
                    Err(_) => fs::read_link(&path)
                        .ok()
                        .and_then(|target| normalize(&dir.join(target)))
                        .is_some_and(|resolved| resolved.starts_with(root)),
                };
                if !inside {
                    found.push(path);
                }
            } else if kind.is_dir() {
                stack.push(path);
            }
        }
    }
    found.sort();
    found
}

/// `path` with `.` and `..` folded away lexically; `None` past the root.
fn normalize(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    Some(out)
}

/// Signature, team, bundle id and (when given) version. Returns the info.
pub(crate) fn check_bundle(
    bundle: &Path,
    signer: &dyn Signer,
    team: Option<&Team>,
    version: Option<&str>,
) -> Result<BundleInfo, String> {
    let info = read_info(bundle)?;
    if info.bundle_id != BUNDLE_ID {
        return Err(format!(
            "the bundle identifier is {}, not {BUNDLE_ID}",
            info.bundle_id
        ));
    }
    if let Some(expected) = version {
        if info.version != expected {
            return Err(format!(
                "the app is version {}, not the expected {expected}",
                info.version
            ));
        }
    }
    signer
        .verify(bundle)
        .map_err(|why| format!("the signature does not verify: {why}"))?;
    if let Some(expected) = team {
        let actual = signer.team(bundle)?;
        match (expected, actual.as_deref()) {
            (Team::Id(want), Some(got)) if want == got => {
                // The TeamIdentifier alone is text in the signature; the
                // requirement proves Apple issued a Developer ID for it.
                let requirement = super::effects::team_requirement(want)?;
                signer
                    .verify_requirement(bundle, &requirement)
                    .map_err(|why| {
                        format!("it is not a Developer ID signature of team {want}: {why}")
                    })?;
            }
            (Team::AdHoc, None) => {}
            (want, got) => {
                return Err(format!(
                    "it is signed by team {}, not the expected {}",
                    got.unwrap_or("none (ad hoc)"),
                    match want {
                        Team::Id(id) => id.as_str(),
                        Team::AdHoc => "none (ad hoc)",
                    }
                ))
            }
        }
    }
    let exe = bundle.join("Contents").join("MacOS").join(&info.executable);
    if !exe.is_file() {
        return Err(format!("its executable {} is missing", exe.display()));
    }
    Ok(info)
}

/// Dotted numeric versions (`0.6.123`); `None` when either part isn't a
/// number, which the caller treats as a downgrade unless they are equal.
pub(crate) fn compare_versions(a: &str, b: &str) -> Option<Ordering> {
    let parse = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    let (mut a, mut b) = (parse(a)?, parse(b)?);
    let len = a.len().max(b.len());
    a.resize(len, 0);
    b.resize(len, 0);
    Some(a.cmp(&b))
}

/// Refuse installing `new` over `installed` when it is lower (or not
/// comparable and different), unless `allow`.
pub(crate) fn check_downgrade(installed: &str, new: &str, allow: bool) -> Result<(), String> {
    if allow || installed == new {
        return Ok(());
    }
    match compare_versions(new, installed) {
        Some(Ordering::Greater) | Some(Ordering::Equal) => Ok(()),
        _ => Err(format!(
            "{new} is older than the installed {installed}; pass --allow-downgrade to install it anyway"
        )),
    }
}

/// Whether two files have the same bytes.
pub(crate) fn same_bytes(a: &Path, b: &Path) -> Result<bool, String> {
    let open = |p: &Path| fs::File::open(p).map_err(|e| format!("{}: {e}", p.display()));
    let (mut fa, mut fb) = (open(a)?, open(b)?);
    let (la, lb) = (
        fa.metadata().map_err(|e| e.to_string())?.len(),
        fb.metadata().map_err(|e| e.to_string())?.len(),
    );
    if la != lb {
        return Ok(false);
    }
    let (mut ba, mut bb) = (vec![0u8; 1 << 16], vec![0u8; 1 << 16]);
    loop {
        let n = fa.read(&mut ba).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(true);
        }
        fb.read_exact(&mut bb[..n]).map_err(|e| e.to_string())?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}
