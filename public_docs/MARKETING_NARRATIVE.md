---
status: narrative
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Narrative, not specification.** Positioning and direction. Some capabilities described here are aspirational. Do not implement from this file — see `agent_docs/`.

# Dystil Marketing Narrative

> Base marketing material. **Superseded on positioning by
> [`POSITIONING.md`](POSITIONING.md)** — read that first; where the two disagree,
> `POSITIONING.md` wins.
>
> An earlier version of this note said proactive automation discovery "must be
> presented as the vision until shipped." That is now out of date: **Worth fixing**
> (discovery, with evidence) and **Ready to use** (kept, reusable artifacts) are
> shipping surfaces. Broader automation *execution* is still being wired up — see
> the Status section of the README for the current line between the two.

## The strategic idea

**Dystil watches how work actually gets done, finds opportunities to make it easier, and helps automate them.**

Memory is the foundation, not the final product.

Dystil first needs to understand a person's real workflow across applications: what they are trying to accomplish, which steps repeat, where they lose time, what breaks, and which decisions recur. That working memory gives Dystil the context to move beyond answering questions about the past. It can identify useful interventions in the present and, with the user's approval, take work off their plate.

The long-term promise is not “software that takes notes for you.” It is:

> **An AI that learns how you work and continuously finds ways to make that work easier.**

## The shortest version

**Dystil understands how you work, spots what can be improved or automated, and helps do it for you.**

## One-line options

Primary:

> Dystil watches your work—and finds ways to make it easier.

Alternatives:

- The AI that improves how you work.
- Your work has patterns. Dystil turns them into leverage.
- Dystil learns your workflows, finds the friction, and helps remove it.
- An AI that notices what keeps slowing you down—and does something about it.
- From repeated work to approved automation.

Supporting privacy line:

> **Understands locally. Acts with permission.**

## The problem

Most automation starts with a blank page.

The user has to notice a repetitive workflow, understand which steps can be automated, explain it to a tool, connect the right systems, and maintain the result. The people who would benefit most from automation are often too busy doing the work to step back and design it.

Meanwhile, everyday work is full of small, repeated costs:

- moving the same information between tools;
- rebuilding the same report every week;
- checking several systems for one status;
- repeating a debugging or operational sequence;
- following up after the same kind of event;
- searching for context before every resumed task;
- performing work that is almost—but not quite—predictable.

Traditional automation tools only see the workflow after someone has formally described it. General AI assistants only see the context pasted into the current conversation. Neither continuously understands how work unfolds across the user's actual desktop.

The result is an automation gap: people repeat work that software could help with because no system has enough context to recognize the opportunity.

## The product idea

Dystil closes that gap by learning from work as it happens.

With the permissions and settings a user chooses, Dystil observes useful context across applications and builds a private working model of tasks, tools, actions, outcomes, and repeated patterns. It can use that understanding in three increasingly valuable ways:

1. **Remember:** recover what happened and where the work left off.
2. **Recognize:** identify repeated steps, bottlenecks, handoffs, and automation opportunities.
3. **Act:** propose a useful shortcut or workflow and, once approved, carry it out.

This progression matters. An agent that acts without understanding the work is brittle. A system that only remembers the work leaves most of the value untouched. Dystil's memory provides the grounded context required for useful, personalized automation.

## The core promise

**Dystil turns observed work into practical leverage.**

It does not ask users to map every process before it can help. It learns from the way work is already being done, notices where time and attention are being spent repeatedly, and surfaces specific opportunities:

- “You perform these same five steps every Friday. Want me to prepare them next time?”
- “This issue always requires checking the same three dashboards. I can gather them into one brief.”
- “You copy this data from email into the CRM after every customer call. Should I draft that update for approval?”
- “You solved a similar deployment failure last month. I can walk through the same checks.”
- “This task has been blocked in the same place three times. Want to create an escalation workflow?”

The product should feel less like configuring an automation platform and more like working with an attentive operator who understands the job.

## How it works, in plain language

### 1. Observe the real workflow

Dystil runs alongside the tools a person already uses. Depending on permissions and settings, it can understand application context, accessibility content, UI activity, and optional screenshots.

### 2. Build working memory

Dystil groups noisy activity into meaningful tasks and work sessions. It records what happened, which tools and artifacts were involved, what state the work reached, and which evidence supports that understanding.

### 3. Find patterns and friction

Across repeated work, Dystil can look for recurring sequences, manual transfers, repeated searches, common failure-recovery paths, unnecessary switching, and steps that appear suitable for assistance.

### 4. Recommend a concrete improvement

Instead of a generic “you could automate this,” Dystil should explain the observed pattern, the proposed workflow, the expected benefit, the systems it would touch, and what approval it needs.

### 5. Help execute—with control

The user decides whether Dystil should simply remind, prepare a draft, ask before every action, or run an approved workflow automatically. Sensitive and irreversible actions should always retain an appropriate confirmation boundary.

### 6. Improve from outcomes

The result becomes part of Dystil's working context: whether the suggestion was accepted, edited, ignored, successful, or rolled back. Over time, recommendations should become more relevant to the individual rather than more generic.

## The product loop

```text
Observe → Understand → Detect opportunity → Propose → Approve → Act → Learn
```

Each stage earns the next. Observation without understanding is surveillance. Understanding without usefulness is a log. Action without approval is unsafe. Dystil's product value comes from joining all three responsibly.

## Why memory still matters

Memory is Dystil's durable advantage because useful automation depends on context that most tools do not have.

A workflow is rarely just a fixed sequence of clicks. It includes intent, exceptions, timing, prior decisions, the meaning of an error, and the state in which work was left. Dystil's work cards and evidence-linked history make these details retrievable and comparable across time.

Memory delivers immediate utility—users can ask what happened or resume a task—while also accumulating the context required to discover higher-value automation. Recall is the first benefit and the training ground for proactive assistance.

## Why now

Knowledge work is more fragmented, and AI is becoming more capable of understanding and operating software.

People move continuously between browsers, editors, documents, terminals, dashboards, communication tools, and internal systems. No single application sees enough of the workflow to identify how it could improve.

At the same time, local models can structure sensitive work context, multimodal systems can interpret interfaces, and agents can carry out bounded actions. The missing layer is continuous, user-owned context: an understanding of how a particular person actually works.

Dystil is building that layer.

## Positioning

Dystil is a **personal workflow intelligence and automation agent**.

It is not primarily a note-taking tool, activity tracker, screen recorder, or generic chatbot. Those categories may preserve information or accept instructions, but they do not close the loop from observing work to discovering and executing improvements.

| Category | Typical model | What Dystil adds |
| --- | --- | --- |
| Notes and wikis | The user documents important knowledge | Learns from work that was never formally documented |
| Activity tracking | Measures time or application usage | Understands task context and looks for actionable patterns |
| Screen recording | Preserves a visual timeline | Distills activity into tasks, patterns, and opportunities |
| Automation builders | The user defines a workflow | Discovers candidate workflows from observed behavior |
| General AI assistants | The user supplies a prompt and context | Builds persistent context across real work and becomes proactive |
| RPA | Executes predefined UI sequences | Connects execution to intent, evidence, exceptions, and approval |

The wedge is private working memory. The category ambition is proactive, personalized automation.

## Who it is for

### Founders and operators

Dystil can notice recurring reporting, follow-up, data-transfer, and status-checking work across the many functions a small team handles without dedicated operations staff.

### Builders and engineers

It can recognize repeated debugging paths, release checks, issue triage, environment setup, and cross-tool coordination—then prepare or run the routine parts while preserving technical judgment for the engineer.

### Customer-facing teams

It can identify the repeated work around calls, tickets, CRM updates, follow-ups, and internal handoffs, and prepare the next step using the context that produced it.

### Researchers and analysts

It can learn recurring research, comparison, extraction, and reporting patterns, then gather inputs or prepare repeatable analysis steps.

### Any knowledge worker with repeated digital work

The user does not need to know which automation to build in advance. Dystil's job is to make the opportunity visible and understandable.

## High-value use cases

### Discover invisible repetition

> “You have copied the same fields from these emails into the CRM nine times this month.”

Dystil identifies the pattern and offers a bounded workflow instead of waiting for the user to recognize and specify it.

### Prepare recurring work

> “You assemble this project update every Monday. I can prepare a draft from the same sources.”

The first useful action can be preparation rather than full autonomy, allowing the user to review the result.

### Turn recovery paths into playbooks

> “The last three certificate failures led to the same diagnostic checks. Save this as a guided workflow?”

Dystil turns remembered problem-solving into reusable operational leverage.

### Reduce context rebuilding

> “You are returning to the release task. Here is where it stopped and the next step you usually take.”

Recall itself becomes a proactive intervention at the moment it is useful.

### Close handoff gaps

> “After calls like this, you usually update the account and send a recap. I drafted both.”

Dystil connects events to the repeated work that follows them.

### Improve before automating

> “This workflow switches between four tools for one approval. Here is the repeated path and a simpler proposed flow.”

Not every opportunity should become a bot. Sometimes the highest-value output is making the process visible so the user or team can redesign it.

## The trust model

Proactive does not mean uncontrolled.

Dystil should make its understanding and authority legible at every step:

| Level | Dystil may… | User control |
| --- | --- | --- |
| Observe | Build private context from allowed sources | Choose apps, data types, exclusions, and retention |
| Suggest | Surface a pattern or opportunity | Dismiss, correct, or explore the evidence |
| Prepare | Draft an output or stage an action | Review and edit before anything is sent or changed |
| Confirm | Execute a bounded action | Approve each run |
| Automate | Repeat an explicitly approved workflow | Set scope, conditions, limits, and revoke access |

Important product behaviors:

- explain what pattern caused a suggestion;
- show the evidence behind the inferred workflow;
- state which applications, data, and actions an automation would use;
- distinguish reversible preparation from external or irreversible action;
- ask for approval at the appropriate boundary;
- make recurring automations visible, editable, pausable, and revocable;
- record what Dystil did and whether it succeeded.

## Privacy story

The strongest formulation for the broader vision is:

> **Understands locally. Acts with permission.**

Dystil's current local-first architecture is important because workflow context is unusually sensitive. The foundation can run on the user's machine:

- raw activity is stored locally;
- screenshots are optional;
- sensitive text can be redacted locally;
- generation, embeddings, and search can use local models;
- no Dystil account or hosted LLM is required for the core local memory workflow;
- team sharing and synchronization are optional extensions.

As action capabilities develop, permission must extend beyond data access to authority: what Dystil may read, prepare, change, send, purchase, publish, or run. The product should request the narrowest useful permission and never treat observation permission as execution permission.

Privacy copy should stay concrete. Do not imply that local software is invulnerable, that redaction catches every sensitive value, or that future automations can operate safely without scoped controls.

## The product story in three acts

### Act I: See the work

Dystil observes the available context across tools and builds a private, evidence-linked memory of tasks and outcomes.

### Act II: Find the leverage

It compares work over time to spot repeated steps, friction, failure patterns, and opportunities for assistance.

### Act III: Make work easier

Dystil proposes a specific improvement and helps carry it out at the level of autonomy the user approves.

## Founder-story draft

We started Dystil because we kept seeing people do work that software could help with—but the opportunity was invisible to the software.

Automation tools are powerful once someone has identified and described a workflow. That is the catch. Most of the best opportunities are buried in ordinary work: copying data between tools, rebuilding the same report, checking the same systems, following the same debugging path, or repeating the same handoff after an event.

The person doing the work is usually too close to it—and too busy—to stop, map the process, and build an automation. An AI assistant cannot help unless the person recognizes the pattern and explains it all over again.

We think the computer should be able to notice.

Dystil runs alongside the tools people already use and builds a private understanding of how work actually happens. That starts with memory: what the task was, which applications and artifacts were involved, what actions were taken, and where the work ended. But memory is only the foundation.

The goal is for Dystil to find repeated work and friction, explain the opportunity it sees, and help turn it into a useful workflow. Sometimes that means bringing back context at exactly the right moment. Sometimes it means preparing the next step. Eventually, it means carrying out an approved automation on the user's behalf.

We are building Dystil local-first because work context is personal and sensitive. And we believe action must be permissioned: software that can see your work has not automatically earned the right to act inside it.

Our goal is simple: Dystil should learn how you work, then keep finding ways to make that work easier.

## Launch-post draft

**Dystil is building the AI that finds your automation opportunities**

Most automation tools begin after you have already done the hard part: noticing a repeated workflow and describing exactly how it should work.

Dystil begins earlier.

It observes the context you allow across your desktop and builds a private memory of real work—tasks, tools, actions, outcomes, and evidence. That memory is useful immediately: you can ask what happened, recover an error, or resume where you left off.

But recall is the foundation, not the destination.

Our goal is for Dystil to recognize repeated sequences and friction, then surface concrete opportunities to help:

- prepare the report you rebuild every week;
- gather the checks you perform after the same kind of failure;
- draft the follow-up that always comes after a customer call;
- turn a proven recovery path into a reusable workflow;
- remove manual transfers between tools.

The user stays in control. Dystil should show what it observed, explain what it proposes, and ask for the right level of approval before acting. The core memory architecture is local-first, and observation permission should never silently become execution permission.

We are starting by giving Dystil the ability to understand and retrieve work. We are building toward something larger: an AI that continuously discovers how it can make each person's work easier.

## Pitch-deck outline

1. **Title:** Dystil — the AI that improves how you work.
2. **Problem:** valuable automation opportunities are hidden in repeated daily work.
3. **Why existing automation misses them:** users must first identify and specify the workflow.
4. **Insight:** understanding real work comes before safely automating it.
5. **Foundation:** local-first observation, structured memory, evidence, and retrieval.
6. **Product loop:** observe → understand → detect → propose → approve → act → learn.
7. **Opportunity engine:** repeated sequences, friction, handoffs, and recovery patterns.
8. **Progressive autonomy:** suggest, prepare, confirm, or automate.
9. **Use cases:** recurring reporting, debugging, follow-ups, data transfer, and task resumption.
10. **Differentiation:** discovers workflows from behavior instead of waiting for a specification.
11. **Trust:** local context, visible evidence, scoped permission, and revocable authority.
12. **Roadmap:** memory foundation today; proactive discovery and execution as the product direction.

## Message hierarchy

When space is limited, communicate in this order:

1. **Outcome:** Dystil finds ways to make a person's work easier.
2. **Differentiator:** it discovers opportunities by understanding how work actually happens.
3. **Action:** it can suggest, prepare, or execute approved workflows.
4. **Foundation:** private working memory makes those interventions contextual and personalized.
5. **Trust:** local-first understanding and explicit authority boundaries.

Do not lead with note-taking, search, or “remember everything.” Recall is supporting proof that Dystil understands the work; proactive improvement is the larger promise.

## Voice and language

Aim for language that feels attentive, practical, ambitious, and respectful of the sensitivity of desktop context.

Prefer:

- understands how you work;
- finds the friction or opportunity;
- proposes a concrete improvement;
- prepares the next step;
- acts with permission;
- learns from outcomes;
- private working context;
- visible, scoped, and revocable.

Use carefully:

- watches or observes — pair with purpose, local processing, and user control;
- proactive — explain what Dystil may do without being asked;
- automation — distinguish the future vision from currently shipped behavior;
- agent — describe its authority and approval boundaries;
- learns — clarify that it infers patterns from allowed work context rather than making unsupported claims.

Avoid:

- positioning Dystil primarily as automatic notes;
- “records everything” or “understands everything”;
- “runs your work for you”;
- implying that observation grants permission to act;
- promising fully autonomous behavior without visible controls;
- “100% secure,” “perfect privacy,” or perfect redaction;
- claiming proactive automation discovery is currently shipped before it is implemented.

## Claims and roadmap guardrails

### Safe current foundation claims

- Dystil is a local-first desktop system that captures and retrieves work context.
- It can use accessibility data, application metadata, UI events, and optional screenshots.
- It turns bounded activity into structured, searchable work cards.
- It supports lexical and semantic retrieval.
- Capture, redaction, storage, generation, embeddings, and search can run locally.
- A hosted account or external LLM is not required for the core local workflow.
- Its read-only MCP interface provides a foundation for approved agent access to derived memory.

### Product-vision claims—use future-oriented language

- Dystil will identify repeated workflows and automation opportunities.
- It is being designed to recommend personalized process improvements.
- The goal is to prepare or execute approved actions across tools.
- Dystil is building toward progressive, user-controlled automation.
- Outcome feedback will help recommendations become more relevant over time.

### Claims requiring evidence before publication

- a specific number of hours saved;
- a percentage of work that can be automated;
- autonomous reliability or accuracy rates;
- universal support across applications and desktop platforms;
- universal detection of sensitive information;
- definitive security or compliance guarantees.

## Reusable boilerplate

### 25 words

Dystil learns how work happens across your desktop, finds repetitive tasks and friction, and helps turn the best opportunities into approved automations.

### 50 words

Dystil is a personal workflow intelligence and automation agent. It builds a private understanding of how you work across applications, uses that memory to identify repeated steps and friction, and helps propose, prepare, or run improvements with your permission. The foundation is local-first; the user controls when understanding becomes action.

### 100 words

Dystil watches how work actually happens and finds ways to make it easier. It observes the context a user allows across desktop applications and turns that activity into a private, evidence-linked memory of tasks, tools, actions, and outcomes. That memory provides immediate recall, but its larger purpose is proactive assistance: identifying repeated sequences, manual transfers, recovery paths, and other automation opportunities. Dystil is being designed to explain each opportunity, prepare a useful workflow, and act only with the appropriate approval. Its current local-first memory and retrieval architecture is the foundation for personalized automation that understands the work before trying to change it.

## Suggested next content pieces

This narrative can seed:

- a manifesto: “Automation should discover you”;
- a product essay: “Memory is the foundation, not the product”;
- a demo built around discovering one repeated workflow over several days;
- a progressive-autonomy explainer;
- a technical post on evidence-linked workflow understanding;
- a privacy post: “Observation is not authority”;
- persona pages for operators, engineers, and customer-facing teams;
- a design-partner pitch focused on finding real automation opportunities.

The strongest early story will show Dystil noticing something the user did not explicitly teach it: a repeated task appears across several work sessions, Dystil explains the pattern with evidence, proposes a bounded improvement, and the user chooses whether it should prepare or perform the next instance.
