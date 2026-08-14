"use client";

/**
 * Mock implementation of the Home data contract.
 *
 * Screens consume `useHomeSource()` and know nothing about where the data came
 * from. Replacing this with real Tauri commands means implementing the same
 * `HomeSource` interface — no component changes.
 *
 * Not wired to the backend on purpose: several fields the design needs do not
 * exist in `WorthFixingCard` yet (evidence stat tiles, origin, recap, steps,
 * runnable). The gap table is in agent_docs/UI_OVERHAUL_PLAN.md.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  FIXTURE_ITEMS,
  FIXTURE_LAST_SPOKE_UP,
  FIXTURE_SHORTCUTS,
} from "./fixtures";
import type { CorrectionReason, Job, HomeSource } from "./types";

export * from "./types";
export {
  CORRECTION_OPTIONS,
  FIXTURE_STARTERS,
} from "./fixtures";

/**
 * Fixtures must never reach users. `NODE_ENV` is "production" for
 * `bun run build`, so a release build gets real data unless someone explicitly
 * opts in with NEXT_PUBLIC_DYSTIL_MOCK=1 for a demo build.
 */
export const isMockEnabled =
  process.env.NODE_ENV !== "production" ||
  process.env.NEXT_PUBLIC_DYSTIL_MOCK === "1";

/** How long each fixture job step takes, so Running is actually watchable. */
const STEP_MS = 2200;

export function useHomeSource(): HomeSource {
  const items = useMemo(() => FIXTURE_ITEMS, []);
  const originalTotal = items.length;

  const [queue, setQueue] = useState<string[]>(() => items.map((item) => item.id));
  const [job, setJob] = useState<Job | null>(null);
  const timer = useRef<ReturnType<typeof setInterval> | null>(null);

  const clearTimer = useCallback(() => {
    if (timer.current) {
      clearInterval(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => clearTimer, [clearTimer]);

  const settle = useCallback((id: string, _reason?: CorrectionReason) => {
    setQueue((current) => current.filter((queued) => queued !== id));
  }, []);

  /**
   * Moves the item to the back. The count and the depletion track are derived
   * from how many items have been *settled*, never from queue position, so
   * deferring cannot look like progress.
   */
  const defer = useCallback((id: string) => {
    setQueue((current) =>
      current.length < 2 ? current : [...current.filter((queued) => queued !== id), id],
    );
  }, []);

  const bringToFront = useCallback((id: string) => {
    setQueue((current) => [id, ...current.filter((queued) => queued !== id)]);
  }, []);

  const restore = useCallback(() => {
    setQueue(items.map((item) => item.id));
  }, [items]);

  const stopJob = useCallback(() => {
    clearTimer();
    setJob((current) => (current ? { ...current, state: "failed" } : null));
  }, [clearTimer]);

  const settleAndRun = useCallback(
    (id: string) => {
      const item = items.find((candidate) => candidate.id === id);
      if (!item) return;

      setQueue((current) => current.filter((queued) => queued !== id));
      clearTimer();
      setJob({
        fixName: item.fixName,
        state: "running",
        currentStep: 1,
        totalSteps: item.steps.length,
      });

      timer.current = setInterval(() => {
        setJob((current) => {
          if (!current || current.state !== "running") return current;
          if (current.currentStep >= current.totalSteps) {
            clearTimer();
            return { ...current, state: "done" };
          }
          return { ...current, currentStep: current.currentStep + 1 };
        });
      }, STEP_MS);
    },
    [items, clearTimer],
  );

  const ask = useCallback((_text: string) => {
    // The Ask flow is a separate screen and is not part of this phase.
  }, []);

  return {
    items,
    queue,
    originalTotal,
    shortcuts: FIXTURE_SHORTCUTS,
    job,
    lastSpokeUp: FIXTURE_LAST_SPOKE_UP,
    settle,
    settleAndRun,
    defer,
    bringToFront,
    restore,
    stopJob,
    ask,
  };
}
