//! The core recording settings type shared across all dystil components.

use serde::{Deserialize, Serialize};

/// A single schedule rule: a day-of-week + time range + what to record.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleRule {
    /// Day of week: 0 = Monday, 6 = Sunday
    pub day_of_week: u8,
    /// Start time in "HH:MM" (24h format, local time)
    pub start_time: String,
    /// End time in "HH:MM" (24h format, local time)
    pub end_time: String,
    /// What to record: "all" or "screen_only"
    pub record_mode: String,
}

/// The single source of truth for recording/capture configuration.
///
/// Used by:
/// - **Desktop app**: embedded inside `SettingsStore` via `#[serde(flatten)]`
/// - **CLI**: built from command-line args or loaded from `~/.dystil/config.toml`
/// - **Engine**: consumed directly for visual, accessibility, and UI capture
///
/// All field names use `camelCase` serde rename to match the existing frontend
/// JSON schema (store.bin). This ensures backwards compatibility — existing
/// `store.bin` files deserialize without migration.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, specta::Type)]
#[serde(default)]
pub struct RecordingSettings {
    // ── Vision ─────────────────────────────────────────────────────────
    /// Disable all screen capture.
    #[serde(rename = "disableVision")]
    pub disable_vision: bool,

    /// Disable the timeline / rewind feature. When true, the engine skips
    /// timeline-only work: warming the hot frame cache from the DB at startup
    /// and buffering captured frames into the in-memory hot cache that
    /// only the timeline streaming endpoint reads.
    #[serde(rename = "disableTimeline", default)]
    pub disable_timeline: bool,

    /// Specific monitor IDs to capture.
    #[serde(rename = "monitorIds")]
    pub monitor_ids: Vec<String>,

    /// Capture from all connected monitors.
    #[serde(rename = "useAllMonitors")]
    pub use_all_monitors: bool,

    /// Video quality preset: "low", "balanced", "high", "max".
    #[serde(rename = "videoQuality")]
    pub video_quality: String,

    /// Maximum width for stored snapshots. Images wider than this are downscaled
    /// (preserving aspect ratio) before JPEG encoding. 0 = no limit (store at
    /// native resolution). Default: 1920.
    #[serde(rename = "maxSnapshotWidth", default = "default_max_snapshot_width")]
    pub max_snapshot_width: u32,

    /// Skip the background JPEG->MP4 snapshot compaction worker.
    /// Use when the MP4 timeline UI is not used, e.g. task-mining tools
    /// that consume frame_text / ui_events only.
    /// Side effect: JPEGs are not compacted, so disk usage depends on retention.
    #[serde(rename = "disableSnapshotCompaction", default)]
    pub disable_snapshot_compaction: bool,

    /// Skip the v2 meeting detector watcher (5s-interval process / AX scan).
    /// Use when meeting detection is not consumed (task-mining, headless analysis,
    /// agents that read frame_text and ui_events only) — avoids the
    /// constant process enumeration + AX tree walk cost.
    /// Side effect: meeting-related DB rows are not generated.
    #[serde(rename = "disableMeetingDetector", default)]
    pub disable_meeting_detector: bool,

    /// Apps / meeting services to exclude from automatic meeting detection
    /// while leaving detection on for everything else. Case-insensitive
    /// substring match against the running app's name/process AND the matched
    /// detection profile's identifiers (native names + browser URL patterns),
    /// so an entry can be what the user sees ("Discord") or a service domain
    /// ("meet.google.com"). Use when one app trips the detector spuriously
    /// (an always-open Teams, a Discord call you don't want logged) but you
    /// still want Zoom/Meet/etc. detected. Empty = detect all known apps.
    #[serde(rename = "ignoredMeetingApps", default)]
    pub ignored_meeting_apps: Vec<String>,

    /// Legacy key-trigger override retained for settings compatibility.
    /// Recording sessions keep keyboard-triggered capture on; raw key/text DB
    /// rows are controlled separately by `disableKeyboardCapture`.
    #[serde(rename = "captureOnKeystroke", default)]
    pub capture_on_keystroke: Option<bool>,

    /// Override `EventDrivenCaptureConfig::capture_on_clipboard`.
    /// None = engine default (false). When true, clipboard changes fire a
    /// paired capture. Clipboard DB rows are still controlled separately by
    /// `disableClipboardCapture`.
    #[serde(rename = "captureOnClipboard", default)]
    pub capture_on_clipboard: Option<bool>,

    /// Override `UiRecorderConfig::capture_scroll`.
    /// None = engine default (false). When true, scroll wheel events are
    /// recorded into `ui_events` so the `ScrollBurstTracker` can fire a
    /// `ScrollStop` trigger at burst-end and link the last scroll row to
    /// the resulting frame. Off by default — wheel ticks fire at ~60Hz
    /// and inflate the table fast.
    #[serde(rename = "captureScroll", default)]
    pub capture_scroll: Option<bool>,

    /// Prioritize mouse/keyboard input latency over a11y event completeness.
    /// Opt-in master switch for the three coordinated optimizations defined on
    /// `UiCaptureConfig.prioritize_input_latency`.
    #[serde(rename = "prioritizeInputLatency", default)]
    pub prioritize_input_latency: bool,

    /// OS thread priority for a11y extraction threads when `prioritize_input_latency`
    /// is true. Values: "normal" / "below_normal" / "lowest" / "idle".
    #[serde(
        rename = "extractionThreadPriority",
        default = "default_extraction_thread_priority"
    )]
    pub extraction_thread_priority: String,

    /// Skip UIA tree captures within this many ms after the most recent input.
    /// 0 disables. Ignored when `prioritize_input_latency` is false.
    #[serde(
        rename = "pauseExtractionOnInputMs",
        default = "default_pause_extraction_on_input_ms"
    )]
    pub pause_extraction_on_input_ms: u64,

    // ── Filters ────────────────────────────────────────────────────────
    /// Window titles to exclude from capture.
    #[serde(rename = "ignoredWindows")]
    pub ignored_windows: Vec<String>,

    /// Window titles to exclusively capture (empty = capture all).
    #[serde(rename = "includedWindows")]
    pub included_windows: Vec<String>,

    /// URLs to exclude from capture.
    #[serde(rename = "ignoredUrls", default)]
    pub ignored_urls: Vec<String>,

    /// Automatically detect and skip incognito / private browsing windows.
    #[serde(rename = "ignoreIncognitoWindows")]
    pub ignore_incognito_windows: bool,

    /// Experimental: pause screen capture when a DRM-protected streaming app
    /// (Netflix, Disney+, etc.) or a remote-desktop client (Omnissa/VMware
    /// Horizon) is focused. These apps blank their windows while screen
    /// recording is active.
    /// Off by default; engine-only pause (no full app shutdown).
    #[serde(rename = "pauseOnDrmContent", default)]
    pub pause_on_drm_content: bool,

    /// Skip persisting clipboard rows/content in the UI recorder. Defaults to
    /// `true` (clipboard DB capture OFF) — passwords / API keys / private keys
    /// frequently pass through the clipboard. Clipboard operations can still
    /// wake event-driven capture when `captureOnClipboard` is enabled.
    #[serde(rename = "disableClipboardCapture", default = "default_true")]
    pub disable_clipboard_capture: bool,

    /// Skip persisting keyboard / typed-text rows in the UI recorder.
    /// Defaults to `true` (keyboard DB capture OFF). Keyboard events still
    /// wake event-driven capture, and the accessibility tree + OCR still
    /// capture on-screen text so Rewind/Ask keep working.
    /// Opt in to keyboard DB rows via the "Capture keyboard" toggle.
    #[serde(rename = "disableKeyboardCapture", default = "default_true")]
    pub disable_keyboard_capture: bool,

    // ── Privacy ────────────────────────────────────────────────────────
    /// Redact personally identifiable information from transcriptions.
    #[serde(rename = "usePiiRemoval")]
    pub use_pii_removal: bool,

    /// Enable the async PII reconciliation worker. When `true`, a
    /// background task runs after capture and OVERWRITES PII in the
    /// source columns of `frames.frame_text` and `ui_events.text_content`. Raw
    /// secrets are gone after the worker processes the row — that's
    /// the contract of the user-facing "AI PII removal" toggle.
    /// Off by default; capture path is unaffected either way. See
    /// `dystil-redact` for the full design.
    #[serde(rename = "asyncPiiRedaction", default)]
    pub async_pii_redaction: bool,

    /// Where text AI PII redaction actually runs. The user-facing
    /// "AI PII removal" toggle is one knob.
    ///
    /// - `"local"` (default): on-device ONNX models. Privacy by
    ///   construction — pixels and text never leave the box. Slower,
    ///   especially on weak hardware (~1-3 s per text row, ~60-180 ms
    ///   per frame).
    /// - `"tinfoil"`: send to the dystil Tinfoil enclave (H200,
    ///   confidential compute). Much faster (~30-100 ms per row /
    ///   frame). Data leaves the device but is end-to-end encrypted
    ///   into an attested confidential-compute enclave that even
    ///   Tinfoil ops can't read into. Requires network +
    ///   `DYSTIL_PRIVACY_FILTER_API_KEY` (or the cloud auth key).
    ///
    /// Note on attestation: the proper attested-transport client
    /// (Tinfoil's secure-client SDK) is Go/Python/JS-only at time of
    /// writing. The Rust adapter currently uses plain HTTPS — which
    /// gives confidentiality vs. the network but NOT vs. a malicious
    /// Tinfoil operator. Tracked separately; structured for swap-in.
    #[serde(rename = "piiBackend", default = "default_pii_backend")]
    pub pii_backend: String,

    /// Which PII classes the AI redaction workers actually rewrite
    /// when `asyncPiiRedaction` is on.
    /// Canonical snake_case `SpanLabel` names (e.g.
    /// `["secret", "email", "person"]`). The models detect every
    /// class but only these are removed — the rest is *value* (a
    /// searchable timeline). Defaults to `["secret"]`, the safety
    /// baseline; `secret` is always treated as included regardless of
    /// what's stored (see dystil-redact `parse_allow_list`). The
    /// Settings UI surfaces a curated subset (Names, Emails, Phones,
    /// Addresses, Sensitive) as opt-in checkboxes.
    #[serde(
        rename = "piiRedactionLabels",
        default = "default_pii_redaction_labels"
    )]
    pub pii_redaction_labels: Vec<String>,

    // ── Cloud / Auth ───────────────────────────────────────────────────
    /// Dystil cloud user ID. Empty string means not logged in.
    /// Kept as String (not Option) to match existing store.bin schema.
    #[serde(rename = "userId")]
    pub user_id: String,

    // ── System ─────────────────────────────────────────────────────────
    /// Power mode preference: "auto", "performance", "battery_saver".
    /// Previously stored in SettingsStore.extra["powerMode"].
    #[serde(rename = "powerMode", default)]
    pub power_mode: Option<String>,

    /// Use Chinese mirror for Hugging Face model downloads.
    #[serde(rename = "useChineseMirror")]
    pub use_chinese_mirror: bool,

    /// Enable AI workflow event detection (cloud feature, requires subscription).
    /// When enabled, classifies desktop activity and triggers event-based pipes.
    #[serde(rename = "enableWorkflowEvents", default)]
    pub enable_workflow_events: bool,

    /// Detected hardware tier ("high", "mid", "low").
    /// Set once on first launch; `None` for existing installs (treated as High).
    #[serde(
        rename = "deviceTier",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub device_tier: Option<String>,

    /// Enable work-hours schedule (when false, records 24/7 as usual)
    #[serde(rename = "scheduleEnabled", default)]
    pub schedule_enabled: bool,

    /// Per-day schedule rules (only used when schedule_enabled is true)
    #[serde(rename = "scheduleRules", default)]
    pub schedule_rules: Vec<ScheduleRule>,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            // Privacy-safe product default: accessibility text is captured,
            // while screenshots require an explicit user opt-in.
            disable_vision: true,
            disable_timeline: false,
            monitor_ids: vec![],
            use_all_monitors: true,
            video_quality: "balanced".to_string(),
            max_snapshot_width: default_max_snapshot_width(),
            disable_snapshot_compaction: false,
            disable_meeting_detector: false,
            ignored_meeting_apps: vec![],
            capture_on_keystroke: None,
            capture_on_clipboard: None,
            capture_scroll: None,
            prioritize_input_latency: false,
            extraction_thread_priority: default_extraction_thread_priority(),
            pause_extraction_on_input_ms: default_pause_extraction_on_input_ms(),
            ignored_windows: vec![],
            included_windows: vec![],
            ignored_urls: vec![],
            ignore_incognito_windows: true,
            pause_on_drm_content: false,
            disable_clipboard_capture: true,
            disable_keyboard_capture: true,
            use_pii_removal: false,
            async_pii_redaction: false,
            pii_backend: default_pii_backend(),
            pii_redaction_labels: default_pii_redaction_labels(),
            user_id: String::new(),
            power_mode: None,
            use_chinese_mirror: false,
            enable_workflow_events: false,
            device_tier: None,
            schedule_enabled: false,
            schedule_rules: vec![],
        }
    }
}

fn default_max_snapshot_width() -> u32 {
    1920
}

fn default_extraction_thread_priority() -> String {
    "below_normal".to_string()
}

fn default_pause_extraction_on_input_ms() -> u64 {
    150
}

fn default_pii_backend() -> String {
    "local".to_string()
}

fn default_true() -> bool {
    true
}

/// Default redaction allow-list: secrets only. The safety baseline —
/// credentials are the one class where a miss is genuinely dangerous.
fn default_pii_redaction_labels() -> Vec<String> {
    vec!["secret".to_string()]
}

use sysinfo::{System, SystemExt};

/// Device performance tier, determined by hardware detection.
/// Used to select conservative or aggressive default settings on first launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceTier {
    /// High-end: ≥24 GB RAM and ≥8 cores.
    High,
    /// Mid-range: ≥12 GB or (≥8 GB and ≥6 cores)
    Mid,
    /// Low-end: <8 GB or <6 cores
    Low,
}

impl DeviceTier {
    /// Parse from a string (stored in settings as "high", "mid", "low").
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "high" => Some(Self::High),
            "mid" | "medium" => Some(Self::Mid),
            "low" => Some(Self::Low),
            _ => None,
        }
    }

    /// Convert to string for storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Mid => "mid",
            Self::Low => "low",
        }
    }
}

/// Classify tier from RAM (GB) and core count. Pure logic, no I/O.
///
/// 8 GB machines are classified as Low because GPU-accelerated models
/// conservative capture defaults on macOS.
pub fn classify_tier(ram_gb: u64, cores: u64) -> DeviceTier {
    if ram_gb >= 24 && cores >= 8 {
        DeviceTier::High
    } else if ram_gb >= 12 || (ram_gb > 8 && cores >= 6) {
        DeviceTier::Mid
    } else {
        DeviceTier::Low
    }
}

/// Detect the device tier based on available RAM and CPU cores.
///
/// | Tier | Criteria                              |
/// |------|---------------------------------------|
/// | High | ≥24 GB RAM and ≥8 cores               |
/// | Mid  | ≥12 GB or (≥8 GB and ≥6 cores)        |
/// | Low  | everything else                        |
pub fn detect_tier() -> DeviceTier {
    let mut sys = System::new();
    sys.refresh_memory();

    let ram_gb = sys.total_memory() / (1024 * 1024 * 1024);
    let cores = sys.cpus().len() as u64;

    // Re-query CPU count via sysinfo's physical core count if cpus() is empty
    // (can happen before refresh_cpu)
    let cores = if cores == 0 {
        sys.physical_core_count().unwrap_or(1) as u64
    } else {
        cores
    };

    classify_tier(ram_gb, cores)
}

/// Apply device-tier defaults to a `RecordingSettings`.
///
/// Called once on first launch after hardware detection. Adjusts capture
/// aggressiveness based on what the hardware can handle comfortably.
pub fn apply_tier_defaults(settings: &mut RecordingSettings, tier: DeviceTier) {
    match tier {
        DeviceTier::High => {
            settings.video_quality = "balanced".to_string();
            settings.power_mode = Some("auto".to_string());
        }
        DeviceTier::Mid => {
            settings.video_quality = "balanced".to_string();
            settings.power_mode = Some("auto".to_string());
            // Only record the primary monitor to reduce CPU/GPU load
            settings.use_all_monitors = true;
        }
        DeviceTier::Low => {
            settings.video_quality = "low".to_string();
            settings.power_mode = Some("battery_saver".to_string());
            // Only record the primary monitor to reduce CPU/GPU load
            settings.use_all_monitors = false;
            settings.monitor_ids = vec!["default".to_string()];
        }
    }
}
