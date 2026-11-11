//! The command palette (`cmd-shift-p`): every registered command, fuzzy
//! searchable, with its real key binding beside it.
//!
//! Without a query the commands run most recently come first, then
//! everything grouped by category. Confirming a command closes the palette,
//! which returns focus to where it was, and dispatches the command's action
//! from there.

use gpui::{App, Context, SharedString, WeakEntity, Window};
use writer_ui::{
    CommandDescriptor, Picker, PickerDelegate, PickerEntry, PickerItem, fuzzy::fuzzy_match,
};

use crate::{commands, workspace::Workspace};

/// The palette as a modal view.
pub type CommandPalette = Picker<PaletteDelegate>;

/// A title match outranks a match on the category or the keywords.
const TITLE_BONUS: i32 = 10_000;
/// A small nudge for commands used recently, to break ties.
const RECENT_BONUS: i32 = 40;

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Header(SharedString),
    Command { index: usize, positions: Vec<usize> },
}

/// The picker delegate behind the command palette.
pub struct PaletteDelegate {
    workspace: WeakEntity<Workspace>,
    commands: Vec<CommandDescriptor>,
    /// Labels ("Category: Title") of recently run commands, newest first.
    recent: Vec<String>,
    rows: Vec<Row>,
}

impl PaletteDelegate {
    /// A delegate over every command in the registry.
    pub fn new(workspace: WeakEntity<Workspace>, recent: Vec<String>, cx: &App) -> Self {
        Self::with_commands(workspace, commands::all(cx), recent)
    }

    /// A delegate over the given commands.
    pub fn with_commands(
        workspace: WeakEntity<Workspace>,
        commands: Vec<CommandDescriptor>,
        recent: Vec<String>,
    ) -> Self {
        Self {
            workspace,
            commands,
            recent,
            rows: Vec::new(),
        }
    }

    /// The titles of the listed commands, in order (headers excluded).
    pub fn listed_titles(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                Row::Command { index, .. } => Some(self.commands[*index].title.to_string()),
                Row::Header(_) => None,
            })
            .collect()
    }

    fn recent_rank(&self, command: &CommandDescriptor) -> Option<usize> {
        let label = command.label();
        self.recent.iter().position(|recent| *recent == label)
    }

    fn browse_rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut recent: Vec<(usize, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| self.recent_rank(command).map(|rank| (rank, index)))
            .collect();
        recent.sort();
        if !recent.is_empty() {
            rows.push(Row::Header("Recently used".into()));
            rows.extend(recent.iter().map(|(_, index)| Row::Command {
                index: *index,
                positions: Vec::new(),
            }));
        }
        for category in commands::categories(&self.commands) {
            rows.push(Row::Header(category.clone()));
            rows.extend(
                self.commands
                    .iter()
                    .enumerate()
                    .filter(|(_, command)| command.category == category)
                    .map(|(index, _)| Row::Command {
                        index,
                        positions: Vec::new(),
                    }),
            );
        }
        rows
    }

    fn search_rows(&self, query: &str) -> Vec<Row> {
        let mut matches: Vec<(i32, usize, Vec<usize>)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let bonus = if self.recent_rank(command).is_some() {
                    RECENT_BONUS
                } else {
                    0
                };
                if let Some(found) = fuzzy_match(query, &command.title) {
                    return Some((found.score + TITLE_BONUS + bonus, index, found.positions));
                }
                // Category and keywords match by whole query words only: a
                // loose subsequence over that much text matches nearly
                // everything.
                words_match(query, &command.search_text()).then(|| (bonus, index, Vec::new()))
            })
            .collect();
        matches.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        matches
            .into_iter()
            .map(|(_, index, positions)| Row::Command { index, positions })
            .collect()
    }
}

/// Whether every whitespace-separated word of `query` occurs in `text`,
/// ignoring case. Used for the parts of a command that are not shown with
/// match highlights (category, keywords).
pub fn words_match(query: &str, text: &str) -> bool {
    let text = text.to_lowercase();
    let mut words = query.split_whitespace().peekable();
    words.peek().is_some() && words.all(|word| text.contains(&word.to_lowercase()))
}

impl PickerDelegate for PaletteDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.rows = if query.trim().is_empty() {
            self.browse_rows()
        } else {
            self.search_rows(query)
        };
    }

    fn entry_count(&self) -> usize {
        self.rows.len()
    }

    fn entry(&self, ix: usize, cx: &App) -> PickerEntry {
        match &self.rows[ix] {
            Row::Header(title) => PickerEntry::Header(title.clone()),
            Row::Command { index, positions } => {
                let command = &self.commands[*index];
                let item = PickerItem::new(command.title.clone())
                    .positions(positions.clone())
                    .key_hint(command.key_hint(cx));
                // While browsing, the headers name the category; in the
                // recent group and in search results the row has to.
                let under_category_header = self.rows[..ix]
                    .iter()
                    .rev()
                    .find_map(|row| match row {
                        Row::Header(title) => Some(*title == command.category),
                        Row::Command { .. } => None,
                    })
                    .unwrap_or(false);
                if under_category_header {
                    item.into()
                } else {
                    item.detail(command.category.clone()).into()
                }
            }
        }
    }

    fn is_selectable(&self, ix: usize, _: &App) -> bool {
        matches!(self.rows.get(ix), Some(Row::Command { .. }))
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(Row::Command { index, .. }) = self.rows.get(ix) else {
            return false;
        };
        let command = self.commands[*index].clone();
        let workspace = self.workspace.clone();
        // Leave this picker's update first: running the command closes the
        // modal, restores focus and only then dispatches the action.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.run_command(&command, window, cx)
                })
                .ok();
        });
        false
    }

    fn placeholder(&self) -> SharedString {
        "Run a command".into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No commands match".into()
    }
}
