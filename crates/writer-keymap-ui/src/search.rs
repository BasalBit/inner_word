//! Searching the list: by what a command is called, by its category and
//! place, and by key.
//!
//! A query is read both ways at once.
//!
//! **As words.** Every word of the query has to stand in the row as it is
//! written, letters together: in the title, at the start of a word or
//! inside one, or else at the start of a word of the heading or the
//! category ("find bar case").
//! Letters scattered over several words are not a match ("italic" does not
//! find "Insert menu (at the start of a line ...)"). Rows whose title has
//! every word at the start of a word come first, then those with a word
//! inside a word, then those that needed the heading. The letters of the
//! title that matched are the ones highlighted.
//!
//! **A small typing mistake** is forgiven only when nothing matches as
//! written, by name or by key: then a word of four letters or more may be
//! one mistake away from the start of a word of the title (one letter
//! wrong, one missing, one too many, or two neighbours swapped; the first
//! letter has to be right). [`Found::close`] says that this happened, and
//! the view says so above the rows.
//!
//! **As a key**, the way a person writes one: `cmd-b`, `cmd+b`, `⌘B`,
//! `opt-up`, `esc` (a space separates keys pressed one after the other:
//! `g g`). A key in the query finds the commands that have it, now or by
//! default. Somebody who wrote a whole key means that key: the keys that
//! only begin with it (`cmd-backspace` for `cmd-b`) are listed only when
//! no command has the key itself. An unfinished key (`cmd-ba`) finds the
//! keys that begin with it, and modifiers alone (`cmd-shift`) find every
//! key with them held.

use writer_keymap::{KeySequence, Modifiers};

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

impl KeyQuery {
    /// Whether the query names a whole key: its last keystroke ends in a
    /// key a keyboard has (a character, a key with a name, a function
    /// key), not in the first letters of one (`cmd-ba`) or in modifiers
    /// alone (`cmd-`).
    pub(crate) fn is_complete(&self) -> bool {
        self.strokes
            .last()
            .and_then(|stroke| stroke.key.as_deref())
            .is_some_and(|key| {
                key.chars().count() == 1 || NAMED_KEYS.contains(&key) || is_function_key(key)
            })
    }
}

/// What a search found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The rows, best first.
    pub matches: Vec<Match>,
    /// Nothing matched the query as it is written: the rows are those one
    /// typing mistake away.
    pub close: bool,
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

/// A text letter by letter, without regard to case, with where each
/// letter stands in the text as it was (a byte offset).
struct Letters {
    letters: Vec<char>,
    offsets: Vec<usize>,
}

impl Letters {
    fn of(text: &str) -> Self {
        let mut letters = Vec::with_capacity(text.len());
        let mut offsets = Vec::with_capacity(text.len());
        for (offset, ch) in text.char_indices() {
            // One letter stays one letter, so the offsets stay true.
            let mut lower = ch.to_lowercase();
            letters.push(match (lower.next(), lower.next()) {
                (Some(folded), None) => folded,
                _ => ch,
            });
            offsets.push(offset);
        }
        Self { letters, offsets }
    }

    fn starts_word(&self, at: usize) -> bool {
        at == 0 || !self.letters[at - 1].is_alphanumeric()
    }

    /// Where `word` stands, letters together: at the start of a word if it
    /// does anywhere, else inside one. The position of its first letter,
    /// and whether that starts a word.
    fn find(&self, word: &[char]) -> Option<(usize, bool)> {
        if word.is_empty() || word.len() > self.letters.len() {
            return None;
        }
        let mut inside = None;
        for at in 0..=self.letters.len() - word.len() {
            if self.letters[at..at + word.len()] != *word {
                continue;
            }
            if self.starts_word(at) {
                return Some((at, true));
            }
            inside.get_or_insert(at);
        }
        inside.map(|at| (at, false))
    }

    /// The start of a word that `word` is one typing mistake away from,
    /// with the letters of the text that it has right.
    fn find_close(&self, word: &[char]) -> Option<Vec<usize>> {
        (0..self.letters.len())
            .filter(|at| self.starts_word(*at) && self.letters[*at].is_alphanumeric())
            .find_map(|at| {
                let end = self.letters[at..]
                    .iter()
                    .position(|ch| !ch.is_alphanumeric())
                    .map_or(self.letters.len(), |length| at + length);
                one_mistake(word, &self.letters[at..end])
                    .map(|right| right.into_iter().map(|ix| at + ix).collect())
            })
    }
}

/// How short a word may be and still be forgiven a typing mistake: in a
/// shorter one a wrong letter leaves too little to go by.
const SHORTEST_FORGIVEN: usize = 4;

/// Whether `typed` is the beginning of `word` but for one typing mistake:
/// one letter wrong, one missing, one too many, or two neighbours swapped.
/// The first letter has to be right. Gives the letters of `word` (their
/// positions) that `typed` has right.
fn one_mistake(typed: &[char], word: &[char]) -> Option<Vec<usize>> {
    if typed.len() < SHORTEST_FORGIVEN {
        return None;
    }
    let same = typed
        .iter()
        .zip(word)
        .take_while(|(typed, word)| typed == word)
        .count();
    if same == 0 || same == typed.len() {
        // The first letter is wrong, or nothing is: not a mistake to
        // forgive here.
        return None;
    }
    let rest_of_typed = &typed[same..];
    let rest_of_word = &word[same.min(word.len())..];
    let all_but = |skip: Option<usize>, upto: usize| -> Vec<usize> {
        (0..upto).filter(|ix| Some(*ix) != skip).collect()
    };
    // Two neighbours swapped.
    if rest_of_typed.len() >= 2
        && rest_of_word.len() >= 2
        && rest_of_typed[0] == rest_of_word[1]
        && rest_of_typed[1] == rest_of_word[0]
        && rest_of_word[2..].starts_with(&rest_of_typed[2..])
    {
        return Some(all_but(None, typed.len()));
    }
    // One letter wrong.
    if !rest_of_word.is_empty() && rest_of_word[1..].starts_with(&rest_of_typed[1..]) {
        return Some(all_but(Some(same), typed.len()));
    }
    // One letter too many.
    if rest_of_word.starts_with(&rest_of_typed[1..]) {
        return Some(all_but(None, typed.len() - 1));
    }
    // One letter missing.
    if !rest_of_word.is_empty() && rest_of_word[1..].starts_with(rest_of_typed) {
        return Some(all_but(Some(same), typed.len() + 1));
    }
    None
}

/// How well the words of a query stand in a row, and the letters of its
/// title that matched (byte offsets, ascending).
struct NameMatch {
    score: i32,
    positions: Vec<usize>,
}

/// The words of `query` in the row called `title` under `heading` and
/// `category`: every word has to stand in the title, or else in the
/// heading or the category. With `forgiving`, a word that stands nowhere
/// may be one typing mistake away from the start of a word of the title.
fn match_name(
    words: &[Vec<char>],
    title: &str,
    heading: &str,
    category: &str,
    forgiving: bool,
) -> Option<NameMatch> {
    if words.is_empty() {
        return None;
    }
    let name = Letters::of(title);
    let around = Letters::of(&format!("{heading} {category}"));
    let mut positions: Vec<usize> = Vec::new();
    let mut in_title = 0;
    let mut at_word_starts = 0;
    let mut mistakes = 0;
    for word in words {
        if let Some((at, starts_word)) = name.find(word) {
            positions.extend(&name.offsets[at..at + word.len()]);
            in_title += 1;
            at_word_starts += usize::from(starts_word);
        } else if around
            .find(word)
            .is_some_and(|(_, starts_word)| starts_word)
        {
            // Found where the row belongs, as a word or the start of one
            // ("form" for "Format", not "or"): nothing of the title to
            // mark.
        } else if forgiving && let Some(right) = name.find_close(word) {
            positions.extend(right.into_iter().map(|ix| name.offsets[ix]));
            in_title += 1;
            mistakes += 1;
        } else {
            return None;
        }
    }
    positions.sort_unstable();
    positions.dedup();
    // Every word at the start of a word of the title; every word in the
    // title; the title and where the row belongs; where it belongs alone.
    let tier = if in_title == words.len() && at_word_starts == words.len() {
        3
    } else if in_title == words.len() {
        2
    } else if in_title > 0 {
        1
    } else {
        0
    };
    let phrase: Vec<char> = words.join(&' ');
    let whole = match name.find(&phrase) {
        _ if mistakes > 0 => 0,
        Some((0, _)) if name.letters.len() == phrase.len() => 400,
        Some((0, _)) => 200,
        Some(_) => 100,
        None => 0,
    };
    Some(NameMatch {
        score: tier * 1000 + whole,
        positions,
    })
}

/// The rows of `entries` that answer `query`, best first; rows that answer
/// equally well keep the order of the list. `changed_only` leaves out the
/// rows the user did not change. An empty query matches every row, in
/// order. See the [module docs](self) for what answers a query.
pub(crate) fn search(entries: &[Entry], query: &str, changed_only: bool) -> Found {
    let query = query.trim();
    let candidates = || {
        entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| !changed_only || entry.changed)
    };
    if query.is_empty() {
        return Found {
            matches: candidates()
                .map(|(entry, _)| Match {
                    entry,
                    positions: Vec::new(),
                })
                .collect(),
            close: false,
        };
    }
    let words: Vec<Vec<char>> = query
        .split_whitespace()
        .map(|word| Letters::of(word).letters)
        .collect();
    let key_query = parse_key_query(query);
    // How well each row's keys answer the query read as a key: the keys
    // it has now, and the keys it had by default.
    let ranks: Vec<(usize, u8, u8)> = candidates()
        .map(|(index, entry)| {
            let rank = |keys: &mut dyn Iterator<Item = &KeySequence>| {
                key_query.as_ref().map_or(0, |key_query| {
                    keys.map(|key| key_rank(key_query, key)).max().unwrap_or(0)
                })
            };
            (
                index,
                rank(&mut entry.keys.iter().map(|key| &key.keys)),
                rank(&mut entry.default_keys.iter()),
            )
        })
        .collect();
    // Somebody who wrote a whole key means that key: the keys that only
    // begin with it are of interest when no row has the key itself.
    let exact_only = key_query.as_ref().is_some_and(KeyQuery::is_complete)
        && ranks
            .iter()
            .any(|(_, now, before)| *now == 3 || *before == 3);
    let find = |forgiving: bool| -> Vec<(i32, Match)> {
        ranks
            .iter()
            .filter_map(|(index, now, before)| {
                let entry = &entries[*index];
                let by_key = match (*now, *before) {
                    (3, _) => Some(40_000),
                    (2, _) if !exact_only => Some(30_000),
                    // What a changed command had by default is found too,
                    // after the commands that have the key now.
                    (_, 3) => Some(8_000),
                    (1, _) => Some(6_000),
                    (_, 2) if !exact_only => Some(4_000),
                    _ => None,
                };
                let by_name = match_name(
                    &words,
                    &entry.row.title,
                    &entry.row.heading,
                    &entry.row.category,
                    forgiving,
                );
                let score = match (by_key, &by_name) {
                    (Some(key), Some(name)) => Some(key.max(10_000 + name.score)),
                    (Some(key), None) => Some(key),
                    (None, Some(name)) => Some(10_000 + name.score),
                    (None, None) => None,
                };
                score.map(|score| {
                    (
                        score,
                        Match {
                            entry: *index,
                            positions: by_name.map(|name| name.positions).unwrap_or_default(),
                        },
                    )
                })
            })
            .collect()
    };
    let mut found = find(false);
    let close = found.is_empty();
    if close {
        found = find(true);
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.entry.cmp(&b.1.entry)));
    Found {
        close: close && !found.is_empty(),
        matches: found.into_iter().map(|(_, matched)| matched).collect(),
    }
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
            .matches
            .into_iter()
            .map(|matched| entries[matched.entry].row.title.to_string())
            .collect()
    }

    /// The letters of each found title that are marked, as texts.
    fn marked(entries: &[Entry], query: &str) -> Vec<String> {
        search(entries, query, false)
            .matches
            .into_iter()
            .map(|matched| {
                let title = &entries[matched.entry].row.title;
                matched
                    .positions
                    .iter()
                    .map(|at| title[*at..].chars().next().unwrap())
                    .collect()
            })
            .collect()
    }

    /// Rows with nothing but a name, in the text.
    fn named(names: &[&'static str]) -> Vec<Entry> {
        let defaults = Defaults::from_bindings(names.iter().enumerate().map(|(ix, _)| {
            DefaultBinding::new(&format!("test::A{ix}"), None, Some("Editor"), "cmd-f1").unwrap()
        }));
        let keymap = Keymap::new(defaults, Places::new().typing("Editor"));
        entries(&Listing::new(
            keymap,
            names
                .iter()
                .enumerate()
                .map(|(ix, name)| {
                    CommandRow::new(
                        Target::new(&format!("test::A{ix}"), Some("Editor")),
                        *name,
                        "In the text",
                    )
                    .category("Format")
                })
                .collect(),
        ))
    }

    #[test]
    fn an_empty_query_lists_everything_in_order() {
        let entries = listing();
        assert_eq!(search(&entries, "  ", false).matches.len(), entries.len());
        assert_eq!(titles(&entries, "")[0], "Command palette");
    }

    #[test]
    fn a_title_is_found_by_the_words_in_it() {
        let entries = listing();
        assert_eq!(titles(&entries, "bold"), vec!["Bold"]);
        assert_eq!(titles(&entries, "numb"), vec!["Numbered list"]);
        // Several words, each found; in any order.
        assert_eq!(titles(&entries, "move up"), vec!["Move block up"]);
        assert_eq!(titles(&entries, "up move"), vec!["Move block up"]);
        assert_eq!(titles(&entries, "BLOCK"), vec!["Move block up"]);
    }

    #[test]
    fn letters_scattered_over_several_words_are_not_a_match() {
        // What the released list did: "italic" found the insert menu by
        // an i here, a t there.
        let entries = named(&[
            "Italic",
            "Insert menu (at the start of a line or after a space)",
            "Bullet list",
            "Numbered list",
        ]);
        assert_eq!(titles(&entries, "italic"), vec!["Italic"]);
        assert_eq!(titles(&entries, "ital"), vec!["Italic"]);
        // "nl" is no word of "Numbered list", "blt" none of "Bullet list".
        assert!(titles(&entries, "nl").is_empty());
        assert!(titles(&entries, "blt").is_empty());
        // Two words are two words: each stands together, and each has to
        // be there.
        assert_eq!(titles(&entries, "num li"), vec!["Numbered list"]);
        assert!(titles(&entries, "num lt").is_empty());
    }

    #[test]
    fn the_start_of_a_word_comes_before_the_inside_of_one() {
        let entries = named(&["Underline", "Line break", "Delete to line start", "Outline"]);
        // "line" starts a word in two titles and is inside a word in two:
        // the first two first, each group in the order of the list.
        assert_eq!(
            titles(&entries, "line"),
            vec!["Line break", "Delete to line start", "Underline", "Outline"]
        );
        // A title that begins with the query before one that has it later.
        let entries = named(&["Replace and find", "Find", "Find next"]);
        assert_eq!(
            titles(&entries, "find"),
            vec!["Find", "Find next", "Replace and find"]
        );
    }

    #[test]
    fn the_marked_letters_are_the_ones_that_matched() {
        let entries = named(&[
            "Italic",
            "Underline",
            "Line break",
            "Move block up",
            "Insert menu (at the start of a line or after a space)",
        ]);
        assert_eq!(marked(&entries, "ital"), vec!["Ital"]);
        // At the start of a word where there is one, inside a word
        // otherwise; never a letter from somewhere else.
        assert_eq!(marked(&entries, "line"), vec!["Line", "line", "line"]);
        assert_eq!(
            titles(&entries, "line"),
            vec![
                "Line break",
                "Insert menu (at the start of a line or after a space)",
                "Underline"
            ]
        );
        assert_eq!(marked(&entries, "up move"), vec!["Moveup"]);
        // Found by the category alone: nothing of the title is marked.
        assert_eq!(marked(&entries, "format").len(), 5);
        assert!(marked(&entries, "format").iter().all(String::is_empty));
        // By the title and the category: the word of the title.
        assert_eq!(marked(&entries, "format ital"), vec!["Ital"]);
    }

    #[test]
    fn a_row_found_by_its_key_has_no_letter_marked() {
        let entries = listing();
        // "cmd-backspace": the key of a row, and no text of any title.
        assert_eq!(
            titles(&entries, "cmd-backspace"),
            vec!["Delete to line start"]
        );
        assert_eq!(marked(&entries, "cmd-backspace"), vec![""]);
    }

    #[test]
    fn one_typing_mistake_is_forgiven_when_nothing_matches_as_written() {
        let entries = named(&["Bold", "Italic", "Underline", "Numbered list", "Find"]);
        let close = |query: &str| search(&entries, query, false).close;
        // Two letters swapped, one wrong, one missing, one too many.
        for (query, found, right) in [
            ("itlaic", "Italic", "Italic"),
            ("itakic", "Italic", "Itaic"),
            ("undrline", "Underline", "Undrline"),
            ("boldd", "Bold", "Bold"),
            ("numberred", "Numbered list", "Numbered"),
        ] {
            assert_eq!(titles(&entries, query), vec![found], "{query}");
            assert!(close(query), "{query} is not there as written");
            assert_eq!(marked(&entries, query), vec![right], "{query}");
        }
        // Not when something matches as written: no close row is added.
        assert_eq!(titles(&entries, "ital"), vec!["Italic"]);
        assert!(!close("ital"));
        // Not two mistakes, not a short word, not a wrong first letter.
        for query in ["itlakc", "bld", "fnd", "ctalic", "xold"] {
            assert!(titles(&entries, query).is_empty(), "{query}");
            assert!(!close(query), "{query}");
        }
        // Each word is forgiven one, the rest has to be there.
        assert_eq!(titles(&entries, "numbred lsit"), vec!["Numbered list"]);
        assert!(titles(&entries, "numbred xyzzy").is_empty());
    }

    #[test]
    fn a_heading_and_a_category_are_found_by_their_words() {
        let entries = listing();
        assert_eq!(titles(&entries, "library"), vec!["First row"]);
        let format = titles(&entries, "format");
        assert!(
            format.contains(&"Bold".to_string()) && format.contains(&"Numbered list".to_string())
        );
        // A word of the title and a word of the place.
        assert_eq!(titles(&entries, "text bold"), vec!["Bold"]);
        assert_eq!(titles(&entries, "library first"), vec!["First row"]);
        assert!(titles(&entries, "library bold").is_empty());
        // Of a place or a category the start of a word counts, not its
        // inside: "or" is no reason to list everything under "Format".
        assert_eq!(titles(&entries, "form").len(), 2);
        assert!(titles(&entries, "ormat").is_empty());
        assert!(titles(&entries, "or").is_empty());
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
    fn a_whole_key_finds_that_key_and_not_the_keys_that_begin_with_it() {
        let entries = listing();
        // ⌘B was Bold's default (it has ⌥⌘B now): that is the row somebody
        // who writes "cmd-b" is looking for, not ⌘⌫, which only begins
        // with a b.
        assert_eq!(titles(&entries, "cmd-b"), vec!["Bold"]);
        assert_eq!(titles(&entries, "⌘B"), vec!["Bold"]);
        // A command that has the key now comes before one that had it.
        let mut keymap_entries = listing();
        keymap_entries[2].keys[0].keys = KeySequence::parse("cmd-b").unwrap();
        assert_eq!(
            titles(&keymap_entries, "cmd-b"),
            vec!["Delete to line start", "Bold"]
        );
        // The same for a key with a name and for a single key of a
        // sequence: "g" has no row of its own, so "g g" is shown.
        // (A single letter is also a word: after the key, the names that
        // have it.)
        assert_eq!(titles(&entries, "g"), vec!["First row", "New paragraph"]);
        assert_eq!(titles(&entries, "alt-up"), vec!["Move block up"]);
    }

    #[test]
    fn a_key_nobody_has_shows_the_keys_that_begin_with_it() {
        let entries = listing();
        // No command has ⌘E: the keys with ⌘ that begin with an e... there
        // are none here; with ⌘ and a b there is ⌘⌫ once ⌘B is nobody's.
        assert!(titles(&entries, "cmd-e").is_empty());
        let unchanged: Vec<Entry> = listing()
            .into_iter()
            .filter(|entry| entry.row.title != "Bold")
            .collect();
        assert_eq!(titles(&unchanged, "cmd-b"), vec!["Delete to line start"]);
    }

    #[test]
    fn an_unfinished_key_finds_the_keys_that_begin_with_it() {
        let entries = listing();
        // "cmd-ba" is no key yet: what begins with it.
        assert_eq!(titles(&entries, "cmd-ba"), vec!["Delete to line start"]);
        assert_eq!(titles(&entries, "cmd-back"), vec!["Delete to line start"]);
        let query = |text: &str| parse_key_query(text).unwrap();
        assert!(query("cmd-b").is_complete());
        assert!(query("⌘B").is_complete());
        assert!(query("f5").is_complete());
        assert!(query("opt-up").is_complete());
        assert!(query("esc").is_complete());
        assert!(query("g g").is_complete());
        assert!(!query("cmd-ba").is_complete());
        assert!(!query("cmd-").is_complete());
        assert!(!query("cmd-shift").is_complete());
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
            .matches
            .into_iter()
            .map(|matched| matched.entry)
            .collect();
        assert_eq!(changed, vec![1]);
        assert!(search(&entries, "palette", true).matches.is_empty());
    }
}
