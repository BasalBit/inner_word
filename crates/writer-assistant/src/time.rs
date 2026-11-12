//! Clock helpers: now, and a moment in words for a person in their own time zone.

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since 1970 (UTC).
pub fn now_seconds() -> i64 {
    now_millis() / 1000
}

/// Milliseconds since 1970 (UTC).
pub fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis() as i64)
        .unwrap_or(0)
}

/// A local date and time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Local {
    year: i32,
    /// 0 = January.
    month: u32,
    day: u32,
    /// Day of the year, from 0.
    day_of_year: i32,
    hour: u32,
    minute: u32,
}

fn local(at: i64) -> Option<Local> {
    let seconds = at as libc::time_t;
    // SAFETY: `tm` is plain data that localtime_r fills in; both pointers are valid for
    // the call, and localtime_r (unlike localtime) keeps no state of its own.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    let filled = unsafe { libc::localtime_r(&seconds, &mut tm) };
    if filled.is_null() {
        return None;
    }
    Some(Local {
        year: tm.tm_year + 1900,
        month: tm.tm_mon.clamp(0, 11) as u32,
        day: tm.tm_mday.max(1) as u32,
        day_of_year: tm.tm_yday,
        hour: tm.tm_hour.clamp(0, 23) as u32,
        minute: tm.tm_min.clamp(0, 59) as u32,
    })
}

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// The moment `at` (seconds since 1970) as it reads after "It resets": "at 15:00" on the
/// day of `now`, "tomorrow at 09:30" on the next, "on 14 November at 09:30" otherwise.
/// In the time zone of this Mac, on a 24-hour clock.
pub fn describe_moment(at: i64, now: i64) -> String {
    let (Some(then), Some(today)) = (local(at), local(now)) else {
        return "later".to_string();
    };
    let clock = format!("{:02}:{:02}", then.hour, then.minute);
    let same_year = then.year == today.year;
    if same_year && then.day_of_year == today.day_of_year {
        format!("at {clock}")
    } else if (same_year && then.day_of_year == today.day_of_year + 1)
        || (then.year == today.year + 1
            && then.day_of_year == 0
            && today.month == 11
            && today.day == 31)
    {
        format!("tomorrow at {clock}")
    } else {
        format!(
            "on {} {} at {clock}",
            then.day, MONTHS[then.month as usize]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moment_reads_as_a_time_today_tomorrow_or_a_date() {
        let now = now_seconds();
        let today = local(now).unwrap();
        // The same minute is today, whatever the zone.
        assert_eq!(
            describe_moment(now, now),
            format!("at {:02}:{:02}", today.hour, today.minute)
        );
        // Ten days on is a date with a month name.
        let later = describe_moment(now + 10 * 86_400, now);
        assert!(later.starts_with("on "), "{later}");
        assert!(MONTHS.iter().any(|month| later.contains(month)), "{later}");
        // A day on is tomorrow (but for the hour a clock change may shift: both readings
        // name the right day).
        let tomorrow = describe_moment(now + 86_400, now);
        assert!(tomorrow.starts_with("tomorrow at "), "{tomorrow}");
    }
}
