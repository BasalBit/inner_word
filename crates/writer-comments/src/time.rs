//! Timestamps for comments: the system clock and relative time labels.
//!
//! `writer-core` never reads a clock; every comment command takes a
//! [`Timestamp`] (milliseconds since the Unix epoch). The panel reads the
//! system clock through [`now`] unless a test installs its own clock with
//! `CommentsPanel::set_clock`.

use std::time::{SystemTime, UNIX_EPOCH};

use writer_core::Timestamp;

const SECOND: i64 = 1000;
const MINUTE: i64 = 60 * SECOND;
const HOUR: i64 = 60 * MINUTE;
const DAY: i64 = 24 * HOUR;
const WEEK: i64 = 7 * DAY;
/// An average month (30.44 days), good enough for "3 months ago".
const MONTH: i64 = 2_629_800_000;
const YEAR: i64 = 365 * DAY;

/// The system clock in milliseconds since the Unix epoch.
pub fn now() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as Timestamp)
        .unwrap_or(0)
}

/// How long ago `then` was, seen from `now`, in plain words: "just now",
/// "5 min ago", "2 hours ago", "yesterday", "3 days ago", "2 weeks ago",
/// "4 months ago", "1 year ago". A time in the future (a clock that was set
/// back) reads "just now".
pub fn relative_time(then: Timestamp, now: Timestamp) -> String {
    let elapsed = now.saturating_sub(then);
    if elapsed < MINUTE {
        return "just now".to_string();
    }
    if elapsed < HOUR {
        return format!("{} min ago", elapsed / MINUTE);
    }
    if elapsed < DAY {
        return counted(elapsed / HOUR, "hour");
    }
    if elapsed < 2 * DAY {
        return "yesterday".to_string();
    }
    if elapsed < WEEK {
        return counted(elapsed / DAY, "day");
    }
    if elapsed < MONTH {
        return counted(elapsed / WEEK, "week");
    }
    if elapsed < YEAR {
        return counted(elapsed / MONTH, "month");
    }
    counted(elapsed / YEAR, "year")
}

fn counted(count: i64, unit: &str) -> String {
    if count == 1 {
        format!("1 {unit} ago")
    } else {
        format!("{count} {unit}s ago")
    }
}
