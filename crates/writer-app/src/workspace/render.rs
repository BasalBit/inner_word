//! The workspace layout: titlebar band, sidebar, editor area, right panel,
//! status bar, and the modal and toast layers on top.

use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, Focusable, MouseButton, MouseMoveEvent,
    MouseUpEvent, SharedString, Window, div, prelude::*, px,
};
use writer_ui::{
    KeyHint,
    theme::{
        ActiveTheme, SPACE_2, SPACE_3, SPACE_4, TITLEBAR_HEIGHT, TRAFFIC_LIGHT_RESERVED_WIDTH,
        UI_TEXT_BASE, UI_TEXT_SM,
    },
};

use writer_editor::actions as editor;

use super::{EditCommand, KEY_CONTEXT, SaveState, Workspace};
use crate::{
    actions::{FocusLibrary, NewDocument, QuickOpen, ToggleCommandPalette},
    character_limit::LimitCounter,
    settings::{DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, keys},
};

/// Height of the status bar.
pub const STATUS_BAR_HEIGHT: f32 = 26.;

/// The side panels give way before the text column gets narrower than this.
pub const MIN_EDITOR_WIDTH: f32 = 360.;

/// The narrowest the panel at the right of the text is drawn.
pub const MIN_RIGHT_PANEL_WIDTH: f32 = 220.;

/// One of the two panels beside the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidePanel {
    /// The library sidebar, at the left.
    Library,
    /// The panel at the right (the comments).
    Right,
}

/// The widths the panels beside the text are drawn at; `None` for a panel
/// that is not drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PanelLayout {
    /// The library sidebar.
    pub sidebar: Option<f32>,
    /// The panel at the right.
    pub right: Option<f32>,
}

/// How the panels beside the text share a window `viewport` wide, so that
/// the text keeps at least [`MIN_EDITOR_WIDTH`].
///
/// `sidebar` and `right` are the widths the panels would like (`None`: the
/// panel is not shown at all). While there is room both get it. In a
/// narrower window both shrink towards their minimum, each by its share of
/// what they have to give. When even the minimums do not fit beside the
/// text, only one panel is drawn: `keep`, the one that was used last.
pub fn panel_layout(
    viewport: f32,
    sidebar: Option<f32>,
    right: Option<f32>,
    keep: SidePanel,
) -> PanelLayout {
    let room = (viewport - MIN_EDITOR_WIDTH).max(0.);
    let alone = |width: f32, min: f32| width.min(room).max(min);
    match (sidebar, right) {
        (None, None) => PanelLayout::default(),
        (Some(sidebar), None) => PanelLayout {
            sidebar: Some(alone(sidebar, MIN_SIDEBAR_WIDTH)),
            right: None,
        },
        (None, Some(right)) => PanelLayout {
            sidebar: None,
            right: Some(alone(right, MIN_RIGHT_PANEL_WIDTH)),
        },
        (Some(sidebar), Some(right)) => {
            let sidebar = sidebar.max(MIN_SIDEBAR_WIDTH);
            let right = right.max(MIN_RIGHT_PANEL_WIDTH);
            let over = sidebar + right - room;
            if over <= 0. {
                PanelLayout {
                    sidebar: Some(sidebar),
                    right: Some(right),
                }
            } else if room >= MIN_SIDEBAR_WIDTH + MIN_RIGHT_PANEL_WIDTH {
                let give_sidebar = sidebar - MIN_SIDEBAR_WIDTH;
                let give_right = right - MIN_RIGHT_PANEL_WIDTH;
                let give = give_sidebar + give_right;
                PanelLayout {
                    sidebar: Some(sidebar - over * give_sidebar / give),
                    right: Some(right - over * give_right / give),
                }
            } else {
                match keep {
                    SidePanel::Library => PanelLayout {
                        sidebar: Some(alone(sidebar, MIN_SIDEBAR_WIDTH)),
                        right: None,
                    },
                    SidePanel::Right => PanelLayout {
                        sidebar: None,
                        right: Some(alone(right, MIN_RIGHT_PANEL_WIDTH)),
                    },
                }
            }
        }
    }
}

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
        // count the post; the result is kept per document revision.
        let revision = editor.state().revision();
        let count = limit_network.map(|network| match self.character_count {
            Some((cached, count)) if cached == revision && count.network == network => count,
            _ => {
                // Block by block: after an edit only the changed blocks are
                // counted again (see `character_limit`).
                let counter = self
                    .limit_counter
                    .get_or_insert_with(|| LimitCounter::new(network));
                if counter.network() != network {
                    *counter = LimitCounter::new(network);
                }
                counter.count(editor.document())
            }
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

    fn render_titlebar(&mut self, sidebar_visible: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
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

    /// How the panels beside the text are drawn in this window: see
    /// [`panel_layout`]. Also notes which of them was used last.
    pub fn side_panels(&mut self, window: &Window, cx: &App) -> PanelLayout {
        if self.comments.read(cx).has_focus(window, cx) {
            self.last_side_panel = SidePanel::Right;
        } else if self.panel_has_focus(window, cx) {
            self.last_side_panel = SidePanel::Library;
        }
        panel_layout(
            f32::from(window.viewport_size().width),
            self.settings
                .sidebar_visible
                .then_some(self.settings.sidebar_width),
            self.right_panel
                .is_some()
                .then_some(f32::from(self.right_panel_width)),
            self.last_side_panel,
        )
    }

    fn render_sidebar(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        div()
            .id("sidebar")
            .debug_selector(|| "sidebar".into())
            .w(px(width))
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

    fn render_resize_handle(&mut self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let dragging = self.sidebar_dragging;
        div()
            .id("sidebar-resize")
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(width - RESIZE_HANDLE_WIDTH / 2.))
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
            .child(hint(
                "Go to the library",
                KeyHint::for_action(&FocusLibrary, cx),
            ))
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

        // The Vim indicator draws nothing while Vim mode is off; leave it
        // out then, or the gap before the next text would be doubled.
        let vim_on = self.vim.read(cx).is_attached();
        let left = div()
            .flex()
            .items_center()
            .gap(SPACE_3)
            .min_w_0()
            .children(
                self.status_items
                    .iter()
                    .filter(|(_, view)| vim_on || view.entity_id() != self.vim_status_id)
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
        self.keep_focus_on_screen(window, cx);
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font: SharedString = theme.fonts.ui.clone();
        let layout = self.side_panels(window, cx);

        let titlebar = self.render_titlebar(layout.sidebar.is_some(), cx);
        let editor_area = self.render_editor_area(cx);
        let status_bar = self.render_status_bar(cx);
        let sidebar = layout.sidebar.map(|width| self.render_sidebar(width, cx));
        let resize_handle = layout
            .sidebar
            .map(|width| self.render_resize_handle(width, cx));
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
        let right_panel = self
            .right_panel
            .clone()
            .zip(layout.right)
            .map(|(view, width)| {
                div()
                    .id("right-panel")
                    .debug_selector(|| "right-panel".into())
                    .w(px(width))
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
            .on_action(cx.listener(Self::on_title_from_text))
            .on_action(cx.listener(Self::on_document_settings))
            .on_action(cx.listener(Self::on_version_history))
            .on_action(cx.listener(Self::on_toggle_orientation))
            .on_action(cx.listener(Self::on_larger_body_text))
            .on_action(cx.listener(Self::on_smaller_body_text))
            .on_action(cx.listener(Self::on_more_line_height))
            .on_action(cx.listener(Self::on_less_line_height))
            .on_action(cx.listener(Self::on_light_theme))
            .on_action(cx.listener(Self::on_dark_theme))
            .on_action(cx.listener(Self::on_system_theme))
            .on_action(cx.listener(Self::on_dismiss_notifications))
            .on_action(cx.listener(Self::on_close_window))
            .on_action(cx.listener(Self::on_about))
            .on_action(cx.listener(Self::on_minimize))
            .on_action(cx.listener(Self::on_zoom))
            .on_action(cx.listener(Self::on_toggle_full_screen))
            // The Edit menu's standard items reach a focused text field as
            // the field's own actions (see `edit_where_the_keyboard_is`).
            .capture_action(cx.listener(|this, _: &editor::Undo, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::Undo, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &editor::Redo, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::Redo, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &editor::Cut, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::Cut, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &editor::Copy, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::Copy, window, cx)
            }))
            .capture_action(cx.listener(|this, _: &editor::Paste, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::Paste, window, cx)
            }))
            .capture_action(
                cx.listener(|this, _: &editor::PasteAsPlainText, window, cx| {
                    this.edit_where_the_keyboard_is(EditCommand::Paste, window, cx)
                }),
            )
            .capture_action(cx.listener(|this, _: &editor::SelectAll, window, cx| {
                this.edit_where_the_keyboard_is(EditCommand::SelectAll, window, cx)
            }))
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
                                            .debug_selector(|| "editor-area".into())
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
