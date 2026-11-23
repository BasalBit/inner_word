//! Whiteboards: made, drawn on, changed and cut down by agents, read as a listing,
//! exported as a PDF, and their pictures from files of this Mac (docs/whiteboard-design.md,
//! section 7). Every test with `fixture()` runs twice: directly, and with an app that has
//! the library open (`THROUGH_THE_APP`).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use innerword_agent::live::{self, Answer, Operation, Request};
use innerword_agent::{
    AddCommentRequest, AddImageRequest, AddToBoardRequest, BOARD_IS_PDF_ONLY, BoardChanged,
    BoardPicture, BoardsRequest, CreateBoardRequest, DeleteDocumentRequest, DocumentId,
    DocumentKind, DuplicateDocumentRequest, EditRequest, Error, ErrorKind, ExportFormat,
    ExportRequest, FindRequest, InfoRequest, InsertRequest, Library, ListCommentsRequest,
    ListVersionsRequest, ReadRequest, ReadVersionRequest, Refusal, RemoveFromBoardRequest,
    RenameDocumentRequest, ReplaceDocumentRequest, RestoreDocumentRequest, RestoreVersionRequest,
    SearchRequest, Target, TextFormat, TrashDocumentRequest, UpdateBoardRequest, WriteOptions,
};
use innerword_board::Scene;
use innerword_store::{FileId, Store};

use super::rules::dry;
use super::{
    Fixture, PLAN, changes, claude, closed_fixture, fixture, skeletons, stored_png, through_the_app,
};

// ----- helpers -------------------------------------------------------------------------------

/// Makes the whiteboard `title` in `folder` (created when missing) with `elements`.
fn board(fx: &Fixture, folder: Option<&str>, title: &str, elements: &str) -> BoardChanged {
    fx.clock.advance_ms(1000);
    fx.library
        .create_board(&CreateBoardRequest {
            title: title.into(),
            folder: folder.map(Target::path),
            create_folders: true,
            elements: skeletons(elements),
            write: claude(),
            ..Default::default()
        })
        .expect("the whiteboard is made")
}

/// The stored scene of a whiteboard.
fn scene(fx: &Fixture, id: DocumentId) -> Scene {
    let content = fx.store().document(id).expect("the whiteboard").content;
    Scene::from_stored(&content)
        .expect("a stored scene reads")
        .scene
}

/// Adds `elements` to the whiteboard `id`.
fn add(fx: &Fixture, id: DocumentId, elements: &str) -> innerword_agent::Result<BoardChanged> {
    fx.clock.advance_ms(1000);
    fx.library.add_to_board(&AddToBoardRequest {
        board: id.into(),
        elements: skeletons(elements),
        write: claude(),
        ..Default::default()
    })
}

/// The listing of a whiteboard, as `innerword get` prints it.
fn listing(fx: &Fixture, id: DocumentId) -> String {
    fx.read(id, TextFormat::Markdown)
}

/// The message and the exit code of an error.
fn said(error: Error) -> (u8, String) {
    (error.exit_code(), error.to_string())
}

/// The labels of the versions of a document, newest first.
fn labels(fx: &Fixture, id: DocumentId) -> Vec<String> {
    fx.library
        .versions(&ListVersionsRequest {
            document: id.into(),
        })
        .unwrap()
        .versions
        .into_iter()
        .filter_map(|version| version.label)
        .collect()
}

// ----- the operations ------------------------------------------------------------------------

#[test]
fn a_whiteboard_is_made_drawn_on_changed_and_cut_down() {
    let fx = fixture();
    let made = board(&fx, Some("Sketches"), "Architecture", "[]");
    let id = made.board.id;
    assert_eq!(made.board.kind, DocumentKind::Board);
    assert!(made.board.is_board() && !made.board.is_page());
    assert_eq!(made.board.reference(), format!("board:{}", id.0));
    assert_eq!(made.board.path, "Sketches/Architecture");
    assert!(made.added.is_empty() && made.version.is_none());
    assert!(made.written.changed && made.written.would_change);
    assert_eq!(made.written.app_is_open, through_the_app());
    assert_eq!(
        listing(&fx, id),
        format!(
            "board:{} \u{201c}Architecture\u{201d} in Sketches: empty",
            id.0
        )
    );
    assert_eq!(
        fx.store().schema_version().unwrap(),
        5,
        "a library with a whiteboard"
    );

    // Shapes with labels, an arrow bound to both, a title and a frame around them.
    let drawn = add(
        &fx,
        id,
        &format!(
            "{},{}",
            PLAN.trim_end().trim_end_matches(']'),
            r#"{"type":"text","id":"title","x":100,"y":20,"text":"System overview","fontSize":28},
               {"type":"frame","id":"backend","children":["api","db","reads"],"name":"Backend"}]"#
        ),
    )
    .unwrap();
    let ids: Vec<&str> = drawn.added.iter().map(|added| added.id.as_str()).collect();
    assert_eq!(ids, ["api", "db", "reads", "title", "backend"]);
    assert_eq!(drawn.added[0].r#type, "rectangle");
    assert_eq!(drawn.added[0].with.len(), 1, "the label came with it");
    assert_eq!(
        drawn.version.as_ref().and_then(|v| v.label.as_deref()),
        Some("Before a drawing by Claude")
    );
    assert!(drawn.written.changed);
    let after = scene(&fx, id);
    assert_eq!(after.elements().len(), 8, "five elements and three labels");
    assert_eq!(
        after.element("api").unwrap().json()["backgroundColor"],
        "#a5d8ff"
    );
    // The stored row: its text is what the library searches, its counts are of that text.
    let meta = fx.store().document_meta(id).unwrap();
    assert_eq!(meta.kind, DocumentKind::Board);
    assert!(meta.title == "Architecture");
    let text = fx.store().document(id).unwrap();
    assert_eq!(text.content, after.to_stored());
    let stats = innerword_core::Fragment::from_plain_text(&after.text())
        .to_document()
        .stats();
    assert_eq!(meta.word_count, stats.words as u64);
    assert_eq!(drawn.board.words, meta.word_count);
    let hits = fx
        .library
        .search(&SearchRequest {
            query: "Postgres".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].document.id, id);

    // Read: the listing, as Markdown and as plain text alike; the scene as JSON.
    let listed = listing(&fx, id);
    assert_eq!(listed, fx.read(id, TextFormat::PlainText));
    let lines: Vec<&str> = listed.lines().collect();
    assert!(
        lines[0].starts_with(&format!(
            "board:{} \u{201c}Architecture\u{201d} in Sketches: 5 elements, 0 pictures; drawn area",
            id.0
        )),
        "{listed}"
    );
    for (line, starts) in lines[1..].iter().zip([
        "rectangle  api",
        "ellipse    db",
        "arrow      reads    from api to db",
        "text       title",
        "frame      backend",
    ]) {
        assert!(line.starts_with(starts), "{line:?} in\n{listed}");
    }
    assert!(lines[1].contains(r#"label "API""#), "{listed}");
    assert!(lines[5].contains("holds api, db, reads"), "{listed}");
    let read = fx
        .library
        .read(&ReadRequest {
            document: id.into(),
            format: TextFormat::Json,
            comments: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(read.content, after.to_stored());
    assert_eq!(read.comments, Some(Vec::new()));
    let info = read.board.expect("what a whiteboard holds");
    assert_eq!(info.summary.elements, 8);
    assert_eq!(info.summary.by_type["text"], 4);
    let elements = info.elements.expect("every element as data");
    let api = elements.iter().find(|e| e.id == "api").unwrap();
    assert_eq!(api.label.as_deref(), Some("API"));
    assert_eq!(api.frame.as_deref(), Some("backend"));
    assert!(read.page.is_none());
    // As JSON results are written.
    let json = serde_json::to_value(&drawn).unwrap();
    assert_eq!(json["added"][0]["id"], "api");
    assert_eq!(json["changed"], true, "{json}");
    assert_eq!(json["updated"], serde_json::json!([]));

    // Info: the counts, and a title that is the name it was given.
    let info = fx
        .library
        .info(&InfoRequest {
            document: id.into(),
        })
        .unwrap();
    assert!(info.custom_title);
    assert_eq!(info.headings, []);
    assert_eq!(info.versions, 1);
    assert_eq!(info.board.as_ref().unwrap().summary.elements, 8);
    assert!(info.board.unwrap().elements.is_none());

    // Find names the element of each hit.
    let find = |text: &str| {
        fx.library
            .find(&FindRequest {
                document: id.into(),
                text: text.into(),
                ..Default::default()
            })
            .unwrap()
    };
    let found = find("reads");
    assert_eq!(found.count, 1);
    assert_eq!(
        found.occurrences[0].element.as_deref(),
        Some("reads (arrow label)")
    );
    assert_eq!(
        find("Backend").occurrences[0].element.as_deref(),
        Some("backend (frame name)")
    );
    assert_eq!(
        find("overview").occurrences[0].element.as_deref(),
        Some("title (text)")
    );
    assert_eq!(
        find("API").occurrences[0].element.as_deref(),
        Some("api (rectangle label)")
    );
    assert_eq!(find("nothing like it").count, 0);
    // Comments: a whiteboard has none.
    assert!(
        fx.library
            .comments(&ListCommentsRequest {
                document: id.into(),
                ..Default::default()
            })
            .unwrap()
            .threads
            .is_empty()
    );

    // Update: what follows by itself does.
    fx.clock.advance_ms(1000);
    let changed = fx
        .library
        .update_board(&UpdateBoardRequest {
            board: id.into(),
            changes: changes(
                r##"[{"id":"api","dx":40,"backgroundColor":"#ffc9c9"},{"id":"db","text":"Postgres 17"}]"##,
            ),
            write: claude(),
        })
        .unwrap();
    for id in ["api", "reads"] {
        assert!(changed.updated.iter().any(|u| u == id), "{changed:?}");
    }
    assert_eq!(
        changed.version.as_ref().and_then(|v| v.label.as_deref()),
        Some("Before an edit by Claude")
    );
    let after = scene(&fx, id);
    assert_eq!(after.element("api").unwrap().x(), 140.0);
    assert_eq!(
        after.element("api").unwrap().json()["backgroundColor"],
        "#ffc9c9"
    );
    assert!(listing(&fx, id).contains(r#"label "Postgres 17""#));

    // Remove: a shape goes with its label, the arrow bound to it stays, unbound.
    fx.clock.advance_ms(1000);
    let before_removal = scene(&fx, id);
    let label = before_removal
        .elements()
        .iter()
        .find(|e| e.container_id() == Some("db"))
        .unwrap()
        .id()
        .to_string();
    let removed = fx
        .library
        .remove_from_board(&RemoveFromBoardRequest {
            board: id.into(),
            ids: vec!["db".into()],
            write: claude(),
        })
        .unwrap();
    assert!(removed.removed.contains(&"db".to_string()));
    assert!(removed.removed.contains(&label));
    assert_eq!(removed.unbound, ["reads"]);
    assert_eq!(
        removed.version.as_ref().and_then(|v| v.label.as_deref()),
        Some("Before a removal by Claude")
    );
    let after = scene(&fx, id);
    assert!(after.element("db").is_none() && after.element(&label).is_none());
    assert!(after.element("reads").unwrap().binding("end").is_none());

    // The versions say why each was kept, newest first.
    assert_eq!(
        labels(&fx, id),
        [
            "Before a removal by Claude",
            "Before an edit by Claude",
            "Before a drawing by Claude"
        ]
    );
    // A version reads as the listing it had, or as its scene.
    let versions = fx
        .library
        .versions(&ListVersionsRequest {
            document: id.into(),
        })
        .unwrap()
        .versions;
    let first = fx
        .library
        .read_version(&ReadVersionRequest {
            version: versions[2].id.0,
            format: TextFormat::Markdown,
        })
        .unwrap();
    assert_eq!(
        first.content,
        format!(
            "board:{} \u{201c}Architecture\u{201d} in Sketches: empty",
            id.0
        )
    );
    let as_json = fx
        .library
        .read_version(&ReadVersionRequest {
            version: versions[0].id.0,
            format: TextFormat::Json,
        })
        .unwrap();
    assert_eq!(as_json.content, before_removal.to_stored());

    // Restoring the drawing as it was before the removal keeps the drawing as it is.
    fx.clock.advance_ms(1000);
    let restored = fx
        .library
        .restore_version(&RestoreVersionRequest {
            version: versions[0].id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(restored.written.changed);
    assert_eq!(scene(&fx, id).to_stored(), before_removal.to_stored());
    assert_eq!(
        labels(&fx, id)[0],
        "Before a version was restored by Claude"
    );
    assert_eq!(fx.store().document_meta(id).unwrap().title, "Architecture");

    // The list of whiteboards.
    let other = board(
        &fx,
        None,
        "Loose sketch",
        r#"[{"type":"ellipse","x":0,"y":0}]"#,
    );
    let all = fx.library.boards(&BoardsRequest::default()).unwrap();
    let names: Vec<&str> = all.boards.iter().map(|b| b.board.title.as_str()).collect();
    assert_eq!(names, ["Loose sketch", "Architecture"], "the newest first");
    assert_eq!((all.boards[1].elements, all.boards[1].pictures), (5, 0));
    assert!(!all.boards[1].trashed);
    let in_folder = fx
        .library
        .boards(&BoardsRequest {
            folder: Some(Target::path("Sketches")),
        })
        .unwrap();
    assert_eq!(in_folder.boards.len(), 1);
    assert_eq!(in_folder.boards[0].board.id, id);

    // Item operations work on a whiteboard as on any item.
    fx.clock.advance_ms(1000);
    let renamed = fx
        .library
        .rename_document(&RenameDocumentRequest {
            document: Target::parse(&format!("board:{}", other.board.id.0)),
            title: "Garden".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(renamed.document.title, "Garden");
    let copy = fx
        .library
        .duplicate_document(&DuplicateDocumentRequest {
            document: id.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(copy.document.kind, DocumentKind::Board);
    assert_eq!(
        scene(&fx, copy.document.id).to_stored(),
        scene(&fx, id).to_stored()
    );
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: id.into(),
            write: claude(),
        })
        .unwrap();
    let trashed = fx.library.boards(&BoardsRequest::default()).unwrap();
    assert!(trashed.boards.iter().any(|b| b.board.id == id && b.trashed));
    let (code, message) =
        said(add(&fx, id, r#"[{"type":"text","x":0,"y":0,"text":"x"}]"#).unwrap_err());
    assert_eq!(code, 9, "{message}");
    fx.library
        .restore_document(&RestoreDocumentRequest {
            document: id.into(),
            write: claude(),
        })
        .unwrap();
    add(&fx, id, r#"[{"type":"text","x":0,"y":400,"text":"Back"}]"#).unwrap();
}

#[test]
fn a_damaged_whiteboard_is_listed_and_printed_as_it_is_stored() {
    let fx = fixture();
    let good = board(&fx, None, "Good", PLAN).board.id;
    let bad = board(&fx, Some("Sketches"), "Bad", PLAN).board.id;
    let kept = add(&fx, bad, r#"[{"type":"text","x":0,"y":300,"text":"More"}]"#)
        .unwrap()
        .version
        .expect("a version was kept first");
    // What a damaged file, or a newer version, could leave: a scene the parser refuses,
    // in the whiteboard and in its version.
    const DAMAGED: &str = r#"{"type":"excalidraw","version":2,"elements":"not a list"#;
    let conn = rusqlite::Connection::open(fx.library.database_path()).unwrap();
    conn.execute(
        "UPDATE documents SET content = ?1 WHERE id = ?2",
        rusqlite::params![DAMAGED, bad.0],
    )
    .unwrap();
    conn.execute(
        "UPDATE snapshots SET content = ?1 WHERE id = ?2",
        rusqlite::params![DAMAGED, kept.id.0],
    )
    .unwrap();
    drop(conn);

    // The list goes on past it, and says what can be said of it.
    let listed = fx.library.boards(&BoardsRequest::default()).unwrap();
    assert_eq!(listed.boards.len(), 2, "{listed:?}");
    let damaged = listed.boards.iter().find(|b| b.board.id == bad).unwrap();
    assert_eq!(damaged.board.title, "Bad");
    assert_eq!(damaged.board.path, "Sketches/Bad");
    assert_eq!((damaged.elements, damaged.pictures), (0, 0));
    assert!(damaged.unreadable.is_some(), "{damaged:?}");
    let fine = listed.boards.iter().find(|b| b.board.id == good).unwrap();
    assert_eq!(fine.unreadable, None);
    assert_eq!(fine.elements, 3);
    // And as data: the field is there only for the damaged one.
    let json = serde_json::to_value(&listed).unwrap();
    let rows = json["boards"].as_array().unwrap();
    assert_eq!(
        rows.iter()
            .filter(|row| row.get("unreadable").is_some())
            .count(),
        1
    );

    // The listing cannot be made, and says why.
    let listing = fx.library.read(&ReadRequest {
        document: bad.into(),
        format: TextFormat::Markdown,
        ..Default::default()
    });
    assert_eq!(listing.unwrap_err().kind(), ErrorKind::Unreadable);
    // As JSON it is printed as it is stored: the one way left to see what it holds.
    let raw = fx
        .library
        .read(&ReadRequest {
            document: bad.into(),
            format: TextFormat::Json,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(raw.content, DAMAGED);
    assert_eq!(raw.board, None, "nothing of it could be read");
    let version = fx
        .library
        .read_version(&ReadVersionRequest {
            version: kept.id.0,
            format: TextFormat::Json,
        })
        .unwrap();
    assert_eq!(version.content, DAMAGED);
    assert_eq!(
        fx.library
            .read_version(&ReadVersionRequest {
                version: kept.id.0,
                format: TextFormat::Markdown,
            })
            .unwrap_err()
            .kind(),
        ErrorKind::Unreadable
    );
    // A whiteboard that reads still reads as JSON with what it is.
    let good_json = fx
        .library
        .read(&ReadRequest {
            document: good.into(),
            format: TextFormat::Json,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        good_json.content,
        fx.store().document(good).unwrap().content
    );
    assert!(good_json.board.is_some());
}

#[test]
fn a_hint_writes_each_item_with_its_own_prefix() {
    let fx = fixture();
    let plan = board(&fx, Some("Sketches"), "Floor plan", PLAN).board.id;
    let chapter = fx.create(Some("Book"), "# Floor notes\n\nText.\n").id;
    // A path that names nothing: the close ones, each as it is named.
    let missing = fx
        .library
        .read(&ReadRequest {
            document: Target::path("Floor"),
            ..Default::default()
        })
        .unwrap_err();
    let message = missing.to_string();
    assert!(
        message.contains(&format!("board:{} Sketches/Floor plan", plan.0)),
        "{message}"
    );
    assert!(
        message.contains(&format!("doc:{} Book/Floor notes", chapter.0)),
        "{message}"
    );
    // In the trash.
    fx.library
        .trash_document(&TrashDocumentRequest {
            document: plan.into(),
            write: claude(),
        })
        .unwrap();
    let trashed = fx
        .library
        .read(&ReadRequest {
            document: Target::path("Sketches/Floor plan"),
            ..Default::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        trashed.contains(&format!("board:{} (in the trash)", plan.0)),
        "{trashed}"
    );
    // Where a folder is needed.
    let not_a_folder = fx
        .library
        .create_board(&CreateBoardRequest {
            title: "Inside".into(),
            folder: Some(Target::parse(&format!("board:{}", plan.0))),
            write: claude(),
            ..Default::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        not_a_folder.starts_with(&format!(
            "board:{} names a whiteboard; a folder is needed here",
            plan.0
        )),
        "{not_a_folder}"
    );
    // A permanent deletion asks, naming it as what it is.
    let asked = fx
        .library
        .delete_document(&DeleteDocumentRequest {
            document: plan.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        asked.contains(&format!(
            "The whiteboard \u{201c}Floor plan\u{201d} (board:{})",
            plan.0
        )),
        "{asked}"
    );
}

#[test]
fn a_whiteboard_is_named_as_board_or_whiteboard() {
    for written in ["board:12", "Board:12", "whiteboard:12", "page:12", "doc:12"] {
        assert_eq!(Target::parse(written), Target::DocumentId(12), "{written}");
    }
    assert_eq!(Target::parse("board:x"), Target::Path("board:x".into()));
}

#[test]
fn every_refusal_says_what_was_wrong_and_changes_nothing() {
    let fx = fixture();
    let plan = board(&fx, Some("Sketches"), "Plan", PLAN).board.id;
    let chapter = fx.create(Some("Book"), "# Chapter 1\n\nText.\n");
    let before = fx.dump();
    let is_a_whiteboard = "\u{201c}Plan\u{201d} is a whiteboard: it is changed with board add, \
                           board update and board remove.";

    // The text operations on a whiteboard.
    let text_ops: Vec<(&str, innerword_agent::Result<()>)> = vec![
        (
            "edit",
            fx.library
                .edit(&EditRequest {
                    document: plan.into(),
                    old_text: "API".into(),
                    new_text: "Gateway".into(),
                    write: claude(),
                    ..Default::default()
                })
                .map(|_| ()),
        ),
        (
            "insert",
            fx.library
                .insert(&InsertRequest {
                    document: plan.into(),
                    markdown: "More.".into(),
                    write: claude(),
                    ..Default::default()
                })
                .map(|_| ()),
        ),
        (
            "replace_document",
            fx.library
                .replace_document(&ReplaceDocumentRequest {
                    document: plan.into(),
                    markdown: "All new.".into(),
                    write: claude(),
                    ..Default::default()
                })
                .map(|_| ()),
        ),
        (
            "add_image",
            fx.library
                .add_image(&AddImageRequest {
                    document: plan.into(),
                    path: fx
                        .picture("a.png", (8, 8), [1, 2, 3])
                        .to_string_lossy()
                        .into_owned(),
                    write: claude(),
                    ..Default::default()
                })
                .map(|_| ()),
        ),
        (
            "add_comment",
            fx.library
                .add_comment(&AddCommentRequest {
                    document: plan.into(),
                    quote: "API".into(),
                    body: "Hm".into(),
                    write: claude(),
                    ..Default::default()
                })
                .map(|_| ()),
        ),
    ];
    for (name, result) in text_ops {
        let error = result.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WrongKind, "{name}");
        assert_eq!(said(error), (15, is_a_whiteboard.to_string()), "{name}");
    }

    // The whiteboard operations on a text and on a page.
    let on_a_text = add(
        &fx,
        chapter.id,
        r#"[{"type":"text","x":0,"y":0,"text":"x"}]"#,
    );
    assert_eq!(
        said(on_a_text.unwrap_err()),
        (
            15,
            "\u{201c}Chapter 1\u{201d} is a document, not a whiteboard.".to_string()
        )
    );
    let update_a_text = fx.library.update_board(&UpdateBoardRequest {
        board: chapter.id.into(),
        changes: changes(r#"[{"id":"x","dx":1}]"#),
        write: claude(),
    });
    assert_eq!(said(update_a_text.unwrap_err()).0, 15);
    assert_eq!(fx.dump(), before, "nothing was changed");
    let page = fx.page_folder(
        "page",
        &[(
            "index.html",
            "<!doctype html><title>Orbitarium</title><p>Hi.</p>",
        )],
    );
    let page = fx
        .library
        .add_page(&innerword_agent::AddPageRequest {
            path: page.to_string_lossy().into_owned(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .page
        .id;
    let before = fx.dump();
    let on_a_page = fx.library.remove_from_board(&RemoveFromBoardRequest {
        board: page.into(),
        ids: vec!["x".into()],
        write: claude(),
    });
    assert_eq!(
        said(on_a_page.unwrap_err()),
        (
            15,
            "\u{201c}Orbitarium\u{201d} is a page, not a whiteboard.".to_string()
        )
    );

    // What the whiteboard does not have, and what cannot be read.
    let ghost = fx
        .library
        .update_board(&UpdateBoardRequest {
            board: plan.into(),
            changes: changes(r#"[{"id":"ghost","dx":1}]"#),
            write: claude(),
        })
        .unwrap_err();
    assert_eq!(ghost.kind(), ErrorKind::NotFound);
    let (code, message) = said(ghost);
    assert_eq!(code, 3);
    assert!(
        message.starts_with("there is no element \u{201c}ghost\u{201d} on the whiteboard \u{201c}Plan\u{201d}. It has "),
        "{message}"
    );
    assert!(message.contains("api"), "{message}");
    let unbound = add(
        &fx,
        plan,
        r#"[{"type":"arrow","x":0,"y":0,"end":{"id":"cache"}}]"#,
    );
    assert_eq!(said(unbound.unwrap_err()).0, 3);
    let removed_ghost = fx.library.remove_from_board(&RemoveFromBoardRequest {
        board: plan.into(),
        ids: vec!["ghost".into()],
        write: claude(),
    });
    assert_eq!(said(removed_ghost.unwrap_err()).0, 3);
    for (elements, field) in [
        (
            r#"[{"type":"rectangle","x":0,"y":0,"colour":"red"}]"#,
            "colour",
        ),
        (
            r#"[{"type":"text","x":0,"y":0,"text":"Hi","width":40}]"#,
            "width",
        ),
        (r#"[{"type":"iframe","x":0,"y":0}]"#, "type"),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"customData":{}}]"#,
            "customData",
        ),
        (
            r#"[{"type":"rectangle","x":0,"y":0,"width":0.5,"height":40}]"#,
            "width",
        ),
        (r#"[{"type":"text","x":0,"y":0,"text":"   "}]"#, "text"),
        (r#"[{"type":"rectangle","id":"api","x":0,"y":0}]"#, "id"),
        (
            r#"[{"type":"arrow","x":0,"y":0,"elbowed":true,"points":[[0,0],[50,0]]}]"#,
            "elbowed",
        ),
    ] {
        let error = add(&fx, plan, elements).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Invalid, "{elements}: {error}");
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("the element at index 0 ({field})")),
            "{elements}: {message}"
        );
    }
    let nothing = add(&fx, plan, "[]").unwrap_err();
    assert_eq!(said(nothing).0, 2);
    let too_many: Vec<String> = (0..2001)
        .map(|n| format!(r#"{{"type":"rectangle","x":{n},"y":0}}"#))
        .collect();
    let error = add(&fx, plan, &format!("[{}]", too_many.join(","))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    let follow = fx.library.rename_document(&RenameDocumentRequest {
        document: plan.into(),
        follow_text: true,
        write: claude(),
        ..Default::default()
    });
    let (code, message) = said(follow.unwrap_err());
    assert_eq!(code, 2);
    assert!(
        message.contains("a whiteboard has no text to take its title from"),
        "{message}"
    );
    let section = fx.library.read(&ReadRequest {
        document: plan.into(),
        heading: Some("Notes".into()),
        ..Default::default()
    });
    assert_eq!(said(section.unwrap_err()).0, 2);
    assert_eq!(fx.dump(), before, "nothing was changed");
    // The errors travel from the app as they were (the second run of the suite went
    // through it above); as data they say what the item is and what was wanted.
    let wire = live::WireError::of(&Error::WrongKind {
        id: 3,
        name: "\u{201c}Plan\u{201d}".into(),
        kind: DocumentKind::Board,
        wanted: None,
        export: true,
    });
    let back = wire.into_error();
    assert_eq!(
        back.to_string(),
        format!("\u{201c}Plan\u{201d} is a whiteboard, and {BOARD_IS_PDF_ONLY}.")
    );
    assert!(matches!(
        back,
        Error::WrongKind {
            kind: DocumentKind::Board,
            wanted: None,
            export: true,
            ..
        }
    ));
    let empty = live::WireError::of(&Error::EmptyBoard {
        id: 3,
        name: "\u{201c}Plan\u{201d}".into(),
    })
    .into_error();
    assert_eq!(
        said(empty),
        (
            9,
            "\u{201c}Plan\u{201d} is empty: there is nothing to export.".to_string()
        )
    );
}

#[test]
fn a_dry_run_writes_nothing_and_names_the_ids_the_real_run_gives() {
    let fx = fixture();
    let plan = board(&fx, Some("Sketches"), "Plan", PLAN).board.id;
    let request = |write: WriteOptions| AddToBoardRequest {
        board: plan.into(),
        elements: skeletons(
            r#"[{"type":"rectangle","x":0,"y":300,"label":{"text":"Cache"}},
                {"type":"arrow","x":0,"y":0,"start":{"id":"api"},"end":{"id":"db"}}]"#,
        ),
        write,
        ..Default::default()
    };
    let bytes = fx.bytes();
    let tried = fx.library.add_to_board(&request(dry())).unwrap();
    assert!(tried.written.dry_run && !tried.written.changed && tried.written.would_change);
    assert!(tried.version.is_none());
    assert_eq!(fx.bytes(), bytes, "a dry run wrote");
    fx.clock.advance_ms(5000);
    let done = fx.library.add_to_board(&request(claude())).unwrap();
    assert_eq!(tried.added, done.added, "the same ids, the same labels");
    assert!(done.written.changed && done.version.is_some());
    // Every value is derived: made again on the same scene, the same request makes the
    // same elements; on another whiteboard, other ids.
    let other = board(&fx, None, "Other", PLAN).board.id;
    let elsewhere = fx
        .library
        .add_to_board(&AddToBoardRequest {
            board: other.into(),
            ..request(claude())
        })
        .unwrap();
    assert_ne!(elsewhere.added[0].id, done.added[0].id);

    let bytes = fx.bytes();
    for result in [
        fx.library
            .update_board(&UpdateBoardRequest {
                board: plan.into(),
                changes: changes(r#"[{"id":"api","dy":30}]"#),
                write: dry(),
            })
            .unwrap(),
        fx.library
            .remove_from_board(&RemoveFromBoardRequest {
                board: plan.into(),
                ids: vec!["api".into()],
                write: dry(),
            })
            .unwrap(),
        fx.library
            .create_board(&CreateBoardRequest {
                title: "Not yet".into(),
                folder: Some(Target::path("New/Folder")),
                create_folders: true,
                elements: skeletons(PLAN),
                write: dry(),
                ..Default::default()
            })
            .unwrap(),
    ] {
        assert!(result.written.dry_run && !result.written.changed);
        assert!(result.written.would_change, "{result:?}");
        assert!(result.version.is_none());
    }
    assert_eq!(fx.bytes(), bytes, "a dry run wrote");
    let planned = fx
        .library
        .create_board(&CreateBoardRequest {
            title: "  Not   yet ".into(),
            folder: Some(Target::path("New/Folder")),
            create_folders: true,
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(planned.board.id, DocumentId(0));
    assert_eq!(planned.board.path, "New/Folder/Not yet");
    assert_eq!(planned.board.kind, DocumentKind::Board);
    // Without a title it is stored without one, as the app's New Whiteboard stores it,
    // and shown "Untitled" (docs/whiteboard-design.md, section 2); a dry run says the same.
    let planned_untitled = fx
        .library
        .create_board(&CreateBoardRequest {
            title: "   ".into(),
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(planned_untitled.board.title, "");
    assert_eq!(planned_untitled.board.path, "Untitled");
    let untitled = board(&fx, None, "", "[]");
    assert_eq!(untitled.board.title, "");
    assert_eq!(untitled.board.path, "Untitled");
    assert_eq!(
        fx.store().document_meta(untitled.board.id).unwrap().title,
        "",
        "stored as the app stores it"
    );
    assert!(
        listing(&fx, untitled.board.id).contains("\u{201c}Untitled\u{201d}"),
        "{}",
        listing(&fx, untitled.board.id)
    );
}

// ----- export --------------------------------------------------------------------------------

#[test]
fn a_whiteboard_is_exported_as_a_pdf_only_and_never_empty() {
    let fx = fixture();
    let plan = board(&fx, Some("Sketches"), "Plan", PLAN).board.id;
    let empty = board(&fx, Some("Sketches"), "Empty", "[]").board.id;
    fx.create(Some("Sketches"), "# Notes\n\nA few words.\n");
    let out = fx.dir.path().join("out");
    let export = |id: DocumentId, path: &Path| {
        fx.library.export(&ExportRequest {
            document: Some(id.into()),
            path: path.to_path_buf(),
            ..Default::default()
        })
    };

    let pdf = out.join("plan.pdf");
    let exported = export(plan, &pdf).unwrap();
    assert_eq!(exported.format, ExportFormat::Pdf);
    assert_eq!(exported.title, "Plan");
    assert_eq!(exported.documents[0].id, plan);
    assert!(exported.warnings.is_empty());
    let bytes = std::fs::read(&pdf).unwrap();
    assert_eq!(exported.bytes, bytes.len() as u64);
    // The fake printer's page is the size Rust estimates for the drawing and its margin.
    let (width, height) = scene(&fx, plan).pdf_size(32.0, 14_400.0).unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains(&format!("/MediaBox [0 0 {width} {height}]")),
        "{text}"
    );
    // A file that is there is not written over unless asked.
    let again = export(plan, &pdf).unwrap_err();
    assert_eq!(again.kind(), ErrorKind::FileExists);
    // A dry run writes nothing.
    let tried = fx
        .library
        .export(&ExportRequest {
            document: Some(plan.into()),
            path: out.join("tried.pdf"),
            dry_run: true,
            ..Default::default()
        })
        .unwrap();
    assert!(tried.dry_run && !out.join("tried.pdf").exists());

    // Every other format is refused.
    let pdf_only = format!("\u{201c}Plan\u{201d} is a whiteboard, and {BOARD_IS_PDF_ONLY}.");
    for path in ["plan.md", "plan.docx", "plan.txt", "plan"] {
        let error = export(plan, &out.join(path)).unwrap_err();
        assert_eq!(said(error), (15, pdf_only.clone()), "{path}");
    }
    for format in [ExportFormat::Html, ExportFormat::Markdown] {
        let error = fx
            .library
            .export(&ExportRequest {
                document: Some(plan.into()),
                path: out.join("named.pdf"),
                format: Some(format),
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(said(error).0, 15, "{format:?}");
    }
    let original = fx
        .library
        .export(&ExportRequest {
            document: Some(plan.into()),
            path: out.join("original.pdf"),
            original: true,
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(said(original).0, 15);

    // An empty whiteboard has no PDF, and WebKit is not even asked.
    let error = export(empty, &out.join("empty.pdf")).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::WrongState);
    assert_eq!(
        said(error),
        (
            9,
            "\u{201c}Empty\u{201d} is empty: there is nothing to export.".to_string()
        )
    );
    assert!(!out.join("empty.pdf").exists());

    // A folder exported as one file leaves its whiteboards out, and says so.
    let folder = fx
        .library
        .export(&ExportRequest {
            folder: Some(Target::path("Sketches")),
            path: out.join("sketches.md"),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(folder.documents.len(), 1);
    assert_eq!(
        folder.warnings,
        ["2 whiteboards were left out: a whiteboard is exported on its own, as a PDF."]
    );
    board(&fx, Some("Only boards"), "Lone", PLAN);
    let only = fx
        .library
        .export(&ExportRequest {
            folder: Some(Target::path("Only boards")),
            path: out.join("only.md"),
            ..Default::default()
        })
        .unwrap_err();
    assert!(only.to_string().contains("only whiteboards"), "{only}");

    // Without a printer there is no PDF.
    let bare = Library::at(fx.data_dir()).with_clock(fx.clock.clone());
    let error = bare
        .export(&ExportRequest {
            document: Some(plan.into()),
            path: out.join("bare.pdf"),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(
        said(error),
        (1, "the PDF of a whiteboard cannot be made here".to_string())
    );
}

// ----- pictures ------------------------------------------------------------------------------

/// The `image` skeleton of a file.
fn image_at(path: &Path, extra: &str) -> String {
    format!(
        r#"[{{"type":"image","x":640,"y":80,"path":{}{extra}}}]"#,
        serde_json::to_string(&path.to_string_lossy()).unwrap()
    )
}

#[test]
fn a_picture_from_a_file_is_read_where_asked_and_stored_before_the_scene_names_it() {
    let fx = fixture();
    let plan = board(&fx, None, "Plan", PLAN).board.id;

    // A PNG is stored as it came, under the id of its bytes.
    let png_path = fx.picture("pictures/harbour.png", (40, 30), [30, 90, 160]);
    let png = std::fs::read(&png_path).unwrap();
    let hex = FileId::of(&png).to_hex();
    let added = add(&fx, plan, &image_at(&png_path, r#","id":"photo""#)).unwrap();
    assert_eq!(added.added[0].id, "photo");
    assert_eq!(added.pictures.len(), 1);
    let picture = &added.pictures[0];
    assert_eq!(
        (picture.id.as_str(), picture.mime_type.as_str()),
        ("photo", "image/png")
    );
    assert_eq!(picture.pixels, (40, 30));
    assert!(picture.untouched);
    assert_eq!((picture.width, picture.height), (40.0, 30.0));
    let after = scene(&fx, plan);
    let element = after.element("photo").unwrap();
    assert_eq!(element.file_id(), Some(hex.as_str()));
    assert_eq!(element.json()["status"], "saved");
    let entry = &after.files()[&hex];
    assert_eq!(entry.mime_type, "image/png");
    assert_eq!(entry.file.as_ref().unwrap().as_str(), hex);
    let store = fx.store();
    assert_eq!(
        store.file_bytes(&FileId::parse(&hex).unwrap()).unwrap(),
        Some(png.clone())
    );
    assert!(listing(&fx, plan).contains("picture stored (png)"));
    let listed = fx.library.boards(&BoardsRequest::default()).unwrap();
    assert_eq!(listed.boards[0].pictures, 1);

    // The density a file declares gives its size on the board; one side given, the other
    // follows; both given, both are taken.
    let retina = fx.dir.path().join("pictures/retina.png");
    std::fs::write(
        &retina,
        innerword_image::fixtures::png_at(200, 100, [9, 9, 9], 144),
    )
    .unwrap();
    for (extra, size) in [
        ("", (100.0, 50.0)),
        (r#","width":50"#, (50.0, 25.0)),
        (r#","height":200"#, (400.0, 200.0)),
        (r#","width":30,"height":70"#, (30.0, 70.0)),
    ] {
        let added = add(&fx, plan, &image_at(&retina, extra)).unwrap();
        let picture = &added.pictures[0];
        assert_eq!((picture.width, picture.height), size, "{extra}");
    }

    // A picture larger than the library keeps is made smaller: not the file's own bytes.
    let wide = fx.dir.path().join("pictures/wide.png");
    std::fs::write(
        &wide,
        innerword_image::fixtures::png(5000, 20, [200, 10, 10]),
    )
    .unwrap();
    let added = add(&fx, plan, &image_at(&wide, "")).unwrap();
    let picture = &added.pictures[0];
    assert!(!picture.untouched);
    assert_eq!(picture.pixels.0, 4096);
    assert_eq!(
        picture.width, 1440.0,
        "a picture is drawn 1,440 wide at most"
    );
    // A TIFF is converted.
    let tiff = fx.dir.path().join("pictures/scan.tiff");
    let made = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "tiff"])
        .arg(&png_path)
        .arg("--out")
        .arg(&tiff)
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let added = add(&fx, plan, &image_at(&tiff, "")).unwrap();
    let picture = &added.pictures[0];
    assert!(!picture.untouched);
    assert!(
        ["image/png", "image/jpeg"].contains(&picture.mime_type.as_str()),
        "{picture:?}"
    );
    assert_eq!(picture.pixels, (40, 30));

    // An SVG document is kept as it came.
    let svg_bytes = br#"<?xml version="1.0"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60"><rect width="120" height="60" fill="teal"/></svg>"#;
    let svg = fx.dir.path().join("pictures/mark.svg");
    std::fs::write(&svg, svg_bytes).unwrap();
    let added = add(&fx, plan, &image_at(&svg, "")).unwrap();
    let picture = &added.pictures[0];
    assert_eq!(picture.mime_type, "image/svg+xml");
    assert_eq!(picture.pixels, (120, 60));
    assert!(picture.untouched);
    let id = FileId::of(svg_bytes);
    assert_eq!(
        fx.store().file_bytes(&id).unwrap(),
        Some(svg_bytes.to_vec())
    );
    assert_eq!(
        scene(&fx, plan).files()[&id.to_hex()].mime_type,
        "image/svg+xml"
    );

    // One file named by two skeletons is read once and stored once: a file nothing has
    // stored yet, so that one more file is all the library gains.
    let fresh_path = fx.picture("pictures/fresh.png", (24, 18), [210, 120, 40]);
    let fresh = FileId::of(&std::fs::read(&fresh_path).unwrap());
    assert_eq!(fx.store().file_bytes(&fresh).unwrap(), None);
    let files_before = fx.store().file_count().unwrap();
    let twice = add(
        &fx,
        plan,
        &format!(
            r#"[{0}, {1}]"#,
            image_at(&fresh_path, r#","id":"one""#).trim_matches(['[', ']']),
            image_at(&fresh_path, r#","id":"two","x":900"#).trim_matches(['[', ']'])
        ),
    )
    .unwrap();
    assert_eq!(fx.store().file_count().unwrap(), files_before + 1);
    assert!(fx.store().file_bytes(&fresh).unwrap().is_some());
    // Two elements, one picture behind both: one source, one file id, one entry of the
    // scene's files.
    assert_eq!(twice.pictures.len(), 2);
    assert!(
        twice
            .pictures
            .iter()
            .all(|picture| picture.source == fresh_path.to_string_lossy()),
        "{:?}",
        twice.pictures
    );
    let drawn = scene(&fx, plan);
    for id in ["one", "two"] {
        assert_eq!(
            drawn.element(id).unwrap().file_id(),
            Some(fresh.to_hex().as_str()),
            "{id}"
        );
    }
    assert_eq!(
        drawn
            .files()
            .values()
            .filter(|entry| entry.file.as_ref().map(|file| file.as_str())
                == Some(fresh.to_hex().as_str()))
            .count(),
        1
    );
    // The request that travels carries the picture once (what the tool read once, and
    // what the app's listener stores once): the request is seen as it reaches the app.
    if let Some(app) = &fx.app {
        let carried: Vec<String> = app
            .taken()
            .into_iter()
            .rev()
            .find_map(|served| match served.request.operation {
                Operation::AddToBoard(request) => Some(request),
                _ => None,
            })
            .expect("the request reached the app")
            .pictures
            .into_iter()
            .map(|picture| picture.file)
            .collect();
        assert_eq!(
            carried,
            vec![fresh.to_hex()],
            "one picture for two skeletons"
        );
    }

    // A relative path is taken from the folder the request names.
    let relative = fx
        .library
        .add_to_board(&AddToBoardRequest {
            board: plan.into(),
            elements: skeletons(r#"[{"type":"image","x":0,"y":600,"path":"harbour.png"}]"#),
            base_dir: Some(fx.dir.path().join("pictures")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(relative.pictures[0].source, "harbour.png");
    assert_eq!(relative.pictures[0].pixels, (40, 30));

    // What cannot be used is refused, with nothing written.
    let before = fx.dump();
    let html = fx.dir.path().join("pictures/page.svg");
    std::fs::write(&html, "<html><body><script>alert(1)</script></body></html>").unwrap();
    let error = add(&fx, plan, &image_at(&html, "")).unwrap_err();
    let (code, message) = said(error);
    assert_eq!(code, 2);
    assert!(
        message.starts_with("the element at index 0 (path): \u{201c}"),
        "{message}"
    );
    assert!(message.contains("page.svg"), "{message}");
    for (path, words) in [
        ("/nowhere/at/all.png", "there is no file"),
        ("https://example.com/a.png", "is not a file on this Mac"),
        ("relative.png", "relative path"),
    ] {
        let error = add(
            &fx,
            plan,
            &format!(r#"[{{"type":"image","x":0,"y":0,"path":"{path}"}}]"#),
        )
        .unwrap_err();
        let (code, message) = said(error);
        assert_eq!(code, 2, "{path}");
        assert!(message.contains(words), "{path}: {message}");
    }
    // A fill means nothing on a picture.
    let error = add(
        &fx,
        plan,
        &image_at(&png_path, r##","backgroundColor":"#ffffff""##),
    );
    assert_eq!(said(error.unwrap_err()).0, 2);
    assert_eq!(fx.dump(), before, "nothing was written");

    // A dry run reads the file, says what it would be, and stores nothing.
    let new_picture = fx.picture("pictures/new.png", (24, 24), [5, 200, 5]);
    let new_id = FileId::of(&std::fs::read(&new_picture).unwrap());
    let tried = fx
        .library
        .add_to_board(&AddToBoardRequest {
            board: plan.into(),
            elements: skeletons(&image_at(&new_picture, "")),
            write: dry(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(tried.pictures[0].pixels, (24, 24));
    assert_eq!(fx.store().file_bytes(&new_id).unwrap(), None);
    assert_eq!(fx.dump(), before);

    // Pictures a request came with are replaced by what the asked process reads.
    let other = innerword_image::fixtures::png(10, 10, [1, 1, 1]);
    let handed = fx
        .library
        .add_to_board(&AddToBoardRequest {
            board: plan.into(),
            elements: skeletons(&image_at(&new_picture, "")),
            pictures: vec![BoardPicture {
                source: new_picture.to_string_lossy().into_owned(),
                file: FileId::of(&other).to_hex(),
                mime_type: "image/png".into(),
                pixels: (10, 10),
                density: 72,
                untouched: true,
                data: other.clone(),
                stored: false,
            }],
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(handed.pictures[0].pixels, (24, 24));
    assert!(fx.store().file_bytes(&new_id).unwrap().is_some());
    assert_eq!(
        fx.store().file_bytes(&FileId::of(&other)).unwrap(),
        None,
        "the bytes the caller handed in were dropped"
    );

    // A whiteboard can be made with a picture on it.
    let made = fx
        .library
        .create_board(&CreateBoardRequest {
            title: "Photo".into(),
            elements: skeletons(&image_at(&png_path, "")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(made.pictures[0].mime_type, "image/png");
    assert_eq!(scene(&fx, made.board.id).stored_files()[0].as_str(), hex);
}

#[test]
fn the_pictures_of_one_request_weigh_32_mb_at_most() {
    let fx = fixture();
    let plan = board(&fx, None, "Plan", "[]").board.id;
    // Three pictures of 16 MB each, which the library keeps as they come.
    let mut elements = Vec::new();
    for (n, rgb) in [[10, 20, 30], [40, 50, 60], [70, 80, 90]]
        .iter()
        .enumerate()
    {
        let path = fx.dir.path().join(format!("big-{n}.png"));
        std::fs::write(&path, stored_png(4096, 1300, *rgb)).unwrap();
        elements.push(image_at(&path, "").trim_matches(['[', ']']).to_string());
    }
    let before = fx.dump();
    let error = add(&fx, plan, &format!("[{}]", elements.join(","))).unwrap_err();
    let (code, message) = said(error);
    assert_eq!(code, 2);
    assert!(message.contains("32 MB together at most"), "{message}");
    assert_eq!(fx.dump(), before);
    // Two of them are taken.
    let two = add(&fx, plan, &format!("[{}]", elements[..2].join(","))).unwrap();
    assert_eq!(two.pictures.len(), 2);
}

/// What the app's listener does with a whiteboard's request that brings pictures: it
/// stores them before the request is queued, so that the main thread writes the scene
/// only, and it refuses the request at once while the owner's switch is off.
#[test]
fn the_listener_stores_a_whiteboards_pictures_and_refuses_them_while_switched_off() {
    let fx = closed_fixture();
    let plan = board(&fx, None, "Plan", PLAN).board.id;
    let database = fx.library.database_path();
    let switch = Arc::new(Mutex::new(true));
    let seen: Arc<Mutex<Vec<(bool, bool)>>> = Arc::new(Mutex::new(Vec::new()));
    let app = super::live::StandIn::open_with(&fx, {
        let (switch, seen) = (switch.clone(), seen.clone());
        Box::new(move |store, mut incoming| {
            let carries = live::carries_page_files(incoming.request());
            if carries && !*switch.lock().unwrap() {
                seen.lock().unwrap().push((true, false));
                incoming.begin();
                incoming.answer(Answer::refused(Refusal::SwitchedOff)).ok();
                return None;
            }
            let mut files = innerword_store::ImageFile::beside(&database);
            if let Err(error) = live::store_page_files(incoming.request_mut(), &mut files) {
                incoming.answer(Answer::failed(&error)).ok();
                return None;
            }
            let stored = match incoming.request().operation() {
                Operation::AddToBoard(add) => add
                    .pictures
                    .iter()
                    .all(|picture| picture.stored && picture.data.is_empty()),
                _ => false,
            };
            seen.lock().unwrap().push((carries, stored));
            super::live::as_the_app(store, incoming)
        })
    });
    let png = fx.picture("listener.png", (32, 16), [100, 100, 0]);
    let id = FileId::of(&std::fs::read(&png).unwrap());
    let added = add(&fx, plan, &image_at(&png, "")).unwrap();
    assert!(added.written.app_is_open && added.written.changed);
    assert_eq!(seen.lock().unwrap().as_slice(), [(true, true)]);
    assert_eq!(
        fx.store().file_bytes(&id).unwrap(),
        Some(std::fs::read(&png).unwrap())
    );

    // With the switch off the request is refused before anything is stored.
    *switch.lock().unwrap() = false;
    let other = fx.picture("refused.png", (16, 32), [0, 100, 100]);
    let other_id = FileId::of(&std::fs::read(&other).unwrap());
    let before = fx.dump();
    let error = add(&fx, plan, &image_at(&other, "")).unwrap_err();
    assert_eq!(error.exit_code(), 5);
    assert!(matches!(
        error,
        Error::AppIsOpen {
            why: Refusal::SwitchedOff,
            ..
        }
    ));
    assert_eq!(fx.store().file_bytes(&other_id).unwrap(), None);
    assert_eq!(fx.dump(), before);
    drop(app);

    // Which requests carry pictures: a real create_board or add_to_board with bytes.
    let picture = BoardPicture {
        source: "/x.png".into(),
        file: id.to_hex(),
        mime_type: "image/png".into(),
        pixels: (32, 16),
        density: 72,
        untouched: true,
        data: std::fs::read(&png).unwrap(),
        stored: false,
    };
    let with = |operation: Operation| Request::new(operation, Duration::from_secs(1));
    let add_with = |pictures: Vec<BoardPicture>, write: WriteOptions| {
        with(Operation::AddToBoard(AddToBoardRequest {
            board: plan.into(),
            pictures,
            write,
            ..Default::default()
        }))
    };
    assert!(live::carries_page_files(&add_with(
        vec![picture.clone()],
        claude()
    )));
    assert!(live::carries_page_files(&with(Operation::CreateBoard(
        CreateBoardRequest {
            pictures: vec![picture.clone()],
            write: claude(),
            ..Default::default()
        }
    ))));
    assert!(!live::carries_page_files(&add_with(
        vec![picture.clone()],
        dry()
    )));
    assert!(!live::carries_page_files(&add_with(Vec::new(), claude())));
    assert!(!live::carries_page_files(&with(Operation::UpdateBoard(
        UpdateBoardRequest::default()
    ))));
    // The bytes travel as base64 and come back the same; `stored` never travels.
    let mut stored = picture.clone();
    stored.stored = true;
    let line = serde_json::to_string(&add_with(vec![stored], claude())).unwrap();
    assert!(!line.contains("stored"), "{line}");
    let back: Request = serde_json::from_str(&line).unwrap();
    let Operation::AddToBoard(back) = back.operation() else {
        panic!()
    };
    assert!(!back.pictures[0].stored);
    assert_eq!(back.pictures[0].data, picture.data);
    // A picture whose bytes are not its name, or not of the kind it says, is not stored.
    let mut files = innerword_store::ImageFile::beside(&fx.library.database_path());
    let mut damaged = picture.clone();
    damaged.data.push(0);
    let mut request = add_with(vec![damaged], claude());
    let error = live::store_page_files(&mut request, &mut files).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Invalid);
    let mut lying = picture.clone();
    lying.mime_type = "image/svg+xml".into();
    let mut request = add_with(vec![lying], claude());
    assert!(live::store_page_files(&mut request, &mut files).is_err());
    // Carried out without the listener, the same is refused where the scene is written.
    let mut lying = picture.clone();
    lying.mime_type = "image/gif".into();
    let mut store = Store::open(fx.library.database_path()).unwrap();
    let answer = live::execute(&mut store, &add_with(vec![lying], claude())).answer;
    assert!(matches!(answer, Answer::Failed { .. }), "{answer:?}");
}

/// A version of the tool that is not the app's knows nothing of whiteboards' requests: an
/// app of 0.10 (protocol 3) is sent one with a picture, and refuses it whole.
#[test]
fn an_app_of_the_protocol_before_whiteboards_refuses_a_whiteboards_request_whole() {
    let fx = closed_fixture();
    let plan = board(&fx, None, "Plan", PLAN).board.id;
    let png = fx.picture("old app.png", (12, 12), [1, 2, 3]);
    let before = fx.dump();
    let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let app = super::live::StandIn::open_with(&fx, {
        let lines = lines.clone();
        Box::new(move |_store, incoming| {
            lines
                .lock()
                .unwrap()
                .push(serde_json::to_string(incoming.request()).unwrap());
            incoming.begin();
            incoming
                .answer(Answer::refused(Refusal::Protocol { app: 3, tool: 4 }))
                .ok();
            None
        })
    });
    let error = add(&fx, plan, &image_at(&png, "")).unwrap_err();
    assert_eq!(error.exit_code(), 5);
    let message = error.to_string();
    assert!(
        message.contains("the app speaks version 3 of their channel, this tool version 4"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
    let line: serde_json::Value = serde_json::from_str(&lines.lock().unwrap()[0]).unwrap();
    assert_eq!(line["version"], 4);
    assert_eq!(line["op"], "add_to_board");
    use base64::Engine;
    assert_eq!(
        line["request"]["pictures"][0]["data"],
        base64::engine::general_purpose::STANDARD.encode(std::fs::read(&png).unwrap())
    );
    drop(app);
    assert_eq!(fx.dump(), before, "refused, not half applied");
}

/// Made directly and through the app, a whiteboard with pictures is the same library, row
/// by row (the derived values of section 7.1).
#[test]
fn the_same_request_makes_the_same_whiteboard_both_ways() {
    let direct = closed_fixture();
    let through = closed_fixture();
    let png = direct.picture("both.png", (20, 10), [9, 8, 7]);
    let app = super::live::StandIn::open(&through);
    for fx in [&direct, &through] {
        let made = board(fx, Some("Sketches"), "Plan", PLAN).board.id;
        add(
            fx,
            made,
            &format!(
                r#"[{{"type":"rectangle","x":0,"y":300,"label":{{"text":"Cache"}}}},
                    {{"type":"image","x":400,"y":300,"path":{}}}]"#,
                serde_json::to_string(&png.to_string_lossy()).unwrap()
            ),
        )
        .unwrap();
    }
    assert_eq!(direct.dump(), through.dump());
    drop(app);
}

#[test]
fn a_folder_with_whiteboards_and_pages_is_exported_without_them() {
    let fx = fixture();
    fx.create(Some("Mixed"), "# Text\n\nWords.\n");
    board(&fx, Some("Mixed"), "Plan", PLAN);
    let page = fx.page_folder(
        "page",
        &[(
            "index.html",
            "<!doctype html><title>Orbitarium</title><p>Hi.</p>",
        )],
    );
    fx.library
        .add_page(&innerword_agent::AddPageRequest {
            path: page.to_string_lossy().into_owned(),
            folder: Some(Target::path("Mixed")),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let out = fx.dir.path().join("mixed.md");
    let exported = fx
        .library
        .export(&ExportRequest {
            folder: Some(Target::path("Mixed")),
            path: out.clone(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        exported.warnings,
        [
            "1 page and 1 whiteboard were left out: a page is exported on its own as a \
             folder, and a whiteboard as a PDF."
        ]
    );
    assert_eq!(exported.documents.len(), 1);
}
