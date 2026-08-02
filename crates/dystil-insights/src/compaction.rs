use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceActivity {
    pub evidence_id: String,
    pub occurred_at: DateTime<Utc>,
    pub app: Option<String>,
    pub window: Option<String>,
    pub url: Option<String>,
    pub text: String,
    pub content_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactActivity {
    pub evidence_id: String,
    pub occurred_at: DateTime<Utc>,
    pub app: Option<String>,
    pub window: Option<String>,
    pub added: Vec<String>,
    pub reappeared: Vec<String>,
    pub omitted_lines: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct CompactionConfig {
    pub episode_gap_seconds: i64,
    pub max_record_chars: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            episode_gap_seconds: 10 * 60,
            max_record_chars: 12_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SurfaceState {
    lines: HashSet<String>,
    normalized: String,
    content_hash: Option<String>,
    last_seen: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompactionState {
    surfaces: HashMap<String, SurfaceState>,
    seen_by_surface: HashMap<String, HashSet<String>>,
}

fn normalize_lines(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| !line.is_empty() && seen.insert(line.clone()))
        .collect()
}

fn surface_key(record: &SourceActivity) -> String {
    format!(
        "{}\n{}\n{}",
        record.app.as_deref().unwrap_or("unknown"),
        record.window.as_deref().unwrap_or("unknown"),
        record.url.as_deref().unwrap_or("")
    )
}

fn take_budget(lines: impl IntoIterator<Item = String>, budget: usize) -> (Vec<String>, usize) {
    let source: Vec<String> = lines.into_iter().collect();
    let mut kept = Vec::new();
    let mut used = 0;
    for line in &source {
        let cost = line.len() + 1;
        if used + cost > budget {
            break;
        }
        used += cost;
        kept.push(line.clone());
    }
    let omitted = source.len() - kept.len();
    (kept, omitted)
}

/// Compacts source activity while preserving bounded evidence that content
/// disappeared and later reappeared on the same semantic surface.
pub fn compact_activity(
    records: &[SourceActivity],
    config: CompactionConfig,
) -> Vec<CompactActivity> {
    compact_activity_incremental(records, config, &mut CompactionState::default())
}

pub fn compact_activity_incremental(
    records: &[SourceActivity],
    config: CompactionConfig,
    state: &mut CompactionState,
) -> Vec<CompactActivity> {
    let mut output = Vec::new();
    for record in records {
        let key = surface_key(record);
        let lines = normalize_lines(&record.text);
        let normalized = lines.join("\n");
        let stored = state.surfaces.get(&key);
        let separated = stored.is_some_and(|previous| {
            (record.occurred_at - previous.last_seen).num_seconds() >= config.episode_gap_seconds
        });
        let previous = (!separated).then_some(stored).flatten();
        if previous.is_some_and(|previous| {
            previous.normalized == normalized
                || (record.content_hash.is_some() && previous.content_hash == record.content_hash)
        }) {
            continue;
        }
        let previous_lines = previous.map(|item| &item.lines);
        let seen = state.seen_by_surface.entry(key.clone()).or_default();
        let changed = lines
            .iter()
            .filter(|line| previous_lines.is_none_or(|set| !set.contains(*line)))
            .cloned()
            .collect::<Vec<_>>();
        let new_lines = changed
            .iter()
            .filter(|line| !seen.contains(*line))
            .cloned()
            .collect::<Vec<_>>();
        let recurring_lines = changed
            .iter()
            .filter(|line| seen.contains(*line))
            .cloned()
            .collect::<Vec<_>>();
        seen.extend(lines.iter().cloned());
        state.surfaces.insert(
            key,
            SurfaceState {
                lines: lines.iter().cloned().collect(),
                normalized,
                content_hash: record.content_hash.clone(),
                last_seen: record.occurred_at,
            },
        );
        if new_lines.is_empty() && recurring_lines.is_empty() {
            continue;
        }
        let (added, omitted_added) = take_budget(new_lines, config.max_record_chars);
        let used = added.iter().map(|line| line.len() + 1).sum::<usize>();
        let (reappeared, omitted_recurring) = take_budget(
            recurring_lines,
            config.max_record_chars.saturating_sub(used),
        );
        output.push(CompactActivity {
            evidence_id: record.evidence_id.clone(),
            occurred_at: record.occurred_at,
            app: record.app.clone(),
            window: record.window.clone(),
            added,
            reappeared,
            omitted_lines: omitted_added + omitted_recurring,
        });
    }
    output
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn source(id: usize, minute: u32, text: &str) -> SourceActivity {
        SourceActivity {
            evidence_id: format!("frame:{id}"),
            occurred_at: Utc.with_ymd_and_hms(2026, 1, 1, 9, minute, 0).unwrap(),
            app: Some("Editor".into()),
            window: Some("Report".into()),
            url: None,
            text: text.into(),
            content_hash: Some(id.to_string()),
        }
    }

    #[test]
    fn preserves_recurrence_after_content_changes() {
        let compact = compact_activity(
            &[
                source(1, 0, "weekly template"),
                source(2, 1, "other work"),
                source(3, 2, "weekly template"),
            ],
            CompactionConfig::default(),
        );
        assert_eq!(compact[2].reappeared, ["weekly template"]);
    }

    #[test]
    fn resets_episode_state_after_gap_without_forgetting_recurrence() {
        let mut later = source(2, 11, "same checklist");
        later.content_hash = Some("same".into());
        let mut first = source(1, 0, "same checklist");
        first.content_hash = Some("same".into());
        let compact = compact_activity(&[first, later], CompactionConfig::default());
        assert_eq!(compact.len(), 2);
        assert_eq!(compact[1].reappeared, ["same checklist"]);
    }

    #[test]
    fn recurrence_memory_survives_a_durable_checkpoint() {
        let mut state = CompactionState::default();
        compact_activity_incremental(
            &[source(1, 0, "weekly template"), source(2, 1, "other work")],
            CompactionConfig::default(),
            &mut state,
        );
        let mut restored: CompactionState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        let compact = compact_activity_incremental(
            &[source(3, 2, "weekly template")],
            CompactionConfig::default(),
            &mut restored,
        );
        assert_eq!(compact[0].reappeared, ["weekly template"]);
    }
}
