---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
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

## Deletion

The privacy surface supports deletion by time range, by application, and by site,
plus a full reset. Deleting capture also removes what was derived from it. Settings,
connections, automations, and downloaded models survive a reset.

When adding a table that stores anything derived from capture, wire it into those
deletion paths — otherwise a user "deleting everything" silently leaves your data
behind.
