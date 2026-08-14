/**
 * Fixture data for the Home route.
 *
 * Content is taken verbatim from the design prototype
 * (agent_docs/design_handoff_home_screen/Dystil App v2.dc.html) so the screens
 * can be checked against the reference render pixel for pixel.
 *
 * Two items are Dystil-originated (evidence, no badge) and one is
 * user-originated (recap, "You asked" badge) — the design needs both to show
 * that the two origins are treated asymmetrically.
 *
 * These never reach a release build; see `isMockEnabled` in ./index.ts.
 */

import type { CorrectionOption, HomeItem, Shortcut } from "./types";

export const FIXTURE_ITEMS: HomeItem[] = [
  {
    id: "item-profiles",
    origin: "dystil",
    when: "yesterday",
    short: "Nine profiles, four different ways",
    title: "You went at the same nine profiles four different ways — about two hours.",
    evidence: [
      { n: "14", label: "searches" },
      { n: "31", label: "posts read" },
      { n: "9", label: "reposts" },
      { n: "9", label: "profiles" },
    ],
    evidenceNote: "Four separate passes, no shared notes between them.",
    offer: "Want a shortcut for next time?",
    fixName: "One review pass over all four",
    steps: [
      { n: "1", t: "Gather the nine profiles and everything you read about them into one place." },
      { n: "2", t: "List the claims each makes, and mark which are backed by something you saw." },
      { n: "3", t: "Hand you the leftovers — the claims nothing supports." },
    ],
    runnable: true,
  },
  {
    id: "item-synthesis",
    origin: "user",
    when: "Tuesday",
    short: "The synthesis problem you described",
    title: "Here is the thing you asked for — a way to stop synthesis eating your week.",
    recap: [
      { label: "Problem", text: "Research takes too long." },
      { label: "Where", text: "Pulling it together. Collecting is fine." },
      { label: "Who reads it", text: "Your team, sometimes a client." },
    ],
    offer: "This is what I would build for you.",
    fixName: "Friday digest from the week you already read",
    steps: [
      { n: "1", t: "Collect what you read this week, grouped by the question you were chasing." },
      { n: "2", t: "Draft one page per question, in the plain register your team reads in." },
      { n: "3", t: "Flag anything you looked at once and never came back to." },
    ],
    runnable: true,
  },
  {
    id: "item-login-loop",
    origin: "dystil",
    when: "this morning",
    short: "A login window repeating itself",
    title: "A login window re-announced itself forty-three times in fifteen minutes.",
    evidence: [
      { n: "43", label: "signals" },
      { n: "15m", label: "window" },
      { n: "1", label: "app" },
      { n: "0", label: "real changes" },
    ],
    evidenceNote: "Same interval each time, which is what makes me think it is noise.",
    offer: "Want me to find out why?",
    fixName: "Trace the repeating capture events",
    steps: [
      { n: "1", t: "Replay the interval against a login screen with no real credentials." },
      { n: "2", t: "Watch which app focus change triggers the repeat." },
      { n: "3", t: "Tell you whether it is safe to ignore." },
    ],
    runnable: true,
  },
];

/**
 * Copy is exact and load-bearing. Each option states what happens BEFORE the
 * user picks it — this is the most important interaction in the app, and it
 * must never be reduced to a grey "dismiss" link.
 */
export const CORRECTION_OPTIONS: CorrectionOption[] = [
  {
    reason: "intended",
    label: "I meant to work that way",
    consequence: "Then it is not waste. I will stop calling this a problem.",
  },
  {
    reason: "numbers-off",
    label: "The numbers are off",
    consequence: "I will show you what I counted so you can tell me where I slipped.",
  },
  {
    reason: "not-worth-it",
    label: "Right, but not worth fixing",
    consequence: "Noted. I will keep spotting it and keep quiet about it.",
  },
  {
    reason: "stop-watching",
    label: "Stop watching this kind of work",
    consequence: "It comes off the list, from now on.",
  },
];

export const FIXTURE_SHORTCUTS: Shortcut[] = [
  { id: "sc-1", title: "One review pass over all four", meta: "Used 6 times", kind: "Runbook", runnable: true },
  { id: "sc-2", title: "Friday digest from the week you already read", meta: "Used 3 times", kind: "Prompt", runnable: true },
  { id: "sc-3", title: "Turn website notes into a decision brief", meta: "Used twice", kind: "Prompt", runnable: false },
  { id: "sc-4", title: "Trace the repeating capture events", meta: "Used once", kind: "Runbook", runnable: true },
];

/** State B's meta line. */
export const FIXTURE_LAST_SPOKE_UP = "Tuesday";

/** Starter chips offered when there is nothing waiting. */
export const FIXTURE_STARTERS = [
  "Something takes me too long every week",
  "I keep redoing the same thing",
  "I want a shortcut for a specific job",
];
