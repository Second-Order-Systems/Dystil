//! Opt-in smoke test for the real automation execution boundary.
//!
//! This spends provider tokens and requires an authenticated Codex CLI, so it
//! remains ignored during normal test runs. See the test's error messages for
//! the required environment variables.

use async_trait::async_trait;
use dystil_ai::{AiAutomationRequest, AiRuntimeEvent, CliProvider, McpServerConfig, ProviderKind};
use dystil_automation::{
    create, enqueue, execute, migrate, AutomationRunner, ErrorCategory, ExecutionRequest,
    ExecutionResult, RunEvent, RunStatus,
};
use sqlx::SqlitePool;
use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::mpsc;

struct RealCodexRunner {
    provider: CliProvider,
}

#[async_trait]
impl AutomationRunner for RealCodexRunner {
    async fn run(
        &self,
        request: ExecutionRequest,
        events: mpsc::Sender<RunEvent>,
    ) -> std::result::Result<ExecutionResult, (ErrorCategory, String)> {
        let (provider_tx, mut provider_rx) = mpsc::channel::<AiRuntimeEvent>(128);
        let diagnostics = Arc::new(Mutex::new(Vec::new()));
        let bridged_diagnostics = diagnostics.clone();
        let bridge = tokio::spawn(async move {
            while let Some(event) = provider_rx.recv().await {
                if let Ok(mut lines) = bridged_diagnostics.lock() {
                    lines.push(format!("{}: {}", event.kind, event.message));
                }
                if events
                    .send(dystil_automation::now_event(event.kind, event.message))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
        let result = self
            .provider
            .run_automation_with_model(
                AiAutomationRequest {
                    prompt: request.prompt,
                    working_directory: request.working_directory,
                    timeout: Duration::from_secs(request.timeout_seconds),
                },
                None,
                provider_tx,
            )
            .await;
        let _ = bridge.await;
        result
            .map(|run| ExecutionResult {
                output: run.output,
                provider: "codex".into(),
                model: "default".into(),
            })
            .map_err(|error| {
                let detail = diagnostics
                    .lock()
                    .map(|lines| lines.join("\n"))
                    .unwrap_or_default();
                (ErrorCategory::Provider, format!("{error}\n{detail}"))
            })
    }
}

fn required_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name)
        .unwrap_or_else(|| panic!("set {name} to an absolute filesystem path"));
    let path = PathBuf::from(value);
    assert!(path.is_absolute(), "{name} must be absolute");
    assert!(
        path.is_file(),
        "{name} does not point to a file: {}",
        path.display()
    );
    path
}

#[tokio::test]
#[ignore = "requires authenticated Codex and spends real provider tokens"]
async fn codex_uses_dystil_tools_and_persists_real_automation_outputs() {
    let codex = required_path("DYSTIL_REAL_CODEX_BIN");
    let mcp = required_path("DYSTIL_REAL_MCP_BIN");
    let capture_database = required_path("DYSTIL_REAL_CAPTURE_DB");
    let workspace = tempfile::tempdir().unwrap();
    let markdown = r#"---
name: real-fixture-recap
title: Real fixture recap
enabled: true
timeout_seconds: 300
trigger:
  type: manual
retry:
  max_attempts: 1
---

Use Dystil's activity tools to inspect the captured work. Call the activity overview, then inspect one representative source. Write `fixture-summary.md` containing a concise evidence-grounded summary with at least one stable evidence ID. Write `fixture.live-view.json` as valid JSON with `title`, `body`, and a non-empty `evidence_ids` array. Read `memory.md` if present, then create or update it with one durable lesson from this run. Do not invent evidence.
"#;
    let document = create(workspace.path(), markdown).unwrap();
    let state = SqlitePool::connect("sqlite::memory:").await.unwrap();
    migrate(&state).await.unwrap();
    let run_id = enqueue(&state, &document.name, "manual", None, 1)
        .await
        .unwrap()
        .unwrap();
    let runner = RealCodexRunner {
        provider: CliProvider {
            provider: ProviderKind::Codex,
            executable: codex,
            runtime_version: None,
            environment: Vec::new(),
            mcp_server: Some(McpServerConfig {
                command: mcp,
                args: vec![
                    "--database".into(),
                    capture_database.to_string_lossy().into_owned(),
                    "--max-calls".into(),
                    "6".into(),
                ],
            }),
        },
    };

    let run = execute(&state, &document, &run_id, &runner).await.unwrap();
    assert_eq!(run.status, RunStatus::Succeeded, "{run:?}");
    assert_eq!(run.provider.as_deref(), Some("codex"));

    let summary = std::fs::read_to_string(document.directory.join("fixture-summary.md")).unwrap();
    assert!(summary.contains("frame:") || summary.contains("event:"));
    let live_view: serde_json::Value = serde_json::from_slice(
        &std::fs::read(document.directory.join("fixture.live-view.json")).unwrap(),
    )
    .unwrap();
    assert!(live_view["title"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(live_view["body"]
        .as_str()
        .is_some_and(|value| !value.is_empty()));
    assert!(live_view["evidence_ids"]
        .as_array()
        .is_some_and(|ids| !ids.is_empty()));
    assert!(document.directory.join("memory.md").is_file());

    let artifact_paths: Vec<String> = sqlx::query_scalar(
        "SELECT relative_path FROM automation_artifacts WHERE run_id=?1 ORDER BY relative_path",
    )
    .bind(&run_id)
    .fetch_all(&state)
    .await
    .unwrap();
    assert!(artifact_paths
        .iter()
        .any(|path| path == "fixture-summary.md"));
    assert!(artifact_paths
        .iter()
        .any(|path| path == "fixture.live-view.json"));
    let event_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM automation_run_events WHERE run_id=?1")
            .bind(&run_id)
            .fetch_one(&state)
            .await
            .unwrap();
    assert!(event_count > 0, "real provider events were not persisted");
}
