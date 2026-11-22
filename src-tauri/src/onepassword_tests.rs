//! Tests for `onepassword`: off by default, `op read` through a stub
//! program, fallback to the Keychain, error states, redaction, nothing secret
//! in argv, the child's environment, status or errors. `op` is never the
//! real one: each test writes a stub script in a temp folder.

use super::*;
use std::sync::{Mutex, MutexGuard};

/// A test's own settings folder and stub `op`.
pub(crate) static DIR: Mutex<Option<PathBuf>> = Mutex::new(None);
pub(crate) static BIN: Mutex<Option<PathBuf>> = Mutex::new(None);
/// The statics above and the module's caches are process-wide.
static TEST_LOCK: Mutex<()> = Mutex::new(());

const VALUE: &str = "hunter2-s3cret-VALUE";
const REF: &str = "op://Private/Hollow Token/credential";

struct Fixture {
    _guard: MutexGuard<'static, ()>,
    dir: tempfile::TempDir,
}

fn reset() {
    *lock(&SETTINGS) = None;
    lock(&FAILED).clear();
    lock(&LAST_ERROR).clear();
    lock(&RESOLVED).clear();
}

impl Fixture {
    fn new() -> Self {
        let guard = TEST_LOCK.lock().unwrap_or_else(PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        let settings = dir.path().join("settings");
        std::fs::create_dir_all(&settings).unwrap();
        let d = dir.path().display();
        let script = format!(
            r#"#!/bin/sh
D='{d}'
echo run >> "$D/runs.log"
{{ for a in "$@"; do printf 'ARG:%s\n' "$a"; done; /usr/bin/env | /usr/bin/sed 's/^/ENV:/'; }} > "$D/seen.txt"
if [ "$1" = "--version" ]; then echo "2.31.1"; exit 0; fi
case "$(/bin/cat "$D/mode")" in
  ok) case "$1" in whoami) echo "URL: example.1password.com";; *) printf '{VALUE}\n';; esac ;;
  locked) echo "[ERROR] authorization prompt dismissed, please try again SECRETLEAK-stderr" >&2; exit 1 ;;
  signedout) echo "[ERROR] You are not currently signed in. Please run op signin SECRETLEAK-stderr" >&2; exit 1 ;;
  missing) echo "[ERROR] \"Hollow Token\" isn't an item in the \"Private\" vault SECRETLEAK-stderr" >&2; exit 1 ;;
  boom) echo "[ERROR] something odd SECRETLEAK-stderr" >&2; exit 1 ;;
  empty) exit 0 ;;
  slow) exec /bin/sleep 5 ;;
esac
"#
        );
        let bin = dir.path().join("op");
        std::fs::write(&bin, script).unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(dir.path().join("mode"), "ok").unwrap();
        *lock(&DIR) = Some(settings);
        *lock(&BIN) = Some(bin);
        reset();
        Fixture { _guard: guard, dir }
    }

    /// Change what the stub does, and forget remembered failures.
    fn mode(&self, mode: &str) {
        self.mode_keep(mode);
        lock(&FAILED).clear();
    }

    /// Change what the stub does, leaving remembered failures alone.
    fn mode_keep(&self, mode: &str) {
        std::fs::write(self.dir.path().join("mode"), mode).unwrap();
    }

    /// How many times the stub ran.
    fn runs(&self) -> usize {
        std::fs::read_to_string(self.dir.path().join("runs.log"))
            .map(|t| t.lines().count())
            .unwrap_or(0)
    }

    /// What the last run saw: its arguments and environment.
    fn seen(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("seen.txt")).unwrap_or_default()
    }

    fn on(&self, key: &str) {
        set_enabled(true).unwrap();
        set_reference(key, REF).unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        *lock(&DIR) = None;
        *lock(&BIN) = None;
        reset();
    }
}

#[test]
fn it_is_off_by_default_and_never_runs_op() {
    let f = Fixture::new();
    assert!(!enabled());
    assert!(lookup("k-default").is_none());
    assert!(!has_reference("k-default"));
    // A saved reference does nothing while the switch is off.
    set_reference("k-default", REF).unwrap();
    assert!(!enabled());
    assert!(lookup("k-default").is_none(), "off: the Keychain answers");
    assert!(!has_reference("k-default"));
    let status = status();
    assert_eq!(status.cli.state, "off");
    assert_eq!(check().cli.state, "off", "Check runs nothing while off");
    assert_eq!(f.runs(), 0, "op was never run");
}

#[test]
fn a_key_without_a_reference_is_read_from_the_keychain_even_when_on() {
    let f = Fixture::new();
    set_enabled(true).unwrap();
    assert!(lookup("k-none").is_none());
    assert_eq!(f.runs(), 0);
}

#[test]
fn it_reads_a_reference_with_op_read_and_nothing_secret_in_argv() {
    let f = Fixture::new();
    f.on("k-read");
    assert_eq!(lookup("k-read").unwrap().unwrap(), VALUE);
    let seen = f.seen();
    let args: Vec<&str> = seen
        .lines()
        .filter_map(|l| l.strip_prefix("ARG:"))
        .collect();
    assert_eq!(args, ["read", "--no-newline", REF]);
    assert!(!seen.contains(VALUE), "the value is never in argv or env");
}

#[test]
fn op_gets_a_cleared_environment() {
    let f = Fixture::new();
    f.on("k-env");
    assert!(lookup("k-env").unwrap().is_ok());
    let seen = f.seen();
    let names: Vec<&str> = seen
        .lines()
        .filter_map(|l| l.strip_prefix("ENV:"))
        .filter_map(|l| l.split('=').next())
        .collect();
    assert!(names.contains(&"PATH"), "{names:?}");
    // cargo sets these in the test process; none may reach op.
    assert!(
        !names
            .iter()
            .any(|n| n.starts_with("CARGO") || n.starts_with("RUST")),
        "{names:?}"
    );
}

#[test]
fn child_env_keeps_the_few_it_needs_and_the_persons_op_variables() {
    let vars = [
        ("PATH", "/usr/bin"),
        ("HOME", "/Users/x"),
        ("USER", "x"),
        ("OP_SERVICE_ACCOUNT_TOKEN", "ops_stand_in"),
        ("OP_CONNECT_HOST", "http://h"),
        ("ANTHROPIC_API_KEY", "sk-ant-nope"),
        ("CLAUDE_CODE_OAUTH_TOKEN", "nope"),
        ("GITHUB_TOKEN", "ghp_nope"),
        ("HOLLOW_RUN_KEY", "nope"),
        ("SSH_AUTH_SOCK", "/tmp/agent"),
    ]
    .map(|(k, v)| (OsString::from(k), OsString::from(v)));
    let out = child_env(vars.into_iter());
    let mut names: Vec<String> = out
        .iter()
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "HOME",
            "OP_CONNECT_HOST",
            "OP_SERVICE_ACCOUNT_TOKEN",
            "PATH",
            "USER"
        ]
    );
    let path = out
        .iter()
        .find(|(k, _)| k == "PATH")
        .unwrap()
        .1
        .to_string_lossy()
        .into_owned();
    assert!(
        path.starts_with("/usr/bin:") && path.contains("/opt/homebrew/bin"),
        "{path}"
    );
}

#[test]
fn the_keychain_is_the_fallback_and_the_error_is_clear_when_there_is_none() {
    let f = Fixture::new();
    f.on("k-fallback");
    crate::secret_store::set("k-fallback", "from-keychain").unwrap();

    // 1Password answers: it wins over the Keychain.
    assert_eq!(
        crate::secret_store::get("k-fallback").unwrap().as_deref(),
        Some(VALUE)
    );

    // Locked: the Keychain value is used, nothing blocks.
    f.mode("locked");
    assert_eq!(
        crate::secret_store::get("k-fallback").unwrap().as_deref(),
        Some("from-keychain")
    );

    // No Keychain value: the caller gets 1Password's reason, not "not set".
    crate::secret_store::delete("k-fallback").unwrap();
    f.mode("signedout");
    let err = crate::secret_store::get("k-fallback").unwrap_err();
    assert_eq!(err.0, OpError::NotSignedIn.message());
    assert!(!err.0.contains("SECRETLEAK"), "op's own words are dropped");

    // Off again: the Keychain path as before.
    set_enabled(false).unwrap();
    crate::secret_store::set("k-fallback", "from-keychain").unwrap();
    assert_eq!(
        crate::secret_store::get("k-fallback").unwrap().as_deref(),
        Some("from-keychain")
    );
    crate::secret_store::delete("k-fallback").unwrap();
}

#[test]
fn another_key_is_not_blocked_by_a_failing_one() {
    let f = Fixture::new();
    f.on("k-bad");
    f.mode("locked");
    assert!(lookup("k-bad").unwrap().is_err());
    crate::secret_store::set("k-other", "plain").unwrap();
    assert_eq!(
        crate::secret_store::get("k-other").unwrap().as_deref(),
        Some("plain")
    );
    crate::secret_store::delete("k-other").unwrap();
}

#[test]
fn present_never_runs_op() {
    let f = Fixture::new();
    f.on("k-present");
    assert!(crate::secret_store::present("k-present").unwrap());
    assert_eq!(f.runs(), 0, "a status line cannot raise an unlock prompt");
}

#[test]
fn each_failure_has_its_own_fixed_message() {
    let f = Fixture::new();
    f.on("k-err");
    for (mode, want) in [
        ("locked", OpError::Locked),
        ("signedout", OpError::NotSignedIn),
        ("missing", OpError::ItemNotFound),
        ("boom", OpError::Failed),
        ("empty", OpError::Failed),
    ] {
        f.mode(mode);
        let got = lookup("k-err").unwrap().unwrap_err();
        assert_eq!(got, want, "{mode}");
        assert!(!got.message().contains("SECRETLEAK"));
        assert!(!got.message().contains('\u{2014}') && !got.message().contains('\u{2013}'));
    }
}

#[test]
fn op_not_installed_is_a_state_that_leaves_the_keychain_alone() {
    let f = Fixture::new();
    f.on("k-gone");
    std::fs::remove_file(lock(&BIN).clone().unwrap()).unwrap();
    assert_eq!(
        lookup("k-gone").unwrap().unwrap_err(),
        OpError::NotInstalled
    );
    let s = status();
    assert!(!s.installed);
    assert_eq!(s.cli.state, "not-installed");
    crate::secret_store::set("k-gone", "kc").unwrap();
    assert_eq!(
        crate::secret_store::get("k-gone").unwrap().as_deref(),
        Some("kc")
    );
    crate::secret_store::delete("k-gone").unwrap();
}

#[test]
fn a_slow_op_is_stopped_and_reported() {
    let f = Fixture::new();
    f.mode("slow");
    let bin = lock(&BIN).clone().unwrap();
    let got = run_op(&bin, &["read", REF], Duration::from_millis(300));
    assert_eq!(got.err(), Some(OpError::TimedOut));
}

#[test]
fn a_failed_read_is_remembered_so_callers_do_not_stack_prompts() {
    let f = Fixture::new();
    f.on("k-memory");
    f.mode("locked");
    assert!(lookup("k-memory").unwrap().is_err());
    assert!(lookup("k-memory").unwrap().is_err());
    assert!(lookup("k-memory").unwrap().is_err());
    assert_eq!(f.runs(), 1, "one op run for three callers");
    // Still remembered after 1Password is unlocked, until Check forgets it.
    f.mode_keep("ok");
    assert!(lookup("k-memory").unwrap().is_err());
    check();
    assert_eq!(lookup("k-memory").unwrap().unwrap(), VALUE);
}

#[test]
fn a_failure_about_1password_itself_never_expires_but_one_reference_does() {
    let old = Instant::now() - FAIL_MEMORY - Duration::from_secs(1);
    let global = Failure {
        scope: None,
        at: old,
        error: OpError::Locked,
    };
    assert!(
        global.live(),
        "locked outlasts the window so an agent can't re-raise the prompt"
    );
    let one = Failure {
        scope: Some(("k".into(), "op://V/I/f".into())),
        at: old,
        error: OpError::ItemNotFound,
    };
    assert!(!one.live());
}

#[test]
fn check_reports_ready_locked_and_not_signed_in() {
    let f = Fixture::new();
    set_enabled(true).unwrap();
    let ready = check();
    assert_eq!(ready.cli.state, "ready");
    assert_eq!(ready.cli.version.as_deref(), Some("2.31.1"));
    f.mode("locked");
    assert_eq!(check().cli.state, "locked");
    f.mode("signedout");
    let out = check();
    assert_eq!(out.cli.state, "not-signed-in");
    assert!(!out.cli.message.contains("SECRETLEAK"));
}

#[test]
fn the_value_is_masked_everywhere_redact_runs_and_is_in_no_status() {
    let f = Fixture::new();
    f.on("k-redact");
    assert_eq!(lookup("k-redact").unwrap().unwrap(), VALUE);
    let echoed = format!("the agent printed {VALUE} and again {VALUE}.");
    let masked = crate::claude_runner::redact(&echoed);
    assert!(!masked.contains(VALUE), "{masked}");
    assert!(masked.contains("[redacted]"));
    // The value inside a JSON string, escaped, is masked too.
    remember("pa\"ss\\word\nline-two-secret");
    let json = serde_json::json!({ "out": "pa\"ss\\word\nline-two-secret" }).to_string();
    assert!(!crate::claude_runner::redact(&json).contains("line-two-secret"));
    // The reference is masked too, whatever its item name.
    let line = crate::claude_runner::redact(&format!("op read {REF} failed"));
    assert!(
        !line.contains("Private") && line.contains("op://[redacted]"),
        "{line}"
    );
    // Nothing Settings is sent carries it.
    let json = serde_json::to_string(&status()).unwrap();
    assert!(!json.contains(VALUE), "{json}");
    // A multi-line value is masked line by line.
    remember("-----line-one-secret\nsecond-line-secret");
    let text = crate::claude_runner::redact("a: second-line-secret b");
    assert!(!text.contains("second-line-secret"), "{text}");
}

#[test]
fn redact_masks_op_shapes() {
    let token = "ops_eyJzaWduSW5BZGRyZXNzIjoibXkuMXBhc3N3b3JkLmNvbSJ9";
    let out =
        crate::claude_runner::redact(&format!("OP_SERVICE_ACCOUNT_TOKEN={token} and {token}"));
    assert!(!out.contains("ops_eyJ"), "{out}");
    let key = "A3-ABCDEF-ABCDEF-ABCDE-ABCDE-ABCDE-ABCDE";
    let out = crate::claude_runner::redact(&format!("secret key {key}"));
    assert!(!out.contains(key) && out.contains("[redacted]"), "{out}");
    let out = crate::claude_runner::redact("op://Vault/Item/field?attribute=otp");
    assert_eq!(out, "op://[redacted]");
    assert_eq!(crate::claude_runner::redact(&out), out, "idempotent");
}

#[test]
fn short_values_are_not_masked_by_value_and_the_registry_is_bounded() {
    let _f = Fixture::new();
    remember("abc");
    assert_eq!(crate::claude_runner::redact("abc abc"), "abc abc");
    for i in 0..(REGISTRY_MAX + 8) {
        remember(&format!("registry-value-{i:03}"));
    }
    assert_eq!(lock(&RESOLVED).len(), REGISTRY_MAX);
    let kept = crate::claude_runner::redact("registry-value-000 registry-value-039");
    assert!(
        kept.starts_with("registry-value-000"),
        "oldest was dropped: {kept}"
    );
    assert!(kept.ends_with("[redacted]"), "{kept}");
}

#[test]
fn references_are_checked_before_they_reach_a_command_line() {
    for good in [
        "op://Private/Item/password",
        "op://Private/Hollow Token/credential",
        "op://Private/Item/section/field",
        "op://Private/Item/one-time password?attribute=otp",
    ] {
        assert!(reference_ok(good).is_ok(), "{good}");
    }
    for bad in [
        "",
        "op://",
        "op://Vault/Item",
        "op://Vault//field",
        "op://Vault/../field",
        "op://a/b/c/d/e",
        "https://example.com/a/b/c",
        "--help",
        "op://Vault/Item/field\nextra",
        "op://Vault/It\\em/field",
        &format!("op://Vault/Item/{}", "x".repeat(600)),
    ] {
        assert_eq!(reference_ok(bad), Err(OpError::BadReference), "{bad:?}");
    }
    let f = Fixture::new();
    assert!(set_reference("k", "not a reference").is_err());
    assert_eq!(f.runs(), 0);
}

#[test]
fn settings_are_private_hold_no_secret_and_survive_a_damaged_file() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.on("k-file");
    lookup("k-file").unwrap().unwrap();
    let path = settings_path().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(REF) && !text.contains(VALUE), "{text}");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o077,
        0
    );
    // A damaged file is off, not a crash and not a stale cache.
    *lock(&SETTINGS) = None;
    std::fs::write(&path, "{ not json").unwrap();
    assert!(!enabled());
    assert!(lookup("k-file").is_none());
    std::fs::write(&path, r#"{"enabled":true,"refs":{"k-file":"op://V/I/f"}}"#).unwrap();
    assert!(enabled(), "a repaired file is read again");
    clear_reference("k-file").unwrap();
    assert!(!has_reference("k-file"));
    assert!(f.runs() >= 1);
}

#[test]
fn a_changed_reference_or_switch_forgets_a_remembered_failure() {
    let f = Fixture::new();
    f.on("k-forget");
    f.mode("locked");
    assert!(lookup("k-forget").unwrap().is_err());
    f.mode_keep("ok");
    // Still remembered until something changes.
    assert!(lookup("k-forget").unwrap().is_err());
    set_reference("k-forget", "op://Private/Other Item/credential").unwrap();
    assert_eq!(lookup("k-forget").unwrap().unwrap(), VALUE);
}

#[test]
fn slots_name_the_github_token_and_the_linear_key() {
    let keys: Vec<String> = slots().into_iter().map(|s| s.key).collect();
    assert!(
        keys.contains(&"runner.github.token".to_string()),
        "{keys:?}"
    );
    assert!(keys.contains(&"tracker:linear".to_string()), "{keys:?}");
}

#[test]
fn wipe_zeroes_a_buffer() {
    let mut bytes = VALUE.as_bytes().to_vec();
    wipe(&mut bytes);
    assert!(bytes.iter().all(|b| *b == 0));
}

#[test]
fn classify_reads_the_known_phrases() {
    for (text, want) in [
        (
            "[ERROR] You are not currently signed in. Please run `op signin`",
            OpError::NotSignedIn,
        ),
        (
            "[ERROR] no accounts configured for use with 1Password CLI",
            OpError::NotSignedIn,
        ),
        (
            "[ERROR] authorization prompt dismissed, please try again",
            OpError::Locked,
        ),
        ("[ERROR] 1Password is locked", OpError::Locked),
        (
            "[ERROR] \"x\" isn't an item in the \"y\" vault",
            OpError::ItemNotFound,
        ),
        ("[ERROR] could not find vault", OpError::ItemNotFound),
        ("[ERROR] network is unreachable", OpError::Failed),
        ("", OpError::Failed),
    ] {
        assert_eq!(classify(text.as_bytes()), want, "{text}");
    }
}
