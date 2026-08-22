---
status: verified
authority: approved-implementation-plan
verified_against: working-tree
verified_on: 2026-08-21
---

# Server-selected app policy — desktop client plan

## Objective

Make the hosted Dystil desktop obtain its organization edition from the cloud and
select the corresponding locally defined `AppPolicy` at runtime. The Community
binary remains unauthenticated, local-only, and compile-time incapable of cloud
product behavior.

This plan depends on `agent_docs/APP_POLICY_IMPLEMENTATION_PLAN.md` being implemented
first. That phase centralizes current behavior in a typed `AppPolicy`; this phase
changes only how the hosted binary selects that policy.

The paired server plan is:
`../memory/docs/organization-app-policy-server-plan.md`.

## Locked decisions

- Edition belongs to the organization.
- The server selects only an edition. The desktop continues to define the feature
  behavior for Community, Individual, and Enterprise.
- Community requires no authentication and always selects Community policy.
- The hosted binary requires authentication, registers a device for both Individual
  and Enterprise, and selects policy from `/me`.
- Individual behavior matches Community product behavior but retains hosted
  authentication and device registration.
- Individual does not use cloud Ask, capture sync, or Dystil Cloud AI.
- Enterprise retains the managed behavior defined in the phase-one `AppPolicy`.
- The desktop is the only feature blocker. The server does not reject service calls
  based on edition. Edition is therefore a product policy, not a security boundary.
- Assignment refresh occurs after login, on app startup with a restored session, and
  whenever the existing profile-refresh flow runs. There is no periodic timer.
- The last verified assignment is cached per authenticated user for offline use.
- No special data cutoff, cleanup, migration, or account/edition-switch protection
  is added. Local data remains machine-scoped.

## Shared `/me` contract

The server adds this required field to the existing identity response:

```json
{
  "appPolicyAssignment": {
    "schemaVersion": 1,
    "edition": "enterprise",
    "revision": 3
  }
}
```

Desktop types should remain closed and generated where practical:

```rust
pub struct EditionAssignment {
    pub schema_version: u32,
    pub edition: Edition,
    pub revision: u64,
}
```

Supported editions in schema version 1 are `individual` and `enterprise`.
Community is never assigned by the server.

The desktop must not accept server-supplied feature booleans or arbitrary policy
JSON. It maps `Edition` to the local typed `AppPolicy`.

## Client policy state

Replace the phase-one static hosted selection with one managed native state:

```rust
pub enum PolicyStatus {
    Resolving,
    Ready {
        assignment: EditionAssignment,
        policy: AppPolicy,
        source: AssignmentSource,
    },
    Error,
}

pub enum AssignmentSource {
    Fresh,
    Cached,
}

pub struct AppPolicyState {
    // Synchronized current status plus policy-change notification.
}
```

Community initializes immediately to `Ready` with Community policy and no server
assignment. Hosted builds initialize to `Resolving` until authentication produces a
fresh or usable cached assignment.

All Rust commands and background services read the same `AppPolicyState`. The
frontend receives a snapshot and subscribes to one policy-changed event; it does not
derive policy from auth route, organization presence, email domain, or build edition.

## Native auth and cache

Extend the existing native secret-store auth record with a cached assignment:

```rust
pub struct CachedEditionAssignment {
    pub user_id: String,
    pub assignment: EditionAssignment,
    pub verified_at: String,
}
```

Citation: `apps/dystil/src-tauri/src/auth.rs :: AuthRecord`, `read_record()`, and
`write_record()` own the existing native session, profile, device token, and pending
onboarding data.

Rules:

1. A successful authenticated `/me` response is validated before use.
2. A valid response is persisted with the resolved app user ID and applied as
   `AssignmentSource::Fresh`.
3. Network errors, timeouts, and server 5xx responses may fall back to a cached
   assignment only when its `user_id` matches the authenticated stored user.
4. An unknown edition or unsupported assignment schema version falls back to the
   same-user cache when one exists; otherwise policy loading enters a retryable error.
5. Missing assignment data is handled like an invalid response, not guessed.
6. `401` preserves current auth semantics: clear session, profile, device credential,
   cached assignment, and active policy, then return to sign-in.
7. Cache has no fixed expiry in this phase. Successful profile refresh replaces it.
8. One user's cached assignment is never used for another user.

The detailed incompatibility or network error stays in local logs. Privacy-safe
telemetry records only bounded event kinds/counts, never response bodies, user IDs,
organization IDs, paths, or exception strings.

## Bootstrap ordering

Hosted startup must follow this order:

```text
restore native auth record
  -> attempt authenticated /me
  -> validate fresh assignment
       or select same-user cache after an allowed failure
  -> update AppPolicyState
  -> reconcile capture and product services
  -> register/restore the device credential
  -> continue onboarding or enter the product
```

Device registration remains part of hosted authentication/bootstrap and occurs for
both Individual and Enterprise. It is not controlled by edition policy.

If there is no valid session or usable assignment, hosted policy-controlled product
UI remains blocked. The login surface must remain usable while policy is Resolving.

## Runtime lifecycle and reconciliation

The effective lifecycle is:

```text
Community
  -> Community policy immediately
  -> existing local capture and workers

Hosted signed out / unresolved
  -> base capture stopped
  -> local Worth Fixing and automation stopped
  -> cloud product activity stopped

Hosted Individual
  -> Individual policy
  -> local Worth Fixing and automation running
  -> local Ask and Ready to Use available
  -> cloud Ask, capture sync, and Dystil Cloud AI inactive

Hosted Enterprise
  -> Enterprise policy
  -> local Worth Fixing and automation stopped
  -> cloud Ask and capture sync active
```

Create one idempotent policy runtime supervisor. It compares the active services with
the desired `AppPolicy` and starts or stops only what changed. Local Worth Fixing and
automation loops need retained cancellation handles and graceful termination rather
than detached infinite tasks.

Policy application ordering:

1. Validate and construct the complete new policy.
2. Stop services forbidden by the new policy.
3. Apply capture settings required by the new policy.
4. Start newly allowed services.
5. Publish the new native policy snapshot.
6. Emit one frontend policy-changed event.

Sign-out ordering:

1. Stop base capture and policy-controlled services.
2. Set hosted policy to Resolving.
3. Clear session, profile, device credential, and cached assignment.
4. Show sign-in.

No transition-specific data behavior is added. In particular:

- no historical-sync cutoff is created when Individual becomes Enterprise;
- no pending sync state is specially deleted when Enterprise becomes Individual;
- no local Worth Fixing or automation database is deleted or migrated;
- no per-user local database partition is introduced.

Normal service behavior applies when a service is later restarted. This means
existing local capture may be eligible for normal sync if Enterprise is selected.

## Client-only cloud gating

Because the server deliberately does not enforce edition, every desktop cloud call
site must consult the active `AppPolicy` before starting or sending:

- cloud Ask;
- segment and image sync;
- sync-config refresh;
- semantic-tree uploads;
- cloud memory requests;
- agent mailbox/peer activity;
- capture-state reporting;
- Dystil Cloud AI use.

This is correctness enforcement, not hostile-client authorization. A modified or
outdated client with a valid credential may still call server endpoints, and the
documentation must say so plainly.

Device registration, device listing/revocation, authentication, and permitted
privacy-safe telemetry remain independent of edition.

## Frontend behavior

The phase-one app-policy provider becomes reactive to native policy snapshots.

- Community/browser-only development receives Community policy immediately.
- Hosted login is visible while policy is Resolving.
- Authenticated product UI waits for Ready policy.
- One automatic policy-load retry occurs before the blocking error screen.
- The error screen provides manual **Try again** and does not expose local logs.
- A cached assignment may satisfy Ready state after an allowed fresh-load failure.
- A policy-changed event updates routes, settings, onboarding, and product surfaces
  without frontend edition checks.

When a changed assignment arrives through an explicit auth/profile refresh, the
native supervisor reconciles first and the frontend updates afterward.

## Implementation sequence

1. Add assignment DTOs, cache representation, and validation tests.
2. Make `AppPolicyState` support Resolving and runtime Ready policies while keeping
   Community initialization static.
3. Refactor auth bootstrap to validate/cache/apply `/me` assignment.
4. Add the policy runtime supervisor and cancellable local worker handles.
5. Gate every desktop cloud producer through semantic policy fields.
6. Make capture stop during hosted signed-out/Resolving state.
7. Make the frontend provider reactive and preserve login access while Resolving.
8. Implement retry, cached fallback, error UI, and privacy-safe telemetry.
9. Update generated bindings and verified edition/privacy documentation.

## Validation matrix

| Case | Expected result |
|---|---|
| Community build | Community policy immediately; no auth or cloud endpoint |
| Hosted fresh Individual | Local product enabled; cloud product producers inactive |
| Hosted fresh Enterprise | Managed Enterprise behavior and cloud producers active |
| Network failure with same-user cache | Cached policy applied and marked Cached |
| Network failure without cache | Retry once, then blocking retry UI |
| Unknown schema/edition with cache | Cache used; incompatibility logged and counted |
| Unknown schema/edition without cache | Blocking retry UI; no guessed policy |
| `/me` 401 | Capture/services stop; auth, device, cache, and policy clear |
| Sign-out | Stop before credential/policy clearing |
| Explicit profile refresh with new revision | Generic policy reconciliation and one UI event |
| Different cached user | Cache rejected |
| Individual cloud producer invocation | Rejected locally before network I/O |

Run default and `enterprise-client` Rust tests, frontend tests/typecheck, and generated
binding freshness checks. Add focused lifecycle tests using controllable fake workers
so start/stop ordering and idempotence are deterministic.

## Acceptance criteria

- Community behavior and compile-time cloud absence remain unchanged.
- Hosted policy comes only from a fresh or valid same-user cached server assignment.
- Individual and Enterprise select their locally defined typed policies.
- Device registration remains common to both hosted editions.
- Hosted signed-out/Resolving state performs no capture or policy-controlled work.
- Worker reconciliation is idempotent and stopped workers actually terminate.
- Cloud product calls are blocked locally for Individual without server changes.
- No periodic assignment polling exists.
- No transition-specific data migration, cleanup, cutoff, or partitioning exists.
- All error, cache, sign-out, and revision-change cases above are tested.

## Deferred

- Server-side edition enforcement.
- Entitlement or per-user overrides.
- Immediate push propagation of edition changes.
- Periodic policy polling.
- Per-user local data isolation.
- Special handling of data captured before or after an edition change.
