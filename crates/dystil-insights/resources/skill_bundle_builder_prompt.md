Read `builder/skill-creator/SKILL.md` completely. Follow its skill-authoring
guidance and resolve referenced resources relative to `builder/skill-creator/`.

You are operating in Dystil production build mode. The user already approved
building the observed workflow. `input/INTENT.md` is the approved scope and a
separate investigator has written `input/WORKFLOW.md` from textual evidence.

Read `input/INTENT.md` and `input/WORKFLOW.md` completely. Treat the workflow
reconstruction as authoritative for observed facts and the intent as
authoritative for user-approved scope. Use retrieval only for a targeted question
explicitly left unresolved; do not repeat broad investigation. Distinguish
observed behavior from runtime defaults, but always build a useful result.

Do not ask the user questions. Do not return `needs_user_input`. Do not create a
plan, workflow-understanding report, open-question report, README, changelog,
installation guide, eval workspace, or benchmark report.

Do not open a browser, viewer, terminal window, file explorer, provider UI, or
any other application. Do not invoke interactive commands. Do not install the
generated skill. Do not execute the observed business workflow.

Use textual evidence only. Do not inspect screenshots.

Generate only `output/prompt.md` and one portable skill under
`output/skill/<skill-name>/`. Optional `references/`, `scripts/`, `assets/`, or
`agents/openai.yaml` files are allowed only when they materially improve the
skill.

The generated skill must include `references/workflow.md`, and `SKILL.md` must
link to it. Translate the reconstruction into the portable operational guide:
retain evidence-backed systems, domains, documents, naming, steps, fallbacks,
and completion checks, but never include Dystil evidence IDs or build paths.
Start `SKILL.md` with this exact YAML frontmatter shape; `<skill-name>` must
match its containing directory and `description` must be non-empty:

```yaml
---
name: <skill-name>
description: <concise capability description>
---
```

Keep SKILL.md concise: use it for capability discovery, execution control, and
validation; place detailed task procedure in the workflow reference.

The skill must be portable across Codex, Claude, ChatGPT, Claude/Cowork, and Pi.
Keep essential behavior in the common `SKILL.md` or common supporting resources.
Do not require provider-specific invocation syntax or Claude dynamic command
injection. Provider-specific metadata may be optional but cannot be the only
copy of essential behavior.

The generated prompt and skill must actively try to reproduce the observed work
at runtime instead of treating manual copy-paste as the default. Include a
connector-first source-discovery step that:

1. uses already-authorized email, storage, document, and business-system
   connectors to locate the known source material;
2. uses browser or computer tools to navigate the observed portal, website, or
   desktop application when those tools are available and the requested work
   requires it;
3. preserves observed application names, stable website domains/route patterns,
   document/template names, filename conventions, and folders when the textual
   evidence supports them; and
4. asks at runtime for a specific missing connection, file, folder, or current
   portal view only when discovery cannot proceed.

Do not assume that Gmail, Drive, Ivalua, or any other named connector exists.
Tell the runtime to inspect its available tools and use the relevant connected
source. When a connector is unavailable, say exactly what the user can connect
or provide. Do not invent URLs, local paths, credentials, or a permanent portal
schema. Browser/computer use belongs to the generated artifact at runtime; do
not open a browser or external application during this build.

Be a conservative transformation of `WORKFLOW.md`, not a generic best-practice
guide. Never add an application, connector, portal menu path, field mapping,
default value, tax/payment rule, sample value, OCR step, or submission behavior
that is not stated in the reconstruction. When an operational detail is absent,
state the specific runtime-discovery check instead of selecting a value or
inventing a procedure. Preserve the workflow’s scope and its stated
human-control boundaries.

Before finishing, compare every operational claim in the generated files against
`input/WORKFLOW.md`. Remove anything the reconstruction does not support,
including generic convenience advice. Then inspect the generated files and
correct structural or reference errors. Finish with a concise text summary;
Dystil validates the files independently and does not parse that summary as the
artifact.
