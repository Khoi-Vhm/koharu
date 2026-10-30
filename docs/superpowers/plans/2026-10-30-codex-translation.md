# Codex Translation Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task by task.

**Goal:** Make signed-in Codex models selectable for manga translation.

**Architecture:** Extract the existing Codex transport into a shared crate. Add a
Codex translator using existing prompts and image encoding, validate segment IDs,
and reuse the current model picker and account UI.

**Tech Stack:** Rust, reqwest streaming, serde, Specta, React/TypeScript.

**Spec:** `docs/superpowers/specs/2026-10-30-codex-translation-design.md`

## Global Constraints

- Reuse existing ChatGPT sign-in; keep Agent and translation configurations separate.
- No tools or conversation history in translation requests.
- Validate all returned IDs exactly once; preserve input order.
- No compatibility re-exports; update every in-repository consumer.
- Focused debug checks only; generate rather than edit the bridge.

## Review Focus

- Signed-out accounts must yield no Codex models and actionable translation errors.
- Concurrent Agent and translation requests must not race token refresh.
- Text-only models must not receive page images.
- Missing, duplicate, invalid, or malformed translation output must fail before scene commits.
- Signing in/out must refresh cached translation choices.

### Task 1: Shared Codex client

**Files:** `crates/koharu-codex/{Cargo.toml,src/*}`, `crates/koharu-agent/src/*`, desktop Agent imports, workspace dependencies.

**Interfaces:** `koharu_codex::{Codex, CodexModel, Reasoning, Control, Request, Delta, Turn, Tool, ToolCall, message}`; retain project context and tool image encoding in Agent.

- [x] Add and run a failing protocol test: an empty tool set disables tool calls.
- [x] Extract existing client and wire types, update all consumers, synchronize authentication refresh, and carry vision capability from the catalog.
- [x] Run focused shared-client debug tests covering protocol, catalog, and stream completion/failure.

### Task 2: Pipeline translation provider

**Files:** translator provider metadata, remote Codex backend, prompt result types, desktop provider preferences.

**Interfaces:** `CodexConfig`, provider-owned `requires_api_key()`, remote model listing and translation through shared `Codex`.

- [x] Add decoder tests for reordered valid IDs and rejected missing/duplicate/out-of-range IDs and malformed JSON; verify the missing implementation fails.
- [x] Add request tests for translation instructions, language, image context, and no tools/history.
- [x] Implement Codex model discovery and translation, respecting advertised image and reasoning support.
- [x] Update provider credential handling to consume provider metadata.
- [x] Run focused Codex translation debug tests.

### Task 3: Desktop integration and verification

**Files:** Agent panel account refresh, generated bridge, English/Japanese/Chinese Agent setup and translation docs.

- [x] Refresh translation models after account login/logout.
- [x] Regenerate bridge with `cargo run -p koharu-app --bin generate` (debug).
- [x] Check TypeScript and formatting; inspect final diff and perform independent review.
- [x] Report verified checks and any live-account limitation.
