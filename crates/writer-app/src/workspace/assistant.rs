//! The assistant in the workspace: whether it is shown, and what the
//! workspace supplies to its two chats.
//!
//! # Whether it is shown
//!
//! The assistant is shown when three things hold ([`Workspace::assistant_shown`]):
//! the app has an engine for it ([`crate::assistant::host`]; `main`
//! installs Claude Code, a test the fake, and without one there is no
//! assistant at all), the engine's program is installed on this Mac, and
//! the setting "Assistant" is on. Whether it is installed is found out in
//! the background, at launch and whenever the settings are opened; until
//! the first answer is in, the assistant is not shown, and the app never
//! waits for it.
//!
//! While it is not shown, nothing of it is: its keys do nothing, its
//! commands are not in the palette or the shortcuts overlay
//! ([`crate::commands::set_category_hidden`]), the menu bar has none of
//! its items ([`crate::menus::set_assistant_shown`]), the text menu and the
//! format bar have no "Ask the Assistant", and the side chat is not drawn.
//! The settings then say in one line what it needs.
//!
//! Installed but not signed in, the assistant is shown, and its chats say
//! how to sign in.

use gpui::{Context, Entity, Subscription};

use super::Workspace;
use crate::{
    actions::CATEGORY_ASSISTANT,
    assistant::{Host, HostEvent},
};

/// The assistant as one workspace holds it.
pub(crate) struct AssistantState {
    /// The engine; `None` in an app without one.
    pub host: Option<Entity<Host>>,
    /// Whether the assistant is shown right now.
    pub shown: bool,
    _subscriptions: Vec<Subscription>,
}

impl AssistantState {
    /// The state of a workspace that has just been made: the engine the
    /// app has, and nothing shown yet.
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let host = crate::assistant::host(cx);
        let subscriptions = host
            .iter()
            .map(|host| {
                cx.subscribe(
                    host,
                    |this: &mut Workspace, _, event: &HostEvent, cx| match event {
                        HostEvent::Checked => this.apply_assistant(cx),
                    },
                )
            })
            .collect();
        Self {
            host,
            shown: false,
            _subscriptions: subscriptions,
        }
    }
}

impl Workspace {
    /// Whether the assistant is shown: the app has an engine, the engine
    /// is installed on this Mac and the setting is on. See the
    /// [module docs](self).
    pub fn assistant_shown(&self) -> bool {
        self.assistant.shown
    }

    /// The engine of the assistant, when the app has one.
    pub fn assistant_host(&self) -> Option<&Entity<Host>> {
        self.assistant.host.as_ref()
    }

    /// Looks for the assistant's engine in the background. Called once at
    /// launch; the settings ask again every time they are opened.
    pub(crate) fn look_for_assistant(&mut self, cx: &mut Context<Self>) {
        if let Some(host) = self.assistant.host.clone() {
            host.update(cx, |host, cx| host.check(cx));
        }
        self.apply_assistant(cx);
    }

    /// Shows or hides the assistant as things are now: after a check of
    /// the engine, and after a change of the setting.
    pub(crate) fn apply_assistant(&mut self, cx: &mut Context<Self>) {
        let installed = self
            .assistant
            .host
            .as_ref()
            .is_some_and(|host| host.read(cx).is_installed());
        let shown = installed && self.settings.assistant.enabled;
        self.assistant.shown = shown;
        crate::commands::set_category_hidden(CATEGORY_ASSISTANT, !shown, cx);
        crate::menus::set_assistant_shown(shown, cx);
        self.sync_right_panel(cx);
        cx.notify();
    }
}
