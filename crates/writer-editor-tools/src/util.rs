//! Small pure helpers: today's date and URL clean-up.

use std::time::{SystemTime, UNIX_EPOCH};

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

/// A calendar date as the insert menu writes it: `11 November 2026`.
/// `month` is 1-based; out-of-range months are clamped.
pub fn format_date(year: i32, month: u32, day: u32) -> String {
    let month = MONTHS[(month.clamp(1, 12) - 1) as usize];
    format!("{day} {month} {year}")
}

/// Today's date in the local time zone, formatted by [`format_date`].
pub fn today_string() -> String {
    let (year, month, day) = local_date();
    format_date(year, month, day)
}

/// `(year, month 1..=12, day)` in the local time zone.
fn local_date() -> (i32, u32, u32) {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as libc::time_t)
        .unwrap_or(0);
    // SAFETY: `localtime_r` only writes into the `tm` we pass and reads the
    // time value; both live on this stack frame.
    let local = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&seconds, &mut tm).is_null() {
            None
        } else {
            Some((tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32))
        }
    };
    local.unwrap_or_else(|| civil_from_days(seconds.div_euclid(86_400)))
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (UTC
/// fallback when the local time zone cannot be read).
pub(crate) fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = (year_of_era + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

/// Cleans up what the user typed into the link field. Returns `None` for an
/// empty field. A URL with a scheme, an anchor (`#part`) or a path
/// (`/notes`) is kept as typed; an e-mail address gets `mailto:`; anything
/// else gets `https://`.
///
/// ```ignore
/// assert_eq!(normalize_url(" example.com/a ").as_deref(), Some("https://example.com/a"));
/// assert_eq!(normalize_url("me@example.com").as_deref(), Some("mailto:me@example.com"));
/// assert_eq!(normalize_url("http://x.org").as_deref(), Some("http://x.org"));
/// assert_eq!(normalize_url("   "), None);
/// ```
pub fn normalize_url(input: &str) -> Option<String> {
    let url = input.trim();
    if url.is_empty() {
        return None;
    }
    if has_scheme(url) || url.starts_with('#') || url.starts_with('/') {
        return Some(url.to_string());
    }
    if url.contains('@') && !url.contains('/') && !url.contains(char::is_whitespace) {
        return Some(format!("mailto:{url}"));
    }
    Some(format!("https://{url}"))
}

/// Whether `url` starts with `scheme:` (RFC 3986 scheme characters). A
/// host with a port (`localhost:8080`) is not a scheme.
fn has_scheme(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'));
    if !valid {
        return false;
    }
    // "localhost:8080/x" and "example.com:443": digits after the colon.
    let port_like = rest
        .split(['/', '?', '#'])
        .next()
        .is_some_and(|port| !port.is_empty() && port.chars().all(|ch| ch.is_ascii_digit()));
    !port_like
}
