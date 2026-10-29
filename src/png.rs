//! A real PNG encoder (RFC 2083 / ISO 15948), deliberately
//! small: filter type 0 (None) on every scanline, the image data
//! carried in a stored-block zlib stream (see `zlib.rs`), and a
//! deterministic gradient/checker pattern for pixels so a test
//! can predict the exact samples from the seed.
//!
//! Interlace method 1 (Adam7) emits the seven passes with the
//! correct origins and steps; each pass is packed independently,
//! exactly as the format requires — the pattern is a function of
//! the pixel's position in the FULL image, sampled per pass.

use crate::crc32::crc32_two;
use crate::zlib::zlib_store;

/// Raw scanline data cap: a fixture bigger than this is a bug in
/// the caller, not a fixture. Named here so the error can quote it.
pub const MAX_RAW_BYTES: u64 = 256 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PngParams {
    pub width: u32,
    pub height: u32,
    pub color_type: u8,
    pub bit_depth: u8,
    pub interlace: bool,
}

pub fn channels(color_type: u8) -> Option<u32> {
    match color_type {
        0 => Some(1), // grayscale
        2 => Some(3), // RGB
        3 => Some(1), // palette index
        4 => Some(2), // grayscale + alpha
        6 => Some(4), // RGBA
        _ => None,
    }
}

/// The (color type, bit depth) pairs the PNG spec allows.
pub fn valid_combo(color_type: u8, bit_depth: u8) -> bool {
    match color_type {
        0 => matches!(bit_depth, 1 | 2 | 4 | 8 | 16),
        2 => matches!(bit_depth, 8 | 16),
        3 => matches!(bit_depth, 1 | 2 | 4 | 8),
        4 | 6 => matches!(bit_depth, 8 | 16),
        _ => false,
    }
}

/// The pattern every fixture pixel comes from. For channel `c`
/// of pixel (x, y) in a `w`×`h` image:
///
/// 1. a diagonal gradient: `base = (gx + gy) / 2` where `gx`
///    runs 0→255 left-to-right and `gy` 0→255 top-to-bottom
///    (a 1-pixel axis contributes a constant: 128 for x, 0 for y);
/// 2. an 8×8 checkerboard flip: on odd checker squares the
///    value inverts (`255 - base`);
/// 3. a per-channel offset (`c * 67`) so channels differ;
/// 4. the seed, folded to one byte, added last.
///
/// The result is a sample value in 0..=255, scaled to the bit
/// depth by `v * (2^depth - 1) / 255`.
pub fn pattern_u8(seed: u64, x: u32, y: u32, w: u32, h: u32, c: u32) -> u8 {
    let gx: u32 = if w > 1 { x * 255 / (w - 1) } else { 128 };
    let gy: u32 = if h > 1 { y * 255 / (h - 1) } else { 0 };
    let base = (gx + gy) / 2;
    let flipped = ((x >> 3) + (y >> 3)) % 2 == 1;
    let v = if flipped { 255 - base } else { base };
    let seed_byte = ((seed ^ (seed >> 8) ^ (seed >> 16) ^ (seed >> 24)) & 0xFF) as u32;
    ((v + c * 67 + seed_byte) % 256) as u8
}

fn sample(seed: u64, x: u32, y: u32, p: &PngParams, c: u32) -> u32 {
    let v = pattern_u8(seed, x, y, p.width, p.height, c) as u32;
    let maxval = (1u32 << p.bit_depth) - 1;
    v * maxval / 255
}

/// Adam7 pass table: (x_start, y_start, x_step, y_step).
const ADAM7: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// (width, height) of one pass's sub-image; (0, 0) if the pass
/// contains no pixels of a `w`×`h` image.
fn pass_dims(w: u32, h: u32, pass: (u32, u32, u32, u32)) -> (u32, u32) {
    let (x0, y0, xs, ys) = pass;
    if w <= x0 || h <= y0 {
        return (0, 0);
    }
    ((w - x0).div_ceil(xs), (h - y0).div_ceil(ys))
}

fn row_bytes(pixels: u32, p: &PngParams) -> u64 {
    let ch = channels(p.color_type).unwrap_or(1) as u64;
    (pixels as u64 * ch * p.bit_depth as u64).div_ceil(8)
}

/// Total bytes of raw scanline data (filter bytes included) the
/// parameters produce. The figure both the encoder's size cap
/// and the tests' size expectations are stated in.
pub fn raw_len(p: &PngParams) -> u64 {
    if p.interlace {
        ADAM7
            .iter()
            .map(|&pass| {
                let (pw, ph) = pass_dims(p.width, p.height, pass);
                if pw == 0 || ph == 0 {
                    0
                } else {
                    ph as u64 * (1 + row_bytes(pw, p))
                }
            })
            .sum()
    } else {
        p.height as u64 * (1 + row_bytes(p.width, p))
    }
}

/// Pack one pass's scanlines (filter byte 0 + samples, sub-byte
/// depths packed most-significant-bit first) onto `raw`.
fn pack_pass(raw: &mut Vec<u8>, seed: u64, p: &PngParams, pass: (u32, u32, u32, u32)) {
    let (x0, y0, xs, ys) = pass;
    let (pw, ph) = pass_dims(p.width, p.height, pass);
    let ch = channels(p.color_type).unwrap_or(1);
    for py in 0..ph {
        raw.push(0); // filter: None
        let y = y0 + py * ys;
        match p.bit_depth {
            16 => {
                for px in 0..pw {
                    let x = x0 + px * xs;
                    for c in 0..ch {
                        raw.extend_from_slice(&(sample(seed, x, y, p, c) as u16).to_be_bytes());
                    }
                }
            }
            8 => {
                for px in 0..pw {
                    let x = x0 + px * xs;
                    for c in 0..ch {
                        raw.push(sample(seed, x, y, p, c) as u8);
                    }
                }
            }
            depth => {
                // 1/2/4-bit samples, packed MSB-first; the row's
                // final byte is zero-padded in its low bits.
                let mut acc: u32 = 0;
                let mut nbits: u32 = 0;
                for px in 0..pw {
                    let x = x0 + px * xs;
                    for c in 0..ch {
                        acc = (acc << depth) | sample(seed, x, y, p, c);
                        nbits += depth as u32;
                        if nbits == 8 {
                            raw.push(acc as u8);
                            acc = 0;
                            nbits = 0;
                        }
                    }
                }
                if nbits > 0 {
                    raw.push((acc << (8 - nbits)) as u8);
                }
            }
        }
    }
}

fn chunk(out: &mut Vec<u8>, typ: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(typ);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32_two(typ, data).to_be_bytes());
}

/// Encode the PNG. Assumes the parameters were already validated
/// (see `valid_combo` and the size rules in `generate`); the raw
/// size cap is enforced here so no caller can skip it.
pub fn encode(seed: u64, p: &PngParams) -> Result<Vec<u8>, String> {
    let raw_total = raw_len(p);
    if raw_total > MAX_RAW_BYTES {
        return Err(format!(
            "png raw scanline data would be {raw_total} bytes, over the {MAX_RAW_BYTES}-byte fixture cap"
        ));
    }
    let mut raw = Vec::with_capacity(raw_total as usize);
    if p.interlace {
        for pass in ADAM7 {
            pack_pass(&mut raw, seed, p, pass);
        }
    } else {
        pack_pass(&mut raw, seed, p, (0, 0, 1, 1));
    }

    let mut out = Vec::with_capacity(raw.len() + 128);
    out.extend_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&p.width.to_be_bytes());
    ihdr.extend_from_slice(&p.height.to_be_bytes());
    ihdr.push(p.bit_depth);
    ihdr.push(p.color_type);
    ihdr.push(0); // compression: deflate
    ihdr.push(0); // filter: adaptive (we emit filter 0 rows)
    ihdr.push(if p.interlace { 1 } else { 0 });
    chunk(&mut out, b"IHDR", &ihdr);
    if p.color_type == 3 {
        // Exactly 2^bit_depth palette entries — the spec's
        // maximum for the depth — from the same pattern function.
        let entries = 1u32 << p.bit_depth;
        let mut plte = Vec::with_capacity(entries as usize * 3);
        for i in 0..entries {
            for c in 0..3 {
                plte.push(pattern_u8(seed, i, 0, entries, 1, c));
            }
        }
        chunk(&mut out, b"PLTE", &plte);
    }
    chunk(&mut out, b"IDAT", &zlib_store(&raw));
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crc32::crc32_two;
    use crate::zlib::inflate_stored;

    struct Walked {
        types: Vec<String>,
        ihdr: Vec<u8>,
        plte_len: Option<usize>,
        raw: Vec<u8>,
    }

    /// Chunk-by-chunk structural walk: signature, per-chunk CRC,
    /// IDAT reassembly + stored-inflate. Any structural flaw is
    /// an Err, so truncations must fail here.
    fn walk(png: &[u8]) -> Result<Walked, String> {
        if png.len() < 8 || png[..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
            return Err("bad signature".into());
        }
        let mut pos = 8usize;
        let mut w = Walked {
            types: Vec::new(),
            ihdr: Vec::new(),
            plte_len: None,
            raw: Vec::new(),
        };
        let mut idat = Vec::new();
        loop {
            if pos + 12 > png.len() {
                return Err("chunk header truncated".into());
            }
            let len = u32::from_be_bytes(png[pos..pos + 4].try_into().unwrap()) as usize;
            let typ: [u8; 4] = png[pos + 4..pos + 8].try_into().unwrap();
            if pos + 12 + len > png.len() {
                return Err("chunk data truncated".into());
            }
            let data = &png[pos + 8..pos + 8 + len];
            let crc = u32::from_be_bytes(png[pos + 8 + len..pos + 12 + len].try_into().unwrap());
            if crc != crc32_two(&typ, data) {
                return Err(format!(
                    "CRC mismatch in chunk {}",
                    String::from_utf8_lossy(&typ)
                ));
            }
            let name = String::from_utf8_lossy(&typ).into_owned();
            match &typ {
                b"IHDR" => w.ihdr = data.to_vec(),
                b"PLTE" => w.plte_len = Some(len),
                b"IDAT" => idat.extend_from_slice(data),
                _ => {}
            }
            w.types.push(name);
            pos += 12 + len;
            if &typ == b"IEND" {
                break;
            }
        }
        if pos != png.len() {
            return Err("trailing bytes after IEND".into());
        }
        w.raw = inflate_stored(&idat)?;
        Ok(w)
    }

    fn params(ct: u8, bd: u8, interlace: bool) -> PngParams {
        PngParams {
            width: 13,
            height: 11,
            color_type: ct,
            bit_depth: bd,
            interlace,
        }
    }

    #[test]
    fn every_color_type_walks_with_valid_crcs_and_expected_raw_size() {
        for (ct, depths) in [
            (0u8, &[1u8, 2, 4, 8, 16][..]),
            (2, &[8, 16][..]),
            (3, &[1, 2, 4, 8][..]),
            (4, &[8, 16][..]),
            (6, &[8, 16][..]),
        ] {
            for &bd in depths {
                for interlace in [false, true] {
                    let p = params(ct, bd, interlace);
                    let png = encode(7, &p).unwrap();
                    let w =
                        walk(&png).unwrap_or_else(|e| panic!("ct{ct} bd{bd} il{interlace}: {e}"));
                    assert_eq!(w.types[0], "IHDR");
                    assert_eq!(w.types[w.types.len() - 1], "IEND");
                    assert_eq!(u32::from_be_bytes(w.ihdr[0..4].try_into().unwrap()), 13);
                    assert_eq!(u32::from_be_bytes(w.ihdr[4..8].try_into().unwrap()), 11);
                    assert_eq!(w.ihdr[8], bd);
                    assert_eq!(w.ihdr[9], ct);
                    assert_eq!(w.ihdr[12], interlace as u8);
                    assert_eq!(
                        w.raw.len() as u64,
                        raw_len(&p),
                        "raw size ct{ct} bd{bd} il{interlace}"
                    );
                    if ct == 3 {
                        assert_eq!(w.plte_len, Some(3 * (1usize << bd)));
                    } else {
                        assert_eq!(w.plte_len, None);
                    }
                }
            }
        }
    }

    #[test]
    fn pixel_pattern_is_the_documented_one() {
        // 1x1 RGBA8: raw = filter byte + the pattern sample at (0,0).
        let p = PngParams {
            width: 1,
            height: 1,
            color_type: 6,
            bit_depth: 8,
            interlace: false,
        };
        let w = walk(&encode(0, &p).unwrap()).unwrap();
        assert_eq!(w.raw[0], 0);
        for c in 0..4 {
            assert_eq!(w.raw[1 + c as usize], pattern_u8(0, 0, 0, 1, 1, c));
        }
        // 16-bit samples are the 8-bit pattern scaled to 65535.
        let p16 = PngParams { bit_depth: 16, ..p };
        let w16 = walk(&encode(0, &p16).unwrap()).unwrap();
        let v = pattern_u8(0, 0, 0, 1, 1, 0) as u32;
        let expect = (v * 65535 / 255) as u16;
        assert_eq!(u16::from_be_bytes([w16.raw[1], w16.raw[2]]), expect);
    }

    #[test]
    fn adam7_passes_cover_each_pixel_once() {
        // For a spread of sizes, the interlaced raw length must
        // equal the sum of the seven pass sizes and every pixel
        // must be visited exactly once across passes.
        for (wd, ht) in [(1u32, 1u32), (8, 8), (9, 7), (16, 16), (3, 40)] {
            let mut seen = vec![0u32; (wd * ht) as usize];
            let mut total = 0u64;
            for pass in ADAM7 {
                let (pw, ph) = pass_dims(wd, ht, pass);
                total += ph as u64 * (1 + row_bytes(pw, &params(0, 8, true)));
                for py in 0..ph {
                    for px in 0..pw {
                        let x = pass.0 + px * pass.2;
                        let y = pass.1 + py * pass.3;
                        seen[(y * wd + x) as usize] += 1;
                    }
                }
            }
            assert!(seen.iter().all(|&n| n == 1), "size {wd}x{ht}");
            let p = PngParams {
                width: wd,
                height: ht,
                color_type: 0,
                bit_depth: 8,
                interlace: true,
            };
            assert_eq!(raw_len(&p), total);
        }
    }

    #[test]
    fn invalid_combos_are_named_invalid() {
        assert!(valid_combo(0, 1) && valid_combo(0, 16));
        assert!(valid_combo(3, 8) && !valid_combo(3, 16));
        assert!(!valid_combo(2, 4));
        assert!(!valid_combo(6, 2));
        assert!(!valid_combo(1, 8));
        assert!(!valid_combo(0, 3));
    }

    #[test]
    fn oversized_image_hits_the_cap() {
        let p = PngParams {
            width: 20000,
            height: 20000,
            color_type: 6,
            bit_depth: 16,
            interlace: false,
        };
        let err = encode(0, &p).unwrap_err();
        assert!(err.contains("cap"), "{err}");
    }

    #[test]
    fn truncated_png_fails_the_walk() {
        let png = encode(3, &params(2, 8, false)).unwrap();
        for keep in [0usize, 8, 20, 40, png.len() - 1] {
            assert!(walk(&png[..keep]).is_err(), "keep={keep}");
        }
    }

    #[test]
    fn deterministic_bytes() {
        let p = params(6, 8, true);
        assert_eq!(encode(42, &p).unwrap(), encode(42, &p).unwrap());
        assert_ne!(encode(42, &p).unwrap(), encode(43, &p).unwrap());
    }
}
