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
    let mut last_scroll: Option<(Instant, u64)> = None;
    loop {
        tokio::select! {
            event = event_rx.recv() => match event {
                Some(event) => {
                    let persist = should_persist(&event, &config);
                    let keyboard_activity = matches!(event.data, EventData::Key { .. } | EventData::Text { .. });
                    let trigger = immediate_trigger(&event);
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
                        let is_scroll = matches!(event.data, EventData::Scroll { .. });
                        let record = UiEventRecord::from_native(event, session_id.clone());
                        if is_scroll {
                            let id = correlation.unwrap_or_else(|| linker.next_correlation_id());
                            last_scroll = Some((Instant::now(), id));
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
                if let Some((at, correlation_id)) = last_scroll {
                    if at.elapsed() >= Duration::from_millis(300) {
                        let _ = trigger_tx.send(CaptureTriggerMessage::with_correlation(
                            CaptureTrigger::ScrollStop, CaptureContext::default(), correlation_id));
                        last_scroll = None;
                    }
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

fn immediate_trigger(event: &UiEvent) -> Option<CaptureTriggerMessage> {
    let mut context = event_context(event);
    let trigger = match &event.data {
        EventData::Click { x, y, .. } => {
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
        }
    }

    #[test]
    fn private_text_is_not_persisted_and_does_not_trigger_immediate_capture() {
        let event = UiEvent::text(Utc::now(), 1, "person@example.com".to_string());
        assert!(!should_persist(&event, &config()));
        assert!(immediate_trigger(&event).is_none());
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
            immediate_trigger(&event).unwrap().trigger,
            CaptureTrigger::Clipboard
        );
    }
}
