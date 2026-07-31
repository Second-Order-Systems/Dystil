# Accessibility capture follow-ups

## Completed

- macOS reads core AX attributes in one batch per node.
- macOS retains non-decorative structural containers with explicit snapshot-local parent IDs.
- macOS performs automation-property and line-bound enrichment only after core traversal and only while the walk budget remains.
- Windows UIA keeps its cached-subtree path node-capped and applies a real wall-clock deadline to the per-element Chromium/Electron fallback.
- Windows remembers windows that require the fallback and reports timeout/node-cap truncation truthfully.

## Not completed

### Diagnostic capture

Add a manually triggered capture that uses the production batched walker, records the normal production deadline, and then continues to a larger diagnostic deadline. Record node start/completion offsets so the output can show both:

- what the production deadline would retain;
- what the extended traversal could additionally reach.

Write a bundle containing the screenshot, production tree, extended tree, traversal timings, attribute failures, and truncation frontier. This is diagnostic-only and must not become a second legacy production walker.

### Live platform validation

- Capture Slack, Chrome, Cursor, and native-app fixtures on macOS.
- Capture Chromium/Electron and native UIA fixtures on Windows.
- Verify visible text recall, message-container hierarchy, sender/body association inputs, duration, and truncation.
- Validate the macOS framework FFI on an actual macOS build runner and the Windows COM behavior on Windows.

### Screenshot and AX coherence

Pair screenshot and accessibility observations with separate timestamps and focused app/window identity before and after acquisition. Mark captures whose context changed during acquisition.

### Cross-platform structural fidelity

The new explicit structural IDs are populated by macOS. Windows and Linux still need to retain parser-relevant non-text containers and populate snapshot-local node/parent IDs instead of relying on the legacy depth fallback.

### Visibility accuracy

Current `on_screen` checks window intersection. It does not yet clip nodes against nested scroll containers, occlusion, or partially hidden application regions.

### Adaptive and semantic-guided capture

Do not enable adaptive throttling until the diagnostic fixtures establish a completeness baseline. Later profiles may choose attributes or enrichment based on stable app/layout evidence, but truncation must not cause progressively smaller incomplete captures.

### Semantic parsing

Re-run the Slack parser experiment on newly captured structural fixtures. Expand parser families only after capture shows reliable conversation, message, actor, timestamp, and body containers.
