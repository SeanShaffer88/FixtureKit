//! Spec parsing and generation dispatch — the whole product
//! behind `fx_generate`. Pure `std`: file bytes are built in
//! memory, written once, and reported with their SHA-256.
//!
//! Determinism is the contract: same spec + seed, same bytes,
//! on every platform. Everything below is integer math except
//! the WAV tone, which uses an in-crate polynomial sine (never
//! the platform libm, whose last-bit answers vary by OS).

use std::path::Path;

use crate::archive::{self, EntrySpec};
use crate::jpeg::JPEG_SAMPLE;
use crate::json::{self, Value};
use crate::png::{self, PngParams};
use crate::sha256::sha256_hex;
use crate::{json_escape, ERR_ACCESS_DENIED, ERR_INVALID_ARG, ERR_NOT_FOUND, ERR_PLATFORM};

pub type FxResult<T> = Result<T, (i32, String)>;

fn bad(msg: impl Into<String>) -> (i32, String) {
    (ERR_INVALID_ARG, msg.into())
}

// ---------------------------------------------------------- spec helpers

fn req_key<'a>(spec: &'a Value, key: &str) -> FxResult<&'a Value> {
    spec.get(key)
        .ok_or_else(|| bad(format!("missing required key \"{key}\"")))
}

fn req_u64(spec: &Value, key: &str) -> FxResult<u64> {
    let v = req_key(spec, key)?;
    match v.as_i64() {
        Some(n) if n >= 0 => Ok(n as u64),
        _ => Err(bad(format!(
            "\"{key}\" must be an unsigned integer, got {}",
            v.type_name()
        ))),
    }
}

fn opt_u64(spec: &Value, key: &str) -> FxResult<Option<u64>> {
    match spec.get(key) {
        None => Ok(None),
        Some(_) => Ok(Some(req_u64(spec, key)?)),
    }
}

fn req_u32(spec: &Value, key: &str) -> FxResult<u32> {
    let n = req_u64(spec, key)?;
    u32::try_from(n).map_err(|_| bad(format!("\"{key}\" is too large: {n}")))
}

fn opt_bool(spec: &Value, key: &str, default: bool) -> FxResult<bool> {
    match spec.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| bad(format!("\"{key}\" must be a boolean"))),
    }
}

fn req_str<'a>(spec: &'a Value, key: &str) -> FxResult<&'a str> {
    req_key(spec, key)?.as_str().ok_or_else(|| {
        bad(format!(
            "\"{key}\" must be a string, got {}",
            req_key(spec, key).unwrap().type_name()
        ))
    })
}

/// Reject keys outside the kind's known set — a typo'd key that
/// silently did nothing would produce a fixture the caller did
/// not ask for.
fn check_keys(spec: &Value, kind: &str, mut allowed: Vec<&str>) -> FxResult<()> {
    allowed.extend(["kind", "path", "seed"]);
    for k in spec.keys() {
        if !allowed.contains(&k) {
            return Err(bad(format!("unknown key \"{k}\" for kind \"{kind}\"")));
        }
    }
    Ok(())
}

// ---------------------------------------------------------- WAV

/// Largest WAV data section: 64 MiB of samples.
pub const MAX_WAV_DATA_BYTES: u64 = 64 * 1024 * 1024;

/// Sine by polynomial, pure f64 arithmetic (IEEE-exact ops only:
/// no libm, no FMA contraction in Rust), so the tone is
/// bit-identical on every OS. Input is a fraction of a turn in
/// [0, 1); quadrant reduction keeps the polynomial argument in
/// [0, pi/2], where the Taylor series through x^13 is accurate
/// to under 1e-9 — far below one step of a 16-bit sample.
fn sin_turns(t: f64) -> f64 {
    const TWO_PI: f64 = std::f64::consts::TAU;
    let (x, neg) = if t < 0.25 {
        (TWO_PI * t, false)
    } else if t < 0.5 {
        (TWO_PI * (0.5 - t), false)
    } else if t < 0.75 {
        (TWO_PI * (t - 0.5), true)
    } else {
        (TWO_PI * (1.0 - t), true)
    };
    let x2 = x * x;
    // Horner form of x - x^3/3! + x^5/5! - ... + x^13/13!.
    let p = 1.0
        + x2 * (-0.16666666666666666
            + x2 * (0.008333333333333333
                + x2 * (-0.0001984126984126984
                    + x2 * (0.0000027557319223985893
                        + x2 * (-0.000000025052108385441718
                            + x2 * 0.00000000016059043836821615)))));
    let s = x * p;
    if neg {
        -s
    } else {
        s
    }
}

fn gen_wav(spec: &Value) -> FxResult<Vec<u8>> {
    let freq = req_u64(spec, "freq_hz")?;
    if freq == 0 {
        return Err(bad("\"freq_hz\" must be at least 1"));
    }
    let duration_ms = req_u64(spec, "duration_ms")?;
    let sample_rate = req_u32(spec, "sample_rate")?;
    if sample_rate == 0 {
        return Err(bad("\"sample_rate\" must be at least 1"));
    }
    let amplitude = match spec.get("amplitude") {
        None => 0.5,
        Some(Value::I64(n)) if *n >= 0 && *n <= 1 => *n as f64,
        Some(Value::F64(f)) if *f >= 0.0 && *f <= 1.0 => *f,
        Some(_) => return Err(bad("\"amplitude\" must be a number in 0.0..=1.0")),
    };
    let samples = (sample_rate as u64)
        .checked_mul(duration_ms)
        .ok_or_else(|| bad("wav duration overflows"))?
        / 1000;
    if samples == 0 {
        return Err(bad(
            "\"duration_ms\" is too short for even one sample at this sample_rate",
        ));
    }
    let data_len = samples * 2;
    if data_len > MAX_WAV_DATA_BYTES {
        return Err(bad(format!(
            "wav data would be {data_len} bytes, over the {MAX_WAV_DATA_BYTES}-byte fixture cap"
        )));
    }
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 2).to_le_bytes()); // byte rate
    out.extend_from_slice(&2u16.to_le_bytes()); // block align
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for i in 0..samples {
        // Phase as an exact fraction of a turn: (i * freq) mod
        // sample_rate over sample_rate — integer math until the
        // final division, so no phase error accumulates.
        let frac = ((i * freq) % sample_rate as u64) as f64 / sample_rate as f64;
        let v = (amplitude * 32767.0 * sin_turns(frac))
            .round()
            .clamp(-32768.0, 32767.0) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    Ok(out)
}

// ---------------------------------------------------------- text

/// The lorem word cycle, word for word. Line `i` (0-based) is
/// eight words: `WORDS[(seed + 8*i + j) mod 10]` for j in 0..8,
/// joined by single spaces.
const LOREM_WORDS: [&str; 10] = [
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
];

/// Line cap for kind "text", named for the error message.
pub const MAX_TEXT_LINES: u64 = 1_000_000;

fn gen_text(spec: &Value, seed: u64) -> FxResult<Vec<u8>> {
    let lines = req_u64(spec, "lines")?;
    if lines > MAX_TEXT_LINES {
        return Err(bad(format!(
            "\"lines\" is {lines}, over the {MAX_TEXT_LINES}-line fixture cap"
        )));
    }
    let pattern = req_str(spec, "line_pattern")?;
    let mut out = String::new();
    match pattern {
        "numbered" => {
            for n in 1..=lines {
                out.push_str(&format!("line {n}\n"));
            }
        }
        "lorem" => {
            let start = (seed % 10) as usize;
            for i in 0..lines {
                let mut words = Vec::with_capacity(8);
                for j in 0..8usize {
                    words.push(LOREM_WORDS[(start + (i as usize) * 8 + j) % 10]);
                }
                out.push_str(&words.join(" "));
                out.push('\n');
            }
        }
        "json-lines" => {
            for n in 1..=lines {
                out.push_str(&format!("{{\"line\":{n},\"seed\":{seed}}}\n"));
            }
        }
        other => {
            return Err(bad(format!(
                "\"line_pattern\" must be \"numbered\", \"lorem\" or \"json-lines\", got \"{other}\""
            )));
        }
    }
    Ok(out.into_bytes())
}

// ---------------------------------------------------------- archives

fn parse_entries(spec: &Value) -> FxResult<Vec<EntrySpec>> {
    let arr = req_key(spec, "entries")?
        .as_array()
        .ok_or_else(|| bad("\"entries\" must be an array"))?;
    if arr.len() > archive::MAX_ENTRIES {
        return Err(bad(format!(
            "\"entries\" has {} items, over the {}-entry fixture cap",
            arr.len(),
            archive::MAX_ENTRIES
        )));
    }
    let mut total: u64 = 0;
    let mut out = Vec::with_capacity(arr.len());
    for (idx, item) in arr.iter().enumerate() {
        if item.as_object().is_none() {
            return Err(bad(format!("entries[{idx}] must be an object")));
        }
        for k in item.keys() {
            if !["name", "size", "fill_byte"].contains(&k) {
                return Err(bad(format!("unknown key \"{k}\" in entries[{idx}]")));
            }
        }
        let name = req_str(item, "name")?.to_string();
        if name.is_empty() {
            return Err(bad(format!("entries[{idx}].name must not be empty")));
        }
        let size = req_u64(item, "size")?;
        if size > archive::MAX_ENTRY_BYTES {
            return Err(bad(format!(
                "entries[{idx}].size is {size}, over the {}-byte per-entry cap",
                archive::MAX_ENTRY_BYTES
            )));
        }
        total = total
            .checked_add(size)
            .ok_or_else(|| bad("entries total size overflows"))?;
        if total > archive::MAX_TOTAL_BYTES {
            return Err(bad(format!(
                "entries total {total} bytes, over the {}-byte fixture cap",
                archive::MAX_TOTAL_BYTES
            )));
        }
        let fill_byte = match opt_u64(item, "fill_byte")? {
            None => 0,
            Some(n) if n <= 255 => n as u8,
            Some(n) => {
                return Err(bad(format!(
                    "entries[{idx}].fill_byte is {n}, over the byte maximum 255"
                )))
            }
        };
        out.push(EntrySpec {
            name,
            size,
            fill_byte,
        });
    }
    Ok(out)
}

// ---------------------------------------------------------- PNG kinds

fn png_params(spec: &Value) -> FxResult<PngParams> {
    let width = req_u32(spec, "width")?;
    let height = req_u32(spec, "height")?;
    if width == 0 || height == 0 {
        return Err(bad("\"width\" and \"height\" must be at least 1"));
    }
    let color_type = req_u64(spec, "color_type")?;
    let bit_depth = req_u64(spec, "bit_depth")?;
    if color_type > 255 || bit_depth > 255 {
        return Err(bad("\"color_type\" and \"bit_depth\" must be byte values"));
    }
    let (ct, bd) = (color_type as u8, bit_depth as u8);
    if png::channels(ct).is_none() {
        return Err(bad(format!(
            "\"color_type\" must be 0, 2, 3, 4 or 6, got {ct}"
        )));
    }
    if !png::valid_combo(ct, bd) {
        return Err(bad(format!(
            "bit_depth {bd} is not allowed for color_type {ct} by the PNG spec"
        )));
    }
    Ok(PngParams {
        width,
        height,
        color_type: ct,
        bit_depth: bd,
        interlace: opt_bool(spec, "interlace", false)?,
    })
}

// ---------------------------------------------------------- dispatch

/// Generate the file the spec names and return the result JSON:
/// `{"path": ..., "kind": ..., "size": N, "sha256": "..."}`.
pub fn generate(spec_json: &str) -> FxResult<String> {
    let spec = json::parse(spec_json).map_err(|e| bad(format!("spec is not valid JSON: {e}")))?;
    if spec.as_object().is_none() {
        return Err(bad("spec must be a JSON object"));
    }
    let kind = req_str(&spec, "kind")?.to_string();
    let path_text = req_str(&spec, "path")?.to_string();
    if path_text.is_empty() {
        return Err(bad("\"path\" must not be empty"));
    }
    if path_text.contains('\0') {
        return Err(bad("\"path\" must not contain a NUL character"));
    }
    let path = Path::new(&path_text);
    if !path.is_absolute() {
        return Err(bad(format!(
            "\"path\" must be absolute, got \"{path_text}\""
        )));
    }
    let seed = opt_u64(&spec, "seed")?.unwrap_or(0);

    let bytes: Vec<u8> = match kind.as_str() {
        "png" => {
            check_keys(
                &spec,
                &kind,
                vec!["width", "height", "color_type", "bit_depth", "interlace"],
            )?;
            png::encode(seed, &png_params(&spec)?).map_err(bad)?
        }
        "png_truncated" => {
            check_keys(
                &spec,
                &kind,
                vec![
                    "width",
                    "height",
                    "color_type",
                    "bit_depth",
                    "interlace",
                    "keep_bytes",
                ],
            )?;
            let full = png::encode(seed, &png_params(&spec)?).map_err(bad)?;
            let keep = req_u64(&spec, "keep_bytes")? as usize;
            full[..keep.min(full.len())].to_vec()
        }
        "jpeg_sample" => {
            check_keys(&spec, &kind, vec![])?;
            JPEG_SAMPLE.to_vec()
        }
        "jpeg_truncated" => {
            check_keys(&spec, &kind, vec!["keep_bytes"])?;
            let keep = req_u64(&spec, "keep_bytes")? as usize;
            JPEG_SAMPLE[..keep.min(JPEG_SAMPLE.len())].to_vec()
        }
        "wav" => {
            check_keys(
                &spec,
                &kind,
                vec!["freq_hz", "duration_ms", "sample_rate", "amplitude"],
            )?;
            gen_wav(&spec)?
        }
        "text" => {
            check_keys(&spec, &kind, vec!["lines", "line_pattern"])?;
            gen_text(&spec, seed)?
        }
        "zip_stored" => {
            check_keys(&spec, &kind, vec!["entries"])?;
            archive::zip_stored(&parse_entries(&spec)?)
        }
        "tar" => {
            check_keys(&spec, &kind, vec!["entries"])?;
            archive::tar(&parse_entries(&spec)?).map_err(bad)?
        }
        other => {
            return Err(bad(format!(
                "unknown kind \"{other}\" (kinds: png, png_truncated, jpeg_sample, jpeg_truncated, wav, text, zip_stored, tar)"
            )));
        }
    };

    std::fs::write(path, &bytes).map_err(|e| {
        let code = match e.kind() {
            std::io::ErrorKind::NotFound => ERR_NOT_FOUND,
            std::io::ErrorKind::PermissionDenied => ERR_ACCESS_DENIED,
            _ => ERR_PLATFORM,
        };
        (code, format!("cannot write \"{path_text}\": {e}"))
    })?;

    Ok(format!(
        "{{\"path\":\"{}\",\"kind\":\"{}\",\"size\":{},\"sha256\":\"{}\"}}",
        json_escape(&path_text),
        json_escape(&kind),
        bytes.len(),
        sha256_hex(&bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;

    fn tmpdir(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("fixturekit-gen-{}-{}", std::process::id(), tag));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn run(tag: &str, spec_body: &str) -> (String, Vec<u8>) {
        let dir = tmpdir(tag);
        let path = dir.join("out.bin");
        let spec = format!(
            "{{\"path\":\"{}\",{}}}",
            json_escape(&path.to_string_lossy()),
            spec_body
        );
        let result = generate(&spec).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        (result, bytes)
    }

    #[test]
    fn wav_header_and_tone_math() {
        let (result, b) = run(
            "wav",
            "\"kind\":\"wav\",\"freq_hz\":1000,\"duration_ms\":1000,\"sample_rate\":8000",
        );
        assert_eq!(&b[0..4], b"RIFF");
        assert_eq!(u32::from_le_bytes(b[4..8].try_into().unwrap()), 36 + 16000);
        assert_eq!(&b[8..12], b"WAVE");
        assert_eq!(u32::from_le_bytes(b[24..28].try_into().unwrap()), 8000);
        assert_eq!(u32::from_le_bytes(b[28..32].try_into().unwrap()), 16000);
        assert_eq!(u16::from_le_bytes(b[34..36].try_into().unwrap()), 16);
        assert_eq!(u32::from_le_bytes(b[40..44].try_into().unwrap()), 16000);
        assert_eq!(b.len(), 44 + 16000);
        let sample = |i: usize| i16::from_le_bytes(b[44 + i * 2..46 + i * 2].try_into().unwrap());
        assert_eq!(sample(0), 0, "sine starts at zero");
        // Quarter period (i=2 at 1 kHz / 8 kHz): full amplitude.
        assert!(
            (sample(2) - 16384).abs() <= 1,
            "quarter period: {}",
            sample(2)
        );
        // Half period: back through zero. Three-quarter: full negative.
        assert!(sample(4).abs() <= 1);
        assert!((sample(6) + 16384).abs() <= 1);
        let parsed = json::parse(&result).unwrap();
        assert_eq!(parsed.get("kind").unwrap().as_str(), Some("wav"));
        assert_eq!(parsed.get("size").unwrap().as_i64(), Some(b.len() as i64));
        assert_eq!(
            parsed.get("sha256").unwrap().as_str(),
            Some(sha256_hex(&b).as_str())
        );
    }

    #[test]
    fn text_patterns_are_exact() {
        let (_, b) = run(
            "textn",
            "\"kind\":\"text\",\"lines\":3,\"line_pattern\":\"numbered\"",
        );
        assert_eq!(String::from_utf8(b).unwrap(), "line 1\nline 2\nline 3\n");
        let (_, b) = run(
            "textj",
            "\"kind\":\"text\",\"lines\":2,\"line_pattern\":\"json-lines\",\"seed\":5",
        );
        let text = String::from_utf8(b).unwrap();
        assert_eq!(text, "{\"line\":1,\"seed\":5}\n{\"line\":2,\"seed\":5}\n");
        for (i, line) in text.lines().enumerate() {
            let v = json::parse(line).unwrap();
            assert_eq!(v.get("line").unwrap().as_i64(), Some(i as i64 + 1));
        }
        let (_, b) = run(
            "textl",
            "\"kind\":\"text\",\"lines\":2,\"line_pattern\":\"lorem\"",
        );
        let text = String::from_utf8(b).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.starts_with("lorem ipsum dolor sit amet consectetur adipiscing elit\n"));
        let (_, b) = run(
            "text0",
            "\"kind\":\"text\",\"lines\":0,\"line_pattern\":\"numbered\"",
        );
        assert!(b.is_empty());
    }

    #[test]
    fn truncated_kinds_are_strict_prefixes() {
        let (_, full) = run(
            "pngf",
            "\"kind\":\"png\",\"width\":9,\"height\":9,\"color_type\":2,\"bit_depth\":8",
        );
        let (result, cut) = run(
            "pngt",
            "\"kind\":\"png_truncated\",\"width\":9,\"height\":9,\"color_type\":2,\"bit_depth\":8,\"keep_bytes\":50",
        );
        assert_eq!(cut.len(), 50);
        assert_eq!(cut, full[..50]);
        assert_ne!(cut, full);
        let parsed = json::parse(&result).unwrap();
        assert_eq!(parsed.get("kind").unwrap().as_str(), Some("png_truncated"));
        // jpeg_truncated keeps the REQUESTED kind in its result.
        let (result, cut) = run("jpegt", "\"kind\":\"jpeg_truncated\",\"keep_bytes\":100");
        assert_eq!(cut.len(), 100);
        assert_eq!(cut, JPEG_SAMPLE[..100]);
        assert_ne!(
            &cut[cut.len() - 2..],
            &[0xFF, 0xD9],
            "no EOI in a truncated jpeg"
        );
        let parsed = json::parse(&result).unwrap();
        assert_eq!(parsed.get("kind").unwrap().as_str(), Some("jpeg_truncated"));
        assert_eq!(parsed.get("size").unwrap().as_i64(), Some(100));
    }

    #[test]
    fn same_spec_twice_is_byte_identical() {
        for body in [
            "\"kind\":\"png\",\"width\":17,\"height\":5,\"color_type\":6,\"bit_depth\":8,\"interlace\":true,\"seed\":99",
            "\"kind\":\"wav\",\"freq_hz\":440,\"duration_ms\":250,\"sample_rate\":22050,\"seed\":1",
            "\"kind\":\"tar\",\"entries\":[{\"name\":\"x\",\"size\":10,\"fill_byte\":65}]",
            "\"kind\":\"jpeg_sample\"",
        ] {
            let (r1, b1) = run("det1", body);
            let (r2, b2) = run("det2", body);
            assert_eq!(b1, b2, "bytes differ for {body}");
            let s1 = json::parse(&r1).unwrap().get("sha256").unwrap().as_str().unwrap().to_string();
            let s2 = json::parse(&r2).unwrap().get("sha256").unwrap().as_str().unwrap().to_string();
            assert_eq!(s1, s2);
        }
    }

    #[test]
    fn spec_errors_are_invalid_arg_with_reasons() {
        let dir = tmpdir("errs");
        let good_path = json_escape(&dir.join("o.bin").to_string_lossy());
        let cases: Vec<(String, &str)> = vec![
            ("{}".into(), "kind"),
            (format!("{{\"kind\":\"nope\",\"path\":\"{good_path}\"}}"), "unknown kind"),
            ("{\"kind\":\"png\",\"path\":\"relative/x.png\",\"width\":1,\"height\":1,\"color_type\":0,\"bit_depth\":8}".into(), "absolute"),
            (format!("{{\"kind\":\"png\",\"path\":\"{good_path}\",\"width\":1,\"height\":1,\"color_type\":2,\"bit_depth\":4}}"), "not allowed"),
            (format!("{{\"kind\":\"png\",\"path\":\"{good_path}\",\"width\":1,\"height\":1,\"color_type\":0,\"bit_depth\":8,\"bogus\":1}}"), "unknown key"),
            (format!("{{\"kind\":\"wav\",\"path\":\"{good_path}\",\"freq_hz\":440,\"duration_ms\":100,\"sample_rate\":8000,\"amplitude\":2}}"), "amplitude"),
            (format!("{{\"kind\":\"zip_stored\",\"path\":\"{good_path}\",\"entries\":[{{\"name\":\"a\",\"size\":1,\"fill_byte\":300}}]}}"), "fill_byte"),
            ("{\"kind\":\"text\",\"path\":\"/tmp/x\",\"lines\":1,\"line_pattern\":\"text\",\"seed\":1,\"seed\":2}".into(), "duplicate"),
        ];
        for (spec, needle) in cases {
            let (code, msg) = generate(&spec).unwrap_err();
            assert_eq!(code, ERR_INVALID_ARG, "spec: {spec}");
            assert!(
                msg.contains(needle),
                "spec {spec}: message \"{msg}\" lacks \"{needle}\""
            );
        }
        // Missing parent folder is NOT_FOUND, not a spec error.
        let (code, _) = generate(&format!(
            "{{\"kind\":\"jpeg_sample\",\"path\":\"{}/nope/x.jpg\"}}",
            dir.to_string_lossy()
        ))
        .unwrap_err();
        assert_eq!(code, ERR_NOT_FOUND);
    }
}
