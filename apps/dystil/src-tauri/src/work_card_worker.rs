use std::collections::HashSet;
use std::time::Instant;

use chrono::{DateTime, Duration, Utc};
use dystil_protocol::{SegmentEvidenceItem, SegmentEvidenceKind};
use dystil_work_cards::{
    build_evidence_windows_from_items, build_work_card_prompt, compact_window, sanitize_work_card,
    validate_work_card, work_card_json_schema, CompactionConfig, PromptConfig, WindowConfig,
    WorkCard,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone)]
pub enum WorkCardGeneratorConfig {
    Http(LocalWorkCardConfig),
    Managed {
        provider: dystil_ai::ProviderKind,
        model: String,
    },
}

impl WorkCardGeneratorConfig {
    fn model_id(&self) -> String {
        match self {
            Self::Http(config) => config.generator_model.clone(),
            Self::Managed { provider, model } => format!("{}:{}", provider.slug(), model),
        }
    }

    fn embedding_config(&self) -> (Option<String>, Option<String>) {
        match self {
            Self::Http(config) => (config.embedding_url.clone(), config.embedding_model.clone()),
            Self::Managed { .. } => LocalWorkCardConfig::from_env()
                .map(|config| (config.embedding_url, config.embedding_model))
                .unwrap_or((None, None)),
        }
    }

    fn max_windows(&self) -> usize {
        match self {
            Self::Http(config) => config.max_windows,
            Self::Managed { .. } => 4,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalWorkCardConfig {
    pub generator_url: String,
    pub generator_model: String,
    pub api_key: Option<String>,
    /// OpenAI-compatible APIs reject several llama.cpp-only parameters.
    pub openai_compatible: bool,
    pub use_max_completion_tokens: bool,
    pub embedding_url: Option<String>,
    pub embedding_model: Option<String>,
    pub max_windows: usize,
}

impl LocalWorkCardConfig {
    pub fn from_env() -> Option<Self> {
        let generator_url = std::env::var("DYSTIL_WORK_CARD_LLM_URL").ok()?;
        Some(Self {
            generator_url: generator_url.trim_end_matches('/').to_string(),
            generator_model: std::env::var("DYSTIL_WORK_CARD_LLM_MODEL")
                .unwrap_or_else(|_| "qwen3.5-2b-q4_k_m".to_string()),
            api_key: std::env::var("DYSTIL_WORK_CARD_API_KEY").ok(),
            openai_compatible: false,
            use_max_completion_tokens: false,
            embedding_url: std::env::var("DYSTIL_WORK_CARD_EMBEDDING_URL")
                .ok()
                .map(|value| value.trim_end_matches('/').to_string()),
            embedding_model: std::env::var("DYSTIL_WORK_CARD_EMBEDDING_MODEL").ok(),
            max_windows: std::env::var("DYSTIL_WORK_CARD_MAX_WINDOWS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(4),
        })
    }
}

/// Resolve the model for both manual and periodic generation. An active BYOK
/// profile takes precedence over the optional local llama.cpp setup, so one
/// user choice powers both Dystil AI surfaces.
pub async fn configured_work_card_config(
    pool: &SqlitePool,
) -> Result<Option<WorkCardGeneratorConfig>, String> {
    if let Some(profile) = crate::byok::active_profile(pool).await? {
        return Ok(Some(WorkCardGeneratorConfig::Http(LocalWorkCardConfig {
            generator_url: profile.endpoint,
            generator_model: profile.work_card_model,
            api_key: Some(profile.api_key),
            openai_compatible: true,
            use_max_completion_tokens: true,
            embedding_url: None,
            embedding_model: None,
            max_windows: 4,
        })));
    }
    if let Some(config) = LocalWorkCardConfig::from_env() {
        return Ok(Some(WorkCardGeneratorConfig::Http(config)));
    }
    let (provider, _) = crate::agent_mailbox::preferences(pool).await?;
    let provider = crate::ai::provider_kind(&provider)?;
    let runtime = crate::ai::provider_runtime(provider.clone())?;
    if !runtime
        .authenticated()
        .await
        .map_err(|error| error.to_string())?
    {
        return Ok(None);
    }
    // Work cards are background enrichment, so do not inherit the often more
    // capable interactive-chat model. Keep the provider-specific low-cost
    // model stable and explicit.
    let model = match provider {
        dystil_ai::ProviderKind::Codex => "gpt-5.6-luna",
        dystil_ai::ProviderKind::Claude => "haiku",
    }
    .to_string();
    Ok(Some(WorkCardGeneratorConfig::Managed { provider, model }))
}

pub fn background_generation_allowed() -> bool {
    if std::env::var("DYSTIL_WORK_CARD_ALLOW_BATTERY").as_deref() == Ok("1") {
        return true;
    }
    on_ac_power().unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn on_ac_power() -> Option<bool> {
    let entries = std::fs::read_dir("/sys/class/power_supply").ok()?;
    let mut saw_battery = false;
    for entry in entries.flatten() {
        let kind = std::fs::read_to_string(entry.path().join("type")).unwrap_or_default();
        if kind.trim() == "Mains" {
            if std::fs::read_to_string(entry.path().join("online"))
                .ok()
                .is_some_and(|value| value.trim() == "1")
            {
                return Some(true);
            }
        } else if kind.trim() == "Battery" {
            saw_battery = true;
            let status = std::fs::read_to_string(entry.path().join("status")).unwrap_or_default();
            if matches!(status.trim(), "Charging" | "Full" | "Not charging") {
                return Some(true);
            }
        }
    }
    saw_battery.then_some(false).or(Some(true))
}

#[cfg(target_os = "macos")]
fn on_ac_power() -> Option<bool> {
    std::process::Command::new("pmset")
        .args(["-g", "batt"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).contains("AC Power"))
}

#[cfg(target_os = "windows")]
fn on_ac_power() -> Option<bool> {
    // Background generation stays conservative until native Windows power
    // status is wired. A user-initiated generation pass still works.
    None
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn on_ac_power() -> Option<bool> {
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct WorkCardGenerationReport {
    pub candidate_windows: u32,
    pub generated_cards: u32,
    pub skipped_existing: u32,
    pub rejected_cards: u32,
    pub elapsed_ms: u64,
}

pub async fn generate_closed_work_cards(
    pool: &SqlitePool,
    config: &WorkCardGeneratorConfig,
) -> Result<WorkCardGenerationReport, String> {
    let started = Instant::now();
    let items = load_recent_evidence(pool).await?;
    let cutoff = Utc::now() - Duration::minutes(5);
    let windows = build_evidence_windows_from_items(
        "local",
        items,
        &WindowConfig {
            inactivity: Duration::minutes(5),
            max_duration: Duration::minutes(15),
        },
    )
    .into_iter()
    .filter(|window| window.close_reason != "end_of_input" || window.end_time <= cutoff)
    .collect::<Vec<_>>();
    let existing = existing_window_ids(pool).await?;
    let skipped_existing = windows
        .iter()
        .filter(|window| existing.contains(&window.window_id))
        .count();
    let candidates = windows
        .into_iter()
        .filter(|window| !existing.contains(&window.window_id))
        .take(config.max_windows())
        .collect::<Vec<_>>();
    let candidate_windows = candidates.len();
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|error| error.to_string())?;
    let mut generated_cards = 0;
    let mut rejected_cards = 0;

    for window in candidates {
        let (evidence, _) = compact_window(
            &window,
            &CompactionConfig {
                max_tokens: 1_800,
                ..CompactionConfig::default()
            },
        );
        let prompt = build_work_card_prompt(&window, &evidence, &PromptConfig::default());
        let schema = work_card_json_schema(&evidence);
        let mut card = match request_work_card(&client, config, &prompt, &schema, 1_200).await {
            Ok(card) => card,
            Err(_) => match request_work_card(&client, config, &prompt, &schema, 1_600).await {
                Ok(card) => card,
                Err(_) => {
                    rejected_cards += 1;
                    continue;
                }
            },
        };
        sanitize_work_card(&mut card, &evidence);
        let validation = validate_work_card(&card, &evidence);
        if !validation.valid {
            rejected_cards += 1;
            continue;
        }
        let searchable = searchable_card_text(&card);
        let (embedding_url, embedding_model) = config.embedding_config();
        let embedding = if let Some(url) = &embedding_url {
            Some(embed_text(&client, url, &searchable).await?)
        } else {
            None
        };
        let source_hash = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(
                serde_json::to_vec(&evidence).map_err(|error| error.to_string())?
            ))
        );
        let status = serde_json::to_value(&card.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let evidence_links = evidence
            .iter()
            .flat_map(|item| {
                item.source_ids
                    .iter()
                    .map(move |source_id| (source_id, &item.occurred_at))
            })
            .filter_map(|(source_id, occurred_at)| {
                let source_id = source_id.strip_prefix("local_").unwrap_or(source_id);
                let (source_type, source_row_id) = source_id.split_once('_')?;
                matches!(source_type, "frame" | "event")
                    .then(|| source_row_id.parse::<i64>().ok())
                    .flatten()
                    .map(|source_row_id| dystil_storage::WorkCardEvidenceLink {
                        source_type: source_type.to_string(),
                        source_row_id,
                        occurred_at: occurred_at.to_rfc3339(),
                    })
            })
            .collect();
        dystil_storage::upsert_work_card(
            pool,
            &dystil_storage::NewWorkCard {
                window_id: window.window_id,
                start_time: window.start_time.to_rfc3339(),
                end_time: window.end_time.to_rfc3339(),
                close_reason: window.close_reason,
                title: card.title.clone(),
                summary: card.summary.text.clone(),
                applications: card.applications.clone(),
                artifacts: serde_json::to_value(&card.artifacts)
                    .map_err(|error| error.to_string())?,
                actions: serde_json::to_value(&card.actions).map_err(|error| error.to_string())?,
                last_observed_state: card.last_observed_state.text.clone(),
                status,
                uncertainties: card.uncertainties.clone(),
                card_json: serde_json::to_value(&card).map_err(|error| error.to_string())?,
                model_id: config.model_id(),
                source_hash,
                embedding_model_id: embedding.as_ref().and(embedding_model),
                embedding,
                evidence: evidence_links,
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        generated_cards += 1;
    }

    Ok(WorkCardGenerationReport {
        candidate_windows: candidate_windows as u32,
        generated_cards,
        skipped_existing: skipped_existing as u32,
        rejected_cards,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

async fn request_work_card(
    client: &Client,
    config: &WorkCardGeneratorConfig,
    prompt: &str,
    schema: &serde_json::Value,
    max_tokens: u32,
) -> Result<WorkCard, String> {
    if let WorkCardGeneratorConfig::Managed { provider, model } = config {
        let runtime = crate::ai::provider_runtime(provider.clone())?;
        let model = (model != "default").then_some(model.as_str());
        let response = runtime
            .run_structured_json_with_model(
                prompt,
                schema,
                std::time::Duration::from_secs(180),
                model,
            )
            .await
            .map_err(|error| format!("managed work-card generation failed: {error}"))?;
        return serde_json::from_value(response)
            .map_err(|error| format!("managed provider returned invalid card JSON: {error}"));
    }
    let WorkCardGeneratorConfig::Http(config) = config else {
        unreachable!("managed configuration is returned above");
    };
    let mut payload = serde_json::json!({
        "model": config.generator_model,
        "messages": [{"role": "user", "content": prompt}],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "work_card",
                "strict": true,
                "schema": schema
            }
        }
    });
    if config.openai_compatible {
        // gpt-5.6-luna requires this for Chat Completions structured output.
        payload["reasoning_effort"] = serde_json::json!("none");
    } else {
        payload["temperature"] = serde_json::json!(0.0);
        payload["chat_template_kwargs"] = serde_json::json!({"enable_thinking": false});
    }
    let token_key = if config.use_max_completion_tokens {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    payload[token_key] = serde_json::json!(max_tokens);
    let request = client
        .post(format!("{}/v1/chat/completions", config.generator_url))
        .json(&payload);
    let request = if let Some(api_key) = &config.api_key {
        request.bearer_auth(api_key)
    } else {
        request
    };
    let response = request
        .send()
        .await
        .map_err(|error| format!("local generator request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("local generator rejected request: {error}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("invalid local generator response: {error}"))?;
    let content = response
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "local generator response omitted message content".to_string())?;
    serde_json::from_str(content)
        .map_err(|error| format!("local generator returned invalid card JSON: {error}"))
}

async fn load_recent_evidence(pool: &SqlitePool) -> Result<Vec<SegmentEvidenceItem>, String> {
    let since = (Utc::now() - Duration::hours(24)).to_rfc3339();
    let rows = sqlx::query(
        "SELECT source,id,timestamp,text_value,app_name,window_name,browser_url,document_path,event_type
         FROM (
           SELECT 'frame' AS source,id,timestamp,frame_text AS text_value,
                  app_name,window_name,browser_url,document_path,NULL AS event_type
           FROM frames
           WHERE timestamp >= ?1 AND frame_text IS NOT NULL AND trim(frame_text) <> ''
           UNION ALL
           SELECT 'event' AS source,id,timestamp,
                  trim(coalesce(text_content,'') || ' ' || coalesce(element_name,'') || ' ' ||
                       coalesce(element_value,'')) AS text_value,
                  app_name,window_title AS window_name,browser_url,NULL AS document_path,event_type
           FROM ui_events
           WHERE timestamp >= ?1 AND (
             trim(coalesce(text_content,'')) <> '' OR
             trim(coalesce(element_name,'')) <> '' OR
             trim(coalesce(element_value,'')) <> ''
           )
         )
         ORDER BY timestamp,id
         LIMIT 30000",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            let source: String = row.try_get("source").map_err(|error| error.to_string())?;
            let id: i64 = row.try_get("id").map_err(|error| error.to_string())?;
            let timestamp: String = row
                .try_get("timestamp")
                .map_err(|error| error.to_string())?;
            let occurred_at = DateTime::parse_from_rfc3339(&timestamp)
                .map(|value| value.with_timezone(&Utc))
                .or_else(|_| {
                    DateTime::parse_from_str(&timestamp, "%Y-%m-%d %H:%M:%S%.f %:z")
                        .map(|value| value.with_timezone(&Utc))
                })
                .map_err(|error| format!("invalid capture timestamp {timestamp}: {error}"))?;
            let raw_text: String = row
                .try_get("text_value")
                .map_err(|error| error.to_string())?;
            let text = dystil_redact::sanitize_text(&raw_text);
            let source_id = format!("{source}:{id}");
            let payload_hash = format!("sha256:{}", hex::encode(Sha256::digest(text.as_bytes())));
            let document_path: Option<String> = row
                .try_get("document_path")
                .map_err(|error| error.to_string())?;
            Ok(SegmentEvidenceItem {
                item_id: format!("local_{source}_{id}"),
                kind: if source == "frame" {
                    SegmentEvidenceKind::Screen
                } else {
                    SegmentEvidenceKind::Input
                },
                occurred_at,
                source_id,
                source_payload_hash: payload_hash,
                text,
                app_name: row.try_get("app_name").map_err(|error| error.to_string())?,
                window_name: row
                    .try_get("window_name")
                    .map_err(|error| error.to_string())?,
                browser_url: row
                    .try_get("browser_url")
                    .map_err(|error| error.to_string())?,
                metadata: document_path
                    .map(|value| serde_json::json!({"document_path": value}))
                    .unwrap_or(serde_json::Value::Null),
            })
        })
        .collect()
}

async fn existing_window_ids(pool: &SqlitePool) -> Result<HashSet<String>, String> {
    sqlx::query_scalar::<_, String>("SELECT window_id FROM work_cards")
        .fetch_all(pool)
        .await
        .map(|values| values.into_iter().collect())
        .map_err(|error| error.to_string())
}

fn searchable_card_text(card: &WorkCard) -> String {
    let mut values = vec![card.title.clone(), card.summary.text.clone()];
    values.extend(card.applications.clone());
    values.extend(card.artifacts.iter().map(|artifact| artifact.value.clone()));
    values.extend(card.actions.iter().map(|action| action.text.clone()));
    values.push(card.last_observed_state.text.clone());
    format!("document: {}", values.join("\n"))
}

pub async fn embed_text(client: &Client, url: &str, text: &str) -> Result<Vec<f32>, String> {
    let response = client
        .post(format!("{url}/v1/embeddings"))
        .json(&serde_json::json!({"input": text}))
        .send()
        .await
        .map_err(|error| format!("local embedding request failed: {error}"))?
        .error_for_status()
        .map_err(|error| format!("local embedder rejected request: {error}"))?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| format!("invalid local embedding response: {error}"))?;
    serde_json::from_value(
        response
            .pointer("/data/0/embedding")
            .cloned()
            .ok_or_else(|| "local embedder response omitted embedding".to_string())?,
    )
    .map_err(|error| format!("invalid embedding vector: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn generates_validates_and_persists_a_closed_local_window() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        dystil_storage::initialize_capture_schema(&pool)
            .await
            .unwrap();
        let occurred_at = (Utc::now() - Duration::minutes(10)).to_rfc3339();
        sqlx::query(
            "INSERT INTO frames(timestamp,device_name,snapshot_path,app_name,window_name,frame_text)
             VALUES (?1,'device','','Editor','query.sql','Reviewed a slow database query')",
        )
        .bind(occurred_at)
        .execute(&pool)
        .await
        .unwrap();

        let items = load_recent_evidence(&pool).await.unwrap();
        let windows = build_evidence_windows_from_items("local", items, &WindowConfig::default());
        let (evidence, _) = compact_window(
            &windows[0],
            &CompactionConfig {
                max_tokens: 1_800,
                ..Default::default()
            },
        );
        let evidence_id = evidence[0].evidence_id.clone();
        let card = serde_json::json!({
            "title": "Reviewed a slow database query",
            "summary": {"text": "Reviewed a slow query.", "evidence_ids": [evidence_id]},
            "applications": ["Editor"],
            "artifacts": [{"kind": "file", "value": "query.sql", "evidence_ids": [evidence_id]}],
            "actions": [{"text": "Reviewed a slow database query.", "evidence_ids": [evidence_id]}],
            "last_observed_state": {"text": "The query remained open.", "evidence_ids": [evidence_id]},
            "status": "in_progress",
            "uncertainties": []
        });
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "choices": [{"message": {"content": card.to_string()}}]
            })))
            .mount(&server)
            .await;
        let report = generate_closed_work_cards(
            &pool,
            &WorkCardGeneratorConfig::Http(LocalWorkCardConfig {
                generator_url: server.uri(),
                generator_model: "test-model".into(),
                api_key: None,
                openai_compatible: false,
                use_max_completion_tokens: false,
                embedding_url: None,
                embedding_model: None,
                max_windows: 4,
            }),
        )
        .await
        .unwrap();

        assert_eq!(report.generated_cards, 1);
        let stored = dystil_storage::search_work_cards(&pool, "database", 10)
            .await
            .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].title, "Reviewed a slow database query");
    }
}
