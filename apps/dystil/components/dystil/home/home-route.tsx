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

import { useEffect, useState } from "react";
import { useRouter } from "next/navigation";
import { useHome } from "@/lib/home/provider";
import type { CorrectionReason } from "@/lib/home/types";
import { getBuildCapabilities } from "@/lib/build-capabilities";
import { NothingWaiting } from "./nothing-waiting";
import { ThePile } from "./the-pile";

export function HomeRoute() {
  const router = useRouter();
  const {
    items,
    queue,
    originalTotal,
    shortcuts,
    save,
    dismiss,
    defer,
    restore,
    loading,
    error,
  } = useHome();

  // Distinguishes "nothing waiting" from "you just cleared it" — the second
  // is a completion moment and has to be earned within this session.
  const [justCleared, setJustCleared] = useState(false);
  const [enterpriseManaged, setEnterpriseManaged] = useState<boolean | null>(null);
  useEffect(() => { void getBuildCapabilities().then((capabilities) => setEnterpriseManaged(capabilities.enterpriseManaged)); }, []);
  useEffect(() => {
    if (enterpriseManaged) router.replace("/home/ask");
  }, [enterpriseManaged, router]);

  if (enterpriseManaged) return null;

  const currentId = queue[0];
  const item = items.find((candidate) => candidate.id === currentId);

  if (loading) return <div className="mx-auto max-w-[600px] px-10 pt-12 text-body-lg text-muted-ink">Looking at the work Dystil has processed…</div>;
  if (error) return <div className="mx-auto max-w-[600px] px-10 pt-12 text-body-lg text-muted-ink">Worth fixing could not load. <button type="button" onClick={() => window.location.reload()} className="font-semibold underline">Try again</button></div>;

  if (!currentId || !item) {
    return (
      <NothingWaiting
        justCleared={justCleared}
        settledCount={originalTotal}
        shortcuts={shortcuts}
        onAsk={(text) => router.push(`/home/ask?initial=${encodeURIComponent(text.trim())}`)}
        onAllShortcuts={() => router.push("/home/ready")}
        onRestore={() => {
          setJustCleared(false);
          restore();
        }}
      />
    );
  }

  const dismissOne = (reason: CorrectionReason) => {
    if (queue.length === 1) setJustCleared(true);
    void dismiss(currentId, reason);
  };

  return (
    <ThePile
      item={item}
      remaining={queue.length}
      originalTotal={originalTotal}
      onSeeAll={() => router.push("/home/all")}
      onSave={() => {
        if (queue.length === 1) setJustCleared(true);
        void save(currentId);
      }}
      onDefer={() => defer(currentId)}
      onCorrect={dismissOne}
    />
  );
}
