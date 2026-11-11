//! Unit tests of the pure parts (parsers, formatting, stored forms). They
//! live here rather than in the lib so the crate has a single test binary.

mod automation {
    use gpui::Keystroke;
    use std::time::Duration;
    use writer_app::automation::*;
    use writer_ui::ThemeMode;

    #[test]
    fn parses_example_script() {
        let script = Script::parse(
            "wait:500; keys:cmd-n; type:Hello world; keys:cmd-b; screenshot:/tmp/x.png; quit",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Wait(Duration::from_millis(500)),
                Step::Keys(vec![Keystroke::parse("cmd-n").unwrap()]),
                Step::Type("Hello world".into()),
                Step::Keys(vec![Keystroke::parse("cmd-b").unwrap()]),
                Step::Screenshot("/tmp/x.png".into()),
                Step::Quit,
            ]
        );
    }

    #[test]
    fn multi_line_comments_and_escapes() {
        let script = Script::parse(
            "# a comment\nkeys: cmd-shift-p escape\ntype:a\\;b\\nc\\s\nresize:800x600\ntheme:dark\naction:app::Quit\n\n",
        )
        .unwrap();
        assert_eq!(
            script.steps,
            vec![
                Step::Keys(vec![
                    Keystroke::parse("cmd-shift-p").unwrap(),
                    Keystroke::parse("escape").unwrap()
                ]),
                Step::Type("a;b\nc ".into()),
                Step::Resize(800., 600.),
                Step::Theme(ThemeMode::Dark),
                Step::Action("app::Quit".into()),
            ]
        );
    }

    #[test]
    fn rejects_bad_steps() {
        assert!(Script::parse("wait:soon").is_err());
        assert!(Script::parse("keys:cmd-opt-s").is_err());
        assert!(Script::parse("dance").is_err());
        assert!(Script::parse("theme:blue").is_err());
        assert_eq!(
            Script::parse("state; dump; info").unwrap().steps,
            vec![Step::State, Step::Dump, Step::Info]
        );
    }

    #[test]
    fn measuring_steps() {
        assert_eq!(
            Script::parse("book:10000; savecost; savecost:3; limitcost; limitcost:7")
                .unwrap()
                .steps,
            vec![
                Step::Book(10_000),
                Step::SaveCost(5),
                Step::SaveCost(3),
                Step::LimitCost(5),
                Step::LimitCost(7),
            ]
        );
        assert!(Script::parse("book").is_err());
        assert!(Script::parse("book:many").is_err());
        assert!(Script::parse("savecost:often").is_err());
    }

    #[test]
    fn the_generated_book_is_a_valid_document() {
        let book = writer_app::welcome::book_document(400);
        assert_eq!(book.block_count(), 400);
        book.check_invariants().unwrap();
        assert_eq!(book.title().as_deref(), Some("Chapter 1"));
        assert!(book.stats().words > 400 * 20);
        // The same every time, so measurements can be compared.
        assert_eq!(
            book.plain_text(),
            writer_app::welcome::book_document(400).plain_text()
        );
        // The save and the count can be measured on any open document.
        assert_eq!(writer_app::welcome::book_document(0).block_count(), 1);
    }

    #[test]
    fn keystrokes_for_characters() {
        let upper = keystroke_for_char('H');
        assert!(upper.modifiers.shift);
        assert_eq!(upper.key, "h");
        assert_eq!(upper.key_char.as_deref(), Some("H"));
        assert_eq!(keystroke_for_char(' ').key, "space");
        assert_eq!(keystroke_for_char('\n').key, "enter");
        assert_eq!(keystroke_for_char('ö').key_char.as_deref(), Some("ö"));
    }
}

mod settings {
    use writer_app::settings::*;
    use writer_core::PaperSize;
    use writer_export::SocialNetwork;

    #[test]
    fn character_limit_names_round_trip() {
        assert_eq!(parse_character_limit("off"), None);
        assert_eq!(parse_character_limit("nonsense"), None);
        for network in SocialNetwork::ALL {
            assert_eq!(
                parse_character_limit(character_limit_name(Some(network))),
                Some(network)
            );
        }
    }

    #[test]
    fn paper_names_round_trip() {
        for paper in Paper::ALL {
            assert_eq!(Paper::parse(paper.as_str()), paper);
        }
        assert_eq!(Paper::parse("b5"), Paper::A4);
    }

    #[test]
    fn new_document_settings_follow_the_defaults() {
        let settings = Settings {
            new_document_font: "Georgia".into(),
            new_document_size: 19.,
            new_document_line_height: 1.4,
            new_document_page_view: true,
            new_document_paper: Paper::Letter,
            ..Settings::default()
        };
        let document = settings.new_document_settings();
        assert_eq!(document.typography.body_font, "Georgia");
        assert_eq!(document.typography.base_size, 19.);
        assert_eq!(document.typography.line_height, 1.4);
        assert!(document.page_view);
        assert_eq!(document.page.paper, PaperSize::UsLetter);
    }
}

mod document_io {
    use writer_app::document_io::*;
    use writer_core::{Block, BlockKind, Document};

    #[test]
    fn stored_form_derives_title_text_and_counts() {
        let document = Document::from_blocks(vec![
            Block::paragraph("An opening line"),
            Block::with_text(BlockKind::heading(2), "The real title"),
            Block::paragraph("Two words"),
        ]);
        let form = StoredForm::of(&document, None);
        assert_eq!(form.title, "The real title");
        assert_eq!(form.words, 8);
        assert!(form.plain_text.contains("Two words"));
        assert_eq!(Document::from_json(&form.json).unwrap(), document);
        assert_eq!(StoredForm::of(&document, Some("Mine")).title, "Mine");
    }

    #[test]
    fn empty_documents_are_untitled() {
        assert_eq!(StoredForm::of(&Document::new(), None).title, UNTITLED);
        let blank = Document::from_blocks(vec![Block::paragraph("   ")]);
        assert_eq!(derived_title(&blank), UNTITLED);
    }

    #[test]
    fn loading_is_lenient() {
        assert!(load_document("").recovered_from.is_none());
        let good = Document::from_blocks(vec![Block::paragraph("Hello")]);
        let loaded = load_document(&good.to_json());
        assert!(loaded.recovered_from.is_none());
        assert_eq!(loaded.document, good);

        let loaded = load_document("not json at all\nsecond line");
        assert!(loaded.recovered_from.is_some());
        assert_eq!(loaded.document.plain_text(), "not json at all\nsecond line");
    }
}

mod welcome {
    use writer_app::welcome::*;
    use writer_core::BlockKind;

    #[test]
    fn rich_markup_becomes_spans() {
        let block = rich(BlockKind::Paragraph, "Press {k:⌘P} to {b:open} a {doc}.");
        assert_eq!(block.text(), "Press ⌘P to open a {doc}.");
        let spans: Vec<(String, bool, bool)> = block
            .text_spans()
            .map(|(_, text, attrs)| (text.to_string(), attrs.code, attrs.bold))
            .collect();
        assert_eq!(
            spans,
            vec![
                ("Press ".to_string(), false, false),
                ("⌘P".to_string(), true, false),
                (" to ".to_string(), false, false),
                ("open".to_string(), false, true),
                (" a {doc}.".to_string(), false, false),
            ]
        );
    }

    #[test]
    fn welcome_document_is_valid_and_titled() {
        let document = welcome_document();
        document.check_invariants().unwrap();
        assert_eq!(document.title().as_deref(), Some("Welcome to The Writer"));
        let text = document.plain_text();
        for topic in [
            "command palette",
            "format menu",
            "insert menu",
            "Markdown",
            "page view",
            "comment",
            "Vim mode",
            "exports",
        ] {
            assert!(text.contains(topic), "the welcome text mentions {topic}");
        }
        // Real formatting, not literal markup.
        assert!(!text.contains("{k:"));
        assert!(
            document
                .blocks()
                .iter()
                .any(|block| { block.text_spans().any(|(_, _, attrs)| attrs.code) })
        );
    }
}

mod session {
    use writer_app::workspace::first_occurrence;
    use writer_core::{Block, Document, Position};

    #[test]
    fn first_occurrence_prefers_the_whole_query() {
        let document = Document::from_blocks(vec![
            Block::paragraph("The harbour was quiet."),
            Block::paragraph("A quiet harbour, then."),
        ]);
        let range = first_occurrence(&document, "quiet harbour").unwrap();
        assert_eq!(range.start, Position::new(1, 2));
        // No exact phrase: the earliest of the words.
        let range = first_occurrence(&document, "then harbour").unwrap();
        assert_eq!(range.start, Position::new(0, 4));
        assert_eq!(document.text_in(range), "harbour");
        assert!(first_occurrence(&document, "  ").is_none());
        assert!(first_occurrence(&document, "ferry").is_none());
        assert!(first_occurrence(&document, "\"Quiet Harbour\"").is_some());
    }
}

mod render {
    use writer_app::workspace::{counted, group_digits};

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(1000), "1,000");
        assert_eq!(group_digits(1234567), "1,234,567");
        assert_eq!(counted(1, "word", "words"), "1 word");
        assert_eq!(counted(2400, "word", "words"), "2,400 words");
    }
}

mod shortcuts {
    use writer_app::shortcuts::*;

    #[test]
    fn action_names_read_as_phrases() {
        assert_eq!(
            humanize_action_name("editor::MoveWordLeft"),
            "Move word left"
        );
        assert_eq!(humanize_action_name("editor::SetHeading1"), "Set heading 1");
        assert_eq!(humanize_action_name("Backspace"), "Backspace");
        assert_eq!(
            humanize_action_name("editor::SelectToDocumentEnd"),
            "Select to document end"
        );
    }
}

mod export {
    use std::path::PathBuf;
    use writer_app::export::*;
    use writer_export::ExportFormat;

    #[test]
    fn extension_is_enforced() {
        let f = |name: &str, format| {
            with_extension(PathBuf::from(name), format)
                .to_string_lossy()
                .to_string()
        };
        assert_eq!(f("/tmp/a.md", ExportFormat::Markdown), "/tmp/a.md");
        assert_eq!(f("/tmp/a.MD", ExportFormat::Markdown), "/tmp/a.MD");
        assert_eq!(f("/tmp/a", ExportFormat::Docx), "/tmp/a.docx");
        assert_eq!(f("/tmp/a.txt", ExportFormat::Docx), "/tmp/a.txt.docx");
        assert_eq!(
            f("/tmp/v1.2 notes", ExportFormat::PlainText),
            "/tmp/v1.2 notes.txt"
        );
    }

    #[test]
    fn choices_cover_the_brief() {
        let labels: Vec<&str> = ExportChoice::ALL
            .iter()
            .filter(|choice| choice.scope == ExportScope::Document)
            .map(ExportChoice::label)
            .collect();
        assert_eq!(
            labels,
            [
                "Markdown file",
                "Word document",
                "Plain text file",
                "Copy as Markdown",
                "Copy as plain text"
            ]
        );
        assert_eq!(
            ExportChoice::ALL
                .iter()
                .filter(|choice| choice.scope == ExportScope::Folder)
                .count(),
            3
        );
    }
}

mod window {
    use gpui::{Bounds, Pixels, point, px, size};
    use writer_app::window::*;

    fn display() -> Vec<Bounds<Pixels>> {
        vec![Bounds {
            origin: point(px(0.), px(0.)),
            size: size(px(1512.), px(982.)),
        }]
    }

    #[test]
    fn bounds_round_trip_through_json() {
        let stored = StoredBounds {
            x: 120.,
            y: 80.,
            width: 1100.,
            height: 760.,
            mode: StoredWindowMode::Maximized,
        };
        let json = serde_json::to_string(&stored).unwrap();
        assert!(json.contains("\"maximized\""));
        let back: StoredBounds = serde_json::from_str(&json).unwrap();
        assert_eq!(back, stored);
        assert_eq!(
            StoredBounds::from_window_bounds(back.to_window_bounds()),
            stored
        );
        // An older value without the mode is a plain window.
        let old: StoredBounds =
            serde_json::from_str(r#"{"x":1,"y":2,"width":800,"height":600}"#).unwrap();
        assert_eq!(old.mode, StoredWindowMode::Windowed);
    }

    #[test]
    fn unusable_bounds_are_rejected() {
        let good = StoredBounds {
            x: 100.,
            y: 60.,
            width: 1000.,
            height: 700.,
            mode: StoredWindowMode::Windowed,
        };
        assert!(good.is_usable(&display()));
        // No display information: trust the numbers.
        assert!(good.is_usable(&[]));
        // Too small.
        assert!(
            !StoredBounds {
                width: 300.,
                ..good
            }
            .is_usable(&display())
        );
        // On a display that is no longer there.
        assert!(!StoredBounds { x: 4000., ..good }.is_usable(&display()));
        assert!(!StoredBounds { y: -900., ..good }.is_usable(&display()));
        // Garbage.
        assert!(
            !StoredBounds {
                x: f32::NAN,
                ..good
            }
            .is_usable(&display())
        );
        // Partly off the left edge but still grabbable.
        assert!(StoredBounds { x: -300., ..good }.is_usable(&display()));
    }
}

mod character_limit {
    use writer_app::character_limit::LimitCounter;
    use writer_core::{Block, BlockKind, Document, EditorState, Position};
    use writer_export::SocialNetwork;

    fn document(lines: &[&str]) -> Document {
        Document::from_blocks(lines.iter().map(|line| Block::paragraph(line)).collect())
    }

    /// The counter and a count of the whole text must agree.
    fn check(counter: &mut LimitCounter, document: &Document) -> usize {
        let whole = counter.network().count(&document.plain_text());
        let counted = counter.count(document);
        assert_eq!(
            counted,
            whole,
            "{:?} on {:?}",
            counter.network(),
            document.plain_text()
        );
        counted.length
    }

    #[test]
    fn the_sum_over_blocks_is_the_count_of_the_whole_text() {
        let texts: [&[&str]; 10] = [
            &[""],
            &["", "  ", ""],
            &["Hello"],
            &["  Hello  "],
            &[
                "",
                "  First line. ",
                "",
                "\tSecond 👍🏽 line  ",
                "   ",
                "Third",
                "",
                " ",
            ],
            &[
                "Launch day! https://example.com/a/very/long/path",
                "see https://x.y.",
            ],
            &[
                "https://example.com",
                "nothttp://x.y",
                "HTTP://X.Y and http://z.w  ",
            ],
            &["日本語 👨‍👩‍👧‍👦 🇦🇱🇺🇸", "café “quoted” — dash", "1️⃣ e\u{301}"],
            &["soft\nbreak inside", " \n ", "end\n"],
            &["\u{3000}ideographic space\u{3000}", "tail\u{a0}"],
        ];
        for network in SocialNetwork::ALL {
            for lines in texts {
                let mut counter = LimitCounter::new(network);
                check(&mut counter, &document(lines));
            }
        }
    }

    #[test]
    fn odd_text_falls_back_to_the_whole_count() {
        // A combining mark right after leading white space joins the space
        // in the whole text; a carriage return would join the line break.
        let texts: [&[&str]; 3] = [
            &["first", " \u{301}accent after a space", "last"],
            &["first", "ends with a return\r", "last"],
            &["\u{600} ", "prepend before a space \u{600} ", "x"],
        ];
        for network in SocialNetwork::ALL {
            for lines in texts {
                let mut counter = LimitCounter::new(network);
                check(&mut counter, &document(lines));
            }
        }
    }

    #[test]
    fn an_edit_recounts_only_the_blocks_it_changed() {
        let mut blocks = vec![Block::with_text(BlockKind::heading(1), "A long post")];
        blocks.extend((0..500).map(|ix| Block::paragraph(&format!("Paragraph {ix} of the text."))));
        let mut state = EditorState::new(Document::from_blocks(blocks));
        for network in [
            SocialNetwork::X,
            SocialNetwork::Mastodon,
            SocialNetwork::LinkedIn,
        ] {
            let mut counter = LimitCounter::new(network);
            let before = check(&mut counter, state.document());
            assert_eq!(counter.recounted(), 501);
            // The same document again: nothing to count.
            check(&mut counter, state.document());
            assert_eq!(counter.recounted(), 0);

            // Typing in the middle.
            state.set_caret(Position::new(250, 3));
            state.insert_text("xyz");
            assert_eq!(check(&mut counter, state.document()), before + 3);
            assert_eq!(counter.recounted(), 1);
            // A new paragraph: the split block and the new one.
            state.insert_paragraph_break();
            assert_eq!(check(&mut counter, state.document()), before + 4);
            assert!(counter.recounted() <= 2, "{}", counter.recounted());
            // Joining them again, then undoing everything.
            state.backspace();
            assert_eq!(check(&mut counter, state.document()), before + 3);
            assert!(counter.recounted() <= 2, "{}", counter.recounted());
            while state.undo() {}
            assert_eq!(check(&mut counter, state.document()), before);
            assert!(counter.recounted() <= 2, "{}", counter.recounted());
            // At the very ends, where trimming applies.
            state.set_caret(Position::new(0, 0));
            state.insert_text("   ");
            assert_eq!(check(&mut counter, state.document()), before);
            let end = state.document().end();
            state.set_caret(end);
            state.insert_text(" \u{1f44d}  ");
            check(&mut counter, state.document());
            assert_eq!(counter.recounted(), 1);
            while state.undo() {}
        }
    }

    #[test]
    fn deleting_and_inserting_many_blocks_keeps_the_count_right() {
        let lines: Vec<String> = (0..60)
            .map(|ix| format!(" line {ix} https://a.b/{ix} "))
            .collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut state = EditorState::new(document(&lines));
        let mut counter = LimitCounter::new(SocialNetwork::X);
        check(&mut counter, state.document());
        // Delete a stretch across blocks.
        state.set_selection(writer_core::Selection::new(
            Position::new(10, 2),
            Position::new(40, 4),
        ));
        state.delete_selection();
        check(&mut counter, state.document());
        // Paste several paragraphs.
        state.insert_text("one\ntwo");
        check(&mut counter, state.document());
        state.select_all();
        state.delete_selection();
        assert_eq!(check(&mut counter, state.document()), 0);
        state.undo();
        check(&mut counter, state.document());
    }
}

mod lengths {
    use writer_app::settings::{LengthUnit, format_length, parse_length};
    use writer_core::PaperSize;

    #[test]
    fn lengths_are_shown_briefly_and_read_with_units() {
        let mm = LengthUnit::Millimetres;
        let inches = LengthUnit::Inches;
        assert_eq!(LengthUnit::for_paper(PaperSize::A4), mm);
        assert_eq!(LengthUnit::for_paper(PaperSize::UsLetter), inches);
        assert_eq!(
            LengthUnit::for_paper(PaperSize::Custom {
                width: 300.,
                height: 400.
            }),
            mm
        );

        assert_eq!(format_length(72., mm), "25.4");
        assert_eq!(format_length(72., inches), "1");
        assert_eq!(format_length(612., inches), "8.5");
        assert_eq!(format_length(595.276, mm), "210");
        assert_eq!(format_length(841.89, mm), "297");
        assert_eq!(format_length(0., mm), "0");
        assert_eq!(format_length(54., inches), "0.75");

        let close = |text: &str, unit, points: f32| {
            let read = parse_length(text, unit).unwrap_or_else(|| panic!("{text:?} is a length"));
            assert!((read - points).abs() < 0.01, "{text:?}: {read} != {points}");
        };
        close("25.4", mm, 72.);
        close(" 25,4 ", mm, 72.);
        close("1", inches, 72.);
        close("20mm", inches, 56.693);
        close("2 cm", mm, 56.693);
        close("0.75in", mm, 54.);
        close("0.75\"", mm, 54.);
        close("36pt", mm, 36.);
        close("36 PT", inches, 36.);
        close("0", mm, 0.);
        for bad in ["", "abc", "12 parsecs", "-3", "1.2.3", "mm"] {
            assert_eq!(parse_length(bad, mm), None, "{bad:?}");
        }
        // What is shown reads back as the same length.
        for points in [72., 36., 595.276, 841.89, 612., 792., 100.] {
            for unit in [mm, inches] {
                let shown = format_length(points, unit);
                let read = parse_length(&shown, unit).unwrap();
                // To a tenth of a millimetre, a hundredth of an inch.
                assert!((read - points).abs() < 0.4, "{shown}: {read} != {points}");
            }
        }
    }
}

mod version_times {
    use writer_app::version_history::{LocalTime, version_time_label};

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> LocalTime {
        LocalTime {
            year,
            month,
            day,
            hour,
            minute,
        }
    }

    #[test]
    fn versions_are_dated_the_way_people_say_it() {
        let now = at(2026, 11, 11, 15, 30);
        assert_eq!(
            version_time_label(at(2026, 11, 11, 14, 5), now),
            "Today 14:05"
        );
        assert_eq!(
            version_time_label(at(2026, 11, 11, 0, 0), now),
            "Today 00:00"
        );
        assert_eq!(
            version_time_label(at(2026, 11, 10, 23, 59), now),
            "Yesterday 23:59"
        );
        assert_eq!(
            version_time_label(at(2026, 11, 9, 8, 15), now),
            "9 Nov, 08:15"
        );
        assert_eq!(
            version_time_label(at(2026, 3, 3, 17, 40), now),
            "3 Mar, 17:40"
        );
        assert_eq!(
            version_time_label(at(2025, 12, 31, 9, 0), now),
            "31 Dec 2025, 09:00"
        );
        // Across the turn of a month and of a year.
        assert_eq!(
            version_time_label(at(2026, 2, 28, 10, 0), at(2026, 3, 1, 9, 0)),
            "Yesterday 10:00"
        );
        assert_eq!(
            version_time_label(at(2024, 2, 29, 10, 0), at(2024, 3, 1, 9, 0)),
            "Yesterday 10:00"
        );
        assert_eq!(
            version_time_label(at(2025, 12, 31, 22, 0), at(2026, 1, 1, 1, 0)),
            "Yesterday 22:00"
        );
    }

    #[test]
    fn local_time_is_read_from_the_clock() {
        // Whatever the time zone of the machine, a minute later is a minute
        // later, and the fields are in range.
        let a = LocalTime::from_unix_ms(1_800_000_000_000);
        let b = LocalTime::from_unix_ms(1_800_000_060_000);
        assert!((2026..=2027).contains(&a.year));
        assert!((1..=12).contains(&a.month) && (1..=31).contains(&a.day));
        assert!(a.hour < 24 && a.minute < 60);
        assert_eq!((b.minute + 60 - a.minute) % 60, 1);
    }
}

mod quick_open_titles {
    use writer_app::quick_open::clip_title;

    #[test]
    fn long_titles_are_cut_and_keep_their_matches() {
        let short = "Chapter 2: The Harbour";
        assert_eq!(
            clip_title(short, &[0, 8], 48),
            (short.to_string(), vec![0, 8])
        );

        let long = "The ferry came in late, as it always did in October, and Mara was the only";
        let (clipped, positions) = clip_title(long, &[4, 9, 70], 48);
        assert_eq!(clipped.chars().count(), 48);
        assert!(clipped.ends_with('\u{2026}'));
        assert!(long.starts_with(clipped.trim_end_matches('\u{2026}')));
        // Matches that are still in view stay, the one that was cut goes.
        assert_eq!(positions, vec![4, 9]);

        // Cut between characters, never inside one.
        let accents = "é".repeat(60);
        let (clipped, _) = clip_title(&accents, &[], 10);
        assert_eq!(clipped, format!("{}\u{2026}", "é".repeat(9)));
        let families = "👨‍👩‍👧‍👦".repeat(12);
        let (clipped, _) = clip_title(&families, &[], 5);
        assert_eq!(clipped, format!("{}\u{2026}", "👨‍👩‍👧‍👦".repeat(4)));
        // No space is left hanging before the ellipsis.
        let (clipped, _) = clip_title("word word word word", &[], 11);
        assert_eq!(clipped, "word word\u{2026}");
    }
}

mod shortcut_places {
    use writer_app::shortcuts::Place;

    #[test]
    fn a_key_context_says_where_the_keyboard_must_be() {
        let place = |predicate: &str| Place::of_predicate(Some(predicate));
        assert_eq!(Place::of_predicate(None), Place::Anywhere);
        assert_eq!(place("Editor"), Place::Text);
        assert_eq!(place("EditorTools"), Place::Text);
        assert_eq!(place("FindBar"), Place::FindBar);
        assert_eq!(place("FindBar > TextField"), Place::FindBar);
        assert_eq!(place("LibraryPanel"), Place::Library);
        assert_eq!(
            place("LibraryPanel && mode == tree && !TextField"),
            Place::Library
        );
        assert_eq!(
            place("LibraryPanel && mode == trash && !TextField"),
            Place::Trash
        );
        assert_eq!(place("CommentThreads"), Place::CommentList);
        assert_eq!(place("CommentComposer"), Place::CommentField);
        assert_eq!(place("Picker"), Place::List);
        assert_eq!(place("SettingsRow"), Place::Settings);
        assert_eq!(place("VersionHistory"), Place::VersionHistory);
        assert_eq!(place("TextField"), Place::TextField);
        assert_eq!(place("SomethingNew"), Place::Elsewhere);

        assert_eq!(Place::Anywhere.heading("File"), "Anywhere \u{b7} File");
        assert_eq!(Place::Text.heading("Format"), "In the text \u{b7} Format");
        // A place that is its own category is not said twice.
        assert_eq!(Place::Library.heading("Library"), "In the library");
        assert_eq!(
            Place::CommentList.heading("Comments"),
            "In the comment list"
        );
        assert_eq!(Place::FindBar.heading(""), "In the find bar");
        // The overlay lists the places in this order.
        assert!(Place::Anywhere < Place::Text && Place::Text < Place::Library);
    }
}

mod panel_layout {
    use writer_app::workspace::{
        MIN_EDITOR_WIDTH, MIN_RIGHT_PANEL_WIDTH, PanelLayout, SidePanel, panel_layout,
    };

    #[test]
    fn the_text_keeps_its_room() {
        let both = |viewport: f32, keep| panel_layout(viewport, Some(260.), Some(320.), keep);
        // Room for everything.
        assert_eq!(
            both(1200., SidePanel::Right),
            PanelLayout {
                sidebar: Some(260.),
                right: Some(320.)
            }
        );
        // Tighter: both give, the text has its minimum.
        let tight = both(900., SidePanel::Right);
        let (sidebar, right) = (tight.sidebar.unwrap(), tight.right.unwrap());
        assert!((sidebar + right + MIN_EDITOR_WIDTH - 900.).abs() < 0.01);
        assert!((200. ..260.).contains(&sidebar));
        assert!((MIN_RIGHT_PANEL_WIDTH..320.).contains(&right));
        // At the point where both are at their minimum.
        assert_eq!(
            both(780., SidePanel::Library),
            PanelLayout {
                sidebar: Some(200.),
                right: Some(220.)
            }
        );
        // Narrower still: one panel, the one used last, and never both.
        assert_eq!(
            both(700., SidePanel::Right),
            PanelLayout {
                sidebar: None,
                right: Some(320.)
            }
        );
        assert_eq!(
            both(700., SidePanel::Library),
            PanelLayout {
                sidebar: Some(260.),
                right: None
            }
        );
        assert_eq!(both(640., SidePanel::Right).right, Some(280.));
        assert_eq!(both(640., SidePanel::Library).sidebar, Some(260.));
        // A single panel yields down to its minimum.
        assert_eq!(
            panel_layout(640., Some(400.), None, SidePanel::Right).sidebar,
            Some(280.)
        );
        assert_eq!(
            panel_layout(500., Some(400.), None, SidePanel::Right).sidebar,
            Some(200.)
        );
        assert_eq!(
            panel_layout(500., None, Some(400.), SidePanel::Library).right,
            Some(220.)
        );
        assert_eq!(
            panel_layout(1000., None, None, SidePanel::Right),
            PanelLayout::default()
        );
        // Whatever the window, the text keeps its minimum while one panel
        // at its minimum fits beside it.
        for viewport in (640..=1600).step_by(20) {
            for keep in [SidePanel::Library, SidePanel::Right] {
                let layout = both(viewport as f32, keep);
                let panels = layout.sidebar.unwrap_or(0.) + layout.right.unwrap_or(0.);
                assert!(
                    viewport as f32 - panels >= MIN_EDITOR_WIDTH - 0.01,
                    "{viewport}: {layout:?}"
                );
                assert!(layout.sidebar.is_some() || layout.right.is_some());
            }
        }
    }
}

mod command_titles {
    use writer_app::commands::sentence_case;

    #[test]
    fn titles_are_brought_to_sentence_case() {
        for (given, expected) in [
            ("Bold", "Bold"),
            ("Code Block", "Code block"),
            ("Toggle To-do Checked", "Toggle to-do checked"),
            ("Paste as Plain Text", "Paste as plain text"),
            ("Open Link in Browser", "Open link in browser"),
            ("Today's Date", "Today's date"),
            ("Heading 1", "Heading 1"),
            ("Align Centre", "Align centre"),
            // Already in sentence case, names and abbreviations included.
            ("Move to trash", "Move to trash"),
            ("Toggle Vim mode", "Toggle Vim mode"),
            ("Copy as Markdown", "Copy as Markdown"),
            ("Export as DOCX", "Export as DOCX"),
            ("Open a Word Document", "Open a Word document"),
            ("", ""),
        ] {
            assert_eq!(sentence_case(given), expected);
        }
    }
}
