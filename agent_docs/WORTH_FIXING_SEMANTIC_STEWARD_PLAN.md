---
status: unreviewed
authority: approved-implementation-plan
verified_against: 17be863
verified_on: 2026-08-13
---

# Worth Fixing semantic Steward implementation plan

This document pins the approved design for making Worth Fixing distinguish
meaningful repeated user work from repeated observations. The target behavior is
an approved future change, not a claim that it already exists. Current-state claims
are cited to code by path and symbol, following `AGENTS.md`.

## Outcome

The Explorer will describe intentional activity without promoting capture or
interface mechanics into work. The Steward will first discover candidate work
patterns, then group distinct episodes, and finally classify each candidate as:

- `qualified`: meaningful and mature enough to produce a finding;
- `watching`: meaningful, but not mature enough to produce a finding;
- `discarded`: not a credible Worth Fixing opportunity.

Different suppliers, items, documents, identifiers, prices, recipients, or portal
records may be variable inputs to one repeated work pattern. The Steward must match
shared goals and core transformations, not exact nouns. Conversely, use of the same
application or broad business domain is not enough to merge unrelated work.

The deterministic kernel remains authoritative for evidence ownership, construct
thresholds, cadence, capability verification, handoff completeness, ranking, and
selection. No adversarial model or second semantic judge is added.

## Current baseline

- Explorer v2 receives admitted compact evidence and returns timestamped,
  evidence-linked observations. Citation:
  `crates/dystil-insights/src/engine.rs :: run_explorer_batch_inner()` and
  `crates/dystil-insights/resources/explorer_prompt_v2.md`.
- Steward v3 receives pending observations plus bounded durable memory. Citation:
  `crates/dystil-insights/src/engine.rs :: StewardPacket` and
  `run_steward_wake_inner()`.
- Bounded memory currently contains up to ten recent non-retired opportunities,
  their durable occurrence counts, up to three recent occurrence outlines, and ten
  recent user dispositions. Citation:
  `crates/dystil-insights/src/store.rs :: steward_memory()` and its call
  `steward_memory(pool, 10, 3)` in
  `crates/dystil-insights/src/engine.rs :: run_steward_wake_inner()`.
- An occurrence already supports multiple observation and evidence IDs. Citation:
  `crates/dystil-insights/src/types.rs :: OccurrenceDelta`.
- The kernel counts stored occurrence rows, not Explorer observations, when deriving
  construct eligibility. Citation:
  `crates/dystil-insights/src/store.rs :: opportunity_occurrence_sets()` and
  `crates/dystil-insights/src/kernel.rs :: derive_eligibility()`.
- A later Steward wake can update durable memory by returning
  `existing_opportunity_id` and occurrence deltas. Citation:
  `crates/dystil-insights/src/types.rs :: OpportunityDelta` and
  `crates/dystil-insights/src/store.rs :: apply_reconciliation_inner()`.
- Accepted structured Steward output is already stored atomically as
  `reconciliations.output_json`. Citation:
  `crates/dystil-insights/src/store.rs` — `CREATE TABLE IF NOT EXISTS reconciliations`
  and `apply_reconciliation_inner()`.
- The backfill binary can run the production Explorer and Steward over a read-only
  capture database, and has a Steward-only mode for pending observations. Citation:
  `crates/dystil-insights/src/bin/worth_fixing_backfill.rs :: Args` and `main()`.

## Backtest evidence motivating the change

The private August 6–7 replay in
`.local/ambient-harness/backtest-worth-fixing-v3-aug06.sqlite` processed 13,030
evidence records through 66 Explorer calls and produced 270 accepted observations.
The accepted repaired Steward result produced no opportunity, occurrence, or
finding. The observations nevertheless contain repeated procurement work across
mail, Excel, Word, supplier material, and the procurement portal.

This private result is not a repository fixture and must not be copied into tracked
artifacts. It establishes the evaluation target: avoid capture-noise false positives
without discarding meaningful but not-yet-mature work.

The date CLI uses inclusive `from_date` and `through_date` bounds. Therefore a true
single-local-day replay uses the same date for both arguments. Citation:
`crates/dystil-insights/src/bin/worth_fixing_backfill.rs :: TimeBounds::from_args()`.

## Approved responsibility split

### Explorer

Explorer describes observable intentional activity, transfers, decisions, authored
content, and outcomes. It consolidates a continuous interaction where the evidence
supports doing so. It does not describe scrolling, focus, accessibility-tree
changes, text reappearance, capture events, or telemetry as user work unless the
evidence establishes that investigating those mechanics was the user's task.

Keep Explorer v2 unless backtesting shows that it still emits capture mechanics as
activity. The current semantic false-negative is primarily a Steward grouping and
retention problem.

### Steward

The Steward performs these steps in order:

1. Cluster observations into candidate work patterns.
2. Establish each candidate's shared user goal, output or transformation, and
   reducible burden.
3. Separate distinct work episodes from multiple observations of one episode.
4. Identify stable core steps and variable inputs.
5. Propose a handoff that directly reduces the established burden.
6. Classify the candidate as `qualified`, `watching`, or `discarded`.

Thresholds are applied after candidate discovery. Insufficient repetition must not
erase otherwise meaningful work: it produces a `watching` opportunity with explicit
`unresolved_questions`. A future wake may attach a distinct occurrence and promote
that opportunity through the existing durable-memory path.

### Kernel

The kernel continues to reject malformed output, foreign or overlapping evidence,
below-threshold findings, unsupported cadence, unverified capabilities, and missing
handoffs. Candidate assessments are explanatory audit metadata; they never override
kernel eligibility.

## Superseded prompt contract

This historical v4 proposal is superseded by the packet-local reference design in
`.local/STEWARD_PACKET_LOCAL_OBSERVATION_REFS_PLAN.md`. Do not implement from this
section.

The prompt must include the following contract. Wording may be tightened for token
economy, but the distinctions and ordering are normative.

```text
You are Dystil's Worth Fixing Steward. Reconcile supplied observations into
meaningful opportunities for reducing repeated user work.

Return only the requested JSON. Use only supplied observations, cited evidence,
bounded opportunity memory, and verified capabilities. Preserve uncertainty.
Never invent activity, counts, cadence, completion, or user intent. The
deterministic kernel owns evidence admission, identities, eligibility thresholds,
cadence validation, ranking, and final selection.

WORTH FIXING

Worth Fixing concerns intentional user work with:
1. a recognizable user goal;
2. an output, transfer, decision, or result;
3. effort, risk, repeated judgment, or manual coordination that could be reduced;
4. a handoff that directly helps perform that same work.

Repeated observations are not necessarily repeated work. Interface states,
scrolling, focus changes, accessibility-tree changes, text reappearance, capture
events, telemetry, and application mechanics are not user work. Technical
mechanics qualify only when the evidence establishes that diagnosing or changing
them was itself the user's intentional task.

DISCOVER PATTERNS BEFORE APPLYING THRESHOLDS

First cluster observations that plausibly belong to the same kind of work. Compare
their goal, inputs, outputs, transfers, decisions, and core steps.

Do not require every occurrence to involve the same supplier, item, document,
identifier, price, recipient, or application state. These may be variable inputs to
the same work pattern.

For example, separate procurement requests may represent one recurring work
pattern when each involves substantially the same process of reviewing a request,
collecting supplier or quotation information, recording or calculating values,
entering proposal details, and preparing or submitting a response.

Do not merge activity merely because it uses the same application, document type,
or business domain. The goal and core transformation must match.

DISTINCT OCCURRENCES

An occurrence is one distinct episode of intentional user work. It is not one
observation, frame, event, screen change, text change, or recurrence signal.

Group observations from one continuous task episode into one occurrence. An
occurrence may and usually should contain multiple observation and evidence IDs.
Do not split an episode because the user changed windows, applications, documents,
focus, scroll position, visible text, or workflow steps.

Separate occurrences only when the evidence supports a new instance of the work:
a different request or input was handled, a prior result was completed, or the user
left and later began another instance. When distinctness is unclear, count the
evidence as one occurrence.

DECISIONS

Classify every plausible candidate as qualified, watching, or discarded.

qualified: Meaningful work, the applicable occurrence threshold, and a directly
usable handoff are established. Include an opportunity and finding.

watching: A meaningful goal and plausible reducible burden are established, but
distinct occurrences, completion, or handoff specificity are insufficient for a
finding. Include an opportunity with no finding and state exactly what evidence is
still needed in unresolved_questions. Do not discard meaningful work merely because
it is not mature enough to surface.

discarded: The candidate is observation noise, interface or capture mechanics, a
one-off action without plausible reusable help, unrelated activity sharing only an
application or topic, an unsupported interpretation, or work with no concrete
reducible burden. Do not create an opportunity.

THRESHOLDS

Recognition needs one occurrence and an immediately useful generic prompt,
instruction, runbook, or verified capability.

Manual transfer needs one established directional transfer from a source into a
different destination and a handoff that reduces that transfer burden.

Unchanged repetition needs two distinct work episodes with the same goal and
substantially the same core steps.

Temporal pattern needs three distinct work episodes and evidence-supported daily,
weekly, or monthly cadence.

Repeated composition needs three distinct authored outputs with the same purpose or
structure. Viewing content is not authorship.

HANDOFF FIT

Every qualified finding needs a complete, bounded, directly usable handoff that
addresses the established burden. Do not return a generic suggestion that could
apply to any activity.

For prompts or saved prompts, return copy-ready prompt text. For runbooks, return
ordered executable steps, prerequisites, variable inputs, and a completion signal.
For an existing capability, reference only a supplied verified capability ID.

Prefer a small number of coherent opportunities. Prefer watching over a speculative
finding, and discarded over treating observation mechanics as work.
```

## Candidate-assessment schema

Version the Steward output schema. Add a bounded `candidate_assessments` array to
`ReconciliationOutput`; do not add a new database table.

Each assessment contains:

```json
{
  "local_id": "candidate_01",
  "observation_ids": ["obl_..."],
  "decision": "qualified",
  "reason_code": "meaningful_repeated_work",
  "reason": "Distinct requests share the same goal and core transfer steps.",
  "shared_goal": "Prepare and submit a commercially complete proposal",
  "reducible_burden": "Repeatedly transfer and verify commercial terms",
  "stable_steps": ["review request", "collect quotation", "enter terms"],
  "variable_inputs": ["item", "supplier", "price", "tax", "delivery"],
  "distinct_episode_basis": ["different request identifiers"],
  "missing_to_qualify": [],
  "opportunity_local_id": "opp_01"
}
```

Use these decision values:

- `qualified`
- `watching`
- `discarded`

Use these reason codes:

- `meaningful_repeated_work`
- `meaningful_but_immature`
- `system_mechanics`
- `no_user_goal`
- `no_recognizable_output`
- `no_reducible_burden`
- `same_episode`
- `different_goals`
- `insufficient_evidence`
- `uncertain_completion`
- `handoff_not_grounded`
- `one_off_without_reusable_help`

Schema constraints:

- maximum 20 assessments per wake;
- every assessment has at least one supplied observation ID;
- assessment observation IDs are unique within that assessment;
- assessments may not overlap observation IDs;
- `qualified` and `watching` assessments must reference exactly one emitted
  opportunity by `opportunity_local_id`;
- `discarded` assessments must use `opportunity_local_id: null`;
- every emitted opportunity must be linked from exactly one assessment;
- `qualified` must have a finding;
- `watching` must have no finding and at least one `missing_to_qualify` item;
- `discarded` must have no opportunity;
- assessments explain candidate clusters, not every incidental observation;
- `considered_observation_ids` remains the exact all-observation coverage receipt.

Validate these relations in deterministic Rust code before applying any opportunity.
An invalid assessment gets the existing one-repair opportunity; it must never be
partially persisted.

Because `apply_reconciliation_inner()` already serializes the complete accepted
`ReconciliationOutput` into `reconciliations.output_json` in the same transaction as
opportunity changes, candidate assessments become durable audit metadata without a
migration or another table.

## Steward-only replay support

Add an explicit developer replay mode that can reuse accepted Explorer evidence and
observations from one insights database while writing Steward state into a fresh
insights database. It must not call Explorer or mutate the source database.

Suggested CLI contract:

```text
worth_fixing_backfill
  --steward-replay-source <existing-insights.sqlite>
  --insights-db <fresh-output.sqlite>
  --codex <managed-codex-path>
  --timezone <offset>
  --steward-observation-limit <n>
```

Implementation requirements:

1. Open the source insights database read-only.
2. Copy only evidence cited by observations and preserve its immutable identity and
   admission flags.
3. Copy observations through the existing evidence/observation admission functions,
   preserving IDs, timestamps, certainty, and evidence ownership.
4. Do not copy Explorer jobs, prior Steward jobs, opportunities, occurrences,
   findings, dispositions, reconciliations, or cursors.
5. Run normal Steward replay wakes against the fresh destination.
6. Print usage totals for accepted and invalid attempts separately, grouped by
   Explorer and Steward stage.
7. Refuse a destination that already contains Steward state unless an explicit
   resumable replay identity matches. Never overwrite an existing database.

This replay path permits prompt comparisons without repeating the 66 Explorer calls
or their token cost.

## Diagnostic export

Add a developer-only export or query helper that reads
`reconciliations.output_json` and emits:

- candidate assessments by decision and reason code;
- linked opportunity and finding IDs;
- observation count per assessment;
- occurrence count and distinctness basis;
- unresolved evidence needed for watching candidates;
- model usage and latency for every Steward attempt, including rejected repairs.

Do not expose this diagnostic material in the product UI or telemetry.

## Tests

Add focused Rust tests for:

1. Same episode: many observations from one continuous interaction form one
   occurrence and cannot satisfy unchanged repetition.
2. Variable inputs: two procurement requests with different items and suppliers but
   the same goal and core steps may form two occurrences of one opportunity.
3. Broad-domain rejection: two unrelated actions in email or Excel do not merge.
4. System mechanics: capture, scrolling, focus, accessibility, and reappearance
   activity is discarded without an opportunity.
5. Watching retention: meaningful one-occurrence work persists with no finding and
   explicit missing evidence.
6. Promotion: a later wake references the watching opportunity, adds a distinct
   occurrence, and becomes eligible through the existing kernel.
7. Assessment linkage: qualified/watching/discarded relations obey the schema rules.
8. Atomicity: invalid diagnostics do not persist opportunities or a reconciliation.
9. Durability: accepted assessments survive in `reconciliations.output_json` and
   projection rebuild remains deterministic.
10. Replay isolation: the source database remains byte-identical and the destination
    receives no prior Steward state.

Do not add private captured text as tracked fixtures. Use compact synthetic inputs in
unit tests and the ignored `.local/` database for human backtesting.

## Evaluation sequence

1. Implement Steward v4, candidate assessments, deterministic validation, and
   Steward-only replay.
2. Copy the accepted August 6–7 Explorer inputs from the existing private backtest
   database into a fresh destination.
3. Run only Steward v4.
4. Review candidate assessments, especially Mail → Excel/Word → procurement portal
   patterns.
5. Confirm that capture/interface mechanics are discarded.
6. Confirm that meaningful but immature work is retained as watching rather than
   erased.
7. Run a true August 6-only end-to-end replay separately with
   `--from-date 2026-08-06 --through-date 2026-08-06` only after the Steward-only
   result is satisfactory.

## Acceptance criteria

- No finding or watching opportunity is created from scroll-area reappearance,
  accessibility changes, capture events, or one continuous burst of observations.
- A continuous task produces at most one occurrence unless a new instance is
  evidenced.
- Different procurement entities can be grouped when their goal and core steps
  match, while unrelated use of Mail or Excel remains separate.
- Meaningful one-occurrence work can remain watching and load on a later wake.
- A later distinct occurrence can promote that same durable opportunity.
- Every emitted opportunity has one explanatory assessment.
- Every discarded candidate has a bounded reason code and evidence-linked
  explanation in the accepted reconciliation output.
- Zero opportunities is acceptable only when candidate assessments explain why no
  meaningful candidate warranted even watching status.
- The runtime adds no second model judge and no product-visible diagnostics.
- Steward-only comparison incurs no Explorer model calls.

## Explicitly out of scope

- A production adversarial or grounding judge.
- New opportunity or diagnostic database tables.
- UI display of discard reasons.
- Telemetry containing observations, assessments, prompts, findings, or evidence.
- Keyword-based hard rejection of technical work.
- Treating a fixed time gap as conclusive semantic episode identity.
