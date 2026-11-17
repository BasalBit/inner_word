//! Images in the workspace: what the app adds to what the editor and its
//! tools do with a picture.
//!
//! The editor draws, inserts, resizes and moves an image
//! (`innerword_editor::images`), its tools have the menu and the description
//! (`innerword_editor_tools`), and the bytes come through
//! [`crate::images::LibraryImages`], which [`Workspace::new`] gives the
//! editor of the document. What is left for the workspace:
//!
//! - **A command that is only about an image, chosen without one.** The
//!   size presets, "Original Size" and "Description…" are in the menu bar
//!   and the palette whenever a document is open (the workspace has a
//!   handler for every command of the text, so AppKit cannot grey them out
//!   by what is selected). The editor and its tools take such a command
//!   only while the caret is on an image; otherwise it comes up to the
//!   workspace root, and the writer is told what it needs
//!   ([`SELECT_AN_IMAGE_FIRST`]).
//! - **What a scripted run says about images** ([`Workspace::describe_images`]).

use gpui::{Action, Context, Focusable, Window};
use innerword_core::Alignment;
use innerword_editor::{ImageStatus, actions as editor};
use innerword_ui::Toast;

use super::Workspace;

/// What is said when a command that needs a selected image is chosen
/// without one.
pub const SELECT_AN_IMAGE_FIRST: &str =
    "Select an image first: click it, or move the caret onto it.";

/// Whether `action` is one of the commands that are only about the
/// selected image: the four widths, the original size and the
/// description. ("Larger" and "smaller" are not: without an image they
/// change the size of the text.)
pub fn is_image_command(action: &dyn Action) -> bool {
    let only_images: [&dyn Action; 6] = [
        &editor::ImageWidth25,
        &editor::ImageWidth50,
        &editor::ImageWidth75,
        &editor::ImageWidth100,
        &editor::ImageOriginalSize,
        &innerword_editor_tools::actions::EditImageDescription,
    ];
    only_images
        .iter()
        .any(|command| command.name() == action.name())
}

impl Workspace {
    /// A command for the text came up to the workspace root while the
    /// keyboard is in the text or in one of its tools. When it is one that
    /// is only about an image ([`is_image_command`]) this deals with it
    /// and returns true:
    ///
    /// - no image is selected: nothing changes, and the writer is told
    ///   ([`SELECT_AN_IMAGE_FIRST`]);
    /// - an image is selected and the keyboard is beside the text (in a
    ///   tool's field, in the field a comment is written in): the command
    ///   goes to the text, which takes the keyboard.
    pub(crate) fn image_command_came_this_far(
        &mut self,
        action: &dyn Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !is_image_command(action) {
            return false;
        }
        let (on_image, focus) = {
            let editor = self.editor.read(cx);
            (editor.selected_image().is_some(), editor.focus_handle(cx))
        };
        if !on_image {
            self.show_toast(Toast::new(SELECT_AN_IMAGE_FIRST), cx);
        } else if !focus.is_focused(window) {
            window.focus(&focus);
            window.dispatch_action(action.boxed_clone(), cx);
        }
        true
    }

    /// What the `state` step of a scripted run says about images: how
    /// many the open document holds, and while the caret is on one where
    /// it is, how wide, its pixel size and density, its alignment and
    /// whether its picture is there:
    /// `images=2 image=[block=3 width=50% natural=1600x1067@144 align=center ready]`.
    pub fn describe_images(&self, cx: &gpui::App) -> String {
        let editor = self.editor.read(cx);
        let count = editor.document().images().count();
        let mut said = format!("images={count}");
        if let Some((block, image)) = editor.selected_image() {
            let align = match editor
                .document()
                .block(block)
                .map(|block| block.alignment())
            {
                Some(Alignment::Center) => "center",
                Some(Alignment::Right) => "right",
                _ => "left",
            };
            let status = match editor.image_status(image.id) {
                ImageStatus::Ready => "ready",
                ImageStatus::Loading | ImageStatus::NoProvider => "loading",
                ImageStatus::Missing => "missing",
                ImageStatus::Broken => "broken",
            };
            let description = if image.description.is_empty() {
                String::new()
            } else {
                format!(" alt={:?}", image.description)
            };
            said.push_str(&format!(
                " image=[block={block} width={}% natural={}x{}@{} align={align} {status}{description}]",
                image.width.percent(),
                image.pixels.0,
                image.pixels.1,
                image.density,
            ));
        }
        said
    }
}
