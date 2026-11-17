//! Scroll commands and what they ask of the host.

use crate::harness::*;
use innerword_core::Position;
use innerword_vim::{Mode, Scroll, ScrollAlign, VimOptions};

/// Twenty one-row blocks `l0` … `l19`, five rows visible, caret on `line`.
fn tall(line: usize) -> Session {
    let buffer: Vec<String> = (0..20)
        .map(|i| {
            if i == line {
                format!("|l{i}")
            } else {
                format!("l{i}")
            }
        })
        .collect();
    let mut s = Session::with(&buffer.join("\n"), 1000, 5, VimOptions::default());
    s.clear_effects();
    s
}

fn line(s: &Session) -> usize {
    s.vim.caret(&s.ed).block
}

#[test]
fn half_pages() {
    let mut s = tall(0);
    s.keys("<C-d>");
    assert_eq!(s.host.scrolls, vec![Scroll::Rows(2)]);
    assert_eq!((line(&s), s.host.top), (2, 2));
    s.keys("<C-u>");
    assert_eq!(s.host.scrolls.last(), Some(&Scroll::Rows(-2)));
    assert_eq!((line(&s), s.host.top), (0, 0));
    // A count is the number of rows.
    s.keys("7<C-d>");
    assert_eq!(s.host.scrolls.last(), Some(&Scroll::Rows(7)));
    assert_eq!(line(&s), 7);
    s.keys("3<C-u>");
    assert_eq!(line(&s), 4);
    // At the end the caret stops on the last row.
    s.keys("50<C-d>");
    assert_eq!(line(&s), 19);
    s.clear_effects();
    s.keys("<C-d>");
    assert_eq!(line(&s), 19);
    assert_eq!(s.bells(), 1);
}

#[test]
fn whole_pages() {
    let mut s = tall(0);
    s.keys("<C-f>");
    assert_eq!(s.host.scrolls, vec![Scroll::Rows(3)]);
    assert_eq!(line(&s), 3);
    s.keys("2<C-f>");
    assert_eq!(s.host.scrolls.last(), Some(&Scroll::Rows(6)));
    assert_eq!(line(&s), 9);
    s.keys("<C-b>");
    assert_eq!(s.host.scrolls.last(), Some(&Scroll::Rows(-3)));
    assert_eq!(line(&s), 6);
    s.keys("<PageDown>");
    assert_eq!(line(&s), 9);
    s.keys("<PageUp>");
    assert_eq!(line(&s), 6);
}

#[test]
fn single_rows_keep_the_caret_on_screen() {
    // The caret is on the top row: scrolling down pushes it along.
    let mut s = tall(0);
    s.keys("<C-e>");
    assert_eq!(s.host.scrolls, vec![Scroll::Rows(1)]);
    assert_eq!((line(&s), s.host.top), (1, 1));
    // In the middle of the screen it stays.
    let mut s = tall(2);
    s.keys("<C-e>");
    assert_eq!((line(&s), s.host.top), (2, 1));
    s.keys("3<C-e>");
    assert_eq!((line(&s), s.host.top), (4, 4));
    // Scrolling up pulls it up from the bottom row.
    s.keys("<C-y>");
    assert_eq!((line(&s), s.host.top), (4, 3));
    s.keys("3<C-y>");
    assert_eq!((line(&s), s.host.top), (4, 0));
    // At the top of the document nothing moves.
    s.keys("<C-y>");
    assert_eq!((line(&s), s.host.top), (4, 0));
    let mut s = tall(4);
    s.host.top = 1;
    s.keys("<C-y>");
    assert_eq!((line(&s), s.host.top), (4, 0));
    s.host.top = 1;
    s.keys("2<C-y>");
    assert_eq!(line(&s), 4);
}

#[test]
fn aligning_the_caret_row() {
    let mut s = tall(10);
    let caret = Position::new(10, 0);
    s.keys("zz");
    assert_eq!(
        s.host.scrolls,
        vec![Scroll::Align {
            caret,
            align: ScrollAlign::Center
        }]
    );
    assert_eq!(s.host.top, 8);
    s.keys("zt");
    assert_eq!(s.host.top, 10);
    s.keys("zb");
    assert_eq!(s.host.top, 6);
    assert_eq!(line(&s), 10);
    s.keys("z.");
    assert_eq!(s.host.top, 8);
    s.keys("z<CR>");
    assert_eq!(s.host.top, 10);
    s.keys("z-");
    assert_eq!(s.host.top, 6);
    assert_eq!(s.mode(), Mode::Normal);
    // H, M and L follow the scrolled viewport.
    s.keys("H");
    assert_eq!(line(&s), 6);
    s.keys("L");
    assert_eq!(line(&s), 10);
    s.keys("M");
    assert_eq!(line(&s), 8);
}

#[test]
fn the_caret_lands_on_the_first_non_blank_for_some() {
    run(&[
        c("a\n  b|c\nd", "z<CR>", "a\n  |bc\nd"),
        c("a\n  b|c\nd", "z.", "a\n  |bc\nd"),
        c("a\n  b|c\nd", "z-", "a\n  |bc\nd"),
        c("a\n  b|c\nd", "zz", "a\n  b|c\nd"),
        c("a\n  b|c\nd", "zt", "a\n  b|c\nd"),
    ]);
}

#[test]
fn scrolling_in_visual_mode_extends_the_selection() {
    let mut s = tall(0);
    s.keys("V<C-d>");
    assert_eq!(s.mode(), Mode::VisualLine);
    assert_eq!(s.vim.visual_line_blocks(&s.ed), Some(0..3));
    s.keys("d");
    assert_eq!(s.ed.document().block_count(), 17);
}

#[test]
fn wrapped_rows_count_as_rows() {
    // One block of 30 characters in rows of 5, then short lines.
    let mut s = Session::with(
        "|abcdefghijklmnopqrstuvwxyz1234\nx\ny",
        5,
        4,
        VimOptions::default(),
    );
    s.keys("<C-d>");
    assert_eq!(s.host.scrolls, vec![Scroll::Rows(2)]);
    assert_eq!(s.buffer(), "abcdefghij|klmnopqrstuvwxyz1234\nx\ny");
    s.keys("ll<C-d>");
    assert_eq!(s.buffer(), "abcdefghijklmnopqrstuv|wxyz1234\nx\ny");
    s.keys("<C-d>");
    assert_eq!(s.buffer(), "abcdefghijklmnopqrstuvwxyz1234\n|x\ny");
}

/// As `tall`, with one row of `scrolloff`.
fn tall_with_margin(line: usize) -> Session {
    let mut s = tall(line);
    let mut options = s.vim.options().clone();
    options.scroll_off = 1;
    s.vim.set_options(options);
    s
}

#[test]
fn scroll_off_keeps_a_row_at_the_edges() {
    // In the middle of the document H and L stay one row inside.
    let mut s = tall_with_margin(10);
    s.host.top = 8;
    s.keys("H");
    assert_eq!(line(&s), 9);
    s.keys("L");
    assert_eq!(line(&s), 11);
    s.keys("M");
    assert_eq!(line(&s), 10);
    // A count that reaches further than the margin counts from the edge.
    s.keys("3H");
    assert_eq!(line(&s), 10);
    s.keys("1H");
    assert_eq!(line(&s), 9);
    s.keys("4L");
    assert_eq!(line(&s), 9);
    // Never past the other margin.
    s.keys("9H");
    assert_eq!(line(&s), 11);
    s.keys("9L");
    assert_eq!(line(&s), 9);

    // The first and the last row of the document have nothing to keep.
    let mut s = tall_with_margin(2);
    s.keys("H");
    assert_eq!(line(&s), 0);
    s.keys("L");
    assert_eq!(line(&s), 3);
    let mut s = tall_with_margin(17);
    s.host.top = 15;
    s.keys("L");
    assert_eq!(line(&s), 19);
    s.keys("H");
    assert_eq!(line(&s), 16);
}

#[test]
fn scroll_off_drags_the_caret_a_row_early() {
    // The caret is on the second visible row: one row down and it would
    // be on the first, so it moves along.
    let mut s = tall_with_margin(1);
    s.keys("<C-e>");
    assert_eq!((line(&s), s.host.top), (2, 1));
    // From the middle it stays.
    s.keys("j<C-e>");
    assert_eq!((line(&s), s.host.top), (3, 2));
    // Scrolling up pulls it up from the last row but one.
    let mut s = tall_with_margin(8);
    s.host.top = 5;
    s.keys("<C-y>");
    assert_eq!((line(&s), s.host.top), (7, 4));
    // Back at the top of the document the first row is allowed again.
    let mut s = tall_with_margin(0);
    s.host.top = 1;
    s.keys("j<C-y>");
    assert_eq!((line(&s), s.host.top), (1, 0));
    // The pages are counted from the viewport as the host reports it.
    let mut s = tall_with_margin(0);
    s.keys("<C-d>");
    assert_eq!(s.host.scrolls, vec![Scroll::Rows(2)]);
    s.keys("<C-f>");
    assert_eq!(s.host.scrolls.last(), Some(&Scroll::Rows(3)));
}
