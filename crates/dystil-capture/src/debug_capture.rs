//! Explicitly enabled, local-only capture diagnostics for the standalone harness.
//!
//! This module is excluded unless the `debug-capture` feature is selected. It
//! never uploads data and has no production initializer.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use sysinfo::{ProcessExt, SystemExt};

use crate::a11y::{EventData, UiEvent};
use crate::{AccessibilitySnapshot, CaptureContext, CaptureTrigger};

#[cfg(feature = "debug-capture")]
use std::sync::OnceLock;

pub const DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;

static ACTIVE: Lazy<RwLock<Option<Arc<DiagnosticSink>>>> = Lazy::new(|| RwLock::new(None));

#[derive(Debug, Clone)]
pub struct DebugCaptureConfig {
    pub run_dir: PathBuf,
    pub run_id: String,
    pub policy: String,
    pub measurement_mode: String,
    pub baseline_frame_id: i64,
    pub baseline_event_id: i64,
}

pub struct DebugCaptureSession {
    sink: Arc<DiagnosticSink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEnvelope {
    pub schema_version: u32,
    pub run_id: String,
    pub policy: String,
    pub measurement_mode: String,
    pub sequence: u64,
    pub timestamp: DateTime<Utc>,
    pub monotonic_ms: u64,
    #[serde(flatten)]
    pub payload: Value,
}

struct Writers {
    events: BufWriter<File>,
    captures: BufWriter<File>,
    process: BufWriter<File>,
}

struct DiagnosticSink {
    run_id: String,
    policy: String,
    measurement_mode: String,
    started: Instant,
    sequence: AtomicU64,
    capture_sequence: AtomicU64,
    baseline_frame_id: i64,
    returned_frames: Mutex<HashSet<i64>>,
    writers: Mutex<Writers>,
    markers_path: PathBuf,
}

impl DebugCaptureSession {
    pub fn start(config: DebugCaptureConfig) -> Result<Self, String> {
        fs::create_dir_all(&config.run_dir).map_err(|error| error.to_string())?;
        let manifest = json!({
            "schema_version": DIAGNOSTIC_SCHEMA_VERSION,
            "run_id": config.run_id,
            "policy": config.policy,
            "measurement_mode": config.measurement_mode,
            "created_at": Utc::now(),
            "remote_writes": false,
            "uploads": false
            ,"baseline_frame_id": config.baseline_frame_id
            ,"baseline_event_id": config.baseline_event_id
        });
        fs::write(
            config.run_dir.join("run.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        let markers_path = config.run_dir.join("markers.jsonl");
        File::create(&markers_path).map_err(|error| error.to_string())?;
        let sink = Arc::new(DiagnosticSink {
            run_id: config.run_id,
            policy: config.policy,
            measurement_mode: config.measurement_mode,
            started: Instant::now(),
            sequence: AtomicU64::new(1),
            capture_sequence: AtomicU64::new(1),
            baseline_frame_id: config.baseline_frame_id,
            returned_frames: Mutex::new(HashSet::new()),
            writers: Mutex::new(Writers {
                events: writer(&config.run_dir, "events.jsonl")?,
                captures: writer(&config.run_dir, "captures.jsonl")?,
                process: writer(&config.run_dir, "process.jsonl")?,
            }),
            markers_path,
        });
        let mut active = ACTIVE.write().map_err(|_| "diagnostic lock poisoned")?;
        if active.is_some() {
            return Err("a debug capture session is already active".to_string());
        }
        *active = Some(Arc::clone(&sink));
        drop(active);
        sink.write(
            Stream::Marker,
            json!({
                "kind": "run_boundary",
                "phase": "start",
                "label": "capture_started"
            }),
        );
        Ok(Self { sink })
    }
}

impl Drop for DebugCaptureSession {
    fn drop(&mut self) {
        self.sink.write(
            Stream::Marker,
            json!({
                "kind": "run_boundary",
                "phase": "end",
                "label": "capture_stopped"
            }),
        );
        if let Ok(mut active) = ACTIVE.write() {
            if active
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &self.sink))
            {
                *active = None;
            }
        }
    }
}

fn writer(run_dir: &Path, name: &str) -> Result<BufWriter<File>, String> {
    File::create(run_dir.join(name))
        .map(BufWriter::new)
        .map_err(|error| error.to_string())
}

#[derive(Clone, Copy)]
enum Stream {
    Event,
    Capture,
    Process,
    Marker,
}

impl DiagnosticSink {
    fn envelope(&self, payload: Value) -> DiagnosticEnvelope {
        DiagnosticEnvelope {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            run_id: self.run_id.clone(),
            policy: self.policy.clone(),
            measurement_mode: self.measurement_mode.clone(),
            sequence: self.sequence.fetch_add(1, Ordering::Relaxed),
            timestamp: Utc::now(),
            monotonic_ms: self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
            payload,
        }
    }

    fn write(&self, stream: Stream, payload: Value) {
        let envelope = self.envelope(payload);
        if matches!(stream, Stream::Marker) {
            if let Ok(mut writer) = File::options()
                .create(true)
                .append(true)
                .open(&self.markers_path)
            {
                if serde_json::to_writer(&mut writer, &envelope).is_ok() {
                    let _ = writer.write_all(b"\n");
                    let _ = writer.flush();
                }
            }
            return;
        }
        let Ok(mut writers) = self.writers.lock() else {
            return;
        };
        let writer = match stream {
            Stream::Event => &mut writers.events,
            Stream::Capture => &mut writers.captures,
            Stream::Process => &mut writers.process,
            Stream::Marker => unreachable!(),
        };
        if serde_json::to_writer(&mut *writer, &envelope).is_ok() {
            let _ = writer.write_all(b"\n");
            let _ = writer.flush();
        }
    }

    fn next_capture_id(&self) -> u64 {
        self.capture_sequence.fetch_add(1, Ordering::Relaxed)
    }

    fn frame_outcome(&self, frame_id: i64) -> &'static str {
        let Ok(mut returned) = self.returned_frames.lock() else {
            return "unknown";
        };
        let first_return = returned.insert(frame_id);
        if frame_id > self.baseline_frame_id && first_return {
            "persisted"
        } else {
            "reused"
        }
    }
}

fn active() -> Option<Arc<DiagnosticSink>> {
    ACTIVE.read().ok()?.clone()
}

pub fn record_ui_event(event: &UiEvent, persist_candidate: bool, trigger_candidate: bool) {
    let Some(sink) = active() else {
        return;
    };
    let (event_type, source, logical_action_id, details) = match &event.data {
        EventData::Click {
            x,
            y,
            button,
            click_count,
            modifiers,
        } => {
            let source = if *click_count == 0 {
                "element_enrichment"
            } else {
                "physical_click"
            };
            (
                "click",
                source,
                Some(format!(
                    "click:{}:{x}:{y}:{button}",
                    event.timestamp.timestamp_micros()
                )),
                json!({
                    "x": x,
                    "y": y,
                    "button": button,
                    "click_count": click_count,
                    "modifiers": modifiers
                }),
            )
        }
        EventData::Move { x, y } => ("move", "native", None, json!({"x": x, "y": y})),
        EventData::Scroll {
            x,
            y,
            delta_x,
            delta_y,
        } => (
            "scroll",
            "native",
            None,
            json!({"x": x, "y": y, "delta_x": delta_x, "delta_y": delta_y}),
        ),
        EventData::Key {
            key_code,
            modifiers,
        } => (
            "key",
            "native",
            None,
            json!({"key_code": key_code, "modifiers": modifiers}),
        ),
        EventData::Text { char_count, .. } => {
            ("text", "native", None, json!({"char_count": char_count}))
        }
        EventData::AppSwitch { name, pid } => (
            "app_switch",
            "native",
            None,
            json!({"name": name, "pid": pid}),
        ),
        EventData::WindowFocus { app, title } => (
            "window_focus",
            "native",
            None,
            json!({"app": app, "title": title}),
        ),
        EventData::Clipboard { operation, .. } => {
            ("clipboard", "native", None, json!({"operation": operation}))
        }
    };
    sink.write(
        Stream::Event,
        json!({
            "kind": "ui_event",
            "event_type": event_type,
            "source": source,
            "logical_action_id": logical_action_id,
            "native_timestamp": event.timestamp,
            "relative_ms": event.relative_ms,
            "persist_candidate": persist_candidate,
            "capture_trigger_candidate": trigger_candidate,
            "app_name": event.app_name,
            "window_title": event.window_title,
            "browser_url": event.browser_url,
            "has_element_context": event.element.is_some(),
            "details": details
        }),
    );
}

pub fn record_capture_request(
    trigger: &CaptureTrigger,
    context: &CaptureContext,
    correlation_count: usize,
    heartbeat: bool,
) -> Option<u64> {
    let sink = active()?;
    let capture_id = sink.next_capture_id();
    sink.write(
        Stream::Capture,
        json!({
            "kind": "capture_request",
            "capture_id": capture_id,
            "trigger": trigger.as_str(),
            "heartbeat": heartbeat,
            "correlation_count": correlation_count,
            "context": context
        }),
    );
    Some(capture_id)
}

pub fn record_capture_result(
    capture_id: Option<u64>,
    started: Instant,
    frame_id: Option<i64>,
    explicit_outcome: Option<&str>,
    error: Option<&str>,
) {
    let Some(sink) = active() else {
        return;
    };
    let outcome = explicit_outcome.unwrap_or_else(|| {
        frame_id
            .map(|id| sink.frame_outcome(id))
            .unwrap_or("no_frame")
    });
    sink.write(
        Stream::Capture,
        json!({
            "kind": "capture_result",
            "capture_id": capture_id,
            "duration_ms": millis(started.elapsed()),
            "outcome": outcome,
            "frame_id": frame_id,
            "error": error
        }),
    );
}

pub fn record_accessibility_attempt(
    trigger: &CaptureTrigger,
    started: Instant,
    snapshot: Option<&AccessibilitySnapshot>,
    outcome: &str,
    error: Option<&str>,
) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Capture,
        json!({
            "kind": "accessibility_attempt",
            "trigger": trigger.as_str(),
            "duration_ms": millis(started.elapsed()),
            "outcome": outcome,
            "app_name": snapshot.and_then(|value| value.context.application.as_deref()),
            "window_title": snapshot.and_then(|value| value.context.window.as_deref()),
            "browser_url": snapshot.and_then(|value| value.context.browser_url.as_deref()),
            "document_path": snapshot.and_then(|value| value.context.document_path.as_deref()),
            "node_count": snapshot.map(|value| value.node_count),
            "nodes_retained": snapshot.map(|value| value.nodes.len()),
            "max_depth_reached": snapshot.map(|value| value.max_depth_reached),
            "walk_duration_ms": snapshot.map(|value| value.walk_duration_ms),
            "truncated": snapshot.map(|value| value.truncated),
            "truncation_reason": snapshot.map(|value| value.truncation_reason),
            "content_hash": snapshot.map(|value| value.content_hash),
            "simhash": snapshot.map(|value| value.simhash),
            "text_bytes": snapshot.map(|value| value.text.len()),
            "error": error
        }),
    );
}

#[allow(clippy::too_many_arguments)]
pub fn record_background_tree_attempt(
    reason: &str,
    outcome: &str,
    started: Instant,
    app_name: Option<&str>,
    window_title: Option<&str>,
    node_count: Option<usize>,
    max_depth: Option<usize>,
    tree_hash: Option<u64>,
    truncation_reason: Option<&str>,
) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Capture,
        json!({
            "kind": "background_tree_attempt",
            "reason": reason,
            "outcome": outcome,
            "duration_ms": millis(started.elapsed()),
            "app_name": app_name,
            "window_title": window_title,
            "node_count": node_count,
            "max_depth_reached": max_depth,
            "tree_hash": tree_hash,
            "truncation_reason": truncation_reason
        }),
    );
}

pub fn record_process_sample(cpu_percent: f32, rss_bytes: u64, database_bytes: u64) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Process,
        json!({
            "kind": "process_sample",
            "cpu_percent": cpu_percent,
            "rss_bytes": rss_bytes,
            "database_bytes": database_bytes
        }),
    );
}

/// Records the work performed by the hook thread for one non-timer Windows
/// message. This is local harness instrumentation only; it lets the fixture
/// distinguish a low CPU average from a foreground input stall.
pub fn record_message_pump_sample(message: u32, duration: Duration) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Process,
        json!({
            "kind": "message_pump_sample",
            "message": message,
            "duration_us": duration.as_micros().min(u64::MAX as u128) as u64
        }),
    );
}

/// Return the capture process's current resident memory for phase attribution.
/// This is intentionally available only to the local debug session; production
/// capture has no phase sampler and no extra memory query.
pub fn process_rss_bytes() -> Option<u64> {
    static SYSTEM: OnceLock<Mutex<sysinfo::System>> = OnceLock::new();
    static PID: OnceLock<Option<sysinfo::Pid>> = OnceLock::new();
    let system = SYSTEM.get_or_init(|| Mutex::new(sysinfo::System::new()));
    let pid = (*PID.get_or_init(|| sysinfo::get_current_pid().ok())).as_ref()?;
    let mut system = system.lock().ok()?;
    system.refresh_process(*pid);
    system.process(*pid).map(|process| process.memory())
}

#[allow(clippy::too_many_arguments)]
pub fn record_capture_phase(
    phase: &str,
    trigger: &str,
    started: Instant,
    app_name: Option<&str>,
    node_count: Option<usize>,
    text_bytes: Option<usize>,
    truncated: Option<bool>,
    truncation_reason: Option<&str>,
    rss_before: Option<u64>,
    rss_after: Option<u64>,
) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Capture,
        json!({
            "kind": "capture_phase",
            "phase": phase,
            "trigger": trigger,
            "duration_us": started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            "app_name": app_name,
            "node_count": node_count,
            "text_bytes": text_bytes,
            "truncated": truncated,
            "truncation_reason": truncation_reason,
            "rss_before_bytes": rss_before,
            "rss_after_bytes": rss_after,
            "rss_delta_bytes": rss_after.zip(rss_before).map(|(after, before)| after as i128 - before as i128)
        }),
    );
}

pub fn record_persistence(
    frame_id: i64,
    trigger: &str,
    started: Instant,
    normalization_duration: Duration,
    text_bytes: usize,
    snapshot_bytes: u64,
) {
    let Some(sink) = active() else {
        return;
    };
    sink.write(
        Stream::Capture,
        json!({
            "kind": "persistence_result",
            "frame_id": frame_id,
            "trigger": trigger,
            "outcome": "persisted",
            "duration_ms": millis(started.elapsed()),
            "normalization_duration_ms": millis(normalization_duration),
            "text_bytes": text_bytes,
            "snapshot_bytes": snapshot_bytes
        }),
    );
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(u64::MAX as u128) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_schema_is_stable_and_flattened() {
        let envelope = DiagnosticEnvelope {
            schema_version: DIAGNOSTIC_SCHEMA_VERSION,
            run_id: "run-1".to_string(),
            policy: "baseline".to_string(),
            measurement_mode: "baseline".to_string(),
            sequence: 7,
            timestamp: DateTime::parse_from_rfc3339("2026-08-10T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            monotonic_ms: 42,
            payload: json!({"kind": "process_sample", "rss_bytes": 100}),
        };
        let value = serde_json::to_value(envelope).unwrap();
        assert_eq!(value["schema_version"], 1);
        assert_eq!(value["kind"], "process_sample");
        assert_eq!(value["rss_bytes"], 100);
        assert!(value.get("payload").is_none());
    }

    #[test]
    fn click_enrichment_uses_the_physical_click_logical_identity() {
        let timestamp = DateTime::parse_from_rfc3339("2026-08-10T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let identity = |click_count| {
            let event = UiEvent {
                id: None,
                timestamp,
                relative_ms: 1,
                data: EventData::Click {
                    x: 10,
                    y: 20,
                    button: 0,
                    click_count,
                    modifiers: 0,
                },
                app_name: None,
                window_title: None,
                browser_url: None,
                element: None,
                frame_id: None,
            };
            match event.data {
                EventData::Click { x, y, button, .. } => format!(
                    "click:{}:{x}:{y}:{button}",
                    event.timestamp.timestamp_micros()
                ),
                _ => unreachable!(),
            }
        };
        assert_eq!(identity(1), identity(0));
    }
}
