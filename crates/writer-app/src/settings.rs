//! User settings: the keys under which they are stored in the library and a
//! typed snapshot of their current values.
//!
//! Every setting is one key in the library's settings table (JSON values).
//! Writing a key through [`writer_library::Library::set_setting`] is all it
//! takes to change a setting from anywhere: the workspace listens for
//! [`writer_library::LibraryEvent::SettingChanged`], reloads [`Settings`],
//! applies the change (theme, editor configuration) and emits
//! [`crate::WorkspaceEvent::SettingsChanged`].

use gpui::App;
use writer_core::{DocumentSettings, PaperSize};
use writer_export::SocialNetwork;
use writer_library::Library;
use writer_ui::ThemeMode;

/// The setting keys. Values are JSON; the type is given per key.
pub mod keys {
    /// `"system"`, `"light"` or `"dark"`.
    pub const THEME: &str = "theme";
    /// `bool`: Vim mode.
    pub const VIM_MODE: &str = "editor.vim_mode";
    /// `bool`: Markdown shortcuts while typing (input rules).
    pub const INPUT_RULES: &str = "editor.input_rules";
    /// `bool`: smart quotes, dashes and ellipses.
    pub const SMART_PUNCTUATION: &str = "editor.smart_punctuation";
    /// `bool`: comment tints in the text and the comments panel.
    pub const COMMENTS_VISIBLE: &str = "editor.comments_visible";
    /// `string`: the author name written on new comments.
    pub const COMMENT_AUTHOR: &str = "comments.author";
    /// `string`: body font family of new documents.
    pub const NEW_DOCUMENT_FONT: &str = "new_document.font";
    /// `number`: base text size of new documents, in points.
    pub const NEW_DOCUMENT_SIZE: &str = "new_document.size";
    /// `number`: line height of new documents, as a multiple of the size.
    pub const NEW_DOCUMENT_LINE_HEIGHT: &str = "new_document.line_height";
    /// `bool`: whether new documents start in page view.
    pub const NEW_DOCUMENT_PAGE_VIEW: &str = "new_document.page_view";
    /// `"a4"` or `"letter"`: paper size of new documents.
    pub const NEW_DOCUMENT_PAPER: &str = "new_document.paper";
    /// `"off"` or a network: `"x"`, `"bluesky"`, `"mastodon"`, `"threads"`,
    /// `"instagram"`, `"linkedin"`. The character-limit indicator in the
    /// status bar.
    pub const CHARACTER_LIMIT: &str = "status.character_limit";
    /// `bool`: whether the library sidebar is shown.
    pub const SIDEBAR_VISIBLE: &str = "sidebar.visible";
    /// `number`: width of the library sidebar in pixels.
    pub const SIDEBAR_WIDTH: &str = "sidebar.width";
    /// Object `{x, y, width, height, mode}`: the main window's bounds.
    pub const WINDOW_BOUNDS: &str = "window.bounds";
    /// `number`: id of the document that was open last.
    pub const LAST_DOCUMENT: &str = "session.last_document";
    /// Array of action names: commands run from the palette, newest first.
    pub const RECENT_COMMANDS: &str = "palette.recent";
    /// `bool`: the welcome document has been created once.
    pub const WELCOME_CREATED: &str = "app.welcome_created";
    /// `string`: the directory of the last export.
    pub const EXPORT_DIRECTORY: &str = "export.directory";
    /// Prefix of the per-document flag "the user chose this title":
    /// `document.custom_title.<id>` holds `true`.
    pub const CUSTOM_TITLE_PREFIX: &str = "document.custom_title.";

    /// The key of the custom-title flag of document `id`.
    pub fn custom_title(id: writer_library::DocumentId) -> String {
        format!("{CUSTOM_TITLE_PREFIX}{}", id.0)
    }
}

/// Default width of the library sidebar.
pub const DEFAULT_SIDEBAR_WIDTH: f32 = 260.;
/// The narrowest the sidebar can be (the traffic lights sit above it).
pub const MIN_SIDEBAR_WIDTH: f32 = 200.;
/// The widest the sidebar can be.
pub const MAX_SIDEBAR_WIDTH: f32 = 480.;

/// Text sizes offered for new documents, in points.
pub const TEXT_SIZES: [f32; 11] = [13., 14., 15., 16., 17., 18., 19., 20., 22., 24., 28.];
/// Line heights offered for new documents.
pub const LINE_HEIGHTS: [f32; 8] = [1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 2.0];
/// Font families offered for new documents (filtered by what is installed).
pub const FONT_CANDIDATES: [&str; 12] = [
    "Charter",
    "Iowan Old Style",
    "Georgia",
    "Palatino",
    "Baskerville",
    "Hoefler Text",
    "Times New Roman",
    "New York",
    "Helvetica Neue",
    "Avenir Next",
    "Menlo",
    "Courier New",
];

/// Paper sizes offered for new documents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paper {
    /// ISO A4.
    #[default]
    A4,
    /// US Letter.
    Letter,
}

impl Paper {
    /// Every paper size, in menu order.
    pub const ALL: [Paper; 2] = [Paper::A4, Paper::Letter];

    /// The stored name.
    pub fn as_str(self) -> &'static str {
        match self {
            Paper::A4 => "a4",
            Paper::Letter => "letter",
        }
    }

    /// The name shown in the settings.
    pub fn label(self) -> &'static str {
        match self {
            Paper::A4 => "A4",
            Paper::Letter => "US Letter",
        }
    }

    /// Parses a stored name; unknown names give A4.
    pub fn parse(name: &str) -> Paper {
        match name {
            "letter" => Paper::Letter,
            _ => Paper::A4,
        }
    }

    /// The writer-core paper size.
    pub fn paper_size(self) -> PaperSize {
        match self {
            Paper::A4 => PaperSize::A4,
            Paper::Letter => PaperSize::UsLetter,
        }
    }
}

/// The stored name of a character-limit choice (`None` = `"off"`).
pub fn character_limit_name(limit: Option<SocialNetwork>) -> &'static str {
    match limit {
        None => "off",
        Some(SocialNetwork::X) => "x",
        Some(SocialNetwork::Bluesky) => "bluesky",
        Some(SocialNetwork::Mastodon) => "mastodon",
        Some(SocialNetwork::Threads) => "threads",
        Some(SocialNetwork::Instagram) => "instagram",
        Some(SocialNetwork::LinkedIn) => "linkedin",
    }
}

/// Parses a stored character-limit name; unknown names give `None`.
pub fn parse_character_limit(name: &str) -> Option<SocialNetwork> {
    SocialNetwork::ALL
        .into_iter()
        .find(|network| character_limit_name(Some(*network)) == name)
}

/// A typed snapshot of the user settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// Paper, Night or follow the system.
    pub theme: ThemeMode,
    /// Vim mode.
    pub vim_mode: bool,
    /// Markdown shortcuts while typing.
    pub input_rules: bool,
    /// Smart quotes, dashes and ellipses.
    pub smart_punctuation: bool,
    /// Comment tints in the text and the comments panel.
    pub comments_visible: bool,
    /// Author name written on new comments.
    pub comment_author: String,
    /// Body font of new documents.
    pub new_document_font: String,
    /// Base text size of new documents, in points.
    pub new_document_size: f32,
    /// Line height of new documents.
    pub new_document_line_height: f32,
    /// Whether new documents start in page view.
    pub new_document_page_view: bool,
    /// Paper size of new documents.
    pub new_document_paper: Paper,
    /// The network whose post limit the status bar counts against.
    pub character_limit: Option<SocialNetwork>,
    /// Whether the library sidebar is shown.
    pub sidebar_visible: bool,
    /// Width of the library sidebar in pixels.
    pub sidebar_width: f32,
}

impl Default for Settings {
    fn default() -> Self {
        let typography = writer_core::Typography::default();
        Self {
            theme: ThemeMode::System,
            vim_mode: false,
            input_rules: true,
            smart_punctuation: true,
            comments_visible: true,
            comment_author: default_author(),
            new_document_font: typography.body_font,
            new_document_size: typography.base_size,
            new_document_line_height: typography.line_height,
            new_document_page_view: false,
            new_document_paper: Paper::A4,
            character_limit: None,
            sidebar_visible: true,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
        }
    }
}

/// The login name, capitalised, as a first guess for the comment author.
fn default_author() -> String {
    let user = std::env::var("USER").unwrap_or_default();
    let mut chars = user.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Me".to_string(),
    }
}

impl Settings {
    /// Reads every setting from the library; missing or malformed values
    /// fall back to the defaults.
    pub fn load(library: &Library) -> Self {
        let defaults = Settings::default();
        let theme: String = library.setting_or(keys::THEME, defaults.theme.as_str().to_string());
        let paper: String = library.setting_or(
            keys::NEW_DOCUMENT_PAPER,
            defaults.new_document_paper.as_str().to_string(),
        );
        let limit: String = library.setting_or(keys::CHARACTER_LIMIT, "off".to_string());
        let author: String = library.setting_or(keys::COMMENT_AUTHOR, String::new());
        let font: String = library.setting_or(keys::NEW_DOCUMENT_FONT, String::new());
        Self {
            theme: ThemeMode::parse(&theme).unwrap_or(defaults.theme),
            vim_mode: library.setting_or(keys::VIM_MODE, defaults.vim_mode),
            input_rules: library.setting_or(keys::INPUT_RULES, defaults.input_rules),
            smart_punctuation: library
                .setting_or(keys::SMART_PUNCTUATION, defaults.smart_punctuation),
            comments_visible: library.setting_or(keys::COMMENTS_VISIBLE, defaults.comments_visible),
            comment_author: if author.trim().is_empty() {
                defaults.comment_author
            } else {
                author.trim().to_string()
            },
            new_document_font: if font.trim().is_empty() {
                defaults.new_document_font
            } else {
                font
            },
            new_document_size: library
                .setting_or(keys::NEW_DOCUMENT_SIZE, defaults.new_document_size)
                .clamp(writer_core::MIN_TEXT_SIZE, 72.),
            new_document_line_height: library
                .setting_or(
                    keys::NEW_DOCUMENT_LINE_HEIGHT,
                    defaults.new_document_line_height,
                )
                .clamp(1.0, 3.0),
            new_document_page_view: library.setting_or(
                keys::NEW_DOCUMENT_PAGE_VIEW,
                defaults.new_document_page_view,
            ),
            new_document_paper: Paper::parse(&paper),
            character_limit: parse_character_limit(&limit),
            sidebar_visible: library.setting_or(keys::SIDEBAR_VISIBLE, defaults.sidebar_visible),
            sidebar_width: library
                .setting_or(keys::SIDEBAR_WIDTH, defaults.sidebar_width)
                .clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH),
        }
    }

    /// The document settings a new document starts with.
    pub fn new_document_settings(&self) -> DocumentSettings {
        let mut settings = DocumentSettings::default();
        settings.typography.body_font = self.new_document_font.clone();
        settings.typography.base_size = self.new_document_size;
        settings.typography.line_height = self.new_document_line_height;
        settings.page.paper = self.new_document_paper.paper_size();
        settings.page_view = self.new_document_page_view;
        settings.sanitized()
    }
}

/// The font families offered for new documents: the installed ones among
/// [`FONT_CANDIDATES`], plus `current` when it is not in the list.
pub fn available_fonts(current: &str, cx: &App) -> Vec<String> {
    let installed = cx.text_system().all_font_names();
    let mut fonts: Vec<String> = FONT_CANDIDATES
        .iter()
        .filter(|candidate| installed.iter().any(|name| name == *candidate))
        .map(|candidate| candidate.to_string())
        .collect();
    if !current.is_empty() && !fonts.iter().any(|font| font == current) {
        fonts.insert(0, current.to_string());
    }
    if fonts.is_empty() {
        fonts.push(writer_core::Typography::default().body_font);
    }
    fonts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn character_limit_names_round_trip() {
        assert_eq!(parse_character_limit("off"), None);
        assert_eq!(parse_character_limit("nonsense"), None);
        for network in SocialNetwork::ALL {
            assert_eq!(
                parse_character_limit(character_limit_name(Some(network))),
                Some(network)
            );
        }
    }

    #[test]
    fn paper_names_round_trip() {
        for paper in Paper::ALL {
            assert_eq!(Paper::parse(paper.as_str()), paper);
        }
        assert_eq!(Paper::parse("b5"), Paper::A4);
    }

    #[test]
    fn new_document_settings_follow_the_defaults() {
        let settings = Settings {
            new_document_font: "Georgia".into(),
            new_document_size: 19.,
            new_document_line_height: 1.4,
            new_document_page_view: true,
            new_document_paper: Paper::Letter,
            ..Settings::default()
        };
        let document = settings.new_document_settings();
        assert_eq!(document.typography.body_font, "Georgia");
        assert_eq!(document.typography.base_size, 19.);
        assert_eq!(document.typography.line_height, 1.4);
        assert!(document.page_view);
        assert_eq!(document.page.paper, PaperSize::UsLetter);
    }
}
