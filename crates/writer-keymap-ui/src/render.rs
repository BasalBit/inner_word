//! How the view is drawn: the overlays' look. A rounded panel; the query
//! field on top; under it a line with the count of changed keys, the
//! changed-only switch and the controls for Import, Export and Reset All;
//! the list; while the view waits for a key or asks something, a band with
//! the sentences and the choices; hints at the bottom.
//!
//! Layout notes. Every row of the list has one height
//! ([`ROW_HEIGHT`]): the title is one line that is cut off, never wrapped,
//! so no row is taller than its text. The sentences of the band do wrap;
//! they are plain blocks inside a flex column whose width is the panel's
//! (wrapped text inside a growing child of a flex *row* is measured at no
//! width, see `docs/gpui-notes.md`).

use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Context, FontWeight, Hsla, KeyDownEvent, KeyUpEvent,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, Pixels, ScrollStrategy, SharedString,
    Stateful, Window, canvas, div, prelude::*, px, uniform_list,
};
use writer_ui::{
    Button, ButtonSize, Divider, HighlightedLabel, Icon, IconName, KeyHint, KeyLabel, Tooltip,
    context_menu::OpenContextMenu,
    theme::{
        ActiveTheme, RADIUS_LG, RADIUS_MD, RADIUS_SM, ThemeColors, UI_TEXT_BASE, UI_TEXT_LG,
        UI_TEXT_SM, UI_TEXT_XS,
    },
};

use crate::{
    AddKey, CHOICES_KEY_CONTEXT, CHOICES_STACK, Cancel, ChangeKey, Choose, Close, Export, Import,
    KEY_CONTEXT, LIST_STACK, NextChoice, NextKey, PressAnotherKey, PreviousChoice, PreviousKey,
    RECORDING_KEY_CONTEXT, RemoveKey, ResetAll, RunCommand, SelectFirst, SelectLast, SelectNext,
    SelectPageDown, SelectPageUp, SelectPrevious, ToggleChangedOnly, UseDefault,
    keys::{self, glyphs, hint_in, modifier_glyphs},
    question::{Choice, Preview, PreviewRow, Question, Subject},
    view::{DisplayRow, KeymapView, Mode, default_label, stack_of},
    words::{Line, Tone, count},
};

/// The width of the panel.
pub(crate) const WIDTH: Pixels = px(680.);
/// The height of every row of the list.
pub(crate) const ROW_HEIGHT: Pixels = px(32.);
/// The height of every row of the import preview.
pub(crate) const PREVIEW_ROW_HEIGHT: Pixels = px(28.);

/// What a row shows in the place of a key while one is recorded or judged
/// for it.
struct Pending {
    /// The key it stands in for; `None`: it is one more key.
    replacing: Option<writer_keymap::KeySequence>,
    label: String,
    /// Still waiting for the key (as opposed to showing the one pressed).
    waiting: bool,
}

fn tone_color(tone: Tone, colors: &ThemeColors) -> Hsla {
    match tone {
        Tone::Plain => colors.ink,
        Tone::Good => colors.success,
        Tone::Warning => colors.muted,
        Tone::Stop => colors.danger,
    }
}

impl KeymapView {
    fn pending_for(&self, target: &writer_keymap::Target) -> Option<Pending> {
        match &self.mode {
            Mode::Recording(recording) if recording.target == *target => {
                let held = modifier_glyphs(&recording.held);
                Some(Pending {
                    replacing: recording.replacing.clone(),
                    label: if held.is_empty() {
                        "Press a key".to_string()
                    } else {
                        format!("{held}\u{2026}")
                    },
                    waiting: true,
                })
            }
            Mode::Asking(Question {
                subject:
                    Subject::Key {
                        target: asked,
                        replacing,
                        verdict,
                    },
                ..
            }) if asked == target => Some(Pending {
                replacing: replacing.clone(),
                label: glyphs(&verdict.key),
                waiting: false,
            }),
            _ => None,
        }
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors;
        let browsing = self.browsing(cx);
        range
            .map(|ix| {
                let outer = div()
                    .id(("keymap-row", ix))
                    .debug_selector(move || format!("keymap-row-{ix}"))
                    .h(ROW_HEIGHT)
                    .w_full()
                    .px(px(4.))
                    .flex();
                match self.rows[ix].clone() {
                    DisplayRow::Note(text) => outer
                        .items_center()
                        .child(
                            div()
                                .px(px(8.))
                                .min_w_0()
                                .text_size(UI_TEXT_SM)
                                .text_color(colors.muted)
                                .truncate()
                                .child(text),
                        )
                        .into_any_element(),
                    DisplayRow::Header(text) => outer
                        .items_end()
                        .pb(px(4.))
                        .child(
                            div()
                                .px(px(8.))
                                .text_size(UI_TEXT_XS)
                                .line_height(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child(text),
                        )
                        .into_any_element(),
                    DisplayRow::Command { entry, positions } => {
                        self.render_command(ix, entry, positions, browsing, outer, cx)
                    }
                }
            })
            .collect()
    }

    fn render_command(
        &self,
        ix: usize,
        entry_ix: usize,
        positions: Vec<usize>,
        browsing: bool,
        outer: Stateful<gpui::Div>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let entry = &self.entries[entry_ix];
        let selected = self.selected == Some(ix);
        let changeable = entry.changeable();
        let pending = self.pending_for(&entry.row.target);
        let several = entry.keys.len() > 1;

        let pill = |pending: &Pending| {
            div()
                .flex_none()
                .h(px(20.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(RADIUS_SM)
                .text_size(UI_TEXT_SM)
                .whitespace_nowrap()
                .map(|el| {
                    if pending.waiting {
                        el.bg(colors.accent).text_color(colors.on_accent)
                    } else {
                        el.bg(colors.accent_subtle)
                            .border_1()
                            .border_color(colors.accent)
                            .text_color(colors.ink)
                    }
                })
                .child(pending.label.clone())
        };

        let mut key_area = div().flex().flex_none().items_center().gap(px(4.));
        if entry.keys.is_empty() && pending.is_none() {
            key_area = key_area.child(
                div()
                    .text_size(UI_TEXT_SM)
                    .text_color(colors.faint)
                    .child("No key"),
            );
        }
        for (k, key) in entry.keys.iter().enumerate() {
            if let Some(pending) = &pending
                && pending.replacing.as_ref() == Some(&key.keys)
            {
                key_area = key_area.child(pill(pending));
                continue;
            }
            let marked = selected && several && k == self.selected_key;
            let chip = div()
                .id(("keymap-key", ix * 16 + k))
                .debug_selector(move || format!("keymap-key-{ix}-{k}"))
                .flex_none()
                .p(px(1.))
                .rounded(RADIUS_SM)
                .border_1()
                .border_color(if marked {
                    colors.accent
                } else {
                    gpui::transparent_black()
                })
                .when(changeable, |el| {
                    el.cursor_pointer()
                        .hover(|style| style.bg(colors.hover))
                        .tooltip(Tooltip::text("Change this key"))
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            cx.stop_propagation();
                            this.click_key(ix, k, window, cx);
                        }))
                })
                .child(keys::hint(&key.keys).boxed().color(if changeable {
                    colors.ink
                } else {
                    colors.muted
                }));
            key_area = key_area.child(chip);
        }
        if let Some(pending) = &pending
            && pending.replacing.is_none()
        {
            key_area = key_area.child(pill(pending));
        }

        let controls = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .when(!selected, |el| {
                el.opacity(0.)
                    .group_hover("keymap-row", |style| style.opacity(1.))
            })
            .when(changeable, |el| {
                el.child(
                    div()
                        .debug_selector(move || format!("keymap-add-{ix}"))
                        .child(
                            Button::icon_only(("keymap-add", ix), IconName::Plus)
                                .size(ButtonSize::Compact)
                                .pointer_only()
                                .tooltip(if entry.keys.is_empty() {
                                    "Add a key"
                                } else {
                                    "Add another key"
                                })
                                .key_hint(hint_in(&AddKey, &LIST_STACK, cx))
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    cx.stop_propagation();
                                    this.click_row(ix, window, cx);
                                    this.add_key(window, cx);
                                })),
                        ),
                )
            })
            .child(
                div()
                    .debug_selector(move || format!("keymap-more-{ix}"))
                    .child(
                        Button::icon_only(("keymap-more", ix), IconName::More)
                            .size(ButtonSize::Compact)
                            .pointer_only()
                            .tooltip("More")
                            .key_hint(hint_in(&OpenContextMenu, &LIST_STACK, cx))
                            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                this.click_row(ix, window, cx);
                                this.open_row_menu(Some(event.position()), window, cx);
                            })),
                    ),
            );

        let detail: Option<SharedString> = if let Some(reason) = entry
            .row
            .locked
            .as_ref()
            .map(|locked| locked.reason.clone())
            .filter(|_| selected)
        {
            Some(reason)
        } else if !browsing {
            Some(entry.row.heading.clone())
        } else {
            None
        };

        let bounds = self.selected_bounds.clone();
        outer
            .group("keymap-row")
            .cursor_pointer()
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_row(ix, window, cx);
                if event.click_count() == 2 {
                    this.change_key(window, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.click_row(ix, window, cx);
                    this.open_row_menu(Some(event.position), window, cx);
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(RADIUS_MD)
                    .relative()
                    .when(!selected, |el| el.hover(|style| style.bg(colors.hover)))
                    .when(selected, |el| {
                        el.bg(colors.accent_subtle)
                            .child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top(px(5.))
                                    .bottom(px(5.))
                                    .w(px(2.))
                                    .rounded(px(1.))
                                    .bg(colors.accent),
                            )
                            .child(
                                canvas(move |at, _, _| bounds.set(Some(at)), |_, _, _, _| {})
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .size_full(),
                            )
                    })
                    // The mark of a row the user changed: a dot, so it does
                    // not depend on the words beside the key alone.
                    .child(
                        div()
                            .flex_none()
                            .size(px(6.))
                            .rounded_full()
                            .when(entry.changed, |el| el.bg(colors.accent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .overflow_hidden()
                            .child(
                                div().flex_none().max_w_full().child(HighlightedLabel::new(
                                    entry.row.title.clone(),
                                    positions,
                                )),
                            )
                            .when_some(detail, |el, detail| {
                                el.child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .text_size(UI_TEXT_SM)
                                        .text_color(colors.muted)
                                        .truncate()
                                        .child(detail),
                                )
                            }),
                    )
                    .when(entry.changed, |el| {
                        el.child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_XS)
                                .text_color(colors.muted)
                                .whitespace_nowrap()
                                .child(format!("Default {}", default_label(&entry.default_keys))),
                        )
                    })
                    .child(key_area)
                    .child(controls),
            )
            .into_any_element()
    }

    /// A click on the row at `ix`: leaves what the view was doing for
    /// another row and selects it.
    fn click_row(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::List) {
            self.back_to_list(window, cx);
        }
        self.select_row(ix, ScrollStrategy::Center, cx);
    }

    /// A click on the key `k` of the row at `ix`: change that key.
    fn click_key(&mut self, ix: usize, k: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.click_row(ix, window, cx);
        self.selected_key = k;
        self.change_key(window, cx);
    }

    fn leave_for_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.mode, Mode::List) {
            self.back_to_list(window, cx);
        }
    }

    // ---------------------------------------------------------------------
    // The parts of the panel
    // ---------------------------------------------------------------------

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let listing = matches!(self.mode, Mode::List);
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .h(px(48.))
            .px(px(14.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .relative()
                    .child(Icon::new(IconName::Search).color(colors.muted))
                    .child(div().flex_1().min_w_0().child(self.query_field.clone()))
                    // While the view waits for a key or asks something the
                    // keyboard is not in the field; a click on it goes back
                    // to the list instead of into the field behind.
                    .when(!listing, |el| {
                        el.child(
                            div()
                                .id("keymap-query-cover")
                                .absolute()
                                .inset_0()
                                .occlude()
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.leave_for_list(window, cx)
                                })),
                        )
                    }),
            )
            .child(
                div().debug_selector(|| "keymap-close".into()).child(
                    Button::icon_only("keymap-close", IconName::Close)
                        .pointer_only()
                        .tooltip("Close")
                        .key_hint(hint_in(&Close, &LIST_STACK, cx))
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.close(window, cx)),
                        ),
                ),
            )
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let changed = self.changed_count();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .h(px(36.))
            .px(px(14.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(UI_TEXT_SM)
                    .text_color(colors.muted)
                    .whitespace_nowrap()
                    .child(
                        div()
                            .size(px(6.))
                            .rounded_full()
                            .when(changed > 0, |el| el.bg(colors.accent)),
                    )
                    .child(if changed == 0 {
                        "No key changed".to_string()
                    } else {
                        format!("{} changed", count(changed, "key", "keys"))
                    }),
            )
            .child(
                div().debug_selector(|| "keymap-changed-only".into()).child(
                    Button::new("keymap-changed-only", "Changed Only")
                        .size(ButtonSize::Compact)
                        .pointer_only()
                        .selected(self.changed_only)
                        .key_hint(hint_in(&ToggleChangedOnly, &LIST_STACK, cx))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.leave_for_list(window, cx);
                            this.set_changed_only(!this.changed_only, cx);
                        })),
                ),
            )
            .child(div().flex_1())
            .child(
                div().debug_selector(|| "keymap-import".into()).child(
                    Button::new("keymap-import", "Import\u{2026}")
                        .size(ButtonSize::Compact)
                        .pointer_only()
                        .key_hint(hint_in(&Import, &LIST_STACK, cx))
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.import(window, cx)),
                        ),
                ),
            )
            .child(
                div().debug_selector(|| "keymap-export".into()).child(
                    Button::new("keymap-export", "Export\u{2026}")
                        .size(ButtonSize::Compact)
                        .pointer_only()
                        .key_hint(hint_in(&Export, &LIST_STACK, cx))
                        .on_click(
                            cx.listener(|this, _: &ClickEvent, window, cx| this.export(window, cx)),
                        ),
                ),
            )
            .child(
                div().debug_selector(|| "keymap-reset-all".into()).child(
                    Button::new("keymap-reset-all", "Reset All\u{2026}")
                        .size(ButtonSize::Compact)
                        .pointer_only()
                        .disabled(!self.can_reset_all())
                        .key_hint(hint_in(&ResetAll, &LIST_STACK, cx))
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.reset_all(window, cx);
                        })),
                ),
            )
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let colors = cx.theme().colors;
        let rows = self.rows.len();
        if rows == 0 {
            let text = if self.changed_only && self.browsing(cx) {
                "No key differs from its default"
            } else if self.changed_only {
                "No changed shortcut matches"
            } else {
                "No shortcuts match"
            };
            return div()
                .py(px(20.))
                .px(px(12.))
                .flex()
                .justify_center()
                .text_color(colors.muted)
                .child(text)
                .into_any_element();
        }
        let height = ROW_HEIGHT * rows.min(self.visible_rows) as f32;
        div()
            .py(px(4.))
            .child(
                uniform_list(
                    "keymap-list",
                    rows,
                    cx.processor(|this, range, window, cx| this.render_rows(range, window, cx)),
                )
                .track_scroll(self.scroll.clone())
                .h(height)
                .w_full(),
            )
            .into_any_element()
    }

    /// The sentences of a question, each a block that wraps.
    fn render_lines(lines: &[Line], colors: &ThemeColors) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .children(lines.iter().map(|line| {
                div()
                    .text_size(UI_TEXT_BASE)
                    .line_height(px(18.))
                    .text_color(tone_color(line.tone, colors))
                    .child(line.text.clone())
            }))
    }

    /// The choices of a question as buttons: the marked one filled, with
    /// the key that takes it.
    fn render_choices(&self, question: &Question, cx: &mut Context<Self>) -> impl IntoElement {
        let choose = hint_in(&Choose, &CHOICES_STACK, cx);
        div().flex().flex_wrap().justify_end().gap(px(8.)).children(
            question.choices.iter().enumerate().map(|(ix, button)| {
                let marked = ix == question.selected;
                let choice = button.choice;
                let hint: Option<KeyHint> = if marked {
                    choose.clone()
                } else {
                    match choice {
                        Choice::Cancel => hint_in(&Cancel, &CHOICES_STACK, cx),
                        Choice::PressAnotherKey => hint_in(&PressAnotherKey, &CHOICES_STACK, cx),
                        _ => None,
                    }
                };
                div()
                    .id(("keymap-choice", ix))
                    .debug_selector(move || format!("keymap-choice-{ix}"))
                    // The pointer marks what it is over, so an import
                    // preview shows what that way of importing would do.
                    .on_mouse_move(cx.listener(move |this, _, _, cx| this.mark_choice(ix, cx)))
                    .child(
                        Button::new(("keymap-choice-button", ix), button.label.clone())
                            .pointer_only()
                            .map(|button| if marked { button.primary() } else { button })
                            .key_hint(hint)
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.choose(choice, window, cx);
                            })),
                    )
            }),
        )
    }

    /// The band under the list: waiting for a key, or a question.
    fn render_band(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let colors = cx.theme().colors;
        let band = div()
            .debug_selector(|| "keymap-band".into())
            .flex_none()
            .flex()
            .flex_col()
            .gap(px(8.))
            .px(px(14.))
            .py(px(12.))
            .border_t_1()
            .border_color(colors.hairline)
            .bg(colors.canvas);
        match &self.mode {
            Mode::List => None,
            Mode::Recording(recording) => {
                let names = self.names();
                let title = names.title(&recording.target);
                let place = names.place(&recording.target);
                let what = match &recording.replacing {
                    Some(old) => format!(
                        "Press the new key for \u{201c}{title}\u{201d} ({place}), in the place of {}",
                        glyphs(old)
                    ),
                    None => format!("Press one more key for \u{201c}{title}\u{201d} ({place})"),
                };
                Some(
                    band.track_focus(&self.capture_focus)
                        .key_context(RECORDING_KEY_CONTEXT)
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.ink)
                                .child(what),
                        )
                        .child(
                            div()
                                .text_size(UI_TEXT_SM)
                                .line_height(px(17.))
                                .text_color(colors.muted)
                                .child(
                                    "The next key you press is taken, whatever it is, Esc and \u{21a9} \
                                     too. To leave without a key: Cancel, or any key and then \
                                     Esc. A modifier held alone is not a key yet.",
                                ),
                        )
                        .when_some(recording.problem.clone(), |el, problem| {
                            el.child(div().text_color(colors.danger).child(problem))
                        })
                        .child(
                            div().flex().justify_end().child(
                                div().debug_selector(|| "keymap-record-cancel".into()).child(
                                    Button::new("keymap-record-cancel", "Cancel")
                                        .pointer_only()
                                        .on_click(cx.listener(
                                            |this, _: &ClickEvent, window, cx| {
                                                this.cancel(window, cx)
                                            },
                                        )),
                                ),
                            ),
                        )
                        .into_any_element(),
                )
            }
            Mode::Asking(question) => {
                // The import preview has its own place; its choices stand
                // here all the same.
                let preview = matches!(question.subject, Subject::Import(_));
                Some(
                    band.track_focus(&self.choices_focus)
                        .key_context(CHOICES_KEY_CONTEXT)
                        .when(!preview, |el| {
                            el.child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(colors.ink)
                                    .child(question.title.clone()),
                            )
                            .child(Self::render_lines(&question.lines, &colors))
                        })
                        .child(self.render_choices(question, cx))
                        .into_any_element(),
                )
            }
        }
    }

    // ---------------------------------------------------------------------
    // The import preview
    // ---------------------------------------------------------------------

    fn render_preview_rows(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors;
        let Some(preview) = self.preview_mut() else {
            return Vec::new();
        };
        let selected = preview.selected;
        let rows: Vec<PreviewRow> = preview.rows[range.clone()].to_vec();
        range
            .zip(rows)
            .map(|(ix, row)| {
                let outer = div()
                    .id(("keymap-preview-row", ix))
                    .debug_selector(move || format!("keymap-preview-row-{ix}"))
                    .h(PREVIEW_ROW_HEIGHT)
                    .w_full()
                    .px(px(4.))
                    .flex();
                if let PreviewRow::Header(text) = row {
                    return outer
                        .items_end()
                        .pb(px(4.))
                        .child(
                            div()
                                .px(px(8.))
                                .text_size(UI_TEXT_XS)
                                .line_height(px(14.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(colors.muted)
                                .child(text),
                        )
                        .into_any_element();
                }
                let marked = selected == Some(ix);
                let inner = div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(RADIUS_MD)
                    .when(marked, |el| el.bg(colors.accent_subtle))
                    .when(!marked, |el| el.hover(|style| style.bg(colors.hover)));
                let inner = match row {
                    PreviewRow::Change {
                        title,
                        place,
                        before,
                        after,
                    } => inner
                        .child(div().flex_none().max_w(px(260.)).truncate().child(title))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(UI_TEXT_SM)
                                .text_color(colors.muted)
                                .truncate()
                                .child(place),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_SM)
                                .text_color(colors.muted)
                                .whitespace_nowrap()
                                .child(before),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_SM)
                                .text_color(colors.faint)
                                .child("\u{2192}"),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(UI_TEXT_SM)
                                .text_color(colors.ink)
                                .whitespace_nowrap()
                                .child(after),
                        ),
                    PreviewRow::Collision(text) | PreviewRow::Skipped(text) => inner.child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(UI_TEXT_SM)
                            .truncate()
                            .child(text),
                    ),
                    PreviewRow::Header(_) => inner,
                };
                outer
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if let Some(preview) = this.preview_mut() {
                            preview.selected = Some(ix);
                            cx.notify();
                        }
                    }))
                    .child(inner)
                    .into_any_element()
            })
            .collect()
    }

    fn render_preview(
        &self,
        question: &Question,
        preview: &Preview,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = cx.theme().colors;
        let rows = preview.rows.len();
        let visible = rows.min(self.visible_rows.saturating_sub(1).max(4));
        let detail = preview
            .selected
            .and_then(|selected| preview.rows.get(selected))
            .map(|row| row.text());
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(10.))
                    .h(px(48.))
                    .px(px(14.))
                    .child(Icon::new(IconName::Keyboard).color(colors.muted))
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(360.))
                            .truncate()
                            .text_size(UI_TEXT_LG)
                            .child(question.title.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(UI_TEXT_SM)
                            .text_color(colors.muted)
                            .child(crate::words::import_file_line(&preview.plan)),
                    ),
            )
            .child(Divider::horizontal())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .px(px(14.))
                    .py(px(10.))
                    .child(Self::render_lines(&question.lines, &colors)),
            )
            .when(rows > 0, |el| {
                el.child(Divider::horizontal()).child(
                    div().py(px(4.)).child(
                        uniform_list(
                            "keymap-preview",
                            rows,
                            cx.processor(|this, range, window, cx| {
                                this.render_preview_rows(range, window, cx)
                            }),
                        )
                        .track_scroll(preview.scroll.clone())
                        .h(PREVIEW_ROW_HEIGHT * visible as f32)
                        .w_full(),
                    ),
                )
            })
            // The selected row in full: a row of the list is one line.
            .when_some(detail, |el, detail| {
                el.child(Divider::horizontal()).child(
                    div()
                        .debug_selector(|| "keymap-preview-detail".into())
                        .flex()
                        .flex_col()
                        .px(px(14.))
                        .py(px(8.))
                        .child(
                            div()
                                .text_size(UI_TEXT_SM)
                                .line_height(px(17.))
                                .text_color(colors.muted)
                                .child(detail),
                        ),
                )
            })
            .into_any_element()
    }

    // ---------------------------------------------------------------------
    // The hints at the bottom
    // ---------------------------------------------------------------------

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors;
        let stack = stack_of(&self.mode);
        let label = |text: &'static str, action: &dyn gpui::Action, cx: &App| {
            hint_in(action, stack, cx).map(|hint| KeyLabel::new(text, hint))
        };
        let mut hints: Vec<KeyLabel> = Vec::new();
        let mut note: Option<SharedString> = None;
        match &self.mode {
            Mode::List => {
                if let Some(entry) = self.selected_entry() {
                    if let Some(reason) = entry.locked_reason() {
                        note = Some(format!("Cannot be changed here. {reason}").into());
                    } else {
                        if entry.keys.is_empty() {
                            hints.extend(label("Add a Key", &ChangeKey, cx));
                        } else {
                            hints.extend(label("Change", &ChangeKey, cx));
                            hints.extend(label("Add Another", &AddKey, cx));
                            hints.extend(label("Remove", &RemoveKey, cx));
                        }
                        if entry.keys.len() > 1 {
                            hints.extend(label("Next Key", &NextKey, cx));
                        }
                        if entry.changed {
                            hints.extend(label("Default", &UseDefault, cx));
                        }
                    }
                    // Running the command is in the row's menu, with its key.
                    hints.extend(label("Menu", &OpenContextMenu, cx));
                }
            }
            Mode::Recording(_) => {
                note = Some("Waiting for a key".into());
            }
            Mode::Asking(question) => {
                if question.choices.len() > 1 {
                    // Both keys in one hint: "Choose ← →".
                    let keystrokes: Vec<gpui::Keystroke> = [
                        hint_in(&PreviousChoice, stack, cx),
                        hint_in(&NextChoice, stack, cx),
                    ]
                    .into_iter()
                    .flatten()
                    .flat_map(|hint| hint.keystrokes().to_vec())
                    .collect();
                    if !keystrokes.is_empty() {
                        hints.push(KeyLabel::new("Choose", KeyHint::new(keystrokes)));
                    }
                }
                hints.extend(label("Take", &Choose, cx));
                if matches!(question.subject, Subject::Key { .. }) {
                    hints.extend(label("Another Key", &PressAnotherKey, cx));
                }
                if matches!(question.subject, Subject::Import(_)) {
                    hints.extend(label("Look Through", &SelectNext, cx));
                }
                hints.extend(label("Cancel", &Cancel, cx));
            }
        }
        if matches!(self.mode, Mode::List) {
            hints.extend(label("Close", &Close, cx));
        }
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(14.))
            .h(px(32.))
            .px(px(14.))
            .overflow_hidden()
            .when_some(note, |el, note| {
                el.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .child(note),
                )
            })
            .children(hints)
    }
}

impl Render for KeymapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let shadow = theme.shadow_overlay();
        let font = theme.fonts.ui.clone();

        // As many rows as the window has room for under the modal's top
        // margin, with the band when it is shown.
        let band_room = if matches!(self.mode, Mode::List) {
            0.
        } else {
            150.
        };
        let room =
            f32::from(window.viewport_size().height) * 0.86 - 48. - 36. - 32. - 16. - band_room;
        self.visible_rows = ((room / f32::from(ROW_HEIGHT)).floor() as isize).clamp(4, 14) as usize;

        let preview = match &self.mode {
            Mode::Asking(question) => match &question.subject {
                Subject::Import(preview) => Some(self.render_preview(question, preview, cx)),
                _ => None,
            },
            _ => None,
        };
        let previewing = preview.is_some();

        div()
            .key_context(KEY_CONTEXT)
            .debug_selector(|| "keymap-panel".into())
            .on_action(
                cx.listener(|this, _: &SelectPrevious, _, cx| match this.mode {
                    Mode::List => this.select_previous(cx),
                    _ => this.move_in_preview(-1, cx),
                }),
            )
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| match this.mode {
                Mode::List => this.select_next(cx),
                _ => this.move_in_preview(1, cx),
            }))
            .on_action(
                cx.listener(|this, _: &SelectPageUp, _, cx| match this.mode {
                    Mode::List => this.select_page(false, cx),
                    _ => this.move_in_preview(-8, cx),
                }),
            )
            .on_action(
                cx.listener(|this, _: &SelectPageDown, _, cx| match this.mode {
                    Mode::List => this.select_page(true, cx),
                    _ => this.move_in_preview(8, cx),
                }),
            )
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| match this.mode {
                Mode::List => this.select_first(cx),
                _ => this.move_in_preview(isize::MIN / 2, cx),
            }))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| match this.mode {
                Mode::List => this.select_last(cx),
                _ => this.move_in_preview(isize::MAX / 2, cx),
            }))
            .on_action(cx.listener(|this, _: &ChangeKey, window, cx| {
                this.change_key(window, cx);
            }))
            .on_action(cx.listener(|this, _: &AddKey, window, cx| {
                this.add_key(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RemoveKey, _, cx| {
                this.remove_key(cx);
            }))
            .on_action(cx.listener(|this, _: &UseDefault, _, cx| {
                this.use_default(cx);
            }))
            .on_action(cx.listener(|this, _: &RunCommand, _, cx| {
                this.run_command(cx);
            }))
            .on_action(cx.listener(|this, _: &NextKey, _, cx| this.step_key(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousKey, _, cx| this.step_key(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleChangedOnly, _, cx| {
                this.set_changed_only(!this.changed_only, cx)
            }))
            .on_action(cx.listener(|this, _: &Import, window, cx| this.import(window, cx)))
            .on_action(cx.listener(|this, _: &Export, window, cx| this.export(window, cx)))
            .on_action(cx.listener(|this, _: &ResetAll, window, cx| {
                this.reset_all(window, cx);
            }))
            .on_action(cx.listener(|this, _: &Close, window, cx| this.close(window, cx)))
            .on_action(cx.listener(|this, _: &NextChoice, _, cx| this.step_choice(true, cx)))
            .on_action(cx.listener(|this, _: &PreviousChoice, _, cx| this.step_choice(false, cx)))
            .on_action(cx.listener(|this, _: &Choose, window, cx| this.choose_marked(window, cx)))
            .on_action(cx.listener(|this, _: &PressAnotherKey, window, cx| {
                this.choose(Choice::PressAnotherKey, window, cx);
            }))
            .on_action(cx.listener(|this, _: &Cancel, window, cx| this.cancel(window, cx)))
            // Before the query field, which has a menu of its own on the
            // same action: with a row selected the key opens the row's.
            .capture_action(cx.listener(|this, _: &OpenContextMenu, window, cx| {
                if this.open_row_menu(None, window, cx) {
                    cx.stop_propagation();
                }
            }))
            .on_key_up(
                cx.listener(|this, event: &KeyUpEvent, _, _| this.released(&event.keystroke)),
            )
            .on_modifiers_changed(cx.listener(|this, event: &ModifiersChangedEvent, _, cx| {
                this.modifiers_changed(event.modifiers, cx)
            }))
            // Reached only when no binding took the key: `escape` leaves
            // even after its bindings here were given away.
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    this.cancel(window, cx);
                    cx.stop_propagation();
                }
            }))
            .w(WIDTH)
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(shadow)
            .font_family(font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .map(|el| match preview {
                Some(preview) => el.child(preview),
                None => el
                    .child(self.render_header(cx))
                    .child(Divider::horizontal())
                    .child(self.render_toolbar(cx))
                    .child(Divider::horizontal())
                    .child(self.render_list(cx)),
            })
            .children(self.render_band(cx))
            .child(Divider::horizontal())
            .child(self.render_footer(cx))
            .when(previewing, |el| el)
    }
}
