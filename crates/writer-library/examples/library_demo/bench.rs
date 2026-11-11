//! `LIBRARY_DEMO_BENCH=<documents>`: measures what an autosave costs in a big library,
//! without opening a window.
//!
//! ```bash
//! LIBRARY_DEMO_BENCH=5000 cargo run --release -p writer-library --example library_demo
//! ```
//!
//! The library is a real database file in a temporary directory (so every commit pays for
//! its `fsync`, as in the app), filled with `<documents>` documents of about 300 words in
//! 20 folders, one small document and one of about 1 MB. A [`LibraryPanel`] with every
//! folder open follows the library, as the sidebar does. Each measured save changes the
//! text a little, as typing does, and is followed by the `snapshot_if_stale` call the app
//! makes after every autosave. The times include the delivery of the library's events to
//! the panel.

use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, Application, Entity};
use writer_library::store::{OpenOptions, Store};
use writer_library::{
    DocumentContent, DocumentId, EXPANDED_FOLDERS_SETTING, FolderId, Library, LibraryPanel,
};

pub const ENV: &str = "LIBRARY_DEMO_BENCH";

const ROUNDS: usize = 40;
const WORDS: &[&str] = &[
    "salt",
    "road",
    "harbour",
    "letter",
    "tide",
    "inland",
    "ash",
    "morning",
    "window",
    "paper",
    "ink",
    "river",
    "stone",
    "quiet",
    "market",
    "candle",
    "winter",
    "ledger",
    "mast",
    "rope",
    "gull",
    "storm",
    "bread",
    "copper",
    "lantern",
    "bridge",
    "orchard",
    "widow",
    "sailor",
    "debt",
    "promise",
    "silence",
    "the",
    "a",
    "of",
    "and",
    "to",
    "in",
    "was",
    "her",
    "his",
    "that",
    "it",
    "with",
    "for",
    "as",
    "had",
    "but",
    "not",
    "they",
    "from",
    "she",
    "he",
    "were",
    "said",
    "when",
    "there",
    "would",
    "after",
    "before",
    "under",
    "against",
    "between",
    "through",
    "nothing",
    "always",
    "never",
    "again",
    "almost",
    "slowly",
    "carried",
    "waited",
    "counted",
    "remembered",
];

/// A tiny deterministic generator, so every run saves the same text.
struct Prose(u64);

impl Prose {
    fn word(&mut self) -> &'static str {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        WORDS[(self.0 >> 33) as usize % WORDS.len()]
    }

    /// About `bytes` of prose in paragraphs of about 80 words.
    fn text(&mut self, bytes: usize) -> String {
        let mut text = String::with_capacity(bytes + 16);
        let mut in_paragraph = 0;
        while text.len() < bytes {
            if in_paragraph == 80 {
                text.push('\n');
                in_paragraph = 0;
            } else if in_paragraph > 0 {
                text.push(' ');
            }
            text.push_str(self.word());
            in_paragraph += 1;
        }
        text
    }
}

/// The stored form of `text`: something shaped like the app's document JSON.
fn json(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8 + 64);
    out.push_str("{\"version\":1,\"blocks\":[");
    for (ix, paragraph) in text.split('\n').enumerate() {
        if ix > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"id\":{ix},\"kind\":\"paragraph\",\"text\":\"{paragraph}\"}}"
        ));
    }
    out.push_str("]}");
    out
}

struct Saved {
    json: String,
    text: String,
}

impl Saved {
    fn new(text: String) -> Self {
        Self {
            json: json(&text),
            text,
        }
    }

    fn content<'a>(&'a self, title: &'a str) -> DocumentContent<'a> {
        DocumentContent {
            content: &self.json,
            title,
            plain_text: &self.text,
            word_count: self.text.split_whitespace().count() as u64,
            char_count: self.text.len() as u64,
        }
    }
}

fn summary(label: &str, mut times: Vec<Duration>) {
    times.sort();
    let ms = |d: Duration| d.as_secs_f64() * 1000.;
    let pick = |q: f64| times[((times.len() - 1) as f64 * q).round() as usize];
    println!(
        "{label:<50} median {:>8.3} ms   p90 {:>8.3} ms   max {:>8.3} ms",
        ms(pick(0.5)),
        ms(pick(0.9)),
        ms(times[times.len() - 1]),
    );
}

/// Saves `document` [`ROUNDS`] times, each time with a little more text, and prints what
/// the save and the snapshot check cost.
fn measure(
    label: &str,
    library: &Entity<Library>,
    document: DocumentId,
    title: &str,
    mut text: String,
    prose: &mut Prose,
    cx: &mut App,
) {
    let mut saves = Vec::with_capacity(ROUNDS);
    let mut snapshots = Vec::with_capacity(ROUNDS);
    for _ in 0..ROUNDS {
        text.push(' ');
        text.push_str(prose.word());
        let saved = Saved::new(text.clone());
        let content = saved.content(title);
        let start = Instant::now();
        library
            .update(cx, |library, cx| {
                library.save_document(document, &content, cx)
            })
            .expect("save");
        saves.push(start.elapsed());
        let start = Instant::now();
        library
            .update(cx, |library, cx| {
                library.snapshot_if_stale(document, Duration::from_secs(600), cx)
            })
            .expect("snapshot check");
        snapshots.push(start.elapsed());
    }
    summary(&format!("{label}: save_document"), saves);
    summary(&format!("{label}: snapshot_if_stale"), snapshots);
}

fn bench(documents: usize, cx: &mut App) {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut store = Store::open(dir.path().join("library.db")).expect("open store");
    let mut prose = Prose(7);

    let start = Instant::now();
    let mut folders: Vec<FolderId> = Vec::new();
    for ix in 0..20 {
        let folder = store
            .create_folder(None, &format!("Box {}", ix + 1))
            .expect("create folder");
        folders.push(folder.id);
    }
    for ix in 0..documents {
        let saved = Saved::new(prose.text(1800));
        store
            .create_document(
                Some(folders[ix % folders.len()]),
                &saved.content(&format!("Entry {:05}", ix + 1)),
            )
            .expect("create document");
    }
    let small_text = prose.text(2_000);
    let small = store
        .create_document(
            None,
            &Saved::new(small_text.clone()).content("A short note"),
        )
        .expect("create document")
        .id;
    let big_text = prose.text(950_000);
    let big_saved = Saved::new(big_text.clone());
    let big = store
        .create_document(None, &big_saved.content("The whole book"))
        .expect("create document")
        .id;
    store
        .set_setting(EXPANDED_FOLDERS_SETTING, &folders)
        .expect("expand");
    println!(
        "library: {documents} documents in {} folders, seeded in {:.1} s; the big document is {} bytes of JSON, {} bytes of text",
        folders.len(),
        start.elapsed().as_secs_f64(),
        big_saved.json.len(),
        big_text.len(),
    );

    let library = Library::from_store(store).expect("library");
    let library = cx.new(|_| library);
    let panel = cx.new(|cx| LibraryPanel::new(library.clone(), cx));
    println!("panel: {} rows", panel.read(cx).visible_items().len());

    // The parts, on their own.
    let mut reloads = Vec::new();
    for _ in 0..ROUNDS {
        let start = Instant::now();
        let tree = library.read(cx).store().library_tree().expect("tree");
        reloads.push(start.elapsed());
        drop(tree);
    }
    summary("store.library_tree() (full reload)", reloads);

    measure(
        "2 KB document",
        &library,
        small,
        "A short note",
        small_text.clone(),
        &mut prose,
        cx,
    );

    // The app's background autosave writes through a second connection and then tells the
    // library. What telling it costs on the main thread, both ways.
    let mut background = Store::open_with(
        dir.path().join("library.db"),
        OpenOptions {
            daily_backup: false,
            ..OpenOptions::default()
        },
    )
    .expect("second connection");
    let mut text = small_text;
    let (mut refreshes, mut notes) = (Vec::new(), Vec::new());
    for round in 0..ROUNDS * 2 {
        text.push(' ');
        text.push_str(prose.word());
        // The clock has millisecond steps and these saves are faster than that.
        std::thread::sleep(Duration::from_millis(2));
        let saved = Saved::new(text.clone());
        let meta = background
            .save_document(small, &saved.content("A short note"))
            .expect("background save");
        let start = Instant::now();
        if round % 2 == 0 {
            library
                .update(cx, |library, cx| library.refresh(cx))
                .expect("refresh");
            refreshes.push(start.elapsed());
        } else {
            library
                .update(cx, |library, cx| {
                    library.document_saved_elsewhere(&meta, cx)
                })
                .expect("note");
            notes.push(start.elapsed());
        }
    }
    summary("after an outside save: refresh()", refreshes);
    summary("after an outside save: document_saved_elsewhere()", notes);

    measure(
        "1 MB document",
        &library,
        big,
        "The whole book",
        big_text,
        &mut prose,
        cx,
    );
    drop(panel);
}

/// Runs the measurement when `LIBRARY_DEMO_BENCH` is set. Returns whether it did.
pub fn run_from_env() -> bool {
    let Ok(value) = std::env::var(ENV) else {
        return false;
    };
    let documents = value.parse().unwrap_or(5000);
    Application::headless().run(move |cx: &mut App| {
        bench(documents, cx);
        cx.quit();
    });
    true
}
