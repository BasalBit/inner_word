//! Changes while the app is open: the channel, with a thread standing in for the app.

use std::sync::mpsc;
use std::thread::JoinHandle;

use writer_agent::live::{self, Change, Closer, Listener, Request};
use writer_agent::{EditRequest, ReadRequest, Target};
use writer_store::Store;
use writer_store::instance_lock::{self, InstanceLock};

use super::{Fixture, claude, fixture};

/// A thread that does what the app does with the channel: it holds the lock of the data
/// directory, listens, and carries every request out on a connection of its own.
pub struct StandIn {
    closer: Closer,
    thread: Option<JoinHandle<()>>,
    /// Every request that was carried out, with what it changed.
    pub served: mpsc::Receiver<(Request, Vec<Change>)>,
    _lock: InstanceLock,
}

impl StandIn {
    /// Starts one on the library of `fx`, which has to exist.
    pub fn open(fx: &Fixture) -> StandIn {
        let lock = instance_lock::acquire(&fx.data_dir())
            .unwrap()
            .lock()
            .expect("nobody else has the library");
        let mut store = Store::open(fx.library.database_path()).unwrap();
        store.set_clock(fx.clock.clone());
        let listener = Listener::bind(&fx.data_dir()).unwrap();
        let closer = listener.closer();
        let (tell, served) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            while let Some(incoming) = listener.accept() {
                if !incoming.begin() {
                    continue;
                }
                let executed = live::execute(&mut store, incoming.request());
                tell.send((incoming.request().clone(), executed.changes))
                    .ok();
                incoming.answer(executed.answer).ok();
            }
        });
        StandIn {
            closer,
            thread: Some(thread),
            served,
            _lock: lock,
        }
    }

    /// What was served since the last look, by name.
    pub fn names(&self) -> Vec<&'static str> {
        self.served
            .try_iter()
            .map(|(request, _)| request.name())
            .collect()
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.closer.close();
        if let Some(thread) = self.thread.take() {
            thread.join().ok();
        }
    }
}

#[test]
fn a_change_made_while_the_app_is_open_is_carried_out_by_the_app() {
    let fx = fixture();
    let chapter = fx.create(Some("Book"), "# Chapter 1\n\nThe *Maren* left at dawn.\n");
    let app = StandIn::open(&fx);
    assert!(fx.library.app_is_open());
    let status = fx.library.status().unwrap();
    assert!(status.app_is_open && status.app_takes_changes);

    let edited = fx
        .library
        .edit(&EditRequest {
            document: Target::path("Book/Chapter 1"),
            old_text: "at dawn".into(),
            new_text: "before dawn".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert!(edited.written.changed);
    assert_eq!(edited.replacements, 1);
    assert!(edited.snapshot.is_some());
    let served: Vec<_> = app.served.try_iter().collect();
    let (request, changes) = served.last().expect("the app was asked");
    assert_eq!(request.name(), "edit");
    assert_eq!(request.author(), Some("Claude"));
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
    let text = fx
        .library
        .read(&ReadRequest {
            document: chapter.id.into(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        text.content,
        "# Chapter 1\n\nThe *Maren* left before dawn.\n"
    );
    assert_eq!(app.names(), ["flush"]);

    // The app quits: the tool writes directly again.
    drop(app);
    assert!(!fx.library.app_is_open());
    fx.library
        .edit(&EditRequest {
            document: chapter.id.into(),
            old_text: "before dawn".into(),
            new_text: "at noon".into(),
            write: claude(),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        fx.markdown(chapter.id),
        "# Chapter 1\n\nThe *Maren* left at noon.\n"
    );
}
