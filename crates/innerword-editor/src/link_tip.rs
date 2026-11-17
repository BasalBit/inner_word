//! The tooltip of a link: its address and how to open it.
//!
//! A link in the text looks like a link but a click on it places the caret,
//! as in every editor. Someone who works with the mouse learns from this
//! tooltip where the link goes and that `cmd`-click follows it (the context
//! menu has "open" as well).
//!
//! The tooltip appears after the pointer has rested on a link for a moment
//! and goes away when the pointer leaves the link, a button is pressed, the
//! wheel moves, or anything is typed or changed. It is drawn through
//! `Window::set_tooltip`, on top of the rest of the window, under the row
//! the link is on. Nothing in it can be clicked; it never takes focus.

use std::time::Duration;

use gpui::{
    AnyTooltip, AnyView, App, AppContext, Context, IntoElement, Pixels, Point, Render,
    SharedString, Task, Window, div, point, prelude::*, px,
};
use innerword_core::{LinkInfo, Position, text};
use innerword_ui::theme::{ActiveTheme, RADIUS_MD, UI_TEXT_SM};

use crate::editor::Editor;

/// How long the pointer rests on a link before the tooltip shows (the
/// delay of GPUI's own tooltips).
pub(crate) const LINK_TIP_DELAY: Duration = Duration::from_millis(500);
/// Longest address shown in full; longer ones lose their middle.
const MAX_ADDRESS_CHARS: usize = 64;
/// Distance between the link's row and the tooltip.
const TIP_GAP: Pixels = px(2.);

/// The link the pointer is resting on, and its tooltip once it shows.
pub(crate) struct LinkTip {
    pub(crate) link: LinkInfo,
    /// The pointer's last position over the link.
    pub(crate) pointer: Point<Pixels>,
    /// The tooltip, once the delay has passed.
    pub(crate) view: Option<AnyView>,
    _delay: Task<()>,
}

impl LinkTip {
    pub(crate) fn is_shown(&self) -> bool {
        self.view.is_some()
    }
}

impl Editor {
    /// The link directly under a window point: its address and the whole
    /// stretch of text that carries it.
    pub(crate) fn link_under_point(&self, point: Point<Pixels>) -> Option<LinkInfo> {
        let under = self.position_under_point(point)?;
        let document = self.state.document();
        let block = document.block(under.block)?;
        let url = block.attrs_at(under.offset)?.link.clone()?;
        // `link_at` looks at the character before a position first: ask
        // at the end of the character under the pointer.
        let after = text::next_grapheme_boundary(block.text(), under.offset);
        document
            .link_at(Position::new(under.block, after))
            .filter(|info| info.url == url)
    }

    /// Follows the pointer: `link` is the link under it (`None` when it is
    /// not over one, or not over the editor at all).
    pub(crate) fn update_link_tip(
        &mut self,
        link: Option<LinkInfo>,
        pointer: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(link) = link else {
            self.hide_link_tip(cx);
            return;
        };
        if let Some(tip) = &mut self.link_tip
            && tip.link == link
        {
            // Still on the same link. A tooltip that is showing stays
            // where it came up.
            if !tip.is_shown() {
                tip.pointer = pointer;
            }
            return;
        }
        self.hide_link_tip(cx);
        let delay = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINK_TIP_DELAY).await;
            this.update(cx, |editor, cx| {
                let Some(tip) = &mut editor.link_tip else {
                    return;
                };
                let url = tip.link.url.clone();
                tip.view = Some(cx.new(|_| LinkTipView::new(&url)).into());
                cx.notify();
            })
            .ok();
        });
        self.link_tip = Some(LinkTip {
            link,
            pointer,
            view: None,
            _delay: delay,
        });
    }

    /// Takes the link tooltip away (and forgets a pending one).
    pub(crate) fn hide_link_tip(&mut self, cx: &mut Context<Self>) {
        if let Some(tip) = self.link_tip.take()
            && tip.is_shown()
        {
            cx.notify();
        }
    }

    /// Prepaint: hands a showing tooltip to the window for this frame, as
    /// long as its link is still under the pointer (the text may have
    /// scrolled or changed since the pointer last moved).
    pub(crate) fn prepaint_link_tip(&mut self, window: &mut Window) {
        let Some(tip) = &self.link_tip else {
            return;
        };
        let Some(view) = tip.view.clone() else {
            return;
        };
        if self.link_under_point(tip.pointer).as_ref() != Some(&tip.link) {
            self.link_tip = None;
            return;
        }
        // Under the row the link is on, at the pointer's x.
        let row_bottom = self
            .position_under_point(tip.pointer)
            .and_then(|position| self.position_bounds(position))
            .map_or(tip.pointer.y, |row| row.bottom());
        window.set_tooltip(AnyTooltip {
            view,
            mouse_position: point(tip.pointer.x, row_bottom + TIP_GAP),
            check_visible_and_update: std::rc::Rc::new(|_, _, _| true),
        });
    }

    /// What the link tooltip says, when one is showing (for tests and
    /// scripted runs).
    #[doc(hidden)]
    pub fn link_tooltip(&self, cx: &App) -> Option<String> {
        let view = self.link_tip.as_ref()?.view.clone()?;
        let view = view.downcast::<LinkTipView>().ok()?;
        let view = view.read(cx);
        Some(format!("{} {}", view.address, LinkTipView::HOW))
    }
}

/// The floating label.
pub(crate) struct LinkTipView {
    address: SharedString,
}

impl LinkTipView {
    /// How a link is opened with the mouse.
    const HOW: &'static str = "\u{2318}-click to open";

    fn new(url: &str) -> Self {
        Self {
            address: shortened(url).into(),
        }
    }
}

/// `url` with its middle left out when it is very long: the start says
/// where it goes, the end which page.
fn shortened(url: &str) -> String {
    let count = url.chars().count();
    if count <= MAX_ADDRESS_CHARS {
        return url.to_string();
    }
    let head: String = url.chars().take(MAX_ADDRESS_CHARS - 20).collect();
    let tail: String = url.chars().skip(count - 19).collect();
    format!("{head}\u{2026}{tail}")
}

impl Render for LinkTipView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = theme.colors;
        div().child(
            div()
                .debug_selector(|| "link-tip".into())
                .flex()
                .items_center()
                .gap(px(8.))
                .h(px(24.))
                .px(px(8.))
                .rounded(RADIUS_MD)
                .bg(colors.elevated)
                .border_1()
                .border_color(colors.border_strong)
                .shadow(theme.shadow_popover())
                .font_family(theme.fonts.ui.clone())
                .text_size(UI_TEXT_SM)
                .line_height(px(16.))
                .text_color(colors.ink)
                .whitespace_nowrap()
                .child(self.address.clone())
                .child(div().text_color(colors.muted).child(Self::HOW)),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_addresses_lose_their_middle() {
        assert_eq!(shortened("https://example.com/a"), "https://example.com/a");
        let long = format!("https://example.com/{}/end-of-the-address", "x".repeat(200));
        let short = shortened(&long);
        assert_eq!(short.chars().count(), MAX_ADDRESS_CHARS);
        assert!(short.starts_with("https://example.com/xxx"));
        assert!(short.ends_with("/end-of-the-address"));
        assert!(short.contains('\u{2026}'));
        // Never cut inside a character.
        let wide = "https://example.com/".to_string() + &"\u{00e9}".repeat(100);
        assert_eq!(shortened(&wide).chars().count(), MAX_ADDRESS_CHARS);
    }
}
