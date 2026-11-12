//! "Keyboard shortcuts": the one place where the keys of The Writer are
//! looked up and changed.
//!
//! [`KeymapView`] is a modal view in the overlays' look: a query field on
//! top, the list of every command with its keys grouped by where the
//! keyboard has to be, hints at the bottom. In it the user finds a command
//! (by name, by place, by key), gives it another key, a second key or none,
//! puts one command or everything back to the defaults, and exports and
//! imports the shortcuts file.
//!
//! # The view shows and asks; the host does
//!
//! The view holds no keymap of its own and changes nothing. The host (the
//! app) hands it a [`Listing`]: a copy of its [`writer_keymap::Keymap`] and
//! the rows to list ([`CommandRow`]: the command and its place, its title,
//! category and heading, whether it can be changed). What the user decides
//! comes out as a [`Request`], an event of the view; the host carries it
//! out ([`Request::apply_to`] does the part that concerns the keymap),
//! installs and stores the keymap, and hands the view the new listing.
//!
//! ```ignore
//! let view = cx.new(|cx| KeymapView::new(listing(cx), window, cx));
//! cx.subscribe_in(&view, window, |this, view, request: &Request, window, cx| {
//!     match request {
//!         Request::Export => this.export_shortcuts(window, cx),
//!         Request::ChooseImportFile => this.choose_shortcuts_file(window, cx),
//!         // -> view.show_import_preview(name, plan, ..) or view.show_import_error(name, &error, ..)
//!         Request::Run { target } => this.run(target, window, cx),
//!         change => {
//!             if let Some(before) = change.apply_to(&mut this.keymap) {
//!                 this.install_and_store(cx);           // and offer Undo with `before`
//!             }
//!             view.update(cx, |view, cx| view.set_listing(listing(cx), window, cx));
//!         }
//!     }
//! })
//! .detach();
//! modal_layer.update(cx, |layer, cx| layer.show(view, window, cx));
//! ```
//!
//! The view emits [`gpui::DismissEvent`] when it wants to close, like every
//! modal of the app.
//!
//! # Changing a key
//!
//! On a row, [`ChangeKey`] (or a click on the key) makes the view wait for
//! the next keystroke. **The next key pressed is taken, whatever it is**:
//! no key ends the waiting instead of being recorded, so `escape` and
//! `enter` can be given to a command like any other key. (A modifier held
//! alone is not a key yet; the view shows it and goes on waiting.) The
//! pointer leaves the waiting with its Cancel control. The key reaches the
//! view before anything else sees it (one `App::intercept_keystrokes`
//! callback, installed by [`init`]): no binding of the app runs, the menu
//! bar does not get it, nothing is typed.
//!
//! Then the view says what the key would do, in the words of
//! [`writer_keymap::Verdict`]: free; refused, with the reason; taken, by
//! which command. The user chooses with the keys of the
//! [`CHOICES_KEY_CONTEXT`] or the pointer: use it, take it from the
//! command that has it, keep both where the two are in different places,
//! press another key, or cancel.
//!
//! A key that is held down repeats. The view does not take the repeats of
//! the key that started the waiting for the new key, nor those of the key
//! just recorded for an answer to its question: such a key counts again
//! once it was released (or another key was pressed, a modifier changed,
//! or two seconds went by without it; scripted keystrokes come without a
//! release, see [`KeymapView::key_released`]).
//!
//! # Keys
//!
//! All keys of the view are GPUI bindings, bound by [`init`] in three key
//! contexts, listed in the view like every other key and changeable there:
//!
//! - [`LIST_KEY_CONTEXT`] (`KeymapView > TextField`: the keyboard is in the
//!   query field while the list is shown): `up` / `down` (also `ctrl-p` /
//!   `ctrl-n`), `pageup` / `pagedown`, `home` / `end` (also `cmd-up` /
//!   `cmd-down`) move; `enter` changes the key of the selected row,
//!   `cmd-enter` adds another key, `cmd-backspace` removes the key, `cmd-d`
//!   puts the command back to its default, `cmd-r` runs it; `tab` /
//!   `shift-tab` choose among the keys of a row that has several;
//!   `ctrl-enter` (and `shift-f10`) opens the row's menu; `cmd-shift-c`
//!   shows the changed rows only; `cmd-i` imports, `cmd-e` exports,
//!   `cmd-shift-d` resets everything; `escape` closes.
//! - [`CHOICES_KEY_CONTEXT`] (a question is open): `left` / `right` (also
//!   `shift-tab` / `tab`) mark a choice, `enter` takes it, `backspace`
//!   goes back to waiting for a key, `up` / `down` / `pageup` / `pagedown`
//!   look through the import preview, `escape` cancels.
//! - [`RECORDING_KEY_CONTEXT`] has no bindings: every key is recorded.
//!
//! **Call [`init`] after `writer_ui::init`.** The query field is a
//! `writer_ui::TextField`, which binds `enter`, `escape`, `ctrl-enter`,
//! the arrows and `cmd-backspace` itself; the view's bindings are at the
//! same depth and win because they are added later.
//!
//! # Nobody is locked in
//!
//! The pointer always closes the view: its close control and a click
//! outside are not bindings. `escape` closes it (or cancels a question)
//! as long as the user did not give `escape` to another command of the
//! view: when the binding of [`Close`] is gone, the `escape` of the query
//! field and, last, the key itself (when no binding takes it) still close.
//!
//! # Tests and scripted runs
//!
//! [`KeymapView::describe`] is one line of state; [`KeymapView::press`]
//! presses a control by its label as a click would. Bounds for simulated
//! clicks come from debug selectors: `keymap-row-<n>` (a row, by its
//! position in the list as drawn), `keymap-key-<n>-<k>` (a key of that
//! row), `keymap-add-<n>`, `keymap-more-<n>`, `keymap-import`,
//! `keymap-export`, `keymap-reset-all`, `keymap-changed-only`,
//! `keymap-close`, `keymap-record-cancel`, `keymap-choice-<n>`,
//! `keymap-preview-row-<n>`, `keymap-panel`, `keymap-band`.

mod keys;
mod listing;
mod question;
mod record;
mod render;
mod request;
mod search;
mod view;
mod words;

use gpui::{App, KeyBinding, actions};

pub use keys::{from_pressed, glyph_list, glyphs};
pub use listing::{CommandRow, Listing, Locked};
pub use question::Choice;
pub use request::Request;
pub use view::{KeymapView, ModeKind};
pub use words::{Line, Tone};

actions!(
    keymap_view,
    [
        /// Select the previous row.
        SelectPrevious,
        /// Select the next row.
        SelectNext,
        /// Move the selection up by a page.
        SelectPageUp,
        /// Move the selection down by a page.
        SelectPageDown,
        /// Select the first row.
        SelectFirst,
        /// Select the last row.
        SelectLast,
        /// Change the key of the selected row: wait for the next keystroke.
        ChangeKey,
        /// Give the selected row one more key.
        AddKey,
        /// Take the key from the selected row.
        RemoveKey,
        /// Put the selected row back to its default keys.
        UseDefault,
        /// Run the command of the selected row.
        RunCommand,
        /// Choose the next key of a row that has several.
        NextKey,
        /// Choose the previous key of a row that has several.
        PreviousKey,
        /// Show only the rows whose keys were changed, or all of them.
        ToggleChangedOnly,
        /// Import a shortcuts file.
        Import,
        /// Export the shortcuts to a file.
        Export,
        /// Put every key back to its default.
        ResetAll,
        /// Close the keyboard shortcuts.
        Close,
        /// Mark the next choice of a question.
        NextChoice,
        /// Mark the previous choice of a question.
        PreviousChoice,
        /// Take the marked choice.
        Choose,
        /// Go back to waiting for a key.
        PressAnotherKey,
        /// Leave a question without changing anything.
        Cancel,
    ]
);

/// The key context of the view's panel. The three below lie inside it.
pub const KEY_CONTEXT: &str = "KeymapView";

/// Where the keys of the list apply: the query field inside the panel has
/// the keyboard while the list is shown.
pub const LIST_KEY_CONTEXT: &str = "KeymapView > TextField";

/// Where the keys of a question apply (use this key?, reset everything?,
/// the import preview).
pub const CHOICES_KEY_CONTEXT: &str = "KeymapChoices";

/// Where the keyboard is while the view waits for a key. Nothing is bound
/// here: every key is recorded.
pub const RECORDING_KEY_CONTEXT: &str = "KeymapRecording";

/// The key contexts of the list, outermost first, for whoever asks the
/// keymap what a key does there.
pub(crate) const LIST_STACK: [&str; 2] = [KEY_CONTEXT, "TextField"];

/// The key contexts of a question, outermost first.
pub(crate) const CHOICES_STACK: [&str; 2] = [KEY_CONTEXT, CHOICES_KEY_CONTEXT];

/// The key bindings of the view. See the [crate docs](crate) for the list.
/// Alternatives come first, the key to show last.
pub fn key_bindings() -> Vec<KeyBinding> {
    use writer_ui::context_menu::OpenContextMenu;
    let list = Some(LIST_KEY_CONTEXT);
    let choices = Some(CHOICES_KEY_CONTEXT);
    vec![
        KeyBinding::new("ctrl-p", SelectPrevious, list),
        KeyBinding::new("up", SelectPrevious, list),
        KeyBinding::new("ctrl-n", SelectNext, list),
        KeyBinding::new("down", SelectNext, list),
        KeyBinding::new("pageup", SelectPageUp, list),
        KeyBinding::new("pagedown", SelectPageDown, list),
        KeyBinding::new("cmd-up", SelectFirst, list),
        KeyBinding::new("home", SelectFirst, list),
        KeyBinding::new("cmd-down", SelectLast, list),
        KeyBinding::new("end", SelectLast, list),
        KeyBinding::new("enter", ChangeKey, list),
        KeyBinding::new("cmd-enter", AddKey, list),
        KeyBinding::new("cmd-backspace", RemoveKey, list),
        KeyBinding::new("cmd-d", UseDefault, list),
        KeyBinding::new("cmd-r", RunCommand, list),
        KeyBinding::new("tab", NextKey, list),
        KeyBinding::new("shift-tab", PreviousKey, list),
        KeyBinding::new("cmd-shift-c", ToggleChangedOnly, list),
        KeyBinding::new("cmd-i", Import, list),
        KeyBinding::new("cmd-e", Export, list),
        KeyBinding::new("cmd-shift-d", ResetAll, list),
        // `shift-f10` first, so key hints show `ctrl-enter`.
        KeyBinding::new("shift-f10", OpenContextMenu, list),
        KeyBinding::new("ctrl-enter", OpenContextMenu, list),
        KeyBinding::new("escape", Close, list),
        KeyBinding::new("shift-tab", PreviousChoice, choices),
        KeyBinding::new("left", PreviousChoice, choices),
        KeyBinding::new("tab", NextChoice, choices),
        KeyBinding::new("right", NextChoice, choices),
        KeyBinding::new("up", SelectPrevious, choices),
        KeyBinding::new("down", SelectNext, choices),
        KeyBinding::new("pageup", SelectPageUp, choices),
        KeyBinding::new("pagedown", SelectPageDown, choices),
        KeyBinding::new("home", SelectFirst, choices),
        KeyBinding::new("end", SelectLast, choices),
        KeyBinding::new("enter", Choose, choices),
        KeyBinding::new("backspace", PressAnotherKey, choices),
        KeyBinding::new("escape", Cancel, choices),
    ]
}

/// Binds the keys of the view and installs the callback through which a
/// view that waits for a key sees every keystroke first. Call once at
/// startup, **after `writer_ui::init`** (see the [crate docs](crate)).
pub fn init(cx: &mut App) {
    cx.bind_keys(key_bindings());
    record::init(cx);
}
