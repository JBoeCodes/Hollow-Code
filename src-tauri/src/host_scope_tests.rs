//! The host scope matcher (BG1): suffix tricks, deny wins, default deny, and
//! patterns the matcher cannot enforce are rejected.

use super::*;

fn scope(allow: &[&str], deny: &[&str]) -> Scope {
    Scope {
        allow: allow.iter().map(|p| parse_pattern(p).unwrap()).collect(),
        deny: deny.iter().map(|p| parse_pattern(p).unwrap()).collect(),
        agent_deny: vec![],
        invalid: vec![],
    }
}

#[test]
fn a_bare_domain_matches_itself_and_its_subdomains_and_a_wildcard_only_subdomains() {
    let s = scope(&["example.com"], &[]);
    for host in [
        "example.com",
        "docs.example.com",
        "a.b.example.com",
        "EXAMPLE.com",
    ] {
        assert_eq!(s.decide(host), Decision::Allowed, "{host}");
    }
    let wild = scope(&["*.example.com"], &[]);
    assert_eq!(wild.decide("docs.example.com"), Decision::Allowed);
    assert!(
        matches!(wild.decide("example.com"), Decision::Denied(_)),
        "the apex is not a subdomain"
    );
}

#[test]
fn suffix_tricks_are_refused() {
    let s = scope(&["example.com"], &[]);
    for host in [
        "notexample.com",
        "example.com.evil.com",
        "evilexample.com",
        "example.co",
        "xexample.com",
        "example.com.",
        "com",
    ] {
        // `example.com.` loses its trailing dot (same host) and is allowed; the rest are not.
        let allowed = s.decide(host.trim_end_matches('.')) == Decision::Allowed;
        assert_eq!(allowed, host == "example.com.", "{host}");
    }
    let deny = scope(&[], &["bank.com"]);
    assert!(matches!(deny.decide("bank.com"), Decision::Denied(_)));
    assert!(matches!(deny.decide("login.bank.com"), Decision::Denied(_)));
    assert_eq!(deny.decide("notbank.com"), Decision::Allowed);
    assert_eq!(deny.decide("bank.com.evil.org"), Decision::Allowed);
}

#[test]
fn deny_wins_and_an_allow_list_makes_the_rest_denied_with_a_reason() {
    let s = scope(&["example.com"], &["private.example.com"]);
    let Decision::Denied(why) = s.decide("private.example.com") else {
        panic!("deny wins")
    };
    assert!(
        why.contains("deny list") && why.contains("private.example.com"),
        "{why}"
    );
    let Decision::Denied(why) = s.decide("other.org") else {
        panic!("default deny")
    };
    assert!(why.contains("not on the allow list"), "{why}");
    assert_eq!(s.decide("docs.example.com"), Decision::Allowed);
    // No lists at all allow everything (the caller decides when that is wrong).
    assert_eq!(Scope::default().decide("anything.org"), Decision::Allowed);
}

#[test]
fn patterns_the_matcher_cannot_enforce_are_rejected_not_accepted() {
    for bad in [
        "",
        "   ",
        "https://example.com",
        "example.com/path",
        "example.com:8080",
        "user@example.com",
        "ex*mple.com",
        "*example.com",
        "*.com",
        "com",
        "exa mple.com",
        "bücher.de",
        "-bad.com",
        "bad-.com",
        "a..b.com",
        "example.com?x=1",
        "*.",
        "**.example.com",
    ] {
        assert!(parse_pattern(bad).is_err(), "`{bad}` must be rejected");
    }
    assert_eq!(
        parse_pattern(" Example.COM. ").unwrap(),
        Pattern::Domain("example.com".into())
    );
    assert_eq!(
        parse_pattern("*.Example.com").unwrap(),
        Pattern::Subdomains("example.com".into())
    );
    assert_eq!(
        parse_pattern("xn--bcher-kva.de").unwrap(),
        Pattern::Domain("xn--bcher-kva.de".into())
    );
}

#[test]
fn the_host_of_an_address_drops_user_port_and_case_and_refuses_the_rest() {
    assert_eq!(
        host_of("https://User:pw@Docs.Example.com:8443/a?b#c").as_deref(),
        Some("docs.example.com")
    );
    assert_eq!(
        host_of("http://example.com.").as_deref(),
        Some("example.com")
    );
    // A trick: the user part looks like the allowed host.
    assert_eq!(
        host_of("https://example.com@evil.org/").as_deref(),
        Some("evil.org")
    );
    for bad in [
        "file:///etc/passwd",
        "ftp://example.com",
        "javascript:alert(1)",
        "https://[::1]/",
        "https://bücher.de/",
        "https://localhost/",
        "example.com",
        "",
    ] {
        assert_eq!(host_of(bad), None, "{bad}");
    }
    let s = scope(&["example.com"], &[]);
    assert!(matches!(
        s.decide_url("https://example.com@evil.org/"),
        Decision::Denied(_)
    ));
    assert!(matches!(
        s.decide_url("file:///etc/passwd"),
        Decision::Denied(_)
    ));
    assert_eq!(
        s.decide_url("https://docs.example.com/x"),
        Decision::Allowed
    );
}

#[test]
fn the_built_in_deny_list_refuses_webmail_and_banking_until_the_person_lifts_a_host() {
    let s = Scope::built_in(&[]);
    for url in [
        "https://mail.google.com/mail/u/0",
        "https://outlook.office.com/mail",
        "https://secure.chase.com/x",
        "https://www.paypal.com/",
    ] {
        assert!(matches!(s.decide_url(url), Decision::Denied(_)), "{url}");
    }
    assert_eq!(
        s.decide_url("https://docs.google.com/d/1"),
        Decision::Allowed
    );
    assert_eq!(s.decide_url("https://notchase.com/"), Decision::Allowed);
    let lifted = vec![parse_pattern("paypal.com").unwrap()];
    assert_eq!(
        Scope::built_in(&lifted).decide_url("https://www.paypal.com/"),
        Decision::Allowed
    );
    assert!(matches!(
        Scope::built_in(&lifted).decide_url("https://mail.google.com/"),
        Decision::Denied(_)
    ));
}

#[test]
fn the_persons_file_adds_lists_and_a_bad_entry_closes_the_scope() {
    let dir = tempfile::tempdir().unwrap();
    let (none, _) = Scope::load_user_from(dir.path());
    assert!(none.allow.is_empty() && none.invalid.is_empty());
    let write = |s: &str| std::fs::write(dir.path().join(SCOPE_FILE), s).unwrap();
    write(
        r#"{"allow":["docs.example.com"],"deny":["bad.example.com"],"allowDefaultDenied":["paypal.com"]}"#,
    );
    let eff = Scope::effective_from(Some(dir.path()));
    assert_eq!(
        eff.decide_url("https://docs.example.com/"),
        Decision::Allowed
    );
    assert!(matches!(
        eff.decide_url("https://bad.example.com/"),
        Decision::Denied(_)
    ));
    assert!(
        matches!(eff.decide_url("https://other.org/"), Decision::Denied(_)),
        "an allow list denies the rest"
    );
    // A bad entry, or a file that is not JSON, denies every host.
    for broken in [
        r#"{"allow":["https://docs.example.com"]}"#,
        "not json",
        r#"{"deny":["*.com"]}"#,
    ] {
        write(broken);
        let s = Scope::effective_from(Some(dir.path()));
        assert!(
            matches!(s.decide_url("https://docs.example.com/"), Decision::Denied(why) if why.contains("cannot enforce") || why.contains("not valid")),
            "{broken}"
        );
    }
}

#[test]
fn the_hosts_an_text_names_are_found_once_each_in_order() {
    let text = "go to https://Docs.Example.com/a?x=1, then (http://other.org/b) and https://docs.example.com/c again; file:///etc/passwd ftp://nope.org \"https://quoted.net\"";
    assert_eq!(
        hosts_in(text),
        ["docs.example.com", "other.org", "quoted.net"]
    );
    assert!(hosts_in("no links, just example.com").is_empty());
    assert!(hosts_in("https://[::1]/ https://bücher.de/").is_empty());
    let many: String = (0..40)
        .map(|i| format!("https://h{i}.example.com "))
        .collect();
    assert_eq!(hosts_in(&many).len(), 20);
}

#[test]
fn a_file_that_is_not_an_object_or_has_odd_entries_closes_the_scope() {
    let dir = tempfile::tempdir().unwrap();
    for bad in [
        r#"{"allow":"example.com"}"#,
        r#"{"allow":[123]}"#,
        "[]",
        r#""x""#,
        r#"{"deny":{"a":1}}"#,
    ] {
        std::fs::write(dir.path().join(SCOPE_FILE), bad).unwrap();
        let (s, _) = Scope::load_user_from(dir.path());
        assert!(
            matches!(s.decide("example.com"), Decision::Denied(_)),
            "{bad}"
        );
    }
}

// ND1: the restrict-only ratchet.

fn dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("host-scope-")
        .tempdir()
        .unwrap()
}

#[test]
fn an_agent_can_only_narrow_and_the_addition_holds_on_the_next_decision() {
    let d = dir();
    assert_eq!(
        Scope::effective_from(Some(d.path())).decide("shop.example.com"),
        Decision::Allowed
    );
    let out = add_agent_deny_in(d.path(), "Example.com", "chat s-1").unwrap();
    assert!(out.added && out.pattern == "example.com" && out.entries == 1 && out.capacity == 100);
    // Read at the decision, not cached: the very next one sees it.
    for scope in [
        Scope::effective_from(Some(d.path())),
        Scope::load_user_from(d.path()).0.with_agent_deny(d.path()),
    ] {
        assert!(matches!(
            scope.decide("shop.example.com"),
            Decision::Denied(_)
        ));
        assert!(matches!(scope.decide("example.com"), Decision::Denied(_)));
        assert_eq!(scope.decide("notexample.com"), Decision::Allowed);
    }
    // It never widens: the person's allow list cannot be lifted by it, and an
    // allow entry does not undo it (deny wins).
    std::fs::write(d.path().join(SCOPE_FILE), r#"{"allow":["example.com"]}"#).unwrap();
    assert!(matches!(
        Scope::effective_from(Some(d.path())).decide("example.com"),
        Decision::Denied(_)
    ));
}

#[test]
fn what_cannot_be_enforced_is_refused_as_text_and_an_address_reduces_to_its_host() {
    let d = dir();
    for bad in [
        "",
        "com",
        "*",
        "a.*.com",
        "example.com/path",
        "example.com:8080",
        "evil.com@x.com",
        "bücher.de",
        "http://",
        "ftp://example.com",
        "https://[::1]/",
    ] {
        assert!(add_agent_deny_in(d.path(), bad, "c").is_err(), "{bad}");
    }
    let out =
        add_agent_deny_in(d.path(), "https://User@Login.Example.org:8443/a/b?c#d", "c").unwrap();
    assert_eq!(out.pattern, "login.example.org");
    assert!(
        add_agent_deny_in(d.path(), "*.tracker.net", "c")
            .unwrap()
            .added
    );
    assert_eq!(read_agent_deny(d.path()).unwrap().len(), 2);
}

#[test]
fn a_covered_or_repeated_host_adds_nothing() {
    let d = dir();
    assert!(
        add_agent_deny_in(d.path(), "example.com", "c")
            .unwrap()
            .added
    );
    for again in ["example.com", "www.example.com", "*.example.com"] {
        let out = add_agent_deny_in(d.path(), again, "c").unwrap();
        assert!(!out.added && out.note.is_some(), "{again}");
    }
    // Already on the built-in list.
    assert!(
        !add_agent_deny_in(d.path(), "mail.google.com", "c")
            .unwrap()
            .added
    );
    // A wider pattern than an existing narrower one is new.
    assert!(
        add_agent_deny_in(d.path(), "evil.example.co.uk", "c")
            .unwrap()
            .added
    );
    assert_eq!(read_agent_deny(d.path()).unwrap().len(), 2);
}

#[test]
fn the_list_is_capped_at_one_hundred_and_only_the_person_removes() {
    let d = dir();
    for i in 0..MAX_AGENT_DENY {
        let by = format!("chat {}", i / MAX_PER_CHAT);
        assert!(
            add_agent_deny_in(d.path(), &format!("site{i}.example.net"), &by)
                .unwrap()
                .added,
            "{i}"
        );
    }
    let full = add_agent_deny_in(d.path(), "one-more.example.net", "chat 9").unwrap_err();
    assert!(
        full.contains("full") && full.contains("only the person can remove"),
        "{full}"
    );
    assert_eq!(read_agent_deny(d.path()).unwrap().len(), MAX_AGENT_DENY);
    let left = remove_agent_deny_in(d.path(), "site0.example.net").unwrap();
    assert_eq!(left.len(), MAX_AGENT_DENY - 1);
    assert!(remove_agent_deny_in(d.path(), "site0.example.net").is_err());
    assert!(
        add_agent_deny_in(d.path(), "one-more.example.net", "chat 9")
            .unwrap()
            .added
    );
    // After the removal the host is reachable again.
    assert_eq!(
        Scope::effective_from(Some(d.path())).decide("site0.example.net"),
        Decision::Allowed
    );
}

#[test]
fn a_corrupt_agent_file_closes_the_scope_instead_of_reading_as_empty() {
    let d = dir();
    std::fs::write(d.path().join(AGENT_DENY_FILE), "{ nope").unwrap();
    let scope = Scope::effective_from(Some(d.path()));
    assert!(matches!(
        scope.decide("anything.example.com"),
        Decision::Denied(_)
    ));
    assert!(add_agent_deny_in(d.path(), "example.com", "c").is_err());
    std::fs::write(
        d.path().join(AGENT_DENY_FILE),
        r#"{"entries":[{"pattern":"a b"}]}"#,
    )
    .unwrap();
    assert!(matches!(
        Scope::effective_from(Some(d.path())).decide("x.example.com"),
        Decision::Denied(_)
    ));
}

#[test]
fn one_chat_cannot_use_up_the_list_and_a_shared_suffix_is_refused() {
    let d = dir();
    for i in 0..MAX_PER_CHAT {
        assert!(
            add_agent_deny_in(d.path(), &format!("a{i}.example.net"), "chat 1\n")
                .unwrap()
                .added
        );
    }
    let own = add_agent_deny_in(d.path(), "extra.example.net", "chat 1\n").unwrap_err();
    assert!(
        own.contains("this chat") && own.contains("ask the person"),
        "{own}"
    );
    assert!(
        add_agent_deny_in(d.path(), "extra.example.net", "chat 2")
            .unwrap()
            .added
    );
    // The by label has no control characters.
    assert!(read_agent_deny(d.path())
        .unwrap()
        .iter()
        .all(|e| !e.by.contains('\n')));
    for shared in ["co.uk", "github.io", "*.amazonaws.com", "vercel.app"] {
        let why = add_agent_deny_in(d.path(), shared, "chat 3").unwrap_err();
        assert!(why.contains("shared suffix"), "{shared}: {why}");
    }
    assert!(
        add_agent_deny_in(d.path(), "evil.github.io", "chat 3")
            .unwrap()
            .added
    );
}

#[test]
fn a_refusal_says_the_block_came_from_an_agent_and_the_person_can_repair_a_corrupt_file() {
    let d = dir();
    add_agent_deny_in(d.path(), "tracker.example.net", "chat 1").unwrap();
    let Decision::Denied(why) = Scope::effective_from(Some(d.path())).decide("tracker.example.net")
    else {
        panic!("denied")
    };
    assert!(
        why.contains("agents blocked for themselves") && why.contains("Settings"),
        "{why}"
    );
    // Removal is case-insensitive.
    assert_eq!(
        remove_agent_deny_in(d.path(), " Tracker.Example.NET ")
            .unwrap()
            .len(),
        0
    );
    // A corrupt file: a named removal fails, the empty one resets it.
    std::fs::write(d.path().join(AGENT_DENY_FILE), "{ nope").unwrap();
    assert!(remove_agent_deny_in(d.path(), "x.example.net").is_err());
    assert_eq!(remove_agent_deny_in(d.path(), "").unwrap().len(), 0);
    assert_eq!(
        Scope::effective_from(Some(d.path())).decide("x.example.net"),
        Decision::Allowed
    );
    // With a healthy file an empty pattern removes nothing.
    assert!(remove_agent_deny_in(d.path(), "").is_err());
}
