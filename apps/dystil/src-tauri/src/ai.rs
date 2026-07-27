//! Tauri-facing provider and MCP commands.
//!
//! Provider OAuth is intentionally delegated to the provider's own official
//! runtime. Dystil never stores provider tokens.

use crate::recording::RecordingState;
use dystil_ai::{build_daily_context, AiError, CliProvider, DailyUpdate, ProviderKind};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Stdio;
use tauri::{AppHandle, Manager, State};
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{info, warn};

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
    if matches!(provider, ProviderKind::Codex) {
        let state_dir = bun_install_root(provider)?.join("state");
        std::fs::create_dir_all(&state_dir).map_err(|error| error.to_string())?;
        return Ok(vec![(
            "CODEX_HOME".into(),
            state_dir.to_string_lossy().into_owned(),
        )]);
    }
    Ok(Vec::new())
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
    }))
}

/// Installs an official CLI package into a Dystil-owned Bun prefix. Bun's
/// package integrity verification applies; no user-global package manager or
/// shell is used.
async fn install_with_bundled_bun(provider: &ProviderKind) -> Result<CliProvider, String> {
    let bun = bundled_bun()?;
    let install_root = bun_install_root(provider)?;
    std::fs::create_dir_all(&install_root).map_err(|error| error.to_string())?;
    let package = std::env::var(match provider {
        ProviderKind::Codex => "DYSTIL_AI_CODEX_PACKAGE",
        ProviderKind::Claude => "DYSTIL_AI_CLAUDE_PACKAGE",
    })
    .unwrap_or_else(|_| provider_package(provider).into());
    let output = timeout(
        Duration::from_secs(180),
        Command::new(bun)
            .args(["add", "--global", "--exact", &package])
            .env("BUN_INSTALL", &install_root)
            .env("NO_COLOR", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| format!("{} installation timed out", provider.slug()))?
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr)
            .chars()
            .take(1200)
            .collect::<String>();
        return Err(dystil_redact::sanitize_text(&format!(
            "{} installation failed: {detail}",
            provider.slug()
        )));
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

/// Install the official CLI privately with Dystil's bundled Bun.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_install(provider: String) -> Result<AiProviderStatusView, String> {
    let provider = provider_kind(&provider)?;
    install_with_bundled_bun(&provider).await?;
    ai_provider_status(provider.slug().into()).await
}

/// Start the provider-owned browser sign-in. No OAuth token passes through Dystil.
#[tauri::command]
#[specta::specta]
pub async fn ai_provider_login(provider: String) -> Result<(), String> {
    provider_runtime(provider_kind(&provider)?)?
        .begin_login()
        .map_err(|error| error.to_string())
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

fn mcp_binary(app: &AppHandle) -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("DYSTIL_MCP_BINARY").map(PathBuf::from) {
        if path.is_file() {
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
    let mut roots = vec![app
        .path()
        .resource_dir()
        .map_err(|error| error.to_string())?];
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")));
    for root in roots {
        for name in &names {
            for candidate in [root.join(name), root.join("binaries").join(name)] {
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
    }
    Err("Dystil MCP sidecar is not bundled yet".into())
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
