//! The crate from the outside, as the app uses it: the keystroke text
//! against every key the tree binds today, order and precedence after
//! every kind of change, the shortcuts file there and back, files of
//! other versions, files that are not such files.

use serde_json::{Value, json};
use writer_keymap::{
    Catalogue, Changes, Context, DefaultBinding, Defaults, Effective, FILE_FORMAT, FILE_NAME,
    FILE_VERSION, ImportError, ImportMode, InactiveReason, KeySequence, Keymap, Keystroke, Origin,
    Overlap, Places, Refusal, SkipReason, Target, VerdictKind, apply,
};

// ---------------------------------------------------------------------
// The keys of the tree
// ---------------------------------------------------------------------

/// Every keystroke text the tree binds (see the header of the file).
fn corpus() -> Vec<&'static str> {
    include_str!("fixtures/keystrokes.txt")
        .lines()
        .filter(|line| !line.starts_with("# ") && !line.is_empty())
        .collect()
}

/// A keystroke as GPUI holds it: control, alt, shift, platform, function,
/// and the key.
type GpuiKeystroke = (bool, bool, bool, bool, bool, String);

/// `gpui::Keystroke::parse` of gpui 0.2.2 (src/platform/keystroke.rs),
/// copied line for line, on a Mac, without the character a key types.
/// What this crate writes has to be read by it.
fn gpui_parse(source: &str) -> Option<GpuiKeystroke> {
    let (mut control, mut alt, mut shift, mut platform, mut function) =
        (false, false, false, false, false);
    let mut key = None;
    let mut components = source.split('-').peekable();
    while let Some(component) = components.next() {
        if component.eq_ignore_ascii_case("ctrl") {
            control = true;
            continue;
        }
        if component.eq_ignore_ascii_case("alt") {
            alt = true;
            continue;
        }
        if component.eq_ignore_ascii_case("shift") {
            shift = true;
            continue;
        }
        if component.eq_ignore_ascii_case("fn") {
            function = true;
            continue;
        }
        if component.eq_ignore_ascii_case("secondary")
            || component.eq_ignore_ascii_case("cmd")
            || component.eq_ignore_ascii_case("super")
            || component.eq_ignore_ascii_case("win")
        {
            platform = true;
            continue;
        }
        let mut key_str = component.to_string();
        if let Some(next) = components.peek() {
            if next.is_empty() && source.ends_with('-') {
                key = Some(String::from("-"));
                break;
            } else if next.len() > 1 && next.starts_with('>') {
                key = Some(key_str);
                components.next();
            } else {
                return None;
            }
            continue;
        }
        if component.len() == 1 && component.as_bytes()[0].is_ascii_uppercase() {
            shift = true;
        }
        key_str.make_ascii_lowercase();
        key = Some(key_str);
    }
    let key = key.or_else(|| {
        if std::mem::take(&mut shift) {
            Some("shift".to_string())
        } else if std::mem::take(&mut control) {
            Some("control".to_string())
        } else if std::mem::take(&mut alt) {
            Some("alt".to_string())
        } else if std::mem::take(&mut platform) {
            Some("platform".to_string())
        } else if std::mem::take(&mut function) {
            Some("function".to_string())
        } else {
            None
        }
    })?;
    Some((control, alt, shift, platform, function, key))
}

fn gpui_parse_all(source: &str) -> Option<Vec<GpuiKeystroke>> {
    source.split_whitespace().map(gpui_parse).collect()
}

fn key(text: &str) -> KeySequence {
    KeySequence::parse(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

fn keys(texts: &[&str]) -> Vec<KeySequence> {
    texts.iter().map(|text| key(text)).collect()
}

#[test]
fn the_corpus_is_the_tree_s() {
    let corpus = corpus();
    assert!(corpus.len() >= 160, "{} keys", corpus.len());
    for text in [
        "cmd-shift-p",
        "cmd-&",
        "cmd-shift-7",
        "cmd-\"",
        "cmd-\\",
        "g g",
        "/",
        "cmd->",
    ] {
        assert!(corpus.contains(&text), "{text}");
    }
}

#[test]
fn every_key_of_the_tree_is_read_as_gpui_reads_it() {
    for text in corpus() {
        let sequence = key(text);
        let gpui =
            gpui_parse_all(text).unwrap_or_else(|| panic!("the copy of GPUI's parser: {text}"));
        assert_eq!(sequence.strokes().len(), gpui.len(), "{text}");

        // Binding the spellings binds what the tree bound: GPUI's reading
        // of the source is GPUI's reading of one of them.
        let spellings = sequence.spellings();
        assert!(
            spellings
                .iter()
                .any(|spelling| gpui_parse_all(spelling).as_ref() == Some(&gpui)),
            "{text}: {spellings:?}"
        );
        // Every spelling is a text GPUI reads, and the same key here.
        for spelling in &spellings {
            assert!(gpui_parse_all(spelling).is_some(), "{text}: {spelling}");
            assert_eq!(key(spelling), sequence, "{text}: {spelling}");
        }
        // The normal form reads as itself and is the last spelling.
        let normal = sequence.to_string();
        assert_eq!(key(&normal), sequence, "{text}");
        assert_eq!(spellings.last(), Some(&normal), "{text}");

        // Where the key is not a shifted character, this crate and GPUI
        // agree on every part.
        for (stroke, gpui) in sequence.strokes().iter().zip(&gpui) {
            let modifiers = stroke.modifiers();
            let ours = (
                modifiers.control,
                modifiers.alt,
                modifiers.shift,
                modifiers.command,
                modifiers.function,
                stroke.key().to_string(),
            );
            if stroke.spellings().len() == 1 {
                assert_eq!(&ours, gpui, "{text}");
            } else {
                // One key, two spellings: shift with the plain key.
                assert!(ours.2, "{text}");
                assert_eq!(
                    (ours.0, ours.1, ours.3, ours.4),
                    (gpui.0, gpui.1, gpui.3, gpui.4)
                );
            }
        }
    }
}

#[test]
fn the_tree_writes_its_keys_in_the_normal_form() {
    // The order of the modifiers this crate writes is the tree's own, so
    // an exported file reads like the sources and the documentation. The
    // only keys that print differently are the shifted characters, which
    // print as shift with their key.
    let mut shifted = Vec::new();
    for text in corpus() {
        let normal = key(text).to_string();
        if normal != text {
            shifted.push((text, normal));
        }
    }
    shifted.sort();
    let mut expected: Vec<(&str, String)> = [
        ("cmd-\"", "cmd-shift-'"),
        ("cmd-&", "cmd-shift-7"),
        ("cmd-(", "cmd-shift-9"),
        ("cmd-*", "cmd-shift-8"),
        ("cmd-:", "cmd-shift-;"),
        ("cmd-<", "cmd-shift-,"),
        ("cmd->", "cmd-shift-."),
    ]
    .into_iter()
    .map(|(text, normal)| (text, normal.to_string()))
    .collect();
    expected.sort();
    assert_eq!(shifted, expected);
    // And each of those has its readable twin in the tree: the pair is
    // one key.
    for (sent, readable) in &expected {
        assert!(corpus().contains(&readable.as_str()), "{readable}");
        assert_eq!(key(sent), key(readable));
        assert_eq!(
            key(sent).spellings(),
            vec![sent.to_string(), readable.clone()]
        );
    }
}

#[test]
fn gpui_s_own_writing_of_a_key_is_read() {
    // Keystroke::unparse writes ctrl, alt, cmd, shift: another order.
    assert_eq!(key("alt-cmd-s").to_string(), "cmd-alt-s");
    assert_eq!(
        key("ctrl-alt-cmd-shift-p").to_string(),
        "ctrl-cmd-alt-shift-p"
    );
    assert_eq!(key("cmd-shift-;"), key("cmd-:"));
    let stroke = Keystroke::parse("alt-cmd-shift-;").unwrap();
    assert_eq!(stroke.spellings(), vec!["cmd-alt-:", "cmd-alt-shift-;"]);
}

/// Every key context predicate the tree binds keys in, as it is written
/// in the sources (crates/*/src, at version 0.4.0).
const CONTEXTS: [&str; 30] = [
    "AgentsView",
    "AssistantComposer",
    "AssistantContext",
    "AssistantHeader",
    "AssistantMessages",
    "AssistantPanel",
    "Button",
    "CommentComposer",
    "CommentThreads && !confirming && !composing",
    "CommentThreads && confirming",
    "ConfirmDialog",
    "Editor",
    "Editor && assistant",
    "Editor && assistant_quick",
    "Editor && assistant_side",
    "EditorTools",
    "FindBar",
    "FindBar > TextField",
    "LibraryPanel",
    "LibraryPanel && !TextField",
    "LibraryPanel && mode == trash && !TextField",
    "LibraryPanel && mode == tree && !TextField",
    "LinkPopover",
    "Pane",
    "Picker",
    "SegmentedControl",
    "SettingsRow",
    "SettingsView",
    "TextField",
    "VersionHistory",
];

/// What GPUI's `Display` prints for a predicate parsed from `text`, for
/// the shapes the tree uses: "and" in parentheses, nested to the left.
fn gpui_print(text: &str) -> String {
    let mut parts = text.split(" && ");
    let mut printed = parts.next().unwrap().to_string();
    for part in parts {
        printed = format!("({printed} && {part})");
    }
    printed
}

#[test]
fn every_context_of_the_tree_is_one_context_however_it_is_spelled() {
    assert_eq!(gpui_print("a && b == c && !d"), "((a && b == c) && !d)");
    for written in CONTEXTS {
        let context = Context::parse(written);
        // The canonical text is what the sources write.
        assert_eq!(context.text(), Some(written));
        // What GPUI prints for it is the same context.
        assert_eq!(Context::parse(&gpui_print(written)), context, "{written}");
        assert!(!context.names().is_empty(), "{written}");
    }
    // Different texts are different contexts.
    for (ix, one) in CONTEXTS.iter().enumerate() {
        for other in &CONTEXTS[ix + 1..] {
            assert_ne!(Context::parse(one), Context::parse(other));
        }
    }
}

#[test]
fn the_idle_and_the_confirming_comment_list_are_different_places() {
    let places = Places::new();
    let relation = places.relation(
        &Context::parse("CommentThreads && !confirming && !composing"),
        &Context::parse("CommentThreads && confirming"),
    );
    assert_eq!(relation.extent, writer_keymap::Extent::Apart);
    let relation = places.relation(
        &Context::parse("Editor && assistant_quick"),
        &Context::parse("Editor && assistant"),
    );
    assert_eq!(relation.extent, writer_keymap::Extent::Overlapping);
}

/// A small generator of texts, the same on every run.
struct Texts(u64);

impl Texts {
    fn next(&mut self, alphabet: &[&str], max: usize) -> String {
        let mut step = || {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        };
        let len = (step() % (max as u64 + 1)) as usize;
        (0..len)
            .map(|_| alphabet[(step() % alphabet.len() as u64) as usize])
            .collect()
    }
}

#[test]
fn no_text_makes_the_readers_panic() {
    let mut texts = Texts(0x9E37_79B9_7F4A_7C15);
    let key_parts = [
        "cmd", "ctrl", "alt", "shift", "fn", "-", "-", "-", ">", "a", "B", "7", "&", " ", "enter",
        "é", "opt", "space", "", "\t", "\u{0}",
    ];
    let context_parts = [
        "Editor",
        "TextField",
        " ",
        " ",
        "&&",
        "||",
        "==",
        "!=",
        "!",
        "(",
        ")",
        ">",
        "mode",
        "é",
        "&",
        "|",
        "=",
        "-",
        "_",
    ];
    let json_parts = [
        "{",
        "}",
        "[",
        "]",
        ":",
        ",",
        "\"",
        "format",
        "the-writer-shortcuts",
        "version",
        "1",
        "shortcuts",
        "changes",
        "command",
        "keys",
        "default",
        "context",
        "null",
        "cmd-b",
        " ",
        "\n",
        "editor::ToggleBold",
        "Editor",
    ];
    let keymap = keymap();
    for _ in 0..4000 {
        let text = texts.next(&key_parts, 6);
        if let Ok(sequence) = KeySequence::parse(&text) {
            // What is read is written as something that reads the same.
            assert_eq!(key(&sequence.to_string()), sequence, "{text:?}");
            for spelling in sequence.spellings() {
                assert_eq!(key(&spelling), sequence, "{text:?} as {spelling:?}");
                assert!(
                    gpui_parse_all(&spelling).is_some(),
                    "{text:?} as {spelling:?}"
                );
            }
        }
        let text = texts.next(&context_parts, 8);
        let context = Context::parse(&text);
        if let Some(canonical) = context.text() {
            assert_eq!(Context::parse(canonical), context, "{text:?}");
        }
        let _ = places().is_typing(&context);
        let _ = places().relation(&context, &Context::parse("Editor"));
        let text = texts.next(&json_parts, 24);
        let _ = keymap.plan_import(&text);
        let _ = Changes::from_stored(&text);
    }
}

// ---------------------------------------------------------------------
// A keymap like the app's, in miniature
// ---------------------------------------------------------------------

/// Action, context, keystrokes, in the order the app binds: its own
/// window-wide keys first, the widgets, the text, the library, the
/// comments, the tools.
const BINDINGS: [(&str, Option<&str>, &str); 24] = [
    ("app::CommandPalette", None, "cmd-shift-p"),
    ("app::QuickOpen", None, "cmd-p"),
    ("app::Quit", None, "cmd-q"),
    ("field::Confirm", Some("TextField"), "enter"),
    ("field::Cancel", Some("TextField"), "escape"),
    ("context_menu::Open", Some("TextField"), "shift-f10"),
    ("context_menu::Open", Some("TextField"), "ctrl-enter"),
    ("editor::MoveLeft", Some("Editor"), "ctrl-b"),
    ("editor::MoveLeft", Some("Editor"), "left"),
    ("editor::ToggleBold", Some("Editor"), "cmd-b"),
    ("editor::ToggleNumberedList", Some("Editor"), "cmd-&"),
    ("editor::ToggleNumberedList", Some("Editor"), "cmd-shift-7"),
    ("editor::InsertPageBreak", Some("Editor"), "cmd-enter"),
    ("spell::PreviousMisspelling", Some("Editor"), "cmd-:"),
    ("spell::PreviousMisspelling", Some("Editor"), "cmd-shift-;"),
    (
        "library::SelectFirst",
        Some("(LibraryPanel && !TextField)"),
        "g g",
    ),
    (
        "library::NewDocument",
        Some("((LibraryPanel && mode == tree) && !TextField)"),
        "n",
    ),
    (
        "context_menu::Open",
        Some("(LibraryPanel && !TextField)"),
        "ctrl-enter",
    ),
    ("comments::Toggle", None, "alt-cmd-c"),
    ("tools::Find", Some("EditorTools"), "cmd-f"),
    ("tools::FindPrevious", Some("FindBar"), "shift-enter"),
    ("tools::FindPrevious", Some("EditorTools"), "cmd-shift-g"),
    (
        "tools::CaseSensitive",
        Some("FindBar > TextField"),
        "alt-cmd-c",
    ),
    (
        "tools::ReplaceAll",
        Some("FindBar > TextField"),
        "cmd-enter",
    ),
];

fn defaults() -> Defaults {
    let mut defaults = Defaults::new();
    for (action, context, keystrokes) in BINDINGS {
        defaults.push(DefaultBinding::new(action, None, context, keystrokes).unwrap());
    }
    defaults.add_keyless(Target::new("app::VersionHistory", None));
    defaults.add_keyless(Target::new("app::DocumentSettings", None));
    defaults
}

fn places() -> Places {
    Places::new()
        .typing("Editor")
        .typing("TextField")
        .inside("TextField", "FindBar")
        .inside("TextField", "LibraryPanel")
        .inside("FindBar", "EditorTools")
        .container("FindBar")
}

fn keymap() -> Keymap {
    Keymap::new(defaults(), places())
}

fn catalogue() -> Catalogue {
    let mut catalogue = Catalogue::new();
    // The order of the list: by place, not by binding.
    for (action, context, title, place) in [
        ("app::CommandPalette", None, "Command palette", "Anywhere"),
        ("app::QuickOpen", None, "Quick open document", "Anywhere"),
        ("app::VersionHistory", None, "Version history", "Anywhere"),
        ("app::Quit", None, "Quit", "Anywhere"),
        (
            "comments::Toggle",
            None,
            "Show or hide comments",
            "Anywhere",
        ),
        ("editor::ToggleBold", Some("Editor"), "Bold", "In the text"),
        (
            "editor::ToggleNumberedList",
            Some("Editor"),
            "Numbered \"list\"",
            "In the text",
        ),
        (
            "tools::CaseSensitive",
            Some("FindBar > TextField"),
            "Case sensitive",
            "In the find bar",
        ),
    ] {
        catalogue.add(Target::new(action, context), title, place);
    }
    catalogue
}

fn bold() -> Target {
    Target::new("editor::ToggleBold", Some("Editor"))
}

fn list() -> Target {
    Target::new("editor::ToggleNumberedList", Some("Editor"))
}

fn left() -> Target {
    Target::new("editor::MoveLeft", Some("Editor"))
}

fn history() -> Target {
    Target::new("app::VersionHistory", None)
}

/// The entries of the stored changes, each as its JSON, sorted: the order
/// in which changes were made is not a part of what they are (the keymap
/// they give is the same).
fn entries(changes: &Changes) -> Vec<String> {
    let mut entries: Vec<String> = changes
        .entries()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect();
    entries.sort();
    entries
}

fn spellings(effective: &Effective) -> Vec<String> {
    effective
        .bindings()
        .iter()
        .map(|binding| binding.spelling.clone())
        .collect()
}

/// The actions bound in the keymap, in order: what precedence is made of.
fn actions(effective: &Effective) -> Vec<String> {
    effective
        .bindings()
        .iter()
        .map(|binding| binding.target.command.action.clone())
        .collect()
}

// ---------------------------------------------------------------------
// Order and precedence
// ---------------------------------------------------------------------

#[test]
fn unchanged_the_keymap_is_what_was_handed_over() {
    let keymap = keymap();
    let effective = keymap.effective();
    let handed: Vec<String> = BINDINGS
        .iter()
        .map(|(_, _, keys)| keys.to_string())
        .collect();
    assert_eq!(spellings(&effective), handed);
    let bound: Vec<String> = BINDINGS
        .iter()
        .map(|(action, _, _)| action.to_string())
        .collect();
    assert_eq!(actions(&effective), bound);
    for (index, binding) in effective.bindings().iter().enumerate() {
        assert_eq!(binding.source, Some(index));
        assert_eq!(binding.origin, Origin::Default);
        // The context is the one handed over, however GPUI printed it.
        assert_eq!(
            binding.target.context,
            Context::from_option(BINDINGS[index].1)
        );
    }
    assert_eq!(keymap.stored(), None);
    assert!(keymap.collisions().is_empty());
    // Every target once, the keyless ones last.
    assert_eq!(effective.targets().len(), 22);
    assert_eq!(effective.targets()[20].target, history());
}

#[test]
fn every_kind_of_change_keeps_every_other_binding_in_its_place() {
    let mut keymap = keymap();
    // Replaced, added, removed, given.
    assert!(keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b")));
    assert!(keymap.add_key(&left(), key("alt-h")));
    assert!(keymap.remove_key(&Target::new("app::QuickOpen", None), &key("cmd-p")));
    assert!(keymap.add_key(&history(), key("cmd-alt-h")));
    assert!(keymap.replace_key(&list(), &key("cmd-&"), key("cmd-alt-shift-7")));
    let effective = keymap.effective();

    let expected: Vec<&str> = vec![
        // Given to a command without a key: the front.
        "cmd-alt-h",
        "cmd-shift-p",
        // cmd-p removed: a gap.
        "cmd-q",
        "enter",
        "escape",
        "shift-f10",
        "ctrl-enter",
        // Added: before the target's first default.
        "alt-h",
        "ctrl-b",
        "left",
        // Replaced: where the default stood.
        "cmd-alt-b",
        // A key in two spellings replaced by one in two spellings, at the
        // position of the first.
        "cmd-alt-&",
        "cmd-alt-shift-7",
        "cmd-enter",
        "cmd-:",
        "cmd-shift-;",
        "g g",
        "n",
        "ctrl-enter",
        "alt-cmd-c",
        "cmd-f",
        "shift-enter",
        "cmd-shift-g",
        "alt-cmd-c",
        "cmd-enter",
    ];
    assert_eq!(spellings(&effective), expected);

    // The bindings nobody changed are in the order they were handed over.
    let untouched = |effective: &Effective| -> Vec<usize> {
        effective
            .bindings()
            .iter()
            .filter(|binding| binding.origin == Origin::Default)
            .map(|binding| binding.source.unwrap())
            .collect()
    };
    let order = untouched(&effective);
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
    // And a changed or added binding names a default of its own target.
    for binding in effective.bindings() {
        if let Some(source) = binding.source {
            assert_eq!(
                keymap.defaults().bindings()[source].target(),
                binding.target
            );
        } else {
            assert_eq!(binding.target, history());
        }
    }

    // What the rows say.
    let row = effective.target(&bold()).unwrap();
    assert!(row.changed);
    assert_eq!(
        row.keys[0].origin,
        Origin::Changed {
            default: key("cmd-b")
        }
    );
    let row = effective.target(&left()).unwrap();
    assert_eq!(
        row.shown,
        Some(key("left")),
        "hints keep showing the default"
    );
    assert_eq!(row.keys.last().unwrap().origin, Origin::Added);
    assert_eq!(effective.changed().count(), 5);
    let palette = effective
        .target(&Target::new("app::CommandPalette", None))
        .unwrap();
    assert!(!palette.changed);
}

#[test]
fn one_command_back_and_everything_back() {
    let mut keymap = keymap();
    let pristine = spellings(&keymap.effective());
    keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
    keymap.add_key(&history(), key("cmd-alt-h"));
    let menu = Target::new("context_menu::Open", Some("TextField"));
    keymap.remove_key(&menu, &key("shift-f10"));
    keymap.remove_key(
        &Target::new("context_menu::Open", Some("LibraryPanel && !TextField")),
        &key("ctrl-enter"),
    );

    assert!(keymap.reset(&bold()));
    assert!(!keymap.effective().target(&bold()).unwrap().changed);
    assert_eq!(keymap.effective().changed().count(), 3);

    // One command back in every place it has keys.
    assert!(keymap.reset_command(&menu.command));
    assert_eq!(keymap.effective().changed().count(), 1);

    let before = keymap.reset_all();
    assert_eq!(spellings(&keymap.effective()), pristine);
    assert_eq!(keymap.stored(), None);
    // Undo of "reset all".
    keymap.set_changes(before);
    assert_eq!(keymap.effective().changed().count(), 1);
}

#[test]
fn the_precedence_facts_hold_after_a_change() {
    // "A binding without a context ranks like the deepest": the find
    // bar's cmd-alt-c is bound later than the window's and at the level of
    // the field, so it wins there. Changing either key leaves the other
    // where it stood.
    let mut keymap = keymap();
    let comments = Target::new("comments::Toggle", None);
    let case = Target::new("tools::CaseSensitive", Some("FindBar > TextField"));
    let position = |effective: &Effective, target: &Target| {
        effective
            .bindings()
            .iter()
            .position(|binding| binding.target == *target)
            .unwrap()
    };
    let effective = keymap.effective();
    assert!(position(&effective, &comments) < position(&effective, &case));

    keymap.replace_key(&comments, &key("cmd-alt-c"), key("cmd-alt-k"));
    keymap.replace_key(&case, &key("cmd-alt-c"), key("cmd-alt-k"));
    let effective = keymap.effective();
    assert!(position(&effective, &comments) < position(&effective, &case));
    // And the verdict for that pair is what it is today: allowed, the
    // field keeps the key.
    keymap.reset(&case);
    let verdict = keymap.check(&case, &key("cmd-alt-k"), Some(&key("cmd-alt-c")));
    assert_eq!(verdict.kind(), VerdictKind::UsedElsewhere);
    assert_eq!(verdict.elsewhere[0].overlap, Overlap::ProposedWinsThere);
    assert_eq!(verdict.elsewhere[0].target, comments);
}

#[test]
fn a_check_changes_nothing() {
    let keymap = keymap();
    let before = keymap.clone();
    let verdict = keymap.check(&bold(), &key("cmd-enter"), Some(&key("cmd-b")));
    assert_eq!(verdict.kind(), VerdictKind::Taken);
    assert_eq!(keymap, before);
    // Refused: a character where text is typed, and a key macOS keeps.
    assert_eq!(
        keymap.check(&bold(), &key("b"), None).refusal,
        Some(Refusal::TypesText)
    );
    assert!(matches!(
        keymap.check(&bold(), &key("cmd-tab"), None).refusal,
        Some(Refusal::SystemKeeps(_))
    ));
    // The library's own bare keys are where no text is typed.
    let new = Target::new(
        "library::NewDocument",
        Some("LibraryPanel && mode == tree && !TextField"),
    );
    assert_eq!(
        keymap.check(&new, &key("d"), Some(&key("n"))).kind(),
        VerdictKind::Fine
    );
}

// ---------------------------------------------------------------------
// The stored value
// ---------------------------------------------------------------------

#[test]
fn the_keymap_comes_back_from_what_was_stored() {
    let mut keymap = keymap();
    keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
    keymap.add_key(&history(), key("cmd-alt-h"));
    keymap.remove_key(&left(), &key("ctrl-b"));
    let stored = keymap.stored().unwrap();

    let (again, error) = Keymap::with_stored(defaults(), places(), Some(&stored));
    assert_eq!(error, None);
    assert_eq!(again, keymap);
    assert_eq!(again.effective(), keymap.effective());

    let (fresh, error) = Keymap::with_stored(defaults(), places(), None);
    assert_eq!(error, None);
    assert_eq!(fresh.stored(), None);

    let (broken, error) = Keymap::with_stored(defaults(), places(), Some("{\"changes\": [}"));
    assert!(error.is_some());
    assert_eq!(broken.stored(), None);
    assert_eq!(broken.effective().changed().count(), 0);
}

#[test]
fn a_later_version_s_defaults_arrive_by_themselves() {
    // Stored by an older version, in which bold was cmd-b, moving left
    // had ctrl-h, and there was a command that is gone.
    let stored = json!({
        "version": 1,
        "changes": [
            {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": ["cmd-alt-b"]},
            {"command": "editor::MoveLeft", "context": "Editor", "default": ["ctrl-h", "left"], "keys": ["left"]},
            {"command": "app::Typewriter", "context": null, "default": [], "keys": ["cmd-alt-t"]},
        ]
    })
    .to_string();
    // This version: bold is cmd-shift-b and has a second key, moving left
    // has ctrl-b (the tree's own), and a new command exists.
    let mut defaults = Defaults::new();
    for (action, context, keystrokes) in [
        ("editor::ToggleBold", Some("Editor"), "cmd-shift-b"),
        ("editor::ToggleBold", Some("Editor"), "f9"),
        ("editor::MoveLeft", Some("Editor"), "ctrl-b"),
        ("editor::MoveLeft", Some("Editor"), "left"),
        ("editor::Focus", Some("Editor"), "cmd-shift-f"),
    ] {
        defaults.push(DefaultBinding::new(action, None, context, keystrokes).unwrap());
    }
    let (keymap, error) = Keymap::with_stored(defaults, places(), Some(&stored));
    assert_eq!(error, None);
    let effective = keymap.effective();
    // The user's key for bold is still there; so are the new defaults.
    assert_eq!(
        spellings(&effective),
        [
            "cmd-alt-b",
            "cmd-shift-b",
            "f9",
            "ctrl-b",
            "left",
            "cmd-shift-f"
        ]
    );
    let reasons: Vec<&InactiveReason> = effective
        .inactive()
        .iter()
        .map(|change| &change.reason)
        .collect();
    assert_eq!(
        reasons,
        [
            &InactiveReason::ReplacedKeyGone {
                from: key("cmd-b"),
                to: key("cmd-alt-b")
            },
            &InactiveReason::RemovedKeyGone(key("ctrl-h")),
            &InactiveReason::UnknownCommand,
        ]
    );
    // Nothing of it is lost when the value is written again.
    let written: Value = serde_json::from_str(&keymap.stored().unwrap()).unwrap();
    let original: Value = serde_json::from_str(&stored).unwrap();
    assert_eq!(written, original);
}

// ---------------------------------------------------------------------
// The file
// ---------------------------------------------------------------------

#[test]
fn the_exported_file_is_complete_and_reads_as_json() {
    let mut keymap = keymap();
    keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
    let text = keymap.export(&catalogue(), "0.5.0");
    assert_eq!(FILE_NAME, "The Writer Shortcuts.json");
    assert!(text.ends_with("}\n"));

    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["format"], FILE_FORMAT);
    assert_eq!(file["version"], FILE_VERSION);
    assert_eq!(file["app_version"], "0.5.0");
    assert!(file["about"].as_str().unwrap().contains("edit \"keys\""));
    let shortcuts = file["shortcuts"].as_array().unwrap();
    // Every target once: those with keys and those without.
    assert_eq!(shortcuts.len(), keymap.defaults().targets().len());

    // The catalogue's order first, with its words.
    assert_eq!(
        shortcuts[2],
        json!({
            "command": "app::VersionHistory",
            "title": "Version history",
            "where": "Anywhere",
            "context": null,
            "default": [],
            "keys": []
        })
    );
    assert_eq!(
        shortcuts[5],
        json!({
            "command": "editor::ToggleBold",
            "title": "Bold",
            "where": "In the text",
            "context": "Editor",
            "default": ["cmd-b"],
            "keys": ["cmd-alt-b"]
        })
    );
    // A key in two spellings is one key, written the readable way; a
    // title is escaped.
    assert_eq!(shortcuts[6]["title"], "Numbered \"list\"");
    assert_eq!(shortcuts[6]["default"], json!(["cmd-shift-7"]));
    assert_eq!(shortcuts[7]["context"], "FindBar > TextField");
    // Then what the catalogue does not name, in the order of the
    // defaults, under the action's name.
    assert_eq!(shortcuts[8]["command"], "field::Confirm");
    assert_eq!(shortcuts[8]["title"], "field::Confirm");
    assert_eq!(shortcuts[8]["where"], "TextField");
    assert_eq!(
        shortcuts.last().unwrap()["command"],
        "app::DocumentSettings"
    );

    // A person reads it: one key list per line, keys inline.
    assert!(text.contains("      \"default\": [\"cmd-b\"],\n      \"keys\": [\"cmd-alt-b\"]\n"));
    assert!(text.contains("      \"keys\": [\"ctrl-b\", \"left\"]\n"));
    // The same keymap gives the same text.
    assert_eq!(text, keymap.export(&catalogue(), "0.5.0"));
}

#[test]
fn an_unchanged_keymap_exported_and_imported_changes_nothing() {
    let keymap = keymap();
    let text = keymap.export(&catalogue(), "0.5.0");
    let plan = keymap.plan_import(&text).unwrap();
    assert_eq!(plan.app_version.as_deref(), Some("0.5.0"));
    assert_eq!(plan.entries, 22);
    assert_eq!(plan.unchanged, 22);
    assert!(plan.changes.is_empty());
    assert!(plan.skipped.is_empty());
    for mode in [ImportMode::Replace, ImportMode::Merge] {
        let outcome = plan.outcome(mode);
        assert!(outcome.differences.is_empty());
        assert!(outcome.conflicts.is_empty());
        assert!(outcome.changes.is_empty());
    }
    assert!(!plan.modes_differ());
}

/// A keymap with every kind of change in it.
fn changed_keymap() -> Keymap {
    let mut keymap = keymap();
    keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
    keymap.replace_key(&list(), &key("cmd-shift-7"), key("cmd-alt-shift-7"));
    keymap.add_key(&left(), key("alt-h"));
    keymap.remove_key(&left(), &key("ctrl-b"));
    keymap.remove_key(&Target::new("app::QuickOpen", None), &key("cmd-p"));
    keymap.add_key(&history(), key("cmd-alt-h"));
    keymap.add_key(&Target::new("tools::Find", Some("EditorTools")), key("f3"));
    keymap
}

#[test]
fn a_changed_keymap_exported_and_imported_gives_the_same_changes() {
    let changed = changed_keymap();
    let text = changed.export(&catalogue(), "0.5.0");

    // Into the keymap it came from: nothing to do.
    let plan = changed.plan_import(&text).unwrap();
    assert_eq!(plan.changes.len(), 6);
    assert_eq!(plan.unchanged, 16);
    assert!(plan.skipped.is_empty());
    assert!(plan.outcome(ImportMode::Replace).differences.is_empty());
    assert!(plan.outcome(ImportMode::Merge).differences.is_empty());
    assert_eq!(
        entries(&plan.outcome(ImportMode::Replace).changes),
        entries(changed.changes())
    );

    // Into a fresh keymap: the same changes, the same keymap, binding
    // for binding.
    let mut fresh = keymap();
    let plan = fresh.plan_import(&text).unwrap();
    assert_eq!(plan.outcome(ImportMode::Replace).differences.len(), 6);
    assert!(plan.outcome(ImportMode::Replace).conflicts.is_empty());
    let undo = fresh.import(&plan, ImportMode::Replace);
    assert_eq!(entries(fresh.changes()), entries(changed.changes()));
    assert_eq!(fresh.changes().entries().count(), 6);
    assert_eq!(fresh.effective(), changed.effective());
    // And once more through a file gives the same file.
    assert_eq!(fresh.export(&catalogue(), "0.5.0"), text);

    // Undo puts back what was there.
    fresh.set_changes(undo);
    assert_eq!(fresh, keymap());
}

#[test]
fn replacing_the_user_s_changes_or_adding_to_them() {
    // The file: bold on cmd-alt-b, a key for the version history.
    let mut theirs = keymap();
    theirs.replace_key(&bold(), &key("cmd-b"), key("cmd-alt-b"));
    theirs.add_key(&history(), key("cmd-alt-h"));
    let text = theirs.export(&catalogue(), "0.5.0");

    // The user: bold on f5 (the file changes it too), and moving left
    // without ctrl-b (the file says nothing changed there).
    let mut mine = keymap();
    mine.replace_key(&bold(), &key("cmd-b"), key("f5"));
    mine.remove_key(&left(), &key("ctrl-b"));
    let plan = mine.plan_import(&text).unwrap();
    assert!(plan.modes_differ());

    // Replace: only the file's changes are left.
    let replace = plan.outcome(ImportMode::Replace);
    let changed: Vec<&Target> = replace.differences.iter().map(|d| &d.target).collect();
    // In the order of the defaults.
    assert_eq!(changed, [&left(), &bold(), &history()]);
    assert_eq!(replace.differences[0].before, keys(&["left"]));
    assert_eq!(replace.differences[0].after, keys(&["ctrl-b", "left"]));
    assert_eq!(replace.differences[1].before, keys(&["f5"]));
    assert_eq!(replace.differences[1].after, keys(&["cmd-alt-b"]));
    assert_eq!(entries(&replace.changes), entries(theirs.changes()));

    // Merge: the file wins for bold, the user's own change stays.
    let merge = plan.outcome(ImportMode::Merge);
    let changed: Vec<&Target> = merge.differences.iter().map(|d| &d.target).collect();
    assert_eq!(changed, [&bold(), &history()]);
    let carried = plan.carry_out(ImportMode::Merge);
    assert_eq!(carried.undo, *mine.changes());
    assert_eq!(
        carried.changes.keys_of(mine.defaults(), &bold()),
        keys(&["cmd-alt-b"])
    );
    assert_eq!(
        carried.changes.keys_of(mine.defaults(), &left()),
        keys(&["left"])
    );
    assert_eq!(
        carried.changes.keys_of(mine.defaults(), &history()),
        keys(&["cmd-alt-h"])
    );

    // Carried out and undone.
    let before = mine.clone();
    let undo = mine.import(&plan, ImportMode::Merge);
    assert_eq!(mine.effective().changed().count(), 3);
    mine.set_changes(undo);
    assert_eq!(mine, before);
}

/// A file as a person or an older version would write it.
fn file(shortcuts: Value) -> String {
    json!({"format": FILE_FORMAT, "version": 1, "app_version": "0.4.1", "shortcuts": shortcuts})
        .to_string()
}

#[test]
fn a_file_from_an_older_version() {
    // The older version: bold was cmd-shift-b (not changed by the user),
    // the palette was changed, a command existed that is gone, and the
    // version history did not exist as a command with a key.
    let text = file(json!([
        {"command": "editor::ToggleBold", "title": "Bold", "where": "In the text",
         "context": "Editor", "default": ["cmd-shift-b"], "keys": ["cmd-shift-b"]},
        {"command": "app::CommandPalette", "context": null,
         "default": ["cmd-shift-p"], "keys": ["cmd-k"]},
        {"command": "app::Typewriter", "title": "Typewriter mode", "where": "Anywhere",
         "context": null, "default": ["cmd-alt-t"], "keys": ["f7"]},
        // A default that has since changed, and that the user had changed:
        // moving left had ctrl-h then, replaced by alt-h.
        {"command": "editor::MoveLeft", "context": "Editor",
         "default": ["ctrl-h", "left"], "keys": ["alt-h", "left"]},
    ]));
    let mut keymap = keymap();
    let plan = keymap.plan_import(&text).unwrap();
    assert_eq!(plan.app_version.as_deref(), Some("0.4.1"));
    assert_eq!(plan.entries, 4);
    // Bold: its keys are the default it was written with. Not a change,
    // so the newer default (cmd-b) is not undone.
    assert_eq!(plan.unchanged, 1);
    assert_eq!(plan.changes.len(), 2);
    // The command that is gone is skipped, by name.
    assert_eq!(plan.skipped.len(), 1);
    assert_eq!(plan.skipped[0].reason, SkipReason::UnknownCommand);
    assert_eq!(plan.skipped[0].entry, 3);
    assert_eq!(
        plan.skipped[0].to_string(),
        "Entry 3 (Typewriter mode, app::Typewriter): this version does not have the command"
    );

    keymap.import(&plan, ImportMode::Replace);
    let effective = keymap.effective();
    assert!(!effective.target(&bold()).unwrap().changed);
    assert_eq!(effective.target(&bold()).unwrap().shown, Some(key("cmd-b")));
    let palette = effective
        .target(&Target::new("app::CommandPalette", None))
        .unwrap();
    assert_eq!(palette.keys[0].keys, key("cmd-k"));
    // The user's key for moving left arrives; the default that replaced
    // ctrl-h since (ctrl-b) stays, and so does the command that is new
    // (the version history, untouched).
    let row = effective.target(&left()).unwrap();
    let row_keys: Vec<String> = row.keys.iter().map(|key| key.keys.to_string()).collect();
    assert_eq!(row_keys, ["alt-h", "left", "ctrl-b"]);
    assert!(spellings(&effective).contains(&"ctrl-b".to_string()));
    assert!(effective.target(&history()).unwrap().keys.is_empty());
    // What was stored is made against the defaults of now: nothing in it
    // is stale.
    assert!(effective.inactive().is_empty());
}

#[test]
fn entries_that_are_skipped_say_why() {
    let text = file(json!([
        "not an entry",
        {"title": "No command"},
        {"command": "editor::ToggleBold", "context": "LibraryPanel", "keys": ["f5"]},
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": ["cmd-"]},
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": "cmd-b"},
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": ["b"]},
        {"command": "app::Quit", "context": null, "default": ["cmd-q"], "keys": ["cmd-tab"]},
        {"command": "app::QuickOpen", "context": 7, "keys": ["cmd-p"]},
        {"command": "editor::MoveLeft", "context": "Editor", "default": ["ctrl-b", "left"]},
        {"command": "comments::Toggle", "context": null, "keys": ["cmd-alt-k"]},
        {"command": "comments::Toggle", "context": null, "keys": ["cmd-alt-j"]},
        // Written by hand, with the names a person uses and no default.
        {"command": "tools::Find", "context": "EditorTools", "keys": ["Command-f", "opt-cmd-g"]},
    ]));
    let keymap = keymap();
    let plan = keymap.plan_import(&text).unwrap();
    let reasons: Vec<(usize, &SkipReason)> = plan
        .skipped
        .iter()
        .map(|skipped| (skipped.entry, &skipped.reason))
        .collect();
    assert_eq!(reasons.len(), 10, "{reasons:#?}");
    assert_eq!(reasons[0], (1, &SkipReason::NotAnEntry));
    assert_eq!(reasons[1], (2, &SkipReason::NoCommand));
    assert_eq!(reasons[2], (3, &SkipReason::UnknownPlace));
    assert!(matches!(
        reasons[3],
        (4, SkipReason::UnreadableKey { field: "keys", .. })
    ));
    assert_eq!(reasons[4], (5, &SkipReason::BadField { field: "keys" }));
    assert_eq!(
        reasons[5],
        (
            6,
            &SkipReason::Refused {
                key: key("b"),
                refusal: Refusal::TypesText
            }
        )
    );
    assert!(matches!(
        reasons[6],
        (
            7,
            SkipReason::Refused {
                refusal: Refusal::SystemKeeps(_),
                ..
            }
        )
    ));
    assert_eq!(reasons[7], (8, &SkipReason::BadField { field: "context" }));
    assert_eq!(reasons[8], (9, &SkipReason::BadField { field: "keys" }));
    assert_eq!(reasons[9], (11, &SkipReason::Duplicate));
    // Each says what and where.
    assert_eq!(
        plan.skipped[3].to_string(),
        "Entry 4 (editor::ToggleBold): a key cannot be read: \"cmd-\" has no key after its \
         modifiers (a key is written like cmd-shift-p)"
    );
    // A key that was read is named as the list names keys: in glyphs.
    assert_eq!(
        plan.skipped[5].to_string(),
        "Entry 6 (editor::ToggleBold): B is a character, and text is typed where the command \
         applies"
    );
    assert!(
        plan.skipped[6]
            .to_string()
            .contains("macOS keeps ⌘⇥ for itself: it switches between apps")
    );
    assert_eq!(
        plan.skipped[4].to_string(),
        "Entry 5 (editor::ToggleBold): its \"keys\" is not a list of keys in quotes (like \
         [\"cmd-b\", \"f5\"])"
    );
    assert_eq!(
        plan.skipped[7].to_string(),
        "Entry 8 (app::QuickOpen): its \"context\" is not a text in quotes"
    );
    assert_eq!(
        plan.skipped[1].to_string(),
        "Entry 2 (No command): it names no command"
    );

    // What is left is applied: the two entries without a default are
    // compared with the defaults of now.
    assert_eq!(plan.changes.len(), 2);
    let outcome = plan.outcome(ImportMode::Merge);
    assert_eq!(outcome.differences.len(), 2);
    let comments = Target::new("comments::Toggle", None);
    assert_eq!(
        outcome.changes.keys_of(keymap.defaults(), &comments),
        keys(&["cmd-alt-k"])
    );
    let find = Target::new("tools::Find", Some("EditorTools"));
    assert_eq!(
        outcome.changes.keys_of(keymap.defaults(), &find),
        keys(&["cmd-f", "cmd-alt-g"])
    );
}

/// A key that cannot be read is skipped with what is wrong with it, for
/// each way a key text can be wrong, in words for whoever edited the
/// file. (The report that found this: "hyper-e", one key with a modifier
/// the app does not know, was said to have "more than one key".)
#[test]
fn a_key_that_cannot_be_read_says_what_is_wrong_with_it() {
    let reason = |written: Value| -> String {
        let text = file(json!([
            {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": [written]},
        ]));
        let plan = keymap().plan_import(&text).unwrap();
        assert_eq!(plan.skipped.len(), 1, "{written}");
        assert!(plan.changes.is_empty(), "{written}");
        plan.skipped[0].reason.to_string()
    };
    assert_eq!(
        reason(json!("hyper-e")),
        "a key cannot be read: \"hyper-e\": \"hyper\" is not a modifier (the modifiers are cmd, \
         ctrl, alt (the option key), shift and fn)"
    );
    assert_eq!(
        reason(json!("cmd-entr")),
        "a key cannot be read: \"cmd-entr\": \"entr\" is not the name of a key (a key is one \
         character, or one of enter, tab, space, escape, backspace, delete, insert, up, down, \
         left, right, home, end, pageup, pagedown, or a function key like f5)"
    );
    assert_eq!(
        reason(json!("cmd+b")),
        "a key cannot be read: \"cmd+b\" is joined with +: modifiers and a key are joined by - \
         (a key is written like cmd-shift-p)"
    );
    assert_eq!(
        reason(json!("")),
        "a key cannot be read: an empty text stands where a key should (a key is written like \
         cmd-shift-p; a command without a key has an empty list)"
    );
    assert_eq!(
        reason(json!("cmd-")),
        "a key cannot be read: \"cmd-\" has no key after its modifiers (a key is written like \
         cmd-shift-p)"
    );
    assert_eq!(
        reason(json!("cmd-shift")),
        "a key cannot be read: \"cmd-shift\" is modifiers alone, without a key (a key is \
         written like cmd-shift-p)"
    );
    assert_eq!(
        reason(json!("cmd-a-b")),
        "a key cannot be read: \"cmd-a-b\" has more than one key in one keystroke: \"a\" and \
         \"b\" (modifiers and one key are joined by -, keys pressed one after the other are \
         separated by a space)"
    );
    // One keystroke of a sequence that is wrong: that keystroke is named.
    assert_eq!(
        reason(json!("cmd-k hyper-e")),
        "a key cannot be read: \"hyper-e\": \"hyper\" is not a modifier (the modifiers are cmd, \
         ctrl, alt (the option key), shift and fn)"
    );
    // Not a text at all.
    assert_eq!(
        reason(json!(7)),
        "its \"keys\" is not a list of keys in quotes (like [\"cmd-b\", \"f5\"])"
    );
    // A sequence is supported, and read.
    let text = file(json!([
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": ["cmd-k cmd-b"]},
    ]));
    let plan = keymap().plan_import(&text).unwrap();
    assert!(plan.skipped.is_empty());
    assert_eq!(plan.changes[0].keys, keys(&["cmd-k cmd-b"]));
    // A key in the entry's "default" is read the same way.
    let text = file(json!([
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["hyper-b"], "keys": ["cmd-b"]},
    ]));
    let plan = keymap().plan_import(&text).unwrap();
    assert!(matches!(
        plan.skipped[0].reason,
        SkipReason::UnreadableKey {
            field: "default",
            ..
        }
    ));
}

/// The commands of a part of the app that is not there (the assistant
/// without its engine) hold no key: a key of theirs is free, no conflict
/// with them is reported, an import counts none. Their own keys and the
/// user's changes to them are untouched, and when they are there again
/// the same keymap says what collides.
#[test]
fn the_keys_of_commands_that_are_not_there_count_for_nothing() {
    let quit = Target::new("app::Quit", None);
    let palette = Target::new("app::CommandPalette", None);
    let page = Target::new("editor::InsertPageBreak", Some("Editor"));
    let mut keymap = keymap();

    // All there: the palette's key is taken, and bold on the page
    // break's key too.
    assert_eq!(
        keymap.check(&quit, &key("cmd-shift-p"), None).kind(),
        VerdictKind::Taken
    );
    assert_eq!(
        keymap.check(&bold(), &key("cmd-enter"), None).kind(),
        VerdictKind::Taken
    );

    // The palette and the page break are not there.
    assert!(keymap.set_absent([palette.clone(), page.clone()]));
    assert!(!keymap.set_absent([palette.clone(), page.clone()]));
    assert!(keymap.is_absent(&palette));
    assert!(!keymap.is_absent(&quit));
    let verdict = keymap.check(&quit, &key("cmd-shift-p"), None);
    assert_eq!(verdict.kind(), VerdictKind::Fine, "{verdict:?}");
    let verdict = keymap.check(&bold(), &key("cmd-enter"), None);
    // The find bar's field has the key in another place, as before.
    assert_eq!(verdict.kind(), VerdictKind::UsedElsewhere);
    assert!(verdict.taken.is_empty());
    assert!(
        verdict.elsewhere.iter().all(|holder| holder.target != page),
        "{verdict:?}"
    );

    // The user takes both keys. No collision is reported ...
    assert!(keymap.add_key(&quit, key("cmd-shift-p")));
    assert!(keymap.replace_key(&bold(), &key("cmd-b"), key("cmd-enter")));
    assert!(keymap.collisions().is_empty());
    // ... the absent commands keep their keys, bound as before ...
    let effective = keymap.effective();
    assert_eq!(
        effective.target(&palette).unwrap().keys[0].keys,
        key("cmd-shift-p")
    );
    assert!(!effective.target(&palette).unwrap().changed);
    assert!(
        effective
            .bindings()
            .iter()
            .any(|binding| binding.target == page && binding.keys == key("cmd-enter"))
    );
    // ... and a change the user made to one of them before is kept, in
    // what is stored and in the exported file.
    let mut with_a_change = keymap.clone();
    assert!(with_a_change.replace_key(&palette, &key("cmd-shift-p"), key("f1")));
    let stored = with_a_change.stored().unwrap();
    assert!(stored.contains("app::CommandPalette"), "{stored}");
    let exported: Value =
        serde_json::from_str(&with_a_change.export(&catalogue(), "0.5.1")).unwrap();
    let entry = exported["shortcuts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["command"] == "app::CommandPalette")
        .expect("an absent command is in the file");
    assert_eq!(entry["keys"], json!(["f1"]));

    // They are there again: the keymap as it is says what collides.
    assert!(keymap.set_absent([]));
    let collisions = keymap.collisions();
    assert_eq!(collisions.len(), 2, "{collisions:#?}");
    assert!(
        collisions
            .iter()
            .any(|collision| collision.target == quit && collision.holder.target == palette)
    );
    assert!(
        collisions
            .iter()
            .any(|collision| collision.target == bold() && collision.holder.target == page)
    );
    assert_eq!(
        keymap.check(&history(), &key("cmd-shift-p"), None).kind(),
        VerdictKind::Taken
    );
}

/// An import counts no collision with a command that is not there, takes
/// the file's changes to such a command like every other, and lists the
/// collision once the command is there.
#[test]
fn an_import_counts_no_collision_with_a_command_that_is_not_there() {
    let palette = Target::new("app::CommandPalette", None);
    let text = file(json!([
        {"command": "app::VersionHistory", "context": null, "default": [], "keys": ["cmd-shift-p"]},
        {"command": "app::CommandPalette", "context": null, "default": ["cmd-shift-p"], "keys": ["cmd-shift-p", "f1"]},
    ]));
    let mut keymap = keymap();
    keymap.set_absent([palette.clone()]);
    let plan = keymap.plan_import(&text).unwrap();
    assert!(plan.skipped.is_empty());
    let outcome = plan.outcome(ImportMode::Merge);
    // Both changes are taken: the file means the same on every Mac.
    assert_eq!(outcome.differences.len(), 2);
    assert!(
        outcome
            .differences
            .iter()
            .any(|difference| difference.target == palette)
    );
    assert!(outcome.conflicts.is_empty(), "{:#?}", outcome.conflicts);

    keymap.set_absent([]);
    let plan = keymap.plan_import(&text).unwrap();
    let conflicts = &plan.outcome(ImportMode::Merge).conflicts;
    assert_eq!(conflicts.len(), 1, "{conflicts:#?}");
    assert_eq!(conflicts[0].target, history());
    assert_eq!(conflicts[0].holder.target, palette);
}

/// One rule for the question about a key and for the plan of an import:
/// for every command and every key of the keymap, the import of that one
/// change lists a collision exactly when the question would say "taken",
/// and with the same commands.
#[test]
fn the_question_about_a_key_and_the_plan_of_an_import_agree() {
    // The tools are only around the text, as in the app.
    let places = places().around("EditorTools", "Editor");
    let keymap = Keymap::new(defaults(), places);
    let effective = keymap.effective();
    let mut all_keys: Vec<KeySequence> = effective
        .bindings()
        .iter()
        .map(|binding| binding.keys.clone())
        .collect();
    all_keys.sort();
    all_keys.dedup();
    let mut taken_somewhere = 0;
    for state in effective.targets() {
        let target = &state.target;
        let default: Vec<String> = state.default_keys.iter().map(|k| k.to_string()).collect();
        for proposed in &all_keys {
            let verdict = keymap.check(target, proposed, None);
            if matches!(verdict.refusal, Some(Refusal::AlreadyHasKey)) {
                continue;
            }
            let mut with_it = default.clone();
            with_it.push(proposed.to_string());
            let text = file(json!([{
                "command": target.command.action,
                "context": target.context.text(),
                "default": default,
                "keys": with_it,
            }]));
            let plan = keymap.plan_import(&text).unwrap();
            if verdict.refusal.is_some() {
                // What the question refuses, the import skips.
                assert_eq!(plan.skipped.len(), 1, "{target} {proposed}");
                continue;
            }
            assert!(plan.skipped.is_empty(), "{target} {proposed}");
            let mut asked: Vec<(Target, KeySequence)> = verdict
                .taken
                .iter()
                .map(|holder| (holder.target.clone(), holder.keys.clone()))
                .collect();
            let mut planned: Vec<(Target, KeySequence)> = plan
                .outcome(ImportMode::Merge)
                .conflicts
                .iter()
                .map(|collision| {
                    assert_eq!(collision.target, *target, "{target} {proposed}");
                    assert_eq!(collision.key, *proposed);
                    (
                        collision.holder.target.clone(),
                        collision.holder.keys.clone(),
                    )
                })
                .collect();
            asked.sort();
            planned.sort();
            assert_eq!(asked, planned, "{target} with {proposed}");
            taken_somewhere += usize::from(!asked.is_empty());
        }
    }
    assert!(taken_somewhere > 20, "{taken_somewhere}");

    // The pair the report named: a command of the text on a key of the
    // tools around the text. Taken, and the import says the same.
    let find = Target::new("tools::Find", Some("EditorTools"));
    let verdict = keymap.check(&bold(), &key("cmd-f"), None);
    assert_eq!(verdict.kind(), VerdictKind::Taken);
    assert_eq!(verdict.taken[0].target, find);
    assert_eq!(verdict.taken[0].overlap, Overlap::SamePlace);
    // The other way round too: a tool on a key of the text.
    let verdict = keymap.check(&find, &key("cmd-b"), None);
    assert_eq!(verdict.kind(), VerdictKind::Taken);
    assert_eq!(verdict.taken[0].target, bold());
}

#[test]
fn an_import_says_what_would_collide() {
    // The file gives bold the page break's key and the version history
    // the palette's; the page break keeps its own.
    let text = file(json!([
        {"command": "editor::ToggleBold", "context": "Editor", "default": ["cmd-b"], "keys": ["cmd-enter"]},
        {"command": "app::VersionHistory", "context": null, "default": [], "keys": ["cmd-shift-p"]},
        {"command": "app::Quit", "context": null, "default": ["cmd-q"], "keys": ["cmd-alt-q"]},
    ]));
    let keymap = keymap();
    let plan = keymap.plan_import(&text).unwrap();
    assert!(plan.skipped.is_empty());
    let outcome = plan.outcome(ImportMode::Replace);
    assert_eq!(outcome.differences.len(), 3);
    assert_eq!(outcome.conflicts.len(), 2, "{:#?}", outcome.conflicts);
    let history_conflict = &outcome.conflicts[0];
    assert_eq!(history_conflict.target, history());
    assert_eq!(history_conflict.key, key("cmd-shift-p"));
    assert_eq!(
        history_conflict.holder.target,
        Target::new("app::CommandPalette", None)
    );
    assert_eq!(history_conflict.holder.overlap, Overlap::SamePlace);
    let bold_conflict = &outcome.conflicts[1];
    assert_eq!(bold_conflict.target, bold());
    assert_eq!(
        bold_conflict.holder.target,
        Target::new("editor::InsertPageBreak", Some("Editor"))
    );

    // A conflict the user has already is not the import's.
    let mut mine = keymap.clone();
    mine.replace_key(&bold(), &key("cmd-b"), key("cmd-enter"));
    assert_eq!(mine.collisions().len(), 1);
    let plan = mine.plan_import(&text).unwrap();
    assert_eq!(plan.outcome(ImportMode::Merge).conflicts.len(), 1);
    assert_eq!(
        plan.outcome(ImportMode::Merge).conflicts[0].target,
        history()
    );
}

#[test]
fn texts_that_are_not_such_a_file() {
    let keymap = keymap();
    let error = |text: &str| keymap.plan_import(text).unwrap_err();

    // Not JSON: the line and the column.
    let not_json = error("{\n  \"format\": \"the-writer-shortcuts\",\n  \"version\": 1,,\n}");
    match &not_json {
        ImportError::NotJson {
            line,
            column,
            problem,
        } => {
            assert_eq!((*line, *column), (3, 16));
            assert!(!problem.contains("line"), "{problem}");
        }
        other => panic!("{other:?}"),
    }
    let message = not_json.to_string();
    assert!(
        message.starts_with("This is not a shortcuts file: it is not JSON. Line 3, column 16: "),
        "{message}"
    );
    assert!(matches!(error(""), ImportError::NotJson { line: 1, .. }));
    assert!(matches!(
        error("# Chapter one\n\nIt was a dark"),
        ImportError::NotJson {
            line: 1,
            column: 1,
            ..
        }
    ));

    // JSON, but something else.
    assert_eq!(error("[1, 2, 3]"), ImportError::NotAnObject);
    assert_eq!(
        error("{\"name\": \"package\", \"version\": \"1.0.0\"}"),
        ImportError::NoFormat
    );
    assert_eq!(
        error("{\"format\": \"zed-keymap\", \"version\": 1, \"shortcuts\": []}"),
        ImportError::OtherFormat {
            found: "zed-keymap".to_string()
        }
    );
    assert!(
        error("{\"format\": \"zed-keymap\"}")
            .to_string()
            .contains("\"zed-keymap\"")
    );
    assert_eq!(
        error("{\"format\": 3, \"version\": 1}"),
        ImportError::OtherFormat {
            found: "3".to_string()
        }
    );

    // The right format, not readable.
    assert_eq!(
        error("{\"format\": \"the-writer-shortcuts\", \"shortcuts\": []}"),
        ImportError::BadVersion
    );
    assert_eq!(
        error("{\"format\": \"the-writer-shortcuts\", \"version\": \"1\", \"shortcuts\": []}"),
        ImportError::BadVersion
    );
    let newer = error("{\"format\": \"the-writer-shortcuts\", \"version\": 2, \"shortcuts\": []}");
    assert_eq!(
        newer,
        ImportError::NewerVersion {
            found: 2,
            supported: 1
        }
    );
    assert!(newer.to_string().contains("later version of The Writer"));
    assert_eq!(
        error("{\"format\": \"the-writer-shortcuts\", \"version\": 1}"),
        ImportError::NoShortcuts
    );
    assert_eq!(
        error("{\"format\": \"the-writer-shortcuts\", \"version\": 1, \"shortcuts\": {}}"),
        ImportError::NoShortcuts
    );

    // An empty list is a file that changes nothing.
    let plan = keymap
        .plan_import("{\"format\": \"the-writer-shortcuts\", \"version\": 1, \"shortcuts\": []}")
        .unwrap();
    assert_eq!(plan.entries, 0);
    assert_eq!(plan.app_version, None);
    assert!(plan.outcome(ImportMode::Replace).differences.is_empty());
}

#[test]
fn a_file_without_changes_replaces_the_user_s_own_with_nothing() {
    let pristine = keymap();
    let text = pristine.export(&catalogue(), "0.5.0");
    let mut mine = changed_keymap();
    let plan = mine.plan_import(&text).unwrap();
    assert!(plan.changes.is_empty());
    assert!(plan.modes_differ());
    assert_eq!(plan.outcome(ImportMode::Replace).differences.len(), 6);
    assert!(plan.outcome(ImportMode::Merge).differences.is_empty());
    mine.import(&plan, ImportMode::Replace);
    assert_eq!(mine.stored(), None);
    assert_eq!(apply(mine.defaults(), &Changes::new()), mine.effective());
}

// ---------------------------------------------------------------------
// Many changes, made up
// ---------------------------------------------------------------------

#[test]
fn whatever_is_changed_the_keymap_stays_whole() {
    let mut numbers = Texts(0x2545_F491_4F6C_DD1D);
    let mut pick = |below: usize| -> usize {
        numbers.0 ^= numbers.0 << 13;
        numbers.0 ^= numbers.0 >> 7;
        numbers.0 ^= numbers.0 << 17;
        (numbers.0 % below as u64) as usize
    };
    let pool = keys(&[
        "cmd-b",
        "cmd-alt-b",
        "cmd-shift-7",
        "cmd-alt-shift-7",
        "f5",
        "f6",
        "ctrl-b",
        "left",
        "cmd-enter",
        "cmd-p",
        "cmd-k cmd-b",
        "cmd-g g",
        "alt-h",
        "cmd-shift-;",
        "ctrl-enter",
        "cmd-q",
    ]);
    let pristine = keymap();
    let targets: Vec<Target> = pristine.defaults().targets().to_vec();
    let catalogue = catalogue();

    for _ in 0..60 {
        let mut keymap = keymap();
        for _ in 0..25 {
            let target = &targets[pick(targets.len())];
            let has = keymap.changes().keys_of(keymap.defaults(), target);
            let new = pool[pick(pool.len())].clone();
            match pick(6) {
                0 if !has.is_empty() => {
                    let old = has[pick(has.len())].clone();
                    keymap.replace_key(target, &old, new);
                }
                1 if !has.is_empty() => {
                    let old = has[pick(has.len())].clone();
                    keymap.remove_key(target, &old);
                }
                2 => {
                    let wanted: Vec<KeySequence> = (0..pick(3))
                        .map(|_| pool[pick(pool.len())].clone())
                        .collect();
                    keymap.set_keys(target, &wanted);
                }
                3 => {
                    keymap.reset(target);
                }
                _ => {
                    keymap.add_key(target, new);
                }
            }

            let effective = keymap.effective();
            // Every target is there, and its row is what the changes say.
            assert_eq!(effective.targets().len(), targets.len());
            let mut expected: Vec<(Target, KeySequence)> = Vec::new();
            for row in effective.targets() {
                let row_keys: Vec<KeySequence> =
                    row.keys.iter().map(|key| key.keys.clone()).collect();
                assert_eq!(
                    row_keys,
                    keymap.changes().keys_of(keymap.defaults(), &row.target)
                );
                assert_eq!(
                    row.changed,
                    keymap.changes().is_changed(keymap.defaults(), &row.target)
                );
                // No key twice in a row; a changed row differs from its defaults.
                for (ix, one) in row_keys.iter().enumerate() {
                    assert!(!row_keys[ix + 1..].contains(one), "{row:?}");
                }
                let mut sorted = (row_keys.clone(), row.default_keys.clone());
                sorted.0.sort();
                sorted.1.sort();
                assert_eq!(row.changed, sorted.0 != sorted.1, "{row:?}");
                assert_eq!(row.shown.is_some(), !row_keys.is_empty());
                for keys in row_keys {
                    expected.push((row.target.clone(), keys));
                }
            }
            // What is bound is what the rows show, each key in all its
            // spellings, side by side.
            let mut bound: Vec<(Target, KeySequence)> = Vec::new();
            for binding in effective.bindings() {
                assert!(
                    binding.keys.spellings().contains(&binding.spelling)
                        || binding.origin == Origin::Default
                );
                assert_eq!(key(&binding.spelling), binding.keys);
                let pair = (binding.target.clone(), binding.keys.clone());
                if bound.last() != Some(&pair) {
                    assert!(
                        !bound.contains(&pair),
                        "a key bound in two places: {pair:?}"
                    );
                    bound.push(pair);
                }
            }
            expected.sort();
            bound.sort();
            assert_eq!(bound, expected);
            // The defaults nobody changed keep their order.
            let order: Vec<usize> = effective
                .bindings()
                .iter()
                .filter(|binding| binding.origin == Origin::Default)
                .map(|binding| binding.source.unwrap())
                .collect();
            assert!(order.windows(2).all(|pair| pair[0] < pair[1]));
            // An entry is stored for a changed target and for no other.
            assert_eq!(
                keymap.changes().entries().count(),
                effective.changed().count()
            );
            assert!(effective.inactive().is_empty());
        }

        // Stored and read back: the same.
        let (again, error) = Keymap::with_stored(defaults(), places(), keymap.stored().as_deref());
        assert_eq!(error, None);
        assert_eq!(again, keymap);
        // Through a file into a fresh keymap: the same keymap, binding
        // for binding, whatever the guard rails think of the made-up keys
        // (none of the pool is refused).
        let text = keymap.export(&catalogue, "0.5.0");
        let mut fresh = pristine.clone();
        let plan = fresh.plan_import(&text).unwrap();
        assert!(plan.skipped.is_empty(), "{:?}", plan.skipped);
        fresh.import(&plan, ImportMode::Replace);
        assert_eq!(fresh.effective(), keymap.effective());
        assert_eq!(entries(fresh.changes()), entries(keymap.changes()));
        // And into itself it changes nothing, either way.
        let plan = keymap.plan_import(&text).unwrap();
        assert!(plan.outcome(ImportMode::Replace).differences.is_empty());
        assert!(plan.outcome(ImportMode::Merge).differences.is_empty());
    }
}
