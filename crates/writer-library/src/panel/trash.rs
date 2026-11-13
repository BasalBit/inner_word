//! The rows of the trash view, built from the library's trash listing.

use gpui::SharedString;
use writer_store::{DocumentKind, FolderId, TrashEntry};

use super::LibraryItem;
use super::rows::{UNTITLED, plural};
use crate::library::Library;

/// The label of the top level of the library, as a place.
pub(super) const ROOT_LABEL: &str = "Library root";

/// One top-level entry of the trash.
#[derive(Clone, Debug)]
pub(super) struct TrashRow {
    pub item: LibraryItem,
    pub label: SharedString,
    /// Where the item was, and so where a restore puts it: "Novel / Part one" or
    /// "Library root".
    pub location: SharedString,
    /// Folders: what went to the trash with it ("3 documents, 1 subfolder").
    pub contents: Option<SharedString>,
    /// A page rather than a text. Always false for folders.
    pub page: bool,
}

/// Reads the trash and describes each entry. One store query plus one cached-tree walk per
/// entry; called when the trash changes, never per frame.
pub(super) fn build_trash_rows(library: &Library) -> Vec<TrashRow> {
    let entries = library.trash().unwrap_or_default();
    entries
        .into_iter()
        .map(|entry| match entry {
            TrashEntry::Document(meta) => {
                let title = meta.title.trim();
                TrashRow {
                    item: LibraryItem::Document(meta.id),
                    label: if title.is_empty() {
                        UNTITLED.into()
                    } else {
                        title.to_string().into()
                    },
                    location: location_label(library, meta.folder_id),
                    contents: None,
                    page: meta.kind == DocumentKind::Page,
                }
            }
            TrashEntry::Folder {
                folder,
                folder_count,
                document_count,
            } => {
                let mut contents = plural(document_count, "document");
                if folder_count > 0 {
                    contents.push_str(", ");
                    contents.push_str(&plural(folder_count, "subfolder"));
                }
                TrashRow {
                    item: LibraryItem::Folder(folder.id),
                    label: folder.name.clone().into(),
                    location: location_label(library, folder.parent_id),
                    contents: Some(contents.into()),
                    page: false,
                }
            }
        })
        .collect()
}

/// The place a trashed item came from. When its folder is gone (trashed or deleted), a
/// restore puts the item at the root, and the label says so.
fn location_label(library: &Library, folder: Option<FolderId>) -> SharedString {
    let Some(folder) = folder else {
        return ROOT_LABEL.into();
    };
    let path = library.folder_path(Some(folder));
    if path.is_empty() {
        ROOT_LABEL.into()
    } else {
        path.join(" / ").into()
    }
}
