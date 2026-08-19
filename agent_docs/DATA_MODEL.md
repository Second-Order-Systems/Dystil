---
status: verified
authority: ground-truth
verified_against: working-tree
verified_on: 2026-08-18
---

> **Verified** against `e84d34c`. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# Data model

One SQLite database on the user's machine. `dystil-storage` owns it — module doc:
*"Dystil-owned SQLite bootstrap for capture data. The schema is intentionally
limited to what Dystil writes or reads."*

No other crate should create tables. `dystil-retrieval`'s doc states the division
explicitly: *"Storage owns SQLite."*

## Tables

Created in `crates/dystil-storage/src/lib.rs`.

| Table | Group | Written by |
|---|---|---|
| `frames` | Capture | `dystil-capture` |
| `elements` | Capture | `dystil-capture` |
| `ui_events` | Capture | `dystil-capture` |
| `activity_search_documents` | Search | work index / storage |
| `activity_search_fts` | Search | FTS5 virtual table |
| `dystil_text_redaction_state` | Redaction | `dystil-redact` ML pass |
| `ai_presets` | Config | `ai_presets.rs` |
| `local_chat_sessions` | Chat | app |
| `local_chat_messages` | Chat | app |
| `agent_mailbox_state` | Agent messaging | app |
| `agent_messages` | Agent messaging | app |
| `legacy_unused` | — | vestigial; do not extend |

## Full-text search

`activity_search_fts` is an FTS5 virtual table:

```
CREATE VIRTUAL TABLE IF NOT EXISTS activity_search_fts USING fts5(
```

Ranking is BM25, FTS5's default. There is no vector column and no embedding table —
see `GLOSSARY.md`.

## Redaction state

`dystil_text_redaction_state` tracks progress of the asynchronous ML PII pass so it
can resume. The deterministic pass runs before any write and leaves no state — see
`CAPTURE_PIPELINE.md`.

## Where it lives on disk

Under `~/.dystil/`. The privacy UI can open this folder directly
(`components/dystil/pages/privacy.tsx`, "Dystil could not open ~/.dystil").

`~/.dystil/models/` holds the ONNX PII model only. `~/.dystil/runs/` holds
automation run logs (`dystil-automation/src/lib.rs` joins `.dystil` / `runs` /
`run_id`).

## Portable prompt and skill bundles

The separate insights database also records the durable state for an explicitly
built portable prompt and Agent Skill. `crates/dystil-insights/src/store.rs ::
migrate()` creates `artifact_bundle_jobs`, `artifact_bundles`, and
`artifact_bundle_installs` in schema version 9, and schema version 10 adds the
safe `stage` label on `artifact_bundle_jobs`. The first table holds progress
and errors, the second indexes immutable validated revisions, and the third is
an explicit local-install or export receipt. The generated files themselves are
not duplicated in SQLite: `crates/dystil-insights/src/skill_bundle.rs ::
build_skill_bundle()` moves the validated output under Dystil's data directory,
and `SkillBundlePaths::new()` names the `skill-bundles` and
`skill-bundle-builds` roots.

Schema version 11 additionally creates `artifact_workflow_reconstructions`.
Each row is tied to one bundle job and records the validated, evidence-grounded
workflow Markdown, cited evidence IDs, reconstruction version, provider, model,
and elapsed time. Citation: `crates/dystil-insights/src/store.rs :: migrate()`.
`crates/dystil-insights/src/skill_bundle.rs :: run_skill_bundle_build()` persists
that record before it starts the separate final skill-builder call.

`crates/dystil-insights/src/skill_bundle.rs :: validate_output_with_urls()`
independently checks each generated bundle before retention, including local
references, required `references/workflow.md`, optional `agents/openai.yaml`,
text encoding, unsafe paths, absence of Dystil provenance, and evidence-backed
literal URLs. `crates/dystil-insights/src/store.rs :: upsert_evidence()` uses
explicit evidence column names so fresh and migrated SQLite tables project URL
and excerpt correctly regardless of physical column order.

## Deletion

The privacy surface supports deletion by time range, by application, and by site,
plus a full reset. Deleting capture also removes what was derived from it. Settings,
connections, automations, and downloaded models survive a reset.

When adding a table that stores anything derived from capture, wire it into those
deletion paths — otherwise a user "deleting everything" silently leaves your data
behind.

`crates/dystil-insights/src/store.rs :: invalidate_workflow_reconstructions()`
removes reconstructions after scoped capture forgetting, and
`apps/dystil/src-tauri/src/deletion.rs :: delete_capture_data()` calls it beside
Ask retrieval-memo invalidation.
