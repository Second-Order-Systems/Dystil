/**
 * The UI data contract for the Home route.
 *
 * This is the boundary between the redesigned screens and the backend. Screens
 * import from here and never call `commands.*` or `invoke` directly, so the
 * implementation behind it can be swapped from fixtures to real Tauri commands
 * without touching a single component.
 *
 * Shape follows agent_docs/design_handoff_home_screen/README.md, "State
 * management" — not the current backend types. Several fields do not exist in
 * `WorthFixingCard` yet; see the gap table in agent_docs/UI_OVERHAUL_PLAN.md.
 */

/**
 * Where an item came from. The two origins are deliberately asymmetric:
 * a Dystil-originated item must justify interrupting you, so it leads with
 * evidence. A user-originated one was requested, so it only has to prove it
 * listened — hence recap instead.
 */
export type ItemOrigin = "dystil" | "user";

/** One numeric tile in the evidence panel. Numbers, never prose — a count can
 *  be audited in two seconds; a sentence has to be trusted. */
export type EvidenceStat = {
  /** Displayed verbatim, so "15m" and "43" are both valid. */
  n: string;
  label: string;
};

/** One row of "built from what you told me" playback. */
export type RecapRow = {
  label: string;
  text: string;
};

/** One numbered step in the fix panel. Steps exist so consent is informed —
 *  the user agrees to something specific, not a black box. */
export type FixStep = {
  n: string;
  t: string;
};

export type HomeItem = {
  id: string;
  origin: ItemOrigin;
  /** Human phrasing: "yesterday", "this morning", "Tuesday". */
  when: string;
  /** Short label for the scan list. */
  short: string;
  /** The headline. Plain-language, first person, states a fact. */
  title: string;

  /** Dystil-originated only. Exactly four tiles. */
  evidence?: EvidenceStat[];
  evidenceNote?: string;

  /** User-originated only. */
  recap?: RecapRow[];

  /** The offer heading — always a question. */
  offer: string;
  fixName: string;
  steps: FixStep[];
  /** Whether Dystil can run this itself, which drives the "DYSTIL CAN RUN
   *  THIS" badge and whether "Yes, run it" is offered. */
  runnable: boolean;
};

/**
 * Why an item was rejected. Each option states its consequence before the user
 * picks it — precision is unknown, so being corrected is how trust is kept.
 */
export type CorrectionReason =
  | "intended"      // "I meant to work that way"
  | "numbers-off"   // "The numbers are off"
  | "not-worth-it"  // "Right, but not worth fixing"
  | "stop-watching"; // "Stop watching this kind of work"

export type CorrectionOption = {
  reason: CorrectionReason;
  label: string;
  /** Shown beneath the label, before selection. Never hide this. */
  consequence: string;
};

/** A fix that is running, finished, or stopped. Drives the bottom-strip chips. */
export type JobState = "running" | "done" | "failed";

export type Job = {
  fixName: string;
  state: JobState;
  /** 1-based; only meaningful while running. */
  currentStep: number;
  totalSteps: number;
};

/** One kept artifact, shown in the bottom shelf and the shortcuts screen. */
export type Shortcut = {
  id: string;
  title: string;
  /** e.g. "Used 6x". */
  meta: string;
};

/**
 * Everything the Home route needs.
 *
 * `queue` holds ids of unsettled items with index 0 the current one. It is a
 * depleting pile, not a carousel — there is no next/previous.
 */
export type HomeData = {
  items: HomeItem[];
  queue: string[];
  /** The original queue length, so the depletion track does not shrink. */
  originalTotal: number;
  shortcuts: Shortcut[];
  job: Job | null;
  /** For state B's meta line: "LAST SPOKE UP ON TUESDAY". */
  lastSpokeUp: string;
};

/**
 * The operations the screens perform.
 *
 * Implementations must preserve two behaviours the design depends on:
 *  - `defer` moves the item to the back and does NOT change the count or the
 *    track. Deferring is not progress and must not look like it.
 *  - `settle` removing the last item resolves to the cleared state; the current
 *    item must never fall back to the first of the original stack, or the user
 *    is re-served something they already settled above a "0 left" counter.
 */
export type HomeActions = {
  settle: (id: string, reason?: CorrectionReason) => void;
  settleAndRun: (id: string) => void;
  defer: (id: string) => void;
  /** Moves an item to the front of the queue, used by the scan list. */
  bringToFront: (id: string) => void;
  /** Restores the queue — "Look at the three again" on the cleared state. */
  restore: () => void;
  stopJob: () => void;
  ask: (text: string) => void;
};

export type HomeSource = HomeData & HomeActions;
