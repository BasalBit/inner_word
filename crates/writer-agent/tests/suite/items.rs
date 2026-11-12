//! Names, places and the trash, for documents and for folders.

use writer_agent::{
    CreateFolderRequest, DeleteDocumentRequest, DeleteFolderRequest, DuplicateDocumentRequest,
    EditRequest, EmptyTrashRequest, Error, ErrorKind, InfoRequest, Item, ItemKind,
    ListVersionsRequest, MoveDocumentRequest, MoveFolderRequest, Placement, RenameDocumentRequest,
    RenameFolderRequest, RestoreDocumentRequest, RestoreFolderRequest, Target,
    TrashDocumentRequest, TrashFolderRequest, TrashItem, TreeRequest,
};

use super::{Fixture, claude, fixture};

fn titles(fx: &Fixture, folder: &str) -> Vec<String> {
    fx.library
        .tree(&TreeRequest {
            folder: Some(Target::path(folder)),
            depth: Some(0),
            ..Default::default()
        })
        .unwrap()
        .documents
        .into_iter()
        .map(|document| document.title)
        .collect()
}

fn folders(fx: &Fixture, folder: &str) -> Vec<String> {
    fx.library
        .tree(&TreeRequest {
            folder: Some(Target::path(folder)),
            depth: Some(0),
            folders_only: true,
        })
        .unwrap()
        .folders
        .into_iter()
        .map(|folder| folder.folder.name)
        .collect()
}

#[test]
fn documents_are_renamed_moved_and_copied() {
    let fx = fixture();
    let one = fx.create(Some("Book"), "# One\n\nfirst");
    let two = fx.create(Some("Book"), "# Two\n\nsecond");
    let three = fx.create(Some("Book"), "# Three\n\nthird");
    fx.create(Some("Archive"), "# Old\n\nold");

    // Rename: the title is kept from then on; the same name again changes nothing.
    fx.clock.advance_ms(5000);
    let renamed = fx
        .library
        .rename_document(&RenameDocumentRequest {
            document: Target::path("Book/One"),
            title: "  Chapter   One ".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(renamed.written.changed);
    assert_eq!(renamed.before.as_ref().unwrap().path, "Book/One");
    assert_eq!(renamed.document.path, "Book/Chapter One");
    assert_eq!(renamed.document.id, one.id);
    assert!(renamed.document.updated_at > renamed.before.as_ref().unwrap().updated_at);
    let again = fx
        .library
        .rename_document(&RenameDocumentRequest {
            document: one.id.into(),
            title: "Chapter One".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(!again.written.changed);
    assert!(matches!(
        fx.library.rename_document(&RenameDocumentRequest {
            document: one.id.into(),
            title: " ".into(),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    // The text still says "One"; an edit does not bring the old title back.
    fx.library
        .edit(&EditRequest {
            document: one.id.into(),
            old_text: "first".into(),
            new_text: "the first".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        fx.store().document_meta(one.id).unwrap().title,
        "Chapter One"
    );
    assert!(fx.markdown(one.id).starts_with("# One\n"));

    // Move: to another folder (last there), to a place, within its folder.
    let moved = fx
        .library
        .move_document(&MoveDocumentRequest {
            document: two.id.into(),
            folder: Some(Target::path("Archive")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(moved.before.as_ref().unwrap().path, "Book/Two");
    assert_eq!(moved.document.path, "Archive/Two");
    assert_eq!(titles(&fx, "Archive"), ["Old", "Two"]);
    assert_eq!(titles(&fx, "Book"), ["Chapter One", "Three"]);
    assert_eq!(
        moved.document.updated_at,
        moved.before.as_ref().unwrap().updated_at,
        "moving is not editing"
    );
    let placed = fx
        .library
        .move_document(&MoveDocumentRequest {
            document: two.id.into(),
            folder: Some(Target::path("Book")),
            placement: Some(Placement::Before(Target::path("Three"))),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(placed.written.changed);
    assert_eq!(titles(&fx, "Book"), ["Chapter One", "Two", "Three"]);
    let reorder = |placement: Placement| {
        fx.library
            .move_document(&MoveDocumentRequest {
                document: three.id.into(),
                placement: Some(placement),
                write: claude(),
                ..Default::default()
            })
            .unwrap()
    };
    assert!(reorder(Placement::First).written.changed);
    assert_eq!(titles(&fx, "Book"), ["Three", "Chapter One", "Two"]);
    assert!(
        !reorder(Placement::First).written.changed,
        "it is there already"
    );
    reorder(Placement::After(one.id.into()));
    assert_eq!(titles(&fx, "Book"), ["Chapter One", "Three", "Two"]);
    reorder(Placement::Last);
    reorder(Placement::Index(1));
    assert_eq!(titles(&fx, "Book"), ["Chapter One", "Three", "Two"]);
    // To the top of the library, and into a folder that is made for it.
    let top = fx
        .library
        .move_document(&MoveDocumentRequest {
            document: three.id.into(),
            folder: Some(Target::root()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(top.document.path, "Three");
    assert_eq!(top.document.folder_id, None);
    let made = fx
        .library
        .move_document(&MoveDocumentRequest {
            document: three.id.into(),
            folder: Some(Target::path("Book/Part two/Drafts")),
            create_folders: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(made.document.path, "Book/Part two/Drafts/Three");
    assert_eq!(made.created_folders.len(), 2);
    // Mistakes.
    assert!(matches!(
        fx.library.move_document(&MoveDocumentRequest {
            document: three.id.into(),
            folder: Some(Target::path("Nowhere")),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::NotFound {
            kind: ItemKind::Folder,
            ..
        })
    ));
    assert!(matches!(
        fx.library.move_document(&MoveDocumentRequest {
            document: three.id.into(),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        fx.library.move_document(&MoveDocumentRequest {
            document: three.id.into(),
            folder: Some(Target::path("Archive")),
            placement: Some(Placement::After(one.id.into())),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));

    // Copy: beside the original, named as the library names a copy; or named and placed.
    let copy = fx
        .library
        .duplicate_document(&DuplicateDocumentRequest {
            document: one.id.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(copy.before.as_ref().unwrap().id, one.id);
    assert_ne!(copy.document.id, one.id);
    assert_eq!(copy.document.title, "Chapter One copy");
    assert_eq!(
        titles(&fx, "Book"),
        ["Chapter One", "Chapter One copy", "Two"]
    );
    assert_eq!(fx.markdown(copy.document.id), fx.markdown(one.id));
    assert_eq!(
        copy.document.words,
        fx.store().document_meta(one.id).unwrap().word_count
    );
    let versions = fx
        .library
        .versions(&ListVersionsRequest {
            document: copy.document.id.into(),
        })
        .unwrap();
    assert!(versions.versions.is_empty(), "versions are not copied");
    let named = fx
        .library
        .duplicate_document(&DuplicateDocumentRequest {
            document: one.id.into(),
            title: Some("Chapter One, second draft".into()),
            folder: Some(Target::path("Archive")),
            write: claude(),
        })
        .unwrap();
    assert_eq!(named.document.path, "Archive/Chapter One, second draft");
    assert!(
        fx.library
            .info(&InfoRequest {
                document: named.document.id.into()
            })
            .unwrap()
            .custom_title
    );
    // The copy of a renamed document keeps its name, as the app keeps it. The copy of a
    // document whose title follows its text has the library's "copy" name until its
    // first change, and follows its text from then on, as in the app.
    assert!(
        fx.library
            .info(&InfoRequest {
                document: copy.document.id.into()
            })
            .unwrap()
            .custom_title
    );
    let plain = fx
        .library
        .duplicate_document(&DuplicateDocumentRequest {
            document: Target::path("Book/Two"),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(plain.document.title, "Two copy");
    assert!(
        !fx.library
            .info(&InfoRequest {
                document: plain.document.id.into()
            })
            .unwrap()
            .custom_title
    );
    fx.library
        .edit(&EditRequest {
            document: plain.document.id.into(),
            old_text: "second".into(),
            new_text: "another second".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        fx.store().document_meta(plain.document.id).unwrap().title,
        "Two"
    );
}

#[test]
fn documents_go_to_the_trash_and_come_back() {
    let fx = fixture();
    let one = fx.create(Some("Book"), "# One\n\nfirst");
    let two = fx.create(Some("Book"), "# Two\n\nsecond");
    let three = fx.create(Some("Book"), "# Three\n\nthird");
    fx.library
        .edit(&EditRequest {
            document: two.id.into(),
            old_text: "second".into(),
            new_text: "2nd".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();

    fx.clock.advance_ms(1000);
    let trashed = fx
        .library
        .trash_document(&TrashDocumentRequest {
            document: Target::path("Book/Two"),
            write: claude(),
        })
        .unwrap();
    assert!(trashed.written.changed);
    assert!(trashed.document.trashed_at.is_some());
    assert_eq!(trashed.before.unwrap().trashed_at, None);
    assert_eq!(titles(&fx, "Book"), ["One", "Three"]);
    // In the trash it is found by id, not by path; it cannot be edited or trashed again.
    assert!(matches!(
        fx.library.resolve_document(&Target::path("Book/Two")),
        Err(Error::NotFound { .. })
    ));
    let in_trash = fx.library.edit(&EditRequest {
        document: two.id.into(),
        old_text: "2nd".into(),
        new_text: "x".into(),
        write: claude(),
        ..Default::default()
    });
    match in_trash {
        Err(error @ Error::InTrash { .. }) => {
            assert_eq!(error.kind(), ErrorKind::WrongState);
            assert!(error.to_string().contains("restore it first"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        fx.library.trash_document(&TrashDocumentRequest {
            document: two.id.into(),
            write: claude(),
        }),
        Err(Error::InTrash { .. })
    ));
    // It can still be read.
    assert!(fx.markdown(two.id).contains("2nd"));

    let list = fx.library.trash().unwrap();
    assert_eq!(list.entries.len(), 1);
    match &list.entries[0] {
        TrashItem::Document { document } => {
            assert_eq!(document.id, two.id);
            assert_eq!(document.path, "Book/Two", "where it was");
        }
        other => panic!("{other:?}"),
    }
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(json["entries"][0]["kind"], "document");
    assert_eq!(json["entries"][0]["document"]["title"], "Two");
    assert!(matches!(
        fx.library.resolve(&Target::path("Two"), true).unwrap(),
        Item::Document { document } if document.id == two.id
    ));

    // Back, by name, to where it was; its versions came along.
    let restored = fx
        .library
        .restore_document(&RestoreDocumentRequest {
            document: Target::path("Two"),
            write: claude(),
        })
        .unwrap();
    assert_eq!(restored.document.trashed_at, None);
    assert_eq!(restored.document.path, "Book/Two");
    assert_eq!(titles(&fx, "Book"), ["One", "Two", "Three"]);
    assert_eq!(
        fx.library
            .versions(&ListVersionsRequest {
                document: two.id.into()
            })
            .unwrap()
            .versions
            .len(),
        1
    );
    assert!(fx.library.trash().unwrap().entries.is_empty());
    let not_there = fx.library.restore_document(&RestoreDocumentRequest {
        document: two.id.into(),
        write: claude(),
    });
    match not_there {
        Err(error @ Error::NotInTrash { .. }) => {
            assert_eq!(error.kind(), ErrorKind::WrongState);
            assert!(error.to_string().contains("is not in the trash"));
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        fx.library.restore_document(&RestoreDocumentRequest {
            document: Target::path("Nothing"),
            write: claude(),
        }),
        Err(Error::NotFound { .. })
    ));

    // For good: only with the explicit field; from the trash, or straight away.
    let trash = |id| {
        fx.library
            .trash_document(&TrashDocumentRequest {
                document: Target::DocumentId(id),
                write: claude(),
            })
            .unwrap()
    };
    trash(two.id.0);
    let refuse = fx.library.delete_document(&DeleteDocumentRequest {
        document: Target::path("Two"),
        permanently: false,
        write: claude(),
    });
    assert!(matches!(refuse, Err(Error::ConfirmationRequired { .. })));
    assert_eq!(fx.library.trash().unwrap().entries.len(), 1);
    let deleted = fx
        .library
        .delete_document(&DeleteDocumentRequest {
            document: Target::path("Two"),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert!(deleted.written.changed);
    assert_eq!(deleted.document.id, two.id);
    assert_eq!(deleted.before, None);
    assert!(fx.library.trash().unwrap().entries.is_empty());
    assert!(matches!(
        fx.library.info(&InfoRequest {
            document: two.id.into()
        }),
        Err(Error::NotFound { .. })
    ));
    // A live document, with its chosen-title flag.
    fx.library
        .rename_document(&RenameDocumentRequest {
            document: three.id.into(),
            title: "Third".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let key = format!("document.custom_title.{}", three.id.0);
    assert!(fx.store().setting::<i64>(&key).unwrap().is_some());
    fx.library
        .delete_document(&DeleteDocumentRequest {
            document: three.id.into(),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        fx.store().setting::<i64>(&key).unwrap(),
        None,
        "the flag went with it"
    );
    assert_eq!(titles(&fx, "Book"), ["One"]);
    let status = fx.library.status().unwrap();
    assert_eq!((status.documents, status.trashed), (1, 0));
    // A document that gets the id of the deleted one is not taken for it.
    fx.create(Some("Book"), "# Fresh\n\ntext");
    let reborn = fx.create(Some("Book"), "# Fresher\n\ntext");
    assert_eq!(reborn.id, three.id, "SQLite hands the id out again");
    assert!(
        !fx.library
            .info(&InfoRequest {
                document: reborn.id.into()
            })
            .unwrap()
            .custom_title
    );
    let _ = one;
}

#[test]
fn folders_are_created_renamed_and_moved() {
    let fx = fixture();
    let mkdir = |path: &str, parent: Option<Target>| {
        fx.library.create_folder(&CreateFolderRequest {
            path: path.into(),
            parent,
            write: claude(),
        })
    };
    let made = mkdir("The Salt Road/Part one/Drafts", None).unwrap();
    assert!(made.written.changed);
    assert_eq!(made.folder.path, "The Salt Road/Part one/Drafts");
    let created: Vec<&str> = made.created.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(created, ["The Salt Road", "Part one", "Drafts"]);
    assert_eq!(made.created[1].parent_id, Some(made.created[0].id));
    // Again: it is there, nothing is made, and that is not an error.
    let again = mkdir("the salt road/Part one/Drafts/", None).unwrap();
    assert!(!again.written.changed && again.created.is_empty());
    assert_eq!(again.folder.id, made.folder.id);
    // Below a folder; only what is missing is made.
    let part_two = mkdir("Part two/Notes", Some(Target::path("The Salt Road"))).unwrap();
    assert_eq!(part_two.folder.path, "The Salt Road/Part two/Notes");
    assert_eq!(part_two.created.len(), 2);
    assert_eq!(folders(&fx, "The Salt Road"), ["Part one", "Part two"]);
    mkdir("Ünïcödé — 東京", None).unwrap();
    assert_eq!(
        fx.library
            .resolve_folder(&Target::path("ünïcödé — 東京"))
            .unwrap()
            .name,
        "Ünïcödé — 東京"
    );
    assert!(matches!(mkdir("  /  ", None), Err(Error::Invalid { .. })));
    assert!(matches!(
        mkdir("x", Some(Target::path("Nowhere"))),
        Err(Error::NotFound { .. })
    ));

    // Rename: everything inside follows.
    fx.create(Some("The Salt Road/Part one"), "# Chapter 1\n\ntext");
    let renamed = fx
        .library
        .rename_folder(&RenameFolderRequest {
            folder: Target::path("Part one"),
            name: "Part I".into(),
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        renamed.before.as_ref().unwrap().path,
        "The Salt Road/Part one"
    );
    assert_eq!(renamed.folder.path, "The Salt Road/Part I");
    assert_eq!(
        fx.library
            .resolve_document(&Target::path("Chapter 1"))
            .unwrap()
            .path,
        "The Salt Road/Part I/Chapter 1"
    );
    assert!(
        !fx.library
            .rename_folder(&RenameFolderRequest {
                folder: Target::path("Part I"),
                name: "Part I".into(),
                write: claude(),
            })
            .unwrap()
            .written
            .changed
    );
    assert!(matches!(
        fx.library.rename_folder(&RenameFolderRequest {
            folder: Target::path("Part I"),
            name: "".into(),
            write: claude(),
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        fx.library.rename_folder(&RenameFolderRequest {
            folder: Target::root(),
            name: "x".into(),
            write: claude(),
        }),
        Err(Error::Invalid { .. })
    ));

    // Move: into another folder with everything in it, to the top, to a place.
    let moved = fx
        .library
        .move_folder(&MoveFolderRequest {
            folder: Target::path("Part I"),
            parent: Some(Target::path("Part two")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(moved.folder.path, "The Salt Road/Part two/Part I");
    assert_eq!((moved.folders, moved.documents), (1, 1));
    assert_eq!(
        fx.library
            .resolve_document(&Target::path("Chapter 1"))
            .unwrap()
            .path,
        "The Salt Road/Part two/Part I/Chapter 1"
    );
    assert_eq!(folders(&fx, "Part two"), ["Notes", "Part I"]);
    let first = fx
        .library
        .move_folder(&MoveFolderRequest {
            folder: Target::path("Part I"),
            placement: Some(Placement::First),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(first.written.changed);
    assert_eq!(folders(&fx, "Part two"), ["Part I", "Notes"]);
    let top = fx
        .library
        .move_folder(&MoveFolderRequest {
            folder: Target::path("Part I"),
            parent: Some(Target::root()),
            placement: Some(Placement::Before(Target::path("/The Salt Road"))),
            write: claude(),
        })
        .unwrap();
    assert_eq!(top.folder.path, "Part I");
    assert_eq!(top.folder.parent_id, None);
    assert_eq!(
        folders(&fx, "/"),
        ["Part I", "The Salt Road", "Ünïcödé — 東京"]
    );
    // Not into itself or into what is inside it.
    for parent in ["The Salt Road", "The Salt Road/Part two/Notes"] {
        let cycle = fx.library.move_folder(&MoveFolderRequest {
            folder: Target::path("/The Salt Road"),
            parent: Some(Target::path(parent)),
            write: claude(),
            ..Default::default()
        });
        match cycle {
            Err(error @ Error::Invalid { .. }) => {
                assert!(error.to_string().contains("into itself"), "{error}");
            }
            other => panic!("{parent}: {other:?}"),
        }
    }
    assert!(matches!(
        fx.library.move_folder(&MoveFolderRequest {
            folder: Target::path("Part I"),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
}

#[test]
fn folders_go_to_the_trash_with_everything_in_them() {
    let fx = fixture();
    fx.create(Some("Book/Part one"), "# Chapter 1\n\nfirst");
    fx.create(Some("Book/Part one"), "# Chapter 2\n\nsecond");
    let outline = fx.create(Some("Book"), "# Outline\n\nplan");
    fx.create(Some("Other"), "# Kept\n\nkept");
    let loose = fx.create(None, "# Loose\n\nloose");

    let trashed = fx
        .library
        .trash_folder(&TrashFolderRequest {
            folder: Target::path("Book"),
            write: claude(),
        })
        .unwrap();
    assert_eq!((trashed.folders, trashed.documents), (1, 3));
    assert!(trashed.folder.trashed_at.is_some());
    assert_eq!(folders(&fx, "/"), ["Other"]);
    let status = fx.library.status().unwrap();
    assert_eq!(
        (status.documents, status.folders, status.trashed),
        (2, 1, 1)
    );
    let list = fx.library.trash().unwrap();
    match &list.entries[0] {
        TrashItem::Folder {
            folder,
            folders,
            documents,
        } => {
            assert_eq!(folder.name, "Book");
            assert_eq!((*folders, *documents), (1, 3));
        }
        other => panic!("{other:?}"),
    }
    // What went with the folder comes back with the folder, not alone.
    let alone = fx.library.restore_document(&RestoreDocumentRequest {
        document: outline.id.into(),
        write: claude(),
    });
    match alone {
        Err(error @ Error::NotInTrash { .. }) => {
            let message = error.to_string();
            assert!(
                message.contains("together with the folder \u{201c}Book\u{201d}"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        fx.library.delete_document(&DeleteDocumentRequest {
            document: outline.id.into(),
            permanently: true,
            write: claude(),
        }),
        Err(Error::NotInTrash { .. })
    ));
    // A document trashed on its own is its own entry, the newest first.
    fx.clock.advance_ms(1000);
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: loose.id.into(),
            write: claude(),
        })
        .unwrap();
    let list = fx.library.trash().unwrap();
    assert_eq!(list.entries.len(), 2);
    assert!(
        matches!(&list.entries[0], TrashItem::Document { document } if document.id == loose.id)
    );

    let restored = fx
        .library
        .restore_folder(&RestoreFolderRequest {
            folder: Target::path("Book"),
            write: claude(),
        })
        .unwrap();
    assert_eq!(restored.folder.trashed_at, None);
    assert_eq!((restored.folders, restored.documents), (1, 3));
    assert_eq!(folders(&fx, "/"), ["Book", "Other"]);
    assert_eq!(
        fx.library
            .resolve_document(&Target::path("Part one/Chapter 2"))
            .unwrap()
            .path,
        "Book/Part one/Chapter 2"
    );
    assert!(matches!(
        fx.library.restore_folder(&RestoreFolderRequest {
            folder: Target::path("Book"),
            write: claude(),
        }),
        Err(Error::NotFound { .. })
    ));

    // For good: with the explicit field only; a live folder goes in one step.
    let refused = fx.library.delete_folder(&DeleteFolderRequest {
        folder: Target::path("Book"),
        permanently: false,
        write: claude(),
    });
    match refused {
        Err(error @ Error::ConfirmationRequired { .. }) => {
            assert!(
                error.to_string().contains("with 3 documents in it"),
                "{error}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.library.status().unwrap().documents, 4);
    let deleted = fx
        .library
        .delete_folder(&DeleteFolderRequest {
            folder: Target::path("Book"),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert_eq!((deleted.folders, deleted.documents), (1, 3));
    assert!(matches!(
        fx.library.info(&InfoRequest {
            document: outline.id.into()
        }),
        Err(Error::NotFound { .. })
    ));

    // Emptying the trash.
    fx.library
        .trash_folder(&TrashFolderRequest {
            folder: Target::path("Other"),
            write: claude(),
        })
        .unwrap();
    assert!(matches!(
        fx.library.empty_trash(&EmptyTrashRequest {
            permanently: false,
            write: claude(),
        }),
        Err(Error::ConfirmationRequired { .. })
    ));
    assert_eq!(fx.library.trash().unwrap().entries.len(), 2);
    let emptied = fx
        .library
        .empty_trash(&EmptyTrashRequest {
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert_eq!((emptied.folders, emptied.documents), (1, 2));
    assert!(emptied.written.changed);
    assert!(fx.library.trash().unwrap().entries.is_empty());
    let status = fx.library.status().unwrap();
    assert_eq!(
        (status.documents, status.folders, status.trashed),
        (0, 0, 0)
    );
    let nothing = fx
        .library
        .empty_trash(&EmptyTrashRequest {
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert!(!nothing.written.changed);
}
