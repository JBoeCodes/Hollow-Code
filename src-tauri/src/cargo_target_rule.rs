//! Which `CARGO_TARGET_DIR` values cargo would put somewhere unintended: a
//! relative path (resolved against whatever folder cargo runs in), or one
//! with a segment starting with `~` (a tilde the shell never expanded). On
//! 2026-09-27 agent commands left 17.9 GB of cargo output in literal
//! `src-tauri/~/Library/Caches/…` folders. Shared with `build.rs` (through
//! `#[path]`), which refuses such a build, and mirrored by
//! `scripts/check-cargo-target.mjs`, which the npm gate and bundle run first.

/// Why `value` is unsafe as `CARGO_TARGET_DIR`, or `None`.
pub fn problem(value: &str) -> Option<String> {
    if value.is_empty() {
        return None;
    }
    if value.split('/').any(|segment| segment.starts_with('~')) {
        return Some(format!(
            "CARGO_TARGET_DIR is \"{value}\": a \"~\" there is a literal folder name (the shell \
             didn't expand it). Use an absolute path, e.g. \"$HOME/Library/Caches/…\"."
        ));
    }
    if !value.starts_with('/') {
        return Some(format!(
            "CARGO_TARGET_DIR is \"{value}\": a relative path lands wherever cargo runs. Use an \
             absolute path, e.g. \"$HOME/Library/Caches/…\"."
        ));
    }
    None
}

/// Why a build script's `OUT_DIR` sits under a literal `~` folder (however
/// the target was chosen: the env, `--target-dir` or a cargo config), or `None`.
pub fn out_dir_problem(out_dir: &str) -> Option<String> {
    out_dir
        .split('/')
        .any(|segment| segment.starts_with('~'))
        .then(|| {
            format!(
                "cargo is building into \"{out_dir}\", under a folder literally named \"~\" (an \
                 unexpanded tilde in the target folder). Use an absolute path, and remove that \
                 \"~\" folder."
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_tilde_and_relative_targets() {
        for bad in [
            "~/Library/Caches/hollow-app-a1-target",
            "~",
            "~dev/target",
            "/Users/dev/w/src-tauri/~/Library/Caches/t",
            "target-2",
            "./target",
            "../target",
        ] {
            assert!(problem(bad).is_some(), "{bad}");
        }
        assert!(problem("~/x").unwrap().contains("literal folder"));
        assert!(problem("target").unwrap().contains("relative"));
    }

    #[test]
    fn accepts_absolute_targets_and_unset() {
        for good in [
            "",
            "/Users/dev/Library/Caches/hollow-overnight-a",
            "/Users/dev/Library/Mobile Documents/com~apple~CloudDocs/t",
            "/tmp/a~b",
        ] {
            assert_eq!(problem(good), None, "{good}");
        }
    }

    #[test]
    fn out_dir_under_a_tilde_folder_is_refused() {
        assert!(out_dir_problem(
            "/Users/dev/w/src-tauri/~/Library/Caches/t/debug/build/hollow-1/out"
        )
        .is_some());
        assert_eq!(
            out_dir_problem("/Users/dev/Library/Caches/t/debug/build/hollow-1/out"),
            None
        );
    }
}
