//! Command descriptors: how a crate publishes its user-facing commands to the
//! command palette and the keyboard-shortcuts overlay.
//!
//! A [`CommandDescriptor`] is a title, a category, the GPUI action that runs
//! the command and optional search keywords. It holds no key binding: the
//! palette and the overlay look the shortcut up in the keymap
//! ([`CommandDescriptor::key_hint`]), so the keymap stays the one source of
//! truth for shortcuts.
//!
//! Every crate that has commands exports a `commands()` function:
//!
//! ```ignore
//! use writer_ui::CommandDescriptor;
//!
//! pub fn commands() -> Vec<CommandDescriptor> {
//!     vec![
//!         CommandDescriptor::new("Bold", "Format", ToggleBold).keywords(["strong", "weight"]),
//!         CommandDescriptor::new("Heading 1", "Blocks", SetHeading1).keywords(["title", "h1"]),
//!     ]
//! }
//! ```
//!
//! The palette lists them, matches the query against
//! [`CommandDescriptor::search_text`], shows [`CommandDescriptor::key_hint`]
//! and runs the chosen one with
//! `window.dispatch_action(command.action(), cx)` after restoring focus to
//! where the palette was opened from, so the action reaches the same
//! handlers as its key binding would.

use gpui::{Action, App, SharedString};

use crate::key_hint::KeyHint;

/// One user-facing command. See the [module docs](self).
pub struct CommandDescriptor {
    /// What the command does, as shown in the palette: "Bold", "Heading 1".
    pub title: SharedString,
    /// The group it is listed under: "Format", "Blocks", "View".
    pub category: SharedString,
    /// Extra words the palette matches besides the title and the category.
    pub keywords: Vec<SharedString>,
    action: Box<dyn Action>,
}

impl CommandDescriptor {
    /// A command that runs `action`.
    pub fn new(
        title: impl Into<SharedString>,
        category: impl Into<SharedString>,
        action: impl Action,
    ) -> Self {
        Self::from_boxed(title, category, Box::new(action))
    }

    /// Like [`CommandDescriptor::new`] for an action that is already boxed.
    pub fn from_boxed(
        title: impl Into<SharedString>,
        category: impl Into<SharedString>,
        action: Box<dyn Action>,
    ) -> Self {
        Self {
            title: title.into(),
            category: category.into(),
            keywords: Vec::new(),
            action,
        }
    }

    /// Adds search keywords (synonyms the user might type).
    pub fn keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SharedString>,
    {
        self.keywords.extend(keywords.into_iter().map(Into::into));
        self
    }

    /// A fresh copy of the action, ready for `window.dispatch_action`.
    pub fn action(&self) -> Box<dyn Action> {
        self.action.boxed_clone()
    }

    /// The action, borrowed (for keymap lookups).
    pub fn action_ref(&self) -> &dyn Action {
        self.action.as_ref()
    }

    /// The action's registered name, for example `"editor::ToggleBold"`.
    /// Stable, so it can serve as the command's id.
    pub fn action_name(&self) -> &'static str {
        self.action.name()
    }

    /// `"Category: Title"`, the label of the command in a flat list.
    pub fn label(&self) -> String {
        format!("{}: {}", self.category, self.title)
    }

    /// The text a palette matches a query against: the label followed by the
    /// keywords.
    pub fn search_text(&self) -> String {
        let mut text = self.label();
        for keyword in &self.keywords {
            text.push(' ');
            text.push_str(keyword);
        }
        text
    }

    /// The shortcut bound to the command's action, if any (the last binding
    /// added for it, see [`KeyHint::for_action`]).
    pub fn key_hint(&self, cx: &App) -> Option<KeyHint> {
        KeyHint::for_action(self.action.as_ref(), cx)
    }
}

impl Clone for CommandDescriptor {
    fn clone(&self) -> Self {
        Self {
            title: self.title.clone(),
            category: self.category.clone(),
            keywords: self.keywords.clone(),
            action: self.action.boxed_clone(),
        }
    }
}

impl std::fmt::Debug for CommandDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandDescriptor")
            .field("title", &self.title)
            .field("category", &self.category)
            .field("keywords", &self.keywords)
            .field("action", &self.action.name())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyBinding, TestAppContext, actions};

    actions!(command_tests, [Ping, Pong]);

    #[test]
    fn describes_a_command() {
        let command = CommandDescriptor::new("Bold", "Format", Ping).keywords(["strong", "weight"]);
        assert_eq!(command.title.as_ref(), "Bold");
        assert_eq!(command.category.as_ref(), "Format");
        assert_eq!(command.label(), "Format: Bold");
        assert_eq!(command.search_text(), "Format: Bold strong weight");
        assert_eq!(command.action_name(), "command_tests::Ping");
        assert!(command.action().partial_eq(&Ping));
        assert!(!command.action().partial_eq(&Pong));

        let copy = command.clone();
        assert_eq!(copy.action_name(), command.action_name());
        assert_eq!(copy.keywords, command.keywords);
        assert!(format!("{copy:?}").contains("command_tests::Ping"));
    }

    #[gpui::test]
    fn finds_the_shortcut_in_the_keymap(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.bind_keys([KeyBinding::new("cmd-j", Ping, None)]);
            let bound = CommandDescriptor::new("Ping", "Test", Ping);
            let unbound = CommandDescriptor::from_boxed("Pong", "Test", Box::new(Pong));
            assert_eq!(bound.key_hint(cx).map(|hint| hint.text()), Some("⌘J".into()));
            assert!(unbound.key_hint(cx).is_none());
        });
    }
}
