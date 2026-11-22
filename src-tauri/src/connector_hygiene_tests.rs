//! The connector hygiene table (CD1): a pinned version and every opt-out.

use super::*;

fn connector(args: &[&str], env: &[(&str, &str)]) -> Connector {
    let mut c: Connector = serde_json::from_value(serde_json::json!({
        "id": "devtools", "name": "Chrome DevTools", "command": "npx",
        "args": args, "env": env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<std::collections::BTreeMap<_, _>>(),
    }))
    .unwrap();
    c.id = "devtools".into();
    c
}

#[test]
fn an_unpinned_connector_with_telemetry_on_reports_each_missing_opt_out() {
    let c = connector(&["-y", "chrome-devtools-mcp@latest"], &[]);
    let f = findings(&c);
    assert_eq!(f.len(), 4, "{f:?}");
    assert!(f[0].contains("no pinned version"));
    assert!(f
        .iter()
        .any(|x| x.contains("usage statistics") && x.contains("--no-usage-statistics")));
    assert!(f.iter().any(
        |x| x.contains("update checks") && x.contains("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS=1")
    ));
    assert!(f
        .iter()
        .any(|x| x.contains("CrUX") && x.contains("--no-performance-crux")));
}

#[test]
fn a_pinned_connector_with_every_opt_out_is_clean_by_flag_or_by_variable() {
    let by_flags = connector(
        &[
            "-y",
            "chrome-devtools-mcp@0.9.1",
            "--no-usage-statistics",
            "--no-performance-crux",
        ],
        &[("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS", "1")],
    );
    assert!(findings(&by_flags).is_empty(), "{:?}", findings(&by_flags));
    let by_env = connector(
        &["-y", "chrome-devtools-mcp@0.9.1", "--no-performance-crux"],
        &[
            ("CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS", "1"),
            ("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS", "1"),
        ],
    );
    assert!(findings(&by_env).is_empty());
    // A variable set to the wrong value is not an opt-out.
    let wrong = connector(
        &["chrome-devtools-mcp@0.9.1", "--no-performance-crux"],
        &[
            ("CHROME_DEVTOOLS_MCP_NO_USAGE_STATISTICS", "0"),
            ("CHROME_DEVTOOLS_MCP_NO_UPDATE_CHECKS", "1"),
        ],
    );
    assert_eq!(findings(&wrong).len(), 1);
}

#[test]
fn only_a_version_is_a_pin_and_unknown_connectors_are_left_alone() {
    for pin in ["pkg@1.2.3", "pkg@0.9.1-beta.2", "@scope/pkg@2.0.0"] {
        assert!(pinned(pin), "{pin}");
    }
    for loose in [
        "pkg",
        "pkg@latest",
        "pkg@^1.2.3",
        "pkg@~1.2",
        "pkg@",
        "@scope/pkg",
        "@1.2.3",
    ] {
        assert!(!pinned(loose), "{loose}");
    }
    let other = connector(&["-y", "some-other-server@latest"], &[]);
    assert!(findings(&other).is_empty());
    assert_eq!(
        library_findings(&[other, connector(&["chrome-devtools-mcp"], &[])]).len(),
        4
    );
}
