//! "Show Why": what came of adding pages, page by page, with every sentence
//! whole (docs/pages-design.md, section 10, "Adding pages in the app").
//!
//! A refusal names up to five reasons with whole addresses and ends
//! "Nothing was saved."; it never fits the three lines of a toast, and
//! several drops would be more than the four toasts the layer shows. So the
//! summary toast of a gesture says how many pages were added and offers
//! "Show Why", which opens this dialog: every page with its outcome and its
//! sentence, wrapped, the files that were no pages, and a button that copies
//! all of it.
//!
//! It is a dialog of the modal layer and has no key context of its own:
//! `escape` (the layer's Dismiss) and a click outside close it, `tab` and
//! `shift-tab` go from page to page (each is a stop, and the list scrolls to
//! the one that has the keyboard), then to the buttons.

use std::{path::PathBuf, time::Duration};

use gpui::{
    App, ClipboardItem, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    SharedString, Window, div, prelude::*, px,
};
use writer_ui::{
    Toast,
    button::Button,
    focus::tab_navigation,
    theme::{ActiveTheme, RADIUS_LG, SPACE_2, SPACE_3, UI_TEXT_BASE, UI_TEXT_LG, UI_TEXT_SM},
};

use crate::workspace::{PageOutcome, PagesReport};

/// The width of the dialog.
const WIDTH: f32 = 600.;

/// The tallest the list of pages grows before it scrolls.
const LIST_HEIGHT: f32 = 380.;

/// The report of one gesture of adding pages, as a dialog.
pub struct PageReport {
    report: PagesReport,
    focus: FocusHandle,
    /// One stop for each page, and one for the list of files left out.
    stops: Vec<FocusHandle>,
    copy_focus: FocusHandle,
    close_focus: FocusHandle,
    scroll: ScrollHandle,
    /// The stop the list was last scrolled to.
    shown: Option<usize>,
}

impl EventEmitter<DismissEvent> for PageReport {}

impl Focusable for PageReport {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PageReport {
    pub fn new(report: PagesReport, cx: &mut Context<Self>) -> Self {
        let stops = (0..report.pages.len() + usize::from(!report.left_out.is_empty()))
            .map(|_| cx.focus_handle().tab_stop(true))
            .collect();
        Self {
            report,
            focus: cx.focus_handle(),
            stops,
            copy_focus: cx.focus_handle().tab_stop(true),
            close_focus: cx.focus_handle().tab_stop(true),
            scroll: ScrollHandle::new(),
            shown: None,
        }
    }

    /// The report the dialog shows.
    pub fn report(&self) -> &PagesReport {
        &self.report
    }

    /// What "Copy" puts on the clipboard: the whole report as plain text.
    pub fn text(&self) -> String {
        self.report.text()
    }

    /// "Copy": the whole report on the clipboard. A scripted run leaves the
    /// clipboard alone: it is the clipboard of whoever is at the machine.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        let scripted = std::env::var_os(crate::automation::AUTOMATION_ENV)
            .is_some_and(|script| !script.is_empty());
        if !scripted {
            cx.write_to_clipboard(ClipboardItem::new_string(self.text()));
        }
        writer_ui::toast::show_toast(Toast::new("Copied").duration(Duration::from_secs(2)), cx);
    }

    fn close(&mut self, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    /// The list follows the keyboard: the stop that has it is scrolled into
    /// view.
    fn follow_focus(&mut self, window: &Window) {
        let focused = self.stops.iter().position(|stop| stop.is_focused(window));
        if focused.is_some() && focused != self.shown {
            if let Some(ix) = focused {
                self.scroll.scroll_to_item(ix);
            }
            self.shown = focused;
        }
    }
}

/// The line under a page's name: what came of it.
fn outcome_line(outcome: &PageOutcome) -> String {
    match outcome {
        PageOutcome::Added {
            files,
            bytes,
            externals,
            ..
        } => {
            let pieces = match externals {
                0 => String::new(),
                1 => ", 1 piece from the web".to_string(),
                n => format!(", {n} pieces from the web"),
            };
            let files = if *files == 1 {
                "1 file".to_string()
            } else {
                format!("{files} files")
            };
            format!(
                "Added ({files}, {}{pieces})",
                writer_page::megabytes(*bytes)
            )
        }
        PageOutcome::NotAdded { .. } => "Not added".to_string(),
        PageOutcome::Cancelled => "Not added: adding pages was cancelled first".to_string(),
    }
}

impl Render for PageReport {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.follow_focus(window);
        let theme = cx.theme();
        let colors = theme.colors;
        let mut list = div()
            .id("page-report-list")
            .track_scroll(&self.scroll)
            .max_h(px(LIST_HEIGHT))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(SPACE_2);
        for (ix, page) in self.report.pages.iter().enumerate() {
            let mut lines: Vec<SharedString> = Vec::new();
            match &page.outcome {
                PageOutcome::Added { warnings, .. } => {
                    lines.extend(warnings.iter().map(|warning| warning.clone().into()));
                }
                PageOutcome::NotAdded { sentence } => lines.push(sentence.clone().into()),
                PageOutcome::Cancelled => {}
            }
            let focused = self.stops[ix].is_focused(window);
            list = list.child(
                div()
                    .id(("page-report-row", ix))
                    .track_focus(&self.stops[ix])
                    .px(SPACE_3)
                    .py(SPACE_2)
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if focused {
                        colors.accent
                    } else {
                        colors.hairline
                    })
                    .child(
                        div()
                            .text_size(UI_TEXT_BASE)
                            .text_color(colors.ink)
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("\u{201c}{}\u{201d}", page.name)),
                    )
                    .child(
                        div()
                            .text_size(UI_TEXT_SM)
                            .text_color(colors.muted)
                            .child(outcome_line(&page.outcome)),
                    )
                    .children(lines.into_iter().map(|line| {
                        div()
                            .mt(px(4.))
                            .text_size(UI_TEXT_SM)
                            .line_height(px(17.))
                            .text_color(colors.ink)
                            .whitespace_normal()
                            .child(line)
                    })),
            );
        }
        if !self.report.left_out.is_empty() {
            let ix = self.report.pages.len();
            let focused = self.stops[ix].is_focused(window);
            let names: Vec<String> = self
                .report
                .left_out
                .iter()
                .map(|path| file_name(path))
                .collect();
            list = list.child(
                div()
                    .id(("page-report-row", ix))
                    .track_focus(&self.stops[ix])
                    .px(SPACE_3)
                    .py(SPACE_2)
                    .rounded(px(4.))
                    .border_1()
                    .border_color(if focused {
                        colors.accent
                    } else {
                        colors.hairline
                    })
                    .child(
                        div()
                            .text_size(UI_TEXT_BASE)
                            .text_color(colors.ink)
                            .child(PagesReport::left_out_sentence(self.report.left_out.len())),
                    )
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(UI_TEXT_SM)
                            .line_height(px(17.))
                            .text_color(colors.muted)
                            .whitespace_normal()
                            .child(names.join(", ")),
                    ),
            );
        }
        let copy = Button::new("page-report-copy", "Copy")
            .track_focus(&self.copy_focus)
            .on_click(cx.listener(|this, _, _, cx| this.copy(cx)));
        let close = Button::new("page-report-close", "Close")
            .primary()
            .track_focus(&self.close_focus)
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));
        tab_navigation(
            div()
                .id("page-report")
                .debug_selector(|| "page-report".into())
                .track_focus(&self.focus)
                .w(px(WIDTH))
                .max_w_full()
                .p(px(20.))
                .flex()
                .flex_col()
                .gap(SPACE_3)
                .bg(colors.elevated)
                .border_1()
                .border_color(colors.hairline)
                .rounded(RADIUS_LG)
                .shadow(theme.shadow_overlay())
                .font_family(theme.fonts.ui.clone()),
        )
        .child(
            div()
                .text_size(UI_TEXT_LG)
                .line_height(px(20.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(colors.ink)
                .child(self.report.summary()),
        )
        .child(list)
        .child(
            div()
                .flex()
                .justify_end()
                .gap(SPACE_2)
                .child(copy)
                .child(close),
        )
    }
}

/// The name of a file as the report shows it.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// The paths of `report` that were no pages, by name (for tests).
pub fn left_out_names(report: &PagesReport) -> Vec<String> {
    report
        .left_out
        .iter()
        .map(|path: &PathBuf| file_name(path))
        .collect()
}
