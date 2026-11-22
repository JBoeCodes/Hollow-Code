//! The review card's commands (CX20): `crate::context_review` owns the list
//! and the Accept; this file resolves the staging folder, the org queue, the
//! context clones (Settings, Sources, Context) and the record in app data,
//! and runs the work off the main thread.
//!
//! `context_review_list` only reads. Accept, Skip and Unskip are the
//! person's clicks on one item (user-only in the command map): Accept takes
//! exactly one id, and there is no batch form.

use super::AppState;
use crate::context_review::{
    self as review, ContextReviewAccepted, ContextReviewLayer, ContextReviewList, Ctx, Roots,
};
use crate::context_roots::{self, Layer};
use crate::session_import::staging::{self, Staging};
use tauri::State;

fn staging() -> Result<Staging, String> {
    staging::default_root()
        .map(Staging::new)
        .ok_or_else(|| "Hollow has no app support folder for staged scans.".to_string())
}

/// The card's items: scan `scan_id`'s proposals (else the newest scan's)
/// and the queued org facts, each with its proposed layer, targets and why
/// Accept may be off. Reads only.
#[tauri::command]
pub async fn context_review_list(scan_id: Option<String>) -> Result<ContextReviewList, String> {
    crate::vault::spawn_blocking_logged(move || {
        let manifest = review::manifest_for(&staging()?, scan_id.as_deref())?;
        let pending = crate::memory_homes::pending_org()?;
        let decisions = review::read_decisions(&review::decisions_path()?)?;
        let personal = context_roots::root(Layer::Personal);
        let org = context_roots::root(Layer::Org);
        let machine = context_roots::machine_slug();
        let roots = Roots {
            personal: personal.as_deref(),
            org: org.as_deref(),
            machine: machine.as_deref(),
        };
        Ok(review::list(
            manifest.as_ref(),
            &pending,
            &decisions,
            &roots,
        ))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Accept one item into one layer: Personal copies the file into the
/// personal clone (never over an existing file), Project writes nothing, Org
/// opens a pull request (CX8; the window confirms first). The person's click.
#[tauri::command]
pub async fn context_review_accept(
    state: State<'_, AppState>,
    id: String,
    layer: ContextReviewLayer,
    scan_id: Option<String>,
) -> Result<ContextReviewAccepted, String> {
    let vault = state.config.vault_path.clone();
    crate::vault::spawn_blocking_logged(move || {
        let manifest = review::manifest_for(&staging()?, scan_id.as_deref())?;
        let pending = crate::memory_homes::pending_org()?;
        let found = review::find(manifest.as_ref(), &pending, &id)?;
        let decisions = review::decisions_path()?;
        let pending_file = crate::memory_homes::org_pending_file()?;
        let personal = context_roots::root(Layer::Personal);
        let org = context_roots::root(Layer::Org);
        let machine = context_roots::machine_slug();
        let ctx = Ctx {
            roots: Roots {
                personal: personal.as_deref(),
                org: org.as_deref(),
                machine: machine.as_deref(),
            },
            decisions: &decisions,
            pending_file: &pending_file,
            vault: Some(&vault),
        };
        let ledger = crate::context_propose::ledger_path()?;
        let work = std::env::temp_dir().join(format!(
            "hollow-org-propose-{}",
            std::process::id()
        ));
        let propose = |req: &crate::context_propose::Request| {
            let org_root = org.as_deref().ok_or(
                "Connect the organization context in Settings, Sources, Context first. Nothing was proposed.",
            )?;
            let env = crate::context_propose::Env {
                runner: &crate::context_propose::SystemRunner,
                org_root,
                work_dir: &work,
                ledger: &ledger,
                sleep: &std::thread::sleep,
            };
            crate::context_propose::propose(&env, req)
        };
        review::accept(&ctx, &found, layer, &propose)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Skip one item: it stays listed as skipped until Unskip. Recorded in app
/// data, never the vault. The person's click.
#[tauri::command]
pub async fn context_review_skip(id: String) -> Result<bool, String> {
    crate::vault::spawn_blocking_logged(move || review::skip_at(&review::decisions_path()?, &id))
        .await
        .map_err(|e| e.to_string())?
}

/// Take back a Skip. The person's click.
#[tauri::command]
pub async fn context_review_unskip(id: String) -> Result<bool, String> {
    crate::vault::spawn_blocking_logged(move || review::unskip_at(&review::decisions_path()?, &id))
        .await
        .map_err(|e| e.to_string())?
}
