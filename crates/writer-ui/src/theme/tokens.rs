//! Theme-independent design tokens: typography, spacing, radii.
//!
//! Colours live in [`super::ThemeColors`]; everything here is the same in
//! Paper and Night.

use gpui::{Pixels, px};

// ---------------------------------------------------------------------------
// Typography
// ---------------------------------------------------------------------------

/// GPUI's name for the macOS system UI font (SF Pro). Always resolvable.
pub const UI_FONT_FAMILY: &str = ".SystemUIFont";

/// Interface text sizes. The interface uses 13px as its base.
pub const UI_TEXT_XS: Pixels = px(11.);
pub const UI_TEXT_SM: Pixels = px(12.);
pub const UI_TEXT_BASE: Pixels = px(13.);
pub const UI_TEXT_LG: Pixels = px(15.);
/// Titles inside overlays and settings sections.
pub const UI_TEXT_TITLE: Pixels = px(20.);
/// Line height for single-line interface text (rows, buttons, fields).
pub const UI_LINE_HEIGHT: Pixels = px(18.);

/// Document serif stack, in order of preference. The first family installed
/// on the machine wins (see [`super::ResolvedFonts`]).
pub const DOCUMENT_FONT_STACK: &[&str] = &["Charter", "Iowan Old Style", "Georgia"];
/// Monospace stack for inline code and code blocks.
pub const MONO_FONT_STACK: &[&str] = &["SF Mono", "Menlo", "Monaco"];

/// Default document body size.
pub const DOCUMENT_TEXT_SIZE: Pixels = px(17.);
/// Default document line height as a multiple of the font size.
pub const DOCUMENT_LINE_HEIGHT_RATIO: f32 = 1.6;
/// Monospace text is set slightly smaller than the serif body so that the
/// x-heights look balanced next to each other.
pub const MONO_TEXT_SCALE: f32 = 0.88;
/// Width of the flow-view text column.
pub const DOCUMENT_COLUMN_WIDTH: Pixels = px(700.);

/// Heading sizes as multiples of the body size, heading 1 to heading 6.
pub const HEADING_SCALE: [f32; 6] = [2.0, 1.6, 1.3, 1.15, 1.0, 0.9];

/// Line height for a document block at the given font size.
pub fn document_line_height(font_size: Pixels) -> Pixels {
    font_size * DOCUMENT_LINE_HEIGHT_RATIO
}

// ---------------------------------------------------------------------------
// Spacing: a 4px grid
// ---------------------------------------------------------------------------

/// One spacing unit.
pub const SPACE_UNIT: f32 = 4.;
pub const SPACE_0_5: Pixels = px(2.);
pub const SPACE_1: Pixels = px(4.);
pub const SPACE_2: Pixels = px(8.);
pub const SPACE_3: Pixels = px(12.);
pub const SPACE_4: Pixels = px(16.);
pub const SPACE_5: Pixels = px(20.);
pub const SPACE_6: Pixels = px(24.);
pub const SPACE_8: Pixels = px(32.);
pub const SPACE_10: Pixels = px(40.);
pub const SPACE_12: Pixels = px(48.);
pub const SPACE_16: Pixels = px(64.);

/// `steps` units of the 4px grid: `space(3.) == px(12.)`.
pub fn space(steps: f32) -> Pixels {
    px(SPACE_UNIT * steps)
}

// ---------------------------------------------------------------------------
// Radii and borders
// ---------------------------------------------------------------------------

/// Inline chips, key hints, small buttons.
pub const RADIUS_SM: Pixels = px(4.);
/// Fields, list rows, buttons.
pub const RADIUS_MD: Pixels = px(6.);
/// Overlay panels (palette, pickers, menus), modals.
pub const RADIUS_LG: Pixels = px(8.);
/// Page sheets in page view: paper has almost square corners.
pub const RADIUS_PAGE: Pixels = px(2.);
/// Hairline border width.
pub const BORDER_WIDTH: Pixels = px(1.);

// ---------------------------------------------------------------------------
// Chrome metrics (macOS)
// ---------------------------------------------------------------------------

/// Height of the transparent titlebar area that the window content draws
/// under. Keep interactive content out of this band, except a drag area.
pub const TITLEBAR_HEIGHT: Pixels = px(38.);
/// Traffic light position used by the app window (see `writer-app`).
pub const TRAFFIC_LIGHT_X: Pixels = px(14.);
pub const TRAFFIC_LIGHT_Y: Pixels = px(13.);
/// Horizontal space reserved for the traffic lights at the left of the
/// titlebar band.
pub const TRAFFIC_LIGHT_RESERVED_WIDTH: Pixels = px(78.);
