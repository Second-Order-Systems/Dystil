//! Minimal BYOK profile metadata and OS-keyring boundary.
//!
//! SQLite contains only routing metadata. API keys are never returned to the
//! frontend and are kept in the platform credential store.

use keyring::Entry;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use tauri::State;
use uuid::Uuid;

use crate::{ai, recording::RecordingState};

const KEYRING_SERVICE: &str = "com.dystil.app.byok";

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ByokProfileView {
    pub id: String,
    pub provider_kind: String,
    pub endpoint: String,
    pub chat_model: String,
    pub work_card_model: String,
    pub active: bool,
    pub credential_present: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct ActiveByokProfile {
    pub id: String,
    pub endpoint: String,
    pub chat_model: String,
    pub work_card_model: String,
    pub api_key: String,
}

async fn pool(state: &RecordingState) -> Result<sqlx::SqlitePool, String> {
    ai::capture_pool(state).await
}

fn normalize_endpoint(value: &str) -> Result<String, String> {
    let value = value.trim().trim_end_matches('/');
    if !(value.starts_with("https://")
        || value.starts_with("http://localhost")
        || value.starts_with("http://127.0.0.1"))
    {
        return Err("BYOK endpoint must use https (or localhost http)".into());
    }
    if value.contains('?') || value.contains('#') || value.len() > 500 {
        return Err("invalid BYOK endpoint".into());
    }
    Ok(value.to_string())
}

fn keyring_entry(profile_id: &str) -> Result<Entry, String> {
    Entry::new(KEYRING_SERVICE, profile_id).map_err(|error| error.to_string())
}

async fn credential_present(profile_id: String) -> bool {
    tokio::task::spawn_blocking(move || {
        keyring_entry(&profile_id)
            .and_then(|entry| entry.get_password().map_err(|error| error.to_string()))
            .is_ok()
    })
    .await
    .unwrap_or(false)
}

#[tauri::command]
#[specta::specta]
pub async fn byok_list_profiles(
    state: State<'_, RecordingState>,
) -> Result<Vec<ByokProfileView>, String> {
    let database = pool(&state).await?;
    let rows = sqlx::query("SELECT id, provider_kind, endpoint, chat_model, work_card_model, active FROM ai_provider_profiles ORDER BY active DESC, updated_at DESC")
        .fetch_all(&database).await.map_err(|error| error.to_string())?;
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.get("id");
        result.push(ByokProfileView {
            provider_kind: row.get("provider_kind"),
            endpoint: row.get("endpoint"),
            chat_model: row.get("chat_model"),
            work_card_model: row.get("work_card_model"),
            active: row.get::<i64, _>("active") != 0,
            credential_present: credential_present(id.clone()).await,
            id,
        });
    }
    Ok(result)
}

#[tauri::command]
#[specta::specta]
pub async fn byok_save_profile(
    endpoint: String,
    chat_model: String,
    work_card_model: String,
    api_key: String,
    state: State<'_, RecordingState>,
) -> Result<ByokProfileView, String> {
    let endpoint = normalize_endpoint(&endpoint)?;
    if chat_model.trim().is_empty()
        || work_card_model.trim().is_empty()
        || api_key.trim().is_empty()
    {
        return Err("BYOK endpoint, both models, and API key are required".into());
    }
    if chat_model.len() > 200 || work_card_model.len() > 200 || api_key.len() > 1_024 {
        return Err("invalid BYOK profile value".into());
    }
    let id = Uuid::new_v4().to_string();
    let key = api_key.trim().to_string();
    let key_id = id.clone();
    tokio::task::spawn_blocking(move || {
        keyring_entry(&key_id)?
            .set_password(&key)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    let database = pool(&state).await?;
    let mut tx = database.begin().await.map_err(|error| error.to_string())?;
    sqlx::query("UPDATE ai_provider_profiles SET active = 0 WHERE active = 1")
        .execute(&mut *tx)
        .await
        .map_err(|error| error.to_string())?;
    sqlx::query("INSERT INTO ai_provider_profiles(id, provider_kind, endpoint, chat_model, work_card_model, active) VALUES (?1, 'openai_compatible', ?2, ?3, ?4, 1)")
        .bind(&id).bind(&endpoint).bind(chat_model.trim()).bind(work_card_model.trim())
        .execute(&mut *tx).await.map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(ByokProfileView {
        id,
        provider_kind: "openai_compatible".into(),
        endpoint,
        chat_model: chat_model.trim().into(),
        work_card_model: work_card_model.trim().into(),
        active: true,
        credential_present: true,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn byok_delete_profile(
    profile_id: String,
    state: State<'_, RecordingState>,
) -> Result<(), String> {
    if profile_id.len() != 36 || Uuid::parse_str(&profile_id).is_err() {
        return Err("invalid BYOK profile ID".into());
    }
    let database = pool(&state).await?;
    let deleted = sqlx::query("DELETE FROM ai_provider_profiles WHERE id = ?1")
        .bind(&profile_id)
        .execute(&database)
        .await
        .map_err(|error| error.to_string())?
        .rows_affected();
    if deleted == 0 {
        return Err("BYOK profile not found".into());
    }
    tokio::task::spawn_blocking(move || {
        keyring_entry(&profile_id)?
            .delete_credential()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(())
}

pub(crate) async fn active_profile(
    pool: &sqlx::SqlitePool,
) -> Result<Option<ActiveByokProfile>, String> {
    let row = sqlx::query("SELECT id, endpoint, chat_model, work_card_model FROM ai_provider_profiles WHERE active = 1")
        .fetch_optional(pool).await.map_err(|error| error.to_string())?;
    let Some(row) = row else {
        return Ok(None);
    };
    let id: String = row.get("id");
    let key_id = id.clone();
    let api_key = tokio::task::spawn_blocking(move || {
        keyring_entry(&key_id)?
            .get_password()
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| error.to_string())??;
    Ok(Some(ActiveByokProfile {
        id,
        endpoint: row.get("endpoint"),
        chat_model: row.get("chat_model"),
        work_card_model: row.get("work_card_model"),
        api_key,
    }))
}

const MAX_RETRIEVAL_CALLS: usize = 6;

/// Run the same narrow retrieval surface exposed by the stdio sidecar, but
/// directly inside Dystil for OpenAI-compatible BYOK providers. The model has
/// no shell, SQL, filesystem, or network tool.
pub(crate) async fn answer_question(
    profile: &ActiveByokProfile,
    pool: &sqlx::SqlitePool,
    bundle: &dystil_ai::ContextBundle,
    question: &str,
) -> Result<dystil_ai::TeammateAnswerRun, String> {
    let started = std::time::Instant::now();
    let prompt = dystil_ai::teammate_answer_prompt(bundle, "you", question)
        .map_err(|error| error.to_string())?;
    let client = reqwest::Client::new();
    let mut messages = vec![json!({"role": "user", "content": prompt})];
    let mut calls_used = 0usize;
    // Final citations may name cards found during tool use as well as the
    // initial retrieval bundle. Activity records are never valid card IDs.
    let mut available_cards = bundle.cards.clone();

    let content = loop {
        let tools = (calls_used < MAX_RETRIEVAL_CALLS).then(retrieval_tools);
        let mut request = json!({
            "model": profile.chat_model,
            "messages": messages,
            "max_completion_tokens": 1600,
            // gpt-5.6-luna requires this for function tools on the Chat
            // Completions compatibility endpoint.
            "reasoning_effort": "none",
            "response_format": {"type": "json_schema", "json_schema": {"name": "dystil_answer", "strict": true, "schema": dystil_ai::teammate_answer_schema()}}
        });
        if let Some(tools) = tools {
            request["tools"] = tools;
            request["tool_choice"] = json!("auto");
        }
        let response = client
            .post(format!("{}/v1/chat/completions", profile.endpoint))
            .bearer_auth(&profile.api_key)
            .json(&request)
            .send()
            .await
            .map_err(|error| format!("BYOK request failed: {error}"))?
            .error_for_status()
            .map_err(|error| format!("BYOK request rejected: {error}"))?
            .json::<Value>()
            .await
            .map_err(|error| format!("invalid BYOK response: {error}"))?;
        let message = response
            .pointer("/choices/0/message")
            .cloned()
            .ok_or("BYOK response omitted a message")?;
        let tool_calls = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if tool_calls.is_empty() {
            break message
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or("BYOK response omitted message content")?;
        }
        messages.push(message);
        for call in tool_calls {
            let call_id = call
                .get("id")
                .and_then(Value::as_str)
                .ok_or("BYOK tool call omitted ID")?;
            let name = call
                .pointer("/function/name")
                .and_then(Value::as_str)
                .unwrap_or("");
            let arguments = call
                .pointer("/function/arguments")
                .and_then(Value::as_str)
                .unwrap_or("{}");
            let result = if calls_used >= MAX_RETRIEVAL_CALLS {
                json!({"error": "retrieval call budget exhausted; answer from the evidence already available"})
            } else {
                calls_used += 1;
                match serde_json::from_str::<Value>(arguments) {
                    Ok(arguments) => match run_retrieval_tool(pool, name, &arguments).await {
                        Ok(result) => {
                            if name == "dystil_search_work_cards" {
                                if let Ok(cards) = serde_json::from_value::<
                                    Vec<dystil_ai::ContextCard>,
                                >(result.clone())
                                {
                                    for card in cards {
                                        if !available_cards.iter().any(|known| known.id == card.id)
                                        {
                                            available_cards.push(card);
                                        }
                                    }
                                }
                            }
                            result
                        }
                        Err(error) => json!({"error": error}),
                    },
                    Err(_) => json!({"error": "tool arguments must be valid JSON"}),
                }
            };
            messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": serde_json::to_string(&result).map_err(|error| error.to_string())?}));
        }
    };
    let mut answer = serde_json::from_str::<dystil_ai::TeammateAnswer>(&content)
        .map_err(|error| format!("invalid BYOK structured answer: {error}"))?;
    let known_card_ids = available_cards
        .iter()
        .map(|card| card.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    let dropped_activity_citations = answer.evidence.iter().any(|claim| {
        claim
            .card_ids
            .iter()
            .any(|id| !known_card_ids.contains(id.as_str()))
    });
    answer.evidence.retain(|claim| {
        claim
            .card_ids
            .iter()
            .all(|id| known_card_ids.contains(id.as_str()))
    });
    if dropped_activity_citations && answer.uncertainties.len() < 10 {
        answer.uncertainties.push(
            "Some detail was supported by sanitized activity rather than a work-card citation."
                .into(),
        );
    }
    let validation_bundle = dystil_ai::ContextBundle {
        cards: available_cards,
        ..bundle.clone()
    };
    dystil_ai::validate_teammate_answer(&validation_bundle, &answer)
        .map_err(|error| error.to_string())?;
    Ok(dystil_ai::TeammateAnswerRun {
        provider: dystil_ai::ProviderKind::Codex,
        runtime_version: Some(format!("byok:{}", profile.id)),
        elapsed_ms: started.elapsed().as_millis() as u64,
        answer,
    })
}

fn retrieval_tools() -> Value {
    json!([
        {"type": "function", "function": {"name": "dystil_search_work_cards", "description": "Search derived Dystil work cards.", "strict": true, "parameters": {"type": "object", "additionalProperties": false, "required": ["query"], "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 20}}}}},
        {"type": "function", "function": {"name": "dystil_get_work_card_evidence", "description": "Get sanitized activity evidence linked to a work card.", "strict": true, "parameters": {"type": "object", "additionalProperties": false, "required": ["card_id"], "properties": {"card_id": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 40}}}}},
        {"type": "function", "function": {"name": "dystil_search_activity", "description": "Full-text search sanitized local activity when cards are insufficient.", "strict": true, "parameters": {"type": "object", "additionalProperties": false, "required": ["query"], "properties": {"query": {"type": "string"}, "limit": {"type": "integer", "minimum": 1, "maximum": 20}}}}},
        {"type": "function", "function": {"name": "dystil_get_activity_context", "description": "Get a bounded time window around an activity result.", "strict": true, "parameters": {"type": "object", "additionalProperties": false, "required": ["source_id"], "properties": {"source_id": {"type": "string"}, "before_seconds": {"type": "integer", "minimum": 1, "maximum": 3600}, "after_seconds": {"type": "integer", "minimum": 1, "maximum": 3600}, "limit": {"type": "integer", "minimum": 1, "maximum": 50}}}}}
    ])
}

async fn run_retrieval_tool(
    pool: &sqlx::SqlitePool,
    name: &str,
    arguments: &Value,
) -> Result<Value, String> {
    let text = |key: &str| {
        arguments
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| format!("{key} is required"))
    };
    let number = |key: &str, default: u32| {
        arguments
            .get(key)
            .and_then(Value::as_u64)
            .map(|value| value.min(u32::MAX as u64) as u32)
            .unwrap_or(default)
    };
    match name {
        "dystil_search_work_cards" => {
            let cards =
                dystil_storage::search_work_cards(pool, text("query")?, number("limit", 12))
                    .await
                    .map_err(|error| error.to_string())?;
            Ok(json!(cards
                .iter()
                .map(dystil_ai::ContextCard::from)
                .collect::<Vec<_>>()))
        }
        "dystil_get_work_card_evidence" => Ok(json!(dystil_storage::get_work_card_evidence(
            pool,
            text("card_id")?,
            number("limit", 30)
        )
        .await
        .map_err(|error| error.to_string())?)),
        "dystil_search_activity" => Ok(json!(dystil_storage::search_activity(
            pool,
            text("query")?,
            number("limit", 12)
        )
        .await
        .map_err(|error| error.to_string())?)),
        "dystil_get_activity_context" => Ok(json!(dystil_storage::get_activity_context(
            pool,
            text("source_id")?,
            number("before_seconds", 120),
            number("after_seconds", 120),
            number("limit", 30)
        )
        .await
        .map_err(|error| error.to_string())?)),
        _ => Err("unknown Dystil retrieval tool".into()),
    }
}
