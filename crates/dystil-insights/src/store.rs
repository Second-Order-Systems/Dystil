use std::{collections::HashSet, path::Path, str::FromStr, time::Duration};

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    Row, SqlitePool,
};
use thiserror::Error;

use crate::{
    cadence_supported, derive_eligibility, handoff_preview, rank, select_top, user_label, Cadence,
    CandidateDecision, Construct, DispositionKind, EligibilityContext, EvidenceRecord,
    FindingCandidate, FindingPage, HandoffType, ObservationCertainty, ObservationRecord,
    OpportunityStatus, ReconciliationOutput, WorthFixingCard, WorthFixingEvidenceLine,
    WorthFixingSummary,
};

const SCHEMA_VERSION: i64 = 5;
const MANUAL_REFRESH_MIN_OBSERVATION_SPAN_HOURS: f64 = 3.0;

#[derive(Debug, Error)]
pub enum InsightsError {
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid Worth Fixing state: {0}")]
    Invalid(String),
    #[error("identity collision for {0}")]
    IdentityCollision(String),
    #[error("unsupported Worth Fixing schema version {0}")]
    UnsupportedSchema(i64),
}

pub type Result<T> = std::result::Result<T, InsightsError>;

fn has_duplicates(values: &[String]) -> bool {
    let mut seen = HashSet::with_capacity(values.len());
    values.iter().any(|value| !seen.insert(value))
}

fn validate_candidate_assessments(
    output: &ReconciliationOutput,
    expected_observation_ids: &[String],
) -> Result<()> {
    if output.candidate_assessments.len() > 8 {
        return Err(InsightsError::Invalid(
            "reconciliation has too many candidate assessments".into(),
        ));
    }

    let expected: HashSet<&str> = expected_observation_ids
        .iter()
        .map(String::as_str)
        .collect();
    let opportunity_ids: HashSet<&str> = output
        .opportunities
        .iter()
        .map(|opportunity| opportunity.local_id.as_str())
        .collect();
    if output
        .opportunities
        .iter()
        .any(|opportunity| opportunity.local_id.is_empty())
    {
        return Err(InsightsError::Invalid(
            "opportunity local_id is empty".into(),
        ));
    }
    if opportunity_ids.len() != output.opportunities.len() {
        return Err(InsightsError::Invalid(
            "reconciliation repeats an opportunity local_id".into(),
        ));
    }

    let mut assessment_ids = HashSet::new();
    let mut covered_observations = HashSet::new();
    let mut linked_opportunities = HashSet::new();
    if !expected_observation_ids.is_empty() && output.candidate_assessments.is_empty() {
        return Err(InsightsError::Invalid(
            "reconciliation has no candidate assessments".into(),
        ));
    }
    for assessment in &output.candidate_assessments {
        if assessment.local_id.is_empty() || !assessment_ids.insert(assessment.local_id.as_str()) {
            return Err(InsightsError::Invalid(
                "candidate assessment local_id is empty or repeated".into(),
            ));
        }
        if assessment.observation_ids.is_empty() || has_duplicates(&assessment.observation_ids) {
            return Err(InsightsError::Invalid(
                "candidate assessment has empty or repeated observations".into(),
            ));
        }
        for observation_id in &assessment.observation_ids {
            if !expected.contains(observation_id.as_str()) {
                return Err(InsightsError::Invalid(
                    "candidate assessment uses an observation outside its job".into(),
                ));
            }
            if !covered_observations.insert(observation_id.as_str()) {
                return Err(InsightsError::Invalid(
                    "candidate assessments overlap observations".into(),
                ));
            }
        }

        match assessment.decision {
            CandidateDecision::Qualified => {
                let Some(opportunity_local_id) = assessment.opportunity_local_id.as_deref() else {
                    return Err(InsightsError::Invalid(
                        "qualified assessment has no opportunity".into(),
                    ));
                };
                let opportunity = output
                    .opportunities
                    .iter()
                    .find(|opportunity| opportunity.local_id == opportunity_local_id)
                    .ok_or_else(|| {
                        InsightsError::Invalid(
                            "candidate assessment references an unknown opportunity".into(),
                        )
                    })?;
                if opportunity.finding.is_none() {
                    return Err(InsightsError::Invalid(
                        "qualified assessment opportunity has no finding".into(),
                    ));
                }
                if !linked_opportunities.insert(opportunity_local_id) {
                    return Err(InsightsError::Invalid(
                        "opportunity is linked from multiple assessments".into(),
                    ));
                }
            }
            CandidateDecision::Watching => {
                let Some(opportunity_local_id) = assessment.opportunity_local_id.as_deref() else {
                    return Err(InsightsError::Invalid(
                        "watching assessment has no opportunity".into(),
                    ));
                };
                let opportunity = output
                    .opportunities
                    .iter()
                    .find(|opportunity| opportunity.local_id == opportunity_local_id)
                    .ok_or_else(|| {
                        InsightsError::Invalid(
                            "candidate assessment references an unknown opportunity".into(),
                        )
                    })?;
                if opportunity.finding.is_some() || assessment.missing_to_qualify.is_empty() {
                    return Err(InsightsError::Invalid(
                        "watching assessment must explain missing qualification evidence".into(),
                    ));
                }
                if !linked_opportunities.insert(opportunity_local_id) {
                    return Err(InsightsError::Invalid(
                        "opportunity is linked from multiple assessments".into(),
                    ));
                }
            }
            CandidateDecision::Discarded => {
                if assessment.opportunity_local_id.is_some() {
                    return Err(InsightsError::Invalid(
                        "discarded assessment has an opportunity".into(),
                    ));
                }
            }
        }
    }

    if linked_opportunities.len() != opportunity_ids.len()
        || opportunity_ids
            .iter()
            .any(|local_id| !linked_opportunities.contains(local_id))
    {
        return Err(InsightsError::Invalid(
            "every opportunity must have exactly one candidate assessment".into(),
        ));
    }
    Ok(())
}

pub(crate) fn fingerprint<T: Serialize>(value: &T) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

pub(crate) fn stable_id<T: Serialize>(prefix: &str, value: &T) -> Result<String> {
    Ok(format!("{prefix}_{}", &fingerprint(value)?[..24]))
}

fn parse_construct(value: &str) -> Result<Construct> {
    match value {
        "recognition" => Ok(Construct::Recognition),
        "manual_transfer" => Ok(Construct::ManualTransfer),
        "unchanged_repetition" => Ok(Construct::UnchangedRepetition),
        "temporal_pattern" => Ok(Construct::TemporalPattern),
        "repeated_composition" => Ok(Construct::RepeatedComposition),
        _ => Err(InsightsError::Invalid(format!("unknown construct {value}"))),
    }
}

fn parse_cadence(value: &str) -> Result<Cadence> {
    match value {
        "none" => Ok(Cadence::None),
        "daily" => Ok(Cadence::Daily),
        "weekly" => Ok(Cadence::Weekly),
        "monthly" => Ok(Cadence::Monthly),
        _ => Err(InsightsError::Invalid(format!("unknown cadence {value}"))),
    }
}

fn cadence_str(value: Cadence) -> &'static str {
    match value {
        Cadence::None => "none",
        Cadence::Daily => "daily",
        Cadence::Weekly => "weekly",
        Cadence::Monthly => "monthly",
    }
}

fn certainty_str(value: ObservationCertainty) -> &'static str {
    match value {
        ObservationCertainty::Explicit => "explicit",
        ObservationCertainty::StronglyImplied => "strongly_implied",
        ObservationCertainty::Tentative => "tentative",
    }
}

fn parse_certainty(value: &str) -> Result<ObservationCertainty> {
    match value {
        "explicit" => Ok(ObservationCertainty::Explicit),
        "strongly_implied" => Ok(ObservationCertainty::StronglyImplied),
        "tentative" => Ok(ObservationCertainty::Tentative),
        _ => Err(InsightsError::Invalid(format!("unknown certainty {value}"))),
    }
}

pub async fn open_insights_database(path: impl AsRef<Path>) -> Result<SqlitePool> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent).map_err(|error| sqlx::Error::Io(error.into()))?;
    }
    let options = SqliteConnectOptions::from_str(path.as_ref().to_string_lossy().as_ref())?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Full)
        .busy_timeout(Duration::from_secs(30))
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await?;
    migrate(&pool).await?;
    Ok(pool)
}

async fn migrate(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS insights_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS insights_schema_migrations(
         version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT value FROM insights_metadata WHERE key='schema_version'")
            .fetch_optional(&mut *tx)
            .await?;
    let current_version = existing
        .as_deref()
        .map(|value| value.parse::<i64>().unwrap_or(-1))
        .unwrap_or(0);
    if !(0..=SCHEMA_VERSION).contains(&current_version) {
        return Err(InsightsError::UnsupportedSchema(current_version));
    }
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS evidence(
          evidence_id TEXT PRIMARY KEY, source_namespace TEXT NOT NULL, source_id TEXT NOT NULL,
          occurred_at TEXT NOT NULL, app TEXT, window TEXT, excerpt TEXT NOT NULL,
          immutable_fingerprint TEXT NOT NULL, policy_allowed INTEGER NOT NULL,
          redaction_ready INTEGER NOT NULL, deleted INTEGER NOT NULL, sensitive INTEGER NOT NULL,
          UNIQUE(source_namespace,source_id))",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS capabilities(
          capability_id TEXT PRIMARY KEY, app TEXT NOT NULL, description TEXT NOT NULL,
          immutable_fingerprint TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS observations(
          sequence INTEGER PRIMARY KEY AUTOINCREMENT, observation_id TEXT NOT NULL UNIQUE,
          source_key TEXT NOT NULL UNIQUE, occurred_at TEXT NOT NULL, statement TEXT NOT NULL,
          certainty TEXT NOT NULL, evidence_ids_json TEXT NOT NULL, immutable_fingerprint TEXT NOT NULL)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS inference_jobs(
          job_id TEXT PRIMARY KEY, input_fingerprint TEXT NOT NULL UNIQUE, local_day TEXT NOT NULL,
          reason TEXT NOT NULL, status TEXT NOT NULL, prompt_hash TEXT, schema_hash TEXT, model TEXT,
          input_json TEXT NOT NULL,
          attempts INTEGER NOT NULL DEFAULT 0, error_code TEXT, created_at TEXT NOT NULL,
          updated_at TEXT NOT NULL, accepted_at TEXT)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS explorer_jobs(
          job_id TEXT PRIMARY KEY,batch_id TEXT NOT NULL UNIQUE,input_fingerprint TEXT NOT NULL UNIQUE,
          status TEXT NOT NULL,input_json TEXT NOT NULL,prompt_hash TEXT NOT NULL,schema_hash TEXT NOT NULL,
          model TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,error_code TEXT,
          created_at TEXT NOT NULL,updated_at TEXT NOT NULL,accepted_at TEXT)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS explorer_attempts(
          job_id TEXT NOT NULL REFERENCES explorer_jobs(job_id),attempt INTEGER NOT NULL,
          request_fingerprint TEXT NOT NULL,output_fingerprint TEXT,status TEXT NOT NULL,
          usage_json TEXT NOT NULL,latency_ms INTEGER NOT NULL,error_code TEXT,created_at TEXT NOT NULL,
          PRIMARY KEY(job_id,attempt))",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS one_active_insights_job
         ON inference_jobs((1)) WHERE status IN ('pending','running')",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS job_attempts(
          job_id TEXT NOT NULL REFERENCES inference_jobs(job_id), attempt INTEGER NOT NULL,
          request_fingerprint TEXT NOT NULL, output_fingerprint TEXT,
          status TEXT NOT NULL, usage_json TEXT NOT NULL, latency_ms INTEGER NOT NULL,
          error_code TEXT, created_at TEXT NOT NULL, PRIMARY KEY(job_id,attempt))",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS job_observations(
          job_id TEXT NOT NULL REFERENCES inference_jobs(job_id), observation_id TEXT NOT NULL
          REFERENCES observations(observation_id), ordinal INTEGER NOT NULL,
          PRIMARY KEY(job_id,observation_id), UNIQUE(observation_id))",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS reconciliations(
          reconciliation_id TEXT PRIMARY KEY, job_id TEXT NOT NULL UNIQUE REFERENCES inference_jobs(job_id),
          output_fingerprint TEXT NOT NULL, output_json TEXT NOT NULL, accepted_at TEXT NOT NULL)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS opportunities(
          opportunity_id TEXT PRIMARY KEY, construct TEXT NOT NULL, signature TEXT NOT NULL,
          current_status TEXT NOT NULL, current_version INTEGER NOT NULL, current_summary TEXT NOT NULL,
          cadence TEXT NOT NULL, updated_at TEXT NOT NULL)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS opportunity_versions(
          version_id TEXT PRIMARY KEY, opportunity_id TEXT NOT NULL REFERENCES opportunities(opportunity_id),
          ordinal INTEGER NOT NULL, job_id TEXT NOT NULL REFERENCES inference_jobs(job_id),
          status TEXT NOT NULL, proposal_json TEXT NOT NULL, eligibility_json TEXT NOT NULL,
          created_at TEXT NOT NULL, UNIQUE(opportunity_id,ordinal))",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS occurrences(
          occurrence_id TEXT PRIMARY KEY, opportunity_id TEXT NOT NULL REFERENCES opportunities(opportunity_id),
          observation_ids_json TEXT NOT NULL, evidence_ids_json TEXT NOT NULL,
          proposal_json TEXT NOT NULL, started_at TEXT NOT NULL, ended_at TEXT NOT NULL,
          UNIQUE(opportunity_id,observation_ids_json,evidence_ids_json))",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS findings(
          finding_id TEXT PRIMARY KEY, opportunity_id TEXT NOT NULL REFERENCES opportunities(opportunity_id),
          version_id TEXT NOT NULL REFERENCES opportunity_versions(version_id), active INTEGER NOT NULL,
          construct TEXT NOT NULL, label TEXT NOT NULL, claim TEXT NOT NULL,
          why_worth_fixing TEXT NOT NULL, handoff_type TEXT NOT NULL, handoff_title TEXT NOT NULL,
          handoff_preview TEXT NOT NULL, occurrence_count INTEGER NOT NULL, cadence TEXT NOT NULL,
          rank_score INTEGER NOT NULL, rank_vector_json TEXT NOT NULL, created_at TEXT NOT NULL)",
    ).execute(&mut *tx).await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS finding_evidence(
          finding_id TEXT NOT NULL REFERENCES findings(finding_id), evidence_id TEXT NOT NULL
          REFERENCES evidence(evidence_id), PRIMARY KEY(finding_id,evidence_id))",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS dispositions(
          disposition_id TEXT PRIMARY KEY, finding_id TEXT NOT NULL REFERENCES findings(finding_id),
          kind TEXT NOT NULL, correction_text TEXT, intent TEXT, created_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS insights_cursor(
          stream TEXT PRIMARY KEY, last_observation_sequence INTEGER NOT NULL,
          last_job_id TEXT, updated_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS wake_starts(
          wake_id TEXT PRIMARY KEY,local_day TEXT NOT NULL,reason TEXT NOT NULL,
          normal INTEGER NOT NULL,started_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS capture_cursors(
          source TEXT PRIMARY KEY,last_row_id INTEGER NOT NULL,updated_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS compaction_checkpoints(
          stream TEXT PRIMARY KEY,state_json TEXT NOT NULL,updated_at TEXT NOT NULL)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO insights_cursor(stream,last_observation_sequence,updated_at)
         VALUES('explorer',0,datetime('now'))",
    )
    .execute(&mut *tx)
    .await?;
    if current_version < 1 {
        sqlx::query(
            "INSERT OR IGNORE INTO insights_schema_migrations(version,applied_at) VALUES(1,?1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    if current_version < 2 {
        sqlx::query("ALTER TABLE findings ADD COLUMN handoff_body TEXT NOT NULL DEFAULT ''")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE capabilities ADD COLUMN action_kind TEXT")
            .execute(&mut *tx)
            .await?;
        sqlx::query("ALTER TABLE capabilities ADD COLUMN action_target TEXT")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "CREATE TABLE artifacts(
              artifact_id TEXT PRIMARY KEY,source_kind TEXT NOT NULL,
              source_finding_id TEXT UNIQUE REFERENCES findings(finding_id),source_request_id TEXT,
              kind TEXT NOT NULL,title TEXT NOT NULL,current_version INTEGER NOT NULL,
              status TEXT NOT NULL,capability_id TEXT REFERENCES capabilities(capability_id),
              kept_at TEXT NOT NULL,last_used_at TEXT,updated_at TEXT NOT NULL,removed_at TEXT)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE artifact_change_jobs(
              job_id TEXT PRIMARY KEY,artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id),
              base_version INTEGER NOT NULL,request_text TEXT NOT NULL,input_fingerprint TEXT NOT NULL UNIQUE,
              status TEXT NOT NULL,input_json TEXT NOT NULL,prompt_hash TEXT NOT NULL,schema_hash TEXT NOT NULL,
              model TEXT NOT NULL,proposed_title TEXT,proposed_body TEXT,attempts INTEGER NOT NULL DEFAULT 0,
              error_code TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,accepted_at TEXT)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX one_active_artifact_change_job ON artifact_change_jobs(artifact_id)
             WHERE status IN ('pending','running','preview_ready')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE artifact_versions(
              version_id TEXT PRIMARY KEY,artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id),
              ordinal INTEGER NOT NULL,title TEXT NOT NULL,body TEXT NOT NULL,
              source_finding_version_id TEXT REFERENCES opportunity_versions(version_id),
              change_job_id TEXT REFERENCES artifact_change_jobs(job_id),created_at TEXT NOT NULL,
              UNIQUE(artifact_id,ordinal))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE artifact_events(
              event_id TEXT PRIMARY KEY,artifact_id TEXT NOT NULL REFERENCES artifacts(artifact_id),
              event_type TEXT NOT NULL,action TEXT,created_at TEXT NOT NULL)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE artifact_change_attempts(
              job_id TEXT NOT NULL REFERENCES artifact_change_jobs(job_id),attempt INTEGER NOT NULL,
              request_fingerprint TEXT NOT NULL,output_fingerprint TEXT,status TEXT NOT NULL,
              usage_json TEXT NOT NULL,latency_ms INTEGER NOT NULL,error_code TEXT,created_at TEXT NOT NULL,
              PRIMARY KEY(job_id,attempt))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO insights_schema_migrations(version,applied_at) VALUES(2,?1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
        let skipped_dispositions = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dispositions WHERE kind IN ('accepted','saved')",
        )
        .fetch_one(&mut *tx)
        .await?;
        let incomplete_findings = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM findings WHERE active=1 AND handoff_body=''",
        )
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE opportunities SET current_status='withdrawn' WHERE opportunity_id IN
             (SELECT opportunity_id FROM findings WHERE active=1 AND handoff_body='')",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query("UPDATE findings SET active=0 WHERE active=1 AND handoff_body=''")
            .execute(&mut *tx)
            .await?;
        for (key, value) in [
            ("legacy_artifact_dispositions_skipped", skipped_dispositions),
            ("legacy_incomplete_findings_withdrawn", incomplete_findings),
        ] {
            sqlx::query(
                "INSERT INTO insights_metadata(key,value) VALUES(?1,?2)
                 ON CONFLICT(key) DO UPDATE SET value=?2",
            )
            .bind(key)
            .bind(value.to_string())
            .execute(&mut *tx)
            .await?;
        }
    }
    if current_version < 3 {
        sqlx::query("ALTER TABLE observations ADD COLUMN admitted_at TEXT")
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE observations SET admitted_at=COALESCE(
               (SELECT accepted_at FROM explorer_jobs
                WHERE observations.source_key LIKE explorer_jobs.batch_id || ':%'
                ORDER BY accepted_at LIMIT 1),
               occurred_at)
             WHERE admitted_at IS NULL",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO insights_schema_migrations(version,applied_at) VALUES(3,?1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    if current_version < 4 {
        sqlx::query(
            "CREATE TABLE ask_sessions(
              session_id TEXT PRIMARY KEY,phase TEXT NOT NULL,status TEXT NOT NULL,
              question_count INTEGER NOT NULL,understanding_json TEXT NOT NULL,
              pending_move_json TEXT,locked_understanding_json TEXT,presentation_json TEXT,
              last_error_code TEXT,last_error_detail TEXT,provider TEXT,model TEXT,
              artifact_kept_id TEXT REFERENCES artifacts(artifact_id),
              created_at TEXT NOT NULL,updated_at TEXT NOT NULL)",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE ask_messages(
              message_id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES ask_sessions(session_id),
              ordinal INTEGER NOT NULL,role TEXT NOT NULL,text TEXT NOT NULL,event_json TEXT,
              created_at TEXT NOT NULL,UNIQUE(session_id,ordinal))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE ask_questions(
              question_id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES ask_sessions(session_id),
              ordinal INTEGER NOT NULL,question_text TEXT NOT NULL,question_json TEXT NOT NULL,
              created_at TEXT NOT NULL,UNIQUE(session_id,ordinal))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE ask_jobs(
              job_id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES ask_sessions(session_id),
              purpose TEXT NOT NULL,status TEXT NOT NULL,stable_prompt_hash TEXT NOT NULL,
              schema_hash TEXT NOT NULL,input_fingerprint TEXT NOT NULL,input_json TEXT NOT NULL,
              model TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,error_code TEXT,
              created_at TEXT NOT NULL,updated_at TEXT NOT NULL,accepted_at TEXT,
              UNIQUE(session_id,input_fingerprint))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE TABLE ask_attempts(
              job_id TEXT NOT NULL REFERENCES ask_jobs(job_id),attempt INTEGER NOT NULL,
              request_fingerprint TEXT NOT NULL,output_fingerprint TEXT,status TEXT NOT NULL,
              usage_json TEXT NOT NULL,latency_ms INTEGER NOT NULL,error_code TEXT,
              created_at TEXT NOT NULL,PRIMARY KEY(job_id,attempt))",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "CREATE UNIQUE INDEX one_running_ask_job_per_session ON ask_jobs(session_id)
             WHERE status='running'",
        )
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT OR IGNORE INTO insights_schema_migrations(version,applied_at) VALUES(4,?1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    if current_version < 5 {
        sqlx::query(
            "CREATE TABLE ask_retrieval_reports(
              retrieval_id TEXT PRIMARY KEY,session_id TEXT NOT NULL REFERENCES ask_sessions(session_id),
              input_fingerprint TEXT NOT NULL,status TEXT NOT NULL,report_json TEXT NOT NULL,memo TEXT NOT NULL,
              provider TEXT,model TEXT,usage_json TEXT NOT NULL,latency_ms INTEGER NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,
              error_code TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,ready_at TEXT,
              UNIQUE(session_id,input_fingerprint))",
        ).execute(&mut *tx).await?;
        sqlx::query("CREATE INDEX ask_retrieval_reports_session ON ask_retrieval_reports(session_id,updated_at DESC)")
            .execute(&mut *tx).await?;
        sqlx::query(
            "INSERT OR IGNORE INTO insights_schema_migrations(version,applied_at) VALUES(5,?1)",
        )
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO insights_metadata(key,value) VALUES('schema_version',?1)
         ON CONFLICT(key) DO UPDATE SET value=?1",
    )
    .bind(SCHEMA_VERSION.to_string())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Serialize)]
struct ImmutableEvidence<'a> {
    source_namespace: &'a str,
    source_id: &'a str,
    occurred_at: &'a str,
    app: &'a Option<String>,
    window: &'a Option<String>,
    excerpt: &'a str,
}

pub async fn upsert_evidence(pool: &SqlitePool, item: &EvidenceRecord) -> Result<()> {
    let immutable = fingerprint(&ImmutableEvidence {
        source_namespace: &item.source_namespace,
        source_id: &item.source_id,
        occurred_at: &item.occurred_at,
        app: &item.app,
        window: &item.window,
        excerpt: &item.excerpt,
    })?;
    let mut tx = pool.begin().await?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT immutable_fingerprint FROM evidence WHERE evidence_id=?1")
            .bind(&item.evidence_id)
            .fetch_optional(&mut *tx)
            .await?;
    if let Some(stored) = stored {
        if stored != immutable {
            return Err(InsightsError::IdentityCollision(item.evidence_id.clone()));
        }
        sqlx::query(
            "UPDATE evidence SET policy_allowed=?2,redaction_ready=?3,deleted=?4,sensitive=?5
             WHERE evidence_id=?1",
        )
        .bind(&item.evidence_id)
        .bind(item.policy_allowed)
        .bind(item.redaction_ready)
        .bind(item.deleted)
        .bind(item.sensitive)
        .execute(&mut *tx)
        .await?;
    } else {
        sqlx::query("INSERT INTO evidence VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)")
            .bind(&item.evidence_id)
            .bind(&item.source_namespace)
            .bind(&item.source_id)
            .bind(&item.occurred_at)
            .bind(&item.app)
            .bind(&item.window)
            .bind(&item.excerpt)
            .bind(&immutable)
            .bind(item.policy_allowed)
            .bind(item.redaction_ready)
            .bind(item.deleted)
            .bind(item.sensitive)
            .execute(&mut *tx)
            .await?;
    }
    if !item.admissible() {
        sqlx::query(
            "UPDATE findings SET active=0 WHERE finding_id IN
             (SELECT finding_id FROM finding_evidence WHERE evidence_id=?1)",
        )
        .bind(&item.evidence_id)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn mark_source_deleted(
    pool: &SqlitePool,
    source_namespace: &str,
    source_id: &str,
) -> Result<bool> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "UPDATE evidence SET deleted=1 WHERE source_namespace=?1 AND source_id=?2 AND deleted=0",
    )
    .bind(source_namespace)
    .bind(source_id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() > 0 {
        sqlx::query(
            "UPDATE findings SET active=0 WHERE finding_id IN(
             SELECT fe.finding_id FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
             WHERE e.source_namespace=?1 AND e.source_id=?2)",
        ).bind(source_namespace).bind(source_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    if result.rows_affected() > 0 {
        recompute_surface_status(pool).await?;
    }
    Ok(result.rows_affected() > 0)
}

/// Removes the locally retained content for capture evidence while preserving
/// only its opaque identity so references cannot be accidentally re-admitted.
/// Findings that relied on any forgotten source are withdrawn immediately.
pub async fn forget_capture_evidence(
    pool: &SqlitePool,
    source_namespace: &str,
    source_ids: &[String],
) -> Result<(u64, u64)> {
    if source_ids.is_empty() {
        return Ok((0, 0));
    }

    let mut forgotten_evidence = 0u64;
    let mut affected_findings = std::collections::HashSet::new();
    for chunk in source_ids.chunks(400) {
        let mut tx = pool.begin().await?;
        let mut finding_query = sqlx::QueryBuilder::new(
            "SELECT DISTINCT fe.finding_id FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id WHERE e.source_namespace=",
        );
        finding_query
            .push_bind(source_namespace)
            .push(" AND e.source_id IN (");
        let mut separated = finding_query.separated(",");
        for source_id in chunk {
            separated.push_bind(source_id);
        }
        separated.push_unseparated(")");
        for row in finding_query.build().fetch_all(&mut *tx).await? {
            affected_findings.insert(row.get::<String, _>("finding_id"));
        }

        let mut update = sqlx::QueryBuilder::new(
            "UPDATE evidence SET excerpt='',app=NULL,window=NULL,deleted=1 WHERE source_namespace=",
        );
        update
            .push_bind(source_namespace)
            .push(" AND source_id IN (");
        let mut separated = update.separated(",");
        for source_id in chunk {
            separated.push_bind(source_id);
        }
        separated.push_unseparated(")");
        forgotten_evidence += update.build().execute(&mut *tx).await?.rows_affected();
        tx.commit().await?;
    }

    if !affected_findings.is_empty() {
        let mut tx = pool.begin().await?;
        for chunk in affected_findings.iter().collect::<Vec<_>>().chunks(400) {
            let mut update =
                sqlx::QueryBuilder::new("UPDATE findings SET active=0 WHERE finding_id IN (");
            let mut separated = update.separated(",");
            for finding_id in chunk {
                separated.push_bind(*finding_id);
            }
            separated.push_unseparated(")");
            update.build().execute(&mut *tx).await?;
        }
        tx.commit().await?;
        recompute_surface_status(pool).await?;
    }

    Ok((forgotten_evidence, affected_findings.len() as u64))
}

pub async fn admit_observation(pool: &SqlitePool, item: &ObservationRecord) -> Result<i64> {
    if item.evidence_ids.is_empty() {
        return Err(InsightsError::Invalid("observation has no evidence".into()));
    }
    let mut tx = pool.begin().await?;
    for evidence_id in &item.evidence_ids {
        let row = sqlx::query(
            "SELECT policy_allowed,redaction_ready,deleted,sensitive FROM evidence WHERE evidence_id=?1",
        )
        .bind(evidence_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| InsightsError::Invalid(format!("unknown evidence {evidence_id}")))?;
        if !row.get::<bool, _>("policy_allowed")
            || !row.get::<bool, _>("redaction_ready")
            || row.get::<bool, _>("deleted")
            || row.get::<bool, _>("sensitive")
        {
            return Err(InsightsError::Invalid(format!(
                "inadmissible evidence {evidence_id}"
            )));
        }
    }
    let immutable = fingerprint(item)?;
    let stored: Option<(i64, String)> = sqlx::query_as(
        "SELECT sequence,immutable_fingerprint FROM observations WHERE observation_id=?1",
    )
    .bind(&item.observation_id)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some((sequence, stored)) = stored {
        if stored != immutable {
            return Err(InsightsError::IdentityCollision(
                item.observation_id.clone(),
            ));
        }
        tx.commit().await?;
        return Ok(sequence);
    }
    let result = sqlx::query(
        "INSERT INTO observations(observation_id,source_key,occurred_at,statement,certainty,evidence_ids_json,immutable_fingerprint,admitted_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
    )
    .bind(&item.observation_id)
    .bind(&item.source_key)
    .bind(&item.occurred_at)
    .bind(&item.statement)
    .bind(certainty_str(item.certainty))
    .bind(serde_json::to_string(&item.evidence_ids)?)
    .bind(immutable)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    let sequence = result.last_insert_rowid();
    tx.commit().await?;
    Ok(sequence)
}

/// Copy the accepted Explorer evidence and observations into a fresh insights
/// database for a Steward-only replay. Jobs, cursors, opportunities, and
/// findings are deliberately not copied.
pub async fn copy_observations_for_steward_replay(
    source: &SqlitePool,
    destination: &SqlitePool,
    source_identity: &str,
) -> Result<u64> {
    let recorded_identity: Option<String> = sqlx::query_scalar(
        "SELECT value FROM insights_metadata WHERE key='steward_replay_source_identity'",
    )
    .fetch_optional(destination)
    .await?;
    if let Some(recorded_identity) = recorded_identity {
        if recorded_identity != source_identity {
            return Err(InsightsError::Invalid(
                "Steward replay destination belongs to a different source".into(),
            ));
        }
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM observations")
            .fetch_one(destination)
            .await?;
        return Ok(count as u64);
    }

    for table in [
        "inference_jobs",
        "opportunities",
        "occurrences",
        "findings",
        "reconciliations",
    ] {
        let present: i64 = sqlx::query_scalar(&format!("SELECT EXISTS(SELECT 1 FROM {table})"))
            .fetch_one(destination)
            .await?;
        if present != 0 {
            return Err(InsightsError::Invalid(format!(
                "Steward replay destination already contains {table}"
            )));
        }
    }
    let existing_rows: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM evidence) + (SELECT COUNT(*) FROM observations)",
    )
    .fetch_one(destination)
    .await?;
    if existing_rows != 0 {
        return Err(InsightsError::Invalid(
            "Steward replay destination already contains source data without a replay identity"
                .into(),
        ));
    }

    let observations = sqlx::query(
        "SELECT observation_id,source_key,occurred_at,statement,certainty,evidence_ids_json
         FROM observations ORDER BY sequence",
    )
    .fetch_all(source)
    .await?;
    let mut copied = 0;
    for row in observations {
        let evidence_ids = serde_json::from_str::<Vec<String>>(row.get("evidence_ids_json"))?;
        for evidence_id in &evidence_ids {
            let evidence = sqlx::query(
                "SELECT evidence_id,source_namespace,source_id,occurred_at,app,window,excerpt,
                        policy_allowed,redaction_ready,deleted,sensitive
                 FROM evidence WHERE evidence_id=?1",
            )
            .bind(evidence_id)
            .fetch_one(source)
            .await?;
            upsert_evidence(
                destination,
                &EvidenceRecord {
                    evidence_id: evidence.get("evidence_id"),
                    source_namespace: evidence.get("source_namespace"),
                    source_id: evidence.get("source_id"),
                    occurred_at: evidence.get("occurred_at"),
                    app: evidence.get("app"),
                    window: evidence.get("window"),
                    excerpt: evidence.get("excerpt"),
                    policy_allowed: evidence.get("policy_allowed"),
                    redaction_ready: evidence.get("redaction_ready"),
                    deleted: evidence.get("deleted"),
                    sensitive: evidence.get("sensitive"),
                },
            )
            .await?;
        }
        admit_observation(
            destination,
            &ObservationRecord {
                observation_id: row.get("observation_id"),
                source_key: row.get("source_key"),
                occurred_at: row.get("occurred_at"),
                statement: row.get("statement"),
                certainty: parse_certainty(row.get::<String, _>("certainty").as_str())?,
                evidence_ids,
            },
        )
        .await?;
        copied += 1;
    }
    sqlx::query(
        "INSERT INTO insights_metadata(key,value) VALUES('steward_replay_source_identity',?1)",
    )
    .bind(source_identity)
    .execute(destination)
    .await?;
    Ok(copied)
}

#[derive(Debug, Clone)]
pub struct NewExplorerJob<'a> {
    pub batch_id: &'a str,
    pub input_fingerprint: &'a str,
    pub input_json: &'a str,
    pub prompt_hash: &'a str,
    pub schema_hash: &'a str,
    pub model: &'a str,
}

pub async fn create_explorer_job(pool: &SqlitePool, input: NewExplorerJob<'_>) -> Result<String> {
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT job_id FROM explorer_jobs WHERE input_fingerprint=?1",
    )
    .bind(input.input_fingerprint)
    .fetch_optional(pool)
    .await?
    {
        return Ok(existing);
    }
    let job_id = stable_id("wfx", &input.input_fingerprint)?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO explorer_jobs VALUES(?1,?2,?3,'pending',?4,?5,?6,?7,0,NULL,?8,?8,NULL)",
    )
    .bind(&job_id)
    .bind(input.batch_id)
    .bind(input.input_fingerprint)
    .bind(input.input_json)
    .bind(input.prompt_hash)
    .bind(input.schema_hash)
    .bind(input.model)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(job_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredExplorerJob {
    pub job_id: String,
    pub batch_id: String,
    pub input_json: String,
    pub status: String,
}

pub async fn recoverable_explorer_job(
    pool: &SqlitePool,
    batch_id: &str,
) -> Result<Option<StoredExplorerJob>> {
    Ok(sqlx::query("SELECT job_id,batch_id,input_json,status FROM explorer_jobs WHERE batch_id=?1 AND status!='rejected'")
        .bind(batch_id).fetch_optional(pool).await?.map(|row| StoredExplorerJob {
            job_id: row.get("job_id"), batch_id: row.get("batch_id"),
            input_json: row.get("input_json"), status: row.get("status"),
        }))
}

pub async fn pending_explorer_batch_id(pool: &SqlitePool) -> Result<Option<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT batch_id FROM explorer_jobs WHERE status IN ('pending','running')
         ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(pool)
    .await?)
}

pub async fn claim_explorer_job(pool: &SqlitePool, job_id: &str) -> Result<bool> {
    Ok(sqlx::query(
        "UPDATE explorer_jobs SET status='running',attempts=attempts+1,updated_at=?2
         WHERE job_id=?1 AND status IN ('pending','running')",
    )
    .bind(job_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?
    .rows_affected()
        == 1)
}

pub async fn record_explorer_attempt(
    pool: &SqlitePool,
    job_id: &str,
    request_fingerprint: &str,
    output_fingerprint: Option<&str>,
    status: &str,
    usage: &impl Serialize,
    latency_ms: u64,
    error_code: Option<&str>,
) -> Result<()> {
    let attempt = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(attempt),0)+1 FROM explorer_attempts WHERE job_id=?1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO explorer_attempts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
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
        .await?;
    Ok(())
}

pub async fn mark_explorer_job(
    pool: &SqlitePool,
    job_id: &str,
    status: &str,
    error: &str,
) -> Result<()> {
    if !matches!(status, "pending" | "rejected") {
        return Err(InsightsError::Invalid(
            "invalid Explorer terminal transition".into(),
        ));
    }
    sqlx::query(
        "UPDATE explorer_jobs SET status=?2,error_code=?3,updated_at=?4
         WHERE job_id=?1 AND status!='accepted'",
    )
    .bind(job_id)
    .bind(status)
    .bind(error)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct AcceptedAttemptReceipt {
    pub request_fingerprint: String,
    pub output_fingerprint: String,
    pub usage: serde_json::Value,
    pub latency_ms: u64,
}

pub async fn apply_explorer_output(
    pool: &SqlitePool,
    job_id: &str,
    output: &crate::ExplorerOutput,
) -> Result<Vec<String>> {
    apply_explorer_output_inner(pool, job_id, output, None).await
}

pub async fn apply_explorer_output_with_attempt(
    pool: &SqlitePool,
    job_id: &str,
    output: &crate::ExplorerOutput,
    receipt: AcceptedAttemptReceipt,
) -> Result<Vec<String>> {
    apply_explorer_output_inner(pool, job_id, output, Some(receipt)).await
}

async fn apply_explorer_output_inner(
    pool: &SqlitePool,
    job_id: &str,
    output: &crate::ExplorerOutput,
    receipt: Option<AcceptedAttemptReceipt>,
) -> Result<Vec<String>> {
    if output.schema_version != 1 {
        return Err(InsightsError::Invalid(
            "wrong Explorer schema version".into(),
        ));
    }
    let row = sqlx::query("SELECT batch_id,status FROM explorer_jobs WHERE job_id=?1")
        .bind(job_id)
        .fetch_one(pool)
        .await?;
    let batch_id: String = row.get("batch_id");
    if row.get::<String, _>("status") == "accepted" {
        return Ok(sqlx::query_scalar::<_, String>(
            "SELECT observation_id FROM observations WHERE source_key LIKE ?1 ORDER BY sequence",
        )
        .bind(format!("{batch_id}:%"))
        .fetch_all(pool)
        .await?);
    }
    let mut seen_local = HashSet::new();
    let mut tx = pool.begin().await?;
    let mut accepted = Vec::new();
    let accepted_at = Utc::now().to_rfc3339();
    for draft in &output.observations {
        if draft.statement.trim().is_empty()
            || draft.evidence_ids.is_empty()
            || has_duplicates(&draft.evidence_ids)
            || !seen_local.insert(draft.local_id.clone())
        {
            return Err(InsightsError::Invalid(
                "malformed or duplicate Explorer observation".into(),
            ));
        }
        for evidence_id in &draft.evidence_ids {
            let admissible = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM evidence WHERE evidence_id=?1 AND policy_allowed=1
                 AND redaction_ready=1 AND deleted=0 AND sensitive=0",
            )
            .bind(evidence_id)
            .fetch_one(&mut *tx)
            .await?;
            if admissible != 1 {
                return Err(InsightsError::Invalid(format!(
                    "inadmissible Explorer evidence {evidence_id}"
                )));
            }
        }
        let source_key = format!("{batch_id}:{}", draft.local_id);
        let observation_id =
            stable_id("obl", &(&source_key, &draft.statement, &draft.occurred_at))?;
        let immutable = fingerprint(&(
            &source_key,
            &draft.occurred_at,
            &draft.statement,
            draft.certainty,
            &draft.evidence_ids,
        ))?;
        let existing = sqlx::query_scalar::<_, String>(
            "SELECT immutable_fingerprint FROM observations WHERE observation_id=?1 OR source_key=?2",
        ).bind(&observation_id).bind(&source_key).fetch_optional(&mut *tx).await?;
        if let Some(existing) = existing {
            if existing != immutable {
                return Err(InsightsError::IdentityCollision(observation_id));
            }
        } else {
            sqlx::query(
                "INSERT INTO observations(observation_id,source_key,occurred_at,statement,certainty,evidence_ids_json,immutable_fingerprint,admitted_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            ).bind(&observation_id).bind(&source_key).bind(&draft.occurred_at)
                .bind(&draft.statement).bind(certainty_str(draft.certainty))
                .bind(serde_json::to_string(&draft.evidence_ids)?).bind(immutable)
                .bind(&accepted_at)
                .execute(&mut *tx).await?;
        }
        accepted.push(observation_id);
    }
    if let Some(receipt) = receipt {
        let attempt = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM explorer_attempts WHERE job_id=?1",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO explorer_attempts VALUES(?1,?2,?3,?4,'accepted',?5,?6,NULL,?7)")
            .bind(job_id)
            .bind(attempt)
            .bind(receipt.request_fingerprint)
            .bind(receipt.output_fingerprint)
            .bind(serde_json::to_string(&receipt.usage)?)
            .bind(receipt.latency_ms as i64)
            .bind(Utc::now().to_rfc3339())
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query(
        "UPDATE explorer_jobs SET status='accepted',accepted_at=?2,updated_at=?2 WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(&accepted_at)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(accepted)
}

#[derive(Debug, Clone)]
pub struct NewJob<'a> {
    pub input_fingerprint: &'a str,
    pub local_day: &'a str,
    pub reason: &'a str,
    pub observation_ids: &'a [String],
    pub prompt_hash: &'a str,
    pub schema_hash: &'a str,
    pub model: &'a str,
    /// Frozen normalized packet used to reconstruct the exact provider request.
    pub input_json: &'a str,
}

pub async fn create_job(pool: &SqlitePool, input: NewJob<'_>) -> Result<String> {
    if input.observation_ids.is_empty() {
        return Err(InsightsError::Invalid("job owns no observations".into()));
    }
    if let Some(existing) = sqlx::query_scalar::<_, String>(
        "SELECT job_id FROM inference_jobs WHERE input_fingerprint=?1",
    )
    .bind(input.input_fingerprint)
    .fetch_optional(pool)
    .await?
    {
        return Ok(existing);
    }
    let job_id = stable_id("wfj", &input.input_fingerprint)?;
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO inference_jobs(job_id,input_fingerprint,local_day,reason,status,prompt_hash,schema_hash,model,input_json,created_at,updated_at)
         VALUES(?1,?2,?3,?4,'pending',?5,?6,?7,?8,?9,?9)",
    )
    .bind(&job_id)
    .bind(input.input_fingerprint)
    .bind(input.local_day)
    .bind(input.reason)
    .bind(input.prompt_hash)
    .bind(input.schema_hash)
    .bind(input.model)
    .bind(input.input_json)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    for (index, observation_id) in input.observation_ids.iter().enumerate() {
        sqlx::query("INSERT INTO job_observations VALUES(?1,?2,?3)")
            .bind(&job_id)
            .bind(observation_id)
            .bind(index as i64)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(job_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredJob {
    pub job_id: String,
    pub input_json: String,
    pub model: String,
    pub observation_ids: Vec<String>,
}

pub async fn recoverable_job(pool: &SqlitePool) -> Result<Option<StoredJob>> {
    let row = sqlx::query(
        "SELECT job_id,input_json,model FROM inference_jobs
         WHERE status IN ('pending','running') ORDER BY created_at LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let job_id: String = row.get("job_id");
    Ok(Some(StoredJob {
        observation_ids: job_observation_ids(pool, &job_id).await?,
        job_id,
        input_json: row.get("input_json"),
        model: row.get("model"),
    }))
}

/// Bounded semantic memory for a Steward wake. Durable counts stay in SQLite;
/// the provider sees summaries plus only the most recent occurrence outlines.
pub async fn steward_memory(
    pool: &SqlitePool,
    opportunity_limit: u32,
    recent_occurrence_limit: u32,
) -> Result<serde_json::Value> {
    let opportunities = sqlx::query(
        "SELECT opportunity_id,construct,current_status,current_version,current_summary,cadence
         FROM opportunities WHERE current_status NOT IN ('retired')
         ORDER BY updated_at DESC,opportunity_id LIMIT ?1",
    )
    .bind(opportunity_limit.clamp(1, 10))
    .fetch_all(pool)
    .await?;
    let mut result = Vec::new();
    for opportunity in opportunities {
        let opportunity_id: String = opportunity.get("opportunity_id");
        let occurrence_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM occurrences WHERE opportunity_id=?1")
                .bind(&opportunity_id)
                .fetch_one(pool)
                .await?;
        let recent = sqlx::query(
            "SELECT started_at,ended_at,proposal_json FROM occurrences
             WHERE opportunity_id=?1 ORDER BY started_at DESC,occurrence_id DESC LIMIT ?2",
        )
        .bind(&opportunity_id)
        .bind(recent_occurrence_limit.clamp(1, 5))
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(|row| {
            serde_json::json!({
                "started_at": row.get::<String, _>("started_at"),
                "ended_at": row.get::<String, _>("ended_at"),
                "outline": serde_json::from_str::<serde_json::Value>(row.get("proposal_json"))
                    .unwrap_or(serde_json::Value::Null),
            })
        })
        .collect::<Vec<_>>();
        result.push(serde_json::json!({
            "opportunity_id": opportunity_id,
            "construct": opportunity.get::<String, _>("construct"),
            "status": opportunity.get::<String, _>("current_status"),
            "version": opportunity.get::<i64, _>("current_version"),
            "summary": opportunity.get::<String, _>("current_summary"),
            "cadence": opportunity.get::<String, _>("cadence"),
            "occurrence_count": occurrence_count,
            "recent_occurrences": recent,
        }));
    }
    let feedback = sqlx::query(
        "SELECT d.kind,d.correction_text,d.intent,f.claim FROM dispositions d
         JOIN findings f ON f.finding_id=d.finding_id
         ORDER BY d.created_at DESC,d.disposition_id DESC LIMIT 10",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        serde_json::json!({
            "kind": row.get::<String, _>("kind"),
            "claim": row.get::<String, _>("claim"),
            "correction": row.get::<Option<String>, _>("correction_text"),
            "intent": row.get::<Option<String>, _>("intent"),
        })
    })
    .collect::<Vec<_>>();
    Ok(serde_json::json!({"opportunities": result, "recent_user_feedback": feedback}))
}

/// Developer-facing, privacy-bounded diagnostics for accepted Steward output.
/// It reads the durable reconciliation JSON and attempt receipts; it does not
/// include captured evidence text or expose a product projection.
#[cfg(test)]
pub(crate) async fn steward_diagnostics(pool: &SqlitePool) -> Result<serde_json::Value> {
    let reconciliations = sqlx::query(
        "SELECT reconciliation_id,job_id,output_json,accepted_at
         FROM reconciliations ORDER BY accepted_at,reconciliation_id",
    )
    .fetch_all(pool)
    .await?;
    let mut reconciliation_values = Vec::with_capacity(reconciliations.len());
    for row in reconciliations {
        let output: ReconciliationOutput = serde_json::from_str(row.get("output_json"))?;
        let mut assessments = Vec::with_capacity(output.candidate_assessments.len());
        for assessment in output.candidate_assessments {
            let mut value = serde_json::to_value(&assessment)?;
            if let Some(opportunity_local_id) = assessment.opportunity_local_id.as_deref() {
                if let Some(opportunity) = output
                    .opportunities
                    .iter()
                    .find(|opportunity| opportunity.local_id == opportunity_local_id)
                {
                    let opportunity_id =
                        opportunity
                            .existing_opportunity_id
                            .clone()
                            .unwrap_or(stable_id(
                                "wfo",
                                &(opportunity.construct.as_str(), &opportunity.signature),
                            )?);
                    let finding_id: Option<String> = sqlx::query_scalar(
                        "SELECT finding_id FROM findings WHERE opportunity_id=?1
                         ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(&opportunity_id)
                    .fetch_optional(pool)
                    .await?;
                    let occurrence_count: i64 = sqlx::query_scalar(
                        "SELECT COUNT(*) FROM occurrences WHERE opportunity_id=?1",
                    )
                    .bind(&opportunity_id)
                    .fetch_one(pool)
                    .await?;
                    if let Some(object) = value.as_object_mut() {
                        object.insert(
                            "durable_opportunity_id".into(),
                            serde_json::json!(opportunity_id),
                        );
                        object.insert("finding_id".into(), serde_json::json!(finding_id));
                        object.insert(
                            "occurrence_count".into(),
                            serde_json::json!(occurrence_count),
                        );
                        object.insert(
                            "distinctness_basis".into(),
                            serde_json::json!(opportunity
                                .occurrences_to_add
                                .iter()
                                .flat_map(|occurrence| occurrence.distinctness_basis.clone())
                                .collect::<Vec<_>>()),
                        );
                    }
                }
            }
            assessments.push(value);
        }
        reconciliation_values.push(serde_json::json!({
            "reconciliation_id": row.get::<String, _>("reconciliation_id"),
            "job_id": row.get::<String, _>("job_id"),
            "accepted_at": row.get::<String, _>("accepted_at"),
            "candidate_assessments": assessments,
        }));
    }

    let attempts = sqlx::query(
        "SELECT ja.job_id,ij.model,ja.attempt,ja.status,ja.usage_json,ja.latency_ms,
                ja.error_code,ja.created_at
         FROM job_attempts ja JOIN inference_jobs ij ON ij.job_id=ja.job_id
         ORDER BY ja.created_at,ja.job_id,ja.attempt",
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        serde_json::json!({
            "job_id": row.get::<String, _>("job_id"),
            "model": row.get::<Option<String>, _>("model"),
            "attempt": row.get::<i64, _>("attempt"),
            "status": row.get::<String, _>("status"),
            "usage": serde_json::from_str::<serde_json::Value>(row.get("usage_json"))
                .unwrap_or(serde_json::Value::Null),
            "latency_ms": row.get::<i64, _>("latency_ms"),
            "error_code": row.get::<Option<String>, _>("error_code"),
            "created_at": row.get::<String, _>("created_at"),
        })
    })
    .collect::<Vec<_>>();
    Ok(serde_json::json!({
        "reconciliations": reconciliation_values,
        "steward_attempts": attempts,
    }))
}

pub async fn claim_job(pool: &SqlitePool, job_id: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE inference_jobs SET status='running',attempts=attempts+1,updated_at=?2
         WHERE job_id=?1 AND status IN ('pending','running')",
    )
    .bind(job_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn accepted_job(pool: &SqlitePool, job_id: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM inference_jobs WHERE job_id=?1 AND status='accepted'",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?
        == 1)
}

pub async fn job_observation_ids(pool: &SqlitePool, job_id: &str) -> Result<Vec<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT observation_id FROM job_observations WHERE job_id=?1 ORDER BY ordinal",
    )
    .bind(job_id)
    .fetch_all(pool)
    .await?)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ApplyOptions {
    /// Test-only crash injection. The surrounding transaction must roll back.
    pub fail_after_opportunity: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyResult {
    pub reconciliation_id: String,
    pub already_accepted: bool,
    pub opportunities_changed: usize,
    pub occurrences_added: usize,
    pub findings_created: usize,
}

fn status_str(value: OpportunityStatus) -> &'static str {
    match value {
        OpportunityStatus::Watching => "watching",
        OpportunityStatus::Eligible => "eligible",
        OpportunityStatus::Surfaced => "surfaced",
        OpportunityStatus::Withdrawn => "withdrawn",
        OpportunityStatus::Retired => "retired",
    }
}

pub(crate) fn handoff_str(value: HandoffType) -> &'static str {
    match value {
        HandoffType::Prompt => "prompt",
        HandoffType::SavedPrompt => "saved_prompt",
        HandoffType::ExistingCapability => "existing_capability",
        HandoffType::Runbook => "runbook",
    }
}

pub(crate) fn parse_handoff(value: &str) -> Result<HandoffType> {
    match value {
        "prompt" => Ok(HandoffType::Prompt),
        "saved_prompt" => Ok(HandoffType::SavedPrompt),
        "existing_capability" => Ok(HandoffType::ExistingCapability),
        "runbook" => Ok(HandoffType::Runbook),
        _ => Err(InsightsError::Invalid(format!("unknown handoff {value}"))),
    }
}

async fn opportunity_occurrence_sets(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    opportunity_id: &str,
) -> Result<(HashSet<String>, HashSet<String>, usize)> {
    let rows = sqlx::query(
        "SELECT observation_ids_json,evidence_ids_json FROM occurrences
         WHERE opportunity_id=?1 ORDER BY started_at,occurrence_id",
    )
    .bind(opportunity_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut observations = HashSet::new();
    let mut evidence = HashSet::new();
    for row in &rows {
        observations.extend(serde_json::from_str::<Vec<String>>(
            row.get("observation_ids_json"),
        )?);
        evidence.extend(serde_json::from_str::<Vec<String>>(
            row.get("evidence_ids_json"),
        )?);
    }
    Ok((observations, evidence, rows.len()))
}

async fn opportunity_certainties_and_starts(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    opportunity_id: &str,
) -> Result<(Vec<ObservationCertainty>, Vec<chrono::DateTime<Utc>>)> {
    let rows = sqlx::query(
        "SELECT observation_ids_json,started_at FROM occurrences
         WHERE opportunity_id=?1 ORDER BY started_at,occurrence_id",
    )
    .bind(opportunity_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut certainties = Vec::new();
    let mut starts = Vec::new();
    for row in rows {
        if let Ok(start) = row.get::<String, _>("started_at").parse() {
            starts.push(start);
        }
        for observation_id in serde_json::from_str::<Vec<String>>(row.get("observation_ids_json"))?
        {
            let certainty: String =
                sqlx::query_scalar("SELECT certainty FROM observations WHERE observation_id=?1")
                    .bind(observation_id)
                    .fetch_one(&mut **tx)
                    .await?;
            certainties.push(parse_certainty(&certainty)?);
        }
    }
    Ok((certainties, starts))
}

pub async fn apply_reconciliation(
    pool: &SqlitePool,
    job_id: &str,
    output: &ReconciliationOutput,
    options: ApplyOptions,
) -> Result<ApplyResult> {
    apply_reconciliation_inner(pool, job_id, output, options, None).await
}

pub async fn apply_reconciliation_with_attempt(
    pool: &SqlitePool,
    job_id: &str,
    output: &ReconciliationOutput,
    options: ApplyOptions,
    receipt: AcceptedAttemptReceipt,
) -> Result<ApplyResult> {
    apply_reconciliation_inner(pool, job_id, output, options, Some(receipt)).await
}

async fn apply_reconciliation_inner(
    pool: &SqlitePool,
    job_id: &str,
    output: &ReconciliationOutput,
    options: ApplyOptions,
    receipt: Option<AcceptedAttemptReceipt>,
) -> Result<ApplyResult> {
    if accepted_job(pool, job_id).await? {
        let reconciliation_id: String =
            sqlx::query_scalar("SELECT reconciliation_id FROM reconciliations WHERE job_id=?1")
                .bind(job_id)
                .fetch_one(pool)
                .await?;
        return Ok(ApplyResult {
            reconciliation_id,
            already_accepted: true,
            opportunities_changed: 0,
            occurrences_added: 0,
            findings_created: 0,
        });
    }
    if !matches!(output.schema_version, 1 | 2 | 3) {
        return Err(InsightsError::Invalid(
            "wrong reconciliation schema version".into(),
        ));
    }
    let expected = job_observation_ids(pool, job_id).await?;
    if has_duplicates(&output.considered_observation_ids) {
        return Err(InsightsError::Invalid(
            "reconciliation repeats a considered observation".into(),
        ));
    }
    let expected_set: HashSet<_> = expected.iter().collect();
    let considered_set: HashSet<_> = output.considered_observation_ids.iter().collect();
    if expected.len() != output.considered_observation_ids.len() || expected_set != considered_set {
        return Err(InsightsError::Invalid(
            "reconciliation does not own the exact job observations".into(),
        ));
    }
    if matches!(output.schema_version, 2 | 3) {
        validate_candidate_assessments(output, &expected).map_err(|error| {
            InsightsError::Invalid(format!("invalid candidate assessments: {error}"))
        })?;
    }
    // Candidate assessments are a transient inference guardrail. Validate them
    // before applying, then omit them from durable/product-facing reconciliation
    // output. Existing stored versions remain readable through serde defaults.
    let mut durable_output = output.clone();
    if matches!(durable_output.schema_version, 2 | 3) {
        durable_output.schema_version = 1;
        durable_output.candidate_assessments.clear();
    }
    let output = &durable_output;
    let output_json = serde_json::to_string(output)?;
    let output_fingerprint = fingerprint(output)?;
    let reconciliation_id = stable_id("wfr", &(job_id, &output_fingerprint))?;
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    let status: String = sqlx::query_scalar("SELECT status FROM inference_jobs WHERE job_id=?1")
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
    if status == "accepted" {
        tx.rollback().await?;
        return Ok(ApplyResult {
            reconciliation_id,
            already_accepted: true,
            opportunities_changed: 0,
            occurrences_added: 0,
            findings_created: 0,
        });
    }
    if status != "running" && status != "pending" {
        return Err(InsightsError::Invalid(format!("job is {status}")));
    }
    let mut occurrence_total = 0;
    let mut finding_total = 0;
    for (index, proposal) in output.opportunities.iter().enumerate() {
        if proposal.retire && proposal.finding.is_some() {
            return Err(InsightsError::Invalid(
                "retired opportunity cannot create a finding".into(),
            ));
        }
        let opportunity_id = if let Some(existing) = &proposal.existing_opportunity_id {
            let stored: Option<String> =
                sqlx::query_scalar("SELECT construct FROM opportunities WHERE opportunity_id=?1")
                    .bind(existing)
                    .fetch_optional(&mut *tx)
                    .await?;
            let stored = stored
                .ok_or_else(|| InsightsError::Invalid(format!("unknown opportunity {existing}")))?;
            if parse_construct(&stored)? != proposal.construct {
                return Err(InsightsError::Invalid(
                    "existing opportunity construct changed".into(),
                ));
            }
            existing.clone()
        } else {
            stable_id("wfo", &(proposal.construct.as_str(), &proposal.signature))?
        };
        let existing: Option<(i64,)> =
            sqlx::query_as("SELECT current_version FROM opportunities WHERE opportunity_id=?1")
                .bind(&opportunity_id)
                .fetch_optional(&mut *tx)
                .await?;
        if proposal.existing_opportunity_id.is_none() && existing.is_some() {
            return Err(InsightsError::Invalid(
                "existing opportunity must use its durable identity".into(),
            ));
        }
        if existing.is_none() && proposal.occurrences_to_add.is_empty() {
            return Err(InsightsError::Invalid(
                "new opportunity has no occurrence".into(),
            ));
        }
        if existing.is_none() {
            sqlx::query("INSERT INTO opportunities VALUES(?1,?2,?3,'watching',0,?4,?5,?6)")
                .bind(&opportunity_id)
                .bind(proposal.construct.as_str())
                .bind(&proposal.signature)
                .bind(&proposal.summary)
                .bind(cadence_str(proposal.cadence))
                .bind(&now)
                .execute(&mut *tx)
                .await?;
        }
        let (mut prior_observations, mut prior_evidence, _) =
            opportunity_occurrence_sets(&mut tx, &opportunity_id).await?;
        for occurrence in &proposal.occurrences_to_add {
            if occurrence.observation_ids.is_empty() || occurrence.evidence_ids.is_empty() {
                return Err(InsightsError::Invalid(
                    "occurrence has empty evidence".into(),
                ));
            }
            if has_duplicates(&occurrence.observation_ids)
                || has_duplicates(&occurrence.evidence_ids)
            {
                return Err(InsightsError::Invalid(
                    "occurrence repeats an observation or evidence reference".into(),
                ));
            }
            if occurrence
                .observation_ids
                .iter()
                .any(|id| !expected_set.contains(id))
            {
                return Err(InsightsError::Invalid(
                    "occurrence delta uses an observation outside its job".into(),
                ));
            }
            if occurrence
                .observation_ids
                .iter()
                .any(|id| prior_observations.contains(id))
                || occurrence
                    .evidence_ids
                    .iter()
                    .any(|id| prior_evidence.contains(id))
            {
                return Err(InsightsError::Invalid(
                    "occurrence overlaps durable opportunity history".into(),
                ));
            }
            let mut allowed_evidence = HashSet::new();
            let mut times = Vec::new();
            for observation_id in &occurrence.observation_ids {
                let row = sqlx::query(
                    "SELECT occurred_at,evidence_ids_json FROM observations WHERE observation_id=?1",
                )
                .bind(observation_id)
                .fetch_one(&mut *tx)
                .await?;
                times.push(row.get::<String, _>("occurred_at"));
                allowed_evidence.extend(serde_json::from_str::<Vec<String>>(
                    row.get("evidence_ids_json"),
                )?);
            }
            if occurrence
                .evidence_ids
                .iter()
                .any(|id| !allowed_evidence.contains(id))
            {
                return Err(InsightsError::Invalid(
                    "occurrence evidence is not owned by its observations".into(),
                ));
            }
            times.sort();
            let canonical_observations = {
                let mut values = occurrence.observation_ids.clone();
                values.sort();
                values
            };
            let canonical_evidence = {
                let mut values = occurrence.evidence_ids.clone();
                values.sort();
                values
            };
            let occurrence_id = stable_id(
                "woc",
                &(
                    &opportunity_id,
                    &canonical_observations,
                    &canonical_evidence,
                ),
            )?;
            sqlx::query("INSERT INTO occurrences VALUES(?1,?2,?3,?4,?5,?6,?7)")
                .bind(&occurrence_id)
                .bind(&opportunity_id)
                .bind(serde_json::to_string(&canonical_observations)?)
                .bind(serde_json::to_string(&canonical_evidence)?)
                .bind(serde_json::to_string(occurrence)?)
                .bind(times.first().cloned().unwrap_or_default())
                .bind(times.last().cloned().unwrap_or_default())
                .execute(&mut *tx)
                .await?;
            prior_observations.extend(canonical_observations);
            prior_evidence.extend(canonical_evidence);
            occurrence_total += 1;
        }
        let (_, _, occurrence_count) =
            opportunity_occurrence_sets(&mut tx, &opportunity_id).await?;
        let (certainties, starts) =
            opportunity_certainties_and_starts(&mut tx, &opportunity_id).await?;
        let capability_verified = if let Some(handoff) = &proposal.handoff {
            if handoff.kind == HandoffType::ExistingCapability {
                if let Some(capability_id) = &handoff.capability_id {
                    sqlx::query_scalar::<_, i64>(
                        "SELECT COUNT(*) FROM capabilities WHERE capability_id=?1
                         AND action_kind IS NOT NULL AND action_target IS NOT NULL",
                    )
                    .bind(capability_id)
                    .fetch_one(&mut *tx)
                    .await?
                        == 1
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };
        let cadence_ok = cadence_supported(proposal.cadence, &starts);
        let eligibility = derive_eligibility(
            proposal,
            &EligibilityContext {
                occurrence_count,
                cadence_supported: cadence_ok,
                capability_verified,
            },
        );
        if proposal.finding.is_some() && !eligibility.eligible {
            return Err(InsightsError::Invalid(format!(
                "below-threshold finding: {}",
                eligibility.errors.join(", ")
            )));
        }
        let status = if proposal.retire {
            OpportunityStatus::Retired
        } else if proposal.withdraw_current_finding || proposal.finding.is_none() {
            if eligibility.eligible {
                OpportunityStatus::Withdrawn
            } else {
                OpportunityStatus::Watching
            }
        } else if eligibility.eligible {
            OpportunityStatus::Eligible
        } else {
            OpportunityStatus::Watching
        };
        let ordinal = existing.map(|row| row.0 + 1).unwrap_or(1);
        let version_id = stable_id("wfv", &(&opportunity_id, ordinal, proposal, job_id))?;
        sqlx::query("INSERT INTO opportunity_versions VALUES(?1,?2,?3,?4,?5,?6,?7,?8)")
            .bind(&version_id)
            .bind(&opportunity_id)
            .bind(ordinal)
            .bind(job_id)
            .bind(status_str(status))
            .bind(serde_json::to_string(proposal)?)
            .bind(serde_json::to_string(&eligibility.errors)?)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "UPDATE opportunities SET current_status=?2,current_version=?3,current_summary=?4,
             cadence=?5,updated_at=?6 WHERE opportunity_id=?1",
        )
        .bind(&opportunity_id)
        .bind(status_str(status))
        .bind(ordinal)
        .bind(&proposal.summary)
        .bind(cadence_str(proposal.cadence))
        .bind(&now)
        .execute(&mut *tx)
        .await?;
        if proposal.withdraw_current_finding || proposal.retire || proposal.finding.is_none() {
            sqlx::query("UPDATE findings SET active=0 WHERE opportunity_id=?1")
                .bind(&opportunity_id)
                .execute(&mut *tx)
                .await?;
        }
        if let (Some(finding), Some(handoff)) = (&proposal.finding, &proposal.handoff) {
            if finding.evidence_ids.is_empty()
                || has_duplicates(&finding.evidence_ids)
                || finding
                    .evidence_ids
                    .iter()
                    .any(|id| !prior_evidence.contains(id))
            {
                return Err(InsightsError::Invalid(
                    "finding evidence is outside opportunity history".into(),
                ));
            }
            for evidence_id in &finding.evidence_ids {
                let admissible: i64 = sqlx::query_scalar(
                    "SELECT policy_allowed AND redaction_ready AND NOT deleted AND NOT sensitive
                     FROM evidence WHERE evidence_id=?1",
                )
                .bind(evidence_id)
                .fetch_one(&mut *tx)
                .await?;
                if admissible != 1 {
                    return Err(InsightsError::Invalid(
                        "finding uses inadmissible evidence".into(),
                    ));
                }
            }
            let (rank_score, rank_vector) = rank(proposal, occurrence_count, &certainties);
            sqlx::query("UPDATE findings SET active=0 WHERE opportunity_id=?1")
                .bind(&opportunity_id)
                .execute(&mut *tx)
                .await?;
            let finding_id = stable_id("wff", &(&opportunity_id, &version_id, finding, handoff))?;
            sqlx::query(
                "INSERT INTO findings(
                   finding_id,opportunity_id,version_id,active,construct,label,claim,
                   why_worth_fixing,handoff_type,handoff_title,handoff_preview,handoff_body,
                   occurrence_count,cadence,rank_score,rank_vector_json,created_at)
                 VALUES(?1,?2,?3,1,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
            )
            .bind(&finding_id)
            .bind(&opportunity_id)
            .bind(&version_id)
            .bind(proposal.construct.as_str())
            .bind(user_label(
                proposal.construct,
                occurrence_count,
                proposal.cadence,
            ))
            .bind(&finding.claim)
            .bind(&finding.why_worth_fixing)
            .bind(handoff_str(handoff.kind))
            .bind(&handoff.title)
            .bind(handoff_preview(&handoff.body))
            .bind(&handoff.body)
            .bind(occurrence_count as i64)
            .bind(cadence_str(proposal.cadence))
            .bind(rank_score)
            .bind(serde_json::to_string(&rank_vector)?)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
            for evidence_id in &finding.evidence_ids {
                sqlx::query("INSERT INTO finding_evidence VALUES(?1,?2)")
                    .bind(&finding_id)
                    .bind(evidence_id)
                    .execute(&mut *tx)
                    .await?;
            }
            finding_total += 1;
        }
        if options.fail_after_opportunity == Some(index + 1) {
            return Err(InsightsError::Invalid("injected apply failure".into()));
        }
    }
    sqlx::query("INSERT INTO reconciliations VALUES(?1,?2,?3,?4,?5)")
        .bind(&reconciliation_id)
        .bind(job_id)
        .bind(&output_fingerprint)
        .bind(&output_json)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    let max_sequence: i64 = sqlx::query_scalar(
        "SELECT MAX(o.sequence) FROM observations o JOIN job_observations j
         ON j.observation_id=o.observation_id WHERE j.job_id=?1",
    )
    .bind(job_id)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE insights_cursor SET last_observation_sequence=?2,last_job_id=?1,updated_at=?3
         WHERE stream='explorer'",
    )
    .bind(job_id)
    .bind(max_sequence)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE inference_jobs SET status='accepted',accepted_at=?2,updated_at=?2 WHERE job_id=?1",
    )
    .bind(job_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    if let Some(receipt) = receipt {
        let attempt = sqlx::query_scalar::<_, i64>(
            "SELECT COALESCE(MAX(attempt),0)+1 FROM job_attempts WHERE job_id=?1",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("INSERT INTO job_attempts VALUES(?1,?2,?3,?4,'accepted',?5,?6,NULL,?7)")
            .bind(job_id)
            .bind(attempt)
            .bind(receipt.request_fingerprint)
            .bind(receipt.output_fingerprint)
            .bind(serde_json::to_string(&receipt.usage)?)
            .bind(receipt.latency_ms as i64)
            .bind(&now)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    recompute_surface_status(pool).await?;
    Ok(ApplyResult {
        reconciliation_id,
        already_accepted: false,
        opportunities_changed: output.opportunities.len(),
        occurrences_added: occurrence_total,
        findings_created: finding_total,
    })
}

async fn active_candidates(pool: &SqlitePool) -> Result<Vec<FindingCandidate>> {
    let rows = sqlx::query(
        "SELECT f.*,
          NOT EXISTS(SELECT 1 FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
            WHERE fe.finding_id=f.finding_id AND
            (NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive)) evidence_available
         FROM findings f WHERE f.active=1 ORDER BY f.finding_id",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            let construct = parse_construct(row.get("construct"))?;
            let cadence = parse_cadence(row.get("cadence"))?;
            let handoff_type = parse_handoff(row.get("handoff_type"))?;
            Ok(FindingCandidate {
                card: WorthFixingCard {
                    finding_id: row.get("finding_id"),
                    label: row.get("label"),
                    claim: row.get("claim"),
                    why_worth_fixing: row.get("why_worth_fixing"),
                    handoff_type,
                    handoff_title: row.get("handoff_title"),
                    handoff_preview: row.get("handoff_preview"),
                    occurrence_count: row.get::<i64, _>("occurrence_count") as u32,
                    cadence,
                    evidence_available: row.get::<bool, _>("evidence_available"),
                },
                construct,
                rank_score: row.get::<i64, _>("rank_score") as i32,
                rank_vector: serde_json::from_str(row.get("rank_vector_json"))?,
                active: row.get("active"),
            })
        })
        .collect()
}

async fn active_candidates_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<Vec<FindingCandidate>> {
    let rows = sqlx::query(
        "SELECT f.*,
          NOT EXISTS(SELECT 1 FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
            WHERE fe.finding_id=f.finding_id AND
            (NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive)) evidence_available
         FROM findings f WHERE f.active=1 ORDER BY f.finding_id",
    )
    .fetch_all(&mut **tx)
    .await?;
    rows.into_iter()
        .map(|row| {
            let construct = parse_construct(row.get("construct"))?;
            let cadence = parse_cadence(row.get("cadence"))?;
            Ok(FindingCandidate {
                card: WorthFixingCard {
                    finding_id: row.get("finding_id"),
                    label: row.get("label"),
                    claim: row.get("claim"),
                    why_worth_fixing: row.get("why_worth_fixing"),
                    handoff_type: parse_handoff(row.get("handoff_type"))?,
                    handoff_title: row.get("handoff_title"),
                    handoff_preview: row.get("handoff_preview"),
                    occurrence_count: row.get::<i64, _>("occurrence_count") as u32,
                    cadence,
                    evidence_available: row.get::<bool, _>("evidence_available"),
                },
                construct,
                rank_score: row.get::<i64, _>("rank_score") as i32,
                rank_vector: serde_json::from_str(row.get("rank_vector_json"))?,
                active: row.get("active"),
            })
        })
        .collect()
}

pub(crate) async fn recompute_surface_status_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<()> {
    let candidates = active_candidates_tx(tx).await?;
    let selected: HashSet<String> = select_top(candidates, 5)
        .into_iter()
        .map(|candidate| candidate.card.finding_id)
        .collect();
    sqlx::query(
        "UPDATE opportunities SET current_status='eligible'
         WHERE current_status='surfaced'",
    )
    .execute(&mut **tx)
    .await?;
    for finding_id in selected {
        sqlx::query(
            "UPDATE opportunities SET current_status='surfaced' WHERE opportunity_id=
             (SELECT opportunity_id FROM findings WHERE finding_id=?1)",
        )
        .bind(finding_id)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

pub async fn recompute_surface_status(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    recompute_surface_status_tx(&mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Rebuilds mutable opportunity/finding projections solely from append-only
/// versions, dispositions, and current evidence admission state.
pub async fn rebuild_projections(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE findings SET active=0")
        .execute(&mut *tx)
        .await?;
    let opportunities =
        sqlx::query("SELECT opportunity_id FROM opportunities ORDER BY opportunity_id")
            .fetch_all(&mut *tx)
            .await?;
    for row in opportunities {
        let opportunity_id: String = row.get("opportunity_id");
        let latest = sqlx::query(
            "SELECT ordinal,status,proposal_json FROM opportunity_versions
             WHERE opportunity_id=?1 ORDER BY ordinal DESC LIMIT 1",
        )
        .bind(&opportunity_id)
        .fetch_one(&mut *tx)
        .await?;
        let proposal: crate::OpportunityDelta = serde_json::from_str(latest.get("proposal_json"))?;
        let version: i64 = latest.get("ordinal");
        let status: String = latest.get("status");
        sqlx::query(
            "UPDATE opportunities SET current_version=?2,current_summary=?3,cadence=?4,current_status=?5
             WHERE opportunity_id=?1",
        )
        .bind(&opportunity_id)
        .bind(version)
        .bind(&proposal.summary)
        .bind(cadence_str(proposal.cadence))
        .bind(&status)
        .execute(&mut *tx).await?;
        let finding = sqlx::query_scalar::<_, String>(
            "SELECT f.finding_id FROM findings f JOIN opportunity_versions v ON v.version_id=f.version_id
             WHERE f.opportunity_id=?1 AND v.ordinal=?2
             AND NOT EXISTS(SELECT 1 FROM dispositions d WHERE d.finding_id=f.finding_id)
             AND NOT EXISTS(SELECT 1 FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
               WHERE fe.finding_id=f.finding_id AND
               (NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive))
             LIMIT 1",
        ).bind(&opportunity_id).bind(version).fetch_optional(&mut *tx).await?;
        if let Some(finding_id) = finding {
            sqlx::query("UPDATE findings SET active=1 WHERE finding_id=?1")
                .bind(finding_id)
                .execute(&mut *tx)
                .await?;
        } else if sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM dispositions d JOIN findings f ON f.finding_id=d.finding_id
             WHERE f.opportunity_id=?1",
        )
        .bind(&opportunity_id)
        .fetch_one(&mut *tx)
        .await?
            > 0
        {
            sqlx::query(
                "UPDATE opportunities SET current_status='withdrawn' WHERE opportunity_id=?1",
            )
            .bind(&opportunity_id)
            .execute(&mut *tx)
            .await?;
        }
    }
    tx.commit().await?;
    recompute_surface_status(pool).await
}

pub async fn projection_fingerprint(pool: &SqlitePool) -> Result<String> {
    let opportunities = sqlx::query(
        "SELECT opportunity_id,construct,signature,current_status,current_version,current_summary,cadence
         FROM opportunities ORDER BY opportunity_id",
    ).fetch_all(pool).await?.into_iter().map(|row| serde_json::json!({
        "id": row.get::<String, _>("opportunity_id"),
        "construct": row.get::<String, _>("construct"),
        "signature": row.get::<String, _>("signature"),
        "status": row.get::<String, _>("current_status"),
        "version": row.get::<i64, _>("current_version"),
        "summary": row.get::<String, _>("current_summary"),
        "cadence": row.get::<String, _>("cadence"),
    })).collect::<Vec<_>>();
    let active_findings = sqlx::query_scalar::<_, String>(
        "SELECT finding_id FROM findings WHERE active=1 ORDER BY finding_id",
    )
    .fetch_all(pool)
    .await?;
    fingerprint(&serde_json::json!({
        "opportunities": opportunities,
        "active_findings": active_findings,
    }))
}

pub async fn worth_fixing_summary(
    pool: &SqlitePool,
    provider_ready: bool,
) -> Result<WorthFixingSummary> {
    let candidates = active_candidates(pool).await?;
    let eligible_count = candidates.len() as u32;
    let selected = select_top(candidates, 5)
        .into_iter()
        .map(|candidate| candidate.card)
        .collect();
    let watching_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM opportunities WHERE current_status='watching'",
    )
    .fetch_one(pool)
    .await? as u32;
    let pending_observation_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM observations o WHERE NOT EXISTS
         (SELECT 1 FROM job_observations j WHERE j.observation_id=o.observation_id)",
    )
    .fetch_one(pool)
    .await? as u32;
    let has_generated_finding =
        sqlx::query_scalar::<_, i64>("SELECT EXISTS(SELECT 1 FROM findings)")
            .fetch_one(pool)
            .await?
            != 0;
    let observation_span_hours = sqlx::query_scalar::<_, f64>(
        "SELECT CAST(
             COALESCE(MAX(julianday(occurred_at)) - MIN(julianday(occurred_at)), 0)
             AS REAL
         ) * 24.0
         FROM observations",
    )
    .fetch_one(pool)
    .await?;
    let manual_refresh_ready = has_generated_finding
        || observation_span_hours >= MANUAL_REFRESH_MIN_OBSERVATION_SPAN_HOURS;
    let processing = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM inference_jobs WHERE status IN ('pending','running')",
    )
    .fetch_one(pool)
    .await?
        > 0;
    let stale_evidence_count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT f.finding_id) FROM findings f JOIN finding_evidence fe
         ON fe.finding_id=f.finding_id JOIN evidence e ON e.evidence_id=fe.evidence_id
         WHERE NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive",
    )
    .fetch_one(pool)
    .await? as u32;
    let last_successful_wake_at = sqlx::query_scalar::<_, String>(
        "SELECT accepted_at FROM inference_jobs WHERE status='accepted'
         ORDER BY accepted_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(WorthFixingSummary {
        selected,
        eligible_count,
        watching_count,
        pending_observation_count,
        manual_refresh_ready,
        processing,
        stale_evidence_count,
        provider_ready,
        last_successful_wake_at,
    })
}

pub async fn normal_wakes_started(pool: &SqlitePool, local_day: &str) -> Result<u8> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM wake_starts WHERE local_day=?1 AND normal=1",
    )
    .bind(local_day)
    .fetch_one(pool)
    .await?
    .clamp(0, u8::MAX as i64) as u8)
}

pub async fn capture_cursor(pool: &SqlitePool, source: &str) -> Result<i64> {
    Ok(
        sqlx::query_scalar::<_, i64>("SELECT last_row_id FROM capture_cursors WHERE source=?1")
            .bind(source)
            .fetch_optional(pool)
            .await?
            .unwrap_or(0),
    )
}

pub async fn advance_capture_cursor(pool: &SqlitePool, source: &str, row_id: i64) -> Result<()> {
    let current = capture_cursor(pool, source).await?;
    if row_id < current {
        return Err(InsightsError::Invalid(
            "capture cursor cannot move backwards".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO capture_cursors VALUES(?1,?2,?3)
         ON CONFLICT(source) DO UPDATE SET last_row_id=?2,updated_at=?3",
    )
    .bind(source)
    .bind(row_id)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn load_compaction_state(pool: &SqlitePool) -> Result<crate::CompactionState> {
    let value = sqlx::query_scalar::<_, String>(
        "SELECT state_json FROM compaction_checkpoints WHERE stream='capture'",
    )
    .fetch_optional(pool)
    .await?;
    value
        .map(|value| serde_json::from_str(&value).map_err(Into::into))
        .unwrap_or_else(|| Ok(Default::default()))
}

pub async fn save_compaction_state(
    pool: &SqlitePool,
    state: &crate::CompactionState,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO compaction_checkpoints VALUES('capture',?1,?2)
         ON CONFLICT(stream) DO UPDATE SET state_json=?1,updated_at=?2",
    )
    .bind(serde_json::to_string(state)?)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn commit_compaction_checkpoint(
    pool: &SqlitePool,
    state: &crate::CompactionState,
    frame_row_id: i64,
    event_row_id: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    for (source, row_id) in [("frames", frame_row_id), ("events", event_row_id)] {
        let current =
            sqlx::query_scalar::<_, i64>("SELECT last_row_id FROM capture_cursors WHERE source=?1")
                .bind(source)
                .fetch_optional(&mut *tx)
                .await?
                .unwrap_or(0);
        if row_id < current {
            return Err(InsightsError::Invalid(
                "capture cursor cannot move backwards".into(),
            ));
        }
        sqlx::query(
            "INSERT INTO capture_cursors VALUES(?1,?2,?3)
             ON CONFLICT(source) DO UPDATE SET last_row_id=?2,updated_at=?3",
        )
        .bind(source)
        .bind(row_id)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await?;
    }
    sqlx::query(
        "INSERT INTO compaction_checkpoints VALUES('capture',?1,?2)
         ON CONFLICT(stream) DO UPDATE SET state_json=?1,updated_at=?2",
    )
    .bind(serde_json::to_string(state)?)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn known_source_refs(pool: &SqlitePool, limit: u32) -> Result<Vec<(String, String)>> {
    Ok(sqlx::query(
        "SELECT source_namespace,source_id FROM evidence WHERE deleted=0 ORDER BY occurred_at LIMIT ?1",
    )
    .bind(limit.clamp(1, 10_000))
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| (row.get("source_namespace"), row.get("source_id")))
    .collect())
}

pub async fn enhanced_diagnostics_enabled(pool: &SqlitePool) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT value FROM insights_metadata WHERE key='enhanced_diagnostics'",
    )
    .fetch_optional(pool)
    .await?
    .is_some_and(|value| value == "true"))
}

pub async fn set_enhanced_diagnostics(pool: &SqlitePool, enabled: bool) -> Result<()> {
    sqlx::query(
        "INSERT INTO insights_metadata(key,value) VALUES('enhanced_diagnostics',?1)
         ON CONFLICT(key) DO UPDATE SET value=?1",
    )
    .bind(if enabled { "true" } else { "false" })
    .execute(pool)
    .await?;
    Ok(())
}

/// Clears all user-derived Worth Fixing and Ready-to-use content while keeping
/// the migrated schema usable. This is the insights side of the app-wide
/// delete-everything operation.
pub async fn delete_all_insights_data(pool: &SqlitePool) -> Result<()> {
    let mut tx = pool.begin().await?;
    for table in [
        "ask_attempts",
        "ask_jobs",
        "ask_questions",
        "ask_messages",
        "ask_sessions",
        "artifact_change_attempts",
        "artifact_versions",
        "artifact_change_jobs",
        "artifact_events",
        "artifacts",
        "dispositions",
        "finding_evidence",
        "findings",
        "occurrences",
        "opportunity_versions",
        "opportunities",
        "reconciliations",
        "job_attempts",
        "job_observations",
        "inference_jobs",
        "explorer_attempts",
        "explorer_jobs",
        "observations",
        "evidence",
        "capabilities",
        "wake_starts",
        "capture_cursors",
        "compaction_checkpoints",
        "insights_cursor",
    ] {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut *tx)
            .await?;
    }
    sqlx::query("DELETE FROM insights_metadata WHERE key!='schema_version'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO insights_cursor(stream,last_observation_sequence,updated_at)
         VALUES('explorer',0,?1)",
    )
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn record_wake_start(
    pool: &SqlitePool,
    local_day: &str,
    reason: &str,
    normal: bool,
) -> Result<String> {
    let mut tx = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    let wake_id = stable_id("wfw", &(local_day, reason, normal, &now))?;
    sqlx::query("INSERT INTO wake_starts VALUES(?1,?2,?3,?4,?5)")
        .bind(&wake_id)
        .bind(local_day)
        .bind(reason)
        .bind(normal)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(wake_id)
}

pub async fn other_findings(
    pool: &SqlitePool,
    after_finding_id: Option<&str>,
    limit: u32,
) -> Result<FindingPage> {
    let candidates = active_candidates(pool).await?;
    let selected: HashSet<String> = select_top(candidates.clone(), 5)
        .into_iter()
        .map(|candidate| candidate.card.finding_id)
        .collect();
    let mut others: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| !selected.contains(&candidate.card.finding_id))
        .map(|candidate| candidate.card)
        .collect();
    others.sort_by(|left, right| left.finding_id.cmp(&right.finding_id));
    if let Some(cursor) = after_finding_id {
        others.retain(|card| card.finding_id.as_str() > cursor);
    }
    let limit = limit.clamp(1, 50) as usize;
    let has_more = others.len() > limit;
    others.truncate(limit);
    let next_cursor = has_more
        .then(|| others.last().map(|card| card.finding_id.clone()))
        .flatten();
    Ok(FindingPage {
        items: others,
        next_cursor,
    })
}

pub async fn finding_evidence(
    pool: &SqlitePool,
    finding_id: &str,
    limit: u32,
) -> Result<Vec<WorthFixingEvidenceLine>> {
    let rows = sqlx::query(
        "SELECT e.evidence_id,e.occurred_at,e.app,e.excerpt,
          e.policy_allowed AND e.redaction_ready AND NOT e.deleted AND NOT e.sensitive available
         FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
         WHERE fe.finding_id=?1 ORDER BY e.occurred_at,e.evidence_id LIMIT ?2",
    )
    .bind(finding_id)
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let available = row.get::<bool, _>("available");
            WorthFixingEvidenceLine {
                evidence_id: row.get("evidence_id"),
                occurred_at: row.get("occurred_at"),
                app: row.get("app"),
                description: if available {
                    row.get::<String, _>("excerpt").chars().take(500).collect()
                } else {
                    "This evidence is no longer available.".into()
                },
                available,
            }
        })
        .collect())
}

pub(crate) fn disposition_str(value: DispositionKind) -> &'static str {
    match value {
        DispositionKind::Accepted => "accepted",
        DispositionKind::Saved => "saved",
        DispositionKind::NotAProblem => "not_a_problem",
        DispositionKind::LeaveIt => "leave_it",
        DispositionKind::CloseBut => "close_but",
    }
}

pub async fn record_disposition(
    pool: &SqlitePool,
    finding_id: &str,
    kind: DispositionKind,
    correction_text: Option<&str>,
    intent: Option<&str>,
) -> Result<String> {
    if kind == DispositionKind::CloseBut
        && correction_text.is_none_or(|value| value.trim().is_empty())
    {
        return Err(InsightsError::Invalid(
            "close-but requires correction text".into(),
        ));
    }
    let exists = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM findings WHERE finding_id=?1 AND active=1",
    )
    .bind(finding_id)
    .fetch_one(pool)
    .await?;
    if exists != 1 {
        return Err(InsightsError::Invalid("finding is not active".into()));
    }
    let now = Utc::now().to_rfc3339();
    let disposition_id = stable_id(
        "wfd",
        &(
            finding_id,
            disposition_str(kind),
            correction_text,
            intent,
            &now,
        ),
    )?;
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO dispositions VALUES(?1,?2,?3,?4,?5,?6)")
        .bind(&disposition_id)
        .bind(finding_id)
        .bind(disposition_str(kind))
        .bind(correction_text)
        .bind(intent)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    sqlx::query("UPDATE findings SET active=0 WHERE finding_id=?1")
        .bind(finding_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE opportunities SET current_status='withdrawn',updated_at=?2 WHERE opportunity_id=
         (SELECT opportunity_id FROM findings WHERE finding_id=?1)",
    )
    .bind(finding_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    recompute_surface_status_tx(&mut tx).await?;
    tx.commit().await?;
    Ok(disposition_id)
}

pub async fn pending_observations(pool: &SqlitePool, limit: u32) -> Result<Vec<ObservationRecord>> {
    let rows = sqlx::query(
        "SELECT o.* FROM observations o WHERE NOT EXISTS
         (SELECT 1 FROM job_observations j WHERE j.observation_id=o.observation_id)
         ORDER BY o.sequence LIMIT ?1",
    )
    .bind(limit.clamp(1, 1000))
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok(ObservationRecord {
                observation_id: row.get("observation_id"),
                source_key: row.get("source_key"),
                occurred_at: row.get("occurred_at"),
                statement: row.get("statement"),
                certainty: parse_certainty(row.get("certainty"))?,
                evidence_ids: serde_json::from_str(row.get("evidence_ids_json"))?,
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingObservationStats {
    pub count: usize,
    pub episode_groups: usize,
    pub oldest_admitted_at: Option<String>,
}

/// Scheduling metadata for observations not yet owned by a Steward job.
/// Admission time controls queue latency; activity time is used only to
/// estimate distinct ten-minute work episodes. Historical backfill therefore
/// cannot look overdue the instant Explorer admits it.
pub async fn pending_observation_stats(pool: &SqlitePool) -> Result<PendingObservationStats> {
    let rows = sqlx::query(
        "SELECT o.occurred_at,o.admitted_at FROM observations o WHERE NOT EXISTS
         (SELECT 1 FROM job_observations j WHERE j.observation_id=o.observation_id)",
    )
    .fetch_all(pool)
    .await?;
    let count = rows.len();
    let oldest_admitted_at = rows
        .iter()
        .filter_map(|row| row.get::<Option<String>, _>("admitted_at"))
        .min();
    let mut occurred = rows
        .iter()
        .filter_map(|row| {
            DateTime::parse_from_rfc3339(row.get::<String, _>("occurred_at").as_str())
                .ok()
                .map(|value| value.with_timezone(&Utc))
        })
        .collect::<Vec<_>>();
    occurred.sort_unstable();
    let episode_groups = if count == 0 {
        0
    } else if occurred.is_empty() {
        1
    } else {
        1 + occurred
            .windows(2)
            .filter(|pair| (pair[1] - pair[0]).num_minutes() >= 10)
            .count()
    };
    Ok(PendingObservationStats {
        count,
        episode_groups,
        oldest_admitted_at,
    })
}

pub async fn last_successful_steward_wake_at(pool: &SqlitePool) -> Result<Option<String>> {
    Ok(sqlx::query_scalar::<_, String>(
        "SELECT accepted_at FROM inference_jobs WHERE status='accepted'
         ORDER BY accepted_at DESC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?)
}

pub async fn wake_reason_started(pool: &SqlitePool, local_day: &str, reason: &str) -> Result<bool> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT EXISTS(SELECT 1 FROM wake_starts WHERE local_day=?1 AND reason=?2)",
    )
    .bind(local_day)
    .bind(reason)
    .fetch_one(pool)
    .await?
        != 0)
}

pub async fn watching_opportunity_count(pool: &SqlitePool) -> Result<u32> {
    Ok(sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM opportunities WHERE current_status='watching'",
    )
    .fetch_one(pool)
    .await? as u32)
}

pub async fn mark_job_failed(pool: &SqlitePool, job_id: &str, error_code: &str) -> Result<()> {
    sqlx::query(
        "UPDATE inference_jobs SET status='pending',error_code=?2,updated_at=?3
         WHERE job_id=?1 AND status!='accepted'",
    )
    .bind(job_id)
    .bind(error_code)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_job_rejected(pool: &SqlitePool, job_id: &str, error_code: &str) -> Result<()> {
    sqlx::query(
        "UPDATE inference_jobs SET status='rejected',error_code=?2,updated_at=?3
         WHERE job_id=?1 AND status!='accepted'",
    )
    .bind(job_id)
    .bind(error_code)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Supersedes failed or interrupted bulk backfill jobs and releases their
/// exclusive observation reservations. Jobs and attempts remain as audit
/// history; this is deliberately an explicit developer/backfill operation.
pub async fn release_bulk_backfill_job_observations(pool: &SqlitePool) -> Result<u64> {
    let mut tx = pool.begin().await?;
    let superseded_at = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE inference_jobs
         SET status='rejected',
             error_code='superseded_by_steward_only',
             input_fingerprint=input_fingerprint || ':superseded:' || ?1,
             updated_at=?1
         WHERE reason IN ('fixture_backfill','fixture_backfill_steward_only')
           AND status IN ('pending','running','rejected')",
    )
    .bind(&superseded_at)
    .execute(&mut *tx)
    .await?;
    let released = sqlx::query(
        "DELETE FROM job_observations WHERE job_id IN
         (SELECT job_id FROM inference_jobs
          WHERE status='rejected' AND reason IN ('fixture_backfill','fixture_backfill_steward_only'))",
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    tx.commit().await?;
    Ok(released)
}

pub async fn record_job_attempt(
    pool: &SqlitePool,
    job_id: &str,
    request_fingerprint: &str,
    output_fingerprint: Option<&str>,
    status: &str,
    usage: &impl Serialize,
    latency_ms: u64,
    error_code: Option<&str>,
) -> Result<u32> {
    let attempt = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(attempt),0)+1 FROM job_attempts WHERE job_id=?1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    sqlx::query("INSERT INTO job_attempts VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)")
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
        .await?;
    Ok(attempt as u32)
}

pub async fn register_capability(
    pool: &SqlitePool,
    capability_id: &str,
    app: &str,
    description: &str,
) -> Result<()> {
    let immutable = fingerprint(&(app, description))?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT immutable_fingerprint FROM capabilities WHERE capability_id=?1")
            .bind(capability_id)
            .fetch_optional(pool)
            .await?;
    if let Some(existing) = existing {
        if existing != immutable {
            return Err(InsightsError::IdentityCollision(capability_id.into()));
        }
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO capabilities(
          capability_id,app,description,immutable_fingerprint,action_kind,action_target)
         VALUES(?1,?2,?3,?4,NULL,NULL)",
    )
    .bind(capability_id)
    .bind(app)
    .bind(description)
    .bind(immutable)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn register_actionable_capability(
    pool: &SqlitePool,
    capability_id: &str,
    app: &str,
    description: &str,
    action_target: &str,
) -> Result<()> {
    if !action_target.starts_with("https://")
        || action_target.chars().count() > 2_048
        || action_target.chars().any(char::is_control)
    {
        return Err(InsightsError::Invalid(
            "capability action target must be a bounded HTTPS URL".into(),
        ));
    }
    let immutable = fingerprint(&(app, description, "https_url", action_target))?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT immutable_fingerprint FROM capabilities WHERE capability_id=?1")
            .bind(capability_id)
            .fetch_optional(pool)
            .await?;
    if let Some(existing) = existing {
        if existing != immutable {
            return Err(InsightsError::IdentityCollision(capability_id.into()));
        }
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO capabilities(
          capability_id,app,description,immutable_fingerprint,action_kind,action_target)
         VALUES(?1,?2,?3,?4,'https_url',?5)",
    )
    .bind(capability_id)
    .bind(app)
    .bind(description)
    .bind(immutable)
    .bind(action_target)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::{
        CandidateAssessment, CandidateReasonCode, EvidenceQuality, FindingDraft, Handoff,
        OccurrenceDelta, OpportunityDelta, RankSignals,
    };

    async fn setup() -> (TempDir, SqlitePool) {
        let directory = tempfile::tempdir().unwrap();
        let pool = open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();
        (directory, pool)
    }

    fn assessment(
        decision: CandidateDecision,
        opportunity_local_id: Option<&str>,
        missing_to_qualify: Vec<&str>,
    ) -> CandidateAssessment {
        CandidateAssessment {
            local_id: "candidate_01".into(),
            observation_ids: vec![format!("obl_{:024x}", 1)],
            decision,
            reason_code: if decision == CandidateDecision::Qualified {
                CandidateReasonCode::MeaningfulRepeatedWork
            } else {
                CandidateReasonCode::MeaningfulButImmature
            },
            reason: "The supplied observation supports this candidate.".into(),
            shared_goal: "Prepare a useful report".into(),
            reducible_burden: "Repeated manual preparation".into(),
            stable_steps: vec!["prepare report".into()],
            variable_inputs: vec![],
            distinct_episode_basis: vec![],
            missing_to_qualify: missing_to_qualify.into_iter().map(str::to_owned).collect(),
            opportunity_local_id: opportunity_local_id.map(str::to_owned),
        }
    }

    async fn claimed_job(pool: &SqlitePool) -> String {
        let item = observation(1);
        upsert_evidence(pool, &evidence(1)).await.unwrap();
        admit_observation(pool, &item).await.unwrap();
        let observation_ids = [item.observation_id];
        let job_id = create_job(
            pool,
            NewJob {
                input_fingerprint: "candidate-assessment-test",
                local_day: "2026-01-01",
                reason: "test",
                observation_ids: &observation_ids,
                prompt_hash: "prompt",
                schema_hash: "schema",
                model: "mock",
                input_json: "{}",
            },
        )
        .await
        .unwrap();
        claim_job(pool, &job_id).await.unwrap();
        job_id
    }

    #[tokio::test]
    async fn candidate_assessments_are_validated_but_not_durable() {
        let (_directory, pool) = setup().await;
        let job_id = claimed_job(&pool).await;
        let output = ReconciliationOutput {
            schema_version: 2,
            considered_observation_ids: vec![format!("obl_{:024x}", 1)],
            opportunities: vec![delta(1, None, true)],
            candidate_assessments: vec![assessment(
                CandidateDecision::Qualified,
                Some("opp_01"),
                vec![],
            )],
        };
        apply_reconciliation(&pool, &job_id, &output, ApplyOptions::default())
            .await
            .unwrap();
        let stored: String = sqlx::query_scalar("SELECT output_json FROM reconciliations")
            .fetch_one(&pool)
            .await
            .unwrap();
        let parsed: ReconciliationOutput = serde_json::from_str(&stored).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert!(parsed.candidate_assessments.is_empty());
        assert!(!stored.contains("candidate_assessments"));
        let diagnostics = steward_diagnostics(&pool).await.unwrap();
        assert!(diagnostics["reconciliations"][0]["candidate_assessments"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(diagnostics["steward_attempts"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn invalid_candidate_assessments_are_rejected_atomically() {
        let (_directory, pool) = setup().await;
        let job_id = claimed_job(&pool).await;
        let output = ReconciliationOutput {
            schema_version: 2,
            considered_observation_ids: vec![format!("obl_{:024x}", 1)],
            opportunities: vec![delta(1, None, true)],
            candidate_assessments: vec![assessment(
                CandidateDecision::Discarded,
                Some("opp_01"),
                vec![],
            )],
        };
        assert!(
            apply_reconciliation(&pool, &job_id, &output, ApplyOptions::default())
                .await
                .is_err()
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM opportunities")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM reconciliations")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn steward_replay_copies_only_evidence_and_observations() {
        let (source_dir, source) = setup().await;
        upsert_evidence(&source, &evidence(1)).await.unwrap();
        admit_observation(&source, &observation(1)).await.unwrap();
        let destination_dir = tempfile::tempdir().unwrap();
        let destination = open_insights_database(destination_dir.path().join("replay.sqlite"))
            .await
            .unwrap();
        let copied =
            copy_observations_for_steward_replay(&source, &destination, "source-fixture-1")
                .await
                .unwrap();
        assert_eq!(copied, 1);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observations")
                .fetch_one(&destination)
                .await
                .unwrap(),
            1
        );
        for table in [
            "inference_jobs",
            "opportunities",
            "occurrences",
            "findings",
            "reconciliations",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&destination)
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM observations")
                .fetch_one(&source)
                .await
                .unwrap(),
            1
        );
        drop(source_dir);
    }

    fn evidence(index: usize) -> EvidenceRecord {
        EvidenceRecord {
            evidence_id: format!("frame:{index}"),
            source_namespace: "device:test".into(),
            source_id: format!("frame:{index}"),
            occurred_at: format!("2026-01-{:02}T09:00:00Z", index.min(28)),
            app: Some("Editor".into()),
            window: Some("Report".into()),
            excerpt: format!("Evidence for occurrence {index}"),
            policy_allowed: true,
            redaction_ready: true,
            deleted: false,
            sensitive: false,
        }
    }

    #[tokio::test]
    async fn forgetting_capture_evidence_scrubs_retained_content() {
        let (_directory, pool) = setup().await;
        let mut record = evidence(1);
        record.source_namespace = "local-capture".into();
        record.app = Some("Mail".into());
        record.window = Some("Private subject".into());
        record.excerpt = "Private message body".into();
        upsert_evidence(&pool, &record).await.unwrap();

        let (forgotten, findings) =
            forget_capture_evidence(&pool, "local-capture", &["frame:1".to_string()])
                .await
                .unwrap();

        assert_eq!(forgotten, 1);
        assert_eq!(findings, 0);
        let row = sqlx::query(
            "SELECT excerpt,app,window,deleted FROM evidence WHERE source_namespace='local-capture' AND source_id='frame:1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(row.get::<String, _>("excerpt"), "");
        assert_eq!(row.get::<Option<String>, _>("app"), None);
        assert_eq!(row.get::<Option<String>, _>("window"), None);
        assert_eq!(row.get::<i64, _>("deleted"), 1);
    }

    fn observation(index: usize) -> ObservationRecord {
        ObservationRecord {
            observation_id: format!("obl_{index:024x}"),
            source_key: format!("test:{index}"),
            occurred_at: format!("2026-01-{:02}T09:00:00Z", index.min(28)),
            statement: format!("A useful observation number {index}"),
            certainty: ObservationCertainty::Explicit,
            evidence_ids: vec![format!("frame:{index}")],
        }
    }

    fn delta(index: usize, existing: Option<String>, with_finding: bool) -> OpportunityDelta {
        OpportunityDelta {
            local_id: "opp_01".into(),
            existing_opportunity_id: existing,
            construct: Construct::Recognition,
            summary: "Prepare this report with a reusable prompt".into(),
            signature: "prepare-report".into(),
            occurrences_to_add: vec![OccurrenceDelta {
                local_id: "occ_01".into(),
                observation_ids: vec![format!("obl_{index:024x}")],
                evidence_ids: vec![format!("frame:{index}")],
                steps: vec!["prepare report".into()],
                distinctness_basis: if index == 1 {
                    vec![]
                } else {
                    vec!["separate_input".into()]
                },
            }],
            withdraw_current_finding: false,
            retire: false,
            transfer_established: false,
            authorship_established: false,
            cadence: Cadence::None,
            unresolved_questions: vec![],
            evidence_quality: EvidenceQuality::High,
            handoff: Some(Handoff {
                kind: HandoffType::Prompt,
                title: "Prepare the report".into(),
                body: "Turn the supplied material into the expected report.".into(),
                capability_id: None,
            }),
            automation_potential: false,
            rank_signals: RankSignals {
                actionability: 3,
                estimated_burden: 2,
                novelty: 2,
                user_relevance: 3,
                sensitivity_risk: 0,
            },
            finding: with_finding.then(|| FindingDraft {
                claim: "A reusable prompt can prepare this report.".into(),
                why_worth_fixing: "This avoids rebuilding the same instructions.".into(),
                evidence_ids: vec![format!("frame:{index}")],
            }),
        }
    }

    async fn insert_source(pool: &SqlitePool, index: usize) {
        upsert_evidence(pool, &evidence(index)).await.unwrap();
        admit_observation(pool, &observation(index)).await.unwrap();
    }

    async fn job(pool: &SqlitePool, index: usize) -> String {
        let observation_id = format!("obl_{index:024x}");
        create_job(
            pool,
            NewJob {
                input_fingerprint: &format!("input-{index}"),
                local_day: "2026-01-01",
                reason: "test",
                observation_ids: &[observation_id],
                prompt_hash: "prompt",
                schema_hash: "schema",
                model: "mock",
                input_json: "{}",
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn crash_rollback_and_committed_before_receipt_resume_are_safe() {
        let (_directory, pool) = setup().await;
        insert_source(&pool, 1).await;
        let job_id = job(&pool, 1).await;
        claim_job(&pool, &job_id).await.unwrap();
        let output = ReconciliationOutput {
            schema_version: 1,
            considered_observation_ids: vec![format!("obl_{:024x}", 1)],
            opportunities: vec![delta(1, None, true)],
            candidate_assessments: vec![],
        };
        let failed = apply_reconciliation(
            &pool,
            &job_id,
            &output,
            ApplyOptions {
                fail_after_opportunity: Some(1),
            },
        )
        .await;
        assert!(failed.is_err());
        assert!(!accepted_job(&pool, &job_id).await.unwrap());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM opportunities")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );

        let first = apply_reconciliation(&pool, &job_id, &output, ApplyOptions::default())
            .await
            .unwrap();
        assert!(!first.already_accepted);
        // This is the commit-before-filesystem-receipt restart boundary: the
        // durable job is checked before output is revalidated against new state.
        let resumed = apply_reconciliation(&pool, &job_id, &output, ApplyOptions::default())
            .await
            .unwrap();
        assert!(resumed.already_accepted);
        assert_eq!(first.reconciliation_id, resumed.reconciliation_id);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM findings")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn delta_updates_support_more_than_twenty_occurrences() {
        let (_directory, pool) = setup().await;
        let mut opportunity_id = None;
        for index in 1..=21 {
            insert_source(&pool, index).await;
            let job_id = job(&pool, index).await;
            claim_job(&pool, &job_id).await.unwrap();
            let output = ReconciliationOutput {
                schema_version: 1,
                considered_observation_ids: vec![format!("obl_{index:024x}")],
                opportunities: vec![delta(index, opportunity_id.clone(), index == 1)],
                candidate_assessments: vec![],
            };
            apply_reconciliation(&pool, &job_id, &output, ApplyOptions::default())
                .await
                .unwrap();
            if opportunity_id.is_none() {
                opportunity_id =
                    sqlx::query_scalar("SELECT opportunity_id FROM opportunities LIMIT 1")
                        .fetch_optional(&pool)
                        .await
                        .unwrap();
            }
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM occurrences")
                .fetch_one(&pool)
                .await
                .unwrap(),
            21
        );
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT current_version FROM opportunities")
                .fetch_one(&pool)
                .await
                .unwrap(),
            21
        );
    }

    #[tokio::test]
    async fn fresh_database_has_a_valid_empty_worth_fixing_summary() {
        let (_directory, pool) = setup().await;
        let summary = worth_fixing_summary(&pool, false).await.unwrap();
        assert!(summary.selected.is_empty());
        assert_eq!(summary.eligible_count, 0);
        assert_eq!(summary.watching_count, 0);
        assert_eq!(summary.pending_observation_count, 0);
        assert!(!summary.manual_refresh_ready);
        assert!(!summary.processing);
        assert!(!summary.provider_ready);
    }

    #[tokio::test]
    async fn manual_refresh_waits_for_three_hours_of_explorer_observations() {
        let (_directory, pool) = setup().await;
        upsert_evidence(&pool, &evidence(1)).await.unwrap();
        let mut first = observation(1);
        first.occurred_at = "2026-01-01T09:00:00Z".into();
        admit_observation(&pool, &first).await.unwrap();
        assert!(
            !worth_fixing_summary(&pool, true)
                .await
                .unwrap()
                .manual_refresh_ready
        );

        upsert_evidence(&pool, &evidence(2)).await.unwrap();
        let mut second = observation(2);
        second.occurred_at = "2026-01-01T11:59:00Z".into();
        admit_observation(&pool, &second).await.unwrap();
        assert!(
            !worth_fixing_summary(&pool, true)
                .await
                .unwrap()
                .manual_refresh_ready
        );

        upsert_evidence(&pool, &evidence(3)).await.unwrap();
        let mut third = observation(3);
        third.occurred_at = "2026-01-01T12:00:00Z".into();
        admit_observation(&pool, &third).await.unwrap();
        assert!(
            worth_fixing_summary(&pool, true)
                .await
                .unwrap()
                .manual_refresh_ready
        );
    }

    #[tokio::test]
    async fn admission_rejects_policy_barred_evidence_and_withdraws_deleted_cards() {
        let (_directory, pool) = setup().await;
        let mut barred = evidence(1);
        barred.redaction_ready = false;
        upsert_evidence(&pool, &barred).await.unwrap();
        assert!(admit_observation(&pool, &observation(1)).await.is_err());

        barred.redaction_ready = true;
        upsert_evidence(&pool, &barred).await.unwrap();
        admit_observation(&pool, &observation(1)).await.unwrap();
        let job_id = job(&pool, 1).await;
        claim_job(&pool, &job_id).await.unwrap();
        apply_reconciliation(
            &pool,
            &job_id,
            &ReconciliationOutput {
                schema_version: 1,
                considered_observation_ids: vec![format!("obl_{:024x}", 1)],
                opportunities: vec![delta(1, None, true)],
                candidate_assessments: vec![],
            },
            ApplyOptions::default(),
        )
        .await
        .unwrap();
        let generated = worth_fixing_summary(&pool, true).await.unwrap();
        assert_eq!(generated.selected.len(), 1);
        assert!(generated.manual_refresh_ready);
        barred.deleted = true;
        upsert_evidence(&pool, &barred).await.unwrap();
        let withdrawn = worth_fixing_summary(&pool, true).await.unwrap();
        assert!(withdrawn.selected.is_empty());
        assert!(withdrawn.manual_refresh_ready);
    }

    #[tokio::test]
    async fn immutable_identity_collisions_are_rejected() {
        let (_directory, pool) = setup().await;
        let original = evidence(1);
        upsert_evidence(&pool, &original).await.unwrap();
        let mut changed = original;
        changed.excerpt = "different immutable source content".into();
        assert!(matches!(
            upsert_evidence(&pool, &changed).await,
            Err(InsightsError::IdentityCollision(_))
        ));
    }

    #[tokio::test]
    async fn capability_catalog_allows_only_bounded_https_targets() {
        let (_directory, pool) = setup().await;
        assert!(register_actionable_capability(
            &pool,
            "cap_safe",
            "Editor",
            "Open documentation",
            "https://example.com/docs"
        )
        .await
        .is_ok());
        for target in [
            "http://example.com",
            "file:///tmp/private",
            "javascript:alert(1)",
            "https://example.com/\nunsafe",
        ] {
            assert!(register_actionable_capability(
                &pool,
                &format!("cap_{}", target.len()),
                "Editor",
                "Unsafe",
                target
            )
            .await
            .is_err());
        }
    }

    #[tokio::test]
    async fn cursor_is_ordered_and_job_requires_exact_observation_ownership() {
        let (_directory, pool) = setup().await;
        insert_source(&pool, 1).await;
        insert_source(&pool, 2).await;
        let observation_ids = vec![format!("obl_{:024x}", 1), format!("obl_{:024x}", 2)];
        let job_id = create_job(
            &pool,
            NewJob {
                input_fingerprint: "ordered-input",
                local_day: "2026-01-01",
                reason: "test",
                observation_ids: &observation_ids,
                prompt_hash: "prompt",
                schema_hash: "schema",
                model: "mock",
                input_json: "{}",
            },
        )
        .await
        .unwrap();
        assert_eq!(
            job_observation_ids(&pool, &job_id).await.unwrap(),
            observation_ids
        );
        claim_job(&pool, &job_id).await.unwrap();
        let wrong = ReconciliationOutput {
            schema_version: 1,
            considered_observation_ids: vec![format!("obl_{:024x}", 1)],
            opportunities: vec![],
            candidate_assessments: vec![],
        };
        assert!(
            apply_reconciliation(&pool, &job_id, &wrong, ApplyOptions::default())
                .await
                .is_err()
        );
        let correct = ReconciliationOutput {
            schema_version: 1,
            considered_observation_ids: observation_ids,
            opportunities: vec![],
            candidate_assessments: vec![],
        };
        apply_reconciliation(&pool, &job_id, &correct, ApplyOptions::default())
            .await
            .unwrap();
        let cursor: i64 = sqlx::query_scalar(
            "SELECT last_observation_sequence FROM insights_cursor WHERE stream='explorer'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(cursor, 2);
    }

    #[tokio::test]
    async fn unsupported_schema_version_is_rejected_without_mutation() {
        let (directory, pool) = setup().await;
        sqlx::query("UPDATE insights_metadata SET value='999' WHERE key='schema_version'")
            .execute(&pool)
            .await
            .unwrap();
        pool.close().await;
        let reopened = open_insights_database(directory.path().join("insights.sqlite")).await;
        assert!(matches!(
            reopened,
            Err(InsightsError::UnsupportedSchema(999))
        ));
    }

    #[tokio::test]
    async fn schema_one_database_migrates_to_artifacts_without_losing_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("schema-one.sqlite");
        let options = SqliteConnectOptions::from_str(path.to_string_lossy().as_ref())
            .unwrap()
            .create_if_missing(true);
        let legacy = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE insights_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE insights_schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
             CREATE TABLE capabilities(capability_id TEXT PRIMARY KEY,app TEXT NOT NULL,
               description TEXT NOT NULL,immutable_fingerprint TEXT NOT NULL);",
        )
        .execute(&legacy)
        .await
        .unwrap();
        sqlx::query("INSERT INTO insights_metadata VALUES('schema_version','1')")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query("INSERT INTO insights_schema_migrations VALUES(1,'2026-01-01T00:00:00Z')")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query("INSERT INTO capabilities VALUES('cap_legacy','Editor','Legacy tool','hash')")
            .execute(&legacy)
            .await
            .unwrap();
        legacy.close().await;

        let migrated = open_insights_database(&path).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT description FROM capabilities WHERE capability_id='cap_legacy'"
            )
            .fetch_one(&migrated)
            .await
            .unwrap(),
            "Legacy tool"
        );
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT value FROM insights_metadata WHERE key='schema_version'"
            )
            .fetch_one(&migrated)
            .await
            .unwrap(),
            SCHEMA_VERSION.to_string()
        );
        for table in [
            "artifacts",
            "artifact_versions",
            "artifact_events",
            "artifact_change_jobs",
            "artifact_change_attempts",
            "ask_sessions",
            "ask_messages",
            "ask_questions",
            "ask_jobs",
            "ask_attempts",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1"
                )
                .bind(table)
                .fetch_one(&migrated)
                .await
                .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn schema_two_backfills_observation_admission_time_from_explorer_receipt() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("schema-two.sqlite");
        let options = SqliteConnectOptions::from_str(path.to_string_lossy().as_ref())
            .unwrap()
            .create_if_missing(true);
        let legacy = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE insights_metadata(key TEXT PRIMARY KEY,value TEXT NOT NULL);
             CREATE TABLE insights_schema_migrations(version INTEGER PRIMARY KEY,applied_at TEXT NOT NULL);
             CREATE TABLE observations(
               sequence INTEGER PRIMARY KEY AUTOINCREMENT,observation_id TEXT NOT NULL UNIQUE,
               source_key TEXT NOT NULL UNIQUE,occurred_at TEXT NOT NULL,statement TEXT NOT NULL,
               certainty TEXT NOT NULL,evidence_ids_json TEXT NOT NULL,immutable_fingerprint TEXT NOT NULL);
             CREATE TABLE explorer_jobs(
               job_id TEXT PRIMARY KEY,batch_id TEXT NOT NULL UNIQUE,input_fingerprint TEXT NOT NULL UNIQUE,
               status TEXT NOT NULL,input_json TEXT NOT NULL,prompt_hash TEXT NOT NULL,schema_hash TEXT NOT NULL,
               model TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,error_code TEXT,
               created_at TEXT NOT NULL,updated_at TEXT NOT NULL,accepted_at TEXT);",
        )
        .execute(&legacy)
        .await
        .unwrap();
        sqlx::query("INSERT INTO insights_metadata VALUES('schema_version','2')")
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::query("INSERT INTO insights_schema_migrations VALUES(1,'2026-01-01T00:00:00Z'),(2,'2026-01-02T00:00:00Z')")
            .execute(&legacy).await.unwrap();
        sqlx::query(
            "INSERT INTO explorer_jobs VALUES(
              'job','capture-f1','fingerprint','accepted','{}','prompt','schema','mock',1,NULL,
              '2026-01-03T09:00:00Z','2026-01-03T09:05:00Z','2026-01-03T09:05:00Z')",
        )
        .execute(&legacy)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO observations VALUES(
              1,'observation','capture-f1:obs-1','2025-12-01T08:00:00Z','work happened',
              'explicit','[]','immutable')",
        )
        .execute(&legacy)
        .await
        .unwrap();
        legacy.close().await;

        let migrated = open_insights_database(&path).await.unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, String>(
                "SELECT admitted_at FROM observations WHERE observation_id='observation'"
            )
            .fetch_one(&migrated)
            .await
            .unwrap(),
            "2026-01-03T09:05:00Z"
        );
        let stats = pending_observation_stats(&migrated).await.unwrap();
        assert_eq!(stats.count, 1);
        assert_eq!(stats.episode_groups, 1);
        assert_eq!(
            stats.oldest_admitted_at.as_deref(),
            Some("2026-01-03T09:05:00Z")
        );
    }

    #[tokio::test]
    async fn disposition_promotes_the_next_eligible_card_and_later_version_withdraws() {
        let (_directory, pool) = setup().await;
        let mut opportunity_ids = Vec::new();
        let claims = [
            "A prompt can triage the overflowing inbox.",
            "A prompt can normalize spreadsheet headings.",
            "A prompt can draft calendar follow-ups.",
        ];
        for index in 1..=3 {
            insert_source(&pool, index).await;
            let job_id = job(&pool, index).await;
            claim_job(&pool, &job_id).await.unwrap();
            let mut proposal = delta(index, None, true);
            proposal.signature = format!("prepare-report-{index}");
            proposal.summary = format!("Prepare report {index}");
            proposal.finding.as_mut().unwrap().claim = claims[index - 1].into();
            apply_reconciliation(
                &pool,
                &job_id,
                &ReconciliationOutput {
                    schema_version: 1,
                    considered_observation_ids: vec![format!("obl_{index:024x}")],
                    opportunities: vec![proposal],
                    candidate_assessments: vec![],
                },
                ApplyOptions::default(),
            )
            .await
            .unwrap();
            opportunity_ids.push(
                sqlx::query_scalar::<_, String>(
                    "SELECT opportunity_id FROM opportunities WHERE signature=?1",
                )
                .bind(format!("prepare-report-{index}"))
                .fetch_one(&pool)
                .await
                .unwrap(),
            );
        }
        let first = worth_fixing_summary(&pool, true).await.unwrap();
        assert_eq!(first.selected.len(), 2);
        assert_eq!(
            other_findings(&pool, None, 10).await.unwrap().items.len(),
            1
        );
        let removed = first.selected[0].finding_id.clone();
        record_disposition(&pool, &removed, DispositionKind::NotAProblem, None, None)
            .await
            .unwrap();
        let promoted = worth_fixing_summary(&pool, true).await.unwrap();
        assert_eq!(promoted.selected.len(), 2);
        assert!(!promoted
            .selected
            .iter()
            .any(|card| card.finding_id == removed));

        insert_source(&pool, 4).await;
        let withdraw_job = job(&pool, 4).await;
        claim_job(&pool, &withdraw_job).await.unwrap();
        let mut withdrawal = delta(4, Some(opportunity_ids[1].clone()), false);
        withdrawal.occurrences_to_add.clear();
        withdrawal.withdraw_current_finding = true;
        apply_reconciliation(
            &pool,
            &withdraw_job,
            &ReconciliationOutput {
                schema_version: 1,
                considered_observation_ids: vec![format!("obl_{:024x}", 4)],
                opportunities: vec![withdrawal],
                candidate_assessments: vec![],
            },
            ApplyOptions::default(),
        )
        .await
        .unwrap();
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM findings WHERE opportunity_id=?1 AND active=1",
        )
        .bind(&opportunity_ids[1])
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(active, 0);
    }

    #[tokio::test]
    async fn projections_rebuild_deterministically_from_durable_versions() {
        let (_directory, pool) = setup().await;
        insert_source(&pool, 1).await;
        let job_id = job(&pool, 1).await;
        claim_job(&pool, &job_id).await.unwrap();
        apply_reconciliation(
            &pool,
            &job_id,
            &ReconciliationOutput {
                schema_version: 1,
                considered_observation_ids: vec![format!("obl_{:024x}", 1)],
                opportunities: vec![delta(1, None, true)],
                candidate_assessments: vec![],
            },
            ApplyOptions::default(),
        )
        .await
        .unwrap();
        let expected = projection_fingerprint(&pool).await.unwrap();
        sqlx::query("UPDATE opportunities SET current_status='retired',current_summary='corrupt'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("UPDATE findings SET active=0")
            .execute(&pool)
            .await
            .unwrap();
        rebuild_projections(&pool).await.unwrap();
        assert_eq!(projection_fingerprint(&pool).await.unwrap(), expected);
    }

    #[tokio::test]
    async fn wake_starts_are_durable_without_a_product_daily_cap() {
        let (_directory, pool) = setup().await;
        for index in 0..6 {
            record_wake_start(&pool, "2026-01-01", &format!("threshold-{index}"), true)
                .await
                .unwrap();
        }
        assert_eq!(normal_wakes_started(&pool, "2026-01-01").await.unwrap(), 6);
        assert!(wake_reason_started(&pool, "2026-01-01", "threshold-5")
            .await
            .unwrap());
        record_wake_start(&pool, "2026-01-01", "recovery", false)
            .await
            .unwrap();
        assert_eq!(normal_wakes_started(&pool, "2026-01-01").await.unwrap(), 6);
    }

    #[tokio::test]
    async fn compaction_checkpoint_and_diagnostic_preference_are_durable() {
        let (directory, pool) = setup().await;
        let mut state = crate::CompactionState::default();
        crate::compact_activity_incremental(
            &[crate::SourceActivity {
                evidence_id: "device:test:frame:41".into(),
                occurred_at: "2026-01-01T09:00:00Z".parse().unwrap(),
                app: Some("Editor".into()),
                window: Some("Report".into()),
                url: None,
                text: "weekly metrics ready".into(),
                content_hash: None,
            }],
            crate::CompactionConfig::default(),
            &mut state,
        );
        commit_compaction_checkpoint(&pool, &state, 41, 72)
            .await
            .unwrap();
        set_enhanced_diagnostics(&pool, true).await.unwrap();
        pool.close().await;

        let reopened = open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();
        assert_eq!(capture_cursor(&reopened, "frames").await.unwrap(), 41);
        assert_eq!(capture_cursor(&reopened, "events").await.unwrap(), 72);
        assert_eq!(
            serde_json::to_value(load_compaction_state(&reopened).await.unwrap()).unwrap(),
            serde_json::to_value(state).unwrap()
        );
        assert!(enhanced_diagnostics_enabled(&reopened).await.unwrap());
    }
}
