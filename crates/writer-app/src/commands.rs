//! The command registry: the one place where every crate's commands meet.
//!
//! Each crate publishes its user-facing commands as
//! [`writer_ui::CommandDescriptor`]s. The app gathers them here; the command
//! palette and the keyboard shortcuts overlay read from here, and look the
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
}

impl Global for CommandRegistry {}

impl CommandRegistry {
    /// Every registered command, in registration order.
    pub fn commands(&self) -> &[CommandDescriptor] {
        &self.commands
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

/// Every registered command, in registration order.
pub fn all(cx: &App) -> Vec<CommandDescriptor> {
    cx.try_global::<CommandRegistry>()
        .map(|registry| registry.commands.clone())
        .unwrap_or_default()
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
