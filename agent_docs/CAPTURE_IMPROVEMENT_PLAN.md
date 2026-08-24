---
status: unreviewed
---

# Windows capture improvement plan

This branch contains the validated pre-semantic capture work only. The later
semantic-frame experiments are preserved on
`backup/semantic-capture-20260824`.

## Scope retained here

1. Remove the unused periodic/focus background UIA tree stream while keeping
   capture-triggered UIA walks and click target enrichment.
2. Merge delayed UIA click enrichment into its physical click record. Genuine
   rapid clicks remain separate actions.
3. Settle bursty click, scroll, and typing activity before capturing; emit
   bounded checkpoints during sustained activity; never capture periodically
   while genuinely idle.
4. Persist compact activity spans so rapid scrolling, typing, and app/window
   transitions remain observable even when intermediate frames are omitted.
5. Reuse exact unchanged surface frames and link events/spans to the reused
   frame instead of writing duplicate frame rows.
6. Capture visible/relevant foreground UIA content with bounded walks, improve
   browser URL and clicked-window context, and retain truncation diagnostics.
7. Keep the standalone real-app fixture and report tooling for repeatable
   baseline-versus-candidate validation. Fixture-owned apps and tabs are closed
   after a run unless explicitly retained.

## Validation gates

- One stored click per physical click, with precise element context where UIA
  enrichment succeeds; rapid physical clicks are not collapsed.
- No focus/periodic background tree attempts in candidate policies.
- No idle capture requests.
- Scroll and typing bursts plus app/window transitions are represented by
  linked events or activity spans with a final frame.
- Three distinct Gmail facts, browser URL evidence, Explorer/editor activity,
  typing pauses, sustained scrolling, and rapid app switching are observed by
  the real-app fixture.
- Every stored event and activity span expected to carry evidence links to a
  persisted or reused frame.
- Compare frame count, text bytes, duplicate ratios, UIA walk time, mean/peak
  CPU, and peak RSS against the unchanged baseline policy.

## Deferred

Semantic frame rendering, app/site classifiers, compact semantic deltas, and
semantic-specific holdout fixtures are intentionally not part of this branch.
They remain available on the backup branch for separate design and review.
