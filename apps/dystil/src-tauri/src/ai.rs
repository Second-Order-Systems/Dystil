//! Tauri-facing provider and MCP commands.
//!
//! Provider OAuth is intentionally delegated to the provider's own official
//! runtime. Dystil never stores provider tokens.

use crate::recording::RecordingState;
use chrono::{DateTime, FixedOffset, Local, Offset};
use dystil_ai::{AiError, CliProvider, ProviderKind};
use dystil_telemetry::{AiErrorKind, AiOperationKind, AiProviderKind, Outcome};
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
filesystem searches. Start with Dystil's activity overview for broad questions, or its exact
activity search for names, messages, tickets, files, and quotes. Use shell or Git for explicit codebase or current-file
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
                let state = app_handle.state::<RecordingState>();
                let result: Result<(), String> = if authenticated {
                    Ok(())
                } else {
                    Err("Codex sign-in ended without an authenticated session".into())
                };
                record_ai_result(&state, &ProviderKind::Codex, AiOperationKind::SignIn, &result)
                    .await;
                let _ = app_handle.emit("ai-provider-login-updated", status);
                return;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let state = app_handle.state::<RecordingState>();
        let result: Result<(), String> = Err("Codex sign-in timed out".into());
        record_ai_result(&state, &ProviderKind::Codex, AiOperationKind::SignIn, &result).await;
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
    }
}

fn telemetry_provider(provider: &ProviderKind) -> AiProviderKind {
    match provider {
        ProviderKind::Codex => AiProviderKind::Codex,
        ProviderKind::Claude => AiProviderKind::Claude,
    }
}

// Classification happens locally against Dystil-owned boundary messages; the
// message itself is never recorded or exported.
fn telemetry_error_kind(operation: AiOperationKind, error: &str) -> AiErrorKind {
    let error = error.to_ascii_lowercase();
    if error.contains("bundled bun") || error.contains("mcp sidecar") {
        AiErrorKind::SidecarMissing
    } else if error.contains("not installed") || error.contains("executable was not found") {
        AiErrorKind::RuntimeMissing
    } else if error.contains("timed out") {
        AiErrorKind::Timeout
    } else if error.contains("login is required") {
        AiErrorKind::LoginRequired
    } else if error.contains("authorization code") || error.contains("rejected") {
        AiErrorKind::AuthenticationFailed
    } else if error.contains("invalid output") {
        AiErrorKind::InvalidOutput
    } else if error.contains("database") || error.contains("directory") || error.contains("guidance") {
        AiErrorKind::Filesystem
    } else if matches!(operation, AiOperationKind::McpSetup | AiOperationKind::McpConnect)
        && (error.contains("could not start") || error.contains("cli"))
    {
        AiErrorKind::McpClientUnavailable
    } else if matches!(operation, AiOperationKind::McpSetup | AiOperationKind::McpConnect) {
        AiErrorKind::McpRegistrationFailed
    } else {
        AiErrorKind::ProcessFailed
    }
}

async fn record_ai_result<T>(
    state: &RecordingState,
    provider: &ProviderKind,
    operation: AiOperationKind,
    result: &Result<T, String>,
) {
    let telemetry = state
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.telemetry.clone());
    let Some(telemetry) = telemetry else { return };
    let (outcome, error) = match result {
        Ok(_) => (Outcome::Succeeded, AiErrorKind::None),
        Err(error) => (Outcome::Failed, telemetry_error_kind(operation, error)),
    };
    telemetry.record_ai_operation(telemetry_provider(provider), operation, outcome, error);
}

pub(crate) fn provider_kind(provider: &str) -> Result<ProviderKind, String> {
    match provider.trim().to_ascii_lowercase().as_str() {
        "codex" | "chatgpt" | "chatgpt_plus" => Ok(ProviderKind::Codex),
        "claude" | "claude_code" => Ok(ProviderKind::Claude),
        _ => Err("provider must be codex or claude".into()),
    }
}

pub(crate) fn runtime_root() -> Result<PathBuf, String> {
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
pub(crate) fn bundled_bun() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DYSTIL_AI_BUN_EXECUTABLE").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
    }
    let extension = if cfg!(target_os = "windows") {
        ".exe"
    } else {
        ""
    };
    // Tauri consumes the target-qualified source file named in `externalBin`,
    // then installs it under the configured sidecar name (`bun`/`bun.exe`).
    // Keep the qualified name first so development builds still use the freshly
    // staged binary in `src-tauri`.
    let names = [
        format!("bun-{}{}", target_triple(), extension),
        format!("bun{extension}"),
    ];
    let mut roots = Vec::new();
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    find_bundled_binary(&roots, &names).ok_or_else(|| {
        format!(
            "Dystil's bundled Bun sidecar ({}) is unavailable",
            names.join(" or ")
        )
    })
}

fn find_bundled_binary(roots: &[PathBuf], names: &[String]) -> Option<PathBuf> {
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
    state: State<'_, RecordingState>,
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
        let result: Result<AiProviderStatusView, String> = Err(error.clone());
        record_ai_result(&state, &provider, AiOperationKind::Install, &result).await;
        return Err(error);
    }
    let _ = app_handle.emit(
        "ai-provider-install-progress",
        serde_json::json!({"provider": provider.slug(), "phase": "verifying"}),
    );
    let result = ai_provider_status(provider.slug().into()).await;
    record_ai_result(&state, &provider, AiOperationKind::Install, &result).await;
    result
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
    let provider = if client == "codex" { AiProviderKind::Codex } else { AiProviderKind::Claude };
    let sidecar = match mcp_binary(&app_handle) {
        Ok(value) => value,
        Err(error) => {
            let result: Result<(), String> = Err(error.clone());
            let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
            record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
            return Err(error);
        }
    };
    let database = match capture_database_path(&state).await {
        Ok(value) => value,
        Err(error) => {
            let result: Result<(), String> = Err(error.clone());
            let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
            record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
            return Err(error);
        }
    };
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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let output = match timeout(Duration::from_secs(30), command.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            let error = format!("could not start the external {client} CLI: {error}");
            let result: Result<(), String> = Err(error.clone());
            let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
            record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
            return Err(error);
        }
        Err(_) => {
            let error = format!("{client} did not finish configuring Dystil within 30 seconds");
            let result: Result<(), String> = Err(error.clone());
            let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
            record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
            return Err(error);
        }
    };
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = dystil_redact::sanitize_text(detail.trim());
        warn!(client, "external MCP setup failed");
        let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
        let result: Result<(), String> = Err("MCP registration failed".into());
        record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
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
    let provider_kind = if matches!(provider, AiProviderKind::Codex) { ProviderKind::Codex } else { ProviderKind::Claude };
    let result: Result<(), String> = Ok(());
    record_ai_result(&state, &provider_kind, AiOperationKind::McpSetup, &result).await;
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

/// Sign out of Dystil's isolated provider session without touching a user's
/// separately installed global Codex or Claude Code credentials.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_logout(
    app_handle: AppHandle,
    provider: String,
) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    match provider {
        ProviderKind::Codex => {
            if let Some(mut child) = codex_login_process().lock().await.take() {
                let _ = child.start_kill();
            }
        }
        ProviderKind::Claude => {
            if let Some(mut child) = claude_login_process().lock().await.take() {
                let _ = child.start_kill();
            }
        }
    }
    let runtime = provider_runtime(provider.clone())?;
    runtime.logout().await.map_err(|error| error.to_string())?;
    let status = ai_provider_status(provider.slug().into()).await?;
    if status.authenticated == Some(true) {
        return Err(format!(
            "{} still reports an authenticated session after logout",
            provider.slug()
        ));
    }
    let _ = app_handle.emit("ai-provider-login-updated", &status);
    Ok(status)
}

/// Pass Claude Code the short-lived authorization code shown by its provider
/// page. The code remains in memory and is never persisted by Dystil.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_complete_claude_login(
    state: State<'_, RecordingState>,
    authorization_code: String,
) -> Result<AiProviderStatusView, String> {
    let result = async {
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
    .await;
    record_ai_result(&state, &ProviderKind::Claude, AiOperationKind::SignIn, &result).await;
    result
}

/// Verify the official runtime and its account session without invoking a model.
///
/// A model request is intentionally not part of setup: remote queue and cold
/// start latency make it a poor connection diagnostic. The first inquiry is
/// the first request that lets the selected runtime query activity evidence.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_test(
    state: State<'_, RecordingState>,
    provider: String,
) -> Result<AiProviderStatusView, String> {
    ai_provider_test_with_telemetry(Some(&state), provider).await
}

async fn ai_provider_test_with_telemetry(
    state: Option<&RecordingState>,
    provider: String,
) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    let runtime = match provider_runtime(provider.clone()) {
        Ok(runtime) => runtime,
        Err(error) => {
            let result: Result<AiProviderStatusView, String> = Err(error.clone());
            if let Some(state) = state {
                record_ai_result(state, &provider, AiOperationKind::ConnectionTest, &result).await;
            }
            return Err(error);
        }
    };
    info!(
        provider = runtime.provider.slug(),
        "starting AI provider connection test"
    );
    let result = match runtime.authenticated().await {
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
    };
    if let Some(state) = state {
        record_ai_result(state, &provider, AiOperationKind::ConnectionTest, &result).await;
    }
    result
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
    _timezone: &str,
) -> Result<dystil_ai::McpServerConfig, String> {
    Ok(dystil_ai::McpServerConfig {
        command: mcp_binary(app)?,
        args: vec![
            "--database".into(),
            capture_database_path(state)
                .await?
                .to_string_lossy()
                .into_owned(),
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
    let result = async {
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
    .await;
    record_ai_result(&state, &ProviderKind::Claude, AiOperationKind::McpConnect, &result).await;
    result
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
        append_dystil_codex_guidance, codex_guidance_path, find_bundled_binary, find_mcp_binary,
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
    fn bundled_bun_resolution_accepts_tauri_installed_name() {
        let root = tempfile::tempdir().unwrap();
        let installed_bun = root.path().join("bun.exe");
        std::fs::write(&installed_bun, b"bun").unwrap();

        let resolved = find_bundled_binary(
            &[root.path().to_path_buf()],
            &[
                "bun-x86_64-pc-windows-msvc.exe".to_string(),
                "bun.exe".to_string(),
            ],
        );

        assert_eq!(resolved.as_deref(), Some(installed_bun.as_path()));
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
