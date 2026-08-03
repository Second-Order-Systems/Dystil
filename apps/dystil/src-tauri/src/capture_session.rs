//! Short-lived capture session: vision and accessibility/UI recording.
//!
//! Can be started and stopped independently of [`ServerCore`].
//! Borrows shared state from `ServerCore` without taking ownership.
//! without taking ownership — the server stays alive across capture cycles.

use std::sync::Arc;
use std::time::Duration;

use crate::capture_config::DystilCaptureConfig;
use dystil_capture::a11y::tree::TreeWalkerConfig;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

use dystil_capture::{
    accessibility_provider::DystilAccessibilityProvider,
    capture_loop::{DystilAxCaptureConfig, DystilAxCaptureHandle},
    capture_store::DystilCaptureStore,
    linker::DystilLinkerRuntime,
    start_dystil_ui_recording, CaptureConfig, CaptureCoordinator, CaptureMode,
    CaptureTriggerMessage, DystilUiRecorderConfig, DystilUiRecorderHandle, TriggerBus,
    VisualProvider, TRIGGER_CHANNEL_BUFFER,
};

#[cfg(not(target_os = "macos"))]
use dystil_capture::non_macos_visual_capture::DystilFullCaptureVisualProvider;
#[cfg(target_os = "macos")]
use dystil_capture::visual_capture::DystilMacosOneShotVisualProvider;

use crate::server_core::ServerCore;

/// Load the opt-in Dystil ONNX text model.
/// Downloads from HuggingFace on first enable (~168 MB). Returns `None` if the
/// feature is compiled out or the model is unavailable/offline — the worker
/// will fall back to regex-only redaction.
async fn load_text_redactor() -> Option<std::sync::Arc<dyn dystil_redact::TextRedactor>> {
    #[cfg(feature = "onnx-cpu")]
    {
        use dystil_redact::onnx::OnnxRedactor;
        match OnnxRedactor::load_or_download(Default::default()).await {
            Ok(m) => {
                tracing::info!("ONNX text redactor loaded");
                return Some(std::sync::Arc::new(m));
            }
            Err(e) => {
                tracing::warn!(
                    "ONNX text model unavailable, falling back to regex-only redaction: {e}"
                );
            }
        }
    }
    None
}

/// Handle for a running capture session.
///
/// Dropping this without calling [`CaptureSession::stop`] will leak tasks.
/// Always use `stop()` for clean shutdown.
///
pub struct CaptureSession {
    shutdown_tx: broadcast::Sender<()>,
    ui_recorder_handle: Option<DystilUiRecorderHandle>,
    ax_capture_handle: Option<DystilAxCaptureHandle>,
    linker_runtime: Option<DystilLinkerRuntime>,
    redaction_worker: Option<dystil_capture::redaction_worker::RedactionWorker>,
    redactor_load_task: Option<tokio::task::JoinHandle<()>>,
    // Own the trigger bus independently of Dystil's legacy visual loop.
    _capture_trigger_bus: TriggerBus<CaptureTriggerMessage>,
}

impl CaptureSession {
    /// Start all capture pipelines using shared state from `server`.
    ///
    /// This starts:
    /// - Dystil's trigger-driven FullCapture/TextOnly coordinator
    /// - UI event recording (accessibility + input)
    /// - Schedule monitor
    /// - Snapshot compaction
    pub async fn start(
        server: &ServerCore,
        config: &DystilCaptureConfig,
        _close_orphaned_meetings_on_start: bool,
    ) -> Result<Self, String> {
        info!("Starting capture session");

        let (shutdown_tx, _) = broadcast::channel::<()>(1);
        let capture_mode = crate::capture_policy::product_capture_mode(config.disable_vision);
        info!(?capture_mode, "applying code-owned capture policy");

        let capture_trigger_bus = TriggerBus::<CaptureTriggerMessage>::new(TRIGGER_CHANNEL_BUFFER);
        let linker_runtime = DystilLinkerRuntime::start(server.db.pool.clone());
        // Deterministic redaction happens before every SQLite write regardless
        // of this preference. The model, its download, and the async worker are
        // created only after an explicit opt-in. The worker waits for the model
        // rather than prematurely completing queued rows with regex alone.
        let (redaction_worker, redactor_load_task) = if config.async_pii_redaction {
            let worker = dystil_capture::redaction_worker::RedactionWorker::start(
                server.db.pool.clone(),
                None,
            );
            let redactor_model = worker.model_handle();
            let load_task = tokio::spawn(async move {
                if let Some(model) = load_text_redactor().await {
                    *redactor_model.write().await = Some(model);
                    info!("ONNX text redactor is now available to the background worker");
                }
            });
            (Some(worker), Some(load_task))
        } else {
            info!("AI PII removal disabled; skipping local model download and worker startup");
            (None, None)
        };

        // Both channels are session-owned. UI recording produces activity
        // triggers; Dystil owns all evidence capture for every platform.
        let capture_trigger_tx = capture_trigger_bus.sender();
        let mut ax_capture_handle = None;

        // Check accessibility independently from visual permission. AX-only
        // mode must not accidentally request or depend on Screen Recording.
        #[cfg(target_os = "macos")]
        let accessibility_permitted = crate::permissions::check_accessibility_inline();
        #[cfg(not(target_os = "macos"))]
        let accessibility_permitted = true;

        // --- Vision ---
        // Gate on screen recording permission before calling any ScreenCaptureKit API.
        // On macOS 15+ SCShareableContent::current() (called by list_monitors inside
        // VisionManager::start) shows Apple's native TCC padlock dialog if the app has
        // not been granted Screen Recording access yet — even before onboarding runs.
        // check_screen_recording_tauri() skips capture_probe on macOS 15+ (avoids the
        // native TCC dialog CGWindowListCreateImage triggers). Skip vision entirely when not granted;
        // spawn_capture is called again from onboarding after the user grants access.
        #[cfg(target_os = "macos")]
        let screen_recording_permitted = capture_mode == CaptureMode::TextOnly
            || crate::permissions::check_screen_recording_inline();
        #[cfg(not(target_os = "macos"))]
        let screen_recording_permitted = true;

        if capture_mode == CaptureMode::FullCapture && !screen_recording_permitted {
            warn!("Screen recording permission not yet granted — FullCapture will retain accessibility evidence only");
        }

        // One Dystil-owned capture plane consumes the UI trigger stream on all
        // platforms. FullCapture gets AX and a screenshot in one persisted
        // frame; TextOnly never invokes a visual provider.
        if accessibility_permitted {
            let tree_config = TreeWalkerConfig {
                ignored_windows: config.ignored_windows.clone(),
                included_windows: config.included_windows.clone(),
                ignored_urls: config.ignored_urls.clone(),
                ignore_incognito_windows: config.ignore_incognito_windows,
                ..TreeWalkerConfig::default()
            };
            let accessibility = Arc::new(DystilAccessibilityProvider::new(tree_config));
            let store = Arc::new(DystilCaptureStore::new(
                server.db.pool.clone(),
                server.data_path.join("data"),
                "accessibility",
                config.async_pii_redaction,
            ));
            #[cfg(target_os = "macos")]
            let visual_provider: Option<Arc<dyn VisualProvider>> =
                if capture_mode == CaptureMode::FullCapture && screen_recording_permitted {
                    Some(Arc::new(DystilMacosOneShotVisualProvider::new(
                        config.ignored_windows.clone(),
                        config.included_windows.clone(),
                        config.ignored_urls.clone(),
                    )))
                } else {
                    None
                };
            #[cfg(not(target_os = "macos"))]
            let visual_provider: Option<Arc<dyn VisualProvider>> =
                (capture_mode == CaptureMode::FullCapture).then(|| {
                    Arc::new(DystilFullCaptureVisualProvider::new()) as Arc<dyn VisualProvider>
                });

            let coordinator = Arc::new(CaptureCoordinator::new(
                CaptureConfig { capture_mode },
                accessibility,
                visual_provider.clone(),
                store,
            ));
            ax_capture_handle = Some(DystilAxCaptureHandle::start(
                capture_trigger_bus.subscribe(),
                linker_runtime.sender(),
                coordinator.clone(),
                DystilAxCaptureConfig::default(),
            ));

            info!(
                ?capture_mode,
                has_visual_provider = visual_provider.is_some(),
                "Dystil capture coordinator started"
            );
        }

        // --- UI event recording ---
        // Gate on accessibility permission before calling start_ui_recording.
        // Internally it calls recorder.request_permissions() →
        // AXIsProcessTrustedWithOptions(prompt: true) which shows Apple's
        // native accessibility TCC dialog for users who haven't granted it yet.
        // AXIsProcessTrusted() (used by check_accessibility) is silent.
        let ui_recorder_handle = if !accessibility_permitted {
            warn!("Accessibility permission not yet granted — skipping UI event recording to avoid native TCC dialog; will start on next spawn_capture after onboarding grants access");
            None
        } else {
            let ui_config = DystilUiRecorderConfig {
                capture_clicks: true,
                capture_scroll: config.capture_scroll.unwrap_or(false),
                capture_clipboard: !config.disable_clipboard_capture
                    || config.capture_on_clipboard.unwrap_or(true),
                capture_clipboard_content: !config.disable_clipboard_capture,
                capture_text: !config.disable_keyboard_capture,
                capture_keystrokes: true,
                record_keyboard_events: !config.disable_keyboard_capture,
                record_clipboard_events: !config.disable_clipboard_capture,
                ignored_windows: config.ignored_windows.clone(),
                included_windows: config.included_windows.clone(),
                batch_size: 100,
                batch_timeout_ms: 1000,
                typing_pause_delay_ms: 300,
                prioritize_input_latency: config.prioritize_input_latency,
                extraction_thread_priority: config
                    .extraction_thread_priority
                    .parse()
                    .unwrap_or_default(),
                pause_extraction_on_input_ms: config.pause_extraction_on_input_ms,
            };
            match start_dystil_ui_recording(
                server.db.pool.clone(),
                ui_config,
                capture_trigger_tx,
                linker_runtime.sender(),
            ) {
                Ok(handle) => {
                    info!("UI event recording started successfully");
                    Some(handle)
                }
                Err(e) => {
                    error!("Failed to start UI event recording: {}", e);
                    None
                }
            }
        };

        // --- Snapshot compaction ---
        info!("snapshot compaction disabled for Dystil image ingest");

        info!("Capture session started successfully");

        Ok(Self {
            shutdown_tx,
            ui_recorder_handle,
            ax_capture_handle,
            linker_runtime: Some(linker_runtime),
            redaction_worker,
            redactor_load_task,
            _capture_trigger_bus: capture_trigger_bus,
        })
    }

    /// Stop all capture pipelines. The server stays alive.
    ///
    /// This is self-contained — no external references needed.
    pub async fn stop(mut self) {
        info!("Stopping capture session");

        // Signal UI recorder to stop
        if let Some(ref ui_handle) = self.ui_recorder_handle {
            ui_handle.stop();
        }

        // Broadcast shutdown to schedule monitor and other session consumers.
        let _ = self.shutdown_tx.send(());

        // Wait for UI recorder tasks to finish
        if let Some(ui_handle) = self.ui_recorder_handle.take() {
            info!("Waiting for UI recorder tasks to finish...");
            match tokio::time::timeout(Duration::from_secs(5), ui_handle.join()).await {
                Ok(()) => info!("UI recorder tasks finished cleanly"),
                Err(_) => warn!("UI recorder tasks did not finish within 5s"),
            }
        }

        // Already-emitted triggers can finish while the UI recorder's final
        // event batch is flushed to the database.
        if let Some(ax_capture_handle) = self.ax_capture_handle.take() {
            ax_capture_handle.shutdown().await;
        }

        // This legacy invalidation is intentionally after both visual lanes:
        // it cannot race an in-flight on-demand stream.
        invalidate_macos_screen_streams("capture session stop").await;

        // Stop the linker after every producer has drained so pending UI
        // events can still be paired with their final persisted frame.
        if let Some(linker_runtime) = self.linker_runtime.take() {
            linker_runtime.shutdown().await;
        }

        if let Some(task) = self.redactor_load_task.take() {
            task.abort();
            let _ = task.await;
        }

        // Redaction worker runs independently; stop it last so it can process
        // any rows queued during the final capture flush above.
        if let Some(worker) = self.redaction_worker.take() {
            worker.shutdown().await;
        }

        info!("Capture session stopped");
    }
}

#[cfg(target_os = "macos")]
async fn invalidate_macos_screen_streams(reason: &str) {
    info!("Invalidating macOS ScreenCaptureKit screenshot streams ({reason})");
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::task::spawn_blocking(|| {
            dystil_capture::screen::stream_invalidation::invalidate_streams();
        }),
    )
    .await;

    match result {
        Ok(Ok(())) => info!("macOS ScreenCaptureKit screenshot streams invalidated"),
        Ok(Err(e)) => warn!("macOS ScreenCaptureKit invalidation task failed: {}", e),
        Err(_) => warn!("macOS ScreenCaptureKit stream invalidation timed out after 5s"),
    }

    tokio::time::sleep(Duration::from_millis(500)).await;
}

#[cfg(not(target_os = "macos"))]
async fn invalidate_macos_screen_streams(_reason: &str) {}
