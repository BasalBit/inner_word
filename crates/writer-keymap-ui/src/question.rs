//! What the view asks the user: whether to use the key that was pressed,
//! whether to put everything back, what to do with a file. One shape for
//! all of them: a few sentences and a row of choices, one of which is
//! marked and taken with the key that chooses.

use gpui::{SharedString, UniformListScrollHandle};
use writer_keymap::{ImportError, ImportMode, ImportPlan, KeySequence, Target, Verdict, VerdictKind};

use crate::{
    keys::{glyph_list, glyphs},
    words::{self, Line, Names, Tone},
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
    pub lines: Vec<Line>,
    pub choices: Vec<ChoiceButton>,
    pub selected: usize,
}

impl Question {
    /// The verdict on the key pressed for the command called `title`.
    pub fn about_key(
        target: Target,
        replacing: Option<KeySequence>,
        verdict: Verdict,
        title: &str,
        names: &Names,
    ) -> Self {
        let key = glyphs(&verdict.key);
        let mut choices = Vec::new();
        match verdict.kind() {
            VerdictKind::Fine => choices.push(button(Choice::UseKey, format!("Use {key}"))),
            VerdictKind::UsedElsewhere => {
                choices.push(button(Choice::KeepBoth, "Keep Both"));
                choices.push(button(
                    Choice::TakeKey,
                    words::take_label(&verdict.elsewhere, names),
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
        Self {
            title: words::verdict_title(&verdict).into(),
            lines: words::verdict_lines(&verdict, title, names),
            subject: Subject::Key {
                target,
                replacing,
                verdict: Box::new(verdict),
            },
            choices,
            selected: 0,
        }
    }

    /// Whether to put everything back, for a keymap in which `changed`
    /// commands differ from their defaults.
    pub fn about_reset(changed: usize) -> Self {
        let text = if changed == 0 {
            "No key differs from its default in this version. What was kept of changes this \
             version cannot use is removed."
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
            selected: None,
            scroll: UniformListScrollHandle::new(),
        };
        preview.rebuild(names);
        let mut question = Self {
            title: format!("Import \u{201c}{file}\u{201d}").into(),
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
            self.lines = words::import_summary(&preview.plan, preview.plan.outcome(preview.mode));
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
    },
    /// A collision the import would leave.
    Collision(String),
    /// An entry of the file that is skipped, with the reason.
    Skipped(String),
}

impl PreviewRow {
    /// The row as one sentence: what the line under the list shows in
    /// full for the selected row, and what a test reads.
    pub fn text(&self) -> String {
        match self {
            PreviewRow::Header(text) | PreviewRow::Collision(text) | PreviewRow::Skipped(text) => {
                text.clone()
            }
            PreviewRow::Change {
                title,
                place,
                before,
                after,
            } => format!("{title} ({place}): {before} \u{2192} {after}"),
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
    pub selected: Option<usize>,
    pub scroll: UniformListScrollHandle,
}

impl Preview {
    /// Builds the rows for the outcome of the current mode.
    pub fn rebuild(&mut self, names: &Names) {
        let outcome = self.plan.outcome(self.mode);
        let mut rows = Vec::new();
        if !outcome.differences.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Keys that change ({})",
                outcome.differences.len()
            )));
            for difference in &outcome.differences {
                rows.push(PreviewRow::Change {
                    title: names.title(&difference.target),
                    place: names.place(&difference.target),
                    before: glyph_list(&difference.before),
                    after: glyph_list(&difference.after),
                });
            }
        }
        if !outcome.conflicts.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Collisions it would leave ({})",
                outcome.conflicts.len()
            )));
            for collision in &outcome.conflicts {
                rows.push(PreviewRow::Collision(words::collision_text(collision, names)));
            }
        }
        if !self.plan.skipped.is_empty() {
            rows.push(PreviewRow::Header(format!(
                "Entries that are skipped ({})",
                self.plan.skipped.len()
            )));
            for skipped in &self.plan.skipped {
                rows.push(PreviewRow::Skipped(skipped.to_string()));
            }
        }
        self.rows = rows;
        self.selected = self.rows.iter().position(|row| !row.is_header());
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
        let next = (at + delta).clamp(0, selectable.len() as isize - 1) as usize;
        self.selected = Some(selectable[next]);
    }
}
