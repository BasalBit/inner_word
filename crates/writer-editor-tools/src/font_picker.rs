//! The font picker (`cmd-t`): every installed font family, fuzzy filtered.
//!
//! "Document default" comes first, then the installed ones of a short
//! list of faces made for long text ([`WRITING_FONTS`]), the recently used
//! families, and all families. `enter` applies the family to the selection, or to the
//! text typed next when the caret is collapsed; `cmd-enter` makes it the
//! document's body font.
//!
//! The family list and the recently used families live in the
//! [`FontLibrary`] global, shared by every editor. The list is read from
//! the text system once, on first use.

use std::sync::Arc;

use gpui::{
    App, AppContext, Context, Entity, Focusable, Global, Pixels, Point, SharedString, Subscription,
    Window, px,
};
use writer_core::{Choice, Typography};
use writer_editor::PopupAnchor;
use writer_ui::{ContextMenu, ContextMenuItem, KeyHint, fuzzy};

use crate::{
    menu::{Mark, Menu, MenuEntry, MenuItem, MenuOptions},
    tools::{EditorTools, PopupTool},
};

/// How many recently used families are remembered.
pub const RECENT_FONT_LIMIT: usize = 6;

const DEFAULT_LABEL: &str = "Document default";

/// Faces a writer is likely to want, in the order the font picker leads
/// with them: serifs made for long reading, two quiet sans-serifs and a
/// monospace. Only the ones that are installed are listed. Without this
/// group the picker opens on whatever sorts first on the machine (symbol
/// and display fonts), and every useful face has to be typed.
pub const WRITING_FONTS: [&str; 9] = [
    "Charter",
    "Iowan Old Style",
    "Georgia",
    "Baskerville",
    "Palatino",
    "New York",
    "Avenir Next",
    "Helvetica Neue",
    "Menlo",
];

/// The installed font families and the recently used ones. A GPUI global:
/// observe it (`cx.observe_global::<FontLibrary>(..)`) to persist
/// [`recent_fonts`] when they change.
#[derive(Default)]
pub struct FontLibrary {
    families: Option<Arc<[SharedString]>>,
    recent: Vec<SharedString>,
}

impl Global for FontLibrary {}

/// Every installed font family, sorted, without the hidden system families
/// (names starting with a dot). Read from the text system on first use and
/// cached; replace the list with [`set_font_families`].
pub fn font_families(cx: &mut App) -> Arc<[SharedString]> {
    if let Some(families) = cx
        .try_global::<FontLibrary>()
        .and_then(|library| library.families.clone())
    {
        return families;
    }
    let mut names: Vec<String> = cx
        .text_system()
        .all_font_names()
        .into_iter()
        .filter(|name| !name.is_empty() && !name.starts_with('.'))
        .collect();
    names.sort_by_cached_key(|name| name.to_lowercase());
    names.dedup();
    let families: Arc<[SharedString]> = names.into_iter().map(SharedString::from).collect();
    cx.default_global::<FontLibrary>().families = Some(families.clone());
    families
}

/// Replaces the family list the font picker offers (for example after
/// fonts were installed, or in tests). Kept in the given order.
pub fn set_font_families<I, S>(families: I, cx: &mut App)
where
    I: IntoIterator<Item = S>,
    S: Into<SharedString>,
{
    let families: Arc<[SharedString]> = families.into_iter().map(Into::into).collect();
    cx.default_global::<FontLibrary>().families = Some(families);
}

/// The recently used families, most recent first.
pub fn recent_fonts(cx: &App) -> Vec<SharedString> {
    cx.try_global::<FontLibrary>()
        .map(|library| library.recent.clone())
        .unwrap_or_default()
}

/// Restores the recently used families (most recent first), for example
/// from the app's settings at startup.
pub fn set_recent_fonts<I, S>(recent: I, cx: &mut App)
where
    I: IntoIterator<Item = S>,
    S: Into<SharedString>,
{
    let mut list: Vec<SharedString> = Vec::new();
    for family in recent.into_iter().map(Into::into) {
        if !family.is_empty() && !list.contains(&family) {
            list.push(family);
        }
    }
    list.truncate(RECENT_FONT_LIMIT);
    cx.default_global::<FontLibrary>().recent = list;
}

fn remember_font(family: &SharedString, cx: &mut App) {
    let mut recent = recent_fonts(cx);
    if recent.first() == Some(family) {
        return;
    }
    recent.retain(|other| other != family);
    recent.insert(0, family.clone());
    recent.truncate(RECENT_FONT_LIMIT);
    // A mutable borrow of the global tells its observers.
    cx.default_global::<FontLibrary>().recent = recent;
}

pub(crate) struct FontMenu {
    pub(crate) menu: Entity<Menu>,
    /// Snapshot of the family list; row id `i + 1` is `families[i]`, row id
    /// 0 is "Document default".
    families: Arc<[SharedString]>,
    recent: Vec<SharedString>,
    /// The family of the selection: `Some(None)` is the document default,
    /// `None` is mixed.
    current: Option<Option<String>>,
    body_font: SharedString,
    _subscription: Subscription,
}

impl FontMenu {
    fn family_id(&self, family: &str) -> Option<usize> {
        self.families
            .iter()
            .position(|other| other.as_ref() == family)
            .map(|index| index + 1)
    }

    fn current_id(&self) -> Option<usize> {
        match self.current.as_ref()? {
            None => Some(0),
            Some(family) => self.family_id(family),
        }
    }

    fn item(&self, id: usize, positions: Vec<usize>) -> MenuEntry {
        let mark = if self.current_id() == Some(id) {
            Mark::Check
        } else {
            Mark::None
        };
        MenuEntry::Item(match id {
            0 => MenuItem {
                id,
                label: DEFAULT_LABEL.into(),
                positions,
                detail: Some(self.body_font.clone()),
                mark,
                more: true,
                ..MenuItem::default()
            },
            _ => {
                let family = self.families[id - 1].clone();
                MenuItem {
                    id,
                    label: family.clone(),
                    positions,
                    mark,
                    sample_font: Some(family),
                    more: true,
                    ..MenuItem::default()
                }
            }
        })
    }

    fn entries(&self, query: &str) -> Vec<MenuEntry> {
        if query.trim().is_empty() {
            let mut entries = Vec::with_capacity(
                self.families.len() + self.recent.len() + WRITING_FONTS.len() + 4,
            );
            entries.push(self.item(0, Vec::new()));
            let writing: Vec<usize> = WRITING_FONTS
                .iter()
                .filter_map(|family| self.family_id(family))
                .collect();
            if !writing.is_empty() {
                entries.push(MenuEntry::Header("Writing fonts".into()));
                entries.extend(writing.into_iter().map(|id| self.item(id, Vec::new())));
            }
            // A recent family that is a writing font is already in view.
            let recent: Vec<usize> = self
                .recent
                .iter()
                .filter(|family| !WRITING_FONTS.contains(&family.as_ref()))
                .filter_map(|family| self.family_id(family))
                .collect();
            if !recent.is_empty() {
                entries.push(MenuEntry::Header("Recent".into()));
                entries.extend(recent.into_iter().map(|id| self.item(id, Vec::new())));
            }
            if !self.families.is_empty() {
                entries.push(MenuEntry::Header("All fonts".into()));
                entries.extend((1..=self.families.len()).map(|id| self.item(id, Vec::new())));
            }
            return entries;
        }
        let candidates =
            std::iter::once(DEFAULT_LABEL).chain(self.families.iter().map(|name| name.as_ref()));
        fuzzy::match_strings(query, candidates, usize::MAX)
            .into_iter()
            .map(|found| self.item(found.candidate_index, found.positions))
            .collect()
    }
}

impl EditorTools {
    /// Opens the font picker at the caret or selection and moves focus into
    /// its filter field. Does nothing in a read-only editor.
    pub fn open_font_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        self.make_room(window, cx);
        let families = font_families(cx);
        let recent = recent_fonts(cx);
        let editor = self.editor.read(cx);
        let collapsed = editor.selection().is_collapsed();
        let current = match editor.state().format_state().font {
            Choice::Same(font) => Some(font),
            Choice::Mixed => None,
        };
        let body_font: SharedString = editor
            .document()
            .settings()
            .typography
            .body_font
            .clone()
            .into();
        let menu = cx.new(|cx| {
            Menu::new(
                MenuOptions {
                    placeholder: Some("Font".into()),
                    width: px(380.),
                    max_rows: 10,
                    marks: true,
                    empty_text: "No matching font".into(),
                    footer: [("enter", "Apply"), ("cmd-enter", "Set as document font")]
                        .into_iter()
                        .filter_map(|(keys, label)| Some((KeyHint::parse(keys)?, label.into())))
                        .collect(),
                },
                window,
                cx,
            )
        });
        let tool = FontMenu {
            menu: menu.clone(),
            families,
            recent,
            current,
            body_font,
            _subscription: cx.subscribe_in(&menu, window, Self::on_menu_event),
        };
        let entries = tool.entries("");
        let select = tool.current_id();
        menu.update(cx, |menu, cx| menu.set_entries(entries, select, cx));
        let anchor = if collapsed {
            PopupAnchor::Caret
        } else {
            PopupAnchor::Selection
        };
        window.focus(&menu.read(cx).focus_handle(cx));
        self.show_popup(PopupTool::Font(tool), menu.into(), anchor, cx);
    }

    /// `cmd-t`: opens the font picker, or closes it when it is open.
    pub(crate) fn toggle_font_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popup, Some(PopupTool::Font(_))) {
            self.close_popup(true, window, cx);
        } else {
            self.open_font_picker(window, cx);
        }
    }

    pub(crate) fn refresh_font_menu(&mut self, cx: &mut Context<Self>) {
        let Some(PopupTool::Font(tool)) = &self.popup else {
            return;
        };
        let query = tool.menu.read(cx).query(cx);
        let entries = tool.entries(&query);
        tool.menu
            .update(cx, |menu, cx| menu.set_entries(entries, None, cx));
    }

    /// `enter`: the family goes on the selection (or on the text typed
    /// next). `cmd-enter` (`as_body_font`): it becomes the document's body
    /// font.
    pub(crate) fn confirm_font_menu(
        &mut self,
        as_body_font: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PopupTool::Font(tool)) = &self.popup else {
            return;
        };
        let Some(id) = tool.menu.read(cx).selected_id() else {
            return;
        };
        self.confirm_font_row(id, as_body_font, window, cx);
    }

    /// The menu of a row of the font picker (a right-click on the row, or
    /// its "more" button): the two things `enter` and `cmd-enter` do with
    /// the family, so that both can be chosen with the mouse.
    pub(crate) fn open_font_row_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PopupTool::Font(tool)) = &self.popup else {
            return;
        };
        let Some(id) = tool.menu.read(cx).selected_id() else {
            return;
        };
        let collapsed = self.editor.read(cx).selection().is_collapsed();
        let this = cx.weak_entity();
        let item = |label: &'static str, as_body_font: bool, keys: &str| {
            let this = this.clone();
            ContextMenuItem::new(label, move |window, cx| {
                this.update(cx, |tools, cx| {
                    tools.confirm_font_row(id, as_body_font, window, cx)
                })
                .ok();
            })
            .key_hint(KeyHint::parse(keys))
        };
        ContextMenu::open(
            position,
            vec![
                item(
                    if collapsed {
                        "Use for the Text Typed Next"
                    } else {
                        "Apply to the Selection"
                    },
                    false,
                    "enter",
                ),
                item("Set as Document Font", true, "cmd-enter"),
            ],
            window,
            cx,
        );
    }

    /// Applies row `id` of the open font picker (0 is "Document
    /// default").
    fn confirm_font_row(
        &mut self,
        id: usize,
        as_body_font: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(PopupTool::Font(tool)) = &self.popup else {
            return;
        };
        if id > tool.families.len() {
            return;
        }
        let family = (id > 0).then(|| tool.families[id - 1].clone());
        self.close_popup(true, window, cx);
        self.editor.update(cx, |editor, cx| {
            if editor.is_read_only() {
                return;
            }
            if as_body_font {
                editor.edit(
                    |state| {
                        let mut settings = state.document().settings().clone();
                        settings.typography.body_font = match &family {
                            Some(family) => family.to_string(),
                            None => Typography::default().body_font,
                        };
                        state.set_settings(settings)
                    },
                    cx,
                );
            } else {
                editor.edit(
                    |state| state.set_font_family(family.as_ref().map(|family| family.as_ref())),
                    cx,
                );
            }
        });
        if let Some(family) = &family {
            remember_font(family, cx);
        }
    }
}
