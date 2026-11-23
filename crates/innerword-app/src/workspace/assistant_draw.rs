//! An answer's drawing applied (`docs/assistant-draws-design.md`, sections 5
//! and 6): on the open whiteboard ("Draw on Whiteboard"), or as a new
//! whiteboard beside the open document ("New Whiteboard from Answer").
//!
//! The side chat reads the drawing from the answer and asks for it
//! ([`innerword_assistant_ui::AssistantPanelEvent::Draw`]); nothing here
//! happens without that press. What is drawn is made by the converter agents
//! use ([`innerword_board::draw`]), whole or not at all.
//!
//! **On the open whiteboard** the path is Restore's (section 6.5 of
//! `docs/whiteboard-design.md`) with the writer as the one who asked: the
//! page is asked for what it holds, then in one turn of the main thread the
//! scene it handed over is saved, the drawing is made on the stored scene, a
//! version "Before the assistant's drawing" and the save are one outcome,
//! and the stored scene is taken into the view by the base rule as one step
//! of Excalidraw's undo. The keyboard goes to the canvas, and the writer is
//! told, only once the view answered that adopt, so that a `cmd-z` right
//! after it takes the drawing out and nothing else.
//!
//! **Over a document** a new whiteboard is made in the document's folder
//! with the drawing on it, and opened.

use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use gpui::{Context, SharedString, Window};
use innerword_assistant::{Drawing, MessageId};
use innerword_board::{DrawError, Drawn, HashedMaking, Scene};
use innerword_board_web::calls;
use innerword_library::{DocumentId, OutsideChange, StoreError};
use innerword_ui::Toast;

use super::{Workspace, conflicts_sentence, saver::prepared_of, session_title};
use crate::{boards, pages};

/// The label of the version kept before an answer's drawing.
pub const ASSISTANT_DRAWING_LABEL: &str = "Before the assistant's drawing";

/// The press found no whiteboard open.
pub const OPEN_A_WHITEBOARD: &str = "Open a whiteboard to draw on it.";
/// Another drawing or an outside change is being taken in.
pub const BOARD_BUSY: &str = "The whiteboard is taking in another change. Press again in a moment.";
/// The whiteboard closed between the press and the drawing.
pub const CLOSED_BEFORE_DRAWN: &str =
    "Nothing was drawn: the whiteboard was closed before the drawing was made.";
/// What the page handed over could not be saved first.
pub const NOT_SAVED_FIRST: &str =
    "Nothing was drawn: the whiteboard as it is now could not be saved first.";
/// The version could not be kept.
pub const NO_VERSION: &str =
    "Nothing was drawn: the whiteboard as it was could not be kept as a version.";
/// The stored scene changed under the drawing (cannot happen after the flush
/// in the same turn; said all the same).
pub const CHANGED_MEANWHILE: &str =
    "Nothing was drawn: the whiteboard changed while the drawing was made. Press again.";
/// Every removal named something gone already, and there was nothing else.
pub const ONLY_GONE: &str = "Nothing was drawn: everything it removes is gone already.";
/// Every change set what its element had already.
pub const CHANGES_NOTHING: &str = "Nothing was drawn: it changes nothing on the whiteboard.";
/// After the words about ids that are gone.
pub const ASK_AGAIN: &str = "Ask again: the assistant is sent the whiteboard as it is now.";
/// Over a document, a drawing that changes or removes elements.
pub const CHANGES_A_WHITEBOARD: &str =
    "This drawing changes elements of a whiteboard: open that whiteboard and ask there.";
/// The press found no document open for a new whiteboard.
pub const OPEN_A_DOCUMENT: &str = "Open a document to make a whiteboard from the answer.";
/// The toast's Undo after the writer drew.
pub const NOT_UNDONE: &str =
    "Not undone: the whiteboard has changed since. Version History has it as it was.";
/// A drawing that stands on its own put elsewhere, in view.
pub const PUT_IN_VIEW: &str = "It was put where it is free and in view.";
/// One put beside the drawing (no room in view).
pub const PUT_BESIDE: &str = "It was put beside the drawing.";
/// After what was drawn: the version.
pub const KEPT_AS_A_VERSION: &str = "The whiteboard as it was is kept as a version.";
/// A failed adopt: the view loaded the stored scene.
pub const LOADED_AGAIN: &str = "The whiteboard was loaded again to show it.";
/// The palette's command while the side chat is not shown: the answer is
/// shown first, the second run draws.
pub const SHOWN_IN_THE_SIDE_CHAT: &str =
    "The drawing is shown in the side chat. Run the command again to draw it.";

/// How long the toast of a drawing stays.
const DRAWN_TOAST: Duration = Duration::from_secs(8);

/// `title` in the quotation marks of a toast ("Untitled" for none).
fn quoted(title: &str) -> String {
    format!("\u{201c}{}\u{201d}", session_title(title))
}

/// "api", "api and db", "a, b, c, d, e and 3 more".
fn listed(ids: &[String]) -> String {
    let shown: Vec<&str> = ids.iter().take(5).map(String::as_str).collect();
    match (shown.as_slice(), ids.len()) {
        ([], _) => String::new(),
        ([one], 1) => (*one).to_string(),
        (_, n) if n > 5 => format!("{} and {} more", shown.join(", "), n - 5),
        (shown, _) => format!(
            "{} and {}",
            shown[..shown.len() - 1].join(", "),
            shown[shown.len() - 1]
        ),
    }
}

/// "1 element added", "2 elements added, 1 changed, 3 removed".
fn counts(added: usize, changed: usize, removed: usize) -> String {
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!(
            "{added} element{} added",
            if added == 1 { "" } else { "s" }
        ));
    }
    let noun = if parts.is_empty() { " element" } else { "" };
    let plural = |n: usize| if n == 1 || noun.is_empty() { "" } else { "s" };
    if changed > 0 {
        parts.push(format!("{changed}{noun}{} changed", plural(changed)));
    }
    let noun = if parts.is_empty() { " element" } else { "" };
    let plural = |n: usize| if n == 1 || noun.is_empty() { "" } else { "s" };
    if removed > 0 {
        parts.push(format!("{removed}{noun}{} removed", plural(removed)));
    }
    parts.join(", ")
}

/// What the toast of a drawing on the open whiteboard says first: "Drew on
/// “Plan”: 2 elements added, 1 changed. The whiteboard as it was is kept as
/// a version."
pub fn drew_sentence(title: &str, added: usize, changed: usize, removed: usize) -> String {
    format!(
        "Drew on {}: {}. {KEPT_AS_A_VERSION}",
        quoted(title),
        counts(added, changed, removed)
    )
}

/// "1 element to remove was gone already." `None` for none.
pub fn gone_sentence(gone: usize) -> Option<String> {
    match gone {
        0 => None,
        1 => Some("1 element to remove was gone already.".to_string()),
        n => Some(format!("{n} elements to remove were gone already.")),
    }
}

/// What a new whiteboard's drawing says when it joins arrows or frames to
/// elements it does not add: on an empty scene no other element is meant.
pub fn new_board_names_missing(named: &[String]) -> String {
    format!(
        "Nothing was drawn: the answer joins arrows or frames to {}, which the drawing does not \
         have. Ask again.",
        listed(named)
    )
}

/// "Made the whiteboard “Who serves whom” in Part one from the answer." (at
/// the root of the library: "… from the answer.").
pub fn made_sentence(title: &str, folder: Option<&str>) -> String {
    match folder {
        Some(folder) => format!(
            "Made the whiteboard {} in {folder} from the answer.",
            quoted(title)
        ),
        None => format!("Made the whiteboard {} from the answer.", quoted(title)),
    }
}

/// The empty state's line of a whiteboard made from an answer about the
/// document titled `title`.
pub fn origin_note(title: &str) -> String {
    format!("Drawn from an answer about {}.", quoted(title))
}

/// What became of the adopt of an answer's drawing.
pub(crate) enum DrawAnswered {
    /// The view took it in: its rev, and the elements it kept the writer's
    /// copy of.
    Adopted { rev: u64, conflicts: Vec<String> },
    /// The adopt failed and the view loaded the stored scene again.
    Reloaded,
    /// The view loaded again before it answered.
    Forgotten,
    /// No adopt was sent (no element changed for the view: cannot happen
    /// for a drawing that changed something, handled all the same).
    NotSent,
}

/// The adopt of an answer's drawing that the view has not answered
/// (`BoardState::assistant_drawing`).
pub(crate) struct PendingDrawing {
    /// The number of the adopt (`BoardState::adopt_in_flight`).
    pub seq: u64,
    done: pages::AnswerTx<DrawAnswered>,
}

impl PendingDrawing {
    /// Tells the task that waits for the answer what it was.
    pub(crate) fn answered(self, answered: DrawAnswered) {
        self.done.send(answered);
    }
}

/// What a drawing did, for the toast once the view answers and for the line
/// of a scripted run.
struct Report {
    title: String,
    added: usize,
    changed: usize,
    removed: usize,
    gone: usize,
    warnings: Vec<String>,
    placed: &'static str,
    put: Option<&'static str>,
    /// The main thread's part up to the adopt: the flush, the drawing, the
    /// version and the save.
    change: Duration,
    /// The adopt sent: the two scenes read and their delta.
    window: Duration,
    sent: Instant,
    keyboard_was_in_chat: bool,
}

/// Whether this run is scripted (its `draw:` lines are printed).
fn scripted() -> bool {
    std::env::var_os(crate::automation::AUTOMATION_ENV).is_some_and(|script| !script.is_empty())
}

impl Workspace {
    /// What the last answer's drawing did, as a scripted run prints it:
    /// `draw: 2 added, 1 changed, 0 removed, version kept, placed as written,
    /// …`, `draw: nothing: <why>`, `draw: new board:14 “Who serves whom” …`.
    pub fn last_drawing_report(&self) -> Option<&str> {
        self.assistant.last_draw_report.as_deref()
    }

    fn report_drawing(&mut self, line: String) {
        if scripted() {
            eprintln!("[automation] draw: {line}");
        }
        self.assistant.last_draw_report = Some(format!("draw: {line}"));
    }

    /// Nothing was drawn: the writer is told why, and a scripted run too.
    fn drew_nothing(&mut self, words: impl Into<String>, cx: &mut Context<Self>) {
        let words = words.into();
        self.report_drawing(format!("nothing: {words}"));
        self.toast_error(words, cx);
    }

    // ----- on the open whiteboard -----------------------------------------------------

    /// "Draw on Whiteboard" (section 5): answer `message`'s `drawing` on the
    /// open whiteboard. Nothing happens at once; the drawing is applied after
    /// the page was asked for what it holds. One drawing at a time: a second
    /// press while one is under way (or while an outside change is taken in)
    /// draws nothing and says so.
    pub fn draw_answer_on_board(
        &mut self,
        drawing: Drawing,
        message: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.open_board_id() else {
            self.drew_nothing(OPEN_A_WHITEBOARD, cx);
            return;
        };
        if !self.board_ready() {
            let sentence = self.board_not_ready_sentence(cx);
            self.report_drawing(format!("nothing: {sentence}"));
            self.say_about_page(&sentence, cx);
            return;
        }
        let busy = self.board().is_some_and(|board| {
            board.drawing_pending
                || board.assistant_drawing.is_some()
                || board.adopt_in_flight().is_some()
        });
        if busy {
            self.report_drawing(format!("nothing: {BOARD_BUSY}"));
            self.show_toast(Toast::new(BOARD_BUSY).duration(Duration::from_secs(5)), cx);
            return;
        }
        let keyboard_was_in_chat = self.assistant_panel_has_focus(window, cx);
        let Some(board) = self.board_of_mut(id) else {
            return;
        };
        board.drawing_pending = true;
        let session = board.session();
        let holds_more = board.status.holds_more();
        let collect = holds_more.then(|| self.collect_board(id, cx));
        cx.spawn_in(window, async move |this, cx| {
            if let Some(collect) = collect {
                collect.await;
            }
            this.update_in(cx, |this, window, cx| {
                this.apply_answer_drawing(
                    id,
                    session,
                    &drawing,
                    message,
                    keyboard_was_in_chat,
                    window,
                    cx,
                );
            })
            .ok();
        })
        .detach();
    }

    /// Step 3 of 5.1, in one turn of the main thread from the checks to the
    /// adopt call.
    #[allow(clippy::too_many_arguments)]
    fn apply_answer_drawing(
        &mut self,
        id: DocumentId,
        session: u64,
        drawing: &Drawing,
        message: MessageId,
        keyboard_was_in_chat: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Everything is checked again: the await let other turns run.
        let same = self.open_board_id() == Some(id)
            && self.board().is_some_and(|board| board.session() == session);
        if !same {
            if let Some(board) = self
                .board_of_mut(id)
                .filter(|board| board.session() == session)
            {
                board.drawing_pending = false;
            }
            self.drew_nothing(CLOSED_BEFORE_DRAWN, cx);
            return;
        }
        let refuse = |this: &mut Self, words: String, cx: &mut Context<Self>| {
            if let Some(board) = this.board_of_mut(id) {
                board.drawing_pending = false;
            }
            this.drew_nothing(words, cx);
        };
        if !self.board_ready() {
            let sentence = self.board_not_ready_sentence(cx);
            refuse(self, sentence, cx);
            return;
        }
        if self
            .board()
            .is_some_and(|board| board.adopt_in_flight().is_some())
        {
            // An agent's request carried out during the await started its
            // own adopt: a second one now would leave one answer unmatched.
            if let Some(board) = self.board_of_mut(id) {
                board.drawing_pending = false;
            }
            self.report_drawing(format!("nothing: {BOARD_BUSY}"));
            self.show_toast(Toast::new(BOARD_BUSY).duration(Duration::from_secs(5)), cx);
            return;
        }
        let started = Instant::now();
        if !self.save_board_now(id, cx) {
            refuse(self, NOT_SAVED_FIRST.to_string(), cx);
            return;
        }
        let Some((base, stamp, rev)) = self
            .board()
            .map(|board| (board.base.clone(), board.stamp, board.status.rev))
        else {
            return;
        };
        let before = match Scene::from_stored(&base) {
            Ok(parsed) => parsed.scene,
            Err(error) => {
                refuse(
                    self,
                    format!("Nothing was drawn: the whiteboard cannot be read ({error})."),
                    cx,
                );
                return;
            }
        };
        let view = self.board_view_bounds();
        let now = self.library.read(cx).clock().now_ms();
        let seed = drawing.to_json();
        let stamp_hex = stamp.to_hex();
        let mut making = HashedMaking::new(
            &[
                stamp_hex.as_bytes(),
                &rev.to_le_bytes(),
                &now.to_le_bytes(),
                seed.as_bytes(),
            ],
            now,
        );
        let mut after = before.clone();
        let drawn = match innerword_board::draw(&mut after, drawing, view, &mut making) {
            Ok(drawn) => drawn,
            Err(error @ DrawError::Gone { .. }) => {
                refuse(self, format!("Nothing was drawn: {error}. {ASK_AGAIN}"), cx);
                return;
            }
            Err(DrawError::Board(error)) => {
                refuse(self, format!("Nothing was drawn: {error}."), cx);
                self.offer_to_ask_again(&error.to_string(), cx);
                return;
            }
        };
        if drawn.is_nothing() {
            let only_removals = drawing.add.is_empty() && drawing.update.is_empty();
            let words = if only_removals {
                ONLY_GONE
            } else {
                CHANGES_NOTHING
            };
            refuse(self, words.to_string(), cx);
            return;
        }
        // The version and the save: one outcome. A version kept over a save
        // that failed would say "Before the assistant's drawing" over nothing.
        let prepared = prepared_of(&after);
        let fail_on_purpose = boards::board_saves_fail(cx);
        let (result, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let snapshot = match store.create_labeled_snapshot(id, ASSISTANT_DRAWING_LABEL)
                    {
                        Ok(meta) => meta,
                        Err(error) => return (Err(DrawFailure::Version(error)), Vec::new()),
                    };
                    let saved = if fail_on_purpose {
                        Err(DrawFailure::Save(boards::FAILED_ON_PURPOSE.to_string()))
                    } else {
                        store
                            .save_board(id, &prepared.content(), Some(stamp))
                            .map_err(|error| match error {
                                StoreError::Stale { .. } => DrawFailure::Stale,
                                error => DrawFailure::Save(error.to_string()),
                            })
                    };
                    match saved {
                        Ok(saved) => (
                            Ok(saved),
                            vec![
                                OutsideChange::SnapshotsChanged(id),
                                OutsideChange::DocumentSaved(id),
                            ],
                        ),
                        Err(failure) => {
                            let gone = store.delete_snapshot(snapshot.id).is_ok();
                            let changes = if gone {
                                Vec::new()
                            } else {
                                vec![OutsideChange::SnapshotsChanged(id)]
                            };
                            (Err(failure), changes)
                        }
                    }
                },
                cx,
            )
        });
        if let Err(failure) = result {
            let words = match failure {
                DrawFailure::Version(error) => {
                    eprintln!("Inner Word: no version before the assistant's drawing: {error}");
                    NO_VERSION.to_string()
                }
                DrawFailure::Stale => CHANGED_MEANWHILE.to_string(),
                DrawFailure::Save(error) => {
                    format!("Nothing was drawn: it could not be saved ({error}).")
                }
            };
            refuse(self, words, cx);
            return;
        }
        // From here the library has the drawing, whatever the view does.
        if let Some(side) = self.assistant.side.clone() {
            side.update(cx, |side, cx| side.mark_drawn(message, cx));
        }
        // An agents' run on this whiteboard offers no Undo from here.
        self.writer_changed_the_board(id, cx);
        let gone = drawn.already_gone.len();
        let mut report = Report {
            title: self.page_title().unwrap_or_default().to_string(),
            added: drawn.added.len(),
            changed: changed_count(drawing, &drawn),
            removed: removed_count(drawing, &drawn),
            gone,
            warnings: drawn.warnings.clone(),
            placed: placed_words(&drawn, view.is_some()),
            put: drawn.placement.and_then(|placement| {
                if placement.dx == 0.0 && placement.dy == 0.0 {
                    None
                } else if placement.in_view {
                    Some(PUT_IN_VIEW)
                } else {
                    Some(PUT_BESIDE)
                }
            }),
            change: started.elapsed(),
            window: Duration::ZERO,
            sent: Instant::now(),
            keyboard_was_in_chat,
        };
        let zoom_to_fit = view.is_some() && drawn.placement.is_some_and(|p| !p.in_view);
        let seq = self.adopt_stored_scene(id, &prepared.scene, true, cx);
        report.window = report.sent.elapsed();
        if let Some(board) = self.board_of_mut(id) {
            board.drawing_pending = false;
        }
        let Some(seq) = seq else {
            // No delta reached the view (cannot happen for a drawing that
            // changed something): said at once, without Undo.
            self.drawing_answered(id, session, Some(DrawAnswered::NotSent), report, window, cx);
            return;
        };
        let (done, answer) = pages::answer();
        if let Some(board) = self.board_of_mut(id) {
            board.assistant_drawing = Some(PendingDrawing { seq, done });
        }
        if zoom_to_fit {
            // The page runs its calls in order: after the adopt.
            self.zoom_board(calls::Zoom::Fit);
        }
        cx.spawn_in(window, async move |this, cx| {
            let answered = answer.await;
            this.update_in(cx, |this, window, cx| {
                this.drawing_answered(id, session, answered, report, window, cx)
            })
            .ok();
        })
        .detach();
    }

    /// The view answered the adopt of an answer's drawing (or will not):
    /// the keyboard goes to the canvas when it is still in the side chat (or
    /// on the canvas already), so that the next `cmd-z` takes the drawing
    /// out; a writer who went elsewhere meanwhile keeps it there. Then the
    /// toast, with Undo while the canvas is at the rev the adopt answered.
    fn drawing_answered(
        &mut self,
        id: DocumentId,
        session: u64,
        answered: Option<DrawAnswered>,
        report: Report,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let open = self.open_board_id() == Some(id)
            && self.board().is_some_and(|board| board.session() == session);
        let answered_in = report.sent.elapsed();
        let mut moved = false;
        let takes_the_keyboard = matches!(
            answered,
            Some(DrawAnswered::Adopted { .. } | DrawAnswered::Reloaded | DrawAnswered::NotSent)
        );
        if open && takes_the_keyboard {
            let in_chat = self.assistant_panel_has_focus(window, cx);
            let on_canvas = self.page_has_focus(window);
            if on_canvas || (in_chat && report.keyboard_was_in_chat) {
                self.give_the_board_the_keyboard(window, cx);
                moved = true;
            }
        }
        let mut line = format!(
            "{} added, {} changed, {} removed, version kept, {}, change {:.1} ms, window {:.1} ms",
            report.added,
            report.changed,
            report.removed,
            report.placed,
            report.change.as_secs_f64() * 1000.0,
            report.window.as_secs_f64() * 1000.0,
        );
        let mut extra = Vec::new();
        extra.extend(gone_sentence(report.gone));
        let (message, undo_rev) = match answered {
            Some(DrawAnswered::Adopted { rev, conflicts }) => {
                line.push_str(&format!(
                    ", adopt answered in {:.0} ms",
                    answered_in.as_secs_f64() * 1000.0
                ));
                extra.extend(conflicts_sentence(&conflicts));
                extra.extend(report.warnings.iter().map(|warning| sentence_of(warning)));
                extra.extend(report.put.map(str::to_string));
                (
                    drew_sentence(&report.title, report.added, report.changed, report.removed),
                    Some(rev),
                )
            }
            Some(DrawAnswered::Reloaded) => {
                line.push_str(", the adopt failed and the view loaded again");
                (
                    format!("Drew on {}. {LOADED_AGAIN}", quoted(&report.title)),
                    None,
                )
            }
            Some(DrawAnswered::NotSent) => {
                line.push_str(", no adopt was needed");
                extra.extend(report.warnings.iter().map(|warning| sentence_of(warning)));
                (
                    drew_sentence(&report.title, report.added, report.changed, report.removed),
                    None,
                )
            }
            Some(DrawAnswered::Forgotten) | None => {
                line.push_str(", the view did not answer");
                (format!("Drew on {}.", quoted(&report.title)), None)
            }
        };
        line.push_str(if moved {
            ", keyboard to the canvas after the adopt answered"
        } else {
            ", keyboard left where it was"
        });
        self.report_drawing(line);
        if !open {
            return;
        }
        let mut words = message;
        for sentence in extra {
            words.push(' ');
            words.push_str(&sentence);
        }
        let mut toast = Toast::new(words).success().duration(DRAWN_TOAST);
        if let Some(rev) = undo_rev {
            let workspace = cx.weak_entity();
            toast = toast.on_action("Undo", move |_, cx| {
                workspace
                    .update(cx, |workspace, cx| {
                        workspace.undo_assistant_drawing(id, rev, cx)
                    })
                    .ok();
            });
        }
        self.show_toast(toast, cx);
        cx.notify();
    }

    /// The toast's Undo: one step of the whiteboard's own undo, while the
    /// canvas is at the rev the drawing's adopt answered (the writer has not
    /// drawn since); otherwise it says so.
    pub fn undo_assistant_drawing(&mut self, id: DocumentId, rev: u64, cx: &mut Context<Self>) {
        let holds = self.open_board_id() == Some(id)
            && self.board().is_some_and(|board| board.status.rev <= rev);
        if !holds {
            self.show_toast(Toast::new(NOT_UNDONE).duration(Duration::from_secs(6)), cx);
            return;
        }
        drop(self.board_call(id, calls::UNDO, calls::steps_argument(1)));
    }

    /// The converter refused the drawing: the composer, when it is empty, is
    /// offered a message that asks the assistant to write it again.
    fn offer_to_ask_again(&mut self, why: &str, cx: &mut Context<Self>) {
        if let Some(side) = self.assistant.side.clone() {
            let text = format!("Your drawing could not be used: {why}. Please write it again.");
            side.update(cx, |side, cx| side.offer_message(&text, cx));
        }
    }

    // ----- a new whiteboard -----------------------------------------------------------

    /// "New Whiteboard from Answer" (section 6): a new whiteboard in the
    /// folder of the open document (at the root when the document or a
    /// folder above it is in the trash), titled with the drawing's title,
    /// with the drawing on it, opened with the keyboard on its canvas.
    /// Returns its id.
    pub fn new_board_from_answer(
        &mut self,
        drawing: Drawing,
        message: MessageId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DocumentId> {
        let Some((document, document_title)) = self
            .session
            .as_ref()
            .map(|session| (session.id, session.title.clone()))
        else {
            self.drew_nothing(OPEN_A_DOCUMENT, cx);
            return None;
        };
        if pages::engine(cx).is_none() {
            let sentence = boards::not_shown_sentence(cx);
            self.drew_nothing(sentence, cx);
            return None;
        }
        if drawing.has_changes() {
            self.drew_nothing(CHANGES_A_WHITEBOARD, cx);
            return None;
        }
        let started = Instant::now();
        let now = self.library.read(cx).clock().now_ms();
        let seed = drawing.to_json();
        let mut making = HashedMaking::new(
            &[
                &document.0.to_le_bytes(),
                seed.as_bytes(),
                &now.to_le_bytes(),
            ],
            now,
        );
        let mut scene = Scene::empty();
        let drawn = match innerword_board::draw(&mut scene, &drawing, None, &mut making) {
            Ok(drawn) => drawn,
            Err(DrawError::Gone { changed, named }) => {
                let mut missing = changed;
                missing.extend(named);
                self.drew_nothing(new_board_names_missing(&missing), cx);
                return None;
            }
            Err(DrawError::Board(error)) => {
                self.drew_nothing(format!("Nothing was drawn: {error}."), cx);
                self.offer_to_ask_again(&error.to_string(), cx);
                return None;
            }
        };
        // Where: the document's folder, unless the document or a folder above
        // it is in the trash (a document opened from the trash makes no
        // whiteboard there), or the folder is gone.
        let (folder, folder_name) = {
            let library = self.library.read(cx);
            let tree = library.tree();
            let folder = tree
                .find_document(document)
                .and_then(|meta| meta.folder_id)
                .filter(|folder| tree.find_folder(*folder).is_some());
            let name = folder
                .and_then(|folder| tree.find_folder(folder))
                .map(|node| node.folder.name.clone());
            (folder, name)
        };
        let title = drawing
            .title
            .clone()
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| {
                // A document shown "Untitled" has no title to name it after.
                let shown = session_title(document_title.trim());
                if shown == crate::document_io::UNTITLED {
                    "Drawing".to_string()
                } else {
                    format!("{shown} (drawing)")
                }
            });
        let prepared = prepared_of(&scene);
        let (created, _) = self.library.update(cx, |library, cx| {
            library.change_outside(
                |store| {
                    let created = store.create_board(folder, &title, &prepared.content());
                    let changes = match &created {
                        Ok(saved) => vec![OutsideChange::DocumentCreated(saved.meta.id)],
                        Err(_) => Vec::new(),
                    };
                    (created, changes)
                },
                cx,
            )
        });
        let created = match created {
            Ok(created) => created,
            Err(error) => {
                self.drew_nothing(format!("Could not create the whiteboard: {error}"), cx);
                return None;
            }
        };
        let id = created.meta.id;
        // A name given is a name chosen (it never follows the text).
        let flag = crate::view_memory::TitleFlag::of(&created.meta);
        self.library
            .update(cx, |library, cx| {
                library.set_setting(&crate::settings::keys::custom_title(id), &flag, cx)
            })
            .ok();
        // The answer is marked before the chat switches to the whiteboard's.
        if let Some(side) = self.assistant.side.clone() {
            side.update(cx, |side, cx| side.mark_drawn(message, cx));
        }
        self.assistant
            .origins
            .insert(id, SharedString::from(origin_note(&document_title)));
        let change = started.elapsed();
        self.report_drawing(format!(
            "new board:{} {} in {}, {} added, change {:.1} ms",
            id.0,
            quoted(&title),
            folder_name.as_deref().unwrap_or("the library"),
            drawn.added.len(),
            change.as_secs_f64() * 1000.0,
        ));
        if !self.open_document(id, true, window, cx) {
            return Some(id);
        }
        let mut words = made_sentence(&title, folder_name.as_deref());
        for warning in &drawn.warnings {
            words.push(' ');
            words.push_str(&sentence_of(warning));
        }
        let workspace = cx.weak_entity();
        let back = format!("Back to {}", quoted(&document_title));
        let toast =
            Toast::new(words)
                .success()
                .duration(DRAWN_TOAST)
                .on_action(back, move |window, cx| {
                    workspace
                        .update(cx, |workspace, cx| {
                            workspace.open_document(document, true, window, cx);
                        })
                        .ok();
                });
        self.show_toast(toast, cx);
        Some(id)
    }
}

/// Why the version and the save of a drawing did not happen.
enum DrawFailure {
    Version(StoreError),
    Stale,
    Save(String),
}

/// A warning of the converter as a sentence: a capital and a period.
fn sentence_of(warning: &str) -> String {
    let mut chars = warning.trim().chars();
    let mut out = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
        None => String::new(),
    };
    if !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// How many of the whiteboard's elements the drawing's changes changed:
/// the distinct ids of its changes that changed something (labels, arrows
/// and children that followed are not counted apart).
fn changed_count(drawing: &Drawing, drawn: &Drawn) -> usize {
    let unchanged: HashSet<usize> = drawn.unchanged.iter().copied().collect();
    let mut ids = HashSet::new();
    for (index, change) in drawing.update.iter().enumerate() {
        if unchanged.contains(&index) {
            continue;
        }
        if let Some(id) = change.0.get("id").and_then(serde_json::Value::as_str) {
            ids.insert(id.to_string());
        }
    }
    ids.len()
}

/// How many elements the drawing removed: the distinct ids of its removals
/// that the whiteboard had (a shape's label goes with it, uncounted).
fn removed_count(drawing: &Drawing, drawn: &Drawn) -> usize {
    let gone: HashSet<&str> = drawn.already_gone.iter().map(String::as_str).collect();
    drawing
        .removed_ids()
        .iter()
        .filter(|id| !gone.contains(id.as_str()))
        .count()
}

/// Where the drawing went, for the line of a scripted run.
fn placed_words(drawn: &Drawn, view_known: bool) -> &'static str {
    match drawn.placement {
        None => "placed as written (it names what is there)",
        Some(placement) if placement.dx == 0.0 && placement.dy == 0.0 => "placed as written",
        Some(placement) if placement.in_view => "placed free in view",
        Some(_) if view_known => "placed beside the drawing, zoomed to fit",
        Some(_) => "placed beside the drawing",
    }
}
