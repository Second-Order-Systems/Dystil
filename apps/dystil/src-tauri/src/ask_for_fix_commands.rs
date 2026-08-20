//! Typed Tauri boundary for the bounded Ask-for-a-fix conversation.

use std::collections::HashMap;

use dystil_ai::{AiRuntime, AiRuntimeError, AiRuntimeErrorCode};
use dystil_insights::{
    cancel_ask_for_fix_turn as cancel_turn, create_ask_for_fix_session, get_ask_for_fix_session,
    keep_ask_for_fix_artifact, latest_ask_for_fix_session, lock_ask_for_fix,
    recover_interrupted_ask_for_fix_turn, retry_ask_for_fix, review_ask_for_fix_watch,
    run_locked_ask_for_fix, run_staged_ask_for_fix, set_ask_for_fix_error, stage_ask_for_fix_turn,
    start_ask_for_fix_watch, stop_ask_for_fix_watch, update_ask_for_fix_watch_guidance,
    AskInputEvent, AskSessionView, AskUserTurn,
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
                AiRuntimeError::new(
                    AiRuntimeErrorCode::NotReady,
                    "capture database is not ready",
                )
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
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state);
        return crate::enterprise_ask::latest().await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let pool = state.pool(&app).await?;
        let latest = latest_ask_for_fix_session(pool)
            .await
            .map_err(|error| error.to_string())?;
        match latest {
            Some(session) if session.status == "working" => {
                recover_interrupted_ask_for_fix_turn(pool, &session.session_id)
                    .await
                    .map(Some)
                    .map_err(|error| error.to_string())
            }
            other => Ok(other),
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_get(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state);
        return crate::enterprise_ask::get(&session_id).await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        get_ask_for_fix_session(state.pool(&app).await?, &session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_create(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state);
        return crate::enterprise_ask::create().await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        create_ask_for_fix_session(state.pool(&app).await?)
            .await
            .map_err(|error| error.to_string())
    }
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
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, ask_state, insights, recording);
        return crate::enterprise_ask::submit(&session_id, turn).await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let pool = insights.pool(&app).await?;
        let session_for_run = session_id.clone();
        run_cancellable(pool, &ask_state, &session_id, async move {
            let current = get_ask_for_fix_session(pool, &session_for_run).await?;
            if current.status == "working" {
                recover_interrupted_ask_for_fix_turn(pool, &session_for_run).await?;
            }
            let event: AskInputEvent = turn.event.clone();
            stage_ask_for_fix_turn(pool, &session_for_run, turn).await?;
            let runtime = match runtime(&app, &recording).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    return record_runtime_error(pool, &session_for_run, &error)
                        .await
                        .map_err(dystil_insights::AskForFixError::InvalidState);
                }
            };
            run_staged_ask_for_fix(pool, runtime.as_ref(), &session_for_run, Some(event)).await
        })
        .await
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_confirm(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    session_id: String,
    summary: Option<String>,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, ask_state, insights, recording);
        return crate::enterprise_ask::finalize(&session_id, summary).await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let _ = summary;
        let pool = insights.pool(&app).await?;
        let session_for_run = session_id.clone();
        run_cancellable(pool, &ask_state, &session_id, async move {
            lock_ask_for_fix(pool, &session_for_run).await?;
            let runtime = match runtime(&app, &recording).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    return record_runtime_error(pool, &session_for_run, &error)
                        .await
                        .map_err(dystil_insights::AskForFixError::InvalidState);
                }
            };
            run_locked_ask_for_fix(pool, runtime.as_ref(), &session_for_run).await
        })
        .await
    }
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
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, ask_state, insights, recording);
        return crate::enterprise_ask::get(&session_id).await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let pool = insights.pool(&app).await?;
        let session_for_run = session_id.clone();
        run_cancellable(pool, &ask_state, &session_id, async move {
            let runtime = match runtime(&app, &recording).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    return record_runtime_error(pool, &session_for_run, &error)
                        .await
                        .map_err(dystil_insights::AskForFixError::InvalidState);
                }
            };
            retry_ask_for_fix(pool, runtime.as_ref(), &session_for_run).await
        })
        .await
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_cancel(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, ask_state, insights);
        return crate::enterprise_ask::get(&session_id).await;
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        if let Some(sender) = ask_state.inflight.lock().await.remove(&session_id) {
            let _ = sender.send(());
        }
        cancel_turn(insights.pool(&app).await?, &session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_keep_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<String, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state, session_id);
        return Err(
            "Enterprise Ask requests are saved to cloud; they do not create local artifacts."
                .to_string(),
        );
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        keep_ask_for_fix_artifact(state.pool(&app).await?, &session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_start_watching(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state, session_id);
        return Err(
            "Enterprise Ask creates its cloud watch when you confirm the summary.".to_string(),
        );
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        start_ask_for_fix_watch(state.pool(&app).await?, &session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_stop_watching(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state, session_id);
        return Err("Enterprise Ask requests are managed in Dystil Cloud.".to_string());
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        stop_ask_for_fix_watch(state.pool(&app).await?, &session_id)
            .await
            .map_err(|error| error.to_string())
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_review_watch(
    app: AppHandle,
    ask_state: State<'_, AskForFixState>,
    insights: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    session_id: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, ask_state, insights, recording, session_id);
        return Err("Enterprise Ask does not review local watches.".to_string());
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        let pool = insights.pool(&app).await?;
        let session_for_run = session_id.clone();
        run_cancellable(pool, &ask_state, &session_id, async move {
            let runtime = match runtime(&app, &recording).await {
                Ok(runtime) => runtime,
                Err(error) => {
                    return record_runtime_error(pool, &session_for_run, &error)
                        .await
                        .map_err(dystil_insights::AskForFixError::InvalidState);
                }
            };
            review_ask_for_fix_watch(pool, runtime.as_ref(), &session_for_run).await
        })
        .await
    }
}

#[tauri::command]
#[specta::specta]
pub async fn ask_for_fix_update_watch_guidance(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    session_id: String,
    guidance: String,
) -> Result<AskSessionView, String> {
    #[cfg(feature = "enterprise-client")]
    {
        let _ = (app, state, session_id, guidance);
        return Err("Enterprise Ask does not use local watch guidance.".to_string());
    }
    #[cfg(not(feature = "enterprise-client"))]
    {
        update_ask_for_fix_watch_guidance(state.pool(&app).await?, &session_id, &guidance)
            .await
            .map_err(|error| error.to_string())
    }
}
