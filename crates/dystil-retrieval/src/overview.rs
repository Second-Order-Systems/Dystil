use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::evidence::evidence_from_record;
use crate::{Evidence, Result, RetrievalError, RetrievalService};

const IDLE_CAP_SECONDS: f64 = 300.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct OverviewRequest {
    pub start_time: String,
    pub end_time: String,
    pub app_name: Option<String>,
    pub max_apps: Option<u32>,
    pub max_windows: Option<u32>,
    pub max_snippets: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DataStatus {
    Ok,
    NoCaptureInRange,
    CaptureStopped,
    SparseCoverage,
    FiltersTooRestrictive,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct UsageEntry {
    pub name: String,
    pub active_minutes: f64,
    pub observations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WindowUsage {
    pub app_name: String,
    pub window_name: String,
    pub browser_url: Option<String>,
    pub active_minutes: f64,
    pub observations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Transition {
    pub from: String,
    pub to: String,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetrievalHealth {
    pub last_frame_at: Option<String>,
    pub last_event_at: Option<String>,
    pub redaction_backlog: u64,
    pub search_documents: u64,
    pub fts_documents: u64,
    pub fts_consistent: bool,
    pub connection_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityOverview {
    pub start_time: String,
    pub end_time: String,
    pub data_status: DataStatus,
    pub estimated_active_minutes: f64,
    pub duration_is_estimated: bool,
    pub first_capture_at: Option<String>,
    pub last_capture_at: Option<String>,
    pub frame_count: u64,
    pub event_count: u64,
    pub coverage_gap_count: u64,
    pub apps: Vec<UsageEntry>,
    pub windows: Vec<WindowUsage>,
    pub transitions: Vec<Transition>,
    pub representative_evidence: Vec<Evidence>,
    pub health: RetrievalHealth,
}

impl RetrievalService {
    pub async fn overview(&self, request: OverviewRequest) -> Result<ActivityOverview> {
        let start = parse_time(&request.start_time, "start_time")?;
        let end = parse_time(&request.end_time, "end_time")?;
        if start >= end {
            return Err(RetrievalError::InvalidRequest(
                "start_time must be before end_time".into(),
            ));
        }
        let raw = dystil_storage::get_activity_overview_raw(
            &self.pool,
            &request.start_time,
            &request.end_time,
            request.app_name.as_deref(),
        )
        .await?;
        let unfiltered_count = if request.app_name.is_some() && raw.frames.is_empty() {
            dystil_storage::count_activity_in_range(
                &self.pool,
                &request.start_time,
                &request.end_time,
            )
            .await?
        } else {
            0
        };

        let mut app_usage: HashMap<String, (f64, u64)> = HashMap::new();
        let mut window_usage: HashMap<(String, String, Option<String>), (f64, u64)> =
            HashMap::new();
        let mut transition_counts: BTreeMap<(String, String), u64> = BTreeMap::new();
        let mut active_seconds = 0.0;
        let mut gap_count = 0u64;
        let mut previous_app: Option<&str> = None;
        for (index, frame) in raw.frames.iter().enumerate() {
            let app = frame.app_name.as_deref().unwrap_or("Unknown");
            let seconds = raw
                .frames
                .get(index + 1)
                .and_then(|next| seconds_between(&frame.timestamp, &next.timestamp))
                .filter(|gap| *gap > 0.0 && *gap < IDLE_CAP_SECONDS)
                .unwrap_or_else(|| {
                    if index + 1 < raw.frames.len() {
                        gap_count += 1;
                    }
                    0.0
                });
            active_seconds += seconds;
            let app_entry = app_usage.entry(app.to_string()).or_default();
            app_entry.0 += seconds;
            app_entry.1 += 1;
            let window_key = (
                app.to_string(),
                frame.window_name.clone().unwrap_or_default(),
                frame.browser_url.clone(),
            );
            let window_entry = window_usage.entry(window_key).or_default();
            window_entry.0 += seconds;
            window_entry.1 += 1;
            if let Some(previous) = previous_app.filter(|previous| *previous != app) {
                *transition_counts
                    .entry((previous.to_string(), app.to_string()))
                    .or_default() += 1;
            }
            previous_app = Some(app);
        }

        let mut apps = app_usage
            .into_iter()
            .map(|(name, (seconds, observations))| UsageEntry {
                name,
                active_minutes: rounded_minutes(seconds),
                observations,
            })
            .collect::<Vec<_>>();
        apps.sort_by(|left, right| {
            right
                .active_minutes
                .total_cmp(&left.active_minutes)
                .then_with(|| right.observations.cmp(&left.observations))
        });
        apps.truncate(request.max_apps.unwrap_or(20).clamp(1, 50) as usize);

        let mut windows = window_usage
            .into_iter()
            .map(
                |((app_name, window_name, browser_url), (seconds, observations))| WindowUsage {
                    app_name,
                    window_name,
                    browser_url,
                    active_minutes: rounded_minutes(seconds),
                    observations,
                },
            )
            .collect::<Vec<_>>();
        windows.sort_by(|left, right| {
            right
                .active_minutes
                .total_cmp(&left.active_minutes)
                .then_with(|| right.observations.cmp(&left.observations))
        });
        windows.truncate(request.max_windows.unwrap_or(30).clamp(1, 60) as usize);

        let mut transitions = transition_counts
            .into_iter()
            .map(|((from, to), count)| Transition { from, to, count })
            .collect::<Vec<_>>();
        transitions.sort_by(|left, right| right.count.cmp(&left.count));
        transitions.truncate(20);

        let data_status = diagnose_status(
            &raw,
            unfiltered_count,
            end,
            gap_count,
            request.app_name.is_some(),
        );
        let snippet_limit = request.max_snippets.unwrap_or(8).clamp(0, 12) as usize;
        let representative_evidence = evenly_spaced(&raw.samples, snippet_limit)
            .into_iter()
            .map(|record| evidence_from_record(record.clone(), 500))
            .collect::<Result<Vec<_>>>()?;
        Ok(ActivityOverview {
            start_time: request.start_time,
            end_time: request.end_time,
            data_status,
            estimated_active_minutes: rounded_minutes(active_seconds),
            duration_is_estimated: true,
            first_capture_at: raw.frames.first().map(|frame| frame.timestamp.clone()),
            last_capture_at: raw.frames.last().map(|frame| frame.timestamp.clone()),
            frame_count: raw.frames.len() as u64,
            event_count: raw.event_count,
            coverage_gap_count: gap_count,
            apps,
            windows,
            transitions,
            representative_evidence,
            health: RetrievalHealth {
                last_frame_at: raw.health.last_frame_at,
                last_event_at: raw.health.last_event_at,
                redaction_backlog: raw.health.redaction_backlog,
                search_documents: raw.health.search_documents,
                fts_documents: raw.health.fts_documents,
                fts_consistent: raw.health.search_documents == raw.health.fts_documents,
                connection_status: "not_configured".into(),
            },
        })
    }
}

fn diagnose_status(
    raw: &dystil_storage::ActivityOverviewRaw,
    unfiltered_count: u64,
    end: DateTime<Utc>,
    gap_count: u64,
    filtered: bool,
) -> DataStatus {
    if raw.frames.is_empty() && filtered && unfiltered_count > 0 {
        return DataStatus::FiltersTooRestrictive;
    }
    if raw.frames.is_empty() && raw.event_count == 0 {
        let near_now = (Utc::now() - end).num_minutes().unsigned_abs() <= 10;
        let last_is_stale = raw
            .health
            .last_frame_at
            .as_deref()
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .is_none_or(|last| (Utc::now() - last.with_timezone(&Utc)).num_minutes() > 10);
        return if near_now && last_is_stale {
            DataStatus::CaptureStopped
        } else {
            DataStatus::NoCaptureInRange
        };
    }
    if raw.frames.len() < 2 || gap_count > 0 {
        DataStatus::SparseCoverage
    } else {
        DataStatus::Ok
    }
}

fn parse_time(value: &str, field: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|_| RetrievalError::InvalidRequest(format!("{field} must be RFC3339")))
}

fn seconds_between(start: &str, end: &str) -> Option<f64> {
    let start = DateTime::parse_from_rfc3339(start).ok()?;
    let end = DateTime::parse_from_rfc3339(end).ok()?;
    Some((end - start).num_milliseconds() as f64 / 1_000.0)
}

fn rounded_minutes(seconds: f64) -> f64 {
    (seconds / 6.0).round() / 10.0
}

fn evenly_spaced<T>(values: &[T], limit: usize) -> Vec<&T> {
    if limit == 0 || values.is_empty() {
        return Vec::new();
    }
    if values.len() <= limit {
        return values.iter().collect();
    }
    (0..limit)
        .map(|index| {
            let position = index * (values.len() - 1) / (limit - 1).max(1);
            &values[position]
        })
        .collect()
}
