# FixtureKit Roadmap

## v0.1.0 (current)
Deterministic fixture generation behind one C ABI call, `fx_generate(spec_json)`: a real PNG encoder (colour types 0/2/3/4/6 at every spec-legal bit depth, Adam7 interlacing, per-chunk CRC-32, stored-block zlib IDAT, a documented gradient/checker pattern); `png_truncated`/`jpeg_truncated` strict-prefix corrupt variants; a fixed embedded 1×1 baseline JPEG sample; a 16-bit PCM mono WAV tone with a polynomial sine (byte-identical on every OS — no libm); numbered/lorem/json-lines text; stored ZIP and ustar TAR writers with real CRCs/checksums and pinned timestamps. Result JSON reports path, kind, size and SHA-256. Zero dependencies.

## Next, in order
1. **Parametric JPEG encoder**: a real baseline DCT encoder (colour conversion, 8×8 DCT, quantisation, Huffman entropy coding) so `jpeg` can take width/height/quality like `png` does. It lands only as a real encoder whose output independent decoders read — the fixed `jpeg_sample` stays until then, and stays labelled a sample. This is the honest completion of the JPEG story, not a widening of the sample trick.
2. **DEFLATE compression for PNG IDAT**: fixed-Huffman compressed blocks (a real compressor, still zero-dependency) so large fixtures stop costing raw-pixel bytes. The stored-block writer stays selectable (`"compression":"stored"`) because some decoder tests specifically want stored blocks.
3. **More fixture kinds, as real generators**: BMP (24-bit, trivially a real encoder), WAV stereo + a square-wave option, and a minimal valid MP4 sample (fixed sample rules apply: embedded real bytes, labelled as such, until a real muxer earns its place).
4. **PNG encoder filters**: optional per-row filter selection (Paeth) for fixtures that exist to exercise decoder filter paths — the decoder-side coverage argument, made only when a consumer needs filter variety in generated files.

## Explicitly not planned
- Random/non-seeded content: a fixture whose bytes cannot be reproduced from its spec is a blob with extra steps. Everything stays a pure function of spec + seed.
- A general-purpose DEFLATE *decoder* in this crate: `zlib::inflate_stored` exists only to verify this kit's own stored-block output in tests and says so.
- Fake or partial files presented as valid: corrupt fixtures are explicit kinds (`*_truncated`), never a flag that quietly breaks a valid one.
