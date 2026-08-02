use std::convert::Infallible;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use tokio_stream::wrappers::ReceiverStream;
use work_insights_db::ai_gateway::{self as db, AiKeyRecord, NewAiUsage};

use crate::AppState;

#[derive(Debug, Clone)]
pub(crate) struct AiGatewayConfig {
    pub upstream_base_url: String,
    pub openai_api_key: String,
    pub models: Vec<AiModel>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct AiModel {
    pub id: String,
    pub input_microusd_per_million_tokens: i64,
    pub cached_input_microusd_per_million_tokens: i64,
    pub cache_write_microusd_per_million_tokens: i64,
    pub output_microusd_per_million_tokens: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct TokenUsage {
    input_tokens: i64,
    cached_input_tokens: i64,
    cache_write_tokens: i64,
    output_tokens: i64,
}

impl AiGatewayConfig {
    pub(crate) fn from_env() -> anyhow::Result<Option<Self>> {
        let Some(openai_api_key) = std::env::var("DYSTIL_OPENAI_API_KEY").ok() else {
            return Ok(None);
        };
        if openai_api_key.trim().is_empty() {
            anyhow::bail!("DYSTIL_OPENAI_API_KEY must not be empty");
        }
        let models = default_models();
        validate_models(&models)?;
        Ok(Some(Self {
            upstream_base_url: std::env::var("DYSTIL_OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_string())
                .trim_end_matches('/')
                .to_string(),
            openai_api_key,
            models,
        }))
    }

    fn model(&self, id: &str) -> Option<&AiModel> {
        self.models.iter().find(|model| model.id == id)
    }
}

// Standard short-context prices per 1M tokens from
// https://developers.openai.com/api/docs/pricing,
// checked 2026-08-02. Dystil's Pi custom-provider context window is 128K,
// below the 272K threshold where these models switch to long-context rates.
fn default_models() -> Vec<AiModel> {
    vec![
        AiModel {
            id: "gpt-5.6-sol".to_string(),
            input_microusd_per_million_tokens: 5_000_000,
            cached_input_microusd_per_million_tokens: 500_000,
            cache_write_microusd_per_million_tokens: 6_250_000,
            output_microusd_per_million_tokens: 30_000_000,
        },
        AiModel {
            id: "gpt-5.6-terra".to_string(),
            input_microusd_per_million_tokens: 2_000_000,
            cached_input_microusd_per_million_tokens: 200_000,
            cache_write_microusd_per_million_tokens: 2_500_000,
            output_microusd_per_million_tokens: 12_000_000,
        },
        AiModel {
            id: "gpt-5.6-luna".to_string(),
            input_microusd_per_million_tokens: 200_000,
            cached_input_microusd_per_million_tokens: 20_000,
            cache_write_microusd_per_million_tokens: 250_000,
            output_microusd_per_million_tokens: 1_200_000,
        },
    ]
}

fn validate_models(models: &[AiModel]) -> anyhow::Result<()> {
    if models.is_empty() {
        anyhow::bail!("Dystil AI model catalog must contain at least one model");
    }
    let mut ids = std::collections::HashSet::new();
    for model in models {
        if model.id.trim().is_empty() || model.id.len() > 200 || !ids.insert(model.id.as_str()) {
            anyhow::bail!("Dystil AI model catalog contains an empty or duplicate model id");
        }
        if model.input_microusd_per_million_tokens < 0
            || model.cached_input_microusd_per_million_tokens < 0
            || model.cache_write_microusd_per_million_tokens < 0
            || model.output_microusd_per_million_tokens < 0
        {
            anyhow::bail!("Dystil AI model prices must be non-negative integers");
        }
    }
    Ok(())
}

pub(crate) async fn get_models(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(config) = state.config.ai_gateway.as_ref() else {
        return openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_not_configured",
            "Dystil AI is not configured",
        );
    };
    if let Err(response) = authenticate(&state, &headers).await {
        return response;
    }
    json_response(
        StatusCode::OK,
        json!({
            "object": "list",
            "data": config.models.iter().map(|model| json!({
                "id": model.id,
                "object": "model",
                "created": 0,
                "owned_by": "dystil"
            })).collect::<Vec<_>>()
        }),
    )
}

pub(crate) async fn post_chat_completions(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(config) = state.config.ai_gateway.clone() else {
        return openai_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "gateway_not_configured",
            "Dystil AI is not configured",
        );
    };
    let key = match authenticate(&state, &headers).await {
        Ok(key) => key,
        Err(response) => return response,
    };
    if key.spent_microusd >= key.spend_limit_microusd {
        return openai_error(
            StatusCode::TOO_MANY_REQUESTS,
            "insufficient_quota",
            "This Dystil AI key has reached its spend limit",
        );
    }

    let mut request: Value = match serde_json::from_slice(&body) {
        Ok(Value::Object(object)) => Value::Object(object),
        _ => {
            return openai_error(
                StatusCode::BAD_REQUEST,
                "invalid_request_error",
                "Request body must be a JSON object",
            )
        }
    };
    let Some(model_id) = request
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "A model is required",
        );
    };
    let Some(model) = config.model(&model_id).cloned() else {
        return openai_error(
            StatusCode::BAD_REQUEST,
            "model_not_found",
            "The requested model is not available through this gateway",
        );
    };
    let object = request.as_object_mut().expect("validated object");
    object.remove("max_completion_tokens");
    object.remove("max_tokens");
    object.insert(
        "service_tier".to_string(),
        Value::String("default".to_string()),
    );
    let streaming = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if streaming {
        let object = request.as_object_mut().expect("validated object");
        let stream_options = object.entry("stream_options").or_insert_with(|| json!({}));
        if !stream_options.is_object() {
            *stream_options = json!({});
        }
        stream_options
            .as_object_mut()
            .expect("stream_options object")
            .insert("include_usage".to_string(), Value::Bool(true));
    }

    let upstream = match state
        .http
        .post(format!("{}/chat/completions", config.upstream_base_url))
        .bearer_auth(&config.openai_api_key)
        .json(&request)
        .send()
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::warn!(error = %error, "Dystil AI upstream request failed");
            return openai_error(
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                "The AI provider could not be reached",
            );
        }
    };

    let status = upstream.status();
    let content_type = upstream.headers().get(header::CONTENT_TYPE).cloned();
    let openai_request_id = upstream
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    if !status.is_success() {
        return buffered_upstream_response(upstream, status, content_type).await;
    }

    if !streaming {
        let bytes = match upstream.bytes().await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(error = %error, "Dystil AI upstream response failed");
                return openai_error(
                    StatusCode::BAD_GATEWAY,
                    "upstream_error",
                    "The AI provider response was interrupted",
                );
            }
        };
        if let Some(usage) = parse_usage_json(&bytes) {
            persist_usage(&state, &key, &model, openai_request_id.as_deref(), usage).await;
        } else {
            tracing::warn!(model = %model.id, "Dystil AI response omitted token usage");
        }
        return response_with_body(status, content_type, Body::from(bytes));
    }

    let (sender, receiver) = tokio::sync::mpsc::channel::<Result<Bytes, Infallible>>(16);
    let state_for_usage = state.clone();
    tokio::spawn(async move {
        let mut stream = upstream.bytes_stream();
        let mut line_buffer = Vec::new();
        let mut usage = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => {
                    observe_sse_usage(&mut line_buffer, &chunk, &mut usage);
                    let _ = sender.send(Ok(chunk)).await;
                }
                Err(error) => {
                    tracing::warn!(error = %error, model = %model.id, "Dystil AI stream was interrupted");
                    break;
                }
            }
        }
        if let Some(usage) = usage {
            persist_usage(
                &state_for_usage,
                &key,
                &model,
                openai_request_id.as_deref(),
                usage,
            )
            .await;
        } else {
            tracing::warn!(model = %model.id, "Dystil AI stream omitted token usage");
        }
    });

    response_with_body(
        status,
        content_type.or_else(|| Some(header::HeaderValue::from_static("text/event-stream"))),
        Body::from_stream(ReceiverStream::new(receiver)),
    )
}

async fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<AiKeyRecord, Response> {
    let raw = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| value.starts_with("dst_live_"))
        .ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "Invalid Dystil AI key",
            )
        })?;
    db::resolve_active_ai_key(&state.pool, raw)
        .await
        .map_err(|error| {
            tracing::error!(error = %error, "Dystil AI key lookup failed");
            openai_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "gateway_unavailable",
                "Dystil AI is temporarily unavailable",
            )
        })?
        .ok_or_else(|| {
            openai_error(
                StatusCode::UNAUTHORIZED,
                "invalid_api_key",
                "Invalid Dystil AI key",
            )
        })
}

async fn persist_usage(
    state: &AppState,
    key: &AiKeyRecord,
    model: &AiModel,
    openai_request_id: Option<&str>,
    usage: TokenUsage,
) {
    let cost_microusd = calculate_cost_microusd(model, &usage);
    if let Err(error) = db::record_ai_usage(
        &state.pool,
        NewAiUsage {
            key_id: key.id,
            openai_request_id,
            model: &model.id,
            input_tokens: usage.input_tokens,
            cached_input_tokens: usage.cached_input_tokens,
            cache_write_tokens: usage.cache_write_tokens,
            output_tokens: usage.output_tokens,
            cost_microusd,
        },
    )
    .await
    {
        tracing::error!(error = %error, key_prefix = %key.key_prefix, "Dystil AI usage persistence failed");
    }
}

fn calculate_cost_microusd(model: &AiModel, usage: &TokenUsage) -> i64 {
    let cached = usage.cached_input_tokens.clamp(0, usage.input_tokens);
    let cache_write = usage
        .cache_write_tokens
        .clamp(0, usage.input_tokens.saturating_sub(cached));
    let uncached = usage
        .input_tokens
        .saturating_sub(cached)
        .saturating_sub(cache_write);
    let numerator = i128::from(uncached) * i128::from(model.input_microusd_per_million_tokens)
        + i128::from(cached) * i128::from(model.cached_input_microusd_per_million_tokens)
        + i128::from(cache_write) * i128::from(model.cache_write_microusd_per_million_tokens)
        + i128::from(usage.output_tokens) * i128::from(model.output_microusd_per_million_tokens);
    let rounded_up = numerator.saturating_add(999_999) / 1_000_000;
    rounded_up.clamp(0, i128::from(i64::MAX)) as i64
}

fn parse_usage_json(bytes: &[u8]) -> Option<TokenUsage> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
    usage_from_value(&value)
}

fn usage_from_value(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(TokenUsage {
        input_tokens: usage.get("prompt_tokens")?.as_i64()?,
        cached_input_tokens: usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        cache_write_tokens: usage
            .pointer("/prompt_tokens_details/cache_write_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        output_tokens: usage.get("completion_tokens")?.as_i64()?,
    })
}

fn observe_sse_usage(buffer: &mut Vec<u8>, chunk: &[u8], usage: &mut Option<TokenUsage>) {
    buffer.extend_from_slice(chunk);
    while let Some(newline) = buffer.iter().position(|byte| *byte == b'\n') {
        let mut line = buffer.drain(..=newline).collect::<Vec<_>>();
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        let Some(data) = line.strip_prefix(b"data:") else {
            continue;
        };
        let data = data.strip_prefix(b" ").unwrap_or(data);
        if data == b"[DONE]" {
            continue;
        }
        if let Ok(value) = serde_json::from_slice::<Value>(data) {
            if let Some(found) = usage_from_value(&value) {
                *usage = Some(found);
            }
        }
    }
}

async fn buffered_upstream_response(
    upstream: reqwest::Response,
    status: StatusCode,
    content_type: Option<header::HeaderValue>,
) -> Response {
    match upstream.bytes().await {
        Ok(bytes) => response_with_body(status, content_type, Body::from(bytes)),
        Err(_) => openai_error(
            StatusCode::BAD_GATEWAY,
            "upstream_error",
            "The AI provider response was interrupted",
        ),
    }
}

fn response_with_body(
    status: StatusCode,
    content_type: Option<header::HeaderValue>,
    body: Body,
) -> Response {
    let mut response = Response::builder().status(status);
    if let Some(content_type) = content_type {
        response = response.header(header::CONTENT_TYPE, content_type);
    }
    response.body(body).expect("valid gateway response")
}

fn json_response(status: StatusCode, value: Value) -> Response {
    response_with_body(
        status,
        Some(header::HeaderValue::from_static("application/json")),
        Body::from(value.to_string()),
    )
}

fn openai_error(status: StatusCode, code: &str, message: &str) -> Response {
    json_response(
        status,
        json!({
            "error": {
                "message": message,
                "type": code,
                "param": null,
                "code": code
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::routing::post;
    use axum::Router;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    use crate::{Config, MemoryServiceConfig, StorageConfig};

    fn model() -> AiModel {
        AiModel {
            id: "test-model".to_string(),
            input_microusd_per_million_tokens: 1_000_000,
            cached_input_microusd_per_million_tokens: 100_000,
            cache_write_microusd_per_million_tokens: 1_250_000,
            output_microusd_per_million_tokens: 2_000_000,
        }
    }

    #[test]
    fn cost_uses_cached_and_uncached_rates_and_rounds_up() {
        let cost = calculate_cost_microusd(
            &model(),
            &TokenUsage {
                input_tokens: 1_000,
                cached_input_tokens: 400,
                cache_write_tokens: 100,
                output_tokens: 500,
            },
        );
        assert_eq!(cost, 1_665);
    }

    #[test]
    fn streaming_usage_can_cross_chunk_boundaries() {
        let mut buffer = Vec::new();
        let mut usage = None;
        observe_sse_usage(&mut buffer, b"data: {\"choices\":[],\"us", &mut usage);
        assert!(usage.is_none());
        observe_sse_usage(
            &mut buffer,
            b"age\":{\"prompt_tokens\":12,\"completion_tokens\":3,\"prompt_tokens_details\":{\"cached_tokens\":4,\"cache_write_tokens\":2}}}\n\ndata: [DONE]\n\n",
            &mut usage,
        );
        assert_eq!(
            usage,
            Some(TokenUsage {
                input_tokens: 12,
                cached_input_tokens: 4,
                cache_write_tokens: 2,
                output_tokens: 3,
            })
        );
    }

    #[test]
    fn catalog_rejects_duplicate_models() {
        assert!(validate_models(&[model(), model()]).is_err());
    }

    #[test]
    fn built_in_catalog_contains_current_chat_models_and_prices() {
        let models = default_models();
        validate_models(&models).unwrap();
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"]
        );
        assert_eq!(models[0].output_microusd_per_million_tokens, 30_000_000);
        assert_eq!(models[1].output_microusd_per_million_tokens, 12_000_000);
        assert_eq!(models[2].output_microusd_per_million_tokens, 1_200_000);
    }

    #[derive(Clone, Default)]
    struct UpstreamState {
        requests: Arc<Mutex<Vec<(String, Value)>>>,
    }

    #[tokio::test]
    async fn gateway_lists_models_proxies_streams_tracks_cost_and_stops_after_limit() {
        let database_url = std::env::var("AI_GATEWAY_TEST_DATABASE_URL")
            .ok()
            .or_else(|| std::env::var("MEMORY_TEST_DATABASE_URL").ok());
        let Some(database_url) = database_url else {
            return;
        };

        let upstream_state = UpstreamState::default();
        let upstream_app = Router::new()
            .route("/v1/chat/completions", post(mock_openai))
            .with_state(upstream_state.clone());
        let (upstream_url, upstream_task) = spawn_test_server(upstream_app).await;

        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&database_url)
            .await
            .unwrap();
        work_insights_db::migrate(&pool).await.unwrap();

        let suffix = Uuid::new_v4().simple().to_string();
        let raw_key = format!("dst_live_{suffix}_secret");
        let key_prefix = format!("dst_live_{suffix}");
        let key_id = db::insert_ai_key(
            &pool,
            "gateway-test@example.invalid",
            &key_prefix,
            &raw_key,
            1,
        )
        .await
        .unwrap();
        let stream_raw_key = format!("dst_live_stream_{suffix}_secret");
        let stream_key_prefix = format!("dst_live_stream_{suffix}");
        let stream_key_id = db::insert_ai_key(
            &pool,
            "gateway-stream-test@example.invalid",
            &stream_key_prefix,
            &stream_raw_key,
            1_000_000,
        )
        .await
        .unwrap();

        let config = Arc::new(Config {
            database_url: database_url.clone(),
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            auth_internal_url: None,
            ai_gateway: Some(AiGatewayConfig {
                upstream_base_url: format!("{upstream_url}/v1"),
                openai_api_key: "upstream-secret".to_string(),
                models: vec![model()],
            }),
            memory: MemoryServiceConfig {
                internal_url: "http://127.0.0.1:1".to_string(),
                internal_api_token: "test-internal-token-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
                    .to_string(),
                upstream_timeout_secs: 5,
                max_body_bytes: 32 * 1024,
                rate_limit_per_minute: 10,
            },
            storage: StorageConfig {
                endpoint: "https://storage.invalid".to_string(),
                bucket: "test".to_string(),
                region: "test".to_string(),
                access_key_id: "test".to_string(),
                secret_access_key: "test".to_string(),
                presign_expiry_secs: 60,
            },
        });
        let state = AppState {
            config,
            pool: pool.clone(),
            http: reqwest::Client::new(),
            http_no_redirect: reqwest::Client::new(),
            memory_http: reqwest::Client::new(),
            memory_query_limiter: crate::memory_proxy::MemoryQueryRateLimiter::new(10),
            agent_connections: Arc::new(Mutex::new(HashMap::new())),
        };
        let gateway_app = Router::new()
            .route("/v1/models", axum::routing::get(get_models))
            .route("/v1/chat/completions", post(post_chat_completions))
            .with_state(state);
        let (gateway_url, gateway_task) = spawn_test_server(gateway_app).await;
        let client = reqwest::Client::new();

        let models_response = client
            .get(format!("{gateway_url}/v1/models"))
            .bearer_auth(&raw_key)
            .send()
            .await
            .unwrap();
        assert_eq!(models_response.status(), StatusCode::OK);
        let models_body: Value = models_response.json().await.unwrap();
        assert_eq!(models_body["data"][0]["id"], "test-model");

        let response = client
            .post(format!("{gateway_url}/v1/chat/completions"))
            .bearer_auth(&raw_key)
            .json(&json!({
                "model": "test-model",
                "messages": [{"role": "user", "content": "test"}]
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value = response.json().await.unwrap();
        assert_eq!(body["choices"][0]["message"]["content"], "ok");

        let spent: i64 = sqlx::query_scalar("SELECT spent_microusd FROM ai_keys WHERE id = $1")
            .bind(key_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(spent, 1_665);

        let blocked = client
            .post(format!("{gateway_url}/v1/chat/completions"))
            .bearer_auth(&raw_key)
            .json(&json!({
                "model": "test-model",
                "messages": [],
                "max_completion_tokens": 8192,
                "max_tokens": 8192
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::TOO_MANY_REQUESTS);
        let blocked_body: Value = blocked.json().await.unwrap();
        assert_eq!(blocked_body["error"]["code"], "insufficient_quota");

        let streamed = client
            .post(format!("{gateway_url}/v1/chat/completions"))
            .bearer_auth(&stream_raw_key)
            .json(&json!({
                "model": "test-model",
                "messages": [{"role": "user", "content": "stream"}],
                "stream": true
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(streamed.status(), StatusCode::OK);
        assert!(streamed.text().await.unwrap().contains("[DONE]"));

        let mut stream_spent: i64 = 0;
        for _ in 0..20 {
            stream_spent = sqlx::query_scalar("SELECT spent_microusd FROM ai_keys WHERE id = $1")
                .bind(stream_key_id)
                .fetch_one(&pool)
                .await
                .unwrap();
            if stream_spent > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert_eq!(stream_spent, 1_665);

        let requests = upstream_state.requests.lock().await;
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].0, "Bearer upstream-secret");
        assert!(requests[0].1.get("max_completion_tokens").is_none());
        assert!(requests[0].1.get("max_tokens").is_none());
        assert_eq!(requests[0].1["service_tier"], "default");
        assert_eq!(requests[1].1["stream_options"]["include_usage"], true);
        drop(requests);

        sqlx::query("DELETE FROM ai_usage WHERE key_id = ANY($1)")
            .bind(vec![key_id, stream_key_id])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM ai_keys WHERE id = ANY($1)")
            .bind(vec![key_id, stream_key_id])
            .execute(&pool)
            .await
            .unwrap();
        gateway_task.abort();
        upstream_task.abort();
    }

    async fn mock_openai(
        State(state): State<UpstreamState>,
        headers: HeaderMap,
        axum::Json(body): axum::Json<Value>,
    ) -> Response {
        let authorization = headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_string();
        state
            .requests
            .lock()
            .await
            .push((authorization, body.clone()));
        if body.get("stream").and_then(Value::as_bool) == Some(true) {
            let sse = concat!(
                "data: {\"id\":\"chatcmpl-test\",\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                "data: {\"id\":\"chatcmpl-test\",\"choices\":[],\"usage\":{\"prompt_tokens\":1000,\"completion_tokens\":500,\"prompt_tokens_details\":{\"cached_tokens\":400,\"cache_write_tokens\":100}}}\n\n",
                "data: [DONE]\n\n"
            );
            response_with_body(
                StatusCode::OK,
                Some(header::HeaderValue::from_static("text/event-stream")),
                Body::from(sse),
            )
        } else {
            json_response(
                StatusCode::OK,
                json!({
                    "id": "chatcmpl-test",
                    "object": "chat.completion",
                    "choices": [{"message": {"role": "assistant", "content": "ok"}}],
                    "usage": {
                        "prompt_tokens": 1000,
                        "completion_tokens": 500,
                        "prompt_tokens_details": {"cached_tokens": 400, "cache_write_tokens": 100}
                    }
                }),
            )
        }
    }

    async fn spawn_test_server(app: Router) -> (String, JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), task)
    }
}
