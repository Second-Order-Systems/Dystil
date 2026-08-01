use dystil_protocol::agent_mailbox::AgentMessagePayload;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{agent_mailbox, ai, recording::RecordingState};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentPeerView {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: String,
    pub agent_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessageView {
    pub message_id: String,
    pub conversation_id: String,
    pub peer_user_id: String,
    pub direction: String,
    pub kind: String,
    pub local_status: String,
    pub text: String,
    pub evidence: Vec<AgentEvidenceView>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvidenceView {
    pub label: String,
    pub local_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalChatSessionView {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct LocalChatMessageView {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub mode: String,
    pub question: Option<String>,
    pub answer: Option<String>,
    pub status: String,
    pub citations_json: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub elapsed_ms: Option<i64>,
    pub error_code: Option<String>,
    pub created_at: String,
}

async fn pool(state: &RecordingState) -> Result<sqlx::SqlitePool, String> {
    ai::capture_pool(state).await
}

#[tauri::command]
#[specta::specta]
pub async fn agent_list_peers() -> Result<Vec<AgentPeerView>, String> {
    let peers = agent_mailbox::list_peers().await?;
    Ok(peers
        .people
        .into_iter()
        .map(|peer| AgentPeerView {
            user_id: peer.user_id,
            display_name: peer.display_name,
            email: peer.email,
            agent_status: peer.agent_status,
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn agent_send_question(
    recipient_user_id: String,
    question: String,
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
) -> Result<AgentMessageView, String> {
    let pool = pool(&state).await?;
    let input = agent_mailbox::new_request(recipient_user_id, question);
    input.validate()?;
    let message = agent_mailbox::send(&input).await?;
    agent_mailbox::persist_outgoing(&pool, &message).await?;
    let _ = app.emit("agent-mailbox-updated", ());
    Ok(to_view(&message, "outgoing", "sent"))
}

#[tauri::command]
#[specta::specta]
pub async fn agent_sync_now(
    state: State<'_, RecordingState>,
    app: tauri::AppHandle,
) -> Result<usize, String> {
    let pool = pool(&state).await?;
    let messages = agent_mailbox::sync(&pool).await?;
    if !messages.is_empty() {
        let _ = app.emit("agent-mailbox-updated", ());
    }
    Ok(messages.len())
}

#[tauri::command]
#[specta::specta]
pub async fn agent_list_messages(
    state: State<'_, RecordingState>,
) -> Result<Vec<AgentMessageView>, String> {
    let pool = pool(&state).await?;
    let rows = sqlx::query(
        "SELECT peer_user_id, direction, local_status, payload_json
         FROM agent_messages ORDER BY sequence_id DESC LIMIT 200",
    )
    .fetch_all(&pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            let direction: String = row.get("direction");
            let local_status: String = row.get("local_status");
            let message = serde_json::from_str::<dystil_protocol::agent_mailbox::AgentMessage>(
                &row.get::<String, _>("payload_json"),
            )
            .map_err(|error| error.to_string())?;
            Ok(to_view(&message, &direction, &local_status))
        })
        .collect()
}

#[tauri::command]
#[specta::specta]
pub async fn local_chat_list_sessions(
    state: State<'_, RecordingState>,
) -> Result<Vec<LocalChatSessionView>, String> {
    let rows = sqlx::query(
        "SELECT id, title, updated_at FROM local_chat_sessions
         ORDER BY updated_at DESC, id DESC LIMIT 200",
    )
    .fetch_all(&pool(&state).await?)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| LocalChatSessionView {
            id: row.get("id"),
            title: row.get("title"),
            updated_at: row.get("updated_at"),
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn local_chat_get_messages(
    session_id: String,
    state: State<'_, RecordingState>,
) -> Result<Vec<LocalChatMessageView>, String> {
    let rows = sqlx::query(
        "SELECT id, session_id, role, mode, question, answer, status,
                citations_json, provider, model, elapsed_ms,
                error_code, created_at
         FROM local_chat_messages WHERE session_id = ?1
         ORDER BY rowid ASC",
    )
    .bind(session_id)
    .fetch_all(&pool(&state).await?)
    .await
    .map_err(|error| error.to_string())?;
    Ok(rows.into_iter().map(local_chat_message_view).collect())
}

#[tauri::command]
#[specta::specta]
pub async fn local_chat_send(
    app: AppHandle,
    session_id: String,
    question: String,
    state: State<'_, RecordingState>,
) -> Result<LocalChatMessageView, String> {
    if session_id.is_empty()
        || session_id.len() > 128
        || question.trim().is_empty()
        || question.len() > 2_000
    {
        return Err("invalid local chat message".into());
    }
    let database = pool(&state).await?;
    let user_id = Uuid::new_v4().to_string();
    let assistant_id = Uuid::new_v4().to_string();
    let title = question.chars().take(96).collect::<String>();
    sqlx::query("INSERT OR IGNORE INTO local_chat_sessions (id, title) VALUES (?1, ?2)")
        .bind(&session_id)
        .bind(title)
        .execute(&database)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("INSERT INTO local_chat_messages (id, session_id, role, mode, question, status) VALUES (?1, ?2, 'user', 'local', ?3, 'complete')")
        .bind(&user_id).bind(&session_id).bind(&question).execute(&database).await.map_err(|error| error.to_string())?;

    // Retrieval is tool-driven. Do not guess context before the runtime has
    // chosen an overview or exact evidence query.
    sqlx::query("INSERT INTO local_chat_messages (id, session_id, role, mode, status) VALUES (?1, ?2, 'assistant', 'local', 'pending')")
        .bind(&assistant_id).bind(&session_id).execute(&database).await.map_err(|error| error.to_string())?;
    sqlx::query("UPDATE local_chat_sessions SET updated_at = datetime('now') WHERE id = ?1")
        .bind(&session_id)
        .execute(&database)
        .await
        .map_err(|error| error.to_string())?;

    let timezone = ai::local_timezone_offset();
    let search_end = chrono::Utc::now();
    let search_start = search_end - chrono::Duration::days(30);
    let provider_question =
        local_chat_question_with_history(&database, &session_id, &user_id, &question).await?;
    let runtime = match crate::ai_runtime::resolve(&app, &state, &database, &timezone).await {
        Ok(runtime) => runtime,
        Err(_) => {
            return complete_local_chat_error(
                &database,
                &assistant_id,
                "provider_not_ready",
                "Choose and connect an AI preset in Settings to answer local questions.",
            )
            .await
        }
    };
    let answer_provider = runtime.descriptor().provider_label.clone();
    let answer_model = runtime.descriptor().model.clone();
    let generated = runtime
        .answer(dystil_ai::AiAnswerRequest {
            requester_name: "you".into(),
            question: provider_question,
            search_start: search_start.to_rfc3339(),
            search_end: search_end.to_rfc3339(),
            timezone: timezone.clone(),
        })
        .await;
    let generated = match generated {
        Ok(value) => value,
        Err(error) if error.code == dystil_ai::AiRuntimeErrorCode::Timeout => {
            return complete_local_chat_error(
                &database,
                &assistant_id,
                "provider_timeout",
                "The local AI provider timed out.",
            )
            .await
        }
        Err(error) => {
            tracing::warn!(reason = %error, "configured AI provider returned an invalid local-chat answer");
            let (error_code, message) = match error.code {
                dystil_ai::AiRuntimeErrorCode::InvalidOutput => (
                    "provider_invalid_output",
                    "The AI runtime returned an answer Dystil could not validate.",
                ),
                dystil_ai::AiRuntimeErrorCode::Authentication => (
                    "provider_authentication",
                    "The AI preset needs authentication. Reconnect it in Settings.",
                ),
                dystil_ai::AiRuntimeErrorCode::NotReady => (
                    "provider_not_ready",
                    "The AI runtime is not ready. Check the active preset in Settings.",
                ),
                dystil_ai::AiRuntimeErrorCode::Transport => (
                    "provider_unreachable",
                    "Dystil could not reach the configured AI provider. Check that it is running, then retry.",
                ),
                _ => (
                    "provider_failed",
                    "The configured AI provider could not produce a valid answer.",
                ),
            };
            return complete_local_chat_error(&database, &assistant_id, error_code, message).await;
        }
    };
    let mut citations = Vec::new();
    let mut cited_evidence_ids = HashSet::new();
    for claim in &generated.answer.evidence {
        for evidence_id_text in &claim.evidence_ids {
            if !cited_evidence_ids.insert(evidence_id_text.clone()) {
                continue;
            }
            if let Ok(evidence_id) = evidence_id_text.parse::<dystil_retrieval::EvidenceId>() {
                if let Ok(evidence) = dystil_retrieval::RetrievalService::new(database.clone())
                    .get_source(&evidence_id, Some(500))
                    .await
                {
                    let label = evidence
                        .window_name
                        .clone()
                        .or(evidence.app_name.clone())
                        .unwrap_or_else(|| evidence.evidence_id.to_string());
                    citations.push(serde_json::json!({
                        "evidenceId": evidence.evidence_id.to_string(),
                        "deepLink": evidence.deep_link,
                        "label": label,
                        "localDate": ai::local_date_for_timestamp(&evidence.timestamp, &timezone)
                    }));
                }
            }
        }
    }
    sqlx::query("UPDATE local_chat_messages SET status = 'complete', answer = ?1, citations_json = ?2, provider = ?3, model = ?4, elapsed_ms = ?5 WHERE id = ?6")
        .bind(&generated.answer.answer).bind(serde_json::to_string(&citations).map_err(|error| error.to_string())?).bind(answer_provider).bind(answer_model).bind(generated.elapsed_ms as i64).bind(&assistant_id).execute(&database).await.map_err(|error| error.to_string())?;
    let row = sqlx::query("SELECT id, session_id, role, mode, question, answer, status, citations_json, provider, model, elapsed_ms, error_code, created_at FROM local_chat_messages WHERE id = ?1")
        .bind(assistant_id).fetch_one(&database).await.map_err(|error| error.to_string())?;
    Ok(local_chat_message_view(row))
}

async fn local_chat_question_with_history(
    database: &sqlx::SqlitePool,
    session_id: &str,
    current_user_message_id: &str,
    current_question: &str,
) -> Result<String, String> {
    let rows = sqlx::query(
        "SELECT role, question, answer FROM local_chat_messages
         WHERE session_id = ?1 AND id <> ?2 AND status = 'complete'
         ORDER BY rowid DESC LIMIT 6",
    )
    .bind(session_id)
    .bind(current_user_message_id)
    .fetch_all(database)
    .await
    .map_err(|error| error.to_string())?;
    let mut turns = rows
        .into_iter()
        .rev()
        .filter_map(|row| {
            let role: String = row.get("role");
            let text = if role == "user" {
                row.try_get::<Option<String>, _>("question").ok().flatten()
            } else {
                row.try_get::<Option<String>, _>("answer").ok().flatten()
            }?;
            (!text.trim().is_empty())
                .then(|| format!("{role}: {}", text.chars().take(1_200).collect::<String>()))
        })
        .collect::<Vec<_>>();
    // The current question was already saved above. Retain the explicit final
    // label so the provider cannot mistake a prior turn for the active ask.
    turns.push(format!("current user question: {current_question}"));
    Ok(turns.join("\n"))
}

async fn complete_local_chat_error(
    database: &sqlx::SqlitePool,
    message_id: &str,
    code: &str,
    answer: &str,
) -> Result<LocalChatMessageView, String> {
    sqlx::query("UPDATE local_chat_messages SET status = 'failed', answer = ?1, error_code = ?2 WHERE id = ?3")
        .bind(answer).bind(code).bind(message_id).execute(database).await.map_err(|error| error.to_string())?;
    let row = sqlx::query("SELECT id, session_id, role, mode, question, answer, status, citations_json, provider, model, elapsed_ms, error_code, created_at FROM local_chat_messages WHERE id = ?1")
        .bind(message_id).fetch_one(database).await.map_err(|error| error.to_string())?;
    Ok(local_chat_message_view(row))
}

fn local_chat_message_view(row: sqlx::sqlite::SqliteRow) -> LocalChatMessageView {
    LocalChatMessageView {
        id: row.get("id"),
        session_id: row.get("session_id"),
        role: row.get("role"),
        mode: row.get("mode"),
        question: row.get("question"),
        answer: row.get("answer"),
        status: row.get("status"),
        citations_json: row.get("citations_json"),
        provider: row.get("provider"),
        model: row.get("model"),
        elapsed_ms: row.get("elapsed_ms"),
        error_code: row.get("error_code"),
        created_at: row.get("created_at"),
    }
}

fn to_view(
    message: &dystil_protocol::agent_mailbox::AgentMessage,
    direction: &str,
    local_status: &str,
) -> AgentMessageView {
    let evidence = match &message.payload {
        AgentMessagePayload::Response(body) => body
            .evidence
            .iter()
            .map(|item| AgentEvidenceView {
                label: item.label.clone(),
                local_date: item.local_date.clone(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let text = match &message.payload {
        AgentMessagePayload::Request(body) => body.question.clone(),
        AgentMessagePayload::Status(body) => format!("{:?}", body.stage).to_ascii_lowercase(),
        AgentMessagePayload::Response(body) => body.answer.clone(),
        AgentMessagePayload::Error(body) => body.message.clone(),
    };
    AgentMessageView {
        message_id: message.message_id.clone(),
        conversation_id: message.conversation_id.clone(),
        peer_user_id: if direction == "outgoing" {
            message.recipient_user_id.clone()
        } else {
            message.sender_user_id.clone()
        },
        direction: direction.into(),
        kind: message.payload.kind().as_str().into(),
        local_status: local_status.into(),
        text,
        evidence,
        created_at: message.created_at.clone(),
    }
}
