//! Pages: web pages added from a file or a folder, kept whole, read, found, exported, and
//! never edited. The web answers from memory (`MapFetcher`) and the check is the fake one
//! (`StaticChecker`): no test here reaches the network or opens WebKit.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use innerword_agent::live::{self, Answer, Operation, Request};
use innerword_agent::{
    AddCommentRequest, AddImageRequest, AddPageRequest, CommentRequest, CreateDocumentRequest,
    DeleteCommentRequest, EditRequest, Error, ErrorKind, ExportFormat, ExportRequest, FindRequest,
    InfoRequest, InsertRequest, Library, ListCommentsRequest, ListVersionsRequest,
    PAGE_IS_NOT_A_TEXT_FILE, PageAdded, PageChecker, PageOutcome, PagesRequest, PreparedPage,
    ReadRequest, ReadVersionRequest, RenameDocumentRequest, ReplaceDocumentRequest,
    ReplaceSectionRequest, ReplyRequest, SearchRequest, StaticChecker, Target, TextFormat,
    TrashDocumentRequest, TreeRequest, WriteOptions,
};
use innerword_page::{Bundle, CheckReport, FetchError, Fetched, Fetcher, MapFetcher};
use innerword_store::{DocumentKind, Store};

use super::rules::dry;
use super::{Fixture, claude, closed_fixture, fixture};

// ----- what the tests share ---------------------------------------------------------------

/// The page of most tests: a title, a heading, a script and a style sheet whose words are
/// not text, a picture of its own, and a style sheet on the web.
fn orbitarium(fx: &Fixture, words: &str) -> PathBuf {
    fx.page_folder(
        "orbitarium",
        &[
            (
                "index.html",
                &format!(
                    "<!doctype html><html><head><title>Orbitarium</title>\
                     <link rel=stylesheet href=\"https://fonts.example.com/k.css\">\
                     <style>p {{ color: red }}</style></head>\
                     <body><h1>The sky at night</h1><p>{words}</p>\
                     <img src=\"stars.png\" alt=\"\"><script>var hidden = 'not text';</script>\
                     </body></html>"
                ),
            ),
            ("stars.png", "not really a picture, but bytes all the same"),
        ],
    )
}

/// The answers of the web for [`orbitarium`].
fn web() -> MapFetcher {
    MapFetcher::new().with(
        "https://fonts.example.com/k.css",
        "text/css",
        "body { font-family: serif }",
    )
}

/// A checker that counts how often it was asked, and checks as the fake does.
#[derive(Default)]
struct Counting {
    checks: AtomicUsize,
}

impl PageChecker for Counting {
    fn check(&self, bundle: &Bundle) -> innerword_agent::Result<CheckReport> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        StaticChecker.check(bundle)
    }

    fn print(&self, dir: &Path, out: &Path) -> innerword_agent::Result<()> {
        StaticChecker.print(dir, out)
    }

    fn print_board(&self, dir: &Path, out: &Path) -> innerword_agent::Result<()> {
        StaticChecker.print_board(dir, out)
    }
}

/// A fetcher that notes every address it is asked, and answers as `inner` does.
struct Noting {
    asked: Mutex<Vec<String>>,
    inner: MapFetcher,
}

impl Fetcher for Noting {
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError> {
        self.asked.lock().unwrap().push(url.to_string());
        self.inner.fetch(url)
    }
}

/// The library of `fx` with the web of [`web`] and a checker that counts.
fn counted(fx: &Fixture) -> (Library, Arc<Counting>) {
    let checker = Arc::new(Counting::default());
    let library = fx
        .library
        .clone()
        .with_fetcher(Arc::new(web()))
        .with_page_checker(checker.clone());
    (library, checker)
}

fn add(path: &Path) -> AddPageRequest {
    AddPageRequest {
        path: path.to_string_lossy().into_owned(),
        write: claude(),
        ..Default::default()
    }
}

fn with_address(path: &Path, url: &str) -> AddPageRequest {
    AddPageRequest {
        source_url: Some(url.to_string()),
        folder: Some(Target::path("Claude Pages")),
        create_folders: true,
        ..add(path)
    }
}

const ADDRESS: &str = "https://claude.ai/artifact/ExampleArtifactId00001";

fn read(
    library: &Library,
    page: impl Into<Target>,
    format: TextFormat,
) -> innerword_agent::DocumentText {
    library
        .read(&ReadRequest {
            document: page.into(),
            format,
            ..Default::default()
        })
        .unwrap()
}

// ----- adding ------------------------------------------------------------------------------

#[test]
fn a_page_is_added_from_a_folder_with_what_it_names_on_the_web() {
    let fx = fixture();
    let (library, checker) = counted(&fx);
    let folder = orbitarium(&fx, "Orion is up.");
    let added = library
        .add_page(&AddPageRequest {
            folder: Some(Target::path("Claude Pages")),
            create_folders: true,
            source_url: Some("HTTPS://Claude.AI/artifact/ExampleArtifactId00001/#top".into()),
            ..add(&folder)
        })
        .unwrap();
    assert_eq!(checker.checks.load(Ordering::SeqCst), 1);
    assert_eq!(added.outcome, PageOutcome::Added);
    assert!(added.written.changed && !added.written.dry_run);
    assert!(added.snapshot.is_none());
    let page = &added.page;
    assert_eq!(page.kind, DocumentKind::Page);
    assert!(page.is_page());
    assert_eq!(page.title, "Orbitarium");
    assert_eq!(page.path, "Claude Pages/Orbitarium");
    assert_eq!(
        page.source_url.as_deref(),
        Some(ADDRESS),
        "in its normal form"
    );
    assert_eq!(added.externals, 1);
    assert_eq!(
        added.files, 3,
        "the two files and the style sheet from the web"
    );
    assert!(added.bytes > 0);
    assert!(added.warnings.is_empty(), "{:?}", added.warnings);

    // The files are in the library, the row is a page.
    let store = fx.store();
    assert_eq!(
        store.file_count().unwrap(),
        4,
        "the two files as served and the HTML as given"
    );
    let meta = store.document_meta(page.id).unwrap();
    assert_eq!(meta.kind, DocumentKind::Page);
    assert_eq!(meta.source_url.as_deref(), Some(ADDRESS));
    let manifest =
        innerword_page::Manifest::from_json(&store.document(page.id).unwrap().content).unwrap();
    assert_eq!(manifest.entry, "index.html");
    assert_eq!(manifest.source_url.as_deref(), Some(ADDRESS));
    assert_eq!(manifest.added_at, super::START);
    assert!(manifest.text.contains("Orion is up."));
    assert!(!manifest.text.contains("not text"), "a script is no text");
    assert!(
        manifest
            .files
            .iter()
            .any(|file| file.path == "_external/fonts.example.com/k.css")
    );
    // The text is the row's, and search finds it.
    let hits = library
        .search(&SearchRequest {
            query: "Orion".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(hits.hits.len(), 1);
    assert_eq!(hits.hits[0].document.id, page.id);
    assert_eq!(hits.hits[0].document.kind, DocumentKind::Page);

    // A file on its own: its name is the title when it has none.
    let single = fx.dir.path().join("loose/explainer.html");
    std::fs::create_dir_all(single.parent().unwrap()).unwrap();
    std::fs::write(&single, "<!doctype html><p>Just this.</p>").unwrap();
    let alone = library.add_page(&add(&single)).unwrap();
    assert_eq!(alone.outcome, PageOutcome::Added);
    assert_eq!(alone.page.title, "explainer");
    assert_eq!(alone.page.path, "explainer");
    assert_eq!(alone.page.source_url, None);
    assert_eq!(alone.files, 1);
    // A title that is given is the title.
    let titled = library
        .add_page(&AddPageRequest {
            title: Some("  The   explainer ".into()),
            ..add(&single)
        })
        .unwrap();
    assert_eq!(titled.page.title, "The explainer");
    assert_ne!(
        titled.page.id, alone.page.id,
        "without an address every add is a new page"
    );
}

#[test]
fn the_same_address_again_is_unchanged_updated_or_left_in_the_trash() {
    let fx = fixture();
    let (library, checker) = counted(&fx);
    let folder = orbitarium(&fx, "First words.");
    let first = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(first.outcome, PageOutcome::Added);
    let id = first.page.id;
    assert_eq!(checker.checks.load(Ordering::SeqCst), 1);

    // The same page: nothing is checked or written, whatever case, fragment or slash the
    // address is given with.
    let before = fx.dump();
    for address in [
        ADDRESS.to_string(),
        ADDRESS.replace("claude.ai", "CLAUDE.ai"),
        format!("{ADDRESS}/"),
        format!("{ADDRESS}#section-2"),
        ADDRESS.replace("https://", "HTTPS://"),
    ] {
        fx.clock.advance_ms(1000);
        let again = library.add_page(&with_address(&folder, &address)).unwrap();
        assert_eq!(again.outcome, PageOutcome::Unchanged, "{address}");
        assert_eq!(again.page.id, id);
        assert!(!again.written.changed && !again.written.would_change);
        assert!(again.snapshot.is_none());
        assert_eq!(
            (again.files, again.externals),
            (first.files, first.externals)
        );
    }
    assert_eq!(
        checker.checks.load(Ordering::SeqCst),
        1,
        "nothing was checked again"
    );
    assert_eq!(fx.dump(), before, "nothing was written");

    // A rename in between: the title is the one chosen from now on, and the
    // fingerprint still decides.
    library
        .rename_document(&RenameDocumentRequest {
            document: id.into(),
            title: "My sky".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let after_rename = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(after_rename.outcome, PageOutcome::Unchanged);
    assert_eq!(after_rename.page.title, "My sky");

    // Another page at the address: updated, and the page as it was kept as a version.
    fx.clock.advance_ms(60_000);
    orbitarium(&fx, "Second words, and more of them.");
    let updated = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(updated.outcome, PageOutcome::Updated);
    assert_eq!(updated.page.id, id);
    assert_eq!(updated.page.title, "My sky", "a chosen title is kept");
    assert_eq!(
        updated.page.path, "Claude Pages/My sky",
        "and so is its place"
    );
    assert!(updated.written.changed);
    let snapshot = updated.snapshot.clone().expect("the page as it was");
    assert_eq!(
        snapshot.label.as_deref(),
        Some("Before an update by Claude")
    );
    assert_eq!(checker.checks.load(Ordering::SeqCst), 2);
    let text = read(&library, id, TextFormat::PlainText);
    assert!(text.content.contains("Second words"), "{}", text.content);
    let page = text.page.expect("a page says what it is");
    assert_eq!(page.content_at.unix_ms(), fx.clock.now_ms());
    let old = library
        .read_version(&ReadVersionRequest {
            version: snapshot.id.0,
            format: TextFormat::PlainText,
        })
        .unwrap();
    assert!(old.content.contains("First words."), "{}", old.content);
    let versions = library
        .versions(&ListVersionsRequest {
            document: id.into(),
        })
        .unwrap();
    assert_eq!(versions.versions[0].id, snapshot.id);
    // The search follows the new text.
    let found = |query: &str| {
        library
            .search(&SearchRequest {
                query: query.into(),
                ..Default::default()
            })
            .unwrap()
            .hits
            .len()
    };
    assert_eq!((found("Second"), found("First")), (1, 0));

    // Without a chosen title an update takes the page's title.
    library
        .rename_document(&RenameDocumentRequest {
            document: id.into(),
            follow_text: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    orbitarium(&fx, "Third words.");
    let third = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(third.outcome, PageOutcome::Updated);
    assert_eq!(third.page.title, "Orbitarium");

    // To the trash: an add of the address leaves it there and says so.
    library
        .trash_document(&TrashDocumentRequest {
            document: id.into(),
            write: claude(),
        })
        .unwrap();
    let checks = checker.checks.load(Ordering::SeqCst);
    let before = fx.dump();
    let trashed = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(trashed.outcome, PageOutcome::Trashed);
    assert_eq!(trashed.page.id, id);
    assert!(trashed.page.trashed_at.is_some());
    assert!(!trashed.written.changed);
    assert_eq!(
        checker.checks.load(Ordering::SeqCst),
        checks,
        "nothing was checked"
    );
    assert_eq!(fx.dump(), before);
    // Asked to, it is added as new.
    let again = library
        .add_page(&AddPageRequest {
            include_trashed: true,
            ..with_address(&folder, ADDRESS)
        })
        .unwrap();
    assert_eq!(again.outcome, PageOutcome::Added);
    assert_ne!(again.page.id, id);
    // From then on the live one is the page at the address.
    let once_more = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    assert_eq!(once_more.outcome, PageOutcome::Unchanged);
    assert_eq!(once_more.page.id, again.page.id);
}

#[test]
fn pieces_the_caller_has_are_not_fetched_and_a_piece_that_is_missing_saves_nothing() {
    let fx = fixture();
    fx.create(None, "# A text\n\nFirst.\n");
    let folder = fx.page_folder(
        "assets",
        &[(
            "index.html",
            "<!doctype html><title>Assets</title><img src=\"https://claude.ai/api/assets/1f\">",
        )],
    );
    let asked = Arc::new(Noting {
        asked: Mutex::new(Vec::new()),
        inner: MapFetcher::new(),
    });
    let library = fx.library.clone().with_fetcher(asked.clone());

    // Without the piece: not added, and why.
    let before = fx.dump();
    let error = library.add_page(&add(&folder)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PageIncomplete);
    assert_eq!(error.exit_code(), 14);
    assert_eq!(error.code(), "page_incomplete");
    let message = error.to_string();
    assert!(
        message.starts_with("\u{201c}Assets\u{201d} was not added: it could not be kept whole. "),
        "{message}"
    );
    assert!(
        message.contains("https://claude.ai/api/assets/1f could not be fetched: HTTP 404"),
        "{message}"
    );
    assert!(message.ends_with("Nothing was saved."), "{message}");
    assert_eq!(fx.dump(), before, "nothing was saved");
    let report = error.report();
    assert_eq!(report.code, "page_incomplete");
    assert_eq!(report.message, message);
    // A page given a title is refused under that title, not under the one it has itself
    // (an artifact of a type calls itself "Claude Docs").
    let titled = library
        .add_page(&AddPageRequest {
            title: Some("Planning doc".to_string()),
            ..add(&folder)
        })
        .unwrap_err();
    assert_eq!(titled.kind(), ErrorKind::PageIncomplete);
    assert!(
        titled
            .to_string()
            .starts_with("\u{201c}Planning doc\u{201d} was not added: "),
        "{titled}"
    );
    assert_eq!(fx.dump(), before, "nothing was saved");

    // With the file that stands for it: nothing is fetched, the file is taken.
    asked.asked.lock().unwrap().clear();
    let picture = fx.dir.path().join("saved/asset.png");
    std::fs::create_dir_all(picture.parent().unwrap()).unwrap();
    std::fs::write(&picture, innerword_image::fixtures::png(4, 4, [1, 2, 3])).unwrap();
    let added = library
        .add_page(&AddPageRequest {
            externals: BTreeMap::from([(
                "https://claude.ai/api/assets/1f".to_string(),
                picture.to_string_lossy().into_owned(),
            )]),
            ..add(&folder)
        })
        .unwrap();
    assert_eq!(added.outcome, PageOutcome::Added);
    assert_eq!(added.externals, 1);
    assert!(
        asked.asked.lock().unwrap().is_empty(),
        "{:?}",
        asked.asked.lock().unwrap()
    );
    // A relative file is taken from the folder the request names.
    let relative = library
        .add_page(&AddPageRequest {
            externals: BTreeMap::from([(
                "https://claude.ai/api/assets/1f".to_string(),
                "saved/asset.png".to_string(),
            )]),
            base_dir: Some(fx.dir.path().to_path_buf()),
            ..add(&folder)
        })
        .unwrap();
    assert_eq!(relative.outcome, PageOutcome::Added);
    // And without a folder it is refused before anything happens.
    let error = library
        .add_page(&AddPageRequest {
            externals: BTreeMap::from([(
                "https://claude.ai/api/assets/1f".to_string(),
                "saved/asset.png".to_string(),
            )]),
            ..add(&folder)
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    assert!(error.to_string().contains("relative path"), "{error}");
}

#[test]
fn a_page_the_request_says_was_prepared_is_prepared_again() {
    let fx = fixture();
    let (library, checker) = counted(&fx);
    let folder = orbitarium(&fx, "Prepared?");
    // Made up: "the same page is there".
    let forged = library
        .add_page(&AddPageRequest {
            prepared: Some(PreparedPage::Unchanged {
                page: innerword_store::DocumentId(1),
                fingerprint: "0".repeat(64),
            }),
            ..with_address(&folder, ADDRESS)
        })
        .unwrap();
    assert_eq!(forged.outcome, PageOutcome::Added);
    assert_eq!(checker.checks.load(Ordering::SeqCst), 1, "the check ran");

    // Made up: a page whose files are stored already (they are not).
    let other = fx.page_folder(
        "other",
        &[("index.html", "<title>Other</title><p>Other.</p>")],
    );
    let input = innerword_page::read_input(&other, None).unwrap();
    let bundle = innerword_page::bundle(input, &MapFetcher::new()).unwrap();
    let mut page =
        innerword_page::prepare(bundle.clone(), &innerword_page::static_check(&bundle)).unwrap();
    for blob in &mut page.blobs {
        blob.data.clear();
        blob.stored = true;
    }
    let added = library
        .add_page(&AddPageRequest {
            prepared: Some(PreparedPage::Page(page)),
            ..add(&other)
        })
        .unwrap();
    assert_eq!(added.outcome, PageOutcome::Added);
    assert_eq!(
        checker.checks.load(Ordering::SeqCst),
        2,
        "the check ran again"
    );
    let store = fx.store();
    let manifest =
        innerword_page::Manifest::from_json(&store.document(added.page.id).unwrap().content)
            .unwrap();
    for file in &manifest.files {
        let id = innerword_store::FileId::parse(&file.file).unwrap();
        assert!(
            store.file_info(&id).unwrap().is_some(),
            "{} is stored",
            file.path
        );
    }
}

#[test]
fn without_a_checker_pages_cannot_be_added_or_printed() {
    let fx = fixture();
    let folder = orbitarium(&fx, "Unchecked.");
    let bare = Library::at(fx.data_dir())
        .with_clock(fx.clock.clone())
        .with_fetcher(Arc::new(web()));
    let error = bare.add_page(&add(&folder)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Failed);
    assert_eq!(error.exit_code(), 1);
    assert_eq!(error.to_string(), "pages cannot be checked here");
    assert!(!fx.library.database_path().exists() || fx.store().pages().unwrap().is_empty());

    let added = fx
        .library
        .clone()
        .with_fetcher(Arc::new(web()))
        .add_page(&add(&folder))
        .unwrap();
    let error = bare
        .export(&ExportRequest {
            document: Some(added.page.id.into()),
            path: fx.dir.path().join("out/page.pdf"),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Failed);
    assert_eq!(error.to_string(), "pages cannot be checked here");
    assert!(!fx.dir.path().join("out/page.pdf").exists());
}

#[test]
fn a_page_is_not_a_text_and_a_text_is_not_a_page() {
    let fx = fixture();
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = orbitarium(&fx, "Read only.");
    let page = library.add_page(&add(&folder)).unwrap().page;
    let target = || Target::DocumentId(page.id.0);
    let before = fx.dump();
    let o = claude;
    let picture = fx.picture("p.png", (8, 8), [9, 9, 9]);
    let results: Vec<(&str, innerword_agent::Result<()>)> = vec![
        (
            "replace_document",
            library
                .replace_document(&ReplaceDocumentRequest {
                    document: target(),
                    markdown: "x".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "edit",
            library
                .edit(&EditRequest {
                    document: target(),
                    old_text: "Read only.".into(),
                    new_text: "Changed.".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "insert",
            library
                .insert(&InsertRequest {
                    document: target(),
                    markdown: "More.".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "replace_section",
            library
                .replace_section(&ReplaceSectionRequest {
                    document: target(),
                    heading: "The sky at night".into(),
                    markdown: "x".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "add_image",
            library
                .add_image(&AddImageRequest {
                    document: target(),
                    path: picture.to_string_lossy().into_owned(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "add_comment",
            library
                .add_comment(&AddCommentRequest {
                    document: target(),
                    quote: "Read only".into(),
                    body: "?".into(),
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "reply",
            library
                .reply(&ReplyRequest {
                    document: target(),
                    comment: 1,
                    body: "?".into(),
                    write: o(),
                })
                .map(drop),
        ),
        (
            "resolve_comment",
            library
                .resolve_comment(&CommentRequest {
                    document: target(),
                    comment: 1,
                    write: o(),
                })
                .map(drop),
        ),
        (
            "reopen_comment",
            library
                .reopen_comment(&CommentRequest {
                    document: target(),
                    comment: 1,
                    write: o(),
                })
                .map(drop),
        ),
        (
            "delete_comment",
            library
                .delete_comment(&DeleteCommentRequest {
                    document: target(),
                    comment: 1,
                    write: o(),
                    ..Default::default()
                })
                .map(drop),
        ),
    ];
    for (name, result) in results {
        let error = result.expect_err(name);
        assert_eq!(error.kind(), ErrorKind::WrongKind, "{name}: {error}");
        assert_eq!(error.exit_code(), 15, "{name}");
        assert_eq!(error.code(), "wrong_kind", "{name}");
        assert_eq!(
            error.to_string(),
            "\u{201c}Orbitarium\u{201d} is a page: it is kept as it was saved, and its text \
             cannot be changed. To bring a newer copy, add it again with its source address.",
            "{name}"
        );
        let report = error.report();
        assert_eq!(report.code, "wrong_kind");
    }
    // An export as Markdown or DOCX changes nothing either, and is answered with the
    // formats a page has (not with how a newer copy comes in).
    let exports = [
        (
            "export as Markdown",
            library
                .export(&ExportRequest {
                    document: Some(target()),
                    path: fx.dir.path().join("out/page.md"),
                    ..Default::default()
                })
                .map(drop),
        ),
        (
            "export as DOCX",
            library
                .export(&ExportRequest {
                    document: Some(target()),
                    path: fx.dir.path().join("out/page.out"),
                    format: Some(ExportFormat::Docx),
                    ..Default::default()
                })
                .map(drop),
        ),
    ];
    for (name, result) in exports {
        let error = result.expect_err(name);
        assert_eq!(error.kind(), ErrorKind::WrongKind, "{name}: {error}");
        assert_eq!(error.exit_code(), 15, "{name}");
        assert_eq!(
            error.to_string(),
            "\u{201c}Orbitarium\u{201d} is a page, and a page is not exported as Markdown or \
             DOCX: it is written as a folder of its files (the format html), as a PDF (pdf) or as \
             its text (text).",
            "{name}"
        );
        assert!(error.to_string().contains(PAGE_IS_NOT_A_TEXT_FILE));
    }
    assert_eq!(fx.dump(), before, "nothing was changed");
    assert!(!fx.dir.path().join("out/page.md").exists());

    // A page's formats are not a text's.
    let text = fx.create(None, "# Chapter 2\n\nWords.\n");
    for (path, format) in [
        ("out/c.pdf", None),
        ("out/c", Some(ExportFormat::Html)),
        ("out/c", Some(ExportFormat::Pdf)),
        // The files a text was added from: it was added from none.
        ("out/c.md", None),
    ] {
        let error = library
            .export(&ExportRequest {
                document: Some(text.id.into()),
                path: fx.dir.path().join(path),
                format,
                original: path == "out/c.md",
                ..Default::default()
            })
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::WrongKind, "{path} {format:?}");
        assert_eq!(
            error.to_string(),
            "\u{201c}Chapter 2\u{201d} is a document, not a page."
        );
    }
    // As the app would answer it.
    for error in [
        Error::WrongKind {
            id: 3,
            name: "\u{201c}Orbitarium\u{201d}".into(),
            kind: innerword_store::DocumentKind::Page,
            wanted: None,
            export: false,
        },
        Error::WrongKind {
            id: 3,
            name: "\u{201c}Orbitarium\u{201d}".into(),
            kind: innerword_store::DocumentKind::Page,
            wanted: None,
            export: true,
        },
        Error::PageIncomplete {
            name: "Orbitarium".into(),
            problems: vec![innerword_page::Problem::new(
                "https://x.example/a.woff2",
                "https://x.example/a.woff2 could not be fetched: HTTP 404",
            )],
        },
        Error::failed("pages cannot be checked here"),
    ] {
        let travelled = live::WireError::of(&error).into_error();
        assert_eq!(travelled.to_string(), error.to_string());
        assert_eq!(travelled.kind(), error.kind());
        assert_eq!(format!("{travelled:?}"), format!("{error:?}"));
    }
}

/// A claude.ai artifact made from a type (a Claude Docs document) is the type's program,
/// which asks claude.ai for the content: its refusal says that, not which file of the
/// program asked for something it does not have.
#[test]
fn an_artifact_made_from_a_type_is_refused_as_one() {
    let fx = fixture();
    fx.create(None, "# A text\n\nFirst.\n");
    let files: &[(&str, &str)] = &[
        (
            "index.html",
            "<!doctype html><title>Claude Docs</title><p>Loading the document.</p>",
        ),
        (
            "artifact-type/thumbnail/render.html",
            "<!doctype html><title>Thumbnail</title><img src=\"__thumb/data.png\" alt=\"\">",
        ),
    ];
    let folder = fx.page_folder("growth", files);
    // As WebKit finds it: the thumbnail asks for a file the artifact does not publish.
    struct Thumbnail;
    impl PageChecker for Thumbnail {
        fn check(&self, bundle: &Bundle) -> innerword_agent::Result<CheckReport> {
            let mut report = StaticChecker.check(bundle)?;
            for load in &mut report.loads {
                if let Some(dir) = load.path.strip_suffix("render.html") {
                    load.missing.push(format!("{dir}__thumb/data.json"));
                }
            }
            Ok(report)
        }

        fn print(&self, dir: &Path, out: &Path) -> innerword_agent::Result<()> {
            StaticChecker.print(dir, out)
        }

        fn print_board(&self, dir: &Path, out: &Path) -> innerword_agent::Result<()> {
            StaticChecker.print_board(dir, out)
        }
    }
    let library = fx
        .library
        .clone()
        .with_fetcher(Arc::new(MapFetcher::new()))
        .with_page_checker(Arc::new(Thumbnail));
    let before = fx.dump();
    let error = library
        .add_page(&AddPageRequest {
            title: Some("Nimbus Planning Doc".to_string()),
            ..with_address(&folder, "https://claude.ai/artifact/ExampleArtifactId00003")
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PageIncomplete, "{error}");
    assert_eq!(
        error.to_string(),
        "\u{201c}Nimbus Planning Doc\u{201d} was not added: it could not be kept whole. It is \
         a claude.ai artifact made from a type (a Claude Docs document, a deck): its files are \
         the type's program, which asks claude.ai for the content, so the content is not among \
         them. Export the content as HTML (a Claude Docs document: the Claude Docs connector's \
         export) and add that. Nothing was saved."
    );
    assert_eq!(fx.dump(), before, "nothing was saved");

    // The same files in a folder of another name: the file is named, as for any page.
    let other: Vec<(&str, &str)> = files
        .iter()
        .map(|(path, html)| (path.trim_start_matches("artifact-type/"), *html))
        .collect();
    let folder = fx.page_folder("plain", &other);
    let error = library.add_page(&add(&folder)).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::PageIncomplete, "{error}");
    assert!(error.to_string().contains("__thumb/data.json"), "{error}");
    assert!(!error.to_string().contains("claude.ai artifact"), "{error}");
}

#[test]
fn every_item_operation_works_on_a_page() {
    let fx = fixture();
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = orbitarium(&fx, "An item.");
    let page = library
        .add_page(&with_address(&folder, ADDRESS))
        .unwrap()
        .page;
    // Renamed: a chosen title.
    let renamed = library
        .rename_document(&RenameDocumentRequest {
            document: page.id.into(),
            title: "Night sky".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(renamed.document.title, "Night sky");
    assert!(
        library
            .info(&InfoRequest {
                document: page.id.into(),
            })
            .unwrap()
            .custom_title
    );
    // Copied: a page without an address.
    let copy = library
        .duplicate_document(&innerword_agent::DuplicateDocumentRequest {
            document: page.id.into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap()
        .document;
    assert_eq!(copy.kind, DocumentKind::Page);
    assert_eq!(copy.source_url, None);
    // Listed without the address too: the copy carries the original's manifest.
    let listed = library.pages(&PagesRequest::default()).unwrap();
    let listed_copy = listed.pages.iter().find(|p| p.page.id == copy.id).unwrap();
    assert_eq!(listed_copy.summary.source_url, None);
    let listed_page = listed.pages.iter().find(|p| p.page.id == page.id).unwrap();
    assert_eq!(listed_page.summary.source_url.as_deref(), Some(ADDRESS));
    assert_eq!(
        read(&library, copy.id, TextFormat::PlainText).content,
        read(&library, page.id, TextFormat::PlainText).content
    );
    // Moved, to the trash, back, and deleted for good.
    library
        .move_document(&innerword_agent::MoveDocumentRequest {
            document: copy.id.into(),
            folder: Some(Target::root()),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    library
        .trash_document(&TrashDocumentRequest {
            document: copy.id.into(),
            write: claude(),
        })
        .unwrap();
    library
        .restore_document(&innerword_agent::RestoreDocumentRequest {
            document: copy.id.into(),
            write: claude(),
        })
        .unwrap();
    library
        .delete_document(&innerword_agent::DeleteDocumentRequest {
            document: copy.id.into(),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert!(fx.store().document_meta(copy.id).is_err());
    // A version back: the page as it was, with the title somebody chose.
    orbitarium(&fx, "A newer item.");
    let updated = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    let restored = library
        .restore_version(&innerword_agent::RestoreVersionRequest {
            version: updated.snapshot.unwrap().id.0,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(restored.written.changed);
    assert!(
        restored.snapshot.is_some(),
        "the newer page is kept as a version"
    );
    assert_eq!(restored.document.title, "Night sky");
    assert!(
        read(&library, page.id, TextFormat::PlainText)
            .content
            .contains("An item.")
    );
    let meta = fx.store().document_meta(page.id).unwrap();
    assert_eq!(
        meta.source_url.as_deref(),
        Some(ADDRESS),
        "the address stays"
    );
    // The page as it was is the same as the one added again: unchanged.
    orbitarium(&fx, "An item.");
    assert_eq!(
        library
            .add_page(&with_address(&folder, ADDRESS))
            .unwrap()
            .outcome,
        PageOutcome::Unchanged
    );
}

// ----- reading -----------------------------------------------------------------------------

#[test]
fn a_page_is_read_found_listed_and_described() {
    let fx = fixture();
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = orbitarium(&fx, "Orion and Taurus. Orion again.");
    let added = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    let id = added.page.id;
    fx.create(Some("Claude Pages"), "# Notes\n\nA text beside it.\n");

    // The text as the page showed it, the same in Markdown and as plain text.
    let markdown = read(&library, id, TextFormat::Markdown);
    let text = read(&library, id, TextFormat::PlainText);
    assert_eq!(markdown.content, text.content);
    assert!(
        markdown.content.contains("The sky at night"),
        "{}",
        markdown.content
    );
    assert!(markdown.content.contains("Orion and Taurus."));
    assert!(markdown.images.is_empty() && markdown.section.is_none());
    let summary = markdown.page.clone().expect("what the page is");
    assert_eq!(summary.source_url.as_deref(), Some(ADDRESS));
    assert_eq!(summary.entry, "index.html");
    assert_eq!(summary.files, added.files);
    assert_eq!(summary.bytes, added.bytes);
    assert_eq!(summary.externals, 1);
    assert_eq!(summary.engine, innerword_page::STATIC_ENGINE);
    assert_eq!(summary.content_at.unix_ms(), super::START);
    // As JSON: the manifest without the text.
    let json = read(&library, id, TextFormat::Json);
    let manifest: serde_json::Value = serde_json::from_str(&json.content).unwrap();
    assert_eq!(manifest["entry"], "index.html");
    assert!(manifest.get("text").is_none(), "left out, not emptied");
    assert!(manifest["files"].as_array().unwrap().len() >= 3);
    // A section of a page cannot be read; its comments are none.
    let error = library
        .read(&ReadRequest {
            document: id.into(),
            heading: Some("The sky at night".into()),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Invalid);
    let with_comments = library
        .read(&ReadRequest {
            document: id.into(),
            comments: true,
            ..Default::default()
        })
        .unwrap();
    assert_eq!(with_comments.comments, Some(Vec::new()));
    let comments = library
        .comments(&ListCommentsRequest {
            document: id.into(),
            ..Default::default()
        })
        .unwrap();
    assert!(comments.threads.is_empty());
    assert_eq!(comments.document.id, id);

    // What is known about it.
    let info = library
        .info(&InfoRequest {
            document: id.into(),
        })
        .unwrap();
    assert_eq!(info.document.kind, DocumentKind::Page);
    assert!(!info.custom_title);
    assert!(info.headings.is_empty());
    assert_eq!((info.open_comments, info.resolved_comments), (0, 0));
    assert_eq!(info.page.as_ref(), Some(&summary));
    assert_eq!(info.versions, 0);
    let text_info = library
        .info(&InfoRequest {
            document: Target::path("Claude Pages/Notes"),
        })
        .unwrap();
    assert_eq!(text_info.page, None);
    assert_eq!(text_info.document.kind, DocumentKind::Text);

    // Found in its text.
    let found = library
        .find(&FindRequest {
            document: id.into(),
            text: "Orion".into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(found.count, 2);
    assert_eq!(found.document.id, id);

    // Listed: in the tree with its kind, among the pages with its summary.
    let tree = library.tree(&TreeRequest::default()).unwrap();
    let in_folder = &tree.folders[0].documents;
    assert_eq!(in_folder.len(), 2);
    let listed = in_folder.iter().find(|d| d.id == id).unwrap();
    assert_eq!(listed.kind, DocumentKind::Page);
    assert_eq!(listed.source_url.as_deref(), Some(ADDRESS));
    let json = serde_json::to_value(listed).unwrap();
    assert_eq!(json["kind"], "page");
    assert_eq!(json["source_url"], ADDRESS);
    let text_json = serde_json::to_value(in_folder.iter().find(|d| d.id != id).unwrap()).unwrap();
    assert_eq!(text_json["kind"], "text");
    assert!(text_json.get("source_url").is_none());
    let pages = library.pages(&PagesRequest::default()).unwrap();
    assert_eq!(pages.pages.len(), 1);
    assert_eq!(pages.pages[0].page.id, id);
    assert_eq!(pages.pages[0].summary, summary);
    assert!(!pages.pages[0].trashed);
    // The pages of a folder, and of another one; one in the trash is marked.
    let elsewhere = fx.page_folder("elsewhere", &[("index.html", "<title>Elsewhere</title>")]);
    let other = library
        .add_page(&AddPageRequest {
            folder: Some(Target::path("Other")),
            create_folders: true,
            ..add(&elsewhere)
        })
        .unwrap();
    let in_claude = library
        .pages(&PagesRequest {
            folder: Some(Target::path("Claude Pages")),
        })
        .unwrap();
    assert_eq!(in_claude.pages.len(), 1);
    library
        .trash_document(&TrashDocumentRequest {
            document: other.page.id.into(),
            write: claude(),
        })
        .unwrap();
    let all = library.pages(&PagesRequest::default()).unwrap();
    assert_eq!(all.pages.len(), 2);
    let trashed = all
        .pages
        .iter()
        .find(|p| p.page.id == other.page.id)
        .unwrap();
    assert!(trashed.trashed);
    assert!(trashed.summary.source_url.is_none());
    // `page:<id>` names it as `doc:<id>` does.
    let by_page = read(
        &library,
        Target::parse(&format!("page:{}", id.0)),
        TextFormat::PlainText,
    );
    assert_eq!(by_page.document.id, id);
}

// ----- export ------------------------------------------------------------------------------

#[test]
fn a_page_is_exported_as_a_folder_as_it_was_given_as_its_text_and_as_a_pdf() {
    let fx = fixture();
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = orbitarium(&fx, "Exported.");
    let page = library.add_page(&with_address(&folder, ADDRESS)).unwrap();
    let id = page.page.id;
    let out = fx.dir.path().join("out");
    let export = |request: ExportRequest| library.export(&request);

    // A folder that opens offline: the served files.
    let served = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("Orbitarium"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(served.format, ExportFormat::Html);
    assert_eq!(served.files, 3);
    assert!(served.path.is_absolute());
    assert!(served.warnings.is_empty());
    let index = std::fs::read_to_string(out.join("Orbitarium/index.html")).unwrap();
    assert!(
        index.contains("_external/fonts.example.com/k.css"),
        "{index}"
    );
    assert!(!index.contains("https://fonts.example.com"), "{index}");
    assert_eq!(
        std::fs::read_to_string(out.join("Orbitarium/_external/fonts.example.com/k.css")).unwrap(),
        "body { font-family: serif }"
    );
    // Again into the same folder: refused unless asked.
    let again = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("Orbitarium"),
        format: Some(ExportFormat::Html),
        ..Default::default()
    });
    assert!(matches!(again, Err(Error::FileExists { .. })), "{again:?}");
    export(ExportRequest {
        document: Some(id.into()),
        path: out.join("Orbitarium"),
        overwrite: true,
        ..Default::default()
    })
    .unwrap();
    // The files it was added from, as they were.
    let original = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("original"),
        original: true,
        ..Default::default()
    })
    .unwrap();
    assert_eq!(original.files, 2);
    assert_eq!(
        std::fs::read(out.join("original/index.html")).unwrap(),
        std::fs::read(folder.join("index.html")).unwrap()
    );
    // A dry run writes nothing.
    let tried = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("tried"),
        dry_run: true,
        ..Default::default()
    })
    .unwrap();
    assert!(tried.dry_run && tried.files == 3 && tried.bytes > 0);
    assert!(!out.join("tried").exists());
    // Its text.
    let text = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("page.txt"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(text.format, ExportFormat::PlainText);
    assert!(
        std::fs::read_to_string(out.join("page.txt"))
            .unwrap()
            .contains("Exported.")
    );
    // A PDF, as the checker prints it (the fake prints a fixed page).
    let pdf = export(ExportRequest {
        document: Some(id.into()),
        path: out.join("page.pdf"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(pdf.format, ExportFormat::Pdf);
    let bytes = std::fs::read(out.join("page.pdf")).unwrap();
    assert!(bytes.starts_with(b"%PDF-"));
    assert_eq!(pdf.bytes, bytes.len() as u64);
    let names: Vec<String> = std::fs::read_dir(&out)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.iter().all(|name| !name.contains("partial")),
        "{names:?}"
    );
    assert!(matches!(
        export(ExportRequest {
            document: Some(id.into()),
            path: out.join("page.pdf"),
            ..Default::default()
        }),
        Err(Error::FileExists { .. })
    ));

    // A folder exported as one file leaves its pages out and says so.
    fx.create(Some("Claude Pages"), "# Notes\n\nBeside the page.\n");
    let book = export(ExportRequest {
        folder: Some(Target::path("Claude Pages")),
        path: out.join("pages.md"),
        ..Default::default()
    })
    .unwrap();
    assert_eq!(book.documents.len(), 1);
    assert_eq!(
        book.warnings,
        ["1 page was left out: a page is exported on its own, as a folder."]
    );
    assert!(
        !std::fs::read_to_string(out.join("pages.md"))
            .unwrap()
            .contains("Exported.")
    );
}

// ----- dry runs, limits, the channel -------------------------------------------------------

#[test]
fn a_dry_run_of_a_page_writes_nothing_and_sends_no_bytes() {
    // Without an app.
    let fx = closed_fixture();
    fx.create(None, "# A text\n\nFirst.\n");
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = orbitarium(&fx, "Tried.");
    let before = fx.dump();
    let tried = library
        .add_page(&AddPageRequest {
            write: dry(),
            ..with_address(&folder, ADDRESS)
        })
        .unwrap();
    assert_eq!(tried.outcome, PageOutcome::Added);
    assert!(tried.written.dry_run && !tried.written.changed && tried.written.would_change);
    assert_eq!(tried.page.id.0, 0);
    assert_eq!(tried.page.path, "Claude Pages/Orbitarium");
    assert_eq!(tried.page.kind, DocumentKind::Page);
    assert!(tried.files == 3 && tried.bytes > 0);
    assert_eq!(fx.dump(), before);
    assert!(!fx.images_path().exists(), "no file was stored");
    // In a library that does not exist yet, nothing is made either.
    let empty = closed_fixture();
    let nothing = empty
        .library
        .clone()
        .with_fetcher(Arc::new(web()))
        .add_page(&AddPageRequest {
            write: dry(),
            ..add(&orbitarium(&empty, "Tried."))
        })
        .unwrap();
    assert_eq!(nothing.outcome, PageOutcome::Added);
    assert!(!empty.data_dir().exists());

    // With an app: the dry run is worked out here, and the app is only asked to save.
    let app = super::live::StandIn::open(&fx);
    app.names();
    let tried = library
        .add_page(&AddPageRequest {
            write: dry(),
            ..with_address(&folder, ADDRESS)
        })
        .unwrap();
    assert!(tried.written.dry_run && tried.written.app_is_open && tried.written.app_takes_changes);
    assert_eq!(app.changes(), [] as [&str; 0]);
    assert_eq!(fx.dump(), before);
    drop(app);

    // Carried out by the app, a dry run's page, which has no bytes, is worked out and
    // nothing is stored.
    let mut page = prepared(
        &fx,
        "dry",
        &[("index.html", "<title>Dry</title><p>Dry.</p>")],
    );
    for blob in &mut page.blobs {
        blob.data = Vec::new();
    }
    let request = request_for(page, dry());
    assert!(!live::carries_page_files(&request));
    let mut store = Store::open(fx.library.database_path()).unwrap();
    let executed = live::execute(&mut store, &request);
    let Answer::Done { result } = executed.answer else {
        panic!("{:?}", executed.answer);
    };
    assert_eq!(result["outcome"], "added");
    assert_eq!(result["dry_run"], true);
    assert_eq!(result["page"]["id"], 0);
    assert!(executed.changes.is_empty());
    assert_eq!(fx.dump(), before);
    assert!(!fx.images_path().exists());
}

/// A page prepared here, as the tool sends it.
fn prepared(fx: &Fixture, name: &str, files: &[(&str, &str)]) -> innerword_page::Prepared {
    let folder = fx.page_folder(name, files);
    let input = innerword_page::read_input(&folder, None).unwrap();
    let bundle = innerword_page::bundle(input, &MapFetcher::new()).unwrap();
    innerword_page::prepare(bundle.clone(), &innerword_page::static_check(&bundle)).unwrap()
}

fn request_for(page: innerword_page::Prepared, write: WriteOptions) -> Request {
    Request::new(
        Operation::AddPage(AddPageRequest {
            path: "/the/tool/read/it".into(),
            prepared: Some(PreparedPage::Page(page)),
            write,
            ..Default::default()
        }),
        Duration::from_secs(5),
    )
}

#[test]
fn where_the_page_is_stored_its_limits_and_its_bytes_are_checked_again() {
    let fx = closed_fixture();
    fx.create(None, "# A text\n\nFirst.\n");
    let mut store = Store::open(fx.library.database_path()).unwrap();
    store.set_clock(fx.clock.clone());
    let before = fx.dump();
    let failed = |request: &Request| match live::execute(
        &mut Store::open(fx.library.database_path()).unwrap(),
        request,
    )
    .answer
    {
        Answer::Failed { error } => error.into_error(),
        other => panic!("{other:?}"),
    };

    // Bytes that are not the ones their name says.
    let mut page = prepared(
        &fx,
        "damaged",
        &[("index.html", "<title>Damaged</title><p>x</p>")],
    );
    page.blobs[0].data[0] ^= 1;
    let error = failed(&request_for(page, claude()));
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    assert!(error.to_string().contains("damaged"), "{error}");
    // A file over 32 MB.
    let mut page = prepared(&fx, "large", &[("index.html", "<title>Large</title>")]);
    let large = vec![7u8; 33 * 1024 * 1024];
    page.blobs.push(innerword_page::Blob {
        file: innerword_page::file_id_hex(&large),
        size: large.len() as u64,
        data: large,
        stored: false,
    });
    page.served.push(innerword_page::PathEntry {
        path: "big.bin".into(),
        media_type: "application/octet-stream".into(),
        blob: 1,
    });
    let error = failed(&request_for(page, claude()));
    assert_eq!(error.kind(), ErrorKind::PageIncomplete, "{error}");
    assert!(
        error
            .to_string()
            .contains("big.bin is 33 MB; a file can be 32 MB at most"),
        "{error}"
    );
    // A path that names no file of the page.
    let mut page = prepared(&fx, "broken", &[("index.html", "<title>Broken</title>")]);
    page.served[0].blob = 9;
    let error = failed(&request_for(page, claude()));
    assert_eq!(error.kind(), ErrorKind::Invalid, "{error}");
    // A request that was not prepared.
    let error = failed(&Request::new(
        Operation::AddPage(AddPageRequest {
            path: "/x".into(),
            write: claude(),
            ..Default::default()
        }),
        Duration::from_secs(5),
    ));
    assert_eq!(error.to_string(), "the page was not prepared");
    assert_eq!(fx.dump(), before, "nothing was stored");
    assert!(!fx.images_path().exists());

    // A blob the listener stored that is not there after all: the store refuses the row.
    let mut page = prepared(&fx, "lost", &[("index.html", "<title>Lost</title>")]);
    for blob in &mut page.blobs {
        blob.data.clear();
        blob.stored = true;
    }
    let error = failed(&request_for(page, claude()));
    assert_eq!(error.kind(), ErrorKind::Failed, "{error}");
    assert!(
        error
            .to_string()
            .contains("could not be stored in the library (1 is missing)"),
        "{error}"
    );
    assert!(fx.store().pages().unwrap().is_empty());
    drop(store);
}

#[test]
fn the_listener_stores_the_files_and_the_main_thread_only_writes_the_row() {
    let fx = closed_fixture();
    fx.create(None, "# A text\n\nFirst.\n");
    let images = innerword_store::images_path(&fx.library.database_path());
    let database = fx.library.database_path();
    let seen: Arc<Mutex<Vec<bool>>> = Arc::new(Mutex::new(Vec::new()));
    let noted = seen.clone();
    let app = super::live::StandIn::open_with(
        &fx,
        Box::new(move |store, mut incoming| {
            noted
                .lock()
                .unwrap()
                .push(live::carries_page_files(incoming.request()));
            let mut files = innerword_store::ImageFile::beside(&database);
            if let Err(error) = live::store_page_files(incoming.request_mut(), &mut files) {
                incoming.answer(Answer::failed(&error)).ok();
                return None;
            }
            if let Operation::AddPage(add) = incoming.request().operation()
                && let Some(PreparedPage::Page(page)) = &add.prepared
            {
                assert!(
                    page.blobs
                        .iter()
                        .all(|blob| blob.stored && blob.data.is_empty())
                );
            }
            super::live::as_the_app(store, incoming)
        }),
    );
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let added: PageAdded = library
        .add_page(&add(&orbitarium(&fx, "Stored aside.")))
        .unwrap();
    assert_eq!(added.outcome, PageOutcome::Added);
    assert!(added.written.app_is_open && added.written.changed);
    assert_eq!(seen.lock().unwrap().as_slice(), [true]);
    assert!(images.exists());
    let store = fx.store();
    assert_eq!(store.file_count().unwrap(), 4);
    assert!(
        read(&library, added.page.id, TextFormat::PlainText)
            .content
            .contains("Stored aside.")
    );
    drop(app);
}

#[test]
fn which_requests_carry_page_files() {
    let fx = closed_fixture();
    let page = prepared(&fx, "carried", &[("index.html", "<title>Carried</title>")]);
    let with = |prepared: Option<PreparedPage>, write: WriteOptions| {
        Request::new(
            Operation::AddPage(AddPageRequest {
                prepared,
                write,
                ..Default::default()
            }),
            Duration::from_secs(1),
        )
    };
    let mut empty = page.clone();
    empty.blobs.clear();
    for (request, carries) in [
        (with(Some(PreparedPage::Page(page.clone())), claude()), true),
        (with(Some(PreparedPage::Page(page.clone())), dry()), false),
        (with(Some(PreparedPage::Page(empty)), claude()), false),
        (
            with(
                Some(PreparedPage::Unchanged {
                    page: innerword_store::DocumentId(1),
                    fingerprint: "f".into(),
                }),
                claude(),
            ),
            false,
        ),
        (
            with(
                Some(PreparedPage::Trashed {
                    page: innerword_store::DocumentId(1),
                }),
                claude(),
            ),
            false,
        ),
        (with(None, claude()), false),
        (
            Request::new(Operation::Flush, Duration::from_secs(1)),
            false,
        ),
        (
            Request::new(
                Operation::CreateDocument(CreateDocumentRequest::default()),
                Duration::from_secs(1),
            ),
            false,
        ),
    ] {
        assert_eq!(live::carries_page_files(&request), carries, "{request:?}");
        // A request that carries none is left as it is.
        if !carries {
            let mut left = request.clone();
            let mut files = innerword_store::ImageFile::beside(&fx.library.database_path());
            live::store_page_files(&mut left, &mut files).unwrap();
            assert_eq!(left, request);
        }
    }
    // The bytes of a request travel as base64 and come back the same; `stored` never
    // travels.
    let mut stored = page.clone();
    stored.blobs[0].stored = true;
    let request = with(Some(PreparedPage::Page(stored)), claude());
    let line = serde_json::to_string(&request).unwrap();
    assert!(!line.contains("stored"), "{line}");
    let back: Request = serde_json::from_str(&line).unwrap();
    let Operation::AddPage(add) = back.operation() else {
        panic!()
    };
    let Some(PreparedPage::Page(back)) = &add.prepared else {
        panic!()
    };
    assert!(!back.blobs[0].stored);
    assert_eq!(back.blobs[0].data, page.blobs[0].data);
    // A blob whose bytes are not its name is not stored by the listener.
    let mut damaged = page.clone();
    damaged.blobs[0].data.push(b'!');
    damaged.blobs[0].size += 1;
    let mut request = with(Some(PreparedPage::Page(damaged)), claude());
    let mut files = innerword_store::ImageFile::beside(&fx.library.database_path());
    let error = live::store_page_files(&mut request, &mut files).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Invalid);
    assert!(!fx.images_path().exists(), "nothing was stored");
}

#[test]
fn the_counts_of_a_page_are_those_of_its_text() {
    let fx = fixture();
    let library = fx.library.clone().with_fetcher(Arc::new(web()));
    let folder = fx.page_folder(
        "counted",
        &[(
            "index.html",
            "<title>Counted</title><p>one two three</p><p>four five</p>",
        )],
    );
    let added = library.add_page(&add(&folder)).unwrap();
    assert_eq!(added.page.words, 5);
    let text = read(&library, added.page.id, TextFormat::PlainText).content;
    let document = innerword_core::Fragment::from_plain_text(&text).to_document();
    assert_eq!(added.page.characters, document.stats().characters as u64);
}

// ----- the checker in a child process ------------------------------------------------------

/// A program that stands in for `innerword __page-check` and `__page-pdf`: a shell script
/// that does what `body` says, with `$1` the argument and `$2` the directory.
fn checker_program(fx: &Fixture, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = fx.dir.path().join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

#[test]
fn the_checker_runs_as_a_child_with_its_input_closed_and_its_time_bounded() {
    let fx = closed_fixture();
    let folder = fx.page_folder("child", &[("index.html", "<title>Child</title><p>Hi.</p>")]);
    let input = innerword_page::read_input(&folder, None).unwrap();
    let bundle = innerword_page::bundle(input, &MapFetcher::new()).unwrap();
    let seen = fx.dir.path().join("seen");
    // A child that reads its standard input (end of file at once), says where it was
    // asked to look, finds the request and the files there, and answers one line.
    let report = innerword_page::CheckReport {
        engine: "a stand-in".into(),
        elapsed_ms: 3,
        loads: vec![innerword_page::LoadReport {
            path: "index.html".into(),
            title: "Child".into(),
            text: "Hi.".into(),
            finished: true,
            ..Default::default()
        }],
        failure: None,
    };
    let answer = serde_json::to_string(&report).unwrap();
    let good = checker_program(
        &fx,
        "good",
        &format!(
            "cat > /dev/null\n\
             [ \"$1\" = __page-check ] || exit 3\n\
             [ -f \"$2/request.json\" ] && [ -f \"$2/files/index.html\" ] || exit 4\n\
             echo \"$2\" > '{}'\n\
             echo 'some words before the answer'\n\
             echo '{answer}'",
            seen.display()
        ),
    );
    let checked = innerword_agent::ProcessChecker::with_program(good)
        .check(&bundle)
        .unwrap();
    assert_eq!(checked, report);
    let dir = PathBuf::from(std::fs::read_to_string(&seen).unwrap().trim());
    assert!(
        dir.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("check-"),
        "{}",
        dir.display()
    );
    assert!(
        !dir.exists(),
        "the directory of the check is removed afterwards"
    );
    {
        use std::os::unix::fs::MetadataExt;
        let parent = std::fs::metadata(dir.parent().unwrap()).unwrap();
        assert_eq!(parent.mode() & 0o777, 0o700, "only this user enters it");
    }

    // A child that fails: its last line of standard error says why.
    let failing = checker_program(
        &fx,
        "failing",
        "echo 'starting' >&2\necho 'there is no window server' >&2\nexit 1",
    );
    let error = innerword_agent::ProcessChecker::with_program(failing)
        .check(&bundle)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Failed);
    assert_eq!(
        error.to_string(),
        "the page could not be checked: there is no window server"
    );
    // One that answers nothing that can be read.
    let mumbling = checker_program(&fx, "mumbling", "echo 'not json'");
    let error = innerword_agent::ProcessChecker::with_program(mumbling)
        .check(&bundle)
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("the page could not be checked: its checker answered nothing"),
        "{error}"
    );
    // One that does not finish is killed.
    let hanging = checker_program(&fx, "hanging", "sleep 30");
    let started = std::time::Instant::now();
    let error = innerword_agent::ProcessChecker::with_program(hanging)
        .with_time_limit(Duration::from_millis(300))
        .check(&bundle)
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(
        error.to_string().contains("did not finish within"),
        "{error}"
    );
    // One that is not there.
    let error = innerword_agent::ProcessChecker::with_program(fx.dir.path().join("nothing"))
        .check(&bundle)
        .unwrap_err();
    assert_eq!(error.kind(), ErrorKind::Failed);

    // A PDF: the child writes the file it is given.
    let printer = checker_program(
        &fx,
        "printer",
        "[ \"$1\" = __page-pdf ] || exit 3\n[ -f \"$2/request.json\" ] || exit 4\nprintf '%%PDF-1.4' > \"$3\"",
    );
    let out = fx.dir.path().join("printed.pdf");
    innerword_agent::ProcessChecker::with_program(printer.clone())
        .print(&folder, &out)
        .expect_err("the folder of the page is no served directory");
    let served = fx.dir.path().join("served");
    std::fs::create_dir_all(&served).unwrap();
    std::fs::write(served.join("request.json"), "{}").unwrap();
    innerword_agent::ProcessChecker::with_program(printer)
        .print(&served, &out)
        .unwrap();
    assert_eq!(std::fs::read(&out).unwrap(), b"%PDF-1.4");
    let silent = checker_program(&fx, "silent", "exit 0");
    let error = innerword_agent::ProcessChecker::with_program(silent)
        .print(&served, &fx.dir.path().join("none.pdf"))
        .unwrap_err();
    assert!(error.to_string().contains("wrote no file"), "{error}");
}

/// Not a test of its own: run by the test below in a second process, with
/// `INNER_WORD_PAGE_CHECK=fake` in its environment.
#[test]
fn the_child_answers_with_the_fake_check() {
    if std::env::var_os("INNERWORD_AGENT_TEST_FAKE_CHECK").is_none() {
        return;
    }
    let fx = closed_fixture();
    let folder = fx.page_folder(
        "fake",
        &[("index.html", "<title>Fake</title><p>Faked.</p>")],
    );
    let input = innerword_page::read_input(&folder, None).unwrap();
    let bundle = innerword_page::bundle(input, &MapFetcher::new()).unwrap();
    // A program that is not there is never started.
    let checker = innerword_agent::ProcessChecker::with_program(fx.dir.path().join("not there"));
    let report = checker.check(&bundle).unwrap();
    assert_eq!(report, innerword_page::static_check(&bundle));
    let out = fx.dir.path().join("fake.pdf");
    checker.print(fx.dir.path(), &out).unwrap();
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-1.4"));
    println!("\nchild ok");
}

#[test]
fn a_debug_build_checks_with_the_fake_when_the_environment_says_so() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "suite::pages::the_child_answers_with_the_fake_check",
            "--nocapture",
        ])
        .env("INNERWORD_AGENT_TEST_FAKE_CHECK", "1")
        .env(innerword_agent::FAKE_CHECK_ENV, "fake")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(stdout.contains("child ok"), "{stdout}");
    // Any other value is no fake.
    let fx = closed_fixture();
    let folder = fx.page_folder("real", &[("index.html", "<title>Real</title>")]);
    let input = innerword_page::read_input(&folder, None).unwrap();
    let bundle = innerword_page::bundle(input, &MapFetcher::new()).unwrap();
    if std::env::var(innerword_agent::FAKE_CHECK_ENV).is_err() {
        let error = innerword_agent::ProcessChecker::with_program(fx.dir.path().join("not there"))
            .check(&bundle)
            .unwrap_err();
        assert!(
            error.to_string().contains("could not be started"),
            "{error}"
        );
    }
}
