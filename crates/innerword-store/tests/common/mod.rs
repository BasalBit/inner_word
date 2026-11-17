#![allow(dead_code)]

use innerword_store::{Clock, DocumentContent, DocumentMeta, FolderId, Store};

pub const START: i64 = 1_700_000_000_000;
pub const MINUTE: i64 = 60_000;
pub const HOUR: i64 = 60 * MINUTE;
pub const DAY: i64 = 24 * HOUR;

pub fn store() -> (Store, Clock) {
    let clock = Clock::manual(START);
    (
        Store::open_in_memory_with_clock(clock.clone()).unwrap(),
        clock,
    )
}

pub fn content<'a>(title: &'a str, text: &'a str, json: &'a str) -> DocumentContent<'a> {
    DocumentContent {
        content: json,
        title,
        plain_text: text,
        word_count: text.split_whitespace().count() as u64,
        char_count: text.chars().count() as u64,
    }
}

pub fn doc(store: &mut Store, folder: Option<FolderId>, title: &str, text: &str) -> DocumentMeta {
    let json = format!("{{\"text\":{:?}}}", text);
    store
        .create_document(folder, &content(title, text, &json))
        .unwrap()
}

pub fn folder_names(store: &Store, parent: Option<FolderId>) -> Vec<String> {
    store
        .child_folders(parent)
        .unwrap()
        .into_iter()
        .map(|f| f.name)
        .collect()
}

pub fn doc_titles(store: &Store, folder: Option<FolderId>) -> Vec<String> {
    store
        .documents_in_folder(folder)
        .unwrap()
        .into_iter()
        .map(|d| d.title)
        .collect()
}
