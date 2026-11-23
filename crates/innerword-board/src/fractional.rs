//! Excalidraw's fractional indices (the `index` of an element, which orders the elements of
//! a scene): a port of `fractional-indexing` 3.2.0's `generateKeyBetween`, the package
//! Excalidraw 0.18.1 uses, with its base-62 digits.

const DIGITS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const ZERO: u8 = b'0';

fn digit_value(c: u8) -> Option<usize> {
    DIGITS.iter().position(|d| *d == c)
}

fn integer_length(head: u8) -> Option<usize> {
    match head {
        b'a'..=b'z' => Some((head - b'a') as usize + 2),
        b'A'..=b'Z' => Some((b'Z' - head) as usize + 2),
        _ => None,
    }
}

/// Whether `key` is an order key `generateKeyBetween` accepts.
pub(crate) fn is_valid(key: &str) -> bool {
    let bytes = key.as_bytes();
    let Some(&head) = bytes.first() else { return false };
    let Some(length) = integer_length(head) else { return false };
    if length > bytes.len() || !bytes.iter().all(|b| digit_value(*b).is_some() || b.is_ascii_alphabetic()) {
        return false;
    }
    if !bytes[1..].iter().all(|b| digit_value(*b).is_some()) {
        return false;
    }
    if head == b'A' && bytes.len() == 27 && bytes[1..].iter().all(|b| *b == ZERO) {
        return false;
    }
    let fraction = &bytes[length..];
    fraction.last() != Some(&ZERO)
}

fn midpoint(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    if let Some(b) = b {
        let mut n = 0;
        while a.get(n).copied().unwrap_or(ZERO) == b.get(n).copied().unwrap_or(0xFF) {
            n += 1;
        }
        if n > 0 {
            let mut out = b[..n].to_vec();
            out.extend(midpoint(a.get(n..).unwrap_or(&[]), Some(&b[n..])));
            return out;
        }
    }
    let digit_a = a.first().and_then(|c| digit_value(*c)).unwrap_or(0);
    let digit_b = match b {
        Some(b) => b.first().and_then(|c| digit_value(*c)).unwrap_or(0),
        None => DIGITS.len(),
    };
    if digit_b - digit_a > 1 {
        // Math.round(0.5 * (a + b)): halves round up.
        let mid = (digit_a + digit_b).div_ceil(2);
        vec![DIGITS[mid]]
    } else if let Some(b) = b.filter(|b| b.len() > 1) {
        vec![b[0]]
    } else {
        let mut out = vec![DIGITS[digit_a]];
        out.extend(midpoint(a.get(1..).unwrap_or(&[]), None));
        out
    }
}

fn increment_integer(x: &[u8]) -> Option<Vec<u8>> {
    let head = x[0];
    let mut digits: Vec<u8> = x[1..].to_vec();
    let mut carry = true;
    let mut i = digits.len();
    while carry && i > 0 {
        i -= 1;
        let d = digit_value(digits[i]).unwrap_or(0) + 1;
        if d == DIGITS.len() {
            digits[i] = ZERO;
        } else {
            digits[i] = DIGITS[d];
            carry = false;
        }
    }
    if !carry {
        let mut out = vec![head];
        out.extend(digits);
        return Some(out);
    }
    if head == b'Z' {
        return Some(vec![b'a', ZERO]);
    }
    if head == b'z' {
        return None;
    }
    let next = head + 1;
    if next > b'a' {
        digits.push(ZERO);
    } else {
        digits.pop();
    }
    let mut out = vec![next];
    out.extend(digits);
    Some(out)
}

/// A key after `a` (`None`: the first key, `a0`), with nothing above it: what an element
/// put on top of the drawing gets.
pub(crate) fn key_after(a: Option<&str>) -> String {
    let Some(a) = a.filter(|a| is_valid(a)) else {
        return "a0".to_string();
    };
    let bytes = a.as_bytes();
    let length = integer_length(bytes[0]).unwrap_or(2);
    let (integer, fraction) = bytes.split_at(length);
    let key = match increment_integer(integer) {
        Some(next) => next,
        None => {
            let mut out = integer.to_vec();
            out.extend(midpoint(fraction, None));
            out
        }
    };
    String::from_utf8(key).unwrap_or_else(|_| "a0".to_string())
}

/// The greatest valid index among `keys`: what a new element's index follows.
pub(crate) fn greatest<'a>(keys: impl Iterator<Item = Option<&'a str>>) -> Option<&'a str> {
    keys.flatten().filter(|k| is_valid(k)).max()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_fractional_indexing() {
        assert_eq!(key_after(None), "a0");
        assert_eq!(key_after(Some("a0")), "a1");
        assert_eq!(key_after(Some("a9")), "aA");
        assert_eq!(key_after(Some("az")), "b00");
        assert_eq!(key_after(Some("b0z")), "b10");
        assert_eq!(key_after(Some("Zz")), "a0");
        assert_eq!(key_after(Some("a0V")), "a1");
        assert_eq!(key_after(Some("bad key")), "a0");
        let mut key = key_after(None);
        for _ in 0..5000 {
            let next = key_after(Some(&key));
            assert!(next > key, "{next} after {key}");
            assert!(is_valid(&next));
            key = next;
        }
    }

    #[test]
    fn validity() {
        assert!(is_valid("a0"));
        assert!(is_valid("a0V"));
        assert!(!is_valid("a0V0"));
        assert!(!is_valid("a"));
        assert!(!is_valid(""));
        assert!(!is_valid("!0"));
        assert!(is_valid("Zz"));
    }

    #[test]
    fn midpoints() {
        assert_eq!(String::from_utf8(midpoint(b"", None)).unwrap(), "V");
        assert_eq!(String::from_utf8(midpoint(b"V", None)).unwrap(), "l");
        assert_eq!(String::from_utf8(midpoint(b"", Some(b"V"))).unwrap(), "G");
    }
}
