//! UTF-16 ranges (what `NSString` counts in) to UTF-8 byte ranges and
//! back, without the system checker.

use innerword_spell::{byte_ranges_of_utf16, utf16_range_of_bytes};

/// Every word of `text` there and back, against `encode_utf16`.
fn round_trip(text: &str) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let mut starts = Vec::new();
    let mut at = 0;
    for word in text.split(' ') {
        starts.push((at, at + word.len()));
        at += word.len() + 1;
    }
    for (start, end) in starts {
        let utf16 = utf16_range_of_bytes(text, start..end);
        assert_eq!(
            String::from_utf16(&units[utf16.clone()]).unwrap(),
            &text[start..end],
            "{text:?}"
        );
        let bytes = byte_ranges_of_utf16(text, &[(utf16.start, utf16.len())]);
        assert_eq!(bytes, [start..end], "{text:?}");
    }
}

#[test]
fn words_round_trip_through_utf16() {
    round_trip("plain ascii words");
    round_trip("naïve café Zürich español");
    round_trip("cafe\u{301} e\u{301}te\u{301} decomposed");
    round_trip("😀 mispeled 👨‍👩‍👧 wrods 🇦🇱 after 𝒳 astral");
    round_trip("日本語 のテキスト と mispeled");
    round_trip("");
}

#[test]
fn several_ranges_in_one_walk() {
    let text = "😀😀 ab 😀 cd é ef";
    //          UTF-16: 😀=2 each. "😀😀 " = 5 units, "ab" at 5..7, " 😀 " 7..11, "cd" 11..13
    let ranges = byte_ranges_of_utf16(text, &[(5, 2), (11, 2), (16, 2)]);
    let words: Vec<&str> = ranges.iter().map(|r| &text[r.clone()]).collect();
    assert_eq!(words, ["ab", "cd", "ef"]);
    // Asked out of order, the walk starts again.
    let ranges = byte_ranges_of_utf16(text, &[(11, 2), (5, 2)]);
    let words: Vec<&str> = ranges.iter().map(|r| &text[r.clone()]).collect();
    assert_eq!(words, ["cd", "ab"]);
}

#[test]
fn never_inside_a_character() {
    let text = "a😀b";
    // Unit 2 is the second half of the surrogate pair: the range grows to
    // the whole character instead of splitting it.
    assert_eq!(byte_ranges_of_utf16(text, &[(1, 1)]), [1..5]);
    assert_eq!(byte_ranges_of_utf16(text, &[(2, 1)]), [1..5]);
    assert_eq!(byte_ranges_of_utf16(text, &[(2, 2)]), [1..6]);
    // Past the end stops at the end.
    assert_eq!(byte_ranges_of_utf16(text, &[(3, 99)]), [5..6]);
    assert_eq!(byte_ranges_of_utf16(text, &[(99, 1)]), [6..6]);
}
