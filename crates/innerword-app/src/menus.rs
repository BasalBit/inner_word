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
//! the palette and the list of shortcuts show ("Find Previous" got
//! `shift-enter`, a key of the find bar), and a key named `enter` reaches
//! AppKit as that word instead of the return key.
//!
//! So the keys the menu bar should show are bound once more, in
//! [`MENU_KEY_CONTEXT`]: a context no element of the app ever has, which
//! holds in GPUI's made-up one. These bindings are added before all others
//! (`crate::keymap` installs them first, whenever it installs the keymap),
//! so GPUI finds them first for the menu, they never run, and key hints
//! (which show the binding added last) never show them. They are computed
//! from the keymap that is installed ([`menu_key_bindings`]), so an item
//! shows the key the user gave its command, and none when the user took
//! the key away. A test holds every item's key to a key that really runs
//! its action. An item whose command has only keys with return or tab in
//! them shows no key (with the default keys there are three).
//!
//! The key beside an item is more than a label: AppKit runs an enabled
//! item when its key is pressed and GPUI has no binding for that key where
//! the keyboard is. A key the user took from a command must therefore
//! leave its menu item too, which is why the menu bar is built again after
//! every change of the keymap.

use std::rc::Rc;

use gpui::{Action, App, KeyBinding, KeyContext, Keymap, Menu, MenuItem, OsAction, SystemMenuType};
use innerword_assistant_ui as assistant;
use innerword_comments as comments;
use innerword_editor::actions as editor;
use innerword_editor_tools::actions as tools;
use innerword_spell::actions as spell;

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
    crate::zoom::add_menu_items(&mut menus);
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
            name: "Inner Word".into(),
            items: vec![
                MenuItem::action("About Inner Word", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::action("Agents…", ShowAgents),
                // What an agent changed last while the app was open: the
                // note about it has these two as buttons.
                MenuItem::action("Undo Last Change by an Agent", crate::live::UndoAgentChange),
                MenuItem::action(
                    "Go to Last Change by an Agent",
                    crate::live::ShowAgentChange,
                ),
                MenuItem::separator(),
                // Every key of the app, to look up and to change. "Reset
                // All" is here for the day a change went wrong: the menu
                // bar works whatever the keys do.
                MenuItem::action("Keyboard Shortcuts…", ShowShortcuts),
                MenuItem::action("Import Keyboard Shortcuts…", ImportShortcuts),
                MenuItem::action("Export Keyboard Shortcuts…", ExportShortcuts),
                MenuItem::action("Reset All Keyboard Shortcuts…", ResetAllShortcuts),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Inner Word", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Inner Word", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("New Document", NewDocument),
                MenuItem::action("New Folder", NewFolder),
                // Greyed out without a page engine (a whiteboard is shown by it).
                MenuItem::action("New Whiteboard", crate::actions::NewBoard),
                // Pages: greyed out without a page engine (Import) and while
                // nothing is being added (Cancel).
                MenuItem::action("Import Page…", crate::actions::ImportPage),
                MenuItem::action("Cancel Adding Pages", crate::actions::CancelAddingPages),
                // Also for whoever missed the summary's "Show Why": the
                // report of the last gesture (or that nothing was added).
                MenuItem::action(
                    "Show Report of Pages Added Last…",
                    crate::actions::ShowPageReport,
                ),
                MenuItem::separator(),
                MenuItem::action("Open Document…", QuickOpen),
                MenuItem::action("Search Library…", SearchLibrary),
                MenuItem::separator(),
                MenuItem::action("Save Now", SaveNow),
                MenuItem::action("Rename…", RenameDocument),
                MenuItem::action("Export…", Export),
                // Greyed out without a document (or a page), and the two
                // Cancel items except while the pages of a PDF export or
                // of a print are being prepared: each has a handler only
                // then (`pdf_export.rs`, `workspace/print.rs`).
                MenuItem::action("Export as PDF…", ExportPdf),
                MenuItem::action("Cancel PDF Export", CancelPdfExport),
                MenuItem::action("Print…", Print),
                MenuItem::action("Cancel Printing", CancelPrint),
                MenuItem::separator(),
                MenuItem::action("Version History…", ShowVersionHistory),
                MenuItem::action("Document Settings…", OpenDocumentSettings),
                MenuItem::action("Reveal in Library", RevealInLibrary),
                MenuItem::separator(),
                // An open page with the address it came from.
                MenuItem::action("Copy Page Address", crate::actions::CopyPageAddress),
                MenuItem::action(
                    "Open Page Address in Browser",
                    crate::actions::OpenPageAddress,
                ),
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
                // What there is to do with the selected image. "Larger"
                // and "Smaller" are the commands of the two lines below
                // (one key means larger for whatever is selected); the
                // widths and the description say what they need when no
                // image is selected.
                MenuItem::submenu(Menu {
                    name: "Image".into(),
                    items: vec![
                        MenuItem::action("Larger", editor::IncreaseTextSize),
                        MenuItem::action("Smaller", editor::DecreaseTextSize),
                        MenuItem::separator(),
                        MenuItem::action("25 %", editor::ImageWidth25),
                        MenuItem::action("50 %", editor::ImageWidth50),
                        MenuItem::action("75 %", editor::ImageWidth75),
                        MenuItem::action("100 %", editor::ImageWidth100),
                        MenuItem::action("Original Size", editor::ImageOriginalSize),
                        MenuItem::separator(),
                        MenuItem::action("Description…", tools::EditImageDescription),
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
                MenuItem::action("Image…", editor::InsertImage),
                MenuItem::action("Link…", tools::OpenLink),
                MenuItem::action("Comment", comments::AddComment),
                MenuItem::separator(),
                MenuItem::action("Insert Menu…", tools::OpenInsertMenu),
                MenuItem::separator(),
                // An open whiteboard: drawing without the pointer.
                MenuItem::submenu(Menu {
                    name: "Whiteboard".into(),
                    items: vec![
                        MenuItem::action("Text", crate::workspace::AddText),
                        MenuItem::action("Rectangle", crate::workspace::AddRectangle),
                        MenuItem::action("Ellipse", crate::workspace::AddEllipse),
                        MenuItem::action("Diamond", crate::workspace::AddDiamond),
                        MenuItem::action("Arrow", crate::workspace::AddArrow),
                        MenuItem::action("Frame", crate::workspace::AddFrame),
                        MenuItem::separator(),
                        MenuItem::action("From Mermaid…", crate::workspace::MermaidToDrawing),
                        MenuItem::action(
                            "From Shape Library…",
                            crate::workspace::InsertFromShapeLibrary,
                        ),
                        MenuItem::separator(),
                        MenuItem::action("Shape Library", crate::workspace::ToggleShapeLibrary),
                        MenuItem::action(
                            "Add Selection to Shape Library",
                            crate::workspace::AddToShapeLibrary,
                        ),
                    ],
                }),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                // An open page.
                MenuItem::action("Reload Page", crate::actions::ReloadPage),
                MenuItem::separator(),
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
                MenuItem::action("Keyboard Shortcuts…", ShowShortcuts),
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
/// for their action (see the [module docs](self)), computed from `keymap`:
/// the bindings that run, the defaults with the user's changes. They never
/// run themselves. Add them before every other binding.
///
/// An item shows the key the list of keyboard shortcuts shows for its
/// command ([`shown_binding`]). Where GPUI's own rule
/// ([`native_menu_binding`]) already picks that key, nothing is added;
/// where it picks another one, the right key is bound once more in
/// [`MENU_KEY_CONTEXT`]. And where the command has keys but none the menu
/// bar can draw ([`menu_can_show`]: a key with return or tab in it, which
/// GPUI hands AppKit as the word), a sequence of two keys is bound, which
/// gets no key in the menu at all: better than a wrong one. With the
/// default keys that is "Page Break", "Line Break" and "Tick or Untick
/// To-do". A command the user left without a key has no binding and shows
/// none.
///
/// While a whiteboard is the open item (`open`), no item shows a key that
/// is the whiteboard's own (`innerword_board_web::keys::WHITEBOARD_KEYS`):
/// such an item gets the two-key placeholder, so AppKit offers none of those
/// chords to the menu bar and the whiteboard has them (section 4.7). The
/// Edit menu's Undo, Redo, Cut, Copy, Paste and Select All keep theirs
/// ([`keeps_its_key_on_a_board`]): the page is asked first and Excalidraw
/// handles undo, redo and select all on the canvas, while copy and paste
/// reach it only as the `copy:` and `paste:` the menu items send (it has no
/// key handler for them), and a text field of the page needs `selectAll:`.
pub fn menu_key_bindings(keymap: &Keymap, open: OpenItemKind) -> Vec<KeyBinding> {
    let context: Rc<gpui::KeyBindingContextPredicate> = Rc::new(
        gpui::KeyBindingContextPredicate::parse(MENU_KEY_CONTEXT).expect("a valid key context"),
    );
    let the_boards = |binding: &KeyBinding| {
        open == OpenItemKind::Board
            && !keeps_its_key_on_a_board(binding.action())
            && crate::boards::is_whiteboard_keys(&crate::keymap::keys_of(binding))
    };
    let mut bindings = Vec::new();
    let mut seen: Vec<Box<dyn Action>> = Vec::new();
    for (_, action) in menu_actions(&app_menus()) {
        // "Link…" and "Keyboard Shortcuts…" are in two menus.
        if seen.iter().any(|seen| seen.partial_eq(action.as_ref())) {
            continue;
        }
        let Some(native) = native_menu_binding(keymap, action.as_ref()) else {
            // No key at all: the item shows none.
            seen.push(action);
            continue;
        };
        let keys = match shown_binding(keymap, action.as_ref()) {
            // A whiteboard's own key: the item shows none while one is open.
            _ if the_boards(native) => {
                let key = crate::keymap::keys_of(native);
                Some(format!("{key} {key}"))
            }
            Some(shown) if the_boards(shown) => {
                let key = crate::keymap::keys_of(native);
                Some(format!("{key} {key}"))
            }
            // GPUI finds the right key by itself.
            Some(shown) if shown.keystrokes() == native.keystrokes() => None,
            Some(shown) => Some(crate::keymap::keys_of(shown)),
            // Nothing the menu bar can draw, and GPUI would draw it.
            None if native.keystrokes().len() == 1 => {
                let key = crate::keymap::keys_of(native);
                Some(format!("{key} {key}"))
            }
            None => None,
        };
        if let Some(binding) = keys.and_then(|keys| {
            crate::keymap::bind(&keys, action.as_ref(), Some(context.clone()), None, None)
        }) {
            bindings.push(binding);
        }
        seen.push(action);
    }
    bindings
}

/// Whether the menu bar item of `action` keeps its key while a whiteboard is
/// open although the key is the whiteboard's: the six items of the Edit menu
/// that AppKit offers a key to only after the page left it (see
/// [`menu_key_bindings`]).
pub fn keeps_its_key_on_a_board(action: &dyn Action) -> bool {
    let kept: [&dyn Action; 6] = [
        &editor::Undo,
        &editor::Redo,
        &editor::Cut,
        &editor::Copy,
        &editor::Paste,
        &editor::SelectAll,
    ];
    kept.iter().any(|kept| kept.partial_eq(action))
}

/// Whether `binding` exists only to give a menu bar item its key.
pub fn is_menu_key_binding(binding: &KeyBinding) -> bool {
    binding
        .predicate()
        .is_some_and(|predicate| predicate.to_string() == MENU_KEY_CONTEXT)
}

/// Whether the menu bar can draw the key of `binding`: one keystroke
/// whose key is a character or one of the keys GPUI knows AppKit's
/// character for (`key_to_native` in GPUI's `platform/mac/events.rs`).
/// Return and tab are not among them.
pub fn menu_can_show(binding: &KeyBinding) -> bool {
    let [keystroke] = binding.keystrokes() else {
        return false;
    };
    let key = keystroke.key();
    key.chars().count() == 1
        || matches!(
            key,
            "space"
                | "backspace"
                | "escape"
                | "up"
                | "down"
                | "left"
                | "right"
                | "pageup"
                | "pagedown"
                | "home"
                | "end"
                | "delete"
                | "insert"
        )
        || key
            .strip_prefix('f')
            .and_then(|number| number.parse::<u8>().ok())
            .is_some_and(|number| (1..=35).contains(&number))
}

/// The binding whose key the menu bar item of `action` is to show: among
/// the bindings of the place the list of keyboard shortcuts names the
/// command under (the widest it is bound in), the one the app prefers (the
/// one bound last) that the menu bar can draw. "Indent" is `tab` in the
/// list and `cmd-]` here.
pub fn shown_binding<'a>(keymap: &'a Keymap, action: &'a dyn Action) -> Option<&'a KeyBinding> {
    use crate::shortcuts::Place;
    let place_of = |binding: &KeyBinding| {
        let predicate = binding.predicate().map(|predicate| predicate.to_string());
        Place::of_predicate(predicate.as_deref())
    };
    let real = || {
        keymap
            .bindings_for_action(action)
            .filter(|binding| !is_menu_key_binding(binding))
    };
    let widest = real().map(place_of).min()?;
    real()
        .rev()
        .filter(|binding| place_of(binding) == widest)
        .find(|binding| menu_can_show(binding))
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

// ----- what is open ----------------------------------------------------------------

/// What the open item is, as far as the menu bar cares: a whiteboard keeps
/// its own keys out of it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpenItemKind {
    /// A text, a page or nothing.
    #[default]
    Other,
    Board,
}

#[derive(Default)]
struct OpenItem(OpenItemKind);
impl gpui::Global for OpenItem {}

/// What the open item is, for the menu bar.
pub fn open_item(cx: &App) -> OpenItemKind {
    cx.try_global::<OpenItem>()
        .map(|open| open.0)
        .unwrap_or_default()
}

/// The open item became `kind`: when that changes whether a whiteboard is
/// open, the keymap's menu bindings and the menu bar are built again (1 to 5
/// ms, the cost of a change of a key).
pub fn set_open_item(kind: OpenItemKind, cx: &mut App) {
    if open_item(cx) == kind {
        return;
    }
    cx.set_global(OpenItem(kind));
    crate::keymap::reinstall(cx);
    install(cx);
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

/// The key AppKit has beside the menu bar item titled `title` (submenus
/// included), in GPUI's spelling (`cmd-alt-b`): `Some("")` for an item
/// without a key, `None` when the menu bar has no such item. Read from
/// the native menu itself, so it says what the user sees and what AppKit
/// answers to. For scripted runs (the `menukey:` step); only call it in a
/// process that runs an `NSApplication`, on the main thread.
#[cfg(target_os = "macos")]
// objc 0.2's msg_send! expands to a `cfg(feature = "cargo-clippy")` check.
#[allow(unexpected_cfgs)]
pub fn native_key_of(title: &str) -> Option<String> {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    use std::{ffi::CStr, os::raw::c_char};

    /// SAFETY: `string` is a live `NSString` (or nil).
    unsafe fn text(string: *mut Object) -> Option<String> {
        if string.is_null() {
            return None;
        }
        unsafe {
            let utf8: *const c_char = msg_send![string, UTF8String];
            (!utf8.is_null()).then(|| CStr::from_ptr(utf8).to_string_lossy().into_owned())
        }
    }

    /// SAFETY: `menu` is a live `NSMenu` (or nil); only AppKit accessors
    /// are called, on the main thread, and nothing is kept.
    unsafe fn walk(menu: *mut Object, title: &str) -> Option<String> {
        if menu.is_null() {
            return None;
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
                    if let Some(found) = walk(submenu, title) {
                        return Some(found);
                    }
                    continue;
                }
                let name: *mut Object = msg_send![item, title];
                if text(name).as_deref() != Some(title) {
                    continue;
                }
                let key: *mut Object = msg_send![item, keyEquivalent];
                let key = text(key).unwrap_or_default();
                if key.is_empty() {
                    return Some(String::new());
                }
                // NSEventModifierFlags: shift, control, option, command.
                let mask: usize = msg_send![item, keyEquivalentModifierMask];
                let mut out = String::new();
                for (bit, name) in [(18, "ctrl-"), (20, "cmd-"), (19, "alt-"), (17, "shift-")] {
                    if mask & (1 << bit) != 0 {
                        out.push_str(name);
                    }
                }
                // A key that is not a character is one of AppKit's
                // function-key code points (NSUpArrowFunctionKey = 0xF700,
                // NSF1FunctionKey = 0xF704).
                match key.chars().next().map(|ch| (ch, ch as u32)) {
                    Some((_, 0xF700)) => out.push_str("up"),
                    Some((_, 0xF701)) => out.push_str("down"),
                    Some((_, 0xF702)) => out.push_str("left"),
                    Some((_, 0xF703)) => out.push_str("right"),
                    Some((_, code @ 0xF704..=0xF726)) => {
                        out.push_str(&format!("f{}", code - 0xF703))
                    }
                    Some((ch, code)) if ch.is_control() || code >= 0xF700 => {
                        out.push_str(&format!("U+{code:04X}"))
                    }
                    Some(_) => out.push_str(&key),
                    None => {}
                }
                return Some(out);
            }
            None
        }
    }

    // SAFETY: called on the main thread of a process that runs an
    // NSApplication (a scripted run of the app).
    unsafe {
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return None;
        }
        let menu: *mut Object = msg_send![app, mainMenu];
        walk(menu, title)
    }
}

#[cfg(not(target_os = "macos"))]
pub fn native_key_of(_title: &str) -> Option<String> {
    None
}
