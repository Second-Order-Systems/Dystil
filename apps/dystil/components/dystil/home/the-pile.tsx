"use client";

/**
 * State A — the pile. The default Home state.
 *
 * One finding fills the screen. The user decides, and the next appears. This
 * is deliberately never a scrollable list of findings.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "State A".
 */

import { useState } from "react";
import { CORRECTION_OPTIONS } from "@/lib/home/types";
import type { CorrectionReason, HomeItem } from "@/lib/home/types";
import { FindingEvidence } from "../worth-fixing/finding-evidence";
import { Droplet } from "../primitives/droplet";
import { SegmentedTrack, pileSegments } from "../primitives/segmented-track";

type PileProps = {
  item: HomeItem;
  remaining: number;
  originalTotal: number;
  onSeeAll: () => void;
  onSave: () => void;
  onDefer: () => void;
  onCorrect: (reason: CorrectionReason) => void;
};

export function ThePile({
  item,
  remaining,
  originalTotal,
  onSeeAll,
  onSave,
  onDefer,
  onCorrect,
}: PileProps) {
  const [correcting, setCorrecting] = useState(false);
  const [evidenceOpen, setEvidenceOpen] = useState(false);

  return (
    <div className="mx-auto max-w-[650px] px-10 pt-[22px]">
      <ContextStrip
        remaining={remaining}
        originalTotal={originalTotal}
        onSeeAll={onSeeAll}
      />

      <h1 className="mb-5 text-pretty font-display text-display font-normal text-ink">
        {item.title}
      </h1>

      {/*
        Collapsed while correcting: the user is disputing the claim, not
        re-reading it, and without collapsing the panel cannot fit the viewport.
      */}
      {!correcting ? <EvidencePanel item={item} open={evidenceOpen} onToggle={() => setEvidenceOpen((open) => !open)} /> : null}

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
            saveAvailable={item.saveAvailable}
            onSave={onSave}
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
function EvidencePanel({ item, open, onToggle }: { item: HomeItem; open: boolean; onToggle: () => void }) {
  return (
    <section className="mb-5 rounded-panel border border-line-2 bg-paper px-[18px] pb-[14px] pt-4">
      <div className="mb-3 flex items-center gap-[9px]">
        <span className="text-label-sm font-bold uppercase tracking-[0.12em] text-muted-ink-2">
          What I saw
        </span>
        <span className="h-[3px] w-[3px] rounded-full bg-chevron" />
        <span className="text-meta text-muted-ink">{item.when}</span>
        <div className="flex-1" />
        <button type="button" aria-expanded={open} onClick={onToggle} className="text-ui-sm font-semibold text-ink-3 hover:underline">
          {open ? "Hide the record" : "Open the record →"}
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
      {open ? <FindingEvidence findingId={item.id} /> : null}
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
          Built from material Dystil observed locally. Nothing uploaded.
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
  saveAvailable,
  onSave,
  onDispute,
  onDefer,
}: {
  saveAvailable: boolean;
  onSave: () => void;
  onDispute: () => void;
  onDefer: () => void;
}) {
  return (
    <div className="sticky bottom-0 -mx-10 flex items-center gap-3 border-t border-line bg-ground px-10 pb-[17px] pt-[13px]">
      <button
        type="button"
        disabled={!saveAvailable}
        onClick={onSave}
        className="rounded-button bg-green-deep px-[22px] py-3 text-body font-semibold text-paper transition-colors hover:bg-green-deep-hover disabled:cursor-not-allowed disabled:bg-line-2b"
      >
        Save to shortcuts
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
