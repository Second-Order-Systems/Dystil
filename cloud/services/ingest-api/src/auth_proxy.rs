use axum::body::{Body, Bytes};
use axum::extract::{OriginalUri, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Response, StatusCode};
use axum::response::IntoResponse;

use crate::{AppError, AppState};

const HOP_BY_HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
];

pub async fn proxy_auth_fallback(
    State(state): State<AppState>,
    OriginalUri(original_uri): OriginalUri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response<Body>, AppError> {
    let path = original_uri
        .path()
        .strip_prefix("/api/auth")
        .unwrap_or("")
        .trim_start_matches('/')
        .to_string();
    proxy_auth_path(state, path, original_uri, method, headers, body).await
}

async fn proxy_auth_path(
    state: AppState,
    path: String,
    original_uri: axum::http::Uri,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response<Body>, AppError> {
    let auth_internal_url = state
        .config
        .auth_internal_url
        .as_deref()
        .ok_or_else(|| AppError::Internal("AUTH_INTERNAL_URL is not configured".to_string()))?;

    let query = original_uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let target = if path.is_empty() {
        format!(
            "{}/api/auth{}",
            auth_internal_url.trim_end_matches('/'),
            query
        )
    } else {
        format!(
            "{}/api/auth/{}{}",
            auth_internal_url.trim_end_matches('/'),
            path,
            query
        )
    };

    let mut request = state.http_no_redirect.request(method, target);
    for (name, value) in headers.iter() {
        if should_forward_request_header(name) {
            request = request.header(name, value);
        }
    }

    let upstream = request
        .body(body)
        .send()
        .await
        .map_err(|err| AppError::Internal(format!("auth proxy request failed: {err}")))?;

    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|err| AppError::Internal(format!("auth proxy status invalid: {err}")))?;
    let response_headers = upstream.headers().clone();
    let body = upstream
        .bytes()
        .await
        .map_err(|err| AppError::Internal(format!("auth proxy response failed: {err}")))?;

    let mut response = Response::builder()
        .status(status)
        .body(Body::from(body))
        .map_err(|err| AppError::Internal(format!("auth proxy response invalid: {err}")))?;

    for (name, value) in response_headers.iter() {
        if should_forward_response_header(name) {
            append_response_header(response.headers_mut(), name, value)?;
        }
    }

    Ok(response)
}

pub async fn auth_health(State(state): State<AppState>) -> impl IntoResponse {
    let Some(auth_internal_url) = state.config.auth_internal_url.as_deref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "ok": false,
                "service": "better-auth",
                "error": "AUTH_INTERNAL_URL is not configured"
            })),
        );
    };

    match state
        .http
        .get(format!(
            "{}/health",
            auth_internal_url.trim_end_matches('/')
        ))
        .send()
        .await
    {
        Ok(response) if response.status().is_success() => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok": true,
                "service": "better-auth"
            })),
        ),
        Ok(response) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "ok": false,
                "service": "better-auth",
                "status": response.status().as_u16()
            })),
        ),
        Err(err) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "ok": false,
                "service": "better-auth",
                "error": err.to_string()
            })),
        ),
    }
}

fn should_forward_request_header(name: &HeaderName) -> bool {
    let lower = name.as_str().to_ascii_lowercase();
    lower != header::HOST.as_str()
        && lower != header::CONTENT_LENGTH.as_str()
        && !HOP_BY_HOP_HEADERS.contains(&lower.as_str())
}

fn should_forward_response_header(name: &HeaderName) -> bool {
    let lower = name.as_str().to_ascii_lowercase();
    lower != header::CONTENT_LENGTH.as_str() && !HOP_BY_HOP_HEADERS.contains(&lower.as_str())
}

fn append_response_header(
    headers: &mut HeaderMap,
    name: &HeaderName,
    value: &HeaderValue,
) -> Result<(), AppError> {
    let name = HeaderName::from_bytes(name.as_str().as_bytes())
        .map_err(|err| AppError::Internal(format!("auth proxy header invalid: {err}")))?;
    let value = HeaderValue::from_bytes(value.as_bytes())
        .map_err(|err| AppError::Internal(format!("auth proxy header value invalid: {err}")))?;
    headers.append(name, value);
    Ok(())
}
