//! Looking around: the tree, ids and paths, ambiguity, search, recent, info.

use innerword_agent::{
    Error, ErrorKind, FolderInfoRequest, InfoRequest, Item, ItemKind, ReadRequest, RecentRequest,
    RenameDocumentRequest, SearchRequest, Target, TrashDocumentRequest, TrashFolderRequest,
    TreeRequest,
};

use super::{Fixture, claude, fixture};

/// A small library:
///
/// ```text
/// The Salt Road/
///   Part one/
///     Chapter 1
///     Chapter 2
///   Part two/
///     Chapter 1
///   Outline
/// Notes/
///   Chapter 1
/// Ünïcödé — 東京/
///   Καλημέρα κόσμε
/// 1984
/// ```
fn salt_road() -> Fixture {
    let fx = fixture();
    fx.create(
        Some("The Salt Road/Part one"),
        "# Chapter 1\n\nThe harbour at dawn.\n",
    );
    fx.create(
        Some("The Salt Road/Part one"),
        "# Chapter 2\n\nThe lighthouse keeper.\n",
    );
    fx.create(
        Some("The Salt Road/Part two"),
        "# Chapter 1\n\nThe storm over the harbour.\n",
    );
    fx.create(Some("The Salt Road"), "# Outline\n\nThree parts.\n");
    fx.create(
        Some("Notes"),
        "# Chapter 1\n\nNotes on the first chapter.\n",
    );
    fx.create(
        Some("\u{dc}n\u{ef}c\u{f6}d\u{e9} \u{2014} \u{6771}\u{4eac}"),
        "# \u{39a}\u{3b1}\u{3bb}\u{3b7}\u{3bc}\u{3ad}\u{3c1}\u{3b1} \u{3ba}\u{3cc}\u{3c3}\u{3bc}\u{3b5}\n\n\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{6587}\u{7ae0} caf\u{e9} na\u{ef}ve \u{1f600}\n",
    );
    fx.create(None, "# 1984\n\nA year.\n");
    fx
}

#[test]
fn the_tree_lists_everything_with_ids_and_paths() {
    let fx = salt_road();
    let tree = fx.library.tree(&TreeRequest::default()).unwrap();
    assert_eq!(tree.folder, None);
    assert_eq!((tree.total_folders, tree.total_documents), (5, 7));
    let names: Vec<&str> = tree
        .folders
        .iter()
        .map(|f| f.folder.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "The Salt Road",
            "Notes",
            "\u{dc}n\u{ef}c\u{f6}d\u{e9} \u{2014} \u{6771}\u{4eac}"
        ]
    );
    assert_eq!(tree.documents.len(), 1);
    assert_eq!(tree.documents[0].path, "1984");
    assert_eq!(tree.documents[0].folder, "");
    assert_eq!(tree.documents[0].folder_id, None);

    let book = &tree.folders[0];
    assert_eq!((book.folder_count, book.document_count), (2, 4));
    assert_eq!(book.folder.path, "The Salt Road");
    assert_eq!(book.documents[0].path, "The Salt Road/Outline");
    let part_one = &book.folders[0];
    assert_eq!(part_one.folder.path, "The Salt Road/Part one");
    assert_eq!(part_one.folder.parent_id, Some(book.folder.id));
    let chapters: Vec<&str> = part_one.documents.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(
        chapters,
        [
            "The Salt Road/Part one/Chapter 1",
            "The Salt Road/Part one/Chapter 2"
        ]
    );
    assert_eq!(part_one.documents[0].folder, "The Salt Road/Part one");
    assert_eq!(part_one.documents[0].folder_id, Some(part_one.folder.id));
    assert_eq!(part_one.documents[0].words, 6);
    assert!(part_one.documents[0].created_at.0 > super::START);
    assert_eq!(tree.total_words, tree_words(&fx));

    // One folder, to a depth, with or without the documents.
    let part = fx
        .library
        .tree(&TreeRequest {
            folder: Some(Target::path("The Salt Road")),
            depth: Some(0),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(part.folder.as_ref().unwrap().name, "The Salt Road");
    assert_eq!((part.total_folders, part.total_documents), (2, 4));
    assert_eq!(part.documents.len(), 1);
    assert_eq!(part.folders.len(), 2);
    assert!(part.folders[0].documents.is_empty() && part.folders[0].folders.is_empty());
    assert_eq!(part.folders[0].document_count, 2);
    let folders = fx
        .library
        .tree(&TreeRequest {
            folders_only: true,
            ..Default::default()
        })
        .unwrap();
    assert!(folders.documents.is_empty());
    assert!(folders.folders[0].documents.is_empty());
    assert_eq!(folders.folders[0].folders.len(), 2);
    assert_eq!(folders.total_documents, 7);

    // The tree is JSON for whoever asks for it.
    let json = serde_json::to_value(&tree).unwrap();
    assert_eq!(json["folders"][0]["name"], "The Salt Road");
    assert_eq!(
        json["folders"][0]["folders"][0]["documents"][0]["title"],
        "Chapter 1"
    );
    assert_eq!(
        json["documents"][0]["created_at"].as_str().unwrap().len(),
        "2027-01-15T08:00:00.000Z".len()
    );
    let back: innerword_agent::Tree = serde_json::from_value(json).unwrap();
    assert_eq!(back, tree);
}

fn tree_words(fx: &Fixture) -> u64 {
    fx.store()
        .documents()
        .unwrap()
        .iter()
        .map(|d| d.word_count)
        .sum()
}

#[test]
fn documents_and_folders_are_named_by_id_or_by_path() {
    let fx = salt_road();
    let resolve = |target: &str| fx.library.resolve_document(&Target::parse(target));
    let two = resolve("The Salt Road/Part one/Chapter 2").unwrap();
    assert_eq!(two.title, "Chapter 2");
    // The end of a path is enough when it is unique.
    assert_eq!(resolve("Chapter 2").unwrap().id, two.id);
    assert_eq!(resolve("Part one/Chapter 2").unwrap().id, two.id);
    assert_eq!(resolve("  part ONE / chapter 2 ").unwrap().id, two.id);
    // By id, in every spelling.
    for spelling in [
        two.id.0.to_string(),
        format!("#{}", two.id.0),
        format!("id:{}", two.id.0),
        format!("doc:{}", two.id.0),
    ] {
        assert_eq!(resolve(&spelling).unwrap().id, two.id, "{spelling}");
    }
    assert_eq!(fx.library.resolve_document(&two.id.into()).unwrap(), two);
    assert_eq!(
        fx.library.resolve_document(&Target::Id(two.id.0)).unwrap(),
        two
    );
    // A name that is a number is a path from the top.
    assert_eq!(resolve("/1984").unwrap().title, "1984");
    assert!(matches!(resolve("1984"), Err(Error::NotFound { .. })));
    // From the top only.
    assert!(matches!(resolve("/Chapter 2"), Err(Error::NotFound { .. })));
    assert_eq!(resolve("/The Salt Road/Outline").unwrap().title, "Outline");
    // Unicode names, as they are and without regard to case.
    let greek = resolve("\u{dc}n\u{ef}c\u{f6}d\u{e9} \u{2014} \u{6771}\u{4eac}/\u{39a}\u{3b1}\u{3bb}\u{3b7}\u{3bc}\u{3ad}\u{3c1}\u{3b1} \u{3ba}\u{3cc}\u{3c3}\u{3bc}\u{3b5}").unwrap();
    assert_eq!(
        resolve("\u{3ba}\u{3b1}\u{3bb}\u{3b7}\u{3bc}\u{3ad}\u{3c1}\u{3b1} \u{3ba}\u{3cc}\u{3c3}\u{3bc}\u{3b5}").unwrap().id,
        greek.id
    );
    assert_eq!(
        greek.folder,
        "\u{dc}n\u{ef}c\u{f6}d\u{e9} \u{2014} \u{6771}\u{4eac}"
    );
    let text = fx
        .library
        .read(&ReadRequest {
            document: Target::path("\u{fc}n\u{ef}c\u{f6}d\u{e9} \u{2014} \u{6771}\u{4eac}/\u{39a}\u{3b1}\u{3bb}\u{3b7}\u{3bc}\u{3ad}\u{3c1}\u{3b1} \u{3ba}\u{3cc}\u{3c3}\u{3bc}\u{3b5}"),
            ..Default::default()
        })
        .unwrap();
    assert!(text.content.contains(
        "\u{65e5}\u{672c}\u{8a9e}\u{306e}\u{6587}\u{7ae0} caf\u{e9} na\u{ef}ve \u{1f600}"
    ));

    // Folders.
    let part = fx
        .library
        .resolve_folder(&Target::path("Part two"))
        .unwrap();
    assert_eq!(part.path, "The Salt Road/Part two");
    assert_eq!(
        fx.library
            .resolve_folder(&Target::parse(&format!("folder:{}", part.id.0)))
            .unwrap(),
        part
    );
    assert_eq!(fx.library.resolve_folder(&part.id.into()).unwrap(), part);
    assert!(matches!(
        fx.library.resolve_folder(&Target::root()),
        Err(Error::Invalid { .. })
    ));
    // A typed id of the wrong kind is refused instead of being taken for the other.
    assert!(matches!(
        fx.library.resolve_folder(&two.id.into()),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        fx.library.resolve_document(&part.id.into()),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        fx.library.resolve_document(&Target::default()),
        Err(Error::Invalid { .. })
    ));

    // What is not there.
    match resolve("Chapter 9") {
        Err(error @ Error::NotFound { .. }) => {
            assert_eq!(error.kind(), ErrorKind::NotFound);
            assert_eq!(error.exit_code(), 3);
            let message = error.to_string();
            assert!(
                message.contains("there is no document at \u{201c}Chapter 9\u{201d}"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    match resolve("Chapter") {
        Err(Error::NotFound {
            hint: Some(hint), ..
        }) => {
            assert!(hint.starts_with("Close: doc:"), "{hint}");
            assert!(hint.contains("The Salt Road/Part one/Chapter 1"), "{hint}");
        }
        other => panic!("{other:?}"),
    }
    match resolve("Part one") {
        Err(Error::NotFound {
            hint: Some(hint), ..
        }) => assert_eq!(hint, "A folder has that path"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(resolve("doc:999"), Err(Error::NotFound { .. })));
    assert!(matches!(
        fx.library.resolve_folder(&Target::path("Part three")),
        Err(Error::NotFound {
            kind: ItemKind::Folder,
            ..
        })
    ));

    // A name with a slash in it.
    let odd = fx.create(Some("AC\\/DC"), "# Back in Black / Live\n\ntext");
    assert_eq!(odd.path, "AC\\/DC/Back in Black \\/ Live");
    assert_eq!(resolve(&odd.path).unwrap().id, odd.id);
    assert_eq!(resolve("Back in Black \\/ Live").unwrap().id, odd.id);
    assert_eq!(
        fx.library
            .resolve_folder(&Target::path("AC\\/DC"))
            .unwrap()
            .name,
        "AC/DC"
    );
}

#[test]
fn a_name_that_fits_several_items_lists_them_with_their_ids() {
    let fx = salt_road();
    let error = fx
        .library
        .resolve_document(&Target::path("Chapter 1"))
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Ambiguous);
    assert_eq!(error.exit_code(), 4);
    let Error::Ambiguous {
        kind, candidates, ..
    } = &error
    else {
        panic!("{error:?}");
    };
    assert_eq!(*kind, ItemKind::Document);
    let paths: Vec<&str> = candidates.iter().map(|c| c.path.as_str()).collect();
    assert_eq!(
        paths,
        [
            "The Salt Road/Part one/Chapter 1",
            "The Salt Road/Part two/Chapter 1",
            "Notes/Chapter 1"
        ]
    );
    let message = error.to_string();
    for candidate in candidates {
        assert!(
            message.contains(&format!("{}  {}", candidate.id, candidate.path)),
            "{message}"
        );
        // Each candidate's id names exactly that document.
        let found = fx
            .library
            .resolve_document(&Target::DocumentId(candidate.id))
            .unwrap();
        assert_eq!(found.path, candidate.path);
    }
    let report = error.report();
    assert_eq!(report.code, "ambiguous");
    assert_eq!(report.candidates.len(), 3);
    let json = serde_json::to_value(&report).unwrap();
    assert_eq!(json["candidates"][2]["path"], "Notes/Chapter 1");
    assert_eq!(json["candidates"][2]["kind"], "document");
    // A longer path decides.
    let one = fx
        .library
        .resolve_document(&Target::path("Part two/Chapter 1"))
        .unwrap();
    assert_eq!(one.id.0, candidates[1].id);
    // The same in every operation that takes a document.
    assert!(matches!(
        fx.library.read(&ReadRequest {
            document: Target::path("Chapter 1"),
            ..Default::default()
        }),
        Err(Error::Ambiguous { .. })
    ));
    assert!(matches!(
        fx.library.rename_document(&RenameDocumentRequest {
            document: Target::path("chapter 1"),
            title: "x".into(),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Ambiguous { .. })
    ));

    // Folders with one name; and a name that is a folder and a document.
    fx.create(Some("Notes/Part one"), "x");
    match fx.library.resolve_folder(&Target::path("Part one")) {
        Err(Error::Ambiguous {
            kind, candidates, ..
        }) => {
            assert_eq!(kind, ItemKind::Folder);
            assert_eq!(candidates.len(), 2);
            assert!(candidates.iter().all(|c| c.kind == ItemKind::Folder));
        }
        other => panic!("{other:?}"),
    }
    fx.create(None, "# Notes\n\nA document called like the folder.");
    match fx.library.resolve(&Target::path("Notes"), false) {
        Err(Error::Ambiguous { candidates, .. }) => {
            let kinds: Vec<ItemKind> = candidates.iter().map(|c| c.kind).collect();
            assert_eq!(kinds, [ItemKind::Folder, ItemKind::Document]);
        }
        other => panic!("{other:?}"),
    }
    // Where only one kind is asked for, the name is not ambiguous.
    assert_eq!(
        fx.library
            .resolve_folder(&Target::path("Notes"))
            .unwrap()
            .name,
        "Notes"
    );
    assert_eq!(
        fx.library
            .resolve_document(&Target::path("/Notes"))
            .unwrap()
            .title,
        "Notes"
    );
    // Either kind, when the name says which.
    match fx.library.resolve(&Target::path("Outline"), false).unwrap() {
        Item::Document { document } => assert_eq!(document.path, "The Salt Road/Outline"),
        other => panic!("{other:?}"),
    }
    let item = fx
        .library
        .resolve(&Target::path("Part two"), false)
        .unwrap();
    assert!(matches!(&item, Item::Folder { folder } if folder.path == "The Salt Road/Part two"));
    assert_eq!(item.path(), "The Salt Road/Part two");
    assert!(!item.is_trashed());
    assert_eq!(fx.library.resolve(&item.target(), false).unwrap(), item);
    // A bare id that a document and a folder both have.
    match fx.library.resolve(&Target::Id(1), false) {
        Err(Error::Ambiguous { candidates, .. }) => assert_eq!(candidates.len(), 2),
        other => panic!("{other:?}"),
    }
    assert!(matches!(
        fx.library.resolve(&Target::parse("doc:1"), false).unwrap(),
        Item::Document { .. }
    ));
    assert!(matches!(
        fx.library
            .resolve(&Target::parse("folder:1"), false)
            .unwrap(),
        Item::Folder { .. }
    ));
    assert!(matches!(
        fx.library.resolve(&Target::path("Nowhere"), false),
        Err(Error::NotFound { .. })
    ));
}

#[test]
fn search_finds_words_in_titles_and_text() {
    let fx = salt_road();
    let search = |query: &str, folder: Option<&str>, limit: Option<usize>| {
        fx.library
            .search(&SearchRequest {
                query: query.into(),
                limit,
                folder: folder.map(Target::path),
            })
            .unwrap()
    };
    let found = search("harbour", None, None);
    assert_eq!(found.query, "harbour");
    let paths: Vec<&str> = found
        .hits
        .iter()
        .map(|h| h.document.path.as_str())
        .collect();
    assert_eq!(paths.len(), 2);
    assert!(paths.contains(&"The Salt Road/Part one/Chapter 1"));
    assert!(paths.contains(&"The Salt Road/Part two/Chapter 1"));
    let hit = &found.hits[0];
    assert!(
        hit.snippet.to_lowercase().contains("harbour"),
        "{}",
        hit.snippet
    );
    assert!(
        !hit.snippet.contains('\u{e000}'),
        "the markers are not part of the snippet"
    );
    let matched: Vec<&str> = hit
        .snippet_parts
        .iter()
        .filter(|part| part.is_match)
        .map(|part| part.text.as_str())
        .collect();
    assert_eq!(matched, ["harbour"]);
    assert_eq!(
        hit.snippet_parts
            .iter()
            .map(|p| p.text.as_str())
            .collect::<String>(),
        hit.snippet
    );
    assert!(found.hits[0].rank >= found.hits[1].rank);

    // Every word, the last one as a beginning, a phrase, a title.
    assert_eq!(search("storm harb", None, None).hits.len(), 1);
    assert_eq!(search("\"lighthouse keeper\"", None, None).hits.len(), 1);
    assert_eq!(search("\"keeper lighthouse\"", None, None).hits.len(), 0);
    assert_eq!(
        search("outline", None, None).hits[0].document.title,
        "Outline"
    );
    // In one folder, at any depth; a limit.
    let in_book = search("chapter", Some("The Salt Road"), None);
    assert_eq!(in_book.hits.len(), 3);
    assert!(
        in_book
            .hits
            .iter()
            .all(|h| h.document.path.starts_with("The Salt Road/"))
    );
    assert_eq!(search("chapter", Some("Part two"), None).hits.len(), 1);
    assert_eq!(search("chapter", None, None).hits.len(), 4);
    assert_eq!(search("chapter", None, Some(2)).hits.len(), 2);
    assert_eq!(search("chapter", Some("/"), None).hits.len(), 4);
    // Other scripts, accents or not; nothing; anything at all as a query.
    assert_eq!(search("cafe", None, None).hits.len(), 1);
    assert_eq!(
        search("\u{3ba}\u{3cc}\u{3c3}\u{3bc}\u{3b5}", None, None)
            .hits
            .len(),
        1
    );
    assert!(search("zebra", None, None).hits.is_empty());
    assert!(search("", None, None).hits.is_empty());
    assert!(search("\"unclosed AND (", None, None).hits.is_empty());
    assert!(matches!(
        fx.library.search(&SearchRequest {
            query: "x".into(),
            folder: Some(Target::path("No such folder")),
            ..Default::default()
        }),
        Err(Error::NotFound { .. })
    ));
    // What is in the trash is not found.
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: Target::path("Part two/Chapter 1"),
            write: claude(),
        })
        .unwrap();
    assert_eq!(search("harbour", None, None).hits.len(), 1);
}

#[test]
fn recent_documents_and_what_is_known_about_one() {
    let fx = salt_road();
    let recent = fx.library.recent(&RecentRequest::default()).unwrap();
    assert_eq!(recent.documents.len(), 7);
    assert_eq!(recent.documents[0].title, "1984");
    assert_eq!(recent.documents[6].path, "The Salt Road/Part one/Chapter 1");
    let two = fx
        .library
        .recent(&RecentRequest { limit: Some(2) })
        .unwrap();
    assert_eq!(two.documents.len(), 2);

    let created = fx.create(
        Some("Notes"),
        "# Plan\n\nIntro words here.\n\n## First\n\nOne two three.\n\n### Detail\n\nFour.\n\n## Second\n\nFive six.\n",
    );
    let info = fx
        .library
        .info(&InfoRequest {
            document: Target::path("Notes/Plan"),
        })
        .unwrap();
    assert_eq!(info.document, created);
    assert_eq!(info.document.path, "Notes/Plan");
    assert!(!info.custom_title);
    assert_eq!(info.paragraphs, 8);
    assert_eq!(info.document.words, 13);
    assert_eq!(info.reading_minutes, 1);
    assert!(info.characters_no_spaces < info.document.characters);
    let outline: Vec<(u8, &str, usize, u64)> = info
        .headings
        .iter()
        .map(|h| (h.level, h.text.as_str(), h.block, h.words))
        .collect();
    assert_eq!(
        outline,
        [
            (1, "Plan", 0, 13),
            (2, "First", 2, 6),
            (3, "Detail", 4, 2),
            (2, "Second", 6, 3)
        ]
    );
    assert_eq!(
        (info.open_comments, info.resolved_comments, info.versions),
        (0, 0, 0)
    );
    assert_eq!(info.settings.paper, "A4");
    assert_eq!(
        info.settings.font,
        innerword_core::Typography::default().body_font
    );
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["document"]["id"], created.id.0);
    assert_eq!(json["headings"][1]["text"], "First");

    // A folder: where it is and what is in it.
    let folder = fx
        .library
        .folder_info(&FolderInfoRequest {
            folder: Target::path("The Salt Road"),
        })
        .unwrap();
    assert_eq!(folder.folder.as_ref().unwrap().path, "The Salt Road");
    assert_eq!(folder.folders.len(), 2);
    assert_eq!(folder.documents.len(), 1);
    assert_eq!((folder.total_folders, folder.total_documents), (2, 4));
    assert!(folder.total_words > 0);
    let top = fx
        .library
        .folder_info(&FolderInfoRequest {
            folder: Target::root(),
        })
        .unwrap();
    assert_eq!(top.folder, None);
    assert_eq!(top.folders.len(), 3);
    assert_eq!(top.total_documents, 8);

    // By id a trashed document is found; by path it is not.
    fx.library
        .trash_folder(&TrashFolderRequest {
            folder: Target::path("Notes"),
            write: claude(),
        })
        .unwrap();
    let trashed = fx
        .library
        .info(&InfoRequest {
            document: created.id.into(),
        })
        .unwrap();
    assert!(trashed.document.trashed_at.is_some());
    assert_eq!(trashed.document.path, "Notes/Plan", "where it was");
    assert!(matches!(
        fx.library.info(&InfoRequest {
            document: Target::path("Notes/Plan"),
        }),
        Err(Error::NotFound { .. })
    ));
    assert_eq!(
        fx.library
            .recent(&RecentRequest::default())
            .unwrap()
            .documents
            .len(),
        6
    );
}
