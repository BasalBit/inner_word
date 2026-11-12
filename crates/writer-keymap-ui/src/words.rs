//! What the view says, as sentences: the verdict on a proposed key, what
//! an import would do, and the names of the commands in them. Plain
//! functions from the keymap's types to text, so the wording is tested
//! without a window.

use writer_keymap::{
    Catalogue, Collision, Holder, ImportOutcome, ImportPlan, KeySequence, Overlap, Refusal,
    Target, Verdict, VerdictKind, Warning,
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
        if let Some(entry) = self.entries.iter().find(|entry| entry.row.target == *target) {
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

    /// Where `target` applies, in words as they stand in a sentence: "in
    /// the text", "anywhere".
    pub fn place(&self, target: &Target) -> String {
        let place = if let Some(entry) =
            self.entries.iter().find(|entry| entry.row.target == *target)
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

fn lower_first(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
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

/// The verdict in a few words, above its sentences: "⌥⌘B is free".
pub(crate) fn verdict_title(verdict: &Verdict) -> String {
    let key = glyphs(&verdict.key);
    match verdict.kind() {
        VerdictKind::Fine => format!("{key} is free"),
        VerdictKind::UsedElsewhere => format!("{key} is also used in another place"),
        VerdictKind::Taken => format!("{key} is taken"),
        VerdictKind::Refused => format!("{key} cannot be used"),
    }
}

/// The verdict on giving the command called `command` the key, sentence
/// by sentence: the refusal alone when it is refused; otherwise who has
/// the key where it would apply, who has it elsewhere, and the warnings.
pub(crate) fn verdict_lines(verdict: &Verdict, command: &str, names: &Names) -> Vec<Line> {
    let key = glyphs(&verdict.key);
    if let Some(refusal) = &verdict.refusal {
        return vec![Line::new(Tone::Stop, refusal_text(refusal, &key, command))];
    }
    let mut lines = Vec::new();
    for holder in &verdict.taken {
        lines.push(Line::new(Tone::Stop, holder_text(holder, &verdict.key, names)));
    }
    for holder in &verdict.elsewhere {
        lines.push(Line::new(Tone::Plain, holder_text(holder, &verdict.key, names)));
    }
    for warning in &verdict.warnings {
        lines.push(Line::new(Tone::Warning, warning_text(warning, &key)));
    }
    if lines.is_empty() {
        lines.push(Line::new(
            Tone::Good,
            format!("No other command has {key} where \u{201c}{command}\u{201d} applies."),
        ));
    }
    lines
}

fn refusal_text(refusal: &Refusal, key: &str, command: &str) -> String {
    match refusal {
        Refusal::TypesText => format!(
            "{key} types a character, and text is typed where this command applies: the \
             character could no longer be typed there. Hold \u{2318}, \u{2303} or \u{2325} with it."
        ),
        Refusal::SystemKeeps(system) => format!(
            "macOS keeps {key} for itself: it {}. The key never reaches The Writer.",
            system.what
        ),
        Refusal::AlreadyHasKey => format!("\u{201c}{command}\u{201d} has {key} already."),
        Refusal::UnknownCommand => "This version cannot give this command a key.".to_string(),
    }
}

fn warning_text(warning: &Warning, key: &str) -> String {
    match warning {
        Warning::SpecialCharacter => format!(
            "On a Mac keyboard {key} types a special character or starts an accent. Text is \
             typed where this command applies, and that character could no longer be typed there."
        ),
        Warning::EditingKey => format!(
            "{key} is a key that text is edited with. Text is typed where this command \
             applies, and there the key would run the command instead."
        ),
        Warning::SystemUses(system) => format!(
            "macOS uses {key} itself: it {}. Unless that is switched off in System Settings, \
             the shortcut may not run.",
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
        Overlap::SamePlace => format!(
            "{key} is the key of {who} {place}. Only one of the two can have it there."
        ),
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
        Overlap::Apart => format!(
            "{key} is also the key of {who} {place}. That is a different place: both work."
        ),
    }
}

/// "Take It from “Italic”", "Take It from 2 Commands".
pub(crate) fn take_label(holders: &[Holder], names: &Names) -> String {
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

/// What an import would do, in a few sentences above its list.
pub(crate) fn import_summary(plan: &ImportPlan, outcome: &ImportOutcome) -> Vec<Line> {
    let mut lines = Vec::new();
    let changing = outcome.differences.len();
    if changing == 0 {
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
        let n = outcome.conflicts.len();
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

/// "The Writer 0.4.0 wrote it; 212 entries, 209 of them leave their
/// command as it is."
pub(crate) fn import_file_line(plan: &ImportPlan) -> String {
    let mut parts = Vec::new();
    if let Some(version) = &plan.app_version {
        parts.push(format!("Written by The Writer {version}"));
    }
    parts.push(count(plan.entries, "entry", "entries"));
    if plan.unchanged > 0 {
        parts.push(format!("{} without a change", plan.unchanged));
    }
    parts.join(" \u{b7} ")
}

/// A collision an import would leave, as a sentence.
pub(crate) fn collision_text(collision: &Collision, names: &Names) -> String {
    let key = glyphs(&collision.key);
    let one = names.title(&collision.target);
    let other = names.title(&collision.holder.target);
    let place = names.place(&collision.target);
    let other_place = names.place(&collision.holder.target);
    if place == other_place {
        format!("{key} would be the key of \u{201c}{one}\u{201d} and of \u{201c}{other}\u{201d} {place}.")
    } else {
        format!(
            "{key} would be the key of \u{201c}{one}\u{201d} {place} and of \u{201c}{other}\u{201d} {other_place}."
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use writer_keymap::{DefaultBinding, Defaults, Keymap, Places};

    fn keymap() -> Keymap {
        let binding = |action: &str, context: Option<&str>, keys: &str| {
            DefaultBinding::new(action, None, context, keys).unwrap()
        };
        let defaults = Defaults::from_bindings([
            binding("app::QuickOpen", None, "cmd-p"),
            binding("comments::Toggle", None, "cmd-alt-c"),
            binding("editor::ToggleBold", Some("Editor"), "cmd-b"),
            binding("editor::ToggleItalic", Some("Editor"), "cmd-i"),
            binding("tools::CaseSensitive", Some("FindBar > TextField"), "cmd-alt-c"),
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

    fn names_of(catalogue: &Catalogue) -> Names<'_> {
        Names {
            entries: &[],
            catalogue,
        }
    }

    fn catalogue() -> Catalogue {
        let mut catalogue = Catalogue::new();
        catalogue.add(Target::new("editor::ToggleBold", Some("Editor")), "Bold", "In the text");
        catalogue.add(
            Target::new("editor::ToggleItalic", Some("Editor")),
            "Italic",
            "In the text",
        );
        catalogue.add(Target::new("app::QuickOpen", None), "Quick open", "Anywhere");
        catalogue
    }

    fn key(text: &str) -> KeySequence {
        KeySequence::parse(text).unwrap()
    }

    #[test]
    fn a_free_key_is_said_to_be_free() {
        let keymap = keymap();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let verdict = keymap.check(&bold, &key("cmd-alt-b"), Some(&key("cmd-b")));
        assert_eq!(verdict_title(&verdict), "⌥⌘B is free");
        let catalogue = catalogue();
        let lines = verdict_lines(&verdict, "Bold", &names_of(&catalogue));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].tone, Tone::Good);
        assert!(lines[0].text.contains("“Bold”"), "{}", lines[0].text);
    }

    #[test]
    fn a_taken_key_names_the_command_that_has_it() {
        let keymap = keymap();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let verdict = keymap.check(&bold, &key("cmd-i"), Some(&key("cmd-b")));
        assert_eq!(verdict_title(&verdict), "⌘I is taken");
        let catalogue = catalogue();
        let names = names_of(&catalogue);
        let lines = verdict_lines(&verdict, "Bold", &names);
        assert_eq!(lines[0].tone, Tone::Stop);
        assert_eq!(
            lines[0].text,
            "⌘I is the key of “Italic” in the text. Only one of the two can have it there."
        );
        assert_eq!(take_label(&verdict.taken, &names), "Take It from “Italic”");
    }

    #[test]
    fn a_character_is_refused_with_the_reason() {
        let keymap = keymap();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let verdict = keymap.check(&bold, &key("b"), None);
        assert_eq!(verdict_title(&verdict), "B cannot be used");
        let catalogue = catalogue();
        let lines = verdict_lines(&verdict, "Bold", &names_of(&catalogue));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].tone, Tone::Stop);
        assert!(lines[0].text.starts_with("B types a character"), "{}", lines[0].text);
    }

    #[test]
    fn a_key_macos_keeps_is_refused_with_what_it_does() {
        let keymap = keymap();
        let bold = Target::new("editor::ToggleBold", Some("Editor"));
        let verdict = keymap.check(&bold, &key("cmd-tab"), None);
        let catalogue = catalogue();
        let lines = verdict_lines(&verdict, "Bold", &names_of(&catalogue));
        assert!(lines[0].text.starts_with("macOS keeps ⌘⇥ for itself"), "{}", lines[0].text);
    }

    #[test]
    fn a_command_without_a_name_gets_one_from_its_action() {
        let catalogue = Catalogue::new();
        let names = names_of(&catalogue);
        assert_eq!(names.title(&Target::new("editor::MoveWordLeft", Some("Editor"))), "Move word left");
        assert_eq!(names.place(&Target::new("editor::MoveWordLeft", Some("Editor"))), "in Editor");
        assert_eq!(names.place(&Target::new("app::Quit", None)), "anywhere");
    }

    #[test]
    fn counts_are_singular_and_plural() {
        assert_eq!(count(1, "key", "keys"), "1 key");
        assert_eq!(count(0, "key", "keys"), "0 keys");
        assert_eq!(count(12, "entry", "entries"), "12 entries");
    }
}
