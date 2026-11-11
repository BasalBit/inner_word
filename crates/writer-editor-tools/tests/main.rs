//! Headless tests of the editor tools, one module per tool. One test
//! binary for the whole crate (see the workspace rules on GPUI binaries).

mod find;
mod font_picker;
mod format_menu;
mod insert_menu;
mod integration;
mod layout;
mod link;
mod mouse;
mod perf;
mod support;
mod text_menu;
mod util;
