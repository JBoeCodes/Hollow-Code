//! Keyset page cursors (merged plan slice 4, B·F15).
//!
//! A cursor names the last item a page carried by its sort key, `(at, id)`,
//! not by its position. The next page is everything strictly after that key
//! in the list's order, so an item resolving between two reads never shifts
//! a later one off the page boundary (an offset would skip it), and an item
//! arriving never repeats the boundary item. Callers treat the string as
//! opaque: base64 (URL-safe, unpadded) of the JSON `[at, id]`.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};

/// The sort key of the last item on a page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Keyset {
    /// When the item happened (RFC 3339), when known.
    pub at: Option<String>,
    /// Unique within the list.
    pub id: String,
}

impl Keyset {
    pub(crate) fn new(at: Option<&str>, id: &str) -> Self {
        Self {
            at: at.map(str::to_string),
            id: id.to_string(),
        }
    }

    /// The opaque cursor string for this key.
    pub(crate) fn encode(&self) -> String {
        let json = serde_json::json!([self.at, self.id]).to_string();
        URL_SAFE_NO_PAD.encode(json)
    }

    /// A cursor string back to its key; an error for anything this module
    /// did not write, so a bad cursor never silently restarts the list.
    pub(crate) fn decode(cursor: &str) -> Result<Self, String> {
        let bad = || "That page cursor is not one this list gave out.".to_string();
        let bytes = URL_SAFE_NO_PAD.decode(cursor.trim()).map_err(|_| bad())?;
        let (at, id): (Option<String>, String) =
            serde_json::from_slice(&bytes).map_err(|_| bad())?;
        Ok(Self { at, id })
    }

    /// Newest first (an item with no time after every timed one), ties by
    /// id ascending: the order every keyset-paged list here uses.
    pub(crate) fn order(&self, other: &Self) -> std::cmp::Ordering {
        newest_first(self.at.as_deref(), &self.id, other.at.as_deref(), &other.id)
    }
}

/// The shared order as a comparator over borrowed keys: `at` descending
/// (`None` last), then `id` ascending.
pub(crate) fn newest_first(
    a_at: Option<&str>,
    a_id: &str,
    b_at: Option<&str>,
    b_id: &str,
) -> std::cmp::Ordering {
    b_at.cmp(&a_at).then_with(|| a_id.cmp(b_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn a_cursor_round_trips_and_a_foreign_one_is_refused() {
        for key in [
            Keyset::new(Some("2026-10-02T10:00:00Z"), "approval:apr-1"),
            Keyset::new(None, "result:workspaces/a/outputs/r.md"),
        ] {
            assert_eq!(Keyset::decode(&key.encode()).unwrap(), key);
        }
        assert!(Keyset::decode("25").is_err());
        assert!(Keyset::decode("").is_err());
        assert!(Keyset::decode("not base64 at all!").is_err());
    }

    #[test]
    fn newest_first_puts_untimed_last_and_breaks_ties_by_id() {
        let new = Keyset::new(Some("2026-10-02T10:00:00Z"), "b");
        let old = Keyset::new(Some("2026-10-01T10:00:00Z"), "a");
        let none = Keyset::new(None, "a");
        assert_eq!(new.order(&old), Ordering::Less);
        assert_eq!(old.order(&none), Ordering::Less);
        let tie = Keyset::new(Some("2026-10-02T10:00:00Z"), "c");
        assert_eq!(new.order(&tie), Ordering::Less);
    }
}
