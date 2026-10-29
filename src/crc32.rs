//! CRC-32 (IEEE 802.3, polynomial 0xEDB88320, reflected) — the
//! checksum PNG chunks and ZIP entries both use. Hand-rolled:
//! the crate is zero-dependency and std has no CRC.

const fn build_table() -> [u32; 256] {
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

static TABLE: [u32; 256] = build_table();

/// CRC-32 of `data`, init/xorout 0xFFFFFFFF (PNG/ZIP flavour).
pub fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c = TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

/// CRC-32 over two consecutive slices without copying them —
/// PNG chunk CRCs cover the chunk type bytes followed by the
/// chunk data.
pub fn crc32_two(a: &[u8], b: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &byte in a.iter().chain(b.iter()) {
        c = TABLE[((c ^ byte as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    c ^ 0xFFFF_FFFF
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // The standard CRC-32 check value.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082); // PNG's IEND chunk CRC
    }

    #[test]
    fn two_slice_matches_joined() {
        let joined = b"hello world";
        assert_eq!(crc32_two(b"hello ", b"world"), crc32(joined));
        assert_eq!(crc32_two(b"", b"world"), crc32(b"world"));
    }
}
