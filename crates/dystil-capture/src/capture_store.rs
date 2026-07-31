use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use image::{codecs::jpeg::JpegEncoder, imageops::FilterType, DynamicImage};
use sqlx::SqlitePool;

use crate::{AccessibilityNode, CaptureError, CaptureObservation, CaptureStore, StoredCapture};

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
}

impl DystilCaptureStore {
    pub fn new(
        pool: SqlitePool,
        snapshots_root: impl Into<PathBuf>,
        default_device_name: impl Into<String>,
    ) -> Self {
        Self {
            pool,
            snapshot_writer: DystilSnapshotWriter::new(snapshots_root),
            default_device_name: default_device_name.into(),
        }
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
        let accessibility_tree_json = sanitized_nodes
            .as_ref()
            .map(serde_json::to_string)
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
                accessibility_tree_json, content_hash, simhash, elements_ref_frame_id\
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NULL)",
        )
        .bind(observation.captured_at.to_rfc3339())
        .bind(device_name)
        .bind(&snapshot_path)
        .bind(app_name)
        .bind(window_name)
        .bind(browser_url)
        .bind(document_path)
        .bind(observation.context.focused.unwrap_or(true))
        .bind(observation.trigger.as_str())
        .bind(accessibility_text.as_deref())
        .bind(text_source)
        .bind(accessibility_tree_json)
        .bind(content_hash)
        .bind(simhash)
        .execute(&self.pool)
        .await
        .map_err(|error| CaptureError::Store(error.to_string()))?;

        let frame_id = result.last_insert_rowid();
        // Deterministic redaction has already happened in this transaction.
        // Record asynchronous strengthening state separately so model failure
        // never makes a safe capture unavailable to sync.
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
        if let Some(nodes) = sanitized_nodes.filter(|nodes| !nodes.is_empty()) {
            let pool = self.pool.clone();
            // Match the old deferred write: structured elements enrich a frame
            // after it is searchable, and a missing/legacy elements table must
            // not make the primary capture write fail.
            tokio::spawn(async move {
                let _ = insert_accessibility_elements(&pool, frame_id, &nodes).await;
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

async fn insert_accessibility_elements(
    pool: &SqlitePool,
    frame_id: i64,
    nodes: &[AccessibilityNode],
) -> Result<(), sqlx::Error> {
    let mut inserted_ids = std::collections::HashMap::<u32, i64>::new();
    let mut depth_stack: Vec<(u8, i64)> = Vec::new();
    for (sort_order, node) in nodes.iter().enumerate() {
        let depth = node.depth as i32;
        let parent_id = node
            .parent_node_id
            .and_then(|parent_node_id| inserted_ids.get(&parent_node_id).copied())
            .or_else(|| {
                (node.node_id == 0 && depth > 0)
                    .then(|| {
                        depth_stack
                            .iter()
                            .rev()
                            .find(|(node_depth, _)| *node_depth as i32 == depth - 1)
                            .map(|(_, id)| *id)
                    })
                    .flatten()
            });
        let (left, top, width, height) = match &node.bounds {
            Some(bounds) => (
                Some(bounds.left as f64),
                Some(bounds.top as f64),
                Some(bounds.width as f64),
                Some(bounds.height as f64),
            ),
            None => (None, None, None, None),
        };
        let properties = accessibility_properties(node);
        let result = sqlx::query(
            "INSERT INTO elements (frame_id, source, role, text, parent_id, depth, left_bound, \
                top_bound, width_bound, height_bound, confidence, sort_order, properties, on_screen) \
             VALUES (?, 'accessibility', ?, ?, ?, ?, ?, ?, ?, ?, NULL, ?, ?, ?)",
        )
        .bind(frame_id)
        .bind(&node.role)
        .bind((!node.text.is_empty()).then_some(&node.text))
        .bind(parent_id)
        .bind(depth)
        .bind(left)
        .bind(top)
        .bind(width)
        .bind(height)
        .bind(sort_order as i32)
        .bind(properties)
        .bind(node.on_screen.map(i64::from))
        .execute(pool)
        .await?;

        let database_id = result.last_insert_rowid();
        if node.node_id != 0 {
            inserted_ids.insert(node.node_id, database_id);
        }
        while depth_stack
            .last()
            .is_some_and(|(node_depth, _)| *node_depth as i32 >= depth)
        {
            depth_stack.pop();
        }
        depth_stack.push((node.depth, database_id));
    }
    Ok(())
}

fn accessibility_properties(node: &AccessibilityNode) -> Option<String> {
    let mut properties = serde_json::Map::new();
    for (name, value) in [
        ("automation_id", &node.automation_id),
        ("class_name", &node.class_name),
        ("value", &node.value),
        ("help_text", &node.help_text),
        ("url", &node.url),
        ("placeholder", &node.placeholder),
        ("role_description", &node.role_description),
        ("subrole", &node.subrole),
        ("dom_identifier", &node.dom_identifier),
        ("dom_classes", &node.dom_classes),
        ("accelerator_key", &node.accelerator_key),
        ("access_key", &node.access_key),
    ] {
        if let Some(value) = value {
            properties.insert(name.to_string(), serde_json::Value::String(value.clone()));
        }
    }
    for (name, value) in [
        ("is_enabled", node.is_enabled),
        ("is_focused", node.is_focused),
        ("is_selected", node.is_selected),
        ("is_expanded", node.is_expanded),
        ("is_password", node.is_password),
        ("is_keyboard_focusable", node.is_keyboard_focusable),
    ] {
        if let Some(value) = value {
            properties.insert(name.to_string(), serde_json::Value::Bool(value));
        }
    }
    (!properties.is_empty()).then(|| serde_json::Value::Object(properties).to_string())
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
                accessibility_tree_json TEXT, content_hash INTEGER, simhash INTEGER, \
                elements_ref_frame_id INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE elements (\
                id INTEGER PRIMARY KEY AUTOINCREMENT, frame_id INTEGER NOT NULL, source TEXT NOT NULL, \
                role TEXT NOT NULL, text TEXT, parent_id INTEGER, depth INTEGER NOT NULL, \
                left_bound REAL, top_bound REAL, width_bound REAL, height_bound REAL, \
                confidence REAL, sort_order INTEGER, properties TEXT, on_screen INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let store = DystilCaptureStore::new(pool.clone(), temp.path(), "test_monitor");
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
        for column in ["frame_text", "accessibility_tree_json", "window_name"] {
            assert!(!row.get::<String, _>(column).contains("person@example.com"));
        }
        assert!(!row
            .get::<String, _>("accessibility_tree_json")
            .contains("+1-234-567-8901"));
        assert!(!row
            .get::<String, _>("accessibility_tree_json")
            .contains("AKIAIOSFODNN7EXAMPLE"));
    }

    #[tokio::test]
    async fn materializes_sanitized_accessibility_elements_with_parent_links() {
        let temp = TempDir::new().unwrap();
        let (pool, store) = test_store(&temp).await;
        let mut value = observation(None);
        value.accessibility.as_mut().unwrap().nodes = vec![
            AccessibilityNode {
                node_id: 10,
                parent_node_id: None,
                role: "window".to_string(),
                text: "root".to_string(),
                depth: 0,
                bounds: None,
                on_screen: Some(true),
                lines: None,
                automation_id: None,
                class_name: None,
                value: None,
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
            },
            AccessibilityNode {
                node_id: 20,
                parent_node_id: Some(10),
                role: "text".to_string(),
                text: "person@example.com".to_string(),
                depth: 1,
                bounds: None,
                on_screen: Some(false),
                lines: None,
                automation_id: None,
                class_name: None,
                value: None,
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
            },
        ];
        let frame_id = store.persist(value).await.unwrap().frame_id;
        let elements = tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                let rows = sqlx::query(
                    "SELECT id, parent_id, text, on_screen FROM elements WHERE frame_id = ? ORDER BY sort_order",
                )
                .bind(frame_id)
                .fetch_all(&pool)
                .await
                .unwrap();
                if rows.len() == 2 {
                    break rows;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("deferred element insert did not finish");
        assert_eq!(
            elements[1].get::<Option<i64>, _>("parent_id"),
            Some(elements[0].get("id"))
        );
        assert!(!elements[1]
            .get::<String, _>("text")
            .contains("person@example.com"));
        assert_eq!(elements[0].get::<i64, _>("on_screen"), 1);
        assert_eq!(elements[1].get::<i64, _>("on_screen"), 0);
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
