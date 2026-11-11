//! The contextual format menu (`cmd-.`).
//!
//! A filterable list at the caret or selection. With a selection it leads
//! with inline formatting, with a collapsed caret with the block options.
//! Rows that have an editor action run that action (and show its real key
//! binding); highlight colours and text colours, which have no action, go
//! through `Editor::edit`.

use std::rc::Rc;

use gpui::{
    Action, App, AppContext, Context, Entity, Focusable, Hsla, Rgba, SharedString, Subscription,
    Window, px,
};
use writer_core::{
    Alignment, BlockKind, Choice, EditorState, FormatState, Highlight, Rgb, Toggle,
};
use writer_editor::{PopupAnchor, actions as editor};
use writer_ui::{
    KeyHint,
    theme::{ActiveTheme, HighlightColor},
};

use crate::{
    actions::{OpenFontPicker, OpenLink},
    menu::{Mark, Menu, MenuEntry, MenuItem, MenuOptions, rank},
    tools::{EditorTools, PopupTool},
};

/// The text colours the format menu offers, chosen to read on both the
/// Paper and the Night canvas.
pub const TEXT_COLORS: [(&str, Rgb); 6] = [
    ("Red", Rgb::new(0xC2, 0x3B, 0x2E)),
    ("Orange", Rgb::new(0xC4, 0x6A, 0x1B)),
    ("Green", Rgb::new(0x2F, 0x8F, 0x5B)),
    ("Blue", Rgb::new(0x2F, 0x73, 0xB8)),
    ("Purple", Rgb::new(0x80, 0x55, 0xB5)),
    ("Grey", Rgb::new(0x8A, 0x85, 0x80)),
];

/// Blocks looked at to tell which values a mixed selection contains.
const SCAN_LIMIT: usize = 2_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Text,
    Highlight,
    Color,
    TurnInto,
    Align,
    Size,
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

#[derive(Clone)]
enum Effect {
    /// Dispatch an editor action to the editor.
    Editor(Rc<dyn Action>),
    Highlight(Highlight),
    Color(Option<Rgb>),
    Link,
    Font,
}

struct FormatItem {
    section: Section,
    label: SharedString,
    keywords: &'static str,
    mark: Mark,
    swatch: Option<Hsla>,
    key_hint: Option<KeyHint>,
    effect: Effect,
}

pub(crate) struct FormatMenu {
    pub(crate) menu: Entity<Menu>,
    items: Vec<FormatItem>,
    order: [Section; 6],
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
                mark: item.mark,
                swatch: item.swatch,
                key_hint: item.key_hint.clone(),
                ..MenuItem::default()
            })
        };
        if query.trim().is_empty() {
            let mut entries = Vec::with_capacity(self.items.len() + self.order.len());
            for section in self.order {
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
        self.make_room(false, window, cx);
        let editor = self.editor.read(cx);
        let collapsed = editor.selection().is_collapsed();
        let items = format_items(editor.state(), cx);
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
            order: if collapsed { BLOCK_FIRST } else { INLINE_FIRST },
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
            Effect::Color(color) => self.editor.update(cx, |editor, cx| {
                if !editor.is_read_only() {
                    editor.edit(|state| state.set_text_color(color), cx);
                }
            }),
            Effect::Link => self.open_link(window, cx),
            Effect::Font => self.open_font_picker(window, cx),
        }
    }
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
            if !block.is_void() && !present.kinds.iter().any(|kind| kind.same_type(block.kind())) {
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
fn format_items(state: &EditorState, cx: &App) -> Vec<FormatItem> {
    let format = state.format_state();
    let present = present_values(state, &format);
    let colors = cx.theme().colors;
    let mut items = Vec::new();

    let mut action = |section, label: &str, keywords, mark, action: Rc<dyn Action>| {
        items.push(FormatItem {
            section,
            label: SharedString::from(label.to_string()),
            keywords,
            mark,
            swatch: None,
            key_hint: KeyHint::for_action(action.as_ref(), cx),
            effect: Effect::Editor(action),
        });
    };

    // Inline formatting.
    let text = Section::Text;
    action(text, "Bold", "strong weight", toggle_mark(format.bold), Rc::new(editor::ToggleBold));
    action(text, "Italic", "emphasis slanted", toggle_mark(format.italic), Rc::new(editor::ToggleItalic));
    action(text, "Underline", "", toggle_mark(format.underline), Rc::new(editor::ToggleUnderline));
    action(
        text,
        "Strikethrough",
        "strike crossed out deleted",
        toggle_mark(format.strikethrough),
        Rc::new(editor::ToggleStrikethrough),
    );
    action(text, "Inline code", "monospace", toggle_mark(format.code), Rc::new(editor::ToggleInlineCode));

    // Block kinds.
    let kind_mark = |kind: &BlockKind| {
        choice_mark(&format.kind, &present.kinds, |value| value.same_type(kind))
    };
    let turn = Section::TurnInto;
    action(turn, "Paragraph", "text body normal plain", kind_mark(&BlockKind::Paragraph), Rc::new(editor::SetParagraph));
    let headings: [(Rc<dyn Action>, &str, &str); 6] = [
        (Rc::new(editor::SetHeading1), "Heading 1", "h1 title"),
        (Rc::new(editor::SetHeading2), "Heading 2", "h2 section"),
        (Rc::new(editor::SetHeading3), "Heading 3", "h3 subsection"),
        (Rc::new(editor::SetHeading4), "Heading 4", "h4"),
        (Rc::new(editor::SetHeading5), "Heading 5", "h5"),
        (Rc::new(editor::SetHeading6), "Heading 6", "h6"),
    ];
    for (level, (heading, label, keywords)) in headings.into_iter().enumerate() {
        let kind = BlockKind::heading(level as u8 + 1);
        action(turn, label, keywords, kind_mark(&kind), heading);
    }
    action(turn, "Bullet list", "ul unordered points", kind_mark(&BlockKind::BulletItem), Rc::new(editor::ToggleBulletList));
    action(turn, "Numbered list", "ol ordered steps", kind_mark(&BlockKind::NumberedItem), Rc::new(editor::ToggleNumberedList));
    action(turn, "To-do list", "todo task checkbox checklist", kind_mark(&BlockKind::todo()), Rc::new(editor::ToggleTodoList));
    action(turn, "Quote", "blockquote citation", kind_mark(&BlockKind::Quote), Rc::new(editor::ToggleQuote));
    action(turn, "Code block", "pre monospace snippet", kind_mark(&BlockKind::code()), Rc::new(editor::ToggleCodeBlock));

    // Alignment.
    let align_mark = |alignment: Alignment| {
        choice_mark(&format.alignment, &present.alignments, |value| *value == alignment)
    };
    let align = Section::Align;
    action(align, "Align left", "", align_mark(Alignment::Left), Rc::new(editor::AlignLeft));
    action(align, "Align centre", "center middle", align_mark(Alignment::Center), Rc::new(editor::AlignCenter));
    action(align, "Align right", "", align_mark(Alignment::Right), Rc::new(editor::AlignRight));
    action(align, "Justify", "align both edges", align_mark(Alignment::Justify), Rc::new(editor::AlignJustify));

    // Text size.
    let size = Section::Size;
    action(size, "Larger text", "bigger increase size", Mark::None, Rc::new(editor::IncreaseTextSize));
    action(size, "Smaller text", "decrease size", Mark::None, Rc::new(editor::DecreaseTextSize));
    action(size, "Reset text size", "default normal size", Mark::None, Rc::new(editor::ResetTextSize));

    // Highlight colours.
    for highlight in Highlight::ALL {
        let name = highlight.name();
        let label = format!("{}{} highlight", name[..1].to_uppercase(), &name[1..]);
        items.push(FormatItem {
            section: Section::Highlight,
            label: label.into(),
            keywords: "marker background",
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
        keywords: "color automatic ink black reset",
        mark: choice_mark(&format.color, &present.colors, |value| value.is_none()),
        swatch: Some(colors.ink),
        key_hint: None,
        effect: Effect::Color(None),
    });
    for (name, rgb) in TEXT_COLORS {
        items.push(FormatItem {
            section: Section::Color,
            label: format!("{name} text").into(),
            keywords: "color colour",
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
        keywords: "url hyperlink web address",
        mark: link_mark,
        swatch: None,
        key_hint: KeyHint::for_action(&OpenLink, cx),
        effect: Effect::Link,
    });
    items.push(FormatItem {
        section: Section::Text,
        label: "Font".into(),
        keywords: "typeface family",
        mark: Mark::None,
        swatch: None,
        key_hint: KeyHint::for_action(&OpenFontPicker, cx),
        effect: Effect::Font,
    });
    let clear: Rc<dyn Action> = Rc::new(editor::ClearFormatting);
    items.push(FormatItem {
        section: Section::Text,
        label: "Clear formatting".into(),
        keywords: "remove plain reset styles",
        mark: Mark::None,
        swatch: None,
        key_hint: KeyHint::for_action(clear.as_ref(), cx),
        effect: Effect::Editor(clear),
    });
    items
}
