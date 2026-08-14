# Handoff: Dystil — Home screen (three states)

## Overview

Dystil is a macOS desktop app that watches a person's work locally, notices repeated or wasteful patterns, and proposes a fix. This handoff covers the **Home route** — what the user sees when they open the app.

Home is **not one screen.** It is one route with three states, chosen by how many unsettled items are in the queue:

| State | Condition | What it is |
|---|---|---|
| **A — The pile** | `queue.length > 0` | One finding fills the screen. Decide, and the next appears. This is the default. |
| **B — Nothing waiting** | `queue.length === 0` and the user did not just clear it | "Nothing new worth interrupting you about." The ask box is the centrepiece. |
| **C — Just cleared** | `queue.length === 0` immediately after settling the last item | A completion moment, then the ask box. |

If you implement only one, implement **A**. B and C share almost all their markup.

## About the design files

`Dystil App v2.dc.html` in this folder is a **design reference built in HTML** — a prototype showing intended look and behaviour. It is not production code to copy. Recreate these designs in the target codebase's existing environment (React/Electron/SwiftUI/etc.) using its established patterns, component library, and state management. If the codebase has no established UI layer yet, pick the framework that fits the app and build there.

Open the file in a browser. A "States" switcher sits above the window frame — it is a **review control only, not part of the product.** Do not implement it. Use it to see every state.

## Fidelity

**High-fidelity.** Colours, type, spacing, radii, shadows and copy are final. Recreate pixel-accurately, but use the codebase's existing primitives where they already match (buttons, cards). Every hex value and measurement below is exact.

---

## Shell (present in all three states)

Vertical flex column, full window height.

### 1. Title bar — 38px
- Background `#EFEBE1`, `border-bottom: 1px solid #E1DBCD`
- macOS traffic lights only. If the real app uses a native title bar, use that instead and drop this.

### 2. Top bar — 54px, `padding: 0 22px`, `gap: 10px`, no bottom border

Left to right:
- **Logo button** — droplet mark 14×23 + wordmark "DYSTIL" at `12.5px/600`, `letter-spacing: .19em`, `#3A372F`. Click → Home. Hover background `#EFEBE1`, radius 8.
- **Queue pill** — *only when `queue.length > 0` AND the current state is not A.* Background `#F8EDD8`, radius 20, `padding: 6px 12px 6px 10px`, droplet 8×11 `#C98A2B`, label `11.5px/700 #8A5A11 white-space: nowrap`, text `"{n} worth fixing"`. Click → state A. Hover `#F2E2C5`.
  - **Guard:** never render this at `queue.length === 0`.
- Flex spacer
- **"Invite your team"** — `13px/500 #3A372F`, `padding: 8px 13px`, radius 9, 14×12 person+plus icon (strokes `#4C6B58`, plus `#2F9E7B`). Hover background `#EFEBE1`.
- **"Your shortcuts"** + count badge — label `13px/500 #3A372F`; badge `10.5px/700 #726C5E` on `#E7E2D6`, `padding: 1px 6px`, radius 20.
- **"Ask for a fix"** — PRIMARY. Background `#0E4F3C`, text `#FFFEFB 13px/600`, `padding: 9px 17px`, radius 9, `box-shadow: 0 2px 6px -2px rgba(14,79,60,.45)`, leading droplet 8×11 `#8FD9BE`. Hover `#0A3B2C`. **Must be reachable from every screen** — it is the app's one persistent action.
- **Settings** — 32×32 icon button, radius 8, two-slider glyph, strokes `#5E5A51` at 1.5. Hover `#EFEBE1`.

### 3. Content area
`flex: 1`, `overflow-y: auto`, `min-height: 0`.

### 4. Bottom status strip — 34px, `position: relative`
- Background `#EFEBE1`, `border-top: 1px solid #E1DBCD`, `padding: 0 12px`, `gap: 8px`
- **Watching status (static text, not a button):** 6px dot `#2F6B4A` with a `halo` pulse behind it; `"Watching, locally"` `11.5px/600 #3A372F`; `"nothing has left this Mac"` `11.5px #6B6558`
- **"What stays on this computer →"** — outlined button, `11.5px/600 #5E5A51`, `border: 1px solid #DCD6C8`, `padding: 4px 10px`, radius 7. Hover background `#E7E2D6`, text `#1B1A17`, border `#CFC8B8`.
- Flex spacer
- **Job chips (only when a fix is running or just finished):**
  - Running: 6px `#2F9E7B` dot with `pulse`; `"Running"` `11.5px/600 #3A372F`; `"{fix name} — step 2 of 3"` `11.5px #6B6558`; plus a **"Stop"** button `11.5px/600 #8A5A11`
  - Running also shows a **2px indeterminate bar** absolutely positioned at `top: -1px, left: 0, right: 0`, track `#E1DBCD`, a 25%-wide `#2F9E7B` fill running the `crawl` animation
  - Finished: 14px `#2F6B4A` circle with a white tick; `"{fix name} finished"` `11.5px/600 #3A372F`; `"see what came back"` `11.5px #6B6558`
- Keep green here to **the dot/tick only** — status text stays ink and warm grey so it never competes with the primary action in the top bar.

---

## State A — The pile (default)

**Purpose:** settle one item. The user decides, and the next item appears. Never a scrollable list of findings.

**Layout:** single centred column, `max-width: 650px`, `margin: 0 auto`, `padding: 22px 40px 0`.

### A1. Context strip
`display: flex; align-items: center; gap: 11px; margin-bottom: 26px`
- **Count:** remaining number in Newsreader `21px/1 #1B1A17`, then the word in `11.5px #6B6558`.
  - `queue.length > 1` → `"left to settle"`  ·  `queue.length === 1` → `"last one"`
- **Depletion track:** one segment per item in the ORIGINAL total (not the remaining count). Each `15×4px`, radius 2, `gap: 3px`.
  - Settled → `#1B1A17` · Waiting → `#DCD6C8`
  - Segments fill left to right as items are settled.
- Flex spacer
- **"See all {n}"** — `11.5px/600 #5E5A51`, `padding: 5px 10px`, radius 7, `margin-right: -10px`. Opens the scan list.

### A2. Source badge — only for user-originated items
Items have two origins and they are deliberately asymmetric:
- **Dystil-originated ("I noticed"):** **no badge and no eyebrow.** The headline leads. The timestamp lives in the evidence panel header instead.
- **User-originated ("You asked"):** badge `10.5px/700` uppercase, `letter-spacing: .11em`, `#8A5A11` on `#F8EDD8`, `padding: 4px 10px`, radius 20, text `"You asked · {when}"`, `margin-bottom: 12px`.

### A3. Headline
Newsreader `400`, `33px`, `line-height: 1.26`, `letter-spacing: -.015em`, `margin: 0 0 20px`, `text-wrap: pretty`. Plain-language, first person, states a fact.

### A4a. Evidence panel — Dystil-originated items only
Background `#FFFEFB`, `border: 1px solid #E7E2D6`, radius 13, `padding: 16px 18px 14px`, `margin-bottom: 20px`.
- Header row (`gap: 9px`): `"WHAT I SAW"` `10.5px/700` uppercase `.12em` `#726C5E` · 3px dot `#C6C0B2` · timestamp `11.5px #6B6558` · right-aligned **"Open the record →"** `12px/600 #5E5A51`
- **Four stat tiles:** `display: flex; gap: 8px`, each `flex: 1`, background `#F6F3EC`, radius 9, `padding: 10px 12px`. Number Newsreader `21px/1`; label `11px #6B6558`, `margin-top: 4px`.
- Note line `11.5px #6B6558`
- **Why it matters:** the evidence sits ABOVE the offer. The user checks before being asked to decide. Numbers, never prose — a count can be audited in two seconds; a sentence has to be trusted.

### A4b. Recap panel — user-originated items only
Same shell as A4a. Label `"BUILT FROM WHAT YOU TOLD ME"`. Rows: label column `width: 96px; white-space: nowrap`, `10.5px/700` uppercase `.08em` `#4C6B58`; value `14px/1.45 #3A372F`. Footer link **"Something here is wrong →"** `12px/600 #5E5A51`.
- A requested answer doesn't need to justify interrupting you — it needs to prove it listened. Hence own-words playback instead of evidence.

### A5. Offer heading
Newsreader `400`, `24px`, `letter-spacing: -.01em`, `margin: 0 0 14px`. A question: *"Want a shortcut for next time?"*

### A6. Fix panel
Background `#FFFEFB`, `border: 1px solid #D2E0D4` (sage — marks it as the actionable one), radius 13, `padding: 17px 19px`, `margin-bottom: 16px`.
- Name `15.5px/600` + badge `"DYSTIL CAN RUN THIS"` `10.5px/700` uppercase `.09em` `#8A5A11` on `#F8EDD8`, radius 5, `padding: 3px 8px`
- **Numbered steps** (`gap: 9px`): 19px circle, background `#E4EDE5`, text `#24503B 11px/700`; step text `14px/1.45 #3A372F`
- Reassurance footer: `padding-top: 12px`, `border-top: 1px solid #F1EDE4`, droplet 9×12 `#2F9E7B`, text `12px #4C6B58` — *"Runs on this Mac, on material you already have. Nothing uploaded."*
- The steps exist so consent is informed. The user agrees to something specific, not a black box.

### A7. Decision row — STICKY
`position: sticky; bottom: 0; margin: 0 -40px;` background `#F4F1EA`; `border-top: 1px solid #E4DFD4`; `padding: 13px 40px 17px`.

One row, four controls:
| Control | Style | Action |
|---|---|---|
| **"Yes, run it"** | bg `#0E4F3C`, `#FFFEFB`, `14px/600`, `padding: 12px 22px`, radius 10, hover `#0A3B2C` | remove item from queue → Running |
| **"Just give me the prompt"** | `border: 1px solid #E1DBCD`, bg `#FFFEFB`, `14px/500 #3A372F`, `padding: 12px 18px`, radius 10 | remove item from queue |
| *(flex spacer)* | | |
| **"This isn't right"** | `13px/600 #8A5A11` on `#F8EDD8`, `padding: 10px 15px`, radius 9, hover `#F2E2C5` | open correction panel |
| **"Decide later"** | `13px/500 #5E5A51`, `padding: 10px 12px`, radius 9 | rotate item to back of queue — count does NOT drop |

**Must stay visible without scrolling.** Content scrolls behind it. Reserve bottom padding on scrollable content equal to this row's height, or the last element will hide behind it.

### A8. Correction panel — replaces A5–A7 when open
Opened by "This isn't right". **When open, collapse the evidence/recap panel (A4)** — the user is disputing the claim, not re-reading it, and without collapsing, the panel cannot fit the viewport.
- Heading Newsreader `24px` — *"What did I get wrong?"*
- Sub `13.5px #6B6558` — *"Each one changes what I do next. Nothing is hidden from you."*
- Four option buttons: bg `#FFFEFB`, `border: 1px solid #E7E2D6`, radius 12, `padding: 13px 16px`; label `14.5px/600`; **consequence line** `12.5px/1.45 #6B6558`; chevron `15px #C6C0B2`. Hover border `#4C6B58`, bg `#FBFAF6`. Container `padding-bottom: 20px`.
- Sticky footer: **"← Never mind, go back"** `13px/500 #5E5A51`
- **This is the most important interaction in the app.** Precision is unknown, so being corrected is how trust is kept. Each option states its consequence *before* the user picks it. Never reduce this to a grey "dismiss" link.

**Option copy (exact):**
1. "I meant to work that way" → *Then it is not waste. I will stop calling this a problem.*
2. "The numbers are off" → *I will show you what I counted so you can tell me where I slipped.*
3. "Right, but not worth fixing" → *Noted. I will keep spotting it and keep quiet about it.*
4. "Stop watching this kind of work" → *It comes off the list, from now on.*

Picking any option settles the item (removes it from the queue).

---

## State B — Nothing waiting

**Purpose:** the ask box is the product. At 1–2 findings a week this is the app's normal state.

**Layout:** `max-width: 600px`, `margin: 0 auto`, vertically centred (`flex: 1; justify-content: center`), `padding: 20px 40px 40px`. Plus a bottom shelf.

- **Meta line:** `"LAST SPOKE UP ON TUESDAY"` `11px/700` uppercase `.13em` `#726C5E`, `margin-bottom: 14px`. Do **not** repeat the watching status here — the bottom strip owns it.
- **Headline:** Newsreader `400`, `36px`, `line-height: 1.24`, `letter-spacing: -.018em` — *"Nothing new worth interrupting you about."* An app that says nothing today is proving it won't cry wolf; state it in words.
- **Sub:** `14.5px/1.55 #6B6558`, `max-width: 44ch`, `margin-bottom: 28px` — *"I'll knock when I find something. If something is dragging right now, tell me and we'll work it out together."*
- **Writing box** — the hero. `display: flex; flex-direction: column`, background `#FFFEFB`, `border: 1px solid #DDD6C7`, radius 14, `padding: 18px 20px 12px`, `box-shadow: 0 3px 14px -6px rgba(40,34,20,.16)`, **`min-height: 138px`**, `margin-bottom: 15px`. Hover/focus: border `#B9CFBD`, shadow `0 10px 28px -12px rgba(40,34,20,.26)`.
  - Placeholder `16.5px/1.5 #726C5E` — *"What's slowing you down?"*
  - Inner footer: `padding-top: 12px`, `border-top: 1px solid #F1EDE4`; hint `12.5px #6B6558` — *"Say it however it comes out — I'll ask a few questions after."*; **"Ask"** button bg `#0E4F3C`, `14px/600`, `padding: 11px 24px`, radius 10
  - Must be a multi-line textarea, not a single-line input. The roominess is the invitation to ramble.
- **Starters:** `"Or start here —"` `12.5px #6B6558`, then chips `13px #5E5A51` on `#EFEBE1`, `padding: 7px 14px`, radius 20, hover bg `#E4EDE5` text `#24503B`:
  - "Something takes me too long every week" · "I keep redoing the same thing" · "I want a shortcut for a specific job"
- **Bottom shelf:** `border-top: 1px solid #E4DFD4`, `padding: 15px 40px 18px`, inner `max-width: 600px`
  - Header: `"YOUR SHORTCUTS"` `10.5px/700` uppercase `.12em` `#726C5E`; right **"All 4 →"** `12px/600 #5E5A51`
  - Three mini cards, `display: flex; gap: 8px`, each `flex: 1`: bg `#FFFEFB`, `border: 1px solid #E7E2D6`, radius 10, `padding: 10px 13px`; title `13px/500` with ellipsis overflow; meta `11px #6B6558` (e.g. "Used 6×")

---

## State C — Just cleared

Identical to B except:
- **Strip:** three FILLED track segments (`15×4px`, `#1B1A17`, `gap: 3px`) + `"ALL THREE SETTLED · FOUR MINUTES"` `11px/700` uppercase `.13em` `#726C5E`
- **Headline:** *"That's the lot. Nothing else waiting."*
- **Sub:** *"I'll knock when I find the next one — probably not today. While you're here, anything else dragging?"*
- Writing box `min-height: 132px`
- Starters row gains a right-aligned (`margin-left: auto`) **"Look at the three again"** `12.5px/600 #5E5A51`, `padding: 7px 12px`, radius 20 — restores the queue
- **No bottom shelf**
- Wrapper animates in with `rise` (0.5s)

Finishing has to feel like something. Before this state existed the queue looped forever and clearing it produced no acknowledgement.

---

## Interactions & behaviour

### Queue mechanics
The queue is a **depleting pile, not a carousel.** There is no next/previous.
- `settle()` — remove `queue[0]`. If the queue is now empty → state C.
- `settleAndRun()` — remove `queue[0]`, start the job, go to Running.
- `defer()` — move `queue[0]` to the end. **Count and track do not change** — deferring is not progress and the UI must not imply it is.
- Scan list → selecting an item moves it to the front of the queue and returns to state A.

### Emptiness guards (bugs to avoid)
- `queue.length === 0` must resolve to state C everywhere. Do not let the current item fall back to `stack[0]` — the user will be re-served something they already settled, above a "0 left" counter.
- Hide the top-bar queue pill entirely at `queue.length === 0`.
- The scan list needs its own empty line: *"Nothing waiting. You have settled everything I found."*

### Animations
| Name | Spec | Used by |
|---|---|---|
| `halo` | 3.6–4s ease-in-out infinite; `opacity .22→.5`, `scale 1→1.22` | watching dot |
| `pulse` | 1.4–1.6s ease-in-out infinite; `opacity 1→.35` | running dot, active step |
| `crawl` | 2.2s linear infinite; `translateX(-100% → 400%)` on a 25%-wide bar | indeterminate progress |
| `rise` | 0.3–0.5s ease; `opacity 0→1`, `translateY(8px→0)` | correction panel, cleared state |

Hover transitions are instantaneous (no declared transition) in the prototype; adding ~120ms ease on background/border is fine.

## State management

```
screen   : 'stack' | 'cleared' | 'running' | 'done' | 'failed' | 'home' | 'ask' | 'all' | 'shortcuts' | 'privacy' | 'invite'
queue    : number[]            // ids of unsettled items, index 0 is current
correcting : boolean           // correction panel open; collapses the evidence panel
job      : null | 'running' | 'done'   // drives the bottom-strip chips
```

Derived: `remaining = queue.length` · `item = stack[queue[0]]` · `isStack = screen === 'stack' && queue.length > 0` · `isCleared = screen === 'cleared' || (screen === 'stack' && queue.length === 0)` · `showQueuePill = screen !== 'stack' && queue.length > 0`

**Item shape:**
```
{ origin: 'dystil' | 'user', when: string, title: string,
  evidence: [{ n: string, label: string }],   // dystil only, 4 tiles
  evidenceNote: string,                        // dystil only
  recap: [{ label: string, text: string }],    // user only
  offer: string, fixName: string, steps: [{ n, t }], runnable: boolean }
```

Data fetching: findings come from the local daemon. The counts in the evidence panel must be real — "Open the record" is expected to show the underlying observations (that screen is **not yet designed**; wire the button but it can be a stub).

---

## Design tokens

### Colour
| Token | Hex | Use |
|---|---|---|
| Ground | `#F4F1EA` | main app background |
| Chrome | `#EFEBE1` | title bar, bottom strip, hover fills |
| Paper | `#FFFEFB` | cards, panels, inputs |
| Recessed | `#F6F3EC` | panels inside paper (stat tiles) |
| Ink | `#1B1A17` | primary text, filled track |
| Ink 2 | `#3A372F` | body inside panels |
| Ink 3 | `#5E5A51` | secondary buttons |
| Muted | `#6B6558` | supporting copy |
| Muted 2 | `#726C5E` | labels, placeholders |
| Line | `#E4DFD4` | dividers |
| Line 2 | `#E7E2D6` / `#E1DBCD` / `#DCD6C8` | panel borders, chrome borders, strong borders |
| Line 3 | `#F1EDE4` | dividers inside paper |
| Green deep | `#0E4F3C` | primary buttons (hover `#0A3B2C`) |
| Green mark | `#2F9E7B` | droplet, live progress |
| Green mid | `#2F6B4A` | status dot, step ticks |
| Sage | `#4C6B58` | reassurance text |
| Sage dark | `#24503B` | text on sage tint |
| Sage tint | `#E4EDE5` (border `#D2E0D4`, hover `#B9CFBD`) | privacy/actionable surfaces |
| Marigold | `#C98A2B` | droplet accents, stopped segment |
| Marigold text | `#8A5A11` | text on marigold tint |
| Marigold tint | `#F8EDD8` (hover `#F2E2C5`) | "aha" and correction affordances |

**Colour rules:** green = you can act / it is local and safe. Marigold = something needs your judgement (a finding, a correction, a stop). Never use green for navigation or movement. Keep at most one green element per region — the bottom strip gets a dot, not green text.

### Type
- **Display / serif:** Newsreader, weights 300–500. Headlines 33–36px, sub-heads 24px, numbers 19–22px. Always `letter-spacing: -.015em` to `-.02em` on large sizes.
- **UI:** Instrument Sans, weights 400–700.
- Scale in use: `36 / 33 / 24 / 21 / 19 / 16.5 / 15.5 / 14.5 / 14 / 13.5 / 13 / 12.5 / 11.5 / 11 / 10.5`
- Uppercase labels: `10.5–11px`, weight `700`, `letter-spacing .08–.13em`
- Body line-height `1.45–1.6`; headline `1.24–1.26`
- Use `text-wrap: pretty` on every headline and paragraph
- **Minimum readable size is 11px and only for uppercase labels.** Never below 4.5:1 contrast — `#726C5E` is the lightest permissible grey on ground/paper.

### Spacing / radius / shadow
- Spacing: `3 4 6 7 8 9 10 11 12 13 14 16 18 20 22 26 28 34 40`
- Radius: `2` track · `5` small badge · `7` strip button · `8` icon button · `9` panel tile & buttons · `10` primary button · `13` panel · `14` card/hero · `20` pill/chip · `50%` dots
- Shadows: card hover `0 6px 18px -10px rgba(40,34,20,.2)` · hero input `0 3px 14px -6px rgba(40,34,20,.16)` → hover `0 10px 28px -12px rgba(40,34,20,.26)` · primary button `0 2px 6px -2px rgba(14,79,60,.45)`

### The segmented track (reusable)
One device meaning "progress through a finite thing" — used for the pile, run steps, and question count. Segments `15×4px`, radius 2, `gap: 3px`.
`settled: #1B1A17` · `active: #2F9E7B` · `stopped: #C98A2B` · `waiting: #DCD6C8`

---

## Assets

- **Logo** — `dystil.svg`, included in this folder. Two-tone deliberately: the Y-funnel in `#14201B` is the apparatus, the droplet in `#2F9E7B` is what comes out of it. **Do not make the Y green** — the two-tone is the mark's whole idea, and at 14px the dark Y is what keeps it legible.
- **The droplet path** (from the logo) is the app's motif — reused at 8–11px for accents. Extract it as an icon component:
  `M512 650 C578 733 620 782 620 840 C620 900 572 940 512 940 C452 940 404 900 404 840 C404 782 446 733 512 650 Z` on `viewBox="395 640 234 310"`
- **Fonts** — Google Fonts: `Newsreader` (opsz 6–72, wght 300–500) and `Instrument Sans` (400–700). Self-host if the app must work offline.
- All other icons are inline SVG paths in the prototype (settings sliders, tick, person+plus, chevron). Substitute the codebase's existing icon set if it has equivalents.

## Files

- `Dystil App v2.dc.html` — the full prototype, all states. Home states are the `stack`, `home` and `cleared` blocks.
- `dystil.svg` — logo source
- Not in scope for this handoff but present in the prototype: Running, What came back, Stopped partway, Scan view, Ask for a fix, Your shortcuts, What stays on this computer, Invite your team, and the macOS notification.

## Not yet designed

Wire these buttons but expect stubs: **"Open the record"** (the raw observations behind the evidence counts) and **"Open the full result"**. First-run onboarding also does not exist yet.
