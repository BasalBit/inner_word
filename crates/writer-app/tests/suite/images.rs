//! Images in the assembled app: a picture comes in (the menu bar, the
//! palette, the "/" menu, the text's menu, a paste), is stored before a
//! block names it, is saved with its document and found again after a
//! relaunch, goes out with the exports and onto paper, and its bytes are
//! kept for as long as anything can name them.
//!
//! The pictures are real files, made with ImageIO when a test runs
//! (`writer_image::fixtures`): no binary is in the repository. The
//! provider is the app's own (`writer_app::images::LibraryImages`), over an
//! in-memory library or one in a temporary directory. **No test opens the
//! open panel** (a test names the files, `images::choose_files_beforehand`)
//! **or reads the system pasteboard** (`pasteboard::stand_in`).
//!
//! What the editor does with a picture by itself (the box, the handles,
//! resize and move with the pointer, paste and copy) is held by its own
//! tests (`crates/writer-editor/src/tests/images.rs`), the image's menu
//! and its description by the tools' (`crates/writer-editor-tools/tests/image.rs`),
//! what agents do with an image by `writer-agent`'s. Here it is the
//! assembly. More of it is where its helpers are: `context_menus.rs` (the
//! image's menu), `mouse.rs` (the description with the pointer alone),
//! `live.rs` (agents), `launch.rs`, `assistant.rs`, `findings.rs`,
//! `story.rs`.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use gpui::{Action, TestAppContext};
use writer_app::{
    SaveState,
    automation::{ImageStep, Script, Step, describe_state},
    export::{ExportChoice, ExportDestination, ExportScope, pictures_sentence, short_path},
    images::{self, COLLECT_DELAY, LOOK_EVERY, REFUSALS_NAMED, copy_not_updated, more_refused},
    menus, pasteboard, printing,
    settings::keys,
    shortcuts::{Place, collect_shortcuts},
    version_history::{IMAGE_WITHOUT_DESCRIPTION, VersionHistory},
    workspace::{AUTOSAVE_DELAY, SELECT_AN_IMAGE_FIRST, is_image_command},
};
use writer_core::{Alignment, DocumentSettings, Image, ImageId, ImageWidth, Position};
use writer_editor::{ImageStatus, NewImageSource, actions as editor};
use writer_editor_tools::{ToolKind, actions as tools};
use writer_export::ExportFormat;
use writer_library::{Clock, ImageFormat, Library, NewImage, StoredImageId};
use writer_ui::ContextMenu;

use super::*;

// ----- pictures ----------------------------------------------------------------------------

/// A PNG of one colour, as a file would hold it.
pub fn png(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    writer_image::fixtures::png(width, height, rgb)
}

/// The id a document has for these stored bytes.
pub fn id_of(bytes: &[u8]) -> ImageId {
    ImageId::from_bytes(StoredImageId::of(bytes).0)
}

/// Stores a PNG of that size and colour in `library` and returns the
/// image as a document names it: the whole column wide, no description.
pub fn stored_png(
    library: &Entity<Library>,
    width: u32,
    height: u32,
    rgb: [u8; 3],
    cx: &mut TestAppContext,
) -> Image {
    let bytes = png(width, height, rgb);
    let info = library.update(cx, |library, _| {
        library
            .put_image(&NewImage {
                bytes: &bytes,
                format: ImageFormat::Png,
                width,
                height,
                density: 72,
            })
            .expect("the picture is stored")
    });
    images::image_of(&info)
}

/// `image` as a block: `percent` of the column wide, aligned, described.
pub fn picture(image: &Image, percent: u16, alignment: Alignment, description: &str) -> Block {
    let mut image = image.clone();
    image.width = ImageWidth::from_percent(percent);
    image.description = description.to_string();
    Block::image(image).with_alignment(alignment)
}

/// Writes `bytes` as the file `name` in `dir`.
pub fn file_in(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("the file is written");
    path
}

/// The stored bytes of `image`, as the library has them.
pub fn bytes_in(
    library: &Entity<Library>,
    image: ImageId,
    cx: &mut VisualTestContext,
) -> Option<Vec<u8>> {
    library.read_with(cx, |library, _| {
        library
            .store()
            .image_bytes(&images::stored_id(image))
            .expect("the library answers")
    })
}

/// The images of the open document: the index of each block and the image.
pub fn images_in(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<(usize, Image)> {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .document()
            .images()
            .map(|(index, image)| (index, image.clone()))
            .collect()
    })
}

/// The image the caret is on.
pub fn selected_image(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Option<(usize, Image)> {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).selected_image())
}

/// What the editor has to show for `image`.
pub fn status(
    workspace: &Entity<Workspace>,
    image: ImageId,
    cx: &mut VisualTestContext,
) -> ImageStatus {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).image_status(image))
}

/// Lets the pictures on screen arrive: the frame asks for them, the bytes
/// are read and decoded in the background, and the next frame has them.
pub fn let_pictures_arrive(cx: &mut VisualTestContext) {
    for _ in 0..6 {
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
    }
    cx.run_until_parked();
}

/// Puts the caret on block `block` (an image is then selected).
pub fn caret_on(workspace: &Entity<Workspace>, block: usize, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(|state| state.set_caret(Position::new(block, 0)), cx);
    });
    cx.run_until_parked();
}

fn kinds(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<&'static str> {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .document()
            .blocks()
            .iter()
            .map(|block| block.kind().name())
            .collect()
    })
}

fn undo_depth(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> usize {
    workspace.read_with(cx, |w, cx| w.editor().read(cx).state().undo_depth())
}

fn toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) -> Vec<String> {
    workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| toast.message.to_string())
            .collect()
    })
}

fn clear_toasts(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let layer = workspace.read_with(cx, |w, _| w.toast_layer().clone());
    layer.update(cx, |layer, cx| layer.clear(cx));
    cx.run_until_parked();
}

/// The action of the item `name` of the menu bar's menu `menu`.
fn menu_bar_item(menu: &str, name: &str) -> Box<dyn Action> {
    let menu = menus::app_menus()
        .into_iter()
        .find(|candidate| candidate.name.as_ref() == menu)
        .unwrap_or_else(|| panic!("the menu bar has no menu {menu}"));
    menus::menu_actions(&[menu])
        .into_iter()
        .find(|(item, _)| item == name)
        .map(|(_, action)| action)
        .unwrap_or_else(|| panic!("the menu has no item {name}"))
}

fn choose_in_menu_bar(menu: &str, name: &str, cx: &mut VisualTestContext) {
    let action = menu_bar_item(menu, name);
    cx.update(|window, cx| window.dispatch_action(action, cx));
    cx.run_until_parked();
}

fn available(action: &dyn Action, cx: &mut VisualTestContext) -> bool {
    cx.run_until_parked();
    cx.update(|window, cx| window.is_action_available(action, cx))
}

/// Runs the command the palette lists as `title` (its category before it:
/// "Insert: Image"), found by typing `query`.
fn run_in_palette(query: &str, cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input(query);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

/// A workspace over an in-memory library with `document` open and the
/// keyboard in its text.
fn open_with<'a>(
    document: &Document,
    library: &Entity<Library>,
    cx: &'a mut TestAppContext,
) -> (DocumentId, Entity<Workspace>, &'a mut VisualTestContext) {
    let id = add_document(library, None, document, cx);
    let (workspace, vcx) = open_workspace(library, cx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(id, true, window, cx));
    });
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    (id, workspace, vcx)
}

fn name_the_files(paths: &[&PathBuf], cx: &mut VisualTestContext) {
    let paths: Vec<PathBuf> = paths.iter().map(|path| (*path).clone()).collect();
    cx.update(|_, cx| images::choose_files_beforehand(paths, cx));
}

const RED: [u8; 3] = [200, 60, 40];
const GREEN: [u8; 3] = [60, 160, 90];
const BLUE: [u8; 3] = [50, 80, 190];
const SAND: [u8; 3] = [220, 200, 150];

// ----- a picture comes in ------------------------------------------------------------------

#[gpui::test]
fn insert_image_puts_the_file_in_from_the_menu_bar_the_palette_the_slash_menu_and_the_text_menu(
    cx: &mut TestAppContext,
) {
    init(cx);
    let library = library(cx);
    let files = tempfile::tempdir().unwrap();
    let pictures = [
        ("menu bar.png", png(320, 200, RED)),
        ("palette.png", png(300, 200, GREEN)),
        ("slash.png", png(280, 200, BLUE)),
        ("text menu.png", png(260, 200, SAND)),
    ];
    let paths: Vec<PathBuf> = pictures
        .iter()
        .map(|(name, bytes)| file_in(files.path(), name, bytes))
        .collect();
    let (_, workspace, cx) = open_with(&simple_document("Harbour", "The boats."), &library, cx);
    cx.simulate_keystrokes("cmd-down");

    // The menu bar: Insert > Image…. The text is not empty where the
    // caret is, so the picture goes after its block, and is selected.
    name_the_files(&[&paths[0]], cx);
    choose_in_menu_bar("Insert", "Image…", cx);
    assert_eq!(kinds(&workspace, cx), ["heading", "paragraph", "image"]);
    let (block, image) = selected_image(&workspace, cx).expect("the new image is selected");
    assert_eq!(block, 2);
    assert_eq!(
        image.id,
        id_of(&pictures[0].1),
        "named by the hash of its bytes"
    );
    assert_eq!(image.pixels, (320, 200));
    assert_eq!(image.description, "", "no description by itself");
    assert_eq!(
        bytes_in(&library, image.id, cx).as_deref(),
        Some(pictures[0].1.as_slice()),
        "the library has the file as it came"
    );
    // Centred, at its own size on the page (320 of 451 points).
    let alignment = workspace.read_with(cx, |w, cx| {
        w.editor().read(cx).document().block(2).unwrap().alignment()
    });
    assert_eq!(alignment, Alignment::Center);
    assert_eq!(image.width.percent(), 71);
    assert!(editor_focused(&workspace, cx));

    // The palette: "Image" under Insert. Enter on the selected image
    // first: a paragraph below it.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("More text.");
    name_the_files(&[&paths[1]], cx);
    run_in_palette("insert image", cx);
    assert_eq!(
        kinds(&workspace, cx),
        ["heading", "paragraph", "image", "paragraph", "image"]
    );
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, image)| (block, image.id)),
        Some((4, id_of(&pictures[1].1)))
    );

    // "/" in an empty paragraph: the paragraph becomes the image.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("/");
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool()),
        Some(ToolKind::Insert)
    );
    cx.simulate_input("image");
    name_the_files(&[&paths[2]], cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        kinds(&workspace, cx),
        [
            "heading",
            "paragraph",
            "image",
            "paragraph",
            "image",
            "image"
        ]
    );
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, image)| (block, image.id)),
        Some((5, id_of(&pictures[2].1)))
    );

    // The text's own menu: Insert > Image…, from a line of text.
    cx.simulate_keystrokes("enter");
    cx.simulate_input("A last line.");
    cx.simulate_keystrokes("ctrl-enter");
    cx.run_until_parked();
    name_the_files(&[&paths[3]], cx);
    assert!(
        cx.update(|window, cx| ContextMenu::choose("Image…", window, cx)),
        "the text's menu has Insert > Image…"
    );
    cx.run_until_parked();
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, image)| (block, image.id)),
        Some((7, id_of(&pictures[3].1)))
    );
    assert_eq!(images_in(&workspace, cx).len(), 4);
    for (_, bytes) in &pictures {
        assert!(bytes_in(&library, id_of(bytes), cx).is_some());
    }
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .document()
            .check_invariants()
            .expect("the document is whole")
    });

    // The pictures are on screen.
    let_pictures_arrive(cx);
    assert_eq!(
        status(&workspace, id_of(&pictures[3].1), cx),
        ImageStatus::Ready
    );
}

#[gpui::test]
fn a_file_that_is_no_image_says_so_and_three_files_are_one_undo_step(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let files = tempfile::tempdir().unwrap();
    let pdf = file_in(files.path(), "report.pdf", b"%PDF-1.4\nnot a picture\n");
    let svg = file_in(
        files.path(),
        "drawing.svg",
        b"<svg xmlns='http://www.w3.org/2000/svg'/>",
    );
    let text = file_in(files.path(), "notes.txt", b"Just words.");
    let gone = files.path().join("gone.png");
    let three: Vec<(PathBuf, Vec<u8>)> = [RED, GREEN, BLUE]
        .into_iter()
        .enumerate()
        .map(|(index, colour)| {
            let bytes = png(200 + index as u32 * 20, 120, colour);
            (
                file_in(files.path(), &format!("{index}.png"), &bytes),
                bytes,
            )
        })
        .collect();
    let (_, workspace, cx) = open_with(&simple_document("Harbour", "The boats."), &library, cx);
    cx.simulate_keystrokes("cmd-down");
    let depth = undo_depth(&workspace, cx);

    // A PDF: the sentence, and no block.
    name_the_files(&[&pdf], cx);
    choose_in_menu_bar("Insert", "Image…", cx);
    assert_eq!(
        toasts(&workspace, cx),
        ["“report.pdf” is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."]
    );
    assert_eq!(kinds(&workspace, cx), ["heading", "paragraph"]);
    assert_eq!(undo_depth(&workspace, cx), depth, "nothing to undo");
    assert_eq!(
        library.read_with(cx, |library, _| library.store().image_count().unwrap()),
        0
    );

    // Five files of which none can be used, chosen a while later (a
    // gesture of its own): three are named, the rest counted in one
    // sentence.
    clear_toasts(&workspace, cx);
    cx.executor().advance_clock(Duration::from_secs(5));
    name_the_files(&[&pdf, &svg, &text, &gone, &pdf], cx);
    choose_in_menu_bar("Insert", "Image…", cx);
    assert_eq!(REFUSALS_NAMED, 3);
    assert_eq!(
        toasts(&workspace, cx),
        [
            "“report.pdf” is a PDF file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
                .to_string(),
            "“drawing.svg” is an SVG file. Images can be PNG, JPEG, GIF, HEIC, TIFF, WebP or BMP."
                .to_string(),
            "“notes.txt” is not an image.".to_string(),
            more_refused(2),
        ]
    );
    assert_eq!(more_refused(2), "2 more files could not be used.");
    assert_eq!(more_refused(1), "1 more file could not be used.");
    assert_eq!(kinds(&workspace, cx), ["heading", "paragraph"]);

    // Three pictures and a file that is none, chosen together: the three
    // go in, in the order they were chosen, as one step of undo.
    clear_toasts(&workspace, cx);
    cx.executor().advance_clock(Duration::from_secs(5));
    name_the_files(&[&three[0].0, &pdf, &three[1].0, &three[2].0], cx);
    choose_in_menu_bar("Insert", "Image…", cx);
    assert_eq!(
        kinds(&workspace, cx),
        ["heading", "paragraph", "image", "image", "image"]
    );
    let ids: Vec<ImageId> = images_in(&workspace, cx)
        .into_iter()
        .map(|(_, image)| image.id)
        .collect();
    assert_eq!(
        ids,
        three
            .iter()
            .map(|(_, bytes)| id_of(bytes))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, _)| block),
        Some(4)
    );
    assert_eq!(
        toasts(&workspace, cx).len(),
        1,
        "the PDF among them said so"
    );
    assert_eq!(undo_depth(&workspace, cx), depth + 1, "one step");

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(kinds(&workspace, cx), ["heading", "paragraph"]);
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(
        kinds(&workspace, cx),
        ["heading", "paragraph", "image", "image", "image"]
    );
}

#[gpui::test]
fn a_cancelled_panel_and_a_document_that_cannot_be_changed_insert_nothing(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let (workspace, cx) = open_workspace(&library, cx);
    // No document: Insert > Image… is greyed out, with the image commands.
    assert!(workspace.read_with(cx, |w, _| w.open_document_id().is_none()));
    assert!(!available(&editor::InsertImage, cx));
    assert!(!available(&editor::ImageWidth50, cx));
    assert!(!available(&tools::EditImageDescription, cx));

    cx.simulate_keystrokes("cmd-n");
    cx.run_until_parked();
    assert!(available(&editor::InsertImage, cx));
    assert!(available(&editor::ImageWidth50, cx));
    assert!(available(&tools::EditImageDescription, cx));
    // A test that names no file has a panel that was cancelled: nothing
    // comes in, nothing is said.
    choose_in_menu_bar("Insert", "Image…", cx);
    assert!(images_in(&workspace, cx).is_empty());
    assert!(toasts(&workspace, cx).is_empty());
    assert!(!cx.did_prompt_for_new_path());
}

// ----- on disk: stored first, in the background; a relaunch -----------------------------------

/// A library in a temporary directory.
struct Disk {
    dir: tempfile::TempDir,
}

impl Disk {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a temporary directory"),
        }
    }

    fn database(&self) -> PathBuf {
        self.dir.path().join("library.db")
    }

    fn images_file(&self) -> PathBuf {
        self.dir.path().join("images.db")
    }

    /// The bytes of `image` as `images.db` has them, read through a
    /// connection of its own.
    fn stored(&self, image: ImageId) -> Option<Vec<u8>> {
        writer_library::store::ImageFile::read_only(&self.database())
            .bytes(&images::stored_id(image))
            .expect("images.db answers")
    }

    /// The bytes of `image` in the copy of the pictures among the
    /// backups (`backups/images.db`), read by itself.
    fn in_the_copy(&self, image: ImageId) -> Option<Vec<u8>> {
        let copy = writer_library::store::images_backup_path(&self.database());
        if !copy.exists() {
            return None;
        }
        // `ImageFile` names its file by the library database beside it.
        let beside = copy.with_file_name("library.db");
        writer_library::store::ImageFile::read_only(&beside)
            .bytes(&images::stored_id(image))
            .expect("the copy answers")
    }

    /// One launch: a library of its own over the file (its clock is
    /// `clock` when there is one), and a workspace.
    fn launch<'a>(
        &self,
        clock: Option<&writer_library::Clock>,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<Library>,
        Entity<Workspace>,
        &'a mut VisualTestContext,
    ) {
        use writer_library::store::{OpenOptions, Store};
        let options = OpenOptions {
            clock: clock.cloned().unwrap_or_default(),
            ..OpenOptions::default()
        };
        let database = self.database();
        let library = cx.new(|_| {
            Library::from_store(Store::open_with(database, options).expect("the library opens"))
                .expect("the library loads")
        });
        library.update(cx, |library, cx| {
            library
                .set_setting(keys::WELCOME_CREATED, &true, cx)
                .unwrap();
        });
        let (workspace, vcx) = open_workspace(&library, cx);
        (library, workspace, vcx)
    }
}

/// Lets the autosave run and waits for the save of the background thread.
fn let_it_save(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(AUTOSAVE_DELAY * 2);
    cx.run_until_parked();
    for _ in 0..400 {
        if workspace.read_with(cx, |w, _| w.save_state()) == Some(SaveState::Saved) {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
        cx.run_until_parked();
    }
    panic!("the document was not saved");
}

/// The end of a launch: everything is saved and the window goes.
fn quit(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    workspace.update(cx, |workspace, cx| {
        assert!(workspace.flush(cx), "the last save works");
    });
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
}

fn stored_content(library: &Entity<Library>, id: DocumentId, cx: &mut VisualTestContext) -> String {
    library.read_with(cx, |library, _| library.document(id).unwrap().content)
}

fn insert_files(workspace: &Entity<Workspace>, paths: Vec<PathBuf>, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update_in(cx, |editor, window, cx| {
        editor.insert_image_files(paths, window, cx)
    });
}

#[gpui::test]
fn on_disk_a_picture_is_stored_in_the_background_before_a_block_names_it_and_a_relaunch_shows_it(
    cx: &mut TestAppContext,
) {
    init(cx);
    let disk = Disk::new();
    let files = tempfile::tempdir().unwrap();
    let bytes = writer_image::fixtures::png_at(640, 400, BLUE, 144);
    let path = file_in(files.path(), "harbour.png", &bytes);
    let image = id_of(&bytes);

    let id = {
        let (library, workspace, vcx) = disk.launch(None, cx);
        let id = add_document(
            &library,
            None,
            &simple_document("Harbour", "The boats."),
            vcx,
        );
        workspace.update_in(vcx, |w, window, cx| {
            assert!(w.open_document(id, true, window, cx));
        });
        vcx.run_until_parked();
        vcx.simulate_keystrokes("cmd-down");
        let_it_save(&workspace, vcx);
        // A library without images has no second file.
        assert!(!disk.images_file().exists());

        // The file is chosen. On this turn of the main thread nothing is
        // read, converted or written: that is a task of the background.
        insert_files(&workspace, vec![path.clone()], vcx);
        assert!(images_in(&workspace, vcx).is_empty(), "no block yet");
        assert!(!disk.images_file().exists(), "nothing written yet");

        // The task has run: the bytes are committed, and only then is
        // the block there.
        vcx.run_until_parked();
        assert_eq!(disk.stored(image).as_deref(), Some(bytes.as_slice()));
        let (block, inserted) = selected_image(&workspace, vcx).expect("the image is selected");
        assert_eq!((block, inserted.id), (2, image));
        // A screenshot of a Retina display: 640 pixels are 320 points.
        assert_eq!((inserted.pixels, inserted.density), ((640, 400), 144));
        assert_eq!(inserted.natural_points().0, 320.);
        // The stored document does not name the image yet, and the
        // bytes are there: never the other way round.
        assert_eq!(
            workspace.read_with(vcx, |w, _| w.save_state()),
            Some(SaveState::Edited)
        );
        assert!(!stored_content(&library, id, vcx).contains("\"image\""));

        let_it_save(&workspace, vcx);
        let content = stored_content(&library, id, vcx);
        assert!(
            content.contains(&format!("\"image\":\"{image}\"")),
            "the saved document names the image: {content}"
        );
        assert_eq!(disk.stored(image).as_deref(), Some(bytes.as_slice()));
        // What the collector looks for is what a save writes.
        let named = library.read_with(vcx, |library, _| {
            library.store().referenced_images().unwrap()
        });
        assert!(named.contains(&images::stored_id(image)));

        // The picture is on screen, read through the provider's own
        // connection.
        let_pictures_arrive(vcx);
        assert_eq!(status(&workspace, image, vcx), ImageStatus::Ready);
        quit(&workspace, vcx);
        id
    };

    // The next launch: the document opens with its image.
    let (_library, workspace, vcx) = disk.launch(None, cx);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.open_document_id()),
        Some(id)
    );
    let found = images_in(&workspace, vcx);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].0, found[0].1.id), (2, image));
    let_pictures_arrive(vcx);
    assert_eq!(status(&workspace, image, vcx), ImageStatus::Ready);
    assert_eq!(
        workspace.read_with(vcx, |w, _| w.save_state()),
        Some(SaveState::Saved),
        "showing a picture is no edit"
    );
}

#[gpui::test]
fn undo_of_an_insertion_then_a_save_then_redo_and_the_image_is_there(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let files = tempfile::tempdir().unwrap();
    let bytes = png(300, 200, GREEN);
    let path = file_in(files.path(), "boat.png", &bytes);
    let image = id_of(&bytes);
    let (id, workspace, cx) = open_with(&simple_document("Harbour", "The boats."), &library, cx);
    cx.simulate_keystrokes("cmd-down");

    insert_files(&workspace, vec![path], cx);
    cx.run_until_parked();
    assert_eq!(images_in(&workspace, cx).len(), 1);

    // Undo, and what is saved names no image any more.
    cx.simulate_keystrokes("cmd-z");
    assert!(images_in(&workspace, cx).is_empty());
    workspace.update(cx, |w, cx| assert!(w.flush(cx)));
    assert!(!stored_content(&library, id, cx).contains("\"image\""));
    // The bytes stay: nothing but the collector ever deletes them.
    assert_eq!(
        bytes_in(&library, image, cx).as_deref(),
        Some(bytes.as_slice())
    );

    // Redo: the image is back, with its picture.
    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(
        images_in(&workspace, cx)
            .iter()
            .map(|(block, image)| (*block, image.id))
            .collect::<Vec<_>>(),
        [(2, image)]
    );
    let_pictures_arrive(cx);
    assert_eq!(status(&workspace, image, cx), ImageStatus::Ready);
    workspace.update(cx, |w, cx| assert!(w.flush(cx)));
    assert!(stored_content(&library, id, cx).contains(&format!("\"image\":\"{image}\"")));
}

#[gpui::test]
fn an_image_cut_in_one_document_is_pasted_whole_into_another(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 300, RED, cx);
    let source = Document::from_blocks(vec![
        Block::paragraph("Before."),
        picture(&image, 50, Alignment::Right, "The harbour at dusk"),
        Block::paragraph("After."),
    ]);
    let other = add_document(&library, None, &simple_document("Notes", "A line."), cx);
    let (first, workspace, cx) = open_with(&source, &library, cx);

    // Cut, with the caret on the image: the picture itself goes to the
    // clipboard (it can be pasted into other apps), and the block is gone.
    caret_on(&workspace, 1, cx);
    assert!(selected_image(&workspace, cx).is_some());
    cx.simulate_keystrokes("cmd-x");
    cx.run_until_parked();
    assert_eq!(kinds(&workspace, cx), ["paragraph", "paragraph"]);
    let copied = cx
        .read_from_clipboard()
        .expect("something is on the clipboard");
    assert!(
        copied
            .entries()
            .iter()
            .any(|entry| matches!(entry, gpui::ClipboardEntry::Image(_))),
        "the clipboard holds the picture"
    );

    // Another document: paste. The block comes back as it was.
    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(other, true, window, cx));
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("cmd-down cmd-v");
    cx.run_until_parked();
    let pasted = images_in(&workspace, cx);
    assert_eq!(pasted.len(), 1);
    let (block, pasted) = &pasted[0];
    assert_eq!(pasted.id, image.id);
    assert_eq!(pasted.width, ImageWidth::from_percent(50));
    assert_eq!(pasted.description, "The harbour at dusk");
    assert_eq!(
        workspace.read_with(cx, |w, cx| {
            w.editor()
                .read(cx)
                .document()
                .block(*block)
                .unwrap()
                .alignment()
        }),
        Alignment::Right
    );
    assert_eq!(
        selected_image(&workspace, cx).map(|(at, _)| at),
        Some(*block)
    );
    let_pictures_arrive(cx);
    assert_eq!(status(&workspace, image.id, cx), ImageStatus::Ready);

    // Both documents are saved as they are now.
    workspace.update(cx, |w, cx| assert!(w.flush(cx)));
    assert!(!stored_content(&library, first, cx).contains("\"image\""));
    assert!(stored_content(&library, other, cx).contains(&format!("\"image\":\"{}\"", image.id)));
}

/// A picture GPUI's clipboard does not show (a file copied in Finder,
/// "Copy Image" in a browser) is what the app's own look at the
/// pasteboard is for. In a test the pasteboard is a stand-in.
#[gpui::test]
fn paste_takes_a_picture_only_the_pasteboard_shows(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let files = tempfile::tempdir().unwrap();
    let bytes = png(240, 160, SAND);
    let path = file_in(files.path(), "copied in Finder.png", &bytes);
    let (_, workspace, cx) = open_with(&simple_document("Harbour", "The boats."), &library, cx);
    cx.simulate_keystrokes("cmd-down");

    // "Paste" in the text's menu is there for a picture GPUI does not
    // show, also while its clipboard is empty.
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    assert!(!editor.update(cx, |editor, cx| editor.can_paste(cx)));
    cx.update(|_, cx| pasteboard::stand_in(Some(NewImageSource::File(path.clone())), cx));
    assert!(editor.update(cx, |editor, cx| editor.can_paste(cx)));
    cx.update(|_, cx| pasteboard::stand_in(None, cx));
    assert!(!editor.update(cx, |editor, cx| editor.can_paste(cx)));

    // Without anybody to ask (a test, a scripted run), a paste of text is
    // a paste of text.
    assert!(cx.update(|_, cx| pasteboard::image(cx).is_none()));
    cx.write_to_clipboard(gpui::ClipboardItem::new_string(
        "copied in Finder.png".to_string(),
    ));
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert!(images_in(&workspace, cx).is_empty());
    assert!(editor_text(&workspace, cx).ends_with("The boats.copied in Finder.png"));

    // A file copied in Finder: GPUI hands over its name as text, the
    // pasteboard has the file.
    cx.update(|_, cx| pasteboard::stand_in(Some(NewImageSource::File(path.clone())), cx));
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(
        selected_image(&workspace, cx).map(|(_, image)| image.id),
        Some(id_of(&bytes))
    );

    // "Copy Image" in a browser: the picture's bytes. A picture that is
    // no picture says so, about the clipboard.
    cx.update(|_, cx| pasteboard::stand_in(Some(NewImageSource::Bytes(b"<html>".to_vec())), cx));
    cx.simulate_keystrokes("cmd-v");
    cx.run_until_parked();
    assert_eq!(images_in(&workspace, cx).len(), 1);
    assert_eq!(
        toasts(&workspace, cx),
        ["The picture on the clipboard is not an image."]
    );
}

/// What counts as a picture on the pasteboard is a pure function of what
/// the pasteboard offers.
#[test]
fn what_a_paste_takes_from_the_pasteboard() {
    use pasteboard::{Contents, FILE_URL, PLAIN_TEXT, Take, decide, is_an_address};
    let png = Path::new("/Users/someone/Desktop/harbour.PNG");
    let heic = Path::new("/Users/someone/Pictures/IMG_0042.heic");
    let text = Path::new("/Users/someone/Documents/notes.txt");
    let pdf = Path::new("/Users/someone/Documents/report.pdf");

    // An image file copied in Finder: the file, whatever else Finder put
    // beside it (its name as text, its icon as a picture).
    let finder = Contents {
        types: &[FILE_URL, "NSFilenamesPboardType", PLAIN_TEXT, "public.tiff"],
        file: Some(png),
        text: Some("harbour.PNG"),
    };
    assert_eq!(decide(&finder), Some(Take::File(png)));
    assert_eq!(
        decide(&Contents {
            file: Some(heic),
            text: Some("IMG_0042.heic"),
            ..finder
        }),
        Some(Take::File(heic))
    );
    // A file that is no image is its name, as text: the icon beside it is
    // not what was copied.
    for other in [text, pdf] {
        assert_eq!(
            decide(&Contents {
                file: Some(other),
                text: other.file_name().and_then(|name| name.to_str()),
                ..finder
            }),
            None
        );
    }

    // "Copy Image" in a browser: the picture, and beside it the address it
    // came from. A kind the library stores as it is comes before one that
    // has to be converted.
    let browser = Contents {
        types: &["public.tiff", "public.png", PLAIN_TEXT, "public.html"],
        file: None,
        text: Some("https://example.com/pictures/harbour.png"),
    };
    assert_eq!(decide(&browser), Some(Take::Data("public.png")));
    assert_eq!(
        decide(&Contents {
            types: &["public.tiff", PLAIN_TEXT],
            text: Some("  data:image/png;base64,AAAA\n"),
            ..browser
        }),
        Some(Take::Data("public.tiff"))
    );

    // Cells copied in a spreadsheet carry a picture of themselves beside
    // their text: they stay text.
    let cells = Contents {
        types: &[PLAIN_TEXT, "public.rtf", "public.tiff", "public.png"],
        file: None,
        text: Some("Month\tSalt\nMay\t12\nJune\t9"),
    };
    assert_eq!(decide(&cells), None);
    // So does a sentence with a picture beside it, and an address that is
    // part of a sentence.
    assert_eq!(
        decide(&Contents {
            text: Some("See https://example.com/harbour.png for the picture"),
            ..cells
        }),
        None
    );

    // Plain text.
    let words = Contents {
        types: &[PLAIN_TEXT],
        file: None,
        text: Some("The boats came by road."),
    };
    assert_eq!(decide(&words), None);
    // An address by itself, with no picture: text.
    assert_eq!(
        decide(&Contents {
            text: Some("https://example.com"),
            ..words
        }),
        None
    );

    // A screenshot, a photo copied in Photos: a picture and no text.
    assert_eq!(
        decide(&Contents {
            types: &["public.png", "public.tiff"],
            file: None,
            text: None,
        }),
        Some(Take::Data("public.png"))
    );
    assert_eq!(
        decide(&Contents {
            types: &["public.heic", "public.jpeg"],
            file: None,
            text: Some(""),
        }),
        Some(Take::Data("public.jpeg"))
    );
    assert_eq!(
        decide(&Contents {
            types: &["public.heic"],
            file: None,
            text: None,
        }),
        Some(Take::Data("public.heic"))
    );
    // Nothing at all.
    assert_eq!(decide(&Contents::default()), None);

    for address in [
        "http://a.example/x.png",
        "HTTPS://A.EXAMPLE",
        "file:///tmp/a.png",
        "data:,x",
    ] {
        assert!(is_an_address(address), "{address}");
    }
    for other in [
        "",
        "harbour.png",
        "see https://a.example",
        "https://a.example\nhttps://b.example",
    ] {
        assert!(!is_an_address(other), "{other:?}");
    }
}

// ----- the commands ----------------------------------------------------------------------------

/// The seven commands that came with images: what the palette calls each,
/// and the key a test gives it.
const COMMANDS: [(&str, &str, &str); 7] = [
    ("editor::InsertImage", "Insert: Image", "ctrl-alt-i"),
    (
        "editor::ImageWidth25",
        "Layout: Image width 25 %",
        "ctrl-alt-1",
    ),
    (
        "editor::ImageWidth50",
        "Layout: Image width 50 %",
        "ctrl-alt-2",
    ),
    (
        "editor::ImageWidth75",
        "Layout: Image width 75 %",
        "ctrl-alt-3",
    ),
    (
        "editor::ImageWidth100",
        "Layout: Image width 100 %",
        "ctrl-alt-4",
    ),
    (
        "editor::ImageOriginalSize",
        "Layout: Image original size",
        "ctrl-alt-0",
    ),
    (
        "editor_tools::EditImageDescription",
        "Format: Image description",
        "ctrl-alt-d",
    ),
];

/// A heading, an image at the whole width, a paragraph.
fn around(image: &Image) -> Document {
    Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Harbour"),
        picture(image, 100, Alignment::Center, ""),
        Block::paragraph("The boats came by road every spring."),
    ])
}

fn width_of(workspace: &Entity<Workspace>, block: usize, cx: &mut VisualTestContext) -> u16 {
    workspace.read_with(cx, |w, cx| {
        w.editor()
            .read(cx)
            .document()
            .block(block)
            .and_then(|block| block.as_image())
            .expect("the block is an image")
            .width
            .percent()
    })
}

#[gpui::test]
fn the_image_commands_are_in_the_palette_and_the_list_without_a_key_and_take_one(
    cx: &mut TestAppContext,
) {
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 200, 150, RED, cx);
    let files = tempfile::tempdir().unwrap();
    let second = png(100, 80, GREEN);
    let path = file_in(files.path(), "second.png", &second);
    let (_, workspace, cx) = open_with(&around(&image), &library, cx);

    // The palette.
    let labels = cx.update(|_, cx| {
        let all = writer_app::commands::all(cx);
        COMMANDS
            .iter()
            .map(|(action, _, _)| {
                all.iter()
                    .find(|command| command.action_name() == *action)
                    .unwrap_or_else(|| panic!("{action} is no command"))
                    .label()
                    .to_string()
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        labels,
        COMMANDS
            .iter()
            .map(|(_, label, _)| *label)
            .collect::<Vec<_>>()
    );

    // The list of keyboard shortcuts: each is a row "In the text", and
    // has no key.
    let rows = |cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            let rows = collect_shortcuts(cx);
            COMMANDS
                .iter()
                .map(|(action, _, _)| {
                    let row = rows
                        .iter()
                        .find(|row| row.action_name() == *action)
                        .unwrap_or_else(|| panic!("{action} is no row"));
                    (
                        row.place,
                        row.all_keys
                            .iter()
                            .map(|key| key.keys.to_string())
                            .collect::<Vec<_>>(),
                    )
                })
                .collect::<Vec<_>>()
        })
    };
    for (index, (place, keys)) in rows(cx).into_iter().enumerate() {
        assert_eq!(place, Place::Text, "{}", COMMANDS[index].0);
        assert!(keys.is_empty(), "{} has {keys:?}", COMMANDS[index].0);
    }
    // "Larger" and "smaller" are the keys of the text's size, and say so
    // in their titles: the list finds them under "image".
    cx.update(|_, cx| {
        let sizes: Vec<(String, Vec<String>)> = collect_shortcuts(cx)
            .into_iter()
            .filter(|row| row.title.to_lowercase().contains("or image"))
            .map(|row| {
                (
                    row.title.to_string(),
                    row.all_keys
                        .iter()
                        .map(|key| key.keys.to_string())
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            sizes,
            [
                (
                    "Larger text or image".to_string(),
                    vec!["cmd-shift-.".to_string()]
                ),
                (
                    "Smaller text or image".to_string(),
                    vec!["cmd-shift-,".to_string()]
                ),
            ]
        );
    });

    // A key from the list, with its guard rails: the key is free in the
    // text.
    let half = writer_keymap::Target::new("editor::ImageWidth50", Some("Editor"));
    let added = workspace
        .update_in(cx, |w, window, cx| {
            w.change_keys(window, cx, |keymap| {
                let key = writer_keymap::KeySequence::parse("ctrl-alt-2").unwrap();
                assert!(keymap.check(&half, &key, None).is_allowed());
                keymap.add_key(&half, key)
            })
        })
        .expect("the app has a keymap");
    assert!(added);
    // And the others.
    for (action, _, key) in COMMANDS {
        let key = writer_keymap::KeySequence::parse(key).unwrap();
        workspace
            .update_in(cx, |w, window, cx| {
                w.set_keys_of_command(action, None, Some(&[key]), window, cx)
            })
            .unwrap_or_else(|error| panic!("{action}: {error}"));
    }
    cx.run_until_parked();
    assert_eq!(
        rows(cx)
            .into_iter()
            .map(|(_, keys)| keys.join(" "))
            .collect::<Vec<_>>(),
        COMMANDS.iter().map(|(_, _, key)| *key).collect::<Vec<_>>()
    );

    // The keys do what the commands do.
    caret_on(&workspace, 1, cx);
    cx.simulate_keystrokes("ctrl-alt-2");
    assert_eq!(width_of(&workspace, 1, cx), 50);
    cx.simulate_keystrokes("ctrl-alt-1");
    assert_eq!(width_of(&workspace, 1, cx), 25);
    cx.simulate_keystrokes("ctrl-alt-3");
    assert_eq!(width_of(&workspace, 1, cx), 75);
    cx.simulate_keystrokes("ctrl-alt-4");
    assert_eq!(width_of(&workspace, 1, cx), 100);
    // 200 pixels at 72 to the inch on a column of 451 points.
    cx.simulate_keystrokes("ctrl-alt-0");
    assert_eq!(width_of(&workspace, 1, cx), 44);
    // The keys of the text's size step the image while it is selected.
    cx.simulate_keystrokes("cmd-shift-.");
    assert_eq!(width_of(&workspace, 1, cx), 50);
    cx.simulate_keystrokes("cmd-shift-,");
    assert_eq!(width_of(&workspace, 1, cx), 40);
    cx.simulate_keystrokes("ctrl-alt-d");
    cx.run_until_parked();
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool()),
        Some(ToolKind::ImageDescription)
    );
    cx.simulate_input("A red boat");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        selected_image(&workspace, cx).unwrap().1.description,
        "A red boat"
    );
    assert!(editor_focused(&workspace, cx));
    name_the_files(&[&path], cx);
    cx.simulate_keystrokes("ctrl-alt-i");
    cx.run_until_parked();
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, image)| (block, image.id)),
        Some((2, id_of(&second)))
    );

    // The menu bar shows the key beside the item.
    cx.update(|_, cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let shown = |action: &dyn Action| {
            menus::native_menu_binding(&keymap, action)
                .map(|binding| binding.keystrokes()[0].unparse())
        };
        assert_eq!(shown(&editor::ImageWidth50).as_deref(), Some("ctrl-alt-2"));
        assert_eq!(shown(&editor::InsertImage).as_deref(), Some("ctrl-alt-i"));
        assert_eq!(
            shown(&tools::EditImageDescription).as_deref(),
            Some("ctrl-alt-d")
        );
    });

    // The shortcuts file has an entry for each, with the key it was given.
    let out = tempfile::tempdir().unwrap();
    let file = out.path().join("The Writer Shortcuts.json");
    cx.dispatch_action(writer_app::actions::ExportShortcuts);
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "the save panel is asked");
    let target = file.clone();
    cx.simulate_new_path_selection(move |_| Some(target));
    cx.run_until_parked();
    let exported: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
    for (action, label, key) in COMMANDS {
        let entry = exported["shortcuts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["command"] == action)
            .unwrap_or_else(|| panic!("{action} is an entry of the file"));
        let title = label.split_once(": ").unwrap().1;
        assert_eq!(entry["title"], title);
        assert_eq!(entry["where"], "In the text", "{action}");
        assert_eq!(entry["default"], serde_json::json!([]));
        assert_eq!(entry["keys"], serde_json::json!([key]));
    }
}

#[gpui::test]
fn an_image_command_without_a_selected_image_changes_nothing_and_says_what_it_needs(
    cx: &mut TestAppContext,
) {
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 200, 150, RED, cx);
    let (_, workspace, cx) = open_with(&around(&image), &library, cx);
    // The caret is in the paragraph under the image.
    caret_on(&workspace, 2, cx);
    let before = workspace.read_with(cx, |w, cx| w.editor().read(cx).document().clone());
    let depth = undo_depth(&workspace, cx);
    let unchanged = |what: &str, cx: &mut VisualTestContext| {
        assert_eq!(
            workspace.read_with(cx, |w, cx| w.editor().read(cx).document().clone()),
            before,
            "{what} changed the document"
        );
        assert_eq!(
            undo_depth(&workspace, cx),
            depth,
            "{what} left a step of undo"
        );
        assert_eq!(toasts(&workspace, cx), [SELECT_AN_IMAGE_FIRST], "{what}");
        clear_toasts(&workspace, cx);
    };
    assert_eq!(
        SELECT_AN_IMAGE_FIRST,
        "Select an image first: click it, or move the caret onto it."
    );

    // The menu bar: the items are there (a document is open), and say what
    // they need.
    for item in [
        "25 %",
        "50 %",
        "75 %",
        "100 %",
        "Original Size",
        "Description…",
    ] {
        let action = menu_bar_item("Format", item);
        assert!(is_image_command(action.as_ref()), "{item}");
        assert!(available(action.as_ref(), cx), "{item} is not greyed out");
        choose_in_menu_bar("Format", item, cx);
        unchanged(item, cx);
        assert_eq!(
            workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool()),
            None
        );
    }
    // The palette.
    run_in_palette("image width 50", cx);
    unchanged("the palette", cx);
    assert!(editor_focused(&workspace, cx));
    run_in_palette("image description", cx);
    unchanged("Image description in the palette", cx);

    // With the keyboard in the library: the command goes to the text (as
    // every command of the text does), and the text has no image selected.
    cx.simulate_keystrokes("cmd-0");
    cx.run_until_parked();
    assert!(!editor_focused(&workspace, cx));
    choose_in_menu_bar("Format", "50 %", cx);
    unchanged("50 % from the library", cx);
    assert!(editor_focused(&workspace, cx));

    // "Larger" and "Smaller" in Format > Image are the commands of the
    // text's size: without an image they change the text, and say nothing.
    assert!(!is_image_command(
        menu_bar_item("Format", "Larger").as_ref()
    ));
    choose_in_menu_bar("Format", "Larger", cx);
    assert!(toasts(&workspace, cx).is_empty());
    assert!(
        workspace.read_with(cx, |w, cx| {
            w.editor()
                .read(cx)
                .document()
                .block(2)
                .unwrap()
                .text_size()
                .is_some()
        }),
        "the paragraph is larger"
    );
    cx.simulate_keystrokes("cmd-z");

    // With the image selected the same items work, from the menu bar and
    // with the keyboard in the library.
    caret_on(&workspace, 1, cx);
    choose_in_menu_bar("Format", "50 %", cx);
    assert_eq!(width_of(&workspace, 1, cx), 50);
    cx.simulate_keystrokes("cmd-0");
    cx.run_until_parked();
    choose_in_menu_bar("Format", "25 %", cx);
    assert_eq!(width_of(&workspace, 1, cx), 25);
    assert!(editor_focused(&workspace, cx));
    choose_in_menu_bar("Format", "Larger", cx);
    assert_eq!(width_of(&workspace, 1, cx), 33);
    choose_in_menu_bar("Format", "Description…", cx);
    assert_eq!(
        workspace.read_with(cx, |w, cx| w.tools().read(cx).active_tool()),
        Some(ToolKind::ImageDescription)
    );
    // The field of the description has the keyboard, and a width chosen
    // in the menu bar meanwhile still goes to the selected image.
    assert!(!editor_focused(&workspace, cx));
    choose_in_menu_bar("Format", "75 %", cx);
    assert_eq!(width_of(&workspace, 1, cx), 75);
    assert!(toasts(&workspace, cx).is_empty());
}

// ----- the menu bar -----------------------------------------------------------------------------

#[gpui::test]
fn the_menu_bar_has_insert_image_and_format_image(cx: &mut TestAppContext) {
    init(cx);
    let all = menus::app_menus();
    let submenu = |menu: &str, name: &str| -> Vec<String> {
        let found = all.iter().find(|candidate| candidate.name == menu).unwrap();
        found
            .items
            .iter()
            .find_map(|item| match item {
                gpui::MenuItem::Submenu(submenu) if submenu.name == name => Some(
                    submenu
                        .items
                        .iter()
                        .map(|item| match item {
                            gpui::MenuItem::Action { name, .. } => name.to_string(),
                            gpui::MenuItem::Separator => "-".to_string(),
                            _ => "?".to_string(),
                        })
                        .collect(),
                ),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{menu} has no submenu {name}"))
    };
    assert_eq!(
        submenu("Format", "Image"),
        [
            "Larger",
            "Smaller",
            "-",
            "25 %",
            "50 %",
            "75 %",
            "100 %",
            "Original Size",
            "-",
            "Description…"
        ]
    );
    // After "Align", before the sizes of the text.
    let format: Vec<String> = all
        .iter()
        .find(|menu| menu.name == "Format")
        .unwrap()
        .items
        .iter()
        .map(|item| match item {
            gpui::MenuItem::Action { name, .. } => name.to_string(),
            gpui::MenuItem::Submenu(menu) => format!("{} >", menu.name),
            _ => "-".to_string(),
        })
        .collect();
    let align = format.iter().position(|item| item == "Align >").unwrap();
    assert_eq!(
        format[align..align + 5],
        ["Align >", "Image >", "-", "Larger Text", "Smaller Text"]
    );
    // Insert: Image… stands with the link and the comment.
    let insert: Vec<String> = all
        .iter()
        .find(|menu| menu.name == "Insert")
        .unwrap()
        .items
        .iter()
        .map(|item| match item {
            gpui::MenuItem::Action { name, .. } => name.to_string(),
            _ => "-".to_string(),
        })
        .collect();
    assert_eq!(
        insert,
        [
            "Divider",
            "Page Break",
            "Line Break",
            "Today's Date",
            "-",
            "Image…",
            "Link…",
            "Comment",
            "-",
            "Insert Menu…"
        ]
    );
    // The actions behind them.
    let action = |menu: &str, item: &str| menu_bar_item(menu, item).name().to_string();
    assert_eq!(action("Insert", "Image…"), "editor::InsertImage");
    assert_eq!(action("Format", "Larger"), "editor::IncreaseTextSize");
    assert_eq!(action("Format", "Smaller"), "editor::DecreaseTextSize");
    assert_eq!(action("Format", "25 %"), "editor::ImageWidth25");
    assert_eq!(action("Format", "100 %"), "editor::ImageWidth100");
    assert_eq!(
        action("Format", "Original Size"),
        "editor::ImageOriginalSize"
    );
    assert_eq!(
        action("Format", "Description…"),
        "editor_tools::EditImageDescription"
    );
}

// ----- a description being written when the window goes -----------------------------------------

/// The description of the image in the stored document `id`.
fn stored_description(
    library: &Entity<Library>,
    id: DocumentId,
    cx: &mut TestAppContext,
) -> String {
    let content = library.read_with(cx, |library, _| library.document(id).unwrap().content);
    Document::from_json(&content)
        .unwrap()
        .images()
        .map(|(_, image)| image.description.clone())
        .next()
        .expect("the stored document has its image")
}

/// The popover of a description keeps what was typed however it closes
/// (`writer_editor_tools::image_description`), and closing the window or
/// quitting is one more way: no key and no click comes first that would
/// close it, so the workspace puts the field into the text before the
/// last save. (Found in review: cmd-q, cmd-w and SIGTERM with the popover
/// open saved the image without its description.)
#[gpui::test]
fn a_description_being_written_is_kept_when_the_window_closes_or_the_app_quits(
    cx: &mut TestAppContext,
) {
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 240, GREEN, cx);
    let document = around(&image);
    // cmd-w, the window's close button, and quitting (cmd-q, the menu bar,
    // a signal: all of them are `cx.quit()`). Quitting ends the app, so it
    // is the last.
    for (way, words) in [
        ("cmd-w", "The harbour at dusk"),
        ("the close button", "The pier in the rain"),
        ("quitting", "Boats on the slipway"),
    ] {
        let id = {
            let (id, workspace, vcx) = open_with(&document, &library, cx);
            caret_on(&workspace, 1, vcx);
            vcx.update(|window, cx| {
                window.dispatch_action(Box::new(tools::EditImageDescription), cx)
            });
            vcx.run_until_parked();
            assert_eq!(
                workspace.read_with(vcx, |w, cx| w.tools().read(cx).active_tool()),
                Some(ToolKind::ImageDescription),
                "{way}"
            );
            vcx.simulate_input(words);
            assert_eq!(images_in(&workspace, vcx)[0].1.description, "", "{way}");
            match way {
                "cmd-w" => vcx.simulate_keystrokes("cmd-w"),
                "the close button" => {
                    assert!(vcx.simulate_close(), "the window may close");
                    vcx.update(|window, _| window.remove_window());
                }
                // Below, on the app's own context: quitting drops the
                // window's test context.
                _ => {}
            }
            vcx.run_until_parked();
            id
        };
        if way == "quitting" {
            cx.quit();
            cx.run_until_parked();
        }
        assert_eq!(stored_description(&library, id, cx), words, "{way}");
    }
}

// ----- exports ----------------------------------------------------------------------------------

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[gpui::test]
fn the_exports_carry_the_pictures(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let bytes = png(400, 300, BLUE);
    let image = stored_png(&library, 400, 300, BLUE, cx);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Harbour"),
        Block::paragraph("Before."),
        picture(&image, 50, Alignment::Center, "The harbour at dusk"),
        Block::paragraph("After."),
    ]);
    let (_, workspace, cx) = open_with(&document, &library, cx);
    let prepare = |format: ExportFormat, cx: &mut VisualTestContext| {
        workspace
            .update(cx, |w, cx| {
                w.prepare_export(ExportScope::Document, format, cx)
            })
            .expect("the export is made")
    };

    // A Word document holds the picture.
    let docx = prepare(ExportFormat::Docx, cx);
    assert_eq!(docx.file_name, "Harbour.docx");
    assert!(
        docx.assets.is_empty(),
        "nothing goes beside a Word document"
    );
    assert!(
        contains(&docx.bytes, b"word/media/image1.png"),
        "the package has the picture among its parts"
    );

    // Plain text has the description on a line of its own.
    let plain = String::from_utf8(prepare(ExportFormat::PlainText, cx).bytes).unwrap();
    assert!(
        plain.lines().any(|line| line == "The harbour at dusk"),
        "{plain:?}"
    );

    // A Markdown file links the picture in a folder beside it.
    let markdown = prepare(ExportFormat::Markdown, cx);
    assert_eq!(markdown.file_name, "Harbour.md");
    let digits: String = image.id.to_string().chars().take(8).collect();
    let linked = format!("Harbour images/image-{digits}.png");
    assert_eq!(
        markdown
            .assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect::<Vec<_>>(),
        [linked.as_str()]
    );
    assert_eq!(markdown.assets[0].bytes, bytes);
    assert_eq!(markdown.pictures_folder(), Some("Harbour images"));
    let text = String::from_utf8(markdown.bytes.clone()).unwrap();
    assert!(text.contains("![The harbour at dusk]("), "{text}");
    assert!(
        text.contains(&linked) || text.contains(&linked.replace(' ', "%20")),
        "the Markdown links the file: {text}"
    );
    assert!(!text.contains("image:"), "{text}");

    // Through the save panel, under a name of the writer's own: the
    // folder keeps the name the links have.
    let out = tempfile::tempdir().unwrap();
    let target = out.path().join("Chapter one.md");
    workspace.update_in(cx, |w, window, cx| {
        w.export(
            ExportChoice::new(
                ExportScope::Document,
                ExportFormat::Markdown,
                ExportDestination::File,
            ),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path(), "the save panel opened");
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), text);
    let written = out
        .path()
        .join("Harbour images")
        .join(format!("image-{digits}.png"));
    assert_eq!(std::fs::read(&written).unwrap(), bytes);
    assert_eq!(
        toasts(&workspace, cx),
        [format!(
            "Exported “Harbour” to {}. Its 1 picture is in “Harbour images”.",
            short_path(&target)
        )]
    );
    assert_eq!(
        pictures_sentence(3, "Part one images", 4),
        "Their 3 pictures are in “Part one images”."
    );
    clear_toasts(&workspace, cx);

    // Markdown for the clipboard names the image as the library does:
    // there is no place to put a file.
    workspace.update_in(cx, |w, window, cx| {
        w.export(
            ExportChoice::new(
                ExportScope::Document,
                ExportFormat::Markdown,
                ExportDestination::Clipboard,
            ),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert!(
        copied.contains(&format!("![The harbour at dusk](image:{})", image.id)),
        "{copied}"
    );

    // A Word document without a picture says nothing about pictures, and
    // an image whose bytes the library does not have is exported as what
    // it shows.
    let missing = Image::new(ImageId::from_bytes([9; 16]), (800, 600), 72);
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    editor.update(cx, |editor, cx| {
        editor.edit(
            |state| {
                state.set_caret(Position::new(3, 0));
                let mut missing = missing.clone();
                missing.description = "A map that is gone".to_string();
                state.insert_image(missing, Alignment::Left)
            },
            cx,
        );
    });
    let markdown = prepare(ExportFormat::Markdown, cx);
    assert_eq!(markdown.assets.len(), 1, "only the picture that is there");
    let plain = String::from_utf8(prepare(ExportFormat::PlainText, cx).bytes).unwrap();
    assert!(plain.contains("A map that is gone"), "{plain:?}");
    let docx = prepare(ExportFormat::Docx, cx);
    assert!(contains(&docx.bytes, b"word/media/image1.png"));
    assert!(!contains(&docx.bytes, b"word/media/image2.png"));
}

/// An export whose pictures cannot be read from the library (here
/// `images.db` of a newer version) is not an export of missing pictures:
/// it fails, says why, and writes nothing, as `writer export` does. What
/// reads no picture (plain text, Markdown for the clipboard) still works.
/// (Found in review: the source took every error of the store for a
/// picture the library does not have, and the success toast came.)
#[gpui::test]
fn an_export_whose_pictures_cannot_be_read_fails_and_writes_nothing(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let id = {
        let (library, workspace, vcx) = disk.launch(None, cx);
        let image = stored_png(&library, 400, 300, BLUE, vcx);
        let document = Document::from_blocks(vec![
            Block::with_text(BlockKind::heading(1), "Harbour"),
            picture(&image, 50, Alignment::Center, "The harbour at dusk"),
        ]);
        let id = add_document(&library, None, &document, vcx);
        quit(&workspace, vcx);
        id
    };
    {
        let images = rusqlite::Connection::open(disk.images_file()).unwrap();
        images.pragma_update(None, "user_version", 99).unwrap();
    }

    let (_library, workspace, vcx) = disk.launch(None, cx);
    workspace.update_in(vcx, |w, window, cx| {
        assert!(w.open_document(id, true, window, cx));
    });
    vcx.run_until_parked();
    clear_toasts(&workspace, vcx);
    let prepare = |format: ExportFormat, cx: &mut VisualTestContext| {
        workspace.update(cx, |w, cx| {
            w.prepare_export(ExportScope::Document, format, cx)
        })
    };
    for format in [ExportFormat::Docx, ExportFormat::Markdown] {
        let error = prepare(format, vcx).err().expect("the export fails");
        assert!(
            error.starts_with("its pictures could not be read from the library (")
                && error.ends_with("), so nothing was written"),
            "{format:?}: {error}"
        );
    }
    let plain = prepare(ExportFormat::PlainText, vcx).expect("plain text reads no picture");
    assert!(
        String::from_utf8(plain.bytes)
            .unwrap()
            .contains("The harbour at dusk")
    );

    // As the writer does it: the toast says so and no save panel opens.
    workspace.update_in(vcx, |w, window, cx| {
        w.export(
            ExportChoice::new(
                ExportScope::Document,
                ExportFormat::Docx,
                ExportDestination::File,
            ),
            window,
            cx,
        )
    });
    vcx.run_until_parked();
    assert!(!vcx.did_prompt_for_new_path(), "nothing is written");
    let said = toasts(&workspace, vcx);
    assert_eq!(said.len(), 1, "{said:?}");
    assert!(
        said[0].starts_with("Could not export: its pictures could not be read from the library"),
        "{said:?}"
    );
}

// ----- print ------------------------------------------------------------------------------------

/// Lets the editor's background layout measure every block.
fn settle(workspace: &Entity<Workspace>, cx: &mut VisualTestContext) {
    let editor = workspace.read_with(cx, |w, _| w.editor().clone());
    for _ in 0..5000 {
        if !editor.read_with(cx, |editor, _| editor.layout_pending()) {
            return;
        }
        cx.executor().advance_clock(Duration::from_millis(2));
        cx.run_until_parked();
    }
    panic!("the background layout did not settle");
}

/// A document in page view with a picture used twice, a picture the
/// library does not have, and enough text for a second page.
fn illustrated(image: &Image) -> Document {
    let missing = Image::new(ImageId::from_bytes([9; 16]), (800, 600), 72);
    let mut blocks = vec![
        Block::with_text(BlockKind::heading(1), "Harbour"),
        picture(image, 100, Alignment::Center, "The harbour at dusk"),
    ];
    blocks.extend((0..12).map(|index| {
        Block::paragraph(&format!(
            "Paragraph {index}. The boats came by road every spring, and nobody asked why."
        ))
    }));
    blocks.push(picture(image, 25, Alignment::Right, ""));
    blocks.push(picture(&missing, 50, Alignment::Left, "A map that is gone"));
    blocks.push(Block::paragraph("The end."));
    Document::from_blocks(blocks).with_settings(DocumentSettings {
        page_view: true,
        ..DocumentSettings::default()
    })
}

fn print_and_wait(
    workspace: &Entity<Workspace>,
    jobs: &std::rc::Rc<std::cell::RefCell<Vec<printing::PrintJob>>>,
    cx: &mut VisualTestContext,
) -> printing::PrintJob {
    let before = jobs.borrow().len();
    cx.dispatch_action(writer_app::actions::Print);
    for _ in 0..20_000 {
        cx.run_until_parked();
        if jobs.borrow().len() > before && workspace.read_with(cx, |w, _| w.printing()).is_none() {
            return jobs.borrow().last().unwrap().clone();
        }
        cx.executor().advance_clock(Duration::from_millis(1));
    }
    panic!("the pages were never done");
}

#[gpui::test]
fn a_print_has_the_pictures_of_the_document(cx: &mut TestAppContext) {
    init(cx);
    let jobs = cx.update(printing::record);
    let library = library(cx);
    let image = stored_png(&library, 800, 500, GREEN, cx);
    let (_, workspace, cx) = open_with(&illustrated(&image), &library, cx);

    let job = print_and_wait(&workspace, &jobs, cx);
    // One picture is in the PDF (used twice, held once); the one the
    // library does not have is its box and its description.
    assert_eq!(job.pdf.report.images, 1);
    assert_eq!(job.pdf.report.images_unreadable, 0);
    settle(&workspace, cx);
    let pages = workspace.read_with(cx, |w, cx| w.editor().read(cx).page_count());
    assert_eq!(Some(job.pdf.report.pages), pages, "page view's pages");
    assert!(job.pdf.report.pages >= 2);
    let file = writer_print::PdfFile::open(job.pdf.bytes()).expect("a PDF");
    assert_eq!(file.page_count(), job.pdf.report.pages);

    // The job made at once (a scripted step) has the picture too.
    let at_once = workspace
        .update_in(cx, |w, window, cx| w.print_job(window, cx))
        .expect("a document is open");
    assert_eq!(at_once.pdf.report.images, 1);
    assert_eq!(at_once.pdf.report.pages, job.pdf.report.pages);
    assert!(
        job.pdf.len() > 2_000,
        "the PDF holds more than its text: {} bytes",
        job.pdf.len()
    );
}

/// With a library on disk the pictures of a print are read through the
/// provider's own connection, in the task that makes the pages, before
/// the first page is made.
#[gpui::test]
fn on_disk_a_print_reads_its_pictures_in_the_background(cx: &mut TestAppContext) {
    init(cx);
    let jobs = cx.update(printing::record);
    let disk = Disk::new();
    let (library, workspace, cx) = disk.launch(None, cx);
    let image = stored_png(&library, 800, 500, GREEN, cx);
    let id = add_document(&library, None, &illustrated(&image), cx);
    workspace.update_in(cx, |w, window, cx| {
        assert!(w.open_document(id, true, window, cx));
    });
    cx.run_until_parked();

    // A slice is one block, and the next comes a millisecond of the test
    // clock later: the job can be seen while it prepares.
    cx.update(|_, cx| printing::set_slice(Duration::ZERO, cx));
    let preparing = Some(writer_app::workspace::Printing::Preparing);
    cx.dispatch_action(writer_app::actions::Print);
    assert_eq!(workspace.read_with(cx, |w, _| w.printing()), preparing);
    assert!(jobs.borrow().is_empty());
    let job = print_and_wait_for(&workspace, &jobs, cx);
    // The picture came through the provider's own connection to
    // images.db.
    assert_eq!(job.pdf.report.images, 1);
    // Cancelled on the way: nothing is handed over.
    cx.dispatch_action(writer_app::actions::Print);
    assert_eq!(workspace.read_with(cx, |w, _| w.printing()), preparing);
    cx.dispatch_action(writer_app::actions::CancelPrint);
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(workspace.read_with(cx, |w, _| w.printing()), None);
    assert_eq!(jobs.borrow().len(), 1);
}

/// Waits for the job a "Print…" that was already dispatched hands over.
fn print_and_wait_for(
    workspace: &Entity<Workspace>,
    jobs: &std::rc::Rc<std::cell::RefCell<Vec<printing::PrintJob>>>,
    cx: &mut VisualTestContext,
) -> printing::PrintJob {
    for _ in 0..20_000 {
        cx.run_until_parked();
        if !jobs.borrow().is_empty() && workspace.read_with(cx, |w, _| w.printing()).is_none() {
            return jobs.borrow().last().unwrap().clone();
        }
        cx.executor().advance_clock(Duration::from_millis(1));
    }
    panic!("the pages were never done");
}

// ----- the collector ----------------------------------------------------------------------------

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

fn collected_at(library: &Entity<Library>, cx: &mut VisualTestContext) -> Option<i64> {
    library.read_with(cx, |library, _| {
        library.setting::<i64>(keys::IMAGES_COLLECTED_AT).unwrap()
    })
}

#[gpui::test]
fn the_collector_runs_a_moment_after_a_launch_and_not_again_within_a_day(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let files = tempfile::tempdir().unwrap();
    let first = png(200, 120, RED);
    let second = png(220, 120, GREEN);
    let clock = Clock::manual(1_800_000_000_000);

    // The first launch, as `main` starts it.
    {
        let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
        vcx.update(|_, cx| images::collect_in_background(&library, cx));
        vcx.simulate_keystrokes("cmd-n");
        vcx.run_until_parked();
        insert_files(
            &workspace,
            vec![file_in(files.path(), "first.png", &first)],
            vcx,
        );
        vcx.run_until_parked();
        assert_eq!(images_in(&workspace, vcx).len(), 1);
        let_it_save(&workspace, vcx);
        assert_eq!(collected_at(&library, vcx), None, "not before its time");
        assert!(disk.in_the_copy(id_of(&first)).is_none());

        clock.advance_ms(20_000);
        vcx.executor().advance_clock(COLLECT_DELAY);
        vcx.run_until_parked();
        assert_eq!(
            collected_at(&library, vcx),
            Some(clock.now_ms()),
            "the run is noted with its time"
        );
        // The picture is named, and it is there; and the run ended by
        // bringing the copy of the pictures among the backups up to date.
        assert_eq!(
            disk.stored(id_of(&first)).as_deref(),
            Some(first.as_slice())
        );
        assert_eq!(
            disk.in_the_copy(id_of(&first)).as_deref(),
            Some(first.as_slice())
        );
        quit(&workspace, vcx);
    }
    let ran = clock.now_ms();

    // An hour later, a second launch: the collector does not run again,
    // and a picture that came in since is copied among the backups.
    clock.advance_ms(60 * 60 * 1000);
    {
        let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
        vcx.update(|_, cx| images::collect_in_background(&library, cx));
        vcx.simulate_keystrokes("cmd-down");
        insert_files(
            &workspace,
            vec![file_in(files.path(), "second.png", &second)],
            vcx,
        );
        vcx.run_until_parked();
        assert_eq!(images_in(&workspace, vcx).len(), 2);
        assert!(disk.in_the_copy(id_of(&second)).is_none());
        vcx.executor().advance_clock(COLLECT_DELAY);
        vcx.run_until_parked();
        assert_eq!(collected_at(&library, vcx), Some(ran), "no second run");
        assert_eq!(
            disk.in_the_copy(id_of(&second)).as_deref(),
            Some(second.as_slice())
        );
        quit(&workspace, vcx);
    }

    // More than a day after the run: the next launch runs it again.
    clock.advance_ms(DAY_MS);
    let (library, _workspace, vcx) = disk.launch(Some(&clock), cx);
    vcx.update(|_, cx| images::collect_in_background(&library, cx));
    vcx.executor().advance_clock(COLLECT_DELAY);
    vcx.run_until_parked();
    assert_eq!(collected_at(&library, vcx), Some(clock.now_ms()));
    assert!(clock.now_ms() > ran + DAY_MS);
    // Both pictures are named by the document and are there.
    assert!(disk.stored(id_of(&first)).is_some());
    assert!(disk.stored(id_of(&second)).is_some());
}

/// While the app stays open it looks once a day at which images are
/// named, and deletes nothing: an image that only the undo history holds
/// is still in the file, however long the app has been open.
#[gpui::test]
fn while_the_app_stays_open_it_looks_once_a_day_and_deletes_nothing(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let files = tempfile::tempdir().unwrap();
    let undone = png(200, 120, RED);
    let later = png(220, 120, GREEN);
    let clock = Clock::manual(1_800_000_000_000);
    let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
    vcx.update(|_, cx| images::collect_in_background(&library, cx));
    vcx.simulate_keystrokes("cmd-n");
    vcx.run_until_parked();
    vcx.simulate_input("A line.");

    // A picture comes in and is taken out again with undo: nothing that
    // is stored names it, only the undo history of the open editor.
    insert_files(
        &workspace,
        vec![file_in(files.path(), "undone.png", &undone)],
        vcx,
    );
    vcx.run_until_parked();
    vcx.simulate_keystrokes("cmd-z");
    assert!(images_in(&workspace, vcx).is_empty());
    let_it_save(&workspace, vcx);

    // The run at launch.
    clock.advance_ms(20_000);
    vcx.executor().advance_clock(COLLECT_DELAY);
    vcx.run_until_parked();
    let ran = collected_at(&library, vcx).expect("the collector ran");
    assert!(disk.stored(id_of(&undone)).is_some());

    // The app stays open for forty days, and the writer adds a picture
    // on the way. Every day the app looks; it never deletes. (The
    // keyboard is in the library meanwhile: a caret that blinks in the
    // text is a timer every half second of the test's clock.)
    vcx.simulate_keystrokes("cmd-0");
    vcx.run_until_parked();
    assert!(!editor_focused(&workspace, vcx));
    for day in 1..=40 {
        if day == 2 {
            insert_files(
                &workspace,
                vec![file_in(files.path(), "later.png", &later)],
                vcx,
            );
            vcx.run_until_parked();
            let_it_save(&workspace, vcx);
            assert!(
                disk.in_the_copy(id_of(&later)).is_none(),
                "not copied before the next look"
            );
        }
        clock.advance_ms(DAY_MS);
        vcx.executor().advance_clock(LOOK_EVERY);
        vcx.run_until_parked();
        if day == 2 {
            // The look ends by bringing the copy up to date: that it ran
            // can be seen there.
            assert_eq!(
                disk.in_the_copy(id_of(&later)).as_deref(),
                Some(later.as_slice())
            );
        }
    }
    assert_eq!(
        collected_at(&library, vcx),
        Some(ran),
        "the look is not the collector"
    );
    assert_eq!(
        disk.stored(id_of(&undone)).as_deref(),
        Some(undone.as_slice()),
        "unnamed for forty days, and still there"
    );

    // Redo brings the image back, and its picture is there to show.
    vcx.simulate_keystrokes("cmd-1");
    vcx.run_until_parked();
    assert!(editor_focused(&workspace, vcx));
    vcx.simulate_keystrokes("cmd-z");
    assert!(
        images_in(&workspace, vcx).is_empty(),
        "the later picture is undone"
    );
    vcx.simulate_keystrokes("cmd-shift-z cmd-shift-z");
    let_pictures_arrive(vcx);
    let found: Vec<ImageId> = images_in(&workspace, vcx)
        .into_iter()
        .map(|(_, image)| image.id)
        .collect();
    assert!(found.contains(&id_of(&later)), "{found:?}");
    assert_eq!(status(&workspace, id_of(&later), vcx), ImageStatus::Ready);
}

/// The notes on screen, oldest first, each with the label of its button.
fn notes(
    workspace: &Entity<Workspace>,
    cx: &mut VisualTestContext,
) -> Vec<(String, Option<String>)> {
    workspace.read_with(cx, |w, cx| {
        w.toast_layer()
            .read(cx)
            .toasts()
            .map(|toast| {
                (
                    toast.message.to_string(),
                    toast.action_label().map(ToString::to_string),
                )
            })
            .collect()
    })
}

/// A copy of the pictures among the backups that cannot be brought up to
/// date is said once a launch, not only on stderr: it is what would bring
/// back a picture that `images.db` loses. (Handed over by the store's
/// review: `ImagesCollected::copy_failed` was thrown away.)
#[gpui::test]
fn a_copy_of_the_pictures_that_is_behind_is_said_once_a_launch(cx: &mut TestAppContext) {
    use std::os::unix::fs::PermissionsExt;
    init(cx);
    let disk = Disk::new();
    let clock = Clock::manual(1_800_000_000_000);
    let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
    vcx.update(|_, cx| images::collect_in_background(&library, cx));
    let image = stored_png(&library, 200, 120, RED, vcx);
    add_document(&library, None, &around(&image), vcx);
    // The backups directory cannot be written to: the copy cannot be made.
    let backups = writer_library::store::images_backup_path(&disk.database())
        .parent()
        .unwrap()
        .to_path_buf();
    std::fs::create_dir_all(&backups).unwrap();
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o500)).unwrap();
    clear_toasts(&workspace, vcx);

    clock.advance_ms(20_000);
    vcx.executor().advance_clock(COLLECT_DELAY);
    vcx.run_until_parked();
    let said = notes(&workspace, vcx);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].0.starts_with(
                "The backup copy of the pictures could not be brought up to date, and is tried again later ("
            ),
            "{said:?}"
        );
        assert!(collected_at(&library, vcx).is_some(), "the collection ran");
        assert_eq!(disk.in_the_copy(image.id), None);

        // A day later it is still behind: stderr says so, the note not
        // again. (The keyboard is in the library: no caret blinks all day.)
        vcx.simulate_keystrokes("cmd-0");
        clear_toasts(&workspace, vcx);
        clock.advance_ms(DAY_MS);
        vcx.executor().advance_clock(images::LOOK_EVERY);
        vcx.run_until_parked();
        assert_eq!(notes(&workspace, vcx), [], "once a launch");
    }));
    std::fs::set_permissions(&backups, std::fs::Permissions::from_mode(0o700)).unwrap();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
    // Once it can be written, the next look brings it up to date.
    clock.advance_ms(DAY_MS);
    vcx.executor().advance_clock(images::LOOK_EVERY);
    vcx.run_until_parked();
    assert!(disk.in_the_copy(image.id).is_some());
    assert_eq!(
        copy_not_updated("x"),
        "The backup copy of the pictures could not be brought up to date, and is tried again later (x)."
    );
}

/// A damaged `images.db` that the store set aside (and made again from the
/// copy) is told of once, by its path, with a way to it in the Finder: a
/// picture only it had is in it and nowhere else. (Handed over by the
/// store's review: `writer_store::images_moved_aside`.)
#[gpui::test]
fn a_damaged_pictures_file_that_was_set_aside_is_told_of_once(cx: &mut TestAppContext) {
    init(cx);
    let disk = Disk::new();
    let clock = Clock::manual(1_800_000_000_000);
    let image = {
        let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
        vcx.update(|_, cx| images::collect_in_background(&library, cx));
        let image = stored_png(&library, 200, 120, GREEN, vcx);
        add_document(&library, None, &around(&image), vcx);
        clock.advance_ms(20_000);
        vcx.executor().advance_clock(COLLECT_DELAY);
        vcx.run_until_parked();
        assert!(disk.in_the_copy(image.id).is_some(), "the copy is made");
        assert_eq!(notes(&workspace, vcx), [], "nothing to say");
        quit(&workspace, vcx);
        image
    };
    // images.db goes bad while the app is closed.
    for end in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{end}", disk.images_file().display()));
        std::fs::remove_file(&path).ok();
    }
    std::fs::write(
        disk.images_file(),
        b"not a database, only some text that is long enough",
    )
    .unwrap();

    clock.advance_ms(2 * DAY_MS);
    let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
    vcx.update(|_, cx| images::collect_in_background(&library, cx));
    clear_toasts(&workspace, vcx);
    vcx.executor().advance_clock(COLLECT_DELAY);
    vcx.run_until_parked();
    // The collector found it damaged, set it aside and made it again from
    // the copy.
    let aside = writer_library::store::images_moved_aside(&disk.database());
    assert_eq!(aside.len(), 1, "{aside:?}");
    assert_eq!(disk.stored(image.id).is_some(), true, "back from the copy");
    assert_eq!(
        notes(&workspace, vcx),
        [(
            images::moved_aside_note(&aside[0]),
            Some("Show in Finder".to_string())
        )]
    );
    assert!(
        images::moved_aside_note(&aside[0])
            .starts_with("A damaged pictures file was set aside as ")
    );
    let told: Vec<String> = library.read_with(vcx, |library, _| {
        library.setting_or(keys::IMAGES_MOVED_ASIDE_TOLD, Vec::new())
    });
    assert_eq!(told, [aside[0].display().to_string()]);

    // Once: not at the next look, nor at the next launch. (The keyboard
    // is in the library meanwhile: a caret that blinks in the text is a
    // timer every half second of the test's clock.)
    vcx.simulate_keystrokes("cmd-0");
    clear_toasts(&workspace, vcx);
    clock.advance_ms(DAY_MS);
    vcx.executor().advance_clock(images::LOOK_EVERY);
    vcx.run_until_parked();
    assert_eq!(notes(&workspace, vcx), []);
    quit(&workspace, vcx);
    let (library, workspace, vcx) = disk.launch(Some(&clock), cx);
    vcx.update(|_, cx| images::collect_in_background(&library, cx));
    clear_toasts(&workspace, vcx);
    vcx.executor().advance_clock(COLLECT_DELAY);
    vcx.run_until_parked();
    assert_eq!(notes(&workspace, vcx), []);
}

/// A library in memory has no file to collect in.
#[gpui::test]
fn a_library_in_memory_has_no_collector(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    cx.update(|cx| images::collect_in_background(&library, cx));
    let (_workspace, vcx) = open_workspace(&library, cx);
    vcx.executor().advance_clock(COLLECT_DELAY * 2);
    vcx.run_until_parked();
    assert_eq!(collected_at(&library, vcx), None);
}

// ----- versions ---------------------------------------------------------------------------------

#[gpui::test]
fn the_version_history_shows_an_image_as_what_it_reads_and_a_restore_brings_it_back(
    cx: &mut TestAppContext,
) {
    init(cx);
    let library = library(cx);
    let described = stored_png(&library, 400, 300, RED, cx);
    let plain = stored_png(&library, 300, 300, GREEN, cx);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Chapter"),
        picture(&described, 50, Alignment::Right, "The harbour at dusk"),
        picture(&plain, 100, Alignment::Center, ""),
        Block::paragraph("Text."),
    ]);
    let (id, workspace, cx) = open_with(&document, &library, cx);
    library.update(cx, |library, cx| {
        library.create_snapshot(id, cx).expect("a version");
    });

    // The writer deletes both pictures.
    caret_on(&workspace, 1, cx);
    cx.simulate_keystrokes("backspace");
    assert_eq!(images_in(&workspace, cx).len(), 1);
    let on_image = selected_image(&workspace, cx).map(|(block, _)| block);
    if on_image.is_none() {
        caret_on(&workspace, 1, cx);
    }
    cx.simulate_keystrokes("backspace");
    assert!(images_in(&workspace, cx).is_empty());

    // The preview is text: an image is its description, or "Image".
    cx.simulate_keystrokes("cmd-shift-p");
    cx.simulate_input("version history");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let view = workspace
        .read_with(cx, |w, cx| {
            w.modal_layer().read(cx).active::<VersionHistory>()
        })
        .expect("the version history is open");
    assert_eq!(IMAGE_WITHOUT_DESCRIPTION, "Image");
    // The newest version is what opening the history kept (the text
    // without the pictures); the one before it has them.
    cx.simulate_keystrokes("end");
    assert_eq!(
        view.read_with(cx, |view, _| view.preview_text()),
        "Chapter\nThe harbour at dusk\nImage\nText."
    );

    // Restoring brings the blocks back as they were, and their pictures.
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(editor_focused(&workspace, cx));
    let back = images_in(&workspace, cx);
    assert_eq!(
        back.iter()
            .map(|(block, image)| (*block, image.id, image.width.percent()))
            .collect::<Vec<_>>(),
        [(1, described.id, 50), (2, plain.id, 100)]
    );
    assert_eq!(back[0].1.description, "The harbour at dusk");
    let_pictures_arrive(cx);
    assert_eq!(status(&workspace, described.id, cx), ImageStatus::Ready);
    assert_eq!(status(&workspace, plain.id, cx), ImageStatus::Ready);
}

// ----- scripted runs ----------------------------------------------------------------------------

#[gpui::test]
fn a_scripted_run_inserts_and_selects_images_and_its_state_says_what_is_there(
    cx: &mut TestAppContext,
) {
    // The steps.
    let script = Script::parse(
        "image:/some/tmp/my\\sharbour.png; image:select:2; image: relative.png ;IMAGE:Select:1",
    )
    .expect("the steps parse");
    assert_eq!(
        script.steps,
        [
            Step::Image(ImageStep::Insert(PathBuf::from("/some/tmp/my harbour.png"))),
            Step::Image(ImageStep::Select(2)),
            Step::Image(ImageStep::Insert(PathBuf::from("relative.png"))),
            Step::Image(ImageStep::Insert(PathBuf::from("Select:1"))),
        ]
    );
    for wrong in ["image", "image:", "image:select:0", "image:select:two"] {
        assert!(Script::parse(wrong).is_err(), "{wrong:?} is no step");
    }

    // The state line.
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 300, RED, cx);
    let missing = Image::new(ImageId::from_bytes([9; 16]), (1600, 1067), 144);
    let document = Document::from_blocks(vec![
        Block::with_text(BlockKind::heading(1), "Harbour"),
        picture(&image, 50, Alignment::Center, "The harbour at dusk"),
        Block::paragraph("Text."),
        picture(&missing, 100, Alignment::Right, ""),
    ]);
    let (_, workspace, cx) = open_with(&document, &library, cx);
    let state = |cx: &mut VisualTestContext| {
        workspace.update_in(cx, |workspace, window, cx| {
            describe_state(workspace, window, cx)
        })
    };
    let_pictures_arrive(cx);
    caret_on(&workspace, 2, cx);
    let line = state(cx);
    assert!(line.contains(" images=2 vim="), "{line}");
    assert!(!line.contains("image=["), "{line}");

    caret_on(&workspace, 1, cx);
    let_pictures_arrive(cx);
    let line = state(cx);
    assert!(
        line.contains(
            " images=2 image=[block=1 width=50% natural=400x300@72 align=center ready \
             alt=\"The harbour at dusk\"] vim="
        ),
        "{line}"
    );
    caret_on(&workspace, 3, cx);
    let_pictures_arrive(cx);
    let line = state(cx);
    assert!(
        line.contains(
            " images=2 image=[block=3 width=100% natural=1600x1067@144 align=right missing] "
        ),
        "{line}"
    );
}

// ----- Vim mode ---------------------------------------------------------------------------------

/// Vim mode on, in the assembled app. What the keys do to a block without
/// text is the engine's and is held there (`crates/writer-vim/tests/images.rs`);
/// this holds the app to it: an image is one stop, shows as selected, and
/// no command takes it apart.
#[gpui::test]
fn in_vim_mode_an_image_is_one_stop_and_stays_whole(cx: &mut TestAppContext) {
    init(cx);
    let library = library(cx);
    let image = stored_png(&library, 400, 300, BLUE, cx);
    let document = Document::from_blocks(vec![
        Block::paragraph("Above."),
        picture(&image, 50, Alignment::Right, "The harbour at dusk"),
        Block::paragraph("Below."),
    ]);
    let (_, workspace, cx) = open_with(&document, &library, cx);
    workspace.update(cx, |w, cx| w.toggle_vim_mode(cx));
    cx.run_until_parked();
    let mode = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| {
            let vim = w.vim().read(cx);
            vim.is_attached().then(|| vim.mode())
        })
    };
    assert_eq!(mode(cx), Some(writer_vim_mode::Mode::Normal));
    let caret = |cx: &mut VisualTestContext| {
        workspace.read_with(cx, |w, cx| w.editor().read(cx).selection().head)
    };
    let whole = |cx: &mut VisualTestContext| {
        let found = images_in(&workspace, cx);
        assert_eq!(found.len(), 1, "one image");
        let (block, found) = &found[0];
        assert_eq!(found.id, image.id);
        assert_eq!(found.width, ImageWidth::from_percent(50));
        assert_eq!(found.description, "The harbour at dusk");
        assert_eq!(
            workspace.read_with(cx, |w, cx| {
                w.editor()
                    .read(cx)
                    .document()
                    .block(*block)
                    .unwrap()
                    .alignment()
            }),
            Alignment::Right
        );
        *block
    };

    // j and k stop on the image once.
    cx.simulate_input("gg");
    assert_eq!(caret(cx), Position::new(0, 0));
    cx.simulate_input("j");
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, _)| block),
        Some(1)
    );
    // It shows as every selected image does: the handles are there, and
    // the caret is the picture's own box, not a block of Vim's on it.
    workspace.read_with(cx, |w, cx| {
        let editor = w.editor().read(cx);
        assert_eq!(
            editor.config().caret_style,
            writer_editor::CaretStyle::Block
        );
        let box_of_the_picture = editor.image_bounds(1).expect("the picture is laid out");
        assert_eq!(editor.caret_bounds(), Some(box_of_the_picture));
        let handles = editor
            .image_handle_bounds()
            .expect("the handles of a selected image");
        assert_eq!(handles[0].center(), box_of_the_picture.origin);
    });
    cx.simulate_input("j");
    assert_eq!(caret(cx), Position::new(2, 0));
    cx.simulate_input("k");
    assert_eq!(
        selected_image(&workspace, cx).map(|(block, _)| block),
        Some(1)
    );
    cx.simulate_input("k");
    assert_eq!(caret(cx), Position::new(0, 0));

    // J on the line above joins nothing into the image, and x on the
    // image takes nothing away.
    cx.simulate_input("J");
    assert_eq!(editor_text(&workspace, cx), "Above.\n\nBelow.");
    assert_eq!(whole(cx), 1);
    cx.simulate_input("jx");
    assert_eq!(editor_text(&workspace, cx), "Above.\n\nBelow.");
    assert_eq!(whole(cx), 1);

    // dd takes the image out whole, and p puts it back as it was.
    cx.simulate_input("dd");
    assert!(images_in(&workspace, cx).is_empty());
    assert_eq!(editor_text(&workspace, cx), "Above.\nBelow.");
    cx.simulate_input("p");
    assert_eq!(whole(cx), 2, "after the line the caret was on");
    cx.simulate_input("uu");
    assert_eq!(whole(cx), 1);
    assert_eq!(editor_text(&workspace, cx), "Above.\n\nBelow.");

    // i and a letter on the image: a paragraph below it.
    caret_on(&workspace, 1, cx);
    cx.simulate_input("iX");
    cx.simulate_keystrokes("escape");
    assert_eq!(mode(cx), Some(writer_vim_mode::Mode::Normal));
    assert_eq!(editor_text(&workspace, cx), "Above.\n\nX\nBelow.");
    assert_eq!(whole(cx), 1);
    // And the image is still shown after all of it.
    let_pictures_arrive(cx);
    assert_eq!(status(&workspace, image.id, cx), ImageStatus::Ready);
}

// ----- words ------------------------------------------------------------------------------------

/// The welcome document says how a picture comes in and what there is to
/// do with one, and names the two keys that size it by what they are by
/// default.
#[gpui::test]
fn the_welcome_document_says_how_a_picture_comes_in(cx: &mut TestAppContext) {
    init(cx);
    let text = writer_app::welcome::welcome_document().plain_text();
    assert!(
        text.contains(
            "Image… in the Insert menu puts a picture into the text, and so does Image in the \
             insert menu, a paste, or an image file dragged in from the Finder."
        ),
        "{text}"
    );
    assert!(
        text.contains("⇧⌘. and ⇧⌘, make the selected picture larger and smaller."),
        "{text}"
    );
    // The keys it names are the keys of the two commands.
    cx.update(|cx| {
        let keymap = cx.key_bindings();
        let keymap = keymap.borrow();
        let keys = |action: &dyn Action| -> Vec<String> {
            keymap
                .bindings_for_action(action)
                .filter(|binding| !menus::is_menu_key_binding(binding))
                .map(|binding| binding.keystrokes()[0].unparse())
                .collect()
        };
        assert!(
            keys(&editor::IncreaseTextSize).contains(&"cmd-shift-.".to_string()),
            "{:?}",
            keys(&editor::IncreaseTextSize)
        );
        assert!(
            keys(&editor::DecreaseTextSize).contains(&"cmd-shift-,".to_string()),
            "{:?}",
            keys(&editor::DecreaseTextSize)
        );
    });
}

/// The read of `NSPasteboard` itself, against a pasteboard of the test's
/// own that is filled the way other apps fill the general one. The
/// general pasteboard (the user's clipboard) is neither read nor written.
#[test]
fn the_app_reads_a_real_pasteboard_the_way_it_decides() {
    use pasteboard::{FILE_URL, PLAIN_TEXT, private::Pasteboard};
    let files = tempfile::tempdir().unwrap();
    let bytes = png(64, 48, RED);
    let path = file_in(files.path(), "copied in Finder.png", &bytes);
    let fresh = || Pasteboard::new().expect("a pasteboard of the test's own");

    // An empty pasteboard has nothing.
    assert_eq!(fresh().read(), None);

    // The Finder: the file's address (with its space escaped, as an
    // address has it), and the file's name as text.
    let finder = fresh();
    let address = format!("file://{}", path.to_string_lossy().replace(' ', "%20"));
    assert!(finder.add_text(FILE_URL, &address));
    assert!(finder.add_text(PLAIN_TEXT, "copied in Finder.png"));
    match finder.read() {
        Some(NewImageSource::File(read)) => assert_eq!(
            read.canonicalize().unwrap(),
            path.canonicalize().unwrap(),
            "the path the address names"
        ),
        other => panic!("a file copied in the Finder reads as {other:?}"),
    }
    // A file that is no image is not a picture.
    let other = fresh();
    assert!(other.add_text(FILE_URL, "file:///etc/hosts"));
    assert!(other.add_text(PLAIN_TEXT, "hosts"));
    assert_eq!(other.read(), None);

    // A browser's "Copy Image": the picture, and its address as text.
    let browser = fresh();
    assert!(browser.add_data("public.png", &bytes));
    assert!(browser.add_text(PLAIN_TEXT, "https://example.com/harbour.png"));
    assert_eq!(browser.read(), Some(NewImageSource::Bytes(bytes.clone())));

    // Cells of a spreadsheet: a picture of them beside their text. Text.
    let cells = fresh();
    assert!(cells.add_data("public.png", &bytes));
    assert!(cells.add_text(PLAIN_TEXT, "Month\tSalt\nMay\t12"));
    assert_eq!(cells.read(), None);

    // A photo of a kind GPUI does not know, and no text: its data, as it
    // is on the pasteboard (what it is, is for `writer_image::prepare`).
    let photo = fresh();
    assert!(photo.add_data("public.heic", b"not really a HEIC"));
    assert_eq!(
        photo.read(),
        Some(NewImageSource::Bytes(b"not really a HEIC".to_vec()))
    );
    // And of two kinds the one the library stores as it is.
    let two = fresh();
    assert!(two.add_data("public.tiff", b"a TIFF"));
    assert!(two.add_data("public.png", &bytes));
    assert_eq!(two.read(), Some(NewImageSource::Bytes(bytes)));

    // Plain text.
    let words = fresh();
    assert!(words.add_text(PLAIN_TEXT, "The boats came by road."));
    assert_eq!(words.read(), None);
}
