"use client";

/**
 * Home is one route with three states, chosen by queue length.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "Overview".
 *
 * Guards the emptiness bugs the handoff calls out by construction: the current
 * item is read from `queue[0]` and never falls back to the first of the
 * original stack, so an empty queue can only ever resolve to the cleared
 * state — the user can never be re-served something they already settled
 * above a "0 left" counter.
 */

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useHome } from "@/lib/mock/provider";
import type { CorrectionReason } from "@/lib/mock/types";
import { NothingWaiting } from "./nothing-waiting";
import { ThePile } from "./the-pile";

export function HomeRoute() {
  const router = useRouter();
  const {
    items,
    queue,
    originalTotal,
    shortcuts,
    lastSpokeUp,
    settle,
    settleAndRun,
    defer,
    restore,
  } = useHome();

  // Distinguishes "nothing waiting" from "you just cleared it" — the second
  // is a completion moment and has to be earned within this session.
  const [justCleared, setJustCleared] = useState(false);

  const currentId = queue[0];
  const item = items.find((candidate) => candidate.id === currentId);

  if (!currentId || !item) {
    return (
      <NothingWaiting
        justCleared={justCleared}
        lastSpokeUp={lastSpokeUp}
        settledCount={originalTotal}
        shortcuts={shortcuts}
        onAsk={() => router.push("/home/ask")}
        onAllShortcuts={() => router.push("/home/ready")}
        onRestore={() => {
          setJustCleared(false);
          restore();
        }}
      />
    );
  }

  const settleOne = (reason?: CorrectionReason) => {
    if (queue.length === 1) setJustCleared(true);
    settle(currentId, reason);
  };

  return (
    <ThePile
      item={item}
      remaining={queue.length}
      originalTotal={originalTotal}
      onSeeAll={() => router.push("/home/all")}
      onRun={() => {
        if (queue.length === 1) setJustCleared(true);
        settleAndRun(currentId);
      }}
      onTakePrompt={() => settleOne()}
      onDefer={() => defer(currentId)}
      onCorrect={(reason) => settleOne(reason)}
    />
  );
}
