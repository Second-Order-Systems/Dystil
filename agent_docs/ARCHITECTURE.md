---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. Every claim below cites a path plus a symbol name
> or verbatim quote. If a citation no longer resolves, this document is wrong — fix
> it or file it. See `AGENTS.md` for the source-of-truth rule.

# Architecture

Dystil is a Tauri v2 desktop application. A Next.js frontend renders the UI; a Rust
backend does capture, redaction, storage, indexing, and provider calls. Everything
in the core loop runs on the user's machine.

## Shape

```
Next.js frontend  (apps/dystil/app, components)
        │  Tauri commands (generated bindings)
Rust backend      (apps/dystil/src-tauri)
        │
        ├── dystil-capture     accessibility trees, UI events, windows, optional pixels
        ├── dystil-redact      text-only privacy boundary
        ├── dystil-storage     SQLite
        ├── dystil-work-index  deterministic surface visits
        ├── dystil-retrieval   sanitized evidence projection
        ├── dystil-insights    Worth Fixing
        ├── dystil-ai          provider-neutral AI
        ├── dystil-automation  reusable automation definitions
        └── dystil-mcp         capabilities over MCP

optional: dystil-engine, dystil-sync, dystil-protocol, dystil-telemetry
```

## Crates and their boundaries

The workspace member list lives in the root `Cargo.toml` under `members = [`.

| Crate | Responsibility (from its own module doc) |
|---|---|
| `dystil-capture` | "Platform-neutral capture orchestration for Dystil." Screen pixels are an *optional* evidence source. |
| `dystil-redact` | "Dystil-owned text privacy boundary." Explicitly: "This crate intentionally handles text only. Images are never inspected." |
| `dystil-storage` | "Dystil-owned SQLite bootstrap for capture data." Owns the schema. |
| `dystil-work-index` | "Deterministic construction of compact surface visits from captured frames." |
| `dystil-retrieval` | "Agent-safe retrieval over Dystil's sanitized evidence projection." Owns evidence identifiers, response budgets, dedup, deep links. |
| `dystil-insights` | "Local Worth Fixing backend." Owns admission, compaction, durable jobs, ranking, dispositions. |
| `dystil-ai` | "Provider-neutral, privacy-bounded AI support for Dystil." Providers receive bounded context and read sanitized evidence only. |
| `dystil-automation` | "Provider-neutral automation definitions, persistence, and execution primitives." |
| `dystil-telemetry` | "Privacy-safe telemetry primitives." Deliberately has no exporter or OpenTelemetry SDK dependency. |

### Two boundaries worth not crossing

**`dystil-work-index` does not infer.** Its module doc states it "deliberately does
not infer task intent, causality, completion, or success." It records observable
application/surface continuity and text changes. Inference belongs in
`dystil-insights`.

**`dystil-redact` never touches images.** Text only, by design.

## Storage

SQLite, bootstrapped by `dystil-storage`. Tables created by `lib.rs`:

| Group | Tables |
|---|---|
| Capture | `frames`, `elements`, `ui_events` |
| Search | `activity_search_documents`, `activity_search_fts` |
| AI | `ai_presets` |
| Redaction | `dystil_text_redaction_state` |
| Local chat | `local_chat_sessions`, `local_chat_messages` |
| Agent messaging | `agent_mailbox_state`, `agent_messages` |

## Retrieval

Full-text search over SQLite FTS5. `dystil-storage/src/lib.rs` creates it with
`CREATE VIRTUAL TABLE IF NOT EXISTS activity_search_fts USING fts5(`.

**There is no embedding model and no vector search.** A repo-wide grep for
`embedding` across `.rs` files returns zero matches. Ranking is lexical (BM25 via
FTS5) plus the deterministic ordering `dystil-retrieval` applies.

## AI

See `AI_PROVIDERS.md`. In short: providers are user-configured and external to the
app. Nothing is bundled.

## Editions

See `EDITIONS.md`. Editions are Cargo features resolved at build time, not runtime
configuration.

## Local model files

`~/.dystil/models/` holds exactly one thing: the ONNX PII redaction model. See
`dystil-redact/src/onnx.rs`, which resolves
`.join(".dystil").join("models").join("v45_phase5_pruned")` and documents the path in
its module comment. The app does not download language models.

## Frontend

Next.js on port `1420` in development (`tauri.conf.json` → `"devUrl":
"http://localhost:1420"`), static export to `../out` for release
(`"frontendDist": "../out"`).

Rust → TypeScript bindings are generated, not hand-written. `package.json` exposes
`bindings:check` and `bindings:generate`, both of which shell out to
`cargo test -p dystil-app ... --manifest-path src-tauri/Cargo.toml`. CI fails on
stale bindings.

The three primary UI surfaces are `components/dystil/pages/worth-fixing.tsx`,
`ask-for-fix.tsx`, and `ready-to-use.tsx`.

## What this document deliberately does not claim

Earlier documentation described a bundled `llama.cpp` runtime managed by the app,
specific generator and embedder models, fixed ports for them, semantic embeddings,
and vector search. Repo-wide greps for `llama`, `LocalLlmManager`, those port
numbers, and `embedding` all return zero matches. None of it exists, and it is not
planned in the form described. It has been removed rather than relabelled.
