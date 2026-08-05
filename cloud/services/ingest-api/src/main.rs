use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use anyhow::Context;
use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, State};
use axum::http::{header::CONTENT_ENCODING, HeaderMap, HeaderName};
use axum::response::IntoResponse;
use axum::routing::{get, post, put};
use axum::{Json, Router};
use chrono::{DateTime, Duration, Utc};
use dystil_protocol::{
    DeviceCaptureState, DeviceSummary, DeviceSyncStateResponse, ImageCompleteRequest,
    ImageCompleteResponse, ImagePrepareRequest, ImagePrepareResponse, ImagePrepareResult,
    ImageSyncMode, ImageSyncPolicy, ImageUploadTicket, ListDevicesResponse, RegisterDeviceRequest,
    RegisterDeviceResponse, RevokeDeviceResponse, SegmentUploadResponse, SegmentingPolicy,
    SyncPolicy, UpdateDeviceCaptureStateRequest, UpdateDeviceCaptureStateResponse,
    WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
};
use hmac::{Hmac, Mac};
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use serde_json::json;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use url::Url;
use uuid::Uuid;
use work_insights_db::identity::{self, AppIdentity};
use work_insights_db::ingest as db_ingest;
use work_insights_db::segments as db_segments;
use work_insights_db::DbError;
use work_insights_ingest::{process_segment_upload, IngestProcessError};

mod agent_mailbox;
mod ai_gateway;
mod auth;
mod auth_proxy;
mod memory_proxy;
mod semantic_trees;

#[derive(Debug, Clone)]
pub(crate) struct Config {
    database_url: String,
    bind_addr: SocketAddr,
    auth_internal_url: Option<String>,
    ai_gateway: Option<ai_gateway::AiGatewayConfig>,
    memory: MemoryServiceConfig,
    storage: StorageConfig,
}

#[derive(Debug, Clone)]
struct MemoryServiceConfig {
    internal_url: String,
    internal_api_token: String,
    upstream_timeout_secs: u64,
    max_body_bytes: usize,
    rate_limit_per_minute: usize,
}

#[derive(Debug, Clone)]
struct StorageConfig {
    endpoint: String,
    bucket: String,
    region: String,
    access_key_id: String,
    secret_access_key: String,
    presign_expiry_secs: i64,
}

impl Config {
    fn from_env() -> anyhow::Result<Self> {
        let bind_addr = std::env::var("WORK_INSIGHTS_BIND_ADDR")
            .unwrap_or_else(|_| "127.0.0.1:8089".to_string())
            .parse()
            .context("WORK_INSIGHTS_BIND_ADDR must be host:port")?;
        Ok(Self {
            database_url: required_env("WORK_INSIGHTS_DATABASE_URL")?,
            bind_addr,
            auth_internal_url: std::env::var("AUTH_INTERNAL_URL")
                .ok()
                .map(|value| value.trim_end_matches('/').to_string()),
            ai_gateway: ai_gateway::AiGatewayConfig::from_env()?,
            memory: MemoryServiceConfig::from_env()?,
            storage: StorageConfig::from_env()?,
        })
    }
}

impl MemoryServiceConfig {
    fn from_env() -> anyhow::Result<Self> {
        let internal_api_token = required_env("MEMORY_INTERNAL_API_TOKEN")?;
        if internal_api_token.len() < 32 {
            anyhow::bail!("MEMORY_INTERNAL_API_TOKEN must contain at least 32 characters");
        }
        let upstream_timeout_secs = env_u64("MEMORY_QUERY_UPSTREAM_TIMEOUT_SECONDS", 120)?;
        let max_body_bytes = env_usize("MEMORY_QUERY_MAX_BODY_BYTES", 32 * 1024)?;
        let rate_limit_per_minute = env_usize("MEMORY_QUERY_RATE_LIMIT_PER_MINUTE", 10)?;
        if upstream_timeout_secs == 0 || max_body_bytes == 0 || rate_limit_per_minute == 0 {
            anyhow::bail!("memory query limits and timeout must be positive");
        }
        Ok(Self {
            internal_url: required_env("MEMORY_INTERNAL_URL")?
                .trim_end_matches('/')
                .to_string(),
            internal_api_token,
            upstream_timeout_secs,
            max_body_bytes,
            rate_limit_per_minute,
        })
    }
}

impl StorageConfig {
    fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            endpoint: required_env("WORK_INSIGHTS_STORAGE_ENDPOINT")?
                .trim_end_matches('/')
                .to_string(),
            bucket: required_env("WORK_INSIGHTS_STORAGE_BUCKET")?,
            region: std::env::var("WORK_INSIGHTS_STORAGE_REGION")
                .unwrap_or_else(|_| "us-east-1".to_string()),
            access_key_id: required_env("WORK_INSIGHTS_STORAGE_ACCESS_KEY_ID")?,
            secret_access_key: required_env("WORK_INSIGHTS_STORAGE_SECRET_ACCESS_KEY")?,
            presign_expiry_secs: std::env::var("WORK_INSIGHTS_STORAGE_PRESIGN_EXPIRY_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(900),
        })
    }
}

fn required_env(name: &str) -> anyhow::Result<String> {
    std::env::var(name).with_context(|| format!("{name} is required"))
}

fn env_u64(name: &str, default: u64) -> anyhow::Result<u64> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("reading {name} failed")),
    }
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(err) => Err(err).with_context(|| format!("reading {name} failed")),
    }
}

#[derive(Clone)]
struct AppState {
    config: Arc<Config>,
    pool: PgPool,
    http: reqwest::Client,
    http_no_redirect: reqwest::Client,
    memory_http: reqwest::Client,
    memory_query_limiter: memory_proxy::MemoryQueryRateLimiter,
    agent_connections: Arc<tokio::sync::Mutex<HashMap<String, tokio::sync::mpsc::Sender<()>>>>,
}

#[derive(Debug, thiserror::Error)]
enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("{0}")]
    TooManyRequests(String),
    #[error("{0}")]
    ServiceUnavailable(String),
    #[error("{0}")]
    BadGateway(String),
    #[error("{0}")]
    GatewayTimeout(String),
    #[error("{0}")]
    Internal(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            AppError::Unauthorized => axum::http::StatusCode::UNAUTHORIZED,
            AppError::BadRequest(_) => axum::http::StatusCode::BAD_REQUEST,
            AppError::NotFound(_) => axum::http::StatusCode::NOT_FOUND,
            AppError::Forbidden(_) => axum::http::StatusCode::FORBIDDEN,
            AppError::TooManyRequests(_) => axum::http::StatusCode::TOO_MANY_REQUESTS,
            AppError::ServiceUnavailable(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            AppError::BadGateway(_) => axum::http::StatusCode::BAD_GATEWAY,
            AppError::GatewayTimeout(_) => axum::http::StatusCode::GATEWAY_TIMEOUT,
            AppError::Internal(_) | AppError::Io(_) | AppError::Sqlx(_) | AppError::Json(_) => {
                axum::http::StatusCode::INTERNAL_SERVER_ERROR
            }
        };
        let message = self.to_string();
        (status, Json(json!({ "ok": false, "error": message }))).into_response()
    }
}

impl From<DbError> for AppError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::Sqlx(err) => Self::Sqlx(err),
            DbError::Json(err) => Self::Json(err),
            DbError::Other(msg) => Self::Internal(msg),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "work_insights_ingest_api=info,tower_http=info".into()),
        )
        .init();

    serve().await
}

async fn serve() -> anyhow::Result<()> {
    let config = Arc::new(Config::from_env()?);
    let state = build_state(config).await?;
    let bind_addr = state.config.bind_addr;
    let app = router(state);

    tracing::info!("work-insights API listening on {}", bind_addr);
    let listener = tokio::net::TcpListener::bind(bind_addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn build_state(config: Arc<Config>) -> anyhow::Result<AppState> {
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        .context("connecting to Postgres failed")?;
    work_insights_db::migrate(&pool)
        .await
        .context("running migrations failed")?;
    let http = reqwest::Client::new();
    let http_no_redirect = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building no-redirect HTTP client")?;
    let memory_http = reqwest::Client::builder()
        .timeout(StdDuration::from_secs(config.memory.upstream_timeout_secs))
        .build()
        .context("building memory HTTP client")?;
    let memory_query_limiter =
        memory_proxy::MemoryQueryRateLimiter::new(config.memory.rate_limit_per_minute);
    Ok(AppState {
        config,
        pool,
        http,
        http_no_redirect,
        memory_http,
        memory_query_limiter,
        agent_connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
    })
}

fn router(state: AppState) -> Router {
    let memory_query_max_body_bytes = state.config.memory.max_body_bytes;
    Router::new()
        .route("/health", get(health))
        .route("/health/auth", get(auth_proxy::auth_health))
        .nest(
            "/api/auth",
            Router::new().fallback(auth_proxy::proxy_auth_fallback),
        )
        .route("/me", get(get_me))
        .route("/me/onboarding", put(put_me_onboarding))
        .route("/devices/register", post(register_device))
        .route("/devices", get(list_devices))
        .route("/devices/self/capture-state", put(put_device_capture_state))
        .route("/devices/:device_id/revoke", post(revoke_device))
        .route("/v1/models", get(ai_gateway::get_models))
        .route(
            "/v1/chat/completions",
            post(ai_gateway::post_chat_completions),
        )
        .route("/v1/agent/peers", get(agent_mailbox::get_peers))
        .route(
            "/v1/agent/messages",
            get(agent_mailbox::get_messages)
                .post(agent_mailbox::post_message)
                .layer(DefaultBodyLimit::max(
                    dystil_protocol::agent_mailbox::MAX_AGENT_BODY_BYTES,
                )),
        )
        .route("/v1/agent/ws", get(agent_mailbox::websocket))
        .route("/v1/ingest/segments", post(post_segments))
        .route(
            "/v1/ingest/segments/device-state",
            get(get_device_segment_state),
        )
        .route("/v1/ingest/config", get(get_sync_config))
        .route("/v1/ingest/images/prepare", post(post_image_prepare))
        .route("/v1/ingest/images/complete", post(post_image_complete))
        .route(
            "/v1/semantic-tree-samples",
            post(semantic_trees::post_semantic_tree_sample).layer(DefaultBodyLimit::max(
                semantic_trees::MAX_MULTIPART_BODY_BYTES,
            )),
        )
        .route("/v1/dashboard/session", get(get_dashboard_session))
        .route("/v1/tenant/:slug", get(get_tenant_org))
        .route(
            "/v1/memory/query",
            post(memory_proxy::post_memory_query)
                .layer(DefaultBodyLimit::max(memory_query_max_body_bytes)),
        )
        .layer(
            CorsLayer::new()
                .allow_origin([
                    "http://localhost:1420".parse().unwrap(),
                    "http://localhost:5173".parse().unwrap(),
                    "tauri://localhost".parse().unwrap(),
                ])
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::PATCH,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::CONTENT_TYPE,
                    axum::http::header::AUTHORIZATION,
                    CONTENT_ENCODING,
                    HeaderName::from_static("x-dystil-payload-sha256"),
                    axum::http::header::HeaderName::from_static("platform"),
                ])
                .allow_credentials(true)
                .expose_headers([
                    axum::http::header::HeaderName::from_static("set-auth-token"),
                    axum::http::header::HeaderName::from_static("set-auth-jwt"),
                ]),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({ "ok": true, "service": "rust-api" }))
}

async fn get_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AppIdentity>, AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    Ok(Json(identity))
}

async fn put_me_onboarding(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    if !body.is_object() {
        return Err(AppError::BadRequest(
            "onboarding payload must be a JSON object".to_string(),
        ));
    }

    let payload_size = serde_json::to_vec(&body).map_err(AppError::Json)?.len();
    if payload_size > 64 * 1024 {
        return Err(AppError::BadRequest(
            "onboarding payload exceeds 64KB limit".to_string(),
        ));
    }

    identity::save_onboarding_data(&state.pool, &identity.user_id, &body).await?;
    Ok(Json(json!({ "ok": true })))
}

async fn register_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegisterDeviceRequest>,
) -> Result<Json<RegisterDeviceResponse>, AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    let device_label = body.device_label.trim();
    let platform = body.platform.trim();
    if device_label.is_empty() || platform.is_empty() {
        return Err(AppError::BadRequest(
            "device_label and platform are required".to_string(),
        ));
    }

    let registered = identity::register_device(
        &state.pool,
        &identity.org_id,
        &identity.user_id,
        device_label,
        platform,
    )
    .await?;
    Ok(Json(RegisterDeviceResponse {
        ok: true,
        device_id: registered.device.device_id,
        device_token: registered.raw_token,
        device_label: registered.device.device_label,
        platform: registered.device.platform,
    }))
}

async fn list_devices(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<ListDevicesResponse>, AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    let devices = identity::list_devices_for_org(&state.pool, &identity.org_id).await?;
    Ok(Json(ListDevicesResponse {
        ok: true,
        devices: devices.into_iter().map(device_summary).collect(),
    }))
}

async fn put_device_capture_state(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UpdateDeviceCaptureStateRequest>,
) -> Result<Json<UpdateDeviceCaptureStateResponse>, AppError> {
    match (body.capture_state, body.capture_pause_until) {
        (DeviceCaptureState::Recording, Some(_)) => {
            return Err(AppError::BadRequest(
                "recording state cannot include capture_pause_until".to_string(),
            ));
        }
        (DeviceCaptureState::Paused, None) => {
            return Err(AppError::BadRequest(
                "paused state requires capture_pause_until".to_string(),
            ));
        }
        _ => {}
    }

    let principal = auth::authenticate_device(&state, &headers).await?;
    let updated_at = identity::update_device_capture_state(
        &state.pool,
        &principal.device_id,
        body.capture_state.as_str(),
        body.capture_pause_until,
    )
    .await?;

    // Pause history is operational telemetry, not part of the capture-state
    // contract. Never make the desktop client retry or fail because analytics
    // storage is unavailable after the live device state was updated.
    if let Err(error) = identity::record_device_capture_pause_transition(
        &state.pool,
        &principal.device_id,
        body.capture_state.as_str(),
        body.capture_pause_until,
    )
    .await
    {
        tracing::warn!(
            %error,
            device_id = %principal.device_id,
            capture_state = body.capture_state.as_str(),
            "device capture pause history update failed; continuing with live state"
        );
    }

    Ok(Json(UpdateDeviceCaptureStateResponse {
        ok: true,
        capture_state: body.capture_state,
        capture_pause_until: body.capture_pause_until,
        capture_state_updated_at: updated_at,
    }))
}

async fn revoke_device(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(device_id): Path<String>,
) -> Result<Json<RevokeDeviceResponse>, AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    let device = identity::find_device_for_org(&state.pool, &identity.org_id, &device_id)
        .await?
        .ok_or_else(|| AppError::NotFound("device not found".to_string()))?;

    if identity.user_id != device.user_id {
        return Err(AppError::Forbidden(
            "may only revoke own devices".to_string(),
        ));
    }

    let revoked = identity::revoke_device(&state.pool, &identity.org_id, &device_id).await?;
    Ok(Json(RevokeDeviceResponse {
        ok: true,
        device_id,
        revoked,
    }))
}

async fn post_segments(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SegmentUploadResponse>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let compressed_sha256 = headers
        .get("x-dystil-payload-sha256")
        .ok_or_else(|| AppError::BadRequest("missing X-Dystil-Payload-Sha256 header".to_string()))?
        .to_str()
        .map_err(|_| AppError::BadRequest("invalid X-Dystil-Payload-Sha256 header".to_string()))?;
    let content_encoding = headers
        .get(CONTENT_ENCODING)
        .ok_or_else(|| AppError::BadRequest("missing Content-Encoding header".to_string()))?
        .to_str()
        .map_err(|_| AppError::BadRequest("invalid Content-Encoding header".to_string()))?;
    if content_encoding != "gzip" {
        return Err(AppError::BadRequest(
            "Content-Encoding must be gzip".to_string(),
        ));
    }

    match process_segment_upload(&state.pool, &principal, &body, compressed_sha256).await {
        Ok(response) => {
            tracing::info!(
                org_id = %principal.org_id,
                user_id = %principal.user_id,
                device_id = %principal.device_id,
                inserted_count = response.inserted_count,
                deduped_count = response.deduped_count,
                "segment_upload_processed"
            );
            Ok(Json(response))
        }
        Err(err) if err.is_bad_payload() => Err(AppError::BadRequest(err.to_string())),
        Err(IngestProcessError::Temporary(err)) => {
            tracing::error!(
                org_id = %principal.org_id,
                user_id = %principal.user_id,
                device_id = %principal.device_id,
                error = %err,
                "segment_upload_database_error"
            );
            Err(AppError::Internal(format!("db error: {err}")))
        }
        Err(IngestProcessError::Json(err)) => {
            Err(AppError::BadRequest(format!("invalid JSON: {err}")))
        }
        Err(err) => Err(AppError::Internal(err.to_string())),
    }
}

async fn get_dashboard_session(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<work_insights_db::identity::DashboardIdentity>, AppError> {
    match auth::resolve_dashboard_session(&state, &headers).await? {
        Some(identity) => Ok(Json(identity)),
        None => Err(AppError::Forbidden("not authorized".to_string())),
    }
}

async fn get_tenant_org(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<work_insights_db::identity::OrganizationInfo>, AppError> {
    match work_insights_db::identity::lookup_organization_by_slug(&state.pool, &slug).await? {
        Some(info) => Ok(Json(info)),
        None => Err(AppError::NotFound("organization not found".to_string())),
    }
}

async fn get_device_segment_state(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DeviceSyncStateResponse>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let sync_state = db_segments::get_device_sync_state(&state.pool, &principal).await?;
    Ok(Json(sync_state))
}

async fn get_sync_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<SyncPolicy>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let stored: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT policy_json FROM organization_sync_policies WHERE org_id = $1")
            .bind(&principal.org_id)
            .fetch_optional(&state.pool)
            .await?;
    let policy = match stored {
        Some(value) => serde_json::from_value(value)
            .map_err(|err| AppError::Internal(format!("stored sync policy is invalid: {err}")))?,
        None => default_sync_policy(),
    };
    Ok(Json(policy))
}

fn default_sync_policy() -> SyncPolicy {
    SyncPolicy {
        schema_version: 1,
        policy_version: "server-default-v1".to_string(),
        issued_at: Utc::now(),
        refresh_after_seconds: 60,
        image_sync: ImageSyncPolicy {
            mode: ImageSyncMode::AllWithShadow,
            evaluator_version: "image-filter-v1".to_string(),
            stable_text_change_min_seconds: 60,
            min_text_change_chars: 200,
            min_text_change_tokens: 40,
            text_change_jaccard_distance_threshold: 0.40,
            max_selected_per_minute: 3,
            candidate_min_gap_seconds: 20,
            max_uploads_per_pass: 100,
            max_upload_bytes_per_pass: 100 * 1024 * 1024,
            jpeg_quality: 86,
            max_jpeg_width: 1920,
        },
        segmenting: SegmentingPolicy {
            max_tokens: 10_000,
            inactivity_seconds: 300,
            max_duration_seconds: 900,
        },
    }
}

async fn post_image_prepare(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImagePrepareRequest>,
) -> Result<Json<ImagePrepareResponse>, AppError> {
    tracing::info!(
        schema_version = body.schema_version,
        image_count = body.images.len(),
        "ingest-api: image prepare request received"
    );
    if body.schema_version != WORK_INSIGHTS_IMAGE_SCHEMA_VERSION {
        tracing::warn!(
            schema_version = body.schema_version,
            expected_schema_version = WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
            "ingest-api: image prepare rejected due to unsupported schema version"
        );
        return Err(AppError::BadRequest(format!(
            "unsupported image schema_version {}",
            body.schema_version
        )));
    }
    let principal = auth::authenticate_device(&state, &headers).await?;
    tracing::info!(
        org_id = %principal.org_id,
        user_id = %principal.user_id,
        device_id = %principal.device_id,
        image_count = body.images.len(),
        "ingest-api: image prepare authenticated"
    );
    let mut results = Vec::with_capacity(body.images.len());
    for image in body.images {
        if image.client_image_key.trim().is_empty()
            || image.content_hash.trim().is_empty()
            || image.mime_type.trim().is_empty()
            || image.linked_frame_ids.is_empty()
        {
            tracing::warn!(
                org_id = %principal.org_id,
                user_id = %principal.user_id,
                device_id = %principal.device_id,
                client_image_key = %image.client_image_key,
                linked_frame_count = image.linked_frame_ids.len(),
                mime_type = %image.mime_type,
                "ingest-api: image prepare rejected due to invalid manifest"
            );
            return Err(AppError::BadRequest(
                "image manifests require client_image_key, content_hash, mime_type, and linked_frame_ids"
                    .to_string(),
            ));
        }

        let image_id = Uuid::new_v4().to_string();
        let sync_metadata = image
            .sync_metadata
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(AppError::Json)?;
        let object_ext = object_extension_for_mime(&image.mime_type)?;
        let object_key = format!(
            "org/{}/user/{}/device/{}/capture-images/{}/{}.{}",
            principal.org_id,
            principal.user_id,
            principal.device_id,
            Utc::now().format("%Y/%m/%d"),
            image_id,
            object_ext
        );
        if let Err(err) = db_ingest::upsert_prepared_capture_image(
            &state.pool,
            &principal,
            &image_id,
            &image.client_image_key,
            &image.content_hash,
            &object_key,
            &image.mime_type,
            image.byte_size as i64,
            image.width as i32,
            image.height as i32,
            &image.selection_reason,
            sync_metadata.as_ref(),
        )
        .await
        {
            tracing::error!(
                org_id = %principal.org_id,
                user_id = %principal.user_id,
                device_id = %principal.device_id,
                client_image_key = %image.client_image_key,
                image_id = %image_id,
                object_key = %object_key,
                error = %err,
                "ingest-api: image prepare failed while storing prepared image record"
            );
            return Err(err.into());
        }
        let expires_at = Utc::now() + Duration::seconds(state.config.storage.presign_expiry_secs);
        let upload_url = match presign_put_url(&state.config.storage, &object_key, expires_at) {
            Ok(upload_url) => upload_url,
            Err(err) => {
                tracing::error!(
                    org_id = %principal.org_id,
                    user_id = %principal.user_id,
                    device_id = %principal.device_id,
                    client_image_key = %image.client_image_key,
                    image_id = %image_id,
                    object_key = %object_key,
                    error = %err,
                    "ingest-api: image prepare failed while generating presigned upload url"
                );
                return Err(err);
            }
        };
        tracing::info!(
            org_id = %principal.org_id,
            user_id = %principal.user_id,
            device_id = %principal.device_id,
            client_image_key = %image.client_image_key,
            image_id = %image_id,
            object_key = %object_key,
            linked_frame_count = image.linked_frame_ids.len(),
            byte_size = image.byte_size,
            width = image.width,
            height = image.height,
            expires_at = %expires_at,
            "ingest-api: image prepare issued upload ticket"
        );
        results.push(ImagePrepareResult {
            client_image_key: image.client_image_key,
            image_id: image_id.clone(),
            upload_required: true,
            upload_ticket: Some(ImageUploadTicket {
                image_id,
                object_key,
                upload_url,
                expires_at,
            }),
        });
    }

    tracing::info!(
        org_id = %principal.org_id,
        user_id = %principal.user_id,
        device_id = %principal.device_id,
        prepared_count = results.len(),
        "ingest-api: image prepare completed"
    );
    Ok(Json(ImagePrepareResponse { ok: true, results }))
}

async fn post_image_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ImageCompleteRequest>,
) -> Result<Json<ImageCompleteResponse>, AppError> {
    tracing::info!(
        schema_version = body.schema_version,
        image_count = body.images.len(),
        "ingest-api: image complete request received"
    );
    if body.schema_version != WORK_INSIGHTS_IMAGE_SCHEMA_VERSION {
        tracing::warn!(
            schema_version = body.schema_version,
            expected_schema_version = WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
            "ingest-api: image complete rejected due to unsupported schema version"
        );
        return Err(AppError::BadRequest(format!(
            "unsupported image schema_version {}",
            body.schema_version
        )));
    }
    let principal = auth::authenticate_device(&state, &headers).await?;
    tracing::info!(
        org_id = %principal.org_id,
        user_id = %principal.user_id,
        device_id = %principal.device_id,
        image_count = body.images.len(),
        "ingest-api: image complete authenticated"
    );
    if let Err(err) =
        db_ingest::complete_capture_images(&state.pool, &principal, &body.images).await
    {
        tracing::error!(
            org_id = %principal.org_id,
            user_id = %principal.user_id,
            device_id = %principal.device_id,
            image_count = body.images.len(),
            error = %err,
            "ingest-api: image complete failed while finalizing image records"
        );
        return Err(err.into());
    }
    tracing::info!(
        org_id = %principal.org_id,
        user_id = %principal.user_id,
        device_id = %principal.device_id,
        completed_count = body.images.len(),
        "ingest-api: image complete finished"
    );
    Ok(Json(ImageCompleteResponse {
        ok: true,
        completed: body.images.len(),
        linked: 0,
    }))
}

#[cfg(test)]
fn is_safe_id(value: &str) -> bool {
    value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == ':')
}

fn device_summary(device: work_insights_db::identity::DeviceRecord) -> DeviceSummary {
    DeviceSummary {
        device_id: device.device_id,
        org_id: device.org_id,
        user_id: device.user_id,
        device_label: device.device_label,
        platform: device.platform,
        revoked_at: device.revoked_at,
        last_seen_at: device.last_seen_at,
        capture_state: device.capture_state.as_deref().and_then(|state| match state {
            "recording" => Some(DeviceCaptureState::Recording),
            "paused" => Some(DeviceCaptureState::Paused),
            _ => None,
        }),
        capture_pause_until: device.capture_pause_until,
        capture_state_updated_at: device.capture_state_updated_at,
        created_at: device.created_at,
    }
}

fn object_extension_for_mime(mime_type: &str) -> Result<&'static str, AppError> {
    match mime_type {
        "image/jpeg" | "image/jpg" => Ok("jpg"),
        _ => Err(AppError::BadRequest(format!(
            "unsupported image mime_type {mime_type}"
        ))),
    }
}

type HmacSha256 = Hmac<sha2::Sha256>;

const AWS_QUERY_ENCODE_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

fn presign_put_url(
    storage: &StorageConfig,
    object_key: &str,
    expires_at: DateTime<Utc>,
) -> Result<String, AppError> {
    let now = Utc::now();
    let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let date_stamp = now.format("%Y%m%d").to_string();
    let credential_scope = format!("{date_stamp}/{}/s3/aws4_request", storage.region);
    let host = Url::parse(&storage.endpoint)
        .map_err(|err| AppError::Internal(format!("invalid storage endpoint: {err}")))?
        .host_str()
        .ok_or_else(|| AppError::Internal("storage endpoint missing host".to_string()))?
        .to_string();

    let canonical_uri = format!(
        "/{}/{}",
        encode_uri_path(&storage.bucket),
        encode_uri_path(object_key)
    );
    let algorithm = "AWS4-HMAC-SHA256";
    let signed_headers = "host";
    let expires = (expires_at - now).num_seconds().max(1);

    let mut query = vec![
        ("X-Amz-Algorithm".to_string(), algorithm.to_string()),
        (
            "X-Amz-Credential".to_string(),
            format!("{}/{}", storage.access_key_id, credential_scope),
        ),
        ("X-Amz-Date".to_string(), amz_date.clone()),
        ("X-Amz-Expires".to_string(), expires.to_string()),
        (
            "X-Amz-SignedHeaders".to_string(),
            signed_headers.to_string(),
        ),
    ];
    query.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_query = query
        .iter()
        .map(|(key, value)| format!("{}={}", encode_query(key), encode_query(value)))
        .collect::<Vec<_>>()
        .join("&");
    let canonical_headers = format!("host:{host}\n");
    let canonical_request = format!(
        "PUT\n{}\n{}\n{}\n{}\nUNSIGNED-PAYLOAD",
        canonical_uri, canonical_query, canonical_headers, signed_headers
    );
    let string_to_sign = format!(
        "{algorithm}\n{amz_date}\n{credential_scope}\n{}",
        sha256_hex(canonical_request.as_bytes())
    );
    let signing_key = aws_signing_key(&storage.secret_access_key, &date_stamp, &storage.region)?;
    let mut mac = HmacSha256::new_from_slice(&signing_key)
        .map_err(|err| AppError::Internal(format!("failed to initialize signing mac: {err}")))?;
    mac.update(string_to_sign.as_bytes());
    let signature = hex::encode(mac.finalize().into_bytes());
    let final_query = format!("{canonical_query}&X-Amz-Signature={signature}");
    Ok(format!(
        "{}/{}/{}?{}",
        storage.endpoint.trim_end_matches('/'),
        encode_uri_path(&storage.bucket),
        encode_uri_path(object_key),
        final_query
    ))
}

fn aws_signing_key(
    secret_access_key: &str,
    date_stamp: &str,
    region: &str,
) -> Result<Vec<u8>, AppError> {
    let k_date = hmac_sha256(
        format!("AWS4{secret_access_key}").as_bytes(),
        date_stamp.as_bytes(),
    )?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, b"s3")?;
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|err| AppError::Internal(format!("failed to initialize hmac: {err}")))?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;

    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

fn encode_query(value: &str) -> String {
    utf8_percent_encode(value, AWS_QUERY_ENCODE_SET).to_string()
}

fn encode_uri_path(value: &str) -> String {
    value
        .split('/')
        .map(|segment| utf8_percent_encode(segment, AWS_QUERY_ENCODE_SET).to_string())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use tokio::sync::Mutex;
    use tokio::task::JoinHandle;

    #[derive(Clone)]
    struct MemoryMockState {
        requests: Arc<Mutex<Vec<serde_json::Value>>>,
        token: String,
    }

    #[test]
    fn unsafe_batch_ids_are_rejected() {
        assert!(is_safe_id("abc_123-DEF:42"));
        assert!(!is_safe_id("../abc"));
        assert!(!is_safe_id("abc/def"));
    }

    #[tokio::test]
    async fn authenticated_memory_query_forwards_only_trusted_scope() {
        let Some(database_url) = std::env::var("MEMORY_TEST_DATABASE_URL").ok() else {
            return;
        };
        let auth_user_id = format!("auth_{}", Uuid::new_v4().simple());
        let auth_app = Router::new().route(
            "/api/auth/get-session",
            get({
                let auth_user_id = auth_user_id.clone();
                move || {
                    let auth_user_id = auth_user_id.clone();
                    async move {
                        Json(json!({
                            "user": {
                                "id": auth_user_id,
                                "email": format!("{}@example.invalid", Uuid::new_v4().simple()),
                                "emailVerified": true,
                                "name": "Memory Test User"
                            }
                        }))
                    }
                }
            }),
        );
        let (auth_url, auth_task) = spawn_test_server(auth_app).await;

        let captured = Arc::new(Mutex::new(Vec::new()));
        let internal_token = "test-internal-token-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx".to_string();
        let memory_app = Router::new()
            .route("/internal/v1/memory/query", post(mock_memory_query))
            .with_state(MemoryMockState {
                requests: Arc::clone(&captured),
                token: internal_token.clone(),
            });
        let (memory_url, memory_task) = spawn_test_server(memory_app).await;

        let config = Arc::new(Config {
            database_url: database_url.clone(),
            bind_addr: "127.0.0.1:0".parse().unwrap(),
            auth_internal_url: Some(auth_url),
            ai_gateway: None,
            memory: MemoryServiceConfig {
                internal_url: memory_url,
                internal_api_token: internal_token,
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
        let pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(&database_url)
            .await
            .unwrap();
        let state = AppState {
            config,
            pool,
            http: reqwest::Client::new(),
            http_no_redirect: reqwest::Client::new(),
            memory_http: reqwest::Client::builder()
                .timeout(StdDuration::from_secs(5))
                .build()
                .unwrap(),
            memory_query_limiter: memory_proxy::MemoryQueryRateLimiter::new(10),
            agent_connections: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        };
        let (api_url, api_task) = spawn_test_server(router(state)).await;
        let client = reqwest::Client::new();

        let forged = client
            .post(format!("{api_url}/v1/memory/query"))
            .bearer_auth("user-session-token")
            .json(&json!({
                "question": "What happened?",
                "org_id": "org_attacker"
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(forged.status(), reqwest::StatusCode::BAD_REQUEST);
        assert!(captured.lock().await.is_empty());

        let response = client
            .post(format!("{api_url}/v1/memory/query"))
            .bearer_auth("user-session-token")
            .json(&json!({"question": "What happened?"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert!(response.headers().contains_key("x-request-id"));
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["citation_ids"], json!(["fct_1"]));

        let requests = captured.lock().await;
        assert_eq!(requests.len(), 1);
        assert_ne!(requests[0]["principal"]["org_id"], "org_attacker");
        assert!(requests[0]["principal"]["org_id"].as_str().is_some());
        assert!(requests[0]["principal"]["user_id"].as_str().is_some());
        assert!(requests[0].get("org_id").is_none());
        drop(requests);

        api_task.abort();
        memory_task.abort();
        auth_task.abort();
    }

    async fn mock_memory_query(
        State(state): State<MemoryMockState>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> (StatusCode, Json<serde_json::Value>) {
        let expected = format!("Bearer {}", state.token);
        if headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            != Some(expected.as_str())
        {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "unauthorized"})),
            );
        }
        state.requests.lock().await.push(body);
        (
            StatusCode::OK,
            Json(json!({
                "answer": "The cited fact describes what happened.",
                "insufficient_evidence": false,
                "citation_ids": ["fct_1"]
            })),
        )
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
