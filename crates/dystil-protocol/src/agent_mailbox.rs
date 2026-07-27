//! Shared, bounded wire types for Dystil's teammate-agent mailbox.
//!
//! These messages deliberately contain only a question, progress state, or a
//! derived answer. Work cards and raw capture evidence never cross this API.

use serde::{Deserialize, Serialize};

pub const AGENT_MAILBOX_SCHEMA_VERSION: &str = "dystil-agent-message-v1";
pub const AGENT_MAILBOX_CAPABILITY: &str = "agent_mailbox_v1";
pub const MAX_AGENT_QUESTION_BYTES: usize = 2_000;
pub const MAX_AGENT_ANSWER_BYTES: usize = 12_000;
pub const MAX_AGENT_BODY_BYTES: usize = 24 * 1024;
pub const MAX_AGENT_EVIDENCE: usize = 10;
pub const MAX_AGENT_LOOKBACK_DAYS: u16 = 90;
pub const MAX_AGENT_CANDIDATE_CARDS: u8 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentMessageKind {
    Request,
    Status,
    Response,
    Error,
}

impl AgentMessageKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Request => "request",
            Self::Status => "status",
            Self::Response => "response",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentStage {
    Delivered,
    Searching,
    Generating,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentSearchScope {
    pub lookback_days: u16,
    pub max_cards: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentRequestBody {
    pub question: String,
    pub search: AgentSearchScope,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentStatusBody {
    pub stage: AgentStage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentEvidenceLabel {
    pub label: String,
    pub local_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentResponseBody {
    pub answer: String,
    #[serde(default)]
    pub evidence: Vec<AgentEvidenceLabel>,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentErrorBody {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "body", rename_all = "snake_case")]
pub enum AgentMessagePayload {
    Request(AgentRequestBody),
    Status(AgentStatusBody),
    Response(AgentResponseBody),
    Error(AgentErrorBody),
}

impl AgentMessagePayload {
    pub fn kind(&self) -> AgentMessageKind {
        match self {
            Self::Request(_) => AgentMessageKind::Request,
            Self::Status(_) => AgentMessageKind::Status,
            Self::Response(_) => AgentMessageKind::Response,
            Self::Error(_) => AgentMessageKind::Error,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Request(body) => {
                validate_non_empty_limited("question", &body.question, MAX_AGENT_QUESTION_BYTES)?;
                if !(1..=MAX_AGENT_LOOKBACK_DAYS).contains(&body.search.lookback_days) {
                    return Err("lookback_days is outside the supported range".into());
                }
                if !(1..=MAX_AGENT_CANDIDATE_CARDS).contains(&body.search.max_cards) {
                    return Err("max_cards is outside the supported range".into());
                }
            }
            Self::Status(_) => {}
            Self::Response(body) => {
                validate_non_empty_limited("answer", &body.answer, MAX_AGENT_ANSWER_BYTES)?;
                if body.evidence.len() > MAX_AGENT_EVIDENCE {
                    return Err("too many evidence entries".into());
                }
                for item in &body.evidence {
                    validate_non_empty_limited("evidence label", &item.label, 512)?;
                    validate_non_empty_limited("evidence local_date", &item.local_date, 32)?;
                }
            }
            Self::Error(body) => {
                validate_non_empty_limited("error code", &body.code, 80)?;
                validate_non_empty_limited("error message", &body.message, 512)?;
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|error| error.to_string())?;
        if bytes.len() > MAX_AGENT_BODY_BYTES {
            return Err("message body exceeds the supported size".into());
        }
        Ok(())
    }
}

fn validate_non_empty_limited(name: &str, value: &str, limit: usize) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{name} is required"));
    }
    if value.len() > limit {
        return Err(format!("{name} exceeds the supported size"));
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AgentMessageInput {
    pub schema_version: String,
    pub message_id: String,
    pub conversation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipient_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub turn_index: u8,
    #[serde(flatten)]
    pub payload: AgentMessagePayload,
}

impl AgentMessageInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != AGENT_MAILBOX_SCHEMA_VERSION {
            return Err("unsupported agent mailbox schema version".into());
        }
        validate_non_empty_limited("message_id", &self.message_id, 128)?;
        validate_non_empty_limited("conversation_id", &self.conversation_id, 128)?;
        self.payload.validate()?;
        match &self.payload {
            AgentMessagePayload::Request(_) => {
                if self.turn_index != 0
                    || self.recipient_user_id.as_deref().unwrap_or("").is_empty()
                {
                    return Err("request requires recipient_user_id and turn_index 0".into());
                }
                if self.in_reply_to.is_some() {
                    return Err("request must not include in_reply_to".into());
                }
            }
            _ => {
                if self.turn_index != 1 || self.in_reply_to.as_deref().unwrap_or("").is_empty() {
                    return Err("reply requires in_reply_to and turn_index 1".into());
                }
                if self.recipient_user_id.is_some() {
                    return Err("reply recipient is derived from the original request".into());
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMessage {
    pub schema_version: String,
    pub sequence_id: i64,
    pub message_id: String,
    pub conversation_id: String,
    pub sender_user_id: String,
    pub sender_device_id: String,
    pub recipient_user_id: String,
    pub recipient_device_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<String>,
    pub turn_index: u8,
    pub created_at: String,
    pub expires_at: String,
    #[serde(flatten)]
    pub payload: AgentMessagePayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentMessagesResponse {
    pub schema_version: String,
    pub messages: Vec<AgentMessage>,
    pub next_cursor: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPeer {
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub email: String,
    pub agent_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPeersResponse {
    pub schema_version: String,
    pub people: Vec<AgentPeer>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_requires_bounded_target_and_scope() {
        let input = AgentMessageInput {
            schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
            message_id: "m1".into(),
            conversation_id: "c1".into(),
            recipient_user_id: Some("u1".into()),
            in_reply_to: None,
            turn_index: 0,
            payload: AgentMessagePayload::Request(AgentRequestBody {
                question: "What happened?".into(),
                search: AgentSearchScope {
                    lookback_days: 30,
                    max_cards: 12,
                },
            }),
        };
        assert!(input.validate().is_ok());
    }

    #[test]
    fn response_cannot_set_its_own_recipient() {
        let input = AgentMessageInput {
            schema_version: AGENT_MAILBOX_SCHEMA_VERSION.into(),
            message_id: "m2".into(),
            conversation_id: "c1".into(),
            recipient_user_id: Some("u1".into()),
            in_reply_to: Some("m1".into()),
            turn_index: 1,
            payload: AgentMessagePayload::Response(AgentResponseBody {
                answer: "Found it".into(),
                evidence: vec![],
                uncertainties: vec![],
            }),
        };
        assert!(input.validate().is_err());
    }

    #[test]
    fn unknown_wire_fields_are_rejected() {
        let value = r#"{
            "schema_version":"dystil-agent-message-v1",
            "message_id":"m1",
            "conversation_id":"c1",
            "recipient_user_id":"u1",
            "turn_index":0,
            "kind":"request",
            "body":{"question":"What happened?","search":{"lookback_days":30,"max_cards":12},"unexpected":true}
        }"#;
        assert!(serde_json::from_str::<AgentMessageInput>(value).is_err());
    }
}
