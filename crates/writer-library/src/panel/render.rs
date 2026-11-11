//! Rendering of the [`LibraryPanel`]: the filter row, the virtualised tree, the trash row,
//! and the trash view.

use std::ops::Range;

use gpui::{
    AnyElement, ClickEvent, Context, Focusable, FontWeight, KeyContext, Window, anchored, deferred,
    div, point, prelude::*, px, uniform_list,
};
use writer_ui::theme::{
    ActiveTheme, RADIUS_SM, SPACE_1, SPACE_2, SPACE_3, UI_TEXT_BASE, UI_TEXT_SM, UI_TEXT_XS,
};
use writer_ui::{HighlightedLabel, Icon, IconName, KeyHint};

use super::rows::{count_documents, grouped};
use super::{
    DeleteForever, Dismiss, EmptyTrash, FocusFilter, HEADER_HEIGHT, KEY_CONTEXT, LibraryItem,
    LibraryPanel, NewDocument, NewFolder, ROOT_ROW_HEIGHT, ROW_HEIGHT, RestoreSelected, ShowTrash,
    TRASH_ROW_HEIGHT, View,
};

/// The height of the strip under the list: the trash row, or the key hints of the trash
/// view.
const FOOTER_HEIGHT: gpui::Pixels = px(32.);
/// How far each level of the tree is indented.
const INDENT: gpui::Pixels = px(14.);

/// The mark at the left edge of the selected row while the panel has the keyboard.
fn focus_mark(color: gpui::Hsla) -> gpui::Div {
    div()
        .absolute()
        .left_0()
        .top(px(5.))
        .bottom(px(5.))
        .w(px(2.))
        .rounded(px(1.))
        .bg(color)
}

impl Render for LibraryPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_subscription.is_none() {
            self.focus_subscription =
                Some(cx.on_focus(&self.focus_handle, window, |this, _, cx| this.focused(cx)));
        }
        let filter_field = self.ensure_filter_field(window, cx);
        // A text field that is no longer drawn must not keep the keyboard: a rename that
        // ended without a window at hand, or the filter when the trash view took over.
        let rename_focused = self
            .rename_field
            .as_ref()
            .is_some_and(|field| field.focus_handle(cx).is_focused(window));
        let filter_focused = filter_field.focus_handle(cx).is_focused(window);
        if (rename_focused && self.renaming.is_none())
            || (filter_focused && self.view == View::Trash)
        {
            window.focus(&self.focus_handle);
        }
        // The other way round: the keyboard left the rename field (another pane took the
        // focus, a modal opened), so the rename is over and the typed name is kept. Focus
        // events say the same, but only reach an active window.
        if self.renaming.is_some() && !rename_focused {
            cx.defer_in(window, |this, window, cx| {
                let still_away = this
                    .rename_field
                    .as_ref()
                    .is_none_or(|field| !field.focus_handle(cx).is_focused(window));
                if still_away {
                    this.finish_rename(true, cx);
                }
            });
        }
        // The row being renamed holds the keyboard, so it must be drawn. The list only
        // draws the rows in view: keep that row in view (scrolling it out with the wheel
        // stops at the edge), and end the rename if its row is gone from the list
        // altogether (a folder above it was collapsed).
        if let Some(item) = self.renaming {
            match self.rows.iter().position(|row| row.item == item) {
                Some(ix) if self.view == View::Tree => {
                    self.scroll.scroll_to_item(ix, gpui::ScrollStrategy::Top)
                }
                _ => cx.defer_in(window, |this, window, cx| {
                    let hidden = this
                        .renaming
                        .is_some_and(|item| !this.rows.iter().any(|row| row.item == item))
                        || (this.renaming.is_some() && this.view != View::Tree);
                    if hidden {
                        let had_keyboard = this
                            .rename_field
                            .as_ref()
                            .is_some_and(|field| field.focus_handle(cx).is_focused(window));
                        this.finish_rename(true, cx);
                        if had_keyboard {
                            window.focus(&this.focus_handle);
                        }
                    }
                }),
            }
        }

        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font = theme.fonts.ui.clone();
        let panel_focused = self.focus_handle.contains_focused(window, cx);

        let mut context = KeyContext::default();
        context.add(KEY_CONTEXT);
        context.set(
            "mode",
            match self.view {
                View::Tree => "tree",
                View::Trash => "trash",
            },
        );

        let (header, root_row, body, footer) = match self.view {
            View::Tree => (
                self.render_filter_row(filter_field, filter_focused, cx),
                Some(self.render_root_row(panel_focused, cx)),
                self.render_tree(cx),
                self.render_trash_footer(panel_focused, cx),
            ),
            View::Trash => (
                self.render_trash_header(cx),
                None,
                self.render_trash_list(cx),
                self.render_trash_hints(cx),
            ),
        };

        let panel = div()
            .id("library-panel")
            .key_context(context)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_select_next))
            .on_action(cx.listener(Self::on_select_previous))
            .on_action(cx.listener(Self::on_select_first))
            .on_action(cx.listener(Self::on_select_last))
            .on_action(cx.listener(Self::on_select_page_up))
            .on_action(cx.listener(Self::on_select_page_down))
            .on_action(cx.listener(Self::on_expand))
            .on_action(cx.listener(Self::on_collapse))
            .on_action(cx.listener(Self::on_open_selected))
            .on_action(cx.listener(Self::on_dismiss))
            .on_action(cx.listener(Self::on_focus_tree))
            .on_action(cx.listener(Self::on_new_document))
            .on_action(cx.listener(Self::on_new_folder))
            .on_action(cx.listener(Self::on_rename))
            .on_action(cx.listener(Self::on_duplicate))
            .on_action(cx.listener(Self::on_move_to_folder))
            .on_action(cx.listener(Self::on_move_up))
            .on_action(cx.listener(Self::on_move_down))
            .on_action(cx.listener(Self::on_move_to_trash))
            .on_action(cx.listener(Self::on_undo_trash))
            .on_action(cx.listener(Self::on_focus_filter))
            .on_action(cx.listener(Self::on_show_trash))
            .on_action(cx.listener(Self::on_restore_selected))
            .on_action(cx.listener(Self::on_delete_forever))
            .on_action(cx.listener(Self::on_empty_trash))
            .on_action(cx.listener(Self::on_toggle_row_detail))
            .size_full()
            .flex()
            .flex_col()
            .bg(colors.sidebar)
            .text_color(colors.ink)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            // Draws nothing: its place in the frame tells whether the panel is on screen.
            .child(
                div()
                    .id("library-on-screen")
                    .track_focus(&self.probe)
                    .absolute()
                    .size_0(),
            )
            .child(header)
            .children(root_row)
            .child(body)
            .child(footer);

        // The panel's own modal layer is drawn over the whole window, and outside the
        // panel's key context so that its keys do not reach the tree behind it.
        let own_modal = (!self.modal_layer_is_shared && self.modal_layer.read(cx).has_modal())
            .then(|| {
                let viewport = window.viewport_size();
                deferred(
                    anchored().position(point(px(0.), px(0.))).child(
                        div()
                            .relative()
                            .w(viewport.width)
                            .h(viewport.height)
                            .child(self.modal_layer.clone()),
                    ),
                )
                .with_priority(1)
            });

        div().size_full().child(panel).children(own_modal)
    }
}

impl LibraryPanel {
    fn render_filter_row(
        &self,
        field: gpui::Entity<writer_ui::TextField>,
        focused: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let filtering = self.filter.is_some();
        let hint = (!focused && !filtering)
            .then(|| KeyHint::for_action(&FocusFilter, cx))
            .flatten();
        let matches = filtering.then(|| {
            self.rows
                .iter()
                .filter(|row| !row.positions.is_empty())
                .count()
        });
        div()
            .id("library-filter")
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_2)
            .h(HEADER_HEIGHT)
            .px(SPACE_3)
            .border_b_1()
            .border_color(colors.hairline)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.focus_filter(window, cx)))
            .child(
                Icon::new(IconName::Search)
                    .size(px(14.))
                    .color(if focused || filtering {
                        colors.ink
                    } else {
                        colors.muted
                    }),
            )
            .child(div().flex_1().min_w_0().child(field))
            .children(hint)
            .when_some(matches, |el, count| {
                el.child(
                    div()
                        .flex_none()
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .child(count.to_string()),
                )
            })
            .into_any_element()
    }

    /// The "Library" row above the tree: the root, selectable like any row, so that the
    /// keyboard has a way to say "the top level" when every top-level item is a folder.
    fn render_root_row(&self, panel_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let selected = self.on_root_row;
        let tree = self.library.read(cx).tree();
        let documents =
            tree.documents.len() + tree.folders.iter().map(count_documents).sum::<usize>();
        div()
            .id("library-root-row")
            .flex_none()
            .h(ROOT_ROW_HEIGHT)
            .px(SPACE_1)
            .pt(SPACE_1)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.click_root_row(window, cx)),
            )
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .h_full()
                    .pl(SPACE_2)
                    .pr(SPACE_2)
                    .rounded(RADIUS_SM)
                    .map(|el| match (selected, panel_focused) {
                        (true, true) => el.bg(colors.accent_subtle),
                        (true, false) => el.bg(colors.hover),
                        (false, _) => el.hover(|style| style.bg(colors.hover)),
                    })
                    .children((selected && panel_focused).then(|| focus_mark(colors.accent)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(UI_TEXT_SM)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(if selected { colors.ink } else { colors.muted })
                            .child("Library"),
                    )
                    .when(documents > 0, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_XS)
                                .text_color(colors.faint)
                                .child(grouped(documents as u64)),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_tree(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let list = div()
            .id("library-tree")
            .flex_1()
            .min_h_0()
            .py(SPACE_1)
            // A click below the last row selects the library itself (new items then go to
            // the top level).
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if this.renaming.is_none() {
                    this.select_root(cx);
                }
            }));
        if !self.rows.is_empty() {
            return list
                .child(
                    uniform_list(
                        "library-rows",
                        self.rows.len(),
                        cx.processor(|this, range: Range<usize>, window, cx| {
                            this.render_rows(range, window, cx)
                        }),
                    )
                    .track_scroll(self.scroll.clone())
                    .size_full(),
                )
                .into_any_element();
        }
        let message = if self.filter.is_some() {
            div()
                .child(format!("Nothing is named “{}”.", self.filter_text.trim()))
                .into_any_element()
        } else {
            let hint = |label: &'static str, hint: Option<KeyHint>| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(SPACE_2)
                    .child(label)
                    .children(hint.map(|hint| hint.boxed()))
            };
            div()
                .flex()
                .flex_col()
                .gap(SPACE_2)
                .child(div().text_color(colors.ink).child("The library is empty."))
                .child(hint("New document", KeyHint::for_action(&NewDocument, cx)))
                .child(hint("New folder", KeyHint::for_action(&NewFolder, cx)))
                .into_any_element()
        };
        // Lined up with the "Library" label above it.
        list.child(
            div()
                .px(SPACE_3)
                .py(SPACE_2)
                .text_color(colors.muted)
                .child(message),
        )
        .into_any_element()
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors;
        let panel_focused = self.focus_handle.contains_focused(window, cx);
        let now = self.library.read(cx).clock().now_ms();
        let mut elements = Vec::with_capacity(range.len());
        for ix in range {
            let Some(row) = self.rows.get(ix) else {
                break;
            };
            let is_folder = matches!(row.item, LibraryItem::Folder(_));
            let selected = !self.on_trash_row && self.selected == Some(row.item);
            let active =
                matches!(row.item, LibraryItem::Document(id) if self.active_document == Some(id));
            let rename_field = (self.renaming == Some(row.item))
                .then(|| self.rename_field.clone())
                .flatten();
            let detail = self.detail_text(row, now);

            let disclosure = div()
                .id(("library-disclosure", ix))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(16.))
                .when(is_folder && row.has_children, |el| {
                    el.rounded(RADIUS_SM)
                        .hover(|style| style.bg(colors.hover))
                        .child(
                            Icon::new(if row.expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size(px(12.))
                            .color(colors.muted),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            cx.stop_propagation();
                            this.click_disclosure(ix, cx);
                        }))
                });
            let icon = Icon::new(match row.item {
                LibraryItem::Folder(_) if row.expanded => IconName::FolderOpen,
                LibraryItem::Folder(_) => IconName::Folder,
                LibraryItem::Document(_) => IconName::Document,
            })
            .color(if active {
                colors.accent
            } else if selected {
                colors.ink
            } else {
                colors.muted
            });
            // The name sits in an absolutely placed box: GPUI only truncates text with an
            // ellipsis when its first measurement has a definite width, and a flex item is
            // first measured at its minimum content width.
            let label = div()
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .ml(px(2.))
                .child(match rename_field {
                    // Shifted left by the field's padding and border, so the text stays put.
                    Some(field) => div()
                        .absolute()
                        .left(px(-5.))
                        .right_0()
                        .top((ROW_HEIGHT - px(22.)) / 2.)
                        .child(field),
                    None => div()
                        .absolute()
                        .left_0()
                        .right_0()
                        .top((ROW_HEIGHT - px(18.)) / 2.)
                        .child(
                            HighlightedLabel::new(row.label.clone(), row.positions.clone()).color(
                                if active {
                                    colors.accent
                                } else if row.untitled {
                                    colors.muted
                                } else {
                                    colors.ink
                                },
                            ),
                        ),
                });
            // Keyboard focus is visible as a red-pencil mark on the selected row.
            let focus_mark = (selected && panel_focused).then(|| focus_mark(colors.accent));

            elements.push(
                div()
                    .id(("library-row", ix))
                    .h(ROW_HEIGHT)
                    .w_full()
                    .px(SPACE_1)
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.click_row(ix, event.click_count(), window, cx);
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(SPACE_1)
                            .relative()
                            .h_full()
                            .pl(SPACE_1 + INDENT * row.depth as f32)
                            .pr(SPACE_2)
                            .rounded(RADIUS_SM)
                            .map(|el| match (selected, panel_focused) {
                                (true, true) => el.bg(colors.accent_subtle),
                                (true, false) => el.bg(colors.hover),
                                (false, _) => el.hover(|style| style.bg(colors.hover)),
                            })
                            .children(focus_mark)
                            .child(disclosure)
                            .child(icon)
                            .child(label)
                            .when_some(detail, |el, detail| {
                                el.child(
                                    div()
                                        .flex_none()
                                        .text_size(UI_TEXT_XS)
                                        .text_color(if is_folder {
                                            colors.faint
                                        } else {
                                            colors.muted
                                        })
                                        .child(detail),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        elements
    }

    /// The "Trash" row under the tree: the way into the trash view.
    fn render_trash_footer(&self, panel_focused: bool, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let selected = self.on_trash_row;
        let count = self.trash_rows.len();
        div()
            .id("library-trash-row")
            .flex_none()
            .h(FOOTER_HEIGHT)
            .px(SPACE_1)
            .py(px(3.))
            .border_t_1()
            .border_color(colors.hairline)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.show_trash(window, cx)))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .h_full()
                    .px(SPACE_2)
                    .rounded(RADIUS_SM)
                    .map(|el| match (selected, panel_focused) {
                        (true, true) => el.bg(colors.accent_subtle),
                        (true, false) => el.bg(colors.hover),
                        (false, _) => el.hover(|style| style.bg(colors.hover)),
                    })
                    .children((selected && panel_focused).then(|| focus_mark(colors.accent)))
                    .child(Icon::new(IconName::Trash).size(px(14.)).color(if selected {
                        colors.ink
                    } else {
                        colors.muted
                    }))
                    .child(
                        div()
                            .flex_none()
                            .text_color(if selected { colors.ink } else { colors.muted })
                            .child("Trash"),
                    )
                    // How many items are in it, right after the name.
                    .when(count > 0, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_XS)
                                .text_color(colors.faint)
                                .child(count.to_string()),
                        )
                    })
                    .child(div().flex_1())
                    .children(KeyHint::for_action(&ShowTrash, cx)),
            )
            .into_any_element()
    }

    fn render_trash_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(SPACE_2)
            .h(HEADER_HEIGHT)
            .px(SPACE_3)
            .border_b_1()
            .border_color(colors.hairline)
            .child(Icon::new(IconName::Trash).size(px(14.)).color(colors.ink))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_weight(FontWeight::MEDIUM)
                    .child("Trash"),
            )
            .child(
                div()
                    .id("library-trash-back")
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(SPACE_1)
                    .px(SPACE_1)
                    .rounded(RADIUS_SM)
                    .text_size(UI_TEXT_SM)
                    .text_color(colors.muted)
                    .hover(|style| style.bg(colors.hover))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.show_tree(cx)))
                    .child("Back")
                    .children(KeyHint::for_action(&Dismiss, cx)),
            )
            .into_any_element()
    }

    fn render_trash_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let list = div().id("library-trash").flex_1().min_h_0().py(SPACE_1);
        if self.trash_rows.is_empty() {
            return list
                .child(
                    div()
                        .px(SPACE_3)
                        .py(SPACE_2)
                        .text_color(colors.muted)
                        .child("The trash is empty."),
                )
                .into_any_element();
        }
        list.child(
            uniform_list(
                "library-trash-rows",
                self.trash_rows.len(),
                cx.processor(|this, range: Range<usize>, window, cx| {
                    this.render_trash_rows(range, window, cx)
                }),
            )
            .track_scroll(self.trash_scroll.clone())
            .size_full(),
        )
        .into_any_element()
    }

    fn render_trash_rows(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors;
        let panel_focused = self.focus_handle.contains_focused(window, cx);
        let mut elements = Vec::with_capacity(range.len());
        for ix in range {
            let Some(row) = self.trash_rows.get(ix) else {
                break;
            };
            let selected = self.trash_selected == Some(row.item);
            let mut origin = format!("From {}", row.location);
            if let Some(contents) = &row.contents {
                origin.push_str(" · ");
                origin.push_str(contents);
            }
            elements.push(
                div()
                    .id(("library-trash-row", ix))
                    .h(TRASH_ROW_HEIGHT)
                    .w_full()
                    .px(SPACE_1)
                    .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        this.click_trash_row(ix, event.click_count(), window, cx);
                    }))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(SPACE_2)
                            .h_full()
                            .px(SPACE_2)
                            .rounded(RADIUS_SM)
                            .map(|el| match (selected, panel_focused) {
                                (true, true) => el.bg(colors.accent_subtle),
                                (true, false) => el.bg(colors.hover),
                                (false, _) => el.hover(|style| style.bg(colors.hover)),
                            })
                            .children(
                                (selected && panel_focused).then(|| focus_mark(colors.accent)),
                            )
                            .child(
                                Icon::new(match row.item {
                                    LibraryItem::Folder(_) => IconName::Folder,
                                    LibraryItem::Document(_) => IconName::Document,
                                })
                                .color(if selected {
                                    colors.ink
                                } else {
                                    colors.muted
                                }),
                            )
                            .child(
                                // Absolutely placed lines, so both truncate with an
                                // ellipsis (see the tree rows).
                                div()
                                    .relative()
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .right_0()
                                            .top(px(5.))
                                            .truncate()
                                            .child(row.label.clone()),
                                    )
                                    .child(
                                        div()
                                            .absolute()
                                            .left_0()
                                            .right_0()
                                            .top(px(24.))
                                            .truncate()
                                            .text_size(UI_TEXT_XS)
                                            .line_height(px(15.))
                                            .text_color(colors.muted)
                                            .child(origin),
                                    ),
                            ),
                    )
                    .into_any_element(),
            );
        }
        elements
    }

    /// The keys of the trash view, under its list.
    fn render_trash_hints(&self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let hint = |label: &'static str, hint: Option<KeyHint>| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(SPACE_1)
                .child(label)
                .children(hint)
        };
        div()
            .flex()
            .flex_none()
            .flex_wrap()
            .items_center()
            .gap_x(SPACE_3)
            .min_h(FOOTER_HEIGHT)
            .px(SPACE_3)
            .py(SPACE_1)
            .border_t_1()
            .border_color(colors.hairline)
            .text_size(UI_TEXT_SM)
            .text_color(colors.muted)
            .child(hint("Restore", KeyHint::for_action(&RestoreSelected, cx)))
            .child(hint("Delete", KeyHint::for_action(&DeleteForever, cx)))
            .child(hint("Empty", KeyHint::for_action(&EmptyTrash, cx)))
            .into_any_element()
    }
}
