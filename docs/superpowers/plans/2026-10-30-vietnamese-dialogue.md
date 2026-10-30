# Automatic Vietnamese dialogue translation

**Goal:** Automatically resolve Vietnamese forms of address using page images, preceding dialogue and persistent relationship context.

**Architecture:** The translator owns an analysis → translation → review workflow for Vietnamese LLM requests. Analysis is a compact narrative containing stable character identities, directional address rules, evidence and uncertainty; it uses the existing structured segment transport. The pipeline owns scene context, source-region coordinates and a page extension component storing the resulting memory. Native translation services retain their supported single-pass behavior.

**Constraints:** Reuse existing workspace dependencies; no compatibility paths, end-to-end runs or non-debug builds. Preserve user-authored translations and segment identity. Never infer gender or hierarchy from appearance alone, or equate anger with derogatory address.

## Tasks

- [x] Add focused tests for three-pass sequencing, shared image/context, empty or malformed output, and single-pass behavior. Implement the translator workflow and task prompts; update API consumers.
- [x] Collect preceding dialogue and validated cumulative memory from document order. Persist memory atomically with translations; invalidate it when source, translations, page order, images, inherited memory or translation instructions change. Add focused persistence/invalidation tests.
- [x] Verify existing ordered translation scheduling, document automatic behavior and additional model calls, run focused debug tests and inspect the final diff.

**Review focus:** Sparse source-only preceding pages; partial-page reruns; manually edited translations; stale inherited context; malformed model output; providers without prompt support.

**Execution:** Inline, authorized by the user's request to implement. Existing feature branch is used; unrelated in-progress Codex-provider changes are preserved. No commits or external API calls are required.

**Review:** Independent review identified inherited-memory invalidation, presentation order and concurrent source/geometry edits. These were corrected with regression coverage. Committers now validate input evidence against the latest snapshot under their commit lock; unrelated inpainting remains composable.

**Validation:** Focused translator dialogue/prompt/image tests and pipeline translation tests pass; `cargo check -p koharu-translator -p koharu-pipeline --bins --offline` passes in debug. Full app checking is blocked by cef-dll-sys attempting to download CEF with unavailable DNS. No live-model or end-to-end quality evaluation was run.

**Follow-up fix:** The broad commit-time document hash could reject fields that were never sent to the translator. Input preparation is now shared by translation and validation; configuration and scope travel with the stage output. Commit guards compare actual prompt data, target identities and the attached image. Durable memory fingerprints exclude presentation frames and geometry provenance. Four focused translation tests and `cargo check -p koharu-pipeline --bins --offline` pass; independent review found no remaining actionable issues.
