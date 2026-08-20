//! Typed Tauri boundary for the local Worth Fixing backend.
//!
//! Commands expose only product DTOs and user dispositions; inference and
//! SQLite remain native.

use dystil_insights::{
    cleanup_diagnostics, finding_evidence, home_worth_fixing_summary,
    interrupt_abandoned_skill_bundle_builds, keep_finding, open_insights_database, other_findings,
    pending_observations, record_disposition, record_wake_start, run_steward_wake,
    set_enhanced_diagnostics, worth_fixing_summary, DiagnosticRetention, DispositionKind,
    FindingPage, KeepFindingResult, WakeResult, WorthFixingEvidenceLine, WorthFixingSummary,
};
use serde::Serialize;
use sqlx::SqlitePool;
use tauri::{AppHandle, State};
use tokio::sync::OnceCell;

/// A finding is counted only once it has been accepted into the local
/// projection and can appear in Worth Fixing. No finding content or ID enters
/// telemetry.
pub(crate) async fn record_worth_fixing_findings_shown(
    recording: &crate::recording::RecordingState,
    count: usize,
) {
    if count == 0 {
        return;
    }
    let telemetry = recording
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.telemetry.clone());
    if let Some(telemetry) = telemetry {
        telemetry.record_product_event(
            dystil_telemetry::ProductEventKind::WorthFixingFindingShown,
            count as u64,
        );
    }
}

#[derive(Default)]
pub struct WorthFixingState {
    pool: OnceCell<SqlitePool>,
}

impl WorthFixingState {
    pub(crate) async fn pool(&self, app: &AppHandle) -> Result<&SqlitePool, String> {
        #[cfg(feature = "enterprise-client")]
        {
            let _ = (self, app);
            return Err("Local Worth Fixing is disabled in this enterprise build.".to_string());
        }
        #[cfg(not(feature = "enterprise-client"))]
        {
            self.pool
                .get_or_try_init(|| async {
                    let data_dir =
                        crate::log_files::get_data_dir(app).map_err(|error| error.to_string())?;
                    let path = data_dir.join("worth-fixing.sqlite");
                    let pool = open_insights_database(&path)
                        .await
                        .map_err(|error| error.to_string())?;
                    interrupt_abandoned_skill_bundle_builds(&pool)
                        .await
                        .map_err(|error| error.to_string())?;
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if path.exists() {
                            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                                .map_err(|error| error.to_string())?;
                        }
                    }
                    Ok(pool)
                })
                .await
        }
    }
}

pub(crate) async fn provider_ready(state: &crate::recording::RecordingState) -> bool {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = state;
        return false;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let server = state.server.lock().await;
        let Some(server) = server.as_ref() else {
            return false;
        };
        crate::ai_presets::active(&server.db.pool)
            .await
            .ok()
            .flatten()
            .is_some()
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorthFixingRefreshResult {
    pub status: String,
    pub job_id: Option<String>,
    pub opportunities_changed: u32,
    pub findings_created: u32,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorthFixingCleanupResult {
    pub removed_files: u32,
    pub removed_bytes: u64,
    pub retained_bytes: u64,
    pub enhanced_diagnostics: bool,
}

#[tauri::command]
#[specta::specta]
pub async fn get_worth_fixing_summary(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
) -> Result<WorthFixingSummary, String> {
    let pool = state.pool(&app).await?;
    worth_fixing_summary(pool, provider_ready(&recording).await)
        .await
        .map_err(|error| error.to_string())
}

/// The sole data source for the redesigned Home Worth fixing experience.
#[tauri::command]
#[specta::specta]
pub async fn get_home_worth_fixing_summary(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
) -> Result<dystil_insights::HomeWorthFixingSummary, String> {
    let pool = state.pool(&app).await?;
    home_worth_fixing_summary(pool, provider_ready(&recording).await)
        .await
        .map_err(|error| error.to_string())
}

/// Explicit refresh backend boundary. It bypasses adaptive batching thresholds
/// but not evidence admission, one-job-at-a-time execution, or durable recovery.
#[tauri::command]
#[specta::specta]
pub async fn refresh_worth_fixing(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
) -> Result<WorthFixingRefreshResult, String> {
    let insights = state.pool(&app).await?;
    if pending_observations(insights, 1)
        .await
        .map_err(|error| error.to_string())?
        .is_empty()
    {
        return Ok(WorthFixingRefreshResult {
            status: "no_pending_observations".into(),
            job_id: None,
            opportunities_changed: 0,
            findings_created: 0,
        });
    }
    let capture_db = {
        let server = recording.server.lock().await;
        server
            .as_ref()
            .ok_or("capture database is not ready")?
            .db
            .pool
            .clone()
    };
    let timezone = crate::ai::local_timezone_offset();
    let local_day =
        crate::ai::local_date_for_timestamp(&chrono::Utc::now().to_rfc3339(), &timezone);
    let runtime = crate::ai_runtime::resolve(&app, &recording, &capture_db, &timezone)
        .await
        .map_err(|error| error.to_string())?;
    record_wake_start(insights, &local_day, "explicit_request", true)
        .await
        .map_err(|error| error.to_string())?;
    let wake = run_steward_wake(
        insights,
        runtime.as_ref(),
        &local_day,
        &timezone,
        "explicit_request",
        250,
    )
    .await
    .map_err(|error| error.to_string())?;
    if let WakeResult::Accepted { ref apply, .. } = wake {
        record_worth_fixing_findings_shown(&recording, apply.findings_created).await;
    }
    match wake {
        WakeResult::NoWork => Ok(WorthFixingRefreshResult {
            status: "no_pending_observations".into(),
            job_id: None,
            opportunities_changed: 0,
            findings_created: 0,
        }),
        WakeResult::AlreadyAccepted { job_id } => Ok(WorthFixingRefreshResult {
            status: "already_accepted".into(),
            job_id: Some(job_id),
            opportunities_changed: 0,
            findings_created: 0,
        }),
        WakeResult::Accepted { job_id, apply } => Ok(WorthFixingRefreshResult {
            status: "accepted".into(),
            job_id: Some(job_id),
            opportunities_changed: apply.opportunities_changed as u32,
            findings_created: apply.findings_created as u32,
        }),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn cleanup_worth_fixing_diagnostics(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    enhanced_diagnostics: bool,
) -> Result<WorthFixingCleanupResult, String> {
    let insights = state.pool(&app).await?;
    set_enhanced_diagnostics(insights, enhanced_diagnostics)
        .await
        .map_err(|error| error.to_string())?;
    let root = crate::log_files::get_data_dir(&app)
        .map_err(|error| error.to_string())?
        .join("worth-fixing-diagnostics");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let policy = if enhanced_diagnostics {
        DiagnosticRetention::enhanced()
    } else {
        DiagnosticRetention::default()
    };
    let result = cleanup_diagnostics(&root, std::time::SystemTime::now(), policy)
        .map_err(|error| error.to_string())?;
    Ok(WorthFixingCleanupResult {
        removed_files: result.removed_files as u32,
        removed_bytes: result.removed_bytes,
        retained_bytes: result.retained_bytes,
        enhanced_diagnostics,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn get_other_worth_fixing_findings(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    cursor: Option<String>,
    limit: u32,
) -> Result<FindingPage, String> {
    other_findings(state.pool(&app).await?, cursor.as_deref(), limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_worth_fixing_evidence(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    finding_id: String,
) -> Result<Vec<WorthFixingEvidenceLine>, String> {
    finding_evidence(state.pool(&app).await?, &finding_id, 50)
        .await
        .map_err(|error| error.to_string())
}

async fn disposition(
    app: &AppHandle,
    state: &WorthFixingState,
    finding_id: &str,
    kind: DispositionKind,
    correction_text: Option<&str>,
    intent: Option<&str>,
) -> Result<String, String> {
    record_disposition(
        state.pool(app).await?,
        finding_id,
        kind,
        correction_text,
        intent,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn accept_worth_fixing_finding(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
    finding_id: String,
) -> Result<KeepFindingResult, String> {
    keep_worth_fixing_finding(app, state, recording, finding_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn save_worth_fixing_finding(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
    finding_id: String,
) -> Result<KeepFindingResult, String> {
    keep_worth_fixing_finding(app, state, recording, finding_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn keep_worth_fixing_finding(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
    finding_id: String,
) -> Result<KeepFindingResult, String> {
    let pool = state.pool(&app).await?;
    keep_finding(pool, &finding_id, provider_ready(&recording).await)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn dismiss_worth_fixing_finding(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
    finding_id: String,
    reason: DispositionKind,
) -> Result<WorthFixingSummary, String> {
    if !matches!(
        reason,
        DispositionKind::NotAProblem | DispositionKind::LeaveIt
    ) {
        return Err("dismissal reason must be not_a_problem or leave_it".into());
    }
    disposition(&app, &state, &finding_id, reason, None, None).await?;
    get_worth_fixing_summary(app, state, recording).await
}

#[tauri::command]
#[specta::specta]
pub async fn correct_worth_fixing_finding(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, crate::recording::RecordingState>,
    finding_id: String,
    correction_text: String,
    intent: String,
) -> Result<WorthFixingSummary, String> {
    disposition(
        &app,
        &state,
        &finding_id,
        DispositionKind::CloseBut,
        Some(&correction_text),
        Some(&intent),
    )
    .await?;
    get_worth_fixing_summary(app, state, recording).await
}
