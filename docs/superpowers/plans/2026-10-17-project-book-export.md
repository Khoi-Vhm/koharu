# Project EPUB export (PDF deferred)

The user approved the in-chat design and implementation on 2026-10-17.

## Goal and constraints

Export every project page, in snapshot order and with current rendered edits, to one fixed-layout EPUB. Preserve full resolution and lossless pixels. Reuse native rendering, process one page at a time, encode directly into the archive on disk, and publish a completed temporary file only after success. Keep existing uncommitted sequential-export and renderer-cache changes. The user explicitly deferred PDF to phase two and required avoiding project-size-dependent pixel memory.

## Implementation

- [x] Add a small `koharu-epub` crate owning EPUB packaging and temporary-file publication, independent of desktop/ML dependencies. Test output order, dimensions, pixels, XML labels, empty books, and failure cleanup.
- [x] Implement EPUB 3 packaging with stored PNGs, one fixed-layout XHTML document per page, manifest, ordered spine, navigation, identifier, language `und`, and current UTC modification metadata.
- [x] Integrate the EPUB save dialog and sequential book writing into the existing command. Regenerate the bridge through its authoritative generator.
- [x] Add menu entries, extend the focused menu test, and correct export documentation to describe whole-project scope.
- [x] Run focused debug Rust tests and UI export tests; check app integration and formatting. Measure a synthetic many-page export in a separate process to check that pixel memory does not grow with page count. PDF comparison belongs to phase two.

## Verification scope

No full suites, release builds, or end-to-end tests. Any unavailable runtime, compiler, or benchmark input must be reported rather than replaced with an unsupported completion claim.

## Verification results

- `cargo test -p koharu-epub --test export --offline`: 4 passed, manual memory probe ignored by default. Round-trip PNG pixel difference: zero, including alpha.
- Focused frontend export-menu tests: 4 passed (PNG, PSD, CBZ, EPUB).
- `cargo run -p koharu-app --bin generate --offline`: succeeded using the matching existing CEF SDK through `CEF_PATH`; generated bridge includes `epub`. Linker emitted its existing large unwind-table warning.
- TypeScript `--noEmit`, targeted Rust/UI formatting, and `git diff --check`: passed.
- Independent read-only review: no actionable correctness findings.

### Writer-only memory probe

Debug build, macOS arm64, synthetic 1536×2048 RGBA input allocated before timing. Each measurement ran the compiled `streaming_memory_probe` test in a separate process with `KOHARU_EPUB_PROBE_PAGES` set to the page count; Python `resource.getrusage(RUSAGE_CHILDREN).ru_maxrss` recorded peak RSS in bytes.

| Pages | Time | Output bytes | Peak RSS bytes |
| --- | --- | --- | --- |
| 10 (baseline) | 11.06 s | 125,373,293 | 21,987,328 |
| 100 | 109.53 s | 1,253,722,266 | 22,298,624 |

Ten times the pages/output added approximately 0.30 MiB peak RSS. This measures PNG encoding and archive packaging, not desktop/GPU rendering; it does not guarantee freedom from system-wide memory pressure. EPUB metadata and ZIP entries still grow with page count. The application rejects oversized page surfaces before rendering and serializes export jobs. PDF performance comparison remains deferred to phase two.
