use crate::{CompactedEvidence, ReducedEvidenceWindow};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub target_tokens: u32,
    pub hard_max_tokens: u32,
    pub overlap_tokens: u32,
}
impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            target_tokens: 4_000,
            hard_max_tokens: 6_000,
            overlap_tokens: 400,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct EvidenceChunk {
    pub chunk_id: String,
    pub window_id: String,
    pub start_index: usize,
    pub end_index: usize,
    pub estimated_tokens: u32,
    pub evidence: Vec<CompactedEvidence>,
}
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ChunkingStats {
    pub chunks: usize,
    pub total_input_tokens: u32,
    pub overlap_tokens: u32,
}

pub fn chunk_reduced_window(
    window: &ReducedEvidenceWindow,
    config: &ChunkConfig,
) -> (Vec<EvidenceChunk>, ChunkingStats) {
    let source = &window.evidence;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < source.len() {
        let mut end = start;
        let mut tokens = 0u32;
        while end < source.len() {
            let next = source[end].estimated_tokens;
            if end > start && tokens.saturating_add(next) > config.target_tokens {
                break;
            }
            tokens = tokens.saturating_add(next);
            end += 1;
            if tokens >= config.hard_max_tokens {
                break;
            }
        }
        if end == start {
            end += 1;
            tokens = source[start].estimated_tokens;
        }
        let evidence = source[start..end].to_vec();
        let mut hasher = Sha256::new();
        hasher.update(window.window.window_id.as_bytes());
        hasher.update(b"|");
        hasher.update(start.to_le_bytes());
        hasher.update(end.to_le_bytes());
        for item in &evidence {
            hasher.update(item.evidence_id.as_bytes());
        }
        chunks.push(EvidenceChunk {
            chunk_id: format!("chk_{}", &hex::encode(hasher.finalize())[..20]),
            window_id: window.window.window_id.clone(),
            start_index: start,
            end_index: end,
            estimated_tokens: tokens,
            evidence,
        });
        if end >= source.len() {
            break;
        }
        let mut overlap = 0;
        let mut next_start = end;
        while next_start > start && overlap < config.overlap_tokens {
            next_start -= 1;
            overlap += source[next_start].estimated_tokens;
        }
        start = if next_start == start { end } else { next_start };
    }
    let total_input_tokens = chunks.iter().map(|c| c.estimated_tokens).sum();
    let count = chunks.len();
    (
        chunks,
        ChunkingStats {
            chunks: count,
            total_input_tokens,
            overlap_tokens: config.overlap_tokens,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dystil_protocol::SegmentEvidenceKind;
    #[test]
    fn chunking_is_ordered_and_overlaps() {
        let reduced = ReducedEvidenceWindow {
            window: crate::EvidenceWindow {
                window_id: "w".into(),
                device_id: "d".into(),
                start_time: Utc::now(),
                end_time: Utc::now(),
                close_reason: "x".into(),
                segment_ids: vec![],
                items: vec![],
            },
            evidence: (0..8)
                .map(|n| CompactedEvidence {
                    evidence_id: format!("e{n}"),
                    occurred_at: Utc::now(),
                    kind: SegmentEvidenceKind::Input,
                    app_name: None,
                    window_name: None,
                    browser_url: None,
                    text: "x".into(),
                    source_ids: vec![],
                    estimated_tokens: 100,
                })
                .collect(),
            stats: Default::default(),
        };
        let (chunks, _) = chunk_reduced_window(
            &reduced,
            &ChunkConfig {
                target_tokens: 300,
                hard_max_tokens: 400,
                overlap_tokens: 100,
            },
        );
        assert_eq!(
            chunks[0].evidence.last().unwrap().evidence_id,
            chunks[1].evidence.first().unwrap().evidence_id
        );
        assert!(chunks
            .windows(2)
            .all(|w| w[0].start_index < w[1].start_index));
    }
}
