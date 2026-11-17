//! History and export.

use innerword_agent::{
    EditRequest, Error, ErrorKind, ExportFormat, ExportRequest, ItemKind, ListVersionsRequest,
    ReadVersionRequest, RestoreVersionRequest, Target, TextFormat, TrashDocumentRequest,
};

use super::{claude, fixture};

#[test]
fn the_versions_of_a_document_are_listed_read_and_restored() {
    let fx = fixture();
    let doc = fx.create(Some("Book"), "# Draft\n\nVersion one.\n");
    let other = fx.create(Some("Book"), "# Other\n\nAnother text.\n");
    let edit = |old: &str, new: &str| {
        fx.clock.advance_ms(60_000);
        fx.library
            .edit(&EditRequest {
                document: doc.id.into(),
                old_text: old.into(),
                new_text: new.into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap()
    };
    let first = edit("one", "two").snapshot.unwrap();
    let second = edit("two", "three").snapshot.unwrap();
    // A version the app took as the writer worked has no label.
    let mut store = innerword_store::Store::open(fx.library.database_path()).unwrap();
    store.set_clock(fx.clock.clone());
    fx.clock.advance_ms(60_000);
    let by_app = store.create_snapshot(doc.id).unwrap();
    store.close().unwrap();

    let list = fx
        .library
        .versions(&ListVersionsRequest {
            document: Target::path("Book/Draft"),
        })
        .unwrap();
    assert_eq!(list.document.id, doc.id);
    let ids: Vec<i64> = list.versions.iter().map(|v| v.id.0).collect();
    assert_eq!(ids, [by_app.id.0, second.id.0, first.id.0], "newest first");
    assert_eq!(list.versions[0].label, None);
    assert_eq!(
        list.versions[1].label.as_deref(),
        Some("Before an edit by Claude")
    );
    assert_eq!(list.versions[2], first);
    assert!(list.versions[1].created_at > list.versions[2].created_at);
    assert_eq!(list.versions[2].title, "Draft");
    assert_eq!(
        (list.versions[2].words, list.versions[2].characters),
        (3, 17)
    );
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(json["versions"][1]["label"], "Before an edit by Claude");
    assert!(json["versions"][0]["label"].is_null());

    let read = |version: i64, format: TextFormat| {
        fx.library
            .read_version(&ReadVersionRequest { version, format })
    };
    let was = read(first.id.0, TextFormat::Markdown).unwrap();
    assert_eq!(was.content, "# Draft\n\nVersion one.\n");
    assert_eq!(was.version, first);
    assert_eq!(was.document.id, doc.id);
    assert_eq!(
        read(second.id.0, TextFormat::PlainText)
            .unwrap()
            .content
            .trim_end(),
        "Draft\n\nVersion two."
    );
    assert!(
        read(first.id.0, TextFormat::Json)
            .unwrap()
            .content
            .starts_with('{')
    );
    match read(999, TextFormat::Markdown) {
        Err(
            error @ Error::NotFound {
                kind: ItemKind::Version,
                ..
            },
        ) => {
            assert_eq!(error.kind(), ErrorKind::NotFound);
        }
        other => panic!("{other:?}"),
    }

    // Restore: the text as it is now is kept first.
    fx.clock.advance_ms(60_000);
    let restored = fx
        .library
        .restore_version(&RestoreVersionRequest {
            version: first.id.0,
            document: Some(Target::path("Draft")),
            write: claude(),
        })
        .unwrap();
    assert!(restored.written.changed);
    assert_eq!(restored.restored, first);
    assert_eq!(restored.document.id, doc.id);
    assert_eq!((restored.words_before, restored.words_after), (3, 3));
    assert_eq!(fx.markdown(doc.id), "# Draft\n\nVersion one.\n");
    // The newest version already held the text that was replaced (the app's snapshot),
    // so no second copy of it was made.
    assert_eq!(restored.snapshot.as_ref().unwrap().id, by_app.id);
    let count = |id| {
        fx.library
            .versions(&ListVersionsRequest {
                document: Target::DocumentId(id),
            })
            .unwrap()
            .versions
            .len()
    };
    assert_eq!(count(doc.id.0), 3);
    // Restoring what the document already is changes nothing.
    let same = fx
        .library
        .restore_version(&RestoreVersionRequest {
            version: first.id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(!same.written.changed && same.snapshot.is_none());
    // A version of another document than the one named is refused.
    let wrong = fx.library.restore_version(&RestoreVersionRequest {
        version: second.id.0,
        document: Some(other.id.into()),
        write: claude(),
    });
    match wrong {
        Err(error @ Error::Invalid { .. }) => {
            assert!(
                error.to_string().contains("nothing was restored"),
                "{error}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.markdown(doc.id), "# Draft\n\nVersion one.\n");
    assert_eq!(count(other.id.0), 0);
    // The versions of a trashed document are still listed; restoring needs it back.
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: doc.id.into(),
            write: claude(),
        })
        .unwrap();
    assert_eq!(count(doc.id.0), 3);
    assert!(matches!(
        fx.library.restore_version(&RestoreVersionRequest {
            version: second.id.0,
            write: claude(),
            ..Default::default()
        }),
        Err(Error::InTrash { .. })
    ));
}

#[test]
fn documents_and_folders_are_exported_to_files() {
    let fx = fixture();
    let one = fx.create(
        Some("Book/Part one"),
        "# Chapter 1\n\nThe *first* chapter.\n",
    );
    fx.create(
        Some("Book/Part one"),
        "# Chapter 2\n\nThe second chapter.\n",
    );
    fx.create(Some("Book"), "# Outline\n\nThe plan.\n");
    fx.create(None, "A loose note without a heading.\n");
    let out = fx.dir.path().join("out");
    let export = |request: ExportRequest| fx.library.export(&request);

    // A document, in the format the path names.
    let markdown = export(ExportRequest {
        document: Some(Target::path("Chapter 1")),
        path: out.join("deep/dir/chapter.md"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(markdown.format, ExportFormat::Markdown);
    assert_eq!(markdown.title, "Chapter 1");
    assert_eq!(markdown.documents.len(), 1);
    assert_eq!(markdown.documents[0].id, one.id);
    assert!(markdown.path.is_absolute());
    let written = std::fs::read_to_string(&markdown.path).unwrap();
    assert_eq!(written, "# Chapter 1\n\nThe *first* chapter.\n");
    assert_eq!(markdown.bytes, written.len() as u64);
    assert!(!markdown.dry_run);
    let text = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter.txt"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(text.format, ExportFormat::PlainText);
    let written = std::fs::read_to_string(&text.path).unwrap();
    assert!(
        written.contains("The first chapter.") && !written.contains('*'),
        "{written}"
    );
    let docx = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter.docx"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(docx.format, ExportFormat::Docx);
    let bytes = std::fs::read(&docx.path).unwrap();
    assert_eq!(&bytes[..4], b"PK\x03\x04");
    assert_eq!(docx.bytes, bytes.len() as u64);
    // The format can be named; a path that says nothing means Markdown.
    let named = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter.out"),
        format: Some(ExportFormat::PlainText),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(named.format, ExportFormat::PlainText);
    let bare = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(bare.format, ExportFormat::Markdown);

    // An existing file is not replaced unless asked.
    std::fs::write(out.join("chapter.md"), "mine").unwrap();
    let exists = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter.md"),
        ..Default::default()
    });
    match exists {
        Err(error @ Error::FileExists { .. }) => {
            assert_eq!(error.kind(), ErrorKind::FileExists);
            assert_eq!(error.exit_code(), 12);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        std::fs::read_to_string(out.join("chapter.md")).unwrap(),
        "mine"
    );
    export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("chapter.md"),
        overwrite: true,
        ..Default::default()
    })
    .unwrap();
    assert!(
        std::fs::read_to_string(out.join("chapter.md"))
            .unwrap()
            .starts_with("# Chapter 1")
    );
    // A dry run says what would be written.
    let tried = export(ExportRequest {
        document: Some(one.id.into()),
        path: out.join("not written.md"),
        dry_run: true,
        ..Default::default()
    })
    .unwrap();
    assert!(tried.dry_run && tried.bytes > 0);
    assert!(!out.join("not written.md").exists());
    // Nothing is left beside the files.
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|name| !name.contains("partial")),
        "{names:?}"
    );

    // A folder as one file: its own documents, then those of its folders.
    let book = export(ExportRequest {
        folder: Some(Target::path("Book")),
        path: out.join("book.md"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(book.title, "Book");
    let order: Vec<&str> = book.documents.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(order, ["Outline", "Chapter 1", "Chapter 2"]);
    let written = std::fs::read_to_string(&book.path).unwrap();
    let at = |needle: &str| {
        written
            .find(needle)
            .unwrap_or_else(|| panic!("{needle}: {written}"))
    };
    assert!(at("# Outline") < at("# Chapter 1") && at("# Chapter 1") < at("# Chapter 2"));
    let whole = export(ExportRequest {
        folder: Some(Target::root()),
        path: out.join("library.docx"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(whole.title, "Library");
    assert_eq!(whole.documents.len(), 4);
    assert_eq!(whole.documents[0].title, "A loose note without a heading");

    // Mistakes.
    assert!(matches!(
        export(ExportRequest {
            path: out.join("x.md"),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        export(ExportRequest {
            document: Some(one.id.into()),
            folder: Some(Target::path("Book")),
            path: out.join("x.md"),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        export(ExportRequest {
            document: Some(one.id.into()),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        export(ExportRequest {
            document: Some(one.id.into()),
            path: out.clone(),
            overwrite: true,
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        export(ExportRequest {
            document: Some(Target::path("Chapter 9")),
            path: out.join("x.md"),
            ..Default::default()
        }),
        Err(Error::NotFound { .. })
    ));
    fx.library
        .create_folder(&innerword_agent::CreateFolderRequest {
            path: "Empty".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(matches!(
        export(ExportRequest {
            folder: Some(Target::path("Empty")),
            path: out.join("empty.md"),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(!out.join("x.md").exists() && !out.join("empty.md").exists());
}
