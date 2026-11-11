//! The GPUI [`AssetSource`] for this crate's embedded files (the icon set).
//!
//! ```ignore
//! // the app has no other assets:
//! Application::new().with_assets(writer_ui::assets::Assets).run(..);
//!
//! // the app has its own assets: chain them
//! struct AppAssets;
//! impl AssetSource for AppAssets {
//!     fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
//!         if let Some(bytes) = writer_ui::assets::load(path) {
//!             return Ok(Some(bytes));
//!         }
//!         /* own files */
//!     }
//!     fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
//!         Ok(writer_ui::assets::list(path))
//!     }
//! }
//! ```

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

use crate::icon::IconName;

/// Serves `icons/<name>.svg` for every [`IconName`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Assets;

/// The embedded bytes for `path`, if this crate has a file there.
pub fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    IconName::ALL
        .iter()
        .find(|icon| icon.path() == path)
        .map(|icon| Cow::Borrowed(icon.svg_bytes()))
}

/// Paths of the embedded files below `prefix` (e.g. `"icons/"`).
pub fn list(prefix: &str) -> Vec<SharedString> {
    IconName::ALL
        .iter()
        .map(|icon| icon.path())
        .filter(|path| path.starts_with(prefix))
        .map(SharedString::from)
        .collect()
}

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(load(path))
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(list(path))
    }
}
