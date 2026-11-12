//! Between the library's stored rows and the rich document: loading a document, storing
//! it the way the app stores it, the title rule, the version kept before a change, and
//! what a change did.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use writer_core::{BlockId, CommentId, Document, DocumentSettings, Orientation, PaperSize, Stats};
use writer_store::{DocumentContent, DocumentId, DocumentMeta, SnapshotMeta, Store};

use crate::error::{Error, Result, quoted_excerpt};
use crate::resolve::{UNTITLED, title_of};
use crate::types::{DocumentSettingsInfo, Time, VersionInfo};

/// The settings key of the app's flag "somebody chose this title" is this prefix followed
/// by the document's id. The app keeps a title that carries the flag through every save
/// instead of deriving one from the text.
pub(crate) const CUSTOM_TITLE_PREFIX: &str = "document.custom_title.";

/// The stored value of the flag, as the app writes and reads it: `true` in versions up
/// to 0.2.0, the creation time of the document it was set for since (ids of deleted
/// documents are handed out again).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
enum TitleFlag {
    Plain(bool),
    For(i64),
}

/// A document read from the library.
pub(crate) struct Loaded {
    /// Its row.
    pub(crate) meta: DocumentMeta,
    /// The document.
    pub(crate) document: Document,
    /// Whether its title was chosen by somebody (and is kept by a save) or follows the
    /// text.
    pub(crate) custom_title: bool,
    /// The content as it is stored.
    pub(crate) raw: String,
}

/// Reads stored content as a document. Empty content is an empty document, as in the
/// app; content that is not a document is an error here (the app shows it as plain text
/// so that nothing becomes unreachable, but a tool must not write over it).
pub(crate) fn parse(meta: &DocumentMeta, content: &str) -> Result<Document> {
    if content.trim().is_empty() {
        return Ok(Document::new());
    }
    Document::from_json(content).map_err(|error| Error::Unreadable {
        id: meta.id.0,
        name: quoted_excerpt(title_of(meta)),
        reason: error.to_string(),
    })
}

/// Loads the document `meta` names.
pub(crate) fn load(store: &Store, meta: &DocumentMeta) -> Result<Loaded> {
    let stored = store.document(meta.id)?;
    let document = parse(&stored.meta, &stored.content)?;
    let custom_title = title_is_chosen(store, &stored.meta, &document);
    Ok(Loaded {
        meta: stored.meta,
        document,
        custom_title,
        raw: stored.content,
    })
}

/// The title the text of `document` gives: its first heading, else its first line, else
/// "Untitled". The rule of the app (`writer_app::document_io::derived_title`).
pub(crate) fn derived_title(document: &Document) -> String {
    document
        .title()
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| UNTITLED.to_string())
}

/// Whether the title of a document is one somebody chose. The rule of the app
/// (`Workspace::title_was_chosen`): the flag says so, or the stored title is not the one
/// the text gives (and not the name of a copy, "Title copy", which goes back to following
/// the text with the first change).
pub(crate) fn title_is_chosen(store: &Store, meta: &DocumentMeta, document: &Document) -> bool {
    let flag: Option<TitleFlag> = store.setting(&title_flag_key(meta.id)).ok().flatten();
    match flag {
        Some(TitleFlag::Plain(true)) => true,
        Some(TitleFlag::For(created_at)) if created_at == meta.created_at => true,
        _ => named_outside(&meta.title, document),
    }
}

fn named_outside(title: &str, document: &Document) -> bool {
    let title = title.trim();
    if title.is_empty() {
        return false;
    }
    let derived = derived_title(document);
    if title == derived {
        return false;
    }
    let mut rest = title;
    while let Some(shorter) = rest.strip_suffix(" copy") {
        if shorter == derived {
            return false;
        }
        rest = shorter;
    }
    true
}

fn title_flag_key(id: DocumentId) -> String {
    format!("{CUSTOM_TITLE_PREFIX}{}", id.0)
}

/// Sets or removes the flag "somebody chose this title" of a document, in the form the
/// app writes.
pub(crate) fn set_title_flag(store: &mut Store, meta: &DocumentMeta, chosen: bool) -> Result<()> {
    let key = title_flag_key(meta.id);
    if chosen {
        store.set_setting(&key, &TitleFlag::For(meta.created_at))?;
    } else {
        store.remove_setting(&key)?;
    }
    Ok(())
}

/// Everything a save writes for one document: what the app's `StoredForm` holds.
pub(crate) struct Form {
    /// The document as JSON.
    pub(crate) json: String,
    /// The title shown in the library.
    pub(crate) title: String,
    /// The text, for full-text search.
    pub(crate) plain_text: String,
    /// Word count.
    pub(crate) words: u64,
    /// Character count (grapheme clusters).
    pub(crate) characters: u64,
}

impl Form {
    /// The stored form of `document`. With `title` that title is kept; without it the
    /// title comes from the text.
    pub(crate) fn of(document: &Document, title: Option<&str>) -> Form {
        Form::counted(document, title, document.stats())
    }

    /// [`Form::of`] for a document whose words and characters are already counted
    /// (`stats` must be what `document.stats()` gives: counting a book takes as long as
    /// reading it from the library does).
    pub(crate) fn counted(document: &Document, title: Option<&str>, stats: Stats) -> Form {
        Form {
            json: document.to_json(),
            title: match title {
                Some(title) => title.to_string(),
                None => derived_title(document),
            },
            plain_text: document.plain_text(),
            words: stats.words as u64,
            characters: stats.characters as u64,
        }
    }

    /// The store's view of it.
    pub(crate) fn content(&self) -> DocumentContent<'_> {
        DocumentContent {
            content: &self.json,
            title: &self.title,
            plain_text: &self.plain_text,
            word_count: self.words,
            char_count: self.characters,
        }
    }
}

/// The counts of `after`, a later state of `before`, from the counts of `before`
/// (`counted`, what `before.stats()` gives): only the blocks whose text is not the same
/// in both are counted, the ones that are gone taken off and the ones that are new or
/// read differently put on. The counts of a document are the sums of the counts of its
/// blocks, so this is what `after.stats()` gives, in the time the changed blocks take
/// instead of the whole book.
pub(crate) fn stats_after(before: &Document, counted: Stats, after: &Document) -> Stats {
    let old: HashMap<BlockId, &str> = before
        .blocks()
        .iter()
        .map(|block| (block.id(), block.text()))
        .collect();
    let mut same: HashSet<BlockId> = HashSet::new();
    let mut added = Stats::default();
    for block in after.blocks() {
        if old.get(&block.id()) == Some(&block.text()) {
            same.insert(block.id());
        } else {
            added += Stats::of_text(block.text());
        }
    }
    let mut removed = Stats::default();
    for block in before.blocks() {
        if !same.contains(&block.id()) {
            removed += Stats::of_text(block.text());
        }
    }
    let stats = Stats {
        words: counted.words + added.words - removed.words,
        characters: counted.characters + added.characters - removed.characters,
        characters_no_spaces: counted.characters_no_spaces + added.characters_no_spaces
            - removed.characters_no_spaces,
        paragraphs: counted.paragraphs + added.paragraphs - removed.paragraphs,
        reading_minutes: 0,
    } + Stats::default();
    // Every change every test makes is counted both ways.
    debug_assert_eq!(stats, after.stats());
    stats
}

/// Why a version is kept: the start of its label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reason {
    /// A piece of text is replaced.
    Edit,
    /// The whole text is replaced.
    Rewrite,
    /// Text is inserted.
    Insertion,
    /// A section is replaced.
    Section,
    /// A comment or a reply is deleted.
    CommentDeleted,
    /// An earlier version is restored.
    Restore,
}

impl Reason {
    /// The label of the version: "Before an edit by Claude".
    pub(crate) fn label(self, author: &str) -> String {
        let what = match self {
            Reason::Edit => "an edit",
            Reason::Rewrite => "a rewrite",
            Reason::Insertion => "an insertion",
            Reason::Section => "a section was replaced",
            Reason::CommentDeleted => "a comment was deleted",
            Reason::Restore => "a version was restored",
        };
        format!("Before {what} by {author}")
    }
}

/// Keeps the stored text of a document as a version before it is changed, with a label
/// that says why. When the newest version already is this text, that one is the version
/// and no second copy is made; it is given the label when it has none (the app took it
/// when it saved the writer's typing a moment ago, and it is the text from before this
/// change all the same). A label it has stays: it says why the version was first kept.
pub(crate) fn keep_version(
    store: &mut Store,
    id: DocumentId,
    raw: &str,
    label: &str,
) -> Result<VersionInfo> {
    if let Some(newest) = store.snapshots(id)?.into_iter().next()
        && store.snapshot(newest.id)?.content == raw
    {
        let label = match store.snapshot_label(newest.id)? {
            Some(label) => label,
            None => {
                store.label_snapshot(newest.id, label)?;
                label.to_string()
            }
        };
        return Ok(version_info(&newest, Some(label)));
    }
    let meta = store.create_labeled_snapshot(id, label)?;
    // What the app does after it takes one: thin out the old ones.
    let policy = store.snapshot_policy().clone();
    store.prune_snapshots(id, &policy)?;
    store.remove_stale_snapshot_labels()?;
    Ok(version_info(&meta, Some(label.to_string())))
}

/// A version as results show it.
pub(crate) fn version_info(meta: &SnapshotMeta, label: Option<String>) -> VersionInfo {
    VersionInfo {
        id: meta.id,
        document_id: meta.document_id,
        created_at: Time(meta.created_at),
        title: meta.title.clone(),
        words: meta.word_count,
        characters: meta.char_count,
        label,
    }
}

/// The settings of a document as results show them.
pub(crate) fn settings_info(settings: &DocumentSettings) -> DocumentSettingsInfo {
    let (page_width, page_height) = settings.page.page_size();
    let margins = settings.page.margins;
    DocumentSettingsInfo {
        page_view: settings.page_view,
        paper: match settings.page.paper {
            PaperSize::A4 => "A4",
            PaperSize::UsLetter => "US Letter",
            PaperSize::Custom { .. } => "Custom",
        }
        .to_string(),
        page_width,
        page_height,
        orientation: match settings.page.orientation {
            Orientation::Portrait => "portrait",
            Orientation::Landscape => "landscape",
        }
        .to_string(),
        margins: [margins.top, margins.right, margins.bottom, margins.left],
        font: settings.typography.body_font.clone(),
        text_size: settings.typography.base_size,
        line_height: settings.typography.line_height,
        paragraph_spacing: settings.typography.paragraph_spacing,
        spelling_language: settings
            .spelling_language
            .as_ref()
            .map(|language| language.as_setting_str().to_string()),
    }
}

/// The settings a new document starts with: the app's "new document" settings as the
/// library holds them (`writer_app::Settings::new_document_settings`).
pub(crate) fn new_document_settings(store: &Store) -> DocumentSettings {
    let mut settings = DocumentSettings::default();
    let font: String = store
        .setting_or("new_document.font", String::new())
        .unwrap_or_default();
    if !font.trim().is_empty() {
        settings.typography.body_font = font;
    }
    if let Ok(size) = store.setting_or("new_document.size", settings.typography.base_size) {
        settings.typography.base_size = size.clamp(writer_core::MIN_TEXT_SIZE, 72.0);
    }
    if let Ok(line_height) =
        store.setting_or("new_document.line_height", settings.typography.line_height)
    {
        settings.typography.line_height = line_height;
    }
    let paper: String = store
        .setting_or("new_document.paper", String::new())
        .unwrap_or_default();
    if paper == "letter" {
        settings.page.paper = PaperSize::UsLetter;
    }
    settings.page_view = store
        .setting_or("new_document.page_view", false)
        .unwrap_or(false);
    settings.sanitized()
}

/// What a change to the text of a document did, block by block.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Delta {
    /// Blocks that are new.
    pub(crate) added: usize,
    /// Blocks that are gone.
    pub(crate) removed: usize,
    /// Blocks that are still there and read differently.
    pub(crate) changed: usize,
    /// The headings whose sections were touched, in the order of the new document.
    pub(crate) headings: Vec<String>,
    /// The comment threads that had a place in the text before and have none now.
    pub(crate) detached: Vec<CommentId>,
}

impl Delta {
    /// Compares two states of one document. Blocks are followed by their ids.
    pub(crate) fn between(before: &Document, after: &Document) -> Delta {
        let old: HashMap<BlockId, usize> = before
            .blocks()
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id(), index))
            .collect();
        let new: HashMap<BlockId, usize> = after
            .blocks()
            .iter()
            .enumerate()
            .map(|(index, block)| (block.id(), index))
            .collect();
        let mut delta = Delta::default();
        // The heading each touched block is under, in the order of the new document;
        // removed blocks count for the heading they were under, when it is still there.
        let mut touched: BTreeSet<usize> = BTreeSet::new();
        let heading_above = |document: &Document, index: usize| -> Option<usize> {
            (0..=index.min(document.block_count() - 1))
                .rev()
                .find(|&i| document.blocks()[i].kind().heading_level().is_some())
        };
        for (index, block) in after.blocks().iter().enumerate() {
            let same = match old.get(&block.id()) {
                None => {
                    delta.added += 1;
                    false
                }
                Some(&was) => {
                    let same = &before.blocks()[was] == block;
                    if !same {
                        delta.changed += 1;
                    }
                    same
                }
            };
            if !same && let Some(heading) = heading_above(after, index) {
                touched.insert(heading);
            }
        }
        for (index, block) in before.blocks().iter().enumerate() {
            if new.contains_key(&block.id()) {
                continue;
            }
            delta.removed += 1;
            let heading = heading_above(before, index)
                .and_then(|heading| new.get(&before.blocks()[heading].id()).copied());
            if let Some(heading) = heading {
                touched.insert(heading);
            }
        }
        delta.headings = touched
            .into_iter()
            .map(|index| after.blocks()[index].text().trim().to_string())
            .collect();
        delta.detached = before
            .comments()
            .map(|thread| thread.id)
            .filter(|&id| {
                !before.is_orphaned(id) && after.comment(id).is_some() && after.is_orphaned(id)
            })
            .collect();
        delta
    }
}

#[cfg(test)]
mod tests {
    use writer_core::{Block, BlockKind, EditorState, Fragment, Position, Selection, TextRange};

    use super::*;

    /// The counts after a change, worked out from the counts before it, are the counts
    /// of the changed document counted whole: for text put in, taken out, replaced and
    /// moved, and for a document that did not change.
    #[test]
    fn the_counts_after_a_change_are_those_of_the_whole_document() {
        let before = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "The Salt Road"),
            Block::paragraph("The keeper could not swim. It was not a secret."),
            Block::new(BlockKind::Divider),
            Block::paragraph("   "),
            Block::paragraph("中文 e\u{301} and a family \u{1F469}\u{200D}\u{1F467}"),
            Block::paragraph("The last one."),
        ]);
        let counted = before.stats();
        let check = |change: &dyn Fn(&mut EditorState)| {
            let mut editor = EditorState::new(before.clone());
            change(&mut editor);
            let after = editor.into_document();
            assert_eq!(stats_after(&before, counted, &after), after.stats());
            after
        };
        // Nothing.
        let same = check(&|_| {});
        assert_eq!(same.stats(), counted);
        // New blocks, at the end and in the middle.
        let more = check(&|editor| {
            let new = Fragment::new(vec![
                Block::paragraph("One more paragraph."),
                Block::paragraph("And another, with seven words in it."),
            ]);
            editor.insert_blocks_at(6, &new);
            editor.insert_blocks_at(2, &new);
        });
        assert_eq!(more.stats().words, counted.words + 2 * 10);
        // A word replaced inside a block, and one made of other characters.
        let edited = check(&|editor| {
            editor.replace_range(
                TextRange::new(Position::new(1, 4), Position::new(1, 10)),
                "lighthouse keeper of the rock",
            );
            editor.replace_range(
                TextRange::new(Position::new(4, 0), Position::new(4, 6)),
                "x",
            );
        });
        assert_ne!(edited.stats(), counted);
        // Blocks taken out, the first and the last among them.
        let fewer = check(&|editor| {
            editor.set_selection(Selection::new(Position::new(4, 0), Position::new(5, 3)));
            editor.delete_blocks();
            editor.set_selection(Selection::caret(Position::new(0, 0)));
            editor.delete_blocks();
        });
        assert_eq!(fewer.block_count(), 3);
        // Blocks in another order count the same.
        let moved = check(&|editor| {
            editor.set_selection(Selection::caret(Position::new(5, 0)));
            editor.move_blocks_up();
            editor.move_blocks_up();
        });
        assert_eq!(moved.stats(), counted);
        // Everything replaced by one paragraph.
        check(&|editor| {
            editor.select_all();
            editor.insert_text("Only this.");
        });
        // And a document that shares no block with the one before.
        let other = Document::from_blocks(vec![Block::paragraph("Something else entirely.")]);
        assert_eq!(stats_after(&before, counted, &other), other.stats());
        assert_eq!(stats_after(&other, other.stats(), &before), counted);
    }
}
