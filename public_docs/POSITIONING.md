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

**Dystil finds the work you keep redoing, and makes it something you can reuse.**

## What Dystil is

A private desktop app that watches how you work, notices what you do over and over,
and turns it into instructions that can do that work again.

It runs on your machine. Nothing has to leave it.

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
