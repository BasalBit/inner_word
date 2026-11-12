//! The app's own context menus, and what the app adds to the menus of the
//! other crates.
//!
//! Everything the keyboard can do is also on a menu for the mouse:
//!
//! | Where | How it opens | What it offers |
//! |---|---|---|
//! | The title above the text | right-click, or a click on the chevron beside it | rename, export, version history, document settings, reveal in the library |
//! | "Saved" in the status bar | click or right-click | save now, version history |
//! | The counts in the status bar | click or right-click | which counts are shown, the character limit |
//! | The page indicator | click turns page view off; right-click | page view, orientation, document settings |
//! | The character-limit indicator | click or right-click | the network to count against, off |
//! | The Vim mode chip | click or right-click (the Vim crate's own menu) | plus "Settings…" from here |
//! | The text | right-click or `ctrl-enter` (the editor tools' menu) | plus "Add Comment" and "Go to Comment" from here, and "Ask the Assistant" while the assistant is shown |
//! | A word marked as misspelled | the same menu | led by the suggestions, "Learn Spelling" and "Ignore Spelling" (the spell checker's rows, put first from here) |
//! | A selection made with the mouse | by itself (the editor tools' format bar) | plus an "Add Comment" button from here, and "Ask the Assistant" while the assistant is shown |
//! | A row of the library | right-click, `ctrl-enter` or its "more" button (the panel's menu) | plus "Export…" and "Version History…" from here |
//!
//! The menus are [`writer_ui::ContextMenu`]s. An item that is a command is
//! the command's action: the menu shows its key and choosing it does what
//! the key does. A menu never takes the keyboard.

use std::{cell::Cell, rc::Rc};

use gpui::{Bounds, Context, IntoElement, Pixels, Styled as _, Window, canvas};
use writer_comments as comments;
use writer_editor_tools::FormatBarButton;
use writer_export::SocialNetwork;
use writer_library::{DocumentId, LibraryItem, LibraryMenuTarget};
use writer_ui::{ContextMenu, ContextMenuItem, IconName, MenuAnchor, Toast};

use super::{SidePanel, Workspace};
use crate::{
    actions::*,
    export::ExportTarget,
    settings::{character_limit_name, keys},
    settings_view::SettingRow,
    welcome,
};

/// What the text menu and the format bar call the quick chat.
pub const ASK_THE_ASSISTANT: &str = "Ask the Assistant";

/// The menus of the workspace itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MenuKind {
    /// The open document: on its title above the text.
    Title,
    /// Saving: on the save state in the status bar.
    Save,
    /// What the status bar counts: on the word, character and reading
    /// time counts.
    Statistics,
    /// Page view: on the page indicator of the status bar.
    Page,
    /// The character limit: on its indicator in the status bar.
    Limit,
}

impl MenuKind {
    /// Every menu of the workspace.
    pub const ALL: [MenuKind; 5] = [
        MenuKind::Title,
        MenuKind::Save,
        MenuKind::Statistics,
        MenuKind::Page,
        MenuKind::Limit,
    ];

    /// The name of the menu in scripted runs (`menu:title`).
    pub fn name(self) -> &'static str {
        match self {
            MenuKind::Title => "title",
            MenuKind::Save => "save",
            MenuKind::Statistics => "statistics",
            MenuKind::Page => "page",
            MenuKind::Limit => "limit",
        }
    }

    /// The menu called `name` in a scripted run.
    pub fn parse(name: &str) -> Option<MenuKind> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The debug selector of the control the menu belongs to (tests click
    /// it).
    pub fn selector(self) -> &'static str {
        match self {
            MenuKind::Title => "title-menu",
            MenuKind::Save => "status-save",
            MenuKind::Statistics => "status-statistics",
            MenuKind::Page => "status-page",
            MenuKind::Limit => "status-limit",
        }
    }

    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|kind| *kind == self)
            .unwrap_or_default()
    }
}

/// Where the controls that have a menu were drawn in the last frame. A
/// menu opened from the keyboard, from a script or by a plain click drops
/// down from (or rises above) its control.
#[derive(Clone, Default)]
pub struct Anchors(Rc<[Cell<Option<Bounds<Pixels>>>; 5]>);

impl Anchors {
    /// Where the control of `kind` was drawn, if it was.
    pub fn get(&self, kind: MenuKind) -> Option<Bounds<Pixels>> {
        self.0[kind.index()].get()
    }

    /// Forgets every position (before a frame is built: a control that is
    /// not drawn in it has no place).
    pub(crate) fn clear(&self) {
        for cell in self.0.iter() {
            cell.set(None);
        }
    }

    /// An element that fills its (relatively positioned) parent and notes
    /// where that is, as the place of the control of `kind`. It takes no
    /// part in hit testing.
    pub(crate) fn track(&self, kind: MenuKind) -> impl IntoElement + use<> {
        let anchors = self.clone();
        canvas(
            move |bounds, _, _| anchors.0[kind.index()].set(Some(bounds)),
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }
}

impl Workspace {
    /// The items of the workspace menu `kind` as things are now. Empty
    /// when the menu has nothing to act on (no document is open).
    pub fn menu_items(&self, kind: MenuKind, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        if self.session.is_none() {
            return Vec::new();
        }
        let weak = cx.weak_entity();
        match kind {
            MenuKind::Title => vec![
                ContextMenuItem::action("Rename…", RenameDocument),
                ContextMenuItem::action("Export…", Export),
                ContextMenuItem::action("Version History…", ShowVersionHistory),
                ContextMenuItem::action("Document Settings…", OpenDocumentSettings),
                ContextMenuItem::separator(),
                ContextMenuItem::action("Reveal in Library", RevealInLibrary),
            ],
            MenuKind::Save => vec![
                ContextMenuItem::action("Save Now", SaveNow),
                ContextMenuItem::action("Version History…", ShowVersionHistory),
            ],
            MenuKind::Statistics => {
                let shown = |label: &'static str, key: &'static str, on: bool| {
                    let weak = weak.clone();
                    ContextMenuItem::new(label, move |_, cx| {
                        weak.update(cx, |this, cx| this.write_setting(key, &!on, cx))
                            .ok();
                    })
                    .checked(on)
                };
                vec![
                    ContextMenuItem::section("Shown in the status bar"),
                    shown("Word Count", keys::STATUS_WORDS, self.settings.status_words),
                    shown(
                        "Character Count",
                        keys::STATUS_CHARACTERS,
                        self.settings.status_characters,
                    ),
                    shown(
                        "Reading Time",
                        keys::STATUS_READING_TIME,
                        self.settings.status_reading_time,
                    ),
                    ContextMenuItem::separator(),
                    ContextMenuItem::submenu("Character Limit", self.limit_items(cx)),
                ]
            }
            MenuKind::Page => {
                let page_view = self.editor.read(cx).page_view();
                vec![
                    ContextMenuItem::action("Page View", TogglePageView).checked(page_view),
                    ContextMenuItem::action("Switch Portrait and Landscape", TogglePageOrientation),
                    ContextMenuItem::separator(),
                    ContextMenuItem::action("Document Settings…", OpenDocumentSettings),
                ]
            }
            MenuKind::Limit => {
                let mut items = vec![ContextMenuItem::section("Count the text against")];
                items.extend(self.limit_items(cx));
                items.push(ContextMenuItem::separator());
                items.push(ContextMenuItem::new("Settings…", move |window, cx| {
                    weak.update(cx, |this, cx| {
                        this.show_settings(Some(SettingRow::CharacterLimit), window, cx)
                    })
                    .ok();
                }));
                items
            }
        }
    }

    /// The choices of the character limit: off, and every network with its
    /// limit. The one in force is checked.
    fn limit_items(&self, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let current = self.settings.character_limit;
        let weak = cx.weak_entity();
        std::iter::once(None)
            .chain(SocialNetwork::ALL.into_iter().map(Some))
            .map(|choice: Option<SocialNetwork>| {
                let label = match choice {
                    None => "Off".to_string(),
                    Some(network) => format!("{} ({})", network.name(), network.limit()),
                };
                let weak = weak.clone();
                ContextMenuItem::new(label, move |_, cx| {
                    weak.update(cx, |this, cx| {
                        this.write_setting(keys::CHARACTER_LIMIT, character_limit_name(choice), cx)
                    })
                    .ok();
                })
                .checked(choice == current)
            })
            .collect()
    }

    /// Where the control of the workspace menu `kind` (the title, an item
    /// of the status bar) was drawn in the last frame; `None` when it was
    /// not drawn.
    pub fn menu_anchor(&self, kind: MenuKind) -> Option<Bounds<Pixels>> {
        self.anchors.get(kind)
    }

    /// Opens the workspace menu `kind`: at `anchor` (the pointer of a
    /// right-click), or without one at its control, where it was drawn
    /// last. Returns whether a menu opened (it does not when the menu has
    /// nothing to act on, or its control is not on screen).
    pub fn open_menu(
        &mut self,
        kind: MenuKind,
        anchor: Option<MenuAnchor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let items = self.menu_items(kind, cx);
        let anchor = anchor.or_else(|| self.anchors.get(kind).map(MenuAnchor::Below));
        let (false, Some(anchor)) = (items.is_empty(), anchor) else {
            return false;
        };
        ContextMenu::open(anchor, items, window, cx);
        true
    }

    /// Hands the other crates what the app adds to their menus. They
    /// cannot know about comments, exporting or version history
    /// themselves; each of them asks through its hook when a menu is about
    /// to open. Called once, from [`Workspace::new`].
    pub(super) fn extend_the_menus_of_other_crates(&mut self, cx: &mut Context<Self>) {
        // The text: comments. "Add comment" is offered whenever the text
        // menu is, and dimmed when there is nothing to anchor a comment to
        // (the comments panel knows: a selection, or the word at the
        // caret, in a document that can be changed).
        // And the assistant, while it is shown: "Ask the Assistant" opens
        // the quick chat on the selection (or at the caret, for a plain
        // question).
        let panel = self.comments.downgrade();
        let workspace = cx.weak_entity();
        self.tools.update(cx, |tools, _| {
            tools.set_text_menu_extension(move |request, _, cx| {
                let Some(panel) = panel.upgrade() else {
                    return Vec::new();
                };
                let can_comment = panel.read(cx).can_add_comment(cx);
                let mut items = vec![
                    ContextMenuItem::action("Add Comment", comments::AddComment)
                        .disabled(!can_comment),
                ];
                if let Some(thread) = request.comments.first().copied() {
                    items.push(ContextMenuItem::action(
                        "Go to Comment",
                        comments::GoToThread::new(thread),
                    ));
                }
                let assistant = workspace
                    .upgrade()
                    .is_some_and(|workspace| workspace.read(cx).assistant_shown());
                if assistant {
                    items.push(ContextMenuItem::action(ASK_THE_ASSISTANT, AskAssistant));
                }
                items
            });
        });

        // The text, on a word marked as misspelled: the suggestions, Learn
        // Spelling and Ignore Spelling lead the menu, as in every Mac app.
        // The checker composes them; with nothing marked under the pointer
        // (or the caret, for the keyboard) it has none.
        if let Some(spell) = &self.spell {
            let spell = spell.downgrade();
            self.tools.update(cx, |tools, _| {
                tools.set_text_menu_leading_extension(move |request, _, cx| {
                    spell
                        .update(cx, |spell, cx| spell.context_menu_items(request, cx))
                        .unwrap_or_default()
                });
            });
        }

        // The format bar over a selection made with the mouse: the same
        // "Add Comment", as a button between "turn into" and "more". The
        // composer then takes the keyboard and the bar goes by itself.
        // Beside it, while the assistant is shown: "Ask the Assistant",
        // which opens the quick chat on that selection.
        let panel = self.comments.downgrade();
        let workspace = cx.weak_entity();
        self.tools.update(cx, |tools, _| {
            tools.set_format_bar_extension(move |_, cx| {
                let Some(panel) = panel.upgrade() else {
                    return Vec::new();
                };
                let mut buttons = vec![
                    FormatBarButton::action("Add Comment", comments::AddComment)
                        .icon(IconName::Comment)
                        .disabled(!panel.read(cx).can_add_comment(cx)),
                ];
                let assistant = workspace
                    .upgrade()
                    .is_some_and(|workspace| workspace.read(cx).assistant_shown());
                if assistant {
                    buttons.push(
                        FormatBarButton::action(ASK_THE_ASSISTANT, AskAssistant)
                            .icon(IconName::Sparkle),
                    );
                }
                buttons
            });
        });

        // A row of the library: export and version history.
        let weak = cx.weak_entity();
        self.panel.update(cx, |panel, _| {
            panel.set_menu_extension(move |target, _, _| match target {
                LibraryMenuTarget::Document(id) => {
                    let (export, history) = (weak.clone(), weak.clone());
                    vec![
                        ContextMenuItem::new("Export…", move |window, cx| {
                            export
                                .update(cx, |this, cx| {
                                    this.show_export(ExportTarget::Document(id), window, cx)
                                })
                                .ok();
                        }),
                        ContextMenuItem::new("Version History…", move |window, cx| {
                            history
                                .update(cx, |this, cx| this.show_version_history_of(id, window, cx))
                                .ok();
                        }),
                    ]
                }
                LibraryMenuTarget::Folder(id) => {
                    let export = weak.clone();
                    vec![ContextMenuItem::new(
                        "Export Folder…",
                        move |window, cx| {
                            export
                                .update(cx, |this, cx| {
                                    this.show_export(ExportTarget::Folder(id), window, cx)
                                })
                                .ok();
                        },
                    )]
                }
                _ => Vec::new(),
            });
        });
    }

    // ----- the commands behind the menus -------------------------------------------

    /// Saves the open document now and says so. The text is saved as it is
    /// written anyway; this is for the moment someone wants to be sure.
    pub fn save_now_and_say_so(&mut self, cx: &mut Context<Self>) {
        if self.session.is_none() {
            return;
        }
        if self.flush(cx) {
            self.show_toast(
                Toast::new("Saved")
                    .success()
                    .duration(std::time::Duration::from_millis(1500)),
                cx,
            );
        }
        // A failed save has said so itself, and is retried.
    }

    /// Shows the open document's row in the library and gives the library
    /// the keyboard.
    pub fn reveal_in_library(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_document_id() else {
            self.show_toast(Toast::new("No document is open"), cx);
            return;
        };
        self.reveal_document_row(id, window, cx);
    }

    fn reveal_document_row(&mut self, id: DocumentId, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_modal(window, cx);
        if !self.settings.sidebar_visible {
            self.set_sidebar_visible(true, window, cx);
        }
        self.last_side_panel = SidePanel::Library;
        let panel = self.panel.clone();
        panel.update(cx, |panel, cx| {
            panel.show_tree(cx);
            panel.reveal_document(id, cx);
        });
        window.focus(&gpui::Focusable::focus_handle(panel.read(cx), cx));
        cx.notify();
    }

    /// Starts renaming the open document: its row in the library becomes a
    /// field with the name selected.
    pub fn rename_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_document_id() else {
            self.show_toast(Toast::new("Open a document to rename it"), cx);
            return;
        };
        self.reveal_document_row(id, window, cx);
        self.panel.update(cx, |panel, cx| {
            panel.begin_rename(LibraryItem::Document(id), window, cx);
        });
    }

    /// Opens the welcome document. When it is no longer in the library
    /// (renamed, rewritten or trashed) a fresh one is written.
    pub fn open_welcome_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let existing = {
            let library = self.library.read(cx);
            let mut found = None;
            let tree = library.tree();
            let mut folders: Vec<&writer_library::FolderNode> = tree.folders.iter().collect();
            let mut documents: Vec<&writer_library::DocumentMeta> = tree.documents.iter().collect();
            while let Some(folder) = folders.pop() {
                documents.extend(folder.documents.iter());
                folders.extend(folder.folders.iter());
            }
            for meta in documents {
                if meta.title == welcome::WELCOME_TITLE {
                    found = Some(meta.id);
                    break;
                }
            }
            found
        };
        let id = match existing {
            Some(id) => id,
            None => {
                let created = self.library.update(cx, |library, cx| {
                    welcome::create_welcome_document(library, cx)
                });
                match created {
                    Ok(meta) => meta.id,
                    Err(error) => {
                        self.toast_error(
                            format!("Could not write the welcome document: {error}"),
                            cx,
                        );
                        return;
                    }
                }
            }
        };
        self.open_document(id, true, window, cx);
    }

    /// Opens document `id` (when it is not the open one) and its version
    /// history: restoring a version puts it on screen, so the document has
    /// to be the open one.
    pub fn show_version_history_of(
        &mut self,
        id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.open_document(id, false, window, cx) {
            return;
        }
        if !self
            .modal_layer
            .read(cx)
            .is_open::<crate::version_history::VersionHistory>()
        {
            self.toggle_version_history(window, cx);
        }
    }

    // ----- action handlers ----------------------------------------------------------

    pub(crate) fn on_save_now(&mut self, _: &SaveNow, _: &mut Window, cx: &mut Context<Self>) {
        self.save_now_and_say_so(cx);
    }

    pub(crate) fn on_rename_document(
        &mut self,
        _: &RenameDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rename_document(window, cx);
    }

    pub(crate) fn on_reveal_in_library(
        &mut self,
        _: &RevealInLibrary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal_in_library(window, cx);
    }

    pub(crate) fn on_open_welcome(
        &mut self,
        _: &OpenWelcomeDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_welcome_document(window, cx);
    }
}
