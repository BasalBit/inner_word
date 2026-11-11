//! Editor configuration: mode, switches, placeholder and caret style.

use gpui::{Pixels, SharedString, px};

/// How the caret is drawn. Vim mode switches between these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CaretStyle {
    /// A thin vertical bar before the character (the default).
    #[default]
    Bar,
    /// A block covering the character under the caret (Vim normal mode).
    Block,
    /// A line under the character at the caret (Vim replace / operator-pending).
    Underline,
}

/// Options of [`EditorMode::Compact`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct CompactOptions {
    /// The editor grows with its content up to this height, then scrolls.
    pub max_height: Pixels,
    /// `enter` emits [`crate::EditorEvent::Submitted`] instead of starting a
    /// new paragraph; `shift-enter` still inserts a line break.
    pub submit_on_enter: bool,
}

impl Default for CompactOptions {
    fn default() -> Self {
        Self {
            max_height: px(160.),
            submit_on_enter: true,
        }
    }
}

impl CompactOptions {
    /// Sets the height at which the editor stops growing and scrolls.
    pub fn max_height(mut self, max_height: Pixels) -> Self {
        self.max_height = max_height;
        self
    }

    /// Sets whether `enter` submits.
    pub fn submit_on_enter(mut self, submit_on_enter: bool) -> Self {
        self.submit_on_enter = submit_on_enter;
        self
    }
}

/// The two shapes of an editor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum EditorMode {
    /// The document editor: document typography, the centred column (flow
    /// view) or page sheets (page view), fills its container and scrolls.
    #[default]
    Full,
    /// A small input such as a comment composer: interface font, no page
    /// view, no fixed column, as wide as its container and as tall as its
    /// content up to [`CompactOptions::max_height`].
    Compact(CompactOptions),
}

impl EditorMode {
    /// Whether this is [`EditorMode::Compact`].
    pub fn is_compact(&self) -> bool {
        matches!(self, EditorMode::Compact(_))
    }

    /// The compact options, in compact mode.
    pub fn compact_options(&self) -> Option<&CompactOptions> {
        match self {
            EditorMode::Compact(options) => Some(options),
            EditorMode::Full => None,
        }
    }
}

/// Configuration of an [`crate::Editor`]. Build one with
/// [`EditorConfig::full`] or [`EditorConfig::compact`] and the builder
/// methods; change it later with [`crate::Editor::set_config`] or
/// [`crate::Editor::update_config`].
///
/// ```ignore
/// let config = EditorConfig::full().placeholder("Start writing").read_only(false);
/// let composer = EditorConfig::compact(CompactOptions::default().max_height(px(120.)))
///     .placeholder("Add a comment");
/// ```
///
/// The struct is `#[non_exhaustive]`: fields can be read and assigned, but
/// other crates cannot build it with a struct literal, so fields can be
/// added later without breaking them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct EditorConfig {
    /// Full document editor or compact input.
    pub mode: EditorMode,
    /// Tint text that carries comment anchors (see
    /// [`crate::Editor::set_active_comment`]).
    pub comments_visible: bool,
    /// Markdown-style input rules while typing (`# `, `- `, `**bold**`, …).
    pub input_rules: bool,
    /// Smart punctuation while typing (curly quotes, dashes, ellipsis).
    pub smart_punctuation: bool,
    /// The user cannot change the document: typed text and the editing
    /// actions are ignored; moving, selecting and copying still work.
    /// [`crate::Editor::edit`] is not blocked (it is the programmatic seam).
    pub read_only: bool,
    /// Shown in a faint colour while the document is empty.
    pub placeholder: SharedString,
    /// How the caret is drawn.
    pub caret_style: CaretStyle,
}

impl Default for EditorConfig {
    fn default() -> Self {
        Self::full()
    }
}

impl EditorConfig {
    /// The document editor with everything on: comments visible, input
    /// rules, smart punctuation, bar caret, no placeholder.
    pub fn full() -> Self {
        Self {
            mode: EditorMode::Full,
            comments_visible: true,
            input_rules: true,
            smart_punctuation: true,
            read_only: false,
            placeholder: SharedString::default(),
            caret_style: CaretStyle::Bar,
        }
    }

    /// A compact input: comment tints off, Markdown input rules off, smart
    /// punctuation on.
    pub fn compact(options: CompactOptions) -> Self {
        Self {
            mode: EditorMode::Compact(options),
            comments_visible: false,
            input_rules: false,
            smart_punctuation: true,
            read_only: false,
            placeholder: SharedString::default(),
            caret_style: CaretStyle::Bar,
        }
    }

    /// Sets the mode.
    pub fn mode(mut self, mode: EditorMode) -> Self {
        self.mode = mode;
        self
    }

    /// Sets whether comment anchors are tinted.
    pub fn comments_visible(mut self, visible: bool) -> Self {
        self.comments_visible = visible;
        self
    }

    /// Sets whether Markdown input rules run.
    pub fn input_rules(mut self, on: bool) -> Self {
        self.input_rules = on;
        self
    }

    /// Sets whether smart punctuation runs.
    pub fn smart_punctuation(mut self, on: bool) -> Self {
        self.smart_punctuation = on;
        self
    }

    /// Sets read-only.
    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Sets the placeholder text.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Sets the caret style.
    pub fn caret_style(mut self, style: CaretStyle) -> Self {
        self.caret_style = style;
        self
    }
}
