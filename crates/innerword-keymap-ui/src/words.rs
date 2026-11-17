//! What the view says, as sentences: the verdict on a proposed key, what
//! an import would do, and the names of the commands in them. Plain
//! functions from the keymap's types to text, so the wording is tested
//! without a window.

use innerword_keymap::{
    Catalogue, Collision, Holder, ImportOutcome, ImportPlan, KeySequence, Overlap, Refusal, Target,
    Verdict, Warning,
};

use crate::{keys::glyphs, listing::Entry};

/// How a line is to be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// Something to know.
    Plain,
    /// Nothing is in the way.
    Good,
    /// Allowed, with a consequence worth reading.
    Warning,
    /// The reason something cannot be done, or a conflict to settle.
    Stop,
}

/// One sentence of the view, with how it is to be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// How it is to be read.
    pub tone: Tone,
    /// The sentence.
    pub text: String,
}

impl Line {
    fn new(tone: Tone, text: impl Into<String>) -> Self {
        Self {
            tone,
            text: text.into(),
        }
    }
}

/// The names of commands: the rows of the list first, then the host's
/// catalogue, then what can be made of the action's own name.
pub(crate) struct Names<'a> {
    pub entries: &'a [Entry],
    pub catalogue: &'a Catalogue,
}

impl Names<'_> {
    /// What the command of `target` is called.
    pub fn title(&self, target: &Target) -> String {
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.row.target == *target)
        {
            return entry.row.title.to_string();
        }
        if let Some(info) = self.catalogue.info(target) {
            return info.title.clone();
        }
        // The same command in another place has the same name.
        if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.row.target.command == target.command)
        {
            return entry.row.title.to_string();
        }
        humanize_action_name(&target.command.action)
    }

    /// What the command with the action called `action` is called here,
    /// wherever it applies; `None` for an action this version has no
    /// command for.
    pub fn title_of_action(&self, action: &str) -> Option<String> {
        self.entries
            .iter()
            .find(|entry| entry.row.target.command.action == action)
            .map(|entry| entry.row.title.to_string())
            .or_else(|| {
                self.catalogue
                    .entries()
                    .iter()
                    .find(|info| info.target.command.action == action)
                    .map(|info| info.title.clone())
            })
    }

    /// Where `target` applies, in words as they stand in a sentence: "in
    /// the text", "anywhere".
    pub fn place(&self, target: &Target) -> String {
        let place = if let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.row.target == *target)
        {
            entry.row.place.to_string()
        } else if let Some(info) = self.catalogue.info(target) {
            info.place.clone()
        } else {
            match target.context.text() {
                Some(context) => format!("in {context}"),
                None => "anywhere".to_string(),
            }
        };
        lower_first(&place)
    }
}

pub(crate) fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `text` as the start of a sentence: its first letter in upper case,
/// unless its first word is written with a capital of its own further in
/// ("macOS keeps ..." stays as it is).
pub(crate) fn upper_first(text: &str) -> String {
    let first_word = text.split_whitespace().next().unwrap_or_default();
    if first_word.chars().any(char::is_uppercase) {
        return text.to_string();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// `editor::MoveWordLeft` as "Move word left".
pub(crate) fn humanize_action_name(name: &str) -> String {
    let short = name.rsplit("::").next().unwrap_or(name);
    let mut out = String::with_capacity(short.len() + 4);
    let mut previous: Option<char> = None;
    for ch in short.chars() {
        let boundary = previous.is_some_and(|previous| {
            (ch.is_uppercase() && !previous.is_uppercase())
                || (ch.is_ascii_digit() && !previous.is_ascii_digit())
        });
        if boundary {
            out.push(' ');
        }
        if previous.is_none() {
            out.extend(ch.to_uppercase());
        } else {
            out.extend(ch.to_lowercase());
        }
        previous = Some(ch);
    }
    out
}

/// "1 key", "3 keys".
pub(crate) fn count(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// Something of the host's that sees a key before the keymap does: what it
/// is called, when the host said, and its sentence.
pub(crate) struct SeenFirst {
    pub name: Option<String>,
    pub warning: String,
}

/// What the view says about a key that was pressed for a command: a
/// heading, how the heading is to be read (its mark), and the sentences
/// under it.
pub(crate) struct VerdictWords {
    pub title: String,
    pub tone: Tone,
    pub lines: Vec<Line>,
}

impl VerdictWords {
    /// Whether a sentence warns that the key, once given, may not do what
    /// was asked of it, or costs something no choice names: then the
    /// choice that gives it is not the one a bare confirmation takes.
    pub fn warns(&self) -> bool {
        self.lines.iter().any(|line| line.tone == Tone::Warning)
    }
}

/// "another command", "2 other commands": the commands of `holders`, each
/// name once.
fn other_commands<'a>(holders: impl IntoIterator<Item = &'a Holder>, names: &Names) -> String {
    let mut titles: Vec<String> = Vec::new();
    for holder in holders {
        let title = names.title(&holder.target);
        if !titles.contains(&title) {
            titles.push(title);
        }
    }
    match titles.len() {
        0 | 1 => "another command".to_string(),
        n => format!("{n} other commands"),
    }
}

/// The verdict on giving `target` the key, as the view says it. The
/// heading says the most important thing about the key and the first
/// sentence is about the same thing; what matters less follows:
///
/// 1. the key cannot be given (the reason alone);
/// 2. it is taken: another command has it in the same place;
/// 3. it may not run: macOS uses it, or something of the host's
///    (`seen_first`) sees it before any shortcut does;
/// 4. it would not work everywhere: another command keeps it where both
///    apply;
/// 5. it costs something in a text: a key text is edited with, a
///    character typed with option;
/// 6. another command has it where both apply, and this one gets it
///    there;
/// 7. it is free: nobody has it, or only commands in places of their own.
///
/// A heading never says "free", and is never marked as good, above a
/// sentence that warns.
pub(crate) fn verdict_words(
    verdict: &Verdict,
    target: &Target,
    seen_first: &[SeenFirst],
    names: &Names,
) -> VerdictWords {
    let key = glyphs(&verdict.key);
    let command = names.title(target);
    if let Some(refusal) = &verdict.refusal {
        let (title, tone) = match refusal {
            Refusal::SystemKeeps(_) => (format!("macOS keeps {key}"), Tone::Stop),
            Refusal::AlreadyHasKey => (
                format!("\u{201c}{command}\u{201d} has {key} already"),
                Tone::Plain,
            ),
            _ => (format!("{key} cannot be used"), Tone::Stop),
        };
        return VerdictWords {
            title,
            tone,
            lines: vec![Line::new(tone, refusal_text(refusal, &key))],
        };
    }

    // A command the keymap holds in several key contexts of one place is
    // one command to the user (one row of the list): where it has the key
    // in the same place as this one, that is said once, and not again for
    // its other key context ("... keeps it in the rest of its place").
    let called = |holder: &Holder| (names.title(&holder.target), names.place(&holder.target));
    let taken: Vec<(String, String)> = verdict.taken.iter().map(called).collect();
    let said_as_taken = |holder: &&Holder| !taken.contains(&called(holder));
    let keeps: Vec<&Holder> = overlapping(verdict)
        .filter(|holder| holder.overlap == Overlap::HolderWinsThere)
        .filter(said_as_taken)
        .collect();
    let yields: Vec<&Holder> = overlapping(verdict)
        .filter(|holder| holder.overlap != Overlap::HolderWinsThere)
        .filter(said_as_taken)
        .collect();
    let apart: Vec<&Holder> = verdict
        .elsewhere
        .iter()
        .filter(|holder| holder.overlap == Overlap::Apart)
        .filter(said_as_taken)
        .collect();
    let system = verdict.warnings.iter().find_map(|warning| match warning {
        Warning::SystemUses(system) => Some(system),
        _ => None,
    });
    let editing = verdict.warnings.contains(&Warning::EditingKey);
    let special = verdict.warnings.contains(&Warning::SpecialCharacter);

    // The sentences, the most important first; none twice.
    let mut lines: Vec<Line> = Vec::new();
    for holder in &verdict.taken {
        let text = holder_text(holder, &verdict.key, names);
        if !lines.iter().any(|line| line.text == text) {
            lines.push(Line::new(Tone::Stop, text));
        }
    }
    for warning in &verdict.warnings {
        if matches!(warning, Warning::SystemUses(_)) {
            lines.push(Line::new(Tone::Warning, warning_text(warning, &key)));
        }
    }
    for seen in seen_first {
        lines.push(Line::new(Tone::Warning, seen.warning.clone()));
    }
    for holder in &keeps {
        let text = holder_text(holder, &verdict.key, names);
        if !lines.iter().any(|line| line.text == text) {
            lines.push(Line::new(Tone::Warning, text));
        }
    }
    for warning in &verdict.warnings {
        if !matches!(warning, Warning::SystemUses(_)) {
            lines.push(Line::new(Tone::Warning, warning_text(warning, &key)));
        }
    }
    // The commands that share the key where both can apply, one by one;
    // those in places of their own in one sentence (`escape` and the arrows
    // are keys of many places).
    for holder in &yields {
        let text = holder_text(holder, &verdict.key, names);
        if !lines.iter().any(|line| line.text == text) {
            lines.push(Line::new(Tone::Plain, text));
        }
    }
    if !apart.is_empty() {
        lines.push(Line::new(
            Tone::Plain,
            apart_text(&apart, &verdict.key, names),
        ));
    }

    let (title, tone) = if !verdict.taken.is_empty() {
        (format!("{key} is taken"), Tone::Stop)
    } else if system.is_some() {
        (format!("macOS uses {key}"), Tone::Warning)
    } else if let Some(seen) = seen_first.first() {
        (
            match &seen.name {
                Some(name) => format!("{key}: {name} sees it first"),
                None => format!("{key} may not run"),
            },
            Tone::Warning,
        )
    } else if !keeps.is_empty() {
        (format!("{key} would not work everywhere"), Tone::Warning)
    } else if editing {
        (format!("{key} is used for typing"), Tone::Warning)
    } else if special {
        (format!("{key} types a special character"), Tone::Warning)
    } else if !yields.is_empty() {
        (
            format!(
                "{key} is also the key of {}",
                other_commands(yields.iter().copied(), names)
            ),
            Tone::Plain,
        )
    } else if !apart.is_empty() {
        // Free where this command applies. The place is named unless that
        // says nothing ("anywhere") or the other command's place goes by
        // the same name.
        let place = names.place(target);
        let named_twice = apart
            .iter()
            .any(|holder| names.place(&holder.target) == place);
        if place == "anywhere" || named_twice {
            (format!("{key} is free here"), Tone::Good)
        } else {
            (format!("{key} is free {place}"), Tone::Good)
        }
    } else {
        lines.push(Line::new(
            Tone::Good,
            format!("No other command has {key} where \u{201c}{command}\u{201d} applies."),
        ));
        (format!("{key} is free"), Tone::Good)
    };
    VerdictWords { title, tone, lines }
}

/// Why the key cannot be given. The heading has said what is the matter;
/// the sentence says why, or what follows.
fn refusal_text(refusal: &Refusal, key: &str) -> String {
    match refusal {
        Refusal::TypesText => format!(
            "{key} types a character, and text is typed where this command applies: the \
             character could no longer be typed there. Hold \u{2318}, \u{2303} or \u{2325} with it."
        ),
        Refusal::SystemKeeps(system) => format!(
            "In macOS {key} {}, and no setting changes that: the key never reaches Inner Word.",
            system.what
        ),
        Refusal::AlreadyHasKey => "Nothing would change.".to_string(),
        Refusal::UnknownCommand => "This version cannot give this command a key.".to_string(),
    }
}

fn warning_text(warning: &Warning, key: &str) -> String {
    match warning {
        Warning::SpecialCharacter => format!(
            "On a Mac keyboard, Option with a key types a special character or starts an \
             accent. Text is typed where this command applies: as a shortcut, {key} could no \
             longer type its character there."
        ),
        Warning::EditingKey => format!(
            "Text is typed where this command applies. As a shortcut, {key} would run the \
             command there instead of what it does in a text."
        ),
        Warning::SystemUses(system) => format!(
            "In macOS {key} {}. Unless that is switched off in System Settings, the shortcut \
             may not run.",
            system.what
        ),
    }
}

/// One command that has the key (or a key that begins the same way), and
/// how the two get along.
fn holder_text(holder: &Holder, proposed: &KeySequence, names: &Names) -> String {
    let key = glyphs(proposed);
    let title = names.title(&holder.target);
    let place = names.place(&holder.target);
    let who = format!("\u{201c}{title}\u{201d}");
    if holder.keys != *proposed {
        let theirs = glyphs(&holder.keys);
        let tail = match holder.overlap {
            Overlap::Apart => "That is a different place: both work.".to_string(),
            Overlap::ProposedWinsThere => {
                "Where both apply, this command's key is the one that works.".to_string()
            }
            Overlap::HolderWinsThere => format!("Where both apply, {who} keeps its key."),
            _ => "Only one of the two can work there.".to_string(),
        };
        return format!(
            "{who} has {theirs} {place}, and {theirs} and {key} begin with the same key. {tail}"
        );
    }
    match holder.overlap {
        Overlap::SamePlace => {
            format!("{key} is the key of {who} {place}. Only one of the two can have it there.")
        }
        Overlap::HolderNeverRuns => format!(
            "{key} is the key of {who} {place}. This command would get it everywhere that one \
             applies, and {who} could no longer be run with it."
        ),
        Overlap::ProposedNeverRuns => format!(
            "{key} is the key of {who} {place}, which keeps it everywhere this command \
             applies: here the key would do nothing."
        ),
        Overlap::ProposedWinsThere => format!(
            "{key} is also the key of {who} {place}. Where both apply this command gets it, \
             and {who} keeps it in the rest of its place."
        ),
        Overlap::HolderWinsThere => format!(
            "{key} is also the key of {who} {place}. Where both apply {who} keeps it, and this \
             command gets it in the rest of its place."
        ),
        Overlap::Apart => {
            format!("{key} is also the key of {who} {place}. That is a different place: both work.")
        }
    }
}

/// The commands that have the key in a place that overlaps with the
/// proposed one: both would work, and one of them decides where both apply.
pub(crate) fn overlapping(verdict: &Verdict) -> impl Iterator<Item = &Holder> {
    verdict
        .elsewhere
        .iter()
        .filter(|holder| holder.overlap != Overlap::Apart)
}

/// The commands that have the key in places of their own, in one sentence.
fn apart_text(holders: &[&Holder], proposed: &KeySequence, names: &Names) -> String {
    const NAMED: usize = 2;
    let key = glyphs(proposed);
    let mut named: Vec<String> = Vec::new();
    for holder in holders {
        let text = format!(
            "\u{201c}{}\u{201d} {}",
            names.title(&holder.target),
            names.place(&holder.target)
        );
        if !named.contains(&text) {
            named.push(text);
        }
    }
    let total = named.len();
    let more = total.saturating_sub(NAMED);
    named.truncate(NAMED);
    let list = match (named.as_slice(), more) {
        ([one], 0) => one.clone(),
        ([one, two], 0) => format!("{one} and of {two}"),
        (some, more) => format!(
            "{} and of {}",
            some.join(", of "),
            count(more, "more command", "more commands")
        ),
    };
    if total == 1 {
        format!("{key} is also the key of {list}. That is a different place: both work.")
    } else {
        format!("{key} is also the key of {list}. Those are different places: all of them work.")
    }
}

/// What the line under the query says about the user's changes. What is
/// counted is commands (rows of the list), never single keys: a command
/// with two keys of the user's is one, a command left without a key is
/// one. `hidden`: how many more are changed that the list has no row for
/// (the commands of a part of the app that is not there right now), so
/// that the line agrees with what "Reset All" says it puts back.
pub(crate) fn changed_summary(changed: usize, hidden: usize) -> String {
    match (changed, hidden) {
        (0, 0) => "No key changed".to_string(),
        (changed, 0) => format!("{} changed", count(changed, "command", "commands")),
        (0, hidden) => format!(
            "{} changed, not shown now",
            count(hidden, "command", "commands")
        ),
        (changed, hidden) => format!(
            "{} changed, {hidden} more not shown now",
            count(changed, "command", "commands")
        ),
    }
}

/// What the view says under "Press the new key ..." while it waits for
/// one: that every key counts, and how to leave without a change, with
/// the pointer and from the keyboard alone. `cancel` is the key that
/// cancels a question, as the keymap has it now (`None` when the user
/// took it away), and `is_escape` whether that is Esc alone.
///
/// From the keyboard the way out always leads through the question about
/// a key, since no key leaves the waiting itself: with Esc as the key
/// that cancels, Esc pressed twice does it.
pub(crate) fn waiting_help(cancel: Option<&str>, is_escape: bool) -> String {
    let way_out = match cancel {
        Some(_) if is_escape => {
            "press Esc twice (the first is taken as the key, the second cancels it)".to_string()
        }
        Some(key) => format!("press any key and then {key}"),
        None => "press any key and choose Cancel in the question that follows".to_string(),
    };
    format!(
        "Every key counts, Esc and Return included. To leave without a change: click Cancel, \
         or {way_out}."
    )
}

/// What stands above the rows of a search that found nothing as it was
/// written, only names one typing mistake away.
pub(crate) fn close_matches_note(query: &str) -> String {
    format!("Nothing matches \u{201c}{query}\u{201d} as written. One letter away:")
}

/// "Take It from “Italic”", "Take It from 2 Commands".
pub(crate) fn take_label<'a>(
    holders: impl IntoIterator<Item = &'a Holder>,
    names: &Names,
) -> String {
    let mut titles: Vec<String> = Vec::new();
    for holder in holders {
        let title = names.title(&holder.target);
        if !titles.contains(&title) {
            titles.push(title);
        }
    }
    match titles.as_slice() {
        [one] => format!("Take It from \u{201c}{one}\u{201d}"),
        many => format!("Take It from {} Commands", many.len()),
    }
}

/// What an import would do, in a few sentences above its list. `changing`
/// and `colliding`: how many commands get other keys and how many
/// collisions are left, as the list under the sentences shows them.
pub(crate) fn import_summary(
    plan: &ImportPlan,
    outcome: &ImportOutcome,
    changing: usize,
    colliding: usize,
) -> Vec<Line> {
    let mut lines = Vec::new();
    if outcome.differences.is_empty() {
        lines.push(Line::new(
            Tone::Plain,
            if plan.changes.is_empty() {
                "Nothing would change: the file changes no key.".to_string()
            } else {
                "Nothing would change: you already have the keys of the file.".to_string()
            },
        ));
    } else {
        lines.push(Line::new(
            Tone::Plain,
            format!(
                "{} {} other keys.",
                count(changing, "command", "commands"),
                if changing == 1 { "gets" } else { "get" }
            ),
        ));
    }
    if !outcome.conflicts.is_empty() {
        let n = colliding;
        lines.push(Line::new(
            Tone::Warning,
            format!(
                "It would leave {}: two commands on one key in the same place.",
                count(n, "collision", "collisions")
            ),
        ));
    }
    if !plan.skipped.is_empty() {
        let n = plan.skipped.len();
        lines.push(Line::new(
            Tone::Warning,
            format!(
                "{} of the file {} skipped.",
                count(n, "entry", "entries"),
                if n == 1 { "is" } else { "are" }
            ),
        ));
    }
    lines
}

/// "Inner Word 0.4.0 wrote it; 212 entries, 209 of them leave their
/// command as it is."
pub(crate) fn import_file_line(plan: &ImportPlan) -> String {
    let mut parts = Vec::new();
    if let Some(version) = &plan.app_version {
        parts.push(format!("Written by Inner Word {version}"));
    }
    parts.push(count(plan.entries, "entry", "entries"));
    if plan.unchanged > 0 {
        parts.push(format!("{} without a change", plan.unchanged));
    }
    parts.join(" \u{b7} ")
}

/// What is wrong with a row whose key another command has in the same
/// place.
pub(crate) fn clash_text(key: &KeySequence, other: &Target, names: &Names) -> String {
    format!(
        "{} is also the key of \u{201c}{}\u{201d} {}: one of the two cannot run with it.",
        glyphs(key),
        names.title(other),
        names.place(other)
    )
}

/// A collision an import would leave, as a sentence.
pub(crate) fn collision_text(collision: &Collision, names: &Names) -> String {
    let key = glyphs(&collision.key);
    let one = names.title(&collision.target);
    let other = names.title(&collision.holder.target);
    let place = names.place(&collision.target);
    let other_place = names.place(&collision.holder.target);
    if place == other_place {
        format!(
            "{key} would be the key of \u{201c}{one}\u{201d} and of \u{201c}{other}\u{201d} {place}."
        )
    } else {
        format!(
            "{key} would be the key of \u{201c}{one}\u{201d} {place} and of \u{201c}{other}\u{201d} {other_place}."
        )
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use innerword_keymap::{DefaultBinding, Defaults, Keymap, Places};

    /// A small keymap with every kind of neighbour: commands of the
    /// window, of the text (inside the window), and of the find bar's
    /// field (a place of its own).
    pub(crate) fn keymap() -> Keymap {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let defaults = Defaults::from_bindings([
            binding("app::QuickOpen", None, "cmd-p"),
            binding("comments::Toggle", None, "cmd-alt-c"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::ToggleItalic", Some("Editor"), "cmd-i"),
            binding("editor::Complete", Some("Editor"), "cmd-space"),
            // One command to the user, bound for the bar and for its field.
            binding("tools::CaseSensitive", Some("FindBar"), "cmd-alt-c"),
            binding(
                "tools::CaseSensitive",
                Some("FindBar > TextField"),
                "cmd-alt-c",
            ),
            binding("tools::FindNext", Some("FindBar > TextField"), "cmd-g"),
            binding("tools::FindPrevious", Some("FindBar > TextField"), "f3"),
        ]);
        Keymap::new(
            defaults,
            Places::new()
                .typing("Editor")
                .typing("TextField")
                .inside("TextField", "FindBar")
                .container("FindBar"),
        )
    }

    pub(crate) fn names_of(catalogue: &Catalogue) -> Names<'_> {
        Names {
            entries: &[],
            catalogue,
        }
    }

    pub(crate) fn catalogue() -> Catalogue {
        let mut catalogue = Catalogue::new();
        for (action, context, title, place) in [
            ("editor::ToggleBold", Some("Editor"), "Bold", "In the text"),
            (
                "editor::ToggleItalic",
                Some("Editor"),
                "Italic",
                "In the text",
            ),
            (
                "editor::Complete",
                Some("Editor"),
                "Complete",
                "In the text",
            ),
            ("app::QuickOpen", None, "Quick open", "Anywhere"),
            (
                "comments::Toggle",
                None,
                "Show or hide comments",
                "Anywhere",
            ),
            (
                "tools::CaseSensitive",
                Some("FindBar"),
                "Case sensitive",
                "In the find bar",
            ),
            (
                "tools::CaseSensitive",
                Some("FindBar > TextField"),
                "Case sensitive",
                "In the find bar",
            ),
            (
                "tools::FindNext",
                Some("FindBar > TextField"),
                "Find next",
                "In the find bar",
            ),
            (
                "tools::FindPrevious",
                Some("FindBar > TextField"),
                "Find previous",
                "In the find bar",
            ),
        ] {
            catalogue.add(Target::new(action, context), title, place);
        }
        catalogue
    }

    pub(crate) fn key(text: &str) -> KeySequence {
        KeySequence::parse(text).unwrap()
    }

    #[test]
    fn a_taken_key_names_the_command_that_has_it() {
        let keymap = keymap();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let verdict = keymap.check(&bold, &key("cmd-i"), Some(&key("cmd-b")));
        let catalogue = catalogue();
        let names = names_of(&catalogue);
        let says = verdict_words(&verdict, &bold, &[], &names);
        assert_eq!(says.title, "⌘I is taken");
        assert_eq!(says.lines[0].tone, Tone::Stop);
        assert_eq!(
            says.lines[0].text,
            "⌘I is the key of “Italic” in the text. Only one of the two can have it there."
        );
        assert_eq!(take_label(&verdict.taken, &names), "Take It from “Italic”");
    }

    #[test]
    fn commands_in_places_of_their_own_are_named_in_one_sentence() {
        let catalogue = catalogue();
        let names = names_of(&catalogue);
        let holder = |action: &str, context: Option<&str>| Holder {
            target: Target::new(action, context),
            keys: key("f3"),
            origin: innerword_keymap::Origin::Default,
            overlap: Overlap::Apart,
        };
        let one = holder("tools::FindNext", Some("FindBar > TextField"));
        let two = holder("app::QuickOpen", None);
        let three = holder("editor::ToggleItalic", Some("Editor"));
        let four = holder("comments::Toggle", None);
        assert_eq!(
            apart_text(&[&one], &key("f3"), &names),
            "F3 is also the key of “Find next” in the find bar. That is a different place: both work."
        );
        assert_eq!(
            apart_text(&[&one, &two], &key("f3"), &names),
            "F3 is also the key of “Find next” in the find bar and of “Quick open” anywhere. \
             Those are different places: all of them work."
        );
        // More than two: the rest is counted, one or several.
        assert!(
            apart_text(&[&one, &two, &three], &key("f3"), &names)
                .contains("of “Quick open” anywhere and of 1 more command."),
            "{}",
            apart_text(&[&one, &two, &three], &key("f3"), &names)
        );
        assert!(
            apart_text(&[&one, &two, &three, &four], &key("f3"), &names)
                .contains("and of 2 more commands."),
        );
    }

    #[test]
    fn a_command_without_a_name_gets_one_from_its_action() {
        let catalogue = Catalogue::new();
        let names = names_of(&catalogue);
        assert_eq!(
            names.title(&Target::new("editor::MoveWordLeft", Some("Editor"))),
            "Move word left"
        );
        assert_eq!(
            names.place(&Target::new("editor::MoveWordLeft", Some("Editor"))),
            "in Editor"
        );
        assert_eq!(names.place(&Target::new("app::Quit", None)), "anywhere");
    }

    #[test]
    fn the_header_counts_commands() {
        assert_eq!(changed_summary(0, 0), "No key changed");
        assert_eq!(changed_summary(1, 0), "1 command changed");
        assert_eq!(changed_summary(4, 0), "4 commands changed");
        // Changed commands the list has no row for right now.
        assert_eq!(
            changed_summary(4, 1),
            "4 commands changed, 1 more not shown now"
        );
        assert_eq!(changed_summary(0, 1), "1 command changed, not shown now");
        assert_eq!(changed_summary(0, 2), "2 commands changed, not shown now");
    }

    #[test]
    fn waiting_for_a_key_says_how_to_leave() {
        assert_eq!(
            waiting_help(Some("Esc"), true),
            "Every key counts, Esc and Return included. To leave without a change: click \
             Cancel, or press Esc twice (the first is taken as the key, the second cancels it)."
        );
        assert!(waiting_help(Some("F1"), false).ends_with("or press any key and then F1."));
        assert!(
            waiting_help(None, false)
                .ends_with("or press any key and choose Cancel in the question that follows.")
        );
    }

    #[test]
    fn a_reason_becomes_a_sentence_without_spoiling_a_name() {
        assert_eq!(upper_first("a key cannot be read"), "A key cannot be read");
        assert_eq!(
            upper_first("its \"keys\" is not a list"),
            "Its \"keys\" is not a list"
        );
        // A key in glyphs, and a word with a capital of its own.
        assert_eq!(upper_first("I is a character"), "I is a character");
        assert_eq!(upper_first("⌘⇥ is kept"), "⌘⇥ is kept");
        assert_eq!(
            upper_first("macOS keeps ⌘⇥ for itself"),
            "macOS keeps ⌘⇥ for itself"
        );
        assert_eq!(upper_first(""), "");
    }

    #[test]
    fn counts_are_singular_and_plural() {
        assert_eq!(count(1, "key", "keys"), "1 key");
        assert_eq!(count(0, "key", "keys"), "0 keys");
        assert_eq!(count(12, "entry", "entries"), "12 entries");
    }
}
