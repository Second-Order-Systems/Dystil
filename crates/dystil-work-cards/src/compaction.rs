use std::collections::{HashMap, HashSet};

use dystil_protocol::{SegmentEvidenceItem, SegmentEvidenceKind};
use regex::Regex;
use sha2::{Digest, Sha256};

use crate::{CompactedEvidence, EvidenceWindow};

#[derive(Debug, Clone)]
pub struct CompactionConfig {
    pub max_tokens: u32,
    pub max_item_tokens: usize,
    pub context_tokens_around_change: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4_000,
            max_item_tokens: 160,
            context_tokens_around_change: 5,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CompactionStats {
    pub source_items: usize,
    pub kept_items: usize,
    pub duplicate_items: usize,
    pub source_estimated_tokens: u32,
    pub compacted_estimated_tokens: u32,
    pub truncated: bool,
}

#[derive(Debug)]
struct Candidate {
    evidence: CompactedEvidence,
    score: i32,
    ordinal: usize,
}

pub fn compact_window(
    window: &EvidenceWindow,
    config: &CompactionConfig,
) -> (Vec<CompactedEvidence>, CompactionStats) {
    let mut stats = CompactionStats {
        source_items: window.items.len(),
        source_estimated_tokens: window
            .items
            .iter()
            .map(|item| estimate_tokens(&item.text))
            .sum(),
        ..CompactionStats::default()
    };
    let mut last_text_by_surface: HashMap<String, String> = HashMap::new();
    let mut exact_seen: HashSet<String> = HashSet::new();
    let mut candidates = Vec::new();

    for (ordinal, item) in window.items.iter().enumerate() {
        let normalized = normalize(&item.text);
        if normalized.is_empty() {
            continue;
        }
        let surface = surface_key(item);
        let exact_key = format!("{}|{}|{}", kind_name(&item.kind), surface, normalized);
        if !exact_seen.insert(exact_key) {
            stats.duplicate_items += 1;
            continue;
        }

        let previous = last_text_by_surface.get(&surface).map(String::as_str);
        let mut compacted = match item.kind {
            SegmentEvidenceKind::Input => normalized.clone(),
            SegmentEvidenceKind::Screen => compact_screen_text(
                previous,
                &normalized,
                config.max_item_tokens,
                config.context_tokens_around_change,
            ),
        };
        last_text_by_surface.insert(surface, normalized);
        if compacted.is_empty() {
            stats.duplicate_items += 1;
            continue;
        }
        compacted = cap_tokens(&compacted, config.max_item_tokens);
        let estimated_tokens = estimate_tokens(&compacted);
        let evidence_id = compacted_id(item, &compacted);
        let mut score = match item.kind {
            SegmentEvidenceKind::Input => 100,
            SegmentEvidenceKind::Screen => 40,
        };
        if item
            .browser_url
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        {
            score += 25;
        }
        if item
            .metadata
            .get("document_path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
        {
            score += 25;
        }
        if contains_artifact_like_token(&compacted) {
            score += 20;
        }
        // The beginning establishes context; the end is essential for resume.
        if ordinal < 3 || ordinal + 3 >= window.items.len() {
            score += 35;
        }
        candidates.push(Candidate {
            evidence: CompactedEvidence {
                evidence_id,
                occurred_at: item.occurred_at,
                kind: item.kind.clone(),
                app_name: item.app_name.clone(),
                window_name: item.window_name.clone(),
                browser_url: item.browser_url.clone(),
                text: compacted,
                source_ids: vec![item.item_id.clone()],
                estimated_tokens,
            },
            score,
            ordinal,
        });
    }

    let total: u32 = candidates
        .iter()
        .map(|candidate| candidate.evidence.estimated_tokens)
        .sum();
    let selected = if total <= config.max_tokens {
        candidates
    } else {
        stats.truncated = true;
        select_with_budget(candidates, config.max_tokens)
    };
    let mut selected = selected;
    selected.sort_by_key(|candidate| candidate.ordinal);
    let evidence = selected
        .into_iter()
        .map(|candidate| candidate.evidence)
        .collect::<Vec<_>>();
    stats.kept_items = evidence.len();
    stats.compacted_estimated_tokens = evidence.iter().map(|item| item.estimated_tokens).sum();
    (evidence, stats)
}

fn select_with_budget(mut candidates: Vec<Candidate>, budget: u32) -> Vec<Candidate> {
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    let mut selected = Vec::new();
    let mut used = 0_u32;
    for candidate in candidates {
        if used.saturating_add(candidate.evidence.estimated_tokens) <= budget {
            used += candidate.evidence.estimated_tokens;
            selected.push(candidate);
        }
    }
    selected
}

fn compact_screen_text(
    previous: Option<&str>,
    current: &str,
    max_tokens: usize,
    radius: usize,
) -> String {
    let Some(previous) = previous else {
        return cap_tokens(current, max_tokens);
    };
    if previous == current {
        return String::new();
    }
    let previous_counts = token_counts(previous);
    let tokens = current.split_whitespace().collect::<Vec<_>>();
    let mut consumed: HashMap<String, usize> = HashMap::new();
    let mut changed = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        let key = normalized_token(token);
        let count = consumed.entry(key.clone()).or_default();
        *count += 1;
        if *count > previous_counts.get(&key).copied().unwrap_or_default() {
            changed.push(index);
        }
    }
    if changed.is_empty() {
        return cap_tokens(current, max_tokens.min(48));
    }
    let mut spans = Vec::new();
    for index in changed {
        let start = index.saturating_sub(radius);
        let end = (index + radius + 1).min(tokens.len());
        if let Some((_, last_end)) = spans.last_mut() {
            if start <= *last_end {
                *last_end = (*last_end).max(end);
                continue;
            }
        }
        spans.push((start, end));
    }
    let mut output = Vec::new();
    for (span_index, (start, end)) in spans.into_iter().enumerate() {
        if span_index > 0 {
            output.push("…".to_string());
        }
        output.extend(tokens[start..end].iter().map(|value| (*value).to_string()));
        if output.len() >= max_tokens {
            break;
        }
    }
    output.truncate(max_tokens);
    output.join(" ")
}

fn token_counts(value: &str) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for token in value.split_whitespace() {
        *counts.entry(normalized_token(token)).or_default() += 1;
    }
    counts
}

fn normalized_token(value: &str) -> String {
    value
        .trim_matches(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .to_lowercase()
}

fn surface_key(item: &SegmentEvidenceItem) -> String {
    format!(
        "{}|{}|{}",
        item.app_name.as_deref().unwrap_or_default(),
        item.window_name.as_deref().unwrap_or_default(),
        item.browser_url.as_deref().unwrap_or_default()
    )
}

fn compacted_id(item: &SegmentEvidenceItem, text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(item.item_id.as_bytes());
    hasher.update(b"|");
    hasher.update(text.as_bytes());
    let digest = hex::encode(hasher.finalize());
    format!("cev_{}", &digest[..20])
}

fn kind_name(kind: &SegmentEvidenceKind) -> &'static str {
    match kind {
        SegmentEvidenceKind::Screen => "screen",
        SegmentEvidenceKind::Input => "input",
    }
}

pub fn estimate_tokens(value: &str) -> u32 {
    (value.chars().count() as u32).div_ceil(4).max(1)
}

fn cap_tokens(value: &str, max_tokens: usize) -> String {
    value
        .split_whitespace()
        .take(max_tokens)
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn contains_artifact_like_token(value: &str) -> bool {
    // Compiled per call today because this path is dominated by model input
    // preparation, not regex matching. Keep the patterns conservative.
    let pattern = Regex::new(
        r"(?ix)(https?://|(?:^|\s)[A-Z]{2,10}-\d+|\bPR\s*\#?\d+|[\w.-]+\.(?:rs|ts|tsx|js|jsx|py|go|java|sql|md|json|ya?ml)\b)",
    )
    .expect("valid artifact regex");
    pattern.is_match(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_delta_keeps_changed_terms_with_context() {
        let previous = "sidebar home settings editor auth callback loading footer";
        let current = "sidebar home settings editor auth callback OAuth state mismatch footer";
        let result = compact_screen_text(Some(previous), current, 40, 2);
        assert!(result.contains("OAuth state mismatch"));
        assert!(!result.starts_with("sidebar home"));
    }

    #[test]
    fn artifact_detector_covers_workplace_identifiers() {
        assert!(contains_artifact_like_token("opened INC-912 and auth.ts"));
        assert!(contains_artifact_like_token("reviewed PR #184"));
        assert!(!contains_artifact_like_token("generic navigation text"));
    }
}
