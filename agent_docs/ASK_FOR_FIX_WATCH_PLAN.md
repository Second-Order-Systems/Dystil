---
status: verified
authority: approved-implementation-plan
verified_against: working-tree
verified_on: 2026-08-16
---

# Ask-for-fix evidence watch plan

This document records the approved future design for **Keep watching** in Ask
for fix. It is an implementation plan, not a claim that watches already exist.
Current-state claims are cited to code as required by `AGENTS.md`.

## Outcome

When Ask for fix cannot honestly propose a fix because its available evidence is
missing, unrelated, or insufficient, the user may opt into a durable local
watch. Dystil gathers only later relevant evidence, revisits the request once a
credible end-to-end instance is available, and asks the user to review its
understanding before producing an artifact.

The watch is not an automatic fix, a general-purpose monitor, or a promise to
notify on every keyword hit. It is a user-authorized, evidence-led investigation
of one previously framed problem.

## Current baseline

- Ask for fix persists sessions and its retrieval reports in the insights
  database. Citation: `crates/dystil-insights/src/store.rs` —
  `CREATE TABLE ask_sessions` and `CREATE TABLE ask_retrieval_reports`.
- The Ask flow has an Economy retrieval explorer and a Frontier follow-up; the
  explorer report can represent `relevant`, `nothing_found`, `capture_gap`, or
  `unavailable`. Citation: `crates/dystil-insights/src/ask_for_fix.rs ::
  RetrievalStatus`, `run_retrieval_explorer()`, and `run_staged_ask_for_fix()`.
- The Ask retrieval report is currently an internal memo, not a field returned
  in `AskSessionView`. Citation: `crates/dystil-insights/src/ask_for_fix.rs ::
  TurnPacket` and `AskSessionView`.
- The capture-to-insights engine already turns new sanitized capture into
  Explorer batches and observations on a background tick. Citation:
  `apps/dystil/src-tauri/src/worth_fixing_engine.rs :: tick()` and
  `maybe_explore()`.
- Capture retrieval is lexical FTS5 today; it is not embedding/vector search.
  Citation: `agent_docs/CAPTURE_PIPELINE.md :: Retrieval` and
  `agent_docs/CAPTURE_PIPELINE.md :: Not implemented: embeddings`.
- The existing Steward is for general Worth Fixing inference over pending
  observations. Citation: `apps/dystil/src-tauri/src/worth_fixing_engine.rs ::
  maybe_steward()` and `crates/dystil-insights/src/store.rs :: observations`.
- Full capture deletion clears Ask sessions and all derived insights; scoped
  capture deletion invalidates Ask retrieval memos. Citation:
  `apps/dystil/src-tauri/src/deletion.rs :: delete_capture_data()` and
  `crates/dystil-insights/src/store.rs :: delete_all_insights_data()`.

## Product decisions

| Decision | Approved behaviour |
|---|---|
| When to offer a watch | Whenever Ask-for-fix Frontier review finds the available evidence irrelevant or insufficient, including a non-empty but misleading explorer result. Never offer it for provider/retrieval failure alone. |
| Evidence sufficient for review | One credible, observed end-to-end instance. The Frontier reviewer still makes the final judgement. |
| Expiry | Never silently expire. After one week without useful evidence, show an in-app checkpoint to give guidance, keep watching, or stop. |
| Capacity | At most five active watches. Starting a sixth requires stopping an existing watch. |
| Notification | In-app notification first; native OS notifications are out of scope until separately designed and opt-in. |
| Final answer | Never create or deliver an artifact silently. A sufficient watch reopens Ask for fix at its normal understanding/confirmation gate. |
| Historical re-check | A revised, user-approved watch specification permits one bounded historical re-check. After that, evaluate only newer activity. |

## User flow

1. The usual retrieval/explorer and Ask-for-fix Frontier pass run.
2. If the Frontier judges the evidence unrelated or insufficient, the user sees
   a plain-language insufficient-evidence state, not a retrieval error:

   > I found some related activity, but not enough to trust a fix yet.

   Available actions are **Keep watching**, **Add context and search again**,
   and **Continue without observed evidence** when a clearly limited generic
   artifact remains honest.
3. Choosing **Keep watching** shows a concise, editable description of what
   Dystil will seek and what remains unknown. The user confirms this spec.
4. The Ask-for-fix surface shows the active local watch and a **Stop watching**
   control. It does not claim a match merely because a candidate was found.
5. Once a review finds a credible end-to-end instance, an in-app notification
   says that Dystil has enough evidence to revisit the request. Selecting it
   returns the user to a refreshed consolidation screen.
6. The user approves or corrects the renewed understanding before the normal
   presentation step can generate an artifact.

## Durable model

Add durable tables to `dystil-insights`; do not put watches in the capture
database.

### `ask_watches`

One row per user-authorized request:

- identity and source `session_id`;
- `state`: `active`, `review_ready`, `stopped`, or `dismissed`;
- a versioned, user-approved `watch_spec_json`;
- `baseline_observation_sequence` and `last_evaluated_sequence` cursors;
- `historical_recheck_used`, timestamps, and the one-week checkpoint state.

The watch spec contains the user goal, relevant signals, disqualifying/adjacent
work, missing evidence, and the end-to-end sufficiency rule. It is specific
enough for evaluation but remains editable and user-approved.

### `ask_watch_evidence`

Store retained and rejected evidence separately from the prose memo:

- `watch_id`, stable `evidence_id`, and optional observation/batch identity;
- `disposition`: `supporting` or `rejected`;
- evaluator explanation and timestamps.

Rejected evidence stays as an audit and deduplication record, but cannot ground
a later review or answer.

### `ask_watch_evaluations`

Persist every Economy evaluation receipt, including its input cursor,
fingerprint, model/provider receipt, result, diagnostics, and output summary.
Valid results are `no_signal`, `add_evidence`, and `ready_for_review`.
Failures do not advance the cursor, so a later eligible run can retry safely.

All new derived tables must be included in `delete_all_insights_data()`. Scoped
capture deletion must remove affected watch evidence and return a previously
ready watch to `active` when its supporting dossier is no longer valid.

## Collection loop

The watch collector is deliberately separate from the global Worth Fixing
Steward. The Steward discovers general opportunities from noisy activity; a
watch evaluates a known, user-authorized question.

1. After the existing Explorer accepts a new observation batch, find active
   watches whose cursor precedes that batch.
2. Run a deterministic candidate filter before using a model. It matches
   normalized application names and normalized browser hosts/path segments,
   plus watch terms. App aliases, case, punctuation, executable/display-name
   variations, URL query strings, fragments, tracking parameters, and volatile
   IDs are normalized away. Fuzzy matching creates candidates only; it never
   counts as evidence.
3. For each watch with candidates, run one Economy request at high reasoning
   effort. Supply only the watch spec, unseen candidate observations, and
   existing retained evidence. It may use the same read-only retrieval tools to
   inspect a bounded source/context around promising IDs.
4. Require structured output with a decision, supporting and rejected IDs,
   why a candidate relates to the request, and what evidence is still missing.
5. Validate returned IDs against the candidate set, retained dossier, and
   policy-allowed non-deleted evidence before writing them. Deduplicate IDs.
6. Advance the watch cursor only after a valid completed evaluation. A watch is
   evaluated no more than once for the same Explorer batch.
7. If one credible end-to-end instance is now present, mark the watch
   `review_ready`; otherwise it continues silently.

This cadence is event-driven by newly accepted Explorer batches, rather than a
model call per raw capture event or a fixed polling call when no new candidate
activity exists.

## Review loop

`review_ready` runs a separate Frontier Ask-for-fix review job, rather than
changing the existing general Steward output contract. Its bounded input is the
original conversation, approved watch spec, retained evidence ledger, collector
summaries, and explicit uncertainty. It may inspect those evidence IDs through
read-only retrieval.

It returns one of:

- `sufficient`: produce a refreshed understanding and notify the user to
  review it;
- `needs_more_observation`: keep the watch active and retain its updated missing
  evidence criteria;
- `not_the_same_work`: retain the misleading items as rejected, then either
  continue watching or ask the user to revise cues;
- `needs_more_than_one_person`: reopen consolidation with that limitation.

The reviewer never skips `consolidate` and never produces a user-visible final
artifact without the existing confirmation action.

## Historical versus future evidence

`not_the_same_work` is a diagnostic, not a reason to rescan all history:

| Cause | Next action |
|---|---|
| A false positive and a still-clear spec | Retain as rejected, advance the cursor, and watch new observations only. |
| Ambiguous/too-broad cues | Ask for revised user guidance; then run one targeted historical re-check. |
| A likely incomplete original historical search | Run one bounded historical re-check. |
| The activity has not been captured | Watch new observations only. |

After a permitted backfill, record a new baseline and continue from newer
activity. Historical search must have explicit date/result/tool budgets and use
the same normalized app/URL candidate rules.

## Tauri/UI boundary

Extend `AskSessionView` with a compact user-facing watch state; do not expose
raw observation records, full retrieval reports, or tool traces. Add commands
to start and stop a watch, list relevant watch state, and resume a review-ready
watch. Regenerate Rust-to-TypeScript bindings after changing these DTOs or
commands.

The UI needs distinct treatments for:

- insufficient/irrelevant evidence with a watch offer;
- active watch and stop control;
- capacity reached (choose a watch to stop before starting another);
- one-week no-signal checkpoint;
- review-ready notification and return to consolidation;
- stopped/dismissed watch history.

## Non-goals

- No raw capture, screenshots, accessibility trees, shell access, network
  access, or external retrieval is added.
- No semantic/vector retrieval is implied; all initial candidate narrowing is
  deterministic lexical/metadata matching.
- No changes to the global Steward's general Worth Fixing schema are required.
- No automatic execution, automation, or artifact creation occurs from a watch.

## Deferred follow-up: narrower workflow found

The collector can accumulate strong evidence for a workflow adjacent to, but
narrower than, the user's watch specification. For example, it may establish
recurring RFQ drafting and sending while failing to link the same message to an
explicit attachment check and Gmail Sent-state verification.

The current safe behavior is to keep the strict watch active: Dystil must not
silently turn partial, separately observed steps into the requested end-to-end
workflow (`crates/dystil-insights/src/ask_for_fix.rs ::
collect_ask_for_fix_watches`). A future iteration should add a user-mediated
branch when this occurs:

1. explain the narrower recurring workflow that is supported;
2. offer the user a choice to review that narrower workflow, continue watching
   for the original stricter proof, or revise the watch cues; and
3. require the user's choice before changing the watch goal or presenting a fix.

This is a product follow-up, not a relaxation of the evidence requirement.

## Verification

- Migration, CRUD, five-watch cap, stop/restart, and deletion tests.
- Cursor/idempotency tests: no duplicate evaluation of a batch, retry after
  failure, and no duplicate evidence IDs.
- Evidence-admission tests: reject hallucinated, deleted, disallowed, or
  out-of-candidate IDs.
- Historical-recheck tests: one permitted backfill after an approved revision,
  then future-only evaluation.
- Frontend tests for insufficient evidence, active/ready/stopped states,
  capacity, one-week checkpoint, and resume-to-consolidation.
- Binding generation/check and the relevant Rust and frontend test suites.
