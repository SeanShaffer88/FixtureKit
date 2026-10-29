//! zlib streams made of DEFLATE *stored* (uncompressed) blocks.
//!
//! FixtureKit only ever WRITES stored blocks: a fixture's job is
//! to be a valid, predictable file, not a small one, and a stored
//! block encoder is a page of auditable code instead of a
//! compressor. The matching reader (`inflate_stored`) exists so
//! tests can prove the PNG encoder's IDAT payload really is the
//! scanlines it claims — it accepts stored blocks only and says
//! so, rather than pretending to be a general inflater.

/// Adler-32 (RFC 1950) of `data`.
pub fn adler32(data: &[u8]) -> u32 {
    const MOD: u32 = 65521;
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    // Process in chunks so b cannot overflow u32 between mods.
    for chunk in data.chunks(5552) {
        for &byte in chunk {
            a += byte as u32;
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

/// Wrap `data` in a zlib stream (RFC 1950) whose DEFLATE payload
/// is stored blocks of at most 65535 bytes each (RFC 1951 §3.2.4).
pub fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 6 + 5 * (data.len() / 65535 + 1));
    // CMF/FLG: 32K window, no preset dictionary, no compression.
    // 0x7801 is the standard pair: (0x78 * 256 + 0x01) % 31 == 0.
    out.push(0x78);
    out.push(0x01);
    if data.is_empty() {
        out.push(0x01); // BFINAL=1, BTYPE=00
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&(!0u16).to_le_bytes());
    } else {
        let mut chunks = data.chunks(65535).peekable();
        while let Some(chunk) = chunks.next() {
            let last = chunks.peek().is_none();
            out.push(if last { 0x01 } else { 0x00 });
            let len = chunk.len() as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(chunk);
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

/// Inflate a zlib stream that contains ONLY stored blocks.
/// Any fixed/dynamic-Huffman block is an error, by design —
/// this reader verifies FixtureKit's own encoder output, it is
/// not a general-purpose DEFLATE decoder.
pub fn inflate_stored(z: &[u8]) -> Result<Vec<u8>, String> {
    if z.len() < 2 {
        return Err("zlib stream shorter than its 2-byte header".into());
    }
    let cmf = z[0] as u16;
    let flg = z[1] as u16;
    if (cmf * 256 + flg) % 31 != 0 {
        return Err("zlib header check bits (FCHECK) do not validate".into());
    }
    if cmf & 0x0F != 8 {
        return Err(format!(
            "zlib compression method {} is not DEFLATE",
            cmf & 0x0F
        ));
    }
    if flg & 0x20 != 0 {
        return Err("zlib stream declares a preset dictionary".into());
    }
    let mut pos = 2usize;
    let mut out = Vec::new();
    loop {
        let hdr = *z
            .get(pos)
            .ok_or("deflate stream ends inside a block header")?;
        pos += 1;
        let bfinal = hdr & 1;
        let btype = (hdr >> 1) & 3;
        if btype != 0 {
            return Err(format!(
                "block type {btype} is not stored (0); inflate_stored only reads stored blocks"
            ));
        }
        // Bits 3..7 are the skip to the byte boundary; RFC 1951
        // says their value is ignored, so they are not checked.
        if pos + 4 > z.len() {
            return Err("stored block header is truncated".into());
        }
        let len = u16::from_le_bytes([z[pos], z[pos + 1]]) as usize;
        let nlen = u16::from_le_bytes([z[pos + 2], z[pos + 3]]);
        pos += 4;
        if nlen != !(len as u16) {
            return Err("stored block NLEN is not the complement of LEN".into());
        }
        if pos + len > z.len() {
            return Err("stored block data is truncated".into());
        }
        out.extend_from_slice(&z[pos..pos + len]);
        pos += len;
        if bfinal == 1 {
            break;
        }
    }
    if pos + 4 != z.len() {
        return Err("zlib stream does not end exactly after its Adler-32".into());
    }
    let stored_adler = u32::from_be_bytes([z[pos], z[pos + 1], z[pos + 2], z[pos + 3]]);
    if stored_adler != adler32(&out) {
        return Err("zlib Adler-32 does not match the inflated data".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adler_known_vector() {
        // RFC 1950's worked example: Adler-32("Wikipedia") = 0x11E60398.
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
        assert_eq!(adler32(b""), 1);
    }

    #[test]
    fn store_inflate_roundtrip() {
        for data in [
            Vec::new(),
            b"x".to_vec(),
            (0..65535u32).map(|i| (i % 251) as u8).collect(),
            (0..200_000u32).map(|i| (i % 251) as u8).collect(),
        ] {
            let z = zlib_store(&data);
            assert_eq!(inflate_stored(&z).unwrap(), data);
        }
    }

    #[test]
    fn inflate_rejects_compressed_blocks_and_corruption() {
        // A real fixed-Huffman stream (header byte 0x63 -> BTYPE=01).
        let compressed = [
            0x78, 0x9c, 0x63, 0x60, 0x60, 0x60, 0x60, 0x60, 0x04, 0x00, 0x00, 0x06, 0x00, 0x01,
        ];
        assert!(inflate_stored(&compressed).is_err());
        let mut z = zlib_store(b"hello");
        let n = z.len();
        z[n - 1] ^= 0xFF; // break the Adler-32
        assert!(inflate_stored(&z).is_err());
        assert!(inflate_stored(&z[..3]).is_err()); // truncated
        assert!(inflate_stored(&[]).is_err());
    }
}
