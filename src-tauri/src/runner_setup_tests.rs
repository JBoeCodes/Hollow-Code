//! Setting up a remote runner (remote runners 3). A rig stands in for both
//! machines: a fake `ssh` that answers `-G` from the rig's own
//! `~/.ssh/config` and runs commands in a fake admin home or the runner
//! account's home (only when that account's authorized_keys holds this
//! Mac's key), stand-ins for the macOS admin tools (`sudo`, `dscl`,
//! `sysadminctl`, `pmset`, `sshd`, `launchctl`, `brew`...) that record what
//! they were asked and keep state in files, a fake `gcloud` that records its
//! argv and answers canned output, and a fake `hollow-slot`. Nothing here
//! touches this machine's accounts, settings, `~/.ssh` or Google Cloud.

use super::*;
use regex::Regex;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

const FAKE_SSH: &str = r##"#!/bin/sh
rig='@RIG@'
cfg='@LOCAL@/.ssh/config'
if [ "$1" = -G ]; then
  host=$2
  if [ ! -f "$cfg" ]; then printf 'hostname %s\nuser localuser\n' "$host"; exit 0; fi
  awk -v h="$host" -v home='@LOCAL@' '
    BEGIN { hn = h; u = "localuser" }
    {
      line = $0; sub(/^[ \t]+/, "", line)
      if (line == "" || substr(line, 1, 1) == "#") next
      k = line; sub(/[ \t=].*/, "", k); k = tolower(k)
      v = substr(line, length(k) + 1); sub(/^[ \t=]+/, "", v)
      if (k == "host" || k == "match") {
        inb = 0
        if (k == "host") { n = split(v, pats, /[ \t]+/); for (i = 1; i <= n; i++) if (pats[i] == h || pats[i] == "*") inb = 1 }
        next
      }
      if (!inb) next
      if (k == "hostname" && !shn) { hn = v; shn = 1 }
      if (k == "user" && !su) { u = v; su = 1 }
      if (k == "identityfile") { if (substr(v, 1, 2) == "~/") v = home "/" substr(v, 3); ids = ids "identityfile " v "\n" }
      if (k == "proxycommand" && !sp) { pc = v; sp = 1 }
    }
    END { print "hostname " hn; print "user " u; printf "%s", ids; if (sp) print "proxycommand " pc }' "$cfg"
  exit 0
fi
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o | -p | -i | -l) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
dest=$1
shift
printf '%s\n' "$dest" >> "$rig/ssh.log"
case "$dest" in
  down*) echo "ssh: connect to host $dest port 22: Operation timed out" >&2; exit 255 ;;
  *@*)
    if [ -e "$rig/state/admin-locked" ]; then echo "$dest: Permission denied (publickey,password)." >&2; exit 255; fi
    home="$rig/admin" ;;
  *)
    user=$("$0" -G "$dest" | awk '$1 == "user" { print $2; exit }')
    idf=$("$0" -G "$dest" | awk '$1 == "identityfile" { print $2; exit }')
    home="$rig/root/Users/$user"
    b64=$(cut -d' ' -f2 "$idf.pub" 2>/dev/null)
    if [ -z "$b64" ] || ! grep -qF "$b64" "$home/.ssh/authorized_keys" 2>/dev/null; then
      echo "$dest: Permission denied (publickey)." >&2
      exit 255
    fi ;;
esac
cd "$home" || exit 255
exec env -i HOME="$home" PATH="$rig/bin:/usr/bin:/bin:/usr/sbin:/sbin" RIG="$rig" /bin/sh -c "$*"
"##;

/// The macOS admin tools, as one script installed under each name.
const STUB: &str = r##"#!/bin/sh
name=${0##*/}
s="$RIG/state"
printf '%s %s\n' "$name" "$*" >> "$RIG/tools.log"
case "$name" in
  sudo)
    [ "$1" = -n ] && shift
    if [ -e "$s/sudo-password" ]; then echo "sudo: a password is required" >&2; exit 1; fi
    [ "$1" = -u ] && shift 2
    exec "$@" ;;
  sw_vers) echo 26.1 ;;
  dscl)
    # Interactive (commands on stdin): keep them, as dscl would read them.
    # dscl exits 0 when a command fails; the error is in its output.
    if [ "$#" = 1 ]; then
      cat > "$s/dscl-stdin"
      # Failing, it repeats the command it read (password and all), as a dscl might.
      if [ -e "$s/dscl-fail" ]; then cat "$s/dscl-stdin"; echo "passwd: DS Error: -14165 (eDSAuthPasswordQualityCheckFailed)"; fi
      echo Goodbye; exit 0
    fi
    verb=$2 rec=$3 key=${4:-}
    case "$verb:$rec" in
      -read:/Groups/com.apple.access_ssh) [ -e "$s/access_ssh" ] ;;
      -read:/Users/*)
        a=${rec#/Users/}
        if [ ! -e "$s/user-$a" ]; then echo "DS Error: -14136 (eDSRecordNotFound)" >&2; exit 56; fi
        case "$key" in
          RealName) if [ -e "$s/made-$a" ]; then printf 'RealName:\n Hollow runner\n'; else echo "RealName: Someone Else"; fi ;;
          IsHidden) if [ -e "$s/hidden-$a" ]; then echo "IsHidden: 1"; else echo "No such key: IsHidden" >&2; exit 181; fi ;;
          *) echo "UniqueID: 550" ;;
        esac ;;
      -create:/Users/*) [ "$key" = IsHidden ] && touch "$s/hidden-${rec#/Users/}" ;;
    esac ;;
  dseditgroup)
    case "$2" in
      checkmember) if [ -e "$s/$5-$4" ]; then echo "yes $4 is a member of $5"; else echo "no $4 is NOT a member of $5"; exit 1; fi ;;
      edit) touch "$s/$7-$4" ;;
    esac ;;
  sysadminctl) [ "$1" = -addUser ] && touch "$s/user-$2" "$s/made-$2" ;;
  createhomedir) mkdir -p "$RIG/root/Users/$3" ;;
  pmset)
    if [ "$1" = -g ]; then
      if [ -e "$s/pmset-set" ]; then printf ' sleep                0\n disksleep            0\n autorestart          1\n'
      else printf ' standby              1\n sleep                1 (sleep prevented by sharingd)\n disksleep            10\n autorestart          0\n'; fi
    else touch "$s/pmset-set"; fi ;;
  sshd) [ ! -e "$s/sshd-bad" ] ;;
  launchctl | chown | brew | curl | npx) : ;;
esac
"##;

const STUB_NAMES: [&str; 13] = [
    "sudo",
    "sw_vers",
    "dscl",
    "dseditgroup",
    "sysadminctl",
    "createhomedir",
    "pmset",
    "sshd",
    "launchctl",
    "chown",
    "brew",
    "curl",
    "npx",
];

const FAKE_GCLOUD: &str = r##"#!/bin/sh
s='@RIG@/gstate'
mkdir -p "$s"
{ for a in "$@"; do printf '%s\037' "$a"; done; printf '\n'; } >> '@RIG@/gcloud.log'
all="$*"
if [ -f "$s/fail" ]; then
  f=$(cat "$s/fail")
  case "$all" in *"$f"*) echo "ERROR: (gcloud) PERMISSION_DENIED: the stand-in refuses $f" >&2; exit 1 ;; esac
fi
case "$all" in
  "config list account"*) [ -e "$s/signed-out" ] || echo someone@example.com ;;
  "projects describe"*) echo "$3 ACTIVE" ;;
  "services list"*) cat "$s/services" 2>/dev/null; true ;;
  "services enable"*) printf '%s\n' compute.googleapis.com iap.googleapis.com billingbudgets.googleapis.com oslogin.googleapis.com > "$s/services" ;;
  "billing budgets list"*) cat "$s/budget" 2>/dev/null; true ;;
  "billing budgets create"*) for a in "$@"; do case "$a" in --display-name=*) echo "${a#--display-name=}" > "$s/budget" ;; esac; done ;;
  "iam service-accounts describe"*) if [ -e "$s/sa" ]; then echo "$4"; else echo "ERROR: (gcloud.iam.service-accounts.describe) NOT_FOUND: Unknown service account" >&2; exit 1; fi ;;
  "iam service-accounts create"*) if [ -e "$s/sa" ]; then echo "ERROR: (gcloud.iam.service-accounts.create) Service account $4 already exists within project" >&2; exit 1; fi; touch "$s/sa" ;;
  "compute firewall-rules describe"*) if [ -e "$s/fw" ]; then echo '35.235.240.0/20'; else echo "ERROR: (gcloud.compute.firewall-rules.describe) Could not fetch resource: The resource was not found" >&2; exit 1; fi ;;
  "compute firewall-rules create"*) touch "$s/fw" ;;
  "compute instances describe"*) if [ -e "$s/vm" ]; then cat "$s/vm"; else echo "ERROR: (gcloud.compute.instances.describe) Could not fetch resource: The resource was not found" >&2; exit 1; fi ;;
  "compute instances create"*)
    for a in "$@"; do case "$a" in --metadata-from-file=startup-script=*) cp "${a#--metadata-from-file=startup-script=}" "$s/startup-copy" ;; esac; done
    echo RUNNING > "$s/vm" ;;
  "compute routers create"*) touch "$s/router" ;;
  "compute routers nats create"*)
    [ -e "$s/router" ] || { echo "ERROR: no router" >&2; exit 1; }
    if [ -e "$s/nat" ]; then echo "ERROR: (gcloud.compute.routers.nats.create) Invalid value for field 'resource.nats[1].name': '$5'. Duplicate NAT name." >&2; exit 1; fi
    touch "$s/nat" ;;
  "compute routers nats describe"*) if [ -e "$s/nat" ]; then echo "$5"; else echo "ERROR: (gcloud.compute.routers.nats.describe) NAT $5 not found" >&2; exit 1; fi ;;
  "compute instances start"*) echo RUNNING > "$s/vm" ;;
  "compute os-login describe-profile"*) echo someone_example_com ;;
  "compute os-login ssh-keys add"*) touch "$s/oslogin-key" ;;
  "compute start-iap-tunnel"*) echo "SSH-2.0-stand-in" ;;
  *) echo "ERROR: the stand-in doesn't know: $all" >&2; exit 2 ;;
esac
"##;

const FAKE_SLOT: &str = r##"#!/bin/sh
printf '%s\n' "$*" >> '@RIG@/slot.log'
git rev-parse --is-inside-work-tree >/dev/null 2>&1 || { echo "hollow-slot: not in a git checkout" >&2; exit 1; }
if [ -e '@RIG@/state/slot-fallback' ]; then
  echo "hollow-slot: test-runner didn't answer; running on this Mac instead" >&2
fi
echo "Darwin arm64"; echo v24.0.0; echo "cargo 1.96.0"
"##;

struct Rig {
    dir: tempfile::TempDir,
}

fn write_exec(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

impl Rig {
    fn new() -> Rig {
        let dir = tempfile::Builder::new()
            .prefix("rset")
            .tempdir_in("/tmp")
            .unwrap();
        let rig = Rig { dir };
        for d in [
            "local",
            "admin",
            "root/Users",
            "root/etc/ssh/sshd_config.d",
            "state",
            "gstate",
            "bin",
            "tmp",
        ] {
            std::fs::create_dir_all(rig.p(d)).unwrap();
        }
        std::fs::write(
            rig.p("root/etc/ssh/sshd_config"),
            "# stand-in sshd_config\nPasswordAuthentication no\n",
        )
        .unwrap();
        let fill = |t: &str| {
            t.replace("@RIG@", &rig.s(""))
                .replace("@LOCAL@", &rig.s("local"))
        };
        write_exec(&rig.p("bin/ssh"), &fill(FAKE_SSH));
        write_exec(&rig.p("bin/gcloud"), &fill(FAKE_GCLOUD));
        write_exec(&rig.p("bin/hollow-slot"), &fill(FAKE_SLOT));
        for name in STUB_NAMES {
            write_exec(&rig.p(&format!("bin/{name}")), STUB);
        }
        rig
    }
    fn p(&self, rel: &str) -> PathBuf {
        if rel.is_empty() {
            self.dir.path().to_path_buf()
        } else {
            self.dir.path().join(rel)
        }
    }
    fn s(&self, rel: &str) -> String {
        self.p(rel).to_string_lossy().into_owned()
    }
    fn ctx(&self) -> Ctx {
        Ctx {
            home: self.p("local"),
            path: OsString::from(format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", self.s("bin"))),
            ssh: self.s("bin/ssh"),
            gcloud: self.p("bin/gcloud"),
            hollow_slot: self.p("bin/hollow-slot"),
            tmp: self.p("tmp"),
            today: "20261103".into(),
            root: self.s("root"),
            slot_remote: Some(self.p("slot-remote")),
        }
    }
    fn mark(&self, name: &str) {
        std::fs::write(self.p(&format!("state/{name}")), "").unwrap();
    }
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.p(rel)).unwrap_or_default()
    }
    fn gcloud_calls(&self) -> Vec<Vec<String>> {
        self.read("gcloud.log")
            .lines()
            .map(|l| {
                l.split('\u{1f}')
                    .filter(|a| !a.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .collect()
    }
}

fn mac() -> SetupParams {
    SetupParams {
        path: "mac".into(),
        admin: "admin@test-mac.local".into(),
        alias: "test-runner".into(),
        account: "testworker".into(),
        host_name: "test-mac.local".into(),
        ..SetupParams::default()
    }
}

fn cloud() -> SetupParams {
    SetupParams {
        path: "cloud".into(),
        project: "my-project-123".into(),
        zone: "us-central1-a".into(),
        machine_type: "e2-standard-8".into(),
        billing_account: "01ABCD-23EF45-6789AB".into(),
        budget_usd: 80,
        ..SetupParams::default()
    }
}

fn by_id<'a>(states: &'a [StepState], id: &str) -> &'a StepState {
    states.iter().find(|s| s.id == id).unwrap()
}

fn step<'a>(steps: &'a [Step], id: &str) -> &'a Step {
    steps.iter().find(|s| s.id == id).unwrap()
}

// ---- What is shown is what runs ----------------------------------------------

/// Every shell command parses, its here-documents end where they should,
/// and a gcloud command's shown text, typed into a shell, is exactly the
/// argv that runs.
#[test]
fn what_is_shown_is_what_runs() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    for params in [mac(), cloud()] {
        let plan = plan(&ctx, &params);
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        for s in &plan.steps {
            for c in s.check.iter().chain(&s.apply).chain(&s.by_hand) {
                match &c.run {
                    Run::Shell | Run::Hollow(Builtin::TestCall) => {
                        let ok = Command::new("/bin/sh")
                            .args(["-n", "-c", &c.shown])
                            .status()
                            .unwrap();
                        assert!(ok.success(), "{}: doesn't parse:\n{}", s.id, c.shown);
                        for end in ["HOLLOW", "STARTUP"] {
                            if c.shown.contains(&format!("<<'{end}'"))
                                || c.shown.contains(&format!("<<{end}"))
                            {
                                let ends = c.shown.lines().filter(|l| *l == end).count();
                                assert_eq!(ends, 1, "{}: a line {end} inside:\n{}", s.id, c.shown);
                            }
                        }
                    }
                    Run::Gcloud(args) => {
                        let rest = c.shown.strip_prefix("gcloud ").unwrap();
                        let out = Command::new("/bin/sh")
                            .arg("-c")
                            .arg(format!(
                                "f() {{ for a; do printf '%s\\037' \"$a\"; done; }}; f {rest}"
                            ))
                            .output()
                            .unwrap();
                        let typed: Vec<String> = String::from_utf8_lossy(&out.stdout)
                            .split('\u{1f}')
                            .filter(|a| !a.is_empty())
                            .map(str::to_string)
                            .collect();
                        assert_eq!(&typed, args, "{}", c.shown);
                    }
                    Run::Hollow(_) => assert!(c.shown.starts_with("Hollow "), "{}", c.shown),
                }
                if c.on == On::Admin || c.on == On::Runner {
                    assert!(c.shown.starts_with(&format!("{SSH} ")), "{}", c.shown);
                }
            }
        }
    }
    for (name, text, end) in [
        ("preflight", PREFLIGHT_SH, "PREFLIGHT"),
        ("proxy", PROXY_SH, "PROXY"),
    ] {
        assert!(!text.lines().any(|l| l == end || l == "HOLLOW"), "{name}");
    }
}

/// Words that write. A check command must contain none of them (after
/// redirects to /dev/null and between descriptors, which write nothing).
fn writes(shown: &str) -> Vec<String> {
    let quiet =
        Regex::new(r"[0-9]?>\s*/dev/null|[0-9]?>&[0-9]|sudo -n true|sudo=(?:yes|no)").unwrap();
    let text = quiet.replace_all(shown, "");
    let deny = Regex::new(
        r"(?x)
        \bsudo\b | > | \btee\b | \bmkdir\b | \brm\b | \bmv\b | \bcp\b | \bchmod\b | \bchown\b
        | \binstall\b | \btouch\b | \bln\b | \bdd\b | \btruncate\b | \bsed\s+-i
        | -create\b | -append\b | -delete\b | -passwd\b | -addUser\b | \bsysadminctl\b
        | \bcreatehomedir\b | \bdseditgroup\s+-o\s+edit\b | \bpmset\s+-a\b | \blaunchctl\b
        | \bkickstart\b | \bssh-keygen\s+-t\b | \bssh-copy-id\b | \bgit\s+init\b | \bbrew\b
        | \bcurl\b | \bnpx\b | \brustup\s+(?:default|toolchain\s+install)\b | \bkill\b | \bpkill\b
        | \bcreate\b | \bdelete\b | \benable\b | \bdisable\b | \bupdate\b | \badd\b | \bremove\b
        | \bstart\b | \bstop\b | \bset\b | \breset\b | \bset-iam-policy\b | \badd-iam-policy-binding\b
        | Hollow\s+(?:writes|records)",
    )
    .unwrap();
    deny.find_iter(&text)
        .map(|m| m.as_str().to_string())
        .collect()
}

/// Check runs only reads: no check command of either path writes, while
/// every Mac step that applies something is caught by the same list (so it
/// isn't empty by accident).
#[test]
fn check_commands_never_write() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    for params in [mac(), cloud()] {
        for s in plan_steps(&ctx, &filled(&ctx, &params)) {
            for c in &s.check {
                let shown = match &c.run {
                    Run::Gcloud(args) => format!("gcloud {}", args.join(" ")),
                    _ => c.shown.clone(),
                };
                assert_eq!(writes(&shown), Vec::<String>::new(), "{}: {shown}", s.id);
            }
            if !s.apply.is_empty() {
                assert!(
                    s.apply.iter().any(|c| !writes(&c.shown).is_empty()),
                    "{}: the deny-list misses its apply",
                    s.id
                );
            }
        }
    }
    assert_eq!(
        writes("ls x 2>/dev/null; echo y >&2; sudo -n true"),
        Vec::<String>::new()
    );
    assert_eq!(writes("echo x > f"), vec![">".to_string()]);
}

#[test]
fn a_plan_fills_defaults_and_shows_placeholders_until_fields_are_valid() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let empty = plan(&ctx, &SetupParams::default());
    assert_eq!(empty.params.path, "mac");
    assert_eq!(empty.params.alias, DEFAULT_ALIAS);
    assert_eq!(empty.params.account, DEFAULT_ACCOUNT);
    assert_eq!(empty.params.key_file, "~/.ssh/id_ed25519_hollow-runner");
    assert!(
        empty.problems.iter().any(|p| p.contains("admin login")),
        "{:?}",
        empty.problems
    );
    let first = &empty.steps[0].check[0].shown;
    assert!(first.contains(" admin@your-mac.local '"), "{first}");
    assert!(check(&ctx, &SetupParams::default()).is_err());
    let hostile = SetupParams {
        account: "x; rm -rf /".into(),
        ..mac()
    };
    let p = plan(&ctx, &hostile);
    assert!(!p.problems.is_empty());
    assert!(p.steps.iter().all(|s| s
        .check
        .iter()
        .chain(&s.apply)
        .all(|c| !c.shown.contains("rm -rf /"))));
    let c = plan(
        &ctx,
        &SetupParams {
            path: "cloud".into(),
            ..SetupParams::default()
        },
    );
    assert_eq!(
        (
            c.params.alias.as_str(),
            c.params.disk_gb,
            c.params.idle_minutes,
            c.params.hours_per_month
        ),
        (DEFAULT_CLOUD_ALIAS, 200, 30, 160)
    );
    assert!(c.estimate.is_some());
    assert_eq!(c.machine_types.len(), MACHINE_TYPES.len());
    assert!(c.machine_types.iter().any(|m| m.arm));
    // The existing runner prefills the alias, and its Host block the account and address.
    std::fs::write(rig.p("slot-remote"), "# x\nold-runner\n").unwrap();
    std::fs::create_dir_all(rig.p("local/.ssh")).unwrap();
    std::fs::write(
        rig.p("local/.ssh/config"),
        "Host old-runner\n  HostName old-mac.local\n  User oldworker\n",
    )
    .unwrap();
    let again = plan(
        &ctx,
        &SetupParams {
            admin: "admin@old-mac.local".into(),
            ..SetupParams::default()
        },
    );
    assert_eq!(
        (
            again.params.alias.as_str(),
            again.params.account.as_str(),
            again.params.host_name.as_str()
        ),
        ("old-runner", "oldworker", "old-mac.local")
    );
    assert_eq!(again.params.key_file, "~/.ssh/id_ed25519_old-runner");
    // ... and the key the block already offers, when its .pub is here (a
    // runner set up by hand with a key named otherwise isn't keyless).
    std::fs::write(
        rig.p("local/.ssh/config"),
        "Host old-runner\n  HostName old-mac.local\n  User oldworker\n  IdentityFile ~/.ssh/id_ed25519_gone\n  IdentityFile ~/.ssh/id_ed25519_old_key\n",
    )
    .unwrap();
    std::fs::write(
        rig.p("local/.ssh/id_ed25519_old_key.pub"),
        "ssh-ed25519 AAAA old\n",
    )
    .unwrap();
    let keyed = plan(
        &ctx,
        &SetupParams {
            admin: "admin@old-mac.local".into(),
            ..SetupParams::default()
        },
    );
    assert_eq!(keyed.params.key_file, "~/.ssh/id_ed25519_old_key");
    assert!(keyed.notes.is_empty(), "{:?}", keyed.notes);
    // A `Host *` IdentityFile above it (which ssh -G lists first) isn't the runner's key.
    std::fs::write(
        rig.p("local/.ssh/config"),
        "Host *\n  IdentityFile ~/.ssh/id_ed25519\n\nHost old-runner\n  HostName old-mac.local\n  User oldworker\n  IdentityFile ~/.ssh/id_ed25519_old_key\n",
    )
    .unwrap();
    std::fs::write(rig.p("local/.ssh/id_ed25519.pub"), "ssh-ed25519 AAAA me\n").unwrap();
    let starred = plan(
        &ctx,
        &SetupParams {
            admin: "admin@old-mac.local".into(),
            ..SetupParams::default()
        },
    );
    assert_eq!(starred.params.key_file, "~/.ssh/id_ed25519_old_key");
    // A new Mac's admin login while the old runner's address is still filled in.
    let new_mac = plan(
        &ctx,
        &SetupParams {
            admin: "admin@new-mac.local".into(),
            ..SetupParams::default()
        },
    );
    assert_eq!(new_mac.params.host_name, "old-mac.local");
    assert_eq!(new_mac.notes.len(), 1, "{:?}", new_mac.notes);
    assert!(
        new_mac.notes[0].starts_with(
            "The admin login goes to new-mac.local, but the runner's address is old-mac.local."
        ),
        "{:?}",
        new_mac.notes
    );
    assert!(new_mac.problems.is_empty(), "{:?}", new_mac.problems);
}

// ---- Judging what a check found ----------------------------------------------

fn out(code: i32, stdout: &str, stderr: &str) -> Outcome {
    Outcome {
        code: Some(code),
        out: stdout.into(),
        err: stderr.into(),
        not_run: None,
    }
}

#[test]
fn checks_are_read_into_states() {
    let p = mac();
    let a = judge_admin(&p, &out(0, "admin\n26.1\narm64\nsudo=no\n", ""));
    assert_eq!(a.state, "done");
    assert!(a.summary.contains("macOS 26.1, arm64"), "{}", a.summary);
    assert!(a.details[0].contains("asks for a password"));
    assert!(
        judge_admin(&p, &out(0, "admin\n26.1\narm64\nsudo=yes\n", "")).details[0]
            .contains("sudo -n")
    );
    let denied = judge_admin(
        &p,
        &out(
            255,
            "",
            "admin@test-mac.local: Permission denied (publickey,password).",
        ),
    );
    assert_eq!(denied.state, "missing");
    assert!(denied.details[0].contains("ssh-copy-id admin@test-mac.local"));
    let down = judge_admin(
        &p,
        &out(
            255,
            "",
            "ssh: connect to host x port 22: Operation timed out",
        ),
    );
    assert_eq!(down.state, "unknown");
    assert_eq!(
        judge_admin(&p, &out(0, "admin\n\nx86_64\nsudo=no\n", "")).state,
        "missing"
    );
    assert_eq!(
        judge_admin(&p, &Outcome::skipped("x".into())).state,
        "unknown"
    );

    assert_eq!(
        judge_account(&p, &out(0, "account=none\n", "")).state,
        "missing"
    );
    let fine = judge_account(
        &p,
        &out(
            0,
            "account=yes\nhidden=yes\nadmin=no\nssh_group=open\nhome=yes\n",
            "",
        ),
    );
    assert_eq!(fine.state, "done");
    let bad = judge_account(
        &p,
        &out(
            0,
            "account=yes\nhidden=no\nadmin=yes\nssh_group=no\nhome=yes\n",
            "",
        ),
    );
    assert_eq!(bad.state, "missing");
    assert_eq!(bad.details.len(), 3, "{:?}", bad.details);

    let tools = judge_tools(&out(0, "clt=yes\ngit=git version 2.50.1\nnode=v24.1.0\ncargo=cargo 1.96.0\nrustc=rustc 1.96.0\nuv=uv 0.9\npython3=Python 3.9.6\ngh=\ngcloud=Google Cloud SDK 592.0.0\ntoolchains=1.95.0-aarch64-apple-darwin \npg_ctl=pg_ctl (PostgreSQL) 17.6\nchromium=chromium-1228\n", ""));
    assert_eq!(tools.state, "missing");
    assert_eq!(
        tools.details[0],
        format!("Not there: gh, Rust {RUST_TOOLCHAIN}.")
    );
    let all = judge_tools(&out(0, "clt=yes\ngit=g\nnode=n\ncargo=c\nrustc=r\nuv=u\npython3=p\ngh=g\ngcloud=g\ntoolchains=1.96.0-aarch64-apple-darwin (default)\npg_ctl=p\nchromium=chromium-1228\n", ""));
    assert_eq!(all.state, "done");

    assert_eq!(
        judge_preflight(&out(0, "preflight=none\n", "")).state,
        "missing"
    );
    let pre = judge_preflight(&out(0, "preflight=/h/.local/bin/hollow-run-preflight\nno: 12GB free disk on m, under 20GB\nexit=1\n", ""));
    assert_eq!(pre.state, "done");
    assert!(
        pre.details[1].starts_with("It refuses work now (no: 12GB"),
        "{:?}",
        pre.details
    );

    let asleep = judge_sleep(&out(0, " sleep                1 (sleep prevented by sharingd)\n disksleep            10\n autorestart          0\n---\n", ""));
    assert_eq!(asleep.state, "missing");
    assert_eq!(asleep.details.len(), 4, "{:?}", asleep.details);
    let awake = judge_sleep(&out(0, " sleep                0\n disksleep            0\n autorestart          1\n---\nClientAliveInterval 30\nClientAliveCountMax 4\n", ""));
    assert_eq!(awake.state, "done");
}

#[test]
fn the_runner_in_use_is_judged_from_slot_remote_and_its_status() {
    let p = mac();
    let ready = serde_json::json!({ "state": "ready", "preflight": "ok host=m", "owner": { "state": "you", "warnings": [] } });
    let s = judge_use(
        &p,
        &[out(0, "test-runner", ""), out(0, &ready.to_string(), "")],
    );
    assert_eq!(s.state, "done", "{:?}", s);
    let other = serde_json::json!({ "state": "ready", "owner": { "state": "none", "warnings": ["1 other SSH key can log in"] } });
    let s = judge_use(&p, &[out(0, "", ""), out(0, &other.to_string(), "")]);
    assert_eq!(s.state, "missing");
    assert_eq!(s.summary, "Not in use yet.");
    assert_eq!(s.details.len(), 3, "{:?}", s.details);
    // Named and ready, only the owner record missing: in use already.
    let unowned =
        serde_json::json!({ "state": "ready", "owner": { "state": "none", "warnings": [] } });
    let s = judge_use(
        &p,
        &[out(0, "test-runner", ""), out(0, &unowned.to_string(), "")],
    );
    assert_eq!(s.state, "missing");
    assert_eq!(
        s.summary,
        "test-runner is this Mac's runner, with something left to do."
    );
    assert_eq!(
        s.details,
        vec!["test-runner has no owner recorded (Apply records this Hollow).".to_string()]
    );
    let down =
        serde_json::json!({ "state": "unreachable", "message": "test-runner didn't answer." });
    assert_eq!(
        judge_use(
            &p,
            &[out(0, "test-runner", ""), out(255, &down.to_string(), "")]
        )
        .state,
        "unknown"
    );
}

// ---- Check and apply against the rig -----------------------------------------

/// A Mac from nothing to in use: Check first changes nothing, each Apply
/// does its step (and only through the admin tools' stand-ins), and Check
/// at the end finds every step done.
#[test]
fn a_mac_is_set_up_step_by_step() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    std::fs::create_dir_all(rig.p("local/.ssh")).unwrap();
    std::fs::write(
        rig.p("local/.ssh/config"),
        "Host elsewhere\n  User someone\n",
    )
    .unwrap();
    let p = mac();

    let first = check(&ctx, &p).unwrap();
    let states: Vec<(&str, &str)> = first.iter().map(|s| (s.id.as_str(), s.state)).collect();
    assert_eq!(
        states,
        vec![
            ("admin", "done"),
            ("account", "missing"),
            ("key", "missing"),
            ("tools", "unknown"),
            ("preflight", "unknown"),
            ("sleep", "missing"),
            ("use", "missing")
        ],
        "{first:#?}"
    );
    assert!(
        by_id(&first, "tools").summary.contains("not asked"),
        "{:?}",
        by_id(&first, "tools")
    );
    // With no Host block for the alias, nothing was offered to that name.
    assert!(
        !rig.read("ssh.log").lines().any(|l| l == "test-runner"),
        "{}",
        rig.read("ssh.log")
    );
    // Check changed nothing: no admin tool was asked to write, no files.
    let log = rig.read("tools.log");
    assert!(
        !log.contains("sysadminctl") && !log.contains("pmset -a") && !log.contains("launchctl"),
        "{log}"
    );
    assert!(!rig.p("local/.ssh/id_ed25519_test-runner").exists());
    assert!(!rig.p("slot-remote").exists());

    let account = apply(&ctx, &p, "account").unwrap();
    assert_eq!(account.state, "done", "{account:#?}");
    let log = rig.read("tools.log");
    let add = log
        .lines()
        .find(|l| l.starts_with("sysadminctl -addUser testworker"))
        .unwrap();
    let pw = add
        .split(" -password ")
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap();
    assert_eq!(pw.len(), 32, "{add}");
    assert!(log.contains("dscl . -create /Users/testworker IsHidden 1"));
    assert!(!account
        .ran
        .iter()
        .any(|r| r.shown.contains(pw) || r.output.contains(pw)));
    // The password it keeps replaced that one through dscl's stdin: it was
    // in no command's argv (so in no ps line or sudo log) and isn't shown.
    let kept = rig.read("state/dscl-stdin");
    assert!(kept.starts_with("passwd /Users/testworker "), "{kept}");
    let kept_pw = kept.trim_end().rsplit(' ').next().unwrap();
    assert_eq!(kept_pw.len(), 32, "{kept}");
    assert_ne!(kept_pw, pw);
    assert!(!rig.read("tools.log").contains(kept_pw));
    assert!(!account
        .ran
        .iter()
        .any(|r| r.shown.contains(kept_pw) || r.output.contains(kept_pw)));

    let key = apply(&ctx, &p, "key").unwrap();
    assert_eq!(key.state, "done", "{key:#?}");
    // Its files are written as the runner account, never by root there.
    let log = rig.read("tools.log");
    assert!(
        log.contains("sudo -n -u testworker /bin/sh -c umask 077;"),
        "{log}"
    );
    assert!(
        !log.contains("sudo -n tee") && !log.contains("chown"),
        "{log}"
    );
    let config = rig.read("local/.ssh/config");
    assert!(
        config.starts_with("Host elsewhere\n  User someone\n"),
        "{config}"
    );
    assert!(config.contains("\nHost test-runner\n  HostName test-mac.local\n  User testworker\n  IdentityFile ~/.ssh/id_ed25519_test-runner\n  IdentitiesOnly yes\n  ForwardAgent no\n"), "{config}");
    assert_eq!(
        rig.read("local/.ssh/config.bak-hollow-20261103"),
        "Host elsewhere\n  User someone\n"
    );
    let keys = rig.read("root/Users/testworker/.ssh/authorized_keys");
    assert_eq!(keys.lines().count(), 1, "{keys}");
    assert!(
        keys.starts_with("ssh-ed25519 ") && keys.trim_end().ends_with("hollow runner test-runner"),
        "{keys}"
    );
    let mode = |rel: &str| std::fs::metadata(rig.p(rel)).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        (
            mode("root/Users/testworker/.ssh"),
            mode("root/Users/testworker/.ssh/authorized_keys")
        ),
        (0o700, 0o600)
    );
    // Again: the Host block is there already and the key isn't added twice.
    let again = apply(&ctx, &p, "key").unwrap();
    assert_eq!(again.state, "done");
    assert!(
        again.ran[1]
            .output
            .starts_with("Not run: ~/.ssh/config already has"),
        "{:?}",
        again.ran[1]
    );
    assert_eq!(rig.read("root/Users/testworker/.ssh/authorized_keys"), keys);

    let pre = apply(&ctx, &p, "preflight").unwrap();
    assert_eq!(pre.state, "done", "{pre:#?}");
    let installed = rig.p("root/Users/testworker/.local/bin/hollow-run-preflight");
    assert_eq!(std::fs::read_to_string(&installed).unwrap(), PREFLIGHT_SH);
    assert_eq!(
        mode("root/Users/testworker/.local/bin/hollow-run-preflight"),
        0o755
    );
    // From here a preflight that always says yes, whatever this machine's load.
    write_exec(
        &installed,
        "#!/bin/sh\necho 'ok host=rig free=200GB mem=80% rust=1.96.0 node=v24.0.0'\n",
    );

    let sleep = apply(&ctx, &p, "sleep").unwrap();
    assert_eq!(sleep.state, "done", "{sleep:#?}");
    assert_eq!(
        rig.read("root/etc/ssh/sshd_config.d/100-hollow-keepalive.conf")
            .lines()
            .skip(1)
            .collect::<Vec<_>>(),
        vec!["ClientAliveInterval 30", "ClientAliveCountMax 4"]
    );
    // sshd starts per connection on macOS: nothing restarts it.
    assert!(!rig.read("tools.log").contains("launchctl"));

    let used = apply(&ctx, &p, "use").unwrap();
    assert_eq!(used.state, "done", "{used:#?}");
    assert!(rig.read("slot-remote").contains("\ntest-runner\n"));
    assert!(rig
        .p("root/Users/testworker/hollow-remote/owner.json")
        .is_file());
    let call = used
        .ran
        .iter()
        .find(|r| r.shown.contains("--on test-runner sh -c"))
        .unwrap();
    assert!(call.output.contains("cargo 1.96.0"), "{call:?}");
    assert_eq!(
        rig.read("slot.log").trim(),
        "--on test-runner sh -c uname -sm; node --version; cargo --version"
    );
    // Named last, after the status, the owner record and the test call.
    let order: Vec<&str> = used.ran[..5]
        .iter()
        .map(|r| match r.shown.strip_prefix("Hollow ") {
            Some(rest) => rest.split(' ').next().unwrap_or(""),
            None => "the test call",
        })
        .collect();
    assert_eq!(
        order,
        ["asks", "records", "puts", "the test call", "writes"],
        "{:#?}",
        &used.ran[..5]
    );

    let last = check(&ctx, &p).unwrap();
    for id in ["admin", "account", "key", "preflight", "sleep", "use"] {
        assert_eq!(by_id(&last, id).state, "done", "{:#?}", by_id(&last, id));
    }
    assert!(
        matches!(by_id(&last, "tools").state, "done" | "missing"),
        "{:?}",
        by_id(&last, "tools")
    );
}

/// A Host block for the alias that says otherwise is never rewritten: the
/// step stops before anything else, and ~/.ssh/config stays as it was.
#[test]
fn a_different_host_block_is_refused() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    std::fs::create_dir_all(rig.p("local/.ssh")).unwrap();
    let mine = "Host other test-runner\n  HostName elsewhere.local\n  User testworker\n";
    std::fs::write(rig.p("local/.ssh/config"), mine).unwrap();
    let s = apply(&ctx, &mac(), "key").unwrap();
    assert_eq!(s.state, "failed");
    assert!(
        s.summary.contains("already has a Host test-runner block")
            && s.summary
                .contains("HostName elsewhere.local, not test-mac.local"),
        "{}",
        s.summary
    );
    assert_eq!(rig.read("local/.ssh/config"), mine);
    assert!(!rig.p("local/.ssh/config.bak-hollow-20261103").exists());
    assert!(
        !rig.read("tools.log").contains("tee"),
        "the key wasn't installed"
    );
}

/// sudo wanting a password stops the step before it changes anything there,
/// and says to run the by-hand commands.
#[test]
fn sudo_wanting_a_password_stops_with_the_by_hand_commands() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("sudo-password");
    let s = apply(&ctx, &mac(), "account").unwrap();
    assert_eq!(s.state, "failed");
    assert!(
        s.needs_by_hand && s.summary.contains("wants a password"),
        "{s:#?}"
    );
    assert!(!rig
        .read("tools.log")
        .lines()
        .any(|l| l.starts_with("sysadminctl")));
    let steps = plan_steps(&ctx, &filled(&ctx, &mac()));
    let hand = &step(&steps, "account").by_hand;
    assert!(
        hand[0]
            .shown
            .starts_with("ssh admin@test-mac.local 'cat > ~/hollow-setup-account.sh' <<'HOLLOW'"),
        "{}",
        hand[0].shown
    );
    assert_eq!(hand[1].shown, "ssh -t admin@test-mac.local 'sudo /bin/sh ~/hollow-setup-account.sh; rm -f ~/hollow-setup-account.sh'");
    let key_hand = &step(&steps, "key").by_hand[1].shown;
    assert!(key_hand.contains(r"sudo /bin/sh ~/hollow-setup-key.sh testworker $(awk '/^ssh-ed25519 [A-Za-z0-9+\/=]+( |$)/ { print $1, $2; exit }' ~/.ssh/id_ed25519_test-runner.pub)"), "{key_hand}");
}

/// Only a well-formed ed25519 line leaves this Mac for the key step: a
/// .pub with shell in it sends nothing, and the script there refuses.
#[test]
fn a_public_key_with_shell_in_it_sends_nothing() {
    let rig = Rig::new();
    std::fs::create_dir_all(rig.p("local/.ssh")).unwrap();
    for (line, want) in [
        (
            "ssh-ed25519 AAAAC3Nz+/= me@mac\n",
            "ssh-ed25519 AAAAC3Nz+/=",
        ),
        ("ssh-ed25519 AAAAC3Nz+/=\n", "ssh-ed25519 AAAAC3Nz+/="),
        ("ssh-ed25519 AAAA;touch${IFS}x\n", ""),
        ("ssh-ed25519 AAAA$(touch x)\n", ""),
        ("ssh-rsa AAAAB3 me\n", ""),
    ] {
        std::fs::write(rig.p("local/.ssh/k.pub"), line).unwrap();
        // The very words the key step puts on its command lines.
        let out = Command::new("/bin/sh")
            .arg("-c")
            .arg(format!("echo {}", pub_key_words("~/.ssh/k")))
            .current_dir(rig.p("local"))
            .env("HOME", rig.p("local"))
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), want, "{line}");
        assert!(!rig.p("local/x").exists());
    }
    let steps = plan_steps(&rig.ctx(), &filled(&rig.ctx(), &mac()));
    let key = step(&steps, "key");
    let words = pub_key_words("~/.ssh/id_ed25519_test-runner");
    assert!(key.apply[2].shown.contains(&format!("testworker {words}")));
    assert!(key.by_hand[1]
        .shown
        .contains(&format!("testworker {words}")));
}

/// An existing standard account that Hollow didn't make (its full name
/// isn't Hollow runner) is hidden and admitted, and keeps its password.
#[test]
fn an_account_hollow_didnt_make_keeps_its_password() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("user-testworker");
    std::fs::create_dir_all(rig.p("root/Users/testworker")).unwrap();
    let s = apply(&ctx, &mac(), "account").unwrap();
    assert_eq!(s.state, "done", "{s:#?}");
    assert!(!rig.p("state/dscl-stdin").exists());
    let log = rig.read("tools.log");
    assert!(
        !log.contains("sysadminctl") && log.contains("dscl . -create /Users/testworker IsHidden 1"),
        "{log}"
    );
}

/// A password swap that fails says so without dscl's output, and the next
/// Apply swaps it again (the throwaway in sysadminctl's argv never stays).
#[test]
fn a_failed_password_swap_is_redone_and_never_shown() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("dscl-fail");
    let s = apply(&ctx, &mac(), "account").unwrap();
    assert_eq!(s.state, "failed", "{s:#?}");
    assert!(
        s.summary.contains("dscl couldn't set its password"),
        "{}",
        s.summary
    );
    let first = rig.read("state/dscl-stdin");
    let pw = first.trim_end().rsplit(' ').next().unwrap().to_string();
    assert!(!s.ran.iter().any(|r| r.output.contains(&pw)));
    assert!(rig.p("state/user-testworker").exists());
    std::fs::remove_file(rig.p("state/dscl-fail")).unwrap();
    let again = apply(&ctx, &mac(), "account").unwrap();
    assert_eq!(again.state, "done", "{again:#?}");
    let second = rig.read("state/dscl-stdin");
    assert!(second.starts_with("passwd /Users/testworker ") && second != first);
    assert_eq!(
        rig.read("tools.log")
            .lines()
            .filter(|l| l.starts_with("sysadminctl"))
            .count(),
        1
    );
}

/// A runner that doesn't answer is never named: Use it stops at the status
/// check, before the owner record and before slot-remote changes.
#[test]
fn use_it_names_the_runner_only_after_it_answers() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    std::fs::write(rig.p("slot-remote"), "old-runner\n").unwrap();
    std::fs::create_dir_all(rig.p("local/.ssh")).unwrap();
    std::fs::write(
        rig.p("local/.ssh/config"),
        "Host test-runner\n  HostName test-mac.local\n  User testworker\n  IdentityFile ~/.ssh/id_ed25519_test-runner\n",
    )
    .unwrap();
    let s = apply(&ctx, &mac(), "use").unwrap();
    assert_eq!(s.state, "failed", "{s:#?}");
    assert!(
        s.summary.starts_with("Command 1 of 5 failed"),
        "{}",
        s.summary
    );
    assert_eq!(rig.read("slot-remote"), "old-runner\n");
    assert!(!rig.p("slot.log").exists());
    assert!(!rig
        .p("root/Users/testworker/hollow-remote/owner.json")
        .exists());
}

/// sshd -t refusing the new setting removes it again and leaves sshd alone.
#[test]
fn a_keepalive_sshd_refuses_is_removed_again() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("sshd-bad");
    let s = apply(&ctx, &mac(), "sleep").unwrap();
    assert_eq!(s.state, "failed", "{s:#?}");
    assert!(s.summary.contains("sshd -t refused"), "{}", s.summary);
    assert!(!rig
        .p("root/etc/ssh/sshd_config.d/100-hollow-keepalive.conf")
        .exists());
    assert!(!rig.read("tools.log").contains("launchctl"));
}

/// An admin login this Mac's key doesn't open: Check says so and asks
/// nothing else of it; the runner steps aren't asked either.
#[test]
fn a_closed_admin_login_skips_what_depends_on_it() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("admin-locked");
    let states = check(&ctx, &mac()).unwrap();
    assert_eq!(by_id(&states, "admin").state, "missing");
    assert_eq!(by_id(&states, "account").state, "unknown");
    assert!(
        by_id(&states, "account")
            .summary
            .contains("not asked: the admin login didn't work"),
        "{:?}",
        by_id(&states, "account")
    );
    assert_eq!(
        rig.read("ssh.log")
            .lines()
            .filter(|l| l.contains('@'))
            .count(),
        1
    );
}

/// hollow-slot falling back to this Mac is a failed test call, not a pass.
#[test]
fn a_test_call_that_ran_here_fails() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    rig.mark("slot-fallback");
    let p = mac();
    let steps = plan_steps(&ctx, &filled(&ctx, &p));
    let call = step(&steps, "use")
        .apply
        .iter()
        .find(|c| matches!(c.run, Run::Hollow(Builtin::TestCall)))
        .unwrap()
        .clone();
    let o = run_cmd(&ctx, &p, &call);
    assert_eq!(o.code, Some(1));
    assert!(
        o.err
            .contains("hollow-slot ran it on this Mac, not on test-runner"),
        "{}",
        o.err
    );
}

// ---- Google Cloud --------------------------------------------------------------

#[test]
fn the_estimate_is_the_machine_at_its_hours_plus_the_disk() {
    let e = estimate(&SetupParams {
        hours_per_month: 160,
        disk_gb: 200,
        ..cloud()
    })
    .unwrap();
    // 42.88 + 20.00 + NAT (0.0014 x 160 + 3.65 for its address) = 66.75:
    // the total is the sum of the lines as shown.
    assert_eq!(
        (e.total_cents, e.compute_usd, e.disk_usd, e.nat_usd),
        (6675, 42.88, 20.0, 3.87)
    );
    assert_eq!(
        (e.region.as_str(), e.prices_region, e.prices_taken),
        ("us-central1", "us-central1", PRICES_TAKEN)
    );
    let arm = estimate(&SetupParams {
        machine_type: "t2a-standard-8".into(),
        hours_per_month: 100,
        disk_gb: 50,
        ..cloud()
    })
    .unwrap();
    assert_eq!(arm.total_cents, 3959);
    let small = estimate(&SetupParams {
        machine_type: "e2-standard-4".into(),
        hours_per_month: 1,
        disk_gb: 20,
        ..cloud()
    })
    .unwrap();
    assert_eq!(small.total_cents, 578);
    assert!(estimate(&SetupParams {
        machine_type: "m3-ultramem-128".into(),
        ..cloud()
    })
    .is_err());
    assert_eq!(usd(6288), "$62.88");
}

/// Nothing runs without the estimate the person was shown: not a wrong one,
/// not Apply, not invalid fields.
#[test]
fn create_runs_nothing_without_the_shown_estimate() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let p = filled(&ctx, &cloud());
    let right = estimate(&p).unwrap().total_cents;
    let wrong = create_cloud(&ctx, &p, right + 1).unwrap_err();
    assert!(wrong.contains("isn't this machine's"), "{wrong}");
    assert!(apply(&ctx, &p, "cloud").unwrap_err().contains("Create"));
    let bad = SetupParams {
        billing_account: "nope".into(),
        ..p.clone()
    };
    assert!(create_cloud(&ctx, &bad, right).is_err());
    assert!(create_cloud(&ctx, &mac(), right).is_err());
    assert!(
        !rig.p("gcloud.log").exists(),
        "gcloud ran: {}",
        rig.read("gcloud.log")
    );
    assert!(!ctx.startup_script_path().exists());
}

/// With the shown estimate, Create runs exactly the plan's commands in order
/// (the startup script written, APIs, budget, service account, firewall,
/// router and NAT, VM, the script removed), then its check.
#[test]
fn create_runs_the_planned_gcloud_commands_in_order() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let p = filled(&ctx, &cloud());
    let steps = plan_steps(&ctx, &p);
    let apply = &step(&steps, "cloud").apply;
    // The startup script is shown: written by the first command, removed by the last.
    assert!(
        apply[0].on == On::Local && apply[0].shown.contains(&startup_script()),
        "{}",
        apply[0].shown
    );
    assert!(apply.last().unwrap().shown.starts_with("rm -f "));
    let planned: Vec<Vec<String>> = apply[1..apply.len() - 1]
        .iter()
        .map(|c| match &c.run {
            Run::Gcloud(a) => a.clone(),
            _ => panic!("not gcloud: {}", c.shown),
        })
        .collect();
    let s = create_cloud(&ctx, &p, estimate(&p).unwrap().total_cents).unwrap();
    assert_eq!(s.state, "done", "{s:#?}");
    let calls = rig.gcloud_calls();
    let n = planned.len();
    assert_eq!(&calls[..n], &planned[..]);
    let verbs: Vec<String> = calls[..n].iter().map(|c| c[..3].join(" ")).collect();
    assert_eq!(
        verbs,
        [
            "services enable compute.googleapis.com",
            "billing budgets list",
            "billing budgets create",
            "iam service-accounts create",
            "compute firewall-rules create",
            "compute routers create",
            "compute routers nats",
            "compute routers nats",
            "compute instances create"
        ]
    );
    // The NAT's describe found none, and Create went on to make it.
    assert_eq!(calls[6][3], "describe");
    assert!(s.ran[7].output.contains("Not there yet"), "{:?}", s.ran[7]);
    // gcloud read the very script shown, from a file gone afterwards.
    assert_eq!(rig.read("gstate/startup-copy"), startup_script());
    assert!(
        calls[7][3] == "create"
            && calls[7].iter().any(|a| a == "--nat-all-subnet-ip-ranges")
            && calls[7].iter().any(|a| a == "--region=us-central1")
            && calls[7]
                .iter()
                .any(|a| a == "--router=hollow-runner-router")
    );
    let vm = &calls[8];
    for flag in [
        "--no-address",
        "--no-scopes",
        "--shielded-secure-boot",
        "--image-family=debian-12",
        "--machine-type=e2-standard-8",
        "--boot-disk-size=200GB",
        "--metadata=enable-oslogin=TRUE,hollow-idle-minutes=30",
        "--service-account=hollow-runner@my-project-123.iam.gserviceaccount.com",
        "--tags=hollow-runner",
    ] {
        assert!(vm.iter().any(|a| a == flag), "{flag} in {vm:?}");
    }
    assert!(vm
        .iter()
        .any(|a| a.starts_with("--metadata-from-file=startup-script=")
            && a.ends_with("hollow-runner-startup.sh")));
    assert!(
        calls[4]
            .iter()
            .any(|a| a == "--source-ranges=35.235.240.0/20")
            && calls[4].iter().any(|a| a == "--rules=tcp:22")
    );
    assert!(
        calls[2].iter().any(|a| a == "--threshold-rule=percent=0.5")
            && calls[2].iter().any(|a| a == "--threshold-rule=percent=0.9")
            && calls[2].iter().any(|a| a == "--budget-amount=80USD")
    );
    // The check after it: reads only.
    assert!(
        calls[n..].iter().all(|c| writes(&c.join(" ")).is_empty()),
        "{:?}",
        &calls[n..]
    );
    assert!(!ctx.startup_script_path().exists());

    // Again: everything is there; neither the budget alert nor the NAT is
    // made twice (a second NAT create fails as a duplicate, not "already
    // exists", so without the skip Create would stop there).
    let again = create_cloud(&ctx, &p, estimate(&p).unwrap().total_cents).unwrap();
    assert_eq!(again.state, "done", "{again:#?}");
    let count = |verb: &[&str]| {
        rig.gcloud_calls()
            .iter()
            .filter(|c| c.len() >= verb.len() && c.iter().zip(verb).all(|(a, b)| a == b))
            .count()
    };
    assert_eq!(count(&["billing", "budgets", "create"]), 1);
    assert_eq!(count(&["compute", "routers", "nats", "create"]), 1);
    assert!(
        again.ran[3].output.starts_with("Not run: the budget alert"),
        "{:?}",
        again.ran[3]
    );
    assert!(
        again.ran[8].output.starts_with("Not run: the Cloud NAT"),
        "{:?}",
        again.ran[8]
    );
}

/// A failing command stops the sequence and is named; what is already
/// there counts as done.
#[test]
fn a_failure_stops_create_and_says_which_command() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let p = filled(&ctx, &cloud());
    let cents = estimate(&p).unwrap().total_cents;
    std::fs::write(rig.p("gstate/fail"), "firewall-rules create").unwrap();
    std::fs::write(rig.p("gstate/sa"), "").unwrap();
    let s = create_cloud(&ctx, &p, cents).unwrap();
    assert_eq!(s.state, "failed");
    assert!(
        s.summary
            .starts_with("Command 6 of 11 failed (exit 1: ERROR: (gcloud) PERMISSION_DENIED"),
        "{}",
        s.summary
    );
    assert!(
        s.details[0].contains("gcloud compute firewall-rules create hollow-runner-iap-ssh"),
        "{:?}",
        s.details
    );
    assert!(s.ran[4].output.contains("Already there"), "{:?}", s.ran[4]);
    let calls = rig.gcloud_calls();
    assert_eq!(calls.len(), 5, "{calls:?}");
    assert!(!calls
        .iter()
        .any(|c| c.starts_with(&["compute".to_string(), "instances".to_string()])));
    // The startup script's removal ran all the same (shown, last).
    assert!(s.ran.last().unwrap().shown.starts_with("rm -f "));
    assert!(!ctx.startup_script_path().exists());
}

/// Check on Google Cloud reads only, and never asks a VM that isn't
/// running (the ProxyCommand would start it).
#[test]
fn a_cloud_check_never_starts_the_vm() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let states = check(&ctx, &cloud()).unwrap();
    assert_eq!(by_id(&states, "gcloud").state, "done");
    assert_eq!(by_id(&states, "cloud").state, "missing");
    let used = by_id(&states, "use");
    assert!(
        used.ran[1].output.contains("not asked: the VM isn't there"),
        "{used:#?}"
    );
    assert!(!rig.p("ssh.log").exists());
    assert!(rig
        .gcloud_calls()
        .iter()
        .all(|c| writes(&format!("gcloud {}", c.join(" "))).is_empty()));
    std::fs::write(rig.p("gstate/signed-out"), "").unwrap();
    let out = check(&ctx, &cloud()).unwrap();
    assert_eq!(by_id(&out, "gcloud").state, "missing");
    assert!(
        by_id(&out, "cloud")
            .details
            .iter()
            .any(|d| d.contains("not asked: gcloud isn't signed in")),
        "{:?}",
        by_id(&out, "cloud")
    );
}

/// This Mac's way in: the key, the OS Login key, the proxy helper and a
/// Host block whose User is the OS Login name.
#[test]
fn the_way_in_to_a_cloud_runner() {
    let rig = Rig::new();
    let ctx = rig.ctx();
    let s = apply(&ctx, &cloud(), "cloud-local").unwrap();
    assert_eq!(s.state, "done", "{s:#?}");
    let config = rig.read("local/.ssh/config");
    assert!(
        config.contains("Host hollow-gcp\n  HostName hollow-runner\n  User someone_example_com\n"),
        "{config}"
    );
    assert!(config.contains("  ProxyCommand ~/.config/hollow/gcp-runner-proxy hollow-runner my-project-123 us-central1-a\n"));
    assert!(config.contains("  StrictHostKeyChecking accept-new\n"));
    assert_eq!(rig.read("local/.config/hollow/gcp-runner-proxy"), PROXY_SH);
    assert!(rig.p("gstate/oslogin-key").exists());
    let add = rig
        .gcloud_calls()
        .into_iter()
        .find(|c| c.starts_with(&["compute".into(), "os-login".into(), "ssh-keys".into()]))
        .unwrap();
    assert!(
        add.iter().any(|a| a
            == &format!(
                "--key-file={}",
                rig.s("local/.ssh/id_ed25519_hollow-gcp.pub")
            )),
        "{add:?}"
    );
}

// ---- The scripts -------------------------------------------------------------

#[test]
fn the_scripts_parse() {
    let dir = tempfile::tempdir().unwrap();
    for (name, shell, text) in [
        ("startup.sh", "bash", startup_script()),
        ("proxy.sh", "/bin/sh", PROXY_SH.to_string()),
        ("preflight.sh", "/bin/sh", PREFLIGHT_SH.to_string()),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, &text).unwrap();
        let ok = Command::new(shell).arg("-n").arg(&path).status().unwrap();
        assert!(ok.success(), "{name} doesn't parse");
    }
    let startup = startup_script();
    assert!(
        !startup.contains("@PREFLIGHT@")
            && !startup.contains("@RUST@")
            && !startup.contains("@PLAYWRIGHT@")
    );
    assert!(
        startup.contains(&format!("--default-toolchain {RUST_TOOLCHAIN}"))
            && startup.contains(&format!(
                "playwright@{PLAYWRIGHT_VERSION} install --with-deps chromium"
            ))
    );
    assert!(
        startup.contains("ok host=$host free=${gb}GB"),
        "the preflight is inside"
    );
    // The idle power-off is set up before any install, and a failed
    // install doesn't end the script before it (the VM still stops).
    let timer = startup
        .find("systemctl enable --now hollow-idle.timer")
        .unwrap();
    assert!(timer < startup.find("apt-get update").unwrap());
    assert!(!startup.contains("|| exit 1"), "{startup}");
    // ... and the install counts as busy (bounded), so the VM doesn't stop mid-install.
    assert!(startup.contains("find /run/hollow-installing -mmin -90"));
    let marked = startup.find("touch /run/hollow-installing").unwrap();
    let call = startup.find("  if install_tools; then").unwrap();
    assert!(marked < call && call < startup.find("rm -f /run/hollow-installing").unwrap());
    // A failed download fails its install; a half-done dpkg is finished first.
    assert!(startup.contains("\nset -o pipefail\n"));
    assert!(startup.find("dpkg --configure -a").unwrap() < startup.find("apt-get update").unwrap());
    // rustup already there (a boot stopped part way): the toolchain is installed again.
    assert!(startup.contains(&format!(
        "/opt/cargo/bin/rustup toolchain install {RUST_TOOLCHAIN} --profile minimal"
    )));
}

/// The default preflight prints one line in the status's shape.
#[test]
fn the_default_preflight_says_ok_or_no() {
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(PREFLIGHT_SH)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.trim();
    let shape =
        Regex::new(r"^(ok host=\S+ free=\d+GB mem=\S+% rust=\S+ node=\S+|no: .+)$").unwrap();
    assert!(shape.is_match(line), "{line}");
    assert_eq!(out.status.success(), line.starts_with("ok "));
}

/// The proxy starts a stopped VM, waits for it, then hands ssh the tunnel.
#[test]
fn the_proxy_starts_a_stopped_vm_then_tunnels() {
    let rig = Rig::new();
    std::fs::write(rig.p("gstate/vm"), "TERMINATED\n").unwrap();
    let out = Command::new("/bin/sh")
        .arg("-c")
        .arg(PROXY_SH)
        .args([
            "gcp-runner-proxy",
            "hollow-runner",
            "my-project-123",
            "us-central1-a",
        ])
        .env("PATH", rig.ctx().path)
        .env("HOME", rig.p("local"))
        .env("HOLLOW_PROXY_BOOT_WAIT", "0")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "SSH-2.0-stand-in"
    );
    let verbs: Vec<String> = rig
        .gcloud_calls()
        .iter()
        .map(|c| c[..2].join(" "))
        .collect();
    assert_eq!(
        verbs,
        [
            "compute instances",
            "compute instances",
            "compute instances",
            "compute start-iap-tunnel"
        ]
    );
    assert_eq!(rig.gcloud_calls()[1][2], "start");
}

/// Check against a real Mac, read-only: `HOLLOW_SETUP_IT_ADMIN` (the admin
/// login), `HOLLOW_SETUP_IT_HOST` (the runner's SSH alias) and
/// `HOLLOW_SETUP_IT_ACCOUNT` (its runner account). Prints each step's state.
/// `cargo test --lib runner_setup::tests::real_mac_setup_check -- --ignored --nocapture`
#[test]
#[ignore = "reads a real Mac over ssh; set HOLLOW_SETUP_IT_ADMIN, HOLLOW_SETUP_IT_HOST and HOLLOW_SETUP_IT_ACCOUNT, then cargo test --lib real_mac_setup_check -- --ignored --nocapture"]
fn real_mac_setup_check() {
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("set {name}"));
    let ctx = Ctx::real().unwrap();
    let params = SetupParams {
        path: "mac".into(),
        admin: var("HOLLOW_SETUP_IT_ADMIN"),
        alias: var("HOLLOW_SETUP_IT_HOST"),
        account: var("HOLLOW_SETUP_IT_ACCOUNT"),
        ..SetupParams::default()
    };
    let started = Instant::now();
    for s in check(&ctx, &params).unwrap() {
        println!("{:<10} {:<8} {}", s.id, s.state, s.summary);
        for d in &s.details {
            println!("{:<19} {d}", "");
        }
    }
    println!("checked in {} ms", started.elapsed().as_millis());
}
