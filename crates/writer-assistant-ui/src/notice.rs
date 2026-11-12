//! [`Notice`]: a row that says something went wrong, or something the
//! user should know, with at most one thing to do about it.
//!
//! ```ignore
//! // An answer that failed:
//! let notice = Notice::for_error(&error, &backend.info());
//! // The agent is installed and nobody is signed in:
//! let notice = Notice::signed_out(&backend.info());
//!
//! notice_row("answer-failed", &notice, false, &colors)
//!     .on_action_click(cx.listener(|this, _, window, cx| this.try_again(window, cx)))
//! ```
//!
//! The words come from the engine (`AssistantError::text`,
//! `BackendInfo::sign_in_help`): they name the agent the user has, and say
//! what to do.

use gpui::{App, ClickEvent, Div, SharedString, Window, div, prelude::*, px};
use writer_assistant::{AssistantError, BackendInfo, Mode, privacy_notice};
use writer_ui::{
    Icon, IconName,
    theme::{RADIUS_MD, SPACE_1, SPACE_2, ThemeColors, UI_TEXT_SM},
};

use crate::controls::{Look, text_button};

/// How loud a [`Notice`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoticeKind {
    /// Something to know.
    Info,
    /// Something that stops the assistant from answering.
    Error,
}

/// See the [module docs](self).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    /// How loud it is.
    pub kind: NoticeKind,
    /// What happened, one sentence.
    pub headline: SharedString,
    /// What to do about it. May be empty.
    pub advice: SharedString,
    /// The label of the one action, when there is one.
    pub action: Option<SharedString>,
}

impl Notice {
    /// A quiet notice.
    pub fn info(headline: impl Into<SharedString>, advice: impl Into<SharedString>) -> Self {
        Self {
            kind: NoticeKind::Info,
            headline: headline.into(),
            advice: advice.into(),
            action: None,
        }
    }

    /// A notice about something that failed.
    pub fn error(headline: impl Into<SharedString>, advice: impl Into<SharedString>) -> Self {
        Self {
            kind: NoticeKind::Error,
            headline: headline.into(),
            advice: advice.into(),
            action: None,
        }
    }

    /// With this action.
    pub fn action(mut self, label: impl Into<SharedString>) -> Self {
        self.action = Some(label.into());
        self
    }

    /// An answer that failed, in the engine's words for the agent of
    /// `info`. The action is "Try Again" (send the question again), except
    /// where trying again cannot help: the agent is not installed.
    pub fn for_error(error: &AssistantError, info: &BackendInfo) -> Self {
        let text = error.text(info);
        let notice = Self::error(text.headline, text.advice);
        match error {
            AssistantError::NotInstalled => notice,
            _ => notice.action("Try Again"),
        }
    }

    /// The agent is installed and nobody is signed in: what to run, and a
    /// button that looks again.
    pub fn signed_out(info: &BackendInfo) -> Self {
        Self::info(
            format!("Sign in with {}", info.name),
            info.sign_in_help.clone(),
        )
        .action("Check Again")
    }

    /// The agent is not on this Mac: how to install it, and a button that
    /// looks again.
    pub fn not_installed(info: &BackendInfo) -> Self {
        Self::info(
            format!("{} is not installed on this Mac", info.name),
            info.install_help.clone(),
        )
        .action("Check Again")
    }

    /// What the user is told once, before the first message: where the
    /// text goes and what it costs.
    pub fn privacy(info: &BackendInfo, mode: Mode) -> Self {
        Self::info("Where your text goes", privacy_notice(info, mode)).action("OK")
    }

    /// Both sentences as one line (what tests and scripted runs read).
    pub fn line(&self) -> String {
        if self.advice.is_empty() {
            self.headline.to_string()
        } else {
            format!("{} {}", self.headline, self.advice)
        }
    }
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// A drawn [`Notice`]. Attach what its action does with
/// [`NoticeRow::on_action_click`].
#[derive(IntoElement)]
pub struct NoticeRow {
    name: SharedString,
    notice: Notice,
    focused: bool,
    colors: ThemeColors,
    on_click: Option<ClickHandler>,
}

/// Draws `notice`. `name` is the debug selector of the row; its button is
/// `<name>-action`. `focused` draws the keyboard's ring around the button.
pub fn notice_row(
    name: impl Into<SharedString>,
    notice: &Notice,
    focused: bool,
    colors: &ThemeColors,
) -> NoticeRow {
    NoticeRow {
        name: name.into(),
        notice: notice.clone(),
        focused,
        colors: *colors,
        on_click: None,
    }
}

impl NoticeRow {
    /// What a click on the action does.
    pub fn on_action_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for NoticeRow {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let colors = self.colors;
        let is_error = self.notice.kind == NoticeKind::Error;
        let name = self.name.clone();
        let button_name = format!("{}-action", self.name);
        let button = self.notice.action.clone().map(|label| {
            let button = text_button(
                SharedString::from(button_name.clone()),
                button_name.clone(),
                label,
                None,
                None,
                Look::Quiet,
                false,
                self.focused,
                &colors,
            );
            match self.on_click {
                Some(handler) => button.on_click(move |event, window, cx| {
                    cx.stop_propagation();
                    handler(event, window, cx);
                }),
                None => button,
            }
        });
        // A plain block with margins between its parts: text that wraps
        // inside a growing flex column is measured at no width.
        let body: Div = div()
            .child(
                div()
                    .text_color(colors.ink)
                    .child(self.notice.headline.clone()),
            )
            .when(!self.notice.advice.is_empty(), |body| {
                body.child(
                    div()
                        .mt(px(2.))
                        .text_color(colors.muted)
                        .child(self.notice.advice.clone()),
                )
            })
            .children(button.map(|button| div().mt(SPACE_2).flex().child(button)));
        div()
            .debug_selector(move || name.to_string())
            .relative()
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(if is_error {
                colors.danger.opacity(0.35)
            } else {
                colors.hairline
            })
            .bg(if is_error {
                colors.danger_subtle
            } else {
                colors.elevated
            })
            .py(SPACE_2)
            .pr(px(10.))
            // Room for the icon, which sits in the left padding.
            .pl(px(30.))
            .text_size(UI_TEXT_SM)
            .line_height(px(17.))
            .child(div().absolute().left(px(9.)).top(px(9.)).child(
                Icon::new(IconName::Alert).size(px(14.)).color(if is_error {
                    colors.danger
                } else {
                    colors.muted
                }),
            ))
            .child(body.pl(SPACE_1).ml(-SPACE_1))
    }
}
