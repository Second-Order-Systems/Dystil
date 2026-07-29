use dystil_protocol::agent_mailbox::AgentMessagePayload;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::HashSet;
use tauri::{AppHandle, Emitter, State};
use uuid::Uuid;

use crate::{agent_mailbox, ai, recording::RecordingState};

const MAX_LOCAL_CHAT_CONTEXT_CARDS: usize = 36;

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
pub struct AgentPreferencesView {
    pub provider: String,
    pub model: String,
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
    pub selected_cards_json: Option<String>,
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
pub async fn agent_get_preferences(
    state: State<'_, RecordingState>,
) -> Result<AgentPreferencesView, String> {
    let (provider, model) = agent_mailbox::preferences(&pool(&state).await?).await?;
    Ok(AgentPreferencesView { provider, model })
}

#[tauri::command]
#[specta::specta]
pub async fn agent_set_preferences(
    provider: String,
    model: String,
    state: State<'_, RecordingState>,
) -> Result<AgentPreferencesView, String> {
    let provider_kind = ai::provider_kind(&provider)?;
    let model = model.trim();
    if model.is_empty()
        || model.len() > 80
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err("invalid provider model identifier".into());
    }
    let pool = pool(&state).await?;
    agent_mailbox::set_preferences(&pool, provider_kind.slug(), model).await?;
    Ok(AgentPreferencesView {
        provider: provider_kind.slug().into(),
        model: model.into(),
    })
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
                selected_cards_json, citations_json, provider, model, elapsed_ms,
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

    let cards = dystil_storage::search_work_cards(&database, &question, 12)
        .await
        .map_err(|error| error.to_string())?;
    let current_cards = cards
        .iter()
        .map(dystil_ai::ContextCard::from)
        .collect::<Vec<_>>();
    let context_cards = local_chat_context_cards(&database, &session_id, current_cards).await?;
    let snapshot = serde_json::to_string(&context_cards).map_err(|error| error.to_string())?;
    sqlx::query("INSERT INTO local_chat_messages (id, session_id, role, mode, status, selected_cards_json) VALUES (?1, ?2, 'assistant', 'local', 'pending', ?3)")
        .bind(&assistant_id).bind(&session_id).bind(snapshot).execute(&database).await.map_err(|error| error.to_string())?;
    sqlx::query("UPDATE local_chat_sessions SET updated_at = datetime('now') WHERE id = ?1")
        .bind(&session_id)
        .execute(&database)
        .await
        .map_err(|error| error.to_string())?;

    let timezone = ai::local_timezone_offset();
    let bundle = dystil_ai::ContextBundle {
        schema_version: dystil_ai::CONTEXT_SCHEMA_VERSION.into(),
        task: "answer_local_question".into(),
        timezone: timezone.clone(),
        range: dystil_ai::ContextRange {
            start: context_cards
                .iter()
                .filter(|card| !card.start.is_empty())
                .map(|card| card.start.clone())
                .min()
                .unwrap_or_default(),
            end: context_cards
                .iter()
                .filter(|card| !card.end.is_empty())
                .map(|card| card.end.clone())
                .max()
                .unwrap_or_default(),
        },
        coverage: dystil_ai::ContextCoverage {
            card_count: context_cards.len(),
            first_observation: context_cards
                .iter()
                .filter(|card| !card.start.is_empty())
                .map(|card| card.start.clone())
                .min(),
            last_observation: context_cards
                .iter()
                .filter(|card| !card.end.is_empty())
                .map(|card| card.end.clone())
                .max(),
            truncated: context_cards.len() == MAX_LOCAL_CHAT_CONTEXT_CARDS,
        },
        cards: context_cards,
    };
    let provider_question =
        local_chat_question_with_history(&database, &session_id, &user_id, &question).await?;
    let (provider, model) = agent_mailbox::preferences(&database).await?;
    let (generated, answer_provider, answer_model) = if let Some(profile) =
        crate::byok::active_profile(&database).await?
    {
        let answer_model = profile.chat_model.clone();
        (
            crate::byok::answer_question(&profile, &database, &bundle, &provider_question).await,
            "byok".to_string(),
            answer_model,
        )
    } else {
        let runtime = match ai::provider_kind(&provider).and_then(ai::provider_runtime) {
            Ok(runtime) if runtime.authenticated().await.unwrap_or(false) => runtime,
            _ => return complete_local_chat_error(&database, &assistant_id, "provider_not_ready", "Connect your AI provider in Settings or add a BYOK profile to answer local questions.").await,
        };
        let runtime = match ai::internal_mcp_server(&app, &state, &timezone).await {
            Ok(mcp) => runtime.with_mcp_server(mcp),
            Err(error) => {
                return complete_local_chat_error(
                    &database,
                    &assistant_id,
                    "mcp_not_ready",
                    &format!("Dystil's local retrieval sidecar is unavailable: {error}"),
                )
                .await
            }
        };
        (
            runtime
                .run_teammate_answer_with_model(
                    &bundle,
                    "you",
                    &provider_question,
                    (model != "default").then_some(model.as_str()),
                )
                .await
                .map_err(|error| error.to_string()),
            provider,
            model,
        )
    };
    let generated = match generated {
        Ok(value) => value,
        Err(error) if error.contains("timed out") => {
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
            return complete_local_chat_error(
                &database,
                &assistant_id,
                "provider_invalid_output",
                "The configured AI provider could not produce a valid answer.",
            )
            .await;
        }
    };
    let mut citations = Vec::new();
    let mut cited_card_ids = HashSet::new();
    let mut persisted_context_cards = bundle.cards.clone();
    for claim in &generated.answer.evidence {
        for card_id in &claim.card_ids {
            if !cited_card_ids.insert(card_id.clone()) {
                continue;
            }
            if let Some(card) = bundle.cards.iter().find(|card| &card.id == card_id) {
                citations.push(serde_json::json!({
                    "cardId": card.id,
                    "label": card.title,
                    "localDate": ai::local_date_for_timestamp(&card.start, &timezone)
                }));
            } else if let Some(card) = dystil_storage::get_work_card(&database, card_id)
                .await
                .map_err(|error| error.to_string())?
            {
                let context_card = dystil_ai::ContextCard::from(&card);
                citations.push(serde_json::json!({
                    "cardId": &card.window_id,
                    "label": &card.title,
                    "localDate": ai::local_date_for_timestamp(&card.start_time, &timezone)
                }));
                if persisted_context_cards.len() < MAX_LOCAL_CHAT_CONTEXT_CARDS {
                    persisted_context_cards.push(context_card);
                }
            }
        }
    }
    let persisted_snapshot =
        serde_json::to_string(&persisted_context_cards).map_err(|error| error.to_string())?;
    sqlx::query("UPDATE local_chat_messages SET status = 'complete', answer = ?1, citations_json = ?2, provider = ?3, model = ?4, elapsed_ms = ?5, selected_cards_json = ?6 WHERE id = ?7")
        .bind(&generated.answer.answer).bind(serde_json::to_string(&citations).map_err(|error| error.to_string())?).bind(answer_provider).bind(answer_model).bind(generated.elapsed_ms as i64).bind(persisted_snapshot).bind(&assistant_id).execute(&database).await.map_err(|error| error.to_string())?;
    let row = sqlx::query("SELECT id, session_id, role, mode, question, answer, status, selected_cards_json, citations_json, provider, model, elapsed_ms, error_code, created_at FROM local_chat_messages WHERE id = ?1")
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

async fn local_chat_context_cards(
    database: &sqlx::SqlitePool,
    session_id: &str,
    current_cards: Vec<dystil_ai::ContextCard>,
) -> Result<Vec<dystil_ai::ContextCard>, String> {
    let rows = sqlx::query(
        "SELECT selected_cards_json FROM local_chat_messages
         WHERE session_id = ?1 AND role = 'assistant' AND selected_cards_json IS NOT NULL
         ORDER BY rowid DESC LIMIT 6",
    )
    .bind(session_id)
    .fetch_all(database)
    .await
    .map_err(|error| error.to_string())?;

    let mut cards = Vec::new();
    let mut known = HashSet::new();
    for card in current_cards {
        if known.insert(card.id.clone()) {
            cards.push(card);
        }
    }
    for row in rows {
        let Some(snapshot) = row
            .try_get::<Option<String>, _>("selected_cards_json")
            .ok()
            .flatten()
        else {
            continue;
        };
        let Ok(previous_cards) = serde_json::from_str::<Vec<dystil_ai::ContextCard>>(&snapshot)
        else {
            continue;
        };
        for card in previous_cards {
            if cards.len() == MAX_LOCAL_CHAT_CONTEXT_CARDS {
                return Ok(cards);
            }
            if known.insert(card.id.clone()) {
                cards.push(card);
            }
        }
    }
    Ok(cards)
}

async fn complete_local_chat_error(
    database: &sqlx::SqlitePool,
    message_id: &str,
    code: &str,
    answer: &str,
) -> Result<LocalChatMessageView, String> {
    sqlx::query("UPDATE local_chat_messages SET status = 'failed', answer = ?1, error_code = ?2 WHERE id = ?3")
        .bind(answer).bind(code).bind(message_id).execute(database).await.map_err(|error| error.to_string())?;
    let row = sqlx::query("SELECT id, session_id, role, mode, question, answer, status, selected_cards_json, citations_json, provider, model, elapsed_ms, error_code, created_at FROM local_chat_messages WHERE id = ?1")
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
        selected_cards_json: row.get("selected_cards_json"),
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
