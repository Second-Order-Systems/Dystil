use crate::stop_capture;
use crate::tray::QUIT_REQUESTED;
use crate::RecordingState;
use log::{debug, error, info, warn};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;
use tokio::time::interval;

const AUTO_UPDATE_GATE_TIMEOUT: Duration = Duration::from_secs(5 * 60);

async fn await_restart_gate(timeout: Duration, label: &str) -> bool {
    let outcome = crate::health::wait_for_boot_ready(timeout).await;
    match outcome {
        crate::health::BootReadiness::Ready => true,
        crate::health::BootReadiness::Errored => {
            warn!("{}: boot phase is 'error' — deferring restart", label);
            false
        }
        crate::health::BootReadiness::Pending => {
            warn!(
                "{}: boot phase still pending after {}s — deferring restart",
                label,
                timeout.as_secs()
            );
            false
        }
    }
}

async fn check_for_updates(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    info!("checking for updates...");

    let check_result = app.updater_builder().build()?.check().await;

    match check_result {
        Ok(Some(update)) => {
            info!("update found: v{}", update.version);

            let retry_delays = [
                Duration::from_secs(30),
                Duration::from_secs(120),
                Duration::from_secs(300),
            ];
            let mut attempt = 0;
            loop {
                let result = update.download_and_install(|_, _| {}, || {}).await;
                match result {
                    Ok(()) => break,
                    Err(e) => {
                        let next_delay = retry_delays.get(attempt).copied();
                        if next_delay.is_none() {
                            error!(
                                "update download failed after {} attempts: {}",
                                attempt + 1,
                                e
                            );
                            return Err(e.into());
                        }
                        let delay = next_delay.unwrap();
                        warn!(
                            "download attempt {} failed: {} — retrying in {}s",
                            attempt + 1,
                            e,
                            delay.as_secs()
                        );
                        tokio::time::sleep(delay).await;
                        attempt += 1;
                    }
                }
            }

            info!("update downloaded, preparing restart...");

            if !await_restart_gate(AUTO_UPDATE_GATE_TIMEOUT, "auto-update").await {
                warn!("auto-update: restart gate did not proceed, deferring");
                return Ok(());
            }

            QUIT_REQUESTED.store(true, Ordering::SeqCst);

            let _ = tokio::time::timeout(
                Duration::from_secs(15),
                stop_capture(app.state::<RecordingState>(), app.clone()),
            )
            .await;

            app.restart();
        }
        Ok(None) => {
            debug!("no update available");
            Ok(())
        }
        Err(e) => {
            warn!("update check failed: {}", e);
            Err(e.into())
        }
    }
}

pub fn start_update_check(app: &tauri::AppHandle, interval_minutes: u64) {
    let app_handle = app.clone();

    tokio::spawn(async move {
        if let Err(e) = check_for_updates(&app_handle).await {
            warn!("boot update check failed: {}", e);
        }
    });

    let app_handle = app.clone();
    tokio::spawn(async move {
        let check_interval = Duration::from_secs(interval_minutes * 60);
        let mut interval = interval(check_interval);
        interval.tick().await;
        loop {
            interval.tick().await;
            if let Err(e) = check_for_updates(&app_handle).await {
                warn!("periodic update check failed: {}", e);
            }
        }
    });
}
