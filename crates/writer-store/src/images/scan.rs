//! Which images stored content names.
//!
//! The store does not parse the content of a document. It looks for one token in it: the
//! key `"image"` with a value of 32 hex digits, which is how an image block names its
//! bytes (`{"type":"image","image":"3f2a…"}`, written by `writer-core`). Everything the
//! collector decides rests on this, so the scan errs to one side only: a hit that is not
//! an image block (a writer's own text holding the token) keeps an image alive that
//! could go, and nothing an image block can be written as is missed.

use std::collections::HashSet;

use rusqlite::Connection;

use super::ImageId;
use crate::error::Result;

/// The key an image block's id is written under, as it stands in JSON.
const KEY: &[u8] = b"\"image\"";

/// The tables whose `content` column holds documents: the documents themselves, live and
/// trashed, and their version snapshots.
const TABLES: [&str; 2] = ["documents", "snapshots"];

/// Every image id that the content stored in a library database names.
pub(crate) fn referenced(conn: &Connection) -> Result<HashSet<ImageId>> {
    let mut found = HashSet::new();
    for table in TABLES {
        // `instr` keeps the rows without the key (nearly all of a library) from being
        // handed over at all.
        let mut stmt = conn.prepare(&format!(
            "SELECT content FROM {table} WHERE instr(content, '\"image\"') > 0"
        ))?;
        let mut rows = stmt.query([])?;
        while let Some(row) = rows.next()? {
            let content = row.get_ref(0)?.as_bytes().map_err(rusqlite::Error::from)?;
            collect(content, &mut found);
        }
    }
    Ok(found)
}

/// Adds every id that `content` names: `"image"`, optional white space, `:`, optional
/// white space, `"`, 32 hex digits, `"`.
pub(crate) fn collect(content: &[u8], found: &mut HashSet<ImageId>) {
    let mut rest = content;
    while let Some(at) = find(rest, KEY) {
        rest = &rest[at + KEY.len()..];
        if let Some(id) = id_after_key(rest) {
            found.insert(id);
        }
    }
}

/// The id in `: "<32 hex digits>"` at the start of `after`, when that is what stands
/// there.
fn id_after_key(after: &[u8]) -> Option<ImageId> {
    let after = skip_white_space(after).strip_prefix(b":")?;
    let after = skip_white_space(after).strip_prefix(b"\"")?;
    let digits = after.get(..32)?;
    if after.get(32) != Some(&b'"') {
        return None;
    }
    ImageId::from_hex(digits)
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
}
