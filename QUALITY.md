# FixtureKit — Quality

Scope assessed: the whole v0.1.0 crate (spec parsing and dispatch, PNG encoder, stored-block zlib writer/reader, ZIP and TAR writers, WAV and text generators, embedded JPEG sample, SHA-256/CRC-32, JSON engine, C ABI, header, docs) as built and gated locally on Linux. Windows runtime behaviour is **Unverified** until the Windows CI build/link and an on-machine run exist — this file does not claim them.

## Scorecard

| Dimension | Score (0–5) | Evidence |
|---|---|---|
| Correctness | 5 | 30 tests pass, including chunk-by-chunk PNG walks with per-chunk CRC verification and stored-block inflation to the exact expected scanline length for every colour type at every legal bit depth, interlaced and not. Development cross-check with independent tools: Pillow decoded all six generated PNG variants (Adam7 included) and reproduced the documented pattern value at a checked pixel exactly; Python `zipfile` CRC check clean, `tarfile` and `wave` read their fixtures. |
| Robustness | 4 | Spec errors (unknown kind/key, forbidden PNG combination, bad entry fields, relative path, duplicate keys) are all FX_ERR_INVALID_ARG with the offending key named; oversize requests hit named caps before allocating; the FFI boundary is catch_unwind-wrapped and null/invalid-UTF-8 specs are clean errors. |
| Honesty of scope | 5 | `jpeg_sample` is labelled a fixed sample in the header, README, ROADMAP and PARITY — no parametric JPEG claim exists anywhere; corrupt files are explicit `*_truncated` kinds; `inflate_stored` documents that it reads stored blocks only. |
| Bounds & resources | 4 | Named caps on PNG raw bytes (256 MiB, checked from parameters before encoding), WAV data (64 MiB), archive entry count/size/total, and text lines, each asserted by the error paths. Generation is one in-memory buffer + one write; no threads, no handles to leak. |
| API clarity | 5 | One function; the header documents every kind, every key, the result JSON, the caps and the error contract; unknown keys are errors so the documented surface is the real surface. |
| Documentation | 4 | README has TL;DR, a C example, the pixel-pattern formula, determinism rules, limitations, and test description; ROADMAP and docs/PARITY.md included. Screenshots do not apply to a DLL. |
| Portability | 4 | Pure `std` logic, no OS APIs, no `#[link]` blocks (no platform module at all); deterministic across platforms by construction (integer math + polynomial sine + pinned timestamps). Host gate green including the windows-msvc check/clippy targets; a real Windows link is CI's job and is Unverified locally. |

Total: 31/35. No dimension below 3; no Fail.

## Gate

`~/workspace/windevkit/tools/gate.sh` — format check, clippy on host and `x86_64-pc-windows-msvc` targets with `-D warnings`, full test suite (never a filtered subset). Result is recorded in the delivery report for this kit; a green gate here is the claim, and CI's real Windows build remains the link gate.

## Known limitations (also in README)

- JPEG output is one fixed 1×1 sample; parametric JPEG encoding is ROADMAP item 1.
- PNG IDATs are stored blocks — valid but uncompressed; PNG rows use filter 0 only.
- Archive entry content is a single repeated fill byte.
- The WAV generator produces a mono sine tone only (16-bit PCM).
