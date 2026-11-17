//! Images, as the app sees them: where their bytes are, how a picture
//! comes in, and when bytes that nothing names any more go.
//!
//! An image in a document is a block that names stored bytes by their
//! hash (`innerword_core::ImageId`); the bytes are in `images.db` beside the
//! library database (`innerword-store`). The editor knows neither: it asks an
//! [`innerword_editor::ImageProvider`], and this module is the one the app
//! gives the editor of the document ([`LibraryImages`]).
//!
//! | What | Where |
//! |---|---|
//! | The bytes of an image for the editor, and a picture the user chose, pasted or dropped made into stored bytes | [`LibraryImages`] |
//! | The files "Insert > Image…" asks for | [`use_open_panel`], [`choose_files_beforehand`] |
//! | The bytes for the exporters | [`StoreImages`] |
//! | Bytes nothing names any more; a copy that is behind, a damaged file set aside | [`collect_in_background`] |
//! | The files of a page that is being added, stored off the main thread | [`LibraryImages::put_files`] |
//!
//! A web page dropped on the text (an `.html` file or a folder) is no
//! picture: the provider hands it to the import of pages
//! ([`LibraryImages::set_page_import`]) and takes the rest as pictures.
//!
//! # Nothing large on the main thread
//!
//! For a library on disk the provider has a connection of its own to
//! `images.db` ([`innerword_library::store::ImageFile`]), which only
//! background tasks use: reading the bytes of a picture, and for a new
//! picture the whole of it (reading the file, converting it with
//! [`innerword_image::prepare`], the hash, the write of up to 16 MB with a
//! sync). A library in memory (the tests) has one connection, the
//! store's, and does the same at once.
//!
//! # Bytes first
//!
//! A picture is stored, committed, before a block that names it enters a
//! document: [`ImageProvider::import`] answers with the image only when
//! the write is back. So no save ever names bytes that are not there.
//!
//! # The open panel
//!
//! "Insert > Image…" asks for files in the macOS open panel only in a run
//! that was told to ([`use_open_panel`]: `main`, when the run is not
//! scripted). A test names the files beforehand
//! ([`choose_files_beforehand`]); a scripted run inserts with its own step
//! (`image:<path>`). Without either the panel was "cancelled": no test and
//! no scripted run ever opens a panel.

use std::{
    borrow::Cow,
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, Mutex, PoisonError},
    time::{Duration, Instant},
};

use gpui::{App, AppContext as _, Entity, Global, PathPromptOptions, Task, WeakEntity, Window};
use innerword_core::{Image, ImageId};
use innerword_editor::{ImageProvider, NewImageSource};
use innerword_export::{ImageData, ImageFileFormat, ImageSource};
use innerword_library::{
    ImageFormat, ImageInfo, Library, StoredImageId,
    store::{self, ImageFile, Store},
};
use innerword_ui::{Toast, ToastLayer, toast::ToastId};

use crate::settings::keys;

/// How long after a launch the collector runs: the window is up and the
/// first document is on screen long before.
pub const COLLECT_DELAY: Duration = Duration::from_secs(20);

/// How often the app looks at which images are named while it stays open,
/// and how old the last run of the collector has to be before a launch
/// runs it again.
pub const LOOK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// How many files that could not be used are named one by one; the rest
/// are counted in one more sentence.
pub const REFUSALS_NAMED: usize = 3;

/// How long a sentence about a file that could not be used stays.
const REFUSAL_STAYS: Duration = Duration::from_secs(8);

/// Two imports that are further apart than this are two gestures of the
/// user's: the count of refused files begins again. (Within one gesture
/// the editor asks for the next picture as soon as the one before is
/// stored.)
const SAME_GESTURE: Duration = Duration::from_secs(1);

/// The id of an image as the store has it.
pub fn stored_id(image: ImageId) -> StoredImageId {
    StoredImageId(*image.as_bytes())
}

/// The id of stored bytes as a document has it.
pub fn document_id(image: StoredImageId) -> ImageId {
    ImageId::from_bytes(image.0)
}

/// The image a document holds for stored bytes: their id, their pixel
/// size and the density their file declares. The width and the
/// description are whoever inserts it to set.
pub fn image_of(info: &ImageInfo) -> Image {
    let density = info
        .density
        .clamp(u32::from(Image::MIN_DENSITY), u32::from(Image::MAX_DENSITY));
    Image::new(
        document_id(info.id),
        (info.width.max(1), info.height.max(1)),
        u16::try_from(density).unwrap_or(Image::DEFAULT_DENSITY),
    )
}

/// What a sentence about a picture that could not be used is about: the
/// name of the file in quotes, or the picture on the clipboard.
pub fn subject_of(source: &NewImageSource) -> String {
    match source {
        NewImageSource::File(path) => {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            format!("\u{201c}{name}\u{201d}")
        }
        NewImageSource::Bytes(_) => "The picture on the clipboard".to_string(),
    }
}

/// The sentence for the files that could not be used and are not named
/// one by one.
pub fn more_refused(count: usize) -> String {
    if count == 1 {
        "1 more file could not be used.".to_string()
    } else {
        format!("{count} more files could not be used.")
    }
}

// ----- the open panel -----------------------------------------------------------------

/// Whether "Insert > Image…" may open the macOS open panel, and the files
/// a test named instead.
#[derive(Default)]
struct Chooser {
    panel: bool,
    beforehand: VecDeque<Vec<PathBuf>>,
}

impl Global for Chooser {}

/// From now on "Insert > Image…" asks for files in the macOS open panel.
/// Called by `main` in a launch that is not scripted, and by nothing
/// else.
pub fn use_open_panel(cx: &mut App) {
    cx.default_global::<Chooser>().panel = true;
}

/// Names the files the next "Insert > Image…" takes, instead of asking for
/// them in a panel (once; a second call names the files of the one after).
/// For tests.
pub fn choose_files_beforehand(paths: Vec<PathBuf>, cx: &mut App) {
    cx.default_global::<Chooser>().beforehand.push_back(paths);
}

// ----- the provider -------------------------------------------------------------------

/// The files that could not be used in one gesture: how many, and the
/// sentence that counts those which are not named.
#[derive(Default)]
struct Refusals {
    count: usize,
    more: Option<ToastId>,
    /// When the last import ended.
    last_end: Option<Instant>,
}

/// The [`ImageProvider`] over the library: what the editor of the document
/// is given (see the [module docs](self)).
pub struct LibraryImages {
    library: Entity<Library>,
    /// `images.db` through a connection of its own, which only background
    /// tasks lock; `None` for a library in memory.
    file: Option<Arc<Mutex<ImageFile>>>,
    toasts: WeakEntity<ToastLayer>,
    refusals: Rc<RefCell<Refusals>>,
    /// Who hears that a picture was stored ([`LibraryImages::on_stored`]).
    stored: Rc<RefCell<Option<Stored>>>,
    /// Who takes the web pages dropped on the text
    /// ([`LibraryImages::set_page_import`]), and the ones of the drop that
    /// is coming in.
    pages: Rc<RefCell<PageDrop>>,
    /// A test's stand-in for a store that lost what it was given
    /// ([`LibraryImages::lose_page_files`]).
    lose_page_files: std::cell::Cell<bool>,
}

/// What is called with the web pages of one drop on the text.
type PageImport = Rc<dyn Fn(Vec<PathBuf>, &mut App)>;

/// The web pages of a drop on the text, gathered until the editor has asked
/// for every file of it.
#[derive(Default)]
struct PageDrop {
    import: Option<PageImport>,
    gathered: Vec<PathBuf>,
}

/// What is called when a picture was stored.
type Stored = Rc<dyn Fn(&mut App)>;

impl LibraryImages {
    /// The provider over `library`. What it has to say about a file that
    /// could not be used goes to `toasts`.
    pub fn new(library: Entity<Library>, toasts: &Entity<ToastLayer>, cx: &App) -> Rc<Self> {
        let file = library
            .read(cx)
            .path()
            .map(|database| Arc::new(Mutex::new(ImageFile::beside(database))));
        Rc::new(Self {
            library,
            file,
            toasts: toasts.downgrade(),
            refusals: Rc::default(),
            stored: Rc::default(),
            pages: Rc::default(),
            lose_page_files: std::cell::Cell::new(false),
        })
    }

    /// From now on [`LibraryImages::put_files`] answers that the files are
    /// stored and stores none: what a write in the background that was lost
    /// (a full disk) looks like to the one that writes the page's row. For
    /// tests.
    #[doc(hidden)]
    pub fn lose_page_files(&self, lose: bool) {
        self.lose_page_files.set(lose);
    }

    /// `import` is called with the web pages (`.html` and `.htm` files,
    /// folders) of a drop on the text, instead of their being taken as
    /// pictures: the workspace adds them as pages. The other files of the
    /// drop are pictures, as before.
    pub fn set_page_import(&self, import: impl Fn(Vec<PathBuf>, &mut App) + 'static) {
        self.pages.borrow_mut().import = Some(Rc::new(import));
    }

    /// Whether `source` is a web page the import of pages takes; then it is
    /// gathered with the others of its drop, which go to the import together
    /// once the editor has asked for every file of the drop (it asks for the
    /// next as soon as one is answered).
    fn took_as_a_page(&self, source: &NewImageSource, cx: &mut App) -> bool {
        let NewImageSource::File(path) = source else {
            return false;
        };
        if self.pages.borrow().import.is_none() || !crate::pages::is_page_input(path) {
            return false;
        }
        let first = {
            let mut pages = self.pages.borrow_mut();
            pages.gathered.push(path.clone());
            pages.gathered.len() == 1
        };
        if first {
            let pages = self.pages.clone();
            cx.spawn(async move |cx| {
                // A turn of the main thread: the rest of the drop is asked
                // for meanwhile.
                cx.background_executor().timer(Duration::ZERO).await;
                cx.update(|cx| {
                    let (import, gathered) = {
                        let mut pages = pages.borrow_mut();
                        (pages.import.clone(), std::mem::take(&mut pages.gathered))
                    };
                    if let Some(import) = import {
                        import(gathered, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
        true
    }

    /// Stores the files of a page that is being added (`blobs`: each one's
    /// id as 32 hex digits and its bytes), off the main thread for a library
    /// on disk (the provider's own connection, as for a picture), at once
    /// for one in memory. Each is stored once, after checking that its bytes
    /// are the ones its id names. `Err` is the reason, for the sentence that
    /// says the page was not added.
    pub fn put_files(
        &self,
        blobs: Vec<(String, Vec<u8>)>,
        cx: &mut App,
    ) -> Task<Result<(), String>> {
        fn put_all(
            blobs: Vec<(String, Vec<u8>)>,
            mut put: impl FnMut(&[u8]) -> store::Result<innerword_library::FileInfo>,
        ) -> Result<(), String> {
            for (id, bytes) in blobs {
                let stored = put(&bytes).map_err(|error| error.to_string())?;
                if stored.id.to_hex() != id {
                    return Err("a file of the page is not what its name says".to_string());
                }
            }
            Ok(())
        }
        if self.lose_page_files.get() {
            return Task::ready(Ok(()));
        }
        match &self.file {
            Some(file) => {
                let file = file.clone();
                cx.background_spawn(async move {
                    let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
                    put_all(blobs, |bytes| file.put_file(bytes))
                })
            }
            None => Task::ready(self.library.update(cx, |library, _| {
                put_all(blobs, |bytes| library.put_file(bytes))
            })),
        }
    }

    /// `heard` is called on the main thread whenever a picture was stored
    /// through this provider, before the picture is handed to whoever
    /// asked for it. The workspace tells the editor, which then asks again
    /// for images it had found missing: the same picture inserted a second
    /// time brings the bytes of the first back.
    pub fn on_stored(&self, heard: impl Fn(&mut App) + 'static) {
        *self.stored.borrow_mut() = Some(Rc::new(heard));
    }

    /// The stored bytes of `image`, read now, on this thread: for what is
    /// made at once (a print job of a test or a scripted step). `None`
    /// when the library has no such image.
    pub fn bytes_now(&self, image: ImageId, cx: &App) -> Option<Arc<[u8]>> {
        let name = stored_id(image);
        let bytes = match &self.file {
            Some(file) => read(file, &name),
            None => self
                .library
                .read(cx)
                .store()
                .image_bytes(&name)
                .ok()
                .flatten(),
        };
        bytes.map(Arc::from)
    }

    /// The stored bytes of every image of `images` the library has, read
    /// in the background one after the other (a job of pages reads the
    /// pictures of its documents before it makes the first page), with the
    /// first error of the store a read met ([`ReadPictures::failure`]): a
    /// print draws such a picture as its box, a PDF export fails with it.
    pub fn bytes_of_all(&self, images: Vec<ImageId>, cx: &mut App) -> Task<ReadPictures> {
        match &self.file {
            Some(file) => {
                let file = file.clone();
                cx.background_spawn(async move {
                    ReadPictures::of(images, |name| read_or_fail(&file, name))
                })
            }
            None => {
                let store = self.library.read(cx).store();
                Task::ready(ReadPictures::of(images, |name| store.image_bytes(name)))
            }
        }
    }

    /// Says that a file could not be used: its sentence, for the first
    /// [`REFUSALS_NAMED`] of a gesture, and after them one sentence that
    /// counts the rest.
    fn refuse(
        refusals: &Rc<RefCell<Refusals>>,
        toasts: &WeakEntity<ToastLayer>,
        sentence: String,
        cx: &mut App,
    ) {
        let Some(toasts) = toasts.upgrade() else {
            return;
        };
        let (count, old) = {
            let mut refusals = refusals.borrow_mut();
            refusals.count += 1;
            (refusals.count, refusals.more.take())
        };
        let message = if count <= REFUSALS_NAMED {
            sentence
        } else {
            more_refused(count - REFUSALS_NAMED)
        };
        let id = toasts.update(cx, |layer, cx| {
            if let Some(old) = old {
                layer.dismiss(old, cx);
            }
            layer.push(Toast::new(message).error().duration(REFUSAL_STAYS), cx)
        });
        if count > REFUSALS_NAMED {
            refusals.borrow_mut().more = Some(id);
        }
    }
}

/// Reads the bytes of an image through the provider's own connection.
fn read(file: &Mutex<ImageFile>, name: &StoredImageId) -> Option<Vec<u8>> {
    read_or_fail(file, name).ok().flatten()
}

/// Reads the bytes of an image through the provider's own connection, an
/// error of the store kept apart from an image the library does not have.
fn read_or_fail(file: &Mutex<ImageFile>, name: &StoredImageId) -> store::Result<Option<Vec<u8>>> {
    let file = file.lock().unwrap_or_else(PoisonError::into_inner);
    file.bytes(name)
}

/// The pictures of a job of pages, read beforehand
/// ([`LibraryImages::bytes_of_all`]).
#[derive(Default)]
pub struct ReadPictures {
    /// The bytes of every image the library has.
    pub found: HashMap<ImageId, Arc<[u8]>>,
    /// The first error of the store a read met (`images.db` of a newer
    /// version, busy past its timeout), in words: the job has pictures
    /// missing that the library has. `None` when every read was answered,
    /// a picture the library does not have included.
    pub failure: Option<String>,
}

impl ReadPictures {
    fn of(
        images: Vec<ImageId>,
        mut read: impl FnMut(&StoredImageId) -> store::Result<Option<Vec<u8>>>,
    ) -> Self {
        let mut pictures = Self::default();
        for image in images {
            match read(&stored_id(image)) {
                Ok(Some(bytes)) => {
                    pictures.found.insert(image, Arc::from(bytes));
                }
                Ok(None) => {}
                Err(error) => {
                    pictures.failure.get_or_insert_with(|| error.to_string());
                }
            }
        }
        pictures
    }
}

/// Makes `source` into what the library stores.
fn prepare(source: &NewImageSource) -> Result<innerword_image::Prepared, innerword_image::PrepareError> {
    match source {
        NewImageSource::File(path) => innerword_image::prepare_file(path),
        NewImageSource::Bytes(bytes) => innerword_image::prepare(bytes),
    }
}

/// Why a picture did not come in, as the user is told.
fn not_stored(subject: &str, error: &store::StoreError) -> String {
    format!("{subject} could not be stored in the library: {error}")
}

impl ImageProvider for LibraryImages {
    fn bytes(&self, image: ImageId, cx: &mut App) -> Task<Option<Arc<[u8]>>> {
        match &self.file {
            Some(file) => {
                let file = file.clone();
                let name = stored_id(image);
                cx.background_spawn(async move { read(&file, &name).map(Arc::from) })
            }
            None => Task::ready(self.bytes_now(image, cx)),
        }
    }

    fn import(&self, source: NewImageSource, cx: &mut App) -> Task<Option<Image>> {
        // A web page is no picture: the import of pages takes it.
        if self.took_as_a_page(&source, cx) {
            return Task::ready(None);
        }
        // A new gesture, or one more picture of the same one?
        let now = cx.background_executor().now();
        {
            let mut refusals = self.refusals.borrow_mut();
            let same = refusals
                .last_end
                .is_some_and(|end| now.saturating_duration_since(end) <= SAME_GESTURE);
            if !same {
                refusals.count = 0;
                refusals.more = None;
            }
        }
        let subject = subject_of(&source);
        // Reading, converting, hashing and writing: all of it away from
        // the main thread, for a library on disk.
        let stored: Task<Result<ImageInfo, String>> = match &self.file {
            Some(file) => {
                let file = file.clone();
                let subject = subject.clone();
                cx.background_spawn(async move {
                    let prepared = prepare(&source).map_err(|error| error.sentence(&subject))?;
                    let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
                    file.put(&prepared.as_new_image())
                        .map_err(|error| not_stored(&subject, &error))
                })
            }
            None => Task::ready(
                prepare(&source)
                    .map_err(|error| error.sentence(&subject))
                    .and_then(|prepared| {
                        self.library
                            .update(cx, |library, _| library.put_image(&prepared.as_new_image()))
                            .map_err(|error| not_stored(&subject, &error))
                    }),
            ),
        };
        let refusals = self.refusals.clone();
        let toasts = self.toasts.clone();
        let heard = self.stored.clone();
        cx.spawn(async move |cx| {
            let stored = stored.await;
            cx.update(|cx| {
                refusals.borrow_mut().last_end = Some(cx.background_executor().now());
                match stored {
                    // The bytes are committed: now a block may name them.
                    Ok(info) => {
                        let heard = heard.borrow().clone();
                        if let Some(heard) = heard {
                            heard(cx);
                        }
                        Some(image_of(&info))
                    }
                    Err(sentence) => {
                        Self::refuse(&refusals, &toasts, sentence, cx);
                        None
                    }
                }
            })
            .ok()
            .flatten()
        })
    }

    fn choose(&self, _: &mut Window, cx: &mut App) -> Task<Option<Vec<PathBuf>>> {
        let chooser = cx.default_global::<Chooser>();
        if let Some(paths) = chooser.beforehand.pop_front() {
            return Task::ready(Some(paths).filter(|paths| !paths.is_empty()));
        }
        if !chooser.panel {
            return Task::ready(None);
        }
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Insert".into()),
        });
        let toasts = self.toasts.clone();
        cx.spawn(async move |cx| match receiver.await {
            Ok(Ok(paths)) => paths.filter(|paths| !paths.is_empty()),
            Ok(Err(error)) => {
                toasts
                    .update(cx, |layer, cx| {
                        layer.push(
                            Toast::new(format!("Could not open the file panel: {error}"))
                                .error()
                                .duration(REFUSAL_STAYS),
                            cx,
                        );
                    })
                    .ok();
                None
            }
            // The panel went away without an answer.
            Err(_) => None,
        })
    }

    fn pasteboard(&self, cx: &mut App) -> Option<NewImageSource> {
        crate::pasteboard::image(cx)
    }

    fn touch(&self, images: &[ImageId], cx: &mut App) {
        self.library.update(cx, |library, _| {
            for image in images {
                // An image the library does not have shows as missing;
                // there is nothing to say about it here.
                library.touch_image(&stored_id(*image)).ok();
            }
        });
    }
}

// ----- the exporters ------------------------------------------------------------------

/// The pictures of the library for the exporters
/// ([`innerword_export::ExportSource::with_images`]). An image the library
/// does not have is exported as a missing one: its description stands in
/// for it.
///
/// An image that could not be read (`images.db` of a newer version, busy
/// past its timeout, not to be opened) is not a missing one: the first
/// such error is kept ([`StoreImages::failure`]), and the export fails
/// with it instead of writing a file without its pictures. (A damaged file
/// is no such error: the store reads the copy then.)
pub struct StoreImages<'a> {
    store: &'a Store,
    failed: RefCell<Option<store::StoreError>>,
}

impl<'a> StoreImages<'a> {
    /// The pictures of `store`.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            failed: RefCell::new(None),
        }
    }

    /// The first error of the store a picture met, if one did: the
    /// export that asked for it has pictures missing that the library
    /// has.
    pub fn failure(&self) -> Option<String> {
        self.failed.borrow().as_ref().map(ToString::to_string)
    }

    fn keep<T>(&self, read: store::Result<Option<T>>) -> Option<T> {
        match read {
            Ok(found) => found,
            Err(error) => {
                self.failed.borrow_mut().get_or_insert(error);
                None
            }
        }
    }
}

impl ImageSource for StoreImages<'_> {
    fn image(&self, id: ImageId) -> Option<ImageData<'_>> {
        let name = stored_id(id);
        let info = self.keep(self.store.image_info(&name))?;
        let bytes = self.keep(self.store.image_bytes(&name))?;
        Some(ImageData {
            bytes: Cow::Owned(bytes),
            format: match info.format {
                ImageFormat::Png => ImageFileFormat::Png,
                ImageFormat::Jpeg => ImageFileFormat::Jpeg,
                ImageFormat::Gif => ImageFileFormat::Gif,
            },
        })
    }
}

// ----- the collector ------------------------------------------------------------------

/// Sees to the bytes that nothing names any more, for a library on disk
/// and for as long as the app runs. Called once, by `main`.
///
/// - [`COLLECT_DELAY`] after the launch, when the last run is a day old
///   or older (the setting `images.collected_at`, unix milliseconds): the
///   collector (`innerword_store::collect_unused_images`). It is **the only
///   thing that ever deletes the bytes of an image**, and only of one that
///   no document (live or in the trash) and no version has named for
///   thirty days and that no kept backup of the library can name. It runs
///   at a launch because then nothing but stored content can hold an id:
///   the undo history, a Vim register and a cut that waits to be pasted
///   are empty. In a launch that does not run it, the copy of the
///   pictures among the backups is brought up to date instead
///   (`innerword_store::back_up_all_images`; the collector ends with the
///   same).
/// - From then on, every [`LOOK_EVERY`]: the looking half alone
///   (`innerword_store::mark_unused_images`). It deletes nothing. It keeps
///   the stamp "unnamed since" from growing old on an image that is named
///   again without the store being asked (an undo brings back an image
///   that was cut and saved away).
///
/// What the writer is told, after each run: a copy of the pictures that
/// could not be brought up to date ([`copy_not_updated`], once a launch),
/// and each damaged file of pictures the store set aside
/// ([`moved_aside_note`], once for each file: the setting
/// `images.moved_aside_told`), with "Show in Finder". A file the store set
/// aside in between, when a read found `images.db` damaged, is told of at
/// the next run.
///
/// Everything runs on the background executor with connections of its
/// own; the timers are the app's, so a test drives them with its clock.
pub fn collect_in_background(library: &Entity<Library>, cx: &mut App) {
    let (database, clock) = {
        let library = library.read(cx);
        let Some(database) = library.path() else {
            return;
        };
        (database.to_path_buf(), library.clock().clone())
    };
    let library = library.downgrade();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(COLLECT_DELAY).await;
        let Ok(last) = library.read_with(cx, |library, _| {
            library.setting_or(keys::IMAGES_COLLECTED_AT, 0i64)
        }) else {
            return;
        };
        let now = clock.now_ms();
        let day = i64::try_from(LOOK_EVERY.as_millis()).unwrap_or(i64::MAX);
        // A time in the future is a clock that was set back: run.
        let due = last > now || now - last >= day;
        // What the run did, and why the copy of the pictures among the
        // backups could not be brought up to date, if it could not.
        let run = cx.background_spawn({
            let database = database.clone();
            async move {
                if due {
                    match store::collect_unused_images(&database, now, store::IMAGE_RETENTION_MS) {
                        Ok(collected) => (Ok(()), collected.copy_failed),
                        Err(error) => (Err(error), None),
                    }
                } else {
                    match store::back_up_all_images(&database) {
                        Ok(_) => (Ok(()), None),
                        Err(error) => (Ok(()), Some(error.to_string())),
                    }
                }
            }
        });
        let mut said = Said::default();
        let (ran, copy_failed) = run.await;
        match ran {
            Ok(()) if due => {
                let stored = library.update(cx, |library, cx| {
                    library.set_setting(keys::IMAGES_COLLECTED_AT, &now, cx)
                });
                if let Ok(Err(error)) = stored {
                    eprintln!("Inner Word: could not note the run of the image collector: {error}");
                }
            }
            Ok(()) => {}
            Err(error) => eprintln!("Inner Word: the image collector did not run: {error}"),
        }
        said.copy_failed(copy_failed, cx);
        tell_of_files_moved_aside(&database, &library, cx);
        loop {
            cx.background_executor().timer(LOOK_EVERY).await;
            // The library is gone with its window.
            if library.upgrade().is_none() {
                return;
            }
            let now = clock.now_ms();
            let looked = cx
                .background_spawn({
                    let database = database.clone();
                    async move { store::mark_unused_images(&database, now) }
                })
                .await;
            match looked {
                Ok(collected) => said.copy_failed(collected.copy_failed, cx),
                Err(error) => {
                    eprintln!("Inner Word: could not look at which images are in use: {error}")
                }
            }
            tell_of_files_moved_aside(&database, &library, cx);
        }
    })
    .detach();
}

/// What the writer is told when the copy of the pictures among the
/// backups could not be brought up to date. The pictures themselves are
/// in `images.db`; the copy is what brings back one that file loses.
pub fn copy_not_updated(error: &str) -> String {
    format!(
        "The backup copy of the pictures could not be brought up to date, and is tried again later ({error})."
    )
}

/// What the writer is told of a damaged file of pictures that was set
/// aside at `path` (`innerword_store::images_moved_aside`): its pictures came
/// back from the other file, and one that only it had is still in it.
pub fn moved_aside_note(path: &Path) -> String {
    format!(
        "A damaged pictures file was set aside as {}. A picture only it had is still in it.",
        crate::export::short_path(path)
    )
}

/// What the collector of this launch has said.
#[derive(Default)]
struct Said {
    copy_failed: bool,
}

impl Said {
    /// A copy that could not be brought up to date: always on stderr, and
    /// once a launch in a note (it is tried again at every run, once a day).
    fn copy_failed(&mut self, error: Option<String>, cx: &mut gpui::AsyncApp) {
        let Some(error) = error else {
            return;
        };
        eprintln!("Inner Word: the copy of the pictures among the backups is behind: {error}");
        if std::mem::replace(&mut self.copy_failed, true) {
            return;
        }
        cx.update(|cx| {
            innerword_ui::toast::show_toast(
                Toast::new(copy_not_updated(&error))
                    .error()
                    .duration(SAID_FOR),
                cx,
            )
        })
        .ok();
    }
}

/// A path the writer was told of (`keys::IMAGES_MOVED_ASIDE_TOLD`, absolute)
/// as it is now: one told of while the data directory had the old name,
/// "The Writer" (0.9.0 and before), is in the folder of the new name since
/// the move (`innerword_store::data_dir`), so it is read as the same file
/// there and not told of again.
pub fn as_in_this_folder(told: &Path, database: &Path) -> PathBuf {
    let Some(data_dir) = database.parent() else {
        return told.to_path_buf();
    };
    let old_name = data_dir.with_file_name(store::OLD_DATA_DIR_NAME);
    if data_dir.file_name() == Some(std::ffi::OsStr::new(store::DATA_DIR_NAME))
        && let Ok(rest) = told.strip_prefix(&old_name)
    {
        return data_dir.join(rest);
    }
    told.to_path_buf()
}

/// How long a note of the collector stays.
const SAID_FOR: Duration = Duration::from_secs(12);

/// Tells the writer, once for each, of the damaged files of pictures the
/// store set aside (when it found `images.db` or its copy damaged, it made
/// the file again from the other one). Looked at after each run of the
/// collector: at a launch, and once a day while the app stays open.
fn tell_of_files_moved_aside(
    database: &Path,
    library: &WeakEntity<Library>,
    cx: &mut gpui::AsyncApp,
) {
    let found = store::images_moved_aside(database);
    let Ok(told) = library.read_with(cx, |library, _| {
        library.setting_or(keys::IMAGES_MOVED_ASIDE_TOLD, Vec::<String>::new())
    }) else {
        return;
    };
    let named: Vec<String> = found
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    let told: Vec<PathBuf> = told
        .iter()
        .map(|name| as_in_this_folder(Path::new(name), database))
        .collect();
    let new: Vec<&PathBuf> = found
        .iter()
        .filter(|path| !told.contains(path))
        .collect();
    if new.is_empty() && told.len() == named.len() {
        return;
    }
    for path in new {
        eprintln!(
            "Inner Word: a damaged pictures file was set aside: {}",
            path.display()
        );
        let shown = path.clone();
        cx.update(|cx| {
            innerword_ui::toast::show_toast(
                Toast::new(moved_aside_note(path))
                    .error()
                    .duration(SAID_FOR)
                    .on_action("Show in Finder", move |_, cx| cx.reveal_path(&shown)),
                cx,
            )
        })
        .ok();
    }
    // Only the files that are still there: one that was deleted drops out.
    let stored = library.update(cx, |library, cx| {
        library.set_setting(keys::IMAGES_MOVED_ASIDE_TOLD, &named, cx)
    });
    if let Ok(Err(error)) = stored {
        eprintln!("Inner Word: could not note the damaged pictures files told of: {error}");
    }
}
