//! From the Markdown of an answer to the document an answer view shows.
//!
//! An answer arrives as Markdown, piece by piece. It is read with the
//! app's own importer (`writer_export::from_markdown`), so a heading, a
//! list or a quote in an answer is the same block it would be in a
//! document, and is drawn by the same editor (a read-only, formatted
//! compact one per answer, see [`crate::MessageList`]).
//!
//! While an answer is still being written its last line may hold the
//! first half of a pair: `**bold` without the closing stars, a code span
//! without its second backtick. [`settle`] closes those, so that the text
//! does not show raw markers for the moment it takes the other half to
//! arrive.

use std::borrow::Cow;

use writer_core::{Document, DocumentSettings, Typography};
use writer_export::{MarkdownImportOptions, MarkdownSoftBreaks, from_markdown};
use writer_ui::theme::UI_FONT_FAMILY;

/// Which font an answer is set in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AnswerFont {
    /// The font of documents (a serif made for reading): an answer is
    /// text, and reads as text.
    #[default]
    Document,
    /// The font of the interface.
    Interface,
}

/// How answers are set: font, size, line height, space between paragraphs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnswerStyle {
    /// The font.
    pub font: AnswerFont,
    /// Text size in pixels.
    pub size: f32,
    /// Line height as a multiple of the text size.
    pub line_height: f32,
    /// Space after a paragraph in pixels.
    pub paragraph_spacing: f32,
}

impl Default for AnswerStyle {
    /// The side chat: the document font at 15px, a little smaller than in
    /// a document (17px), for a column half as wide.
    fn default() -> Self {
        Self {
            font: AnswerFont::Document,
            size: 15.,
            line_height: 1.5,
            paragraph_spacing: 9.,
        }
    }
}

impl AnswerStyle {
    /// A smaller setting for a small surface (the quick chat): the
    /// document font at 14px.
    pub fn small() -> Self {
        Self {
            font: AnswerFont::Document,
            size: 14.,
            line_height: 1.45,
            paragraph_spacing: 7.,
        }
    }

    /// The typography of the documents answers are shown as.
    pub fn typography(&self) -> Typography {
        let defaults = Typography::default();
        Typography {
            body_font: match self.font {
                // The default body font goes through the theme's resolved
                // stack (Charter, or what is installed).
                AnswerFont::Document => defaults.body_font,
                AnswerFont::Interface => UI_FONT_FAMILY.to_string(),
            },
            base_size: self.size,
            line_height: self.line_height,
            paragraph_spacing: self.paragraph_spacing,
        }
        .sanitized()
    }
}

/// The document an answer view shows for `markdown`.
///
/// `streaming` says the answer is still being written: its unfinished last
/// line is settled first (see [`settle`]).
pub fn answer_document(markdown: &str, style: &AnswerStyle, streaming: bool) -> Document {
    let text = if streaming {
        settle(markdown)
    } else {
        Cow::Borrowed(markdown)
    };
    let options = MarkdownImportOptions {
        // A line end inside a paragraph is a space, as Markdown has it.
        soft_breaks: MarkdownSoftBreaks::Space,
        settings: DocumentSettings {
            typography: style.typography(),
            ..DocumentSettings::default()
        },
        ..MarkdownImportOptions::default()
    };
    from_markdown(text.trim_end(), &options).document
}

/// The plain words of an answer, without its Markdown: what a "copy as
/// text" puts on the clipboard.
pub fn answer_plain_text(markdown: &str) -> String {
    answer_document(markdown, &AnswerStyle::default(), false).plain_text()
}

/// Closes what the last line of a text that is still arriving has left
/// open: a `**` or `__` without its partner, a single `*`, `_` that began
/// emphasis, a backtick of a code span. A text inside an open code fence
/// is left alone (the importer shows it as code to the end, which is what
/// it will be).
///
/// Only the last line is looked at, and only markers that clearly opened
/// something: one that follows white space or starts the line and is
/// followed by a character that is not white space.
pub fn settle(markdown: &str) -> Cow<'_, str> {
    if in_open_fence(markdown) {
        return Cow::Borrowed(markdown);
    }
    let line_start = markdown.rfind('\n').map_or(0, |ix| ix + 1);
    let line = &markdown[line_start..];
    let mut closers: Vec<&'static str> = Vec::new();
    let mut in_code = false;
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut ix = 0;
    while ix < chars.len() {
        let (_, ch) = chars[ix];
        let next = chars.get(ix + 1).map(|(_, ch)| *ch);
        let previous = ix.checked_sub(1).map(|ix| chars[ix].1);
        if ch == '\\' {
            ix += 2;
            continue;
        }
        if ch == '`' {
            in_code = !in_code;
            ix += 1;
            continue;
        }
        if in_code {
            ix += 1;
            continue;
        }
        if ch == '*' || ch == '_' {
            let double = next == Some(ch);
            let marker: &'static str = match (ch, double) {
                ('*', true) => "**",
                ('*', false) => "*",
                ('_', true) => "__",
                _ => "_",
            };
            let width = if double { 2 } else { 1 };
            let after = chars.get(ix + width).map(|(_, ch)| *ch);
            if closers.last() == Some(&marker) {
                // The partner of the marker that is open.
                closers.pop();
            } else {
                // A marker opens something where a word begins: after
                // white space or an opening bracket or quote. One inside a
                // word (snake_case, 2*3) is left as it is, and so is a
                // star followed by a space (a list bullet, a product).
                let opens_word =
                    previous.is_none_or(|ch| ch.is_whitespace() || "([\"'“‘".contains(ch));
                let has_text = after.is_some_and(|ch| !ch.is_whitespace());
                if has_text && opens_word {
                    closers.push(marker);
                }
            }
            ix += width;
            continue;
        }
        ix += 1;
    }
    if !in_code && closers.is_empty() {
        return Cow::Borrowed(markdown);
    }
    let mut settled = String::with_capacity(markdown.len() + 8);
    settled.push_str(markdown.trim_end_matches([' ', '\t']));
    if in_code {
        settled.push('`');
    }
    for closer in closers.iter().rev() {
        settled.push_str(closer);
    }
    Cow::Owned(settled)
}

/// Whether the text ends inside a fenced code block that was not closed.
fn in_open_fence(markdown: &str) -> bool {
    let mut open: Option<(char, usize)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        // A fence is indented by at most three spaces.
        if line.len() - trimmed.len() > 3 {
            continue;
        }
        let Some(marker) = trimmed.chars().next().filter(|ch| *ch == '`' || *ch == '~') else {
            continue;
        };
        let run = trimmed.chars().take_while(|ch| *ch == marker).count();
        if run < 3 {
            continue;
        }
        match open {
            None => open = Some((marker, run)),
            Some((open_marker, open_run)) => {
                let rest = trimmed[run * marker.len_utf8()..].trim();
                if marker == open_marker && run >= open_run && rest.is_empty() {
                    open = None;
                }
            }
        }
    }
    open.is_some()
}
