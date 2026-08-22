---
status: verified
authority: ground-truth
verified_against: working-tree
verified_on: 2026-08-21
---

> **Verified** against the working tree on 2026-08-21. Claims cite a path plus a symbol name or verbatim
> quote. If a citation no longer resolves, this document is wrong.

# Editions

`AppPolicy` is the source of product behavior. Edition selection is still build
derived in this phase: Community for the default binary and Enterprise for
`enterprise-client`; Individual is defined for the later authenticated hosted flow
but is not selectable yet. Citation: `src-tauri/src/app_policy.rs :: current()`.

## The feature

`apps/dystil/src-tauri/Cargo.toml`:

```toml
enterprise-client = ["cloud-sync", "official-build"]
```

CI builds the two editions from separate workflows —
`.github/workflows/release-app.yml` (community) and `release-enterprise.yml`, which
passes `cargo_features: enterprise-client` and `publish_mode: enterprise`.

## What the feature gates

| Behaviour | Where |
|---|---|
| Product behavior | `src-tauri/src/app_policy.rs :: AppPolicy` |
| Capture policy | `src-tauri/src/capture_policy.rs :: product_capture_mode()` |
| Screenshot and segment capture defaults | `src-tauri/src/store.rs :: SyncConsent::effective()` |
| Consent requirements | `src-tauri/src/commands.rs :: set_sync_consent()` |

## Telemetry is not gated by this feature

`dystil-telemetry` is an unconditional dependency (root `Cargo.toml`, no
`optional = true`), and `mod telemetry_exporter` carries no `#[cfg]`. Telemetry is
independent of `cloud-sync`; enabling one does not enable the other.

What the edition does change:

| | Community | Enterprise |
|---|---|---|
| Default | on, user-disableable | on, organization-managed |
| Telemetry UI | toggle + disclosure card | **none shown at all** |
| `set_telemetry_enabled(false)` | persists | rejected |
| `DYSTIL_TELEMETRY=0` | honored | honored |

See `PRIVACY_AND_TELEMETRY.md` for the resolution order and the payload contract.

## The cloud endpoint is absent, not disabled

`apps/dystil/src-tauri/src/app_config.rs` resolves both network endpoints through
`option_env!`, so they are compiled in only when set at build time:

- `cloud_base_url()` → `option_env!("DYSTIL_CLOUD_BASE_URL")`
- `telemetry_endpoint()` → `option_env!("DYSTIL_TELEMETRY_ENDPOINT")`

Its module doc states the endpoint "is only injected into cloud-capable release
builds," and the telemetry doc comment states it "is absent from community builds
and must use HTTPS outside a debug localhost build."

There is a test asserting this: `app_config.rs :: community_build_has_no_cloud_url`.

**Do not replace this with a runtime flag.** The guarantee is that the community
binary has nowhere to send data, which is stronger than a setting that defaults to
off.

## Summary

| | Community (open source) | Enterprise |
|---|---|---|
| Local Worth Fixing / automation / Ready to use | yes | disabled |
| Captured content transmitted | never | never |
| Cloud endpoint compiled in | no | yes (`cloud-sync`) |
| Telemetry endpoint in official builds | yes, disableable | yes, org-managed |
| Telemetry in source builds | only if you set `DYSTIL_TELEMETRY_ENDPOINT` | same |
| Screenshot + segment capture | user choice / consent | organization enabled / required |
| Signed official builds | build it yourself | yes (`official-build`) |
