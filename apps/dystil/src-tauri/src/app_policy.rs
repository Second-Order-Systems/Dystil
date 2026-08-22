//! Product behavior selected by the compiled edition.
//!
//! This is deliberately separate from `BuildCapabilities`: capabilities describe
//! immutable facts about the binary, while this module describes what the
//! product permits and who controls it.

use std::sync::{Arc, OnceLock, RwLock};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Edition {
    Community,
    Individual,
    Enterprise,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Availability {
    Enabled,
    Disabled,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum Management {
    User,
    Organization,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum ScreenshotPolicy {
    UserChoice,
    OrganizationEnabled,
    Prohibited,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum SyncPolicy {
    Disabled,
    UserConsent,
    Required,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AskBackend {
    Local,
    Cloud,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PreferenceControl {
    UserEditable,
    Fixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct CapturePolicy {
    pub availability: Availability,
    pub permanent_control: Management,
    pub temporary_pause: Availability,
    pub exclusions_control: Management,
    pub local_deletion: Availability,
    pub screenshots: ScreenshotPolicy,
    pub sync: SyncPolicy,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPolicy {
    pub delivery: Availability,
    pub preferences: PreferenceControl,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct EditionAssignment {
    pub schema_version: u32,
    pub edition: Edition,
    pub revision: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum AssignmentSource {
    Fresh,
    Cached,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AppPolicySnapshot {
    pub status: String,
    pub assignment: Option<EditionAssignment>,
    pub policy: Option<AppPolicy>,
    pub source: Option<AssignmentSource>,
}

#[derive(Clone)]
pub struct AppPolicyState(Arc<RwLock<AppPolicySnapshot>>);

static ACTIVE_POLICY: OnceLock<AppPolicyState> = OnceLock::new();

const COMMUNITY: AppPolicy = AppPolicy {
    edition: Edition::Community,
    local_worth_fixing: Availability::Enabled,
    local_automation: Availability::Enabled,
    local_ai: Availability::Enabled,
    ready_to_use: Availability::Enabled,
    ask_backend: AskBackend::Local,
    capture: CapturePolicy {
        availability: Availability::Enabled,
        permanent_control: Management::User,
        temporary_pause: Availability::Enabled,
        exclusions_control: Management::User,
        local_deletion: Availability::Enabled,
        screenshots: ScreenshotPolicy::UserChoice,
        sync: SyncPolicy::UserConsent,
    },
    telemetry_management: Management::User,
    update_management: Management::User,
    manual_update: Availability::Enabled,
    autostart_management: Management::User,
    notifications: NotificationPolicy {
        delivery: Availability::Enabled,
        preferences: PreferenceControl::UserEditable,
    },
    team_invitation: Availability::Enabled,
};
const INDIVIDUAL: AppPolicy = AppPolicy {
    edition: Edition::Individual,
    ..COMMUNITY
};
const ENTERPRISE: AppPolicy = AppPolicy {
    edition: Edition::Enterprise,
    local_worth_fixing: Availability::Disabled,
    local_automation: Availability::Disabled,
    local_ai: Availability::Disabled,
    ready_to_use: Availability::Disabled,
    ask_backend: AskBackend::Cloud,
    capture: CapturePolicy {
        availability: Availability::Enabled,
        permanent_control: Management::Organization,
        temporary_pause: Availability::Enabled,
        exclusions_control: Management::User,
        local_deletion: Availability::Enabled,
        screenshots: ScreenshotPolicy::OrganizationEnabled,
        sync: SyncPolicy::Required,
    },
    telemetry_management: Management::Organization,
    update_management: Management::Organization,
    manual_update: Availability::Enabled,
    autostart_management: Management::Organization,
    notifications: NotificationPolicy {
        delivery: Availability::Enabled,
        preferences: PreferenceControl::Fixed,
    },
    team_invitation: Availability::Disabled,
};

pub const fn community() -> AppPolicy {
    COMMUNITY
}
pub const fn individual() -> AppPolicy {
    INDIVIDUAL
}
pub const fn enterprise() -> AppPolicy {
    ENTERPRISE
}
const LOCKED_HOSTED: AppPolicy = AppPolicy {
    edition: Edition::Enterprise,
    local_worth_fixing: Availability::Disabled,
    local_automation: Availability::Disabled,
    local_ai: Availability::Disabled,
    ready_to_use: Availability::Disabled,
    ask_backend: AskBackend::Local,
    capture: CapturePolicy {
        availability: Availability::Disabled,
        permanent_control: Management::Organization,
        temporary_pause: Availability::Disabled,
        exclusions_control: Management::Organization,
        local_deletion: Availability::Disabled,
        screenshots: ScreenshotPolicy::Prohibited,
        sync: SyncPolicy::Disabled,
    },
    telemetry_management: Management::Organization,
    update_management: Management::Organization,
    manual_update: Availability::Disabled,
    autostart_management: Management::Organization,
    notifications: NotificationPolicy {
        delivery: Availability::Disabled,
        preferences: PreferenceControl::Fixed,
    },
    team_invitation: Availability::Disabled,
};

impl AppPolicyState {
    pub fn new() -> Self {
        let snapshot = if cfg!(feature = "enterprise-client") {
            AppPolicySnapshot {
                status: "resolving".to_string(),
                assignment: None,
                policy: None,
                source: None,
            }
        } else {
            AppPolicySnapshot {
                status: "ready".to_string(),
                assignment: None,
                policy: Some(COMMUNITY),
                source: None,
            }
        };
        Self(Arc::new(RwLock::new(snapshot)))
    }

    pub fn snapshot(&self) -> AppPolicySnapshot {
        self.0.read().expect("policy state lock poisoned").clone()
    }

    fn set(&self, snapshot: AppPolicySnapshot) {
        *self.0.write().expect("policy state lock poisoned") = snapshot;
    }

    pub fn resolve(
        &self,
        assignment: EditionAssignment,
        source: AssignmentSource,
    ) -> Result<(), String> {
        let policy = policy_for_assignment(&assignment)?;
        self.set(AppPolicySnapshot {
            status: "ready".to_string(),
            assignment: Some(assignment),
            policy: Some(policy),
            source: Some(source),
        });
        Ok(())
    }

    pub fn resolving(&self) {
        self.set(AppPolicySnapshot {
            status: "resolving".to_string(),
            assignment: None,
            policy: None,
            source: None,
        });
    }
    pub fn error(&self) {
        self.set(AppPolicySnapshot {
            status: "error".to_string(),
            assignment: None,
            policy: None,
            source: None,
        });
    }
}

pub fn install(state: AppPolicyState) {
    let _ = ACTIVE_POLICY.set(state);
}

pub fn state() -> Option<&'static AppPolicyState> {
    ACTIVE_POLICY.get()
}

pub fn current() -> AppPolicy {
    state()
        .and_then(|state| state.snapshot().policy)
        .unwrap_or_else(|| {
            if cfg!(feature = "enterprise-client") {
                LOCKED_HOSTED
            } else {
                COMMUNITY
            }
        })
}

pub fn policy_for_assignment(assignment: &EditionAssignment) -> Result<AppPolicy, String> {
    if assignment.schema_version != 1 || assignment.revision == 0 {
        return Err("unsupported app-policy assignment".to_string());
    }
    match assignment.edition {
        Edition::Individual => Ok(INDIVIDUAL),
        Edition::Enterprise => Ok(ENTERPRISE),
        Edition::Community => Err("hosted policy assignment cannot select community".to_string()),
    }
}

/// Gate every desktop-owned cloud product producer. Authentication, device
/// management, and privacy-safe telemetry intentionally do not use this gate.
pub fn require_cloud_product() -> Result<(), String> {
    if matches!(current().capture.sync, SyncPolicy::Required)
        && matches!(current().ask_backend, AskBackend::Cloud)
    {
        Ok(())
    } else {
        Err("This cloud product feature is unavailable for the active policy.".to_string())
    }
}

pub fn apply_assignment(
    assignment: EditionAssignment,
    source: AssignmentSource,
) -> Result<(), String> {
    let state = state().ok_or_else(|| "app policy state is unavailable".to_string())?;
    state.resolve(assignment, source)
}

pub fn clear_hosted() {
    if let Some(state) = state() {
        state.resolving();
    }
}

pub fn mark_error() {
    if let Some(state) = state() {
        state.error();
    }
}

pub fn publish_snapshot(app: &AppHandle) -> Result<(), String> {
    let state = state().ok_or_else(|| "app policy state is unavailable".to_string())?;
    app.emit("app-policy-changed", state.snapshot())
        .map_err(|error| error.to_string())
}

/// Reconcile local and cloud product workers after an assignment transition.
/// Each worker owns an idempotent retained task handle, so this may be called
/// after every login, refresh, cache fallback, and sign-out.
pub async fn reconcile_runtime(app: &AppHandle) -> Result<(), String> {
    let policy = current();
    if matches!(policy.capture.availability, Availability::Enabled) {
        // A policy transition must not override a user-selected timed pause.
        // The pause-resume path will start capture again when it expires.
        let capture_is_paused = crate::store::SettingsStore::get(app)
            .ok()
            .flatten()
            .is_some_and(|settings| settings.capture_paused);
        if !capture_is_paused {
            if let Some(recording) = app.try_state::<crate::recording::RecordingState>() {
                let server_is_running = recording.server.lock().await.is_some();
                if server_is_running
                    && !recording
                        .capture_active
                        .load(std::sync::atomic::Ordering::SeqCst)
                {
                    crate::recording::start_capture(recording, app.clone()).await?;
                }
            }
        }
    } else if let Some(recording) = app.try_state::<crate::recording::RecordingState>() {
        crate::recording::stop_capture(recording, app.clone()).await?;
    }
    if matches!(policy.local_automation, Availability::Enabled) {
        crate::automation_commands::start_manager(app.clone());
    } else {
        crate::automation_commands::stop_manager();
    }
    if matches!(policy.local_worth_fixing, Availability::Enabled) {
        crate::worth_fixing_engine::start(app.clone());
    } else {
        crate::worth_fixing_engine::stop();
    }
    #[cfg(feature = "cloud-sync")]
    crate::work_insights_engine::reconcile(app.clone()).await?;
    Ok(())
}

pub async fn reconcile_and_publish(app: &AppHandle) -> Result<(), String> {
    reconcile_runtime(app).await?;
    publish_snapshot(app)
}
#[tauri::command]
#[specta::specta]
pub fn get_app_policy() -> AppPolicy {
    current()
}

#[tauri::command]
#[specta::specta]
pub fn get_app_policy_snapshot(state: tauri::State<'_, AppPolicyState>) -> AppPolicySnapshot {
    state.snapshot()
}

/// Records only the policy-load failure count. The browser retains the detailed
/// failure in local logs; no error text or browser data is exported.
#[tauri::command]
#[specta::specta]
pub async fn record_app_policy_load_failed(
    recording: tauri::State<'_, crate::recording::RecordingState>,
) -> Result<(), String> {
    let telemetry = recording
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.telemetry.clone());
    if let Some(telemetry) = telemetry {
        telemetry.record_product_event(dystil_telemetry::ProductEventKind::AppPolicyLoadFailed, 1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_policy_matches_build() {
        assert_eq!(
            current().edition,
            if cfg!(feature = "enterprise-client") {
                Edition::Enterprise
            } else {
                Edition::Community
            }
        );
    }

    #[test]
    fn individual_is_community_product_behavior() {
        let mut expected = community();
        expected.edition = Edition::Individual;
        assert_eq!(individual(), expected);
    }

    #[test]
    fn community_values_are_user_controlled_and_local() {
        assert_eq!(
            community(),
            AppPolicy {
                edition: Edition::Community,
                local_worth_fixing: Availability::Enabled,
                local_automation: Availability::Enabled,
                local_ai: Availability::Enabled,
                ready_to_use: Availability::Enabled,
                ask_backend: AskBackend::Local,
                capture: CapturePolicy {
                    availability: Availability::Enabled,
                    permanent_control: Management::User,
                    temporary_pause: Availability::Enabled,
                    exclusions_control: Management::User,
                    local_deletion: Availability::Enabled,
                    screenshots: ScreenshotPolicy::UserChoice,
                    sync: SyncPolicy::UserConsent
                },
                telemetry_management: Management::User,
                update_management: Management::User,
                manual_update: Availability::Enabled,
                autostart_management: Management::User,
                notifications: NotificationPolicy {
                    delivery: Availability::Enabled,
                    preferences: PreferenceControl::UserEditable
                },
                team_invitation: Availability::Enabled
            }
        );
    }

    #[test]
    fn enterprise_values_are_organization_managed() {
        assert_eq!(
            enterprise(),
            AppPolicy {
                edition: Edition::Enterprise,
                local_worth_fixing: Availability::Disabled,
                local_automation: Availability::Disabled,
                local_ai: Availability::Disabled,
                ready_to_use: Availability::Disabled,
                ask_backend: AskBackend::Cloud,
                capture: CapturePolicy {
                    availability: Availability::Enabled,
                    permanent_control: Management::Organization,
                    temporary_pause: Availability::Enabled,
                    exclusions_control: Management::User,
                    local_deletion: Availability::Enabled,
                    screenshots: ScreenshotPolicy::OrganizationEnabled,
                    sync: SyncPolicy::Required
                },
                telemetry_management: Management::Organization,
                update_management: Management::Organization,
                manual_update: Availability::Enabled,
                autostart_management: Management::Organization,
                notifications: NotificationPolicy {
                    delivery: Availability::Enabled,
                    preferences: PreferenceControl::Fixed
                },
                team_invitation: Availability::Disabled
            }
        );
    }

    #[test]
    fn server_assignment_selects_only_hosted_policies() {
        assert_eq!(
            policy_for_assignment(&EditionAssignment {
                schema_version: 1,
                edition: Edition::Individual,
                revision: 1,
            })
            .unwrap(),
            individual()
        );
        assert_eq!(
            policy_for_assignment(&EditionAssignment {
                schema_version: 1,
                edition: Edition::Enterprise,
                revision: 2,
            })
            .unwrap(),
            enterprise()
        );
    }

    #[test]
    fn unsupported_server_assignment_is_never_guessed() {
        assert!(policy_for_assignment(&EditionAssignment {
            schema_version: 2,
            edition: Edition::Enterprise,
            revision: 1,
        })
        .is_err());
        assert!(policy_for_assignment(&EditionAssignment {
            schema_version: 1,
            edition: Edition::Community,
            revision: 1,
        })
        .is_err());
        assert!(policy_for_assignment(&EditionAssignment {
            schema_version: 1,
            edition: Edition::Individual,
            revision: 0,
        })
        .is_err());
    }
}
