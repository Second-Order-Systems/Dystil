//! Typed Tauri boundary for the bounded Ask-for-a-fix conversation.

use std::collections::HashMap;

use dystil_ai::{AiRuntime, AiRuntimeError, AiRuntimeErrorCode};
use dystil_insights::{
    cancel_ask_for_fix_turn as cancel_turn, create_ask_for_fix_session, get_ask_for_fix_session,
    keep_ask_for_fix_artifact, latest_ask_for_fix_session, lock_ask_for_fix, retry_ask_for_fix,
    run_locked_ask_for_fix, run_staged_ask_for_fix, set_ask_for_fix_error,
    stage_ask_for_fix_turn, AskInputEvent, AskSessionView, AskUserTurn,
};
use tauri::{AppHandle, State};
use tokio::sync::{oneshot, Mutex};

use crate::{recording::RecordingState, worth_fixing_commands::WorthFixingState};

#[derive(Default)]
pub struct AskForFixState {
    inflight: Mutex<HashMap<String, oneshot::Sender<()>>>,
}

fn runtime_error_code(error: &AiRuntimeError) -> &'static str {
    match error.code {
        AiRuntimeErrorCode::NotReady => "provider_not_ready",
        AiRuntimeErrorCode::Authentication => "authentication",
        AiRuntimeErrorCode::Timeout => "timeout",
        AiRuntimeErrorCode::InvalidOutput => "invalid_output",
        AiRuntimeErrorCode::Transport => "transport",
        AiRuntimeErrorCode::Internal => "internal",
    }
}

async fn runtime(
    app: &AppHandle,
    recording: &RecordingState,
) -> Result<Box<dyn AiRuntime>, AiRuntimeError> {
    let capture = {
        let server = recording.server.lock().await;
        server
            .as_ref()
            .ok_or_else(|| {
                AiRuntimeError::new(AiRuntimeErrorCode::NotReady, "capture database is not ready")
            })?
            .db
            .pool
            .clone()
    };
    let timezone = crate::ai::local_timezone_offset();
    crate::ai_runtime::resolve(app, recording, &capture, &timezone).await
}

async fn record_runtime_error(
    pool: &sqlx::SqlitePool,
    session_id: &str,
    error: &AiRuntimeError,
) -> Result<AskSessionView, String> {
    set_ask_for_fix_error(pool, session_id, runtime_error_code(error), &error.message)
        .await
        .map_err(|failure| failure.to_string())
}

async fn run_cancellable<F>(
    pool: &sqlx::SqlitePool,
    state: &AskForFixState,
    session_id: &str,
    future: F,
) -> Result<AskSessionView, String>
where
    F: std::future::Future<Output = dystil_insights::AskForFixResult<AskSessionView>>,
{
    let (sender, receiver) = oneshot::channel();
    {
        let mut inflight = state.inflight.lock().await;
        if inflight.contains_key(session_id) {
            return Err("an Ask-for-a-fix response is already running".into());
        }
        inflight.insert(session_id.to_string(), sender);
    }
    let result = tokio::select! {
        result = future => result.map_err(|error| error.to_string()),
        _ = receiver => {
            cancel_turn(pool, session_id).await.map_err(|error| error.to_string())?;
            Err("user_cancelled".into())
        }
    };
    state.inflight.lock().await.remove(session_id);
    result
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_latest(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
) -> Result<Option<AskSessionView>, String> {
    latest_ask_for_fix_session(state.pool(&app).await?)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_get(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    get_ask_for_fix_session(state.pool(&app).await?, &session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_create(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
) -> Result<AskSessionView, String> {
    create_ask_for_fix_session(state.pool(&app).await?)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_submit(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    session_id: String,
    turn: AskUserTurn,
) -> Result<AskSessionView, String> {
    let pool = insights.pool(&app).await?;
    let event: AskInputEvent = turn.event.clone();
    stage_ask_for_fix_turn(pool, &session_id, turn)
        .await
        .map_err(|error| error.to_string())?;
    let runtime = match runtime(&app, &recording).await {
        Ok(runtime) => runtime,
        Err(error) => {
            return record_runtime_error(pool, &session_id, &error).await;
        }
    };
    run_cancellable(
        pool,
        &ask_state,
        &session_id,
        run_staged_ask_for_fix(pool, runtime.as_ref(), &session_id, Some(event)),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_confirm(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    let pool = insights.pool(&app).await?;
    lock_ask_for_fix(pool, &session_id)
        .await
        .map_err(|error| error.to_string())?;
    let runtime = match runtime(&app, &recording).await {
        Ok(runtime) => runtime,
        Err(error) => return record_runtime_error(pool, &session_id, &error).await,
    };
    run_cancellable(
        pool,
        &ask_state,
        &session_id,
        run_locked_ask_for_fix(pool, runtime.as_ref(), &session_id),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_retry(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    let pool = insights.pool(&app).await?;
    let runtime = match runtime(&app, &recording).await {
        Ok(runtime) => runtime,
        Err(error) => return record_runtime_error(pool, &session_id, &error).await,
    };
    run_cancellable(
        pool,
        &ask_state,
        &session_id,
        retry_ask_for_fix(pool, runtime.as_ref(), &session_id),
    )
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_cancel(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    if let Some(sender) = ask_state.inflight.lock().await.remove(&session_id) {
        let _ = sender.send(());
    }
    cancel_turn(insights.pool(&app).await?, &session_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_keep_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<String, String> {
    keep_ask_for_fix_artifact(state.pool(&app).await?, &session_id)
        .await
        .map_err(|error| error.to_string())
}
