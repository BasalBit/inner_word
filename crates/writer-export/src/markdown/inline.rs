//! Inline rendering: attribute runs to nested Markdown markup.
//!
//! The pipeline for one block:
//!
//! 1. The text is cut into characters, lines are trimmed, and every mark
//!    (link, bold, italic, …) becomes a set of intervals over the
//!    characters.
//! 2. Whitespace at the edges of an interval moves outside it, because
//!    `** bold **` is not emphasis.
//! 3. Intervals that partly overlap are split until they nest (links are
//!    never split; the mark of lower rank gives way).
//! 4. The nested intervals are written as tokens, text is escaped, and the
//!    emphasis delimiters are checked with a simulation of the CommonMark
//!    delimiter algorithm ([`super::emphasis`]). A mark whose delimiters a
//!    parser would not pair as intended (for example bold that ends in
//!    punctuation directly before a letter) is written as an HTML tag
//!    instead, which always works.

use writer_core::{Block, InlineAttrs};

use super::emphasis::{self, Piece};
use super::escape::{ChunkPosition, TextContext, escape_text, is_autolinkable, link_destination};
use super::{MarkdownHighlight, MarkdownOptions};

/// Raw text inserted at a byte offset of the block text (a comment or a
/// footnote reference).
#[derive(Clone, Debug)]
pub(crate) struct Anchor {
    pub offset: usize,
    pub raw: String,
}

/// A mark that wraps a range of characters. The order is the rank: when
/// two marks partly overlap, the later one is split.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mark {
    Link(String),
    Span(String),
    Highlight,
    Underline,
    Strike,
    Bold,
    Italic,
}

impl Mark {
    fn rank(&self) -> u8 {
        match self {
            Mark::Link(_) => 0,
            Mark::Span(_) => 1,
            Mark::Highlight => 2,
            Mark::Underline => 3,
            Mark::Strike => 4,
            Mark::Bold => 5,
            Mark::Italic => 6,
        }
    }

    /// The emphasis delimiter of the mark, if it has one.
    fn delimiter(&self) -> Option<&'static str> {
        match self {
            Mark::Strike => Some("~~"),
            Mark::Bold => Some("**"),
            Mark::Italic => Some("*"),
            _ => None,
        }
    }
}

const FAMILIES: usize = 7;

/// How many delimiters are replaced by HTML tags one at a time before the
/// rest are replaced together.
const SINGLE_REPLACEMENTS: usize = 8;

/// The marks of one attribute run, one slot per family in rank order.
fn marks_of(attrs: &InlineAttrs, options: &MarkdownOptions) -> [Option<Mark>; FAMILIES] {
    let link = attrs
        .link
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty());
    let mut style = String::new();
    if options.colors
        && let Some(c) = attrs.color
    {
        style.push_str(&format!("color:{}", c.to_hex()));
    }
    if options.fonts
        && let Some(f) = attrs.font.as_deref()
    {
        let name: String = f
            .chars()
            .filter(|c| !matches!(c, '"' | '\'' | '<' | '>' | ';' | '&' | '\\') && !c.is_control())
            .collect();
        if !name.trim().is_empty() {
            if !style.is_empty() {
                style.push(';');
            }
            style.push_str(&format!("font-family:'{}'", name.trim()));
        }
    }
    [
        link.map(|u| Mark::Link(u.to_string())),
        (!style.is_empty()).then_some(Mark::Span(style)),
        (attrs.highlight.is_some() && options.highlight != MarkdownHighlight::Drop)
            .then_some(Mark::Highlight),
        (attrs.underline && options.underline_html).then_some(Mark::Underline),
        attrs.strikethrough.then_some(Mark::Strike),
        attrs.bold.then_some(Mark::Bold),
        attrs.italic.then_some(Mark::Italic),
    ]
}

#[derive(Clone, Copy, Debug)]
struct Cell {
    ch: char,
    /// Index of the attribute run the character came from.
    span: usize,
    code: bool,
}

impl Cell {
    /// Whitespace for the purpose of delimiter placement. Characters of a
    /// code span never count: the delimiter touches the backtick.
    fn is_space(&self) -> bool {
        !self.code && self.ch.is_whitespace()
    }
}

#[derive(Clone, Debug)]
struct Interval {
    start: usize,
    end: usize,
    mark: Mark,
    /// Write the mark as an HTML tag instead of a delimiter.
    html: bool,
}

#[derive(Clone, Debug)]
enum Token {
    Text(String),
    Code(String),
    Break,
    Raw(String),
    Open(usize),
    Close(usize),
}

fn trim_interval(cells: &[Cell], mut start: usize, mut end: usize) -> Option<(usize, usize)> {
    while start < end && cells[start].is_space() {
        start += 1;
    }
    while end > start && cells[end - 1].is_space() {
        end -= 1;
    }
    (start < end).then_some((start, end))
}

/// The characters of a block after trimming, and where the anchors go.
struct Prepared {
    cells: Vec<Cell>,
    /// `(cell index, raw text)`, sorted by index.
    anchors: Vec<(usize, String)>,
}

fn prepare(block: &Block, anchors: &[Anchor], single_line: bool) -> Prepared {
    let mut cells = Vec::with_capacity(block.len());
    let mut bytes = Vec::with_capacity(block.len());
    for (si, (range, text, attrs)) in block.text_spans().enumerate() {
        for (off, ch) in text.char_indices() {
            cells.push(Cell {
                ch,
                span: si,
                code: attrs.code && ch != '\n',
            });
            bytes.push(range.start + off);
        }
    }
    let n = cells.len();
    let mut keep = vec![true; n];
    let trimmable = |c: &Cell| !c.code && c.ch != '\n' && c.ch.is_whitespace();
    // Whitespace (no-break spaces too) at both ends of every line.
    let mut line_start = 0;
    for i in 0..=n {
        if i == n || cells[i].ch == '\n' {
            let mut a = line_start;
            while a < i && trimmable(&cells[a]) {
                keep[a] = false;
                a += 1;
            }
            let mut b = i;
            while b > a && trimmable(&cells[b - 1]) {
                keep[b - 1] = false;
                b -= 1;
            }
            line_start = i + 1;
        }
    }
    // Empty lines at the start and the end of the block.
    for i in 0..n {
        if keep[i] && cells[i].ch != '\n' {
            break;
        }
        keep[i] = false;
    }
    for i in (0..n).rev() {
        if keep[i] && cells[i].ch != '\n' {
            break;
        }
        keep[i] = false;
    }
    let mut kept: Vec<Cell> = Vec::with_capacity(n);
    let mut kept_bytes: Vec<usize> = Vec::with_capacity(n);
    for i in 0..n {
        if !keep[i] {
            continue;
        }
        let mut cell = cells[i];
        if single_line && cell.ch == '\n' {
            // A heading is one line: line breaks become single spaces.
            if kept.last().is_some_and(|c: &Cell| c.ch == ' ' && !c.code) {
                continue;
            }
            cell.ch = ' ';
        }
        kept.push(cell);
        kept_bytes.push(bytes[i]);
    }

    let line_start_at = |i: usize| i == 0 || kept[i - 1].ch == '\n';
    let mut placed: Vec<(usize, String)> = anchors
        .iter()
        .filter(|a| !a.raw.is_empty())
        .map(|a| {
            let mut i = kept_bytes.partition_point(|b| *b < a.offset);
            // Raw HTML or a footnote label at the start of a line could be
            // read as a block; the end of that line is as good a place.
            while i < kept.len() && line_start_at(i) {
                match kept[i..].iter().position(|c| c.ch == '\n') {
                    Some(0) => i += 1,
                    Some(len) => i += len,
                    None => i = kept.len(),
                }
            }
            (i, a.raw.clone())
        })
        .collect();
    placed.sort_by_key(|(i, _)| *i);
    Prepared {
        cells: kept,
        anchors: placed,
    }
}

/// The nested intervals of all marks, sorted outermost first.
fn intervals(cells: &[Cell], spans: &[[Option<Mark>; FAMILIES]]) -> Vec<Interval> {
    let mut accepted: Vec<Interval> = Vec::new();
    // One pass per family, in rank order: `family` indexes the inner arrays.
    #[allow(clippy::needless_range_loop)]
    for family in 0..FAMILIES {
        let mut candidates: Vec<(usize, usize, Mark)> = Vec::new();
        let mut i = 0;
        while i < cells.len() {
            let Some(mark) = &spans[cells[i].span][family] else {
                i += 1;
                continue;
            };
            let start = i;
            while i < cells.len() && spans[cells[i].span][family].as_ref() == Some(mark) {
                i += 1;
            }
            candidates.push((start, i, mark.clone()));
        }
        for (start, end, mark) in candidates {
            let Some((start, end)) = trim_interval(cells, start, end) else {
                continue;
            };
            // Split where an accepted interval of higher rank crosses.
            let mut cuts = vec![start, end];
            for a in &accepted {
                if a.start > start && a.start < end && a.end > end {
                    cuts.push(a.start);
                }
                if a.end > start && a.end < end && a.start < start {
                    cuts.push(a.end);
                }
            }
            cuts.sort_unstable();
            cuts.dedup();
            for w in cuts.windows(2) {
                if let Some((s, e)) = trim_interval(cells, w[0], w[1]) {
                    accepted.push(Interval {
                        start: s,
                        end: e,
                        mark: mark.clone(),
                        html: false,
                    });
                }
            }
        }
    }
    accepted.sort_by(|a, b| {
        a.start
            .cmp(&b.start)
            .then(b.end.cmp(&a.end))
            .then(a.mark.rank().cmp(&b.mark.rank()))
    });
    accepted
}

struct Tokens {
    tokens: Vec<Token>,
    /// Token index of the opener and closer of every interval that was
    /// written with tokens (not as an autolink).
    ends: Vec<Option<(usize, usize)>>,
}

fn tokenize(prepared: &Prepared, ivs: &[Interval]) -> Tokens {
    let cells = &prepared.cells;
    let mut tokens: Vec<Token> = Vec::new();
    let mut ends: Vec<Option<(usize, usize)>> = vec![None; ivs.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut next_iv = 0;
    let mut next_anchor = 0;
    let mut skip_until = 0;

    let push_char = |tokens: &mut Vec<Token>, cell: &Cell| {
        if cell.ch == '\n' {
            tokens.push(Token::Break);
        } else if cell.code {
            match tokens.last_mut() {
                Some(Token::Code(s)) => s.push(cell.ch),
                _ => tokens.push(Token::Code(cell.ch.to_string())),
            }
        } else {
            match tokens.last_mut() {
                Some(Token::Text(s)) => s.push(cell.ch),
                _ => tokens.push(Token::Text(cell.ch.to_string())),
            }
        }
    };

    for pos in 0..=cells.len() {
        while let Some(&top) = stack.last() {
            if ivs[top].end != pos {
                break;
            }
            stack.pop();
            let open = ends[top].map_or(0, |e| e.0);
            ends[top] = Some((open, tokens.len()));
            tokens.push(Token::Close(top));
        }
        while next_anchor < prepared.anchors.len() && prepared.anchors[next_anchor].0 == pos {
            if pos >= skip_until {
                tokens.push(Token::Raw(prepared.anchors[next_anchor].1.clone()));
            }
            next_anchor += 1;
        }
        while next_iv < ivs.len() && ivs[next_iv].start == pos {
            let iv = &ivs[next_iv];
            if let Mark::Link(url) = &iv.mark
                && autolink(prepared, ivs, next_iv, url)
            {
                tokens.push(Token::Raw(format!("<{url}>")));
                skip_until = iv.end;
            } else {
                ends[next_iv] = Some((tokens.len(), 0));
                tokens.push(Token::Open(next_iv));
                stack.push(next_iv);
            }
            next_iv += 1;
        }
        if pos < cells.len() && pos >= skip_until {
            push_char(&mut tokens, &cells[pos]);
        }
    }
    Tokens { tokens, ends }
}

/// Whether the link interval `index` can be written as `<url>`: its text is
/// exactly the URL and nothing else starts or ends inside it.
fn autolink(prepared: &Prepared, ivs: &[Interval], index: usize, url: &str) -> bool {
    let iv = &ivs[index];
    let cells = &prepared.cells[iv.start..iv.end];
    is_autolinkable(url)
        && cells.iter().all(|c| !c.code)
        && cells.iter().map(|c| c.ch).eq(url.chars())
        && ivs.get(index + 1).is_none_or(|n| n.start >= iv.end)
        && prepared
            .anchors
            .iter()
            .all(|(i, _)| *i <= iv.start || *i >= iv.end)
}

/// A code span holding `content` literally.
pub(crate) fn code_span(content: &str) -> String {
    let mut longest = 0;
    let mut run = 0;
    for c in content.chars() {
        run = if c == '`' { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    let fence = "`".repeat(longest + 1);
    let all_spaces = content.chars().all(|c| c == ' ');
    let pad = content.starts_with('`')
        || content.ends_with('`')
        || (content.starts_with(' ') && content.ends_with(' ') && !all_spaces);
    if pad {
        format!("{fence} {content} {fence}")
    } else {
        format!("{fence}{content}{fence}")
    }
}

/// Code as an HTML element, for the rare place where a backtick code span
/// cannot be used. Every character that means something to Markdown or
/// HTML is escaped.
fn html_code(content: &str) -> String {
    let mut out = String::from("<code>");
    for c in content.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c if c.is_ascii_punctuation() => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push_str("</code>");
    out
}

fn open_text(iv: &Interval, options: &MarkdownOptions) -> String {
    match &iv.mark {
        Mark::Link(_) => "[".to_string(),
        Mark::Span(style) => format!("<span style=\"{style}\">"),
        Mark::Highlight if options.highlight == MarkdownHighlight::Equals => "==".to_string(),
        Mark::Highlight => "<mark>".to_string(),
        Mark::Underline => "<u>".to_string(),
        Mark::Strike if iv.html => "<del>".to_string(),
        Mark::Bold if iv.html => "<strong>".to_string(),
        Mark::Italic if iv.html => "<em>".to_string(),
        mark => mark.delimiter().unwrap_or_default().to_string(),
    }
}

fn close_text(iv: &Interval, options: &MarkdownOptions) -> String {
    match &iv.mark {
        Mark::Link(url) => format!("]({})", link_destination(url)),
        Mark::Span(_) => "</span>".to_string(),
        Mark::Highlight if options.highlight == MarkdownHighlight::Equals => "==".to_string(),
        Mark::Highlight => "</mark>".to_string(),
        Mark::Underline => "</u>".to_string(),
        Mark::Strike if iv.html => "</del>".to_string(),
        Mark::Bold if iv.html => "</strong>".to_string(),
        Mark::Italic if iv.html => "</em>".to_string(),
        mark => mark.delimiter().unwrap_or_default().to_string(),
    }
}

/// The literal text of every token.
fn render(
    tokens: &[Token],
    ivs: &[Interval],
    options: &MarkdownOptions,
    heading: bool,
    ctx: TextContext,
) -> Vec<String> {
    let mut link_depth = 0usize;
    let mut out = Vec::with_capacity(tokens.len());
    // A block that starts with a link starts with `[`. If a code span in
    // that link held `]:`, the line would be a link reference definition
    // (block structure wins over code spans) and the paragraph would
    // vanish. Such a code span is written as `<code>` instead.
    let html_code_until = match tokens.first() {
        Some(Token::Open(iv)) if matches!(ivs[*iv].mark, Mark::Link(_)) => tokens
            .iter()
            .position(|t| matches!(t, Token::Close(c) if c == iv))
            .unwrap_or(tokens.len()),
        _ => 0,
    };
    for (i, token) in tokens.iter().enumerate() {
        let s = match token {
            Token::Text(text) => {
                let pos = ChunkPosition {
                    block_start: i == 0,
                    line_start: i == 0 || matches!(tokens[i - 1], Token::Break),
                    line_end: i + 1 == tokens.len() || matches!(tokens[i + 1], Token::Break),
                    in_link: link_depth > 0,
                    heading_end: heading && i + 1 == tokens.len(),
                    after_bracket: i > 0
                        && matches!(&tokens[i - 1], Token::Raw(raw) if raw.ends_with(']')),
                    before_bracket: match tokens.get(i + 1) {
                        Some(Token::Open(iv)) => matches!(ivs[*iv].mark, Mark::Link(_)),
                        Some(Token::Raw(raw)) => raw.starts_with('['),
                        _ => false,
                    },
                };
                escape_text(text, pos, ctx)
            }
            Token::Code(code) if i < html_code_until && code.contains("]:") => html_code(code),
            Token::Code(code) => code_span(code),
            Token::Break => "\\\n".to_string(),
            Token::Raw(raw) => raw.clone(),
            Token::Open(iv) => {
                if matches!(ivs[*iv].mark, Mark::Link(_)) {
                    link_depth += 1;
                }
                open_text(&ivs[*iv], options)
            }
            Token::Close(iv) => {
                if matches!(ivs[*iv].mark, Mark::Link(_)) {
                    link_depth = link_depth.saturating_sub(1);
                }
                close_text(&ivs[*iv], options)
            }
        };
        out.push(s);
    }
    out
}

/// The view of the rendered tokens the emphasis simulation needs.
fn pieces(tokens: &[Token], ivs: &[Interval], strings: &[String]) -> Vec<Piece> {
    tokens
        .iter()
        .zip(strings)
        .map(|(token, s)| match token {
            Token::Open(i) | Token::Close(i) => {
                let iv = &ivs[*i];
                let open = matches!(token, Token::Open(_));
                match (&iv.mark, iv.html) {
                    (Mark::Link(_), _) if open => Piece::LinkOpen,
                    (Mark::Link(_), _) => Piece::LinkClose,
                    (mark, false) if mark.delimiter().is_some() => Piece::Delimiter {
                        ch: s.chars().next().unwrap_or('*'),
                        len: s.chars().count(),
                    },
                    _ => Piece::Other {
                        first: s.chars().next(),
                        last: s.chars().next_back(),
                    },
                }
            }
            _ => Piece::Other {
                first: s.chars().next(),
                last: s.chars().next_back(),
            },
        })
        .collect()
}

/// How many underscores could open or close emphasis: all but those
/// between two letters or digits, which never do.
fn lone_underscores(cells: &[Cell], spans: &[[Option<Mark>; FAMILIES]]) -> usize {
    // A neighbour with other marks may be separated by markup. (Runs that
    // differ only in what is not written, a colour for one, are written
    // as one piece of text.)
    let word = |c: Option<&Cell>, span: usize| {
        c.is_some_and(|c| !c.code && spans[c.span] == spans[span] && c.ch.is_alphanumeric())
    };
    (0..cells.len())
        .filter(|&i| {
            let span = cells[i].span;
            !cells[i].code
                && cells[i].ch == '_'
                && !(word(i.checked_sub(1).map(|p| &cells[p]), span)
                    && word(cells.get(i + 1), span))
        })
        .count()
}

/// Renders the inline content of a text block as Markdown. Line breaks in
/// the block come out as hard breaks (a backslash and a newline); the
/// caller adds the container prefix after each newline. `heading` makes it
/// one line. The result is empty when the block has no visible text.
pub(crate) fn render_inline(
    block: &Block,
    anchors: &[Anchor],
    options: &MarkdownOptions,
    heading: bool,
) -> String {
    let mut prepared = prepare(block, anchors, heading);
    if prepared.cells.is_empty() {
        return String::new();
    }
    let span_marks: Vec<[Option<Mark>; FAMILIES]> =
        block.spans().map(|(_, a)| marks_of(a, options)).collect();
    let mut ivs = intervals(&prepared.cells, &span_marks);
    // A footnote reference inside link text would break the link, and one
    // right before a link would be read together with it: anchors inside
    // or directly before a link move to its end.
    for (at, _) in &mut prepared.anchors {
        // Links can touch, so look again from the new place.
        while let Some(l) = ivs
            .iter()
            .find(|iv| matches!(iv.mark, Mark::Link(_)) && iv.start <= *at && *at < iv.end)
        {
            *at = l.end;
        }
    }
    prepared.anchors.sort_by_key(|(at, _)| *at);
    let Tokens { tokens, ends } = tokenize(&prepared, &ivs);

    let literal = |ch: char| {
        prepared
            .cells
            .iter()
            .filter(|c| !c.code && c.ch == ch)
            .count()
    };
    let has = |mark: &Mark| ivs.iter().any(|iv| iv.mark == *mark);
    let ctx = TextContext {
        stars: has(&Mark::Bold) || has(&Mark::Italic) || literal('*') > 1,
        underscores: if prepared.anchors.is_empty() {
            lone_underscores(&prepared.cells, &span_marks) > 1
        } else {
            literal('_') > 1
        },
        tildes: has(&Mark::Strike) || literal('~') > 1,
        // The `>` can also come from what the exporter writes itself: an
        // HTML tag, a comment or an autolink.
        // In a quote every further line starts with the quote
        // marker, which some parsers take for the end of a tag.
        angles: !ivs.is_empty()
            || !prepared.anchors.is_empty()
            || matches!(block.kind(), writer_core::BlockKind::Quote)
            || prepared.cells.iter().any(|c| c.ch == '>'),
        code_label: prepared
            .cells
            .windows(2)
            .any(|w| w[0].code && w[0].ch == ']' && w[1].code && w[1].ch == ':'),
    };

    // Replace delimiters a parser would not pair as intended with HTML
    // tags, one mark at a time, until the simulation agrees.
    let delimited = ivs
        .iter()
        .filter(|iv| iv.mark.delimiter().is_some())
        .count();
    let mut strings = render(&tokens, &ivs, options, heading, ctx);
    // After a few single replacements (a paragraph rarely needs more),
    // every mark that fails is replaced in one go: one at a time is
    // quadratic in a paragraph with thousands of marks.
    for round in 0..=delimited {
        let view = pieces(&tokens, &ivs, &strings);
        let bad = emphasis::mismatches(&view, &ends, round >= SINGLE_REPLACEMENTS);
        if bad.is_empty() {
            break;
        }
        for mark in bad {
            ivs[mark].html = true;
        }
        strings = render(&tokens, &ivs, options, heading, ctx);
    }
    strings.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_spans() {
        assert_eq!(code_span("x"), "`x`");
        assert_eq!(code_span("a`b"), "``a`b``");
        assert_eq!(code_span("`"), "`` ` ``");
        assert_eq!(code_span("``"), "``` `` ```");
        assert_eq!(code_span("a``b`c"), "```a``b`c```");
        assert_eq!(code_span(" x "), "`  x  `");
        assert_eq!(code_span(" x"), "` x`");
        assert_eq!(code_span("  "), "`  `");
        assert_eq!(code_span("x`"), "`` x` ``");
    }
}
