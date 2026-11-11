//! Character counting for social posts.
//!
//! Social networks count what a reader perceives as characters, not bytes:
//! an emoji family or a letter with combining accents is one character.
//! [`social_length`] counts extended grapheme clusters, and
//! [`SocialNetwork`] adds each network's limit and its special rules, for a
//! status-bar indicator such as "212 / 280".
//!
//! ```
//! use writer_export::social::{SocialNetwork, social_length};
//!
//! assert_eq!(social_length("Hi 👨‍👩‍👧 café"), 9);
//! let count = SocialNetwork::X.count("Launch day! https://example.com/a/very/long/path");
//! assert_eq!(count.length, 12 + 23); // every link counts as 23 on X
//! assert_eq!(count.remaining(), 245);
//! assert!(count.fits());
//! ```

use unicode_properties::UnicodeEmoji;
use unicode_segmentation::UnicodeSegmentation;

/// The number of characters in `text` as social networks count them:
/// extended grapheme clusters, after trimming whitespace at both ends
/// (networks drop it when posting).
pub fn social_length(text: &str) -> usize {
    text.trim().graphemes(true).count()
}

/// The length every link is counted as on networks that shorten links
/// (X, Mastodon).
pub const LINK_LENGTH: usize = 23;

/// A network with a known post length limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocialNetwork {
    /// X (Twitter): 280, weighted.
    X,
    /// Bluesky: 300.
    Bluesky,
    /// Mastodon: 500 (the default of most servers).
    Mastodon,
    /// Threads: 500.
    Threads,
    /// LinkedIn post: 3,000.
    LinkedIn,
    /// Instagram caption: 2,200.
    Instagram,
}

impl SocialNetwork {
    /// Every network, shortest limit first.
    pub const ALL: [SocialNetwork; 6] = [
        SocialNetwork::X,
        SocialNetwork::Bluesky,
        SocialNetwork::Mastodon,
        SocialNetwork::Threads,
        SocialNetwork::Instagram,
        SocialNetwork::LinkedIn,
    ];

    /// Display name.
    pub fn name(self) -> &'static str {
        match self {
            SocialNetwork::X => "X",
            SocialNetwork::Bluesky => "Bluesky",
            SocialNetwork::Mastodon => "Mastodon",
            SocialNetwork::Threads => "Threads",
            SocialNetwork::LinkedIn => "LinkedIn",
            SocialNetwork::Instagram => "Instagram",
        }
    }

    /// The post length limit in characters.
    pub fn limit(self) -> usize {
        match self {
            SocialNetwork::X => 280,
            SocialNetwork::Bluesky => 300,
            SocialNetwork::Mastodon => 500,
            SocialNetwork::Threads => 500,
            SocialNetwork::LinkedIn => 3000,
            SocialNetwork::Instagram => 2200,
        }
    }

    /// The length of `text` as this network counts it.
    ///
    /// - Bluesky, Threads, LinkedIn and Instagram: grapheme clusters
    ///   ([`social_length`]).
    /// - Mastodon: grapheme clusters, every `http://` or `https://` link
    ///   counted as [`LINK_LENGTH`].
    /// - X: the weighted count of its published rules: every link counts as
    ///   [`LINK_LENGTH`], every emoji as 2, characters outside the Latin,
    ///   Greek, Cyrillic and general punctuation ranges (for example CJK)
    ///   as 2, everything else as 1. Only links with a scheme are
    ///   recognised; X also treats bare domains such as `example.com` as
    ///   links.
    pub fn length(self, text: &str) -> usize {
        let text = text.trim();
        match self {
            SocialNetwork::X => x_length(text),
            SocialNetwork::Mastodon => split_links(text)
                .map(|(part, is_link)| {
                    if is_link {
                        LINK_LENGTH
                    } else {
                        part.graphemes(true).count()
                    }
                })
                .sum(),
            _ => social_length(text),
        }
    }

    /// Length, limit and what is left, for a status-bar indicator.
    pub fn count(self, text: &str) -> SocialCount {
        SocialCount {
            network: self,
            length: self.length(text),
            limit: self.limit(),
        }
    }
}

/// The result of [`SocialNetwork::count`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocialCount {
    /// The network counted for.
    pub network: SocialNetwork,
    /// The length of the text as the network counts it.
    pub length: usize,
    /// The network's limit.
    pub limit: usize,
}

impl SocialCount {
    /// Characters left; negative when the text is too long.
    pub fn remaining(&self) -> i64 {
        self.limit as i64 - self.length as i64
    }

    /// Whether the text fits the limit.
    pub fn fits(&self) -> bool {
        self.length <= self.limit
    }

    /// `"212 / 280"`.
    pub fn label(&self) -> String {
        format!("{} / {}", self.length, self.limit)
    }
}

/// Splits `text` into alternating non-link and link parts. A link starts at
/// `http://` or `https://` (any case) at a word start and runs to the next
/// whitespace, without trailing sentence punctuation.
fn split_links(text: &str) -> impl Iterator<Item = (&str, bool)> {
    let mut parts = Vec::new();
    let mut plain_start = 0;
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let scheme = ["https://", "http://"].into_iter().find(|s| {
            rest.len() > s.len()
                && rest.is_char_boundary(s.len())
                && rest[..s.len()].eq_ignore_ascii_case(s)
        });
        let at_word_start = text[..i]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric());
        if let (Some(scheme), true) = (scheme, at_word_start) {
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            let link =
                rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '"', '\'']);
            if link.len() > scheme.len() {
                if plain_start < i {
                    parts.push((&text[plain_start..i], false));
                }
                parts.push((link, true));
                i += link.len();
                plain_start = i;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    if plain_start < text.len() {
        parts.push((&text[plain_start..], false));
    }
    parts.into_iter()
}

/// Whether a grapheme cluster is an emoji as X counts them.
fn is_emoji_grapheme(g: &str) -> bool {
    g.chars()
        .any(|c| c == '\u{fe0f}' || c == '\u{20e3}' || (!c.is_ascii() && c.is_emoji_char()))
}

fn x_length(text: &str) -> usize {
    // Weights in hundredths, as in X's published configuration.
    let light =
        |c: char| matches!(u32::from(c), 0..=4351 | 8192..=8205 | 8208..=8223 | 8242..=8247);
    let mut weight = 0usize;
    for (part, is_link) in split_links(text) {
        if is_link {
            weight += LINK_LENGTH * 100;
            continue;
        }
        for g in part.graphemes(true) {
            if is_emoji_grapheme(g) {
                weight += 200;
            } else {
                weight += g
                    .chars()
                    .map(|c| if light(c) { 100 } else { 200 })
                    .sum::<usize>();
            }
        }
    }
    weight.div_ceil(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes() {
        assert_eq!(social_length(""), 0);
        assert_eq!(social_length("  \n"), 0);
        assert_eq!(social_length("abc"), 3);
        assert_eq!(social_length("  abc \n"), 3);
        assert_eq!(social_length("👨‍👩‍👧‍👦"), 1);
        assert_eq!(social_length("🇦🇱🇺🇸"), 2);
        assert_eq!(social_length("e\u{301}"), 1);
        assert_eq!(social_length("a\nb"), 3);
        assert_eq!(social_length("שלום"), 4);
        assert_eq!(social_length("👍🏽"), 1);
    }

    #[test]
    fn limits() {
        assert_eq!(SocialNetwork::X.limit(), 280);
        assert_eq!(SocialNetwork::Bluesky.limit(), 300);
        assert_eq!(SocialNetwork::Mastodon.limit(), 500);
        assert_eq!(SocialNetwork::Threads.limit(), 500);
        assert_eq!(SocialNetwork::LinkedIn.limit(), 3000);
        assert_eq!(SocialNetwork::Instagram.limit(), 2200);
        let mut last = 0;
        for n in SocialNetwork::ALL {
            assert!(n.limit() >= last);
            last = n.limit();
            assert!(!n.name().is_empty());
        }
    }

    #[test]
    fn x_weights() {
        let x = |t: &str| SocialNetwork::X.length(t);
        assert_eq!(x("hello"), 5);
        assert_eq!(x("café “quoted” — dash"), 20);
        assert_eq!(x("日本語"), 6);
        assert_eq!(x("👍"), 2);
        assert_eq!(x("👨‍👩‍👧‍👦"), 2);
        assert_eq!(x("🇦🇱"), 2);
        assert_eq!(x("1️⃣"), 2);
        assert_eq!(x("a 👍 b"), 6);
        assert_eq!(x("https://example.com"), 23);
        assert_eq!(x("see https://example.com/a/b/c?d=e#f."), 4 + 23 + 1);
        assert_eq!(x("HTTP://X.Y and http://z.w"), 23 + 5 + 23);
        assert_eq!(x("nothttp://x.y"), 13);
        assert_eq!(x("https://"), 8);
        assert_eq!(x(&"a".repeat(280)), 280);
    }

    #[test]
    fn counts() {
        let c = SocialNetwork::X.count(&"a".repeat(300));
        assert_eq!(c.length, 300);
        assert_eq!(c.remaining(), -20);
        assert!(!c.fits());
        assert_eq!(c.label(), "300 / 280");
        let c = SocialNetwork::Bluesky.count("日本語 👍");
        assert_eq!(c.length, 5);
        assert!(c.fits());
        assert_eq!(
            SocialNetwork::Mastodon.length("a https://example.com/long/long/long b"),
            2 + 23 + 2
        );
        assert_eq!(SocialNetwork::LinkedIn.length("https://example.com"), 19);
    }
}
