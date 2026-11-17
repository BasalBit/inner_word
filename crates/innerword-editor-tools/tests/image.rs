//! Images in the tools: the description popover, the image's context
//! menu, "Image" in the insert menu, the image's rows of the format menu.
//!
//! Every test uses `innerword_editor::FakeImages`: pictures in memory, no
//! library, no panel, no pasteboard.

use std::{cell::RefCell, path::PathBuf, rc::Rc};

use gpui::{
    Action, AppContext, Bounds, ClipboardItem, Context, Entity, IntoElement, Modifiers,
    MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels, Point, Render, Styled,
    TestAppContext, VisualTestContext, Window, div, point, px,
};
use innerword_core::{
    Alignment, Block, BlockKind, Document, EditorState, Image, ImageWidth, Position, Selection,
};
use innerword_editor::{Editor, EditorConfig, FakeImages, NewImageSource, actions as editor_actions};
use innerword_editor_tools::{
    EditorTools, Mark, ToolKind,
    actions::{CloseTool, EditImageDescription},
};
use innerword_ui::{ContextMenu, ContextMenuItem, KeyHint, keymap, text_field};

use crate::support::*;

const BEFORE: usize = 0;
const PICTURE: usize = 1;
const AFTER: usize = 2;

/// The block of `image` at `thousandths` of the column.
fn picture(image: &Image, thousandths: u16, alignment: Alignment) -> Block {
    let mut image = image.clone();
    image.width = ImageWidth::new(thousandths);
    Block::image(image).with_alignment(alignment)
}

fn described(image: &Image, description: &str) -> Image {
    let mut image = image.clone();
    image.description = description.to_string();
    image
}

/// A paragraph, a picture half the column wide, a paragraph.
fn around(image: &Image) -> Document {
    Document::from_blocks(vec![
        Block::paragraph("Before the picture"),
        picture(image, 500, Alignment::Center),
        Block::paragraph("After the picture"),
    ])
}

/// What the app's root view is to the tools: it draws the context menus
/// and sees the actions nothing nearer to the keyboard took.
struct Host {
    tools: Entity<EditorTools>,
    reached: Rc<RefCell<Vec<&'static str>>>,
}

impl Host {
    fn seen<A: Action>(
        name: &'static str,
        cx: &mut Context<Self>,
    ) -> impl Fn(&A, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |host: &mut Host, _: &A, _, _| host.reached.borrow_mut().push(name))
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui::InteractiveElement;
        div()
            .size_full()
            .on_action::<EditImageDescription>(Self::seen("description", cx))
            .on_action::<editor_actions::InsertImage>(Self::seen("insert image", cx))
            .child(self.tools.clone())
            .child(innerword_ui::ContextMenuLayer)
    }
}

struct Opened {
    tools: Entity<EditorTools>,
    editor: Entity<Editor>,
    /// The actions that reached the root of the window.
    reached: Rc<RefCell<Vec<&'static str>>>,
}

/// Opens `document` in a window that draws menus, with `images` as the
/// editor's provider, and lets the pictures on screen arrive.
fn open_pictures<'a>(
    document: Document,
    config: EditorConfig,
    images: Option<&Rc<FakeImages>>,
    cx: &'a mut TestAppContext,
) -> (Opened, &'a mut VisualTestContext) {
    let reached = Rc::new(RefCell::new(Vec::new()));
    let (host, cx) = cx.add_window_view(|window, cx| {
        let editor = cx.new(|cx| Editor::new(document, config, window, cx));
        editor.read(cx).focus(window);
        let tools = cx.new(|cx| EditorTools::new(editor, window, cx));
        Host {
            tools,
            reached: reached.clone(),
        }
    });
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let tools = host.read_with(cx, |host, _| host.tools.clone());
    let editor = tools.read_with(cx, |tools, _| tools.editor().clone());
    if let Some(images) = images {
        editor.update(cx, |editor, cx| {
            editor.set_image_provider(Some(images.clone()), cx)
        });
        cx.run_until_parked();
    }
    (
        Opened {
            tools,
            editor,
            reached,
        },
        cx,
    )
}

/// A document editor with a picture between two paragraphs, the caret on
/// the picture.
fn open_around<'a>(
    image: &Image,
    images: &Rc<FakeImages>,
    cx: &'a mut TestAppContext,
) -> (Opened, &'a mut VisualTestContext) {
    let (opened, cx) = open_pictures(around(image), EditorConfig::full(), Some(images), cx);
    caret(&opened.editor, PICTURE, 0, cx);
    (opened, cx)
}

fn image_at(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> Image {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[index]
            .as_image()
            .unwrap_or_else(|| panic!("block {index} is no image"))
            .clone()
    })
}

fn description(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> String {
    image_at(editor, index, cx).description
}

fn kinds(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    editor.read_with(cx, |editor, _| {
        editor
            .document()
            .blocks()
            .iter()
            .map(|block| block.kind().name())
            .collect()
    })
}

fn selection(editor: &Entity<Editor>, cx: &mut VisualTestContext) -> Selection {
    editor.read_with(cx, |editor, _| editor.selection())
}

fn field(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Option<String> {
    tools.read_with(cx, |tools, cx| tools.image_description_field_text(cx))
}

fn buttons(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    tools.read_with(cx, |tools, cx| tools.image_description_buttons(cx))
}

fn button_key(
    tools: &Entity<EditorTools>,
    button: &str,
    cx: &mut VisualTestContext,
) -> Option<String> {
    tools.read_with(cx, |tools, cx| {
        tools.image_description_button_key(button, cx)
    })
}

fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Bounds<Pixels> {
    cx.debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} was not drawn"))
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let target = bounds(selector, cx).center();
    cx.simulate_mouse_move(target, None, Modifiers::none());
    cx.simulate_click(target, Modifiers::none());
    cx.run_until_parked();
}

fn click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_mouse_move(at, None, Modifiers::none());
    cx.simulate_click(at, Modifiers::none());
    cx.run_until_parked();
}

fn right_click_at(at: Point<Pixels>, cx: &mut VisualTestContext) {
    cx.simulate_event(MouseDownEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: at,
        modifiers: Modifiers::none(),
        button: MouseButton::Right,
        click_count: 1,
    });
    cx.run_until_parked();
}

/// The middle of the picture of block `index`, in the window.
fn on_picture(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> Point<Pixels> {
    editor
        .read_with(cx, |editor, _| editor.image_bounds(index))
        .unwrap_or_else(|| panic!("block {index} has no picture on screen"))
        .center()
}

/// A window point inside the character at a document position.
fn in_text(
    editor: &Entity<Editor>,
    block: usize,
    offset: usize,
    cx: &mut VisualTestContext,
) -> Point<Pixels> {
    let bounds = editor.read_with(cx, |editor, _| {
        editor
            .position_bounds(Position::new(block, offset))
            .unwrap()
    });
    point(bounds.left() + px(2.), bounds.center().y)
}

/// What somebody else (an agent) makes of the document as the editor has
/// it.
fn elsewhere(
    editor: &Entity<Editor>,
    cx: &mut VisualTestContext,
    change: impl FnOnce(&mut EditorState),
) -> Document {
    let stored = editor.read_with(cx, |editor, _| editor.document().to_json());
    let mut other = EditorState::new(Document::from_json(&stored).unwrap());
    change(&mut other);
    Document::from_json(&other.document().to_json()).unwrap()
}

fn open_description(cx: &mut VisualTestContext) {
    cx.dispatch_action(EditImageDescription);
    cx.run_until_parked();
}

fn hint(action: &dyn Action, cx: &mut VisualTestContext) -> Option<String> {
    cx.update(|_, cx| KeyHint::for_action(action, cx).map(|hint| hint.text()))
}

fn rebind(action: &dyn Action, from: &str, to: Option<&str>, cx: &mut VisualTestContext) -> usize {
    let changed = cx.update(|_, cx| keymap::replace_key(action, from, to, cx));
    cx.run_until_parked();
    changed
}

// ---------------------------------------------------------------------------
// The description popover
// ---------------------------------------------------------------------------

#[gpui::test]
fn the_description_is_typed_under_the_picture_and_kept_on_submit(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    open_description(cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    assert!(tools.read_with(cx, |tools, _| tools.is_open(ToolKind::ImageDescription)));
    assert!(!editor_focused(&editor, cx), "the field has the keyboard");
    assert_eq!(field(&tools, cx).as_deref(), Some(""));
    // No description yet: nothing to remove.
    assert_eq!(
        buttons(&tools, cx),
        ["image-description-done", "image-description-cancel"]
    );
    assert!(cx.debug_bounds("image-description-remove").is_none());

    // Under the picture, at its left edge, inside the window.
    let picture = editor
        .read_with(cx, |editor, _| editor.image_bounds(PICTURE))
        .unwrap();
    let popover = bounds("image-description-popover", cx);
    assert!(popover.top() >= picture.bottom(), "{popover:?} {picture:?}");
    assert!(popover.top() - picture.bottom() < px(12.));
    assert!((popover.left() - picture.left()).abs() < px(1.));
    // The line that says where a description goes is drawn, on one line,
    // inside the panel.
    let note = bounds("image-description-note", cx);
    assert!(note.left() >= popover.left() && note.right() <= popover.right());
    assert!(note.size.height < px(30.), "{note:?}");

    cx.simulate_input("The harbour at dusk");
    assert_eq!(field(&tools, cx).as_deref(), Some("The harbour at dusk"));
    assert_eq!(description(&editor, PICTURE, cx), "", "not before Done");
    assert_eq!(text(&editor, cx), "Before the picture\n\nAfter the picture");

    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "The harbour at dusk");
    // The image is as it was otherwise, and still selected.
    let image = image_at(&editor, PICTURE, cx);
    assert_eq!((image.id, image.width.thousandths()), (photo.id, 500));
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(PICTURE, 0))
    );

    // One undo step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(description(&editor, PICTURE, cx), "");
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(description(&editor, PICTURE, cx), "The harbour at dusk");

    // Opened again it holds the description, all selected: typing
    // replaces it. An emptied field removes the description.
    open_description(cx);
    assert_eq!(field(&tools, cx).as_deref(), Some("The harbour at dusk"));
    assert_eq!(
        buttons(&tools, cx),
        [
            "image-description-done",
            "image-description-remove",
            "image-description-cancel"
        ]
    );
    cx.simulate_input("Boats");
    assert_eq!(field(&tools, cx).as_deref(), Some("Boats"));
    cx.simulate_keystrokes("backspace backspace backspace backspace backspace enter");
    assert_eq!(description(&editor, PICTURE, cx), "");

    // A description is one line, trimmed (what an image keeps).
    open_description(cx);
    cx.simulate_input("  Two   boats  ");
    cx.simulate_keystrokes("enter");
    assert_eq!(description(&editor, PICTURE, cx), "Two   boats");

    // Submit with nothing changed is no edit: undo takes out the step
    // before it.
    open_description(cx);
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "Two   boats");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(description(&editor, PICTURE, cx), "");
}

#[gpui::test]
fn done_remove_and_cancel_are_buttons(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = described(&images.add_png(400, 300), "A lighthouse");
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    // Cancel: nothing changes, whatever was typed.
    open_description(cx);
    cx.simulate_input("Something else");
    click("image-description-cancel", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse");

    // The field's own cancel key: the same.
    open_description(cx);
    cx.simulate_input("Something else");
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse");
    // Neither was an edit.
    assert!(!editor.read_with(cx, |editor, _| editor.state().can_undo()));

    // Done.
    open_description(cx);
    cx.simulate_input("A lighthouse on a rock");
    click("image-description-done", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse on a rock");

    // Remove: the description goes, whatever the field holds, in one step.
    open_description(cx);
    cx.simulate_input("typed and not wanted");
    click("image-description-remove", cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse on a rock");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse");

    // The keys beside the buttons are the keymap's.
    open_description(cx);
    assert_eq!(
        button_key(&tools, "image-description-done", cx),
        hint(&text_field::Submit, cx)
    );
    assert_eq!(
        button_key(&tools, "image-description-cancel", cx),
        hint(&CloseTool, cx)
    );
    assert_eq!(button_key(&tools, "image-description-remove", cx), None);
    assert!(button_key(&tools, "image-description-done", cx).is_some());
    assert!(button_key(&tools, "image-description-cancel", cx).is_some());
}

#[gpui::test]
fn what_was_typed_is_kept_when_the_popover_closes_another_way(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    // A click outside, in the text: the description is kept and the click
    // does what a click does.
    open_description(cx);
    cx.simulate_input("One");
    let at = in_text(&editor, AFTER, 6, cx);
    click_at(at, cx);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "One");
    assert!(editor_focused(&editor, cx));
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(AFTER, 6))
    );

    // The keyboard leaves the field for the text.
    caret(&editor, PICTURE, 0, cx);
    open_description(cx);
    cx.simulate_input("Two");
    cx.update(|window, cx| editor.read(cx).focus(window));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "Two");

    // Another tool takes the popover's place.
    open_description(cx);
    cx.simulate_input("Three");
    cx.simulate_keystrokes("cmd-f");
    assert_eq!(active(&tools, cx), Some(ToolKind::Find));
    assert_eq!(description(&editor, PICTURE, cx), "Three");
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);

    // The command once more closes it, and keeps the field.
    caret(&editor, PICTURE, 0, cx);
    open_description(cx);
    cx.simulate_input("Four");
    open_description(cx);
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "Four");

    // Each of them was one undo step.
    for before in ["Three", "Two", "One", ""] {
        cx.simulate_keystrokes("cmd-z");
        assert_eq!(description(&editor, PICTURE, cx), before);
    }
    assert_eq!(text(&editor, cx), "Before the picture\n\nAfter the picture");

    // Another window in front is not a reason to close: the popover is
    // there when the user comes back, with what was typed.
    caret(&editor, PICTURE, 0, cx);
    open_description(cx);
    cx.simulate_input("Five");
    cx.deactivate_window();
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    assert_eq!(description(&editor, PICTURE, cx), "");
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    assert_eq!(field(&tools, cx).as_deref(), Some("Five"));
    cx.simulate_keystrokes("enter");
    assert_eq!(description(&editor, PICTURE, cx), "Five");
}

#[gpui::test]
fn the_description_with_the_pointer_alone(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(around(&photo), EditorConfig::full(), Some(&images), cx);
    cx.write_to_clipboard(ClipboardItem::new_string("A harbour".into()));
    caret(&editor, BEFORE, 0, cx);

    // A right-click on the picture selects it and opens its menu.
    let at = on_picture(&editor, PICTURE, cx);
    right_click_at(at, cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(PICTURE, 0))
    );
    let row = menu_item_bounds(&tools, "Description…", cx);
    click_at(row.center(), cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    assert_eq!(field(&tools, cx).as_deref(), Some(""));

    // The field's own menu pastes.
    let popover = bounds("image-description-popover", cx);
    let in_field = point(popover.left() + px(200.), popover.top() + px(17.));
    right_click_at(in_field, cx);
    assert_eq!(
        active(&tools, cx),
        Some(ToolKind::ImageDescription),
        "the field's menu does not close the popover"
    );
    let paste = cx.update(|window, cx| {
        let menu = ContextMenu::current(window, cx).expect("the field's menu");
        let menu = menu.read(cx);
        let index = menu
            .items()
            .iter()
            .position(|item| item.title().as_ref() == "Paste")
            .expect("Paste");
        menu.item_bounds(index).expect("drawn")
    });
    click_at(paste.center(), cx);
    assert_eq!(field(&tools, cx).as_deref(), Some("A harbour"));
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));

    click("image-description-done", cx);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "A harbour");
    assert!(editor_focused(&editor, cx));
}

fn text_menu(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Entity<ContextMenu> {
    tools
        .read_with(cx, |tools, _| tools.text_menu().cloned())
        .expect("the text menu is open")
}

/// The window bounds of the root item `title` of the open text menu.
fn menu_item_bounds(
    tools: &Entity<EditorTools>,
    title: &str,
    cx: &mut VisualTestContext,
) -> Bounds<Pixels> {
    text_menu(tools, cx).read_with(cx, |menu, _| {
        let index = menu
            .items()
            .iter()
            .position(|item| item.title().as_ref() == title)
            .unwrap_or_else(|| panic!("no item {title:?}"));
        menu.item_bounds(index).expect("the item is drawn")
    })
}

#[gpui::test]
fn the_description_command_is_for_a_selected_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (
        Opened {
            tools,
            editor,
            reached,
        },
        cx,
    ) = open_pictures(around(&photo), EditorConfig::full(), Some(&images), cx);

    // The caret in text: the action travels on, and nothing opens.
    caret(&editor, BEFORE, 3, cx);
    open_description(cx);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(*reached.borrow(), ["description"]);

    // A range that covers the image is not a selected image.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(
                    Position::new(BEFORE, 2),
                    Position::new(AFTER, 3),
                ))
            },
            cx,
        )
    });
    cx.run_until_parked();
    open_description(cx);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(*reached.borrow(), ["description", "description"]);

    // On the image it is taken.
    caret(&editor, PICTURE, 0, cx);
    open_description(cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    assert_eq!(reached.borrow().len(), 2);
    cx.simulate_keystrokes("escape");

    // It is a command of the palette, without a key.
    let command = innerword_editor_tools::commands()
        .into_iter()
        .find(|command| command.title.as_ref() == "Image Description")
        .expect("the command");
    assert_eq!(
        command.category.as_ref(),
        innerword_editor_tools::CATEGORY_FORMAT
    );
    assert!(command.action_ref().as_any().is::<EditImageDescription>());
    assert_eq!(hint(&EditImageDescription, cx), None);
}

#[gpui::test]
fn nothing_of_the_description_in_an_editor_that_cannot_be_changed(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = described(&images.add_png(400, 300), "A lighthouse");
    let (
        Opened {
            tools,
            editor,
            reached,
        },
        cx,
    ) = open_pictures(
        around(&photo),
        EditorConfig::full().read_only(true),
        Some(&images),
        cx,
    );
    caret(&editor, PICTURE, 0, cx);
    assert!(editor.read_with(cx, |editor, _| editor.selected_image().is_some()));

    open_description(cx);
    assert_eq!(active(&tools, cx), None);
    assert!(cx.debug_bounds("image-description-popover").is_none());
    // The image is selected: the command is not one for somebody else.
    assert!(reached.borrow().is_empty());
    assert_eq!(description(&editor, PICTURE, cx), "A lighthouse");
}

#[gpui::test]
fn a_description_is_only_ever_written_to_the_image_it_was_typed_for(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let other = images.add_png(300, 300);
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    // Blocks come and go around the image (an agent's change): the
    // popover stays, and the description lands on the image where it is
    // by then.
    open_description(cx);
    cx.simulate_input("The harbour");
    let changed = elsewhere(&editor, cx, |state| {
        state.set_caret(Position::new(BEFORE, 0));
        state.insert_paragraph_break();
    });
    editor.update(cx, |editor, cx| editor.adopt_document(changed, cx));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    assert_eq!(field(&tools, cx).as_deref(), Some("The harbour"));
    cx.simulate_keystrokes("enter");
    assert_eq!(description(&editor, PICTURE + 1, cx), "The harbour");

    // The image is taken out under the popover: it closes, and nothing is
    // written anywhere.
    open_description(cx);
    cx.simulate_input("Typed for a picture that goes");
    let changed = elsewhere(&editor, cx, |state| {
        state.set_caret(Position::new(PICTURE + 1, 0));
        state.backspace();
    });
    editor.update(cx, |editor, cx| editor.adopt_document(changed, cx));
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    assert!(!editor.read_with(cx, |editor, _| editor.has_popup()));
    assert!(
        editor_focused(&editor, cx),
        "the keyboard is back in the text"
    );
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "paragraph", "paragraph"],
        "no image, and nothing else changed"
    );

    // Another document is put into the editor under the popover, with an
    // image in the same place: its description is not touched.
    editor.update(cx, |editor, cx| editor.set_document(around(&photo), cx));
    cx.run_until_parked();
    caret(&editor, PICTURE, 0, cx);
    open_description(cx);
    cx.simulate_input("Typed for the old document");
    editor.update(cx, |editor, cx| {
        editor.set_document(around(&described(&other, "The other picture")), cx)
    });
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "The other picture");
    assert!(!editor.read_with(cx, |editor, _| editor.state().can_undo()));
}

#[gpui::test]
fn the_keys_of_the_description_follow_the_keymap(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = described(&images.add_png(400, 300), "A lighthouse");
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    // The field's submit on another key: that key is Done, and the button
    // says so.
    assert_eq!(rebind(&text_field::Submit, "enter", Some("cmd-s"), cx), 1);
    open_description(cx);
    assert_eq!(
        button_key(&tools, "image-description-done", cx).as_deref(),
        Some("⌘S")
    );
    cx.simulate_input("On a rock");
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    cx.simulate_keystrokes("cmd-s");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(description(&editor, PICTURE, cx), "On a rock");

    // The field's cancel without a key: the key that closes a tool is
    // what the button shows, and it cancels.
    assert_eq!(rebind(&text_field::Cancel, "escape", None, cx), 1);
    open_description(cx);
    assert_eq!(
        button_key(&tools, "image-description-cancel", cx),
        hint(&CloseTool, cx)
    );
    cx.simulate_input("Not wanted");
    cx.simulate_keystrokes("escape");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(description(&editor, PICTURE, cx), "On a rock");

    // The command on a key of the user's: it opens and closes the popover.
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-alt-d",
            EditImageDescription,
            Some("Editor"),
        )])
    });
    cx.simulate_keystrokes("cmd-alt-d");
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
}

// ---------------------------------------------------------------------------
// The context menu of an image
// ---------------------------------------------------------------------------

fn menu_labels(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<String> {
    text_menu(tools, cx).read_with(cx, |menu, _| menu.labels())
}

/// The item titled `title` of the open text menu: a root item, or
/// `"Submenu > Item"`.
fn item(tools: &Entity<EditorTools>, title: &str, cx: &mut VisualTestContext) -> ContextMenuItem {
    text_menu(tools, cx).read_with(cx, |menu, _| {
        let find = |items: &[ContextMenuItem], title: &str| {
            items
                .iter()
                .find(|item| item.title().as_ref() == title)
                .cloned()
                .unwrap_or_else(|| panic!("no item {title:?}"))
        };
        match title.split_once(" > ") {
            None => find(menu.items(), title),
            Some((parent, child)) => {
                let parent = find(menu.items(), parent);
                find(parent.submenu_items().expect("a submenu"), child)
            }
        }
    })
}

/// The rows of a submenu of the open text menu, a separator as `"-"`.
fn submenu_labels(
    tools: &Entity<EditorTools>,
    title: &str,
    cx: &mut VisualTestContext,
) -> Vec<String> {
    item(tools, title, cx)
        .submenu_items()
        .unwrap()
        .iter()
        .map(|item| {
            if item.is_separator() {
                "-".to_string()
            } else {
                item.title().to_string()
            }
        })
        .collect()
}

/// The rows of a submenu that carry a check mark.
fn checked(tools: &Entity<EditorTools>, title: &str, cx: &mut VisualTestContext) -> Vec<String> {
    item(tools, title, cx)
        .submenu_items()
        .unwrap()
        .iter()
        .filter(|item| item.is_checked() == Some(true))
        .map(|item| item.title().to_string())
        .collect()
}

/// The rows of a submenu that cannot be chosen (separators left out).
fn dimmed(tools: &Entity<EditorTools>, title: &str, cx: &mut VisualTestContext) -> Vec<String> {
    item(tools, title, cx)
        .submenu_items()
        .unwrap()
        .iter()
        .filter(|item| !item.is_separator() && item.is_disabled())
        .map(|item| item.title().to_string())
        .collect()
}

/// Chooses an item of the open menu as a click on it would.
#[track_caller]
fn choose(title: &str, cx: &mut VisualTestContext) {
    let chosen = cx.update(|window, cx| ContextMenu::choose(title, window, cx));
    assert!(chosen, "{title:?} cannot be chosen");
    cx.run_until_parked();
}

fn dismiss(cx: &mut VisualTestContext) {
    cx.update(ContextMenu::dismiss);
    cx.run_until_parked();
}

fn right_click_picture(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) {
    let at = on_picture(editor, index, cx);
    right_click_at(at, cx);
}

fn alignment(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> Alignment {
    editor.read_with(cx, |editor, _| {
        editor.document().blocks()[index].alignment()
    })
}

fn width(editor: &Entity<Editor>, index: usize, cx: &mut VisualTestContext) -> u16 {
    image_at(editor, index, cx).width.thousandths()
}

const IMAGE_MENU: [&str; 15] = [
    "Cut",
    "Copy",
    "Paste",
    "Paste as Plain Text",
    "-",
    "Size",
    "Align",
    "Description…",
    "-",
    "Insert",
    "-",
    "Find in Document",
    "Select All",
    "-",
    "Delete Image",
];

#[gpui::test]
fn a_right_click_on_a_picture_opens_the_menu_of_the_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(around(&photo), EditorConfig::full(), Some(&images), cx);
    caret(&editor, BEFORE, 2, cx);

    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(PICTURE, 0)),
        "the click selected the image"
    );
    assert_eq!(menu_labels(&tools, cx), IMAGE_MENU);

    // The picture can be cut and copied; there is nothing to paste.
    assert!(!item(&tools, "Cut", cx).is_disabled());
    assert!(!item(&tools, "Copy", cx).is_disabled());
    assert!(item(&tools, "Paste", cx).is_disabled());
    assert!(item(&tools, "Paste as Plain Text", cx).is_disabled());
    for title in ["Size", "Align", "Description…", "Insert", "Delete Image"] {
        assert!(!item(&tools, title, cx).is_disabled(), "{title}");
    }
    // The rows are the commands: they name them and show their keys.
    assert_eq!(item(&tools, "Cut", cx).action_name(), Some("editor::Cut"));
    assert_eq!(
        item(&tools, "Description…", cx).action_name(),
        Some("editor_tools::EditImageDescription")
    );
    let delete = item(&tools, "Delete Image", cx);
    assert_eq!(delete.action_name(), Some("editor::Backspace"));
    assert!(delete.is_danger());
    assert_eq!(
        delete.explicit_key_hint().map(KeyHint::text),
        hint(&editor_actions::Backspace, cx)
    );
    assert!(delete.explicit_key_hint().is_some());

    // Size: a step, the presets, the picture's own size. Half the column
    // is what it has.
    assert_eq!(
        submenu_labels(&tools, "Size", cx),
        [
            "Larger",
            "Smaller",
            "-",
            "25 %",
            "50 %",
            "75 %",
            "100 %",
            "Original Size"
        ]
    );
    assert_eq!(checked(&tools, "Size", cx), ["50 %"]);
    assert!(dimmed(&tools, "Size", cx).is_empty());
    assert_eq!(
        item(&tools, "Size > Larger", cx)
            .explicit_key_hint()
            .map(KeyHint::text),
        hint(&editor_actions::IncreaseTextSize, cx)
    );
    assert_eq!(
        item(&tools, "Size > 25 %", cx).action_name(),
        Some("editor::ImageWidth25")
    );
    // Align: an image stands left, in the middle or right.
    assert_eq!(
        submenu_labels(&tools, "Align", cx),
        ["Left", "Centre", "Right"]
    );
    assert_eq!(checked(&tools, "Align", cx), ["Centre"]);
    // Insert begins with the two ways to a line beside the picture.
    assert_eq!(
        submenu_labels(&tools, "Insert", cx),
        [
            "Paragraph Above",
            "Paragraph Below",
            "-",
            "Divider",
            "Page Break",
            "Image…",
            "Today's Date",
            "Link"
        ]
    );
    assert!(dimmed(&tools, "Insert", cx).is_empty());

    // A preset, and the check follows.
    choose("25 %", cx);
    assert_eq!(active(&tools, cx), None, "choosing closes the menu");
    assert!(editor_focused(&editor, cx));
    assert_eq!(width(&editor, PICTURE, cx), 250);
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(checked(&tools, "Size", cx), ["25 %"]);

    // One step larger: no preset has that width.
    choose("Larger", cx);
    assert_eq!(width(&editor, PICTURE, cx), 333);
    right_click_picture(&editor, PICTURE, cx);
    assert!(checked(&tools, "Size", cx).is_empty());

    // Its own size is its size on the page: 400 points of the 451 an A4
    // page has between its margins, in flow view too.
    choose("Original Size", cx);
    let original = editor.read_with(cx, |editor, _| {
        let (_, image) = editor.selected_image().unwrap();
        editor.state().original_image_width(&image).thousandths()
    });
    assert_eq!(width(&editor, PICTURE, cx), original);
    assert_eq!(original, 886);
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(checked(&tools, "Size", cx), ["Original Size"]);

    // Where it stands.
    choose("Right", cx);
    assert_eq!(alignment(&editor, PICTURE, cx), Alignment::Right);
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(checked(&tools, "Align", cx), ["Right"]);
    choose("Left", cx);
    assert_eq!(alignment(&editor, PICTURE, cx), Alignment::Left);

    // Every choice was one undo step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(alignment(&editor, PICTURE, cx), Alignment::Right);
    cx.simulate_keystrokes("cmd-z cmd-z");
    assert_eq!(width(&editor, PICTURE, cx), 333);

    // Delete Image takes the block out; undo brings it back.
    right_click_picture(&editor, PICTURE, cx);
    choose("Delete Image", cx);
    assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph"]);
    assert_eq!(text(&editor, cx), "Before the picture\nAfter the picture");
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(image_at(&editor, PICTURE, cx).id, photo.id);
}

#[gpui::test]
fn the_menu_of_an_image_from_the_keyboard_and_with_the_hosts_rows(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);
    tools.update(cx, |tools, _| {
        tools.set_text_menu_extension(|request, _, _| {
            vec![ContextMenuItem::new(
                format!("Export Block {}", request.block),
                |_, _| {},
            )]
        })
    });

    // The key of the context menu, with the caret on the image: the menu
    // hangs under the picture, its first row marked.
    cx.simulate_keystrokes("ctrl-enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ContextMenu));
    let picture = editor
        .read_with(cx, |editor, _| editor.image_bounds(PICTURE))
        .unwrap();
    let menu = text_menu(&tools, cx);
    let bounds = menu.read_with(cx, |menu, _| menu.bounds().unwrap());
    assert!(bounds.top() >= picture.bottom() - px(1.), "{bounds:?}");
    assert!(
        (bounds.left() - picture.left()).abs() < px(2.),
        "{bounds:?}"
    );
    assert_eq!(
        menu.read_with(cx, |menu, _| menu
            .selected_item()
            .map(|item| item.title().to_string())),
        Some("Cut".to_string())
    );
    assert!(editor_focused(&editor, cx));

    // The host's rows come before what ends the menu.
    let labels = menu_labels(&tools, cx);
    let tail = &labels[labels.len() - 6..];
    assert_eq!(
        tail,
        [
            "Find in Document",
            "Select All",
            "-",
            "Export Block 1",
            "-",
            "Delete Image"
        ]
    );

    // With the keys alone: down to "Description…" and take it.
    cx.simulate_keystrokes("down down down down enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    cx.simulate_input("From the keyboard");
    cx.simulate_keystrokes("enter");
    assert_eq!(description(&editor, PICTURE, cx), "From the keyboard");
}

#[gpui::test]
fn a_size_the_page_does_not_allow_is_dimmed(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    // Twice as high as wide: at the whole column it would be taller than
    // a page.
    let tall = images.add_png(300, 600);
    let widest = EditorState::new(Document::new())
        .max_image_width(&tall)
        .thousandths();
    assert!((760..790).contains(&widest), "{widest}");
    let document = Document::from_blocks(vec![
        Block::paragraph("Before the picture"),
        picture(&tall, widest, Alignment::Left),
        Block::paragraph("After the picture"),
    ]);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(document, EditorConfig::full(), Some(&images), cx);

    // At its widest: no step larger, and no "100 %".
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(dimmed(&tools, "Size", cx), ["Larger", "100 %"]);
    assert!(checked(&tools, "Size", cx).is_empty());
    choose("75 %", cx);
    assert_eq!(width(&editor, PICTURE, cx), 750);

    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(dimmed(&tools, "Size", cx), ["100 %"]);
    assert_eq!(checked(&tools, "Size", cx), ["75 %"]);
    // "Larger" stops at the widest the page allows.
    choose("Larger", cx);
    assert_eq!(width(&editor, PICTURE, cx), widest);

    // At the smallest step there is no step smaller.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| state.set_image_width(PICTURE, ImageWidth::new(ImageWidth::STEPS[0])),
            cx,
        )
    });
    cx.run_until_parked();
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(dimmed(&tools, "Size", cx), ["Smaller", "100 %"]);
    dismiss(cx);

    // The format menu leaves out what the context menu dims.
    cx.simulate_keystrokes("cmd-.");
    let rows = labels(&tools, cx);
    assert!(rows.contains(&"75 %".to_string()), "{rows:?}");
    assert!(!rows.contains(&"100 %".to_string()), "{rows:?}");
}

#[gpui::test]
fn paste_is_offered_for_a_picture_only_the_provider_sees(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(around(&photo), EditorConfig::full(), Some(&images), cx);

    // Nothing anywhere: dimmed, on text and on the image.
    right_click_at(in_text(&editor, AFTER, 3, cx), cx);
    assert!(item(&tools, "Paste", cx).is_disabled());
    dismiss(cx);
    right_click_picture(&editor, PICTURE, cx);
    assert!(item(&tools, "Paste", cx).is_disabled());
    dismiss(cx);

    // A picture GPUI's clipboard does not show (a file copied in Finder):
    // Paste would take it, so Paste is offered. There is no text in it.
    images.set_pasteboard(Some(NewImageSource::Bytes(FakeImages::png(64, 48))));
    caret(&editor, AFTER, 3, cx);
    cx.simulate_keystrokes("ctrl-enter");
    assert!(!item(&tools, "Paste", cx).is_disabled());
    assert!(item(&tools, "Paste as Plain Text", cx).is_disabled());
    choose("Paste", cx);
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "paragraph", "image"],
        "the picture came in"
    );
    assert_eq!(image_at(&editor, 3, cx).pixels, (64, 48));

    // Nothing again: dimmed again.
    images.set_pasteboard(None);
    right_click_picture(&editor, PICTURE, cx);
    assert!(item(&tools, "Paste", cx).is_disabled());
    dismiss(cx);

    // Text on the clipboard: both, as before.
    cx.write_to_clipboard(ClipboardItem::new_string("words".into()));
    right_click_picture(&editor, PICTURE, cx);
    assert!(!item(&tools, "Paste", cx).is_disabled());
    assert!(!item(&tools, "Paste as Plain Text", cx).is_disabled());
    dismiss(cx);

    // An editor that cannot be changed pastes nothing.
    images.set_pasteboard(Some(NewImageSource::Bytes(FakeImages::png(64, 48))));
    editor.update(cx, |editor, cx| editor.set_read_only(true, cx));
    right_click_picture(&editor, PICTURE, cx);
    assert!(item(&tools, "Paste", cx).is_disabled());
    assert!(item(&tools, "Paste as Plain Text", cx).is_disabled());
}

#[gpui::test]
fn the_menus_of_text_and_of_a_divider_beside_an_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let document = Document::from_blocks(vec![
        Block::paragraph("Before the picture"),
        picture(&photo, 500, Alignment::Center),
        Block::paragraph("After the picture"),
        Block::new(BlockKind::Divider),
        Block::paragraph("The end"),
    ]);
    const DIVIDER: usize = 3;
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(document, EditorConfig::full(), Some(&images), cx);

    // A caret in text: nothing to cut or copy, nothing of an image, and a
    // picture can be inserted.
    caret(&editor, AFTER, 5, cx);
    cx.simulate_keystrokes("ctrl-enter");
    let labels = menu_labels(&tools, cx);
    for title in ["Size", "Description…", "Delete Image"] {
        assert!(!labels.contains(&title.to_string()), "{title} on text");
    }
    assert!(labels.contains(&"Turn Into".to_string()));
    assert!(item(&tools, "Cut", cx).is_disabled());
    assert!(item(&tools, "Copy", cx).is_disabled());
    assert_eq!(
        submenu_labels(&tools, "Insert", cx),
        ["Divider", "Page Break", "Image…", "Today's Date", "Link"]
    );
    assert!(!item(&tools, "Insert > Image…", cx).is_disabled());
    assert_eq!(
        submenu_labels(&tools, "Align", cx),
        ["Left", "Centre", "Right", "Justify"]
    );
    dismiss(cx);

    // A divider: the two ways to a line beside it, nothing to turn it
    // into, and it is not an image.
    caret(&editor, DIVIDER, 0, cx);
    cx.simulate_keystrokes("ctrl-enter");
    let labels = menu_labels(&tools, cx);
    for title in ["Size", "Description…", "Delete Image"] {
        assert!(!labels.contains(&title.to_string()), "{title} on a divider");
    }
    for title in ["Turn Into", "Align", "Text Size"] {
        assert!(
            item(&tools, title, cx).is_disabled(),
            "{title} on a divider"
        );
    }
    assert!(item(&tools, "Copy", cx).is_disabled());
    assert_eq!(
        submenu_labels(&tools, "Insert", cx),
        [
            "Paragraph Above",
            "Paragraph Below",
            "-",
            "Divider",
            "Page Break",
            "Image…",
            "Today's Date",
            "Link"
        ]
    );
    choose("Paragraph Below", cx);
    assert_eq!(
        kinds(&editor, cx),
        [
            "paragraph",
            "image",
            "paragraph",
            "divider",
            "paragraph",
            "paragraph"
        ]
    );
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(DIVIDER + 1, 0))
    );
    cx.simulate_keystrokes("cmd-z");

    // A selection that runs over the picture is text with a picture in
    // it: a right-click on the picture keeps it and opens the menu of a
    // selection.
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_selection(Selection::new(
                    Position::new(BEFORE, 3),
                    Position::new(AFTER, 5),
                ))
            },
            cx,
        )
    });
    cx.run_until_parked();
    right_click_picture(&editor, PICTURE, cx);
    assert_eq!(
        selection(&editor, cx),
        Selection::new(Position::new(BEFORE, 3), Position::new(AFTER, 5))
    );
    let labels = menu_labels(&tools, cx);
    for title in ["Size", "Description…", "Delete Image"] {
        assert!(!labels.contains(&title.to_string()), "{title} on a range");
    }
    assert!(labels.contains(&"Bold".to_string()));
    assert!(!item(&tools, "Copy", cx).is_disabled());
    assert_eq!(
        submenu_labels(&tools, "Insert", cx),
        ["Divider", "Page Break", "Image…", "Today's Date", "Link"],
        "no caret on the picture: a new line would replace the selection"
    );
    dismiss(cx);
}

#[gpui::test]
fn a_paragraph_above_an_image_that_is_the_first_block(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let document = Document::from_blocks(vec![
        picture(&photo, 500, Alignment::Center),
        Block::paragraph("After the picture"),
    ]);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(document, EditorConfig::full(), Some(&images), cx);

    right_click_picture(&editor, 0, cx);
    let above = item(&tools, "Insert > Paragraph Above", cx);
    let below = item(&tools, "Insert > Paragraph Below", cx);
    assert_eq!(above.action_name(), Some("editor::InsertSoftBreak"));
    assert_eq!(below.action_name(), Some("editor::Newline"));
    assert_eq!(
        above.explicit_key_hint().map(KeyHint::text).as_deref(),
        Some("⇧↩")
    );
    assert_eq!(
        below.explicit_key_hint().map(KeyHint::text).as_deref(),
        Some("↩")
    );
    choose("Paragraph Above", cx);
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(0, 0)),
        "the caret is in the new first paragraph"
    );
    // One undo step.
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx), ["image", "paragraph"]);
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(0, 0))
    );

    // The key is the keymap's: changed, the row shows the new one, and
    // the row still does what it did.
    assert_eq!(
        rebind(
            &editor_actions::InsertSoftBreak,
            "shift-enter",
            Some("ctrl-o"),
            cx
        ),
        1
    );
    right_click_picture(&editor, 0, cx);
    assert_eq!(
        item(&tools, "Insert > Paragraph Above", cx)
            .explicit_key_hint()
            .map(KeyHint::text)
            .as_deref(),
        Some("⌃O")
    );
    choose("Paragraph Above", cx);
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(0, 0))
    );
    cx.simulate_input("A line before everything");
    assert_eq!(
        text(&editor, cx),
        "A line before everything\n\nAfter the picture"
    );
}

#[gpui::test]
fn the_menu_of_an_image_that_cannot_be_changed(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = described(&images.add_png(400, 300), "A lighthouse");
    let (Opened { tools, editor, .. }, cx) = open_pictures(
        around(&photo),
        EditorConfig::full().read_only(true),
        Some(&images),
        cx,
    );

    right_click_picture(&editor, PICTURE, cx);
    // The menu keeps its shape; only what reads is left to choose.
    assert_eq!(menu_labels(&tools, cx), IMAGE_MENU);
    for title in [
        "Cut",
        "Paste",
        "Paste as Plain Text",
        "Size",
        "Align",
        "Description…",
        "Insert",
        "Delete Image",
    ] {
        assert!(
            item(&tools, title, cx).is_disabled(),
            "{title} when read-only"
        );
    }
    for title in ["Copy", "Find in Document", "Select All"] {
        assert!(
            !item(&tools, title, cx).is_disabled(),
            "{title} when read-only"
        );
    }
    dismiss(cx);

    // No format menu either, as for text.
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), None);
    assert_eq!(image_at(&editor, PICTURE, cx), {
        let mut image = photo.clone();
        image.width = ImageWidth::new(500);
        image
    });
}

// ---------------------------------------------------------------------------
// "Image" in the insert menu
// ---------------------------------------------------------------------------

/// A folder for the picture files of one test, removed when it ends.
struct Files(PathBuf);

impl Files {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "innerword-editor-tools-images-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("a temporary folder");
        Files(dir)
    }

    /// Writes a PNG of that size and returns its path.
    fn png(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, FakeImages::png(width, height)).expect("a picture file");
        path
    }
}

impl Drop for Files {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).ok();
    }
}

#[gpui::test]
fn slash_then_image_inserts_the_chosen_file(cx: &mut TestAppContext) {
    setup(cx);
    let files = Files::new("slash");
    let images = FakeImages::new();
    let document = paragraphs(&["First", "", "Last"]);
    let (
        Opened {
            tools,
            editor,
            reached,
        },
        cx,
    ) = open_pictures(document, EditorConfig::full(), Some(&images), cx);

    // After "Page break", and found by what people call a picture.
    caret(&editor, 1, 0, cx);
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    let rows = labels(&tools, cx);
    let at = rows.iter().position(|row| row == "Image").expect("Image");
    assert_eq!(rows[at - 1], "Page break");
    assert_eq!(rows[at + 1], "Link");
    for word in ["photo", "picture", "screenshot", "figure", "image"] {
        cx.simulate_keystrokes("escape");
        caret(&editor, 1, 0, cx);
        editor.update(cx, |editor, cx| {
            editor.edit(
                |state| {
                    state.select_range(state.document().block_range(1));
                    state.backspace()
                },
                cx,
            )
        });
        cx.run_until_parked();
        cx.simulate_input("/");
        cx.simulate_input(word);
        assert_eq!(selected(&tools, cx).as_deref(), Some("Image"), "{word}");
    }

    // Taking the row removes the slash and the query and asks for files;
    // the empty paragraph becomes the image.
    images.choose_beforehand(vec![files.png("harbour.png", 320, 200)]);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(active(&tools, cx), None);
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);
    assert_eq!(text(&editor, cx), "First\n\nLast", "the slash is gone");
    assert_eq!(image_at(&editor, 1, cx).pixels, (320, 200));
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(1, 0)),
        "the new image is selected"
    );
    assert!(editor_focused(&editor, cx));
    assert!(reached.borrow().is_empty(), "the editor took the command");

    // After text and a space: the slash goes, the text stays, and the
    // picture comes after that block.
    caret(&editor, 0, 5, cx);
    cx.simulate_input(" /ima");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Image"));
    images.choose_beforehand(vec![files.png("boat.png", 100, 100)]);
    cx.simulate_keystrokes("tab");
    cx.run_until_parked();
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );
    assert_eq!(text(&editor, cx), "First \n\n\nLast");
    assert_eq!(image_at(&editor, 1, cx).pixels, (100, 100));

    // A cancelled panel: the slash is gone and nothing came in.
    caret(&editor, 3, 4, cx);
    cx.simulate_input(" /image");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(text(&editor, cx), "First \n\n\nLast ");
    assert_eq!(
        kinds(&editor, cx),
        ["paragraph", "image", "image", "paragraph"]
    );
}

#[gpui::test]
fn image_is_not_in_the_insert_menu_of_an_editor_that_takes_no_pictures(cx: &mut TestAppContext) {
    setup(cx);
    // Nobody to store a picture.
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(paragraphs(&[""]), EditorConfig::full(), None, cx);
    cx.simulate_input("/");
    assert_eq!(active(&tools, cx), Some(ToolKind::Insert));
    assert!(!labels(&tools, cx).contains(&"Image".to_string()));
    cx.simulate_input("ima");
    assert!(labels(&tools, cx).is_empty());
    cx.simulate_keystrokes("escape");

    // A provider arrives: the next menu has the row.
    let images = FakeImages::new();
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.select_range(state.document().block_range(0));
                state.backspace()
            },
            cx,
        );
        editor.set_image_provider(Some(images.clone()), cx)
    });
    cx.run_until_parked();
    cx.simulate_input("/");
    assert!(labels(&tools, cx).contains(&"Image".to_string()));
    // The row shows the key the user gave the command.
    assert_eq!(row(&tools, "Image", cx).key_hint, None);
    cx.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            "cmd-alt-i",
            editor_actions::InsertImage,
            Some("Editor"),
        )])
    });
    cx.run_until_parked();
    assert_eq!(row(&tools, "Image", cx).key_hint.as_deref(), Some("⌥⌘I"));
}

// ---------------------------------------------------------------------------
// The format menu on an image
// ---------------------------------------------------------------------------

fn marked(tools: &Entity<EditorTools>, cx: &mut VisualTestContext) -> Vec<String> {
    rows(tools, cx)
        .into_iter()
        .filter(|row| row.mark == Mark::Check)
        .map(|row| row.label)
        .collect()
}

#[gpui::test]
fn the_format_menu_on_an_image_is_the_images(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) = open_around(&photo, &images, cx);

    cx.simulate_keystrokes("cmd-.");
    assert_eq!(active(&tools, cx), Some(ToolKind::Format));
    let all = rows(&tools, cx);
    let shown: Vec<(&str, bool)> = all
        .iter()
        .map(|row| (row.label.as_str(), row.header))
        .collect();
    assert_eq!(
        shown,
        [
            ("Size", true),
            ("Larger", false),
            ("Smaller", false),
            ("25 %", false),
            ("50 %", false),
            ("75 %", false),
            ("100 %", false),
            ("Original size", false),
            ("Align", true),
            ("Align left", false),
            ("Align centre", false),
            ("Align right", false),
            ("Image", true),
            ("Description…", false),
            ("Delete image", false),
        ]
    );
    assert_eq!(marked(&tools, cx), ["50 %", "Align centre"]);
    // The rows show the keys of their commands.
    assert_eq!(
        row(&tools, "Larger", cx).key_hint,
        hint(&editor_actions::IncreaseTextSize, cx)
    );
    assert_eq!(
        row(&tools, "Align right", cx).key_hint,
        hint(&editor_actions::AlignRight, cx)
    );
    assert_eq!(
        row(&tools, "Delete image", cx).key_hint,
        hint(&editor_actions::Backspace, cx)
    );
    assert_eq!(row(&tools, "Description…", cx).key_hint, None);
    // Under the picture.
    let picture = editor
        .read_with(cx, |editor, _| editor.image_bounds(PICTURE))
        .unwrap();
    let menu = bounds("tools-menu", cx);
    assert!(menu.top() >= picture.bottom(), "{menu:?} {picture:?}");

    // Filtered and taken: a preset.
    cx.simulate_input("75");
    assert_eq!(selected(&tools, cx).as_deref(), Some("75 %"));
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), None);
    assert!(editor_focused(&editor, cx));
    assert_eq!(width(&editor, PICTURE, cx), 750);
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(PICTURE, 0))
    );

    // Where it stands, and the marks follow.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("right");
    cx.simulate_keystrokes("enter");
    assert_eq!(alignment(&editor, PICTURE, cx), Alignment::Right);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(marked(&tools, cx), ["75 %", "Align right"]);

    // Its own size, and a step from there.
    cx.simulate_input("original");
    cx.simulate_keystrokes("enter");
    assert_eq!(width(&editor, PICTURE, cx), 886);
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(marked(&tools, cx), ["Original size", "Align right"]);
    cx.simulate_input("smaller");
    cx.simulate_keystrokes("enter");
    assert_eq!(width(&editor, PICTURE, cx), 800);

    // "Description…" opens the popover; what is typed there is marked on
    // the row the next time.
    cx.simulate_keystrokes("cmd-.");
    cx.simulate_input("desc");
    assert_eq!(selected(&tools, cx).as_deref(), Some("Description…"));
    cx.simulate_keystrokes("enter");
    assert_eq!(active(&tools, cx), Some(ToolKind::ImageDescription));
    cx.simulate_input("A harbour");
    cx.simulate_keystrokes("enter");
    assert_eq!(description(&editor, PICTURE, cx), "A harbour");
    cx.simulate_keystrokes("cmd-.");
    assert_eq!(marked(&tools, cx), ["Align right", "Description…"]);

    // With the pointer: a click on a row (the list shows ten rows; the
    // filter brings the last one into view).
    cx.simulate_input("delete");
    assert_eq!(labels(&tools, cx), ["Delete image"]);
    click("menu-row-0", cx);
    assert_eq!(active(&tools, cx), None);
    assert_eq!(kinds(&editor, cx), ["paragraph", "paragraph"]);
    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&editor, cx), ["paragraph", "image", "paragraph"]);

    // In text the menu is the one of text again.
    caret(&editor, BEFORE, 3, cx);
    cx.simulate_keystrokes("cmd-.");
    let text_rows = labels(&tools, cx);
    assert!(text_rows.contains(&"Bold".to_string()));
    assert!(!text_rows.contains(&"Delete image".to_string()));
    assert!(!text_rows.contains(&"Original size".to_string()));
}

#[gpui::test]
fn the_format_bar_never_comes_up_for_a_selected_image(cx: &mut TestAppContext) {
    setup(cx);
    let images = FakeImages::new();
    let photo = images.add_png(400, 300);
    let (Opened { tools, editor, .. }, cx) =
        open_pictures(around(&photo), EditorConfig::full(), Some(&images), cx);

    // A click on the picture selects it: a selection made with the
    // pointer, and not one the bar is for.
    let at = on_picture(&editor, PICTURE, cx);
    click_at(at, cx);
    assert_eq!(
        selection(&editor, cx),
        Selection::caret(Position::new(PICTURE, 0))
    );
    cx.executor()
        .advance_clock(innerword_editor_tools::FORMAT_BAR_DELAY * 4);
    cx.run_until_parked();
    assert!(!tools.read_with(cx, |tools, _| tools.format_bar_visible()));
}
