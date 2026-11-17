//! Which images and which files of pages stored content names.
//!
//! The store does not parse the content of a document. It looks for two tokens in it: the
//! key `"image"` with a value of 32 hex digits, which is how an image block names its
//! bytes (`{"type":"image","image":"3f2a…"}`, written by `innerword-core`), and the key
//! `"file"` with a value of 32 hex digits, which is how the manifest of a page names the
//! bytes of each of its files (`{"path":"index.html","file":"3f2a…",…}`, written by
//! `innerword-page`). Everything the collector decides rests on this, so the scan errs to one
//! side only: a hit that is not what it looks like (a writer's own text holding the token)
//! keeps bytes alive that could go, and nothing an image block or a manifest can be written
//! as is missed.

use std::collections::{BTreeSet, HashSet};

use rusqlite::Connection;

use super::file::Table;
use super::{FileId, ImageId, parse16};
use crate::error::Result;

/// The key an image block's id is written under, as it stands in JSON.
const KEY: &[u8] = b"\"image\"";

/// The key the manifest of a page writes the id of a file's bytes under.
const FILE_KEY: &[u8] = b"\"file\"";

/// The tables whose `content` column holds documents: the documents themselves (texts and
/// pages), live and trashed, and their version snapshots.
const TABLES: [&str; 2] = ["documents", "snapshots"];

/// Every id stored content names, of images and of files.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Named {
    /// The images.
    pub(crate) images: HashSet<ImageId>,
    /// The files of pages.
    pub(crate) files: HashSet<FileId>,
}

impl Named {
    /// Whether the row of `table` with this id is named.
    pub(crate) fn names(&self, table: Table, id: [u8; 16]) -> bool {
        match table {
            Table::Images => self.images.contains(&ImageId(id)),
            Table::Files => self.files.contains(&FileId(id)),
        }
    }

    /// The same as [`Named::names`] for the id as the column holds it; an id that cannot be
    /// read is named by nothing that can be found.
    pub(crate) fn names_hex(&self, table: Table, hex: &str) -> bool {
        parse16(hex.as_bytes()).is_some_and(|id| self.names(table, id))
    }

    pub(crate) fn extend(&mut self, other: Named) {
        self.images.extend(other.images);
        self.files.extend(other.files);
    }
}

/// Every image id that the content stored in a library database names.
pub(crate) fn referenced(conn: &Connection) -> Result<HashSet<ImageId>> {
    let mut found = HashSet::new();
    scan(conn, "instr(content, '\"image\"') > 0", |content| {
        collect(content, &mut found)
    })?;
    Ok(found)
}

/// Every file id that the content stored in a library database names.
pub(crate) fn referenced_files(conn: &Connection) -> Result<HashSet<FileId>> {
    let mut found = HashSet::new();
    scan(conn, "instr(content, '\"file\"') > 0", |content| {
        collect_files(content, &mut found)
    })?;
    Ok(found)
}

/// Every image id and every file id that the content stored in a library database names,
/// in one pass over the content.
pub(crate) fn referenced_all(conn: &Connection) -> Result<Named> {
    let mut found = Named::default();
    scan(
        conn,
        "instr(content, '\"image\"') > 0 OR instr(content, '\"file\"') > 0",
        |content| {
            collect(content, &mut found.images);
            collect_files(content, &mut found.files);
        },
    )?;
    Ok(found)
}

/// The file ids one manifest names, each once, in order.
pub(crate) fn files_in(content: &str) -> BTreeSet<FileId> {
    let mut found = BTreeSet::new();
    ids_after(content.as_bytes(), FILE_KEY, |id| {
        found.insert(FileId(id));
    });
    found
}

/// Hands `each` the content of every row of [`TABLES`] that `filter` lets through.
fn scan(conn: &Connection, filter: &str, mut each: impl FnMut(&[u8])) -> Result<()> {
    for table in TABLES {
        // `instr` keeps the rows without the key (nearly all of a library) from being
        // handed over at all.
        let mut stmt = conn.prepare(&format!("SELECT content FROM {table} WHERE {filter}"))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let content = row.get_ref(0)?.as_bytes().map_err(rusqlite::Error::from)?;
            each(content);
        }
    }
    Ok(())
}

/// Adds every image id that `content` names: `"image"`, optional white space, `:`,
/// optional white space, `"`, 32 hex digits, `"`.
pub(crate) fn collect(content: &[u8], found: &mut HashSet<ImageId>) {
    ids_after(content, KEY, |id| {
        found.insert(ImageId(id));
    });
}

/// Adds every file id that `content` names: `"file"`, optional white space, `:`,
/// optional white space, `"`, 32 hex digits, `"`.
pub(crate) fn collect_files(content: &[u8], found: &mut HashSet<FileId>) {
    ids_after(content, FILE_KEY, |id| {
        found.insert(FileId(id));
    });
}

/// Hands `each` every id that stands after `key` in `content`.
fn ids_after(content: &[u8], key: &[u8], mut each: impl FnMut([u8; 16])) {
    let mut rest = content;
    while let Some(at) = find(rest, key) {
        rest = &rest[at + key.len()..];
        if let Some(id) = id_after_key(rest) {
            each(id);
        }
    }
}

/// The id in `: "<32 hex digits>"` at the start of `after`, when that is what stands
/// there.
fn id_after_key(after: &[u8]) -> Option<[u8; 16]> {
    let after = skip_white_space(after).strip_prefix(b":")?;
    let after = skip_white_space(after).strip_prefix(b"\"")?;
    let digits = after.get(..32)?;
    if after.get(32) != Some(&b'"') {
        return None;
    }
    parse16(digits)
}

/// White space as JSON has it between a key, the colon and a value.
fn skip_white_space(text: &[u8]) -> &[u8] {
    let white = text
        .iter()
        .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        .count();
    &text[white..]
}

/// Where `needle` first stands in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    let first = *needle.first()?;
    let mut from = 0;
    while let Some(offset) = haystack[from..].iter().position(|byte| *byte == first) {
        let at = from + offset;
        if haystack[at..].starts_with(needle) {
            return Some(at);
        }
        from = at + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "3f2a9c1b5d6e7f80a1b2c3d4e5f60718";
    const B: &str = "00112233445566778899aabbccddeeff";

    fn ids(content: &str) -> Vec<String> {
        let mut found = HashSet::new();
        collect(content.as_bytes(), &mut found);
        let mut found: Vec<String> = found.iter().map(ImageId::to_hex).collect();
        found.sort();
        found
    }

    #[test]
    fn the_id_of_an_image_block_is_found() {
        let content = format!(
            r#"{{"version":2,"blocks":[{{"id":1,"type":"paragraph","text":"Before"}},{{"id":7,"type":"image","image":"{A}","px":[1600,1067],"ppi":144,"width":500,"alt":"The harbour at dusk","align":"center"}},{{"id":8,"type":"image","image":"{B}","px":[1,1]}}]}}"#
        );
        assert_eq!(ids(&content), vec![B.to_owned(), A.to_owned()]);
    }

    #[test]
    fn white_space_around_the_colon_is_read_past() {
        assert_eq!(ids(&format!(r#"{{"image" : "{A}"}}"#)), vec![A]);
        assert_eq!(ids(&format!("{{\"image\"\n\t:\r\n  \"{A}\"}}")), vec![A]);
        assert_eq!(
            ids(&format!(r#"{{"image":"{}"}}"#, A.to_uppercase())),
            vec![A]
        );
    }

    #[test]
    fn what_is_not_an_id_is_not_taken_for_one() {
        // A number, a value one digit short, one digit long, one that is not hex, one
        // that does not end, the type of the block, the key at the very end.
        for content in [
            r#"{"image": 3}"#.to_owned(),
            format!(r#"{{"image":"{}"}}"#, &A[..31]),
            format!(r#"{{"image":"{A}0"}}"#),
            format!(r#"{{"image":"{}g"}}"#, &A[..31]),
            format!(r#"{{"image":"{A}"#),
            format!(r#"{{"type":"image","px":[1,1],"alt":"{A}"}}"#),
            format!(r#"{{"images":"{A}"}}"#),
            format!(r#"{{"image""{A}"}}"#),
            r#"{"type":"image"}"#.to_owned(),
            r#""image""#.to_owned(),
            r#""image":"#.to_owned(),
            r#""image":""#.to_owned(),
            String::new(),
        ] {
            assert_eq!(ids(&content), Vec::<String>::new(), "{content}");
        }
    }

    /// Text a writer typed is inside a JSON string, where its quotes are escaped: it
    /// does not read as the key. (If it did, an image would only be kept for longer.)
    #[test]
    fn the_token_typed_into_a_text_is_escaped_and_not_a_hit() {
        let typed = format!(r#""image":"{A}""#);
        let content = serde_json::json!({ "blocks": [{ "type": "paragraph", "text": typed }] });
        assert_eq!(ids(&content.to_string()), Vec::<String>::new());
    }

    #[test]
    fn every_occurrence_is_looked_at() {
        let content =
            format!(r#"[{{"image":3}},{{"image":"{A}"}},{{"image":"{A}"}},{{"image":"{B}"}}]"#);
        assert_eq!(ids(&content), vec![B.to_owned(), A.to_owned()]);
    }

    fn files(content: &str) -> Vec<String> {
        let mut found = HashSet::new();
        collect_files(content.as_bytes(), &mut found);
        let mut found: Vec<String> = found.iter().map(FileId::to_hex).collect();
        found.sort();
        found
    }

    /// The manifest of a page names each blob under "file"; the images' key and the files'
    /// key are told apart, and a path that holds the word is not a key.
    #[test]
    fn the_files_of_a_manifest_are_found_and_kept_apart_from_images() {
        let manifest = format!(
            r#"{{"page":1,"entry":"index.html","files":[{{"path":"index.html","file":"{A}","type":"text/html","size":3}},{{"path":"file/b.css","file" : "{B}","type":"text/css","size":1}}],"original":[{{"path":"index.html","file":"{A}","type":"text/html","size":3}}],"text":"\"file\":\"{B}\""}}"#
        );
        assert_eq!(files(&manifest), vec![B.to_owned(), A.to_owned()]);
        assert_eq!(ids(&manifest), Vec::<String>::new());
        let both = format!(r#"[{{"image":"{A}"}},{{"file":"{B}"}},{{"files":"{A}"}}]"#);
        assert_eq!(ids(&both), vec![A]);
        assert_eq!(files(&both), vec![B]);
        let in_order: Vec<String> = files_in(&manifest).iter().map(FileId::to_hex).collect();
        assert_eq!(in_order, vec![B.to_owned(), A.to_owned()]);
    }
}
