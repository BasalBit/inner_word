//! The limits of a whiteboard (`docs/whiteboard-design.md`, section 2, "Limits").
//!
//! Two kinds, because a writer's drawing must never become unsaveable: the limits for
//! agents refuse a request (exit 2, nothing written); the writer's own scene has only a
//! hard cap far above any drawing a person makes, over which the app writes a recovery
//! file instead of saving.

// For agents (a request over them is refused).

/// The stored scene after an agent's change, in bytes (about 25,000 elements at the 620
/// bytes Excalidraw spends on one).
pub const MAX_SCENE_BYTES: usize = 16 * 1024 * 1024;
/// Elements on a whiteboard after an agent's change.
pub const MAX_ELEMENTS: usize = 20_000;
/// Elements (skeletons, changes or ids) in one request of an agent.
pub const MAX_REQUEST_ELEMENTS: usize = 2_000;
/// Characters in one text an agent writes (a text element, a label, a frame's name).
pub const MAX_TEXT_CHARS: usize = 20_000;
/// Characters in an id an agent gives (`[A-Za-z0-9_-]`).
pub const MAX_ID_CHARS: usize = 64;

// For the writer's own scene: a note above the first, a recovery file above the second.

/// Over this the save state says once that saving this whiteboard takes longer.
pub const LARGE_VIEW_SCENE_BYTES: usize = 16 * 1024 * 1024;
/// The hard cap of the writer's stored scene: over it the scene goes to a recovery file.
pub const MAX_VIEW_SCENE_BYTES: usize = 256 * 1024 * 1024;

// The PDF of a whiteboard (section 5).

/// The margin around the drawing, in points (CSS pixels, 1:1).
pub const PDF_PADDING: f64 = 32.0;
/// The longest side of the PDF's page (Acrobat's limit, 200 inches): a larger drawing is
/// scaled down as a whole.
pub const PDF_MAX_SIDE: f64 = 14_400.0;

// An agent's pictures (section 7.2): an SVG kept as it came, and the drawn size by default.

/// The most an SVG kept as it came may weigh (Excalidraw allows 4 MB).
pub const MAX_SVG_BYTES: usize = 4 * 1024 * 1024;
/// Neither side of a picture drawn at its own size passes this (the size Excalidraw gives
/// a pasted picture).
pub const PICTURE_MAX_SIDE: f64 = 1_440.0;

// The shape library (section 4.15).

/// The stored shape library, in bytes.
pub const MAX_LIBRARY_BYTES: usize = 8 * 1024 * 1024;
/// Items in the shape library.
pub const MAX_LIBRARY_ITEMS: usize = 1_000;
