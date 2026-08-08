---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# MCP surface

Dystil exposes a bounded view of captured work over the Model Context Protocol, so
an external agent — Claude Code, Codex, or any MCP client — can answer questions
about what the user has been doing without being handed the raw database.

The server is a standalone binary: `crates/dystil-mcp/src/main.rs`.

## Tools

| Tool | Purpose |
|---|---|
| `dystil_search_activity` | Search captured work. Takes a `query`. |
| `dystil_get_activity_overview` | High-level summary of a period. |
| `dystil_get_activity_range` | Activity within a time range. |
| `dystil_get_activity_context` | Context around a specific result. |
| `dystil_get_source` | Resolve an `evidence_id` back to its source. |

Results are addressed by `evidence_id`, which `dystil-retrieval` owns — its module
doc describes "stable evidence identifiers, response budgets, deduplication, deep
links and deterministic overview diagnosis so every AI adapter observes the same
behavior."

## Why it is shaped this way

The design constraint is that an external agent gets *relevant* material, not
everything. `dystil-ai`'s module doc states providers "receive bounded context and
can read sanitized evidence only." The same boundary applies here: MCP serves the
sanitized evidence projection, not raw capture.

Response budgets exist so a broad query cannot pull the whole history into an
agent's context.

## Contributor notes

- New MCP tools must go through `dystil-retrieval`, not directly to
  `dystil-storage`. Bypassing it loses the sanitization and budget guarantees.
- Anything returned here can reach a third-party model, depending on which client
  the user has connected. Treat the return shape as an external boundary and check
  it against `PRIVACY_AND_TELEMETRY.md`.
