//! Whiteboards: drawings in the library, drawn in the app with Excalidraw
//! (docs/whiteboard-design.md, sections 2, 5 and 7).
//!
//! A whiteboard is a row of `documents` of the kind `board`, whose content is its stored
//! scene (`innerword_board::Scene::to_stored`) and whose pictures are files of
//! `images.db`. Everything an agent writes on one is made in pure Rust by
//! `innerword_board` ([`innerword_board::add`], [`innerword_board::update`],
//! [`innerword_board::remove`]), with every id, seed and nonce derived from the request and
//! the whiteboard ([`HashedMaking`]): the operation runs the same in the tool, on the
//! app's main thread and in a dry run, and gives the same elements each time.
//!
//! A picture file an `image` skeleton names is read in the process that was asked
//! ([`Prepare`] for [`CreateBoardRequest`] and [`AddToBoardRequest`]): an SVG document is
//! kept as it came, anything else is made by `innerword_image::prepare`, and the bytes
//! travel with the request ([`BoardPicture`]). Where the request is carried out they are
//! stored before the scene names them: by the operation itself when the tool writes, by
//! the app's listener when the app does (`live::store_page_files`).

use std::borrow::Cow;
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};

use innerword_board::{
    Added, BoardError, Element, ElementChange, ElementType, FileHex, HashedMaking, ListingHeader,
    Picture, Pictures, Scene,
};
use innerword_core::{Document, Fragment, Stats};
use innerword_store::{
    BoardContent, ContentStamp, DocumentId, DocumentKind, DocumentMeta, FileId, Folder, FolderId,
    ImageFormat, Snapshot, Store, StoreError,
};
use serde::Serialize;
use serde_json::Value;

use crate::attach::{Destination, Prepare, destination, file_of, subject};
use crate::content::{self, Reason, version_info};
use crate::error::{Error, ItemKind, Result, quoted_excerpt};
use crate::items::{place_among, plan_folders};
use crate::library::{Library, Session, author_phrase};
use crate::live::{Change, MAX_ATTACHED_BYTES};
use crate::matching;
use crate::resolve::{Index, UNTITLED, join_path, title_of};
use crate::types::{
    AddToBoardRequest, BoardChanged, BoardInfo, BoardListed, BoardPicture, BoardsList,
    BoardsRequest, CreateBoardRequest, DocumentInfo, DocumentSummary, DocumentText, ExportFormat,
    ExportRequest, Exported, FindRequest, FindResults, PictureAdded, RemoveFromBoardRequest,
    RenameDocumentRequest, RestoreVersionRequest, Target, TextFormat, Time, UpdateBoardRequest,
    VersionInfo, VersionRestored, VersionText, WriteOptions,
};

/// What an export of a whiteboard as a PDF says without a printer.
pub(crate) const NO_PRINTER: &str = "the PDF of a whiteboard cannot be made here";

/// What is said of a picture whose bytes are not the ones its name says, or not of the
/// kind the request says.
pub(crate) const DAMAGED_PICTURE: &str = "a picture of the whiteboard as it was sent is damaged: \
     its bytes are not the ones its name says, so nothing was changed; add it again";

/// What is said when the pictures of one request weigh more than
/// [`MAX_ATTACHED_BYTES`] together.
pub(crate) const TOO_MUCH_PICTURES: &str =
    "the pictures of one request can be 32 MB together at most; add them in several requests";

/// How many occurrences a find in a whiteboard returns unless the request says.
const DEFAULT_FIND_LIMIT: usize = 20;

/// The media type of an SVG picture.
const SVG: &str = "image/svg+xml";

// ----- the stored scene ----------------------------------------------------------------------

/// The scene of the whiteboard `meta` from stored content (its own, or a version's), with
/// what was left out of it as warnings.
fn scene_of(meta: &DocumentMeta, content: &str) -> Result<(Scene, Vec<String>)> {
    if content.trim().is_empty() {
        return Ok((Scene::empty(), Vec::new()));
    }
    let parsed = Scene::from_stored(content).map_err(|error| Error::Unreadable {
        id: meta.id.0,
        name: quoted_excerpt(title_of(meta)),
        reason: error.to_string(),
    })?;
    Ok((parsed.scene, parsed.warnings))
}

/// The scene of the whiteboard `meta` as it is stored now, its content, and what was
/// left out of it.
fn stored_scene(store: &Store, meta: &DocumentMeta) -> Result<(Scene, String, Vec<String>)> {
    let raw = store.document(meta.id)?.content;
    let (scene, warnings) = scene_of(meta, &raw)?;
    Ok((scene, raw, warnings))
}

/// The text of a whiteboard as a document of paragraphs, one per line: for counting and
/// finding, by the rule of a text.
fn text_document(text: &str) -> Document {
    Fragment::from_plain_text(text).to_document()
}

/// What the store is given for a scene: its stored form, the files it names, its text and
/// the counts of that text.
struct Stored {
    scene: String,
    files: Vec<FileId>,
    text: String,
    stats: Stats,
}

impl Stored {
    fn of(scene: &Scene) -> Stored {
        let text = scene.text();
        Stored {
            scene: scene.to_stored(),
            files: scene
                .stored_files()
                .iter()
                .filter_map(|hex| FileId::parse(hex.as_str()))
                .collect(),
            stats: text_document(&text).stats(),
            text,
        }
    }

    fn content(&self) -> BoardContent<'_> {
        BoardContent {
            scene: &self.scene,
            files: &self.files,
            plain_text: &self.text,
            word_count: self.stats.words as u64,
            char_count: self.stats.characters as u64,
        }
    }
}

/// Whether the library has the bytes of a picture (for the listing: "stored" or
/// "missing").
fn has_picture(store: &Store) -> impl Fn(&FileHex) -> bool + '_ {
    move |hex: &FileHex| {
        FileId::parse(hex.as_str())
            .and_then(|id| store.file_info(&id).ok().flatten())
            .is_some()
    }
}

/// The listing of a whiteboard's scene: a header, then one element per line.
fn listing(scene: &Scene, summary: &DocumentSummary, has: &dyn Fn(&FileHex) -> bool) -> String {
    scene.listing_with(
        &ListingHeader {
            reference: &summary.reference(),
            title: &summary.title,
            folder: Some(summary.folder.as_str()).filter(|folder| !folder.is_empty()),
        },
        has,
    )
}

/// Whether `element` is a label: a text bound to a shape or an arrow of the scene.
fn is_label(element: &Element, scene: &Scene) -> bool {
    element.kind() == ElementType::Text
        && element
            .container_id()
            .is_some_and(|container| scene.element(container).is_some())
}

/// How many elements a whiteboard has as its listing counts them (a label is part of its
/// shape or arrow), and how many pictures.
fn counted(scene: &Scene) -> (u64, u64) {
    let elements = scene
        .elements()
        .iter()
        .filter(|element| !is_label(element, scene))
        .count();
    let pictures = scene
        .elements()
        .iter()
        .filter(|element| element.kind() == ElementType::Image)
        .count();
    (elements as u64, pictures as u64)
}

// ----- reading -------------------------------------------------------------------------------

/// The ids of a folder and of every folder inside it.
fn subtree(node: &innerword_store::FolderNode, ids: &mut HashSet<FolderId>) {
    ids.insert(node.folder.id);
    for child in &node.folders {
        subtree(child, ids);
    }
}

impl Library {
    /// The whiteboards of the library, live and in the trash (marked), the one updated
    /// last first, each with how many elements and pictures it has. With a folder: the
    /// whiteboards in it, at any depth. A whiteboard whose stored scene cannot be read is
    /// listed too, with no counts and why ([`BoardListed::unreadable`]).
    pub fn boards(&self, request: &BoardsRequest) -> Result<BoardsList> {
        let session = self.reader()?;
        let store = &session.store;
        let index = Index::load(store)?;
        let within: Option<HashSet<FolderId>> = match &request.folder {
            Some(target) => index.live_folder_or_root(store, target)?.map(|folder| {
                let mut ids = HashSet::new();
                if let Some(node) = index.node(folder.id) {
                    subtree(node, &mut ids);
                }
                ids
            }),
            None => None,
        };
        let mut boards = Vec::new();
        for meta in store.boards()? {
            if let Some(within) = &within
                && !meta.folder_id.is_some_and(|id| within.contains(&id))
            {
                continue;
            }
            // One whiteboard that cannot be read is listed with what can be said of it,
            // and never keeps the others from being listed.
            let ((elements, pictures), unreadable) = match stored_scene(store, &meta) {
                Ok((scene, _, _)) => (counted(&scene), None),
                Err(Error::Unreadable { reason, .. }) => ((0, 0), Some(reason)),
                Err(error) => return Err(error),
            };
            boards.push(BoardListed {
                board: index.document_summary(store, &meta),
                elements,
                pictures,
                trashed: meta.trashed_at.is_some(),
                unreadable,
            });
        }
        Ok(BoardsList { boards })
    }
}

/// [`Library::read`] of a whiteboard: the listing of its elements (the same in Markdown
/// and as plain text), or as JSON its stored scene as it is, unparsed (with no
/// [`BoardInfo`] when it cannot be read).
pub(crate) fn read(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &crate::types::ReadRequest,
) -> Result<DocumentText> {
    if let Some(heading) = &request.heading {
        return Err(Error::invalid(format!(
            "{} is a whiteboard: it has no sections, so the section {} cannot be read; read \
             it whole",
            quoted_excerpt(title_of(meta)),
            quoted_excerpt(heading)
        )));
    }
    let summary = index.document_summary(store, meta);
    let has = has_picture(store);
    let (content, scene) = match request.format {
        // The stored scene as it is, also one the parser refuses: printing it is the one
        // way left to see, and to save, what a damaged whiteboard holds.
        TextFormat::Json => {
            let raw = store.document(meta.id)?.content;
            let scene = scene_of(meta, &raw).ok().map(|(scene, _)| scene);
            (raw, scene)
        }
        TextFormat::Markdown | TextFormat::PlainText => {
            let (scene, _, _) = stored_scene(store, meta)?;
            (listing(&scene, &summary, &has), Some(scene))
        }
    };
    Ok(DocumentText {
        document: summary,
        format: request.format,
        content,
        section: None,
        comments: request.comments.then(Vec::new),
        images: Vec::new(),
        page: None,
        board: scene.map(|scene| BoardInfo {
            summary: scene.summary(),
            elements: Some(scene.listed_with(&has)),
        }),
    })
}

/// [`Library::info`] of a whiteboard.
pub(crate) fn info(store: &Store, index: &Index, meta: &DocumentMeta) -> Result<DocumentInfo> {
    let (scene, _, _) = stored_scene(store, meta)?;
    let stats = text_document(&scene.text()).stats();
    Ok(DocumentInfo {
        document: index.document_summary(store, meta),
        // A whiteboard's title is the name it was given: it never follows its text.
        custom_title: true,
        paragraphs: stats.paragraphs as u64,
        characters_no_spaces: stats.characters_no_spaces as u64,
        reading_minutes: stats.reading_minutes as u64,
        headings: Vec::new(),
        open_comments: 0,
        resolved_comments: 0,
        versions: store.snapshots(meta.id)?.len(),
        settings: content::settings_info(&innerword_core::DocumentSettings::default()),
        page: None,
        board: Some(BoardInfo {
            summary: scene.summary(),
            elements: None,
        }),
    })
}

/// The text of an element of the text of a whiteboard: a text's own (unwrapped), a
/// frame's name; `None` for an element that has none.
fn text_on(element: &Element) -> Option<String> {
    let text = match element.kind() {
        ElementType::Text => element
            .json()
            .get("originalText")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .or_else(|| element.text())
            .unwrap_or(""),
        ElementType::Frame => element.name().unwrap_or(""),
        _ => return None,
    };
    (!text.trim().is_empty()).then(|| innerword_core::text::normalize_input(text).into_owned())
}

/// What an element of the text of a whiteboard is, for a hit of a find: `reads (arrow
/// label)`, `title (text)`, `backend (frame name)`.
fn what_it_is(element: &Element, scene: &Scene) -> String {
    match element.kind() {
        ElementType::Frame => format!("{} (frame name)", element.id()),
        _ => match element
            .container_id()
            .and_then(|container| scene.element(container))
        {
            Some(container) => format!("{} ({} label)", container.id(), container.type_name()),
            None => format!("{} (text)", element.id()),
        },
    }
}

/// For each line of `text` (the scene's text, [`Scene::text`]), the element it is on.
///
/// The text is the texts of the elements in reading order, one after the other. Which
/// element comes where is asked of `innerword_board` itself, so that its order and this
/// one never part: the texts of a copy of the scene are replaced by the number of their
/// element (where an element stands does not depend on its text: a text keeps its width
/// and height), and the text of the copy is those numbers in reading order. Should the
/// lines not add up, the lines are matched to the texts instead ([`lines_matched`]).
fn line_owners(scene: &Scene, text: &str) -> Vec<Option<String>> {
    let normalized = innerword_core::text::normalize_input(text);
    let count = normalized.split('\n').count();
    let mut owners: Vec<Option<String>> = Vec::with_capacity(count);
    if let Some(order) = reading_order(scene) {
        for n in order {
            let element = &scene.elements()[n];
            let Some(own) = text_on(element) else {
                owners.clear();
                break;
            };
            let what = what_it_is(element, scene);
            owners.extend(std::iter::repeat_n(Some(what), own.split('\n').count()));
        }
        if owners.len() == count {
            return owners;
        }
    }
    lines_matched(scene, &normalized)
}

/// The elements whose texts make the scene's text ([`text_on`]), by their place in
/// [`Scene::elements`], in the order [`Scene::text`] puts them; `None` when the copy that
/// asks cannot be read back.
fn reading_order(scene: &Scene) -> Option<Vec<usize>> {
    let elements = scene.elements();
    let with_text: Vec<usize> = (0..elements.len())
        .filter(|&n| text_on(&elements[n]).is_some())
        .collect();
    if with_text.is_empty() {
        return Some(Vec::new());
    }
    // The elements with a text and the containers of the labels among them: where a
    // label stands is where its container stands.
    let containers: HashSet<&str> = with_text
        .iter()
        .filter_map(|&n| elements[n].container_id())
        .collect();
    let mut copy = Vec::new();
    for (n, element) in elements.iter().enumerate() {
        let mut json = element.json().clone();
        if text_on(element).is_some() {
            let marker = Value::String(n.to_string());
            if element.kind() == ElementType::Frame {
                json.insert("name".into(), marker);
            } else {
                json.insert("originalText".into(), marker.clone());
                json.insert("text".into(), marker);
            }
        } else if !containers.contains(element.id()) {
            continue;
        }
        copy.push(Value::Object(json));
    }
    let stored = serde_json::json!({ "elements": copy }).to_string();
    let parsed = Scene::from_stored(&stored).ok()?;
    let order: Vec<usize> = parsed
        .scene
        .text()
        .split('\n')
        .map(|line| line.parse::<usize>().ok())
        .collect::<Option<_>>()?;
    (order.len() == with_text.len()).then_some(order)
}

/// For each line of the normalized text of a scene, the element it is on, by matching:
/// each line is given to the element whose text the lines from there are, the first one
/// in the order of the drawing that is not given yet (two elements with the same text
/// are the same to whoever reads a hit). Wrong where one text begins another's and the
/// shorter one is drawn first, which is why [`line_owners`] asks for the order first.
fn lines_matched(scene: &Scene, normalized: &str) -> Vec<Option<String>> {
    struct Entry {
        lines: Vec<String>,
        what: String,
        taken: bool,
    }
    let mut entries: Vec<Entry> = scene
        .elements()
        .iter()
        .filter_map(|element| {
            text_on(element).map(|text| Entry {
                lines: text.split('\n').map(str::to_string).collect(),
                what: what_it_is(element, scene),
                taken: false,
            })
        })
        .collect();
    let lines: Vec<&str> = normalized.split('\n').collect();
    let mut owners: Vec<Option<String>> = vec![None; lines.len()];
    let mut at = 0;
    while at < lines.len() {
        let rest = &lines[at..];
        let found = entries.iter_mut().find(|entry| {
            !entry.taken
                && entry.lines.len() <= rest.len()
                && entry.lines.iter().zip(rest).all(|(a, b)| a == b)
        });
        match found {
            Some(entry) => {
                entry.taken = true;
                for owner in &mut owners[at..at + entry.lines.len()] {
                    *owner = Some(entry.what.clone());
                }
                at += entry.lines.len().max(1);
            }
            None => at += 1,
        }
    }
    owners
}

/// [`Library::find`] in a whiteboard: in the text of its elements, each hit naming the
/// element it is on.
pub(crate) fn find(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &FindRequest,
) -> Result<FindResults> {
    let (scene, _, _) = stored_scene(store, meta)?;
    let text = scene.text();
    let document = text_document(&text);
    let matches = matching::find(&document, &request.text, true)?;
    let mut occurrences = matching::occurrences(
        &document,
        &matches.ranges,
        matches.matched_as,
        request.limit.unwrap_or(DEFAULT_FIND_LIMIT),
    );
    let owners = line_owners(&scene, &text);
    for occurrence in &mut occurrences {
        occurrence.element = owners.get(occurrence.block).cloned().flatten();
    }
    Ok(FindResults {
        document: index.document_summary(store, meta),
        count: matches.ranges.len(),
        occurrences,
        matched_as: matches.matched_as,
    })
}

/// [`Library::read_version`] of a version of a whiteboard: the listing of its elements
/// then, or its stored scene.
pub(crate) fn read_version(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    snapshot: &Snapshot,
    format: TextFormat,
) -> Result<VersionText> {
    let summary = index.document_summary(store, meta);
    let content = match format {
        // As it was stored, unparsed: also a version the parser refuses.
        TextFormat::Json => snapshot.content.clone(),
        TextFormat::Markdown | TextFormat::PlainText => {
            let (scene, _) = scene_of(meta, &snapshot.content)?;
            let then = DocumentSummary {
                title: snapshot.meta.title.clone(),
                ..summary.clone()
            };
            listing(&scene, &then, &has_picture(store))
        }
    };
    Ok(VersionText {
        document: summary,
        version: version_info(&snapshot.meta, store.snapshot_label(snapshot.meta.id)?),
        format,
        content,
    })
}

// ----- names, versions -----------------------------------------------------------------------

/// [`rename_document`](crate::items::rename_document) of a whiteboard: its title is the
/// name it was given, and it has no text to take one from.
pub(crate) fn rename(
    session: &mut Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    request: &RenameDocumentRequest,
) -> Result<crate::types::DocumentChanged> {
    if request.follow_text {
        return Err(Error::invalid(format!(
            "{} is a whiteboard, and a whiteboard has no text to take its title from: give \
             it a title",
            quoted_excerpt(title_of(meta))
        )));
    }
    let title = one_line(&request.title);
    if title.is_empty() {
        return Err(Error::invalid("the new title is empty"));
    }
    let changed = title != meta.title;
    let before = index.document_summary(session.store, meta);
    let mut after = DocumentMeta {
        title: title.clone(),
        ..meta.clone()
    };
    if changed && !session.dry_run {
        session.store.rename_document(meta.id, &title)?;
        after = session.store.document_meta(meta.id)?;
        content::set_title_flag(session.store, &after, true)?;
        session.note(Change::DocumentRenamed { id: meta.id });
    }
    Ok(crate::types::DocumentChanged {
        document: index.document_summary(session.store, &after),
        before: Some(before),
        created_folders: Vec::new(),
        written: session.written(changed),
    })
}

/// [`Library::restore_version`] of a version of a whiteboard: that scene is the drawing
/// again, after the drawing as it is was kept as a version. The title stays: a
/// whiteboard's title is the name it was given last.
pub(crate) fn restore_version(
    session: &mut Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    snapshot: &Snapshot,
    request: &RestoreVersionRequest,
) -> Result<VersionRestored> {
    let (_, raw, _) = stored_scene(session.store, meta)?;
    let (restored, _) = scene_of(meta, &snapshot.content)?;
    let stored = Stored::of(&restored);
    let changed = raw != stored.scene;
    let words_before = meta.word_count;
    let restored_version = version_info(
        &snapshot.meta,
        session.store.snapshot_label(snapshot.meta.id)?,
    );
    let mut after = DocumentMeta {
        word_count: stored.stats.words as u64,
        char_count: stored.stats.characters as u64,
        ..meta.clone()
    };
    let mut kept = None;
    if changed && !session.dry_run {
        kept = Some(content::keep_version(
            session.store,
            meta.id,
            &raw,
            &Reason::Restore.label(&author_phrase(&request.write)),
        )?);
        session.note(Change::VersionKept { document: meta.id });
        after = session
            .store
            .save_board(meta.id, &stored.content(), None)
            .map_err(|error| store_refused(&quoted_excerpt(title_of(meta)), error))?
            .meta;
        session.note(Change::DocumentText { id: meta.id });
    } else if !changed {
        after = meta.clone();
    }
    Ok(VersionRestored {
        document: index.document_summary(session.store, &after),
        restored: restored_version,
        snapshot: kept,
        words_before,
        words_after: after.word_count,
        written: session.written(changed),
    })
}

// ----- export --------------------------------------------------------------------------------

/// Fails unless an export of a whiteboard asks for a PDF (named, or by the `.pdf` of the
/// path): a whiteboard is exported as a PDF only.
pub(crate) fn export_format(meta: &DocumentMeta, request: &ExportRequest) -> Result<()> {
    if request.original {
        // The files it was added from are a page's.
        return Err(content::not_a(meta, DocumentKind::Page));
    }
    let format = request.format.or_else(|| {
        request
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(ExportFormat::parse)
    });
    match format {
        Some(ExportFormat::Pdf) => Ok(()),
        Some(
            ExportFormat::Markdown
            | ExportFormat::Docx
            | ExportFormat::PlainText
            | ExportFormat::Html,
        )
        | None => Err(content::not_in_that_format(meta)),
    }
}

/// "1 picture of the whiteboard is missing from the library and is not in the PDF"
fn missing_pictures_sentence(count: usize) -> String {
    if count == 1 {
        "1 picture of the whiteboard is missing from the library and is not in the PDF".to_string()
    } else {
        format!(
            "{count} pictures of the whiteboard are missing from the library and are not in the PDF"
        )
    }
}

/// [`Library::export`] of a whiteboard: its PDF, one page exactly the size of the drawing
/// and a margin, made by the printer of the library (the tool's child, which runs the
/// whiteboard's export page in WebKit). An empty whiteboard has no PDF.
pub(crate) fn export(
    library: &Library,
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &ExportRequest,
    path: PathBuf,
) -> Result<Exported> {
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    if path.is_dir() {
        return Err(Error::invalid(format!(
            "{} is a directory; name the PDF file to write",
            path.display()
        )));
    }
    if path.exists() && !request.overwrite {
        return Err(Error::FileExists { path });
    }
    let (scene, raw, _) = stored_scene(store, meta)?;
    if scene.is_empty() {
        return Err(Error::EmptyBoard {
            id: meta.id.0,
            name: quoted_excerpt(title_of(meta)),
        });
    }
    let Some(checker) = library.page_checker.clone() else {
        return Err(Error::failed(NO_PRINTER));
    };
    let mut exported = Exported {
        path: path.clone(),
        format: ExportFormat::Pdf,
        bytes: 0,
        title: title_of(meta).to_string(),
        documents: vec![index.document_summary(store, meta)],
        assets: Vec::new(),
        files: 0,
        warnings: Vec::new(),
        dry_run: request.dry_run,
    };
    if request.dry_run {
        return Ok(exported);
    }
    // The pictures first: one the library could not read fails the export, one it does
    // not have is left out of the PDF, as Excalidraw's export leaves it out.
    let mut files = Vec::new();
    let mut missing = 0;
    for hex in scene.stored_files() {
        let Some(id) = FileId::parse(hex.as_str()) else {
            missing += 1;
            continue;
        };
        match store.file_bytes(&id) {
            Ok(Some(bytes)) => files.push((hex, bytes)),
            Ok(None) => missing += 1,
            Err(source) => return Err(Error::Pictures { source }),
        }
    }
    let dir = crate::checker::WorkDir::new("board-pdf")?;
    std::fs::create_dir_all(&dir.path).map_err(io(&dir.path))?;
    innerword_board::export_dir::write(&dir.path, &raw, &mut files.into_iter())
        .map_err(io(&dir.path))?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(io(parent))?;
    }
    // Beside the file, so that nobody sees half of it.
    let mut partial = path.clone().into_os_string();
    partial.push(format!(".partial-{}.pdf", std::process::id()));
    let partial = PathBuf::from(partial);
    if let Err(error) = checker.print_board(&dir.path, &partial) {
        std::fs::remove_file(&partial).ok();
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&partial, &path) {
        std::fs::remove_file(&partial).ok();
        return Err(io(&path)(error));
    }
    exported.bytes = std::fs::metadata(&path)
        .map(|found| found.len())
        .unwrap_or(0);
    if missing > 0 {
        exported.warnings.push(missing_pictures_sentence(missing));
    }
    Ok(exported)
}

// ----- pictures, in the process that was asked -----------------------------------------------

/// The pixel size an SVG document declares on its root element: `width` and `height`
/// (in pixels, or without a unit), else the size of its `viewBox`, else the size a browser
/// gives an SVG picture without one (300 x 150).
pub(crate) fn svg_size(bytes: &[u8]) -> (u32, u32) {
    const DEFAULT: (f64, f64) = (300.0, 150.0);
    let text = String::from_utf8_lossy(bytes);
    let attributes = root_svg_attributes(&text);
    let get = |name: &str| {
        attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    let length = |value: Option<&str>| -> Option<f64> {
        let value = value?.trim();
        let number = value.strip_suffix("px").unwrap_or(value).trim();
        number
            .parse::<f64>()
            .ok()
            .filter(|n| n.is_finite() && *n > 0.0)
    };
    let view_box = get("viewBox").and_then(|value| {
        let numbers: Vec<f64> = value
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<f64>().ok())
            .collect();
        match numbers.as_slice() {
            [_, _, w, h] if w.is_finite() && h.is_finite() && *w > 0.0 && *h > 0.0 => {
                Some((*w, *h))
            }
            _ => None,
        }
    });
    let (width, height) = match (length(get("width")), length(get("height")), view_box) {
        (Some(w), Some(h), _) => (w, h),
        (Some(w), None, Some((vw, vh))) => (w, w * vh / vw),
        (None, Some(h), Some((vw, vh))) => (h * vw / vh, h),
        (None, None, Some(size)) => size,
        (Some(w), None, None) => (w, DEFAULT.1),
        (None, Some(h), None) => (DEFAULT.0, h),
        (None, None, None) => DEFAULT,
    };
    let whole = |n: f64| n.round().clamp(1.0, f64::from(u32::MAX)) as u32;
    (whole(width), whole(height))
}

/// The attributes of the first `<svg` start tag of `text` (with or without a prefix), as
/// written.
fn root_svg_attributes(text: &str) -> Vec<(String, String)> {
    let mut from = 0;
    let start = loop {
        let Some(found) = text[from..].find('<') else {
            return Vec::new();
        };
        let at = from + found + 1;
        let name: String = text[at..]
            .chars()
            .take_while(|c| !c.is_whitespace() && *c != '>' && *c != '/')
            .collect();
        let local = name.rsplit(':').next().unwrap_or("");
        if local == "svg" {
            break at + name.len();
        }
        from = at;
    };
    let mut attributes = Vec::new();
    let mut chars = text[start..].char_indices().peekable();
    let rest = &text[start..];
    loop {
        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let Some(&(at, c)) = chars.peek() else { break };
        if c == '>' || c == '/' {
            break;
        }
        let name_start = at;
        while chars
            .peek()
            .is_some_and(|(_, c)| !c.is_whitespace() && *c != '=' && *c != '>' && *c != '/')
        {
            chars.next();
        }
        let name_end = chars.peek().map(|(at, _)| *at).unwrap_or(rest.len());
        let name = rest[name_start..name_end].to_string();
        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            chars.next();
        }
        if chars.peek().is_none_or(|(_, c)| *c != '=') {
            if name.is_empty() {
                chars.next();
            }
            attributes.push((name, String::new()));
            continue;
        }
        chars.next();
        while chars.peek().is_some_and(|(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let value = match chars.peek().copied() {
            Some((at, quote @ ('"' | '\''))) => {
                chars.next();
                let value_start = at + 1;
                let mut value_end = rest.len();
                for (at, c) in chars.by_ref() {
                    if c == quote {
                        value_end = at;
                        break;
                    }
                }
                rest[value_start..value_end].to_string()
            }
            Some((at, _)) => {
                let value_start = at;
                while chars
                    .peek()
                    .is_some_and(|(_, c)| !c.is_whitespace() && *c != '>')
                {
                    chars.next();
                }
                let value_end = chars.peek().map(|(at, _)| *at).unwrap_or(rest.len());
                rest[value_start..value_end].to_string()
            }
            None => String::new(),
        };
        attributes.push((name, value));
    }
    attributes
}

/// The media type of bytes as a whiteboard stores them: an SVG document, else a PNG, a
/// JPEG or a GIF by its first bytes. `None` for anything else.
pub(crate) fn picture_type(bytes: &[u8]) -> Option<&'static str> {
    if innerword_board::sniff_svg(bytes) {
        return Some(SVG);
    }
    ImageFormat::sniff(bytes).map(ImageFormat::mime_type)
}

/// The picture file `source` (an `image` skeleton's `path`) as a whiteboard keeps it,
/// or the sentence that says why it cannot be used. The rule of the whiteboard's
/// pictures (docs/whiteboard-design.md, section 2): bytes that are an SVG document are
/// kept as they came (at most 4 MB), anything else goes through
/// `innerword_image::prepare`.
pub(crate) fn board_picture(
    source: &str,
    base_dir: Option<&Path>,
) -> std::result::Result<BoardPicture, String> {
    let given = source.trim();
    if given.is_empty() {
        return Err("name the picture file".to_string());
    }
    if destination(given) != Destination::Local {
        return Err(format!(
            "{} is not a file on this Mac; give the path of a picture file on this Mac",
            subject(given)
        ));
    }
    let file = match file_of(given, base_dir) {
        Ok(Some(file)) => file,
        Ok(None) => {
            return Err(format!(
                "there is no file {}; give the path of a picture file on this Mac",
                subject(given)
            ));
        }
        Err(_) if base_dir.is_none() && !Path::new(given).is_absolute() => {
            return Err(format!(
                "{} is a relative path, and nothing says what it is relative to: give the \
                 whole path of the picture file",
                subject(given)
            ));
        }
        Err(why) => return Err(format!("{} {why}", subject(given))),
    };
    let sentence = |error: innerword_image::PrepareError| error.sentence(&subject(given));
    let max_svg = innerword_board::limits::MAX_SVG_BYTES;
    let made = |prepared: innerword_image::Prepared| BoardPicture {
        source: source.to_string(),
        file: FileId::of(&prepared.bytes).to_hex(),
        mime_type: prepared.format.mime_type().to_string(),
        pixels: prepared.pixels,
        density: prepared.density,
        untouched: prepared.untouched,
        data: prepared.bytes,
        stored: false,
    };
    // As `innerword_image::prepare_file` does: a path that names no plain file is not
    // opened (a named pipe waits for a writer), and no more is read than is needed.
    let metadata = std::fs::metadata(&file)
        .map_err(|error| sentence(innerword_image::PrepareError::Failed(error.to_string())))?;
    if !metadata.is_file() {
        return Err(sentence(innerword_image::PrepareError::NotAnImage));
    }
    if metadata.len() > max_svg as u64 {
        return innerword_image::prepare_file(&file)
            .map(made)
            .map_err(sentence);
    }
    let opened = std::fs::File::open(&file)
        .map_err(|error| sentence(innerword_image::PrepareError::Failed(error.to_string())))?;
    if !opened.metadata().is_ok_and(|found| found.is_file()) {
        return Err(sentence(innerword_image::PrepareError::NotAnImage));
    }
    let mut bytes = Vec::new();
    opened
        .take(max_svg as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| sentence(innerword_image::PrepareError::Failed(error.to_string())))?;
    if bytes.len() > max_svg {
        // It grew since it was looked at.
        return innerword_image::prepare_file(&file)
            .map(made)
            .map_err(sentence);
    }
    if innerword_board::sniff_svg(&bytes) {
        return Ok(BoardPicture {
            source: source.to_string(),
            file: FileId::of(&bytes).to_hex(),
            mime_type: SVG.to_string(),
            pixels: svg_size(&bytes),
            density: 72,
            untouched: true,
            data: bytes,
            stored: false,
        });
    }
    innerword_image::prepare(&bytes).map(made).map_err(sentence)
}

/// The pictures the `image` skeletons of `elements` name, read and made here: one for
/// each distinct `path`, in the order of the skeletons. A dry run's carry no bytes (it
/// stores nothing, here or in the app).
fn prepare_pictures(
    elements: &[innerword_board::Skeleton],
    base_dir: Option<&Path>,
    dry_run: bool,
) -> Result<Vec<BoardPicture>> {
    let mut pictures = Vec::new();
    let mut bytes = 0usize;
    for path in innerword_board::picture_paths(elements) {
        let index = elements
            .iter()
            .position(|skeleton| {
                skeleton.0.get("type").and_then(Value::as_str) == Some("image")
                    && skeleton.0.get("path").and_then(Value::as_str) == Some(path.as_str())
            })
            .unwrap_or(0);
        let mut picture = board_picture(&path, base_dir).map_err(|message| {
            Error::invalid(format!("the element at index {index} (path): {message}"))
        })?;
        bytes += picture.data.len();
        if bytes > MAX_ATTACHED_BYTES {
            return Err(Error::invalid(TOO_MUCH_PICTURES));
        }
        if dry_run {
            picture.data = Vec::new();
        }
        pictures.push(picture);
    }
    Ok(pictures)
}

impl Prepare for CreateBoardRequest {
    /// Reads and makes the picture files the `image` skeletons name, always: `pictures`
    /// the request was handed are dropped, so that the bytes a whiteboard gets are the
    /// bytes of the files the asked process read.
    fn prepared(&self, library: &Library) -> Result<Cow<'_, Self>> {
        let base_dir = self.base_dir.as_deref().or(library.base_dir.as_deref());
        let pictures = prepare_pictures(&self.elements, base_dir, self.write.dry_run)?;
        if pictures.is_empty() && self.pictures.is_empty() {
            return Ok(Cow::Borrowed(self));
        }
        let mut request = self.clone();
        request.pictures = pictures;
        Ok(Cow::Owned(request))
    }
}

impl Prepare for AddToBoardRequest {
    /// As for [`CreateBoardRequest`]: the pictures are always read again here.
    fn prepared(&self, library: &Library) -> Result<Cow<'_, Self>> {
        let base_dir = self.base_dir.as_deref().or(library.base_dir.as_deref());
        let pictures = prepare_pictures(&self.elements, base_dir, self.write.dry_run)?;
        if pictures.is_empty() && self.pictures.is_empty() {
            return Ok(Cow::Borrowed(self));
        }
        let mut request = self.clone();
        request.pictures = pictures;
        Ok(Cow::Owned(request))
    }
}

// ----- the pictures, where the request is carried out ----------------------------------------

/// Fails unless a picture a request brings holds together: its name is the id of stored
/// bytes, and (when its bytes are there) they are the ones the name says and of the kind
/// the request says.
pub(crate) fn check_picture(picture: &BoardPicture) -> Result<()> {
    if FileId::parse(&picture.file).is_none() {
        return Err(Error::invalid(DAMAGED_PICTURE));
    }
    if picture.stored {
        return Ok(());
    }
    if picture.data.is_empty()
        || FileId::of(&picture.data).to_hex() != picture.file
        || picture_type(&picture.data) != Some(picture.mime_type.as_str())
    {
        return Err(Error::invalid(DAMAGED_PICTURE));
    }
    Ok(())
}

/// The pictures a request brings, for [`innerword_board::add`], by the `path` their
/// skeletons name. In a real run each must have its bytes with it (or have been stored by
/// the app's listener); a dry run's have none.
fn pictures_of(pictures: &[BoardPicture], dry_run: bool) -> Result<Pictures> {
    let mut map = Pictures::new();
    let mut bytes = 0usize;
    for picture in pictures {
        let Some(file) = FileHex::parse(&picture.file) else {
            return Err(Error::invalid(DAMAGED_PICTURE));
        };
        if !matches!(
            picture.mime_type.as_str(),
            "image/png" | "image/jpeg" | "image/gif" | SVG
        ) {
            return Err(Error::invalid(DAMAGED_PICTURE));
        }
        if !dry_run {
            check_picture(picture)?;
        }
        bytes += picture.data.len();
        if bytes > MAX_ATTACHED_BYTES {
            return Err(Error::invalid(TOO_MUCH_PICTURES));
        }
        map.insert(
            picture.source.clone(),
            Picture {
                file,
                mime_type: picture.mime_type.clone(),
                pixels: picture.pixels,
                density: picture.density.max(1),
            },
        );
    }
    Ok(map)
}

/// Stores the pictures that are not stored yet (the app's listener stored them, or this
/// is the tool writing directly): before the scene that names them is saved.
fn store_pictures(store: &mut Store, pictures: &[BoardPicture]) -> Result<()> {
    for picture in pictures {
        if picture.stored {
            continue;
        }
        check_picture(picture)?;
        let info = store.put_file(&picture.data)?;
        if info.id.to_hex() != picture.file {
            return Err(Error::invalid(DAMAGED_PICTURE));
        }
    }
    Ok(())
}

/// What became of the pictures of a request: one for each `image` element that was added.
fn pictures_added(scene: &Scene, added: &Added, pictures: &[BoardPicture]) -> Vec<PictureAdded> {
    let mut out = Vec::new();
    for element in &added.elements {
        if element.r#type != "image" {
            continue;
        }
        let Some(drawn) = scene.element(&element.id) else {
            continue;
        };
        let Some(file) = drawn
            .file_id()
            .and_then(|id| scene.files().get(id))
            .and_then(|entry| entry.file.as_ref())
        else {
            continue;
        };
        let Some(picture) = pictures
            .iter()
            .find(|picture| picture.file == file.as_str())
        else {
            continue;
        };
        out.push(PictureAdded {
            id: element.id.clone(),
            source: picture.source.clone(),
            mime_type: picture.mime_type.clone(),
            pixels: picture.pixels,
            untouched: picture.untouched,
            width: drawn.width(),
            height: drawn.height(),
        });
    }
    out
}

// ----- writing -------------------------------------------------------------------------------

/// A title on one line.
fn one_line(title: &str) -> String {
    title.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `value` as JSON text with the keys of every object in order: the same text whatever
/// order the keys were read in (a build with serde_json's `preserve_order`, the app's,
/// keeps them as they came), so that the values an operation makes do not depend on who
/// carries it out.
fn canonical_json(value: &Value) -> String {
    fn sorted(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                let mut out = serde_json::Map::new();
                for key in keys {
                    out.insert(key.clone(), sorted(&map[key]));
                }
                Value::Object(out)
            }
            Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
            other => other.clone(),
        }
    }
    sorted(value).to_string()
}

/// Where the values an operation makes come from (docs/whiteboard-design.md, section
/// 7.1): SHA-256 of the whiteboard's id, the stamp of its stored scene, what the request
/// asks for as JSON and the ids of its pictures (never their bytes), with the time of the
/// library's clock. The same request on the same whiteboard makes the same elements in the
/// tool, in the app and in a dry run.
fn making(
    board: Option<DocumentId>,
    raw: &str,
    asked: &impl Serialize,
    pictures: &[BoardPicture],
    now_ms: i64,
) -> Result<HashedMaking> {
    let id = board.map(|id| id.0.to_string()).unwrap_or_default();
    let stamp = ContentStamp::of(raw).to_hex();
    let asked = serde_json::to_value(asked)
        .map_err(|error| Error::invalid(format!("the request cannot be read: {error}")))?;
    let asked = canonical_json(&asked);
    let files = pictures
        .iter()
        .map(|picture| picture.file.as_str())
        .collect::<Vec<_>>()
        .join(",");
    Ok(HashedMaking::new(
        &[
            id.as_bytes(),
            stamp.as_bytes(),
            asked.as_bytes(),
            files.as_bytes(),
        ],
        now_ms,
    ))
}

/// The error of a request `innerword_board` refused (nothing was changed): an id the
/// whiteboard does not have is `not_found`, anything else is `invalid`.
fn refused(error: BoardError, name: &str) -> Error {
    match error {
        BoardError::NotFound { id, known } => Error::NotFound {
            kind: ItemKind::Element,
            reference: format!("{} on the whiteboard {name}", quoted_excerpt(&id)),
            hint: Some(if known.is_empty() {
                "The whiteboard has no elements".to_string()
            } else {
                format!("It has {}", known.join(", "))
            }),
        },
        error @ (BoardError::Invalid { .. } | BoardError::TooLarge { .. }) => {
            Error::invalid(error.to_string())
        }
    }
}

/// The error of a store that refused a whiteboard.
fn store_refused(name: &str, error: StoreError) -> Error {
    match error {
        StoreError::FilesMissing { ids } => Error::failed(format!(
            "the pictures of {name} could not be stored in the library ({} {} missing), so \
             nothing was changed; try again",
            ids.len(),
            if ids.len() == 1 { "is" } else { "are" }
        )),
        StoreError::WrongKind { id, kind, saving } => Error::WrongKind {
            id,
            name: name.to_string(),
            kind,
            wanted: Some(saving),
            export: false,
        },
        other => other.into(),
    }
}

/// The whiteboard a request names: live, and a whiteboard.
fn live_board(store: &Store, index: &Index, target: &Target) -> Result<DocumentMeta> {
    let meta = index.live_document(store, target)?;
    match meta.kind {
        DocumentKind::Board => Ok(meta),
        DocumentKind::Text | DocumentKind::Page => Err(content::not_a(&meta, DocumentKind::Board)),
    }
}

/// The label of the version kept before a change: "Before a drawing by Claude".
fn label(reason: Reason, write: &WriteOptions) -> String {
    reason.label(&author_phrase(write))
}

/// What a change of a whiteboard leaves behind: the whiteboard after it, the version kept
/// before it.
struct Saved {
    board: DocumentSummary,
    version: Option<VersionInfo>,
}

/// Saves `scene` as the whiteboard `meta` (whose content was `raw`) after keeping `raw`
/// as a version with the label `label` ([`label`]), the pictures first. A dry run writes
/// nothing and says what the whiteboard would be.
fn save_change(
    session: &mut Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    raw: &str,
    scene: &Scene,
    label: &str,
    pictures: &[BoardPicture],
) -> Result<Saved> {
    let stored = Stored::of(scene);
    if session.dry_run {
        let planned = DocumentMeta {
            word_count: stored.stats.words as u64,
            char_count: stored.stats.characters as u64,
            updated_at: session.clock.now_ms(),
            ..meta.clone()
        };
        return Ok(Saved {
            board: index.document_summary(session.store, &planned),
            version: None,
        });
    }
    // The bytes first: the store names no file it does not have.
    store_pictures(session.store, pictures)?;
    let kept = content::keep_version(session.store, meta.id, raw, label)?;
    session.note(Change::VersionKept { document: meta.id });
    let saved = session
        .store
        .save_board(meta.id, &stored.content(), None)
        .map_err(|error| store_refused(&quoted_excerpt(title_of(meta)), error))?;
    session.note(Change::DocumentText { id: meta.id });
    Ok(Saved {
        board: index.document_summary(session.store, &saved.meta),
        version: Some(kept),
    })
}

/// The result of an operation that changed nothing.
fn unchanged(session: &Session<'_>, index: &Index, meta: &DocumentMeta) -> BoardChanged {
    BoardChanged {
        board: index.document_summary(session.store, meta),
        added: Vec::new(),
        updated: Vec::new(),
        removed: Vec::new(),
        unbound: Vec::new(),
        pictures: Vec::new(),
        version: None,
        warnings: Vec::new(),
        written: session.written(false),
    }
}

/// Makes a whiteboard, empty or with elements. See [`CreateBoardRequest`].
pub(crate) fn create_board(
    session: &mut Session<'_>,
    request: &CreateBoardRequest,
) -> Result<BoardChanged> {
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
    // Stored as it was given, "" included, as the app's New Whiteboard stores it: a
    // whiteboard without a name shows "Untitled" (docs/whiteboard-design.md, section 2).
    let title = one_line(&request.title);
    let shown = if title.is_empty() {
        UNTITLED
    } else {
        title.as_str()
    };
    let name = quoted_excerpt(shown);

    let pictures = pictures_of(&request.pictures, session.dry_run)?;
    let mut scene = Scene::empty();
    let added = if request.elements.is_empty() {
        Added {
            elements: Vec::new(),
            warnings: Vec::new(),
        }
    } else {
        let mut making = making(
            None,
            &scene.to_stored(),
            &request.elements,
            &request.pictures,
            session.clock.now_ms(),
        )?;
        innerword_board::add(&mut scene, &request.elements, &pictures, &mut making)
            .map_err(|error| refused(error, &name))?
    };
    let stored = Stored::of(&scene);
    let result = |board: DocumentSummary, written| BoardChanged {
        board,
        pictures: pictures_added(&scene, &added, &request.pictures),
        added: added.elements.clone(),
        updated: Vec::new(),
        removed: Vec::new(),
        unbound: Vec::new(),
        version: None,
        warnings: added.warnings.clone(),
        written,
    };

    if session.dry_run {
        let now = session.clock.now_ms();
        let mut folder_path = index.folder_segments(store, parent.as_ref().map(|f| f.id));
        let mut above = parent.as_ref().map(|folder| folder.id);
        for name in &missing {
            folder_path.push(name.clone());
            above = Some(FolderId(0));
        }
        if missing.is_empty()
            && let Some(placement) = &request.placement
        {
            // Checks that a "before" or "after" names an item of the folder.
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
        path.push(shown.to_string());
        let board = DocumentSummary {
            id: DocumentId(0),
            title: title.clone(),
            path: join_path(&path),
            folder_id: above,
            folder: join_path(&folder_path),
            words: stored.stats.words as u64,
            characters: stored.stats.characters as u64,
            created_at: Time(now),
            updated_at: Time(now),
            trashed_at: None,
            kind: DocumentKind::Board,
            source_url: None,
        };
        return Ok(result(board, session.written(true)));
    }

    // The pictures first: the store names no file it does not have.
    store_pictures(session.store, &request.pictures)?;
    let mut folder_id = parent.as_ref().map(|folder| folder.id);
    for name in &missing {
        let folder = session.store.create_folder(folder_id, name)?;
        session.note(Change::FolderCreated { id: folder.id });
        folder_id = Some(folder.id);
    }
    let place = match &request.placement {
        None => None,
        Some(placement) => {
            let siblings: Vec<i64> = session
                .store
                .documents_in_folder(folder_id)?
                .iter()
                .map(|meta| meta.id.0)
                .collect();
            let store = &session.store;
            Some(place_among(&siblings, placement, |target| {
                Ok(index.live_document(store, target)?.id.0)
            })?)
        }
    };
    let saved = session
        .store
        .create_board(folder_id, &title, &stored.content())
        .map_err(|error| store_refused(&name, error))?;
    let id = saved.meta.id;
    session.note(Change::DocumentCreated { id });
    if let Some(place) = place {
        session.store.reorder_document(id, place)?;
    }
    let meta = session.store.document_meta(id)?;
    Ok(result(
        index.document_summary(session.store, &meta),
        session.written(true),
    ))
}

/// Adds elements to a whiteboard. See [`AddToBoardRequest`].
pub(crate) fn add_to_board(
    session: &mut Session<'_>,
    request: &AddToBoardRequest,
) -> Result<BoardChanged> {
    let index = Index::load(session.store)?;
    let meta = live_board(session.store, &index, &request.board)?;
    if request.elements.is_empty() {
        return Err(Error::invalid(
            "there is nothing to add: give at least one element",
        ));
    }
    let name = quoted_excerpt(title_of(&meta));
    let (mut scene, raw, mut warnings) = stored_scene(session.store, &meta)?;
    let pictures = pictures_of(&request.pictures, session.dry_run)?;
    let mut making = making(
        Some(meta.id),
        &raw,
        &request.elements,
        &request.pictures,
        session.clock.now_ms(),
    )?;
    let added = innerword_board::add(&mut scene, &request.elements, &pictures, &mut making)
        .map_err(|error| refused(error, &name))?;
    warnings.extend(added.warnings.iter().cloned());
    let saved = save_change(
        session,
        &index,
        &meta,
        &raw,
        &scene,
        &label(Reason::Drawing, &request.write),
        &request.pictures,
    )?;
    Ok(BoardChanged {
        board: saved.board,
        pictures: pictures_added(&scene, &added, &request.pictures),
        added: added.elements,
        updated: Vec::new(),
        removed: Vec::new(),
        unbound: Vec::new(),
        version: saved.version,
        warnings,
        written: session.written(true),
    })
}

/// Changes elements of a whiteboard by id. See [`UpdateBoardRequest`].
pub(crate) fn update_board(
    session: &mut Session<'_>,
    request: &UpdateBoardRequest,
) -> Result<BoardChanged> {
    let index = Index::load(session.store)?;
    let meta = live_board(session.store, &index, &request.board)?;
    if request.changes.is_empty() {
        return Err(Error::invalid(
            "there is nothing to change: give at least one change, with the id of its element",
        ));
    }
    let name = quoted_excerpt(title_of(&meta));
    let (mut scene, raw, mut warnings) = stored_scene(session.store, &meta)?;
    let before = scene.clone();
    let mut making = making(
        Some(meta.id),
        &raw,
        &request.changes,
        &[],
        session.clock.now_ms(),
    )?;
    let mut updated = innerword_board::update(&mut scene, &request.changes, &mut making)
        .map_err(|error| refused(error, &name))?;
    // A change that sets what an element already has changes nothing (spec 7.7): it is
    // left out, so that its element, and the arrows joined to it, keep their versions; a
    // request of nothing but such changes keeps no version of the whiteboard and says it
    // would change nothing. What is left is made again from the scene as it was, so a
    // dry run and the real run make the same.
    let kept = changes_that_change(&request.changes, &before, &scene, |scene, changes| {
        let mut making = self::making(Some(meta.id), &raw, &changes, &[], session.clock.now_ms())?;
        innerword_board::update(scene, changes, &mut making)
            .map_err(|error| refused(error, &name))?;
        Ok(())
    })?;
    let effective: Vec<ElementChange> = request
        .changes
        .iter()
        .zip(&kept)
        .filter(|(_, kept)| **kept)
        .map(|(change, _)| change.clone())
        .collect();
    if effective.is_empty() {
        updated.changed.clear();
    } else if effective.len() < request.changes.len() {
        scene = before;
        let mut making =
            self::making(Some(meta.id), &raw, &effective, &[], session.clock.now_ms())?;
        updated = innerword_board::update(&mut scene, &effective, &mut making)
            .map_err(|error| refused(error, &name))?;
    }
    warnings.extend(updated.warnings.iter().cloned());
    if updated.changed.is_empty() {
        return Ok(BoardChanged {
            warnings,
            ..unchanged(session, &index, &meta)
        });
    }
    let saved = save_change(
        session,
        &index,
        &meta,
        &raw,
        &scene,
        &label(Reason::Edit, &request.write),
        &[],
    )?;
    Ok(BoardChanged {
        board: saved.board,
        added: Vec::new(),
        updated: updated.changed,
        removed: Vec::new(),
        unbound: Vec::new(),
        pictures: Vec::new(),
        version: saved.version,
        warnings,
        written: session.written(true),
    })
}

/// The fields of an element that every change of it sets anew: they say that it changed,
/// not what it is.
const STAMPS: [&str; 3] = ["version", "versionNonce", "updated"];

/// Whether two values are the same as JSON reads them (a number by its value: `100` and
/// `100.0` are one).
fn same_value(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a == b || a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_value(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(key, a)| b.get(key).is_some_and(|b| same_value(a, b)))
        }
        _ => a == b,
    }
}

/// Whether an element is what it was, its stamps ([`STAMPS`]) aside.
fn same_element(before: &Element, after: &Element) -> bool {
    let (before, after) = (before.json(), after.json());
    let fields = |map: &'_ serde_json::Map<String, Value>| {
        map.keys()
            .filter(|key| !STAMPS.contains(&key.as_str()))
            .count()
    };
    fields(before) == fields(after)
        && before
            .iter()
            .filter(|(key, _)| !STAMPS.contains(&key.as_str()))
            .all(|(key, value)| after.get(key).is_some_and(|other| same_value(value, other)))
}

/// Which changes of an update change something, each judged against the scene as it
/// stood just before it, as the update applies them one after the other (`apply` runs
/// some of them on a scene). A change is no change when it leaves its element and that
/// element's label as they were then: it set what the element already had.
///
/// Judging a change by the scene after the whole update would drop one that undoes what
/// another change of the request did (a frame moved and its child set back where it
/// was), and the drawing saved would not be the one asked for. So the whole update
/// (`before` to `after`) only says which changes may be no change (their elements end as
/// they were); those are judged where they stand. A run of them that leaves every element
/// it names as it was is no change at all (one request that sets every element as it is
/// costs one more update, not one per change); otherwise each is applied alone.
fn changes_that_change(
    changes: &[ElementChange],
    before: &Scene,
    after: &Scene,
    mut apply: impl FnMut(&mut Scene, &[ElementChange]) -> Result<()>,
) -> Result<Vec<bool>> {
    let maybe: Vec<bool> = changes
        .iter()
        .map(|change| left_as_it_was(change, before, after))
        .collect();
    let mut kept = vec![true; changes.len()];
    if !maybe.contains(&true) {
        return Ok(kept);
    }
    let mut work = before.clone();
    let mut start = 0;
    while start < changes.len() {
        let mut end = start + 1;
        while end < changes.len() && maybe[end] == maybe[start] {
            end += 1;
        }
        let run = &changes[start..end];
        if !maybe[start] {
            apply(&mut work, run)?;
        } else {
            let was = work.clone();
            apply(&mut work, run)?;
            if run.iter().all(|change| left_as_it_was(change, &was, &work)) {
                kept[start..end].fill(false);
                work = was;
            } else {
                work = was;
                for (n, change) in run.iter().enumerate() {
                    let was = work.clone();
                    apply(&mut work, std::slice::from_ref(change))?;
                    if left_as_it_was(change, &was, &work) {
                        kept[start + n] = false;
                        work = was;
                    }
                }
            }
        }
        start = end;
    }
    Ok(kept)
}

/// Whether a change of an update left its element as it was: the element it names and
/// that element's label are what they were in `before` and in `after`. Between the scene
/// just before the change and just after it, such a change set what the element already
/// has, and is no change at all.
fn left_as_it_was(change: &ElementChange, before: &Scene, after: &Scene) -> bool {
    let Some(id) = change.0.get("id").and_then(Value::as_str) else {
        return false;
    };
    let label = |scene: &Scene| {
        scene.element(id).and_then(|element| {
            element
                .bound_elements()
                .into_iter()
                .find(|(_, kind)| *kind == "text")
                .map(|(label, _)| label.to_string())
        })
    };
    let same = |id: &str| match (before.element(id), after.element(id)) {
        (Some(was), Some(is)) => same_element(was, is),
        (None, None) => true,
        _ => false,
    };
    let (label_before, label_after) = (label(before), label(after));
    label_before == label_after && same(id) && label_before.as_deref().is_none_or(same)
}

/// Removes elements of a whiteboard by id. See [`RemoveFromBoardRequest`].
pub(crate) fn remove_from_board(
    session: &mut Session<'_>,
    request: &RemoveFromBoardRequest,
) -> Result<BoardChanged> {
    let index = Index::load(session.store)?;
    let meta = live_board(session.store, &index, &request.board)?;
    if request.ids.is_empty() {
        return Err(Error::invalid(
            "there is nothing to remove: give the id of at least one element",
        ));
    }
    let name = quoted_excerpt(title_of(&meta));
    let (mut scene, raw, warnings) = stored_scene(session.store, &meta)?;
    let mut making = making(
        Some(meta.id),
        &raw,
        &request.ids,
        &[],
        session.clock.now_ms(),
    )?;
    let removed = innerword_board::remove(&mut scene, &request.ids, &mut making)
        .map_err(|error| refused(error, &name))?;
    if removed.removed.is_empty() {
        return Ok(BoardChanged {
            warnings,
            ..unchanged(session, &index, &meta)
        });
    }
    let saved = save_change(
        session,
        &index,
        &meta,
        &raw,
        &scene,
        &label(Reason::Removal, &request.write),
        &[],
    )?;
    Ok(BoardChanged {
        board: saved.board,
        added: Vec::new(),
        updated: Vec::new(),
        removed: removed.removed,
        unbound: removed.unbound,
        pictures: Vec::new(),
        version: saved.version,
        warnings,
        written: session.written(true),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_size_an_svg_declares_is_read_from_its_root() {
        let size = |text: &str| svg_size(text.as_bytes());
        assert_eq!(
            size(r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"/>"#),
            (120, 80)
        );
        assert_eq!(
            size(
                r#"<?xml version="1.0"?><svg width='64px' height='32px' xmlns="http://www.w3.org/2000/svg"></svg>"#
            ),
            (64, 32)
        );
        assert_eq!(
            size(r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200, 100"></svg>"#),
            (200, 100)
        );
        assert_eq!(
            size(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="50" viewBox="0 0 200 100"></svg>"#
            ),
            (50, 25)
        );
        assert_eq!(
            size(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100%" height="2em"></svg>"#),
            (300, 150)
        );
        assert_eq!(
            size(
                r#"<!-- a <comment> --><s:svg xmlns:s="http://www.w3.org/2000/svg" width="7" height="9"/>"#
            ),
            (7, 9)
        );
    }

    #[test]
    fn the_json_of_a_request_does_not_depend_on_the_order_of_its_keys() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[{"z":1,"y":2}],"c":2}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"c":2,"d":[{"y":2,"z":1}]},"b":1}"#).unwrap();
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(
            canonical_json(&a),
            r#"{"a":{"c":2,"d":[{"y":2,"z":1}]},"b":1}"#
        );
    }

    #[test]
    fn one_file_named_by_two_skeletons_is_read_once_and_travels_once() {
        let dir = tempfile::tempdir().unwrap();
        let png = innerword_image::fixtures::png(6, 4, [5, 6, 7]);
        std::fs::write(dir.path().join("one.png"), &png).unwrap();
        let elements: Vec<innerword_board::Skeleton> = serde_json::from_str(
            r#"[{"type":"image","x":0,"y":0,"path":"one.png"},
                {"type":"rectangle","x":0,"y":0},
                {"type":"image","x":90,"y":0,"path":"one.png"}]"#,
        )
        .unwrap();
        let pictures = prepare_pictures(&elements, Some(dir.path()), false).unwrap();
        assert_eq!(pictures.len(), 1, "{pictures:?}");
        assert_eq!(pictures[0].source, "one.png");
        assert_eq!(pictures[0].file, FileId::of(&png).to_hex());
        assert_eq!(pictures[0].data, png);
        // A dry run reads it to say what it is, and carries no bytes.
        let dry = prepare_pictures(&elements, Some(dir.path()), true).unwrap();
        assert_eq!(dry.len(), 1);
        assert!(dry[0].data.is_empty());
    }

    #[test]
    fn the_kind_of_a_picture_is_read_from_its_bytes() {
        let png = innerword_image::fixtures::png(4, 3, [1, 2, 3]);
        assert_eq!(picture_type(&png), Some("image/png"));
        let jpeg = innerword_image::fixtures::jpeg(4, 3, [1, 2, 3]);
        assert_eq!(picture_type(&jpeg), Some("image/jpeg"));
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="3"/>"#;
        assert_eq!(picture_type(svg), Some(SVG));
        assert_eq!(picture_type(b"<html><body>no</body></html>"), None);
    }
}
