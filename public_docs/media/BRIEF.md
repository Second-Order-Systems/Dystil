---
status: narrative
---

# Media brief for the README

Four screenshot slots are marked in `README.md` as HTML comments
(`<!-- MEDIA SLOT n -->`). Drop files here and replace the comment with an image
tag. Until then the README renders cleanly with no broken images.

## Conventions

- Put files in `public_docs/media/`, **not** `apps/dystil/public/` — that directory
  is the app bundle, and its filenames contain spaces, which is what broke the
  banners on GitHub.
- Kebab-case names, no spaces: `worth-fixing-finding.png`.
- Light theme unless the dark one reads better; GitHub shows both to different users.
- Keep GIFs under ~5 MB. GitHub serves them uncompressed.
- Crop to the surface being shown. Full-window screenshots waste the reader's eye.

Markdown to use:

```markdown
<p align="center">
  <img src="public_docs/media/worth-fixing-finding.png" alt="A Worth fixing finding with its evidence" width="100%">
</p>
<p align="center"><em>Caption goes here.</em></p>
```

## The four slots

### Slot 1 — hero (after "What Dystil is")

The single image that has to communicate "this finds work you repeat." Best
candidate is the Worth fixing list with two or three real findings visible, so a
reader sees concrete repetitive work rather than chrome.

Avoid an empty state. Avoid a settings screen.

### Slot 2 — Worth fixing

One finding, expanded, with its evidence visible. The evidence is the credibility —
it shows Dystil is not guessing. Caption should say so.

### Slot 3 — Ask for fix

The clarification exchange. Show that the user's judgement is the input: a question
from Dystil and the user's answer shaping the result.

### Slot 4 — Ready to use

Saved artifacts. Ideally shows more than one, so "a library of work you no longer
redo" reads immediately.

## Content safety

These are screenshots of real work. Before committing any of them:

- No real customer names, emails, domains, or account identifiers
- No API keys, tokens, or file paths containing a username
- No third-party app content you do not have permission to show

Use seeded demo data where possible. If you AI-edit them for clarity, keep the UI
truthful — a screenshot showing behaviour that does not exist is the same failure
as a README describing an architecture that does not exist.
