//! Vim mode for The Writer: the glue between the pure [`writer_vim`] engine
//! and the GPUI [`writer_editor::Editor`].
//!
//! The engine turns keys into edits on a `writer_core::EditorState` and
//! reports what to show as plain data. This crate gives it a home in the
//! app: a [`VimMode`] entity per editor that routes the editor's keys to the
//! engine, answers the engine's questions about wrapped rows, scrolling and
//! the clipboard from the editor, keeps the editor's caret, text input and
//! highlights in step with the mode, and exposes status, a command line and
//! requests for the application.
//!
//! # Setup
//!
//! ```ignore
//! // once at startup, after writer_editor::init
//! writer_vim_mode::init(cx);
//! // optional: the palette entries ("Toggle Vim mode", "Clear search highlight")
//! palette.extend(writer_vim_mode::commands());
//!
//! // per editor, kept for as long as the editor lives
//! let vim = cx.new(|cx| VimMode::new(editor.clone(), cx));
//! let status = cx.new(|cx| VimStatusItem::new(vim.clone(), cx));
//! let command_line = cx.new(|cx| VimCommandLine::new(vim.clone(), cx));
//! workspace.update(cx, |workspace, cx| {
//!     workspace.set_status_item("vim", Some(status.into()), cx);
//!     workspace.set_editor_strip("vim-command-line", Some(command_line.into()), cx);
//! });
//!
//! // on and off (a setting, cmd-alt-v)
//! vim.update(cx, |vim, cx| vim.set_attached(settings.vim_mode, cx));
//!
//! // what the engine asks of the application
//! cx.subscribe(&vim, |this: &mut Shell, vim, event: &VimEvent, cx| match event {
//!     VimEvent::Request(HostRequest::Save) => this.save_now(cx),
//!     VimEvent::Request(HostRequest::Close { .. }) => this.close_document(cx),
//!     VimEvent::Request(HostRequest::SaveAndClose) => { this.save_now(cx); this.close_document(cx) }
//!     VimEvent::Request(HostRequest::Command { name, args, .. }) => {
//!         if !this.run_named_command(name, args, cx) {
//!             let error = format!("E492: Not an editor command: {name}");
//!             vim.update(cx, |vim, cx| vim.show_error(error, cx));
//!         }
//!     }
//!     // A toggle from the palette entry of this crate: remember the setting.
//!     VimEvent::Attached | VimEvent::Detached => this.write_vim_setting(vim.read(cx).is_attached(), cx),
//!     _ => {}
//! }).detach();
//!
//! // a status bar of your own instead of VimStatusItem
//! cx.observe(&vim, |this, vim, cx| { this.vim = vim.read(cx).status().clone(); cx.notify(); }).detach();
//! ```
//!
//! Both views can stay mounted all the time: the status item draws nothing
//! while Vim mode is off and the command line has no height while it is
//! closed.
//!
//! # What happens to a key
//!
//! While attached, a [`VimMode`] registers one key interceptor on its
//! editor at `InterceptPriority::MODAL`. For one key press of the focused
//! editor:
//!
//! 1. Interceptors at `InterceptPriority::OVERLAY` (an open insert menu)
//!    go first. That is the escape order: `escape` closes the open menu,
//!    the next `escape` reaches Vim. Popups that take the keyboard focus
//!    (format menu, link field, font picker) get their keys directly, since
//!    interception only applies while the editor itself is focused.
//! 2. Keys held with `cmd`, function keys, and every key of an IME
//!    composition in progress pass untouched: application and editor
//!    shortcuts work in every mode. One exception: `cmd-v` pastes into an
//!    open command line.
//! 3. Every other key goes to the engine. In Normal, Visual, operator
//!    pending and command-line mode the engine consumes nearly all of them
//!    (see the [`writer_vim`] guide for the few it leaves to the editor:
//!    option chords, shifted arrows, unknown control chords). In Insert and
//!    Replace mode it consumes only its own keys (`escape`, `ctrl-w`, …);
//!    typing takes the editor's normal path, so the platform input method,
//!    the Markdown input rules and smart punctuation all work.
//! 4. What the editor then inserts is reported back to the engine, which is
//!    what makes counts (`3ihello`), dot-repeat and Replace mode work: the
//!    typed character once the editor's `Edited` event confirms the
//!    insertion, and the text of an IME composition when it is committed.
//!
//! Outside Insert and Replace mode the editor's platform text input is
//! switched off (`Editor::set_text_input_enabled(false)`), so nothing
//! dictated, composed or picked from the emoji palette can reach the text
//! behind the engine's back.
//!
//! Changes that do not come from the engine (a mouse click, a `cmd`
//! shortcut, find and replace, the comments panel) arrive as editor events;
//! the engine is synced on each: a click in Normal mode puts the caret on a
//! character, a drag or `shift`-arrows start Visual mode.
//!
//! # What the editor shows
//!
//! - **Caret**: block in Normal and Visual mode, bar in Insert mode,
//!   underline in Replace mode and while an operator or `r` waits.
//! - **Visual selections** are ordinary editor selections covering the
//!   selected characters (so `cmd-b`, comments and copy work on them). The
//!   Vim cursor inside one is marked through the highlight layer
//!   [`CURSOR_LAYER`], because the editor's own caret sits at the far end
//!   of the selection.
//! - **Search matches**: highlight layer [`SEARCH_LAYER`], kept current
//!   across edits, cleared by `:noh`.
//! - **Key context**: the editor's context carries [`KEY_CONTEXT_MODE`]
//!   (`vim_mode=normal`, …) while attached.
//! - **Scrolling**: `ctrl-e/y/d/u/f/b`, `zz`, `zt`, `zb` scroll the editor.
//!   The viewport the engine sees leaves one row of context at the top and
//!   bottom (Vim's `scrolloff=1`), which is the margin the editor itself
//!   keeps around the caret.
//!
//! # Rows and the editor seam
//!
//! `j`, `k`, `gj`, `gk`, `g0`, `g$`, `H`, `M`, `L` and the scroll commands
//! depend on how text wraps, which only the editor's layout knows. See the
//! `host` module source for how those calls run (on a copy of the editor
//! state, because the editor cannot lend its state and answer layout
//! questions at the same time) and for the request that would remove the
//! copy.
//!
//! # Keyboard layouts
//!
//! The engine gets the character a key types (`shift-4` is `$`). Two
//! adjustments are made for it: a character that the layout types with the
//! option key (`{`, `[`, `|`, `@` on a German keyboard) is passed as that
//! character rather than as an option chord, and on a layout without Latin
//! letters a command key is taken by its Latin position, so `j` moves down
//! on a Russian layout too. Where a character is wanted (Insert mode, the
//! command line, the argument of `f`, `t`, `r`) the typed character counts.
//!
//! # Not covered
//!
//! - The command line takes keys only: no IME composition and no dictation
//!   into `:` or `/` (the engine's `command_line_input` is used for
//!   `cmd-v`).
//! - Text that reaches the document in Insert mode without a key and
//!   without a composition (the emoji palette, a system text replacement)
//!   is inserted, but the engine sees it as an outside change: it is not
//!   part of what `.` repeats, and what was typed before it in the same
//!   session is not either.
//! - In Visual Line mode an empty line at the end of the selection shows no
//!   tint (there is no text to tint); the status shows the mode.
//! - Whatever the engine does not support (macros, blockwise Visual mode,
//!   regular expressions): see the [`writer_vim`] crate docs.

mod host;
mod mode;
mod search;
mod status;
mod views;

use gpui::{App, actions};
use writer_ui::CommandDescriptor;

pub use mode::{CURSOR_LAYER, KEY_CONTEXT_MODE, SEARCH_LAYER, VimMode};
pub use status::{VimEvent, VimMessage, VimStatus};
pub use views::{VimCommandLine, VimStatusItem};
/// The engine types that appear in this crate's API.
pub use writer_vim::{
    CaretShape, CommandLine, HostRequest, Mode, RegisterContent, SearchPattern, VimOptions,
};

actions!(
    vim_mode,
    [
        /// Turn Vim mode on or off for the editor that was focused last.
        ToggleVimMode,
        /// Stop highlighting the matches of the last Vim search (`:noh`).
        ClearSearchHighlight,
    ]
);

/// The category of this crate's commands in the palette.
pub const CATEGORY_VIM: &str = "Vim";

/// Installs the handlers of this crate's actions. Call once at startup,
/// after `writer_editor::init`.
///
/// The actions are handled window-wide (they need no focus) and act on the
/// [`VimMode`] whose editor had the keyboard focus last, or on the only
/// one. No key is bound here: the application owns `cmd-alt-v`. An
/// application that keeps Vim mode in a setting can either bind its own
/// action and call [`VimMode::set_attached`], or use [`ToggleVimMode`] and
/// store the new state on [`VimEvent::Attached`] / [`VimEvent::Detached`].
pub fn init(cx: &mut App) {
    cx.on_action(|_: &ToggleVimMode, cx| {
        if let Some(vim) = mode::target_for_global_action(cx) {
            vim.update(cx, |vim, cx| vim.toggle(cx));
        }
    });
    cx.on_action(|_: &ClearSearchHighlight, cx| {
        if let Some(vim) = mode::target_for_global_action(cx) {
            vim.update(cx, |vim, cx| vim.execute("nohlsearch", cx));
        }
    });
}

/// The user-facing commands of this crate, for the command palette and the
/// shortcuts overlay.
pub fn commands() -> Vec<CommandDescriptor> {
    vec![
        CommandDescriptor::new("Toggle Vim mode", CATEGORY_VIM, ToggleVimMode).keywords([
            "modal",
            "vi",
            "normal mode",
            "keyboard",
        ]),
        CommandDescriptor::new("Clear search highlight", CATEGORY_VIM, ClearSearchHighlight)
            .keywords(["noh", "nohlsearch", "matches"]),
    ]
}
