use super::get_base_dir;
use super::secrets;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use specta::Type;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};
use tauri::AppHandle;
use tauri_plugin_store::StoreBuilder;
use tracing::{error, warn};

/// Social services are private by default. The onboarding answer stores the
/// small allow-list of services the user explicitly chose for work; recording
/// settings are then derived from that answer.
const SOCIAL_CAPTURE_ALLOWED_KEY: &str = "socialCaptureAllowed";
/// One-time migration marker. AI-backed PII removal previously started and
/// downloaded its model without an explicit user choice.
const AI_PII_EXPLICIT_OPT_IN_KEY: &str = "aiPiiExplicitOptInV1";

struct SocialCaptureService {
    id: &'static str,
    window_patterns: &'static [&'static str],
    domains: &'static [&'static str],
}

const SOCIAL_CAPTURE_SERVICES: &[SocialCaptureService] = &[
    SocialCaptureService {
        id: "facebook",
        window_patterns: &["Facebook"],
        domains: &["facebook.com", "fb.com"],
    },
    SocialCaptureService {
        id: "instagram",
        window_patterns: &["Instagram"],
        domains: &["instagram.com"],
    },
    SocialCaptureService {
        id: "messenger",
        window_patterns: &["Messenger"],
        domains: &["messenger.com", "m.me"],
    },
    SocialCaptureService {
        id: "reddit",
        window_patterns: &["Reddit"],
        domains: &["reddit.com"],
    },
    SocialCaptureService {
        id: "telegram",
        window_patterns: &["Telegram"],
        domains: &["telegram.org", "t.me"],
    },
    SocialCaptureService {
        id: "tiktok",
        window_patterns: &["TikTok"],
        domains: &["tiktok.com"],
    },
    SocialCaptureService {
        id: "whatsapp",
        window_patterns: &["WhatsApp"],
        domains: &["whatsapp.com"],
    },
    // Do not use "X" as a window pattern: legacy window matching treats it
    // as a substring and would match unrelated applications. Twitter remains
    // a useful distinctive title/process alias.
    SocialCaptureService {
        id: "x",
        window_patterns: &["Twitter"],
        domains: &["twitter.com"],
    },
    SocialCaptureService {
        id: "youtube",
        window_patterns: &["YouTube"],
        domains: &["youtube.com", "youtu.be"],
    },
    SocialCaptureService {
        id: "discord",
        window_patterns: &["Discord"],
        domains: &["discord.com", "discord.gg"],
    },
];

fn normalized_social_service_ids(selected: &[String]) -> std::collections::HashSet<String> {
    selected
        .iter()
        .map(|value| value.trim().to_lowercase())
        .filter(|value| {
            SOCIAL_CAPTURE_SERVICES
                .iter()
                .any(|service| service.id == value)
        })
        .collect()
}

fn is_social_window_pattern(value: &str) -> bool {
    social_service_id_for_window_pattern(value).is_some()
}

fn social_service_id_for_window_pattern(value: &str) -> Option<&'static str> {
    let normalized = value.trim().to_lowercase();
    SOCIAL_CAPTURE_SERVICES.iter().find_map(|service| {
        let matches = service.id == normalized
            || service
                .window_patterns
                .iter()
                .any(|pattern| pattern.eq_ignore_ascii_case(&normalized));
        matches.then_some(service.id)
    })
}

fn is_social_domain(value: &str) -> bool {
    let normalized = value.trim().to_lowercase();
    SOCIAL_CAPTURE_SERVICES.iter().any(|service| {
        service
            .domains
            .iter()
            .any(|domain| domain.eq_ignore_ascii_case(&normalized))
    })
}

/// Magic header for encrypted store.bin files.
const STORE_MAGIC: &[u8; 8] = b"SPSTORE1";

// ---------------------------------------------------------------------------
// Settings-loss recovery
//
// Goal: a user can never be silently reset to default settings on update.
// 4 layers, defense in depth:
//   L1: snapshot `store.bin.last-good` after every successful save (only if
//       the snapshot has a non-empty settings object — never freeze a degraded state).
//   L2: at boot, before the Tauri store plugin opens the file, auto-restore
//       from `.last-good` IFF the current file is degraded (parses but has
//       empty/missing settings) AND last-good is healthy. The bad file is
//       moved to `store.bin.pre-restore-<ts>` for forensics.
//   L3: refuse `create_new()` over a healthy on-disk file (would otherwise
//       create a fresh in-memory store that overwrites disk on next save).
//   L4: stop writing `b"{}"` on encryption-key failures — keep the encrypted
//       file in place and let the load fail loudly instead.
// ---------------------------------------------------------------------------

/// Suffix for the most-recent known-healthy snapshot.
const LAST_GOOD_SUFFIX: &str = "bin.last-good";

/// Did this store JSON parse and contain a non-empty `settings` object?
/// Used as the "is this a real user state" signal — missing or empty settings
/// means the store was wiped/corrupted and should be restored from last-good.
fn store_json_is_healthy(data: &[u8]) -> bool {
    serde_json::from_slice::<Value>(data)
        .ok()
        .and_then(|v| {
            v.pointer("/settings")
                .and_then(|p| p.as_object())
                .map(|o| !o.is_empty())
        })
        .unwrap_or(false)
}

/// L1 — copy `store.bin` → `store.bin.last-good` if the current file parses
/// and has a non-empty settings object. Skipped silently otherwise so we never
/// freeze a wiped state as the recovery source. Called after every successful save.
pub fn snapshot_last_good(store_path: &Path) {
    let data = match std::fs::read(store_path) {
        Ok(d) => d,
        Err(_) => return,
    };
    if !store_json_is_healthy(&data) {
        return;
    }
    let last_good = store_path.with_extension(LAST_GOOD_SUFFIX);
    if let Err(e) = std::fs::write(&last_good, &data) {
        tracing::warn!(
            "snapshot_last_good: failed to write {}: {}",
            last_good.display(),
            e
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&last_good, std::fs::Permissions::from_mode(0o600));
    }
}

/// L2 — if `store.bin` is degraded (parses but settings is empty) and
/// `.last-good` is healthy, restore it before anything else touches the file.
/// The bad current file is preserved as `.pre-restore-<UTC ts>` so we have
/// forensics if a user reports the restore was wrong.
///
/// Returns `true` when a restore happened (telemetry hook). Logged loudly so
/// it shows up in dystil-app.YYYY-MM-DD.log.
pub fn auto_restore_if_wiped(store_path: &Path) -> bool {
    // Only act on plain-JSON files. Encrypted files are handled by the
    // decrypt path; we don't want to restore over a still-encrypted blob.
    let cur = match std::fs::read(store_path) {
        Ok(d) => d,
        Err(_) => return false,
    };
    if cur.len() >= 8 && &cur[..8] == STORE_MAGIC {
        return false;
    }
    if store_json_is_healthy(&cur) {
        return false; // current state is healthy, nothing to do
    }
    let last_good = store_path.with_extension(LAST_GOOD_SUFFIX);
    let Ok(lg) = std::fs::read(&last_good) else {
        return false;
    };
    if !store_json_is_healthy(&lg) {
        return false; // last-good is also wiped (shouldn't happen — L1 guards this)
    }

    // Move the bad file aside before overwriting it
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let pre_restore = store_path.with_extension(format!("bin.pre-restore-{}", ts));
    if let Err(e) = std::fs::copy(store_path, &pre_restore) {
        tracing::warn!(
            "auto_restore_if_wiped: failed to back up {} to {}: {} — aborting restore",
            store_path.display(),
            pre_restore.display(),
            e
        );
        return false;
    }

    if let Err(e) = std::fs::write(store_path, &lg) {
        tracing::error!(
            "auto_restore_if_wiped: failed to restore {} from {}: {}",
            store_path.display(),
            last_good.display(),
            e
        );
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(store_path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::warn!(
        "auto_restore_if_wiped: restored {} from {} (settings were empty); \
         pre-restore copy at {}",
        store_path.display(),
        last_good.display(),
        pre_restore.display()
    );
    true
}

/// Decrypt store.bin in place if it's encrypted and keychain key is available.
/// No-op if the file is already plain JSON or keychain is unavailable.
fn decrypt_store_file(path: &Path) {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return,
    };
    if data.len() < 8 || &data[..8] != STORE_MAGIC {
        return; // already plain JSON (or empty)
    }
    // File is encrypted, so user must have encryption enabled
    // Use get_key_if_encryption_enabled to prevent prompts if encryption is somehow disabled
    match secrets::get_key_if_encryption_enabled() {
        secrets::KeyResult::NotFound => {
            // L4 — DO NOT wipe. Previously this branch wrote `b"{}"` over
            // store.bin and lost the user's settings on every signed update
            // (macOS code-signing identity changes can evict keychain keys).
            // The encrypted file still has the user's data; leave it in
            // place and let the load fall through to L2 auto_restore from
            // store.bin.last-good. Manual recovery: re-grant keychain
            // access in System Settings → Privacy & Security → Keychain.
            let backup = path.with_extension("bin.encrypted.bak");
            let _ = std::fs::copy(path, &backup);
            tracing::error!(
                "store.bin is encrypted but keychain key not found — \
                 leaving the encrypted file in place ({}). Restore from \
                 store.bin.last-good or grant keychain access and restart.",
                backup.display()
            );
            return;
        }
    }
}

/// Encrypt store.bin in place if keychain key is available AND encryption is opted-in.
///
/// DISABLED BY DEFAULT — the macOS keychain doesn't reliably persist keys across
/// app updates (code signing identity changes), causing settings loss on every update.
/// The 0o600 file permissions are sufficient protection for now.
///
/// To opt in: create ~/.dystil/.encrypt-store or set DYSTIL_ENCRYPT_STORE=1.
fn encrypt_store_file(path: &Path) {
    // Check opt-in flag
    let opted_in = std::env::var("DYSTIL_ENCRYPT_STORE")
        .map(|v| v == "1")
        .unwrap_or(false)
        || path
            .parent()
            .map(|p| p.join(".encrypt-store").exists())
            .unwrap_or(false);
    if !opted_in {
        return;
    }

    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return,
    };
    if data.len() >= 8 && &data[..8] == STORE_MAGIC {
        return; // already encrypted
    }
    // Vault crypto is excluded from the Dystil product — encryption is a no-op.
}

/// Re-encrypt store.bin on disk. Called after the Tauri store plugin writes plain JSON.
/// Also syncs the .encrypt-store flag file from the encryptStore setting.
pub fn reencrypt_store_file(app: &AppHandle) {
    if let Ok(base_dir) = get_base_dir(app, None) {
        // Sync the flag file from the store's encryptStore setting
        let flag_path = base_dir.join(".encrypt-store");
        let store_path = base_dir.join("store.bin");

        // Read the setting from the store JSON on disk
        let encrypt_enabled = std::fs::read(&store_path)
            .ok()
            .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
            .and_then(|json| {
                json.get("settings")
                    .and_then(|s| s.get("encryptStore"))
                    .and_then(|v| v.as_bool())
            })
            .unwrap_or(true);

        if encrypt_enabled && !flag_path.exists() {
            let _ = std::fs::write(&flag_path, b"");
        } else if !encrypt_enabled && flag_path.exists() {
            let _ = std::fs::remove_file(&flag_path);
        }

        // L1 — snapshot the current state to .last-good IFF it's healthy
        // (parses + has aiPresets). Runs BEFORE encryption so the snapshot
        // is plain JSON and recoverable even if keychain access is lost on
        // the next update. No-op for degraded states so we never freeze
        // bad data as the recovery source.
        snapshot_last_good(&store_path);

        encrypt_store_file(&store_path);
    }
}

/// Tauri command: re-encrypt store.bin after frontend saves.
#[tauri::command]
#[specta::specta]
pub fn reencrypt_store(app: AppHandle) -> Result<(), String> {
    reencrypt_store_file(&app);
    Ok(())
}

/// Cached store instance — reusable across the process lifetime.
/// Uses Mutex instead of OnceLock so the cache can be invalidated when the
/// Tauri resource table drops the underlying store (e.g. after an in-place
/// update restart on Windows where resource IDs become stale).
static STORE_CACHE: Mutex<Option<Arc<tauri_plugin_store::Store<tauri::Wry>>>> = Mutex::new(None);

/// Build (or rebuild) the store, retrying on TOCTOU races and stale resource IDs.
fn build_store(app: &AppHandle) -> anyhow::Result<Arc<tauri_plugin_store::Store<tauri::Wry>>> {
    let base_dir = get_base_dir(app, None)?;
    let store_path = base_dir.join("store.bin");

    // Decrypt store.bin before the plugin reads it (no-op if plain JSON or keychain unavailable)
    if store_path.exists() {
        decrypt_store_file(&store_path);
    }

    // L2 — if the file is degraded (parses but has no aiPresets), restore
    // from .last-good before the plugin reads it. Runs after decrypt so
    // we operate on the plain-JSON form. No-op if the current state is
    // already healthy or no .last-good exists yet.
    if store_path.exists() {
        let _ = auto_restore_if_wiped(&store_path);
    }

    let mut last_err = None;
    // Ensure store.bin has restrictive permissions (contains API keys)
    #[cfg(unix)]
    if store_path.exists() {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&store_path, std::fs::Permissions::from_mode(0o600));
    }

    for attempt in 0..3u32 {
        match StoreBuilder::new(app, store_path.clone()).build() {
            Ok(s) => {
                // Re-encrypt immediately after the plugin loaded the file
                encrypt_store_file(&store_path);
                return Ok(s);
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("os error 17") || msg.contains("File exists") {
                    tracing::warn!(
                        "store build race (attempt {}): {}, retrying",
                        attempt + 1,
                        msg
                    );
                    std::thread::sleep(std::time::Duration::from_millis(
                        100 * (attempt as u64 + 1),
                    ));
                    last_err = Some(e);
                    continue;
                }
                // After cleanup_before_exit or in-place update on Windows, the
                // resources_table is cleared but StoreState.stores still holds the
                // old resource ID. Force a fresh store via create_new to evict it.
                if msg.contains("resource id") && msg.contains("invalid") {
                    // L3 — refuse `create_new()` over a healthy on-disk
                    // file. The fresh in-memory store would later flush
                    // empty defaults to disk and silently overwrite the
                    // user's settings (verified root cause for Louis's
                    // 2026-05-09 wipe). If the file has aiPresets, surface
                    // the error so the retry loop runs again instead.
                    let disk_healthy = std::fs::read(&store_path)
                        .map(|d| store_json_is_healthy(&d))
                        .unwrap_or(false);
                    if disk_healthy {
                        tracing::error!(
                            "store resource stale (attempt {}): {}, but disk \
                             is healthy — refusing create_new() to avoid \
                             overwriting user data; will retry .build()",
                            attempt + 1,
                            msg
                        );
                        last_err = Some(e);
                        std::thread::sleep(std::time::Duration::from_millis(
                            200 * (attempt as u64 + 1),
                        ));
                        continue;
                    }
                    tracing::warn!(
                        "store resource stale (attempt {}): {}, rebuilding fresh \
                         (disk file empty/missing presets, safe to create_new)",
                        attempt + 1,
                        msg
                    );
                    match StoreBuilder::new(app, store_path.clone())
                        .create_new()
                        .build()
                    {
                        Ok(s) => {
                            encrypt_store_file(&store_path);
                            return Ok(s);
                        }
                        Err(e2) => {
                            tracing::warn!("fresh store build also failed: {}", e2);
                            last_err = Some(e);
                            continue;
                        }
                    }
                }
                return Err(anyhow::anyhow!(e));
            }
        }
    }
    Err(anyhow::anyhow!(last_err.unwrap()))
}

pub fn get_store(
    app: &AppHandle,
    _profile_name: Option<String>, // Keep parameter for API compatibility but ignore it
) -> anyhow::Result<Arc<tauri_plugin_store::Store<tauri::Wry>>> {
    {
        let guard = STORE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(ref cached) = *guard {
            return Ok(cached.clone());
        }
    }

    let in_tokio = tokio::runtime::Handle::try_current().is_ok();
    let store = if in_tokio {
        tokio::task::block_in_place(|| build_store(app))?
    } else {
        build_store(app)?
    };

    let mut guard = STORE_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ref cached) = *guard {
        return Ok(cached.clone());
    }
    *guard = Some(store.clone());
    Ok(store)
}

#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(default)]
pub struct OnboardingStore {
    #[serde(rename = "isCompleted")]
    pub is_completed: bool,
    #[serde(rename = "completedAt")]
    pub completed_at: Option<String>,
    /// Current step in onboarding flow (login, intro, usecases, status)
    /// Used to resume after app restart (e.g., after granting permissions)
    #[serde(rename = "currentStep", default)]
    pub current_step: Option<String>,
    /// The capability selected in the final onboarding step. This records a
    /// local preference only; provider credentials live in their own stores.
    #[serde(rename = "aiSetupChoice", default)]
    pub ai_setup_choice: Option<String>,
}

impl Default for OnboardingStore {
    fn default() -> Self {
        Self {
            is_completed: false,
            completed_at: None,
            current_step: None,
            ai_setup_choice: None,
        }
    }
}

impl OnboardingStore {
    pub fn get(app: &AppHandle) -> Result<Option<Self>, String> {
        let store = get_store(app, None).map_err(|e| e.to_string())?;

        match store.is_empty() {
            true => Ok(None),
            false => {
                let onboarding =
                    serde_json::from_value(store.get("onboarding").unwrap_or(Value::Null));
                match onboarding {
                    Ok(onboarding) => Ok(onboarding),
                    Err(e) => {
                        error!("Failed to deserialize onboarding: {}", e);
                        Err(e.to_string())
                    }
                }
            }
        }
    }

    pub fn update(
        app: &AppHandle,
        update: impl FnOnce(&mut OnboardingStore),
    ) -> Result<(), String> {
        let Ok(store) = get_store(app, None) else {
            return Err("Failed to get onboarding store".to_string());
        };

        let mut onboarding = Self::get(app)?.unwrap_or_default();
        update(&mut onboarding);
        store.set("onboarding", json!(onboarding));
        store.save().map_err(|e| e.to_string())?;
        reencrypt_store_file(app);
        Ok(())
    }

    pub fn save(&self, app: &AppHandle) -> Result<(), String> {
        let Ok(store) = get_store(app, None) else {
            return Err("Failed to get onboarding store".to_string());
        };

        store.set("onboarding", json!(self));
        store.save().map_err(|e| e.to_string())?;
        reencrypt_store_file(app);
        Ok(())
    }

    pub fn complete(&mut self) {
        self.is_completed = true;
        self.completed_at = Some(chrono::Utc::now().to_rfc3339());
        self.current_step = None;
        self.ai_setup_choice = None;
    }

    pub fn reset(&mut self) {
        self.is_completed = false;
        self.completed_at = None;
        self.current_step = None;
    }
}

fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    T: Default + Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    let opt = Option::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(default)]
pub struct SettingsStore {
    // ── Recording settings (shared source of truth) ──────────────────────
    /// All recording/capture config lives here. Flattened so the JSON shape
    /// is unchanged for the visual/accessibility settings that remain supported.
    #[serde(flatten)]
    pub recording: crate::recording_settings::RecordingSettings,

    // ── App-only fields (UI and metadata) ───────────────────────────────
    #[serde(rename = "isLoading")]
    pub is_loading: bool,

    #[serde(rename = "devMode")]
    pub dev_mode: bool,
    #[serde(rename = "ocrEngine")]
    pub ocr_engine: String,
    #[serde(rename = "dataDir")]
    pub data_dir: String,
    #[serde(rename = "autoStartEnabled")]
    pub auto_start_enabled: bool,
    /// Whether capture was explicitly paused by the user. Kept separate from
    /// the live capture session so a privacy pause survives an app restart.
    #[serde(rename = "capturePaused", default)]
    pub capture_paused: bool,
    /// Absolute UTC deadline for a timed pause. Paused state without a valid
    /// deadline is treated as stale legacy state and cleared at startup.
    #[serde(rename = "capturePauseUntil", default)]
    pub capture_pause_until: Option<String>,
    /// Number of days of raw capture to retain locally. Zero means forever.
    /// Findings and other derived artifacts are not governed by this setting.
    #[serde(rename = "retentionDays", default = "default_retention_days")]
    pub retention_days: u32,
    #[serde(rename = "platform")]
    pub platform: String,
    #[serde(rename = "user", deserialize_with = "deserialize_null_as_default")]
    pub user: User,
    /// Explicit, local permission for cloud synchronization. This is never
    /// inferred from login/device state or remote policy.
    #[serde(rename = "syncConsent", default)]
    pub sync_consent: SyncConsent,
    /// Anonymous operational telemetry (counts and durations only — never
    /// captured content, window titles, URLs, prompts, or file paths).
    ///
    /// On by default in community builds and user-disableable here or via
    /// `DYSTIL_TELEMETRY=0`. Under `enterprise-client` this is organization-
    /// managed and forced on — see [`SettingsStore::telemetry_effective`].
    ///
    /// Nothing is exported until onboarding completes, so a user always sees
    /// the disclosure before the first payload leaves the machine.
    #[serde(rename = "telemetryEnabled", default = "default_true")]
    pub telemetry_enabled: bool,
    /// Unique device ID for AI usage tracking (generated on first launch)
    #[serde(rename = "deviceId", default = "generate_device_id")]
    pub device_id: String,
    /// Auto-install updates and restart when a new version is available.
    /// When disabled, users must click "update now" in the tray menu.
    #[serde(rename = "autoUpdate", default = "default_true")]
    pub auto_update: bool,
    /// Auto-update store-installed pipes that haven't been locally modified.
    #[serde(rename = "autoUpdatePipes", default = "default_true")]
    pub auto_update_pipes: bool,
    /// Timeline overlay mode: "fullscreen" (floating panel above everything) or
    /// "window" (normal resizable window with title bar).
    #[serde(rename = "overlayMode", default = "default_overlay_mode")]
    pub overlay_mode: String,
    /// Allow screen recording apps to capture the overlay.
    /// Disabled by default so the overlay doesn't appear in dystil's own recordings.
    #[serde(rename = "showOverlayInScreenRecording", default)]
    pub show_overlay_in_screen_recording: bool,

    /// Show restart notifications when visual capture stalls.
    /// Disabled by default for now until the stall detector is more reliable.
    #[serde(rename = "showRestartNotifications", default)]
    pub show_restart_notifications: bool,

    /// When true, apply macOS vibrancy effect to the sidebar for a translucent look.
    #[serde(rename = "translucentSidebar", default)]
    pub translucent_sidebar: bool,

    /// When true (default), hide model "thinking" reasoning blocks in the chat
    /// transcript. The model still emits them server-side; we just don't
    /// render the collapsible block in the UI.
    #[serde(rename = "hideThinkingBlocks", default = "default_true")]
    pub hide_thinking_blocks: bool,

    /// UI theme: "light", "dark", or "system".
    #[serde(rename = "uiTheme", default = "default_ui_theme")]
    pub ui_theme: String,

    /// Catch-all for fields added by the frontend (e.g. chatHistory)
    /// that the Rust struct doesn't know about. Without this, `save()` would
    /// serialize only known fields and silently wipe frontend-only data.
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_json::Value>,

    /// Windows-only: when true, clicking the X on the Home window hides it to
    /// the system tray (and removes it from the taskbar) instead of minimizing.
    /// Read by the CloseRequested handler in main.rs. Default off (historical
    /// minimize-to-taskbar behavior).
    #[serde(rename = "minimizeToTrayOnClose", default)]
    pub minimize_to_tray_on_close: bool,
}

#[derive(Debug, Serialize, Deserialize, Type, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SyncConsent {
    pub segments: bool,
    pub screenshots: bool,
}

/// Whether the `DYSTIL_TELEMETRY` environment variable explicitly disables
/// telemetry. Accepts `0`, `false`, `off`, and `no`, case-insensitively.
///
/// Read at runtime rather than compile time so an operator can disable
/// telemetry on a machine they did not build.
pub fn telemetry_disabled_by_env() -> bool {
    std::env::var("DYSTIL_TELEMETRY")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            matches!(value.as_str(), "0" | "false" | "off" | "no")
        })
        .unwrap_or(false)
}

impl SettingsStore {
    /// Resolve whether telemetry may be collected and exported.
    ///
    /// Precedence, highest first:
    ///
    /// 1. `DYSTIL_TELEMETRY=0` always wins, including for enterprise builds —
    ///    an operator must be able to stop egress on a machine they control.
    /// 2. `enterprise-client` forces it on. Consent is organizational, agreed
    ///    by an administrator, so there is no per-user prompt or toggle. This
    ///    mirrors [`SyncConsent::effective`].
    /// 3. Otherwise the user's setting, which defaults to on.
    pub fn telemetry_effective(&self) -> bool {
        if telemetry_disabled_by_env() {
            return false;
        }
        if cfg!(feature = "enterprise-client") {
            return true;
        }
        self.telemetry_enabled
    }
}

impl SyncConsent {
    pub const fn effective(self) -> Self {
        if cfg!(feature = "enterprise-client") {
            Self {
                segments: true,
                screenshots: true,
            }
        } else {
            self
        }
    }

    pub fn validate(self) -> Result<Self, String> {
        if self.screenshots && !self.segments {
            return Err("screenshot sync requires segment sync".to_string());
        }
        Ok(self)
    }
}

fn generate_device_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn default_true() -> bool {
    true
}

fn default_retention_days() -> u32 {
    90
}

fn default_ui_theme() -> String {
    "light".to_string()
}

fn default_overlay_mode() -> String {
    #[cfg(target_os = "macos")]
    {
        "fullscreen".to_string()
    }
    #[cfg(not(target_os = "macos"))]
    {
        "window".to_string()
    }
}

#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(default)]
pub struct User {
    pub id: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    pub image: Option<String>,
    pub token: Option<String>,
    pub api_key: Option<String>,
    pub credits: Option<Credits>,
    pub bio: Option<String>,
    pub website: Option<String>,
    pub contact: Option<String>,
    pub credits_balance: Option<i32>,
}

impl Default for User {
    fn default() -> Self {
        Self {
            id: None,
            name: None,
            email: None,
            image: None,
            token: None,
            api_key: None,
            credits: None,
            bio: None,
            website: None,
            contact: None,
            credits_balance: None,
        }
    }
}

#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(default)]
pub struct Credits {
    pub amount: i32,
}

impl Default for Credits {
    fn default() -> Self {
        Self { amount: 0 }
    }
}

impl Default for SettingsStore {
    fn default() -> Self {
        // Default ignored windows for all OS
        let mut ignored_windows = vec![
            "bit".to_string(),
            "VPN".to_string(),
            "Trash".to_string(),
            "Private".to_string(),
            "Incognito".to_string(),
            "Wallpaper".to_string(),
            "Settings".to_string(),
            "Keepass".to_string(),
            "Recorder".to_string(),
            "vault".to_string(),
            "OBS Studio".to_string(),
            "dystil".to_string(),
        ];

        #[cfg(target_os = "macos")]
        ignored_windows.extend([
            ".env".to_string(),
            "Item-0".to_string(),
            "App Icon Window".to_string(),
            "Battery".to_string(),
            "Shortcuts".to_string(),
            "WiFi".to_string(),
            "BentoBox".to_string(),
            "Clock".to_string(),
            "Dock".to_string(),
            "DeepL".to_string(),
            "Control Center".to_string(),
        ]);

        #[cfg(target_os = "windows")]
        ignored_windows.extend([
            "Nvidia".to_string(),
            "Control Panel".to_string(),
            "System Properties".to_string(),
            "LockApp.exe".to_string(),
            "SearchHost.exe".to_string(),
            "ShellExperienceHost.exe".to_string(),
            "PickerHost.exe".to_string(),
            "Taskmgr.exe".to_string(),
            "SnippingTool.exe".to_string(),
        ]);

        #[cfg(target_os = "linux")]
        ignored_windows.extend([
            "Info center".to_string(),
            "Discover".to_string(),
            "Parted".to_string(),
        ]);

        let mut settings = Self {
            // App-specific defaults override RecordingSettings::default() where needed
            recording: crate::recording_settings::RecordingSettings {
                monitor_ids: vec!["default".to_string()],
                use_pii_removal: true,
                async_pii_redaction: false,
                pii_backend: "local".to_string(),
                ignored_windows,
                ..crate::recording_settings::RecordingSettings::default()
            },
            is_loading: false,
            dev_mode: false,
            #[cfg(target_os = "macos")]
            ocr_engine: "apple-native".to_string(),
            #[cfg(target_os = "windows")]
            ocr_engine: "windows-native".to_string(),
            #[cfg(target_os = "linux")]
            ocr_engine: "tesseract".to_string(),
            data_dir: "default".to_string(),
            auto_start_enabled: true,
            capture_paused: false,
            capture_pause_until: None,
            retention_days: default_retention_days(),
            platform: "unknown".to_string(),
            user: User::default(),
            sync_consent: SyncConsent::default(),
            telemetry_enabled: true,
            device_id: uuid::Uuid::new_v4().to_string(),
            auto_update: true,
            auto_update_pipes: true,
            #[cfg(target_os = "macos")]
            overlay_mode: "fullscreen".to_string(),
            #[cfg(not(target_os = "macos"))]
            overlay_mode: "window".to_string(),
            show_overlay_in_screen_recording: false,
            show_restart_notifications: false,
            #[cfg(target_os = "macos")]
            translucent_sidebar: true,
            #[cfg(not(target_os = "macos"))]
            translucent_sidebar: false,
            hide_thinking_blocks: true,
            ui_theme: "light".to_string(),
            minimize_to_tray_on_close: true,
            extra: std::collections::HashMap::new(),
        };
        settings.apply_social_capture_policy(&[]);
        settings
    }
}

impl SettingsStore {
    /// Remove legacy field aliases that conflict with their renamed counterparts.
    /// e.g. `enableUiEvents` was renamed to `enableAccessibility` — if both exist
    /// in the stored JSON, serde rejects it as a duplicate field.
    /// Also sanitize unknown AI provider types to prevent deserialization failures
    /// (e.g. synced settings from a newer version with a provider this version doesn't know).
    fn sanitize_legacy_fields(mut val: Value) -> Value {
        if let Some(obj) = val.as_object_mut() {
            if obj.contains_key("enableAccessibility") {
                obj.remove("enableUiEvents");
            } else if let Some(v) = obj.remove("enableUiEvents") {
                obj.insert("enableAccessibility".to_string(), v);
            }

            // These frontend-only fields never controlled backend deletion.
            // Replace them with the single code-owned retention policy.
            obj.remove("localRetentionEnabled");
            obj.remove("localRetentionDays");
            obj.remove("localRetentionMode");
            obj.entry("retentionDays".to_string())
                .or_insert_with(|| Value::from(default_retention_days()));

            // Temporary one-time migration: disable restart notifications for all
            // existing users until the stall detector is more reliable. Users can
            // still opt back in manually from Settings; once they've seen this
            // version, we stop overriding their choice.
            if !obj.contains_key("restartNotificationsDefaultedOff") {
                obj.insert("showRestartNotifications".to_string(), Value::Bool(false));
                obj.insert(
                    "restartNotificationsDefaultedOff".to_string(),
                    Value::Bool(true),
                );
            }

            // The local model is optional and relatively large. No prior UI
            // exposed this preference, so an existing `true` value cannot
            // represent an informed opt-in. Default it off once; subsequent
            // user choices are preserved by the marker.
            if !obj.contains_key(AI_PII_EXPLICIT_OPT_IN_KEY) {
                obj.insert("asyncPiiRedaction".to_string(), Value::Bool(false));
                obj.insert(AI_PII_EXPLICIT_OPT_IN_KEY.to_string(), Value::Bool(true));
            }
        }
        val
    }

    pub fn get(app: &AppHandle) -> Result<Option<Self>, String> {
        let store = get_store(app, None).map_err(|e| format!("Failed to get store: {}", e))?;

        match store.is_empty() {
            true => Ok(None),
            false => {
                let raw = store.get("settings").unwrap_or(Value::Null);
                let sanitized = Self::sanitize_legacy_fields(raw.clone());
                // Persist sanitized fields back to store so the migration only warns once
                if sanitized != raw {
                    store.set("settings", sanitized.clone());
                    let _ = store.save();
                    reencrypt_store_file(app);
                }
                let settings = serde_json::from_value(sanitized);
                match settings {
                    Ok(settings) => Ok(settings),
                    Err(e) => {
                        error!("Failed to deserialize settings: {}", e);
                        Err(e.to_string())
                    }
                }
            }
        }
    }

    /// Build a `RecordingSettings` from this settings store.
    ///
    /// Since RecordingSettings is now embedded via flatten, this is mostly a
    /// clone with the authenticated user ID override.
    pub fn to_recording_settings(&self) -> crate::recording_settings::RecordingSettings {
        let mut settings = self.recording.clone();
        if crate::capture_policy::enterprise_managed() {
            settings.disable_vision = false;
        }
        // Override user_id with the Clerk JWT token from the auth user object.
        // This token is used as the Bearer credential for dystil cloud
        // (transcription proxy, Pi agent, etc.), not as a database ID.
        // Fallback to user.id if token is unavailable.
        settings.user_id = self
            .user
            .token
            .as_ref()
            .filter(|t| !t.is_empty())
            .or(self.user.id.as_ref().filter(|id| !id.is_empty()))
            .cloned()
            .unwrap_or_default();
        settings
    }

    /// Build a `DystilCaptureConfig` from this settings store.
    pub fn to_dystil_capture_config(
        &self,
        data_dir: std::path::PathBuf,
    ) -> crate::capture_config::DystilCaptureConfig {
        let settings = self.to_recording_settings();
        crate::capture_config::DystilCaptureConfig {
            data_dir,
            disable_vision: settings.disable_vision,
            capture_scroll: settings.capture_scroll,
            disable_clipboard_capture: settings.disable_clipboard_capture,
            capture_on_clipboard: settings.capture_on_clipboard,
            disable_keyboard_capture: settings.disable_keyboard_capture,
            ignored_windows: settings.ignored_windows.clone(),
            included_windows: settings.included_windows.clone(),
            ignored_urls: settings.ignored_urls.clone(),
            ignore_incognito_windows: settings.ignore_incognito_windows,
            prioritize_input_latency: settings.prioritize_input_latency,
            extraction_thread_priority: settings.extraction_thread_priority.clone(),
            pause_extraction_on_input_ms: settings.pause_extraction_on_input_ms,
            async_pii_redaction: settings.async_pii_redaction,
        }
    }

    pub fn is_logged_in(&self) -> bool {
        self.user
            .token
            .as_ref()
            .is_some_and(|token| !token.is_empty())
            || self.user.id.as_ref().is_some_and(|id| !id.is_empty())
    }

    pub fn save(&self, app: &AppHandle) -> Result<(), String> {
        let Ok(store) = get_store(app, None) else {
            return Err("Failed to get store".to_string());
        };

        store.set("settings", json!(self));
        store.save().map_err(|e| e.to_string())?;
        reencrypt_store_file(app);
        Ok(())
    }

    /// Apply Dystil's social-media privacy policy without affecting unrelated
    /// user-created window and URL filters.
    pub fn apply_social_capture_policy(&mut self, selected_services: &[String]) {
        let selected = normalized_social_service_ids(selected_services);

        // Remove the previous derived policy before rebuilding it. This keeps
        // the operation idempotent when onboarding is resumed or repeated.
        self.recording
            .ignored_windows
            .retain(|pattern| !is_social_window_pattern(pattern));
        self.recording
            .ignored_urls
            .retain(|domain| !is_social_domain(domain));

        for service in SOCIAL_CAPTURE_SERVICES {
            if selected.contains(service.id) {
                continue;
            }

            append_unique_patterns(
                &mut self.recording.ignored_windows,
                service.window_patterns.iter().copied(),
            );
            append_unique_patterns(
                &mut self.recording.ignored_urls,
                service.domains.iter().copied(),
            );
        }

        self.extra.insert(
            SOCIAL_CAPTURE_ALLOWED_KEY.to_string(),
            Value::Array(selected.into_iter().map(Value::String).collect()),
        );
    }
}

fn append_unique_patterns<'a>(
    target: &mut Vec<String>,
    additions: impl IntoIterator<Item = &'a str>,
) {
    for addition in additions {
        if !target
            .iter()
            .any(|existing| existing.trim().eq_ignore_ascii_case(addition))
        {
            target.push(addition.to_string());
        }
    }
}

pub fn init_store(app: &AppHandle) -> Result<SettingsStore, String> {
    println!("Initializing settings store");

    let raw_obj = get_store(app, None)
        .ok()
        .and_then(|store| store.get("settings"))
        .and_then(|raw| raw.as_object().cloned());

    let should_persist_restart_notification_migration = raw_obj
        .as_ref()
        .map(|obj| !obj.contains_key("restartNotificationsDefaultedOff"))
        .unwrap_or(false);

    let is_new_store;
    let (mut store, mut should_save) = match SettingsStore::get(app) {
        Ok(Some(store)) => {
            is_new_store = false;
            (store, should_persist_restart_notification_migration)
        }
        Ok(None) => {
            is_new_store = true;
            (SettingsStore::default(), true) // New store, save defaults
        }
        Err(e) => {
            is_new_store = false;
            // Fallback to defaults when deserialization fails (e.g., corrupted store)
            // DON'T save - preserve original store in case it can be manually recovered
            // This prevents crashes from invalid values like negative integers in u32 fields
            // Non-fatal — logged as warn (not error) so Sentry doesn't pick it up.
            warn!(
                "Failed to deserialize settings, using defaults (store not overwritten): {}",
                e
            );
            (SettingsStore::default(), false)
        }
    };

    // Tier detection. Two cases:
    // - New install: detect tier AND apply tier defaults (video_quality, power_mode, etc.)
    // - Existing user upgrading: detect tier for DB/channel config but do NOT override
    //   their existing capture settings (they may have customized video_quality etc.)
    // Also re-detect if the stored tier doesn't match current hardware classification
    // (e.g. tier boundaries changed in an update).
    {
        let detected = crate::recording_settings::detect_tier();
        let stored_tier = store
            .recording
            .device_tier
            .as_deref()
            .and_then(crate::recording_settings::DeviceTier::from_str_loose);
        if stored_tier != Some(detected) {
            tracing::info!("hardware tier changed: {:?} -> {:?}", stored_tier, detected);
            if is_new_store || store.recording.device_tier.is_none() {
                crate::recording_settings::apply_tier_defaults(&mut store.recording, detected);
            }
            store.recording.device_tier = Some(detected.as_str().to_string());
            should_save = true;
        }
    }

    if should_save {
        if let Err(e) = store.save(app) {
            // Non-fatal — logged as warn (not error) so Sentry doesn't pick it up.
            // Common cause on Windows: antivirus / Controlled Folder Access / OneDrive
            // blocks the first write; we retry on subsequent saves so the user isn't
            // actually stuck. Not worth paging Louis about.
            warn!("Failed to save initial settings store (non-fatal): {}", e);
        }
    }
    Ok(store)
}

pub fn init_onboarding_store(app: &AppHandle) -> Result<OnboardingStore, String> {
    println!("Initializing onboarding store");

    let (onboarding, should_save) = match OnboardingStore::get(app) {
        Ok(Some(onboarding)) => (onboarding, false),
        Ok(None) => (OnboardingStore::default(), true),
        Err(e) => {
            // Fallback to defaults when deserialization fails
            // DON'T save - preserve original store
            // Non-fatal — logged as warn (not error) so Sentry doesn't pick it up.
            warn!(
                "Failed to deserialize onboarding, using defaults (store not overwritten): {}",
                e
            );
            (OnboardingStore::default(), false)
        }
    };

    if should_save {
        if let Err(e) = onboarding.save(app) {
            // Non-fatal — logged as warn (not error) so Sentry doesn't pick it up.
            // See matching comment in init_settings_store.
            warn!("Failed to save initial onboarding store (non-fatal): {}", e);
        }
    }
    Ok(onboarding)
}

#[cfg(test)]
mod social_capture_policy_tests {
    use super::*;

    #[test]
    fn defaults_block_all_social_services_without_using_global_includes() {
        let settings = SettingsStore::default();

        assert!(settings
            .recording
            .ignored_windows
            .iter()
            .any(|value| value == "WhatsApp"));
        assert!(settings
            .recording
            .ignored_urls
            .iter()
            .any(|value| value == "youtube.com"));
        assert!(settings.recording.included_windows.is_empty());
    }

    #[test]
    fn selected_service_is_allowed_while_other_social_services_stay_blocked() {
        let mut settings = SettingsStore::default();
        settings
            .recording
            .ignored_windows
            .push("Keepass".to_string());

        settings.apply_social_capture_policy(&["whatsapp".to_string()]);

        assert!(!settings
            .recording
            .ignored_windows
            .iter()
            .any(|value| value.eq_ignore_ascii_case("WhatsApp")));
        assert!(!settings
            .recording
            .ignored_urls
            .iter()
            .any(|value| value.eq_ignore_ascii_case("whatsapp.com")));
        assert!(settings
            .recording
            .ignored_windows
            .iter()
            .any(|value| value.eq_ignore_ascii_case("YouTube")));
        assert!(settings
            .recording
            .ignored_windows
            .iter()
            .any(|value| value == "Keepass"));
        assert!(settings.recording.included_windows.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn auto_update_defaults_to_enabled() {
        assert!(SettingsStore::default().auto_update);
    }

    #[test]
    fn ai_pii_is_opt_in_for_new_and_existing_settings() {
        let defaults = SettingsStore::default();
        assert!(!defaults.recording.async_pii_redaction);
        assert!(
            !defaults
                .to_dystil_capture_config(std::path::PathBuf::from("/tmp/dystil-test"))
                .async_pii_redaction
        );

        let migrated = SettingsStore::sanitize_legacy_fields(json!({
            "asyncPiiRedaction": true
        }));
        assert_eq!(migrated.get("asyncPiiRedaction"), Some(&json!(false)));
        assert_eq!(migrated.get(AI_PII_EXPLICIT_OPT_IN_KEY), Some(&json!(true)));

        let opted_in = SettingsStore::sanitize_legacy_fields(json!({
            "asyncPiiRedaction": true,
            AI_PII_EXPLICIT_OPT_IN_KEY: true
        }));
        assert_eq!(opted_in.get("asyncPiiRedaction"), Some(&json!(true)));
    }

    #[test]
    fn missing_auto_update_deserializes_enabled() {
        let settings: SettingsStore = serde_json::from_value(json!({})).unwrap();

        assert!(settings.auto_update);
    }

    #[test]
    fn retention_defaults_to_three_months_and_replaces_ghost_fields() {
        let migrated = SettingsStore::sanitize_legacy_fields(json!({
            "localRetentionEnabled": true,
            "localRetentionDays": 14,
            "localRetentionMode": "media"
        }));
        let settings: SettingsStore = serde_json::from_value(migrated.clone()).unwrap();

        assert_eq!(settings.retention_days, 90);
        assert_eq!(migrated.get("retentionDays"), Some(&json!(90)));
        assert!(migrated.get("localRetentionDays").is_none());
    }

    #[test]
    fn sync_consent_defaults_to_local_only_for_legacy_settings() {
        let settings: SettingsStore = serde_json::from_value(json!({})).unwrap();
        assert_eq!(settings.sync_consent, SyncConsent::default());
    }

    #[test]
    fn effective_sync_consent_matches_the_compiled_policy() {
        let effective = SyncConsent::default().effective();
        assert_eq!(effective.segments, cfg!(feature = "enterprise-client"));
        assert_eq!(effective.screenshots, cfg!(feature = "enterprise-client"));
    }

    #[test]
    fn screenshot_sync_requires_segment_sync() {
        assert!(SyncConsent {
            segments: false,
            screenshots: true,
        }
        .validate()
        .is_err());
        assert!(SyncConsent {
            segments: true,
            screenshots: true,
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn explicit_auto_update_true_is_respected() {
        let settings: SettingsStore = serde_json::from_value(json!({
            "autoUpdate": true
        }))
        .unwrap();

        assert!(settings.auto_update);
    }

    // ---- Settings-loss recovery ----

    fn write_store(dir: &Path, contents: &Value) -> std::path::PathBuf {
        let p = dir.join("store.bin");
        std::fs::write(&p, serde_json::to_vec_pretty(contents).unwrap()).unwrap();
        p
    }

    fn write_last_good(dir: &Path, contents: &Value) -> std::path::PathBuf {
        let p = dir.join("store.bin.last-good");
        std::fs::write(&p, serde_json::to_vec_pretty(contents).unwrap()).unwrap();
        p
    }

    fn healthy_settings() -> Value {
        json!({"settings": {"ocrEngine": "apple-native", "dataDir": "/tmp/dystil"}})
    }

    fn degraded_settings() -> Value {
        json!({"settings": {}})
    }

    #[test]
    fn store_json_is_healthy_recognises_healthy() {
        let healthy = serde_json::to_vec(&healthy_settings()).unwrap();
        assert!(store_json_is_healthy(&healthy));
    }

    #[test]
    fn store_json_is_healthy_rejects_empty_or_missing() {
        let empty_obj = serde_json::to_vec(&degraded_settings()).unwrap();
        let no_settings = serde_json::to_vec(&json!({})).unwrap();
        let invalid_json = b"{not json".to_vec();
        assert!(!store_json_is_healthy(&empty_obj));
        assert!(!store_json_is_healthy(&no_settings));
        assert!(!store_json_is_healthy(&invalid_json));
    }

    #[test]
    fn snapshot_last_good_writes_when_healthy() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store(tmp.path(), &healthy_settings());
        snapshot_last_good(&store_path);
        let lg = store_path.with_extension(LAST_GOOD_SUFFIX);
        assert!(lg.exists(), "should have written .last-good");
        let lg_data = std::fs::read(&lg).unwrap();
        assert!(store_json_is_healthy(&lg_data));
    }

    #[test]
    fn snapshot_last_good_skips_degraded() {
        // L1's contract: never freeze a wiped state as the recovery source.
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store(tmp.path(), &degraded_settings());
        snapshot_last_good(&store_path);
        let lg = store_path.with_extension(LAST_GOOD_SUFFIX);
        assert!(!lg.exists(), "must not snapshot a degraded store");
    }

    #[test]
    fn auto_restore_recovers_wiped_store_from_last_good() {
        let tmp = tempfile::tempdir().unwrap();
        // Simulate the wipe — current file has empty settings, last-good is healthy
        let store_path = write_store(tmp.path(), &degraded_settings());
        write_last_good(tmp.path(), &healthy_settings());

        let restored = auto_restore_if_wiped(&store_path);
        assert!(restored, "should report a restore happened");

        let now = std::fs::read(&store_path).unwrap();
        assert!(
            store_json_is_healthy(&now),
            "store must be healthy after restore"
        );

        // Forensic copy of the wiped file must exist
        let entries: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().into_string().unwrap_or_default())
            .filter(|n| n.contains("pre-restore-"))
            .collect();
        assert_eq!(
            entries.len(),
            1,
            "expected 1 pre-restore backup, got {entries:?}"
        );
    }

    #[test]
    fn auto_restore_noop_when_current_is_healthy() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store(tmp.path(), &healthy_settings());
        // Even if last-good exists, current is fine — don't touch.
        write_last_good(tmp.path(), &healthy_settings());

        let restored = auto_restore_if_wiped(&store_path);
        assert!(!restored);
    }

    #[test]
    fn auto_restore_noop_when_last_good_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store(tmp.path(), &degraded_settings());
        let restored = auto_restore_if_wiped(&store_path);
        assert!(!restored, "no last-good means no restore");
    }

    #[test]
    fn auto_restore_noop_when_last_good_is_also_degraded() {
        // Defense: even if .last-good somehow got written wiped (shouldn't
        // happen due to L1's guard, but belt + suspenders), don't restore
        // garbage over garbage.
        let tmp = tempfile::tempdir().unwrap();
        let store_path = write_store(tmp.path(), &degraded_settings());
        write_last_good(tmp.path(), &degraded_settings());
        let restored = auto_restore_if_wiped(&store_path);
        assert!(!restored);
    }

    #[test]
    fn auto_restore_skips_encrypted_files() {
        // L2 must not try to "restore" over a still-encrypted blob — the
        // decrypt path owns that case.
        let tmp = tempfile::tempdir().unwrap();
        let store_path = tmp.path().join("store.bin");
        let mut blob = STORE_MAGIC.to_vec();
        blob.extend_from_slice(b"<<encrypted ciphertext>>");
        std::fs::write(&store_path, &blob).unwrap();
        write_last_good(tmp.path(), &healthy_settings());

        let restored = auto_restore_if_wiped(&store_path);
        assert!(
            !restored,
            "encrypted file must be left for the decrypt path"
        );
        // And the file must be unchanged
        assert_eq!(std::fs::read(&store_path).unwrap(), blob);
    }

    #[test]
    fn test_deserialize_settings_with_null_fields() {
        let json_data = json!({
            "recording": {
                "video": true
            },
            "user": null
        });

        let settings: Result<SettingsStore, _> = serde_json::from_value(json_data);
        if let Err(e) = &settings {
            println!("Deser error: {:?}", e);
        }
        assert!(
            settings.is_ok(),
            "Failed to deserialize settings with null fields"
        );
        let settings = settings.unwrap();

        assert_eq!(settings.user.token, None);
    }
}
