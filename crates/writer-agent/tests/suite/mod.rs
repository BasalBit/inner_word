//! What the tests share: a library in a temporary directory.

mod browse;
mod comments;
mod items;
mod rules;
mod text;
mod versions_export;

use std::path::PathBuf;

use writer_agent::{
    Clock, CreateDocumentRequest, DocumentSummary, Library, ReadRequest, Target, TextFormat,
    WriteOptions,
};
use writer_core::Document;
use writer_store::{DocumentId, ReadOnlyLibrary, Store};

/// The time every fixture starts at.
pub const START: i64 = 1_800_000_000_000;

/// A library in a temporary directory, with a clock the test moves.
pub struct Fixture {
    /// The directory; everything in it goes when the fixture does.
    pub dir: tempfile::TempDir,
    /// The library in `dir/data`.
    pub library: Library,
    /// The clock of the library.
    pub clock: Clock,
}

/// A fixture whose library does not exist yet.
pub fn fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let clock = Clock::manual(START);
    let data = dir.path().join("data");
    let library = Library::at(&data).with_clock(clock.clone());
    // The whole point: never the user's library.
    assert!(library.data_dir().starts_with(dir.path()));
    assert!(
        !library
            .data_dir()
            .to_string_lossy()
            .contains("Application Support")
    );
    Fixture {
        dir,
        library,
        clock,
    }
}

/// The options of a real write by "Claude".
pub fn claude() -> WriteOptions {
    WriteOptions::by("Claude")
}

impl Fixture {
    /// The data directory.
    pub fn data_dir(&self) -> PathBuf {
        self.library.data_dir().to_path_buf()
    }

    /// Creates a document from Markdown in `folder` (created when missing).
    pub fn create(&self, folder: Option<&str>, markdown: &str) -> DocumentSummary {
        self.clock.advance_ms(1000);
        self.library
            .create_document(&CreateDocumentRequest {
                markdown: markdown.to_string(),
                folder: folder.map(Target::path),
                create_folders: true,
                write: claude(),
                ..Default::default()
            })
            .expect("the document is created")
            .document
    }

    /// The Markdown of a document.
    pub fn markdown(&self, id: DocumentId) -> String {
        self.read(id, TextFormat::Markdown)
    }

    /// The text of a document in `format`.
    pub fn read(&self, id: DocumentId, format: TextFormat) -> String {
        self.library
            .read(&ReadRequest {
                document: id.into(),
                format,
                ..Default::default()
            })
            .expect("the document is read")
            .content
    }

    /// The library's store, opened the way a reader opens it.
    pub fn store(&self) -> Store {
        match Store::open_read_only(self.library.database_path()).expect("the library opens") {
            ReadOnlyLibrary::Open(store) => *store,
            ReadOnlyLibrary::NeedsUpgrade { .. } => panic!("a fixture's library is current"),
        }
    }

    /// The stored document, as `writer-core` reads it.
    pub fn document(&self, id: DocumentId) -> Document {
        let stored = self.store().document(id).expect("the document exists");
        Document::from_json(&stored.content).expect("stored content is a document")
    }

    /// What a dry run, a refused write and a read must leave as it is: the bytes of the
    /// database file and of its write-ahead log (a reader may leave an empty log file
    /// behind, which is no change). Compared through [`Bytes`] so that a difference does
    /// not print the whole file.
    pub fn bytes(&self) -> Bytes {
        let path = self.library.database_path();
        let mut wal = path.clone().into_os_string();
        wal.push("-wal");
        Bytes(
            std::fs::read(&path).expect("the database file"),
            std::fs::read(wal).unwrap_or_default(),
        )
    }
}

/// The content of a library's files.
#[derive(PartialEq, Eq)]
pub struct Bytes(Vec<u8>, Vec<u8>);

impl std::fmt::Debug for Bytes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.0.hash(&mut hasher);
        self.1.hash(&mut hasher);
        write!(
            f,
            "library.db of {} bytes, log of {} bytes, fingerprint {:016x}",
            self.0.len(),
            self.1.len(),
            hasher.finish()
        )
    }
}
