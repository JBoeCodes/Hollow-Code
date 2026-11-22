//! Context roots (CX3): the path rules, saved roots, machine slug and status
//! read, each against real folders in its own `tempfile::TempDir`
//! (canonicalized, since macOS's temp folder sits behind the `/tmp` and
//! `/var` symlinks). A fake home folder stands in for `~`; nothing touches the
//! real one or the app data folder.

use crate::context_roots::{self as cr, Layer, PathError};
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

struct Fixture {
    _tmp: tempfile::TempDir,
    base: PathBuf,
    home: PathBuf,
}

fn fixture() -> Fixture {
    let tmp = tempfile::TempDir::new().unwrap();
    let base = fs::canonicalize(tmp.path()).unwrap();
    let home = base.join("home");
    fs::create_dir_all(&home).unwrap();
    Fixture {
        _tmp: tmp,
        base,
        home,
    }
}

fn s(p: &Path) -> &str {
    p.to_str().unwrap()
}

// ── normalize_root ──────────────────────────────────────────────────────

#[test]
fn a_tilde_root_expands_and_may_not_exist_yet() {
    let f = fixture();
    let got = cr::normalize_root("~/Hollow/Context/personal", &f.home).unwrap();
    assert_eq!(got, f.home.join("Hollow/Context/personal"));
    let got = cr::normalize_root(&format!("{}//Hollow/Context/org/", s(&f.home)), &f.home).unwrap();
    assert_eq!(got, f.home.join("Hollow/Context/org"));
}

#[test]
fn a_relative_or_other_users_root_is_refused() {
    let f = fixture();
    for bad in ["Hollow/Context", "./x", "~other/x"] {
        assert_eq!(
            cr::normalize_root(bad, &f.home),
            Err(PathError::NotAbsolute),
            "{bad}"
        );
    }
    assert_eq!(cr::normalize_root("", &f.home), Err(PathError::Empty));
    assert_eq!(cr::normalize_root("   ", &f.home), Err(PathError::Empty));
}

#[test]
fn dot_dot_and_dot_parts_are_refused_even_when_they_would_land_inside() {
    let f = fixture();
    let inside = format!("{}/Hollow/../Hollow/Context", s(&f.home));
    assert_eq!(
        cr::normalize_root(&inside, &f.home),
        Err(PathError::BadPart)
    );
    assert_eq!(
        cr::normalize_root("~/Hollow/./Context", &f.home),
        Err(PathError::BadPart)
    );
    assert_eq!(cr::normalize_root("~/..", &f.home), Err(PathError::BadPart));
    assert_eq!(
        cr::normalize_root("~/a\0b", &f.home),
        Err(PathError::BadPart)
    );
}

#[test]
fn root_home_and_folders_above_home_are_too_broad() {
    let f = fixture();
    for bad in ["/", "~", "~/", s(&f.base)] {
        assert_eq!(
            cr::normalize_root(bad, &f.home),
            Err(PathError::TooBroad),
            "{bad}"
        );
    }
}

#[test]
fn home_in_another_case_is_still_too_broad() {
    let f = fixture();
    let upper = s(&f.home).to_uppercase();
    assert_eq!(
        cr::normalize_root(&upper, &f.home),
        Err(PathError::TooBroad)
    );
    assert_eq!(
        cr::normalize_root(&s(&f.base).to_uppercase(), &f.home),
        Err(PathError::TooBroad)
    );
}

#[test]
fn a_symlinked_root_is_refused() {
    let f = fixture();
    let real = f.base.join("elsewhere");
    fs::create_dir_all(&real).unwrap();
    let link = f.home.join("ctx");
    symlink(&real, &link).unwrap();
    assert_eq!(
        cr::normalize_root("~/ctx", &f.home),
        Err(PathError::Symlink(link.clone()))
    );
    // A dangling one too.
    let dangling = f.home.join("gone");
    symlink(f.base.join("nothing"), &dangling).unwrap();
    assert_eq!(
        cr::normalize_root("~/gone", &f.home),
        Err(PathError::Symlink(dangling))
    );
}

#[test]
fn a_symlinked_parent_is_resolved_once_so_the_saved_path_has_none() {
    let f = fixture();
    let real = f.base.join("real");
    fs::create_dir_all(&real).unwrap();
    symlink(&real, f.home.join("Hollow")).unwrap();
    let got = cr::normalize_root("~/Hollow/Context/org", &f.home).unwrap();
    assert_eq!(got, real.join("Context/org"));
}

#[test]
fn a_parent_symlink_that_resolves_to_a_broad_folder_is_refused() {
    let f = fixture();
    symlink("/", f.home.join("slash")).unwrap();
    // `~/slash` itself is the symlink: refused as one.
    assert!(matches!(
        cr::normalize_root("~/slash", &f.home),
        Err(PathError::Symlink(_))
    ));
    // Beneath a symlink, the resolved path is checked again: `~/up/home`
    // is the home folder itself.
    symlink(&f.base, f.home.join("up")).unwrap();
    assert_eq!(
        cr::normalize_root("~/up/home", &f.home),
        Err(PathError::TooBroad)
    );
    assert_eq!(
        cr::normalize_root("~/up", &f.home),
        Err(PathError::Symlink(f.home.join("up")))
    );
}

#[test]
fn a_file_where_the_root_or_a_parent_should_be_is_refused() {
    let f = fixture();
    fs::write(f.home.join("file"), "x").unwrap();
    assert_eq!(
        cr::normalize_root("~/file", &f.home),
        Err(PathError::NotFolder(f.home.join("file")))
    );
    assert!(matches!(
        cr::normalize_root("~/file/below", &f.home),
        Err(PathError::NotFolder(_))
    ));
}

// ── check_root and resolve_in_root ──────────────────────────────────────

#[test]
fn check_root_needs_an_existing_folder_with_no_symlink_anywhere() {
    let f = fixture();
    let root = f.home.join("ctx");
    assert_eq!(cr::check_root(&root), Err(PathError::Missing(root.clone())));
    fs::create_dir_all(&root).unwrap();
    assert_eq!(cr::check_root(&root), Ok(()));
    // The same folder reached through a symlink is refused at use time.
    symlink(&f.home, f.base.join("alias")).unwrap();
    let through = f.base.join("alias/ctx");
    assert_eq!(
        cr::check_root(&through),
        Err(PathError::Symlink(f.base.join("alias")))
    );
    // Not in saved form.
    assert_eq!(
        cr::check_root(Path::new("relative")),
        Err(PathError::NotAbsolute)
    );
    assert_eq!(
        cr::check_root(&f.home.join("ctx/../ctx")),
        Err(PathError::BadPart)
    );
    assert_eq!(cr::check_root(Path::new("/")), Err(PathError::BadPart));
}

#[test]
fn a_root_swapped_for_a_symlink_after_saving_is_refused() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(cr::check_root(&root), Ok(()));
    fs::remove_dir(&root).unwrap();
    symlink(f.base.join("other"), &root).unwrap();
    assert_eq!(cr::check_root(&root), Err(PathError::Symlink(root.clone())));
}

#[test]
fn entries_resolve_inside_the_root_and_may_be_missing() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(root.join("home")).unwrap();
    assert_eq!(
        cr::resolve_in_root(&root, "home/macbook.md"),
        Ok(root.join("home/macbook.md"))
    );
    assert_eq!(
        cr::resolve_in_root(&root, "memory/x/MEMORY.md"),
        Ok(root.join("memory/x/MEMORY.md"))
    );
}

#[test]
fn entries_that_escape_or_are_malformed_are_refused() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        cr::resolve_in_root(&root, "../secret"),
        Err(PathError::BadPart)
    );
    assert_eq!(
        cr::resolve_in_root(&root, "home/../../secret"),
        Err(PathError::BadPart)
    );
    assert_eq!(
        cr::resolve_in_root(&root, "./home"),
        Err(PathError::BadPart)
    );
    assert_eq!(
        cr::resolve_in_root(&root, "home//x.md"),
        Err(PathError::BadPart)
    );
    assert_eq!(cr::resolve_in_root(&root, "home/"), Err(PathError::BadPart));
    assert_eq!(
        cr::resolve_in_root(&root, "/etc/passwd"),
        Err(PathError::NotAbsolute)
    );
    assert_eq!(cr::resolve_in_root(&root, ""), Err(PathError::Empty));
    assert_eq!(cr::resolve_in_root(&root, "a\0b"), Err(PathError::BadPart));
}

#[test]
fn a_symlink_inside_the_clone_is_refused_wherever_it_sits() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(root.join("home")).unwrap();
    fs::create_dir_all(f.home.join(".ssh")).unwrap();
    fs::write(f.home.join(".ssh/id"), "secret").unwrap();
    // A file symlink, as a cloned repo can carry.
    symlink(f.home.join(".ssh/id"), root.join("home/mac.md")).unwrap();
    assert_eq!(
        cr::resolve_in_root(&root, "home/mac.md"),
        Err(PathError::Symlink(root.join("home/mac.md")))
    );
    // A folder symlink partway down.
    symlink(f.home.join(".ssh"), root.join("memory")).unwrap();
    assert_eq!(
        cr::resolve_in_root(&root, "memory/id"),
        Err(PathError::Symlink(root.join("memory")))
    );
    // A file where a folder part should be.
    fs::write(root.join("plain"), "x").unwrap();
    assert_eq!(
        cr::resolve_in_root(&root, "plain/x"),
        Err(PathError::NotFolder(root.join("plain")))
    );
}

// ── read_in_root ────────────────────────────────────────────────────────

#[test]
fn read_in_root_reads_a_plain_file_and_reports_missing_as_none() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(root.join("home")).unwrap();
    fs::write(root.join("home/mac.md"), "# Mac\n").unwrap();
    assert_eq!(
        cr::read_in_root(&root, "home/mac.md", 4096),
        Ok(Some("# Mac\n".into()))
    );
    assert_eq!(cr::read_in_root(&root, "home/none.md", 4096), Ok(None));
    assert_eq!(cr::read_in_root(&root, "nofolder/none.md", 4096), Ok(None));
    assert_eq!(
        cr::read_in_root(&f.home.join("noroot"), "x.md", 4096),
        Ok(None)
    );
}

#[test]
fn read_in_root_refuses_symlinks_folders_and_oversized_files() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(root.join("home/sub")).unwrap();
    fs::write(f.home.join("secret"), "secret").unwrap();
    symlink(f.home.join("secret"), root.join("home/link.md")).unwrap();
    assert!(matches!(
        cr::read_in_root(&root, "home/link.md", 4096),
        Err(PathError::Symlink(_))
    ));
    assert_eq!(
        cr::read_in_root(&root, "home/sub", 4096),
        Err(PathError::NotFile(root.join("home/sub")))
    );
    fs::write(root.join("home/big.md"), "x".repeat(10)).unwrap();
    assert_eq!(
        cr::read_in_root(&root, "home/big.md", 9),
        Err(PathError::TooLarge(9))
    );
    assert_eq!(
        cr::read_in_root(&root, "home/big.md", 10),
        Ok(Some("x".repeat(10)))
    );
    assert_eq!(
        cr::read_in_root(&root, "../secret", 4096),
        Err(PathError::BadPart)
    );
}

/// The parts of `path` after `/`, as [`cr::read_walk`] takes them.
fn names(path: &Path) -> Vec<String> {
    path.components()
        .skip(1)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect()
}

#[test]
fn the_nofollow_walk_refuses_symlinks_the_check_never_saw() {
    // read_walk skips resolve_in_root, as a symlink swapped in after the
    // check would: each kind must still be refused by the walk itself.
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(root.join("home")).unwrap();
    fs::create_dir_all(f.home.join("secret")).unwrap();
    fs::write(f.home.join("secret/id"), "secret").unwrap();
    fs::write(root.join("home/mac.md"), "ok").unwrap();
    assert_eq!(
        cr::read_walk(&names(&root.join("home/mac.md")), 64),
        Ok(Some("ok".into()))
    );
    // The file itself.
    symlink(f.home.join("secret/id"), root.join("home/link.md")).unwrap();
    assert_eq!(
        cr::read_walk(&names(&root.join("home/link.md")), 64),
        Err(PathError::Symlink(root.join("home/link.md")))
    );
    // A folder partway down.
    symlink(f.home.join("secret"), root.join("memory")).unwrap();
    assert_eq!(
        cr::read_walk(&names(&root.join("memory/id")), 64),
        Err(PathError::Symlink(root.join("memory")))
    );
    // A part of the root.
    symlink(&root, f.home.join("alias")).unwrap();
    assert_eq!(
        cr::read_walk(&names(&f.home.join("alias/home/mac.md")), 64),
        Err(PathError::Symlink(f.home.join("alias")))
    );
    // A file where a folder should be, a folder where the file should be.
    assert_eq!(
        cr::read_walk(&names(&root.join("home/mac.md/x")), 64),
        Err(PathError::NotFolder(root.join("home/mac.md")))
    );
    assert_eq!(
        cr::read_walk(&names(&root.join("home")), 64),
        Err(PathError::NotFile(root.join("home")))
    );
    assert_eq!(
        cr::read_walk(&names(&root.join("home/none.md")), 64),
        Ok(None)
    );
    assert_eq!(
        cr::read_walk(&names(&root.join("home/mac.md")), u64::MAX),
        Ok(Some("ok".into()))
    );
}

#[cfg(unix)]
#[test]
fn read_in_root_does_not_open_a_fifo_and_hang() {
    let f = fixture();
    let root = f.home.join("ctx");
    fs::create_dir_all(&root).unwrap();
    let fifo = root.join("pipe.md");
    let c = std::ffi::CString::new(s(&fifo)).unwrap();
    // SAFETY: a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o600) }, 0);
    assert_eq!(
        cr::read_in_root(&root, "pipe.md", 4096),
        Err(PathError::NotFile(fifo))
    );
}

// ── Saved roots ─────────────────────────────────────────────────────────

#[test]
fn unset_roots_read_as_none_and_a_bad_file_is_unset() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    assert_eq!(cr::settings_at(&path), cr::RootSettings::default());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{not json").unwrap();
    assert_eq!(cr::settings_at(&path), cr::RootSettings::default());
}

#[test]
fn set_root_saves_the_resolved_path_and_unsets_on_blank() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    let saved = cr::set_root_at(
        &path,
        &f.home,
        Layer::Personal,
        Some("~/Hollow/Context/personal"),
    )
    .unwrap();
    assert_eq!(
        saved.personal.as_deref(),
        Some(s(&f.home.join("Hollow/Context/personal")))
    );
    assert_eq!(saved.org, None);
    assert_eq!(cr::settings_at(&path), saved);
    let text = fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("\"personal\"") && text.contains("\"org\": null"),
        "{text}"
    );
    let saved = cr::set_root_at(&path, &f.home, Layer::Personal, Some("  ")).unwrap();
    assert_eq!(saved, cr::RootSettings::default());
    let saved = cr::set_root_at(&path, &f.home, Layer::Personal, None).unwrap();
    assert_eq!(saved, cr::RootSettings::default());
}

#[test]
fn a_refused_root_leaves_the_saved_one_alone() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    cr::set_root_at(&path, &f.home, Layer::Org, Some("~/Hollow/Context/org")).unwrap();
    let before = fs::read_to_string(&path).unwrap();
    assert_eq!(
        cr::set_root_at(&path, &f.home, Layer::Org, Some("~/../x")),
        Err(PathError::BadPart)
    );
    assert_eq!(fs::read_to_string(&path).unwrap(), before);
}

#[test]
fn a_corrupt_settings_file_is_refused_not_overwritten() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{\"personal\": \"/x\", oops").unwrap();
    assert!(matches!(
        cr::set_root_at(&path, &f.home, Layer::Org, Some("~/ctx/org")),
        Err(PathError::Io(_))
    ));
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "{\"personal\": \"/x\", oops"
    );
}

#[test]
fn a_hand_edited_broad_or_overlapping_root_reads_as_none() {
    let f = fixture();
    fs::create_dir_all(f.home.join("ctx/org/inner")).unwrap();
    let home_root = cr::RootSettings {
        personal: Some(s(&f.home).to_string()),
        org: None,
    };
    assert_eq!(cr::root_in(&home_root, Layer::Personal, &f.home), None);
    let nested = cr::RootSettings {
        personal: Some(s(&f.home.join("ctx/org/inner")).to_string()),
        org: Some(s(&f.home.join("ctx/org")).to_string()),
    };
    assert_eq!(cr::root_in(&nested, Layer::Personal, &f.home), None);
    assert_eq!(cr::root_in(&nested, Layer::Org, &f.home), None);
    let apart = cr::RootSettings {
        personal: None,
        org: Some(s(&f.home.join("ctx/org")).to_string()),
    };
    assert_eq!(
        cr::root_in(&apart, Layer::Org, &f.home),
        Some(f.home.join("ctx/org"))
    );
}

#[test]
fn personal_and_org_may_not_be_the_same_or_nested() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    cr::set_root_at(
        &path,
        &f.home,
        Layer::Personal,
        Some("~/Hollow/Context/personal"),
    )
    .unwrap();
    for bad in [
        "~/Hollow/Context/personal",
        "~/Hollow/Context/personal/org",
        "~/Hollow/Context",
        "~/Hollow",
    ] {
        assert_eq!(
            cr::set_root_at(&path, &f.home, Layer::Org, Some(bad)),
            Err(PathError::Overlap),
            "{bad}"
        );
    }
    // APFS is case-insensitive: another case is the same folder.
    for bad in ["~/hollow/context/PERSONAL", "~/HOLLOW"] {
        assert_eq!(
            cr::set_root_at(&path, &f.home, Layer::Org, Some(bad)),
            Err(PathError::Overlap),
            "{bad}"
        );
    }
    // A sibling whose name only starts the same is fine.
    cr::set_root_at(
        &path,
        &f.home,
        Layer::Org,
        Some("~/Hollow/Context/personal-org"),
    )
    .unwrap();
}

#[test]
fn root_is_none_until_the_saved_folder_exists_and_none_if_it_turns_into_a_symlink() {
    let f = fixture();
    let path = f.base.join("app/context-roots.json");
    let saved = cr::set_root_at(&path, &f.home, Layer::Org, Some("~/ctx/org")).unwrap();
    assert_eq!(cr::settings_at(&path).get(Layer::Personal), None);
    let root = f.home.join("ctx/org");
    let resolve = |saved: &cr::RootSettings| cr::root_in(saved, Layer::Org, &f.home);
    assert_eq!(resolve(&saved), None);
    fs::create_dir_all(&root).unwrap();
    assert_eq!(resolve(&saved), Some(root.clone()));
    fs::remove_dir(&root).unwrap();
    symlink(&f.base, &root).unwrap();
    assert_eq!(resolve(&saved), None);
}

#[test]
fn the_default_root_is_under_hollow_context() {
    let home = cr::default_root(Layer::Personal).unwrap();
    assert!(home.ends_with("Hollow/Context/personal"), "{home:?}");
    assert!(cr::default_root(Layer::Org)
        .unwrap()
        .ends_with("Hollow/Context/org"));
    assert_eq!(Layer::ALL.map(Layer::as_str), ["personal", "org"]);
}

// ── Machine slug ────────────────────────────────────────────────────────

#[test]
fn slugs_are_lowercase_letters_digits_and_single_dashes() {
    for good in [
        "macbook-pro",
        "m4",
        "a",
        "x1-carbon-2",
        &"a".repeat(cr::SLUG_MAX),
    ] {
        assert_eq!(cr::valid_slug(good), Ok(()), "{good}");
    }
    let long = "a".repeat(cr::SLUG_MAX + 1);
    for bad in [
        "",
        "-mac",
        "mac-",
        "mac--mini",
        "MacBook",
        "mac book",
        "mac_book",
        "mac.md",
        "../mac",
        "mac/x",
        "mac\0",
        "macé",
        &long,
    ] {
        assert!(
            matches!(cr::valid_slug(bad), Err(PathError::BadSlug(_))),
            "{bad:?}"
        );
    }
    assert!(cr::home_file("../../etc").is_err());
    assert_eq!(cr::home_file("macbook-pro").unwrap(), "home/macbook-pro.md");
}

#[test]
fn a_slug_is_suggested_from_the_macs_name() {
    assert_eq!(
        cr::suggest_slug("Ada\u{2019}s MacBook Pro").as_deref(),
        Some("macbook-pro")
    );
    assert_eq!(
        cr::suggest_slug("Ada's Mac mini").as_deref(),
        Some("mac-mini")
    );
    assert_eq!(
        cr::suggest_slug("  Studio (2) ").as_deref(),
        Some("studio-2")
    );
    assert_eq!(cr::suggest_slug("\u{1F600}"), None);
    // The first owner mark wins, whichever kind it is.
    assert_eq!(
        cr::suggest_slug("Ada\u{2019}s Bob's Mac").as_deref(),
        Some("bob-s-mac")
    );
    let long = cr::suggest_slug(&"word ".repeat(30)).unwrap();
    assert!(
        long.len() <= cr::SLUG_MAX && cr::valid_slug(&long).is_ok(),
        "{long}"
    );
}

#[test]
fn machine_json_round_trips_and_keeps_unknown_fields() {
    let f = fixture();
    let path = f.base.join("config/hollow/machine.json");
    assert_eq!(cr::machine_slug_at(&path), None);
    assert_eq!(
        cr::set_machine_slug_at(&path, Some("macbook-pro")),
        Ok(Some("macbook-pro".into()))
    );
    assert_eq!(cr::machine_slug_at(&path).as_deref(), Some("macbook-pro"));
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"version": 1, "slug": "macbook-pro"})
    );
    fs::write(
        &path,
        r#"{"version": 1, "slug": "old", "homeFile": "home/old.md"}"#,
    )
    .unwrap();
    cr::set_machine_slug_at(&path, Some("new")).unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"version": 1, "slug": "new", "homeFile": "home/old.md"})
    );
    assert_eq!(cr::set_machine_slug_at(&path, None), Ok(None));
    assert_eq!(cr::machine_slug_at(&path), None);
}

#[test]
fn a_bad_slug_or_bad_machine_file_is_refused_and_left_alone() {
    let f = fixture();
    let path = f.base.join("config/hollow/machine.json");
    assert!(matches!(
        cr::set_machine_slug_at(&path, Some("Mac Mini")),
        Err(PathError::BadSlug(_))
    ));
    assert!(!path.exists());
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{hand edited").unwrap();
    assert!(matches!(
        cr::set_machine_slug_at(&path, Some("mac")),
        Err(PathError::Io(_))
    ));
    assert_eq!(fs::read_to_string(&path).unwrap(), "{hand edited");
    // A file holding an invalid slug reads as no slug.
    fs::write(&path, r#"{"slug": "../x"}"#).unwrap();
    assert_eq!(cr::machine_slug_at(&path), None);
}

/// M2: the machine id is written once, kept by the slug's writer (which is
/// how a Hollow that knows only the slug writes the file: `MachineFile`
/// keeps unknown fields), and never replaced; a hand-edited file is refused
/// and left alone.
#[test]
fn the_machine_id_is_written_once_and_kept_by_every_writer() {
    let f = fixture();
    let path = f.base.join("config/hollow/machine.json");
    assert_eq!(cr::machine_id_at(&path), None);
    let id = cr::ensure_machine_id_at(&path).unwrap();
    assert!(id.starts_with("m-") && id.len() == 18, "{id}");
    assert!(id[2..]
        .chars()
        .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    assert_eq!(cr::machine_id_at(&path).as_deref(), Some(id.as_str()));
    assert_eq!(cr::ensure_machine_id_at(&path).unwrap(), id);
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    // The slug's writer, setting and removing a slug, keeps the id.
    cr::set_machine_slug_at(&path, Some("boe-mbp")).unwrap();
    cr::set_machine_slug_at(&path, Some("other-name")).unwrap();
    assert_eq!(cr::machine_id_at(&path).as_deref(), Some(id.as_str()));
    assert_eq!(cr::ensure_machine_id_at(&path).unwrap(), id);
    let text = fs::read_to_string(&path).unwrap();
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(value["slug"], "other-name");
    assert_eq!(value["machineId"], id.as_str());
    // A file that already has the slug and a later field gets an id and
    // keeps both.
    fs::write(&path, r#"{"version": 1, "slug": "mini", "later": [1]}"#).unwrap();
    let fresh = cr::ensure_machine_id_at(&path).unwrap();
    assert_ne!(fresh, id);
    let value: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["slug"], "mini");
    assert_eq!(value["later"], serde_json::json!([1]));
    // An id Hollow never writes is replaced; a hand edit is refused.
    fs::write(&path, r#"{"machineId": "M-NOT-OURS"}"#).unwrap();
    assert_eq!(cr::machine_id_at(&path), None);
    assert!(cr::ensure_machine_id_at(&path).unwrap().starts_with("m-"));
    fs::write(&path, "{hand edited").unwrap();
    assert!(cr::ensure_machine_id_at(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "{hand edited");
}

// ── Status ──────────────────────────────────────────────────────────────

fn git(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn status_reports_missing_plain_and_cloned_folders() {
    let f = fixture();
    let root = f.home.join("ctx");
    let st = cr::status_of(Layer::Personal, &root, false).await;
    assert!(
        !st.exists && !st.git_clone && !st.configured && st.problem.is_none(),
        "{st:?}"
    );
    fs::create_dir_all(&root).unwrap();
    let st = cr::status_of(Layer::Personal, &root, true).await;
    assert!(
        st.exists && !st.git_clone && st.last_commit_at.is_none() && st.problem.is_none(),
        "{st:?}"
    );
    git(&root, &["init", "-q"]);
    let st = cr::status_of(Layer::Personal, &root, true).await;
    assert!(st.git_clone && st.last_commit_at.is_none(), "{st:?}");
    fs::write(root.join("INDEX.md"), "x").unwrap();
    git(&root, &["add", "INDEX.md"]);
    git(&root, &["commit", "-q", "-m", "seed"]);
    let st = cr::status_of(Layer::Personal, &root, true).await;
    assert!(st.git_clone, "{st:?}");
    let when = st.last_commit_at.unwrap();
    assert!(
        chrono::DateTime::parse_from_rfc3339(&when).is_ok(),
        "{when}"
    );
    let json = serde_json::to_value(cr::status_of(Layer::Org, &root, true).await).unwrap();
    for key in [
        "layer",
        "path",
        "configured",
        "exists",
        "gitClone",
        "lastCommitAt",
        "problem",
    ] {
        assert!(json.get(key).is_some(), "{key} in {json}");
    }
    assert_eq!(json["layer"], "org");
}

#[tokio::test]
async fn status_flags_a_symlinked_root_or_dot_git_and_a_folder_inside_another_repo() {
    let f = fixture();
    let real = f.base.join("real");
    fs::create_dir_all(&real).unwrap();
    let link = f.home.join("ctx");
    symlink(&real, &link).unwrap();
    let st = cr::status_of(Layer::Org, &link, true).await;
    assert!(
        st.exists && !st.git_clone && st.problem.as_deref().unwrap().contains("symlink"),
        "{st:?}"
    );

    let outer = f.home.join("outer");
    fs::create_dir_all(outer.join("inner")).unwrap();
    git(&outer, &["init", "-q"]);
    // No .git of its own: not a clone, and git is never asked.
    let st = cr::status_of(Layer::Org, &outer.join("inner"), true).await;
    assert!(st.exists && !st.git_clone && st.problem.is_none(), "{st:?}");

    let dotlink = f.home.join("dotlink");
    fs::create_dir_all(&dotlink).unwrap();
    symlink(outer.join(".git"), dotlink.join(".git")).unwrap();
    let st = cr::status_of(Layer::Org, &dotlink, true).await;
    assert!(
        !st.git_clone && st.problem.as_deref().unwrap().contains("symlink"),
        "{st:?}"
    );
}
