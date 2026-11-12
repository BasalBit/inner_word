//! Reading and writing text: read, find, create, replace, edit, insert, replace section.

use writer_agent::{
    AddCommentRequest, ContentChanged, CreateDocumentRequest, EditRequest, Error, ErrorKind,
    FindRequest, InsertPosition, InsertRequest, ItemKind, ListCommentsRequest, MatchedAs,
    Placement, ReadRequest, ReplaceDocumentRequest, ReplaceSectionRequest, Target, TextFormat,
    TreeRequest,
};
use writer_core::{BlockKind, Document, Rgb};
use writer_store::DocumentId;

use super::{Fixture, claude, fixture};

const CHAPTER: &str = "# The Harbour\n\nThe *Maren* left at dawn. The gulls left too.\n\n## Crew\n\n- Ada, the **captain**\n- Ben\n\n## Cargo\n\nSalt, and [more salt](https://example.com/salt).\n";

fn edit(fx: &Fixture, id: DocumentId, old: &str, new: &str) -> Result<ContentChanged, Error> {
    fx.library.edit(&EditRequest {
        document: id.into(),
        old_text: old.into(),
        new_text: new.into(),
        write: claude(),
        ..Default::default()
    })
}

fn comment(fx: &Fixture, id: DocumentId, quote: &str) -> u64 {
    fx.library
        .add_comment(&AddCommentRequest {
            document: id.into(),
            quote: quote.into(),
            body: format!("About {quote}"),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .thread
        .id
}

fn anchors(fx: &Fixture, id: DocumentId) -> Vec<(u64, Option<String>)> {
    fx.library
        .comments(&ListCommentsRequest {
            document: id.into(),
            ..Default::default()
        })
        .unwrap()
        .threads
        .into_iter()
        .map(|thread| (thread.id, thread.anchored_text))
        .collect()
}

#[test]
fn a_document_is_read_as_markdown_text_or_json() {
    let fx = fixture();
    let doc = fx.create(Some("Book"), CHAPTER);
    assert_eq!(fx.markdown(doc.id), CHAPTER);
    let text = fx.read(doc.id, TextFormat::PlainText);
    assert!(text.starts_with("The Harbour\n"), "{text}");
    assert!(text.contains("The Maren left at dawn."), "{text}");
    assert!(!text.contains('*') && !text.contains('#'), "{text}");
    let json = fx.read(doc.id, TextFormat::Json);
    assert_eq!(Document::from_json(&json).unwrap(), fx.document(doc.id));
    assert_eq!(json, fx.store().document(doc.id).unwrap().content);

    // One section: the heading and everything under it, subsections included.
    let read = |heading: &str, format: TextFormat| {
        fx.library.read(&ReadRequest {
            document: doc.id.into(),
            heading: Some(heading.into()),
            format,
            ..Default::default()
        })
    };
    let crew = read("Crew", TextFormat::Markdown).unwrap();
    assert_eq!(crew.content, "## Crew\n\n- Ada, the **captain**\n- Ben\n");
    let section = crew.section.unwrap();
    assert_eq!(
        (section.level, section.text.as_str(), section.block),
        (2, "Crew", 2)
    );
    assert_eq!(crew.document, doc);
    assert_eq!(crew.format, TextFormat::Markdown);
    assert_eq!(
        read("## cargo", TextFormat::Markdown)
            .unwrap()
            .content
            .lines()
            .next(),
        Some("## Cargo")
    );
    assert_eq!(
        read("The Harbour", TextFormat::Markdown).unwrap().content,
        CHAPTER
    );
    assert!(
        read("Crew", TextFormat::PlainText)
            .unwrap()
            .content
            .contains("Ada, the captain")
    );
    let part = Document::from_json(&read("Crew", TextFormat::Json).unwrap().content).unwrap();
    assert_eq!(part.block_count(), 3);
    match read("Passengers", TextFormat::Markdown) {
        Err(Error::NotFound {
            kind: ItemKind::Heading,
            hint: Some(hint),
            ..
        }) => {
            assert!(
                hint.contains("## Crew") && hint.contains("# The Harbour"),
                "{hint}"
            );
        }
        other => panic!("{other:?}"),
    }

    // With the comments, of the whole text or of the section.
    comment(&fx, doc.id, "Maren");
    comment(&fx, doc.id, "captain");
    let with = fx
        .library
        .read(&ReadRequest {
            document: doc.id.into(),
            comments: true,
            ..Default::default()
        })
        .unwrap();
    let threads = with.comments.unwrap();
    assert_eq!(threads.len(), 2);
    assert_eq!(threads[0].anchored_text.as_deref(), Some("Maren"));
    assert_eq!(
        with.content, CHAPTER,
        "comments are not part of the Markdown"
    );
    let crew = fx
        .library
        .read(&ReadRequest {
            document: doc.id.into(),
            heading: Some("Crew".into()),
            comments: true,
            ..Default::default()
        })
        .unwrap();
    let threads = crew.comments.unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].anchored_text.as_deref(), Some("captain"));
    assert_eq!(threads[0].heading.as_deref(), Some("Crew"));
    assert_eq!(read("Crew", TextFormat::Markdown).unwrap().comments, None);

    // Where a text stands.
    let found = fx
        .library
        .find(&FindRequest {
            document: doc.id.into(),
            text: "left".into(),
            limit: None,
        })
        .unwrap();
    assert_eq!(found.count, 2);
    assert_eq!(found.matched_as, Some(MatchedAs::Text));
    assert_eq!(found.occurrences[1].occurrence, 2);
    assert_eq!(found.occurrences[1].heading.as_deref(), Some("The Harbour"));
    let none = fx
        .library
        .find(&FindRequest {
            document: doc.id.into(),
            text: "anchor".into(),
            limit: None,
        })
        .unwrap();
    assert_eq!((none.count, none.matched_as), (0, None));
}

#[test]
fn documents_are_created_from_markdown() {
    let fx = fixture();
    let create = |request: CreateDocumentRequest| fx.library.create_document(&request);
    // The first write creates the library.
    assert!(!fx.data_dir().exists());
    let first = create(CreateDocumentRequest {
        markdown: CHAPTER.into(),
        write: claude(),
        ..Default::default()
    })
    .unwrap();
    assert!(fx.library.status().unwrap().exists);
    assert!(first.written.changed && !first.written.dry_run);
    assert!(first.created_folders.is_empty() && first.warnings.is_empty());
    assert_eq!(first.document.title, "The Harbour");
    assert_eq!(first.document.path, "The Harbour");
    assert_eq!(first.document.words, 21);
    assert_eq!(fx.markdown(first.document.id), CHAPTER);
    let blocks = fx.document(first.document.id);
    assert_eq!(blocks.block_count(), 7);
    assert_eq!(blocks.blocks()[3].kind(), &BlockKind::BulletItem);
    assert!(blocks.blocks()[3].attrs_at(9).unwrap().bold);
    assert_eq!(
        blocks.blocks()[6].attrs_at(10).unwrap().link.as_deref(),
        Some("https://example.com/salt")
    );

    // Into a folder that is there; one that is not is an error unless it may be made.
    fx.create(Some("Book/Part one"), "x");
    let missing = create(CreateDocumentRequest {
        markdown: "y".into(),
        folder: Some(Target::path("Book/Part three")),
        write: claude(),
        ..Default::default()
    });
    assert!(matches!(
        missing,
        Err(Error::NotFound {
            kind: ItemKind::Folder,
            ..
        })
    ));
    let made = create(CreateDocumentRequest {
        markdown: "# In three\n".into(),
        folder: Some(Target::path("Book/Part three/Drafts")),
        create_folders: true,
        write: claude(),
        ..Default::default()
    })
    .unwrap();
    let created: Vec<&str> = made
        .created_folders
        .iter()
        .map(|f| f.path.as_str())
        .collect();
    assert_eq!(created, ["Book/Part three", "Book/Part three/Drafts"]);
    assert_eq!(made.document.path, "Book/Part three/Drafts/In three");
    // A folder that is there is used, however it is named.
    let again = create(CreateDocumentRequest {
        markdown: "# Also here\n".into(),
        folder: Some(Target::path("part ONE")),
        create_folders: true,
        write: claude(),
        ..Default::default()
    })
    .unwrap();
    assert!(again.created_folders.is_empty());
    assert_eq!(again.document.folder, "Book/Part one");

    // At a place among the documents of the folder.
    let place = |title: &str, placement: Placement| {
        create(CreateDocumentRequest {
            markdown: format!("# {title}\n"),
            folder: Some(Target::path("Book/Part one")),
            placement: Some(placement),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
    };
    place("Opening", Placement::First);
    place("After x", Placement::After(Target::path("Part one/x")));
    place("Second", Placement::Index(1));
    place("Before also", Placement::Before(Target::path("Also here")));
    let tree = fx
        .library
        .tree(&TreeRequest {
            folder: Some(Target::path("Part one")),
            ..Default::default()
        })
        .unwrap();
    let order: Vec<&str> = tree.documents.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(
        order,
        [
            "Opening",
            "Second",
            "x",
            "After x",
            "Before also",
            "Also here"
        ]
    );
    assert!(matches!(
        create(CreateDocumentRequest {
            markdown: "z".into(),
            placement: Some(Placement::After(Target::path("Part one/x"))),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));

    // What Markdown has that a document has not is kept as text and reported.
    let table = create(CreateDocumentRequest {
        markdown: "# Costs\n\n| Item | Price |\n|---|---|\n| Ink | 12 |\n\n![a chart](chart.png)\n"
            .into(),
        write: claude(),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(table.warnings.len(), 2, "{:?}", table.warnings);
    assert!(table.warnings[0].contains("table"), "{:?}", table.warnings);
    assert!(table.warnings[1].contains("image"), "{:?}", table.warnings);
    assert!(fx.markdown(table.document.id).contains("Ink"));
}

#[test]
fn an_edit_replaces_exactly_the_text_it_names() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);

    let edited = edit(&fx, doc.id, "at dawn", "before dawn").unwrap();
    assert_eq!(edited.replacements, 1);
    assert_eq!(edited.matched_as, Some(MatchedAs::Text));
    assert_eq!(
        (
            edited.blocks_added,
            edited.blocks_removed,
            edited.blocks_changed
        ),
        (0, 0, 1)
    );
    assert_eq!((edited.words_before, edited.words_after), (21, 21));
    assert_eq!(edited.characters_after, edited.characters_before + 4);
    assert_eq!(edited.headings, ["The Harbour"]);
    assert!(edited.warnings.is_empty() && edited.comments_detached.is_empty());
    assert_eq!(edited.document.words, 21);
    assert_eq!(
        fx.markdown(doc.id),
        CHAPTER.replace("at dawn", "before dawn")
    );

    // The old text is not there: nothing changes, and the error says so.
    let before = fx.bytes();
    let missing = edit(&fx, doc.id, "at noon", "at night").unwrap_err();
    assert_eq!(missing.kind(), ErrorKind::NoMatch);
    assert_eq!(missing.exit_code(), 6);
    assert!(
        missing.to_string().contains("\u{201c}at noon\u{201d}"),
        "{missing}"
    );
    // Capitals count; the error names the text that is there.
    match edit(&fx, doc.id, "the maren", "x") {
        Err(Error::NoMatch {
            hint: Some(hint), ..
        }) => assert!(hint.contains("The Maren"), "{hint}"),
        other => panic!("{other:?}"),
    }
    // It is there twice: nothing changes, and the error lists the places.
    let twice = edit(&fx, doc.id, "left", "sailed").unwrap_err();
    assert_eq!(twice.kind(), ErrorKind::NotUnique);
    assert_eq!(twice.exit_code(), 7);
    let Error::NotUnique {
        count, occurrences, ..
    } = &twice
    else {
        panic!("{twice:?}");
    };
    assert_eq!(*count, 2);
    assert_eq!(occurrences.len(), 2);
    assert_eq!(occurrences[0].block, 1);
    assert!(occurrences[0].context.contains("left before dawn"));
    let message = twice.to_string();
    assert!(message.contains("occurs 2 times"), "{message}");
    assert!(message.contains("[The Harbour]"), "{message}");
    assert_eq!(twice.report().occurrences.len(), 2);
    assert_eq!(fx.bytes(), before, "a failed edit changed the library");
    assert!(matches!(
        edit(&fx, doc.id, "", "x"),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        fx.library.edit(&EditRequest {
            document: doc.id.into(),
            old_text: "left".into(),
            new_text: "x".into(),
            all: true,
            occurrence: Some(1),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));

    // One of them by its number; all of them.
    let second = fx
        .library
        .edit(&EditRequest {
            document: doc.id.into(),
            old_text: "left".into(),
            new_text: "stayed".into(),
            occurrence: Some(2),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(second.replacements, 1);
    assert!(
        fx.markdown(doc.id)
            .contains("left before dawn. The gulls stayed too.")
    );
    let every = fx
        .library
        .edit(&EditRequest {
            document: doc.id.into(),
            old_text: "salt".into(),
            new_text: "pepper".into(),
            all: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    // "Salt" with a capital was not meant, and the address of the link is not text.
    assert_eq!(every.replacements, 1);
    assert_eq!(every.headings, ["Cargo"]);
    assert!(
        fx.markdown(doc.id)
            .contains("Salt, and [more pepper](https://example.com/salt)."),
        "{}",
        fx.markdown(doc.id)
    );

    // Replacing with the same text changes nothing.
    let same = edit(&fx, doc.id, "Ben", "Ben").unwrap();
    assert!(!same.written.changed && same.snapshot.is_none());
    assert_eq!(same.replacements, 1);
    // An empty new text deletes; a whole block goes with its text.
    let deleted = edit(&fx, doc.id, " The gulls stayed too.", "").unwrap();
    assert_eq!(deleted.blocks_changed, 1);
    let removed = edit(&fx, doc.id, "Ben", "").unwrap();
    assert_eq!((removed.blocks_removed, removed.blocks_changed), (1, 0));
    assert_eq!(
        fx.markdown(doc.id),
        "# The Harbour\n\nThe *Maren* left before dawn.\n\n## Crew\n\n- Ada, the **captain**\n\n## Cargo\n\nSalt, and [more pepper](https://example.com/salt).\n"
    );
}

#[test]
fn words_quoted_around_a_change_keep_the_formatting_each_has() {
    // An agent quotes more of the text than it changes, so that the place is found once.
    // What it quotes and leaves as it is must keep what it has: a word in bold, a link.
    let fx = fixture();
    let doc = fx.create(
        None,
        "The boats came in. **Nobody** on the quay counted them, and *she* said so. A [link](https://example.com/a) here.\n",
    );
    // The bold word changes, the plain words after it are only quoted.
    edit(&fx, doc.id, "Nobody on the quay", "No one on the quay").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("**No one** on the quay counted"),
        "{markdown}"
    );
    // Words added after a quoted italic word are typed after plain words: plain.
    edit(&fx, doc.id, "she said so", "she said so twice").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(markdown.contains("and *she* said so twice."), "{markdown}");
    // A link in the quoted words stays a link.
    edit(&fx, doc.id, "A link here", "A link there").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("A [link](https://example.com/a) there."),
        "{markdown}"
    );
    // A comma goes between two plain words; the italic word after them is untouched.
    edit(&fx, doc.id, "counted them, and she", "counted them and she").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("counted them and *she* said"),
        "{markdown}"
    );
    // Words put in front of a quoted formatted word take its formatting, as the words
    // that replace a formatted word do; words put behind it are typed behind it.
    edit(&fx, doc.id, "she said", "only she said").unwrap();
    edit(&fx, doc.id, "link", "link text").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("and *only she* said so twice"),
        "{markdown}"
    );
    assert!(
        markdown.contains("A [link text](https://example.com/a) there."),
        "{markdown}"
    );
    // Markup in the new text comes on top of what the quoted words have.
    edit(
        &fx,
        doc.id,
        "No one on the quay",
        "No one on the *old* quay",
    )
    .unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("**No one** on the *old* quay counted"),
        "{markdown}"
    );
    // A comment on a quoted word stays where it is.
    fx.library
        .add_comment(&AddCommentRequest {
            document: doc.id.into(),
            quote: "quay".into(),
            body: "Which one?".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let changed = edit(&fx, doc.id, "old quay counted", "old quay never counted").unwrap();
    assert!(changed.comments_detached.is_empty());
    let threads = fx
        .library
        .comments(&ListCommentsRequest {
            document: doc.id.into(),
            ..Default::default()
        })
        .unwrap()
        .threads;
    assert_eq!(threads[0].anchored_text.as_deref(), Some("quay"));
    // Two changes in one edit, the first in an italic word: each takes the formatting of
    // the word it replaces, and the words between them stay as they are.
    let both = fx.create(None, "She read it *twice*, standing up.\n");
    edit(
        &fx,
        both.id,
        "read it twice, standing up",
        "read it three times, sitting down",
    )
    .unwrap();
    assert_eq!(
        fx.markdown(both.id),
        "She read it *three times*, sitting down.\n"
    );
    // Accents: a word is a word, whatever its letters.
    let accents = fx.create(None, "Le **café** était fermé.\n");
    edit(&fx, accents.id, "café était", "cafè était").unwrap();
    assert!(
        fx.markdown(accents.id).contains("Le **cafè** était fermé."),
        "{}",
        fx.markdown(accents.id)
    );
}

#[test]
fn an_edit_keeps_the_formatting_and_reads_markdown() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);
    // Words without markup take the formatting of what they replace.
    edit(&fx, doc.id, "Maren", "Gannet").unwrap();
    edit(&fx, doc.id, "captain", "first mate").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(markdown.contains("The *Gannet* left"), "{markdown}");
    assert!(markdown.contains("the **first mate**"), "{markdown}");
    // Markup in the new text is formatting.
    edit(&fx, doc.id, "at dawn", "at **first** light").unwrap();
    let markdown = fx.markdown(doc.id);
    assert!(
        markdown.contains("left at **first** light. The gulls"),
        "{markdown}"
    );
    edit(
        &fx,
        doc.id,
        "The gulls",
        "The [gulls](https://example.com/gulls)",
    )
    .unwrap();
    assert!(
        fx.markdown(doc.id)
            .contains("The [gulls](https://example.com/gulls) left too.")
    );
    // Spaces at the ends of the new text are kept.
    edit(&fx, doc.id, "left too", "left,  too").unwrap();
    edit(&fx, doc.id, "Ben", "Ben ").unwrap();
    assert!(fx.document(doc.id).blocks()[4].text() == "Ben ");
    edit(&fx, doc.id, "Ben ", " *Ben*").unwrap();
    let ben = &fx.document(doc.id).blocks()[4].clone();
    assert_eq!(ben.text(), " Ben");
    assert!(ben.attrs_at(1).unwrap().italic && !ben.attrs_at(0).unwrap().italic);

    // The old text as it stands in the Markdown, markup and all.
    let as_markdown = edit(&fx, doc.id, "Ada, the **first mate**", "Ada, the *cook*").unwrap();
    assert_eq!(as_markdown.matched_as, Some(MatchedAs::Markdown));
    let ada = fx.document(doc.id).blocks()[3].clone();
    assert_eq!(ada.text(), "Ada, the cook");
    assert!(
        ada.attrs_at(9).unwrap().italic,
        "the new markup is the formatting"
    );
    assert!(
        !ada.attrs_at(9).unwrap().bold,
        "the old markup is gone with the old text"
    );
    assert_eq!(ada.kind(), &BlockKind::BulletItem);
    // Across blocks, with the blank line Markdown puts between them.
    let across = edit(
        &fx,
        doc.id,
        "## Cargo\n\nSalt, and [more salt](https://example.com/salt).",
        "## Hold\n\nEmpty.",
    )
    .unwrap();
    assert_eq!(across.matched_as, Some(MatchedAs::Markdown));
    assert!(
        fx.markdown(doc.id).ends_with("## Hold\n\nEmpty.\n"),
        "{}",
        fx.markdown(doc.id)
    );
    // A heading stays a heading when its text is replaced by plain words...
    edit(&fx, doc.id, "Hold", "The hold").unwrap();
    assert!(fx.markdown(doc.id).contains("## The hold\n"));
    // ...and takes the level the new text gives when the whole block is replaced.
    edit(&fx, doc.id, "The hold", "### Below deck").unwrap();
    assert!(fx.markdown(doc.id).contains("### Below deck\n"));
    // In the middle of a sentence a "#" is a "#", and "1." is "1.".
    edit(&fx, doc.id, "Empty", "# 1 on the list").unwrap();
    assert!(
        fx.document(doc.id)
            .plain_text()
            .ends_with("# 1 on the list.")
    );
    assert_eq!(
        fx.document(doc.id).blocks().last().unwrap().kind(),
        &BlockKind::Paragraph
    );
    // Several paragraphs in place of a piece of one split it.
    let split = edit(
        &fx,
        doc.id,
        " on the list",
        "\n\nA new paragraph\n\nand another",
    )
    .unwrap();
    assert!(split.blocks_added >= 1, "{split:?}");
    let text = fx.document(doc.id).plain_text();
    assert!(
        text.ends_with("# 1\nA new paragraph\nand another."),
        "{text}"
    );

    // Taken literally, nothing is markup.
    let literal = fx
        .library
        .edit(&EditRequest {
            document: doc.id.into(),
            old_text: "and another".into(),
            new_text: "**not bold** & [not a link](x)".into(),
            literal: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(literal.replacements, 1);
    assert!(
        fx.document(doc.id)
            .plain_text()
            .ends_with("**not bold** & [not a link](x).")
    );
    assert!(matches!(
        fx.library.edit(&EditRequest {
            document: doc.id.into(),
            old_text: "The *Gannet*".into(),
            new_text: "x".into(),
            literal: true,
            write: claude(),
            ..Default::default()
        }),
        Err(Error::NoMatch { .. })
    ));
}

#[test]
fn an_edit_leaves_what_it_does_not_name_exactly_as_it_was() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);
    // What Markdown cannot say: a colour, a font, a text size, an alignment. Put there the
    // way the app would, through the editing engine.
    let mut store = writer_store::Store::open(fx.library.database_path()).unwrap();
    let mut ed = writer_core::EditorState::new(fx.document(doc.id));
    ed.select_range(writer_core::TextRange::in_block(1, 14, 18));
    ed.set_text_color(Some(Rgb {
        r: 200,
        g: 30,
        b: 30,
    }));
    ed.set_font_family(Some("Menlo"));
    ed.select_block(6);
    ed.set_alignment(writer_core::Alignment::Center);
    ed.set_text_size(Some(22.0));
    let styled = ed.into_document();
    let stats = styled.stats();
    store
        .save_document(
            doc.id,
            &writer_store::DocumentContent {
                content: &styled.to_json(),
                title: "The Harbour",
                plain_text: &styled.plain_text(),
                word_count: stats.words as u64,
                char_count: stats.characters as u64,
            },
        )
        .unwrap();
    store.close().unwrap();
    let before = fx.document(doc.id);
    let colour = before.blocks()[1].attrs_at(15).unwrap().clone();
    assert_eq!(colour.font.as_deref(), Some("Menlo"));
    let dawn = comment(&fx, doc.id, "dawn");
    let captain = comment(&fx, doc.id, "captain");
    let before = fx.document(doc.id);

    // An edit in the same paragraph, beside the coloured and the commented words.
    edit(&fx, doc.id, "The gulls left too", "The terns left as well").unwrap();
    let after = fx.document(doc.id);
    assert_eq!(after.blocks()[1].attrs_at(15).unwrap().color, colour.color);
    assert_eq!(after.blocks()[1].attrs_at(15).unwrap().font, colour.font);
    assert_eq!(
        anchors(&fx, doc.id),
        [
            (dawn, Some("dawn".into())),
            (captain, Some("captain".into()))
        ]
    );
    // Every other block is the very block it was: same id, same everything.
    for (index, block) in before.blocks().iter().enumerate() {
        if index != 1 {
            assert_eq!(&after.blocks()[index], block, "block {index}");
        }
        assert_eq!(after.blocks()[index].id(), block.id());
    }
    assert_eq!(
        after.blocks()[6].alignment(),
        writer_core::Alignment::Center
    );
    assert_eq!(after.blocks()[6].text_size(), Some(22.0));
    assert_eq!(after.settings(), before.settings());

    // A comment on a word that is replaced by plain words stays on the replacement.
    let edited = edit(&fx, doc.id, "dawn", "first light").unwrap();
    assert!(edited.comments_detached.is_empty());
    assert_eq!(anchors(&fx, doc.id)[0], (dawn, Some("first light".into())));
    // A comment whose text is deleted loses its place, is kept, and is reported.
    let removed = edit(&fx, doc.id, ", the **captain**", "").unwrap();
    assert_eq!(removed.comments_detached.len(), 1);
    assert_eq!(removed.comments_detached[0].id, captain);
    assert_eq!(removed.comments_detached[0].anchored_text, None);
    assert_eq!(
        anchors(&fx, doc.id),
        [(dawn, Some("first light".into())), (captain, None)]
    );
    // The version kept before that edit has the comment in its place.
    let was = Document::from_json(
        &fx.store()
            .snapshot(removed.snapshot.unwrap().id)
            .unwrap()
            .content,
    )
    .unwrap();
    assert!(!was.is_orphaned(writer_core::CommentId(captain)));
}

#[test]
fn text_is_inserted_at_the_end_the_start_and_at_headings() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);
    let insert = |markdown: &str, position: InsertPosition, heading: Option<&str>| {
        fx.library.insert(&InsertRequest {
            document: doc.id.into(),
            markdown: markdown.into(),
            position,
            heading: heading.map(String::from),
            write: claude(),
            ..Default::default()
        })
    };
    let before = fx.document(doc.id);
    let appended = insert("## Log\n\nDay one.", InsertPosition::End, None).unwrap();
    assert_eq!(
        (
            appended.blocks_added,
            appended.blocks_removed,
            appended.blocks_changed
        ),
        (2, 0, 0)
    );
    assert_eq!(appended.headings, ["Log"]);
    assert_eq!(appended.words_after, appended.words_before + 3);
    insert("*An epigraph.*", InsertPosition::Start, None).unwrap();
    insert(
        "Before the crew.",
        InsertPosition::BeforeHeading,
        Some("Crew"),
    )
    .unwrap();
    insert(
        "Right under the heading.",
        InsertPosition::AfterHeading,
        Some("## Crew"),
    )
    .unwrap();
    insert("- Cy", InsertPosition::EndOfSection, Some("Crew")).unwrap();
    let top = insert(
        "Last of all in the first heading.",
        InsertPosition::EndOfSection,
        Some("The Harbour"),
    )
    .unwrap();
    assert_eq!(
        top.headings,
        ["Log"],
        "the end of the top section is the end of the document"
    );
    assert_eq!(
        fx.markdown(doc.id),
        "*An epigraph.*\n\n# The Harbour\n\nThe *Maren* left at dawn. The gulls left too.\n\nBefore the crew.\n\n## Crew\n\nRight under the heading.\n\n- Ada, the **captain**\n- Ben\n- Cy\n\n## Cargo\n\nSalt, and [more salt](https://example.com/salt).\n\n## Log\n\nDay one.\n\nLast of all in the first heading.\n"
    );
    // Nothing that was there was touched.
    let after = fx.document(doc.id);
    for block in before.blocks() {
        let index = after.block_index(block.id()).expect("still there");
        assert_eq!(&after.blocks()[index], block);
    }

    // The title follows the first heading, not the epigraph above it.
    assert_eq!(
        fx.store().document_meta(doc.id).unwrap().title,
        "The Harbour"
    );
    // Mistakes.
    assert!(matches!(
        insert("x", InsertPosition::AfterHeading, None),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        insert("x", InsertPosition::End, Some("Crew")),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        insert("x", InsertPosition::AfterHeading, Some("Passengers")),
        Err(Error::NotFound {
            kind: ItemKind::Heading,
            ..
        })
    ));
    let nothing = insert("  \n", InsertPosition::End, None).unwrap();
    assert!(!nothing.written.changed);

    // Two headings with one text: which one has to be said.
    insert("## Log\n\nDay two.", InsertPosition::End, None).unwrap();
    match insert("x", InsertPosition::AfterHeading, Some("Log")) {
        Err(
            error @ Error::Ambiguous {
                kind: ItemKind::Heading,
                ..
            },
        ) => {
            assert_eq!(error.kind(), ErrorKind::Ambiguous);
        }
        other => panic!("{other:?}"),
    }
    fx.library
        .insert(&InsertRequest {
            document: doc.id.into(),
            markdown: "Second log.".into(),
            position: InsertPosition::AfterHeading,
            heading: Some("Log".into()),
            heading_occurrence: Some(2),
            write: claude(),
        })
        .unwrap();
    assert!(
        fx.markdown(doc.id)
            .ends_with("## Log\n\nSecond log.\n\nDay two.\n")
    );

    // Into an empty document, and after the empty paragraphs a document may end with.
    let empty = fx.create(None, "");
    fx.library
        .insert(&InsertRequest {
            document: empty.id.into(),
            markdown: "First words.".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(fx.document(empty.id).block_count(), 1);
    assert_eq!(fx.markdown(empty.id), "First words.\n");
    assert_eq!(
        fx.store().document_meta(empty.id).unwrap().title,
        "First words"
    );
}

#[test]
fn a_section_is_replaced_under_its_heading() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);
    let maren = comment(&fx, doc.id, "Maren");
    let ben = comment(&fx, doc.id, "Ben");
    let replace = |heading: &str, markdown: &str, include_heading: bool| {
        fx.library.replace_section(&ReplaceSectionRequest {
            document: doc.id.into(),
            heading: heading.into(),
            markdown: markdown.into(),
            include_heading,
            write: claude(),
            ..Default::default()
        })
    };
    let replaced = replace(
        "Crew",
        "- Ada, the **captain**\n- Cy, the cook\n- Di",
        false,
    )
    .unwrap();
    assert_eq!(replaced.headings, ["Crew"]);
    assert_eq!(
        replaced.snapshot.unwrap().label.as_deref(),
        Some("Before a section was replaced by Claude")
    );
    // The item that reads the same was not touched; "Ben" is gone and its comment has no
    // place any more.
    assert_eq!(
        (
            replaced.blocks_added,
            replaced.blocks_removed,
            replaced.blocks_changed
        ),
        (2, 1, 0)
    );
    assert_eq!(replaced.comments_detached.len(), 1);
    assert_eq!(replaced.comments_detached[0].id, ben);
    assert_eq!(
        fx.markdown(doc.id),
        CHAPTER.replace("- Ben\n", "- Cy, the cook\n- Di\n")
    );
    assert_eq!(anchors(&fx, doc.id)[0], (maren, Some("Maren".into())));

    // With the heading; the section of a top heading is everything under it.
    replace("## Cargo", "## Hold\n\nNothing.", true).unwrap();
    assert!(
        fx.markdown(doc.id)
            .ends_with("- Di\n\n## Hold\n\nNothing.\n")
    );
    let all = replace("The Harbour", "One paragraph.", false).unwrap();
    assert_eq!(fx.markdown(doc.id), "# The Harbour\n\nOne paragraph.\n");
    assert_eq!(all.headings, ["The Harbour"]);
    assert_eq!(all.comments_detached.len(), 1, "the comment on Maren");
    // An empty section.
    replace("The Harbour", "", false).unwrap();
    assert_eq!(fx.markdown(doc.id), "# The Harbour\n");
    assert!(matches!(
        replace("Crew", "x", false),
        Err(Error::NotFound {
            kind: ItemKind::Heading,
            ..
        })
    ));
}

#[test]
fn replacing_a_document_keeps_what_reads_the_same() {
    let fx = fixture();
    let doc = fx.create(None, CHAPTER);
    let maren = comment(&fx, doc.id, "Maren");
    let gulls = comment(&fx, doc.id, "gulls");
    let captain = comment(&fx, doc.id, "captain");
    let salt = comment(&fx, doc.id, "more salt");
    let before = fx.document(doc.id);
    let replace = |markdown: &str| {
        fx.library
            .replace_document(&ReplaceDocumentRequest {
                document: doc.id.into(),
                markdown: markdown.into(),
                write: claude(),
            })
            .unwrap()
    };

    // What an agent does: read the Markdown, change a few things, write it back.
    let changed = fx
        .markdown(doc.id)
        .replace("at dawn", "at first light")
        .replace("- Ben\n", "- Ben, the cook\n- Cy\n")
        .replace("## Cargo", "## The cargo");
    let result = replace(&changed);
    assert_eq!(fx.markdown(doc.id), changed);
    assert_eq!(
        (
            result.blocks_added,
            result.blocks_removed,
            result.blocks_changed
        ),
        (1, 0, 3)
    );
    assert_eq!(result.headings, ["The Harbour", "Crew", "The cargo"]);
    assert!(result.comments_detached.is_empty());
    assert!(result.warnings.is_empty());
    // Every comment is where it was, in changed paragraphs too.
    assert_eq!(
        anchors(&fx, doc.id),
        [
            (maren, Some("Maren".into())),
            (gulls, Some("gulls".into())),
            (captain, Some("captain".into())),
            (salt, Some("more salt".into())),
        ]
    );
    // The blocks are the blocks they were.
    let after = fx.document(doc.id);
    let ids = |document: &Document| document.blocks().iter().map(|b| b.id()).collect::<Vec<_>>();
    let mut expected = ids(&before);
    expected.insert(5, after.blocks()[5].id());
    assert_eq!(ids(&after), expected);
    assert_eq!(after.blocks()[0], before.blocks()[0]);
    assert_eq!(after.blocks()[3], before.blocks()[3]);

    // A paragraph that is rewritten takes its comments with the words that go.
    let rewritten = fx.markdown(doc.id).replace(
        "The *Maren* left at first light. The gulls left too.",
        "Nobody saw the ship leave.",
    );
    let result = replace(&rewritten);
    assert_eq!(fx.markdown(doc.id), rewritten);
    let detached: Vec<u64> = result.comments_detached.iter().map(|t| t.id).collect();
    assert_eq!(detached, [maren, gulls]);
    assert_eq!(result.comments_detached[0].body, "About Maren");
    assert_eq!(
        anchors(&fx, doc.id),
        [
            (captain, Some("captain".into())),
            (salt, Some("more salt".into())),
            (maren, None),
            (gulls, None),
        ]
    );
    // The same text again: nothing to do.
    let same = replace(&rewritten);
    assert!(!same.written.changed);
    assert_eq!(
        (same.blocks_added, same.blocks_removed, same.blocks_changed),
        (0, 0, 0)
    );

    // Formatting that Markdown says is brought to what the new text says.
    let unbold = fx.markdown(doc.id).replace("**captain**", "captain");
    replace(&unbold);
    assert_eq!(fx.markdown(doc.id), unbold);
    assert_eq!(anchors(&fx, doc.id)[0], (captain, Some("captain".into())));
    // Blocks in another order.
    let moved = "## The cargo\n\nSalt, and [more salt](https://example.com/salt).\n\n# The Harbour\n\nNobody saw the ship leave.\n";
    let result = replace(moved);
    assert_eq!(fx.markdown(doc.id), moved);
    assert!(result.blocks_removed >= 2);
    assert_eq!(fx.store().document_meta(doc.id).unwrap().title, "The cargo");
    // Everything gone: one empty paragraph, as an empty document is.
    let emptied = replace("");
    assert_eq!(fx.document(doc.id).block_count(), 1);
    assert_eq!(fx.markdown(doc.id), "");
    assert_eq!(emptied.words_after, 0);
    assert_eq!(fx.store().document_meta(doc.id).unwrap().title, "Untitled");
    // The threads are all still there, without a place.
    assert_eq!(anchors(&fx, doc.id).len(), 4);
    assert!(anchors(&fx, doc.id).iter().all(|(_, text)| text.is_none()));
    // And back from nothing.
    replace(CHAPTER);
    assert_eq!(fx.markdown(doc.id), CHAPTER);
}

#[test]
fn replacing_a_document_keeps_what_markdown_cannot_say() {
    let fx = fixture();
    let doc = fx.create(
        None,
        "# Title\n\nA red word and a plain one.\n\nSecond paragraph.\n",
    );
    let mut store = writer_store::Store::open(fx.library.database_path()).unwrap();
    let mut ed = writer_core::EditorState::new(fx.document(doc.id));
    ed.select_range(writer_core::TextRange::in_block(1, 2, 5));
    ed.set_text_color(Some(Rgb { r: 200, g: 0, b: 0 }));
    ed.select_block(2);
    ed.set_alignment(writer_core::Alignment::Right);
    let mut settings = ed.document().settings().clone();
    settings.page_view = true;
    settings.typography.body_font = "Georgia".into();
    ed.set_settings(settings.clone());
    let styled = ed.into_document();
    let stats = styled.stats();
    store
        .save_document(
            doc.id,
            &writer_store::DocumentContent {
                content: &styled.to_json(),
                title: "Title",
                plain_text: &styled.plain_text(),
                word_count: stats.words as u64,
                char_count: stats.characters as u64,
            },
        )
        .unwrap();
    store.close().unwrap();

    let changed = fx.markdown(doc.id).replace("a plain one", "a **bold** one");
    assert!(
        !changed.contains("color"),
        "the colour is not in the Markdown: {changed}"
    );
    fx.library
        .replace_document(&ReplaceDocumentRequest {
            document: doc.id.into(),
            markdown: changed.clone(),
            write: claude(),
        })
        .unwrap();
    let after = fx.document(doc.id);
    assert_eq!(fx.markdown(doc.id), changed);
    assert_eq!(
        after.blocks()[1].attrs_at(3).unwrap().color,
        Some(Rgb { r: 200, g: 0, b: 0 })
    );
    assert_eq!(after.blocks()[1].attrs_at(8).unwrap().color, None);
    assert_eq!(after.blocks()[2].alignment(), writer_core::Alignment::Right);
    assert_eq!(after.settings(), &settings.sanitized());
}

/// A document with one of everything the app can make, stored as the app stores it.
fn everything(fx: &Fixture) -> DocumentId {
    use writer_core::{
        Alignment, Block, EditorState, Highlight, InlineAttrs, InlineStyle, TextRange,
    };
    let doc = fx.create(None, "placeholder");
    let plain = InlineAttrs::default;
    let blocks = vec![
        Block::with_text(BlockKind::heading(1), "Everything"),
        Block::paragraph("Plain, ")
            .with_span("bold", plain().with(InlineStyle::Bold))
            .with_span(", ", plain())
            .with_span("italic", plain().with(InlineStyle::Italic))
            .with_span(", ", plain())
            .with_span("underlined", plain().with(InlineStyle::Underline))
            .with_span(", ", plain())
            .with_span("struck", plain().with(InlineStyle::Strikethrough))
            .with_span(", ", plain())
            .with_span("code", plain().with(InlineStyle::Code))
            .with_span(" and ", plain())
            .with_span(
                "bold italic",
                plain().with(InlineStyle::Bold).with(InlineStyle::Italic),
            )
            .with_span(".", plain()),
        Block::paragraph(""),
        Block::paragraph("A line\nand a second line after a soft break, with trailing spaces.  "),
        Block::with_text(BlockKind::heading(2), "Lists"),
        Block::with_text(BlockKind::BulletItem, "One"),
        Block::with_text(BlockKind::BulletItem, "Nested").with_indent(1),
        Block::with_text(BlockKind::NumberedItem, "First"),
        Block::with_text(BlockKind::NumberedItem, "Second"),
        Block::with_text(BlockKind::TodoItem { checked: false }, "To do"),
        Block::with_text(BlockKind::TodoItem { checked: true }, "Done"),
        Block::with_text(BlockKind::Quote, "A quote."),
        Block::with_text(
            BlockKind::CodeBlock {
                language: Some("rust".into()),
            },
            "fn main() {\n    println!(\"hi\");\n}",
        ),
        Block::new(BlockKind::Divider),
        Block::paragraph("Centred, large and partly coloured.")
            .with_alignment(Alignment::Center)
            .with_text_size(Some(24.0)),
        Block::new(BlockKind::PageBreak),
        Block::paragraph(""),
        Block::paragraph(
            "Unicode: caf\u{e9}, \u{6771}\u{4eac}, \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}, 5 * 3 = 15, a_b_c, [not a link], <tag>.",
        ),
        Block::paragraph(""),
    ];
    let mut ed = EditorState::new(Document::from_blocks(blocks));
    ed.select_range(TextRange::in_block(14, 9, 14));
    ed.set_text_color(Some(Rgb {
        r: 10,
        g: 120,
        b: 200,
    }));
    ed.set_font_family(Some("Georgia"));
    ed.select_range(TextRange::in_block(14, 19, 25));
    ed.set_highlight(Some(Highlight::Green));
    let document = ed.into_document();
    let stats = document.stats();
    let mut store = writer_store::Store::open(fx.library.database_path()).unwrap();
    store
        .save_document(
            doc.id,
            &writer_store::DocumentContent {
                content: &document.to_json(),
                title: "Everything",
                plain_text: &document.plain_text(),
                word_count: stats.words as u64,
                char_count: stats.characters as u64,
            },
        )
        .unwrap();
    store.close().unwrap();
    doc.id
}

#[test]
fn writing_back_what_was_read_changes_nothing() {
    let fx = fixture();
    let id = everything(&fx);
    let before = fx.document(id);
    let markdown = fx.markdown(id);
    let result = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: id.into(),
            markdown: markdown.clone(),
            write: claude(),
        })
        .unwrap();
    let after = fx.document(id);
    for (index, (old, new)) in before.blocks().iter().zip(after.blocks()).enumerate() {
        assert_eq!(old, new, "block {index} of:\n{markdown}");
    }
    assert_eq!(before, after, "{markdown}");
    assert!(!result.written.changed, "{result:?}");
    assert_eq!(
        (
            result.blocks_added,
            result.blocks_removed,
            result.blocks_changed
        ),
        (0, 0, 0)
    );
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert!(result.snapshot.is_none());
    // Section by section too.
    for heading in ["Everything", "Lists"] {
        let section = fx
            .library
            .read(&ReadRequest {
                document: id.into(),
                heading: Some(heading.into()),
                ..Default::default()
            })
            .unwrap();
        let result = fx
            .library
            .replace_section(&ReplaceSectionRequest {
                document: id.into(),
                heading: heading.into(),
                markdown: section.content,
                include_heading: true,
                write: claude(),
                ..Default::default()
            })
            .unwrap();
        assert!(!result.written.changed, "{heading}: {result:?}");
    }
    assert_eq!(fx.document(id), before);

    // One change among all of it touches one block.
    let result = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: id.into(),
            markdown: markdown.replace("A quote.", "A longer quote."),
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        (
            result.blocks_added,
            result.blocks_removed,
            result.blocks_changed
        ),
        (0, 0, 1)
    );
    let after = fx.document(id);
    assert_eq!(after.block_count(), before.block_count());
    assert_eq!(
        after.blocks()[14],
        before.blocks()[14],
        "colour, font, size and alignment stay"
    );
    assert_eq!(
        after.blocks()[2],
        before.blocks()[2],
        "an empty paragraph stays"
    );
}
