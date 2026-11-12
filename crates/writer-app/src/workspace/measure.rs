//! Measuring what a save and the character-limit indicator cost (the
//! `book`, `savecost` and `limitcost` steps of the automation script). Not
//! used by the app itself.
//!
//! `savecost` times every part of the synchronous save on the main thread:
//! the same work [`Workspace::flush`] does when a document is closed. The
//! autosave does most of it on a background thread instead (see
//! [`super::saver`]); what that costs is reported by the `state` step after
//! a real autosave.

use std::time::{Duration, Instant};

use gpui::{Context, Window};

use super::Workspace;
use crate::{document_io::StoredForm, welcome};

fn millis(duration: Duration) -> String {
    format!("{:.2} ms", duration.as_secs_f64() * 1000.)
}

fn summary(label: &str, samples: &mut [Duration]) -> String {
    samples.sort();
    let median = samples[samples.len() / 2];
    format!(
        "{label}: median {}, min {}, max {}",
        millis(median),
        millis(samples[0]),
        millis(samples[samples.len() - 1])
    )
}

impl Workspace {
    /// Creates a generated document of `blocks` blocks at the root of the
    /// library and opens it. Returns its title.
    pub fn open_generated_book(
        &mut self,
        blocks: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<String> {
        let document =
            welcome::book_document(blocks).with_settings(self.settings.new_document_settings());
        let form = StoredForm::of(&document, None);
        let meta = self
            .library
            .update(cx, |library, cx| {
                library.create_document(None, &form.content(), cx)
            })
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        if !self.open_document(meta.id, true, window, cx) {
            anyhow::bail!("the generated document could not be opened");
        }
        Ok(meta.title)
    }

    /// Times each part of a synchronous save of the open document, `rounds`
    /// times, and returns one line per part. Every round really saves (the
    /// title is nudged so the stored row differs).
    pub fn measure_save_cost(&mut self, rounds: usize, cx: &mut Context<Self>) -> Vec<String> {
        let Some(id) = self.open_document_id() else {
            return vec!["no document is open".to_string()];
        };
        let rounds = rounds.max(1);
        let mut clone = Vec::new();
        let mut json = Vec::new();
        let mut plain = Vec::new();
        let mut stats = Vec::new();
        let mut write = Vec::new();
        let mut snapshot = Vec::new();
        let mut view_state = Vec::new();
        let mut total = Vec::new();
        let mut sizes = (0, 0, 0);

        for round in 0..rounds {
            let started = Instant::now();
            let editor = self.editor.read(cx);
            let document = editor.document();

            let at = Instant::now();
            let copy = document.clone();
            clone.push(at.elapsed());

            let at = Instant::now();
            let content = document.to_json();
            json.push(at.elapsed());

            let at = Instant::now();
            let text = document.plain_text();
            plain.push(at.elapsed());

            // What the save uses: the counts the editor keeps current.
            let at = Instant::now();
            let counted = if editor.stats_pending() {
                document.stats()
            } else {
                editor.stats()
            };
            stats.push(at.elapsed());

            sizes = (copy.block_count(), content.len(), text.len());
            let form = StoredForm {
                json: content,
                title: format!("Measured {round}"),
                plain_text: text,
                words: counted.words as u64,
                characters: counted.characters as u64,
            };
            drop(copy);

            let at = Instant::now();
            let saved = self.library.update(cx, |library, cx| {
                library.save_document(id, &form.content(), cx)
            });
            write.push(at.elapsed());
            if let Err(error) = saved {
                return vec![format!("the save failed: {error}")];
            }

            let at = Instant::now();
            let taken = self.library.update(cx, |library, cx| {
                library.snapshot_if_stale(id, Duration::ZERO, cx)
            });
            snapshot.push(at.elapsed());
            if let Err(error) = taken {
                return vec![format!("the snapshot failed: {error}")];
            }

            let at = Instant::now();
            let state =
                crate::view_memory::stored_view_state(self.editor.read(cx)).unwrap_or_default();
            let stored = self
                .library
                .update(cx, |library, _| library.save_view_state(id, &state));
            view_state.push(at.elapsed());
            if let Err(error) = stored {
                return vec![format!("the view state was not stored: {error}")];
            }
            total.push(started.elapsed());
        }

        vec![
            format!(
                "{} blocks, {} bytes of JSON, {} bytes of text, {rounds} rounds",
                sizes.0, sizes.1, sizes.2
            ),
            summary("copy of the document", &mut clone),
            summary("serialise to JSON", &mut json),
            summary("plain text", &mut plain),
            summary("word and character counts (kept by the editor)", &mut stats),
            summary("library write (row, full-text index, commit)", &mut write),
            summary("version snapshot", &mut snapshot),
            summary("view state write", &mut view_state),
            summary("whole save on the main thread", &mut total),
        ]
    }

    /// Times the character-limit indicator on the open document: counting
    /// the whole text (what every revision used to cost) against the
    /// block-by-block counter after an edit of one block. One line each.
    pub fn measure_limit_cost(&mut self, rounds: usize, cx: &mut Context<Self>) -> Vec<String> {
        use crate::character_limit::LimitCounter;
        use writer_export::SocialNetwork;

        if self.open_document_id().is_none() {
            return vec!["no document is open".to_string()];
        }
        let rounds = rounds.max(1);
        let network = SocialNetwork::X;
        let mut whole = Vec::new();
        let mut first = Vec::new();
        let mut after_edit = Vec::new();
        let mut counter = LimitCounter::new(network);
        let mut blocks = 0;
        for round in 0..rounds {
            let editor = self.editor.clone();
            {
                let editor = editor.read(cx);
                let document = editor.document();
                blocks = document.block_count();
                let at = Instant::now();
                let counted = network.count(&document.plain_text());
                whole.push(at.elapsed());
                if round == 0 {
                    let at = Instant::now();
                    let by_block = counter.count(document);
                    first.push(at.elapsed());
                    assert_eq!(by_block, counted);
                }
            }
            // One keystroke somewhere in the middle, then count again.
            editor.update(cx, |editor, cx| {
                editor.edit_with(
                    writer_editor::Autoscroll::None,
                    |state| {
                        let middle = state.document().block_count() / 2;
                        state.set_caret(writer_core::Position::new(middle, 0));
                        state.insert_text("x");
                    },
                    cx,
                );
            });
            let editor = editor.read(cx);
            let at = Instant::now();
            let by_block = counter.count(editor.document());
            after_edit.push(at.elapsed());
            assert_eq!(by_block, network.count(&editor.document().plain_text()));
        }
        vec![
            format!("{blocks} blocks, {rounds} rounds"),
            summary("counting the whole text", &mut whole),
            summary("block by block, the first time", &mut first),
            summary("block by block, after an edit", &mut after_edit),
        ]
    }
}
