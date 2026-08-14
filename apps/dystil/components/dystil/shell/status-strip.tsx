"use client";

/**
 * The 34px bottom status strip.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "Shell".
 *
 * Green is kept to the dot and the tick only — the status text stays ink and
 * warm grey so it never competes with the primary action in the top bar.
 */

import type { Job } from "@/lib/mock/types";

type StatusStripProps = {
  job: Job | null;
  onPrivacy: () => void;
  onStopJob: () => void;
  onOpenResult: () => void;
};

export function StatusStrip({ job, onPrivacy, onStopJob, onOpenResult }: StatusStripProps) {
  const running = job?.state === "running";

  return (
    <footer className="relative flex h-[34px] shrink-0 items-center gap-2 border-t border-line-2b bg-chrome px-3">
      {/* Indeterminate progress, riding the top edge of the strip. */}
      {running ? (
        <div className="absolute inset-x-0 top-[-1px] h-[2px] overflow-hidden bg-line-2b">
          <div className="h-full w-1/4 animate-crawl bg-green-mark" />
        </div>
      ) : null}

      {/* Static text, not a button. */}
      <span className="relative flex h-[6px] w-[6px] items-center justify-center">
        <span className="absolute h-[6px] w-[6px] animate-halo rounded-full bg-green-mid" />
        <span className="relative h-[6px] w-[6px] rounded-full bg-green-mid" />
      </span>
      <span className="text-meta font-semibold text-ink-2">Watching, locally</span>
      <span className="text-meta text-muted-ink">nothing has left this Mac</span>

      <button
        type="button"
        onClick={onPrivacy}
        className="rounded-strip border border-line-2c px-[10px] py-[4px] text-meta font-semibold text-ink-3 transition-colors hover:border-chevron hover:bg-line-2 hover:text-ink"
      >
        What stays on this computer →
      </button>

      <div className="flex-1" />

      {job ? (
        <div className="flex items-center gap-2">
          {job.state === "running" ? (
            <>
              <span className="h-[6px] w-[6px] animate-dot-pulse rounded-full bg-green-mark" />
              <span className="text-meta font-semibold text-ink-2">Running</span>
              <span className="text-meta text-muted-ink">
                {job.fixName} — step {job.currentStep} of {job.totalSteps}
              </span>
              <button
                type="button"
                onClick={onStopJob}
                className="text-meta font-semibold text-marigold-text"
              >
                Stop
              </button>
            </>
          ) : (
            <>
              <span className="flex h-[14px] w-[14px] items-center justify-center rounded-full bg-green-mid">
                <svg width="8" height="8" viewBox="0 0 8 8" fill="none" aria-hidden="true">
                  <path
                    d="M1.5 4.2 3 5.7 6.5 2.2"
                    stroke="hsl(var(--paper))"
                    strokeWidth="1.6"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </span>
              <span className="text-meta font-semibold text-ink-2">{job.fixName} finished</span>
              <button
                type="button"
                onClick={onOpenResult}
                className="text-meta text-muted-ink underline-offset-2 hover:underline"
              >
                see what came back
              </button>
            </>
          )}
        </div>
      ) : null}
    </footer>
  );
}
