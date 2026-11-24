//! What a message carries: the quick chat's prompt, the side chat's context, the
//! outline of a long document, the carried-over conversation, and the system prompts.

use innerword_assistant::{
    AttachedPart, Attachment, DocumentPart, DrawingTarget, HistoryMessage, Mode, OpenDocument,
    OpenKind, QUICK_SYSTEM_PROMPT, Role, SELECTION_LIMIT, SIDE_SYSTEM_PROMPT, SideContext,
    WHOLE_DOCUMENT_LIMIT, bounded_history, outline, quick_prompt, side_prompt, side_system_prompt,
    system_prompt, with_history,
};

fn document(markdown: &str) -> OpenDocument {
    OpenDocument {
        id: Some(12),
        title: "Chapter 2".into(),
        path: "The Salt Road/Part one/Chapter 2".into(),
        markdown: markdown.into(),
        kind: OpenKind::Text,
    }
}

// ----- the quick chat -------------------------------------------------------------------

#[test]
fn with_nothing_selected_the_question_goes_clean() {
    assert_eq!(
        quick_prompt("A word for very tired?", None),
        "A word for very tired?"
    );
    assert_eq!(
        quick_prompt("  A word for very tired?\n", None),
        "A word for very tired?"
    );
    // A selection of nothing but white space is no selection.
    assert_eq!(quick_prompt("A word?", Some("  \n ")), "A word?");
    assert_eq!(quick_prompt("A word?", Some("")), "A word?");
}

#[test]
fn the_selection_is_marked_as_the_text_the_question_is_about() {
    let prompt = quick_prompt(
        "Make it shorter.",
        Some("The rain, which had been falling, fell."),
    );
    assert_eq!(
        prompt,
        "This is the text I have selected in my document. My request below is about this text.\n\n\
         <selected_text>\nThe rain, which had been falling, fell.\n</selected_text>\n\n\
         Make it shorter."
    );
}

#[test]
fn the_selection_goes_as_it_is_with_its_markdown_and_its_own_lines() {
    let selection = "## A heading\n\nA *line* with <angle> brackets & \"quotes\".\n\n- one\n- two";
    let prompt = quick_prompt("Tighten this.", Some(selection));
    assert!(prompt.contains(&format!("<selected_text>\n{selection}\n</selected_text>")));
    // Blank lines around the selection are not part of it; inner ones are.
    let padded = quick_prompt("x", Some("\n\nfirst\n\nsecond\n\n"));
    assert!(padded.contains("<selected_text>\nfirst\n\nsecond\n</selected_text>"));
}

#[test]
fn a_selection_too_long_to_send_is_cut_in_the_middle_and_says_so() {
    let selection = format!("START{}END", "x".repeat(SELECTION_LIMIT * 2));
    let prompt = quick_prompt("Summarise.", Some(&selection));
    assert!(prompt.len() < SELECTION_LIMIT + 1000);
    assert!(prompt.contains("START"));
    assert!(prompt.contains("END\n</selected_text>"));
    assert!(prompt.contains("characters in the middle are left out"));
    // A selection of exactly the limit is sent whole.
    let fits = "y".repeat(SELECTION_LIMIT);
    assert!(!quick_prompt("x", Some(&fits)).contains("left out"));
}

// ----- the side chat --------------------------------------------------------------------

#[test]
fn the_document_goes_whole_with_its_title_path_and_id() {
    let doc = document("# Chapter 2\n\nThe heron stood in the shallows.\n");
    let prompt = side_prompt(
        "Is the opening slow?",
        DocumentPart::Current(&doc),
        AttachedPart::None,
        true,
    );
    assert_eq!(
        prompt,
        "This is the document I have open, as it is now.\n\n\
         <open_document title=\"Chapter 2\" path=\"The Salt Road/Part one/Chapter 2\" id=\"doc:12\">\n\
         # Chapter 2\n\nThe heron stood in the shallows.\n\
         </open_document>\n\n\
         Is the opening slow?"
    );
}

#[test]
fn a_question_without_context_goes_clean() {
    assert_eq!(
        side_prompt(
            " What rhymes with dawn? ",
            DocumentPart::None,
            AttachedPart::None,
            true
        ),
        "What rhymes with dawn?"
    );
}

#[test]
fn an_unchanged_document_is_one_line_and_an_empty_one_says_so() {
    let doc = document("text");
    let prompt = side_prompt(
        "And now?",
        DocumentPart::Unchanged(&doc),
        AttachedPart::None,
        true,
    );
    assert_eq!(
        prompt,
        "(The open document, \"Chapter 2\", has not changed since my last message.)\n\nAnd now?"
    );
    let empty = document("  \n");
    let prompt = side_prompt(
        "Where do I begin?",
        DocumentPart::Current(&empty),
        AttachedPart::None,
        true,
    );
    assert!(prompt.contains("(The document is empty.)"));
}

#[test]
fn a_title_with_quotes_or_line_breaks_cannot_break_the_tag() {
    let doc = OpenDocument {
        id: None,
        title: "The \"Salt\"\nRoad".into(),
        path: String::new(),
        markdown: "x".into(),
        kind: OpenKind::Text,
    };
    let prompt = side_prompt("?", DocumentPart::Current(&doc), AttachedPart::None, true);
    // No id and no path: neither is invented.
    assert!(
        prompt.contains("<open_document title=\"The 'Salt' Road\">\nx\n</open_document>"),
        "{prompt}"
    );
}

#[test]
fn attached_items_are_listed_with_their_ids_and_paths() {
    let attached = [
        Attachment::document(14, "Characters", "The Salt Road/Notes/Characters"),
        Attachment::folder(3, "Part two", "The Salt Road/Part two"),
    ];
    let prompt = side_prompt(
        "Who is Mara's brother?",
        DocumentPart::None,
        AttachedPart::Current(&attached),
        true,
    );
    assert!(prompt.contains(
        "<attached>\n- document doc:14 \"The Salt Road/Notes/Characters\"\n- folder folder:3 \"The Salt Road/Part two\"\n</attached>"
    ));
    assert!(prompt.contains("read_document"), "it says how to read them");
    assert!(prompt.ends_with("Who is Mara's brother?"));
    // Without the tools it says that they cannot be read.
    let prompt = side_prompt(
        "?",
        DocumentPart::None,
        AttachedPart::Current(&attached),
        false,
    );
    assert!(prompt.contains("you cannot read them now"));
    // An empty list after there was one says that nothing is attached any more.
    let prompt = side_prompt("?", DocumentPart::None, AttachedPart::Current(&[]), true);
    assert_eq!(
        prompt,
        "(Nothing from the library is attached any more.)\n\n?"
    );
}

fn long_document() -> OpenDocument {
    let mut markdown = String::from("# The Salt Road\n\nA preface of a few words.\n\n");
    for chapter in 1..=30 {
        markdown.push_str(&format!("## Chapter {chapter}\n\n"));
        for _ in 0..700 {
            markdown.push_str("The tide went out and came in again. ");
        }
        markdown.push_str("\n\n");
    }
    assert!(markdown.chars().count() > WHOLE_DOCUMENT_LIMIT);
    document(&markdown)
}

#[test]
fn a_document_too_long_to_send_goes_as_an_outline_to_read_from_with_the_tools() {
    let doc = long_document();
    let prompt = side_prompt(
        "Where does the pace drop?",
        DocumentPart::Current(&doc),
        AttachedPart::None,
        true,
    );
    assert!(
        prompt.len() < 6000,
        "the outline is small: {}",
        prompt.len()
    );
    assert!(prompt.contains("too long to include whole"));
    assert!(prompt.contains("whole=\"false\""));
    assert!(prompt.contains("# The Salt Road (6 words)"));
    assert!(prompt.contains("## Chapter 1 (5600 words)"));
    assert!(prompt.contains("## Chapter 30 (5600 words)"));
    // It names the tool, the document and how to ask for a part.
    assert!(
        prompt.contains(
            "read_document tool (document \"doc:12\", heading: a heading of the outline)"
        )
    );
    assert!(prompt.contains("find_in_document"));
    assert!(
        !prompt.contains("The tide went out"),
        "none of the text itself"
    );
    assert!(prompt.ends_with("Where does the pace drop?"));
}

#[test]
fn without_tools_a_long_document_goes_as_outline_beginning_and_end() {
    let doc = long_document();
    let prompt = side_prompt("?", DocumentPart::Current(&doc), AttachedPart::None, false);
    assert!(prompt.contains("<outline>"));
    assert!(prompt.contains("<beginning>\n# The Salt Road"));
    assert!(prompt.contains("</end>"));
    assert!(prompt.contains("Say so when a question needs a part you cannot see"));
    assert!(prompt.chars().count() < WHOLE_DOCUMENT_LIMIT + 5000);
    // A long document that is not in the library yet cannot be read with a tool either.
    let mut unsaved = doc.clone();
    unsaved.id = None;
    let prompt = side_prompt(
        "?",
        DocumentPart::Current(&unsaved),
        AttachedPart::None,
        true,
    );
    assert!(prompt.contains("<beginning>"));
    // A long document without headings says that it has none.
    let flat = document(&"word ".repeat(WHOLE_DOCUMENT_LIMIT / 4));
    let prompt = side_prompt("?", DocumentPart::Current(&flat), AttachedPart::None, true);
    assert!(prompt.contains("(The document has no headings.)"));
}

#[test]
fn the_outline_reads_headings_and_counts_the_words_under_each() {
    let markdown = "Before any heading, four words.\n\n# One\n\nalpha beta\n\n## Two ##\n\n\
                    ```\n# not a heading, code\n```\n\n~~~~\n## nor this\n~~~~\n\n\
                    gamma\n\n#NoSpace is text\n\n   ### Three\n\n    # indented code\n\n####### seven is text\n#\n";
    let entries = outline(markdown);
    let read: Vec<(u8, &str, usize)> = entries
        .iter()
        .map(|e| (e.level, e.text.as_str(), e.words))
        .collect();
    assert_eq!(
        read,
        [
            (1, "One", 2),
            // The words in the code blocks and the lines that only look like headings
            // all stand under "Two" (the fence marks themselves are not words).
            (2, "Two", 12),
            (3, "Three", 8),
        ]
    );
    assert!(outline("no headings here").is_empty());
    assert!(outline("").is_empty());
}

#[test]
fn the_side_context_sends_the_document_again_only_when_it_changed() {
    let mut context = SideContext::new();
    let mut doc = document("first version");
    let first = context.compose("one", Some(&doc), &[], true);
    assert!(first.contains("first version"));
    let second = context.compose("two", Some(&doc), &[], true);
    assert!(second.contains("has not changed"));
    assert!(!second.contains("first version"));
    doc.markdown = "second version".into();
    assert!(
        context
            .compose("three", Some(&doc), &[], true)
            .contains("second version")
    );
    // A changed title or another document counts as a change.
    doc.title = "Chapter Two".into();
    assert!(
        context
            .compose("four", Some(&doc), &[], true)
            .contains("title=\"Chapter Two\"")
    );
    // After a reset (a new process of the agent) everything goes again.
    context.reset();
    assert!(
        context
            .compose("five", Some(&doc), &[], true)
            .contains("second version")
    );
    // No document: nothing is said about one, and the next one is sent whole.
    assert_eq!(context.compose("six", None, &[], true), "six");
    assert!(
        context
            .compose("seven", Some(&doc), &[], true)
            .contains("second version")
    );
}

#[test]
fn the_side_context_lists_the_attached_items_only_when_the_list_changed() {
    let mut context = SideContext::new();
    let notes = Attachment::document(14, "Characters", "Notes/Characters");
    let part = Attachment::folder(3, "Part two", "Part two");
    assert_eq!(
        context.compose("one", None, &[], true),
        "one",
        "nothing attached: nothing said"
    );
    let listed = context.compose("two", None, std::slice::from_ref(&notes), true);
    assert!(listed.contains("- document doc:14"));
    assert_eq!(
        context.compose("three", None, std::slice::from_ref(&notes), true),
        "three"
    );
    let both = context.compose("four", None, &[notes.clone(), part], true);
    assert!(both.contains("doc:14") && both.contains("folder:3"));
    let none = context.compose("five", None, &[], true);
    assert!(none.contains("Nothing from the library is attached any more"));
    assert_eq!(context.compose("six", None, &[], true), "six");
}

// ----- a conversation carried over ----------------------------------------------------

#[test]
fn a_carried_over_conversation_stands_before_the_next_message() {
    let history = [
        HistoryMessage {
            role: Role::User,
            text: "A name for a lighthouse keeper?".into(),
        },
        HistoryMessage {
            role: Role::Assistant,
            text: "Mara Venn.".into(),
        },
        HistoryMessage {
            role: Role::Assistant,
            text: "  ".into(),
        },
    ];
    let prompt = with_history(&history, "And her dog?");
    assert_eq!(
        prompt,
        "We are in the middle of a conversation. This is what was said so far; continue from it, \
         and do not repeat or summarise it.\n\n<conversation_so_far>\n\
         <me>\nA name for a lighthouse keeper?\n</me>\n<you>\nMara Venn.\n</you>\n\
         </conversation_so_far>\n\nMy next message:\n\nAnd her dog?"
    );
    assert_eq!(with_history(&[], "Just this."), "Just this.");
    assert_eq!(with_history(&history[2..], "Just this."), "Just this.");
}

#[test]
fn a_long_history_keeps_its_most_recent_part_and_begins_with_a_question() {
    let mut history = Vec::new();
    for turn in 0..50 {
        history.push(HistoryMessage {
            role: Role::User,
            text: format!("question {turn} {}", "q".repeat(90)),
        });
        history.push(HistoryMessage {
            role: Role::Assistant,
            text: format!("answer {turn} {}", "a".repeat(90)),
        });
    }
    let kept = bounded_history(&history, 1000);
    assert!(kept.len() < 12 && kept.len() >= 8, "{}", kept.len());
    assert_eq!(kept.first().unwrap().role, Role::User);
    assert!(kept.last().unwrap().text.starts_with("answer 49"));
    assert!(kept.iter().map(|m| m.text.len()).sum::<usize>() <= 1000);
    // The last message always stays, however long.
    let one = [HistoryMessage {
        role: Role::User,
        text: "x".repeat(5000),
    }];
    assert_eq!(bounded_history(&one, 100).len(), 1);
    assert_eq!(bounded_history(&history, usize::MAX).len(), 100);
}

// ----- the system prompts ---------------------------------------------------------------

#[test]
fn the_quick_prompt_is_a_concise_helper() {
    let prompt = QUICK_SYSTEM_PROMPT;
    assert_eq!(system_prompt(Mode::Quick), prompt);
    for says in [
        "<selected_text>",
        "answer with the new text and nothing else",
        "at most five",
        "Format: Markdown",
        "answer in the language of the user's request",
        "you cannot change it",
    ] {
        assert!(
            prompt.contains(says),
            "the quick prompt does not say: {says}"
        );
    }
    assert!(prompt.len() < 3500, "concise: {}", prompt.len());
}

#[test]
fn the_side_prompt_is_a_collaborator_who_reads_and_advises() {
    let prompt = SIDE_SYSTEM_PROMPT;
    assert_eq!(system_prompt(Mode::Side), prompt);
    for says in [
        "<open_document>",
        "<attached>",
        "You advise; the user writes",
        "You cannot change the document",
        "Format: Markdown",
        "answer in the language of the user's message",
        "Never invent what a document says",
    ] {
        assert!(
            prompt.contains(says),
            "the side prompt does not say: {says}"
        );
    }
    // Every tool it names is one the assistant really gets, and it names no tool
    // that writes.
    let read_only = innerword_assistant::read_only_tools();
    for tool in [
        "list_library",
        "search_library",
        "read_document",
        "find_in_document",
        "document_info",
        "folder_info",
        "recent_documents",
        "list_comments",
        "list_versions",
        "read_version",
    ] {
        assert!(prompt.contains(tool));
        assert!(
            read_only.iter().any(|known| known == tool),
            "{tool} is not a read-only tool of the server"
        );
    }
    // With every target's drawing section too: a drawing is text in an answer, never a
    // tool, and no section names a tool that writes.
    for target in [
        DrawingTarget::None,
        DrawingTarget::OpenWhiteboard,
        DrawingTarget::NewWhiteboard,
        DrawingTarget::NewWhiteboardBesidePage,
    ] {
        for tools in [true, false] {
            let prompt = side_system_prompt(tools, target);
            for tool in innerword_assistant::withheld_tools() {
                assert!(
                    !prompt.contains(&tool),
                    "the prompt of {target:?} names {tool}, which the assistant does not have"
                );
            }
        }
    }
    // Without tools it says so.
    assert_eq!(side_system_prompt(true, DrawingTarget::None), prompt);
    assert!(side_system_prompt(false, DrawingTarget::None).contains("you have no tools"));
}

#[test]
fn both_prompts_tell_the_assistant_to_ignore_the_programs_environment_note() {
    for prompt in [QUICK_SYSTEM_PROMPT, SIDE_SYSTEM_PROMPT] {
        assert!(prompt.contains("you are not a coding assistant"));
        assert!(prompt.contains("Ignore it and never mention it"));
        assert!(
            !prompt.contains("Claude"),
            "nothing in a prompt assumes the agent"
        );
    }
}

fn page(text: &str) -> OpenDocument {
    OpenDocument {
        id: Some(41),
        title: "Reading Orbitarium".into(),
        path: "Claude Pages/Reading Orbitarium".into(),
        markdown: text.into(),
        kind: OpenKind::Page {
            source_url: Some("https://claude.ai/artifact/ExampleArtifactId00001".into()),
        },
    }
}

/// A page is sent with the sentence that says what it is, its text in place of the
/// Markdown, its address, and the id the tools read it by.
#[test]
fn an_open_page_is_sent_as_a_page_with_its_text() {
    let open = page("Orbitarium\nThe sky at night, chapter by chapter.");
    let prompt = side_prompt(
        "What is chapter one about?",
        DocumentPart::Current(&open),
        AttachedPart::None,
        true,
    );
    assert!(
        prompt.starts_with(
            "The open item is a page: a web page saved in the library, shown as it was made. \
             It cannot be changed. Its text, as the page showed it:\n\n"
        ),
        "{prompt}"
    );
    assert!(prompt.contains(innerword_assistant::PAGE_PREFACE));
    assert!(
        prompt.contains(
            "<open_document title=\"Reading Orbitarium\" path=\"Claude Pages/Reading \
             Orbitarium\" id=\"doc:41\" kind=\"page\" \
             source=\"https://claude.ai/artifact/ExampleArtifactId00001\">\n\
             Orbitarium\nThe sky at night, chapter by chapter.\n</open_document>"
        ),
        "{prompt}"
    );
    assert!(prompt.ends_with("What is chapter one about?"));
    assert!(!prompt.contains("This is the document I have open"));
    // The reference stays doc:<id>, which the tools read for a page too.
    assert_eq!(open.reference().as_deref(), Some("doc:41"));

    // Without an address the tag has none; an empty page says so.
    let mut bare = page("");
    bare.kind = OpenKind::Page { source_url: None };
    let prompt = side_prompt("?", DocumentPart::Current(&bare), AttachedPart::None, false);
    assert!(
        prompt.contains("kind=\"page\">\n(The page shows no text.)\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("source="), "{prompt}");
}

/// The page is not sent again while it is the same, and the line says "page".
#[test]
fn an_unchanged_page_is_named_as_a_page() {
    let mut context = SideContext::new();
    let open = page("The sky at night.");
    let first = context.compose("One?", Some(&open), &[], true);
    assert!(first.contains("The sky at night."));
    let second = context.compose("Two?", Some(&open), &[], true);
    assert_eq!(
        second,
        "(The open page, \"Reading Orbitarium\", has not changed since my last message.)\n\nTwo?"
    );
    // The same id as a text is another item: sent again.
    let mut as_text = open.clone();
    as_text.kind = OpenKind::Text;
    let third = context.compose("Three?", Some(&as_text), &[], true);
    assert!(
        third.contains("This is the document I have open"),
        "{third}"
    );
}

/// A page too long to send whole goes as its beginning and its end, and with the tools
/// the assistant is told to read the rest of it by its id.
#[test]
fn a_long_page_goes_as_its_beginning_and_its_end() {
    let text = "The tide went out and came in again. ".repeat(6_000);
    assert!(text.chars().count() > WHOLE_DOCUMENT_LIMIT);
    let open = page(&text);
    let with_tools = side_prompt("?", DocumentPart::Current(&open), AttachedPart::None, true);
    assert!(with_tools.starts_with(innerword_assistant::PAGE_PREFACE));
    assert!(with_tools.contains("whole=\"false\""));
    assert!(with_tools.contains("<beginning>\nThe tide went out"));
    assert!(with_tools.contains("</end>\n</open_document>"));
    assert!(
        with_tools.contains("Read the rest with the read_document tool (document \"doc:41\")"),
        "{with_tools}"
    );
    assert!(with_tools.chars().count() < WHOLE_DOCUMENT_LIMIT + 2_000);
    let without = side_prompt("?", DocumentPart::Current(&open), AttachedPart::None, false);
    assert!(without.contains("Say so when a question needs a part you cannot see."));
    assert!(!without.contains("read_document"));
}

#[test]
fn a_text_is_the_default_kind() {
    assert_eq!(OpenKind::default(), OpenKind::Text);
    assert!(!OpenKind::Text.is_page());
    assert!(OpenKind::Page { source_url: None }.is_page());
    assert!(OpenKind::Text.is_text());
    assert!(!OpenKind::Board.is_text());
    assert!(!OpenKind::Board.is_page());
    assert!(OpenKind::Board.is_board());
    assert!(!OpenKind::Page { source_url: None }.is_board());
}

fn board(listing: &str) -> OpenDocument {
    OpenDocument {
        id: Some(57),
        title: "Architecture".into(),
        path: "Plans/Architecture".into(),
        markdown: listing.into(),
        kind: OpenKind::Board,
    }
}

/// A whiteboard is sent with the sentence that says what it is, the listing of its
/// elements in place of the Markdown, and its own id, `board:<id>`.
#[test]
fn an_open_whiteboard_is_sent_as_the_listing_of_its_elements() {
    let listing = "rectangle api at (100, 80) 200 x 90 \"API\"\n\
                   ellipse db at (420, 80) 160 x 90 \"Postgres\"";
    let open = board(listing);
    let prompt = side_prompt(
        "What does the API read from?",
        DocumentPart::Current(&open),
        AttachedPart::None,
        true,
    );
    assert!(
        prompt.starts_with(
            "The open item is a whiteboard: a drawing of shapes, arrows, text and pictures. \
             It is changed only by its owner, in the app. Its elements, one per line, with \
             their ids, positions, sizes and text:\n\n"
        ),
        "{prompt}"
    );
    assert!(prompt.starts_with(innerword_assistant::BOARD_PREFACE));
    assert!(
        prompt.contains(&format!(
            "<open_document title=\"Architecture\" path=\"Plans/Architecture\" \
             id=\"board:57\" kind=\"board\">\n{listing}\n</open_document>"
        )),
        "{prompt}"
    );
    assert!(prompt.ends_with("What does the API read from?"));
    assert!(!prompt.contains("This is the document I have open"));
    assert!(!prompt.contains(innerword_assistant::PAGE_PREFACE));
    assert!(!prompt.contains("source="));
    assert_eq!(open.reference().as_deref(), Some("board:57"));

    // An empty whiteboard says so.
    let prompt = side_prompt(
        "?",
        DocumentPart::Current(&board("")),
        AttachedPart::None,
        false,
    );
    assert!(
        prompt.contains("kind=\"board\">\n(The whiteboard is empty.)\n"),
        "{prompt}"
    );
}

/// The whiteboard is not sent again while it is the same, and the line says
/// "whiteboard"; the same id as a page is another item.
#[test]
fn an_unchanged_whiteboard_is_named_as_a_whiteboard() {
    let mut context = SideContext::new();
    let open = board("text title at (100, 20) \"System overview\"");
    let first = context.compose("One?", Some(&open), &[], true);
    assert!(first.contains("System overview"));
    let second = context.compose("Two?", Some(&open), &[], true);
    assert_eq!(
        second,
        "(The open whiteboard, \"Architecture\", has not changed since my last message.)\n\nTwo?"
    );
    let mut as_page = open.clone();
    as_page.kind = OpenKind::Page { source_url: None };
    let third = context.compose("Three?", Some(&as_page), &[], true);
    assert!(
        third.starts_with(innerword_assistant::PAGE_PREFACE),
        "{third}"
    );
}

/// A listing too long to send whole goes as its beginning and its end, as a page's text
/// does, and with the tools the assistant is told to read the rest by `board:<id>`.
#[test]
fn a_long_whiteboard_goes_as_its_beginning_and_its_end() {
    let listing = "rectangle box at (0, 0) 100 x 50 \"A step of the process\"\n".repeat(4_000);
    assert!(listing.chars().count() > WHOLE_DOCUMENT_LIMIT);
    let open = board(&listing);
    let with_tools = side_prompt("?", DocumentPart::Current(&open), AttachedPart::None, true);
    assert!(with_tools.starts_with(innerword_assistant::BOARD_PREFACE));
    assert!(with_tools.contains("kind=\"board\""));
    assert!(with_tools.contains("whole=\"false\""));
    assert!(with_tools.contains("<beginning>\nrectangle box"));
    assert!(with_tools.contains("</end>\n</open_document>"));
    assert!(
        with_tools.contains("Read the rest with the read_document tool (document \"board:57\")"),
        "{with_tools}"
    );
    assert!(with_tools.chars().count() < WHOLE_DOCUMENT_LIMIT + 2_000);
    let without = side_prompt("?", DocumentPart::Current(&open), AttachedPart::None, false);
    assert!(without.contains("Say so when a question needs a part you cannot see."));
    assert!(!without.contains("read_document"));
}
