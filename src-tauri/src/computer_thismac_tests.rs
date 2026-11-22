//! This Mac's policy (CU6 part 1): which app a call names, the default deny
//! list and the person's allowances, and the Driver's bounded manifest.

use super::*;

#[test]
fn a_call_names_its_app_by_bundle_id_or_name_and_oddities_are_bad() {
    assert_eq!(
        look(&json!({ "bundle_id": "com.apple.Notes" })),
        Look::App("com.apple.notes".into())
    );
    assert_eq!(
        look(&json!({ "bundleId": " com.apple.Notes " })),
        Look::App("com.apple.notes".into())
    );
    assert_eq!(
        look(&json!({ "app": "Notes" })),
        Look::App("name:notes".into())
    );
    // A bundle id and a name together are two claims: refused.
    assert_eq!(
        look(&json!({ "app": "Notes", "bundle_id": "com.apple.notes" })),
        Look::Bad
    );
    assert_eq!(look(&json!({ "x": 1 })), Look::Unnamed);
    assert_eq!(look(&json!({ "bundle_id": "  " })), Look::Unnamed);
    for bad in ["a/b", "a;b", "../x", "$(x)", &"a".repeat(200)] {
        assert_eq!(look(&json!({ "bundle_id": bad })), Look::Bad, "{bad}");
    }
}

#[test]
fn run_steps_is_judged_by_every_step() {
    let steps = json!({ "steps": [
        { "tool": "click", "arguments": { "bundle_id": "a.b" } },
        { "tool": "click", "arguments": {} },
    ] });
    assert_eq!(
        looks_for_call("run_steps", &steps),
        [Look::App("a.b".into()), Look::Unnamed]
    );
    assert_eq!(
        looks_for_call("click", &json!({ "app": "x" })),
        [Look::App("name:x".into())]
    );
}

#[test]
fn the_default_deny_list_holds_until_the_person_allows_one() {
    let none = Policy::default();
    for app in [
        "com.apple.terminal",
        "com.googlecode.iterm2",
        "com.apple.systempreferences",
        "com.apple.keychainaccess",
        "com.1password.1password",
        "com.agilebits.onepassword7",
        "org.keepassxc.keepassxc",
        "com.hollow.app",
        "name:terminal",
        "name:system settings",
        "name:hollow",
    ] {
        assert!(none.denied(app), "{app}");
    }
    for app in [
        "com.apple.notes",
        "com.apple.calculator",
        "name:notes",
        "com.apple.terminalx",
    ] {
        assert!(!none.denied(app), "{app}");
    }
    let allow = Policy {
        allowed: ["com.apple.terminal".to_string()].into(),
    };
    assert!(!allow.denied("com.apple.terminal"));
    assert!(
        allow.denied("com.apple.keychainaccess"),
        "one allowance is not all"
    );
}

#[test]
fn the_policy_file_only_ever_allows_what_it_names_and_a_bad_file_widens_nothing() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(Policy::load_from(dir.path()), Policy::default());
    let write = |s: &str| std::fs::write(dir.path().join(POLICY_FILE), s).unwrap();
    write(r#"{"allowDenied":["com.apple.Terminal"," name:Terminal ","bad/id",7]}"#);
    let p = Policy::load_from(dir.path());
    assert_eq!(p.allowed.len(), 2);
    assert!(!p.denied("com.apple.terminal") && !p.denied("name:terminal"));
    for broken in ["not json", r#"{"allowDenied":"all"}"#, r#"[]"#] {
        write(broken);
        assert_eq!(Policy::load_from(dir.path()), Policy::default(), "{broken}");
    }
}

#[test]
fn the_manifest_names_apps_and_the_display_and_carries_the_run_bounds() {
    let mut m = Manifest::new(" run-1 ");
    m.apps.insert("com.apple.notes".into());
    m.apps.insert("name:calculator".into());
    m.display = true;
    let v = m.to_json();
    assert_eq!(v["mode"], "bounded");
    assert_eq!(v["run"], "run-1");
    assert_eq!(v["apps"], json!(["com.apple.notes"]));
    assert_eq!(v["app_names"], json!(["calculator"]));
    assert_eq!(v["desktop"]["display"], json!(true));
    assert_eq!(v["expires_after"], 1800);
    assert_eq!(v["idle_timeout"], 300);
    let empty = Manifest::empty("warm-up").to_json();
    assert_eq!(empty["apps"], json!([]));
    assert_eq!(empty["desktop"]["display"], json!(false));
}

#[test]
fn only_one_app_may_be_named_and_no_other_way_to_aim_at_a_process() {
    let bad = |v: Value| assert_eq!(look(&v), Look::Bad, "{v}");
    // A bundle id and a name together, two different ids, repeats that differ.
    bad(json!({ "bundle_id": "com.apple.notes", "app": "Terminal" }));
    bad(json!({ "bundle_id": "com.apple.notes", "bundleId": "com.apple.terminal" }));
    bad(json!({ "app": "Notes", "app_name": "Terminal" }));
    // Non-string values under a naming key.
    bad(json!({ "bundle_id": 7 }));
    bad(json!({ "app": ["Notes"] }));
    // Another way to aim the call.
    for key in ["pid", "window_id", "windowId", "process", "window"] {
        bad(json!({ "bundle_id": "com.apple.notes", key: 1 }));
    }
    // The same app under several keys is one app.
    assert_eq!(
        look(&json!({ "bundle_id": "com.apple.Notes", "bundleId": "COM.apple.notes" })),
        Look::App("com.apple.notes".into())
    );
    assert_eq!(
        look(&json!({ "app": "Google  Chrome.app", "app_name": "google chrome" })),
        Look::App("name:google chrome".into())
    );
    // A null target key is not a target.
    assert_eq!(
        look(&json!({ "bundle_id": "a.b", "pid": null })),
        Look::App("a.b".into())
    );
}

#[test]
fn names_are_compared_as_names_and_sub_bundles_follow_their_app() {
    let none = Policy::default();
    let name = |n: &str| match look(&json!({ "app": n })) {
        Look::App(a) => a,
        other => panic!("{n}: {other:?}"),
    };
    for n in [
        "Terminal.app",
        "TERMINAL",
        "1Password 7",
        "1Password 8",
        "Bitwarden Desktop",
        "Keychain Access.app",
        "System Settings.app",
        "Hollow Dev",
        "iTerm2",
    ] {
        assert!(none.denied(&name(n)), "{n}");
    }
    for n in ["Notes", "Terminal Velocity Game", "Hollowed"] {
        // A name that only begins with a denied word in the same token is
        // another app ("Hollowed"); one that starts a new word is denied.
        let denied = none.denied(&name(n));
        assert_eq!(denied, n == "Terminal Velocity Game", "{n}");
    }
    for id in [
        "com.hollow.app.candidate",
        "com.apple.terminal.helper",
        "com.apple.systempreferences.x",
    ] {
        assert!(none.denied(id), "{id}");
    }
    assert!(!none.denied("com.hollow.apped"));
    // The person's name allowance is normalized the same way.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join(POLICY_FILE),
        r#"{"allowDenied":["name:Terminal.app"]}"#,
    )
    .unwrap();
    let p = Policy::load_from(dir.path());
    assert!(!p.denied(&name("terminal")));
}

#[test]
fn settings_allows_and_blocks_a_whole_group_and_the_file_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Policy::default();
    assert!(p.groups().iter().all(|(_, allowed)| !allowed));
    assert!(p.set_group("1Password", true));
    assert!(!p.set_group("Not an app", true));
    assert!(!p.denied("com.1password.1password") && !p.denied("com.agilebits.onepassword7"));
    assert!(!p.denied("name:1password 8"), "its names continue");
    assert!(p.denied("com.bitwarden.desktop"), "one group is not all");
    assert!(p.groups().iter().any(|(l, a)| *l == "1Password" && *a));
    p.save_to(dir.path()).unwrap();
    let back = Policy::load_from(dir.path());
    assert_eq!(back, p);
    assert!(!back.denied("com.1password.1password"));
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.path().join(POLICY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    let mut back = back;
    back.set_group("1Password", false);
    assert!(back.denied("com.1password.1password") && back.allowed.is_empty());
    back.save_to(dir.path()).unwrap();
    assert!(Policy::load_from(dir.path()).denied("com.1password.1password"));
}

#[test]
fn a_loose_wildcard_in_the_file_allows_nothing() {
    let dir = tempfile::tempdir().unwrap();
    for entry in ["*", "com*", "com.apple.*x", "name:*"] {
        std::fs::write(
            dir.path().join(POLICY_FILE),
            json!({ "allowDenied": [entry] }).to_string(),
        )
        .unwrap();
        let p = Policy::load_from(dir.path());
        assert!(
            p.denied("com.apple.terminal") && p.denied("com.hollow.app"),
            "{entry}"
        );
    }
    // A family wildcard is fine, and only for that family.
    std::fs::write(
        dir.path().join(POLICY_FILE),
        json!({ "allowDenied": ["com.1password.*"] }).to_string(),
    )
    .unwrap();
    let p = Policy::load_from(dir.path());
    assert!(!p.denied("com.1password.1password") && p.denied("com.agilebits.onepassword7"));
}

#[test]
fn an_id_given_as_a_name_or_a_name_as_an_id_is_still_denied_and_aliases_are_refused() {
    let none = Policy::default();
    for args in [
        json!({ "app": "com.apple.Terminal" }),
        json!({ "app": "com.agilebits.onepassword7" }),
        json!({ "bundle_id": "Terminal" }),
        json!({ "bundle_id": "1Password 8" }),
    ] {
        match look(&args) {
            Look::App(a) => assert!(none.denied(&a), "{args}: {a}"),
            other => panic!("{args}: {other:?}"),
        }
    }
    // The person's allowance works across the same forms.
    let mut p = Policy::default();
    p.set_group("Terminal", true);
    for args in [
        json!({ "app": "com.apple.Terminal" }),
        json!({ "bundle_id": "Terminal" }),
        json!({ "app": "Terminal.app" }),
    ] {
        let Look::App(a) = look(&args) else {
            panic!("{args}")
        };
        assert!(!p.denied(&a), "{args}");
    }
    // Aliases the Driver might honour are refused, nested targets too.
    for args in [
        json!({ "bundle_id": "com.apple.safari", "bundleID": "com.apple.terminal" }),
        json!({ "bundle_id": "com.apple.safari", "appName": "Terminal" }),
        json!({ "bundle_id": "com.apple.safari", "application": "Terminal" }),
        json!({ "bundle_id": "com.apple.safari", "target": "x" }),
        json!({ "bundle_id": "com.apple.safari", "name": "x" }),
        json!({ "bundle_id": "com.apple.safari", "target": { "bundle_id": "com.apple.terminal" } }),
        json!({ "bundle_id": "com.apple.safari", "options": { "a": 1 } }),
    ] {
        assert_eq!(look(&args), Look::Bad, "{args}");
    }
    // Plain arguments are fine, and so are null aliases.
    assert_eq!(
        look(
            &json!({ "bundle_id": "a.b", "x": 1, "y": 2, "text": "hi", "keys": ["cmd", "return"], "target": null })
        ),
        Look::App("a.b".into())
    );
    // The terminals the person uses are on the list.
    for id in [
        "com.mitchellh.ghostty",
        "dev.warp.warp-stable",
        "org.alacritty",
        "net.kovidgoyal.kitty",
        "com.github.wez.wezterm",
        "com.apple.scripteditor2",
        "com.apple.passwords",
        "com.trycua.driver",
    ] {
        assert!(none.denied(id), "{id}");
    }
}
