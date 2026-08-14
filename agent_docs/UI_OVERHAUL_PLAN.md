---
status: verified
authority: proposed-implementation-plan
verified_against: de6723a
verified_on: 2026-08-14
---

# UI overhaul plan — Home route to the v2 design

> **Read this first.** Current-state claims below are verified against the code and cited by path +
> symbol. The **target** design is a proposal pending approval — it is not a claim that any of it
> exists yet. Nothing in the "Target" sections describes shipped behavior.

## Context

The desktop app's product screens have drifted from any coherent design system, and a high-fidelity
redesign now exists. Three findings make this more than a reskin.

**1. The app has two disjoint visual systems.** `/onboarding`, `/permission-recovery`,
`components/auth/*` and every `components/ui/*` primitive use the CSS-variable token layer defined in
`app/globals.css` and mapped in `tailwind.config.ts`, and they render dark mode correctly. The four
screens users actually live in — `components/dystil/pages/{worth-fixing,ready-to-use,ask-for-fix,privacy}.tsx`
— bypass that layer: roughly 285 hardcoded `text-[Npx]` sizes across 18 files against ~99 token-scale
uses, plus raw hex (`#087b5f`, `#0f6e56`, `#fdfdfc`). They are light-only and cannot render dark.

**2. The oversized type has a specific cause.** The token scale in `app/globals.css`
(`--text-xs` … `--text-4xl`, derived from `--font-size-base: 16px`) reproduces stock Tailwind exactly
— it is not inflated. The size comes from per-component literals: `components/dystil/sidebar.tsx` nav
items at `text-[20px]`, `components/dystil/page-primitives.tsx :: PageHeading` at `text-[31px]`.

**3. A shipped accessibility bug follows from the same cause.** `lib/utils/font-size.ts ::
applyFontSize` writes `--font-size-base`, which only reaches text using token utilities. Choosing
"X-Large" in Settings therefore moves roughly a quarter of the interface.

**Also live today, and worth fixing on the way through:**

- **Mismatched typeface.** `tailwind.config.ts` sets both `sans` and `mono` to JetBrains Mono, which
  is not bundled anywhere, so `font-sans` resolves to SF Mono. `components/dystil/ai-models-settings.tsx`
  applies `font-sans` to Select triggers while `components/ui/select.tsx` applies `font-mono` to the
  same elements — both land on monospace, so those dropdowns render in a different typeface than the
  rest of the app. The runtime body face is Inter, loaded in `app/layout.tsx`. `font-serif`: zero uses.
- **A silently broken feature.** `src-tauri/src/commands.rs` and `src-tauri/src/window/show.rs` emit a
  `navigate` event when the user clicks *Manage* on a native notification or picks a tray section.
  No frontend listener exists, so that action does nothing.

**Intended outcome:** the Home route rebuilt to the handoff, on a token foundation the remaining
screens can adopt incrementally, with all data behind a mock interface so UI work is not blocked by
backend gaps.

## Source of truth for the design

Committed in this repo at **`agent_docs/design_handoff_home_screen/`**:

- `README.md` — the specification. Colours, type, spacing, radii, shadows and copy are **final**.
  This is the authority for every measurement in this plan.
- `Dystil App v2.dc.html` — visual reference prototype covering all states. **Not production code**;
  it is a `dc-runtime` React export. Open it in a browser to see the states; do not copy from it.
  The "States" switcher above the window frame is a review control and is not part of the product.
- `support.js` — the `dc-runtime` bundle the prototype needs in order to render. Reference only;
  nothing in it ships.
- `dystil.svg` — logo. The two-tone is deliberate: the dark `#14201B` Y-funnel is the apparatus, the
  `#2F9E7B` droplet is what comes out of it. Never make the Y green — at 14px the dark Y is what
  keeps the mark legible.

The handoff documents the **Home route only**. The prototype contains further screens that have no
written specification yet; see Phase 5.

## Decisions taken with the product owner

- The handoff is the target. An earlier screenshot showed the *current* app and is not a reference.
- **Screens only, mock data only.** No backend wiring in this effort.
- Automation-driven job states (Running / Done / Failed) are in scope **as screens**.
- Handoff copy is final: "Your shortcuts" replaces "Ready to use"; findings are "settled".
- Core screens first.
- **The sidebar is deleted.** `components/dystil/sidebar.tsx` goes; navigation moves to the top bar.
- **No dark mode** on the new screens — they are built light-only, per the handoff.
- **Settings and onboarding are out of scope** for this effort and keep their current design.

> **The theme system is retired, not just unused.** Dark mode goes; there is a single light token set.
> Because Settings, onboarding, auth and permission-recovery consume tokens rather than hex, changing
> the token *values* restyles them automatically — they will shift to the new palette without
> breaking, and get proper design attention in a later effort.

> **The interface is the deliverable.** Because other engineers wire it up later, the UI/data boundary
> matters more than usual. One module defines the types, one provides fixtures, and no screen calls
> `commands.*` or `invoke` directly. If a screen needs data the interface does not expose, the
> interface changes — not the screen.

---

## What the design changes

The handoff is an information-architecture change, not a restyle.

| | Today | Target |
|---|---|---|
| Navigation | 268px left sidebar (`components/dystil/sidebar.tsx`) | 38px title bar + 54px top bar + 34px bottom status strip; **no sidebar** |
| Home | Scrollable list of findings | One finding fills the screen; decide and the next appears |
| Primary action | "Ask for a fix" is a sidebar destination | Persistent primary button, reachable from every screen |
| Palette | `#F7F8F6` / `#2F9E7B` tokens, plus ad-hoc hex | Warm cream ground `#F4F1EA`, paper `#FFFEFB`, green-deep `#0E4F3C`, marigold tint `#F8EDD8` |
| Type | Inter, with SF Mono leaking in | Newsreader (display) + Instrument Sans (UI) |

The handoff is explicit: *"Never a scrollable list of findings."*

Home is one route with three states, chosen by queue length: **A — the pile** (default), **B —
nothing waiting**, **C — just cleared**. If only one is built, build A; B and C share most markup.

---

## Phase 0 — Foundation

**0.1 Delete before rebuilding.**

- `app/home/page.tsx` — the `peers` / `agentMessages` / `sessions` pipeline, `parseCitations`,
  `toChatTurns`, and the `agent-mailbox-updated` listener. `components/chat-shell.tsx` renders none of
  it. Strip the matching fields from `components/dystil/types.ts :: DystilShellProps`. Keep
  `userName`, `userEmail`, `version`, `onLogout`, `loggingOut` — `SettingsWorkspace` uses those.
  *Rule for the new shell: it accepts nothing it does not render.*
- 15 orphaned `components/ui/*` — **except `dropdown-menu.tsx`, which should be rescued**; the design
  needs a real menu layer. Also 4 orphaned `components/auth/*` and `lib/hooks/use-status-dialog.tsx`.
- PatternFly (`pf-v5-*`, `pf-v6-*`) and ProseMirror blocks in `app/globals.css`. There are zero
  `@tiptap` imports anywhere, so the five `@tiptap/*` runtime dependencies are dead too — a real
  bundle win. **Keep the `.dystil-thinking-dots` keyframes**; they are live.
- The sidebar's "Invite your team" active test, which can never be true (the tab is a query param).

**Do not delete:**

- The `request-server-restart` listener in `lib/hooks/use-permission-monitor.tsx` — it **does** have an
  emitter in `src-tauri/src/recording.rs`. Only `capture-health-changed`, `permission-lost` and
  `permission_needed` are genuinely emitter-less.
- The `navigate` emit — it is a broken feature to fix, not dead code (see Phase 2).
- `app/page.tsx`'s no-op. Its comment documents a real incident: every Tauri window briefly renders
  `/`, so anything placed there flash-executes in every window.

**0.2 Tokens.** Replace the palette in `app/globals.css` with the handoff's 24 tokens. Encode the
**colour rules** as comments, since they are semantic: green = actionable / local / safe; marigold =
needs your judgement; at most one green element per region. Delete the contradictory header comment
("Black & White Geometric Minimalism / No color. Sharp corners. Monospace typography") — it describes
a design that no longer exists.

**0.3 Type.** In `app/layout.tsx`, replace Inter with **Newsreader** (300–500) and **Instrument Sans**
(400–700) via `next/font/google`, which self-hosts at build time — required, since the app must work
offline and the prototype's Google CDN link would fail. Fix `tailwind.config.ts`'s phantom JetBrains
Mono and strip the `font-mono` leftovers in `components/ui/{card,dialog,select}.tsx`. Expose the
handoff's scale (`36 / 33 / 24 / 21 / 19 / 16.5 / 15.5 / 14.5 / 14 / 13.5 / 13 / 12.5 / 11.5 / 11 /
10.5`) as named tokens so screens stop hardcoding.

**0.4 Two shared primitives.**

- **Segmented track** — `15×4px`, radius 2, `gap 3px`; states settled `#1B1A17`, active `#2F9E7B`,
  stopped `#C98A2B`, waiting `#DCD6C8`. The handoff reuses it for the pile, run steps and question
  count. Build once.
- **Droplet icon**, extracted from `dystil.svg`, used at 8–11px throughout.

**0.5 Retire the theme system.** The handoff is light-only, so collapse to one token set and remove
the machinery rather than leaving it dormant:

- the `.dark` block in `app/globals.css`, and `darkMode: ["class"]` in `tailwind.config.ts`
- `components/theme-provider.tsx` and its cross-window sync (`storage` / `focus` /
  `visibilitychange` listeners plus Tauri's `onThemeChanged`)
- the pre-paint theme script in `app/layout.tsx` and `defaultTheme` in `app/providers.tsx`
- `ColorTheme` in `lib/constants/colors.ts`, the theme field in `lib/hooks/use-settings.tsx`, the
  Settings control that sets it, and the `setNativeTheme` mirroring to Rust
- the `prefers-color-scheme` fallback blocks behind the macOS vibrancy classes in `app/globals.css`

Settings, onboarding, auth and permission-recovery consume tokens, so they inherit the new palette
automatically and keep working. They are not redesigned in this effort.

---

## Phase 1 — Data contract and mock layer

The handoff's item shape needs fields the backend does not return. Define the contract, mock it, and
record the gaps for the engineers who will wire it.

Required shape, from the handoff:

```
{ origin: 'dystil' | 'user', when, title,
  evidence: [{ n, label }],    // dystil-originated only — exactly 4 stat tiles
  evidenceNote,
  recap: [{ label, text }],    // user-originated only
  offer, fixName, steps: [{ n, t }], runnable }
```

Gap analysis against today's `WorthFixingCard` in `lib/utils/tauri.ts`, backed by
`src-tauri/src/worth_fixing_commands.rs`:

| Design needs | Today | Status |
|---|---|---|
| `evidence` — four numeric tiles | `occurrenceCount`, `cadence`, `WorthFixingEvidenceLine[]` (prose) | **Gap.** Handoff insists on numbers, never prose |
| `origin: dystil \| user` | — | **Gap.** Drives badge-vs-none and recap-vs-evidence |
| `recap` | — | **Gap** |
| `steps`, `runnable` | — | **Gap.** Likely derivable from an automation definition |
| "Decide later" (rotate, count unchanged) | no matching `DispositionKind` | **Gap.** May be client-side only |
| Correction options 1–3 | `not_a_problem` / `close_but` / `leave_it` | Maps cleanly |
| Correction option 4, "Stop watching this kind of work" | — | **Gap.** Needs capture-source disabling |

Add `lib/mock/` holding the typed interface and fixtures — the prototype's three items (two
Dystil-originated with evidence, one user-originated with recap), plus a fixture job that advances on
a timer so the Running / Done / Failed states are demonstrable. Gate the mock source behind a flag
that is **off in release builds**, so fixtures can never reach users.

Shape the interface to match what the automation subsystem can supply (`automationRunNow`,
`automationListRuns`, `automationRunEvents`, and the already-emitted `automation-run-event` /
`automation-run-updated`) so the later mapping is direct — but do not call those commands here.

---

## Phase 2 — Shell and real routing

Replace `components/chat-shell.tsx` with the handoff's vertical flex column and **delete
`components/dystil/sidebar.tsx`**: title bar 38px · top bar 54px (logo · queue pill, only when the
queue is non-empty and not in state A · "Invite your team" · "Your shortcuts" + count · **"Ask for a
fix"** primary · settings) · scrolling content · bottom strip 34px (watching dot with `halo` pulse ·
"What stays on this computer →" · job chips with an indeterminate `crawl` bar when running).

Make routing real at the same time. `app/home/{ask,ready,privacy,settings}/page.tsx` are 3-line
re-exports and the real switch is a `pathname` ternary in `chat-shell.tsx`, which re-mounts the whole
data layer on every navigation. Move the shell into `app/home/layout.tsx` with real child routes.
Static export already emits `out/home/{ask,ready,privacy,settings}.html`, so `output: 'export'` is not
a blocker.

> **⚠️ Rust-side booby trap.** `src-tauri/src/window/show.rs` runs
> `window.eval("if (window.location.pathname !== '/home') window.location.replace(<url>)")`. Once
> sub-routes exist, a user on `/home/settings` who triggers a tray action is **bounced off their
> route**. Fix both sides: relax the guard to a `startsWith('/home')` test, and add the frontend
> `navigate` listener — which also un-breaks the notification *Manage* action. No automated check
> covers this; verify by hand.

---

## Phase 3 — Home, three states

**A — the pile.** `max-width: 650px` centred. Context strip (remaining count + depletion track +
"See all n") · source badge, user-originated only · Newsreader 33px headline · evidence panel
(Dystil-originated: four stat tiles) or recap panel (user-originated) · offer heading 24px · fix panel
with numbered steps and the "Runs on this Mac" reassurance footer · **sticky decision row** — "Yes,
run it" / "Just give me the prompt" / "This isn't right" / "Decide later".

**A8 — correction panel.** Replaces the offer and decision rows, and **collapses the evidence panel**.
Four options, each stating its consequence *before* selection. The handoff calls this *"the most
important interaction in the app"* — it must never degrade into a grey dismiss link.

**B — nothing waiting.** Hero writing box (multi-line textarea, `min-height: 138px`), starter chips,
bottom shelf with three shortcut mini-cards.

**C — just cleared.** Filled track, "That's the lot.", no bottom shelf, `rise` animation.

**Queue mechanics** are a depleting pile, not a carousel. `settle()` removes the head; `defer()` moves
it to the end and **must not** advance the count or track. Guard the failure modes the handoff calls
out: at zero length always resolve to state C, never fall back to the first item of the original
stack, and hide the top-bar queue pill.

---

## Phase 4 — Job states (screens only)

Running / Done / Failed plus the bottom-strip job chips and Stop, driven entirely by the mock
interface.

---

## Phase 5 — Remaining screens

The prototype visually contains Ask, Scan ("See all"), Your shortcuts, Privacy, Invite, Running, Done
and Failed, but only Home has a written handoff. Extract specs where the prototype is unambiguous and
request written handoffs where it is not.

Not designed anywhere, per the handoff: **"Open the record"**, **"Open the full result"**, and
first-run onboarding. Wire these as stubs.

---

## Documentation

- `agent_docs/GLOSSARY.md` credits "Ready to use" to `dystil-automation`. It is backed by
  `dystil-insights` via `src-tauri/src/ready_to_use_commands.rs`. Fix this standalone and first, so it
  does not get lost in a redesign commit. Add the new vocabulary ("settle", "the pile", "Your
  shortcuts") once the code uses it.
- `agent_docs/DESIGN_SYSTEM.md` is stamped `status: verified` / `authority: ground-truth` but
  describes an IA that exists nowhere in code — destinations "Ask Your Work / Inquiries / Work Index"
  and components "Inquiry Control / Inquiry Trail / Evidence Brief". Under the precedence rules in
  `AGENTS.md` that is a bug. **Two steps:**
  - *Now:* flip `status: verified` → `status: unreviewed`. One line, using the mechanism `AGENTS.md`
    already defines. It stops the doc misleading the next agent immediately.
  - *After the code matches:* rewrite to describe what exists, flip back to `verified`, update
    `verified_against` / `verified_on`.
  - **Do not rewrite it from the handoff now.** Stamping a `verified` spec for a design that does not
    yet exist recreates exactly the failure `AGENTS.md` warns about, and is how this document reached
    its current state.

---

## Verification

| Check | Catches | Blind to |
|---|---|---|
| `bun run typecheck` | prop-bag and type-drift removal | anything visual |
| `bun run test` | IA / copy / a11y-name regressions | anything visual |
| `bun run bindings:check` | stale Rust→TS bindings | every pure-UI phase |
| `cargo check`, `cargo fmt --check` | Rust compile and format | every pure-UI phase |
| `bunx tauri dev` | everything visual, multi-window, drag region, tray | nothing |

Only Phase 2 crosses the Rust boundary. Run the app with `bunx tauri dev` from `apps/dystil` — **not**
`cargo run`, which skips `beforeDevCommand` so Next never starts. `DYSTIL_CLOUD_BASE_URL` must be
unset in a community build or `build.rs` panics by design.

> **Do not run `bun run build` while `bunx tauri dev` is running.** They share `.next/`, so the export
> build overwrites what the dev server is serving. The running app then 404s on every stylesheet and
> chunk and renders as a blank grey window — which looks exactly like a catastrophic CSS failure and
> is not one. Recover by stopping the dev server, `rm -rf .next`, and restarting it.

Visual acceptance: with mocks on, walk A → correction → settle → C and confirm the queue-pill and
empty-queue guards. The window opens at 1180×800 centred
(`src-tauri/src/window/show.rs :: PRIMARY_DEFAULT_SIZE`); confirm the sticky decision row stays visible
without scrolling at the 800×600 minimum.

## Risks

1. **The existing tests are the IA contract, and this redesign breaks them deliberately.**
   `worth-fixing.test.tsx` asserts on copy, e.g. a heading named "Dystil has started reading how you
   work." The handoff's copy is final and different, so these will fail — that is signal. Update each
   assertion **in the same commit** as the copy change. Per the comment in `vitest.config.ts`, nothing
   may be added to the KNOWN-BROKEN exclude list.
2. **Routing versus the `show.rs` eval guard** — silently breaks tray, notifications and deep links.
3. **The macOS drag region.** `MacDragRegion` and its `pt-[38px]` offset in `chat-shell.tsx` is what
   makes the window draggable, and it is invisible to every automated check. It must survive the shell
   rewrite — verify by dragging the window.
4. **Mock fixtures leaking into a release build.**
5. **Retiring the theme touches shared surfaces.** Removing the pre-paint script in `app/layout.tsx`
   risks reintroducing the FOUC it was added to prevent, and the macOS vibrancy classes carry
   `prefers-color-scheme` fallbacks that must come out cleanly. Verify first paint under
   `bunx tauri dev`, not just in a browser.
6. **Settings and onboarding will look transitional** — new palette, old layout — until their own
   redesign lands.

## Open questions

- **Settings and onboarding** have no handoff yet. They inherit the new token values automatically but
  keep their current layout and composition; a later effort will redesign them properly. Expect them
  to look transitional in the meantime.
- **Backend gaps**, recorded here for the engineers who will wire the interface, not to be solved in
  this effort: the four evidence stat tiles, `origin`, `recap`, `steps` / `runnable`, "Decide later",
  and correction option 4.
