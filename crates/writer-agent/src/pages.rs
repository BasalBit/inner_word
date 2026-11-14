//! Pages: web pages kept whole in the library (docs/pages-design.md, sections 1 to 4, 8
//! and 9).
//!
//! A page is a row of `documents` of the kind `page`, whose content is its manifest
//! (`writer_page::Manifest`) and whose files are blobs in `images.db`. Everything that is
//! made of a page happens in the process that was asked, before the operation goes
//! anywhere ([`Prepare`] for [`AddPageRequest`]): the file or folder is read, what it
//! names on the web is fetched, the page is checked whole by a [`PageChecker`]
//! (`crate::checker`), and the request carries the result ([`PreparedPage`]). The app,
//! when it carries the request out, never reads an agent's path and never fetches:
//! [`add_page`] stores what the request brings and writes the row.
//!
//! [`PageChecker`]: crate::PageChecker

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use writer_core::{Document, Fragment, Stats};
use writer_page::{Blob, Manifest, Prepared, Problem};
use writer_store::{
    DocumentId, DocumentKind, DocumentMeta, FileId, Folder, FolderId, PageContent, Snapshot, Store,
    StoreError,
};

use crate::attach::Prepare;
use crate::content::{self, Reason, version_info};
use crate::error::{Error, Result, quoted_excerpt};
use crate::items::plan_folders;
use crate::library::{Library, Session, author_phrase};
use crate::live::Change;
use crate::matching;
use crate::resolve::{Index, join_path, title_of};
use crate::types::{
    AddPageRequest, CommentList, DocumentInfo, DocumentSummary, DocumentText, ExportFormat,
    ExportRequest, Exported, FindRequest, FindResults, PageAdded, PageListed, PageOutcome,
    PageSummary, PagesList, PagesRequest, PreparedPage, ReadRequest, RestoreVersionRequest, Target,
    TextFormat, Time, VersionRestored, VersionText,
};

/// What add_page and a PDF export say without a checker.
pub(crate) const NO_CHECKER: &str = "pages cannot be checked here";

/// What is said of a blob whose bytes are not the ones its name says.
pub(crate) const DAMAGED_BLOB: &str = "the page as it was sent is damaged: the bytes of one of \
     its files are not the ones its name says, so nothing was saved; add it again";

/// How many occurrences a find in a page returns unless the request says.
const DEFAULT_FIND_LIMIT: usize = 20;

/// "the page weighs 71 MB; a page can be 64 MB at most"
pub(crate) fn too_heavy_sentence(bytes: u64) -> String {
    let size = writer_page::megabytes(bytes);
    let most = writer_page::megabytes(writer_page::MAX_PAGE_BYTES);
    let size = if size == most {
        format!("over {size}")
    } else {
        size
    };
    format!("the page weighs {size}; a page can be {most} at most")
}

/// Fails unless the bytes of `blob` are there and are the ones its name says.
pub(crate) fn check_blob(blob: &Blob) -> Result<()> {
    if !writer_page::is_file_id(&blob.file)
        || blob.data.len() as u64 != blob.size
        || writer_page::file_id_hex(&blob.data) != blob.file
    {
        return Err(Error::invalid(DAMAGED_BLOB));
    }
    Ok(())
}

/// The counts of a page: those of its text, read as a text is.
fn text_stats(text: &str) -> Stats {
    text_document(text).stats()
}

/// The text of a page as a document of paragraphs, for counting and finding.
fn text_document(text: &str) -> Document {
    Fragment::from_plain_text(text).to_document()
}

/// The manifest of a stored page (or of one of its versions).
fn manifest_of(meta: &DocumentMeta, content: &str) -> Result<Manifest> {
    Manifest::from_json(content).map_err(|error| Error::Unreadable {
        id: meta.id.0,
        name: quoted_excerpt(title_of(meta)),
        reason: match error {
            writer_page::PageError::Unreadable { reason } => reason,
            other => other.sentence(),
        },
    })
}

/// The manifest of the page `meta` as it is stored now.
fn stored_manifest(store: &Store, meta: &DocumentMeta) -> Result<(Manifest, String)> {
    let raw = store.document(meta.id)?.content;
    Ok((manifest_of(meta, &raw)?, raw))
}

/// The manifest as `read` gives it in JSON: everything but the text.
fn without_text(manifest: &Manifest) -> String {
    let mut manifest = manifest.clone();
    manifest.text.clear();
    manifest.to_json()
}

/// What a page is made of, as results show it.
pub(crate) fn summary(meta: &DocumentMeta, manifest: &Manifest) -> PageSummary {
    PageSummary {
        source_url: meta
            .source_url
            .clone()
            .or_else(|| manifest.source_url.clone()),
        entry: manifest.entry.clone(),
        files: manifest.files.len(),
        bytes: manifest.bytes(),
        externals: manifest.externals.len(),
        content_at: Time(manifest.added_at),
        engine: manifest.checked.engine.clone(),
        warnings: manifest.checked.warnings.clone(),
    }
}

/// The source address of a request in its normal form.
fn normal_source(source_url: &Option<String>) -> Result<Option<String>> {
    match source_url.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(url) => Ok(Some(writer_page::normal_source_url(url)?)),
    }
}

/// The title a request gives, on one line.
fn given_title(request: &AddPageRequest) -> Option<String> {
    let title = request
        .title
        .as_deref()?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!title.is_empty()).then(|| title.chars().take(writer_page::MAX_TITLE_CHARS).collect())
}

// ----- preparing, in the process that was asked ---------------------------------------------

/// The local path a request names: absolute, `~/...`, or relative to `base_dir`.
fn local_path(given: &str, base_dir: Option<&Path>, what: &str) -> Result<PathBuf> {
    let given = given.trim();
    if given.is_empty() {
        return Err(Error::invalid(format!("name the {what}")));
    }
    let path = if given == "~" || given.starts_with("~/") {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return Err(Error::invalid(format!(
                "{} starts at the home folder, and this process does not know where that is",
                quoted_excerpt(given)
            )));
        };
        PathBuf::from(home).join(given[1..].trim_start_matches('/'))
    } else {
        PathBuf::from(given)
    };
    if path.is_absolute() {
        return Ok(path);
    }
    match base_dir {
        Some(base) => Ok(base.join(path)),
        None => Err(Error::invalid(format!(
            "{} is a relative path, and nothing says what it is relative to: give the absolute \
             path of the {what}",
            quoted_excerpt(given)
        ))),
    }
}

/// Whether what a request names as its input is an address on the web.
fn is_address(given: &str) -> bool {
    let lower = given.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("www.")
}

impl Library {
    /// What the library knows of the page at `url`: the same page (unchanged), or one in
    /// the trash that is not to be added again. `None`: the page is to be prepared whole.
    fn known_page(
        &self,
        url: &str,
        fingerprint: &str,
        include_trashed: bool,
    ) -> Result<Option<PreparedPage>> {
        if !self.database_path().is_file() {
            return Ok(None);
        }
        let reader = self.reader()?;
        let store = &reader.store;
        if let Some(meta) = store.page_by_source_url(url)? {
            let raw = store.document(meta.id)?.content;
            let same =
                Manifest::from_json(&raw).is_ok_and(|stored| stored.fingerprint == fingerprint);
            return Ok(same.then(|| PreparedPage::Unchanged {
                page: meta.id,
                fingerprint: fingerprint.to_string(),
            }));
        }
        if !include_trashed && let Some(meta) = store.trashed_page_by_source_url(url)? {
            return Ok(Some(PreparedPage::Trashed { page: meta.id }));
        }
        Ok(None)
    }
}

impl Prepare for AddPageRequest {
    /// Reads, fetches and checks the page, always: a `prepared` the request was handed
    /// is dropped, so that nobody can skip the fetching, the check or the verdict by
    /// saying that the page was prepared already.
    fn prepared(&self, library: &Library) -> Result<Cow<'_, Self>> {
        let mut request = self.clone();
        request.prepared = None;
        let Some(checker) = library.page_checker.clone() else {
            return Err(Error::failed(NO_CHECKER));
        };
        let source_url = normal_source(&self.source_url)?;
        let base_dir = self.base_dir.as_deref().or(library.base_dir.as_deref());
        let input = if is_address(&self.path) {
            // Said in the bundler's words: a live site is not fetched.
            PathBuf::from(self.path.trim())
        } else {
            local_path(&self.path, base_dir, "file or folder of the page")?
        };
        let entry = self
            .entry
            .as_deref()
            .map(str::trim)
            .filter(|entry| !entry.is_empty());
        let input = writer_page::read_input(&input, entry)?;
        // The same page again: nothing is fetched or checked.
        if let Some(url) = &source_url
            && let Some(known) =
                library.known_page(url, &input.fingerprint, self.include_trashed)?
        {
            request.prepared = Some(known);
            return Ok(Cow::Owned(request));
        }
        let fetcher: Arc<dyn writer_page::Fetcher> = if self.externals.is_empty() {
            library.fetcher.clone()
        } else {
            let mut known: BTreeMap<String, PathBuf> = BTreeMap::new();
            for (address, file) in &self.externals {
                known.insert(
                    address.clone(),
                    local_path(file, base_dir, "file that stands for a piece of the page")?,
                );
            }
            Arc::new(writer_page::KnownPieces::read(
                &known,
                library.fetcher.clone(),
            )?)
        };
        let bundle = writer_page::bundle(input, fetcher.as_ref())?;
        let report = checker.check(&bundle)?;
        let mut page = writer_page::prepare(bundle, &report)?;
        if page.distinct_bytes() > crate::live::MAX_PAGE_BYTES as u64 {
            return Err(Error::PageIncomplete {
                name: page.title(),
                problems: vec![Problem::new(
                    "the page",
                    too_heavy_sentence(page.distinct_bytes()),
                )],
            });
        }
        if self.write.dry_run {
            // A dry run stores nothing, here or in the app: its request carries no bytes.
            for blob in &mut page.blobs {
                blob.data = Vec::new();
            }
        }
        request.prepared = Some(PreparedPage::Page(page));
        Ok(Cow::Owned(request))
    }
}

// ----- adding, where the request is carried out ---------------------------------------------

/// What is said when the library changed between the look of the tool and the change.
fn changed_meanwhile(name: &str) -> Error {
    Error::failed(format!(
        "{name} changed in the library while it was being added, so nothing was changed; add \
         it again"
    ))
}

/// The limits of a page, again where the request is carried out, and whether what the
/// request says of its files holds together.
fn check_prepared(page: &Prepared, dry_run: bool) -> Result<()> {
    let name = page.title();
    let mut problems = Vec::new();
    let count = |n: usize| {
        let digits = n.to_string();
        let mut out = String::new();
        for (i, digit) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i).is_multiple_of(3) {
                out.push(',');
            }
            out.push(digit);
        }
        out
    };
    if page.served.len() > writer_page::MAX_FILES || page.original.len() > writer_page::MAX_FILES {
        problems.push(Problem::new(
            "the page",
            format!(
                "the page has {} files; a page can have {} at most",
                count(page.served.len().max(page.original.len())),
                count(writer_page::MAX_FILES)
            ),
        ));
    }
    if page.externals.len() > writer_page::MAX_EXTERNALS {
        problems.push(Problem::new(
            "the page",
            format!(
                "the page names {} pieces on the web; a page can have {} at most",
                count(page.externals.len()),
                count(writer_page::MAX_EXTERNALS)
            ),
        ));
    }
    for (blob, path) in page.blobs.iter().zip(blob_paths(page)) {
        if blob.size > writer_page::MAX_FILE_BYTES {
            problems.push(Problem::new(
                path.clone(),
                format!(
                    "{path} is {}; a file can be {} at most",
                    writer_page::megabytes(blob.size),
                    writer_page::megabytes(writer_page::MAX_FILE_BYTES)
                ),
            ));
        }
    }
    if page.distinct_bytes() > writer_page::MAX_PAGE_BYTES {
        problems.push(Problem::new(
            "the page",
            too_heavy_sentence(page.distinct_bytes()),
        ));
    }
    if !problems.is_empty() {
        return Err(Error::PageIncomplete { name, problems });
    }
    let unreadable =
        |why: &str| Error::invalid(format!("the page as it was sent cannot be stored: {why}"));
    if page
        .served
        .iter()
        .chain(&page.original)
        .any(|entry| entry.blob >= page.blobs.len())
    {
        return Err(unreadable(
            "a path names a file that is not among its files",
        ));
    }
    if !page.served.iter().any(|entry| entry.path == page.entry) {
        return Err(unreadable("the file it opens with is not among its files"));
    }
    for blob in &page.blobs {
        if !writer_page::is_file_id(&blob.file) {
            return Err(Error::invalid(DAMAGED_BLOB));
        }
        if !dry_run && !blob.stored {
            check_blob(blob)?;
        }
    }
    Ok(())
}

/// For each blob, a path it is served (or was given) at, for a sentence.
fn blob_paths(page: &Prepared) -> Vec<String> {
    let mut paths = vec![String::new(); page.blobs.len()];
    for entry in page.original.iter().chain(&page.served) {
        if let Some(path) = paths.get_mut(entry.blob) {
            *path = entry.path.clone();
        }
    }
    paths
}

/// Stores the blobs that are not stored yet (the app's listener stored them, or this is
/// the tool writing directly).
fn store_blobs(store: &mut Store, page: &Prepared) -> Result<()> {
    for blob in &page.blobs {
        if blob.stored {
            continue;
        }
        check_blob(blob)?;
        let info = store.put_file(&blob.data)?;
        if info.id.to_hex() != blob.file {
            return Err(Error::invalid(DAMAGED_BLOB));
        }
    }
    Ok(())
}

/// The error of a store that refused a page.
fn store_refused(name: &str, error: StoreError) -> Error {
    match error {
        StoreError::FilesMissing { ids } => Error::failed(format!(
            "the files of {name} could not be stored in the library ({} {} missing), so it was \
             not added; add it again",
            ids.len(),
            if ids.len() == 1 { "is" } else { "are" }
        )),
        StoreError::WrongKind { id, kind } => Error::WrongKind {
            id,
            name: name.to_string(),
            is_page: kind == DocumentKind::Page,
        },
        other => other.into(),
    }
}

/// The result of an operation that found the page as it is.
fn as_it_is(
    session: &Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    manifest: &Manifest,
    outcome: PageOutcome,
) -> PageAdded {
    PageAdded {
        page: index.document_summary(session.store, meta),
        outcome,
        snapshot: None,
        files: manifest.files.len(),
        bytes: manifest.bytes(),
        externals: manifest.externals.len(),
        warnings: Vec::new(),
        written: session.written(false),
    }
}

/// Adds a page that was prepared by the process that was asked: as new, as an update of
/// the page at its address (the page as it was kept as a version first), or not at all
/// (the same page is there, or the page with the address is in the trash).
pub(crate) fn add_page(session: &mut Session<'_>, request: &AddPageRequest) -> Result<PageAdded> {
    let Some(prepared) = &request.prepared else {
        return Err(Error::invalid("the page was not prepared"));
    };
    let source_url = normal_source(&request.source_url)?;
    let index = Index::load(session.store)?;
    match prepared {
        PreparedPage::Unchanged { page, fingerprint } => {
            let meta = session
                .store
                .document_meta(*page)
                .map_err(|_| changed_meanwhile("the page"))?;
            let name = quoted_excerpt(title_of(&meta));
            if meta.kind != DocumentKind::Page
                || meta.trashed_at.is_some()
                || source_url.is_none()
                || meta.source_url != source_url
            {
                return Err(changed_meanwhile(&name));
            }
            let (manifest, _) = stored_manifest(session.store, &meta)?;
            if manifest.fingerprint != *fingerprint {
                return Err(changed_meanwhile(&name));
            }
            Ok(as_it_is(
                session,
                &index,
                &meta,
                &manifest,
                PageOutcome::Unchanged,
            ))
        }
        PreparedPage::Trashed { page } => {
            let meta = session
                .store
                .document_meta(*page)
                .map_err(|_| changed_meanwhile("the page"))?;
            let name = quoted_excerpt(title_of(&meta));
            let Some(url) = &source_url else {
                return Err(changed_meanwhile(&name));
            };
            if meta.kind != DocumentKind::Page
                || meta.trashed_at.is_none()
                || meta.source_url.as_ref() != Some(url)
                || session.store.page_by_source_url(url)?.is_some()
            {
                return Err(changed_meanwhile(&name));
            }
            let (manifest, _) = stored_manifest(session.store, &meta)?;
            Ok(as_it_is(
                session,
                &index,
                &meta,
                &manifest,
                PageOutcome::Trashed,
            ))
        }
        PreparedPage::Page(page) => add_prepared(session, &index, request, page, source_url),
    }
}

fn add_prepared(
    session: &mut Session<'_>,
    index: &Index,
    request: &AddPageRequest,
    page: &Prepared,
    source_url: Option<String>,
) -> Result<PageAdded> {
    check_prepared(page, session.dry_run)?;
    let now = session.clock.now_ms();
    let manifest = page.manifest(source_url.as_deref(), now);
    let json = manifest.to_json();
    // What the library will be asked to read later, read now.
    Manifest::from_json(&json).map_err(|error| {
        Error::invalid(format!(
            "the page as it was sent cannot be stored: {}",
            error.sentence()
        ))
    })?;
    let stats = text_stats(&page.text);
    let (words, characters) = (stats.words as u64, stats.characters as u64);
    let result = |document: DocumentSummary, outcome, snapshot, written| PageAdded {
        page: document,
        outcome,
        snapshot,
        files: page.served.len(),
        bytes: page.distinct_bytes(),
        externals: page.externals.len(),
        warnings: page.warnings.clone(),
        written,
    };

    // The page at this address: the same, or one to update.
    let existing = match &source_url {
        Some(url) => session.store.page_by_source_url(url)?,
        None => None,
    };
    if let Some(meta) = existing {
        let (old, raw) = stored_manifest(session.store, &meta)?;
        if old.fingerprint == page.fingerprint {
            return Ok(as_it_is(
                session,
                index,
                &meta,
                &old,
                PageOutcome::Unchanged,
            ));
        }
        let title = if content::title_flag_set(session.store, &meta) {
            meta.title.clone()
        } else {
            given_title(request).unwrap_or_else(|| page.title())
        };
        let name = quoted_excerpt(&title);
        if session.dry_run {
            let planned = DocumentMeta {
                title: title.clone(),
                word_count: words,
                char_count: characters,
                updated_at: now,
                ..meta.clone()
            };
            return Ok(result(
                index.document_summary(session.store, &planned),
                PageOutcome::Updated,
                None,
                session.written(true),
            ));
        }
        store_blobs(session.store, page)?;
        let kept = content::keep_version(
            session.store,
            meta.id,
            &raw,
            &Reason::Update.label(&author_phrase(&request.write)),
        )?;
        session.note(Change::VersionKept { document: meta.id });
        let after = session
            .store
            .save_page(
                meta.id,
                &PageContent {
                    manifest: &json,
                    title: &title,
                    plain_text: &page.text,
                    word_count: words,
                    char_count: characters,
                    source_url: source_url.as_deref(),
                },
            )
            .map_err(|error| store_refused(&name, error))?;
        session.note(Change::DocumentText { id: meta.id });
        if after.title != meta.title {
            session.note(Change::DocumentRenamed { id: meta.id });
        }
        return Ok(result(
            index.document_summary(session.store, &after),
            PageOutcome::Updated,
            Some(kept),
            session.written(true),
        ));
    }

    // A page with this address in the trash is left there.
    if let Some(url) = &source_url
        && !request.include_trashed
        && let Some(meta) = session.store.trashed_page_by_source_url(url)?
    {
        let (old, _) = stored_manifest(session.store, &meta)?;
        return Ok(as_it_is(session, index, &meta, &old, PageOutcome::Trashed));
    }

    // A new page. Where it goes: a folder that is there, or one that is created for it.
    let store = &session.store;
    let (parent, missing): (Option<Folder>, Vec<String>) = match &request.folder {
        None => (None, Vec::new()),
        Some(target) => match index.live_folder_or_root(store, target) {
            Ok(folder) => (folder, Vec::new()),
            Err(Error::NotFound { .. }) if request.create_folders => match target {
                Target::Path(path) => plan_folders(index, None, path)?,
                _ => return Err(index.live_folder_or_root(store, target).unwrap_err()),
            },
            Err(error) => return Err(error),
        },
    };
    let title = given_title(request).unwrap_or_else(|| page.title());
    let name = quoted_excerpt(&title);
    if session.dry_run {
        let mut folder_path = index.folder_segments(store, parent.as_ref().map(|f| f.id));
        let mut above = parent.as_ref().map(|folder| folder.id);
        for name in &missing {
            folder_path.push(name.clone());
            above = Some(FolderId(0));
        }
        let mut path = folder_path.clone();
        path.push(title.clone());
        let document = DocumentSummary {
            id: DocumentId(0),
            title: title.clone(),
            path: join_path(&path),
            folder_id: above,
            folder: join_path(&folder_path),
            words,
            characters,
            created_at: Time(now),
            updated_at: Time(now),
            trashed_at: None,
            kind: DocumentKind::Page,
            source_url: source_url.clone(),
        };
        return Ok(result(
            document,
            PageOutcome::Added,
            None,
            session.written(true),
        ));
    }
    // The files first: the store names no file it does not have.
    store_blobs(session.store, page)?;
    let mut folder_id = parent.as_ref().map(|folder| folder.id);
    for name in &missing {
        let folder = session.store.create_folder(folder_id, name)?;
        session.note(Change::FolderCreated { id: folder.id });
        folder_id = Some(folder.id);
    }
    let meta = session
        .store
        .create_page(
            folder_id,
            &PageContent {
                manifest: &json,
                title: &title,
                plain_text: &page.text,
                word_count: words,
                char_count: characters,
                source_url: source_url.as_deref(),
            },
        )
        .map_err(|error| store_refused(&name, error))?;
    session.note(Change::DocumentCreated { id: meta.id });
    Ok(result(
        index.document_summary(session.store, &meta),
        PageOutcome::Added,
        None,
        session.written(true),
    ))
}

// ----- reading -------------------------------------------------------------------------------

/// The ids of a folder and of every folder inside it.
fn subtree(node: &writer_store::FolderNode, ids: &mut HashSet<FolderId>) {
    ids.insert(node.folder.id);
    for child in &node.folders {
        subtree(child, ids);
    }
}

impl Library {
    /// The pages of the library, live and in the trash (marked), the one updated last
    /// first, each with what it is made of, where it came from and when its content was
    /// taken (`content_at`). With a folder: the pages in it, at any depth.
    pub fn pages(&self, request: &PagesRequest) -> Result<PagesList> {
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
        let mut pages = Vec::new();
        for meta in store.pages()? {
            if let Some(within) = &within
                && !meta.folder_id.is_some_and(|id| within.contains(&id))
            {
                continue;
            }
            let (manifest, _) = stored_manifest(store, &meta)?;
            pages.push(PageListed {
                page: index.document_summary(store, &meta),
                summary: summary(&meta, &manifest),
                trashed: meta.trashed_at.is_some(),
            });
        }
        Ok(PagesList { pages })
    }
}

/// [`Library::read`] of a page: the text it showed (the same in Markdown and as plain
/// text), or as JSON its manifest without the text.
pub(crate) fn read(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &ReadRequest,
) -> Result<DocumentText> {
    if let Some(heading) = &request.heading {
        return Err(Error::invalid(format!(
            "{} is a page: its text has no sections, so the section {} cannot be read; read \
             it whole",
            quoted_excerpt(title_of(meta)),
            quoted_excerpt(heading)
        )));
    }
    let (manifest, _) = stored_manifest(store, meta)?;
    Ok(DocumentText {
        document: index.document_summary(store, meta),
        format: request.format,
        content: match request.format {
            TextFormat::Json => without_text(&manifest),
            TextFormat::Markdown | TextFormat::PlainText => manifest.text.clone(),
        },
        section: None,
        comments: request.comments.then(Vec::new),
        images: Vec::new(),
        page: Some(summary(meta, &manifest)),
    })
}

/// [`Library::info`] of a page.
pub(crate) fn info(store: &Store, index: &Index, meta: &DocumentMeta) -> Result<DocumentInfo> {
    let (manifest, _) = stored_manifest(store, meta)?;
    let stats = text_stats(&manifest.text);
    Ok(DocumentInfo {
        document: index.document_summary(store, meta),
        custom_title: content::title_flag_set(store, meta),
        paragraphs: stats.paragraphs as u64,
        characters_no_spaces: stats.characters_no_spaces as u64,
        reading_minutes: stats.reading_minutes as u64,
        headings: Vec::new(),
        open_comments: 0,
        resolved_comments: 0,
        versions: store.snapshots(meta.id)?.len(),
        settings: content::settings_info(&writer_core::DocumentSettings::default()),
        page: Some(summary(meta, &manifest)),
    })
}

/// [`Library::find`] in a page: in the text it showed.
pub(crate) fn find(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &FindRequest,
) -> Result<FindResults> {
    let (manifest, _) = stored_manifest(store, meta)?;
    let document = text_document(&manifest.text);
    let matches = matching::find(&document, &request.text, true)?;
    Ok(FindResults {
        document: index.document_summary(store, meta),
        count: matches.ranges.len(),
        occurrences: matching::occurrences(
            &document,
            &matches.ranges,
            matches.matched_as,
            request.limit.unwrap_or(DEFAULT_FIND_LIMIT),
        ),
        matched_as: matches.matched_as,
    })
}

/// [`Library::comments`] of a page: a page has none.
pub(crate) fn comments(store: &Store, index: &Index, meta: &DocumentMeta) -> CommentList {
    CommentList {
        document: index.document_summary(store, meta),
        threads: Vec::new(),
    }
}

/// [`Library::read_version`] of a version of a page: the text it showed then, or its
/// manifest without the text.
pub(crate) fn read_version(
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    snapshot: &Snapshot,
    format: TextFormat,
) -> Result<VersionText> {
    let manifest = manifest_of(meta, &snapshot.content)?;
    Ok(VersionText {
        document: index.document_summary(store, meta),
        version: version_info(&snapshot.meta, store.snapshot_label(snapshot.meta.id)?),
        format,
        content: match format {
            TextFormat::Json => without_text(&manifest),
            TextFormat::Markdown | TextFormat::PlainText => manifest.text,
        },
    })
}

/// [`Library::restore_version`] of a version of a page: that manifest is current again,
/// after the page as it is was kept as a version. Its title comes back too, unless
/// somebody chose the one it has; its address stays.
pub(crate) fn restore_version(
    session: &mut Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    snapshot: &Snapshot,
    request: &RestoreVersionRequest,
) -> Result<VersionRestored> {
    let (_, raw) = stored_manifest(session.store, meta)?;
    let restored = manifest_of(meta, &snapshot.content)?;
    let changed = raw != snapshot.content;
    let words_before = meta.word_count;
    let stats = text_stats(&restored.text);
    let title = if content::title_flag_set(session.store, meta) {
        meta.title.clone()
    } else {
        snapshot.meta.title.clone()
    };
    let restored_version = version_info(
        &snapshot.meta,
        session.store.snapshot_label(snapshot.meta.id)?,
    );
    let mut after = DocumentMeta {
        title: title.clone(),
        word_count: stats.words as u64,
        char_count: stats.characters as u64,
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
            .save_page(
                meta.id,
                &PageContent {
                    manifest: &snapshot.content,
                    title: &title,
                    plain_text: &restored.text,
                    word_count: stats.words as u64,
                    char_count: stats.characters as u64,
                    source_url: meta.source_url.as_deref(),
                },
            )
            .map_err(|error| store_refused(&quoted_excerpt(&title), error))?;
        session.note(Change::DocumentText { id: meta.id });
        if after.title != meta.title {
            session.note(Change::DocumentRenamed { id: meta.id });
        }
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

/// [`rename_document`](crate::items::rename_document) of a page: a title somebody
/// chose, kept by every update; or, with `follow_text`, the flag taken away, so that the
/// next update takes the title the page gives (a page has no text a title follows).
pub(crate) fn rename(
    session: &mut Session<'_>,
    index: &Index,
    meta: &DocumentMeta,
    request: &crate::types::RenameDocumentRequest,
) -> Result<crate::types::DocumentChanged> {
    let chosen_before = content::title_flag_set(session.store, meta);
    let title = if request.follow_text {
        meta.title.clone()
    } else {
        let title = request
            .title
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if title.is_empty() {
            return Err(Error::invalid("the new title is empty"));
        }
        title
    };
    let chosen = !request.follow_text;
    let changed = title != meta.title || chosen != chosen_before;
    let before = index.document_summary(session.store, meta);
    let mut after = DocumentMeta {
        title: title.clone(),
        ..meta.clone()
    };
    if changed && !session.dry_run {
        if title != meta.title {
            session.store.rename_document(meta.id, &title)?;
        }
        after = session.store.document_meta(meta.id)?;
        content::set_title_flag(session.store, &after, chosen)?;
        session.note(Change::DocumentRenamed { id: meta.id });
    }
    Ok(crate::types::DocumentChanged {
        document: index.document_summary(session.store, &after),
        before: Some(before),
        created_folders: Vec::new(),
        written: session.written(changed),
    })
}

// ----- export --------------------------------------------------------------------------------

/// The format an export of a page is in: what the request says, else by the path (a
/// `.pdf`, a `.txt`, a path without a known ending a folder). A format of a text
/// (Markdown, DOCX, named or by the ending) is refused.
pub(crate) fn export_format(meta: &DocumentMeta, request: &ExportRequest) -> Result<ExportFormat> {
    let format = match request.format {
        Some(format) => format,
        None => match request
            .path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(ExportFormat::parse)
        {
            Some(format) => format,
            None => ExportFormat::Html,
        },
    };
    match format {
        ExportFormat::Markdown | ExportFormat::Docx => Err(content::is_a_page(meta)),
        format => Ok(format),
    }
}

/// [`Library::export`] of a page: its files as a folder that opens offline (or the files
/// it was added from), its text, or a PDF as WebKit prints it.
pub(crate) fn export(
    library: &Library,
    store: &Store,
    index: &Index,
    meta: &DocumentMeta,
    request: &ExportRequest,
    format: ExportFormat,
    path: PathBuf,
) -> Result<Exported> {
    let (manifest, _) = stored_manifest(store, meta)?;
    let title = title_of(meta).to_string();
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| Error::Io { path, source }
    };
    let mut exported = Exported {
        path: path.clone(),
        format,
        bytes: 0,
        title,
        documents: vec![index.document_summary(store, meta)],
        assets: Vec::new(),
        files: 0,
        warnings: Vec::new(),
        dry_run: request.dry_run,
    };
    let mut bytes_of = |id: &str| -> std::io::Result<Option<Vec<u8>>> {
        let Some(id) = FileId::parse(id) else {
            return Ok(None);
        };
        store
            .file_bytes(&id)
            .map_err(|error| std::io::Error::other(error.to_string()))
    };
    match format {
        ExportFormat::PlainText => {
            if path.is_dir() {
                return Err(Error::invalid(format!(
                    "{} is a directory; name the file to write",
                    path.display()
                )));
            }
            if path.exists() && !request.overwrite {
                return Err(Error::FileExists { path });
            }
            let mut text = manifest.text.clone();
            if !text.ends_with('\n') {
                text.push('\n');
            }
            exported.bytes = text.len() as u64;
            if !request.dry_run {
                if let Some(parent) = path
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                {
                    std::fs::create_dir_all(parent).map_err(io(parent))?;
                }
                crate::export::write_whole(&path, text.as_bytes())?;
            }
        }
        ExportFormat::Html => {
            if path.is_file() {
                return Err(Error::invalid(format!(
                    "{} is a file; a page is written as a folder: name the folder to write",
                    path.display()
                )));
            }
            let which = if request.original {
                writer_page::Which::Original
            } else {
                writer_page::Which::Served
            };
            let files = match which {
                writer_page::Which::Served => &manifest.files,
                writer_page::Which::Original => &manifest.original,
            };
            if request.dry_run {
                let mut seen = HashSet::new();
                exported.files = files.len();
                exported.bytes = files
                    .iter()
                    .filter(|file| seen.insert(file.path.as_str()))
                    .map(|file| file.size)
                    .sum();
                for file in files {
                    let target = path.join(&file.path);
                    if !request.overwrite && std::fs::symlink_metadata(&target).is_ok() {
                        return Err(Error::FileExists { path: target });
                    }
                }
                return Ok(exported);
            }
            let written = writer_page::write_folder(
                &manifest,
                which,
                &path,
                request.overwrite,
                &mut bytes_of,
            )?;
            exported.files = written.files;
            exported.bytes = written.bytes;
            if !written.missing.is_empty() {
                exported.warnings.push(missing_sentence(&written.missing));
            }
        }
        ExportFormat::Pdf => {
            if request.original {
                return Err(Error::invalid(
                    "a PDF is made of the page as it is shown; the files it was added from are \
                     written as a folder",
                ));
            }
            if path.is_dir() {
                return Err(Error::invalid(format!(
                    "{} is a directory; name the PDF file to write",
                    path.display()
                )));
            }
            if path.exists() && !request.overwrite {
                return Err(Error::FileExists { path });
            }
            let Some(checker) = library.page_checker.clone() else {
                return Err(Error::failed(NO_CHECKER));
            };
            if request.dry_run {
                return Ok(exported);
            }
            let dir = crate::checker::WorkDir::new("pdf")?;
            let mut missing = Vec::new();
            let mut counted = |id: &str| -> std::io::Result<Option<Vec<u8>>> {
                let found = bytes_of(id)?;
                if found.is_none() {
                    missing.push(id.to_string());
                }
                Ok(found)
            };
            writer_page::write_served_dir(&manifest, &dir.path, &mut counted)?;
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).map_err(io(parent))?;
            }
            let mut partial = path.clone().into_os_string();
            partial.push(format!(".partial-{}", std::process::id()));
            let partial = PathBuf::from(partial);
            let printed = checker.print(&dir.path, &partial);
            if let Err(error) = printed {
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
            if !missing.is_empty() {
                let paths: Vec<String> = manifest
                    .files
                    .iter()
                    .filter(|file| missing.contains(&file.file))
                    .map(|file| file.path.clone())
                    .collect();
                exported.warnings.push(missing_sentence(&paths));
            }
        }
        ExportFormat::Markdown | ExportFormat::Docx => return Err(content::is_a_page(meta)),
    }
    Ok(exported)
}

/// "3 files of the page are missing from the library and were not written: a.png, b.png
/// and 1 more"
fn missing_sentence(paths: &[String]) -> String {
    let named: Vec<&str> = paths.iter().take(5).map(String::as_str).collect();
    let mut list = named.join(", ");
    if paths.len() > named.len() {
        list.push_str(&format!(" and {} more", paths.len() - named.len()));
    }
    format!(
        "{} {} of the page {} missing from the library and {} not written: {list}",
        paths.len(),
        if paths.len() == 1 { "file" } else { "files" },
        if paths.len() == 1 { "is" } else { "are" },
        if paths.len() == 1 { "was" } else { "were" },
    )
}
