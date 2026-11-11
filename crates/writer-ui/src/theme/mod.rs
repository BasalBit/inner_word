//! Theme tokens for The Writer: the Paper (light) and Night (dark) palettes,
//! typography, spacing, radii and shadows, installed as a GPUI global.
//!
//! ```ignore
//! use writer_ui::theme::{self, ActiveTheme, ThemeMode};
//!
//! // once, at startup
//! theme::init(ThemeMode::System, cx);
//! // per window, so `System` follows the macOS appearance
//! theme::observe_system_appearance(window, cx).detach();
//!
//! // anywhere with an `App` / `Context<T>`
//! let colors = &cx.theme().colors;
//! div().bg(colors.canvas).text_color(colors.ink);
//!
//! // switch mode (redraws all windows)
//! theme::set_theme_mode(ThemeMode::Dark, cx);
//! ```

mod palette;
pub mod tokens;

use gpui::{
    App, BoxShadow, Font, FontFallbacks, Global, Hsla, SharedString, Subscription, Window,
    WindowAppearance, font, point, px,
};

pub use palette::{night, paper};
pub use tokens::*;

/// Light or dark, after resolving [`ThemeMode::System`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Appearance {
    Light,
    Dark,
}

impl Appearance {
    pub fn is_dark(self) -> bool {
        self == Appearance::Dark
    }
}

impl From<WindowAppearance> for Appearance {
    fn from(appearance: WindowAppearance) -> Self {
        match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => Appearance::Light,
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Appearance::Dark,
        }
    }
}

/// The user's theme preference.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ThemeMode {
    /// Always Paper.
    Light,
    /// Always Night.
    Dark,
    /// Follow the macOS appearance.
    #[default]
    System,
}

impl ThemeMode {
    pub const ALL: [ThemeMode; 3] = [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System];

    /// Stable identifier for settings storage: `"light"`, `"dark"`, `"system"`.
    pub fn as_str(self) -> &'static str {
        match self {
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
            ThemeMode::System => "system",
        }
    }

    /// Parses [`Self::as_str`] (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "light" | "paper" => Some(ThemeMode::Light),
            "dark" | "night" => Some(ThemeMode::Dark),
            "system" | "auto" => Some(ThemeMode::System),
            _ => None,
        }
    }

    /// Label for settings and the command palette.
    pub fn label(self) -> &'static str {
        match self {
            ThemeMode::Light => "Paper (light)",
            ThemeMode::Dark => "Night (dark)",
            ThemeMode::System => "Match system",
        }
    }

    /// The appearance this mode produces given the current system appearance.
    pub fn resolve(self, system: WindowAppearance) -> Appearance {
        match self {
            ThemeMode::Light => Appearance::Light,
            ThemeMode::Dark => Appearance::Dark,
            ThemeMode::System => system.into(),
        }
    }
}

/// Background colours for highlighted text, one set per theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HighlightPalette {
    pub yellow: Hsla,
    pub green: Hsla,
    pub blue: Hsla,
    pub pink: Hsla,
    pub orange: Hsla,
    pub purple: Hsla,
}

/// The named highlight colours a document can use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum HighlightColor {
    #[default]
    Yellow,
    Green,
    Blue,
    Pink,
    Orange,
    Purple,
}

impl HighlightColor {
    pub const ALL: [HighlightColor; 6] = [
        HighlightColor::Yellow,
        HighlightColor::Green,
        HighlightColor::Blue,
        HighlightColor::Pink,
        HighlightColor::Orange,
        HighlightColor::Purple,
    ];

    /// Stable identifier for storage: `"yellow"`, `"green"`, ...
    pub fn as_str(self) -> &'static str {
        match self {
            HighlightColor::Yellow => "yellow",
            HighlightColor::Green => "green",
            HighlightColor::Blue => "blue",
            HighlightColor::Pink => "pink",
            HighlightColor::Orange => "orange",
            HighlightColor::Purple => "purple",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|color| color.as_str().eq_ignore_ascii_case(value.trim()))
    }

    /// Label for menus.
    pub fn label(self) -> &'static str {
        match self {
            HighlightColor::Yellow => "Yellow",
            HighlightColor::Green => "Green",
            HighlightColor::Blue => "Blue",
            HighlightColor::Pink => "Pink",
            HighlightColor::Orange => "Orange",
            HighlightColor::Purple => "Purple",
        }
    }
}

impl HighlightPalette {
    pub fn get(&self, color: HighlightColor) -> Hsla {
        match color {
            HighlightColor::Yellow => self.yellow,
            HighlightColor::Green => self.green,
            HighlightColor::Blue => self.blue,
            HighlightColor::Pink => self.pink,
            HighlightColor::Orange => self.orange,
            HighlightColor::Purple => self.purple,
        }
    }
}

/// Every colour token. All fields are opaque colours unless noted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThemeColors {
    pub appearance: Appearance,

    // Surfaces
    /// Main background of the window and of the flow-view writing surface.
    pub canvas: Hsla,
    /// Library sidebar background.
    pub sidebar: Hsla,
    /// Floating panels: palette, pickers, menus, modals, tooltips.
    pub elevated: Hsla,
    /// Background behind page sheets in page view.
    pub desk: Hsla,
    /// Page sheets in page view.
    pub page: Hsla,
    /// Hovered list rows and buttons (on canvas, sidebar or elevated).
    pub hover: Hsla,

    // Text
    /// Body text and primary labels.
    pub ink: Hsla,
    /// Secondary labels, metadata, key hints.
    pub muted: Hsla,
    /// Placeholders, disabled text, counts, page numbers: quieter than
    /// `muted`, still readable (about 4:1).
    pub faint: Hsla,
    /// Text drawn on an `accent` background.
    pub on_accent: Hsla,

    // Borders
    /// Default 1px separators and panel outlines.
    pub hairline: Hsla,
    /// Field outlines and stronger separators.
    pub border_strong: Hsla,

    // Accent: red-pencil terracotta
    /// Focus rings, active items, primary buttons.
    pub accent: Hsla,
    pub accent_hover: Hsla,
    /// Background of the selected row in lists and overlays.
    pub accent_subtle: Hsla,

    // Editing
    /// Text selection background (translucent) while the editor is focused.
    pub selection: Hsla,
    /// Text selection background (translucent) while the editor is not focused.
    pub selection_inactive: Hsla,
    pub caret: Hsla,
    /// Text highlight backgrounds.
    pub highlight: HighlightPalette,
    /// Background of text anchored to a comment thread.
    pub comment: Hsla,
    /// Background of text anchored to the active (selected) comment thread.
    pub comment_active: Hsla,
    /// Underline colour for commented text.
    pub comment_underline: Hsla,
    /// Find match background (translucent): the hue of the current match,
    /// paler, so it is not taken for a highlight of the document.
    pub find_match: Hsla,
    /// Current find match background (translucent).
    pub find_match_current: Hsla,

    // Status
    pub danger: Hsla,
    pub danger_subtle: Hsla,
    pub success: Hsla,
    pub success_subtle: Hsla,

    // Overlays
    /// Translucent scrim drawn behind modal overlays.
    pub overlay_backdrop: Hsla,
    /// Base shadow colour; the presets below apply their own alpha.
    pub shadow: Hsla,
}

/// Font families resolved against the fonts installed on this machine.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedFonts {
    /// Interface font family (the system UI font).
    pub ui: SharedString,
    /// First installed family of [`DOCUMENT_FONT_STACK`].
    pub document: SharedString,
    /// First installed family of [`MONO_FONT_STACK`].
    pub mono: SharedString,
}

impl ResolvedFonts {
    /// Picks the first installed family of each stack. `installed` is the list
    /// from `cx.text_system().all_font_names()`.
    pub fn resolve(installed: &[String]) -> Self {
        fn pick(stack: &[&str], installed: &[String]) -> SharedString {
            stack
                .iter()
                .find(|family| installed.iter().any(|name| name == *family))
                .or_else(|| stack.first())
                .map(|family| SharedString::from(family.to_string()))
                .unwrap_or_default()
        }
        Self {
            ui: UI_FONT_FAMILY.into(),
            document: pick(DOCUMENT_FONT_STACK, installed),
            mono: pick(MONO_FONT_STACK, installed),
        }
    }

    /// Stack defaults without checking what is installed.
    pub fn unresolved() -> Self {
        Self {
            ui: UI_FONT_FAMILY.into(),
            document: DOCUMENT_FONT_STACK[0].into(),
            mono: MONO_FONT_STACK[0].into(),
        }
    }
}

/// The active theme.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    /// "Paper" or "Night".
    pub name: SharedString,
    pub colors: ThemeColors,
    pub fonts: ResolvedFonts,
}

impl Theme {
    pub fn paper(fonts: ResolvedFonts) -> Self {
        Self {
            name: "Paper".into(),
            colors: paper(),
            fonts,
        }
    }

    pub fn night(fonts: ResolvedFonts) -> Self {
        Self {
            name: "Night".into(),
            colors: night(),
            fonts,
        }
    }

    pub fn for_appearance(appearance: Appearance, fonts: ResolvedFonts) -> Self {
        match appearance {
            Appearance::Light => Self::paper(fonts),
            Appearance::Dark => Self::night(fonts),
        }
    }

    pub fn appearance(&self) -> Appearance {
        self.colors.appearance
    }

    pub fn is_dark(&self) -> bool {
        self.colors.appearance.is_dark()
    }

    pub fn highlight(&self, color: HighlightColor) -> Hsla {
        self.colors.highlight.get(color)
    }

    /// The interface font.
    pub fn ui_font(&self) -> Font {
        font(self.fonts.ui.clone())
    }

    /// The default document serif, with the rest of the stack as glyph
    /// fallbacks.
    pub fn document_font(&self) -> Font {
        with_fallbacks(font(self.fonts.document.clone()), DOCUMENT_FONT_STACK)
    }

    /// The monospace font for code, with the rest of the stack as glyph
    /// fallbacks.
    pub fn mono_font(&self) -> Font {
        with_fallbacks(font(self.fonts.mono.clone()), MONO_FONT_STACK)
    }

    /// Floating overlays: palette, pickers, format menu, modals.
    pub fn shadow_overlay(&self) -> Vec<BoxShadow> {
        let (near, far) = if self.is_dark() {
            (0.40, 0.45)
        } else {
            (0.08, 0.14)
        };
        vec![
            shadow(self.colors.shadow, near, 0., 1., 3., 0.),
            shadow(self.colors.shadow, far, 0., 8., 24., -4.),
        ]
    }

    /// Small popovers that sit close to their origin: tooltips, inline menus.
    pub fn shadow_popover(&self) -> Vec<BoxShadow> {
        let (near, far) = if self.is_dark() {
            (0.35, 0.35)
        } else {
            (0.08, 0.10)
        };
        vec![
            shadow(self.colors.shadow, near, 0., 1., 2., 0.),
            shadow(self.colors.shadow, far, 0., 4., 12., -2.),
        ]
    }

    /// Page sheets on the desk in page view.
    pub fn shadow_page(&self) -> Vec<BoxShadow> {
        let (near, far) = if self.is_dark() {
            (0.45, 0.35)
        } else {
            (0.06, 0.08)
        };
        vec![
            shadow(self.colors.shadow, near, 0., 1., 2., 0.),
            shadow(self.colors.shadow, far, 0., 6., 20., 0.),
        ]
    }
}

/// Focus ring for keyboard-focused controls (fields, buttons, switches): a
/// 3px translucent accent halo drawn outside the control's bounds. Use it on
/// controls with an opaque background; the halo is a filled shadow and would
/// tint a transparent control's interior.
pub fn focus_ring(colors: &ThemeColors) -> Vec<BoxShadow> {
    let alpha = if colors.appearance.is_dark() {
        0.45
    } else {
        0.30
    };
    vec![BoxShadow {
        color: colors.accent.alpha(alpha),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(3.),
    }]
}

fn with_fallbacks(mut font: Font, stack: &[&str]) -> Font {
    let fallbacks: Vec<String> = stack
        .iter()
        .filter(|family| **family != font.family.as_ref())
        .map(|family| family.to_string())
        .collect();
    if !fallbacks.is_empty() {
        font.fallbacks = Some(FontFallbacks::from_fonts(fallbacks));
    }
    font
}

fn shadow(color: Hsla, alpha: f32, x: f32, y: f32, blur: f32, spread: f32) -> BoxShadow {
    BoxShadow {
        color: color.alpha(alpha),
        offset: point(px(x), px(y)),
        blur_radius: px(blur),
        spread_radius: px(spread),
    }
}

/// The GPUI global holding the theme mode and the active theme. Observe it
/// with `cx.observe_global::<GlobalTheme>(..)` to react to theme changes.
pub struct GlobalTheme {
    mode: ThemeMode,
    theme: Theme,
}

impl Global for GlobalTheme {}

impl GlobalTheme {
    pub fn mode(&self) -> ThemeMode {
        self.mode
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }
}

/// Access to the active theme: `cx.theme()` on `App`, `Context<T>` (derefs
/// to `App`), and the test contexts' `cx.update(|cx| cx.theme())`.
pub trait ActiveTheme {
    fn theme(&self) -> &Theme;
}

impl ActiveTheme for App {
    /// Panics if [`init`] was not called.
    fn theme(&self) -> &Theme {
        &self.global::<GlobalTheme>().theme
    }
}

/// Installs the theme global. Resolves the font stacks against the installed
/// fonts, then applies `mode` using the current system appearance.
pub fn init(mode: ThemeMode, cx: &mut App) {
    let installed = cx.text_system().all_font_names();
    let fonts = ResolvedFonts::resolve(&installed);
    let appearance = mode.resolve(cx.window_appearance());
    cx.set_global(GlobalTheme {
        mode,
        theme: Theme::for_appearance(appearance, fonts),
    });
}

/// The current theme mode. Panics if [`init`] was not called.
pub fn theme_mode(cx: &App) -> ThemeMode {
    cx.global::<GlobalTheme>().mode
}

/// Changes the theme mode and redraws every window. `System` resolves against
/// `cx.window_appearance()`.
pub fn set_theme_mode(mode: ThemeMode, cx: &mut App) {
    let appearance = mode.resolve(cx.window_appearance());
    apply(mode, appearance, cx);
}

/// Re-resolves `System` mode against a new system appearance. Does nothing
/// in `Light` or `Dark` mode, or before [`init`].
pub fn sync_system_appearance(system: WindowAppearance, cx: &mut App) {
    let Some(mode) = cx.try_global::<GlobalTheme>().map(|global| global.mode) else {
        return;
    };
    if mode == ThemeMode::System {
        apply(mode, system.into(), cx);
    }
}

/// Keeps `System` mode in sync with the window's appearance (which follows the
/// macOS setting). Call once per window and keep or detach the subscription.
pub fn observe_system_appearance(window: &Window, _cx: &mut App) -> Subscription {
    window.observe_window_appearance(|window, cx| {
        sync_system_appearance(window.appearance(), cx);
    })
}

fn apply(mode: ThemeMode, appearance: Appearance, cx: &mut App) {
    let global = cx.global::<GlobalTheme>();
    if global.mode == mode && global.theme.appearance() == appearance {
        return;
    }
    let fonts = global.theme.fonts.clone();
    cx.set_global(GlobalTheme {
        mode,
        theme: Theme::for_appearance(appearance, fonts),
    });
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn linear(channel: f32) -> f32 {
        if channel <= 0.04045 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }

    fn luminance(color: Hsla) -> f32 {
        let rgb = color.to_rgb();
        0.2126 * linear(rgb.r) + 0.7152 * linear(rgb.g) + 0.0722 * linear(rgb.b)
    }

    /// WCAG 2 contrast ratio between two opaque colours.
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (la, lb) = (luminance(a), luminance(b));
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn check(theme: &ThemeColors, name: &str) {
        let at_least = |what: &str, fg: Hsla, bg: Hsla, min: f32| {
            let ratio = contrast(fg, bg);
            assert!(
                ratio >= min,
                "{name}: {what} contrast {ratio:.2} is below {min}"
            );
        };
        for (surface_name, surface) in [
            ("canvas", theme.canvas),
            ("page", theme.page),
            ("sidebar", theme.sidebar),
            ("elevated", theme.elevated),
        ] {
            at_least(&format!("ink on {surface_name}"), theme.ink, surface, 12.0);
            at_least(
                &format!("muted on {surface_name}"),
                theme.muted,
                surface,
                4.5,
            );
        }
        // A selected row has to be found at a glance (it was 1.09:1 on the
        // Night panels); its secondary text gives up a little for that.
        at_least(
            "muted on accent_subtle",
            theme.muted,
            theme.accent_subtle,
            4.0,
        );
        at_least("ink on accent_subtle", theme.ink, theme.accent_subtle, 8.0);
        at_least(
            "accent_subtle on elevated",
            theme.accent_subtle,
            theme.elevated,
            1.3,
        );
        at_least(
            "danger_subtle on elevated",
            theme.danger_subtle,
            theme.elevated,
            1.3,
        );
        at_least(
            "danger on danger_subtle",
            theme.danger,
            theme.danger_subtle,
            4.0,
        );
        at_least("ink on hover", theme.ink, theme.hover, 10.0);
        // Faint text carries information (counts, placeholders, page
        // numbers): readable, and still quieter than muted.
        for (surface_name, surface) in [
            ("canvas", theme.canvas),
            ("page", theme.page),
            ("sidebar", theme.sidebar),
            ("elevated", theme.elevated),
        ] {
            at_least(
                &format!("faint on {surface_name}"),
                theme.faint,
                surface,
                3.9,
            );
            assert!(
                contrast(theme.faint, surface) + 0.8 < contrast(theme.muted, surface),
                "{name}: faint is not quieter than muted on {surface_name}"
            );
        }
        // A find match is not the yellow of the default highlight.
        let hue_gap = (theme.find_match.h - theme.highlight.yellow.h).abs() * 360.;
        assert!(
            hue_gap > 15.,
            "{name}: find match hue is {hue_gap:.0} degrees from yellow"
        );
        at_least("on_accent on accent", theme.on_accent, theme.accent, 4.5);
        at_least("accent on canvas", theme.accent, theme.canvas, 3.0);
        at_least("caret on page", theme.caret, theme.page, 3.0);
        at_least("danger on canvas", theme.danger, theme.canvas, 4.5);
        at_least("success on canvas", theme.success, theme.canvas, 4.5);
        at_least("ink on comment", theme.ink, theme.comment, 10.0);
        at_least(
            "ink on comment_active",
            theme.ink,
            theme.comment_active,
            7.0,
        );
        for color in HighlightColor::ALL {
            at_least(
                &format!("ink on {} highlight", color.as_str()),
                theme.ink,
                theme.highlight.get(color),
                7.0,
            );
        }
        // Surfaces must be distinguishable from each other, but quietly.
        let distinct = |what: &str, a: Hsla, b: Hsla| {
            let ratio = contrast(a, b);
            assert!(
                (1.03..1.6).contains(&ratio),
                "{name}: {what} separation {ratio:.3} is not subtle-but-visible"
            );
        };
        distinct("desk vs page", theme.desk, theme.page);
        distinct("sidebar vs canvas", theme.sidebar, theme.canvas);
        distinct("hairline vs canvas", theme.hairline, theme.canvas);
    }

    #[test]
    fn paper_contrast() {
        let theme = paper();
        assert_eq!(theme.appearance, Appearance::Light);
        check(&theme, "Paper");
    }

    #[test]
    fn night_contrast() {
        let theme = night();
        assert_eq!(theme.appearance, Appearance::Dark);
        check(&theme, "Night");
    }

    #[test]
    fn mode_round_trip() {
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(
            ThemeMode::System.resolve(WindowAppearance::VibrantDark),
            Appearance::Dark
        );
        assert_eq!(
            ThemeMode::Light.resolve(WindowAppearance::Dark),
            Appearance::Light
        );
        for color in HighlightColor::ALL {
            assert_eq!(HighlightColor::parse(color.as_str()), Some(color));
        }
    }

    #[test]
    fn font_stacks_resolve_to_first_installed() {
        let installed = vec!["Georgia".to_string(), "Menlo".to_string()];
        let fonts = ResolvedFonts::resolve(&installed);
        assert_eq!(fonts.document.as_ref(), "Georgia");
        assert_eq!(fonts.mono.as_ref(), "Menlo");
        let fonts = ResolvedFonts::resolve(&[]);
        assert_eq!(fonts.document.as_ref(), "Charter");
        let theme = Theme::paper(ResolvedFonts::resolve(&installed));
        let document = theme.document_font();
        assert_eq!(document.family.as_ref(), "Georgia");
        assert_eq!(
            document.fallbacks.unwrap().fallback_list(),
            &["Charter".to_string(), "Iowan Old Style".to_string()]
        );
    }
}
