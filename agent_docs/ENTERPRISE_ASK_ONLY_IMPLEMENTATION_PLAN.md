---
status: verified
authority: approved-implementation-plan
verified_against: working-tree
verified_on: 2026-08-20
---

# Enterprise ask-only client and cloud watch-request plan

## Objective

Make `enterprise-client` the enterprise product experience. It continues to
capture and upload approved work data to Dystil Cloud, but it does not run the
local Worth Fixing or local AI pipeline. Its user-facing product is a bounded
**Ask for a fix** conversation that creates one durable cloud watch request.

This plan does not add a second feature flag. `enterprise-client` is the gate.

```text
Enterprise desktop capture
  -> existing redaction / segment upload
  -> Dystil Cloud
  -> future cloud-side investigation over capture + watch requests

Enterprise user
  -> Ask for a fix chat
  -> bounded clarification
  -> concise summary confirmed or edited by user
  -> persisted cloud watch request
```

## Verified baseline

- Enterprise is already a compile-time product feature:
  `enterprise-client = ["cloud-sync", "official-build"]`. Citation:
  `Cargo.toml` — `enterprise-client` feature; `agent_docs/EDITIONS.md` —
  `Build capabilities`.
- The desktop exposes `enterprise_managed` and cloud/auth capabilities to the
  frontend. Citation: `apps/dystil/src-tauri/src/build_capabilities.rs ::
  BuildCapabilities` and `current()`.
- Desktop capture state already syncs through an authenticated device credential.
  Citation: `apps/dystil/src-tauri/src/capture_state_reporter.rs :: send_state()`.
- Cloud already accepts authenticated capture segment uploads. Citation:
  `../memory/cloud/services/ingest-api/src/main.rs :: post_segments()` and
  `router()` route `/v1/ingest/segments`.
- The current desktop Worth Fixing loop starts Explorer, Ask-watch collection,
  and Steward work from the app setup path. Citation:
  `apps/dystil/src-tauri/src/main.rs :: run()` and
  `apps/dystil/src-tauri/src/worth_fixing_engine.rs :: start()` / `tick()`.
- The current Ask-for-fix commands use local `WorthFixingState` and local
  runtime processing. Citation: `apps/dystil/src-tauri/src/ask_for_fix_commands.rs
  :: ask_for_fix_submit()`.

## Product decisions

| Area | Approved behaviour |
|---|---|
| Build gate | `enterprise-client`; no extra feature or runtime product switch. |
| Capture | Remains enabled and uploads approved enterprise data to cloud. |
| Local inference | Explorer, Steward, local retrieval, local Ask watches, and skill building are disabled. |
| Models | Cloud selects the conversation model. Enterprise users cannot select/configure local providers. |
| User surface | Ask for a fix only. No Worth Fixing, Your shortcuts, Ready to Use, or local skill export. |
| Conversation limit | Up to seven follow-up questions; it may finish sooner. It never asks an eighth. |
| Watch output | One concise plain-language `summary` field, plus full conversation history. |
| AI credential | Reuse existing per-user cloud `ai_keys` and `ai_usage`; the raw key remains server-side. |
| This phase | Persist a watch request only. No future watcher result, update, or notification is required. |

## Desktop changes

### Keep the capture-and-sync path

Do not disable recording, redaction, segmenting, device registration, capture
state reporting, or cloud sync in enterprise builds. The cloud copy of captured
data is the future evidence source.

### Disable local insight execution

For `enterprise-client`, do not start `worth_fixing_engine::start()`. Also
guard its commands and any Ready-to-Use/skill-generation commands so direct UI
or IPC invocation cannot re-enable local processing.

The enterprise build must not invoke the local AI runtime for Ask for a fix.
Its Tauri implementation calls the cloud API instead of local
`WorthFixingState`, local Ask SQLite records, or `AiRuntime`.

### Reuse the existing Ask UI

Keep the existing in-app chat rendering, messages, follow-up UI, and summary
confirmation presentation. Do not build a parallel chat application.

In the enterprise build, the same frontend interaction calls enterprise Tauri
commands which proxy authenticated requests to cloud. Cloud is authoritative
for session state and replies; the desktop renders them.

### Enterprise UI and routing

- Hide **Your shortcuts**, Worth Fixing count/queue, and Ready-to-Use surfaces.
- Redirect `/home`, Worth Fixing, and Ready/Shortcuts routes to Ask for a fix.
- Hide AI Models and local-provider controls from Settings.
- Skip model/provider setup during onboarding; retain workspace sign-in and
  capture consent/policy steps.
- Do not display local Build skill, install/export, or local artifact states.

## Cloud implementation (`../memory/cloud`)

### Authentication and AI-key accounting

Reuse the existing authenticated device principal for desktop-to-cloud requests.
The desktop stores no Dystil AI/API key and does not directly call the model
gateway. The cloud derives `org_id`, `user_id`, and `device_id` from the device
credential for every request.

Citation: `../memory/cloud/services/ingest-api/src/auth.rs ::
principal_from_device()` and `parse_device_token()`.

Reuse the existing cloud `ai_keys` and `ai_usage` tables; do not create a new
organization-key or credential table. On the first successful enterprise Ask
request, cloud provisions or retrieves the `ai_keys` record for the signed-in
user email. Cloud uses that record for model allowance/revocation and records
each Ask model call in `ai_usage`.

The raw key never leaves cloud. It is an internal authorization/accounting
implementation detail; the desktop calls the Ask API only with its normal
device credential. Organization membership is resolved from the authenticated
user identity rather than copied into `ai_keys`.

Citation: `../memory/cloud/crates/work-insights-db/migrations/
20260802000000_ai_gateway.sql` — `ai_keys` and `ai_usage`; `../memory/cloud/
crates/work-insights-db/src/ai_gateway.rs :: resolve_active_ai_key()` and
`record_ai_usage()`.

### API

Add endpoints to `services/ingest-api`:

```text
POST /v1/ask/conversations
GET  /v1/ask/conversations/:id
POST /v1/ask/conversations/:id/messages
POST /v1/ask/conversations/:id/finalize
```

Every read/write is tenant-scoped server-side by the authenticated principal.
The desktop must never send trusted `org_id` or another user's identity.

### Durable model

```text
ask_conversations
  id, org_id, user_id, device_id
  state                 -- collecting | confirming | finalized
  follow_up_count
  created_at, updated_at

ask_messages
  id, conversation_id
  role                  -- user | assistant
  content
  created_at

watch_requests
  id, org_id, user_id, conversation_id
  status                -- watching
  summary               -- concise description of what the user wants
  created_at
```

`watch_requests.summary` is intentionally the only extracted requirement in
this phase. Any later cloud-side investigator can use that summary alongside
the full message history and cloud-captured data to infer deeper detail.

## Bounded conversation protocol

Cloud owns the conversation state and chooses the model centrally. The desktop
does not select a model or receive model credentials.

```text
initial user request
  -> 0..7 targeted follow-up questions
  -> concise draft summary
  -> user confirms or edits it
  -> one watch request is persisted
```

For each message, the cloud model returns structured control data with exactly
one action:

```text
ask_follow_up | confirm
```

It also returns the assistant message and current draft summary. The service
enforces the limit independently of the model: once `follow_up_count == 7`,
the next result must be `confirm`, never `ask_follow_up`. Finalization is a
separate user-confirmed API action.

The model may finish early when it has a clear enough summary. V1 does not need
to retrieve cloud capture during clarification; later cloud investigation uses
the persisted request, history, and capture data.

## Summary edit and finalization

The confirmation screen shows a direct editable summary:

```text
We’ll watch for:
[ concise description of what the user wants ]
```

- **Create watch** finalizes the displayed text.
- **Edit request** edits that text directly.
- **Save and create watch** persists the edited text as `summary`.
- **Back to conversation** adds one correction message and asks cloud to
  regenerate a summary; it does not restart an unlimited clarification loop.

Finalization transactionally marks the conversation `finalized` and inserts
exactly one `watch_requests` row. The final UI response is an acknowledgement,
not a promise of a result or notification.

## Validation

### Desktop

- Community build remains unchanged.
- Enterprise build continues authenticated capture upload.
- Enterprise startup does not start Explorer, Steward, local watch collection,
  local retrieval, local AI runtime, or skill builder.
- No local model/provider UI appears in onboarding or Settings.
- Worth Fixing, Ready/Shortcuts, and skill UI are inaccessible even by direct
  route/command.
- Restart restores the remote conversation or finalized request.

### Cloud

- Device credentials can access only their organization/user records.
- Cloud provisions/reuses the signed-in user's existing `ai_keys` record and
  records each conversation-model call in `ai_usage`; no raw AI key appears on
  the desktop.
- Up to seven follow-ups are allowed; an eighth is rejected/converted to
  confirmation server-side.
- Finalizing creates exactly one request with the user-approved summary.
- Authentication expiry and service outage return a clean retryable error to
  the desktop.
- Conversation text and device credentials are excluded from anonymous
  telemetry/logging.

## Explicitly out of scope

- Cloud-side retrieval/evidence investigation for a watch.
- Automatically resolving a watch request.
- Watch updates, results, or native notifications.
- Skill generation, installation, or local artifact management in the
  enterprise client.
