//! Version history: the snapshots of the open document, a preview of each,
//! and the way back.
//!
//! The library takes a snapshot of a document after a save when the newest
//! one is at least ten minutes old (and the text differs), and thins old
//! ones out over time. This modal lists them, newest first, with the time
//! and the word count; beside the list it shows the text of the selected
//! one. `enter` restores it: the text as it is now is saved and kept as a
//! version first, so restoring can itself be taken back from this list.
//!
//! A version that was kept for a reason says so in place of how long ago it
//! was taken: the `writer` tool keeps one before an agent changes a document
//! and labels it ("Before an edit by Claude",
//! `writer_store::Store::create_labeled_snapshot`), so the versions an agent
//! left can be told from the ones kept while writing.
//!
//! Opened from the command palette ("Version history"). Keys, in the
//! `"VersionHistory"` context: `up` / `down` (also `ctrl-p` / `ctrl-n`)
//! choose a version, `home` / `end` the newest and the oldest, `pagedown` /
//! `pageup` (also `space` / `shift-space`) scroll the preview, `enter`
//! restores, `escape` closes.

use gpui::{
    App, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, KeyBinding,
    MouseButton, ScrollHandle, ScrollStrategy, SharedString, UniformListScrollHandle, WeakEntity,
    Window, actions, div, point, prelude::*, px, uniform_list,
};
use writer_core::BlockKind;
use writer_library::{DocumentId, Library, SnapshotId, SnapshotMeta};
use writer_ui::{
    Button, ButtonSize, Icon, IconName,
    theme::{
        ActiveTheme, RADIUS_LG, RADIUS_MD, SPACE_2, SPACE_3, SPACE_4, UI_TEXT_BASE, UI_TEXT_LG,
        UI_TEXT_SM, UI_TEXT_XS,
    },
};

use crate::{
    document_io::load_document,
    workspace::{Workspace, counted},
};

actions!(
    version_history,
    [
        /// Choose the next (older) version.
        SelectNext,
        /// Choose the previous (newer) version.
        SelectPrevious,
        /// Choose the newest version.
        SelectFirst,
        /// Choose the oldest version.
        SelectLast,
        /// Scroll the preview down.
        ScrollPreviewDown,
        /// Scroll the preview up.
        ScrollPreviewUp,
        /// Put the chosen version back as the text of the document.
        Restore,
    ]
);

/// The key context of the version history modal.
pub const KEY_CONTEXT: &str = "VersionHistory";

/// The key bindings of the version history modal.
pub fn key_bindings() -> Vec<KeyBinding> {
    let c = Some(KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-n", SelectNext, c),
        KeyBinding::new("down", SelectNext, c),
        KeyBinding::new("ctrl-p", SelectPrevious, c),
        KeyBinding::new("up", SelectPrevious, c),
        KeyBinding::new("cmd-up", SelectFirst, c),
        KeyBinding::new("home", SelectFirst, c),
        KeyBinding::new("cmd-down", SelectLast, c),
        KeyBinding::new("end", SelectLast, c),
        // The key a hint shows is the one bound last: space.
        KeyBinding::new("pagedown", ScrollPreviewDown, c),
        KeyBinding::new("space", ScrollPreviewDown, c),
        KeyBinding::new("pageup", ScrollPreviewUp, c),
        KeyBinding::new("shift-space", ScrollPreviewUp, c),
        KeyBinding::new("enter", Restore, c),
    ]
}

/// How many blocks of a version the preview shows.
pub const PREVIEW_BLOCKS: usize = 200;

/// Height of one row of the list.
const ROW_HEIGHT: f32 = 46.;

/// Width of the list: room for "Before a section was replaced by Claude".
const LIST_WIDTH: f32 = 290.;

/// A date and time of day in the local time zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LocalTime {
    /// The year.
    pub year: i32,
    /// The month, 1 to 12.
    pub month: u32,
    /// The day of the month, from 1.
    pub day: u32,
    /// The hour, 0 to 23.
    pub hour: u32,
    /// The minute, 0 to 59.
    pub minute: u32,
}

impl LocalTime {
    /// The local time of `unix_ms` (milliseconds since the Unix epoch).
    /// Falls back to UTC when the time zone cannot be read.
    pub fn from_unix_ms(unix_ms: i64) -> LocalTime {
        let seconds = unix_ms.div_euclid(1000);
        local_time(seconds).unwrap_or_else(|| {
            let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
            let of_day = seconds.rem_euclid(86_400);
            LocalTime {
                year,
                month,
                day,
                hour: (of_day / 3600) as u32,
                minute: (of_day % 3600 / 60) as u32,
            }
        })
    }

    fn day_number(&self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }
}

#[cfg(unix)]
fn local_time(seconds: i64) -> Option<LocalTime> {
    let seconds = seconds as libc::time_t;
    // SAFETY: `localtime_r` only reads the time value and writes into the
    // `tm` passed to it; both live on this stack frame.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&seconds, &mut tm).is_null() {
            return None;
        }
        Some(LocalTime {
            year: tm.tm_year + 1900,
            month: (tm.tm_mon + 1) as u32,
            day: tm.tm_mday as u32,
            hour: tm.tm_hour as u32,
            minute: tm.tm_min as u32,
        })
    }
}

#[cfg(not(unix))]
fn local_time(_seconds: i64) -> Option<LocalTime> {
    None
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = i64::from(year) - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// The proleptic Gregorian date of a day count since 1970-01-01.
fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_index + 2) / 5 + 1) as u32;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    } as u32;
    let year = (year_of_era + era * 400 + i64::from(month <= 2)) as i32;
    (year, month, day)
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// When a version was taken, as the list says it: `Today 14:32`,
/// `Yesterday 09:10`, `10 Nov, 08:15`, and with the year when it is not
/// this one: `3 Mar 2025, 17:40`.
pub fn version_time_label(then: LocalTime, now: LocalTime) -> String {
    let time = format!("{:02}:{:02}", then.hour, then.minute);
    let days_ago = now.day_number() - then.day_number();
    let month = MONTHS[(then.month.clamp(1, 12) - 1) as usize];
    match days_ago {
        0 => format!("Today {time}"),
        1 => format!("Yesterday {time}"),
        _ if then.year == now.year => format!("{} {month}, {time}", then.day),
        _ => format!("{} {month} {}, {time}", then.day, then.year),
    }
}

/// One block of a preview.
#[derive(Clone, Debug, PartialEq)]
struct PreviewBlock {
    text: SharedString,
    heading: Option<u8>,
    marker: Option<&'static str>,
    quote: bool,
    code: bool,
}

/// The text of one version, ready to draw.
#[derive(Clone, Debug, PartialEq)]
struct Preview {
    snapshot: SnapshotId,
    blocks: Vec<PreviewBlock>,
    /// Blocks that are not shown.
    more: usize,
    /// Why the version could not be read, when it could not.
    error: Option<String>,
}

/// The version history modal. See the [module docs](self).
pub struct VersionHistory {
    workspace: WeakEntity<Workspace>,
    library: Entity<Library>,
    document: DocumentId,
    title: String,
    versions: Vec<SnapshotMeta>,
    labels: Vec<(SharedString, SharedString)>,
    selected: usize,
    preview: Option<Preview>,
    focus_handle: FocusHandle,
    list_scroll: UniformListScrollHandle,
    preview_scroll: ScrollHandle,
}

impl EventEmitter<DismissEvent> for VersionHistory {}

impl Focusable for VersionHistory {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl VersionHistory {
    /// The versions of `document` (titled `title`) in `library`, newest
    /// first, with the newest selected.
    pub fn new(
        workspace: WeakEntity<Workspace>,
        library: Entity<Library>,
        document: DocumentId,
        title: String,
        cx: &mut Context<Self>,
    ) -> Self {
        let versions = library.read(cx).snapshots(document).unwrap_or_default();
        let now_ms = library.read(cx).clock().now_ms();
        let now = LocalTime::from_unix_ms(now_ms);
        // Why a version was kept, for those that say (the ones the `writer`
        // tool keeps before an agent changes the document).
        let reasons = library
            .read(cx)
            .store()
            .snapshot_labels(document)
            .unwrap_or_default();
        let labels = versions
            .iter()
            .map(|version| {
                let then = LocalTime::from_unix_ms(version.created_at);
                let when = version_time_label(then, now);
                let words = counted(version.word_count as usize, "word", "words");
                // A version kept for a reason says the reason on its second line, whole
                // (the word count moves up beside the time). "2 hours ago" helps for
                // today's other versions; an older one already says its day.
                if let Some(reason) = reasons.get(&version.id) {
                    return (
                        format!("{when} \u{b7} {words}").into(),
                        reason.clone().into(),
                    );
                }
                let detail = if then.day_number() == now.day_number() {
                    format!(
                        "{} \u{b7} {words}",
                        writer_comments::time::relative_time(version.created_at, now_ms)
                    )
                } else {
                    words
                };
                (when.into(), detail.into())
            })
            .collect();
        let mut this = Self {
            workspace,
            library,
            document,
            title,
            versions,
            labels,
            selected: 0,
            preview: None,
            focus_handle: cx.focus_handle(),
            list_scroll: UniformListScrollHandle::new(),
            preview_scroll: ScrollHandle::new(),
        };
        this.load_preview(cx);
        this
    }

    /// The versions listed, newest first.
    pub fn versions(&self) -> &[SnapshotMeta] {
        &self.versions
    }

    /// The selected version.
    pub fn selected(&self) -> Option<&SnapshotMeta> {
        self.versions.get(self.selected)
    }

    /// The two lines of each row: when the version was taken, and how long
    /// ago with its word count. For a version kept for a reason: when with
    /// its word count, and the reason.
    pub fn row_labels(&self) -> Vec<(String, String)> {
        self.labels
            .iter()
            .map(|(when, detail)| (when.to_string(), detail.to_string()))
            .collect()
    }

    /// The text the preview shows for the selected version (its blocks,
    /// one per line).
    pub fn preview_text(&self) -> String {
        self.preview
            .as_ref()
            .map(|preview| {
                preview
                    .blocks
                    .iter()
                    .map(|block| block.text.as_ref())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default()
    }

    /// Selects version `index` (clamped) and shows its text.
    pub fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.versions.is_empty() {
            return;
        }
        let index = index.min(self.versions.len() - 1);
        if index != self.selected {
            self.selected = index;
            self.load_preview(cx);
        }
        self.list_scroll
            .scroll_to_item(index, ScrollStrategy::Center);
        cx.notify();
    }

    fn load_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_scroll.set_offset(point(px(0.), px(0.)));
        let Some(version) = self.versions.get(self.selected) else {
            self.preview = None;
            return;
        };
        let snapshot = version.id;
        if self.preview.as_ref().map(|preview| preview.snapshot) == Some(snapshot) {
            return;
        }
        self.preview = Some(match self.library.read(cx).snapshot(snapshot) {
            Ok(stored) => {
                let loaded = load_document(&stored.content);
                let document = loaded.document;
                let blocks = document
                    .blocks()
                    .iter()
                    .take(PREVIEW_BLOCKS)
                    .map(|block| PreviewBlock {
                        text: SharedString::from(block.text().to_string()),
                        heading: block.kind().heading_level(),
                        marker: match block.kind() {
                            BlockKind::BulletItem => Some("\u{2022}"),
                            BlockKind::NumberedItem => Some("#"),
                            BlockKind::TodoItem { checked: true } => Some("\u{2611}"),
                            BlockKind::TodoItem { checked: false } => Some("\u{2610}"),
                            BlockKind::Divider => Some("\u{2014}\u{2014}\u{2014}"),
                            _ => None,
                        },
                        quote: matches!(block.kind(), BlockKind::Quote),
                        code: block.kind().is_code(),
                    })
                    .collect();
                Preview {
                    snapshot,
                    blocks,
                    more: document.block_count().saturating_sub(PREVIEW_BLOCKS),
                    error: None,
                }
            }
            Err(error) => Preview {
                snapshot,
                blocks: Vec::new(),
                more: 0,
                error: Some(error.to_string()),
            },
        });
    }

    fn scroll_preview(&mut self, pages: f32, cx: &mut Context<Self>) {
        let height = f32::from(self.preview_scroll.bounds().size.height).max(80.);
        let max = f32::from(self.preview_scroll.max_offset().height).max(0.);
        let current = f32::from(self.preview_scroll.offset().y);
        // Offsets are zero or negative.
        let target = (current - pages * height * 0.85).clamp(-max, 0.);
        self.preview_scroll.set_offset(point(px(0.), px(target)));
        cx.notify();
    }

    fn restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(version) = self.versions.get(self.selected) else {
            return;
        };
        let snapshot = version.id;
        let document = self.document;
        let workspace = self.workspace.clone();
        // Leave this view's update first: restoring closes the modal.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    if workspace.open_document_id() == Some(document) {
                        workspace.restore_version(snapshot, window, cx);
                    }
                })
                .ok();
        });
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        let colors = cx.theme().colors;
        range
            .map(|ix| {
                let (when, detail) = self.labels[ix].clone();
                let selected = ix == self.selected;
                div()
                    .id(("version", ix))
                    .debug_selector(move || format!("version-{ix}"))
                    .h(px(ROW_HEIGHT))
                    .w_full()
                    .px(px(4.))
                    .py(px(1.))
                    .cursor_pointer()
                    .when(!selected, |el| el.hover(|style| style.bg(colors.hover)))
                    // A click chooses the version; a double click brings
                    // it back (what enter does).
                    .on_click(
                        cx.listener(move |this, event: &gpui::ClickEvent, window, cx| {
                            this.select(ix, cx);
                            if event.click_count() == 2 {
                                this.restore(window, cx);
                            }
                        }),
                    )
                    .child(
                        div()
                            .size_full()
                            .px(SPACE_2)
                            .flex()
                            .flex_col()
                            .justify_center()
                            .gap(px(1.))
                            .rounded(RADIUS_MD)
                            .when(selected, |el| el.bg(colors.accent_subtle))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(UI_TEXT_BASE)
                                    .line_height(px(18.))
                                    .child(when),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(UI_TEXT_SM)
                                    .line_height(px(16.))
                                    .text_color(colors.muted)
                                    .child(detail),
                            ),
                    )
                    .into_any_element()
            })
            .collect()
    }

    fn render_preview(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let Some(preview) = &self.preview else {
            return div().into_any_element();
        };
        if let Some(error) = &preview.error {
            return div()
                .p(SPACE_4)
                .text_color(colors.muted)
                .child(format!("This version could not be read: {error}"))
                .into_any_element();
        }
        let blocks = preview.blocks.iter().map(|block| {
            let text = if block.text.is_empty() && block.marker.is_none() {
                // An empty paragraph keeps its line.
                SharedString::from(" ")
            } else {
                block.text.clone()
            };
            div()
                .flex()
                .gap(px(6.))
                .when(block.heading.is_some(), |el| {
                    el.font_weight(gpui::FontWeight::SEMIBOLD)
                        .pt(px(6.))
                        .text_color(colors.ink)
                })
                .when(block.heading == Some(1), |el| el.text_size(px(18.)))
                .when(block.heading == Some(2), |el| el.text_size(px(16.)))
                .when(block.quote, |el| {
                    el.pl(SPACE_3)
                        .border_l_2()
                        .border_color(colors.border_strong)
                        .text_color(colors.muted)
                })
                .when(block.code, |el| {
                    el.font_family(theme.fonts.mono.clone()).text_size(px(12.))
                })
                .children(
                    block
                        .marker
                        .map(|marker| div().flex_none().text_color(colors.muted).child(marker)),
                )
                .child(div().flex_1().min_w_0().child(text))
        });
        div()
            .id("version-preview")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.preview_scroll)
            .px(SPACE_4)
            .py(SPACE_3)
            .flex()
            .flex_col()
            .gap(px(8.))
            .font_family(theme.fonts.document.clone())
            .text_size(px(14.))
            .line_height(px(21.))
            .text_color(colors.ink)
            .children(blocks)
            .when(preview.more > 0, |el| {
                el.child(
                    div()
                        .pt(SPACE_2)
                        .font_family(theme.fonts.ui.clone())
                        .text_size(UI_TEXT_SM)
                        .text_color(colors.muted)
                        .child(format!(
                            "and {} further on",
                            counted(preview.more, "paragraph", "paragraphs")
                        )),
                )
            })
            .into_any_element()
    }
}

impl Render for VersionHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        let ui_font = theme.fonts.ui.clone();
        let overlay_shadow = theme.shadow_overlay();
        // Leave room for the modal's top offset and a margin below.
        let body_height =
            (f32::from(window.viewport_size().height) * 0.80 - 150.).clamp(160., 520.);
        let count = self.versions.len();

        // The footer names the keys as the keymap has them.
        let choose_hint = crate::controls::footer_hint(
            &[&SelectPrevious, &SelectNext],
            KEY_CONTEXT,
            "Choose",
            cx,
        );
        let scroll_hint =
            crate::controls::footer_hint(&[&ScrollPreviewDown], KEY_CONTEXT, "Scroll the text", cx);
        let close_hint = crate::controls::close_hint(cx);

        let close = crate::controls::close_control(
            cx.listener(|_, _: &gpui::ClickEvent, _, cx| cx.emit(DismissEvent)),
            cx,
        );
        // What enter does, as a button.
        let restore = (count > 0).then(|| {
            Button::new("restore-version", "Restore This Version")
                .primary()
                .size(ButtonSize::Compact)
                .key_hint(crate::keymap::hint_in(&Restore, KEY_CONTEXT, cx))
                .on_click(
                    cx.listener(|this, _: &gpui::ClickEvent, window, cx| this.restore(window, cx)),
                )
        });

        let body = if count == 0 {
            div()
                .h(px(120.))
                .px(SPACE_4)
                .flex()
                .items_center()
                .justify_center()
                .text_color(colors.muted)
                .child(
                    "No earlier versions yet. One is kept about every ten minutes while you \
                     write.",
                )
                .into_any_element()
        } else {
            div()
                .h(px(body_height))
                .flex()
                .child(
                    div()
                        .w(px(LIST_WIDTH))
                        .flex_none()
                        .h_full()
                        .py(px(4.))
                        .border_r_1()
                        .border_color(colors.hairline)
                        .child(
                            uniform_list(
                                "versions",
                                count,
                                cx.processor(|this, range, _, cx| this.render_rows(range, cx)),
                            )
                            .track_scroll(self.list_scroll.clone())
                            .size_full(),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .bg(colors.canvas)
                        .child(self.render_preview(cx)),
                )
                .into_any_element()
        };

        div()
            .id("version-history")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(
                cx.listener(|this, _: &SelectNext, _, cx| this.select(this.selected + 1, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectPrevious, _, cx| {
                this.select(this.selected.saturating_sub(1), cx)
            }))
            .on_action(cx.listener(|this, _: &SelectFirst, _, cx| this.select(0, cx)))
            .on_action(cx.listener(|this, _: &SelectLast, _, cx| this.select(usize::MAX, cx)))
            .on_action(
                cx.listener(|this, _: &ScrollPreviewDown, _, cx| this.scroll_preview(1., cx)),
            )
            .on_action(cx.listener(|this, _: &ScrollPreviewUp, _, cx| this.scroll_preview(-1., cx)))
            .on_action(cx.listener(|this, _: &Restore, window, cx| this.restore(window, cx)))
            // Clicks on the panel must not close the modal or move the
            // keyboard out of it.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .w(px(860.))
            .max_w_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(colors.elevated)
            .border_1()
            .border_color(colors.hairline)
            .rounded(RADIUS_LG)
            .shadow(overlay_shadow)
            .font_family(ui_font)
            .text_size(UI_TEXT_BASE)
            .line_height(px(18.))
            .text_color(colors.ink)
            .child(
                div()
                    .flex_none()
                    .h(px(48.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_2)
                    .border_b_1()
                    .border_color(colors.hairline)
                    .child(Icon::new(IconName::Page).color(colors.muted))
                    .child(
                        div()
                            .flex_none()
                            .text_size(UI_TEXT_LG)
                            .child("Version history"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(colors.muted)
                            .child(self.title.clone()),
                    )
                    .child(close),
            )
            .child(body)
            .child(
                div()
                    .flex_none()
                    .h(px(40.))
                    .px(SPACE_4)
                    .flex()
                    .items_center()
                    .gap(SPACE_4)
                    .border_t_1()
                    .border_color(colors.hairline)
                    .text_size(UI_TEXT_XS)
                    .text_color(colors.muted)
                    .when(count > 0, |el| {
                        el.children(choose_hint).children(scroll_hint)
                    })
                    .children(close_hint)
                    .child(div().flex_1())
                    .children(restore.map(|button| {
                        div()
                            .debug_selector(|| "version-restore".into())
                            .child(button)
                    })),
            )
    }
}
