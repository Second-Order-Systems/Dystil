//! Replays captured local activity through the Worth Fixing engine without
//! starting the desktop application or its periodic scheduler.
//!
//! The capture database is opened read-only. Progress is durable in the
//! insights database, so rerunning this command resumes from its cursors.

use std::{path::PathBuf, str::FromStr};

use async_trait::async_trait;
use chrono::{DateTime, Days, FixedOffset, NaiveDate, TimeZone, Utc};
use clap::Parser;
use dystil_ai::{
    AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiModelTier, AiReasoningEffort,
    AiRuntime, AiRuntimeDescriptor, AiRuntimeError, AiRuntimeErrorCode, AiRuntimeEvent,
    AiRuntimeKind, AiStructuredRequest, AiStructuredRun, CliProvider, ProviderKind,
    TeammateAnswerRun,
};
use dystil_insights::{
    capture_cursor, commit_compaction_checkpoint, compact_activity_incremental,
    copy_observations_for_steward_replay, load_compaction_state, open_insights_database,
    pending_observation_stats, release_bulk_backfill_job_observations, resolve_capture_evidence,
    run_explorer_batch_with_compaction, run_steward_replay_wake,
    run_steward_replay_wake_with_reasoning, run_steward_wake, CaptureAdmissionRules,
    CompactionConfig, ExplorerRunResult, SourceActivity,
};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};

const SOURCE_NAMESPACE: &str = "local-capture";
const LOOK_AHEAD_PER_SOURCE: i64 = 200;
const MERGED_BATCH_LIMIT: usize = 200;

#[derive(Debug, Parser)]
#[command(
    name = "worth-fixing-backfill",
    about = "Replay a captured Dystil SQLite database through the Worth Fixing engine"
)]
struct Args {
    /// Dystil capture database. It is always opened read-only.
    #[arg(
        long,
        required_unless_present = "steward_replay_source",
        conflicts_with = "steward_replay_source"
    )]
    capture_db: Option<PathBuf>,

    /// Durable Worth Fixing output database. This is created or resumed.
    #[arg(long)]
    insights_db: PathBuf,

    /// Managed Codex executable used for the existing Economy/Frontier calls.
    #[arg(long)]
    codex: PathBuf,

    /// Optional managed Codex state directory. Passed as CODEX_HOME only to this process.
    #[arg(long)]
    codex_home: Option<PathBuf>,

    /// Timezone offset used by the existing Worth Fixing prompts and Steward day boundary.
    #[arg(long, default_value = "+00:00")]
    timezone: String,

    /// First local calendar date to replay, inclusive (YYYY-MM-DD).
    #[arg(long)]
    from_date: Option<NaiveDate>,

    /// Last local calendar date to replay, inclusive (YYYY-MM-DD).
    #[arg(long)]
    through_date: Option<NaiveDate>,

    /// Stop after this many durable Explorer batches. Omit when using --all.
    #[arg(long, conflicts_with = "all")]
    max_batches: Option<usize>,

    /// Drain all available captured activity before waking the Steward.
    #[arg(long, conflicts_with = "max_batches")]
    all: bool,

    /// Reconcile existing Explorer observations only. This never reads capture
    /// rows or invokes Explorer, and releases observations reserved by prior
    /// rejected Steward jobs before regrouping them.
    #[arg(long)]
    steward_only: bool,

    /// Existing insights database containing accepted Explorer observations.
    /// Evidence and observations are copied into the fresh destination, then
    /// only Steward is run. The source is opened read-only.
    #[arg(long, conflicts_with_all = ["capture_db", "steward_only", "all", "max_batches", "from_date", "through_date"])]
    steward_replay_source: Option<PathBuf>,

    /// Maximum observations in each Steward-only reconciliation packet.
    #[arg(long, default_value_t = 40)]
    steward_observation_limit: u32,

    /// Stop a Steward replay after this many reconciliation packets.
    #[arg(long)]
    steward_replay_batches: Option<usize>,

    /// Frontier-tier model used by Steward during this developer backfill.
    #[arg(long, default_value = "gpt-5.6-sol")]
    steward_model: String,

    /// Provider reasoning effort for Steward-only replay: low, medium, or high.
    #[arg(long, default_value = "medium", value_parser = parse_reasoning_effort)]
    steward_reasoning_effort: String,
}

fn parse_reasoning_effort(value: &str) -> Result<String, String> {
    match value {
        "low" | "medium" | "high" => Ok(value.to_owned()),
        _ => Err("must be one of: low, medium, high".into()),
    }
}

fn reasoning_effort(value: &str) -> AiReasoningEffort {
    match value {
        "low" => AiReasoningEffort::Low,
        "high" => AiReasoningEffort::High,
        _ => AiReasoningEffort::Medium,
    }
}

#[derive(Clone)]
struct TimeBounds {
    start: Option<String>,
    end: Option<String>,
}

impl TimeBounds {
    fn from_args(args: &Args, offset: FixedOffset) -> Result<Self, Box<dyn std::error::Error>> {
        let start = args
            .from_date
            .map(|date| {
                offset
                    .from_local_datetime(&date.and_hms_opt(0, 0, 0).expect("midnight is valid"))
                    .single()
                    .ok_or("the local start date is ambiguous")
                    .map(|value| value.with_timezone(&Utc).to_rfc3339())
            })
            .transpose()?;
        let end = args
            .through_date
            .map(|date| {
                let next_day = date
                    .checked_add_days(Days::new(1))
                    .ok_or("the through date is out of range")?;
                offset
                    .from_local_datetime(&next_day.and_hms_opt(0, 0, 0).expect("midnight is valid"))
                    .single()
                    .ok_or("the local through date is ambiguous")
                    .map(|value| value.with_timezone(&Utc).to_rfc3339())
            })
            .transpose()?;
        if let (Some(start), Some(end)) = (&start, &end) {
            if start >= end {
                return Err("--through-date must not be before --from-date".into());
            }
        }
        Ok(Self { start, end })
    }
}

struct CodexRuntime {
    descriptor: AiRuntimeDescriptor,
    provider: CliProvider,
    steward_model: String,
}

#[async_trait]
impl AiRuntime for CodexRuntime {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }

    fn model_for_tier(&self, tier: AiModelTier) -> String {
        match tier {
            AiModelTier::Economy => "gpt-5.6-luna".into(),
            AiModelTier::Frontier => self.steward_model.clone(),
        }
    }

    async fn answer(&self, _request: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "the Worth Fixing backfill runtime only supports structured inference",
        ))
    }

    async fn run_automation(
        &self,
        _request: AiAutomationRequest,
        _events: tokio::sync::mpsc::Sender<AiRuntimeEvent>,
    ) -> Result<AiAutomationRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "the Worth Fixing backfill runtime only supports structured inference",
        ))
    }

    async fn infer_structured(
        &self,
        request: AiStructuredRequest,
    ) -> Result<AiStructuredRun, AiRuntimeError> {
        let model = self.model_for_tier(request.model_tier);
        self.provider
            .run_structured_with_model(request, Some(&model))
            .await
            .map_err(Into::into)
    }
}

async fn open_capture_database(path: &PathBuf) -> Result<SqlitePool, sqlx::Error> {
    SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            SqliteConnectOptions::from_str(path.to_string_lossy().as_ref())?.read_only(true),
        )
        .await
}

async fn next_source_records(
    insights: &SqlitePool,
    capture: &SqlitePool,
    bounds: &TimeBounds,
) -> Result<(Vec<dystil_insights::EvidenceRecord>, i64, i64), Box<dyn std::error::Error>> {
    let frame_cursor = capture_cursor(insights, "frames").await?;
    let event_cursor = capture_cursor(insights, "events").await?;
    let rules = CaptureAdmissionRules {
        excluded_apps: Vec::new(),
        excluded_windows: Vec::new(),
        excluded_urls: Vec::new(),
        ignore_private_windows: false,
    };
    let frames = sqlx::query(
        "SELECT id FROM frames
         WHERE id>?1 AND (?2 IS NULL OR timestamp>=?2) AND (?3 IS NULL OR timestamp<?3)
         ORDER BY id LIMIT ?4",
    )
    .bind(frame_cursor)
    .bind(&bounds.start)
    .bind(&bounds.end)
    .bind(LOOK_AHEAD_PER_SOURCE)
    .fetch_all(capture)
    .await?;
    let mut candidates = Vec::new();
    for row in frames {
        let id = row.get::<i64, _>("id");
        let record =
            resolve_capture_evidence(capture, SOURCE_NAMESPACE, &format!("frame:{id}"), &rules)
                .await?;
        if let Some(record) = record {
            candidates.push(("frames", id, record));
        }
    }

    let events = sqlx::query(
        "SELECT id FROM ui_events
         WHERE id>?1 AND (?2 IS NULL OR timestamp>=?2) AND (?3 IS NULL OR timestamp<?3)
         ORDER BY id LIMIT ?4",
    )
    .bind(event_cursor)
    .bind(&bounds.start)
    .bind(&bounds.end)
    .bind(LOOK_AHEAD_PER_SOURCE)
    .fetch_all(capture)
    .await?;
    for row in events {
        let id = row.get::<i64, _>("id");
        let record =
            resolve_capture_evidence(capture, SOURCE_NAMESPACE, &format!("event:{id}"), &rules)
                .await?;
        if let Some(record) = record {
            candidates.push(("events", id, record));
        }
    }
    candidates.sort_by(|a, b| {
        a.2.occurred_at
            .cmp(&b.2.occurred_at)
            .then(a.2.evidence_id.cmp(&b.2.evidence_id))
    });
    candidates.truncate(MERGED_BATCH_LIMIT);
    let last_frame = candidates
        .iter()
        .filter(|(source, _, _)| *source == "frames")
        .map(|(_, id, _)| *id)
        .max()
        .unwrap_or(frame_cursor);
    let last_event = candidates
        .iter()
        .filter(|(source, _, _)| *source == "events")
        .map(|(_, id, _)| *id)
        .max()
        .unwrap_or(event_cursor);
    let records = candidates
        .into_iter()
        .map(|(_, _, record)| record)
        .filter(|record| record.policy_allowed && !record.sensitive)
        .collect();
    Ok((records, last_frame, last_event))
}

async fn remaining_source_records(
    capture: &SqlitePool,
    table: &str,
    cursor: i64,
    bounds: &TimeBounds,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!(
        "SELECT COUNT(*) FROM {table}
         WHERE id>?1 AND (?2 IS NULL OR timestamp>=?2) AND (?3 IS NULL OR timestamp<?3)"
    ))
    .bind(cursor)
    .bind(&bounds.start)
    .bind(&bounds.end)
    .fetch_one(capture)
    .await
}

async fn count(pool: &SqlitePool, table: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(pool)
        .await
}

async fn usage_totals_by_stage(
    pool: &SqlitePool,
) -> Result<Vec<(String, String, String, i64, i64, i64, i64, i64)>, sqlx::Error> {
    sqlx::query_as(
        "SELECT stage,model,status,
           COALESCE(SUM(CAST(json_extract(usage_json, '$.input_tokens') AS INTEGER)), 0),
           COALESCE(SUM(CAST(json_extract(usage_json, '$.cached_input_tokens') AS INTEGER)), 0),
           COALESCE(SUM(CAST(json_extract(usage_json, '$.output_tokens') AS INTEGER)), 0),
           COALESCE(SUM(CAST(json_extract(usage_json, '$.reasoning_output_tokens') AS INTEGER)), 0),
           COALESCE(SUM(latency_ms), 0)
         FROM (
           SELECT 'explorer' AS stage,ej.model,ea.status,ea.usage_json,ea.latency_ms
           FROM explorer_attempts ea JOIN explorer_jobs ej ON ej.job_id=ea.job_id
           UNION ALL
           SELECT 'steward' AS stage,ij.model,ja.status,ja.usage_json,ja.latency_ms
           FROM job_attempts ja JOIN inference_jobs ij ON ij.job_id=ja.job_id
         )
         GROUP BY stage,model,status ORDER BY stage,model,status",
    )
    .fetch_all(pool)
    .await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.steward_replay_source.is_none()
        && !args.steward_only
        && !args.all
        && args.max_batches.is_none()
    {
        return Err("choose --all or a bounded --max-batches value".into());
    }
    let timezone = args.timezone.parse::<FixedOffset>()?;
    let insights = open_insights_database(&args.insights_db).await?;
    let codex_executable = std::fs::canonicalize(&args.codex)?;
    let provider = CliProvider {
        provider: ProviderKind::Codex,
        executable: codex_executable,
        runtime_version: None,
        environment: args
            .codex_home
            .clone()
            .map(std::fs::canonicalize)
            .transpose()?
            .map(|path| vec![("CODEX_HOME".into(), path.to_string_lossy().into_owned())])
            .unwrap_or_default(),
        mcp_server: None,
    };
    let runtime = CodexRuntime {
        descriptor: AiRuntimeDescriptor {
            kind: AiRuntimeKind::Codex,
            provider_label: "codex".into(),
            model: args.steward_model.clone(),
        },
        provider,
        steward_model: args.steward_model.clone(),
    };
    if let Some(source_path) = &args.steward_replay_source {
        let source = open_capture_database(source_path).await?;
        let source_identity = std::fs::canonicalize(source_path)?
            .to_string_lossy()
            .into_owned();
        let copied =
            copy_observations_for_steward_replay(&source, &insights, &source_identity).await?;
        eprintln!("steward replay source: copied {copied} observations");
        let local_day = Utc::now().with_timezone(&timezone).date_naive().to_string();
        let replay_nonce = format!("steward-replay:{source_identity}");
        let mut completed = 0_usize;
        loop {
            if args
                .steward_replay_batches
                .is_some_and(|limit| completed >= limit)
            {
                break;
            }
            let pending = pending_observation_stats(&insights).await?;
            if pending.count == 0 {
                break;
            }
            let result = run_steward_replay_wake_with_reasoning(
                &insights,
                &runtime,
                &local_day,
                &args.timezone,
                "steward_replay",
                args.steward_observation_limit,
                &replay_nonce,
                reasoning_effort(&args.steward_reasoning_effort),
            )
            .await?;
            completed += 1;
            eprintln!(
                "steward replay batch {completed}: pending_observations={} result={result:?}",
                pending.count
            );
        }
        println!(
            "steward replay complete: batches={completed}, observations={}, occurrences={}, opportunities={}, findings={}",
            count(&insights, "observations").await?,
            count(&insights, "occurrences").await?,
            count(&insights, "opportunities").await?,
            count(&insights, "findings").await?,
        );
        for (stage, model, status, input, cached, output, reasoning, latency) in
            usage_totals_by_stage(&insights).await?
        {
            println!(
                "{stage} model={model} attempts status={status}: input_tokens={input}, cached_input_tokens={cached}, output_tokens={output}, reasoning_output_tokens={reasoning}, latency_ms={latency}"
            );
        }
        return Ok(());
    }
    let capture_path = args
        .capture_db
        .as_ref()
        .expect("capture_db is required outside Steward replay")
        .clone();
    let bounds = TimeBounds::from_args(&args, timezone)?;
    let capture = open_capture_database(&capture_path).await?;
    if args.steward_only {
        let released = release_bulk_backfill_job_observations(&insights).await?;
        eprintln!("steward-only replay: released {released} observations from bulk backfill jobs");
        let local_day = Utc::now().with_timezone(&timezone).date_naive().to_string();
        let replay_nonce = uuid::Uuid::new_v4().to_string();
        let mut completed = 0_usize;
        loop {
            let pending = pending_observation_stats(&insights).await?;
            if pending.count == 0 {
                break;
            }
            let result = run_steward_replay_wake(
                &insights,
                &runtime,
                &local_day,
                &args.timezone,
                "fixture_backfill_steward_only",
                args.steward_observation_limit,
                &replay_nonce,
            )
            .await?;
            completed += 1;
            eprintln!(
                "steward batch {completed}: pending_observations={} result={result:?}",
                pending.count
            );
        }
        println!(
            "steward-only replay complete: batches={completed}, opportunities={}, findings={}",
            count(&insights, "opportunities").await?,
            count(&insights, "findings").await?
        );
        return Ok(());
    }
    let limit = args.max_batches.unwrap_or(usize::MAX);
    let mut completed = 0_usize;
    let mut accepted = 0_usize;
    let mut skipped = 0_usize;
    let initial_frame_cursor = capture_cursor(&insights, "frames").await?;
    let initial_event_cursor = capture_cursor(&insights, "events").await?;
    let total_frames =
        remaining_source_records(&capture, "frames", initial_frame_cursor, &bounds).await?;
    let total_events =
        remaining_source_records(&capture, "ui_events", initial_event_cursor, &bounds).await?;
    eprintln!(
        "backfill scope: frames={total_frames}, events={total_events}, local dates={}..{}",
        args.from_date
            .map(|date| date.to_string())
            .as_deref()
            .unwrap_or("beginning"),
        args.through_date
            .map(|date| date.to_string())
            .as_deref()
            .unwrap_or("end"),
    );

    while completed < limit {
        let prior_frame = capture_cursor(&insights, "frames").await?;
        let prior_event = capture_cursor(&insights, "events").await?;
        let (records, last_frame, last_event) =
            next_source_records(&insights, &capture, &bounds).await?;
        if last_frame == prior_frame && last_event == prior_event {
            break;
        }
        let mut state = load_compaction_state(&insights).await?;
        let source = records
            .iter()
            .filter_map(|record| {
                Some(SourceActivity {
                    evidence_id: record.evidence_id.clone(),
                    occurred_at: DateTime::parse_from_rfc3339(&record.occurred_at)
                        .ok()?
                        .with_timezone(&Utc),
                    app: record.app.clone(),
                    window: record.window.clone(),
                    url: None,
                    text: record.excerpt.clone(),
                    content_hash: None,
                })
            })
            .collect::<Vec<_>>();
        let compact =
            compact_activity_incremental(&source, CompactionConfig::default(), &mut state);
        let batch_id = format!(
            "capture-f{}-{}-e{}-{}",
            prior_frame + 1,
            last_frame,
            prior_event + 1,
            last_event
        );
        let result = if compact.is_empty() {
            ExplorerRunResult::NoAdmissibleEvidence
        } else {
            run_explorer_batch_with_compaction(
                &insights,
                &runtime,
                &batch_id,
                &args.timezone,
                &records,
                &compact,
            )
            .await?
        };
        commit_compaction_checkpoint(&insights, &state, last_frame, last_event).await?;
        completed += 1;
        match result {
            ExplorerRunResult::Accepted { .. } | ExplorerRunResult::AlreadyAccepted { .. } => {
                accepted += 1
            }
            ExplorerRunResult::NoAdmissibleEvidence => skipped += 1,
        }
        eprintln!(
            "batch {completed}: frames {}/{}; events {}/{}; explorer_batches={accepted}, skipped={skipped}",
            total_frames - remaining_source_records(&capture, "frames", last_frame, &bounds).await?,
            total_frames,
            total_events - remaining_source_records(&capture, "ui_events", last_event, &bounds).await?,
            total_events,
        );
    }

    let pending = pending_observation_stats(&insights).await?;
    if pending.count > 0 {
        let offset = args.timezone.parse::<FixedOffset>()?;
        let local_day = Utc::now().with_timezone(&offset).date_naive().to_string();
        let result = run_steward_wake(
            &insights,
            &runtime,
            &local_day,
            &args.timezone,
            "fixture_backfill",
            250,
        )
        .await?;
        eprintln!("steward result: {result:?}");
    }

    println!(
        "backfill complete: batches={completed}, explorer_batches={accepted}, skipped={skipped}, evidence={}, observations={}, occurrences={}, opportunities={}, findings={}",
        count(&insights, "evidence").await?,
        count(&insights, "observations").await?,
        count(&insights, "occurrences").await?,
        count(&insights, "opportunities").await?,
        count(&insights, "findings").await?,
    );
    for (stage, model, status, input, cached, output, reasoning, latency) in
        usage_totals_by_stage(&insights).await?
    {
        println!(
            "{stage} model={model} attempts status={status}: input_tokens={input}, cached_input_tokens={cached}, output_tokens={output}, reasoning_output_tokens={reasoning}, latency_ms={latency}"
        );
    }
    Ok(())
}
