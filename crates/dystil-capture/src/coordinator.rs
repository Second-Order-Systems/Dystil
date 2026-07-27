use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use tokio::sync::{Mutex, RwLock};
use tracing::warn;

use crate::{
    AccessibilityProvider, CaptureConfig, CaptureContext, CaptureError, CaptureMode,
    CaptureObservation, CaptureStore, CaptureTrigger, StoredCapture, VisualProvider, VisualRequest,
};

/// Platform-neutral coordinator. It owns policy; providers own OS APIs.
pub struct CaptureCoordinator {
    config: RwLock<CaptureConfig>,
    accessibility: Arc<dyn AccessibilityProvider>,
    visual: Option<Arc<dyn VisualProvider>>,
    store: Arc<dyn CaptureStore>,
    // AX implementations call synchronous platform APIs internally. Keep tree
    // walks single-flight even though the immediate AX lane and settled visual
    // scheduler are independent tasks.
    accessibility_gate: Mutex<()>,
    // Per-window cache used to turn rapid duplicate AX observations into a
    // link to the existing frame rather than a new syncable frame row.
    dedup: Mutex<DedupState>,
    // Serializes `dedup decision -> persist -> cache update`; AX acquisition
    // itself stays outside this lock.
    commit_gate: Mutex<()>,
    // Serializing visual acquisition prevents parallel one-shot sessions. A
    // later adapter can let compatible callers join the same in-flight result.
    visual_gate: Mutex<()>,
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
            accessibility_gate: Mutex::new(()),
            dedup: Mutex::new(DedupState::default()),
            commit_gate: Mutex::new(()),
            visual_gate: Mutex::new(()),
        }
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
                )
                .await;
        }

        let mut first_stored = None;
        let mut first_error = None;
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
            match self.persist_or_reuse(observation, false).await {
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
        let _guard = self.visual_gate.lock().await;
        if !self.config.read().await.capture_mode.captures_for_trigger() {
            return Ok(Vec::new());
        }

        let provider = self
            .visual
            .as_ref()
            .ok_or(CaptureError::VisualProviderUnavailable)?;
        provider.capture_all(&request).await
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
    ) -> Result<StoredCapture, CaptureError> {
        match self
            .persist_or_reuse_decision(observation, heartbeat)
            .await?
        {
            PersistDecision::Persisted(stored) | PersistDecision::Reused(stored) => Ok(stored),
        }
    }

    async fn persist_or_reuse_decision(
        &self,
        observation: CaptureObservation,
        heartbeat: bool,
    ) -> Result<PersistDecision, CaptureError> {
        // Image observations are durable by definition; avoid silently
        // throwing away a just-acquired visual checkpoint.
        if observation.visual.is_some() || observation.accessibility.is_none() {
            let stored = self.store.persist(observation.clone()).await?;
            self.remember_persisted_accessibility(&observation, &stored)
                .await;
            return Ok(PersistDecision::Persisted(stored));
        }

        let _commit = self.commit_gate.lock().await;
        let snapshot = observation.accessibility.as_ref().expect("checked above");
        let key = WindowKey::from_context(&observation.context);
        let fingerprint = ContextFingerprint::from_context(&observation.context);
        let now = Instant::now();
        let mut dedup = self.dedup.lock().await;
        if let Some(previous) = dedup.entries.get(&key) {
            let same_context = previous.context == fingerprint;
            let rapid = now.saturating_duration_since(previous.persisted_at) <= DEDUP_RAPID_HORIZON;
            let exact = previous.content_hash == snapshot.content_hash;
            let fuzzy = !previous.truncated
                && !snapshot.truncated
                && simhash_distance(previous.simhash, snapshot.simhash)
                    <= SIMHASH_DISTANCE_THRESHOLD;
            if same_context
                && ((heartbeat && exact)
                    || (rapid && exact)
                    || (rapid && fuzzy && allows_fuzzy_dedup(&observation.trigger)))
            {
                return Ok(PersistDecision::Reused(previous.stored.clone()));
            }
        }

        let stored = self.store.persist(observation.clone()).await?;
        remember_entry(&mut dedup, key, fingerprint, snapshot, stored.clone(), now);
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
