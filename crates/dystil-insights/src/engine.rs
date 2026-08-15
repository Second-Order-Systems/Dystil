use std::{
    collections::{BTreeMap, HashMap, HashSet},
    time::Duration,
};

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
    AcceptedAttemptReceipt, ApplyOptions, ApplyResult, CandidateAssessment, CandidateDecision,
    CandidateReasonCode, CompactActivity, EvidenceRecord, ExplorerOutput, FindingDraft, Handoff,
    InsightsError, NewExplorerJob, NewJob, ObservationRecord, OccurrenceDelta, OpportunityDelta,
    RankSignals, ReconciliationOutput,
};

const EXPLORER_PROMPT_VERSION: &str = "worth-fixing-explorer-v2";
const EXPLORER_PROMPT: &str = include_str!("../resources/explorer_prompt_v2.md");
const EXPLORER_MODEL_TIER: AiModelTier = AiModelTier::Economy;
const STEWARD_PROMPT_VERSION: &str = "worth-fixing-steward-v3";
const STEWARD_PROMPT: &str = include_str!("../resources/steward_prompt_v3.md");
const STEWARD_MODEL_TIER: AiModelTier = AiModelTier::Frontier;
const STEWARD_REASONING_EFFORT: AiReasoningEffort = AiReasoningEffort::Default;
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

/// The only Steward-facing observation identity is `ref`, a one-based ordinal
/// in the frozen job packet. Durable observation and evidence identities remain
/// local to the kernel.
#[derive(Debug, Clone, Serialize)]
struct StewardModelObservation {
    #[serde(rename = "ref")]
    reference: u32,
    occurred_at: String,
    statement: String,
    certainty: crate::ObservationCertainty,
}

#[derive(Debug, Clone, Serialize)]
struct StewardModelMemoryOpportunity {
    #[serde(rename = "ref")]
    reference: u32,
    construct: String,
    status: String,
    summary: String,
    cadence: String,
    occurrence_count: i64,
}

#[derive(Debug, Clone, Serialize)]
struct StewardModelPacket {
    schema_version: u32,
    prompt_version: String,
    local_day: String,
    timezone: String,
    observations: Vec<StewardModelObservation>,
    memory: Vec<StewardModelMemoryOpportunity>,
}

#[derive(Debug, Clone, Deserialize)]
struct StewardModelOutput {
    schema_version: u32,
    candidates: Vec<StewardModelCandidate>,
}

#[derive(Debug, Clone, Deserialize)]
struct StewardModelCandidate {
    observation_groups: Vec<StewardModelObservationGroup>,
    decision: CandidateDecision,
    reason_code: CandidateReasonCode,
    reason: String,
    shared_goal: String,
    reducible_burden: String,
    stable_steps: Vec<String>,
    variable_inputs: Vec<String>,
    distinct_episode_basis: Vec<String>,
    missing_to_qualify: Vec<String>,
    opportunity: Option<StewardModelOpportunity>,
}

#[derive(Debug, Clone, Deserialize)]
struct StewardModelObservationGroup {
    observation_refs: Vec<u32>,
    steps: Vec<String>,
    distinctness_basis: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct StewardModelOpportunity {
    existing_opportunity_ref: Option<u32>,
    construct: crate::Construct,
    summary: String,
    signature: String,
    withdraw_current_finding: bool,
    retire: bool,
    transfer_established: bool,
    authorship_established: bool,
    cadence: crate::Cadence,
    unresolved_questions: Vec<String>,
    evidence_quality: crate::EvidenceQuality,
    handoff: Option<Handoff>,
    finding: Option<StewardModelFinding>,
    rank_signals: RankSignals,
    automation_potential: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct StewardModelFinding {
    claim: String,
    why_worth_fixing: String,
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
    serde_json::from_str(include_str!("../resources/steward_schema_v3.json"))
        .expect("bundled Steward schema must be valid JSON")
}

fn retain_raw_steward_response(reason: &str) -> bool {
    matches!(
        reason,
        "steward_replay" | "fixture_backfill" | "fixture_backfill_steward_only"
    )
}

fn model_packet(packet: &StewardPacket) -> StewardModelPacket {
    let memory = packet
        .memory
        .get("opportunities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, item)| StewardModelMemoryOpportunity {
                    reference: (index + 1) as u32,
                    construct: item
                        .get("construct")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    status: item
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    summary: item
                        .get("summary")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    cadence: item
                        .get("cadence")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .into(),
                    occurrence_count: item
                        .get("occurrence_count")
                        .and_then(Value::as_i64)
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    StewardModelPacket {
        schema_version: 1,
        prompt_version: STEWARD_PROMPT_VERSION.into(),
        local_day: packet.local_day.clone(),
        timezone: packet.timezone.clone(),
        observations: packet
            .observations
            .iter()
            .enumerate()
            .map(|(index, observation)| StewardModelObservation {
                reference: (index + 1) as u32,
                occurred_at: observation.occurred_at.clone(),
                statement: observation.statement.clone(),
                certainty: observation.certainty,
            })
            .collect(),
        memory,
    }
}

fn normalize_steward_output(
    output: StewardModelOutput,
    packet: &StewardPacket,
) -> Result<ReconciliationOutput, InsightsError> {
    if output.schema_version != 1 {
        return Err(InsightsError::Invalid(
            "wrong packet-local Steward schema version".into(),
        ));
    }
    if output.candidates.len() > 8 {
        return Err(InsightsError::Invalid("too many Steward candidates".into()));
    }
    let observation_by_ref = packet
        .observations
        .iter()
        .enumerate()
        .map(|(index, observation)| ((index + 1) as u32, observation))
        .collect::<HashMap<_, _>>();
    let memory_ids = packet
        .memory
        .get("opportunities")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("opportunity_id").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut claimed_refs = HashSet::new();
    let mut opportunities = Vec::new();
    let mut assessments = Vec::new();
    for (candidate_index, candidate) in output.candidates.into_iter().enumerate() {
        if candidate.observation_groups.is_empty() {
            return Err(InsightsError::Invalid(format!(
                "candidate {} has no observation groups",
                candidate_index + 1
            )));
        }
        let mut candidate_observation_ids = Vec::new();
        let mut occurrences = Vec::new();
        for (group_index, group) in candidate.observation_groups.into_iter().enumerate() {
            if group.observation_refs.is_empty() {
                return Err(InsightsError::Invalid(format!(
                    "candidate {} group {} has no observation refs",
                    candidate_index + 1,
                    group_index + 1
                )));
            }
            let mut local_refs = HashSet::new();
            let mut observation_ids = Vec::new();
            let mut evidence_ids = Vec::new();
            for reference in group.observation_refs {
                if !local_refs.insert(reference) {
                    return Err(InsightsError::Invalid(format!(
                        "candidate {} group {} repeats observation ref {reference}",
                        candidate_index + 1,
                        group_index + 1
                    )));
                }
                if !claimed_refs.insert(reference) {
                    return Err(InsightsError::Invalid(format!(
                        "observation ref {reference} belongs to multiple candidates or episodes"
                    )));
                }
                let observation = observation_by_ref.get(&reference).ok_or_else(|| {
                    InsightsError::Invalid(format!("observation ref {reference} is outside packet"))
                })?;
                observation_ids.push(observation.observation_id.clone());
                evidence_ids.extend(observation.evidence_ids.clone());
            }
            evidence_ids.sort();
            evidence_ids.dedup();
            candidate_observation_ids.extend(observation_ids.clone());
            occurrences.push(OccurrenceDelta {
                local_id: format!(
                    "candidate_{}_episode_{}",
                    candidate_index + 1,
                    group_index + 1
                ),
                observation_ids,
                evidence_ids,
                steps: group.steps,
                distinctness_basis: group.distinctness_basis,
            });
        }
        candidate_observation_ids.sort();
        candidate_observation_ids.dedup();
        let opportunity_local_id = match candidate.decision {
            CandidateDecision::Discarded => {
                if candidate.opportunity.is_some() {
                    return Err(InsightsError::Invalid(
                        "discarded candidate has an opportunity".into(),
                    ));
                }
                None
            }
            CandidateDecision::Qualified | CandidateDecision::Watching => {
                let opportunity = candidate.opportunity.ok_or_else(|| {
                    InsightsError::Invalid(
                        "qualified or watching candidate has no opportunity".into(),
                    )
                })?;
                if candidate.decision == CandidateDecision::Qualified
                    && (opportunity.finding.is_none() || opportunity.handoff.is_none())
                {
                    return Err(InsightsError::Invalid(
                        "qualified candidate needs a finding and handoff".into(),
                    ));
                }
                if candidate.decision == CandidateDecision::Watching
                    && opportunity.finding.is_some()
                {
                    return Err(InsightsError::Invalid(
                        "watching candidate has a finding".into(),
                    ));
                }
                let existing_opportunity_id = opportunity
                    .existing_opportunity_ref
                    .map(|reference| {
                        memory_ids
                            .get(reference.saturating_sub(1) as usize)
                            .cloned()
                            .ok_or_else(|| {
                                InsightsError::Invalid(format!(
                                    "opportunity memory ref {reference} is outside packet"
                                ))
                            })
                    })
                    .transpose()?;
                let local_id = format!("opportunity_{}", candidate_index + 1);
                let finding = opportunity.finding.map(|finding| {
                    let mut evidence_ids = occurrences
                        .iter()
                        .flat_map(|occurrence| occurrence.evidence_ids.clone())
                        .collect::<HashSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>();
                    evidence_ids.sort();
                    FindingDraft {
                        claim: finding.claim,
                        why_worth_fixing: finding.why_worth_fixing,
                        evidence_ids,
                    }
                });
                opportunities.push(OpportunityDelta {
                    local_id: local_id.clone(),
                    existing_opportunity_id,
                    construct: opportunity.construct,
                    summary: opportunity.summary,
                    signature: opportunity.signature,
                    occurrences_to_add: occurrences,
                    withdraw_current_finding: opportunity.withdraw_current_finding,
                    retire: opportunity.retire,
                    transfer_established: opportunity.transfer_established,
                    authorship_established: opportunity.authorship_established,
                    cadence: opportunity.cadence,
                    unresolved_questions: opportunity.unresolved_questions,
                    evidence_quality: opportunity.evidence_quality,
                    handoff: opportunity.handoff,
                    finding,
                    rank_signals: opportunity.rank_signals,
                    automation_potential: opportunity.automation_potential,
                });
                Some(local_id)
            }
        };
        assessments.push(CandidateAssessment {
            local_id: format!("candidate_{}", candidate_index + 1),
            observation_ids: candidate_observation_ids,
            decision: candidate.decision,
            reason_code: candidate.reason_code,
            reason: candidate.reason,
            shared_goal: candidate.shared_goal,
            reducible_burden: candidate.reducible_burden,
            stable_steps: candidate.stable_steps,
            variable_inputs: candidate.variable_inputs,
            distinct_episode_basis: candidate.distinct_episode_basis,
            missing_to_qualify: candidate.missing_to_qualify,
            opportunity_local_id,
        });
    }
    if opportunities.len() > 6 {
        return Err(InsightsError::Invalid(
            "too many Steward opportunities".into(),
        ));
    }
    Ok(ReconciliationOutput {
        schema_version: 3,
        considered_observation_ids: packet
            .observations
            .iter()
            .map(|observation| observation.observation_id.clone())
            .collect(),
        opportunities,
        candidate_assessments: assessments,
    })
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
    reasoning_effort: AiReasoningEffort,
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
            reasoning_effort,
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
        STEWARD_REASONING_EFFORT,
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
        STEWARD_REASONING_EFFORT,
    )
    .await
}

/// Backfill-only Steward replay with an explicit provider reasoning setting.
pub async fn run_steward_replay_wake_with_reasoning<R: AiRuntime + ?Sized>(
    pool: &SqlitePool,
    runtime: &R,
    local_day: &str,
    timezone: &str,
    reason: &str,
    observation_limit: u32,
    replay_nonce: &str,
    reasoning_effort: AiReasoningEffort,
) -> EngineResult<WakeResult> {
    run_steward_wake_inner(
        pool,
        runtime,
        local_day,
        timezone,
        reason,
        observation_limit,
        Some(replay_nonce),
        reasoning_effort,
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
    reasoning_effort: AiReasoningEffort,
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
            schema_version: 2,
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

    let packet: StewardPacket = serde_json::from_str(&packet_json)?;
    let model_packet_json = serde_json::to_string(&model_packet(&packet))?;
    let mut prompt = request_prompt(&model_packet_json);
    let mut last_error = String::new();
    for attempt_index in 0..=1 {
        let request_fingerprint = hash_bytes(
            format!(
                "{}\n{}\n{}\n{:?}\n{}",
                model,
                STEWARD_PROMPT_VERSION,
                hash_bytes(&serde_json::to_vec(&schema())?),
                reasoning_effort,
                prompt
            )
            .as_bytes(),
        );
        let run = match infer_once(
            runtime,
            "worth_fixing_steward",
            prompt.clone(),
            reasoning_effort,
        )
        .await
        {
            Ok(run) => run,
            Err(error) => {
                let schema_bytes = serde_json::to_vec(&schema())?.len();
                let error_message = error.message.chars().take(1000).collect::<String>();
                let diagnostics = serde_json::json!({
                    "request_prompt_bytes": prompt.len(),
                    "packet_bytes": packet_json.len(),
                    "schema_bytes": schema_bytes,
                    "reasoning_effort": format!("{reasoning_effort:?}").to_lowercase(),
                    "provider_error_code": format!("{:?}", error.code),
                    "provider_error_message": error_message,
                });
                record_job_attempt(
                    pool,
                    &job_id,
                    &request_fingerprint,
                    None,
                    "provider_error",
                    &diagnostics,
                    0,
                    Some(&format!("{:?}", error.code)),
                )
                .await?;
                mark_job_failed(pool, &job_id, &format!("{:?}", error.code)).await?;
                return Err(error.into());
            }
        };
        let output_fingerprint = hash_bytes(&serde_json::to_vec(&run.output)?);
        let parsed = serde_json::from_value::<StewardModelOutput>(run.output.clone())
            .map_err(|error| InsightsError::Invalid(error.to_string()))
            .and_then(|output| normalize_steward_output(output, &packet));
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
        let mut invalid_diagnostics = serde_json::to_value(&run.usage)?;
        if let Some(diagnostics) = invalid_diagnostics.as_object_mut() {
            diagnostics.insert(
                "validation_error".into(),
                last_error.chars().take(1000).collect::<String>().into(),
            );
            diagnostics.insert(
                "response_bytes".into(),
                serde_json::to_vec(&run.output)?.len().into(),
            );
            if retain_raw_steward_response(reason) {
                diagnostics.insert("response_json".into(), run.output.clone());
            }
            diagnostics.insert(
                "reasoning_effort".into(),
                format!("{reasoning_effort:?}").to_lowercase().into(),
            );
        }
        record_job_attempt(
            pool,
            &job_id,
            &request_fingerprint,
            Some(&output_fingerprint),
            "invalid_output",
            &invalid_diagnostics,
            run.elapsed_ms,
            Some("invalid_output"),
        )
        .await?;
        if attempt_index == 0 {
            prompt = repair_prompt(&model_packet_json, &run.output, &last_error);
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

    fn model_candidate_json(refs: Vec<u32>) -> Value {
        json!({
            "schema_version": 1,
            "candidates": [{
                "observation_groups": [{"observation_refs": refs, "steps": ["Reviewed and sent"], "distinctness_basis": ["One completed task"]}],
                "decision": "qualified", "reason_code": "meaningful_repeated_work",
                "reason": "A reusable prompt helps.", "shared_goal": "Prepare an RFQ.",
                "reducible_burden": "Repeated composition.", "stable_steps": ["Review", "Send"],
                "variable_inputs": ["Requirement"], "distinct_episode_basis": ["A completed task"],
                "missing_to_qualify": [],
                "opportunity": {
                    "existing_opportunity_ref": null, "construct": "recognition",
                    "summary": "Draft RFQs", "signature": "draft-rfqs",
                    "withdraw_current_finding": false, "retire": false,
                    "transfer_established": false, "authorship_established": true,
                    "cadence": "none", "unresolved_questions": [], "evidence_quality": "high",
                    "handoff": {"kind": "prompt", "title": "Draft RFQ", "body": "Draft an RFQ from supplied requirements.", "capability_id": null},
                    "finding": {"claim": "An RFQ was drafted.", "why_worth_fixing": "This saves composition work."},
                    "rank_signals": {"actionability": 3, "estimated_burden": 2, "novelty": 2, "user_relevance": 3, "sensitivity_risk": 0},
                    "automation_potential": false
                }
            }]
        })
    }

    #[test]
    fn model_packet_hides_durable_observation_and_evidence_ids() {
        let packet = StewardPacket {
            schema_version: 2,
            prompt_version: STEWARD_PROMPT_VERSION.into(),
            local_day: "2026-08-02".into(),
            timezone: "+05:30".into(),
            observations: vec![ObservationRecord {
                observation_id: "obl_durable_secret".into(),
                source_key: "source-secret".into(),
                occurred_at: "2026-08-02T10:00:00Z".into(),
                statement: "Reviewed a requirement".into(),
                certainty: ObservationCertainty::Explicit,
                evidence_ids: vec!["evidence-secret".into()],
            }],
            memory: json!({"opportunities": [{"opportunity_id": "wfo_secret", "construct": "recognition", "status": "watching", "summary": "Draft RFQ", "cadence": "none", "occurrence_count": 1}]}),
        };
        let rendered = serde_json::to_string(&model_packet(&packet)).unwrap();
        assert!(rendered.contains("\"ref\":1"));
        assert!(!rendered.contains("obl_durable_secret"));
        assert!(!rendered.contains("source-secret"));
        assert!(!rendered.contains("evidence-secret"));
        assert!(!rendered.contains("wfo_secret"));
    }

    #[test]
    fn packet_refs_derive_precise_observation_and_finding_evidence() {
        let packet = StewardPacket {
            schema_version: 2,
            prompt_version: STEWARD_PROMPT_VERSION.into(),
            local_day: "2026-08-02".into(),
            timezone: "+05:30".into(),
            observations: vec![
                ObservationRecord {
                    observation_id: "obl_1".into(),
                    source_key: "one".into(),
                    occurred_at: "2026-08-02T10:00:00Z".into(),
                    statement: "Reviewed".into(),
                    certainty: ObservationCertainty::Explicit,
                    evidence_ids: vec!["ev_1".into()],
                },
                ObservationRecord {
                    observation_id: "obl_2".into(),
                    source_key: "two".into(),
                    occurred_at: "2026-08-02T10:01:00Z".into(),
                    statement: "Sent".into(),
                    certainty: ObservationCertainty::Explicit,
                    evidence_ids: vec!["ev_2".into()],
                },
                ObservationRecord {
                    observation_id: "obl_3".into(),
                    source_key: "three".into(),
                    occurred_at: "2026-08-02T10:02:00Z".into(),
                    statement: "Unrelated".into(),
                    certainty: ObservationCertainty::Explicit,
                    evidence_ids: vec!["ev_unselected".into()],
                },
            ],
            memory: json!({"opportunities": []}),
        };
        let raw: StewardModelOutput =
            serde_json::from_value(model_candidate_json(vec![1, 2])).unwrap();
        let normalized = normalize_steward_output(raw, &packet).unwrap();
        assert_eq!(
            normalized.opportunities[0].occurrences_to_add[0].observation_ids,
            vec!["obl_1", "obl_2"]
        );
        assert_eq!(
            normalized.opportunities[0].occurrences_to_add[0].evidence_ids,
            vec!["ev_1", "ev_2"]
        );
        assert_eq!(
            normalized.opportunities[0]
                .finding
                .as_ref()
                .unwrap()
                .evidence_ids,
            vec!["ev_1", "ev_2"]
        );
        assert!(!normalized.opportunities[0]
            .finding
            .as_ref()
            .unwrap()
            .evidence_ids
            .contains(&"ev_unselected".into()));
    }

    #[tokio::test]
    async fn repairs_once_then_commits_and_does_not_recall_on_resume() {
        let (_dir, pool, _observation_id) = fixture().await;
        let runtime = MockRuntime::new(vec![
            json!({"bad": true}),
            json!({
                "schema_version": 1,
                "candidates": [{
                    "observation_groups": [{"observation_refs": [1], "steps": ["Copied"], "distinctness_basis": []}],
                    "decision": "discarded", "reason_code": "system_mechanics", "reason": "Noise.",
                    "shared_goal": "", "reducible_burden": "", "stable_steps": [], "variable_inputs": [],
                    "distinct_episode_basis": [], "missing_to_qualify": [], "opportunity": null
                }]
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
    async fn invalid_production_attempt_omits_raw_model_response() {
        let (_dir, pool, _observation_id) = fixture().await;
        let runtime = MockRuntime::new(vec![json!({"bad": true}), json!({"bad": true})]);
        assert!(run_steward_wake(
            &pool,
            &runtime,
            "2026-08-02",
            "Asia/Kolkata",
            "explicit_request",
            50,
        )
        .await
        .is_err());
        let usage: String = sqlx::query_scalar("SELECT usage_json FROM job_attempts LIMIT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(serde_json::from_str::<Value>(&usage)
            .unwrap()
            .get("response_json")
            .is_none());
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
        let (_dir, pool, _observation_id) = fixture().await;
        let runtime = MockRuntime::new(vec![
            json!({
                "schema_version": 1,
                "candidates": [{
                    "observation_groups": [{"observation_refs": [1, 1], "steps": ["Copied"], "distinctness_basis": []}],
                    "decision": "discarded", "reason_code": "system_mechanics", "reason": "Noise.",
                    "shared_goal": "", "reducible_burden": "", "stable_steps": [], "variable_inputs": [],
                    "distinct_episode_basis": [], "missing_to_qualify": [], "opportunity": null
                }]
            }),
            json!({
                "schema_version": 1,
                "candidates": [{
                    "observation_groups": [{"observation_refs": [1], "steps": ["Copied"], "distinctness_basis": []}],
                    "decision": "discarded", "reason_code": "system_mechanics", "reason": "Noise.",
                    "shared_goal": "", "reducible_burden": "", "stable_steps": [], "variable_inputs": [],
                    "distinct_episode_basis": [], "missing_to_qualify": [], "opportunity": null
                }]
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
        assert!(!rendered.contains("observation_ids"));
        assert!(!rendered.contains("evidence_ids"));
        assert!(!rendered.contains("considered_observation_ids"));
        assert!(!STEWARD_PROMPT.contains("grounding judge"));
        assert!(!EXPLORER_PROMPT.contains("grounding judge"));
        let _compile_guard = PathBuf::from("provider-neutral");
    }

    #[test]
    fn production_prompts_define_meaningful_work_and_distinct_episodes() {
        assert!(EXPLORER_PROMPT.contains("intentional user activity"));
        assert!(EXPLORER_PROMPT.contains("Reappearing text is evidence, not itself a task"));
        assert!(STEWARD_PROMPT.contains("Worth Fixing concerns intentional user work"));
        assert!(STEWARD_PROMPT.contains("packet-local integer `ref`"));
        assert!(STEWARD_PROMPT.contains("An observation group is one distinct"));
        assert!(STEWARD_PROMPT.contains("at most 8 candidates and 6 opportunities"));
        assert_eq!(schema()["properties"]["candidates"]["maxItems"], 8);
    }

    #[test]
    fn occurrence_schema_allows_grouping_observations_into_one_episode() {
        let occurrence = schema()["properties"]["candidates"]["items"]["properties"]
            ["observation_groups"]["items"]["properties"]["observation_refs"]
            .clone();
        assert_eq!(occurrence["type"], "array");
        assert_eq!(occurrence["minItems"], 1);
        assert_eq!(occurrence["items"]["type"], "integer");
        assert_eq!(occurrence["maxItems"], 40);
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
