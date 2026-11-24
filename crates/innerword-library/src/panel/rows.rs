//! The flattened row model of the tree view, the name filter and the small text formatters.
//! Everything here is plain data: no GPUI, no store access.

use std::collections::HashSet;

use gpui::SharedString;
use innerword_store::{DocumentKind, DocumentMeta, FolderId, FolderNode, LibraryTree};

use super::LibraryItem;

/// The label of a document without a title.
pub(super) const UNTITLED: &str = "Untitled";

/// One visible row of the tree.
#[derive(Clone, Debug)]
pub(super) struct Row {
    pub item: LibraryItem,
    pub depth: usize,
    pub label: SharedString,
    /// A document without a title (its label is the placeholder).
    pub untitled: bool,
    /// Folders: drawn open. Always false for documents.
    pub expanded: bool,
    /// Folders: has at least one subfolder or document.
    pub has_children: bool,
    pub parent: Option<FolderId>,
    /// Byte offsets in `label` of the characters matched by the filter.
    pub positions: Vec<usize>,
    /// Documents: last edit, unix milliseconds.
    pub updated_at: i64,
    /// Documents: words in the document.
    pub word_count: u64,
    /// Folders: live documents in the whole subtree.
    pub document_count: usize,
    /// The kind of a document's item: a text, a page (an HTML page kept in the library) or
    /// a whiteboard. Always [`DocumentKind::Text`] for folders, where it means nothing.
    pub kind: DocumentKind,
}

/// A name filter: every whitespace-separated word must occur in the name, ignoring case.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Filter {
    words: Vec<Vec<char>>,
}

impl Filter {
    /// `None` when `text` has no words.
    pub fn new(text: &str) -> Option<Self> {
        let words: Vec<Vec<char>> = text
            .split_whitespace()
            .map(|word| word.chars().flat_map(char::to_lowercase).collect())
            .collect();
        (!words.is_empty()).then_some(Self { words })
    }

    /// The byte offsets of the matched characters of `label` (sorted, unique), or `None`
    /// when a word is missing.
    pub fn matches(&self, label: &str) -> Option<Vec<usize>> {
        // Lowercasing can change a character's length, so keep each folded character's
        // offset in the original string.
        let folded: Vec<(usize, char)> = label
            .char_indices()
            .flat_map(|(offset, ch)| ch.to_lowercase().map(move |lower| (offset, lower)))
            .collect();
        let mut positions = Vec::new();
        for word in &self.words {
            let start = folded
                .windows(word.len())
                .position(|window| window.iter().map(|(_, ch)| ch).eq(word.iter()))?;
            positions.extend(
                folded[start..start + word.len()]
                    .iter()
                    .map(|(offset, _)| *offset),
            );
        }
        positions.sort_unstable();
        positions.dedup();
        Some(positions)
    }
}

fn document_label(document: &DocumentMeta) -> (SharedString, bool) {
    let title = document.title.trim();
    if title.is_empty() {
        (UNTITLED.into(), true)
    } else {
        (title.to_string().into(), false)
    }
}

fn document_row(
    document: &DocumentMeta,
    depth: usize,
    parent: Option<FolderId>,
    positions: Vec<usize>,
) -> Row {
    let (label, untitled) = document_label(document);
    Row {
        item: LibraryItem::Document(document.id),
        depth,
        label,
        untitled,
        expanded: false,
        has_children: false,
        parent,
        positions,
        updated_at: document.updated_at,
        word_count: document.word_count,
        document_count: 0,
        kind: document.kind,
    }
}

fn folder_row(
    node: &FolderNode,
    depth: usize,
    parent: Option<FolderId>,
    expanded: bool,
    positions: Vec<usize>,
) -> Row {
    Row {
        item: LibraryItem::Folder(node.folder.id),
        depth,
        label: node.folder.name.clone().into(),
        untitled: false,
        expanded,
        has_children: !node.folders.is_empty() || !node.documents.is_empty(),
        parent,
        positions,
        updated_at: node.folder.updated_at,
        word_count: 0,
        document_count: count_documents(node),
        kind: DocumentKind::Text,
    }
}

/// Live documents in a folder's whole subtree.
pub(super) fn count_documents(node: &FolderNode) -> usize {
    node.documents.len() + node.folders.iter().map(count_documents).sum::<usize>()
}

/// What lives in a folder's whole subtree besides its folders, in one count named by
/// kind: "3 documents", "1 page", "2 whiteboards", or "4 items" when they are of more than
/// one kind. `None` when there is nothing.
pub(super) fn items_in(node: &FolderNode) -> Option<String> {
    fn kinds(node: &FolderNode, seen: &mut Vec<DocumentKind>) -> usize {
        for document in &node.documents {
            if !seen.contains(&document.kind) {
                seen.push(document.kind);
            }
        }
        node.documents.len()
            + node
                .folders
                .iter()
                .map(|folder| kinds(folder, seen))
                .sum::<usize>()
    }
    let mut seen = Vec::new();
    let count = kinds(node, &mut seen) as u64;
    match seen.as_slice() {
        [] => None,
        [kind] => Some(plural(count, kind.noun())),
        _ => Some(plural(count, "item")),
    }
}

/// Subfolders in a folder's whole subtree.
pub(super) fn count_folders(node: &FolderNode) -> usize {
    node.folders.len() + node.folders.iter().map(count_folders).sum::<usize>()
}

/// Flattens the tree into the visible rows: folders first, then documents, each in the
/// user's order. Without a filter, collapsed folders hide their contents. With a filter
/// the rows are the items whose name matches, everything inside a matching folder, and the
/// folders that lead to them, all drawn open.
pub(super) fn build_rows(
    tree: &LibraryTree,
    expanded: &HashSet<FolderId>,
    filter: Option<&Filter>,
) -> Vec<Row> {
    let mut rows = Vec::new();
    match filter {
        None => {
            push_folders(&tree.folders, 0, None, expanded, &mut rows);
            for document in &tree.documents {
                rows.push(document_row(document, 0, None, Vec::new()));
            }
        }
        Some(filter) => {
            for node in &tree.folders {
                push_filtered(node, 0, None, false, filter, &mut rows);
            }
            for document in &tree.documents {
                let (label, _) = document_label(document);
                if let Some(positions) = filter.matches(&label) {
                    rows.push(document_row(document, 0, None, positions));
                }
            }
        }
    }
    rows
}

fn push_folders(
    nodes: &[FolderNode],
    depth: usize,
    parent: Option<FolderId>,
    expanded: &HashSet<FolderId>,
    rows: &mut Vec<Row>,
) {
    for node in nodes {
        let id = node.folder.id;
        let open = expanded.contains(&id);
        rows.push(folder_row(node, depth, parent, open, Vec::new()));
        if open {
            push_folders(&node.folders, depth + 1, Some(id), expanded, rows);
            for document in &node.documents {
                rows.push(document_row(document, depth + 1, Some(id), Vec::new()));
            }
        }
    }
}

/// Pushes `node` and what the filter keeps of its subtree. Returns whether anything was
/// pushed.
fn push_filtered(
    node: &FolderNode,
    depth: usize,
    parent: Option<FolderId>,
    inside_match: bool,
    filter: &Filter,
    rows: &mut Vec<Row>,
) -> bool {
    let id = node.folder.id;
    let matched = filter.matches(&node.folder.name);
    let keep_all = inside_match || matched.is_some();
    let start = rows.len();
    rows.push(folder_row(
        node,
        depth,
        parent,
        true,
        matched.unwrap_or_default(),
    ));
    let mut any = false;
    for child in &node.folders {
        any |= push_filtered(child, depth + 1, Some(id), keep_all, filter, rows);
    }
    for document in &node.documents {
        let (label, _) = document_label(document);
        let positions = filter.matches(&label);
        if keep_all || positions.is_some() {
            rows.push(document_row(
                document,
                depth + 1,
                Some(id),
                positions.unwrap_or_default(),
            ));
            any = true;
        }
    }
    if any || keep_all {
        true
    } else {
        rows.truncate(start);
        false
    }
}

/// The folders that lead to `item`, outermost first, or `None` when it is not in the tree.
pub(super) fn ancestors(tree: &LibraryTree, item: LibraryItem) -> Option<Vec<FolderId>> {
    fn walk(nodes: &[FolderNode], item: LibraryItem, path: &mut Vec<FolderId>) -> bool {
        for node in nodes {
            if item == LibraryItem::Folder(node.folder.id) {
                return true;
            }
            path.push(node.folder.id);
            let here = node
                .documents
                .iter()
                .any(|document| item == LibraryItem::Document(document.id));
            if here || walk(&node.folders, item, path) {
                return true;
            }
            path.pop();
        }
        false
    }
    if tree
        .documents
        .iter()
        .any(|document| item == LibraryItem::Document(document.id))
    {
        return Some(Vec::new());
    }
    let mut path = Vec::new();
    walk(&tree.folders, item, &mut path).then_some(path)
}

/// How long ago `then_ms` was, as a short label: "now", "5m", "3h", "2d", "3w", "5mo", "2y".
pub(super) fn relative_time(now_ms: i64, then_ms: i64) -> String {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let seconds = (now_ms - then_ms).max(0) / 1000;
    if seconds < MINUTE {
        "now".into()
    } else if seconds < HOUR {
        format!("{}m", seconds / MINUTE)
    } else if seconds < DAY {
        format!("{}h", seconds / HOUR)
    } else if seconds < 7 * DAY {
        format!("{}d", seconds / DAY)
    } else if seconds < 30 * DAY {
        format!("{}w", seconds / (7 * DAY))
    } else if seconds < 365 * DAY {
        format!("{}mo", seconds / (30 * DAY))
    } else {
        format!("{}y", seconds / (365 * DAY))
    }
}

/// A number with thousands separators: 1240 gives "1,240".
pub(super) fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "1 document" / "3 documents".
pub(super) fn plural(count: u64, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{} {noun}s", grouped(count))
    }
}

/// Where a live item is: the folder that holds it (`None` is the root) and its index among
/// its siblings of the same kind (folders among folders, documents among documents). `None`
/// when the item is not in the tree.
pub(super) fn place_of(tree: &LibraryTree, item: LibraryItem) -> Option<(Option<FolderId>, usize)> {
    let parent = ancestors(tree, item)?.last().copied();
    let (folders, documents) = match parent {
        Some(id) => {
            let node = tree.find_folder(id)?;
            (&node.folders, &node.documents)
        }
        None => (&tree.folders, &tree.documents),
    };
    let index = match item {
        LibraryItem::Folder(id) => folders.iter().position(|node| node.folder.id == id),
        LibraryItem::Document(id) => documents.iter().position(|document| document.id == id),
    }?;
    Some((parent, index))
}

/// The display name of a live item: the folder's name, or the document's title
/// ("Untitled" when it has none). `None` when the item is not in the tree.
pub(super) fn item_name(tree: &LibraryTree, item: LibraryItem) -> Option<String> {
    match item {
        LibraryItem::Folder(id) => tree.find_folder(id).map(|node| node.folder.name.clone()),
        LibraryItem::Document(id) => tree
            .find_document(id)
            .map(|document| document_label(document).0.to_string()),
    }
}

/// The stored name of a live item: the folder's name, or the document's title (empty when
/// it has none). `None` when the item is not in the tree.
pub(super) fn stored_name(tree: &LibraryTree, item: LibraryItem) -> Option<String> {
    match item {
        LibraryItem::Folder(id) => tree.find_folder(id).map(|node| node.folder.name.clone()),
        LibraryItem::Document(id) => tree
            .find_document(id)
            .map(|document| document.title.trim().to_string()),
    }
}

/// `name` as it appears inside a one-line message: cut to `max` characters with an
/// ellipsis. Toasts are one line and clip what does not fit, so a long title must not push
/// the end of the sentence (the key that undoes) out of sight.
pub fn short(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        return name.to_string();
    }
    let cut: String = name.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// A folder path ("Novel / Part one / Scenes") as it appears inside a one-line message: when
/// it is longer than `max` characters the folders at the end are kept, which say most
/// about where something is ("… / Part one / Scenes").
pub fn short_path(names: &[String], max: usize) -> String {
    let whole = names.join(" / ");
    if whole.chars().count() <= max {
        return whole;
    }
    let mut kept: Vec<&str> = Vec::new();
    let mut length = 1;
    for name in names.iter().rev() {
        let more = name.chars().count() + 3;
        if length + more > max && !kept.is_empty() {
            break;
        }
        kept.push(name);
        length += more;
    }
    kept.reverse();
    match kept.as_slice() {
        // Even the last folder alone is too long.
        [only] if length > max => format!("… / {}", short(only, max.saturating_sub(4))),
        kept => format!("… / {}", kept.join(" / ")),
    }
}

/// A row cuts a long name off at its end. When the filter's first match in `label` would
/// fall outside the first `fit` characters, this returns the label cut at the front
/// instead ("…the sound the radiator makes") with the match positions moved along, so the
/// reason the row is in the list can be seen. `None` when the match is in view anyway.
pub fn label_around_match(
    label: &str,
    positions: &[usize],
    fit: usize,
) -> Option<(String, Vec<usize>)> {
    let first = *positions.first()?;
    let before = label.get(..first)?.chars().count();
    // The first run of matched characters (one word of the filter).
    let mut run = 0;
    let mut expected = first;
    for (offset, ch) in label[first..].char_indices() {
        if positions.binary_search(&(first + offset)).is_err() || first + offset != expected {
            break;
        }
        run += 1;
        expected = first + offset + ch.len_utf8();
    }
    // One place is the row's own ellipsis at the end.
    if before + run < fit {
        return None;
    }
    // Keep some of what comes before the match, starting at a word if there is one.
    let lead = before.min(fit.saturating_sub(run + 2) / 2);
    let earliest = before - lead;
    let chars: Vec<(usize, char)> = label.char_indices().collect();
    let start = (earliest..=before)
        .find(|&ix| ix == 0 || chars[ix - 1].1.is_whitespace())
        .unwrap_or(earliest);
    if start == 0 {
        return None;
    }
    let cut = chars[start].0;
    const ELLIPSIS: &str = "…";
    let moved = positions
        .iter()
        .filter(|&&position| position >= cut)
        .map(|&position| position - cut + ELLIPSIS.len())
        .collect();
    Some((format!("{ELLIPSIS}{}", &label[cut..]), moved))
}
