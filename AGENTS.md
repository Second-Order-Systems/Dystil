# AGENTS.md

Instructions for coding agents working in this repository. Humans are welcome to
read it too — it is the fastest orientation available.

## What Dystil is

A local-first desktop app that observes how you work, identifies work that repeats,
and produces reusable instructions that can do that work again. The user-facing loop
is **Worth fixing** (repetitive work found, with evidence) → **Ask for fix** (the user
clarifies) → **Ready to use** (kept, reusable artifacts).

It is not a note-taking app, a day planner, or a chatbot over your history.

---

## Source of truth — read this before trusting any document

Precedence, highest first:

1. **The code.** Always wins.
2. **`agent_docs/`** — verified against the code, and every factual claim
   carries a citation. Trust it, but if a citation no longer resolves, the document
   is wrong; fix it or file it.
3. **Everything else** — `public_docs/`, `README.md`, marketing copy.

`public_docs/` is **positioning, not specification.** It describes what Dystil is
for and where it is going, and it is deliberately allowed to run ahead of the code.
It may use explanatory concepts (for example "work cards") that do not correspond to
any type in the codebase.

- **Never implement from `public_docs/`.**
- **Never cite it in review.**
- **Never treat a capability named there as existing.**

If narrative and engineering disagree, engineering is right.
If engineering and the code disagree, the code is right — and the document is a bug.

> **Why this exists:** an earlier `docs/TECHNICAL_ARCHITECTURE.md` confidently
> described a bundled `llama.cpp` runtime, named models, fixed ports, embeddings, and
> vector search. None of it existed. It read exactly like a real engineering
> reference, and it propagated into the README. Assume any uncited document is
> capable of the same.

### Writing docs

- A claim in `agent_docs/` needs a citation: **path + symbol name or a short
  verbatim quote.** Never `path:line` — line numbers drift and break the check.
  Good: ``app_config.rs :: telemetry_endpoint()`` or
  ``Cargo.toml — `enterprise-client = ["cloud-sync", "official-build"]` ``
- `public_docs/` must contain **no falsifiable specifics** — no ports, model
  names, schemas, crate names, file paths, or sizes. If it can be wrong, it belongs
  in `agent_docs/`.
- Every doc carries a `status:` in its frontmatter. Three values:

  | `status` | Meaning |
  |---|---|
  | `verified` | Checked against the code, claims cited. Trust it. |
  | `unreviewed` | Lives in `agent_docs/` but predates the split and has not been audited. **Do not treat its specifics as facts** until someone verifies it and flips the header. |
  | `narrative` | Positioning. May run ahead of the code. Never implement from it. |

  If you verify an `unreviewed` document while working, change the header. That is a
  welcome contribution and it is how the backlog shrinks.

---

## Repository layout

```
apps/dystil/            Tauri v2 desktop app; Next.js frontend in app/ + components/
  src-tauri/            Rust backend, Tauri commands, app wiring

crates/
  dystil-capture/       Accessibility, UI-event, window, optional visual capture
  dystil-redact/        Text-only privacy boundary (images are never inspected)
  dystil-storage/       SQLite bootstrap, schema, migrations, queries
  dystil-work-index/    Deterministic surface visits built from captured frames
  dystil-retrieval/     Agent-safe retrieval over sanitized evidence
  dystil-insights/      "Worth Fixing" inference and projection engine
  dystil-ai/            Provider-neutral, privacy-bounded AI support
  dystil-automation/    Automation definitions, persistence, execution
  dystil-mcp/           Dystil capabilities exposed over MCP
  dystil-telemetry/     Privacy-safe telemetry schema + local aggregation
  dystil-engine/        Optional orchestration/sync engine
  dystil-sync/          Optional peer and cloud synchronization
  dystil-protocol/      Multiplayer and wire-protocol types

Hosted ingest and telemetry services are outside this repository; this repository
contains the desktop client and shared wire protocol.

agent_docs/             Verified reference. Cite it. Start at agent_docs/README.md
public_docs/            Positioning and marketing. Do not implement from it.
```

### Where to look before searching the codebase

`agent_docs/` exists so you do not have to re-derive the same facts every session.
Check it first:

| Question | File |
|---|---|
| What owns what, where are the boundaries | `agent_docs/ARCHITECTURE.md` |
| How does capture → redaction → index work | `agent_docs/CAPTURE_PIPELINE.md` |
| Which AI providers, how is Ollama wired | `agent_docs/AI_PROVIDERS.md` |
| What does Dystil expose over MCP | `agent_docs/MCP.md` |
| What tables exist, who writes them | `agent_docs/DATA_MODEL.md` |
| Community vs enterprise differences | `agent_docs/EDITIONS.md` |
| What leaves the device | `agent_docs/PRIVACY_AND_TELEMETRY.md` |
| Colors, type, spacing, components | `agent_docs/DESIGN_SYSTEM.md` |
| **Is this noun real?** | `agent_docs/GLOSSARY.md` |

`GLOSSARY.md` is worth reading once in full. It maps marketing vocabulary onto real
types and lists terms that describe nothing — several of which appeared in older
docs and still circulate.

---

## Commands

All frontend and app commands run from `apps/dystil/`. The package manager is
**bun** — there is no npm/yarn/pnpm lockfile.

```bash
cd apps/dystil
bun install

bunx tauri dev            # Next.js on :1420 + Rust backend together
bunx tauri build          # same command CI runs (.github/workflows/release-app.yml)
```

Tests and checks:

```bash
bun run test              # vitest + bun test
bun run typecheck         # tsc --noEmit
bun run bindings:check    # fails if Rust->TS bindings are stale
bun run bindings:generate # regenerate them
```

Rust, from the repo root:

```bash
cargo fmt
cargo clippy -p dystil-<crate> --all-targets -- -W clippy::all
cargo test  -p dystil-<crate>
```

Install the pre-commit hooks once: `bunx lefthook install`.
They run `cargo fmt --check` and scoped clippy on staged files only.
Bypass sparingly with `LEFTHOOK=0 git commit`.

---

## Things that will bite you

- **`cargo run` against `src-tauri` is not how you run the app.** It skips Tauri's
  `beforeDevCommand`, so the Next.js dev server never starts. Use `bunx tauri dev`.
- **Rust → TypeScript bindings are generated.** After changing a Tauri command
  signature or a shared type, run `bun run bindings:generate`. CI fails on stale
  bindings.
- **Editions are Cargo features, not runtime config.**
  `enterprise-client = ["cloud-sync", "official-build"]`. Community builds have no
  cloud URL and no telemetry endpoint at all — both are `option_env!` and simply
  absent. Do not add a runtime flag that weakens this; there is a test asserting it
  (`app_config.rs :: community_build_has_no_cloud_url`).
- **`dystil-redact` handles text only.** Images are never inspected. Keep it that way.
- **`dystil-work-index` is deliberately dumb.** It records observable continuity and
  text changes; it does not infer intent, causality, completion, or success. Do not
  add inference there — that belongs in `dystil-insights`.
- **AI providers are user-configured**, not bundled. Ollama, Anthropic, OpenAI, and
  custom endpoints. There is no inference runtime shipped inside the app.

---

## Conventions

- Rust 2021, toolchain pinned by `rust-toolchain.toml`.
- TypeScript with Biome (`biome.json`) for lint and format.
- Prefer adding to an existing crate over creating one; the workspace member list in
  the root `Cargo.toml` must stay in sync with `crates/`.
- Privacy is a structural constraint, not a policy note. If a change makes more data
  leave the device, it needs an explicit opt-in and a line in
  `agent_docs/PRIVACY_AND_TELEMETRY.md`.
