//! Comments: list, add on a quote, reply, resolve, reopen, delete.

use innerword_agent::{
    AddCommentRequest, CommentRequest, DeleteCommentRequest, EditRequest, Error, ErrorKind,
    InfoRequest, InsertRequest, ItemKind, ListCommentsRequest, ListVersionsRequest, ReplyRequest,
    Target, WriteOptions,
};
use innerword_core::CommentId;

use super::{claude, fixture};

const TEXT: &str = "# Review me\n\nThe ship left at dawn. The ship came back at dusk.\n\n## Later\n\nNobody saw the *ship* again.\n";

#[test]
fn comments_are_anchored_to_an_exact_quote() {
    let fx = fixture();
    let doc = fx.create(None, TEXT);
    let add = |quote: &str, occurrence: Option<usize>, write: WriteOptions| {
        fx.library.add_comment(&AddCommentRequest {
            document: doc.id.into(),
            quote: quote.into(),
            occurrence,
            body: format!("On {quote}"),
            write,
        })
    };
    fx.clock.set_ms(super::START + 5_000_000);
    let first = add("left at dawn", None, claude()).unwrap();
    assert!(first.written.changed);
    assert_eq!(first.snapshot, None, "adding a comment loses nothing");
    assert_eq!(first.reply, None);
    let thread = &first.thread;
    assert_eq!(thread.id, 1);
    assert_eq!(thread.author, "Claude");
    assert_eq!(thread.body, "On left at dawn");
    assert_eq!(thread.created_at.0, super::START + 5_000_000);
    assert!(!thread.resolved);
    assert_eq!(thread.anchored_text.as_deref(), Some("left at dawn"));
    assert_eq!(thread.heading.as_deref(), Some("Review me"));
    assert_eq!(thread.block, Some(1));
    assert!(thread.replies.is_empty());
    // It is in the document the way the app's comments panel reads it.
    let document = fx.document(doc.id);
    let stored = document.comment(CommentId(1)).unwrap();
    assert_eq!(
        (stored.author.as_str(), stored.body.as_str()),
        ("Claude", "On left at dawn")
    );
    assert_eq!(
        document.comment_ranges(CommentId(1)),
        [innerword_core::TextRange::in_block(1, 9, 21)]
    );
    // The text did not change, so neither did the title, the counts or the search index.
    assert_eq!(fx.markdown(doc.id), TEXT);
    assert_eq!(
        fx.store().document_meta(doc.id).unwrap().word_count,
        doc.words
    );

    // A quote that occurs several times needs its number.
    let twice = add("The ship", None, claude()).unwrap_err();
    assert_eq!(twice.kind(), ErrorKind::NotUnique);
    let Error::NotUnique { count, .. } = twice else {
        panic!()
    };
    assert_eq!(count, 2);
    let second = add("The ship", Some(2), claude()).unwrap();
    assert_eq!(second.thread.id, 2);
    assert_eq!(
        fx.document(doc.id).comment_ranges(CommentId(2)),
        [innerword_core::TextRange::in_block(1, 23, 31)]
    );
    assert!(matches!(
        add("The ship", Some(3), claude()),
        Err(Error::Invalid { .. })
    ));
    // A quote that is not there; one given as Markdown; one across blocks; no name.
    assert_eq!(
        add("the ship sank", None, claude()).unwrap_err().kind(),
        ErrorKind::NoMatch
    );
    let italic = add("the *ship* again", None, WriteOptions::default()).unwrap();
    assert_eq!(italic.thread.author, "Agent");
    assert_eq!(
        italic.thread.anchored_text.as_deref(),
        Some("the ship again")
    );
    assert_eq!(italic.thread.heading.as_deref(), Some("Later"));
    let across = add("at dusk.\nLater", None, claude()).unwrap();
    assert_eq!(
        across.thread.anchored_text.as_deref(),
        Some("at dusk.\nLater")
    );
    assert!(matches!(
        fx.library.add_comment(&AddCommentRequest {
            document: doc.id.into(),
            quote: "ship".into(),
            body: "  ".into(),
            write: claude(),
            ..Default::default()
        }),
        Err(Error::Invalid { .. })
    ));
    assert!(matches!(
        add("", None, claude()),
        Err(Error::Invalid { .. })
    ));

    // The list, in the order of the text.
    let list = fx
        .library
        .comments(&ListCommentsRequest {
            document: doc.id.into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(list.document.id, doc.id);
    let order: Vec<(u64, &str)> = list
        .threads
        .iter()
        .map(|t| (t.id, t.anchored_text.as_deref().unwrap()))
        .collect();
    assert_eq!(
        order,
        [
            (1, "left at dawn"),
            (2, "The ship"),
            (4, "at dusk.\nLater"),
            (3, "the ship again")
        ]
    );
    let json = serde_json::to_value(&list).unwrap();
    assert_eq!(json["threads"][0]["anchored_text"], "left at dawn");
    assert_eq!(json["threads"][0]["author"], "Claude");
    let info = fx
        .library
        .info(&InfoRequest {
            document: doc.id.into(),
        })
        .unwrap();
    assert_eq!((info.open_comments, info.resolved_comments), (4, 0));
}

#[test]
fn threads_are_answered_resolved_reopened_and_deleted() {
    let fx = fixture();
    let doc = fx.create(None, TEXT);
    let thread = fx
        .library
        .add_comment(&AddCommentRequest {
            document: Target::path("Review me"),
            quote: "dawn".into(),
            body: "Too early.".into(),
            write: WriteOptions::by("Ada"),
            ..Default::default()
        })
        .unwrap()
        .thread
        .id;
    let reply = |body: &str, by: &str| {
        fx.clock.advance_ms(1000);
        fx.library.reply(&ReplyRequest {
            document: doc.id.into(),
            comment: thread,
            body: body.into(),
            write: WriteOptions::by(by),
        })
    };
    let first = reply("It is in the outline.", "Ben").unwrap();
    assert_eq!(first.reply, Some(1));
    assert_eq!(first.thread.replies.len(), 1);
    assert_eq!(first.thread.replies[0].author, "Ben");
    assert_eq!(first.thread.replies[0].body, "It is in the outline.");
    let second = reply("Then keep it.", "Ada").unwrap();
    assert_eq!(second.reply, Some(2));
    assert!(second.thread.replies[1].created_at > second.thread.replies[0].created_at);
    assert!(matches!(reply(" ", "Ada"), Err(Error::Invalid { .. })));
    match fx.library.reply(&ReplyRequest {
        document: doc.id.into(),
        comment: 99,
        body: "x".into(),
        write: claude(),
    }) {
        Err(Error::NotFound {
            kind: ItemKind::Comment,
            hint: Some(hint),
            ..
        }) => {
            assert!(hint.contains("ids 1"), "{hint}");
        }
        other => panic!("{other:?}"),
    }

    let request = |comment: u64| CommentRequest {
        document: doc.id.into(),
        comment,
        write: claude(),
    };
    let resolved = fx.library.resolve_comment(&request(thread)).unwrap();
    assert!(resolved.thread.resolved && resolved.written.changed);
    assert_eq!(
        resolved.thread.anchored_text.as_deref(),
        Some("dawn"),
        "the anchor stays"
    );
    let again = fx.library.resolve_comment(&request(thread)).unwrap();
    assert!(again.thread.resolved && !again.written.changed);
    let open = fx
        .library
        .comments(&ListCommentsRequest {
            document: doc.id.into(),
            open_only: true,
        })
        .unwrap();
    assert!(open.threads.is_empty());
    let info = fx
        .library
        .info(&InfoRequest {
            document: doc.id.into(),
        })
        .unwrap();
    assert_eq!((info.open_comments, info.resolved_comments), (0, 1));
    let reopened = fx.library.reopen_comment(&request(thread)).unwrap();
    assert!(!reopened.thread.resolved && reopened.written.changed);
    assert!(
        !fx.library
            .reopen_comment(&request(thread))
            .unwrap()
            .written
            .changed
    );
    assert!(matches!(
        fx.library.resolve_comment(&request(7)),
        Err(Error::NotFound {
            kind: ItemKind::Comment,
            ..
        })
    ));

    // Deleting keeps a version first: it is the one thing here that loses something.
    let versions = || {
        fx.library
            .versions(&ListVersionsRequest {
                document: doc.id.into(),
            })
            .unwrap()
            .versions
    };
    assert!(versions().is_empty());
    let delete = |reply: Option<u64>, comment: u64| {
        fx.library.delete_comment(&DeleteCommentRequest {
            document: doc.id.into(),
            comment,
            reply,
            write: claude(),
        })
    };
    let one = delete(Some(1), thread).unwrap();
    assert_eq!(one.reply, Some(1));
    assert_eq!(one.thread.replies.len(), 1);
    assert_eq!(one.thread.replies[0].id, 2);
    assert_eq!(
        one.snapshot.as_ref().unwrap().label.as_deref(),
        Some("Before a comment was deleted by Claude")
    );
    assert!(matches!(
        delete(Some(1), thread),
        Err(Error::NotFound {
            kind: ItemKind::Reply,
            ..
        })
    ));
    let gone = delete(None, thread).unwrap();
    assert_eq!(gone.thread.id, thread, "the thread as it was");
    assert_eq!(gone.thread.body, "Too early.");
    assert_eq!(gone.thread.anchored_text.as_deref(), Some("dawn"));
    assert!(gone.snapshot.is_some());
    assert_eq!(versions().len(), 2);
    let document = fx.document(doc.id);
    assert_eq!(document.comment_count(), 0);
    assert!(
        document
            .blocks()
            .iter()
            .all(|block| block.comment_ids().is_empty())
    );
    assert_eq!(fx.markdown(doc.id), TEXT, "the text is not touched");
    assert!(matches!(
        delete(None, thread),
        Err(Error::NotFound {
            kind: ItemKind::Comment,
            hint: Some(_),
            ..
        })
    ));
}

#[test]
fn comments_stay_where_they_are_when_the_text_is_edited_elsewhere() {
    let fx = fixture();
    let doc = fx.create(None, TEXT);
    let add = |quote: &str| {
        fx.library
            .add_comment(&AddCommentRequest {
                document: doc.id.into(),
                quote: quote.into(),
                body: "?".into(),
                write: claude(),
                ..Default::default()
            })
            .unwrap()
            .thread
            .id
    };
    let dawn = add("left at dawn");
    let again = add("again");
    let anchored = || -> Vec<(u64, Option<String>, Option<usize>)> {
        fx.library
            .comments(&ListCommentsRequest {
                document: doc.id.into(),
                ..Default::default()
            })
            .unwrap()
            .threads
            .into_iter()
            .map(|thread| (thread.id, thread.anchored_text, thread.block))
            .collect()
    };
    let edit = |old: &str, new: &str| {
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
    // Before the anchor in the same paragraph, after it, in another block, in the title.
    edit("The ship left", "The old ship left");
    edit("came back at dusk", "returned by night");
    edit("Nobody saw", "No one ever saw");
    edit("Review me", "Reviewed");
    assert_eq!(
        anchored(),
        [
            (dawn, Some("left at dawn".into()), Some(1)),
            (again, Some("again".into()), Some(3)),
        ]
    );
    // New blocks above and between move the anchors along.
    for (position, heading) in [
        (innerword_agent::InsertPosition::Start, None),
        (
            innerword_agent::InsertPosition::BeforeHeading,
            Some("Later"),
        ),
    ] {
        fx.library
            .insert(&InsertRequest {
                document: doc.id.into(),
                markdown: "A new paragraph.\n\nAnd one more.".into(),
                position,
                heading: heading.map(String::from),
                write: claude(),
                ..Default::default()
            })
            .unwrap();
    }
    assert_eq!(
        anchored(),
        [
            (dawn, Some("left at dawn".into()), Some(3)),
            (again, Some("again".into()), Some(7)),
        ]
    );
    // An edit inside the anchored text keeps the thread on what is left of it.
    edit("at dawn", "before dawn");
    assert_eq!(
        anchored()[0],
        (dawn, Some("left before dawn".into()), Some(3))
    );
    // The ranges in the stored document are exactly the quoted words.
    let document = fx.document(doc.id);
    let range = document.comment_ranges(CommentId(again))[0];
    assert_eq!(document.text_in(range), "again");
}
