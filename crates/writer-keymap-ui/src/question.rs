//! What the view asks the user: whether to use the key that was pressed,
//! whether to put everything back, what to do with a file. One shape for
//! all of them: a few sentences and a row of choices, one of which is
//! marked and taken with the key that chooses.

use gpui::{ScrollHandle, SharedString};
#[cfg(test)]
use writer_keymap::Catalogue;
use writer_keymap::{
    ImportError, ImportMode, ImportPlan, KeySequence, SkipReason, Skipped, Target, Verdict,
    VerdictKind,
};

use crate::{
    keys::{glyph_list, glyphs},
    words::{self, Line, Names, SeenFirst, Tone},
};

/// One of the answers the view offers to a question.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    /// Use the key that was pressed; nothing is in the way.
    UseKey,
    /// Use the key and take it from the commands that have it.
    TakeKey,
    /// Use the key and leave it with the commands that have it in another
    /// place.
    KeepBoth,
    /// Go back to waiting for a key.
    PressAnotherKey,
    /// Put every key back to its default.
    ResetAll,
    /// Import the file, adding its changes to the user's own (the file
    /// wins where both change a command).
    ImportMerge,
    /// Import the file in the place of the user's own changes.
    ImportReplace,
    /// Choose another file to import.
    ChooseFile,
    /// Leave things as they are.
    Cancel,
}

/// A choice as it is drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ChoiceButton {
    pub choice: Choice,
    pub label: SharedString,
}

fn button(choice: Choice, label: impl Into<SharedString>) -> ChoiceButton {
    ChoiceButton {
        choice,
        label: label.into(),
    }
}

/// What a question is about.
pub(crate) enum Subject {
    /// The key pressed for a command.
    Key {
        target: Target,
        replacing: Option<KeySequence>,
        verdict: Box<Verdict>,
    },
    /// Putting everything back.
    ResetAll,
    /// A file that is not a shortcuts file.
    ImportError,
    /// A shortcuts file, read and planned.
    Import(Box<Preview>),
}

/// See the [module docs](self).
pub(crate) struct Question {
    pub subject: Subject,
    pub title: SharedString,
    /// How the title is to be read: the mark beside it.
    pub tone: Tone,
    pub lines: Vec<Line>,
    pub choices: Vec<ChoiceButton>,
    pub selected: usize,
}

impl Question {
    /// What to do with the key that was pressed for `target`, given the
    /// keymap's verdict and what the host said sees such a key first.
    ///
    /// The marked choice (the one a bare confirmation takes) is the first
    /// one, which gives the command the key, unless a sentence warns: then
    /// it is "Press Another Key", and giving the key takes a deliberate
    /// step.
    pub fn about_key(
        target: Target,
        replacing: Option<KeySequence>,
        verdict: Verdict,
        seen_first: &[SeenFirst],
        names: &Names,
    ) -> Self {
        let key = glyphs(&verdict.key);
        let mut choices = Vec::new();
        match verdict.kind() {
            VerdictKind::Fine => choices.push(button(Choice::UseKey, format!("Use {key}"))),
            // Only commands in places of their own have the key: nothing
            // to settle.
            VerdictKind::UsedElsewhere if words::overlapping(&verdict).next().is_none() => {
                choices.push(button(Choice::UseKey, format!("Use {key}")))
            }
            VerdictKind::UsedElsewhere => {
                choices.push(button(Choice::KeepBoth, "Keep Both"));
                choices.push(button(
                    Choice::TakeKey,
                    words::take_label(words::overlapping(&verdict), names),
                ));
            }
            VerdictKind::Taken => choices.push(button(
                Choice::TakeKey,
                words::take_label(&verdict.taken, names),
            )),
            VerdictKind::Refused => {}
        }
        choices.push(button(Choice::PressAnotherKey, "Press Another Key"));
        choices.push(button(Choice::Cancel, "Cancel"));
        let says = words::verdict_words(&verdict, &target, seen_first, names);
        let selected = if says.warns() {
            choices
                .iter()
                .position(|button| button.choice == Choice::PressAnotherKey)
                .unwrap_or(0)
        } else {
            0
        };
        Self {
            title: says.title.into(),
            tone: says.tone,
            lines: says.lines,
            subject: Subject::Key {
                target,
                replacing,
                verdict: Box::new(verdict),
            },
            choices,
            selected,
        }
    }

    /// Whether to put everything back, for a keymap in which `changed`
    /// commands differ from their defaults.
    pub fn about_reset(changed: usize) -> Self {
        let text = if changed == 0 {
            "No key differs from its default in this version. The changes kept from another \
             version, which do nothing in this one, are removed."
                .to_string()
        } else {
            format!(
                "{} {} back to the default keys. Every change you made is removed.",
                words::count(changed, "command", "commands"),
                if changed == 1 { "goes" } else { "go" }
            )
        };
        Self {
            subject: Subject::ResetAll,
            title: "Put every key back to its default?".into(),
            tone: Tone::Plain,
            lines: vec![Line {
                tone: Tone::Warning,
                text,
            }],
            choices: vec![
                button(Choice::ResetAll, "Reset All"),
                button(Choice::Cancel, "Cancel"),
            ],
            selected: 0,
        }
    }

    /// A file that could not be read as a shortcuts file.
    pub fn about_import_error(file: &str, error: &ImportError) -> Self {
        Self {
            subject: Subject::ImportError,
            title: format!("\u{201c}{file}\u{201d} was not imported").into(),
            tone: Tone::Plain,
            lines: vec![Line {
                tone: Tone::Stop,
                text: error.to_string(),
            }],
            choices: vec![
                button(Choice::ChooseFile, "Choose Another File\u{2026}"),
                button(Choice::Cancel, "Close"),
            ],
            selected: 0,
        }
    }

    /// The preview of an import.
    pub fn about_import(file: &str, plan: ImportPlan, names: &Names) -> Self {
        let nothing = plan.outcome(ImportMode::Merge).differences.is_empty()
            && plan.outcome(ImportMode::Replace).differences.is_empty();
        let mut choices = Vec::new();
        if nothing {
            choices.push(button(Choice::Cancel, "Close"));
        } else if plan.modes_differ() {
            choices.push(button(Choice::ImportMerge, "Add to My Changes"));
            choices.push(button(Choice::ImportReplace, "Replace My Changes"));
            choices.push(button(Choice::Cancel, "Cancel"));
        } else {
            // Nothing of the user's own to keep or to lose: one way in.
            choices.push(button(Choice::ImportMerge, "Import"));
            choices.push(button(Choice::Cancel, "Cancel"));
        }
        let mut preview = Preview {
            plan,
            mode: ImportMode::Merge,
            rows: Vec::new(),
            changing: 0,
            colliding: 0,
            selected: None,
            scroll: ScrollHandle::new(),
        };
        preview.rebuild(names);
        let mut question = Self {
            title: format!("Import \u{201c}{file}\u{201d}").into(),
            tone: Tone::Plain,
            lines: Vec::new(),
            subject: Subject::Import(Box::new(preview)),
            choices,
            selected: 0,
        };
        question.refresh_import_lines();
        question
    }

    /// The choice that is marked.
    pub fn selected_choice(&self) -> Choice {
        self.choices[self.selected.min(self.choices.len() - 1)].choice
    }

    /// Marks the choice at `index`; an import preview then shows what that
    /// way of importing would do.
    pub fn select(&mut self, index: usize, names: &Names) {
        self.selected = index.min(self.choices.len() - 1);
        let mode = match self.selected_choice() {
            Choice::ImportMerge => Some(ImportMode::Merge),
            Choice::ImportReplace => Some(ImportMode::Replace),
            _ => None,
        };
        if let (Some(mode), Subject::Import(preview)) = (mode, &mut self.subject)
            && preview.mode != mode
        {
            preview.mode = mode;
            preview.rebuild(names);
            self.refresh_import_lines();
        }
    }

    fn refresh_import_lines(&mut self) {
        if let Subject::Import(preview) = &self.subject {
            self.lines = words::import_summary(
                &preview.plan,
                preview.plan.outcome(preview.mode),
                preview.changing,
                preview.colliding,
            );
        }
    }
}

/// One row of the import preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PreviewRow {
    /// A heading above a group of rows.
    Header(String),
    /// A command whose keys change.
    Change {
        title: String,
        place: String,
        before: String,
        after: String,
        /// The command in each key context the file changes it in.
        targets: Vec<Target>,
    },
    /// A collision the import would leave.
    Collision {
        text: String,
        /// The two commands.
        targets: Vec<Target>,
    },
    /// An entry of the file that is skipped, with the reason.
    Skipped(SkippedRow),
}

/// An entry of the file that is skipped, as its row shows it: what the
/// command is called first, then why it is skipped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SkippedRow {
    /// What the command is called here; for a command this version does
    /// not have, what the file calls it (its title and the action's name:
    /// that name is all there is to go by); "Entry 12" for an entry that
    /// names nothing.
    pub title: String,
    /// The place written in the entry, where the reason is about it.
    pub place: Option<String>,
    /// Why it is skipped, in the keymap's words.
    pub reason: String,
    /// Which entry of the file, from 1.
    pub entry: usize,
    /// The action's name as the file has it.
    pub command: Option<String>,
}

impl SkippedRow {
    fn new(skipped: &Skipped, names: &Names) -> Self {
        let known = skipped
            .command
            .as_deref()
            .and_then(|action| names.title_of_action(action));
        let title = match (known, &skipped.title, &skipped.command) {
            (Some(title), _, _) => title,
            (None, Some(title), Some(command)) => format!("{title} ({command})"),
            (None, Some(name), None) | (None, None, Some(name)) => name.clone(),
            (None, None, None) => format!("Entry {}", skipped.entry),
        };
        Self {
            title,
            place: match skipped.reason {
                SkipReason::UnknownPlace => skipped.place.as_deref().map(words::lower_first),
                _ => None,
            },
            reason: skipped.reason.to_string(),
            entry: skipped.entry,
            command: skipped.command.clone(),
        }
    }

    /// The reason as it stands beside the title: a sentence of its own.
    pub fn reason_shown(&self) -> String {
        words::upper_first(&self.reason)
    }
}

/// `"editor::ToggleBold" in "Editor"`: a command as a shortcuts file
/// names it.
fn as_the_file_names(target: &Target) -> String {
    match target.context.text() {
        Some(context) => format!("\"{}\" in \"{context}\"", target.command.action),
        None => format!("\"{}\"", target.command.action),
    }
}

impl PreviewRow {
    /// The row as one sentence, as it reads on screen: what a test reads.
    pub fn text(&self) -> String {
        match self {
            PreviewRow::Header(text) | PreviewRow::Collision { text, .. } => text.clone(),
            PreviewRow::Change {
                title,
                place,
                before,
                after,
                ..
            } => format!("{title} ({place}): {before} \u{2192} {after}"),
            PreviewRow::Skipped(row) => match &row.place {
                Some(place) => format!("{} ({place}): {}", row.title, row.reason),
                None => format!("{}: {}", row.title, row.reason),
            },
        }
    }

    /// What the line under the rows says while this row is selected: how
    /// the file names it, for whoever edits the file. The row itself has
    /// said the rest.
    pub fn detail(&self) -> Option<String> {
        let named = |targets: &[Target]| {
            let mut names: Vec<String> = Vec::new();
            for target in targets {
                let name = as_the_file_names(target);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            names.join(" and ")
        };
        match self {
            PreviewRow::Header(_) => None,
            PreviewRow::Change { targets, .. } | PreviewRow::Collision { targets, .. } => {
                Some(format!("In the file: {}", named(targets)))
            }
            PreviewRow::Skipped(row) => Some(match &row.command {
                Some(command) => format!("In the file: entry {}, \"{command}\"", row.entry),
                None => format!("In the file: entry {}", row.entry),
            }),
        }
    }

    pub fn is_header(&self) -> bool {
        matches!(self, PreviewRow::Header(_))
    }
}

/// The preview of an import: the plan, and what one way of carrying it out
/// would do, row by row.
pub(crate) struct Preview {
    pub plan: ImportPlan,
    /// The way of importing whose outcome the rows show.
    pub mode: ImportMode,
    pub rows: Vec<PreviewRow>,
    /// How many commands get other keys, and how many collisions would be
    /// left, as the rows show them: a command that the file changes in
    /// several key contexts of one place, the same way, is one row.
    pub changing: usize,
    pub colliding: usize,
    pub selected: Option<usize>,
    /// The rows scroll as a whole: a row is as tall as its text.
    pub scroll: ScrollHandle,
}

impl Preview {
    /// Builds the rows for the outcome of the current mode.
    pub fn rebuild(&mut self, names: &Names) {
        let outcome = self.plan.outcome(self.mode);
        let mut rows = Vec::new();
        // A command bound in several key contexts of one place is in the
        // file once for each; where they change the same way they are one
        // row here, as they are one row of the list.
        let mut changes: Vec<PreviewRow> = Vec::new();
        for difference in &outcome.differences {
            let shown = (
                names.title(&difference.target),
                names.place(&difference.target),
                glyph_list(&difference.before),
                glyph_list(&difference.after),
            );
            let same = changes.iter_mut().find_map(|row| match row {
                PreviewRow::Change {
                    title,
                    place,
                    before,
                    after,
                    targets,
                } if (&*title, &*place, &*before, &*after)
                    == (&shown.0, &shown.1, &shown.2, &shown.3) =>
                {
                    Some(targets)
                }
                _ => None,
            });
            match same {
                Some(targets) => targets.push(difference.target.clone()),
                None => changes.push(PreviewRow::Change {
                    title: shown.0,
                    place: shown.1,
                    before: shown.2,
                    after: shown.3,
                    targets: vec![difference.target.clone()],
                }),
            }
        }
        let mut collisions: Vec<PreviewRow> = Vec::new();
        for collision in &outcome.conflicts {
            let text = words::collision_text(collision, names);
            let same = collisions.iter().any(
                |row| matches!(row, PreviewRow::Collision { text: have, .. } if *have == text),
            );
            if !same {
                collisions.push(PreviewRow::Collision {
                    text,
                    targets: vec![collision.target.clone(), collision.holder.target.clone()],
                });
            }
        }
        self.changing = changes.len();
        self.colliding = collisions.len();
        if !changes.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Commands that get other keys ({})",
                changes.len()
            )));
            rows.extend(changes);
        }
        if !collisions.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Collisions it would leave ({})",
                collisions.len()
            )));
            rows.extend(collisions);
        }
        if !self.plan.skipped.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Entries that are skipped ({})",
                self.plan.skipped.len()
            )));
            for skipped in &self.plan.skipped {
                rows.push(PreviewRow::Skipped(SkippedRow::new(skipped, names)));
            }
        }
        self.rows = rows;
        self.selected = self.rows.iter().position(|row| !row.is_header());
        // What another way of importing shows starts at its top.
        self.scroll.set_offset(gpui::Point::default());
    }

    /// Moves the selection by `delta` rows, over the headings.
    pub fn move_selection(&mut self, delta: isize) {
        let selectable: Vec<usize> = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| !row.is_header())
            .map(|(ix, _)| ix)
            .collect();
        if selectable.is_empty() {
            return;
        }
        let at = self
            .selected
            .and_then(|selected| selectable.iter().position(|ix| *ix == selected))
            .unwrap_or(0) as isize;
        let next = at
            .saturating_add(delta)
            .clamp(0, selectable.len() as isize - 1) as usize;
        self.selected = Some(selectable[next]);
    }
}

#[cfg(test)]
mod tests {
    //! Every kind of answer the view gives about a key: the heading, its
    //! mark, the sentences, the choices, and the marked choice.

    use super::*;
    use crate::words::tests::{catalogue, key, keymap, names_of};

    struct Asked {
        title: String,
        mark: Tone,
        lines: Vec<(Tone, String)>,
        choices: Vec<String>,
        marked: String,
    }

    impl Asked {
        fn says(&self) -> String {
            self.lines
                .iter()
                .map(|(_, text)| text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        }

        fn tones(&self) -> Vec<Tone> {
            self.lines.iter().map(|(tone, _)| *tone).collect()
        }

        /// What every answer has to hold: a heading that says "free", or
        /// one marked as good, never stands above a warning; and when a
        /// sentence warns, confirming does not give the key.
        fn check(&self) {
            let warns = self.tones().contains(&Tone::Warning);
            let stops = self.tones().contains(&Tone::Stop);
            if warns || stops {
                assert!(!self.title.contains("free"), "{}", self.title);
                assert_ne!(self.mark, Tone::Good, "{}", self.title);
            }
            if warns {
                assert_eq!(self.marked, "Press Another Key", "{}", self.title);
            }
            assert!(self.choices.contains(&"Cancel".to_string()));
            assert!(self.choices.contains(&"Press Another Key".to_string()));
        }
    }

    fn bold() -> Target {
        Target::new("editor::ToggleBold", Some("Editor"))
    }

    fn quick_open() -> Target {
        Target::new("app::QuickOpen", None)
    }

    fn vim() -> Vec<SeenFirst> {
        vec![SeenFirst {
            name: Some("Vim mode".into()),
            warning: "In the text Vim sees this key before any shortcut does.".into(),
        }]
    }

    fn ask(target: &Target, keys: &str, seen_first: &[SeenFirst]) -> Asked {
        let keymap = keymap();
        let catalogue = catalogue();
        let names = names_of(&catalogue);
        let verdict = keymap.check(target, &key(keys), None);
        let question = Question::about_key(target.clone(), None, verdict, seen_first, &names);
        let asked = Asked {
            title: question.title.to_string(),
            mark: question.tone,
            lines: question
                .lines
                .iter()
                .map(|line| (line.tone, line.text.clone()))
                .collect(),
            choices: question
                .choices
                .iter()
                .map(|button| button.label.to_string())
                .collect(),
            marked: question.choices[question.selected].label.to_string(),
        };
        asked.check();
        asked
    }

    #[test]
    fn a_free_key() {
        let asked = ask(&bold(), "cmd-alt-b", &[]);
        assert_eq!(asked.title, "⌥⌘B is free");
        assert_eq!(asked.mark, Tone::Good);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Good,
                "No other command has ⌥⌘B where “Bold” applies.".to_string()
            )]
        );
        assert_eq!(asked.choices, ["Use ⌥⌘B", "Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Use ⌥⌘B");
    }

    #[test]
    fn a_key_only_commands_in_places_of_their_own_have() {
        // The find bar has ⌘G; the text is another place.
        let asked = ask(&bold(), "cmd-g", &[]);
        assert_eq!(asked.title, "⌘G is free in the text");
        assert_eq!(asked.mark, Tone::Good);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Plain,
                "⌘G is also the key of “Find next” in the find bar. That is a different place: \
                 both work."
                    .to_string()
            )]
        );
        assert_eq!(asked.choices, ["Use ⌘G", "Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Use ⌘G");
    }

    #[test]
    fn a_key_another_command_has_where_both_apply_and_this_one_gets() {
        // The window has ⌥⌘C for the comments; the text lies inside the
        // window and wins there. The find bar has it too, apart.
        let asked = ask(&bold(), "cmd-alt-c", &[]);
        assert_eq!(asked.title, "⌥⌘C is also the key of another command");
        assert_eq!(asked.mark, Tone::Plain);
        assert_eq!(asked.tones(), [Tone::Plain, Tone::Plain]);
        assert_eq!(
            asked.lines[0].1,
            "⌥⌘C is also the key of “Show or hide comments” anywhere. Where both apply this \
             command gets it, and “Show or hide comments” keeps it in the rest of its place."
        );
        assert!(
            asked.lines[1]
                .1
                .contains("“Case sensitive” in the find bar"),
            "{}",
            asked.lines[1].1
        );
        assert_eq!(
            asked.choices,
            [
                "Keep Both",
                "Take It from “Show or hide comments”",
                "Press Another Key",
                "Cancel"
            ]
        );
        assert_eq!(asked.marked, "Keep Both");
    }

    #[test]
    fn a_key_another_command_keeps_where_both_apply() {
        // A command of the window is given the key Bold has in the text:
        // in the text Bold keeps it.
        let asked = ask(&quick_open(), "cmd-b", &[]);
        assert_eq!(asked.title, "⌘B would not work everywhere");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Warning,
                "⌘B is also the key of “Bold” in the text. Where both apply “Bold” keeps it, and \
                 this command gets it in the rest of its place."
                    .to_string()
            )]
        );
        assert_eq!(
            asked.choices,
            [
                "Keep Both",
                "Take It from “Bold”",
                "Press Another Key",
                "Cancel"
            ]
        );
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_taken_key() {
        let asked = ask(&bold(), "cmd-i", &[]);
        assert_eq!(asked.title, "⌘I is taken");
        assert_eq!(asked.mark, Tone::Stop);
        assert_eq!(asked.tones(), [Tone::Stop]);
        assert_eq!(
            asked.choices,
            ["Take It from “Italic”", "Press Another Key", "Cancel"]
        );
        // The button says what it costs: it is the marked one.
        assert_eq!(asked.marked, "Take It from “Italic”");
    }

    #[test]
    fn a_character_is_refused() {
        let asked = ask(&bold(), "b", &[]);
        assert_eq!(asked.title, "B cannot be used");
        assert_eq!(asked.mark, Tone::Stop);
        assert_eq!(asked.tones(), [Tone::Stop]);
        assert!(
            asked.says().starts_with("B types a character"),
            "{}",
            asked.says()
        );
        assert_eq!(asked.choices, ["Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_key_macos_keeps_is_refused() {
        let asked = ask(&bold(), "cmd-tab", &[]);
        assert_eq!(asked.title, "macOS keeps ⌘⇥");
        assert_eq!(asked.mark, Tone::Stop);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Stop,
                "In macOS ⌘⇥ switches between apps, and no setting changes that: the key never \
                 reaches The Writer."
                    .to_string()
            )]
        );
        assert_eq!(asked.choices, ["Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Press Another Key");
        // Whatever else could be said about it is left out.
        let asked = ask(&bold(), "cmd-tab", &vim());
        assert_eq!(asked.lines.len(), 1);
    }

    #[test]
    fn the_key_the_command_has_already() {
        let asked = ask(&bold(), "cmd-b", &[]);
        assert_eq!(asked.title, "“Bold” has ⌘B already");
        assert_eq!(asked.mark, Tone::Plain);
        assert_eq!(
            asked.lines,
            vec![(Tone::Plain, "Nothing would change.".to_string())]
        );
        assert_eq!(asked.choices, ["Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_key_macos_uses() {
        let asked = ask(&bold(), "cmd-shift-5", &[]);
        assert_eq!(asked.title, "macOS uses ⇧⌘5");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Warning,
                "In macOS ⇧⌘5 opens the screenshot tools. Unless that is switched off in System \
                 Settings, the shortcut may not run."
                    .to_string()
            )]
        );
        assert_eq!(asked.choices, ["Use ⇧⌘5", "Press Another Key", "Cancel"]);
        // The key will probably not run: confirming does not take it.
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_key_something_of_the_host_sees_first() {
        let asked = ask(&bold(), "ctrl-y", &vim());
        assert_eq!(asked.title, "⌃Y: Vim mode sees it first");
        assert_eq!(asked.mark, Tone::Warning);
        // The sentence of the host alone: not "no other command has it"
        // with a tick above it.
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Warning,
                "In the text Vim sees this key before any shortcut does.".to_string()
            )]
        );
        assert_eq!(asked.choices, ["Use ⌃Y", "Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Press Another Key");

        // A host that did not say what it is called.
        let nameless = [SeenFirst {
            name: None,
            warning: "Something else sees this key first.".into(),
        }];
        let asked = ask(&bold(), "ctrl-y", &nameless);
        assert_eq!(asked.title, "⌃Y may not run");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_key_text_is_edited_with() {
        let asked = ask(&bold(), "tab", &[]);
        assert_eq!(asked.title, "⇥ is used for typing");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(
            asked.lines,
            vec![(
                Tone::Warning,
                "Text is typed where this command applies. As a shortcut, ⇥ would run the command \
                 there instead of what it does in a text."
                    .to_string()
            )]
        );
        assert_eq!(asked.choices, ["Use ⇥", "Press Another Key", "Cancel"]);
        // Return pressed twice by somebody who meant to confirm must not
        // give Return to the command.
        assert_eq!(asked.marked, "Press Another Key");
        assert_eq!(ask(&bold(), "enter", &[]).marked, "Press Another Key");
    }

    #[test]
    fn option_with_a_character() {
        let asked = ask(&bold(), "alt-b", &[]);
        assert_eq!(asked.title, "⌥B types a special character");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(asked.tones(), [Tone::Warning]);
        assert!(
            asked
                .says()
                .ends_with("as a shortcut, ⌥B could no longer type its character there."),
            "{}",
            asked.says()
        );
        assert_eq!(asked.choices, ["Use ⌥B", "Press Another Key", "Cancel"]);
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_taken_key_that_macos_uses_too() {
        // "Complete" has ⌘Space in the text, and macOS opens Spotlight
        // with it.
        let asked = ask(&bold(), "cmd-space", &[]);
        assert_eq!(asked.title, "⌘Space is taken");
        assert_eq!(asked.mark, Tone::Stop);
        assert_eq!(asked.tones(), [Tone::Stop, Tone::Warning]);
        assert!(
            asked.lines[1]
                .1
                .starts_with("In macOS ⌘Space opens Spotlight.")
        );
        assert_eq!(
            asked.choices,
            ["Take It from “Complete”", "Press Another Key", "Cancel"]
        );
        // Taking it would still leave a key that may not run.
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_taken_key_that_the_host_sees_first() {
        let keymap = keymap();
        let catalogue = catalogue();
        let names = names_of(&catalogue);
        let italic = Target::new("editor::ToggleItalic", Some("Editor"));
        // Italic on a key Vim may take; then Bold is given the same key.
        let mut keymap = keymap;
        assert!(keymap.replace_key(&italic, &key("cmd-i"), key("ctrl-y")));
        let verdict = keymap.check(&bold(), &key("ctrl-y"), None);
        let question = Question::about_key(bold(), None, verdict, &vim(), &names);
        assert_eq!(question.title.as_ref(), "⌃Y is taken");
        assert_eq!(question.tone, Tone::Stop);
        let tones: Vec<Tone> = question.lines.iter().map(|line| line.tone).collect();
        assert_eq!(tones, [Tone::Stop, Tone::Warning]);
        assert_eq!(question.selected_choice(), Choice::PressAnotherKey);
    }

    #[test]
    fn macos_comes_before_the_host_and_both_are_said() {
        // ⌃↑ opens Mission Control, and Vim may take it as well.
        let asked = ask(&bold(), "ctrl-up", &vim());
        assert_eq!(asked.title, "macOS uses ⌃↑");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(asked.tones(), [Tone::Warning, Tone::Warning]);
        assert!(
            asked.lines[0]
                .1
                .starts_with("In macOS ⌃↑ opens Mission Control.")
        );
        assert!(asked.lines[1].1.contains("Vim sees this key"));
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_warning_comes_before_what_only_is_to_know() {
        // ⌥⌘C for Bold with Vim on: the comments of the window have it
        // (Bold wins in the text), the find bar has it apart, and Vim may
        // take it. The heading is the warning, the rest follows.
        let asked = ask(&bold(), "cmd-alt-c", &vim());
        assert_eq!(asked.title, "⌥⌘C: Vim mode sees it first");
        assert_eq!(asked.mark, Tone::Warning);
        assert_eq!(asked.tones(), [Tone::Warning, Tone::Plain, Tone::Plain]);
        assert_eq!(
            asked.choices,
            [
                "Keep Both",
                "Take It from “Show or hide comments”",
                "Press Another Key",
                "Cancel"
            ]
        );
        assert_eq!(asked.marked, "Press Another Key");
    }

    #[test]
    fn a_free_key_named_like_its_neighbours_place() {
        // A command of the find bar's field, and a key another command of
        // "the find bar" has in a key context that lies apart: the heading
        // does not say "free in the find bar" above "... in the find bar".
        let keymap = {
            use writer_keymap::{DefaultBinding, Defaults, Keymap, Places};
            let binding = |action: &str, context: &str, keys: &str| {
                DefaultBinding::new(action, None, Some(context), keys).unwrap()
            };
            Keymap::new(
                Defaults::from_bindings([
                    binding("chat::Send", "ChatComposer", "cmd-enter"),
                    binding("chat::Copy", "ChatList", "cmd-shift-c"),
                ]),
                Places::new().typing("ChatComposer"),
            )
        };
        let mut catalogue = Catalogue::new();
        let send = Target::new("chat::Send", Some("ChatComposer"));
        catalogue.add(send.clone(), "Send", "In the chat");
        catalogue.add(
            Target::new("chat::Copy", Some("ChatList")),
            "Copy answer",
            "In the chat",
        );
        let names = names_of(&catalogue);
        let verdict = keymap.check(&send, &key("cmd-shift-c"), None);
        if verdict.kind() == VerdictKind::UsedElsewhere
            && words::overlapping(&verdict).next().is_none()
        {
            let question = Question::about_key(send, None, verdict, &[], &names);
            assert_eq!(question.title.as_ref(), "⇧⌘C is free here");
            assert_eq!(question.tone, Tone::Good);
        } else {
            panic!("the fixture's two key contexts are apart: {verdict:?}");
        }
    }
}
