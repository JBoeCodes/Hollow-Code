//! The updater client (updates design 2026-11-14, sections 2, 6 to 9; lane U
//! row U5, PR 1: everything but the UI, which is PR 2).
//!
//! Checks the person's channel (`main` or `nightly`) on the update host
//! about 45 seconds after launch and every 30 minutes, or on Check now;
//! downloads an offered build in the background with `tauri-plugin-updater`
//! (its `check` and `download` only: the minisign signature check), unpacks
//! it into a staging folder under app data, verifies the bundle with the
//! installer's own checks, and only then reports `ready` (the top bar's
//! Update button appears only then). "Restart to update" hands the staged
//! bundle to `hollow install-update` (`update_install`) under the restart
//! checks; "Update when chats finish" waits with no expiry. No agent action
//! installs a channel update (design section 7).
//!
//! | File | What |
//! | --- | --- |
//! | `state.rs` | phases, the state the window gets, the offer rule |
//! | `config.rs` | host, endpoint, public key, team; inert reasons |
//! | `settings.rs` | `update-settings.json` (channel, overnight, skipped, Not now) |
//! | `token.rs` | the token's shape and its Keychain store |
//! | `manifest.rs` | the Worker's manifest, checked |
//! | `fetch.rs` | check and download through the plugin |
//! | `unpack.rs`, `stage.rs` | safe unpack, the staging folder |
//! | `install.rs` | installer arguments, the no-expiry wait |
//! | `last.rs` | the notice after the restart |
//! | `goback.rs` | Go back's plan: previous copy, schema verdict (row U6) |
//! | `goback_run.rs` | Go back's flow and commands, the skip's Undo (row U6) |
//! | `local.rs` | Local builds from a build workspace (row U10) |
//! | `overnight.rs` | row U7: the overnight installer and its morning notification |
//! | `client.rs` | the client itself |
//! | `link.rs` | `hollow://updates/connect?token=…` |
//! | `ipc.rs` | the Tauri commands |
//! | `publish.rs` | Publish to Main and Withdraw, the run history, the nudge (row U8) |
//! | `people.rs` | row U9: `/me`, allowed channels, the fallback |
//! | `people_admin.rs` | row U9: People on the publisher's Mac (add, revoke, channels) |
//!
//! Commands, events and settings: `docs/UPDATES.md`, "The updater client".

pub(crate) mod client;
pub(crate) mod config;
pub(crate) mod fetch;
pub(crate) mod goback;
pub(crate) mod goback_run;
pub(crate) mod install;
pub(crate) mod ipc;
pub(crate) mod last;
pub(crate) mod link;
pub(crate) mod local;
pub(crate) mod manifest;
pub(crate) mod overnight;
pub(crate) mod people;
pub(crate) mod people_admin;
pub(crate) mod publish;
pub(crate) mod settings;
pub(crate) mod stage;
pub(crate) mod state;
pub(crate) mod token;
pub(crate) mod unpack;

#[cfg(test)]
mod client_tests;
#[cfg(test)]
mod fixture_tests;
#[cfg(test)]
mod goback_store_tests;
#[cfg(test)]
mod goback_tests;
#[cfg(test)]
mod local_tests;
#[cfg(test)]
mod overnight_tests;
#[cfg(test)]
mod people_tests;
#[cfg(test)]
mod publish_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod unpack_tests;

use client::{Client, Deps, Helper, Paths, Sink, Started, Work};
use install::UpdateWork;
use state::UpdateState;
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use tauri::{Emitter, Manager};

static CLIENT: OnceLock<Arc<Client>> = OnceLock::new();

/// The app's client, once [`start`] ran.
pub(crate) fn client() -> Option<Arc<Client>> {
    CLIENT.get().cloned()
}

/// Check now (Check now, a connect, a channel switch), on the schedule
/// thread. Returns at once; the state events follow.
pub(crate) fn kick(client: &Arc<Client>) {
    // The schedule thread checks at once (and restarts its wait, so a
    // reconnect after a long backoff isn't left waiting).
    client.wake_schedule();
}

/// Offer workspace `id`'s finished build in the Update button as "Local
/// build of <branch>" (row U10): when the build finishes, at launch for a
/// build saved across the restart, on the person's Show in Update button in
/// Changes (`asked`), and for an agent's install the person allowed
/// (`approved`, the receipt its card bound; also `asked`). The bytes are
/// checked against the receipt each time.
pub(crate) fn offer_local_build_with(
    vault: &Path,
    id: &str,
    approved: Option<&crate::self_install::BuildReceipt>,
    asked: bool,
) -> Result<UpdateState, String> {
    let client = client().ok_or("Hollow's updater isn't running in this window.")?;
    let ready = match approved {
        Some(receipt) => crate::self_install::check_approved(id, receipt)?,
        None => crate::self_install::finished_build(id)?,
    };
    let notes = match crate::install_brings::for_build(vault, id) {
        Ok(report) => local::notes(&report),
        Err(why) => format!("What this build brings couldn't be listed: {why}"),
    };
    client.offer_local(local::LocalOffer {
        receipt: ready.receipt,
        branch: ready.branch,
        notes,
        asked,
    })
}

/// [`offer_local_build_with`] for the build's own receipt.
pub(crate) fn offer_local_build(
    vault: &Path,
    id: &str,
    asked: bool,
) -> Result<UpdateState, String> {
    offer_local_build_with(vault, id, None, asked)
}

/// Whether a Local build's install is starting: `self_install::start_build`
/// refuses a new app build meanwhile (its withdraw would be a no-op and its
/// cleanup would race the install).
pub(crate) fn installing_local_build() -> bool {
    client().is_some_and(|c| c.installing_local())
}

/// Take a Local build out of the Update button (a new build started).
pub(crate) fn withdraw_local_build(build_id: Option<&str>) {
    if let Some(client) = client() {
        client.withdraw_local(build_id);
    }
}

/// Build the client (always, so the commands answer) and, unless this is
/// a review copy, start its schedule. Called once from `lib.rs` setup.
pub(crate) fn start(app: &tauri::AppHandle) {
    let review_copy = !crate::instance::may_change_shared_setup();
    let data = crate::agent_settings::app_data_dir().ok();
    let updates = data
        .clone()
        .unwrap_or_else(std::env::temp_dir)
        .join("updates");
    let plugins = serde_json::Value::Object(
        app.config()
            .plugins
            .0
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
    );
    let pubkey = config::pubkey_from_plugins(&plugins);
    let deps = Deps {
        config: config::Config {
            host: config::host_from(config::host_override(), !cfg!(debug_assertions)),
            pubkey: pubkey.clone(),
            team: config::team_from(config::team_override()),
            running_version: crate::app_menu::build_identity().version,
            review_copy: review_copy || data.is_none(),
        },
        paths: Paths {
            settings: data.unwrap_or_else(std::env::temp_dir).join(settings::FILE),
            staging: updates.join("staging"),
            last: updates.join(last::FILE),
            installed_app: std::env::current_exe()
                .ok()
                .and_then(|exe| install::bundle_of(&exe)),
            state_dir: crate::update_install::default_state_dir(),
            sessions: crate::update_install::default_sessions_dir(),
        },
        fetcher: Box::new(fetch::PluginFetcher::new(app.clone(), pubkey)),
        tokens: Box::new(token::Keychain),
        signer: Box::new(crate::update_install::effects::Codesign),
        work: Box::new(AppWork { app: app.clone() }),
        helper: Box::new(AppHelper { app: app.clone() }),
        clock: Box::new(crate::update_install::effects::SystemClock),
        sink: Box::new(AppSink { app: app.clone() }),
        limits: unpack::Limits::default(),
        own_pid: std::process::id(),
        naps: client::Naps::Interruptible,
    };
    let client = Arc::new(Client::new(deps));
    if CLIENT.set(client.clone()).is_err() {
        return;
    }
    if review_copy {
        return;
    }
    publish::start(app.clone(), client.clone());
    let app = app.clone();
    let _ = std::thread::Builder::new()
        .name("hollow-updates".into())
        .spawn(move || {
            // The notice after an update, for a window already listening
            // (the window also asks `update_last_result` when it loads).
            if let Some(result) = client.last_result() {
                // A Local build's install goes on its build receipt too.
                if let Ok(dir) = crate::self_install::state_dir() {
                    if let Err(why) = client.settle_local_receipt(&dir) {
                        eprintln!("[updates] the local build's receipt wasn't written: {why}");
                    }
                }
                let _ = app.emit(client::LAST_RESULT_EVENT, result);
            }
            // Row U7: one Hollow notification after an overnight install.
            if let (Some(notice), Ok(vault)) = (
                client.overnight_notice(),
                (AppWork { app: app.clone() }).vault(),
            ) {
                let posted = crate::notifications::create_registered(
                    &vault,
                    crate::notifications::NotificationInput {
                        title: notice.title,
                        body: notice.body,
                        kind: notice.kind.into(),
                        source: Some("updates.overnight".into()),
                        action_label: Some("Updates".into()),
                        action_url: Some("settings:updates".into()),
                        notify_native: true,
                    },
                );
                if let Some(Err(why)) = posted {
                    eprintln!("[updates] the overnight update's notification failed: {why}");
                }
            }
            // Row U6: the skip after a go back, what this build knows (so
            // a newer build can tell whether going back here is safe), and
            // a finished go back's helper copy.
            client.settle_go_back_skip();
            if let Some(dir) = crate::update_install::default_state_dir() {
                goback::record_known_schema(&dir, &crate::app_menu::build_identity().version);
                goback_run::sweep_helper_copies(&dir);
            }
            // The first Keychain read, here rather than on the main thread.
            client.load_connection();
            // A finished build saved across the restart waits in the button
            // again, unless the person hid it with Not now.
            if let (Some(id), Ok(vault)) = (
                crate::self_install::finished_workspace(),
                (AppWork { app: app.clone() }).vault(),
            ) {
                if let Err(why) = offer_local_build(&vault, &id, false) {
                    eprintln!("[updates] the saved build isn't in the Update button: {why}");
                }
            }
            // A Connect Hollow link that arrived before the client existed.
            if let Some(token) = link::take_early() {
                let _ = client.connect_from_link(token);
            }
            // Row U7: the overnight installer, on its own thread (it reads
            // the setting at every look, so it always runs).
            let night = client.clone();
            let _ = std::thread::Builder::new()
                .name("hollow-updates-overnight".into())
                .spawn(move || {
                    night.run_overnight(&overnight::Mac, overnight::EVERY, None);
                });
            client.run_schedule(client::LAUNCH_DELAY, client::EVERY, None);
        });
}

struct AppSink {
    app: tauri::AppHandle,
}

impl Sink for AppSink {
    fn state(&self, state: &UpdateState) {
        let _ = self.app.emit(client::EVENT, state);
    }
}

/// The restart checks, against the open vault.
struct AppWork {
    app: tauri::AppHandle,
}

impl AppWork {
    fn vault(&self) -> Result<std::path::PathBuf, String> {
        self.app
            .try_state::<crate::commands::AppState>()
            .map(|s| s.config.vault_path.clone())
            .ok_or_else(|| "Hollow's state isn't loaded yet.".to_string())
    }
}

impl Work for AppWork {
    fn running(&self) -> Result<Vec<UpdateWork>, String> {
        Ok(crate::lifecycle::running_work_now(&self.vault()?)?
            .into_iter()
            .map(|(key, name)| UpdateWork { key, name })
            .collect())
    }

    fn confirm(&self, keys: &[String]) -> Result<(), String> {
        crate::lifecycle::check_restart_work(&self.vault()?, keys)
    }

    fn blocked(&self) -> Option<String> {
        match self.vault() {
            Ok(vault) => crate::self_install::install_blocked(&vault),
            Err(why) => Some(why),
        }
    }
}

/// The detached installer, then the same quit the build install uses.
struct AppHelper {
    app: tauri::AppHandle,
}

impl Helper for AppHelper {
    fn spawn(&self, exe: &Path, args: &[OsString], log: &Path) -> Result<Box<dyn Started>, String> {
        let child = crate::update_install::detach::spawn_detached(exe, args, log)?;
        Ok(Box::new(Installer { child }))
    }

    fn quit(&self, last_look: Box<dyn FnOnce() -> bool + Send>) {
        // Let the command's answer and the last state event reach the
        // window first, then look at running work once more, off the
        // runtime's threads.
        let app = self.app.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
            let go = tauri::async_runtime::spawn_blocking(last_look)
                .await
                .unwrap_or(false);
            if go {
                crate::lifecycle::expect_exit();
                app.exit(0);
            }
        });
    }
}

struct Installer {
    child: std::process::Child,
}

impl Started for Installer {
    fn exited_early(&mut self) -> Option<String> {
        for _ in 0..10 {
            if let Ok(Some(status)) = self.child.try_wait() {
                return Some(format!(
                    "The installer stopped as soon as it started ({status}); its log is {} in Hollow's installer folder.",
                    install::LOG
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }

    fn exited(&mut self) -> Option<String> {
        match self.child.try_wait() {
            Ok(Some(status)) => Some(format!(
                "Hollow didn't restart: the installer stopped before Hollow quit ({status}). The update is still ready."
            )),
            _ => None,
        }
    }

    fn stop(&mut self) {
        let pid = self.child.id() as i32;
        if pid > 1 {
            // SAFETY: a plain signal to our own child.
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
        }
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}
