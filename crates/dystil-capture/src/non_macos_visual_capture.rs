use std::collections::HashMap;
use std::sync::Arc;

use crate::screen::monitor::{list_monitors_detailed, SafeMonitor};
use async_trait::async_trait;
use chrono::Utc;
use image::DynamicImage;
use tokio::sync::Mutex;
#[cfg(target_os = "linux")]
use tracing::debug;
use tracing::{info, warn};

use crate::monitor_selection::select_monitors;
use crate::{CaptureError, VisualProvider, VisualRequest, VisualSnapshot};

/// Trigger-driven screenshot provider for Windows and Linux.
///
/// The provider owns every connected `SafeMonitor`. On Windows this retains
/// Dystil's WGC session between accepted FullCapture triggers; on Linux
/// `SafeMonitor::capture_image()` keeps its existing per-frame behavior.
///
/// A reliably mapped trigger/focus captures one monitor. If routing metadata
/// is absent or uses a coordinate space that does not match the capture API,
/// the provider captures every connected monitor. This is the same safe
/// fallback used by Dystil's former per-monitor loops.
pub struct DystilFullCaptureVisualProvider {
    monitors: Mutex<HashMap<u32, SafeMonitor>>,
}

impl DystilFullCaptureVisualProvider {
    pub fn new() -> Self {
        Self {
            monitors: Mutex::new(HashMap::new()),
        }
    }

    async fn monitors_for(
        &self,
        request: &VisualRequest,
    ) -> Result<Vec<SafeMonitor>, CaptureError> {
        let connected = list_monitors_detailed().await.map_err(|error| {
            CaptureError::Visual(format!("failed to enumerate displays: {error}"))
        })?;
        if connected.is_empty() {
            return Err(CaptureError::Visual(
                "no capturable display was found".to_string(),
            ));
        }
        let connected_monitors = connected
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
        #[cfg(target_os = "linux")]
        let selection_context = resolve_wayland_monitor_context(&connected, &request.context);
        #[cfg(not(target_os = "linux"))]
        let selection_context = request.context.clone();

        // Unknown focus is represented explicitly and safely fans out when
        // neither the platform-native output geometry nor the capture API can
        // reconcile the trigger point.
        let focused = None;
        let selection = select_monitors(connected.clone(), &selection_context, focused);
        let selected_ids = selection
            .monitors
            .iter()
            .map(SafeMonitor::id)
            .collect::<Vec<_>>();

        let mut cached = self.monitors.lock().await;
        cached.retain(|id, monitor| {
            let still_connected = connected.iter().any(|candidate| candidate.id() == *id);
            if !still_connected {
                monitor.release_capture_stream();
            }
            still_connected
        });
        for monitor in connected {
            cached.entry(monitor.id()).or_insert(monitor);
        }

        let selected = selected_ids
            .iter()
            .filter_map(|id| cached.get(id).cloned())
            .collect::<Vec<_>>();
        info!(
            selection_reason = ?selection.reason,
            selected_monitor_ids = ?selected_ids,
            trigger_target = ?request.context.target,
            context_monitor_id = ?request.context.monitor_id,
            focused_monitor_id = ?focused,
            connected_monitors = ?connected_monitors,
            "FullCapture selected Linux/Windows monitor set"
        );
        Ok(selected)
    }
}

#[cfg(target_os = "linux")]
fn resolve_wayland_monitor_context(
    monitors: &[SafeMonitor],
    context: &crate::CaptureContext,
) -> crate::CaptureContext {
    if context.monitor_id.is_some() || context.target.is_none() {
        return context.clone();
    }
    let Some(target) = context.target else {
        return context.clone();
    };
    let Ok(connection) = libwayshot_xcap::WayshotConnection::new() else {
        return context.clone();
    };
    let matching_name = connection.get_all_outputs().iter().find_map(|output| {
        let region = output.logical_region.inner;
        point_in_logical_output(
            target.x,
            target.y,
            region.position.x,
            region.position.y,
            region.size.width,
            region.size.height,
        )
        .then_some(output.name.as_str())
    });
    let Some(monitor_id) = matching_name.and_then(|name| {
        monitors
            .iter()
            .find(|monitor| monitor.name() == name)
            .map(SafeMonitor::id)
    }) else {
        return context.clone();
    };
    let mut resolved = context.clone();
    resolved.monitor_id = Some(monitor_id);
    resolved
}

#[cfg(target_os = "linux")]
fn point_in_logical_output(x: i32, y: i32, left: i32, top: i32, width: u32, height: u32) -> bool {
    let x = i64::from(x);
    let y = i64::from(y);
    let left = i64::from(left);
    let top = i64::from(top);
    x >= left && x < left + i64::from(width) && y >= top && y < top + i64::from(height)
}

impl Default for DystilFullCaptureVisualProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VisualProvider for DystilFullCaptureVisualProvider {
    async fn capture_all(
        &self,
        request: &VisualRequest,
    ) -> Result<Vec<VisualSnapshot>, CaptureError> {
        let monitors = self.monitors_for(request).await?;
        let attempted = monitors.len();
        let mut snapshots = Vec::with_capacity(attempted);
        let mut failures = Vec::new();

        // Capture sequentially so Windows never initializes multiple WGC
        // sessions concurrently and Linux avoids a burst of portal/xcap work.
        for monitor in monitors {
            let monitor_id = monitor.id();
            let device_name = monitor.name().to_string();
            match capture_monitor_image(&monitor).await {
                Ok(image) => {
                    info!(
                        monitor_id,
                        device_name,
                        captured_width = image.width(),
                        captured_height = image.height(),
                        "FullCapture acquired Linux/Windows screenshot"
                    );
                    snapshots.push(VisualSnapshot {
                        captured_at: Utc::now(),
                        image: Arc::new(image),
                        monitor_id: Some(monitor_id),
                        device_name: Some(device_name),
                    });
                }
                Err(error) => {
                    warn!(
                        monitor_id,
                        device_name,
                        %error,
                        "FullCapture failed to capture one Linux/Windows monitor"
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
                "FullCapture completed with partial Linux/Windows monitor coverage"
            );
        }
        Ok(snapshots)
    }

    async fn stop(&self) -> Result<(), CaptureError> {
        let mut monitors = self.monitors.lock().await;
        for monitor in monitors.values() {
            monitor.release_capture_stream();
        }
        monitors.clear();
        Ok(())
    }
}

#[cfg(target_os = "windows")]
async fn capture_monitor_image(monitor: &SafeMonitor) -> Result<DynamicImage, CaptureError> {
    monitor.capture_image().await.map_err(|error| {
        CaptureError::Visual(format!(
            "failed to capture display {}: {error}",
            monitor.id()
        ))
    })
}

#[cfg(target_os = "linux")]
async fn capture_monitor_image(monitor: &SafeMonitor) -> Result<DynamicImage, CaptureError> {
    let output_name = monitor.name().to_string();
    let wayland_name = output_name.clone();
    let wayland_capture =
        tokio::task::spawn_blocking(move || capture_wayland_output(&wayland_name)).await;

    match wayland_capture {
        Ok(Ok(image)) => {
            debug!(
                monitor_id = monitor.id(),
                output_name,
                captured_width = image.width(),
                captured_height = image.height(),
                "FullCapture used identity-based Wayland output capture"
            );
            Ok(image)
        }
        Ok(Err(error)) => {
            debug!(
                monitor_id = monitor.id(),
                output_name,
                %error,
                "identity-based Wayland capture unavailable; falling back to xcap"
            );
            monitor.capture_image().await.map_err(|fallback_error| {
                CaptureError::Visual(format!(
                    "Wayland output capture failed ({error}); xcap fallback failed: {fallback_error}"
                ))
            })
        }
        Err(error) => {
            debug!(
                monitor_id = monitor.id(),
                output_name,
                %error,
                "identity-based Wayland capture task failed; falling back to xcap"
            );
            monitor.capture_image().await.map_err(|fallback_error| {
                CaptureError::Visual(format!(
                    "Wayland output capture task failed ({error}); xcap fallback failed: {fallback_error}"
                ))
            })
        }
    }
}

#[cfg(target_os = "linux")]
fn capture_wayland_output(output_name: &str) -> Result<DynamicImage, String> {
    let connection =
        libwayshot_xcap::WayshotConnection::new().map_err(|error| error.to_string())?;
    let output = connection
        .get_all_outputs()
        .iter()
        .find(|output| output.name == output_name)
        .ok_or_else(|| format!("Wayland output {output_name:?} was not found"))?;
    let image = connection
        .screenshot_single_output(output, false)
        .map_err(|error| error.to_string())?
        .to_rgba8();
    let width = image.width();
    let height = image.height();
    let raw = image.into_vec();
    let image = image::RgbaImage::from_raw(width, height, raw).ok_or_else(|| {
        format!("invalid RGBA buffer returned for Wayland output {output_name:?}")
    })?;
    Ok(DynamicImage::ImageRgba8(image))
}

#[cfg(all(test, target_os = "linux"))]
mod live_wayland_tests {
    use super::*;

    #[test]
    fn logical_output_matching_supports_negative_origins() {
        assert!(point_in_logical_output(-1_425, 414, -1_600, 0, 1_600, 900));
        assert!(!point_in_logical_output(-1_425, 414, 0, 0, 1_600, 900));
    }

    /// Native smoke test for wlroots compositors. It is ignored in ordinary
    /// test runs because it requires a live graphical session.
    #[test]
    #[ignore = "requires a live wlroots Wayland session"]
    fn captures_each_wayland_output_by_identity_at_native_size() {
        let connection = libwayshot_xcap::WayshotConnection::new().unwrap();
        let outputs = connection.get_all_outputs();
        assert!(!outputs.is_empty());

        for output in outputs {
            let image = capture_wayland_output(&output.name).unwrap();
            eprintln!(
                "{}: {}x{} (expected physical {}x{})",
                output.name,
                image.width(),
                image.height(),
                output.physical_size.width,
                output.physical_size.height
            );
            assert_eq!(image.width(), output.physical_size.width);
            assert_eq!(image.height(), output.physical_size.height);
        }
    }
}
