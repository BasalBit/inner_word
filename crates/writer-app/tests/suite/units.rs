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
