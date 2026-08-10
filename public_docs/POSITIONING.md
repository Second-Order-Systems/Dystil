---
status: narrative
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Narrative, not specification.** This is positioning — what Dystil is for and how
> to talk about it. It may run ahead of the code and uses explanatory concepts that
> are not types. Do not implement from it. For what exists, see `agent_docs/`.

# Positioning

The canonical source for the README, the website, and marketing copy. If those
disagree with this file, this file wins. If this file disagrees with the product,
fix this file.

---

## The one-liner

**Make your workday genuinely AI-native.**

Supporting line: *Dystil watches how you actually work, then builds the agents,
automations, and skills that do that work — for the AI you already use.*

The bold claim, and the one to lead with everywhere:

> **It watches how you work, then builds the agent, automation, or skill that does
> that work — for the AI you already use.**

Do not lead with "finds work you repeat." Repetition is the easiest signal to spot
and the least interesting half of the product. The lead is that Dystil **builds the
thing**, and that the thing runs in the tools the reader already pays for.

## What Dystil is

A private desktop app that watches how you work, notices what you do over and over,
and turns it into instructions that can do that work again.

It runs on your machine. Nothing has to leave it.

## The frame that matters

Lead with **AI-native**, not with repetition. "Finds the work you keep redoing" is
true and it is a good second sentence, but on its own it lands as a reminder app or
a macro recorder. The larger claim is the right one:

> Becoming AI-native is not something you install. It is knowing which parts of your
> work should stop being done by hand, and in what order.

That is the gap Dystil fills, and it is why the product has to observe before it
recommends. The site's opener carries it best:

> **Everyone has a solution. Nobody's looked at your work.**
>
> Every vendor arrives with the answer already chosen, and none of them have watched
> how work actually moves through your day. So you buy the answer and hope it matches
> the question. Impressions, not evidence.

Reusable phrases: *impressions, not evidence*; *fitted to you, not tuned on someone
else's work*; *the finding comes to you*. Company sign-off: *Specific Intelligence.
Built with you.*

## What Dystil is not

Getting this wrong is the most common failure in our own copy, so state it plainly:

- **Not a note-taking app.** You never write anything for it.
- **Not a day planner or time tracker.** It is not trying to organise your calendar
  or tell you where your hours went.
- **Not a chatbot over your history.** Answering "what did I do Tuesday" is a
  by-product, not the point.

The point is **automation**. Dystil is looking for work worth not doing again.

## Who it is for

**The open-source app is for one person.** An individual on their own machine,
with their own data, running their own models if they want to. It is complete on
its own — not a demo of something better.

**Teams are the paid product.** Shared automations, managed sync, administration.
Individuals should never feel they are using a crippled version; teams should see
an obvious reason to upgrade. Both things have to be true at once.

**It is the same app.** What a team installs is the app in this repository; the team
edition adds capability on top rather than replacing it. Say this plainly — it is
what makes "the open-source edition is not a trial" credible instead of defensive.

**Keep pricing and deployment tiers out of the README.** They belong on the site,
where they can change without a commit. The README links to `2os.ai` and stops there.

---

## The loop

This is the spine of the product and the spine of the README. Three surfaces, in
order:

### 1. Worth fixing

Dystil watches your work and surfaces things that repeat, with the evidence that
led it there. You did not ask it to look — that is the difference between this and
a tool you have to remember to use.

The shapes it looks for:

- *The same work, over and over* — "I keep copying the same customer details
  between two apps."
- *Work that arrives on a schedule* — "Every Friday I rebuild the same client
  report from scattered files."
- *The same avoidable mistake* — "Our final review catches the same errors every
  time."

### 2. Ask for fix

You clarify. Dystil asks what it misunderstood, which reading is closer, what you
would actually want. Your judgement is the input it cannot generate.

### 3. Ready to use

What you keep. Reusable instructions for work you want done the same way again —
"the report, the reply, the summary, done to the standard you would want if you had
the time."

---

## Why it is better

Four claims, in the order they land:

1. **It finds work you did not think to ask about.** Most automation tools require
   you to already know what to automate, and to sit down and build it. Dystil
   arrives with the finding and the evidence.

2. **It runs on your machine.** Capture, redaction, storage, and search are local.
   Point it at a local model and inference is local too — no key, no per-token
   cost, no data leaving the device.

3. **Your judgement stays yours.** Dystil does the groundwork that repeats, not the
   deciding. *"The judgement has to be yours. Rebuilding the same groundwork before
   every one of them does not."*

4. **It plugs into the agents you already use.** Your work history is available to
   Claude Code, Codex, or any MCP client — bounded and sanitized, not a raw
   database dump.

## On privacy

Privacy is a structural property, not a promise. The strongest true statement:

> Everything Dystil has read stays in one folder on this machine, and there is no
> copy of it to ask for.

Supporting facts worth using, all verifiable:

- Sensitive text is redacted **twice** — deterministically before anything is
  written, then by a local model afterwards.
- The open-source build has **no cloud endpoint compiled into it at all**. Not
  disabled: absent, with a test enforcing it.
- Captured content is never transmitted in any build.

### Two qualifications we must always make

**Hosted model providers.** If a user chooses Anthropic or OpenAI, their prompts and
the bounded context go to that provider. Never claim data "never leaves the device"
without this caveat. The honest framing: Dystil does not require it, and Ollama
avoids it entirely.

**Anonymous usage counts.** Official community builds send operational counters by
default — counts, timings, and bounded enums, never content. It is one switch to
disable, and nothing is sent before onboarding completes.

Never write "nothing is ever sent" or "no telemetry." That was true of an earlier
build and is not true now. The accurate line is: *nothing we send could identify you
or reveal what you were working on.* If a claim needs the reader not to check, do
not make it.

---

## What Dystil builds — and the rule for saying so

The output list is the lead claim, so it is also the easiest place to overclaim.
Split it every time it appears, in copy and in graphics:

| In the app today | Being built next |
|---|---|
| Automations that run through Claude Code or Codex | Agents generated from a finding |
| Prompts | Skills for Claude and ChatGPT |
| Runbooks | Workflows for n8n and similar tools |
| Pointers to a tool the user already owns | A browser extension |

**The right column is a roadmap and must always be labelled as one.** It is fine —
encouraged — to lead with the ambition. It is not fine to let a reader install the
app expecting a browser extension. Every surface that names these has to carry the
distinction: the README uses a status column, the hero banner uses "in the app
today" and "being built next" chip groups.

Checked at the time of writing: n8n appears nowhere in the codebase; the only
"skill" references are `--no-skills` flags that disable them; every "browser
extension" reference is capture code that *avoids* recording extension popups.
Automation execution is real (`dystil-automation :: execute`).

## What it looks for

Five shapes, and the app shows this exact list on day one. Use all five — the last
two are the ones no competitor asks about, because the user would never think to
put them on a list.

1. **The same work, over and over** — you do it the same way every time.
2. **Work that arrives on a schedule** — the Monday report, the month-end close.
3. **Work where you make the call** — the judgement is yours; the groundwork is not.
4. **Work that could come out better** — done to the standard you would want if you
   had the time.
5. **What you would do if you had the time** — skipped because the day is full, not
   because it does not matter.

## Vocabulary

Use the words the product uses. **Worth fixing**, **Ask for fix**, **Ready to use**
are real surfaces the user sees, and they are good names — spend the reader's
attention on those rather than inventing internal nouns.

Explanatory concepts (how observed work gets structured) are fine in prose. They
must never appear as a schema, a field list, or an API shape — see
`agent_docs/GLOSSARY.md` for which terms are real and which are illustrative.

## Things we may not say

Learned the hard way; all of these were published and none were true:

- That Dystil bundles or runs its own inference engine. It connects to one you
  control.
- That it does semantic or vector search. Retrieval is keyword-based today.
- Any specific model name, size, port, or download figure — unless someone has
  just checked it.

Direction we may describe **as direction, clearly labelled**: semantic search over
the work index, and team/multiplayer mode.
