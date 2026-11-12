//! What the assistant is sent with a question: pure functions from the question and its
//! context to the text of the message.
//!
//! * The quick chat sends the selected text and the question ([`quick_prompt`]), or the
//!   question alone, exactly as typed, when nothing is selected.
//! * The side chat sends the open document as Markdown with its title and library path,
//!   the list of the library items the user attached, and the question
//!   ([`side_prompt`]). The document is sent again only when it changed since the last
//!   message, and a document too long to send whole is sent as an outline that the
//!   assistant reads from with the tools. [`SideContext`] remembers what was sent.
//! * A conversation that continues an earlier one in a new process of the agent begins
//!   with what was said before ([`with_history`]).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::backend::{HistoryMessage, Role};
use crate::transcript::{Attachment, AttachmentKind};

/// The longest document that is sent whole, in characters of Markdown (about 30,000
/// words, a long chapter). A longer one is sent as an outline; the assistant reads the
/// parts it needs with the tools.
pub const WHOLE_DOCUMENT_LIMIT: usize = 180_000;

/// The longest selection the quick chat sends, in characters. A longer one is cut in
/// the middle and says so.
pub const SELECTION_LIMIT: usize = 60_000;

/// The most characters of earlier conversation that are carried into a new process.
pub const HISTORY_LIMIT: usize = 60_000;

/// The most entries of an outline.
const OUTLINE_LIMIT: usize = 400;

/// The message of the quick chat: the selected text, marked as the text the question is
/// about, and the question; with nothing selected, the question as it was typed.
///
/// ```
/// use writer_assistant::quick_prompt;
///
/// assert_eq!(quick_prompt("A word for very tired?", None), "A word for very tired?");
/// let prompt = quick_prompt("Make it shorter.", Some("The rain, which had been falling, fell."));
/// assert!(prompt.contains("<selected_text>\nThe rain, which had been falling, fell.\n</selected_text>"));
/// assert!(prompt.ends_with("Make it shorter."));
/// ```
pub fn quick_prompt(question: &str, selection: Option<&str>) -> String {
    let question = question.trim();
    let Some(selection) = selection.filter(|selection| !selection.trim().is_empty()) else {
        return question.to_string();
    };
    let selection = cut_in_the_middle(selection.trim_matches('\n'), SELECTION_LIMIT);
    format!(
        "This is the text I have selected in my document. My request below is about this \
         text.\n\n<selected_text>\n{selection}\n</selected_text>\n\n{question}"
    )
}

/// The document that is open beside the side chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenDocument {
    /// Its id in the library, when it has one.
    pub id: Option<i64>,
    /// Its title.
    pub title: String,
    /// Its path in the library: "The Salt Road/Part one/Chapter 1".
    pub path: String,
    /// Its text as Markdown, as it is now.
    pub markdown: String,
}

impl OpenDocument {
    /// The id as the tools take it (`doc:12`), when the document has one.
    pub fn reference(&self) -> Option<String> {
        self.id.map(|id| format!("doc:{id}"))
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.id.hash(&mut hasher);
        self.title.hash(&mut hasher);
        self.path.hash(&mut hasher);
        self.markdown.hash(&mut hasher);
        hasher.finish()
    }
}

/// How the open document goes into a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DocumentPart<'a> {
    /// There is no open document, or it is not to be mentioned.
    None,
    /// The document as it is now: whole, or as an outline when it is too long.
    Current(&'a OpenDocument),
    /// The document was sent before and has not changed: one line says so.
    Unchanged(&'a OpenDocument),
}

/// How the attached items go into a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachedPart<'a> {
    /// Nothing to say: nothing is attached and nothing was, or the list was sent before
    /// and has not changed.
    None,
    /// The list as it is now (an empty list says that nothing is attached any more).
    Current(&'a [Attachment]),
}

/// The message of the side chat: the open document, the attached items, the question.
/// `tools` says whether the assistant has the tools of the library; without them a
/// document too long to send whole is sent as its beginning and its end.
pub fn side_prompt(
    question: &str,
    document: DocumentPart<'_>,
    attached: AttachedPart<'_>,
    tools: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    match document {
        DocumentPart::None => {}
        DocumentPart::Current(document) => parts.push(document_block(document, tools)),
        DocumentPart::Unchanged(document) => parts.push(format!(
            "(The open document, \"{}\", has not changed since my last message.)",
            document.title
        )),
    }
    match attached {
        AttachedPart::None => {}
        AttachedPart::Current([]) => {
            parts.push("(Nothing from the library is attached any more.)".to_string())
        }
        AttachedPart::Current(attachments) => parts.push(attached_block(attachments, tools)),
    }
    if parts.is_empty() {
        return question.trim().to_string();
    }
    parts.push(question.trim().to_string());
    parts.join("\n\n")
}

/// The open document as a block of the message.
fn document_block(document: &OpenDocument, tools: bool) -> String {
    let mut header = format!("<open_document title={}", attribute(&document.title));
    if !document.path.is_empty() {
        header.push_str(&format!(" path={}", attribute(&document.path)));
    }
    if let Some(reference) = document.reference() {
        header.push_str(&format!(" id={}", attribute(&reference)));
    }
    let markdown = document.markdown.trim_matches('\n');
    let chars = markdown.chars().count();
    if chars <= WHOLE_DOCUMENT_LIMIT {
        let body = if markdown.trim().is_empty() {
            "(The document is empty.)"
        } else {
            markdown
        };
        return format!(
            "This is the document I have open, as it is now.\n\n{header}>\n{body}\n</open_document>"
        );
    }
    let words = markdown.split_whitespace().count();
    header.push_str(&format!(" words=\"{words}\" whole=\"false\""));
    let outline = outline(markdown);
    let mut body = String::new();
    if outline.is_empty() {
        body.push_str("(The document has no headings.)\n");
    } else {
        for entry in outline.iter().take(OUTLINE_LIMIT) {
            body.push_str(&format!(
                "{} {} ({} words)\n",
                "#".repeat(entry.level as usize),
                entry.text,
                entry.words
            ));
        }
        if outline.len() > OUTLINE_LIMIT {
            body.push_str(&format!(
                "(and {} more headings)\n",
                outline.len() - OUTLINE_LIMIT
            ));
        }
    }
    match (tools, document.reference()) {
        (true, Some(reference)) => format!(
            "This is the document I have open. It is too long to include whole ({words} \
             words), so here is its outline: every heading with the number of words under \
             it. Read the parts you need with the read_document tool (document \
             \"{reference}\", heading: a heading of the outline) and look for a phrase with \
             find_in_document before you answer about the text.\n\n{header}>\n{body}</open_document>"
        ),
        _ => {
            let edge = WHOLE_DOCUMENT_LIMIT / 2;
            let beginning: String = markdown.chars().take(edge).collect();
            let end: String = markdown.chars().skip(chars - edge).collect();
            format!(
                "This is the document I have open. It is too long to include whole ({words} \
                 words): here are its outline, its beginning and its end. Say so when a \
                 question needs a part you cannot see.\n\n{header}>\n<outline>\n{body}</outline>\n\
                 <beginning>\n{beginning}\n</beginning>\n<end>\n{end}\n</end>\n</open_document>"
            )
        }
    }
}

/// The attached items as a block of the message.
fn attached_block(attachments: &[Attachment], tools: bool) -> String {
    let mut block = String::from(if tools {
        "I have attached these items of my library as context. Read what you need of them \
         with the tools (read_document for a document, list_library for the documents of a \
         folder, search_library with the folder to search inside one) before you answer.\n\n\
         <attached>\n"
    } else {
        "I have attached these items of my library as context, but you cannot read them \
         now: say so if the answer needs them.\n\n<attached>\n"
    });
    for attachment in attachments {
        let kind = match attachment.kind {
            AttachmentKind::Document => "document",
            AttachmentKind::Folder => "folder",
        };
        let path = if attachment.path.is_empty() {
            &attachment.title
        } else {
            &attachment.path
        };
        block.push_str(&format!(
            "- {kind} {} {}\n",
            attachment.reference(),
            attribute(path)
        ));
    }
    block.push_str("</attached>");
    block
}

/// A value in double quotes, on one line.
fn attribute(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for ch in value.chars() {
        match ch {
            '"' => quoted.push('\''),
            '\n' | '\r' | '\t' => quoted.push(' '),
            ch => quoted.push(ch),
        }
    }
    quoted.push('"');
    quoted
}

/// `text` with its middle taken out when it is longer than `limit` characters.
fn cut_in_the_middle(text: &str, limit: usize) -> String {
    let chars = text.chars().count();
    if chars <= limit {
        return text.to_string();
    }
    let edge = limit / 2;
    let beginning: String = text.chars().take(edge).collect();
    let end: String = text.chars().skip(chars - edge).collect();
    format!(
        "{beginning}\n\n[... {} characters in the middle are left out ...]\n\n{end}",
        chars - 2 * edge
    )
}

/// A heading of a document and how much stands under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineEntry {
    /// 1 for `#`, up to 6.
    pub level: u8,
    /// The heading's text, without the marks.
    pub text: String,
    /// The words between this heading and the next one of any level.
    pub words: usize,
}

/// The headings of a Markdown text (`#` to `######` at the start of a line, outside
/// fenced code), each with the number of words that follow it up to the next heading.
pub fn outline(markdown: &str) -> Vec<OutlineEntry> {
    let mut entries: Vec<OutlineEntry> = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent <= 3
            && let Some(mark) = trimmed.chars().next().filter(|c| *c == '`' || *c == '~')
        {
            let run = trimmed.chars().take_while(|c| *c == mark).count();
            if run >= 3 {
                match fence {
                    None => {
                        fence = Some((mark, run));
                        continue;
                    }
                    Some((open, length))
                        if open == mark && run >= length && trimmed[run..].trim().is_empty() =>
                    {
                        fence = None;
                        continue;
                    }
                    Some(_) => {}
                }
            }
        }
        if fence.is_none() && indent <= 3 {
            let level = trimmed.chars().take_while(|c| *c == '#').count();
            let rest = &trimmed[level..];
            if (1..=6).contains(&level) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
                let text = rest.trim().trim_end_matches('#').trim();
                if !text.is_empty() {
                    entries.push(OutlineEntry {
                        level: level as u8,
                        text: text.to_string(),
                        words: 0,
                    });
                    continue;
                }
            }
        }
        if let Some(entry) = entries.last_mut() {
            entry.words += line.split_whitespace().count();
        }
    }
    entries
}

/// Remembers what the side chat has sent in the conversation that is running, so that
/// the document and the list of attached items are sent again only when they changed.
///
/// [`ChatSession`](crate::ChatSession) keeps one and resets it whenever the agent's
/// process is started anew (a new process knows nothing of what was sent before).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SideContext {
    document: Option<u64>,
    attached: Option<Vec<Attachment>>,
}

impl SideContext {
    /// Nothing was sent yet.
    pub fn new() -> Self {
        SideContext::default()
    }

    /// Forgets what was sent: the next message carries the document and the list again.
    pub fn reset(&mut self) {
        *self = SideContext::default();
    }

    /// The message for `question` with what has to be sent of the context, and
    /// remembers it as sent.
    pub fn compose(
        &mut self,
        question: &str,
        document: Option<&OpenDocument>,
        attachments: &[Attachment],
        tools: bool,
    ) -> String {
        let document_part = match document {
            None => {
                self.document = None;
                DocumentPart::None
            }
            Some(document) => {
                let fingerprint = document.fingerprint();
                if self.document == Some(fingerprint) {
                    DocumentPart::Unchanged(document)
                } else {
                    self.document = Some(fingerprint);
                    DocumentPart::Current(document)
                }
            }
        };
        let attached_part = match &self.attached {
            Some(sent) if sent.as_slice() == attachments => AttachedPart::None,
            None if attachments.is_empty() => AttachedPart::None,
            _ => {
                self.attached = Some(attachments.to_vec());
                AttachedPart::Current(attachments)
            }
        };
        side_prompt(question, document_part, attached_part, tools)
    }
}

/// The first message of a conversation that continues an earlier one in a new process:
/// what was said before, then `prompt`. With no history it is `prompt` itself.
///
/// The history is what the user saw: the questions as typed and the answers. The open
/// document is not part of it; the side chat sends it again with `prompt`.
pub fn with_history(history: &[HistoryMessage], prompt: &str) -> String {
    let spoken: Vec<&HistoryMessage> = history
        .iter()
        .filter(|message| !message.text.trim().is_empty())
        .collect();
    if spoken.is_empty() {
        return prompt.to_string();
    }
    let mut text = String::from(
        "We are in the middle of a conversation. This is what was said so far; continue \
         from it, and do not repeat or summarise it.\n\n<conversation_so_far>\n",
    );
    for message in spoken {
        let (open, close) = match message.role {
            Role::User => ("<me>", "</me>"),
            Role::Assistant => ("<you>", "</you>"),
        };
        text.push_str(&format!("{open}\n{}\n{close}\n", message.text.trim()));
    }
    text.push_str("</conversation_so_far>\n\nMy next message:\n\n");
    text.push_str(prompt);
    text
}

/// The most recent messages of `history` that fit in `max_chars` characters, beginning
/// with a question.
pub fn bounded_history(history: &[HistoryMessage], max_chars: usize) -> Vec<HistoryMessage> {
    let mut kept: Vec<HistoryMessage> = Vec::new();
    let mut used = 0usize;
    for message in history.iter().rev() {
        let size = message.text.chars().count();
        if used + size > max_chars && !kept.is_empty() {
            break;
        }
        used += size;
        kept.push(message.clone());
    }
    kept.reverse();
    while kept
        .first()
        .is_some_and(|message| message.role == Role::Assistant)
    {
        kept.remove(0);
    }
    kept
}
