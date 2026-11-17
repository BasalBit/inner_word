//! The contextual format menu (`cmd-.`).
//!
//! A filterable list at the caret or selection. With a selection it leads
//! with inline formatting, with a collapsed caret with the block options.
//! Rows that have an editor action run that action (and show its real key
//! binding); highlight colours and text colours, which have no action, go
//! through `Editor::edit`.
//!
//! On a selected image (the caret is on its block) none of that applies,
//! and the menu is the image's: its size (one step larger or smaller, the
//! presets the page allows for this picture, its original size, with a
//! check on the one it has), where it stands (left, centre, right), its
//! description, and deleting it.

use std::rc::Rc;

use gpui::{
    Action, App, AppContext, Context, Entity, Focusable, Hsla, Rgba, SharedString, Subscription,
    Window, px,
};
use innerword_core::{
    Alignment, BlockKind, Choice, EditorState, FormatState, Highlight, Image, ImageWidth, Rgb,
    Toggle,
};
use innerword_editor::{PopupAnchor, actions as editor};
use innerword_ui::{
    KeyHint,
    theme::{ActiveTheme, HighlightColor},
};

use crate::{
    actions::{EditImageDescription, OpenFontPicker, OpenLink},
    menu::{Mark, Menu, MenuEntry, MenuItem, MenuOptions, rank},
    tools::{EditorTools, PopupTool},
};

/// The text colours the format menu offers.
///
/// A document stores a text colour as plain RGB and the editor paints it
/// as stored, in both themes. One fixed colour cannot have a high contrast
/// against a white page and against a charcoal one at the same time: the
/// best any colour can do on both is about 4:1. These six sit at that
/// balance point (relative luminance about 0.205), which gives each of
/// them between 3.8:1 and 4.3:1 on the Paper canvas, the white page, the
/// Night canvas and the Night page. Before, red, blue and purple were
/// tuned for Paper and dropped to 3:1 on Night. The tests hold every
/// colour to at least 3.8:1 on all four surfaces.
pub const TEXT_COLORS: [(&str, Rgb); 6] = [
    ("Red", Rgb::new(0xDC, 0x4A, 0x3A)),
    ("Orange", Rgb::new(0xBF, 0x65, 0x15)),
    ("Green", Rgb::new(0x29, 0x8E, 0x58)),
    ("Blue", Rgb::new(0x30, 0x80, 0xCF)),
    ("Purple", Rgb::new(0x96, 0x66, 0xCC)),
    ("Grey", Rgb::new(0x82, 0x7C, 0x76)),
];

/// Blocks looked at to tell which values a mixed selection contains.
const SCAN_LIMIT: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Section {
    Text,
    Highlight,
    Color,
    TurnInto,
    Align,
    Size,
    /// The width of the selected image.
    ImageSize,
    /// What else there is to do with the selected image.
    Image,
}

impl Section {
    fn title(self) -> &'static str {
        match self {
            Section::Text => "Text",
            Section::Highlight => "Highlight",
            Section::Color => "Text colour",
            Section::TurnInto => "Turn into",
            Section::Align => "Align",
            Section::Size => "Text size",
            Section::ImageSize => "Size",
            Section::Image => "Image",
        }
    }
}

const INLINE_FIRST: [Section; 6] = [
    Section::Text,
    Section::Highlight,
    Section::Color,
    Section::TurnInto,
    Section::Align,
    Section::Size,
];
const BLOCK_FIRST: [Section; 6] = [
    Section::TurnInto,
    Section::Align,
    Section::Size,
    Section::Text,
    Section::Highlight,
    Section::Color,
];
/// The menu of a selected image.
const IMAGE: [Section; 3] = [Section::ImageSize, Section::Align, Section::Image];

#[derive(Clone)]
pub(crate) enum Effect {
    /// Dispatch an editor action to the editor.
    Editor(Rc<dyn Action>),
    /// Toggle a highlight colour.
    Highlight(Highlight),
    /// Take the highlight off (the context menu's "None").
    NoHighlight,
    Color(Option<Rgb>),
    Link,
    Font,
    /// Open the description of the selected image.
    ImageDescription,
}

pub(crate) struct FormatItem {
    pub(crate) section: Section,
    pub(crate) label: SharedString,
    /// The row's name in the text context menu, where the section is a
    /// submenu and its title need not be repeated ("Yellow" under
    /// "Highlight", "Left" under "Align").
    pub(crate) short: &'static str,
    keywords: &'static str,
    /// The current value, where a mark cannot say it (the font's name,
    /// the text size).
    pub(crate) detail: Option<SharedString>,
    pub(crate) mark: Mark,
    pub(crate) swatch: Option<Hsla>,
    pub(crate) key_hint: Option<KeyHint>,
    pub(crate) effect: Effect,
}

pub(crate) struct FormatMenu {
    pub(crate) menu: Entity<Menu>,
    items: Vec<FormatItem>,
    order: &'static [Section],
    _subscription: Subscription,
}

impl FormatMenu {
    fn entries(&self, query: &str) -> Vec<MenuEntry> {
        let entry = |id: usize, positions: Vec<usize>| {
            let item = &self.items[id];
            MenuEntry::Item(MenuItem {
                id,
                label: item.label.clone(),
                positions,
                detail: item.detail.clone(),
                mark: item.mark,
                swatch: item.swatch,
                key_hint: item.key_hint.clone(),
                ..MenuItem::default()
            })
        };
        if query.trim().is_empty() {
            let mut entries = Vec::with_capacity(self.items.len() + self.order.len());
            for &section in self.order {
                entries.push(MenuEntry::Header(section.title().into()));
                entries.extend(
                    self.items
                        .iter()
                        .enumerate()
                        .filter(|(_, item)| item.section == section)
                        .map(|(id, _)| entry(id, Vec::new())),
                );
            }
            return entries;
        }
        // Rank in display order, so equal scores keep the menu's order.
        let ordered = self.order.iter().flat_map(|section| {
            self.items
                .iter()
                .enumerate()
                .filter(move |(_, item)| item.section == *section)
                .map(|(id, item)| (id, item.label.as_ref(), item.keywords))
        });
        rank(query, ordered)
            .into_iter()
            .map(|(id, positions)| entry(id, positions))
            .collect()
    }
}

impl EditorTools {
    /// Opens the format menu at the caret or selection and moves focus
    /// into its filter field. Does nothing in a read-only editor.
    pub fn open_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only(cx) {
            return;
        }
        self.make_room(window, cx);
        let editor = self.editor.read(cx);
        let collapsed = editor.selection().is_collapsed();
        let (items, order) = format_menu_content(editor.state(), cx);
        let menu = cx.new(|cx| {
            Menu::new(
                MenuOptions {
                    placeholder: Some("Format".into()),
                    width: px(300.),
                    max_rows: 10,
                    marks: true,
                    empty_text: "No matching format".into(),
                    footer: Vec::new(),
                },
                window,
                cx,
            )
        });
        let tool = FormatMenu {
            menu: menu.clone(),
            items,
            order,
            _subscription: cx.subscribe_in(&menu, window, Self::on_menu_event),
        };
        let entries = tool.entries("");
        menu.update(cx, |menu, cx| menu.set_entries(entries, None, cx));
        let anchor = if collapsed {
            PopupAnchor::Caret
        } else {
            PopupAnchor::Selection
        };
        window.focus(&menu.read(cx).focus_handle(cx));
        self.show_popup(PopupTool::Format(tool), menu.into(), anchor, cx);
    }

    /// `cmd-.`: opens the format menu, or closes it when it is open.
    pub(crate) fn toggle_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.popup, Some(PopupTool::Format(_))) {
            self.close_popup(true, window, cx);
        } else {
            self.open_format_menu(window, cx);
        }
    }

    /// The theme changed under an open format menu: its swatches (the
    /// highlight colours, the ink of "Default text colour") are colours of
    /// the theme.
    pub(crate) fn retheme_format_menu(&mut self, cx: &mut Context<Self>) {
        let Some(PopupTool::Format(tool)) = &mut self.popup else {
            return;
        };
        (tool.items, tool.order) = format_menu_content(self.editor.read(cx).state(), cx);
        let query = tool.menu.read(cx).query(cx);
        let selected = tool.menu.read(cx).selected_id();
        let entries = tool.entries(&query);
        tool.menu
            .update(cx, |menu, cx| menu.set_entries(entries, selected, cx));
    }

    pub(crate) fn refresh_format_menu(&mut self, cx: &mut Context<Self>) {
        let Some(PopupTool::Format(tool)) = &self.popup else {
            return;
        };
        let query = tool.menu.read(cx).query(cx);
        let entries = tool.entries(&query);
        tool.menu
            .update(cx, |menu, cx| menu.set_entries(entries, None, cx));
    }

    pub(crate) fn confirm_format_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PopupTool::Format(tool)) = &self.popup else {
            return;
        };
        let Some(effect) = tool
            .menu
            .read(cx)
            .selected_id()
            .map(|id| tool.items[id].effect.clone())
        else {
            return;
        };
        self.close_popup(true, window, cx);
        self.apply_format_effect(effect, window, cx);
    }

    /// Does what a row of the format menu (or the same row in the text
    /// context menu) stands for.
    pub(crate) fn apply_format_effect(
        &mut self,
        effect: Effect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match effect {
            Effect::Editor(action) => {
                // The same path as the action's key binding.
                let focus = self.editor.read(cx).focus_handle(cx);
                focus.dispatch_action(action.as_ref(), window, cx);
            }
            Effect::Highlight(highlight) => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(|state| state.toggle_highlight(highlight), cx);
                }
            }),
            Effect::NoHighlight => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(|state| state.set_highlight(None), cx);
                }
            }),
            Effect::Color(color) => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(|state| state.set_text_color(color), cx);
                }
            }),
            Effect::Link => self.open_link(window, cx),
            Effect::Font => self.open_font_picker(window, cx),
            Effect::ImageDescription => {
                self.open_image_description(window, cx);
            }
        }
    }
}

/// What the format menu offers for the selection as it is, and the order
/// of its sections: the rows of the selected image, or the formatting of
/// text (inline formatting first with a selection, the block first with a
/// collapsed caret).
fn format_menu_content(state: &EditorState, cx: &App) -> (Vec<FormatItem>, &'static [Section]) {
    if let Some((index, image)) = state.selected_image() {
        let alignment = state
            .document()
            .block(index)
            .map_or(Alignment::Left, |block| block.alignment());
        return (image_items(state, image, alignment, cx), &IMAGE);
    }
    let order: &'static [Section] = if state.selection().is_collapsed() {
        &BLOCK_FIRST
    } else {
        &INLINE_FIRST
    };
    (format_items(state, cx), order)
}

/// The rows of the format menu for the selected image. Written as the
/// rows of this menu are (a capital at the start only); the image's
/// context menu has the same commands under its own names.
///
/// A preset the page does not allow for this picture (at that width it
/// would be taller than a page) is left out: a row of this menu cannot be
/// dimmed, and one that changes nothing is not offered.
fn image_items(
    state: &EditorState,
    image: &Image,
    alignment: Alignment,
    cx: &App,
) -> Vec<FormatItem> {
    let mut items = Vec::new();
    let mut row = |section, label: &'static str, keywords, mark, effect: Effect| {
        let key_hint = match &effect {
            Effect::Editor(action) => KeyHint::for_action(action.as_ref(), cx),
            Effect::ImageDescription => KeyHint::for_action(&EditImageDescription, cx),
            _ => None,
        };
        items.push(FormatItem {
            section,
            label: label.into(),
            short: label,
            keywords,
            detail: None,
            mark,
            swatch: None,
            key_hint,
            effect,
        });
    };
    let check = |on: bool| if on { Mark::Check } else { Mark::None };
    let action = |action: Rc<dyn Action>| Effect::Editor(action);

    let size = Section::ImageSize;
    row(
        size,
        "Larger",
        "bigger increase wider grow size image picture",
        Mark::None,
        action(Rc::new(editor::IncreaseTextSize)),
    );
    row(
        size,
        "Smaller",
        "decrease narrower shrink size image picture",
        Mark::None,
        action(Rc::new(editor::DecreaseTextSize)),
    );
    let widest = state.max_image_width(image);
    let presets: [(&'static str, u16, &'static str, Rc<dyn Action>); 4] = [
        (
            "25 %",
            25,
            "quarter width size",
            Rc::new(editor::ImageWidth25),
        ),
        ("50 %", 50, "half width size", Rc::new(editor::ImageWidth50)),
        (
            "75 %",
            75,
            "three quarters width size",
            Rc::new(editor::ImageWidth75),
        ),
        (
            "100 %",
            100,
            "full column width size",
            Rc::new(editor::ImageWidth100),
        ),
    ];
    for (label, percent, keywords, preset) in presets {
        let width = ImageWidth::from_percent(percent);
        if width <= widest {
            row(
                size,
                label,
                keywords,
                check(image.width == width),
                action(preset),
            );
        }
    }
    row(
        size,
        "Original size",
        "actual natural reset own size",
        check(image.width == state.original_image_width(image)),
        action(Rc::new(editor::ImageOriginalSize)),
    );

    let align = Section::Align;
    row(
        align,
        "Align left",
        "",
        check(alignment == Alignment::Left),
        action(Rc::new(editor::AlignLeft)),
    );
    row(
        align,
        "Align centre",
        "center middle",
        check(alignment == Alignment::Center),
        action(Rc::new(editor::AlignCenter)),
    );
    row(
        align,
        "Align right",
        "",
        check(alignment == Alignment::Right),
        action(Rc::new(editor::AlignRight)),
    );

    let rest = Section::Image;
    row(
        rest,
        "Description…",
        "alt text caption describe accessibility",
        check(!image.description.is_empty()),
        Effect::ImageDescription,
    );
    row(
        rest,
        "Delete image",
        "remove picture",
        Mark::None,
        action(Rc::new(editor::Backspace)),
    );
    items
}

fn toggle_mark(toggle: Toggle) -> Mark {
    match toggle {
        Toggle::On => Mark::Check,
        Toggle::Mixed => Mark::Mixed,
        Toggle::Off => Mark::None,
    }
}

/// The mark of a row that stands for one value of a multi-valued
/// attribute: a check when the whole selection has it, the "mixed" dash
/// when a part of the selection has it.
fn choice_mark<T>(choice: &Choice<T>, present: &[T], is_value: impl Fn(&T) -> bool) -> Mark {
    match choice {
        Choice::Same(value) if is_value(value) => Mark::Check,
        Choice::Same(_) => Mark::None,
        Choice::Mixed if present.iter().any(is_value) => Mark::Mixed,
        Choice::Mixed => Mark::None,
    }
}

/// Which values a mixed selection contains.
#[derive(Default)]
struct Present {
    kinds: Vec<BlockKind>,
    alignments: Vec<Alignment>,
    highlights: Vec<Option<Highlight>>,
    colors: Vec<Option<Rgb>>,
}

fn present_values(state: &EditorState, format: &FormatState) -> Present {
    let mut present = Present::default();
    let document = state.document();
    if matches!(format.kind, Choice::Mixed) || matches!(format.alignment, Choice::Mixed) {
        for block in document.blocks()[state.touched_blocks()]
            .iter()
            .take(SCAN_LIMIT)
        {
            if !block.is_void()
                && !present
                    .kinds
                    .iter()
                    .any(|kind| kind.same_type(block.kind()))
            {
                present.kinds.push(block.kind().clone());
            }
            if !present.alignments.contains(&block.alignment()) {
                present.alignments.push(block.alignment());
            }
        }
    }
    if matches!(format.highlight, Choice::Mixed) || matches!(format.color, Choice::Mixed) {
        let range = state.selection().range();
        for index in (range.start.block..=range.end.block).take(SCAN_LIMIT) {
            let (Some(block), Some(part)) =
                (document.block(index), document.range_in_block(range, index))
            else {
                continue;
            };
            for (span, attrs) in block.spans() {
                if span.start < part.end && part.start < span.end {
                    if !present.highlights.contains(&attrs.highlight) {
                        present.highlights.push(attrs.highlight);
                    }
                    if !present.colors.contains(&attrs.color) {
                        present.colors.push(attrs.color);
                    }
                }
            }
        }
    }
    present
}

/// A highlight colour's name as a menu row.
fn highlight_title(highlight: Highlight) -> &'static str {
    match highlight {
        Highlight::Yellow => "Yellow",
        Highlight::Green => "Green",
        Highlight::Blue => "Blue",
        Highlight::Pink => "Pink",
        Highlight::Orange => "Orange",
        Highlight::Purple => "Purple",
    }
}

fn rgb_color(rgb: Rgb) -> Hsla {
    Rgba {
        r: f32::from(rgb.r) / 255.,
        g: f32::from(rgb.g) / 255.,
        b: f32::from(rgb.b) / 255.,
        a: 1.,
    }
    .into()
}

/// Every row of the format menu with its current state, in canonical
/// order (the menu reorders the sections).
pub(crate) fn format_items(state: &EditorState, cx: &App) -> Vec<FormatItem> {
    let format = state.format_state();
    let present = present_values(state, &format);
    let colors = cx.theme().colors;
    let mut items = Vec::new();

    let mut action =
        |section, (label, short): (&str, &'static str), keywords, mark, action: Rc<dyn Action>| {
            items.push(FormatItem {
                section,
                label: SharedString::from(label.to_string()),
                short,
                keywords,
                detail: None,
                mark,
                swatch: None,
                key_hint: KeyHint::for_action(action.as_ref(), cx),
                effect: Effect::Editor(action),
            });
        };

    // Inline formatting.
    let text = Section::Text;
    action(
        text,
        ("Bold", "Bold"),
        "strong weight",
        toggle_mark(format.bold),
        Rc::new(editor::ToggleBold),
    );
    action(
        text,
        ("Italic", "Italic"),
        "emphasis slanted",
        toggle_mark(format.italic),
        Rc::new(editor::ToggleItalic),
    );
    action(
        text,
        ("Underline", "Underline"),
        "",
        toggle_mark(format.underline),
        Rc::new(editor::ToggleUnderline),
    );
    action(
        text,
        ("Strikethrough", "Strikethrough"),
        "strike crossed out deleted",
        toggle_mark(format.strikethrough),
        Rc::new(editor::ToggleStrikethrough),
    );
    action(
        text,
        ("Inline code", "Inline Code"),
        "monospace",
        toggle_mark(format.code),
        Rc::new(editor::ToggleInlineCode),
    );

    // Block kinds.
    let kind_mark =
        |kind: &BlockKind| choice_mark(&format.kind, &present.kinds, |value| value.same_type(kind));
    let turn = Section::TurnInto;
    action(
        turn,
        ("Paragraph", "Paragraph"),
        "text body normal plain",
        kind_mark(&BlockKind::Paragraph),
        Rc::new(editor::SetParagraph),
    );
    let headings: [(Rc<dyn Action>, &'static str, &str); 6] = [
        (Rc::new(editor::SetHeading1), "Heading 1", "h1 title"),
        (Rc::new(editor::SetHeading2), "Heading 2", "h2 section"),
        (Rc::new(editor::SetHeading3), "Heading 3", "h3 subsection"),
        (Rc::new(editor::SetHeading4), "Heading 4", "h4"),
        (Rc::new(editor::SetHeading5), "Heading 5", "h5"),
        (Rc::new(editor::SetHeading6), "Heading 6", "h6"),
    ];
    for (level, (heading, label, keywords)) in headings.into_iter().enumerate() {
        let kind = BlockKind::heading(level as u8 + 1);
        action(turn, (label, label), keywords, kind_mark(&kind), heading);
    }
    action(
        turn,
        ("Bullet list", "Bullet List"),
        "ul unordered points",
        kind_mark(&BlockKind::BulletItem),
        Rc::new(editor::ToggleBulletList),
    );
    action(
        turn,
        ("Numbered list", "Numbered List"),
        "ol ordered steps",
        kind_mark(&BlockKind::NumberedItem),
        Rc::new(editor::ToggleNumberedList),
    );
    action(
        turn,
        ("To-do list", "To-do List"),
        "todo task checkbox checklist",
        kind_mark(&BlockKind::todo()),
        Rc::new(editor::ToggleTodoList),
    );
    action(
        turn,
        ("Quote", "Quote"),
        "blockquote citation",
        kind_mark(&BlockKind::Quote),
        Rc::new(editor::ToggleQuote),
    );
    action(
        turn,
        ("Code block", "Code Block"),
        "pre monospace snippet",
        kind_mark(&BlockKind::code()),
        Rc::new(editor::ToggleCodeBlock),
    );

    // Alignment.
    let align_mark = |alignment: Alignment| {
        choice_mark(&format.alignment, &present.alignments, |value| {
            *value == alignment
        })
    };
    let align = Section::Align;
    action(
        align,
        ("Align left", "Left"),
        "",
        align_mark(Alignment::Left),
        Rc::new(editor::AlignLeft),
    );
    action(
        align,
        ("Align centre", "Centre"),
        "center middle",
        align_mark(Alignment::Center),
        Rc::new(editor::AlignCenter),
    );
    action(
        align,
        ("Align right", "Right"),
        "",
        align_mark(Alignment::Right),
        Rc::new(editor::AlignRight),
    );
    action(
        align,
        ("Justify", "Justify"),
        "align both edges",
        align_mark(Alignment::Justify),
        Rc::new(editor::AlignJustify),
    );

    // Text size.
    let size = Section::Size;
    action(
        size,
        ("Larger text", "Larger"),
        "bigger increase size",
        Mark::None,
        Rc::new(editor::IncreaseTextSize),
    );
    action(
        size,
        ("Smaller text", "Smaller"),
        "decrease size",
        Mark::None,
        Rc::new(editor::DecreaseTextSize),
    );
    let reset: Rc<dyn Action> = Rc::new(editor::ResetTextSize);
    items.push(FormatItem {
        section: Section::Size,
        label: "Reset text size".into(),
        short: "Reset",
        keywords: "default normal size",
        detail: match &format.text_size {
            Choice::Same(Some(points)) => Some(format!("Now {points} pt").into()),
            Choice::Same(None) => None,
            Choice::Mixed => Some("Mixed".into()),
        },
        mark: Mark::None,
        swatch: None,
        key_hint: KeyHint::for_action(reset.as_ref(), cx),
        effect: Effect::Editor(reset),
    });

    // Highlight colours.
    for highlight in Highlight::ALL {
        let name = highlight.name();
        let label = format!("{}{} highlight", name[..1].to_uppercase(), &name[1..]);
        items.push(FormatItem {
            section: Section::Highlight,
            label: label.into(),
            short: highlight_title(highlight),
            keywords: "marker background",
            detail: None,
            mark: choice_mark(&format.highlight, &present.highlights, |value| {
                *value == Some(highlight)
            }),
            swatch: HighlightColor::parse(name).map(|color| colors.highlight.get(color)),
            key_hint: (highlight == Highlight::Yellow)
                .then(|| KeyHint::for_action(&editor::ToggleHighlight, cx))
                .flatten(),
            effect: Effect::Highlight(highlight),
        });
    }

    // Text colours.
    items.push(FormatItem {
        section: Section::Color,
        label: "Default text colour".into(),
        short: "Default",
        keywords: "color automatic ink black reset",
        detail: None,
        mark: choice_mark(&format.color, &present.colors, |value| value.is_none()),
        swatch: Some(colors.ink),
        key_hint: None,
        effect: Effect::Color(None),
    });
    for (name, rgb) in TEXT_COLORS {
        items.push(FormatItem {
            section: Section::Color,
            label: format!("{name} text").into(),
            short: name,
            keywords: "color colour",
            detail: None,
            mark: choice_mark(&format.color, &present.colors, |value| *value == Some(rgb)),
            swatch: Some(rgb_color(rgb)),
            key_hint: None,
            effect: Effect::Color(Some(rgb)),
        });
    }

    // Link, font, clear: at the end of the text section.
    let link_mark = match &format.link {
        Choice::Same(Some(_)) => Mark::Check,
        Choice::Same(None) => Mark::None,
        Choice::Mixed => Mark::Mixed,
    };
    items.push(FormatItem {
        section: Section::Text,
        label: "Link".into(),
        short: "Link",
        keywords: "url hyperlink web address",
        detail: None,
        mark: link_mark,
        swatch: None,
        key_hint: KeyHint::for_action(&OpenLink, cx),
        effect: Effect::Link,
    });
    items.push(FormatItem {
        section: Section::Text,
        label: "Font".into(),
        short: "Font",
        keywords: "typeface family",
        detail: match &format.font {
            Choice::Same(Some(family)) => Some(family.clone().into()),
            Choice::Same(None) => None,
            Choice::Mixed => Some("Mixed".into()),
        },
        mark: Mark::None,
        swatch: None,
        key_hint: KeyHint::for_action(&OpenFontPicker, cx),
        effect: Effect::Font,
    });
    let clear: Rc<dyn Action> = Rc::new(editor::ClearFormatting);
    items.push(FormatItem {
        section: Section::Text,
        label: "Clear formatting".into(),
        short: "Clear Formatting",
        keywords: "remove plain reset styles",
        detail: None,
        mark: Mark::None,
        swatch: None,
        key_hint: KeyHint::for_action(clear.as_ref(), cx),
        effect: Effect::Editor(clear),
    });
    items
}
