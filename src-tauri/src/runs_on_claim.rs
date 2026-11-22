//! Ownerless automations on unpaired Macs (follow-up to M4, 2026-11-17).
//! Two Macs that share a `workflows/` folder by sync but never paired each
//! think they hold the vault alone, so each fired an automation with no
//! `runs_on`. [`crate::runs_on::decide`] still lets such an automation fire
//! when no other Mac is known; this module then picks exactly one of the
//! Macs with no talk between them, through files the sync already carries.
//!
//! **Claims.** Each Mac writes only its own `workflows/claims/<slug>.<id>.json`
//! (`{id, since, beat}`, epoch seconds): `since` when it first claimed (or
//! claimed again after a silence over [`STALE_SECS`]), `beat` renewed every
//! [`BEAT_SECS`] while it passes. One writer per file, so sync never makes a
//! conflict copy.
//!
//! **The tiebreak** ([`turn`]). A claim counts once it has matured
//! ([`WINDOW_SECS`] after `since`, time for the other Mac's claim to arrive)
//! and while it is live (`beat` within [`STALE_SECS`]). A Mac fires only
//! with a matured claim of its own and no live matured claim with a lower
//! machine id. So with both up the lowest id fires; with one up (the other
//! never claimed, or went quiet) that one fires after its window; a Mac that
//! joins late waits out its window, and the incumbent keeps firing until the
//! newcomer's claim matures, then the lowest id holds. Decision: a window
//! shorter than the sync lag can still let both fire until the claims
//! arrive (once, then the lowest id holds); a Mac that sees no other live claim fires at once (alone
//! it never waits), so two Macs that start before either claim has synced both
//! fire until it arrives, then the lowest id holds. A Mac with no id, or one that can't
//! write its claim, fires as before.

use std::path::{Path, PathBuf};

/// Where claims live under the vault (a visible folder: sync skips dotted ones).
pub(crate) const DIR: &str = "workflows/claims";
/// A claim matures this long after it is made.
pub(crate) const WINDOW_SECS: i64 = 90;
/// A claim not renewed for this long is a Mac that is down.
pub(crate) const STALE_SECS: i64 = 30 * 60;
/// A live claim is renewed this often.
pub(crate) const BEAT_SECS: i64 = 5 * 60;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Claim {
    pub id: String,
    pub since: i64,
    pub beat: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Turn {
    /// This Mac fires it.
    Mine,
    /// Its own claim hasn't matured, or it has none yet.
    Waiting,
    /// Another Mac (this id) holds it.
    Theirs(String),
}

/// The tiebreak over `claims` for Mac `me` at `now` (module docs).
pub(crate) fn turn(claims: &[Claim], me: &str, now: i64) -> Turn {
    let Some(mine) = claims.iter().find(|c| c.id == me) else {
        return Turn::Waiting;
    };
    // No other Mac has a live claim: nothing to wait out (a Mac that holds
    // the vault alone fires at once, as before M4's follow-up).
    if !claims
        .iter()
        .any(|c| c.id != me && now - c.beat <= STALE_SECS)
    {
        return Turn::Mine;
    }
    if now - mine.since < WINDOW_SECS {
        return Turn::Waiting;
    }
    let rival = claims
        .iter()
        .filter(|c| c.id != me && now - c.beat <= STALE_SECS && now - c.since >= WINDOW_SECS)
        .map(|c| c.id.as_str())
        .min();
    match rival {
        Some(r) if r < me => Turn::Theirs(r.to_string()),
        _ => Turn::Mine,
    }
}

fn file_of(vault: &Path, slug: &str, id: &str) -> PathBuf {
    vault.join(DIR).join(format!("{slug}.{id}.json"))
}

/// Claims not renewed for this long are deleted ([`prune`]).
pub(crate) const PRUNE_SECS: i64 = 3 * 24 * 3600;

/// Every claim on `slug` in `vault` (unreadable files are skipped). A claim
/// counts only when its file is named for its own id (a Mac writes only its
/// own file), and its times are clamped to `now`, so a clock ahead of this
/// one can't hold a claim "live" for ever or mature it early.
pub(crate) fn read_claims(vault: &Path, slug: &str, now: i64) -> Vec<Claim> {
    let prefix = format!("{slug}.m-");
    let Ok(read) = std::fs::read_dir(vault.join(DIR)) else {
        return Vec::new();
    };
    read.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(&prefix) && n.ends_with(".json")
        })
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let c = serde_json::from_str::<Claim>(&std::fs::read_to_string(e.path()).ok()?).ok()?;
            (name == format!("{slug}.{}.json", c.id)).then_some(c)
        })
        .filter(|c| crate::phone_access::valid_machine_id(&c.id))
        .map(|mut c| {
            c.since = c.since.min(now);
            c.beat = c.beat.min(now);
            c
        })
        .collect()
}

/// Delete claim files that are not for one of `keep` (the ownerless,
/// non-manual automations that exist) or whose beat is over [`PRUNE_SECS`]
/// old. Returns how many went. A file that isn't a claim name is left alone.
pub(crate) fn prune(vault: &Path, keep: &std::collections::HashSet<String>, now: i64) -> usize {
    let Ok(read) = std::fs::read_dir(vault.join(DIR)) else {
        return 0;
    };
    let mut gone = 0;
    for e in read.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let Some(stem) = name.strip_suffix(".json") else {
            continue;
        };
        let Some((slug, id)) = stem.rsplit_once('.') else {
            continue;
        };
        if !crate::phone_access::valid_machine_id(id) {
            continue;
        }
        let stale = std::fs::read_to_string(e.path())
            .ok()
            .and_then(|t| serde_json::from_str::<Claim>(&t).ok())
            .is_none_or(|c| now - c.beat > PRUNE_SECS);
        if (!keep.contains(slug) || stale) && std::fs::remove_file(e.path()).is_ok() {
            gone += 1;
        }
    }
    gone
}

/// Renew or make this Mac's claim on `slug`, then say whose turn it is.
/// Fails open (`Mine`) when the claim can't be written.
pub(crate) fn turn_for(vault: &Path, slug: &str, me: &str, now: i64) -> Turn {
    let mut claims = read_claims(vault, slug, now);
    let fresh = Claim {
        id: me.to_string(),
        since: now,
        beat: now,
    };
    let write = match claims.iter_mut().find(|c| c.id == me) {
        None => {
            claims.push(fresh.clone());
            Some(fresh)
        }
        Some(c) if now - c.beat > STALE_SECS => {
            *c = fresh.clone();
            Some(fresh)
        }
        Some(c) if now - c.beat >= BEAT_SECS => {
            c.beat = now;
            Some(c.clone())
        }
        Some(_) => None,
    };
    if let Some(claim) = write {
        let path = file_of(vault, slug, me);
        let text = serde_json::to_string(&claim).unwrap_or_default();
        let done = path
            .parent()
            .map(std::fs::create_dir_all)
            .transpose()
            .map_err(|e| e.to_string())
            .and_then(|_| {
                crate::vault::atomic_write_private(&path, format!("{text}\n").as_bytes())
            });
        if let Err(e) = done {
            eprintln!("[runs_on] claim on {slug} not written ({e}); it fires as before");
            return Turn::Mine;
        }
    }
    turn(&claims, me, now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "m-00000000000000aa";
    const B: &str = "m-00000000000000bb";
    const T0: i64 = 1_800_000_000;

    /// Both ownerless, each reading the same folder; `a_first` picks who
    /// claims first. Returns (a fires, b fires) at `now`.
    fn both(v: &Path, a_first: bool, now: i64) -> (bool, bool) {
        let (x, y) = if a_first { (A, B) } else { (B, A) };
        for who in [x, y] {
            turn_for(v, "nightly", who, T0);
        }
        let a = turn_for(v, "nightly", A, now) == Turn::Mine;
        let b = turn_for(v, "nightly", B, now) == Turn::Mine;
        (a, b)
    }

    #[test]
    fn both_ownerless_fire_once_in_either_order() {
        for a_first in [true, false] {
            let v = tempfile::TempDir::new().unwrap();
            assert_eq!(both(v.path(), a_first, T0 + 10), (false, false), "windows");
            let v = tempfile::TempDir::new().unwrap();
            assert_eq!(
                both(v.path(), a_first, T0 + WINDOW_SECS),
                (true, false),
                "lowest id fires, order {a_first}"
            );
        }
    }

    #[test]
    fn one_offline_the_other_fires_after_its_window() {
        let v = tempfile::TempDir::new().unwrap();
        assert_eq!(
            turn_for(v.path(), "n", B, T0),
            Turn::Mine,
            "no rival: at once"
        );
        assert_eq!(turn_for(v.path(), "n", B, T0 + WINDOW_SECS), Turn::Mine);
        // The lower id claimed once, then went quiet: the higher takes over.
        let v = tempfile::TempDir::new().unwrap();
        turn_for(v.path(), "n", A, T0);
        turn_for(v.path(), "n", B, T0);
        assert_eq!(
            turn_for(v.path(), "n", B, T0 + WINDOW_SECS),
            Turn::Theirs(A.into())
        );
        // B keeps passing each minute; A never does again.
        let later = T0 + WINDOW_SECS + STALE_SECS + 1;
        let mut t = T0 + WINDOW_SECS;
        while t < later {
            t += 60;
            turn_for(v.path(), "n", B, t);
        }
        assert_eq!(turn_for(v.path(), "n", B, later), Turn::Mine);
    }

    #[test]
    fn a_late_lower_id_waits_while_the_incumbent_keeps_firing() {
        let v = tempfile::TempDir::new().unwrap();
        turn_for(v.path(), "n", B, T0);
        assert_eq!(turn_for(v.path(), "n", B, T0 + 100), Turn::Mine);
        let join = T0 + 200;
        assert_eq!(turn_for(v.path(), "n", A, join), Turn::Waiting);
        assert_eq!(
            turn_for(v.path(), "n", B, join + 30),
            Turn::Mine,
            "incumbent"
        );
        let matured = join + WINDOW_SECS;
        assert_eq!(turn_for(v.path(), "n", A, matured), Turn::Mine);
        assert_eq!(turn_for(v.path(), "n", B, matured), Turn::Theirs(A.into()));
    }

    #[test]
    fn a_silent_mac_that_returns_claims_afresh() {
        let v = tempfile::TempDir::new().unwrap();
        turn_for(v.path(), "n", A, T0);
        let back = T0 + STALE_SECS + 5;
        turn_for(v.path(), "n", A, back);
        assert_eq!(
            read_claims(v.path(), "n", back)[0].since,
            back,
            "new window"
        );
    }
}
