//! Durable, bounded Ask-for-a-fix conversation engine.

use std::{collections::BTreeMap, time::Duration};

use chrono::Utc;
use dystil_ai::{
    AiModelTier, AiReasoningEffort, AiRuntime, AiRuntimeError, AiRuntimeErrorCode,
    AiStructuredRequest, AiToolPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use specta::Type;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use thiserror::Error;

use crate::{
    store::{fingerprint, stable_id},
    InsightsError,
};

const PROMPT_VERSION: &str = "ask_for_fix_v1";
const STABLE_PROMPT: &str = include_str!("../resources/ask_for_fix_prompt_v1.md");
const SCHEMA_JSON: &str = include_str!("../resources/ask_for_fix_schema_v1.json");
const MODEL_TIER: AiModelTier = AiModelTier::Frontier;
const MAX_QUESTIONS: u32 = 5;

#[derive(Debug, Error)]
pub enum AskForFixError {
    #[error(transparent)]
    Store(#[from] InsightsError),
    #[error(transparent)]
    Runtime(#[from] AiRuntimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid Ask-for-a-fix state: {0}")]
    InvalidState(String),
    #[error("model output remained invalid after one repair: {0}")]
    InvalidOutput(String),
}

pub type AskForFixResult<T> = std::result::Result<T, AskForFixError>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AskPhase {
    Understand,
    FollowUp,
    Consolidate,
    Present,
}

impl AskPhase {
    fn parse(value: &str) -> AskForFixResult<Self> {
        match value {
            "understand" => Ok(Self::Understand),
            "follow_up" => Ok(Self::FollowUp),
            "consolidate" => Ok(Self::Consolidate),
            "present" => Ok(Self::Present),
            other => Err(AskForFixError::InvalidState(format!(
                "unknown phase {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AskMessageRole {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AskQuestionKind {
    FreeText,
    SingleSelect,
    MultiSelect,
    Compare,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskOption {
    pub id: String,
    pub label: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskQuestion {
    pub kind: AskQuestionKind,
    pub text: String,
    pub helper: String,
    pub options: Vec<AskOption>,
    pub min_selections: u32,
    pub max_selections: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskUnderstanding {
    pub synthesis: String,
    pub grounding: Vec<String>,
    pub inferences: Vec<String>,
    pub preserved_boundary: String,
    pub uncertainty: Vec<String>,
    pub solution_target: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AskPresentationRoute {
    AnswerNow,
    SomethingNowMoreLater,
    CannotSee,
    NeedsMoreThanOnePerson,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum AskArtifactKind {
    Prompt,
    Runbook,
    ExistingCapability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskArtifact {
    pub kind: AskArtifactKind,
    pub title: String,
    pub description: String,
    pub body: String,
    pub steps: Vec<String>,
    pub tool: String,
    pub capability: String,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskPresentation {
    pub route: AskPresentationRoute,
    pub headline: String,
    pub explanation: String,
    pub limitations: Vec<String>,
    pub artifact: Option<AskArtifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AskMoveKind {
    Ask,
    Consolidate,
    Present,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AskModelMove {
    schema_version: u32,
    #[serde(rename = "move")]
    move_kind: AskMoveKind,
    assistant_message: String,
    understanding: AskUnderstanding,
    question: Option<AskQuestion>,
    presentation: Option<AskPresentation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskInputEvent {
    pub kind: String,
    pub question_id: Option<String>,
    pub selected_option_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskUserTurn {
    /// Canonical semantic message seen by the model and user.
    pub text: String,
    /// Exact UI event retained for replay and analytics.
    pub event: AskInputEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskMessageView {
    pub message_id: String,
    pub role: AskMessageRole,
    pub text: String,
    pub event: Option<AskInputEvent>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct AskSessionView {
    pub session_id: String,
    pub phase: AskPhase,
    pub status: String,
    pub question_count: u32,
    pub max_questions: u32,
    pub messages: Vec<AskMessageView>,
    pub understanding: AskUnderstanding,
    pub current_question_id: Option<String>,
    pub current_question: Option<AskQuestion>,
    pub presentation: Option<AskPresentation>,
    pub locked: bool,
    pub last_error_code: Option<String>,
    pub last_error_detail: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub cached_input_tokens: u64,
    pub artifact_kept_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
struct PromptMessage {
    role: &'static str,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct TurnPacket {
    schema_version: u32,
    phase: AskPhase,
    allowed_moves: Vec<&'static str>,
    question_count: u32,
    max_questions: u32,
    provenance_boundary: &'static str,
    current_understanding: AskUnderstanding,
    locked_understanding: Option<AskUnderstanding>,
    transcript: Vec<PromptMessage>,
    latest_event: Option<AskInputEvent>,
}

pub fn ask_for_fix_schema() -> Value {
    serde_json::from_str(SCHEMA_JSON).expect("bundled Ask-for-a-fix schema must be valid")
}

pub fn ask_for_fix_stable_prompt() -> &'static str {
    STABLE_PROMPT
}

pub fn ask_for_fix_stable_prompt_hash() -> String {
    fingerprint(&STABLE_PROMPT).expect("static prompt must fingerprint")
}

fn normalize_question(value: &str) -> String {
    value
        .chars()
        .filter(|character| character.is_alphanumeric() || character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn validate_move(
    output: AskModelMove,
    phase: AskPhase,
    question_count: u32,
    previous_questions: &[String],
) -> std::result::Result<AskModelMove, String> {
    if output.schema_version != 1 || output.assistant_message.trim().is_empty() {
        return Err("wrong schema version or empty assistant message".into());
    }
    match output.move_kind {
        AskMoveKind::Ask => {
            if phase == AskPhase::Present || question_count >= MAX_QUESTIONS {
                return Err("another question is not legal in this phase".into());
            }
            if output.presentation.is_some() {
                return Err("ask move included a presentation".into());
            }
            let question = output
                .question
                .as_ref()
                .ok_or("ask move omitted its question")?;
            let count = question.options.len();
            let valid_shape = match question.kind {
                AskQuestionKind::FreeText => {
                    count == 0 && question.min_selections == 0 && question.max_selections == 0
                }
                AskQuestionKind::SingleSelect => {
                    (2..=5).contains(&count)
                        && question.min_selections == 1
                        && question.max_selections == 1
                }
                AskQuestionKind::MultiSelect => {
                    (2..=7).contains(&count)
                        && question.min_selections >= 1
                        && question.min_selections <= question.max_selections
                        && question.max_selections as usize <= count
                }
                AskQuestionKind::Compare => {
                    count == 2 && question.min_selections == 1 && question.max_selections == 1
                }
            };
            if !valid_shape {
                return Err("question renderer constraints were violated".into());
            }
            let mut ids = question
                .options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>();
            ids.sort_unstable();
            ids.dedup();
            if ids.len() != count {
                return Err("question option IDs are not unique".into());
            }
            let normalized = normalize_question(&question.text);
            if previous_questions
                .iter()
                .any(|previous| normalize_question(previous) == normalized)
            {
                return Err("question substantially repeats an earlier question".into());
            }
        }
        AskMoveKind::Consolidate => {
            if phase == AskPhase::Present
                || output.question.is_some()
                || output.presentation.is_some()
            {
                return Err("consolidation is not legal in this shape or phase".into());
            }
            let understanding = &output.understanding;
            if understanding.synthesis.trim().is_empty()
                || understanding.grounding.is_empty()
                || understanding.preserved_boundary.trim().is_empty()
                || understanding.solution_target.trim().is_empty()
            {
                return Err("consolidation lacks a causal synthesis or required boundary".into());
            }
        }
        AskMoveKind::Present => {
            if phase != AskPhase::Present || output.question.is_some() {
                return Err("presentation was returned before confirmation".into());
            }
            let presentation = output
                .presentation
                .as_ref()
                .ok_or("present move omitted its presentation")?;
            if presentation.headline.trim().is_empty() || presentation.explanation.trim().is_empty()
            {
                return Err("presentation is empty".into());
            }
            if let Some(artifact) = &presentation.artifact {
                let valid = match artifact.kind {
                    AskArtifactKind::Prompt => !artifact.body.trim().is_empty(),
                    AskArtifactKind::Runbook => (2..=8).contains(&artifact.steps.len()),
                    AskArtifactKind::ExistingCapability => {
                        !artifact.tool.trim().is_empty()
                            && !artifact.capability.trim().is_empty()
                            && !artifact.instructions.is_empty()
                    }
                };
                if !valid {
                    return Err("artifact content does not match its kind".into());
                }
            }
        }
    }
    Ok(output)
}

async fn append_message_tx(
    tx: &mut Transaction<'_, Sqlite>,
    session_id: &str,
    role: AskMessageRole,
    text: &str,
    event: Option<&AskInputEvent>,
) -> AskForFixResult<()> {
    let ordinal = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(ordinal),0)+1 FROM ask_messages WHERE session_id=?1",
    )
    .bind(session_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(InsightsError::from)?;
    let message_id = stable_id("afm", &(session_id, ordinal, role, text))?;
    sqlx::query(
        "INSERT INTO ask_messages(message_id,session_id,ordinal,role,text,event_json,created_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7)",
    )
    .bind(message_id)
    .bind(session_id)
    .bind(ordinal)
    .bind(match role {
        AskMessageRole::User => "user",
        AskMessageRole::Assistant => "assistant",
    })
    .bind(text)
    .bind(event.map(serde_json::to_string).transpose()?)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut **tx)
    .await
    .map_err(InsightsError::from)?;
    Ok(())
}

async fn prompt_messages(
    pool: &SqlitePool,
    session_id: &str,
) -> AskForFixResult<Vec<PromptMessage>> {
    let rows =
        sqlx::query("SELECT role,text FROM ask_messages WHERE session_id=?1 ORDER BY ordinal")
            .bind(session_id)
            .fetch_all(pool)
            .await
            .map_err(InsightsError::from)?;
    Ok(rows
        .into_iter()
        .map(|row| PromptMessage {
            role: if row.get::<String, _>("role") == "user" {
                "user"
            } else {
                "assistant"
            },
            content: row.get("text"),
        })
        .collect())
}

async fn previous_questions(pool: &SqlitePool, session_id: &str) -> AskForFixResult<Vec<String>> {
    let rows = sqlx::query_scalar::<_, String>(
        "SELECT question_text FROM ask_questions WHERE session_id=?1 ORDER BY ordinal",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(InsightsError::from)?;
    Ok(rows)
}

async fn turn_packet(
    pool: &SqlitePool,
    session_id: &str,
    latest_event: Option<AskInputEvent>,
) -> AskForFixResult<TurnPacket> {
    let row = sqlx::query(
        "SELECT phase,question_count,understanding_json,locked_understanding_json
         FROM ask_sessions WHERE session_id=?1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| AskForFixError::InvalidState("session does not exist".into()))?;
    let phase = AskPhase::parse(row.get("phase"))?;
    let understanding = serde_json::from_str::<AskUnderstanding>(row.get("understanding_json"))?;
    let locked = row
        .get::<Option<String>, _>("locked_understanding_json")
        .map(|json| serde_json::from_str(&json))
        .transpose()?;
    // Retries must reconstruct the same authoritative turn packet, including
    // the raw interaction event retained beside the semantic user message.
    let latest_event = match latest_event {
        Some(event) => Some(event),
        None => {
            let event_json = sqlx::query_scalar::<_, String>(
                "SELECT event_json FROM ask_messages
                 WHERE session_id=?1 AND role='user' AND event_json IS NOT NULL
                 ORDER BY ordinal DESC LIMIT 1",
            )
            .bind(session_id)
            .fetch_optional(pool)
            .await
            .map_err(InsightsError::from)?;
            event_json
                .map(|json| serde_json::from_str(&json))
                .transpose()?
        }
    };
    Ok(TurnPacket {
        schema_version: 1,
        phase,
        allowed_moves: if phase == AskPhase::Present {
            vec!["present"]
        } else {
            vec!["ask", "consolidate"]
        },
        question_count: row.get::<i64, _>("question_count") as u32,
        max_questions: MAX_QUESTIONS,
        provenance_boundary: "user_answers_only",
        current_understanding: understanding,
        locked_understanding: locked,
        transcript: prompt_messages(pool, session_id).await?,
        latest_event,
    })
}

fn error_code(error: &AskForFixError) -> &'static str {
    match error {
        AskForFixError::Runtime(runtime) => match runtime.code {
            AiRuntimeErrorCode::NotReady => "provider_not_ready",
            AiRuntimeErrorCode::Authentication => "authentication",
            AiRuntimeErrorCode::Timeout => "timeout",
            AiRuntimeErrorCode::InvalidOutput => "invalid_output",
            AiRuntimeErrorCode::Transport => "transport",
            AiRuntimeErrorCode::Internal => "internal",
        },
        AskForFixError::InvalidOutput(_) => "invalid_output",
        AskForFixError::InvalidState(_) => "invalid_state",
        AskForFixError::Store(_) | AskForFixError::Json(_) => "internal",
    }
}

async fn record_attempt(
    pool: &SqlitePool,
    job_id: &str,
    attempt: i64,
    request_fingerprint: &str,
    output_fingerprint: Option<&str>,
    status: &str,
    usage: &BTreeMap<String, u64>,
    latency_ms: u64,
    error: Option<&str>,
) -> AskForFixResult<()> {
    sqlx::query(
        "INSERT INTO ask_attempts(job_id,attempt,request_fingerprint,output_fingerprint,status,
         usage_json,latency_ms,error_code,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
    )
    .bind(job_id)
    .bind(attempt)
    .bind(request_fingerprint)
    .bind(output_fingerprint)
    .bind(status)
    .bind(serde_json::to_string(usage)?)
    .bind(latency_ms as i64)
    .bind(error)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    Ok(())
}

async fn mark_session_error(
    pool: &SqlitePool,
    session_id: &str,
    error: &AskForFixError,
) -> AskForFixResult<()> {
    let fallback_status = sqlx::query_scalar::<_, String>(
        "SELECT CASE WHEN locked_understanding_json IS NULL THEN 'active' ELSE 'locked' END
         FROM ask_sessions WHERE session_id=?1",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "UPDATE ask_sessions SET status=?2,last_error_code=?3,last_error_detail=?4,updated_at=?5
         WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(fallback_status)
    .bind(error_code(error))
    .bind(error.to_string())
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    Ok(())
}

pub async fn set_ask_for_fix_error(
    pool: &SqlitePool,
    session_id: &str,
    code: &str,
    detail: &str,
) -> AskForFixResult<AskSessionView> {
    let fallback_status = sqlx::query_scalar::<_, String>(
        "SELECT CASE WHEN locked_understanding_json IS NULL THEN 'active' ELSE 'locked' END
         FROM ask_sessions WHERE session_id=?1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| AskForFixError::InvalidState("session does not exist".into()))?;
    sqlx::query(
        "UPDATE ask_sessions SET status=?2,last_error_code=?3,last_error_detail=?4,updated_at=?5
         WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(fallback_status)
    .bind(code)
    .bind(detail)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, session_id).await
}

async fn infer_move<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
    latest_event: Option<AskInputEvent>,
) -> AskForFixResult<AskModelMove> {
    let packet = turn_packet(pool, session_id, latest_event).await?;
    let phase = packet.phase;
    let question_count = packet.question_count;
    let previous = previous_questions(pool, session_id).await?;
    let packet_json = serde_json::to_string_pretty(&packet)?;
    let model = runtime.model_for_tier(MODEL_TIER);
    let schema = ask_for_fix_schema();
    let schema_hash = fingerprint(&schema)?;
    let stable_hash = ask_for_fix_stable_prompt_hash();
    let input_fingerprint = fingerprint(&(
        PROMPT_VERSION,
        &stable_hash,
        &schema_hash,
        &model,
        &packet_json,
    ))?;
    let job_id = stable_id("afj", &(session_id, &input_fingerprint))?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO ask_jobs(job_id,session_id,purpose,status,stable_prompt_hash,schema_hash,
         input_fingerprint,input_json,model,attempts,error_code,created_at,updated_at,accepted_at)
         VALUES(?1,?2,?3,'running',?4,?5,?6,?7,?8,0,NULL,?9,?9,NULL)
         ON CONFLICT(job_id) DO UPDATE SET status='running',error_code=NULL,updated_at=excluded.updated_at",
    )
    .bind(&job_id)
    .bind(session_id)
    .bind(if phase == AskPhase::Present {
        "ask_for_fix_present"
    } else {
        "ask_for_fix_intake"
    })
    .bind(&stable_hash)
    .bind(&schema_hash)
    .bind(&input_fingerprint)
    .bind(&packet_json)
    .bind(&model)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;

    // A provider retry reuses the same deterministic job. Continue its receipt
    // sequence instead of overwriting the attempts from the previous run.
    let prior_attempts = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(attempt),0) FROM ask_attempts WHERE job_id=?1",
    )
    .bind(&job_id)
    .fetch_one(pool)
    .await
    .map_err(InsightsError::from)?;

    let mut volatile_prompt = format!(
        "APPLICATION TURN STATE (authoritative JSON):\n{packet_json}\n\nReturn the single best legal move."
    );
    let mut last_error = String::new();
    for offset in 1..=2i64 {
        let attempt = prior_attempts + offset;
        let request_fingerprint =
            fingerprint(&(&stable_hash, &volatile_prompt, &schema_hash, &model))?;
        let run = match runtime
            .infer_structured(AiStructuredRequest {
                purpose: if phase == AskPhase::Present {
                    "ask_for_fix_present".into()
                } else {
                    "ask_for_fix_intake".into()
                },
                model_tier: MODEL_TIER,
                stable_prompt: STABLE_PROMPT.into(),
                prompt: volatile_prompt.clone(),
                output_schema: schema.clone(),
                timeout: Duration::from_secs(180),
                reasoning_effort: AiReasoningEffort::High,
                tool_policy: AiToolPolicy::None,
            })
            .await
        {
            Ok(run) => run,
            Err(error) => {
                let wrapped = AskForFixError::Runtime(error);
                record_attempt(
                    pool,
                    &job_id,
                    attempt,
                    &request_fingerprint,
                    None,
                    "provider_error",
                    &BTreeMap::new(),
                    0,
                    Some(error_code(&wrapped)),
                )
                .await?;
                sqlx::query(
                    "UPDATE ask_jobs SET status='pending',attempts=?2,error_code=?3,updated_at=?4 WHERE job_id=?1",
                )
                .bind(&job_id)
                .bind(attempt)
                .bind(error_code(&wrapped))
                .bind(Utc::now().to_rfc3339())
                .execute(pool)
                .await
                .map_err(InsightsError::from)?;
                mark_session_error(pool, session_id, &wrapped).await?;
                return Err(wrapped);
            }
        };
        let output_fingerprint = fingerprint(&run.output)?;
        let parsed = serde_json::from_value::<AskModelMove>(run.output.clone())
            .map_err(|error| error.to_string())
            .and_then(|output| validate_move(output, phase, question_count, &previous));
        match parsed {
            Ok(output) => {
                record_attempt(
                    pool,
                    &job_id,
                    attempt,
                    &request_fingerprint,
                    Some(&output_fingerprint),
                    "accepted",
                    &run.usage,
                    run.elapsed_ms,
                    None,
                )
                .await?;
                sqlx::query(
                    "UPDATE ask_jobs SET status='accepted',attempts=?2,error_code=NULL,updated_at=?3,
                     accepted_at=?3 WHERE job_id=?1",
                )
                .bind(&job_id)
                .bind(attempt)
                .bind(Utc::now().to_rfc3339())
                .execute(pool)
                .await
                .map_err(InsightsError::from)?;
                return Ok(output);
            }
            Err(error) => last_error = error,
        }
        record_attempt(
            pool,
            &job_id,
            attempt,
            &request_fingerprint,
            Some(&output_fingerprint),
            "invalid_output",
            &run.usage,
            run.elapsed_ms,
            Some("invalid_output"),
        )
        .await?;
        if offset == 1 {
            volatile_prompt = format!(
                "APPLICATION TURN STATE (authoritative JSON):\n{packet_json}\n\nYour prior object was invalid. Repair it once without changing the user's facts. Validation error: {last_error}\n\nINVALID OBJECT:\n{}",
                serde_json::to_string(&run.output).unwrap_or_else(|_| "null".into())
            );
        }
    }
    let error = AskForFixError::InvalidOutput(last_error);
    sqlx::query(
        "UPDATE ask_jobs SET status='rejected',attempts=?2,error_code='invalid_output',updated_at=?3
         WHERE job_id=?1",
    )
    .bind(&job_id)
    .bind(prior_attempts + 2)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    mark_session_error(pool, session_id, &error).await?;
    Err(error)
}

async fn apply_move(
    pool: &SqlitePool,
    session_id: &str,
    output: AskModelMove,
    provider: &str,
    model: &str,
) -> AskForFixResult<AskSessionView> {
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    let now = Utc::now().to_rfc3339();
    let mut assistant_text = output.assistant_message.trim().to_string();
    if let Some(question) = &output.question {
        assistant_text.push_str("\n\n");
        assistant_text.push_str(question.text.trim());
    }
    append_message_tx(
        &mut tx,
        session_id,
        AskMessageRole::Assistant,
        &assistant_text,
        None,
    )
    .await?;
    let understanding_json = serde_json::to_string(&output.understanding)?;
    let pending_json = serde_json::to_string(&output)?;
    match output.move_kind {
        AskMoveKind::Ask => {
            let question = output.question.as_ref().expect("validated question");
            let ordinal = sqlx::query_scalar::<_, i64>(
                "SELECT COALESCE(MAX(ordinal),0)+1 FROM ask_questions WHERE session_id=?1",
            )
            .bind(session_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(InsightsError::from)?;
            let question_id = stable_id("afq", &(session_id, ordinal, &question.text))?;
            sqlx::query(
                "INSERT INTO ask_questions(question_id,session_id,ordinal,question_text,question_json,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6)",
            )
            .bind(question_id)
            .bind(session_id)
            .bind(ordinal)
            .bind(&question.text)
            .bind(serde_json::to_string(question)?)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(InsightsError::from)?;
            sqlx::query(
                "UPDATE ask_sessions SET phase='follow_up',status='active',question_count=question_count+1,
                 understanding_json=?2,pending_move_json=?3,last_error_code=NULL,last_error_detail=NULL,
                 provider=?4,model=?5,updated_at=?6 WHERE session_id=?1",
            )
            .bind(session_id)
            .bind(understanding_json)
            .bind(pending_json)
            .bind(provider)
            .bind(model)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(InsightsError::from)?;
        }
        AskMoveKind::Consolidate => {
            sqlx::query(
                "UPDATE ask_sessions SET phase='consolidate',status='active',understanding_json=?2,
                 pending_move_json=?3,last_error_code=NULL,last_error_detail=NULL,provider=?4,model=?5,
                 updated_at=?6 WHERE session_id=?1",
            )
            .bind(session_id)
            .bind(understanding_json)
            .bind(pending_json)
            .bind(provider)
            .bind(model)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(InsightsError::from)?;
        }
        AskMoveKind::Present => {
            sqlx::query(
                "UPDATE ask_sessions SET phase='present',status='answered',understanding_json=?2,
                 presentation_json=?3,pending_move_json=NULL,last_error_code=NULL,last_error_detail=NULL,
                 provider=?4,model=?5,updated_at=?6 WHERE session_id=?1",
            )
            .bind(session_id)
            .bind(understanding_json)
            .bind(serde_json::to_string(&output.presentation)?)
            .bind(provider)
            .bind(model)
            .bind(&now)
            .execute(&mut *tx)
            .await
            .map_err(InsightsError::from)?;
        }
    }
    tx.commit().await.map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, session_id).await
}

pub async fn create_ask_for_fix_session(pool: &SqlitePool) -> AskForFixResult<AskSessionView> {
    let now = Utc::now().to_rfc3339();
    let session_id = stable_id("afs", &(now.clone(), uuid::Uuid::new_v4().to_string()))?;
    sqlx::query(
        "INSERT INTO ask_sessions(session_id,phase,status,question_count,understanding_json,
         pending_move_json,locked_understanding_json,presentation_json,last_error_code,last_error_detail,
         provider,model,artifact_kept_id,created_at,updated_at)
         VALUES(?1,'understand','active',0,?2,NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL,?3,?3)",
    )
    .bind(&session_id)
    .bind(serde_json::to_string(&AskUnderstanding::default())?)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, &session_id).await
}

pub async fn latest_ask_for_fix_session(
    pool: &SqlitePool,
) -> AskForFixResult<Option<AskSessionView>> {
    let id = sqlx::query_scalar::<_, String>(
        "SELECT session_id FROM ask_sessions ORDER BY created_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?;
    match id {
        Some(id) => Ok(Some(get_ask_for_fix_session(pool, &id).await?)),
        None => Ok(None),
    }
}

pub async fn get_ask_for_fix_session(
    pool: &SqlitePool,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    let row = sqlx::query("SELECT * FROM ask_sessions WHERE session_id=?1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(InsightsError::from)?
        .ok_or_else(|| AskForFixError::InvalidState("session does not exist".into()))?;
    let message_rows = sqlx::query(
        "SELECT message_id,role,text,event_json,created_at FROM ask_messages
         WHERE session_id=?1 ORDER BY ordinal",
    )
    .bind(session_id)
    .fetch_all(pool)
    .await
    .map_err(InsightsError::from)?;
    let messages = message_rows
        .into_iter()
        .map(|message| {
            Ok(AskMessageView {
                message_id: message.get("message_id"),
                role: if message.get::<String, _>("role") == "user" {
                    AskMessageRole::User
                } else {
                    AskMessageRole::Assistant
                },
                text: message.get("text"),
                event: message
                    .get::<Option<String>, _>("event_json")
                    .map(|json| serde_json::from_str(&json))
                    .transpose()?,
                created_at: message.get("created_at"),
            })
        })
        .collect::<std::result::Result<Vec<_>, serde_json::Error>>()?;
    let pending = row
        .get::<Option<String>, _>("pending_move_json")
        .map(|json| serde_json::from_str::<AskModelMove>(&json))
        .transpose()?;
    let presentation = row
        .get::<Option<String>, _>("presentation_json")
        .map(|json| serde_json::from_str::<Option<AskPresentation>>(&json))
        .transpose()?
        .flatten();
    let cached_input_tokens = sqlx::query_scalar::<_, String>(
        "SELECT COALESCE((SELECT usage_json FROM ask_attempts a JOIN ask_jobs j ON j.job_id=a.job_id
         WHERE j.session_id=?1 AND a.status='accepted' ORDER BY a.created_at DESC LIMIT 1),'{}')",
    )
    .bind(session_id)
    .fetch_one(pool)
    .await
    .map_err(InsightsError::from)
    .ok()
    .and_then(|json| serde_json::from_str::<BTreeMap<String, u64>>(&json).ok())
    .and_then(|usage| usage.get("cached_input_tokens").copied())
    .unwrap_or_default();
    let current_question_id = if pending
        .as_ref()
        .and_then(|move_| move_.question.as_ref())
        .is_some()
    {
        sqlx::query_scalar::<_, String>(
            "SELECT question_id FROM ask_questions WHERE session_id=?1 ORDER BY ordinal DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_optional(pool)
        .await
        .map_err(InsightsError::from)?
    } else {
        None
    };
    Ok(AskSessionView {
        session_id: row.get("session_id"),
        phase: AskPhase::parse(row.get("phase"))?,
        status: row.get("status"),
        question_count: row.get::<i64, _>("question_count") as u32,
        max_questions: MAX_QUESTIONS,
        messages,
        understanding: serde_json::from_str(row.get("understanding_json"))?,
        current_question_id,
        current_question: pending.and_then(|move_| move_.question),
        presentation,
        locked: row
            .get::<Option<String>, _>("locked_understanding_json")
            .is_some(),
        last_error_code: row.get("last_error_code"),
        last_error_detail: row.get("last_error_detail"),
        provider: row.get("provider"),
        model: row.get("model"),
        cached_input_tokens,
        artifact_kept_id: row.get("artifact_kept_id"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    })
}

pub async fn stage_ask_for_fix_turn(
    pool: &SqlitePool,
    session_id: &str,
    turn: AskUserTurn,
) -> AskForFixResult<AskSessionView> {
    let text = turn.text.trim();
    if text.is_empty() || text.chars().count() > 1600 {
        return Err(AskForFixError::InvalidState(
            "message must contain between 1 and 1600 characters".into(),
        ));
    }
    let current = get_ask_for_fix_session(pool, session_id).await?;
    if current.status == "working" || current.status == "answered" || current.locked {
        return Err(AskForFixError::InvalidState(
            "session is not accepting another answer".into(),
        ));
    }
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    append_message_tx(
        &mut tx,
        session_id,
        AskMessageRole::User,
        text,
        Some(&turn.event),
    )
    .await?;
    sqlx::query(
        "UPDATE ask_sessions SET status='working',last_error_code=NULL,last_error_detail=NULL,
         updated_at=?2 WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, session_id).await
}

pub async fn run_staged_ask_for_fix<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
    latest_event: Option<AskInputEvent>,
) -> AskForFixResult<AskSessionView> {
    let output = infer_move(pool, runtime, session_id, latest_event).await?;
    let descriptor = runtime.descriptor();
    apply_move(
        pool,
        session_id,
        output,
        &descriptor.provider_label,
        &runtime.model_for_tier(MODEL_TIER),
    )
    .await
}

pub async fn submit_ask_for_fix_turn<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
    turn: AskUserTurn,
) -> AskForFixResult<AskSessionView> {
    let event = turn.event.clone();
    stage_ask_for_fix_turn(pool, session_id, turn).await?;
    run_staged_ask_for_fix(pool, runtime, session_id, Some(event)).await
}

pub async fn lock_ask_for_fix(
    pool: &SqlitePool,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    let current = get_ask_for_fix_session(pool, session_id).await?;
    if current.phase != AskPhase::Consolidate || current.locked || current.status == "working" {
        return Err(AskForFixError::InvalidState(
            "only an unlocked consolidation can be confirmed".into(),
        ));
    }
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    append_message_tx(
        &mut tx,
        session_id,
        AskMessageRole::User,
        "Solve this.",
        Some(&AskInputEvent {
            kind: "confirm".into(),
            question_id: None,
            selected_option_ids: Vec::new(),
        }),
    )
    .await?;
    sqlx::query(
        "UPDATE ask_sessions SET phase='present',status='working',locked_understanding_json=understanding_json,
         pending_move_json=NULL,last_error_code=NULL,last_error_detail=NULL,updated_at=?2 WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, session_id).await
}

pub async fn run_locked_ask_for_fix<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    let output = infer_move(
        pool,
        runtime,
        session_id,
        Some(AskInputEvent {
            kind: "confirm".into(),
            question_id: None,
            selected_option_ids: Vec::new(),
        }),
    )
    .await?;
    let descriptor = runtime.descriptor();
    apply_move(
        pool,
        session_id,
        output,
        &descriptor.provider_label,
        &runtime.model_for_tier(MODEL_TIER),
    )
    .await
}

pub async fn confirm_ask_for_fix<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    lock_ask_for_fix(pool, session_id).await?;
    run_locked_ask_for_fix(pool, runtime, session_id).await
}

pub async fn retry_ask_for_fix<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    let current = get_ask_for_fix_session(pool, session_id).await?;
    if current.last_error_code.is_none() {
        return Err(AskForFixError::InvalidState(
            "there is no failed turn to retry".into(),
        ));
    }
    sqlx::query(
        "UPDATE ask_sessions SET status='working',last_error_code=NULL,last_error_detail=NULL,
         updated_at=?2 WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    let output = infer_move(pool, runtime, session_id, None).await?;
    let descriptor = runtime.descriptor();
    apply_move(
        pool,
        session_id,
        output,
        &descriptor.provider_label,
        &runtime.model_for_tier(MODEL_TIER),
    )
    .await
}

pub async fn cancel_ask_for_fix_turn(
    pool: &SqlitePool,
    session_id: &str,
) -> AskForFixResult<AskSessionView> {
    let status = sqlx::query_scalar::<_, String>(
        "SELECT CASE WHEN locked_understanding_json IS NULL THEN 'active' ELSE 'locked' END
         FROM ask_sessions WHERE session_id=?1",
    )
    .bind(session_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| AskForFixError::InvalidState("session does not exist".into()))?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE ask_sessions SET status=?2,last_error_code='user_cancelled',
         last_error_detail='You stopped this response. Your conversation is still here.',updated_at=?3
         WHERE session_id=?1",
    )
    .bind(session_id)
    .bind(status)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "UPDATE ask_jobs SET status='cancelled',error_code='user_cancelled',updated_at=?2
         WHERE session_id=?1 AND status='running'",
    )
    .bind(session_id)
    .bind(now)
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    get_ask_for_fix_session(pool, session_id).await
}

pub async fn keep_ask_for_fix_artifact(
    pool: &SqlitePool,
    session_id: &str,
) -> AskForFixResult<String> {
    let session = get_ask_for_fix_session(pool, session_id).await?;
    if let Some(id) = session.artifact_kept_id {
        return Ok(id);
    }
    let artifact = session
        .presentation
        .and_then(|presentation| presentation.artifact)
        .ok_or_else(|| {
            AskForFixError::InvalidState("this answer has no artifact to keep".into())
        })?;
    let body = match artifact.kind {
        AskArtifactKind::Prompt => artifact.body.clone(),
        AskArtifactKind::Runbook => artifact
            .steps
            .iter()
            .enumerate()
            .map(|(index, step)| format!("{}. {step}", index + 1))
            .collect::<Vec<_>>()
            .join("\n\n"),
        AskArtifactKind::ExistingCapability => format!(
            "{} — {}\n\n{}",
            artifact.tool,
            artifact.capability,
            artifact
                .instructions
                .iter()
                .enumerate()
                .map(|(index, step)| format!("{}. {step}", index + 1))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };
    let kind = match artifact.kind {
        AskArtifactKind::Prompt => "prompt",
        AskArtifactKind::Runbook => "runbook",
        AskArtifactKind::ExistingCapability => "existing_capability",
    };
    let artifact_id = stable_id("wfa", &("request", session_id))?;
    let version_id = stable_id("wav", &(&artifact_id, 1, session_id, &body))?;
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    sqlx::query(
        "INSERT OR IGNORE INTO artifacts(artifact_id,source_kind,source_finding_id,source_request_id,
         kind,title,current_version,status,capability_id,kept_at,last_used_at,updated_at,removed_at)
         VALUES(?1,'request',NULL,?2,?3,?4,1,'active',NULL,?5,NULL,?5,NULL)",
    )
    .bind(&artifact_id)
    .bind(session_id)
    .bind(kind)
    .bind(&artifact.title)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "INSERT OR IGNORE INTO artifact_versions(version_id,artifact_id,ordinal,title,body,
         source_finding_version_id,change_job_id,created_at) VALUES(?1,?2,1,?3,?4,NULL,NULL,?5)",
    )
    .bind(version_id)
    .bind(&artifact_id)
    .bind(&artifact.title)
    .bind(body)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    let event_id = stable_id("wae", &(&artifact_id, "kept"))?;
    sqlx::query(
        "INSERT OR IGNORE INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'kept',NULL,?3)",
    )
    .bind(event_id)
    .bind(&artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query("UPDATE ask_sessions SET artifact_kept_id=?2,updated_at=?3 WHERE session_id=?1")
        .bind(session_id)
        .bind(&artifact_id)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    Ok(artifact_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use dystil_ai::{
        AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiRuntimeDescriptor, AiRuntimeEvent,
        AiRuntimeKind, AiStructuredRun, TeammateAnswerRun,
    };
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    struct FakeRuntime {
        outputs: Mutex<Vec<Value>>,
        prompts: Arc<Mutex<Vec<AiStructuredRequest>>>,
        descriptor: AiRuntimeDescriptor,
    }

    #[async_trait]
    impl AiRuntime for FakeRuntime {
        fn descriptor(&self) -> &AiRuntimeDescriptor {
            &self.descriptor
        }

        async fn answer(
            &self,
            _request: AiAnswerRequest,
        ) -> std::result::Result<TeammateAnswerRun, AiRuntimeError> {
            unreachable!()
        }

        async fn run_automation(
            &self,
            _request: AiAutomationRequest,
            _events: mpsc::Sender<AiRuntimeEvent>,
        ) -> std::result::Result<AiAutomationRun, AiRuntimeError> {
            unreachable!()
        }

        async fn infer_structured(
            &self,
            request: AiStructuredRequest,
        ) -> std::result::Result<AiStructuredRun, AiRuntimeError> {
            self.prompts.lock().unwrap().push(request);
            let output = self.outputs.lock().unwrap().remove(0);
            Ok(AiStructuredRun {
                runtime: AiRuntimeKind::Codex,
                runtime_version: Some("test".into()),
                model: "gpt-5.6-sol".into(),
                elapsed_ms: 12,
                output,
                usage: BTreeMap::from([
                    ("input_tokens".into(), 1400),
                    ("cached_input_tokens".into(), 1024),
                ]),
            })
        }
    }

    fn understanding() -> Value {
        serde_json::json!({
            "synthesis": "The report is not the repeated work; rebuilding its context is.",
            "grounding": ["A report is rebuilt every Friday"],
            "inferences": ["Reusable context is missing"],
            "preservedBoundary": "The final judgement remains with the user",
            "uncertainty": ["Whether input formats remain stable"],
            "solutionTarget": "Prepare a current starting point"
        })
    }

    fn ask_output() -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "move": "ask",
            "assistantMessage": "I want to separate preparation from judgement.",
            "understanding": understanding(),
            "question": {
                "kind": "single_select",
                "text": "What should remain yours?",
                "helper": "Choose the closest answer or describe it yourself.",
                "options": [
                    {"id":"final_judgement","label":"Final judgement","description":"Prepare the groundwork but leave the decision to me."},
                    {"id":"nothing","label":"Nothing","description":"Take the unchanged task off me."}
                ],
                "minSelections": 1,
                "maxSelections": 1
            },
            "presentation": null
        })
    }

    fn consolidate_output() -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "move": "consolidate",
            "assistantMessage": "I have a working model of the problem.",
            "understanding": understanding(),
            "question": null,
            "presentation": null
        })
    }

    fn present_output() -> Value {
        serde_json::json!({
            "schemaVersion": 1,
            "move": "present",
            "assistantMessage": "The repeated groundwork can be written down once.",
            "understanding": understanding(),
            "question": null,
            "presentation": {
                "route": "answer_now",
                "headline": "Prepare the report context before judgement begins",
                "explanation": "Use a short runbook to collect current inputs and flag gaps while keeping the final call with you.",
                "limitations": ["This is based on your answers only."],
                "artifact": {
                    "kind": "runbook",
                    "title": "Weekly report preparation",
                    "description": "Prepare the stable groundwork before review.",
                    "body": "",
                    "steps": ["Collect the current inputs.", "Flag missing or contradictory values.", "Hand the prepared context to the final reviewer."],
                    "tool": "",
                    "capability": "",
                    "instructions": []
                }
            }
        })
    }

    async fn pool() -> SqlitePool {
        let dir = tempdir().unwrap().keep();
        crate::open_insights_database(dir.join("ask.sqlite"))
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn stable_prefix_is_identical_and_follow_up_replays_full_transcript() {
        let pool = pool().await;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let runtime = FakeRuntime {
            outputs: Mutex::new(vec![ask_output(), consolidate_output()]),
            prompts: prompts.clone(),
            descriptor: AiRuntimeDescriptor {
                kind: AiRuntimeKind::Codex,
                provider_label: "Codex".into(),
                model: "gpt-5.6-sol".into(),
            },
        };
        let session = create_ask_for_fix_session(&pool).await.unwrap();
        submit_ask_for_fix_turn(
            &pool,
            &runtime,
            &session.session_id,
            AskUserTurn {
                text: "Every Friday I rebuild the same report.".into(),
                event: AskInputEvent {
                    kind: "free_text".into(),
                    question_id: None,
                    selected_option_ids: vec![],
                },
            },
        )
        .await
        .unwrap();
        let result = submit_ask_for_fix_turn(
            &pool,
            &runtime,
            &session.session_id,
            AskUserTurn {
                text: "The closest answer is Final judgement — prepare the groundwork but leave the decision to me.".into(),
                event: AskInputEvent {
                    kind: "single_select".into(),
                    question_id: None,
                    selected_option_ids: vec!["final_judgement".into()],
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(result.phase, AskPhase::Consolidate);
        assert_eq!(result.cached_input_tokens, 1024);
        let captured = prompts.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].stable_prompt, captured[1].stable_prompt);
        assert_eq!(captured[0].stable_prompt, STABLE_PROMPT);
        assert_eq!(captured[0].reasoning_effort, AiReasoningEffort::High);
        assert_eq!(captured[0].tool_policy, AiToolPolicy::None);
        assert!(captured[1]
            .prompt
            .contains("Every Friday I rebuild the same report."));
        assert!(captured[1].prompt.contains("What should remain yours?"));
        assert!(captured[1].prompt.contains("Final judgement"));
    }

    #[tokio::test]
    async fn consolidation_cannot_be_a_transcript_replay_without_required_synthesis() {
        let invalid = AskModelMove {
            schema_version: 1,
            move_kind: AskMoveKind::Consolidate,
            assistant_message: "Here is what you said.".into(),
            understanding: AskUnderstanding {
                synthesis: String::new(),
                grounding: vec!["Weekly report".into()],
                inferences: vec![],
                preserved_boundary: String::new(),
                uncertainty: vec![],
                solution_target: String::new(),
            },
            question: None,
            presentation: None,
        };
        assert!(validate_move(invalid, AskPhase::FollowUp, 2, &[]).is_err());
    }

    #[tokio::test]
    async fn invalid_output_gets_one_repair_with_the_same_cacheable_prefix() {
        let pool = pool().await;
        let prompts = Arc::new(Mutex::new(Vec::new()));
        let runtime = FakeRuntime {
            outputs: Mutex::new(vec![serde_json::json!({"not": "the schema"}), ask_output()]),
            prompts: prompts.clone(),
            descriptor: AiRuntimeDescriptor {
                kind: AiRuntimeKind::Codex,
                provider_label: "Codex".into(),
                model: "gpt-5.6-sol".into(),
            },
        };
        let session = create_ask_for_fix_session(&pool).await.unwrap();
        let next = submit_ask_for_fix_turn(
            &pool,
            &runtime,
            &session.session_id,
            AskUserTurn {
                text: "Every Friday I rebuild the same report.".into(),
                event: AskInputEvent {
                    kind: "initial_problem".into(),
                    question_id: None,
                    selected_option_ids: vec![],
                },
            },
        )
        .await
        .unwrap();
        assert!(next.current_question.is_some());
        let captured = prompts.lock().unwrap();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].stable_prompt, captured[1].stable_prompt);
        assert!(captured[1].prompt.contains("Your prior object was invalid"));
        let statuses =
            sqlx::query_scalar::<_, String>("SELECT status FROM ask_attempts ORDER BY attempt")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(statuses, vec!["invalid_output", "accepted"]);
    }

    #[tokio::test]
    async fn consolidation_must_be_confirmed_before_presenting_and_artifact_keep_is_explicit() {
        let pool = pool().await;
        let runtime = FakeRuntime {
            outputs: Mutex::new(vec![consolidate_output(), present_output()]),
            prompts: Arc::new(Mutex::new(Vec::new())),
            descriptor: AiRuntimeDescriptor {
                kind: AiRuntimeKind::Codex,
                provider_label: "Codex".into(),
                model: "gpt-5.6-sol".into(),
            },
        };
        let session = create_ask_for_fix_session(&pool).await.unwrap();
        let checkpoint = submit_ask_for_fix_turn(
            &pool,
            &runtime,
            &session.session_id,
            AskUserTurn {
                text: "Every Friday I rebuild the report context from scattered files.".into(),
                event: AskInputEvent {
                    kind: "initial_problem".into(),
                    question_id: None,
                    selected_option_ids: vec![],
                },
            },
        )
        .await
        .unwrap();
        assert_eq!(checkpoint.phase, AskPhase::Consolidate);
        assert!(!checkpoint.locked);
        assert!(checkpoint.presentation.is_none());

        let answer = confirm_ask_for_fix(&pool, &runtime, &session.session_id)
            .await
            .unwrap();
        assert_eq!(answer.phase, AskPhase::Present);
        assert_eq!(answer.status, "answered");
        assert!(answer.locked);
        assert!(answer.presentation.as_ref().unwrap().artifact.is_some());
        assert!(answer.artifact_kept_id.is_none());

        let artifact_id = keep_ask_for_fix_artifact(&pool, &session.session_id)
            .await
            .unwrap();
        let kept = get_ask_for_fix_session(&pool, &session.session_id)
            .await
            .unwrap();
        assert_eq!(kept.artifact_kept_id.as_deref(), Some(artifact_id.as_str()));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts WHERE artifact_id=?1")
                .bind(&artifact_id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }
}
