use writer_editor_tools::{format_date, normalize_url, today_string};

#[test]
fn dates_are_written_day_month_year() {
    assert_eq!(format_date(2026, 11, 11), "11 November 2026");
    assert_eq!(format_date(2024, 2, 29), "29 February 2024");
    assert_eq!(format_date(1999, 1, 1), "1 January 1999");
    // Out-of-range months are clamped, not a panic.
    assert_eq!(format_date(2026, 0, 5), "5 January 2026");
    assert_eq!(format_date(2026, 13, 5), "5 December 2026");

    let today = today_string();
    let parts: Vec<&str> = today.split(' ').collect();
    assert_eq!(parts.len(), 3, "{today}");
    assert!((1..=31).contains(&parts[0].parse::<u32>().unwrap()));
    assert!(parts[2].parse::<i32>().unwrap() >= 2025);
}

#[test]
fn urls_are_cleaned_up() {
    let url = |input: &str| normalize_url(input);
    assert_eq!(url("   "), None);
    assert_eq!(url(""), None);
    assert_eq!(
        url(" example.com/a ").as_deref(),
        Some("https://example.com/a")
    );
    assert_eq!(
        url("www.example.com").as_deref(),
        Some("https://www.example.com")
    );
    assert_eq!(url("http://x.org").as_deref(), Some("http://x.org"));
    assert_eq!(
        url("HTTPS://X.org/Path?q=1#f").as_deref(),
        Some("HTTPS://X.org/Path?q=1#f")
    );
    assert_eq!(
        url("me@example.com").as_deref(),
        Some("mailto:me@example.com")
    );
    assert_eq!(
        url("mailto:me@example.com").as_deref(),
        Some("mailto:me@example.com")
    );
    assert_eq!(url("tel:+15551234").as_deref(), Some("tel:+15551234"));
    assert_eq!(url("#chapter-2").as_deref(), Some("#chapter-2"));
    assert_eq!(url("/notes/today").as_deref(), Some("/notes/today"));
    // A host with a port is not a scheme.
    assert_eq!(
        url("localhost:8080/x").as_deref(),
        Some("https://localhost:8080/x")
    );
    assert_eq!(
        url("example.com:443").as_deref(),
        Some("https://example.com:443")
    );
    assert_eq!(url("writer://doc/12").as_deref(), Some("writer://doc/12"));
}
