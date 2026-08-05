//! Persistent Windows Graphics Capture session for monitor screenshots.

use anyhow::{anyhow, Result};
use image::{DynamicImage, RgbaImage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use xcap::Frame;
use xcap::Monitor as XcapMonitor;

/// Keeps one GraphicsCaptureSession alive, avoiding the orange border flash
/// caused by creating and destroying a session for every screenshot.
pub struct PersistentCapture {
    recorder: xcap::VideoRecorder,
    latest_frame: Arc<Mutex<Option<Frame>>>,
    consumer_handle: Option<JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    consumer_alive: Arc<AtomicBool>,
}

impl PersistentCapture {
    pub fn new(monitor_id: u32) -> Result<Self> {
        let monitors =
            XcapMonitor::all().map_err(|error| anyhow!("failed to list monitors: {error}"))?;
        let monitor = monitors
            .into_iter()
            .find(|monitor| monitor.id().unwrap_or(0) == monitor_id)
            .ok_or_else(|| anyhow!("monitor {monitor_id} not found for persistent capture"))?;

        let (recorder, receiver) = monitor
            .video_recorder()
            .map_err(|error| anyhow!("failed to create video recorder: {error}"))?;
        recorder
            .start()
            .map_err(|error| anyhow!("failed to start video recorder: {error}"))?;

        let latest_frame = Arc::new(Mutex::new(None));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let consumer_alive = Arc::new(AtomicBool::new(true));
        let frame_ref = Arc::clone(&latest_frame);
        let flag_ref = Arc::clone(&stop_flag);
        let alive_ref = Arc::clone(&consumer_alive);
        let consumer_handle = std::thread::Builder::new()
            .name(format!("wgc-consumer-{monitor_id}"))
            .spawn(move || {
                Self::consumer_loop(receiver, frame_ref, flag_ref);
                alive_ref.store(false, Ordering::Release);
            })
            .map_err(|error| anyhow!("failed to spawn consumer thread: {error}"))?;

        tracing::info!(monitor_id, "persistent WGC capture started");
        Ok(Self {
            recorder,
            latest_frame,
            consumer_handle: Some(consumer_handle),
            stop_flag,
            consumer_alive,
        })
    }

    fn consumer_loop(
        receiver: Receiver<Frame>,
        latest_frame: Arc<Mutex<Option<Frame>>>,
        stop_flag: Arc<AtomicBool>,
    ) {
        loop {
            if stop_flag.load(Ordering::Relaxed) {
                break;
            }
            match receiver.recv_timeout(Duration::from_millis(500)) {
                Ok(frame) => match latest_frame.lock() {
                    Ok(mut slot) => *slot = Some(frame),
                    Err(_) => {
                        tracing::error!("WGC consumer frame mutex poisoned; exiting");
                        break;
                    }
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    tracing::debug!("WGC consumer channel disconnected; exiting");
                    break;
                }
            }
        }
    }

    pub fn get_latest_image(&self, timeout: Duration) -> Result<DynamicImage> {
        let deadline = Instant::now() + timeout;
        loop {
            if !self.consumer_alive.load(Ordering::Acquire) {
                return Err(anyhow!("WGC session dead (consumer exited)"));
            }
            {
                let slot = self
                    .latest_frame
                    .lock()
                    .map_err(|error| anyhow!("frame mutex poisoned: {error}"))?;
                if let Some(frame) = slot.as_ref() {
                    return Self::frame_to_image(frame);
                }
            }
            if Instant::now() >= deadline {
                return Err(anyhow!("no frame received within {timeout:?}"));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn frame_to_image(frame: &Frame) -> Result<DynamicImage> {
        let image =
            RgbaImage::from_raw(frame.width, frame.height, frame.raw.clone()).ok_or_else(|| {
                anyhow!(
                    "failed to create RgbaImage from frame {}x{}",
                    frame.width,
                    frame.height
                )
            })?;
        Ok(DynamicImage::ImageRgba8(image))
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        if let Err(error) = self.recorder.stop() {
            tracing::warn!(%error, "failed to stop WGC recorder");
        }
        if let Some(handle) = self.consumer_handle.take() {
            if let Err(error) = handle.join() {
                tracing::warn!(?error, "WGC consumer thread panicked");
            }
        }
        tracing::debug!("persistent WGC capture stopped");
    }
}

impl Drop for PersistentCapture {
    fn drop(&mut self) {
        self.stop();
    }
}
