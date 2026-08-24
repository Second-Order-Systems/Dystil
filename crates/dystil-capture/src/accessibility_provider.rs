use crate::a11y::tree::{
    create_tree_walker, AccessibilityTreeNode, LineSpan, NodeBounds, TreeSnapshot, TreeWalkResult,
    TreeWalkerConfig, TreeWalkerPlatform, TruncationReason,
};
use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
#[cfg(feature = "debug-capture")]
use std::time::Instant;
use tracing::debug;

use crate::{
    AccessibilityLine, AccessibilityNode, AccessibilityProvider, AccessibilitySnapshot,
    AccessibilityTruncationReason, Bounds, CaptureContext, CaptureError, CaptureTrigger,
};

/// Dystil-backed tree walker at the dependency edge of `dystil-capture`.
/// The blocking platform AX call is isolated from the async capture runtime.
pub struct DystilAccessibilityProvider {
    // Keep the AT-SPI connection and event registration alive. Chromium may
    // only materialize its accessibility tree while an assistive technology
    // client remains connected; constructing a fresh walker for every click
    // made that registration effectively transient.
    walker: Arc<Mutex<Box<dyn TreeWalkerPlatform>>>,
    visible_relevant_projection: bool,
}

impl DystilAccessibilityProvider {
    pub fn new(config: TreeWalkerConfig) -> Self {
        Self {
            walker: Arc::new(Mutex::new(create_tree_walker(config))),
            visible_relevant_projection: false,
        }
    }

    /// Candidate-only projection for the local capture harness. The platform
    /// still performs the same walk; this only changes the stored text and
    /// fingerprints derived from already-captured nodes.
    pub fn with_visible_relevant_projection(mut self) -> Self {
        self.visible_relevant_projection = true;
        self
    }
}

#[async_trait]
impl AccessibilityProvider for DystilAccessibilityProvider {
    async fn capture(
        &self,
        _trigger: &CaptureTrigger,
    ) -> Result<Option<AccessibilitySnapshot>, CaptureError> {
        #[cfg(feature = "debug-capture")]
        let diagnostic_started = Instant::now();
        #[cfg(feature = "debug-capture")]
        let trigger_name = _trigger.as_str();
        let walker = Arc::clone(&self.walker);
        let visible_relevant_projection = self.visible_relevant_projection;
        let result = tokio::task::spawn_blocking(move || {
            let walker = walker.lock().map_err(|error| {
                CaptureError::Accessibility(format!("accessibility walker lock poisoned: {error}"))
            })?;
            match walker.walk_focused_window() {
                Ok(TreeWalkResult::Found(snapshot)) => {
                    #[cfg(feature = "debug-capture")]
                    let conversion_rss_before = crate::debug_capture::process_rss_bytes();
                    #[cfg(feature = "debug-capture")]
                    let conversion_started = Instant::now();
                    let mut snapshot = convert_tree_snapshot(snapshot);
                    #[cfg(feature = "debug-capture")]
                    crate::debug_capture::record_capture_phase(
                        "snapshot_node_conversion",
                        trigger_name,
                        conversion_started,
                        snapshot.context.application.as_deref(),
                        Some(snapshot.node_count),
                        Some(snapshot.text.len()),
                        Some(snapshot.truncated),
                        Some(match snapshot.truncation_reason {
                            AccessibilityTruncationReason::None => "none",
                            AccessibilityTruncationReason::Timeout => "timeout",
                            AccessibilityTruncationReason::MaxNodes => "max_nodes",
                        }),
                        conversion_rss_before,
                        crate::debug_capture::process_rss_bytes(),
                    );
                    if visible_relevant_projection {
                        #[cfg(feature = "debug-capture")]
                        let projection_rss_before = crate::debug_capture::process_rss_bytes();
                        #[cfg(feature = "debug-capture")]
                        let projection_started = Instant::now();
                        snapshot.text = project_visible_relevant_text(&snapshot.nodes);
                        #[cfg(feature = "debug-capture")]
                        crate::debug_capture::record_capture_phase(
                            "visible_relevant_projection",
                            trigger_name,
                            projection_started,
                            snapshot.context.application.as_deref(),
                            Some(snapshot.node_count),
                            Some(snapshot.text.len()),
                            Some(snapshot.truncated),
                            Some(match snapshot.truncation_reason {
                                AccessibilityTruncationReason::None => "none",
                                AccessibilityTruncationReason::Timeout => "timeout",
                                AccessibilityTruncationReason::MaxNodes => "max_nodes",
                            }),
                            projection_rss_before,
                            crate::debug_capture::process_rss_bytes(),
                        );
                        #[cfg(feature = "debug-capture")]
                        let hashing_rss_before = crate::debug_capture::process_rss_bytes();
                        #[cfg(feature = "debug-capture")]
                        let hashing_started = Instant::now();
                        snapshot.content_hash = TreeSnapshot::compute_hash(&snapshot.text);
                        snapshot.simhash = TreeSnapshot::compute_simhash(&snapshot.text);
                        #[cfg(feature = "debug-capture")]
                        crate::debug_capture::record_capture_phase(
                            "hashing",
                            trigger_name,
                            hashing_started,
                            snapshot.context.application.as_deref(),
                            Some(snapshot.node_count),
                            Some(snapshot.text.len()),
                            Some(snapshot.truncated),
                            Some(match snapshot.truncation_reason {
                                AccessibilityTruncationReason::None => "none",
                                AccessibilityTruncationReason::Timeout => "timeout",
                                AccessibilityTruncationReason::MaxNodes => "max_nodes",
                            }),
                            hashing_rss_before,
                            crate::debug_capture::process_rss_bytes(),
                        );
                    }
                    Ok(Some(snapshot))
                }
                Ok(TreeWalkResult::Skipped(reason)) => {
                    debug!(?reason, "accessibility capture skipped focused window");
                    Ok(None)
                }
                Ok(TreeWalkResult::NotFound) => {
                    debug!("accessibility capture found no focused window");
                    Ok(None)
                }
                Err(error) => Err(CaptureError::Accessibility(error.to_string())),
            }
        })
        .await
        .map_err(|error| CaptureError::Accessibility(error.to_string()))?;
        #[cfg(feature = "debug-capture")]
        match &result {
            Ok(Some(snapshot)) => crate::debug_capture::record_accessibility_attempt(
                _trigger,
                diagnostic_started,
                Some(snapshot),
                "found",
                None,
            ),
            Ok(None) => crate::debug_capture::record_accessibility_attempt(
                _trigger,
                diagnostic_started,
                None,
                "no_snapshot",
                None,
            ),
            Err(error) => crate::debug_capture::record_accessibility_attempt(
                _trigger,
                diagnostic_started,
                None,
                "error",
                Some(&error.to_string()),
            ),
        }
        result
    }
}

pub fn convert_tree_snapshot(snapshot: TreeSnapshot) -> AccessibilitySnapshot {
    AccessibilitySnapshot {
        captured_at: snapshot.timestamp,
        context: CaptureContext {
            application: non_empty(snapshot.app_name),
            window: non_empty(snapshot.window_name),
            browser_url: snapshot.browser_url,
            document_path: snapshot.document_path,
            display_id: None,
            monitor_id: None,
            device_name: None,
            focused: Some(true),
            target: None,
        },
        text: snapshot.text_content,
        nodes: snapshot.nodes.into_iter().map(convert_node).collect(),
        node_count: snapshot.node_count,
        walk_duration_ms: snapshot.walk_duration.as_millis().min(u64::MAX as u128) as u64,
        content_hash: snapshot.content_hash,
        simhash: snapshot.simhash,
        truncated: snapshot.truncated,
        truncation_reason: convert_truncation_reason(snapshot.truncation_reason),
        max_depth_reached: snapshot.max_depth_reached,
    }
}

fn convert_node(node: AccessibilityTreeNode) -> AccessibilityNode {
    AccessibilityNode {
        node_id: node.node_id,
        parent_node_id: node.parent_node_id,
        role: node.role,
        text: node.text,
        depth: node.depth,
        bounds: node.bounds.map(convert_bounds),
        on_screen: node.on_screen,
        lines: node
            .lines
            .map(|lines| lines.into_iter().map(convert_line).collect()),
        automation_id: node.automation_id,
        class_name: node.class_name,
        value: node.value,
        help_text: node.help_text,
        url: node.url,
        placeholder: node.placeholder,
        role_description: node.role_description,
        subrole: node.subrole,
        dom_identifier: node.dom_identifier,
        dom_classes: node.dom_classes,
        is_enabled: node.is_enabled,
        is_focused: node.is_focused,
        is_selected: node.is_selected,
        is_expanded: node.is_expanded,
        is_password: node.is_password,
        is_keyboard_focusable: node.is_keyboard_focusable,
        accelerator_key: node.accelerator_key,
        access_key: node.access_key,
    }
}

fn convert_line(line: LineSpan) -> AccessibilityLine {
    AccessibilityLine {
        char_start: line.char_start,
        char_count: line.char_count,
        bounds: convert_bounds(line.bounds),
    }
}

fn convert_bounds(bounds: NodeBounds) -> Bounds {
    Bounds {
        left: bounds.left,
        top: bounds.top,
        width: bounds.width,
        height: bounds.height,
    }
}

fn convert_truncation_reason(reason: TruncationReason) -> AccessibilityTruncationReason {
    match reason {
        TruncationReason::None => AccessibilityTruncationReason::None,
        TruncationReason::Timeout => AccessibilityTruncationReason::Timeout,
        TruncationReason::MaxNodes => AccessibilityTruncationReason::MaxNodes,
    }
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

/// Build the candidate's canonical `frame_text` from a completed UIA walk.
/// Unknown geometry remains visible (fail-open); explicit off-screen content
/// and content clipped by a credible Windows viewport do not enter the text.
/// Duplicate strings survive unless they describe the same element or the
/// same ancestor/descendant UIA lineage.
pub(crate) fn project_visible_relevant_text(nodes: &[AccessibilityNode]) -> String {
    let index_by_id: HashMap<u32, usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.node_id != 0)
        .map(|(index, node)| (node.node_id, index))
        .collect();
    let mut emitted: Vec<usize> = Vec::new();
    let mut lines = Vec::new();

    for (index, node) in nodes.iter().enumerate() {
        let text = node.text.trim();
        if text.is_empty() || node.on_screen == Some(false) {
            continue;
        }
        if !visible_after_windows_viewports(index, nodes, &index_by_id) {
            continue;
        }
        if emitted
            .iter()
            .any(|previous| same_text_lineage_or_element(index, *previous, nodes, &index_by_id))
        {
            continue;
        }

        let mut line = text.to_string();
        if node.is_selected == Some(true) {
            line.push_str(" [selected]");
        }
        if node.is_focused == Some(true) {
            line.push_str(" [focused]");
        }
        emitted.push(index);
        lines.push(line);
    }
    lines.join("\n")
}

fn visible_after_windows_viewports(
    index: usize,
    nodes: &[AccessibilityNode],
    index_by_id: &HashMap<u32, usize>,
) -> bool {
    // Explicit off-screen is handled by the caller. If geometry is missing,
    // preserve the node rather than pretending it was invisible.
    let Some(mut visible) = ProjectionRect::from_bounds(nodes[index].bounds.as_ref()) else {
        return true;
    };
    let mut cursor = index;
    let mut visited = HashSet::new();
    while let Some(parent_id) = nodes[cursor].parent_node_id {
        if !visited.insert(parent_id) {
            return true;
        }
        let Some(parent_index) = index_by_id.get(&parent_id).copied() else {
            return true;
        };
        let parent = &nodes[parent_index];
        if is_windows_viewport(parent) {
            if let Some(viewport) = ProjectionRect::from_bounds(parent.bounds.as_ref()) {
                if viewport.credible() {
                    let Some(intersection) = visible.intersection(viewport) else {
                        return false;
                    };
                    visible = intersection;
                }
            }
        }
        cursor = parent_index;
    }
    true
}

fn same_text_lineage_or_element(
    current: usize,
    previous: usize,
    nodes: &[AccessibilityNode],
    index_by_id: &HashMap<u32, usize>,
) -> bool {
    if nodes[current].text.trim() != nodes[previous].text.trim() {
        return false;
    }
    let same_stable_element = nodes[current]
        .automation_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .zip(
            nodes[previous]
                .automation_id
                .as_deref()
                .filter(|value| !value.is_empty()),
        )
        .is_some_and(|(left, right)| left == right);
    same_stable_element
        || bounds_equivalent(
            nodes[current].bounds.as_ref(),
            nodes[previous].bounds.as_ref(),
        )
        || are_related(current, previous, nodes, index_by_id)
}

fn are_related(
    first: usize,
    second: usize,
    nodes: &[AccessibilityNode],
    index_by_id: &HashMap<u32, usize>,
) -> bool {
    is_ancestor(first, second, nodes, index_by_id) || is_ancestor(second, first, nodes, index_by_id)
}

fn is_ancestor(
    ancestor: usize,
    mut descendant: usize,
    nodes: &[AccessibilityNode],
    index_by_id: &HashMap<u32, usize>,
) -> bool {
    let ancestor_id = nodes[ancestor].node_id;
    if ancestor_id == 0 {
        return false;
    }
    let mut visited = HashSet::new();
    while let Some(parent_id) = nodes[descendant].parent_node_id {
        if !visited.insert(parent_id) {
            return false;
        }
        if parent_id == ancestor_id {
            return true;
        }
        let Some(parent) = index_by_id.get(&parent_id).copied() else {
            return false;
        };
        descendant = parent;
    }
    false
}

fn bounds_equivalent(left: Option<&Bounds>, right: Option<&Bounds>) -> bool {
    let (Some(left), Some(right)) = (left, right) else {
        return false;
    };
    const EPSILON: f32 = 0.002;
    (left.left - right.left).abs() <= EPSILON
        && (left.top - right.top).abs() <= EPSILON
        && (left.width - right.width).abs() <= EPSILON
        && (left.height - right.height).abs() <= EPSILON
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
struct ProjectionRect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl ProjectionRect {
    fn from_bounds(bounds: Option<&Bounds>) -> Option<Self> {
        let bounds = bounds?;
        (bounds.width > 0.0 && bounds.height > 0.0).then_some(Self {
            left: bounds.left,
            top: bounds.top,
            right: bounds.left + bounds.width,
            bottom: bounds.top + bounds.height,
        })
    }

    fn credible(self) -> bool {
        self.right - self.left >= 0.01 && self.bottom - self.top >= 0.01
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
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;

    fn projection_node(
        node_id: u32,
        parent_node_id: Option<u32>,
        role: &str,
        text: &str,
        on_screen: Option<bool>,
        bounds: Option<Bounds>,
    ) -> AccessibilityNode {
        AccessibilityNode {
            node_id,
            parent_node_id,
            role: role.to_string(),
            text: text.to_string(),
            depth: 0,
            bounds,
            on_screen,
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

    #[test]
    fn visible_projection_removes_lineage_duplicates_but_keeps_unrelated_equal_text() {
        let nodes = vec![
            projection_node(1, None, "Group", "Invoice 42", Some(true), None),
            projection_node(2, Some(1), "Text", "Invoice 42", Some(true), None),
            projection_node(3, None, "DataItem", "Invoice 42", Some(true), None),
        ];
        assert_eq!(
            project_visible_relevant_text(&nodes),
            "Invoice 42\nInvoice 42"
        );
    }

    #[test]
    fn visible_projection_drops_explicit_or_viewport_clipped_text_but_keeps_unknown() {
        let nodes = vec![
            projection_node(
                1,
                None,
                "Document",
                "",
                Some(true),
                Some(bounds(0.0, 0.0, 0.4, 0.4)),
            ),
            projection_node(
                2,
                Some(1),
                "Text",
                "clipped",
                Some(true),
                Some(bounds(0.7, 0.7, 0.1, 0.1)),
            ),
            projection_node(3, None, "Text", "offscreen", Some(false), None),
            projection_node(4, None, "Text", "unknown geometry", None, None),
        ];
        assert_eq!(project_visible_relevant_text(&nodes), "unknown geometry");
    }

    #[test]
    fn visible_projection_retains_selected_and_focused_state() {
        let mut selected = projection_node(1, None, "TabItem", "Inbox", Some(true), None);
        selected.is_selected = Some(true);
        let mut focused = projection_node(2, None, "Edit", "search terms", Some(true), None);
        focused.is_focused = Some(true);
        assert_eq!(
            project_visible_relevant_text(&[selected, focused]),
            "Inbox [selected]\nsearch terms [focused]"
        );
    }

    #[test]
    fn conversion_preserves_context_quality_and_all_node_fields() {
        let timestamp = Utc::now();
        let source = TreeSnapshot {
            app_name: "Code".to_string(),
            window_name: "matcher.rs".to_string(),
            text_content: "focused content".to_string(),
            nodes: vec![AccessibilityTreeNode {
                node_id: 7,
                parent_node_id: Some(3),
                role: "TextField".to_string(),
                text: "query".to_string(),
                depth: 3,
                bounds: Some(NodeBounds {
                    left: 0.1,
                    top: 0.2,
                    width: 0.3,
                    height: 0.4,
                }),
                on_screen: Some(true),
                lines: Some(vec![LineSpan {
                    char_start: 1,
                    char_count: 4,
                    bounds: NodeBounds {
                        left: 0.11,
                        top: 0.21,
                        width: 0.2,
                        height: 0.1,
                    },
                }]),
                automation_id: Some("search".to_string()),
                class_name: Some("input".to_string()),
                value: Some("query".to_string()),
                help_text: Some("Search".to_string()),
                url: Some("https://example.com".to_string()),
                placeholder: Some("Find".to_string()),
                role_description: Some("edit".to_string()),
                subrole: Some("search".to_string()),
                dom_identifier: Some("search-input".to_string()),
                dom_classes: Some("composer active".to_string()),
                is_enabled: Some(true),
                is_focused: Some(true),
                is_selected: Some(false),
                is_expanded: Some(false),
                is_password: Some(false),
                is_keyboard_focusable: Some(true),
                accelerator_key: Some("Ctrl+F".to_string()),
                access_key: Some("F".to_string()),
            }],
            browser_url: Some("https://example.com".to_string()),
            document_path: Some("/tmp/matcher.rs".to_string()),
            timestamp,
            node_count: 42,
            walk_duration: Duration::from_millis(275),
            content_hash: 11,
            simhash: 22,
            truncated: true,
            truncation_reason: TruncationReason::Timeout,
            max_depth_reached: 9,
        };

        let converted = convert_tree_snapshot(source);

        assert_eq!(converted.captured_at, timestamp);
        assert_eq!(converted.context.application.as_deref(), Some("Code"));
        assert_eq!(converted.context.window.as_deref(), Some("matcher.rs"));
        assert_eq!(converted.node_count, 42);
        assert_eq!(converted.walk_duration_ms, 275);
        assert_eq!(
            converted.truncation_reason,
            AccessibilityTruncationReason::Timeout
        );
        assert_eq!(converted.max_depth_reached, 9);

        let node = &converted.nodes[0];
        assert_eq!(node.node_id, 7);
        assert_eq!(node.parent_node_id, Some(3));
        assert_eq!(node.automation_id.as_deref(), Some("search"));
        assert_eq!(node.help_text.as_deref(), Some("Search"));
        assert_eq!(node.role_description.as_deref(), Some("edit"));
        assert_eq!(node.dom_identifier.as_deref(), Some("search-input"));
        assert_eq!(node.is_keyboard_focusable, Some(true));
        assert_eq!(node.accelerator_key.as_deref(), Some("Ctrl+F"));
        assert_eq!(node.lines.as_ref().unwrap()[0].char_count, 4);
    }

    #[test]
    fn empty_app_and_window_names_become_absent_context() {
        let source = TreeSnapshot {
            app_name: "  ".to_string(),
            window_name: String::new(),
            text_content: String::new(),
            nodes: vec![],
            browser_url: None,
            document_path: None,
            timestamp: Utc::now(),
            node_count: 0,
            walk_duration: Duration::ZERO,
            content_hash: 0,
            simhash: 0,
            truncated: false,
            truncation_reason: TruncationReason::None,
            max_depth_reached: 0,
        };

        let converted = convert_tree_snapshot(source);
        assert_eq!(converted.context.application, None);
        assert_eq!(converted.context.window, None);
    }
}
