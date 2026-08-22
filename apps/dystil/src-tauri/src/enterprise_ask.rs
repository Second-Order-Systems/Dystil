//! Cloud-backed Ask-for-fix transport for the enterprise client.
//!
//! The desktop has no enterprise AI key. Its registered device credential
//! authenticates requests to the cloud conversation service.

use dystil_insights::{AskSessionView, AskUserTurn};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::auth;

fn headers(device_token: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Device {device_token}"))
            .map_err(|error| error.to_string())?,
    );
    Ok(headers)
}

async fn client() -> Result<(reqwest::Client, String), String> {
    crate::app_policy::require_cloud_product()?;
    let device_token = auth::current_device_token()
        .await?
        .ok_or_else(|| "Sign in to your organization before asking for a fix.".to_string())?;
    let base = auth::cloud_base_url()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(90))
        .default_headers(headers(&device_token)?)
        .build()
        .map_err(|error| error.to_string())?;
    Ok((client, base))
}

async fn response_json<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, String> {
    let status = response.status();
    let body = response.text().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let detail = serde_json::from_str::<serde_json::Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            .unwrap_or(body);
        return Err(if status == reqwest::StatusCode::UNAUTHORIZED {
            "Your organization sign-in has expired. Sign in again, then retry.".to_string()
        } else {
            detail
        });
    }
    serde_json::from_str(&body)
        .map_err(|error| format!("Cloud returned an invalid Ask response: {error}"))
}

async fn post_json<T: DeserializeOwned, B: Serialize>(path: &str, body: &B) -> Result<T, String> {
    let (client, base) = client().await?;
    response_json(
        client
            .post(format!("{base}{path}"))
            .json(body)
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await
}

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let (client, base) = client().await?;
    response_json(
        client
            .get(format!("{base}{path}"))
            .send()
            .await
            .map_err(|error| error.to_string())?,
    )
    .await
}

pub(crate) async fn latest() -> Result<Option<AskSessionView>, String> {
    get_json("/v1/ask/conversations/latest").await
}

pub(crate) async fn get(session_id: &str) -> Result<AskSessionView, String> {
    get_json(&format!("/v1/ask/conversations/{session_id}")).await
}

pub(crate) async fn create() -> Result<AskSessionView, String> {
    post_json("/v1/ask/conversations", &serde_json::json!({})).await
}

pub(crate) async fn submit(session_id: &str, turn: AskUserTurn) -> Result<AskSessionView, String> {
    post_json(
        &format!("/v1/ask/conversations/{session_id}/messages"),
        &serde_json::json!({ "text": turn.text }),
    )
    .await
}

pub(crate) async fn finalize(
    session_id: &str,
    summary: Option<String>,
) -> Result<AskSessionView, String> {
    post_json(
        &format!("/v1/ask/conversations/{session_id}/finalize"),
        &serde_json::json!({ "summary": summary }),
    )
    .await
}
