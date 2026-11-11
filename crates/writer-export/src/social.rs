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

/// The length every link is counted as on networks that count links at a
/// fixed length (X, Mastodon).
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
    /// - Threads, LinkedIn and Instagram: grapheme clusters
    ///   ([`social_length`]). Their exact rules are not published.
    /// - Bluesky: grapheme clusters, with every link counted as its
    ///   composer shortens it: without the scheme, and with what follows
    ///   the host cut to 13 characters and "..." when it is longer than 15
    ///   (`https://example.com/a/very/long/path` counts as
    ///   `example.com/a/very/long/...`, 27).
    /// - Mastodon: grapheme clusters, every `http://` or `https://` link
    ///   counted as [`LINK_LENGTH`], and of a remote mention
    ///   (`@name@server.example`) only `@name`.
    /// - X: the weighted count of its published rules: every link counts as
    ///   [`LINK_LENGTH`], every emoji (a whole sequence such as a family
    ///   or a flag) as 2, characters outside the Latin, Greek, Cyrillic
    ///   and general punctuation ranges (for example CJK) as 2, everything
    ///   else as 1.
    ///
    /// On X and Bluesky a domain name without a scheme is a link too
    /// (`example.com`, `notes.md`, `www.example.org/page`), as long as it
    /// ends in a country code or one of the common generic top-level
    /// domains and is not part of an e-mail address or a handle. The text
    /// is counted as it is: X normalises to NFC first, so a letter typed
    /// as a base letter plus a combining accent counts one more here.
    pub fn length(self, text: &str) -> usize {
        let text = text.trim();
        match self {
            SocialNetwork::X => x_length(text),
            SocialNetwork::Mastodon => mastodon_length(text),
            SocialNetwork::Bluesky => bluesky_length(text),
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

/// Country-code top-level domains.
const COUNTRY_TLDS: &str = "ac ad ae af ag ai al am ao aq ar as at au aw ax az ba bb bd be bf bg bh bi bj bm bn bo br bs bt bw by bz \
    ca cc cd cf cg ch ci ck cl cm cn co cr cu cv cw cx cy cz de dj dk dm do dz ec ee eg er es et eu fi fj fk fm fo fr \
    ga gd ge gf gg gh gi gl gm gn gp gq gr gs gt gu gw gy hk hm hn hr ht hu id ie il im in io iq ir is it je jm jo jp \
    ke kg kh ki km kn kp kr kw ky kz la lb lc li lk lr ls lt lu lv ly ma mc md me mg mh mk ml mm mn mo mp mq mr ms mt \
    mu mv mw mx my mz na nc ne nf ng ni nl no np nr nu nz om pa pe pf pg ph pk pl pm pn pr ps pt pw py qa re ro rs ru \
    rw sa sb sc sd se sg sh si sk sl sm sn so sr ss st su sv sx sy sz tc td tf tg th tj tk tl tm tn to tr tt tv tw tz \
    ua ug uk us uy uz va vc ve vg vi vn vu wf ws ye yt za zm zw";

/// The generic top-level domains in common use. The full list has more
/// than a thousand entries; a domain under one that is missing here is
/// only recognised with a scheme in front.
const GENERIC_TLDS: &str = "com org net edu gov mil int info biz name pro mobi asia tel travel jobs coop museum aero cat xxx \
    app dev page blog shop store site online tech xyz top club live news media agency studio design art email cloud \
    digital world today life space fun wiki link click social network systems solutions services company group \
    academy school education university institute foundation community center zone plus one ai game games video \
    photo photos pics gallery press review reviews guide tips how software codes tools works build engineering \
    science health care fitness finance money capital bank ventures fund law legal consulting marketing events team \
    house homes rentals city nyc london berlin paris tokyo africa lol wtf ninja rocks cool global earth eco green \
    bio farm coffee pizza beer wine cafe bar restaurant kitchen fashion style music band film movie tv radio audio \
    show theater book books writer author poetry ink page pub fm so";

fn is_tld(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    COUNTRY_TLDS
        .split_ascii_whitespace()
        .chain(GENERIC_TLDS.split_ascii_whitespace())
        .any(|tld| tld == label)
}

/// Characters that end a link when they are its last character.
const TRAILING: [char; 10] = ['.', ',', ';', ':', '!', '?', ')', ']', '"', '\''];

/// The length in bytes of the domain name at the start of `rest`: letters,
/// digits and inner hyphens in dot-separated labels, the last of which is
/// a known top-level domain. `None` when `rest` does not start with one.
fn domain_len(rest: &str) -> Option<usize> {
    let bytes = rest.as_bytes();
    let mut end = 0; // end of the last label that is a top-level domain
    let mut labels = 0;
    let mut i = 0;
    loop {
        let start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        let label = &rest[start..i];
        if label.is_empty() || label.starts_with('-') || label.ends_with('-') {
            break;
        }
        labels += 1;
        // A letter of another script directly after the label means the
        // word goes on: not a domain boundary.
        let boundary = rest[i..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if labels >= 2 && boundary && is_tld(label) {
            end = i;
        }
        if i < bytes.len() && bytes[i] == b'.' {
            i += 1;
        } else {
            break;
        }
    }
    (end > 0).then_some(end)
}

/// A link found in a text: its byte range, and where its host ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Link {
    pub start: usize,
    pub end: usize,
    /// Start of the host (after the scheme, if any).
    pub host: usize,
    /// End of the host and port: start of the path, query or fragment.
    pub host_end: usize,
}

/// The links in `text`, in order.
///
/// A link starts at `http://` or `https://` (any case) at a word start and
/// runs to the next whitespace, without trailing sentence punctuation.
/// With `bare`, a domain name without a scheme (`example.com`,
/// `www.example.org/page`) is a link too, as on X and Bluesky: its last
/// label must be a known top-level domain, and it must not follow `@`
/// (an e-mail address or a handle), `#`, `$`, `-`, `_`, `.` or `/`.
pub(crate) fn find_links(text: &str, bare: bool) -> Vec<Link> {
    let mut links = Vec::new();
    let mut i = 0;
    while i < text.len() {
        let rest = &text[i..];
        let before = text[..i].chars().next_back();
        let at_word_start = before.is_none_or(|c| !c.is_alphanumeric());
        let scheme = ["https://", "http://"].into_iter().find(|s| {
            rest.len() > s.len()
                && rest.is_char_boundary(s.len())
                && rest[..s.len()].eq_ignore_ascii_case(s)
        });
        let run_end = |from: usize| {
            let end = rest[from..]
                .find(char::is_whitespace)
                .map_or(rest.len(), |e| from + e);
            from + rest[from..end].trim_end_matches(TRAILING).len()
        };
        let host_end = |from: usize, end: usize| {
            rest[from..end]
                .find(['/', '?', '#'])
                .map_or(end, |e| from + e)
        };
        if let (Some(scheme), true) = (scheme, at_word_start) {
            let end = run_end(scheme.len());
            if end > scheme.len() {
                links.push(Link {
                    start: i,
                    end: i + end,
                    host: i + scheme.len(),
                    host_end: i + host_end(scheme.len(), end),
                });
                i += end;
                continue;
            }
        }
        let bare_start = bare
            && at_word_start
            && !matches!(
                before,
                Some('@' | '＠' | '#' | '＃' | '$' | '-' | '_' | '.' | '/')
            );
        let domain = if bare_start { domain_len(rest) } else { None };
        if let Some(domain) = domain {
            // A path, query or fragment directly after the domain belongs
            // to the link; so does a port.
            let after = &rest[domain..];
            let port = after
                .strip_prefix(':')
                .map_or(0, |p| p.bytes().take_while(u8::is_ascii_digit).count())
                .checked_add(1)
                .filter(|n| *n > 1)
                .unwrap_or(0);
            let tail = &rest[domain + port..];
            let end = if tail.starts_with(['/', '?', '#']) {
                run_end(domain + port)
            } else {
                domain + port
            };
            // `name@example.com` is an address, not a link.
            if !rest[end..].starts_with('@') {
                links.push(Link {
                    start: i,
                    end: i + end,
                    host: i,
                    host_end: i + host_end(0, end),
                });
                i += end;
                continue;
            }
        }
        i += rest.chars().next().map_or(1, char::len_utf8);
    }
    links
}

/// Splits `text` into alternating non-link and link parts (see
/// [`find_links`]).
fn split_links(text: &str, bare: bool) -> Vec<(&str, Option<Link>)> {
    let mut parts = Vec::new();
    let mut at = 0;
    for link in find_links(text, bare) {
        if at < link.start {
            parts.push((&text[at..link.start], None));
        }
        parts.push((&text[link.start..link.end], Some(link)));
        at = link.end;
    }
    if at < text.len() {
        parts.push((&text[at..], None));
    }
    parts
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
    for (part, link) in split_links(text, true) {
        if link.is_some() {
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

/// Mastodon: every link with a scheme counts as [`LINK_LENGTH`], and of a
/// remote mention (`@name@server.example`) only `@name` counts.
fn mastodon_length(text: &str) -> usize {
    split_links(text, false)
        .into_iter()
        .map(|(part, link)| {
            if link.is_some() {
                LINK_LENGTH
            } else {
                part.graphemes(true).count() - mention_servers(part)
            }
        })
        .sum()
}

/// The total length of the `@server` parts of the remote mentions in
/// `text` (`@name@server.example`).
fn mention_servers(text: &str) -> usize {
    let name = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let server = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '.';
    let mut total = 0;
    let mut from = 0;
    while let Some(found) = text[from..].find('@') {
        let at = from + found;
        from = at + 1;
        let before = text[..at].chars().next_back();
        let after = &text[at + 1..];
        let name_len = after.chars().take_while(|c| name(*c)).count();
        if name_len == 0 || before.is_some_and(|c| c.is_alphanumeric() || c == '@') {
            continue;
        }
        if let Some(host) = after[name_len..].strip_prefix('@') {
            let len = host.chars().take_while(|c| server(*c)).count();
            let host = host[..len].trim_end_matches(['.', '-']);
            if host.contains('.') {
                total += 1 + host.len();
                from = at + 1 + name_len + 1 + host.len();
            }
        }
    }
    total
}

/// Bluesky: grapheme clusters after its composer has shortened the links.
/// A link is shown without its scheme, and what follows the host is cut
/// to 13 characters and "..." when it is longer than 15.
fn bluesky_length(text: &str) -> usize {
    split_links(text, true)
        .into_iter()
        .map(|(part, link)| {
            let Some(link) = link else {
                return part.graphemes(true).count();
            };
            let host = text[link.host..link.host_end].graphemes(true).count();
            let path = &text[link.host_end..link.end];
            let path = if path == "/" { 0 } else { path.chars().count() };
            host + if path > 15 { 16 } else { path }
        })
        .sum()
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
        // Emoji are 2 each, however they are built.
        assert_eq!(x("👍🏽"), 2);
        assert_eq!(x("🏳️‍🌈"), 2);
        assert_eq!(x("❤️"), 2);
        assert_eq!(x("😀😀😀"), 6);
        assert_eq!(
            x("ok 👍🏽 日本 https://example.com/x"),
            3 + 2 + 1 + 4 + 1 + 23
        );
    }

    #[test]
    fn x_counts_bare_domains_as_links() {
        let x = |t: &str| SocialNetwork::X.length(t);
        assert_eq!(x("example.com"), 23);
        assert_eq!(x("Read example.com/a/long/path?x=1#y now"), 5 + 23 + 4);
        assert_eq!(x("www.example.co.uk."), 23 + 1);
        assert_eq!(x("(example.org)"), 1 + 23 + 1);
        assert_eq!(x("see notes.md and Cargo.toml"), 4 + 23 + 5 + 10);
        assert_eq!(x("EXAMPLE.COM:8080/x"), 23);
        assert_eq!(x("a.b.example.io/path, then"), 23 + 6);
        // Not links: addresses, handles, numbers, abbreviations, words
        // that merely contain a dot, unknown endings.
        assert_eq!(x("me@example.com"), 14);
        assert_eq!(x("@example.com"), 12);
        assert_eq!(x("#example.com"), 12);
        assert_eq!(x("3.14 and v1.2.3"), 15);
        assert_eq!(x("e.g. this, i.e. that"), 20);
        assert_eq!(x("end.Start of the next"), 21);
        assert_eq!(x("file.zzz"), 8);
        assert_eq!(x("example.com@host"), 16);
        assert_eq!(x("-example.com _example.com /example.com"), 38);
        assert_eq!(x("example.comet"), 13);
        assert_eq!(x("example.com日本"), 15);
        // A sentence end directly before a known ending is a link on X.
        assert_eq!(x("ok.so"), 23);
    }

    #[test]
    fn links_are_found_with_their_hosts() {
        let l = find_links("a https://Example.com:80/p?q#f, b example.org/x.", true);
        assert_eq!(l.len(), 2);
        let text = "a https://Example.com:80/p?q#f, b example.org/x.";
        assert_eq!(&text[l[0].start..l[0].end], "https://Example.com:80/p?q#f");
        assert_eq!(&text[l[0].host..l[0].host_end], "Example.com:80");
        assert_eq!(&text[l[1].start..l[1].end], "example.org/x");
        assert_eq!(&text[l[1].host..l[1].host_end], "example.org");
        assert_eq!(find_links("example.org and http://x.y", false).len(), 1);
        assert!(find_links("no links here. None", true).is_empty());
        assert!(find_links("", true).is_empty());
    }

    #[test]
    fn bluesky_shortens_links() {
        let b = |t: &str| SocialNetwork::Bluesky.length(t);
        assert_eq!(b("hello 👨‍👩‍👧 world"), 13);
        // Without the scheme.
        assert_eq!(b("https://example.com"), 11);
        assert_eq!(b("https://example.com/"), 11);
        assert_eq!(b("https://example.com/short/path?x"), 11 + 13);
        // More than 15 after the host: 13 and "...".
        assert_eq!(b("https://example.com/0123456789abcde"), 11 + 16);
        assert_eq!(
            b("https://example.com/0123456789abcdef/and/much/more"),
            11 + 16
        );
        assert_eq!(
            b("see https://example.com/0123456789abcdef/and/much/more."),
            4 + 27 + 1
        );
        assert_eq!(b("example.com/0123456789abcdef/and/much/more"), 27);
        assert_eq!(b("me@example.com"), 14);
    }

    #[test]
    fn mastodon_counts_links_and_remote_mentions() {
        let m = |t: &str| SocialNetwork::Mastodon.length(t);
        assert_eq!(m("a https://example.com/long/long/long b"), 2 + 23 + 2);
        // Bare domains are text on Mastodon.
        assert_eq!(m("example.com"), 11);
        assert_eq!(m("hi @ada@mastodon.social!"), 3 + 4 + 1);
        assert_eq!(m("@ada@mastodon.social @ben@example.org."), 4 + 1 + 4 + 1);
        assert_eq!(m("@ada and me@example.com"), 23);
        assert_eq!(m("@ada@"), 5);
        assert_eq!(m("@@x@y.z"), 7);
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
