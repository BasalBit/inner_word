//! The id of stored bytes as text.

use serde::{Deserialize, Deserializer, Serialize};

/// 32 lowercase hex digits: the id of stored bytes (`innerword_store::FileId` as text; this
/// crate does not depend on the store). Only [`FileHex::parse`] makes one, so every value
/// is well formed, also one that was deserialized.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FileHex(String);

impl FileHex {
    /// The id in `text`: exactly 32 hex digits, lower or upper case (kept lower case).
    pub fn parse(text: &str) -> Option<FileHex> {
        if text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit()) {
            Some(FileHex(text.to_ascii_lowercase()))
        } else {
            None
        }
    }

    /// The 32 lowercase hex digits.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for FileHex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for FileHex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<FileHex, D::Error> {
        let text = String::deserialize(deserializer)?;
        FileHex::parse(&text)
            .ok_or_else(|| serde::de::Error::custom("not the id of stored bytes (32 hex digits)"))
    }
}
