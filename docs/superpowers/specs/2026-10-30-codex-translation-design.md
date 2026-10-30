# Codex translation

Approved in chat on 2026-10-30.

Codex becomes a selectable translation provider in Koharu's existing translation
model picker and processing pipeline. It reuses the Agent's ChatGPT sign-in and
account model catalog, with translation model selection independent of Agent
configuration. Each request contains the existing manga translation prompt, OCR
segments, target language, custom instructions, and optional source page image.
Requests have no project tools or Agent conversation history. Responses must
contain exactly one translation per segment, restored to input order by ID.

Extract authentication, model discovery, streaming, and wire protocol into
`koharu-codex`, consumed directly by Agent, translator, and desktop account
commands. Keep project-specific context and tool results in `koharu-agent`.
Authentication refresh must be synchronized across both consumers. Codex uses
OAuth rather than an API key; provider metadata owns that distinction.

The alternative was translating through Agent chat alone. The user chose normal
pipeline integration so Codex can be used by Translate and full processing.
No new sign-in flow, shell process, or translation UI is needed. Update model
discovery after sign-in/sign-out and document the shared account behavior.

Use focused debug tests and regenerate the TypeScript bridge from Rust. Do not
run full suites or end-to-end tests, and do not make authenticated inference
requests as part of verification.
