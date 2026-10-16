# MangaDex Project Import Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create Koharu projects from MangaDex URLs, download all chapters in a chosen language with Koharu-managed Python and MangaDex Downloader, and import pages named `chapter [chapter number] page [page number].[extension]`.

**Architecture:** `koharu-runtime` owns a cached standalone Python environment and the pinned MangaDex Downloader invocation. `koharu-app` owns MangaDex metadata discovery, the cancellable download/import job, page renaming, and project lifecycle integration; the generated bridge protocol exposes these operations to a localized start-screen wizard.

**Tech Stack:** Rust/Tokio/Tauri/Specta, reqwest, serde, Python standalone distribution, MangaDex Downloader, React/TypeScript, i18next, Vitest.

**Spec:** [docs/superpowers/specs/2026-10-16-mangadex-project-import-design.md](../specs/2026-10-16-mangadex-project-import-design.md)

## Global Constraints

- Users should not need to install Python or MangaDex Downloader themselves.
- Runtime assets and the MangaDex Downloader package must be version-pinned and validated before execution.
- Support the same OS and architecture combinations Koharu ships: aarch64 macOS, x86_64 Windows, x86_64 Linux, and aarch64 Linux.
- Download all available chapters in the selected language; chapter ranges and group filters are out of scope.
- Name each downloaded page `chapter [chapter number] page [page number].[extension]`; restart page numbering at 1 for each chapter and preserve the source image extension.
- Use a manga-specific subfolder under the user-selected output folder; leave that folder and created project intact on failure or cancellation.
- Use the existing folder-import behavior and project library; do not duplicate page decoding, source asset creation, or project commits.
- Do not import a partial download. Import only after successful download and page renaming.
- Do not add authentication, chapter range selection, archive format selection, a background queue, or automatic project deletion.
- Run only focused debug checks; end-to-end tests are out of scope unless requested.

## Review Focus

- Manga chapters with missing numeric chapter labels must use their stable ordinal in selected-language download order; test that the page names remain unique within each chapter folder.
- Chapters may share a chapter number across groups; test that their downloaded pages are associated with the correct chapter metadata before renaming.
- Cancellation during runtime setup or an active child process must stop work and leave downloaded files/project intact; test both states.
- A failed or nonzero downloader exit must never trigger import; test the lifecycle boundary.
- Existing files in the chosen destination must not be silently overwritten; test the output directory preparation behavior.

---

### Task 1: Add a managed MangaDex Downloader runtime

**Files:**
- Create: `crates/koharu-runtime/src/mangadex.rs`
- Modify: `crates/koharu-runtime/src/lib.rs`
- Modify: `crates/koharu-runtime/Cargo.toml` only if archive or process support needs an already workspace-managed dependency
- Test: `crates/koharu-runtime/src/mangadex.rs` unit tests

**Interfaces:**
- Produces: `pub struct MangaDexDownloader` with `pub async fn prepare() -> anyhow::Result<Self>` and `pub fn command(&self, manga_url: &str, language: &str, output: &Path) -> tokio::process::Command`.
- `command` invokes the managed interpreter and the pinned `mangadex-dl` module/entry point, passing the URL, selected language, raw image format, and output path as separate arguments.
- Runtime selection/install details and all version pins remain private to this module.

- [ ] **Step 1: Add failing runtime resolution tests** for all four shipping targets, validated cache contents, and an incomplete cache entry.
- [ ] **Step 2: Run the focused runtime test** with `cargo test -p koharu-runtime mangadex --lib`; confirm the new tests fail because the runtime owner is absent.
- [ ] **Step 3: Select and pin a standalone Python distribution and MangaDex Downloader release** that support the four shipping targets; record artifact URLs, hashes, and required license notices in the runtime module.
- [ ] **Step 4: Implement `MangaDexDownloader::prepare`** using `Store::directory` and Koharu's existing download/extraction helpers to install the standalone interpreter and package into the versioned cache; validate the interpreter, entry point, and license files before accepting the cache.
- [ ] **Step 5: Implement `MangaDexDownloader::command`** so arguments are passed without shell interpolation and output paths are absolute.
- [ ] **Step 6: Run the focused runtime tests**; require passing platform mapping, cache reuse, invalid-cache replacement, and exact command argument assertions.
- [ ] **Step 7: Commit** the runtime owner and tests.

### Task 2: Add MangaDex metadata discovery and page manifest

**Files:**
- Create: `crates/koharu-app/src/commands/mangadex.rs`
- Modify: `crates/koharu-app/src/commands/mod.rs`
- Modify: `crates/koharu-app/Cargo.toml` only if a direct dependency is needed for `url` or `reqwest`
- Test: `crates/koharu-app/src/commands/mangadex.rs` unit tests

**Interfaces:**
- Consumes: `koharu_runtime::http_client()` for MangaDex API requests.
- Produces: serializable `MangaDexMetadata { manga_id: String, title: String, languages: Vec<MangaDexLanguage> }`; `MangaDexLanguage { code: String, name: String }`; and a private ordered chapter manifest with chapter ID, chapter number, and page count.
- Produces: `pub(crate) async fn resolve_manga(url: &str) -> anyhow::Result<MangaDexMetadata>` and a native chapter manifest resolver used by the download job.

- [ ] **Step 1: Add failing tests** for accepted MangaDex title URLs, rejected non-title URLs, localized title selection, duplicate translated languages, empty language metadata, pagination, and missing chapter numbers.
- [ ] **Step 2: Run the focused app test** with `cargo test -p koharu-app commands::mangadex --lib`; confirm the tests fail before implementation.
- [ ] **Step 3: Implement URL parsing and MangaDex API requests** using the shared `koharu_runtime::http_client()` policy; fetch title and available translated languages, then enumerate selected-language chapter metadata in stable download order.
- [ ] **Step 4: Implement the page manifest lookup** so every downloaded chapter directory can be matched to its MangaDex chapter ID and page ordering, including duplicate chapter numbers from different groups.
- [ ] **Step 5: Run the focused app tests**; require expected title/language selection and stable manifest ordering for all fixture cases.
- [ ] **Step 6: Commit** the MangaDex metadata service and tests.

### Task 3: Implement cancellable download, rename, and import lifecycle

**Files:**
- Modify: `crates/koharu-app/src/commands/mangadex.rs`
- Modify: `crates/koharu-app/src/app.rs`
- Modify: `crates/koharu-app/src/commands/import/mod.rs`
- Modify: `crates/koharu-app/src/commands/lifecycle.rs`
- Modify: `crates/koharu-app/src/commands/mod.rs`
- Test: `crates/koharu-app/src/commands/mangadex.rs` unit tests
- Test: `crates/koharu-app/src/commands/import/mod.rs` tests

**Interfaces:**
- Consumes: metadata and chapter manifest from Task 2, runtime command from Task 1, and existing `ProjectLibrary`, `import::import`, and project commit behavior.
- Produces: `MangaDexImportEvent` states `runtime_setup | downloading | importing | finished | failed | cancelled`, with optional stage progress and user-readable error; command functions `start_mangadex_import(...)` and `cancel_mangadex_import(job_id)`.
- `start_mangadex_import` validates its inputs, registers a job, spawns the work, and immediately returns its job ID; the Tauri channel reports progress and the terminal event so the frontend can cancel concurrently.
- Native command signatures: `resolve_mangadex(url: String) -> Result<MangaDexMetadata>`, `pick_mangadex_output_folder() -> Result<Option<String>>`, `start_mangadex_import(name: String, url: String, language: String, output: String, on_event: Channel<MangaDexImportEvent>) -> Result<String>`, and `cancel_mangadex_import(job_id: String) -> Result<()>`.
- Produces: a reusable `import_folder(path: &Path) -> anyhow::Result<Vec<import::Page>>` helper for recursive supported-image enumeration and decoding, shared by the existing folder dialog and the MangaDex lifecycle.

- [ ] **Step 1: Add failing tests** for recursive image enumeration, naming with numeric and missing chapter numbers, page numbering restart, original extensions, duplicate chapter-number disambiguation, and refusal to rename unrelated files.
- [ ] **Step 2: Run focused tests** with `cargo test -p koharu-app commands::mangadex --lib` and `cargo test -p koharu-app commands::import --lib`; confirm failures before implementation.
- [ ] **Step 3: Extract `import_folder`** from `lifecycle::import` into `commands/import/mod.rs`; keep the existing user-selected folder command behavior and route its file list through the shared importer.
- [ ] **Step 4: Implement job state and process ownership** so the app can cancel the active Tokio child during runtime setup or download and reliably report terminal state.
- [ ] **Step 5: Capture downloader output** and relay CLI progress when its pinned output format exposes it; otherwise emit stage progress only, and return useful nonzero-exit diagnostics without logging unrelated environment data.
- [ ] **Step 6: Implement destination preparation and page renaming** inside a manga-specific subfolder; preserve extensions, use chapter ordinals when chapter numbers are unavailable, and avoid overwriting existing files.
- [ ] **Step 7: Implement the lifecycle** to create/open the project with the selected editable name, prepare the runtime, run the downloader, rename pages from the manifest, and only then import the folder in one existing project update. On failure/cancel, retain project and files and skip import.
- [ ] **Step 8: Run focused app tests**; require no import on cancellation/failure, intact output files, successful project import on a zero exit, and shared folder-import parity.
- [ ] **Step 9: Commit** the native download/import lifecycle and tests.

### Task 4: Expose the typed native protocol

**Files:**
- Modify: `crates/koharu-app/src/commands/mangadex.rs`
- Modify: `crates/koharu-app/src/commands/mod.rs`
- Modify: `crates/koharu-app/src/bin/generate.rs` only if generation needs a new command registration path
- Generated: `packages/bridge/src/protocol.ts`
- Test: `crates/koharu-app/src/commands/mangadex.rs` command tests

**Interfaces:**
- Consumes: `MangaDexMetadata`, `MangaDexImportEvent`, and the native lifecycle commands from Tasks 2 and 3.
- Produces: generated `commands.resolveMangadex(url)`, `commands.pickMangadexOutputFolder()`, `commands.startMangadexImport(...)`, and `commands.cancelMangadexImport(jobId)` with Specta-derived types; progress uses a Tauri `Channel<MangaDexImportEvent>`.

- [ ] **Step 1: Add command-level tests** for invalid URLs, unavailable languages, job creation, cancellation IDs, destination-folder cancellation, and terminal event delivery.
- [ ] **Step 2: Register the commands and event types** in `commands::bindings()`; implement `pick_mangadex_output_folder` with the existing native `AsyncFileDialog::pick_folder` pattern and return the selected path as an optional string. Do not hand-edit the generated bridge.
- [ ] **Step 3: Generate the bridge** with the repository's `cargo run -p koharu-app --bin generate` workflow and check the generated signatures match the native types.
- [ ] **Step 4: Run focused command tests** with `cargo test -p koharu-app commands::mangadex --lib`; require passing error serialization and event lifecycle checks.
- [ ] **Step 5: Commit** native protocol registration and generated bridge bindings.

### Task 5: Build the start-screen MangaDex wizard

**Files:**
- Create: `packages/koharu/components/start/MangaDexProjectDialog.tsx`
- Modify: `packages/koharu/components/start/StartView.tsx`
- Modify: `packages/koharu/public/locales/*/translation.json`
- Test: `packages/koharu/tests/components/start-view.test.tsx` or the existing welcome/start-view component test

**Interfaces:**
- Consumes: generated `commands.resolveMangadex`, `commands.pickMangadexOutputFolder`, `commands.startMangadexImport`, and `commands.cancelMangadexImport` from Task 4.
- Produces: a wizard that resolves title/languages from a URL, pre-fills an editable project name, selects a language and output folder, starts/cancels the job, and displays stage/progress/error states.

- [ ] **Step 1: Add failing component tests** for title prefill/editing, language selection, required destination, progress display, cancellation, and actionable failures.
- [ ] **Step 2: Run the focused component test** with `bun run --filter @koharu/app test -- start-view`; confirm the new tests fail before implementation.
- [ ] **Step 3: Implement the wizard dialog** using the generated destination-folder picker; keep file-system access on the native side.
- [ ] **Step 4: Connect the wizard from StartView** and refresh project/page queries after successful import.
- [ ] **Step 5: Add all new UI strings** to the locale files, using the existing English copy as the source and maintaining each locale's JSON structure.
- [ ] **Step 6: Run the focused component and localization checks**; require all wizard tests to pass and every locale file to parse.
- [ ] **Step 7: Commit** the wizard, localization, and focused tests.

### Task 6: Final focused integration review

**Files:**
- Review: all files changed by Tasks 1–5
- Test: focused Rust and frontend tests listed below

- [ ] **Step 1: Run focused Rust checks** for `koharu-runtime` MangaDex runtime tests and `koharu-app` MangaDex lifecycle/import tests in the debug profile.
- [ ] **Step 2: Run focused frontend checks** for the start-screen MangaDex wizard and locale JSON validity.
- [ ] **Step 3: Review the generated diff** for version pins, runtime license notices, user-selected folder safety, event cancellation, page naming, and no partial imports.
- [ ] **Step 4: Commit** any final fixes with their owning task's tests.
