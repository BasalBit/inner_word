//! The zoom of the text, as the app has it.
//!
//! The zoom itself is the editor's (`writer_editor::Zoom`): it draws the
//! view of the text larger or smaller and changes nothing of the document.
//! This module holds what the app adds around it:
//!
//! - **the commands** ([`commands`]) with their keys ([`key_bindings`]):
//!   zoom in, zoom out, actual size, fit width, fit page, life size and a
//!   command for every step the menus offer ([`MENU_STEPS`]). They are in
//!   the palette, in View of the menu bar ([`add_menu_items`]) and in the
//!   menu of the zoom item of the status bar (`workspace/zoom.rs`);
//! - **how a zoom is stored** ([`stored_form`], [`from_stored`]): each of
//!   the two views has its own, in the library's settings
//!   ([`FLOW_ZOOM_KEY`], [`PAGE_ZOOM_KEY`]). A zoom that follows the
//!   window is stored as such, and follows the window again after a
//!   relaunch;
//! - **how large the display is** ([`life_size_percent`]): the zoom at
//!   which a sheet on the glass is as large as the sheet of paper it
//!   stands for. `main` asks this Mac ([`use_this_mac`]); a test names a
//!   density ([`set_display_density`]), and without either nobody knows
//!   and there is no life size.
//!
//! # What 100 % is, and why a page looked small
//!
//! At 100 % one point of the document is one pixel of the window (a
//! logical pixel), in both views. Pages and TextEdit draw their 100 % that
//! way, and by everything that could be found Word for Mac does too, so a
//! sheet at 100 % is as wide on screen here as there. On today's displays
//! that is much smaller than paper: a Mac's built-in display has about 127
//! pixels to the inch where a point is a 72nd of one, so an A4 sheet at
//! 100 % is 12 cm wide on the glass. Life size there is 177 %.
//!
//! So a page that was never given a zoom opens as large as its paper, or
//! as wide as the window where the window is narrower than that
//! ([`first_page_zoom`]; at 100 % where the display does not say how
//! large it is); from the first zoom the writer chooses, page view shows
//! what they chose last. Flow view starts at 100 %, the size
//! its column was made to be read at.

use gpui::{Action, App, Global, KeyBinding, Menu, MenuItem, Window, actions};
use writer_editor::{ZOOM_MAX, ZOOM_MIN, Zoom};
use writer_ui::CommandDescriptor;

use crate::actions::CATEGORY_VIEW;

actions!(
    zoom,
    [
        /// Draw the text one step larger.
        ZoomIn,
        /// Draw the text one step smaller.
        ZoomOut,
        /// Draw the text at 100 %: a point of the document is a pixel.
        ActualSize,
        /// Draw the page (or the column of text) as wide as its room.
        FitWidth,
        /// Draw one whole page in the window.
        FitPage,
        /// Draw the page as large as the sheet of paper it stands for.
        LifeSize,
        /// Draw the text at 50 %.
        ZoomTo50,
        /// Draw the text at 75 %.
        ZoomTo75,
        /// Draw the text at 125 %.
        ZoomTo125,
        /// Draw the text at 150 %.
        ZoomTo150,
        /// Draw the text at 175 %.
        ZoomTo175,
        /// Draw the text at 200 %.
        ZoomTo200,
        /// Draw the text at 250 %.
        ZoomTo250,
        /// Draw the text at 300 %.
        ZoomTo300,
        /// Draw the text at 400 %.
        ZoomTo400,
    ]
);

/// The zooms the menus offer by name, in percent. "Zoom in" and "zoom
/// out" go through a finer list (`writer_editor::ZOOM_STEPS`).
pub const MENU_STEPS: [u16; 10] = [50, 75, 100, 125, 150, 175, 200, 250, 300, 400];

/// The setting that holds the zoom of flow view.
pub const FLOW_ZOOM_KEY: &str = "view.zoom.flow";
/// The setting that holds the zoom of page view. Absent until the writer
/// chooses one: see [`first_page_zoom`].
pub const PAGE_ZOOM_KEY: &str = "view.zoom.page";

/// The command that sets the zoom to `percent`, for a step of
/// [`MENU_STEPS`]. 100 % is "actual size".
pub fn step_action(percent: u16) -> Option<Box<dyn Action>> {
    Some(match percent {
        50 => Box::new(ZoomTo50),
        75 => Box::new(ZoomTo75),
        100 => Box::new(ActualSize),
        125 => Box::new(ZoomTo125),
        150 => Box::new(ZoomTo150),
        175 => Box::new(ZoomTo175),
        200 => Box::new(ZoomTo200),
        250 => Box::new(ZoomTo250),
        300 => Box::new(ZoomTo300),
        400 => Box::new(ZoomTo400),
        _ => return None,
    })
}

/// How a zoom is written in a menu and in the status bar: "125 %".
pub fn percent_label(percent: f32) -> String {
    format!("{} %", percent.round() as i32)
}

/// The keys of the zoom, anywhere in the window.
///
/// Zoom in is the Mac's `cmd-+`, and `cmd-=` beside it (the same key
/// without shift, as every Mac app has it). A real keyboard reports shift
/// with a punctuation key as the shifted character, a script spells the
/// shift out: both spellings are bound, the one a person reads last.
/// Actual size is `cmd-0` on a Mac, which is "Go to library" here and
/// stays that: it is `cmd-shift-0`, the same key with shift.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-shift-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("cmd-)", ActualSize, None),
        KeyBinding::new("cmd-shift-0", ActualSize, None),
    ]
}

/// The commands of the zoom, for the palette and the list of keyboard
/// shortcuts.
pub fn commands() -> Vec<CommandDescriptor> {
    let mut commands = vec![
        CommandDescriptor::new("Zoom in", CATEGORY_VIEW, ZoomIn).keywords([
            "larger", "bigger", "magnify", "enlarge", "closer", "view", "scale",
        ]),
        CommandDescriptor::new("Zoom out", CATEGORY_VIEW, ZoomOut)
            .keywords(["smaller", "reduce", "shrink", "further", "view", "scale"]),
        // Named like its neighbours, so that "zoom" finds it with them in
        // the palette and in the list of keyboard shortcuts (which searches
        // titles; 0.6.0 called it "Actual size" and the list found fourteen
        // zoom commands, not the one with a key). The menus say "Actual
        // Size", as a Mac does.
        CommandDescriptor::new("Zoom to actual size", CATEGORY_VIEW, ActualSize).keywords([
            "100",
            "reset zoom",
            "normal size",
            "original",
            "one to one",
        ]),
        CommandDescriptor::new("Zoom to fit width", CATEGORY_VIEW, FitWidth).keywords([
            "page width",
            "window",
            "fill",
            "whole width",
            "scale",
        ]),
        CommandDescriptor::new("Zoom to fit page", CATEGORY_VIEW, FitPage).keywords([
            "whole page",
            "one page",
            "window",
            "scale",
        ]),
        CommandDescriptor::new("Zoom to life size", CATEGORY_VIEW, LifeSize).keywords([
            "paper",
            "real size",
            "true size",
            "print size",
            "as printed",
            "sheet",
        ]),
    ];
    for percent in MENU_STEPS {
        if percent == 100 {
            // "Zoom to actual size" above.
            continue;
        }
        if let Some(action) = step_action(percent) {
            commands.push(
                CommandDescriptor::from_boxed(
                    format!("Zoom to {}", percent_label(f32::from(percent))),
                    CATEGORY_VIEW,
                    action,
                )
                .keywords(["scale", "percent", "magnify"]),
            );
        }
    }
    commands
}

/// The item of the View menu the zoom follows.
const AFTER_MENU_ITEM: &str = "Turn Vim Mode On or Off";

/// Puts the zoom into the View menu of the menu bar: in, out, actual size
/// and fit width as items, the rest under "Zoom To".
pub fn add_menu_items(menus: &mut [Menu]) {
    let Some(view) = menus.iter_mut().find(|menu| menu.name.as_ref() == "View") else {
        return;
    };
    let at = view
        .items
        .iter()
        .position(
            |item| matches!(item, MenuItem::Action { name, .. } if name.as_ref() == AFTER_MENU_ITEM),
        )
        .map_or(view.items.len(), |ix| ix + 1);
    let mut steps = vec![
        MenuItem::action("Fit Page", FitPage),
        MenuItem::action("Life Size", LifeSize),
        MenuItem::separator(),
    ];
    for percent in MENU_STEPS {
        if let Some(action) = step_action(percent) {
            steps.push(MenuItem::Action {
                name: percent_label(f32::from(percent)).into(),
                action,
                os_action: None,
            });
        }
    }
    let items = [
        MenuItem::separator(),
        MenuItem::action("Zoom In", ZoomIn),
        MenuItem::action("Zoom Out", ZoomOut),
        MenuItem::action("Actual Size", ActualSize),
        MenuItem::action("Fit Width", FitWidth),
        MenuItem::submenu(Menu {
            name: "Zoom To".into(),
            items: steps,
        }),
    ];
    for (offset, item) in items.into_iter().enumerate() {
        view.items.insert(at + offset, item);
    }
}

// ----- storage ---------------------------------------------------------------------

/// What the library keeps for a zoom: the percentage as a number in text,
/// or the name of a zoom that follows the window. `None` for a zoom that
/// is not a choice of the writer's ([`first_page_zoom`]): nothing is kept
/// for it.
pub fn stored_form(zoom: Zoom) -> Option<String> {
    match zoom {
        Zoom::Percent(percent) => Some(percent.to_string()),
        Zoom::FitWidth => Some("fit-width".to_string()),
        Zoom::FitPage => Some("fit-page".to_string()),
        Zoom::FitWidthUpTo(_) => None,
    }
}

/// The zoom a stored text stands for. `None` for a text this version
/// cannot read: the view then has the zoom it starts with.
pub fn from_stored(text: &str) -> Option<Zoom> {
    match text.trim() {
        "fit-width" => Some(Zoom::FitWidth),
        "fit-page" => Some(Zoom::FitPage),
        number => {
            let percent: f32 = number.parse().ok()?;
            (percent.is_finite()
                && percent >= f32::from(ZOOM_MIN)
                && percent <= f32::from(ZOOM_MAX))
            .then(|| Zoom::percent(percent))
        }
    }
}

/// What page view shows before the writer has chosen a zoom for it: the
/// sheet as large as the paper it stands for, and as wide as its room
/// where the window is narrower than that. On a display that does not say
/// how large it is the sheet is at 100 %, and likewise never wider than
/// its room (which is all a page did before it could be zoomed).
pub fn first_page_zoom(life_size: Option<u16>) -> Zoom {
    Zoom::FitWidthUpTo(life_size.unwrap_or(100))
}

// ----- the display ------------------------------------------------------------------

/// Where the app learns how dense a display is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum DisplaySource {
    /// Ask this Mac about the display a window is on.
    ThisMac,
    /// Every display has this many pixels to the inch (`None`: unknown).
    Fixed(Option<f32>),
}

impl Global for DisplaySource {}

/// Has the app ask this Mac how large its displays are. Called by `main`.
/// It only reads (`CGDisplayScreenSize`).
pub fn use_this_mac(cx: &mut App) {
    cx.set_global(DisplaySource::ThisMac);
}

/// Names the density of every display, in window pixels to the inch
/// (`None`: unknown). For tests: the test platform's display has no size.
pub fn set_display_density(pixels_per_inch: Option<f32>, cx: &mut App) {
    cx.set_global(DisplaySource::Fixed(pixels_per_inch));
}

/// How many pixels of a window (logical pixels: two device pixels on a
/// Retina display) the display `window` is on has to the inch. `None`
/// when nobody knows: no source was installed, or the display does not
/// report a size (a projector, some televisions).
pub fn display_density(window: &Window, cx: &App) -> Option<f32> {
    match cx.try_global::<DisplaySource>().copied()? {
        DisplaySource::Fixed(density) => density,
        DisplaySource::ThisMac => {
            let display = window.display(cx)?;
            let width = f32::from(display.bounds().size.width);
            let millimetres = native::display_width_mm(display.id().into())?;
            density_of(width, millimetres)
        }
    }
}

/// Pixels to the inch of a display `pixels` wide whose glass is
/// `millimetres` wide. `None` for measures that cannot be a display's.
pub fn density_of(pixels: f32, millimetres: f32) -> Option<f32> {
    if !(pixels.is_finite() && millimetres.is_finite()) || pixels <= 0. || millimetres <= 0. {
        return None;
    }
    let density = pixels / (millimetres / 25.4);
    // From a wall of a television seen as a display to the densest phone
    // screen: anything outside is a display that reports nonsense.
    (36. ..=600.).contains(&density).then_some(density)
}

/// The zoom at which a point of the document is a 72nd of an inch on the
/// glass of a display with `density` pixels to the inch: a sheet is then
/// as large as the paper it stands for. `None` where that is outside what
/// a view can be zoomed to.
pub fn life_size_of(density: f32) -> Option<u16> {
    let percent = (density / 72. * 100.).round();
    (percent >= f32::from(ZOOM_MIN) && percent <= f32::from(ZOOM_MAX)).then_some(percent as u16)
}

/// Life size on the display `window` is on, in percent ([`life_size_of`]).
pub fn life_size_percent(window: &Window, cx: &App) -> Option<u16> {
    display_density(window, cx).and_then(life_size_of)
}

#[cfg(target_os = "macos")]
mod native {
    #[repr(C)]
    struct Size {
        width: f64,
        height: f64,
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        /// The size of a display's glass in millimetres; zero when the
        /// display does not say.
        fn CGDisplayScreenSize(display: u32) -> Size;
    }

    /// The width of the glass of display `id` in millimetres.
    pub(super) fn display_width_mm(id: u32) -> Option<f32> {
        // SAFETY: a plain query by value; an id that names no display
        // answers with a size of zero.
        let size = unsafe { CGDisplayScreenSize(id) };
        (size.width > 0. && size.height > 0.).then_some(size.width as f32)
    }
}

#[cfg(not(target_os = "macos"))]
mod native {
    pub(super) fn display_width_mm(_: u32) -> Option<f32> {
        None
    }
}
