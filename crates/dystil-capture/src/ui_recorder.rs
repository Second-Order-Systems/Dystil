use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::a11y::{EventData, UiEvent, UiRecorder};
use sqlx::SqlitePool;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tracing::warn;
use uuid::Uuid;

use crate::linker::DystilLinkerSender;
use crate::ui_event_store::enrich_persisted_physical_click;
use crate::{
    insert_ui_event_batch, CaptureContext, CaptureTrigger, CaptureTriggerMessage,
    DystilUiRecorderConfig, ScreenPoint, UiEventRecord,
};

pub struct DystilUiRecorderHandle {
    stop: Arc<AtomicBool>,
    producer: Option<JoinHandle<()>>,
    consumer: Option<JoinHandle<()>>,
}

impl DystilUiRecorderHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn stop_flag(&self) -> Arc<AtomicBool> {
        self.stop.clone()
    }
    pub fn is_running(&self) -> bool {
        !self.stop.load(Ordering::Relaxed)
    }
    pub async fn join(mut self) {
        if let Some(handle) = self.producer.take() {
            let _ = handle.await;
        }
        if let Some(handle) = self.consumer.take() {
            let _ = handle.await;
        }
    }
}

pub fn start_dystil_ui_recording(
    pool: SqlitePool,
    config: DystilUiRecorderConfig,
    trigger_tx: broadcast::Sender<CaptureTriggerMessage>,
    linker: DystilLinkerSender,
) -> Result<DystilUiRecorderHandle, String> {
    let recorder = UiRecorder::new(config.native_config());
    let permissions = recorder.check_permissions();
    if !permissions.accessibility {
        return Err("accessibility permission unavailable".to_string());
    }
    let native = recorder.start().map_err(|error| error.to_string())?;
    let stop = Arc::new(AtomicBool::new(false));
    let producer_stop = stop.clone();
    let (event_tx, event_rx) = mpsc::channel(1024);
    let producer = tokio::task::spawn_blocking(move || {
        while !producer_stop.load(Ordering::Relaxed) {
            if let Some(event) = native.recv_timeout(Duration::from_millis(100)) {
                if event_tx.blocking_send(event).is_err() {
                    break;
                }
            }
        }
        native.stop();
    });
    let consumer_stop = stop.clone();
    let consumer = tokio::spawn(run_consumer(
        pool,
        config,
        trigger_tx,
        linker,
        event_rx,
        consumer_stop,
    ));
    Ok(DystilUiRecorderHandle {
        stop,
        producer: Some(producer),
        consumer: Some(consumer),
    })
}

async fn run_consumer(
    pool: SqlitePool,
    config: DystilUiRecorderConfig,
    trigger_tx: broadcast::Sender<CaptureTriggerMessage>,
    linker: DystilLinkerSender,
    mut event_rx: mpsc::Receiver<UiEvent>,
    stop: Arc<AtomicBool>,
) {
    let session_id = Uuid::new_v4().to_string();
    let mut batch: Vec<(UiEventRecord, Option<u64>)> = Vec::with_capacity(config.batch_size);
    let mut tick = tokio::time::interval(Duration::from_millis(config.batch_timeout_ms.max(1)));
    let typing_timer = tokio::time::sleep(Duration::from_secs(24 * 60 * 60));
    tokio::pin!(typing_timer);
    let mut typing_pending = false;
    let mut typing_context = CaptureContext::default();
    let mut typing_correlations = Vec::new();
    let mut last_scroll: Option<(Instant, Vec<u64>, CaptureContext, i64, i64)> = None;
    loop {
        tokio::select! {
            event = event_rx.recv() => match event {
                Some(event) => {
                    let is_enrichment = is_element_enrichment(&event);
                    if config.merge_click_enrichment && is_enrichment {
                        if merge_pending_click_enrichment(&mut batch, &event) {
                            #[cfg(feature = "debug-capture")]
                            crate::debug_capture::record_ui_event(&event, false, false);
                            continue;
                        }
                        match enrich_persisted_physical_click(&pool, &session_id, &event).await {
                            Ok(true) => {
                                #[cfg(feature = "debug-capture")]
                                crate::debug_capture::record_ui_event(&event, false, false);
                                continue;
                            }
                            Ok(false) => {
                                // Keep a visible fallback rather than silently dropping an
                                // enrichment whose physical click was unavailable.
                                warn!("click enrichment had no matching physical click; retaining fallback event");
                            }
                            Err(error) => {
                                warn!(%error, "could not merge persisted click enrichment; retaining fallback event");
                            }
                        }
                    }
                    let persist = should_persist(&event, &config);
                    let keyboard_activity = matches!(event.data, EventData::Key { .. } | EventData::Text { .. });
                    if config.settled_state_scheduler && matches!(event.data, EventData::Key { .. }) {
                        let _ = trigger_tx.send(CaptureTriggerMessage::new(CaptureTrigger::KeyPress, event_context(&event)));
                    }
                    let trigger = immediate_trigger(&event, config.merge_click_enrichment);
                    #[cfg(feature = "debug-capture")]
                    crate::debug_capture::record_ui_event(
                        &event,
                        persist,
                        trigger.is_some() || keyboard_activity,
                    );
                    let mut correlation = if persist && (trigger.is_some() || keyboard_activity) && trigger_tx.receiver_count() > 0 {
                        Some(linker.next_correlation_id())
                    } else { None };
                    if keyboard_activity {
                        typing_pending = true;
                        typing_context = event_context(&event);
                        if let Some(id) = correlation {
                            typing_correlations.push(id);
                        }
                        typing_timer.as_mut().reset(
                            tokio::time::Instant::now()
                                + Duration::from_millis(config.typing_pause_delay_ms),
                        );
                    } else if let Some(message) = trigger {
                        let message = CaptureTriggerMessage { correlation_id: correlation, ..message };
                        if trigger_tx.send(message).is_err() { correlation = None; }
                    }
                    if persist {
                        let scroll = match &event.data {
                            EventData::Scroll { delta_x, delta_y, .. } => {
                                Some((i64::from(*delta_x), i64::from(*delta_y)))
                            }
                            _ => None,
                        };
                        let record = UiEventRecord::from_native(event, session_id.clone());
                        if let Some((delta_x, delta_y)) = scroll {
                            let id = correlation.unwrap_or_else(|| linker.next_correlation_id());
                            let context = CaptureContext {
                                application: record.app_name.clone(),
                                window: record.window_title.clone(),
                                browser_url: record.browser_url.clone(),
                                ..CaptureContext::default()
                            };
                            match &mut last_scroll {
                                Some((at, ids, previous_context, total_x, total_y)) => {
                                    *at = Instant::now();
                                    ids.push(id);
                                    *previous_context = context.with_fallback(previous_context);
                                    *total_x += delta_x;
                                    *total_y += delta_y;
                                }
                                None => {
                                    last_scroll = Some((
                                        Instant::now(),
                                        vec![id],
                                        context,
                                        delta_x,
                                        delta_y,
                                    ));
                                }
                            }
                            batch.push((record, Some(id)));
                        } else {
                            batch.push((record, correlation));
                        }
                    }
                    if batch.len() >= config.batch_size { flush(&pool, &linker, &mut batch).await; }
                    if batch.len() > config.batch_size.saturating_mul(2) {
                        let drain = batch.len() - config.batch_size;
                        warn!(drain, "dropping oldest retained UI events after repeated SQLite failure");
                        batch.drain(..drain);
                    }
                }
                None => break,
            },
            _ = &mut typing_timer, if typing_pending => {
                typing_pending = false;
                if typing_correlations.is_empty() {
                    let _ = trigger_tx.send(CaptureTriggerMessage::new(
                        CaptureTrigger::TypingPause,
                        typing_context.clone(),
                    ));
                } else {
                    for correlation_id in typing_correlations.drain(..) {
                        let _ = trigger_tx.send(CaptureTriggerMessage::with_correlation(
                            CaptureTrigger::TypingPause,
                            typing_context.clone(),
                            correlation_id,
                        ));
                    }
                }
            }
            _ = tick.tick() => {
                flush(&pool, &linker, &mut batch).await;
                if let Some((at, correlation_ids, context, delta_x, delta_y)) = last_scroll.take() {
                    if at.elapsed() >= Duration::from_millis(config.scroll_stop_delay_ms) {
                        let mut message = CaptureTriggerMessage::new(CaptureTrigger::ScrollStop, context);
                        message.correlation_id = correlation_ids.first().copied();
                        message.additional_correlation_ids = correlation_ids.into_iter().skip(1).collect();
                        message.activity_delta_x = delta_x;
                        message.activity_delta_y = delta_y;
                        let _ = trigger_tx.send(message);
                    }
                    else { last_scroll = Some((at, correlation_ids, context, delta_x, delta_y)); }
                }
                if stop.load(Ordering::Relaxed) && event_rx.is_empty() { break; }
            }
        }
    }
    flush(&pool, &linker, &mut batch).await;
}

async fn flush(
    pool: &SqlitePool,
    linker: &DystilLinkerSender,
    batch: &mut Vec<(UiEventRecord, Option<u64>)>,
) {
    if batch.is_empty() {
        return;
    }
    let records: Vec<_> = batch.iter().map(|(record, _)| record.clone()).collect();
    match insert_ui_event_batch(pool, &records).await {
        Ok(ids) => {
            for (row_id, (_, correlation)) in ids.into_iter().zip(batch.iter()) {
                if let Some(correlation) = correlation {
                    linker.event_persisted(*correlation, row_id);
                }
            }
            batch.clear();
        }
        Err(error) => {
            warn!(%error, count = batch.len(), "Dystil UI-event batch insert failed; retaining batch")
        }
    }
}

fn should_persist(event: &UiEvent, config: &DystilUiRecorderConfig) -> bool {
    match event.data {
        EventData::Key { .. } | EventData::Text { .. } => config.record_keyboard_events,
        EventData::Clipboard { .. } => config.record_clipboard_events,
        _ => true,
    }
}

fn immediate_trigger(
    event: &UiEvent,
    merge_click_enrichment: bool,
) -> Option<CaptureTriggerMessage> {
    let mut context = event_context(event);
    let trigger = match &event.data {
        EventData::Click {
            x, y, click_count, ..
        } => {
            if merge_click_enrichment && *click_count == 0 {
                return None;
            }
            context.target = Some(ScreenPoint { x: *x, y: *y });
            CaptureTrigger::Click
        }
        EventData::AppSwitch { name, .. } => {
            context.application = Some(name.clone());
            CaptureTrigger::AppSwitch
        }
        EventData::WindowFocus { app, title } => {
            context.application = Some(app.clone());
            context.window = title.clone();
            CaptureTrigger::WindowFocus
        }
        EventData::Clipboard { .. } => CaptureTrigger::Clipboard,
        EventData::Key { .. }
        | EventData::Text { .. }
        | EventData::Scroll { .. }
        | EventData::Move { .. } => return None,
    };
    Some(CaptureTriggerMessage::new(trigger, context))
}

fn is_element_enrichment(event: &UiEvent) -> bool {
    matches!(event.data, EventData::Click { click_count: 0, .. }) && event.element.is_some()
}

fn merge_pending_click_enrichment(
    batch: &mut [(UiEventRecord, Option<u64>)],
    enrichment: &UiEvent,
) -> bool {
    let (x, y, element) = match (&enrichment.data, enrichment.element.as_ref()) {
        (
            EventData::Click {
                x,
                y,
                click_count: 0,
                ..
            },
            Some(element),
        ) => (*x, *y, element),
        _ => return false,
    };
    if let Some((record, _)) = batch.iter_mut().rev().find(|(record, _)| {
        record.event_type == "click"
            && record.click_count.is_some_and(|count| count > 0)
            && record.timestamp == enrichment.timestamp
            && record.x == Some(x)
            && record.y == Some(y)
    }) {
        record.apply_element_context(element);
        true
    } else {
        false
    }
}

fn event_context(event: &UiEvent) -> CaptureContext {
    CaptureContext {
        application: event.app_name.clone(),
        window: event.window_title.clone(),
        browser_url: event.browser_url.clone(),
        ..CaptureContext::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::a11y::ElementContext;
    use chrono::Utc;

    fn config() -> DystilUiRecorderConfig {
        DystilUiRecorderConfig {
            capture_clicks: true,
            capture_scroll: false,
            capture_clipboard: true,
            capture_clipboard_content: false,
            capture_text: false,
            capture_keystrokes: true,
            record_keyboard_events: false,
            record_clipboard_events: false,
            ignored_windows: vec![],
            included_windows: vec![],
            batch_size: 100,
            batch_timeout_ms: 1000,
            typing_pause_delay_ms: 1_500,
            prioritize_input_latency: false,
            extraction_thread_priority: Default::default(),
            pause_extraction_on_input_ms: 150,
            merge_click_enrichment: false,
            settled_state_scheduler: false,
            scroll_stop_delay_ms: 300,
            capture_background_trees: true,
            precise_click_window_context: false,
        }
    }

    #[test]
    fn private_text_is_not_persisted_and_does_not_trigger_immediate_capture() {
        let event = UiEvent::text(Utc::now(), 1, "person@example.com".to_string());
        assert!(!should_persist(&event, &config()));
        assert!(immediate_trigger(&event, false).is_none());
    }

    #[test]
    fn clipboard_trigger_survives_disabled_storage() {
        let event = UiEvent {
            id: None,
            timestamp: Utc::now(),
            relative_ms: 1,
            data: EventData::Clipboard {
                operation: 'v',
                content: Some("secret".to_string()),
            },
            app_name: None,
            window_title: None,
            browser_url: None,
            element: None,
            frame_id: None,
        };
        assert!(!should_persist(&event, &config()));
        assert_eq!(
            immediate_trigger(&event, false).unwrap().trigger,
            CaptureTrigger::Clipboard
        );
    }

    #[test]
    fn candidate_merges_precise_target_into_pending_physical_click() {
        let timestamp = Utc::now();
        let physical = UiEvent::click(timestamp, 12, 44, 55, 0, 1, 0);
        let mut batch = vec![(
            UiEventRecord::from_native(physical, "session".into()),
            Some(7),
        )];
        let enrichment = UiEvent {
            id: None,
            timestamp,
            relative_ms: 0,
            data: EventData::Click {
                x: 44,
                y: 55,
                button: 0,
                click_count: 0,
                modifiers: 0,
            },
            app_name: None,
            window_title: None,
            browser_url: None,
            element: Some(ElementContext {
                role: "Button".into(),
                name: Some("Send".into()),
                value: None,
                description: None,
                automation_id: Some("send-button".into()),
                bounds: None,
            }),
            frame_id: None,
        };

        assert!(merge_pending_click_enrichment(&mut batch, &enrichment));
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].0.element_role.as_deref(), Some("Button"));
        assert_eq!(
            batch[0].0.element_automation_id.as_deref(),
            Some("send-button")
        );
        assert!(immediate_trigger(&enrichment, true).is_none());
    }

    #[test]
    fn unmatched_enrichment_is_not_merged_with_another_click() {
        let timestamp = Utc::now();
        let physical = UiEvent::click(timestamp, 12, 44, 55, 0, 1, 0);
        let mut batch = vec![(
            UiEventRecord::from_native(physical, "session".into()),
            Some(7),
        )];
        let enrichment = UiEvent {
            id: None,
            timestamp,
            relative_ms: 0,
            data: EventData::Click {
                x: 45,
                y: 55,
                button: 0,
                click_count: 0,
                modifiers: 0,
            },
            app_name: None,
            window_title: None,
            browser_url: None,
            element: Some(ElementContext {
                role: "Button".into(),
                name: None,
                value: None,
                description: None,
                automation_id: None,
                bounds: None,
            }),
            frame_id: None,
        };

        assert!(!merge_pending_click_enrichment(&mut batch, &enrichment));
        assert_eq!(batch.len(), 1);
    }
}
