//! Real bundles: a tiny `Hollow.app` (Info.plist and a shell-script
//! executable) made in `$TMPDIR` and signed ad hoc with `codesign -s -`,
//! then checked by the real `codesign` and swapped with the real `ditto`
//! and `renamex_np`. Processes, `open` and time stay fake: nothing is
//! launched and nothing outside the temp folder is touched.

use super::effects::{Codesign, Deps, RealFs};
use super::run::{self, GoBackArgs};
use super::tests::{info_plist, H};
use super::verify::{self, Team};
use super::{Exit, DEFAULT_TEAM};
use std::path::Path;
use std::process::Command;

fn signed_bundle(at: &Path, version: &str) {
    std::fs::create_dir_all(at.join("Contents/MacOS")).unwrap();
    std::fs::write(
        at.join("Contents/Info.plist"),
        info_plist(version, super::BUNDLE_ID),
    )
    .unwrap();
    let exe = at.join("Contents/MacOS/hollow");
    std::fs::write(&exe, format!("#!/bin/sh\necho hollow {version}\n")).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(at)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "codesign: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn deps(h: &H) -> Deps<'_> {
    Deps {
        fs: &RealFs,
        signer: &Codesign,
        procs: &h.procs,
        launcher: &h.launcher,
        clock: &*h.clock,
        checkpoint: &|_| {},
    }
}

#[test]
fn real_codesign_accepts_an_ad_hoc_bundle_only_when_ad_hoc_is_expected() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Hollow.app");
    signed_bundle(&app, "0.6.2");
    let info = verify::check_bundle(&app, &Codesign, Some(&Team::AdHoc), Some("0.6.2")).unwrap();
    assert_eq!(info.executable, "hollow");
    let team = verify::check_bundle(
        &app,
        &Codesign,
        Some(&Team::Id(DEFAULT_TEAM.into())),
        Some("0.6.2"),
    );
    assert!(team.unwrap_err().contains("not the expected TEAM000000"));
    // Changing a sealed file breaks the signature.
    std::fs::write(
        app.join("Contents/MacOS/hollow"),
        "#!/bin/sh\necho tampered\n",
    )
    .unwrap();
    let tampered = verify::check_bundle(&app, &Codesign, Some(&Team::AdHoc), Some("0.6.2"));
    assert!(tampered.unwrap_err().contains("signature does not verify"));
}

#[test]
fn one_real_swap_with_ditto_and_renamex_then_go_back() {
    let h = H::new();
    signed_bundle(&h.app(), "0.6.1");
    let staged = h.root.join("download/Hollow.app");
    signed_bundle(&staged, "0.6.2");
    let mut args = h.args(&staged, "0.6.2");
    args.team = Team::AdHoc;
    let out = run::install(&args, &deps(&h));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    let version = |p: &Path| verify::read_info(p).unwrap().version;
    assert_eq!(version(&h.app()), "0.6.2");
    assert_eq!(version(&h.layout().previous), "0.6.1");
    // The copy keeps its signature (a script's lives in extended attributes).
    verify::check_bundle(&h.app(), &Codesign, Some(&Team::AdHoc), Some("0.6.2")).unwrap();
    verify::check_bundle(&h.layout().previous, &Codesign, None, None).unwrap();
    assert!(!h.layout().staging.exists() && !h.layout().discard.exists());

    h.world.borrow_mut().procs.clear();
    let back = GoBackArgs {
        app: h.app(),
        state_dir: h.state.clone(),
        relaunch: false,
        install_id: "20261115-120000-go-back".into(),
        timing: Default::default(),
        restore: None,
    };
    let out = run::go_back(&back, &deps(&h));
    assert_eq!(out.exit, Exit::Ok, "{}", out.message);
    assert_eq!(version(&h.app()), "0.6.1");
    assert_eq!(version(&h.layout().previous), "0.6.2");
}

#[test]
fn real_codesign_parses_the_developer_id_requirement_and_an_ad_hoc_bundle_fails_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("Hollow.app");
    signed_bundle(&app, "0.6.2");
    let requirement = super::effects::team_requirement(DEFAULT_TEAM).unwrap();
    use super::effects::Signer;
    let err = Codesign.verify_requirement(&app, &requirement).unwrap_err();
    // A requirement codesign could not parse says so; this one parses and
    // is not satisfied.
    assert!(err.contains("satisfy"), "{err}");
}
