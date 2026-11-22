//! Setting up a remote runner (remote runners 3, 2026-11-03): Settings,
//! Devices → Remote runner → Set up. Two paths, each a list of
//! steps ([`plan`]):
//!
//! - **A Mac you own, always on** (`mac`): an admin login used only for
//!   setup, a hidden standard runner account, a key of its own, the
//!   toolchains, Hollow's default `hollow-run-preflight`, no sleep and an SSH
//!   keepalive, then naming it the runner (`runner_owner::set_runner`,
//!   `record_owner`) with a test call through `hollow-slot --remote`.
//! - **A Google Cloud machine** (`cloud`): a Debian 12 VM with no public
//!   address, reached only through IAP, with a service account that holds no
//!   roles, an idle power-off timer and a budget alert, then this Mac's way
//!   in (a ProxyCommand helper that starts it) and the same last step.
//!
//! Every step has its own **check** commands (read-only: a unit test holds
//! them to a deny-list of writes) and **apply** commands, and what a step
//! shows is exactly what runs: a shell command line run with `/bin/sh -c`
//! on this Mac (an ssh call to the admin or runner account included), a
//! gcloud argv shown shell-quoted, or a named action of Hollow's own.
//! [`check`] runs every step's check in order and changes nothing;
//! [`apply`] runs one step's apply, then its check, and is the only way
//! anything changes. Every process is bounded and ends with its process
//! group. ssh always runs with BatchMode (never `-t`): a step that needs
//! sudo uses `sudo -n`, and when sudo wants a password it fails, saying so,
//! with the commands to run by hand in Terminal. Creating the Google Cloud
//! machine ([`create_cloud`]) runs only when it is handed the exact monthly
//! estimate the person was shown ([`estimate`]), which Settings shows in its
//! confirm dialog. The person's clicks, and since COO parity (L13,
//! 2026-11-10) the `build.runner_setup_*` and `build.runner_cloud_*` app
//! actions: plan and estimate are reads, check, apply and create are
//! external actions whose card shows exactly what runs (create's card
//! names the billing account and the monthly estimate it is bound to).

use crate::remote_session::{shell_quote, valid_host};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The Rust toolchain this repository pins (`rust-toolchain.toml`).
pub(crate) const RUST_TOOLCHAIN: &str = "1.96.0";
/// The Playwright this repository's smoke suite uses (`package-lock.json`).
pub(crate) const PLAYWRIGHT_VERSION: &str = "1.61.1";
pub(crate) const DEFAULT_ALIAS: &str = "hollow-runner";
pub(crate) const DEFAULT_CLOUD_ALIAS: &str = "hollow-gcp";
pub(crate) const DEFAULT_ACCOUNT: &str = "hollowworker";
pub(crate) const DEFAULT_INSTANCE: &str = "hollow-runner";
pub(crate) const DEFAULT_SERVICE_ACCOUNT: &str = "hollow-runner";
pub(crate) const DEFAULT_ZONE: &str = "us-central1-a";
pub(crate) const DEFAULT_MACHINE: &str = "e2-standard-8";
/// IAP's TCP forwarding range: the only source the VM's SSH port admits.
pub(crate) const IAP_RANGE: &str = "35.235.240.0/20";
/// Every ssh call setup makes: no agent or X11 forwarding, never a password
/// prompt, a bounded connect, no shared master connection.
const SSH: &str = "ssh -a -x -o BatchMode=yes -o ConnectTimeout=10 -o ControlPath=none";
/// The ProxyCommand helper a Google Cloud runner's Host block names.
const PROXY_PATH: &str = "~/.config/hollow/gcp-runner-proxy";
/// The sshd setting the sleep step writes there.
const KEEPALIVE_CONF: &str = "/etc/ssh/sshd_config.d/100-hollow-keepalive.conf";
/// The longest text kept from one command's output.
const OUTPUT_MAX: usize = 2000;

const QUICK: Duration = Duration::from_secs(30);
const MEDIUM: Duration = Duration::from_secs(10 * 60);
const LONG: Duration = Duration::from_secs(30 * 60);

/// What the person typed (empty fields take defaults in [`filled`]).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct SetupParams {
    /// `mac` or `cloud`.
    pub path: String,
    /// An SSH destination for an admin account on the Mac (`user@host` or an
    /// alias), used for setup only.
    pub admin: String,
    /// The `Host` alias this Mac reaches the runner by (and names in
    /// `slot-remote`).
    pub alias: String,
    /// The runner account on the Mac.
    pub account: String,
    /// The private key this Mac uses for the runner (`~/.ssh/...`).
    pub key_file: String,
    /// The Mac's address for the Host block (from the admin login when
    /// empty).
    pub host_name: String,
    pub project: String,
    pub zone: String,
    pub machine_type: String,
    pub disk_gb: u32,
    pub idle_minutes: u32,
    pub hours_per_month: u32,
    pub billing_account: String,
    pub budget_usd: u32,
    pub instance: String,
    pub service_account: String,
}

/// Who runs a command.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum On {
    /// This Mac, as the person.
    Local,
    /// The admin login, over ssh.
    Admin,
    /// The runner account, over ssh.
    Runner,
    /// gcloud on this Mac.
    Gcloud,
    /// Hollow itself (no shell).
    Hollow,
}

/// A command as shown, and how it runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cmd {
    pub on: On,
    /// Who runs it, in words.
    pub who: String,
    /// Exactly what runs (a shell command line, a quoted gcloud argv, or
    /// what Hollow does).
    pub shown: String,
    /// It uses `sudo -n` there.
    pub sudo: bool,
    #[serde(skip)]
    run: Run,
    #[serde(skip)]
    limit: Duration,
    /// Appends a Host block to `~/.ssh/config`: refused when one for the
    /// alias says otherwise, skipped when one already says the same.
    #[serde(skip)]
    guard: Option<HostGuard>,
}

#[derive(Debug, Clone, PartialEq)]
enum Run {
    /// `/bin/sh -c <shown>` on this Mac.
    Shell,
    /// gcloud with these arguments (the binary is a parameter).
    Gcloud(Vec<String>),
    /// One of Hollow's own actions.
    Hollow(Builtin),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Builtin {
    /// Which runner `slot-remote` names.
    ReadRunner,
    /// The runner status check of the alias (`remote_status::check`).
    Status,
    /// Name the alias in `slot-remote` (`runner_owner::set_runner`).
    SetRunner,
    /// Record this Hollow as its owner (`runner_owner::record_owner`).
    RecordOwner,
    /// Put Hollow's own `hollow-slot` in its app data, as before any build
    /// (`build_slots::ensure_tool`).
    EnsureTool,
    /// The test call through `hollow-slot --on <alias>` (a shell command;
    /// this marks it so its output is read for a fallback to this Mac).
    TestCall,
}

/// The Host block a guarded append writes, as `ssh_config` keywords
/// (lowercase) and values.
#[derive(Debug, Clone, PartialEq)]
struct HostGuard {
    alias: String,
    want: Vec<(&'static str, String)>,
}

/// One step of a path.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub id: &'static str,
    pub title: &'static str,
    pub why: String,
    /// What its Apply changes, in a few words each ("creates a user").
    pub changes: Vec<&'static str>,
    /// The confirm dialog's text when Apply asks first.
    pub confirm: Option<String>,
    pub check: Vec<Cmd>,
    pub apply: Vec<Cmd>,
    /// Shown, never run: for the person to run in Terminal.
    pub by_hand: Vec<Cmd>,
    pub by_hand_note: Option<String>,
}

/// A machine type the Google Cloud path offers, with its approximate price.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MachineType {
    pub id: &'static str,
    pub label: &'static str,
    /// On-demand USD per hour in us-central1, in millionths.
    pub hourly_micro: u64,
    pub arm: bool,
}

/// Approximate Google Cloud list prices (on-demand, us-central1), as of
/// [`PRICES_TAKEN`]. Other regions differ; the UI says so.
pub(crate) const MACHINE_TYPES: [MachineType; 5] = [
    MachineType {
        id: "e2-standard-4",
        label: "4 vCPU, 16 GB",
        hourly_micro: 134_012,
        arm: false,
    },
    MachineType {
        id: "e2-standard-8",
        label: "8 vCPU, 32 GB",
        hourly_micro: 268_024,
        arm: false,
    },
    MachineType {
        id: "n2-standard-8",
        label: "8 vCPU, 32 GB, faster cores",
        hourly_micro: 388_472,
        arm: false,
    },
    MachineType {
        id: "t2a-standard-8",
        label: "8 vCPU, 32 GB, Arm",
        hourly_micro: 308_000,
        arm: true,
    },
    MachineType {
        id: "e2-standard-16",
        label: "16 vCPU, 64 GB",
        hourly_micro: 536_048,
        arm: false,
    },
];
/// pd-balanced, USD per GB-month in millionths (us-central1).
pub(crate) const DISK_MICRO_PER_GB_MONTH: u64 = 100_000;
/// Cloud NAT: the gateway, USD per VM-hour while the VM runs, and the
/// external address it holds, USD per month (0.005 an hour, all month), in
/// millionths. The data it carries (0.045 per GB) is extra.
pub(crate) const NAT_MICRO_PER_VM_HOUR: u64 = 1_400;
pub(crate) const NAT_IP_MICRO_PER_MONTH: u64 = 3_650_000;
pub(crate) const PRICES_TAKEN: &str = "2026-11-03";
pub(crate) const PRICES_REGION: &str = "us-central1";

/// A machine type as Settings lists it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MachineTypeView {
    pub id: &'static str,
    pub label: &'static str,
    pub hourly_usd: f64,
    pub arm: bool,
}

/// The monthly estimate shown before Create.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub machine_type: String,
    pub machine_label: String,
    pub hourly_usd: f64,
    pub hours: u32,
    pub compute_usd: f64,
    pub disk_gb: u32,
    pub disk_usd: f64,
    /// Cloud NAT: its gateway at the expected hours and its address.
    pub nat_usd: f64,
    pub total_usd: f64,
    /// The total in cents: what Create must be handed back.
    pub total_cents: u64,
    pub region: String,
    pub prices_region: &'static str,
    pub prices_taken: &'static str,
}

/// Everything Settings shows before anything runs.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupPlan {
    /// The fields with their defaults filled in.
    pub params: SetupParams,
    /// What is missing or not valid; Check and Apply refuse until it is
    /// empty, and the commands show placeholders for those fields.
    pub problems: Vec<String>,
    /// Worth a look, stops nothing (see [`notes`]).
    pub notes: Vec<String>,
    pub steps: Vec<Step>,
    pub estimate: Option<Estimate>,
    pub machine_types: Vec<MachineTypeView>,
}

/// One command's run, as a step's state shows it.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RanCmd {
    pub shown: String,
    pub code: Option<i32>,
    pub output: String,
}

/// What a check or apply found for one step.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StepState {
    pub id: String,
    /// `done`, `missing` (with what), `unknown` (couldn't tell, with why) or
    /// `failed` (an apply stopped).
    pub state: &'static str,
    pub summary: String,
    pub details: Vec<String>,
    /// sudo wanted a password there: the step's by-hand commands do it.
    pub needs_by_hand: bool,
    pub ran: Vec<RanCmd>,
}

/// Where commands run and what they see. [`Ctx::real`] in the app; tests
/// point every part at a rig.
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    /// This Mac's home (`HOME` for every local command).
    pub home: PathBuf,
    /// `PATH` for local commands.
    pub path: OsString,
    /// The ssh program Hollow's own calls use (`ssh -G`, the owner record,
    /// the status check).
    pub ssh: String,
    /// The gcloud program.
    pub gcloud: PathBuf,
    /// The installed `hollow-slot`.
    pub hollow_slot: PathBuf,
    /// Where Create writes the VM's startup script.
    pub tmp: PathBuf,
    /// `YYYYMMDD`, for backup names.
    pub today: String,
    /// Put before `/Users` and `/etc/ssh` in the admin scripts ("" in the
    /// app; a rig folder in tests).
    pub root: String,
    /// `slot-remote` to write instead of the real one (tests).
    pub slot_remote: Option<PathBuf>,
}

impl Ctx {
    /// This Mac as it is.
    pub(crate) fn real() -> Result<Ctx, String> {
        let home = dirs::home_dir().ok_or("Couldn't find the home folder.")?;
        let gcloud =
            crate::runner_access::gcloud_program().unwrap_or_else(|| PathBuf::from("gcloud"));
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(dir) = gcloud.parent().filter(|d| !d.as_os_str().is_empty()) {
            dirs.push(dir.to_path_buf());
        }
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        for d in ["/usr/bin", "/bin", "/usr/sbin", "/sbin"] {
            dirs.push(PathBuf::from(d));
        }
        Ok(Ctx {
            home,
            path: std::env::join_paths(dirs).map_err(|e| e.to_string())?,
            ssh: "ssh".into(),
            gcloud,
            hollow_slot: crate::agent_settings::app_data_dir()?
                .join("bin")
                .join("hollow-slot"),
            tmp: std::env::temp_dir(),
            today: chrono::Local::now().format("%Y%m%d").to_string(),
            root: String::new(),
            slot_remote: None,
        })
    }

    /// `~/...` on this Mac as a path.
    fn expand(&self, path: &str) -> PathBuf {
        match path.strip_prefix("~/") {
            Some(rest) => self.home.join(rest),
            None => PathBuf::from(path),
        }
    }

    fn startup_script_path(&self) -> PathBuf {
        self.tmp.join("hollow-runner-startup.sh")
    }
}

/// One check, apply or create at a time.
static BUSY: Mutex<()> = Mutex::new(());

fn one_at_a_time() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    // Tests take turns instead of refusing each other.
    if cfg!(test) {
        return Ok(BUSY.lock().unwrap_or_else(|e| e.into_inner()));
    }
    match BUSY.try_lock() {
        Ok(guard) => Ok(guard),
        Err(std::sync::TryLockError::Poisoned(p)) => Ok(p.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => {
            Err("Set up is already running a check or a step; wait for it to finish.".into())
        }
    }
}

// ---- Fields ------------------------------------------------------------

fn is_cloud(p: &SetupParams) -> bool {
    p.path == "cloud"
}

/// `params` with blanks filled: the defaults, and for a Mac the runner
/// already set up (the alias from `slot-remote`, its account and address
/// from `ssh -G`) and the address from the admin login.
pub(crate) fn filled(ctx: &Ctx, params: &SetupParams) -> SetupParams {
    let mut p = params.clone();
    for field in [
        &mut p.path,
        &mut p.admin,
        &mut p.alias,
        &mut p.account,
        &mut p.key_file,
        &mut p.host_name,
        &mut p.project,
        &mut p.zone,
        &mut p.machine_type,
        &mut p.billing_account,
        &mut p.instance,
        &mut p.service_account,
    ] {
        *field = field.trim().to_string();
    }
    if p.path != "cloud" {
        p.path = "mac".into();
    }
    let cloud = is_cloud(&p);
    if p.alias.is_empty() {
        p.alias = match named_runner(ctx) {
            Some(current) if !cloud => current,
            _ if cloud => DEFAULT_CLOUD_ALIAS.into(),
            _ => DEFAULT_ALIAS.into(),
        };
    }
    if !cloud {
        if (p.account.is_empty() || p.host_name.is_empty() || p.key_file.is_empty())
            && valid_alias(&p.alias)
        {
            if let Some(g) = ssh_g(ctx, &p.alias).filter(|g| has_block(g, &p.alias)) {
                if p.account.is_empty() {
                    p.account = first(&g, "user").unwrap_or_default();
                }
                if p.host_name.is_empty() {
                    p.host_name = first(&g, "hostname").unwrap_or_default();
                }
                if p.key_file.is_empty() {
                    p.key_file = block_key(ctx, &p.alias, &g).unwrap_or_default();
                }
            }
        }
        if p.account.is_empty() {
            p.account = DEFAULT_ACCOUNT.into();
        }
        if p.host_name.is_empty() && valid_host(&p.admin) {
            p.host_name = match p.admin.split_once('@') {
                Some((_, host)) => host.to_string(),
                None => ssh_g(ctx, &p.admin)
                    .and_then(|g| first(&g, "hostname"))
                    .unwrap_or_default(),
            };
        }
    } else {
        if p.zone.is_empty() {
            p.zone = DEFAULT_ZONE.into();
        }
        if p.machine_type.is_empty() {
            p.machine_type = DEFAULT_MACHINE.into();
        }
        if p.disk_gb == 0 {
            p.disk_gb = 200;
        }
        if p.idle_minutes == 0 {
            p.idle_minutes = 30;
        }
        if p.hours_per_month == 0 {
            p.hours_per_month = 160;
        }
        if p.budget_usd == 0 {
            p.budget_usd = 100;
        }
        if p.instance.is_empty() {
            p.instance = DEFAULT_INSTANCE.into();
        }
        if p.service_account.is_empty() {
            p.service_account = DEFAULT_SERVICE_ACCOUNT.into();
        }
    }
    if p.key_file.is_empty() && valid_alias(&p.alias) {
        p.key_file = format!("~/.ssh/id_ed25519_{}", p.alias);
    }
    p
}

/// The key a runner's Host block offers (an `IdentityFile` under `~/.ssh/`
/// with a `.pub` beside it here), so a runner set up by hand with another
/// key name isn't reported as keyless. The alias's own block in
/// `~/.ssh/config` first: `ssh -G` (`g`) puts a `Host *` block's keys
/// first. Else `ssh -G`'s order, which is ssh's own choice when the block
/// names no key or sits in an `Include`d file.
fn block_key(ctx: &Ctx, alias: &str, g: &[(String, String)]) -> Option<String> {
    let home = ctx.home.to_string_lossy();
    let usable = |v: &str| {
        let k = match v.strip_prefix(home.as_ref()) {
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            _ => v.to_string(),
        };
        (valid_key_file(&k) && ctx.expand(&format!("{k}.pub")).is_file()).then_some(k)
    };
    let keys = |list: &[(String, String)]| {
        list.iter()
            .filter(|(k, _)| k == "identityfile")
            .find_map(|(_, v)| usable(v))
    };
    own_block(ctx, alias)
        .and_then(|own| keys(&own))
        .or_else(|| keys(g))
}

fn valid_alias(alias: &str) -> bool {
    valid_host(alias) && !alias.contains('@') && !matches!(alias, "off" | "local" | "none")
}

fn valid_account(a: &str) -> bool {
    let b = a.as_bytes();
    !b.is_empty()
        && b.len() <= 31
        && (b[0].is_ascii_lowercase() || b[0] == b'_')
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
        && !matches!(a, "root" | "admin" | "daemon" | "nobody")
}

fn valid_key_file(k: &str) -> bool {
    k.strip_prefix("~/.ssh/").is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 80
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    })
}

/// A lowercase name: a letter first, then letters, digits and `-`, not
/// ending in `-`, `min..=max` long.
fn valid_gcp_name(name: &str, min: usize, max: usize) -> bool {
    let b = name.as_bytes();
    (min..=max).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[b.len() - 1] != b'-'
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

fn valid_zone(zone: &str) -> bool {
    let parts: Vec<&str> = zone.split('-').collect();
    parts.len() == 3
        && parts[0].len() >= 2
        && parts[0].bytes().all(|c| c.is_ascii_lowercase())
        && parts[1].len() >= 3
        && parts[1]
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && parts[1].bytes().last().is_some_and(|c| c.is_ascii_digit())
        && parts[2].len() == 1
        && parts[2].bytes().all(|c| c.is_ascii_lowercase())
}

fn valid_billing(b: &str) -> bool {
    let parts: Vec<&str> = b.split('-').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            p.len() == 6
                && p.bytes()
                    .all(|c| c.is_ascii_digit() || (b'A'..=b'F').contains(&c))
        })
}

fn machine(id: &str) -> Option<&'static MachineType> {
    MACHINE_TYPES.iter().find(|m| m.id == id)
}

/// What is missing or not valid in filled `p`, one sentence each.
pub(crate) fn problems(p: &SetupParams) -> Vec<String> {
    let mut out = Vec::new();
    let mut need = |ok: bool, what: String| {
        if !ok {
            out.push(what);
        }
    };
    need(
        valid_alias(&p.alias),
        "The runner alias must be one SSH host name: letters, digits, dots, dashes and underscores.".into(),
    );
    need(
        valid_key_file(&p.key_file),
        "The key file must be a file name in ~/.ssh (like ~/.ssh/id_ed25519_hollow-runner).".into(),
    );
    if is_cloud(p) {
        need(
            valid_gcp_name(&p.project, 6, 30),
            "The project id is 6 to 30 lowercase letters, digits and dashes, starting with a letter.".into(),
        );
        need(
            valid_zone(&p.zone),
            "The zone looks like us-central1-a.".into(),
        );
        need(
            machine(&p.machine_type).is_some(),
            "Pick a machine type from the list.".into(),
        );
        need(
            (20..=2000).contains(&p.disk_gb),
            "The boot disk is 20 to 2000 GB.".into(),
        );
        need(
            (5..=720).contains(&p.idle_minutes),
            "Stop after 5 to 720 idle minutes.".into(),
        );
        need(
            (1..=744).contains(&p.hours_per_month),
            "Expected hours a month are 1 to 744.".into(),
        );
        need(
            valid_billing(&p.billing_account),
            "The billing account id looks like 0X0X0X-0X0X0X-0X0X0X (gcloud billing accounts list).".into(),
        );
        need(
            (1..=100_000).contains(&p.budget_usd),
            "The monthly budget is 1 to 100000 USD.".into(),
        );
        need(
            valid_gcp_name(&p.instance, 1, 54),
            "The instance name is up to 54 lowercase letters, digits and dashes, starting with a letter.".into(),
        );
        need(
            valid_gcp_name(&p.service_account, 6, 30),
            "The service account name is 6 to 30 lowercase letters, digits and dashes, starting with a letter.".into(),
        );
    } else {
        need(
            valid_host(&p.admin),
            "The admin login is an SSH destination for an admin account there, like admin@your-mac.local.".into(),
        );
        need(
            valid_account(&p.account),
            "The runner account is a short lowercase account name (not root or admin).".into(),
        );
        need(
            valid_host(&p.host_name) && !p.host_name.contains('@'),
            "Its address is the Mac's host name or IP address (from the admin login when empty)."
                .into(),
        );
        need(
            p.alias != p.admin,
            "The runner alias must differ from the admin login.".into(),
        );
    }
    out
}

/// Every field valid, or what isn't.
fn validate(p: &SetupParams) -> Result<(), String> {
    let found = problems(p);
    if found.is_empty() {
        Ok(())
    } else {
        Err(found.join(" "))
    }
}

/// The values the commands are built from: the fields, or a placeholder
/// for one that isn't valid (shown, never run: Check and Apply refuse).
struct Names {
    admin: String,
    alias: String,
    account: String,
    key: String,
    host_name: String,
    project: String,
    zone: String,
    machine: &'static MachineType,
    disk_gb: u32,
    idle: u32,
    billing: String,
    budget: u32,
    instance: String,
    sa: String,
}

impl Names {
    fn of(p: &SetupParams) -> Names {
        let pick = |ok: bool, value: &str, placeholder: &str| {
            if ok {
                value.to_string()
            } else {
                placeholder.to_string()
            }
        };
        let cloud = is_cloud(p);
        let alias = pick(
            valid_alias(&p.alias),
            &p.alias,
            if cloud {
                DEFAULT_CLOUD_ALIAS
            } else {
                DEFAULT_ALIAS
            },
        );
        Names {
            admin: pick(valid_host(&p.admin), &p.admin, "admin@your-mac.local"),
            key: pick(
                valid_key_file(&p.key_file),
                &p.key_file,
                &format!("~/.ssh/id_ed25519_{alias}"),
            ),
            alias,
            account: pick(valid_account(&p.account), &p.account, DEFAULT_ACCOUNT),
            host_name: pick(
                valid_host(&p.host_name) && !p.host_name.contains('@'),
                &p.host_name,
                "your-mac.local",
            ),
            project: pick(
                valid_gcp_name(&p.project, 6, 30),
                &p.project,
                "your-project",
            ),
            zone: pick(valid_zone(&p.zone), &p.zone, DEFAULT_ZONE),
            machine: machine(&p.machine_type)
                .or_else(|| machine(DEFAULT_MACHINE))
                .expect("the default machine type is listed"),
            disk_gb: p.disk_gb.clamp(20, 2000),
            idle: p.idle_minutes.clamp(5, 720),
            billing: pick(
                valid_billing(&p.billing_account),
                &p.billing_account,
                "000000-000000-000000",
            ),
            budget: p.budget_usd.clamp(1, 100_000),
            instance: pick(
                valid_gcp_name(&p.instance, 1, 54),
                &p.instance,
                DEFAULT_INSTANCE,
            ),
            sa: pick(
                valid_gcp_name(&p.service_account, 6, 30),
                &p.service_account,
                DEFAULT_SERVICE_ACCOUNT,
            ),
        }
    }

    fn sa_email(&self) -> String {
        format!("{}@{}.iam.gserviceaccount.com", self.sa, self.project)
    }

    fn firewall(&self) -> String {
        format!("{}-iap-ssh", self.instance)
    }

    /// The zone's region (`us-central1-a` → `us-central1`).
    fn region(&self) -> String {
        self.zone
            .rsplit_once('-')
            .map_or_else(|| self.zone.clone(), |(r, _)| r.to_string())
    }

    /// The Cloud Router and NAT that give the VM (no public address) its
    /// way out to the internet: apt, Node, rustup, npm and crates.
    fn router(&self) -> String {
        format!("{}-router", self.instance)
    }

    fn nat(&self) -> String {
        format!("{}-nat", self.instance)
    }

    fn budget_name(&self) -> String {
        format!("Hollow runner {}", self.instance)
    }
}

// ---- Commands ------------------------------------------------------------

fn cmd(on: On, who: String, shown: String, run: Run, limit: Duration) -> Cmd {
    Cmd {
        on,
        who,
        shown,
        sudo: false,
        run,
        limit,
        guard: None,
    }
}

fn local(shown: String, limit: Duration) -> Cmd {
    cmd(On::Local, "this Mac".into(), shown, Run::Shell, limit)
}

/// `script` run there by ssh: in quotes when it is one simple line, else
/// fed to `/bin/sh -s` (a quoted here-document: nothing in it is expanded
/// here). `args` go to the script as `$1`... (evaluated here).
fn over_ssh(dest: &str, args: Option<&str>, script: &str) -> String {
    let simple = !script.contains('\n') && !script.contains('\'') && !script.contains('*');
    match args {
        None if simple => format!("{SSH} {dest} '{script}'"),
        None => format!("{SSH} {dest} /bin/sh -s <<'HOLLOW'\n{script}\nHOLLOW"),
        Some(args) => format!("{SSH} {dest} /bin/sh -s -- {args} <<'HOLLOW'\n{script}\nHOLLOW"),
    }
}

fn admin(n: &Names, script: &str, limit: Duration) -> Cmd {
    cmd(
        On::Admin,
        format!("{} (admin login)", n.admin),
        over_ssh(&n.admin, None, script),
        Run::Shell,
        limit,
    )
}

fn admin_sudo(n: &Names, args: Option<&str>, script: &str, limit: Duration) -> Cmd {
    let mut c = cmd(
        On::Admin,
        format!("{} (admin login, sudo -n)", n.admin),
        over_ssh(&n.admin, args, script),
        Run::Shell,
        limit,
    );
    c.sudo = true;
    c
}

fn runner(n: &Names, script: &str, limit: Duration) -> Cmd {
    cmd(
        On::Runner,
        format!("{} (runner account)", n.alias),
        over_ssh(&n.alias, None, script),
        Run::Shell,
        limit,
    )
}

fn hollow(shown: String, b: Builtin, limit: Duration) -> Cmd {
    cmd(On::Hollow, "Hollow".into(), shown, Run::Hollow(b), limit)
}

/// One shell word as typed: bare when it can be, else single-quoted.
fn word(w: &str) -> String {
    if !w.is_empty()
        && w.bytes().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    b'_' | b'@' | b'%' | b'+' | b'=' | b':' | b',' | b'.' | b'/' | b'-'
                )
        })
    {
        w.to_string()
    } else {
        shell_quote(w)
    }
}

fn gcloud(args: Vec<String>, limit: Duration) -> Cmd {
    let shown = std::iter::once("gcloud".to_string())
        .chain(args.iter().map(|a| word(a)))
        .collect::<Vec<_>>()
        .join(" ");
    cmd(
        On::Gcloud,
        "gcloud on this Mac".into(),
        shown,
        Run::Gcloud(args),
        limit,
    )
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// Upload `script` to the admin's home and run it with sudo in Terminal
/// (sudo asks for the password there), then remove it.
fn by_hand_sudo(n: &Names, name: &str, args: Option<&str>, script: &str) -> Vec<Cmd> {
    let file = format!("hollow-setup-{name}.sh");
    let upload = format!(
        "ssh {} 'cat > ~/{file}' <<'HOLLOW'\n{script}\nHOLLOW",
        n.admin
    );
    let run = match args {
        None => format!("ssh -t {} 'sudo /bin/sh ~/{file}; rm -f ~/{file}'", n.admin),
        Some(args) => format!(
            "ssh -t {} \"sudo /bin/sh ~/{file} {args}; rm -f ~/{file}\"",
            n.admin
        ),
    };
    vec![local(upload, QUICK), local(run, QUICK)]
}

const BY_HAND_SUDO: &str = "When sudo there asks for a password, Apply stops and changes nothing more. Run these two in Terminal instead: the first copies the same script to the admin's home, the second runs it with sudo (it asks for the password) and removes it. Then Check.";

fn keygen(n: &Names) -> Cmd {
    local(
        format!(
            "mkdir -p ~/.ssh && chmod 700 ~/.ssh && {{ test -f {key} || ssh-keygen -t ed25519 -N '' -C 'hollow runner {alias}' -f {key}; }}",
            key = n.key,
            alias = n.alias
        ),
        QUICK,
    )
}

fn key_file_check(n: &Names) -> Cmd {
    local(
        format!(
            "test -f {key} && ssh-keygen -l -E sha256 -f {key}.pub",
            key = n.key
        ),
        QUICK,
    )
}

/// Append `block` (lines after the Host line) to `~/.ssh/config`, after
/// backing it up beside itself once a day. `pre` runs first (`&&`); with
/// `expand`, the here-document expands `$` (a shell variable `pre` set).
fn append_host_block(
    ctx: &Ctx,
    alias: &str,
    lines: &[(&'static str, String)],
    pre: &str,
    expand: bool,
) -> String {
    let bak = format!("~/.ssh/config.bak-hollow-{}", ctx.today);
    let mut body = format!(
        "\n# Hollow's remote runner (Settings, Remote runner, Set up; {})\nHost {alias}\n",
        ctx.today
    );
    for (k, v) in lines {
        body.push_str(&format!("  {} {v}\n", ssh_keyword(k)));
    }
    format!(
        "{pre}mkdir -p ~/.ssh && chmod 700 ~/.ssh && if [ -e ~/.ssh/config ]; then [ -e {bak} ] || cp -p ~/.ssh/config {bak}; fi && cat >> ~/.ssh/config <<{q}HOLLOW{q} && chmod 600 ~/.ssh/config\n{body}HOLLOW",
        q = if expand { "" } else { "'" }
    )
}

/// The canonical spelling of an `ssh_config` keyword.
fn ssh_keyword(k: &str) -> &str {
    match k {
        "hostname" => "HostName",
        "user" => "User",
        "identityfile" => "IdentityFile",
        "identitiesonly" => "IdentitiesOnly",
        "forwardagent" => "ForwardAgent",
        "stricthostkeychecking" => "StrictHostKeyChecking",
        "proxycommand" => "ProxyCommand",
        other => other,
    }
}

/// The steps for `params` (filled), placeholders where a field isn't valid.
pub(crate) fn plan_steps(ctx: &Ctx, p: &SetupParams) -> Vec<Step> {
    let n = Names::of(p);
    if is_cloud(p) {
        vec![
            step_gcloud(&n),
            step_cloud(ctx, &n),
            step_cloud_local(ctx, &n),
            step_use(ctx, &n, true),
        ]
    } else {
        vec![
            step_admin(&n),
            step_account(ctx, &n),
            step_key(ctx, &n),
            step_tools(&n),
            step_preflight(ctx, &n),
            step_sleep(ctx, &n),
            step_use(ctx, &n, false),
        ]
    }
}

fn step_admin(n: &Names) -> Step {
    Step {
        id: "admin",
        title: "Admin login",
        why: format!("Setup logs in to {} to make the runner account and change settings there. It is used for setup only; Hollow never runs work as an admin.", n.admin),
        changes: vec![],
        confirm: None,
        check: vec![admin(
            n,
            "id -un; sw_vers -productVersion; uname -m; sudo -n true 2>/dev/null && echo sudo=yes || echo sudo=no",
            QUICK,
        )],
        apply: vec![],
        by_hand: vec![local(format!("ssh-copy-id {}", n.admin), QUICK)],
        by_hand_note: Some("Run this once in Terminal: it asks for that account's password and lets this Mac's key in. Setup itself never asks for a password.".into()),
    }
}

fn account_check_script(ctx: &Ctx, n: &Names) -> String {
    format!(
        r##"a={acct}
if ! dscl . -read "/Users/$a" UniqueID >/dev/null 2>&1; then echo account=none; exit 0; fi
echo account=yes
dscl . -read "/Users/$a" IsHidden 2>/dev/null | grep -q 'IsHidden: *1' && echo hidden=yes || echo hidden=no
dseditgroup -o checkmember -m "$a" admin >/dev/null 2>&1 && echo admin=yes || echo admin=no
if dscl . -read /Groups/com.apple.access_ssh >/dev/null 2>&1; then
  dseditgroup -o checkmember -m "$a" com.apple.access_ssh >/dev/null 2>&1 && echo ssh_group=yes || echo ssh_group=no
else
  echo ssh_group=open
fi
[ -d "{root}/Users/$a" ] && echo home=yes || echo home=no"##,
        acct = n.account,
        root = ctx.root
    )
}

fn account_apply_script(ctx: &Ctx, n: &Names) -> String {
    format!(
        r##"a={acct}
if dscl . -read "/Users/$a" UniqueID >/dev/null 2>&1; then
  if dseditgroup -o checkmember -m "$a" admin >/dev/null 2>&1; then
    echo "$a is an admin account there; the runner account must be a standard user. Pick another name." >&2
    exit 1
  fi
else
  # Random passwords made here, never kept or shown: the account logs in with its SSH key only.
  # sysadminctl takes its password in argv (so in ps and sudo's log); the one below replaces it.
  pw=$(LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom | head -c 32)
  sudo -n sysadminctl -addUser "$a" -fullName 'Hollow runner' -password "$pw" -home "/Users/$a" || exit 1
fi
# An account Hollow made (its full name says so), on every Apply: a fresh random password through
# dscl's standard input (printf is built in), so the one it keeps was never on a command line, even
# when an earlier Apply stopped right after sysadminctl. An account someone else made keeps its own.
if dscl . -read "/Users/$a" RealName 2>/dev/null | grep -q 'Hollow runner'; then
  pw=$(LC_ALL=C tr -dc 'A-Za-z0-9' < /dev/urandom | head -c 32)
  out=$(printf 'passwd /Users/%s %s\n' "$a" "$pw" | sudo -n dscl . 2>&1); s=$?
  pw=
  case "$s:$out" in
    *"password is required"*) echo "sudo: a password is required" >&2; exit 1 ;;
    0:*rror*|0:*enied*|[1-9]*) echo "dscl couldn't set its password (its output isn't shown: it could hold the password)." >&2; exit 1 ;;
  esac
fi
sudo -n createhomedir -c -u "$a" >/dev/null || exit 1
sudo -n dscl . -create "/Users/$a" IsHidden 1 || exit 1
if dscl . -read /Groups/com.apple.access_ssh >/dev/null 2>&1; then
  sudo -n dseditgroup -o edit -a "$a" -t user com.apple.access_ssh || exit 1
fi
[ -d "{root}/Users/$a" ] || {{ echo "$a has no home folder at /Users/$a" >&2; exit 1; }}
echo "$a is a hidden standard user""##,
        acct = n.account,
        root = ctx.root
    )
}

fn step_account(ctx: &Ctx, n: &Names) -> Step {
    let script = account_apply_script(ctx, n);
    Step {
        id: "account",
        title: "A hidden standard user",
        why: format!("Work runs as {}, a standard user (no admin rights, no sudo) hidden from the login window, so branch code and agents never run as you or as an admin there.", n.account),
        changes: vec!["creates a user"],
        confirm: Some(format!("This creates the user {} on that Mac (a hidden standard user named Hollow runner, with a fresh random password nobody keeps: it logs in with its SSH key only), or hides and admits the existing one. An account Hollow didn't make keeps its password.", n.account)),
        check: vec![admin(n, &account_check_script(ctx, n), QUICK)],
        apply: vec![admin_sudo(n, None, &script, QUICK)],
        by_hand: by_hand_sudo(n, "account", None, &script),
        by_hand_note: Some(BY_HAND_SUDO.into()),
    }
}

fn key_install_script(ctx: &Ctx, n: &Names) -> String {
    format!(
        r##"a=$1; shift; key="$*"
case "$key" in "ssh-ed25519 "*) ;; *) echo "that isn't an ed25519 public key" >&2; exit 1 ;; esac
# As the runner account, never as root: branch code runs in that home, so a link planted there must not reach a file only root may change.
sudo -n -u "$a" /bin/sh -c 'umask 077; d=$1; f=$d/authorized_keys; mkdir -p "$d" && chmod 700 "$d" && touch "$f" && chmod 600 "$f" || exit 1; grep -qF "$2" "$f" || printf "%s hollow runner %s\n" "$2" "$3" >> "$f"' sh "{root}/Users/$a/.ssh" "$key" '{alias}'"##,
        root = ctx.root,
        alias = n.alias
    )
}

/// `key`'s public key as two shell words, evaluated on this Mac: only a
/// well-formed ed25519 line leaves it (type and key, no comment), else
/// nothing, and the script there refuses.
fn pub_key_words(key: &str) -> String {
    format!("$(awk '/^ssh-ed25519 [A-Za-z0-9+\\/=]+( |$)/ {{ print $1, $2; exit }}' {key}.pub)")
}

fn mac_host_lines(n: &Names) -> Vec<(&'static str, String)> {
    vec![
        ("hostname", n.host_name.clone()),
        ("user", n.account.clone()),
        ("identityfile", n.key.clone()),
        ("identitiesonly", "yes".into()),
        ("forwardagent", "no".into()),
    ]
}

fn step_key(ctx: &Ctx, n: &Names) -> Step {
    let lines = mac_host_lines(n);
    let mut append = local(append_host_block(ctx, &n.alias, &lines, "", false), QUICK);
    append.guard = Some(HostGuard {
        alias: n.alias.clone(),
        want: lines[..3].to_vec(),
    });
    let key_args = format!("{} {}", n.account, pub_key_words(&n.key));
    let install = key_install_script(ctx, n);
    Step {
        id: "key",
        title: "A key of its own",
        why: format!("This Mac logs in to {} with a key made for it alone ({}), through a Host block named {} in ~/.ssh/config. Agent forwarding stays off, so nothing there can use your other keys.", n.account, n.key, n.alias),
        changes: vec!["writes ~/.ssh/config", "writes files in the runner account"],
        confirm: Some(format!("This makes {} on this Mac if it isn't there, adds a Host {} block to ~/.ssh/config (backed up first as ~/.ssh/config.bak-hollow-{}), and adds the key to {}'s authorized_keys there.", n.key, n.alias, ctx.today, n.account)),
        check: vec![
            key_file_check(n),
            local(format!("ssh -G {}", n.alias), QUICK),
            runner(n, "true", QUICK),
        ],
        apply: vec![keygen(n), append, admin_sudo(n, Some(&key_args), &install, QUICK)],
        by_hand: by_hand_sudo(n, "key", Some(&key_args), &install),
        by_hand_note: Some(BY_HAND_SUDO.into()),
    }
}

/// What the runner account has, by reads only: gcloud's version comes from
/// its SDK's VERSION file because running gcloud rewrites caches in its
/// config folder (its GCE probe, every 10 minutes), no rustup proxy may
/// install a toolchain, and gh doesn't look for updates.
const TOOLS_CHECK: &str = r##"echo "path=$PATH"
PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
export RUSTUP_AUTO_INSTALL=0 GH_NO_UPDATE_NOTIFIER=1
if xcode-select -p >/dev/null 2>&1; then clt=yes; else clt=no; fi
echo "clt=$clt"
for t in git node cargo rustc uv python3 gh gcloud; do
  p=$(command -v "$t" 2>/dev/null)
  case "$clt:$p" in no:/usr/bin/*) p= ;; esac
  if [ -z "$p" ]; then echo "$t="
  elif [ "$t" = gcloud ]; then
    v=$(cat "$(dirname "$(readlink -f "$p")")/../VERSION" 2>/dev/null)
    echo "gcloud=Google Cloud SDK ${v:-at $p}"
  else echo "$t=$("$p" --version 2>/dev/null </dev/null | head -n 1)"; fi
done
echo "toolchains=$(rustup toolchain list 2>/dev/null </dev/null | tr '\n' ' ')"
p=$(ls -d /opt/homebrew/opt/postgresql@*/bin/pg_ctl /usr/local/opt/postgresql@*/bin/pg_ctl 2>/dev/null | tail -n 1)
if [ -n "$p" ]; then echo "pg_ctl=$("$p" --version </dev/null 2>/dev/null)"; else echo "pg_ctl="; fi
c=$(ls -d "$HOME"/Library/Caches/ms-playwright/chromium-* 2>/dev/null | tail -n 1)
echo "chromium=${c##*/}""##;

/// The runner account's `~/.zshenv` line that puts its tools on the PATH
/// of every ssh command (its login shell is zsh; a non-interactive ssh
/// command reads only `~/.zshenv`). Added once.
const ZSHENV_APPLY: &str = r##"f="$HOME/.zshenv"
if grep -qs 'Hollow runner: tools on PATH' "$f"; then exit 0; fi
printf '%s\n' '' '# Hollow runner: tools on PATH for ssh commands (Settings, Remote runner, Set up)' 'export PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"' >> "$f""##;

const BREW_APPLY: &str = r##"b=$(command -v brew 2>/dev/null || ls /opt/homebrew/bin/brew /usr/local/bin/brew 2>/dev/null | head -n 1)
if [ -z "$b" ]; then
  echo "Homebrew isn't installed there. Install it from https://brew.sh as this admin account, then Apply again." >&2
  exit 1
fi
"$b" install node uv gh postgresql@17 </dev/null || exit 1
"$b" install --cask gcloud-cli </dev/null || exit 1
# The cask leaves its SDK readable by its owner only; the runner account runs it too.
chmod -R go+rX "$("$b" --prefix)/share/google-cloud-sdk/" 2>/dev/null || true"##;

fn rustup_apply() -> String {
    format!(
        r##"PATH="$HOME/.cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
if command -v rustup >/dev/null 2>&1; then
  rustup toolchain install {t} --profile minimal -c rustfmt -c clippy </dev/null && rustup default {t} </dev/null
else
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain {t} -c rustfmt -c clippy
fi"##,
        t = RUST_TOOLCHAIN
    )
}

fn playwright_apply() -> String {
    format!(
        r##"PATH="/opt/homebrew/bin:/usr/local/bin:$PATH"
command -v npx >/dev/null 2>&1 || {{ echo "npx isn't there yet: Apply runs Homebrew's node first; check its output." >&2; exit 1; }}
npx -y playwright@{PLAYWRIGHT_VERSION} install chromium </dev/null"##
    )
}

fn step_tools(n: &Names) -> Step {
    Step {
        id: "tools",
        title: "Toolchains",
        why: format!("Builds, tests and agent turns there need git, Node, Rust {RUST_TOOLCHAIN} (this repository's pin), uv, Python, gh, gcloud, PostgreSQL and Playwright's Chromium. Homebrew installs run as the admin; Rust and Chromium go into the runner account's own home."),
        changes: vec!["installs software"],
        confirm: Some(format!("This installs node, uv, gh, postgresql@17 and gcloud-cli with Homebrew as {}, Rust {RUST_TOOLCHAIN} with rustup and Playwright's Chromium in {}'s home. It can take a while.", n.admin, n.account)),
        check: vec![runner(n, TOOLS_CHECK, QUICK)],
        apply: vec![
            admin(n, BREW_APPLY, LONG),
            runner(n, ZSHENV_APPLY, QUICK),
            runner(n, &rustup_apply(), MEDIUM),
            runner(n, &playwright_apply(), MEDIUM),
        ],
        by_hand: vec![],
        by_hand_note: None,
    }
}

const PREFLIGHT_CHECK: &str = r##"p=$(command -v hollow-run-preflight 2>/dev/null)
[ -n "$p" ] || { [ -x "$HOME/.local/bin/hollow-run-preflight" ] && p="$HOME/.local/bin/hollow-run-preflight"; }
if [ -z "$p" ]; then echo preflight=none; exit 0; fi
echo "preflight=$p"
out=$("$p" </dev/null 2>&1); s=$?
printf '%s\n' "$out" | tail -n 1
echo "exit=$s""##;

fn preflight_apply(ctx: &Ctx) -> String {
    format!(
        r##"umask 022
d="$HOME/.local/bin"; f="$d/hollow-run-preflight"
mkdir -p "$d" || exit 1
cat > "$f.new.$$" <<'PREFLIGHT' || exit 1
{PREFLIGHT_SH}PREFLIGHT
if [ -e "$f" ] && ! cmp -s "$f" "$f.new.$$"; then cp -p "$f" "$f.bak-hollow-{today}" || exit 1; fi
chmod 755 "$f.new.$$" && mv -f "$f.new.$$" "$f""##,
        today = ctx.today
    )
}

fn step_preflight(ctx: &Ctx, n: &Names) -> Step {
    Step {
        id: "preflight",
        title: "Preflight",
        why: "Before sending work there, Hollow and hollow-slot ask its hollow-run-preflight whether it can take it. Hollow's default says no under 20 GB free disk or 30% free memory, and otherwise names the machine, its free disk and memory, Rust and Node.".into(),
        changes: vec!["writes files in the runner account"],
        confirm: None,
        check: vec![runner(n, PREFLIGHT_CHECK, QUICK)],
        apply: vec![runner(n, &preflight_apply(ctx), QUICK)],
        by_hand: vec![],
        by_hand_note: None,
    }
}

fn sleep_check(ctx: &Ctx) -> String {
    format!(
        r##"pmset -g | grep -E '^ *(sleep|disksleep|autorestart) '
echo ---
grep -h -i '^[[:space:]]*clientalive' {root}/etc/ssh/sshd_config {root}/etc/ssh/sshd_config.d/* 2>/dev/null
true"##,
        root = ctx.root
    )
}

fn sleep_apply(ctx: &Ctx) -> String {
    format!(
        r##"f={root}{KEEPALIVE_CONF}
sudo -n pmset -a sleep 0 disksleep 0 autorestart 1 || exit 1
sudo -n mkdir -p {root}/etc/ssh/sshd_config.d || exit 1
printf '%s\n' '# Hollow remote runner: drop dead SSH sessions (Settings, Remote runner, Set up)' 'ClientAliveInterval 30' 'ClientAliveCountMax 4' | sudo -n tee "$f" >/dev/null || exit 1
if ! sudo -n sshd -t; then
  sudo -n rm -f "$f"
  echo "sshd -t refused the new setting, so it was removed and sshd left as it was." >&2
  exit 1
fi"##,
        root = ctx.root
    )
}

fn step_sleep(ctx: &Ctx, n: &Names) -> Step {
    let script = sleep_apply(ctx);
    let apply = admin_sudo(n, None, &script, QUICK);
    Step {
        id: "sleep",
        title: "No sleep, and an SSH keepalive",
        why: "A runner that sleeps drops the work on it, and one that stays off after a power cut can't take any. sshd's keepalive ends sessions whose connection died, so nothing waits on them.".into(),
        changes: vec!["changes sleep settings", "changes sshd"],
        confirm: Some(format!("This changes sleep settings and sshd on that Mac: pmset -a sleep 0 disksleep 0 autorestart 1, and {KEEPALIVE_CONF} (ClientAliveInterval 30, ClientAliveCountMax 4), checked with sshd -t, and a failed check removes the file again. macOS starts sshd for each connection, so new connections use it and none is cut.")),
        check: vec![admin(n, &sleep_check(ctx), QUICK)],
        apply: vec![apply],
        by_hand: by_hand_sudo(n, "sleep", None, &script),
        by_hand_note: Some(BY_HAND_SUDO.into()),
    }
}

/// The test call: a throwaway git repository in `$TMPDIR`, one command
/// through `hollow-slot --on <alias>` (so before the alias is this Mac's
/// runner), the repository removed.
fn test_call(ctx: &Ctx, alias: &str) -> String {
    format!(
        "d=$(mktemp -d \"${{TMPDIR:-/tmp}}/hollow-runner-test.XXXXXX\") && cd \"$d\" && git init -q && git -c user.name=Hollow -c user.email=hollow@localhost -c commit.gpgSign=false commit -q --allow-empty -m 'runner test' && {} --on {} sh -c 'uname -sm; node --version; cargo --version'; s=$?; cd / && rm -rf \"$d\"; exit $s",
        shell_quote(&ctx.hollow_slot.to_string_lossy()),
        word(alias)
    )
}

fn step_use(ctx: &Ctx, n: &Names, cloud: bool) -> Step {
    let mut call = local(test_call(ctx, &n.alias), MEDIUM);
    call.run = Run::Hollow(Builtin::TestCall);
    Step {
        id: "use",
        title: "Use it",
        why: format!("Hollow checks {alias} (its preflight included), records itself as its owner, sends one test call there through hollow-slot --on {alias}, and only then names it as this Mac's runner (~/.config/hollow/slot-remote), so a runner that fails one of these never replaces the one in use.{}", if cloud { " Checking a stopped VM would start it, so Check asks only while it runs." } else { "" }, alias = n.alias),
        changes: vec!["names this Mac's runner", "writes files in the runner account"],
        confirm: None,
        check: vec![
            hollow("Hollow reads ~/.config/hollow/slot-remote: which runner this Mac uses.".into(), Builtin::ReadRunner, QUICK),
            hollow(format!("Hollow asks {} how it is (the runner status check: one ssh call, read-only; it runs its preflight).", n.alias), Builtin::Status, QUICK),
        ],
        apply: vec![
            hollow(format!("Hollow asks {} how it is (the runner status check).", n.alias), Builtin::Status, QUICK),
            hollow(format!("Hollow records itself as {}'s owner in ~/hollow-remote/owner.json there (its install id and this Mac's key fingerprint).", n.alias), Builtin::RecordOwner, QUICK),
            hollow("Hollow puts its own hollow-slot in its app data folder (bin/hollow-slot), as it does before any build.".into(), Builtin::EnsureTool, QUICK),
            call,
            hollow(format!("Hollow writes {} on its own line in ~/.config/hollow/slot-remote.", n.alias), Builtin::SetRunner, QUICK),
        ],
        by_hand: vec![local("hollow-slot --remote npm run gate".into(), QUICK)],
        by_hand_note: Some("To run a project's gate there: in a terminal in the project's folder, or from an agent turn. Its output streams back here.".into()),
    }
}

fn step_gcloud(n: &Names) -> Step {
    Step {
        id: "gcloud",
        title: "Google Cloud sign-in",
        why: format!("Setup runs gcloud on this Mac as you, in project {}.", n.project),
        changes: vec![],
        confirm: None,
        check: vec![
            gcloud(args(&["config", "list", "account", "--format=value(core.account)"]), QUICK),
            gcloud(
                vec![
                    "projects".into(),
                    "describe".into(),
                    n.project.clone(),
                    "--format=value(projectId,lifecycleState)".into(),
                ],
                QUICK,
            ),
        ],
        apply: vec![],
        by_hand: vec![local("gcloud auth login".into(), QUICK)],
        by_hand_note: Some("When gcloud isn't signed in on this Mac, run this once in Terminal (it opens a browser).".into()),
    }
}

fn services() -> [&'static str; 3] {
    [
        "compute.googleapis.com",
        "iap.googleapis.com",
        "billingbudgets.googleapis.com",
    ]
}

/// The budget alert's list: before its create in Create (a budget never
/// says "already exists", so a second Create would add another) and in
/// Check.
fn budgets_list(n: &Names) -> Cmd {
    gcloud(
        vec![
            "billing".into(),
            "budgets".into(),
            "list".into(),
            format!("--billing-account={}", n.billing),
            "--format=value(displayName)".into(),
        ],
        QUICK,
    )
}

/// The NAT's describe: before its create in Create (a second create of a
/// NAT by the same name doesn't say "already exists") and in Check.
fn nat_describe(n: &Names) -> Cmd {
    gcloud(
        vec![
            "compute".into(),
            "routers".into(),
            "nats".into(),
            "describe".into(),
            n.nat(),
            format!("--project={}", n.project),
            format!("--router={}", n.router()),
            format!("--region={}", n.region()),
            "--format=value(name)".into(),
        ],
        QUICK,
    )
}

fn is_gcloud(c: &Cmd, verb: &[&str]) -> bool {
    matches!(&c.run, Run::Gcloud(a) if a.len() >= verb.len() && a.iter().zip(verb).all(|(x, y)| x == y))
}

/// The create sequence, in order: the VM's startup script written where
/// gcloud reads it, free things first, the budget before anything billed,
/// the VM last, then the script removed.
fn cloud_create_cmds(ctx: &Ctx, n: &Names) -> Vec<Cmd> {
    let p = format!("--project={}", n.project);
    let mut enable: Vec<String> = args(&["services", "enable"]);
    enable.extend(services().iter().map(|s| s.to_string()));
    enable.push(p.clone());
    let script = word(&ctx.startup_script_path().to_string_lossy());
    vec![
        local(
            format!(
                "umask 077 && cat > {script} <<'STARTUP'\n{}STARTUP",
                startup_script()
            ),
            QUICK,
        ),
        gcloud(enable, MEDIUM),
        budgets_list(n),
        gcloud(
            vec![
                "billing".into(),
                "budgets".into(),
                "create".into(),
                format!("--billing-account={}", n.billing),
                format!("--display-name={}", n.budget_name()),
                format!("--budget-amount={}USD", n.budget),
                "--threshold-rule=percent=0.5".into(),
                "--threshold-rule=percent=0.9".into(),
                format!("--filter-projects=projects/{}", n.project),
            ],
            QUICK,
        ),
        gcloud(
            vec![
                "iam".into(),
                "service-accounts".into(),
                "create".into(),
                n.sa.clone(),
                p.clone(),
                "--display-name=Hollow runner VM (no roles)".into(),
            ],
            QUICK,
        ),
        gcloud(
            vec![
                "compute".into(),
                "firewall-rules".into(),
                "create".into(),
                n.firewall(),
                p.clone(),
                "--network=default".into(),
                "--direction=INGRESS".into(),
                "--action=ALLOW".into(),
                "--rules=tcp:22".into(),
                format!("--source-ranges={IAP_RANGE}"),
                format!("--target-tags={}", n.instance),
            ],
            QUICK,
        ),
        gcloud(
            vec![
                "compute".into(),
                "routers".into(),
                "create".into(),
                n.router(),
                p.clone(),
                "--network=default".into(),
                format!("--region={}", n.region()),
            ],
            QUICK,
        ),
        nat_describe(n),
        gcloud(
            vec![
                "compute".into(),
                "routers".into(),
                "nats".into(),
                "create".into(),
                n.nat(),
                p.clone(),
                format!("--router={}", n.router()),
                format!("--region={}", n.region()),
                "--auto-allocate-nat-external-ips".into(),
                "--nat-all-subnet-ip-ranges".into(),
            ],
            MEDIUM,
        ),
        gcloud(
            vec![
                "compute".into(),
                "instances".into(),
                "create".into(),
                n.instance.clone(),
                p,
                format!("--zone={}", n.zone),
                format!("--machine-type={}", n.machine.id),
                format!(
                    "--image-family={}",
                    if n.machine.arm {
                        "debian-12-arm64"
                    } else {
                        "debian-12"
                    }
                ),
                "--image-project=debian-cloud".into(),
                format!("--boot-disk-size={}GB", n.disk_gb),
                "--boot-disk-type=pd-balanced".into(),
                "--no-address".into(),
                "--shielded-secure-boot".into(),
                "--shielded-vtpm".into(),
                "--shielded-integrity-monitoring".into(),
                format!("--service-account={}", n.sa_email()),
                "--no-scopes".into(),
                format!("--tags={}", n.instance),
                format!(
                    "--metadata=enable-oslogin=TRUE,hollow-idle-minutes={}",
                    n.idle
                ),
                format!(
                    "--metadata-from-file=startup-script={}",
                    ctx.startup_script_path().display()
                ),
            ],
            MEDIUM,
        ),
        local(format!("rm -f {script}"), QUICK),
    ]
}

fn step_cloud(ctx: &Ctx, n: &Names) -> Step {
    let p = format!("--project={}", n.project);
    Step {
        id: "cloud",
        title: "The machine",
        why: format!("A Debian 12 VM, {} ({}), with no public address: SSH reaches it only through IAP, from IAP's range {IAP_RANGE}, as your own Google account (OS Login), and it reaches the internet out through a Cloud NAT ({}). It runs as a service account that holds no roles, powers itself off after {} idle minutes (set up before anything else at its first boot, so a failed install still stops it), and a budget alert mails at 50% and 90% of {} USD a month. Its startup script (the first command below; Create writes it to a private temporary file for gcloud and removes it at the end, also when a command fails) installs git, Node, uv, Rust {RUST_TOOLCHAIN}, Playwright's Chromium and Hollow's preflight.", n.machine.id, n.machine.label, n.nat(), n.idle, n.budget),
        changes: vec!["creates a VM (billed)", "creates a Cloud NAT (billed)", "creates a service account", "creates a firewall rule", "creates a budget alert", "enables APIs"],
        confirm: Some(format!("This creates the VM {} ({}) in project {}, with a service account, a firewall rule, a Cloud Router and NAT ({}) and a budget alert.", n.instance, n.machine.id, n.project, n.nat())),
        check: vec![
            gcloud(vec!["services".into(), "list".into(), "--enabled".into(), p.clone(), "--format=value(config.name)".into()], QUICK),
            gcloud(vec!["compute".into(), "instances".into(), "describe".into(), n.instance.clone(), p.clone(), format!("--zone={}", n.zone), "--format=value(status)".into()], QUICK),
            gcloud(vec!["iam".into(), "service-accounts".into(), "describe".into(), n.sa_email(), p.clone(), "--format=value(email)".into()], QUICK),
            gcloud(vec!["compute".into(), "firewall-rules".into(), "describe".into(), n.firewall(), p, "--format=value(sourceRanges.list())".into()], QUICK),
            budgets_list(n),
            nat_describe(n),
        ],
        apply: cloud_create_cmds(ctx, n),
        by_hand: vec![],
        by_hand_note: None,
    }
}

fn cloud_host_lines(n: &Names) -> Vec<(&'static str, String)> {
    vec![
        ("hostname", n.instance.clone()),
        ("user", "$u".into()),
        ("identityfile", n.key.clone()),
        ("identitiesonly", "yes".into()),
        ("forwardagent", "no".into()),
        ("stricthostkeychecking", "accept-new".into()),
        (
            "proxycommand",
            format!("{PROXY_PATH} {} {} {}", n.instance, n.project, n.zone),
        ),
    ]
}

fn step_cloud_local(ctx: &Ctx, n: &Names) -> Step {
    let lines = cloud_host_lines(n);
    let pre = format!(
        "u=$(gcloud compute os-login describe-profile --project={} '--format=value(posixAccounts[0].username)') && [ -n \"$u\" ] && ",
        n.project
    );
    let mut append = local(append_host_block(ctx, &n.alias, &lines, &pre, true), QUICK);
    append.guard = Some(HostGuard {
        alias: n.alias.clone(),
        want: vec![lines[0].clone(), lines[6].clone()],
    });
    let proxy = local(
        format!("mkdir -p ~/.config/hollow && cat > {PROXY_PATH}.new <<'PROXY' && chmod 755 {PROXY_PATH}.new && mv -f {PROXY_PATH}.new {PROXY_PATH}\n{PROXY_SH}PROXY"),
        QUICK,
    );
    let pubkey = ctx.expand(&format!("{}.pub", n.key));
    Step {
        id: "cloud-local",
        title: "This Mac's way in",
        why: format!("This Mac reaches the VM as {} through a ProxyCommand helper ({PROXY_PATH}) that starts the VM when it is stopped, waits for it, then opens an IAP tunnel. Your OS Login profile gets this Mac's key for it ({}).", n.alias, n.key),
        changes: vec!["writes ~/.ssh/config", "writes ~/.config/hollow/gcp-runner-proxy", "adds a key to your OS Login profile"],
        confirm: Some(format!("This makes {} on this Mac if it isn't there, adds it to your OS Login profile, writes {PROXY_PATH} and adds a Host {} block to ~/.ssh/config (backed up first).", n.key, n.alias)),
        check: vec![
            key_file_check(n),
            local(format!("ssh -G {}", n.alias), QUICK),
            local(format!("test -x {PROXY_PATH} && echo proxy=yes || echo proxy=no"), QUICK),
        ],
        apply: vec![
            keygen(n),
            gcloud(vec!["compute".into(), "os-login".into(), "ssh-keys".into(), "add".into(), format!("--key-file={}", pubkey.display()), format!("--project={}", n.project)], QUICK),
            proxy,
            append,
        ],
        by_hand: vec![],
        by_hand_note: None,
    }
}

/// What stops nothing but is worth a look: an admin login that goes to
/// another machine than the runner's address, as when a new Mac is set up
/// while the fields still hold the current runner's (its alias, account and
/// address come from its Host block). Fine when both names reach one Mac.
fn notes(ctx: &Ctx, p: &SetupParams) -> Vec<String> {
    if is_cloud(p) || !valid_host(&p.admin) || p.host_name.is_empty() {
        return Vec::new();
    }
    let admin_host = match p.admin.split_once('@') {
        Some((_, host)) => host.to_string(),
        None => ssh_g(ctx, &p.admin)
            .and_then(|g| first(&g, "hostname"))
            .unwrap_or_default(),
    };
    if admin_host.is_empty() || admin_host.eq_ignore_ascii_case(&p.host_name) {
        return Vec::new();
    }
    vec![format!(
        "The admin login goes to {admin_host}, but the runner's address is {}. That is fine when both reach the same Mac; for another Mac, change Its address and the Runner alias.",
        p.host_name
    )]
}

/// What Settings shows: the filled fields, what isn't valid, the steps and
/// for Google Cloud the estimate.
pub(crate) fn plan(ctx: &Ctx, params: &SetupParams) -> SetupPlan {
    let p = filled(ctx, params);
    SetupPlan {
        problems: problems(&p),
        notes: notes(ctx, &p),
        steps: plan_steps(ctx, &p),
        estimate: if is_cloud(&p) {
            estimate(&p).ok()
        } else {
            None
        },
        machine_types: MACHINE_TYPES
            .iter()
            .map(|m| MachineTypeView {
                id: m.id,
                label: m.label,
                hourly_usd: m.hourly_micro as f64 / 1e6,
                arm: m.arm,
            })
            .collect(),
        params: p,
    }
}

// ---- The estimate --------------------------------------------------------

fn cents_of_micro(micro: u64) -> u64 {
    (micro + 5_000) / 10_000
}

/// The monthly estimate: the machine at the expected hours plus the disk
/// all month, at [`MACHINE_TYPES`]' approximate prices.
pub(crate) fn estimate(p: &SetupParams) -> Result<Estimate, String> {
    let m = machine(&p.machine_type).ok_or("Pick a machine type from the list.")?;
    if !(1..=744).contains(&p.hours_per_month) || !(20..=2000).contains(&p.disk_gb) {
        return Err("Expected hours a month are 1 to 744, and the boot disk 20 to 2000 GB.".into());
    }
    let compute = m.hourly_micro * u64::from(p.hours_per_month);
    let disk = DISK_MICRO_PER_GB_MONTH * u64::from(p.disk_gb);
    let nat = NAT_MICRO_PER_VM_HOUR * u64::from(p.hours_per_month) + NAT_IP_MICRO_PER_MONTH;
    // The sum of the lines as shown, so they add up to it.
    let total_cents = cents_of_micro(compute) + cents_of_micro(disk) + cents_of_micro(nat);
    Ok(Estimate {
        machine_type: m.id.into(),
        machine_label: m.label.into(),
        hourly_usd: m.hourly_micro as f64 / 1e6,
        hours: p.hours_per_month,
        compute_usd: cents_of_micro(compute) as f64 / 100.0,
        disk_gb: p.disk_gb,
        disk_usd: cents_of_micro(disk) as f64 / 100.0,
        nat_usd: cents_of_micro(nat) as f64 / 100.0,
        total_usd: total_cents as f64 / 100.0,
        total_cents,
        region: p.zone.rsplit_once('-').map(|(r, _)| r).unwrap_or("").into(),
        prices_region: PRICES_REGION,
        prices_taken: PRICES_TAKEN,
    })
}

fn usd(cents: u64) -> String {
    format!("${}.{:02}", cents / 100, cents % 100)
}

// ---- Running -------------------------------------------------------------

/// What one command did.
#[derive(Debug, Clone, Default, PartialEq)]
struct Outcome {
    code: Option<i32>,
    out: String,
    err: String,
    /// Why it didn't run or didn't finish.
    not_run: Option<String>,
}

impl Outcome {
    fn ok(&self) -> bool {
        self.not_run.is_none() && self.code == Some(0)
    }

    fn said(text: &str) -> Outcome {
        Outcome {
            code: Some(0),
            out: text.into(),
            ..Outcome::default()
        }
    }

    fn skipped(why: String) -> Outcome {
        Outcome {
            not_run: Some(why),
            ..Outcome::default()
        }
    }

    /// The last line it printed on stderr (else stdout), one line, clipped.
    fn last(&self) -> String {
        let line = self
            .err
            .lines()
            .rev()
            .chain(self.out.lines().rev())
            .map(str::trim)
            .find(|l| !l.is_empty())
            .unwrap_or("");
        clip(line, 300)
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut cut: String = text.chars().take(max).collect();
        cut.push('…');
        cut
    }
}

/// The end of `text`, at most `max` characters.
fn tail(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max {
        text.to_string()
    } else {
        let mut cut = String::from("…");
        cut.extend(text.chars().skip(n - max));
        cut
    }
}

fn reader<R: Read + Send + 'static>(pipe: Option<R>) -> std::sync::mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut pipe) = pipe {
            let _ = pipe.by_ref().take(1 << 20).read_to_end(&mut bytes);
        }
        let _ = tx.send(bytes);
    });
    rx
}

/// Run `cmd` with stdin closed in a process group of its own, killed with
/// everything it started after `limit`.
fn bounded(mut cmd: Command, limit: Duration) -> Outcome {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::process_group::isolate_probe(&mut cmd);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::skipped(format!("it couldn't start ({e})")),
    };
    let pid = child.id();
    let out = reader(child.stdout.take());
    let err = reader(child.stderr.take());
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                crate::process_group::end_probe_group(pid);
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    if status.is_some() {
        crate::process_group::end_probe_group(pid);
    }
    let grace = Duration::from_secs(2);
    let text = |rx: std::sync::mpsc::Receiver<Vec<u8>>| {
        String::from_utf8_lossy(&rx.recv_timeout(grace).unwrap_or_default()).into_owned()
    };
    let (out, err) = (text(out), text(err));
    match status {
        Some(status) => Outcome {
            code: status.code(),
            out,
            err,
            not_run: None,
        },
        None => Outcome {
            code: None,
            out,
            err,
            not_run: Some(format!(
                "it didn't finish in {} s and was stopped",
                limit.as_secs()
            )),
        },
    }
}

fn run_shell(ctx: &Ctx, script: &str, limit: Duration) -> Outcome {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg("-c")
        .arg(script)
        .current_dir(&ctx.home)
        .env("HOME", &ctx.home)
        .env("PATH", &ctx.path);
    bounded(cmd, limit)
}

fn run_gcloud(ctx: &Ctx, args: &[String], limit: Duration) -> Outcome {
    let mut cmd = Command::new(&ctx.gcloud);
    cmd.args(args)
        .current_dir(&ctx.home)
        .env("HOME", &ctx.home)
        .env("PATH", &ctx.path);
    bounded(cmd, limit)
}

/// `ssh -G host` as `(keyword, value)` pairs, in order.
fn ssh_g(ctx: &Ctx, host: &str) -> Option<Vec<(String, String)>> {
    if !valid_host(host) {
        return None;
    }
    let mut cmd = Command::new(&ctx.ssh);
    cmd.args(["-G", host])
        .env("HOME", &ctx.home)
        .env("PATH", &ctx.path);
    let o = bounded(cmd, QUICK);
    o.ok().then(|| parse_ssh_g(&o.out))
}

fn parse_ssh_g(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.split_once(' '))
        .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string()))
        .collect()
}

fn first(g: &[(String, String)], key: &str) -> Option<String> {
    g.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Whether `ssh -G alias` came from a Host block (its HostName isn't the
/// alias itself).
fn has_block(g: &[(String, String)], alias: &str) -> bool {
    first(g, "hostname").is_some_and(|h| !h.eq_ignore_ascii_case(alias))
}

/// Which runner this Mac uses now.
fn named_runner(ctx: &Ctx) -> Option<String> {
    match &ctx.slot_remote {
        None => crate::remote_session::configured_runner(),
        Some(path) => std::fs::read_to_string(path).ok().and_then(|text| {
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
                .find(|l| !l.is_empty())
                .filter(|h| valid_alias(h))
        }),
    }
}

fn run_builtin(ctx: &Ctx, p: &SetupParams, cmd: &Cmd, b: Builtin) -> Outcome {
    let alias = p.alias.as_str();
    match b {
        Builtin::ReadRunner => Outcome::said(&named_runner(ctx).unwrap_or_default()),
        Builtin::Status => {
            let status = crate::remote_status::check(&ctx.ssh, alias);
            Outcome {
                code: Some(match status.state {
                    "ready" => 0,
                    "unreachable" => 255,
                    _ => 1,
                }),
                out: serde_json::to_string(&status).unwrap_or_default(),
                err: status.message.clone().unwrap_or_default(),
                not_run: None,
            }
        }
        Builtin::SetRunner => {
            let set = match &ctx.slot_remote {
                None => crate::runner_owner::set_runner(Some(alias)).map(|_| ()),
                Some(path) => crate::runner_owner::set_runner_at(path, Some(alias), None),
            };
            match set {
                Ok(()) => Outcome::said(&format!("{alias} is this Mac's runner.")),
                Err(e) => Outcome {
                    code: Some(1),
                    err: e,
                    ..Outcome::default()
                },
            }
        }
        Builtin::RecordOwner => match crate::runner_owner::record_owner(&ctx.ssh, alias) {
            Ok(_) => Outcome::said(&format!("Recorded as {alias}'s owner.")),
            Err(e) => Outcome {
                code: Some(1),
                err: e,
                ..Outcome::default()
            },
        },
        Builtin::EnsureTool => match crate::build_slots::ensure_tool() {
            // `ensure_tool` answers with its folder.
            Ok(bin) => Outcome::said(&format!(
                "{} is in place.",
                bin.join("hollow-slot").display()
            )),
            Err(e) => Outcome {
                code: Some(1),
                err: e,
                ..Outcome::default()
            },
        },
        Builtin::TestCall => {
            let mut o = run_shell(ctx, &cmd.shown, cmd.limit);
            let all = format!("{}\n{}", o.out, o.err);
            if o.ok()
                && (all.contains("running on this Mac instead")
                    || all.contains("no remote runner is set up")
                    || all.contains("is turned off"))
            {
                o.code = Some(1);
                let why = all
                    .lines()
                    .find(|l| l.starts_with("hollow-slot: ") && l.contains("this Mac"))
                    .unwrap_or("")
                    .trim()
                    .to_string();
                o.err = format!("hollow-slot ran it on this Mac, not on {alias}: {why}");
            }
            o
        }
    }
}

fn run_cmd(ctx: &Ctx, p: &SetupParams, cmd: &Cmd) -> Outcome {
    match &cmd.run {
        Run::Shell => run_shell(ctx, &cmd.shown, cmd.limit),
        Run::Gcloud(args) => run_gcloud(ctx, args, cmd.limit),
        Run::Hollow(b) => run_builtin(ctx, p, cmd, *b),
    }
}

fn ran(cmd: &Cmd, o: &Outcome) -> RanCmd {
    let mut output = String::new();
    for part in [o.out.trim_end(), o.err.trim_end()] {
        if !part.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(part);
        }
    }
    if let Some(why) = &o.not_run {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&format!("({why})"));
    }
    RanCmd {
        shown: cmd.shown.clone(),
        code: o.code,
        output: tail(&output, OUTPUT_MAX),
    }
}

// ---- Check -----------------------------------------------------------------

/// What earlier steps found that later ones depend on: a reason to skip.
#[derive(Debug, Default)]
struct Gates {
    admin: Option<String>,
    runner: Option<String>,
    gcloud: Option<String>,
    vm: Option<String>,
}

impl Gates {
    fn closed(&self, cmd: &Cmd) -> Option<String> {
        match (cmd.on, &cmd.run) {
            (On::Admin, _) => self.admin.clone(),
            (On::Runner, _) | (On::Hollow, Run::Hollow(Builtin::Status)) => {
                self.vm.clone().or_else(|| self.runner.clone())
            }
            (On::Gcloud, _) => self.gcloud.clone(),
            _ => None,
        }
    }
}

fn state(id: &str, state: &'static str, summary: String, details: Vec<String>) -> StepState {
    StepState {
        id: id.into(),
        state,
        summary,
        details,
        needs_by_hand: false,
        ran: Vec::new(),
    }
}

/// An ssh call that failed itself (exit 255): what it means for `dest`.
fn ssh_failure(dest: &str, o: &Outcome) -> (&'static str, String) {
    let last = o.last();
    let lower = last.to_ascii_lowercase();
    if lower.contains("permission denied") || lower.contains("too many authentication failures") {
        (
            "missing",
            format!("{dest} doesn't let this Mac's key in yet ({last})."),
        )
    } else if lower.contains("host key verification failed") {
        ("missing", format!("This Mac doesn't know {dest}'s host key yet: run ssh {dest} once in Terminal and confirm it."))
    } else {
        (
            "unknown",
            format!(
                "{dest} didn't answer{}. {}",
                if last.is_empty() {
                    String::new()
                } else {
                    format!(": {last}")
                },
                crate::remote_status::likely_causes(&last)
            ),
        )
    }
}

fn not_run_state(id: &str, o: &Outcome) -> Option<StepState> {
    o.not_run
        .as_ref()
        .map(|why| state(id, "unknown", format!("Couldn't tell: {why}."), Vec::new()))
}

/// `key=value` lines (the first `=` splits).
fn kv(out: &str) -> std::collections::BTreeMap<String, String> {
    out.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect()
}

fn judge_admin(p: &SetupParams, o: &Outcome) -> StepState {
    let id = "admin";
    if let Some(s) = not_run_state(id, o) {
        return s;
    }
    match o.code {
        Some(0) => {
            let lines: Vec<&str> = o.out.lines().map(str::trim).collect();
            let user = lines.first().copied().unwrap_or("");
            let version = lines.get(1).copied().unwrap_or("");
            let arch = lines.get(2).copied().unwrap_or("");
            let sudo = lines.contains(&"sudo=yes");
            if !version.starts_with(|c: char| c.is_ascii_digit()) {
                return state(id, "missing", format!("{} answered, but it doesn't look like a Mac (no macOS version). This path is for a Mac; use Google Cloud or name an existing runner instead.", p.admin), Vec::new());
            }
            state(
                id,
                "done",
                format!("Logs in as {user}: macOS {version}, {arch}."),
                vec![if sudo {
                    "sudo works there without a password (sudo -n), so Apply can make changes there.".into()
                } else {
                    "sudo asks for a password there, so the steps that change accounts, keys and sleep settings stop with commands to run by hand in Terminal instead.".into()
                }],
            )
        }
        Some(255) => {
            let (s, why) = ssh_failure(&p.admin, o);
            let mut details = Vec::new();
            if s == "missing" && why.contains("key in") {
                details.push(format!(
                    "Run ssh-copy-id {} in Terminal once (it asks that account's password).",
                    p.admin
                ));
            }
            state(id, s, why, details)
        }
        _ => state(
            id,
            "unknown",
            format!("{} answered, but the check failed ({}).", p.admin, o.last()),
            Vec::new(),
        ),
    }
}

fn judge_account(p: &SetupParams, o: &Outcome) -> StepState {
    let id = "account";
    if let Some(s) = not_run_state(id, o) {
        return s;
    }
    if o.code != Some(0) {
        return state(
            id,
            "unknown",
            format!("Couldn't tell: the check failed ({}).", o.last()),
            Vec::new(),
        );
    }
    let f = kv(&o.out);
    let a = &p.account;
    if f.get("account").map(String::as_str) != Some("yes") {
        return state(
            id,
            "missing",
            format!("There is no {a} account there yet."),
            Vec::new(),
        );
    }
    let mut missing = Vec::new();
    if f.get("admin").map(String::as_str) == Some("yes") {
        missing.push(format!("{a} is an admin there; the runner account must be a standard user. Apply refuses: pick another account name."));
    }
    if f.get("hidden").map(String::as_str) != Some("yes") {
        missing.push(format!("{a} isn't hidden from the login window."));
    }
    if f.get("ssh_group").map(String::as_str) == Some("no") {
        missing.push(format!(
            "Remote Login there admits only listed users, and {a} isn't one."
        ));
    }
    if f.get("home").map(String::as_str) != Some("yes") {
        missing.push(format!("{a} has no home folder."));
    }
    if missing.is_empty() {
        state(
            id,
            "done",
            format!("{a} is a hidden standard user."),
            Vec::new(),
        )
    } else {
        state(
            id,
            "missing",
            format!("{a} is there, but not ready."),
            missing,
        )
    }
}

/// `path` with a leading `~/` as this Mac's home, for comparing.
fn norm_path(ctx: &Ctx, path: &str) -> PathBuf {
    ctx.expand(path)
}

fn judge_key(ctx: &Ctx, p: &SetupParams, outs: &[Outcome], gates: &mut Gates) -> StepState {
    let id = "key";
    let mut missing = Vec::new();
    let mut unknown = Vec::new();
    let mut fp = String::new();
    match outs.first() {
        Some(o) if o.ok() => {
            fp = o
                .out
                .split_whitespace()
                .find(|w| w.starts_with("SHA256:"))
                .unwrap_or("")
                .to_string();
        }
        Some(o) if o.not_run.is_some() => unknown.push(format!(
            "Couldn't read {}: {}.",
            p.key_file,
            o.not_run.clone().unwrap_or_default()
        )),
        _ => missing.push(format!("There is no key at {} yet.", p.key_file)),
    }
    let cloud = is_cloud(p);
    match outs.get(1) {
        Some(o) if o.ok() => {
            let g = parse_ssh_g(&o.out);
            if !has_block(&g, &p.alias) {
                missing.push(format!("~/.ssh/config has no Host {} yet.", p.alias));
            } else {
                let host = first(&g, "hostname").unwrap_or_default();
                let want_host = if cloud { &p.instance } else { &p.host_name };
                if !host.eq_ignore_ascii_case(want_host) {
                    missing.push(format!("Host {} goes to {host}, not {want_host}.", p.alias));
                }
                if !cloud {
                    let user = first(&g, "user").unwrap_or_default();
                    if user != p.account {
                        missing.push(format!("Host {} logs in as {user}, not {} (an earlier Host or Match block in ~/.ssh/config may set User first).", p.alias, p.account));
                    }
                } else if !first(&g, "proxycommand").is_some_and(|c| c.contains("gcp-runner-proxy"))
                {
                    missing.push(format!(
                        "Host {} doesn't use the gcp-runner-proxy ProxyCommand.",
                        p.alias
                    ));
                }
                let want_key = norm_path(ctx, &p.key_file);
                if !g
                    .iter()
                    .any(|(k, v)| k == "identityfile" && norm_path(ctx, v) == want_key)
                {
                    missing.push(format!("Host {} doesn't offer {}.", p.alias, p.key_file));
                }
            }
        }
        Some(o) => unknown.push(format!("ssh -G {} failed ({}).", p.alias, o.last())),
        None => {}
    }
    if cloud {
        match outs.get(2) {
            Some(o) if o.ok() && o.out.contains("proxy=yes") => {}
            Some(o) if o.not_run.is_some() => {
                unknown.push("Couldn't look for the proxy helper.".into())
            }
            _ => missing.push(format!("There is no {PROXY_PATH} yet.")),
        }
    } else {
        match outs.get(2) {
            Some(o) if o.ok() => {}
            Some(o) if o.not_run.is_some() => {
                let why = o.not_run.clone().unwrap_or_default();
                match why.strip_prefix("not asked: ") {
                    // Not tried without a Host block, which `missing` says.
                    Some(rest) => gates.runner = Some(rest.to_string()),
                    None => {
                        gates.runner = Some(why.clone());
                        unknown.push(format!("Couldn't log in to {}: {why}.", p.alias));
                    }
                }
            }
            Some(o) => {
                let (s, why) = if o.code == Some(255) {
                    ssh_failure(&p.alias, o)
                } else {
                    (
                        "unknown",
                        format!("{} answered, but true failed there.", p.alias),
                    )
                };
                gates.runner = Some(format!("this Mac can't log in to {} yet", p.alias));
                if s == "missing" {
                    missing.push(why);
                } else {
                    unknown.push(why);
                }
            }
            None => {}
        }
    }
    let what = if cloud {
        format!("{} reaches the VM through IAP with {}", p.alias, p.key_file)
    } else {
        format!(
            "This Mac logs in to {} as {} with {}",
            p.alias, p.account, p.key_file
        )
    };
    if !missing.is_empty() {
        missing.extend(unknown);
        state(id, "missing", "Not set up yet.".into(), missing)
    } else if !unknown.is_empty() {
        state(id, "unknown", "Couldn't tell.".into(), unknown)
    } else {
        state(
            id,
            "done",
            format!(
                "{what}{}.",
                if fp.is_empty() {
                    String::new()
                } else {
                    format!(" ({fp})")
                }
            ),
            Vec::new(),
        )
    }
}

fn judge_tools(o: &Outcome) -> StepState {
    let id = "tools";
    if let Some(s) = not_run_state(id, o) {
        return s;
    }
    if o.code == Some(255) {
        let (s, why) = ssh_failure("the runner", o);
        return state(id, s, why, Vec::new());
    }
    let f = kv(&o.out);
    let names = [
        ("git", "git"),
        ("node", "Node"),
        ("cargo", "cargo"),
        ("rustc", "rustc"),
        ("uv", "uv"),
        ("python3", "Python"),
        ("gh", "gh"),
        ("gcloud", "gcloud"),
        ("pg_ctl", "PostgreSQL (pg_ctl)"),
        ("chromium", "Playwright's Chromium"),
    ];
    let mut missing: Vec<String> = names
        .iter()
        .filter(|(k, _)| f.get(*k).is_none_or(|v| v.is_empty()))
        .map(|(_, label)| label.to_string())
        .collect();
    let mut details = Vec::new();
    if f.get("rustc").is_some_and(|v| !v.is_empty())
        && !f
            .get("toolchains")
            .is_some_and(|t| t.contains(RUST_TOOLCHAIN))
    {
        missing.push(format!("Rust {RUST_TOOLCHAIN}"));
    }
    if let Some(path) = f.get("path") {
        let has = |dir: &str| path.split(':').any(|d| d.ends_with(dir));
        if !(has("/.cargo/bin") && (has("/opt/homebrew/bin") || has("/usr/local/bin"))) {
            missing.push("Homebrew and cargo on its PATH (~/.zshenv)".into());
        }
    }
    if f.get("clt").map(String::as_str) == Some("no") {
        details.push("Apple's Command Line Tools aren't installed there: run xcode-select --install in Terminal on that Mac as the admin.".into());
    }
    let have: Vec<String> = names
        .iter()
        .filter_map(|(k, _)| f.get(*k).filter(|v| !v.is_empty()).map(|v| clip(v, 40)))
        .collect();
    if missing.is_empty() {
        state(id, "done", "Everything is there.".into(), have)
    } else {
        details.insert(0, format!("Not there: {}.", missing.join(", ")));
        state(
            id,
            "missing",
            format!("{} missing.", missing.len()),
            details,
        )
    }
}

fn judge_preflight(o: &Outcome) -> StepState {
    let id = "preflight";
    if let Some(s) = not_run_state(id, o) {
        return s;
    }
    if o.code == Some(255) {
        let (s, why) = ssh_failure("the runner", o);
        return state(id, s, why, Vec::new());
    }
    let lines: Vec<&str> = o
        .out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.first() == Some(&"preflight=none") || lines.is_empty() {
        return state(
            id,
            "missing",
            "No hollow-run-preflight there yet.".into(),
            Vec::new(),
        );
    }
    let exit = lines
        .last()
        .and_then(|l| l.strip_prefix("exit="))
        .and_then(|v| v.parse::<i32>().ok());
    let said = if lines.len() >= 3 {
        clip(lines[lines.len() - 2], 300)
    } else {
        String::new()
    };
    let path = lines[0].strip_prefix("preflight=").unwrap_or("");
    let mut details = vec![format!("{path} says: {said}")];
    if exit != Some(0) {
        details.push(format!(
            "It refuses work now{}.",
            if said.is_empty() {
                String::new()
            } else {
                format!(" ({said})")
            }
        ));
    }
    state(id, "done", "hollow-run-preflight is there.".into(), details)
}

fn judge_sleep(o: &Outcome) -> StepState {
    let id = "sleep";
    if let Some(s) = not_run_state(id, o) {
        return s;
    }
    if o.code != Some(0) {
        return state(
            id,
            "unknown",
            format!("Couldn't tell: the check failed ({}).", o.last()),
            Vec::new(),
        );
    }
    let (pm, sshd) = o.out.split_once("---").unwrap_or((&o.out, ""));
    let value = |name: &str| {
        pm.lines()
            .map(str::split_whitespace)
            .find_map(|mut w| (w.next() == Some(name)).then(|| w.next().unwrap_or("").to_string()))
    };
    let mut missing = Vec::new();
    match value("sleep").as_deref() {
        Some("0") => {}
        Some(n) => missing.push(format!("It sleeps after {n} idle minutes.")),
        None => missing.push("pmset didn't say when it sleeps.".into()),
    }
    if !matches!(value("disksleep").as_deref(), Some("0")) {
        missing.push("Its disks sleep when idle.".into());
    }
    if !matches!(value("autorestart").as_deref(), Some("1")) {
        missing.push("It stays off after a power cut (autorestart is off).".into());
    }
    let alive = sshd.lines().any(|l| {
        let mut w = l.split_whitespace();
        w.next()
            .is_some_and(|k| k.eq_ignore_ascii_case("ClientAliveInterval"))
            && w.next()
                .and_then(|v| v.parse::<u32>().ok())
                .is_some_and(|v| v > 0)
    });
    if !alive {
        missing
            .push("sshd has no ClientAliveInterval, so a dead connection's session stays.".into());
    }
    if missing.is_empty() {
        state(
            id,
            "done",
            "Never sleeps, starts again after a power cut, and sshd drops dead sessions.".into(),
            Vec::new(),
        )
    } else {
        state(id, "missing", "Not set yet.".into(), missing)
    }
}

fn judge_use(p: &SetupParams, outs: &[Outcome]) -> StepState {
    let id = "use";
    let mut missing = Vec::new();
    let mut unknown = Vec::new();
    let named = matches!(outs.first(), Some(o) if o.ok() && o.out.trim() == p.alias);
    match outs.first() {
        _ if named => {}
        Some(o) if o.ok() && !o.out.trim().is_empty() => missing.push(format!(
            "This Mac's runner is {} now, not {}.",
            o.out.trim(),
            p.alias
        )),
        _ => missing.push("This Mac has no runner named yet.".into()),
    }
    let mut summary = format!("{} is this Mac's runner, yours, and ready.", p.alias);
    match outs.get(1) {
        Some(o) if o.not_run.is_some() => unknown.push(format!(
            "Couldn't ask {}: {}.",
            p.alias,
            o.not_run.clone().unwrap_or_default()
        )),
        Some(o) => {
            let status: Value = serde_json::from_str(&o.out).unwrap_or(Value::Null);
            let st = status.get("state").and_then(Value::as_str).unwrap_or("");
            let message = status
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            match st {
                "ready" => {
                    if let Some(pre) = status.get("preflight").and_then(Value::as_str) {
                        summary = format!("{summary} Its preflight: {pre}");
                    }
                }
                "unreachable" => unknown.push(message),
                _ => missing.push(if message.is_empty() {
                    format!("{} isn't ready ({st}).", p.alias)
                } else {
                    message
                }),
            }
            match status.pointer("/owner/state").and_then(Value::as_str) {
                Some("you") => {}
                Some("other") => missing.push("Another Hollow is recorded as its owner.".into()),
                Some(_) => missing.push(format!(
                    "{} has no owner recorded (Apply records this Hollow).",
                    p.alias
                )),
                None => {}
            }
            if let Some(ws) = status.pointer("/owner/warnings").and_then(Value::as_array) {
                for w in ws.iter().filter_map(Value::as_str) {
                    if !missing.iter().any(|m| m == w) {
                        missing.push(w.to_string());
                    }
                }
            }
        }
        None => {}
    }
    if !missing.is_empty() {
        missing.extend(unknown);
        let summary = if named {
            // In use already (a runner set up by hand, or before owners).
            format!(
                "{} is this Mac's runner, with something left to do.",
                p.alias
            )
        } else {
            "Not in use yet.".into()
        };
        state(id, "missing", summary, missing)
    } else if !unknown.is_empty() {
        state(id, "unknown", "Couldn't tell.".into(), unknown)
    } else {
        state(id, "done", summary, Vec::new())
    }
}

fn gcloud_failed(o: &Outcome) -> String {
    match &o.not_run {
        Some(why) => format!("gcloud didn't run: {why}"),
        None => o.last(),
    }
}

fn not_found(o: &Outcome) -> bool {
    let e = o.err.to_ascii_lowercase();
    o.not_run.is_none()
        && (e.contains("not found") || e.contains("not_found") || e.contains("was not found"))
}

fn judge_gcloud(p: &SetupParams, outs: &[Outcome], gates: &mut Gates) -> StepState {
    let id = "gcloud";
    let account = outs.first().cloned().unwrap_or_default();
    if account.not_run.is_some() {
        let why = format!("gcloud isn't installed on this Mac ({}). Install the Google Cloud CLI, then Check again.", account.not_run.clone().unwrap_or_default());
        gates.gcloud = Some("gcloud isn't installed on this Mac".into());
        return state(id, "missing", why, Vec::new());
    }
    let who = account.out.trim().to_string();
    if !account.ok() || who.is_empty() {
        gates.gcloud = Some("gcloud isn't signed in on this Mac".into());
        return state(
            id,
            "missing",
            "gcloud isn't signed in on this Mac: run gcloud auth login in Terminal.".into(),
            Vec::new(),
        );
    }
    match outs.get(1) {
        Some(o) if o.ok() && o.out.contains("ACTIVE") => state(
            id,
            "done",
            format!("Signed in as {who}; project {} is active.", p.project),
            Vec::new(),
        ),
        Some(o) => {
            gates.gcloud = Some(format!("project {} can't be read", p.project));
            state(
                id,
                "missing",
                format!(
                    "Signed in as {who}, but project {} can't be read: {}",
                    p.project,
                    gcloud_failed(o)
                ),
                Vec::new(),
            )
        }
        None => state(id, "unknown", "Couldn't tell.".into(), Vec::new()),
    }
}

fn judge_cloud(p: &SetupParams, outs: &[Outcome], gates: &mut Gates) -> StepState {
    let id = "cloud";
    let n = Names::of(p);
    let mut missing = Vec::new();
    let mut unknown = Vec::new();
    let mut found = |o: Option<&Outcome>, what: String, ok: &dyn Fn(&Outcome) -> bool| match o {
        Some(o) if o.not_run.is_some() => unknown.push(format!("{what}: {}", gcloud_failed(o))),
        Some(o) if o.ok() && ok(o) => {}
        Some(o) if o.ok() || not_found(o) => missing.push(what),
        Some(o) => unknown.push(format!("{what}: {}", gcloud_failed(o))),
        None => {}
    };
    found(
        outs.first(),
        "the Compute Engine, IAP and Billing Budget APIs".into(),
        &|o| {
            services()
                .iter()
                .all(|s| o.out.lines().any(|l| l.trim() == *s))
        },
    );
    found(outs.get(1), format!("the VM {}", n.instance), &|o| {
        !o.out.trim().is_empty()
    });
    found(outs.get(2), format!("the service account {}", n.sa), &|o| {
        !o.out.trim().is_empty()
    });
    found(
        outs.get(3),
        format!("the firewall rule {}", n.firewall()),
        &|o| o.out.contains(IAP_RANGE),
    );
    let name = n.budget_name();
    found(outs.get(4), format!("the budget alert \"{name}\""), &|o| {
        o.out.lines().any(|l| l.trim() == name)
    });
    found(outs.get(5), format!("the Cloud NAT {}", n.nat()), &|o| {
        !o.out.trim().is_empty()
    });
    let vm = outs
        .get(1)
        .filter(|o| o.ok())
        .map(|o| o.out.trim().to_string())
        .unwrap_or_default();
    if vm != "RUNNING" {
        gates.vm = Some(match outs.get(1) {
            Some(o) if o.ok() => format!("the VM is {vm}, and asking it would start it"),
            Some(o) if not_found(o) => "the VM isn't there".into(),
            _ => "couldn't tell whether the VM is running".into(),
        });
    }
    if !missing.is_empty() {
        let mut details = vec![format!("Not there yet: {}.", missing.join(", "))];
        details.extend(unknown);
        state(id, "missing", "Not created yet.".into(), details)
    } else if !unknown.is_empty() {
        state(id, "unknown", "Couldn't tell.".into(), unknown)
    } else {
        state(
            id,
            "done",
            format!(
                "The VM is {vm}; its service account, firewall rule, Cloud NAT and budget alert are there."
            ),
            Vec::new(),
        )
    }
}

/// Judge one step from its check commands' outcomes.
fn judge(ctx: &Ctx, p: &SetupParams, id: &str, outs: &[Outcome], gates: &mut Gates) -> StepState {
    let one = outs.first().cloned().unwrap_or_default();
    match id {
        "admin" => {
            let s = judge_admin(p, &one);
            if s.state != "done" {
                gates.admin = Some(format!(
                    "the admin login didn't work ({})",
                    s.summary.trim_end_matches('.')
                ));
            }
            s
        }
        "account" => judge_account(p, &one),
        "key" | "cloud-local" => {
            let mut s = judge_key(ctx, p, outs, gates);
            s.id = id.into();
            s
        }
        "tools" => judge_tools(&one),
        "preflight" => judge_preflight(&one),
        "sleep" => judge_sleep(&one),
        "use" => judge_use(p, outs),
        "gcloud" => judge_gcloud(p, outs, gates),
        "cloud" => judge_cloud(p, outs, gates),
        _ => state(id, "unknown", "Couldn't tell.".into(), Vec::new()),
    }
}

/// Run `step`'s check commands (skipping those an earlier step ruled out)
/// and judge them.
fn check_step(ctx: &Ctx, p: &SetupParams, step: &Step, gates: &mut Gates) -> StepState {
    let mut outs = Vec::new();
    let mut ran_cmds = Vec::new();
    for c in &step.check {
        // The key step's `ssh <alias> true` only through a Host block: a
        // bare name would get this Mac's other keys offered to it.
        let no_block = step.id == "key"
            && c.on == On::Runner
            && outs
                .get(1)
                .is_some_and(|o: &Outcome| o.ok() && !has_block(&parse_ssh_g(&o.out), &p.alias));
        let o = match gates.closed(c) {
            Some(why) => Outcome::skipped(format!("not asked: {why}")),
            None if no_block => Outcome::skipped(format!(
                "not asked: ~/.ssh/config has no Host {} yet",
                p.alias
            )),
            None => run_cmd(ctx, p, c),
        };
        ran_cmds.push(ran(c, &o));
        outs.push(o);
    }
    let mut s = judge(ctx, p, step.id, &outs, gates);
    s.ran = ran_cmds;
    s
}

/// Check every step of `params`' path in order. Read-only: no check
/// command writes anything anywhere.
pub(crate) fn check(ctx: &Ctx, params: &SetupParams) -> Result<Vec<StepState>, String> {
    let _one = one_at_a_time()?;
    let p = filled(ctx, params);
    validate(&p)?;
    let mut gates = Gates::default();
    Ok(plan_steps(ctx, &p)
        .iter()
        .map(|step| check_step(ctx, &p, step, &mut gates))
        .collect())
}

// ---- Apply -----------------------------------------------------------------

/// What `~/.ssh/config` already says about a Host block.
#[derive(Debug, PartialEq)]
enum Block {
    Absent,
    Same,
    Different(String),
}

/// Look for `guard.alias`'s Host block in `~/.ssh/config` (that file only)
/// and compare it with what the append would write.
fn host_block(ctx: &Ctx, guard: &HostGuard) -> Block {
    let Some(values) = own_block(ctx, &guard.alias) else {
        return Block::Absent;
    };
    for (k, want) in &guard.want {
        let have = values
            .iter()
            .find(|(key, _)| key == k)
            .map(|(_, v)| v.as_str());
        let same = match (*k, have) {
            ("identityfile", Some(h)) => ctx.expand(h) == ctx.expand(want),
            ("hostname", Some(h)) => h.eq_ignore_ascii_case(want),
            (_, Some(h)) => h == want,
            (_, None) => false,
        };
        if !same {
            return Block::Different(format!(
                "~/.ssh/config already has a Host {} block, and it {}. Hollow doesn't rewrite it: change it by hand, or pick another runner alias.",
                guard.alias,
                match have {
                    Some(h) => format!("sets {} {h}, not {want}", ssh_keyword(k)),
                    None => format!("has no {} {want}", ssh_keyword(k)),
                }
            ));
        }
    }
    Block::Same
}

/// The keywords (lowercase, first value each) of the `Host` blocks in
/// `~/.ssh/config` (that file only) that name `alias`; `None` when none
/// does. Unlike `ssh -G`, nothing from `Host *` or ssh's defaults.
fn own_block(ctx: &Ctx, alias: &str) -> Option<Vec<(String, String)>> {
    let text = std::fs::read_to_string(ctx.home.join(".ssh/config")).ok()?;
    let mut found = false;
    let mut inside = false;
    let mut values: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (key, rest) = line
            .split_once(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or((line, ""));
        let key = key.to_ascii_lowercase();
        let rest = rest
            .trim_start_matches(|c: char| c.is_whitespace() || c == '=')
            .trim();
        if key == "host" || key == "match" {
            inside = key == "host"
                && rest
                    .split_whitespace()
                    .any(|pat| pat.eq_ignore_ascii_case(alias));
            found |= inside;
            continue;
        }
        // ssh takes a keyword's first value, but every IdentityFile.
        if inside && (key == "identityfile" || !values.iter().any(|(k, _)| *k == key)) {
            values.push((key, rest.to_string()));
        }
    }
    found.then_some(values)
}

fn sudo_wanted_password(o: &Outcome) -> bool {
    let e = o.err.to_ascii_lowercase();
    e.contains("a password is required") || e.contains("a terminal is required")
}

/// The state of an apply that stopped at `cmd`.
fn stopped(step: &Step, ran_cmds: Vec<RanCmd>, cmd: &Cmd, o: &Outcome) -> StepState {
    let n = ran_cmds.len();
    let total = step.apply.len();
    let mut s = if cmd.sudo && sudo_wanted_password(o) {
        let mut s = state(step.id, "failed", "sudo there wants a password, so Apply stopped there. Run the by-hand commands in Terminal (sudo asks for the password there), then Check.".into(), Vec::new());
        s.needs_by_hand = true;
        s
    } else {
        let why = match (&o.not_run, o.code) {
            (Some(why), _) => why.clone(),
            (None, Some(255)) if cmd.on == On::Admin || cmd.on == On::Runner => {
                ssh_failure(cmd.who.split(" (").next().unwrap_or(""), o).1
            }
            (None, code) => format!(
                "exit {}{}",
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "by a signal".into()),
                {
                    let last = o.last();
                    if last.is_empty() {
                        String::new()
                    } else {
                        format!(": {last}")
                    }
                }
            ),
        };
        state(
            step.id,
            "failed",
            format!("Command {n} of {total} failed ({why}). Nothing after it ran."),
            vec![format!(
                "It was: {}",
                clip(cmd.shown.lines().next().unwrap_or(""), 200)
            )],
        )
    };
    s.ran = ran_cmds;
    s
}

/// Run `step_id`'s apply commands in order, stopping at the first that
/// fails, then its check. The only way a step changes anything.
pub(crate) fn apply(ctx: &Ctx, params: &SetupParams, step_id: &str) -> Result<StepState, String> {
    let _one = one_at_a_time()?;
    let p = filled(ctx, params);
    validate(&p)?;
    let steps = plan_steps(ctx, &p);
    let step = steps
        .iter()
        .find(|s| s.id == step_id)
        .ok_or_else(|| format!("There is no step {step_id} on this path."))?;
    if step.id == "cloud" {
        return Err("The machine is created with Create, after its monthly estimate.".into());
    }
    if step.apply.is_empty() {
        return Err(format!(
            "{} has nothing to apply: {}",
            step.title,
            step.by_hand_note
                .as_deref()
                .unwrap_or("Check shows what it found.")
        ));
    }
    let mut ran_cmds = Vec::new();
    for c in &step.apply {
        if let Some(guard) = &c.guard {
            match host_block(ctx, guard) {
                Block::Absent => {}
                Block::Same => {
                    ran_cmds.push(RanCmd {
                        shown: c.shown.clone(),
                        code: None,
                        output: format!(
                            "Not run: ~/.ssh/config already has this Host {} block.",
                            guard.alias
                        ),
                    });
                    continue;
                }
                Block::Different(why) => {
                    let mut s = state(step.id, "failed", why, Vec::new());
                    s.ran = ran_cmds;
                    return Ok(s);
                }
            }
        }
        let o = run_cmd(ctx, &p, c);
        ran_cmds.push(ran(c, &o));
        if !o.ok() {
            return Ok(stopped(step, ran_cmds, c, &o));
        }
    }
    let mut gates = Gates::default();
    let mut after = check_step(ctx, &p, step, &mut gates);
    if after.state != "done" {
        after
            .details
            .insert(0, "Apply finished, but Check still finds this:".into());
    }
    ran_cmds.append(&mut after.ran);
    after.ran = ran_cmds;
    Ok(after)
}

/// Create the Google Cloud machine: only with `shown_cents`, the monthly
/// estimate the person confirmed, equal to the one for `params` now (else
/// nothing runs). Runs the create commands in order through `ctx.gcloud`;
/// one that finds its resource already there counts as done, any other
/// failure stops the sequence, naming the command. Then the step's check.
pub(crate) fn create_cloud(
    ctx: &Ctx,
    params: &SetupParams,
    shown_cents: u64,
) -> Result<StepState, String> {
    let _one = one_at_a_time()?;
    let p = filled(ctx, params);
    if !is_cloud(&p) {
        return Err("Create is for the Google Cloud path.".into());
    }
    validate(&p)?;
    let est = estimate(&p)?;
    if shown_cents != est.total_cents {
        return Err(format!(
            "The estimate you confirmed ({}) isn't this machine's ({} a month). Look at the estimate again, then Create.",
            usd(shown_cents),
            usd(est.total_cents)
        ));
    }
    let steps = plan_steps(ctx, &p);
    let step = steps
        .iter()
        .find(|s| s.id == "cloud")
        .ok_or("The Google Cloud path has no machine step.")?;
    let Some((cleanup, cmds)) = step.apply.split_last() else {
        return Err("The machine step has no commands.".into());
    };
    let n = Names::of(&p);
    let budget = n.budget_name();
    let mut budget_there = false;
    let mut nat_there = false;
    let mut ran_cmds = Vec::new();
    let mut result = None;
    for c in cmds {
        let skip = if is_gcloud(c, &["billing", "budgets", "create"]) && budget_there {
            Some(format!("the budget alert \"{budget}\" is already there"))
        } else if is_gcloud(c, &["compute", "routers", "nats", "create"]) && nat_there {
            Some(format!("the Cloud NAT {} is already there", n.nat()))
        } else {
            None
        };
        if let Some(why) = skip {
            ran_cmds.push(RanCmd {
                shown: c.shown.clone(),
                code: None,
                output: format!("Not run: {why}."),
            });
            continue;
        }
        let o = run_cmd(ctx, &p, c);
        if is_gcloud(c, &["billing", "budgets", "list"]) && o.ok() {
            budget_there = o.out.lines().any(|l| l.trim() == budget);
        }
        let mut r = ran(c, &o);
        if is_gcloud(c, &["compute", "routers", "nats", "describe"]) {
            nat_there = o.ok() && !o.out.trim().is_empty();
            // Not there is what a first Create expects: its create comes next.
            if not_found(&o) {
                r.output = format!("{}\n(Not there yet: the next command makes it.)", r.output);
                ran_cmds.push(r);
                continue;
            }
        }
        let there = !o.ok() && o.not_run.is_none() && {
            let e = o.err.to_ascii_lowercase();
            e.contains("already exists") || e.contains("already_exists")
        };
        if there {
            r.output = format!("{}\n(Already there: kept as it is.)", r.output);
        }
        ran_cmds.push(r);
        if !o.ok() && !there {
            result = Some(stopped(step, ran_cmds.clone(), c, &o));
            break;
        }
    }
    // The startup script's removal (the last command shown), whatever happened.
    let o = run_cmd(ctx, &p, cleanup);
    ran_cmds.push(ran(cleanup, &o));
    if let Some(mut s) = result {
        s.ran = ran_cmds.clone();
        s.details.push("What ran before it stays; Check shows what is there. The startup script's temporary file was removed all the same (the last command).".into());
        return Ok(s);
    }
    let mut gates = Gates::default();
    let mut after = check_step(ctx, &p, step, &mut gates);
    ran_cmds.append(&mut after.ran);
    after.ran = ran_cmds;
    Ok(after)
}

// ---- Scripts ---------------------------------------------------------------

/// Hollow's default `hollow-run-preflight` (macOS and Linux): one line,
/// exit 1 to refuse under 20 GB free disk or 30% free memory.
pub(crate) const PREFLIGHT_SH: &str = r##"#!/bin/sh
# hollow-run-preflight: Hollow's default (Settings, Remote runner, Set up).
# Hollow and hollow-slot ask it before sending work here. It prints one line
# and exits 1 to refuse: under 20 GB free disk or 30% free memory. Edit the
# limits to suit this machine; Set up keeps an edited copy beside a new one.
PATH="$HOME/.cargo/bin:$HOME/.local/bin:/opt/cargo/bin:/opt/homebrew/bin:/usr/local/bin:$PATH"
min_gb=20
min_mem=30
host=$(hostname -s 2>/dev/null || uname -n)
kb=$(df -Pk "$HOME" 2>/dev/null | awk 'NR == 2 { print $4 }')
case "$(uname -s)" in
  Darwin) mem=$(sysctl -n kern.memorystatus_level 2>/dev/null) ;;
  *) mem=$(awk '/^MemTotal:/ { t = $2 } /^MemAvailable:/ { a = $2 } END { if (t > 0) printf "%d", a * 100 / t }' /proc/meminfo 2>/dev/null) ;;
esac
case "$kb" in '' | *[!0-9]*) echo "no: couldn't read the free disk on $host"; exit 1 ;; esac
case "$mem" in '' | *[!0-9]*) mem= ;; esac
gb=$((kb / 1048576))
if [ "$gb" -lt "$min_gb" ]; then echo "no: ${gb}GB free disk on $host, under ${min_gb}GB"; exit 1; fi
if [ -n "$mem" ] && [ "$mem" -lt "$min_mem" ]; then echo "no: ${mem}% memory free on $host, under ${min_mem}%"; exit 1; fi
rust=$(rustc --version 2>/dev/null </dev/null | awk '{ print $2 }')
node=$(node --version 2>/dev/null </dev/null)
echo "ok host=$host free=${gb}GB mem=${mem:-?}% rust=${rust:-none} node=${node:-none}"
"##;

/// The ProxyCommand helper for a Google Cloud runner, on this Mac:
/// `gcp-runner-proxy <instance> <project> <zone>`. Starts the VM when it
/// isn't running, waits for it, then hands ssh an IAP tunnel to port 22.
pub(crate) const PROXY_SH: &str = r##"#!/bin/sh
# gcp-runner-proxy: Hollow's ProxyCommand for a Google Cloud runner (Settings,
# Remote runner, Set up). Usage: gcp-runner-proxy <instance> <project> <zone>.
# Starts the VM when it isn't running, waits for it, then connects ssh to its
# port 22 through IAP. The VM powers itself off when idle.
set -u
inst=$1 project=$2 zone=$3
g=
for c in "$(command -v gcloud 2>/dev/null)" /opt/homebrew/bin/gcloud /usr/local/bin/gcloud "$HOME/google-cloud-sdk/bin/gcloud" /opt/homebrew/share/google-cloud-sdk/bin/gcloud; do
  if [ -n "$c" ] && [ -x "$c" ]; then g=$c; break; fi
done
if [ -z "$g" ]; then echo "gcp-runner-proxy: gcloud isn't installed on this Mac" >&2; exit 1; fi
status() { "$g" compute instances describe "$inst" --project="$project" --zone="$zone" --format='value(status)' </dev/null 2>/dev/null; }
s=$(status)
case "$s" in
  RUNNING) ;;
  '') echo "gcp-runner-proxy: couldn't read $inst in $project ($zone)" >&2; exit 1 ;;
  *)
    echo "gcp-runner-proxy: starting $inst (it was $s)" >&2
    "$g" compute instances start "$inst" --project="$project" --zone="$zone" --quiet </dev/null >&2 || exit 1
    i=0
    while [ "$(status)" != RUNNING ]; do
      i=$((i + 1))
      if [ "$i" -ge 60 ]; then echo "gcp-runner-proxy: $inst didn't start in 5 minutes" >&2; exit 1; fi
      sleep 5
    done
    # sshd needs a moment after boot.
    sleep "${HOLLOW_PROXY_BOOT_WAIT:-20}"
    ;;
esac
exec "$g" compute start-iap-tunnel "$inst" 22 --listen-on-stdin --project="$project" --zone="$zone" --verbosity=warning
"##;

/// The VM's startup script (bash, as root, at every boot). The installs
/// run once; the preflight, the environment and the idle timer are written
/// every boot. `@PREFLIGHT@` is [`PREFLIGHT_SH`].
const STARTUP_SH: &str = r##"#!/bin/bash
# Hollow remote runner: setup for a Debian 12 VM (Settings, Remote runner,
# Set up, A Google Cloud machine). Runs as root at every boot.
set -u
# A failed download (curl | sh) fails its install instead of feeding sh nothing.
set -o pipefail
log() { echo "hollow-runner-setup: $*"; }
meta() { curl -fsS -H 'Metadata-Flavor: Google' "http://metadata.google.internal/computeMetadata/v1/instance/attributes/$1" 2>/dev/null; }
idle=$(meta hollow-idle-minutes)
case "$idle" in '' | *[!0-9]*) idle=30 ;; esac
# First, before any install: power off after $idle idle minutes (no SSH
# connection, no hollow-remote-run or hollow-slot process, and no install
# under way for less than 90 minutes), so a VM whose install fails or hangs
# still stops, and one mid-install doesn't.
echo "$idle" > /etc/hollow-idle-minutes
cat > /usr/local/sbin/hollow-idle-check <<'IDLE'
#!/bin/sh
n=$(cat /etc/hollow-idle-minutes 2>/dev/null)
case "$n" in '' | *[!0-9]*) n=30 ;; esac
f=/run/hollow-idle-count
busy=0
if [ "$(ss -Htn state established '( sport = :22 )' 2>/dev/null | wc -l)" -gt 0 ]; then busy=1; fi
if pgrep -f 'hollow-remote-run|hollow-slot' >/dev/null 2>&1; then busy=1; fi
if [ -n "$(find /run/hollow-installing -mmin -90 2>/dev/null)" ]; then busy=1; fi
if [ "$busy" = 1 ]; then echo 0 > "$f"; exit 0; fi
c=$(cat "$f" 2>/dev/null)
case "$c" in '' | *[!0-9]*) c=0 ;; esac
c=$((c + 1))
echo "$c" > "$f"
if [ "$c" -ge "$n" ]; then
  logger -t hollow-idle "idle for $c minutes; powering off"
  systemctl poweroff
fi
IDLE
chmod 755 /usr/local/sbin/hollow-idle-check
cat > /etc/systemd/system/hollow-idle.service <<'UNIT'
[Unit]
Description=Hollow runner: power off when idle

[Service]
Type=oneshot
ExecStart=/usr/local/sbin/hollow-idle-check
UNIT
cat > /etc/systemd/system/hollow-idle.timer <<'UNIT'
[Unit]
Description=Hollow runner: look for idle every minute

[Timer]
OnBootSec=5min
OnUnitActiveSec=1min

[Install]
WantedBy=timers.target
UNIT
systemctl daemon-reload
systemctl enable --now hollow-idle.timer
# What every login (ssh included, through pam_env) sees.
cat > /etc/environment <<'ENVIRONMENT'
PATH="/opt/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
RUSTUP_HOME="/opt/rustup"
PLAYWRIGHT_BROWSERS_PATH="/opt/ms-playwright"
ENVIRONMENT
cat > /usr/local/bin/hollow-run-preflight.new <<'PREFLIGHT'
@PREFLIGHT@PREFLIGHT
chmod 755 /usr/local/bin/hollow-run-preflight.new && mv -f /usr/local/bin/hollow-run-preflight.new /usr/local/bin/hollow-run-preflight
export DEBIAN_FRONTEND=noninteractive
mark=/var/lib/hollow-runner/installed-v1
install_tools() {
  log "installing packages"
  # A power cut mid-install leaves dpkg half done; finish it first.
  dpkg --configure -a || return 1
  apt-get update -q || return 1
  apt-get install -y -q git perl util-linux procps build-essential curl ca-certificates python3 pkg-config || return 1
  if ! command -v node >/dev/null 2>&1; then
    log "installing Node LTS"
    curl -fsSL https://deb.nodesource.com/setup_lts.x | bash - || return 1
    apt-get install -y -q nodejs || return 1
  fi
  if ! command -v uv >/dev/null 2>&1; then
    log "installing uv"
    curl -LsSf https://astral.sh/uv/install.sh | env UV_INSTALL_DIR=/usr/local/bin UV_NO_MODIFY_PATH=1 sh || return 1
  fi
  # One shared Rust for every login user, read-only for them.
  export RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo
  log "installing Rust @RUST@"
  # With rustup there already (an earlier boot stopped part way), install the toolchain again: it
  # is idempotent and repairs a toolchain missing a component.
  if [ -x /opt/cargo/bin/rustup ]; then
    /opt/cargo/bin/rustup toolchain install @RUST@ --profile minimal -c rustfmt -c clippy && /opt/cargo/bin/rustup default @RUST@ || return 1
  else
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path --profile minimal --default-toolchain @RUST@ -c rustfmt -c clippy || return 1
  fi
  chmod -R a+rX /opt/rustup /opt/cargo
  log "installing Playwright's Chromium and its libraries"
  export PLAYWRIGHT_BROWSERS_PATH=/opt/ms-playwright
  npx -y playwright@@PLAYWRIGHT@ install --with-deps chromium || return 1
  chmod -R a+rX /opt/ms-playwright
}
if [ ! -e "$mark" ]; then
  # The idle check counts this as busy (up to 90 minutes).
  touch /run/hollow-installing
  if install_tools; then
    mkdir -p /var/lib/hollow-runner && touch "$mark"
  else
    log "an install failed; the next boot tries again (the VM still powers itself off when idle)"
  fi
  rm -f /run/hollow-installing
fi
if [ -e "$mark" ]; then log "ready"; fi
"##;

/// The startup script Create hands the VM.
pub(crate) fn startup_script() -> String {
    STARTUP_SH
        .replace("@PREFLIGHT@", PREFLIGHT_SH)
        .replace("@RUST@", RUST_TOOLCHAIN)
        .replace("@PLAYWRIGHT@", PLAYWRIGHT_VERSION)
}

#[cfg(test)]
#[path = "runner_setup_tests.rs"]
mod tests;
