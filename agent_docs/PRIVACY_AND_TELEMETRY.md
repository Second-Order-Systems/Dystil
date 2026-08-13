---
status: verified
authority: ground-truth
verified_against: e84d34c
verified_on: 2026-08-08
---

> **Verified** against `e84d34c`. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# Privacy and telemetry

Privacy in Dystil is enforced structurally — by what is compiled in and by which
crate is allowed to see what — rather than by policy. This document states what
actually leaves the machine, per edition.

## The redaction boundary

`dystil-redact` is the text privacy boundary. Its module doc: "This crate
intentionally handles text only. Images are never inspected or..." — the boundary is
deliberate, not an omission.

It uses a local ONNX model plus deterministic detectors. `dystil-redact/src/onnx.rs`
resolves the model to `~/.dystil/models/v45_phase5_pruned/`, documented in its module
comment and built with `.join(".dystil").join("models").join("v45_phase5_pruned")`.

This is the **only** model Dystil downloads. It is not a language model.

Redaction state is persisted in the `dystil_text_redaction_state` table.

## What leaves the device

### Community builds: anonymous counters, on by default

**Captured content never leaves.** No accessibility text, window titles, app names,
URLs, file paths, prompts, or model replies are transmitted in any build that exists
today.

> **Planned change — do not write copy that assumes this stays true for teams.**
> The team edition is intended to process raw capture **server-side**, which is the
> designed difference between the editions. When that ships, the sentence above
> becomes true of the open-source build only, and this section must be split per
> edition before the feature lands — not after. The open-source guarantee is
> unaffected: processing stays on the machine, which is what that edition is.

**Anonymous operational counters do leave, by default.** Official community releases
are built with `DYSTIL_TELEMETRY_ENDPOINT` set from the `vars.DYSTIL_TELEMETRY_ENDPOINT`
repository variable in `.github/workflows/release-app.yml`. A source build without
that variable has no endpoint and cannot report.

The desktop exporter does not require a device credential to attempt an export
(`telemetry_exporter.rs :: start`). When a device token is available, it adds the
`Authorization: Device <token>` header; otherwise it sends the same sanitized
OTLP payload anonymously (`telemetry_exporter.rs :: build_export_request`).
Authentication and any identity association are relay concerns; the client does
not add a user identifier to telemetry.

`cloud_base_url()` remains absent from community builds — it is only passed by
`release-enterprise.yml`. Test: `app_config.rs :: community_build_has_no_cloud_url`.

Consent resolution lives in one place, `telemetry_consent.rs :: resolve`, in
precedence order:

1. `DYSTIL_TELEMETRY=0` — wins over everything, including enterprise
   (`store.rs :: telemetry_disabled_by_env`)
2. `enterprise-client` — organization-managed, forced on, no prompt or UI
3. Onboarding incomplete — withheld, so the in-app disclosure is always seen first
4. Otherwise the user's setting, `SettingsStore::telemetry_enabled`, default `true`

`Telemetry::new()` starts at `ConsentDecision::Unknown`, which records **nothing** —
`is_enabled()` gates the `record_*` hot paths as well as `drain_interval`. If the
resolution path is removed or never runs, no data is gathered at all. Test:
`aggregate.rs :: collection_is_off_until_consent_is_resolved`.

Revoking clears accumulated counters before the next send
(`aggregate.rs :: set_consent` transitions through `Denied` and calls `clear`).

Inference is local when the Ollama provider is selected — see `AI_PROVIDERS.md`.
Choosing Anthropic or OpenAI means prompts and their bounded context leave the
machine; that is the user's explicit choice of provider.

### Enterprise builds: telemetry, organization-managed, no content

`dystil-telemetry`'s module doc: "Privacy-safe telemetry primitives for Dystil. This
crate deliberately has no exporter or OpenTelemetry SDK dependency." Product crates
record typed, bounded values; a separate exporter drains snapshots.

`agent_docs/TELEMETRY_FOUNDATION_PLAN.md` states the constraint verbatim:

> "Never export screenshots, accessibility text, window titles, application names,
> URLs, paths, prompts, completions, work cards, evidence, raw errors, user
> email/name, auth credentials, or local database contents."

and:

> "No prompt, completion, model endpoint, query text, result text, work-card data,
> evidence ID, or file content is exported."

Transport must be HTTPS outside a debug localhost build (`app_config.rs`).

Enterprise builds show **no telemetry UI at all** — no toggle, no disclosure card,
no prompt. Consent is organizational, agreed by an administrator. The UI hides on
`BuildCapabilities.enterpriseManaged`, and `set_telemetry_enabled` rejects attempts
to disable, mirroring `set_sync_consent`.

## Payload contract

Every exported attribute is a bounded enum or a number. There is no free-text field.
`schema.rs :: RESOURCE_ATTRIBUTES` is the full resource-attribute list, and
`schema.rs :: registry_has_no_known_sensitive_attribute_keys` fails the build if a
sensitive key is introduced.

Resource attributes: service name and version, deployment environment, build
channel, `dystil.edition` (`community` or `enterprise`), OS type, host arch, a
random install id, and the schema version.

Both editions report to the same endpoint and are separated by `dystil.edition`.

### Spans carry no attributes

Traces are the part of an OpenTelemetry payload that would normally be able to
leak free text — attributes commonly hold error strings, file paths, or URLs.
Here they cannot, and this is deliberate:

```rust
pub struct TracePoint { kind: TraceKind }
pub enum TraceKind { CaptureSessionStart, CaptureSessionStop }
```

`aggregate.rs :: TracePoint` has a single field, and `aggregate.rs :: TraceKind`
has two variants mapping to the fixed strings `capture.session.start` and
`capture.session.stop`. When encoded in
`telemetry_exporter.rs :: encode_traces`, each span sets `attributes: Vec::new()`
and `events: Vec::new()`, with `trace_id` and `span_id` generated as random
UUIDs rather than derived from anything local.

A span therefore conveys "a capture session started or stopped, at this time",
and nothing else.

**Keep it that way.** Adding a span attribute is the single easiest route to
exporting user content from this codebase. If one is ever needed, it must be a
bounded enum — never a message, path, name, or identifier taken from captured
work. The buffer is capped at `aggregate.rs :: MAX_PENDING_TRACES` (50) per
interval.

## User-facing controls

The privacy surface is `components/dystil/pages/privacy.tsx`. It offers deletion by
time range, by application, and by site, plus a full reset. Its own copy states the
position plainly:

> "Everything Dystil has read is in one folder on this machine, and there is no
> copy of it to ask for. The only thing that leaves is anonymous usage counts,
> which you can turn off."

That sentence previously ended "Nothing has been sent anywhere." It was corrected
when community telemetry was enabled. Enterprise builds render the shorter form
without the telemetry clause.

Deleting capture also removes what was derived from it — the page states "Anything
Dystil worked out from this history will be removed too." Settings, connections,
automations, and downloaded models are retained.

## Rules for contributors

If a change causes more data to leave the device:

1. It must be opt-in.
2. It must be added to this document.
3. It must not weaken the community-build guarantee that endpoints are absent
   rather than disabled.
