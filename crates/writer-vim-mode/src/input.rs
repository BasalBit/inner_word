//! Platform text input for the command line and for commands that wait for
//! a character.
//!
//! The keys of the command line arrive through the editor's key
//! interception. For a key that types a character that is not enough: a
//! dead key (`option-e`) reaches the key path looking like any option
//! chord, and only the platform's text input makes `é` of it and the
//! letter that follows (holding the accent as marked text in between, in
//! an active window). The same goes for an input method and for the emoji
//! palette. Outside Insert mode the editor refuses platform text (it
//! would land in the document), so while the command line is open, or
//! `f`, `t` or `r` waits for its character, [`VimMode`] is the window's
//! input handler itself: [`crate::VimCommandLine`] registers it in its
//! paint, for the editor's focus handle. From the frame that did so, the
//! keys that type text are left to the text input (as the editor leaves
//! them in Insert mode), and the text it delivers goes to the engine.
//! Until that frame, and when no command line view is mounted, the engine
//! takes the keys itself, as before: nothing typed quickly after `:` is
//! lost, it just cannot be composed.
//!
//! The platform talks in UTF-16 offsets into "the document". Here that is
//! the text of the command line with the marked text at its cursor, or the
//! marked text alone while a command waits for a character.

use std::ops::Range;

use gpui::{Bounds, Context, EntityInputHandler, Pixels, Point, UTF16Selection, Window};

use crate::mode::VimMode;

/// The text the platform sees, and the byte range of the marked text in it.
fn composed(vim: &VimMode) -> (String, Range<usize>) {
    let marked = vim.composing().unwrap_or_default();
    match &vim.status().command_line {
        Some(line) => {
            let cursor = floor_char_boundary(&line.text, line.cursor);
            let mut text = String::with_capacity(line.text.len() + marked.len());
            text.push_str(&line.text[..cursor]);
            text.push_str(marked);
            text.push_str(&line.text[cursor..]);
            (text, cursor..cursor + marked.len())
        }
        None => (marked.to_string(), 0..marked.len()),
    }
}

fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn utf16_offset(text: &str, offset: usize) -> usize {
    text[..floor_char_boundary(text, offset)]
        .encode_utf16()
        .count()
}

fn byte_offset(text: &str, utf16: usize) -> usize {
    let mut units = 0;
    for (index, character) in text.char_indices() {
        if units >= utf16 {
            return index;
        }
        units += character.len_utf16();
    }
    text.len()
}

impl EntityInputHandler for VimMode {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let (text, _) = composed(self);
        let start = byte_offset(&text, range_utf16.start);
        let end = byte_offset(&text, range_utf16.end).max(start);
        adjusted_range.replace(utf16_offset(&text, start)..utf16_offset(&text, end));
        Some(text[start..end].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        if !self.wants_text_input() {
            return None;
        }
        // The cursor, after the marked text.
        let (text, marked) = composed(self);
        let cursor = utf16_offset(&text, marked.end);
        Some(UTF16Selection {
            range: cursor..cursor,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.composing()?;
        let (text, marked) = composed(self);
        Some(utf16_offset(&text, marked.start)..utf16_offset(&text, marked.end))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        // The composition ends without a result: what is marked is not
        // typed (an accent alone is no pattern and no argument).
        self.set_composition(None, cx);
    }

    fn replace_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The result of a composition, or text without one (the emoji
        // palette). A range other than the marked text would name text
        // typed earlier (the accent menu of a held key); a held key does
        // not get that far here, its presses are command-line keys.
        self.text_from_platform(text, cx);
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range_utf16: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.wants_text_input() {
            return;
        }
        self.set_composition(Some(new_text.to_string()), cx);
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        element_bounds: Bounds<Pixels>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Where a candidate window goes: at the command line, or at the
        // caret while a command waits for a character.
        if self.status().command_line.is_some() {
            Some(element_bounds)
        } else {
            self.editor()
                .read(cx)
                .caret_bounds()
                .or(Some(element_bounds))
        }
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
