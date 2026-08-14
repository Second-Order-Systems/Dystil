/**
 * The segmented track — one device meaning "progress through a finite thing".
 *
 * The design reuses this for the pile, run steps and question count, so it
 * lives here rather than in any one screen.
 * Spec: agent_docs/design_handoff_home_screen/README.md, "The segmented track".
 *
 * Segments are 15x4px, radius 2, gap 3px. Colours are fixed by meaning:
 *   settled  ink        a thing that is done
 *   active   green mark the one currently running
 *   stopped  marigold   halted and awaiting a decision
 *   waiting  line-2c    not yet reached
 */

export type SegmentState = "settled" | "active" | "stopped" | "waiting";

const SEGMENT_CLASS: Record<SegmentState, string> = {
  settled: "bg-ink",
  active: "bg-green-mark",
  stopped: "bg-marigold",
  waiting: "bg-line-2c",
};

type SegmentedTrackProps = {
  segments: SegmentState[];
  /**
   * Describes the track for assistive tech. Omit when an adjacent visible
   * count already says the same thing — the track is then decorative and is
   * hidden instead, so a screen reader does not read the number twice.
   */
  label?: string;
  className?: string;
};

export function SegmentedTrack({ segments, label, className }: SegmentedTrackProps) {
  return (
    <div
      className={`flex items-center gap-[3px] ${className ?? ""}`}
      {...(label ? { role: "img", "aria-label": label } : { "aria-hidden": true })}
    >
      {segments.map((state, index) => (
        <span
          key={index}
          className={`h-1 w-[15px] rounded-track ${SEGMENT_CLASS[state]}`}
        />
      ))}
    </div>
  );
}

/**
 * The pile's track: one segment per item in the ORIGINAL total, filling left
 * to right as items are settled. Deliberately built from the original total
 * rather than the remaining count — deferring an item must not look like
 * progress.
 */
export function pileSegments(total: number, settled: number): SegmentState[] {
  return Array.from({ length: total }, (_, index) =>
    index < settled ? "settled" : "waiting",
  );
}
