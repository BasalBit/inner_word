//! The native menu bar: the complete map of the app for the mouse.
//!
//! App menu, File, Edit, Format, Insert, View, Window, Help. Every item is
//! an action that already exists, the one its key runs, so a menu item and
//! its key can never do different things. GPUI enables an item when its
//! action has a handler on the way from the keyboard to the window, and
//! greys it out otherwise: the commands for the text are greyed while no
//! document is open, and work wherever the keyboard is while one is (the
//! workspace passes them on to the text, see `Workspace::forward_to_text`).
//!
//! The product's "few menus" rule is about chrome inside the window; the
//! system menu bar is where macOS users look for About, Settings and Quit,
//! and where someone who does not know the keys finds every command.
//!
//! The standard items of the Edit menu (undo, redo, cut, copy, paste,
//! select all) are the editor's actions. They act on whatever has the
//! keyboard: the workspace turns them into the text field's own actions
//! when a text field has it (see `Workspace::edit_where_the_keyboard_is`).
//!
//! # The key beside an item
//!
//! GPUI takes the key of a menu item from the keymap, but not the one the
//! rest of the app shows: it takes the *first* binding of the action whose
//! key context holds in a made-up `Workspace Pane Editor` context, or
//! failing that the first binding at all (`create_menu_item` in GPUI's
//! `platform/mac/platform.rs`). For several actions that is not the key
//! the palette and the shortcuts overlay show ("Find Previous" got
//! `shift-enter`, a key of the find bar), and a key named `enter` reaches
//! AppKit as that word instead of the return key.
//!
//! So the keys the menu bar should show are bound once more, in
//! [`MENU_KEY_CONTEXT`]: a context no element of the app ever has, which
//! holds in GPUI's made-up one. These bindings are added before all others
//! ([`menu_key_bindings`], first in `crate::init`), so GPUI finds them
//! first for the menu, they never run, and key hints (which show the
//! binding added last) never show them. A test holds every item's key to
//! a key that really runs its action. Three items whose only key has
//! return in it show no key (see [`menu_key_bindings`]).

use gpui::{Action, App, KeyBinding, KeyContext, Keymap, Menu, MenuItem, OsAction, SystemMenuType};
use writer_assistant_ui as assistant;
use writer_comments as comments;
use writer_editor::actions as editor;
use writer_editor_tools::actions as tools;
use writer_spell::actions as spell;

use crate::actions::*;

/// The key context of the bindings that exist only to give menu items
/// their key. No element has it; GPUI's menu code evaluates bindings in a
/// context that does.
pub const MENU_KEY_CONTEXT: &str = "Pane";

/// The menus of the app, with the assistant's items.
pub fn app_menus() -> Vec<Menu> {
    app_menus_with(true)
}

/// The menus of the app. `assistant` is whether the assistant is shown
/// (its engine is installed and the setting is on): without it the menu
/// bar has none of its items, as the rest of the window has nothing of it.
pub fn app_menus_with(assistant: bool) -> Vec<Menu> {
    let mut menus = base_menus();
    if assistant {
        add_assistant_items(&mut menus);
    }
    menus
}

/// The name of the menu the quick chat is in.
const EDIT_MENU: &str = "Edit";
/// The name of the menu the side chat is in.
const VIEW_MENU: &str = "View";

/// Puts the assistant's items where a writer looks for them: asking about
/// the selection is something done to the text (Edit, after Find), the
/// side chat is a panel like the library and the comments (View).
fn add_assistant_items(menus: &mut [Menu]) {
    for menu in menus {
        match menu.name.as_ref() {
            EDIT_MENU => {
                let at = menu
                    .items
                    .iter()
                    .position(|item| {
                        matches!(item, MenuItem::Submenu(submenu) if submenu.name.as_ref() == "Spelling")
                    })
                    .unwrap_or(menu.items.len());
                menu.items
                    .insert(at, MenuItem::action("Ask the Assistant…", AskAssistant));
                menu.items.insert(at + 1, MenuItem::separator());
            }
            VIEW_MENU => {
                let toggle = menu
                    .items
                    .iter()
                    .position(|item| {
                        matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Show or Hide Comments")
                    })
                    .map_or(menu.items.len(), |ix| ix + 1);
                menu.items.insert(
                    toggle,
                    MenuItem::action("Show or Hide Assistant", ToggleAssistant),
                );
                let go = menu
                    .items
                    .iter()
                    .position(|item| {
                        matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Go to Comments")
                    })
                    .map_or(menu.items.len(), |ix| ix + 1);
                menu.items
                    .insert(go, MenuItem::action("Go to Assistant", FocusAssistant));
                let end = menu
                    .items
                    .iter()
                    .position(|item| {
                        matches!(item, MenuItem::Action { name, .. } if name.as_ref() == "Command Palette…")
                    })
                    .unwrap_or(menu.items.len());
                menu.items
                    .insert(end, MenuItem::action("New Chat", assistant::NewChat));
                menu.items.insert(
                    end + 1,
                    MenuItem::action("Clear Chat", assistant::ClearChat),
                );
                menu.items.insert(end + 2, MenuItem::separator());
            }
            _ => {}
        }
    }
}

fn base_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "The Writer".into(),
            items: vec![
                MenuItem::action("About The Writer", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("Agents…", ShowAgents),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide The Writer", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit The Writer", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Document", NewDocument),
                MenuItem::action("New Folder", NewFolder),
                MenuItem::separator(),
                MenuItem::action("Open Document…", QuickOpen),
                MenuItem::action("Search Library…", SearchLibrary),
                MenuItem::separator(),
                MenuItem::action("Save Now", SaveNow),
                MenuItem::action("Rename…", RenameDocument),
                MenuItem::action("Export…", Export),
                MenuItem::separator(),
                MenuItem::action("Version History…", ShowVersionHistory),
                MenuItem::action("Document Settings…", OpenDocumentSettings),
                MenuItem::action("Reveal in Library", RevealInLibrary),
                MenuItem::separator(),
                MenuItem::action("Close Window", CloseWindow),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", editor::Undo, OsAction::Undo),
                MenuItem::os_action("Redo", editor::Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", editor::Cut, OsAction::Cut),
                MenuItem::os_action("Copy", editor::Copy, OsAction::Copy),
                MenuItem::os_action("Paste", editor::Paste, OsAction::Paste),
                MenuItem::action("Paste as Plain Text", editor::PasteAsPlainText),
                MenuItem::separator(),
                MenuItem::os_action("Select All", editor::SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", tools::Find),
                MenuItem::action("Find and Replace…", tools::FindAndReplace),
                MenuItem::action("Find Next", tools::FindNext),
                MenuItem::action("Find Previous", tools::FindPrevious),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Spelling".into(),
                    items: vec![
                        // Carries a check mark while the setting is on
                        // (`set_check_mark`, from `Workspace::apply_spelling`).
                        MenuItem::action(
                            crate::workspace::CHECK_SPELLING_MENU_ITEM,
                            spell::ToggleSpellChecking,
                        ),
                        MenuItem::separator(),
                        MenuItem::action("Next Misspelling", spell::NextMisspelling),
                        MenuItem::action("Previous Misspelling", spell::PreviousMisspelling),
                        MenuItem::action("Show Suggestions", spell::ShowSuggestions),
                        MenuItem::separator(),
                        MenuItem::action("Learn Spelling", spell::LearnSpelling),
                        MenuItem::action("Unlearn Spelling", spell::UnlearnSpelling),
                        MenuItem::action("Ignore Spelling", spell::IgnoreSpelling),
                        MenuItem::separator(),
                        MenuItem::action("Spelling Language…", ChooseSpellingLanguage),
                    ],
                }),
            ],
        },
        Menu {
            name: "Format".into(),
            items: vec![
                MenuItem::action("Bold", editor::ToggleBold),
                MenuItem::action("Italic", editor::ToggleItalic),
                MenuItem::action("Underline", editor::ToggleUnderline),
                MenuItem::action("Strikethrough", editor::ToggleStrikethrough),
                MenuItem::action("Inline Code", editor::ToggleInlineCode),
                MenuItem::action("Highlight", editor::ToggleHighlight),
                MenuItem::separator(),
                MenuItem::action("Add or Edit Link…", tools::OpenLink),
                MenuItem::action("Remove Link", tools::RemoveLink),
                MenuItem::action("Font…", tools::OpenFontPicker),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Turn Into".into(),
                    items: vec![
                        MenuItem::action("Paragraph", editor::SetParagraph),
                        MenuItem::action("Heading 1", editor::SetHeading1),
                        MenuItem::action("Heading 2", editor::SetHeading2),
                        MenuItem::action("Heading 3", editor::SetHeading3),
                        MenuItem::action("Heading 4", editor::SetHeading4),
                        MenuItem::action("Heading 5", editor::SetHeading5),
                        MenuItem::action("Heading 6", editor::SetHeading6),
                        MenuItem::separator(),
                        MenuItem::action("Bullet List", editor::ToggleBulletList),
                        MenuItem::action("Numbered List", editor::ToggleNumberedList),
                        MenuItem::action("To-do List", editor::ToggleTodoList),
                        MenuItem::separator(),
                        MenuItem::action("Quote", editor::ToggleQuote),
                        MenuItem::action("Code Block", editor::ToggleCodeBlock),
                    ],
                }),
                MenuItem::submenu(Menu {
                    name: "Align".into(),
                    items: vec![
                        MenuItem::action("Left", editor::AlignLeft),
                        MenuItem::action("Centre", editor::AlignCenter),
                        MenuItem::action("Right", editor::AlignRight),
                        MenuItem::action("Justify", editor::AlignJustify),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::action("Larger Text", editor::IncreaseTextSize),
                MenuItem::action("Smaller Text", editor::DecreaseTextSize),
                MenuItem::action("Reset Text Size", editor::ResetTextSize),
                MenuItem::separator(),
                MenuItem::action("Indent", editor::Indent),
                MenuItem::action("Outdent", editor::Outdent),
                MenuItem::action("Move Block Up", editor::MoveBlockUp),
                MenuItem::action("Move Block Down", editor::MoveBlockDown),
                MenuItem::action("Tick or Untick To-do", editor::ToggleTodoChecked),
                MenuItem::separator(),
                MenuItem::action("Clear Formatting", editor::ClearFormatting),
                MenuItem::action("Format Menu…", tools::OpenFormatMenu),
            ],
        },
        Menu {
            name: "Insert".into(),
            items: vec![
                MenuItem::action("Divider", editor::InsertDivider),
                MenuItem::action("Page Break", editor::InsertPageBreak),
                MenuItem::action("Line Break", editor::InsertSoftBreak),
                MenuItem::action("Today's Date", tools::InsertDate),
                MenuItem::separator(),
                MenuItem::action("Link…", tools::OpenLink),
                MenuItem::action("Comment", comments::AddComment),
                MenuItem::separator(),
                MenuItem::action("Insert Menu…", tools::OpenInsertMenu),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Show or Hide Library", ToggleSidebar),
                MenuItem::action("Show or Hide Comments", comments::ToggleComments),
                MenuItem::action("Turn Page View On or Off", TogglePageView),
                MenuItem::action("Turn Vim Mode On or Off", ToggleVimMode),
                MenuItem::separator(),
                MenuItem::submenu(Menu {
                    name: "Theme".into(),
                    items: vec![
                        MenuItem::action("Paper", UseLightTheme),
                        MenuItem::action("Night", UseDarkTheme),
                        MenuItem::action("Follow the System", UseSystemTheme),
                    ],
                }),
                MenuItem::separator(),
                MenuItem::action("Go to Library", FocusLibrary),
                MenuItem::action("Go to Text", FocusEditor),
                MenuItem::action("Go to Comments", comments::FocusComments),
                MenuItem::action("Next Comment", comments::NextComment),
                MenuItem::action("Previous Comment", comments::PreviousComment),
                MenuItem::separator(),
                MenuItem::action("Command Palette…", ToggleCommandPalette),
                // No full screen item: macOS adds its own to a menu called
                // "View", and two of them were listed.
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ],
        },
        Menu {
            name: "Help".into(),
            items: vec![
                MenuItem::action("Keyboard Shortcuts", ShowShortcuts),
                MenuItem::action("Welcome Document", OpenWelcomeDocument),
            ],
        },
    ]
}

/// Every action item of `menus`, submenus included, with its name.
pub fn menu_actions(menus: &[Menu]) -> Vec<(String, Box<dyn Action>)> {
    fn walk(items: &[MenuItem], out: &mut Vec<(String, Box<dyn Action>)>) {
        for item in items {
            match item {
                MenuItem::Action { name, action, .. } => {
                    out.push((name.to_string(), action.boxed_clone()))
                }
                MenuItem::Submenu(menu) => walk(&menu.items, out),
                MenuItem::Separator | MenuItem::SystemMenu(_) => {}
            }
        }
    }
    let mut out = Vec::new();
    for menu in menus {
        walk(&menu.items, &mut out);
    }
    out
}

/// The bindings that give menu items the key the rest of the app shows
/// for their action (see the [module docs](self)). They never run. Add
/// them before every other binding.
pub fn menu_key_bindings() -> Vec<KeyBinding> {
    fn key(keystroke: &str, action: impl Action) -> KeyBinding {
        KeyBinding::new(keystroke, action, Some(MENU_KEY_CONTEXT))
    }
    vec![
        // The find bar's own key for this (shift-enter) comes first in the
        // keymap.
        key("cmd-shift-g", tools::FindPrevious),
        // The spelling the keyboard really sends (cmd-:) comes first in the
        // keymap.
        key("cmd-shift-;", spell::PreviousMisspelling),
        // Shift with a digit or punctuation is bound in two spellings (the
        // character a keyboard really sends, and the readable one): the
        // menu shows the readable one, like the rest of the app.
        key("cmd-shift-7", editor::ToggleNumberedList),
        key("cmd-shift-8", editor::ToggleBulletList),
        key("cmd-shift-9", editor::ToggleTodoList),
        key("cmd-shift-'", editor::ToggleCodeBlock),
        key("cmd-shift-.", editor::IncreaseTextSize),
        key("cmd-shift-,", editor::DecreaseTextSize),
        // Keys with return in them. GPUI hands AppKit the word "enter"
        // where AppKit wants a return character, and a binding cannot be
        // written with one. A sequence of two keys gets no key in the menu
        // at all, which is better than a wrong one: these three items show
        // none. (Indent and Outdent show their second keys, cmd-] and
        // cmd-[, instead of tab.)
        key("cmd-enter cmd-enter", editor::InsertPageBreak),
        key("shift-enter shift-enter", editor::InsertSoftBreak),
        key("cmd-shift-enter cmd-shift-enter", editor::ToggleTodoChecked),
    ]
}

/// The context GPUI evaluates key contexts in when it looks for the key of
/// a menu item.
fn native_menu_context() -> Vec<KeyContext> {
    let mut context = KeyContext::new_with_defaults();
    for name in ["Workspace", "Pane", "Editor"] {
        context.add(name);
    }
    vec![context]
}

/// The binding whose key GPUI puts beside the menu item of `action`: the
/// first one whose context holds in [`native_menu_context`], else the
/// first one. (A copy of GPUI's rule, for the test that holds the menu
/// bar to the keys the app shows elsewhere.)
pub fn native_menu_binding<'a>(
    keymap: &'a Keymap,
    action: &'a dyn Action,
) -> Option<&'a KeyBinding> {
    let context = native_menu_context();
    let mut first = None;
    for binding in keymap.bindings_for_action(action) {
        if binding
            .predicate()
            .is_none_or(|predicate| predicate.eval_inner(&context, &context))
        {
            return Some(binding);
        }
        first.get_or_insert(binding);
    }
    first
}

/// Installs the menu bar. Call after the key bindings are in place: the
/// shortcuts in the menus are read from the keymap at this moment.
pub fn install(cx: &mut App) {
    let assistant = assistant_shown(cx);
    cx.set_menus(app_menus_with(assistant));
    // A new menu bar has new items: their check marks go on again.
    apply_check_marks(cx);
}

// ----- the assistant's items -----------------------------------------------------

/// Whether the menu bar has the assistant's items.
#[derive(Default)]
struct AssistantItems(bool);

impl gpui::Global for AssistantItems {}

/// Whether the menu bar has the assistant's items right now.
pub fn assistant_shown(cx: &App) -> bool {
    cx.try_global::<AssistantItems>()
        .is_some_and(|items| items.0)
}

/// Gives the menu bar the assistant's items or takes them out, and builds
/// it again when that changed. The workspace calls this when it learns
/// whether the assistant's engine is installed, and when the setting that
/// switches the assistant off changes.
pub fn set_assistant_shown(shown: bool, cx: &mut App) {
    if assistant_shown(cx) == shown {
        return;
    }
    cx.set_global(AssistantItems(shown));
    install(cx);
}

// ----- check marks -------------------------------------------------------------

/// The check marks of the menu bar. GPUI's menu items have no state of
/// their own, so the app sets the state of the native item itself (see
/// [`set_check_mark`]).
#[derive(Default)]
struct CheckMarks {
    /// Whether this process has a real menu bar to mark: only the app
    /// itself ([`manage_check_marks`]), never a test.
    native: bool,
    /// What every marked item should show, by its title.
    marks: Vec<(String, bool)>,
}

impl gpui::Global for CheckMarks {}

/// From now on [`set_check_mark`] reaches the native menu bar. Called by
/// `main`; tests keep the bookkeeping only (there is no menu bar to mark,
/// and a test must not start an `NSApplication`).
pub fn manage_check_marks(cx: &mut App) {
    cx.default_global::<CheckMarks>().native = true;
    apply_check_marks(cx);
}

/// Shows or removes the check mark beside the menu bar item titled
/// `title` ("Check Spelling While Typing"). The mark is kept and put back
/// when the menu bar is built again ([`install`]).
pub fn set_check_mark(title: &str, on: bool, cx: &mut App) {
    let marks = cx.default_global::<CheckMarks>();
    match marks.marks.iter_mut().find(|(name, _)| name == title) {
        Some(mark) if mark.1 == on => return,
        Some(mark) => mark.1 = on,
        None => marks.marks.push((title.to_string(), on)),
    }
    apply_check_marks(cx);
}

/// Whether the menu bar item titled `title` is to show a check mark
/// (`None`: nobody has said).
pub fn check_mark(title: &str, cx: &App) -> Option<bool> {
    cx.try_global::<CheckMarks>()?
        .marks
        .iter()
        .find(|(name, _)| name == title)
        .map(|(_, on)| *on)
}

fn apply_check_marks(cx: &mut App) {
    let Some(marks) = cx.try_global::<CheckMarks>() else {
        return;
    };
    if !marks.native {
        return;
    }
    for (title, on) in &marks.marks {
        set_native_state(title, *on);
    }
}

/// Finds the item titled `title` in the main menu (submenus included) and
/// sets its state.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
fn set_native_state(title: &str, on: bool) {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::{ffi::CStr, os::raw::c_char};

    /// SAFETY: `menu` is a live `NSMenu` (or nil); only AppKit accessors
    /// are called, on the main thread, and nothing is kept.
    unsafe fn walk(menu: *mut Object, title: &str, on: bool) {
        if menu.is_null() {
            return;
        }
        unsafe {
            let count: isize = msg_send![menu, numberOfItems];
            for ix in 0..count {
                let item: *mut Object = msg_send![menu, itemAtIndex: ix];
                if item.is_null() {
                    continue;
                }
                let submenu: *mut Object = msg_send![item, submenu];
                if !submenu.is_null() {
                    walk(submenu, title, on);
                    continue;
                }
                let name: *mut Object = msg_send![item, title];
                if name.is_null() {
                    continue;
                }
                let utf8: *const c_char = msg_send![name, UTF8String];
                if utf8.is_null() {
                    continue;
                }
                if CStr::from_ptr(utf8).to_bytes() == title.as_bytes() {
                    // NSControlStateValueOn / Off.
                    let state: isize = if on { 1 } else { 0 };
                    let _: () = msg_send![item, setState: state];
                }
            }
        }
    }

    // SAFETY: called on the main thread of a process that runs an
    // NSApplication (`manage_check_marks` is only called by `main`).
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return;
        }
        let menu: *mut Object = msg_send![app, mainMenu];
        walk(menu, title, on);
    }
}

#[cfg(not(target_os = "macos"))]
fn set_native_state(_title: &str, _on: bool) {}
