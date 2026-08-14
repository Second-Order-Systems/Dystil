"use client";

/**
 * "Your shortcuts" — the kept-artifact library.
 *
 * Called "Ready to use" before the v2 design; the handoff renames it, and the
 * top bar's count badge points here.
 *
 * Spec: the `isToolkit` block of
 * agent_docs/design_handoff_home_screen/Dystil App v2.dc.html.
 */

import { useRouter } from "next/navigation";
import { useHome } from "@/lib/mock/provider";

/** The headline counts in words, so it reads as a sentence rather than a stat. */
const WORDS = ["No", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine"];
const countWord = (n: number) => WORDS[n] ?? String(n);

export function YourShortcuts() {
  const router = useRouter();
  const { shortcuts } = useHome();
  const runnable = shortcuts.filter((shortcut) => shortcut.runnable).length;

  return (
    <div className="mx-auto max-w-[760px] px-10 pb-[50px] pt-8">
      <div className="mb-[26px] flex items-center gap-[11px]">
        <span className="text-meta text-ink-2">
          <span className="font-semibold">{shortcuts.length} kept</span> · {runnable} I can run
          myself
        </span>
        <div className="flex-1" />
        <button
          type="button"
          onClick={() => router.push("/home/ask")}
          className="-mr-[10px] whitespace-nowrap rounded-strip px-[10px] py-[5px] text-meta font-semibold text-ink-3 transition-colors hover:bg-chrome hover:text-ink"
        >
          Ask for another
        </button>
      </div>

      <h1 className="mb-[26px] max-w-[30ch] text-pretty font-display text-display font-normal text-ink">
        {shortcuts.length === 0
          ? "Nothing kept yet."
          : `${countWord(shortcuts.length)} ${
              shortcuts.length === 1 ? "thing" : "things"
            } you no longer do by hand.`}
      </h1>

      <div className="flex flex-col gap-[9px]">
        {shortcuts.map((shortcut) => (
          <div
            key={shortcut.id}
            className="flex items-center gap-[18px] rounded-[12px] border border-line-2 bg-paper px-[18px] py-[15px] transition-shadow hover:border-sage-border hover:shadow-card-hover"
          >
            <div className="min-w-0 flex-1">
              <div className="mb-[3px] flex items-center gap-[9px]">
                <span className="truncate text-[15px] font-semibold text-ink">
                  {shortcut.title}
                </span>
                <span className="shrink-0 rounded-[4px] bg-line-3 px-[7px] py-[2px] text-[10px] font-bold uppercase tracking-[0.09em] text-muted-ink">
                  {shortcut.kind}
                </span>
              </div>
              <div className="text-ui-sm text-muted-ink">{shortcut.meta}</div>
            </div>

            {shortcut.runnable ? (
              <button
                type="button"
                className="shrink-0 rounded-icon bg-green-deep px-4 py-2 text-ui-sm font-semibold text-paper transition-colors hover:bg-green-deep-hover"
              >
                Run it
              </button>
            ) : null}
            <button
              type="button"
              className="shrink-0 rounded-icon px-[13px] py-2 text-ui-sm font-medium text-ink-3 transition-colors hover:bg-chrome hover:text-ink"
            >
              Copy
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}
