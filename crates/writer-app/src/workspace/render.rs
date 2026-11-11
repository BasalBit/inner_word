//! The workspace layout: titlebar band, sidebar, editor area, right panel,
//! status bar, and the modal and toast layers on top.

use gpui::{
    AnyElement, ClickEvent, Context, CursorStyle, Focusable, MouseButton, MouseMoveEvent,
    MouseUpEvent, SharedString, Window, div, prelude::*, px,
};
use writer_ui::{
    KeyHint,
    theme::{
        ActiveTheme, SPACE_2, SPACE_3, SPACE_4, TITLEBAR_HEIGHT, TRAFFIC_LIGHT_RESERVED_WIDTH,
        UI_TEXT_BASE, UI_TEXT_SM,
    },
};

use super::{KEY_CONTEXT, SaveState, Workspace};
use crate::{
    actions::{FocusLibrary, NewDocument, QuickOpen, ToggleCommandPalette},
    settings::{DEFAULT_SIDEBAR_WIDTH, keys},
};

/// Height of the status bar.
pub const STATUS_BAR_HEIGHT: f32 = 26.;

/// Width of the strip at the sidebar edge that takes the resize drag.
const RESIZE_HANDLE_WIDTH: f32 = 7.;

/// `1234567` as `1,234,567`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `1 word`, `2 words`.
pub fn counted(n: usize, singular: &str, plural: &str) -> String {
    format!(
        "{} {}",
        group_digits(n),
        if n == 1 { singular } else { plural }
    )
}

/// The texts of the status bar, computed from the workspace state. Split
/// from the rendering so tests can read them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusLine {
    /// "Saved", "Edited" or the failure note. Empty without a document.
    pub save: String,
    /// Whether the save note reports a failure.
    pub save_failed: bool,
    /// "Page 2 of 5" in page view.
    pub page: Option<String>,
    /// "1,234 words", or "12 of 1,234 words" with a selection.
    pub words: String,
    /// "6,789 characters", or the selection's.
    pub characters: String,
    /// "5 min read". Empty for an empty document or with a selection.
    pub reading: String,
    /// "212 / 280" when the character-limit indicator is on.
    pub limit: Option<String>,
    /// Whether the text is over the limit.
    pub over_limit: bool,
}

impl Workspace {
    /// The current status bar texts.
    pub fn status_line(&mut self, cx: &mut Context<Self>) -> StatusLine {
        let Some(session) = self.session.as_ref() else {
            return StatusLine::default();
        };
        let (save, save_failed) = match session.save_state() {
            SaveState::Saved => ("Saved".to_string(), false),
            SaveState::Edited => ("Edited".to_string(), false),
            SaveState::Failed(_) => ("Not saved, retrying".to_string(), true),
        };

        let limit_network = self.settings.character_limit;
        let editor = self.editor.read(cx);
        let stats = editor.stats();
        let selection = editor.selection_stats();
        let page = match (editor.caret_page(), editor.page_count()) {
            (Some(page), Some(count)) => Some(format!("Page {page} of {count}")),
            _ => None,
        };
        let (words, characters, reading) = match selection {
            Some(selected) => (
                format!(
                    "{} of {}",
                    group_digits(selected.words),
                    counted(stats.words, "word", "words")
                ),
                format!(
                    "{} selected",
                    counted(selected.characters, "character", "characters")
                ),
                String::new(),
            ),
            None => (
                counted(stats.words, "word", "words"),
                counted(stats.characters, "character", "characters"),
                if stats.reading_minutes == 0 {
                    String::new()
                } else {
                    format!("{} min read", group_digits(stats.reading_minutes))
                },
            ),
        };

        // The limit is counted over the whole text, the way a network would
        // count the post; cached per document revision.
        let revision = editor.state().revision();
        let count = limit_network.map(|network| match self.character_count {
            Some((cached, count)) if cached == revision && count.network == network => count,
            _ => network.count(&editor.document().plain_text()),
        });
        if let Some(count) = count {
            self.character_count = Some((revision, count));
        }

        StatusLine {
            save,
            save_failed,
            page,
            words,
            characters,
            reading,
            limit: count.map(|count| format!("{} {}", count.network.name(), count.label())),
            over_limit: count.is_some_and(|count| !count.fits()),
        }
    }

    fn render_titlebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let sidebar_visible = self.settings.sidebar_visible;
        div()
            .id("titlebar")
            .h(TITLEBAR_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            // Keep the title centred and clear of the traffic lights.
            .px(if sidebar_visible {
                SPACE_4
            } else {
                TRAFFIC_LIGHT_RESERVED_WIDTH
            })
            .on_click(|event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_size(UI_TEXT_BASE)
                    .text_color(colors.muted)
                    .child(self.display_title()),
            )
            .into_any_element()
    }

    fn render_sidebar(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        div()
            .id("sidebar")
            .w(px(self.settings.sidebar_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .border_r_1()
            .border_color(colors.hairline)
            // The traffic lights sit over the top of the sidebar.
            .child(div().h(TITLEBAR_HEIGHT).flex_none())
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.panel.clone()),
            )
            .into_any_element()
    }

    fn render_resize_handle(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let dragging = self.sidebar_dragging;
        div()
            .id("sidebar-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(self.settings.sidebar_width - RESIZE_HANDLE_WIDTH / 2.))
            .w(px(RESIZE_HANDLE_WIDTH))
            .cursor(CursorStyle::ResizeLeftRight)
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(1.))
                    .h_full()
                    .when(dragging, |el| el.bg(colors.accent)),
            )
            .hover(|style| style.bg(colors.hover))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    if event.click_count == 2 {
                        this.set_sidebar_width(DEFAULT_SIDEBAR_WIDTH, cx);
                    } else {
                        this.sidebar_dragging = true;
                        cx.notify();
                    }
                }),
            )
            .into_any_element()
    }

    fn render_empty_state(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let document_font = theme.fonts.document.clone();
        let hint = |label: &'static str, hint: Option<KeyHint>| {
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(SPACE_4)
                .w(px(240.))
                .text_color(colors.muted)
                .child(label)
                .children(hint.map(|hint| hint.boxed()))
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(SPACE_2)
            .child(
                div()
                    .font_family(document_font)
                    .text_size(px(24.))
                    .text_color(colors.ink)
                    .child("No document open"),
            )
            .child(div().h(SPACE_2))
            .child(hint("New document", KeyHint::for_action(&NewDocument, cx)))
            .child(hint("Open a document", KeyHint::for_action(&QuickOpen, cx)))
            .child(hint("Go to the library", KeyHint::for_action(&FocusLibrary, cx)))
            .child(hint(
                "Every command",
                KeyHint::for_action(&ToggleCommandPalette, cx),
            ))
            .into_any_element()
    }

    fn render_status_bar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let status = self.status_line(cx);
        let colors = cx.theme().colors;
        let text = |text: String| div().flex_none().whitespace_nowrap().child(text);

        let left = div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .min_w_0()
            .children(
                self.status_items
                    .iter()
                    .map(|(_, view)| div().flex_none().child(view.clone())),
            )
            .when(!status.save.is_empty(), |el| {
                el.child(
                    text(status.save.clone())
                        .when(status.save_failed, |el| el.text_color(colors.danger)),
                )
            });

        let right = div()
            .flex()
            .items_center()
            .gap(SPACE_4)
            .flex_none()
            .children(status.limit.clone().map(|limit| {
                text(limit).when(status.over_limit, |el| {
                    el.text_color(colors.danger)
                        .font_weight(gpui::FontWeight::MEDIUM)
                })
            }))
            .children(status.page.clone().map(text))
            .when(!status.words.is_empty(), |el| {
                el.child(text(status.words.clone()))
                    .child(text(status.characters.clone()))
            })
            .when(!status.reading.is_empty(), |el| {
                el.child(text(status.reading.clone()))
            });

        div()
            .id("status-bar")
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(SPACE_4)
            .px(SPACE_3)
            .border_t_1()
            .border_color(colors.hairline)
            .text_size(UI_TEXT_SM)
            .text_color(colors.muted)
            .overflow_hidden()
            .child(left)
            .child(right)
            .into_any_element()
    }

    fn render_editor_area(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if self.session.is_none() {
            return self.render_empty_state(cx);
        }
        match &self.editor_host {
            Some(host) => host.clone().into_any_element(),
            None => self.editor.clone().into_any_element(),
        }
    }

    fn end_sidebar_drag(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_dragging {
            self.sidebar_dragging = false;
            let width = self.settings.sidebar_width;
            self.write_setting(keys::SIDEBAR_WIDTH, &width, cx);
            cx.notify();
        }
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.update_window_title(window);
        // Focus must always be somewhere inside the workspace: with nothing
        // focused, key bindings are dispatched at the window root and never
        // reach the handlers below. This happens when the focused element
        // goes away (a popup closes, a panel is removed).
        if window.focused(cx).is_none() {
            let target = match self.session {
                Some(_) => self.editor.focus_handle(cx),
                None => self.focus_handle.clone(),
            };
            window.focus(&target);
        }
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font: SharedString = theme.fonts.ui.clone();
        let sidebar_visible = self.settings.sidebar_visible;

        let titlebar = self.render_titlebar(window, cx);
        let editor_area = self.render_editor_area(cx);
        let status_bar = self.render_status_bar(cx);
        let sidebar = sidebar_visible.then(|| self.render_sidebar(window, cx));
        let resize_handle = sidebar_visible.then(|| self.render_resize_handle(cx));
        let strips: Vec<AnyElement> = self
            .strips
            .iter()
            .map(|(_, view)| {
                div()
                    .flex_none()
                    .w_full()
                    .child(view.clone())
                    .into_any_element()
            })
            .collect();
        let right_panel = self.right_panel.clone().map(|view| {
            div()
                .id("right-panel")
                .w(self.right_panel_width)
                .h_full()
                .flex_none()
                .overflow_hidden()
                .border_l_1()
                .border_color(colors.hairline)
                .bg(colors.sidebar)
                .child(view)
        });

        // The library panel's commands (rename, move to trash, ...) work from
        // anywhere in the window, e.g. after the palette handed focus back
        // to the editor.
        writer_library::forward_actions(div().id("workspace"), &self.panel)
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle(cx))
            .on_action(cx.listener(Self::on_toggle_palette))
            .on_action(cx.listener(Self::on_quick_open))
            .on_action(cx.listener(Self::on_search_library))
            .on_action(cx.listener(Self::on_show_shortcuts))
            .on_action(cx.listener(Self::on_export))
            .on_action(cx.listener(Self::on_open_settings))
            .on_action(cx.listener(Self::on_new_document))
            .on_action(cx.listener(Self::on_new_folder))
            .on_action(cx.listener(Self::on_focus_library))
            .on_action(cx.listener(Self::on_focus_editor))
            .on_action(cx.listener(Self::on_toggle_sidebar))
            .on_action(cx.listener(Self::on_widen_sidebar))
            .on_action(cx.listener(Self::on_narrow_sidebar))
            .on_action(cx.listener(Self::on_toggle_page_view))
            .on_action(cx.listener(Self::on_toggle_vim))
            .on_action(cx.listener(Self::on_toggle_comments))
            .on_action(cx.listener(Self::on_title_from_text))
            .on_action(cx.listener(Self::on_light_theme))
            .on_action(cx.listener(Self::on_dark_theme))
            .on_action(cx.listener(Self::on_system_theme))
            .on_action(cx.listener(Self::on_dismiss_notifications))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_about))
            .on_action(cx.listener(Self::on_minimize))
            .on_action(cx.listener(Self::on_zoom))
            .on_action(cx.listener(Self::on_toggle_full_screen))
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if this.sidebar_dragging {
                    if event.dragging() {
                        this.set_sidebar_width_live(f32::from(event.position.x), cx);
                    } else {
                        this.end_sidebar_drag(cx);
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_sidebar_drag(cx)),
            )
            .relative()
            .size_full()
            .flex()
            .bg(colors.canvas)
            .text_color(colors.ink)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .when(self.sidebar_dragging, |el| {
                el.cursor(CursorStyle::ResizeLeftRight)
            })
            .children(sidebar)
            .child(
                div()
                    .id("main")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(titlebar)
                    .child(
                        div()
                            .flex_1()
                            .min_h_0()
                            .flex()
                            .child(
                                div()
                                    .id("editor-column")
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .id("editor-area")
                                            .flex_1()
                                            .min_h_0()
                                            .w_full()
                                            .overflow_hidden()
                                            .child(editor_area),
                                    )
                                    .children(strips),
                            )
                            .children(right_panel),
                    )
                    .child(status_bar),
            )
            .children(resize_handle)
            .child(self.modal_layer.clone())
            .child(self.toast_layer.clone())
    }
}
