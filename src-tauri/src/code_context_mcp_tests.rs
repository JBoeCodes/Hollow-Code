//! CC2: the MCP transport and the CLI twin return the same bytes for every
//! tool (the row's done-when), and the attach entry's own rules.

use super::*;
use crate::code_context::tests::{fixture, repo, target};

fn mcp(t: &Target, tool: &str, args: &Value) -> (String, bool) {
    let req = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/call", "params": { "name": tool, "arguments": args } });
    let resp = handle(&req, t).unwrap();
    let result = &resp["result"];
    let mut out = String::new();
    for block in result["content"].as_array().unwrap() {
        out.push_str(block["text"].as_str().unwrap());
        out.push('\n');
    }
    (out, result["isError"] == true)
}

/// Claude and Codex (MCP) and Grok (the CLI) get identical JSON from each
/// of the nine tools, and from a refusal.
#[test]
fn parity_mcp_and_cli_print_identical_bytes_for_all_nine_tools() {
    let (repo, cache) = (fixture(), tempfile::tempdir().unwrap());
    let t = target(&repo, &cache);
    let cases = [
        ("map", json!({ "path": "src" })),
        ("symbols", json!({ "query": "check" })),
        (
            "callers",
            json!({ "name": "check", "qualifier": "memory_guard" }),
        ),
        ("callees", json!({ "name": "a" })),
        ("impact", json!({ "name": "decisions" })),
        (
            "read",
            json!({ "path": "src/user.rs", "fromLine": 2, "toLine": 4 }),
        ),
        ("callers", json!({ "name": "check", "qualifer": "x" })),
        // CC4: a slice by symbol and by task, a bad handle and a near miss.
        ("slice", json!({ "symbol": "a", "depth": 2 })),
        ("slice", json!({ "task": "memory guard check" })),
        (
            "slice",
            json!({ "symbol": "a", "handle": "h1.0123456789abcdef" }),
        ),
        ("slice", json!({ "symbl": "a" })),
        // CC5: changes (no commit here: refused), affected_tests by symbol
        // and a near miss.
        ("changes", json!({})),
        ("affected_tests", json!({ "symbols": ["a", "check"] })),
        ("affected_tests", json!({ "depht": 1 })),
    ];
    for (tool, args) in cases {
        let (via_mcp, mcp_error) = mcp(&t, tool, &args);
        let (via_cli, code) = cli(&[tool.to_string(), args.to_string()], &t);
        assert_eq!(via_mcp, via_cli, "{tool} {args}");
        assert_eq!(code, i32::from(mcp_error), "{tool}");
        assert_eq!(via_cli.starts_with('{'), !mcp_error, "{tool}: {via_cli}");
    }
    // CC4: a handle from the MCP server works through the CLI (they share
    // the index folder), with the same bytes.
    let (out, _) = mcp(&t, "slice", &json!({ "symbol": "a" }));
    let first_handle =
        serde_json::from_str::<Value>(out.lines().next().unwrap()).unwrap()["handle"].clone();
    let again = json!({ "symbol": "a", "handle": first_handle });
    let (via_mcp, _) = mcp(&t, "slice", &again);
    let (via_cli, code) = cli(&["slice".into(), again.to_string()], &t);
    assert_eq!((via_mcp.as_str(), code), (via_cli.as_str(), 0));
    assert!(via_cli.contains("\"notModified\":true"), "{via_cli}");
    // A callers question answered the same way through both.
    let (out, _) = cli(&["callers".into(), r#"{"name":"decisions"}"#.into()], &t);
    let first: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!(first["totals"]["resolved"], 2);
    // The CLI's tool list is the server's.
    let list = handle(
        &json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" }),
        &t,
    )
    .unwrap();
    let (tools, code) = cli(&["tools".into()], &t);
    assert_eq!((tools, code), (format!("{}\n", list["result"]["tools"]), 0));
    let (help, _) = cli(&[], &t);
    assert!(
        help.contains(
            "map, symbols, callers, callees, impact, read, slice, changes, affected_tests"
        ),
        "{help}"
    );
    let (bad, code) = cli(&["callers".into(), "{not json".into()], &t);
    assert!(bad.contains("INVALID_ARGUMENT") && code == 1, "{bad}");
}

#[test]
fn server_entry_needs_the_toggle_a_checkout_and_a_cwd_outside_the_vault() {
    let _toggles = crate::mcp_attach::SERVER_TOGGLES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let checkout = repo(&[("src/lib.rs", "fn a() {}\n")]);
    let plain = tempfile::tempdir().unwrap();
    set_code_context_mcp_enabled(true);
    let (name, entry) = server_entry(&checkout.path().join("src"), None).unwrap();
    assert_eq!(name, "hollow_code_context");
    let root = std::fs::canonicalize(checkout.path()).unwrap();
    assert_eq!(entry["env"], json!({ ENV_ROOT: root.to_string_lossy() }));
    assert!(server_entry(plain.path(), None).is_none(), "not a checkout");
    assert!(
        server_entry(&checkout.path().join("src"), Some(checkout.path())).is_none(),
        "in the vault"
    );
    // A root that holds the vault is refused even from a cwd outside it,
    // and an entry outside the vault carries it for the server's own check.
    let vault = checkout.path().join("notes");
    std::fs::create_dir_all(&vault).unwrap();
    assert!(
        server_entry(&checkout.path().join("src"), Some(&vault)).is_none(),
        "the root holds the vault"
    );
    let elsewhere = tempfile::tempdir().unwrap();
    let (_, entry) = server_entry(checkout.path(), Some(elsewhere.path())).unwrap();
    assert_eq!(
        entry["env"][ENV_VAULT],
        json!(elsewhere.path().to_string_lossy())
    );
    // The CLI twin refuses the same root (should-fix c).
    let cache = tempfile::tempdir().unwrap();
    let mut t = target(&checkout, &cache);
    t.vault = Some(vault.clone());
    let (out, code) = cli(&["map".into(), "{}".into()], &t);
    assert_eq!(code, 1, "{out}");
    assert!(out.contains("REFUSED") && out.contains("vault"), "{out}");
    let (help, _) = cli(&[], &t);
    assert!(help.contains("can't read"), "{help}");
    set_code_context_mcp_enabled(false);
    let off = server_entry(checkout.path(), None);
    set_code_context_mcp_enabled(true);
    assert!(off.is_none());
}
