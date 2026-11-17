//! The context menu of the text: what a right-click in the document (or
//! `ctrl-enter`) offers.
//!
//! The editor adjusts the selection and asks
//! (`EditorEvent::ContextMenuRequested`); this module composes the menu for
//! that request and runs the row that is chosen. Every row runs the code
//! its key runs: rows with an action dispatch that action where the
//! keyboard is in the text, the formatting rows are the rows of the format
//! menu (`cmd-.`), and each shows the key the registry has for it.
//!
//! The rows, in order, each group only where it applies:
//!
//! 1. on a link: Open Link, Edit Link, Remove Link, Copy Link Address (the
//!    whole link, whatever part of it is selected);
//! 2. Cut, Copy, Paste, Paste as Plain Text;
//! 3. with a selection: Bold, Italic, Underline, Strikethrough, Inline
//!    Code, Highlight and Text Colour as submenus, Link, Font, Clear
//!    Formatting;
//! 4. the block: Turn Into, Align and Text Size as submenus, and on a
//!    to-do item Mark as Done / Mark as Not Done;
//! 5. Insert as a submenu (divider, page break, image, date, link; on an
//!    image, a divider or a page break it begins with Paragraph Above and
//!    Paragraph Below, the only way to a line above one that is the first
//!    thing in a document);
//! 6. Find in Document, Select All;
//! 7. what the host adds ([`EditorTools::set_text_menu_extension`]).
//!
//! Before all of them, behind a separator, come the items the host puts
//! first ([`EditorTools::set_text_menu_leading_extension`]): the place of
//! spelling suggestions in every Mac app.
//!
//! **On a selected image** (the caret is on its block, which is where a
//! right-click on a picture puts it) the menu is the image's: groups 3 and
//! 4 make way for Size (Larger, Smaller, the presets 25 to 100 % and
//! Original Size, with a check mark on the one the image has), Align
//! (Left, Centre, Right) and Description…, and after everything else, the
//! host's items included, comes Delete Image. A preset the page does not
//! allow for this picture (it would be taller than a page) is dimmed. A
//! range selection that covers an image is text with a picture in it and
//! gets the menu of a selection.
//!
//! The menu itself is `innerword_ui::ContextMenu`, the app's one menu widget:
//! it never takes the keyboard, so the editor keeps it, an action row
//! arrives where its key would, and closing the menu needs no focus
//! restore.
//!
//! A row that cannot apply right now (Cut without a selection, anything
//! that changes a read-only document, bold in a code block) is shown
//! disabled rather than left out, so the menu keeps its shape.

use std::rc::Rc;

use gpui::{
    Action, App, ClipboardItem, Context, DismissEvent, Entity, Hsla, SharedString, Subscription,
    WeakEntity, Window,
};
use innerword_core::{Alignment, BlockKind, Choice, EditorState, Image, ImageWidth, TextRange};
use innerword_editor::{ContextMenuRequest, actions as editor};
use innerword_ui::{ContextMenu, ContextMenuItem, KeyHint, MenuAnchor};

use crate::{
    actions::{EditImageDescription, Find, InsertDate, OpenLink, RemoveLink},
    format_menu::{Effect, FormatItem, Section, format_items},
    insert_menu::takes_pictures,
    menu::Mark,
    tools::{EditorTools, ToolKind, ToolsEvent},
};

/// What the host adds to the text menu for a request.
pub(crate) type TextMenuExtension =
    Rc<dyn Fn(&ContextMenuRequest, &mut Window, &mut App) -> Vec<ContextMenuItem>>;

/// The text menu that is open.
pub(crate) struct OpenTextMenu {
    pub(crate) request: ContextMenuRequest,
    pub(crate) menu: Entity<ContextMenu>,
    _subscription: Subscription,
}

/// What choosing a row does, for the rows that are not a plain action.
#[derive(Clone)]
pub(crate) enum Run {
    /// A row of the format menu.
    Format(Effect),
    OpenUrl(String),
    EditLink(TextRange),
    RemoveLink(TextRange),
    CopyText(String),
}

enum What {
    /// Dispatched where the keyboard is in the text: the path of the
    /// action's key binding.
    Action(Rc<dyn Action>),
    Run(Run),
    Submenu(Vec<Row>),
    /// A line between two groups of a submenu.
    Separator,
}

/// A row while the menu is put together.
struct Row {
    label: SharedString,
    key_hint: Option<KeyHint>,
    /// `Some` for a row with a state: its check mark column.
    checked: Option<bool>,
    /// The colour a row stands for (a highlight, a text colour).
    swatch: Option<Hsla>,
    enabled: bool,
    /// The style of a row that deletes.
    danger: bool,
    what: What,
}

impl Row {
    fn new(label: impl Into<SharedString>, what: What) -> Self {
        Self {
            label: label.into(),
            key_hint: None,
            checked: None,
            swatch: None,
            enabled: true,
            danger: false,
            what,
        }
    }

    fn separator() -> Self {
        Self::new("", What::Separator).enabled(false)
    }

    fn run(label: &'static str, run: Run) -> Self {
        Self::new(label, What::Run(run))
    }

    /// A row that dispatches `action` and shows its key.
    fn action(label: &'static str, action: impl Action, cx: &App) -> Self {
        let key_hint = KeyHint::for_action(&action, cx);
        Self::new(label, What::Action(Rc::new(action))).key(key_hint)
    }

    /// A row of the format menu under its menu name. With `state`, it
    /// carries a check mark when it is on for the whole selection (a
    /// value that only part of the selection has is not checked).
    fn format(item: &FormatItem, state: bool) -> Self {
        let what = match &item.effect {
            Effect::Editor(action) => What::Action(action.clone()),
            effect => What::Run(Run::Format(effect.clone())),
        };
        let mut row = Self::new(item.short, what).key(item.key_hint.clone());
        row.swatch = item.swatch;
        if state {
            row.checked = Some(item.mark == Mark::Check);
        }
        row
    }

    /// A submenu. It is disabled when none of its rows can be chosen.
    fn submenu(label: &'static str, rows: Vec<Row>) -> Self {
        let enabled = rows.iter().any(|row| row.enabled);
        Self::new(label, What::Submenu(rows)).enabled(enabled)
    }

    fn key(mut self, hint: Option<KeyHint>) -> Self {
        self.key_hint = hint;
        self
    }

    fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    fn danger(mut self) -> Self {
        self.danger = true;
        self
    }

    fn into_item(self, tools: &WeakEntity<EditorTools>) -> ContextMenuItem {
        let item = match self.what {
            What::Action(action) => ContextMenuItem::action_boxed(self.label, action.boxed_clone()),
            What::Run(run) => {
                let tools = tools.clone();
                ContextMenuItem::new(self.label, move |window, cx| {
                    tools
                        .update(cx, |tools, cx| {
                            tools.run_text_menu_row(run.clone(), window, cx)
                        })
                        .ok();
                })
            }
            What::Submenu(rows) => ContextMenuItem::submenu(
                self.label,
                rows.into_iter().map(|row| row.into_item(tools)).collect(),
            ),
            What::Separator => return ContextMenuItem::separator(),
        };
        let item = item.key_hint(self.key_hint).disabled(!self.enabled);
        let item = if self.danger { item.danger() } else { item };
        let item = match self.swatch {
            Some(color) => item.swatch(color),
            None => item,
        };
        match self.checked {
            Some(checked) => item.checked(checked),
            None => item,
        }
    }
}

impl EditorTools {
    /// Lets the host add its own items to the text context menu.
    ///
    /// `extension` is called each time the menu is about to open, with the
    /// editor's request: where the pointer is, whether there is a
    /// selection, the link and the comment threads under the pointer. The
    /// items it returns are shown after the menu's own, behind a
    /// separator; put [`ContextMenuItem::separator`] between groups of
    /// your own. An empty list adds nothing. By the time it is called the
    /// selection is what the menu acts on and the editor has the keyboard;
    /// both are still so when an item runs, so an action item arrives
    /// exactly as its key would.
    ///
    /// The extension runs outside any update of this view: it may read
    /// and update whatever it likes, this view included.
    ///
    /// ```ignore
    /// tools.update(cx, |tools, _| {
    ///     tools.set_text_menu_extension(move |request, _window, _cx| {
    ///         let mut items = Vec::new();
    ///         if request.has_selection {
    ///             items.push(ContextMenuItem::action("Add Comment", comments::AddComment));
    ///         }
    ///         if let Some(thread) = request.comments.first().copied() {
    ///             let panel = panel.clone();
    ///             items.push(ContextMenuItem::new("Go to Comment", move |window, cx| {
    ///                 panel.update(cx, |panel, cx| panel.reveal(thread, window, cx))
    ///             }));
    ///         }
    ///         items
    ///     })
    /// });
    /// ```
    pub fn set_text_menu_extension(
        &mut self,
        extension: impl Fn(&ContextMenuRequest, &mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) {
        self.text_menu_extension = Some(Rc::new(extension));
    }

    /// Lets the host put items of its own at the very top of the text
    /// context menu: the place of what belongs to the word under the
    /// pointer before anything else (spelling suggestions, Learn Spelling,
    /// Ignore Spelling).
    ///
    /// `extension` is called each time the menu is about to open, under
    /// the same conditions as [`EditorTools::set_text_menu_extension`]
    /// (outside any update of this view, with the selection already what
    /// the menu acts on and the keyboard in the text). The items it
    /// returns are shown before the menu's own, followed by a separator;
    /// an empty list adds nothing and leaves the menu as it was. Opened
    /// from the keyboard, the first of them is the marked row.
    ///
    /// The format bar is not affected.
    pub fn set_text_menu_leading_extension(
        &mut self,
        extension: impl Fn(&ContextMenuRequest, &mut Window, &mut App) -> Vec<ContextMenuItem> + 'static,
    ) {
        self.text_menu_leading_extension = Some(Rc::new(extension));
    }

    /// Whether the text context menu is open.
    pub fn text_menu_open(&self) -> bool {
        self.text_menu.is_some()
    }

    /// The open text context menu (the shared widget: its items, bounds
    /// and selection), for tests and automation.
    pub fn text_menu(&self) -> Option<&Entity<ContextMenu>> {
        self.text_menu.as_ref().map(|open| &open.menu)
    }

    /// The request the open text context menu was composed for.
    pub fn text_menu_request(&self) -> Option<&ContextMenuRequest> {
        self.text_menu.as_ref().map(|open| &open.request)
    }

    /// Opens the text context menu for `request` (the editor's
    /// `EditorEvent::ContextMenuRequested` does this): this crate's items
    /// from [`EditorTools::text_menu_items`], then the host's (and before
    /// them what the host puts first; after them the row that ends the
    /// menu of an image, "Delete Image"). A
    /// right-click opens it at the pointer; `ctrl-enter` drops it down
    /// from the caret with the first item marked. An open popup tool
    /// closes; the find bar stays.
    ///
    /// The menu opens right after the current update (the host's
    /// extension runs outside it).
    pub fn open_text_menu(
        &mut self,
        request: ContextMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_popup(false, window, cx);
        // The menu is what the pointer asked for now.
        self.dismiss_format_bar(cx);
        let (mut items, last) = self.text_menu_parts(&request, cx);
        let extension = self.text_menu_extension.clone();
        let leading = self.text_menu_leading_extension.clone();
        let caret = self.editor.read(cx).caret_bounds();
        let anchor = match caret {
            Some(caret) if request.from_keyboard => MenuAnchor::Below(caret),
            _ => MenuAnchor::Point(request.position),
        };
        let this = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            if let Some(leading) = leading {
                let mut first = leading(&request, window, cx);
                if !first.is_empty() {
                    first.push(ContextMenuItem::separator());
                    first.append(&mut items);
                    items = first;
                }
            }
            if let Some(extension) = extension {
                let extra = extension(&request, window, cx);
                if !extra.is_empty() {
                    items.push(ContextMenuItem::separator());
                    items.extend(extra);
                }
            }
            // What ends the menu comes after the host's items too.
            if !last.is_empty() {
                items.push(ContextMenuItem::separator());
                items.extend(last);
            }
            let menu = ContextMenu::open(anchor, items, window, cx);
            if request.from_keyboard {
                menu.update(cx, |menu, cx| menu.select_first(cx));
            }
            this.update(cx, |this, cx| {
                let subscription = cx.subscribe(&menu, |this, menu, _: &DismissEvent, cx| {
                    if this
                        .text_menu
                        .as_ref()
                        .is_some_and(|open| open.menu == menu)
                    {
                        this.text_menu = None;
                        cx.emit(ToolsEvent::Closed(ToolKind::ContextMenu));
                        cx.notify();
                    }
                });
                this.text_menu = Some(OpenTextMenu {
                    request,
                    menu,
                    _subscription: subscription,
                });
                cx.emit(ToolsEvent::Opened(ToolKind::ContextMenu));
                cx.notify();
            })
            .ok();
        });
    }

    /// Closes the text context menu, if it is the menu open in the
    /// window. Focus is not touched (the menu never had it).
    pub(crate) fn close_text_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(open) = &self.text_menu else {
            return;
        };
        if ContextMenu::current(window, cx).is_some_and(|current| current == open.menu) {
            // The menu tells us (`DismissEvent`); the state follows there.
            ContextMenu::dismiss(window, cx);
        } else {
            self.text_menu = None;
            cx.notify();
        }
    }

    /// This crate's items of the text context menu for `request`, in
    /// order, groups separated by separators (without the host's items):
    /// the link under the pointer; cut, copy and paste; the formatting of
    /// the selection; the block; insert; find and select all. On a
    /// selected image: cut, copy and paste; size, alignment and
    /// description; insert; find and select all; delete. Items that cannot
    /// apply right now are disabled.
    ///
    /// `cx` is the app as it can be changed: whether Paste would put
    /// something in is asked of the editor ([`Editor::can_paste`]), which
    /// asks its image provider for a picture GPUI's clipboard does not
    /// show.
    ///
    /// [`Editor::can_paste`]: innerword_editor::Editor::can_paste
    pub fn text_menu_items(
        &self,
        request: &ContextMenuRequest,
        cx: &mut App,
    ) -> Vec<ContextMenuItem> {
        let (mut items, last) = self.text_menu_parts(request, cx);
        if !last.is_empty() {
            items.push(ContextMenuItem::separator());
            items.extend(last);
        }
        items
    }

    /// The items of [`EditorTools::text_menu_items`] in two parts: the
    /// menu, and what ends it. The host's items go between the two.
    fn text_menu_parts(
        &self,
        request: &ContextMenuRequest,
        cx: &mut App,
    ) -> (Vec<ContextMenuItem>, Vec<ContextMenuItem>) {
        // GPUI's clipboard shows nothing for a picture it cannot read (a
        // HEIC, a file copied in Finder) although Paste takes it.
        let can_paste = self.editor.update(cx, |editor, cx| editor.can_paste(cx));
        let (sections, last) = self.text_menu_sections(request, can_paste, cx);
        let mut items = Vec::new();
        for section in sections {
            if !items.is_empty() {
                items.push(ContextMenuItem::separator());
            }
            items.extend(section.into_iter().map(|row| row.into_item(&self.this)));
        }
        let last = last
            .into_iter()
            .map(|row| row.into_item(&self.this))
            .collect();
        (items, last)
    }

    /// The groups of the menu, and the rows that end it.
    fn text_menu_sections(
        &self,
        request: &ContextMenuRequest,
        can_paste: bool,
        cx: &App,
    ) -> (Vec<Vec<Row>>, Vec<Row>) {
        let editor = self.editor.read(cx);
        let state = editor.state();
        let document = editor.document();
        let editable = !editor.is_read_only();
        let has_selection = !editor.selection().is_collapsed();
        let block_kind = document
            .block(request.block)
            .map(|block| block.kind().clone());
        let on_void = block_kind.as_ref().is_some_and(BlockKind::is_void);
        // The image the menu is for: the selected one, when the request is
        // about its block.
        let image = state
            .selected_image()
            .filter(|(index, _)| *index == request.block)
            .map(|(index, image)| (index, image.clone()));
        let mut sections: Vec<Vec<Row>> = Vec::new();
        let mut last = Vec::new();

        // 1. The link under the pointer.
        if let Some(url) = &request.link {
            let range = request.link_range.or_else(|| {
                document
                    .link_at(request.document_position)
                    .map(|link| link.range)
            });
            let mut rows = vec![Row::run("Open Link", Run::OpenUrl(url.clone()))];
            if let Some(range) = range {
                rows.push(
                    Row::run("Edit Link", Run::EditLink(range))
                        .key(KeyHint::for_action(&OpenLink, cx))
                        .enabled(editable),
                );
                rows.push(
                    Row::run("Remove Link", Run::RemoveLink(range))
                        .key(KeyHint::for_action(&RemoveLink, cx))
                        .enabled(editable),
                );
            }
            rows.push(Row::run("Copy Link Address", Run::CopyText(url.clone())));
            sections.push(rows);
        }

        // 2. The clipboard. A selected image is something to copy: the
        // picture itself.
        let can_copy = state.has_something_to_copy();
        let can_paste_text = editable && cx.read_from_clipboard().is_some();
        sections.push(vec![
            Row::action("Cut", editor::Cut, cx).enabled(editable && can_copy),
            Row::action("Copy", editor::Copy, cx).enabled(can_copy),
            Row::action("Paste", editor::Paste, cx).enabled(editable && can_paste),
            Row::action("Paste as Plain Text", editor::PasteAsPlainText, cx)
                .enabled(can_paste_text),
        ]);

        match &image {
            // 3 and 4 for an image: its size, its place, what it shows.
            Some((_, image)) => {
                let alignment = document
                    .block(request.block)
                    .map_or(Alignment::Left, |block| block.alignment());
                sections.push(vec![
                    Row::submenu("Size", image_size_rows(state, image, editable, cx)),
                    Row::submenu("Align", image_align_rows(alignment, editable, cx)),
                    Row::action("Description…", EditImageDescription, cx).enabled(editable),
                ]);
                last.push(
                    Row::action("Delete Image", editor::Backspace, cx)
                        .enabled(editable)
                        .danger(),
                );
            }
            None => {
                let format = state.format_state();
                let items = format_items(state, cx);
                // A code block's text carries no inline formatting.
                let all_code = matches!(&format.kind, Choice::Same(kind) if kind.is_code());
                let section =
                    |section: Section| items.iter().filter(move |item| item.section == section);
                let format_rows = |wanted: Section, state: bool, enabled: bool| -> Vec<Row> {
                    section(wanted)
                        .map(|item| Row::format(item, state).enabled(enabled))
                        .collect()
                };

                // 3. The selected text.
                if has_selection {
                    let inline = editable && !all_code;
                    let mut rows = Vec::new();
                    let mut tail = Vec::new();
                    for item in section(Section::Text) {
                        match &item.effect {
                            // "Edit Link" above is this row for a link.
                            Effect::Link if request.link.is_some() => {}
                            Effect::Link | Effect::Font => {
                                tail.push(Row::format(item, false).enabled(inline))
                            }
                            _ if item.short == "Clear Formatting" => {
                                tail.push(Row::format(item, false).enabled(inline))
                            }
                            _ => rows.push(Row::format(item, true).enabled(inline)),
                        }
                    }
                    let mut highlights = format_rows(Section::Highlight, true, inline);
                    highlights.push(
                        Row::run("None", Run::Format(Effect::NoHighlight))
                            .checked(matches!(&format.highlight, Choice::Same(None)))
                            .enabled(inline),
                    );
                    rows.push(Row::submenu("Highlight", highlights));
                    rows.push(Row::submenu(
                        "Text Colour",
                        format_rows(Section::Color, true, inline),
                    ));
                    rows.extend(tail);
                    sections.push(rows);
                }

                // 4. The block.
                let block = editable && !on_void;
                let mut rows = vec![
                    Row::submenu("Turn Into", format_rows(Section::TurnInto, true, block)),
                    Row::submenu("Align", format_rows(Section::Align, true, block)),
                    Row::submenu("Text Size", format_rows(Section::Size, false, block)),
                ];
                if let Some(BlockKind::TodoItem { checked }) = block_kind {
                    rows.push(
                        Row::action(
                            if checked {
                                "Mark as Not Done"
                            } else {
                                "Mark as Done"
                            },
                            editor::ToggleTodoChecked,
                            cx,
                        )
                        .enabled(editable),
                    );
                }
                sections.push(rows);
            }
        }

        // 5. Insert.
        let mut insert = Vec::new();
        // With the caret on a block that holds no text, a new line and a
        // line break mean a paragraph below and a paragraph above it.
        let caret_on_void = !has_selection
            && document
                .block(editor.selection().head.block)
                .is_some_and(|block| block.is_void());
        if on_void && caret_on_void {
            insert.extend([
                Row::action("Paragraph Above", editor::InsertSoftBreak, cx).enabled(editable),
                Row::action("Paragraph Below", editor::Newline, cx).enabled(editable),
                Row::separator(),
            ]);
        }
        // A picture comes in where the editor has somebody to store it.
        let pictures = takes_pictures(editor);
        insert.extend([
            Row::action("Divider", editor::InsertDivider, cx).enabled(editable),
            Row::action("Page Break", editor::InsertPageBreak, cx).enabled(editable),
            Row::action("Image…", editor::InsertImage, cx).enabled(pictures),
            Row::action("Today's Date", InsertDate, cx).enabled(editable),
            Row::action("Link", OpenLink, cx).enabled(editable),
        ]);
        sections.push(vec![Row::submenu("Insert", insert)]);

        // 6. The document.
        sections.push(vec![
            Row::action("Find in Document", Find, cx),
            Row::action("Select All", editor::SelectAll, cx),
        ]);

        (sections, last)
    }

    /// Does what a chosen row stands for. The menu is closed and the
    /// keyboard is in the text by now.
    pub(crate) fn run_text_menu_row(
        &mut self,
        run: Run,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match run {
            Run::Format(effect) => self.apply_format_effect(effect, window, cx),
            Run::OpenUrl(url) => cx.open_url(&url),
            Run::EditLink(range) => {
                // The whole link is what is edited, whatever part of it
                // the right-click selected.
                self.editor.update(cx, |editor, cx| {
                    editor.edit(|state| state.select_range(range), cx);
                });
                self.open_link(window, cx);
            }
            Run::RemoveLink(range) => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(
                        |state| {
                            state.select_range(range);
                            state.remove_link()
                        },
                        cx,
                    );
                }
            }),
            Run::CopyText(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        }
    }
}

/// The rows of "Size" in the menu of an image: one step larger and
/// smaller (the keys that make text larger and smaller), the presets, the
/// picture's own size. The preset the image has is checked, and so is
/// "Original Size" while the image has that width. A row that would
/// change nothing because the page does not allow it is dimmed: a preset
/// wider than the widest this picture is drawn (it would be taller than a
/// page), "Larger" at that width, "Smaller" at the smallest step.
fn image_size_rows(state: &EditorState, image: &Image, editable: bool, cx: &App) -> Vec<Row> {
    let widest = state.max_image_width(image);
    // What a step starts from: the width as it is drawn.
    let drawn = image.width.min(widest);
    let steps_to = |steps: i32| drawn.stepped(steps).min(widest) != image.width;
    let preset = |label: &'static str, percent: u16, action: Rc<dyn Action>| {
        let width = ImageWidth::from_percent(percent);
        Row::new(label, What::Action(action.clone()))
            .key(KeyHint::for_action(action.as_ref(), cx))
            .checked(image.width == width)
            .enabled(editable && width <= widest)
    };
    vec![
        Row::action("Larger", editor::IncreaseTextSize, cx).enabled(editable && steps_to(1)),
        Row::action("Smaller", editor::DecreaseTextSize, cx).enabled(editable && steps_to(-1)),
        Row::separator(),
        preset("25 %", 25, Rc::new(editor::ImageWidth25)),
        preset("50 %", 50, Rc::new(editor::ImageWidth50)),
        preset("75 %", 75, Rc::new(editor::ImageWidth75)),
        preset("100 %", 100, Rc::new(editor::ImageWidth100)),
        Row::action("Original Size", editor::ImageOriginalSize, cx)
            .checked(image.width == state.original_image_width(image))
            .enabled(editable),
    ]
}

/// The rows of "Align" in the menu of an image. An image stands left, in
/// the middle or right; "Justify" means nothing for it.
fn image_align_rows(alignment: Alignment, editable: bool, cx: &App) -> Vec<Row> {
    vec![
        Row::action("Left", editor::AlignLeft, cx)
            .checked(alignment == Alignment::Left)
            .enabled(editable),
        Row::action("Centre", editor::AlignCenter, cx)
            .checked(alignment == Alignment::Center)
            .enabled(editable),
        Row::action("Right", editor::AlignRight, cx)
            .checked(alignment == Alignment::Right)
            .enabled(editable),
    ]
}
