"use client";

/**
 * The scan list behind "See all N".
 *
 * The pile is deliberately one-at-a-time; this is the escape hatch for someone
 * who wants to survey what is waiting and pick. Selecting an item brings it to
 * the front of the queue and returns to the pile — it does not settle anything.
 *
 * Spec: the `isAll` block of
 * agent_docs/design_handoff_home_screen/Dystil App v2.dc.html.
 */

import { useRouter } from "next/navigation";
import { useHome } from "@/lib/mock/provider";
import { SegmentedTrack, pileSegments } from "../primitives/segmented-track";

export function ScanView() {
  const router = useRouter();
  const { items, queue, originalTotal, bringToFront } = useHome();

  const waiting = queue
    .map((id) => items.find((item) => item.id === id))
    .filter((item): item is NonNullable<typeof item> => Boolean(item));

  const open = (id: string) => {
    bringToFront(id);
    router.push("/home");
  };

  return (
    <div className="flex h-full flex-col">
      <div className="flex h-[44px] shrink-0 items-center border-b border-line px-[30px]">
        <span className="mr-[11px] flex items-baseline gap-[5px]">
          <span className="font-display text-num-sm leading-none text-ink">{queue.length}</span>
          <span className="text-meta text-muted-ink">
            {queue.length === 1 ? "last one" : "left to settle"}
          </span>
        </span>
        <SegmentedTrack segments={pileSegments(originalTotal, originalTotal - queue.length)} />
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => router.push("/home")}
          className="rounded-icon px-[11px] py-[5px] text-ui-sm font-medium text-ink-3 transition-colors hover:bg-chrome hover:text-ink"
        >
          Take them one at a time →
        </button>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-[700px] flex-col gap-2 px-10 pb-[50px] pt-[22px]">
          {waiting.length === 0 ? (
            <p className="px-1 py-[26px] text-body-lg text-muted-ink">
              Nothing waiting. You have settled everything I found.
            </p>
          ) : null}

          {waiting.map((item) => (
            <button
              key={item.id}
              type="button"
              onClick={() => open(item.id)}
              className="flex w-full items-center gap-4 rounded-[12px] border border-line-2 bg-paper px-[18px] py-[15px] text-left transition-shadow hover:border-sage-border hover:shadow-card-hover"
            >
              {/*
                Fixed-width column so the titles line up. The scan list labels
                both origins — unlike the pile, where a Dystil-originated item
                deliberately carries no badge and lets the headline lead.
              */}
              <span className="w-[94px] shrink-0">
                {item.origin === "user" ? (
                  <span className="inline-block whitespace-nowrap rounded-[4px] bg-marigold-tint px-[7px] py-[3px] text-[10px] font-bold uppercase tracking-[0.08em] text-marigold-text">
                    You asked
                  </span>
                ) : (
                  <span className="inline-block whitespace-nowrap rounded-[4px] bg-sage-tint px-[7px] py-[3px] text-[10px] font-bold uppercase tracking-[0.08em] text-sage">
                    I noticed
                  </span>
                )}
              </span>
              <span className="min-w-0 flex-1 text-body-lg leading-[1.4] text-ink">
                {item.short}
              </span>
              <span className="shrink-0 text-ui-sm text-muted-ink">{item.when}</span>
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
