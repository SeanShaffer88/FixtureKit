# FixtureKit

## TL;DR

One DLL call writes the exact test file your test needs — a real PNG in any colour type and bit depth (interlaced too), a WAV tone, a text file, a stored ZIP, a ustar TAR, or a deliberately truncated PNG/JPEG for corrupt-file tests — and hands back the file's size and SHA-256. Same spec, same seed, same bytes, every platform, every run: fixtures stop being binary blobs checked into repos and become one line of JSON.

```c
#include "fixturekit.h"

int main(void) {
    char *r = fx_generate(
        "{\"kind\":\"png\",\"path\":\"C:/tests/fixtures/rgb.png\","
        "\"width\":64,\"height\":48,\"color_type\":2,\"bit_depth\":8,\"seed\":7}");
    printf("%s\n", r);  /* {"path":"...","kind":"png","size":...,"sha256":"..."} */
    fx_free_string(r);
}
```

## What it does

- **One function, JSON in, JSON out.** `fx_generate(spec)` writes the file and returns `{"path","kind","size","sha256"}`. Failures return NULL with the reason in `fx_last_error()` — a spec problem (bad combination, unknown key, relative path) is `FX_ERR_INVALID_ARG` and the message says which key and why.
- **A real PNG encoder.** Colour types 0 (grayscale), 2 (RGB), 3 (palette), 4 (gray+alpha) and 6 (RGBA) at every bit depth the PNG spec allows for the type (1/2/4/8/16 as applicable), Adam7 interlacing with the correct seven-pass structure, per-chunk CRC-32, and the image data carried in a stored-block zlib stream. Pixels are a deterministic **gradient/checker pattern** (formula below), so a test can predict exact sample values, not just "the file parses". Palette images get a PLTE chunk of exactly 2^bit_depth entries generated from the same pattern.
- **Truncated variants for corrupt-file tests.** `png_truncated` and `jpeg_truncated` write the first `keep_bytes` of the file the full kind would produce — a strict prefix of a valid file, which is what real-world truncation actually looks like.
- **A WAV tone with exact header math.** 16-bit PCM mono sine: `freq_hz`, `duration_ms`, `sample_rate`, optional `amplitude` (0.0–1.0, default 0.5). The sample count is `sample_rate * duration_ms / 1000`, and the phase of each sample is computed as an exact fraction of a turn, so no rounding error accumulates across the file.
- **Text files.** `numbered` (`line 1`, `line 2`, …), `lorem` (eight words per line from a fixed ten-word cycle, offset by the seed), or `json-lines` (`{"line":N,"seed":S}` per line). Every line ends with `\n`, including the last.
- **Archives that real tools accept.** `zip_stored`: STORED entries with real CRC-32s and a full central directory. `tar`: ustar headers with valid checksums, long names split into the prefix field the way the format intends. Archive filler content is one repeated `fill_byte` per entry.
- **A fixed JPEG sample — honestly labelled.** `jpeg_sample` writes an embedded, real, complete baseline JPEG (SOF0, 631 bytes, a 1×1 white pixel, produced with a real encoder). It is a *fixed sample*, not an encoder: FixtureKit v0.1.0 cannot make a JPEG of your size or content, and it does not pretend to. A parametric JPEG encoder is ROADMAP item 1.

## Determinism

The same spec + seed produces byte-identical output on every platform. Everything is integer math except the WAV tone's sine, which is computed by an in-crate polynomial (never the platform math library, whose last-bit answers differ between operating systems). ZIP entries carry the DOS epoch date (1980-01-01) and TAR headers an mtime of 0, so archives have no clock in them either. The SHA-256 in the result JSON is there so a test can pin the bytes it produced.

## The pixel pattern

For channel `c` of pixel (x, y) in a `w`×`h` image, the 8-bit pattern value is:

1. a diagonal gradient: `base = (gx + gy) / 2`, where `gx` runs 0→255 left-to-right and `gy` 0→255 top-to-bottom (a 1-pixel-wide image uses gx = 128; a 1-pixel-high image uses gy = 0);
2. an 8×8 checkerboard: on odd checker squares the value inverts to `255 - base`;
3. a per-channel offset of `c * 67`, so channels differ;
4. the seed, folded to one byte (its four low bytes XORed), added last — all additions wrap modulo 256.

Samples at other bit depths scale it: `value * (2^depth - 1) / 255`. Interlaced images use the same values — the pattern is a function of the pixel's position in the full image, sampled per Adam7 pass.

## Spec errors and caps

Unknown keys — in the spec or in an archive entry — are errors, never silently ignored, and duplicate keys in one JSON object are a parse error. Invalid PNG combinations (say colour type 2 at bit depth 4, which the PNG standard forbids) are rejected with the reason named. Caps, each named in its error message: PNG raw scanline data 256 MiB; WAV data 64 MiB; archive entries 4096, 64 MiB per entry, 128 MiB total; text 1,000,000 lines.

## Honest limitations

- **JPEG is a fixed 1×1 sample only.** No parametric JPEG encoding — see above; ROADMAP item 1.
- **PNG compression is stored blocks.** The files are valid PNGs any decoder reads, but they are as large as the raw pixels. A fixture's job is to be predictable, not small; DEFLATE compression is ROADMAP item 2.
- **PNG rows all use filter 0 (None).** Legal and simple; decoders must handle it (it is the mandatory baseline filter).
- **Archive entries are filler.** Entry content is one repeated byte — these fixtures test archive *handling* (listing, extracting, CRC checks), not realistic file content.
- **The WAV tone is a pure sine.** No envelopes, no stereo, no other waveforms in v0.1.0.

## Building and tests

```sh
cargo build --release   # -> fixture_kit.dll / libfixture_kit.so / libfixture_kit.a
cargo test --locked     # full suite
```

The crate is Rust, edition 2021, zero dependencies, and builds as a `cdylib` + `rlib` + `staticlib`. The 30 tests cover: PNG chunk-by-chunk walks with per-chunk CRC verification and stored-block inflation back to the exact expected scanline size for **every** colour type at every legal bit depth, interlaced and not; Adam7 pass coverage (each pixel visited exactly once); the documented pattern value at real pixel positions; WAV header/length math and quarter-period tone samples; ZIP re-parsing through its central directory with fresh CRCs; TAR header checksums and the long-name prefix split; determinism (two runs of the same spec byte-equal, for images, audio and archives); the truncated kinds as strict prefixes that fail structural parsing; and the spec-error surface (unknown kind/key, forbidden PNG combination, relative path, duplicate key, missing parent folder). During development the output was additionally cross-checked against independent tools: Pillow decoded every PNG variant — including both Adam7 files — and reproduced the documented pattern pixel exactly; Python's `zipfile` (CRC check clean), `tarfile` and `wave` modules all read their fixtures.

License: MIT. Part of the WinDevKit family — see `AGENTS.md` for build discipline and `ROADMAP.md` for what is next.
