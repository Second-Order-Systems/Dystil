use crate::a11y::tree::{
    create_tree_walker, AccessibilityTreeNode, LineSpan, NodeBounds, TreeSnapshot, TreeWalkResult,
    TreeWalkerConfig, TreeWalkerPlatform, TruncationReason,
};
use async_trait::async_trait;
use std::sync::{Arc, Mutex};
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
}

impl DystilAccessibilityProvider {
    pub fn new(config: TreeWalkerConfig) -> Self {
        Self {
            walker: Arc::new(Mutex::new(create_tree_walker(config))),
        }
    }
}

#[async_trait]
impl AccessibilityProvider for DystilAccessibilityProvider {
    async fn capture(
        &self,
        _trigger: &CaptureTrigger,
    ) -> Result<Option<AccessibilitySnapshot>, CaptureError> {
        let walker = Arc::clone(&self.walker);
        tokio::task::spawn_blocking(move || {
            let walker = walker.lock().map_err(|error| {
                CaptureError::Accessibility(format!("accessibility walker lock poisoned: {error}"))
            })?;
            match walker.walk_focused_window() {
                Ok(TreeWalkResult::Found(snapshot)) => Ok(Some(convert_tree_snapshot(snapshot))),
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
        .map_err(|error| CaptureError::Accessibility(error.to_string()))?
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

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;

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
