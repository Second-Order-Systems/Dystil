use async_trait::async_trait;
use dystil_automation::{
    AutomationDocument, AutomationRunner, ErrorCategory, ExecutionRequest, ExecutionResult,
    RunEvent,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Semaphore;

use crate::{ai, ai_runtime, recording::RecordingState};

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutomationView {
    pub name: String,
    pub title: String,
    pub description: Option<String>,
    pub enabled: bool,
    pub trigger_type: String,
    pub trigger_detail: Option<String>,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunView {
    pub id: String,
    pub automation_name: String,
    pub status: String,
    pub trigger: String,
    pub attempt: u32,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub output: Option<String>,
    pub error_category: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutomationDraftView {
    pub id: String,
    pub request: String,
    pub markdown: String,
    pub automation: AutomationView,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutomationArtifactView {
    pub id: String,
    pub run_id: String,
    pub automation_name: String,
    pub relative_path: String,
    pub size_bytes: i64,
    pub media_type: String,
    pub live_view: bool,
    pub output_kind: String,
    pub content_json: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRunEventView {
    pub id: i64,
    pub run_id: String,
    pub kind: String,
    pub message: String,
    pub created_at: String,
}

fn root() -> PathBuf {
    crate::dystil_paths::data_dir().join("automations")
}
fn drafts_root() -> PathBuf {
    crate::dystil_paths::data_dir().join("automation-drafts")
}
fn concurrency() -> Arc<Semaphore> {
    static LIMIT: OnceLock<Arc<Semaphore>> = OnceLock::new();
    LIMIT.get_or_init(|| Arc::new(Semaphore::new(2))).clone()
}
fn running_tasks() -> &'static Mutex<HashMap<String, tokio::task::AbortHandle>> {
    static TASKS: OnceLock<Mutex<HashMap<String, tokio::task::AbortHandle>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn manager_task() -> &'static Mutex<Option<tokio::sync::watch::Sender<()>>> {
    static TASK: OnceLock<Mutex<Option<tokio::sync::watch::Sender<()>>>> = OnceLock::new();
    TASK.get_or_init(|| Mutex::new(None))
}

async fn pool(state: &RecordingState) -> Result<sqlx::SqlitePool, String> {
    let pool = ai::capture_pool(state).await?;
    dystil_automation::migrate(&pool)
        .await
        .map_err(|error| error.to_string())?;
    Ok(pool)
}

fn require_local_automation() -> Result<(), String> {
    if matches!(
        crate::app_policy::current().local_automation,
        crate::app_policy::Availability::Disabled
    ) {
        return Err("Local automations are disabled in this enterprise build.".to_string());
    }
    Ok(())
}

fn to_view(document: &AutomationDocument) -> AutomationView {
    let (trigger_type, trigger_detail) = match &document.trigger {
        dystil_automation::Trigger::Manual => ("manual".into(), None),
        dystil_automation::Trigger::Schedule { schedule } => {
            ("schedule".into(), Some(schedule.clone()))
        }
        dystil_automation::Trigger::Event { source, filter } => (
            "event".into(),
            Some(
                filter
                    .as_ref()
                    .map(|value| format!("{source}: {value}"))
                    .unwrap_or_else(|| source.clone()),
            ),
        ),
    };
    AutomationView {
        name: document.name.clone(),
        title: document
            .title
            .clone()
            .unwrap_or_else(|| document.name.clone()),
        description: document.description.clone(),
        enabled: document.enabled,
        trigger_type,
        trigger_detail,
        path: document
            .directory
            .join("automation.md")
            .to_string_lossy()
            .into(),
    }
}

fn run_view(run: dystil_automation::RunRecord) -> AutomationRunView {
    AutomationRunView {
        id: run.id,
        automation_name: run.automation_name,
        status: serde_json::to_value(run.status)
            .unwrap()
            .as_str()
            .unwrap()
            .into(),
        trigger: run.trigger,
        attempt: run.attempt,
        started_at: run.started_at,
        finished_at: run.finished_at,
        provider: run.provider,
        model: run.model,
        output: run.output,
        error_category: run.error_category.map(|value| {
            serde_json::to_value(value)
                .unwrap()
                .as_str()
                .unwrap()
                .into()
        }),
        error_message: run.error_message,
    }
}

fn document(name: &str) -> Result<AutomationDocument, String> {
    dystil_automation::discover(&root())
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|item| item.name == name)
        .ok_or_else(|| format!("automation not found: {name}"))
}

#[tauri::command]
#[specta::specta]
pub async fn automation_list() -> Result<Vec<AutomationView>, String> {
    Ok(dystil_automation::discover(&root())
        .map_err(|error| error.to_string())?
        .iter()
        .map(to_view)
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_create(markdown: String) -> Result<AutomationView, String> {
    let document =
        dystil_automation::create(&root(), &markdown).map_err(|error| error.to_string())?;
    Ok(to_view(&document))
}

#[tauri::command]
#[specta::specta]
pub async fn automation_delete(name: String) -> Result<(), String> {
    let document = document(&name)?;
    std::fs::remove_dir_all(document.directory).map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_set_enabled(name: String, enabled: bool) -> Result<AutomationView, String> {
    let current = document(&name)?;
    let updated = dystil_automation::set_enabled(&current.directory.join("automation.md"), enabled)
        .map_err(|error| error.to_string())?;
    Ok(to_view(&updated))
}

#[tauri::command]
#[specta::specta]
pub async fn automation_open_definition(name: String, app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let path = document(&name)?.directory.join("automation.md");
    app.opener()
        .open_path(path.to_string_lossy(), None::<&str>)
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_list_runs(
    name: Option<String>,
    before: Option<String>,
    limit: Option<u32>,
    state: State<'_, RecordingState>,
) -> Result<Vec<AutomationRunView>, String> {
    Ok(dystil_automation::list_runs(
        &pool(&state).await?,
        name.as_deref(),
        before.as_deref(),
        limit.unwrap_or(50),
    )
    .await
    .map_err(|error| error.to_string())?
    .into_iter()
    .map(run_view)
    .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_list_artifacts(
    run_id: Option<String>,
    limit: Option<u32>,
    state: State<'_, RecordingState>,
) -> Result<Vec<AutomationArtifactView>, String> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT a.id,a.run_id,r.automation_name,a.relative_path,a.size_bytes,a.media_type,a.created_at FROM automation_artifacts a JOIN automation_runs r ON r.id=a.run_id WHERE (?1 IS NULL OR a.run_id=?1) ORDER BY a.created_at DESC,a.id DESC LIMIT ?2").bind(run_id).bind(limit.unwrap_or(100).clamp(1,200) as i64).fetch_all(&pool(&state).await?).await.map_err(|error|error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let path: String = row.get("relative_path");
            let automation_name: String = row.get("automation_name");
            let size_bytes: i64 = row.get("size_bytes");
            let live_view = path.ends_with(".live.json") || path.ends_with(".live-view.json");
            let notification = path.ends_with(".notification.json");
            let output_kind = if live_view {
                "live_view"
            } else if notification {
                "notification"
            } else {
                "artifact"
            }
            .to_string();
            let content_json = if (live_view || notification) && size_bytes <= 256 * 1024 {
                std::fs::read_to_string(root().join(&automation_name).join(&path))
                    .ok()
                    .filter(|value| serde_json::from_str::<serde_json::Value>(value).is_ok())
            } else {
                None
            };
            AutomationArtifactView {
                id: row.get("id"),
                run_id: row.get("run_id"),
                automation_name,
                live_view,
                output_kind,
                content_json,
                relative_path: path,
                size_bytes,
                media_type: row.get("media_type"),
                created_at: row.get("created_at"),
            }
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_run_events(
    run_id: String,
    before_id: Option<i64>,
    limit: Option<u32>,
    state: State<'_, RecordingState>,
) -> Result<Vec<AutomationRunEventView>, String> {
    use sqlx::Row;
    let rows=sqlx::query("SELECT id,run_id,kind,message,created_at FROM automation_run_events WHERE run_id=?1 AND (?2 IS NULL OR id<?2) ORDER BY id DESC LIMIT ?3").bind(run_id).bind(before_id).bind(limit.unwrap_or(100).clamp(1,500) as i64).fetch_all(&pool(&state).await?).await.map_err(|error|error.to_string())?;
    Ok(rows
        .into_iter()
        .map(|row| AutomationRunEventView {
            id: row.get("id"),
            run_id: row.get("run_id"),
            kind: row.get("kind"),
            message: row.get("message"),
            created_at: row.get("created_at"),
        })
        .collect())
}

#[tauri::command]
#[specta::specta]
pub async fn automation_reveal_artifact(
    artifact_id: String,
    state: State<'_, RecordingState>,
    app: AppHandle,
) -> Result<(), String> {
    use sqlx::Row;
    use tauri_plugin_opener::OpenerExt;
    let row=sqlx::query("SELECT r.automation_name,a.relative_path FROM automation_artifacts a JOIN automation_runs r ON r.id=a.run_id WHERE a.id=?1").bind(artifact_id).fetch_optional(&pool(&state).await?).await.map_err(|error|error.to_string())?.ok_or("artifact not found")?;
    let directory = root().join(row.get::<String, _>("automation_name"));
    let path = directory.join(row.get::<String, _>("relative_path"));
    let canonical = path.canonicalize().map_err(|error| error.to_string())?;
    let canonical_directory = directory
        .canonicalize()
        .map_err(|error| error.to_string())?;
    if !canonical.starts_with(canonical_directory) {
        return Err("artifact path escaped its automation folder".into());
    }
    app.opener()
        .reveal_item_in_dir(canonical)
        .map_err(|error| error.to_string())
}

struct AppRunner {
    runtime: Box<dyn dystil_ai::AiRuntime>,
    app: AppHandle,
}

#[async_trait]
impl AutomationRunner for AppRunner {
    async fn run(
        &self,
        request: ExecutionRequest,
        events: tokio::sync::mpsc::Sender<RunEvent>,
    ) -> std::result::Result<ExecutionResult, (ErrorCategory, String)> {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<dystil_ai::AiRuntimeEvent>(128);
        let event_tx = events.clone();
        let app = self.app.clone();
        let run_id = request.run_id.clone();
        let bridge = tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let normalized = dystil_automation::now_event(event.kind, event.message);
                let _ = app.emit(
                    "automation-run-event",
                    serde_json::json!({"runId":run_id,"event":normalized}),
                );
                if event_tx.send(normalized).await.is_err() {
                    break;
                }
            }
        });
        let result = self
            .runtime
            .run_automation(
                dystil_ai::AiAutomationRequest {
                    prompt: request.prompt,
                    working_directory: request.working_directory,
                    timeout: Duration::from_secs(request.timeout_seconds),
                },
                tx,
            )
            .await;
        let _ = bridge.await;
        result
            .map(|run| ExecutionResult {
                output: run.output,
                provider: self.runtime.descriptor().provider_label.clone(),
                model: self.runtime.descriptor().model.clone(),
            })
            .map_err(|error| {
                let category = match error.code {
                    dystil_ai::AiRuntimeErrorCode::Authentication => ErrorCategory::ProviderAuth,
                    dystil_ai::AiRuntimeErrorCode::Timeout => ErrorCategory::Timeout,
                    dystil_ai::AiRuntimeErrorCode::NotReady => ErrorCategory::Configuration,
                    dystil_ai::AiRuntimeErrorCode::Transport => ErrorCategory::Provider,
                    _ => ErrorCategory::Internal,
                };
                (category, error.message)
            })
    }
}

#[tauri::command]
#[specta::specta]
pub async fn automation_run_now(
    name: String,
    app: AppHandle,
    state: State<'_, RecordingState>,
) -> Result<AutomationRunView, String> {
    require_local_automation()?;
    let document = document(&name)?;
    let database = pool(&state).await?;
    let _permit = concurrency()
        .acquire_owned()
        .await
        .map_err(|_| "automation queue is closed")?;
    let run_id = dystil_automation::enqueue(&database, &name, "manual", None, 1)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("run was deduplicated")?;
    let timezone = ai::local_timezone_offset();
    let runtime = ai_runtime::resolve(&app, &state, &database, &timezone)
        .await
        .map_err(|error| error.to_string())?;
    let execution_database = database.clone();
    let execution_id = run_id.clone();
    let execution_app = app.clone();
    let task = tokio::spawn(async move {
        dystil_automation::execute(
            &execution_database,
            &document,
            &execution_id,
            &AppRunner {
                runtime,
                app: execution_app,
            },
        )
        .await
    });
    running_tasks()
        .lock()
        .map_err(|_| "automation task registry is unavailable")?
        .insert(run_id.clone(), task.abort_handle());
    let _=app.emit("automation-run-event",serde_json::json!({"runId":run_id,"event":{"kind":"status","message":"queued","createdAt":chrono::Utc::now().to_rfc3339()}}));
    let run = task
        .await
        .map_err(|error| {
            if error.is_cancelled() {
                "automation cancelled".to_string()
            } else {
                error.to_string()
            }
        })?
        .map_err(|error| error.to_string())?;
    if let Ok(mut tasks) = running_tasks().lock() {
        tasks.remove(&run_id);
    }
    let view = run_view(run);
    let _ = app.emit("automation-run-updated", &view);
    Ok(view)
}

#[tauri::command]
#[specta::specta]
pub async fn automation_cancel(
    run_id: String,
    state: State<'_, RecordingState>,
    app: AppHandle,
) -> Result<(), String> {
    let handle = running_tasks()
        .lock()
        .map_err(|_| "automation task registry is unavailable")?
        .remove(&run_id)
        .ok_or("automation run is not active")?;
    handle.abort();
    sqlx::query("UPDATE automation_runs SET status='cancelled',finished_at=datetime('now'),error_category='cancelled',error_message='cancelled by user' WHERE id=?1 AND status IN ('queued','running','retrying')").bind(&run_id).execute(&pool(&state).await?).await.map_err(|error|error.to_string())?;
    let _ = app.emit(
        "automation-run-updated",
        serde_json::json!({"id":run_id,"status":"cancelled"}),
    );
    Ok(())
}

async fn run_background(
    app: AppHandle,
    database: sqlx::SqlitePool,
    document: AutomationDocument,
    trigger: &str,
    event_key: Option<String>,
) -> Result<AutomationRunView, String> {
    let _permit = concurrency()
        .acquire_owned()
        .await
        .map_err(|_| "automation queue is closed")?;
    let timezone = ai::local_timezone_offset();
    let attempts = document.retry.max_attempts.max(1);
    let mut attempt = 1;
    loop {
        let Some(run_id) = dystil_automation::enqueue(
            &database,
            &document.name,
            trigger,
            event_key.as_deref(),
            attempt,
        )
        .await
        .map_err(|error| error.to_string())?
        else {
            return Err("event already completed".into());
        };
        let runtime =
            ai_runtime::resolve(&app, &app.state::<RecordingState>(), &database, &timezone)
                .await
                .map_err(|error| error.to_string())?;
        let run = dystil_automation::execute(
            &database,
            &document,
            &run_id,
            &AppRunner {
                runtime,
                app: app.clone(),
            },
        )
        .await
        .map_err(|error| error.to_string())?;
        let retryable = run
            .error_category
            .as_ref()
            .map(|value| value.retryable())
            .unwrap_or(false);
        let view = run_view(run);
        let _ = app.emit("automation-run-updated", &view);
        if view.status == "succeeded" || !retryable || attempt >= attempts {
            return Ok(view);
        }
        attempt += 1;
        tokio::time::sleep(Duration::from_secs(
            document
                .retry
                .backoff_seconds
                .saturating_mul(1 << (attempt - 2).min(8)),
        ))
        .await;
    }
}

fn schedule_due(expression: &str, last: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    if let Some(minutes) = expression
        .strip_prefix("every ")
        .and_then(|value| value.strip_suffix(" minutes"))
        .and_then(|value| value.trim().parse::<i64>().ok())
    {
        return chrono::DateTime::parse_from_rfc3339(last)
            .map(|value| {
                now.signed_duration_since(value.with_timezone(&chrono::Utc))
                    .num_minutes()
                    >= minutes
            })
            .unwrap_or(true);
    }
    cron::Schedule::from_str(expression)
        .ok()
        .and_then(|schedule| {
            chrono::DateTime::parse_from_rfc3339(last)
                .ok()
                .and_then(|last| schedule.after(&last.with_timezone(&chrono::Utc)).next())
        })
        .map(|next| next <= now)
        .unwrap_or(false)
}

async fn poll_once(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<RecordingState>();
    let database = pool(&state).await?;
    let now = chrono::Utc::now();
    for document in dystil_automation::discover(&root())
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|item| item.enabled)
    {
        match &document.trigger {
            dystil_automation::Trigger::Manual => {}
            dystil_automation::Trigger::Schedule { schedule } => {
                let last = dystil_automation::cursor(&database, &document.name, "schedule")
                    .await
                    .map_err(|error| error.to_string())?;
                if let Some(last) = last {
                    if schedule_due(schedule, &last, now) {
                        dystil_automation::set_cursor(
                            &database,
                            &document.name,
                            "schedule",
                            &now.to_rfc3339(),
                        )
                        .await
                        .map_err(|error| error.to_string())?;
                        let app = app.clone();
                        let database = database.clone();
                        tokio::spawn(async move {
                            let _ = run_background(app, database, document, "schedule", None).await;
                        });
                    }
                } else {
                    dystil_automation::set_cursor(
                        &database,
                        &document.name,
                        "schedule",
                        &now.to_rfc3339(),
                    )
                    .await
                    .map_err(|error| error.to_string())?;
                }
            }
            dystil_automation::Trigger::Event { source, .. } if source == "frames" => {
                let newest: Option<i64> = sqlx::query_scalar("SELECT MAX(id) FROM frames")
                    .fetch_one(&database)
                    .await
                    .map_err(|error| error.to_string())?;
                if let Some(newest) = newest {
                    let previous = dystil_automation::cursor(&database, &document.name, source)
                        .await
                        .map_err(|error| error.to_string())?;
                    let newest_text = newest.to_string();
                    if previous.as_deref() != Some(&newest_text) {
                        if previous.is_some() {
                            let key = dystil_automation::new_event_key(source, &newest_text);
                            let app = app.clone();
                            let database = database.clone();
                            let cursor_database = database.clone();
                            let automation_name = document.name.clone();
                            let event_source = source.clone();
                            let event_cursor = newest_text.clone();
                            tokio::spawn(async move {
                                if let Ok(run) =
                                    run_background(app, database, document, "event", Some(key))
                                        .await
                                {
                                    if run.status == "succeeded" {
                                        let _ = dystil_automation::set_cursor(
                                            &cursor_database,
                                            &automation_name,
                                            &event_source,
                                            &event_cursor,
                                        )
                                        .await;
                                    }
                                }
                            });
                        } else {
                            dystil_automation::set_cursor(
                                &database,
                                &document.name,
                                source,
                                &newest_text,
                            )
                            .await
                            .map_err(|error| error.to_string())?;
                        }
                    }
                }
            }
            dystil_automation::Trigger::Event { source, .. } => {
                tracing::warn!(%source,"automation event source is not registered")
            }
        }
    }
    Ok(())
}

pub(crate) fn start_manager(app: AppHandle) {
    let mut task = manager_task()
        .lock()
        .expect("automation manager lock poisoned");
    if task.is_some() {
        return;
    }
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(());
    *task = Some(stop_tx);
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = poll_once(&app).await {
                tracing::debug!(%error,"automation manager waiting for local database");
            }
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(15)) => {}
                _ = stop_rx.changed() => break,
            }
        }
    });
}

pub(crate) fn stop_manager() {
    if let Some(stop) = manager_task()
        .lock()
        .expect("automation manager lock poisoned")
        .take()
    {
        let _ = stop.send(());
    }
}

#[tauri::command]
#[specta::specta]
pub async fn automation_draft(
    request: String,
    app: AppHandle,
    state: State<'_, RecordingState>,
) -> Result<Vec<AutomationDraftView>, String> {
    require_local_automation()?;
    if request.trim().is_empty() || request.len() > 4000 {
        return Err("automation request must be between 1 and 4000 characters".into());
    }
    let batch_id = uuid::Uuid::new_v4().to_string();
    let directory = drafts_root().join(&batch_id);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let database = pool(&state).await?;
    let timezone = ai::local_timezone_offset();
    let runtime = ai_runtime::resolve(&app, &state, &database, &timezone)
        .await
        .map_err(|error| error.to_string())?;
    let suggestions = [
        "suggest",
        "recommend",
        "opportunit",
        "what can",
        "which automation",
    ]
    .iter()
    .any(|needle| request.to_ascii_lowercase().contains(needle));
    let count = if suggestions { 3 } else { 1 };
    let prompt=format!("Use Dystil retrieval tools if captured work is relevant. Create {count} distinct, useful Dystil automation proposal(s) for this user request: {request}\nWrite complete definitions to ./proposal-1.md through ./proposal-{count}. Each must use YAML frontmatter with a lowercase kebab-case name, title, enabled: false, one trigger (manual, schedule with schedule, or event with source), retry.max_attempts, then a Markdown instruction body. Include Dystil memory instructions that read and update ./memory.md. Ground suggestions in actual recurring evidence when available. Do not run any proposal.");
    let (tx, mut rx) = tokio::sync::mpsc::channel::<dystil_ai::AiRuntimeEvent>(128);
    let app_events = app.clone();
    let draft_id = batch_id.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = app_events.emit(
                "automation-draft-event",
                serde_json::json!({"draftId":draft_id,"event":event}),
            );
        }
    });
    runtime
        .run_automation(
            dystil_ai::AiAutomationRequest {
                prompt,
                working_directory: directory.clone(),
                timeout: Duration::from_secs(300),
            },
            tx,
        )
        .await
        .map_err(|error| error.to_string())?;
    let mut drafts = Vec::new();
    for index in 1..=count {
        let source = directory.join(format!("proposal-{index}.md"));
        let markdown = std::fs::read_to_string(&source)
            .map_err(|_| format!("AI did not create proposal-{index}.md"))?;
        let id = uuid::Uuid::new_v4().to_string();
        let target = drafts_root().join(&id);
        std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        let path = target.join("automation.md");
        std::fs::write(&path, &markdown).map_err(|error| error.to_string())?;
        let parsed =
            AutomationDocument::parse(&path, &markdown).map_err(|error| error.to_string())?;
        drafts.push(AutomationDraftView {
            id,
            request: request.clone(),
            markdown,
            automation: to_view(&parsed),
        });
    }
    let _ = std::fs::remove_dir_all(directory);
    Ok(drafts)
}

#[tauri::command]
#[specta::specta]
pub async fn automation_save_draft(draft_id: String) -> Result<AutomationView, String> {
    if !draft_id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err("invalid draft id".into());
    }
    let path = drafts_root().join(&draft_id).join("automation.md");
    let markdown = std::fs::read_to_string(path).map_err(|error| error.to_string())?;
    let document =
        dystil_automation::create(&root(), &markdown).map_err(|error| error.to_string())?;
    Ok(to_view(&document))
}
