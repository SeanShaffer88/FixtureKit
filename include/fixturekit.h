/* FixtureKit.dll — deterministic test-fixture file generator.
 *
 * One call, fx_generate, writes the file a JSON spec names and
 * returns a JSON receipt. Same spec + seed = byte-identical
 * file, on every platform: the generators are pure integer math
 * (the WAV tone uses an in-crate polynomial sine, never the
 * platform libm), ZIP entries carry the 1980-01-01 DOS date and
 * TAR headers an mtime of 0.
 *
 * All strings are UTF-8, NUL-terminated. char* results are
 * caller-owned: free with fx_free_string(). On failure
 * fx_generate returns NULL; fx_last_error() then holds the
 * reason and fx_last_error_code() the FX_ERR_* code.
 */
#ifndef FIXTUREKIT_H
#define FIXTUREKIT_H
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
#if defined(_WIN32) && defined(FX_BUILD)
#define FX_API __declspec(dllexport)
#elif defined(_WIN32)
#define FX_API __declspec(dllimport)
#else
#define FX_API
#endif
#define FX_OK 0
#define FX_ERR_INVALID_ARG 1
#define FX_ERR_NOT_SUPPORTED 2
#define FX_ERR_NOT_FOUND 3
#define FX_ERR_ACCESS_DENIED 4
#define FX_ERR_PLATFORM 5
#define FX_ERR_INTERNAL 6

FX_API const char *fx_version(void);
FX_API const char *fx_last_error(void);
FX_API int fx_last_error_code(void);
FX_API void fx_free_string(char *p);

/* fx_generate(spec_json) -> result JSON (caller-owned) or NULL.
 *
 * Spec (one JSON object; duplicate keys are a parse error;
 * unknown keys are an error, never ignored):
 *
 *   "kind"  string, required — one of the kinds below.
 *   "path"  string, required — ABSOLUTE path of the file to
 *           write; an existing file is overwritten. The parent
 *           folder must already exist (FX_ERR_NOT_FOUND).
 *   "seed"  unsigned integer, optional, default 0. Feeds the
 *           PNG pixel pattern and the "lorem" text pattern.
 *
 * Kinds and their extra keys:
 *
 *   "png"           width, height (both required, >= 1),
 *                   color_type (0, 2, 3, 4 or 6), bit_depth —
 *                   the pairs the PNG spec allows: color 0:
 *                   1/2/4/8/16; color 2: 8/16; color 3: 1/2/4/8;
 *                   colors 4 and 6: 8/16. "interlace" (bool,
 *                   default false) selects Adam7. A real PNG:
 *                   per-chunk CRC-32s, stored-block zlib IDAT,
 *                   filter 0 rows. Pixels are a deterministic
 *                   gradient/checker pattern (see README.md).
 *                   Palette images get a PLTE of 2^bit_depth
 *                   entries from the same pattern.
 *   "png_truncated" the "png" keys plus "keep_bytes": writes
 *                   only the first keep_bytes of the PNG the
 *                   same keys would produce — a corrupt-file
 *                   fixture that is a strict prefix of a valid
 *                   one.
 *   "jpeg_sample"   no extra keys. Writes a FIXED embedded
 *                   sample: a real, complete 631-byte baseline
 *                   JPEG (SOF0) of a 1x1 white pixel. This is a
 *                   fixed sample, NOT a JPEG encoder —
 *                   FixtureKit v0.1.0 has no parametric JPEG
 *                   encoder (see ROADMAP.md item 1).
 *   "jpeg_truncated" "keep_bytes": the first keep_bytes of the
 *                   fixed sample above.
 *   "wav"           freq_hz (whole hertz, >= 1), duration_ms,
 *                   sample_rate — all required; "amplitude"
 *                   optional, 0.0..=1.0, default 0.5. 16-bit
 *                   PCM mono sine tone, standard 44-byte header.
 *   "text"          lines (required, <= 1,000,000) and
 *                   line_pattern (required): "numbered"
 *                   ("line 1", "line 2", ...), "lorem" (eight
 *                   words per line from a fixed 10-word cycle,
 *                   offset by the seed), or "json-lines"
 *                   ({"line":N,"seed":S} per line). Every line
 *                   ends with '\n', including the last.
 *   "zip_stored"    entries: array of {"name", "size",
 *                   "fill_byte"? (0-255, default 0)}. A valid
 *                   ZIP with STORED entries and real CRC-32s.
 *   "tar"           entries, same shape. A valid ustar archive
 *                   (names over 100 bytes use the prefix split).
 *
 * Caps (an error names the cap when hit): PNG raw scanline
 * data 256 MiB; WAV data 64 MiB; archive entries 4096, 64 MiB
 * per entry, 128 MiB total; text lines 1,000,000.
 *
 * Result: {"path": "...", "kind": "...", "size": N,
 *          "sha256": "..."} — kind echoes the requested kind,
 * size is the bytes written, sha256 their digest, so a test
 * can pin the exact file it asked for.
 */
FX_API char *fx_generate(const char *spec_json);

#ifdef __cplusplus
}
#endif
#endif
