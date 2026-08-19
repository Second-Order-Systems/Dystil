---
status: verified
authority: ground-truth
verified_against: working-tree
verified_on: 2026-08-18
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

## Headless skill-bundle builds

An explicit **Build skill** action uses the existing provider-neutral automation
surface rather than a provider-native skill command. `crates/dystil-ai/src/lib.rs
:: AiRuntime::run_automation()` receives an isolated build workspace.
`crates/dystil-insights/src/skill_bundle.rs :: run_skill_bundle_build()` makes
two distinct calls: the Workflow Reconstruction Agent writes and validates
`input/WORKFLOW.md`, then the Skill Builder consumes it to create the prompt and
portable Agent Skill. The reconstruction may use Dystil's textual retrieval
tools to investigate related occurrences; the builder is limited to targeted
follow-up retrieval.

Managed Codex and Claude automation are implemented by
`crates/dystil-ai/src/lib.rs :: CliProvider::run_automation_with_model()`.
The Claude branch writes and supplies its Dystil MCP configuration while retaining
its established broad automation permissions. Pi goes through
`apps/dystil/src-tauri/src/ai_presets.rs :: pi_automation()`, whose normal
automation tool set includes the Dystil retrieval tools. None of these paths
opens a provider UI as part of a skill build.

The two production prompts are embedded resources:
`crates/dystil-insights/resources/workflow_reconstruction_prompt.md` and
`crates/dystil-insights/resources/skill_bundle_builder_prompt.md`. The first
explicitly prohibits screenshots, external application launches, and business
workflow execution; the second requires a generated skill to prefer available
connectors/MCP, then local tools, then browser/computer control at runtime.

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
