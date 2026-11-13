//! Images: reading them, adding them from a file, bringing them in with Markdown, and
//! keeping them, with their size, through every change an agent makes.

use std::path::Path;
use std::time::Duration;

use writer_agent::live::{self, Answer, MAX_ATTACHED_BYTES, Operation, Request};
use writer_agent::{
    AddCommentRequest, AddImageRequest, AttachedImage, ContentChanged, CreateDocumentRequest,
    DocumentSummary, DuplicateDocumentRequest, EditRequest, Error, ExportFormat, ExportRequest,
    FindRequest, ImageSummary, InsertPosition, InsertRequest, ListVersionsRequest, MatchedAs,
    ReadRequest, ReadVersionRequest, ReplaceDocumentRequest, ReplaceSectionRequest,
    RestoreDocumentRequest, RestoreVersionRequest, Target, TextFormat, TrashDocumentRequest,
    WriteOptions,
};
use writer_core::{Alignment, BlockKind, Image, ImageWidth};
use writer_store::{DocumentId, Store};

use super::rules::dry;
use super::{Fixture, claude, closed_fixture, fixture};

const HARBOUR: &str = "# Harbour\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n";
const DUSK: &str = "The harbour at dusk";

/// The name of the stored bytes of a picture file.
fn id_of(file: &Path) -> String {
    writer_store::ImageId::of(&std::fs::read(file).unwrap()).to_hex()
}

fn stored(id: &str) -> writer_store::ImageId {
    writer_store::ImageId::parse(id).expect("32 hex digits")
}

/// The line an image is in Markdown.
fn line(description: &str, id: &str) -> String {
    format!("![{description}](image:{id})")
}

fn path_of(file: &Path) -> String {
    file.to_string_lossy().into_owned()
}

/// A document "Harbour" whose first block under the heading is an image: 800 × 500
/// pixels, 37 % of the column wide, centred, with a description. Its id with it.
fn harbour(fx: &Fixture) -> (DocumentSummary, String) {
    let doc = fx.create(None, HARBOUR);
    let file = fx.picture("harbour.png", (800, 500), [30, 90, 160]);
    let added = fx
        .library
        .add_image(&AddImageRequest {
            document: doc.id.into(),
            path: path_of(&file),
            description: DUSK.into(),
            width_percent: Some(37),
            position: InsertPosition::AfterHeading,
            heading: Some("Harbour".into()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(added.images_added, 1);
    (doc, id_of(&file))
}

/// The image of the harbour as the document should still hold it.
#[track_caller]
fn assert_as_it_was(image: &Image, alignment: Alignment, id: &str) {
    assert_eq!(image.id.to_string(), id);
    assert_eq!(image.width, ImageWidth::from_percent(37));
    assert_eq!(image.description, DUSK);
    assert_eq!(image.pixels, (800, 500));
    assert_eq!(alignment, Alignment::Center);
}

fn markdown(fx: &Fixture, id: DocumentId) -> String {
    fx.markdown(id)
}

fn replace(fx: &Fixture, id: DocumentId, markdown: &str) -> ContentChanged {
    fx.clock.advance_ms(1000);
    fx.library
        .replace_document(&ReplaceDocumentRequest {
            document: id.into(),
            markdown: markdown.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
}

fn edit(fx: &Fixture, id: DocumentId, old: &str, new: &str) -> Result<ContentChanged, Error> {
    fx.clock.advance_ms(1000);
    fx.library.edit(&EditRequest {
        document: id.into(),
        old_text: old.into(),
        new_text: new.into(),
        write: claude(),
        ..Default::default()
    })
}

fn insert(fx: &Fixture, id: DocumentId, markdown: &str) -> ContentChanged {
    fx.clock.advance_ms(1000);
    fx.library
        .insert(&InsertRequest {
            document: id.into(),
            markdown: markdown.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
}

/// The kinds of the blocks of a stored document, images by "image".
fn kinds(fx: &Fixture, id: DocumentId) -> Vec<String> {
    fx.document(id)
        .blocks()
        .iter()
        .map(|block| match block.kind() {
            BlockKind::Image(_) => "image".to_string(),
            _ => block.text().to_string(),
        })
        .collect()
}

// ----- adding and reading -------------------------------------------------------------------

#[test]
fn an_image_is_added_from_a_file_and_read_as_a_line_of_markdown() {
    let fx = fixture();
    let doc = fx.create(None, HARBOUR);
    let file = fx.picture("harbour.png", (800, 500), [30, 90, 160]);
    let bytes = std::fs::read(&file).unwrap();
    let id = id_of(&file);
    fx.clock.advance_ms(1000);
    let added = fx
        .library
        .add_image(&AddImageRequest {
            document: doc.id.into(),
            path: path_of(&file),
            description: DUSK.into(),
            width_percent: Some(37),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(added.written.changed && !added.written.dry_run);
    assert_eq!(
        (added.images_added, added.images_removed, added.blocks_added),
        (1, 0, 1)
    );
    assert_eq!(added.warnings, [] as [&str; 0]);
    // The result says what was stored: the width, and that it is centred.
    let image = ImageSummary {
        id: id.clone(),
        block: 6,
        format: Some("png".into()),
        pixels: (800, 500),
        width_percent: 37,
        align: "center".into(),
        description: DUSK.into(),
        missing: false,
    };
    assert_eq!(added.image, Some(image.clone()));
    // The text as it was is kept, as before every change of an agent's.
    let kept = added.snapshot.expect("a version is kept");
    assert_eq!(kept.label.as_deref(), Some("Before an insertion by Claude"));
    // No word was added: an image is no text.
    assert_eq!(added.words_before, added.words_after);

    // The library has the bytes of the file, as they were (a PNG is stored as it came),
    // and the document names them by the token the store looks for.
    let store = fx.store();
    assert_eq!(store.image_count().unwrap(), 1);
    assert_eq!(store.image_bytes(&stored(&id)).unwrap(), Some(bytes));
    assert!(
        store.referenced_images().unwrap().contains(&stored(&id)),
        "what writer-core writes for an image is what the store finds: {}",
        store.document(doc.id).unwrap().content
    );
    let (index, block, alignment) = fx.images(doc.id).remove(0);
    assert_eq!(index, 6);
    assert_as_it_was(&block, alignment, &id);

    // Read: in Markdown a line that names the image; the images are listed beside it.
    let read = fx
        .library
        .read(&ReadRequest {
            document: doc.id.into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(read.content, format!("{HARBOUR}\n{}\n", line(DUSK, &id)));
    assert_eq!(read.images, std::slice::from_ref(&image));
    // As plain text: what it shows, on a line of its own. As JSON: the block.
    let text = fx.read(doc.id, TextFormat::PlainText);
    assert!(text.ends_with(&format!("Five.\n\n{DUSK}\n")), "{text}");
    let json = fx.read(doc.id, TextFormat::Json);
    assert!(json.contains(&format!("\"image\":\"{id}\"")), "{json}");
    assert!(json.contains("\"width\":370"), "{json}");
    // A section has the images that are in it, and they are numbered as the document
    // numbers its blocks.
    let section = |heading: &str| {
        fx.library
            .read(&ReadRequest {
                document: doc.id.into(),
                heading: Some(heading.into()),
                ..Default::default()
            })
            .unwrap()
    };
    assert_eq!(section("Harbour").images, std::slice::from_ref(&image));
    // The search of the library and the counts know nothing of an image.
    assert_eq!(read.document.words, doc.words);

    // Where it goes: at the start, and at a heading, as an insertion does.
    let small = fx.picture("small.png", (200, 100), [200, 40, 40]);
    let small_id = id_of(&small);
    let notes = fx.create(
        None,
        "# Top\n\nText.\n\n## Notes\n\nA note.\n\n## After\n\nEnd.\n",
    );
    let place = |position: InsertPosition, heading: Option<&str>| {
        fx.clock.advance_ms(1000);
        fx.library
            .add_image(&AddImageRequest {
                document: notes.id.into(),
                path: path_of(&small),
                position,
                heading: heading.map(str::to_string),
                write: claude(),
                ..Default::default()
            })
            .unwrap()
            .image
            .unwrap()
    };
    let first = place(InsertPosition::Start, None);
    assert_eq!(first.block, 0);
    // Without a width: the picture's own size on the page (200 points of a column of
    // about 451), in the middle of the line. Without a description: none.
    let page = fx.document(notes.id).settings().page;
    let own = Image::new(
        writer_core::ImageId::parse(&small_id).unwrap(),
        (200, 100),
        72,
    )
    .original_width(&page);
    assert_eq!(own.percent(), 44);
    assert_eq!(
        (
            first.width_percent,
            first.align.as_str(),
            first.description.as_str()
        ),
        (44, "center", "")
    );
    assert_eq!(fx.images(notes.id)[0].1.width, own);
    assert_eq!(place(InsertPosition::AfterHeading, Some("Notes")).block, 4);
    assert_eq!(place(InsertPosition::EndOfSection, Some("Notes")).block, 6);
    assert_eq!(
        place(InsertPosition::BeforeHeading, Some("## After")).block,
        7
    );
    assert_eq!(place(InsertPosition::End, None).block, 10);
    assert_eq!(
        kinds(&fx, notes.id),
        [
            "image", "Top", "Text.", "Notes", "image", "A note.", "image", "image", "After",
            "End.", "image"
        ]
    );
    assert_eq!(
        markdown(&fx, notes.id),
        format!(
            "{image}\n\n# Top\n\nText.\n\n## Notes\n\n{image}\n\nA note.\n\n{image}\n\n{image}\n\n## After\n\nEnd.\n\n{image}\n",
            image = line("", &small_id)
        )
    );
    // The same file five times is stored once.
    assert_eq!(fx.store().image_count().unwrap(), 2);

    // A document without text: the image takes the place of its empty line.
    let empty = fx.create(None, "");
    fx.library
        .add_image(&AddImageRequest {
            document: empty.id.into(),
            path: path_of(&small),
            align: Some("right".into()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(kinds(&fx, empty.id), ["image"]);
    assert_eq!(fx.images(empty.id)[0].2, Alignment::Right);
}

#[test]
fn an_image_is_never_stored_taller_than_a_page_and_the_result_says_the_width() {
    let fx = fixture();
    let doc = fx.create(None, "# Tall\n\nText.\n");
    // Ten times as high as wide: at the width of the column it would be as high as
    // six pages.
    let tall = fx.picture("tall.png", (100, 1000), [10, 10, 10]);
    let add = |width_percent: Option<u16>| {
        fx.clock.advance_ms(1000);
        fx.library
            .add_image(&AddImageRequest {
                document: doc.id.into(),
                path: path_of(&tall),
                width_percent,
                align: Some("left".into()),
                write: claude(),
                ..Default::default()
            })
            .unwrap()
            .image
            .unwrap()
    };
    let page = fx.document(doc.id).settings().page;
    let image = Image::new(
        writer_core::ImageId::parse(&id_of(&tall)).unwrap(),
        (100, 1000),
        72,
    );
    let most = image.max_width(&page);
    assert!(most < ImageWidth::from_percent(20), "{most:?}");
    let wide = add(Some(80));
    assert_eq!(wide.width_percent, most.percent());
    assert_eq!(wide.align, "left");
    assert_eq!(fx.images(doc.id)[0].1.width, most);
    // A width the page allows is the width.
    assert_eq!(add(Some(10)).width_percent, 10);
    // Its own size is not taller than a page either.
    assert_eq!(add(None).width_percent, most.percent());
}

#[test]
fn what_cannot_be_added_says_why_and_changes_nothing() {
    let fx = fixture();
    let doc = fx.create(None, HARBOUR);
    let file = fx.picture("harbour.png", (80, 50), [30, 90, 160]);
    let before = fx.dump();
    let add = |request: AddImageRequest| {
        fx.library.add_image(&AddImageRequest {
            document: doc.id.into(),
            write: claude(),
            ..request
        })
    };
    #[track_caller]
    fn invalid(result: Result<ContentChanged, Error>) -> String {
        match result {
            Err(error @ Error::Invalid { .. }) => {
                assert_eq!(error.exit_code(), 2);
                error.to_string()
            }
            other => panic!("{other:?}"),
        }
    }
    let good = || AddImageRequest {
        path: path_of(&file),
        ..Default::default()
    };
    // The file. (Its path is the subject of the sentence, in quotation marks; a very
    // long one is shortened in the middle, so the name at its end is what is looked at.)
    #[track_caller]
    fn says(message: &str, before: &str, after: &str) {
        assert!(
            message.starts_with(before) && message.ends_with(after),
            "{message}"
        );
    }
    let missing = fx.dir.path().join("no such picture.png");
    says(
        &invalid(add(AddImageRequest {
            path: path_of(&missing),
            ..Default::default()
        })),
        "there is no file \u{201c}/",
        "no such picture.png\u{201d}; give the path of an image file",
    );
    let pdf = fx.dir.path().join("notes.pdf");
    std::fs::write(&pdf, b"%PDF-1.7\n%...\n").unwrap();
    says(
        &invalid(add(AddImageRequest {
            path: path_of(&pdf),
            ..Default::default()
        })),
        "\u{201c}/",
        "notes.pdf\u{201d} is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP.",
    );
    let words = fx.dir.path().join("words.png");
    std::fs::write(&words, b"not a picture at all").unwrap();
    says(
        &invalid(add(AddImageRequest {
            path: path_of(&words),
            ..Default::default()
        })),
        "\u{201c}/",
        "words.png\u{201d} is not an image.",
    );
    assert_eq!(
        invalid(add(AddImageRequest::default())),
        "name the image file to add"
    );
    // A path that starts nowhere: whoever has no directory to work in (the MCP
    // server) asks for the whole path.
    assert_eq!(
        invalid(add(AddImageRequest {
            path: "harbour.png".into(),
            ..Default::default()
        })),
        "give the absolute path of the image file"
    );
    assert!(
        invalid(add(AddImageRequest {
            path: "https://example.com/harbour.png".into(),
            ..Default::default()
        }))
        .contains("is not a file on this Mac")
    );
    // A file in the home folder is looked for there (this one is not there; nothing of
    // the home folder is read or written).
    let home = invalid(add(AddImageRequest {
        path: "~/no-such-folder-of-the-writers-tests/harbour.png".into(),
        ..Default::default()
    }));
    assert!(home.starts_with("there is no file"), "{home}");
    // How and where.
    for width in [0, 4, 101, 1000] {
        assert!(
            invalid(add(AddImageRequest {
                width_percent: Some(width),
                ..good()
            }))
            .contains("from 5 to 100 percent"),
            "{width}"
        );
    }
    for align in ["justify", "middle"] {
        assert!(
            invalid(add(AddImageRequest {
                align: Some(align.into()),
                ..good()
            }))
            .contains("\"left\", \"center\" or \"right\""),
            "{align}"
        );
    }
    assert!(
        invalid(add(AddImageRequest {
            position: InsertPosition::AfterHeading,
            ..good()
        }))
        .contains("name the heading")
    );
    assert!(
        invalid(add(AddImageRequest {
            heading: Some("Harbour".into()),
            ..good()
        }))
        .contains("not relative to one")
    );
    assert!(matches!(
        add(AddImageRequest {
            position: InsertPosition::AfterHeading,
            heading: Some("No such heading".into()),
            ..good()
        }),
        Err(Error::NotFound { .. })
    ));
    assert!(matches!(
        fx.library.add_image(&AddImageRequest {
            document: Target::path("No such document"),
            write: claude(),
            ..good()
        }),
        Err(Error::NotFound { .. })
    ));
    // Nothing of all this stored anything, not even the bytes of the good file.
    assert_eq!(fx.dump(), before);
    assert!(!fx.images_path().exists());

    // With a directory to start from, a relative path is a file: the request's own
    // directory, or the one the library was given.
    let relative = AddImageRequest {
        path: "harbour.png".into(),
        ..Default::default()
    };
    let with_request = add(AddImageRequest {
        base_dir: Some(fx.dir.path().to_path_buf()),
        ..relative.clone()
    })
    .unwrap();
    assert_eq!(with_request.images_added, 1);
    let library = fx.library.clone().with_base_dir(fx.dir.path());
    let with_library = library
        .add_image(&AddImageRequest {
            document: doc.id.into(),
            write: claude(),
            ..relative
        })
        .unwrap();
    assert_eq!(with_library.image.unwrap().id, id_of(&file));
    assert_eq!(fx.images(doc.id).len(), 2);
}

#[test]
fn a_dry_run_of_an_image_stores_nothing_and_reports_the_same() {
    let fx = fixture();
    let doc = fx.create(None, HARBOUR);
    let file = fx.picture("harbour.png", (800, 500), [30, 90, 160]);
    let other = fx.picture("other.png", (64, 64), [200, 200, 0]);
    let before = fx.dump();
    let add = |write: WriteOptions| {
        fx.library
            .add_image(&AddImageRequest {
                document: doc.id.into(),
                path: path_of(&file),
                description: DUSK.into(),
                width_percent: Some(37),
                write,
                ..Default::default()
            })
            .unwrap()
    };
    let bring = |write: WriteOptions| {
        fx.library
            .insert(&InsertRequest {
                document: doc.id.into(),
                markdown: format!(
                    "![A yellow square]({})\n\n![Nothing](/no/such/picture.png)\n",
                    other.display()
                ),
                write,
                ..Default::default()
            })
            .unwrap()
    };
    let tried = add(dry());
    assert!(tried.written.dry_run && !tried.written.changed && tried.written.would_change);
    assert_eq!(tried.snapshot, None);
    let brought = bring(dry());
    assert_eq!((brought.images_added, brought.blocks_added), (1, 2));
    assert_eq!(fx.dump(), before, "a dry run changed the library");
    assert!(
        !fx.images_path().exists(),
        "a dry run made the file of the pictures"
    );

    // The real runs report what the dry runs reported.
    let done = add(claude());
    assert_eq!(done.image, tried.image);
    assert_eq!(
        (done.images_added, done.blocks_added, done.words_after),
        (tried.images_added, tried.blocks_added, tried.words_after)
    );
    let really = bring(claude());
    assert_eq!(really.warnings, brought.warnings);
    assert_eq!(
        (really.images_added, really.blocks_added),
        (brought.images_added, brought.blocks_added)
    );
    assert_eq!(fx.store().image_count().unwrap(), 2);
    // And once the bytes are there, a dry run still writes nothing.
    let stored_before = fx.dump();
    assert_eq!(
        add(dry()).image.map(|image| image.id),
        done.image.map(|image| image.id)
    );
    bring(dry());
    assert_eq!(fx.dump(), stored_before);
}

// ----- keeping ----------------------------------------------------------------------------------

#[test]
fn a_text_that_is_written_back_keeps_its_image_as_it_was() {
    let fx = fixture();
    let (doc, id) = harbour(&fx);
    let read = markdown(&fx, doc.id);
    let image_line = line(DUSK, &id);
    assert_eq!(
        read,
        format!("# Harbour\n\n{image_line}\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n")
    );
    let before = fx.document(doc.id);
    let block_id = before.blocks()[1].id();

    // What was read, written back: nothing changes.
    let same = replace(&fx, doc.id, &read);
    assert!(!same.written.changed, "{same:?}");
    assert_eq!((same.images_added, same.images_removed), (0, 0));
    assert_eq!(fx.document(doc.id), before);

    // A word changed elsewhere: the image block is the block it was.
    let edited = replace(&fx, doc.id, &read.replace("Two.", "Two, again."));
    assert_eq!(
        (
            edited.blocks_changed,
            edited.blocks_added,
            edited.blocks_removed
        ),
        (1, 0, 0)
    );
    let after = fx.document(doc.id);
    assert_eq!(after.blocks()[1], before.blocks()[1]);
    assert_eq!(after.blocks()[1].id(), block_id);
    assert_eq!(edited.warnings, [] as [&str; 0]);

    // Other words under the picture: the description changes, and nothing else of it.
    let now = markdown(&fx, doc.id);
    let described = replace(&fx, doc.id, &now.replace(DUSK, "Boats, and a low sun"));
    assert_eq!(
        (
            described.blocks_changed,
            described.blocks_added,
            described.blocks_removed
        ),
        (1, 0, 0)
    );
    let (index, image, alignment) = fx.images(doc.id).remove(0);
    assert_eq!(index, 1);
    assert_eq!(fx.document(doc.id).blocks()[1].id(), block_id);
    assert_eq!(image.description, "Boats, and a low sun");
    assert_eq!(
        (image.width, alignment),
        (ImageWidth::from_percent(37), Alignment::Center)
    );
    // A line without words under it keeps the ones the image had (Markdown has no way
    // to say "none").
    let wordless = replace(
        &fx,
        doc.id,
        &markdown(&fx, doc.id).replace("Boats, and a low sun", ""),
    );
    assert!(!wordless.written.changed);
    replace(
        &fx,
        doc.id,
        &markdown(&fx, doc.id).replace("Boats, and a low sun", DUSK),
    );

    // The image line three blocks further down: the image is there as it was (it is a
    // block that was taken out and put in, so its own id may be another).
    let moved =
        format!("# Harbour\n\nOne.\n\nTwo, again.\n\nThree.\n\n{image_line}\n\nFour.\n\nFive.\n");
    let changed = replace(&fx, doc.id, &moved);
    assert!(changed.written.changed);
    assert_eq!(
        (changed.images_added, changed.images_removed),
        (0, 0),
        "an image that changed its place is neither new nor gone"
    );
    assert_eq!(changed.warnings, [] as [&str; 0]);
    let (index, image, alignment) = fx.images(doc.id).remove(0);
    assert_eq!(index, 4);
    assert_as_it_was(&image, alignment, &id);
    assert_eq!(markdown(&fx, doc.id), moved);
    assert_eq!(fx.store().image_count().unwrap(), 1);
}

#[test]
fn an_image_that_is_named_again_comes_as_the_document_has_it() {
    let fx = fixture();
    let (doc, id) = harbour(&fx);
    let image_line = line(DUSK, &id);

    // A section is given the image line of another section: the image comes with its
    // width, its place on the line and its description, and stays where it was too.
    let two = fx.create(None, "# First\n\nText one.\n\n# Second\n\nText two.\n");
    fx.library
        .add_image(&AddImageRequest {
            document: two.id.into(),
            path: path_of(&fx.dir.path().join("harbour.png")),
            description: DUSK.into(),
            width_percent: Some(37),
            position: InsertPosition::AfterHeading,
            heading: Some("First".into()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    fx.clock.advance_ms(1000);
    let section = fx
        .library
        .replace_section(&ReplaceSectionRequest {
            document: two.id.into(),
            heading: "Second".into(),
            markdown: format!("Text two.\n\n![]({})\n", format_args!("image:{id}")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((section.images_added, section.images_removed), (1, 0));
    let images = fx.images(two.id);
    assert_eq!(images.len(), 2);
    assert_eq!(images[1].0, 5);
    for (_, image, alignment) in &images {
        assert_as_it_was(image, *alignment, &id);
    }

    // An insertion that names an image the document has: a second block like the first.
    let again = insert(
        &fx,
        doc.id,
        &format!("And once more:\n\n![]({})", format_args!("image:{id}")),
    );
    assert_eq!((again.images_added, again.blocks_added), (1, 2));
    let images = fx.images(doc.id);
    assert_eq!(images.len(), 2);
    assert_as_it_was(&images[1].1, images[1].2, &id);
    assert_eq!(fx.store().image_count().unwrap(), 1);

    // An image of the library that this document does not have comes at its own size,
    // at the left: there is nothing here to take a size from.
    let other = fx.create(None, "# Elsewhere\n\nText.\n");
    let brought = insert(&fx, other.id, &image_line);
    assert_eq!(brought.images_added, 1);
    assert_eq!(brought.warnings, [] as [&str; 0]);
    let (_, image, alignment) = fx.images(other.id).remove(0);
    let page = fx.document(other.id).settings().page;
    assert_eq!(image.width, image.original_width(&page));
    assert_eq!(
        image.width,
        ImageWidth::FULL,
        "800 points are wider than the column"
    );
    assert_eq!(alignment, Alignment::Left);
    assert_eq!(image.description, DUSK, "the words of the line");

    // An id nobody has stays a link, and is said.
    let unknown = "00000000000000000000000000000000";
    let nobody = insert(&fx, other.id, &line("A ghost", unknown));
    assert_eq!((nobody.images_added, nobody.blocks_added), (0, 1));
    assert_eq!(
        nobody.warnings,
        [format!(
            "\u{201c}image:{unknown}\u{201d} names an image the library does not have. It was kept as a link."
        )]
    );
    assert!(
        markdown(&fx, other.id).ends_with(&format!("[A ghost](image:{unknown})\n")),
        "{}",
        markdown(&fx, other.id)
    );
}

#[test]
fn one_picture_at_two_sizes_stays_two_sizes() {
    let fx = fixture();
    let doc = fx.create(None, "First.\n\nSecond.\n");
    let file = fx.picture("harbour.png", (800, 500), [30, 90, 160]);
    let id = id_of(&file);
    let add = |percent: u16, position: InsertPosition, align: &str| {
        fx.clock.advance_ms(1000);
        fx.library
            .add_image(&AddImageRequest {
                document: doc.id.into(),
                path: path_of(&file),
                width_percent: Some(percent),
                align: Some(align.into()),
                position,
                write: claude(),
                ..Default::default()
            })
            .unwrap();
    };
    add(25, InsertPosition::Start, "left");
    add(75, InsertPosition::End, "right");
    let image_line = line("", &id);
    assert_eq!(
        markdown(&fx, doc.id),
        format!("{image_line}\n\nFirst.\n\nSecond.\n\n{image_line}\n")
    );
    // The two paragraphs between them change places.
    let swapped = format!("{image_line}\n\nSecond.\n\nFirst.\n\n{image_line}\n");
    let changed = replace(&fx, doc.id, &swapped);
    assert!(changed.written.changed);
    assert_eq!((changed.images_added, changed.images_removed), (0, 0));
    let sizes: Vec<(usize, u16, Alignment)> = fx
        .images(doc.id)
        .into_iter()
        .map(|(index, image, alignment)| (index, image.width.percent(), alignment))
        .collect();
    assert_eq!(sizes, [(0, 25, Alignment::Left), (3, 75, Alignment::Right)]);
    assert_eq!(kinds(&fx, doc.id), ["image", "Second.", "First.", "image"]);
    // Read, the two listed with what Markdown does not say.
    let read = fx
        .library
        .read(&ReadRequest {
            document: doc.id.into(),
            ..Default::default()
        })
        .unwrap();
    let listed: Vec<(usize, u16, &str)> = read
        .images
        .iter()
        .map(|image| (image.block, image.width_percent, image.align.as_str()))
        .collect();
    assert_eq!(listed, [(0, 25, "left"), (3, 75, "right")]);
}

#[test]
fn an_edit_never_takes_an_image_it_does_not_name() {
    let fx = fixture();
    let (doc, id) = harbour(&fx);
    let image_line = line(DUSK, &id);
    let before = fx.document(doc.id);

    // The text before the image and the text after it, with the empty line the image
    // is in that text between them: found, and refused.
    let found = fx
        .library
        .find(&FindRequest {
            document: doc.id.into(),
            text: "Harbour\n\nOne.".into(),
            limit: None,
        })
        .unwrap();
    assert_eq!((found.count, found.matched_as), (1, Some(MatchedAs::Text)));
    for (old, new) in [
        ("Harbour\n\nOne.", "Harbour. One."),
        ("Harbour\n\nOne.", ""),
        ("Harbour\n\nOne", "The harbour\n\nOne"),
    ] {
        match edit(&fx, doc.id, old, new) {
            Err(error @ Error::Invalid { .. }) => assert_eq!(
                error.to_string(),
                "the text to replace runs across an image; read the place again and replace \
                 the text before and after the image separately, or name the image as it \
                 reads in Markdown"
            ),
            other => panic!("{old:?}: {other:?}"),
        }
    }
    // Nor does Markdown that names something else where the image stands.
    assert!(matches!(
        edit(&fx, doc.id, "# Harbour\n\n---\n\nOne.", "x"),
        Err(Error::NoMatch { .. })
    ));
    let elsewhere = "ffffffffffffffffffffffffffffffff";
    assert!(matches!(
        edit(
            &fx,
            doc.id,
            &format!("# Harbour\n\n{}\n\nOne.", line(DUSK, elsewhere)),
            "x"
        ),
        Err(Error::NoMatch { .. })
    ));
    assert_eq!(
        fx.document(doc.id),
        before,
        "a refused edit changed the text"
    );

    // An edit beside the image leaves it alone.
    let beside = edit(&fx, doc.id, "One.", "One, changed.").unwrap();
    assert_eq!((beside.images_added, beside.images_removed), (0, 0));
    assert_eq!(fx.document(doc.id).blocks()[1], before.blocks()[1]);
    edit(&fx, doc.id, "Harbour", "The harbour").unwrap();
    assert_eq!(fx.document(doc.id).blocks()[1], before.blocks()[1]);

    // Named as it reads in Markdown, the image is part of what is replaced, and the new
    // text says what becomes of it. Named again: it stays as it was.
    let kept = edit(
        &fx,
        doc.id,
        &format!("# The harbour\n\n{image_line}\n\nOne, changed."),
        &format!("# Harbour\n\n{image_line}\n\nOne."),
    )
    .unwrap();
    assert_eq!(kept.matched_as, Some(MatchedAs::Markdown));
    assert_eq!((kept.images_added, kept.images_removed), (0, 0));
    assert_eq!(fx.document(doc.id).blocks()[1], before.blocks()[1]);
    assert_eq!(markdown(&fx, doc.id), markdown_of(&image_line));
    // Something put between the heading and the image: the image moves down a block.
    let across = edit(
        &fx,
        doc.id,
        &format!("# Harbour\n\n{image_line}\n\nOne."),
        &format!("# Harbour\n\nIn between.\n\n{image_line}\n\nOne."),
    )
    .unwrap();
    assert_eq!((across.images_added, across.images_removed), (0, 0));
    let (index, image, alignment) = fx.images(doc.id).remove(0);
    assert_eq!(index, 2);
    assert_as_it_was(&image, alignment, &id);

    // The image line with the paragraph before it, or after it.
    let after = edit(
        &fx,
        doc.id,
        &format!("In between.\n\n{image_line}"),
        &image_line,
    )
    .unwrap();
    assert_eq!(after.matched_as, Some(MatchedAs::Markdown));
    assert_eq!((after.blocks_removed, after.images_removed), (1, 0));
    assert_eq!(markdown(&fx, doc.id), markdown_of(&image_line));
    let (_, image, alignment) = fx.images(doc.id).remove(0);
    assert_as_it_was(&image, alignment, &id);

    // The image line alone: replaced by words, the image is gone and the result says so.
    let gone = edit(&fx, doc.id, &image_line, "Here was a picture.").unwrap();
    assert_eq!(gone.matched_as, Some(MatchedAs::Markdown));
    assert_eq!((gone.images_added, gone.images_removed), (0, 1));
    assert_eq!(
        gone.warnings,
        ["1 image was removed; it is in the version kept before this change"]
    );
    assert_eq!(markdown(&fx, doc.id), markdown_of("Here was a picture."));
    // The version kept before that edit has it, and the library keeps its bytes.
    let version = gone.snapshot.unwrap();
    let was = fx
        .library
        .read_version(&ReadVersionRequest {
            version: version.id.0,
            ..Default::default()
        })
        .unwrap();
    assert!(was.content.contains(&image_line), "{}", was.content);
    assert!(
        fx.store()
            .referenced_images()
            .unwrap()
            .contains(&stored(&id))
    );
    assert_eq!(fx.store().image_count().unwrap(), 1);
    // Named with the paragraph after it and not named in the new text: gone with it.
    fx.clock.advance_ms(1000);
    fx.library
        .restore_version(&RestoreVersionRequest {
            version: version.id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(markdown(&fx, doc.id), markdown_of(&image_line));
    let both = edit(&fx, doc.id, &format!("{image_line}\n\nOne."), "").unwrap();
    assert_eq!((both.images_removed, both.blocks_removed), (1, 2));
    assert_eq!(
        markdown(&fx, doc.id),
        "# Harbour\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n"
    );
}

/// The document of [`harbour`] with `second` as its second block.
fn markdown_of(second: &str) -> String {
    format!("# Harbour\n\n{second}\n\nOne.\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n")
}

#[test]
fn everything_else_an_agent_does_keeps_the_image() {
    let fx = fixture();
    let (doc, id) = harbour(&fx);
    let block = fx.document(doc.id).blocks()[1].clone();
    let still = |fx: &Fixture, id: DocumentId| fx.document(id).blocks()[1] == block;

    // Text at the end, and at a heading.
    insert(&fx, doc.id, "Six.");
    assert!(still(&fx, doc.id));
    // Comments, and their deletion.
    fx.clock.advance_ms(1000);
    let thread = fx
        .library
        .add_comment(&AddCommentRequest {
            document: doc.id.into(),
            quote: "One.".into(),
            body: "The first.".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .thread
        .id;
    assert!(still(&fx, doc.id));
    fx.clock.advance_ms(1000);
    fx.library
        .delete_comment(&writer_agent::DeleteCommentRequest {
            document: doc.id.into(),
            comment: thread,
            reply: None,
            write: claude(),
        })
        .unwrap();
    assert!(still(&fx, doc.id));
    // A comment has no place on an image, and says so.
    let on_the_image = fx.library.add_comment(&AddCommentRequest {
        document: doc.id.into(),
        quote: line(DUSK, &id),
        body: "Nice.".into(),
        write: claude(),
        ..Default::default()
    });
    assert!(
        matches!(&on_the_image, Err(Error::Invalid { .. })),
        "{on_the_image:?}"
    );

    // A copy has it.
    fx.clock.advance_ms(1000);
    let copy = fx
        .library
        .duplicate_document(&DuplicateDocumentRequest {
            document: doc.id.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .document;
    let (_, image, alignment) = fx.images(copy.id).remove(0);
    assert_as_it_was(&image, alignment, &id);
    // The trash and the way back.
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: doc.id.into(),
            write: claude(),
        })
        .unwrap();
    assert!(
        fx.store()
            .referenced_images()
            .unwrap()
            .contains(&stored(&id))
    );
    fx.library
        .restore_document(&RestoreDocumentRequest {
            document: doc.id.into(),
            write: claude(),
        })
        .unwrap();
    assert!(still(&fx, doc.id));

    // A rewrite from the plain text names no image: it is taken out, the result counts
    // it and says where it still is.
    let text = fx.read(doc.id, TextFormat::PlainText);
    assert!(!text.contains("image:"), "{text}");
    let tried = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: doc.id.into(),
            markdown: text.clone(),
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tried.images_removed, 1, "the dry run says it first");
    let rewritten = replace(&fx, doc.id, &text);
    assert_eq!((rewritten.images_added, rewritten.images_removed), (0, 1));
    assert_eq!(
        rewritten.warnings.last().map(String::as_str),
        Some("1 image was removed; it is in the version kept before this change")
    );
    assert_eq!(rewritten.warnings, tried.warnings);
    assert!(fx.images(doc.id).is_empty());
    // The version before the change names it, and bringing that back brings it back.
    let version = rewritten.snapshot.expect("a version before the rewrite");
    let versions = fx
        .library
        .versions(&ListVersionsRequest {
            document: doc.id.into(),
        })
        .unwrap()
        .versions;
    assert_eq!(versions[0].id, version.id);
    assert!(
        fx.store()
            .snapshot(version.id)
            .unwrap()
            .content
            .contains(&format!("\"image\":\"{id}\""))
    );
    fx.clock.advance_ms(1000);
    fx.library
        .restore_version(&RestoreVersionRequest {
            version: version.id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(still(&fx, doc.id));

    // Several at once: the sentence counts them.
    let file = fx.dir.path().join("harbour.png");
    for _ in 0..2 {
        fx.clock.advance_ms(1000);
        fx.library
            .add_image(&AddImageRequest {
                document: doc.id.into(),
                path: path_of(&file),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
    }
    let all = replace(&fx, doc.id, "# Harbour\n\nNothing else.\n");
    assert_eq!(all.images_removed, 3);
    assert_eq!(
        all.warnings,
        ["3 images were removed; they are in the version kept before this change"]
    );
    // The bytes stay where they are whatever the documents say: nothing an agent does
    // deletes a picture.
    assert_eq!(fx.store().image_count().unwrap(), 1);
}

// ----- bringing files in ----------------------------------------------------------------------

#[test]
fn a_file_named_in_markdown_becomes_an_image_and_what_cannot_stays_a_link() {
    let fx = fixture();
    let red = fx.picture("pictures/red square.png", (60, 60), [220, 30, 30]);
    let green = fx.picture("green.png", (120, 40), [30, 160, 60]);
    let blue = fx.picture("blue.png", (40, 120), [30, 60, 200]);
    std::fs::write(fx.dir.path().join("notes.pdf"), b"%PDF-1.7\n").unwrap();
    let (red_id, green_id, blue_id) = (id_of(&red), id_of(&green), id_of(&blue));
    let text = format!(
        "# Colours\n\n\
         ![Red, from a folder beside the text](<pictures/red square.png>)\n\n\
         ![Green, by its whole path]({green})\n\n\
         ![Blue, as an address of a file](file://{blue})\n\n\
         ![Red once more](pictures/red%20square.png)\n\n\
         ![Not there](pictures/missing.png)\n\n\
         ![No picture](notes.pdf)\n\n\
         ![Far away](https://example.com/far.png)\n",
        green = green.display(),
        blue = blue.display(),
    );
    // (A library that is there: what a dry run leaves alone can be looked at.)
    fx.create(None, "# Seed\n\nText.\n");
    let before = fx.dump();
    let create = |write: WriteOptions| {
        fx.library
            .create_document(&CreateDocumentRequest {
                markdown: text.clone(),
                base_dir: Some(fx.dir.path().to_path_buf()),
                write,
                ..Default::default()
            })
            .unwrap()
    };
    // Tried first: the same warnings, and nothing stored.
    let tried = create(dry());
    assert_eq!(fx.dump(), before);
    assert!(!fx.images_path().exists());
    fx.clock.advance_ms(1000);
    let created = create(claude());
    assert_eq!(created.warnings, tried.warnings);
    assert_eq!(
        created.warnings,
        [
            "an image was kept as a link to it (it is at an address on the network, or inside a \
             heading, a quote or a table): line 15"
                .to_string(),
            "\u{201c}pictures/missing.png\u{201d} was not found. It was kept as a link.".to_string(),
            "\u{201c}notes.pdf\u{201d} is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP \
             or BMP. It was kept as a link."
                .to_string(),
        ]
    );
    let doc = created.document.id;
    assert_eq!(
        markdown(&fx, doc),
        format!(
            "# Colours\n\n\
             {}\n\n{}\n\n{}\n\n{}\n\n\
             [Not there](pictures/missing.png)\n\n\
             [No picture](notes.pdf)\n\n\
             [Far away](https://example.com/far.png)\n",
            line("Red, from a folder beside the text", &red_id),
            line("Green, by its whole path", &green_id),
            line("Blue, as an address of a file", &blue_id),
            line("Red once more", &red_id),
        )
    );
    // Three files, stored once each, as they were.
    let store = fx.store();
    assert_eq!(store.image_count().unwrap(), 3);
    assert_eq!(
        store.image_bytes(&stored(&red_id)).unwrap(),
        Some(std::fs::read(&red).unwrap())
    );
    // An image that Markdown brings in stands at the left, at its own size on the page.
    let page = fx.document(doc).settings().page;
    for (_, image, alignment) in fx.images(doc) {
        assert_eq!(alignment, Alignment::Left);
        assert_eq!(image.width, image.original_width(&page));
        assert!(image.width < ImageWidth::from_percent(30), "{image:?}");
    }
    // The title of a document that begins with a heading is the heading; an image
    // gives none.
    assert_eq!(created.document.title, "Colours");

    // The other operations that take Markdown bring files the same way; the directory
    // can be the library's.
    let library = fx.library.clone().with_base_dir(fx.dir.path());
    let yellow = fx.picture("yellow.png", (30, 30), [230, 210, 20]);
    let yellow_line = line("Yellow", &id_of(&yellow));
    fx.clock.advance_ms(1000);
    let appended = library
        .insert(&InsertRequest {
            document: doc.into(),
            markdown: "![Yellow](yellow.png)".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((appended.images_added, appended.blocks_added), (1, 1));
    fx.clock.advance_ms(1000);
    let edited = library
        .edit(&EditRequest {
            document: doc.into(),
            old_text: "# Colours".into(),
            new_text: "# Colours\n\n![Yellow](yellow.png)\n".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(edited.images_added, 1, "{edited:?}");
    assert!(
        markdown(&fx, doc).starts_with(&format!("# Colours\n\n{yellow_line}\n\n")),
        "{}",
        markdown(&fx, doc)
    );
    fx.clock.advance_ms(1000);
    let section = library
        .replace_section(&ReplaceSectionRequest {
            document: doc.into(),
            heading: "Colours".into(),
            markdown: "Only this one:\n\n![Yellow](yellow.png)\n".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((section.images_added, section.images_removed), (0, 5));
    assert_eq!(
        markdown(&fx, doc),
        format!("# Colours\n\nOnly this one:\n\n{yellow_line}\n")
    );
    fx.clock.advance_ms(1000);
    let rewritten = library
        .replace_document(&ReplaceDocumentRequest {
            document: doc.into(),
            markdown: "# Colours\n\n![Yellow](yellow.png)\n\n![Green](green.png)\n".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!((rewritten.images_added, rewritten.images_removed), (1, 0));
    assert_eq!(fx.store().image_count().unwrap(), 4);
    // Taken literally, a new text is the characters it is, and no file is read for it.
    fx.clock.advance_ms(1000);
    let literal = library
        .edit(&EditRequest {
            document: doc.into(),
            old_text: "Colours".into(),
            new_text: "![Yellow](yellow.png)".into(),
            literal: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(literal.images_added, 0);
    assert!(
        fx.document(doc).blocks()[0]
            .text()
            .contains("![Yellow](yellow.png)")
    );

    // Without a directory to start from, a relative path is not followed, and the
    // warning says what to give instead.
    fx.clock.advance_ms(1000);
    let nowhere = fx
        .library
        .insert(&InsertRequest {
            document: doc.into(),
            markdown: "![Yellow](yellow.png)".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(nowhere.images_added, 0);
    assert_eq!(
        nowhere.warnings,
        [
            "\u{201c}yellow.png\u{201d} is a relative path, and nothing says what it is relative to: \
          give the absolute path of the image file. It was kept as a link."
        ]
    );
}

/// A request that is carried out by the app brings the bytes; the app opens no path.
#[test]
fn the_app_stores_what_a_request_brings_and_opens_no_path() {
    let fx = closed_fixture();
    let doc = fx.create(None, "# Notes\n\nText.\n");
    let png = writer_image::fixtures::png(48, 32, [90, 20, 140]);
    let id = writer_store::ImageId::of(&png).to_hex();
    // A file that is there, and that the request does not bring.
    let on_disk = fx.picture("on disk.png", (10, 10), [1, 2, 3]);
    let mut store = Store::open(fx.library.database_path()).unwrap();
    store.set_clock(fx.clock.clone());
    let request = InsertRequest {
        document: doc.id.into(),
        // The path of the first is one that leads nowhere on this Mac: what makes it an
        // image is what the request brings under that name.
        markdown: format!(
            "![Brought](/no/such/folder/brought.png)\n\n![Left behind](<{}>)\n",
            on_disk.display()
        ),
        images: vec![AttachedImage {
            source: "/no/such/folder/brought.png".into(),
            format: "png".into(),
            pixels: (48, 32),
            density: 144,
            data: png.clone(),
            problem: None,
        }],
        write: claude(),
        ..Default::default()
    };
    let executed = live::execute(
        &mut store,
        &Request::new(Operation::Insert(request.clone()), Duration::from_secs(1)),
    );
    let Answer::Done { result } = executed.answer else {
        panic!("{:?}", executed.answer);
    };
    let done: ContentChanged = serde_json::from_value(result).unwrap();
    assert_eq!((done.images_added, done.blocks_added), (1, 2));
    assert!(
        done.warnings.len() == 1
            && done.warnings[0]
                .ends_with("on disk.png\u{201d} was not found. It was kept as a link."),
        "a file the request did not bring is not read where the request is carried out: {:?}",
        done.warnings
    );
    assert_eq!(store.image_count().unwrap(), 1);
    assert_eq!(store.image_bytes(&stored(&id)).unwrap(), Some(png.clone()));
    drop(store);
    let (_, image, _) = fx.images(doc.id).remove(0);
    assert_eq!(image.id.to_string(), id);
    assert_eq!((image.pixels, image.density), ((48, 32), 144));

    // As a line of the channel the request has its images, as base64, and nothing of
    // where the files were.
    let mut with_dir = request.clone();
    with_dir.base_dir = Some(fx.dir.path().to_path_buf());
    let sent = serde_json::to_value(Request::new(
        Operation::Insert(with_dir),
        Duration::from_secs(1),
    ))
    .unwrap();
    assert_eq!(sent["version"], 2);
    assert_eq!(sent["op"], "insert");
    let brought = &sent["request"]["images"][0];
    assert_eq!(brought["source"], "/no/such/folder/brought.png");
    assert_eq!(brought["format"], "png");
    assert_eq!(brought["pixels"], serde_json::json!([48, 32]));
    let text = brought["data"].as_str().unwrap();
    assert!(text.starts_with("iVBORw0KGgo"), "a PNG in base64: {text}");
    assert!(sent["request"].get("base_dir").is_none(), "{sent}");
    let back: Request = serde_json::from_value(sent).unwrap();
    assert_eq!(back.operation, Operation::Insert(request));
    // A request without images is the line it was before images.
    let plain = serde_json::to_value(Operation::Insert(InsertRequest {
        document: doc.id.into(),
        markdown: "Text.".into(),
        ..Default::default()
    }))
    .unwrap();
    assert!(plain["request"].get("images").is_none(), "{plain}");

    // Bytes that are not what they are said to be are no image, wherever they come
    // from; a problem the sender found is told as it was found.
    let mut store = Store::open(fx.library.database_path()).unwrap();
    let mut ask = |images: Vec<AttachedImage>| -> Result<ContentChanged, String> {
        fx.clock.advance_ms(1000);
        let request = InsertRequest {
            document: doc.id.into(),
            markdown: "![Odd](/somewhere/odd.png)".into(),
            images,
            write: claude(),
            ..Default::default()
        };
        match live::execute(
            &mut store,
            &Request::new(Operation::Insert(request), Duration::from_secs(1)),
        )
        .answer
        {
            Answer::Done { result } => Ok(serde_json::from_value(result).unwrap()),
            Answer::Failed { error } => Err(error.message),
            other => panic!("{other:?}"),
        }
    };
    let odd = |format: &str, data: &[u8], pixels: (u32, u32)| AttachedImage {
        source: "/somewhere/odd.png".into(),
        format: format.into(),
        pixels,
        density: 72,
        data: data.to_vec(),
        problem: None,
    };
    for wrong in [
        odd("png", b"GIF89a and so on", (4, 4)),
        odd("jpeg", &png, (48, 32)),
        odd("tiff", &png, (48, 32)),
        odd("png", &png, (0, 32)),
        odd("png", b"", (4, 4)),
    ] {
        let done = ask(vec![wrong]).unwrap();
        assert_eq!(done.images_added, 0);
        assert_eq!(
            done.warnings,
            [
                "\u{201c}/somewhere/odd.png\u{201d} did not arrive as an image the library can hold. \
              It was kept as a link."
            ]
        );
    }
    let told = ask(vec![AttachedImage {
        source: "/somewhere/odd.png".into(),
        problem: Some("\u{201c}/somewhere/odd.png\u{201d} is not an image.".into()),
        ..AttachedImage::default()
    }])
    .unwrap();
    assert_eq!(
        told.warnings,
        ["\u{201c}/somewhere/odd.png\u{201d} is not an image. It was kept as a link."]
    );
    assert_eq!(fx.store().image_count().unwrap(), 1);

    // More than a request may bring is refused before anything is stored, where the
    // request is carried out as where it is made.
    let heavy = |megabytes: usize| AttachedImage {
        source: format!("/somewhere/{megabytes}.png"),
        format: "png".into(),
        pixels: (4, 4),
        density: 72,
        data: vec![0; megabytes * 1024 * 1024],
        problem: None,
    };
    assert_eq!(MAX_ATTACHED_BYTES, 32 * 1024 * 1024);
    let too_much = ask(vec![heavy(11), heavy(11), heavy(11)]).unwrap_err();
    assert_eq!(
        too_much,
        "the images of one request can be 32 MB together at most; add them in several requests"
    );
    let direct = fx.library.insert(&InsertRequest {
        document: doc.id.into(),
        markdown: "Text.".into(),
        images: vec![heavy(11), heavy(11), heavy(11)],
        write: claude(),
        ..Default::default()
    });
    drop(store);
    assert!(
        matches!(&direct, Err(Error::Invalid { message }) if *message == too_much),
        "{direct:?}"
    );
    // Reading the files of a text stops at the same weight.
    let file = fx.picture("one more.png", (10, 10), [9, 9, 9]);
    let reading = fx.library.insert(&InsertRequest {
        document: doc.id.into(),
        markdown: format!("![One more](<{}>)", file.display()),
        images: vec![heavy(16), heavy(16)],
        write: dry(),
        ..Default::default()
    });
    assert!(
        matches!(&reading, Err(Error::Invalid { message }) if *message == too_much),
        "{reading:?}"
    );
    assert_eq!(fx.store().image_count().unwrap(), 1);
}

/// A document can name a picture the library does not have (its `images.db` was lost, or
/// the document came from elsewhere). The image stays in the document for every reader
/// and through every change; only the picture is missing.
#[test]
fn an_image_whose_picture_is_missing_stays_in_the_document() {
    let fx = closed_fixture();
    let (doc, id) = harbour(&fx);
    let before = fx.document(doc.id);
    // The pictures are gone; the documents are what they were.
    for ending in ["", "-wal", "-shm"] {
        let mut file = fx.images_path().into_os_string();
        file.push(ending);
        std::fs::remove_file(file).ok();
    }
    assert_eq!(fx.store().image_count().unwrap(), 0);
    let read = fx
        .library
        .read(&ReadRequest {
            document: doc.id.into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(read.content, markdown_of(&line(DUSK, &id)));
    assert_eq!(read.images.len(), 1);
    let image = &read.images[0];
    assert!(image.missing && image.format.is_none(), "{image:?}");
    assert_eq!(
        (image.width_percent, image.align.as_str(), image.pixels),
        (37, "center", (800, 500))
    );
    // Written back, edited beside and named again, it is the image it was.
    assert!(!replace(&fx, doc.id, &read.content).written.changed);
    edit(&fx, doc.id, "One.", "One, changed.").unwrap();
    let moved = replace(
        &fx,
        doc.id,
        &format!(
            "# Harbour\n\nOne, changed.\n\n{}\n\nTwo.\n\nThree.\n\nFour.\n\nFive.\n",
            line(DUSK, &id)
        ),
    );
    assert_eq!((moved.images_added, moved.images_removed), (0, 0));
    assert_eq!(moved.warnings, [] as [&str; 0]);
    let (index, image, alignment) = fx.images(doc.id).remove(0);
    assert_eq!(index, 2);
    assert_as_it_was(&image, alignment, &id);
    assert_eq!(before.blocks()[1].as_image(), Some(&image));
    // An export says what the picture shows and writes no file for it.
    let exported = fx
        .library
        .export(&ExportRequest {
            document: Some(doc.id.into()),
            path: fx.dir.path().join("out/harbour.md"),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(exported.assets, [] as [std::path::PathBuf; 0]);
    assert!(
        std::fs::read_to_string(&exported.path)
            .unwrap()
            .contains(DUSK)
    );
    // Nothing of this made a file of pictures again.
    assert!(!fx.images_path().exists());
}

// ----- export -----------------------------------------------------------------------------------

#[test]
fn an_export_takes_the_pictures_with_it() {
    let fx = fixture();
    let (doc, id) = harbour(&fx);
    let bytes = std::fs::read(fx.dir.path().join("harbour.png")).unwrap();
    let out = fx.dir.path().join("out");
    let export = |name: &str, dry_run: bool| {
        fx.library
            .export(&ExportRequest {
                document: Some(doc.id.into()),
                path: out.join(name),
                overwrite: true,
                dry_run,
                ..Default::default()
            })
            .unwrap()
    };
    // Markdown: the picture as a file in a folder beside the text, and a link to it.
    let picture = out
        .join("Chapter one images")
        .join(format!("image-{}.png", &id[..8]));
    let tried = export("Chapter one.md", true);
    assert_eq!(tried.assets, std::slice::from_ref(&picture));
    assert!(!out.exists(), "a dry run wrote something");
    let markdown = export("Chapter one.md", false);
    assert_eq!(markdown.format, ExportFormat::Markdown);
    assert_eq!(markdown.assets, std::slice::from_ref(&picture));
    assert_eq!(std::fs::read(&picture).unwrap(), bytes);
    let written = std::fs::read_to_string(&markdown.path).unwrap();
    assert_eq!(markdown.bytes, written.len() as u64);
    assert!(!written.contains("image:"), "{written}");
    let link = written
        .lines()
        .find(|line| line.starts_with("!["))
        .expect("a line with the image");
    assert!(
        link.starts_with(&format!("![{DUSK}]("))
            && link.contains(&format!("image-{}.png", &id[..8])),
        "{link}"
    );
    // The link leads to the file: read from the folder of the text, the Markdown brings
    // the same image back.
    let back = fx
        .library
        .create_document(&CreateDocumentRequest {
            markdown: written.clone(),
            title: Some("Back".into()),
            base_dir: Some(out.clone()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(back.warnings, [] as [&str; 0]);
    assert_eq!(fx.images(back.document.id)[0].1.id.to_string(), id);
    // Once more over the same files: the text is replaced, the picture is what it was.
    let again = export("Chapter one.md", false);
    assert_eq!(again.assets, std::slice::from_ref(&picture));
    let names: Vec<String> = std::fs::read_dir(picture.parent().unwrap())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, [format!("image-{}.png", &id[..8])]);

    // A Word document holds its pictures.
    let docx = export("chapter.docx", false);
    assert_eq!(docx.assets, [] as [std::path::PathBuf; 0]);
    let package = std::fs::read(&docx.path).unwrap();
    let holds = |needle: &[u8]| package.windows(needle.len()).any(|window| window == needle);
    assert!(holds(b"word/media/image1.png"), "no picture in the package");
    assert!(package.len() > bytes.len());
    // Plain text has what the picture shows, and no file beside it.
    let text = export("chapter.txt", false);
    assert_eq!(text.assets, [] as [std::path::PathBuf; 0]);
    assert!(
        std::fs::read_to_string(&text.path).unwrap().contains(DUSK),
        "the description is in the text"
    );
    // A text without images has no folder.
    let plain = fx.create(None, "# Plain\n\nWords only.\n");
    let exported = fx
        .library
        .export(&ExportRequest {
            document: Some(plain.id.into()),
            path: out.join("plain.md"),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(exported.assets, [] as [std::path::PathBuf; 0]);
    assert!(!out.join("plain images").exists());

    // A folder as one file: the pictures of all its documents in one folder.
    let all = fx
        .library
        .export(&ExportRequest {
            folder: Some(Target::root()),
            path: out.join("library.md"),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        all.assets,
        [out.join("library images")
            .join(format!("image-{}.png", &id[..8]))],
        "one picture, used by two documents, is one file"
    );
}

/// A picture that cannot be read is not a missing one: with `images.db` damaged an export
/// fails and writes nothing, where one whose picture the library does not have is written
/// with the description in its place.
#[test]
fn an_export_whose_pictures_cannot_be_read_fails_and_writes_nothing() {
    let fx = closed_fixture();
    let (doc, _) = harbour(&fx);
    for ending in ["-wal", "-shm"] {
        let mut file = fx.images_path().into_os_string();
        file.push(ending);
        std::fs::remove_file(file).ok();
    }
    // The header of the file of pictures overwritten: SQLite reads no database there.
    let mut damaged = std::fs::read(fx.images_path()).unwrap();
    damaged[..16].copy_from_slice(b"not a database!!");
    std::fs::write(fx.images_path(), &damaged).unwrap();
    let out = fx.dir.path().join("out");
    for name in ["harbour.md", "harbour.docx"] {
        let error = fx
            .library
            .export(&ExportRequest {
                document: Some(doc.id.into()),
                path: out.join(name),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error.code(), "failed", "{error}");
        let message = error.to_string();
        assert!(
            message.starts_with("the pictures of this export could not be read")
                && message.contains("nothing was written"),
            "{message}"
        );
    }
    assert!(!out.exists(), "an export without its pictures was written");
}
