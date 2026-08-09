---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

# agent_docs — index

Ground truth for coding agents and engineers. Everything here is verified against
the code and cites a path plus a symbol name or verbatim quote.

**Start with `AGENTS.md` at the repo root** for the source-of-truth rule and the
command reference. This folder is the detail behind it.

Marketing and positioning live in `public_docs/`. That material is deliberately
allowed to run ahead of the code — never implement from it.

## Contents

| File | What it answers |
|---|---|
| `ARCHITECTURE.md` | What the system is, which crate owns what, where the boundaries are |
| `CAPTURE_PIPELINE.md` | How activity becomes stored, redacted, indexed data |
| `AI_PROVIDERS.md` | Which providers exist, how Ollama is wired, what is *not* bundled |
| `MCP.md` | The five MCP tools Dystil exposes to external agents |
| `DATA_MODEL.md` | SQLite tables and what writes them |
| `EDITIONS.md` | Community vs enterprise, and why it is a build-time feature |
| `PRIVACY_AND_TELEMETRY.md` | What leaves the device, per edition |
| `DESIGN_SYSTEM.md` | Design tokens and component specs |
| `GLOSSARY.md` | Marketing term → real type. Read this before trusting a noun. |
| `TELEMETRY_FOUNDATION_PLAN.md` | Telemetry design and constraints |
| `TELEMETRY_LOCAL_VALIDATION.md` | How to validate telemetry locally |
| `A11Y_CAPTURE_FOLLOWUPS.md` | Open accessibility-capture work |

## Keeping this folder honest

A claim here needs a citation: **path + symbol name or short verbatim quote**.
Never `path:line` — line numbers drift and break the drift check.

```
✅  app_config.rs :: telemetry_endpoint()
✅  Cargo.toml — `enterprise-client = ["cloud-sync", "official-build"]`
❌  app_config.rs:8-12
```

If you find a claim here that the code contradicts, the document is the bug. Fix it
in the same change.
