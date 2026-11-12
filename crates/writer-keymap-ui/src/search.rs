//! Searching the list: by what a command is called, by its category and
//! place, and by key.
//!
//! A query is read both ways at once. As words it is matched against the
//! title (fuzzily, the matched letters are highlighted) and against the
//! heading and the category (every word has to occur). As a key it is read
//! the way a person writes one: `cmd-b`, `cmd+b`, `⌘B`, `opt-up`, `esc`
//! (a space separates keys pressed one after the other: `g g`). A key in the query finds the commands that have it,
//! then those whose key begins with it (`cmd-b` also finds `cmd-backspace`),
//! and modifiers alone (`cmd-shift`) find every key with them held.

use writer_keymap::{KeySequence, Modifiers};
use writer_ui::fuzzy::fuzzy_match;

use crate::listing::Entry;

/// A row that matches, with the letters of its title to highlight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Match {
    pub entry: usize,
    pub positions: Vec<usize>,
}

/// One keystroke of a query: the modifiers named, and what was written of
/// the key (nothing yet for `cmd-`).
#[derive(Clone, Debug, PartialEq, Eq)]
struct StrokeQuery {
    modifiers: Modifiers,
    key: Option<String>,
}

/// A query read as a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct KeyQuery {
    strokes: Vec<StrokeQuery>,
}

/// The glyphs the app shows for keys, as the names a key is written with.
const GLYPHS: [(char, &str); 17] = [
    ('⌘', "cmd-"),
    ('⌥', "alt-"),
    ('⇧', "shift-"),
    ('⌃', "ctrl-"),
    ('^', "ctrl-"),
    ('↩', "enter"),
    ('⌫', "backspace"),
    ('⌦', "delete"),
    ('⇥', "tab"),
    ('⎋', "escape"),
    ('↑', "up"),
    ('↓', "down"),
    ('←', "left"),
    ('→', "right"),
    ('⇞', "pageup"),
    ('⇟', "pagedown"),
    ('↖', "home"),
];

/// Keys with a name, which a query can mean without a modifier.
const NAMED_KEYS: [&str; 16] = [
    "enter",
    "escape",
    "tab",
    "space",
    "backspace",
    "delete",
    "up",
    "down",
    "left",
    "right",
    "home",
    "end",
    "pageup",
    "pagedown",
    "insert",
    "return",
];

fn modifier(word: &str) -> Option<fn(&mut Modifiers)> {
    Some(match word {
        "cmd" | "command" | "super" | "win" | "secondary" => |m: &mut Modifiers| m.command = true,
        "ctrl" | "control" => |m: &mut Modifiers| m.control = true,
        "alt" | "opt" | "option" => |m: &mut Modifiers| m.alt = true,
        "shift" => |m: &mut Modifiers| m.shift = true,
        "fn" => |m: &mut Modifiers| m.function = true,
        _ => return None,
    })
}

fn key_name(word: &str) -> String {
    match word {
        "esc" => "escape".to_string(),
        "return" => "enter".to_string(),
        other => other.to_string(),
    }
}

fn is_function_key(word: &str) -> bool {
    word.strip_prefix('f')
        .is_some_and(|n| !n.is_empty() && n.parse::<u8>().is_ok())
}

/// Reads `query` as a key, if it can be one: every keystroke in it names a
/// modifier, or is a single character, or a key with a name.
pub(crate) fn parse_key_query(query: &str) -> Option<KeyQuery> {
    let mut text = String::new();
    for ch in query.trim().chars() {
        match GLYPHS.iter().find(|(glyph, _)| *glyph == ch) {
            Some((_, name)) => {
                // "⌘↩" has no separator between the modifier and the key,
                // "↩" after a letter would not be a key at all.
                text.push_str(name);
            }
            None if ch == '↘' => text.push_str("end"),
            None => text.extend(ch.to_lowercase()),
        }
    }
    let mut strokes = Vec::new();
    for word in text.split_whitespace() {
        let mut modifiers = Modifiers::default();
        let mut key: Option<String> = None;
        // `cmd+b` is `cmd-b`; a lone `+` or `-`, and one after a separator
        // (`cmd--`), is the key itself.
        let mut parts: Vec<&str> = Vec::new();
        let mut rest = word;
        while !rest.is_empty() {
            match rest.find(['-', '+']) {
                Some(0) => {
                    parts.push(&rest[..1]);
                    rest = &rest[1..];
                    // What follows a key cannot be read.
                    if !rest.is_empty() {
                        return None;
                    }
                }
                Some(at) => {
                    parts.push(&rest[..at]);
                    rest = &rest[at + 1..];
                }
                None => {
                    parts.push(rest);
                    rest = "";
                }
            }
        }
        let mut named_modifier = false;
        for part in parts {
            if let Some(set) = modifier(part) {
                set(&mut modifiers);
                named_modifier = true;
            } else if key.is_none() {
                key = Some(key_name(part));
            } else {
                return None;
            }
        }
        let plausible = match &key {
            None => named_modifier,
            Some(key) => {
                named_modifier
                    || key.chars().count() == 1
                    || NAMED_KEYS.contains(&key.as_str())
                    || is_function_key(key)
            }
        };
        if !plausible {
            return None;
        }
        strokes.push(StrokeQuery { modifiers, key });
    }
    (!strokes.is_empty()).then_some(KeyQuery { strokes })
}

/// How well a key answers a key query: 3 it is the key, 2 it begins with
/// it, 1 it has the modifiers.
fn key_rank(query: &KeyQuery, keys: &KeySequence) -> u8 {
    let strokes = keys.strokes();
    if query.strokes.len() > strokes.len() {
        return 0;
    }
    let mut rank = 3;
    for (ix, wanted) in query.strokes.iter().enumerate() {
        let stroke = &strokes[ix];
        let have = stroke.modifiers();
        let last = ix + 1 == query.strokes.len();
        match &wanted.key {
            Some(key) if have == wanted.modifiers && stroke.key() == key => {}
            Some(key)
                if last && have == wanted.modifiers && stroke.key().starts_with(key.as_str()) =>
            {
                rank = rank.min(2);
            }
            None if last
                && (!wanted.modifiers.control || have.control)
                && (!wanted.modifiers.alt || have.alt)
                && (!wanted.modifiers.shift || have.shift)
                && (!wanted.modifiers.command || have.command)
                && (!wanted.modifiers.function || have.function) =>
            {
                rank = rank.min(1);
            }
            _ => return 0,
        }
    }
    if rank == 3 && query.strokes.len() < strokes.len() {
        // The first keys of a longer sequence.
        rank = 2;
    }
    rank
}

/// Whether every word of `query` occurs in `haystack`, whatever the case.
fn words_match(query: &str, haystack: &str) -> bool {
    let haystack = haystack.to_lowercase();
    query
        .split_whitespace()
        .all(|word| haystack.contains(&word.to_lowercase()))
}

/// The rows of `entries` that answer `query`, best first; rows that answer
/// equally well keep the order of the list. `changed_only` leaves out the
/// rows the user did not change. An empty query matches every row, in
/// order.
pub(crate) fn search(entries: &[Entry], query: &str, changed_only: bool) -> Vec<Match> {
    let query = query.trim();
    let candidates = entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| !changed_only || entry.changed);
    if query.is_empty() {
        return candidates
            .map(|(entry, _)| Match {
                entry,
                positions: Vec::new(),
            })
            .collect();
    }
    let key_query = parse_key_query(query);
    let mut found: Vec<(i32, Match)> = candidates
        .filter_map(|(index, entry)| {
            let mut score: Option<i32> = None;
            let mut positions = Vec::new();
            if let Some(key_query) = &key_query {
                let now = entry
                    .keys
                    .iter()
                    .map(|key| key_rank(key_query, &key.keys))
                    .max()
                    .unwrap_or(0);
                // What a changed command had by default is found too, after
                // the commands that have the key now.
                let before = entry
                    .default_keys
                    .iter()
                    .map(|key| key_rank(key_query, key))
                    .max()
                    .unwrap_or(0);
                score = match (now, before) {
                    (3, _) => Some(40_000),
                    (2, _) => Some(30_000),
                    (_, 3) => Some(8_000),
                    (1, _) => Some(6_000),
                    (_, 2) => Some(4_000),
                    _ => None,
                };
            }
            if let Some(matched) = fuzzy_match(query, &entry.row.title) {
                let text = 10_000 + matched.score;
                if score.is_none_or(|score| text > score) {
                    score = Some(text);
                }
                positions = matched.positions;
            } else if score.is_none() {
                let haystack = format!(
                    "{} {} {}",
                    entry.row.heading, entry.row.category, entry.row.title
                );
                if words_match(query, &haystack) {
                    score = Some(0);
                }
            }
            score.map(|score| {
                (
                    score,
                    Match {
                        entry: index,
                        positions,
                    },
                )
            })
        })
        .collect();
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.entry.cmp(&b.1.entry)));
    found.into_iter().map(|(_, matched)| matched).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::{CommandRow, Listing, entries};
    use writer_keymap::{DefaultBinding, Defaults, Keymap, Places, Target};

    fn listing() -> Vec<Entry> {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let defaults = Defaults::from_bindings([
            binding("app::Palette", None, "cmd-shift-p"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::DeleteLine", Some("Editor"), "cmd-backspace"),
            binding("editor::Newline", Some("Editor"), "enter"),
            binding("editor::Numbered", Some("Editor"), "cmd-&"),
            binding("editor::Numbered", Some("Editor"), "cmd-shift-7"),
            binding("library::First", Some("LibraryPanel"), "g g"),
            binding("editor::MoveUp", Some("Editor"), "alt-up"),
        ]);
        let mut keymap = Keymap::new(defaults, Places::new().typing("Editor"));
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        keymap.replace_key(
            &bold,
            &KeySequence::parse("cmd-b").unwrap(),
            KeySequence::parse("cmd-alt-b").unwrap(),
        );
        let row = |action: &str,
                   context: Option<&str>,
                   title: &'static str,
                   place: &'static str,
                   category: &'static str| {
            CommandRow::new(Target::new(action, context), title, place).category(category)
        };
        entries(&Listing::new(
            keymap,
            vec![
                row("app::Palette", None, "Command palette", "Anywhere", "View"),
                row(
                    "editor::ToggleBold",
                    Some("Editor"),
                    "Bold",
                    "In the text",
                    "Format",
                ),
                row(
                    "editor::DeleteLine",
                    Some("Editor"),
                    "Delete to line start",
                    "In the text",
                    "",
                ),
                row(
                    "editor::Newline",
                    Some("Editor"),
                    "New paragraph",
                    "In the text",
                    "",
                ),
                row(
                    "editor::Numbered",
                    Some("Editor"),
                    "Numbered list",
                    "In the text",
                    "Format",
                ),
                row(
                    "library::First",
                    Some("LibraryPanel"),
                    "First row",
                    "In the library",
                    "",
                ),
                row(
                    "editor::MoveUp",
                    Some("Editor"),
                    "Move block up",
                    "In the text",
                    "",
                ),
            ],
        ))
    }

    fn titles(entries: &[Entry], query: &str) -> Vec<String> {
        search(entries, query, false)
            .into_iter()
            .map(|matched| entries[matched.entry].row.title.to_string())
            .collect()
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let entries = listing();
        assert_eq!(search(&entries, "  ", false).len(), entries.len());
        assert_eq!(titles(&entries, "")[0], "Command palette");
    }

    #[test]
    fn a_title_is_found_by_its_letters() {
        let entries = listing();
        assert_eq!(titles(&entries, "bold"), vec!["Bold"]);
        assert_eq!(titles(&entries, "numb"), vec!["Numbered list"]);
    }

    #[test]
    fn a_heading_and_a_category_are_found_by_their_words() {
        let entries = listing();
        assert_eq!(titles(&entries, "library"), vec!["First row"]);
        let format = titles(&entries, "format");
        assert!(
            format.contains(&"Bold".to_string()) && format.contains(&"Numbered list".to_string())
        );
    }

    #[test]
    fn a_key_is_found_as_it_is_written() {
        let entries = listing();
        // The key Bold has now, in every way a person writes it.
        for query in [
            "cmd-alt-b",
            "alt-cmd-b",
            "cmd+opt+b",
            "⌥⌘B",
            "⌘⌥b",
            "option-command-b",
        ] {
            assert_eq!(titles(&entries, query)[0], "Bold", "{query}");
        }
        assert_eq!(titles(&entries, "⇧⌘P"), vec!["Command palette"]);
        assert_eq!(titles(&entries, "cmd-shift-p"), vec!["Command palette"]);
    }

    #[test]
    fn a_key_finds_the_exact_key_before_keys_that_begin_with_it() {
        let entries = listing();
        // cmd-b was Bold's default: found, after the key that begins with it.
        assert_eq!(
            titles(&entries, "cmd-b"),
            vec!["Delete to line start", "Bold"]
        );
    }

    #[test]
    fn the_two_spellings_of_a_key_are_one() {
        let entries = listing();
        assert_eq!(titles(&entries, "cmd-shift-7"), vec!["Numbered list"]);
        assert_eq!(titles(&entries, "⇧⌘7"), vec!["Numbered list"]);
    }

    #[test]
    fn modifiers_alone_find_every_key_with_them() {
        let entries = listing();
        assert_eq!(
            titles(&entries, "cmd-shift"),
            vec!["Command palette", "Numbered list"]
        );
        assert_eq!(titles(&entries, "⌥"), vec!["Bold", "Move block up"]);
    }

    #[test]
    fn named_keys_and_sequences_are_keys() {
        let entries = listing();
        assert_eq!(titles(&entries, "enter")[0], "New paragraph");
        assert_eq!(titles(&entries, "↩")[0], "New paragraph");
        assert_eq!(titles(&entries, "g g")[0], "First row");
        assert_eq!(titles(&entries, "opt-up"), vec!["Move block up"]);
        assert_eq!(titles(&entries, "⌥↑"), vec!["Move block up"]);
    }

    #[test]
    fn words_are_not_taken_for_keys() {
        assert!(parse_key_query("bold").is_none());
        assert!(parse_key_query("move block").is_none());
        assert!(parse_key_query("cmd-b").is_some());
        assert!(parse_key_query("cmd-").is_some());
        assert!(parse_key_query("cmd--").is_some());
        assert!(parse_key_query("f5").is_some());
        assert!(parse_key_query("x").is_some());
    }

    #[test]
    fn changed_only_leaves_the_rest_out() {
        let entries = listing();
        let changed: Vec<usize> = search(&entries, "", true)
            .into_iter()
            .map(|matched| matched.entry)
            .collect();
        assert_eq!(changed, vec![1]);
        assert!(search(&entries, "palette", true).is_empty());
    }
}
