---
status: verified
authority: design-brief
verified_against: 3e2f41e
verified_on: 2026-08-14
---

# Design brief: Ask for a fix

> Every type, field, state and error named here is real and cited. This is a
> description of what the system **already does** — not a proposal. The design
> question is how to present it, not what it should be.
>
> Backend: `src-tauri/src/ask_for_fix_commands.rs` (crates `dystil-insights` +
> `dystil-ai`). Types: `lib/utils/tauri.ts`. Current UI:
> `components/dystil/pages/ask-for-fix.tsx`, hook
> `components/dystil/ask-for-fix/use-ask-for-fix.ts`.

## The use case

Dystil watches how you work and, on its own, surfaces repeated work worth fixing.
**Ask for a fix is the other direction** — the user has noticed something Dystil
hasn't, and says so in their own words.

> *"Every Friday I rebuild the same client report from scattered files."*

The system's job is to turn that vague, first-person complaint into something
reusable: a prepared prompt, a runbook, or a pointer to a capability the user
already has. It does that by **interviewing the user against evidence it already
holds** — what it actually observed them doing — and then showing them what it
understood *before* it commits to an answer.

The product promise underneath: **it will not guess.** Everything it infers is
labelled as inference, everything it is unsure about is stated, and the user can
correct it at any point. Nothing leaves the machine.

## Why a linear chat thread is the wrong shape

The current UI renders this as a message thread. That metaphor actively hides
five things the machine does, and each one is a design problem worth solving:

1. **It is a bounded interview, not an open conversation.** There is a hard
   question budget (`questionCount` / `maxQuestions`). A chat implies you can
   talk forever; this cannot, and the user should feel the budget depleting.
2. **Most turns are not typing.** Three of the four question kinds are
   structured choices, not free text. Rendering a radio group as a "message"
   wastes the affordance.
3. **There is a confirmation gate.** Before answering, the system shows its
   synthesis and the user must approve or refine it. In a thread this reads as
   just another message, when it is actually the most consequential decision in
   the flow.
4. **There are four different outcomes, and only one is "here's your answer."**
   The other three are refusals of a kind — and refusing well is the product.
   A chat bubble flattens them into the same thing.
5. **The output is an artifact, not a reply.** It gets kept, reused, copied and
   revised. It belongs in a container that looks durable, not ephemeral.

Also worth knowing: **the design system explicitly prohibits chat bubbles as
primary information architecture** — see the "no chat bubbles as IA" rule in the
existing design language. The current screen predates that.

---

## The machine

One session. `AskSessionView` **is** the state — every command returns a whole
new one, so there is no client-side state to reconcile.

```
AskPhase = "understand" | "follow_up" | "consolidate" | "present"
```

### Phase 1 — Understand
The user describes the problem in free text. Roomy, multi-line; the invitation
is to ramble. This is the only phase that starts with typing.

### Phase 2 — Follow up
The system asks up to `maxQuestions` questions. Each is one `AskQuestion`:

```
AskQuestion = { kind, text, helper, options: AskOption[], minSelections, maxSelections }
AskOption   = { id, label, description }
```

Four kinds, each needing its own control:

| `kind` | Control | Notes |
|---|---|---|
| `free_text` | Text input | No options |
| `single_select` | Choose exactly one | Options carry a `description` as well as a `label` |
| `multi_select` | Choose N | **`minSelections` and `maxSelections` are enforced** — the design must show the constraint and the running count |
| `compare` | Two readings side by side | "Which reading is closer?" — a genuine A/B, not a list |

The user must **always** be able to reject the offered options and answer in
their own words instead. That escape hatch is not optional: the options are the
system's guesses.

Progress is real and should be visible: `questionCount` of `maxQuestions`.

### Phase 3 — Consolidate — *the gate*
The system stops and shows what it understood. The user approves it or sends it
back. **This is the highest-value screen in the flow** and the one the current
UI most under-serves.

```
AskUnderstanding = {
  synthesis:         string    // the headline reading
  grounding:         string[]  // what this is based on — things actually observed
  inferences:        string[]  // what was inferred rather than seen
  preservedBoundary: string    // what it will deliberately NOT touch
  uncertainty:       string[]  // what it is still unsure about
  solutionTarget:    string    // what it intends to build
}
```

The design must keep **`grounding` and `inferences` visually distinct** — the
whole trust model rests on the user being able to tell observation from
guesswork at a glance. `uncertainty` is a feature, not an error state; showing
doubt is how the system earns belief.

Two actions: **approve** (`askForFixConfirm`) or **refine** — refining reopens
free text so the user can say what was misread.

### Phase 4 — Present — *four outcomes*
```
AskPresentation = { route, headline, explanation, limitations: string[], artifact }
```

`route` is one of four, and **they are not interchangeable**:

| `route` | Meaning | Design implication |
|---|---|---|
| `answer_now` | Here is the thing | The only route with a full artifact |
| `something_now_more_later` | Partial answer; more needs watching | Must not look like failure |
| `cannot_see` | Not enough evidence to answer honestly | A principled refusal — should read as integrity, not error |
| `needs_more_than_one_person` | The fix isn't solo work | Points outward, at people |

`limitations` is **"what this does not assume"** and must always be visible, not
hidden behind a disclosure. It is the honesty contract.

When present, the artifact:

```
AskArtifact = { kind, title, description, body, steps[], tool, capability, instructions[] }
AskArtifactKind = "prompt" | "runbook" | "existing_capability"
```

- `prompt` — a block of text to copy, monospace, often long
- `runbook` — ordered `steps[]`
- `existing_capability` — you already have the tool; `tool` / `capability` /
  `instructions[]` say which and how

Actions on the result: **Copy**, **Keep** (`askForFixKeepArtifact` → it becomes a
shortcut and `artifactKeptId` is set, so the state is sticky and must be
reflected), and **ask for a change** (free text describing the revision).

---

## Controls required

**Always available**
- Start a new session (destructive to the current one — the design should say so)
- Cancel while thinking (`askForFixCancel`) — long AI calls need an out
- The composer, whenever free text is legal

**Per phase**
- *Understand:* multi-line entry, generous. Starter suggestions for a cold start.
- *Follow up:* the four question controls above; "answer in my own words";
  visible question budget
- *Consolidate:* approve / refine, with the six understanding fields legible and
  observation distinguishable from inference
- *Present:* copy, keep, revise — plus a clear reading of which of the four
  routes you got

**Every state that needs designing** — not just the happy path:

| State | Source | Note |
|---|---|---|
| Loading an existing session | `askForFixLatest` | A session survives app restarts and resumes |
| Thinking | `busy` | Can take a long time; needs a Stop |
| Locked | `session.locked` | Input must be refused, visibly |
| Kept | `artifactKeptId` | Sticky — "Kept" is a state, not a toast |
| Six error kinds | `lastErrorCode` | Below |

Error codes are distinct and deserve distinct treatment (`use-ask-for-fix.ts ::
readableError`): `provider_not_ready` (no AI configured — needs a route to
Settings), `authentication`, `timeout`, `invalid_output` (the model returned
something unusable), `user_cancelled`, `interrupted`. Retry exists
(`askForFixRetry`) but only some of these are retryable.

## Constraints

- **Local-first.** Nothing uploaded; the current composer says "Stays on this
  computer" and that reassurance should survive.
- **Provider transparency.** `provider`, `model` and `cachedInputTokens` are
  available. Whether to surface them is a design call, but the data exists.
- **Sessions resume.** Someone can close the app mid-interview and come back.
- **Copy voice:** plain language, first person, no jargon, never overclaims.
  Compare the Home handoff's register.
- **Design language:** warm cream ground `#F4F1EA`, paper `#FFFEFB`, deep green
  `#0E4F3C` for actions, marigold `#F8EDD8` for "needs your judgement",
  Newsreader display + Instrument Sans UI. Green means *actionable and local*;
  marigold means *needs your judgement*; at most one green element per region.
  Full palette in `agent_docs/design_handoff_home_screen/README.md`.
- **Reusable device:** the segmented track (15×4px segments) is the app's single
  idiom for "progress through a finite thing" — already used for the pile and a
  natural fit for the question budget.

## Out of scope

The backend is fixed for this exercise. The four phases, four question kinds,
six understanding fields and four outcome routes are what the system produces —
a design that needs different data would need backend work first, so flag it
rather than assume it.

## The question for the designer

Given a bounded interview that must show its working, gate on a confirmation
step, and end in one of four differently-shaped outcomes — **what is the right
shape for this, if not a chat thread?**

Worth exploring: a wizard with a visible budget; a single evolving "understanding"
document the user watches get filled in; a split view with the interview on one
side and the accumulating understanding on the other; or something else entirely.
The strongest constraint is that the user should be able to see, at any moment,
*what the system currently believes and how confident it is* — because that is
what they are being asked to correct.
