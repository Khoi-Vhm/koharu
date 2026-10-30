# Koharu Translator

koharu-translator provides one segment-preserving translation interface for
local GGUF models and hosted providers.

Model metadata is provider-neutral: each model has a stable provider ID, a
display name, and (for local models) selectable GGUF quantizations. Every
provider module owns its connection config, defaults, request mapping, and
model catalog or discovery.

The crate owns provider connectivity and translation orchestration. Provider settings such as base URLs
are persisted under `[providers]`; credentials remain in the platform keychain.
Translation model selection, target language, instructions, and generation
options belong to the caller. `koharu-pipeline` owns those values as the
configuration of its translation processor.

`Translator` is an execution engine. A caller supplies a `ModelSelection`,
`GenerationConfig`, and `TranslationRequest` for each operation. The engine
keeps the selected local model resident when possible and reads live provider
connection settings without owning workflow configuration.

The Codex provider shares OAuth authorization and transport with the Agent through
`koharu-codex`. Sign in from the desktop Agent panel, then select a Codex model
for translation. Translation requests carry no Agent history or project tools,
and reject missing, duplicate, or unknown segment IDs before returning results.

Vietnamese requests to LLM backends automatically run page analysis, translation,
and a final review of forms of address. This uses three model calls per nonempty
request and the selected generation settings for each call. Image analysis still
requires vision to be enabled and supported. DeepL, Google Cloud Translation and
Caiyun retain their native single-pass translation APIs.

Callers can supply preceding source/translation pairs, source-image polygons and
validated dialogue memory. `TranslationResult` contains the reviewed translations,
provider ID and new memory. Memory is a compact narrative with stable character
identities, directional address rules, evidence, uncertainties and per-utterance
speaker/listener decisions. It is a model hypothesis, not verified character data.
The pipeline persists it atomically with full-page translations and validates it
against preceding page order, source images, text, translations and configuration
before reuse. Partial-page translations use context without replacing full-page memory.
