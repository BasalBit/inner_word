//! Where new ids, seeds, nonces and times come from: derived, never random
//! (`docs/whiteboard-design.md`, section 7.1).

use sha2::{Digest, Sha256};

/// nanoid's alphabet, which Excalidraw's ids are made of.
const ALPHABET: &[u8; 64] = b"useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";

/// The source of every value an operation makes.
pub trait Making {
    /// A new id, 21 characters of nanoid's alphabet, like Excalidraw's.
    fn id(&mut self) -> String;
    /// A `seed`: a 31-bit number.
    fn seed(&mut self) -> i64;
    /// A `versionNonce`: a 31-bit number.
    fn nonce(&mut self) -> i64;
    /// The time of the change, in milliseconds since 1970 (`updated`, a picture's
    /// `created`).
    fn now_ms(&mut self) -> i64;
}

/// The n-th value from SHA-256 of the parts and n; `now_ms` as given (the store's clock).
/// The same parts give the same values in the same order, in the tool, in the app and in a
/// dry run.
#[derive(Clone, Debug)]
pub struct HashedMaking {
    key: [u8; 32],
    counter: u64,
    now_ms: i64,
}

impl HashedMaking {
    /// `parts`: the board's id, the stored scene's stamp, the request as JSON, the `file`
    /// ids of its pictures (never their bytes). Each part is hashed with its length, so
    /// two lists of parts that join to the same bytes still differ.
    pub fn new(parts: &[&[u8]], now_ms: i64) -> HashedMaking {
        let mut hasher = Sha256::new();
        hasher.update(b"innerword-board making 1");
        for part in parts {
            hasher.update((part.len() as u64).to_le_bytes());
            hasher.update(part);
        }
        HashedMaking {
            key: hasher.finalize().into(),
            counter: 0,
            now_ms,
        }
    }

    fn next(&mut self, what: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(self.key);
        hasher.update(what);
        hasher.update(self.counter.to_le_bytes());
        self.counter += 1;
        hasher.finalize().into()
    }

    fn number(&mut self, what: &[u8]) -> i64 {
        let bytes = self.next(what);
        (u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) & 0x7fff_ffff) as i64
    }
}

impl Making for HashedMaking {
    fn id(&mut self) -> String {
        let bytes = self.next(b"id");
        bytes[..21]
            .iter()
            .map(|b| ALPHABET[(b & 63) as usize] as char)
            .collect()
    }

    fn seed(&mut self) -> i64 {
        self.number(b"seed")
    }

    fn nonce(&mut self) -> i64 {
        self.number(b"nonce")
    }

    fn now_ms(&mut self) -> i64 {
        self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_parts_give_the_same_values() {
        let mut a = HashedMaking::new(&[b"12", b"stamp"], 5);
        let mut b = HashedMaking::new(&[b"12", b"stamp"], 5);
        let mut c = HashedMaking::new(&[b"1", b"2stamp"], 5);
        let first = a.id();
        assert_eq!(first, b.id());
        assert_ne!(first, c.id());
        assert_eq!(first.len(), 21);
        assert!(first.bytes().all(|b| ALPHABET.contains(&b)));
        assert_ne!(a.id(), first);
        let seed = a.seed();
        assert!((0..1 << 31).contains(&seed));
        assert_eq!(a.now_ms(), 5);
    }
}
