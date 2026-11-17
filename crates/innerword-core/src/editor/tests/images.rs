//! Images in the editing engine: what every void block does and an image
//! must go on doing, and what is an image's own (its commands, deletion
//! that selects it first, a paragraph above a void block, copy and cut).

use super::*;
use crate::block::{Image, ImageId, ImageWidth};
use crate::settings::{DocumentSettings, Margins};

fn image_id(n: u8) -> ImageId {
    ImageId::from_bytes([n; 16])
}

/// A landscape picture, wider than a page's column at its own size.
fn pic(n: u8) -> Image {
    Image::new(image_id(n), (800, 600), 72)
}

/// A picture that is small on the page: 200 x 100 points.
fn small(n: u8) -> Image {
    Image::new(image_id(n), (400, 200), 144)
}

/// A screenshot of a phone: at the width of the column it would be taller
/// than a page.
fn phone() -> Image {
    Image::new(image_id(99), (1170, 2532), 144)
}

fn img(n: u8) -> Block {
    Block::image(pic(n))
}

fn divider() -> Block {
    Block::new(BlockKind::Divider)
}

fn page_break() -> Block {
    Block::new(BlockKind::PageBreak)
}

fn ids(e: &EditorState) -> Vec<BlockId> {
    e.document().blocks().iter().map(|b| b.id()).collect()
}

fn image_at(e: &EditorState, index: usize) -> &Image {
    e.document().blocks()[index]
        .as_image()
        .unwrap_or_else(|| panic!("block {index} is not an image: {:?}", kinds(e)))
}

fn is_image(e: &EditorState, index: usize) -> bool {
    e.document().blocks()[index].kind().is_image()
}

/// Runs `command`, which has to change the document, and checks that it is
/// exactly one undo step: one undo gives back the document, the selection
/// and every block's stamp, one redo what the command left.
fn one_step(e: &mut EditorState, command: impl FnOnce(&mut EditorState) -> bool) {
    e.break_undo_group();
    let before = (e.document().clone(), e.selection(), e.undo_depth());
    let stamps = |e: &EditorState| -> Vec<(BlockId, u64)> {
        e.document()
            .blocks()
            .iter()
            .map(|b| (b.id(), b.revision()))
            .collect()
    };
    let stamps_before = stamps(e);
    assert!(command(e), "the command says it changed nothing");
    check(e);
    assert_ne!(e.document(), &before.0, "the document did not change");
    assert_eq!(e.undo_depth(), before.2 + 1, "not one undo step");
    let after = (e.document().clone(), e.selection());
    let stamps_after = stamps(e);
    assert!(e.undo());
    check(e);
    assert_eq!(e.document(), &before.0, "undo: document");
    assert_eq!(e.selection(), before.1, "undo: selection");
    assert_eq!(stamps(e), stamps_before, "undo: stamps");
    assert!(e.redo());
    check(e);
    assert_eq!(e.document(), &after.0, "redo: document");
    assert_eq!(e.selection(), after.1, "redo: selection");
    assert_eq!(stamps(e), stamps_after, "redo: stamps");
}

/// Runs `command`, which has to leave everything as it is.
fn nothing(e: &mut EditorState, command: impl FnOnce(&mut EditorState) -> bool) {
    let before = (
        e.document().clone(),
        e.selection(),
        e.undo_depth(),
        e.revision(),
    );
    assert!(!command(e), "the command says it changed something");
    check(e);
    assert_eq!(e.document(), &before.0);
    assert_eq!(e.selection(), before.1);
    assert_eq!(e.undo_depth(), before.2);
    assert_eq!(e.revision(), before.3);
}

// ----- what every void block does, on an image --------------------------------------------

#[test]
fn the_arrows_stop_on_an_image_once() {
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    e.set_caret(p(0, 2));
    e.move_caret(Movement::Right, false);
    assert_eq!(caret(&e), p(1, 0));
    assert_eq!(e.selected_image().map(|(i, _)| i), Some(1));
    e.move_caret(Movement::Right, false);
    assert_eq!(caret(&e), p(2, 0));
    assert_eq!(e.selected_image(), None);
    e.move_caret(Movement::Left, false);
    assert_eq!(caret(&e), p(1, 0));
    e.move_caret(Movement::Left, false);
    assert_eq!(caret(&e), p(0, 2));
    // It has one position: any offset is that one, and the ends of its
    // line are where the caret is.
    e.set_caret(p(1, 7));
    assert_eq!(caret(&e), p(1, 0));
    for m in [
        Movement::LineStart,
        Movement::LineEnd,
        Movement::BlockStart,
        Movement::BlockEnd,
    ] {
        e.move_caret(m, false);
        assert_eq!(caret(&e), p(1, 0), "{m:?}");
    }
    // Extending onto it takes it into the range.
    e.set_caret(p(0, 2));
    e.move_caret(Movement::Right, true);
    assert_eq!(e.selection(), Selection::new(p(0, 2), p(1, 0)));
    assert_eq!(e.touched_blocks(), 0..2);
    assert_eq!(e.selected_image(), None, "a range is not a selected image");
    // At the two ends of a document.
    let mut e = ed(vec![img(1), para("x"), img(2)]);
    e.move_caret(Movement::Left, false);
    assert_eq!(caret(&e), p(0, 0));
    e.move_caret(Movement::DocEnd, false);
    assert_eq!(caret(&e), p(2, 0));
    e.move_caret(Movement::Right, false);
    assert_eq!(caret(&e), p(2, 0));
    e.move_caret(Movement::DocStart, false);
    assert_eq!(caret(&e), p(0, 0));
}

#[test]
fn typing_and_enter_on_an_image_start_a_paragraph_below() {
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    let image = ids(&e)[1];
    e.set_caret(p(1, 0));
    typ(&mut e, "x");
    assert_eq!(texts(&e), ["a", "", "x", "b"]);
    assert!(is_image(&e, 1));
    assert_eq!(kinds(&e)[2], BlockKind::Paragraph);
    assert_eq!(ids(&e)[1], image);
    assert_eq!(caret(&e), p(2, 1));
    e.undo();
    assert_eq!(texts(&e), ["a", "", "b"]);
    assert_eq!(caret(&e), p(1, 0));

    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.insert_paragraph_break());
    assert_eq!(texts(&e), ["a", "", "", "b"]);
    assert!(is_image(&e, 1));
    assert_eq!(kinds(&e)[2], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(2, 0));
    // The paragraph is a plain one whatever the image is: no alignment
    // of the image's, nothing of its width.
    let mut e = ed(vec![
        Block::image(small(1)).with_alignment(Alignment::Center),
    ]);
    e.insert_paragraph_break();
    assert_eq!(e.document().blocks()[1], {
        let mut plain = Block::paragraph("");
        plain.id = e.document().blocks()[1].id();
        plain
    });
    // Text with line breaks, pasted on an image, starts below it too.
    let mut e = ed(vec![img(1)]);
    e.paste_plain_text("one\ntwo");
    assert_eq!(texts(&e), ["", "one", "two"]);
    assert!(is_image(&e, 0));
    check(&e);
    assert_eq!(
        crate::editor::text_cmds::continuation_kind(&BlockKind::Image(pic(1)), true),
        BlockKind::Paragraph
    );
    assert_eq!(
        crate::editor::text_cmds::continuation_kind(&BlockKind::Image(pic(1)), false),
        BlockKind::Paragraph
    );
}

#[test]
fn backspace_and_delete_on_an_image_remove_it() {
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.backspace());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    assert_eq!(caret(&e), p(0, 1));

    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.delete_forward());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(caret(&e), p(1, 0));

    // The word and line deletions are the same two keys on an image.
    for delete in [
        EditorState::delete_word_backward as fn(&mut EditorState) -> bool,
        EditorState::delete_word_forward,
        EditorState::delete_to_line_start,
        EditorState::delete_to_line_end,
    ] {
        let mut e = ed(vec![para("a"), img(1), para("b")]);
        e.set_caret(p(1, 0));
        assert!(delete(&mut e));
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
        check(&e);
    }
    // The only block: an empty paragraph is left.
    for delete in [
        EditorState::backspace as fn(&mut EditorState) -> bool,
        EditorState::delete_forward,
    ] {
        let mut e = ed(vec![img(1)]);
        assert!(delete(&mut e));
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
        assert_eq!(caret(&e), p(0, 0));
        check(&e);
    }
    // Two pictures one above the other: each press takes one.
    let mut e = ed(vec![img(1), img(2), para("b")]);
    e.set_caret(p(1, 0));
    e.backspace();
    assert_eq!(e.document().block_count(), 2);
    assert_eq!(image_at(&e, 0).id, image_id(1));
    assert_eq!(caret(&e), p(0, 0));
    e.backspace();
    assert_eq!(texts(&e), ["b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
}

#[test]
fn a_range_that_starts_or_ends_on_an_image_includes_it() {
    // Ending on it.
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    e.set_selection(Selection::new(p(0, 1), p(1, 0)));
    let f = e.copy();
    assert_eq!(f.blocks().len(), 2);
    assert_eq!(f.blocks()[1].as_image(), Some(&pic(1)));
    assert!(e.has_something_to_copy());
    one_step(&mut e, |e| e.delete_selection());
    assert_eq!(texts(&e), ["a", "cd"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    assert_eq!(caret(&e), p(0, 1));
    // Starting on it.
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    e.set_selection(Selection::new(p(1, 0), p(2, 1)));
    let f = e.copy();
    assert_eq!(f.blocks().len(), 2);
    assert_eq!(f.blocks()[0].as_image(), Some(&pic(1)));
    assert_eq!(f.to_plain_text(), "\nc");
    one_step(&mut e, |e| e.delete_selection());
    assert_eq!(texts(&e), ["ab", "d"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    // From one image to another: both go, and what is between them.
    let mut e = ed(vec![para("ab"), img(1), para("x"), img(2), para("cd")]);
    e.set_selection(Selection::new(p(1, 0), p(3, 0)));
    assert_eq!(e.touched_blocks(), 1..4);
    one_step(&mut e, |e| e.delete_selection());
    assert_eq!(texts(&e), ["ab", "cd"]);
    // Everything selected, with an image last: nothing is left.
    let mut e = ed(vec![para("ab"), img(1)]);
    e.select_all();
    assert_eq!(e.copy().blocks().len(), 2);
    e.delete_selection();
    assert_eq!(texts(&e), [""]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    // Typing over such a range replaces the image with the text.
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    e.set_selection(Selection::new(p(0, 2), p(1, 0)));
    e.insert_text("!");
    assert_eq!(texts(&e), ["ab!", "cd"]);
    check(&e);
}

#[test]
fn paste_never_merges_an_image() {
    let image = Block::image(Image {
        width: ImageWidth::new(400),
        description: "A boat".into(),
        ..pic(1)
    })
    .with_alignment(Alignment::Right);
    let source = ed(vec![para("one"), image.clone(), para("two")]);
    // The image alone, into the middle of a line: the line is split
    // around it.
    let alone = source.document().block_fragment(1..2);
    let mut e = ed(vec![para("abcd")]);
    e.set_caret(p(0, 2));
    one_step(&mut e, |e| e.paste(&alone));
    assert_eq!(texts(&e), ["ab", "", "cd"]);
    assert_eq!(image_at(&e, 1), image.as_image().unwrap());
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Right);
    assert_eq!(caret(&e), p(2, 0));
    // With text on both sides: the text joins the line, the image stays a
    // block.
    let around = source.document().to_fragment();
    let mut e = ed(vec![para("abcd")]);
    e.set_caret(p(0, 2));
    one_step(&mut e, |e| e.paste(&around));
    assert_eq!(texts(&e), ["abone", "", "twocd"]);
    assert_eq!(image_at(&e, 1), image.as_image().unwrap());
    // On another image: below it, never in its place.
    let mut e = ed(vec![img(7), para("x")]);
    one_step(&mut e, |e| e.paste(&alone));
    assert_eq!(image_at(&e, 0).id, image_id(7));
    assert_eq!(image_at(&e, 1).id, image_id(1));
    // Into a code block an image is no text: nothing of it arrives but
    // the empty line it is in plain text.
    let mut e = ed(vec![kb(BlockKind::code(), "let x;")]);
    e.set_caret(p(0, 3));
    e.paste(&alone);
    assert_eq!(e.document().block_count(), 1);
    assert!(!e.document().has_images());
    check(&e);
    // As whole blocks (Vim's linewise put).
    let mut e = ed(vec![para("a"), para("b")]);
    one_step(&mut e, |e| e.insert_blocks_at(1, &alone));
    assert_eq!(texts(&e), ["a", "", "b"]);
    assert_eq!(image_at(&e, 1), image.as_image().unwrap());
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Right);
    assert_eq!(caret(&e), p(1, 0));
}

#[test]
fn moving_and_duplicating_blocks_take_an_image_along() {
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    let before = ids(&e);
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.move_blocks_up());
    assert_eq!(ids(&e), vec![before[1], before[0], before[2]]);
    assert_eq!(caret(&e), p(0, 0));
    assert_eq!(e.selected_image().map(|(i, _)| i), Some(0));
    assert!(!e.move_blocks_up());
    one_step(&mut e, |e| e.move_blocks_down());
    one_step(&mut e, |e| e.move_blocks_down());
    assert_eq!(ids(&e), vec![before[0], before[2], before[1]]);
    assert_eq!(caret(&e), p(2, 0));
    assert!(!e.move_blocks_down());
    // Text moved past an image leaves it where it is, whole.
    e.set_caret(p(1, 1));
    one_step(&mut e, |e| e.move_blocks_down());
    assert_eq!(ids(&e), vec![before[0], before[1], before[2]]);
    assert_eq!(image_at(&e, 1), &pic(1));

    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.duplicate_blocks());
    assert_eq!(image_at(&e, 1), image_at(&e, 2));
    assert_ne!(ids(&e)[1], ids(&e)[2]);
    assert_eq!(ids(&e)[1], before[1]);
    assert_eq!(caret(&e), p(2, 0));
    assert_eq!(e.document().images().count(), 2);
    one_step(&mut e, |e| e.delete_blocks());
    assert_eq!(e.document().images().count(), 1);
}

#[test]
fn counts_search_title_and_plain_text_see_an_empty_block() {
    let described = Block::image(Image {
        description: "Harbour boats at dusk".into(),
        ..pic(1)
    });
    let blocks = |void: Block| vec![void, para("Harbour text"), para("more words here")];
    let mut with_image = ed(blocks(described.clone()));
    let mut with_divider = ed(blocks(divider()));
    assert_eq!(with_image.stats(), with_divider.stats());
    assert_eq!(with_image.stats().words, 5);
    assert_eq!(
        with_image.document().stats(),
        with_divider.document().stats()
    );
    assert_eq!(
        with_image.document().plain_text(),
        "\nHarbour text\nmore words here"
    );
    assert_eq!(
        with_image.document().plain_text(),
        with_divider.document().plain_text()
    );
    assert_eq!(
        with_image.document().title().as_deref(),
        Some("Harbour text")
    );
    // The description is not text: nothing finds it, nothing replaces it.
    let found = with_image
        .document()
        .find_all("harbour", SearchOptions::default());
    assert_eq!(found, vec![TextRange::in_block(1, 0, 7)]);
    assert!(
        with_image
            .document()
            .find_all("dusk", SearchOptions::default())
            .is_empty()
    );
    assert_eq!(
        with_image.replace_all("Harbour", SearchOptions::default(), "Port"),
        1
    );
    assert_eq!(
        image_at(&with_image, 0).description,
        "Harbour boats at dusk"
    );
    // Selected with the text around it, it adds nothing to what is selected.
    with_image.select_all();
    with_divider.select_all();
    assert_eq!(with_image.selected_text(), "\nPort text\nmore words here");
    assert_eq!(
        with_image.selection_stats().words,
        with_divider.selection_stats().words
    );
    // An image alone has no title and no text.
    let alone = Document::from_blocks(vec![described]);
    assert_eq!(alone.title(), None);
    assert_eq!(alone.plain_text(), "");
    assert_eq!(alone.stats().words, 0);
    // No comment can be anchored to it.
    let mut e = EditorState::new(alone);
    assert!(e.add_comment("on a picture", "me", 1).is_none());
    assert_eq!(e.document().comment_count(), 0);
    // Formatting commands have nothing to format.
    nothing(&mut e, |e| e.toggle_bold());
    nothing(&mut e, |e| e.set_link("https://example.com"));
}

#[test]
fn adopt_takes_a_changed_image_as_a_changed_block() {
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    let stamps: Vec<u64> = e.document().blocks().iter().map(|b| b.revision()).collect();
    // The same document: nothing is adopted.
    let same = e.document().clone();
    assert!(!e.adopt(same).changed());
    assert_eq!(e.undo_depth(), 0);
    // Elsewhere the image got another width and a description.
    let mut elsewhere = EditorState::new(e.document().clone());
    assert!(elsewhere.set_image_width(1, ImageWidth::new(250)));
    assert!(elsewhere.set_image_description(1, "A boat"));
    let adopted = e.adopt(elsewhere.document().clone());
    assert!(adopted.undo_step && !adopted.settings);
    assert_eq!(image_at(&e, 1).width, ImageWidth::new(250));
    assert_eq!(image_at(&e, 1).description, "A boat");
    // The caret stays on the picture, the blocks around it are the blocks
    // they were, and the change is one step.
    assert_eq!(caret(&e), p(1, 0));
    let after: Vec<u64> = e.document().blocks().iter().map(|b| b.revision()).collect();
    assert_eq!((after[0], after[2]), (stamps[0], stamps[2]));
    assert_ne!(after[1], stamps[1]);
    assert_eq!(e.undo_depth(), 1);
    e.undo();
    assert_eq!(image_at(&e, 1), &pic(1));
    // Another picture in the same place is a change too.
    let mut other = e.document().blocks().to_vec();
    other[1] = {
        let mut b = Block::image(pic(2));
        b.id = other[1].id();
        b
    };
    let target = Document::from_blocks(other);
    assert!(e.adopt(target).undo_step);
    assert_eq!(image_at(&e, 1).id, image_id(2));
    check(&e);
}

// ----- deleting beside an image selects it first --------------------------------------------

#[test]
fn backspace_below_an_image_selects_it_first() {
    // From a paragraph with text: only the caret moves.
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(2, 0));
    nothing_but_the_caret(&mut e, |e| e.backspace());
    assert_eq!(caret(&e), p(1, 0));
    assert_eq!(e.selected_image().map(|(i, _)| i), Some(1));
    // The second press deletes it.
    one_step(&mut e, |e| e.backspace());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    assert_eq!(caret(&e), p(0, 1));
    e.undo();
    assert_eq!(caret(&e), p(1, 0));
    assert!(is_image(&e, 1));

    // From an empty paragraph: the paragraph goes in the same step, the
    // image stays and is selected.
    let mut e = ed(vec![para("a"), img(1), para(""), para("b")]);
    let image = ids(&e)[1];
    e.set_caret(p(2, 0));
    one_step(&mut e, |e| e.backspace());
    assert_eq!(texts(&e), ["a", "", "b"]);
    assert_eq!(ids(&e)[1], image);
    assert_eq!(image_at(&e, 1), &pic(1));
    assert_eq!(caret(&e), p(1, 0));
    one_step(&mut e, |e| e.backspace());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);

    // The image is the first block: the same.
    let mut e = ed(vec![img(1), para("b")]);
    e.set_caret(p(1, 0));
    nothing_but_the_caret(&mut e, |e| e.backspace());
    assert_eq!(caret(&e), p(0, 0));
    e.backspace();
    assert_eq!(texts(&e), ["b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);

    // Deleting a word or a line backwards at the start of the paragraph is
    // Backspace there.
    for delete in [
        EditorState::delete_word_backward as fn(&mut EditorState) -> bool,
        EditorState::delete_to_line_start,
    ] {
        let mut e = ed(vec![img(1), para("b")]);
        e.set_caret(p(1, 0));
        nothing_but_the_caret(&mut e, delete);
        assert_eq!(caret(&e), p(0, 0));
    }

    // What Backspace does to the block itself comes first: a list item
    // becomes a paragraph, an indented paragraph loses its indent, and only
    // then the picture above is reached.
    let mut e = ed(vec![
        img(1),
        kb(BlockKind::BulletItem, "item").with_indent(1),
    ]);
    e.set_caret(p(1, 0));
    assert!(e.backspace());
    assert_eq!(e.document().blocks()[1].indent(), 0);
    assert!(e.backspace());
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(1, 0));
    assert!(!e.backspace());
    assert_eq!(caret(&e), p(0, 0));
    assert_eq!(e.document().block_count(), 2);
    let mut e = ed(vec![img(1), para("x").with_indent(1)]);
    e.set_caret(p(1, 0));
    assert!(e.backspace());
    assert_eq!(caret(&e), p(1, 0));
    assert!(!e.backspace());
    assert_eq!(caret(&e), p(0, 0));

    // Inside the text nothing is different.
    let mut e = ed(vec![img(1), para("bc")]);
    e.set_caret(p(1, 1));
    assert!(e.backspace());
    assert_eq!(texts(&e), ["", "c"]);
    assert_eq!(caret(&e), p(1, 0));
    // A selection is deleted as a selection, image or not.
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_selection(Selection::new(p(1, 0), p(2, 0)));
    assert!(e.backspace());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
}

#[test]
fn delete_above_an_image_selects_it_first() {
    // From a block with text: only the caret moves.
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(0, 1));
    nothing_but_the_caret(&mut e, |e| e.delete_forward());
    assert_eq!(caret(&e), p(1, 0));
    // The second press deletes it; the caret is where Delete on a void
    // block leaves it.
    one_step(&mut e, |e| e.delete_forward());
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    assert_eq!(caret(&e), p(1, 0));

    // From an empty paragraph: it goes in the same step.
    let mut e = ed(vec![para("a"), para(""), img(1), para("b")]);
    let image = ids(&e)[2];
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.delete_forward());
    assert_eq!(texts(&e), ["a", "", "b"]);
    assert_eq!(ids(&e)[1], image);
    assert_eq!(caret(&e), p(1, 0));
    assert_eq!(e.selected_image().map(|(i, _)| i), Some(1));
    one_step(&mut e, |e| e.delete_forward());
    assert_eq!(texts(&e), ["a", "b"]);

    // An empty first paragraph above an image that is the last block.
    let mut e = ed(vec![para(""), img(1)]);
    one_step(&mut e, |e| e.delete_forward());
    assert_eq!(e.document().block_count(), 1);
    assert_eq!(caret(&e), p(0, 0));
    assert!(is_image(&e, 0));

    // The end of any kind of block with text.
    for kind in [BlockKind::heading(2), BlockKind::Quote, BlockKind::code()] {
        let mut e = ed(vec![kb(kind.clone(), "t"), img(1)]);
        e.set_caret(p(0, 1));
        nothing_but_the_caret(&mut e, |e| e.delete_forward());
        assert_eq!(caret(&e), p(1, 0), "{kind:?}");
        assert_eq!(kinds(&e)[0], kind);
    }
    // Deleting a word or a line forwards at the end of the block is Delete
    // there.
    for delete in [
        EditorState::delete_word_forward as fn(&mut EditorState) -> bool,
        EditorState::delete_to_line_end,
    ] {
        let mut e = ed(vec![para("a"), img(1)]);
        e.set_caret(p(0, 1));
        nothing_but_the_caret(&mut e, delete);
        assert_eq!(caret(&e), p(1, 0));
    }
    // Before the end of the text nothing is different.
    let mut e = ed(vec![para("ab"), img(1)]);
    e.set_caret(p(0, 1));
    assert!(e.delete_forward());
    assert_eq!(texts(&e), ["a", ""]);
    assert_eq!(caret(&e), p(0, 1));
}

#[test]
fn a_divider_still_goes_at_the_first_press() {
    for void in [divider(), page_break()] {
        let mut e = ed(vec![para("a"), void.clone(), para("b")]);
        e.set_caret(p(2, 0));
        one_step(&mut e, |e| e.backspace());
        assert_eq!(texts(&e), ["a", "b"]);
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
        assert_eq!(caret(&e), p(1, 0));

        let mut e = ed(vec![para("a"), void.clone(), para("b")]);
        e.set_caret(p(0, 1));
        one_step(&mut e, |e| e.delete_forward());
        assert_eq!(texts(&e), ["a", "b"]);
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
        assert_eq!(caret(&e), p(0, 1));

        // An empty paragraph between the two: the divider goes, the empty
        // line stays, as before.
        let mut e = ed(vec![void.clone(), para("")]);
        e.set_caret(p(1, 0));
        e.backspace();
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    }
}

/// Runs `command`, which has to move the caret and change nothing else:
/// no undo step, no change reported, and it says so.
fn nothing_but_the_caret(e: &mut EditorState, command: impl FnOnce(&mut EditorState) -> bool) {
    let before = (
        e.document().clone(),
        e.selection(),
        e.undo_depth(),
        e.revision(),
    );
    e.take_changes();
    assert!(!command(e), "the command says the document changed");
    check(e);
    assert_eq!(e.document(), &before.0);
    assert_ne!(e.selection(), before.1, "the caret did not move");
    assert_eq!(e.undo_depth(), before.2);
    assert_eq!(e.revision(), before.3);
    assert_eq!(e.take_changes(), None);
}

#[test]
fn selecting_an_image_first_ends_the_typing_before_it() {
    // Text typed and deleted again in the line below a picture, then one
    // press more: the picture is selected, and typing after that is a new
    // step that undo takes out by itself.
    let mut e = ed(vec![img(1), para("")]);
    no_rules(&mut e);
    e.set_caret(p(1, 0));
    typ(&mut e, "ab");
    e.move_caret(Movement::BlockStart, false);
    e.toggle_bold();
    assert!(e.pending_attrs().is_some());
    assert!(!e.backspace());
    assert_eq!(caret(&e), p(0, 0));
    assert_eq!(
        e.pending_attrs(),
        None,
        "the caret moved: nothing is pending"
    );
    typ(&mut e, "x");
    assert_eq!(texts(&e), ["", "x", "ab"]);
    assert!(!e.document().blocks()[1].attrs_at(0).unwrap().bold);
    e.undo();
    assert_eq!(texts(&e), ["", "ab"]);
    assert_eq!(caret(&e), p(0, 0));
}

// ----- inserting ---------------------------------------------------------------------------

#[test]
fn insert_image_into_an_empty_paragraph_turns_it_into_the_image() {
    let mut e = ed(vec![
        para("a"),
        para("")
            .with_indent(2)
            .with_text_size(Some(30.0))
            .with_alignment(Alignment::Justify),
        para("b"),
    ]);
    let before = ids(&e);
    e.set_caret(p(1, 0));
    e.toggle_bold();
    one_step(&mut e, |e| e.insert_image(small(1), Alignment::Center));
    assert_eq!(ids(&e), before, "the paragraph became the image");
    let b = &e.document().blocks()[1];
    assert_eq!(b.as_image(), Some(&small(1)));
    assert_eq!(b.alignment(), Alignment::Center);
    assert_eq!((b.indent(), b.text_size()), (0, None));
    assert_eq!(caret(&e), p(1, 0));
    assert_eq!(e.selected_image(), Some((1, &small(1))));
    assert_eq!(e.pending_attrs(), None);
    // Typing then starts a paragraph below it.
    typ(&mut e, "x");
    assert_eq!(texts(&e), ["a", "", "x", "b"]);
    assert!(!e.document().blocks()[2].attrs_at(0).unwrap().bold);
    // An empty document.
    let mut e = EditorState::default();
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Left));
    assert_eq!(e.document().block_count(), 1);
    assert!(is_image(&e, 0));
    assert_eq!(e.document().blocks()[0].alignment(), Alignment::Left);
}

#[test]
fn insert_image_goes_after_the_block_of_the_selection() {
    // After a block with text, wherever the caret is in it.
    for offset in [0, 1, 2] {
        let mut e = ed(vec![para("ab"), para("cd")]);
        let before = ids(&e);
        e.set_caret(p(0, offset));
        one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
        assert_eq!(texts(&e), ["ab", "", "cd"], "{offset}");
        assert_eq!(image_at(&e, 1), &pic(1));
        assert_eq!(e.document().blocks()[1].alignment(), Alignment::Center);
        assert_eq!(caret(&e), p(1, 0));
        assert_eq!(vec![ids(&e)[0], ids(&e)[2]], before);
    }
    // An empty block that is not a paragraph is not replaced.
    let mut e = ed(vec![kb(BlockKind::heading(1), ""), para("x")]);
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
    assert_eq!(kinds(&e)[0], BlockKind::heading(1));
    assert!(is_image(&e, 1));
    // At the end of the document.
    let mut e = ed(vec![para("ab")]);
    e.set_caret(p(0, 2));
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Right));
    assert_eq!(texts(&e), ["ab", ""]);
    assert_eq!(caret(&e), p(1, 0));
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Right);

    // With a range selected: after the block the range ends in. The text
    // of the range stays.
    let mut e = ed(vec![para("ab"), para("cd"), para("ef")]);
    e.set_selection(Selection::new(p(0, 1), p(1, 1)));
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
    assert_eq!(texts(&e), ["ab", "cd", "", "ef"]);
    assert_eq!(caret(&e), p(2, 0));
    // The same range made backwards.
    let mut e = ed(vec![para("ab"), para("cd"), para("ef")]);
    e.set_selection(Selection::new(p(1, 1), p(0, 1)));
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
    assert_eq!(texts(&e), ["ab", "cd", "", "ef"]);
    assert_eq!(caret(&e), p(2, 0));
    // Whole lines selected (the range ends at the start of the next block,
    // which it does not touch): after the last line of the range.
    let mut e = ed(vec![para("ab"), para("cd"), para("ef")]);
    e.set_selection(Selection::new(p(0, 0), p(1, 0)));
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
    assert_eq!(texts(&e), ["ab", "", "cd", "ef"]);
    assert_eq!(caret(&e), p(1, 0));
    // Only a caret in an empty paragraph makes that paragraph the image:
    // a range that ends in one is a range, and nothing is replaced.
    let mut e = ed(vec![para("ab"), para(""), para("ef")]);
    e.set_selection(Selection::new(p(0, 1), p(1, 0)));
    one_step(&mut e, |e| e.insert_image(pic(1), Alignment::Center));
    assert_eq!(texts(&e), ["ab", "", "", "ef"]);
    assert!(is_image(&e, 1));
    assert_eq!(kinds(&e)[2], BlockKind::Paragraph);
    assert_eq!(caret(&e), p(1, 0));

    // With the caret on another image: below it, and the new one is the
    // selected one.
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.insert_image(pic(2), Alignment::Center));
    assert_eq!(image_at(&e, 1).id, image_id(1));
    assert_eq!(image_at(&e, 2).id, image_id(2));
    assert_eq!(
        e.selected_image().map(|(i, image)| (i, image.id)),
        Some((2, image_id(2)))
    );
    // One after the other, as several files chosen at once arrive: in
    // order, the last one selected, and one undo step when grouped.
    let mut e = ed(vec![para("a"), para("b")]);
    one_step(&mut e, |e| {
        e.grouped(|e| (3..6).all(|n| e.insert_image(pic(n), Alignment::Center)))
    });
    let order: Vec<ImageId> = e.document().images().map(|(_, i)| i.id).collect();
    assert_eq!(order, vec![image_id(3), image_id(4), image_id(5)]);
    assert_eq!(texts(&e), ["a", "", "", "", "b"]);
    assert_eq!(caret(&e), p(3, 0));

    // An image cannot be justified: asked for, it stands left.
    let mut e = ed(vec![para("a")]);
    e.insert_image(pic(1), Alignment::Justify);
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Left);
    // What an image is given is what it keeps, repaired.
    let mut e = ed(vec![para("a")]);
    e.insert_image(
        Image {
            pixels: (0, 5),
            density: 3,
            description: " two\nlines ".into(),
            ..pic(1)
        },
        Alignment::Center,
    );
    let stored = image_at(&e, 1);
    assert_eq!(stored.pixels, (1, 5));
    assert_eq!(stored.density, Image::MIN_DENSITY);
    assert_eq!(stored.description, "two lines");
    check(&e);
}

#[test]
fn insert_images_at_a_boundary_between_blocks() {
    let two = || vec![(pic(1), Alignment::Center), (small(2), Alignment::Right)];
    for (index, expected) in [
        (0, ["", "", "a", "b"]),
        (1, ["a", "", "", "b"]),
        (2, ["a", "b", "", ""]),
        (99, ["a", "b", "", ""]),
    ] {
        let mut e = ed(vec![para("a"), para("b")]);
        let before = ids(&e);
        e.set_caret(p(1, 1));
        one_step(&mut e, |e| e.insert_images_at(index, two()));
        assert_eq!(texts(&e), expected, "{index}");
        let at = index.min(2);
        assert_eq!(image_at(&e, at), &pic(1));
        assert_eq!(image_at(&e, at + 1), &small(2));
        assert_eq!(e.document().blocks()[at].alignment(), Alignment::Center);
        assert_eq!(e.document().blocks()[at + 1].alignment(), Alignment::Right);
        assert_eq!(caret(&e), p(at + 1, 0), "the caret is on the last one");
        let text_ids: Vec<BlockId> = e
            .document()
            .blocks()
            .iter()
            .filter(|b| !b.is_void())
            .map(|b| b.id())
            .collect();
        assert_eq!(text_ids, before);
    }
    // Nothing is replaced, an empty paragraph included.
    let mut e = EditorState::default();
    one_step(&mut e, |e| {
        e.insert_images_at(0, vec![(pic(1), Alignment::Justify)])
    });
    assert_eq!(e.document().block_count(), 2);
    assert!(is_image(&e, 0));
    assert_eq!(e.document().blocks()[0].alignment(), Alignment::Left);
    assert_eq!(kinds(&e)[1], BlockKind::Paragraph);
    // Nothing to insert.
    nothing(&mut e, |e| e.insert_images_at(0, Vec::new()));
}

#[test]
fn the_selected_image_is_the_one_the_caret_is_on() {
    let mut e = ed(vec![para("a"), img(1), divider(), img(2)]);
    assert_eq!(e.selected_image(), None);
    assert!(!e.has_something_to_copy());
    e.set_caret(p(1, 0));
    assert_eq!(e.selected_image(), Some((1, &pic(1))));
    assert!(e.has_something_to_copy());
    e.set_caret(p(2, 0));
    assert_eq!(e.selected_image(), None, "a divider is no image");
    assert!(!e.has_something_to_copy());
    e.set_caret(p(3, 0));
    assert_eq!(e.selected_image(), Some((3, &pic(2))));
    // A range over an image is a range.
    e.set_selection(Selection::new(p(1, 0), p(3, 0)));
    assert_eq!(e.selected_image(), None);
    assert!(e.has_something_to_copy());
    e.set_selection(Selection::new(p(0, 0), p(0, 1)));
    assert!(e.has_something_to_copy());
}

// ----- width -------------------------------------------------------------------------------

#[test]
fn the_width_of_an_image_is_set_stepped_and_reset() {
    let mut e = ed(vec![para("a"), Block::image(small(1)), para("b")]);
    e.set_caret(p(0, 1));
    let selection = e.selection();
    assert_eq!(image_at(&e, 1).width, ImageWidth::FULL);
    one_step(&mut e, |e| {
        e.set_image_width(1, ImageWidth::from_percent(50))
    });
    assert_eq!(image_at(&e, 1).width.thousandths(), 500);
    assert_eq!(e.selection(), selection, "the selection stays where it is");
    nothing(&mut e, |e| {
        e.set_image_width(1, ImageWidth::from_percent(50))
    });
    // Everything else of the image stays.
    assert_eq!(
        image_at(&e, 1),
        &Image {
            width: ImageWidth::new(500),
            ..small(1)
        }
    );
    // Not an image, not a block.
    nothing(&mut e, |e| e.set_image_width(0, ImageWidth::MIN));
    nothing(&mut e, |e| e.set_image_width(3, ImageWidth::MIN));
    nothing(&mut e, |e| e.set_image_width(usize::MAX, ImageWidth::MIN));
    nothing(&mut e, |e| e.step_image_width(0, 1));
    nothing(&mut e, |e| e.step_image_width(9, -1));
    nothing(&mut e, |e| e.set_image_original_size(2));
    nothing(&mut e, |e| e.set_image_description(0, "text"));
    nothing(&mut e, |e| e.set_image_description(9, "text"));

    // The steps, up and down, and the two ends.
    one_step(&mut e, |e| e.step_image_width(1, 1));
    assert_eq!(image_at(&e, 1).width.thousandths(), 600);
    one_step(&mut e, |e| e.step_image_width(1, -2));
    assert_eq!(image_at(&e, 1).width.thousandths(), 400);
    one_step(&mut e, |e| e.step_image_width(1, 100));
    assert_eq!(image_at(&e, 1).width, ImageWidth::FULL);
    nothing(&mut e, |e| e.step_image_width(1, 1));
    one_step(&mut e, |e| e.step_image_width(1, i32::MIN));
    assert_eq!(image_at(&e, 1).width.thousandths(), 100);
    nothing(&mut e, |e| e.step_image_width(1, -1));
    nothing(&mut e, |e| e.step_image_width(1, 0));
    // From between two steps the first step lands on the neighbour.
    e.set_image_width(1, ImageWidth::new(620));
    e.step_image_width(1, 1);
    assert_eq!(image_at(&e, 1).width.thousandths(), 667);
    e.set_image_width(1, ImageWidth::new(620));
    e.step_image_width(1, -1);
    assert_eq!(image_at(&e, 1).width.thousandths(), 600);

    // Original size: 200 points in a column of 451.
    let original = e.original_image_width(&small(1));
    assert_eq!(
        original,
        small(1).original_width(&e.document().settings().page)
    );
    assert_eq!(original.thousandths(), 443);
    one_step(&mut e, |e| e.set_image_original_size(1));
    assert_eq!(image_at(&e, 1).width, original);
    nothing(&mut e, |e| e.set_image_original_size(1));
    // A picture wider than the column has the whole column as its
    // original size.
    assert_eq!(e.original_image_width(&pic(1)), ImageWidth::FULL);
    assert_eq!(e.max_image_width(&pic(1)), ImageWidth::FULL);
    check(&e);
}

#[test]
fn an_image_is_never_set_taller_than_a_page() {
    let mut e = ed(vec![para("a")]);
    let page = e.document().settings().page;
    let limit = phone().max_width(&page);
    assert_eq!(e.max_image_width(&phone()), limit);
    // 1170 x 2532 in a column of 451 x 697 points: 71 % of the column.
    assert_eq!(limit.thousandths(), 713);
    let height = |width: ImageWidth| width.fraction() * page.content_size().0 * phone().aspect();
    assert!(height(limit) <= page.content_size().1);
    assert!(height(ImageWidth::new(limit.thousandths() + 2)) > page.content_size().1);

    // Inserted at the whole column it is stored as it came (the insert
    // does not size it); asked for the whole column, the limit is stored.
    e.insert_image(
        Image {
            width: ImageWidth::new(300),
            ..phone()
        },
        Alignment::Center,
    );
    one_step(&mut e, |e| e.set_image_width(1, ImageWidth::FULL));
    assert_eq!(image_at(&e, 1).width, limit);
    // A step up from there changes nothing, and neither does asking again.
    nothing(&mut e, |e| e.step_image_width(1, 1));
    nothing(&mut e, |e| e.step_image_width(1, 5));
    nothing(&mut e, |e| e.set_image_width(1, ImageWidth::FULL));
    nothing(&mut e, |e| e.set_image_width(1, ImageWidth::new(900)));
    // Down goes to the step below the limit, up again stops at the limit
    // and not at the step above it.
    one_step(&mut e, |e| e.step_image_width(1, -1));
    assert_eq!(image_at(&e, 1).width.thousandths(), 667);
    one_step(&mut e, |e| e.step_image_width(1, 1));
    assert_eq!(image_at(&e, 1).width, limit);
    // Below the limit every width is stored as asked.
    one_step(&mut e, |e| e.set_image_width(1, ImageWidth::new(712)));
    assert_eq!(image_at(&e, 1).width.thousandths(), 712);
    // Its original size (585 points) is wider than the column: the limit.
    assert_eq!(e.original_image_width(&phone()), limit);
    one_step(&mut e, |e| e.set_image_original_size(1));
    assert_eq!(image_at(&e, 1).width, limit);
}

#[test]
fn a_width_the_page_no_longer_allows_stays_until_the_next_width_command() {
    let wider_margins = || {
        let mut settings = DocumentSettings::default();
        settings.page.margins = Margins {
            top: 200.0,
            bottom: 200.0,
            ..Margins::default()
        };
        settings
    };
    let start = || {
        let mut e = ed(vec![para("a")]);
        e.insert_image(phone(), Alignment::Center);
        e.set_image_width(1, ImageWidth::FULL);
        e.break_undo_group();
        let before = image_at(&e, 1).width;
        assert_eq!(before.thousandths(), 713);
        // The text area gets lower: the picture would have to be narrower.
        assert!(e.set_settings(wider_margins()));
        let limit = e.max_image_width(&phone());
        assert_eq!(limit.thousandths(), 451);
        // The stored width stays as it is; the layout draws it capped.
        assert_eq!(image_at(&e, 1).width, before);
        check(&e);
        (e, limit)
    };
    // The next step down goes from what is drawn, not from what is stored.
    let (mut e, limit) = start();
    one_step(&mut e, |e| e.step_image_width(1, -1));
    assert_eq!(image_at(&e, 1).width.thousandths(), 400);
    assert!(image_at(&e, 1).width <= limit);
    // A step up cannot make it wider: it stores what is drawn.
    let (mut e, limit) = start();
    one_step(&mut e, |e| e.step_image_width(1, 1));
    assert_eq!(image_at(&e, 1).width, limit);
    nothing(&mut e, |e| e.step_image_width(1, 1));
    // So does setting the width it already has in the document.
    let (mut e, limit) = start();
    one_step(&mut e, |e| e.set_image_width(1, ImageWidth::new(713)));
    assert_eq!(image_at(&e, 1).width, limit);
    // And "original size".
    let (mut e, limit) = start();
    one_step(&mut e, |e| e.set_image_original_size(1));
    assert_eq!(image_at(&e, 1).width, limit);
    // Undo gives the stored width back: settings are not part of undo.
    e.undo();
    assert_eq!(image_at(&e, 1).width.thousandths(), 713);
    check(&e);
}

#[test]
fn the_description_of_an_image() {
    let mut e = ed(vec![para("a"), img(1)]);
    let selection = e.selection();
    one_step(&mut e, |e| {
        e.set_image_description(1, "The harbour at dusk")
    });
    assert_eq!(image_at(&e, 1).description, "The harbour at dusk");
    assert_eq!(e.selection(), selection);
    nothing(&mut e, |e| {
        e.set_image_description(1, "The harbour at dusk")
    });
    // It is kept as an image keeps one: a single trimmed line.
    nothing(&mut e, |e| {
        e.set_image_description(1, "  The harbour at dusk\n")
    });
    one_step(&mut e, |e| {
        e.set_image_description(1, " two\r\nlines\there ")
    });
    assert_eq!(image_at(&e, 1).description, "two lines here");
    let long = "é".repeat(Image::MAX_DESCRIPTION_CHARS + 50);
    one_step(&mut e, |e| e.set_image_description(1, &long));
    assert_eq!(
        image_at(&e, 1).description.chars().count(),
        Image::MAX_DESCRIPTION_CHARS
    );
    // Removing it.
    one_step(&mut e, |e| e.set_image_description(1, "   "));
    assert_eq!(image_at(&e, 1).description, "");
    nothing(&mut e, |e| e.set_image_description(1, ""));
    // Nothing else of the image changed on the way.
    assert_eq!(image_at(&e, 1), &pic(1));
}

// ----- alignment and the commands for text -------------------------------------------------

#[test]
fn alignment_reaches_text_and_images_and_leaves_an_image_unjustified() {
    let mut e = ed(vec![para("a"), img(1), divider(), para("b"), page_break()]);
    e.select_all();
    one_step(&mut e, |e| e.set_alignment(Alignment::Center));
    let alignments = |e: &EditorState| -> Vec<Alignment> {
        e.document()
            .blocks()
            .iter()
            .map(|b| b.alignment())
            .collect()
    };
    assert_eq!(
        alignments(&e),
        [
            Alignment::Center,
            Alignment::Center,
            Alignment::Left,
            Alignment::Center,
            Alignment::Left
        ]
    );
    // Justified: the text is, the image stays as it is.
    one_step(&mut e, |e| e.set_alignment(Alignment::Justify));
    assert_eq!(
        alignments(&e),
        [
            Alignment::Justify,
            Alignment::Center,
            Alignment::Left,
            Alignment::Justify,
            Alignment::Left
        ]
    );
    // With the caret on the image.
    e.set_caret(p(1, 0));
    one_step(&mut e, |e| e.set_alignment(Alignment::Right));
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Right);
    assert_eq!(e.format_state().alignment, Choice::Same(Alignment::Right));
    nothing(&mut e, |e| e.set_alignment(Alignment::Right));
    nothing(&mut e, |e| e.set_alignment(Alignment::Justify));
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Right);
    one_step(&mut e, |e| e.set_alignment(Alignment::Left));
    assert_eq!(e.document().blocks()[1].alignment(), Alignment::Left);
    // On a divider there is still nothing to align.
    e.set_caret(p(2, 0));
    nothing(&mut e, |e| e.set_alignment(Alignment::Center));
}

#[test]
fn the_commands_for_text_leave_an_image_alone() {
    let image = Block::image(Image {
        width: ImageWidth::new(400),
        description: "A boat".into(),
        ..pic(1)
    })
    .with_alignment(Alignment::Center);
    let mut e = ed(vec![para("a"), image.clone(), para("b")]);
    let id = ids(&e)[1];
    let same = |e: &EditorState| {
        let b = &e.document().blocks()[1];
        assert_eq!(b.id(), id);
        assert_eq!(b.as_image(), image.as_image());
        assert_eq!(b.alignment(), Alignment::Center);
        assert_eq!((b.indent(), b.text_size()), (0, None));
        check(e);
    };
    e.select_all();
    assert!(e.step_text_size(1));
    same(&e);
    assert_eq!(e.document().blocks()[0].text_size(), Some(18.0));
    assert!(e.set_text_size(Some(30.0)));
    same(&e);
    assert!(e.indent());
    same(&e);
    assert!(e.set_block_kind(BlockKind::Quote));
    same(&e);
    assert!(e.set_block_kind(BlockKind::code()));
    same(&e);
    assert_eq!(e.document().block_count(), 3, "an image ends a run of code");
    e.select_all();
    e.toggle_bold();
    e.toggle_todo_checked();
    e.clear_formatting();
    same(&e);
    // With only the image selected they do nothing at all.
    e.set_caret(p(1, 0));
    nothing(&mut e, |e| e.step_text_size(1));
    nothing(&mut e, |e| e.set_text_size(Some(12.0)));
    nothing(&mut e, |e| e.indent());
    nothing(&mut e, |e| e.outdent());
    nothing(&mut e, |e| e.set_block_kind(BlockKind::heading(1)));
    nothing(&mut e, |e| e.set_code_language(Some("rust")));
    // No block is ever turned into an image by a change of kind: an image
    // comes from `insert_image`.
    e.select_all();
    nothing(&mut e, |e| e.set_block_kind(BlockKind::Image(pic(2))));
    e.set_caret(p(0, 0));
    nothing(&mut e, |e| e.set_block_kind(BlockKind::Image(pic(2))));
    same(&e);
    // The input rules that make a block from typed text do not see it.
    e.set_caret(p(1, 0));
    typ(&mut e, "# ");
    same(&e);
    assert_eq!(kinds(&e)[2], BlockKind::heading(1));
}

// ----- moving ------------------------------------------------------------------------------

#[test]
fn move_block_to_over_twenty_blocks_is_one_step() {
    let mut blocks: Vec<Block> = (0..22).map(|n| para(&n.to_string())).collect();
    blocks[1] = img(1);
    let mut e = ed(blocks);
    let before = ids(&e);
    let stamps: Vec<u64> = e.document().blocks().iter().map(|b| b.revision()).collect();
    e.set_caret(p(1, 0));
    e.take_changes();
    one_step(&mut e, |e| e.move_block_to(1, 22));
    // It is the last block now, the same block, and everything it passed
    // moved up by one, each the block it was.
    let mut expected = before.clone();
    let moved = expected.remove(1);
    expected.push(moved);
    assert_eq!(ids(&e), expected);
    assert_eq!(image_at(&e, 21), &pic(1));
    assert_eq!(texts(&e)[1], "2");
    assert_eq!(texts(&e)[20], "21");
    let mut expected_stamps = stamps.clone();
    let stamp = expected_stamps.remove(1);
    expected_stamps.push(stamp);
    let after: Vec<u64> = e.document().blocks().iter().map(|b| b.revision()).collect();
    assert_eq!(after, expected_stamps, "no block was touched, only moved");
    // The caret stayed on the picture.
    assert_eq!(caret(&e), p(21, 0));
    assert_eq!(e.selected_image().map(|(i, _)| i), Some(21));
    // What is reported is the blocks between the two places.
    assert_eq!(
        e.last_change().unwrap().blocks,
        Some(BlockSplice {
            start: 1,
            old_len: 21,
            new_len: 21
        })
    );
    // And back up, to before the first block.
    one_step(&mut e, |e| e.move_block_to(21, 0));
    let mut expected = before.clone();
    let moved = expected.remove(1);
    expected.insert(0, moved);
    assert_eq!(ids(&e), expected);
    assert_eq!(caret(&e), p(0, 0));
    // Into the middle, from above and from below.
    one_step(&mut e, |e| e.move_block_to(0, 11));
    assert_eq!(ids(&e)[10], before[1]);
    assert_eq!(caret(&e), p(10, 0));
    one_step(&mut e, |e| e.move_block_to(10, 4));
    assert_eq!(ids(&e)[4], before[1]);
    assert_eq!(
        e.last_change().unwrap().blocks,
        Some(BlockSplice {
            start: 4,
            old_len: 7,
            new_len: 7
        })
    );
    // One step each: four moves, four steps.
    assert_eq!(e.undo_depth(), 4);
    undo_all(&mut e);
    assert_eq!(ids(&e), before);
    assert_eq!(caret(&e), p(1, 0));
}

#[test]
fn move_block_to_where_the_block_is_changes_nothing() {
    let mut e = ed(vec![para("a"), img(1), para("b")]);
    e.set_caret(p(1, 0));
    // Before itself and after itself are the place it has.
    nothing(&mut e, |e| e.move_block_to(1, 1));
    nothing(&mut e, |e| e.move_block_to(1, 2));
    nothing(&mut e, |e| e.move_block_to(0, 0));
    nothing(&mut e, |e| e.move_block_to(0, 1));
    nothing(&mut e, |e| e.move_block_to(2, 2));
    nothing(&mut e, |e| e.move_block_to(2, 3));
    // Beyond the end is the end.
    nothing(&mut e, |e| e.move_block_to(2, 99));
    // No such block.
    nothing(&mut e, |e| e.move_block_to(3, 0));
    nothing(&mut e, |e| e.move_block_to(usize::MAX, 0));
    // A document of one block.
    let mut e = ed(vec![img(1)]);
    nothing(&mut e, |e| e.move_block_to(0, 0));
    nothing(&mut e, |e| e.move_block_to(0, 1));
    nothing(&mut e, |e| e.move_block_to(0, 7));
    // Beyond the end moves to the end.
    let mut e = ed(vec![img(1), para("a"), para("b")]);
    one_step(&mut e, |e| e.move_block_to(0, 99));
    assert_eq!(texts(&e), ["a", "b", ""]);
    assert!(is_image(&e, 2));
}

#[test]
fn move_block_to_takes_the_selection_with_the_text_it_is_in() {
    // A range in the blocks that are passed stays on its text.
    let mut e = ed(vec![img(1), para("abc"), para("def"), para("ghi")]);
    e.set_selection(Selection::new(p(1, 1), p(2, 2)));
    one_step(&mut e, |e| e.move_block_to(0, 3));
    assert_eq!(texts(&e), ["abc", "def", "", "ghi"]);
    assert_eq!(e.selection(), Selection::new(p(0, 1), p(1, 2)));
    assert_eq!(e.selected_text(), "bc\nde");
    one_step(&mut e, |e| e.move_block_to(2, 0));
    assert_eq!(e.selection(), Selection::new(p(1, 1), p(2, 2)));
    assert_eq!(e.selected_text(), "bc\nde");
    // A caret outside the blocks that move does not move.
    e.set_caret(p(3, 2));
    one_step(&mut e, |e| e.move_block_to(0, 2));
    assert_eq!(texts(&e), ["abc", "", "def", "ghi"]);
    assert_eq!(caret(&e), p(3, 2));
    // A block with text moves the same way, with the caret in it.
    e.set_caret(p(3, 1));
    one_step(&mut e, |e| e.move_block_to(3, 0));
    assert_eq!(texts(&e), ["ghi", "abc", "", "def"]);
    assert_eq!(caret(&e), p(0, 1));
    // Comment anchors travel with the text of the block.
    e.select_block(0);
    let thread = e.add_comment("note", "me", 1).unwrap();
    e.move_block_to(0, 4);
    assert_eq!(texts(&e), ["abc", "", "def", "ghi"]);
    assert_eq!(
        e.document().comment_ranges(thread),
        vec![TextRange::in_block(3, 0, 3)]
    );
    check(&e);
}

// ----- a paragraph above and below a void block ---------------------------------------------

#[test]
fn shift_enter_on_a_void_block_starts_a_paragraph_above_it() {
    for void in [img(1), divider(), page_break()] {
        let kind = void.kind().clone();
        // The only block of the document: Enter gives a paragraph below
        // it with the caret in it.
        let mut e = ed(vec![void.clone()]);
        let id = ids(&e)[0];
        one_step(&mut e, |e| e.insert_paragraph_break());
        assert_eq!(kinds(&e), vec![kind.clone(), BlockKind::Paragraph]);
        assert_eq!(caret(&e), p(1, 0));
        assert_eq!(ids(&e)[0], id);
        // shift-enter gives one above it with the caret in it: the void
        // block is block 1 now, and the same block.
        let mut e = ed(vec![void.clone()]);
        one_step(&mut e, |e| e.insert_soft_break());
        assert_eq!(kinds(&e), vec![BlockKind::Paragraph, kind.clone()]);
        assert_eq!(caret(&e), p(0, 0));
        assert_eq!(ids(&e)[1], id);
        assert_eq!(e.undo_depth(), 1);
        // Writing goes on in that paragraph.
        typ(&mut e, "Above");
        assert_eq!(texts(&e), ["Above", ""]);
        assert_eq!(kinds(&e)[1], kind);
        // One undo step takes the paragraph out again (the typing first).
        e.undo();
        e.undo();
        assert_eq!(kinds(&e), vec![kind.clone()]);
        assert_eq!(caret(&e), p(0, 0));
        assert!(!e.can_undo());

        // In the middle of a document both work the same way.
        let mut e = ed(vec![para("a"), void.clone(), para("b")]);
        let before = ids(&e);
        e.set_caret(p(1, 0));
        one_step(&mut e, |e| e.insert_soft_break());
        assert_eq!(texts(&e), ["a", "", "", "b"]);
        assert_eq!(
            kinds(&e),
            vec![
                BlockKind::Paragraph,
                BlockKind::Paragraph,
                kind.clone(),
                BlockKind::Paragraph
            ]
        );
        assert_eq!(caret(&e), p(1, 0));
        assert_eq!(vec![ids(&e)[0], ids(&e)[2], ids(&e)[3]], before);
        let mut e = ed(vec![para("a"), void.clone(), para("b")]);
        e.set_caret(p(1, 0));
        one_step(&mut e, |e| e.insert_paragraph_break());
        assert_eq!(
            kinds(&e),
            vec![
                BlockKind::Paragraph,
                kind.clone(),
                BlockKind::Paragraph,
                BlockKind::Paragraph
            ]
        );
        assert_eq!(caret(&e), p(2, 0));
        assert_eq!(vec![ids(&e)[0], ids(&e)[1], ids(&e)[3]], before);

        // The command behind both keys.
        let mut e = ed(vec![para("a"), void.clone(), para("b")]);
        e.set_caret(p(1, 0));
        one_step(&mut e, |e| e.insert_paragraph_beside_void(true));
        assert_eq!(kinds(&e)[2], kind);
        assert_eq!(caret(&e), p(1, 0));
        e.set_caret(p(2, 0));
        one_step(&mut e, |e| e.insert_paragraph_beside_void(false));
        assert_eq!(kinds(&e)[2], kind);
        assert_eq!(kinds(&e)[3], BlockKind::Paragraph);
        assert_eq!(texts(&e), ["a", "", "", "", "b"]);
        assert_eq!(caret(&e), p(3, 0));
        // The new paragraph is a plain one.
        assert_eq!(e.document().blocks()[1].alignment(), Alignment::Left);
        assert_eq!(e.document().blocks()[1].indent(), 0);
    }
}

#[test]
fn a_paragraph_beside_a_void_block_needs_the_caret_on_one() {
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    // In text.
    e.set_caret(p(0, 1));
    nothing(&mut e, |e| e.insert_paragraph_beside_void(true));
    nothing(&mut e, |e| e.insert_paragraph_beside_void(false));
    // A range, even one that ends on the image or lies between two.
    e.set_selection(Selection::new(p(0, 1), p(1, 0)));
    nothing(&mut e, |e| e.insert_paragraph_beside_void(true));
    nothing(&mut e, |e| e.insert_paragraph_beside_void(false));
    // In text shift-enter is the line break it always was.
    e.set_caret(p(0, 1));
    assert!(e.insert_soft_break());
    assert_eq!(texts(&e), ["a\nb", "", "cd"]);
    // Over a range that ends on an image it replaces the range, as typing
    // does: the image goes with the range.
    let mut e = ed(vec![para("ab"), img(1), para("cd")]);
    e.set_selection(Selection::new(p(0, 1), p(1, 0)));
    assert!(e.insert_soft_break());
    assert_eq!(texts(&e), ["a\n", "cd"]);
    check(&e);
}

// ----- copy and cut ------------------------------------------------------------------------

#[test]
fn copy_and_cut_of_a_selected_image_give_it_back_whole() {
    let image = Image {
        width: ImageWidth::new(333),
        description: "The harbour at dusk".into(),
        ..small(4)
    };
    let block = Block::image(image.clone()).with_alignment(Alignment::Right);
    let mut e = ed(vec![para("a"), block.clone(), para("b")]);
    e.set_caret(p(1, 0));
    assert!(e.has_something_to_copy());
    // Copy: a fragment of that one block. Nothing changes.
    let copied = e.copy();
    assert_eq!(copied.blocks().len(), 1);
    assert_eq!(copied.blocks()[0].as_image(), Some(&image));
    assert_eq!(copied.blocks()[0].alignment(), Alignment::Right);
    assert_eq!(copied.blocks()[0].id(), BlockId::UNASSIGNED);
    assert!(!copied.is_empty());
    assert!(!copied.is_inline());
    assert_eq!(e.document().block_count(), 3);
    assert_eq!(e.undo_depth(), 0);
    // It survives the clipboard's JSON.
    assert_eq!(Fragment::from_json(&copied.to_json()).unwrap(), copied);

    // Cut: the same fragment, and the block is gone in one step, the caret
    // where Backspace on the image leaves it.
    let mut cut = None;
    one_step(&mut e, |e| {
        cut = Some(e.cut());
        true
    });
    let cut = cut.unwrap();
    assert_eq!(cut, copied);
    assert_eq!(texts(&e), ["a", "b"]);
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph; 2]);
    assert_eq!(caret(&e), p(0, 1));

    // Paste puts it back equal: width, alignment, description.
    one_step(&mut e, |e| e.paste(&cut));
    let pasted: Vec<&Block> = e
        .document()
        .blocks()
        .iter()
        .filter(|b| b.kind().is_image())
        .collect();
    assert_eq!(pasted.len(), 1);
    assert_eq!(pasted[0].as_image(), Some(&image));
    assert_eq!(pasted[0].alignment(), Alignment::Right);
    assert_eq!(e.document().block_index(pasted[0].id()), Some(1));
    assert_eq!(texts(&e)[0], "a");
    // Pasted a second time it is a second block with the same picture.
    e.paste(&copied);
    assert_eq!(e.document().images().count(), 2);
    check(&e);

    // Cut of the only block leaves an empty paragraph.
    let mut e = ed(vec![block.clone()]);
    let f = e.cut();
    assert_eq!(f.blocks()[0].as_image(), Some(&image));
    assert_eq!(kinds(&e), vec![BlockKind::Paragraph]);
    assert_eq!(e.undo_depth(), 1);
    // Cut of the first block: the caret is at the start.
    let mut e = ed(vec![block.clone(), para("b")]);
    e.cut();
    assert_eq!(texts(&e), ["b"]);
    assert_eq!(caret(&e), p(0, 0));

    // A caret in text copies nothing and cuts nothing, as before.
    let mut e = ed(vec![para("a"), block, para("b")]);
    e.set_caret(p(0, 1));
    assert!(!e.has_something_to_copy());
    assert!(e.copy().blocks().is_empty());
    assert!(e.cut().blocks().is_empty());
    assert_eq!(e.document().block_count(), 3);
    assert_eq!(e.undo_depth(), 0);
    // And so does a caret on a divider.
    let mut e = ed(vec![para("a"), divider()]);
    e.set_caret(p(1, 0));
    assert!(!e.has_something_to_copy());
    assert!(e.cut().blocks().is_empty());
    assert_eq!(e.document().block_count(), 2);
}

#[test]
fn an_edited_document_with_images_round_trips_through_json() {
    let mut e = ed(vec![para("Text"), para("")]);
    e.set_caret(p(1, 0));
    e.insert_image(phone(), Alignment::Center);
    e.set_image_original_size(1);
    e.set_image_description(1, "A screenshot");
    e.insert_paragraph_beside_void(true);
    typ(&mut e, "Above");
    e.insert_images_at(0, vec![(small(2), Alignment::Right)]);
    let json = e.document().to_json();
    assert!(json.contains(r#""version":2"#));
    let back = Document::from_json(&json).unwrap();
    assert_eq!(&back, e.document());
    assert_eq!(back.to_json(), json);
    assert_eq!(back.images().count(), 2);
    undo_all(&mut e);
    assert!(!e.document().has_images());
    assert!(e.document().to_json().contains(r#""version":1"#));
}
