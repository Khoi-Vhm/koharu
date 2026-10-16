# Task 3: MangaDex download and import lifecycle

## Implementation

- Added a managed MangaDex import job with stage and terminal events, cancellation signalling, and active child-process termination.
- Created or opened the requested project before runtime setup, downloaded to a new manga-specific output folder, and preserved existing destinations by refusing to overwrite them.
- Matched downloader chapter folders by manifest chapter ID, verified each chapter page count, renamed pages with the manifest chapter number and original extension, and rolled back staged filenames on a rename error.
- Extracted recursive supported-image discovery into `import::import_folder` and reused the existing import commit path for both folder imports and MangaDex imports.
- Added an async per-job transition gate that linearizes cancellation against the decoded-page commit without holding a parking-lot guard across the commit await.
- Restored folder-import traversal errors to the previous skip-and-warn behavior.
- Kept command registration and generated bridge changes out of scope for Task 4.

## Focused coverage

- Recursive folder image discovery and decoding.
- Chapter-ID association with duplicate chapter labels, page-number resets, extension preservation, unrelated-file refusal, output-folder no-overwrite, and cancellation before setup/download work continues.
- The commit transition rejects cancellation after decoding and records completion, plus a missing-folder traversal error that is skipped rather than aborting the import.

## Checks

- `git diff --check` passed.
- `cargo test -p koharu-app commands::mangadex --lib` could not run: `cargo` is unavailable in this environment.
- `cargo test -p koharu-app commands::import --lib` could not run: `cargo` is unavailable in this environment.
- `rustc` and `rustfmt` are also unavailable, so Rust compilation, test execution, and formatting were not verified here.

## Commit

- `9d64feac feat(app): add MangaDex import lifecycle`
- `b516e097 fix(app): guard MangaDex import cancellation`
- `1a8e80c6 fix(app): serialize MangaDex import completion`

## Concerns

- The native commands are implemented but intentionally not registered in `commands::bindings()`; Task 4 owns protocol registration and bridge generation.

## Final-review follow-up

- The pinned downloader's raw output uses human-readable volume and chapter folder names, so ID matching those folder names was replaced. Each manifest chapter now runs in its own chapter-ID output root using `https://mangadex.org/chapter/{id}`. Koharu recursively orders pages within that root, discards a residual cover image defensively, validates the manifest page count, and retains duplicate chapter labels in their separate ID roots.
- The download/import boundary is unchanged: a command failure, page-count mismatch, or cancellation exits before `import_pages` receives any pages. Downloaded files and the created project remain available for inspection.

## Final-review ordering follow-up

- After every chapter download and rename succeeds, Koharu imports each chapter-ID root in chapter-manifest order and concatenates the page vectors before the existing single commit. This avoids relying on the parent directory's traversal order when separate chapters have the same renamed basename.
- Added a deterministic regression with reverse-sorting UUID roots, identical `chapter 1 page 1.png` names, and distinct valid PNG bytes. It asserts that the assembled page vector follows the manifest order.
- Cargo, rustc, and rustfmt remain unavailable, so the focused Rust regression could not run in this environment.
