use std::{collections::BTreeMap, time::Duration};

use dystil_ai::{
    AiModelTier, AiReasoningEffort, AiRuntime, AiRuntimeError, AiStructuredRequest, AiToolPolicy,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use thiserror::Error;

use crate::{
    accepted_job, apply_explorer_output_with_attempt, apply_reconciliation_with_attempt,
    claim_explorer_job, claim_job, create_explorer_job, create_job, mark_explorer_job,
    mark_job_failed, mark_job_rejected, pending_observations, record_explorer_attempt,
    record_job_attempt, recoverable_explorer_job, recoverable_job, steward_memory, upsert_evidence,
    AcceptedAttemptReceipt, ApplyOptions, ApplyResult, CompactActivity, EvidenceRecord,
    ExplorerOutput, InsightsError, NewExplorerJob, NewJob, ObservationRecord, ReconciliationOutput,
};

const EXPLORER_PROMPT_VERSION: &str = "worth-fixing-explorer-v1";
const EXPLORER_PROMPT: &str = include_str!("../resources/explorer_prompt_v1.md");
const EXPLORER_MODEL_TIER: AiModelTier = AiModelTier::Economy;
const STEWARD_PROMPT_VERSION: &str = "worth-fixing-steward-v2";
const STEWARD_PROMPT: &str = include_str!("../resources/steward_prompt_v2.md");
const STEWARD_MODEL_TIER: AiModelTier = AiModelTier::Frontier;
const STEWARD_TIMEOUT: Duration = Duration::from_secs(300);

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Store(#[from] InsightsError),
    #[error(transparent)]
    Runtime(#[from] AiRuntimeError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("Steward output remained invalid after one repair: {0}")]
    InvalidOutput(String),
}

pub type EngineResult<T> = std::result::Result<T, EngineError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StewardPacket {
    schema_version: u32,
    prompt_version: String,
    local_day: String,
    timezone: String,
    observations: Vec<ObservationRecord>,
    memory: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExplorerPacket {
    schema_version: u32,
    prompt_version: String,
    batch_id: String,
    timezone: String,
    evidence: Vec<EvidenceRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplorerRunResult {
    NoAdmissibleEvidence,
    Accepted {
        job_id: String,
        observation_ids: Vec<String>,
    },
    AlreadyAccepted {
        job_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WakeResult {
    NoWork,
    Accepted { job_id: String, apply: ApplyResult },
    AlreadyAccepted { job_id: String },
}

fn hash_bytes(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

fn schema() -> Value {
    serde_json::from_str(include_str!("../resources/steward_schema_v2.json"))
        .expect("bundled Steward schema must be valid JSON")
}

fn explorer_schema() -> Value {
    serde_json::from_str(include_str!("../resources/explorer_schema_v1.json"))
        .expect("bundled Explorer schema must be valid JSON")
}

/// Runs one durable Explorer batch with source-policy admission before the
/// packet is frozen. Barred evidence is neither persisted into the packet nor
/// sent to the provider. The accepted observations and job completion commit
/// atomically, and only one repair is permitted.
pub async fn run_explorer_batch<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    batch_id: &str,
    timezone: &str,
    evidence: &[EvidenceRecord],
) -> EngineResult<ExplorerRunResult> {
    run_explorer_batch_inner(pool, runtime, batch_id, timezone, evidence, None).await
}

pub async fn run_explorer_batch_with_compaction<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    batch_id: &str,
    timezone: &str,
    evidence: &[EvidenceRecord],
    compact: &[CompactActivity],
) -> EngineResult<ExplorerRunResult> {
    run_explorer_batch_inner(pool, runtime, batch_id, timezone, evidence, Some(compact)).await
}

async fn run_explorer_batch_inner<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    batch_id: &str,
    timezone: &str,
    evidence: &[EvidenceRecord],
    compact: Option<&[CompactActivity]>,
) -> EngineResult<ExplorerRunResult> {
    if let Some(stored) = recoverable_explorer_job(pool, batch_id).await? {
        if stored.status == "accepted" {
            return Ok(ExplorerRunResult::AlreadyAccepted {
                job_id: stored.job_id,
            });
        }
        return run_frozen_explorer_job(pool, runtime, stored.job_id, stored.input_json).await;
    }
    let admitted = evidence
        .iter()
        .filter(|item| item.admissible())
        .take(200)
        .cloned()
        .collect::<Vec<_>>();
    if admitted.is_empty() {
        return Ok(ExplorerRunResult::NoAdmissibleEvidence);
    }
    for item in &admitted {
        upsert_evidence(pool, item).await?;
    }
    let compact_by_id = compact.map(|items| {
        items
            .iter()
            .map(|item| {
                let mut lines = item
                    .added
                    .iter()
                    .map(|line| format!("+ {line}"))
                    .collect::<Vec<_>>();
                lines.extend(
                    item.reappeared
                        .iter()
                        .map(|line| format!("reappeared: {line}")),
                );
                (item.evidence_id.as_str(), lines.join("\n"))
            })
            .collect::<std::collections::HashMap<_, _>>()
    });
    let admitted = admitted
        .into_iter()
        .filter_map(|mut item| {
            if let Some(compact) = &compact_by_id {
                item.excerpt = compact
                    .get(item.evidence_id.as_str())?
                    .chars()
                    .take(1_000)
                    .collect();
            } else {
                item.excerpt = item.excerpt.chars().take(1_000).collect();
            }
            Some(item)
        })
        .collect::<Vec<_>>();
    if admitted.is_empty() {
        return Ok(ExplorerRunResult::NoAdmissibleEvidence);
    }
    let packet = ExplorerPacket {
        schema_version: 1,
        prompt_version: EXPLORER_PROMPT_VERSION.into(),
        batch_id: batch_id.into(),
        timezone: timezone.into(),
        evidence: admitted,
    };
    let input_json = serde_json::to_string(&packet)?;
    let prompt_hash = hash_bytes(EXPLORER_PROMPT.as_bytes());
    let schema_hash = hash_bytes(&serde_json::to_vec(&explorer_schema())?);
    let model = runtime.model_for_tier(EXPLORER_MODEL_TIER);
    let input_fingerprint = hash_bytes(
        format!(
            "{}\n{}\n{}\n{}",
            model, prompt_hash, schema_hash, input_json
        )
        .as_bytes(),
    );
    let job_id = create_explorer_job(
        pool,
        NewExplorerJob {
            batch_id,
            input_fingerprint: &input_fingerprint,
            input_json: &input_json,
            prompt_hash: &prompt_hash,
            schema_hash: &schema_hash,
            model: &model,
        },
    )
    .await?;
    run_frozen_explorer_job(pool, runtime, job_id, input_json).await
}

async fn run_frozen_explorer_job<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    job_id: String,
    input_json: String,
) -> EngineResult<ExplorerRunResult> {
    if !claim_explorer_job(pool, &job_id).await? {
        return Ok(ExplorerRunResult::AlreadyAccepted { job_id });
    }
    let model = runtime.model_for_tier(EXPLORER_MODEL_TIER);
    let mut prompt = format!("{EXPLORER_PROMPT}\n\nNORMALIZED_INPUT_PACKET:\n{input_json}");
    let mut last_error = String::new();
    for attempt in 0..=1 {
        let request_fingerprint = hash_bytes(
            format!(
                "{}\n{}\n{}\n{}",
                model,
                EXPLORER_PROMPT_VERSION,
                hash_bytes(&serde_json::to_vec(&explorer_schema())?),
                prompt
            )
            .as_bytes(),
        );
        let run = match runtime
            .infer_structured(AiStructuredRequest {
                purpose: "worth_fixing_explorer".into(),
                cache_key: None,
                model_tier: EXPLORER_MODEL_TIER,
                stable_prompt: String::new(),
                prompt: prompt.clone(),
                output_schema: explorer_schema(),
                timeout: Duration::from_secs(180),
                reasoning_effort: AiReasoningEffort::Default,
                tool_policy: AiToolPolicy::None,
            })
            .await
        {
            Ok(run) => run,
            Err(error) => {
                record_explorer_attempt(
                    pool,
                    &job_id,
                    &request_fingerprint,
                    None,
                    "provider_error",
                    &BTreeMap::<String, u64>::new(),
                    0,
                    Some(&format!("{:?}", error.code)),
                )
                .await?;
                mark_explorer_job(pool, &job_id, "pending", &format!("{:?}", error.code)).await?;
                return Err(error.into());
            }
        };
        let output_fingerprint = hash_bytes(&serde_json::to_vec(&run.output)?);
        match serde_json::from_value::<ExplorerOutput>(run.output.clone()) {
            Ok(output) => match apply_explorer_output_with_attempt(
                pool,
                &job_id,
                &output,
                AcceptedAttemptReceipt {
                    request_fingerprint: request_fingerprint.clone(),
                    output_fingerprint: output_fingerprint.clone(),
                    usage: serde_json::to_value(&run.usage)?,
                    latency_ms: run.elapsed_ms,
                },
            )
            .await
            {
                Ok(observation_ids) => {
                    return Ok(ExplorerRunResult::Accepted {
                        job_id,
                        observation_ids,
                    });
                }
                Err(error) => last_error = error.to_string(),
            },
            Err(error) => last_error = error.to_string(),
        }
        record_explorer_attempt(
            pool,
            &job_id,
            &request_fingerprint,
            Some(&output_fingerprint),
            "invalid_output",
            &run.usage,
            run.elapsed_ms,
            Some("invalid_output"),
        )
        .await?;
        if attempt == 0 {
            prompt = format!(
                "{EXPLORER_PROMPT}\n\nRepair the prior response once. Error: {last_error}\n\nNORMALIZED_INPUT_PACKET:\n{input_json}\n\nINVALID_RESPONSE:\n{}",
                serde_json::to_string(&run.output).unwrap_or_else(|_| "null".into()),
            );
        }
    }
    mark_explorer_job(pool, &job_id, "rejected", "invalid_after_repair").await?;
    Err(EngineError::InvalidOutput(last_error))
}

fn request_prompt(packet_json: &str) -> String {
    format!("{STEWARD_PROMPT}\n\nNORMALIZED_INPUT_PACKET:\n{packet_json}")
}

fn repair_prompt(packet_json: &str, invalid: &Value, error: &str) -> String {
    format!(
        "{STEWARD_PROMPT}\n\nYour prior response was structurally or semantically invalid. Repair it once. Error: {error}\n\nNORMALIZED_INPUT_PACKET:\n{packet_json}\n\nINVALID_RESPONSE:\n{}",
        serde_json::to_string(invalid).unwrap_or_else(|_| "null".into())
    )
}

async fn infer_once<R: AiRuntime + ?Sized>(
    runtime: &R,
    purpose: &str,
    prompt: String,
) -> std::result::Result<dystil_ai::AiStructuredRun, AiRuntimeError> {
    runtime
        .infer_structured(AiStructuredRequest {
            purpose: purpose.into(),
            cache_key: None,
            model_tier: STEWARD_MODEL_TIER,
            stable_prompt: String::new(),
            prompt,
            output_schema: schema(),
            timeout: STEWARD_TIMEOUT,
            reasoning_effort: AiReasoningEffort::High,
            tool_policy: AiToolPolicy::None,
        })
        .await
}

/// Executes one durable Steward wake. A job and frozen normalized packet exist
/// before inference; restart recovers that packet and accepted jobs never call
/// the provider again. At most one bounded repair is allowed per execution.
pub async fn run_steward_wake<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    local_day: &str,
    timezone: &str,
    reason: &str,
    observation_limit: u32,
) -> EngineResult<WakeResult> {
    run_steward_wake_inner(
        pool,
        runtime,
        local_day,
        timezone,
        reason,
        observation_limit,
        None,
    )
    .await
}

/// Backfill-only variant that keeps normal application job identity unchanged
/// while allowing an explicit replay to supersede a rejected frozen packet.
pub async fn run_steward_replay_wake<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    local_day: &str,
    timezone: &str,
    reason: &str,
    observation_limit: u32,
    replay_nonce: &str,
) -> EngineResult<WakeResult> {
    run_steward_wake_inner(
        pool,
        runtime,
        local_day,
        timezone,
        reason,
        observation_limit,
        Some(replay_nonce),
    )
    .await
}

async fn run_steward_wake_inner<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    local_day: &str,
    timezone: &str,
    reason: &str,
    observation_limit: u32,
    replay_nonce: Option<&str>,
) -> EngineResult<WakeResult> {
    let model = runtime.model_for_tier(STEWARD_MODEL_TIER);
    let stored = recoverable_job(pool).await?;
    let (job_id, packet_json) = if let Some(job) = stored {
        if accepted_job(pool, &job.job_id).await? {
            return Ok(WakeResult::AlreadyAccepted { job_id: job.job_id });
        }
        (job.job_id, job.input_json)
    } else {
        let observations = pending_observations(pool, observation_limit).await?;
        if observations.is_empty() {
            return Ok(WakeResult::NoWork);
        }
        let packet = StewardPacket {
            schema_version: 1,
            prompt_version: STEWARD_PROMPT_VERSION.into(),
            local_day: local_day.into(),
            timezone: timezone.into(),
            observations: observations.clone(),
            memory: steward_memory(pool, 10, 3).await?,
        };
        let packet_json = serde_json::to_string(&packet)?;
        let prompt_hash = hash_bytes(STEWARD_PROMPT.as_bytes());
        let schema_json = serde_json::to_vec(&schema())?;
        let schema_hash = hash_bytes(&schema_json);
        let input_fingerprint = hash_bytes(
            format!(
                "{}\n{}\n{}\n{}\n{}",
                model,
                prompt_hash,
                schema_hash,
                packet_json,
                replay_nonce.unwrap_or_default(),
            )
            .as_bytes(),
        );
        let observation_ids = observations
            .iter()
            .map(|item| item.observation_id.clone())
            .collect::<Vec<_>>();
        let job_id = create_job(
            pool,
            NewJob {
                input_fingerprint: &input_fingerprint,
                local_day,
                reason,
                observation_ids: &observation_ids,
                prompt_hash: &prompt_hash,
                schema_hash: &schema_hash,
                model: &model,
                input_json: &packet_json,
            },
        )
        .await?;
        (job_id, packet_json)
    };

    if !claim_job(pool, &job_id).await? {
        if accepted_job(pool, &job_id).await? {
            return Ok(WakeResult::AlreadyAccepted { job_id });
        }
        return Err(InsightsError::Invalid("job could not be claimed".into()).into());
    }

    let mut prompt = request_prompt(&packet_json);
    let mut last_error = String::new();
    for attempt_index in 0..=1 {
        let request_fingerprint = hash_bytes(
            format!(
                "{}\n{}\n{}\n{}",
                model,
                STEWARD_PROMPT_VERSION,
                hash_bytes(&serde_json::to_vec(&schema())?),
                prompt
            )
            .as_bytes(),
        );
        let run = match infer_once(runtime, "worth_fixing_steward", prompt.clone()).await {
            Ok(run) => run,
            Err(error) => {
                record_job_attempt(
                    pool,
                    &job_id,
                    &request_fingerprint,
                    None,
                    "provider_error",
                    &BTreeMap::<String, u64>::new(),
                    0,
                    Some(&format!("{:?}", error.code)),
                )
                .await?;
                mark_job_failed(pool, &job_id, &format!("{:?}", error.code)).await?;
                return Err(error.into());
            }
        };
        let output_fingerprint = hash_bytes(&serde_json::to_vec(&run.output)?);
        let parsed = serde_json::from_value::<ReconciliationOutput>(run.output.clone());
        if let Ok(output) = parsed {
            match apply_reconciliation_with_attempt(
                pool,
                &job_id,
                &output,
                ApplyOptions::default(),
                AcceptedAttemptReceipt {
                    request_fingerprint: request_fingerprint.clone(),
                    output_fingerprint: output_fingerprint.clone(),
                    usage: serde_json::to_value(&run.usage)?,
                    latency_ms: run.elapsed_ms,
                },
            )
            .await
            {
                Ok(apply) => {
                    return Ok(WakeResult::Accepted { job_id, apply });
                }
                Err(error) => last_error = error.to_string(),
            }
        } else if let Err(error) = parsed {
            last_error = error.to_string();
        }
        record_job_attempt(
            pool,
            &job_id,
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
    mark_job_rejected(pool, &job_id, "invalid_after_repair").await?;
    Err(EngineError::InvalidOutput(last_error))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, path::PathBuf, sync::Mutex};

    use async_trait::async_trait;
    use dystil_ai::{
        AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiRuntimeDescriptor, AiRuntimeEvent,
        AiRuntimeKind, AiStructuredRun, TeammateAnswerRun,
    };
    use serde_json::json;
    use tempfile::tempdir;
    use tokio::sync::mpsc;

    use super::*;
    use crate::{
        admit_observation, open_insights_database, upsert_evidence, EvidenceRecord,
        ObservationCertainty,
    };

    struct MockRuntime {
        descriptor: AiRuntimeDescriptor,
        outputs: Mutex<VecDeque<Value>>,
        calls: Mutex<Vec<String>>,
        tiers: Mutex<Vec<AiModelTier>>,
    }

    impl MockRuntime {
        fn new(outputs: Vec<Value>) -> Self {
            Self {
                descriptor: AiRuntimeDescriptor {
                    kind: AiRuntimeKind::Pi,
                    provider_label: "mock".into(),
                    model: "mock-model".into(),
                },
                outputs: Mutex::new(outputs.into()),
                calls: Mutex::new(Vec::new()),
                tiers: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl AiRuntime for MockRuntime {
        fn descriptor(&self) -> &AiRuntimeDescriptor {
            &self.descriptor
        }
        async fn answer(&self, _: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
            unreachable!()
        }
        async fn run_automation(
            &self,
            _: AiAutomationRequest,
            _: mpsc::Sender<AiRuntimeEvent>,
        ) -> Result<AiAutomationRun, AiRuntimeError> {
            unreachable!()
        }
        async fn infer_structured(
            &self,
            request: AiStructuredRequest,
        ) -> Result<AiStructuredRun, AiRuntimeError> {
            self.tiers.lock().unwrap().push(request.model_tier);
            self.calls.lock().unwrap().push(request.prompt);
            Ok(AiStructuredRun {
                runtime: AiRuntimeKind::Pi,
                runtime_version: None,
                model: self.descriptor.model.clone(),
                elapsed_ms: 5,
                output: self.outputs.lock().unwrap().pop_front().unwrap(),
                usage: BTreeMap::from([("input_tokens".into(), 10)]),
            })
        }
    }

    async fn fixture() -> (tempfile::TempDir, SqlitePool, String) {
        let dir = tempdir().unwrap();
        let pool = open_insights_database(dir.path().join("insights.sqlite"))
            .await
            .unwrap();
        let evidence_id = "ev-1".to_string();
        upsert_evidence(
            &pool,
            &EvidenceRecord {
                evidence_id: evidence_id.clone(),
                source_namespace: "fixture".into(),
                source_id: "1".into(),
                occurred_at: "2026-08-02T10:00:00Z".into(),
                app: Some("Mail".into()),
                window: None,
                excerpt: "Copied a value".into(),
                policy_allowed: true,
                redaction_ready: true,
                deleted: false,
                sensitive: false,
            },
        )
        .await
        .unwrap();
        admit_observation(
            &pool,
            &ObservationRecord {
                observation_id: "ob-1".into(),
                source_key: "fixture:1".into(),
                occurred_at: "2026-08-02T10:00:00Z".into(),
                statement: "Copied a value".into(),
                certainty: ObservationCertainty::Explicit,
                evidence_ids: vec![evidence_id],
            },
        )
        .await
        .unwrap();
        (dir, pool, "ob-1".into())
    }

    #[tokio::test]
    async fn repairs_once_then_commits_and_does_not_recall_on_resume() {
        let (_dir, pool, observation_id) = fixture().await;
        let runtime = MockRuntime::new(vec![
            json!({"bad": true}),
            json!({"schema_version": 1, "considered_observation_ids": [observation_id], "opportunities": []}),
        ]);
        let result = run_steward_wake(
            &pool,
            &runtime,
            "2026-08-02",
            "Asia/Kolkata",
            "threshold",
            50,
        )
        .await
        .unwrap();
        assert!(matches!(result, WakeResult::Accepted { .. }));
        assert_eq!(runtime.calls.lock().unwrap().len(), 2);
        assert_eq!(
            runtime.tiers.lock().unwrap().as_slice(),
            &[AiModelTier::Frontier, AiModelTier::Frontier]
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM job_attempts WHERE status='accepted'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
        let second = run_steward_wake(
            &pool,
            &runtime,
            "2026-08-02",
            "Asia/Kolkata",
            "threshold",
            50,
        )
        .await
        .unwrap();
        assert_eq!(second, WakeResult::NoWork);
        assert_eq!(runtime.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn explorer_excludes_barred_sources_and_repairs_bad_citations_once() {
        let dir = tempdir().unwrap();
        let pool = open_insights_database(dir.path().join("insights.sqlite"))
            .await
            .unwrap();
        let allowed = EvidenceRecord {
            evidence_id: "allowed:1".into(),
            source_namespace: "fixture".into(),
            source_id: "allowed-1".into(),
            occurred_at: "2026-08-02T10:00:00Z".into(),
            app: Some("Editor".into()),
            window: None,
            excerpt: "Prepared the same weekly update".into(),
            policy_allowed: true,
            redaction_ready: true,
            deleted: false,
            sensitive: false,
        };
        let barred = EvidenceRecord {
            evidence_id: "private:1".into(),
            source_namespace: "fixture".into(),
            source_id: "private-1".into(),
            occurred_at: "2026-08-02T10:01:00Z".into(),
            app: Some("Browser".into()),
            window: None,
            excerpt: "SECRET PRIVATE CONTENT".into(),
            policy_allowed: false,
            redaction_ready: true,
            deleted: false,
            sensitive: true,
        };
        let runtime = MockRuntime::new(vec![
            json!({"schema_version":1,"observations":[{
                "local_id":"obs_01","statement":"private claim","certainty":"explicit",
                "occurred_at":"2026-08-02T10:01:00Z","evidence_ids":["private:1"]
            }]}),
            json!({"schema_version":1,"observations":[{
                "local_id":"obs_01","statement":"Prepared the same weekly update",
                "certainty":"explicit","occurred_at":"2026-08-02T10:00:00Z",
                "evidence_ids":["allowed:1"]
            }]}),
        ]);
        let result = run_explorer_batch(
            &pool,
            &runtime,
            "batch_0001",
            "Asia/Kolkata",
            &[allowed, barred],
        )
        .await
        .unwrap();
        assert!(matches!(result, ExplorerRunResult::Accepted { .. }));
        let calls = runtime.calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(
            runtime.tiers.lock().unwrap().as_slice(),
            &[AiModelTier::Economy, AiModelTier::Economy]
        );
        assert!(!calls[0].contains("SECRET PRIVATE CONTENT"));
        assert!(!calls[0].contains("private:1"));
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM explorer_attempts WHERE status='accepted'"
            )
            .fetch_one(&pool)
            .await
            .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn explorer_repairs_duplicate_evidence_references() {
        let dir = tempdir().unwrap();
        let pool = open_insights_database(dir.path().join("insights.sqlite"))
            .await
            .unwrap();
        let evidence = EvidenceRecord {
            evidence_id: "allowed:duplicate-check".into(),
            source_namespace: "fixture".into(),
            source_id: "duplicate-check".into(),
            occurred_at: "2026-08-02T10:00:00Z".into(),
            app: Some("Editor".into()),
            window: None,
            excerpt: "Prepared the weekly update".into(),
            policy_allowed: true,
            redaction_ready: true,
            deleted: false,
            sensitive: false,
        };
        let runtime = MockRuntime::new(vec![
            json!({"schema_version":1,"observations":[{
                "local_id":"obs_01","statement":"Prepared the weekly update",
                "certainty":"explicit","occurred_at":"2026-08-02T10:00:00Z",
                "evidence_ids":["allowed:duplicate-check","allowed:duplicate-check"]
            }]}),
            json!({"schema_version":1,"observations":[{
                "local_id":"obs_01","statement":"Prepared the weekly update",
                "certainty":"explicit","occurred_at":"2026-08-02T10:00:00Z",
                "evidence_ids":["allowed:duplicate-check"]
            }]}),
        ]);

        let result = run_explorer_batch(
            &pool,
            &runtime,
            "batch_duplicate_check",
            "Asia/Kolkata",
            &[evidence],
        )
        .await
        .unwrap();

        assert!(matches!(result, ExplorerRunResult::Accepted { .. }));
        assert_eq!(runtime.calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn steward_repairs_duplicate_considered_observations() {
        let (_dir, pool, observation_id) = fixture().await;
        let runtime = MockRuntime::new(vec![
            json!({
                "schema_version": 1,
                "considered_observation_ids": [observation_id, observation_id],
                "opportunities": []
            }),
            json!({
                "schema_version": 1,
                "considered_observation_ids": [observation_id],
                "opportunities": []
            }),
        ]);

        let result = run_steward_wake(
            &pool,
            &runtime,
            "2026-08-02",
            "Asia/Kolkata",
            "threshold",
            50,
        )
        .await
        .unwrap();

        assert!(matches!(result, WakeResult::Accepted { .. }));
        assert_eq!(runtime.calls.lock().unwrap().len(), 2);
    }

    #[test]
    fn production_handoff_schema_cannot_request_automation() {
        let rendered = schema().to_string();
        assert!(!rendered.contains("\"automation\""));
        assert!(!STEWARD_PROMPT.contains("grounding judge"));
        assert!(!EXPLORER_PROMPT.contains("grounding judge"));
        let _compile_guard = PathBuf::from("provider-neutral");
    }

    #[test]
    fn production_schemas_match_the_provider_supported_subset() {
        fn assert_compatible(schema: &Value, path: &str) {
            let object = schema
                .as_object()
                .unwrap_or_else(|| panic!("schema node at {path} must be an object"));
            assert!(
                !object.contains_key("uniqueItems"),
                "unsupported uniqueItems at {path}"
            );
            if object.contains_key("const") || object.contains_key("enum") {
                assert!(
                    object.contains_key("type"),
                    "const/enum schema at {path} must declare its type"
                );
            }
            if object.get("type").and_then(Value::as_str) == Some("object") {
                assert_eq!(
                    object.get("additionalProperties"),
                    Some(&Value::Bool(false)),
                    "object schema at {path} must forbid additional properties"
                );
                let property_names = object
                    .get("properties")
                    .and_then(Value::as_object)
                    .unwrap_or_else(|| panic!("object schema at {path} needs properties"))
                    .keys()
                    .cloned()
                    .collect::<std::collections::BTreeSet<_>>();
                let required_names = object
                    .get("required")
                    .and_then(Value::as_array)
                    .unwrap_or_else(|| panic!("object schema at {path} needs required"))
                    .iter()
                    .map(|name| name.as_str().unwrap().to_owned())
                    .collect::<std::collections::BTreeSet<_>>();
                assert_eq!(
                    property_names, required_names,
                    "every property at {path} must be required"
                );
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for (name, property) in properties {
                    assert_compatible(property, &format!("{path}.properties.{name}"));
                }
            }
            if let Some(items) = object.get("items") {
                assert_compatible(items, &format!("{path}.items"));
            }
            if let Some(branches) = object.get("anyOf").and_then(Value::as_array) {
                for (index, branch) in branches.iter().enumerate() {
                    assert_compatible(branch, &format!("{path}.anyOf[{index}]"));
                }
            }
        }

        let artifact_schema: Value =
            serde_json::from_str(include_str!("../resources/artifact_change_schema_v1.json"))
                .unwrap();
        for (name, value) in [
            ("explorer", explorer_schema()),
            ("steward", schema()),
            ("artifact_change", artifact_schema),
        ] {
            assert_compatible(&value, name);
        }
    }
}
