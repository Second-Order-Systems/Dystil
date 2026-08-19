//! Headless backtest for the production prompt-and-skill bundle builder.
//!
//! It operates only on an explicitly supplied copy of an insights database and
//! output root. It never starts the desktop app, a viewer, or an interactive
//! provider session.

use std::{
    fs,
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use clap::{Parser, ValueEnum};
use dystil_ai::{
    AiAnswerRequest, AiAutomationRequest, AiAutomationRun, AiModelTier, AiRuntime,
    AiRuntimeDescriptor, AiRuntimeError, AiRuntimeErrorCode, AiRuntimeEvent, AiRuntimeKind,
    AiStructuredRequest, AiStructuredRun, CliProvider, McpServerConfig, ProviderKind,
    TeammateAnswerRun,
};
use dystil_insights::{build_skill_bundle, open_insights_database, SkillBundlePaths};
use serde_json::{json, Value};
use sqlx::Row;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::mpsc,
};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Provider {
    Codex,
    Claude,
    Pi,
}

#[derive(Debug, Parser)]
#[command(
    name = "skill-bundle-backtest",
    about = "Build portable skills from a copied Dystil fixture without opening UI"
)]
struct Args {
    /// Accepted to make fixture invocations explicit. The bundle builder reads only the supplied insights DB.
    #[arg(long)]
    capture_db: Option<PathBuf>,
    #[arg(long)]
    insights_db: PathBuf,
    #[arg(long)]
    output_root: PathBuf,
    #[arg(long, required = true)]
    artifact_id: Vec<String>,
    #[arg(long, value_enum)]
    provider: Provider,
    #[arg(long)]
    provider_executable: PathBuf,
    #[arg(long)]
    provider_state: Option<PathBuf>,
    #[arg(long)]
    mcp_executable: Option<PathBuf>,
    #[arg(long)]
    model: String,
    /// Pi provider identifier, such as `anthropic`, `ollama`, or `custom`.
    #[arg(long, default_value = "anthropic")]
    pi_provider: String,
    /// Optional Pi provider API key; it is passed only to the child process.
    #[arg(long)]
    pi_api_key: Option<String>,
    /// Pi's API endpoint. It is used only with `--provider pi`.
    #[arg(long, default_value = "https://api.anthropic.com")]
    pi_endpoint: String,
    /// The checked-in Dystil Pi retrieval extension. Required for a Pi run.
    #[arg(long)]
    pi_extension: Option<PathBuf>,
    #[arg(long, default_value_t = 1)]
    repetitions: u32,
    /// Optional diagnostic directory. Each successful case writes its validated
    /// WORKFLOW.md here so the reconstruction can be graded before the bundle.
    #[arg(long)]
    reconstruction_report_dir: Option<PathBuf>,
}

struct BacktestRuntime {
    descriptor: AiRuntimeDescriptor,
    executable: PathBuf,
    provider: Provider,
    state: Option<PathBuf>,
    mcp: Option<McpServerConfig>,
    pi_provider: String,
    pi_api_key: Option<String>,
    pi_endpoint: String,
    pi_extension: Option<PathBuf>,
}

/// A backtest repetition must begin without a ready bundle receipt; otherwise
/// `build_skill_bundle()` correctly returns its existing immutable revision and
/// the purported repetition never reaches the provider. Preserve the SQLite WAL
/// companions too, because the supplied fixture may have recent data there.
fn copy_fixture_database(
    source: &Path,
    destination: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let parent = destination
        .parent()
        .ok_or("fixture destination has no parent")?;
    fs::create_dir_all(parent)?;
    fs::copy(source, destination)?;
    for suffix in ["-wal", "-shm"] {
        let source_sidecar = PathBuf::from(format!("{}{}", source.display(), suffix));
        if source_sidecar.exists() {
            fs::copy(
                source_sidecar,
                PathBuf::from(format!("{}{}", destination.display(), suffix)),
            )?;
        }
    }
    Ok(())
}

#[derive(Default)]
struct PiRpcAccumulator {
    streamed_text: String,
    final_text: Option<String>,
    provider_error: Option<String>,
    settled: bool,
}

impl PiRpcAccumulator {
    fn observe(&mut self, event: &Value) {
        if event.get("type").and_then(Value::as_str) == Some("message_update")
            && event
                .pointer("/assistantMessageEvent/type")
                .and_then(Value::as_str)
                == Some("text_delta")
        {
            if let Some(delta) = event
                .pointer("/assistantMessageEvent/delta")
                .and_then(Value::as_str)
            {
                self.streamed_text.push_str(delta);
            }
        }
        if event.get("type").and_then(Value::as_str) == Some("message_end")
            && event.pointer("/message/role").and_then(Value::as_str) == Some("assistant")
        {
            let stop_reason = event.pointer("/message/stopReason").and_then(Value::as_str);
            if matches!(stop_reason, Some("error" | "aborted")) {
                self.provider_error = event
                    .pointer("/message/errorMessage")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| Some(format!("Pi stopped with {stop_reason:?}")));
            } else if let Some(content) =
                event.pointer("/message/content").and_then(Value::as_array)
            {
                let text = content
                    .iter()
                    .filter(|item| item.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|item| item.get("text").and_then(Value::as_str))
                    .collect::<String>();
                if !text.trim().is_empty() {
                    self.final_text = Some(text);
                    self.provider_error = None;
                }
            }
        }
        if event.get("type").and_then(Value::as_str) == Some("auto_retry_end")
            && event.get("success").and_then(Value::as_bool) == Some(false)
        {
            self.provider_error = event
                .get("finalError")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some("Pi exhausted its provider retries".into()));
        }
        self.settled = event.get("type").and_then(Value::as_str) == Some("agent_settled");
    }

    fn finish(self) -> Result<String, AiRuntimeError> {
        self.final_text
            .filter(|text| !text.trim().is_empty())
            .or_else(|| (!self.streamed_text.trim().is_empty()).then_some(self.streamed_text))
            .ok_or_else(|| {
                AiRuntimeError::new(
                    AiRuntimeErrorCode::Transport,
                    self.provider_error
                        .map(|error| format!("Pi provider failed: {error}"))
                        .unwrap_or_else(|| {
                            "Pi returned invalid output: completed without an assistant response"
                                .into()
                        }),
                )
            })
    }
}

fn write_pi_models(
    state: &std::path::Path,
    provider: &str,
    endpoint: &str,
    model: &str,
) -> Result<(), AiRuntimeError> {
    std::fs::create_dir_all(state)
        .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string()))?;
    let api = if provider == "anthropic" {
        "anthropic-messages"
    } else {
        "openai-completions"
    };
    let config = json!({"providers": {provider: {"baseUrl": endpoint, "api": api, "apiKey": "$CUSTOM_API_KEY", "models": [{"id": model, "name": model, "reasoning": true, "input": ["text"], "maxTokens": 8192, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}}]}}});
    std::fs::write(
        state.join("models.json"),
        serde_json::to_vec_pretty(&config).map_err(|error| {
            AiRuntimeError::new(AiRuntimeErrorCode::Internal, error.to_string())
        })?,
    )
    .map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string()))
}

#[async_trait]
impl AiRuntime for BacktestRuntime {
    fn descriptor(&self) -> &AiRuntimeDescriptor {
        &self.descriptor
    }
    fn model_for_tier(&self, _: AiModelTier) -> String {
        self.descriptor.model.clone()
    }
    async fn answer(&self, _: AiAnswerRequest) -> Result<TeammateAnswerRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "backtest only builds bundles",
        ))
    }
    async fn infer_structured(
        &self,
        _: AiStructuredRequest,
    ) -> Result<AiStructuredRun, AiRuntimeError> {
        Err(AiRuntimeError::new(
            AiRuntimeErrorCode::Internal,
            "backtest only builds bundles",
        ))
    }
    async fn run_automation(
        &self,
        request: AiAutomationRequest,
        events: mpsc::Sender<AiRuntimeEvent>,
    ) -> Result<AiAutomationRun, AiRuntimeError> {
        match self.provider {
            Provider::Codex | Provider::Claude => {
                let provider = CliProvider {
                    provider: if matches!(self.provider, Provider::Codex) {
                        ProviderKind::Codex
                    } else {
                        ProviderKind::Claude
                    },
                    executable: self.executable.clone(),
                    runtime_version: None,
                    environment: self
                        .state
                        .as_ref()
                        .map(|path| {
                            vec![(
                                if matches!(self.provider, Provider::Codex) {
                                    "CODEX_HOME".into()
                                } else {
                                    "CLAUDE_CONFIG_DIR".into()
                                },
                                path.to_string_lossy().into_owned(),
                            )]
                        })
                        .unwrap_or_default(),
                    mcp_server: self.mcp.clone(),
                };
                provider
                    .run_automation_with_model(request, Some(&self.descriptor.model), events)
                    .await
                    .map_err(AiRuntimeError::from)
            }
            Provider::Pi => {
                let mcp = self.mcp.as_ref().ok_or_else(|| {
                    AiRuntimeError::new(
                        AiRuntimeErrorCode::NotReady,
                        "Pi backtests require a Dystil MCP server",
                    )
                })?;
                let extension = self.pi_extension.as_ref().ok_or_else(|| {
                    AiRuntimeError::new(
                        AiRuntimeErrorCode::NotReady,
                        "Pi backtests require --pi-extension",
                    )
                })?;
                let state = self.state.as_ref().ok_or_else(|| {
                    AiRuntimeError::new(
                        AiRuntimeErrorCode::NotReady,
                        "Pi backtests require --provider-state",
                    )
                })?;
                write_pi_models(
                    state,
                    &self.pi_provider,
                    &self.pi_endpoint,
                    &self.descriptor.model,
                )?;
                let started = std::time::Instant::now();
                let mut child = Command::new(&self.executable)
                    .args([
                        "--mode",
                        "rpc",
                        "--provider",
                        &self.pi_provider,
                        "--model",
                        &self.descriptor.model,
                        "--system-prompt",
                        "You run a Dystil automation. Follow automation.md instructions, use Dystil retrieval tools for captured evidence, use filesystem tools for memory and artifacts, and finish with a concise result.",
                        "--tools",
                        "read,write,edit,bash,dystil_get_activity_overview,dystil_search_activity,dystil_get_source,dystil_get_activity_context,dystil_get_activity_range",
                        "--extension",
                        extension.to_string_lossy().as_ref(),
                        "--no-extensions",
                        "--no-skills",
                        "--no-prompt-templates",
                        "--no-context-files",
                        "--no-approve",
                        "--no-session",
                        "--offline",
                    ])
                    .env("PI_CODING_AGENT_DIR", state)
                    .env("PI_SKIP_VERSION_CHECK", "1")
                    .env("PI_TELEMETRY", "0")
                    .current_dir(&request.working_directory)
                    .env("CUSTOM_API_KEY", self.pi_api_key.as_deref().unwrap_or(""))
                    .env("DYSTIL_MCP_COMMAND", &mcp.command)
                    .env("DYSTIL_MCP_ARGS", serde_json::to_string(&mcp.args).map_err(|error| AiRuntimeError::new(AiRuntimeErrorCode::Internal, error.to_string()))?)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .kill_on_drop(true)
                    .spawn()
                    .map_err(|error| {
                        AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string())
                    })?;
                let mut stdin = child.stdin.take().ok_or_else(|| {
                    AiRuntimeError::new(AiRuntimeErrorCode::Transport, "Pi stdin is unavailable")
                })?;
                let message = serde_json::json!({"type":"prompt", "message":request.prompt, "id":"dystil-skill-bundle-backtest"}).to_string();
                stdin
                    .write_all(format!("{message}\n").as_bytes())
                    .await
                    .map_err(|error| {
                        AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string())
                    })?;
                stdin.flush().await.map_err(|error| {
                    AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string())
                })?;
                let stdout = child.stdout.take().ok_or_else(|| {
                    AiRuntimeError::new(AiRuntimeErrorCode::Transport, "Pi stdout is unavailable")
                })?;
                let mut lines = BufReader::new(stdout).lines();
                let read = async {
                    let mut result = PiRpcAccumulator::default();
                    while let Some(line) = lines.next_line().await.map_err(|error| {
                        AiRuntimeError::new(AiRuntimeErrorCode::Transport, error.to_string())
                    })? {
                        let _ = events
                            .send(AiRuntimeEvent {
                                kind: "agent".into(),
                                message: line.clone(),
                            })
                            .await;
                        if let Ok(event) = serde_json::from_str::<Value>(&line) {
                            result.observe(&event);
                            if result.settled {
                                break;
                            }
                        }
                    }
                    result.finish()
                };
                let output =
                    tokio::time::timeout(request.timeout, read)
                        .await
                        .map_err(|_| {
                            AiRuntimeError::new(AiRuntimeErrorCode::Timeout, "Pi timed out")
                        })??;
                let _ = child.kill().await;
                Ok(AiAutomationRun {
                    runtime: AiRuntimeKind::Pi,
                    runtime_version: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    output,
                })
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.insights_db.exists() {
        return Err("--insights-db must point to a copied fixture database".into());
    }
    if matches!(args.provider, Provider::Pi) && args.pi_api_key.as_deref().is_none_or(str::is_empty)
    {
        return Err("--provider pi requires --pi-api-key, just as Dystil's active Pi preset requires a credential".into());
    }
    if let Some(state) = &args.provider_state {
        std::fs::create_dir_all(state)?;
    }
    // Provider subprocesses run inside each per-case workspace, so every path
    // handed to an MCP server must be absolute rather than relative to this
    // binary's original working directory.
    let insights_db = fs::canonicalize(&args.insights_db)?;
    let capture_db = args.capture_db.as_ref().map(fs::canonicalize).transpose()?;
    let provider_executable = fs::canonicalize(&args.provider_executable)?;
    let mcp_executable = args
        .mcp_executable
        .as_ref()
        .map(fs::canonicalize)
        .transpose()?;
    let kind = match args.provider {
        Provider::Codex => AiRuntimeKind::Codex,
        Provider::Claude => AiRuntimeKind::Claude,
        Provider::Pi => AiRuntimeKind::Pi,
    };
    let mcp = match (mcp_executable, capture_db.as_ref()) {
        (Some(command), Some(capture_db)) => Some(McpServerConfig {
            command,
            args: vec![
                "--database".into(),
                capture_db.to_string_lossy().into_owned(),
            ],
        }),
        (Some(_), None) => return Err("--mcp-executable requires --capture-db".into()),
        (None, _) => None,
    };
    let runtime = BacktestRuntime {
        descriptor: AiRuntimeDescriptor {
            kind,
            provider_label: format!("{:?}", args.provider).to_lowercase(),
            model: args.model,
        },
        executable: provider_executable,
        provider: args.provider,
        state: args.provider_state,
        mcp,
        pi_provider: args.pi_provider,
        pi_api_key: args.pi_api_key,
        pi_endpoint: args.pi_endpoint,
        pi_extension: args.pi_extension,
    };
    let mut results = Vec::new();
    let mut failures = 0_u32;
    for repetition in 1..=args.repetitions {
        let repetition_root = args.output_root.join(format!("repetition-{repetition}"));
        let repetition_db = repetition_root.join("worth-fixing.sqlite");
        copy_fixture_database(&insights_db, &repetition_db)?;
        let pool = open_insights_database(&repetition_db).await?;
        let paths = SkillBundlePaths {
            builds_root: repetition_root.join("builds"),
            bundles_root: repetition_root.join("bundles"),
        };
        for artifact_id in &args.artifact_id {
            match build_skill_bundle(&pool, &runtime, artifact_id, &paths).await {
                Ok(bundle) => {
                    let reconstruction = sqlx::query(
                        "SELECT body,evidence_ids_json,reconstruction_version,elapsed_ms
                         FROM artifact_workflow_reconstructions
                         WHERE artifact_id=?1 ORDER BY created_at DESC LIMIT 1",
                    )
                    .bind(artifact_id)
                    .fetch_optional(&pool)
                    .await?;
                    let reconstruction_summary = reconstruction.as_ref().map(|row| {
                        json!({
                            "version": row.get::<String, _>("reconstruction_version"),
                            "evidence_ids": serde_json::from_str::<Value>(&row.get::<String, _>("evidence_ids_json")).unwrap_or(Value::Null),
                            "bytes": row.get::<String, _>("body").len(),
                            "elapsed_ms": row.get::<i64, _>("elapsed_ms"),
                        })
                    });
                    if let (Some(root), Some(row)) =
                        (&args.reconstruction_report_dir, reconstruction)
                    {
                        let report = root
                            .join(format!("repetition-{repetition}"))
                            .join(artifact_id)
                            .join("WORKFLOW.md");
                        if let Some(parent) = report.parent() {
                            fs::create_dir_all(parent)?;
                        }
                        fs::write(report, row.get::<String, _>("body"))?;
                    }
                    results.push(serde_json::json!({
                        "artifact_id": artifact_id,
                        "repetition": repetition,
                        "ok": true,
                        "result": bundle,
                        "reconstruction": reconstruction_summary,
                    }));
                }
                Err(error) => {
                    failures += 1;
                    results.push(serde_json::json!({
                        "artifact_id": artifact_id,
                        "repetition": repetition,
                        "ok": false,
                        "error": error.to_string(),
                    }));
                }
            }
        }
    }
    println!("{}", serde_json::to_string(&results)?);
    if failures > 0 {
        return Err(format!("{failures} skill-bundle backtest case(s) failed").into());
    }
    Ok(())
}
