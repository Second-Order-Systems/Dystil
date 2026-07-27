use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::screen::capture_screenshot_by_window::{get_excluded_sck_window_ids, WindowFilters};
use crate::screen::monitor::{list_monitors_detailed, SafeMonitor};
use async_trait::async_trait;
use chrono::Utc;
use tracing::{debug, info, warn};

use crate::monitor_selection::select_monitors;
use crate::{CaptureError, VisualProvider, VisualRequest, VisualSnapshot};

/// macOS FullCapture provider backed by short-lived SCStreams.
///
/// There is intentionally no retained stream state. Each selected monitor's
/// stream is stopped before the next monitor begins, and all streams have
/// stopped before JPEG encoding or database persistence begins.
pub struct DystilMacosOneShotVisualProvider {
    ignored_windows: Vec<String>,
    included_windows: Vec<String>,
    ignored_urls: Vec<String>,
    first_frame_timeout: Duration,
    max_capture_width: u32,
    active_acquisitions: AtomicUsize,
}

impl DystilMacosOneShotVisualProvider {
    pub fn new(
        ignored_windows: Vec<String>,
        included_windows: Vec<String>,
        ignored_urls: Vec<String>,
    ) -> Self {
        Self {
            ignored_windows,
            included_windows,
            ignored_urls,
            first_frame_timeout: Duration::from_secs(2),
            max_capture_width: 1_920,
            active_acquisitions: AtomicUsize::new(0),
        }
    }

    pub fn with_first_frame_timeout(mut self, timeout: Duration) -> Self {
        self.first_frame_timeout = timeout;
        self
    }

    pub fn active_acquisitions(&self) -> usize {
        self.active_acquisitions.load(Ordering::SeqCst)
    }

    async fn capture_monitor(
        &self,
        monitor: SafeMonitor,
        exclusions: Vec<u32>,
    ) -> Result<VisualSnapshot, CaptureError> {
        let total_started = Instant::now();
        let monitor_id = monitor.id();
        let device_name = monitor.name().to_string();

        let start_started = Instant::now();
        let capture = tokio::task::spawn_blocking(move || monitor.start_hd_capture(1, &exclusions))
            .await
            .map_err(|error| CaptureError::Visual(format!("SCStream start task failed: {error}")))?
            .map_err(|error| CaptureError::Visual(format!("SCStream failed to start: {error}")))?;
        let start_ms = elapsed_ms(start_started);
        let crate::screen::HdCapture {
            stream,
            frames: mut frames,
            ..
        } = capture;

        let frame_started = Instant::now();
        let frame_result = tokio::time::timeout(self.first_frame_timeout, frames.recv()).await;
        let first_frame_ms = elapsed_ms(frame_started);

        // Stop before returning pixels. SnapshotWriter encoding and DB writes
        // therefore cannot extend the macOS recording indicator lifetime.
        let stop_started = Instant::now();
        let stop_result = tokio::task::spawn_blocking(move || drop(stream)).await;
        let stop_ms = elapsed_ms(stop_started);
        drop(frames);
        stop_result
            .map_err(|error| CaptureError::Visual(format!("SCStream stop task failed: {error}")))?;

        let frame = match frame_result {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                return Err(CaptureError::Visual(
                    "SCStream ended before producing a frame".to_string(),
                ));
            }
            Err(_) => {
                return Err(CaptureError::Visual(format!(
                    "timed out after {} ms waiting for the first SCStream frame",
                    self.first_frame_timeout.as_millis()
                )));
            }
        };

        info!(
            monitor_id,
            device_name,
            start_ms,
            first_frame_ms,
            stop_ms,
            capture_lifecycle_ms = start_ms + first_frame_ms + stop_ms,
            total_ms = elapsed_ms(total_started),
            active_acquisitions = self.active_acquisitions(),
            "macOS FullCapture SCStream captured one monitor and stopped"
        );

        Ok(VisualSnapshot {
            captured_at: Utc::now(),
            image: Arc::new(image::DynamicImage::ImageRgba8(frame)),
            monitor_id: Some(monitor_id),
            device_name: Some(device_name),
        })
    }
}

#[async_trait]
impl VisualProvider for DystilMacosOneShotVisualProvider {
    async fn capture_all(
        &self,
        request: &VisualRequest,
    ) -> Result<Vec<VisualSnapshot>, CaptureError> {
        let _active = ActiveAcquisitionGuard::new(&self.active_acquisitions);
        crate::screen::monitor::set_sck_capture_max_width(self.max_capture_width);
        let monitors = list_monitors_detailed().await.map_err(|error| {
            CaptureError::Visual(format!("failed to enumerate displays: {error}"))
        })?;
        if monitors.is_empty() {
            return Err(CaptureError::Visual(
                "no capturable display was found".to_string(),
            ));
        }
        let connected_monitors = monitors
            .iter()
            .map(|monitor| {
                format!(
                    "{}:{}@{},{} {}x{} primary={}",
                    monitor.id(),
                    monitor.name(),
                    monitor.x(),
                    monitor.y(),
                    monitor.width(),
                    monitor.height(),
                    monitor.is_primary()
                )
            })
            .collect::<Vec<_>>();
        let focused = None;
        let selection = select_monitors(monitors, &request.context, focused);
        let selected_monitor_ids = selection
            .monitors
            .iter()
            .map(SafeMonitor::id)
            .collect::<Vec<_>>();
        info!(
            selection_reason = ?selection.reason,
            selected_monitor_ids = ?selected_monitor_ids,
            trigger_target = ?request.context.target,
            context_monitor_id = ?request.context.monitor_id,
            focused_monitor_id = ?focused,
            connected_monitors = ?connected_monitors,
            "FullCapture selected macOS monitor set"
        );

        let ignored_windows = self.ignored_windows.clone();
        let included_windows = self.included_windows.clone();
        let ignored_urls = self.ignored_urls.clone();
        let exclusions = tokio::task::spawn_blocking(move || {
            let filters = WindowFilters::new(&ignored_windows, &included_windows, &ignored_urls);
            get_excluded_sck_window_ids(&filters)
        })
        .await
        .map_err(|error| CaptureError::Visual(format!("window filter task failed: {error}")))?;

        let attempted = selection.monitors.len();
        let mut snapshots = Vec::with_capacity(attempted);
        let mut failures = Vec::new();
        for monitor in selection.monitors {
            let monitor_id = monitor.id();
            let device_name = monitor.name().to_string();
            match self.capture_monitor(monitor, exclusions.clone()).await {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(error) => {
                    warn!(
                        monitor_id,
                        device_name,
                        %error,
                        "FullCapture failed to capture one macOS monitor"
                    );
                    failures.push(format!("{monitor_id} ({device_name}): {error}"));
                }
            }
        }

        if snapshots.is_empty() {
            return Err(CaptureError::Visual(format!(
                "failed to capture all {attempted} selected display(s): {}",
                failures.join("; ")
            )));
        }
        if !failures.is_empty() {
            warn!(
                attempted,
                captured = snapshots.len(),
                failures = ?failures,
                "FullCapture completed with partial macOS monitor coverage"
            );
        }
        Ok(snapshots)
    }

    async fn stop(&self) -> Result<(), CaptureError> {
        // `capture_all` synchronously tears down every stream before returning.
        debug!("macOS one-shot visual provider has no retained stream to stop");
        Ok(())
    }
}

struct ActiveAcquisitionGuard<'a> {
    active: &'a AtomicUsize,
}

impl<'a> ActiveAcquisitionGuard<'a> {
    fn new(active: &'a AtomicUsize) -> Self {
        active.fetch_add(1, Ordering::SeqCst);
        Self { active }
    }
}

impl Drop for ActiveAcquisitionGuard<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}
