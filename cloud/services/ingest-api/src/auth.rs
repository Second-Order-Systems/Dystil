use axum::http::{header::AUTHORIZATION, header::COOKIE, HeaderMap};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;
use work_insights_db::identity::{
    resolve_active_device, resolve_app_identity, resolve_dashboard_identity,
    update_device_client_metadata, AppIdentity, AuthenticatedUser, DeviceRecord,
};
use work_insights_db::Principal;

use crate::{AppError, AppState};

#[derive(Debug, Deserialize)]
struct BetterAuthSessionResponse {
    user: BetterAuthUserResponse,
}

#[derive(Debug, Deserialize)]
struct BetterAuthUserResponse {
    id: String,
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    user_metadata: Option<Value>,
}

pub async fn authenticate_app_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<AppIdentity, AppError> {
    let token = bearer_token(headers)?;
    let user = fetch_better_auth_user(&state.http, &state.config, token).await?;
    let identity = resolve_app_identity(&state.pool, &user).await?;
    Ok(identity)
}

pub async fn resolve_dashboard_session(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Option<work_insights_db::identity::DashboardIdentity>, AppError> {
    let user = fetch_better_auth_user_from_cookie(&state.http, &state.config, headers).await?;
    match user {
        None => Ok(None),
        Some(user) => resolve_dashboard_identity(&state.pool, &user)
            .await
            .map_err(AppError::from),
    }
}

async fn fetch_better_auth_user_from_cookie(
    http: &Client,
    config: &crate::Config,
    headers: &HeaderMap,
) -> Result<Option<AuthenticatedUser>, AppError> {
    let auth_internal_url = config
        .auth_internal_url
        .as_deref()
        .ok_or_else(|| AppError::Internal("AUTH_INTERNAL_URL is not configured".to_string()))?;

    let cookie = headers
        .get(COOKIE)
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;

    let response = http
        .get(format!(
            "{}/api/auth/get-session",
            auth_internal_url.trim_end_matches('/')
        ))
        .header(COOKIE, cookie)
        .send()
        .await
        .map_err(|err| AppError::Internal(format!("better auth request failed: {err}")))?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Ok(None);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "better auth returned {}: {}",
            status, body
        )));
    }

    let session: BetterAuthSessionResponse = response
        .json()
        .await
        .map_err(|err| AppError::Internal(format!("better auth payload invalid: {err}")))?;

    let display_name = display_name_from_user(&session.user);
    let email = session.user.email.ok_or_else(|| {
        AppError::BadRequest("better auth user is missing an email address".to_string())
    })?;
    Ok(Some(AuthenticatedUser {
        user_id: session.user.id,
        email,
        display_name,
    }))
}

pub async fn authenticate_device(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Principal, AppError> {
    let token = device_token(headers)?;
    if let Some(device) = resolve_active_device(&state.pool, token).await? {
        let app_version = bounded_header(headers, "x-dystil-app-version", 64);
        let build_channel = bounded_header(headers, "x-dystil-build-channel", 32);
        let build_commit = bounded_header(headers, "x-dystil-build-commit", 128);
        let capabilities =
            bounded_header(headers, "x-dystil-sync-capabilities", 1024).map(|value| {
                value
                    .split(',')
                    .filter_map(|item| {
                        let item = item.trim();
                        (!item.is_empty() && item.len() <= 64).then(|| item.to_string())
                    })
                    .collect::<Vec<_>>()
            });
        update_device_client_metadata(
            &state.pool,
            &device.device_id,
            app_version.as_deref(),
            build_channel.as_deref(),
            build_commit.as_deref(),
            capabilities.as_deref(),
        )
        .await?;
        tracing::info!(
            device_id = %device.device_id,
            org_id = %device.org_id,
            user_id = %device.user_id,
            "ingest-api: device token accepted"
        );
        return Ok(principal_from_device(&device));
    }

    tracing::warn!(
        device_token_fingerprint = %device_token_fingerprint(token),
        "ingest-api: device token rejected"
    );
    Err(AppError::Unauthorized)
}

fn bounded_header(headers: &HeaderMap, name: &'static str, max_len: usize) -> Option<String> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= max_len)
        .map(ToOwned::to_owned)
}

async fn fetch_better_auth_user(
    http: &Client,
    config: &crate::Config,
    token: &str,
) -> Result<AuthenticatedUser, AppError> {
    let auth_internal_url = config
        .auth_internal_url
        .as_deref()
        .ok_or_else(|| AppError::Internal("AUTH_INTERNAL_URL is not configured".to_string()))?;

    let response = http
        .get(format!(
            "{}/api/auth/get-session",
            auth_internal_url.trim_end_matches('/')
        ))
        .header(AUTHORIZATION, format!("Bearer {}", token))
        .send()
        .await
        .map_err(|err| AppError::Internal(format!("better auth request failed: {err}")))?;

    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Unauthorized);
    }

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Internal(format!(
            "better auth returned {}: {}",
            status, body
        )));
    }

    let session: BetterAuthSessionResponse = response
        .json()
        .await
        .map_err(|err| AppError::Internal(format!("better auth payload invalid: {err}")))?;
    let display_name = display_name_from_user(&session.user);
    let email = session.user.email.ok_or_else(|| {
        AppError::BadRequest("better auth user is missing an email address".to_string())
    })?;
    Ok(AuthenticatedUser {
        user_id: session.user.id,
        email,
        display_name,
    })
}

fn display_name_from_user(user: &BetterAuthUserResponse) -> Option<String> {
    if let Some(metadata) = user.user_metadata.as_ref() {
        if let Some(display_name) = extract_display_name(Some(metadata)) {
            return Some(display_name);
        }
    }

    if let Some(name) = user.name.as_ref() {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }

    None
}

fn extract_display_name(metadata: Option<&Value>) -> Option<String> {
    let metadata = metadata?;
    for key in ["display_name", "full_name", "name"] {
        if let Some(value) = metadata.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

pub(crate) fn bearer_token(headers: &HeaderMap) -> Result<&str, AppError> {
    let raw = headers
        .get(AUTHORIZATION)
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;
    raw.strip_prefix("Bearer ").ok_or(AppError::Unauthorized)
}

pub(crate) fn device_token(headers: &HeaderMap) -> Result<&str, AppError> {
    let raw = headers
        .get(AUTHORIZATION)
        .ok_or(AppError::Unauthorized)?
        .to_str()
        .map_err(|_| AppError::Unauthorized)?;
    raw.strip_prefix("Device ").ok_or(AppError::Unauthorized)
}

fn principal_from_device(device: &DeviceRecord) -> Principal {
    Principal {
        org_id: device.org_id.clone(),
        user_id: device.user_id.clone(),
        device_id: device.device_id.clone(),
    }
}

fn device_token_fingerprint(token: &str) -> String {
    let prefix: String = token.chars().take(12).collect();
    if token.len() <= 12 {
        prefix
    } else {
        format!("{prefix}...")
    }
}

#[cfg(test)]
mod tests {
    use super::extract_display_name;
    use serde_json::json;

    #[test]
    fn extracts_display_name_with_fallback_order() {
        assert_eq!(
            extract_display_name(Some(&json!({"full_name": "Ada Lovelace"}))),
            Some("Ada Lovelace".to_string())
        );
        assert_eq!(
            extract_display_name(Some(&json!({"name": "Grace Hopper"}))),
            Some("Grace Hopper".to_string())
        );
        assert_eq!(extract_display_name(Some(&json!({}))), None);
    }
}
