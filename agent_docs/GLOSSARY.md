---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. If a citation no longer resolves, this document is
> wrong.

# Glossary — marketing term → real type

`public_docs/` and the README use language chosen to explain Dystil to people.
Some of those words have no corresponding type in the code. This table is the
translation layer. **Read it before assuming a noun you saw in marketing copy exists
as a struct.**

| Term you may see | What exists in code | Notes |
|---|---|---|
| **Work card** | *nothing* | Explanatory concept only. Zero occurrences in `.rs`/`.ts`/`.tsx`. Used in narrative to describe how observed work is structured. **Never implement against it.** The nearest real unit is a surface visit. |
| **Work log / work trail** | *nothing* | Same as above — candidate outward-facing names for the same concept. |
| **Surface visit** | `dystil-work-index` | The real unit. "Deterministic construction of compact surface visits from captured frames." ~18 references. |
| **Worth fixing** | `dystil-insights`, `components/dystil/pages/worth-fixing.tsx` | Real, both backend and UI. Findings with evidence. |
| **Ready to use** | `components/dystil/pages/ready-to-use.tsx`, `dystil-insights` via `ready_to_use_commands.rs` | Real. Kept, reusable artifacts. **Not** `dystil-automation` — that crate backs `automation_commands.rs`, a separate subsystem the UI never calls. |
| **Ask for fix** | `components/dystil/pages/ask-for-fix.tsx` | Real UI surface. |
| **Evidence** | `dystil-retrieval` | Real. Addressed by stable `evidence_id`. |
| **Memory** | *no single type* | Loose umbrella for capture + work index + retrieval. Not a module. |
| **Local model / local inference** | Ollama connector | Real, but **not bundled**. `ai_presets.rs :: normalize_endpoint` → `http://localhost:11434/v1`. Dystil calls Ollama; it does not run models itself. |
| **Semantic search / embeddings** | *nothing* | Stated future direction. Zero `embedding` matches in `.rs`. Retrieval is FTS5/BM25. |
| **Multiplayer / team mode** | partial | `dystil-sync`, `dystil-protocol`, `dystil-engine` exist and are optional. The peer Q&A flow described in marketing is direction, not shipped behaviour — verify before claiming any specific capability. |

## Terms that were removed

These appeared in earlier documentation and describe nothing that has ever existed
in this repository. They were deleted rather than relabelled. If you encounter them
in an old branch, a stale doc, or a cached summary, treat the source as unreliable:

- `llama.cpp` / `llama-server` / `LocalLlmManager` — no bundled inference runtime
- Named generator and embedder models with fixed sizes
- Dedicated inference ports
- A work-card JSON schema with `activity_type` / `entities` / `outcome` fields
- Vector similarity search

Each returns zero matches repo-wide.

## Rule

If you need a noun for a thing, check this table first. If the thing is real and
missing here, add it. If the thing is not real, it belongs in `public_docs/`, in
prose, and never in a schema.
