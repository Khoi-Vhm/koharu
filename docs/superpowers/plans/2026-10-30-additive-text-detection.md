# Additive Text Detection Implementation Plan

**Goal:** Recover missed English and Japanese text on rerun while preserving existing text, geometry, translations, and cleanup edits.

**Architecture:** Keep RF-DETR responsible for page layout and primary text instances. Supplement uncovered text with the existing commit-pinned PP-OCRv6 line detector. Detection merges into the saved scene instead of skipping or replacing existing regions. Existing user-authored OCR and translations retain their established protection. Cleanup processes only pixels not already covered by the saved cleanup overlay.

**Tech Stack:** Rust, existing koharu-ml models, image/imageproc, scene patches.

## Constraints

- No new dependencies or compatibility paths; no generated sources edited.
- Work in the current workspace to preserve the in-progress OCR fix and unrelated user changes.
- Focused debug tests and model diagnostics only; no full suites or full translation runs.
- Keep the user's saved project unchanged during diagnostics.

## Tasks

- [x] Add failing regressions for rerun preserving authored content, adding uncovered regions, and avoiding duplicate regions on a second run.
- [x] Add PP-OCRv6 supplementation with polygon masks, excluding lines already covered by primary text or sound-effect regions.
- [x] Replace detection's page-wide skip/replacement behavior with an additive merge; preserve masks and existing bubble associations.
- [x] Ensure automatic cleanup handles new uncovered mask pixels while retaining the existing overlay.
- [x] Run focused debug tests and verify detection on original page 14 using an in-memory scene; inspect final diff.

## Verification

- Focused detector tests cover preserved source/translation edits, partial blocks, repeated runs, and separate parallel tilted rows.
- Cleanup tests cover overlay preservation and excluding prior opaque pixels from automatic masks.
- OCR regression covers quadrilateral rectification.
- Original page 14: 24 text regions before and after repeat detection; representative OCR outputs `ダメージ軽減：LV2` and `Damage reduction`.
- Additional scope: rectify tilted OCR crops after the real model diagnostic showed a neighboring row leaking into recognition.
- 48 focused pipeline-stage tests pass; debug desktop build succeeds (existing linker unwind-table warning).
- Updated debug app and frontend restarted; page 14 reopened and its normal four-stage Vietnamese run started.
- CEF remote inspection is blocked by system policy; desktop inspection uses the native window.
- Original source diagnostics remained in memory. The subsequent app rerun is limited to the user's selected page 14.

## Desktop rerun

The selected page-14 app run saved 25 recognized contents (the saved project retained its existing five regions, unlike the fresh 24-region diagnostic). Newly recognized examples include the complete status panel, `ダメージ軽減：LV2`, and `Physical Enhancement`. The existing five translations remain present while Codex translation runs at 75%. Unrelated OpenAI-compatible/LM Studio catalog errors do not establish a Codex job failure.
