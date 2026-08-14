/**
 * The droplet — the app's motif, taken from the logo.
 *
 * In the mark, the dark Y-funnel is the apparatus and the droplet is what
 * comes out of it. The droplet alone is reused as an accent at 8-11px:
 * on the primary button, the queue pill, and the "runs on this Mac" line.
 *
 * Path and viewBox are lifted verbatim from
 * agent_docs/design_handoff_home_screen/dystil.svg.
 */

type DropletProps = {
  width?: number;
  height?: number;
  /** Tailwind text-* colour class; the path fills with currentColor. */
  className?: string;
};

export function Droplet({ width = 8, height = 11, className }: DropletProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="395 640 234 310"
      fill="none"
      aria-hidden="true"
      focusable="false"
      className={className}
    >
      <path
        d="M512 650 C578 733 620 782 620 840 C620 900 572 940 512 940 C452 940 404 900 404 840 C404 782 446 733 512 650 Z"
        fill="currentColor"
      />
    </svg>
  );
}

/**
 * The full two-tone mark: dark Y-funnel plus green droplet.
 *
 * The two tones are the mark's whole idea, and at 14px the dark Y is what
 * keeps it legible — so this never renders as a single colour.
 */
export function DystilMark({ width = 14, height = 23, className }: DropletProps) {
  return (
    <svg
      width={width}
      height={height}
      viewBox="330 280 364 670"
      fill="none"
      aria-hidden="true"
      focusable="false"
      className={className}
    >
      <path
        d="M330 285 L512 570 L694 285 M512 570 L512 625"
        fill="none"
        stroke="hsl(var(--ink))"
        strokeWidth={54}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <path
        d="M512 650 C578 733 620 782 620 840 C620 900 572 940 512 940 C452 940 404 900 404 840 C404 782 446 733 512 650 Z"
        fill="hsl(var(--green-mark))"
      />
    </svg>
  );
}
