use std::sync::Arc;
use std::time::Duration;

use sqlx::SqlitePool;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::settled_state::{ActivityKind, AppCadenceGuard, SettledStatePolicy};
use crate::{CaptureContext, CaptureCoordinator, CaptureTrigger, CaptureTriggerMessage};

use crate::linker::{DystilLinkDropReason, DystilLinkerSender};

#[derive(Debug, Clone)]
pub struct DystilAxCaptureConfig {
    pub debounce: Duration,
    pub ordinary_min_interval: Duration,
    pub heartbeat: Duration,
    pub settled_state: bool,
    /// Candidate-only scheduling guard. It can defer/coalesce a settled
    /// demand after an expensive walk, but never changes a real walk's UIA
    /// limits or drops a checkpoint.
    pub app_cadence_guard: bool,
    pub activity_span_pool: Option<SqlitePool>,
    pub activity_span_session_id: Option<String>,
}

impl Default for DystilAxCaptureConfig {
    fn default() -> Self {
        Self {
            debounce: Duration::from_millis(250),
            ordinary_min_interval: Duration::from_millis(1_500),
            heartbeat: Duration::from_secs(60),
            settled_state: false,
            app_cadence_guard: false,
            activity_span_pool: None,
            activity_span_session_id: None,
        }
    }
}

/// Handle for the single global AX-only trigger consumer.
pub struct DystilAxCaptureHandle {
    shutdown_tx: watch::Sender<bool>,
    join: Option<JoinHandle<()>>,
}

impl DystilAxCaptureHandle {
    pub fn start(
        trigger_rx: tokio::sync::broadcast::Receiver<CaptureTriggerMessage>,
        linker_tx: DystilLinkerSender,
        coordinator: Arc<CaptureCoordinator>,
        config: DystilAxCaptureConfig,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let join = tokio::spawn(run_ax_capture(
            trigger_rx,
            linker_tx,
            coordinator,
            config,
            shutdown_rx,
        ));
        Self {
            shutdown_tx,
            join: Some(join),
        }
    }

    pub async fn shutdown(mut self) {
        let _ = self.shutdown_tx.send(true);
        if let Some(mut join) = self.join.take() {
            if tokio::time::timeout(Duration::from_secs(5), &mut join)
                .await
                .is_err()
            {
                join.abort();
                let _ = join.await;
            }
        }
    }
}

impl Drop for DystilAxCaptureHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(true);
    }
}

async fn run_ax_capture(
    mut trigger_rx: tokio::sync::broadcast::Receiver<CaptureTriggerMessage>,
    linker_tx: DystilLinkerSender,
    coordinator: Arc<CaptureCoordinator>,
    config: DystilAxCaptureConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut heartbeat = tokio::time::interval(config.heartbeat);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    heartbeat.tick().await;

    let mut activity_since_heartbeat = false;
    let mut last_ordinary_capture = None;
    let mut settled_policy = config.settled_state.then(SettledStatePolicy::default);
    let mut app_cadence_guard = config.app_cadence_guard.then(AppCadenceGuard::default);
    if let Some(pool) = config.activity_span_pool.as_ref() {
        if let Err(error) = crate::activity_spans::ensure_activity_span_schema(pool).await {
            warn!(%error, "could not create candidate activity span schema");
        }
    }
    let mut settled_tick = tokio::time::interval(Duration::from_millis(50));
    settled_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    settled_tick.tick().await;
    loop {
        tokio::select! {
            changed = shutdown_rx.changed() => {
                if changed.is_err() || *shutdown_rx.borrow() {
                    break;
                }
            }
            message = trigger_rx.recv() => {
                match message {
                    Ok(message) => {
                        activity_since_heartbeat = true;
                        if let Some(policy) = settled_policy.as_mut() {
                            if let Some(kind) = settled_activity_kind(&message.trigger) {
                                let mut correlation_ids = message.correlation_id.into_iter().collect::<Vec<_>>();
                                correlation_ids.extend(message.additional_correlation_ids);
                                policy.observe(kind, message.context, correlation_ids, message.activity_delta_x, message.activity_delta_y, std::time::Instant::now());
                                continue;
                            }
                        }
                        if !capture_trigger_batch(
                            message,
                            &mut trigger_rx,
                            &linker_tx,
                            &coordinator,
                            config.debounce,
                            config.ordinary_min_interval,
                            &mut last_ordinary_capture,
                            &mut shutdown_rx,
                        ).await {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        warn!("AX capture trigger receiver lagged; some triggers were coalesced");
                        linker_tx.trigger_dropped(vec![], DystilLinkDropReason::Lagged);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = settled_tick.tick(), if settled_policy.is_some() => {
                let now = std::time::Instant::now();
                let due = settled_policy.as_mut().expect("checked").take_due(now);
                for demand in due {
                    if let Some(deadline) = app_cadence_guard
                        .as_ref()
                        .and_then(|guard| guard.defer_until(&demand, now))
                    {
                        settled_policy
                            .as_mut()
                            .expect("checked")
                            .defer(demand, deadline, now);
                        continue;
                    }
                    let correlation_ids = demand.correlation_ids;
                    let span_id = match (&config.activity_span_pool, &config.activity_span_session_id) {
                        (Some(pool), Some(session_id)) => crate::activity_spans::insert_activity_span(
                            pool, session_id, activity_kind_name(demand.kind), demand.duration_ms,
                            demand.event_count, &demand.app_sequence, &demand.context,
                            demand.scroll_delta_x, demand.scroll_delta_y,
                        ).await.ok(),
                        _ => None,
                    };
                    let first = CaptureTriggerMessage {
                        trigger: demand.trigger,
                        context: demand.context,
                        correlation_id: correlation_ids.first().copied(),
                        additional_correlation_ids: correlation_ids.iter().skip(1).copied().collect(),
                        activity_span_id: span_id,
                        activity_delta_x: demand.scroll_delta_x,
                        activity_delta_y: demand.scroll_delta_y,
                    };
                    if let Some(reason) = capture_pause_reason() {
                        linker_tx.trigger_dropped(correlation_ids, reason);
                        continue;
                    }
                    #[cfg(feature = "debug-capture")]
                    let diagnostic_started = std::time::Instant::now();
                    #[cfg(feature = "debug-capture")]
                    let diagnostic_id = crate::debug_capture::record_capture_request(
                        &first.trigger, &first.context, correlation_ids.len(), false,
                    );
                    let capture_started = std::time::Instant::now();
                    let capture_context = first.context.clone();
                    let result = coordinator.capture(first.trigger, first.context).await;
                    if let Some(guard) = app_cadence_guard.as_mut() {
                        guard.record_capture(&capture_context, capture_started.elapsed(), std::time::Instant::now());
                    }
                    #[cfg(feature = "debug-capture")]
                    match &result {
                        Ok(stored) => crate::debug_capture::record_capture_result(
                            diagnostic_id, diagnostic_started, Some(stored.frame_id), None, None),
                        Err(error) => crate::debug_capture::record_capture_result(
                            diagnostic_id, diagnostic_started, None, Some("error"), Some(&error.to_string())),
                    }
                    match result {
                        Ok(stored) => {
                            if !correlation_ids.is_empty() { linker_tx.frame_captured(stored.frame_id, correlation_ids); }
                            if let (Some(pool), Some(span_id)) = (&config.activity_span_pool, first.activity_span_id) {
                                if let Err(error) = crate::activity_spans::link_activity_span_frame(pool, span_id, stored.frame_id).await {
                                    warn!(%error, span_id, "could not link activity span to settled frame");
                                }
                            }
                        }
                        Err(error) => warn!(%error, "settled activity capture failed"),
                    }
                }
            }
            _ = heartbeat.tick() => {
                if config.settled_state {
                    continue;
                }
                if !activity_since_heartbeat || capture_pause_reason().is_some() {
                    continue;
                }
                activity_since_heartbeat = false;
                #[cfg(feature = "debug-capture")]
                let diagnostic_started = std::time::Instant::now();
                #[cfg(feature = "debug-capture")]
                let diagnostic_id = crate::debug_capture::record_capture_request(
                    &CaptureTrigger::Idle,
                    &CaptureContext::default(),
                    0,
                    true,
                );
                let result = coordinator
                    .capture_accessibility_if_changed(
                        CaptureTrigger::Idle,
                        CaptureContext::default(),
                    )
                    .await;
                #[cfg(feature = "debug-capture")]
                match &result {
                    Ok(Some(stored)) => crate::debug_capture::record_capture_result(
                        diagnostic_id, diagnostic_started, Some(stored.frame_id), Some("persisted"), None),
                    Ok(None) => crate::debug_capture::record_capture_result(
                        diagnostic_id, diagnostic_started, None, Some("unchanged"), None),
                    Err(error) => crate::debug_capture::record_capture_result(
                        diagnostic_id, diagnostic_started, None, Some("error"), Some(&error.to_string())),
                }
                match result {
                    Ok(Some(stored)) => debug!(
                        frame_id = stored.frame_id,
                        "AX heartbeat persisted changed accessibility content"
                    ),
                    Ok(None) => debug!("AX heartbeat skipped unchanged accessibility content"),
                    Err(error) => debug!(%error, "AX heartbeat produced no frame"),
                }
            }
        }
    }

    if let Err(error) = coordinator.stop_visual().await {
        warn!(%error, "failed to release FullCapture visual resources during shutdown");
    }
}

fn activity_kind_name(kind: ActivityKind) -> &'static str {
    match kind {
        ActivityKind::Click => "click_burst",
        ActivityKind::AppSwitch => "app_switch_burst",
        ActivityKind::Scroll => "scroll_burst",
        ActivityKind::Typing => "typing_burst",
    }
}

fn settled_activity_kind(trigger: &CaptureTrigger) -> Option<ActivityKind> {
    match trigger {
        CaptureTrigger::Click => Some(ActivityKind::Click),
        CaptureTrigger::AppSwitch | CaptureTrigger::WindowFocus => Some(ActivityKind::AppSwitch),
        CaptureTrigger::ScrollStop => Some(ActivityKind::Scroll),
        CaptureTrigger::TypingPause | CaptureTrigger::KeyPress => Some(ActivityKind::Typing),
        _ => None,
    }
}

async fn capture_trigger_batch(
    first: CaptureTriggerMessage,
    trigger_rx: &mut tokio::sync::broadcast::Receiver<CaptureTriggerMessage>,
    linker_tx: &DystilLinkerSender,
    coordinator: &CaptureCoordinator,
    debounce: Duration,
    ordinary_min_interval: Duration,
    last_ordinary_capture: &mut Option<std::time::Instant>,
    shutdown_rx: &mut watch::Receiver<bool>,
) -> bool {
    let sleep = tokio::time::sleep(debounce);
    tokio::pin!(sleep);
    tokio::select! {
        _ = &mut sleep => {}
        changed = shutdown_rx.changed() => {
            return !(changed.is_err() || *shutdown_rx.borrow());
        }
    }

    let mut messages = vec![first];
    loop {
        match trigger_rx.try_recv() {
            Ok(message) => messages.push(message),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
            Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {
                linker_tx.trigger_dropped(vec![], DystilLinkDropReason::Lagged);
            }
        }
    }

    let mut correlation_ids = Vec::new();
    let mut selected = None;
    for message in messages {
        if let Some(correlation_id) = message.correlation_id {
            correlation_ids.push(correlation_id);
        }
        // Raw key events can arrive several times per second. The recorder
        // emits a separate TypingPause after a text burst, which is the
        // durable workflow checkpoint we want. Keep scanning so a keypress at
        // the tail of a mixed batch cannot override a click/focus/pause.
        if matches!(&message.trigger, CaptureTrigger::KeyPress) {
            continue;
        }
        selected = Some((message.trigger, message.context));
    }

    let Some((mut trigger, mut context)) = selected else {
        if !correlation_ids.is_empty() {
            linker_tx.trigger_dropped(correlation_ids, DystilLinkDropReason::Other);
        }
        return true;
    };

    let ordinary_trigger = is_ordinary_trigger(&trigger);
    if ordinary_trigger {
        if let Some(last) = *last_ordinary_capture {
            let elapsed = last.elapsed();
            if elapsed < ordinary_min_interval {
                tokio::time::sleep(ordinary_min_interval - elapsed).await;
                // Coalesce messages that arrived while the ordinary interval
                // was active. A later semantic trigger wins the context, and
                // all persisted rows still link to the one resulting frame.
                while let Ok(message) = trigger_rx.try_recv() {
                    if let Some(correlation_id) = message.correlation_id {
                        correlation_ids.push(correlation_id);
                    }
                    if !matches!(&message.trigger, CaptureTrigger::KeyPress) {
                        (trigger, context) = (message.trigger, message.context);
                    }
                }
            }
        }
    }

    if let Some(reason) = capture_pause_reason() {
        if !correlation_ids.is_empty() {
            linker_tx.trigger_dropped(correlation_ids, reason);
        }
        return true;
    }

    #[cfg(feature = "debug-capture")]
    let diagnostic_started = std::time::Instant::now();
    #[cfg(feature = "debug-capture")]
    let diagnostic_id = crate::debug_capture::record_capture_request(
        &trigger,
        &context,
        correlation_ids.len(),
        false,
    );
    let result = coordinator.capture(trigger, context).await;
    #[cfg(feature = "debug-capture")]
    match &result {
        Ok(stored) => crate::debug_capture::record_capture_result(
            diagnostic_id,
            diagnostic_started,
            Some(stored.frame_id),
            None,
            None,
        ),
        Err(error) => crate::debug_capture::record_capture_result(
            diagnostic_id,
            diagnostic_started,
            None,
            Some("error"),
            Some(&error.to_string()),
        ),
    }
    match result {
        Ok(stored) => {
            if ordinary_trigger {
                *last_ordinary_capture = Some(std::time::Instant::now());
            }
            if !correlation_ids.is_empty() {
                linker_tx.frame_captured(stored.frame_id, correlation_ids);
            }
        }
        Err(error) if !correlation_ids.is_empty() => {
            warn!(%error, "AX trigger capture failed");
            linker_tx.trigger_dropped(correlation_ids, DystilLinkDropReason::CaptureError);
        }
        Err(error) => debug!(%error, "AX trigger produced no frame"),
    }
    true
}

fn is_ordinary_trigger(trigger: &CaptureTrigger) -> bool {
    matches!(
        trigger,
        CaptureTrigger::Click
            | CaptureTrigger::ScrollStop
            | CaptureTrigger::VisualChange
            | CaptureTrigger::Idle
    )
}

/// Keep the on-demand AX lane aligned with the Dystil continuous lane's
/// global pause sources. The coordinator owns dedup/persistence; this adapter
/// owns reads of Dystil's global runtime state.
fn capture_pause_reason() -> Option<DystilLinkDropReason> {
    // Pause/lock policy is now owned by the Dystil capture session. The
    // adapter must not read Dystil's process-global monitors.
    None
}

// Covers the retired three-mode API. The trigger consumer itself is exercised
// through the coordinator's FullCapture/TextOnly tests while this adapter is
// simplified in the next vendor-lift phase.
#[cfg(any())]
mod tests {
    use async_trait::async_trait;
    use chrono::Utc;
    use dystil_engine::event_driven_capture::trigger_channel;
    use dystil_engine::frame_linker_actor::linker_channel;

    use super::*;
    use crate::{
        AccessibilityProvider, AccessibilitySnapshot, AccessibilityTruncationReason, CaptureConfig,
        CaptureError, CaptureObservation, CaptureStore, StoredCapture, VisualCaptureMode,
    };

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
                text: "AX evidence".to_string(),
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

    struct StoreStub;

    #[async_trait]
    impl CaptureStore for StoreStub {
        async fn persist(
            &self,
            observation: CaptureObservation,
        ) -> Result<StoredCapture, CaptureError> {
            assert!(observation.visual.is_none());
            Ok(StoredCapture {
                frame_id: 42,
                snapshot_path: None,
            })
        }
    }

    #[test]
    fn app_and_window_context_survive_trigger_conversion() {
        let (trigger, context) = convert_trigger(DystilTrigger::AppSwitch {
            app_name: "Code".to_string(),
            target: None,
        });
        assert_eq!(trigger, CaptureTrigger::AppSwitch);
        assert_eq!(context.application.as_deref(), Some("Code"));

        let (trigger, context) = convert_trigger(DystilTrigger::WindowFocus {
            window_name: "matcher.rs".to_string(),
            target: None,
        });
        assert_eq!(trigger, CaptureTrigger::WindowFocus);
        assert_eq!(context.window.as_deref(), Some("matcher.rs"));
    }

    #[tokio::test]
    async fn ax_only_batch_persists_one_frame_and_reports_all_correlations() {
        let (trigger_tx, mut trigger_rx) = trigger_channel();
        let (linker_tx, mut linker_rx) = linker_channel();
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::Disabled,
            },
            Arc::new(AccessibilityStub),
            None,
            Arc::new(StoreStub),
        );
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut last_ordinary_capture = None;

        let first = CaptureTriggerMsg::with_correlation(DystilTrigger::Click { x: 10, y: 20 }, 7);
        trigger_tx
            .send(CaptureTriggerMsg::with_correlation(
                DystilTrigger::KeyPress,
                8,
            ))
            .unwrap();
        capture_trigger_batch(
            first,
            &mut trigger_rx,
            &linker_tx,
            &coordinator,
            Duration::ZERO,
            Duration::ZERO,
            &mut last_ordinary_capture,
            &mut shutdown_rx,
        )
        .await;

        match linker_rx.recv().await.unwrap() {
            LinkerMessage::FrameCaptured(captured) => {
                assert_eq!(captured.frame_id, 42);
                assert_eq!(captured.correlation_ids, vec![7, 8]);
            }
            other => panic!("unexpected linker message: {other:?}"),
        }
    }

    #[tokio::test]
    async fn keypress_only_batch_is_dropped_without_ax_capture() {
        let (_trigger_tx, mut trigger_rx) = trigger_channel();
        let (linker_tx, mut linker_rx) = linker_channel();
        let coordinator = CaptureCoordinator::new(
            CaptureConfig {
                visual_mode: VisualCaptureMode::Disabled,
            },
            Arc::new(AccessibilityStub),
            None,
            Arc::new(StoreStub),
        );
        let (_shutdown_tx, mut shutdown_rx) = watch::channel(false);
        let mut last_ordinary_capture = None;

        capture_trigger_batch(
            CaptureTriggerMsg::with_correlation(DystilTrigger::KeyPress, 9),
            &mut trigger_rx,
            &linker_tx,
            &coordinator,
            Duration::ZERO,
            Duration::ZERO,
            &mut last_ordinary_capture,
            &mut shutdown_rx,
        )
        .await;

        match linker_rx.recv().await.unwrap() {
            LinkerMessage::TriggerDropped {
                correlation_ids,
                reason,
            } => {
                assert_eq!(correlation_ids, vec![9]);
                assert_eq!(reason, DropReason::Other);
            }
            other => panic!("unexpected linker message: {other:?}"),
        }
    }
}
