//! Bytes as a base64 string in JSON (`Blob::data`).

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Deserializer, Serializer};

pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&STANDARD.encode(bytes))
}

pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
    let text: std::borrow::Cow<'de, str> = Deserialize::deserialize(deserializer)?;
    STANDARD
        .decode(text.as_bytes())
        .map_err(serde::de::Error::custom)
}
