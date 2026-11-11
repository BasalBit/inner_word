//! The folder picker behind "Move to folder": a [`Picker`] listing "Library root" and every
//! folder the item may move into.

use gpui::{App, Context, SharedString, WeakEntity, Window};
use writer_store::{FolderId, FolderNode, LibraryTree};
use writer_ui::fuzzy::{StringMatch, match_strings};
use writer_ui::{IconName, Picker, PickerDelegate, PickerEntry, PickerItem};

use super::trash::ROOT_LABEL;
use super::{LibraryItem, LibraryPanel};

struct Target {
    folder: Option<FolderId>,
    /// "Novel / Part one", or "Library root".
    label: String,
    /// The item is in this folder already.
    current: bool,
}

/// The delegate of the move picker.
pub(super) struct MoveDelegate {
    panel: WeakEntity<LibraryPanel>,
    item: LibraryItem,
    item_label: SharedString,
    targets: Vec<Target>,
    matches: Vec<StringMatch>,
}

impl MoveDelegate {
    /// Lists the places `item` (currently in `current_parent`) may move to. A folder
    /// cannot move into itself or into one of its own subfolders, so those are left out.
    pub fn new(
        panel: WeakEntity<LibraryPanel>,
        item: LibraryItem,
        item_label: SharedString,
        current_parent: Option<FolderId>,
        tree: &LibraryTree,
    ) -> Self {
        fn collect(
            nodes: &[FolderNode],
            path: &mut Vec<String>,
            skip: Option<FolderId>,
            current: Option<FolderId>,
            out: &mut Vec<Target>,
        ) {
            for node in nodes {
                if Some(node.folder.id) == skip {
                    continue;
                }
                path.push(node.folder.name.clone());
                out.push(Target {
                    folder: Some(node.folder.id),
                    label: path.join(" / "),
                    current: current == Some(node.folder.id),
                });
                collect(&node.folders, path, skip, current, out);
                path.pop();
            }
        }
        let skip = match item {
            LibraryItem::Folder(id) => Some(id),
            LibraryItem::Document(_) => None,
        };
        let mut targets = vec![Target {
            folder: None,
            label: ROOT_LABEL.to_string(),
            current: current_parent.is_none(),
        }];
        collect(
            &tree.folders,
            &mut Vec::new(),
            skip,
            current_parent,
            &mut targets,
        );
        Self {
            panel,
            item,
            item_label,
            targets,
            matches: Vec::new(),
        }
    }
}

impl PickerDelegate for MoveDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.matches = match_strings(
            query,
            self.targets.iter().map(|target| target.label.as_str()),
            usize::MAX,
        );
    }

    fn entry_count(&self) -> usize {
        self.matches.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        let matched = &self.matches[ix];
        let target = &self.targets[matched.candidate_index];
        let mut item = PickerItem::new(target.label.clone())
            .positions(matched.positions.clone())
            .icon(if target.folder.is_some() {
                IconName::Folder
            } else {
                IconName::FolderOpen
            });
        if target.current {
            item = item.detail("Current location");
        }
        item.into()
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(matched) = self.matches.get(ix) else {
            return true;
        };
        let folder = self.targets[matched.candidate_index].folder;
        let item = self.item;
        let panel = self.panel.clone();
        // Deferred: runs after the picker closed and focus went back to the panel.
        window.defer(cx, move |_, cx| {
            panel
                .update(cx, |panel, cx| panel.move_item_and_report(item, folder, cx))
                .ok();
        });
        true
    }

    fn placeholder(&self) -> SharedString {
        format!("Move “{}” to…", self.item_label).into()
    }

    fn empty_text(&self, _query: &str) -> SharedString {
        "No matching folder".into()
    }
}
