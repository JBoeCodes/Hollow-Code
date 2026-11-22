//! Natural-language quick-add parsing (Todoist's signature feature).
//!
//! Turns a single line like
//!   `Submit weekly report every friday at 4pm for 90m p1 #Work @urgent`
//! into structured fields: title, priority, due date + time, duration,
//! recurrence, project, and labels. The remainder (after every recognized
//! token is stripped) becomes the task title.
//!
//! The parser is deliberately a focused matcher over the common Todoist
//! patterns rather than a general English date engine: it is predictable,
//! dependency-light (regex + chrono), and easy to extend. The core entry
//! point takes `today` explicitly so it is deterministic under test.

use crate::types::ParsedQuickAdd;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use once_cell::sync::Lazy;
use regex::Regex;
use std::ops::Range;

// ── Compiled patterns (case-insensitive) ─────────────────────────────────

static RE_PRIORITY: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)(?:^|\s)(p[1-4])(?:\s|$)").unwrap());
static RE_PROJECT: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)#("[^"]+"|[^\s#@]+)"#).unwrap());
static RE_LABEL: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?i)@("[^"]+"|[^\s#@]+)"#).unwrap());
// Recurrence: "every <unit>", or the bare adverbs daily/weekly/monthly/yearly.
static RE_RECUR_EVERY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bevery\s+(day|weekday|weekend|week|month|year|mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)\b").unwrap()
});
static RE_RECUR_ADVERB: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(daily|weekdays?|weekly|monthly|yearly|annually)\b").unwrap());
// Duration: "for 90m", "for 2h", "for 1h30m", "for 45 min", "for 1 hour".
static RE_DURATION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\bfor\s+(?:(\d+)\s*h(?:ours?|rs?)?\s*)?(?:(\d+)\s*m(?:in(?:ute)?s?)?)?\b")
        .unwrap()
});
// Clock time: "at 4pm", "4:30pm", "14:30", "noon", "midnight".
static RE_TIME_AMPM: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:at\s+)?(\d{1,2})(?::(\d{2}))?\s*(am|pm)\b").unwrap());
static RE_TIME_24H: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\b(?:at\s+)?([01]?\d|2[0-3]):([0-5]\d)\b").unwrap());
static RE_TIME_WORD: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(noon|midnight)\b").unwrap());
// Dates.
static RE_TODAY: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(today|tonight)\b").unwrap());
static RE_TOMORROW: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\b(tomorrow|tmrw)\b").unwrap());
static RE_WEEKDAY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(next\s+)?(mon(?:day)?|tue(?:sday)?|wed(?:nesday)?|thu(?:rsday)?|fri(?:day)?|sat(?:urday)?|sun(?:day)?)\b").unwrap()
});
static RE_IN_N: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\bin\s+(\d+)\s+(day|days|week|weeks)\b").unwrap());
static RE_MONTH_DAY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t|tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\s+(\d{1,2})\b").unwrap()
});
static RE_DAY_MONTH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)\b(\d{1,2})\s+(jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|jun(?:e)?|jul(?:y)?|aug(?:ust)?|sep(?:t|tember)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?)\b").unwrap()
});
static RE_NUMERIC_DATE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"\b(\d{1,2})/(\d{1,2})(?:/(\d{2,4}))?\b").unwrap());
static RE_WS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());

/// Replace a byte range with equal-length spaces (keeps later byte offsets
/// valid and preserves word boundaries). Regex match ranges are on char
/// boundaries, so overwriting them whole with ASCII spaces stays valid UTF-8.
fn blank(s: &mut String, range: Range<usize>) {
    let spaces = " ".repeat(range.len());
    s.replace_range(range, &spaces);
}

fn weekday_from(s: &str) -> Option<Weekday> {
    // Match whole tokens — truncating to 3 chars would map "month" → "mon" →
    // Monday, which silently corrupts "every month" into "every monday".
    match s.to_ascii_lowercase().as_str() {
        "mon" | "monday" => Some(Weekday::Mon),
        "tue" | "tues" | "tuesday" => Some(Weekday::Tue),
        "wed" | "weds" | "wednesday" => Some(Weekday::Wed),
        "thu" | "thur" | "thurs" | "thursday" => Some(Weekday::Thu),
        "fri" | "friday" => Some(Weekday::Fri),
        "sat" | "saturday" => Some(Weekday::Sat),
        "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn weekday_full_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Mon => "monday",
        Weekday::Tue => "tuesday",
        Weekday::Wed => "wednesday",
        Weekday::Thu => "thursday",
        Weekday::Fri => "friday",
        Weekday::Sat => "saturday",
        Weekday::Sun => "sunday",
    }
}

fn month_from(s: &str) -> Option<u32> {
    Some(match &s.to_ascii_lowercase()[..3.min(s.len())] {
        "jan" => 1,
        "feb" => 2,
        "mar" => 3,
        "apr" => 4,
        "may" => 5,
        "jun" => 6,
        "jul" => 7,
        "aug" => 8,
        "sep" => 9,
        "oct" => 10,
        "nov" => 11,
        "dec" => 12,
        _ => return None,
    })
}

/// Days from `today` to the next date on weekday `w`. 0 means today.
fn days_until_weekday(today: NaiveDate, w: Weekday) -> i64 {
    let cur = today.weekday().num_days_from_monday() as i64;
    let tgt = w.num_days_from_monday() as i64;
    (tgt - cur).rem_euclid(7)
}

/// Build a date for `month`/`day` in the year on/after `today` (rolls to next
/// year when the date has already passed this year).
fn month_day_date(today: NaiveDate, month: u32, day: u32) -> Option<NaiveDate> {
    let cand = NaiveDate::from_ymd_opt(today.year(), month, day)?;
    if cand < today {
        NaiveDate::from_ymd_opt(today.year() + 1, month, day)
    } else {
        Some(cand)
    }
}

/// Parse a quick-add line into structured fields, anchored to `today`.
pub fn parse(input: &str, today: NaiveDate) -> ParsedQuickAdd {
    let mut work = input.to_string();
    let mut out = ParsedQuickAdd::default();

    // 1. Priority (p1..p4). Capture group 1 is the pN token; blank only it.
    if let Some(c) = RE_PRIORITY.captures(&work) {
        let m = c.get(1).unwrap();
        out.priority = Some(m.as_str().to_uppercase());
        let r = m.range();
        blank(&mut work, r);
    }

    // 2. Project (#name or #"multi word"). First wins.
    if let Some(c) = RE_PROJECT.captures(&work) {
        out.project = Some(c.get(1).unwrap().as_str().trim_matches('"').to_string());
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
    }

    // 3. Labels (@name), all occurrences.
    let label_ranges: Vec<Range<usize>> = RE_LABEL
        .captures_iter(&work)
        .filter_map(|c| {
            let val = c.get(1)?.as_str().trim_matches('"').to_string();
            if !val.is_empty() && !out.labels.iter().any(|x| x.eq_ignore_ascii_case(&val)) {
                out.labels.push(val);
            }
            Some(c.get(0).unwrap().range())
        })
        .collect();
    for r in label_ranges.into_iter().rev() {
        blank(&mut work, r);
    }

    // 4. Recurrence — sets a default due anchor a later explicit date overrides.
    if let Some(c) = RE_RECUR_EVERY.captures(&work) {
        let unit = c.get(1).unwrap().as_str().to_ascii_lowercase();
        out.recurrence = Some(format!("every {}", normalize_recur_unit(&unit)));
        out.due = recur_anchor(today, &unit).map(|d| d.to_string());
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
    } else if let Some(c) = RE_RECUR_ADVERB.captures(&work) {
        let adv = c.get(1).unwrap().as_str().to_ascii_lowercase();
        let (phrase, anchor) = match adv.as_str() {
            "daily" => ("every day", Some(today)),
            "weekly" => ("every week", Some(today)),
            "monthly" => ("every month", Some(today)),
            "yearly" | "annually" => ("every year", Some(today)),
            _ => ("every weekday", Some(today)), // weekday(s)
        };
        out.recurrence = Some(phrase.to_string());
        out.due = anchor.map(|d| d.to_string());
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
    }

    // 5. Duration ("for 1h30m"). Only fires when at least one unit captured.
    if let Some(c) = RE_DURATION.captures(&work) {
        let hours: u32 = c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let mins: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let total = hours * 60 + mins;
        if total > 0 {
            out.duration_min = Some(total);
            let r = c.get(0).unwrap().range();
            blank(&mut work, r);
        }
    }

    // 6. Time (am/pm, then 24h, then noon/midnight).
    if let Some(c) = RE_TIME_AMPM.captures(&work) {
        let mut h: u32 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        let min: u32 = c.get(2).and_then(|m| m.as_str().parse().ok()).unwrap_or(0);
        let pm = c.get(3).unwrap().as_str().eq_ignore_ascii_case("pm");
        if pm && h < 12 {
            h += 12;
        }
        if !pm && h == 12 {
            h = 0;
        }
        if h < 24 && min < 60 {
            out.due_time = Some(format!("{h:02}:{min:02}"));
            let r = c.get(0).unwrap().range();
            blank(&mut work, r);
        }
    } else if let Some(c) = RE_TIME_24H.captures(&work) {
        let h: u32 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        let min: u32 = c.get(2).unwrap().as_str().parse().unwrap_or(0);
        out.due_time = Some(format!("{h:02}:{min:02}"));
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
    } else if let Some(c) = RE_TIME_WORD.captures(&work) {
        let w = c.get(1).unwrap().as_str().to_ascii_lowercase();
        out.due_time = Some(if w == "noon" {
            "12:00".into()
        } else {
            "00:00".into()
        });
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
    }

    // 7. Date (explicit forms override the recurrence anchor). First match wins.
    let resolved_date: Option<NaiveDate> = if let Some(c) = RE_TODAY.captures(&work) {
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        Some(today)
    } else if let Some(c) = RE_TOMORROW.captures(&work) {
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        Some(today + Duration::days(1))
    } else if let Some(c) = RE_IN_N.captures(&work) {
        let n: i64 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        let unit = c.get(2).unwrap().as_str().to_ascii_lowercase();
        let days = if unit.starts_with("week") { n * 7 } else { n };
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        Some(today + Duration::days(days))
    } else if let Some(c) = RE_WEEKDAY.captures(&work) {
        let is_next = c.get(1).is_some();
        let w = weekday_from(c.get(2).unwrap().as_str());
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        w.map(|w| {
            let mut ahead = days_until_weekday(today, w);
            if is_next {
                ahead += 7;
            }
            today + Duration::days(ahead)
        })
    } else if let Some(c) = RE_MONTH_DAY.captures(&work) {
        let month = month_from(c.get(1).unwrap().as_str());
        let day: u32 = c.get(2).unwrap().as_str().parse().unwrap_or(0);
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        month.and_then(|m| month_day_date(today, m, day))
    } else if let Some(c) = RE_DAY_MONTH.captures(&work) {
        let day: u32 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        let month = month_from(c.get(2).unwrap().as_str());
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        month.and_then(|m| month_day_date(today, m, day))
    } else if let Some(c) = RE_NUMERIC_DATE.captures(&work) {
        let month: u32 = c.get(1).unwrap().as_str().parse().unwrap_or(0);
        let day: u32 = c.get(2).unwrap().as_str().parse().unwrap_or(0);
        let year: Option<i32> = c.get(3).and_then(|m| {
            let y: i32 = m.as_str().parse().ok()?;
            Some(if y < 100 { 2000 + y } else { y })
        });
        let r = c.get(0).unwrap().range();
        blank(&mut work, r);
        match year {
            Some(y) => NaiveDate::from_ymd_opt(y, month, day),
            None => month_day_date(today, month, day),
        }
    } else {
        None
    };
    if let Some(d) = resolved_date {
        out.due = Some(d.to_string());
    }

    // 8. Title = whatever's left, whitespace-collapsed.
    out.title = RE_WS.replace_all(work.trim(), " ").trim().to_string();
    out
}

fn normalize_recur_unit(unit: &str) -> String {
    if let Some(w) = weekday_from(unit) {
        return weekday_full_name(w).to_string();
    }
    unit.to_string()
}

/// The first due date implied by a freshly-created recurring task.
fn recur_anchor(today: NaiveDate, unit: &str) -> Option<NaiveDate> {
    if let Some(w) = weekday_from(unit) {
        return Some(today + Duration::days(days_until_weekday(today, w)));
    }
    match unit {
        "day" | "week" | "month" | "year" | "weekday" | "weekend" => Some(today),
        _ => Some(today),
    }
}

/// Given a recurrence phrase and the date a recurring task was completed,
/// compute the next occurrence's due date. Returns `None` for phrases we
/// don't recognize (caller then just completes the task normally).
pub fn next_occurrence(recurrence: &str, from: NaiveDate) -> Option<NaiveDate> {
    let r = recurrence.trim().to_ascii_lowercase();
    let unit = r
        .strip_prefix("every")
        .map(|s| s.trim())
        .unwrap_or(r.as_str());
    if let Some(w) = weekday_from(unit) {
        // Strictly after `from`: same weekday rolls a full week.
        let mut ahead = days_until_weekday(from, w);
        if ahead == 0 {
            ahead = 7;
        }
        return Some(from + Duration::days(ahead));
    }
    match unit {
        "day" => Some(from + Duration::days(1)),
        "weekday" => {
            // Next Mon-Fri day.
            let mut d = from + Duration::days(1);
            while matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
                d += Duration::days(1);
            }
            Some(d)
        }
        "week" => Some(from + Duration::days(7)),
        "month" => add_months(from, 1),
        "year" => NaiveDate::from_ymd_opt(from.year() + 1, from.month(), from.day())
            .or_else(|| NaiveDate::from_ymd_opt(from.year() + 1, from.month(), 28)),
        _ => None,
    }
}

/// Add `n` months to a date, clamping the day to the target month's length.
fn add_months(d: NaiveDate, n: u32) -> Option<NaiveDate> {
    let total = (d.year() * 12 + d.month0() as i32) + n as i32;
    let year = total.div_euclid(12);
    let month0 = total.rem_euclid(12) as u32;
    let month = month0 + 1;
    // Clamp day to month length.
    for day in (1..=d.day()).rev() {
        if let Some(nd) = NaiveDate::from_ymd_opt(year, month, day) {
            return Some(nd);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn parses_full_todoist_line() {
        // 2026-06-01 is a Monday.
        let today = d(2026, 6, 1);
        let p = parse(
            "Submit weekly report every friday at 4pm for 90m p1 #Work @urgent",
            today,
        );
        assert_eq!(p.title, "Submit weekly report");
        assert_eq!(p.priority.as_deref(), Some("P1"));
        assert_eq!(p.project.as_deref(), Some("Work"));
        assert_eq!(p.labels, vec!["urgent".to_string()]);
        assert_eq!(p.due_time.as_deref(), Some("16:00"));
        assert_eq!(p.duration_min, Some(90));
        assert_eq!(p.recurrence.as_deref(), Some("every friday"));
        // First Friday on/after Mon 2026-06-01 is 2026-06-05.
        assert_eq!(p.due.as_deref(), Some("2026-06-05"));
    }

    #[test]
    fn plain_title_only() {
        let p = parse("Call the dentist", d(2026, 6, 1));
        assert_eq!(p.title, "Call the dentist");
        assert!(p.priority.is_none());
        assert!(p.due.is_none());
    }

    #[test]
    fn relative_and_explicit_dates() {
        let today = d(2026, 6, 1); // Monday
        assert_eq!(
            parse("ship it tomorrow", today).due.as_deref(),
            Some("2026-06-02")
        );
        assert_eq!(
            parse("ship it today", today).due.as_deref(),
            Some("2026-06-01")
        );
        assert_eq!(
            parse("review in 3 days", today).due.as_deref(),
            Some("2026-06-04")
        );
        assert_eq!(
            parse("pay rent jun 15", today).due.as_deref(),
            Some("2026-06-15")
        );
        assert_eq!(
            parse("pay rent 15 jun", today).due.as_deref(),
            Some("2026-06-15")
        );
        // A past month/day rolls to next year.
        assert_eq!(
            parse("taxes jan 10", today).due.as_deref(),
            Some("2027-01-10")
        );
        // 24h time.
        assert_eq!(
            parse("standup 09:30", today).due_time.as_deref(),
            Some("09:30")
        );
    }

    #[test]
    fn duration_hours_and_minutes() {
        let today = d(2026, 6, 1);
        assert_eq!(parse("code for 2h", today).duration_min, Some(120));
        assert_eq!(parse("code for 1h30m", today).duration_min, Some(90));
        assert_eq!(parse("code for 45 min", today).duration_min, Some(45));
    }

    #[test]
    fn next_occurrence_rolls_forward() {
        // Completed on Friday 2026-06-05; weekly friday → next friday.
        assert_eq!(
            next_occurrence("every friday", d(2026, 6, 5)),
            Some(d(2026, 6, 12))
        );
        assert_eq!(
            next_occurrence("every day", d(2026, 6, 5)),
            Some(d(2026, 6, 6))
        );
        // Friday + every weekday → Monday (skips the weekend).
        assert_eq!(
            next_occurrence("every weekday", d(2026, 6, 5)),
            Some(d(2026, 6, 8))
        );
        assert_eq!(
            next_occurrence("every month", d(2026, 1, 31)),
            Some(d(2026, 2, 28))
        );
        assert_eq!(next_occurrence("not a recurrence", d(2026, 6, 5)), None);
    }
}
