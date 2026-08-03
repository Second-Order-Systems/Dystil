//! Typed, narrow Tauri boundary for durable Ready-to-use artifacts.

use dystil_insights::{
    capability_target, confirm_artifact_change, propose_artifact_change, ready_artifact_detail,
    ready_artifact_provenance, ready_artifacts, record_artifact_used, reject_artifact_change,
    remove_artifact, retry_artifact_change, ArtifactChangePreview, ArtifactPage,
    ReadyArtifactAction, ReadyArtifactDetail, ReadyArtifactMutationResult, ReadyArtifactUseResult,
    WorthFixingEvidenceLine,
};
use tauri::{AppHandle, State};

use crate::{recording::RecordingState, worth_fixing_commands::WorthFixingState};

async fn runtime(
    app: &AppHandle,
    recording: &RecordingState,
) -> Result<Box<dyn dystil_ai::AiRuntime>, String> {
    let capture = {
        let server = recording.server.lock().await;
        server
            .as_ref()
            .ok_or("capture database is not ready")?
            .db
            .pool
            .clone()
    };
    let timezone = crate::ai::local_timezone_offset();
    crate::ai_runtime::resolve(app, recording, &capture, &timezone)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_to_use(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    cursor: Option<String>,
    limit: u32,
) -> Result<ArtifactPage, String> {
    ready_artifacts(state.pool(&app).await?, cursor.as_deref(), limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactDetail, String> {
    ready_artifact_detail(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_artifact_provenance(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<Vec<WorthFixingEvidenceLine>, String> {
    ready_artifact_provenance(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn record_ready_artifact_used(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
    action: ReadyArtifactAction,
) -> Result<ReadyArtifactUseResult, String> {
    record_artifact_used(state.pool(&app).await?, &artifact_id, action)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn open_ready_capability(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactUseResult, String> {
    use tauri_plugin_opener::OpenerExt;

    let pool = state.pool(&app).await?;
    let target = capability_target(pool, &artifact_id)
        .await
        .map_err(|error| error.to_string())?;
    app.opener()
        .open_url(target, None::<&str>)
        .map_err(|error| error.to_string())?;
    record_artifact_used(pool, &artifact_id, ReadyArtifactAction::Open)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn remove_ready_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactMutationResult, String> {
    remove_artifact(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn propose_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    artifact_id: String,
    request: String,
) -> Result<ArtifactChangePreview, String> {
    let runtime = runtime(&app, &recording).await?;
    propose_artifact_change(
        state.pool(&app).await?,
        runtime.as_ref(),
        &artifact_id,
        &request,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    change_job_id: String,
) -> Result<ArtifactChangePreview, String> {
    let runtime = runtime(&app, &recording).await?;
    retry_artifact_change(state.pool(&app).await?, runtime.as_ref(), &change_job_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn confirm_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    change_job_id: String,
) -> Result<ReadyArtifactDetail, String> {
    confirm_artifact_change(state.pool(&app).await?, &change_job_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn reject_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    change_job_id: String,
) -> Result<ReadyArtifactDetail, String> {
    reject_artifact_change(state.pool(&app).await?, &change_job_id)
        .await
        .map_err(|error| error.to_string())
}
