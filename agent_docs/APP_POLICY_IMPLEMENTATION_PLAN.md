---
status: verified
authority: approved-implementation-plan
verified_against: working-tree
verified_on: 2026-08-21
---

# App policy centralization plan

## Objective

Introduce one typed Rust `AppPolicy` that describes the behavior currently selected
by the `enterprise-client` Cargo feature. Migrate backend and frontend gates to that
policy while preserving current behavior except for the two explicitly approved UX
changes below.

This is the behavior-preserving foundation for a later change where the server
assigns an authenticated user to an edition. Server-selected editions and remote
policy are explicitly deferred. Two small intentional UX changes are included:
Enterprise users may manually install an available update, and policy-load failure
gets a blocking retry screen.

## Model

An edition is a label. `AppPolicy` is the behavior selected by that label:

```text
compiled support + selected edition -> AppPolicy -> effective behavior
```

For this implementation, edition selection remains build-derived:

```text
not enterprise-client -> Community -> community policy
enterprise-client     -> Enterprise -> enterprise policy
```

`Individual` exists in the model now and selects the same product behavior as
Community. It is deliberately unreachable in this phase. In the later server-policy
phase, the hosted binary will require authentication and the server will select
Individual or Enterprise. The Community binary will continue to require no
authentication.

## Locked product decisions

These decisions were explicitly reviewed and are requirements for this plan.

### Edition behavior

- Community requires no authentication and always uses Community policy.
- The hosted binary will eventually require authentication before selecting
  Individual or Enterprise; that runtime selection is deferred.
- Individual has Community-like product behavior: local Worth Fixing, local Ask,
  local AI settings, local automation, shortcuts, and user-controlled settings.
- Individual authentication does not itself enable cloud Ask, capture sync, or
  Dystil Cloud AI.
- Enterprise retains Ask-first behavior. Worth Fixing, Ready to Use, shortcuts,
  local automation generation, and local AI are unavailable in both the UI and
  direct backend commands.
- Invite your team remains unavailable for Enterprise and available for Community
  and Individual, preserving current behavior.

### Capture behavior

- Capture remains enabled for Enterprise, but users retain the existing temporary
  pause controls: **1 hour**, **Today** (until local midnight), and **Resume now**.
- Those pause controls remain available in both Settings and the system tray.
- Enterprise users may manage capture exclusions.
- Enterprise users may delete local captured data. The UI must not imply that this
  deletes an organization-managed cloud copy.
- Enterprise users cannot permanently disable capture through product settings.
- Enterprise screenshot capture is organization-enabled, not strictly required.
  The app uses screenshots when OS permission is available and continues without
  them when permission is denied.
- On macOS, Screen Recording is requested but does not block onboarding. On Windows,
  the existing automatically available permission behavior is unchanged.
- Enterprise cloud segment sync remains enabled. Screenshots sync only when a
  screenshot was actually captured.
- Enterprise users cannot change screenshot, cloud-sync, or autostart policy.
- Enterprise autostart preserves the existing stored value but the control remains
  locked. This phase does not force a previously disabled stored value on.

### Managed settings and UI

- Enterprise telemetry is organization-managed, but `DYSTIL_TELEMETRY=0` always
  wins as the machine-level egress kill switch.
- Enterprise automatic-update configuration remains locked. Enterprise users may
  manually select **Update now** when an update is available.
- AI models, notification preferences, Invite your team, telemetry controls,
  automatic-update controls, and autostart controls remain hidden for Enterprise.
  They are visible for Community and Individual.
- Notification delivery and notification-preference control are distinct. Enterprise
  may still receive notifications using fixed preferences even though its preference
  UI is unavailable.

### Loading, errors, and migration

- Rust exposes semantic policy through generated TypeScript bindings. It does not
  expose presentation flags such as `showWorthFixingTab`.
- Browser-only frontend development uses Community policy when Tauri is absent.
- A failed Tauri policy request retries automatically once. A second failure blocks
  policy-controlled UI and shows an error with a manual **Try again** action.
- The error UI does not expose or open local logs.
- Local logs retain the detailed error. Privacy-safe telemetry records only an
  `app_policy_load_failed` count, with no error string, paths, or user data. One
  loading sequence records at most one count regardless of retry count.
- Policy telemetry failure must not hide or replace the original loading error.
- This is a hard internal migration: remove obsolete fields and APIs rather than
  adding compatibility aliases.
- Persisted settings and database schemas are unchanged; no migration is expected.
- No CI grep/allowlist check is added for policy references.

## Verified current behavior

`enterprise-client` enables `cloud-sync` and `official-build`. Citation:
`apps/dystil/src-tauri/Cargo.toml` —
`enterprise-client = ["cloud-sync", "official-build"]`.

| Area | Community | Enterprise | Citation |
|---|---|---|---|
| Local Worth Fixing | Worker and local pool enabled | Worker not started; pool access rejected | `apps/dystil/src-tauri/src/main.rs :: setup()`; `worth_fixing_commands.rs :: WorthFixingState::pool()` |
| Local automation | Manager and commands enabled | Manager not started; commands rejected | `apps/dystil/src-tauri/src/main.rs :: setup()`; `automation_commands.rs :: require_local_automation()` |
| Ask for a fix | Local AI/retrieval | Cloud `enterprise_ask` | `apps/dystil/src-tauri/src/ask_for_fix_commands.rs` — paired `enterprise-client` branches |
| Ready to use | Local runtime enabled | Local runtime rejected | `apps/dystil/src-tauri/src/ready_to_use_commands.rs :: runtime()` |
| Capture | Persisted text/screenshot choice | Full capture forced | `apps/dystil/src-tauri/src/capture_policy.rs :: product_capture_mode()` |
| Screenshot control | User choice | Organization-enabled when OS permission exists; app-level disabling rejected | `apps/dystil/src-tauri/src/commands.rs :: set_screenshot_capture_enabled()` |
| Cloud sync consent | User consent | Segments and screenshots required | `apps/dystil/src-tauri/src/store.rs :: SyncConsent::effective()`; `commands.rs :: set_sync_consent()` |
| Telemetry | User preference after onboarding | Organization-managed unless `DYSTIL_TELEMETRY=0` | `apps/dystil/src-tauri/src/telemetry_consent.rs :: resolve()`; `store.rs :: SettingsStore::telemetry_effective()` |
| Updates | User-managed | Automatic | `apps/dystil/src-tauri/src/updates.rs :: effective_auto_update()` |
| Login startup | User-managed | Organization-managed | `apps/dystil/src-tauri/src/commands.rs :: set_autostart()` |
| UI | Worth Fixing, shortcuts, local settings | Ask-first routes and reduced settings | `apps/dystil/components/dystil/shell/app-shell.tsx :: AppShell`; `settings-workspace.tsx :: enterpriseHiddenTabs` |
| Onboarding | Local AI setup; screenshots optional | AI setup skipped; Screen Recording requested | `apps/dystil/components/onboarding/onboarding-wizard.tsx :: getOnboardingStepIds()`; `onboarding-permissions-step.tsx :: OnboardingPermissionsStep` |

## Proposed Rust policy

Avoid ambiguous booleans such as `user_managed_capture`. Keep availability,
management authority, and required behavior separate.

```rust
pub enum Edition {
    Community,
    Individual,
    Enterprise,
}

pub enum Availability {
    Enabled,
    Disabled,
}

pub enum Management {
    User,
    Organization,
}

pub enum ScreenshotPolicy {
    UserChoice,
    OrganizationEnabled,
    Prohibited,
}

pub enum SyncPolicy {
    Disabled,
    UserConsent,
    Required,
}

pub enum AskBackend {
    Local,
    Cloud,
}

pub enum PreferenceControl {
    UserEditable,
    Fixed,
}

pub struct CapturePolicy {
    pub availability: Availability,
    pub permanent_control: Management,
    pub temporary_pause: Availability,
    pub exclusions_control: Management,
    pub local_deletion: Availability,
    pub screenshots: ScreenshotPolicy,
    pub sync: SyncPolicy,
}

pub struct NotificationPolicy {
    pub delivery: Availability,
    pub preferences: PreferenceControl,
}

pub struct AppPolicy {
    pub edition: Edition,
    pub local_worth_fixing: Availability,
    pub local_automation: Availability,
    pub local_ai: Availability,
    pub ready_to_use: Availability,
    pub ask_backend: AskBackend,
    pub capture: CapturePolicy,
    pub telemetry_management: Management,
    pub update_management: Management,
    pub manual_update: Availability,
    pub autostart_management: Management,
    pub notifications: NotificationPolicy,
    pub team_invitation: Availability,
}
```

Exact names may be refined when a call site exposes a clearer domain distinction.
Do not replace typed controls with a generic string map.

`AppPolicy` describes what is permitted and who controls it. It does not contain
runtime state such as whether capture is paused, whether an AI provider is
configured, or whether an update is available. Those stay in their existing
settings and runtime owners.

## Compile-time boundary

The policy must not weaken community-build guarantees:

- Community builds continue to omit cloud endpoints and cloud-only code.
- `enterprise_ask` remains behind a true code-availability compile guard.
- `cloud-sync` and `official-build` remain Cargo feature decisions.
- Policy can permit only behavior supported by the compiled binary.
- Local Worth Fixing, automation, Ready-to-use, and local Ask implementations are
  compiled into the hosted binary in preparation for Individual, but Enterprise
  policy keeps them disabled.

Initially, `app_policy::current()` may use
`cfg!(feature = "enterprise-client")`. It should become the only behavioral use of
that feature outside tests and genuine code-availability `#[cfg]` guards.

## Implementation plan

### 1. Add `AppPolicy`

- Add `src-tauri/src/app_policy.rs` with typed Community, Individual, and Enterprise
  policies.
- Have `current()` select Community or Enterprise from the existing build feature.
- Add a Tauri command returning the current policy through generated bindings.
- Keep `BuildCapabilities` for immutable compiled facts: cloud availability, auth
  mode, cloud base URL, and official-build status.
- Add unit tests asserting every current Community and Enterprise policy value.
- Assert that Individual product behavior matches Community while remaining
  unreachable through build selection in this phase.

### 2. Migrate backend setting controls

Replace behavioral feature checks with typed policy decisions for:

- capture mode and screenshot requirements;
- temporary pause, exclusions, and local-deletion authority;
- sync consent;
- telemetry consent and settings;
- update management;
- manual update availability;
- autostart management;
- notification preference management and team invitation availability;
- permission requirements.

Backend commands remain authoritative even when frontend controls are hidden.

### 3. Migrate product-service gates

- Gate Worth Fixing startup and pool access through `local_worth_fixing`.
- Gate automation startup and commands through `local_automation`.
- Gate Ready-to-use generation through `ready_to_use` and `local_ai`.
- Select local versus cloud Ask through `ask_backend` at one boundary instead of
  repeated command-level feature branches.
- Preserve compile guards around cloud-only implementations.
- Compile local product implementations into the hosted binary so Individual can
  select them later, while preserving their disabled Enterprise behavior now.

The policy is still static in this phase, so worker lifecycle remains one-time at
startup. Cancellable reconciliation is deferred until policy becomes runtime
changeable.

### 4. Migrate the frontend

- Add one app-policy provider/hook backed by the generated Tauri command.
- Replace repeated `getBuildCapabilities().enterpriseManaged` reads.
- Derive routes, navigation, settings, onboarding, and privacy presentation from
  semantic policy fields.
- Keep a loading state until policy is known; do not briefly render community
  controls in an enterprise build.
- Remove `enterpriseManaged` component props where policy replaces them.
- In browser-only development, return Community policy without invoking Tauri.
- Retry one failed Tauri policy load automatically, then render the blocking error
  and manual retry action.
- Record detailed failures locally and emit only the privacy-safe failure count when
  telemetry is available.

### 5. Remove policy drift

- Remove `BuildCapabilities.enterprise_managed` after all consumers migrate.
- Allow direct `enterprise-client` references only in `app_policy.rs`, tests, Cargo
  declarations, and genuine code-availability guards.
- Regenerate and verify Rust-to-TypeScript bindings.
- Update `agent_docs/EDITIONS.md` and `agent_docs/PRIVACY_AND_TELEMETRY.md` to cite
  the new policy symbols.
- Source the telemetry edition attribute from `AppPolicy.edition`, including the
  future `individual` value, rather than directly from the Cargo feature.

## Validation

Test both build flavors:

| Validation | Community/default | `enterprise-client` |
|---|---:|---:|
| Rust policy tests | Required | Required |
| Frontend policy tests | Community fixture | Enterprise fixture |
| Routes/settings/onboarding | Local behavior | Current managed behavior |
| Capture and consent | User choice | Managed requirements |
| Ask backend | Local | Cloud |
| Worker selection | Local workers selected | Local workers not selected |
| Policy-load failure | Retry then blocking error | Retry then blocking error |
| Manual update | Available when supported | Available when supported |

Run at minimum:

```bash
cd apps/dystil
bun run test
bun run typecheck
bun run bindings:generate
bun run bindings:check

cd ../..
cargo test -p dystil-app
cargo test -p dystil-app --features enterprise-client
cargo fmt --check
```

Run scoped clippy for every Rust area changed.

## Acceptance criteria

- Default/community behavior is unchanged.
- Current enterprise behavior is unchanged except for the two approved UX changes.
- One typed `AppPolicy` is the source of behavioral edition decisions.
- `BuildCapabilities` contains compiled facts, not product policy.
- Frontend components no longer ask whether the build is enterprise-managed.
- Backend commands enforce policy when invoked directly.
- Community builds retain compile-time absence of cloud endpoints and cloud-only
  implementations.
- Both build flavors are covered by tests.
- Individual policy is defined and Community-like but is not selected in this phase.
- Enterprise preserves temporary capture pause, exclusions, local deletion, and
  best-effort screenshots exactly as specified above.
- Enterprise manual update and policy-load retry/error behavior match the two
  approved intentional changes.

## Deferred

- Server-selected editions.
- Runtime policy refresh, expiry, or caching.
- Entitlement overrides.
- Worker cancellation during account switching.
- Server-defined dependency graphs.

These should begin only after this behavior-preserving migration is merged and its
two-edition test matrix is green.
