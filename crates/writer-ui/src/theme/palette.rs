//! The two palettes: Paper (light) and Night (dark).
//!
//! Paper and ink. Warm off-white surfaces with near-black warm ink in the
//! light theme; warm charcoal with off-white ink in the dark theme. One
//! accent, a red-pencil terracotta, used sparingly for focus, the caret and
//! active items. Contrast targets (checked by the tests in `theme::tests`):
//! ink on canvas and page above 12:1, muted text above 4.5:1, ink on every
//! highlight colour above 7:1, the accent above 3:1 against the canvas.

use gpui::{Hsla, hsla, rgb};

use super::{Appearance, HighlightPalette, ThemeColors};

fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Paper: the light theme.
pub fn paper() -> ThemeColors {
    ThemeColors {
        appearance: Appearance::Light,

        canvas: c(0xFAF8F4),
        sidebar: c(0xF3F0EA),
        elevated: c(0xFFFEFC),
        desk: c(0xECE8E1),
        page: c(0xFFFFFF),
        hover: c(0xEEEAE3),

        ink: c(0x1F1C18),
        muted: c(0x6B645B),
        faint: c(0xA0988D),
        on_accent: c(0xFFFFFF),

        hairline: c(0xE4DFD6),
        border_strong: c(0xCDC5B8),

        accent: c(0xB0503A),
        accent_hover: c(0x96412E),
        accent_subtle: c(0xF4E5DF),

        selection: hsla(214. / 360., 0.55, 0.52, 0.22),
        selection_inactive: hsla(30. / 360., 0.10, 0.45, 0.14),
        caret: c(0xB0503A),

        highlight: HighlightPalette {
            yellow: c(0xF6E6A0),
            green: c(0xD4EBC6),
            blue: c(0xD0E2F4),
            pink: c(0xF5D5E1),
            orange: c(0xF7DBBE),
            purple: c(0xE2D8F1),
        },

        comment: c(0xF5E3DB),
        comment_active: c(0xEBC5B6),
        comment_underline: c(0xC9795F),

        find_match: hsla(45. / 360., 0.90, 0.55, 0.32),
        find_match_current: hsla(28. / 360., 0.95, 0.55, 0.58),

        danger: c(0xB3261E),
        danger_subtle: c(0xF7E0DD),
        success: c(0x2E7D4F),
        success_subtle: c(0xDDEFE3),

        overlay_backdrop: hsla(30. / 360., 0.20, 0.10, 0.16),
        shadow: hsla(30. / 360., 0.25, 0.12, 1.0),
    }
}

/// Night: the dark theme.
pub fn night() -> ThemeColors {
    ThemeColors {
        appearance: Appearance::Dark,

        canvas: c(0x1C1A17),
        sidebar: c(0x181614),
        elevated: c(0x272420),
        desk: c(0x131110),
        page: c(0x211F1B),
        hover: c(0x2B2824),

        ink: c(0xEBE5DB),
        muted: c(0xA49B8E),
        faint: c(0x6F685F),
        on_accent: c(0x1A1512),

        hairline: c(0x2F2B26),
        border_strong: c(0x48423B),

        accent: c(0xDB7D62),
        accent_hover: c(0xE8937A),
        accent_subtle: c(0x3A2620),

        selection: hsla(214. / 360., 0.50, 0.60, 0.30),
        selection_inactive: hsla(30. / 360., 0.08, 0.60, 0.16),
        caret: c(0xE08368),

        highlight: HighlightPalette {
            yellow: c(0x564A1D),
            green: c(0x2D4729),
            blue: c(0x243C58),
            pink: c(0x562C3E),
            orange: c(0x5A381C),
            purple: c(0x3D3056),
        },

        comment: c(0x3F2820),
        comment_active: c(0x66392A),
        comment_underline: c(0xC46F55),

        find_match: hsla(45. / 360., 0.80, 0.50, 0.30),
        find_match_current: hsla(28. / 360., 0.90, 0.55, 0.52),

        danger: c(0xF2786D),
        danger_subtle: c(0x41211E),
        success: c(0x6CC08B),
        success_subtle: c(0x1F3527),

        overlay_backdrop: hsla(0., 0., 0., 0.45),
        shadow: hsla(0., 0., 0., 1.0),
    }
}
