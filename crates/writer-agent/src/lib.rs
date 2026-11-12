//! The headless command layer of The Writer: everything an agent can do to a library,
//! without the app.
//!
//! The `writer` command line tool and its MCP server are front ends of this crate. It works
//! on the library directly (the SQLite file the app uses, through `writer-store`), on the
//! rich document model (`writer-core`) and with the exporters and the Markdown importer
//! (`writer-export`). No GPUI, no window, no running app.
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
//! | Write text | [`create_document`](Library::create_document), [`replace_document`](Library::replace_document), [`edit`](Library::edit), [`insert`](Library::insert) (append, at the start, at a heading), [`replace_section`](Library::replace_section) |
//! | Documents | [`rename_document`](Library::rename_document), [`move_document`](Library::move_document), [`duplicate_document`](Library::duplicate_document), [`trash_document`](Library::trash_document), [`restore_document`](Library::restore_document), [`delete_document`](Library::delete_document) |
//! | Folders | [`create_folder`](Library::create_folder), [`rename_folder`](Library::rename_folder), [`move_folder`](Library::move_folder), [`trash_folder`](Library::trash_folder), [`restore_folder`](Library::restore_folder), [`delete_folder`](Library::delete_folder) |
//! | Trash | [`trash`](Library::trash), [`empty_trash`](Library::empty_trash) |
//! | Comments | [`comments`](Library::comments), [`add_comment`](Library::add_comment), [`reply`](Library::reply), [`resolve_comment`](Library::resolve_comment), [`reopen_comment`](Library::reopen_comment), [`delete_comment`](Library::delete_comment) |
//! | History | [`versions`](Library::versions), [`read_version`](Library::read_version), [`restore_version`](Library::restore_version) |
//! | Export | [`export`](Library::export) |
//!
//! # The rules
//!
//! **Reading always works; writing needs the app to be closed.** The app keeps the open
//! document in memory and saves all of it, so anything written behind its back would be
//! lost or would overwrite the writer's work. The app holds the lock of its data directory
//! while it runs (`writer_store::instance_lock`); every operation that writes takes that
//! lock for its duration and fails with [`Error::AppIsOpen`] when the app has it. Nothing
//! is written in that case. Reading opens the database read-only and takes no lock.
//!
//! **Nothing an agent does is final.** Before the text or the comments of a document are
//! changed, a version of the document as it was is kept and labelled ("Before an edit by
//! Claude"); [`ContentChanged::snapshot`] names it and [`Library::restore_version`] brings
//! it back. Deleting means the trash. Deleting for good exists
//! ([`Library::delete_document`], [`Library::delete_folder`], [`Library::empty_trash`]) and
//! is refused with [`Error::ConfirmationRequired`] unless the request says `permanently`.
//!
//! **Every write can be tried first.** With [`WriteOptions::dry_run`] an operation works
//! out and reports exactly what it would do, and changes nothing. A dry run only reads, so
//! it also works while the app is open ([`Written::app_is_open`] then says that the real
//! run would be refused).
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
//! use writer_agent::{
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

mod browse;
mod comments;
mod content;
mod diff;
mod error;
mod export;
mod items;
mod library;
mod markdown_in;
mod matching;
mod resolve;
mod sections;
mod types;
mod versions;
mod write_text;

pub use error::{Candidate, Error, ErrorKind, ErrorReport, ItemKind, Occurrence, Result};
pub use library::{DEFAULT_AUTHOR, Library};
pub use types::*;
pub use writer_store::{Clock, DATA_DIR_ENV};
