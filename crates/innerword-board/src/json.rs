//! JSON helpers: lone surrogates mended, numbers written as Excalidraw writes them, and one
//! writer whose output does not depend on how `serde_json` was built (sorted keys).

use std::borrow::Cow;
use std::fmt::Write as _;

use serde_json::{Map, Number, Value};

/// Replaces every lone surrogate escape in JSON text (`\ud800` without its pair, or a
/// trailing half without its first) by `�`. `JSON.stringify` writes such escapes for
/// a string that holds half a character pair, and `serde_json` refuses them; everything
/// else is left as it is (an escaped backslash before `u` is not an escape).
pub fn well_formed_json(text: &str) -> Cow<'_, str> {
    if !text.contains("\\u") {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut copied = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            i += 1;
            continue;
        }
        // An escape: `\` and one character.
        if i + 1 >= bytes.len() {
            break;
        }
        if bytes[i + 1] != b'u' {
            i += 2;
            continue;
        }
        let Some(unit) = hex4(bytes, i + 2) else {
            i += 2;
            continue;
        };
        if (0xD800..0xDC00).contains(&unit) {
            // A first half: well formed only when a second half follows at once.
            let paired = bytes.get(i + 6) == Some(&b'\\')
                && bytes.get(i + 7) == Some(&b'u')
                && hex4(bytes, i + 8).is_some_and(|next| (0xDC00..0xE000).contains(&next));
            if paired {
                i += 12;
                continue;
            }
        } else if !(0xDC00..0xE000).contains(&unit) {
            i += 6;
            continue;
        }
        // A lone half.
        let buffer = out.get_or_insert_with(|| String::with_capacity(text.len()));
        buffer.push_str(&text[copied..i]);
        buffer.push_str("\\ufffd");
        i += 6;
        copied = i;
    }
    match out {
        Some(mut buffer) => {
            buffer.push_str(&text[copied..]);
            Cow::Owned(buffer)
        }
        None => Cow::Borrowed(text),
    }
}

fn hex4(bytes: &[u8], at: usize) -> Option<u32> {
    let digits = bytes.get(at..at + 4)?;
    let text = std::str::from_utf8(digits).ok()?;
    if !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(text, 16).ok()
}

/// A number as Excalidraw writes it: a whole number without a fraction (`100`, not
/// `100.0`), anything else as the shortest text that reads back as the same `f64`. A value
/// that is not finite is written as `null`, as `JSON.stringify` writes `NaN`.
pub(crate) fn num(value: f64) -> Value {
    if !value.is_finite() {
        return Value::Null;
    }
    if value == value.trunc() && value.abs() < 9.0e15 {
        // -0.0 is written 0, as JavaScript writes it.
        return Value::Number(Number::from(value as i64));
    }
    Number::from_f64(value).map(Value::Number).unwrap_or(Value::Null)
}

/// The value as an `f64`: a number, or 0 for anything else (`null` where Excalidraw wrote
/// `NaN`, a string).
pub(crate) fn as_f64(value: Option<&Value>) -> f64 {
    value.and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0)
}

/// Writes `value` with every object's keys sorted, so that the bytes do not depend on
/// whether `serde_json` keeps insertion order (it does in a build that links GPUI).
pub(crate) fn write_sorted(out: &mut String, value: &Value) {
    match value {
        Value::Object(map) => write_object(out, map),
        Value::Array(items) => {
            out.push('[');
            for (n, item) in items.iter().enumerate() {
                if n > 0 {
                    out.push(',');
                }
                write_sorted(out, item);
            }
            out.push(']');
        }
        Value::String(text) => write_string(out, text),
        Value::Number(number) => {
            let _ = write!(out, "{number}");
        }
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Null => out.push_str("null"),
    }
}

pub(crate) fn write_object(out: &mut String, map: &Map<String, Value>) {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    out.push('{');
    for (n, key) in keys.into_iter().enumerate() {
        if n > 0 {
            out.push(',');
        }
        write_string(out, key);
        out.push(':');
        write_sorted(out, &map[key.as_str()]);
    }
    out.push('}');
}

pub(crate) fn write_string(out: &mut String, text: &str) {
    // serde_json's own escaping of a string cannot fail.
    out.push_str(&serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string()));
}

/// `value` written with sorted keys.
#[cfg(test)]
pub(crate) fn to_sorted_string(value: &Value) -> String {
    let mut out = String::new();
    write_sorted(&mut out, value);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lone_halves_are_mended_and_pairs_kept() {
        assert_eq!(well_formed_json(r#"{"a":"x"}"#), r#"{"a":"x"}"#);
        assert_eq!(well_formed_json(r#""\ud800""#), r#""�""#);
        assert_eq!(well_formed_json(r#""\udc00x""#), r#""�x""#);
        assert_eq!(well_formed_json(r#""😀""#), r#""😀""#);
        assert_eq!(well_formed_json(r#""\ud83dA""#), r#""�A""#);
        assert_eq!(well_formed_json(r#""\\ud800""#), r#""\\ud800""#);
        assert_eq!(well_formed_json(r#""\\\ud800""#), r#""\\�""#);
        assert_eq!(well_formed_json(r#""\ud800\ud800""#), r#""��""#);
        assert_eq!(well_formed_json(r#""é\ud800é""#), r#""é�é""#);
    }

    #[test]
    fn numbers_are_written_as_javascript_writes_them() {
        assert_eq!(num(100.0).to_string(), "100");
        assert_eq!(num(-0.0).to_string(), "0");
        assert_eq!(num(0.5).to_string(), "0.5");
        assert_eq!(num(f64::NAN), Value::Null);
        assert_eq!(num(322.82403564453125).to_string(), "322.82403564453125");
    }

    #[test]
    fn keys_are_sorted() {
        let value: Value = serde_json::from_str(r#"{"b":1,"a":{"d":[1,{"z":2,"y":"\n"}],"c":null}}"#).unwrap();
        assert_eq!(to_sorted_string(&value), r#"{"a":{"c":null,"d":[1,{"y":"\n","z":2}]},"b":1}"#);
    }
}
