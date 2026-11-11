//! The "Welcome" document created on the first run, and a small demo library
//! for visual QA.
//!
//! Both are built through writer-core's block API with real formatting, not
//! parsed from Markdown.

use writer_core::{Block, BlockKind, Document, Highlight, InlineAttrs, InlineStyle};
use writer_library::{FolderId, Library, Result};

use crate::document_io::StoredForm;

/// Builds one block from a line of light markup:
///
/// `{k:⌘P}` a key (inline code), `{b:..}` bold, `{i:..}` italic, `{u:..}`
/// underline, `{s:..}` strikethrough, `{c:..}` inline code, `{h:..}`
/// highlight. Everything else is plain text. Markup does not nest.
pub fn rich(kind: BlockKind, markup: &str) -> Block {
    let mut block = Block::new(kind);
    let mut rest = markup;
    while !rest.is_empty() {
        let Some(open) = rest.find('{') else {
            block = block.with_span(rest, InlineAttrs::default());
            break;
        };
        let after = &rest[open..];
        let tagged = after
            .get(1..3)
            .filter(|tag| tag.ends_with(':'))
            .and_then(|tag| tag.chars().next())
            .and_then(|tag| attrs_for(tag).map(|attrs| (tag, attrs)));
        let close = after.find('}');
        match (tagged, close) {
            (Some((_, attrs)), Some(close)) => {
                if open > 0 {
                    block = block.with_span(&rest[..open], InlineAttrs::default());
                }
                block = block.with_span(&after[3..close], attrs);
                rest = &after[close + 1..];
            }
            _ => {
                // A literal brace.
                block = block.with_span(&rest[..open + 1], InlineAttrs::default());
                rest = &rest[open + 1..];
            }
        }
    }
    block
}

fn attrs_for(tag: char) -> Option<InlineAttrs> {
    let plain = InlineAttrs::default();
    Some(match tag {
        'k' | 'c' => plain.with(InlineStyle::Code),
        'b' => plain.with(InlineStyle::Bold),
        'i' => plain.with(InlineStyle::Italic),
        'u' => plain.with(InlineStyle::Underline),
        's' => plain.with(InlineStyle::Strikethrough),
        'h' => InlineAttrs {
            highlight: Some(Highlight::Yellow),
            ..plain
        },
        _ => return None,
    })
}

fn heading(level: u8, text: &str) -> Block {
    Block::with_text(BlockKind::heading(level), text)
}

fn paragraph(markup: &str) -> Block {
    rich(BlockKind::Paragraph, markup)
}

fn bullet(markup: &str) -> Block {
    rich(BlockKind::BulletItem, markup)
}

fn todo(checked: bool, markup: &str) -> Block {
    rich(BlockKind::TodoItem { checked }, markup)
}

/// The title of the welcome document (its first heading).
pub const WELCOME_TITLE: &str = "Welcome to The Writer";

/// The document a new library starts with. It teaches the essentials: the
/// mouse (right-click menus, the menu bar), the palette, the format and
/// insert menus, Markdown shortcuts, page view, comments, Vim mode and
/// export.
pub fn welcome_document() -> Document {
    Document::from_blocks(vec![
        heading(1, WELCOME_TITLE),
        paragraph(
            "The Writer is a quiet place for long and short writing. Everything can be done \
             from the keyboard, everything can be done with the mouse, and your text is saved \
             as you type. {i:There is no save button.}",
        ),
        heading(2, "With the mouse"),
        bullet(
            "Right-click anything to see what you can do with it: the text or a selection, a \
             link, a comment, a document or a folder in the library, the title above the text, \
             the counts at the bottom of the window.",
        ),
        bullet(
            "Select some text by dragging over it, or double-click a word: a small format bar \
             comes up above it, with bold, italic, highlight, a link, a comment and more. It \
             can be switched off in the settings.",
        ),
        bullet(
            "The menu bar at the top of the screen has every command, with its key beside it: \
             File, Edit, Format, Insert, View.",
        ),
        bullet(
            "In the library, the {b:+} button starts a document or a folder, and rows can be \
             dragged to another folder or to the trash.",
        ),
        bullet("{k:⌃↩} opens the same menus from the keyboard, and so does {k:⇧F10}."),
        heading(2, "Find anything"),
        bullet("{k:⇧⌘P} opens the command palette: every command, with its shortcut beside it."),
        bullet("{k:⌘P} jumps to a document by name. {k:⇧⌘F} searches the text of every document."),
        bullet(
            "{k:⌘0} moves to the library on the left and {k:⌘1} comes back to the text. \
             {k:⌘N} starts a new document, {k:⇧⌘N} a new folder.",
        ),
        bullet("{k:⌘F} finds in this document and {k:⌥⌘F} replaces."),
        bullet("{k:⌘/} lists every keyboard shortcut."),
        heading(2, "Format without a toolbar"),
        paragraph(
            "Select some text and press {k:⌘.} for the format menu, or use the usual keys: \
             {b:bold} with {k:⌘B}, {i:italic} with {k:⌘I}, {u:underline} with {k:⌘U}, \
             {h:highlight} with {k:⇧⌘H} and a link with {k:⌘K}. {k:⌘T} picks a font.",
        ),
        paragraph(
            "Type {k:/} at the start of an empty line for the insert menu: headings, lists, \
             quotes, code blocks, dividers and page breaks.",
        ),
        heading(2, "Markdown shortcuts"),
        paragraph("Markdown typed at the start of a line turns into formatting as you go."),
        bullet("{c:#} and a space starts a heading; {c:##} a smaller one."),
        bullet(
            "{c:-} and a space starts a list like this one, {c:1.} a numbered list, \
             {c:[]} a to-do item and {c:>} a quote.",
        ),
        bullet(
            "Inside a line, {c:**bold**}, {c:*italic*}, {c:`code`}, {c:~~struck~~} and \
             {c:==highlight==} work as you close them.",
        ),
        todo(true, "Open The Writer"),
        todo(false, "Press {k:⇧⌘↩} on this line to tick it"),
        todo(false, "Write the first sentence of something"),
        heading(2, "Pages, comments and Vim"),
        bullet(
            "{k:⌥⌘P} switches between the flowing column and page view. Paper size and the \
             defaults for new documents are in Settings, {k:⌘,}.",
        ),
        bullet(
            "{k:⌥⌘M} adds a comment to the selected text, {k:⌥⌘]} and {k:⌥⌘[} step through \
             the comments, {k:⌘2} moves to the list beside the text. {k:⌥⌘C} shows or hides \
             them.",
        ),
        bullet(
            "{k:⌥⌘V} turns Vim mode on or off. In it, {c::w} saves at once and {c::q} closes \
             the document.",
        ),
        heading(2, "Take your words elsewhere"),
        paragraph(
            "{k:⇧⌘S} exports this document, or its whole folder as one file: Markdown, a Word \
             document or plain text. It can also copy the text to the clipboard.",
        ),
        rich(
            BlockKind::Quote,
            "This document is yours. Edit it, or move it to the trash from the library.",
        ),
    ])
}

/// Creates the welcome document at the root of `library`.
pub fn create_welcome_document(
    library: &mut Library,
    cx: &mut gpui::Context<Library>,
) -> Result<writer_library::DocumentMeta> {
    let form = StoredForm::of(&welcome_document(), None);
    library.create_document(None, &form.content(), cx)
}

/// Environment variable: when set to a non-empty value and the library is
/// empty, the app fills it with [`seed_demo_library`] at startup. For visual
/// QA with a throwaway `THE_WRITER_DATA_DIR`.
pub const SEED_DEMO_ENV: &str = "THE_WRITER_SEED_DEMO";

/// Whether [`SEED_DEMO_ENV`] asks for the demo library.
pub fn demo_seed_requested() -> bool {
    std::env::var_os(SEED_DEMO_ENV).is_some_and(|value| !value.is_empty())
}

fn add(
    library: &mut Library,
    folder: Option<FolderId>,
    blocks: Vec<Block>,
    cx: &mut gpui::Context<Library>,
) -> Result<()> {
    let form = StoredForm::of(&Document::from_blocks(blocks), None);
    library.create_document(folder, &form.content(), cx)?;
    Ok(())
}

/// Fills an empty library with a few folders and documents: a novel with
/// two parts, a blog folder and loose notes. For screenshots and manual QA.
pub fn seed_demo_library(library: &mut Library, cx: &mut gpui::Context<Library>) -> Result<()> {
    let novel = library.create_folder(None, "The Salt Road", cx)?.id;
    let part_one = library.create_folder(Some(novel), "Part one", cx)?.id;
    let part_two = library.create_folder(Some(novel), "Part two", cx)?.id;
    let blog = library.create_folder(None, "Blog", cx)?.id;

    add(
        library,
        Some(part_one),
        vec![
            heading(1, "Chapter 1: Arrival"),
            paragraph(
                "The ferry came in late, as it always did in October, and Mara was the only \
                 passenger who stayed on deck to watch the harbour lights come up out of the \
                 fog. {i:Eleven years}, she thought, and the town had not moved a single lamp.",
            ),
            paragraph(
                "She had one bag, a letter she had read too many times, and the key to a house \
                 that might no longer have a door.",
            ),
            heading(2, "The quay"),
            paragraph(
                "Nobody met her. That was {b:the first surprise}; the second was the smell of \
                 the salt works, sharp as a struck match, which she had forgotten entirely.",
            ),
        ],
        cx,
    )?;
    add(
        library,
        Some(part_one),
        vec![
            heading(1, "Chapter 2: The Harbour"),
            paragraph(
                "By morning the fog had gone and the harbour was a plain grey fact. Boats, \
                 ropes, gulls. A man mending a net looked up at her and then, deliberately, \
                 down again.",
            ),
            rich(
                BlockKind::Quote,
                "A town remembers what you owe it long after it forgets your name.",
            ),
            paragraph("She bought bread, and asked about the house on the hill."),
        ],
        cx,
    )?;
    add(
        library,
        Some(part_two),
        vec![
            heading(1, "Chapter 3: Salt"),
            paragraph(
                "The pans stretched to the horizon in rectangles of pink and white, and the \
                 wind moved over them like a hand smoothing a sheet.",
            ),
        ],
        cx,
    )?;
    add(
        library,
        Some(novel),
        vec![
            heading(1, "Outline"),
            rich(
                BlockKind::NumberedItem,
                "Mara returns; the house; the letter.",
            ),
            rich(
                BlockKind::NumberedItem,
                "The salt works and the strike of {i:'62}.",
            ),
            rich(BlockKind::NumberedItem, "What the harbour master kept."),
            todo(true, "Decide who wrote the letter"),
            todo(false, "Check ferry timetables for the period"),
        ],
        cx,
    )?;
    add(
        library,
        Some(blog),
        vec![
            heading(1, "On keeping a notebook"),
            paragraph(
                "A notebook is not an archive. It is a place where a thought can be {h:wrong} \
                 without consequence, which is the only condition under which most thoughts \
                 are willing to appear at all.",
            ),
            paragraph("I keep three rules, and break the third one daily:"),
            bullet("Date every page."),
            bullet("Never tear one out."),
            bullet("Write the dull thing first."),
        ],
        cx,
    )?;
    add(
        library,
        Some(blog),
        vec![
            heading(1, "Why plain tools"),
            paragraph(
                "Every feature is a small request for attention. A plain tool asks for none, \
                 and gives the whole of it back to the sentence in front of you.",
            ),
        ],
        cx,
    )?;
    add(
        library,
        None,
        vec![
            heading(1, "Ideas"),
            bullet("An essay on ferries as waiting rooms."),
            bullet("Interview the net mender."),
            bullet("A short piece about the colour of salt pans at dusk."),
        ],
        cx,
    )?;
    add(
        library,
        None,
        vec![paragraph(
            "The Salt Road is out next month. It is a small book about a harbour town, a letter \
             and a debt. I would love for you to read it.",
        )],
        cx,
    )?;
    Ok(())
}

/// A book-length document for measuring: `blocks` blocks, a chapter heading
/// every forty, paragraphs of thirty to ninety words with a little inline
/// formatting. The text is generated, the same for the same `blocks`.
pub fn book_document(blocks: usize) -> Document {
    const WORDS: [&str; 48] = [
        "the",
        "harbour",
        "salt",
        "letter",
        "and",
        "morning",
        "she",
        "walked",
        "along",
        "quay",
        "where",
        "nets",
        "were",
        "drying",
        "in",
        "a",
        "wind",
        "that",
        "smelled",
        "of",
        "rain",
        "nobody",
        "remembered",
        "her",
        "name",
        "but",
        "house",
        "on",
        "hill",
        "still",
        "kept",
        "its",
        "door",
        "ferry",
        "came",
        "late",
        "as",
        "it",
        "always",
        "did",
        "October",
        "with",
        "one",
        "lamp",
        "burning",
        "over",
        "water",
        "again",
    ];
    // A small linear congruential generator: no dependency, stable output.
    let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
    let mut next = move |bound: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((seed >> 33) as usize) % bound.max(1)
    };
    let sentence = |words: usize, next: &mut dyn FnMut(usize) -> usize| {
        let mut text = String::new();
        for ix in 0..words {
            let word = WORDS[next(WORDS.len())];
            if ix == 0 {
                let mut chars = word.chars();
                if let Some(first) = chars.next() {
                    text.extend(first.to_uppercase());
                    text.push_str(chars.as_str());
                }
            } else {
                text.push(' ');
                text.push_str(word);
            }
        }
        text.push('.');
        text
    };
    let mut out = Vec::with_capacity(blocks.max(1));
    for ix in 0..blocks.max(1) {
        if ix % 40 == 0 {
            out.push(heading(
                if ix == 0 { 1 } else { 2 },
                &format!("Chapter {}", ix / 40 + 1),
            ));
            continue;
        }
        let mut block = Block::new(BlockKind::Paragraph);
        let sentences = 3 + next(5);
        for s in 0..sentences {
            let words = 8 + next(12);
            let text = sentence(words, &mut next);
            let attrs = match next(12) {
                0 => InlineAttrs::default().with(InlineStyle::Italic),
                1 => InlineAttrs::default().with(InlineStyle::Bold),
                _ => InlineAttrs::default(),
            };
            let text = if s + 1 < sentences {
                format!("{text} ")
            } else {
                text
            };
            block = block.with_span(&text, attrs);
        }
        out.push(block);
    }
    Document::from_blocks(out)
}
