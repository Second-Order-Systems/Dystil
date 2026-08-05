//! Isolated accessibility-tree pruning experiment.
//!
//! This does not participate in capture or sync. It reads existing fixture
//! databases, samples frames deterministically, retains nodes marked on-screen
//! plus the ancestors required to preserve their hierarchy, and writes the
//! resulting JSON to a separate SQLite database.

use std::collections::{HashMap, HashSet};
use std::env;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OpenFlags};
use serde_json::Value;

const DEFAULT_SAMPLE_COUNT: usize = 20;

#[derive(Debug)]
struct FrameTree {
    frame_id: i64,
    timestamp: String,
    app_name: Option<String>,
    window_name: Option<String>,
    tree_json: String,
}

#[derive(Debug)]
struct PrunedTree {
    json: String,
    original_nodes: usize,
    retained_nodes: usize,
    visible_nodes: usize,
    retained_ancestors: usize,
    broken_parent_links: usize,
}

#[derive(Debug, Clone, Copy)]
enum PruneMode {
    WindowOnly,
    AncestorClipped,
}

impl PruneMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::WindowOnly => "window_only",
            Self::AncestorClipped => "ancestor_clipped",
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<String> = env::args().skip(1).collect();
    let windows_fixture = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixture/rich-windows.sqlite"));
    let macos_fixture = args
        .get(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("fixture/rich-macos.sqlite"));
    let output = args
        .get(2)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("a11y-prune-samples.sqlite"));
    let sample_count = args
        .get(3)
        .map(|value| value.parse::<usize>())
        .transpose()
        .context("sample count must be a positive integer")?
        .unwrap_or(DEFAULT_SAMPLE_COUNT);

    if sample_count == 0 {
        bail!("sample count must be greater than zero");
    }
    if output.exists() {
        bail!(
            "refusing to overwrite existing output: {}",
            output.display()
        );
    }

    let output_db = Connection::open(&output)
        .with_context(|| format!("open output database {}", output.display()))?;
    create_output_schema(&output_db)?;

    process_fixture(&output_db, "windows", &windows_fixture, sample_count)?;
    process_fixture(&output_db, "macos", &macos_fixture, sample_count)?;

    output_db.execute_batch("PRAGMA optimize;")?;
    print_summary(&output_db, &output)?;
    Ok(())
}

fn create_output_schema(db: &Connection) -> Result<()> {
    db.execute_batch(
        "CREATE TABLE semantic_tree_samples (
            id INTEGER PRIMARY KEY,
            platform TEXT NOT NULL,
            prune_mode TEXT NOT NULL,
            source_fixture TEXT NOT NULL,
            source_frame_id INTEGER NOT NULL,
            captured_at TEXT NOT NULL,
            app_name TEXT,
            window_name TEXT,
            original_nodes INTEGER NOT NULL,
            retained_nodes INTEGER NOT NULL,
            visible_nodes INTEGER NOT NULL,
            retained_ancestors INTEGER NOT NULL,
            broken_parent_links INTEGER NOT NULL,
            original_bytes INTEGER NOT NULL,
            pruned_bytes INTEGER NOT NULL,
            tree_json TEXT NOT NULL,
            UNIQUE(platform, prune_mode, source_fixture, source_frame_id)
        );",
    )?;
    Ok(())
}

fn process_fixture(
    output_db: &Connection,
    platform: &str,
    fixture: &Path,
    sample_count: usize,
) -> Result<()> {
    let input = Connection::open_with_flags(fixture, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open fixture {}", fixture.display()))?;
    let frames = read_frames(&input)?;
    if frames.is_empty() {
        bail!("fixture has no accessibility trees: {}", fixture.display());
    }

    for index in stress_sample_indices(&frames, sample_count) {
        let frame = &frames[index];
        for mode in [PruneMode::WindowOnly, PruneMode::AncestorClipped] {
            let pruned = prune_visible_tree(&frame.tree_json, mode).with_context(|| {
                format!("prune frame {} from {}", frame.frame_id, fixture.display())
            })?;
            output_db.execute(
                "INSERT INTO semantic_tree_samples (
                    platform, prune_mode, source_fixture, source_frame_id, captured_at,
                    app_name, window_name, original_nodes, retained_nodes,
                    visible_nodes, retained_ancestors, broken_parent_links,
                    original_bytes, pruned_bytes, tree_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    platform,
                    mode.as_str(),
                    fixture.display().to_string(),
                    frame.frame_id,
                    frame.timestamp,
                    frame.app_name,
                    frame.window_name,
                    pruned.original_nodes,
                    pruned.retained_nodes,
                    pruned.visible_nodes,
                    pruned.retained_ancestors,
                    pruned.broken_parent_links,
                    frame.tree_json.len(),
                    pruned.json.len(),
                    pruned.json,
                ],
            )?;
        }
    }
    Ok(())
}

fn read_frames(db: &Connection) -> Result<Vec<FrameTree>> {
    let mut statement = db.prepare(
        "SELECT id, timestamp, app_name, window_name, accessibility_tree_json
         FROM frames
         WHERE accessibility_tree_json IS NOT NULL
           AND trim(accessibility_tree_json) != ''
         ORDER BY id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(FrameTree {
            frame_id: row.get(0)?,
            timestamp: row.get(1)?,
            app_name: row.get(2)?,
            window_name: row.get(3)?,
            tree_json: row.get(4)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn evenly_spaced_indices(total: usize, requested: usize) -> Vec<usize> {
    let count = requested.min(total);
    if count == 1 {
        return vec![0];
    }
    (0..count)
        .map(|sample| sample * (total - 1) / (count - 1))
        .collect()
}

fn stress_sample_indices(frames: &[FrameTree], requested: usize) -> Vec<usize> {
    let count = requested.min(frames.len());
    let mut largest: Vec<usize> = (0..frames.len()).collect();
    largest.sort_by_key(|index| std::cmp::Reverse(frames[*index].tree_json.len()));

    let mut selected = HashSet::new();
    for index in largest.into_iter().take(count / 2) {
        selected.insert(index);
    }
    for index in evenly_spaced_indices(frames.len(), count) {
        if selected.len() >= count {
            break;
        }
        selected.insert(index);
    }
    for index in 0..frames.len() {
        if selected.len() >= count {
            break;
        }
        selected.insert(index);
    }
    let mut selected: Vec<usize> = selected.into_iter().collect();
    selected.sort_unstable();
    selected
}

#[derive(Debug, Clone, Copy)]
struct Rect {
    left: f64,
    top: f64,
    right: f64,
    bottom: f64,
}

impl Rect {
    const MIN_CREDIBLE_VIEWPORT_DIMENSION: f64 = 0.01;

    fn from_node(node: &Value) -> Option<Self> {
        let bounds = node.get("bounds")?;
        let left = bounds.get("left")?.as_f64()?;
        let top = bounds.get("top")?.as_f64()?;
        let width = bounds.get("width")?.as_f64()?;
        let height = bounds.get("height")?.as_f64()?;
        (width > 0.0 && height > 0.0).then_some(Self {
            left,
            top,
            right: left + width,
            bottom: top + height,
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

    fn is_credible_viewport(self) -> bool {
        self.right - self.left >= Self::MIN_CREDIBLE_VIEWPORT_DIMENSION
            && self.bottom - self.top >= Self::MIN_CREDIBLE_VIEWPORT_DIMENSION
    }
}

fn visible_with_ancestor_clipping(
    index: usize,
    nodes: &[Value],
    index_by_id: &HashMap<u64, usize>,
) -> bool {
    if nodes[index].get("on_screen").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let Some(mut visible_rect) = Rect::from_node(&nodes[index]) else {
        return false;
    };
    let mut cursor = index;
    let mut visited_ids = HashSet::new();
    while let Some(parent_id) = nodes[cursor].get("parent_node_id").and_then(Value::as_u64) {
        if !visited_ids.insert(parent_id) {
            return false;
        }
        let Some(parent_index) = index_by_id.get(&parent_id).copied() else {
            return false;
        };
        if is_viewport_container(&nodes[parent_index]) {
            let Some(parent_rect) = Rect::from_node(&nodes[parent_index]) else {
                cursor = parent_index;
                continue;
            };
            if !parent_rect.is_credible_viewport() {
                cursor = parent_index;
                continue;
            }
            let Some(intersection) = visible_rect.intersection(parent_rect) else {
                return false;
            };
            visible_rect = intersection;
        }
        cursor = parent_index;
    }
    true
}

fn is_viewport_container(node: &Value) -> bool {
    let role = node
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let class_name = node
        .get("class_name")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    matches!(
        role.as_str(),
        "list"
            | "tree"
            | "table"
            | "datagrid"
            | "document"
            | "axlist"
            | "axoutline"
            | "axscrollarea"
            | "axtable"
    ) || ["scroll", "overflow", "occlusion", "virtual"]
        .iter()
        .any(|marker| class_name.contains(marker))
}

fn prune_visible_tree(tree_json: &str, mode: PruneMode) -> Result<PrunedTree> {
    let nodes: Vec<Value> = serde_json::from_str(tree_json)?;
    let mut index_by_id = HashMap::<u64, usize>::new();
    for (index, node) in nodes.iter().enumerate() {
        if let Some(node_id) = node.get("node_id").and_then(Value::as_u64) {
            index_by_id.insert(node_id, index);
        }
    }

    let visible: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| match mode {
            PruneMode::WindowOnly => {
                (node.get("on_screen").and_then(Value::as_bool) == Some(true)).then_some(index)
            }
            PruneMode::AncestorClipped => {
                visible_with_ancestor_clipping(index, &nodes, &index_by_id).then_some(index)
            }
        })
        .collect();
    let mut retained: HashSet<usize> = visible.iter().copied().collect();
    let mut broken_parent_links = 0usize;

    for visible_index in &visible {
        let mut cursor = *visible_index;
        let mut visited_ids = HashSet::<u64>::new();
        while let Some(parent_id) = nodes[cursor].get("parent_node_id").and_then(Value::as_u64) {
            if !visited_ids.insert(parent_id) {
                broken_parent_links += 1;
                break;
            }
            let Some(parent_index) = index_by_id.get(&parent_id).copied() else {
                broken_parent_links += 1;
                break;
            };
            retained.insert(parent_index);
            cursor = parent_index;
        }
    }

    let retained_nodes: Vec<Value> = nodes
        .iter()
        .enumerate()
        .filter_map(|(index, node)| retained.contains(&index).then_some(node.clone()))
        .collect();
    let json = serde_json::to_string(&retained_nodes)?;

    Ok(PrunedTree {
        json,
        original_nodes: nodes.len(),
        retained_nodes: retained_nodes.len(),
        visible_nodes: visible.len(),
        retained_ancestors: retained_nodes.len().saturating_sub(visible.len()),
        broken_parent_links,
    })
}

fn print_summary(db: &Connection, output: &Path) -> Result<()> {
    let mut statement = db.prepare(
        "SELECT platform, prune_mode, COUNT(*), SUM(original_nodes), SUM(retained_nodes),
                SUM(visible_nodes), SUM(retained_ancestors), SUM(broken_parent_links),
                SUM(original_bytes), SUM(pruned_bytes)
         FROM semantic_tree_samples
         GROUP BY platform, prune_mode
         ORDER BY platform, prune_mode",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, i64>(7)?,
            row.get::<_, i64>(8)?,
            row.get::<_, i64>(9)?,
        ))
    })?;

    println!("output={}", output.display());
    for row in rows {
        let (
            platform,
            mode,
            samples,
            original_nodes,
            retained_nodes,
            visible_nodes,
            ancestors,
            broken,
            original_bytes,
            pruned_bytes,
        ) = row?;
        let reduction = if original_bytes == 0 {
            0.0
        } else {
            100.0 * (original_bytes - pruned_bytes) as f64 / original_bytes as f64
        };
        println!(
            "platform={platform} mode={mode} samples={samples} nodes={original_nodes}->{retained_nodes} visible={visible_nodes} ancestors={ancestors} broken_parent_links={broken} bytes={original_bytes}->{pruned_bytes} reduction={reduction:.1}%"
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn retains_visible_nodes_and_their_ancestors_only() {
        let input = json!([
            {"node_id": 1, "parent_node_id": null, "role": "Window", "on_screen": true},
            {"node_id": 2, "parent_node_id": 1, "role": "List", "on_screen": false},
            {"node_id": 3, "parent_node_id": 2, "role": "Text", "on_screen": true},
            {"node_id": 4, "parent_node_id": 2, "role": "Text", "on_screen": false}
        ]);
        let result = prune_visible_tree(&input.to_string(), PruneMode::WindowOnly).unwrap();
        let output: Vec<Value> = serde_json::from_str(&result.json).unwrap();
        let ids: Vec<u64> = output
            .iter()
            .map(|node| node["node_id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, vec![1, 2, 3]);
        assert_eq!(result.visible_nodes, 2);
        assert_eq!(result.retained_ancestors, 1);
        assert_eq!(result.broken_parent_links, 0);
    }

    #[test]
    fn sampling_is_deterministic_and_includes_both_ends() {
        assert_eq!(evenly_spaced_indices(101, 3), vec![0, 50, 100]);
        assert_eq!(evenly_spaced_indices(2, 20), vec![0, 1]);
    }

    #[test]
    fn ancestor_clipping_removes_children_outside_their_container() {
        let input = json!([
            {"node_id": 1, "parent_node_id": null, "bounds": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}, "on_screen": true},
            {"node_id": 2, "parent_node_id": 1, "role": "List", "bounds": {"left": 0.2, "top": 0.2, "width": 0.6, "height": 0.6}, "on_screen": true},
            {"node_id": 3, "parent_node_id": 2, "bounds": {"left": 0.3, "top": 0.3, "width": 0.1, "height": 0.1}, "on_screen": true},
            {"node_id": 4, "parent_node_id": 2, "bounds": {"left": 0.3, "top": 0.05, "width": 0.1, "height": 0.1}, "on_screen": true}
        ]);
        let result = prune_visible_tree(&input.to_string(), PruneMode::AncestorClipped).unwrap();
        let output: Vec<Value> = serde_json::from_str(&result.json).unwrap();
        let ids: Vec<u64> = output
            .iter()
            .map(|node| node["node_id"].as_u64().unwrap())
            .collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[test]
    fn ordinary_structural_ancestors_do_not_clip_children() {
        let input = json!([
            {"node_id": 1, "parent_node_id": null, "role": "Window", "bounds": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}, "on_screen": true},
            {"node_id": 2, "parent_node_id": 1, "role": "Group", "bounds": {"left": 0.2, "top": 0.2, "width": 0.6, "height": 0.1}, "on_screen": true},
            {"node_id": 3, "parent_node_id": 2, "role": "Text", "bounds": {"left": 0.3, "top": 0.4, "width": 0.1, "height": 0.1}, "on_screen": true}
        ]);
        let result = prune_visible_tree(&input.to_string(), PruneMode::AncestorClipped).unwrap();
        let output: Vec<Value> = serde_json::from_str(&result.json).unwrap();
        assert_eq!(output.len(), 3);
    }

    #[test]
    fn degenerate_viewport_bounds_do_not_clip_children() {
        let input = json!([
            {"node_id": 1, "parent_node_id": null, "role": "Window", "bounds": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0}, "on_screen": true},
            {"node_id": 2, "parent_node_id": 1, "role": "AXList", "bounds": {"left": 0.8, "top": 0.8, "width": 0.001, "height": 0.001}, "on_screen": true},
            {"node_id": 3, "parent_node_id": 2, "role": "AXStaticText", "bounds": {"left": 0.3, "top": 0.4, "width": 0.1, "height": 0.1}, "on_screen": true}
        ]);
        let result = prune_visible_tree(&input.to_string(), PruneMode::AncestorClipped).unwrap();
        let output: Vec<Value> = serde_json::from_str(&result.json).unwrap();
        assert_eq!(output.len(), 3);
    }
}
