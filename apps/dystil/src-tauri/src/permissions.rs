use crate::tray::QUIT_REQUESTED;
use serde::{Deserialize, Serialize};
use specta::Type;
use tracing::{debug, error, info, warn};

#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(rename_all = "camelCase")]
pub enum OSPermission {
    ScreenRecording,
    Accessibility,
    Automation,
    InputMonitoring,
    Calendar,
}

#[tauri::command(async)]
#[specta::specta]
#[allow(unused_variables)] // permission used on macOS
pub fn open_permission_settings(permission: OSPermission) {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        match permission {
            OSPermission::ScreenRecording => Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture",
                )
                .spawn()
                .expect("Failed to open Screen Recording settings"),
            OSPermission::Accessibility => Command::new("open")
                .arg(
                    "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
                )
                .spawn()
                .expect("Failed to open Accessibility settings"),
            OSPermission::Automation => Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation")
                .spawn()
                .expect("Failed to open Automation settings"),
            OSPermission::InputMonitoring => Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
                .spawn()
                .expect("Failed to open Input Monitoring settings"),
            OSPermission::Calendar => Command::new("open")
                .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Calendars")
                .spawn()
                .expect("Failed to open Calendar settings"),
        };
    }
}

#[tauri::command]
#[specta::specta]
#[allow(unused_variables)] // permission used on macOS
pub async fn request_permission(app: tauri::AppHandle, permission: OSPermission) {
    #[cfg(target_os = "macos")]
    {
        match permission {
            OSPermission::ScreenRecording => {
                use core_graphics_helmer_fork::access::ScreenCaptureAccess;
                if !ScreenCaptureAccess.preflight() {
                    // Open System Settings first so it's in the background,
                    // then request() shows the native modal on top (macOS 15+).
                    // If the user dismisses the modal, Settings is already open.
                    open_permission_settings(OSPermission::ScreenRecording);
                    ScreenCaptureAccess.request();
                }
            }
            OSPermission::Accessibility => {
                // Request accessibility permission (shows system prompt)
                // AXIsProcessTrustedWithOptions with kAXTrustedCheckOptionPrompt
                // handles both NotDetermined and Denied cases on macOS
                request_accessibility_permission();
            }
            OSPermission::Automation => {
                // Open Automation settings — user must toggle manually
                open_permission_settings(OSPermission::Automation);
            }
            OSPermission::InputMonitoring => {
                // Defer to the dedicated request flow (opens Settings + calls
                // CGRequestListenEventAccess). No probe tap is created — the
                // check reads from INPUT_MONITORING_GROUND_TRUTH or preflight.
                let _ = request_input_monitoring_permission().await;
            }
            OSPermission::Calendar => {
                open_permission_settings(OSPermission::Calendar);
            }
        }
    }
}

// Accessibility permission APIs using ApplicationServices framework
#[cfg(target_os = "macos")]
mod accessibility {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: *const std::ffi::c_void) -> bool;
        static kAXTrustedCheckOptionPrompt: *const std::ffi::c_void;
    }

    /// Check accessibility permission and show system prompt if not granted
    pub fn request_with_prompt() -> bool {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt as *const _);
            let value = CFBoolean::true_value();
            let dict = CFDictionary::from_CFType_pairs(&[(key, value)]);
            AXIsProcessTrustedWithOptions(dict.as_concrete_TypeRef() as *const _)
        }
    }
}

// ---------------------------------------------------------------------------
// Inline permission checks (replaces dystil_core::permissions)
// ---------------------------------------------------------------------------

/// Returns `true` when AXIsProcessTrusted() says the process has AX permission.
#[cfg(target_os = "macos")]
pub fn check_accessibility_inline() -> bool {
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }
    unsafe { AXIsProcessTrusted() }
}

/// Returns `true` when CGPreflightScreenCaptureAccess() grants screen recording.
#[cfg(target_os = "macos")]
pub fn check_screen_recording_inline() -> bool {
    use core_graphics_helmer_fork::access::ScreenCaptureAccess;
    ScreenCaptureAccess.preflight()
}

#[cfg(target_os = "macos")]
fn check_accessibility_permission() -> OSPermissionStatus {
    if check_accessibility_inline() {
        OSPermissionStatus::Granted
    } else {
        OSPermissionStatus::Denied
    }
}

#[cfg(target_os = "macos")]
fn request_accessibility_permission() {
    accessibility::request_with_prompt();
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Type)]
#[serde(rename_all = "camelCase")]
pub enum OSPermissionStatus {
    // This platform does not require this permission
    NotNeeded,
    // The user has neither granted nor denied permission
    Empty,
    // The user has explicitly granted permission
    Granted,
    // The user has denied permission, or has granted it but not yet restarted
    Denied,
}

impl OSPermissionStatus {
    pub fn permitted(&self) -> bool {
        matches!(self, Self::NotNeeded | Self::Granted)
    }
}

#[derive(Serialize, Deserialize, Debug, Type)]
#[serde(rename_all = "camelCase")]
pub struct OSPermissionsCheck {
    pub screen_recording: OSPermissionStatus,
    pub accessibility: OSPermissionStatus,
}

impl OSPermissionsCheck {
    pub fn necessary_granted(&self, screenshots_enabled: bool) -> bool {
        self.accessibility.permitted()
            && (!screenshots_enabled || self.screen_recording.permitted())
    }
}

/// Check only screen recording permission (no dialog trigger).
#[tauri::command(async)]
#[specta::specta]
pub fn check_screen_recording_permission() -> OSPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        if check_screen_recording_inline() {
            OSPermissionStatus::Granted
        } else {
            OSPermissionStatus::Denied
        }
    }
    #[cfg(not(target_os = "macos"))]
    OSPermissionStatus::NotNeeded
}

/// Check only accessibility permission.
#[tauri::command(async)]
#[specta::specta]
pub fn check_accessibility_permission_cmd() -> OSPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        check_accessibility_permission()
    }
    #[cfg(not(target_os = "macos"))]
    OSPermissionStatus::NotNeeded
}

/// Check Input Monitoring permission (macOS only).
///
/// Input Monitoring is a TCC category separate from Accessibility. Without
/// it the recorder can still capture clipboard (via NSPasteboard polling)
/// and app/window switches, but not keystrokes or clicks. Polling-safe —
/// uses the preflight variant that doesn't trigger the system prompt.
#[tauri::command(async)]
#[specta::specta]
pub fn check_input_monitoring_permission_cmd() -> OSPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        if dystil_capture::a11y::check_input_monitoring() {
            OSPermissionStatus::Granted
        } else {
            // The TCC preflight API doesn't distinguish NotDetermined from
            // Denied — both return false. We surface as Empty so the UI
            // shows "request" rather than "open settings"; the request
            // flow handles both cases identically (prompt on first call,
            // open System Settings as fallback).
            OSPermissionStatus::Empty
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        OSPermissionStatus::NotNeeded
    }
}

/// Request Input Monitoring permission (macOS only).
///
/// Calls `cg_access::listen_request()` to trigger the system permission
/// flow. On first call this either shows the native prompt (if NotDetermined)
/// or silently no-ops (if already Denied — macOS doesn't re-prompt). For
/// reliability we also open System Settings → Input Monitoring so the user
/// can grant manually if the prompt didn't appear.
///
/// Returns the post-request permission status so the UI can update without
/// waiting for the next poll.
#[tauri::command(async)]
#[specta::specta]
pub async fn request_input_monitoring_permission() -> OSPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        if dystil_capture::a11y::check_input_monitoring() {
            return OSPermissionStatus::Granted;
        }
        // Open the Input Monitoring pane first so when the OS prompt
        // appears it's layered on top of the settings UI the user lands
        // in if they dismiss the prompt. Matches the pattern used by
        // request_permission for ScreenRecording above.
        let _ = Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();
        // Triggers the native consent prompt the first time the process
        // calls it. Subsequent calls are no-ops if denied — the user has
        // to enable from System Settings, which we just opened.
        if dystil_capture::a11y::request_input_monitoring() {
            OSPermissionStatus::Granted
        } else {
            OSPermissionStatus::Denied
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        OSPermissionStatus::NotNeeded
    }
}

#[tauri::command(async)]
#[specta::specta]
pub async fn check_permission(permission: OSPermission) -> OSPermissionStatus {
    #[cfg(target_os = "macos")]
    {
        match permission {
            OSPermission::ScreenRecording => check_screen_recording_permission(),
            OSPermission::Accessibility => check_accessibility_permission(),
            OSPermission::InputMonitoring => {
                if dystil_capture::a11y::check_input_monitoring() {
                    OSPermissionStatus::Granted
                } else {
                    OSPermissionStatus::Denied
                }
            }
            OSPermission::Calendar => {
                // Calendar is excluded from the Dystil product.
                OSPermissionStatus::Denied
            }
            OSPermission::Automation => OSPermissionStatus::Denied,
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = permission;
        OSPermissionStatus::NotNeeded
    }
}

#[tauri::command(async)]
#[specta::specta]
pub async fn reset_permission(
    app: tauri::AppHandle,
    permission: OSPermission,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let service = match &permission {
            OSPermission::ScreenRecording => "ScreenCapture",
            OSPermission::Accessibility => "Accessibility",
            OSPermission::InputMonitoring => "ListenEvent",
            OSPermission::Calendar => "Calendar",
            OSPermission::Automation => {
                open_permission_settings(OSPermission::Automation);
                return Ok(());
            }
        };

        let bundle_id = app.config().identifier.as_str();
        if bundle_id.is_empty() {
            return Err("no bundle identifier in app config".to_string());
        }

        let output = Command::new("tccutil")
            .args(["reset", service, bundle_id])
            .output()
            .map_err(|e| format!("failed to run tccutil: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!(
                "tccutil reset {} failed: {}",
                service,
                stderr.trim()
            ));
        }

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, permission);
        Ok(())
    }
}

/// Reset a permission using tccutil and re-request it
/// This removes the app from the TCC database and triggers a fresh permission request
#[tauri::command(async)]
#[specta::specta]
pub async fn reset_and_request_permission(
    app: tauri::AppHandle,
    permission: OSPermission,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;
        use tokio::time::{sleep, Duration};

        let service = match &permission {
            OSPermission::ScreenRecording => "ScreenCapture",
            OSPermission::Accessibility => "Accessibility",
            OSPermission::InputMonitoring => "ListenEvent",
            OSPermission::Calendar => "Calendar",
            OSPermission::Automation => {
                // Automation doesn't use tccutil reset flow — just open settings
                open_permission_settings(OSPermission::Automation);
                return Ok(());
            }
        };

        // Get bundle identifier from Tauri config (handles dev/beta/prod automatically)
        let bundle_id = app.config().identifier.as_str();

        // Reset permission using tccutil - ONLY for this app's bundle ID
        let output = Command::new("tccutil")
            .args(["reset", service, bundle_id])
            .output()
            .map_err(|e| format!("failed to run tccutil: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            warn!("tccutil reset returned non-zero: {}", stderr);
            // Don't fail - tccutil might return non-zero even when it works
        }

        // Wait for TCC database to update
        sleep(Duration::from_millis(500)).await;

        // Re-request the permission
        request_permission(app, permission).await;

        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, permission);
        Ok(())
    }
}

/// Check all permissions and return which ones are missing
#[tauri::command(async)]
#[specta::specta]
pub fn get_missing_permissions(app: tauri::AppHandle) -> Vec<OSPermission> {
    #[cfg(target_os = "macos")]
    {
        let mut missing = Vec::new();
        let check = do_permissions_check(false);
        let screenshots_enabled = crate::store::SettingsStore::get(&app)
            .ok()
            .flatten()
            .is_some_and(|settings| !settings.recording.disable_vision);

        if screenshots_enabled && !check.screen_recording.permitted() {
            missing.push(OSPermission::ScreenRecording);
        }
        if !check.accessibility.permitted() {
            missing.push(OSPermission::Accessibility);
        }

        missing
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        Vec::new()
    }
}

#[tauri::command(async)]
#[specta::specta]
#[allow(unused_variables)] // initial_check used on macOS
pub fn do_permissions_check(initial_check: bool) -> OSPermissionsCheck {
    #[cfg(target_os = "macos")]
    {
        OSPermissionsCheck {
            screen_recording: {
                use core_graphics_helmer_fork::access::ScreenCaptureAccess;
                let result = ScreenCaptureAccess.preflight();
                match (result, initial_check) {
                    (true, _) => OSPermissionStatus::Granted,
                    (false, true) => OSPermissionStatus::Empty,
                    (false, false) => OSPermissionStatus::Denied,
                }
            },
            accessibility: check_accessibility_permission(),
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        OSPermissionsCheck {
            screen_recording: OSPermissionStatus::NotNeeded,
            accessibility: OSPermissionStatus::NotNeeded,
        }
    }
}

/// Known Chromium-based browsers that use AppleScript for incognito detection
/// and (in Arc's case) URL capture. Each needs its own Automation permission.
#[cfg(target_os = "macos")]
#[allow(dead_code)]
struct ChromiumBrowserInfo {
    name: &'static str,
    bundle_id: &'static str,
    app_path: &'static str,
    process_name: &'static str,
}

#[cfg(target_os = "macos")]
const CHROMIUM_BROWSERS: &[ChromiumBrowserInfo] = &[
    ChromiumBrowserInfo {
        name: "Arc",
        bundle_id: "company.thebrowser.Browser",
        app_path: "/Applications/Arc.app",
        process_name: "Arc",
    },
    ChromiumBrowserInfo {
        name: "Google Chrome",
        bundle_id: "com.google.Chrome",
        app_path: "/Applications/Google Chrome.app",
        process_name: "Google Chrome",
    },
    ChromiumBrowserInfo {
        name: "Brave Browser",
        bundle_id: "com.brave.Browser",
        app_path: "/Applications/Brave Browser.app",
        process_name: "Brave Browser",
    },
    ChromiumBrowserInfo {
        name: "Microsoft Edge",
        bundle_id: "com.microsoft.edgemac",
        app_path: "/Applications/Microsoft Edge.app",
        process_name: "Microsoft Edge",
    },
    ChromiumBrowserInfo {
        name: "Vivaldi",
        bundle_id: "com.vivaldi.Vivaldi",
        app_path: "/Applications/Vivaldi.app",
        process_name: "Vivaldi",
    },
    ChromiumBrowserInfo {
        name: "Opera",
        bundle_id: "com.operasoftware.Opera",
        app_path: "/Applications/Opera.app",
        process_name: "Opera",
    },
    ChromiumBrowserInfo {
        name: "Chromium",
        bundle_id: "org.chromium.Chromium",
        app_path: "/Applications/Chromium.app",
        process_name: "Chromium",
    },
];

/// Check if Arc browser is installed (macOS only)
#[tauri::command(async)]
#[specta::specta]
pub fn check_arc_installed() -> bool {
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new("/Applications/Arc.app").exists()
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Returns the names of installed Chromium browsers that need Automation permission
#[allow(dead_code)]
#[tauri::command(async)]
#[specta::specta]
pub fn get_installed_browsers() -> Vec<String> {
    #[cfg(target_os = "macos")]
    {
        CHROMIUM_BROWSERS
            .iter()
            .filter(|b| std::path::Path::new(b.app_path).exists())
            .map(|b| b.name.to_string())
            .collect()
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Check if Automation permission is granted for all installed Chromium browsers.
/// Returns true only if ALL installed browsers have automation granted.
#[allow(dead_code)]
#[tauri::command(async)]
#[specta::specta]
pub fn check_browsers_automation_permission(_app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        let installed: Vec<&ChromiumBrowserInfo> = CHROMIUM_BROWSERS
            .iter()
            .filter(|b| std::path::Path::new(b.app_path).exists())
            .collect();

        if installed.is_empty() {
            return true;
        }

        if is_app_bundle() {
            installed
                .iter()
                .all(|b| ae_check_automation_direct(b.bundle_id, false) == 0)
        } else {
            // Dev mode: just check Arc as before (launchctl approach doesn't scale to N browsers)
            run_self_detached("--check-arc-automation")
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Request Automation permission for installed Chromium browsers that are already running.
/// Never force-launches browsers — only prompts for ones the user already has open.
/// Opens System Settings > Automation as fallback for browsers not running.
#[allow(dead_code)]
#[tauri::command(async)]
#[specta::specta]
pub fn request_browsers_automation_permission(_app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let installed: Vec<&ChromiumBrowserInfo> = CHROMIUM_BROWSERS
            .iter()
            .filter(|b| std::path::Path::new(b.app_path).exists())
            .collect();

        if installed.is_empty() {
            return true;
        }

        if is_app_bundle() {
            let mut all_granted = true;
            let mut prompted_any = false;

            for browser in &installed {
                // Only prompt browsers that are already running — never force-launch (#2510)
                let running = Command::new("pgrep")
                    .args(["-x", browser.process_name])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);

                if running {
                    let result = ae_check_automation_direct(browser.bundle_id, true);
                    if result != 0 {
                        all_granted = false;
                    }
                    prompted_any = true;
                } else {
                    // Not running — silently check without prompting
                    let result = ae_check_automation_direct(browser.bundle_id, false);
                    if result != 0 {
                        all_granted = false;
                    }
                }
            }

            // Only open System Settings if we couldn't prompt any running browser
            if !all_granted && !prompted_any {
                open_permission_settings(OSPermission::Automation);
            }
            all_granted
        } else {
            open_permission_settings(OSPermission::Automation);
            run_self_detached_fire_and_forget("--trigger-arc-automation");
            false
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Per-browser automation status: "granted", "denied", or "not_asked".
/// Also includes whether the browser is currently running.
#[allow(dead_code)]
#[derive(Serialize, Deserialize, Type, Clone)]
#[serde(rename_all = "camelCase")]
pub struct BrowserAutomationStatus {
    pub name: String,
    pub status: String, // "granted" | "denied" | "not_asked"
    pub running: bool,
}

/// Returns per-browser automation permission status for all installed Chromium browsers.
#[allow(dead_code)]
#[tauri::command(async)]
#[specta::specta]
pub fn get_browsers_automation_status() -> Vec<BrowserAutomationStatus> {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        CHROMIUM_BROWSERS
            .iter()
            .filter(|b| std::path::Path::new(b.app_path).exists())
            .map(|b| {
                let running = Command::new("pgrep")
                    .args(["-x", b.process_name])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);

                let status = if is_app_bundle() {
                    match ae_check_automation_direct(b.bundle_id, false) {
                        0 => "granted",
                        -1744 => "denied",
                        _ => "not_asked",
                    }
                } else {
                    "not_asked" // can't reliably check in dev mode
                };

                BrowserAutomationStatus {
                    name: b.name.to_string(),
                    status: status.to_string(),
                    running,
                }
            })
            .collect()
    }

    #[cfg(not(target_os = "macos"))]
    {
        Vec::new()
    }
}

/// Request automation permission for a single browser by name.
/// Returns the new status: "granted", "denied", or "not_asked".
#[allow(dead_code)]
#[tauri::command(async)]
#[specta::specta]
pub fn request_single_browser_automation(browser_name: String) -> String {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        let browser = CHROMIUM_BROWSERS.iter().find(|b| b.name == browser_name);

        let Some(browser) = browser else {
            return "not_asked".to_string();
        };

        if !std::path::Path::new(browser.app_path).exists() {
            return "not_asked".to_string();
        }

        let running = Command::new("pgrep")
            .args(["-x", browser.process_name])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !running {
            // Can't prompt — open System Settings as fallback
            open_permission_settings(OSPermission::Automation);
            return "not_asked".to_string();
        }

        if is_app_bundle() {
            match ae_check_automation_direct(browser.bundle_id, true) {
                0 => "granted".to_string(),
                -1744 => "denied".to_string(),
                _ => "not_asked".to_string(),
            }
        } else {
            open_permission_settings(OSPermission::Automation);
            "not_asked".to_string()
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = browser_name;
        "not_asked".to_string()
    }
}

/// Check if Automation permission for Arc is already granted.
/// In production (.app bundle): uses direct FFI check (correct identity, no Terminal).
/// In dev mode: runs the binary itself via launchctl (detached from Terminal) so
/// macOS TCC checks the binary's own identity, not Terminal's.
#[tauri::command(async)]
#[specta::specta]
pub fn check_arc_automation_permission(_app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        let target = "company.thebrowser.Browser";
        if is_app_bundle() {
            ae_check_automation_direct(target, false) == 0
        } else {
            // Dev mode: run self via launchctl to check without Terminal inheritance
            run_self_detached("--check-arc-automation")
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

/// Detect whether we're running as a .app bundle (production) or standalone binary (dev mode).
#[cfg(target_os = "macos")]
fn is_app_bundle() -> bool {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().contains(".app/"))
        .unwrap_or(false)
}

/// Call AEDeterminePermissionToAutomateTarget directly from the current process via FFI.
/// Returns the raw OSStatus: 0 = granted, -1744 = denied, -1745 = not yet asked.
/// When `ask_user` is true AND permission was not yet asked, macOS shows a system prompt.
/// Public so main.rs can call it for --check-arc-automation / --trigger-arc-automation.
#[cfg(target_os = "macos")]
pub fn ae_check_automation_direct(target_bundle_id: &str, ask_user: bool) -> i32 {
    use std::ffi::c_void;

    #[repr(C)]
    struct AEDesc {
        descriptor_type: u32,
        data_handle: *mut c_void,
    }

    #[link(name = "Carbon", kind = "framework")]
    extern "C" {
        fn AECreateDesc(
            type_code: u32,
            data_ptr: *const u8,
            data_size: isize,
            result: *mut AEDesc,
        ) -> i16;
        fn AEDeterminePermissionToAutomateTarget(
            target: *const AEDesc,
            the_ae_event_class: u32,
            the_ae_event_id: u32,
            ask_user_if_needed: u8,
        ) -> i32;
        fn AEDisposeDesc(the_ae_desc: *mut AEDesc) -> i16;
    }

    // 'bund' = typeApplicationBundleID
    const TYPE_BUND: u32 = u32::from_be_bytes(*b"bund");
    // '****' = typeWildCard
    const TYPE_WILD: u32 = u32::from_be_bytes(*b"****");

    unsafe {
        let mut desc = AEDesc {
            descriptor_type: 0,
            data_handle: std::ptr::null_mut(),
        };
        let data = target_bundle_id.as_bytes();
        let err = AECreateDesc(TYPE_BUND, data.as_ptr(), data.len() as isize, &mut desc);
        if err != 0 {
            warn!("AECreateDesc failed: {}", err);
            return -1;
        }
        let result = AEDeterminePermissionToAutomateTarget(
            &desc,
            TYPE_WILD,
            TYPE_WILD,
            if ask_user { 1 } else { 0 },
        );
        AEDisposeDesc(&mut desc);
        result
    }
}

/// Run the current binary itself via launchctl (detached from Terminal) with a flag.
/// Waits for the result and returns true if the output is "granted".
/// Used in dev mode so macOS TCC checks the binary's own identity.
#[cfg(target_os = "macos")]
fn run_self_detached(flag: &str) -> bool {
    use std::process::Command;
    use std::time::Duration;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            warn!("failed to get current exe: {}", e);
            return false;
        }
    };

    let label = format!("dystil.self-{}", flag.trim_start_matches("--"));
    let result_path = format!("/tmp/dystil_self_{}_result", flag.trim_start_matches("--"));

    let _ = std::fs::remove_file(&result_path);
    let _ = Command::new("launchctl").args(["remove", &label]).output();

    let exe_str = exe.to_string_lossy().to_string();
    let submit = Command::new("launchctl")
        .args([
            "submit",
            "-l",
            &label,
            "-o",
            &result_path,
            "--",
            &exe_str,
            flag,
        ])
        .output();

    if submit.is_err() {
        warn!("failed to submit self via launchctl with {}", flag);
        return false;
    }

    // Wait for result (binary exits quickly for --check, so 5s is plenty)
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        if std::path::Path::new(&result_path).exists() {
            if let Ok(content) = std::fs::read_to_string(&result_path) {
                if !content.is_empty() {
                    let _ = Command::new("launchctl").args(["remove", &label]).output();
                    return content.trim() == "granted";
                }
            }
        }
    }

    let _ = Command::new("launchctl").args(["remove", &label]).output();
    debug!("self detached {} timed out", flag);
    false
}

/// Fire-and-forget: submit the binary via launchctl with a flag, don't wait for result.
/// Used for --trigger-arc-automation where the user needs to respond to a prompt.
#[cfg(target_os = "macos")]
fn run_self_detached_fire_and_forget(flag: &str) {
    use std::process::Command;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            warn!("failed to get current exe: {}", e);
            return;
        }
    };

    let label = format!("dystil.self-{}", flag.trim_start_matches("--"));
    let result_path = format!("/tmp/dystil_self_{}_result", flag.trim_start_matches("--"));

    let _ = std::fs::remove_file(&result_path);
    let _ = Command::new("launchctl").args(["remove", &label]).output();

    let exe_str = exe.to_string_lossy().to_string();
    let submit = Command::new("launchctl")
        .args([
            "submit",
            "-l",
            &label,
            "-o",
            &result_path,
            "--",
            &exe_str,
            flag,
        ])
        .output();

    if let Err(e) = submit {
        warn!("failed to submit self via launchctl: {}", e);
    }
}

/// Request macOS Automation permission for Arc browser.
/// In production: triggers "dystil wants to control Arc" prompt via direct FFI.
/// In dev mode: runs the binary itself via launchctl to trigger the prompt with
/// the correct binary identity (not Terminal's). Also opens System Settings as fallback.
#[tauri::command(async)]
#[specta::specta]
pub fn request_arc_automation_permission(_app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        // Only prompt if Arc is already running — never force-launch (#2510)
        let arc_running = Command::new("pgrep")
            .args(["-x", "Arc"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if is_app_bundle() {
            if arc_running {
                let result = ae_check_automation_direct("company.thebrowser.Browser", true);
                if result != 0 {
                    open_permission_settings(OSPermission::Automation);
                }
                result == 0
            } else {
                // Arc not running — open System Settings instead of force-launching
                open_permission_settings(OSPermission::Automation);
                false
            }
        } else {
            open_permission_settings(OSPermission::Automation);
            if arc_running {
                run_self_detached_fire_and_forget("--trigger-arc-automation");
            }
            false
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

// NOTE: Runtime permission monitoring is now handled by
// `dystil-engine::permission_monitor` which emits `permission_lost` /
// `permission_restored` events on the shared permission flow. The Tauri app
// refreshes this state when the recovery window is completed. This module
// keeps the synchronous TCC/AV check helpers used by the onboarding UI
// and the preflight startup check.
