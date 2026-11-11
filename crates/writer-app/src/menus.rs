//! The native menu bar. Small, and by the book: app menu, File, Edit, View,
//! Window, Help. The shortcuts shown beside the items come from the keymap.
//!
//! The product's "few menus" rule is about chrome inside the window; the
//! system menu bar is where macOS users look for About, Settings and Quit.

use gpui::{App, Menu, MenuItem, OsAction, SystemMenuType};
use writer_editor::actions as editor;
use writer_editor_tools::actions as tools;

use crate::actions::*;

/// The menus of the app.
pub fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "The Writer".into(),
            items: vec![
                MenuItem::action("About The Writer", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
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
                MenuItem::action("Export…", Export),
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
                MenuItem::action("Add Comment", writer_comments::AddComment),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Command Palette…", ToggleCommandPalette),
                MenuItem::separator(),
                MenuItem::action("Show or Hide Library", ToggleSidebar),
                MenuItem::action("Page View", TogglePageView),
                MenuItem::action("Show or Hide Comments", writer_comments::ToggleComments),
                MenuItem::action("Vim Mode", ToggleVimMode),
                MenuItem::separator(),
                MenuItem::action("Focus Library", FocusLibrary),
                MenuItem::action("Focus Editor", FocusEditor),
                MenuItem::separator(),
                MenuItem::action("Enter or Leave Full Screen", ToggleFullScreen),
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
            items: vec![MenuItem::action("Keyboard Shortcuts", ShowShortcuts)],
        },
    ]
}

/// Installs the menu bar. Call after the key bindings are in place: the
/// shortcuts in the menus are read from the keymap at this moment.
pub fn install(cx: &mut App) {
    cx.set_menus(app_menus());
}
