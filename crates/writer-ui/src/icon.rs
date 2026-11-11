//! [`Icon`]: the embedded 16px line-icon set.
//!
//! All icons are drawn on a 16×16 grid with 1.5px round strokes and are
//! tinted with a single colour (GPUI renders SVGs as alpha masks). The app
//! must install [`crate::assets::Assets`] (or chain [`crate::assets::load`])
//! for them to appear.
//!
//! ```ignore
//! use writer_ui::{Icon, IconName};
//!
//! Icon::new(IconName::Folder)                       // 16px, muted colour
//! Icon::new(IconName::Trash).size(px(14.)).color(cx.theme().colors.danger)
//! ```

use gpui::{App, Hsla, Pixels, Window, prelude::*, px, svg};

use crate::theme::ActiveTheme;

macro_rules! icons {
    ($($variant:ident => $file:literal,)*) => {
        /// Names of the embedded icons.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($variant,)*
        }

        impl IconName {
            /// Every icon, in declaration order.
            pub const ALL: &'static [IconName] = &[$(IconName::$variant,)*];

            /// Asset path, e.g. `"icons/folder.svg"`.
            pub fn path(self) -> &'static str {
                match self {
                    $(IconName::$variant => concat!("icons/", $file, ".svg"),)*
                }
            }

            /// Stable snake_case name, e.g. `"folder_open"`.
            pub fn name(self) -> &'static str {
                match self {
                    $(IconName::$variant => $file,)*
                }
            }

            pub(crate) fn svg_bytes(self) -> &'static [u8] {
                match self {
                    $(IconName::$variant => include_bytes!(concat!("../assets/icons/", $file, ".svg")),)*
                }
            }
        }
    };
}

icons! {
    Folder => "folder",
    FolderOpen => "folder_open",
    Document => "document",
    ChevronRight => "chevron_right",
    ChevronDown => "chevron_down",
    Search => "search",
    Comment => "comment",
    Check => "check",
    Close => "close",
    Plus => "plus",
    More => "more",
    Trash => "trash",
    Export => "export",
    Settings => "settings",
    Page => "page",
    Keyboard => "keyboard",
    Alert => "alert",
    Link => "link",
}

/// The default icon size.
pub const ICON_SIZE: Pixels = px(16.);

/// An icon element. Default size 16px, default colour the theme's `muted`.
#[derive(IntoElement, Clone, Debug)]
pub struct Icon {
    name: IconName,
    size: Pixels,
    color: Option<Hsla>,
}

impl Icon {
    pub fn new(name: IconName) -> Self {
        Self {
            name,
            size: ICON_SIZE,
            color: None,
        }
    }

    pub fn size(mut self, size: Pixels) -> Self {
        self.size = size;
        self
    }

    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    pub fn name(&self) -> IconName {
        self.name
    }
}

impl RenderOnce for Icon {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let color = self.color.unwrap_or(cx.theme().colors.muted);
        svg()
            .path(self.name.path())
            .flex_none()
            .size(self.size)
            .text_color(color)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_16px_svg() {
        for icon in IconName::ALL {
            let text = std::str::from_utf8(icon.svg_bytes()).unwrap();
            assert!(text.starts_with("<svg"), "{}", icon.name());
            assert!(text.contains("viewBox=\"0 0 16 16\""), "{}", icon.name());
            assert_eq!(
                crate::assets::load(icon.path()).unwrap().as_ref(),
                icon.svg_bytes()
            );
        }
        assert_eq!(crate::assets::list("icons/").len(), IconName::ALL.len());
        assert!(crate::assets::load("icons/missing.svg").is_none());
    }
}
