//! The spelling keys of Vim mode (`]s`, `[s`, `z=`, `zg`, `zug`) and the
//! hooks behind them.
//!
//! The engine has no spell checker and neither has this crate: the
//! application connects one with [`VimMode::set_spelling`]. Until it does,
//! or while [`VimSpelling::is_enabled`] says no, the keys report
//! `E756: Spell checking is off`.
//!
//! # When the hooks are called
//!
//! Every hook is called while no entity but the [`VimMode`] itself is
//! being updated: it may read and update the editor and the spell checker
//! freely (it must not update the `VimMode` it belongs to). That takes a
//! detour for `]s` and `[s`, whose answers the engine needs in the middle
//! of a call that holds the editor's state: the answers are fetched right
//! before that call ([`writer_vim::Vim::spell_lookahead`] tells for which
//! key, [`writer_vim::walk_misspellings`] which questions) and replayed
//! during it. The commands (`z=`, `zg`, `zug`) are carried out right after
//! the call; what came of them goes to the status line like any message of
//! the engine.

use std::rc::Rc;

use gpui::{App, Window};
use writer_core::{Position, TextRange};
use writer_vim::{Effect, Outcome, SpellCommand, SpellLookahead, SpellOff, walk_misspellings};

use crate::mode::VimMode;

/// The suggestions for one misspelled word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VimSuggestions {
    /// The misspelled word in the document.
    pub range: TextRange,
    /// The suggestions, best first. `1z=` takes the first.
    pub suggestions: Vec<String>,
}

/// The spell checker as the Vim keys need it. Implement it over the
/// application's checker and hand it to [`VimMode::set_spelling`].
///
/// Positions and ranges are positions of the document in the
/// [`VimMode`]'s editor.
///
/// Every method is called while no entity but the [`VimMode`] itself is
/// being updated: it may read and update the editor and the spell checker
/// freely, and must not update the `VimMode` it belongs to (what it
/// returns is reported in the status by the caller). The answers for `]s`
/// and `[s` are asked for right before the engine handles the `s`, the
/// commands are carried out right after their key.
///
/// ```ignore
/// struct CheckerHooks(Entity<SpellChecker>);
///
/// impl VimSpelling for CheckerHooks {
///     fn is_enabled(&self, cx: &App) -> bool {
///         self.0.read(cx).is_active(cx)
///     }
///     fn misspelling_from(&self, from: Position, forward: bool, cx: &mut App) -> Option<TextRange> {
///         self.0.update(cx, |checker, cx| match forward {
///             true => checker.next_misspelling(from, cx),
///             false => checker.previous_misspelling(from, cx),
///         })
///     }
///     fn suggestions_at(&self, at: Position, cx: &mut App) -> Option<VimSuggestions> {
///         self.0.update(cx, |checker, cx| {
///             let range = checker.misspelling_at(at, cx)?;
///             let suggestions = checker.suggestions_at(at, cx);
///             Some(VimSuggestions { range, suggestions })
///         })
///     }
///     fn replace(&self, range: TextRange, replacement: &str, cx: &mut App) -> bool {
///         self.0.update(cx, |checker, cx| checker.replace(range, replacement, cx))
///     }
///     fn learn(&self, at: Position, _word: &str, cx: &mut App) -> Option<String> {
///         self.0.update(cx, |checker, cx| checker.learn_at(at, cx))
///     }
///     fn unlearn(&self, at: Position, _word: &str, cx: &mut App) -> Option<String> {
///         self.0.update(cx, |checker, cx| checker.unlearn_at(at, cx))
///     }
///     fn open_suggestions(&self, _at: Position, window: &mut Window, cx: &mut App) -> bool {
///         // The caret is at `_at` (Visual mode has ended).
///         self.0.update(cx, |checker, cx| checker.show_suggestions(window, cx))
///     }
/// }
///
/// // per editor, next to `VimMode::new`
/// vim.update(cx, |vim, _| vim.set_spelling(Some(Rc::new(CheckerHooks(checker.clone())))));
/// ```
pub trait VimSpelling {
    /// Whether words are being checked in this editor right now. While
    /// not, the keys report that spell checking is off and no other
    /// method is called.
    fn is_enabled(&self, cx: &App) -> bool;

    /// `]s` / `[s`: forward, the first misspelled word that starts after
    /// `from`; backward, the last one that starts before `from`. Wrapping
    /// around the document: with one misspelled word the answer is that
    /// word from anywhere. `None` when no word is marked. With a count the
    /// method is called once per step, each time from the start of the
    /// last answer, and never more often than there are misspelled words
    /// plus one.
    fn misspelling_from(&self, from: Position, forward: bool, cx: &mut App) -> Option<TextRange>;

    /// `{count}z=`: the misspelled word at `at` (the Vim cursor) and its
    /// suggestions. `None` when the word there is not marked as misspelled.
    fn suggestions_at(&self, at: Position, cx: &mut App) -> Option<VimSuggestions>;

    /// `{count}z=`: replaces `range` (the word of
    /// [`VimSpelling::suggestions_at`]) with one of its suggestions, as one
    /// undo step that keeps the formatting. Returns whether the document
    /// changed. The caret is put on the start of the word afterwards, as
    /// in Vim.
    fn replace(&self, range: TextRange, replacement: &str, cx: &mut App) -> bool;

    /// `zg`: remembers the misspelled word at `at` as correct and returns
    /// it; `None` when there is nothing to learn there. `word` is the word
    /// under the cursor as Vim sees words, for a checker that would rather
    /// learn by text.
    fn learn(&self, at: Position, word: &str, cx: &mut App) -> Option<String>;

    /// `zug`: forgets the learned word at `at` and returns it; `None` when
    /// the word there was never learned.
    fn unlearn(&self, at: Position, word: &str, cx: &mut App) -> Option<String>;

    /// `z=`: opens the suggestions for the misspelled word at `at` (where
    /// the caret is) as a menu. Returns whether a menu opened; `false`
    /// when the word there is not marked as misspelled.
    fn open_suggestions(&self, at: Position, window: &mut Window, cx: &mut App) -> bool;
}

impl VimMode {
    /// Connects (or with `None` disconnects) the spell checker behind
    /// `]s`, `[s`, `z=`, `zg` and `zug`. See [`VimSpelling`].
    pub fn set_spelling(&mut self, spelling: Option<Rc<dyn VimSpelling>>) {
        self.spelling = spelling;
    }

    /// Whether a spell checker is connected.
    pub fn has_spelling(&self) -> bool {
        self.spelling.is_some()
    }

    /// Questions of the engine about misspelled words that had not been
    /// prepared (see the `spell` module). Stays 0; tests
    /// watch it.
    #[doc(hidden)]
    pub fn spell_misses(&self) -> usize {
        self.spell_misses
    }

    /// Before the engine handles `lookahead`'s key: what the spell checker
    /// answers to the questions the engine is going to ask. `None` when
    /// spell checking is off.
    pub(crate) fn prepare_spelling(
        &self,
        lookahead: SpellLookahead,
        cx: &mut App,
    ) -> Option<SpellAnswers> {
        let spelling = self.spelling.clone()?;
        if !spelling.is_enabled(cx) {
            return None;
        }
        let mut answers = SpellAnswers::default();
        if let SpellLookahead::Misspellings {
            from,
            forward,
            count,
        } = lookahead
        {
            answers.forward = forward;
            walk_misspellings(from, forward, count, |p| {
                let answer = spelling.misspelling_from(p, forward, cx);
                answers.steps.push((p, answer));
                answer
            });
        }
        Some(answers)
    }

    /// Carries out a spelling command of the engine, after its call, and
    /// adds what came of it to the call's effects.
    pub(crate) fn run_spell_command(
        &mut self,
        command: SpellCommand,
        outcome: &mut Outcome,
        window: Option<&mut Window>,
        cx: &mut App,
    ) {
        let Some(spelling) = self.spelling.clone() else {
            return;
        };
        let fail = |outcome: &mut Outcome, text: String| {
            outcome.effects.push(Effect::Error(text));
            outcome.effects.push(Effect::Bell);
        };
        const NOT_MISSPELLED: &str = "No misspelled word under the cursor";
        match command {
            SpellCommand::Suggest { at, pick: None } => match window {
                Some(window) => {
                    if !spelling.open_suggestions(at, window, cx) {
                        fail(outcome, NOT_MISSPELLED.into());
                    }
                }
                None => fail(outcome, "z= needs the window of the editor".into()),
            },
            SpellCommand::Suggest {
                at,
                pick: Some(number),
            } => {
                let Some(found) = spelling.suggestions_at(at, cx) else {
                    fail(outcome, NOT_MISSPELLED.into());
                    return;
                };
                let count = found.suggestions.len();
                let Some(replacement) = found.suggestions.get(number.max(1) - 1) else {
                    // Vim's words.
                    fail(
                        outcome,
                        match count {
                            0 => "Sorry, no suggestions".into(),
                            1 => "Sorry, only 1 suggestion".into(),
                            n => format!("Sorry, only {n} suggestions"),
                        },
                    );
                    return;
                };
                if !spelling.replace(found.range, replacement, cx) {
                    outcome.effects.push(Effect::Bell);
                    return;
                }
                // The cursor goes to the start of the word, as in Vim. The
                // edit came from outside the engine: let it catch up now,
                // so the caret is on a character before the next key.
                self.editor.update(cx, |editor, cx| {
                    editor.edit(|state| state.set_caret(found.range.start), cx);
                });
                let (synced, _) = self.engine_call_inner(crate::mode::Call::Sync, cx);
                outcome.effects.extend(synced.effects);
            }
            SpellCommand::MarkGood { at, word } => match spelling.learn(at, &word, cx) {
                Some(learned) => outcome
                    .effects
                    .push(Effect::Message(format!("Learned spelling: {learned}"))),
                None => fail(outcome, NOT_MISSPELLED.into()),
            },
            SpellCommand::UndoMarkGood { at, word } => match spelling.unlearn(at, &word, cx) {
                Some(unlearned) => outcome
                    .effects
                    .push(Effect::Message(format!("Unlearned spelling: {unlearned}"))),
                None => fail(outcome, format!("Not a learned word: {word}")),
            },
        }
    }
}

/// The spell checker's answers for one call into the engine, fetched
/// before it.
#[derive(Default)]
pub(crate) struct SpellAnswers {
    forward: bool,
    /// The misspelled word after (before) each position the engine is
    /// going to ask about.
    steps: Vec<(Position, Option<TextRange>)>,
}

/// The spelling side of the engine's host for one call.
#[derive(Default)]
pub(crate) struct SpellHost {
    /// `None`: spell checking is off (or the key was not expected to ask).
    pub(crate) answers: Option<SpellAnswers>,
    /// The commands of the call, to carry out after it.
    pub(crate) commands: Vec<SpellCommand>,
    /// Questions that had no prepared answer.
    pub(crate) misses: usize,
}

impl SpellHost {
    pub(crate) fn misspelling_from(
        &mut self,
        from: Position,
        forward: bool,
    ) -> Result<Option<TextRange>, SpellOff> {
        let answers = self.answers.as_ref().ok_or(SpellOff)?;
        let prepared = answers
            .steps
            .iter()
            .find(|(asked, _)| *asked == from && answers.forward == forward);
        match prepared {
            Some((_, answer)) => Ok(*answer),
            None => {
                self.misses += 1;
                Ok(None)
            }
        }
    }

    pub(crate) fn command(&mut self, command: SpellCommand) -> Result<(), SpellOff> {
        if self.answers.is_none() {
            return Err(SpellOff);
        }
        self.commands.push(command);
        Ok(())
    }
}
