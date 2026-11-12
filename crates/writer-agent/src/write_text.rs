//! Reading and writing text: the text of a document, creating one, replacing its text,
//! editing it in place.

use writer_core::{Block, BlockKind, Document, EditorState, Fragment};
use writer_export::{ExportSource, MarkdownOptions, PlainTextOptions, to_markdown, to_plain_text};
use writer_store::{DocumentId, DocumentMeta, Folder, FolderId, Store};

use crate::comments::thread_infos;
use crate::content::{self, Delta, Form, Loaded, Reason};
use crate::error::{Error, Result};
use crate::items::{place_among, plan_folders};
use crate::library::{Library, Session, author_phrase};
use crate::markdown_in;
use crate::matching;
use crate::resolve::{Index, UNTITLED, join_path};
use crate::rewrite;
use crate::sections::find_heading;
use crate::types::{
    ContentChanged, CreateDocumentRequest, DocumentCreated, DocumentSummary, DocumentText,
    EditRequest, FindRequest, FindResults, FolderSummary, InsertPosition, InsertRequest, MatchedAs,
    ReadRequest, ReplaceDocumentRequest, ReplaceSectionRequest, Target, TextFormat, Time,
    WriteOptions,
};

/// How many occurrences [`Library::find`] returns unless the request says.
const DEFAULT_FIND_LIMIT: usize = 20;

/// The text of `document` in `format`, as [`Library::read`] returns it.
pub(crate) fn render(document: &Document, title: &str, format: TextFormat) -> String {
    match format {
        TextFormat::Markdown => to_markdown(
            &ExportSource::single(title, document),
            &MarkdownOptions::default(),
        ),
        TextFormat::PlainText => to_plain_text(
            &ExportSource::single(title, document),
            &PlainTextOptions::default(),
        ),
        TextFormat::Json => document.to_json(),
    }
}

/// A document open for a change to its text.
struct Open {
    session: Session,
    index: Index,
    loaded: Loaded,
}

impl Library {
    /// Opens the live document a write names.
    fn open_document(&self, target: &Target, options: &WriteOptions) -> Result<Open> {
        let session = self.writer(options)?;
        let index = Index::load(&session.store)?;
        let meta = index.live_document(&session.store, target)?;
        let loaded = content::load(&session.store, &meta)?;
        Ok(Open {
            session,
            index,
            loaded,
        })
    }

    /// The text of a document, or of one section of it: as Markdown, as plain text or as
    /// the native JSON, optionally with its comment threads.
    pub fn read(&self, request: &ReadRequest) -> Result<DocumentText> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, &request.document)?;
        let loaded = content::load(&session.store, &meta)?;
        let document = &loaded.document;
        let summary = index.document_summary(&session.store, &loaded.meta);
        let heading = match &request.heading {
            Some(name) => Some(find_heading(document, name, request.heading_occurrence)?),
            None => None,
        };
        let (content, blocks) = match &heading {
            Some(heading) => {
                // The section as a document of its own, with the settings of the whole.
                let part = Document::from_blocks(document.blocks()[heading.section()].to_vec())
                    .with_settings(document.settings().clone());
                (
                    render(&part, &heading.text, request.format),
                    heading.section(),
                )
            }
            None => (
                render(document, &summary.title, request.format),
                0..document.block_count(),
            ),
        };
        let comments = request.comments.then(|| {
            thread_infos(document)
                .into_iter()
                .filter(|thread| match thread.block {
                    Some(block) => blocks.contains(&block),
                    None => heading.is_none(),
                })
                .collect()
        });
        Ok(DocumentText {
            document: summary,
            format: request.format,
            content,
            section: heading.map(|heading| heading.info(document)),
            comments,
        })
    }

    /// Where a piece of text occurs in a document: what an edit or a comment with this
    /// text would find, without changing anything.
    pub fn find(&self, request: &FindRequest) -> Result<FindResults> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, &request.document)?;
        let loaded = content::load(&session.store, &meta)?;
        let matches = matching::find(&loaded.document, &request.text, false)?;
        Ok(FindResults {
            document: index.document_summary(&session.store, &loaded.meta),
            count: matches.ranges.len(),
            occurrences: matching::occurrences(
                &loaded.document,
                &matches.ranges,
                request.limit.unwrap_or(DEFAULT_FIND_LIMIT),
            ),
            matched_as: matches.matched_as,
        })
    }

    /// Creates a document from Markdown. See [`CreateDocumentRequest`].
    pub fn create_document(&self, request: &CreateDocumentRequest) -> Result<DocumentCreated> {
        let mut session = self.writer(&request.write)?;
        let index = Index::load(&session.store)?;
        let store = &session.store;

        // Where it goes: a folder that is there, or one that is created for it.
        let (parent, missing): (Option<Folder>, Vec<String>) = match &request.folder {
            None => (None, Vec::new()),
            Some(target) => match index.live_folder_or_root(store, target) {
                Ok(folder) => (folder, Vec::new()),
                Err(Error::NotFound { .. }) if request.create_folders => match target {
                    Target::Path(path) => plan_folders(&index, None, path)?,
                    _ => return Err(index.live_folder_or_root(store, target).unwrap_err()),
                },
                Err(error) => return Err(error),
            },
        };

        let settings = content::new_document_settings(store);
        let imported = markdown_in::import(&request.markdown, &settings);
        let document = imported.document;
        let derived = content::derived_title(&document);
        let given = request
            .title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty());
        let chosen = given.is_some_and(|title| title != derived);
        let form = Form::of(&document, given);

        if session.dry_run {
            let mut folder_path = index.folder_segments(store, parent.as_ref().map(|f| f.id));
            let now = Time(self.clock.now_ms());
            let mut created_folders = Vec::new();
            let mut above = parent.as_ref().map(|folder| folder.id);
            for name in &missing {
                folder_path.push(name.clone());
                created_folders.push(FolderSummary {
                    id: FolderId(0),
                    name: name.clone(),
                    path: join_path(&folder_path),
                    parent_id: above,
                    created_at: now,
                    updated_at: now,
                    trashed_at: None,
                });
                above = Some(FolderId(0));
            }
            if missing.is_empty()
                && let Some(placement) = &request.placement
            {
                // Checks that a "before" or "after" names a document of the folder.
                let siblings: Vec<i64> = index
                    .child_documents(above)
                    .iter()
                    .map(|meta| meta.id.0)
                    .collect();
                place_among(&siblings, placement, |target| {
                    Ok(index.live_document(store, target)?.id.0)
                })?;
            }
            let mut path = folder_path.clone();
            path.push(if form.title.trim().is_empty() {
                UNTITLED.to_string()
            } else {
                form.title.trim().to_string()
            });
            let document = DocumentSummary {
                id: DocumentId(0),
                title: form.title.clone(),
                path: join_path(&path),
                folder_id: above,
                folder: join_path(&folder_path),
                words: form.words,
                characters: form.characters,
                created_at: now,
                updated_at: now,
                trashed_at: None,
            };
            return Ok(DocumentCreated {
                document,
                created_folders,
                warnings: imported.warnings,
                written: session.written(true),
            });
        }

        let store = &mut session.store;
        let mut created = Vec::new();
        let mut folder_id = parent.as_ref().map(|folder| folder.id);
        for name in &missing {
            let folder = store.create_folder(folder_id, name)?;
            folder_id = Some(folder.id);
            created.push(folder);
        }
        let place = match &request.placement {
            None => None,
            Some(placement) => {
                let siblings: Vec<i64> = store
                    .documents_in_folder(folder_id)?
                    .iter()
                    .map(|meta| meta.id.0)
                    .collect();
                Some(place_among(&siblings, placement, |target| {
                    Ok(index.live_document(store, target)?.id.0)
                })?)
            }
        };
        let meta = store.create_document(folder_id, &form.content())?;
        if let Some(place) = place {
            store.reorder_document(meta.id, place)?;
        }
        // A flag left behind by an earlier document with this id must not hold for it.
        content::set_title_flag(store, &meta, chosen)?;
        let meta = store.document_meta(meta.id)?;
        let result = DocumentCreated {
            document: index.document_summary(store, &meta),
            created_folders: created
                .iter()
                .map(|folder| index.folder_summary(store, folder))
                .collect(),
            warnings: imported.warnings,
            written: session.written(true),
        };
        session.finish()?;
        Ok(result)
    }

    /// Replaces the whole text of a document with Markdown, changing as little of the
    /// stored document as the new text allows. See [`ReplaceDocumentRequest`].
    pub fn replace_document(&self, request: &ReplaceDocumentRequest) -> Result<ContentChanged> {
        let open = self.open_document(&request.document, &request.write)?;
        let before = &open.loaded.document;
        let imported = markdown_in::import(&request.markdown, before.settings());
        let mut ed = EditorState::new(before.clone());
        rewrite::rewrite_blocks(&mut ed, 0..before.block_count(), imported.blocks());
        let after = ed.into_document();
        self.finish_change(
            open,
            after,
            Reason::Rewrite,
            &request.write,
            Outcome {
                warnings: imported.warnings,
                ..Outcome::default()
            },
        )
    }

    /// Replaces an exact piece of text: once, or everywhere. See [`EditRequest`].
    pub fn edit(&self, request: &EditRequest) -> Result<ContentChanged> {
        if request.all && request.occurrence.is_some() {
            return Err(Error::invalid(
                "\"all\" and \"occurrence\" exclude each other: every occurrence, or one",
            ));
        }
        if request.old_text.trim_matches(['\n', '\r']).is_empty() {
            return Err(Error::invalid(
                "the text to replace is empty; to add text, insert it (at the end, at the \
                 start or at a heading)",
            ));
        }
        let open = self.open_document(&request.document, &request.write)?;
        let before = &open.loaded.document;
        let matches = matching::find(before, &request.old_text, request.literal)?;
        let ranges = matching::choose(
            before,
            &request.old_text,
            &matches,
            request.all,
            request.occurrence,
        )?;
        let exact = matches.matched_as == Some(MatchedAs::Markdown);
        let mut ed = EditorState::new(before.clone());
        let mut warnings: Vec<String> = Vec::new();
        // From the end, so that the places before a replacement are where they were.
        for range in ranges.iter().rev() {
            for warning in rewrite::replace_text(
                &mut ed,
                *range,
                &request.new_text,
                request.literal,
                exact,
                before.settings(),
            ) {
                if !warnings.contains(&warning) {
                    warnings.push(warning);
                }
            }
        }
        let after = ed.into_document();
        self.finish_change(
            open,
            after,
            Reason::Edit,
            &request.write,
            Outcome {
                replacements: ranges.len(),
                matched_as: matches.matched_as,
                warnings,
            },
        )
    }

    /// Inserts Markdown as new blocks: at the end (append), at the start, or at a heading.
    /// See [`InsertRequest`].
    pub fn insert(&self, request: &InsertRequest) -> Result<ContentChanged> {
        if request.position.needs_heading() && request.heading.is_none() {
            return Err(Error::invalid(
                "this position is relative to a heading: name the heading",
            ));
        }
        if !request.position.needs_heading() && request.heading.is_some() {
            return Err(Error::invalid(
                "a heading was named, but the position is not relative to one: choose \
                 before_heading, after_heading or end_of_section",
            ));
        }
        let open = self.open_document(&request.document, &request.write)?;
        let before = &open.loaded.document;
        let imported = markdown_in::import(&request.markdown, before.settings());
        let blocks = imported.blocks();
        let count = before.block_count();
        let heading = match &request.heading {
            Some(name) => Some(find_heading(before, name, request.heading_occurrence)?),
            None => None,
        };
        let at = match (request.position, &heading) {
            (InsertPosition::Start, _) => 0,
            (InsertPosition::BeforeHeading, Some(heading)) => heading.block,
            (InsertPosition::AfterHeading, Some(heading)) => heading.block + 1,
            (InsertPosition::EndOfSection, Some(heading)) => heading.end,
            // The end: before the empty paragraphs a document may end with.
            _ => {
                let empty =
                    |block: &Block| *block.kind() == BlockKind::Paragraph && block.is_empty();
                count
                    - before
                        .blocks()
                        .iter()
                        .rev()
                        .take_while(|block| empty(block))
                        .count()
                        .min(count - 1)
            }
        };
        let mut ed = EditorState::new(before.clone());
        if !blocks.is_empty() {
            let blank = count == 1
                && *before.blocks()[0].kind() == BlockKind::Paragraph
                && before.blocks()[0].is_empty();
            if blank {
                // An empty document: the text takes the place of its empty paragraph.
                rewrite::replace_block_range(&mut ed, 0..1, blocks);
            } else {
                ed.insert_blocks_at(at, &Fragment::new(blocks.to_vec()));
            }
        }
        let after = ed.into_document();
        self.finish_change(
            open,
            after,
            Reason::Insertion,
            &request.write,
            Outcome {
                warnings: imported.warnings,
                ..Outcome::default()
            },
        )
    }

    /// Replaces the section under a heading with Markdown. See [`ReplaceSectionRequest`].
    pub fn replace_section(&self, request: &ReplaceSectionRequest) -> Result<ContentChanged> {
        let open = self.open_document(&request.document, &request.write)?;
        let before = &open.loaded.document;
        let heading = find_heading(before, &request.heading, request.heading_occurrence)?;
        let imported = markdown_in::import(&request.markdown, before.settings());
        let range = if request.include_heading {
            heading.section()
        } else {
            heading.block + 1..heading.end
        };
        let mut ed = EditorState::new(before.clone());
        rewrite::rewrite_blocks(&mut ed, range, imported.blocks());
        let after = ed.into_document();
        self.finish_change(
            open,
            after,
            Reason::Section,
            &request.write,
            Outcome {
                warnings: imported.warnings,
                ..Outcome::default()
            },
        )
    }

    /// Stores the changed document (unless this is a dry run or nothing changed), after
    /// keeping the text as it was as a version, and reports what the change did.
    fn finish_change(
        &self,
        open: Open,
        after: Document,
        reason: Reason,
        options: &WriteOptions,
        outcome: Outcome,
    ) -> Result<ContentChanged> {
        let Open {
            mut session,
            index,
            loaded,
        } = open;
        let before = &loaded.document;
        let changed = *before != after;
        let delta = Delta::between(before, &after);
        let (stats_before, stats_after) = (before.stats(), after.stats());
        let detached = thread_infos(&after)
            .into_iter()
            .filter(|thread| delta.detached.iter().any(|id| id.0 == thread.id))
            .collect();
        let kept_title = loaded.custom_title.then(|| loaded.meta.title.clone());
        let (meta, snapshot) = if !changed {
            (loaded.meta.clone(), None)
        } else if session.dry_run {
            let form = Form::of(&after, kept_title.as_deref());
            (
                DocumentMeta {
                    title: form.title,
                    word_count: form.words,
                    char_count: form.characters,
                    ..loaded.meta.clone()
                },
                None,
            )
        } else {
            let label = reason.label(&author_phrase(options));
            let (meta, version) = store_change(
                &mut session.store,
                &loaded,
                &after,
                kept_title.as_deref(),
                &label,
            )?;
            (meta, Some(version))
        };
        let result = ContentChanged {
            document: index.document_summary(&session.store, &meta),
            written: session.written(changed),
            replacements: outcome.replacements,
            matched_as: outcome.matched_as,
            blocks_added: delta.added,
            blocks_removed: delta.removed,
            blocks_changed: delta.changed,
            words_before: stats_before.words as u64,
            words_after: stats_after.words as u64,
            characters_before: stats_before.characters as u64,
            characters_after: stats_after.characters as u64,
            headings: delta.headings,
            warnings: outcome.warnings,
            comments_detached: detached,
            snapshot,
        };
        session.finish()?;
        Ok(result)
    }
}

/// What an operation knows about its own change, beyond the two documents.
#[derive(Default)]
struct Outcome {
    replacements: usize,
    matched_as: Option<MatchedAs>,
    warnings: Vec<String>,
}

/// Keeps the stored text as a version, then stores `after` the way the app stores a
/// document.
pub(crate) fn store_change(
    store: &mut Store,
    loaded: &Loaded,
    after: &Document,
    kept_title: Option<&str>,
    label: &str,
) -> Result<(DocumentMeta, crate::types::VersionInfo)> {
    let version = content::keep_version(store, loaded.meta.id, &loaded.raw, label)?;
    let form = Form::of(after, kept_title);
    let meta = store.save_document(loaded.meta.id, &form.content())?;
    Ok((meta, version))
}
