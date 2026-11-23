//! The drawings of answers (`docs/assistant-draws-design.md`, sections 2 and 3): a fenced
//! block with the info string `whiteboard` holds one drawing in the agents' formats,
//! found here ([`drawing_blocks`]), read with `innerword_board`'s own types
//! ([`answer_drawing`]), and shown in the side chat as one line the app writes
//! ([`shown_markdown`], [`drawing_sentence`]), never as its JSON.
//!
//! What a drawing can be applied to follows the open item ([`DrawingTarget`]): the open
//! whiteboard, or a new whiteboard beside the open document.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Mutex;

pub use innerword_board::{Drawing, DrawingError};

use crate::context::OpenKind;

/// The info string of the fenced block that holds an answer's drawing.
pub const DRAWING_INFO: &str = "whiteboard";

/// What drawings a side chat offers, by its open item (section 3.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DrawingTarget {
    /// None: the open item is a page, nothing is open, or whiteboards cannot be shown.
    #[default]
    None,
    /// The open item is a whiteboard: a drawing adds to it, changes and removes elements.
    OpenWhiteboard,
    /// The open item is a document: a drawing that only adds makes a new whiteboard.
    NewWhiteboard,
}

impl DrawingTarget {
    /// The target for an open item of `kind`, where `boards` says whether whiteboards can
    /// be shown (the app has a page engine).
    pub fn of(kind: Option<&OpenKind>, boards: bool) -> DrawingTarget {
        if !boards {
            return DrawingTarget::None;
        }
        match kind {
            Some(OpenKind::Board) => DrawingTarget::OpenWhiteboard,
            Some(OpenKind::Text) => DrawingTarget::NewWhiteboard,
            Some(OpenKind::Page { .. }) | None => DrawingTarget::None,
        }
    }
}

/// One fenced drawing block of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingBlock {
    /// Its bytes in the answer's Markdown: from the start of the opening fence's line to
    /// the end of the closing fence's line (its line end included), or to the end of the
    /// text when it is not closed.
    pub range: Range<usize>,
    /// The text between the fence lines.
    pub body: String,
    /// Whether its closing fence came.
    pub closed: bool,
}

/// An opening fence: its character, its length and whether its info string's first word
/// is [`DRAWING_INFO`].
fn opening_fence(line: &str) -> Option<(u8, usize, bool)> {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let mark = *rest.as_bytes().first()?;
    if mark != b'`' && mark != b'~' {
        return None;
    }
    let run = rest.bytes().take_while(|b| *b == mark).count();
    if run < 3 {
        return None;
    }
    let info = &rest[run..];
    if mark == b'`' && info.contains('`') {
        return None;
    }
    let word = info.split_whitespace().next().unwrap_or("");
    Some((mark, run, word.eq_ignore_ascii_case(DRAWING_INFO)))
}

/// Whether `line` closes a fence of `mark` at least `length` long.
fn closes(line: &str, mark: u8, length: usize) -> bool {
    let line = line.trim_end_matches(['\n', '\r']);
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    let run = rest.bytes().take_while(|b| *b == mark).count();
    run >= length && rest[run..].trim().is_empty()
}

/// The drawing blocks of an answer, in order (section 2.1): a fence of three or more
/// backticks or tildes, indented at most three spaces, whose info string's first word is
/// `whiteboard` (letters compared without case), up to a closing fence of the same
/// character at least as long. A `whiteboard` fence inside another fenced block is text of
/// that block.
pub fn drawing_blocks(markdown: &str) -> Vec<DrawingBlock> {
    let mut blocks = Vec::new();
    // The fence that is open: its mark, its length, and where the block and its body
    // start when it is a drawing.
    type Open = (u8, usize, Option<(usize, usize)>);
    let mut open: Option<Open> = None;
    let mut at = 0usize;
    for line in markdown.split_inclusive('\n') {
        let start = at;
        at += line.len();
        match open {
            None => {
                if let Some((mark, length, drawing)) = opening_fence(line) {
                    open = Some((mark, length, drawing.then_some((start, at))));
                }
            }
            Some((mark, length, drawing)) => {
                if closes(line, mark, length) {
                    if let Some((block_start, body_start)) = drawing {
                        blocks.push(DrawingBlock {
                            range: block_start..at,
                            body: markdown[body_start..start].to_string(),
                            closed: true,
                        });
                    }
                    open = None;
                }
            }
        }
    }
    if let Some((_, _, Some((block_start, body_start)))) = open {
        blocks.push(DrawingBlock {
            range: block_start..markdown.len(),
            body: markdown[body_start.min(markdown.len())..].to_string(),
            closed: false,
        });
    }
    blocks
}

/// Why an answer's drawing cannot be used (here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingProblem {
    /// A block was not closed (`block` from 0, of `of` blocks).
    NotFinished { block: usize, of: usize },
    /// A block cannot be read, or the blocks joined are over a limit (`block` `None`).
    Unreadable {
        block: Option<usize>,
        of: usize,
        error: DrawingError,
    },
    /// Over a document: it changes or removes elements.
    ChangesAWhiteboard,
    /// No drawing can be applied over the open item.
    NotHere,
}

/// "first", "second", … "12th".
fn ordinal(n: usize) -> String {
    const WORDS: [&str; 12] = [
        "first", "second", "third", "fourth", "fifth", "sixth", "seventh", "eighth", "ninth",
        "tenth", "eleventh", "twelfth",
    ];
    if (1..=12).contains(&n) {
        return WORDS[n - 1].to_string();
    }
    let suffix = match (n % 10, n % 100) {
        (1, x) if x != 11 => "st",
        (2, x) if x != 12 => "nd",
        (3, x) if x != 13 => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

/// A clause about one block of several: "it is not JSON" becomes "its second block is not
/// JSON"; another clause is said "in its second block".
fn of_block(clause: &str, block: usize, of: usize) -> String {
    if of <= 1 {
        return clause.to_string();
    }
    let which = format!("its {} block", ordinal(block + 1));
    match clause.strip_prefix("it ") {
        Some(rest) => format!("{which} {rest}"),
        None => format!("in {which}, {clause}"),
    }
}

impl DrawingProblem {
    /// The clause of section 2.4 (no capital, no period), with the block's ordinal when
    /// the answer has several.
    pub fn reason(&self) -> String {
        match self {
            DrawingProblem::NotFinished { block, of } => {
                of_block("it was not finished", *block, *of)
            }
            DrawingProblem::Unreadable {
                block: Some(block),
                of,
                error,
            } => of_block(&error.to_string(), *block, *of),
            DrawingProblem::Unreadable {
                block: None, error, ..
            } => error.to_string(),
            DrawingProblem::ChangesAWhiteboard => {
                "it changes elements of a whiteboard: open that whiteboard and ask there".into()
            }
            DrawingProblem::NotHere => {
                "a drawing can be used only over a whiteboard or a document".into()
            }
        }
    }
}

/// How many closed blocks [`answer_drawing`] remembers.
const CACHE_ENTRIES: usize = 32;

type Read = Result<Drawing, DrawingError>;

/// What `Drawing::from_json` made of the bodies of closed blocks, by the hash of the body
/// (and its length); the oldest goes first.
struct Cache {
    entries: HashMap<(u64, usize), Read>,
    order: VecDeque<(u64, usize)>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

#[cfg(test)]
thread_local! {
    /// How many bodies this thread read with `Drawing::from_json` (the cache's test).
    static PARSED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The drawing of a closed block's body, read once while it stays among the last
/// [`CACHE_ENTRIES`] read.
pub(crate) fn read_block(body: &str) -> Read {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    body.hash(&mut hasher);
    let key = (hasher.finish(), body.len());
    if let Ok(cache) = CACHE.lock()
        && let Some(read) = cache.as_ref().and_then(|cache| cache.entries.get(&key))
    {
        return read.clone();
    }
    #[cfg(test)]
    PARSED.with(|parsed| parsed.set(parsed.get() + 1));
    let read = Drawing::from_json(body);
    if let Ok(mut cache) = CACHE.lock() {
        let cache = cache.get_or_insert_with(|| Cache {
            entries: HashMap::new(),
            order: VecDeque::new(),
        });
        if cache.entries.insert(key, read.clone()).is_none() {
            cache.order.push_back(key);
        }
        while cache.order.len() > CACHE_ENTRIES {
            if let Some(oldest) = cache.order.pop_front() {
                cache.entries.remove(&oldest);
            }
        }
    }
    read
}

/// The drawing of an answer: `None` when it has no block; else its blocks read and
/// joined (section 2.2), or the first problem. An open block is never read.
pub fn answer_drawing(markdown: &str) -> Option<Result<Drawing, DrawingProblem>> {
    let blocks = drawing_blocks(markdown);
    if blocks.is_empty() {
        return None;
    }
    let of = blocks.len();
    let mut drawings = Vec::with_capacity(of);
    for (n, block) in blocks.iter().enumerate() {
        if !block.closed {
            return Some(Err(DrawingProblem::NotFinished { block: n, of }));
        }
        match read_block(&block.body) {
            Ok(drawing) => drawings.push(drawing),
            Err(error) => {
                return Some(Err(DrawingProblem::Unreadable {
                    block: Some(n),
                    of,
                    error,
                }));
            }
        }
    }
    if drawings.len() == 1 {
        return drawings.pop().map(Ok);
    }
    Some(
        Drawing::join(drawings).map_err(|error| DrawingProblem::Unreadable {
            block: None,
            of,
            error,
        }),
    )
}

/// Whether `drawing` can be applied for `target`: `NotHere` for `None`,
/// `ChangesAWhiteboard` for `NewWhiteboard` and a drawing with changes.
pub fn drawing_fits(drawing: &Drawing, target: DrawingTarget) -> Result<(), DrawingProblem> {
    match target {
        DrawingTarget::None => Err(DrawingProblem::NotHere),
        DrawingTarget::NewWhiteboard if drawing.has_changes() => {
            Err(DrawingProblem::ChangesAWhiteboard)
        }
        _ => Ok(()),
    }
}

/// Whether a line holds nothing but white space.
fn is_blank(line: &str) -> bool {
    line.trim().is_empty()
}

/// `text` without the blank lines at its end (the line end of its last line kept).
fn without_blank_end(text: &str) -> &str {
    let mut end = text.len();
    loop {
        let head = &text[..end];
        if head.is_empty() {
            return head;
        }
        let body = head.strip_suffix('\n').unwrap_or(head);
        let line_start = body.rfind('\n').map_or(0, |i| i + 1);
        if is_blank(&body[line_start..]) {
            end = line_start;
        } else {
            return head;
        }
    }
}

/// `text` without the blank lines at its start.
fn without_blank_start(text: &str) -> &str {
    let mut start = 0;
    for line in text.split_inclusive('\n') {
        if is_blank(line) {
            start += line.len();
        } else {
            break;
        }
    }
    &text[start..]
}

/// Puts `between` in place of each block's range, the blank lines around each made part
/// of one paragraph break: `between` is `None` to leave the block out.
pub(crate) fn replace_blocks<'a>(
    markdown: &'a str,
    blocks: &[DrawingBlock],
    mut between: impl FnMut(usize, &DrawingBlock) -> Option<String>,
) -> String {
    let mut pieces: Vec<Cow<'a, str>> = Vec::new();
    let mut at = 0;
    for (n, block) in blocks.iter().enumerate() {
        pieces.push(Cow::Borrowed(&markdown[at..block.range.start]));
        if let Some(text) = between(n, block) {
            pieces.push(Cow::Owned(text));
        }
        at = block.range.end;
    }
    pieces.push(Cow::Borrowed(&markdown[at..]));
    let mut out = String::with_capacity(markdown.len());
    for piece in pieces {
        let piece = without_blank_end(without_blank_start(&piece)).to_string();
        let piece = piece.trim_end_matches(['\n', '\r']);
        if piece.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(piece);
    }
    out
}

/// The answer without its drawing blocks, what Insert, Replace and Copy take (the blank
/// lines around a block made one).
pub fn without_drawings(markdown: &str) -> String {
    let blocks = drawing_blocks(markdown);
    if blocks.is_empty() {
        return markdown.to_string();
    }
    replace_blocks(markdown, &blocks, |_, _| None)
}

/// How an element of the open whiteboard is named in the line (section 2.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementName {
    /// Its type as the listing says it ("rectangle", "arrow", "image", …).
    pub kind: String,
    /// Its text, or its label's, when it has one.
    pub text: Option<String>,
}

/// The names of the open whiteboard's elements, given by the host (the app answers from
/// the scene it holds). `None` for an id it does not have.
pub trait ElementNames {
    fn name(&self, id: &str) -> Option<ElementName>;
}

/// The last line of a text being written, when it has not ended and could still become
/// an opening fence: at most three spaces, then backticks or tildes, all one character,
/// and the start of an info string. Its start, to leave it out until it ends.
fn unfinished_fence_start(markdown: &str) -> Option<usize> {
    if markdown.is_empty() || markdown.ends_with('\n') {
        return None;
    }
    let start = markdown.rfind('\n').map_or(0, |i| i + 1);
    let line = &markdown[start..];
    let indent = line.bytes().take_while(|b| *b == b' ').count();
    if indent > 3 {
        return None;
    }
    let rest = &line[indent..];
    let mark = *rest.as_bytes().first()?;
    if mark != b'`' && mark != b'~' {
        return None;
    }
    let run = rest.bytes().take_while(|b| *b == mark).count();
    let info = &rest[run..];
    if run < 3 {
        return info.is_empty().then_some(start);
    }
    if mark == b'`' && info.contains('`') {
        return None;
    }
    // Only a fence that could still say `whiteboard` is held back.
    let word = info.trim_start();
    let first: String = word
        .chars()
        .take_while(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    let could = if word.contains(char::is_whitespace) && !first.is_empty() {
        first == DRAWING_INFO
    } else {
        DRAWING_INFO.starts_with(&first) || first == DRAWING_INFO
    };
    could.then_some(start)
}

/// The answer as the side chat shows it (section 2.3): each block replaced by one line in
/// italics, the unfinished last line that could be a fence left out while `streaming`.
/// Borrowed when there is no block and nothing is left out.
pub fn shown_markdown<'a>(
    markdown: &'a str,
    streaming: bool,
    target: DrawingTarget,
    names: Option<&dyn ElementNames>,
) -> Cow<'a, str> {
    let text = match streaming
        .then(|| unfinished_fence_start(markdown))
        .flatten()
    {
        Some(start) => &markdown[..start],
        None => markdown,
    };
    let blocks = drawing_blocks(text);
    if blocks.is_empty() {
        return if text.len() == markdown.len() {
            Cow::Borrowed(markdown)
        } else {
            Cow::Borrowed(text)
        };
    }
    Cow::Owned(replace_blocks(text, &blocks, |_, block| {
        let line = if !block.closed {
            if streaming {
                "Drawing…".to_string()
            } else {
                format!(
                    "Drawing that cannot be used: {}.",
                    escape_markdown("it was not finished")
                )
            }
        } else {
            match read_block(&block.body) {
                Ok(drawing) => drawing_sentence(&drawing, target, names),
                Err(error) => format!(
                    "Drawing that cannot be used: {}.",
                    escape_markdown(&error.to_string())
                ),
            }
        };
        Some(format!("*{line}*"))
    }))
}

/// The noun of an added type, for one and for many.
fn noun(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "rectangle" => ("a rectangle", "rectangles"),
        "ellipse" => ("an ellipse", "ellipses"),
        "diamond" => ("a diamond", "diamonds"),
        "text" => ("a text", "texts"),
        "arrow" => ("an arrow", "arrows"),
        "line" => ("a line", "lines"),
        "freedraw" => ("a drawn line", "drawn lines"),
        "frame" => ("a frame", "frames"),
        _ => ("an element", "elements"),
    }
}

/// "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [head @ .., last] => format!("{} and {last}", head.join(", ")),
    }
}

/// What a drawing adds: "4 rectangles, 3 arrows and a frame".
fn added_words(drawing: &Drawing) -> Option<String> {
    let parts: Vec<String> = drawing
        .added_types()
        .into_iter()
        .map(|(kind, count)| {
            let (one, many) = noun(kind);
            if count == 1 {
                one.to_string()
            } else {
                format!("{count} {many}")
            }
        })
        .collect();
    (!parts.is_empty()).then(|| and_list(&parts))
}

/// A text the model wrote, one line, cut at `max` characters.
fn one_line(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        joined
    } else {
        let mut out: String = joined.chars().take(max).collect();
        out.push('…');
        out
    }
}

/// An element as the line names it: "the rectangle “API”", "a picture", "an arrow", or
/// its id, escaped.
fn element_words(id: &str, names: Option<&dyn ElementNames>) -> String {
    let Some(name) = names.and_then(|names| names.name(id)) else {
        return escape_markdown(id);
    };
    let kind = name.kind.as_str();
    if kind == "image" {
        return "a picture".into();
    }
    let (one, _) = match kind {
        "rectangle" | "ellipse" | "diamond" | "text" | "arrow" | "line" | "freedraw" | "frame" => {
            noun(kind)
        }
        _ => ("an element", "elements"),
    };
    match name.text.as_deref().map(|text| one_line(text, 40)) {
        Some(text) if !text.is_empty() => {
            let bare = one.split_once(' ').map_or(one, |(_, rest)| rest);
            format!("the {bare} “{}”", escape_markdown(&text))
        }
        _ => one.to_string(),
    }
}

/// Up to three elements, then "and N more"; "7 elements (api, db, cache and 4 more)".
fn elements_words(ids: &[String], names: Option<&dyn ElementNames>) -> String {
    let named: Vec<String> = ids
        .iter()
        .take(3)
        .map(|id| element_words(id, names))
        .collect();
    if ids.len() <= 3 {
        return and_list(&named);
    }
    format!(
        "{} elements ({}, {} and {} more)",
        ids.len(),
        named[0],
        named[1..].join(", "),
        ids.len() - 3
    )
}

/// An address by its host (`mailto:` by its address), lowercased, without a port or a
/// user.
fn host_of(link: &str) -> String {
    let lower = link.trim();
    if let Some(rest) = lower
        .get(..7)
        .filter(|head| head.eq_ignore_ascii_case("mailto:"))
        .map(|_| &lower[7..])
    {
        return rest.split(['?', '#']).next().unwrap_or("").to_lowercase();
    }
    let rest = lower.split_once("://").map_or(lower, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let host = match host.rsplit_once(':') {
        Some((name, port)) if port.bytes().all(|b| b.is_ascii_digit()) && !name.is_empty() => name,
        _ => host,
    };
    host.to_lowercase()
}

/// The parts of a drawing in words, in the present (`past` false) or the past, with the
/// names of the open whiteboard's elements when given; nothing escaped but what the model
/// wrote.
fn parts(drawing: &Drawing, past: bool, names: Option<&dyn ElementNames>) -> Vec<String> {
    let verb = |present: &str, past_form: &str| if past { past_form } else { present }.to_string();
    let mut out = Vec::new();
    if let Some(added) = added_words(drawing) {
        out.push(format!("{} {added}", verb("adds", "added")));
    }
    let changed = drawing.changed_ids();
    if !changed.is_empty() {
        out.push(format!(
            "{} {}",
            verb("changes", "changed"),
            elements_words(&changed, names)
        ));
    }
    let removed = drawing.removed_ids();
    if !removed.is_empty() {
        out.push(format!(
            "{} {}",
            verb("removes", "removed"),
            elements_words(&removed, names)
        ));
    }
    let (set, unset) = drawing.links();
    let mut hosts: Vec<String> = Vec::new();
    for link in &set {
        let host = host_of(link);
        if !host.is_empty() && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    if !hosts.is_empty() {
        let shown: Vec<String> = hosts.iter().take(3).map(|h| escape_markdown(h)).collect();
        let list = if hosts.len() <= 3 {
            and_list(&shown)
        } else {
            format!("{} and {} more", shown.join(", "), hosts.len() - 3)
        };
        out.push(format!("{} {list}", verb("links to", "linked to")));
    }
    if !unset.is_empty() {
        let noun = if unset.len() == 1 { "link" } else { "links" };
        out.push(format!(
            "{} the {noun} of {}",
            verb("removes", "removed"),
            elements_words(&unset, names)
        ));
    }
    out
}

/// The summary of a drawing (section 2.3), already escaped Markdown, without the italics
/// around it: "Drawing: adds an ellipse and an arrow; changes the rectangle “API”." The
/// counts come first; the title only for [`DrawingTarget::NewWhiteboard`].
pub fn drawing_sentence(
    drawing: &Drawing,
    target: DrawingTarget,
    names: Option<&dyn ElementNames>,
) -> String {
    let parts = parts(drawing, false, names);
    let mut out = String::from("Drawing: ");
    if parts.is_empty() {
        out.push_str("nothing");
    } else {
        out.push_str(&parts.join("; "));
    }
    if target == DrawingTarget::NewWhiteboard
        && let Some(title) = &drawing.title
    {
        out.push_str(&format!(
            " (new whiteboard “{}”)",
            escape_markdown(&one_line(title, 120))
        ));
    }
    out.push('.');
    out
}

/// The note an answer's drawing becomes in the history (section 3.5): "[Inner Word
/// replaced a whiteboard block here: it added an ellipse and an arrow and changed api.]"
pub fn drawing_history_note(drawing: &Drawing) -> String {
    let parts = parts(drawing, true, None);
    let mut what = if parts.is_empty() {
        "it held nothing".to_string()
    } else {
        format!("it {}", and_list(&parts))
    };
    if let Some(title) = &drawing.title {
        what.push_str(&format!(", titled “{}”", one_line(title, 120)));
    }
    format!("[Inner Word replaced a whiteboard block here: {what}.]")
}

/// The characters [`escape_markdown`] puts a backslash before.
const ESCAPED: &str = "\\`*_{}[]()#+-.!|<>~\"";

/// Markdown made plain text: a backslash before every character the importer reads as
/// formatting, line ends made spaces (section 2.3).
pub fn escape_markdown(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(' ');
            }
            '\n' => out.push(' '),
            c if ESCAPED.contains(c) => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A closed block is read once across many calls: the side chat's views ask again
    /// after every delta of a streamed answer.
    #[test]
    fn a_closed_block_is_read_once() {
        let unique = format!(
            "Here.\n\n```whiteboard\n{{\"remove\": [\"cache-test-{}\"]}}\n```\n",
            std::process::id()
        );
        let before = PARSED.with(|parsed| parsed.get());
        for n in 0..50 {
            let mut text = unique.clone();
            text.push_str(&"more ".repeat(n));
            let drawing = answer_drawing(&text).unwrap().unwrap();
            assert_eq!(drawing.remove.len(), 1);
        }
        assert_eq!(PARSED.with(|parsed| parsed.get()) - before, 1);
    }

    #[test]
    fn ordinals() {
        assert_eq!(ordinal(1), "first");
        assert_eq!(ordinal(12), "twelfth");
        assert_eq!(ordinal(13), "13th");
        assert_eq!(ordinal(21), "21st");
        assert_eq!(ordinal(22), "22nd");
        assert_eq!(ordinal(111), "111th");
    }

    #[test]
    fn hosts() {
        assert_eq!(host_of("https://Example.com/a?b"), "example.com");
        assert_eq!(host_of("http://user:pw@a.org:8080/"), "a.org");
        assert_eq!(
            host_of("mailto:Ada@Example.org?subject=x"),
            "ada@example.org"
        );
    }
}
