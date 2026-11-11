//! Theme tokens and shared GPUI widgets for The Writer.
//!
//! # Setup
//!
//! ```ignore
//! Application::new()
//!     .with_assets(writer_ui::assets::Assets)      // icons (see `assets` to chain your own)
//!     .run(|cx| {
//!         writer_ui::theme::init(ThemeMode::System, cx);
//!         // app-wide bindings first, then the widget bindings, so that a
//!         // widget binding wins over a context-free app binding for the same key
//!         writer_ui::init(cx);
//!     });
//! ```
//!
//! # Widgets
//!
//! | Module | What |
//! |---|---|
//! | [`text_field`] | [`TextField`]: single-line input with IME, selection, clipboard, undo |
//! | [`picker`] | [`Picker`]: query field + filtered virtual list, driven by a [`PickerDelegate`] |
//! | [`fuzzy`] | Fuzzy subsequence matcher with match positions |
//! | [`modal`] | [`ModalLayer`]: one modal at a time, backdrop, focus restore |
//! | [`key_hint`] | [`KeyHint`]: key bindings as macOS glyphs, looked up from the keymap |
//! | [`icon`] | [`Icon`] / [`IconName`]: embedded 16px line icons |
//! | [`button`] | [`Button`]: primary / subtle / danger, focusable |
//! | [`toggle`] | [`Switch`] and [`Checkbox`] rows |
//! | [`segmented`] | [`SegmentedControl`] |
//! | [`label`] | [`SectionLabel`], [`Divider`], [`HighlightedLabel`] |
//! | [`confirm`] | [`ConfirmDialog`] |
//! | [`toast`] | [`ToastLayer`] / [`Toast`] |
//! | [`focus`] | tab navigation helper |
//! | [`command`] | [`CommandDescriptor`]: how crates publish commands to the palette and shortcuts overlay |

pub mod assets;
pub mod button;
pub mod command;
pub mod confirm;
pub mod focus;
pub mod fuzzy;
pub mod icon;
pub mod key_hint;
pub mod label;
pub mod modal;
pub mod picker;
pub mod segmented;
pub mod text_field;
pub mod theme;
pub mod toast;
pub mod toggle;

pub use button::{Button, ButtonSize, ButtonVariant};
pub use command::CommandDescriptor;
pub use confirm::{ConfirmDialog, ConfirmDialogEvent};
pub use icon::{Icon, IconName};
pub use key_hint::KeyHint;
pub use label::{Divider, HighlightedLabel, SectionLabel};
pub use modal::{ModalLayer, ModalPlacement};
pub use picker::{Picker, PickerDelegate, PickerEntry, PickerItem};
pub use segmented::SegmentedControl;
pub use text_field::{FieldAppearance, TextField, TextFieldEvent};
pub use theme::{ActiveTheme, Theme, ThemeColors, ThemeMode};
pub use toast::{Toast, ToastKind, ToastLayer};
pub use toggle::{Checkbox, Switch};

/// Installs the key bindings of every widget in this crate. Call once at
/// startup, after [`theme::init`] and after the app's own context-free
/// bindings (bindings without a context tie with the deepest context and
/// the later one wins).
pub fn init(cx: &mut gpui::App) {
    cx.bind_keys(text_field::key_bindings());
    cx.bind_keys(picker::key_bindings());
    cx.bind_keys(modal::key_bindings());
    cx.bind_keys(button::key_bindings());
    cx.bind_keys(toggle::key_bindings());
    cx.bind_keys(segmented::key_bindings());
    cx.bind_keys(confirm::key_bindings());
    cx.bind_keys(focus::key_bindings());
}
