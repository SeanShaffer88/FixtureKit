# FixtureKit — Parity

What the brief asked for, and where each piece stands in v0.1.0. Status words: **Done** (implemented and test-covered), **Partial**, **Not done** (with the reason).

| Brief requirement | Status | Notes |
|---|---|---|
| DLL kit in the WinDevKit family pattern | Done | Rust, zero dependencies, edition 2021, rust-version 1.80, `cdylib`+`rlib`+`staticlib`, JSON in/out with the family parser (duplicate keys are a parse error), thread-local last error via `fx_last_error()`, caller-owned strings freed by `fx_free_string`. Scaffolded from the family skeleton; the skeleton's `platform.rs` was removed — generation is pure `std`, so there is no platform surface (recorded in AGENTS.md). |
| One export `fx_generate(spec_json) -> *mut c_char` (+ `fx_free_string`/`fx_last_error`/`fx_version`) | Done | Result `{"path","kind","size","sha256"}` with a hand-rolled SHA-256 (known-vector tested). Failures return NULL with an FX_ERR_* code and message. |
| `png` — real encoder: stored-block zlib, CRC32 per chunk, deterministic seed-derived pattern, Adam7 pass structure | Done | Colour types 0/2/3/4/6 at every spec-legal bit depth; invalid combinations rejected with the reason. Filter 0 rows; PLTE of exactly 2^bit_depth entries for colour type 3. Pattern formula documented in README and header; tests verify decoded pattern values at real pixel positions. |
| `png_truncated` {png params, keep_bytes} | Done | Writes the first keep_bytes of the full PNG; test-proven a strict prefix that fails structural chunk parsing. |
| `wav` {freq_hz, duration_ms, sample_rate, amplitude?} — 16-bit PCM mono, correct header math | Done | Sample count = rate × ms / 1000; per-sample phase is an exact fraction of a turn (no accumulated error); sine is an in-crate polynomial, not libm, so bytes are platform-identical. Header fields and quarter-period sample values are test-asserted. |
| `text` {lines, line_pattern: numbered/lorem/json-lines} | Done | Exact-output tests, including seed in json-lines and an empty file for lines: 0. |
| `zip_stored` {entries: [{name, size, fill_byte?}]} — valid STORED zip, correct CRCs | Done | Local headers + central directory + EOCD; DOS-epoch timestamps for determinism. Tests re-parse through the central directory and recompute every CRC. |
| `tar` {entries likewise} — valid ustar | Done | Real header checksums, mtime 0, names >100 bytes split into the ustar prefix field (unsplittable names are an error naming the limits). Tests verify checksums by re-summing each header. |
| `jpeg_sample` — fixed embedded minimal valid baseline JPEG (1×1), documented as a fixed sample | Done | 631-byte SOF0 JPEG (white 1×1, produced with Pillow/libjpeg, provenance in src/jpeg.rs), marker-walk tested to EOI with SOF0 declaring 1×1. README, header, ROADMAP and this file all call it a fixed sample; parametric encoding is ROADMAP item 1 — nothing fakes it. |
| `jpeg_truncated` {keep_bytes} | Done | Strict prefix of the sample; tests assert EOI is absent and the result kind echoes `jpeg_truncated`. |
| Determinism: same spec+seed → byte-identical output | Done | Two-run byte-equality tests across png/wav/tar/jpeg kinds; integer math throughout, polynomial sine for the tone, pinned archive timestamps. |
| Tests: PNG chunk walk + CRCs + IDAT inflation to expected raw size per colour type | Done | The walk runs for every colour type at every legal bit depth, interlaced and not, comparing inflated length to `png::raw_len`; Adam7 pass coverage proven pixel-by-pixel. |
| Tests: WAV header/length, ZIP central-directory re-parse, TAR checksums, determinism, corrupt kinds differ | Done | 30 tests total; see QUALITY.md. Development cross-check (not a test-suite dependency): Pillow decoded all generated PNG variants and matched the pattern pixel-exactly, Python zipfile/tarfile/wave read their fixtures. |
| Docs per family | Done | README (TL;DR first), documented header, AGENTS.md, ROADMAP.md, QUALITY.md, this file, LICENSE, ci.yml + `.github/workflows/ci.yml`, release.yml adapted from WatchKit. |
| Full gate green via WinDevKit tools/gate.sh; local only | Done | Gate result recorded in the delivery report; no git/GitHub actions taken. |

## Deviations from the brief, stated plainly

- **The JPEG sample is 631 bytes, not the smallest possible 1×1.** It carries a real encoder's full quantisation and Huffman tables (Pillow/libjpeg at quality 90). "Minimal" in the brief is honoured as *smallest honest form* — a complete, standard file from a real encoder — rather than a hand-trimmed byte string whose validity would rest on the trimmer.
- **`path` must be absolute**, enforced with an error: a relative fixture path would land in whatever folder the host process happens to run from, which is exactly the non-determinism this kit exists to remove.
- **Unknown spec keys are errors** (family rule, as in ThumbKit): the brief did not ask for them to be ignored, and silently ignoring a typo'd key would generate a different fixture than the caller specified.
- **Result `kind` echoes the requested kind** (so a `png_truncated` receipt says `png_truncated`, not `png`) — the receipt describes what was asked for and written, and `size`/`sha256` describe the truncated file actually on disk.
