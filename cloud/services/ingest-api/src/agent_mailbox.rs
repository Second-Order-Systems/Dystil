use axum::extract::ws::{Message, WebSocket};
use axum::extract::{Query, State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::Response;
use axum::Json;
use dystil_protocol::agent_mailbox::{
    AgentMessageInput, AgentMessagePayload, AgentMessagesResponse, AgentPeersResponse,
    AGENT_MAILBOX_SCHEMA_VERSION,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use crate::{auth, AppError, AppState};

#[derive(Debug, Deserialize)]
pub(crate) struct CursorQuery {
    #[serde(default)]
    after: i64,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    50
}

pub(crate) async fn get_peers(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AgentPeersResponse>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let mut people = work_insights_db::agent_mailbox::list_peers(&state.pool, &principal)
        .await
        .map_err(agent_db_error)?;
    let connected_devices = state
        .agent_connections
        .lock()
        .await
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    for peer in &mut people {
        let has_connection = work_insights_db::agent_mailbox::resolve_recipient_device(
            &state.pool,
            &principal,
            &peer.user_id,
        )
        .await
        .map_err(agent_db_error)?
        .map(|device| connected_devices.contains(&device.device_id))
        .unwrap_or(false);
        if has_connection {
            peer.agent_status = "connected".into();
        }
    }
    Ok(Json(AgentPeersResponse {
        schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
        people,
    }))
}

pub(crate) async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AgentMessageInput>,
) -> Result<Json<dystil_protocol::agent_mailbox::AgentMessage>, AppError> {
    input.validate().map_err(AppError::BadRequest)?;
    let principal = auth::authenticate_device(&state, &headers).await?;
    // An exact retry must remain successful even if the daily request budget
    // was exhausted after its original insert.
    if let Some(message) =
        work_insights_db::agent_mailbox::idempotent_message(&state.pool, &principal, &input)
            .await
            .map_err(agent_db_error)?
    {
        notify_device(&state, &message.recipient_device_id).await;
        return Ok(Json(message));
    }
    if let AgentMessagePayload::Request(_) = &input.payload {
        let recipient = input
            .recipient_user_id
            .as_deref()
            .expect("validated request");
        if work_insights_db::agent_mailbox::request_rate_limit_exceeded(
            &state.pool,
            &principal,
            recipient,
        )
        .await
        .map_err(agent_db_error)?
        {
            return Err(AppError::TooManyRequests(
                "automatic_agent_limit_reached".into(),
            ));
        }
    }
    let message = work_insights_db::agent_mailbox::insert_message(&state.pool, &principal, &input)
        .await
        .map_err(agent_db_error)?;
    notify_device(&state, &message.recipient_device_id).await;
    Ok(Json(message))
}

pub(crate) async fn get_messages(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<CursorQuery>,
) -> Result<Json<AgentMessagesResponse>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let messages = work_insights_db::agent_mailbox::list_messages(
        &state.pool,
        &principal,
        query.after,
        query.limit,
    )
    .await
    .map_err(agent_db_error)?;
    let next_cursor = messages
        .last()
        .map(|message| message.sequence_id)
        .unwrap_or(query.after.max(0));
    let _ = work_insights_db::agent_mailbox::delete_expired(&state.pool, 100).await;
    Ok(Json(AgentMessagesResponse {
        schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
        messages,
        next_cursor,
    }))
}

pub(crate) async fn websocket(
    State(state): State<AppState>,
    headers: HeaderMap,
    websocket: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    Ok(websocket.on_upgrade(move |socket| serve_socket(state, principal.device_id, socket)))
}

async fn serve_socket(state: AppState, device_id: String, mut socket: WebSocket) {
    let (sender, mut receiver) = tokio::sync::mpsc::channel::<()>(1);
    state
        .agent_connections
        .lock()
        .await
        .insert(device_id.clone(), sender);
    let connection_id = Uuid::new_v4().to_string();
    tracing::debug!(%device_id, %connection_id, "agent mailbox websocket connected");
    loop {
        tokio::select! {
            next = socket.recv() => match next {
                Some(Ok(Message::Ping(bytes))) => {
                    if socket.send(Message::Pong(bytes)).await.is_err() { break; }
                }
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                Some(Ok(_)) => {}
            },
            notice = receiver.recv() => {
                if notice.is_none() { break; }
                let payload = json!({"type": "mailbox.changed"}).to_string();
                if socket.send(Message::Text(payload)).await.is_err() { break; }
            }
        }
    }
    // Do not remove blindly: a reconnect may already have replaced this sender.
    tracing::debug!(%device_id, %connection_id, "agent mailbox websocket disconnected");
}

async fn notify_device(state: &AppState, device_id: &str) {
    let sender = state.agent_connections.lock().await.get(device_id).cloned();
    if let Some(sender) = sender {
        if sender.try_send(()).is_err() && sender.is_closed() {
            state.agent_connections.lock().await.remove(device_id);
        }
    }
}

fn agent_db_error(error: work_insights_db::DbError) -> AppError {
    match error {
        work_insights_db::DbError::Other(message) => AppError::BadRequest(message),
        work_insights_db::DbError::Sqlx(sqlx::Error::Database(error))
            if error.constraint() == Some("agent_messages_one_terminal_reply") =>
        {
            AppError::BadRequest("request already has a terminal response".into())
        }
        other => AppError::from(other),
    }
}
