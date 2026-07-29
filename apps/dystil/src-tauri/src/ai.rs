//! Tauri-facing provider and MCP commands.
//!
//! Provider OAuth is intentionally delegated to the provider's own official
//! runtime. Dystil never stores provider tokens.

use crate::recording::RecordingState;
use chrono::{DateTime, FixedOffset, Local, Offset};
use dystil_ai::{build_daily_context, AiError, CliProvider, DailyUpdate, ProviderKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::OnceLock;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

static CLAUDE_LOGIN_PROCESS: OnceLock<Mutex<Option<Child>>> = OnceLock::new();
static CODEX_LOGIN_PROCESS: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

const DYSTIL_CODEX_GUIDANCE_START: &str = "<!-- dystil-mcp-guidance:start -->";
const DYSTIL_CODEX_GUIDANCE_END: &str = "<!-- dystil-mcp-guidance:end -->";
const DYSTIL_CODEX_GUIDANCE: &str = r#"<!-- dystil-mcp-guidance:start -->
## Dystil personal activity

For questions about my past desktop activity—what I did or worked on, dates, timestamps,
applications, files, or prior work context—use the Dystil MCP tools before shell, Git, or
filesystem searches. Start with work cards; if they are insufficient, use Dystil's sanitized
activity search and bounded context. Use shell or Git for explicit codebase or current-file
questions. Ground answers in returned Dystil evidence and say when it is insufficient.
<!-- dystil-mcp-guidance:end -->"#;

fn claude_login_process() -> &'static Mutex<Option<Child>> {
    CLAUDE_LOGIN_PROCESS.get_or_init(|| Mutex::new(None))
}

fn codex_login_process() -> &'static Mutex<Option<Child>> {
    CODEX_LOGIN_PROCESS.get_or_init(|| Mutex::new(None))
}

fn external_codex_home() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("CODEX_HOME").map(PathBuf::from) {
        return Ok(path);
    }
    dirs::home_dir()
        .map(|home| home.join(".codex"))
        .ok_or_else(|| "could not find the home directory for Codex guidance".into())
}

fn codex_guidance_path(codex_home: &std::path::Path) -> std::path::PathBuf {
    let override_path = codex_home.join("AGENTS.override.md");
    match std::fs::read_to_string(&override_path) {
        Ok(existing) if !existing.trim().is_empty() => override_path,
        _ => codex_home.join("AGENTS.md"),
    }
}

fn append_dystil_codex_guidance(codex_home: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(codex_home)
        .map_err(|error| format!("could not create Codex guidance directory: {error}"))?;
    let path = codex_guidance_path(codex_home);
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(format!("could not read Codex guidance: {error}")),
    };
    let has_start = existing.contains(DYSTIL_CODEX_GUIDANCE_START);
    let has_end = existing.contains(DYSTIL_CODEX_GUIDANCE_END);
    if has_start && has_end {
        return Ok(());
    }
    if has_start || has_end {
        return Err(format!(
            "Codex guidance at {} contains an incomplete Dystil block; remove it or complete it, then try again",
            path.display()
        ));
    }
    let separator = if existing.trim().is_empty() {
        ""
    } else {
        "\n\n"
    };
    std::fs::write(
        &path,
        format!("{existing}{separator}{DYSTIL_CODEX_GUIDANCE}\n"),
    )
    .map_err(|error| format!("could not update Codex guidance: {error}"))
}

fn watch_codex_login(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        for _ in 0..300 {
            let completed = {
                let mut active = codex_login_process().lock().await;
                match active.as_mut() {
                    Some(child) => match child.try_wait() {
                        Ok(Some(_)) => {
                            active.take();
                            true
                        }
                        Ok(None) => false,
                        Err(_) => {
                            active.take();
                            true
                        }
                    },
                    None => true,
                }
            };
            let status = ai_provider_status("codex".into()).await;
            let authenticated = status
                .as_ref()
                .ok()
                .and_then(|value| value.authenticated)
                .unwrap_or(false);
            if authenticated || completed {
                let _ = app_handle.emit("ai-provider-login-updated", status);
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let _ = app_handle.emit(
            "ai-provider-login-updated",
            serde_json::json!({"error": "Codex sign-in timed out. Try again."}),
        );
    });
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderStatusView {
    pub provider: String,
    pub state: String,
    pub installed_version: Option<String>,
    pub authenticated: Option<bool>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiProviderModelView {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct AiDailyUpdateView {
    pub provider: String,
    pub runtime_version: Option<String>,
    pub elapsed_ms: u64,
    pub update: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct McpConnectionStatus {
    pub connected: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ExternalMcpSetupView {
    pub client: String,
    pub detail: String,
}

fn provider_error_kind(error: &AiError) -> &'static str {
    match error {
        AiError::Timeout => "timeout",
        AiError::LoginRequired => "login_required",
        AiError::Process(_) => "process_failed",
        AiError::InvalidOutput(_) => "invalid_output",
        AiError::Io(_) => "filesystem",
        AiError::Date(_) | AiError::NoCards | AiError::Storage(_) => "context",
    }
}

pub(crate) fn provider_kind(provider: &str) -> Result<ProviderKind, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" | "chatgpt" | "chatgpt_plus" => Ok(ProviderKind::Codex),
        "claude" | "claude_code" => Ok(ProviderKind::Claude),
        _ => Err("provider must be codex or claude".into()),
    }
}

fn runtime_root() -> Result<PathBuf, String> {
    Ok(crate::dystil_paths::data_dir().join("ai-runtimes"))
}

fn executable_override(provider: &ProviderKind) -> Option<PathBuf> {
    let key = match provider {
        ProviderKind::Codex => "DYSTIL_AI_CODEX_EXECUTABLE",
        ProviderKind::Claude => "DYSTIL_AI_CLAUDE_EXECUTABLE",
    };
    std::env::var_os(key)
        .map(PathBuf::from)
        .filter(|path| path.is_file())
}

fn provider_package(provider: &ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Codex => "@openai/codex",
        ProviderKind::Claude => "@anthropic-ai/claude-code",
    }
}

/// Finds Dystil's bundled Bun sidecar. An override exists solely for local
/// adapter tests; production never relies on a user-global Bun installation.
fn bundled_bun() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DYSTIL_AI_BUN_EXECUTABLE").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
    }
    let file = format!(
        "bun-{}{}",
        target_triple(),
        if cfg!(target_os = "windows") {
            ".exe"
        } else {
            ""
        }
    );
    let mut roots = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots
        .into_iter()
        .map(|root| root.join(&file))
        .find(|path| path.is_file())
        .ok_or_else(|| format!("Dystil's bundled Bun sidecar ({file}) is unavailable"))
}

fn bun_install_root(provider: &ProviderKind) -> Result<PathBuf, String> {
    Ok(runtime_root()?
        .join(provider.slug())
        .join("bun")
        .join(target_triple()))
}

fn bun_provider_executable(provider: &ProviderKind) -> Result<PathBuf, String> {
    Ok(bun_install_root(provider)?
        .join("bin")
        .join(provider.executable_name()))
}

fn managed_provider_environment(provider: &ProviderKind) -> Result<Vec<(String, String)>, String> {
    // Codex creates app-server and credential state below CODEX_HOME. Keeping
    // this under Dystil's writable runtime root avoids relying on ~/.codex,
    // which can be unavailable to a packaged desktop process.
    let state_dir = bun_install_root(provider)?.join("state");
    std::fs::create_dir_all(&state_dir).map_err(|error| error.to_string())?;
    Ok(match provider {
        ProviderKind::Codex => vec![(
            "CODEX_HOME".into(),
            state_dir.to_string_lossy().into_owned(),
        )],
        ProviderKind::Claude => vec![(
            "CLAUDE_CONFIG_DIR".into(),
            state_dir.to_string_lossy().into_owned(),
        )],
    })
}

fn managed_bun_runtime(provider: &ProviderKind) -> Result<Option<CliProvider>, String> {
    let executable = bun_provider_executable(provider)?;
    if !executable.is_file() {
        return Ok(None);
    }
    let package_json = bun_install_root(provider)?
        .join("install")
        .join("global")
        .join("node_modules")
        .join(provider_package(provider))
        .join("package.json");
    let version = std::fs::read_to_string(package_json)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|json| {
            json.get("version")
                .and_then(|value| value.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "managed-bun".into());
    Ok(Some(CliProvider {
        provider: provider.clone(),
        executable,
        runtime_version: Some(version),
        environment: managed_provider_environment(provider)?,
        mcp_server: None,
    }))
}

/// Installs an official CLI package into a Dystil-owned Bun prefix. Bun's
/// package integrity verification applies; no user-global package manager or
/// shell is used.
async fn emit_install_output(
    app_handle: AppHandle,
    provider: String,
    stream: &'static str,
    reader: impl tokio::io::AsyncRead + Unpin,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let message = dystil_redact::sanitize_text(&line);
        if message.trim().is_empty() {
            continue;
        }
        let _ = app_handle.emit(
            "ai-provider-install-progress",
            serde_json::json!({"provider": provider, "phase": "output", "stream": stream, "message": message}),
        );
    }
}

async fn install_with_bundled_bun(
    app_handle: &AppHandle,
    provider: &ProviderKind,
) -> Result<CliProvider, String> {
    let bun = bundled_bun()?;
    let install_root = bun_install_root(provider)?;
    std::fs::create_dir_all(&install_root).map_err(|error| error.to_string())?;
    let package = std::env::var(match provider {
        ProviderKind::Codex => "DYSTIL_AI_CODEX_PACKAGE",
        ProviderKind::Claude => "DYSTIL_AI_CLAUDE_PACKAGE",
    })
    .unwrap_or_else(|_| provider_package(provider).into());
    let mut child = Command::new(&bun)
        .args(["add", "--global", "--exact", &package])
        .env("BUN_INSTALL", &install_root)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "missing Bun install stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "missing Bun install stderr".to_string())?;
    let stdout_task = tokio::spawn(emit_install_output(
        app_handle.clone(),
        provider.slug().into(),
        "stdout",
        stdout,
    ));
    let stderr_task = tokio::spawn(emit_install_output(
        app_handle.clone(),
        provider.slug().into(),
        "stderr",
        stderr,
    ));
    let status = timeout(Duration::from_secs(180), child.wait())
        .await
        .map_err(|_| format!("{} installation timed out", provider.slug()))?
        .map_err(|error| error.to_string())?;
    let _ = stdout_task.await;
    let _ = stderr_task.await;
    if !status.success() {
        return Err(dystil_redact::sanitize_text(&format!(
            "{} installation failed; see the setup notice for the package manager output.",
            provider.slug()
        )));
    }
    if matches!(provider, ProviderKind::Claude) {
        let package_dir = install_root
            .join("install")
            .join("global")
            .join("node_modules")
            .join(provider_package(provider));
        let postinstall = package_dir.join("install.cjs");
        let output = timeout(
            Duration::from_secs(60),
            Command::new(&bun)
                .arg(&postinstall)
                .current_dir(&package_dir)
                .env("BUN_INSTALL", &install_root)
                .env("NO_COLOR", "1")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output(),
        )
        .await
        .map_err(|_| "claude native runtime setup timed out".to_string())?
        .map_err(|error| error.to_string())?;
        if !output.status.success() {
            let detail = String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1200)
                .collect::<String>();
            return Err(dystil_redact::sanitize_text(&format!(
                "claude native runtime setup failed: {detail}"
            )));
        }
    }
    managed_bun_runtime(provider)?.ok_or_else(|| {
        format!(
            "{} was installed but its executable was not found",
            provider.slug()
        )
    })
}

pub(crate) fn provider_runtime(provider: ProviderKind) -> Result<CliProvider, String> {
    if let Some(executable) = executable_override(&provider) {
        return Ok(CliProvider {
            provider,
            executable,
            runtime_version: Some("developer-override".into()),
            environment: Vec::new(),
            mcp_server: None,
        });
    }
    if let Some(runtime) = managed_bun_runtime(&provider)? {
        return Ok(runtime);
    }
    Err(format!(
        "{} is not installed; use Dystil's Install button",
        provider.slug()
    ))
}

pub(crate) async fn capture_pool(state: &RecordingState) -> Result<sqlx::SqlitePool, String> {
    state
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.db.pool.clone())
        .ok_or_else(|| "local capture database is not ready".to_string())
}

async fn capture_database_path(state: &RecordingState) -> Result<PathBuf, String> {
    state
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.data_dir.join("db.sqlite"))
        .ok_or_else(|| "local capture database is not ready".to_string())
}

/// Read runtime installation and official runtime authentication state.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_status(provider: String) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    if let Some(executable) = executable_override(&provider) {
        let runtime = CliProvider {
            provider: provider.clone(),
            executable,
            runtime_version: Some("developer-override".into()),
            environment: Vec::new(),
            mcp_server: None,
        };
        return Ok(AiProviderStatusView {
            provider: provider.slug().into(),
            state: "ready".into(),
            installed_version: runtime.runtime_version.clone(),
            authenticated: runtime.authenticated().await.ok(),
            detail: Some("using developer runtime override".into()),
        });
    }
    if let Some(runtime) = managed_bun_runtime(&provider)? {
        if let Err(error) = runtime.healthy().await {
            return Ok(AiProviderStatusView {
                provider: provider.slug().into(),
                state: "repairRequired".into(),
                installed_version: runtime.runtime_version.clone(),
                authenticated: Some(false),
                detail: Some(dystil_redact::sanitize_text(&error.to_string())),
            });
        }
        return Ok(AiProviderStatusView {
            provider: provider.slug().into(),
            state: "ready".into(),
            installed_version: runtime.runtime_version.clone(),
            authenticated: runtime.authenticated().await.ok(),
            detail: Some("installed privately by Dystil".into()),
        });
    }
    Ok(AiProviderStatusView {
        provider: provider.slug().into(),
        state: "notInstalled".into(),
        installed_version: None,
        authenticated: None,
        detail: Some(format!(
            "Click Install to set up the official {} CLI privately with Dystil.",
            provider.slug()
        )),
    })
}

/// List models exposed by the managed provider. Codex is discovered from the
/// signed-in account; Claude Code exposes its provider-maintained aliases.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_models(provider: String) -> Result<Vec<AiProviderModelView>, String> {
    let runtime = provider_runtime(provider_kind(&provider)?)?;
    runtime
        .available_models()
        .await
        .map(|models| {
            models
                .into_iter()
                .map(|model| AiProviderModelView {
                    id: model.id,
                    display_name: model.display_name,
                    description: model.description,
                    is_default: model.is_default,
                })
                .collect()
        })
        .map_err(|error| error.to_string())
}

/// Install the official CLI privately with Dystil's bundled Bun.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_install(
    app_handle: AppHandle,
    provider: String,
) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    let _ = app_handle.emit(
        "ai-provider-install-progress",
        serde_json::json!({"provider": provider.slug(), "phase": "resolving"}),
    );
    let _ = app_handle.emit(
        "ai-provider-install-progress",
        serde_json::json!({"provider": provider.slug(), "phase": "downloading"}),
    );
    if let Err(error) = install_with_bundled_bun(&app_handle, &provider).await {
        warn!(provider = provider.slug(), %error, "managed AI provider installation failed");
        return Err(error);
    }
    let _ = app_handle.emit(
        "ai-provider-install-progress",
        serde_json::json!({"provider": provider.slug(), "phase": "verifying"}),
    );
    ai_provider_status(provider.slug().into()).await
}

/// Register Dystil's read-only stdio sidecar in the user's own Codex CLI or
/// Claude Code configuration. This is intentionally separate from Dystil's
/// managed chat runtime and is only called after explicit UI consent.
#[tauri::command]
#[specta::specta]
pub async fn external_mcp_add(
    app_handle: AppHandle,
    state: State<'_, RecordingState>,
    client: String,
) -> Result<ExternalMcpSetupView, String> {
    let client = match client.as_str() {
        "codex" | "claude" => client,
        _ => return Err("client must be codex or claude".into()),
    };
    let sidecar = mcp_binary(&app_handle)?;
    let database = capture_database_path(&state).await?;
    let timezone = local_timezone_offset();
    let mut command = Command::new(&client);
    if client == "codex" {
        command.args(["mcp", "add", "dystil", "--"]).arg(&sidecar);
    } else {
        command
            .args([
                "mcp",
                "add",
                "--scope",
                "user",
                "--transport",
                "stdio",
                "dystil",
                "--",
            ])
            .arg(&sidecar);
    }
    command
        .arg("--database")
        .arg(&database)
        .args(["--access", "activity", "--timezone", &timezone])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = timeout(Duration::from_secs(30), command.output())
        .await
        .map_err(|_| format!("{client} did not finish configuring Dystil within 30 seconds"))?
        .map_err(|error| format!("could not start the external {client} CLI: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = dystil_redact::sanitize_text(detail.trim());
        warn!(client, "external MCP setup failed");
        return Err(if detail.is_empty() {
            format!("{client} could not add Dystil. Ensure its CLI is installed and try again.")
        } else {
            detail
        });
    }
    if client == "codex" {
        append_dystil_codex_guidance(&external_codex_home()?).map_err(|error| {
            format!(
                "Dystil was added to Codex, but its activity guidance could not be added: {error}"
            )
        })?;
    }
    info!(client, "external MCP sidecar added");
    Ok(ExternalMcpSetupView {
        client: client.clone(),
        detail: if client == "codex" {
            "Dystil was added to Codex, and Codex guidance now prefers it for past-work questions. Start a new Codex session to use it.".into()
        } else {
            format!("Dystil was added to {client}. Start a new {client} session to use it.")
        },
    })
}

/// Start the provider-owned browser sign-in. No OAuth token passes through Dystil.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_login(app_handle: AppHandle, provider: String) -> Result<String, String> {
    let provider = provider_kind(&provider)?;
    let runtime = provider_runtime(provider.clone())?;
    match provider {
        ProviderKind::Codex => {
            let child = runtime
                .begin_login()
                .await
                .map_err(|error| error.to_string())?;
            {
                let mut active = codex_login_process().lock().await;
                if let Some(mut previous) = active.take() {
                    let _ = previous.start_kill();
                }
                *active = Some(child);
            }
            watch_codex_login(app_handle);
            Ok("browserCallback".into())
        }
        ProviderKind::Claude => {
            let child = runtime
                .begin_interactive_login()
                .await
                .map_err(|error| error.to_string())?;
            let mut active = claude_login_process().lock().await;
            if let Some(mut previous) = active.take() {
                let _ = previous.start_kill();
            }
            *active = Some(child);
            Ok("codeRequired".into())
        }
    }
}

/// Pass Claude Code the short-lived authorization code shown by its provider
/// page. The code remains in memory and is never persisted by Dystil.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_complete_claude_login(
    authorization_code: String,
) -> Result<AiProviderStatusView, String> {
    let authorization_code = authorization_code.trim();
    if authorization_code.is_empty() || authorization_code.len() > 4096 {
        return Err("Paste the authorization code shown by Claude.".into());
    }
    let mut child =
        claude_login_process().lock().await.take().ok_or_else(|| {
            "Start Claude Code sign-in again before submitting a code.".to_string()
        })?;
    if let Some(status) = child.try_wait().map_err(|error| error.to_string())? {
        if status.success() {
            return ai_provider_status("claude".into()).await;
        }
        return Err("Claude Code sign-in ended before the code was submitted.".into());
    }
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Claude Code sign-in input is unavailable.".to_string())?;
    stdin
        .write_all(format!("{authorization_code}\n").as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    stdin.shutdown().await.map_err(|error| error.to_string())?;
    let output = timeout(Duration::from_secs(60), child.wait_with_output())
        .await
        .map_err(|_| "Claude Code sign-in timed out after the code was submitted.".to_string())?
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let detail = [output.stderr, output.stdout].concat();
        let detail = String::from_utf8_lossy(&detail)
            .lines()
            .take(6)
            .collect::<Vec<_>>()
            .join(" ");
        let message = if detail.is_empty() {
            "Claude Code rejected the authorization code.".to_string()
        } else {
            detail
        };
        return Err(dystil_redact::sanitize_text(&message));
    }
    ai_provider_status("claude".into()).await
}

/// Verify the official runtime and its account session without invoking a model.
///
/// A model request is intentionally not part of setup: remote queue and cold
/// start latency make it a poor connection diagnostic. The daily-update action
/// is the first request that sends derived work cards to the selected provider.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_test(provider: String) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    let runtime = provider_runtime(provider)?;
    info!(
        provider = runtime.provider.slug(),
        "starting AI provider connection test"
    );
    match runtime.authenticated().await {
        Ok(true) => {
            info!(
                provider = runtime.provider.slug(),
                "AI provider connection test completed"
            );
            Ok(AiProviderStatusView {
                provider: runtime.provider.slug().into(),
                state: "ready".into(),
                installed_version: runtime.runtime_version,
                authenticated: Some(true),
                detail: Some("Official CLI and account session are ready.".into()),
            })
        }
        Ok(false) => Err("provider login is required".into()),
        Err(error) => {
            warn!(
                provider = runtime.provider.slug(),
                reason = provider_error_kind(&error),
                "AI provider connection test failed"
            );
            Err(error.to_string())
        }
    }
}

/// Generate a manager-ready daily update from local derived work cards.
#[tauri::command]
#[specta::specta]
pub async fn ai_generate_daily_update(
    provider: String,
    local_date: String,
    timezone: String,
    model: Option<String>,
    state: State<'_, RecordingState>,
) -> Result<AiDailyUpdateView, String> {
    let runtime = provider_runtime(provider_kind(&provider)?)?;
    let model = model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(model) = model {
        let valid = model.len() <= 80
            && model
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
        if !valid {
            return Err("invalid provider model identifier".into());
        }
    }
    info!(
        provider = runtime.provider.slug(),
        local_date,
        model = model.unwrap_or("provider-default"),
        "AI daily update requested"
    );
    let pool = capture_pool(&state).await.map_err(|error| {
        warn!(
            provider = runtime.provider.slug(),
            reason = "capture_database_unavailable",
            "AI daily update stopped before provider launch"
        );
        error
    })?;
    let bundle = build_daily_context(&pool, &local_date, &timezone)
        .await
        .map_err(|error| {
            warn!(
                provider = runtime.provider.slug(),
                reason = provider_error_kind(&error),
                local_date,
                "AI daily update stopped before provider launch"
            );
            error.to_string()
        })?;
    info!(
        provider = runtime.provider.slug(),
        card_count = bundle.cards.len(),
        context_bytes = bundle
            .as_prompt_json()
            .map(|context| context.len())
            .unwrap_or_default(),
        "launching AI provider with derived work cards"
    );
    let result = runtime.run_daily_update_with_model(&bundle, model).await;
    match result {
        Ok(run) => {
            info!(
                provider = run.provider.slug(),
                elapsed_ms = run.elapsed_ms,
                "AI daily update completed"
            );
            into_view(run)
        }
        Err(error) => {
            // Keep logs privacy-preserving: this deliberately omits the provider's
            // stderr because it can include user-provided context.
            warn!(
                provider = runtime.provider.slug(),
                reason = provider_error_kind(&error),
                "AI daily update failed"
            );
            Err(error.to_string())
        }
    }
}

fn into_view(run: dystil_ai::ProviderRun) -> Result<AiDailyUpdateView, String> {
    Ok(AiDailyUpdateView {
        provider: run.provider.slug().into(),
        runtime_version: run.runtime_version,
        elapsed_ms: run.elapsed_ms,
        update: serde_json::to_value(run.update).map_err(|error| error.to_string())?,
    })
}

pub(crate) fn mcp_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DYSTIL_MCP_BINARY").map(PathBuf::from) {
        if path.is_file() {
            info!(
                path = %path.display(),
                source = "environment_override",
                "resolved Dystil MCP sidecar"
            );
            return Ok(path);
        }
    }
    let extension = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    let names = [
        format!("dystil-mcp-{}{}", target_triple(), extension),
        format!("dystil-mcp{extension}"),
    ];
    let mut roots = Vec::new();
    // `predev` builds the target-specific sidecar into CARGO_MANIFEST_DIR.
    // Prefer it during development so a stale Cargo target/debug binary cannot
    // silently shadow the freshly packaged executable.
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    roots.push(
        app.path()
            .resource_dir()
            .map_err(|error| error.to_string())?,
    );
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Some(path) = find_mcp_binary(&roots, &names) {
        info!(
            path = %path.display(),
            "resolved Dystil MCP sidecar"
        );
        return Ok(path);
    }
    Err("Dystil MCP sidecar is not bundled yet".into())
}

fn find_mcp_binary(roots: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    for root in roots {
        for name in names {
            for candidate in [root.join(name), root.join("binaries").join(name)] {
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub(crate) fn local_timezone_offset() -> String {
    let seconds = Local::now().offset().fix().local_minus_utc();
    let sign = if seconds < 0 { '-' } else { '+' };
    let minutes = seconds.unsigned_abs() / 60;
    format!("{sign}{:02}:{:02}", minutes / 60, minutes % 60)
}

pub(crate) fn local_date_for_timestamp(timestamp: &str, timezone: &str) -> String {
    timestamp
        .parse::<DateTime<FixedOffset>>()
        .ok()
        .zip(timezone.parse::<FixedOffset>().ok())
        .map(|(timestamp, offset)| timestamp.with_timezone(&offset).date_naive().to_string())
        .unwrap_or_else(|| timestamp.get(..10).unwrap_or(timestamp).to_string())
}

pub(crate) async fn internal_mcp_server(
    app: &AppHandle,
    state: &RecordingState,
    timezone: &str,
) -> Result<dystil_ai::McpServerConfig, String> {
    Ok(dystil_ai::McpServerConfig {
        command: mcp_binary(app)?,
        args: vec![
            "--database".into(),
            capture_database_path(state)
                .await?
                .to_string_lossy()
                .into_owned(),
            "--access".into(),
            "activity".into(),
            "--timezone".into(),
            timezone.into(),
            "--max-calls".into(),
            "6".into(),
        ],
    })
}

fn target_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        _ => "unknown-target",
    }
}

async fn claude_mcp_command(
    runtime: &CliProvider,
    args: &[String],
) -> Result<(bool, String), String> {
    let output = Command::new(&runtime.executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|error| error.to_string())?;
    let text: String = String::from_utf8_lossy(if output.status.success() {
        &output.stdout
    } else {
        &output.stderr
    })
    .chars()
    .take(1200)
    .collect();
    Ok((output.status.success(), dystil_redact::sanitize_text(&text)))
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_connection_status(
    state: State<'_, RecordingState>,
) -> Result<McpConnectionStatus, String> {
    let runtime = provider_runtime(ProviderKind::Claude)?;
    let (connected, detail) =
        claude_mcp_command(&runtime, &["mcp".into(), "get".into(), "dystil".into()]).await?;
    // Touch the database path too so a stale registration is not presented as usable.
    let _ = capture_database_path(&state).await?;
    Ok(McpConnectionStatus { connected, detail })
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_connect(
    app: AppHandle,
    state: State<'_, RecordingState>,
) -> Result<McpConnectionStatus, String> {
    let runtime = provider_runtime(ProviderKind::Claude)?;
    let binary = mcp_binary(&app)?;
    let database = capture_database_path(&state).await?;
    let args = vec![
        "mcp".into(),
        "add".into(),
        "--transport".into(),
        "stdio".into(),
        "--scope".into(),
        "user".into(),
        "dystil".into(),
        "--".into(),
        binary.to_string_lossy().into_owned(),
        "--database".into(),
        database.to_string_lossy().into_owned(),
    ];
    let (success, detail) = claude_mcp_command(&runtime, &args).await?;
    if !success {
        return Err(detail);
    }
    Ok(McpConnectionStatus {
        connected: true,
        detail,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn mcp_disconnect() -> Result<McpConnectionStatus, String> {
    let runtime = provider_runtime(ProviderKind::Claude)?;
    let (success, detail) =
        claude_mcp_command(&runtime, &["mcp".into(), "remove".into(), "dystil".into()]).await?;
    if !success {
        return Err(detail);
    }
    Ok(McpConnectionStatus {
        connected: false,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        append_dystil_codex_guidance, codex_guidance_path, find_mcp_binary,
        local_date_for_timestamp, DYSTIL_CODEX_GUIDANCE_START,
    };

    #[test]
    fn citation_date_uses_the_inquiry_timezone() {
        assert_eq!(
            local_date_for_timestamp("2026-07-27T18:36:07+00:00", "+05:30"),
            "2026-07-28"
        );
    }

    #[test]
    fn mcp_resolution_prefers_first_root_and_target_specific_name() {
        let preferred = tempfile::tempdir().unwrap();
        let stale = tempfile::tempdir().unwrap();
        let target_name = "dystil-mcp-test-target".to_string();
        let plain_name = "dystil-mcp".to_string();
        let preferred_binary = preferred.path().join(&target_name);
        let stale_binary = stale.path().join(&plain_name);
        std::fs::write(&preferred_binary, b"fresh").unwrap();
        std::fs::write(&stale_binary, b"stale").unwrap();

        let resolved = find_mcp_binary(
            &[preferred.path().to_path_buf(), stale.path().to_path_buf()],
            &[target_name, plain_name],
        );

        assert_eq!(resolved.as_deref(), Some(preferred_binary.as_path()));
    }

    #[test]
    fn codex_guidance_preserves_existing_instructions_and_is_idempotent() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("AGENTS.md");
        std::fs::write(&path, "# My instructions\n\nKeep answers concise.\n").unwrap();

        append_dystil_codex_guidance(home.path()).unwrap();
        append_dystil_codex_guidance(home.path()).unwrap();

        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.starts_with("# My instructions"));
        assert_eq!(content.matches(DYSTIL_CODEX_GUIDANCE_START).count(), 1);
    }

    #[test]
    fn codex_guidance_uses_existing_override_file() {
        let home = tempfile::tempdir().unwrap();
        let override_path = home.path().join("AGENTS.override.md");
        std::fs::write(&override_path, "# Override\n").unwrap();

        append_dystil_codex_guidance(home.path()).unwrap();

        assert_eq!(codex_guidance_path(home.path()), override_path);
        assert!(std::fs::read_to_string(override_path)
            .unwrap()
            .contains(DYSTIL_CODEX_GUIDANCE_START));
        assert!(!home.path().join("AGENTS.md").exists());
    }
}
