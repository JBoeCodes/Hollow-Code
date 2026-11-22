//! Stale memory and home files in recall (ME2, 2026-11-18).
//!
//! One staleness definition, PX4's fields read by PX7's rules:
//! - a memory is **expired** past its `valid_until`, and **unverified** when
//!   its `verified_at` is older than `context_layers::MEMORY_STALE_DAYS`
//!   (180) and it is not expired ([`memory_staleness`]; the capability
//!   doctor's `context_layers::health_in` counts with this same function);
//! - the home file is judged by those two fields when its frontmatter
//!   carries a readable one (they decide alone), and only otherwise by its
//!   last `Last checked:` date against [`HOME_STALE_DAYS`]
//!   ([`home_staleness`]; `health_in` reports its label as `home_stale`,
//!   which the doctor shows, so the doctor has no rule of its own).
//!
//! What recall does with it. The personal clone's memories and home file
//! reach an agent through the profile block's sections
//! (`context_layers::sections`), not through the retrieval broker, so that is
//! where a stale file is demoted: in the generated index a stale memory moves
//! out of its type group into a last group, [`RECHECK_GROUP`], with its
//! [`Staleness::label`] on the line (ranked behind every fresh file and the
//! first line cut when the index overflows, never removed); the home
//! section's lead line says the file is stale.
//!
//! Waiting on you. Each stale file is offered once as a review card of kind
//! [`REVIEW_KIND`] ("Recheck memory/x.md"), source `memory`. The fingerprint
//! is the file and the date that made it stale ([`fingerprint`]), so a
//! repeated recall finds the card the queue already holds (the queue dedupes
//! on kind and fingerprint, terminal cards included) and adds nothing, while
//! a file rechecked, re-dated and later stale again gets a new card. An
//! in-process set skips even the queue read after the first offer. The
//! offer runs on its own thread, never on a turn's path, and at most
//! [`OPEN_CAP`] cards wait at once (the rest are offered as those close).
//! Rechecked (accept) and Dismiss (reject) only close the card: Hollow never
//! edits the memory file; the person updates its date.

use crate::context_layers::{date_of, last_checked, MEMORY_STALE_DAYS};
use crate::review_queue::{self, NewReviewItem, ReviewItem};
use chrono::NaiveDate;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Days after which an undated home file's `Last checked:` date reads as
/// stale (PX7's 30). The only copy: the doctor shows `health_in`'s label.
pub(crate) const HOME_STALE_DAYS: i64 = 30;
/// The review kind of a recheck card.
pub(crate) const REVIEW_KIND: &str = "memory_recheck";
/// The personal index's last group: stale memories, labelled.
pub(crate) const RECHECK_GROUP: &str = "Needs recheck (stale, listed last)";

/// Why a file reads as stale, with the date that says so.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Staleness {
    /// Past its `valid_until`.
    Expired(NaiveDate),
    /// `verified_at` older than [`MEMORY_STALE_DAYS`].
    Unverified(NaiveDate),
    /// The home file's `Last checked:` older than [`HOME_STALE_DAYS`].
    Unchecked(NaiveDate),
}

impl Staleness {
    /// The label recall puts on the file's line.
    pub(crate) fn label(&self) -> String {
        match self {
            Staleness::Expired(d) => format!("stale: past valid_until {d}"),
            Staleness::Unverified(d) => format!("stale: last verified {d}"),
            Staleness::Unchecked(d) => format!("stale: last checked {d}"),
        }
    }

    /// The fingerprint's half that changes when the file is re-dated.
    fn marker(&self) -> String {
        match self {
            Staleness::Expired(d) => format!("expired:{d}"),
            Staleness::Unverified(d) => format!("verified:{d}"),
            Staleness::Unchecked(d) => format!("checked:{d}"),
        }
    }
}

/// A stale file of the personal clone, by its path in the clone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleFile {
    pub rel: String,
    pub staleness: Staleness,
}

/// A memory file's staleness as of `today` (PX4 fields, PX7 rules); `None`
/// when it is fresh or carries no readable date.
pub(crate) fn memory_staleness(text: &str, today: NaiveDate) -> Option<Staleness> {
    let (verified, until) = crate::memory_guard::memory_dates(text);
    if let Some(until) = until.as_deref().and_then(date_of) {
        if until < today {
            return Some(Staleness::Expired(until));
        }
    }
    let verified = verified.as_deref().and_then(date_of)?;
    ((today - verified).num_days() > MEMORY_STALE_DAYS).then_some(Staleness::Unverified(verified))
}

/// The home file's staleness: when its frontmatter carries a readable
/// `verified_at` or `valid_until`, those decide alone ([`memory_staleness`]);
/// only a file with neither is judged by its `Last checked:` line against
/// [`HOME_STALE_DAYS`].
pub(crate) fn home_staleness(text: &str, today: NaiveDate) -> Option<Staleness> {
    let (verified, until) = crate::memory_guard::memory_dates(text);
    let dated = [verified, until]
        .iter()
        .any(|d| d.as_deref().and_then(date_of).is_some());
    if dated {
        return memory_staleness(text, today);
    }
    let checked = last_checked(text)?;
    ((today - checked).num_days() > HOME_STALE_DAYS).then_some(Staleness::Unchecked(checked))
}

/// The recheck card's fingerprint: the file and the date that made it stale.
pub(crate) fn fingerprint(file: &StaleFile) -> String {
    format!("{}@{}", file.rel, file.staleness.marker()).to_lowercase()
}

/// The recheck card for `file` in the clone at `root`.
pub(crate) fn recheck_item(root: &Path, file: &StaleFile) -> NewReviewItem {
    let full = root.join(&file.rel);
    let what = match file.staleness {
        Staleness::Expired(d) => format!("passed its valid_until date ({d})"),
        Staleness::Unverified(d) => {
            format!("was last verified on {d}, over {MEMORY_STALE_DAYS} days ago")
        }
        Staleness::Unchecked(d) => {
            format!("was last checked on {d}, over {HOME_STALE_DAYS} days ago")
        }
    };
    let fix = match file.staleness {
        Staleness::Unchecked(_) => "update its Last checked line",
        _ => "set verified_at to today, a new valid_until, or retire the file",
    };
    NewReviewItem {
        kind: REVIEW_KIND.into(),
        fingerprint: fingerprint(file),
        payload: serde_json::json!({
            "title": format!("Recheck {}", file.rel),
            "body": format!(
                "{} {what}, so recall labels it stale and ranks it last. Recheck it in {}: if it still holds, {fix}. Rechecked closes this card; Hollow doesn't edit the file.",
                file.rel,
                full.display()
            ),
            "path": file.rel,
            "file": full.to_string_lossy(),
            "staleness": file.staleness.label(),
        }),
        risk: "local_write".into(),
        source: "memory".into(),
        source_path: None,
    }
}

/// Recheck cards open at once. Waiting on you items are copied to the
/// tracker (T3b), so a clone with dozens of old memories offers them a few
/// at a time: the next one is offered once one of these closes.
pub(crate) const OPEN_CAP: usize = 5;

/// How long a file held back by [`OPEN_CAP`] waits before a recall offers it
/// again (each try rereads the queue on a thread).
#[cfg_attr(test, allow(dead_code))]
const HELD_RETRY: Duration = Duration::from_secs(10 * 60);

/// Fingerprints this process tried: `None` once in the queue (a repeated
/// recall skips the queue read entirely), `Some(when)` when the cap held the
/// file back at `when` (not retried before [`HELD_RETRY`]).
#[cfg_attr(test, allow(dead_code))] // Read by `offer_in_background` only.
static OFFERED: Mutex<Option<HashMap<String, Option<Instant>>>> = Mutex::new(None);
/// Serializes [`offer_rechecks`], so two recalls can't both pass the cap.
static OFFER_LOCK: Mutex<()> = Mutex::new(());

/// Offers a recheck card for each of `stale` not yet in `vault`'s review
/// queue, while fewer than [`OPEN_CAP`] are open. A file the queue already
/// holds (pending or decided) adds nothing. Returns the fingerprints now in
/// the queue; a file left out by the cap is not among them, so a later
/// recall offers it.
pub(crate) fn offer_rechecks(vault: &Path, root: &Path, stale: &[StaleFile]) -> Vec<String> {
    let _held = OFFER_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let cards = match review_queue::list(vault, Some(REVIEW_KIND)) {
        Ok(cards) => cards,
        Err(error) => {
            eprintln!("[context-stale] review queue unreadable: {error}");
            return Vec::new();
        }
    };
    let known: HashSet<&str> = cards.iter().map(|c| c.fingerprint.as_str()).collect();
    let mut open = cards
        .iter()
        .filter(|c| matches!(c.status.as_str(), "pending" | "snoozed" | "applying"))
        .count();
    let mut settled = Vec::new();
    for file in stale {
        let fp = fingerprint(file);
        if known.contains(fp.as_str()) {
            settled.push(fp);
            continue;
        }
        // The same file under an older stale reason (an unverified card,
        // then its valid_until passes): that open card gives way to this one.
        for old in cards.iter().filter(|c| {
            matches!(c.status.as_str(), "pending" | "snoozed")
                && c.payload.get("path").and_then(|p| p.as_str()) == Some(file.rel.as_str())
        }) {
            match review_queue::supersede(
                vault,
                &old.id,
                "Superseded: the file is stale for another reason now.",
            ) {
                Ok(Some(_)) => open = open.saturating_sub(1),
                Ok(None) => {}
                Err(error) => eprintln!("[context-stale] retire {}: {error}", old.id),
            }
        }
        if open >= OPEN_CAP {
            continue;
        }
        match review_queue::enqueue(vault, recheck_item(root, file)) {
            Ok(_) => {
                open += 1;
                settled.push(fp);
            }
            Err(error) => eprintln!("[context-stale] recheck card for {}: {error}", file.rel),
        }
    }
    settled
}

/// [`offer_rechecks`] on its own thread for the files this process hasn't
/// offered yet; nothing at all when there are none.
#[cfg_attr(test, allow(dead_code))] // Called by `context_layers::sections` outside tests.
pub(crate) fn offer_in_background(vault: &Path, root: &Path, stale: Vec<StaleFile>) {
    let fresh: Vec<StaleFile> = {
        let seen = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
        stale
            .into_iter()
            .filter(
                |f| match seen.as_ref().and_then(|s| s.get(&fingerprint(f))) {
                    None => true,
                    Some(None) => false,
                    Some(Some(held)) => held.elapsed() >= HELD_RETRY,
                },
            )
            .collect()
    };
    if fresh.is_empty() {
        return;
    }
    let (vault, root): (PathBuf, PathBuf) = (vault.to_path_buf(), root.to_path_buf());
    let spawned = std::thread::Builder::new()
        .name("hollow-stale-recheck".into())
        .spawn(move || {
            let settled: HashSet<String> =
                offer_rechecks(&vault, &root, &fresh).into_iter().collect();
            let now = Instant::now();
            let mut seen = OFFERED.lock().unwrap_or_else(|e| e.into_inner());
            let seen = seen.get_or_insert_with(HashMap::new);
            for file in &fresh {
                let fp = fingerprint(file);
                let held = (!settled.contains(&fp)).then_some(now);
                seen.insert(fp, held);
            }
        });
    if let Err(error) = spawned {
        eprintln!("[context-stale] recheck offer not started: {error}");
    }
}

/// Rechecked on a card: closes it (claim, then accept). The file is not
/// touched; a new stale date later opens a new card.
pub(crate) fn mark_rechecked(vault: &Path, item: &ReviewItem) -> Result<(), String> {
    review_queue::claim(vault, &item.id, None)?;
    review_queue::accept(vault, &item.id).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn memory(dates: &str) -> String {
        format!("---\nname: m\ndescription: d\nmetadata:\n  type: project\n{dates}---\nBody\n")
    }

    #[test]
    fn memory_staleness_is_px7s_rule() {
        let today = day("2026-11-18");
        assert_eq!(
            memory_staleness(&memory("  valid_until: 2026-10-01\n"), today),
            Some(Staleness::Expired(day("2026-10-01")))
        );
        assert_eq!(
            memory_staleness(&memory("  verified_at: 2026-01-01\n"), today),
            Some(Staleness::Unverified(day("2026-01-01")))
        );
        // Expired wins over unverified, as in the doctor.
        assert_eq!(
            memory_staleness(
                &memory("  verified_at: 2026-01-01\n  valid_until: 2026-02-01\n"),
                today
            ),
            Some(Staleness::Expired(day("2026-02-01")))
        );
        for fresh in [
            "  verified_at: 2026-11-01\n",
            "  valid_until: 2026-11-18\n",
            "  verified_at: 2026-05-22\n",
            "",
            "  verified_at: last tuesday\n",
        ] {
            assert_eq!(memory_staleness(&memory(fresh), today), None, "{fresh}");
        }
    }

    #[test]
    fn home_staleness_reads_dates_then_last_checked() {
        let today = day("2026-11-18");
        assert_eq!(
            home_staleness("# Home\n\nLast checked: 2026-09-01\n", today),
            Some(Staleness::Unchecked(day("2026-09-01")))
        );
        assert_eq!(
            home_staleness("# Home\n\nLast checked: 2026-11-01\n", today),
            None
        );
        assert_eq!(
            home_staleness(
                "---\nvalid_until: 2026-11-01\n---\n# Home\nLast checked: 2026-11-17\n",
                today
            ),
            Some(Staleness::Expired(day("2026-11-01")))
        );
        assert_eq!(home_staleness("# Home, never checked\n", today), None);
        // Frontmatter dates decide alone: a current valid_until keeps an old
        // Last checked fresh.
        assert_eq!(
            home_staleness(
                "---\nvalid_until: 2027-01-01\n---\n# Home\nLast checked: 2026-01-01\n",
                today
            ),
            None
        );
    }

    #[test]
    fn a_stale_file_is_listed_once_and_again_only_with_a_new_date() {
        let vault = tempfile::tempdir().unwrap();
        let root = Path::new("/clone");
        let file = StaleFile {
            rel: "memory/old-fact.md".into(),
            staleness: Staleness::Unverified(day("2026-01-01")),
        };
        let cards = || review_queue::list(vault.path(), Some(REVIEW_KIND)).unwrap();
        offer_rechecks(vault.path(), root, std::slice::from_ref(&file));
        // Repeated recalls add nothing, pending or decided.
        offer_rechecks(vault.path(), root, std::slice::from_ref(&file));
        let items = cards();
        assert_eq!(items.len(), 1);
        let card = &items[0];
        assert_eq!(card.status, "pending");
        assert_eq!(card.source, "memory");
        assert_eq!(card.payload["title"], "Recheck memory/old-fact.md");
        assert!(card.payload["body"]
            .as_str()
            .unwrap()
            .contains("/clone/memory/old-fact.md"));
        mark_rechecked(vault.path(), card).unwrap();
        offer_rechecks(vault.path(), root, std::slice::from_ref(&file));
        let items = cards();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, "accepted");
        // Re-dated, then stale again: a new card.
        let later = StaleFile {
            staleness: Staleness::Expired(day("2027-01-01")),
            ..file
        };
        offer_rechecks(vault.path(), root, &[later]);
        assert_eq!(cards().len(), 2);
    }

    #[test]
    fn a_new_stale_reason_retires_the_files_open_card() {
        let vault = tempfile::tempdir().unwrap();
        let root = Path::new("/clone");
        let unverified = StaleFile {
            rel: "memory/m.md".into(),
            staleness: Staleness::Unverified(day("2026-01-01")),
        };
        offer_rechecks(vault.path(), root, std::slice::from_ref(&unverified));
        let expired = StaleFile {
            staleness: Staleness::Expired(day("2026-11-10")),
            ..unverified
        };
        offer_rechecks(vault.path(), root, &[expired]);
        let cards = review_queue::list(vault.path(), Some(REVIEW_KIND)).unwrap();
        let open: Vec<_> = cards.iter().filter(|c| c.status == "pending").collect();
        assert_eq!(open.len(), 1);
        assert!(open[0].fingerprint.contains("expired"));
        assert!(cards.iter().any(review_queue::is_superseded));
    }

    #[test]
    fn at_most_open_cap_cards_wait_at_once() {
        let vault = tempfile::tempdir().unwrap();
        let root = Path::new("/clone");
        let stale: Vec<StaleFile> = (0..OPEN_CAP + 2)
            .map(|i| StaleFile {
                rel: format!("memory/m{i}.md"),
                staleness: Staleness::Expired(day("2026-10-01")),
            })
            .collect();
        let settled = offer_rechecks(vault.path(), root, &stale);
        assert_eq!(settled.len(), OPEN_CAP);
        let cards = review_queue::list(vault.path(), Some(REVIEW_KIND)).unwrap();
        assert_eq!(cards.len(), OPEN_CAP);
        // One closes: the next file is offered, the others stay listed once.
        mark_rechecked(vault.path(), &cards[0]).unwrap();
        let settled = offer_rechecks(vault.path(), root, &stale);
        assert_eq!(settled.len(), OPEN_CAP + 1);
        let cards = review_queue::list(vault.path(), Some(REVIEW_KIND)).unwrap();
        assert_eq!(cards.len(), OPEN_CAP + 1);
    }
}
