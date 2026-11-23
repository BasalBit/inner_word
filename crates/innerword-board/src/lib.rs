//! Whiteboards for Inner Word: the scene in Rust.
//!
//! A whiteboard is a library item of kind `board` whose content is an Excalidraw scene
//! (`docs/whiteboard-design.md`, section 2). This crate is the one place that reads and
//! writes that scene:
//!
//! - [`Scene`]: the stored form ([`Scene::from_stored`], [`Scene::to_stored`]) and the form
//!   the view posts ([`Scene::from_view`]); every element is kept with every field, unknown
//!   ones included, in z-order; deleted elements and the kinds never kept (`iframe`,
//!   `embeddable`, `magicframe`) are left out.
//! - What is read of a scene: its text in reading order ([`Scene::text`], what the library
//!   indexes), its summary, its listing for agents and the assistant ([`Scene::listing`],
//!   [`Scene::listed`]), its bounds, what its PDF shows ([`Scene::pdf_bounds`]: a frame clips its
//!   children) and the size of its PDF ([`Scene::pdf_size`]).
//! - What agents write: skeletons made into Excalidraw 0.18.1's elements ([`add`]), changes
//!   by id ([`update`]) and removals ([`remove`]), in pure Rust, with every value derived
//!   from the input ([`HashedMaking`]), never random.
//! - The delta between two stored scenes ([`Scene::delta_from`], what the view's `adopt`
//!   takes) and the merge by the base rule ([`merge`], section 4.5).
//! - What the assistant proposes: a drawing ([`Drawing`]) read from an answer and checked
//!   before any whiteboard is looked at, applied whole or not at all ([`draw`]), a drawing
//!   that stands on its own put where it is free and seen ([`place`]); and the rule both
//!   agents and the assistant follow, that a change which sets what an element already has
//!   changes nothing ([`changes_that_change`]).
//! - Excalidraw's shape library as the library keeps it ([`ShapeLibrary`]).
//! - [`sniff_svg`] (which bytes are kept as an SVG), [`well_formed_json`] (lone surrogates
//!   mended), [`export_dir`] (what the tool hands its PDF child).
//!
//! Text sizes are measured with the advance widths of the bundled fonts ([`metrics`]):
//! close to Excalidraw's own, not exact. Excalidraw measures every text again whenever it
//! loads a scene, so what the writer sees and every PDF are exact.

pub mod export_dir;
pub mod limits;
pub mod metrics;

mod delta;
mod drawing;
mod file_hex;
mod fractional;
mod frames;
mod geometry;
mod json;
mod library;
mod listing;
mod making;
mod scene;
mod skeleton;
mod svg;
mod unchanged;

pub use delta::{Delta, Merged, merge};
pub use drawing::{
    DRAWABLE_TYPES, DrawError, Drawing, DrawingError, Drawn, MAX_TITLE_CHARS, PLACE_GAP,
    PLACE_MARGIN, Placement, draw, place,
};
pub use file_hex::FileHex;
pub use json::well_formed_json;
pub use library::{LibraryItem, ParsedLibrary, ShapeLibrary};
pub use listing::{BoardSummary, Bounds, ListedElement, ListedStyle, ListingHeader};
pub use making::{HashedMaking, Making};
pub use scene::{Element, ElementType, FileEntry, Parsed, Scene, SceneError};
pub use skeleton::{
    Added, AddedElement, BoardError, ElementChange, Picture, Pictures, Removed, Skeleton, Updated,
    add, is_id, link_host, picture_paths, remove, update,
};
pub use svg::sniff_svg;
pub use unchanged::changes_that_change;

/// The version of `@excalidraw/excalidraw` whose elements this crate writes and whose
/// rules it follows; equal to `innerword_board_web::EXCALIDRAW_VERSION` (a test of that
/// crate) and to the package the bundle pins.
pub const EXCALIDRAW_VERSION: &str = "0.18.1";

/// The `source` of every scene and shape library this crate writes (Excalidraw writes the
/// page's origin there).
pub const SOURCE: &str = "inner-word";
