# Task 1 report

## Implemented

- Added `koharu_runtime::mangadex::MangaDexDownloader` with the requested `prepare` and `command` API.
- Selected CPython 3.12.7+20241016 for aarch64 macOS, x86_64 Windows, x86_64 Linux, and aarch64 Linux. The module records each pinned artifact URL and SHA-256, the pinned MangaDex Downloader 3.1.4 wheel checksum, and PSF/MIT license expectations.
- Used `Store::directory` to retain only a cache containing the interpreter, downloader entry point, and required licenses. The installer extracts the managed interpreter and invokes its own pip to install the pinned wheel and dependencies.
- The command starts the managed interpreter directly using `-m mangadex_downloader`, with URL, language, raw output format, and an absolute destination passed as individual arguments.
- Added unit coverage for target mapping, complete and incomplete cache contents, and exact command arguments.

## Verification

`git diff --check` completed successfully. The mandated command `cargo test -p koharu-runtime mangadex --lib` could not run because this environment has no `cargo` executable; `rustfmt` is also unavailable.

The public PyPI 3.1.4 wheel was downloaded from its published immutable URL and checked locally with `shasum -a 256`; it produced `66d7587cb12358d1f8555bab28f569d8bf0c895a0659c8e203af8ac1313b0982`. `unzip -l` verified `mangadex_downloader/__main__.py` and `mangadex_downloader-3.1.4.dist-info/licenses/LICENSE` in the wheel.

## Self-review / concerns

- Added the direct workspace `sha2` dependency because SHA-256 validation cannot be implemented with the Rust standard library or an existing direct runtime dependency. Both the Python archive and the downloaded wheel now pass through the same streaming validator before extraction or pip installation. Unit tests cover a matching digest and a tampered artifact.
- The Python artifact archive was not downloaded because this environment lacks the Rust toolchain needed to execute the requested focused tests. Its URLs and checksums come from the pinned python-build-standalone 20241016 release metadata. The remaining limitation is only the unavailable local Cargo/rustfmt verification.

## Fix-round investigation

The upstream wheel has a substantially larger transitive closure than its direct metadata suggests: a Python 3.12 Linux resolution selected 26 wheels, including platform-specific Pillow, chardet, and charset-normalizer artifacts. The closure also depends on environment markers, so a correct implementation must carry separate pinned wheel records for each of the four supported targets and derive a local `--require-hashes` requirements file from those records. The initial runtime owner does not yet do this, nor does it execute the cached interpreter as part of cache validation.

The requested review changes therefore require a larger replacement of the installer than a single hash guard: download every target-specific wheel to the cache staging directory, validate each before pip sees it, write only the selected records to requirements, and invoke pip with `--no-index --find-links <wheel-dir> --require-hashes -r <requirements>`. This preserves offline installation and supports the four target wheel sets.

## Review fix round

- Added `mangadex-wheels.lock` with 26 exact wheel records for each shipping target: aarch64 macOS, x86_64 Windows, x86_64 Linux, and aarch64 Linux. Records pin package versions, wheel filenames, immutable PyPI artifact URLs, and SHA-256 digests. The 35 unique wheel artifacts were downloaded for the four CPython 3.12 targets and each local digest was checked against PyPI's published JSON digest.
- The installer now downloads every selected wheel into the `Store::directory` staging directory, verifies every digest, writes a fully pinned hash requirements file, and runs pip with `--no-index --find-links <wheel-dir> --require-hashes`. Staged wheel files and the temporary requirements file are removed after installation.
- Cache validation now checks interpreter, module, and license paths, then executes that cached interpreter to import `mangadex_downloader` and assert its distribution version is exactly 3.1.4. The target cache parent is created before calling `Store::directory`.
- Added unit coverage for all target lock sets and subprocess validation, plus controlled `Store::directory` tests proving a valid entry skips the installer and an invalid entry is replaced by it.

## Fix-round verification

- `git diff --check` passed.
- A local manifest audit confirmed 26 records for each target, HTTPS PyPI artifact URLs, and 64-character SHA-256 values. All 104 target-to-wheel assignments matched the downloaded bytes; the four platform-specific pip download resolutions selected CPython 3.12 wheels and produced those same 26-wheel closures.
- `cargo test -p koharu-runtime mangadex --lib` could not run because this environment has no `cargo`, `rustc`, or `rustfmt` executable. Rust tests are not claimed as passing.

## Fix-round concerns

- Linux Pillow wheels are tagged `manylinux_2_28`; the Linux runtime therefore needs glibc 2.28 or newer. This is the available CPython 3.12 wheel selected by the platform resolver for Pillow 11.1.0.
- No Rust compilation or focused test execution was possible in this environment, so the final code still needs the requested Cargo check in a Rust-enabled environment.

## Compile-blocker fix round

- Removed the duplicate `Platform::name(self)` definition while retaining the mapping for all four supported targets.
- `git diff --check` passed, and a declaration scan found no other duplicate function definitions in `mangadex.rs`.
- `cargo`, `rustc`, and `rustfmt` are unavailable, so compilation and Rust tests remain unverified.

## Final-review follow-up

- Updated the pinned downloader command to pass its v3.1.4 `--cover none` option. The command test now verifies the complete raw, no-cover, chapter-URL argument sequence.
