//! Recovery-safe structured rewriting for user-kept artifacts.

use std::{collections::BTreeMap, time::Duration};

use chrono::Utc;
use dystil_ai::{AiModelTier, AiRuntime, AiRuntimeError, AiStructuredRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use thiserror::Error;

use crate::{
    ready_artifact_detail,
    store::{fingerprint, stable_id},
    ArtifactChangeOutput, ArtifactChangePreview, InsightsError, ReadyArtifactDetail,
};

const CHANGE_PROMPT_VERSION: &str = "artifact_change_v1";
const CHANGE_PROMPT: &str = include_str!("../resources/artifact_change_prompt_v1.md");
const CHANGE_MODEL_TIER: AiModelTier = AiModelTier::Frontier;

#[derive(Debug, Error)]
pub enum ArtifactEngineError {
    #[error(transparent)]
    Store(#[from] InsightsError),
    #[error(transparent)]
    Runtime(#[from] AiRuntimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("artifact change remained invalid after one repair: {0}")]
    InvalidOutput(String),
}

pub type ArtifactEngineResult<T> = std::result::Result<T, ArtifactEngineError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChangePacket {
    schema_version: u32,
    artifact_id: String,
    base_version: u32,
    kind: String,
    current_title: String,
    current_body: String,
    requested_change: String,
}

fn schema() -> Value {
    serde_json::from_str(include_str!("../resources/artifact_change_schema_v1.json"))
        .expect("bundled artifact change schema must be valid")
}

fn hash_bytes(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn request_prompt(packet_json: &str) -> String {
    format!("{CHANGE_PROMPT}\n\nArtifact packet:\n{packet_json}")
}

fn repair_prompt(packet_json: &str, invalid: &Value, error: &str) -> String {
    format!(
        "{CHANGE_PROMPT}\n\nArtifact packet:\n{packet_json}\n\nYour prior output was invalid:\n{}\n\nValidation error:\n{}\nReturn one corrected JSON object.",
        serde_json::to_string(invalid).unwrap_or_else(|_| "null".into()),
        error
    )
}

fn validate_output(output: ArtifactChangeOutput) -> Result<ArtifactChangeOutput, String> {
    if output.schema_version != 1 {
        return Err("wrong artifact-change schema version".into());
    }
    if output.title.trim().is_empty() || output.title.chars().count() > 160 {
        return Err("replacement title is not bounded".into());
    }
    if output.body.trim().is_empty() || output.body.chars().count() > 12_000 {
        return Err("replacement body is not bounded".into());
    }
    Ok(output)
}

fn changed_line_count(before: &str, after: &str) -> u32 {
    let before = before.lines().collect::<Vec<_>>();
    let after = after.lines().collect::<Vec<_>>();
    let length = before.len().max(after.len());
    (0..length)
        .filter(|index| before.get(*index) != after.get(*index))
        .count() as u32
}

async fn preview_for_job(
    pool: &SqlitePool,
    job_id: &str,
) -> ArtifactEngineResult<ArtifactChangePreview> {
    let row = sqlx::query(
        "SELECT j.artifact_id,j.proposed_title,j.proposed_body,v.body current_body
         FROM artifact_change_jobs j JOIN artifact_versions v
          ON v.artifact_id=j.artifact_id AND v.ordinal=j.base_version
         WHERE j.job_id=?1 AND j.status IN ('preview_ready','accepted')",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| InsightsError::Invalid("artifact change has no preview".into()))?;
    let body: String = row
        .get::<Option<String>, _>("proposed_body")
        .unwrap_or_default();
    let title: String = row
        .get::<Option<String>, _>("proposed_title")
        .unwrap_or_default();
    Ok(ArtifactChangePreview {
        change_job_id: job_id.into(),
        artifact_id: row.get("artifact_id"),
        changed_line_count: changed_line_count(row.get("current_body"), &body),
        title,
        body,
    })
}

async fn run_job<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    job_id: &str,
) -> ArtifactEngineResult<ArtifactChangePreview> {
    if let Ok(preview) = preview_for_job(pool, job_id).await {
        return Ok(preview);
    }
    let job =
        sqlx::query("SELECT input_json,status,attempts FROM artifact_change_jobs WHERE job_id=?1")
            .bind(job_id)
            .fetch_one(pool)
            .await
            .map_err(InsightsError::from)?;
    let status: String = job.get("status");
    if !matches!(status.as_str(), "pending" | "running" | "rejected") {
        return Err(InsightsError::Invalid(format!("artifact change job is {status}")).into());
    }
    sqlx::query(
        "UPDATE artifact_change_jobs SET status='running',attempts=attempts+1,updated_at=?2
         WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    let packet_json: String = job.get("input_json");
    let mut prompt = request_prompt(&packet_json);
    let model = runtime.model_for_tier(CHANGE_MODEL_TIER);
    let schema_value = schema();
    let schema_hash = hash_bytes(&serde_json::to_vec(&schema_value)?);
    let mut last_error = String::new();
    for attempt_index in 0..=1 {
        let request_fingerprint = hash_bytes(
            format!(
                "{}\n{}\n{}\n{}",
                model, CHANGE_PROMPT_VERSION, schema_hash, prompt
            )
            .as_bytes(),
        );
        let run = match runtime
            .infer_structured(AiStructuredRequest {
                purpose: "worth_fixing_artifact_change".into(),
                model_tier: CHANGE_MODEL_TIER,
                prompt: prompt.clone(),
                output_schema: schema_value.clone(),
                timeout: Duration::from_secs(180),
            })
            .await
        {
            Ok(run) => run,
            Err(error) => {
                record_attempt(
                    pool,
                    job_id,
                    &request_fingerprint,
                    None,
                    "provider_error",
                    &BTreeMap::<String, u64>::new(),
                    0,
                    Some(&format!("{:?}", error.code)),
                )
                .await?;
                sqlx::query(
                    "UPDATE artifact_change_jobs SET status='pending',error_code=?2,updated_at=?3
                     WHERE job_id=?1",
                )
                .bind(job_id)
                .bind(format!("{:?}", error.code))
                .bind(Utc::now().to_rfc3339())
                .execute(pool)
                .await
                .map_err(InsightsError::from)?;
                return Err(error.into());
            }
        };
        let output_fingerprint = hash_bytes(&serde_json::to_vec(&run.output)?);
        let parsed = serde_json::from_value::<ArtifactChangeOutput>(run.output.clone())
            .map_err(|error| error.to_string())
            .and_then(validate_output);
        match parsed {
            Ok(output) => {
                let mut tx = pool.begin().await.map_err(InsightsError::from)?;
                let attempt = sqlx::query_scalar::<_, i64>(
                    "SELECT COALESCE(MAX(attempt),0)+1 FROM artifact_change_attempts WHERE job_id=?1",
                )
                .bind(job_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(InsightsError::from)?;
                sqlx::query(
                    "INSERT INTO artifact_change_attempts VALUES(?1,?2,?3,?4,'accepted',?5,?6,NULL,?7)",
                )
                .bind(job_id)
                .bind(attempt)
                .bind(&request_fingerprint)
                .bind(&output_fingerprint)
                .bind(serde_json::to_string(&run.usage)?)
                .bind(run.elapsed_ms as i64)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(InsightsError::from)?;
                sqlx::query(
                    "UPDATE artifact_change_jobs SET status='preview_ready',proposed_title=?2,
                     proposed_body=?3,error_code=NULL,updated_at=?4 WHERE job_id=?1",
                )
                .bind(job_id)
                .bind(output.title)
                .bind(output.body)
                .bind(Utc::now().to_rfc3339())
                .execute(&mut *tx)
                .await
                .map_err(InsightsError::from)?;
                tx.commit().await.map_err(InsightsError::from)?;
                return preview_for_job(pool, job_id).await;
            }
            Err(error) => last_error = error,
        }
        record_attempt(
            pool,
            job_id,
            &request_fingerprint,
            Some(&output_fingerprint),
            "invalid_output",
            &run.usage,
            run.elapsed_ms,
            Some("invalid_output"),
        )
        .await?;
        if attempt_index == 0 {
            prompt = repair_prompt(&packet_json, &run.output, &last_error);
        }
    }
    sqlx::query(
        "UPDATE artifact_change_jobs SET status='rejected',error_code='invalid_after_repair',
         updated_at=?2 WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    Err(ArtifactEngineError::InvalidOutput(last_error))
}

#[allow(clippy::too_many_arguments)]
async fn record_attempt(
    pool: &SqlitePool,
    job_id: &str,
    request_fingerprint: &str,
    output_fingerprint: Option<&str>,
    status: &str,
    usage: &impl Serialize,
    latency_ms: u64,
    error_code: Option<&str>,
) -> ArtifactEngineResult<()> {
    let attempt = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(attempt),0)+1 FROM artifact_change_attempts WHERE job_id=?1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query("INSERT INTO artifact_change_attempts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
        .bind(job_id)
        .bind(attempt)
        .bind(request_fingerprint)
        .bind(output_fingerprint)
        .bind(status)
        .bind(serde_json::to_string(usage)?)
        .bind(latency_ms as i64)
        .bind(error_code)
        .bind(Utc::now().to_rfc3339())
        .execute(pool)
        .await
        .map_err(InsightsError::from)?;
    Ok(())
}

pub async fn propose_artifact_change<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    artifact_id: &str,
    request: &str,
) -> ArtifactEngineResult<ArtifactChangePreview> {
    let request = request.trim();
    if request.is_empty() || request.chars().count() > 2_000 {
        return Err(InsightsError::Invalid(
            "change request must contain 1 to 2000 characters".into(),
        )
        .into());
    }
    if let Some(job_id) = sqlx::query_scalar::<_, String>(
        "SELECT job_id FROM artifact_change_jobs WHERE artifact_id=?1
         AND status IN ('pending','running','preview_ready') ORDER BY created_at DESC LIMIT 1",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    {
        return run_job(pool, runtime, &job_id).await;
    }
    let row = sqlx::query(
        "SELECT a.current_version,a.kind,a.title,v.body FROM artifacts a JOIN artifact_versions v
          ON v.artifact_id=a.artifact_id AND v.ordinal=a.current_version
         WHERE a.artifact_id=?1 AND a.status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    let packet = ChangePacket {
        schema_version: 1,
        artifact_id: artifact_id.into(),
        base_version: row.get::<i64, _>("current_version") as u32,
        kind: row.get("kind"),
        current_title: row.get("title"),
        current_body: row.get("body"),
        requested_change: request.into(),
    };
    let packet_json = serde_json::to_string(&packet)?;
    let model = runtime.model_for_tier(CHANGE_MODEL_TIER);
    let prompt_hash = hash_bytes(CHANGE_PROMPT.as_bytes());
    let schema_hash = hash_bytes(&serde_json::to_vec(&schema())?);
    let input_fingerprint = fingerprint(&(
        artifact_id,
        packet.base_version,
        request,
        &model,
        &prompt_hash,
        &schema_hash,
    ))?;
    if let Some(job_id) = sqlx::query_scalar::<_, String>(
        "SELECT job_id FROM artifact_change_jobs WHERE input_fingerprint=?1",
    )
    .bind(&input_fingerprint)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?
    {
        return run_job(pool, runtime, &job_id).await;
    }
    let job_id = stable_id("wac", &input_fingerprint)?;
    let now = Utc::now().to_rfc3339();
    let event_id = stable_id("wae", &(&job_id, "change_requested"))?;
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    sqlx::query(
        "INSERT INTO artifact_change_jobs(
          job_id,artifact_id,base_version,request_text,input_fingerprint,status,input_json,
          prompt_hash,schema_hash,model,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,'pending',?6,?7,?8,?9,?10,?10)",
    )
    .bind(&job_id)
    .bind(artifact_id)
    .bind(packet.base_version as i64)
    .bind(request)
    .bind(input_fingerprint)
    .bind(packet_json)
    .bind(prompt_hash)
    .bind(schema_hash)
    .bind(&model)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "INSERT INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'change_requested',NULL,?3)",
    )
    .bind(event_id)
    .bind(artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    run_job(pool, runtime, &job_id).await
}

pub async fn retry_artifact_change<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    job_id: &str,
) -> ArtifactEngineResult<ArtifactChangePreview> {
    let result = sqlx::query(
        "UPDATE artifact_change_jobs SET status='pending',proposed_title=NULL,proposed_body=NULL,
         error_code=NULL,updated_at=?2 WHERE job_id=?1 AND status IN ('preview_ready','rejected','pending')",
    )
    .bind(job_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await
    .map_err(InsightsError::from)?;
    if result.rows_affected() != 1 {
        return Err(InsightsError::Invalid("artifact change cannot be retried".into()).into());
    }
    run_job(pool, runtime, job_id).await
}

pub async fn confirm_artifact_change(
    pool: &SqlitePool,
    job_id: &str,
) -> ArtifactEngineResult<ReadyArtifactDetail> {
    let accepted_artifact = sqlx::query_scalar::<_, String>(
        "SELECT artifact_id FROM artifact_change_jobs WHERE job_id=?1 AND status='accepted'",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await
    .map_err(InsightsError::from)?;
    if let Some(artifact_id) = accepted_artifact {
        return Ok(ready_artifact_detail(pool, &artifact_id).await?);
    }
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    let row = sqlx::query(
        "SELECT artifact_id,base_version,request_text,proposed_title,proposed_body
         FROM artifact_change_jobs WHERE job_id=?1 AND status='preview_ready'",
    )
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| InsightsError::Invalid("artifact change has no confirmable preview".into()))?;
    let artifact_id: String = row.get("artifact_id");
    let base_version: i64 = row.get("base_version");
    let current_version = sqlx::query_scalar::<_, i64>(
        "SELECT current_version FROM artifacts WHERE artifact_id=?1 AND status='active'",
    )
    .bind(&artifact_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    if current_version != base_version {
        return Err(InsightsError::Invalid(
            "artifact changed after this preview was created".into(),
        )
        .into());
    }
    let next_version = current_version + 1;
    let title: String = row
        .get::<Option<String>, _>("proposed_title")
        .unwrap_or_default();
    let body: String = row
        .get::<Option<String>, _>("proposed_body")
        .unwrap_or_default();
    validate_output(ArtifactChangeOutput {
        schema_version: 1,
        title: title.clone(),
        body: body.clone(),
    })
    .map_err(ArtifactEngineError::InvalidOutput)?;
    let now = Utc::now().to_rfc3339();
    let version_id = stable_id("wav", &(&artifact_id, next_version, job_id, &body))?;
    sqlx::query(
        "INSERT INTO artifact_versions(
          version_id,artifact_id,ordinal,title,body,source_finding_version_id,change_job_id,created_at)
         VALUES(?1,?2,?3,?4,?5,NULL,?6,?7)",
    )
    .bind(version_id)
    .bind(&artifact_id)
    .bind(next_version)
    .bind(&title)
    .bind(&body)
    .bind(job_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "UPDATE artifacts SET title=?2,current_version=?3,updated_at=?4 WHERE artifact_id=?1",
    )
    .bind(&artifact_id)
    .bind(&title)
    .bind(next_version)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    sqlx::query(
        "UPDATE artifact_change_jobs SET status='accepted',accepted_at=?2,updated_at=?2
         WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    let event_id = stable_id("wae", &(&artifact_id, "change_confirmed", job_id))?;
    sqlx::query(
        "INSERT INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'change_confirmed',NULL,?3)",
    )
    .bind(event_id)
    .bind(&artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    ready_artifact_detail(pool, &artifact_id)
        .await
        .map_err(ArtifactEngineError::from)
}

pub async fn reject_artifact_change(
    pool: &SqlitePool,
    job_id: &str,
) -> ArtifactEngineResult<ReadyArtifactDetail> {
    let mut tx = pool.begin().await.map_err(InsightsError::from)?;
    let artifact_id = sqlx::query_scalar::<_, String>(
        "SELECT artifact_id FROM artifact_change_jobs WHERE job_id=?1
         AND status IN ('preview_ready','rejected')",
    )
    .bind(job_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(InsightsError::from)?
    .ok_or_else(|| InsightsError::Invalid("artifact change cannot be rejected".into()))?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE artifact_change_jobs SET status='rejected',proposed_title=NULL,proposed_body=NULL,
         updated_at=?2 WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(&now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    let event_id = stable_id("wae", &(&artifact_id, "change_rejected", job_id))?;
    sqlx::query(
        "INSERT OR IGNORE INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'change_rejected',NULL,?3)",
    )
    .bind(event_id)
    .bind(&artifact_id)
    .bind(now)
    .execute(&mut *tx)
    .await
    .map_err(InsightsError::from)?;
    tx.commit().await.map_err(InsightsError::from)?;
    ready_artifact_detail(pool, &artifact_id)
        .await
        .map_err(ArtifactEngineError::from)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Mutex};

    use dystil_ai::{
        AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiRuntimeDescriptor,
        AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind, AiStructuredRun, TeammateAnswerRun,
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::{keep_finding, test_support};

    struct MockRuntime {
        descriptor: AiRuntimeDescriptor,
        outputs: Mutex<VecDeque<std::result::Result<Value, AiRuntimeError>>>,
    }

    impl MockRuntime {
        fn new(outputs: Vec<std::result::Result<Value, AiRuntimeError>>) -> Self {
            Self {
                descriptor: AiRuntimeDescriptor {
                    kind: AiRuntimeKind::Codex,
                    provider_label: "Mock".into(),
                    model: "mock-artifact-v1".into(),
                },
                outputs: Mutex::new(outputs.into()),
            }
        }
    }

    #[async_trait::async_trait]
    impl AiRuntime for MockRuntime {
        fn descriptor(&self) -> &AiRuntimeDescriptor {
            &self.descriptor
        }

        async fn answer(
            &self,
            _request: AiAnswerRequest,
        ) -> std::result::Result<TeammateAnswerRun, AiRuntimeError> {
            Err(AiRuntimeError::new(AiRuntimeErrorCode::NotReady, "unused"))
        }

        async fn run_automation(
            &self,
            _request: AiAutomationRequest,
            _events: mpsc::Sender<AiRuntimeEvent>,
        ) -> std::result::Result<AiAutomationRun, AiRuntimeError> {
            Err(AiRuntimeError::new(AiRuntimeErrorCode::NotReady, "unused"))
        }

        async fn infer_structured(
            &self,
            request: dystil_ai::AiStructuredRequest,
        ) -> std::result::Result<AiStructuredRun, AiRuntimeError> {
            assert_eq!(request.model_tier, AiModelTier::Frontier);
            let output = self
                .outputs
                .lock()
                .unwrap()
                .pop_front()
                .expect("mock output")?;
            Ok(AiStructuredRun {
                runtime: AiRuntimeKind::Codex,
                runtime_version: Some("test".into()),
                model: self.descriptor.model.clone(),
                elapsed_ms: 4,
                output,
                usage: BTreeMap::from([("input_tokens".into(), 12)]),
            })
        }
    }

    async fn setup() -> (tempfile::TempDir, SqlitePool, String, String) {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();
        let finding_id = test_support::seed_findings(&pool, 1).await.remove(0);
        let kept = keep_finding(&pool, &finding_id, true).await.unwrap();
        let body = crate::ready_artifact_detail(&pool, &kept.artifact.artifact_id)
            .await
            .unwrap()
            .body;
        (directory, pool, kept.artifact.artifact_id, body)
    }

    fn valid(title: &str, body: &str) -> Value {
        serde_json::json!({"schema_version":1,"title":title,"body":body})
    }

    #[tokio::test]
    async fn preview_does_not_mutate_and_confirmation_is_atomic_and_idempotent() {
        let (_directory, pool, artifact_id, original) = setup().await;
        let runtime = MockRuntime::new(vec![Ok(valid(
            "Short report",
            "A shorter complete prompt.",
        ))]);
        let preview = propose_artifact_change(&pool, &runtime, &artifact_id, "Make it shorter")
            .await
            .unwrap();
        assert_eq!(
            crate::ready_artifact_detail(&pool, &artifact_id)
                .await
                .unwrap()
                .body,
            original
        );
        let accepted = confirm_artifact_change(&pool, &preview.change_job_id)
            .await
            .unwrap();
        assert_eq!(accepted.body, "A shorter complete prompt.");
        assert_eq!(accepted.change_count, 1);
        assert_eq!(
            confirm_artifact_change(&pool, &preview.change_job_id)
                .await
                .unwrap()
                .body,
            accepted.body
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artifact_versions WHERE artifact_id=?1"
            )
            .bind(&artifact_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
    }

    #[tokio::test]
    async fn invalid_output_gets_one_repair_then_can_be_rejected_without_mutation() {
        let (_directory, pool, artifact_id, original) = setup().await;
        let runtime = MockRuntime::new(vec![
            Ok(serde_json::json!({"schema_version":1,"title":"","body":""})),
            Ok(valid("Repaired", "A repaired complete prompt.")),
        ]);
        let preview = propose_artifact_change(&pool, &runtime, &artifact_id, "Repair this")
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artifact_change_attempts WHERE job_id=?1"
            )
            .bind(&preview.change_job_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            2
        );
        reject_artifact_change(&pool, &preview.change_job_id)
            .await
            .unwrap();
        assert_eq!(
            crate::ready_artifact_detail(&pool, &artifact_id)
                .await
                .unwrap()
                .body,
            original
        );
    }

    #[tokio::test]
    async fn provider_failure_leaves_a_durable_retryable_job() {
        let (_directory, pool, artifact_id, _original) = setup().await;
        let failed = MockRuntime::new(vec![Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Transport,
            "offline",
        ))]);
        assert!(
            propose_artifact_change(&pool, &failed, &artifact_id, "Clarify it")
                .await
                .is_err()
        );
        let row = sqlx::query("SELECT job_id,status,input_fingerprint FROM artifact_change_jobs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("status"), "pending");
        assert!(!row.get::<String, _>("input_fingerprint").is_empty());
        let recovered = MockRuntime::new(vec![Ok(valid("Clarified", "A clear complete prompt."))]);
        let preview = retry_artifact_change(&pool, &recovered, row.get("job_id"))
            .await
            .unwrap();
        assert_eq!(preview.body, "A clear complete prompt.");
    }

    #[tokio::test]
    async fn end_to_end_keep_restart_use_edit_stale_provenance_and_remove() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("end-to-end.sqlite");
        let pool = crate::open_insights_database(&path).await.unwrap();
        let finding_ids = test_support::seed_findings(&pool, 7).await;
        let constructs = [
            "recognition",
            "recognition",
            "manual_transfer",
            "manual_transfer",
            "unchanged_repetition",
            "temporal_pattern",
            "repeated_composition",
        ];
        for (finding_id, construct) in finding_ids.iter().zip(constructs) {
            sqlx::query("UPDATE findings SET construct=?2 WHERE finding_id=?1")
                .bind(finding_id)
                .bind(construct)
                .execute(&pool)
                .await
                .unwrap();
        }
        crate::recompute_surface_status(&pool).await.unwrap();
        let summary = crate::worth_fixing_summary(&pool, true).await.unwrap();
        assert_eq!(summary.selected.len(), 5);
        let kept_finding = summary.selected[0].finding_id.clone();
        let source_evidence = sqlx::query_scalar::<_, String>(
            "SELECT evidence_id FROM finding_evidence WHERE finding_id=?1 LIMIT 1",
        )
        .bind(&kept_finding)
        .fetch_one(&pool)
        .await
        .unwrap();
        let kept = keep_finding(&pool, &kept_finding, true).await.unwrap();
        assert_eq!(
            crate::ready_artifacts(&pool, None, 50)
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        pool.close().await;

        let reopened = crate::open_insights_database(&path).await.unwrap();
        let retry = keep_finding(&reopened, &kept_finding, true).await.unwrap();
        assert!(retry.already_kept);
        assert_eq!(retry.artifact.artifact_id, kept.artifact.artifact_id);
        crate::record_artifact_used(
            &reopened,
            &kept.artifact.artifact_id,
            crate::ReadyArtifactAction::Copy,
        )
        .await
        .unwrap();
        let runtime = MockRuntime::new(vec![Ok(valid(
            "Edited report prompt",
            "Use this edited and complete report prompt.",
        ))]);
        let preview = propose_artifact_change(
            &reopened,
            &runtime,
            &kept.artifact.artifact_id,
            "Make the completion instruction explicit",
        )
        .await
        .unwrap();
        confirm_artifact_change(&reopened, &preview.change_job_id)
            .await
            .unwrap();
        sqlx::query("UPDATE evidence SET deleted=1 WHERE evidence_id=?1")
            .bind(source_evidence)
            .execute(&reopened)
            .await
            .unwrap();
        let stale = crate::ready_artifact_detail(&reopened, &kept.artifact.artifact_id)
            .await
            .unwrap();
        assert!(!stale.provenance_available);
        assert_eq!(stale.body, "Use this edited and complete report prompt.");
        crate::remove_artifact(&reopened, &kept.artifact.artifact_id)
            .await
            .unwrap();
        assert!(
            crate::ready_artifact_detail(&reopened, &kept.artifact.artifact_id)
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artifact_versions WHERE artifact_id=?1"
            )
            .bind(&kept.artifact.artifact_id)
            .fetch_one(&reopened)
            .await
            .unwrap(),
            0
        );
    }
}
