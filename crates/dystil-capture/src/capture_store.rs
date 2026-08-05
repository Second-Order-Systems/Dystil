use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage};
use sqlx::SqlitePool;

use crate::semantic_tree::{SampleDecision, SemanticSampleCandidate, SemanticTreeStore};
use crate::{AccessibilityNode, CaptureError, CaptureObservation, CaptureStore, StoredCapture};

#[derive(serde::Serialize)]
struct AxCaptureDiagnostics {
    node_count: usize,
    walk_duration_ms: u64,
    truncated: bool,
    truncation_reason: crate::AccessibilityTruncationReason,
    max_depth_reached: usize,
}

const SNAPSHOT_QUALITY: u8 = 80;
const SNAPSHOT_MAX_WIDTH: u32 = 1920;

/// Dystil-owned persistence for capture observations.
///
/// The surrounding app still owns database startup and migrations during the
/// vendor lift. This store only needs an already-open pool and writes the
/// existing `frames` schema directly; it deliberately has no DatabaseManager,
/// SnapshotWriter, or Dystil PII-removal dependency.
pub struct DystilCaptureStore {
    pool: SqlitePool,
    snapshot_writer: DystilSnapshotWriter,
    default_device_name: String,
    queue_ai_redaction: bool,
    semantic_tree_store: Option<SemanticTreeStore>,
    semantic_sample_permit: std::sync::Arc<tokio::sync::Semaphore>,
}

impl DystilCaptureStore {
    pub fn new(
        pool: SqlitePool,
        snapshots_root: impl Into<PathBuf>,
        default_device_name: impl Into<String>,
        queue_ai_redaction: bool,
    ) -> Self {
        Self {
            pool,
            snapshot_writer: DystilSnapshotWriter::new(snapshots_root),
            default_device_name: default_device_name.into(),
            queue_ai_redaction,
            semantic_tree_store: None,
            semantic_sample_permit: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    pub fn with_semantic_samples_best_effort(
        mut self,
        database_path: impl AsRef<std::path::Path>,
    ) -> Self {
        match SemanticTreeStore::open(database_path) {
            Ok(store) => {
                self.semantic_tree_store = Some(store);
            }
            Err(error) => tracing::warn!(
                %error,
                "semantic sample store unavailable; ordinary capture will continue"
            ),
        }
        self
    }
}

#[async_trait]
impl CaptureStore for DystilCaptureStore {
    async fn persist(
        &self,
        observation: CaptureObservation,
    ) -> Result<StoredCapture, CaptureError> {
        let snapshot_path = match observation.visual.as_ref() {
            Some(visual) => {
                let writer = self.snapshot_writer.clone();
                let image = visual.image.clone();
                let captured_at = visual.captured_at;
                let monitor_id = visual
                    .monitor_id
                    .or(observation.context.monitor_id)
                    .unwrap_or(0);
                tokio::task::spawn_blocking(move || writer.write(&image, captured_at, monitor_id))
                    .await
                    .map_err(|error| CaptureError::ImageStore(error.to_string()))?
                    .map_err(CaptureError::ImageStore)?
                    .to_string_lossy()
                    .into_owned()
            }
            None => String::new(),
        };

        let accessibility = observation.accessibility.as_ref();
        let accessibility_text = accessibility
            .map(|snapshot| snapshot.text.trim())
            .filter(|text| !text.is_empty())
            .map(sanitize_text);
        let sanitized_nodes = accessibility.map(|snapshot| sanitize_nodes(&snapshot.nodes));
        let ax_capture_diagnostics_json = accessibility
            .map(|snapshot| AxCaptureDiagnostics {
                node_count: snapshot.node_count,
                walk_duration_ms: snapshot.walk_duration_ms,
                truncated: snapshot.truncated,
                truncation_reason: snapshot.truncation_reason,
                max_depth_reached: snapshot.max_depth_reached,
            })
            .map(|diagnostics| serde_json::to_string(&diagnostics))
            .transpose()
            .map_err(|error| CaptureError::Store(error.to_string()))?;
        let text_source = accessibility_text.as_ref().map(|_| "accessibility");
        let content_hash = accessibility.map(|snapshot| snapshot.content_hash as i64);
        let simhash = accessibility.map(|snapshot| snapshot.simhash as i64);
        let device_name = sanitize_text(
            observation
                .context
                .device_name
                .as_deref()
                .unwrap_or(&self.default_device_name),
        );
        let app_name = observation
            .context
            .application
            .as_deref()
            .map(sanitize_text);
        let window_name = observation.context.window.as_deref().map(sanitize_text);
        let browser_url = observation
            .context
            .browser_url
            .as_deref()
            .map(sanitize_text);
        let document_path = observation
            .context
            .document_path
            .as_deref()
            .map(sanitize_text);

        // Keep the values aligned with DatabaseManager::insert_snapshot_frame_with_ocr
        // for the AX-only Dystil path. The existing frames triggers maintain FTS.
        let result = sqlx::query(
            "INSERT INTO frames (\
                timestamp, device_name, snapshot_path, app_name, window_name, browser_url, \
                document_path, focused, capture_trigger, frame_text, text_source, \
                accessibility_tree_json, ax_capture_diagnostics_json, content_hash, simhash\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(observation.captured_at.to_rfc3339())
        .bind(device_name)
        .bind(&snapshot_path)
        .bind(&app_name)
        .bind(&window_name)
        .bind(&browser_url)
        .bind(&document_path)
        .bind(observation.context.focused.unwrap_or(true))
        .bind(observation.trigger.as_str())
        .bind(accessibility_text.as_deref())
        .bind(text_source)
        // Full trees are sampled into the separate bounded semantic store.
        // Keeping this legacy frame column null prevents per-frame growth.
        .bind(Option::<String>::None)
        .bind(ax_capture_diagnostics_json)
        .bind(content_hash)
        .bind(simhash)
        .execute(&self.pool)
        .await
        .map_err(|error| CaptureError::Store(error.to_string()))?;

        let frame_id = result.last_insert_rowid();

        if let (Some(store), Some(nodes), Some(app_name)) = (
            self.semantic_tree_store.clone(),
            sanitized_nodes,
            app_name.clone(),
        ) {
            let captured_at = observation.captured_at;
            let window_name = window_name.clone();
            let browser_url = browser_url.clone();
            if let Ok(permit) = self.semantic_sample_permit.clone().try_acquire_owned() {
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    let decision = store.record(SemanticSampleCandidate {
                        source_frame_id: frame_id,
                        captured_at,
                        platform: std::env::consts::OS,
                        app_name: &app_name,
                        // This is the captured application's version, not the
                        // Dystil client version. Capture context does not expose
                        // it yet, so preserve the contract by leaving it null.
                        app_version: None,
                        window_name: window_name.as_deref(),
                        browser_url: browser_url.as_deref(),
                        nodes: &nodes,
                    });
                    match decision {
                        Ok(SampleDecision::Stored {
                            sample_id,
                            compressed_bytes,
                        }) => tracing::debug!(
                            frame_id,
                            sample_id,
                            compressed_bytes,
                            "stored semantic tree sample"
                        ),
                        Ok(decision) => tracing::trace!(
                            frame_id,
                            ?decision,
                            "semantic tree sample not retained"
                        ),
                        Err(error) => tracing::warn!(
                            frame_id,
                            %error,
                            "semantic tree sampling failed without affecting frame capture"
                        ),
                    }
                });
            } else {
                tracing::trace!(
                    frame_id,
                    "semantic sampler busy; dropping candidate without affecting frame capture"
                );
            }
        }
        // Deterministic redaction has already happened in this transaction.
        // Queue the optional model pass only while the user has opted in; an
        // opt-out must not accumulate a surprise historical backlog.
        if self.queue_ai_redaction {
            let state_pool = self.pool.clone();
            tokio::spawn(async move {
                let _ = dystil_redact::record_state(
                    &state_pool,
                    "frames",
                    frame_id,
                    "frame_text",
                    dystil_redact::RedactionStatus::Pending,
                    0,
                    None,
                    None,
                )
                .await;
            });
        }
        Ok(StoredCapture {
            // SQLite returns the row ID from the connection used by this exact
            // statement, so this remains correct with a pooled connection.
            frame_id,
            snapshot_path: (!snapshot_path.is_empty()).then_some(snapshot_path),
        })
    }
}

#[derive(Clone)]
struct DystilSnapshotWriter {
    base_dir: PathBuf,
}

impl DystilSnapshotWriter {
    fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    fn write(
        &self,
        image: &DynamicImage,
        captured_at: DateTime<Utc>,
        monitor_id: u32,
    ) -> Result<PathBuf, String> {
        let date_dir = self
            .base_dir
            .join(captured_at.format("%Y-%m-%d").to_string());
        fs::create_dir_all(&date_dir).map_err(|error| error.to_string())?;

        let path = date_dir.join(format!(
            "{}_m{}.jpg",
            captured_at.timestamp_millis(),
            monitor_id
        ));
        let resized;
        let image = if image.width() > SNAPSHOT_MAX_WIDTH {
            resized = image.resize(SNAPSHOT_MAX_WIDTH, u32::MAX, FilterType::Triangle);
            &resized
        } else {
            image
        };

        let file = fs::File::create(&path).map_err(|error| error.to_string())?;
        let mut writer = BufWriter::new(file);
        let mut encoder = JpegEncoder::new_with_quality(&mut writer, SNAPSHOT_QUALITY);
        encoder
            .encode_image(image)
            .map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
        writer
            .get_ref()
            .sync_all()
            .map_err(|error| error.to_string())?;
        Ok(path)
    }
}

// Keep the adapter's call sites stable while the legacy implementation is
// removed with the Dystil adapter. New persistence always uses the owned
// Dystil redactor crate.
pub(crate) fn sanitize_text(text: &str) -> String {
    dystil_redact::sanitize_text(text)
}

fn sanitize_nodes(nodes: &[AccessibilityNode]) -> Vec<AccessibilityNode> {
    nodes
        .iter()
        .cloned()
        .map(|mut node| {
            node.role = sanitize_text(&node.role);
            node.text = sanitize_text(&node.text);
            for value in [
                &mut node.automation_id,
                &mut node.class_name,
                &mut node.value,
                &mut node.help_text,
                &mut node.url,
                &mut node.placeholder,
                &mut node.role_description,
                &mut node.subrole,
                &mut node.dom_identifier,
                &mut node.dom_classes,
                &mut node.accelerator_key,
                &mut node.access_key,
            ] {
                if let Some(value) = value.as_mut() {
                    *value = sanitize_text(value);
                }
            }
            node
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use image::{DynamicImage, GenericImageView};
    use sqlx::{sqlite::SqlitePoolOptions, Row};
    use std::{path::Path, sync::Arc};
    use tempfile::TempDir;

    use super::*;
    use crate::{
        AccessibilityNode, AccessibilitySnapshot, AccessibilityTruncationReason, CaptureContext,
        CaptureObservation, CaptureTrigger, VisualSnapshot,
    };

    async fn test_store(temp: &TempDir) -> (SqlitePool, DystilCaptureStore) {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE frames (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, \
                device_name TEXT NOT NULL DEFAULT '', snapshot_path TEXT, app_name TEXT, \
                window_name TEXT, browser_url TEXT, document_path TEXT, focused BOOLEAN, \
                capture_trigger TEXT, frame_text TEXT, text_source TEXT, \
                accessibility_tree_json TEXT, ax_capture_diagnostics_json TEXT, \
                content_hash INTEGER, simhash INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = DystilCaptureStore::new(pool.clone(), temp.path(), "test_monitor", false);
        (pool, store)
    }

    fn accessibility(now: chrono::DateTime<Utc>) -> AccessibilitySnapshot {
        AccessibilitySnapshot {
            captured_at: now,
            context: CaptureContext::default(),
            text: "AX content".to_string(),
            nodes: vec![],
            node_count: 0,
            walk_duration_ms: 2,
            content_hash: 11,
            simhash: 22,
            truncated: false,
            truncation_reason: AccessibilityTruncationReason::None,
            max_depth_reached: 0,
        }
    }

    fn observation(visual: Option<VisualSnapshot>) -> CaptureObservation {
        let now = Utc::now();
        CaptureObservation {
            captured_at: now,
            trigger: CaptureTrigger::TypingPause,
            context: CaptureContext {
                application: Some("Code".to_string()),
                window: Some("matcher.rs".to_string()),
                monitor_id: Some(7),
                device_name: Some("display-7".to_string()),
                focused: Some(true),
                ..CaptureContext::default()
            },
            accessibility: Some(accessibility(now)),
            visual,
        }
    }

    #[tokio::test]
    async fn ax_only_frame_has_empty_path_and_sanitized_full_text() {
        let temp = TempDir::new().unwrap();
        let (pool, store) = test_store(&temp).await;
        let stored = store.persist(observation(None)).await.unwrap();
        assert_eq!(stored.snapshot_path, None);
        let row =
            sqlx::query("SELECT snapshot_path, frame_text, text_source FROM frames WHERE id = ?")
                .bind(stored.frame_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.get::<String, _>("snapshot_path"), "");
        assert_eq!(row.get::<String, _>("frame_text"), "AX content");
        assert_eq!(row.get::<String, _>("text_source"), "accessibility");
    }

    #[tokio::test]
    async fn stores_accessibility_diagnostics_as_json_and_leaves_non_ax_frames_null() {
        let temp = TempDir::new().unwrap();
        let (pool, store) = test_store(&temp).await;
        let mut with_ax = observation(None);
        let snapshot = with_ax.accessibility.as_mut().unwrap();
        snapshot.node_count = 1_842;
        snapshot.walk_duration_ms = 250;
        snapshot.truncated = true;
        snapshot.truncation_reason = AccessibilityTruncationReason::Timeout;
        snapshot.max_depth_reached = 27;

        let ax_id = store.persist(with_ax).await.unwrap().frame_id;
        let diagnostics: String =
            sqlx::query_scalar("SELECT ax_capture_diagnostics_json FROM frames WHERE id = ?")
                .bind(ax_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let diagnostics: serde_json::Value = serde_json::from_str(&diagnostics).unwrap();
        assert_eq!(diagnostics["node_count"], 1_842);
        assert_eq!(diagnostics["walk_duration_ms"], 250);
        assert_eq!(diagnostics["truncated"], true);
        assert_eq!(diagnostics["truncation_reason"], "timeout");
        assert_eq!(diagnostics["max_depth_reached"], 27);

        let mut without_ax = observation(None);
        without_ax.accessibility = None;
        let non_ax_id = store.persist(without_ax).await.unwrap().frame_id;
        let diagnostics: Option<String> =
            sqlx::query_scalar("SELECT ax_capture_diagnostics_json FROM frames WHERE id = ?")
                .bind(non_ax_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(diagnostics.is_none());
    }

    #[tokio::test]
    async fn visual_frames_write_separate_monitor_paths() {
        let temp = TempDir::new().unwrap();
        let (_pool, store) = test_store(&temp).await;
        for monitor_id in [7, 8] {
            let visual = VisualSnapshot {
                captured_at: Utc::now(),
                image: Arc::new(DynamicImage::new_rgb8(2, 2)),
                monitor_id: Some(monitor_id),
                device_name: None,
            };
            let path = store
                .persist(observation(Some(visual)))
                .await
                .unwrap()
                .snapshot_path
                .unwrap();
            assert!(Path::new(&path).is_file());
            assert!(path.ends_with(&format!("_m{monitor_id}.jpg")));
        }
    }

    #[tokio::test]
    async fn deterministic_redaction_covers_text_tree_and_metadata() {
        let temp = TempDir::new().unwrap();
        let (pool, store) = test_store(&temp).await;
        let mut value = observation(None);
        value.context.window = Some("person@example.com".to_string());
        value.accessibility.as_mut().unwrap().text = "contact person@example.com".to_string();
        value
            .accessibility
            .as_mut()
            .unwrap()
            .nodes
            .push(AccessibilityNode {
                node_id: 1,
                parent_node_id: None,
                role: "text".to_string(),
                text: "phone +1-234-567-8901".to_string(),
                depth: 0,
                bounds: None,
                on_screen: None,
                lines: None,
                automation_id: None,
                class_name: None,
                value: Some("AKIAIOSFODNN7EXAMPLE".to_string()),
                help_text: None,
                url: None,
                placeholder: None,
                role_description: None,
                subrole: None,
                dom_identifier: None,
                dom_classes: None,
                is_enabled: None,
                is_focused: None,
                is_selected: None,
                is_expanded: None,
                is_password: None,
                is_keyboard_focusable: None,
                accelerator_key: None,
                access_key: None,
            });
        let id = store.persist(value).await.unwrap().frame_id;
        let row = sqlx::query(
            "SELECT frame_text, accessibility_tree_json, window_name FROM frames WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
        for column in ["frame_text", "window_name"] {
            assert!(!row.get::<String, _>(column).contains("person@example.com"));
        }
        assert!(row
            .get::<Option<String>, _>("accessibility_tree_json")
            .is_none());
    }

    #[test]
    fn snapshot_downscales_proportionally_without_cropping_or_upscaling() {
        let temp = TempDir::new().unwrap();
        let writer = DystilSnapshotWriter::new(temp.path());
        let large = DynamicImage::new_rgb8(2560, 1440);
        let large_path = writer.write(&large, Utc::now(), 1).unwrap();
        assert_eq!(image::open(large_path).unwrap().dimensions(), (1920, 1080));
        let small = DynamicImage::new_rgb8(1000, 700);
        let small_path = writer
            .write(&small, Utc::now() + chrono::Duration::milliseconds(1), 2)
            .unwrap();
        assert_eq!(image::open(small_path).unwrap().dimensions(), (1000, 700));
    }
}
