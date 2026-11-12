//! The rules: the app-is-open rule, dry runs, the safety net, documents stored as the app
//! stores them, a book for timing.

use std::time::{Duration, Instant};

use writer_agent::{
    AddCommentRequest, CommentRequest, CreateDocumentRequest, CreateFolderRequest,
    DeleteCommentRequest, DeleteDocumentRequest, DeleteFolderRequest, DuplicateDocumentRequest,
    EditRequest, EmptyTrashRequest, Error, ErrorKind, InfoRequest, InsertPosition, InsertRequest,
    Library, ListVersionsRequest, MoveDocumentRequest, MoveFolderRequest, ReadRequest, Refusal,
    RenameDocumentRequest, RenameFolderRequest, ReplaceDocumentRequest, ReplaceSectionRequest,
    ReplyRequest, RestoreDocumentRequest, RestoreFolderRequest, RestoreVersionRequest,
    SearchRequest, Target, TextFormat, TrashDocumentRequest, TrashFolderRequest, TreeRequest,
    WriteOptions,
};
use writer_core::Document;
use writer_store::instance_lock::{self, HolderKind};

use super::live::age_the_lock;
use super::{Fixture, claude, closed_fixture, fixture, through_the_app};

/// The options of a dry run by "Claude".
pub fn dry() -> WriteOptions {
    WriteOptions {
        dry_run: true,
        author: Some("Claude".into()),
    }
}

/// One of every kind of write, against the documents `seed` makes, with the given options:
/// the name of the operation, and whether it changed the library.
pub fn every_write(
    fx: &Fixture,
    options: &WriteOptions,
) -> Vec<(&'static str, Result<bool, Error>)> {
    let library = &fx.library;
    let chapter = Target::path("Book/Chapter 1");
    let o = || options.clone();
    vec![
        (
            "create_document",
            library
                .create_document(&CreateDocumentRequest {
                    markdown: "# New\n\ntext".into(),
                    folder: Some(Target::path("Elsewhere/Deep")),
                    create_folders: true,
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "replace_document",
            library
                .replace_document(&ReplaceDocumentRequest {
                    document: chapter.clone(),
                    markdown: "# Chapter 1\n\nRewritten.".into(),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "edit",
            library
                .edit(&EditRequest {
                    document: chapter.clone(),
                    old_text: "at dawn".into(),
                    new_text: "at dusk".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "insert",
            library
                .insert(&InsertRequest {
                    document: chapter.clone(),
                    markdown: "Appended.".into(),
                    position: InsertPosition::End,
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "replace_section",
            library
                .replace_section(&ReplaceSectionRequest {
                    document: chapter.clone(),
                    heading: "Notes".into(),
                    markdown: "New notes.".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "rename_document",
            library
                .rename_document(&RenameDocumentRequest {
                    document: chapter.clone(),
                    title: "Renamed".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "move_document",
            library
                .move_document(&MoveDocumentRequest {
                    document: Target::path("Loose"),
                    folder: Some(Target::path("Book")),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "duplicate_document",
            library
                .duplicate_document(&DuplicateDocumentRequest {
                    document: Target::path("Loose"),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "trash_document",
            library
                .trash_document(&TrashDocumentRequest {
                    document: Target::path("Loose"),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "restore_document",
            library
                .restore_document(&RestoreDocumentRequest {
                    document: Target::path("Gone"),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "delete_document",
            library
                .delete_document(&DeleteDocumentRequest {
                    document: Target::path("Gone"),
                    permanently: true,
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "create_folder",
            library
                .create_folder(&CreateFolderRequest {
                    path: "Book/Drafts/Old".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "rename_folder",
            library
                .rename_folder(&RenameFolderRequest {
                    folder: Target::path("Book"),
                    name: "Novel".into(),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "move_folder",
            library
                .move_folder(&MoveFolderRequest {
                    folder: Target::path("Notes"),
                    parent: Some(Target::path("Book")),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "trash_folder",
            library
                .trash_folder(&TrashFolderRequest {
                    folder: Target::path("Notes"),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "restore_folder",
            library
                .restore_folder(&RestoreFolderRequest {
                    folder: Target::path("Attic"),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "delete_folder",
            library
                .delete_folder(&DeleteFolderRequest {
                    folder: Target::path("Notes"),
                    permanently: true,
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "empty_trash",
            library
                .empty_trash(&EmptyTrashRequest {
                    permanently: true,
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "add_comment",
            library
                .add_comment(&AddCommentRequest {
                    document: chapter.clone(),
                    quote: "Maren".into(),
                    body: "Which ship?".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "reply",
            library
                .reply(&ReplyRequest {
                    document: chapter.clone(),
                    comment: 1,
                    body: "The second one.".into(),
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "resolve_comment",
            library
                .resolve_comment(&CommentRequest {
                    document: chapter.clone(),
                    comment: 1,
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "reopen_comment",
            library
                .reopen_comment(&CommentRequest {
                    document: chapter.clone(),
                    comment: 1,
                    write: o(),
                })
                .map(|r| r.written.changed),
        ),
        (
            "delete_comment",
            library
                .delete_comment(&DeleteCommentRequest {
                    document: chapter.clone(),
                    comment: 1,
                    write: o(),
                    ..Default::default()
                })
                .map(|r| r.written.changed),
        ),
        (
            "restore_version",
            library
                .versions(&ListVersionsRequest {
                    document: chapter.clone(),
                })
                .and_then(|list| {
                    library.restore_version(&RestoreVersionRequest {
                        version: list.versions.last().expect("a version").id.0,
                        write: o(),
                        ..Default::default()
                    })
                })
                .map(|r| r.written.changed),
        ),
    ]
}

/// A library with something for every kind of write to work on.
fn seeded() -> Fixture {
    let fx = fixture();
    seed(&fx);
    fx
}

/// Puts into the library of `fx` something for every kind of write to work on.
pub fn seed(fx: &Fixture) {
    let chapter = fx.create(
        Some("Book"),
        "# Chapter 1\n\nThe *Maren* left at dawn.\n\n## Notes\n\nOld notes.\n",
    );
    fx.create(Some("Notes"), "# Ideas\n\nA lighthouse.\n");
    fx.create(None, "# Loose\n\nA loose page.\n");
    let gone = fx.create(None, "# Gone\n\nIn the trash.\n");
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: gone.id.into(),
            write: claude(),
        })
        .unwrap();
    // A comment, and a version to go back to.
    fx.library
        .add_comment(&AddCommentRequest {
            document: chapter.id.into(),
            quote: "dawn".into(),
            body: "Too early?".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    fx.library
        .edit(&EditRequest {
            document: chapter.id.into(),
            old_text: "Old notes".into(),
            new_text: "Older notes".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    // A folder in the trash, with a document in it.
    fx.create(Some("Attic"), "# Old box\n\nDust.\n");
    fx.library
        .trash_folder(&TrashFolderRequest {
            folder: Target::path("Attic"),
            write: claude(),
        })
        .unwrap();
}

/// An app from before 0.6.0 holds the lock and listens nowhere. For it the old rule
/// holds: reading works, and nothing is written underneath it. (What an app that takes
/// changes does is in `live.rs`.)
#[test]
fn while_an_app_that_takes_no_changes_is_open_reading_works_and_every_write_is_refused() {
    let fx = closed_fixture();
    seed(&fx);
    let before = fx.bytes();
    let tree_before = fx.library.tree(&TreeRequest::default()).unwrap();

    // The app: it holds the lock of its data directory for as long as it runs.
    let app = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    // It has been running for a while (one that has only just taken the lock is given a
    // moment to say where it listens).
    age_the_lock(&fx);
    assert!(fx.library.app_is_open());
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open);
    assert!(!status.app_takes_changes);
    assert_eq!(status.app_pid, Some(std::process::id()));
    assert_eq!(
        (status.documents, status.folders, status.trashed),
        (3, 2, 2)
    );

    // Everything that reads works.
    assert_eq!(
        fx.library.tree(&TreeRequest::default()).unwrap(),
        tree_before
    );
    let read = fx
        .library
        .read(&ReadRequest {
            document: Target::path("Book/Chapter 1"),
            ..Default::default()
        })
        .unwrap();
    assert!(read.content.contains("The *Maren* left at dawn."));
    let hits = fx
        .library
        .search(&SearchRequest {
            query: "lighthouse".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    fx.library
        .info(&InfoRequest {
            document: Target::path("Ideas"),
        })
        .unwrap();
    let out = fx.dir.path().join("out/chapter.md");
    fx.library
        .export(&writer_agent::ExportRequest {
            document: Some(Target::path("Chapter 1")),
            path: out.clone(),
            ..Default::default()
        })
        .unwrap();
    assert!(
        std::fs::read_to_string(&out)
            .unwrap()
            .starts_with("# Chapter 1")
    );

    // Everything that writes is refused, with the same error, and writes nothing.
    let results = every_write(&fx, &claude());
    assert_eq!(results.len(), writer_agent::live::Operation::CHANGES.len());
    assert_eq!(
        results.iter().map(|(name, _)| *name).collect::<Vec<_>>(),
        writer_agent::live::Operation::CHANGES,
        "one of every operation that writes, in the order of the table"
    );
    for (name, result) in results {
        match result {
            Err(
                error @ Error::AppIsOpen {
                    why: Refusal::OldVersion,
                    ..
                },
            ) => {
                assert_eq!(error.kind(), ErrorKind::AppIsOpen, "{name}");
                assert_eq!(error.exit_code(), 5, "{name}");
                let message = error.to_string();
                assert!(message.contains("nothing was changed"), "{name}: {message}");
                assert!(message.contains("quit The Writer"), "{name}: {message}");
                assert!(
                    message.contains(&format!("process {}", std::process::id())),
                    "{name}: {message}"
                );
            }
            other => panic!("{name} was not refused: {other:?}"),
        }
    }
    assert_eq!(fx.bytes(), before, "a refused write changed the library");
    assert_eq!(
        fx.library.tree(&TreeRequest::default()).unwrap(),
        tree_before
    );

    // A dry run only reads, so it works, and says that the real run would be refused.
    let tried = fx
        .library
        .edit(&EditRequest {
            document: Target::path("Chapter 1"),
            old_text: "at dawn".into(),
            new_text: "at dusk".into(),
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert!(tried.written.dry_run && tried.written.app_is_open && !tried.written.changed);
    assert!(!tried.written.app_takes_changes);
    assert_eq!(tried.replacements, 1);
    assert_eq!(fx.bytes(), before);

    // The app quits: the same writes go through.
    drop(app);
    assert!(!fx.library.app_is_open());
    assert!(!fx.library.status().unwrap().app_is_open);
    let edited = fx
        .library
        .edit(&EditRequest {
            document: Target::path("Chapter 1"),
            old_text: "at dawn".into(),
            new_text: "at dusk".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(edited.written.changed && !edited.written.app_is_open);
    // The lock was held for the write only.
    assert_eq!(instance_lock::holder(&fx.data_dir()).unwrap(), None);
    instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
}

#[test]
fn another_tool_that_is_writing_is_waited_for_and_then_reported() {
    let fx = closed_fixture();
    seed(&fx);
    let edit = |library: &Library, old: &str, new: &str| {
        library.edit(&EditRequest {
            document: Target::path("Chapter 1"),
            old_text: old.into(),
            new_text: new.into(),
            write: claude(),
            ..Default::default()
        })
    };
    // A tool that lets go after a moment is waited for.
    let tool = instance_lock::acquire_as(&fx.data_dir(), HolderKind::Tool)
        .unwrap()
        .lock()
        .unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        drop(tool);
    });
    let started = Instant::now();
    edit(&fx.library, "left", "sailed").unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
    release.join().unwrap();

    // One that does not is reported as busy, not as the app, and nothing is written.
    let tool = instance_lock::acquire_as(&fx.data_dir(), HolderKind::Tool)
        .unwrap()
        .lock()
        .unwrap();
    let impatient = fx.library.clone().with_patience(Duration::from_millis(100));
    let before = fx.bytes();
    match edit(&impatient, "sailed", "went") {
        Err(error @ Error::Busy { .. }) => {
            assert_eq!(error.kind(), ErrorKind::Busy);
            assert!(error.to_string().contains("try again"));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(fx.bytes(), before);
    drop(tool);
    edit(&impatient, "sailed", "went").unwrap();
    assert!(
        fx.markdown(
            fx.library
                .resolve_document(&Target::path("Chapter 1"))
                .unwrap()
                .id
        )
        .contains("went at dawn")
    );
}

#[test]
fn a_dry_run_reports_and_changes_nothing() {
    let fx = seeded();
    let before = fx.bytes();
    let tree = fx.library.tree(&TreeRequest::default()).unwrap();
    let versions = fx
        .library
        .versions(&ListVersionsRequest {
            document: Target::path("Chapter 1"),
        })
        .unwrap();
    for (name, result) in every_write(&fx, &dry()) {
        match result {
            Ok(changed) => assert!(!changed, "{name}: a dry run says it changed the library"),
            Err(error) => panic!("{name}: {error}"),
        }
        assert_eq!(
            fx.bytes(),
            before,
            "{name} changed the library in a dry run"
        );
    }
    assert_eq!(fx.library.tree(&TreeRequest::default()).unwrap(), tree);
    assert_eq!(
        fx.library
            .versions(&ListVersionsRequest {
                document: Target::path("Chapter 1"),
            })
            .unwrap(),
        versions
    );
    if !through_the_app() {
        assert!(!fx.dir.path().join("data/instance.lock").exists() || !fx.library.app_is_open());
    }

    // The report of a dry run is the report of the real run.
    let request = |write: WriteOptions| EditRequest {
        document: Target::path("Chapter 1"),
        old_text: "The *Maren* left".into(),
        new_text: "The **Maren** sailed away".into(),
        write,
        ..Default::default()
    };
    let tried = fx.library.edit(&request(dry())).unwrap();
    let done = fx.library.edit(&request(claude())).unwrap();
    assert!(tried.written.dry_run && !tried.written.changed && tried.snapshot.is_none());
    assert!(
        tried.written.would_change,
        "the dry run says that there is something to do"
    );
    assert!(!done.written.dry_run && done.written.changed && done.snapshot.is_some());
    assert!(done.written.would_change);
    // The same request once more has nothing to do, tried or done.
    let nothing = |write: WriteOptions| {
        fx.library
            .rename_folder(&RenameFolderRequest {
                folder: Target::path("Book"),
                name: "Book".into(),
                write,
            })
            .unwrap()
            .written
    };
    assert!(!nothing(dry()).would_change && !nothing(claude()).would_change);
    assert!(!nothing(claude()).changed);
    assert_eq!(tried.replacements, done.replacements);
    assert_eq!(tried.matched_as, done.matched_as);
    assert_eq!(
        (
            tried.blocks_added,
            tried.blocks_removed,
            tried.blocks_changed
        ),
        (done.blocks_added, done.blocks_removed, done.blocks_changed)
    );
    assert_eq!(tried.words_after, done.words_after);
    assert_eq!(tried.document.words, done.document.words);
    assert_eq!(tried.headings, done.headings);

    // In a library that does not exist yet, a dry run creates nothing either.
    let empty = closed_fixture();
    let created = empty
        .library
        .create_document(&CreateDocumentRequest {
            markdown: "# First\n\nwords here".into(),
            folder: Some(Target::path("A/B")),
            create_folders: true,
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(created.document.id.0, 0);
    assert_eq!(created.document.path, "A/B/First");
    assert_eq!(created.document.words, 3);
    assert_eq!(created.created_folders.len(), 2);
    assert_eq!(created.created_folders[1].path, "A/B");
    assert!(
        !empty.data_dir().exists(),
        "a dry run created the data directory"
    );
    assert!(matches!(
        empty.library.tree(&TreeRequest::default()),
        Err(Error::LibraryNotFound { .. })
    ));
    let status = empty.library.status().unwrap();
    assert!(!status.exists && !status.app_is_open);
    assert!(
        !empty.data_dir().exists(),
        "reading created the data directory"
    );
}

#[test]
fn a_version_is_kept_before_every_change_of_text_and_deleting_needs_a_yes() {
    let fx = fixture();
    let doc = fx.create(None, "# Plan\n\nFirst draft.\n");
    let versions = |fx: &Fixture| {
        fx.library
            .versions(&ListVersionsRequest {
                document: doc.id.into(),
            })
            .unwrap()
            .versions
    };
    assert!(versions(&fx).is_empty());

    fx.clock.advance_ms(60_000);
    let edited = fx
        .library
        .edit(&EditRequest {
            document: doc.id.into(),
            old_text: "First".into(),
            new_text: "Second".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let kept = edited.snapshot.expect("a version");
    assert_eq!(kept.label.as_deref(), Some("Before an edit by Claude"));
    assert_eq!(kept.document_id, doc.id);
    assert_eq!(kept.words, 3);
    let list = versions(&fx);
    assert_eq!(list, vec![kept.clone()]);
    // The version holds the text as it was.
    let was = fx
        .library
        .read_version(&writer_agent::ReadVersionRequest {
            version: kept.id.0,
            format: TextFormat::Markdown,
        })
        .unwrap();
    assert_eq!(was.content, "# Plan\n\nFirst draft.\n");
    assert_eq!(was.version, kept);

    // Each kind of change says what it was; without a name it was "an agent".
    fx.clock.advance_ms(60_000);
    let appended = fx
        .library
        .insert(&InsertRequest {
            document: doc.id.into(),
            markdown: "More.".into(),
            write: WriteOptions::default(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        appended.snapshot.unwrap().label.as_deref(),
        Some("Before an insertion by an agent")
    );
    let rewritten = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: doc.id.into(),
            markdown: "# Plan\n\nAll new.\n".into(),
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        rewritten.snapshot.unwrap().label.as_deref(),
        Some("Before a rewrite by Claude")
    );
    assert_eq!(versions(&fx).len(), 3);

    // A request that changes nothing keeps no version and says so.
    let same = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: doc.id.into(),
            markdown: "# Plan\n\nAll new.\n".into(),
            write: claude(),
        })
        .unwrap();
    assert!(!same.written.changed && same.snapshot.is_none());
    assert_eq!(versions(&fx).len(), 3);

    // Restoring keeps the text it replaces; restoring that version again is a way back.
    let restored = fx
        .library
        .restore_version(&RestoreVersionRequest {
            version: kept.id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(restored.written.changed);
    assert_eq!(
        restored.snapshot.as_ref().unwrap().label.as_deref(),
        Some("Before a version was restored by Claude")
    );
    assert_eq!(fx.markdown(doc.id), "# Plan\n\nFirst draft.\n");
    fx.library
        .restore_version(&RestoreVersionRequest {
            version: restored.snapshot.unwrap().id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(fx.markdown(doc.id), "# Plan\n\nAll new.\n");

    // Deleting for good is refused without the explicit field, for every kind of it.
    let refused = fx.library.delete_document(&DeleteDocumentRequest {
        document: doc.id.into(),
        permanently: false,
        write: claude(),
    });
    match refused {
        Err(error @ Error::ConfirmationRequired { .. }) => {
            assert_eq!(error.kind(), ErrorKind::ConfirmationRequired);
            assert!(error.to_string().contains("Nothing was deleted"));
        }
        other => panic!("{other:?}"),
    }
    fx.create(Some("Folder"), "inside");
    assert!(matches!(
        fx.library.delete_folder(&DeleteFolderRequest {
            folder: Target::path("Folder"),
            permanently: false,
            write: claude(),
        }),
        Err(Error::ConfirmationRequired { .. })
    ));
    assert!(matches!(
        fx.library.empty_trash(&EmptyTrashRequest {
            permanently: false,
            write: claude(),
        }),
        Err(Error::ConfirmationRequired { .. })
    ));
    let status = fx.library.status().unwrap();
    assert_eq!(
        (status.documents, status.folders, status.trashed),
        (2, 1, 0)
    );
    assert_eq!(fx.markdown(doc.id), "# Plan\n\nAll new.\n");
}

/// What the app stores for a document: `writer_app::document_io::StoredForm`.
fn as_the_app_stores(
    document: &Document,
    kept_title: Option<&str>,
) -> (String, String, String, u64, u64) {
    let stats = document.stats();
    let title = match kept_title {
        Some(title) => title.to_string(),
        None => document
            .title()
            .map(|title| title.trim().to_string())
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "Untitled".to_string()),
    };
    (
        document.to_json(),
        title,
        document.plain_text(),
        stats.words as u64,
        stats.characters as u64,
    )
}

#[test]
fn a_document_written_here_is_stored_exactly_as_the_app_stores_one() {
    let fx = fixture();
    let markdown = "# Ein Tag am Meer\n\nDie *M\u{f6}we* \u{2014} wei\u{df} und laut \u{2014} \
                    flog \u{fc}ber den Hafen. \u{1f30a}\n\n- eins\n- zwei\n\n> Zitat\n";
    let created = fx.create(Some("B\u{fc}cher"), markdown);
    let check = |kept_title: Option<&str>| {
        let store = fx.store();
        let stored = store.document(created.id).unwrap();
        let document = Document::from_json(&stored.content).unwrap();
        document.check_invariants().unwrap();
        let (json, title, plain_text, words, characters) = as_the_app_stores(&document, kept_title);
        assert_eq!(
            stored.content, json,
            "the stored JSON is the document's own"
        );
        assert_eq!(stored.meta.title, title);
        assert_eq!(stored.meta.word_count, words);
        assert_eq!(stored.meta.char_count, characters);
        // The search index holds the plain text: a word of it finds the document, with
        // the snippet the library builds from it.
        let word = plain_text
            .split_whitespace()
            .find(|word| word.chars().count() > 4 && word.chars().all(char::is_alphabetic))
            .unwrap()
            .to_string();
        let hits = store.search(&word, 5).unwrap();
        assert_eq!(hits.len(), 1, "{word}");
        assert_eq!(hits[0].document_id, created.id);
        assert_eq!(hits[0].title, title);
        // The tree shows the same row.
        let tree = store.library_tree().unwrap();
        let listed = tree.find_document(created.id).unwrap();
        assert_eq!(listed, &stored.meta);
        document
    };
    let document = check(None);
    assert_eq!(created.title, "Ein Tag am Meer");
    assert_eq!(created.path, "B\u{fc}cher/Ein Tag am Meer");
    assert_eq!(document.blocks().len(), 5);
    assert_eq!(created.words, document.stats().words as u64);
    assert_eq!(created.characters, document.stats().characters as u64);
    // The settings are the app's settings for a new document.
    assert_eq!(
        document.settings(),
        &writer_core::DocumentSettings::default()
    );

    // After every kind of change the row is still what the app would have stored.
    fx.library
        .edit(&EditRequest {
            document: created.id.into(),
            old_text: "wei\u{df} und laut".into(),
            new_text: "grau und **leise**".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let document = check(None);
    assert!(document.plain_text().contains("grau und leise"));
    // The title follows the text, as in the app.
    fx.library
        .edit(&EditRequest {
            document: created.id.into(),
            old_text: "Ein Tag am Meer".into(),
            new_text: "Zwei Tage am Meer".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    check(None);
    assert_eq!(
        fx.store().document_meta(created.id).unwrap().title,
        "Zwei Tage am Meer"
    );
    fx.library
        .insert(&InsertRequest {
            document: created.id.into(),
            markdown: "Ende.".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    check(None);
    fx.library
        .add_comment(&AddCommentRequest {
            document: created.id.into(),
            quote: "Hafen".into(),
            body: "Welcher?".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let document = check(None);
    assert_eq!(document.comment_count(), 1);

    // A chosen title is kept through later changes, and flagged the way the app flags it.
    fx.library
        .rename_document(&RenameDocumentRequest {
            document: created.id.into(),
            title: "Meer".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let flag: Option<i64> = fx
        .store()
        .setting(&format!("document.custom_title.{}", created.id.0))
        .unwrap();
    assert_eq!(
        flag,
        Some(fx.store().document_meta(created.id).unwrap().created_at)
    );
    fx.library
        .replace_document(&ReplaceDocumentRequest {
            document: created.id.into(),
            markdown: "# Ganz anders\n\nNeu.\n".into(),
            write: claude(),
        })
        .unwrap();
    check(Some("Meer"));
    assert!(
        fx.library
            .info(&InfoRequest {
                document: created.id.into()
            })
            .unwrap()
            .custom_title
    );
    // Given back to the text, it follows it again.
    fx.library
        .rename_document(&RenameDocumentRequest {
            document: created.id.into(),
            follow_text: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    check(None);
    assert_eq!(
        fx.store().document_meta(created.id).unwrap().title,
        "Ganz anders"
    );
    let flag: Option<i64> = fx
        .store()
        .setting(&format!("document.custom_title.{}", created.id.0))
        .unwrap();
    assert_eq!(flag, None);

    // A document without a heading takes its first line; an empty one is "Untitled".
    let post = fx.create(None, "Just a thought. And a second sentence.\n");
    assert_eq!(post.title, "Just a thought");
    let empty = fx.create(None, "");
    assert_eq!(empty.title, "Untitled");
    assert_eq!((empty.words, empty.characters), (0, 0));
    assert_eq!(fx.document(empty.id), Document::new());
    // A given title that is not the text's is a chosen one.
    let named = fx
        .library
        .create_document(&CreateDocumentRequest {
            markdown: "Body only.".into(),
            title: Some("My name".into()),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .document;
    assert_eq!(named.title, "My name");
    let info = fx
        .library
        .info(&InfoRequest {
            document: named.id.into(),
        })
        .unwrap();
    assert!(info.custom_title);
    assert!(
        !fx.library
            .info(&InfoRequest {
                document: post.id.into()
            })
            .unwrap()
            .custom_title
    );
}

#[test]
fn new_documents_take_the_settings_the_app_gives_new_documents() {
    let fx = fixture();
    fx.create(None, "first");
    // What the app's settings write.
    let mut store = writer_store::Store::open(fx.library.database_path()).unwrap();
    store.set_setting("new_document.font", "Georgia").unwrap();
    store.set_setting("new_document.size", &19.0f32).unwrap();
    store
        .set_setting("new_document.line_height", &1.4f32)
        .unwrap();
    store.set_setting("new_document.paper", "letter").unwrap();
    store.set_setting("new_document.page_view", &true).unwrap();
    store.close().unwrap();
    let created = fx.create(None, "# Second\n\ntext");
    let info = fx
        .library
        .info(&InfoRequest {
            document: created.id.into(),
        })
        .unwrap();
    assert_eq!(info.settings.font, "Georgia");
    assert_eq!(info.settings.text_size, 19.0);
    assert_eq!(info.settings.line_height, 1.4);
    assert_eq!(info.settings.paper, "US Letter");
    assert!(info.settings.page_view);
    assert_eq!(info.settings.orientation, "portrait");
    assert_eq!(info.settings.spelling_language, None);
    // A rewrite keeps the settings of the document.
    fx.library
        .replace_document(&ReplaceDocumentRequest {
            document: created.id.into(),
            markdown: "# Second\n\nother text".into(),
            write: claude(),
        })
        .unwrap();
    let after = fx
        .library
        .info(&InfoRequest {
            document: created.id.into(),
        })
        .unwrap();
    assert_eq!(after.settings, info.settings);
}

#[test]
fn a_book_of_ten_thousand_blocks_is_read_and_edited_in_good_time() {
    let fx = fixture();
    let mut markdown = String::new();
    for chapter in 0..100 {
        markdown.push_str(&format!("# Chapter {chapter}\n\n"));
        for paragraph in 0..99 {
            markdown.push_str(&format!(
                "Paragraph {paragraph} of chapter {chapter}: the quick brown fox jumps over \
                 the lazy dog, and the *gulls* cry over the harbour while the tide comes in.\n\n"
            ));
        }
    }
    let timed = |what: &str, limit: Duration, run: &mut dyn FnMut()| {
        let started = Instant::now();
        run();
        let took = started.elapsed();
        eprintln!("10,000 blocks: {what} took {took:?}");
        assert!(took < limit, "{what} took {took:?}");
    };
    // The limits are generous: this is a debug build, on a machine that may be busy.
    let limit = Duration::from_secs(20);
    let mut id = None;
    timed("create", limit, &mut || {
        id = Some(fx.create(Some("Book"), &markdown).id);
    });
    let id = id.unwrap();
    assert_eq!(fx.document(id).block_count(), 10_000);
    timed("read as Markdown", limit, &mut || {
        assert_eq!(fx.markdown(id).len(), markdown.trim_end().len() + 1);
    });
    timed("info", limit, &mut || {
        let info = fx
            .library
            .info(&InfoRequest {
                document: id.into(),
            })
            .unwrap();
        assert_eq!(info.headings.len(), 100);
    });
    timed("edit one place", limit, &mut || {
        let edited = fx
            .library
            .edit(&EditRequest {
                document: id.into(),
                old_text: "Paragraph 50 of chapter 50: the quick".into(),
                new_text: "Paragraph 50 of chapter 50: the **slow**".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!((edited.replacements, edited.blocks_changed), (1, 1));
        assert_eq!(edited.headings, ["Chapter 50"]);
    });
    timed("edit every place", limit, &mut || {
        let edited = fx
            .library
            .edit(&EditRequest {
                document: id.into(),
                old_text: "lazy dog".into(),
                new_text: "sleeping dog".into(),
                all: true,
                write: claude(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(edited.replacements, 9_900);
        assert_eq!(edited.blocks_changed, 9_900);
    });
    timed("comment", limit, &mut || {
        fx.library
            .add_comment(&AddCommentRequest {
                document: id.into(),
                quote: "Paragraph 7 of chapter 99".into(),
                body: "Near the end.".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
    });
    timed("append", limit, &mut || {
        fx.library
            .insert(&InsertRequest {
                document: id.into(),
                markdown: "# Epilogue\n\nThe end.".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
    });
    timed("replace a section", limit, &mut || {
        let replaced = fx
            .library
            .replace_section(&ReplaceSectionRequest {
                document: id.into(),
                heading: "Chapter 3".into(),
                markdown: "One paragraph is left.".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!((replaced.blocks_removed, replaced.blocks_added), (99, 1));
    });
    timed("rewrite with a few changes", limit, &mut || {
        let now = fx.markdown(id);
        let changed = now
            .replacen(
                "Paragraph 5 of chapter 10:",
                "Paragraph five of chapter ten:",
                1,
            )
            .replacen("# Chapter 20\n", "# Chapter twenty\n", 1);
        let rewritten = fx
            .library
            .replace_document(&ReplaceDocumentRequest {
                document: id.into(),
                markdown: changed,
                write: claude(),
            })
            .unwrap();
        assert_eq!(
            (
                rewritten.blocks_added,
                rewritten.blocks_removed,
                rewritten.blocks_changed
            ),
            (0, 0, 2)
        );
        // The comment near the end is where it was.
        assert!(rewritten.comments_detached.is_empty());
    });
    timed("search", limit, &mut || {
        let hits = fx
            .library
            .search(&SearchRequest {
                query: "epilogue".into(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(hits.hits.len(), 1);
    });
    let comments = fx
        .library
        .comments(&writer_agent::ListCommentsRequest {
            document: id.into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(comments.threads.len(), 1);
    assert_eq!(
        comments.threads[0].anchored_text.as_deref(),
        Some("Paragraph 7 of chapter 99")
    );
    assert_eq!(comments.threads[0].heading.as_deref(), Some("Chapter 99"));
}

/// Not a test of its own: run by the test below in a second process, where the data
/// directory comes from the environment.
#[test]
fn the_child_names_the_default_library() {
    let Some(expected) = std::env::var_os("WRITER_AGENT_TEST_EXPECTED_DIR") else {
        return;
    };
    let library = Library::default_location().unwrap();
    assert_eq!(library.data_dir(), std::path::Path::new(&expected));
    assert_eq!(
        Library::locate(None).unwrap().data_dir(),
        library.data_dir()
    );
    // Reading it does not create it.
    assert!(matches!(
        library.tree(&TreeRequest::default()),
        Err(Error::LibraryNotFound { .. })
    ));
    assert!(!library.data_dir().exists());
    println!("\nchild ok");
}

#[test]
fn the_default_library_follows_the_environment_variable() {
    let fx = fixture();
    let wanted = fx.dir.path().join("from the environment");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "suite::rules::the_child_names_the_default_library",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(writer_agent::DATA_DIR_ENV, &wanted)
        .env("WRITER_AGENT_TEST_EXPECTED_DIR", &wanted)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("child ok"), "{stdout}");
    // A named directory wins over everything.
    let named = Library::locate(Some(&fx.dir.path().join("named"))).unwrap();
    assert_eq!(named.data_dir(), fx.dir.path().join("named"));
}

#[test]
fn a_library_of_an_older_version_is_upgraded_for_reading_unless_the_app_has_it() {
    let fx = closed_fixture();
    seed(&fx);
    // As schema version 1 left it: without the index version 2 added.
    let downgrade = || {
        let conn = rusqlite::Connection::open(fx.library.database_path()).unwrap();
        conn.execute_batch("DROP INDEX documents_meta; PRAGMA user_version = 1;")
            .unwrap();
    };
    downgrade();
    // The app (of that older version) has it open: reading cannot upgrade it, and says so
    // instead of failing somewhere inside.
    let app = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    match fx.library.tree(&TreeRequest::default()) {
        Err(error @ Error::OutdatedLibrary { found: 1, .. }) => {
            assert_eq!(error.kind(), ErrorKind::AppIsOpen);
            assert!(error.to_string().contains("Quit The Writer"), "{error}");
        }
        other => panic!("{other:?}"),
    }
    drop(app);
    // With the app closed the library is upgraded, as the app would at its next start.
    let tree = fx.library.tree(&TreeRequest::default()).unwrap();
    assert_eq!(tree.total_documents, 3);
    assert_eq!(
        fx.store().schema_version().unwrap(),
        writer_store::SCHEMA_VERSION
    );
    assert_eq!(instance_lock::holder(&fx.data_dir()).unwrap(), None);
    // Writing upgrades it too.
    downgrade();
    fx.create(None, "# After the upgrade\n\ntext");
    assert_eq!(
        fx.store().schema_version().unwrap(),
        writer_store::SCHEMA_VERSION
    );

    // What is not a library is refused, for reading and for writing, and left alone.
    let other = closed_fixture();
    std::fs::create_dir_all(other.data_dir()).unwrap();
    let conn = rusqlite::Connection::open(other.library.database_path()).unwrap();
    conn.execute_batch("CREATE TABLE mine (x); INSERT INTO mine VALUES (1);")
        .unwrap();
    drop(conn);
    let before = std::fs::read(other.library.database_path()).unwrap();
    for error in [
        other.library.tree(&TreeRequest::default()).unwrap_err(),
        other
            .library
            .create_document(&CreateDocumentRequest {
                markdown: "x".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap_err(),
    ] {
        assert_eq!(error.kind(), ErrorKind::Failed);
        assert!(
            error.to_string().contains("not a library of The Writer"),
            "{error}"
        );
    }
    assert_eq!(
        std::fs::read(other.library.database_path()).unwrap(),
        before
    );
}

#[test]
fn content_that_is_not_a_document_is_reported_and_never_written_over() {
    let fx = seeded();
    let id = fx
        .library
        .resolve_document(&Target::path("Loose"))
        .unwrap()
        .id;
    let mut store = writer_store::Store::open(fx.library.database_path()).unwrap();
    let junk = "{\"format\": \"the-writer-document\", \"version\": 99, \"blocks\": []}";
    store
        .save_document(
            id,
            &writer_store::DocumentContent {
                content: junk,
                title: "Loose",
                plain_text: "from the future",
                word_count: 3,
                char_count: 15,
            },
        )
        .unwrap();
    store.close().unwrap();
    let unreadable = |error: Error| {
        assert_eq!(error.kind(), ErrorKind::Unreadable, "{error}");
        assert_eq!(error.exit_code(), 13);
        let message = error.to_string();
        assert!(
            message.contains("\u{201c}Loose\u{201d}") && message.contains("left as it is"),
            "{message}"
        );
    };
    unreadable(
        fx.library
            .read(&ReadRequest {
                document: id.into(),
                ..Default::default()
            })
            .unwrap_err(),
    );
    unreadable(
        fx.library
            .insert(&InsertRequest {
                document: id.into(),
                markdown: "More.".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap_err(),
    );
    unreadable(
        fx.library
            .replace_document(&ReplaceDocumentRequest {
                document: id.into(),
                markdown: "All new.".into(),
                write: claude(),
            })
            .unwrap_err(),
    );
    assert_eq!(fx.store().document(id).unwrap().content, junk);
    // What does not read its text still works: the tree, a move, the trash.
    assert_eq!(
        fx.library
            .tree(&TreeRequest::default())
            .unwrap()
            .total_documents,
        3
    );
    fx.library
        .move_document(&MoveDocumentRequest {
            document: id.into(),
            folder: Some(Target::path("Notes")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: id.into(),
            write: claude(),
        })
        .unwrap();
    assert_eq!(fx.store().document(id).unwrap().content, junk);
}

#[test]
fn requests_and_results_are_json() {
    let fx = seeded();
    // What an MCP client sends: the item as a string or a number, only the fields it
    // needs, the write options beside them.
    let request: EditRequest = serde_json::from_str(
        r#"{"document": "Book/Chapter 1", "old_text": "at dawn", "new_text": "at dusk",
            "author": "Claude", "dry_run": true}"#,
    )
    .unwrap();
    assert_eq!(request.document, Target::path("Book/Chapter 1"));
    assert_eq!(request.write.author.as_deref(), Some("Claude"));
    let tried = fx.library.edit(&request).unwrap();
    let json = serde_json::to_value(&tried).unwrap();
    assert_eq!(json["dry_run"], true);
    assert_eq!(json["changed"], false);
    // (In the run of the suite in which an app has every library open, it says so.)
    assert_eq!(json["app_is_open"], through_the_app());
    assert_eq!(json["app_takes_changes"], through_the_app());
    assert_eq!(json["replacements"], 1);
    assert_eq!(json["matched_as"], "text");
    assert_eq!(json["document"]["path"], "Book/Chapter 1");
    assert_eq!(json["snapshot"], serde_json::Value::Null);
    let back: writer_agent::ContentChanged = serde_json::from_value(json).unwrap();
    assert_eq!(back, tried);

    let request: InsertRequest = serde_json::from_str(
        r#"{"document": "doc:1", "markdown": "More.", "position": "after_heading",
            "heading": "Notes"}"#,
    )
    .unwrap();
    assert_eq!(request.document, Target::DocumentId(1));
    assert_eq!(request.position, InsertPosition::AfterHeading);
    let done = fx.library.insert(&request).unwrap();
    assert_eq!(done.headings, ["Notes"]);
    let request: MoveDocumentRequest =
        serde_json::from_str(r#"{"document": 3, "folder": "/", "placement": {"before": "Loose"}}"#)
            .unwrap();
    assert_eq!(request.folder, Some(Target::root()));
    let request: writer_agent::ExportRequest =
        serde_json::from_str(r#"{"folder": "Book", "path": "/tmp/x.docx", "format": "docx"}"#)
            .unwrap();
    assert_eq!(request.format, Some(writer_agent::ExportFormat::Docx));
    assert!(serde_json::from_str::<ReadRequest>(r#"{"document": 1, "format": "text"}"#).is_ok());
    assert!(serde_json::from_str::<ReadRequest>(r#"{"document": 1, "format": "pdf"}"#).is_err());

    // An error as data.
    let error = fx
        .library
        .edit(&EditRequest {
            document: Target::path("Chapter 1"),
            old_text: "o".into(),
            new_text: "0".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap_err();
    let report = serde_json::to_value(error.report()).unwrap();
    assert_eq!(report["code"], "not_unique");
    assert!(report["message"].as_str().unwrap().contains("occurs"));
    assert_eq!(report["occurrences"][0]["occurrence"], 1);
    assert!(report.get("candidates").is_none());
}
