use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{auth, AppError, AppState};

const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MemoryType {
    Episode,
    Fact,
    Task,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskStatus {
    Open,
    Completed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryQueryFilters {
    #[serde(default = "default_memory_types")]
    memory_types: Vec<MemoryType>,
    #[serde(default)]
    task_statuses: Vec<TaskStatus>,
    #[serde(default)]
    start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    end_time: Option<DateTime<Utc>>,
}

impl Default for MemoryQueryFilters {
    fn default() -> Self {
        Self {
            memory_types: default_memory_types(),
            task_statuses: Vec::new(),
            start_time: None,
            end_time: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryQueryRequest {
    question: String,
    #[serde(default)]
    include_supporting_records: bool,
    #[serde(default)]
    filters: MemoryQueryFilters,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MemoryQueryResponse {
    answer: String,
    insufficient_evidence: bool,
    citation_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supporting_records: Option<Vec<Value>>,
}

#[derive(Serialize)]
struct InternalPrincipal {
    org_id: String,
    user_id: String,
}

#[derive(Serialize)]
struct InternalMemoryQueryRequest {
    request_id: String,
    principal: InternalPrincipal,
    query: MemoryQueryRequest,
}

#[derive(Clone)]
pub(crate) struct MemoryQueryRateLimiter {
    limit: usize,
    window: Duration,
    requests: Arc<Mutex<HashMap<String, VecDeque<Instant>>>>,
}

impl MemoryQueryRateLimiter {
    pub(crate) fn new(limit: usize) -> Self {
        Self {
            limit,
            window: Duration::from_secs(60),
            requests: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn allow(&self, user_id: &str) -> bool {
        let now = Instant::now();
        let mut requests = self.requests.lock().await;
        let user_requests = requests.entry(user_id.to_string()).or_default();
        while user_requests
            .front()
            .is_some_and(|time| now.duration_since(*time) >= self.window)
        {
            user_requests.pop_front();
        }
        if user_requests.len() >= self.limit {
            return false;
        }
        user_requests.push_back(now);
        true
    }
}

pub(crate) async fn post_memory_query(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(HeaderMap, Json<MemoryQueryResponse>), AppError> {
    let identity = auth::authenticate_app_user(&state, &headers).await?;
    if !state.memory_query_limiter.allow(&identity.user_id).await {
        return Err(AppError::TooManyRequests(
            "memory query rate limit exceeded".to_string(),
        ));
    }
    if body.len() > state.config.memory.max_body_bytes {
        return Err(AppError::BadRequest(
            "memory query body exceeds configured limit".to_string(),
        ));
    }

    let mut query: MemoryQueryRequest = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("invalid memory query request".to_string()))?;
    validate_request(&mut query)?;

    let request_id = format!("req_{}", Uuid::new_v4().simple());
    let envelope = InternalMemoryQueryRequest {
        request_id: request_id.clone(),
        principal: InternalPrincipal {
            org_id: identity.org_id.clone(),
            user_id: identity.user_id.clone(),
        },
        query: query.clone(),
    };
    let upstream = state
        .memory_http
        .post(format!(
            "{}/internal/v1/memory/query",
            state.config.memory.internal_url
        ))
        .bearer_auth(&state.config.memory.internal_api_token)
        .json(&envelope)
        .send()
        .await
        .map_err(map_upstream_error)?;
    let status = upstream.status();
    let response_bytes = upstream
        .bytes()
        .await
        .map_err(|_| AppError::BadGateway("memory service response failed".to_string()))?;
    if response_bytes.len() > MAX_RESPONSE_BYTES {
        return Err(AppError::BadGateway(
            "memory service response exceeded limit".to_string(),
        ));
    }
    if !status.is_success() {
        return Err(match status.as_u16() {
            422 => AppError::BadRequest("invalid memory query request".to_string()),
            503 => AppError::ServiceUnavailable("memory query unavailable".to_string()),
            504 => AppError::GatewayTimeout("memory query timed out".to_string()),
            _ => AppError::BadGateway("memory query failed".to_string()),
        });
    }

    let response: MemoryQueryResponse = serde_json::from_slice(&response_bytes)
        .map_err(|_| AppError::BadGateway("invalid memory service response".to_string()))?;
    validate_response(&response, query.include_supporting_records)?;

    tracing::info!(
        request_id = %request_id,
        org_id = %identity.org_id,
        user_id = %identity.user_id,
        citation_count = response.citation_ids.len(),
        insufficient_evidence = response.insufficient_evidence,
        "memory_query_completed"
    );

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        "x-request-id",
        HeaderValue::from_str(&request_id)
            .map_err(|_| AppError::Internal("invalid request ID".to_string()))?,
    );
    Ok((response_headers, Json(response)))
}

fn validate_request(query: &mut MemoryQueryRequest) -> Result<(), AppError> {
    query.question = query.question.trim().to_string();
    let question_chars = query.question.chars().count();
    if !(1..=2_000).contains(&question_chars) {
        return Err(AppError::BadRequest(
            "question must contain 1-2000 characters".to_string(),
        ));
    }
    if query.filters.memory_types.is_empty() || query.filters.memory_types.len() > 3 {
        return Err(AppError::BadRequest(
            "memory_types must contain 1-3 values".to_string(),
        ));
    }
    if query.filters.task_statuses.len() > 4 {
        return Err(AppError::BadRequest(
            "task_statuses must contain at most 4 values".to_string(),
        ));
    }
    let memory_types = query.filters.memory_types.iter().collect::<HashSet<_>>();
    let task_statuses = query.filters.task_statuses.iter().collect::<HashSet<_>>();
    if memory_types.len() != query.filters.memory_types.len()
        || task_statuses.len() != query.filters.task_statuses.len()
    {
        return Err(AppError::BadRequest(
            "memory query filter values must be unique".to_string(),
        ));
    }
    if query
        .filters
        .start_time
        .zip(query.filters.end_time)
        .is_some_and(|(start, end)| start > end)
    {
        return Err(AppError::BadRequest(
            "start_time must not be later than end_time".to_string(),
        ));
    }
    Ok(())
}

fn validate_response(
    response: &MemoryQueryResponse,
    supporting_records_requested: bool,
) -> Result<(), AppError> {
    let answer_chars = response.answer.trim().chars().count();
    if !(1..=4_000).contains(&answer_chars) || response.citation_ids.len() > 8 {
        return Err(AppError::BadGateway(
            "invalid memory service response".to_string(),
        ));
    }
    let citation_ids = response
        .citation_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    if citation_ids.len() != response.citation_ids.len()
        || (response.insufficient_evidence && !response.citation_ids.is_empty())
        || (!response.insufficient_evidence && response.citation_ids.is_empty())
    {
        return Err(AppError::BadGateway(
            "invalid memory service citations".to_string(),
        ));
    }
    match (&response.supporting_records, supporting_records_requested) {
        (None, false) => {}
        (Some(records), true) => {
            let record_ids = records
                .iter()
                .filter_map(|record| record.get("memory_id").and_then(Value::as_str))
                .collect::<HashSet<_>>();
            if record_ids.len() != records.len() || record_ids != citation_ids {
                return Err(AppError::BadGateway(
                    "invalid memory supporting records".to_string(),
                ));
            }
        }
        _ => {
            return Err(AppError::BadGateway(
                "invalid memory supporting-record mode".to_string(),
            ));
        }
    }
    Ok(())
}

fn map_upstream_error(error: reqwest::Error) -> AppError {
    if error.is_timeout() {
        AppError::GatewayTimeout("memory query timed out".to_string())
    } else {
        AppError::BadGateway("memory service unavailable".to_string())
    }
}

fn default_memory_types() -> Vec<MemoryType> {
    vec![MemoryType::Episode, MemoryType::Fact, MemoryType::Task]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_contract_rejects_scope_fields() {
        let result = serde_json::from_value::<MemoryQueryRequest>(serde_json::json!({
            "question": "What happened?",
            "org_id": "org_attacker"
        }));
        assert!(result.is_err());
    }

    #[test]
    fn validates_supported_and_insufficient_responses() {
        let supported = MemoryQueryResponse {
            answer: "OAuth was blocked.".to_string(),
            insufficient_evidence: false,
            citation_ids: vec!["fct_1".to_string()],
            supporting_records: None,
        };
        assert!(validate_response(&supported, false).is_ok());

        let insufficient = MemoryQueryResponse {
            answer: "There is not enough evidence.".to_string(),
            insufficient_evidence: true,
            citation_ids: Vec::new(),
            supporting_records: Some(Vec::new()),
        };
        assert!(validate_response(&insufficient, true).is_ok());
    }

    #[tokio::test]
    async fn rate_limiter_bounds_each_user() {
        let limiter = MemoryQueryRateLimiter::new(2);
        assert!(limiter.allow("user_1").await);
        assert!(limiter.allow("user_1").await);
        assert!(!limiter.allow("user_1").await);
        assert!(limiter.allow("user_2").await);
    }
}
