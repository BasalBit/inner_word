//! Excalidraw's shape library as the library's setting `board.shape_library` keeps it
//! (`docs/whiteboard-design.md`, section 4.15).

use crate::Element;

/// The shape library: one for every whiteboard of the library.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeLibrary {
    pub(crate) items: Vec<LibraryItem>,
}

/// A library that was read, with what was left out.
#[derive(Clone, Debug, PartialEq)]
pub struct ParsedLibrary {
    pub library: ShapeLibrary,
    pub warnings: Vec<String>,
}

/// One item of the shape library: elements kept together to be put on a canvas again.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryItem {
    pub(crate) id: String,
    pub(crate) status: String,
    pub(crate) name: Option<String>,
    pub(crate) created: i64,
    pub(crate) elements: Vec<Element>,
}
