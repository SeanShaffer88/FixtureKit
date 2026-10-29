//! ZIP (stored entries only) and ustar TAR writers. Both are
//! byte-for-byte deterministic: ZIP entries carry the DOS epoch
//! date (1980-01-01, time 0) and TAR headers an mtime of 0, so
//! the same entry list always produces the same archive.

use crate::crc32::crc32;

/// Size/entries caps, named so errors can quote them. A fixture
/// archive past these is a caller bug, not a fixture.
pub const MAX_ENTRIES: usize = 4096;
pub const MAX_ENTRY_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 128 * 1024 * 1024;

pub struct EntrySpec {
    pub name: String,
    pub size: u64,
    pub fill_byte: u8,
}

/// The bytes one entry expands to: `fill_byte` repeated `size`
/// times (the caller has already enforced the caps).
pub fn entry_data(e: &EntrySpec) -> Vec<u8> {
    vec![e.fill_byte; e.size as usize]
}

// ---------------------------------------------------------------- ZIP

/// A valid ZIP archive (APPNOTE) with one STORED (method 0)
/// entry per spec: local header + data per entry, then the
/// central directory, then the end-of-central-directory record.
/// CRCs are the real CRC-32 of each entry's data.
pub fn zip_stored(entries: &[EntrySpec]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for e in entries {
        let data = entry_data(e);
        let crc = crc32(&data);
        let offset = out.len() as u32;
        let name = e.name.as_bytes();
        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time: 0
        out.extend_from_slice(&0x21u16.to_le_bytes()); // mod date: 1980-01-01
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed
        out.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed
        out.extend_from_slice(&(name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra length
        out.extend_from_slice(name);
        out.extend_from_slice(&data);
        // Central directory entry.
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&0u16.to_le_bytes()); // time
        central.extend_from_slice(&0x21u16.to_le_bytes()); // date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(data.len() as u32).to_le_bytes());
        central.extend_from_slice(&(name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk number
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let cd_offset = out.len() as u32;
    let cd_len = central.len() as u32;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // this disk
    out.extend_from_slice(&0u16.to_le_bytes()); // CD disk
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

// ---------------------------------------------------------------- TAR

/// Write `n` into a tar numeric field: zero-padded octal,
/// NUL-terminated, `width` bytes total (width-1 digits).
fn octal_field(out: &mut [u8], n: u64) {
    let width = out.len();
    let text = format!("{:0width$o}", n, width = width - 1);
    out[..width - 1].copy_from_slice(text.as_bytes());
    out[width - 1] = 0;
}

/// One ustar header (512 bytes) for `e`. Names longer than 100
/// bytes are split into prefix (≤155, at a '/') + name (≤100);
/// a name with no legal split is an error naming the limit.
fn tar_header(e: &EntrySpec) -> Result<[u8; 512], String> {
    let mut h = [0u8; 512];
    let name = e.name.as_bytes();
    if name.len() <= 100 {
        h[..name.len()].copy_from_slice(name);
    } else {
        // Find the last '/' that leaves prefix <= 155 and rest <= 100.
        let mut split = None;
        for i in (0..name.len()).rev() {
            if name[i] == b'/' && i <= 155 && name.len() - i - 1 <= 100 && i > 0 {
                split = Some(i);
                break;
            }
        }
        let i = split.ok_or_else(|| {
            format!(
                "tar entry name \"{}\" ({} bytes) cannot be split into a ustar prefix (<=155) + name (<=100)",
                e.name,
                name.len()
            )
        })?;
        h[..name.len() - i - 1].copy_from_slice(&name[i + 1..]);
        h[345..345 + i].copy_from_slice(&name[..i]);
    }
    octal_field(&mut h[100..108], 0o644); // mode
    octal_field(&mut h[108..116], 0); // uid
    octal_field(&mut h[116..124], 0); // gid
    octal_field(&mut h[124..136], e.size); // size
    octal_field(&mut h[136..148], 0); // mtime: epoch, for determinism
    h[156] = b'0'; // typeflag: regular file
    h[257..263].copy_from_slice(b"ustar\0"); // magic
    h[263..265].copy_from_slice(b"00"); // version
    octal_field(&mut h[329..337], 0); // devmajor
    octal_field(&mut h[337..345], 0); // devminor
                                      // Checksum: the field itself counts as spaces while summing.
    for b in &mut h[148..156] {
        *b = b' ';
    }
    let sum: u64 = h.iter().map(|&b| b as u64).sum();
    let text = format!("{sum:06o}");
    h[148..154].copy_from_slice(text.as_bytes());
    h[154] = 0;
    h[155] = b' ';
    Ok(h)
}

/// A valid ustar archive: header + data padded to 512 per
/// entry, closed by the two all-zero blocks the format ends with.
pub fn tar(entries: &[EntrySpec]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    for e in entries {
        out.extend_from_slice(&tar_header(e)?);
        let data = entry_data(e);
        out.extend_from_slice(&data);
        let pad = (512 - data.len() % 512) % 512;
        out.resize(out.len() + pad, 0);
    }
    out.resize(out.len() + 1024, 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc32::crc32;

    fn specs() -> Vec<EntrySpec> {
        vec![
            EntrySpec {
                name: "a.txt".into(),
                size: 5,
                fill_byte: b'A',
            },
            EntrySpec {
                name: "dir/b.bin".into(),
                size: 600,
                fill_byte: 0x7F,
            },
            EntrySpec {
                name: "empty".into(),
                size: 0,
                fill_byte: 0,
            },
        ]
    }

    #[test]
    fn zip_reparses_through_its_central_directory() {
        let entries = specs();
        let z = zip_stored(&entries);
        // EOCD is the final 22 bytes (no archive comment).
        let eocd = &z[z.len() - 22..];
        assert_eq!(
            u32::from_le_bytes(eocd[0..4].try_into().unwrap()),
            0x0605_4b50
        );
        let count = u16::from_le_bytes(eocd[10..12].try_into().unwrap()) as usize;
        assert_eq!(count, entries.len());
        let cd_size = u32::from_le_bytes(eocd[12..16].try_into().unwrap()) as usize;
        let cd_off = u32::from_le_bytes(eocd[16..20].try_into().unwrap()) as usize;
        assert_eq!(cd_off + cd_size, z.len() - 22);
        // Walk the central directory; verify each entry against
        // its local header and a fresh CRC of the data.
        let mut pos = cd_off;
        for e in &entries {
            let c = &z[pos..];
            assert_eq!(u32::from_le_bytes(c[0..4].try_into().unwrap()), 0x0201_4b50);
            assert_eq!(
                u16::from_le_bytes(c[10..12].try_into().unwrap()),
                0,
                "stored"
            );
            let crc = u32::from_le_bytes(c[16..20].try_into().unwrap());
            let size = u32::from_le_bytes(c[20..24].try_into().unwrap()) as usize;
            let nlen = u16::from_le_bytes(c[28..30].try_into().unwrap()) as usize;
            let elen = u16::from_le_bytes(c[30..32].try_into().unwrap()) as usize;
            let clen = u16::from_le_bytes(c[32..34].try_into().unwrap()) as usize;
            let lho = u32::from_le_bytes(c[42..46].try_into().unwrap()) as usize;
            assert_eq!(&c[46..46 + nlen], e.name.as_bytes());
            let l = &z[lho..];
            assert_eq!(u32::from_le_bytes(l[0..4].try_into().unwrap()), 0x0403_4b50);
            let lnlen = u16::from_le_bytes(l[26..28].try_into().unwrap()) as usize;
            let lelen = u16::from_le_bytes(l[28..30].try_into().unwrap()) as usize;
            let data = &l[30 + lnlen + lelen..30 + lnlen + lelen + size];
            assert_eq!(data.len() as u64, e.size);
            assert!(data.iter().all(|&b| b == e.fill_byte));
            assert_eq!(crc32(data), crc, "entry {} CRC", e.name);
            pos += 46 + nlen + elen + clen;
        }
        assert_eq!(pos, cd_off + cd_size);
    }

    #[test]
    fn tar_headers_checksum_and_walk() {
        let entries = specs();
        let t = tar(&entries).unwrap();
        assert_eq!(t.len() % 512, 0);
        let mut pos = 0usize;
        for e in &entries {
            let h = &t[pos..pos + 512];
            assert_eq!(&h[257..262], b"ustar");
            // Checksum: sum with the checksum field as spaces.
            let stored: u64 = {
                let field = std::str::from_utf8(&h[148..154]).unwrap();
                u64::from_str_radix(field.trim(), 8).unwrap()
            };
            let computed: u64 = h
                .iter()
                .enumerate()
                .map(|(i, &b)| {
                    if (148..156).contains(&i) {
                        0x20
                    } else {
                        b as u64
                    }
                })
                .sum();
            assert_eq!(stored, computed, "entry {} checksum", e.name);
            let size_text = std::str::from_utf8(&h[124..135]).unwrap();
            let size = u64::from_str_radix(size_text.trim_start_matches('0'), 8).unwrap_or(0);
            assert_eq!(size, e.size);
            let name = std::str::from_utf8(&h[..h.iter().position(|&b| b == 0).unwrap()]).unwrap();
            assert_eq!(name, e.name);
            let data = &t[pos + 512..pos + 512 + e.size as usize];
            assert!(data.iter().all(|&b| b == e.fill_byte));
            pos += 512 + (e.size as usize).div_ceil(512) * 512;
        }
        // The archive ends with at least two all-zero blocks.
        assert!(t[pos..].iter().all(|&b| b == 0));
        assert!(t[pos..].len() >= 1024);
    }

    #[test]
    fn tar_long_names_use_prefix_and_unsplittable_names_fail() {
        let long = format!("{}/{}.txt", "d".repeat(60), "f".repeat(60));
        let e = EntrySpec {
            name: long,
            size: 1,
            fill_byte: 0,
        };
        let t = tar(&[e]).unwrap();
        let h = &t[..512];
        let name_part = std::str::from_utf8(&h[..100])
            .unwrap()
            .trim_end_matches('\0');
        let prefix = std::str::from_utf8(&h[345..345 + 60]).unwrap();
        assert_eq!(name_part.len(), 64); // 60 f's + ".txt"
        assert_eq!(prefix, "d".repeat(60));
        let bad = EntrySpec {
            name: "x".repeat(300),
            size: 0,
            fill_byte: 0,
        };
        assert!(tar(&[bad]).is_err());
    }

    #[test]
    fn archives_are_deterministic() {
        assert_eq!(zip_stored(&specs()), zip_stored(&specs()));
        assert_eq!(tar(&specs()).unwrap(), tar(&specs()).unwrap());
    }
}
