//! Unpack safety and the staging folder, on archives built here (hostile
//! names are written straight into the header, past the `tar` crate's own
//! checks).

use super::stage::Staging;
use super::unpack::{self, link_stays_inside, Limits};
use std::io::Write;
use std::path::Path;

/// One member: name, kind, contents (file) or link target (symlink).
pub(crate) enum M<'a> {
    Dir(&'a str),
    File(&'a str, &'a [u8], u32),
    Link(&'a str, &'a str),
    Hard(&'a str, &'a str),
    Fifo(&'a str),
}

fn raw_header(name: &str, kind: tar::EntryType, size: u64, mode: u32, link: &str) -> tar::Header {
    let mut h = tar::Header::new_ustar();
    {
        let old = h.as_old_mut();
        old.name = [0; 100];
        old.name[..name.len()].copy_from_slice(name.as_bytes());
        old.linkname = [0; 100];
        old.linkname[..link.len()].copy_from_slice(link.as_bytes());
    }
    h.set_entry_type(kind);
    h.set_size(size);
    h.set_mode(mode);
    h.set_mtime(1_700_000_000);
    h.set_cksum();
    h
}

/// A `.tar.gz` of `members`, names as given.
pub(crate) fn targz(members: &[M]) -> Vec<u8> {
    let mut tar = Vec::new();
    {
        let mut b = tar::Builder::new(&mut tar);
        for m in members {
            let (h, data): (tar::Header, &[u8]) = match m {
                M::Dir(n) => (raw_header(n, tar::EntryType::Directory, 0, 0o755, ""), &[]),
                M::File(n, d, mode) => (
                    raw_header(n, tar::EntryType::Regular, d.len() as u64, *mode, ""),
                    d,
                ),
                M::Link(n, t) => (raw_header(n, tar::EntryType::Symlink, 0, 0o755, t), &[]),
                M::Hard(n, t) => (raw_header(n, tar::EntryType::Link, 0, 0o644, t), &[]),
                M::Fifo(n) => (raw_header(n, tar::EntryType::Fifo, 0, 0o644, ""), &[]),
            };
            b.append(&h, data).unwrap();
        }
        b.finish().unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    gz.finish().unwrap()
}

/// A plausible app layout.
pub(crate) fn app_members() -> Vec<M<'static>> {
    vec![
        M::Dir("Hollow.app/"),
        M::Dir("Hollow.app/Contents/"),
        M::File("Hollow.app/Contents/Info.plist", b"<plist/>", 0o644),
        M::Dir("Hollow.app/Contents/MacOS/"),
        M::File("Hollow.app/Contents/MacOS/hollow", b"#!/bin/sh\n", 0o4755),
        M::Dir("Hollow.app/Contents/Frameworks/X.framework/Versions/A/"),
        M::Link(
            "Hollow.app/Contents/Frameworks/X.framework/Versions/Current",
            "A",
        ),
    ]
}

fn unpack(members: &[M]) -> Result<std::path::PathBuf, String> {
    let dir = tempfile::tempdir().unwrap().keep();
    unpack::unpack_tar_gz(&targz(members), &dir.join("out"), Limits::default())
}

#[test]
fn a_normal_app_unpacks_with_its_inside_links_and_no_set_id_bits() {
    use std::os::unix::fs::PermissionsExt;
    let app = unpack(&app_members()).unwrap();
    assert!(app.ends_with("out/Hollow.app"));
    let exe = app.join("Contents/MacOS/hollow");
    assert_eq!(std::fs::read(&exe).unwrap(), b"#!/bin/sh\n");
    let mode = std::fs::metadata(&exe).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode & 0o111, 0o111, "the executable bit survives");
    assert_eq!(mode & 0o7000, 0, "set-id bits are dropped");
    let current = app.join("Contents/Frameworks/X.framework/Versions/Current");
    assert_eq!(std::fs::read_link(current).unwrap(), Path::new("A"));
}

#[test]
fn path_traversal_absolute_paths_and_strays_are_refused() {
    for (members, why) in [
        (
            vec![
                M::Dir("Hollow.app/"),
                M::File("Hollow.app/../evil", b"x", 0o644),
            ],
            "climbs out",
        ),
        (vec![M::File("../evil", b"x", 0o644)], "climbs out"),
        (vec![M::File("/tmp/evil", b"x", 0o644)], "absolute"),
        (
            vec![M::File("Other.app/x", b"x", 0o644)],
            "outside Hollow.app",
        ),
        (vec![M::File("evil", b"x", 0o644)], "outside Hollow.app"),
        (vec![M::File("Hollow.app", b"x", 0o644)], "not a folder"),
        (vec![M::Dir("Hollow.app/")], ""),
    ] {
        let result = unpack(&members);
        if why.is_empty() {
            assert!(result.is_ok());
            continue;
        }
        let err = result.unwrap_err();
        assert!(err.contains(why), "{why}: {err}");
    }
}

#[test]
fn symlinks_that_leave_the_app_are_refused() {
    for target in ["/etc", "../../..", "../..", "../../Other.app", ""] {
        let mut members = app_members();
        members.push(M::Link("Hollow.app/Contents/escape", target));
        let err = unpack(&members).unwrap_err();
        assert!(
            err.contains("points outside")
                || err.contains("symlink to nothing")
                || err.contains("symlink"),
            "{target}: {err}"
        );
    }
    assert!(link_stays_inside(
        Path::new("Hollow.app/a/b"),
        Path::new("../c")
    ));
    assert!(link_stays_inside(
        Path::new("Hollow.app/b"),
        Path::new("../Hollow.app/c")
    ));
    assert!(!link_stays_inside(
        Path::new("Hollow.app/b"),
        Path::new("../c")
    ));
    assert!(!link_stays_inside(
        Path::new("Hollow.app/b"),
        Path::new("/Applications")
    ));
}

#[test]
fn nothing_is_written_through_an_earlier_symlink() {
    // An inside link, then a member written through it: refused, even though
    // the link itself stays inside (no write ever follows a link).
    let mut members = app_members();
    members.push(M::Link("Hollow.app/Contents/via", "MacOS"));
    members.push(M::File("Hollow.app/Contents/via/hollow", b"swap", 0o755));
    let err = unpack(&members).unwrap_err();
    assert!(err.contains("through a symlink"), "{err}");
    // The same name twice, the second over a link.
    let mut members = app_members();
    members.push(M::File(
        "Hollow.app/Contents/Frameworks/X.framework/Versions/Current",
        b"x",
        0o644,
    ));
    assert!(unpack(&members).unwrap_err().contains("through a symlink"));
}

#[test]
fn hard_links_fifos_and_oversized_archives_are_refused() {
    let mut members = app_members();
    members.push(M::Hard(
        "Hollow.app/Contents/h",
        "Hollow.app/Contents/Info.plist",
    ));
    assert!(unpack(&members)
        .unwrap_err()
        .contains("not a file, folder or symlink"));
    let mut members = app_members();
    members.push(M::Fifo("Hollow.app/Contents/f"));
    assert!(unpack(&members)
        .unwrap_err()
        .contains("not a file, folder or symlink"));
    let dir = tempfile::tempdir().unwrap();
    let small = Limits {
        max_bytes: 4,
        max_entries: 100,
    };
    assert!(
        unpack::unpack_tar_gz(&targz(&app_members()), &dir.path().join("a"), small)
            .unwrap_err()
            .contains("bytes")
    );
    let few = Limits {
        max_bytes: 1 << 20,
        max_entries: 2,
    };
    assert!(
        unpack::unpack_tar_gz(&targz(&app_members()), &dir.path().join("b"), few)
            .unwrap_err()
            .contains("members")
    );
    assert!(unpack::unpack_tar_gz(b"not gzip", &dir.path().join("c"), Limits::default()).is_err());
}

#[test]
fn a_staging_folder_that_isnt_empty_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("stray"), b"x").unwrap();
    let err =
        unpack::unpack_tar_gz(&targz(&app_members()), dir.path(), Limits::default()).unwrap_err();
    assert!(err.contains("isn't empty"));
}

#[test]
fn staging_marks_ready_only_after_verify_and_reuses_and_prunes() {
    let dir = tempfile::tempdir().unwrap();
    let staging = Staging::new(dir.path().join("staging"));
    let archive = targz(&app_members());
    // A failed verify leaves nothing.
    let err = staging
        .stage("0.6.5", &archive, Limits::default(), "now", |_| {
            Err::<(), _>("bad team".to_string())
        })
        .unwrap_err();
    assert_eq!(err, "bad team");
    assert!(staging.ready("0.6.5").is_none());
    assert!(!staging.root().join("0.6.5").exists());
    // A good one is ready, and found again without another unpack.
    let (bundle, ()) = staging
        .stage("0.6.5", &archive, Limits::default(), "now", |b| {
            assert!(b.join("Contents/Info.plist").is_file());
            Ok(())
        })
        .unwrap();
    assert_eq!(staging.ready("0.6.5"), Some(bundle.clone()));
    assert!(staging.ready("0.6.6").is_none());
    {
        use std::os::unix::fs::PermissionsExt;
        for dir in [staging.root().to_path_buf(), staging.root().join("0.6.5")] {
            let mode = std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", dir.display());
        }
    }
    // Older leftovers go; the kept version stays.
    std::fs::create_dir_all(staging.root().join("0.6.4/Hollow.app")).unwrap();
    std::fs::write(staging.root().join("stray.tmp"), b"x").unwrap();
    staging.prune(Some("0.6.5"));
    let left: Vec<_> = std::fs::read_dir(staging.root())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(left, vec!["0.6.5".to_string()]);
    // A version that isn't dotted numbers never names a folder.
    assert!(staging
        .stage("../x", &archive, Limits::default(), "now", |_| Ok(()))
        .is_err());
    staging.discard("0.6.5");
    assert!(staging.ready("0.6.5").is_none());
}

#[test]
fn the_size_cap_counts_a_pax_size_not_the_header_field() {
    // A PAX record says the member is 64 bytes; the ustar header says 0.
    let data = [b'x'; 64];
    let mut tar = Vec::new();
    {
        let mut b = tar::Builder::new(&mut tar);
        b.append(
            &raw_header("Hollow.app/", tar::EntryType::Directory, 0, 0o755, ""),
            &[][..],
        )
        .unwrap();
        let record = b"11 size=64\n";
        b.append(
            &raw_header(
                "PaxHeader",
                tar::EntryType::XHeader,
                record.len() as u64,
                0o644,
                "",
            ),
            &record[..],
        )
        .unwrap();
        b.append(
            &raw_header("Hollow.app/big", tar::EntryType::Regular, 0, 0o644, ""),
            &data[..],
        )
        .unwrap();
        b.finish().unwrap();
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(&tar).unwrap();
    let archive = gz.finish().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let small = Limits {
        max_bytes: 32,
        max_entries: 100,
    };
    let err = unpack::unpack_tar_gz(&archive, &dir.path().join("a"), small).unwrap_err();
    assert!(err.contains("more than 32 bytes"), "{err}");
    // Under a cap it allows, the member unpacks with all 64 bytes.
    let app = unpack::unpack_tar_gz(&archive, &dir.path().join("b"), Limits::default()).unwrap();
    assert_eq!(std::fs::read(app.join("big")).unwrap().len(), 64);
}
