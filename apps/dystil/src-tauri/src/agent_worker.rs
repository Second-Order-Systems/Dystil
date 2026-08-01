//! Background teammate-agent worker.
//!
//! A WebSocket is used only to wake the worker. Cursor HTTP synchronization is
//! authoritative, so a disconnect can delay but cannot lose a mailbox message.

use chrono::{Duration as ChronoDuration, Utc};
use dystil_protocol::agent_mailbox::{
    AgentErrorBody, AgentEvidenceLabel, AgentMessage, AgentMessagePayload, AgentResponseBody,
    AgentStage, AgentStatusBody,
};
use futures::{SinkExt, StreamExt};
use std::str::FromStr;
use tauri::{AppHandle, Emitter, Manager};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::Message;
use tracing::{debug, info, warn};

use crate::{agent_mailbox, ai, recording::RecordingState};

pub(crate) fn start(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut reconnect_delay = std::time::Duration::from_secs(2);
        loop {
            match run_connection(&app).await {
                Ok(()) => reconnect_delay = std::time::Duration::from_secs(2),
                Err(error) => {
                    debug!(reason = %error, "agent mailbox connection deferred");
                    tokio::time::sleep(reconnect_delay).await;
                    reconnect_delay = (reconnect_delay * 2).min(std::time::Duration::from_secs(60));
                }
            }
        }
    });
}

async fn run_connection(app: &AppHandle) -> Result<(), String> {
    let pool = capture_pool(app).await?;
    sync_and_process(app, &pool).await?;
    let (_, base, token) = agent_mailbox::cloud_client().await?;
    let url = agent_mailbox::websocket_url(&base)?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| error.to_string())?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&agent_mailbox::websocket_authorization(&token))
            .map_err(|error| error.to_string())?,
    );
    request.headers_mut().insert(
        "x-dystil-sync-capabilities",
        HeaderValue::from_static(dystil_protocol::agent_mailbox::AGENT_MAILBOX_CAPABILITY),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| error.to_string())?;
    info!("agent mailbox websocket connected");
    let mut reconcile = tokio::time::interval(std::time::Duration::from_secs(60));
    let mut heartbeat = tokio::time::interval(std::time::Duration::from_secs(30));
    loop {
        tokio::select! {
            inbound = socket.next() => match inbound {
                Some(Ok(Message::Text(_))) => { sync_and_process(app, &pool).await?; }
                Some(Ok(Message::Ping(payload))) => { socket.send(Message::Pong(payload)).await.map_err(|error| error.to_string())?; }
                Some(Ok(Message::Close(_))) | None => return Err("agent mailbox websocket closed".into()),
                Some(Err(error)) => return Err(error.to_string()),
                _ => {}
            },
            _ = reconcile.tick() => { sync_and_process(app, &pool).await?; }
            _ = heartbeat.tick() => {
                socket.send(Message::Ping(Vec::new())).await.map_err(|error| error.to_string())?;
            }
        }
    }
}

async fn capture_pool(app: &AppHandle) -> Result<sqlx::SqlitePool, String> {
    let state = app.state::<RecordingState>();
    ai::capture_pool(&state).await
}

async fn sync_and_process(app: &AppHandle, pool: &sqlx::SqlitePool) -> Result<(), String> {
    let new_messages = agent_mailbox::sync(pool).await?;
    if !new_messages.is_empty() {
        let _ = app.emit("agent-mailbox-updated", ());
    }
    for request in agent_mailbox::pending_requests(pool).await? {
        process_request(app, pool, request).await;
    }
    Ok(())
}

async fn process_request(app: &AppHandle, pool: &sqlx::SqlitePool, request: AgentMessage) {
    if let Err(error) = process_request_inner(app, pool, &request).await {
        warn!(message_id = %request.message_id, reason = %error, "agent teammate request failed");
        let _ = agent_mailbox::set_local_status(pool, &request.message_id, "interrupted").await;
        let _ = app.emit("agent-mailbox-updated", ());
    }
}

async fn process_request_inner(
    app: &AppHandle,
    pool: &sqlx::SqlitePool,
    request: &AgentMessage,
) -> Result<(), String> {
    let AgentMessagePayload::Request(body) = &request.payload else {
        return Ok(());
    };
    agent_mailbox::set_local_status(pool, &request.message_id, "processing").await?;
    send_status(pool, request, AgentStage::Delivered).await?;
    send_status(pool, request, AgentStage::Searching).await?;

    let end = Utc::now();
    let start = end - ChronoDuration::days(i64::from(body.search.lookback_days));
    let timezone = ai::local_timezone_offset();
    let state = app.state::<crate::recording::RecordingState>();
    let runtime = match crate::ai_runtime::resolve(app, &state, pool, &timezone).await {
        Ok(runtime) => runtime,
        Err(_) => {
            return send_error(
                pool,
                request,
                "provider_not_ready",
                "This Dystil has no AI provider ready.",
            )
            .await
        }
    };
    send_status(pool, request, AgentStage::Generating).await?;
    let answer = match runtime
        .answer(dystil_ai::AiAnswerRequest {
            requester_name: "a teammate".into(),
            question: body.question.clone(),
            search_start: start.to_rfc3339(),
            search_end: end.to_rfc3339(),
            timezone: timezone.clone(),
        })
        .await
    {
        Ok(answer) => answer,
        Err(error) if error.code == dystil_ai::AiRuntimeErrorCode::Timeout => {
            return send_error(
                pool,
                request,
                "provider_timeout",
                "The local AI provider timed out.",
            )
            .await
        }
        Err(error) if error.code == dystil_ai::AiRuntimeErrorCode::InvalidOutput => {
            return send_error(
                pool,
                request,
                "provider_invalid_output",
                "The local AI provider returned an invalid answer.",
            )
            .await
        }
        Err(_) => {
            return send_error(
                pool,
                request,
                "internal_error",
                "Dystil could not generate an answer.",
            )
            .await
        }
    };
    let mut evidence = Vec::new();
    for claim in &answer.answer.evidence {
        let Some(evidence_id_text) = claim.evidence_ids.first() else {
            continue;
        };
        if let Ok(evidence_id) = evidence_id_text.parse::<dystil_retrieval::EvidenceId>() {
            if let Ok(record) = dystil_retrieval::RetrievalService::new(pool.clone())
                .get_source(&evidence_id, Some(500))
                .await
            {
                evidence.push(AgentEvidenceLabel {
                    label: record
                        .window_name
                        .or(record.app_name)
                        .unwrap_or_else(|| record.evidence_id.to_string()),
                    local_date: ai::local_date_for_timestamp(&record.timestamp, &timezone),
                });
            }
        }
    }
    let input = agent_mailbox::new_reply(
        request,
        AgentMessagePayload::Response(AgentResponseBody {
            answer: answer.answer.answer,
            evidence,
            uncertainties: answer.answer.uncertainties,
        }),
    );
    let sent = agent_mailbox::send(&input).await?;
    agent_mailbox::persist_outgoing(pool, &sent).await?;
    agent_mailbox::set_local_status(pool, &request.message_id, "responded").await?;
    let _ = app.emit("agent-mailbox-updated", ());
    Ok(())
}

async fn send_status(
    pool: &sqlx::SqlitePool,
    request: &AgentMessage,
    stage: AgentStage,
) -> Result<(), String> {
    let input = agent_mailbox::new_reply(
        request,
        AgentMessagePayload::Status(AgentStatusBody { stage }),
    );
    let sent = agent_mailbox::send(&input).await?;
    agent_mailbox::persist_outgoing(pool, &sent).await
}

async fn send_error(
    pool: &sqlx::SqlitePool,
    request: &AgentMessage,
    code: &str,
    message: &str,
) -> Result<(), String> {
    let input = agent_mailbox::new_reply(
        request,
        AgentMessagePayload::Error(AgentErrorBody {
            code: code.into(),
            message: message.into(),
        }),
    );
    let sent = agent_mailbox::send(&input).await?;
    agent_mailbox::persist_outgoing(pool, &sent).await?;
    agent_mailbox::set_local_status(pool, &request.message_id, "responded").await
}
