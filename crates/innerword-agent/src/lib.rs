//! The headless command layer of Inner Word: everything an agent can do to a library,
//! with the app closed or open.
//!
//! The `innerword` command line tool and its MCP server are front ends of this crate. It works
//! on the library directly (the SQLite file the app uses, through `innerword-store`), on the
//! rich document model (`innerword-core`) and with the exporters and the Markdown importer
//! (`innerword-export`). No GPUI, no window. The app does not have to run; while it does, it
//! is the one that makes the changes ([`live`]).
//!
//! # The shape of the API
//!
//! A [`Library`] names a data directory. It holds nothing open: every operation opens the
//! library, does one thing and closes it again, so a long-lived process (the MCP server)
//! never stands in the way of the app. There is one method per operation; each takes a
//! request struct and returns a result struct, and both serialise to JSON. An operation
//! fails with an [`Error`] whose [`ErrorKind`] tells the kinds of failure apart
//! ([`Error::exit_code`] for a command line, [`Error::report`] for JSON).
//!
//! | | Operations |
//! |---|---|
//! | Look around | [`status`](Library::status), [`tree`](Library::tree), [`search`](Library::search), [`recent`](Library::recent), [`info`](Library::info), [`folder_info`](Library::folder_info), [`resolve`](Library::resolve), [`resolve_document`](Library::resolve_document), [`resolve_folder`](Library::resolve_folder) |
//! | Read | [`read`](Library::read) (Markdown, plain text or the native JSON; a section; with comments), [`find`](Library::find) |
//! | Write text | [`create_document`](Library::create_document), [`replace_document`](Library::replace_document), [`edit`](Library::edit), [`insert`](Library::insert) (append, at the start, at a heading), [`replace_section`](Library::replace_section), [`add_image`](Library::add_image) |
//! | Documents | [`rename_document`](Library::rename_document), [`move_document`](Library::move_document), [`duplicate_document`](Library::duplicate_document), [`trash_document`](Library::trash_document), [`restore_document`](Library::restore_document), [`delete_document`](Library::delete_document) |
//! | Folders | [`create_folder`](Library::create_folder), [`rename_folder`](Library::rename_folder), [`move_folder`](Library::move_folder), [`trash_folder`](Library::trash_folder), [`restore_folder`](Library::restore_folder), [`delete_folder`](Library::delete_folder) |
//! | Trash | [`trash`](Library::trash), [`empty_trash`](Library::empty_trash) |
//! | Comments | [`comments`](Library::comments), [`add_comment`](Library::add_comment), [`reply`](Library::reply), [`resolve_comment`](Library::resolve_comment), [`reopen_comment`](Library::reopen_comment), [`delete_comment`](Library::delete_comment) |
//! | History | [`versions`](Library::versions), [`read_version`](Library::read_version), [`restore_version`](Library::restore_version) |
//! | Export | [`export`](Library::export) |
//! | Pages | [`add_page`](Library::add_page), [`pages`](Library::pages) (and every read, item and history operation above works on a page) |
//!
//! # The rules
//!
//! **Reading always works; while the app is open, a change is carried out by the app.**
//! The app keeps the open document in memory, with text that is not saved yet, and its own
//! picture of the library: anything written underneath it would be lost or would overwrite
//! the writer's work. The app holds the lock of its data directory while it runs
//! (`innerword_store::instance_lock`). An operation that writes takes that lock for its
//! duration and writes; when the app has it, the operation is handed to the app, which
//! carries it out between two key presses, shows it in its window and answers with the
//! result the direct write would have given ([`live`]). The front ends know nothing of
//! this. When the app does not make the change (its owner switched that off, it is a
//! version before 0.6.0, it does not answer) the operation fails with
//! [`Error::AppIsOpen`], whose [`Refusal`] says which it was and whether anything may
//! have been changed. Reading opens the database read-only and takes no lock; the app is
//! asked first to save what is not saved yet, so that the text read is the text on
//! screen.
//!
//! **Nothing an agent does is final.** Before the text of a document is changed, or a
//! comment deleted, a version of the document as it was is kept and labelled ("Before an
//! edit by Claude"); [`ContentChanged::snapshot`] names it and
//! [`Library::restore_version`] brings it back. The label lives beside the version in the
//! library (`innerword_store::Store::snapshot_labels`), where the app can show it. Deleting
//! means the trash. Deleting for good exists
//! ([`Library::delete_document`], [`Library::delete_folder`], [`Library::empty_trash`]) and
//! is refused with [`Error::ConfirmationRequired`] unless the request says `permanently`.
//!
//! **Every write can be tried first.** With [`WriteOptions::dry_run`] an operation works
//! out and reports exactly what it would do, and changes nothing. A dry run only reads
//! ([`Written::app_is_open`] and [`Written::app_takes_changes`] say who would carry the
//! real run out, and whether it would be refused; in the result of a real run they say
//! who did).
//!
//! **Documents are written the way the app writes them.** The stored title follows the
//! text (first heading, else first line, else "Untitled") unless somebody chose one, the
//! plain text and the counts are derived by the same functions, and a chosen title is
//! flagged the way a rename in the app flags it. The app, the search index and the library
//! tree see nothing unusual in a document this crate wrote.
//!
//! **Text goes in as Markdown and is changed in place.** [`Library::edit`] replaces an
//! exact piece of text in the rich document, so the formatting and the comments around it
//! stay; [`Library::replace_document`] brings a document to a new text with as few changes
//! as possible, so comments stay wherever their text does.
//!
//! **An image is a line of Markdown, and stays the image it is.** A document can hold
//! images: blocks that name stored pictures. Read as Markdown, an image is the line
//! `![description](image:<id>)`, and [`DocumentText::images`] lists what the line does not
//! say (how wide the image is drawn, where it stands on its line). Every operation keeps
//! the images it does not change, with their size: a text that is written back keeps an
//! image where its line stays and moves it where its line moves, [`Library::edit`] refuses
//! an old text that runs across an image unless the text names it, and a text that leaves
//! the line out removes the image, which [`ContentChanged::images_removed`] counts (the
//! version kept before the change has it).
//!
//! **A file an agent names is read by whoever was asked.** [`Library::add_image`] adds an
//! image from a file, and the Markdown of every operation that takes text may name one
//! (`![what it shows](/Users/ada/harbour.jpg)`). The method of [`Library`] reads the file
//! in the calling process, makes of it what the library stores (`innerword_image::prepare`)
//! and hands the bytes on with the request ([`AttachedImage`]); the app, when it is the
//! one that carries the request out, opens no path. A file that cannot be used is a
//! warning in the result and the image stays a link (for `add_image`: an error).
//!
//! **A page is kept as it was made.** A library item can be a page: a web page (an HTML
//! file or a folder, with its styles, scripts, fonts, pictures and sounds) kept whole and
//! offline ([`Library::add_page`]). Everything it loads from the web is fetched once when
//! it is added, and it is checked, with the network blocked, to be whole; a page that
//! needs something that cannot be kept is not added at all
//! ([`Error::PageIncomplete`]). The reading, the fetching and the check happen in the
//! process that was asked, by a [`PageChecker`] it was given
//! ([`Library::with_page_checker`]; the `innerword` tool runs itself as a child for it), and
//! the page travels with the request ([`PreparedPage`]). A page cannot be edited: an
//! operation on the text of a document fails on a page with [`Error::WrongKind`]. Its
//! text (what it showed) is what a read, a search and a find see; with a source address
//! it is the page at that address, added again unchanged, updated (the page as it was
//! kept as a version) or left in the trash ([`PageOutcome`]).
//!
//! # Naming things
//!
//! A [`Target`] names a document or a folder by id (`12`, `doc:12`, `folder:3`) or by its
//! path in the library (`The Salt Road/Part one/Chapter 1`). A name that fits several
//! items fails with [`Error::Ambiguous`], which lists the candidates with their ids. Every
//! result carries ids, so the next request can be exact.
//!
//! # Example
//!
//! ```
//! use innerword_agent::{
//!     AddCommentRequest, CreateDocumentRequest, EditRequest, Library, ReadRequest, Target,
//!     WriteOptions,
//! };
//!
//! # let dir = tempfile::tempdir().unwrap();
//! // The user's library is `Library::default_location()`; anything that is tried out or
//! // tested names a directory of its own.
//! let library = Library::at(dir.path());
//!
//! let created = library
//!     .create_document(&CreateDocumentRequest {
//!         markdown: "# Harbour\n\nThe *Maren* left at dawn.\n".into(),
//!         folder: Some(Target::path("The Salt Road/Part one")),
//!         create_folders: true,
//!         write: WriteOptions::by("Claude"),
//!         ..Default::default()
//!     })
//!     .unwrap();
//! assert_eq!(created.document.path, "The Salt Road/Part one/Harbour");
//!
//! // By path or by id; the old text has to be there exactly once.
//! let edited = library
//!     .edit(&EditRequest {
//!         document: Target::path("Part one/Harbour"),
//!         old_text: "at dawn".into(),
//!         new_text: "before dawn".into(),
//!         write: WriteOptions::by("Claude"),
//!         ..Default::default()
//!     })
//!     .unwrap();
//! assert_eq!(edited.replacements, 1);
//! assert!(edited.snapshot.is_some(), "the text as it was is kept as a version");
//!
//! // Review instead of rewriting: a comment on an exact quote.
//! library
//!     .add_comment(&AddCommentRequest {
//!         document: created.document.id.into(),
//!         quote: "Maren".into(),
//!         body: "Is this the same ship as in chapter 3?".into(),
//!         write: WriteOptions::by("Claude"),
//!         ..Default::default()
//!     })
//!     .unwrap();
//!
//! let text = library
//!     .read(&ReadRequest { document: created.document.id.into(), ..Default::default() })
//!     .unwrap();
//! assert_eq!(text.content, "# Harbour\n\nThe *Maren* left before dawn.\n");
//! ```

#![warn(missing_docs)]

mod attach;
mod browse;
mod checker;
mod comments;
mod content;
mod diff;
mod error;
mod export;
mod items;
mod library;
pub mod live;
mod markdown_in;
mod matching;
mod operations;
mod pages;
mod resolve;
mod rewrite;
mod sections;
mod types;
mod versions;
mod write_text;

pub use checker::{
    CHECK_ARGUMENT, FAKE_CHECK_ENV, PDF_ARGUMENT, PageChecker, ProcessChecker, StaticChecker,
};
pub use error::{
    Candidate, Error, ErrorKind, ErrorReport, ItemKind, Occurrence, PAGE_IS_NOT_A_TEXT_FILE,
    Refusal, Result,
};
pub use library::{DEFAULT_AUTHOR, FETCH_LOCAL_ENV, Library};
pub use types::*;
pub use innerword_store::{Clock, DATA_DIR_ENV};
