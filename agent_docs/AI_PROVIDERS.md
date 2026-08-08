---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# AI providers

Dystil does not ship an inference runtime. It connects to a provider the user
configures. `dystil-ai`'s module doc describes itself as "Provider-neutral,
privacy-bounded AI support for Dystil," where "providers receive bounded context and
can read sanitized evidence only."

## Supported providers

Configured through `ai_presets` (table) and `apps/dystil/src-tauri/src/ai_presets.rs`.

| Provider | Notes |
|---|---|
| `ollama` | Local. Default endpoint below. |
| `anthropic` | Hosted API. |
| `openai` | Hosted API. |
| `custom` | Any OpenAI-compatible endpoint. |

## Ollama — the local path

`ai_presets.rs :: normalize_endpoint` maps `"ollama"` to
`"http://localhost:11434/v1"` — Ollama's OpenAI-compatible API.

Model selection is discovery-based, not hardcoded: `ai_presets.rs ::
ai_preset_discover_models` enumerates the models the user has already pulled, and the
UI presents them for selection (`components/dystil/ai-models-settings.tsx`, combobox
labelled "Ollama model").

Consequences worth stating plainly:

- Any model Ollama can run, Dystil can use. There is no pinned model.
- No API key, no per-token cost, no network egress for inference.
- If Ollama is not running, the local path is unavailable — Dystil does not start it.

## What is not here

There is no embedded `llama.cpp`, no `llama-server` process supervised by the app,
no `LocalLlmManager`, and no bundled model download. Repo-wide greps for each return
zero matches. The app does not manage inference processes; it makes HTTP calls to an
endpoint the user controls.

Language models are never downloaded into `~/.dystil/models/`. The only model file
there is the ONNX PII redactor — see `PRIVACY_AND_TELEMETRY.md`.

## Embeddings

None. There is no embedding model and no vector index. Retrieval is FTS5/BM25. See
`ARCHITECTURE.md`.
