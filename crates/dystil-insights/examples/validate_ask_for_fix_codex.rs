//! Opt-in end-to-end validation of Ask for a fix with a locally authenticated Codex CLI.
//!
//! Run with:
//! DYSTIL_CODEX_EXECUTABLE=/path/to/codex \
//!   cargo run -p dystil-insights --example validate_ask_for_fix_codex

use std::path::PathBuf;

use async_trait::async_trait;
use dystil_ai::{
    AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiModelTier, AiRuntime,
    AiRuntimeDescriptor, AiRuntimeError, AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind,
    AiStructuredRequest, AiStructuredRun, CliProvider, ProviderKind, TeammateAnswerRun,
};
use dystil_insights::{
    confirm_ask_for_fix, create_ask_for_fix_session, submit_ask_for_fix_turn, AskInputEvent,
    AskPhase, AskQuestionKind, AskSessionView, AskUserTurn,
};
use serde_json::json;
use tempfile::tempdir;
use tokio::sync::mpsc;

const FRONTIER_MODEL: &str = "gpt-5.6-sol";

struct LocalCodexRuntime {
    provider: CliProvider,
    descriptor: AiRuntimeDescriptor,
}

#[async_trait]
impl AiRuntime for LocalCodexRuntime {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }

    fn model_for_tier(&self, tier: AiModelTier) -> String {
        assert_eq!(tier, AiModelTier::Frontier);
        FRONTIER_MODEL.into()
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
        _events: mpsc::Sender<AiRuntimeEvent>,
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
        self.provider
            .run_structured_with_model(request, Some(FRONTIER_MODEL))
            .await
            .map_err(AiRuntimeError::from)
    }
}

fn answer_active_question(session: &AskSessionView) -> AskUserTurn {
    let question = session
        .current_question
        .as_ref()
        .expect("follow-up phase must expose its active question");
    let question_id = session.current_question_id.clone();
    match question.kind {
        AskQuestionKind::FreeText => AskUserTurn {
            text: "This happens between a CRM and a spreadsheet. I copy contact details into the sheet, then check them before anything is sent; that final check must remain mine.".into(),
            event: AskInputEvent {
                kind: "free_text".into(),
                question_id,
                selected_option_ids: vec![],
            },
        },
        AskQuestionKind::SingleSelect | AskQuestionKind::Compare => {
            let option = question.options.first().expect("choice question has options");
            AskUserTurn {
                text: option.label.clone(),
                event: AskInputEvent {
                    kind: if question.kind == AskQuestionKind::Compare {
                        "compare"
                    } else {
                        "single_select"
                    }
                    .into(),
                    question_id,
                    selected_option_ids: vec![option.id.clone()],
                },
            }
        }
        AskQuestionKind::MultiSelect => {
            let count = usize::try_from(question.min_selections.max(1)).unwrap();
            let selected = question
                .options
                .iter()
                .take(count)
                .map(|option| option.id.clone())
                .collect::<Vec<_>>();
            AskUserTurn {
                text: "Selected the applicable conditions.".into(),
                event: AskInputEvent {
                    kind: "multi_select".into(),
                    question_id,
                    selected_option_ids: selected,
                },
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let executable = std::env::var_os("DYSTIL_CODEX_EXECUTABLE")
        .map(PathBuf::from)
        .ok_or("set DYSTIL_CODEX_EXECUTABLE to the locally configured Codex binary")?;
    if !executable.is_file() {
        return Err(format!("Codex executable does not exist: {}", executable.display()).into());
    }
    let runtime = LocalCodexRuntime {
        provider: CliProvider {
            provider: ProviderKind::Codex,
            executable,
            runtime_version: Some("local-validation".into()),
            environment: vec![],
            mcp_server: None,
        },
        descriptor: AiRuntimeDescriptor {
            kind: AiRuntimeKind::Codex,
            provider_label: "Locally configured Codex".into(),
            model: FRONTIER_MODEL.into(),
        },
    };
    let directory = tempdir()?;
    let pool = dystil_insights::open_insights_database(directory.path().join("ask.sqlite")).await?;
    let created = create_ask_for_fix_session(&pool).await?;
    let mut session = submit_ask_for_fix_turn(
        &pool,
        &runtime,
        &created.session_id,
        AskUserTurn {
            text: "I keep copying the same customer details between two apps.".into(),
            event: AskInputEvent {
                kind: "initial_problem".into(),
                question_id: None,
                selected_option_ids: vec![],
            },
        },
    )
    .await?;

    while session.phase == AskPhase::FollowUp {
        let turn = answer_active_question(&session);
        session = submit_ask_for_fix_turn(&pool, &runtime, &session.session_id, turn).await?;
    }
    if session.phase != AskPhase::Consolidate {
        return Err(format!("expected consolidation, got {:?}", session.phase).into());
    }
    if session.understanding.inferences.is_empty()
        || session.understanding.grounding.is_empty()
        || session.understanding.preserved_boundary.trim().is_empty()
        || session.understanding.solution_target.trim().is_empty()
    {
        return Err("consolidation omitted a required synthesized field".into());
    }

    session = confirm_ask_for_fix(&pool, &runtime, &session.session_id).await?;
    if session.phase != AskPhase::Present || session.status != "answered" || !session.locked {
        return Err("confirmed conversation did not reach a locked answer".into());
    }
    let first_headline = session
        .presentation
        .as_ref()
        .ok_or("answered session omitted its presentation")?
        .headline
        .clone();
    session = submit_ask_for_fix_turn(
        &pool,
        &runtime,
        &session.session_id,
        AskUserTurn {
            text: "Add an explicit duplicate check before my final review.".into(),
            event: AskInputEvent {
                kind: "revise".into(),
                question_id: None,
                selected_option_ids: vec![],
            },
        },
    )
    .await?;
    if session.phase != AskPhase::Present || session.status != "answered" || !session.locked {
        return Err("artifact revision did not preserve the locked answer state".into());
    }
    let presentation = session
        .presentation
        .as_ref()
        .ok_or("revised session omitted its presentation")?;
    let attempts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ask_attempts")
        .fetch_one(&pool)
        .await?;
    let usage_receipts: Vec<String> =
        sqlx::query_scalar("SELECT usage_json FROM ask_attempts ORDER BY created_at,attempt")
            .fetch_all(&pool)
            .await?;
    let cached_input_tokens = usage_receipts
        .iter()
        .map(|receipt| {
            serde_json::from_str::<serde_json::Value>(receipt)
                .ok()
                .and_then(|usage| {
                    usage
                        .get("cached_input_tokens")
                        .and_then(|value| value.as_u64())
                })
                .unwrap_or_default()
        })
        .collect::<Vec<_>>();
    let distinct_prefixes: i64 =
        sqlx::query_scalar("SELECT COUNT(DISTINCT stable_prompt_hash) FROM ask_jobs")
            .fetch_one(&pool)
            .await?;
    let high_reasoning_receipts: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM ask_jobs WHERE model=?1 AND status='accepted'")
            .bind(FRONTIER_MODEL)
            .fetch_one(&pool)
            .await?;
    if distinct_prefixes != 1 || high_reasoning_receipts == 0 {
        return Err("model-call receipts do not prove one stable frontier prefix".into());
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "status": "ok",
            "phase": session.phase,
            "questionCount": session.question_count,
            "locked": session.locked,
            "model": session.model,
            "cachedInputTokensLatest": session.cached_input_tokens,
            "cachedInputTokensByAttempt": cached_input_tokens,
            "attempts": attempts,
            "stablePromptHashes": distinct_prefixes,
            "headline": presentation.headline,
            "headlineChanged": presentation.headline != first_headline,
            "route": presentation.route,
            "artifactKind": presentation.artifact.as_ref().map(|artifact| artifact.kind),
        }))?
    );
    Ok(())
}
