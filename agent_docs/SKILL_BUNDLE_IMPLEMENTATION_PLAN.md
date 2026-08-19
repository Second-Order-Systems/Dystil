---
status: verified
authority: approved-implementation-handoff
verified_against: working-tree
verified_on: 2026-08-18
---

# Evidence-grounded workflow reconstruction and skill generation

## Objective

When a user selects **Build skill** for a shortcut, Dystil must investigate the
selected work deeply and then generate a portable Agent Skill capable of doing
that work end to end. The resulting skill should actively use the best runtime
capabilities available: connectors, MCP tools, local tools, and browser/computer
use with existing signed-in sessions.

The skill must be specific to the observed task. It should preserve supported
applications, websites, stable domains or routes, document and template names,
file-naming conventions, source-discovery behavior, decision rules, output
validation, and completion signals. It must not invent URLs, paths, credentials,
connectors, fields, or business-system behavior.

This document replaces the earlier shallow design in which one builder call
received a generic handoff plus the finding's small proof set and immediately
generated final files.

## Fundamental architecture decision

Keep three responsibilities separate:

1. **Worth Fixing Steward — discovery:** decide whether repeated work is worth
   surfacing. Its evidence answers “why is this finding credible?”
2. **Workflow Reconstruction Agent — investigation:** after the user requests a
   skill, reconstruct how that one task is actually performed across all relevant
   occurrences and surrounding textual activity.
3. **Skill Builder — authoring:** use the reconstruction as the authoritative
   workflow input and turn it into a concise prompt and portable Agent Skill.

The reconstruction is a mandatory model phase, not another paragraph added to
the builder prompt. It gets its own model call, persisted result, deterministic
validation, progress stage, and tests.

```text
raw textual capture
  -> Explorer observations
  -> Steward opportunity + occurrences + finding proof
  -> user selects Build skill
  -> Workflow Reconstruction Agent
  -> validated, persisted WORKFLOW.md
  -> Skill Builder using skill-creator
  -> prompt.md + portable skill bundle
```

Steward should remain broad and economical. Reconstruction is allowed to spend
more retrieval calls and reasoning on one user-selected task. A valid finding
must not be withdrawn merely because reconstruction has gaps; gaps become
explicit runtime-discovery behavior in the generated skill.

## Verified current baseline

- A user starts generation explicitly from a kept shortcut. The Tauri command
  creates a durable job and dispatches the provider in the background. Citation:
  `apps/dystil/src-tauri/src/ready_to_use_commands.rs ::
  build_ready_artifact_skill_bundle()`.
- The existing bundle job already exposes safe stages including `preparing`,
  `investigating`, `building`, and `validating`. Citation:
  `crates/dystil-insights/src/types.rs :: SkillBundleStage` and
  `crates/dystil-insights/src/skill_bundle.rs :: update_stage()`.
- The current implementation renders one shallow `BUILD.md` from the artifact,
  finding, and `finding_evidence`, then makes one automation call that produces
  final files. Citation: `crates/dystil-insights/src/skill_bundle.rs ::
  build_markdown()` and `run_skill_bundle_build()`.
- The current storage layer validates that Steward-selected finding evidence is
  admissible and belongs to opportunity history, then persists exactly that
  small list. It does not expand it into a workflow trace. Citation:
  `crates/dystil-insights/src/store.rs :: apply_reconciliation()` — the
  `finding.evidence_ids` validation and `finding_evidence` inserts.
- Broader workflow information already exists in `occurrences`, including
  observation IDs, evidence IDs, proposal steps, and start/end times. Citation:
  `crates/dystil-insights/src/store.rs` — `CREATE TABLE occurrences` and the
  occurrence insertion inside `apply_reconciliation()`.
- Dystil's configured Codex and Claude automation runtimes can receive the local
  Dystil MCP server and use bounded textual retrieval tools. Citation:
  `crates/dystil-ai/src/lib.rs :: CliProvider::run_automation_with_model()` and
  `apps/dystil/src-tauri/src/ai_presets.rs :: DYSTIL_RETRIEVAL_TOOLS`.
- Retrieval provides activity overview, search, source resolution, context
  expansion, and time-range access. Citation: `crates/dystil-mcp/src/main.rs ::
  tools()` and `call_tool()`.
- The final output validator already supports optional `references/`, `scripts/`,
  `assets/`, and `agents/openai.yaml`. Citation:
  `crates/dystil-insights/src/skill_bundle.rs :: validate_skill_layout()`.

## Why current input is insufficient

`finding_evidence` is proof for the Worth Fixing card, not a recording of the
complete task. For the Deepika purchase-order example, the finding is grounded
mostly in Word actions even though retained capture contains the surrounding
email attachment, browser research, Excel calculation, Word template, save,
print, and output-name activity.

The generic handoff is also not a workflow specification. It describes the
approved outcome, but usually omits operational sources, exact systems, access
methods, decisions, and completion checks. Prompting the final builder to “look
for more context” does not create a reliable investigation phase.

## Trigger and durable lifecycle

The existing **Build skill** action remains the only trigger. Saving a shortcut
must not start reconstruction automatically.

One top-level `artifact_bundle_jobs` record should orchestrate two provider
calls:

1. `preparing`: assemble approved intent and reconstruction anchors.
2. `investigating`: run the Workflow Reconstruction Agent and validate/persist
   its `WORKFLOW.md`.
3. `building`: run the Skill Builder using that persisted reconstruction.
4. `validating`: validate and persist the immutable bundle.
5. `ready` or `failed`.

Provider event payloads remain private. The UI sees only the existing safe stage
labels. The entire process remains headless and must not open provider UIs,
browsers, file explorers, terminals, or other windows.

The first implementation may use the same configured Codex or Claude runtime
for both phases. The two calls must still have different prompts, work products,
and responsibilities.

## Reconstruction inputs

Create these workspace inputs before the first provider call:

```text
input/INTENT.md
input/RECONSTRUCTION_SEED.md
```

### `INTENT.md`

This is the user's approved scope, not evidence. Include:

- artifact ID, version, title, and kind;
- current artifact body/handoff;
- source finding claim and reason when one exists;
- Ask-for-Fix approved understanding when that is the origin.

### `RECONSTRUCTION_SEED.md`

This is a set of search anchors, not the final workflow. For a Worth Fixing
origin, include:

- finding and opportunity IDs;
- every retained occurrence for that opportunity, or a deterministic bounded
  representative set if the history is very large;
- each occurrence's start/end time, steps, distinctness basis, observation IDs,
  and evidence IDs;
- referenced observation statements;
- admissible evidence metadata and short textual excerpts;
- the small `finding_evidence` proof set, clearly labelled as proof rather than
  complete workflow context.

For an Ask-for-Fix origin, resolve the session through
`ask_sessions.artifact_kept_id` and include:

- locked understanding and approved presentation;
- relevant user/assistant messages;
- retrieval memo and grounding IDs from the accepted Ask retrieval report;
- any retained evidence anchors from that retrieval.

Citations for the existing Ask provenance path:
`crates/dystil-insights/src/store.rs` — `ask_sessions.artifact_kept_id`, and
`crates/dystil-insights/src/ask_for_fix.rs :: keep_ask_artifact()`.

If neither origin provides evidence anchors, reconstruction still runs using the
approved intent as its initial search query. It must report only what retrieval
actually supports.

## Workflow Reconstruction Agent behavior

Add a dedicated production prompt resource, for example:

```text
crates/dystil-insights/resources/workflow_reconstruction_prompt.md
```

The agent must:

1. Read `INTENT.md` and `RECONSTRUCTION_SEED.md` completely.
2. Resolve supplied evidence anchors with Dystil retrieval.
3. Inspect bounded context before and after anchors.
4. Search for identifiers discovered during investigation: document names,
   order numbers, customer/vendor names, email subjects, applications, domains,
   template names, and output filenames.
5. Compare occurrences to distinguish stable workflow from incidental activity.
6. Follow supported transitions across email, browser, local files, and desktop
   applications.
7. Exclude nearby but unrelated tabs, messages, and files; proximity alone is
   not evidence of relevance.
8. Distinguish observed facts from recommended runtime execution strategies.
9. Map every user-specific operational fact to stable evidence IDs.
10. Write one in-depth `input/WORKFLOW.md` and no final skill files.

It must not ask the user questions or return `needs_user_input`. Missing details
belong under runtime discovery. It uses textual data only and does not inspect
screenshots or execute the observed business workflow.

## Required `WORKFLOW.md` contract

Use Markdown as the semantic contract. Do not replace it with a shallow JSON
specification. Internal structured metadata may be stored alongside it by
Dystil, but the authoring model must read this document.

Require these sections:

```markdown
# Workflow reconstruction

## Task outcome and boundaries
## Trigger and starting state
## Inputs and source discovery
## Systems, surfaces, and access
## Observed end-to-end workflow
## Decisions, variants, and exceptions
## Outputs, destinations, and naming
## Validation and completion signals
## Runtime execution strategy
## Evidence map
## Unknowns and runtime discovery
```

For each workflow stage, capture when supported:

- the action and its purpose;
- the source and destination of information;
- observed application, document, domain, stable route, or file convention;
- preferred runtime mechanism: connector/MCP, local tool, or browser/computer;
- fallback when the preferred capability is unavailable;
- validation before moving to the next stage;
- optional evidence labels supporting the operational claim. These are grounding
  aids for the investigator, not a machine-readable citation protocol; `E1`,
  `E2`, or a raw capture ID are all acceptable.

Avoid copying large raw excerpts or unrelated private content. Preserve only the
specific context required to reproduce the task.

This document is not an unused report. It must be persisted and consumed
directly by the second provider call.

## Reconstruction persistence

Increment the insights schema and add a durable table such as:

```sql
artifact_workflow_reconstructions(
  reconstruction_id TEXT PRIMARY KEY,
  bundle_job_id TEXT NOT NULL UNIQUE,
  artifact_id TEXT NOT NULL,
  artifact_version INTEGER NOT NULL,
  input_fingerprint TEXT NOT NULL,
  body TEXT NOT NULL,
  evidence_ids_json TEXT NOT NULL,
  reconstruction_version TEXT NOT NULL,
  provider TEXT NOT NULL,
  model TEXT NOT NULL,
  runtime_version TEXT,
  elapsed_ms INTEGER NOT NULL,
  created_at TEXT NOT NULL
)
```

The body is the validated Markdown. `evidence_ids_json` is extracted from its
Evidence map and retained for deletion/invalidation auditing. Include the
reconstruction prompt version and seed in the bundle input fingerprint so prompt
or provenance changes produce a new build revision.

If reconstruction succeeds but final building fails, a retry may reuse the
persisted reconstruction only when its complete fingerprint still matches.

## Reconstruction validation

Before calling the Skill Builder:

- require all document sections;
- enforce a bounded UTF-8 size;
- require an ordered workflow and a completion signal;
- extract every stable evidence ID mentioned;
- retain recognizable raw IDs when present, but never reject a reconstruction
  because a label is unknown, stale, or uses a simple form such as `E1`;
- use the Evidence map to keep the investigator grounded, rather than as a
  database-integrity boundary;
- reject Dystil build instructions, provider invocation syntax, or attempts to
  create final output in this phase.

Allow one provider repair attempt for structural validation failure. Do not
start user clarification.

## Skill Builder refactor

Only after reconstruction is ready, create/copy:

```text
input/INTENT.md
input/WORKFLOW.md
builder/skill-creator/
output/
```

The second call must read the vendored upstream `skill-creator` completely, then
read `INTENT.md` and `WORKFLOW.md`. The reconstruction is authoritative for
observed workflow facts; the intent is authoritative for user-approved scope.

The builder should perform only targeted retrieval for an explicitly unresolved
point. It should not repeat broad workflow investigation.

Require final output:

```text
output/prompt.md
output/skill/<skill-name>/SKILL.md
output/skill/<skill-name>/references/workflow.md
output/skill/<skill-name>/scripts/        # only when materially useful
output/skill/<skill-name>/assets/         # only when materially useful
output/skill/<skill-name>/agents/openai.yaml  # optional metadata
```

`SKILL.md` should stay concise and control triggering, capability discovery,
execution order, fallbacks, and validation. It must link directly to
`references/workflow.md`. The reference contains the task-specific operational
details translated from the reconstruction.

Do not ship internal Dystil evidence IDs or raw provenance in the portable
skill. The builder translates evidence-backed facts into usable instructions.

## Runtime capability behavior in every generated skill

Generated skills must use this capability ladder:

1. Inspect available connectors and MCP tools.
2. Prefer already-authorized connectors for email, storage, documents, or the
   relevant business system.
3. Use local file/application tools when the task requires local material.
4. Use browser/computer control with the existing signed-in session when a
   connector is absent or the observed interaction requires a UI.
5. Ask at runtime for one specific missing connection, file, folder, or current
   page only when discovery cannot proceed.

Do not assume a named Gmail, Drive, Ivalua, or other connector exists. The skill
may preserve an evidence-backed service/domain and still instruct the runtime to
discover which available tool can access it.

## Final bundle validation

Retain the current deterministic layout, reference, encoding, size, checksum,
and immutable-revision validation. Add these requirements:

- `references/workflow.md` exists and is referenced from `SKILL.md`;
- final portable files contain no Dystil evidence IDs, build paths, or temporary
  reconstruction instructions;
- every literal URL in the final output is present in the reconstruction's
  evidence-backed URL allowlist;
- local paths must be evidence-backed or expressed as runtime discovery, never
  invented absolutes;
- `prompt.md` and `SKILL.md` include source discovery, action, validation, and a
  completion signal;
- optional scripts are referenced, bounded, and executable/testable without
  provider-specific assumptions.

The validator should not require a connector or browser for tasks whose evidence
shows a different execution mechanism.

## Evidence projection correctness

Future evidence admission must be correct at initial insertion. Replace
positional `INSERT INTO evidence VALUES(...)` with an explicit column list, since
fresh and migrated SQLite tables can have different physical column order.
Citation for the current positional insertion:
`crates/dystil-insights/src/store.rs :: upsert_evidence()`.

Do not add a permanent startup reconciliation scan. Historical fixture repair
remains a disposable ignored script under `.local/`; run it only against copied
fixture data before backtesting. The raw capture database remains authoritative
for that repair.

## UI behavior

No new clarification UI is required. Reuse the current inline build progress.
Suggested user-facing stage labels:

- Preparing context
- Investigating the workflow
- Building the skill
- Validating the skill

The build remains navigable and cancellable only through existing job behavior;
do not open external windows or provider authentication screens automatically.

## Implementation sequence

1. Fix explicit-column evidence admission and add its migration-history test.
2. Add reconstruction prompt, seed renderer, validator, durable record, and
   first automation call inside the bundle job.
3. Refactor the builder workspace and prompt to consume persisted
   `WORKFLOW.md`.
4. Require and validate final `references/workflow.md`.
5. Extend deletion/reset logic for the new reconstruction table.
6. Update builder version/fingerprints so old shallow bundles are not reused.
7. Extend the headless backtest to preserve and report reconstruction artifacts
   when requested for diagnostics.
8. Update verified data-model, AI-provider, and glossary documentation with
   symbol citations.
9. Regenerate Rust-to-TypeScript bindings only if public types or command
   signatures change.

Likely primary files:

- `crates/dystil-insights/src/skill_bundle.rs`
- `crates/dystil-insights/src/store.rs`
- `crates/dystil-insights/src/types.rs`
- `crates/dystil-insights/resources/workflow_reconstruction_prompt.md`
- `crates/dystil-insights/resources/skill_bundle_builder_prompt.md`
- `crates/dystil-insights/src/bin/skill_bundle_backtest.rs`
- `apps/dystil/src-tauri/src/ready_to_use_commands.rs`
- `apps/dystil/components/dystil/home/your-shortcuts.tsx` only if labels need
  adjustment

## Tests

### Unit and integration tests

- Fresh and version-7-migrated evidence tables both write URL and excerpt into
  the correct named columns.
- A Worth Fixing-origin seed contains all occurrence anchors, not only
  `finding_evidence`.
- An Ask-origin seed includes the accepted Ask understanding and retrieval
  grounding.
- Reconstruction rejects missing sections, but accepts best-effort evidence
  labels including simplified or unknown identifiers.
- Reconstruction allows explicit unknowns without returning `needs_user_input`.
- The second model call does not start until a valid reconstruction is persisted.
- A failed builder retry reuses reconstruction only when its fingerprint matches.
- Final validation requires `references/workflow.md` and rejects internal IDs or
  ungrounded literal URLs.
- Idempotency, immutable revisions, safe progress, export, and installation
  tests continue to pass.

### Deepika fixture backtest

Use a copied fixture rooted at:

```text
DYSTIL_DATA_DIR=/home/jayshiai/Projects/2os/capture/dystil/.local/dystil-fixtures/deepika-current-demo
```

Do not mutate the canonical fixture. Copy `db.sqlite`, `worth-fixing.sqlite`, and
their WAL sidecars into an isolated backtest root, then run the disposable URL
repair against the copy.

Required cases:

- Worth Fixing PO artifact:
  `wfa_01a10c6ef20e29d181a996a1`
- Its finding:
  `wff_993462067b22bc4e10a23b3b`
- Ask-origin RFQ artifact:
  `wfa_7803df9f88fc423684e0c734`

Run both Codex and Claude builders. Pi remains out of scope.

For the PO case, inspect the reconstruction before grading the final skill. It
should recover the supported email/attachment, browser/domain, Word template,
Excel calculation, output naming, save/print, and completion behavior while
excluding unrelated nearby activity. The final skill should choose connectors,
local tools, or browser/computer use dynamically and contain no invented URL,
path, or credential.

For the Ask-origin RFQ case, confirm that the agent either finds real retained
workflow evidence or clearly makes source discovery a runtime step. It must not
pretend the generic approved handoff contains websites or files that were never
observed.

## Required verification commands

From the repository root:

```text
cargo fmt --check
cargo test -p dystil-insights
cargo test -p dystil-ai
git diff --check
```

From `apps/dystil/` when bindings or UI change:

```text
bun run bindings:generate
bun run bindings:check
bun run test
bun run typecheck
```

## Acceptance criteria

The work is complete only when:

- clicking Build skill performs two distinct durable phases;
- the first phase produces a detailed evidence-grounded workflow reconstruction;
- the second phase demonstrably reads that reconstruction;
- generated skills contain task-specific workflow details and completion checks;
- skills actively use connectors, MCP, local tools, and browser/computer control
  according to runtime availability;
- evidence-backed URLs and names are preserved, while unsupported ones are not
  invented;
- the PO and RFQ fixture cases pass with both Codex and Claude;
- existing artifact, export, installation, privacy, deletion, idempotency, and
  frontend tests remain green;
- no screenshots are read and no external UI is opened during generation.

## Explicit non-goals

- Do not redesign runtime permissions or reduce current provider permissions.
- Do not execute or schedule the generated business workflow during generation.
- Do not add Pi acceptance work.
- Do not add a user clarification phase or `needs_user_input` result.
- Do not make Steward generate automation blueprints.
- Do not treat the generic artifact/runbook as observed workflow truth.
- Do not add a permanent legacy URL-reconciliation task to application startup.

## Implementation-agent goal message

Give the following message to the implementation agent verbatim (or use it as
the task goal):

```text
Implement the two-phase, evidence-grounded workflow reconstruction and skill
bundle generation vertical described in
agent_docs/SKILL_BUNDLE_IMPLEMENTATION_PLAN.md.

Outcome: when a user explicitly clicks Build skill for a kept shortcut, Dystil
must first conduct a deep, text-only investigation of the selected task and
persist a validated WORKFLOW.md. A second model call must then use that
reconstruction to create prompt.md plus a portable Agent Skill. The final skill
must be specific enough to perform the observed task end to end, dynamically
preferring available connectors/MCP, then local tools, then browser/computer
use with existing signed-in sessions. It must preserve only evidence-backed
websites, files, conventions, decisions, validation, and completion signals;
it must never invent them.

Keep Worth Fixing Steward as discovery only. Do not make it reconstruct or
author skills. Do not add user clarification, needs_user_input, Pi runtime
work, screenshot analysis, external UI launches, scheduling, workflow
execution, or tighter provider permissions.

Implement the complete lifecycle, not just prompts:
1. Fix future evidence projection with named INSERT columns and test fresh plus
   migrated schema column order.
2. Build comprehensive origin-aware reconstruction seeds: all relevant Worth
   Fixing occurrences and evidence, or Ask-for-Fix understanding, messages,
   retrieval memo, and grounding.
3. Add a dedicated Workflow Reconstruction Agent call, Markdown contract,
   deterministic validation, durable persistence, job stages, fingerprinting,
   deletion/reset support, and one structural repair attempt.
4. Refactor the Skill Builder into a second call that consumes the persisted
   reconstruction and the vendored upstream skill-creator instructions.
5. Require references/workflow.md in every generated skill and validate that
   portable output contains no internal Dystil provenance, no invented literal
   URLs or absolute paths, and concrete source-discovery, action, validation,
   and completion behavior.
6. Preserve existing job idempotency, immutable artifact revisions, export,
   and installation behavior. Reuse a reconstruction on retry only when its
   full fingerprint matches.
7. Backtest on copied Deepika fixture data (never mutate the canonical fixture)
   using both Codex and Claude for PO wfa_01a10c6ef20e29d181a996a1 / finding
   wff_993462067b22bc4e10a23b3b and Ask-origin RFQ
   wfa_7803df9f88fc423684e0c734. Run the disposable .local URL repair only
   against the copied fixture before the backtest.

Read the whole handoff document before changing code. Treat source code as the
truth if it conflicts with the document. Preserve unrelated dirty-worktree
changes. Update the cited agent_docs and report the exact checks and backtest
results at handoff.

Required checks:
- cargo fmt --check
- cargo test -p dystil-insights
- cargo test -p dystil-ai
- git diff --check
- plus apps/dystil bindings, unit tests, and typecheck if public bindings or UI
  are changed.

Completion means both model phases are durable and demonstrably linked, the
generated skill reflects a deep reconstruction rather than the finding proof
set, both fixture cases work with Codex and Claude, and the stated checks pass.
```
