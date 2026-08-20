"use client";

/**
 * State B — nothing waiting, and State C — just cleared.
 *
 * At one or two findings a week, B is the app's normal state, so the ask box
 * is the product rather than a fallback. C exists because finishing has to
 * feel like something: before it, clearing the queue produced no
 * acknowledgement at all.
 *
 * Spec: agent_docs/design_handoff_home_screen/README.md, "State B" / "State C".
 */

import { useState } from "react";
import type { Shortcut } from "@/lib/home/types";
import { SegmentedTrack } from "../primitives/segmented-track";

const STARTERS = [
  "Something takes me too long every week",
  "I keep redoing the same thing",
  "I want a shortcut for a specific job",
];

type NothingWaitingProps = {
  /** C is B plus a completion moment; they share almost all their markup. */
  justCleared: boolean;
  settledCount: number;
  shortcuts: Shortcut[];
  onAsk: (text: string) => void;
  onAllShortcuts: () => void;
  onRestore: () => void;
};

export function NothingWaiting({
  justCleared,
  settledCount,
  shortcuts,
  onAsk,
  onAllShortcuts,
  onRestore,
}: NothingWaitingProps) {
  const [draft, setDraft] = useState("");

  return (
    <div className={`flex h-full flex-col ${justCleared ? "animate-rise" : ""}`}>
      <div className="mx-auto flex w-full max-w-[600px] flex-1 flex-col justify-center px-10 pb-10 pt-5">
        {justCleared && (
          <div className="mb-[14px] flex items-center gap-[11px]">
            <SegmentedTrack segments={Array(settledCount).fill("settled")} />
            <span className="text-label font-bold uppercase tracking-[0.13em] text-muted-ink-2">
              All settled · four minutes
            </span>
          </div>
        )}

        {/* An app that says nothing today is proving it will not cry wolf.
            State that in words rather than showing an empty list. */}
        <h1 className="text-pretty font-display text-display-lg font-normal leading-[1.24] tracking-[-0.018em] text-ink">
          {justCleared
            ? "That's the lot. Nothing else waiting."
            : "Nothing new worth interrupting you about."}
        </h1>
        <p className="mb-7 mt-3 max-w-[44ch] text-pretty text-body-lg text-muted-ink">
          {justCleared
            ? "I'll knock when I find the next one — probably not today. While you're here, anything else dragging?"
            : "I'll knock when I find something. If something is dragging right now, tell me and we'll work it out together."}
        </p>

        {/* The hero. Roominess is the invitation to ramble, so this is a
            multi-line textarea and never a single-line input. */}
        <div
          className={`mb-[15px] flex flex-col rounded-card border border-line-2b bg-paper px-5 pb-3 pt-[18px] shadow-hero transition-shadow focus-within:border-sage-hover focus-within:shadow-hero-hover hover:border-sage-hover hover:shadow-hero-hover ${
            justCleared ? "min-h-[132px]" : "min-h-[138px]"
          }`}
        >
          <textarea
            value={draft}
            onChange={(event) => setDraft(event.target.value)}
            placeholder="What's slowing you down?"
            aria-label="What's slowing you down?"
            className="min-h-[56px] flex-1 resize-none bg-transparent text-hero leading-[1.5] text-ink outline-none placeholder:text-muted-ink-2"
          />
          <div className="mt-3 flex items-center gap-3 border-t border-line-3 pt-3">
            <span className="text-ui-sm text-muted-ink">
              Say it however it comes out — I&apos;ll ask a few questions after.
            </span>
            <div className="flex-1" />
            <button
              type="button"
              onClick={() => onAsk(draft)}
              className="rounded-button bg-green-deep px-6 py-[11px] text-body font-semibold text-paper transition-colors hover:bg-green-deep-hover"
            >
              Ask
            </button>
          </div>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <span className="text-ui-sm text-muted-ink">Or start here —</span>
          {STARTERS.map((starter) => (
            <button
              key={starter}
              type="button"
              onClick={() => setDraft(starter)}
              className="rounded-pill bg-chrome px-[14px] py-[7px] text-ui text-ink-3 transition-colors hover:bg-sage-tint hover:text-sage-dark"
            >
              {starter}
            </button>
          ))}
          {justCleared ? (
            <button
              type="button"
              onClick={onRestore}
              className="ml-auto rounded-pill px-3 py-[7px] text-ui-sm font-semibold text-ink-3 hover:underline"
            >
              Look at them again
            </button>
          ) : null}
        </div>
      </div>

      {/* C has no bottom shelf — the completion moment should not compete. */}
      {!justCleared ? (
        <div className="border-t border-line px-10 pb-[18px] pt-[15px]">
          <div className="mx-auto max-w-[600px]">
            <div className="mb-2 flex items-center">
              <span className="text-label-sm font-bold uppercase tracking-[0.12em] text-muted-ink-2">
                Your shortcuts
              </span>
              <div className="flex-1" />
              <button
                type="button"
                onClick={onAllShortcuts}
                className="text-ui-sm font-semibold text-ink-3 hover:underline"
              >
                All {shortcuts.length} →
              </button>
            </div>
            <div className="flex gap-2">
              {shortcuts.slice(0, 3).map((shortcut) => (
                <div
                  key={shortcut.id}
                  className="min-w-0 flex-1 rounded-button border border-line-2 bg-paper px-[13px] py-[10px] transition-shadow hover:shadow-card-hover"
                >
                  <div className="truncate text-ui font-medium text-ink">{shortcut.title}</div>
                  <div className="mt-[2px] text-label text-muted-ink">{shortcut.meta}</div>
                </div>
              ))}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
