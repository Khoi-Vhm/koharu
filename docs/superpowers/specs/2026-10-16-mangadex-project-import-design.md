# MangaDex project creation and chapter import

## Goal

Let users create a Koharu project from a MangaDex manga URL, choose one chapter language, download all available chapters in that language, and import the downloaded pages into the project. Users should not need to install Python or MangaDex Downloader themselves.

## Current behavior

Koharu creates projects in its project library under `Documents`, opens the new project immediately, and imports images from a folder selected with a native dialog. Folder import recursively finds supported image files and adds them as source pages. Koharu also manages versioned runtime packages in an application cache, downloading packages on first use.

The requested downloader, [mansuf/mangadex-downloader](https://github.com/mansuf/mangadex-downloader), is a Python CLI. Its documented requirements include Python 3.10 or newer; it supports MangaDex URLs, language selection, and raw image output.

## User flow

1. On the start screen, choose the MangaDex project creation flow.
2. Enter a MangaDex manga URL and a Koharu project name.
3. Koharu validates the URL and fetches available chapter languages from MangaDex. The user chooses one language.
4. The user chooses an output folder for the downloaded chapter files.
5. Koharu creates and opens the project, downloads every available chapter in the selected language as raw images, and shows progress and any error.
6. After a successful download, Koharu imports supported images recursively from the selected output folder into the open project using the existing source-page import behavior.

The selected output folder is user-owned download output. The Koharu project remains in the Koharu project library. The first version downloads all chapters available in the selected language; chapter ranges and group filters are out of scope.

## Design

### Managed downloader runtime

Add a MangaDex-specific runtime owner in the native runtime layer. It resolves a version-pinned standalone Python runtime for each supported OS and architecture, then installs a version-pinned MangaDex Downloader package and its required dependencies into Koharu's runtime cache. Runtime resolution follows the existing install-on-first-use cache pattern. The app invokes the downloader through its documented CLI with explicit manga URL, selected language, raw-image output, and destination arguments.

Do not require a system Python, write the runtime into the application bundle after installation, or copy runtime ownership into the frontend. Keep platform selection, versions, install validation, paths, and downloader invocation inside the runtime owner. Preserve upstream licensing notices for both the Python distribution and MangaDex Downloader in the packaged or cached runtime metadata as their licenses require.

### MangaDex discovery and download

Add a native MangaDex service responsible for parsing accepted MangaDex manga URLs, retrieving title/language metadata needed by the UI, and starting the downloader process. Use Koharu's existing HTTP client conventions and asynchronous command flow. The frontend receives only typed metadata and download events, never performs MangaDex API calls or starts processes itself.

Represent the workflow as a cancellable job with explicit discovery, runtime setup, download, import, completion, and failure states. Forward progress when the CLI exposes it; otherwise report stage-level progress and process output without inventing percentages. Capture and return useful CLI errors while avoiding logging credentials or unrelated environment data.

### Project lifecycle and import

Create the project using the existing project library and lifecycle owner, then use the existing page import pipeline on the selected output folder after download succeeds. Refactor the folder enumeration/import boundary only as needed to let both the native folder dialog and MangaDex flow share the same import behavior. Do not duplicate image decoding, supported-format handling, source asset creation, or project commits.

If discovery, runtime setup, or download fails, keep the created project available and do not import a partial result automatically. If the user cancels, stop the child process and leave the project and downloaded files intact so the user can inspect or retry. On success, import only files recognized by Koharu's existing importer.

### UI and protocol

Add a start-screen flow for URL, project name, language, and destination selection, with clear busy/error state and download progress. Expose the smallest typed command/event protocol needed for language discovery, job start/cancel, and completion. Keep localized strings aligned with the existing frontend localization structure.

## Constraints and failure behavior

- Support the same OS and architecture combinations Koharu ships. Runtime assets must be pinned and validated before execution.
- Respect MangaDex API rate limits through the upstream downloader; do not implement a second downloader in Koharu.
- Do not overwrite existing output files without the downloader's explicit safe behavior. Surface invalid URLs, empty language lists, unavailable runtime assets, process startup errors, nonzero exits, cancellation, and import errors to the user.
- Import is a single project update consistent with current folder import semantics. Sorting and page naming follow the existing importer behavior.
- No user account, authentication flow, chapter range selection, archive format selection, background queue, or automatic project deletion is included.

## Verification criteria

- A valid MangaDex manga URL yields selectable available languages.
- Choosing a language downloads all available chapters for it to the chosen directory through the pinned upstream CLI.
- Koharu imports the downloaded pages into the created project in the same manner as folder import.
- Runtime setup works without system Python and is reused from the cache on later runs.
- Invalid URLs, no matching chapters, runtime failures, downloader failures, cancellation, and import failures produce actionable UI states without importing incomplete downloads.
- The workflow is covered by focused native and frontend checks; end-to-end tests are out of scope unless requested.

## Open implementation detail

Select the standalone Python distribution and pin the MangaDex Downloader version during implementation after confirming current upstream release compatibility and available assets for Koharu's shipping platforms. Keep this choice behind the runtime owner so it does not affect the UI or project protocol.
