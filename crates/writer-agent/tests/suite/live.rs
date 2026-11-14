//! Changes while the app is open: the channel, with a thread standing in for the app.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use writer_agent::live::{
    self, Answer, CHANGES_SETTING, Change, Closer, Incoming, Listener, Operation, PROTOCOL_VERSION,
    Request,
};
use writer_agent::{
    AddCommentRequest, AddImageRequest, AddPageRequest, CommentRequest, CreateDocumentRequest,
    CreateFolderRequest, DeleteCommentRequest, DeleteDocumentRequest, DeleteFolderRequest,
    DuplicateDocumentRequest, EditRequest, EmptyTrashRequest, Error, ErrorKind, ExportRequest,
    FindRequest, FolderInfoRequest, InfoRequest, InsertPosition, InsertRequest, Library,
    ListCommentsRequest, ListVersionsRequest, MoveDocumentRequest, MoveFolderRequest, Placement,
    ReadRequest, ReadVersionRequest, RecentRequest, Refusal, RenameDocumentRequest,
    RenameFolderRequest, ReplaceDocumentRequest, ReplaceSectionRequest, ReplyRequest,
    RestoreDocumentRequest, RestoreFolderRequest, RestoreVersionRequest, SearchRequest, Target,
    TrashDocumentRequest, TrashFolderRequest, TreeRequest, WriteOptions,
};
use writer_store::instance_lock::{self, InstanceLock};
use writer_store::{DocumentId, Store};

use super::rules::{dry, every_write, seed};
use super::{Fixture, THROUGH_THE_APP, claude, closed_fixture, through_the_app};

// ----- the stand-in for the app ------------------------------------------------------------

/// One request the stand-in carried out.
#[derive(Debug)]
pub struct Served {
    /// What was asked.
    pub request: Request,
    /// What it changed in the library.
    pub changes: Vec<Change>,
    /// What went back.
    pub answer: Answer,
}

/// What the stand-in does with a request: it has the app's own connection to the library
/// and the request with the connection its answer goes back on.
pub type Serve = Box<dyn FnMut(&mut Store, Incoming) -> Option<Served> + Send>;

/// What the app does with a request (after it saved what was not saved, which a stand-in
/// has nothing of): begin, carry out, answer.
pub fn as_the_app(store: &mut Store, incoming: Incoming) -> Option<Served> {
    if !incoming.begin() {
        // The tool stopped waiting and was told that nothing was changed.
        return None;
    }
    let executed = live::execute(store, incoming.request());
    let served = Served {
        request: incoming.request().clone(),
        changes: executed.changes,
        answer: executed.answer.clone(),
    };
    incoming.answer(executed.answer).ok();
    Some(served)
}

/// A thread that does what the app does with the channel: it holds the lock of the data
/// directory, listens, and carries every request out on a connection of its own, one
/// after the other.
pub struct StandIn {
    closer: Closer,
    thread: Option<JoinHandle<()>>,
    /// Every request that was carried out, in order. (Look through [`StandIn::taken`]:
    /// a request is told here a moment after its answer went out.)
    pub served: mpsc::Receiver<Served>,
    /// The socket it listens on.
    pub socket: PathBuf,
    lock: HeldLock,
    /// How many requests arrived, and how many of them are done with: answered, and
    /// told to `served`.
    arrived: Arc<AtomicUsize>,
    done: Arc<AtomicUsize>,
}

/// The lock of a library, where whoever stands in for the app can let go of it.
pub type HeldLock = Arc<Mutex<Option<InstanceLock>>>;

impl StandIn {
    /// Starts one on the library of `fx` (which it creates when there is none, as the
    /// app does when it starts), doing with every request what the app does.
    pub fn open(fx: &Fixture) -> StandIn {
        StandIn::open_with(fx, Box::new(as_the_app))
    }

    /// Starts one that does `serve` with every request.
    pub fn open_with(fx: &Fixture, mut serve: Serve) -> StandIn {
        let lock = instance_lock::acquire(&fx.data_dir())
            .unwrap()
            .lock()
            .expect("nobody else has the library");
        let mut store = Store::open(fx.library.database_path()).unwrap();
        store.set_clock(fx.clock.clone());
        let listener = Listener::bind(&fx.data_dir()).unwrap();
        let closer = listener.closer();
        let socket = listener.path().to_path_buf();
        let (tell, served) = mpsc::channel();
        let (arrived, done) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let (counted, finished) = (arrived.clone(), done.clone());
        let thread = std::thread::spawn(move || {
            while let Some(incoming) = listener.accept() {
                counted.fetch_add(1, Ordering::SeqCst);
                if let Some(served) = serve(&mut store, incoming) {
                    tell.send(served).ok();
                }
                finished.fetch_add(1, Ordering::SeqCst);
            }
        });
        StandIn {
            closer,
            thread: Some(thread),
            served,
            socket,
            lock: Arc::new(Mutex::new(Some(lock))),
            arrived,
            done,
        }
    }

    /// Every request that was carried out since the last look. The tool has its answer
    /// a moment before the stand-in has told what it did: a look right after an answer
    /// waits that moment (and not for a request the stand-in is keeping on purpose
    /// for longer than a test waits).
    pub fn taken(&self) -> Vec<Served> {
        let started = Instant::now();
        while self.done.load(Ordering::SeqCst) < self.arrived.load(Ordering::SeqCst)
            && started.elapsed() < Duration::from_secs(2)
        {
            std::thread::sleep(Duration::from_millis(1));
        }
        self.served.try_iter().collect()
    }

    /// The names of the requests served since the last look.
    pub fn names(&self) -> Vec<&'static str> {
        self.taken()
            .iter()
            .map(|served| served.request.name())
            .collect()
    }

    /// The names of the changes served since the last look: without the requests to
    /// save that the reads send.
    pub fn changes(&self) -> Vec<&'static str> {
        self.names()
            .into_iter()
            .filter(|name| *name != "flush")
            .collect()
    }

    /// A handle on the lock, for a stand-in that lets go of it by itself.
    pub fn lock(&self) -> HeldLock {
        self.lock.clone()
    }

    /// The app is killed: the system takes its lock back. The lock file still names its
    /// channel, and the socket is still there (here somebody even still listens on it,
    /// which a crash would not leave behind; nobody may come to find out).
    pub fn crash(&self) {
        self.lock.lock().unwrap().take();
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.closer.close();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
        self.lock.lock().unwrap().take();
    }
}

/// Switches "Agents can make changes while The Writer is open" in the library of `fx`,
/// as the app does when its owner presses the switch.
fn set_switch(fx: &Fixture, on: bool) {
    let mut store = Store::open(fx.library.database_path()).unwrap();
    store.set_setting(CHANGES_SETTING, &on).unwrap();
}

/// A directory with a path short enough for a socket.
fn socket_dir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("tw")
        .tempdir_in(std::env::temp_dir())
        .unwrap()
}

/// Something that holds the lock of a library as the app and answers on a socket of its
/// own whatever the test says: for answers the real listener never gives.
struct RawApp {
    _lock: InstanceLock,
    _dir: tempfile::TempDir,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    socket: PathBuf,
    /// The request lines that arrived.
    requests: mpsc::Receiver<String>,
}

impl RawApp {
    /// Holds the lock of `fx` and answers every request with what `answer` returns for
    /// its line: the bytes to send before the connection is closed.
    fn open(fx: &Fixture, answer: impl Fn(&str) -> Vec<u8> + Send + 'static) -> RawApp {
        let lock = instance_lock::acquire(&fx.data_dir())
            .unwrap()
            .lock()
            .expect("nobody else has the library");
        let dir = socket_dir();
        let socket = dir.path().join("app.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        instance_lock::announce_channel(&fx.data_dir(), Some(&socket)).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (tell, requests) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(mut stream) = stream else { return };
                let mut line = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut line)
                    .ok();
                tell.send(line.clone()).ok();
                stream.write_all(&answer(&line)).ok();
            }
        });
        RawApp {
            _lock: lock,
            _dir: dir,
            thread: Some(thread),
            stop,
            socket,
            requests,
        }
    }
}

impl Drop for RawApp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        drop(UnixStream::connect(&self.socket));
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

/// Makes the lock file of `fx` as old as that of an app that was started long ago (the
/// tool waits a few seconds for an app that has only just taken the lock to open its
/// channel).
pub fn age_the_lock(fx: &Fixture) {
    let an_hour_ago = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(fx.data_dir().join(instance_lock::LOCK_FILE_NAME))
        .unwrap()
        .set_modified(an_hour_ago)
        .unwrap();
}

fn edit(library: &Library, document: impl Into<Target>, old: &str, new: &str) -> EditRequest {
    let _ = library;
    EditRequest {
        document: document.into(),
        old_text: old.into(),
        new_text: new.into(),
        write: claude(),
        ..Default::default()
    }
}

/// The reason an operation was refused because the app is open.
fn refusal<T: std::fmt::Debug>(result: writer_agent::Result<T>) -> Refusal {
    match result {
        Err(error @ Error::AppIsOpen { .. }) => {
            assert_eq!(error.kind(), ErrorKind::AppIsOpen);
            assert_eq!(error.exit_code(), 5);
            assert_eq!(error.code(), "app_is_open");
            let Error::AppIsOpen { why, .. } = error else {
                unreachable!()
            };
            why
        }
        other => panic!("not refused because the app is open: {other:?}"),
    }
}

// ----- the rule ---------------------------------------------------------------------------

#[test]
fn a_change_made_while_the_app_is_open_is_carried_out_by_the_app() {
    let fx = closed_fixture();
    let chapter = fx.create(Some("Book"), "# Chapter 1\n\nThe *Maren* left at dawn.\n");
    let app = StandIn::open(&fx);
    assert!(fx.library.app_is_open());
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && status.app_takes_changes);
    assert_eq!(app.names(), ["flush"]);

    let started = Instant::now();
    let edited = fx
        .library
        .edit(&edit(
            &fx.library,
            Target::path("Book/Chapter 1"),
            "at dawn",
            "before dawn",
        ))
        .unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(200),
        "a change does not wait for the app's lock: {:?}",
        started.elapsed()
    );
    // The result says who made the change: the app, which is open and takes changes.
    assert!(edited.written.changed && !edited.written.dry_run);
    assert!(edited.written.app_is_open && edited.written.app_takes_changes);
    assert_eq!(edited.replacements, 1);
    let kept = edited.snapshot.expect("the text as it was is kept");
    assert_eq!(kept.label.as_deref(), Some("Before an edit by Claude"));
    let served = app.taken();
    assert_eq!(served.len(), 1, "one request, one answer");
    let Served {
        request,
        changes,
        answer,
    } = &served[0];
    assert_eq!(request.name(), "edit");
    assert_eq!(request.author(), Some("Claude"));
    assert!(request.changes_library() && !request.is_dry_run() && !request.is_flush());
    assert!(answer.is_done());
    assert_eq!(
        changes,
        &vec![
            Change::VersionKept {
                document: chapter.id
            },
            Change::DocumentText { id: chapter.id },
        ]
    );
    // A read asks the app to save first, and sees what the app wrote.
    assert_eq!(
        fx.markdown(chapter.id),
        "# Chapter 1\n\nThe *Maren* left before dawn.\n"
    );
    assert_eq!(app.names(), ["flush"]);
    // The lock is the app's all the while.
    assert!(fx.library.app_is_open());

    // The app quits: the tool writes directly again.
    let socket = app.socket.clone();
    assert!(socket.exists());
    drop(app);
    assert!(!socket.exists(), "the socket goes with the app");
    assert!(!fx.library.app_is_open());
    let status = fx.library.status().unwrap();
    assert!(!status.app_is_open && !status.app_takes_changes);
    let direct = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "before dawn", "at noon"))
        .unwrap();
    assert!(direct.written.changed);
    assert!(!direct.written.app_is_open && !direct.written.app_takes_changes);
    assert_eq!(
        fx.markdown(chapter.id),
        "# Chapter 1\n\nThe *Maren* left at noon.\n"
    );
}

/// An app of 0.6.0 answered a real run with `"app_is_open": false, "app_takes_changes":
/// false` in the result, although it had made the change. A tool of today that is
/// answered so says what is true all the same: it handed the change to the app.
/// (Since the channel speaks version 2 or later an app of 0.6.0 refuses a tool of today whole,
/// `versions_of_the_channel_that_do_not_fit_tell_each_other`; what is held here is that
/// the tool does not take who made a change from the answer, whoever answers.)
#[test]
fn a_result_from_an_app_of_0_6_0_says_that_the_app_made_the_change() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    // What 0.6.0 answered, as its `execute` wrote it: carried out here by a tool, whose
    // direct write says "no app" as that version said of every real run.
    let as_0_6_0 = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    let mut result = serde_json::to_value(&as_0_6_0).unwrap();
    assert_eq!(result["app_is_open"], false);
    assert_eq!(result["app_takes_changes"], false);
    result.as_object_mut().unwrap().remove("would_change");
    let line = format!(
        "{}\n",
        serde_json::json!({"version": 1, "answer": "done", "result": result})
    );
    let app = RawApp::open(&fx, move |_| line.clone().into_bytes());
    let edited = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert!(edited.written.changed);
    assert!(edited.written.app_is_open && edited.written.app_takes_changes);
    let json = serde_json::to_value(&edited).unwrap();
    assert_eq!(json["app_is_open"], true);
    assert_eq!(json["app_takes_changes"], true);
    drop(app);
}

/// The other way round: what an app of today answers is read by a tool of 0.6.0. The
/// result of a real run has every key that version reads about the write, with the kind
/// of value it had then (that tool prints the two fields as the app wrote them, so with
/// an app of today it says what is true too).
#[test]
fn a_result_from_an_app_of_today_is_one_a_tool_of_0_6_0_reads() {
    /// `Written` as 0.6.0 had it, inside a result as that version read one: fields it
    /// does not know are passed over.
    #[derive(serde::Deserialize)]
    struct WrittenThen {
        dry_run: bool,
        changed: bool,
        app_is_open: bool,
        app_takes_changes: bool,
        would_change: bool,
    }
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let mut store = Store::open(fx.library.database_path()).unwrap();
    let request = Request::new(
        Operation::Edit(edit(&fx.library, chapter.id, "Text", "Words")),
        Duration::from_secs(1),
    );
    let Answer::Done { result } = live::execute(&mut store, &request).answer else {
        panic!("not done");
    };
    let then: WrittenThen =
        serde_json::from_value(result.clone()).unwrap_or_else(|error| panic!("{error}: {result}"));
    assert!(then.changed && !then.dry_run && then.would_change);
    assert!(
        then.app_is_open && then.app_takes_changes,
        "the app says so itself"
    );
}

/// What an operation gave, as text: its result as JSON, or its error with everything it
/// carries.
fn outcome<T: serde::Serialize>(result: writer_agent::Result<T>) -> String {
    match result {
        Ok(value) => format!("ok {}", serde_json::to_string_pretty(&value).unwrap()),
        Err(error) => format!(
            "error {error:?}\n{}",
            serde_json::to_string_pretty(&error.report()).unwrap()
        ),
    }
}

type Step = (&'static str, Box<dyn Fn(&Library) -> String>);

/// One of every operation that writes (and some of their failures), against the library
/// `seed` makes, each with the name it has in the table of operations.
fn steps(pictures: &Path) -> Vec<Step> {
    fn step<T: serde::Serialize>(
        name: &'static str,
        run: impl Fn(&Library) -> writer_agent::Result<T> + 'static,
    ) -> Step {
        (name, Box::new(move |library| outcome(run(library))))
    }
    let chapter = || Target::path("Book/Chapter 1");
    // Two picture files, outside both libraries: each library is brought their bytes.
    let picture = |name: &str, size: (u32, u32), rgb: [u8; 3]| -> String {
        let path = pictures.join(name);
        std::fs::write(&path, writer_image::fixtures::png(size.0, size.1, rgb)).unwrap();
        path.to_string_lossy().into_owned()
    };
    let harbour = picture("harbour.png", (640, 400), [30, 90, 160]);
    let square = picture("square.png", (48, 48), [200, 40, 40]);
    // A page, outside both libraries, that names a style sheet on the web.
    let page = pictures.join("orbitarium");
    fn write_page(page: &Path, words: &str) {
        std::fs::create_dir_all(page).unwrap();
        std::fs::write(
            page.join("index.html"),
            format!(
                "<!doctype html><title>Orbitarium</title>\
                 <link rel=stylesheet href=\"https://fonts.example.com/k.css\">\
                 <h1>Orbitarium</h1><p>{words} words.</p>"
            ),
        )
        .unwrap();
    }
    fn with_web(library: &Library) -> Library {
        library
            .clone()
            .with_fetcher(std::sync::Arc::new(writer_page::MapFetcher::new().with(
                "https://fonts.example.com/k.css",
                "text/css",
                "body { color: #222 }",
            )))
    }
    fn orbitarium(page: &Path) -> AddPageRequest {
        AddPageRequest {
            path: page.to_string_lossy().into_owned(),
            source_url: Some("https://claude.ai/artifact/Orbit".into()),
            folder: Some(Target::path("Claude Pages")),
            create_folders: true,
            write: claude(),
            ..Default::default()
        }
    }
    // The ids of the threads of the chapter, and of the replies of the first.
    fn threads(library: &Library) -> Vec<(u64, Vec<u64>)> {
        library
            .comments(&ListCommentsRequest {
                document: Target::path("Book/Chapter 1"),
                ..Default::default()
            })
            .unwrap()
            .threads
            .iter()
            .map(|thread| {
                (
                    thread.id,
                    thread.replies.iter().map(|reply| reply.id).collect(),
                )
            })
            .collect()
    }
    vec![
        // Comments first, while the text they are on is there.
        step("add_comment", move |library| {
            library.add_comment(&AddCommentRequest {
                document: chapter(),
                quote: "Maren".into(),
                body: "Which ship?".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("add_comment", move |library| {
            library.add_comment(&AddCommentRequest {
                document: chapter(),
                quote: "a quote that is not there".into(),
                body: "?".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("reply", move |library| {
            library.reply(&ReplyRequest {
                document: chapter(),
                comment: threads(library)[0].0,
                body: "The second one.".into(),
                write: WriteOptions::default(),
            })
        }),
        step("reply", move |library| {
            library.reply(&ReplyRequest {
                document: chapter(),
                comment: 4711,
                body: "To nobody.".into(),
                write: claude(),
            })
        }),
        step("resolve_comment", move |library| {
            library.resolve_comment(&CommentRequest {
                document: chapter(),
                comment: threads(library)[0].0,
                write: claude(),
            })
        }),
        step("reopen_comment", move |library| {
            library.reopen_comment(&CommentRequest {
                document: chapter(),
                comment: threads(library)[0].0,
                write: claude(),
            })
        }),
        step("delete_comment", move |library| {
            let (thread, replies) = threads(library)[0].clone();
            library.delete_comment(&DeleteCommentRequest {
                document: chapter(),
                comment: thread,
                reply: Some(replies[0]),
                write: claude(),
            })
        }),
        step("delete_comment", move |library| {
            library.delete_comment(&DeleteCommentRequest {
                document: chapter(),
                comment: threads(library)[1].0,
                reply: None,
                write: claude(),
            })
        }),
        // The text.
        step("edit", move |library| {
            library.edit(&edit(library, chapter(), "at dawn", "at dusk"))
        }),
        step("edit", move |library| {
            library.edit(&edit(library, chapter(), "no such text", "x"))
        }),
        step("edit", move |library| {
            library.edit(&EditRequest {
                all: true,
                occurrence: Some(2),
                ..edit(library, chapter(), "a", "b")
            })
        }),
        step("edit", move |library| {
            // "notes" is there more than once: which one?
            library.edit(&edit(library, chapter(), "notes", "words"))
        }),
        step("edit", move |library| {
            library.edit(&edit(library, Target::path("No such document"), "a", "b"))
        }),
        step("insert", move |library| {
            library.insert(&InsertRequest {
                document: chapter(),
                markdown: "Appended, with a [link](https://example.com).".into(),
                position: InsertPosition::End,
                write: claude(),
                ..Default::default()
            })
        }),
        step("insert", move |library| {
            library.insert(&InsertRequest {
                document: chapter(),
                markdown: "Before the notes.\n\n- one\n- two".into(),
                position: InsertPosition::BeforeHeading,
                heading: Some("Notes".into()),
                write: claude(),
                ..Default::default()
            })
        }),
        step("replace_section", move |library| {
            library.replace_section(&ReplaceSectionRequest {
                document: chapter(),
                heading: "Notes".into(),
                markdown: "New notes, **bold**.".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        // Images: a file, as a block of its own; files that the Markdown names; and an
        // image that is named again.
        step("add_image", {
            let harbour = harbour.clone();
            move |library| {
                library.add_image(&AddImageRequest {
                    document: chapter(),
                    path: harbour.clone(),
                    description: "The harbour at dusk".into(),
                    width_percent: Some(40),
                    position: InsertPosition::AfterHeading,
                    heading: Some("Notes".into()),
                    write: claude(),
                    ..Default::default()
                })
            }
        }),
        step("add_image", {
            let square = square.clone();
            move |library| {
                // No such heading: nothing is stored, not even the bytes.
                library.add_image(&AddImageRequest {
                    document: chapter(),
                    path: square.clone(),
                    position: InsertPosition::EndOfSection,
                    heading: Some("No such heading".into()),
                    write: claude(),
                    ..Default::default()
                })
            }
        }),
        step("insert", {
            let (harbour, square) = (harbour.clone(), square.clone());
            move |library| {
                let again = writer_store::ImageId::of(&std::fs::read(&harbour).unwrap());
                library.insert(&InsertRequest {
                    document: chapter(),
                    markdown: format!(
                        "![A red square]({square})\n\nThe harbour once more:\n\n\
                         ![](image:{again})\n\n![Not there](/no/such/picture.png)\n"
                    ),
                    write: claude(),
                    ..Default::default()
                })
            }
        }),
        step("replace_document", move |library| {
            // A text that names none of the three images: they go, and the result says
            // where they still are.
            library.replace_document(&ReplaceDocumentRequest {
                document: chapter(),
                markdown: "# Chapter 1\n\nRewritten at dusk.\n\n## Notes\n\nKept.\n".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("replace_document", move |library| {
            // The same text again: nothing to do.
            library.replace_document(&ReplaceDocumentRequest {
                document: chapter(),
                markdown: "# Chapter 1\n\nRewritten at dusk.\n\n## Notes\n\nKept.\n".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("restore_version", move |library| {
            let versions = library
                .versions(&ListVersionsRequest {
                    document: chapter(),
                })
                .unwrap()
                .versions;
            library.restore_version(&RestoreVersionRequest {
                version: versions.last().expect("a version").id.0,
                write: claude(),
                ..Default::default()
            })
        }),
        step("restore_version", move |library| {
            library.restore_version(&RestoreVersionRequest {
                version: 987_654,
                write: claude(),
                ..Default::default()
            })
        }),
        // Pages: one from a folder, with a piece from the web; the same again; a newer
        // copy; a text operation on it; its first version back; to the trash, where it is
        // left, and added as new all the same.
        step("add_page", {
            let page = page.clone();
            move |library| {
                write_page(&page, "First");
                with_web(library).add_page(&orbitarium(&page))
            }
        }),
        step("add_page", {
            let page = page.clone();
            move |library| with_web(library).add_page(&orbitarium(&page))
        }),
        step("add_page", {
            let page = page.clone();
            move |library| {
                write_page(&page, "Second");
                with_web(library).add_page(&orbitarium(&page))
            }
        }),
        step("edit", move |library| {
            library.edit(&edit(
                library,
                Target::path("Claude Pages/Orbitarium"),
                "Second",
                "x",
            ))
        }),
        step("rename_document", move |library| {
            library.rename_document(&RenameDocumentRequest {
                document: Target::path("Claude Pages/Orbitarium"),
                title: "My Orbitarium".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("restore_version", move |library| {
            let versions = library
                .versions(&ListVersionsRequest {
                    document: Target::path("Claude Pages/My Orbitarium"),
                })
                .unwrap()
                .versions;
            library.restore_version(&RestoreVersionRequest {
                version: versions.last().expect("a version").id.0,
                write: claude(),
                ..Default::default()
            })
        }),
        step("trash_document", move |library| {
            library.trash_document(&TrashDocumentRequest {
                document: Target::path("Claude Pages/My Orbitarium"),
                write: claude(),
            })
        }),
        step("add_page", {
            let page = page.clone();
            move |library| with_web(library).add_page(&orbitarium(&page))
        }),
        step("add_page", {
            let page = page.clone();
            move |library| {
                with_web(library).add_page(&AddPageRequest {
                    include_trashed: true,
                    ..orbitarium(&page)
                })
            }
        }),
        // Documents as items.
        step("create_document", move |library| {
            library.create_document(&CreateDocumentRequest {
                markdown: "# New\n\nSome text, with a ~~flaw~~.".into(),
                title: Some("A chosen title".into()),
                folder: Some(Target::path("Elsewhere/Deep")),
                create_folders: true,
                write: claude(),
                ..Default::default()
            })
        }),
        step("create_document", move |library| {
            library.create_document(&CreateDocumentRequest {
                markdown: "text".into(),
                folder: Some(Target::path("Nowhere")),
                write: claude(),
                ..Default::default()
            })
        }),
        step("create_document", move |library| {
            // A document called like a number, placed first: named by a path, it is a
            // path on the other side too.
            library.create_document(&CreateDocumentRequest {
                markdown: "# 1984\n\nA year.".into(),
                folder: Some(Target::path("Book")),
                placement: Some(Placement::First),
                write: claude(),
                ..Default::default()
            })
        }),
        step("rename_document", move |library| {
            library.rename_document(&RenameDocumentRequest {
                document: Target::path("1984"),
                title: "  Nineteen   eighty-four ".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("rename_document", move |library| {
            library.rename_document(&RenameDocumentRequest {
                document: Target::path("Nineteen eighty-four"),
                follow_text: true,
                write: claude(),
                ..Default::default()
            })
        }),
        step("duplicate_document", move |library| {
            library.duplicate_document(&DuplicateDocumentRequest {
                document: Target::path("Loose"),
                title: Some("Twin".into()),
                folder: Some(Target::path("Notes")),
                write: claude(),
            })
        }),
        step("duplicate_document", move |library| {
            library.duplicate_document(&DuplicateDocumentRequest {
                document: Target::path("Loose"),
                title: Some("Twin".into()),
                write: claude(),
                ..Default::default()
            })
        }),
        step("edit", move |library| {
            // Two documents are called "Twin" now: the error lists both.
            library.edit(&edit(library, Target::path("Twin"), "loose", "tight"))
        }),
        step("move_document", move |library| {
            library.move_document(&MoveDocumentRequest {
                document: Target::path("Loose"),
                folder: Some(Target::path("Book/Parts/One")),
                create_folders: true,
                write: claude(),
                ..Default::default()
            })
        }),
        step("move_document", move |library| {
            library.move_document(&MoveDocumentRequest {
                document: Target::path("Notes/Twin"),
                folder: Some(Target::path("Book")),
                placement: Some(Placement::Before(Target::path("Book/Chapter 1"))),
                write: claude(),
                ..Default::default()
            })
        }),
        step("trash_document", move |library| {
            library.trash_document(&TrashDocumentRequest {
                document: Target::path("Book/Twin"),
                write: claude(),
            })
        }),
        step("restore_document", move |library| {
            library.restore_document(&RestoreDocumentRequest {
                document: Target::path("Gone"),
                write: claude(),
            })
        }),
        step("delete_document", move |library| {
            library.delete_document(&DeleteDocumentRequest {
                document: Target::path("Gone"),
                permanently: false,
                write: claude(),
            })
        }),
        step("delete_document", move |library| {
            library.delete_document(&DeleteDocumentRequest {
                document: Target::path("Gone"),
                permanently: true,
                write: claude(),
            })
        }),
        // Folders.
        step("create_folder", move |library| {
            library.create_folder(&CreateFolderRequest {
                path: "Book/Drafts/Old".into(),
                write: claude(),
                ..Default::default()
            })
        }),
        step("create_folder", move |library| {
            // It is there already.
            library.create_folder(&CreateFolderRequest {
                path: "Drafts".into(),
                parent: Some(Target::path("Book")),
                write: claude(),
            })
        }),
        step("rename_folder", move |library| {
            library.rename_folder(&RenameFolderRequest {
                folder: Target::path("Notes"),
                name: "Jottings".into(),
                write: claude(),
            })
        }),
        step("move_folder", move |library| {
            library.move_folder(&MoveFolderRequest {
                folder: Target::path("Jottings"),
                parent: Some(Target::path("Book")),
                placement: Some(Placement::First),
                write: claude(),
            })
        }),
        step("move_folder", move |library| {
            // Into itself.
            library.move_folder(&MoveFolderRequest {
                folder: Target::path("Book"),
                parent: Some(Target::path("Book/Drafts")),
                write: claude(),
                ..Default::default()
            })
        }),
        step("trash_folder", move |library| {
            library.trash_folder(&TrashFolderRequest {
                folder: Target::path("Book/Drafts"),
                write: claude(),
            })
        }),
        step("restore_folder", move |library| {
            library.restore_folder(&RestoreFolderRequest {
                folder: Target::path("Drafts"),
                write: claude(),
            })
        }),
        step("trash_folder", move |library| {
            library.trash_folder(&TrashFolderRequest {
                folder: Target::path("Book/Jottings"),
                write: claude(),
            })
        }),
        step("delete_folder", move |library| {
            library.delete_folder(&DeleteFolderRequest {
                folder: Target::path("Elsewhere"),
                permanently: false,
                write: claude(),
            })
        }),
        step("delete_folder", move |library| {
            // A live folder with a document in it, for good.
            library.delete_folder(&DeleteFolderRequest {
                folder: Target::path("Elsewhere"),
                permanently: true,
                write: claude(),
            })
        }),
        step("empty_trash", move |library| {
            library.empty_trash(&EmptyTrashRequest {
                permanently: false,
                write: claude(),
            })
        }),
        step("empty_trash", move |library| {
            library.empty_trash(&EmptyTrashRequest {
                permanently: true,
                write: claude(),
            })
        }),
    ]
}

/// The test that lists the operations that write: each of them, carried out by the app,
/// gives the result and leaves the library that the same operation written directly
/// gives and leaves. An operation that is added to the table and not to `steps` fails
/// it, and so does one that the app is not asked for.
#[test]
fn every_operation_gives_the_same_result_and_the_same_library_both_ways() {
    // Two libraries with the same history: one is written to directly, the other has an
    // app open.
    let direct = closed_fixture();
    let through = closed_fixture();
    seed(&direct);
    seed(&through);
    assert_eq!(direct.dump(), through.dump(), "the same start");
    let app = StandIn::open(&through);
    let pictures = tempfile::tempdir().unwrap();

    let mut names = BTreeSet::new();
    let mut most_images = 0;
    for (number, (name, run)) in steps(pictures.path()).into_iter().enumerate() {
        direct.clock.advance_ms(1000);
        through.clock.advance_ms(1000);
        let written = run(&direct.library);
        app.names();
        let carried_out = run(&through.library);
        // The one thing in which the two results differ is who made the change.
        let by_the_app = written
            .replace("\"app_is_open\": false", "\"app_is_open\": true")
            .replace(
                "\"app_takes_changes\": false",
                "\"app_takes_changes\": true",
            );
        assert_eq!(
            written.starts_with("ok "),
            by_the_app != written,
            "step {number} ({name}): every result says whether the app made the change"
        );
        assert_eq!(
            carried_out, by_the_app,
            "step {number} ({name}): the app gave another result than a direct write"
        );
        assert_eq!(
            app.changes(),
            [name],
            "step {number} ({name}): the app was not asked for exactly this"
        );
        assert_eq!(
            through.dump(),
            direct.dump(),
            "step {number} ({name}): the two libraries differ"
        );
        // The pictures are part of what is compared: how many each library holds, and
        // the rows of both `images.db` in the dump, bytes included.
        assert_eq!(
            through.store().image_count().unwrap(),
            direct.store().image_count().unwrap(),
            "step {number} ({name})"
        );
        most_images = most_images.max(direct.dump().matches(" bytes ").count());
        names.insert(name);
    }
    assert!(through.library.app_is_open());
    assert_eq!(
        direct.store().image_count().unwrap(),
        2,
        "two files were brought"
    );
    assert!(direct.dump().contains("== images.db images"));
    assert!(
        most_images >= 2,
        "the bytes of the pictures are in the dump"
    );

    // Every operation of the table was among them, and nothing else.
    let table: BTreeSet<&str> = Operation::CHANGES.iter().copied().collect();
    assert_eq!(table.len(), Operation::CHANGES.len(), "a name twice");
    assert_eq!(
        names, table,
        "the steps above and the table of operations (src/operations.rs) name different \
         operations: an operation that writes has to be in both"
    );
    assert_eq!(table.len(), 26);

    // What a search finds is the same too (the index is not in the dump).
    for query in ["dusk", "lighthouse", "Rewritten", "notes", "year"] {
        let find = |fx: &Fixture| {
            fx.library
                .search(&SearchRequest {
                    query: query.into(),
                    ..Default::default()
                })
                .unwrap()
        };
        assert_eq!(find(&through), find(&direct), "{query}");
    }
    assert_eq!(
        through.library.tree(&TreeRequest::default()).unwrap(),
        direct.library.tree(&TreeRequest::default()).unwrap()
    );
}

/// The list of what changed, for the kinds of change the app treats differently.
#[test]
fn the_app_is_told_what_changed() {
    let fx = closed_fixture();
    seed(&fx);
    let app = StandIn::open(&fx);
    let chapter = fx
        .library
        .resolve_document(&Target::path("Book/Chapter 1"))
        .unwrap()
        .id;
    let changes = |app: &StandIn| -> Vec<Change> {
        app.taken()
            .into_iter()
            .filter(|served| !served.request.is_flush())
            .flat_map(|served| served.changes)
            .collect()
    };
    changes(&app);

    // A comment: only the comments of that document.
    let added = fx
        .library
        .add_comment(&AddCommentRequest {
            document: chapter.into(),
            quote: "Maren".into(),
            body: "Which ship?".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(changes(&app), [Change::DocumentComments { id: chapter }]);
    // Deleting one keeps a version first.
    fx.library
        .delete_comment(&DeleteCommentRequest {
            document: chapter.into(),
            comment: added.thread.id,
            reply: None,
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        changes(&app),
        [
            Change::VersionKept { document: chapter },
            Change::DocumentComments { id: chapter },
        ]
    );
    // An edit that takes the text of a thread away says so.
    let rewritten = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: chapter.into(),
            markdown: "# Chapter 1\n\nAll new.\n".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rewritten.comments_detached.len(), 1);
    assert_eq!(
        changes(&app),
        [
            Change::VersionKept { document: chapter },
            Change::DocumentText { id: chapter },
            Change::DocumentComments { id: chapter },
        ]
    );
    // What asks for what is already so changes nothing, and neither does a failure.
    let same = fx
        .library
        .replace_document(&ReplaceDocumentRequest {
            document: chapter.into(),
            markdown: "# Chapter 1\n\nAll new.\n".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(!same.written.changed);
    assert!(
        fx.library
            .edit(&edit(&fx.library, chapter, "not there", "x"))
            .is_err()
    );
    assert_eq!(changes(&app), []);

    // A document created with folders on the way and a title of its own.
    let created = fx
        .library
        .create_document(&CreateDocumentRequest {
            markdown: "Text.".into(),
            title: Some("Chosen".into()),
            folder: Some(Target::path("A/B")),
            create_folders: true,
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let folders: Vec<_> = created.created_folders.iter().map(|f| f.id).collect();
    assert_eq!(
        changes(&app),
        [
            Change::FolderCreated { id: folders[0] },
            Change::FolderCreated { id: folders[1] },
            Change::DocumentCreated {
                id: created.document.id
            },
            Change::DocumentRenamed {
                id: created.document.id
            },
        ]
    );
    // A live document deleted for good is deleted, not trashed and deleted.
    fx.library
        .delete_document(&DeleteDocumentRequest {
            document: created.document.id.into(),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert_eq!(
        changes(&app),
        [Change::DocumentDeleted {
            id: created.document.id
        }]
    );
    // A folder that goes to the trash is one entry, whatever is in it; so is one that
    // is deleted for good while it is live.
    fx.library
        .trash_folder(&TrashFolderRequest {
            folder: folders[1].into(),
            write: claude(),
        })
        .unwrap();
    assert_eq!(changes(&app), [Change::FolderTrashed { id: folders[1] }]);
    fx.library
        .delete_folder(&DeleteFolderRequest {
            folder: Target::path("Book"),
            permanently: true,
            write: claude(),
        })
        .unwrap();
    let book = changes(&app);
    assert!(
        matches!(book.as_slice(), [Change::FolderDeleted { .. }]),
        "{book:?}"
    );
    fx.library
        .empty_trash(&EmptyTrashRequest {
            permanently: true,
            write: claude(),
        })
        .unwrap();
    assert_eq!(changes(&app), [Change::TrashEmptied]);
    // The list travels as JSON in no answer; it is the app's own. What it looks like:
    assert_eq!(
        serde_json::to_string(&Change::DocumentText { id: DocumentId(3) }).unwrap(),
        r#"{"change":"document_text","id":3}"#
    );
}

/// Nothing in the crate gets at a library it can write to but `Library::write`, and
/// nothing calls that but the table of operations; every other public method of
/// `Library` is one of the readers this test knows.
#[test]
fn nothing_writes_to_a_library_but_through_the_table() {
    const READERS: &[&str] = &[
        "at",
        "default_location",
        "locate",
        "with_clock",
        "with_base_dir",
        "with_patience",
        "with_app_patience",
        "with_save_patience",
        "data_dir",
        "database_path",
        "app_is_open",
        "status",
        "tree",
        "search",
        "recent",
        "info",
        "folder_info",
        "resolve",
        "resolve_document",
        "resolve_folder",
        "read",
        "find",
        "trash",
        "comments",
        "versions",
        "read_version",
        "export",
        "with_page_checker",
        "with_fetcher",
        "pages",
    ];
    fn sources(dir: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                out.push((
                    path.to_string_lossy().into_owned(),
                    std::fs::read_to_string(&path).unwrap(),
                ));
            }
        }
    }
    let mut files = Vec::new();
    sources(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    assert!(files.len() > 15, "the sources of the crate");
    let mut methods = BTreeSet::new();
    for (path, text) in &files {
        let is = |name: &str| path.ends_with(name);
        // The code only: a comment may name anything.
        let code: Vec<&str> = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect();
        for line in &code {
            if line.contains("open_for_writing(") || line.contains("Store::open_with(") {
                assert!(is("src/library.rs"), "{path}: {line}");
            }
            if line.contains("Session::writing(") || line.contains("Session {") {
                assert!(
                    is("src/library.rs") || is("src/live/mod.rs"),
                    "{path}: {line}"
                );
            }
            if line.contains("self.write(") || line.contains(".write(&") {
                assert!(is("src/operations.rs"), "{path}: {line}");
            }
        }
        // The public methods of `Library`, wherever they are.
        let mut in_library = false;
        for line in &code {
            if line.starts_with("impl Library") {
                in_library = true;
            } else if line.starts_with('}') {
                in_library = false;
            }
            if in_library
                && let Some(rest) = line.trim_start().strip_prefix("pub fn ")
                && !is("src/operations.rs")
            {
                let name = rest.split(['(', '<']).next().unwrap();
                methods.insert(name.to_string());
            }
        }
    }
    let readers: BTreeSet<String> = READERS.iter().map(|name| name.to_string()).collect();
    assert_eq!(
        methods, readers,
        "a public method of Library that is not in the table of operations \
         (src/operations.rs): if it writes to the library it belongs there, so that the \
         app carries it out while it is open; if it only reads, name it in this test"
    );
    for name in Operation::CHANGES {
        assert!(!readers.contains(*name), "{name} is in both lists");
    }
}

// ----- reading ----------------------------------------------------------------------------

#[test]
fn a_read_asks_the_app_to_save_first_and_does_not_wait_for_it_for_long() {
    let fx = closed_fixture();
    let chapter = fx.create(Some("Book"), "# Chapter 1\n\nSaved text.\n");
    // An app with text that is not saved yet: it saves when it is asked to, and counts.
    let slow = Arc::new(AtomicBool::new(false));
    let saves = Arc::new(AtomicUsize::new(0));
    let app = StandIn::open_with(&fx, {
        let (slow, saves) = (slow.clone(), saves.clone());
        Box::new(move |store, incoming| {
            if slow.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(400));
            }
            if incoming.request().is_flush() && incoming.begin() {
                // What is on screen: the text that was saved, with a word typed since.
                let number = saves.fetch_add(1, Ordering::SeqCst) + 1;
                let stored = store.document(chapter.id).unwrap();
                let (was, now) = match number {
                    1 => ("Saved text.".to_string(), "Typed once.".to_string()),
                    _ => (
                        format!("Typed {} times.", number - 1),
                        format!("Typed {number} times."),
                    ),
                };
                let was = if number == 2 {
                    "Typed once.".to_string()
                } else {
                    was
                };
                assert!(stored.content.contains(&was), "{}", stored.content);
                let content = stored.content.replace(&was, &now);
                let document = writer_core::Document::from_json(&content).unwrap();
                let stats = document.stats();
                store
                    .save_document(
                        chapter.id,
                        &writer_store::DocumentContent {
                            content: &content,
                            title: &stored.meta.title,
                            plain_text: &document.plain_text(),
                            word_count: stats.words as u64,
                            char_count: stats.characters as u64,
                        },
                    )
                    .unwrap();
            }
            as_the_app(store, incoming)
        })
    });

    // The read sees the text as it is on screen.
    let text = fx
        .library
        .read(&ReadRequest {
            document: chapter.id.into(),
            format: writer_agent::TextFormat::PlainText,
            ..Default::default()
        })
        .unwrap();
    assert!(text.content.contains("Typed once."), "{}", text.content);
    assert_eq!(app.names(), ["flush"]);

    // Everything that reads asks once, and nothing that reads changes anything.
    let out = fx.dir.path().join("out");
    let id = chapter.id;
    let chapter_target = move || Target::from(id);
    let version = fx
        .library
        .edit(&edit(
            &fx.library,
            chapter.id,
            "Chapter 1",
            "Chapter with a window",
        ))
        .unwrap()
        .snapshot
        .unwrap()
        .id
        .0;
    app.names();
    let before = saves.load(Ordering::SeqCst);
    type Reader = Box<dyn Fn(&Library) -> bool>;
    let readers: Vec<(&str, Reader)> = vec![
        ("status", Box::new(|l| l.status().is_ok())),
        (
            "tree",
            Box::new(|l| l.tree(&TreeRequest::default()).is_ok()),
        ),
        (
            "search",
            Box::new(|l| {
                l.search(&SearchRequest {
                    query: "window".into(),
                    ..Default::default()
                })
                .is_ok_and(|found| found.hits.len() == 1)
            }),
        ),
        (
            "recent",
            Box::new(|l| l.recent(&RecentRequest::default()).is_ok()),
        ),
        (
            "info",
            Box::new(move |l| {
                l.info(&InfoRequest {
                    document: chapter_target(),
                })
                .is_ok()
            }),
        ),
        (
            "folder_info",
            Box::new(|l| {
                l.folder_info(&FolderInfoRequest {
                    folder: Target::path("Book"),
                })
                .is_ok()
            }),
        ),
        (
            "resolve",
            Box::new(|l| l.resolve(&Target::path("Book"), false).is_ok()),
        ),
        (
            "resolve_document",
            Box::new(move |l| l.resolve_document(&chapter_target()).is_ok()),
        ),
        (
            "resolve_folder",
            Box::new(|l| l.resolve_folder(&Target::path("Book")).is_ok()),
        ),
        (
            "read",
            Box::new(move |l| {
                l.read(&ReadRequest {
                    document: chapter_target(),
                    ..Default::default()
                })
                .is_ok()
            }),
        ),
        (
            "find",
            Box::new(move |l| {
                l.find(&FindRequest {
                    document: chapter_target(),
                    text: "window".into(),
                    ..Default::default()
                })
                .is_ok_and(|found| found.count == 1)
            }),
        ),
        ("trash", Box::new(|l| l.trash().is_ok())),
        (
            "comments",
            Box::new(move |l| {
                l.comments(&ListCommentsRequest {
                    document: chapter_target(),
                    ..Default::default()
                })
                .is_ok()
            }),
        ),
        (
            "versions",
            Box::new(move |l| {
                l.versions(&ListVersionsRequest {
                    document: chapter_target(),
                })
                .is_ok()
            }),
        ),
        (
            "read_version",
            Box::new(move |l| {
                l.read_version(&ReadVersionRequest {
                    version,
                    ..Default::default()
                })
                .is_ok()
            }),
        ),
        (
            "export",
            Box::new(move |l| {
                l.export(&ExportRequest {
                    document: Some(chapter_target()),
                    path: out.join("chapter.md"),
                    ..Default::default()
                })
                .is_ok()
            }),
        ),
        (
            "a dry run",
            Box::new(move |l| {
                l.edit(&EditRequest {
                    write: dry(),
                    ..edit(l, chapter_target(), "window", "door")
                })
                .is_ok_and(|tried| tried.written.app_is_open && tried.written.app_takes_changes)
            }),
        ),
    ];
    for (count, (name, read)) in readers.iter().enumerate() {
        assert!(read(&fx.library), "{name}");
        assert_eq!(app.names(), ["flush"], "{name} asks the app to save, once");
        assert_eq!(saves.load(Ordering::SeqCst), before + count + 1, "{name}");
    }

    // An app that is busy is waited for as long as the library says, and then the read
    // goes ahead on what is saved.
    slow.store(true, Ordering::SeqCst);
    let impatient = fx
        .library
        .clone()
        .with_save_patience(Duration::from_millis(100));
    let saved_before = saves.load(Ordering::SeqCst);
    let started = Instant::now();
    let text = impatient
        .read(&ReadRequest {
            document: chapter.id.into(),
            format: writer_agent::TextFormat::PlainText,
            ..Default::default()
        })
        .unwrap();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(90) && took < Duration::from_millis(350),
        "{took:?}"
    );
    assert!(
        text.content
            .contains(&format!("Typed {saved_before} times.")),
        "what was saved before: {}",
        text.content
    );
    // The app got to it too late, was not waited for, and did nothing for it.
    std::thread::sleep(Duration::from_millis(450));
    assert_eq!(saves.load(Ordering::SeqCst), saved_before);
    assert_eq!(app.names(), [] as [&str; 0]);
}

// ----- the owner's switch -------------------------------------------------------------------

#[test]
fn with_the_switch_off_the_old_rule_holds() {
    let fx = closed_fixture();
    seed(&fx);
    let app = StandIn::open(&fx);
    set_switch(&fx, false);
    let before = fx.dump();

    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);

    // Everything that writes is refused, in words that say where the switch is.
    let results = every_write(&fx, &claude());
    assert_eq!(results.len(), Operation::CHANGES.len());
    for (name, result) in results {
        let why = refusal(result);
        assert_eq!(why, Refusal::SwitchedOff, "{name}");
        let message = Error::AppIsOpen {
            pid: Some(1),
            data_dir: fx.data_dir(),
            why,
        }
        .to_string();
        assert!(message.contains("switched off"), "{message}");
        assert!(message.contains("nothing was changed"), "{message}");
        assert!(message.contains("Agents panel"), "{message}");
    }
    assert_eq!(fx.dump(), before, "a refused change changed the library");
    let refused: Vec<_> = app
        .taken()
        .into_iter()
        .filter(|served| !served.request.is_flush())
        .collect();
    assert_eq!(refused.len(), Operation::CHANGES.len());
    for served in refused {
        assert_eq!(served.answer, Answer::refused(Refusal::SwitchedOff));
        assert!(served.changes.is_empty());
    }

    // Reading works, and the app still saves for it. A dry run says what the real run
    // would meet.
    assert!(
        fx.library
            .read(&ReadRequest {
                document: Target::path("Book/Chapter 1"),
                ..Default::default()
            })
            .unwrap()
            .content
            .contains("at dawn")
    );
    assert_eq!(app.names(), ["flush"]);
    let tried = fx
        .library
        .edit(&EditRequest {
            write: dry(),
            ..edit(&fx.library, Target::path("Chapter 1"), "at dawn", "at dusk")
        })
        .unwrap();
    assert!(tried.written.dry_run && tried.written.app_is_open);
    assert!(!tried.written.app_takes_changes);
    assert!(tried.written.would_change && !tried.written.changed);
    assert_eq!(fx.dump(), before);

    // On again: the same change is made.
    set_switch(&fx, true);
    assert!(fx.library.status().unwrap().app_takes_changes);
    let done = fx
        .library
        .edit(&edit(
            &fx.library,
            Target::path("Chapter 1"),
            "at dawn",
            "at dusk",
        ))
        .unwrap();
    assert!(done.written.changed);
    assert_ne!(fx.dump(), before);
}

// ----- what can go wrong --------------------------------------------------------------------

/// An app from before 0.6.0 (or one that could not open its channel) holds the lock and
/// names no socket; one that names a socket nobody listens on has lost its channel.
#[test]
fn an_app_that_holds_the_lock_and_does_not_listen_is_told_apart_and_nothing_is_written() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let before = fx.bytes();
    let app = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    age_the_lock(&fx);
    let change = || {
        fx.library
            .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
    };

    // No channel named: a version that takes no changes.
    let started = Instant::now();
    assert_eq!(refusal(change()), Refusal::OldVersion);
    assert!(started.elapsed() < Duration::from_secs(1), "not waited for");

    // A channel named, and nothing there: not even a file.
    let dir = socket_dir();
    let socket = dir.path().join("gone.sock");
    instance_lock::announce_channel(&fx.data_dir(), Some(&socket)).unwrap();
    age_the_lock(&fx);
    let error = change().unwrap_err();
    let message = error.to_string();
    assert_eq!(refusal(Err::<(), _>(error)), Refusal::NotListening);
    assert!(
        message.contains("does not answer where it listens"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");

    // A socket file that nobody listens on (what a crash leaves) is no better.
    drop(UnixListener::bind(&socket).unwrap());
    assert!(socket.exists());
    assert_eq!(refusal(change()), Refusal::NotListening);

    // Reading goes on all the while, without waiting for anybody.
    let started = Instant::now();
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nText.\n");
    assert!(started.elapsed() < Duration::from_millis(500));
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open);
    assert_eq!(fx.bytes(), before);

    // The app goes: the lock file still names the socket, the socket file is still
    // there, and nobody is misled, because nobody holds the lock.
    drop(app);
    assert!(
        std::fs::read_to_string(fx.data_dir().join(instance_lock::LOCK_FILE_NAME))
            .unwrap()
            .contains("channel ")
    );
    assert!(!fx.library.app_is_open());
    assert!(change().unwrap().written.changed);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");
}

/// "It takes no changes" has two reasons, and the tool can tell them apart without
/// anything new on the channel: an app that listens and says "no" has the switch off (an
/// app of 0.6.0 says that in the same words), an app that has been running for a while
/// and names no channel is one that never takes a change. Only an app that has just
/// taken the lock and names no channel yet is not known.
#[test]
fn the_status_and_a_dry_run_say_why_the_app_takes_no_changes_where_that_can_be_known() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let tried = |library: &Library| {
        library
            .edit(&EditRequest {
                write: dry(),
                ..edit(library, chapter.id, "Text", "Words")
            })
            .unwrap()
            .written
    };
    // No app: nothing refuses.
    let status = fx.library.status().unwrap();
    assert!(!status.app_is_open && status.app_refuses.is_none());
    assert_eq!(tried(&fx.library).app_refuses, None);
    assert!(
        !serde_json::to_string(&status)
            .unwrap()
            .contains("app_refuses"),
        "absent, not null"
    );

    // The app of today with the switch off.
    let app = StandIn::open(&fx);
    let status = fx.library.status().unwrap();
    assert!(status.app_takes_changes && status.app_refuses.is_none());
    assert_eq!(tried(&fx.library).app_refuses, None);
    set_switch(&fx, false);
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);
    assert_eq!(status.app_refuses, Some(Refusal::SwitchedOff));
    let written = tried(&fx.library);
    assert!(written.app_is_open && !written.app_takes_changes);
    assert_eq!(written.app_refuses, Some(Refusal::SwitchedOff));
    // What the refusal of the change itself says is the same.
    assert_eq!(
        refusal(
            fx.library
                .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        ),
        Refusal::SwitchedOff
    );
    assert_eq!(
        serde_json::to_value(&status).unwrap()["app_refuses"],
        serde_json::json!({"why": "switched_off"})
    );
    set_switch(&fx, true);
    drop(app);

    // An app of 0.6.0 with the switch off: it answers "flush" with exactly this line.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":1,\"answer\":\"done\",\"result\":{\"takes_changes\":false,\
          \"app_version\":\"0.6.0\"}}\n"
            .to_vec()
    });
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);
    assert_eq!(status.app_refuses, Some(Refusal::SwitchedOff));
    assert_eq!(tried(&fx.library).app_refuses, Some(Refusal::SwitchedOff));
    drop(app);
    // And with the switch on, in its words.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":1,\"answer\":\"done\",\"result\":{\"takes_changes\":true,\
          \"app_version\":\"0.6.0\"}}\n"
            .to_vec()
    });
    let status = fx.library.status().unwrap();
    assert!(status.app_takes_changes && status.app_refuses.is_none());
    drop(app);

    // An app that holds the lock and names no channel. In its first seconds it may be
    // one that is about to: that cannot be known, and nothing is claimed.
    let lock = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);
    assert_eq!(status.app_refuses, None, "it has only just started");
    assert_eq!(tried(&fx.library).app_refuses, None);
    // After that it is a version that takes none.
    age_the_lock(&fx);
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && !status.app_takes_changes);
    assert_eq!(status.app_refuses, Some(Refusal::OldVersion));
    let written = tried(&fx.library);
    assert!(written.app_is_open && !written.app_takes_changes);
    assert_eq!(written.app_refuses, Some(Refusal::OldVersion));
    assert_eq!(
        refusal(
            fx.library
                .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        ),
        Refusal::OldVersion
    );
    drop(lock);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nText.\n");
}

/// What an app of today puts on the channel is what 0.6.0 put there, key for key: the
/// reason is worked out by the tool, so a tool of 0.6.0 meets nothing it did not know.
#[test]
fn nothing_new_goes_over_the_channel_for_the_reason() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let mut store = Store::open(fx.library.database_path()).unwrap();
    store.set_setting(CHANGES_SETTING, &false).unwrap();
    for operation in [Operation::Flush, Operation::Hello] {
        let answer = live::execute(&mut store, &Request::new(operation, Duration::from_secs(1)));
        assert_eq!(
            serde_json::to_value(&answer.answer).unwrap(),
            serde_json::json!({
                "answer": "done",
                "result": {"takes_changes": false, "app_version": env!("CARGO_PKG_VERSION")},
            })
        );
    }
    let refused = live::execute(
        &mut store,
        &Request::new(
            Operation::Edit(edit(&fx.library, chapter.id, "Text", "Words")),
            Duration::from_secs(1),
        ),
    );
    assert_eq!(
        serde_json::to_value(&refused.answer).unwrap(),
        serde_json::json!({"answer": "refused", "refusal": {"why": "switched_off"}})
    );
    // The result of a real run has no key for it either.
    store.set_setting(CHANGES_SETTING, &true).unwrap();
    let Answer::Done { result } = live::execute(
        &mut store,
        &Request::new(
            Operation::Edit(edit(&fx.library, chapter.id, "Text", "Words")),
            Duration::from_secs(1),
        ),
    )
    .answer
    else {
        panic!("not done");
    };
    assert!(result.get("app_refuses").is_none(), "{result}");
}

#[test]
fn a_socket_left_behind_by_a_crash_misleads_nobody() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let app = StandIn::open(&fx);
    fx.library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert_eq!(app.changes(), ["edit"]);

    // The app is killed. Its socket is there, the lock file names it; here the dead
    // app's thread even still listens.
    app.crash();
    assert!(app.socket.exists());
    assert_eq!(
        instance_lock::app_channel(&fx.data_dir()).unwrap(),
        None,
        "no lock, no channel"
    );
    assert!(!fx.library.app_is_open());
    let status = fx.library.status().unwrap();
    assert!(!status.app_is_open && !status.app_takes_changes);
    // The tool writes directly and asks nobody.
    let done = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Words", "Lines"))
        .unwrap();
    assert!(done.written.changed);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nLines.\n");
    assert_eq!(app.names(), [] as [&str; 0], "the dead app was not asked");

    // The app is started again: it listens somewhere else, and is found there.
    let old = app.socket.clone();
    let again = StandIn::open(&fx);
    assert_ne!(again.socket, old);
    fx.library
        .edit(&edit(&fx.library, chapter.id, "Lines", "Pages"))
        .unwrap();
    assert_eq!(again.changes(), ["edit"]);
    assert_eq!(app.names(), [] as [&str; 0]);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nPages.\n");
}

#[test]
fn an_app_that_has_just_started_is_waited_for() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    // The app takes the lock when it starts, and opens its channel a moment later.
    let lock = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    let (data_dir, database, clock) = (fx.data_dir(), fx.library.database_path(), fx.clock.clone());
    let app = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        let mut store = Store::open(database).unwrap();
        store.set_clock(clock);
        let listener = Listener::bind(&data_dir).unwrap();
        let incoming = listener.accept().expect("the tool comes");
        as_the_app(&mut store, incoming).expect("served")
    });
    let started = Instant::now();
    let done = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert!(done.written.changed);
    assert!(started.elapsed() >= Duration::from_millis(250));
    let served = app.join().unwrap();
    assert_eq!(served.request.name(), "edit");
    drop(lock);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");

    // One that never opens a channel is not waited for longer than the tool has
    // patience (five seconds at most from the moment it took the lock).
    let lock = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    let impatient = fx
        .library
        .clone()
        .with_app_patience(Duration::from_millis(150));
    let started = Instant::now();
    assert_eq!(
        refusal(impatient.edit(&edit(&impatient, chapter.id, "Words", "Lines"))),
        Refusal::OldVersion
    );
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(140) && took < Duration::from_secs(2),
        "{took:?}"
    );
    drop(lock);
}

#[test]
fn an_app_that_does_not_get_to_a_change_in_time_changes_nothing_and_says_so() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let began = Arc::new(Mutex::new(Vec::new()));
    let app = StandIn::open_with(&fx, {
        let began = began.clone();
        Box::new(move |store, incoming| {
            // Busy: a dialog of the system is open.
            if !incoming.request().is_flush() {
                std::thread::sleep(Duration::from_millis(400));
            }
            assert_eq!(incoming.expired(), !incoming.request().is_flush());
            began.lock().unwrap().push(incoming.begin());
            as_the_app(store, incoming)
        })
    });
    let before = fx.dump();
    let impatient = fx
        .library
        .clone()
        .with_app_patience(Duration::from_millis(100));
    let started = Instant::now();
    let error = impatient
        .edit(&edit(&impatient, chapter.id, "Text", "Words"))
        .unwrap_err();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(90) && took < Duration::from_millis(350),
        "the tool is told when its patience is over, not when the app gets to it: {took:?}"
    );
    let message = error.to_string();
    assert_eq!(
        refusal(Err::<(), _>(error)),
        Refusal::NoAnswer { seconds: 1 }
    );
    assert!(
        message.contains("did not get to this change within 1 second"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");

    // The app gets to it later, learns that the tool is gone, and does nothing.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(*began.lock().unwrap(), [false]);
    assert_eq!(app.names(), [] as [&str; 0]);
    assert_eq!(fx.dump(), before);
}

#[test]
fn an_app_that_takes_a_change_up_and_does_not_answer_leaves_it_open_whether_it_was_made() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let app = StandIn::open_with(
        &fx,
        Box::new(move |store, incoming| {
            if incoming.request().is_flush() {
                return as_the_app(store, incoming);
            }
            assert!(incoming.begin(), "taken up in time");
            // And then it takes long.
            std::thread::sleep(Duration::from_millis(600));
            as_the_app(store, incoming)
        }),
    );
    let impatient = fx
        .library
        .clone()
        .with_app_patience(Duration::from_millis(150));
    let started = Instant::now();
    let error = impatient
        .edit(&edit(&impatient, chapter.id, "Text", "Words"))
        .unwrap_err();
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(280) && took < Duration::from_millis(550),
        "its patience, and as much again for the answer: {took:?}"
    );
    let message = error.to_string();
    let why = refusal(Err::<(), _>(error));
    assert_eq!(why, Refusal::Unanswered { seconds: 1 });
    assert!(!why.nothing_changed());
    assert!(!message.contains("nothing was changed"), "{message}");
    assert!(
        message.contains("Whether the change was made is not known: read the document again"),
        "{message}"
    );
    // And indeed it was made.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(app.changes(), ["edit"]);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");
}

#[test]
fn an_answer_that_is_cut_off_or_cannot_be_read_is_not_taken_for_one() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let before = fx.bytes();
    let change = |library: &Library| library.edit(&edit(library, chapter.id, "Text", "Words"));
    let cases: Vec<(&str, &[u8])> = vec![
        ("cut off", br#"{"version":1,"answer":"done","resu"#),
        ("nothing at all", b""),
        ("not JSON", b"well done\n"),
        ("no answer in it", b"{\"version\":1}\n"),
        (
            "a kind of answer from the future",
            b"{\"version\":9,\"answer\":\"postponed\",\"until\":\"tomorrow\"}\n",
        ),
    ];
    for (name, bytes) in cases {
        let bytes = bytes.to_vec();
        let app = RawApp::open(&fx, move |_| bytes.clone());
        let error = change(&fx.library).unwrap_err();
        let message = error.to_string();
        let why = refusal(Err::<(), _>(error));
        assert_eq!(why, Refusal::AnswerLost, "{name}");
        assert!(!why.nothing_changed());
        assert!(
            message.contains("its answer did not arrive. Whether the change was made is not known"),
            "{name}: {message}"
        );
        // What it was sent is one line: a request of this version.
        let line = app.requests.try_iter().last().expect("a request");
        assert!(line.ends_with('\n') && line.trim_end().lines().count() == 1);
        let request: Request = serde_json::from_str(&line).unwrap();
        assert_eq!(request.name(), "edit");
        assert_eq!(request.version, live::PROTOCOL_VERSION);
        assert_eq!(request.wait_ms, 10_000);
    }

    // An answer that says "done" with a result that is none: the change was made, and
    // the error says so.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":1,\"answer\":\"done\",\"result\":{\"nonsense\":true}}\n".to_vec()
    });
    match change(&fx.library) {
        Err(error @ Error::InApp { .. }) => {
            assert_eq!(error.kind(), ErrorKind::Failed);
            let message = error.to_string();
            assert!(message.contains("made the change"), "{message}");
            assert!(message.contains("Read the document again"), "{message}");
        }
        other => panic!("{other:?}"),
    }
    drop(app);
    // A failure the app reports in a kind this tool does not know keeps its message.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":1,\"answer\":\"failed\",\"error\":{\"code\":\"moon_phase\",\
          \"message\":\"The moon is wrong.\"}}\n"
            .to_vec()
    });
    match change(&fx.library) {
        Err(error @ Error::InApp { .. }) => {
            assert_eq!(error.kind(), ErrorKind::Failed);
            assert_eq!(error.to_string(), "The moon is wrong.");
        }
        other => panic!("{other:?}"),
    }
    drop(app);
    assert_eq!(fx.bytes(), before, "none of them wrote anything");
}

#[test]
fn versions_of_the_channel_that_do_not_fit_tell_each_other() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let change = |library: &Library| library.edit(&edit(library, chapter.id, "Text", "Words"));

    // A newer app refuses this tool's request, in a form every version reads.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":7,\"answer\":\"refused\",\"refusal\":{\"why\":\"protocol\",\"app\":7,\
          \"tool\":3},\"more\":[1,2]}\n"
            .to_vec()
    });
    let error = change(&fx.library).unwrap_err();
    let message = error.to_string();
    assert_eq!(
        refusal(Err::<(), _>(error)),
        Refusal::Protocol { app: 7, tool: 3 }
    );
    assert!(
        message.contains("the app speaks version 7 of their channel, this tool version 3"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
    assert!(message.contains("Contents/MacOS/writer"), "{message}");
    drop(app);
    // A reason this tool has never heard of is a refusal all the same.
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":7,\"answer\":\"refused\",\"refusal\":{\"why\":\"full_moon\"}}\n".to_vec()
    });
    assert_eq!(refusal(change(&fx.library)), Refusal::Unknown);
    drop(app);
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":7,\"answer\":\"refused\"}\n".to_vec()
    });
    assert_eq!(refusal(change(&fx.library)), Refusal::Unknown);
    drop(app);

    // The other way round: a tool from the future asks this app. The listener answers
    // by itself, hands nothing to the app, and goes on.
    let app = StandIn::open(&fx);
    let ask = |line: &str| -> serde_json::Value {
        let mut stream = UnixStream::connect(&app.socket).unwrap();
        stream.write_all(line.as_bytes()).unwrap();
        stream.write_all(b"\n").unwrap();
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer).unwrap();
        serde_json::from_str(&answer).unwrap_or_else(|_| panic!("{answer:?}"))
    };
    let answer = ask(r#"{"version":4,"op":"teleport","request":{"document":3,"to":"Mars"}}"#);
    assert_eq!(
        answer,
        serde_json::json!({
            "version": 3,
            "answer": "refused",
            "refusal": {"why": "protocol", "app": 3, "tool": 4},
        })
    );
    // And a tool from before images (0.6.0 spoke version 1): its requests bring no
    // image files, and an app that took one would store a link where the agent meant a
    // picture. It is refused whole, a request it could have read and a `flush` alike.
    for line in [
        format!(
            r#"{{"version":1,"wait_ms":10000,"op":"edit","request":{{"document":"doc:{}","old_text":"Text","new_text":"Words"}}}}"#,
            chapter.id
        ),
        r#"{"version":1,"wait_ms":1000,"op":"flush"}"#.to_string(),
    ] {
        assert_eq!(
            ask(&line),
            serde_json::json!({
                "version": 3,
                "answer": "refused",
                "refusal": {"why": "protocol", "app": 3, "tool": 1},
            }),
            "{line}"
        );
    }
    // And a tool from before pages (0.7.0 spoke version 2): it would not know a page
    // the app may hand it in an answer, and an app of version 2 would not know a page's
    // request. Refused whole too.
    for line in [
        format!(
            r#"{{"version":2,"wait_ms":10000,"op":"edit","request":{{"document":"doc:{}","old_text":"Text","new_text":"Words"}}}}"#,
            chapter.id
        ),
        r#"{"version":2,"wait_ms":1000,"op":"flush"}"#.to_string(),
    ] {
        assert_eq!(
            ask(&line),
            serde_json::json!({
                "version": 3,
                "answer": "refused",
                "refusal": {"why": "protocol", "app": 3, "tool": 2},
            }),
            "{line}"
        );
    }
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nText.\n");
    app.names();
    // What is no request is answered as a failure.
    for line in ["nonsense", "{}", r#"{"version":3,"op":"sing"}"#] {
        let answer = ask(line);
        assert_eq!(answer["answer"], "failed", "{line}");
        assert_eq!(answer["error"]["code"], "invalid", "{line}");
    }
    assert_eq!(app.names(), [] as [&str; 0], "none of it reached the app");
    // A request as the documentation writes it is served.
    let answer = ask(&format!(
        r#"{{"version":3,"wait_ms":10000,"op":"edit","request":{{"document":"doc:{}","old_text":"Text","new_text":"Words","author":"Claude"}}}}"#,
        chapter.id
    ));
    assert_eq!(answer["version"], 3);
    assert_eq!(answer["answer"], "done");
    assert_eq!(answer["result"]["replacements"], 1);
    assert_eq!(answer["result"]["changed"], true);
    assert_eq!(app.changes(), ["edit"]);
    let answer = ask(r#"{"version":3,"wait_ms":1000,"op":"flush"}"#);
    assert_eq!(
        answer["result"],
        serde_json::json!({"takes_changes": true, "app_version": env!("CARGO_PKG_VERSION")})
    );
    let answer = ask(r#"{"version":3,"wait_ms":1000,"op":"hello"}"#);
    assert_eq!(answer["result"]["takes_changes"], true);
    // A failure travels with its kind and comes out as the error it was.
    let answer = ask(&format!(
        r#"{{"version":3,"wait_ms":10000,"op":"edit","request":{{"document":{},"old_text":"gone","new_text":"x"}}}}"#,
        chapter.id
    ));
    assert_eq!(answer["answer"], "failed");
    assert_eq!(answer["error"]["code"], "no_match");
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");
}

/// A request that brings a picture, and an app from before pictures (0.6.0, which speaks
/// version 1 of the channel). That app reads past fields it does not know: had the
/// version stayed 1 it would have taken the request, stored a link where the agent meant
/// an image, and answered "done". As it is, it answers what it answers to every line of
/// another version, and nothing is half applied.
#[test]
fn an_app_from_before_images_refuses_a_request_that_brings_one_whole() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let file = fx.picture("harbour.png", (64, 40), [30, 90, 160]);
    let before = fx.dump();
    // What the listener of 0.6.0 writes for a request of version 3 (its `parse_request`
    // compares the number first and reads nothing else of the line).
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":1,\"answer\":\"refused\",\"refusal\":{\"why\":\"protocol\",\"app\":1,\
          \"tool\":3}}\n"
            .to_vec()
    });
    let added = fx.library.add_image(&AddImageRequest {
        document: chapter.id.into(),
        path: file.to_string_lossy().into_owned(),
        description: "The harbour".into(),
        write: claude(),
        ..Default::default()
    });
    let in_markdown = fx.library.insert(&InsertRequest {
        document: chapter.id.into(),
        markdown: format!("![The harbour]({})", file.display()),
        write: claude(),
        ..Default::default()
    });
    for result in [added, in_markdown] {
        let error = result.unwrap_err();
        assert_eq!(error.exit_code(), 5);
        let message = error.to_string();
        assert!(
            message.contains("the app speaks version 1 of their channel, this tool version 3"),
            "{message}"
        );
        assert!(message.contains("nothing was changed"), "{message}");
        assert!(message.contains("Contents/MacOS/writer"), "{message}");
        let report = error.report();
        assert_eq!(report.nothing_changed, Some(true));
        assert_eq!(
            refusal(Err::<(), _>(error)),
            Refusal::Protocol { app: 1, tool: 3 }
        );
    }
    // Both requests said which version they are, and brought the bytes of the file.
    let lines: Vec<serde_json::Value> = app
        .requests
        .try_iter()
        .map(|line| serde_json::from_str(&line).unwrap())
        .collect();
    assert_eq!(lines.len(), 2);
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.encode(std::fs::read(&file).unwrap());
    for line in &lines {
        assert_eq!(line["version"], 3);
        assert_eq!(line["request"]["images"][0]["data"], bytes.as_str());
    }
    assert_eq!(lines[0]["op"], "add_image");
    assert_eq!(lines[1]["op"], "insert");
    drop(app);
    assert_eq!(fx.dump(), before, "refused, not half applied");
    assert!(!fx.images_path().exists());
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nText.\n");
}

/// A page and an app from before pages (0.7.0, which speaks version 2 of the channel):
/// that app does not know `add_page`, and refuses every line of another version whole.
/// The page the tool prepared went in the line, with its files.
#[test]
fn an_app_from_before_pages_refuses_a_page_whole() {
    let fx = closed_fixture();
    fx.create(None, "# Chapter 1\n\nText.\n");
    let folder = fx.page_folder(
        "page",
        &[(
            "index.html",
            "<!doctype html><title>Old app</title><p>Hello.</p>",
        )],
    );
    let before = fx.dump();
    let app = RawApp::open(&fx, |_| {
        b"{\"version\":2,\"answer\":\"refused\",\"refusal\":{\"why\":\"protocol\",\"app\":2,\
          \"tool\":3}}\n"
            .to_vec()
    });
    let error = fx
        .library
        .add_page(&AddPageRequest {
            path: folder.to_string_lossy().into_owned(),
            write: claude(),
            ..Default::default()
        })
        .unwrap_err();
    assert_eq!(error.exit_code(), 5);
    let message = error.to_string();
    assert!(
        message.contains("the app speaks version 2 of their channel, this tool version 3"),
        "{message}"
    );
    assert!(message.contains("nothing was changed"), "{message}");
    assert_eq!(error.report().nothing_changed, Some(true));
    assert_eq!(
        refusal(Err::<(), _>(error)),
        Refusal::Protocol { app: 2, tool: 3 }
    );
    let lines: Vec<serde_json::Value> = app
        .requests
        .try_iter()
        .map(|line| serde_json::from_str(&line).unwrap())
        .collect();
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0]["version"], 3);
    assert_eq!(lines[0]["op"], "add_page");
    let page = &lines[0]["request"]["prepared"];
    assert_eq!(page["state"], "page");
    use base64::Engine;
    let html = std::fs::read(folder.join("index.html")).unwrap();
    let blob = page["blobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|blob| blob["file"] == writer_page::file_id_hex(&html))
        .expect("the page's file is in the line");
    assert_eq!(
        blob["data"],
        base64::engine::general_purpose::STANDARD.encode(&html)
    );
    assert!(blob.get("stored").is_none(), "{blob}");
    drop(app);
    assert_eq!(fx.dump(), before, "refused, not half applied");
    assert!(!fx.images_path().exists());
}

/// The largest page there can be, in one request to the app, in a build without
/// optimisations: a line of more than 80 megabytes is written before the connection is
/// made, read and taken apart on the listener's thread, and the app stores the files it
/// was handed.
#[test]
fn a_page_as_large_as_a_page_can_be_goes_through_the_channel() {
    let fx = closed_fixture();
    fx.create(None, "# Chapter 1\n\nText.\n");
    // Two files a little under 32 MB and the page that names them: as near to the 64 MB
    // a page may hold as two files can come.
    const SIZE: usize = 32 * 1024 * 1024 - 64 * 1024;
    let mut files = Vec::new();
    for (name, seed) in [("a.bin", 1u8), ("b.bin", 2u8)] {
        let bytes: Vec<u8> = (0..SIZE)
            .map(|at| (at as u8).wrapping_mul(31).wrapping_add(seed))
            .collect();
        files.push((name, bytes));
    }
    let folder = fx.page_folder(
        "large page",
        &[(
            "index.html",
            "<!doctype html><title>Large</title><a href=a.bin>a</a> <a href=b.bin>b</a>",
        )],
    );
    for (name, bytes) in &files {
        std::fs::write(folder.join(name), bytes).unwrap();
    }
    let app = StandIn::open(&fx);
    app.names();
    // Measured in this build on this Mac: about 9 s to prepare the page and 8 s for the
    // rest (the line written, read and taken apart, the files hashed and stored with a
    // sync), the app's part well inside the 13 s a tool waits for its answer. The suite
    // runs its tests side by side, so this one is given room.
    let library = fx
        .library
        .clone()
        .with_app_patience(Duration::from_secs(30));
    // What the tool does before it sends anything: the preparing of the page.
    let started = Instant::now();
    library
        .add_page(&AddPageRequest {
            path: folder.to_string_lossy().into_owned(),
            write: super::rules::dry(),
            ..Default::default()
        })
        .unwrap();
    let preparing = started.elapsed();
    let started = Instant::now();
    let added = library
        .add_page(&AddPageRequest {
            path: folder.to_string_lossy().into_owned(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let took = started.elapsed();
    assert!(added.written.changed && added.written.app_is_open);
    assert_eq!(added.files, 3);
    assert!(added.bytes > 63 * 1024 * 1024, "{}", added.bytes);
    assert!(
        added.bytes <= writer_page::MAX_PAGE_BYTES,
        "{}",
        added.bytes
    );
    assert_eq!(app.changes(), ["add_page"]);
    let store = fx.store();
    for (_, bytes) in &files {
        let id = writer_store::FileId::of(bytes);
        assert!(
            store.file_bytes(&id).unwrap().as_deref() == Some(bytes.as_slice()),
            "the app stored other bytes than the file has"
        );
    }
    eprintln!(
        "a page of {} bytes through the channel: {took:?}, of which preparing it {preparing:?}",
        added.bytes
    );
}

/// The largest picture the library holds, in one request to the app: a line of more
/// than twenty megabytes goes over the socket, is read and taken apart on the listener's
/// thread, and the app stores the bytes it was handed.
#[test]
fn a_picture_as_large_as_the_library_holds_goes_through_the_channel() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let app = StandIn::open(&fx);
    app.names();
    // 2,330 pixels square, three bytes each, stored as they are: a little under 16 MB.
    let file = fx.dir.path().join("large.png");
    std::fs::write(&file, super::stored_png(2330, 2330, [120, 30, 200])).unwrap();
    let bytes = std::fs::read(&file).unwrap();
    assert!(
        bytes.len() > 15 * 1024 * 1024 && bytes.len() <= 16 * 1024 * 1024,
        "{}",
        bytes.len()
    );
    let started = Instant::now();
    let added = fx
        .library
        .add_image(&AddImageRequest {
            document: chapter.id.into(),
            path: file.to_string_lossy().into_owned(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    let took = started.elapsed();
    assert!(added.written.changed && added.written.app_is_open);
    let image = added.image.expect("the image that was added");
    assert_eq!(image.pixels, (2330, 2330));
    assert_eq!(app.changes(), ["add_image"]);
    let id = writer_store::ImageId::parse(&image.id).unwrap();
    assert_eq!(
        id,
        writer_store::ImageId::of(&bytes),
        "a PNG is stored as it came"
    );
    assert!(
        fx.store().image_bytes(&id).unwrap().as_deref() == Some(bytes.as_slice()),
        "the app stored other bytes than the file has"
    );
    // (Unoptimised, on a busy machine: the tool waits ten seconds for the app.)
    assert!(took < Duration::from_secs(8), "{took:?}");
    eprintln!(
        "a picture of {} bytes through the channel: {took:?}",
        bytes.len()
    );
}

#[test]
fn two_agents_at_once_are_served_one_after_the_other_and_none_is_lost() {
    let fx = closed_fixture();
    let shared = fx.create(Some("Book"), "# Shared\n\nStart.\n");
    let app = StandIn::open(&fx);
    const AGENTS: usize = 6;
    const ROUNDS: usize = 5;
    let agents: Vec<_> = (0..AGENTS)
        .map(|agent| {
            let library = fx.library.clone();
            std::thread::spawn(move || {
                let name = format!("Agent {agent}");
                let own = library
                    .create_document(&CreateDocumentRequest {
                        markdown: format!("# Own {agent}\n\nMine.\n"),
                        folder: Some(Target::path("Book")),
                        write: WriteOptions::by(&name),
                        ..Default::default()
                    })
                    .unwrap()
                    .document;
                for round in 0..ROUNDS {
                    // Everybody writes into the same document,
                    let done = library
                        .insert(&InsertRequest {
                            document: Target::path("Book/Shared"),
                            markdown: format!("Line {agent}-{round}."),
                            write: WriteOptions::by(&name),
                            ..Default::default()
                        })
                        .unwrap();
                    assert!(done.written.changed);
                    // and into its own, and reads in between.
                    library
                        .insert(&InsertRequest {
                            document: own.id.into(),
                            markdown: format!("Round {round}."),
                            write: WriteOptions::by(&name),
                            ..Default::default()
                        })
                        .unwrap();
                    let text = library
                        .read(&ReadRequest {
                            document: own.id.into(),
                            ..Default::default()
                        })
                        .unwrap();
                    assert!(text.content.contains(&format!("Round {round}.")));
                }
                own.id
            })
        })
        .collect();
    let own: Vec<DocumentId> = agents
        .into_iter()
        .map(|agent| agent.join().expect("no agent failed"))
        .collect();

    // Nothing was lost: every line is in the shared document, once.
    let text = fx.markdown(shared.id);
    for (agent, own) in own.iter().enumerate() {
        for round in 0..ROUNDS {
            let line = format!("Line {agent}-{round}.");
            assert_eq!(text.matches(&line).count(), 1, "{line} in {text}");
        }
        let text = fx.markdown(*own);
        assert_eq!(text.matches("Round ").count(), ROUNDS, "{text}");
    }
    // Each change was a request of its own, carried out whole before the next.
    let served: Vec<Served> = app.taken();
    let changes: Vec<&Served> = served
        .iter()
        .filter(|served| !served.request.is_flush())
        .collect();
    assert_eq!(changes.len(), AGENTS * (1 + 2 * ROUNDS));
    assert!(changes.iter().all(|served| served.answer.is_done()));
    let kept = fx
        .library
        .versions(&ListVersionsRequest {
            document: shared.id.into(),
        })
        .unwrap()
        .versions;
    assert_eq!(kept.len(), AGENTS * ROUNDS, "a version before every change");
    for agent in 0..AGENTS {
        let label = format!("Before an insertion by Agent {agent}");
        assert_eq!(
            kept.iter()
                .filter(|version| version.label.as_deref() == Some(label.as_str()))
                .count(),
            ROUNDS
        );
    }
    fx.document(shared.id).check_invariants().unwrap();
}

#[test]
fn a_data_directory_with_a_very_long_path_is_found_all_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let mut data = dir.path().to_path_buf();
    for name in ["a", "b", "c"] {
        data.push(name.repeat(80));
    }
    data.push("The Writer");
    assert!(data.as_os_str().len() > 280, "{}", data.display());
    let clock = writer_agent::Clock::manual(super::START);
    let fx = Fixture {
        app: None,
        library: Library::at(&data).with_clock(clock.clone()),
        clock,
        dir,
    };
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let app = StandIn::open(&fx);
    // A socket has room for about a hundred bytes: it is not in the data directory.
    assert!(
        app.socket.as_os_str().len() < 104,
        "{}",
        app.socket.display()
    );
    assert!(!app.socket.starts_with(&data));
    assert_eq!(
        instance_lock::app_channel(&data).unwrap().unwrap().path,
        app.socket
    );
    // Only the user gets at it.
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    let socket = std::fs::metadata(&app.socket).unwrap();
    assert!(socket.file_type().is_socket());
    assert_eq!(socket.permissions().mode() & 0o077, 0);
    let folder = std::fs::metadata(app.socket.parent().unwrap()).unwrap();
    assert_eq!(folder.permissions().mode() & 0o777, 0o700);

    let done = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert!(done.written.changed);
    assert_eq!(app.changes(), ["edit"]);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");
}

#[test]
fn an_app_that_quits_with_a_change_in_its_hands_is_waited_for_and_the_change_is_written() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    // The app quits: what it has not begun is answered "quitting" (here by dropping
    // the request), and a moment later its lock is free.
    let lock: Arc<Mutex<Option<HeldLock>>> = Arc::new(Mutex::new(None));
    let app = StandIn::open_with(&fx, {
        let lock = lock.clone();
        Box::new(move |store, incoming| {
            if incoming.request().is_flush() {
                return as_the_app(store, incoming);
            }
            drop(incoming);
            let lock = lock.lock().unwrap().clone().expect("the lock");
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                lock.lock().unwrap().take();
            });
            None
        })
    });
    *lock.lock().unwrap() = Some(app.lock());
    let started = Instant::now();
    let done = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert!(done.written.changed);
    assert!(started.elapsed() >= Duration::from_millis(140));
    assert!(!fx.library.app_is_open());
    assert_eq!(app.names(), [] as [&str; 0], "the tool wrote it itself");
    drop(app);
    assert_eq!(fx.markdown(chapter.id), "# Chapter 1\n\nWords.\n");

    // One that says it quits and stays is not waited for for ever.
    let app = StandIn::open_with(
        &fx,
        Box::new(move |store, incoming| {
            if incoming.request().is_flush() {
                return as_the_app(store, incoming);
            }
            incoming.refuse(Refusal::Quitting).ok();
            None
        }),
    );
    let before = fx.dump();
    let impatient = fx
        .library
        .clone()
        .with_app_patience(Duration::from_millis(200));
    let error = impatient
        .edit(&edit(&impatient, chapter.id, "Words", "Lines"))
        .unwrap_err();
    let message = error.to_string();
    assert_eq!(refusal(Err::<(), _>(error)), Refusal::Quitting);
    assert!(message.contains("is quitting"), "{message}");
    assert!(message.contains("nothing was changed"), "{message}");
    assert_eq!(fx.dump(), before);
    drop(app);
}

#[test]
fn what_the_app_refuses_reaches_the_agent_in_words() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let refusals = [
        (Refusal::Composing, "in the middle of typing a character"),
        (
            Refusal::CouldNotSave {
                message: "the disk is full".into(),
            },
            "could not save the open document before the change (the disk is full)",
        ),
        (Refusal::NoLibrary, "shows no window with this library"),
    ];
    let next = Arc::new(Mutex::new(None::<Refusal>));
    let app = StandIn::open_with(&fx, {
        let next = next.clone();
        Box::new(move |store, incoming| {
            if incoming.request().is_flush() {
                return as_the_app(store, incoming);
            }
            // The app looked at its window before it began, and refuses.
            let refusal = next.lock().unwrap().clone().expect("a refusal");
            incoming.refuse(refusal).ok();
            None
        })
    });
    let before = fx.dump();
    for (why, words) in refusals {
        *next.lock().unwrap() = Some(why.clone());
        let error = fx
            .library
            .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
            .unwrap_err();
        let message = error.to_string();
        assert_eq!(refusal(Err::<(), _>(error)), why);
        assert!(message.contains(words), "{message}");
        assert!(message.contains("nothing was changed"), "{message}");
        assert!(
            message.starts_with(&format!(
                "The Writer is open (process {})",
                std::process::id()
            )),
            "{message}"
        );
    }
    assert_eq!(fx.dump(), before);
    drop(app);
}

#[test]
fn closing_tells_those_who_wait_and_leaves_nothing_behind() {
    let fx = closed_fixture();
    fx.create(None, "# Chapter 1\n\nText.\n");
    let _lock = instance_lock::acquire(&fx.data_dir())
        .unwrap()
        .lock()
        .unwrap();
    let listener = Listener::bind(&fx.data_dir()).unwrap();
    let socket = listener.path().to_path_buf();
    let closer = listener.closer();
    assert!(!closer.is_closed());
    // Two tools have connected and sent their requests; the app has not got to them.
    let waiting: Vec<UnixStream> = (0..2)
        .map(|_| {
            let mut stream = UnixStream::connect(&socket).unwrap();
            stream
                .write_all(
                    format!(
                        "{{\"version\":{PROTOCOL_VERSION},\"wait_ms\":10000,\"op\":\"flush\"}}\n"
                    )
                    .as_bytes(),
                )
                .unwrap();
            stream
        })
        .collect();
    // And one request is in the app's hands, not begun.
    let accepting = std::thread::spawn(move || {
        let first = listener.accept();
        let second = listener.accept();
        (first, second, listener)
    });
    std::thread::sleep(Duration::from_millis(100));

    let started = Instant::now();
    closer.close();
    closer.close();
    assert!(started.elapsed() < Duration::from_millis(100), "quick");
    assert!(closer.is_closed());
    assert!(!socket.exists());
    assert_eq!(instance_lock::app_channel(&fx.data_dir()).unwrap(), None);
    assert!(UnixStream::connect(&socket).is_err());

    let (first, second, listener) = accepting.join().unwrap();
    assert!(listener.accept().is_none(), "closed for good");
    // Whatever the app still holds is answered when it lets go of it.
    drop(first);
    drop(second);
    for stream in waiting {
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer).unwrap();
        let answer: serde_json::Value = serde_json::from_str(&answer).unwrap();
        assert_eq!(answer["answer"], "refused", "{answer}");
        assert_eq!(answer["refusal"]["why"], "quitting", "{answer}");
    }
}

#[test]
fn a_connection_that_says_nothing_does_not_keep_the_others_waiting_for_long() {
    let fx = closed_fixture();
    let chapter = fx.create(None, "# Chapter 1\n\nText.\n");
    let app = StandIn::open(&fx);
    // Somebody connects and sends nothing; somebody else sends half a line.
    let silent = UnixStream::connect(&app.socket).unwrap();
    let mut half = UnixStream::connect(&app.socket).unwrap();
    half.write_all(b"{\"version\":1,").unwrap();
    let started = Instant::now();
    let done = fx
        .library
        .edit(&edit(&fx.library, chapter.id, "Text", "Words"))
        .unwrap();
    assert!(done.written.changed);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "a second for each of them: {:?}",
        started.elapsed()
    );
    assert_eq!(app.changes(), ["edit"]);
    drop((silent, half));
}

#[test]
fn a_dry_run_carried_out_by_the_app_writes_nothing() {
    let fx = closed_fixture();
    seed(&fx);
    let mut store = Store::open(fx.library.database_path()).unwrap();
    store.set_clock(fx.clock.clone());
    let before = fx.dump();
    let request = |write: WriteOptions| {
        Request::new(
            Operation::Edit(EditRequest {
                write,
                ..edit(&fx.library, Target::path("Chapter 1"), "at dawn", "at dusk")
            }),
            Duration::from_secs(1),
        )
    };
    let tried = request(dry());
    assert!(tried.is_dry_run() && !tried.changes_library());
    let executed = live::execute(&mut store, &tried);
    assert_eq!(executed.changes, []);
    let Answer::Done { result } = executed.answer else {
        panic!("{:?}", executed.answer);
    };
    assert_eq!(result["dry_run"], true);
    assert_eq!(result["changed"], false);
    assert_eq!(result["would_change"], true);
    assert_eq!(result["app_is_open"], true, "it is the app that says so");
    assert_eq!(result["app_takes_changes"], true);
    assert_eq!(result["replacements"], 1);
    assert_eq!(result["snapshot"], serde_json::Value::Null);
    assert_eq!(fx.dump(), before);

    // With the switch off a dry run still works, and says what the real run would meet.
    store.set_setting(CHANGES_SETTING, &false).unwrap();
    let executed = live::execute(&mut store, &tried);
    let Answer::Done { result } = executed.answer else {
        panic!("{:?}", executed.answer);
    };
    assert_eq!(result["app_takes_changes"], false);
    let refused = live::execute(&mut store, &request(claude()));
    assert_eq!(refused.answer, Answer::refused(Refusal::SwitchedOff));
    assert_eq!(refused.changes, []);
    assert!(!live::changes_allowed(&store));
    store.set_setting(CHANGES_SETTING, &true).unwrap();
    assert!(live::changes_allowed(&store));

    // The connection writes again afterwards: the real run goes through.
    let done = live::execute(&mut store, &request(claude()));
    assert_eq!(done.changes.len(), 2);
    assert_ne!(fx.dump(), before);
    // The result of the real run says who made the change, as the dry run said who would.
    let Answer::Done { result } = done.answer else {
        panic!("{:?}", done.answer);
    };
    assert_eq!(result["dry_run"], false);
    assert_eq!(result["changed"], true);
    assert_eq!(result["app_is_open"], true);
    assert_eq!(result["app_takes_changes"], true);
    // Saving and saying hello change nothing and say whether changes are taken.
    for operation in [Operation::Flush, Operation::Hello] {
        let asked = Request::new(operation, Duration::from_secs(1));
        assert!(!asked.changes_library() && !asked.is_dry_run());
        assert_eq!(asked.author(), None);
        let executed = live::execute(&mut store, &asked);
        assert_eq!(executed.answer, Answer::state(true));
        assert_eq!(executed.changes, []);
    }
}

/// The whole suite once more, with an app that has every library open: every test of an
/// operation that writes passes through the channel as it passes with a direct write.
#[test]
fn the_whole_suite_passes_while_an_app_has_the_library_open() {
    if through_the_app() {
        return;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .env(THROUGH_THE_APP, "1")
        // The tests of this file make their own apps.
        .args(["--skip", "suite::live::"])
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "the suite fails while an app has the library open:\n{said}\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // It did run, and not next to nothing.
    let passed = said
        .lines()
        .find_map(|line| line.strip_prefix("test result: ok. "))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|count| count.parse::<usize>().ok())
        .unwrap_or(0);
    assert!(passed >= 35, "{said}");
}
