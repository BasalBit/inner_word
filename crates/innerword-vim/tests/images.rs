//! Images: blocks without text, like a divider, that no command may take
//! apart or lose by accident. `[image 3]` is written so only while the
//! block is exactly that picture (its width, its alignment, its
//! description), so every row here also says "and it is whole".

use crate::harness::*;
use innerword_core::{Alignment, ImageWidth};

#[test]
fn the_notation_reads_back_what_it_writes() {
    for n in 0..=9u8 {
        let written = if n == 0 {
            "|[image]".to_string()
        } else {
            format!("|[image {n}]")
        };
        let s = Session::new(&written);
        assert_eq!(s.buffer(), written);
        let block = &s.ed.document().blocks()[0];
        assert_eq!(block.kind(), picture(n).kind());
        assert_eq!(block.alignment(), picture(n).alignment());
        s.check_invariants().unwrap();
    }
    // The nine pictures differ from the plain one in everything.
    let image = picture(4);
    let image = image.as_image().unwrap();
    assert_eq!(image.width, ImageWidth::new(407));
    assert_eq!(image.description, "Picture 4");
    assert_eq!(image.density, 144);
    assert_eq!(picture(4).alignment(), Alignment::Right);
    assert_eq!(picture(3).alignment(), Alignment::Center);
    // A picture that is not what it was is not written as if it were.
    let mut s = Session::new("|[image 3]\nx");
    s.ed.set_image_width(0, ImageWidth::new(500));
    assert_eq!(s.buffer(), "|[image 3 changed]\nx");
    let mut s = Session::new("|[image 3]\nx");
    s.ed.set_alignment(Alignment::Left);
    assert_eq!(s.buffer(), "|[image 3 changed]\nx");
    let mut s = Session::new("|[image 3]\nx");
    s.ed.set_image_description(0, "");
    assert_eq!(s.buffer(), "|[image 3 changed]\nx");
    // Text that only looks like the notation is text.
    let s = Session::new("|[image 12]\n[images]");
    assert!(!s.ed.document().has_images());
    assert_eq!(s.buffer(), "|[image 12]\n[images]");
}

#[test]
fn moving_over_them() {
    run(&[
        c("a\n|[image 3]\nb", "l", "a\n|[image 3]\nb").bell(true),
        c("a\n|[image 3]\nb", "h", "a\n|[image 3]\nb").bell(true),
        c("a\n|[image 3]\nb", "$", "a\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "0", "a\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "^", "a\n|[image 3]\nb"),
        c("|a\n[image 3]\nb", "j", "a\n|[image 3]\nb"),
        c("|a\n[image 3]\nb", "jj", "a\n[image 3]\n|b"),
        c("a\n[image 3]\n|b", "kk", "|a\n[image 3]\nb"),
        c("|a\n[image 3]\nb", "w", "a\n|[image 3]\nb"),
        c("a\n[image 3]\n|b", "b", "a\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "w", "a\n[image 3]\n|b"),
        c("a\n|[image 3]\nb", "%", "a\n|[image 3]\nb").bell(true),
        c("a\n|[image 3]\nb", "fx", "a\n|[image 3]\nb").bell(true),
        c("|a\n[image 3]", "G", "a\n|[image 3]"),
        c("a\n|[image 3]", "gg", "|a\n[image 3]"),
        c("|[image 3]", "j", "|[image 3]").bell(true),
        c("|[image 3]\n[image 4]", "<Space>", "[image 3]\n|[image 4]"),
    ]);
}

#[test]
fn joining_stops_at_an_image() {
    run(&[
        // With an image as the next block nothing is joined.
        c("|a\n[image 3]\nb", "J", "|a\n[image 3]\nb").bell(true),
        c("|a\n[image 3]\nb", "gJ", "|a\n[image 3]\nb").bell(true),
        c("a\n|b\n[image 3]", "J", "a\n|b\n[image 3]").bell(true),
        c("x |a\n[image 3]\nb", "J", "x |a\n[image 3]\nb").bell(true),
        // On an image nothing is joined either.
        c("|[image 3]\nb", "J", "|[image 3]\nb").bell(true),
        c("a\n|[image 3]\nb", "J", "a\n|[image 3]\nb").bell(true),
        c("a\n|[image 3]\nb", "gJ", "a\n|[image 3]\nb").bell(true),
        c("|[image 3]\n[image 4]", "J", "|[image 3]\n[image 4]").bell(true),
        c("|[image 3]", "J", "|[image 3]").bell(true),
        // A divider beside an image does not make it go.
        c("|---\n[image 3]", "J", "|---\n[image 3]").bell(true),
        c("|[image 3]\n---", "J", "|[image 3]\n---").bell(true),
        // A count joins what there is before the image, and stops.
        c("|a\nb\n[image 3]\nc", "3J", "a| b\n[image 3]\nc"),
        c("|a\nb\n[image 3]\nc", "9J", "a| b\n[image 3]\nc"),
        c("|a\n---\n[image 3]\nc", "5J", "|a\n[image 3]\nc"),
        c("|a\n[image 3]\nb", "3J", "|a\n[image 3]\nb"),
        // From Visual mode and from the command line.
        c("|a\n[image 3]\nb", "VjjJ", "|a\n[image 3]\nb"),
        c("|a\nb\n[image 3]\nc", "VGJ", "a| b\n[image 3]\nc"),
        c("|a\n[image 3]\nb", ":j<CR>", "|a\n[image 3]\nb").bell(true),
        c("|a\n[image 3]\nb", ":%j<CR>", "|a\n[image 3]\nb"),
        c("|a\nb\n[image 3]\nc", ":%j<CR>", "|a b\n[image 3]\nc"),
        c("a\n|[image 3]\nb", ":j<CR>", "a\n|[image 3]\nb").bell(true),
        // After the image joining is what it was.
        c("a\n[image 3]\n|b\nc", "J", "a\n[image 3]\nb| c"),
        // And a divider still joins like an empty line.
        c("|a\n---\nb", "J", "|a\nb"),
    ]);
}

#[test]
fn deleting_yanking_and_putting_keep_them_whole() {
    run(&[
        c("a\n|[image 3]\nb", "dd", "a\n|b").regs(&[('"', "[image 3]\n")]),
        c("a\n|[image 3]\nb", "ddu", "a\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "yy", "a\n|[image 3]\nb")
            .regs(&[('"', "[image 3]\n"), ('0', "[image 3]\n")]),
        c("a\n|[image 3]\nb", "yyp", "a\n[image 3]\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "yyP", "a\n|[image 3]\n[image 3]\nb"),
        c("a\n|[image 3]\nb", "ddp", "a\nb\n|[image 3]"),
        c("a\n|[image 3]\nb", "ddkP", "|[image 3]\na\nb"),
        c("a\n|[image 3]\nb", "yyjp", "a\n[image 3]\nb\n|[image 3]"),
        c(
            "a\n|[image 3]\nb",
            "yy3p",
            "a\n[image 3]\n|[image 3]\n[image 3]\n[image 3]\nb",
        ),
        c("a\n|[image 3]\nb", "\"qddG\"qp", "a\nb\n|[image 3]"),
        c("|a\n[image 3]\nb", "yjGp", "a\n[image 3]\nb\n|a\n[image 3]"),
        c("|a\n[image 3]\nb", "dj", "|b").regs(&[('"', "a\n[image 3]\n")]),
        c("|a\n[image 3]\nb", "djp", "b\n|a\n[image 3]"),
        c(
            "|[image 3]\n[image 4]\nx",
            "ddp",
            "[image 4]\n|[image 3]\nx",
        ),
        c("|[image 3]", "dd", "|"),
        c("|[image 3]", "ddu", "|[image 3]"),
        c("|[image 3]", "ddP", "|[image 3]\n"),
        c("a\n|[image 3]\nb", "yyjVp", "a\n[image 3]\n|[image 3]"),
        c("|x\n[image 3]", "yljp", "x\n[image 3]\n|x"),
        c("|a\n[image 3]", "yyjp", "a\n[image 3]\n|a"),
        c("|a\n[image 3]", "yyjP", "a\n|a\n[image 3]"),
        // In Visual mode.
        c("a\n|[image 3]\nb", "vd", "a\n|b"),
        c("a\n|[image 3]\nb", "Vd", "a\n|b").regs(&[('"', "[image 3]\n")]),
        c("a\n|[image 3]\nb", "VdP", "a\n|[image 3]\nb"),
        c("a\n|[image 3]\nb", "Vyjp", "a\n[image 3]\nb\n|[image 3]"),
        c("|a\n[image 3]\nb", "vjjd", "|"),
        c("a\n|[image 3]\nb", "dip", "a\n|b"),
        c("|a\n[image 3]\nb", "dG", "|"),
        c("|a\n[image 3]\nb", "dGu", "|a\n[image 3]\nb"),
        c("|a\n[image 3]\nb", ":2d<CR>", "a\n|b"),
        c("|a\n[image 3]\nb", ":2m$<CR>", "a\nb\n|[image 3]"),
        c(
            "|a\n[image 3]\nb",
            ":2t0<CR>",
            "|[image 3]\na\n[image 3]\nb",
        ),
        // What changes text leaves an image as it is.
        c("a\n|[image 3]\nb", "x", "a\n|[image 3]\nb").bell(false),
        c("a\n|[image 3]\nb", "X", "a\n|[image 3]\nb"),
        c("|[image 3]", "rx", "|[image 3]").bell(true),
        c("|[image 3]", "~", "|[image 3]").bell(true),
        c("|[image 3]", ">>", "|[image 3]"),
        c("|[image 3]", "<lt><lt>", "|[image 3]"),
        c("|[image 3]", "gUU", "|[image 3]"),
        c("|[image 3]", "D", "|[image 3]"),
        c("|[image 3]", "diw", "|[image 3]"),
        c("|a\n[image 3]\nb", "VGgU", "|A\n[image 3]\nB"),
        c("|a\n[image 3]\nb", "VG>", "{1}|a\n[image 3]\n{1}b"),
        c("|a\n[image 3]\nb", ":%s/b/x/<CR>", "a\n[image 3]\n|x"),
    ]);
}

#[test]
fn writing_around_them() {
    run(&[
        // Typing on an image starts a paragraph after it.
        c("a\n|[image 3]\nb", "iX<Esc>", "a\n[image 3]\n|X\nb"),
        c("a\n|[image 3]\nb", "aX<Esc>", "a\n[image 3]\n|X\nb"),
        c("a\n|[image 3]\nb", "AX<Esc>", "a\n[image 3]\n|X\nb"),
        c("a\n|[image 3]\nb", "IX<Esc>", "a\n[image 3]\n|X\nb"),
        c("|[image 3]", "RX<Esc>", "[image 3]\n|X"),
        // A line below and a line above.
        c("|[image 3]", "oX<Esc>", "[image 3]\n|X"),
        c("|[image 3]", "OX<Esc>", "|X\n[image 3]"),
        c("a\n|[image 3]\nb", "OX<Esc>", "a\n|X\n[image 3]\nb"),
        // In Insert mode on a block without text, Enter starts the line
        // below it and shift-enter the line above it.
        c("a\n|[image 3]", "i<CR>X<Esc>", "a\n[image 3]\n|X"),
        c("|[image 3]", "i<S-CR>X<Esc>", "|X\n[image 3]"),
        c("a\n|[image 3]\nb", "i<S-CR>X<Esc>", "a\n|X\n[image 3]\nb"),
        c("|---", "i<S-CR>X<Esc>", "|X\n---"),
        c("|===", "i<S-CR>X<Esc>", "|X\n==="),
        c("|[image 3]", "i<S-CR>X<Esc>u", "|[image 3]"),
        c(
            "|[image 3]\n[image 4]",
            "i<S-CR>X<Esc>jj.",
            "X\n[image 3]\n|X\n[image 4]",
        ),
        // Changing the line takes the image for text.
        c("a\n|[image 3]\nb", "ccX<Esc>", "a\n|X\nb"),
        c("a\n|[image 3]\nb", "SX<Esc>", "a\n|X\nb"),
        c("a\n|[image 3]\nb", "ccX<Esc>u", "a\n|[image 3]\nb"),
        // In Insert mode Backspace at the start of the line below an image
        // goes onto the image first, and deletes it at the second press.
        c("[image 3]\n|b", "i<BS><Esc>", "|[image 3]\nb"),
        c("[image 3]\n|b", "i<BS><BS><Esc>", "|b"),
        c("a\n[image 3]\n|b", "i<BS><BS><Esc>", "|a\nb"),
        c("[image 3]\n|", "i<BS><Esc>", "|[image 3]"),
        c("[image 3]\n|b", "i<C-h><Esc>", "|[image 3]\nb"),
        c("[image 3]\n|b", "i<C-w><Esc>", "|[image 3]\nb"),
        c("[image 3]\n|b", "i<C-u><Esc>", "|[image 3]\nb"),
        // Delete at the end of the line above one, the same.
        c("|a\n[image 3]", "A<Del><Esc>", "a\n|[image 3]"),
        c("|a\n[image 3]", "A<Del><Del><Esc>", "|a"),
        c("|a\n[image 3]\nb", "A<Del><Del><Esc>", "a\n|b"),
        // A divider goes at the first press, as before.
        c("---\n|b", "i<BS><Esc>", "|b"),
        c("|a\n---", "A<Del><Esc>", "|a"),
        // In Normal mode the two keys never reach another line.
        c("[image 3]\n|b", "X", "[image 3]\n|b"),
        c("|a\n[image 3]", "x", "|\n[image 3]"),
        c("|a\n[image 3]", "xx", "|\n[image 3]"),
    ]);
}

#[test]
fn the_dot_command_and_undo() {
    run(&[
        c("|[image 3]\n[image 4]\nx", "dd.", "|x"),
        c(
            "|[image 3]\n[image 4]\nx",
            "dd.uu",
            "|[image 3]\n[image 4]\nx",
        ),
        c("|[image 3]\n[image 4]\nx", "dd.u<C-r>", "|x"),
        c(
            "a\n|[image 3]\nb",
            "yyp.",
            "a\n[image 3]\n[image 3]\n|[image 3]\nb",
        ),
        c("a\n|[image 3]\nb", "yyp.uu", "a\n|[image 3]\nb"),
        c("|[image 3]", "oX<Esc>k.", "[image 3]\n|X\nX"),
    ]);
}
