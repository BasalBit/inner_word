//! Reading and writing text: the text of a document, creating one, replacing its text,
//! editing it in place.

use std::collections::HashMap;

use innerword_core::{
    Alignment, Block, BlockKind, Document, EditorState, Fragment, ImageId, ImageWidth, PageSetup,
};
use innerword_export::{
    ExportSource, MarkdownOptions, PlainTextOptions, to_markdown, to_plain_text,
};
use innerword_store::{DocumentId, DocumentMeta, Folder, FolderId, Store};

use crate::attach;
use crate::comments::thread_infos;
use crate::content::{self, Delta, Form, Loaded, Reason};
use crate::error::{Error, Result};
use crate::items::{place_among, plan_folders};
use crate::library::{Library, Session, author_phrase};
use crate::live::Change;
use crate::markdown_in::{Images, Reader};
use crate::matching;
use crate::resolve::{Index, UNTITLED, join_path};
use crate::rewrite;
use crate::sections::{Heading, find_heading};
use crate::types::{
    AddImageRequest, ContentChanged, CreateDocumentRequest, DocumentCreated, DocumentSummary,
    DocumentText, EditRequest, FindRequest, FindResults, FolderSummary, ImageSummary,
    InsertPosition, InsertRequest, MatchedAs, POSITION_NEEDS_A_HEADING, POSITION_TAKES_NO_HEADING,
    ReadRequest, ReplaceDocumentRequest, ReplaceSectionRequest, Target, TextFormat, Time,
    WriteOptions,
};

/// How many occurrences [`Library::find`] returns unless the request says.
const DEFAULT_FIND_LIMIT: usize = 20;

/// The text of `document` in `format`, as [`Library::read`] returns it. In Markdown an
/// image is a line `![description](image:<id>)` (`MarkdownImages::References`, the
/// exporter's default): what an agent keeps where the image should stay.
///
/// The plain text is the text an edit and a comment match against, so an image is in it
/// what it is there: no text (the blocks around it are a paragraph apart, as two
/// paragraphs are). Its description is left out, unlike in a plain-text export, where it
/// stands for the picture: words an agent reads there and quotes back would be found
/// nowhere. The description is in the list of images and in the Markdown.
pub(crate) fn render(document: &Document, title: &str, format: TextFormat) -> String {
    match format {
        TextFormat::Markdown => to_markdown(
            &ExportSource::single(title, document),
            &MarkdownOptions::default(),
        ),
        TextFormat::PlainText if document.has_images() => {
            let blocks = document
                .blocks()
                .iter()
                .map(|block| match block.as_image() {
                    Some(image) if !image.description.is_empty() => {
                        let mut undescribed = image.clone();
                        undescribed.description.clear();
                        Block::image(undescribed).with_alignment(block.alignment())
                    }
                    _ => block.clone(),
                })
                .collect();
            let matched = Document::from_blocks(blocks).with_settings(document.settings().clone());
            to_plain_text(
                &ExportSource::single(title, &matched),
                &PlainTextOptions::default(),
            )
        }
        TextFormat::PlainText => to_plain_text(
            &ExportSource::single(title, document),
            &PlainTextOptions::default(),
        ),
        TextFormat::Json => document.to_json(),
    }
}

/// The line `image` is in the Markdown of a document, as [`render`] writes it: the
/// exporter's own line, so that the description is escaped as it is there.
pub(crate) fn image_markdown(image: &innerword_core::Image) -> String {
    let document = Document::from_blocks(vec![Block::image(image.clone())]);
    render(&document, "", TextFormat::Markdown)
        .trim()
        .to_string()
}

/// An image block as a result shows it; `None` for a block that is no image. `index` is
/// the place of the block in its document, whose page is `page`.
fn image_summary(
    store: &Store,
    page: &PageSetup,
    index: usize,
    block: &Block,
) -> Result<Option<ImageSummary>> {
    let Some(image) = block.as_image() else {
        return Ok(None);
    };
    let stored = store.image_info(&innerword_store::ImageId(*image.id.as_bytes()))?;
    Ok(Some(ImageSummary {
        id: image.id.to_string(),
        block: index,
        format: stored.as_ref().map(|info| info.format.as_str().to_string()),
        pixels: image.pixels,
        // What is drawn: a width the page no longer allows is drawn at what it allows.
        width_percent: image.width.min(image.max_width(page)).percent(),
        align: block.alignment().name().to_string(),
        description: image.description.clone(),
        missing: stored.is_none(),
        markdown: image_markdown(image),
    }))
}

/// How many times each picture stands in `document`.
fn image_counts(document: &Document) -> HashMap<ImageId, usize> {
    let mut counts = HashMap::new();
    for (_, image) in document.images() {
        *counts.entry(image.id).or_insert(0) += 1;
    }
    counts
}

/// A document open for a change to its text.
struct Open {
    index: Index,
    loaded: Loaded,
}

/// Opens the live document a write names.
fn open_document(session: &Session<'_>, target: &Target) -> Result<Open> {
    let index = Index::load(session.store)?;
    let meta = index.live_document(session.store, target)?;
    let loaded = content::load(session.store, &meta)?;
    Ok(Open { index, loaded })
}

impl Library {
    /// The text of a document, or of one section of it: as Markdown, as plain text or as
    /// the native JSON, optionally with its comment threads.
    pub fn read(&self, request: &ReadRequest) -> Result<DocumentText> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, &request.document)?;
        if meta.kind == innerword_store::DocumentKind::Page {
            return crate::pages::read(&session.store, &index, &meta, request);
        }
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
        let mut images = Vec::new();
        if document.has_images() {
            let page = &document.settings().page;
            for index in blocks.clone() {
                images.extend(image_summary(
                    &session.store,
                    page,
                    index,
                    &document.blocks()[index],
                )?);
            }
        }
        Ok(DocumentText {
            document: summary,
            format: request.format,
            content,
            section: heading.map(|heading| heading.info(document)),
            comments,
            images,
            page: None,
        })
    }

    /// Where a piece of text occurs in a document: what an edit or a comment with this
    /// text would find, without changing anything.
    pub fn find(&self, request: &FindRequest) -> Result<FindResults> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, &request.document)?;
        if meta.kind == innerword_store::DocumentKind::Page {
            return crate::pages::find(&session.store, &index, &meta, request);
        }
        let loaded = content::load(&session.store, &meta)?;
        let matches = matching::find(&loaded.document, &request.text, false)?;
        Ok(FindResults {
            document: index.document_summary(&session.store, &loaded.meta),
            count: matches.ranges.len(),
            occurrences: matching::occurrences(
                &loaded.document,
                &matches.ranges,
                matches.matched_as,
                request.limit.unwrap_or(DEFAULT_FIND_LIMIT),
            ),
            matched_as: matches.matched_as,
        })
    }
}

/// Creates a document from Markdown. See [`CreateDocumentRequest`].
pub(crate) fn create_document(
    session: &mut Session<'_>,
    request: &CreateDocumentRequest,
) -> Result<DocumentCreated> {
    let index = Index::load(session.store)?;
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
    let mut images = Images::new(&request.images, session.dry_run)?;
    let imported = Reader {
        store: &mut *session.store,
        images: &mut images,
    }
    .read(&request.markdown, &settings)?;
    let store = &session.store;
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
        let now = Time(session.clock.now_ms());
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
            kind: innerword_store::DocumentKind::Text,
            source_url: None,
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
        session
            .changes
            .push(Change::FolderCreated { id: folder.id });
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
    session
        .changes
        .push(Change::DocumentCreated { id: meta.id });
    if chosen {
        session
            .changes
            .push(Change::DocumentRenamed { id: meta.id });
    }
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
    Ok(result)
}

/// Replaces the whole text of a document with Markdown, changing as little of the
/// stored document as the new text allows. See [`ReplaceDocumentRequest`].
pub(crate) fn replace_document(
    session: &mut Session<'_>,
    request: &ReplaceDocumentRequest,
) -> Result<ContentChanged> {
    let open = open_document(session, &request.document)?;
    let before = &open.loaded.document;
    let mut images = Images::new(&request.images, session.dry_run)?;
    images.know(before.blocks(), &[], &[]);
    let imported = Reader {
        store: &mut *session.store,
        images: &mut images,
    }
    .read(&request.markdown, before.settings())?;
    let mut ed = EditorState::new(before.clone());
    rewrite::rewrite_blocks(&mut ed, 0..before.block_count(), imported.blocks());
    let after = ed.into_document();
    finish_change(
        session,
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
pub(crate) fn edit(session: &mut Session<'_>, request: &EditRequest) -> Result<ContentChanged> {
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
    let open = open_document(session, &request.document)?;
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
    // An image is an empty line in the text that is matched: replacing a match that
    // runs across one would take it away without a word about it.
    if ranges
        .iter()
        .any(|range| matching::across_an_image(before, range, matches.matched_as))
    {
        return Err(Error::invalid(matching::ACROSS_AN_IMAGE));
    }
    let mut images = Images::new(&request.images, session.dry_run)?;
    let mut ed = EditorState::new(before.clone());
    let mut warnings: Vec<String> = Vec::new();
    // From the end, so that the places before a replacement are where they were.
    for range in ranges.iter().rev() {
        // An image the new text names again is the one the match held, then any other
        // of the document. (Asked only of a document that has one: a text without images
        // is not gone through once more for every place of an edit of all of them.)
        if before.has_images() {
            let (first, end) = (range.start.block, range.end.block + 1);
            let blocks = before.blocks();
            images.know(&blocks[first..end], &blocks[..first], &blocks[end..]);
        }
        let mut reader = Reader {
            store: &mut *session.store,
            images: &mut images,
        };
        for warning in rewrite::replace_text(
            &mut ed,
            *range,
            &request.new_text,
            request.literal,
            exact,
            before.settings(),
            &mut reader,
        )? {
            if !warnings.contains(&warning) {
                warnings.push(warning);
            }
        }
    }
    let after = ed.into_document();
    finish_change(
        session,
        open,
        after,
        Reason::Edit,
        &request.write,
        Outcome {
            replacements: ranges.len(),
            matched_as: matches.matched_as,
            warnings,
            ..Outcome::default()
        },
    )
}

/// Checks that a position and a heading go together: the positions that are relative to
/// a heading name one, the others none.
fn check_place(position: InsertPosition, heading: bool) -> Result<()> {
    if position.needs_heading() && !heading {
        return Err(Error::invalid(POSITION_NEEDS_A_HEADING));
    }
    if !position.needs_heading() && heading {
        return Err(Error::invalid(POSITION_TAKES_NO_HEADING));
    }
    Ok(())
}

/// The index at which new blocks go into `document` for `position`.
fn insertion_point(
    document: &Document,
    position: InsertPosition,
    heading: Option<&Heading>,
) -> usize {
    let count = document.block_count();
    match (position, heading) {
        (InsertPosition::Start, _) => 0,
        (InsertPosition::BeforeHeading, Some(heading)) => heading.block,
        (InsertPosition::AfterHeading, Some(heading)) => heading.block + 1,
        (InsertPosition::EndOfSection, Some(heading)) => heading.end,
        // The end: before the empty paragraphs a document may end with.
        _ => {
            let empty = |block: &Block| *block.kind() == BlockKind::Paragraph && block.is_empty();
            count
                - document
                    .blocks()
                    .iter()
                    .rev()
                    .take_while(|block| empty(block))
                    .count()
                    .min(count - 1)
        }
    }
}

/// Puts `blocks` into the document of `ed` (which is still `before`) at block `at`: as
/// new blocks, or in the place of the one empty paragraph of a document without text.
/// Says where the first of them is afterwards.
fn insert_blocks(ed: &mut EditorState, before: &Document, at: usize, blocks: &[Block]) -> usize {
    if blocks.is_empty() {
        return at;
    }
    let blank = before.block_count() == 1
        && *before.blocks()[0].kind() == BlockKind::Paragraph
        && before.blocks()[0].is_empty();
    if blank {
        // An empty document: the text takes the place of its empty paragraph.
        rewrite::replace_block_range(ed, 0..1, blocks);
        0
    } else {
        ed.insert_blocks_at(at, &Fragment::new(blocks.to_vec()));
        at
    }
}

/// Inserts Markdown as new blocks: at the end (append), at the start, or at a heading.
/// See [`InsertRequest`].
pub(crate) fn insert(session: &mut Session<'_>, request: &InsertRequest) -> Result<ContentChanged> {
    check_place(request.position, request.heading.is_some())?;
    let open = open_document(session, &request.document)?;
    let before = &open.loaded.document;
    let heading = match &request.heading {
        Some(name) => Some(find_heading(before, name, request.heading_occurrence)?),
        None => None,
    };
    // An image the text names again is the one the document has, as it is there.
    let mut images = Images::new(&request.images, session.dry_run)?;
    images.know(&[], before.blocks(), &[]);
    let imported = Reader {
        store: &mut *session.store,
        images: &mut images,
    }
    .read(&request.markdown, before.settings())?;
    let at = insertion_point(before, request.position, heading.as_ref());
    let mut ed = EditorState::new(before.clone());
    insert_blocks(&mut ed, before, at, imported.blocks());
    let after = ed.into_document();
    finish_change(
        session,
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

/// Adds an image from a file to a document, as a block of its own. See
/// [`AddImageRequest`]. The file was read by whoever asked
/// ([`crate::attach::Prepare`]): what is stored here is what the request brought.
pub(crate) fn add_image(
    session: &mut Session<'_>,
    request: &AddImageRequest,
) -> Result<ContentChanged> {
    check_place(request.position, request.heading.is_some())?;
    let alignment = match request.align.as_deref().map(str::trim) {
        None | Some("") => Alignment::Center,
        Some(name) => match Alignment::from_name(name) {
            Some(alignment) if alignment != Alignment::Justify => alignment,
            _ => {
                return Err(Error::invalid(
                    "an image stands at the left, in the middle or at the right of its line: \
                     align is \"left\", \"center\" or \"right\"",
                ));
            }
        },
    };
    if request
        .width_percent
        .is_some_and(|percent| !(5..=100).contains(&percent))
    {
        return Err(Error::invalid(
            "the width of an image is a share of the text column: from 5 to 100 percent",
        ));
    }
    let attached = match request.images.as_slice() {
        [one] => one,
        [] => {
            return Err(Error::invalid(
                "the request brought no image: the file is read by whoever asks, before the \
                 request is carried out",
            ));
        }
        _ => return Err(Error::invalid("one image is added at a time")),
    };
    if let Some(problem) = &attached.problem {
        return Err(Error::invalid(problem.clone()));
    }
    let open = open_document(session, &request.document)?;
    let before = &open.loaded.document;
    let heading = match &request.heading {
        Some(name) => Some(find_heading(before, name, request.heading_occurrence)?),
        None => None,
    };
    let at = insertion_point(before, request.position, heading.as_ref());
    // The bytes first: a document never names a picture the library does not have yet.
    let mut images = Images::new(&request.images, session.dry_run)?;
    let Some(mut image) = images.store(&mut *session.store, attached)? else {
        return Err(Error::invalid(format!(
            "{} did not arrive as an image the library can hold",
            attach::subject(&attached.source)
        )));
    };
    let page = &before.settings().page;
    image.description = request.description.clone();
    image.width = match request.width_percent {
        // Never taller than a page: the width that is stored is the width that is drawn.
        Some(percent) => ImageWidth::from_percent(percent).min(image.max_width(page)),
        None => image.original_width(page),
    };
    let block = Block::image(image).with_alignment(alignment);
    let mut ed = EditorState::new(before.clone());
    let index = insert_blocks(&mut ed, before, at, std::slice::from_ref(&block));
    let after = ed.into_document();
    let added = after.blocks().get(index).and_then(Block::as_image);
    let image = added.map(|image| ImageSummary {
        id: image.id.to_string(),
        block: index,
        format: Some(attached.format.clone()),
        pixels: image.pixels,
        width_percent: image.width.percent(),
        align: after.blocks()[index].alignment().name().to_string(),
        description: image.description.clone(),
        missing: false,
        markdown: image_markdown(image),
    });
    finish_change(
        session,
        open,
        after,
        Reason::Insertion,
        &request.write,
        Outcome {
            image,
            ..Outcome::default()
        },
    )
}

/// Replaces the section under a heading with Markdown. See [`ReplaceSectionRequest`].
pub(crate) fn replace_section(
    session: &mut Session<'_>,
    request: &ReplaceSectionRequest,
) -> Result<ContentChanged> {
    let open = open_document(session, &request.document)?;
    let before = &open.loaded.document;
    let heading = find_heading(before, &request.heading, request.heading_occurrence)?;
    let range = if request.include_heading {
        heading.section()
    } else {
        heading.block + 1..heading.end
    };
    // An image the new text names is first one of the section, then one of the rest of
    // the document (a line that came over from another section brings its image as it
    // is there).
    let blocks = before.blocks();
    let mut images = Images::new(&request.images, session.dry_run)?;
    images.know(
        &blocks[range.clone()],
        &blocks[..range.start],
        &blocks[range.end..],
    );
    let imported = Reader {
        store: &mut *session.store,
        images: &mut images,
    }
    .read(&request.markdown, before.settings())?;
    let mut ed = EditorState::new(before.clone());
    rewrite::rewrite_blocks(&mut ed, range, imported.blocks());
    let after = ed.into_document();
    finish_change(
        session,
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
    session: &mut Session<'_>,
    open: Open,
    after: Document,
    reason: Reason,
    options: &WriteOptions,
    outcome: Outcome,
) -> Result<ContentChanged> {
    let Open { index, loaded } = open;
    let before = &loaded.document;
    let changed = *before != after;
    let delta = Delta::between(before, &after);
    // Counting a book takes as long as reading it from the library: it is counted
    // once, and the changed text from there.
    let stats_before = before.stats();
    let stats_after = content::stats_after(before, stats_before, &after);
    let detached = thread_infos(&after)
        .into_iter()
        .filter(|thread| delta.detached.iter().any(|id| id.0 == thread.id))
        .collect();
    // Images by their pictures: one that only changed its place counts for nothing.
    let (had, has) = (image_counts(before), image_counts(&after));
    let more_than = |a: &HashMap<ImageId, usize>, b: &HashMap<ImageId, usize>| -> usize {
        a.iter()
            .map(|(id, count)| count.saturating_sub(b.get(id).copied().unwrap_or(0)))
            .sum()
    };
    let (images_added, images_removed) = (more_than(&has, &had), more_than(&had, &has));
    let mut warnings = outcome.warnings;
    if images_removed > 0 {
        // A text that was written back without the line of an image reads as "take it
        // out". Whoever did not mean that is told where the image still is.
        warnings.push(if images_removed == 1 {
            "1 image was removed; it is in the version kept before this change".to_string()
        } else {
            format!(
                "{images_removed} images were removed; they are in the version kept before \
                 this change"
            )
        });
    }
    let kept_title = loaded.custom_title.then(|| loaded.meta.title.clone());
    let (meta, snapshot) = if !changed {
        (loaded.meta.clone(), None)
    } else if session.dry_run {
        let form = Form::counted(&after, kept_title.as_deref(), stats_after);
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
        let id = loaded.meta.id;
        let version = content::keep_version(session.store, id, &loaded.raw, &label)?;
        session.note(Change::VersionKept { document: id });
        let form = Form::counted(&after, kept_title.as_deref(), stats_after);
        let meta = session.store.save_document(id, &form.content())?;
        session.note(Change::DocumentText { id });
        if !delta.detached.is_empty() {
            session.note(Change::DocumentComments { id });
        }
        (meta, Some(version))
    };
    let result = ContentChanged {
        document: index.document_summary(session.store, &meta),
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
        images_added,
        images_removed,
        image: outcome.image,
        warnings,
        comments_detached: detached,
        snapshot,
    };
    Ok(result)
}

/// What an operation knows about its own change, beyond the two documents.
#[derive(Default)]
struct Outcome {
    replacements: usize,
    matched_as: Option<MatchedAs>,
    warnings: Vec<String>,
    /// For `add_image`: the image that was added.
    image: Option<ImageSummary>,
}
