//! Vim emulation engine for The Writer.
//!
//! A pure state machine: it turns keys into edits on a
//! [`writer_core::EditorState`]. It knows nothing about windows, fonts or
//! the clipboard; what it needs from the user interface goes through the
//! [`VimHost`] trait, and what it wants shown comes back as plain data
//! ([`Outcome`], [`Effect`], and a few getters on [`Vim`]).
//!
//! ```
//! # use std::ops::Range;
//! # use writer_core::{Block, Document, EditorState, Position};
//! # use writer_vim::*;
//! # struct OneRowPerLine;
//! # impl VimHost for OneRowPerLine {
//! #     fn visual_row(&mut self, s: &EditorState, p: Position) -> Range<usize> {
//! #         s.document().blocks()[p.block].line_range(p.offset)
//! #     }
//! #     fn column_of(&mut self, _: &EditorState, p: Position) -> f32 { p.offset as f32 }
//! #     fn move_by_rows(&mut self, _: &EditorState, from: Position, _: isize, _: GoalColumn) -> Position { from }
//! #     fn viewport(&mut self, _: &EditorState) -> Option<Viewport> { None }
//! #     fn scroll(&mut self, _: &EditorState, _: Scroll) {}
//! #     fn clipboard_read(&mut self) -> Option<RegisterContent> { None }
//! #     fn clipboard_write(&mut self, _: &RegisterContent) {}
//! # }
//! let mut editor = EditorState::new(Document::from_blocks(vec![Block::paragraph("hello brave world")]));
//! let mut host = OneRowPerLine;
//! let mut vim = Vim::new();
//! vim.attach(&mut editor, &mut host);
//!
//! // "wdw": to the second word, delete it.
//! for key in Key::parse_sequence("wdw") {
//!     let outcome = vim.handle_key(key, &mut editor, &mut host);
//!     assert!(outcome.consumed);
//! }
//! assert_eq!(editor.document().plain_text(), "hello world");
//! assert_eq!(vim.register('"').unwrap().text(), "brave ");
//! assert_eq!(vim.mode(), Mode::Normal);
//! ```
//!
//! # Driving the engine
//!
//! One [`Vim`] belongs to one editor.
//!
//! ## Switching Vim mode on and off
//!
//! - On: create a [`Vim`] (or keep one around, registers survive) and call
//!   [`Vim::attach`]. It resets to Normal mode, puts the caret on a
//!   character and returns effects describing the whole display state.
//! - Off, or the editor goes away: call [`Vim::detach`]. It ends an Insert
//!   session (closing the undo group the session holds open; without this
//!   every later edit would join one undo step), leaves Visual mode and
//!   clears the highlights.
//! - Another document is loaded into the same editor state: `detach`
//!   before, `attach` after. Marks and the jump list are dropped by
//!   `attach`.
//!
//! ## Keys
//!
//! Give every key press to [`Vim::handle_key`] *before* the editor's own
//! key bindings and text input, except:
//!
//! - keys held with the command key (they are application shortcuts; the
//!   engine has no notion of them);
//! - keys the platform input method takes while it has marked text (the
//!   engine must not see the keys of a composition);
//! - bare modifier presses.
//!
//! [`Key::from_keystroke`] builds a [`Key`] from the parts of a GPUI
//! keystroke. Then look at [`Outcome::consumed`]:
//!
//! - `true`: the key is done. Do not run a binding and do not insert text.
//! - `false`: handle the key exactly as without Vim.
//!
//! What is consumed depends on [`Vim::mode`]:
//!
//! | Mode | Consumed | Left to the host |
//! |---|---|---|
//! | Normal, Visual, Visual Line, operator pending | every character key (also one that means nothing: it must not be typed into the document), arrows, home, end, page keys, enter, tab, backspace (a motion in Normal mode, a delete in Visual mode), delete, and the control chords that are Vim commands (`ctrl-r`, `ctrl-o`, `ctrl-i`, `ctrl-d`, `ctrl-u`, `ctrl-f`, `ctrl-b`, `ctrl-e`, `ctrl-y`, `ctrl-h`, `ctrl-j`, `ctrl-m`, `ctrl-n`, `ctrl-p`, `ctrl-l`, `ctrl-g`, `ctrl-[`, `ctrl-c`) | option chords, shifted arrows / home / end / page keys, other control chords, and `escape` when there is nothing to cancel |
//! | Insert | `escape`, `ctrl-[`, `ctrl-c`, `ctrl-w`, `ctrl-u`, `ctrl-h`, `ctrl-o`, `ctrl-r` and the register name after it | everything else: typing, enter, backspace, arrows, tab, the editor's own control keys |
//! | Replace | as Insert, plus backspace (it restores what was overwritten) | as Insert |
//! | Command line | every key | nothing |
//!
//! While [`Mode::is_insert`] is false, switch the editor's text input off
//! altogether, so that dictation or an emoji picker cannot type into the
//! document behind the engine's back.
//!
//! ## Changes the engine did not make
//!
//! Whenever the editor reports that its selection or document changed,
//! call [`Vim::sync`]. It does nothing when the editor is as the engine
//! left it, and in Insert mode it accepts the change that follows a key the
//! engine left to the host, so the call is safe after every change report.
//! What it is for:
//!
//! - a mouse click or drag, shift-arrows, an Emacs-style `ctrl-a`: outside
//!   Insert mode the caret may now sit after the last character of a line
//!   or a selection may exist; `sync` moves the caret onto a character or
//!   starts Visual mode, and drops a half-typed command;
//! - a command from a menu or the palette (bold, find and replace, undo);
//! - in Insert mode, a click: what was typed before no longer repeats with
//!   `.`, as in Vim.
//!
//! One exception: do not call `sync` for the changes the platform input
//! method makes while it has marked text. They are neither keys nor typed
//! text yet; the commit is reported with [`Vim::text_inserted`].
//!
//! If `sync` is not called, the engine catches up by itself at the next
//! key outside Insert mode, but the caret and the mode indicator are wrong
//! until then.
//!
//! ## Typing in Insert mode
//!
//! In Insert and Replace mode the engine does not insert text. Typed
//! characters go through the editor's normal path (platform input method,
//! Markdown input rules, smart punctuation). Every time the editor inserts
//! typed or committed text, tell the engine with [`Vim::text_inserted`],
//! passing the text as it was typed, once per insertion (right after the
//! insertion, or right before it). That feeds:
//!
//! - counts: `3ihello<Esc>` types `hello` twice more on escape;
//! - dot-repeat of the session;
//! - the `.` register;
//! - Replace mode, where the engine then removes the characters the new
//!   text overwrites.
//!
//! Keys that are not text need no notification: the engine sees them pass
//! through `handle_key` and remembers enter, shift-enter, backspace,
//! option-backspace, forward delete and the Emacs keys `ctrl-d` / `ctrl-k`
//! as part of the session (a deleting key that turns out to delete nothing,
//! such as backspace at the start of the document, is dropped again).
//! Repeating types the recorded text again one character at a time through
//! [`EditorState::insert_text`](writer_core::EditorState::insert_text), so
//! input rules run as they did the first time.
//!
//! Cursor keys (arrows, home / end, page keys, `ctrl-a/e/f/b/n/p`) are left
//! to the host. If one moves the caret, the session starts afresh there, as
//! in Vim: what was typed before is an undo step of its own and is what `.`
//! repeats until something is typed at the new place. The engine finds out
//! whether the caret moved at the next key, the next `sync`, or the next
//! `text_inserted`; a cursor key that does not move the caret changes
//! nothing.
//!
//! `ctrl-w` and `ctrl-u` first delete back to where typing started (on the
//! same line), and go further when pressed again. In Replace mode they
//! move back and put back what was overwritten.
//!
//! `ctrl-o` runs one Normal-mode command. While it is awaited
//! [`Vim::mode`] is Normal and [`Vim::insert_pending`] is true. As in Vim,
//! a caret after the last character of a line first steps onto that
//! character, and typing resumes after it when the command leaves the
//! caret there (`A<C-o>~`) or goes to the end of the line (`<C-o>$`,
//! `<C-o>D`). The text typed after the command repeats as a plain insert.
//!
//! An Insert session is one undo step: the engine opens an undo group when
//! the session starts and closes it on escape (and around `ctrl-o` and a
//! cursor key that moves the caret). An editor "break the undo group on a
//! typing pause" timer does no harm (it has no effect inside a group).
//!
//! ## Showing state
//!
//! Every call returns the [`Effect`]s that happened, in order; state
//! changes are reported once, when they change:
//!
//! - [`Effect::ModeChanged`]: update the mode indicator and the caret
//!   shape ([`Vim::caret_shape`]); switch text input on or off.
//! - [`Effect::CommandLine`]: show, update or hide the command line
//!   ([`CommandLine`] has the prefix, the text and the cursor).
//! - [`Effect::PendingKeys`]: the keys of the unfinished command.
//! - [`Effect::SearchHighlight`]: mark the matches of the pattern in the
//!   visible blocks ([`SearchPattern::matches_in_block`]), or clear them.
//! - [`Effect::Message`] / [`Effect::Error`]: one line for the status area.
//! - [`Effect::Request`]: [`HostRequest::Save`], [`HostRequest::Close`],
//!   [`HostRequest::SaveAndClose`], or [`HostRequest::Command`] for an ex
//!   command the engine does not know (`:export md`): look the name up in
//!   the application's commands and report unknown ones yourself.
//! - [`Effect::Bell`]: the command failed.
//!
//! The same state can be polled: [`Vim::mode`], [`Vim::pending_keys`],
//! [`Vim::command_line`], [`Vim::search_highlight`], [`Vim::caret_shape`].
//!
//! **Where to draw the caret:** at [`Vim::caret`], not at the head of the
//! editor's selection. In Visual mode the editor's selection covers the
//! selected characters (so the editor's own formatting commands, comments
//! and copy work on a Visual selection), which puts its head *after* the
//! character the Vim cursor is on. In Visual Line mode
//! [`Vim::visual_line_blocks`] gives the selected blocks, for tinting
//! lines that have no text to show a selection on.
//!
//! Text for the command line that does not arrive as keys (committed input
//! method text while `:` or `/` is open) goes to
//! [`Vim::command_line_input`]. [`Vim::execute`] runs an ex command line
//! directly, for a command palette.
//!
//! ## The host trait
//!
//! [`VimHost`] answers what only the user interface knows:
//!
//! - wrapped rows: [`VimHost::visual_row`], [`VimHost::column_of`],
//!   [`VimHost::move_by_rows`] (for `j`, `k`, `g0`, `g$`, and the caret
//!   movement of the scroll commands);
//! - the viewport: [`VimHost::viewport`] (for `H`, `M`, `L`);
//! - scrolling: [`VimHost::scroll`] (for `ctrl-e/y/d/u/f/b`, `zz`, `zt`,
//!   `zb`);
//! - the system clipboard: [`VimHost::clipboard_read`],
//!   [`VimHost::clipboard_write`] (registers `+` and `*`).
//!
//! The layout methods receive the editor state, so the host object does
//! not have to borrow the editor: it needs the layout cache, not the
//! document. Scroll requests and clipboard writes may be collected and
//! carried out after the engine returns.
//!
//! # What "line" means here
//!
//! The document is a list of blocks, not of lines, and prose wraps.
//!
//! - A **hard line** is a block, or a segment between soft line breaks in
//!   a block. `0`, `^`, `$`, `g_`, `I`, `A`, `f`, `t`, `D`, `C`, `x` and
//!   the word and quote objects work on hard lines.
//! - `j` and `k` (and `gj`, `gk`) move by **visual rows**, keeping a goal
//!   column; `g0`, `g^`, `g$` work on the visual row. After `$` (or `g$`)
//!   vertical movement sticks to row ends.
//! - **Linewise** commands (`dd`, `yy`, `cc`, `>>`, `<<`, `J`, `G`, `gg`,
//!   `+`, `-`, `_`, Visual Line mode, `j` / `k` *after an operator*, ex
//!   ranges) count **blocks**, and their registers hold whole blocks with
//!   kind, indent and formatting: `ddp` moves a heading as a heading.
//!   `dj` deletes this block and the next, however many rows they wrap to.
//! - Inside a **code block** the hard lines are the lines: `dd`, `yy`,
//!   `p`, `cc`, `o`, `O`, `J`, `>>`, `<<` and Visual Line mode work on
//!   code lines as long as the range stays inside the block (`>` and `<`
//!   add or remove [`VimOptions::shift_width`] spaces). A range that
//!   leaves the block takes whole blocks.
//! - `>` and `<` change the block indent. `o` and `O` open a block the way
//!   the editor does: a list item or quote continues, anything else gets a
//!   paragraph. `cc` and `S` keep the block and its kind and empty it.
//! - `J` joins blocks with one space (none before `)`, after white space,
//!   or next to an empty block); the first block keeps its kind.
//! - `{` and `}` stop at empty blocks and at dividers and page breaks.
//!   `ip` / `ap` follow Vim's rules: a paragraph is a run of blocks with
//!   text, the lines between paragraphs are runs of empty blocks or blocks
//!   of nothing but blanks, and a run of dividers or page breaks is a
//!   paragraph of its own.
//! - Every block starts a sentence for `(`, `)`, `is`, `as`, and a sentence
//!   never runs on into the next block. An empty block is a stop for `(`
//!   and `)`; a run of them counts once.
//! - Word, quote and bracket objects work as in Vim: `aw` runs on over a
//!   line break when the white space after the word is the end of the
//!   line; brackets are matched through the whole document, and outside
//!   any pair the next pair after the caret is taken; quotes are matched on
//!   the hard line.
//! - A divider or page break is a block without text. It behaves as an
//!   empty line: the caret can rest on it, `dd` / `yy` / `p` treat it as a
//!   line, `J` removes it, typing on it starts a paragraph after it, and
//!   commands that need a character (`x`, `r`, `~`) do nothing.
//!
//! # Rich text
//!
//! - Registers hold [`writer_core::Fragment`]s: a yank and a put carry
//!   character formatting and links, and a linewise register carries block
//!   kinds, indent and alignment. Comment anchors never travel in
//!   fragments (a rule of the document model), so text that is deleted and
//!   put back elsewhere leaves its comment thread behind.
//! - `r`, `~`, `gu`, `gU`, `g~` and same-length `:s` replacements keep the
//!   formatting of every character, comment anchors included. Case changes
//!   map character to character (`ß` becomes `ẞ`, as in Vim), so they
//!   never change the length of the text.
//! - A change (`c`, `s`, `S`, `C`) types in the formatting of the first
//!   changed character, as typing over a selection does in the editor;
//!   Replace mode gives each typed character the formatting of the one it
//!   overwrites.
//! - Text put inside a block takes that block's kind; whole lines keep
//!   their own.
//!
//! # Supported commands
//!
//! Counts work everywhere Vim takes them, before and after an operator
//! (`2d3w`). `"x` names a register.
//!
//! - **Motions:** `h` `l` `j` `k`, arrows, space, backspace, `w` `W` `b`
//!   `B` `e` `E` `ge` `gE`, `0` `^` `$` `g_` `|`, `g0` `g^` `g$`, `gg` `G`
//!   `N%`, `+` `-` `_` enter, `{` `}` `(` `)`, `f` `F` `t` `T` `;` `,`,
//!   `%`, `H` `M` `L`, `*` `#` `g*` `g#`, `n` `N`, `/` `?`, `'x` `` `x ``.
//! - **Operators:** `d` `c` `y` `>` `<` `gu` `gU` `g~` with any motion or
//!   text object, and doubled (`dd`, `cc`, `yy`, `>>`, `<<`, `guu`, `gUU`,
//!   `g~~`). Text objects: `iw` `aw` `iW` `aW` `is` `as` `ip` `ap`, `i"`
//!   `a"` `i'` `a'` `` i` `` `` a` ``, `i(` `a(` `ib`, `i[` `a[`, `i{`
//!   `a{` `iB`, `i<` `a<`. The quote objects also match the typographic
//!   pairs `“ ”` and `‘ ’` that smart punctuation produces.
//! - **Changes:** `x` `X` `s` `S` `r` `R` `~` `D` `C` `Y` `J` `gJ` `p` `P`
//!   `gp` `gP` `u` `ctrl-r` `.` `&` `g&`.
//! - **Insert:** `i` `a` `I` `A` `gI` `gi` `o` `O`, with counts; in Insert
//!   mode `ctrl-w` `ctrl-u` `ctrl-h` `ctrl-o` `ctrl-r {register}`.
//! - **Visual:** `v` `V` `gv` `o`, every operator, `x` `X` `D` `s` `S` `C`
//!   `R` `Y` `u` `U` `~` `J` `gJ` `r` `p` `P`, text objects, `:`.
//! - **Marks and jumps:** `m{a-zA-Z}`, `'` and `` ` `` jumps (also `''`,
//!   `'[` `']` `'<` `'>` `'^` `'.`), `ctrl-o`, `ctrl-i` / tab.
//! - **Registers:** unnamed, `a`–`z` (upper case appends), `0` (last
//!   yank), `1`–`9` (deletes of a line or more), `-` (small deletes), `_`
//!   (black hole), `+` and `*` (system clipboard), and read-only `.`
//!   (last inserted text), `:` (last command line), `/` (last pattern).
//! - **Scrolling:** `ctrl-e` `ctrl-y` `ctrl-d` `ctrl-u` `ctrl-f` `ctrl-b`,
//!   page up / down, `zz` `zt` `zb` `z.` `z<CR>` `z-`.
//! - **Search:** `/` and `?` with history, incremental preview and
//!   highlights; `n` `N`; case-insensitive unless the pattern has an
//!   upper-case letter. Patterns are **literal text**, not regular
//!   expressions; `\c` / `\C` force the case rule, `\<word\>` matches
//!   whole words, `\/` `\\` `\n` `\t` are escapes.
//! - **Ex commands:** `:w` `:q` `:q!` `:wq` `:x` (as [`HostRequest`]s),
//!   `:{number}` and other ranges alone (go to that block), `:d [x] [N]`,
//!   `:y [x] [N]`, `:s/pattern/replacement/[g][i][I][n]` (literal pattern;
//!   `&` or `\0` in the replacement is the match, `\r` breaks the line into
//!   a new block, `\n` is a soft line break), `:&` `:&&` `:s`,
//!   `:j[!] [N]`, `:>` `:<`, `:u`, `:red`, `:noh`.
//!   A count `N` takes that many lines starting with the last of the range.
//!   Ranges: `N`, `.`, `$`, `'m`, `+N` / `-N` offsets, `a,b`, `%`; lines
//!   are blocks. Any other name is passed to the host. `ZZ` and `ZQ` too.
//!
//! Not supported: regular expressions, macros (`q`, `@`), blockwise Visual
//! mode (`ctrl-v`), `U`, folds, digraphs, `ctrl-a` / `ctrl-x`, `gq`, tag
//! objects (`it`, `at`), the `c` flag of `:s`, `:g`, `:m`, `:t`,
//! `:normal`.
//!
//! # Where the engine differs from Vim on purpose
//!
//! The engine is checked against a real Vim: `tests/vimdiff.rs` replays a
//! few thousand key sequences recorded from Vim 9.1 (see
//! `tests/fixtures/gen_vim_corpus.py`) over plain paragraphs and compares
//! text, cursor and registers. What differs on purpose:
//!
//! - `j` and `k` move by visual rows (as `gj` and `gk`); after an operator
//!   they count blocks.
//! - Every block is a paragraph: sentences (`(`, `)`, `is`, `as`) do not
//!   run from one block into the next, where Vim's run across line breaks.
//! - An apostrophe between two letters is part of the word (`don't`,
//!   `it’s`), so `cw`, `dw` and `ciw` treat such a word as one. Turn
//!   [`VimOptions::apostrophe_in_words`] off for Vim's rule.
//! - Searches and `:s` take literal text, not regular expressions; `/` and
//!   `?` use smartcase, `*` and `#` ignore case.
//! - A command that leaves the text exactly as it was (`r` with the same
//!   character, `gU` on capitals) is not an undo step; in Vim it is, and
//!   `u` then appears to do nothing once.
//! - Case operators over an empty motion (`gU0` in the first column) do
//!   nothing; Vim changes one character or the rest of the line there.
//! - Widths are counted in characters, not in screen cells: `.` after a
//!   Visual-mode command takes as many characters as were selected.
//! - The `.` register holds the text that was typed, without the special
//!   keys Vim keeps in it.
//! - `iw` on an empty last line selects nothing (Vim yanks the character
//!   before it), and a few more such edge cases recorded in
//!   `tests/fixtures/vim_deviations.tsv`.
//! - Emoji and CJK scripts are word classes of their own, as in Vim, and
//!   everything moves by grapheme cluster.

#![warn(missing_docs)]

mod buf;
mod effect;
mod engine;
mod ex;
mod host;
mod key;
mod motion;
mod register;
mod search;
mod textobj;

pub use effect::{CaretShape, CommandLine, Effect, HostRequest, Mode, Outcome, SearchPattern};
pub use engine::{Vim, VimOptions};
pub use host::{GoalColumn, Scroll, ScrollAlign, Viewport, VimHost};
pub use key::{Key, KeyCode};
pub use register::RegisterContent;
