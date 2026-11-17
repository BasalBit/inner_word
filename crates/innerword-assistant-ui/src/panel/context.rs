//! The picker of the context row: the documents and folders of the
//! library, fuzzy, with the folder each is in as detail.
//!
//! The host supplies the candidates ([`AssistantPanel::set_library_source`])
//! as the engine's `Attachment`s: kind, id, title and path. What is
//! attached already, and the open document itself, are left out.

use gpui::{App, Context, SharedString, WeakEntity, Window};
use innerword_assistant::{Attachment, AttachmentKind};
use innerword_ui::{
    IconName, Picker, PickerDelegate, PickerEntry, PickerItem,
    fuzzy::{self, StringMatch},
};

use super::AssistantPanel;

/// The picker over the library.
pub type ContextPicker = Picker<ContextPickerDelegate>;

/// What the folder path of a library item is called in the list: the path
/// without the item's own name.
pub(crate) fn folder_of(attachment: &Attachment) -> String {
    let path = attachment.path.trim_matches('/');
    let title = attachment.title.trim();
    let parent = match path.rsplit_once('/') {
        Some((parent, last)) if last.trim() == title => parent,
        // The path is the folder the item is in.
        Some(_) => path,
        None if path == title => "",
        None => path,
    };
    parent.replace('/', " / ")
}

/// See the [module docs](self).
pub struct ContextPickerDelegate {
    panel: WeakEntity<AssistantPanel>,
    candidates: Vec<Attachment>,
    matches: Vec<StringMatch>,
}

impl ContextPickerDelegate {
    pub(crate) fn new(panel: WeakEntity<AssistantPanel>, candidates: Vec<Attachment>) -> Self {
        Self {
            panel,
            candidates,
            matches: Vec::new(),
        }
    }

    /// The candidates the list shows now, best match first.
    pub fn shown(&self) -> Vec<&Attachment> {
        self.matches
            .iter()
            .filter_map(|found| self.candidates.get(found.candidate_index))
            .collect()
    }
}

impl PickerDelegate for ContextPickerDelegate {
    fn update_matches(&mut self, query: &str, _: &mut Window, _: &mut Context<Picker<Self>>) {
        self.matches = fuzzy::match_strings(
            query,
            self.candidates
                .iter()
                .map(|candidate| candidate.title.as_str()),
            usize::MAX,
        );
    }

    fn entry_count(&self) -> usize {
        self.matches.len()
    }

    fn entry(&self, ix: usize, _: &App) -> PickerEntry {
        let found = &self.matches[ix];
        let candidate = &self.candidates[found.candidate_index];
        let folder = folder_of(candidate);
        let item = PickerItem::new(candidate.title.clone())
            .positions(found.positions.clone())
            .icon(match candidate.kind {
                AttachmentKind::Document => IconName::Document,
                AttachmentKind::Folder => IconName::Folder,
            });
        if folder.is_empty() {
            item.into()
        } else {
            item.detail(folder).into()
        }
    }

    fn confirm(
        &mut self,
        ix: usize,
        _secondary: bool,
        window: &mut Window,
        cx: &mut Context<Picker<Self>>,
    ) -> bool {
        let Some(candidate) = self
            .matches
            .get(ix)
            .and_then(|found| self.candidates.get(found.candidate_index))
            .cloned()
        else {
            return true;
        };
        let panel = self.panel.clone();
        // After the picker's own update.
        window.defer(cx, move |_, cx| {
            panel
                .update(cx, |panel, cx| {
                    panel.attach(candidate, cx);
                })
                .ok();
        });
        true
    }

    fn placeholder(&self) -> SharedString {
        "Add a document or a folder".into()
    }

    fn empty_text(&self, query: &str) -> SharedString {
        if query.is_empty() {
            "Nothing else in the library to add".into()
        } else {
            "No document or folder of that name".into()
        }
    }
}
