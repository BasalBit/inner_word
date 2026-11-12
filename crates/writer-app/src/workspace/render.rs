//! The workspace layout: titlebar band, sidebar, editor area, right panel,
//! status bar, and the modal and toast layers on top.

use gpui::{
    AnyElement, App, ClickEvent, Context, CursorStyle, Focusable, MouseButton, MouseMoveEvent,
    MouseUpEvent, SharedString, Window, div, prelude::*, px,
};
use writer_ui::{
    Icon, IconName, KeyHint, Tooltip,
    theme::{
        ActiveTheme, SPACE_2, SPACE_3, SPACE_4, TITLEBAR_HEIGHT, TRAFFIC_LIGHT_RESERVED_WIDTH,
        UI_TEXT_BASE, UI_TEXT_SM,
    },
};

use writer_comments as comments;
use writer_editor::actions as editor;
use writer_spell::actions as spell;

use super::{EditCommand, KEY_CONTEXT, MenuKind, SaveState, Workspace};
use crate::{
    actions::{FocusLibrary, NewDocument, QuickOpen, ToggleCommandPalette},
    character_limit::LimitCounter,
    settings::{DEFAULT_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, keys},
};

/// Height of the status bar. The same as the footer of the library panel
/// (its "Trash" row), so the rules of the two run on one line.
pub const STATUS_BAR_HEIGHT: f32 = 32.;

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

/// How the status bar shows a count against a network's limit: the number
/// in the same form as the other counts, then the limit and the network by
/// name ("1,722 / 280 on X").
pub fn limit_label(count: writer_export::SocialCount) -> String {
    format!(
        "{} / {} on {}",
        group_digits(count.length),
        group_digits(count.limit),
        count.network.name()
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
    /// "6,789 characters", or the selection's. Empty while the
    /// character-limit indicator is on and nothing is selected: the
    /// indicator is the character count then, counted the way the network
    /// counts, and two different numbers side by side would only confuse.
    pub characters: String,
    /// "5 min read". Empty for an empty document or with a selection.
    pub reading: String,
    /// "1,722 / 280 on X" when the character-limit indicator is on.
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

        // What the writer chose to see (the statistics menu of the status
        // bar, or the settings).
        let shown = |on: bool, text: String| if on { text } else { String::new() };
        StatusLine {
            save,
            save_failed,
            page,
            words: shown(self.settings.status_words, words),
            characters: if count.is_some() && selection.is_none() {
                String::new()
            } else {
                shown(self.settings.status_characters, characters)
            },
            reading: shown(self.settings.status_reading_time, reading),
            limit: count.map(limit_label),
            over_limit: count.is_some_and(|count| !count.fits()),
        }
    }

    fn render_titlebar(
        &mut self,
        sidebar_visible: bool,
        tips: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        // Text that has scrolled under the title band is cut at its lower
        // edge: draw that edge then, so the title never seems to touch the
        // text. Page view has the desk's own edge.
        let scrolled = self.session.is_some() && {
            let editor = self.editor.read(cx);
            !editor.page_view() && editor.scroll_offset() > px(0.5)
        };
        div()
            .id("titlebar")
            .debug_selector(|| "titlebar".into())
            .h(TITLEBAR_HEIGHT)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(if scrolled {
                colors.hairline
            } else {
                gpui::transparent_black()
            })
            // Keep the title centred and clear of the traffic lights.
            .px(if sidebar_visible {
                SPACE_4
            } else {
                TRAFFIC_LIGHT_RESERVED_WIDTH
            })
            // A press on the band is not a press on the text: the keyboard
            // stays where it is.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .on_click(|event: &ClickEvent, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(self.render_title(tips, cx))
            .into_any_element()
    }

    /// The title of the open document, with its menu: a right-click on the
    /// title opens it, and so does the small chevron beside it (a
    /// right-click is not something one stumbles on).
    fn render_title(&mut self, tips: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let title = div()
            .min_w_0()
            .truncate()
            .text_size(UI_TEXT_BASE)
            .text_color(colors.muted)
            .child(self.display_title());
        if self.session.is_none() {
            return div().max_w_full().child(title).into_any_element();
        }
        let chevron = div()
            .id("title-chevron")
            .debug_selector(|| "title-chevron".into())
            .flex_none()
            .size(px(18.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(writer_ui::theme::RADIUS_SM)
            .cursor_pointer()
            .hover(|style| style.bg(colors.hover))
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(px(12.))
                    .color(colors.faint),
            )
            .when(tips, |el| {
                el.tooltip(Tooltip::text("Rename, export, versions, settings"))
            })
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                // The keyboard stays in the text; the band under the
                // chevron does not take the press for a window drag.
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.open_menu(MenuKind::Title, None, window, cx);
            }));
        div()
            .id("title")
            .debug_selector(|| MenuKind::Title.selector().into())
            .relative()
            .max_w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(2.))
            // As wide on the left as the chevron on the right, so the
            // title itself stays centred.
            .pl(px(20.))
            .child(title)
            .child(chevron)
            .child(self.anchors.track(MenuKind::Title))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(MenuKind::Title, Some(event.position.into()), window, cx);
                }),
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
            // The handle lies half over the text: a press on it must not
            // also reach the editor underneath (which would move the caret
            // and take the keyboard).
            .occlude()
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
                    cx.stop_propagation();
                    if event.click_count == 2 {
                        this.set_sidebar_width(DEFAULT_SIDEBAR_WIDTH, cx);
                    } else {
                        this.sidebar_dragging = true;
                        cx.notify();
                    }
                }),
            )
            // While the pointer is on the handle the workspace underneath
            // hears nothing of it (the handle takes the pointer): the drag
            // goes on here, and on the workspace root once the pointer is
            // ahead of the handle.
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.drag_sidebar(event, cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.end_sidebar_drag(cx)),
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
            .child(hint("Open document", KeyHint::for_action(&QuickOpen, cx)))
            .child(hint(
                "Go to library",
                KeyHint::for_action(&FocusLibrary, cx),
            ))
            .child(hint(
                "Command palette",
                KeyHint::for_action(&ToggleCommandPalette, cx),
            ))
            .into_any_element()
    }

    /// One item of the status bar that can be clicked: it lights up under
    /// the pointer, says what it is in a tooltip, and opens its menu on a
    /// right-click (at the pointer). What a plain click does is the
    /// caller's: most open the same menu above the item.
    ///
    /// `tooltip` is `None` while a context menu is open: the pointer
    /// rests on the item whose menu has just opened above it, and a
    /// tooltip would come up over the menu.
    fn status_item(
        &self,
        kind: MenuKind,
        id: &'static str,
        tooltip: Option<&'static str>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let colors = cx.theme().colors;
        div()
            .id(id)
            .relative()
            .flex_none()
            .h(px(20.))
            .px(px(6.))
            .flex()
            .items_center()
            .gap(SPACE_3)
            .rounded(writer_ui::theme::RADIUS_SM)
            .whitespace_nowrap()
            .cursor_pointer()
            .hover(|style| style.bg(colors.hover).text_color(colors.ink))
            .when_some(tooltip, |el, tooltip| el.tooltip(Tooltip::text(tooltip)))
            .child(self.anchors.track(kind))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &gpui::MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_menu(kind, Some(event.position.into()), window, cx);
                }),
            )
    }

    fn render_status_bar(&mut self, tips: bool, cx: &mut Context<Self>) -> AnyElement {
        let status = self.status_line(cx);
        let tip = |text: &'static str| tips.then_some(text);
        let colors = cx.theme().colors;
        let weak = cx.weak_entity();
        let opens_menu = move |kind: MenuKind| {
            let weak = weak.clone();
            move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                weak.update(cx, |this, cx| {
                    this.open_menu(kind, None, window, cx);
                })
                .ok();
            }
        };

        // The Vim indicator draws nothing while Vim mode is off; leave it
        // out then, or the gap before the next text would be doubled. With
        // no document open there is no mode to show either.
        let vim_on = self.vim.read(cx).is_attached() && self.session.is_some();
        let save = (!status.save.is_empty()).then(|| {
            self.status_item(
                MenuKind::Save,
                "status-save",
                tip("Saved as you write. Click to save now"),
                cx,
            )
            .debug_selector(|| MenuKind::Save.selector().into())
            .when(status.save_failed, |el| el.text_color(colors.danger))
            .child(status.save.clone())
            .on_click(opens_menu(MenuKind::Save))
        });
        let left = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .children(
                self.status_items
                    .iter()
                    .filter(|(_, view)| vim_on || view.entity_id() != self.vim_status_id)
                    .map(|(_, view)| div().flex_none().child(view.clone())),
            )
            .children(save);

        let limit = status.limit.clone().map(|limit| {
            // Only the number says "too long"; the rest stays quiet.
            let (number, rest) = match limit.split_once(' ') {
                Some((number, rest)) => (number.to_string(), format!(" {rest}")),
                None => (limit.clone(), String::new()),
            };
            self.status_item(
                MenuKind::Limit,
                "status-limit",
                tip("Counted as the network counts. Click to change"),
                cx,
            )
            .debug_selector(|| MenuKind::Limit.selector().into())
            .gap(px(0.))
            .child(div().child(number).when(status.over_limit, |el| {
                el.text_color(colors.danger)
                    .font_weight(gpui::FontWeight::MEDIUM)
            }))
            .child(rest)
            .on_click(opens_menu(MenuKind::Limit))
        });
        let page = status.page.clone().map(|page| {
            self.status_item(
                MenuKind::Page,
                "status-page",
                tip("Page view. Click for one column"),
                cx,
            )
            .debug_selector(|| MenuKind::Page.selector().into())
            .child(page)
            .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                // What the key does.
                window.dispatch_action(Box::new(crate::actions::TogglePageView), cx);
            }))
        });
        let counts: Vec<String> = [&status.words, &status.characters, &status.reading]
            .into_iter()
            .filter(|text| !text.is_empty())
            .cloned()
            .collect();
        let statistics = (!counts.is_empty()).then(|| {
            self.status_item(
                MenuKind::Statistics,
                "status-statistics",
                tip("Click to choose what is counted"),
                cx,
            )
            .debug_selector(|| MenuKind::Statistics.selector().into())
            .gap(SPACE_4)
            .children(counts)
            .on_click(opens_menu(MenuKind::Statistics))
        });
        let right = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .flex_none()
            .children(limit)
            .children(page)
            .children(statistics);

        div()
            .id("status-bar")
            .debug_selector(|| "status-bar".into())
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(SPACE_4)
            .px(px(6.))
            .border_t_1()
            .border_color(colors.hairline)
            .text_size(UI_TEXT_SM)
            .text_color(colors.muted)
            .overflow_hidden()
            // A press on the bar is not a press on the text: the keyboard
            // stays where it is.
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            // A right-click on the bar itself: what it counts (the way
            // back when every count has been switched off).
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.prevent_default();
                    this.open_menu(
                        MenuKind::Statistics,
                        Some(event.position.into()),
                        window,
                        cx,
                    );
                }),
            )
            .child(left)
            .child(right)
            .into_any_element()
    }

    /// The close control of an open picker modal (palette, quick open,
    /// library search, shortcuts, export): a small cross at the right of
    /// its query row, for whoever would rather click than press `escape`.
    ///
    /// The picker draws its own panel and has no place for one, so the
    /// control is laid over it from here, where [`writer_ui::ModalLayer`]
    /// puts a modal: centred, its top at 12% of the window height, with
    /// 16px to the window edges. It lies over the end of the query field
    /// and steps aside when the query is long enough to reach it.
    fn render_picker_close(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        /// The height of a modal picker's query row, and the size of the
        /// control in it.
        const QUERY_ROW: f32 = 48.;
        const CONTROL: f32 = 24.;
        let layer = self.modal_layer.read(cx);
        // The width of the open picker and how long its query is.
        fn query<D: writer_ui::PickerDelegate>(
            layer: &writer_ui::ModalLayer,
            cx: &App,
        ) -> Option<usize> {
            let picker = layer.active::<writer_ui::Picker<D>>()?;
            Some(picker.read(cx).query(cx).chars().count())
        }
        let wide = query::<crate::library_search::LibrarySearchDelegate>(layer, cx)
            .map(|typed| (super::WIDE_MODAL_WIDTH, typed));
        let standard = || {
            query::<crate::palette::PaletteDelegate>(layer, cx)
                .or_else(|| query::<crate::quick_open::QuickOpenDelegate>(layer, cx))
                .or_else(|| query::<crate::shortcuts::ShortcutsDelegate>(layer, cx))
                .or_else(|| query::<crate::export::ExportDelegate>(layer, cx))
                .map(|typed| (super::MODAL_WIDTH, typed))
        };
        let (width, typed) = wide.or_else(standard)?;
        let viewport = window.viewport_size();
        let window_width = f32::from(viewport.width);
        let panel_width = width.min(window_width - 32.);
        // The control lies over the end of the query field. A query long
        // enough to reach it (about nine pixels a character) has the
        // field to itself: the control steps aside.
        if typed as f32 * 9. > panel_width - 110. {
            return None;
        }
        let panel_right = (window_width + panel_width) / 2.;
        let panel_top = f32::from(viewport.height) * 0.12;
        let close = crate::controls::close_control(
            cx.listener(|this, _: &ClickEvent, window, cx| this.dismiss_modal(window, cx)),
            cx,
        );
        Some(
            div()
                .absolute()
                .top(px(panel_top + (QUERY_ROW - CONTROL) / 2.))
                .left(px(panel_right - 12. - CONTROL))
                .child(close)
                .into_any_element(),
        )
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

    /// The pointer moved while the sidebar edge may be held: the edge
    /// follows it, or the drag ends when the button is no longer down.
    fn drag_sidebar(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.sidebar_dragging {
            if event.dragging() {
                self.set_sidebar_width_live(f32::from(event.position.x), cx);
            } else {
                self.end_sidebar_drag(cx);
            }
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
        // Where the title and the status bar items are is noted again as
        // they are drawn.
        self.anchors.clear();
        // Toasts are centred on the text column and sit above the status
        // bar.
        let insets = gpui::Edges {
            top: px(0.),
            left: px(layout.sidebar.unwrap_or(0.)),
            right: px(if self.right_panel.is_some() {
                layout.right.unwrap_or(0.)
            } else {
                0.
            }),
            bottom: px(STATUS_BAR_HEIGHT + 14.),
        };
        self.toast_layer
            .update(cx, |layer, cx| layer.set_insets(insets, cx));

        // No tooltips while a context menu is open (they would come up
        // over it).
        let tips = !writer_ui::ContextMenu::is_open(window, cx);
        let titlebar = self.render_titlebar(layout.sidebar.is_some(), tips, cx);
        let editor_area = self.render_editor_area(cx);
        let status_bar = self.render_status_bar(tips, cx);
        let picker_close = self.render_picker_close(window, cx);
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
        let mut root = writer_library::forward_actions(div().id("workspace"), &self.panel);
        // And the commands for the text work while the keyboard is in the
        // library, the comment list or a modal (the palette and the menu
        // bar offer them everywhere). Only with a document open, so that
        // the menu bar greys them out without one.
        if self.session.is_some() {
            for action in &self.text_commands {
                root = root.on_boxed_action(
                    action.as_ref(),
                    cx.listener(|this, action: &dyn gpui::Action, window, cx| {
                        this.forward_to_text(action, window, cx)
                    }),
                );
            }
        }
        // The spelling commands: the switch always, the ones that act on a
        // word while there is a text and a checker for it.
        let spell_action =
            |this: &mut Self,
             action: &dyn gpui::Action,
             window: &mut Window,
             cx: &mut Context<Self>| { this.on_spell_action(action, window, cx) };
        root = root.on_boxed_action(&spell::ToggleSpellChecking, cx.listener(spell_action));
        if self.session.is_some() && self.spell.is_some() {
            let on_words: [&dyn gpui::Action; 6] = [
                &spell::NextMisspelling,
                &spell::PreviousMisspelling,
                &spell::ShowSuggestions,
                &spell::LearnSpelling,
                &spell::UnlearnSpelling,
                &spell::IgnoreSpelling,
            ];
            for action in on_words {
                root = root.on_boxed_action(action, cx.listener(spell_action));
            }
        }
        root.key_context(KEY_CONTEXT)
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
            .on_action(cx.listener(Self::on_save_now))
            .on_action(cx.listener(Self::on_rename_document))
            .on_action(cx.listener(Self::on_reveal_in_library))
            .on_action(cx.listener(Self::on_open_welcome))
            .on_action(cx.listener(Self::on_choose_spelling_language))
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
            // The comment keys work anywhere in the window (the comments
            // crate handles them globally). Behind an open modal they
            // would move the keyboard or the caret out of sight: the
            // modal closes first.
            .capture_action(cx.listener(|this, _: &comments::AddComment, window, cx| {
                this.close_modal_first(window, cx);
            }))
            .capture_action(
                cx.listener(|this, _: &comments::FocusComments, window, cx| {
                    this.close_modal_first(window, cx);
                }),
            )
            .capture_action(cx.listener(|this, _: &comments::NextComment, window, cx| {
                this.close_modal_first(window, cx);
            }))
            .capture_action(
                cx.listener(|this, _: &comments::PreviousComment, window, cx| {
                    this.close_modal_first(window, cx);
                }),
            )
            .capture_action(
                cx.listener(|this, _: &comments::ToggleComments, window, cx| {
                    this.close_modal_first(window, cx);
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                this.drag_sidebar(event, cx);
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
            .children(picker_close)
            .child(self.toast_layer.clone())
    }
}
