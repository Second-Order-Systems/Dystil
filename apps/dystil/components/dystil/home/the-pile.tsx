"use client";

/**
 * State A — the pile. The default Home state.
 *
 * One finding fills the screen. The user decides, and the next appears. This
 * is deliberately never a scrollable list of findings.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "State A".
 */

import { useState } from "react";
import { CORRECTION_OPTIONS } from "@/lib/mock";
import type { CorrectionReason, HomeItem } from "@/lib/mock/types";
import { Droplet } from "../primitives/droplet";
import { SegmentedTrack, pileSegments } from "../primitives/segmented-track";

type PileProps = {
  item: HomeItem;
  remaining: number;
  originalTotal: number;
  onSeeAll: () => void;
  onRun: () => void;
  onTakePrompt: () => void;
  onDefer: () => void;
  onCorrect: (reason: CorrectionReason) => void;
};

export function ThePile({
  item,
  remaining,
  originalTotal,
  onSeeAll,
  onRun,
  onTakePrompt,
  onDefer,
  onCorrect,
}: PileProps) {
  const [correcting, setCorrecting] = useState(false);

  return (
    <div className="mx-auto max-w-[650px] px-10 pt-[22px]">
      <ContextStrip
        remaining={remaining}
        originalTotal={originalTotal}
        onSeeAll={onSeeAll}
      />

      {/*
        Only user-originated items carry a badge. A finding Dystil raised has
        to justify interrupting you, so the headline leads and the timestamp
        lives in the evidence header instead.
      */}
      {item.origin === "user" ? (
        <span className="mb-3 inline-block rounded-pill bg-marigold-tint px-[10px] py-[4px] text-label-sm font-bold uppercase tracking-[0.11em] text-marigold-text">
          You asked · {item.when}
        </span>
      ) : null}

      <h1 className="mb-5 text-pretty font-display text-display font-normal text-ink">
        {item.title}
      </h1>

      {/*
        Collapsed while correcting: the user is disputing the claim, not
        re-reading it, and without collapsing the panel cannot fit the viewport.
      */}
      {!correcting ? (
        item.origin === "dystil" ? (
          <EvidencePanel item={item} />
        ) : (
          <RecapPanel item={item} />
        )
      ) : null}

      {correcting ? (
        <CorrectionPanel
          onPick={(reason) => {
            setCorrecting(false);
            onCorrect(reason);
          }}
          onBack={() => setCorrecting(false)}
        />
      ) : (
        <>
          <h2 className="mb-[14px] font-display text-display-sm font-normal text-ink">
            {item.offer}
          </h2>
          <FixPanel item={item} />
          <DecisionRow
            runnable={item.runnable}
            onRun={onRun}
            onTakePrompt={onTakePrompt}
            onDispute={() => setCorrecting(true)}
            onDefer={onDefer}
          />
        </>
      )}
    </div>
  );
}

function ContextStrip({
  remaining,
  originalTotal,
  onSeeAll,
}: {
  remaining: number;
  originalTotal: number;
  onSeeAll: () => void;
}) {
  return (
    <div className="mb-[26px] flex items-center gap-[11px]">
      <span className="font-display text-num leading-none text-ink">{remaining}</span>
      <span className="text-meta text-muted-ink">
        {remaining === 1 ? "last one" : "left to settle"}
      </span>
      {/* Segments are per ORIGINAL item, so deferring never looks like progress. */}
      <SegmentedTrack segments={pileSegments(originalTotal, originalTotal - remaining)} />
      <div className="flex-1" />
      <button
        type="button"
        onClick={onSeeAll}
        className="-mr-[10px] rounded-strip px-[10px] py-[5px] text-meta font-semibold text-ink-3 transition-colors hover:bg-chrome"
      >
        See all {remaining}
      </button>
    </div>
  );
}

/**
 * The evidence sits ABOVE the offer so the user can check before being asked
 * to decide. Numbers, never prose — a count can be audited in two seconds; a
 * sentence has to be trusted.
 */
function EvidencePanel({ item }: { item: HomeItem }) {
  return (
    <section className="mb-5 rounded-panel border border-line-2 bg-paper px-[18px] pb-[14px] pt-4">
      <div className="mb-3 flex items-center gap-[9px]">
        <span className="text-label-sm font-bold uppercase tracking-[0.12em] text-muted-ink-2">
          What I saw
        </span>
        <span className="h-[3px] w-[3px] rounded-full bg-chevron" />
        <span className="text-meta text-muted-ink">{item.when}</span>
        <div className="flex-1" />
        <button type="button" className="text-ui-sm font-semibold text-ink-3 hover:underline">
          Open the record →
        </button>
      </div>

      <div className="flex gap-2">
        {item.evidence?.map((stat) => (
          <div key={stat.label} className="flex-1 rounded-tile bg-recessed px-3 py-[10px]">
            <div className="font-display text-num leading-none text-ink">{stat.n}</div>
            <div className="mt-1 text-label text-muted-ink">{stat.label}</div>
          </div>
        ))}
      </div>

      {item.evidenceNote ? (
        <p className="mt-3 text-meta text-muted-ink">{item.evidenceNote}</p>
      ) : null}
    </section>
  );
}

/**
 * A requested answer does not need to justify interrupting you — it needs to
 * prove it listened. Hence own-words playback instead of evidence.
 */
function RecapPanel({ item }: { item: HomeItem }) {
  return (
    <section className="mb-5 rounded-panel border border-line-2 bg-paper px-[18px] pb-[14px] pt-4">
      <div className="mb-3 text-label-sm font-bold uppercase tracking-[0.12em] text-muted-ink-2">
        Built from what you told me
      </div>

      <dl className="space-y-2">
        {item.recap?.map((row) => (
          <div key={row.label} className="flex gap-3">
            <dt className="w-24 shrink-0 whitespace-nowrap text-label-sm font-bold uppercase tracking-[0.08em] text-sage">
              {row.label}
            </dt>
            <dd className="text-body text-ink-2">{row.text}</dd>
          </div>
        ))}
      </dl>

      <button type="button" className="mt-3 text-ui-sm font-semibold text-ink-3 hover:underline">
        Something here is wrong →
      </button>
    </section>
  );
}

/** The steps exist so consent is informed — the user agrees to something
 *  specific, not a black box. */
function FixPanel({ item }: { item: HomeItem }) {
  return (
    <section className="mb-4 rounded-panel border border-sage-border bg-paper px-[19px] py-[17px]">
      <div className="mb-3 flex flex-wrap items-center gap-[10px]">
        <h3 className="text-title font-semibold text-ink">{item.fixName}</h3>
        {item.runnable ? (
          <span className="rounded-badge bg-marigold-tint px-2 py-[3px] text-label-sm font-bold uppercase tracking-[0.09em] text-marigold-text">
            Dystil can run this
          </span>
        ) : null}
      </div>

      <ol className="space-y-[9px]">
        {item.steps.map((step) => (
          <li key={step.n} className="flex items-start gap-[9px]">
            <span className="mt-[2px] flex h-[19px] w-[19px] shrink-0 items-center justify-center rounded-full bg-sage-tint text-label font-bold text-sage-dark">
              {step.n}
            </span>
            <span className="text-body text-ink-2">{step.t}</span>
          </li>
        ))}
      </ol>

      <div className="mt-3 flex items-center gap-2 border-t border-line-3 pt-3">
        <Droplet width={9} height={12} className="text-green-mark" />
        <span className="text-ui-sm text-sage">
          Runs on this Mac, on material you already have. Nothing uploaded.
        </span>
      </div>
    </section>
  );
}

/**
 * Sticky: it must stay visible without scrolling, with content scrolling
 * behind it. The negative margins let its background span the column gutters.
 */
function DecisionRow({
  runnable,
  onRun,
  onTakePrompt,
  onDispute,
  onDefer,
}: {
  runnable: boolean;
  onRun: () => void;
  onTakePrompt: () => void;
  onDispute: () => void;
  onDefer: () => void;
}) {
  return (
    <div className="sticky bottom-0 -mx-10 flex items-center gap-3 border-t border-line bg-ground px-10 pb-[17px] pt-[13px]">
      {runnable ? (
        <button
          type="button"
          onClick={onRun}
          className="rounded-button bg-green-deep px-[22px] py-3 text-body font-semibold text-paper transition-colors hover:bg-green-deep-hover"
        >
          Yes, run it
        </button>
      ) : null}
      <button
        type="button"
        onClick={onTakePrompt}
        className="rounded-button border border-line-2b bg-paper px-[18px] py-3 text-body font-medium text-ink-2 transition-colors hover:bg-recessed"
      >
        Just give me the prompt
      </button>

      <div className="flex-1" />

      <button
        type="button"
        onClick={onDispute}
        className="rounded-tile bg-marigold-tint px-[15px] py-[10px] text-ui font-semibold text-marigold-text transition-colors hover:bg-marigold-hover"
      >
        This isn&apos;t right
      </button>
      {/* Rotates to the back of the queue. The count does NOT drop. */}
      <button
        type="button"
        onClick={onDefer}
        className="rounded-tile px-3 py-[10px] text-ui font-medium text-ink-3 transition-colors hover:bg-chrome"
      >
        Decide later
      </button>
    </div>
  );
}

/**
 * The most important interaction in the app. Precision is unknown, so being
 * corrected is how trust is kept — every option states its consequence before
 * the user picks it. Never reduce this to a grey "dismiss" link.
 */
function CorrectionPanel({
  onPick,
  onBack,
}: {
  onPick: (reason: CorrectionReason) => void;
  onBack: () => void;
}) {
  return (
    <div className="animate-rise">
      <h2 className="mb-2 font-display text-display-sm font-normal text-ink">
        What did I get wrong?
      </h2>
      <p className="mb-5 text-body-sm text-muted-ink">
        Each one changes what I do next. Nothing is hidden from you.
      </p>

      <div className="space-y-2 pb-5">
        {CORRECTION_OPTIONS.map((option) => (
          <button
            key={option.reason}
            type="button"
            onClick={() => onPick(option.reason)}
            className="flex w-full items-center gap-3 rounded-[12px] border border-line-2 bg-paper px-4 py-[13px] text-left transition-colors hover:border-sage hover:bg-recessed"
          >
            <span className="flex-1">
              <span className="block text-body-lg font-semibold text-ink">{option.label}</span>
              <span className="mt-1 block text-ui-sm leading-[1.45] text-muted-ink">
                {option.consequence}
              </span>
            </span>
            <span className="text-[15px] text-chevron">›</span>
          </button>
        ))}
      </div>

      <div className="sticky bottom-0 -mx-10 border-t border-line bg-ground px-10 pb-[17px] pt-[13px]">
        <button
          type="button"
          onClick={onBack}
          className="text-ui font-medium text-ink-3 hover:underline"
        >
          ← Never mind, go back
        </button>
      </div>
    </div>
  );
}
