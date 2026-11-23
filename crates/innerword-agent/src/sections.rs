//! Headings and the sections under them.

use std::ops::Range;

use innerword_core::{Document, TextRange};

use crate::error::{Candidate, Error, ItemKind, Result, quoted_excerpt};
use crate::types::HeadingInfo;

/// A heading of a document and the extent of its section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Heading {
    /// The index of the heading's block.
    pub(crate) block: usize,
    /// Its level, 1 to 6.
    pub(crate) level: u8,
    /// Its text, trimmed.
    pub(crate) text: String,
    /// Its number among the headings with exactly this text, from 1.
    pub(crate) occurrence: usize,
    /// The index of the block after the last block of its section: the next heading of
    /// the same or a higher level, or the end of the document.
    pub(crate) end: usize,
}

impl Heading {
    /// The blocks of the section, the heading included.
    pub(crate) fn section(&self) -> Range<usize> {
        self.block..self.end
    }

    /// The heading as results show it.
    pub(crate) fn info(&self, document: &Document) -> HeadingInfo {
        let last = self.end - 1;
        let words = document
            .stats_in(TextRange::new(
                innerword_core::Position::new(self.block, 0),
                innerword_core::Position::new(last, document.blocks()[last].len()),
            ))
            .words as u64;
        HeadingInfo {
            level: self.level,
            text: self.text.clone(),
            block: self.block,
            occurrence: self.occurrence,
            words,
        }
    }
}

/// Every heading of `document`, in order.
pub(crate) fn headings(document: &Document) -> Vec<Heading> {
    let mut found: Vec<Heading> = Vec::new();
    for (index, block) in document.blocks().iter().enumerate() {
        let Some(level) = block.kind().heading_level() else {
            continue;
        };
        let text = block.text().trim().to_string();
        let occurrence = 1 + found.iter().filter(|other| other.text == text).count();
        found.push(Heading {
            block: index,
            level,
            text,
            occurrence,
            end: document.block_count(),
        });
    }
    for i in 0..found.len() {
        let level = found[i].level;
        if let Some(next) = found[i + 1..].iter().find(|other| other.level <= level) {
            found[i].end = next.block;
        }
    }
    found
}

/// The nearest heading at or above block `index`, as text.
pub(crate) fn heading_above(document: &Document, index: usize) -> Option<String> {
    let index = index.min(document.block_count() - 1);
    document.blocks()[..=index]
        .iter()
        .rev()
        .find(|block| block.kind().heading_level().is_some())
        .map(|block| block.text().trim().to_string())
}

/// Reads how a request names a heading: its text, optionally with the `#` signs of its
/// level in front (`## Notes`).
fn parse_name(name: &str) -> (Option<u8>, String) {
    let trimmed = name.trim();
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    let rest = &trimmed[hashes..];
    if (1..=6).contains(&hashes) && (rest.is_empty() || rest.starts_with([' ', '\t'])) {
        let text = rest.trim().trim_end_matches('#').trim_end();
        // A heading that is all `#` signs is named as it is.
        if !text.is_empty() {
            return (Some(hashes as u8), collapse(text));
        }
    }
    (None, collapse(trimmed))
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The heading a request names. Exact text first, then without regard to case; with
/// several, `occurrence` (from 1) chooses, and without it the request is ambiguous.
pub(crate) fn find_heading(
    document: &Document,
    name: &str,
    occurrence: Option<usize>,
) -> Result<Heading> {
    let (level, wanted) = parse_name(name);
    if wanted.is_empty() {
        return Err(Error::invalid(
            "no heading was named: give the text of the heading",
        ));
    }
    let all = headings(document);
    let fits = |fold: bool| -> Vec<&Heading> {
        all.iter()
            .filter(|heading| level.is_none_or(|level| level == heading.level))
            .filter(|heading| {
                let text = collapse(&heading.text);
                if fold {
                    text.to_lowercase() == wanted.to_lowercase()
                } else {
                    text == wanted
                }
            })
            .collect()
    };
    let mut found = fits(false);
    if found.is_empty() {
        found = fits(true);
    }
    if found.is_empty() {
        let hint = if all.is_empty() {
            "The document has no headings".to_string()
        } else {
            let mut names: Vec<String> = all
                .iter()
                .take(12)
                .map(|heading| format!("{} {}", "#".repeat(heading.level as usize), heading.text))
                .collect();
            if all.len() > names.len() {
                names.push(format!("and {} more", all.len() - names.len()));
            }
            format!("The document's headings: {}", names.join("; "))
        };
        return Err(Error::NotFound {
            kind: ItemKind::Heading,
            reference: quoted_excerpt(name.trim()),
            hint: Some(hint),
        });
    }
    match occurrence {
        Some(number) => found
            .get(number.wrapping_sub(1))
            .map(|heading| (*heading).clone())
            .ok_or_else(|| {
                Error::invalid(format!(
                    "there {} {} with the text {}, so there is no occurrence {number}",
                    if found.len() == 1 { "is" } else { "are" },
                    if found.len() == 1 {
                        "1 heading".to_string()
                    } else {
                        format!("{} headings", found.len())
                    },
                    quoted_excerpt(name.trim()),
                ))
            }),
        None if found.len() == 1 => Ok(found[0].clone()),
        None => Err(Error::Ambiguous {
            kind: ItemKind::Heading,
            reference: quoted_excerpt(name.trim()),
            candidates: found
                .iter()
                .enumerate()
                .map(|(index, heading)| Candidate {
                    kind: ItemKind::Heading,
                    id: index as i64 + 1,
                    path: heading_path(&all, heading),
                    trashed: false,
                    reference: None,
                })
                .collect(),
        }),
    }
}

/// The headings above a heading and itself, as "Part one > Chapter 2 > Notes".
fn heading_path(all: &[Heading], heading: &Heading) -> String {
    let mut names = vec![heading.text.clone()];
    let mut level = heading.level;
    for other in all.iter().rev().filter(|other| other.block < heading.block) {
        if other.level < level {
            names.push(other.text.clone());
            level = other.level;
        }
    }
    names.reverse();
    names.join(" > ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use innerword_core::{Block, BlockKind};

    fn sample() -> Document {
        Document::from_blocks(vec![
            Block::paragraph("Preface."),
            Block::with_text(BlockKind::heading(1), "Part one"),
            Block::with_text(BlockKind::heading(2), "Notes"),
            Block::paragraph("a b c"),
            Block::with_text(BlockKind::heading(3), "Deeper"),
            Block::paragraph("d"),
            Block::with_text(BlockKind::heading(2), "Plan"),
            Block::with_text(BlockKind::heading(1), "Part two"),
            Block::with_text(BlockKind::heading(2), "Notes"),
            Block::paragraph("e f"),
        ])
    }

    #[test]
    fn sections_end_at_the_next_heading_of_the_same_or_a_higher_level() {
        let document = sample();
        let all = headings(&document);
        let extents: Vec<(usize, usize, usize)> = all
            .iter()
            .map(|heading| (heading.block, heading.end, heading.occurrence))
            .collect();
        assert_eq!(
            extents,
            [
                (1, 7, 1),
                (2, 6, 1),
                (4, 6, 1),
                (6, 7, 1),
                (7, 10, 1),
                (8, 10, 2)
            ]
        );
        assert_eq!(all[1].info(&document).words, 6);
        assert_eq!(heading_above(&document, 0), None);
        assert_eq!(heading_above(&document, 5).as_deref(), Some("Deeper"));
        assert_eq!(heading_above(&document, 99).as_deref(), Some("Notes"));
    }

    #[test]
    fn headings_are_named_by_their_text() {
        let document = sample();
        assert_eq!(find_heading(&document, "Plan", None).unwrap().block, 6);
        assert_eq!(find_heading(&document, "## Plan", None).unwrap().block, 6);
        assert_eq!(
            find_heading(&document, " part  TWO ", None).unwrap().block,
            7
        );
        assert!(matches!(
            find_heading(&document, "# Plan", None),
            Err(Error::NotFound { .. })
        ));
        match find_heading(&document, "Notes", None) {
            Err(Error::Ambiguous { candidates, .. }) => {
                assert_eq!(candidates.len(), 2);
                assert_eq!(candidates[0].path, "Part one > Notes");
                assert_eq!(candidates[1].path, "Part two > Notes");
                assert_eq!(candidates[1].id, 2);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(find_heading(&document, "Notes", Some(2)).unwrap().block, 8);
        assert!(matches!(
            find_heading(&document, "Notes", Some(3)),
            Err(Error::Invalid { .. })
        ));
        match find_heading(&document, "Missing", None) {
            Err(Error::NotFound { hint, .. }) => {
                assert!(hint.unwrap().contains("## Notes"));
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            find_heading(&document, "  ", None),
            Err(Error::Invalid { .. })
        ));
    }
}
