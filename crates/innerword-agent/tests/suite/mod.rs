//! What the tests share: a library in a temporary directory.

mod browse;
mod comments;
mod images;
mod items;
pub mod live;
mod pages;
pub mod rules;
mod text;
mod versions_export;

use std::path::PathBuf;

use innerword_agent::{
    Clock, CreateDocumentRequest, DocumentSummary, Library, ReadRequest, Target, TextFormat,
    WriteOptions,
};
use innerword_core::{Alignment, Document, Image};
use innerword_store::{DocumentId, ReadOnlyLibrary, Store};

/// The time every fixture starts at.
pub const START: i64 = 1_800_000_000_000;

/// The variable that makes every [`fixture`] a library an app has open: the run of the
/// whole suite in which each change is carried out by the app (`live.rs`,
/// `the_whole_suite_passes_while_an_app_has_the_library_open`).
pub const THROUGH_THE_APP: &str = "INNERWORD_AGENT_TESTS_THROUGH_THE_APP";

/// Whether this is the run of the suite in which an app has every library open.
pub fn through_the_app() -> bool {
    std::env::var_os(THROUGH_THE_APP).is_some()
}

/// A library in a temporary directory, with a clock the test moves.
pub struct Fixture {
    /// What stands in for the app while it has the library open (it goes first, before
    /// the directory does).
    pub app: Option<live::StandIn>,
    /// The directory; everything in it goes when the fixture does.
    pub dir: tempfile::TempDir,
    /// The library in `dir/data`.
    pub library: Library,
    /// The clock of the library.
    pub clock: Clock,
}

/// A fixture whose library does not exist yet. In the run of the suite in which an app has
/// every library open ([`through_the_app`]) the library exists, as it does once the app
/// was started, and an app has it open.
pub fn fixture() -> Fixture {
    let mut fx = closed_fixture();
    if through_the_app() {
        fx.app = Some(live::StandIn::open(&fx));
    }
    fx
}

/// A fixture whose library does not exist yet and that no app has open, in either run:
/// for the tests that are about just that.
pub fn closed_fixture() -> Fixture {
    let dir = tempfile::tempdir().expect("a temporary directory");
    let clock = Clock::manual(START);
    let data = dir.path().join("data");
    // Pages are checked by the fake check, and nothing is fetched from the network: an
    // address that a test does not answer is not there ("HTTP 404").
    let library = Library::at(&data)
        .with_clock(clock.clone())
        .with_page_checker(std::sync::Arc::new(innerword_agent::StaticChecker))
        .with_fetcher(std::sync::Arc::new(innerword_page::MapFetcher::new()));
    // The whole point: never the user's library.
    assert!(library.data_dir().starts_with(dir.path()));
    assert!(
        !library
            .data_dir()
            .to_string_lossy()
            .contains("Application Support")
    );
    Fixture {
        app: None,
        dir,
        library,
        clock,
    }
}

/// The bytes of a PNG file whose pixel data is stored without compression: `width` ×
/// `height` pixels in one colour, three bytes a pixel. For a picture file of a size the
/// test chooses (the pictures of `innerword_image::fixtures` are a few hundred bytes
/// whatever their pixels).
pub fn stored_png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    fn crc32(bytes: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for (index, entry) in table.iter_mut().enumerate() {
            let mut crc = index as u32;
            for _ in 0..8 {
                crc = if crc & 1 == 1 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
            *entry = crc;
        }
        !bytes.iter().fold(u32::MAX, |crc, byte| {
            table[((crc ^ u32::from(*byte)) & 0xFF) as usize] ^ (crc >> 8)
        })
    }
    fn chunk(kind: &[u8; 4], data: &[u8], out: &mut Vec<u8>) {
        out.extend((data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend(data);
        out.extend(&body);
        out.extend(crc32(&body).to_be_bytes());
    }
    // Every row: the filter "none", then its pixels.
    let mut row = vec![0u8];
    for _ in 0..width {
        row.extend(rgb);
    }
    let rows = row.repeat(height as usize);
    // A zlib stream of stored blocks, and the checksum of what they hold.
    let mut packed = vec![0x78, 0x01];
    let mut blocks = rows.chunks(65_535).peekable();
    while let Some(block) = blocks.next() {
        packed.push(u8::from(blocks.peek().is_none()));
        packed.extend((block.len() as u16).to_le_bytes());
        packed.extend((!(block.len() as u16)).to_le_bytes());
        packed.extend(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for block in rows.chunks(5_552) {
        for byte in block {
            a += u32::from(*byte);
            b += a;
        }
        a %= 65_521;
        b %= 65_521;
    }
    packed.extend(((b << 16) | a).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend(width.to_be_bytes());
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, 0]);
    chunk(b"IHDR", &header, &mut out);
    chunk(b"IDAT", &packed, &mut out);
    chunk(b"IEND", &[], &mut out);
    out
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

    /// The stored document, as `innerword-core` reads it.
    pub fn document(&self, id: DocumentId) -> Document {
        let stored = self.store().document(id).expect("the document exists");
        Document::from_json(&stored.content).expect("stored content is a document")
    }

    /// A small picture file in the fixture's directory (outside the library): a PNG of
    /// `size` pixels in one colour. The same size and colour are the same bytes, and so
    /// the same image to the library.
    pub fn picture(&self, name: &str, size: (u32, u32), rgb: [u8; 3]) -> PathBuf {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("the folder of the picture");
        }
        std::fs::write(&path, innerword_image::fixtures::png(size.0, size.1, rgb))
            .expect("the picture is written");
        path
    }

    /// A folder of a web page in the fixture's directory (outside the library), with
    /// these files (path in the folder, content). Files that are there already are
    /// written over.
    pub fn page_folder(&self, name: &str, files: &[(&str, &str)]) -> PathBuf {
        let folder = self.dir.path().join(name);
        for (path, content) in files {
            let path = folder.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).expect("the folder of the page");
            std::fs::write(&path, content).expect("a file of the page");
        }
        folder
    }

    /// The images of a stored document, in order: the index of the block, the image and
    /// where it stands on its line.
    pub fn images(&self, id: DocumentId) -> Vec<(usize, Image, Alignment)> {
        let document = self.document(id);
        document
            .blocks()
            .iter()
            .enumerate()
            .filter_map(|(index, block)| {
                block
                    .as_image()
                    .map(|image| (index, image.clone(), block.alignment()))
            })
            .collect()
    }

    /// Where the pictures of the library are stored.
    pub fn images_path(&self) -> PathBuf {
        innerword_store::images_path(&self.library.database_path())
    }

    /// Everything the library holds, row by row: every table of the database with its
    /// rows in the order of their keys, and after them the stored pictures (their rows,
    /// with a fingerprint of the bytes of each). Two libraries with the same dump are
    /// the same library to the app and to the tool.
    pub fn dump(&self) -> String {
        let mut out = dump_of(&self.library.database_path(), "");
        let images = self.images_path();
        if images.exists() {
            out.push_str(&dump_of(&images, "images.db "));
        }
        out
    }
}

/// The tables of one database file with their rows, each table under `prefix` and its
/// name.
fn dump_of(path: &std::path::Path, prefix: &str) -> String {
    {
        use std::fmt::Write;
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .expect("the database opens");
        let tables: Vec<String> = conn
            .prepare(
                "SELECT name FROM sqlite_master WHERE type = 'table' \
                 AND name NOT LIKE 'sqlite_%' ORDER BY name",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|name| name.unwrap())
            .collect();
        let mut out = String::new();
        for table in tables {
            // The search index is built from the documents by triggers; its own tables
            // hold the same words in segments that depend on when they were merged.
            if table.starts_with("documents_fts") {
                continue;
            }
            writeln!(out, "== {prefix}{table}").unwrap();
            let mut statement = match conn.prepare(&format!("SELECT * FROM \"{table}\" ORDER BY 1"))
            {
                Ok(statement) => statement,
                Err(_) => continue,
            };
            let columns = statement.column_count();
            let mut rows = statement.query([]).unwrap();
            while let Some(row) = rows.next().unwrap() {
                let cells: Vec<String> = (0..columns)
                    .map(|column| match row.get_ref(column).unwrap() {
                        rusqlite::types::ValueRef::Null => "null".to_string(),
                        rusqlite::types::ValueRef::Integer(value) => value.to_string(),
                        rusqlite::types::ValueRef::Real(value) => value.to_string(),
                        rusqlite::types::ValueRef::Text(text) => {
                            format!("{:?}", String::from_utf8_lossy(text))
                        }
                        rusqlite::types::ValueRef::Blob(bytes) => {
                            use std::hash::{Hash, Hasher};
                            let mut hasher = std::collections::hash_map::DefaultHasher::new();
                            bytes.hash(&mut hasher);
                            format!("{} bytes {:016x}", bytes.len(), hasher.finish())
                        }
                    })
                    .collect();
                writeln!(out, "{}", cells.join(" | ")).unwrap();
            }
        }
        out
    }
}

impl Fixture {
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
