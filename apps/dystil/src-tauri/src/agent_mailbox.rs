//! Local state and HTTP transport for the teammate-agent POC.
//!
//! The cloud is only a short-lived mailbox. This module persists a local copy
//! of exchanged derived messages so the UI survives cloud expiry/reconnects.

use dystil_protocol::agent_mailbox::{
    AgentMessage, AgentMessageInput, AgentMessagePayload, AgentMessagesResponse,
    AgentPeersResponse, AGENT_MAILBOX_CAPABILITY, AGENT_MAILBOX_SCHEMA_VERSION,
};
use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::auth;

fn headers(device_token: &str) -> Result<HeaderMap, String> {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Device {device_token}"))
            .map_err(|error| error.to_string())?,
    );
    headers.insert(
        "x-dystil-sync-capabilities",
        HeaderValue::from_static(AGENT_MAILBOX_CAPABILITY),
    );
    Ok(headers)
}

pub(crate) async fn cloud_client() -> Result<(reqwest::Client, String, String), String> {
    let token = auth::current_device_token()
        .await?
        .ok_or_else(|| "Dystil device is not registered".to_string())?;
    let base = auth::cloud_base_url()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .default_headers(headers(&token)?)
        .build()
        .map_err(|error| error.to_string())?;
    Ok((client, base, token))
}

pub(crate) async fn list_peers() -> Result<AgentPeersResponse, String> {
    let (client, base, _) = cloud_client().await?;
    let response = client
        .get(format!("{base}/v1/agent/peers"))
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response_json(response).await
}

pub(crate) async fn send(input: &AgentMessageInput) -> Result<AgentMessage, String> {
    let (client, base, _) = cloud_client().await?;
    let response = client
        .post(format!("{base}/v1/agent/messages"))
        .json(input)
        .send()
        .await
        .map_err(|error| error.to_string())?;
    response_json(response).await
}

pub(crate) async fn sync(pool: &SqlitePool) -> Result<Vec<AgentMessage>, String> {
    let cursor: i64 = sqlx::query_scalar("SELECT cursor FROM agent_mailbox_state WHERE id = 1")
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?;
    let (client, base, _) = cloud_client().await?;
    let response = client
        .get(format!("{base}/v1/agent/messages"))
        .query(&[("after", cursor.to_string()), ("limit", "100".to_string())])
        .send()
        .await
        .map_err(|error| error.to_string())?;
    let response: AgentMessagesResponse = response_json(response).await?;
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    for message in &response.messages {
        persist_message(&mut *tx, message, "received").await?;
    }
    sqlx::query(
        "UPDATE agent_mailbox_state SET cursor = ?1, updated_at = datetime('now') WHERE id = 1",
    )
    .bind(response.next_cursor)
    .execute(&mut *tx)
    .await
    .map_err(|error| error.to_string())?;
    tx.commit().await.map_err(|error| error.to_string())?;
    Ok(response.messages)
}

pub(crate) async fn persist_outgoing(
    pool: &SqlitePool,
    message: &AgentMessage,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|error| error.to_string())?;
    persist_message(&mut *tx, message, "sent").await?;
    tx.commit().await.map_err(|error| error.to_string())
}

async fn persist_message(
    executor: &mut sqlx::SqliteConnection,
    message: &AgentMessage,
    status: &str,
) -> Result<(), String> {
    let direction = if status == "sent" {
        "outgoing"
    } else {
        "incoming"
    };
    let payload = serde_json::to_string(message).map_err(|error| error.to_string())?;
    sqlx::query(
        "INSERT INTO agent_messages (
            message_id, conversation_id, sequence_id, peer_user_id, direction, kind,
            local_status, payload_json, created_at
         ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
         ON CONFLICT(message_id) DO NOTHING",
    )
    .bind(&message.message_id)
    .bind(&message.conversation_id)
    .bind(message.sequence_id)
    .bind(if direction == "outgoing" {
        &message.recipient_user_id
    } else {
        &message.sender_user_id
    })
    .bind(direction)
    .bind(message.payload.kind().as_str())
    .bind(status)
    .bind(payload)
    .bind(&message.created_at)
    .execute(executor)
    .await
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) async fn pending_requests(pool: &SqlitePool) -> Result<Vec<AgentMessage>, String> {
    let rows = sqlx::query(
        "SELECT payload_json FROM agent_messages
         WHERE direction = 'incoming' AND kind = 'request'
           AND local_status IN ('received', 'interrupted')
         ORDER BY sequence_id ASC LIMIT 10",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| {
            serde_json::from_str::<AgentMessage>(&row.get::<String, _>("payload_json"))
                .map_err(|error| error.to_string())
        })
        .collect()
}

pub(crate) async fn set_local_status(
    pool: &SqlitePool,
    message_id: &str,
    status: &str,
) -> Result<(), String> {
    sqlx::query("UPDATE agent_messages SET local_status = ?1, updated_at = datetime('now') WHERE message_id = ?2")
        .bind(status)
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

pub(crate) fn new_request(recipient_user_id: String, question: String) -> AgentMessageInput {
    AgentMessageInput {
        schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
        message_id: Uuid::new_v4().to_string(),
        conversation_id: Uuid::new_v4().to_string(),
        recipient_user_id: Some(recipient_user_id),
        in_reply_to: None,
        turn_index: 0,
        payload: AgentMessagePayload::Request(dystil_protocol::agent_mailbox::AgentRequestBody {
            question,
            search: dystil_protocol::agent_mailbox::AgentSearchScope {
                lookback_days: 30,
                max_evidence_results: 12,
            },
        }),
    }
}

pub(crate) fn new_reply(
    original: &AgentMessage,
    payload: AgentMessagePayload,
) -> AgentMessageInput {
    AgentMessageInput {
        schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
        message_id: Uuid::new_v4().to_string(),
        conversation_id: original.conversation_id.clone(),
        recipient_user_id: None,
        in_reply_to: Some(original.message_id.clone()),
        turn_index: 1,
        payload,
    }
}

async fn response_json<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();
    let bytes = response.bytes().await.map_err(|error| error.to_string())?;
    if !status.is_success() {
        let error = serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .unwrap_or_else(|| format!("cloud agent request failed with {status}"));
        return Err(error);
    }
    serde_json::from_slice(&bytes).map_err(|error| error.to_string())
}

pub(crate) fn websocket_url(base: &str) -> Result<url::Url, String> {
    let mut url = url::Url::parse(base).map_err(|error| error.to_string())?;
    match url.scheme() {
        "https" => {
            url.set_scheme("wss")
                .map_err(|_| "invalid cloud WebSocket URL".to_string())?;
        }
        "http" => {
            url.set_scheme("ws")
                .map_err(|_| "invalid cloud WebSocket URL".to_string())?;
        }
        _ => return Err("cloud URL must use http or https".into()),
    }
    url.set_path("/v1/agent/ws");
    url.set_query(None);
    Ok(url)
}

pub(crate) fn websocket_authorization(device_token: &str) -> String {
    format!("Device {device_token}")
}
