---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# Capture pipeline

How activity on the machine becomes stored, redacted, retrievable data.

```
user activity
   → trigger              (activity-driven, not continuous)
   → accessibility walk   (AX tree / UI Automation)
   → deterministic redaction   ← runs BEFORE anything is written
   → SQLite                (frames, elements, ui_events)
   → async: ML PII pass    (ONNX, second redaction pass)
   → async: work index     (compact surface visits)
   → retrieval             (FTS5) → UI and MCP
```

## Capture is activity-triggered, not continuous

Dystil does not record constantly. Capture is driven by user activity, which keeps
storage bounded and biases what is kept toward moments of intent.
`dystil-capture/src/coordinator.rs` is built around this — it carries ~121
trigger-related references and idle handling.

`dystil-capture`'s module doc: *"Platform-neutral capture orchestration for Dystil.
Screen pixels are an optional evidence source."*

## What is read

Operating-system accessibility data — AX trees on macOS, UI Automation on Windows.
`crates/dystil-capture/src/` carries ~193 accessibility references. The walk
collects relevant nodes and the text visible on screen, plus window and application
metadata and UI events.

Screenshots are **optional** and, in community builds, off — see `EDITIONS.md`.

## Redaction happens twice

This is a deliberate two-pass design, and the ordering matters.

**Pass 1 — deterministic, strictly before the write.** Verified by reading
`capture_store.rs :: persist`: `sanitize_text` is applied to frame text, app name,
window name, browser URL, document path, and device name, and `sanitize_nodes`
walks the accessibility tree — *all before* the `INSERT INTO frames` in the same
function. Raw sensitive text is never persisted and cleaned afterwards.

`dystil-capture/src/capture_store.rs :: sanitize_text` delegates to
`dystil_redact::sanitize_text`. `dystil-redact/src/lib.rs` exposes that plus
`sanitize_optional`, with `RedactedSpan`, `SpanLabel`, and `RedactionOutput` as
result types. UI events go through the same helper
(`dystil-capture/src/ui_event_store.rs`).

Two tests pin this ordering:
`capture_store.rs :: deterministic_redaction_covers_text_tree_and_metadata` and
`capture_store.rs :: ax_only_frame_has_empty_path_and_sanitized_full_text`.

**Pass 2 — ML, asynchronous.** `dystil-capture/src/redaction_worker.rs` runs a local
ONNX model (`TextRedactor`) over stored data for detections rules miss; its module
doc notes it "falls back to `sanitize_text` on model error." The model resolves to
`~/.dystil/models/v45_phase5_pruned/` (`dystil-redact/src/onnx.rs`). Progress is
tracked in `dystil_text_redaction_state` via `record_state`, so the pass is
resumable.

`dystil-redact` is text-only by design — its module doc states *"This crate
intentionally handles text only. Images are never inspected."* Do not add image
inspection here.

## Work index

An asynchronous worker turns captured frames into compact **surface visits** —
`dystil-work-index`, whose module doc reads *"Deterministic construction of compact
surface visits from captured frames."*

The purpose is token efficiency: a compact, structured record is far cheaper for an
LLM or external agent to retrieve over than raw capture.

**This layer does not infer.** Its doc continues: *"This layer records observable
application/surface continuity and text changes. It deliberately does not infer task
intent, causality, completion, or success."* Inference belongs in `dystil-insights`.

## Retrieval

FTS5 over `activity_search_documents` / `activity_search_fts`
(`dystil-storage/src/lib.rs` — `CREATE VIRTUAL TABLE IF NOT EXISTS
activity_search_fts USING fts5(`). `dystil-retrieval` layers stable evidence
identifiers, response budgets, deduplication, and deep links on top, so every AI
adapter sees the same behaviour.

Results reach the user through the app UI and through MCP — see `MCP.md`.

## Not implemented: embeddings

Semantic search over the work index is a stated future direction, not current
behaviour. A repo-wide grep for `embedding` across `.rs` files returns **zero**
matches, and there is no vector index. Ranking today is lexical.

Do not describe semantic or vector search as shipping.
