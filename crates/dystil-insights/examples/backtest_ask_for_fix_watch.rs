//! Production-shaped Ask-for-fix Watch replay against a captured fixture.
//!
//! The capture database must contain only activity visible when the initial
//! Ask-for-fix question is submitted. `--source-insights-db` can contain later
//! Explorer observations; each `--advance-to` makes one more local day visible
//! to the Watch collector.
//!
//! Example:
//! cargo run -p dystil-insights --example backtest_ask_for_fix_watch -- \
//!   --capture-db /tmp/deepika-day-one.sqlite \
//!   --source-insights-db .local/.../worth-fixing.sqlite \
//!   --insights-db /tmp/ask-watch.sqlite \
//!   --codex /path/to/codex --mcp /path/to/dystil-mcp \
//!   --advance-to 2026-08-07 --advance-to 2026-08-08

use std::{path::PathBuf, str::FromStr};

use async_trait::async_trait;
use clap::Parser;
use dystil_ai::{
    AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiModelTier, AiRuntime,
    AiRuntimeDescriptor, AiRuntimeError, AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind,
    AiStructuredRequest, AiStructuredRun, CliProvider, McpServerConfig, ProviderKind,
    TeammateAnswerRun,
};
use dystil_insights::{
    admit_observation, collect_ask_for_fix_watches, confirm_ask_for_fix,
    create_ask_for_fix_session, open_insights_database, review_ask_for_fix_watch,
    start_ask_for_fix_watch, submit_ask_for_fix_turn, upsert_evidence, AskInputEvent, AskPhase,
    AskQuestionKind, AskSessionView, AskUserTurn, EvidenceRecord, ObservationRecord,
};
use serde_json::json;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row, SqlitePool,
};

const ECONOMY_MODEL: &str = "gpt-5.6-luna";
const FRONTIER_MODEL: &str = "gpt-5.6-sol";

#[derive(Debug, Parser)]
struct Args {
    /// Capture database snapshot visible when the initial question is asked.
    #[arg(long)]
    capture_db: PathBuf,
    /// Remove capture at and after this RFC3339 time before asking the initial
    /// question. Pass a disposable database copy: this intentionally mutates it.
    #[arg(long)]
    initial_end_exclusive: Option<String>,
    /// Existing Explorer output containing observations that will arrive later.
    #[arg(long)]
    source_insights_db: PathBuf,
    /// Explorer observations already available when the initial question is
    /// asked. They establish the watch baseline and are never re-collected.
    #[arg(long)]
    baseline_through: String,
    /// Fresh durable database for the Ask-for-fix and Watch replay.
    #[arg(long)]
    insights_db: PathBuf,
    #[arg(long)]
    codex: PathBuf,
    #[arg(long)]
    mcp: PathBuf,
    #[arg(long)]
    codex_home: Option<PathBuf>,
    /// Local dates made visible to Watch, one batch at a time.
    #[arg(long, required = true)]
    advance_to: Vec<String>,
}

struct LocalRuntime {
    provider: CliProvider,
    descriptor: AiRuntimeDescriptor,
}

#[async_trait]
impl AiRuntime for LocalRuntime {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }

    fn model_for_tier(&self, tier: AiModelTier) -> String {
        match tier {
            AiModelTier::Economy => ECONOMY_MODEL.into(),
            AiModelTier::Frontier => FRONTIER_MODEL.into(),
        }
    }

    async fn answer(&self, _request: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "answer is outside this validation harness",
        ))
    }

    async fn run_automation(
        &self,
        _request: AiAutomationRequest,
        _events: tokio::sync::mpsc::Sender<AiRuntimeEvent>,
    ) -> Result<AiAutomationRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "automation is outside this validation harness",
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

fn answer_active_question(session: &AskSessionView) -> AskUserTurn {
    let question = session
        .current_question
        .as_ref()
        .expect("follow-up phase must expose an active question");
    let question_id = session.current_question_id.clone();
    let event = match question.kind {
        AskQuestionKind::FreeText => AskInputEvent {
            kind: "free_text".into(),
            question_id,
            selected_option_ids: vec![],
        },
        AskQuestionKind::SingleSelect | AskQuestionKind::Compare => AskInputEvent {
            kind: if question.kind == AskQuestionKind::Compare {
                "compare"
            } else {
                "single_select"
            }
            .into(),
            question_id,
            selected_option_ids: vec![question.options[0].id.clone()],
        },
        AskQuestionKind::MultiSelect => AskInputEvent {
            kind: "multi_select".into(),
            question_id,
            selected_option_ids: question
                .options
                .iter()
                .take(question.min_selections.max(1) as usize)
                .map(|option| option.id.clone())
                .collect(),
        },
    };
    AskUserTurn {
        text: "I need observed proof that the RFQ was prepared, checked, and reached Gmail's Sent state. Please wait for a complete example instead of inferring one from partial activity.".into(),
        event,
    }
}

async fn source_pool(path: &PathBuf) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?
        .read_only(true)
        .foreign_keys(true);
    Ok(SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?)
}

async fn trim_initial_capture(
    path: &PathBuf,
    end_exclusive: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let options =
        SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))?.foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    // Delete the indexed projection first so retrieval cannot see activity that
    // has not yet happened in this replay. Its triggers keep FTS in sync.
    sqlx::query("DELETE FROM activity_search_documents WHERE timestamp >=?1")
        .bind(end_exclusive)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM ui_events WHERE timestamp >=?1")
        .bind(end_exclusive)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM frames WHERE timestamp >=?1")
        .bind(end_exclusive)
        .execute(&pool)
        .await?;
    Ok(())
}

async fn import_through(
    source: &SqlitePool,
    target: &SqlitePool,
    through_date: &str,
) -> Result<u64, Box<dyn std::error::Error>> {
    let through = format!("{through_date}T23:59:59.999999999Z");
    let rows = sqlx::query(
        "SELECT observation_id,source_key,occurred_at,statement,certainty,evidence_ids_json
         FROM observations WHERE occurred_at<=?1 ORDER BY sequence",
    )
    .bind(through)
    .fetch_all(source)
    .await?;
    let mut imported = 0;
    for row in rows {
        let observation_id: String = row.get("observation_id");
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM observations WHERE observation_id=?1")
                .bind(&observation_id)
                .fetch_one(target)
                .await?;
        if exists > 0 {
            continue;
        }
        let evidence_ids: Vec<String> = serde_json::from_str(row.get("evidence_ids_json"))?;
        for evidence_id in &evidence_ids {
            let evidence = sqlx::query(
                "SELECT evidence_id,source_namespace,source_id,occurred_at,app,window,excerpt,
                        policy_allowed,redaction_ready,deleted,sensitive
                 FROM evidence WHERE evidence_id=?1",
            )
            .bind(evidence_id)
            .fetch_optional(source)
            .await?;
            let Some(evidence) = evidence else { continue };
            upsert_evidence(
                target,
                &EvidenceRecord {
                    evidence_id: evidence.get("evidence_id"),
                    source_namespace: evidence.get("source_namespace"),
                    source_id: evidence.get("source_id"),
                    occurred_at: evidence.get("occurred_at"),
                    app: evidence.get("app"),
                    window: evidence.get("window"),
                    url: None,
                    excerpt: evidence.get("excerpt"),
                    policy_allowed: evidence.get::<i64, _>("policy_allowed") == 1,
                    redaction_ready: evidence.get::<i64, _>("redaction_ready") == 1,
                    deleted: evidence.get::<i64, _>("deleted") == 1,
                    sensitive: evidence.get::<i64, _>("sensitive") == 1,
                },
            )
            .await?;
        }
        let certainty: String = row.get("certainty");
        admit_observation(
            target,
            &ObservationRecord {
                observation_id,
                source_key: row.get("source_key"),
                occurred_at: row.get("occurred_at"),
                statement: row.get("statement"),
                certainty: serde_json::from_str(&format!("\"{certainty}\""))?,
                evidence_ids,
            },
        )
        .await?;
        imported += 1;
    }
    Ok(imported)
}

async fn watch_cursor(pool: &SqlitePool, session_id: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT last_evaluated_sequence FROM ask_watches WHERE session_id=?1")
        .bind(session_id)
        .fetch_one(pool)
        .await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.capture_db.is_absolute() || !args.source_insights_db.is_absolute() {
        return Err("capture and source-insights paths must be absolute".into());
    }
    if let Some(end_exclusive) = &args.initial_end_exclusive {
        trim_initial_capture(&args.capture_db, end_exclusive).await?;
    }
    let environment = args
        .codex_home
        .as_ref()
        .map(|path| vec![("CODEX_HOME".into(), path.to_string_lossy().into_owned())])
        .unwrap_or_default();
    let runtime = LocalRuntime {
        provider: CliProvider {
            provider: ProviderKind::Codex,
            executable: args.codex,
            runtime_version: Some("fixture-watch-backtest".into()),
            environment,
            mcp_server: Some(McpServerConfig {
                command: args.mcp,
                args: vec![
                    "--database".into(),
                    args.capture_db.to_string_lossy().into_owned(),
                    "--max-calls".into(),
                    "30".into(),
                ],
            }),
        },
        descriptor: AiRuntimeDescriptor {
            kind: AiRuntimeKind::Codex,
            provider_label: "Fixture Codex".into(),
            model: FRONTIER_MODEL.into(),
        },
    };
    let pool = open_insights_database(&args.insights_db).await?;
    let source = source_pool(&args.source_insights_db).await?;
    let baseline_observations = import_through(&source, &pool, &args.baseline_through).await?;

    let created = create_ask_for_fix_session(&pool).await?;
    let mut session = submit_ask_for_fix_turn(
        &pool,
        &runtime,
        &created.session_id,
        AskUserTurn {
            text: "I want a reliable way to send supplier RFQs, but only when Dystil has observed an end-to-end example: prepare the RFQ, check the recipient and attachment, then verify it reached Gmail's Sent state. If that evidence is not available yet, do not invent an answer.".into(),
            event: AskInputEvent {
                kind: "initial_problem".into(),
                question_id: None,
                selected_option_ids: vec![],
            },
        },
    )
    .await?;
    while session.phase == AskPhase::FollowUp {
        session = submit_ask_for_fix_turn(
            &pool,
            &runtime,
            &session.session_id,
            answer_active_question(&session),
        )
        .await?;
    }
    if session.phase != AskPhase::Consolidate {
        return Err(format!(
            "expected an evidence-led consolidation, got {:?}",
            session.phase
        )
        .into());
    }
    session = confirm_ask_for_fix(&pool, &runtime, &session.session_id).await?;
    let initial_route = session.presentation.as_ref().map(|value| value.route);
    if session.watch.is_some() {
        return Err("a watch must require the explicit Keep watching action".into());
    }
    session = start_ask_for_fix_watch(&pool, &session.session_id).await?;

    let mut timeline = Vec::new();
    for date in &args.advance_to {
        let imported = import_through(&source, &pool, date).await?;
        let mut collection_cycles = 0;
        let mut evaluated_watches = 0;
        let mut review_ready_watches = 0;
        loop {
            let before = watch_cursor(&pool, &session.session_id).await?;
            let collection = collect_ask_for_fix_watches(&pool, &runtime).await?;
            let after = watch_cursor(&pool, &session.session_id).await?;
            collection_cycles += 1;
            evaluated_watches += collection.evaluated_watches;
            review_ready_watches += collection.review_ready_watches;
            session = dystil_insights::get_ask_for_fix_session(&pool, &session.session_id).await?;
            if session.watch.as_ref().is_some_and(|watch| {
                matches!(watch.state, dystil_insights::AskWatchState::ReviewReady)
            }) || after == before
            {
                break;
            }
        }
        timeline.push(json!({
            "date": date,
            "newObservations": imported,
            "collectionCycles": collection_cycles,
            "evaluatedWatches": evaluated_watches,
            "reviewReadyWatches": review_ready_watches,
            "watchState": session.watch.as_ref().map(|watch| watch.state),
            "supportingEvidence": session.watch.as_ref().map(|watch| watch.supporting_evidence_count),
        }));
        if session
            .watch
            .as_ref()
            .is_some_and(|watch| matches!(watch.state, dystil_insights::AskWatchState::ReviewReady))
        {
            session = review_ask_for_fix_watch(&pool, &runtime, &session.session_id).await?;
            timeline.push(json!({
                "date": date,
                "afterFrontierReview": true,
                "phase": session.phase,
                "watchState": session.watch.as_ref().map(|watch| watch.state),
                "missingEvidence": session.watch.as_ref().map(|watch| &watch.spec.missing_evidence),
            }));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
        "sessionId": session.session_id,
        "baselineObservations": baseline_observations,
            "initialRoute": initial_route,
            "initialPresentation": session.presentation.as_ref().map(|presentation| presentation.headline.clone()),
            "timeline": timeline,
            "finalPhase": session.phase,
            "finalWatch": session.watch,
        }))?
    );
    Ok(())
}
