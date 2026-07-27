use crate::{compact_window, CompactedEvidence, CompactionConfig, EvidenceWindow};

#[derive(Debug, Clone)]
pub struct PreBudgetReductionConfig {
    pub max_item_tokens: usize,
    pub context_tokens_around_change: usize,
}

impl Default for PreBudgetReductionConfig {
    fn default() -> Self {
        Self {
            max_item_tokens: 160,
            context_tokens_around_change: 5,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PreBudgetReductionStats {
    pub source_items: usize,
    pub source_estimated_tokens: u32,
    pub duplicate_items: usize,
    pub remaining_items: usize,
    pub remaining_estimated_tokens: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReducedEvidenceWindow {
    pub window: EvidenceWindow,
    pub evidence: Vec<CompactedEvidence>,
    pub stats: PreBudgetReductionStats,
}

/// Performs the frozen compaction normalization/deduplication/delta stages but
/// deliberately omits global importance selection.  This is separate from
/// `compact_window` so existing compacted files remain byte-for-byte stable.
pub fn reduce_window_before_budget(
    window: &EvidenceWindow,
    config: &PreBudgetReductionConfig,
) -> ReducedEvidenceWindow {
    let (evidence, compact_stats) = compact_window(
        window,
        &CompactionConfig {
            max_tokens: u32::MAX,
            max_item_tokens: config.max_item_tokens,
            context_tokens_around_change: config.context_tokens_around_change,
        },
    );
    ReducedEvidenceWindow {
        window: window.clone(),
        stats: PreBudgetReductionStats {
            source_items: compact_stats.source_items,
            source_estimated_tokens: compact_stats.source_estimated_tokens,
            duplicate_items: compact_stats.duplicate_items,
            remaining_items: evidence.len(),
            remaining_estimated_tokens: evidence.iter().map(|item| item.estimated_tokens).sum(),
        },
        evidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WindowConfig;
    use chrono::{Duration, Utc};
    use dystil_protocol::{SegmentEvidenceItem, SegmentEvidenceKind};

    #[test]
    fn prebudget_never_applies_global_selection() {
        let now = Utc::now();
        let items = (0..120)
            .map(|index| SegmentEvidenceItem {
                item_id: format!("i{index}"),
                occurred_at: now + Duration::seconds(index),
                kind: SegmentEvidenceKind::Input,
                app_name: None,
                window_name: None,
                browser_url: None,
                text: format!("{index} {}", "x ".repeat(80)),
                metadata: Default::default(),
                source_id: "source".into(),
                source_payload_hash: "hash".into(),
            })
            .collect();
        let window = EvidenceWindow {
            window_id: "w".into(),
            device_id: "d".into(),
            start_time: now,
            end_time: now,
            close_reason: "test".into(),
            segment_ids: vec![],
            items,
        };
        let reduced = reduce_window_before_budget(&window, &Default::default());
        assert!(reduced.stats.remaining_estimated_tokens > 4_000);
        let _ = WindowConfig::default();
    }
}
