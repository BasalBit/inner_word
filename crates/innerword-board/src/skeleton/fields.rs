//! Reading the fields of a skeleton or a change: each value checked, each refusal with the
//! element's index and the field.

use serde_json::{Map, Value};

use super::BoardError;
use crate::limits::{MAX_ID_CHARS, MAX_TEXT_CHARS};
use crate::metrics;

/// The largest coordinate or size an agent may give (Excalidraw warns beyond it).
pub(crate) const MAX_COORDINATE: f64 = 1_000_000.0;
pub(crate) const MAX_FONT_SIZE: f64 = 1_000.0;
pub(crate) const MAX_POINTS: usize = 100_000;

pub(crate) const FILL_STYLES: [&str; 4] = ["hachure", "cross-hatch", "solid", "zigzag"];
pub(crate) const STROKE_STYLES: [&str; 3] = ["solid", "dashed", "dotted"];
pub(crate) const ARROWHEADS: [&str; 12] = [
    "arrow",
    "bar",
    "dot",
    "circle",
    "circle_outline",
    "triangle",
    "triangle_outline",
    "diamond",
    "diamond_outline",
    "crowfoot_one",
    "crowfoot_many",
    "crowfoot_one_or_many",
];

/// The style fields every element may have (section 7.2).
pub(crate) const STYLE_FIELDS: [&str; 9] = [
    "strokeColor",
    "backgroundColor",
    "fillStyle",
    "strokeWidth",
    "strokeStyle",
    "roughness",
    "opacity",
    "roundness",
    "groupIds",
];

/// The fields that mean nothing on a picture.
pub(crate) const FILL_FIELDS: [&str; 6] =
    ["strokeColor", "backgroundColor", "fillStyle", "strokeWidth", "strokeStyle", "roughness"];

/// Fields that are Excalidraw's own and never an agent's.
pub(crate) const NEVER_FROM_AGENTS: [(&str, &str); 7] = [
    ("customData", "customData is not taken from agents"),
    ("fileId", "a picture is named by its path; fileId is the library's"),
    ("status", "status is the library's"),
    ("scale", "scale is not taken from agents"),
    ("crop", "a picture cannot be cropped here"),
    ("lineHeight", "a text's line height follows its font"),
    ("version", "an element's version is the whiteboard's"),
];

/// Where a field was read: the element's index in the request.
#[derive(Clone, Copy, Debug)]
pub(crate) struct At(pub Option<usize>);

impl At {
    pub(crate) fn invalid(self, field: &str, message: impl Into<String>) -> BoardError {
        BoardError::Invalid {
            element: self.0,
            field: if field.is_empty() { None } else { Some(field.to_string()) },
            message: message.into(),
        }
    }

    pub(crate) fn number(self, map: &Map<String, Value>, field: &str) -> Result<Option<f64>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Number(n)) => match n.as_f64() {
                Some(v) if v.is_finite() => Ok(Some(v)),
                _ => Err(self.invalid(field, format!("{field} must be a number"))),
            },
            Some(_) => Err(self.invalid(field, format!("{field} must be a number"))),
        }
    }

    pub(crate) fn coordinate(self, map: &Map<String, Value>, field: &str) -> Result<Option<f64>, BoardError> {
        let value = self.number(map, field)?;
        if let Some(v) = value
            && v.abs() > MAX_COORDINATE
        {
            return Err(self.invalid(field, format!("{field} must be between -1,000,000 and 1,000,000")));
        }
        Ok(value)
    }

    /// A width or a height: at least 1 (anything less would be invisibly small).
    pub(crate) fn size(self, map: &Map<String, Value>, field: &str) -> Result<Option<f64>, BoardError> {
        let value = self.number(map, field)?;
        if let Some(v) = value
            && !(1.0..=MAX_COORDINATE).contains(&v)
        {
            return Err(self.invalid(
                field,
                format!("{field} must be at least 1 and at most 1,000,000 (an element narrower or lower than 1 px cannot be seen)"),
            ));
        }
        Ok(value)
    }

    pub(crate) fn required(self, value: Option<f64>, field: &str, kind: &str) -> Result<f64, BoardError> {
        value.ok_or_else(|| self.invalid(field, format!("a {kind} needs {field}")))
    }

    pub(crate) fn string<'m>(self, map: &'m Map<String, Value>, field: &str) -> Result<Option<&'m str>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::String(s)) => Ok(Some(s)),
            Some(_) => Err(self.invalid(field, format!("{field} must be a string"))),
        }
    }

    pub(crate) fn boolean(self, map: &Map<String, Value>, field: &str) -> Result<Option<bool>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Bool(b)) => Ok(Some(*b)),
            Some(_) => Err(self.invalid(field, format!("{field} must be true or false"))),
        }
    }

    /// A text an agent writes: line ends and tabs normalised, not empty or only white
    /// space, at most [`MAX_TEXT_CHARS`] characters.
    pub(crate) fn text(self, value: &str, field: &str) -> Result<String, BoardError> {
        let text = metrics::normalize_text(value);
        if text.trim().is_empty() {
            return Err(self.invalid(field, format!("{field} is empty: a text without characters cannot be seen")));
        }
        if text.chars().count() > MAX_TEXT_CHARS {
            return Err(BoardError::TooLarge { what: "text", limit: MAX_TEXT_CHARS });
        }
        Ok(text)
    }

    pub(crate) fn id(self, value: &Value, field: &str) -> Result<String, BoardError> {
        let Some(text) = value.as_str() else {
            return Err(self.invalid(field, format!("{field} must be a string")));
        };
        if !is_id(text) {
            return Err(self.invalid(
                field,
                format!("{field} must be 1 to {MAX_ID_CHARS} characters of letters, digits, _ and -"),
            ));
        }
        Ok(text.to_string())
    }

    pub(crate) fn color(self, map: &Map<String, Value>, field: &str) -> Result<Option<String>, BoardError> {
        let Some(text) = self.string(map, field)? else { return Ok(None) };
        let text = text.trim();
        let hex = text.strip_prefix('#').is_some_and(|h| {
            matches!(h.len(), 3 | 4 | 6 | 8) && h.bytes().all(|b| b.is_ascii_hexdigit())
        });
        let named = !text.is_empty() && text.len() <= 30 && text.bytes().all(|b| b.is_ascii_alphabetic());
        if hex || named {
            Ok(Some(text.to_string()))
        } else {
            Err(self.invalid(field, format!("{field} must be a colour such as #1e1e1e or transparent")))
        }
    }

    pub(crate) fn one_of(
        self,
        map: &Map<String, Value>,
        field: &str,
        allowed: &[&str],
    ) -> Result<Option<String>, BoardError> {
        let Some(text) = self.string(map, field)? else { return Ok(None) };
        if allowed.contains(&text) {
            Ok(Some(text.to_string()))
        } else {
            Err(self.invalid(field, format!("{field} must be one of {}", allowed.join(", "))))
        }
    }

    pub(crate) fn font_size(self, map: &Map<String, Value>, field: &str) -> Result<Option<f64>, BoardError> {
        let value = self.number(map, field)?;
        if let Some(v) = value
            && !(1.0..=MAX_FONT_SIZE).contains(&v)
        {
            return Err(self.invalid(field, format!("{field} must be between 1 and 1,000")));
        }
        Ok(value)
    }

    pub(crate) fn font_family(self, map: &Map<String, Value>, field: &str) -> Result<Option<u8>, BoardError> {
        let wrong = || {
            let names: Vec<String> = metrics::FAMILIES.iter().map(|(n, name)| format!("{n} {name}")).collect();
            self.invalid(field, format!("{field} must be one of Excalidraw's fonts: {}", names.join(", ")))
        };
        match map.get(field) {
            None => Ok(None),
            Some(Value::Number(n)) => {
                let number = n.as_u64().filter(|v| *v < 256).ok_or_else(wrong)?;
                metrics::family_by_name(&number.to_string()).map(Some).ok_or_else(wrong)
            }
            Some(Value::String(name)) => metrics::family_by_name(name).map(Some).ok_or_else(wrong),
            Some(_) => Err(wrong()),
        }
    }

    pub(crate) fn link(self, map: &Map<String, Value>, field: &str) -> Result<Option<Option<String>>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(text)) => {
                let text = text.trim();
                let lower = text.to_ascii_lowercase();
                let allowed = ["http://", "https://", "mailto:"].iter().any(|p| lower.starts_with(p));
                if !allowed || text.len() > 2048 || text.chars().any(|c| c.is_control() || c == '"') {
                    return Err(self.invalid(field, "a link must be an address that starts with http://, https:// or mailto:"));
                }
                Ok(Some(Some(text.to_string())))
            }
            Some(_) => Err(self.invalid(field, "a link must be a string, or null")),
        }
    }

    pub(crate) fn roundness(self, map: &Map<String, Value>, field: &str) -> Result<Option<Value>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(Value::Null)),
            Some(Value::Object(r)) => {
                let kind = r.get("type").and_then(Value::as_u64);
                if r.keys().all(|k| k == "type") && matches!(kind, Some(1..=3)) {
                    let mut out = Map::new();
                    out.insert("type".into(), Value::from(kind.unwrap_or(3)));
                    Ok(Some(Value::Object(out)))
                } else {
                    Err(self.invalid(field, "roundness must be {\"type\": 3} (or 1 or 2), or null"))
                }
            }
            Some(_) => Err(self.invalid(field, "roundness must be {\"type\": 3}, or null")),
        }
    }

    pub(crate) fn group_ids(self, map: &Map<String, Value>, field: &str) -> Result<Option<Vec<String>>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Array(items)) => {
                let mut out = Vec::new();
                for item in items {
                    out.push(self.id(item, field)?);
                }
                Ok(Some(out))
            }
            Some(_) => Err(self.invalid(field, "groupIds must be a list of ids")),
        }
    }

    /// Points `[[x, y], …]`, at least two, relative to the element's `x` and `y`.
    pub(crate) fn points(self, map: &Map<String, Value>, field: &str) -> Result<Option<Vec<(f64, f64)>>, BoardError> {
        let Some(value) = map.get(field) else { return Ok(None) };
        let wrong = || self.invalid(field, "points must be a list of at least two [x, y] pairs");
        let Value::Array(items) = value else { return Err(wrong()) };
        if items.len() < 2 {
            return Err(wrong());
        }
        if items.len() > MAX_POINTS {
            return Err(self.invalid(field, "points may hold at most 100,000 pairs"));
        }
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let pair = item.as_array().filter(|p| p.len() == 2).ok_or_else(wrong)?;
            let x = pair[0].as_f64().filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE).ok_or_else(wrong)?;
            let y = pair[1].as_f64().filter(|v| v.is_finite() && v.abs() <= MAX_COORDINATE).ok_or_else(wrong)?;
            out.push((x, y));
        }
        Ok(Some(out))
    }

    /// `{"id": "<element>"}`: the element an arrow's end is bound to. `Some(None)` for
    /// `null` (unbound, in a change).
    pub(crate) fn end(self, map: &Map<String, Value>, field: &str) -> Result<Option<Option<String>>, BoardError> {
        match map.get(field) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::Object(end)) => {
                if end.contains_key("type") || end.keys().any(|k| k != "id") {
                    return Err(self.invalid(
                        field,
                        format!("{field} names an element by id only ({{\"id\": \"api\"}}); give a new shape its own entry"),
                    ));
                }
                let id = end.get("id").ok_or_else(|| self.invalid(field, format!("{field} needs an id")))?;
                Ok(Some(Some(self.id(id, field)?)))
            }
            Some(_) => Err(self.invalid(field, format!("{field} must be {{\"id\": \"<an element>\"}}"))),
        }
    }
}

/// An angle in radians brought into [0, 2π), as Excalidraw keeps it.
pub(crate) fn normalize_angle(angle: f64) -> f64 {
    let full = std::f64::consts::TAU;
    let a = angle % full;
    let a = if a < 0.0 { a + full } else { a };
    if (a - full).abs() < 1e-12 { 0.0 } else { a }
}

/// 1 to 64 characters of `[A-Za-z0-9_-]`.
pub(crate) fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_ID_CHARS
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
