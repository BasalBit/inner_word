//! The command registry: the one place where every crate's commands meet.
//!
//! Each crate publishes its user-facing commands as
//! [`writer_ui::CommandDescriptor`]s. The app gathers them here; the command
//! palette and the list of keyboard shortcuts read from here, and look the
//! shortcut of each command up in GPUI's keymap, so nothing is written twice.
//!
//! ```ignore
//! // at startup, after `writer_app::init(cx)`:
//! writer_app::commands::register(writer_editor_tools::commands(), cx);
//! writer_app::commands::register(writer_comments::commands(), cx);
//! ```
//!
//! A command's action is dispatched to whatever had focus before the palette
//! opened, so the handler can live on the editor, on a panel or on the
//! workspace root.

use gpui::{App, Global};
use writer_ui::CommandDescriptor;

/// See the [module docs](self).
#[derive(Default)]
pub struct CommandRegistry {
    commands: Vec<CommandDescriptor>,
    /// Categories that are registered but not listed right now.
    hidden: Vec<gpui::SharedString>,
}

impl Global for CommandRegistry {}

impl CommandRegistry {
    /// Every registered command, in registration order, hidden ones
    /// included.
    pub fn commands(&self) -> &[CommandDescriptor] {
        &self.commands
    }

    /// Whether the commands of `category` are left out of [`all`].
    pub fn is_hidden(&self, category: &str) -> bool {
        self.hidden.iter().any(|hidden| hidden.as_ref() == category)
    }

    fn add(&mut self, command: CommandDescriptor) {
        let duplicate = self.commands.iter().any(|existing| {
            existing.title == command.title
                && existing.category == command.category
                && existing.action_ref().partial_eq(command.action_ref())
        });
        if !duplicate {
            self.commands.push(command);
        }
    }
}

/// Adds `commands` to the registry. A command that is already registered
/// (same title, category and action) is ignored, so calling this twice is
/// harmless.
pub fn register(commands: impl IntoIterator<Item = CommandDescriptor>, cx: &mut App) {
    let registry = cx.default_global::<CommandRegistry>();
    for command in commands {
        registry.add(command);
    }
}

/// Every command that is listed right now, in registration order: the
/// registered ones without those of a hidden category
/// ([`set_category_hidden`]). The palette, the list of shortcuts and Vim's
/// command line read this.
pub fn all(cx: &App) -> Vec<CommandDescriptor> {
    cx.try_global::<CommandRegistry>()
        .map(|registry| {
            registry
                .commands
                .iter()
                .filter(|command| !registry.is_hidden(&command.category))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Hides the commands of `category` from the palette and the shortcuts
/// overlay, or lists them again. For a feature that is not there on every
/// Mac (the assistant, without its engine): its commands stay registered
/// and are simply not offered.
pub fn set_category_hidden(category: &str, hidden: bool, cx: &mut App) {
    let registry = cx.default_global::<CommandRegistry>();
    let position = registry
        .hidden
        .iter()
        .position(|existing| existing.as_ref() == category);
    match (position, hidden) {
        (None, true) => registry.hidden.push(category.to_string().into()),
        (Some(ix), false) => {
            registry.hidden.remove(ix);
        }
        _ => {}
    }
}

/// Whether the commands of `category` are hidden right now.
pub fn category_hidden(category: &str, cx: &App) -> bool {
    cx.try_global::<CommandRegistry>()
        .is_some_and(|registry| registry.is_hidden(category))
}

/// The category names in the order they were first registered.
pub fn categories(commands: &[CommandDescriptor]) -> Vec<gpui::SharedString> {
    let mut categories: Vec<gpui::SharedString> = Vec::new();
    for command in commands {
        if !categories.contains(&command.category) {
            categories.push(command.category.clone());
        }
    }
    categories
}

/// Words that keep their capital letter in a command title.
const PROPER_NOUNS: [&str; 4] = ["Markdown", "Vim", "Word", "I"];

/// A command title in the case the app uses for its interface text: a
/// capital at the start, and after that only where the word has one of its
/// own (`Code Block` becomes `Code block`, `Paste as Plain Text` becomes
/// `Paste as plain text`). Words in capitals (`DOCX`) and the names in
/// [`PROPER_NOUNS`] are left alone.
///
/// The crates publish their commands in whichever case they chose; the
/// palette and the list of shortcuts show them side by side, so the app
/// brings them to one.
pub fn sentence_case(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    for (ix, word) in title.split(' ').enumerate() {
        if ix > 0 {
            out.push(' ');
        }
        let mut chars = word.chars();
        let keeps_capital = ix == 0
            || PROPER_NOUNS.contains(&word)
            || (word.chars().count() > 1
                && word.chars().all(|ch| !ch.is_lowercase())
                && word.chars().any(char::is_alphabetic));
        match chars.next() {
            Some(first) if !keeps_capital => {
                out.extend(first.to_lowercase());
                out.push_str(chars.as_str());
            }
            _ => out.push_str(word),
        }
    }
    out
}

/// `commands` with their titles in [`sentence_case`].
pub fn in_sentence_case(
    commands: impl IntoIterator<Item = CommandDescriptor>,
) -> impl Iterator<Item = CommandDescriptor> {
    commands.into_iter().map(|mut command| {
        command.title = sentence_case(&command.title).into();
        command
    })
}
