use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dystil_telemetry::{
    CaptureProviderKind, CaptureTriggerKind, NoopRecorder, Outcome, ReasonKind, TelemetryRecorder,
};
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

use crate::{
    AccessibilityProvider, CaptureConfig, CaptureContext, CaptureError, CaptureMode,
    CaptureObservation, CaptureStore, CaptureTrigger, StoredCapture, VisualProvider, VisualRequest,
};

fn telemetry_trigger(trigger: &CaptureTrigger) -> CaptureTriggerKind {
    match trigger {
        CaptureTrigger::AppSwitch => CaptureTriggerKind::AppSwitch,
        CaptureTrigger::WindowFocus => CaptureTriggerKind::WindowFocus,
        CaptureTrigger::Click => CaptureTriggerKind::Click,
        CaptureTrigger::TypingPause => CaptureTriggerKind::TypingPause,
        CaptureTrigger::ScrollStop => CaptureTriggerKind::ScrollStop,
        CaptureTrigger::KeyPress => CaptureTriggerKind::KeyPress,
        CaptureTrigger::Clipboard => CaptureTriggerKind::Clipboard,
        CaptureTrigger::VisualChange => CaptureTriggerKind::VisualChange,
        CaptureTrigger::Idle => CaptureTriggerKind::Idle,
        CaptureTrigger::Manual => CaptureTriggerKind::Manual,
        CaptureTrigger::ActivitySettled => CaptureTriggerKind::ActivitySettled,
    }
}

fn telemetry_reason(error: &CaptureError) -> ReasonKind {
    match error {
        CaptureError::VisualCaptureDisabled => ReasonKind::PolicyDisabled,
        CaptureError::VisualProviderUnavailable => ReasonKind::ProviderUnavailable,
        CaptureError::NoEvidence => ReasonKind::NoEvidence,
        CaptureError::ImageStore(_) | CaptureError::Store(_) => ReasonKind::Storage,
        CaptureError::Accessibility(_) | CaptureError::Visual(_) => ReasonKind::Internal,
    }
}

/// Platform-neutral coordinator. It owns policy; providers own OS APIs.
pub struct CaptureCoordinator {
    config: RwLock<CaptureConfig>,
    accessibility: Arc<dyn AccessibilityProvider>,
    visual: Option<Arc<dyn VisualProvider>>,
    store: Arc<dyn CaptureStore>,
    telemetry: Arc<dyn TelemetryRecorder>,
    // AX implementations call synchronous platform APIs internally. Keep tree
    // walks single-flight even though the immediate AX lane and settled visual
    // scheduler are independent tasks.
    accessibility_gate: Mutex<()>,
    // Per-window cache used to turn rapid duplicate AX observations into a
    // link to the existing frame rather than a new syncable frame row.
    dedup: Mutex<DedupState>,
    visual_dedup: Mutex<VisualDedupState>,
    fingerprint_gate: Arc<tokio::sync::Semaphore>,
    // Serializes `dedup decision -> persist -> cache update`; AX acquisition
    // itself stays outside this lock.
    commit_gate: Mutex<()>,
    // Serializing visual acquisition prevents parallel one-shot sessions. A
    // later adapter can let compatible callers join the same in-flight result.
    visual_gate: Mutex<()>,
    // The local visible/relevant candidate can reuse exact unchanged surface
    // states even after the normal rapid-dedup horizon. Production keeps the
    // existing conservative behavior until an explicit rollout decision.
    reuse_exact_surface_states: bool,
}

impl CaptureCoordinator {
    pub fn new(
        config: CaptureConfig,
        accessibility: Arc<dyn AccessibilityProvider>,
        visual: Option<Arc<dyn VisualProvider>>,
        store: Arc<dyn CaptureStore>,
    ) -> Self {
        Self {
            config: RwLock::new(config),
            accessibility,
            visual,
            store,
            telemetry: Arc::new(NoopRecorder),
            accessibility_gate: Mutex::new(()),
            dedup: Mutex::new(DedupState::default()),
            visual_dedup: Mutex::new(VisualDedupState::default()),
            fingerprint_gate: Arc::new(tokio::sync::Semaphore::new(1)),
            commit_gate: Mutex::new(()),
            visual_gate: Mutex::new(()),
            reuse_exact_surface_states: false,
        }
    }

    pub fn with_exact_surface_reuse(mut self) -> Self {
        self.reuse_exact_surface_states = true;
        self
    }

    /// Attach a bounded, consent-gated metrics recorder. The default recorder
    /// is deliberately a no-op so capture can be used without telemetry.
    pub fn with_telemetry(mut self, telemetry: Arc<dyn TelemetryRecorder>) -> Self {
        self.telemetry = telemetry;
        self
    }

    pub async fn capture_mode(&self) -> CaptureMode {
        self.config.read().await.capture_mode
    }

    /// Change policy without restarting accessibility capture. Transition-time
    /// visual teardown is explicit and awaited.
    pub async fn set_capture_mode(&self, mode: CaptureMode) -> Result<(), CaptureError> {
        // Keep the gate across the policy change and teardown. A capture that
        // was queued under the previous mode re-checks policy after obtaining
        // this same gate and therefore cannot restart a provider we just
        // stopped.
        let _visual_guard = self.visual_gate.lock().await;
        let previous = {
            let mut config = self.config.write().await;
            let previous = config.capture_mode;
            config.capture_mode = mode;
            previous
        };

        if previous == CaptureMode::FullCapture && mode == CaptureMode::TextOnly {
            if let Some(visual) = self.visual.as_ref() {
                visual.stop().await?;
            }
        }
        Ok(())
    }

    /// Release retained platform capture resources without changing policy.
    pub async fn stop_visual(&self) -> Result<(), CaptureError> {
        let _visual_guard = self.visual_gate.lock().await;
        if let Some(visual) = self.visual.as_ref() {
            visual.stop().await?;
        }
        Ok(())
    }

    /// Capture and persist one logical checkpoint. FullCapture acquires one
    /// image for the resolved active monitor, or one per connected monitor
    /// when the platform cannot resolve one safely. TextOnly never requests
    /// screen pixels.
    ///
    /// The returned frame is the first persisted frame and remains the
    /// canonical target for the UI-event linker. Additional monitor frames are
    /// independently durable and syncable.
    pub async fn capture(
        &self,
        trigger: CaptureTrigger,
        context: CaptureContext,
    ) -> Result<StoredCapture, CaptureError> {
        let trigger_kind = telemetry_trigger(&trigger);
        let result = self.capture_inner(trigger, context).await;
        let (outcome, reason) = match &result {
            Ok(_) => (Outcome::Succeeded, ReasonKind::None),
            Err(error) => (Outcome::Failed, telemetry_reason(error)),
        };
        self.telemetry
            .record_capture_trigger(trigger_kind, outcome, reason);
        result
    }

    async fn capture_inner(
        &self,
        trigger: CaptureTrigger,
        context: CaptureContext,
    ) -> Result<StoredCapture, CaptureError> {
        let accessibility = self.capture_accessibility(&trigger).await?;
        let context = accessibility
            .as_ref()
            .map(|snapshot| snapshot.context.with_fallback(&context))
            .unwrap_or(context);
        let visuals = match self
            .capture_visual_for_trigger(VisualRequest {
                trigger: trigger.clone(),
                context: context.clone(),
                demand: None,
            })
            .await
        {
            Ok(visuals) => visuals,
            Err(error) if accessibility.is_some() => {
                warn!(
                    %error,
                    "FullCapture visual acquisition failed; preserving accessibility evidence"
                );
                Vec::new()
            }
            Err(error) => return Err(error),
        };

        if accessibility.is_none() && visuals.is_empty() {
            return Err(CaptureError::NoEvidence);
        }

        if visuals.is_empty() {
            return self
                .persist_or_reuse(
                    CaptureObservation {
                        captured_at: Utc::now(),
                        trigger,
                        context,
                        accessibility,
                        visual: None,
                    },
                    false,
                    None,
                )
                .await;
        }

        let mut first_stored = None;
        let mut first_error = None;
        let fingerprint_deadline = Instant::now() + Duration::from_millis(750);
        for visual in visuals {
            let visual_context = with_visual_context(context.clone(), Some(&visual));
            let monitor_id = visual_context.monitor_id;
            let observation = CaptureObservation {
                captured_at: Utc::now(),
                trigger: trigger.clone(),
                context: visual_context,
                accessibility: accessibility.clone(),
                visual: Some(visual),
            };
            match self
                .persist_or_reuse(observation, false, Some(fingerprint_deadline))
                .await
            {
                Ok(stored) => {
                    if first_stored.is_none() {
                        first_stored = Some(stored);
                    }
                }
                Err(error) => {
                    warn!(
                        ?monitor_id,
                        %error,
                        "FullCapture failed to persist one monitor frame"
                    );
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
        }

        if let Some(stored) = first_stored {
            return Ok(stored);
        }

        if accessibility.is_some() {
            warn!(
                "FullCapture could not persist any monitor image; preserving accessibility evidence"
            );
            return self
                .persist_or_reuse(
                    CaptureObservation {
                        captured_at: Utc::now(),
                        trigger,
                        context,
                        accessibility,
                        visual: None,
                    },
                    false,
                    None,
                )
                .await;
        }

        Err(first_error.unwrap_or_else(|| {
            CaptureError::Store("no monitor frame could be persisted".to_string())
        }))
    }

    /// Capture an accessibility checkpoint only when its content changed
    /// since the last successfully persisted coordinator capture.
    ///
    /// This is intentionally separate from [`Self::capture`]: workflow
    /// triggers remain durable checkpoints even when their AX hash matches,
    /// while periodic heartbeats are safe to suppress.
    pub async fn capture_accessibility_if_changed(
        &self,
        trigger: CaptureTrigger,
        context: CaptureContext,
    ) -> Result<Option<StoredCapture>, CaptureError> {
        let trigger_kind = telemetry_trigger(&trigger);
        let result = self
            .capture_accessibility_if_changed_inner(trigger, context)
            .await;
        let (outcome, reason) = match &result {
            Ok(Some(_)) => (Outcome::Succeeded, ReasonKind::None),
            Ok(None) => (Outcome::Skipped, ReasonKind::Unchanged),
            Err(error) => (Outcome::Failed, telemetry_reason(error)),
        };
        self.telemetry
            .record_capture_trigger(trigger_kind, outcome, reason);
        result
    }

    async fn capture_accessibility_if_changed_inner(
        &self,
        trigger: CaptureTrigger,
        context: CaptureContext,
    ) -> Result<Option<StoredCapture>, CaptureError> {
        let accessibility = self.capture_accessibility(&trigger).await?;
        let Some(accessibility) = accessibility else {
            return Err(CaptureError::NoEvidence);
        };
        let context = accessibility.context.with_fallback(&context);
        match self
            .persist_or_reuse_decision(
                CaptureObservation {
                    captured_at: Utc::now(),
                    trigger,
                    context,
                    accessibility: Some(accessibility),
                    visual: None,
                },
                true,
                None,
            )
            .await?
        {
            PersistDecision::Persisted(stored) => Ok(Some(stored)),
            PersistDecision::Reused(_) => Ok(None),
        }
    }

    async fn capture_visual_for_trigger(
        &self,
        request: VisualRequest,
    ) -> Result<Vec<crate::VisualSnapshot>, CaptureError> {
        let trigger = telemetry_trigger(&request.trigger);
        let _guard = self.visual_gate.lock().await;
        if !self.config.read().await.capture_mode.captures_for_trigger() {
            self.telemetry.record_image_capture(
                trigger,
                CaptureProviderKind::None,
                Outcome::Skipped,
                ReasonKind::PolicyDisabled,
            );
            return Ok(Vec::new());
        }

        let Some(provider) = self.visual.as_ref() else {
            self.telemetry.record_image_capture(
                trigger,
                CaptureProviderKind::Unknown,
                Outcome::Failed,
                ReasonKind::ProviderUnavailable,
            );
            return Err(CaptureError::VisualProviderUnavailable);
        };
        match provider.capture_all(&request).await {
            Ok(visuals) if visuals.is_empty() => {
                self.telemetry.record_image_capture(
                    trigger,
                    CaptureProviderKind::Unknown,
                    Outcome::Skipped,
                    ReasonKind::NoEvidence,
                );
                Ok(visuals)
            }
            Ok(visuals) => {
                for _ in &visuals {
                    self.telemetry.record_image_capture(
                        trigger,
                        CaptureProviderKind::Unknown,
                        Outcome::Succeeded,
                        ReasonKind::None,
                    );
                }
                Ok(visuals)
            }
            Err(error) => {
                self.telemetry.record_image_capture(
                    trigger,
                    CaptureProviderKind::Unknown,
                    Outcome::Failed,
                    telemetry_reason(&error),
                );
                Err(error)
            }
        }
    }

    async fn capture_accessibility(
        &self,
        trigger: &CaptureTrigger,
    ) -> Result<Option<crate::AccessibilitySnapshot>, CaptureError> {
        let _guard = self.accessibility_gate.lock().await;
        self.accessibility.capture(trigger).await
    }

    async fn persist_or_reuse(
        &self,
        observation: CaptureObservation,
        heartbeat: bool,
        fingerprint_deadline: Option<Instant>,
    ) -> Result<StoredCapture, CaptureError> {
        match self
            .persist_or_reuse_decision(observation, heartbeat, fingerprint_deadline)
            .await?
        {
            PersistDecision::Persisted(stored) | PersistDecision::Reused(stored) => Ok(stored),
        }
    }

    async fn persist_or_reuse_decision(
        &self,
        observation: CaptureObservation,
        heartbeat: bool,
        fingerprint_deadline: Option<Instant>,
    ) -> Result<PersistDecision, CaptureError> {
        if observation.visual.is_some() {
            return self
                .persist_or_reuse_visual(observation, fingerprint_deadline)
                .await;
        }
        if observation.accessibility.is_none() {
            let stored = self.store.persist(observation.clone()).await?;
            return Ok(PersistDecision::Persisted(stored));
        }

        let _commit = self.commit_gate.lock().await;
        let snapshot = observation.accessibility.as_ref().expect("checked above");
        let key = WindowKey::from_context(&observation.context);
        let fingerprint = ContextFingerprint::from_context(&observation.context);
        let now = Instant::now();
        #[cfg(feature = "debug-capture")]
        let reuse_rss_before = crate::debug_capture::process_rss_bytes();
        #[cfg(feature = "debug-capture")]
        let reuse_started = Instant::now();
        let mut dedup = self.dedup.lock().await;
        if let Some(previous) = dedup.entries.get(&key) {
            let same_context = previous.context == fingerprint;
            let rapid = now.saturating_duration_since(previous.persisted_at) <= DEDUP_RAPID_HORIZON;
            let exact = previous.content_hash == snapshot.content_hash;
            let fuzzy = !previous.truncated
                && !snapshot.truncated
                && simhash_distance(previous.simhash, snapshot.simhash)
                    <= SIMHASH_DISTANCE_THRESHOLD;
            let exact_typing_duplicate =
                matches!(observation.trigger, CaptureTrigger::TypingPause) && exact;
            let exact_surface_reuse =
                self.reuse_exact_surface_states && exact && !snapshot.truncated;
            let force_new_frame = is_forced_checkpoint(&observation.trigger)
                && !(self.reuse_exact_surface_states
                    && matches!(
                        observation.trigger,
                        CaptureTrigger::AppSwitch | CaptureTrigger::WindowFocus
                    ));
            if !force_new_frame
                && same_context
                && (exact_surface_reuse
                    || (heartbeat && exact)
                    || (rapid && exact)
                    || exact_typing_duplicate
                    || (rapid && fuzzy && allows_fuzzy_dedup(&observation.trigger)))
            {
                #[cfg(feature = "debug-capture")]
                crate::debug_capture::record_capture_phase(
                    "hash_reuse_decision",
                    observation.trigger.as_str(),
                    reuse_started,
                    snapshot.context.application.as_deref(),
                    Some(snapshot.node_count),
                    Some(snapshot.text.len()),
                    Some(snapshot.truncated),
                    Some(match snapshot.truncation_reason {
                        crate::AccessibilityTruncationReason::None => "none",
                        crate::AccessibilityTruncationReason::Timeout => "timeout",
                        crate::AccessibilityTruncationReason::MaxNodes => "max_nodes",
                    }),
                    reuse_rss_before,
                    crate::debug_capture::process_rss_bytes(),
                );
                return Ok(PersistDecision::Reused(previous.stored.clone()));
            }
        }

        let stored = self.store.persist(observation.clone()).await?;
        #[cfg(feature = "debug-capture")]
        crate::debug_capture::record_capture_phase(
            "hash_reuse_decision",
            observation.trigger.as_str(),
            reuse_started,
            snapshot.context.application.as_deref(),
            Some(snapshot.node_count),
            Some(snapshot.text.len()),
            Some(snapshot.truncated),
            Some(match snapshot.truncation_reason {
                crate::AccessibilityTruncationReason::None => "none",
                crate::AccessibilityTruncationReason::Timeout => "timeout",
                crate::AccessibilityTruncationReason::MaxNodes => "max_nodes",
            }),
            reuse_rss_before,
            crate::debug_capture::process_rss_bytes(),
        );
        remember_entry(&mut dedup, key, fingerprint, snapshot, stored.clone(), now);
        Ok(PersistDecision::Persisted(stored))
    }

    async fn persist_or_reuse_visual(
        &self,
        observation: CaptureObservation,
        fingerprint_deadline: Option<Instant>,
    ) -> Result<PersistDecision, CaptureError> {
        let visual = observation.visual.as_ref().expect("checked by caller");
        let visual_key = VisualKey::from_observation(&observation);
        let fingerprint = if let (Some(deadline), Ok(permit)) = (
            fingerprint_deadline,
            Arc::clone(&self.fingerprint_gate).try_acquire_owned(),
        ) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                None
            } else {
                let image = Arc::clone(&visual.image);
                match tokio::time::timeout(
                    remaining,
                    tokio::task::spawn_blocking(move || {
                        let _permit = permit;
                        visual_fingerprint(image.as_ref())
                    }),
                )
                .await
                {
                    Ok(Ok(fingerprint)) => Some(fingerprint),
                    Ok(Err(error)) => {
                        warn!(%error, "visual fingerprint worker failed; preserving screenshot");
                        None
                    }
                    Err(_) => {
                        warn!("visual fingerprint budget exhausted; preserving screenshot");
                        None
                    }
                }
            }
        } else {
            None
        };

        let _commit = self.commit_gate.lock().await;
        let context = ContextFingerprint::from_context(&observation.context);
        if !is_forced_checkpoint(&observation.trigger) {
            if let (Some(current), Some(previous)) = (
                fingerprint.as_ref(),
                self.visual_dedup
                    .lock()
                    .await
                    .entries
                    .get(&visual_key)
                    .cloned(),
            ) {
                let same_context = previous.context == context;
                let visual_delta = compare_visual_fingerprints(&previous.visual, current);
                let ax_changed = match (
                    previous.accessibility.as_ref(),
                    observation.accessibility.as_ref(),
                ) {
                    (Some(previous), Some(current)) => {
                        previous.content_hash != current.content_hash
                    }
                    (None, None) => false,
                    _ => true,
                };
                tracing::debug!(
                    changed_pixel_ratio = visual_delta.changed_pixel_ratio,
                    mean_absolute_delta = visual_delta.mean_absolute_delta,
                    dhash_distance = visual_delta.dhash_distance,
                    near_identical = visual_delta.near_identical,
                    ax_changed,
                    trigger = observation.trigger.as_str(),
                    "evaluated visual checkpoint change"
                );
                if same_context && visual_delta.near_identical && !ax_changed {
                    return Ok(PersistDecision::Reused(previous.stored));
                }
            }
        }

        let stored = self.store.persist(observation.clone()).await?;
        self.remember_persisted_accessibility(&observation, &stored)
            .await;
        if let Some(fingerprint) = fingerprint {
            let mut state = self.visual_dedup.lock().await;
            let can_store = state.entries.contains_key(&visual_key)
                || state.entries.len() < MAX_VISUAL_MONITORS;
            if can_store {
                state.entries.insert(
                    visual_key,
                    VisualDedupEntry {
                        stored: stored.clone(),
                        context,
                        visual: fingerprint,
                        accessibility: observation
                            .accessibility
                            .as_ref()
                            .map(AxFingerprint::from_snapshot),
                    },
                );
            }
        }
        Ok(PersistDecision::Persisted(stored))
    }

    async fn remember_persisted_accessibility(
        &self,
        observation: &CaptureObservation,
        stored: &StoredCapture,
    ) {
        let Some(snapshot) = observation.accessibility.as_ref() else {
            return;
        };
        let mut dedup = self.dedup.lock().await;
        remember_entry(
            &mut dedup,
            WindowKey::from_context(&observation.context),
            ContextFingerprint::from_context(&observation.context),
            snapshot,
            stored.clone(),
            Instant::now(),
        );
    }
}

const DEDUP_RAPID_HORIZON: Duration = Duration::from_secs(10);
const SIMHASH_DISTANCE_THRESHOLD: u32 = 2;
const MAX_DEDUP_WINDOWS: usize = 128;
const MAX_VISUAL_MONITORS: usize = 8;
const VISUAL_MAX_WIDTH: u32 = 960;
const VISUAL_MAX_HEIGHT: u32 = 540;
const VISUAL_PIXEL_DELTA: u8 = 12;
const VISUAL_CHANGED_PIXEL_RATIO: f64 = 0.0005;
const VISUAL_MEAN_ABSOLUTE_DELTA: f64 = 0.10;
const VISUAL_DHASH_DISTANCE: u32 = 0;

#[derive(Default)]
struct DedupState {
    entries: HashMap<WindowKey, DedupEntry>,
}

#[derive(Clone)]
struct DedupEntry {
    stored: StoredCapture,
    persisted_at: Instant,
    content_hash: u64,
    simhash: u64,
    context: ContextFingerprint,
    truncated: bool,
}

#[derive(Default)]
struct VisualDedupState {
    entries: HashMap<VisualKey, VisualDedupEntry>,
}

#[derive(Clone)]
struct VisualDedupEntry {
    stored: StoredCapture,
    context: ContextFingerprint,
    visual: VisualFingerprint,
    accessibility: Option<AxFingerprint>,
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct VisualKey(String);

impl VisualKey {
    fn from_observation(observation: &CaptureObservation) -> Self {
        let visual = observation.visual.as_ref().expect("visual observation");
        Self(match (visual.monitor_id, visual.device_name.as_deref()) {
            (Some(id), Some(name)) => format!("{id}:{name}"),
            (Some(id), None) => id.to_string(),
            (None, Some(name)) => name.to_string(),
            (None, None) => "unknown_monitor".to_string(),
        })
    }
}

#[derive(Clone)]
struct VisualFingerprint {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    dhash: u64,
}

struct VisualDelta {
    changed_pixel_ratio: f64,
    mean_absolute_delta: f64,
    dhash_distance: u32,
    near_identical: bool,
}

#[derive(Clone)]
struct AxFingerprint {
    content_hash: u64,
}

impl AxFingerprint {
    fn from_snapshot(snapshot: &crate::AccessibilitySnapshot) -> Self {
        Self {
            content_hash: snapshot.content_hash,
        }
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct WindowKey(String);

impl WindowKey {
    fn from_context(context: &CaptureContext) -> Self {
        Self(format!(
            "{}\0{}",
            normalize_context_value(context.application.as_deref()),
            normalize_context_value(context.window.as_deref())
        ))
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ContextFingerprint {
    application: String,
    window: String,
    browser_url: String,
    document_path: String,
}

impl ContextFingerprint {
    fn from_context(context: &CaptureContext) -> Self {
        Self {
            application: normalize_context_value(context.application.as_deref()),
            window: normalize_context_value(context.window.as_deref()),
            browser_url: normalize_context_value(context.browser_url.as_deref()),
            document_path: normalize_context_value(context.document_path.as_deref()),
        }
    }
}

fn normalize_context_value(value: Option<&str>) -> String {
    value.unwrap_or_default().trim().to_lowercase()
}

fn allows_fuzzy_dedup(trigger: &CaptureTrigger) -> bool {
    matches!(
        trigger,
        CaptureTrigger::Click
            | CaptureTrigger::ScrollStop
            | CaptureTrigger::VisualChange
            | CaptureTrigger::Idle
    )
}

fn is_forced_checkpoint(trigger: &CaptureTrigger) -> bool {
    matches!(
        trigger,
        CaptureTrigger::AppSwitch | CaptureTrigger::WindowFocus | CaptureTrigger::Manual
    )
}

fn visual_fingerprint(image: &image::DynamicImage) -> VisualFingerprint {
    use image::imageops::FilterType;
    let grayscale = image
        .resize(VISUAL_MAX_WIDTH, VISUAL_MAX_HEIGHT, FilterType::Triangle)
        .to_luma8();
    let hash_image = image::DynamicImage::ImageLuma8(grayscale.clone())
        .resize_exact(9, 8, FilterType::Triangle)
        .to_luma8();
    let mut dhash = 0_u64;
    for y in 0..8 {
        for x in 0..8 {
            dhash <<= 1;
            if hash_image.get_pixel(x, y)[0] > hash_image.get_pixel(x + 1, y)[0] {
                dhash |= 1;
            }
        }
    }
    VisualFingerprint {
        width: grayscale.width(),
        height: grayscale.height(),
        pixels: grayscale.into_raw(),
        dhash,
    }
}

fn compare_visual_fingerprints(
    previous: &VisualFingerprint,
    current: &VisualFingerprint,
) -> VisualDelta {
    if previous.width != current.width || previous.height != current.height {
        return VisualDelta {
            changed_pixel_ratio: 1.0,
            mean_absolute_delta: 255.0,
            dhash_distance: 64,
            near_identical: false,
        };
    }
    let mut changed = 0usize;
    let mut absolute_delta = 0u64;
    for (left, right) in previous.pixels.iter().zip(&current.pixels) {
        let delta = left.abs_diff(*right);
        absolute_delta += u64::from(delta);
        if delta > VISUAL_PIXEL_DELTA {
            changed += 1;
        }
    }
    let pixel_count = previous.pixels.len().max(1);
    let changed_pixel_ratio = changed as f64 / pixel_count as f64;
    let mean_absolute_delta = absolute_delta as f64 / pixel_count as f64;
    let dhash_distance = (previous.dhash ^ current.dhash).count_ones();
    VisualDelta {
        changed_pixel_ratio,
        mean_absolute_delta,
        dhash_distance,
        near_identical: changed_pixel_ratio <= VISUAL_CHANGED_PIXEL_RATIO
            && mean_absolute_delta <= VISUAL_MEAN_ABSOLUTE_DELTA
            && dhash_distance <= VISUAL_DHASH_DISTANCE,
    }
}

fn simhash_distance(left: u64, right: u64) -> u32 {
    (left ^ right).count_ones()
}

enum PersistDecision {
    Persisted(StoredCapture),
    Reused(StoredCapture),
}

fn remember_entry(
    state: &mut DedupState,
    key: WindowKey,
    context: ContextFingerprint,
    snapshot: &crate::AccessibilitySnapshot,
    stored: StoredCapture,
    now: Instant,
) {
    if !state.entries.contains_key(&key) && state.entries.len() >= MAX_DEDUP_WINDOWS {
        if let Some(evicted) = state.entries.keys().next().cloned() {
            state.entries.remove(&evicted);
        }
    }
    state.entries.insert(
        key,
        DedupEntry {
            stored,
            persisted_at: now,
            content_hash: snapshot.content_hash,
            simhash: snapshot.simhash,
            context,
            truncated: snapshot.truncated,
        },
    );
}

fn with_visual_context(
    mut context: CaptureContext,
    visual: Option<&crate::VisualSnapshot>,
) -> CaptureContext {
    if let Some(visual) = visual {
        context.monitor_id = visual.monitor_id.or(context.monitor_id);
        context.device_name = visual.device_name.clone().or(context.device_name);
    }
    context
}

// These tests cover the retired three-mode/on-demand API and are retained in
// history only while the capture-core test suite is rewritten around the two
// product modes below.
#[cfg(any())]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use chrono::Utc;

    use super::*;
    use crate::{AccessibilitySnapshot, AccessibilityTruncationReason, VisualSnapshot};

    struct AccessibilityStub;

    #[async_trait]
    impl AccessibilityProvider for AccessibilityStub {
        async fn capture(
            &self,
            _trigger: &CaptureTrigger,
        ) -> Result<Option<AccessibilitySnapshot>, CaptureError> {
            Ok(Some(AccessibilitySnapshot {
                captured_at: Utc::now(),
                context: CaptureContext::default(),
                text: "focused content".to_string(),
                nodes: vec![],
                node_count: 0,
                walk_duration_ms: 1,
                content_hash: 1,
                simhash: 2,
                truncated: false,
                truncation_reason: AccessibilityTruncationReason::None,
                max_depth_reached: 0,
            }))
        }
    }

    struct CountingVisual {
        captures: AtomicUsize,
        stops: AtomicUsize,
    }

    #[async_trait]
    impl VisualProvider for CountingVisual {
        async fn capture_all(
            &self,
            _request: &VisualRequest,
        ) -> Result<Vec<VisualSnapshot>, CaptureError> {
            self.captures.fetch_add(1, Ordering::SeqCst);
            Ok(vec![VisualSnapshot {
                captured_at: Utc::now(),
                image: Arc::new(image::DynamicImage::new_rgba8(1, 1)),
                monitor_id: None,
                device_name: None,
            }])
        }

        async fn stop(&self) -> Result<(), CaptureError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct RecordingStore {
        visual_observations: AtomicUsize,
    }

    #[async_trait]
    impl CaptureStore for RecordingStore {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            if observation.visual.is_some() {
                self.visual_observations.fetch_add(1, Ordering::SeqCst);
            }
            Ok(StoredCapture {
                frame_id: 1,
                snapshot_path: observation.visual.map(|_| "frame.jpg".to_string()),
            })
        }
    }

    struct CountingStore {
        observations: AtomicUsize,
    }

    #[async_trait]
    impl CaptureStore for CountingStore {
        async fn persist(
            &self,
            _observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            let frame_id = self.observations.fetch_add(1, Ordering::SeqCst) as i64 + 1;
            Ok(StoredCapture {
                frame_id,
                snapshot_path: None,
            })
        }
    }

    struct FailingVisual {
        stops: AtomicUsize,
    }

    #[async_trait]
    impl VisualProvider for FailingVisual {
        async fn capture_all(
            &self,
            _request: &VisualRequest,
        ) -> Result<Vec<VisualSnapshot>, CaptureError> {
            Err(CaptureError::Visual("synthetic failure".to_string()))
        }

        async fn stop(&self) -> Result<(), CaptureError> {
            self.stops.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct ImageFailingStore {
        attempts: AtomicUsize,
    }

    #[async_trait]
    impl CaptureStore for ImageFailingStore {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            self.attempts.fetch_add(1, Ordering::SeqCst);
            if observation.visual.is_some() {
                return Err(CaptureError::ImageStore("disk full".to_string()));
            }
            Ok(StoredCapture {
                frame_id: 9,
                snapshot_path: None,
            })
        }
    }

    fn coordinator(
        mode: VisualCaptureMode,
        visual: Arc<CountingVisual>,
        store: Arc<RecordingStore>,
    ) -> CaptureCoordinator {
        CaptureCoordinator::new(
            CaptureConfig { visual_mode: mode },
            Arc::new(AccessibilityStub),
            Some(visual),
            store,
        )
    }

    #[tokio::test]
    async fn disabled_mode_persists_ax_frame_without_touching_visual_provider() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = coordinator(VisualCaptureMode::Disabled, visual.clone(), store.clone());

        let stored = coordinator
            .capture(CaptureTrigger::TypingPause, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(stored.frame_id, 1);
        assert_eq!(stored.snapshot_path, None);
        assert_eq!(visual.captures.load(Ordering::SeqCst), 0);
        assert_eq!(store.visual_observations.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unchanged_accessibility_heartbeat_is_not_persisted_twice() {
        let store = Arc::new(CountingStore {
            observations: AtomicUsize::new(0),
        });
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::OnDemand,
            },
            Arc::new(AccessibilityStub),
            None,
            store.clone(),
        );

        let first = coordinator
            .capture_accessibility_if_changed(CaptureTrigger::Idle, CaptureContext::default())
            .await
            .unwrap();
        let unchanged = coordinator
            .capture_accessibility_if_changed(CaptureTrigger::Idle, CaptureContext::default())
            .await
            .unwrap();

        assert!(first.is_some());
        assert!(unchanged.is_none());
        assert_eq!(store.observations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rapid_exact_clicks_reuse_the_previous_frame() {
        let store = Arc::new(CountingStore {
            observations: AtomicUsize::new(0),
        });
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::Disabled,
            },
            Arc::new(AccessibilityStub),
            None,
            store.clone(),
        );

        let first = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();
        let second = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(first.frame_id, second.frame_id);
        assert_eq!(store.observations.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn production_simhash_samples_are_near_duplicates() {
        let first = -2_961_068_771_284_286_279_i64 as u64;
        let middle = -2_961_068_762_694_351_687_i64 as u64;
        assert_eq!(simhash_distance(first, middle), 1);
        assert_eq!(simhash_distance(first, first), 0);
    }

    #[tokio::test]
    async fn on_demand_skips_pixels_for_ordinary_triggers() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = coordinator(VisualCaptureMode::OnDemand, visual.clone(), store);

        coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(visual.captures.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn explicit_on_demand_capture_acquires_then_stops_visual_provider() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = coordinator(VisualCaptureMode::OnDemand, visual.clone(), store.clone());

        coordinator
            .request_visual_capture(
                CaptureTrigger::Manual,
                CaptureContext::default(),
                VisualDemand::UserRequested,
            )
            .await
            .unwrap();

        assert_eq!(visual.captures.load(Ordering::SeqCst), 1);
        assert_eq!(visual.stops.load(Ordering::SeqCst), 1);
        assert_eq!(store.visual_observations.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_on_demand_capture_still_stops_visual_provider() {
        let visual = Arc::new(FailingVisual {
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::OnDemand,
            },
            Arc::new(AccessibilityStub),
            Some(visual.clone()),
            store,
        );

        let result = coordinator
            .request_visual_capture(
                CaptureTrigger::Manual,
                CaptureContext::default(),
                VisualDemand::UserRequested,
            )
            .await;

        let result = result.expect("AX evidence should preserve the logical frame");
        assert!(matches!(result.status, VisualCaptureStatus::Failed { .. }));
        assert_eq!(result.stored.snapshot_path, None);
        assert_eq!(visual.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn image_write_failure_falls_back_to_an_ax_only_frame() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(ImageFailingStore {
            attempts: AtomicUsize::new(0),
        });
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::OnDemand,
            },
            Arc::new(AccessibilityStub),
            Some(visual.clone()),
            store.clone(),
        );

        let result = coordinator
            .request_visual_capture(
                CaptureTrigger::ActivitySettled,
                CaptureContext::default(),
                VisualDemand::ActivitySettled,
            )
            .await
            .expect("AX fallback should be persisted");

        assert_eq!(result.stored.frame_id, 9);
        assert_eq!(result.stored.snapshot_path, None);
        assert!(matches!(result.status, VisualCaptureStatus::Failed { .. }));
        assert_eq!(store.attempts.load(Ordering::SeqCst), 2);
        assert_eq!(visual.captures.load(Ordering::SeqCst), 1);
        assert_eq!(visual.stops.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn disabled_mode_rejects_explicit_visual_demand() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = coordinator(VisualCaptureMode::Disabled, visual.clone(), store);

        let result = coordinator
            .request_visual_capture(
                CaptureTrigger::Manual,
                CaptureContext::default(),
                VisualDemand::UserRequested,
            )
            .await;

        assert!(matches!(result, Err(CaptureError::VisualCaptureDisabled)));
        assert_eq!(visual.captures.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn switching_to_disabled_stops_visual_provider() {
        let visual = Arc::new(CountingVisual {
            captures: AtomicUsize::new(0),
            stops: AtomicUsize::new(0),
        });
        let store = Arc::new(RecordingStore {
            visual_observations: AtomicUsize::new(0),
        });
        let coordinator = coordinator(VisualCaptureMode::Continuous, visual.clone(), store);

        coordinator
            .set_visual_mode(VisualCaptureMode::Disabled)
            .await
            .unwrap();

        assert_eq!(visual.stops.load(Ordering::SeqCst), 1);
        assert_eq!(coordinator.visual_mode().await, VisualCaptureMode::Disabled);
    }
}

#[cfg(test)]
mod full_capture_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;

    use async_trait::async_trait;
    use chrono::Utc;
    use dystil_telemetry::{ConsentDecision, SignalKind, Telemetry, TELEMETRY_CONSENT_VERSION};

    use super::*;
    use crate::{AccessibilitySnapshot, AccessibilityTruncationReason, VisualSnapshot};

    struct Ax;
    #[async_trait]
    impl AccessibilityProvider for Ax {
        async fn capture(
            &self,
            _: &CaptureTrigger,
        ) -> Result<Option<AccessibilitySnapshot>, CaptureError> {
            Ok(Some(AccessibilitySnapshot {
                captured_at: Utc::now(),
                context: CaptureContext::default(),
                text: "AX".into(),
                nodes: vec![],
                node_count: 0,
                walk_duration_ms: 0,
                content_hash: 1,
                simhash: 1,
                truncated: false,
                truncation_reason: AccessibilityTruncationReason::None,
                max_depth_reached: 0,
            }))
        }
    }
    struct Visual(AtomicUsize);
    #[async_trait]
    impl VisualProvider for Visual {
        async fn capture_all(
            &self,
            _: &VisualRequest,
        ) -> Result<Vec<VisualSnapshot>, CaptureError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(vec![VisualSnapshot {
                captured_at: Utc::now(),
                image: Arc::new(image::DynamicImage::new_rgba8(1, 1)),
                monitor_id: None,
                device_name: None,
            }])
        }
        async fn stop(&self) -> Result<(), CaptureError> {
            Ok(())
        }
    }
    struct Store;
    #[async_trait]
    impl CaptureStore for Store {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            Ok(StoredCapture {
                frame_id: 1,
                snapshot_path: observation.visual.map(|_| "frame.jpg".into()),
            })
        }
    }

    #[tokio::test]
    async fn telemetry_records_bounded_capture_and_image_outcomes() {
        let telemetry = Arc::new(Telemetry::new());
        telemetry.set_consent(ConsentDecision::Granted {
            policy_version: TELEMETRY_CONSENT_VERSION,
        });
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(Arc::new(Visual(AtomicUsize::new(0)))),
            Arc::new(Store),
        )
        .with_telemetry(telemetry.clone());

        coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();

        let points = telemetry.drain_interval().unwrap().points;
        assert!(points.iter().any(|point| {
            point.signal == SignalKind::CaptureTrigger
                && point.trigger == CaptureTriggerKind::Click
                && point.outcome == Outcome::Succeeded
                && point.reason == ReasonKind::None
                && point.value == 1
        }));
        assert!(points.iter().any(|point| {
            point.signal == SignalKind::ImageCapture
                && point.trigger == CaptureTriggerKind::Click
                && point.outcome == Outcome::Succeeded
                && point.reason == ReasonKind::None
                && point.value == 1
        }));
    }

    #[derive(Default)]
    struct CountingStore(AtomicUsize);

    #[async_trait]
    impl CaptureStore for CountingStore {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            let frame_id = self.0.fetch_add(1, Ordering::SeqCst) as i64 + 1;
            Ok(StoredCapture {
                frame_id,
                snapshot_path: observation.visual.map(|_| format!("frame-{frame_id}.jpg")),
            })
        }
    }

    struct MultiMonitorVisual;
    #[async_trait]
    impl VisualProvider for MultiMonitorVisual {
        async fn capture_all(
            &self,
            _: &VisualRequest,
        ) -> Result<Vec<VisualSnapshot>, CaptureError> {
            Ok(vec![
                VisualSnapshot {
                    captured_at: Utc::now(),
                    image: Arc::new(image::DynamicImage::new_rgba8(1, 1)),
                    monitor_id: Some(1082),
                    device_name: Some("eDP-2".into()),
                },
                VisualSnapshot {
                    captured_at: Utc::now(),
                    image: Arc::new(image::DynamicImage::new_rgba8(1, 1)),
                    monitor_id: Some(1085),
                    device_name: Some("HDMI-A-1".into()),
                },
            ])
        }

        async fn stop(&self) -> Result<(), CaptureError> {
            Ok(())
        }
    }

    struct FailingVisual;
    #[async_trait]
    impl VisualProvider for FailingVisual {
        async fn capture_all(
            &self,
            _: &VisualRequest,
        ) -> Result<Vec<VisualSnapshot>, CaptureError> {
            Err(CaptureError::Visual("synthetic display failure".into()))
        }

        async fn stop(&self) -> Result<(), CaptureError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct MonitorRecordingStore {
        monitor_ids: StdMutex<Vec<Option<u32>>>,
    }

    #[async_trait]
    impl CaptureStore for MonitorRecordingStore {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            let monitor_id = observation.context.monitor_id;
            let mut monitor_ids = self.monitor_ids.lock().unwrap();
            monitor_ids.push(monitor_id);
            Ok(StoredCapture {
                frame_id: monitor_ids.len() as i64,
                snapshot_path: Some(format!("monitor-{}.jpg", monitor_id.unwrap_or(0))),
            })
        }
    }

    #[tokio::test]
    async fn full_capture_persists_ax_and_pixels_in_one_frame() {
        let visual = Arc::new(Visual(AtomicUsize::new(0)));
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(visual.clone()),
            Arc::new(Store),
        );
        let stored = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();
        assert_eq!(stored.snapshot_path.as_deref(), Some("frame.jpg"));
        assert_eq!(visual.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn repeated_near_identical_typing_capture_reuses_the_monitor_frame() {
        let visual = Arc::new(Visual(AtomicUsize::new(0)));
        let store = Arc::new(CountingStore::default());
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(visual),
            store.clone(),
        );

        let first = coordinator
            .capture(CaptureTrigger::TypingPause, CaptureContext::default())
            .await
            .unwrap();
        let second = coordinator
            .capture(CaptureTrigger::TypingPause, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(first.frame_id, second.frame_id);
        assert_eq!(store.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn app_switch_remains_forced_when_pixels_and_ax_are_identical() {
        let visual = Arc::new(Visual(AtomicUsize::new(0)));
        let store = Arc::new(CountingStore::default());
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(visual),
            store.clone(),
        );

        coordinator
            .capture(CaptureTrigger::AppSwitch, CaptureContext::default())
            .await
            .unwrap();
        coordinator
            .capture(CaptureTrigger::AppSwitch, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(store.0.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn full_capture_persists_one_frame_for_each_returned_monitor() {
        let store = Arc::new(MonitorRecordingStore::default());
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(Arc::new(MultiMonitorVisual)),
            store.clone(),
        );

        let canonical = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(canonical.frame_id, 1);
        assert_eq!(
            *store.monitor_ids.lock().unwrap(),
            vec![Some(1082), Some(1085)]
        );
    }

    #[tokio::test]
    async fn full_capture_preserves_ax_when_every_visual_capture_fails() {
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::FullCapture,
            },
            Arc::new(Ax),
            Some(Arc::new(FailingVisual)),
            Arc::new(Store),
        );

        let stored = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();

        assert_eq!(stored.snapshot_path, None);
    }

    #[tokio::test]
    async fn text_only_never_calls_the_visual_provider() {
        let visual = Arc::new(Visual(AtomicUsize::new(0)));
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                capture_mode: CaptureMode::TextOnly,
            },
            Arc::new(Ax),
            Some(visual.clone()),
            Arc::new(Store),
        );
        let stored = coordinator
            .capture(CaptureTrigger::Click, CaptureContext::default())
            .await
            .unwrap();
        assert_eq!(stored.snapshot_path, None);
        assert_eq!(visual.0.load(Ordering::SeqCst), 0);
    }
}

#[cfg(test)]
mod change_detection_tests {
    use super::*;

    #[test]
    fn visual_fingerprint_is_bounded_and_detects_large_changes() {
        let dark = image::DynamicImage::new_luma8(2_560, 1_440);
        let bright = image::DynamicImage::ImageLuma8(image::GrayImage::from_pixel(
            2_560,
            1_440,
            image::Luma([255]),
        ));
        let dark = visual_fingerprint(&dark);
        let bright = visual_fingerprint(&bright);

        assert_eq!((dark.width, dark.height), (960, 540));
        assert!(dark.pixels.len() <= (VISUAL_MAX_WIDTH * VISUAL_MAX_HEIGHT) as usize);
        assert!(!compare_visual_fingerprints(&dark, &bright).near_identical);
        assert!(compare_visual_fingerprints(&dark, &dark).near_identical);
    }

    #[test]
    fn visual_dedupe_only_accepts_tiny_capture_noise() {
        let previous = VisualFingerprint {
            width: 100,
            height: 100,
            pixels: vec![0; 10_000],
            dhash: 0,
        };
        let mut within_cutoff = previous.clone();
        within_cutoff.pixels[..5].fill(13);
        assert!(compare_visual_fingerprints(&previous, &within_cutoff).near_identical);

        let mut beyond_cutoff = previous.clone();
        beyond_cutoff.pixels[..6].fill(13);
        assert!(!compare_visual_fingerprints(&previous, &beyond_cutoff).near_identical);

        let mut hash_changed = previous.clone();
        hash_changed.dhash = 1;
        assert!(!compare_visual_fingerprints(&previous, &hash_changed).near_identical);
    }
}
