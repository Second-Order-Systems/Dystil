//! Typed, narrow Tauri boundary for durable Ready-to-use artifacts.

use dystil_insights::{
    capability_target, confirm_artifact_change, propose_artifact_change, ready_artifact_detail,
    ready_artifact_provenance, ready_artifacts, record_artifact_used, reject_artifact_change,
    remove_artifact, retry_artifact_change, ArtifactChangePreview, ArtifactPage,
    ReadyArtifactAction, ReadyArtifactDetail, ReadyArtifactMutationResult, ReadyArtifactUseResult,
    run_skill_bundle_build, start_skill_bundle_build, SkillBundlePaths, SkillBundleView, SkillInstallReceipt, SkillInstallTarget,
    SkillInstallTargetAvailability, WorthFixingEvidenceLine,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
#[cfg(not(target_os = "linux"))]
use tauri_plugin_notification::NotificationExt;
use dystil_telemetry::{AiErrorKind, AiOperationKind, AiProviderKind, Outcome, Telemetry};

#[derive(Debug, Clone, Copy, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillBundleProvider {
    Claude,
    Chatgpt,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SkillBundleProviderLaunch {
    /// `desktop` means Dystil found and launched the locally installed app.
    /// `web` is the deliberate fallback when that provider is not installed.
    pub destination: String,
}

impl SkillBundleProvider {
    fn web_url(self) -> &'static str {
        match self {
            Self::Claude => "https://claude.ai/new",
            Self::Chatgpt => "https://chatgpt.com/",
        }
    }

    #[cfg(target_os = "linux")]
    fn linux_desktop_ids(self) -> &'static [&'static str] {
        match self {
            Self::Claude => &["com.anthropic.Claude", "Claude"],
            Self::Chatgpt => &["chatgpt", "ChatGPT"],
        }
    }

    #[cfg(target_os = "macos")]
    fn macos_app_name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::Chatgpt => "ChatGPT",
        }
    }

    #[cfg(target_os = "windows")]
    fn windows_executables(self) -> Vec<std::path::PathBuf> {
        let local = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from);
        let program_files = std::env::var_os("PROGRAMFILES").map(std::path::PathBuf::from);
        let mut candidates = Vec::new();
        match self {
            Self::Claude => {
                for root in local.iter().chain(program_files.iter()) {
                    candidates.push(root.join("AnthropicClaude").join("Claude.exe"));
                    candidates.push(root.join("Programs").join("Claude").join("Claude.exe"));
                }
            }
            Self::Chatgpt => {
                for root in local.iter().chain(program_files.iter()) {
                    candidates.push(root.join("Programs").join("chatgpt").join("ChatGPT.exe"));
                    candidates.push(root.join("ChatGPT").join("ChatGPT.exe"));
                }
            }
        }
        candidates
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop_entry_exists(id: &str) -> bool {
    let mut roots = vec![std::path::PathBuf::from("/usr/share/applications"), std::path::PathBuf::from("/usr/local/share/applications")];
    if let Some(home) = dirs::home_dir() {
        roots.push(home.join(".local/share/applications"));
    }
    if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
        roots.push(std::path::PathBuf::from(data_home).join("applications"));
    }
    roots.into_iter().any(|root| root.join(format!("{id}.desktop")).is_file())
}

fn launch_desktop_provider(provider: SkillBundleProvider) -> Result<bool, String> {
    #[cfg(target_os = "linux")]
    {
        if let Some(id) = provider
            .linux_desktop_ids()
            .iter()
            .find(|id| linux_desktop_entry_exists(id))
        {
            std::process::Command::new("gtk-launch")
                .arg(id)
                .spawn()
                .map_err(|error| format!("could not open the desktop app: {error}"))?;
            return Ok(true);
        }
        return Ok(false);
    }

    #[cfg(target_os = "macos")]
    {
        let app_name = provider.macos_app_name();
        let exists = ["/Applications", "/System/Applications"]
            .iter()
            .any(|root| std::path::Path::new(root).join(format!("{app_name}.app")).is_dir())
            || dirs::home_dir()
                .is_some_and(|home| home.join("Applications").join(format!("{app_name}.app")).is_dir());
        if !exists {
            return Ok(false);
        }
        std::process::Command::new("open")
            .args(["-a", app_name])
            .spawn()
            .map_err(|error| format!("could not open the desktop app: {error}"))?;
        return Ok(true);
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(executable) = provider.windows_executables().into_iter().find(|path| path.is_file()) {
            std::process::Command::new(executable)
                .spawn()
                .map_err(|error| format!("could not open the desktop app: {error}"))?;
            return Ok(true);
        }
        return Ok(false);
    }

    #[allow(unreachable_code)]
    Ok(false)
}

use crate::{recording::RecordingState, store::SettingsStore, worth_fixing_commands::WorthFixingState};

/// The ready-work notification is opt-out. Existing installations predate the
/// setting, so a missing or malformed preference deliberately keeps alerts on.
fn requested_work_ready_notifications_enabled(app: &AppHandle) -> bool {
    let settings = match SettingsStore::get(app) {
        Ok(Some(settings)) => settings,
        _ => return true,
    };
    requested_work_ready_notifications_enabled_from_extra(&settings.extra)
}

fn requested_work_ready_notifications_enabled_from_extra(
    extra: &std::collections::HashMap<String, serde_json::Value>,
) -> bool {
    extra
        .get("notificationPrefs")
        .and_then(|prefs| prefs.get("requestedWorkReady"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true)
}

#[cfg(target_os = "linux")]
fn notify_skill_bundle_finished(app: AppHandle, title: String, body: String) {
    if !requested_work_ready_notifications_enabled(&app) {
        return;
    }

    // Tauri's notification plugin dispatches Linux notifications onto Tokio,
    // while notify-rust's D-Bus implementation blocks on a runtime of its own.
    // Calling it from a plain thread avoids nesting those runtimes.
    if let Err(error) = std::thread::Builder::new()
        .name("skill-bundle-notification".into())
        .spawn(move || {
            if let Err(error) = notify_rust::Notification::new()
                .summary(&title)
                .body(&body)
                .auto_icon()
                .show()
            {
                tracing::warn!(%error, "could not show skill bundle notification");
            }
        })
    {
        tracing::warn!(%error, "could not start skill bundle notification thread");
    }
}

#[cfg(not(target_os = "linux"))]
fn notify_skill_bundle_finished(app: AppHandle, title: String, body: String) {
    if !requested_work_ready_notifications_enabled(&app) {
        return;
    }
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(%error, "could not show skill bundle notification");
    }
}

/// Keep telemetry aggregate-only: the detailed validator message may contain
/// workflow text, so it stays in local logs/database and is never exported.
fn skill_bundle_error_kind(error: &str) -> AiErrorKind {
    let error = error.to_ascii_lowercase();
    if error.contains("bundle review") || error.contains("invalid output") {
        AiErrorKind::InvalidOutput
    } else if error.contains("timed out") {
        AiErrorKind::Timeout
    } else if error.contains("directory") || error.contains("filesystem") {
        AiErrorKind::Filesystem
    } else {
        AiErrorKind::ProcessFailed
    }
}

fn record_skill_bundle_build(
    telemetry: Option<&Telemetry>,
    provider: Option<AiProviderKind>,
    result: &Result<SkillBundleView, dystil_insights::InsightsError>,
) {
    let (Some(telemetry), Some(provider)) = (telemetry, provider) else {
        return;
    };
    match result {
        Ok(_) => {
            telemetry.record_ai_operation(
                provider,
                AiOperationKind::SkillBundleBuild,
                Outcome::Succeeded,
                AiErrorKind::None,
            );
        }
        Err(error) => {
            telemetry.record_ai_operation(
                provider,
                AiOperationKind::SkillBundleBuild,
                Outcome::Failed,
                skill_bundle_error_kind(&error.to_string()),
            );
        }
    }
}

async fn record_product_event(app: &AppHandle, event: dystil_telemetry::ProductEventKind) {
    let recording = app.state::<RecordingState>();
    let telemetry = recording
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.telemetry.clone());
    if let Some(telemetry) = telemetry {
        telemetry.record_product_event(event, 1);
    }
}

async fn runtime(
    app: &AppHandle,
    recording: &RecordingState,
) -> Result<Box<dyn dystil_ai::AiRuntime>, String> {
    let capture = {
        let server = recording.server.lock().await;
        server
            .as_ref()
            .ok_or("capture database is not ready")?
            .db
            .pool
            .clone()
    };
    let timezone = crate::ai::local_timezone_offset();
    crate::ai_runtime::resolve(app, recording, &capture, &timezone)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_to_use(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    cursor: Option<String>,
    limit: u32,
) -> Result<ArtifactPage, String> {
    ready_artifacts(state.pool(&app).await?, cursor.as_deref(), limit)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactDetail, String> {
    ready_artifact_detail(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_artifact_provenance(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<Vec<WorthFixingEvidenceLine>, String> {
    ready_artifact_provenance(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn record_ready_artifact_used(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
    action: ReadyArtifactAction,
) -> Result<ReadyArtifactUseResult, String> {
    record_artifact_used(state.pool(&app).await?, &artifact_id, action)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn open_ready_capability(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactUseResult, String> {
    use tauri_plugin_opener::OpenerExt;

    let pool = state.pool(&app).await?;
    let target = capability_target(pool, &artifact_id)
        .await
        .map_err(|error| error.to_string())?;
    app.opener()
        .open_url(target, None::<&str>)
        .map_err(|error| error.to_string())?;
    record_artifact_used(pool, &artifact_id, ReadyArtifactAction::Open)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn remove_ready_artifact(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<ReadyArtifactMutationResult, String> {
    remove_artifact(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn propose_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    artifact_id: String,
    request: String,
) -> Result<ArtifactChangePreview, String> {
    let runtime = runtime(&app, &recording).await?;
    propose_artifact_change(
        state.pool(&app).await?,
        runtime.as_ref(),
        &artifact_id,
        &request,
    )
    .await
    .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    change_job_id: String,
) -> Result<ArtifactChangePreview, String> {
    let runtime = runtime(&app, &recording).await?;
    retry_artifact_change(state.pool(&app).await?, runtime.as_ref(), &change_job_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn confirm_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    change_job_id: String,
) -> Result<ReadyArtifactDetail, String> {
    confirm_artifact_change(state.pool(&app).await?, &change_job_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn reject_ready_artifact_change(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    change_job_id: String,
) -> Result<ReadyArtifactDetail, String> {
    reject_artifact_change(state.pool(&app).await?, &change_job_id)
        .await
        .map_err(|error| error.to_string())
}

/// Build a portable prompt and Agent Skill from a kept shortcut. The builder
/// runs through the already configured headless AI runtime; it never opens a
/// provider UI or installs its output.
#[tauri::command]
#[specta::specta]
pub async fn build_ready_artifact_skill_bundle(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    artifact_id: String,
) -> Result<SkillBundleView, String> {
    record_product_event(&app, dystil_telemetry::ProductEventKind::SkillBuildRequested).await;
    let runtime = runtime(&app, &recording).await?;
    let paths = SkillBundlePaths::new(crate::dystil_paths::data_dir());
    let pool = state.pool(&app).await?.clone();
    let telemetry = recording
        .server
        .lock()
        .await
        .as_ref()
        .map(|server| server.telemetry.clone());
    let telemetry_provider = match runtime.descriptor().kind {
        dystil_ai::AiRuntimeKind::Codex => Some(AiProviderKind::Codex),
        dystil_ai::AiRuntimeKind::Claude => Some(AiProviderKind::Claude),
        dystil_ai::AiRuntimeKind::Pi => None,
    };
    let (view, build) = start_skill_bundle_build(&pool, runtime.as_ref(), &artifact_id, &paths)
        .await
        .map_err(|error| error.to_string())?;
    // The durable job owns its own lifecycle. Returning immediately means the
    // user can navigate without cancelling a provider process or seeing any
    // external provider window; subsequent loads read its persisted state.
    if let Some(build) = build {
        let notification_app = app.clone();
        tokio::spawn(async move {
            let result = run_skill_bundle_build(&pool, runtime.as_ref(), build, &paths).await;
            record_skill_bundle_build(
                telemetry.as_ref().map(|telemetry| telemetry.as_ref()),
                telemetry_provider,
                &result,
            );
            match result {
                Ok(bundle) => {
                    let skill_name = bundle.skill_name.as_deref().unwrap_or("Your reusable skill");
                    notify_skill_bundle_finished(
                        notification_app.clone(),
                        "Skill ready to install".into(),
                        format!("{skill_name} is ready in Your shortcuts."),
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "skill bundle build failed");
                    notify_skill_bundle_finished(
                        notification_app.clone(),
                        "Skill build needs attention".into(),
                        "Dystil could not finish the skill. Open Your shortcuts to retry.".into(),
                    );
                }
            }
        });
    }
    Ok(view)
}

#[cfg(test)]
mod notification_preference_tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn extra_with(prefs: serde_json::Value) -> HashMap<String, serde_json::Value> {
        HashMap::from([("notificationPrefs".to_string(), prefs)])
    }

    #[test]
    fn ready_work_notifications_default_to_enabled() {
        assert!(requested_work_ready_notifications_enabled_from_extra(&HashMap::new()));
        assert!(requested_work_ready_notifications_enabled_from_extra(&extra_with(json!({}))));
    }

    #[test]
    fn ready_work_notifications_respect_the_setting() {
        assert!(!requested_work_ready_notifications_enabled_from_extra(&extra_with(json!({ "requestedWorkReady": false }))));
        assert!(requested_work_ready_notifications_enabled_from_extra(&extra_with(json!({ "requestedWorkReady": true }))));
    }

    #[test]
    fn bundle_review_rejection_is_telemetry_safe_invalid_output() {
        assert_eq!(
            skill_bundle_error_kind("bundle review still requires rewrite"),
            AiErrorKind::InvalidOutput,
        );
    }

    #[test]
    fn portable_skill_archive_keeps_the_skill_directory_and_files() {
        let root = std::env::temp_dir().join(format!("dystil-skill-export-{}", uuid::Uuid::new_v4()));
        let skill = root.join("prepare-purchase-order");
        std::fs::create_dir_all(skill.join("references")).unwrap();
        std::fs::write(skill.join("SKILL.md"), "# Prepare purchase order\n").unwrap();
        std::fs::write(skill.join("references").join("checklist.md"), "Check totals.\n").unwrap();
        let archive_path = root.join("prepare-purchase-order--v1.zip");

        create_skill_bundle_archive(&skill, &archive_path).unwrap();

        let mut archive = zip::ZipArchive::new(std::fs::File::open(&archive_path).unwrap()).unwrap();
        let names = (0..archive.len())
            .map(|index| archive.by_index(index).unwrap().name().to_owned())
            .collect::<Vec<_>>();
        assert!(names.contains(&"prepare-purchase-order/SKILL.md".to_string()));
        assert!(names.contains(&"prepare-purchase-order/references/checklist.md".to_string()));

        std::fs::remove_dir_all(root).unwrap();
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_ready_artifact_skill_bundle(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    artifact_id: String,
) -> Result<SkillBundleView, String> {
    dystil_insights::ready_artifact_skill_bundle(state.pool(&app).await?, &artifact_id)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_ready_artifact_skill_bundle(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    recording: State<'_, RecordingState>,
    artifact_id: String,
) -> Result<SkillBundleView, String> {
    build_ready_artifact_skill_bundle(app, state, recording, artifact_id).await
}

#[tauri::command]
#[specta::specta]
pub async fn get_skill_bundle_prompt(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    bundle_id: String,
) -> Result<String, String> {
    let row = sqlx::query("SELECT b.directory,b.prompt_path FROM artifact_bundles b JOIN artifacts a ON a.artifact_id=b.artifact_id WHERE b.bundle_id=?1 AND b.status='ready' AND a.status='active'")
        .bind(bundle_id)
        .fetch_optional(state.pool(&app).await?)
        .await
        .map_err(|error| error.to_string())?
        .ok_or("skill bundle is not ready")?;
    use sqlx::Row;
    let path = std::path::PathBuf::from(row.get::<String, _>("directory")).join(row.get::<String, _>("prompt_path"));
    std::fs::read_to_string(path).map_err(|error| error.to_string())
}

fn copy_directory(source: &std::path::Path, destination: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(destination).map_err(|error| error.to_string())?;
    for entry in std::fs::read_dir(source).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let target = destination.join(entry.file_name());
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            copy_directory(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), target).map_err(|error| error.to_string())?;
        } else {
            return Err("skill bundle contains an unsupported filesystem entry".into());
        }
    }
    Ok(())
}

/// Compare a Dystil-owned installation with its immutable source before we
/// claim it is installed. This deliberately compares bytes rather than trusting
/// a prior database receipt: users can edit or remove files after installation.
fn directories_match(source: &std::path::Path, destination: &std::path::Path) -> Result<bool, String> {
    let source_entries = std::fs::read_dir(source).map_err(|error| error.to_string())?
        .map(|entry| entry.map_err(|error| error.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let destination_entries = match std::fs::read_dir(destination) {
        Ok(entries) => entries.map(|entry| entry.map_err(|error| error.to_string())).collect::<Result<Vec<_>, _>>()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if source_entries.len() != destination_entries.len() {
        return Ok(false);
    }
    for entry in source_entries {
        let name = entry.file_name();
        let other = destination.join(&name);
        let source_kind = entry.file_type().map_err(|error| error.to_string())?;
        let destination_kind = match std::fs::symlink_metadata(&other) {
            Ok(kind) => kind.file_type(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.to_string()),
        };
        if source_kind.is_symlink() || destination_kind.is_symlink() || source_kind.is_dir() != destination_kind.is_dir() || source_kind.is_file() != destination_kind.is_file() {
            return Ok(false);
        }
        if source_kind.is_dir() {
            if !directories_match(&entry.path(), &other)? {
                return Ok(false);
            }
        } else if source_kind.is_file()
            && std::fs::read(entry.path()).map_err(|error| error.to_string())?
                != std::fs::read(other).map_err(|error| error.to_string())?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Write the portable archive in-process rather than relying on the host's
/// `zip` executable. Windows does not guarantee that executable exists.
///
/// The archive deliberately includes the skill directory itself, matching the
/// layout generated by the old `zip -qr archive skill-name` invocation and the
/// layout expected by Claude and ChatGPT's skill upload flows.
fn create_skill_bundle_archive(source: &std::path::Path, archive: &std::path::Path) -> Result<(), String> {
    let archive_root = source.parent().ok_or("bundle skill path has no parent")?;
    let file = std::fs::File::create(archive).map_err(|error| error.to_string())?;
    let mut writer = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    write_skill_bundle_archive_directory(source, archive_root, &mut writer, options)?;
    writer.finish().map_err(|error| error.to_string())?;

    Ok(())
}

fn write_skill_bundle_archive_directory(
    directory: &std::path::Path,
    archive_root: &std::path::Path,
    writer: &mut zip::ZipWriter<std::fs::File>,
    options: zip::write::SimpleFileOptions,
) -> Result<(), String> {
    let relative = directory
        .strip_prefix(archive_root)
        .map_err(|error| error.to_string())?;
    let directory_name = relative.to_string_lossy().replace('\\', "/");
    writer
        .add_directory(format!("{directory_name}/"), options)
        .map_err(|error| error.to_string())?;

    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            write_skill_bundle_archive_directory(&path, archive_root, writer, options)?;
        } else if kind.is_file() {
            let name = path
                .strip_prefix(archive_root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            writer
                .start_file(name, options)
                .map_err(|error| error.to_string())?;
            let mut input = std::fs::File::open(path).map_err(|error| error.to_string())?;
            std::io::copy(&mut input, writer).map_err(|error| error.to_string())?;
        } else {
            return Err("skill bundle contains an unsupported filesystem entry".into());
        }
    }
    Ok(())
}

fn skill_install_root(target: SkillInstallTarget) -> Result<std::path::PathBuf, String> {
    let home = dirs::home_dir().ok_or("home directory is unavailable")?;
    match target {
        SkillInstallTarget::Codex => Ok(home.join(".agents/skills")),
        SkillInstallTarget::Claude => Ok(home.join(".claude/skills")),
        SkillInstallTarget::Pi => Ok(home.join(".pi/skills")),
        SkillInstallTarget::ClaudeUpload | SkillInstallTarget::Chatgpt => Err("This target uses an exported skill archive; use Export for Claude instead.".into()),
    }
}

#[tauri::command]
#[specta::specta]
pub async fn get_skill_bundle_install_targets(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    bundle_id: String,
) -> Result<Vec<SkillInstallTargetAvailability>, String> {
    let (source, _, _) = dystil_insights::ready_bundle_location(state.pool(&app).await?, &bundle_id)
        .await
        .map_err(|error| error.to_string())?;
    let name = source.file_name().ok_or("bundle skill path has no name")?;
    let mut targets = Vec::new();
    for target in [SkillInstallTarget::Codex, SkillInstallTarget::Claude, SkillInstallTarget::Pi] {
        let root = skill_install_root(target)?;
        targets.push(SkillInstallTargetAvailability {
            target,
            available: root.exists(),
            installed: root.join(name).exists(),
        });
    }
    targets.push(SkillInstallTargetAvailability { target: SkillInstallTarget::ClaudeUpload, available: true, installed: false });
    Ok(targets)
}

/// Records the user's explicit intent to install a ready skill. The bundle is
/// intentionally not an argument: titles, IDs, and destinations never enter
/// telemetry.
#[tauri::command]
#[specta::specta]
pub async fn record_skill_bundle_install_intent(app: AppHandle) -> Result<(), String> {
    record_product_event(&app, dystil_telemetry::ProductEventKind::SkillInstallRequested).await;
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn install_skill_bundle(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    bundle_id: String,
    target: SkillInstallTarget,
) -> Result<SkillInstallReceipt, String> {
    let pool = state.pool(&app).await?;
    let (source, checksum, _) = dystil_insights::ready_bundle_location(pool, &bundle_id)
        .await
        .map_err(|error| error.to_string())?;
    let name = source.file_name().ok_or("bundle skill path has no name")?;
    let destination = skill_install_root(target)?.join(name);
    let already_installed = dystil_insights::skill_bundle_installation_exists(pool, &bundle_id, target, &destination)
        .await
        .map_err(|error| error.to_string())?;
    if destination.exists() && !already_installed {
        return Err("A skill with this name already exists and was not installed by Dystil.".into());
    }
    let needs_copy = !already_installed || !directories_match(&source, &destination)?;
    if needs_copy {
        // A previous Dystil installation is safe to replace on this explicit
        // install action; never overwrite a similarly named user-owned skill.
        let staging = destination.with_file_name(format!(
            ".{}.dystil-staging-{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        if staging.exists() {
            std::fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
        }
        copy_directory(&source, &staging)?;
        if !directories_match(&source, &staging)? {
            let _ = std::fs::remove_dir_all(&staging);
            return Err("staged skill did not match the validated bundle".into());
        }
        if destination.exists() {
            std::fs::remove_dir_all(&destination).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&staging, &destination).map_err(|error| error.to_string())?;
    }
    if !directories_match(&source, &destination)? {
        return Err("installed skill did not match the validated bundle".into());
    }
    dystil_insights::record_skill_bundle_install(pool, &bundle_id, target, &destination, &checksum)
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn export_skill_bundle(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    bundle_id: String,
) -> Result<SkillInstallReceipt, String> {
    let pool = state.pool(&app).await?;
    let (source, checksum, _) = dystil_insights::ready_bundle_location(pool, &bundle_id)
        .await
        .map_err(|error| error.to_string())?;
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM artifact_bundles WHERE bundle_id=?1")
        .bind(&bundle_id)
        .fetch_one(pool)
        .await
        .map_err(|error| error.to_string())?;
    let export_root = crate::dystil_paths::data_dir().join("skill-bundle-exports");
    std::fs::create_dir_all(&export_root).map_err(|error| error.to_string())?;
    let skill_name = source.file_name().ok_or("bundle skill path has no name")?;
    let skill_name = skill_name.to_string_lossy();
    let archive = export_root.join(format!("{skill_name}--v{revision}.zip"));
    let source = source.clone();
    let archive_for_write = archive.clone();
    tokio::task::spawn_blocking(move || create_skill_bundle_archive(&source, &archive_for_write))
        .await
        .map_err(|error| format!("could not export skill archive: {error}"))??;
    dystil_insights::record_skill_bundle_install(pool, &bundle_id, SkillInstallTarget::ClaudeUpload, &archive, &checksum)
        .await
        .map_err(|error| error.to_string())
}

/// Reveal the exported portable bundle without invoking the opener plugin from
/// a Tokio worker. On Linux the plugin's D-Bus implementation is blocking and
/// creates its own runtime, so it must run on the blocking thread pool.
#[tauri::command]
#[specta::specta]
pub async fn reveal_skill_bundle_export(
    app: AppHandle,
    state: State<'_, WorthFixingState>,
    bundle_id: String,
) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;

    let pool = state.pool(&app).await?;
    let archive: String = sqlx::query_scalar(
        "SELECT destination FROM artifact_bundle_installs
         WHERE bundle_id=?1 AND target='claude_upload'
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(&bundle_id)
    .fetch_one(pool)
    .await
    .map_err(|error| format!("skill export is unavailable: {error}"))?;
    let archive = std::path::PathBuf::from(archive)
        .canonicalize()
        .map_err(|error| format!("skill export is unavailable: {error}"))?;
    let export_root = crate::dystil_paths::data_dir()
        .join("skill-bundle-exports")
        .canonicalize()
        .map_err(|error| format!("skill export directory is unavailable: {error}"))?;
    if !archive.starts_with(&export_root) {
        return Err("skill export path escaped Dystil's export directory".into());
    }
    let app_for_reveal = app.clone();
    tokio::task::spawn_blocking(move || {
        app_for_reveal
            .opener()
            .reveal_item_in_dir(archive)
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("could not reveal the skill export: {error}"))?
}

/// Opens an installed desktop provider when available, with a browser fallback
/// for users who have not installed that provider locally.
#[tauri::command]
#[specta::specta]
pub async fn open_skill_bundle_provider(
    app: AppHandle,
    provider: SkillBundleProvider,
) -> Result<SkillBundleProviderLaunch, String> {
    use tauri_plugin_opener::OpenerExt;

    if launch_desktop_provider(provider)? {
        return Ok(SkillBundleProviderLaunch {
            destination: "desktop".into(),
        });
    }
    app.opener()
        .open_url(provider.web_url(), None::<&str>)
        .map_err(|error| error.to_string())?;
    Ok(SkillBundleProviderLaunch {
        destination: "web".into(),
    })
}
