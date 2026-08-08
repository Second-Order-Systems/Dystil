---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
name: Dystil
description: A private desktop app that finds the work you repeat and makes it reusable.
colors:
  ink: "#151616"
  ink-raised: "#1D1F1E"
  paper: "#F5F3EE"
  paper-raised: "#FFFEFA"
  line-dark: "#383B39"
  line-light: "#D9D7D0"
  graphite: "#8A8C87"
  signal: "#56D59D"
  signal-deep: "#157252"
  quiet: "#8FA49B"
  window-close: "#E95E54"
  window-minimize: "#E8B452"
  window-zoom: "#57C46F"
typography:
  display:
    fontFamily: "Iowan Old Style, Palatino Linotype, Book Antiqua, Georgia, serif"
    fontWeight: 400
    lineHeight: 1.08
  body:
    fontFamily: "ui-sans-serif, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontWeight: 400
    lineHeight: 1.45
  label:
    fontFamily: "ui-sans-serif, -apple-system, BlinkMacSystemFont, Segoe UI, sans-serif"
    fontWeight: 650
    letterSpacing: "0.08em"
rounded:
  control: "8px"
  surface: "12px"
spacing:
  compact: "8px"
  regular: "16px"
  spacious: "28px"
components:
  inquiry-control:
    backgroundColor: "{colors.ink-raised}"
    textColor: "{colors.paper}"
    rounded: "{rounded.control}"
    height: "56px"
  selected-record:
    backgroundColor: "{colors.paper-raised}"
    textColor: "{colors.ink}"
    rounded: "{rounded.surface}"
    padding: "{spacing.regular}"
---

# Design System: Dystil

## Overview

**Creative North Star: "The Working Index"**

Dystil is a private instrument for returning to work, not a chat product. Its home is an inquiry surface; the working index is a deliberately navigated destination for browsing a day in detail. It should feel precise enough to scan quickly, calm enough to use all day, and concrete about what was captured versus what the model inferred.

Key characteristics:

- Inquiry is the home action; chronology and provenance live one intentional navigation step away in the work index.
- An inquiry preserves its question trail and selected evidence. A new inquiry is deliberate, never an automatic consequence of asking another question.
- Conclusions are brief and explicitly sourced; uncertain synthesis never impersonates a report.
- Hairline structure, compact labels, and deliberate density replace card grids, message bubbles, and generic dashboard chrome.
- Light mode is a daytime work index; dark mode is neutral charcoal, with green limited to meaningful signal and action.

## Colors

Graphite and paper are the environments. Emerald is reserved for live capture, selected records, and intentional action; it must not tint the entire dark interface green.

**The Evidence Rule.** Signal color marks a real state or action; it never fills the UI merely to make it look like an AI product.

## Typography

**Display Font:** Iowan Old Style / Palatino-style serif fallback
**Body Font:** System UI sans-serif

The serif is for a small number of legible, human headings and conclusions. The system sans handles time, metadata, controls, and dense rows.

## Layout

Use a compact utility rail and three intentional destinations. Ask Your Work is the default home: one clear inquiry field, a small amount of visible recent context, and an honest explanation of response confidence. Inquiries is a compact list of resumable investigations. Work Index is a split workspace where chronology owns the left and center, and the selected record/evidence brief owns the right. On narrower windows, the brief becomes a detail drawer rather than collapsing work into cards.

## Elevation & Depth

Depth comes from tonal planes, dividers, and selected-state framing. Use shadows only for temporary floating controls or drawers, never under every container.

## Shapes

Controls have restrained 8px corners. Record rows are largely square and defined by rhythm, rules, and whitespace. Pills are reserved for small state labels, not navigation or page structure.

## Components

### Inquiry Control

A full-width, command-like question field. It names the action as asking one's work. It starts an inquiry only when no inquiry is active; otherwise it adds a follow-up to the active inquiry.

### Inquiry Trail

An inquiry is a compact, resumable investigation: prior questions, short grounded answers or unavailable states, and the evidence selected along the way. It is not styled as a chat transcript. New Inquiry is explicit and preserves the prior inquiry for later return.

### Work Index

Chronological rows pair time, source application, captured activity, and bounded status. Selecting a row opens its detail; the index remains visible.

### Evidence Brief

The answer is short and marked by status: captured, grounded, tentative, or unavailable. Provenance is a first-class list, never hidden behind citations.

## Do's and Don'ts

### Do:

- **Do** distinguish observed/captured activity from model synthesis.
- **Do** keep the current workday legible at a glance.
- **Do** use the Dystil mark unchanged.
- **Do** make light and dark modes equally deliberate.
- **Do** preserve a user's inquiry context across follow-up questions.

### Don't:

- **Don't** use chat bubbles as the primary information architecture.
- **Don't** start a new inquiry automatically for every question.
- **Don't** invent detailed findings, reports, or work cards the local model cannot reliably produce.
- **Don't** use a grid of soft rounded cards as the page scaffold.
- **Don't** use glow, glass, or gradients to signal AI.
