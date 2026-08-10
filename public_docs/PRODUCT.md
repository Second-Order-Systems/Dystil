---
status: narrative
verified_against: e84d34c
verified_on: 2026-08-10
---

> **Narrative, not specification.** Positioning and direction. Some capabilities described here are aspirational. Do not implement from this file — see `agent_docs/`.
>
> Positioning is owned by [`POSITIONING.md`](POSITIONING.md). Where the two disagree, that file wins.

# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop

## Users

Dystil serves employees, knowledge workers, and domain experts whose work repeats across applications, documents, and tools. They know AI should be taking some of that work off their hands, but not which parts, or in what order — and no vendor can tell them, because none of them have watched how the work actually moves.

## Product Purpose

Dystil makes a person's workday AI-native. It observes how they actually work, distils what is worth taking off their plate, and builds the thing that does it — an automation, a prompt, a runbook, or a pointer to a capability they already own. The output runs in the tools they already pay for.

## Positioning

Dystil is not a note-taking app, a day planner, a screen recorder, or a chatbot over a person's history. Answering "what did I do on Tuesday" is a by-product of observing work, not the purpose. The purpose is automation: finding work worth not doing again, and producing something that does it.

## Operating Context

The product runs as a desktop application alongside a person's normal work. It captures bounded work activity locally, surfaces findings with the evidence behind them, and asks the user to resolve what it could not infer before producing an artifact. Nothing is kept without an explicit decision by the user. A future multiplayer capability is intended to let teammates share automations and context they approve.

## Capabilities and Constraints

- Dystil captures work context automatically; users should not need to take manual notes for Dystil to be useful.
- **Producing something that does the work is the primary user outcome.** Retrieval and search are supporting capabilities, not the product's purpose.
- Shipping today: automations executed through coding-agent runners, prompts, runbooks, and pointers to tools the user already owns.
- In the pipeline, and to be described only as such: agents generated from a finding, skills for Claude and ChatGPT, workflows for n8n and similar tools, and a browser extension.
- Current local models should not be represented as reliably inferring detailed reports or findings from captured activity. Model quality is expected to improve over time.
- Sign-in is optional, though encouraged.
- Team collaboration is a planned future capability; its precise sharing model remains undecided.

## Brand Commitments

Use the name "Dystil." Treat the product as private by design: what it reads stays on the machine, and the open-source build has no cloud endpoint compiled into it.

## Evidence on Hand

- [README.md](../README.md) documents the current local-first desktop implementation, the Worth fixing → Ask for fix → Ready to use loop, and what ships today versus what is being built.
- [`agent_docs/`](../agent_docs/README.md) is the verified engineering reference; every claim there cites the code.
- No customer testimonials, benchmarks, or external proof claims are established for product messaging.

## Product Principles

1. Capture useful work context without demanding manual documentation.
2. Distil — surface only what is genuinely worth taking off someone's plate, with the evidence for it.
3. Produce something that runs, in the tools the person already uses, rather than advice they still have to act on.
4. Keep privacy central to the product's behavior and communication.
5. Treat collaboration as deliberate and contextual, rather than exposing a person's complete work history.

## Accessibility & Inclusion

No product-specific accessibility standard is confirmed yet. Future interfaces should support employees and domain experts with varied technical fluency and work contexts.
