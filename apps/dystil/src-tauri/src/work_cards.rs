use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::State;

use crate::recording::RecordingState;
use crate::work_card_worker::{
    configured_work_card_config, embed_text, generate_closed_work_cards, LocalWorkCardConfig,
    WorkCardGenerationReport,
};

#[derive(Debug, Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkCardView {
    pub window_id: String,
    pub start_time: String,
    pub end_time: String,
    pub close_reason: String,
    pub title: String,
    pub summary: String,
    pub applications: Vec<String>,
    pub artifacts: serde_json::Value,
    pub actions: serde_json::Value,
    pub last_observed_state: String,
    pub status: String,
    pub uncertainties: Vec<String>,
    pub model_id: String,
    pub source_hash: String,
    pub embedding_model_id: Option<String>,
    pub embedding_dimensions: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
}

impl From<dystil_storage::StoredWorkCard> for WorkCardView {
    fn from(value: dystil_storage::StoredWorkCard) -> Self {
        Self {
            window_id: value.window_id,
            start_time: value.start_time,
            end_time: value.end_time,
            close_reason: value.close_reason,
            title: value.title,
            summary: value.summary,
            applications: value.applications,
            artifacts: value.artifacts,
            actions: value.actions,
            last_observed_state: value.last_observed_state,
            status: value.status,
            uncertainties: value.uncertainties,
            model_id: value.model_id,
            source_hash: value.source_hash,
            embedding_model_id: value.embedding_model_id,
            embedding_dimensions: value.embedding_dimensions,
            created_at: value.created_at,
            updated_at: value.updated_at,
        }
    }
}

async fn pool(state: &RecordingState) -> Result<sqlx::SqlitePool, String> {
    state
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.db.pool.clone())
        .ok_or_else(|| "local capture database is not ready".to_string())
}

/// List the newest locally generated work cards.
#[tauri::command]
#[specta::specta]
pub async fn list_work_cards(
    limit: Option<u32>,
    state: State<'_, RecordingState>,
) -> Result<Vec<WorkCardView>, String> {
    let pool = pool(&state).await?;
    dystil_storage::list_work_cards(&pool, limit.unwrap_or(50))
        .await
        .map(|cards| cards.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

/// Search work-card titles, summaries, apps, artifacts, actions, and final state
/// locally through SQLite FTS5. Empty queries return the newest cards.
#[tauri::command]
#[specta::specta]
pub async fn search_work_cards(
    query: String,
    limit: Option<u32>,
    state: State<'_, RecordingState>,
) -> Result<Vec<WorkCardView>, String> {
    let pool = pool(&state).await?;
    if !query.trim().is_empty() {
        if let Some(config) = LocalWorkCardConfig::from_env() {
            if let Some(embedding_url) = config.embedding_url {
                let client = reqwest::Client::new();
                if let Ok(embedding) =
                    embed_text(&client, &embedding_url, &format!("query: {}", query.trim())).await
                {
                    return dystil_storage::hybrid_search_work_cards(
                        &pool,
                        &query,
                        &embedding,
                        limit.unwrap_or(30),
                    )
                    .await
                    .map(|cards| cards.into_iter().map(Into::into).collect())
                    .map_err(|error| error.to_string());
                }
            }
        }
    }
    dystil_storage::search_work_cards(&pool, &query, limit.unwrap_or(30))
        .await
        .map(|cards| cards.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

/// Delete one derived work card. Raw capture evidence is not affected.
#[tauri::command]
#[specta::specta]
pub async fn delete_work_card(
    window_id: String,
    state: State<'_, RecordingState>,
) -> Result<(), String> {
    let pool = pool(&state).await?;
    dystil_storage::delete_work_card(&pool, &window_id)
        .await
        .map_err(|error| error.to_string())
}

/// Generate closed activity windows using the active AI choice. Evidence is
/// sanitized before it reaches a connected provider.
#[tauri::command]
#[specta::specta]
pub async fn generate_work_cards_now(
    state: State<'_, RecordingState>,
) -> Result<WorkCardGenerationReport, String> {
    let pool = pool(&state).await?;
    let config = configured_work_card_config(&pool).await?.ok_or_else(|| {
        "connect an AI provider or enable experimental local processing first".to_string()
    })?;
    generate_closed_work_cards(&pool, &config).await
}
