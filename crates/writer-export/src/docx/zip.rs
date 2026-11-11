//! A minimal ZIP writer: deflated entries, no extensions, deterministic
//! output. Enough for an Office Open XML package.

use miniz_oxide::deflate::compress_to_vec;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

static CRC_TABLE: [u32; 256] = crc_table();

/// CRC-32 (IEEE 802.3), as ZIP uses it.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = CRC_TABLE[((c ^ u32::from(b)) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

struct Entry {
    name: String,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u32,
    method: u16,
}

/// Builds a ZIP archive in memory.
pub(crate) struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<Entry>,
    dos_time: u16,
    dos_date: u16,
}

fn put16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

impl ZipWriter {
    /// A writer whose entries are all stamped with the given UTC time
    /// (milliseconds since the Unix epoch; clamped to what ZIP can store).
    pub fn new(modified_ms: i64) -> ZipWriter {
        let (y, mo, d, h, mi, s) = crate::common::civil_from_millis(modified_ms);
        let (y, mo, d, h, mi, s) = if y < 1980 {
            (1980, 1, 1, 0, 0, 0)
        } else if y > 2107 {
            (2107, 12, 31, 23, 59, 58)
        } else {
            (y, mo, d, h, mi, s)
        };
        ZipWriter {
            out: Vec::new(),
            entries: Vec::new(),
            dos_time: ((h << 11) | (mi << 5) | (s / 2)) as u16,
            dos_date: ((((y - 1980) as u32) << 9) | (mo << 5) | d) as u16,
        }
    }

    /// Adds a file. Names are ASCII paths with `/` separators.
    pub fn add(&mut self, name: &str, data: &[u8]) {
        let deflated = compress_to_vec(data, 6);
        // Store tiny or incompressible data as it is.
        let (method, body): (u16, &[u8]) = if deflated.len() < data.len() {
            (8, &deflated)
        } else {
            (0, data)
        };
        let entry = Entry {
            name: name.to_string(),
            crc: crc32(data),
            compressed: body.len() as u32,
            size: data.len() as u32,
            offset: self.out.len() as u32,
            method,
        };
        put32(&mut self.out, 0x0403_4b50);
        put16(&mut self.out, 20); // version needed
        put16(&mut self.out, 0); // flags
        put16(&mut self.out, entry.method);
        put16(&mut self.out, self.dos_time);
        put16(&mut self.out, self.dos_date);
        put32(&mut self.out, entry.crc);
        put32(&mut self.out, entry.compressed);
        put32(&mut self.out, entry.size);
        put16(&mut self.out, entry.name.len() as u16);
        put16(&mut self.out, 0); // extra length
        self.out.extend_from_slice(entry.name.as_bytes());
        self.out.extend_from_slice(body);
        self.entries.push(entry);
    }

    /// Writes the central directory and returns the archive.
    pub fn finish(mut self) -> Vec<u8> {
        let start = self.out.len() as u32;
        for e in &self.entries {
            put32(&mut self.out, 0x0201_4b50);
            put16(&mut self.out, 20); // version made by
            put16(&mut self.out, 20); // version needed
            put16(&mut self.out, 0); // flags
            put16(&mut self.out, e.method);
            put16(&mut self.out, self.dos_time);
            put16(&mut self.out, self.dos_date);
            put32(&mut self.out, e.crc);
            put32(&mut self.out, e.compressed);
            put32(&mut self.out, e.size);
            put16(&mut self.out, e.name.len() as u16);
            put16(&mut self.out, 0); // extra length
            put16(&mut self.out, 0); // comment length
            put16(&mut self.out, 0); // disk number
            put16(&mut self.out, 0); // internal attributes
            put32(&mut self.out, 0); // external attributes
            put32(&mut self.out, e.offset);
            self.out.extend_from_slice(e.name.as_bytes());
        }
        let size = self.out.len() as u32 - start;
        put32(&mut self.out, 0x0605_4b50);
        put16(&mut self.out, 0); // this disk
        put16(&mut self.out, 0); // directory disk
        put16(&mut self.out, self.entries.len() as u16);
        put16(&mut self.out, self.entries.len() as u16);
        put32(&mut self.out, size);
        put32(&mut self.out, start);
        put16(&mut self.out, 0); // comment length
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn layout() {
        let mut z = ZipWriter::new(1_794_391_505_000);
        z.add("a.txt", b"hello hello hello hello hello hello");
        z.add("dir/b.txt", b"x");
        let bytes = z.finish();
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        // End of central directory: two entries.
        let eocd = bytes.len() - 22;
        assert_eq!(&bytes[eocd..eocd + 4], b"PK\x05\x06");
        assert_eq!(u16::from_le_bytes([bytes[eocd + 10], bytes[eocd + 11]]), 2);
        // Deterministic.
        let mut z2 = ZipWriter::new(1_794_391_505_000);
        z2.add("a.txt", b"hello hello hello hello hello hello");
        z2.add("dir/b.txt", b"x");
        assert_eq!(bytes, z2.finish());
    }
}
