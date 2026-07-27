use chrono::{DateTime, Duration, Utc};
use dystil_protocol::agent_mailbox::{
    AgentMessage, AgentMessageInput, AgentMessageKind, AgentMessagePayload, AgentPeer,
    AGENT_MAILBOX_CAPABILITY, AGENT_MAILBOX_SCHEMA_VERSION,
};
use serde_json::Value;
use sqlx::{FromRow, PgPool};

use crate::{DbError, Principal};

#[derive(Debug, Clone, FromRow)]
struct MessageRow {
    sequence_id: i64,
    message_id: String,
    conversation_id: String,
    sender_user_id: String,
    sender_device_id: String,
    recipient_user_id: String,
    recipient_device_id: String,
    in_reply_to: Option<String>,
    turn_index: i16,
    kind: String,
    body_json: Value,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, FromRow)]
struct PeerRow {
    user_id: String,
    display_name: Option<String>,
    email: String,
    last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct ResolvedAgentDevice {
    pub user_id: String,
    pub device_id: String,
}

pub async fn list_peers(pool: &PgPool, principal: &Principal) -> Result<Vec<AgentPeer>, DbError> {
    let rows = sqlx::query_as::<_, PeerRow>(
        "SELECT au.id AS user_id, au.display_name, au.email, MAX(d.last_seen_at) AS last_seen_at
         FROM app_users au
         JOIN devices d ON d.user_id = au.id
          AND d.org_id = au.org_id
          AND d.revoked_at IS NULL
          AND COALESCE(d.sync_capabilities, ARRAY[]::TEXT[]) @> ARRAY[$3]::TEXT[]
         WHERE au.org_id = $1 AND au.id <> $2
         GROUP BY au.id, au.display_name, au.email
         ORDER BY COALESCE(au.display_name, au.email), au.id",
    )
    .bind(&principal.org_id)
    .bind(&principal.user_id)
    .bind(AGENT_MAILBOX_CAPABILITY)
    .fetch_all(pool)
    .await?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|row| AgentPeer {
            user_id: row.user_id,
            display_name: row.display_name,
            email: row.email,
            agent_status: match row.last_seen_at {
                Some(last_seen) if now - last_seen <= Duration::minutes(2) => {
                    "available_recently".to_string()
                }
                _ => "offline".to_string(),
            },
        })
        .collect())
}

pub async fn resolve_recipient_device(
    pool: &PgPool,
    principal: &Principal,
    recipient_user_id: &str,
) -> Result<Option<ResolvedAgentDevice>, DbError> {
    #[derive(FromRow)]
    struct Row {
        user_id: String,
        device_id: String,
    }
    let row = sqlx::query_as::<_, Row>(
        "SELECT au.id AS user_id, d.id AS device_id
         FROM app_users au
         JOIN devices d ON d.user_id = au.id
          AND d.org_id = au.org_id
          AND d.revoked_at IS NULL
          AND COALESCE(d.sync_capabilities, ARRAY[]::TEXT[]) @> ARRAY[$3]::TEXT[]
         WHERE au.org_id = $1 AND au.id = $2
         ORDER BY d.last_seen_at DESC NULLS LAST, d.created_at DESC
         LIMIT 1",
    )
    .bind(&principal.org_id)
    .bind(recipient_user_id)
    .bind(AGENT_MAILBOX_CAPABILITY)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| ResolvedAgentDevice {
        user_id: row.user_id,
        device_id: row.device_id,
    }))
}

pub async fn request_rate_limit_exceeded(
    pool: &PgPool,
    principal: &Principal,
    recipient_user_id: &str,
) -> Result<bool, DbError> {
    // Incoming teammate requests automatically spend the recipient's AI quota and
    // compute without an approval step. These caps are a safety boundary against
    // accidental loops, buggy clients, and abusive senders; do not remove them as
    // ordinary API throttling.
    let pair_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_messages
         WHERE kind = 'request' AND sender_user_id = $1 AND recipient_user_id = $2
           AND created_at >= now() - interval '1 day'",
    )
    .bind(&principal.user_id)
    .bind(recipient_user_id)
    .fetch_one(pool)
    .await?;
    if pair_count >= 20 {
        return Ok(true);
    }
    let recipient_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_messages
         WHERE kind = 'request' AND recipient_user_id = $1
           AND created_at >= now() - interval '1 day'",
    )
    .bind(recipient_user_id)
    .fetch_one(pool)
    .await?;
    Ok(recipient_count >= 50)
}

pub async fn insert_message(
    pool: &PgPool,
    principal: &Principal,
    input: &AgentMessageInput,
) -> Result<AgentMessage, DbError> {
    input.validate().map_err(DbError::Other)?;
    if let Some(existing) = idempotent_message(pool, principal, input).await? {
        return Ok(existing);
    }
    let kind = input.payload.kind();
    let (recipient_user_id, recipient_device_id) = match &kind {
        AgentMessageKind::Request => {
            let requested = input
                .recipient_user_id
                .as_deref()
                .expect("validated request");
            if requested == principal.user_id {
                return Err(DbError::Other("cannot message yourself".into()));
            }
            let target = resolve_recipient_device(pool, principal, requested)
                .await?
                .ok_or_else(|| {
                    DbError::Other("recipient does not have a compatible Dystil device".into())
                })?;
            (target.user_id, target.device_id)
        }
        AgentMessageKind::Status | AgentMessageKind::Response | AgentMessageKind::Error => {
            let original =
                get_message_by_id(pool, input.in_reply_to.as_deref().expect("validated reply"))
                    .await?
                    .ok_or_else(|| DbError::Other("original request not found".into()))?;
            if original.kind != "request"
                || original.org_id != principal.org_id
                || original.conversation_id != input.conversation_id
                || original.recipient_user_id != principal.user_id
                || original.recipient_device_id != principal.device_id
            {
                return Err(DbError::Other(
                    "reply does not belong to this device request".into(),
                ));
            }
            if kind != AgentMessageKind::Status {
                let existing: Option<i64> = sqlx::query_scalar(
                    "SELECT sequence_id FROM agent_messages
                     WHERE in_reply_to = $1 AND kind IN ('response', 'error') LIMIT 1",
                )
                .bind(&original.message_id)
                .fetch_optional(pool)
                .await?;
                if existing.is_some() {
                    return Err(DbError::Other(
                        "request already has a terminal response".into(),
                    ));
                }
            }
            (original.sender_user_id, original.sender_device_id)
        }
    };

    let body_json = serde_json::to_value(&input.payload)?;
    let expires_at = Utc::now() + Duration::hours(24);
    let result = sqlx::query_as::<_, MessageRow>(
        "INSERT INTO agent_messages (
            message_id, conversation_id, org_id, sender_user_id, sender_device_id,
            recipient_user_id, recipient_device_id, in_reply_to, kind, turn_index,
            body_json, expires_at
         ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
         ON CONFLICT (message_id) DO NOTHING
         RETURNING sequence_id, message_id, conversation_id, sender_user_id, sender_device_id,
                   recipient_user_id, recipient_device_id, in_reply_to, turn_index, kind,
                   body_json, created_at, expires_at",
    )
    .bind(&input.message_id)
    .bind(&input.conversation_id)
    .bind(&principal.org_id)
    .bind(&principal.user_id)
    .bind(&principal.device_id)
    .bind(&recipient_user_id)
    .bind(&recipient_device_id)
    .bind(&input.in_reply_to)
    .bind(kind.as_str())
    .bind(i16::from(input.turn_index))
    .bind(body_json)
    .bind(expires_at)
    .fetch_optional(pool)
    .await?;

    match result {
        Some(row) => row_to_message(row),
        None => {
            let existing = get_message_by_id(pool, &input.message_id)
                .await?
                .ok_or_else(|| DbError::Other("message insert did not persist".into()))?;
            match_matching_message(existing, principal, input)
        }
    }
}

/// Returns the prior message only for an exact retry by the original device.
///
/// A message id is an idempotency key, not a mutable client record.  In
/// particular, accepting a changed body here could turn a network retry into a
/// different question or a second visible answer.
pub async fn idempotent_message(
    pool: &PgPool,
    principal: &Principal,
    input: &AgentMessageInput,
) -> Result<Option<AgentMessage>, DbError> {
    let Some(existing) = get_message_by_id(pool, &input.message_id).await? else {
        return Ok(None);
    };
    Ok(Some(match_matching_message(existing, principal, input)?))
}

fn match_matching_message(
    existing: FullMessageRow,
    principal: &Principal,
    input: &AgentMessageInput,
) -> Result<AgentMessage, DbError> {
    let body_json = serde_json::to_value(&input.payload)?;
    let is_exact_retry = existing.org_id == principal.org_id
        && existing.sender_user_id == principal.user_id
        && existing.sender_device_id == principal.device_id
        && existing.conversation_id == input.conversation_id
        && existing.in_reply_to == input.in_reply_to
        && existing.turn_index == i16::from(input.turn_index)
        && existing.kind == input.payload.kind().as_str()
        && existing.body_json == body_json;
    if !is_exact_retry {
        return Err(DbError::Other(
            "message id was already used with different sender or content".into(),
        ));
    }
    row_to_message(existing)
}

pub async fn list_messages(
    pool: &PgPool,
    principal: &Principal,
    after: i64,
    limit: i64,
) -> Result<Vec<AgentMessage>, DbError> {
    let rows = sqlx::query_as::<_, MessageRow>(
        "SELECT sequence_id, message_id, conversation_id, sender_user_id, sender_device_id,
                recipient_user_id, recipient_device_id, in_reply_to, turn_index, kind,
                body_json, created_at, expires_at
         FROM agent_messages
         WHERE recipient_device_id = $1 AND sequence_id > $2 AND expires_at > now()
         ORDER BY sequence_id ASC LIMIT $3",
    )
    .bind(&principal.device_id)
    .bind(after.max(0))
    .bind(limit.clamp(1, 100))
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(row_to_message).collect()
}

pub async fn delete_expired(pool: &PgPool, limit: i64) -> Result<u64, DbError> {
    let result = sqlx::query(
        "DELETE FROM agent_messages WHERE message_id IN (
            SELECT message_id FROM agent_messages WHERE expires_at <= now()
            ORDER BY expires_at ASC LIMIT $1
         )",
    )
    .bind(limit.clamp(1, 1000))
    .execute(pool)
    .await?;
    Ok(result.rows_affected())
}

#[derive(Debug, Clone, FromRow)]
struct FullMessageRow {
    sequence_id: i64,
    message_id: String,
    conversation_id: String,
    org_id: String,
    sender_user_id: String,
    sender_device_id: String,
    recipient_user_id: String,
    recipient_device_id: String,
    in_reply_to: Option<String>,
    turn_index: i16,
    kind: String,
    body_json: Value,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

async fn get_message_by_id(
    pool: &PgPool,
    message_id: &str,
) -> Result<Option<FullMessageRow>, DbError> {
    sqlx::query_as::<_, FullMessageRow>(
        "SELECT sequence_id, message_id, conversation_id, org_id, sender_user_id, sender_device_id,
                recipient_user_id, recipient_device_id, in_reply_to, turn_index, kind,
                body_json, created_at, expires_at
         FROM agent_messages WHERE message_id = $1",
    )
    .bind(message_id)
    .fetch_optional(pool)
    .await
    .map_err(Into::into)
}

fn row_to_message(row: impl IntoMessageRow) -> Result<AgentMessage, DbError> {
    let row = row.into_message_row();
    let payload: AgentMessagePayload = serde_json::from_value(row.body_json)?;
    if payload.kind().as_str() != row.kind {
        return Err(DbError::Other(
            "stored agent message kind does not match body".into(),
        ));
    }
    Ok(AgentMessage {
        schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
        sequence_id: row.sequence_id,
        message_id: row.message_id,
        conversation_id: row.conversation_id,
        sender_user_id: row.sender_user_id,
        sender_device_id: row.sender_device_id,
        recipient_user_id: row.recipient_user_id,
        recipient_device_id: row.recipient_device_id,
        in_reply_to: row.in_reply_to,
        turn_index: row.turn_index as u8,
        created_at: row.created_at.to_rfc3339(),
        expires_at: row.expires_at.to_rfc3339(),
        payload,
    })
}

struct MessageFields {
    sequence_id: i64,
    message_id: String,
    conversation_id: String,
    sender_user_id: String,
    sender_device_id: String,
    recipient_user_id: String,
    recipient_device_id: String,
    in_reply_to: Option<String>,
    turn_index: i16,
    kind: String,
    body_json: Value,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
}

trait IntoMessageRow {
    fn into_message_row(self) -> MessageFields;
}
impl IntoMessageRow for MessageRow {
    fn into_message_row(self) -> MessageFields {
        MessageFields {
            sequence_id: self.sequence_id,
            message_id: self.message_id,
            conversation_id: self.conversation_id,
            sender_user_id: self.sender_user_id,
            sender_device_id: self.sender_device_id,
            recipient_user_id: self.recipient_user_id,
            recipient_device_id: self.recipient_device_id,
            in_reply_to: self.in_reply_to,
            turn_index: self.turn_index,
            kind: self.kind,
            body_json: self.body_json,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}
impl IntoMessageRow for FullMessageRow {
    fn into_message_row(self) -> MessageFields {
        MessageFields {
            sequence_id: self.sequence_id,
            message_id: self.message_id,
            conversation_id: self.conversation_id,
            sender_user_id: self.sender_user_id,
            sender_device_id: self.sender_device_id,
            recipient_user_id: self.recipient_user_id,
            recipient_device_id: self.recipient_device_id,
            in_reply_to: self.in_reply_to,
            turn_index: self.turn_index,
            kind: self.kind,
            body_json: self.body_json,
            created_at: self.created_at,
            expires_at: self.expires_at,
        }
    }
}
