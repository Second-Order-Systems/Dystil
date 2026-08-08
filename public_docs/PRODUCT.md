---
status: narrative
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Narrative, not specification.** Positioning and direction. Some capabilities described here are aspirational. Do not implement from this file — see `agent_docs/`.

# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Dystil serves employees, knowledge workers, and domain experts whose work accumulates across applications, documents, tools, and conversations. They need to recover the context of past work without manually recording every decision, action, or finding.

## Product Purpose

Dystil observes a person's work and turns it into private, retrievable memory. It helps people answer questions about their own prior work by finding the relevant context, rather than requiring them to maintain notes or reconstruct a task from memory.

## Positioning

Dystil is not a chat app or a manual note-taking system. Its chat interface is one way to interact with an automatically captured work memory: Dystil watches work as it happens, records usable context, and makes that context retrievable later.

## Operating Context

The product runs as a desktop application alongside a person's normal work. It captures bounded work activity and makes the resulting memory searchable and queryable. A user's related questions, available evidence, and answers persist together as an inquiry until they explicitly start a new one. A future multiplayer-AI capability is intended to let teammates collaborate around relevant work context.

## Capabilities and Constraints

- Dystil captures work context automatically; users should not need to take manual notes for Dystil to be useful.
- Retrieval is the primary user outcome; chat is an interaction surface, not the product's purpose.
- Current local models should not be represented as reliably inferring detailed reports, findings, or work cards from captured activity. Model quality is expected to improve over time.
- Sign-in is optional, though encouraged.
- Team collaboration through multiplayer AI is a planned future capability; its precise sharing and collaboration model remains undecided.

## Brand Commitments

Use the name “Dystil.” Treat the product as private by design.

## Evidence on Hand

- [README.md](README.md) documents the current local-first desktop implementation, private local capture, structured work cards, and retrieval architecture.
- No customer testimonials, benchmarks, pricing, or external proof claims are established for product messaging.

## Product Principles

1. Capture useful work context without demanding manual documentation.
2. Make past work easy to retrieve at the moment it becomes relevant.
3. Keep privacy central to the product's behavior and communication.
4. Treat collaboration as deliberate and contextual, rather than exposing a person's complete work history.

## Accessibility & Inclusion

No product-specific accessibility standard is confirmed yet. Future interfaces should support employees and domain experts with varied technical fluency and work contexts.
