# Task 5: MangaDex start-screen wizard

## Implementation

- Added a StartView entry point and `MangaDexProjectDialog` for resolving a MangaDex title URL, editing its prefilled project name, choosing a chapter language, and choosing a native-managed output folder.
- The dialog reports an empty language list, native picker cancellation, and actionable command failures. It uses the native channel stages as visible import progress and keeps cancellation available until a terminal event arrives.
- A finished import closes the dialog and invalidates the existing project, pages, and page queries so the native-created/opened project is shown.
- Added the MangaDex UI strings to every shipped locale.
- Moved the existing StartView suite to `start-view.test.tsx` and added focused coverage for title editing, language selection, required destinations, progress/cancellation, and native error messages.

## Checks

- `git diff --check` passed.
- Every `packages/koharu/public/locales/*/translation.json` file parsed successfully with Ruby's JSON parser.
- `bun run --filter @koharu/app test -- start-view` could not start because this environment has no `bun` executable. `node`, `npm`, and `pnpm` are also unavailable.

## Generated bridge limitation

`packages/bridge/src/protocol.ts` remains unchanged because it is generated source and the required Cargo/Rust generator is unavailable. The dialog uses a local expected view of the four Task 4 commands and their registered native payloads, so TypeScript checking cannot complete until `cargo run -p koharu-app --bin generate` refreshes the protocol.
