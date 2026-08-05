use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use dystil_protocol::{DeviceCaptureState, UpdateDeviceCaptureStateRequest};
use reqwest::header::AUTHORIZATION;
use tauri::{AppHandle, Manager};
use tokio::sync::Notify;

use crate::recording::{persisted_pause_deadline, RecordingState};
use crate::store::SettingsStore;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(120);

static STARTED: AtomicBool = AtomicBool::new(false);
static WAKE: OnceLock<Arc<Notify>> = OnceLock::new();

fn wake() -> &'static Arc<Notify> {
    WAKE.get_or_init(|| Arc::new(Notify::new()))
}

pub(crate) fn start(app: AppHandle) {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let notify = wake().clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if let Err(error) = report_current(&app).await {
                tracing::warn!(%error, "capture state cloud reconciliation deferred");
            }
            tokio::select! {
                _ = tokio::time::sleep(RECONCILE_INTERVAL) => {}
                _ = notify.notified() => {}
            }
        }
    });
}

pub(crate) fn schedule() {
    wake().notify_one();
}

async fn report_current(app: &AppHandle) -> Result<(), String> {
    let settings = SettingsStore::get(app)?.unwrap_or_default();
    let payload = if settings.capture_paused {
        let deadline = persisted_pause_deadline(&settings)
            .ok_or_else(|| "paused capture is missing a valid deadline".to_string())?;
        UpdateDeviceCaptureStateRequest {
            capture_state: DeviceCaptureState::Paused,
            capture_pause_until: Some(deadline),
        }
    } else {
        let Some(state) = app.try_state::<RecordingState>() else {
            return Ok(());
        };
        if !state.capture_active.load(Ordering::SeqCst) {
            return Ok(());
        }
        UpdateDeviceCaptureStateRequest {
            capture_state: DeviceCaptureState::Recording,
            capture_pause_until: None,
        }
    };

    let Some(device_token) = crate::auth::current_device_token().await? else {
        return Ok(());
    };
    let cloud_base = crate::auth::cloud_base_url()?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .put(format!("{cloud_base}/devices/self/capture-state"))
        .header(AUTHORIZATION, format!("Device {device_token}"))
        .json(&payload)
        .send()
        .await
        .map_err(|error| error.to_string())?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "cloud capture-state update returned {status}: {body}"
        ));
    }

    Ok(())
}
