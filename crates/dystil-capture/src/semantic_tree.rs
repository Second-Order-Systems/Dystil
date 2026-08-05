//! Bounded, best-effort semantic accessibility-tree sampling.
//!
//! This store is deliberately independent from the capture database and its
//! sync cursor. A failure here must never reject an ordinary captured frame.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AccessibilityNode, Bounds};

pub const SEMANTIC_TREE_SCHEMA_VERSION: i64 = 1;
pub const MAX_SAMPLES_PER_SURFACE: i64 = 20;
pub const SOFT_ROLLING_BYTES: i64 = 10 * 1024 * 1024;
pub const HARD_ROLLING_BYTES: i64 = 50 * 1024 * 1024;
pub const MAX_PENDING_BYTES: i64 = 50 * 1024 * 1024;
pub const MAX_SAMPLE_BYTES: usize = 1024 * 1024;
const MAX_DATABASE_BYTES: i64 = 64 * 1024 * 1024;
const MAX_WAL_BYTES: i64 = 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct SemanticSampleCandidate<'a> {
    pub source_frame_id: i64,
    pub captured_at: DateTime<Utc>,
    pub platform: &'a str,
    pub app_name: &'a str,
    pub app_version: Option<&'a str>,
    pub window_name: Option<&'a str>,
    pub browser_url: Option<&'a str>,
    pub nodes: &'a [AccessibilityNode],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleDecision {
    Stored {
        sample_id: String,
        compressed_bytes: usize,
    },
    EmptyAfterPruning,
    Duplicate,
    SurfaceSaturated,
    ConservationMode,
    RollingLimit,
    PendingLimit,
    SampleTooLarge {
        compressed_bytes: usize,
    },
}

#[derive(Debug, Clone)]
pub struct PendingSemanticSample {
    pub sample_id: String,
    pub source_frame_id: i64,
    pub surface_key: String,
    pub layout_fingerprint: String,
    pub schema_version: i64,
    pub codec: String,
    pub payload_sha256: String,
    pub payload: Vec<u8>,
    pub captured_at: String,
    pub platform: String,
    pub app_name: String,
    pub app_version: Option<String>,
}

#[derive(Clone)]
pub struct SemanticTreeStore {
    connection: Arc<Mutex<Connection>>,
}

impl SemanticTreeStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        initialize(&connection).map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub fn record(&self, candidate: SemanticSampleCandidate<'_>) -> Result<SampleDecision, String> {
        let normalized = normalize_nodes(candidate.platform, candidate.nodes);
        if normalized.is_empty() {
            return Ok(SampleDecision::EmptyAfterPruning);
        }

        let surface_key = surface_key(&candidate);
        let layout_fingerprint = layout_fingerprint(&normalized)?;
        let payload_json = serde_json::to_vec(&SemanticTreePayload {
            schema_version: SEMANTIC_TREE_SCHEMA_VERSION,
            nodes: normalized,
        })
        .map_err(|error| error.to_string())?;
        let payload_sha256 = sha256_id(&payload_json);
        let payload = zstd::stream::encode_all(Cursor::new(payload_json), 1)
            .map_err(|error| error.to_string())?;
        if payload.len() > MAX_SAMPLE_BYTES {
            return Ok(SampleDecision::SampleTooLarge {
                compressed_bytes: payload.len(),
            });
        }

        let mut connection = self
            .connection
            .lock()
            .map_err(|_| "semantic sample store lock poisoned".to_string())?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| error.to_string())?;

        let duplicate: bool = transaction
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM semantic_tree_samples
                    WHERE surface_key = ?1 AND layout_fingerprint = ?2
                      AND schema_version = ?3
                )",
                params![
                    surface_key,
                    layout_fingerprint,
                    SEMANTIC_TREE_SCHEMA_VERSION
                ],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if duplicate {
            return Ok(SampleDecision::Duplicate);
        }

        let surface_count: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM semantic_tree_samples
                 WHERE surface_key = ?1 AND schema_version = ?2",
                params![surface_key, SEMANTIC_TREE_SCHEMA_VERSION],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if surface_count >= MAX_SAMPLES_PER_SURFACE {
            return Ok(SampleDecision::SurfaceSaturated);
        }

        let rolling_bytes: i64 = transaction
            .query_row(
                "SELECT COALESCE(SUM(payload_bytes), 0)
                 FROM semantic_tree_samples
                 WHERE datetime(captured_at) >= datetime('now', '-24 hours')",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        let pending_bytes: i64 = transaction
            .query_row(
                "SELECT COALESCE(SUM(length(payload)), 0)
                 FROM semantic_tree_samples WHERE payload IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if let Some(decision) = budget_decision(
            rolling_bytes,
            pending_bytes,
            surface_count,
            payload.len() as i64,
        ) {
            return Ok(decision);
        }

        let sample_id = uuid::Uuid::new_v4().to_string();
        transaction
            .execute(
                "INSERT INTO semantic_tree_samples (
                    sample_id, source_frame_id, surface_key, layout_fingerprint,
                    schema_version, codec, payload_sha256, payload, payload_bytes,
                    captured_at, platform, app_name, app_version, upload_state
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'zstd', ?6, ?7, ?8, ?9, ?10, ?11, ?12, 'pending')",
                params![
                    sample_id,
                    candidate.source_frame_id,
                    surface_key,
                    layout_fingerprint,
                    SEMANTIC_TREE_SCHEMA_VERSION,
                    payload_sha256,
                    payload,
                    payload.len() as i64,
                    candidate.captured_at.to_rfc3339(),
                    candidate.platform,
                    candidate.app_name,
                    candidate.app_version,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;

        Ok(SampleDecision::Stored {
            sample_id,
            compressed_bytes: payload.len(),
        })
    }

    pub fn pending(&self, limit: usize) -> Result<Vec<PendingSemanticSample>, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "semantic sample store lock poisoned".to_string())?;
        let mut statement = connection
            .prepare(
                "SELECT sample_id, source_frame_id, surface_key, layout_fingerprint,
                        schema_version, codec, payload_sha256, payload, captured_at,
                        platform, app_name, app_version
                 FROM semantic_tree_samples
                 WHERE upload_state = 'pending' AND payload IS NOT NULL
                 ORDER BY datetime(captured_at), sample_id
                 LIMIT ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([limit as i64], |row| {
                Ok(PendingSemanticSample {
                    sample_id: row.get(0)?,
                    source_frame_id: row.get(1)?,
                    surface_key: row.get(2)?,
                    layout_fingerprint: row.get(3)?,
                    schema_version: row.get(4)?,
                    codec: row.get(5)?,
                    payload_sha256: row.get(6)?,
                    payload: row.get(7)?,
                    captured_at: row.get(8)?,
                    platform: row.get(9)?,
                    app_name: row.get(10)?,
                    app_version: row.get(11)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    pub fn acknowledge(&self, sample_id: &str, payload_sha256: &str) -> Result<bool, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "semantic sample store lock poisoned".to_string())?;
        connection
            .execute(
                "UPDATE semantic_tree_samples
                 SET payload = NULL, upload_state = 'acknowledged', acknowledged_at = datetime('now')
                 WHERE sample_id = ?1 AND payload_sha256 = ?2
                   AND upload_state = 'pending'",
                params![sample_id, payload_sha256],
            )
            .map(|changed| changed == 1)
            .map_err(|error| error.to_string())
    }

    pub fn pending_payload_bytes(&self) -> Result<i64, String> {
        let connection = self
            .connection
            .lock()
            .map_err(|_| "semantic sample store lock poisoned".to_string())?;
        connection
            .query_row(
                "SELECT COALESCE(SUM(length(payload)), 0)
                 FROM semantic_tree_samples WHERE payload IS NOT NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }
}

fn budget_decision(
    rolling_bytes: i64,
    pending_bytes: i64,
    surface_count: i64,
    candidate_bytes: i64,
) -> Option<SampleDecision> {
    if rolling_bytes >= SOFT_ROLLING_BYTES && surface_count > 0 {
        return Some(SampleDecision::ConservationMode);
    }
    if rolling_bytes.saturating_add(candidate_bytes) > HARD_ROLLING_BYTES {
        return Some(SampleDecision::RollingLimit);
    }
    if pending_bytes.saturating_add(candidate_bytes) > MAX_PENDING_BYTES {
        return Some(SampleDecision::PendingLimit);
    }
    None
}

#[derive(Debug, Serialize, Deserialize)]
struct SemanticTreePayload {
    schema_version: i64,
    nodes: Vec<AccessibilityNode>,
}

fn initialize(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(&format!(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA busy_timeout = 30000;
         PRAGMA journal_size_limit = {MAX_WAL_BYTES};
         CREATE TABLE IF NOT EXISTS semantic_tree_samples (
             sample_id TEXT PRIMARY KEY,
             source_frame_id INTEGER NOT NULL,
             surface_key TEXT NOT NULL,
             layout_fingerprint TEXT NOT NULL,
             schema_version INTEGER NOT NULL,
             codec TEXT NOT NULL CHECK (codec = 'zstd'),
             payload_sha256 TEXT NOT NULL,
             payload BLOB,
             payload_bytes INTEGER NOT NULL,
             captured_at TEXT NOT NULL,
             platform TEXT NOT NULL,
             app_name TEXT NOT NULL,
             app_version TEXT,
             upload_state TEXT NOT NULL CHECK (upload_state IN ('pending', 'acknowledged')),
             acknowledged_at TEXT,
             UNIQUE(surface_key, layout_fingerprint, schema_version)
         );
         CREATE INDEX IF NOT EXISTS idx_semantic_samples_pending
         ON semantic_tree_samples(upload_state, captured_at)
         WHERE payload IS NOT NULL;"
    ))?;
    let page_size: i64 = connection.query_row("PRAGMA page_size", [], |row| row.get(0))?;
    let max_pages = MAX_DATABASE_BYTES / page_size.max(1);
    connection.pragma_update(None, "max_page_count", max_pages)?;
    Ok(())
}

fn normalize_nodes(platform: &str, nodes: &[AccessibilityNode]) -> Vec<AccessibilityNode> {
    let index_by_id: HashMap<u32, usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.node_id != 0)
        .map(|(index, node)| (node.node_id, index))
        .collect();
    let mut visible = HashSet::new();
    for (index, node) in nodes.iter().enumerate() {
        if node.on_screen == Some(true)
            && (platform != "windows" || visible_with_windows_viewports(index, nodes, &index_by_id))
        {
            visible.insert(index);
        }
    }

    let mut retained = visible.clone();
    for visible_index in visible {
        let mut cursor = visible_index;
        let mut visited = HashSet::new();
        while let Some(parent_id) = nodes[cursor].parent_node_id {
            if !visited.insert(parent_id) {
                break;
            }
            let Some(parent_index) = index_by_id.get(&parent_id).copied() else {
                break;
            };
            retained.insert(parent_index);
            cursor = parent_index;
        }
    }

    let ordered: Vec<usize> = (0..nodes.len())
        .filter(|index| retained.contains(index))
        .collect();
    let new_ids: HashMap<u32, u32> = ordered
        .iter()
        .enumerate()
        .filter_map(|(index, old_index)| {
            (nodes[*old_index].node_id != 0)
                .then_some((nodes[*old_index].node_id, (index + 1) as u32))
        })
        .collect();

    ordered
        .into_iter()
        .enumerate()
        .map(|(index, old_index)| {
            let mut node = nodes[old_index].clone();
            node.node_id = (index + 1) as u32;
            node.parent_node_id = node
                .parent_node_id
                .and_then(|parent_id| new_ids.get(&parent_id).copied());
            if platform == "macos" {
                strip_unverified_macos_content(&mut node);
            }
            node
        })
        .collect()
}

fn strip_unverified_macos_content(node: &mut AccessibilityNode) {
    node.text.clear();
    node.lines = None;
    node.value = None;
    node.help_text = None;
    node.url = None;
    node.placeholder = None;
    node.accelerator_key = None;
    node.access_key = None;
}

fn visible_with_windows_viewports(
    index: usize,
    nodes: &[AccessibilityNode],
    index_by_id: &HashMap<u32, usize>,
) -> bool {
    let Some(mut visible_rect) = Rect::from_bounds(nodes[index].bounds.as_ref()) else {
        return false;
    };
    let mut cursor = index;
    let mut visited = HashSet::new();
    while let Some(parent_id) = nodes[cursor].parent_node_id {
        if !visited.insert(parent_id) {
            return false;
        }
        let Some(parent_index) = index_by_id.get(&parent_id).copied() else {
            return false;
        };
        let parent = &nodes[parent_index];
        if is_windows_viewport(parent) {
            if let Some(parent_rect) = Rect::from_bounds(parent.bounds.as_ref()) {
                if parent_rect.credible_viewport() {
                    let Some(intersection) = visible_rect.intersection(parent_rect) else {
                        return false;
                    };
                    visible_rect = intersection;
                }
            }
        }
        cursor = parent_index;
    }
    true
}

fn is_windows_viewport(node: &AccessibilityNode) -> bool {
    let role = node.role.to_ascii_lowercase();
    let class_name = node
        .class_name
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(
        role.as_str(),
        "list" | "tree" | "table" | "datagrid" | "document"
    ) || ["scroll", "overflow", "occlusion", "virtual"]
        .iter()
        .any(|marker| class_name.contains(marker))
}

#[derive(Clone, Copy)]
struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    fn from_bounds(bounds: Option<&Bounds>) -> Option<Self> {
        let bounds = bounds?;
        (bounds.width > 0.0 && bounds.height > 0.0).then_some(Self {
            left: bounds.left as f64,
            top: bounds.top as f64,
            right: (bounds.left + bounds.width) as f64,
            bottom: (bounds.top + bounds.height) as f64,
        })
    }

    fn intersection(self, other: Self) -> Option<Self> {
        let result = Self {
            left: self.left.max(other.left),
            top: self.top.max(other.top),
            right: self.right.min(other.right),
            bottom: self.bottom.min(other.bottom),
        };
        (result.right > result.left && result.bottom > result.top).then_some(result)
    }

    fn credible_viewport(self) -> bool {
        self.right - self.left >= 0.01 && self.bottom - self.top >= 0.01
    }
}

#[derive(Serialize)]
struct FingerprintNode<'a> {
    parent_node_id: Option<u32>,
    role: &'a str,
    automation_id: Option<&'a str>,
    class_name: Option<&'a str>,
    role_description: Option<&'a str>,
    subrole: Option<&'a str>,
    dom_identifier: Option<&'a str>,
    dom_classes: Option<&'a str>,
    is_enabled: Option<bool>,
    is_password: Option<bool>,
    is_keyboard_focusable: Option<bool>,
}

fn layout_fingerprint(nodes: &[AccessibilityNode]) -> Result<String, String> {
    let stable: Vec<_> = nodes
        .iter()
        .map(|node| FingerprintNode {
            parent_node_id: node.parent_node_id,
            role: &node.role,
            automation_id: node.automation_id.as_deref(),
            class_name: node.class_name.as_deref(),
            role_description: node.role_description.as_deref(),
            subrole: node.subrole.as_deref(),
            dom_identifier: node.dom_identifier.as_deref(),
            dom_classes: node.dom_classes.as_deref(),
            is_enabled: node.is_enabled,
            is_password: node.is_password,
            is_keyboard_focusable: node.is_keyboard_focusable,
        })
        .collect();
    serde_json::to_vec(&stable)
        .map(|value| sha256_id(&value))
        .map_err(|error| error.to_string())
}

fn surface_key(candidate: &SemanticSampleCandidate<'_>) -> String {
    let family = candidate
        .browser_url
        .and_then(|value| url::Url::parse(value).ok())
        .and_then(|url| url.host_str().map(str::to_owned))
        .or_else(|| candidate.window_name.and_then(window_family))
        .unwrap_or_else(|| "default".to_string());
    sha256_id(
        format!(
            "{}\0{}\0{}\0{}",
            candidate.platform.to_ascii_lowercase(),
            candidate.app_name.to_ascii_lowercase(),
            candidate.app_version.unwrap_or_default(),
            family.to_ascii_lowercase()
        )
        .as_bytes(),
    )
}

fn window_family(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    [
        "chat",
        "channel",
        "calendar",
        "meeting",
        "call",
        "settings",
        "preferences",
        "editor",
        "terminal",
    ]
    .into_iter()
    .find(|marker| lower.contains(marker))
    .map(str::to_owned)
}

fn sha256_id(value: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn node(
        id: u32,
        parent: Option<u32>,
        role: &str,
        text: &str,
        bounds: Bounds,
        on_screen: bool,
    ) -> AccessibilityNode {
        AccessibilityNode {
            node_id: id,
            parent_node_id: parent,
            role: role.to_string(),
            text: text.to_string(),
            depth: parent.is_some() as u8,
            bounds: Some(bounds),
            on_screen: Some(on_screen),
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
            is_enabled: Some(true),
            is_focused: None,
            is_selected: None,
            is_expanded: None,
            is_password: Some(false),
            is_keyboard_focusable: Some(false),
            accelerator_key: None,
            access_key: None,
        }
    }

    fn bounds(left: f32, top: f32, width: f32, height: f32) -> Bounds {
        Bounds {
            left,
            top,
            width,
            height,
        }
    }

    fn candidate<'a>(nodes: &'a [AccessibilityNode]) -> SemanticSampleCandidate<'a> {
        SemanticSampleCandidate {
            source_frame_id: 42,
            captured_at: Utc::now(),
            platform: "windows",
            app_name: "Teams.exe",
            app_version: Some("1"),
            window_name: Some("Chat | Person"),
            browser_url: None,
            nodes,
        }
    }

    #[test]
    fn windows_pruning_keeps_visible_hierarchy_and_removes_virtualized_history() {
        let mut viewport = node(2, Some(1), "Group", "", bounds(0.2, 0.2, 0.6, 0.6), true);
        viewport.class_name = Some("vdi-occlusion".to_string());
        let nodes = vec![
            node(1, None, "Window", "", bounds(0.0, 0.0, 1.0, 1.0), true),
            viewport,
            node(
                3,
                Some(2),
                "Text",
                "visible",
                bounds(0.3, 0.3, 0.1, 0.1),
                true,
            ),
            node(
                4,
                Some(2),
                "Text",
                "history",
                bounds(0.3, 0.05, 0.1, 0.1),
                true,
            ),
        ];
        let output = normalize_nodes("windows", &nodes);
        assert_eq!(output.len(), 3);
        assert_eq!(output[2].text, "visible");
        assert_eq!(output[2].parent_node_id, Some(2));
    }

    #[test]
    fn macos_samples_strip_unverified_free_form_content() {
        let mut value = node(
            1,
            None,
            "AXStaticText",
            "private message",
            bounds(0.0, 0.0, 1.0, 1.0),
            true,
        );
        value.value = Some("private value".to_string());
        value.class_name = Some("message-body".to_string());
        let output = normalize_nodes("macos", &[value]);
        assert_eq!(output.len(), 1);
        assert!(output[0].text.is_empty());
        assert!(output[0].value.is_none());
        assert_eq!(output[0].class_name.as_deref(), Some("message-body"));
    }

    #[test]
    fn store_deduplicates_and_clears_only_matching_ack() {
        let temp = TempDir::new().unwrap();
        let store = SemanticTreeStore::open(temp.path().join("semantic.sqlite")).unwrap();
        let nodes = vec![node(
            1,
            None,
            "Window",
            "hello",
            bounds(0.0, 0.0, 1.0, 1.0),
            true,
        )];
        let first = store.record(candidate(&nodes)).unwrap();
        let SampleDecision::Stored { sample_id, .. } = first else {
            panic!("sample was not stored");
        };
        assert_eq!(
            store.record(candidate(&nodes)).unwrap(),
            SampleDecision::Duplicate
        );
        let pending = store.pending(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(!store.acknowledge(&sample_id, "sha256:wrong").unwrap());
        assert!(store.pending_payload_bytes().unwrap() > 0);
        assert!(store
            .acknowledge(&sample_id, &pending[0].payload_sha256)
            .unwrap());
        assert_eq!(store.pending_payload_bytes().unwrap(), 0);
        assert!(store.pending(10).unwrap().is_empty());
    }

    #[test]
    fn compressed_payload_round_trips_to_versioned_json() {
        let temp = TempDir::new().unwrap();
        let store = SemanticTreeStore::open(temp.path().join("semantic.sqlite")).unwrap();
        let nodes = vec![node(
            1,
            None,
            "Window",
            "hello",
            bounds(0.0, 0.0, 1.0, 1.0),
            true,
        )];
        assert!(matches!(
            store.record(candidate(&nodes)).unwrap(),
            SampleDecision::Stored { .. }
        ));
        let pending = store.pending(1).unwrap();
        let json = zstd::stream::decode_all(Cursor::new(&pending[0].payload)).unwrap();
        let payload: SemanticTreePayload = serde_json::from_slice(&json).unwrap();
        assert_eq!(payload.schema_version, SEMANTIC_TREE_SCHEMA_VERSION);
        assert_eq!(payload.nodes[0].text, "hello");
        assert_eq!(sha256_id(&json), pending[0].payload_sha256);
    }

    #[test]
    fn byte_budget_boundaries_match_the_approved_policy() {
        assert_eq!(
            budget_decision(SOFT_ROLLING_BYTES, 0, 1, 1),
            Some(SampleDecision::ConservationMode)
        );
        assert_eq!(budget_decision(SOFT_ROLLING_BYTES, 0, 0, 1), None);
        assert_eq!(
            budget_decision(HARD_ROLLING_BYTES - 1, 0, 0, 2),
            Some(SampleDecision::RollingLimit)
        );
        assert_eq!(
            budget_decision(0, MAX_PENDING_BYTES - 1, 0, 2),
            Some(SampleDecision::PendingLimit)
        );
        assert_eq!(
            budget_decision(HARD_ROLLING_BYTES - 1, MAX_PENDING_BYTES - 1, 0, 1),
            None
        );
    }

    #[test]
    fn surface_accepts_at_most_twenty_distinct_layouts() {
        let temp = TempDir::new().unwrap();
        let store = SemanticTreeStore::open(temp.path().join("semantic.sqlite")).unwrap();
        for index in 0..MAX_SAMPLES_PER_SURFACE {
            let mut value = node(
                1,
                None,
                "Window",
                "volatile text",
                bounds(0.0, 0.0, 1.0, 1.0),
                true,
            );
            value.automation_id = Some(format!("layout-{index}"));
            assert!(matches!(
                store.record(candidate(&[value])).unwrap(),
                SampleDecision::Stored { .. }
            ));
        }
        let mut overflow = node(
            1,
            None,
            "Window",
            "volatile text",
            bounds(0.0, 0.0, 1.0, 1.0),
            true,
        );
        overflow.automation_id = Some("layout-overflow".to_string());
        assert_eq!(
            store.record(candidate(&[overflow])).unwrap(),
            SampleDecision::SurfaceSaturated
        );
    }

    #[test]
    fn concurrent_duplicate_candidates_store_only_one_payload() {
        let temp = TempDir::new().unwrap();
        let store = SemanticTreeStore::open(temp.path().join("semantic.sqlite")).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let store = store.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let nodes = vec![node(
                        1,
                        None,
                        "Window",
                        "same layout",
                        bounds(0.0, 0.0, 1.0, 1.0),
                        true,
                    )];
                    barrier.wait();
                    store.record(candidate(&nodes)).unwrap()
                })
            })
            .collect();
        let decisions: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| matches!(decision, SampleDecision::Stored { .. }))
                .count(),
            1
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|decision| **decision == SampleDecision::Duplicate)
                .count(),
            7
        );
        assert_eq!(store.pending(20).unwrap().len(), 1);
    }
}
