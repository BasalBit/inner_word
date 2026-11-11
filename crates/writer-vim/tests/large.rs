//! A long document: commands cost what they touch, not the whole book.

use std::time::Instant;

use writer_core::{Block, Document, EditorOptions, EditorState};
use writer_vim::{Key, Vim, VimHost};

/// A host with one row per hard line that answers without scanning.
struct FlatHost;

impl VimHost for FlatHost {
    fn visual_row(
        &mut self,
        state: &EditorState,
        pos: writer_core::Position,
    ) -> std::ops::Range<usize> {
        state.document().blocks()[pos.block].line_range(pos.offset)
    }

    fn column_of(&mut self, state: &EditorState, pos: writer_core::Position) -> f32 {
        let r = self.visual_row(state, pos);
        (pos.offset - r.start) as f32
    }

    fn move_by_rows(
        &mut self,
        state: &EditorState,
        from: writer_core::Position,
        rows: isize,
        _goal: writer_vim::GoalColumn,
    ) -> writer_core::Position {
        let last = state.document().block_count() as isize - 1;
        let block = (from.block as isize + rows).clamp(0, last) as usize;
        writer_core::Position::new(block, 0)
    }

    fn viewport(&mut self, _state: &EditorState) -> Option<writer_vim::Viewport> {
        None
    }

    fn scroll(&mut self, _state: &EditorState, _scroll: writer_vim::Scroll) {}

    fn clipboard_read(&mut self) -> Option<writer_vim::RegisterContent> {
        None
    }

    fn clipboard_write(&mut self, _content: &writer_vim::RegisterContent) {}
}

#[test]
fn ten_thousand_blocks() {
    let blocks: Vec<Block> = (0..10_000)
        .map(|i| {
            Block::paragraph(&format!(
                "Paragraph {i} has a few words in it, and a needle{} near the end.",
                i % 500
            ))
        })
        .collect();
    let mut ed = EditorState::new(Document::from_blocks(blocks));
    ed.set_options(EditorOptions {
        input_rules: false,
        smart_punctuation: false,
    });
    let mut host = FlatHost;
    let mut vim = Vim::new();
    vim.attach(&mut ed, &mut host);
    let mut run = |keys: &str, ed: &mut EditorState| {
        for k in Key::parse_sequence(keys) {
            let o = vim.handle_key(k, ed, &mut host);
            if !o.consumed && vim.mode().is_insert() {
                // Typing is the host's.
                if let Some(c) = k.plain_char() {
                    let mut b = [0u8; 4];
                    let s = c.encode_utf8(&mut b);
                    ed.insert_text(s);
                    vim.text_inserted(s, ed, &mut host);
                }
            }
        }
    };
    let start = Instant::now();
    // Each round: motions, a change, a line move, a search, undo.
    for round in 0..200 {
        run("5000G", &mut ed);
        run("wwwbbe$0^", &mut ed);
        run("cwword<Esc>", &mut ed);
        run("ddp", &mut ed);
        run("yyP", &mut ed);
        run("jjkk", &mut ed);
        run("xu", &mut ed);
        run("A more<Esc>", &mut ed);
        run(">>.<<", &mut ed);
        run("vee~", &mut ed);
        run("mz'z", &mut ed);
        run("uuuu", &mut ed);
        if round % 20 == 0 {
            run("/needle77<CR>n", &mut ed);
            run("gg}G{", &mut ed);
        }
    }
    let per_round = start.elapsed() / 200;
    ed.check_invariants().unwrap();
    // Around forty commands per round; a regression to "scan the document
    // on every key" would cost tens of milliseconds per round.
    assert!(
        per_round.as_millis() < 20,
        "a round of commands took {per_round:?} on 10,000 blocks"
    );
    // The whole document as one range is the one place that may take long.
    let start = Instant::now();
    run(":%s/needle/pin/g<CR>", &mut ed);
    run("u", &mut ed);
    run("ggVGy", &mut ed);
    let whole = start.elapsed();
    assert!(
        whole.as_millis() < 2_000,
        "whole-document commands took {whole:?}"
    );
}
