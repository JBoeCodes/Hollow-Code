//! Go back, the flow (updates design section 9; row U6): the client's
//! side of `update_go_back`, `update_go_back_when_idle` and
//! `update_unskip`, and their Tauri commands. The plan is `goback.rs`.
//!
//! **Order** (as Restart to update's, `client.rs`): the plan must allow it;
//! the previous copy must pass `codesign --verify`; the installed app is
//! cloned to `<installer>/go-back-helper/<id>/Hollow.app` and the clone
//! verified (signature, bundle id, this version, the same executable
//! bytes), because the helper must not run from either bundle it swaps;
//! then the work check (`confirmedKeys` as `setup_restart` takes them),
//! the claim (phase `goingBack`), the skip (`skippedVersion` = this
//! version), `last-update.json` (`kind: goBack`), and the detached
//! `hollow install-update --go-back [--restore-snapshot <folder>]` from the
//! clone. Just before the quit, running work and the helper get one more
//! look; either undoes the claim, the skip and the record.

use super::client::{Client, Started};
use super::goback::{self, PlanInputs, UpdateGoBackPlan, UpdateSchemaVerdict};
use super::install::{self, UpdateInstallOutcome, UpdateWork, WaitAction, WaitKind};
use super::last;
use super::state::{UpdatePhase, UpdateState};
use crate::update_install::effects::Fs as _;
use crate::update_install::{verify, Layout};
use serde::Serialize;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// `update_unskip` while a go back is starting.
pub(crate) const UNSKIP_GOING_BACK: &str =
    "Hollow is going back to the previous version; undo the skip after it reopens.";

/// The helper clones' folder, in the installer folder.
pub(crate) const HELPER_DIR: &str = "go-back-helper";

/// What `update_go_back` did. Running work is an answer, not an error, so
/// the window can offer "Go back when chats finish".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(tag = "outcome", rename_all = "camelCase")]
pub enum UpdateGoBackOutcome {
    /// The installer started; Hollow quits in a moment and reopens as
    /// `version`.
    Started {
        version: String,
    },
    WorkRunning {
        work: Vec<UpdateWork>,
    },
    /// The confirmed list no longer matches what runs: confirm again.
    WorkChanged {
        work: Vec<UpdateWork>,
    },
    /// Nothing happened; `reason` is for a person.
    Refused {
        reason: String,
    },
}

/// What the claim changed, so the last look can undo it.
#[derive(Clone)]
struct Claim {
    prior: UpdatePhase,
    prior_skip: Option<String>,
    restore: bool,
}

fn refused(reason: impl Into<String>) -> UpdateGoBackOutcome {
    UpdateGoBackOutcome::Refused {
        reason: reason.into(),
    }
}

/// Remove helper clones a finished go back left, unless an installer runs.
pub(crate) fn sweep_helper_copies(state_dir: &Path) {
    let live =
        crate::update_install::lock::holder(state_dir, &crate::update_install::effects::MacProcs)
            .is_some_and(|(_, live)| live);
    if !live {
        let _ = std::fs::remove_dir_all(state_dir.join(HELPER_DIR));
    }
}

/// The helper's command line (after `install-update`).
pub(crate) fn go_back_args(
    app: &Path,
    state_dir: &Path,
    install_id: &str,
    restore: Option<&Path>,
) -> Vec<OsString> {
    let mut out: Vec<OsString> = vec!["--go-back".into()];
    let mut flag = |name: &str, value: &Path| {
        out.push(name.into());
        out.push(value.as_os_str().to_owned());
    };
    flag("--app", app);
    flag("--journal-dir", state_dir);
    if let Some(folder) = restore {
        flag("--restore-snapshot", folder);
    }
    out.push("--install-id".into());
    out.push(install_id.into());
    out.push("--relaunch".into());
    out
}

impl Client {
    /// `update_go_back_plan`. Reads only (a `quick_check` of each snapshot
    /// copy, cached while the copy is unchanged: `goback::cached_check`).
    pub(crate) fn go_back_plan(&self) -> UpdateGoBackPlan {
        let state_dir = self.d.paths.state_dir.clone();
        let lock_live = state_dir.as_deref().is_some_and(|dir| {
            crate::update_install::lock::holder(dir, &crate::update_install::effects::MacProcs)
                .is_some_and(|(_, live)| live)
        });
        let support = self
            .d
            .paths
            .sessions
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        goback::plan(&PlanInputs {
            running_version: &self.d.config.running_version,
            installed_app: self.d.paths.installed_app.as_deref(),
            state_dir: state_dir.as_deref(),
            support: support.as_deref(),
            review_copy: self.d.config.review_copy,
            busy: self.lock().phase.busy(),
            lock_live,
            skipped_version: self.settings.read().skipped_version,
            waiting: self.waiting_for(Some(WaitKind::GoBack)),
        })
    }

    /// The plan, or why `restore` can't be done with it.
    fn checked_plan(&self, restore: bool) -> Result<UpdateGoBackPlan, String> {
        let plan = self.go_back_plan();
        if !plan.available {
            return Err(plan
                .disabled_reason
                .unwrap_or_else(|| goback::NO_PREVIOUS.into()));
        }
        if restore && !plan.restore_available {
            return Err(
                "There's no intact copy of your chats from before the update to put back.".into(),
            );
        }
        if !restore && matches!(plan.schema, Some(UpdateSchemaVerdict::NeedsRestore { .. })) {
            let previous = plan
                .previous
                .as_ref()
                .map(|p| p.version.as_str())
                .unwrap_or("?");
            return Err(format!(
                "Hollow {previous} can't safely read the chats this version saved. Choose Go back and restore data from before the update."
            ));
        }
        Ok(plan)
    }

    /// "Go back to the previous version" (`restore`: "Go back and restore
    /// data from before the update"). `confirmed`: the running work the
    /// restart dialog listed, as `update_install_now` takes it.
    pub(crate) fn go_back(
        self: &Arc<Self>,
        restore: bool,
        confirmed: Option<&[String]>,
    ) -> UpdateGoBackOutcome {
        self.go_back_inner(restore, confirmed, false)
    }

    /// From "Go back when chats finish": `Ok(true)` started, `Ok(false)`
    /// work came back (keep waiting), `Err` a refusal ends the wait.
    pub(crate) fn go_back_from_wait(self: &Arc<Self>, restore: bool) -> Result<bool, String> {
        match self.go_back_inner(restore, None, true) {
            UpdateGoBackOutcome::Started { .. } => Ok(true),
            UpdateGoBackOutcome::WorkRunning { .. } | UpdateGoBackOutcome::WorkChanged { .. } => {
                Ok(false)
            }
            UpdateGoBackOutcome::Refused { reason } => {
                self.lock().note = Some(format!("Hollow didn't go back: {reason}"));
                Err(reason)
            }
        }
    }

    fn go_back_inner(
        self: &Arc<Self>,
        restore: bool,
        confirmed: Option<&[String]>,
        from_wait: bool,
    ) -> UpdateGoBackOutcome {
        let plan = match self.checked_plan(restore) {
            Ok(plan) => plan,
            Err(why) => return refused(why),
        };
        let Some(previous) = plan.previous.clone() else {
            return refused(goback::NO_PREVIOUS);
        };
        let (Some(app), Some(state_dir)) = (
            self.d.paths.installed_app.clone(),
            self.d.paths.state_dir.clone(),
        ) else {
            return refused(goback::NOT_FROM_APPLICATIONS);
        };
        let layout = Layout::for_app(&app);
        // The previous copy's signature (the helper checks it again).
        if let Err(why) = verify::check_bundle(&layout.previous, self.d.signer.as_ref(), None, None)
        {
            return refused(format!(
                "The previous copy of Hollow doesn't pass its signature check, so it can't be put back: {why}"
            ));
        }
        let install_id = crate::update_install::run::new_install_id(self.d.clock.now_ms());
        let helper = match self.clone_helper(&app, &state_dir, &install_id) {
            Ok(exe) => exe,
            Err(why) => return refused(why),
        };
        let drop_helper = || {
            let _ = std::fs::remove_dir_all(state_dir.join(HELPER_DIR).join(&install_id));
        };
        match self.work_in_the_way(confirmed) {
            Ok(None) => {}
            Ok(Some(UpdateInstallOutcome::WorkRunning { work })) => {
                drop_helper();
                return UpdateGoBackOutcome::WorkRunning { work };
            }
            Ok(Some(UpdateInstallOutcome::WorkChanged { work })) => {
                drop_helper();
                return UpdateGoBackOutcome::WorkChanged { work };
            }
            Ok(Some(UpdateInstallOutcome::Started { .. })) => {}
            Err(why) => {
                drop_helper();
                return refused(why);
            }
        }
        // The claim: nothing else starts from here.
        let prior = {
            let mut inner = self.lock();
            if inner.phase.busy() {
                drop(inner);
                drop_helper();
                return refused(goback::BUSY);
            }
            let prior = inner.phase.clone();
            inner.phase = UpdatePhase::GoingBack {
                version: previous.version.clone(),
            };
            prior
        };
        let prior_skip = self.settings.read().skipped_version;
        let claim = Claim {
            prior,
            prior_skip,
            restore,
        };
        let running = self.d.config.running_version.clone();
        let started = self
            .settings
            .update(|s| s.skipped_version = Some(running.clone()))
            .and_then(|_| {
                last::write(
                    &self.d.paths.last,
                    &last::Started {
                        schema: 1,
                        install_id: install_id.clone(),
                        from_version: running.clone(),
                        to_version: previous.version.clone(),
                        notes: String::new(),
                        channel: self.settings.read().channel,
                        at: self.now(),
                        pid: self.d.own_pid,
                        build_id: None,
                        acknowledged: false,
                        kind: last::StartedKind::GoBack,
                        restore_data: restore,
                        skip_settled: false,
                        overnight: false,
                        overnight_notified: false,
                        local: None,
                    },
                )
            })
            .and_then(|()| {
                let snapshot = match (&plan.snapshot, restore) {
                    (Some(_), true) => self.snapshot_folder(&state_dir, &previous.version),
                    _ => None,
                };
                if restore && snapshot.is_none() {
                    return Err("The snapshot from before the update is gone.".to_string());
                }
                let args = go_back_args(&app, &state_dir, &install_id, snapshot.as_deref());
                self.d
                    .helper
                    .spawn(&helper, &args, &state_dir.join(install::LOG))
                    .and_then(|mut started| match started.exited_early() {
                        Some(why) => Err(why),
                        None => Ok(started),
                    })
            });
        match started {
            Ok(started) => {
                let me = self.clone();
                let keys = confirmed.map(<[String]>::to_vec);
                self.emit();
                self.d.helper.quit(Box::new(move || {
                    me.go_back_last_look(started, keys.as_deref(), claim, from_wait)
                }));
                UpdateGoBackOutcome::Started {
                    version: previous.version,
                }
            }
            Err(why) => {
                self.undo_claim(&claim, Some(why.clone()));
                drop_helper();
                refused(why)
            }
        }
    }

    /// The snapshot folder of the running version's install from `previous`.
    fn snapshot_folder(&self, state_dir: &Path, previous: &str) -> Option<PathBuf> {
        let (journals, _) = crate::update_install::journal::list(state_dir);
        goback::snapshot_for(&journals, &self.d.config.running_version, previous)
            .and_then(|(s, _)| s.folder)
    }

    /// Clone the installed app for the helper and check the clone is this
    /// app: signature, bundle id, version and executable bytes.
    fn clone_helper(&self, app: &Path, state_dir: &Path, id: &str) -> Result<PathBuf, String> {
        let fail = |why: String| format!("Hollow couldn't prepare to go back: {why}");
        sweep_helper_copies(state_dir);
        let dir = state_dir.join(HELPER_DIR).join(id);
        crate::update_install::lock::create_private_dir(&dir)
            .map_err(|e| fail(format!("{}: {e}", dir.display())))?;
        let clone = dir.join(app.file_name().unwrap_or_else(|| "Hollow.app".as_ref()));
        crate::update_install::effects::RealFs
            .copy_bundle(app, &clone)
            .map_err(fail)?;
        let checked = verify::check_bundle(
            &clone,
            self.d.signer.as_ref(),
            None,
            Some(&self.d.config.running_version),
        )
        .and_then(|info| {
            let exe = |b: &Path| b.join("Contents").join("MacOS").join(&info.executable);
            match verify::same_bytes(&exe(app), &exe(&clone))? {
                true => Ok(exe(&clone)),
                false => Err("the copy's executable differs from the installed one".into()),
            }
        });
        checked.map_err(|why| {
            let _ = std::fs::remove_dir_all(&dir);
            fail(format!("the copy of this app didn't verify ({why})"))
        })
    }

    /// Put back what the claim changed: the phase, the skip, the record.
    fn undo_claim(&self, claim: &Claim, note: Option<String>) {
        let running = self.d.config.running_version.clone();
        let _ = self.settings.update(|s| {
            if s.skipped_version.as_deref() == Some(running.as_str()) {
                s.skipped_version = claim.prior_skip.clone();
            }
        });
        let _ = std::fs::remove_file(&self.d.paths.last);
        {
            let mut inner = self.lock();
            if matches!(inner.phase, UpdatePhase::GoingBack { .. }) {
                inner.phase = claim.prior.clone();
            }
            if note.is_some() {
                inner.note = note;
            }
        }
        self.emit();
    }

    /// Just before the quit (as `last_look` for an update). `true` quits.
    fn go_back_last_look(
        self: &Arc<Self>,
        mut started: Box<dyn Started>,
        confirmed: Option<&[String]>,
        claim: Claim,
        from_wait: bool,
    ) -> bool {
        let in_the_way = match self.work_in_the_way(confirmed) {
            Ok(None) | Ok(Some(UpdateInstallOutcome::Started { .. })) => None,
            Ok(Some(
                UpdateInstallOutcome::WorkRunning { work }
                | UpdateInstallOutcome::WorkChanged { work },
            )) => Some(
                work.into_iter()
                    .map(|w| w.name)
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Err(why) => Some(why),
        };
        let note = match in_the_way {
            Some(what) => {
                started.stop();
                format!("Hollow didn't go back: {what} started meanwhile.")
            }
            None => match started.exited() {
                Some(why) => why,
                None => return true,
            },
        };
        self.undo_claim(&claim, Some(note));
        if from_wait {
            self.clear_wait_slot();
            let _ = self.go_back_when_idle(claim.restore, install::POLL);
        }
        false
    }

    /// "Go back when chats finish": no expiry, in memory only.
    pub(crate) fn go_back_when_idle(
        self: &Arc<Self>,
        restore: bool,
        interval: std::time::Duration,
    ) -> Result<UpdateState, String> {
        self.checked_plan(restore)?;
        self.wait_until_idle(
            WaitAction::GoBack {
                restore_data: restore,
            },
            interval,
            None,
        )
    }

    /// "Skipping 0.6.n" → Undo: the version is offered again (a check
    /// follows).
    pub(crate) fn unskip(&self) -> Result<UpdateState, String> {
        // A go back sets the skip as it starts; undoing it then would let
        // the version it leaves be offered straight back.
        if matches!(self.lock().phase, UpdatePhase::GoingBack { .. }) {
            return Err(UNSKIP_GOING_BACK.into());
        }
        self.settings.update(|s| s.skipped_version = None)?;
        self.emit();
        Ok(self.state())
    }

    /// At launch, after the notice is read: an automatic go back skips the
    /// version that failed to start; a go back that didn't happen is no
    /// longer a skip. Once per record, whether or not the window already
    /// acknowledged the notice (`last::for_skip`).
    pub(crate) fn settle_go_back_skip(&self) {
        let path = &self.d.paths.last;
        let result = last::for_skip(
            path,
            self.d.paths.state_dir.as_deref(),
            &self.d.config.running_version,
            Some(&crate::app_menu::build_identity()),
            self.d.own_pid,
        );
        if result.is_some() {
            let _ = last::mark_skip_settled(path);
        }
        match result {
            Some(last::UpdateLastResult::WentBack {
                attempted,
                manual: false,
                ..
            }) => {
                let _ = self
                    .settings
                    .update(|s| s.skipped_version = Some(attempted.clone()));
            }
            Some(last::UpdateLastResult::NotWentBack { .. }) => {
                let running = self.d.config.running_version.clone();
                let _ = self.settings.update(|s| {
                    if s.skipped_version.as_deref() == Some(running.as_str()) {
                        s.skipped_version = None;
                    }
                });
            }
            _ => {}
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────

/// Settings, Updates' Go back card.
#[tauri::command]
pub async fn update_go_back_plan() -> Result<UpdateGoBackPlan, String> {
    super::ipc::on_client(|c| Ok(c.go_back_plan())).await
}

/// "Go back to the previous version" (`restoreData: false`) or "Go back and
/// restore data from before the update" (`true`).
#[tauri::command]
pub async fn update_go_back(
    restore_data: bool,
    confirmed_keys: Option<Vec<String>>,
) -> Result<UpdateGoBackOutcome, String> {
    super::ipc::on_client(move |c| Ok(c.go_back(restore_data, confirmed_keys.as_deref()))).await
}

/// "Go back when chats finish": waits with no expiry, in memory;
/// `update_cancel_wait` cancels it.
#[tauri::command]
pub async fn update_go_back_when_idle(restore_data: bool) -> Result<UpdateState, String> {
    super::ipc::on_client(move |c| c.go_back_when_idle(restore_data, install::POLL)).await
}

/// "Skipping 0.6.n" → Undo, then a check (the version is offered again).
#[tauri::command]
pub async fn update_unskip() -> Result<UpdateState, String> {
    super::ipc::on_client(|c| {
        let state = c.unskip()?;
        super::kick(c);
        Ok(state)
    })
    .await
}
