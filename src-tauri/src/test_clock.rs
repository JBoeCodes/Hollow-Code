//! Tests that read the local day themselves (overnight D2). A test whose
//! code under test reads the clock, and which writes, waits and reads again,
//! can straddle local midnight and see two days. [`clear_of_midnight`] lets
//! such a test start outside the last `margin` of the day: inside it, it
//! sleeps until midnight has passed (at most `margin`, in that window only).

use chrono::{DateTime, Duration, Local, NaiveTime};

/// How long to wait at `now` so the next `margin` stays on one local day:
/// `None` outside the last `margin` before midnight.
pub(crate) fn wait_at(now: DateTime<Local>, margin: Duration) -> Option<std::time::Duration> {
    let midnight = (now.date_naive() + Duration::days(1))
        .and_time(NaiveTime::MIN)
        .and_local_timezone(Local)
        .earliest()?;
    let left = midnight - now;
    (left <= margin).then(|| {
        (left + Duration::milliseconds(50))
            .to_std()
            .unwrap_or_default()
    })
}

/// Sleep past local midnight if it is less than `margin` away.
pub(crate) fn clear_of_midnight(margin: Duration) {
    if let Some(wait) = wait_at(Local::now(), margin) {
        std::thread::sleep(wait);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn waits_only_in_the_last_margin_of_the_day() {
        let at = |h, m, s| {
            Local
                .with_ymd_and_hms(2026, 10, 7, h, m, s)
                .single()
                .unwrap()
        };
        assert_eq!(wait_at(at(12, 0, 0), Duration::minutes(3)), None);
        assert_eq!(wait_at(at(23, 56, 59), Duration::minutes(3)), None);
        let wait = wait_at(at(23, 59, 0), Duration::minutes(3)).unwrap();
        assert!(
            wait >= std::time::Duration::from_secs(60) && wait < std::time::Duration::from_secs(61),
            "{wait:?}"
        );
    }
}
