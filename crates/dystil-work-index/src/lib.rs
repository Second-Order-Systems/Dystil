//! Deterministic construction of compact surface visits from captured frames.
//!
//! This layer records observable application/surface continuity and text changes.
//! It deliberately does not infer task intent, causality, completion, or success.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;

use chrono::{DateTime, Duration, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

#[derive(Debug, Clone)]
pub struct SurfaceVisitConfig {
    pub inactivity: Duration,
    pub max_duration: Duration,
    /// Event-driven frames are not a clock. Gaps larger than this are not
    /// counted as observed active time even when they remain in one span.
    pub active_gap_cap: Duration,
    pub max_changed_text: usize,
}

impl Default for SurfaceVisitConfig {
    fn default() -> Self {
        Self {
            inactivity: Duration::minutes(5),
            max_duration: Duration::minutes(15),
            active_gap_cap: Duration::seconds(60),
            max_changed_text: 24,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FrameObservation {
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub app_name: String,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub document_path: Option<String>,
    pub capture_trigger: Option<String>,
    pub text: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexedEntity {
    pub kind: String,
    pub value: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedText {
    pub text: String,
    pub first_frame_id: i64,
    pub last_frame_id: i64,
    pub observed_frames: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SurfaceVisit {
    pub id: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub wall_clock_seconds: i64,
    pub observed_active_seconds: i64,
    pub close_reason: String,
    pub app_name: String,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub document_path: Option<String>,
    pub surface_key: String,
    pub frame_count: u32,
    pub capture_triggers: BTreeMap<String, u32>,
    pub source_text_chars: u64,
    pub indexed_text_chars: u64,
    pub changed_text: Vec<ChangedText>,
    pub entities: Vec<IndexedEntity>,
    pub first_frame_id: i64,
    pub last_frame_id: i64,
}

#[derive(Debug, Clone)]
struct Surface {
    app: String,
    window: Option<String>,
    browser_url: Option<String>,
    document_path: Option<String>,
}

impl Surface {
    fn from_observation(frame: &FrameObservation) -> Self {
        Self {
            app: normalize_label(&frame.app_name),
            window: clean_optional(frame.window_name.as_deref()).map(normalize_window_label),
            browser_url: frame.browser_url.as_deref().and_then(normalize_browser_url),
            document_path: clean_optional(frame.document_path.as_deref()).map(normalize_path),
        }
    }

    fn is_compatible(&self, other: &Self) -> bool {
        if self.app != other.app {
            return false;
        }
        if let (Some(left), Some(right)) = (&self.document_path, &other.document_path) {
            return left == right;
        }
        if let (Some(left), Some(right)) = (&self.browser_url, &other.browser_url) {
            return left == right;
        }
        match (&self.window, &other.window) {
            (Some(left), Some(right)) => left == right,
            _ => true,
        }
    }

    fn absorb(&mut self, other: &Self) {
        if self.window.is_none() {
            self.window.clone_from(&other.window);
        }
        if self.browser_url.is_none() {
            self.browser_url.clone_from(&other.browser_url);
        }
        if self.document_path.is_none() {
            self.document_path.clone_from(&other.document_path);
        }
    }

    fn key(&self) -> String {
        if let Some(path) = &self.document_path {
            return format!("{}|document:{}", self.app, path);
        }
        if let Some(url) = &self.browser_url {
            return format!("{}|url:{}", self.app, url);
        }
        if let Some(window) = &self.window {
            return format!("{}|window:{}", self.app, window);
        }
        format!("{}|app", self.app)
    }
}

#[derive(Debug, Clone)]
struct TextStat {
    text: String,
    first_frame_id: i64,
    last_frame_id: i64,
    first_ordinal: usize,
    last_ordinal: usize,
    observed_frames: u32,
    appeared_as_change: bool,
}

struct VisitAccumulator {
    first: FrameObservation,
    last_timestamp: DateTime<Utc>,
    last_frame_id: i64,
    surface: Surface,
    observed_active_seconds: i64,
    frame_count: u32,
    source_text_chars: u64,
    triggers: BTreeMap<String, u32>,
    previous_lines: HashSet<String>,
    text: HashMap<String, TextStat>,
    entities: BTreeMap<(String, String), IndexedEntity>,
    ordinal: usize,
}

impl VisitAccumulator {
    fn new(frame: FrameObservation, config: &SurfaceVisitConfig) -> Self {
        let surface = Surface::from_observation(&frame);
        let mut value = Self {
            last_timestamp: frame.timestamp,
            last_frame_id: frame.id,
            surface,
            first: frame,
            observed_active_seconds: 0,
            frame_count: 0,
            source_text_chars: 0,
            triggers: BTreeMap::new(),
            previous_lines: HashSet::new(),
            text: HashMap::new(),
            entities: BTreeMap::new(),
            ordinal: 0,
        };
        let first = value.first.clone();
        value.push(&first, config);
        value
    }

    fn can_accept(&self, frame: &FrameObservation, config: &SurfaceVisitConfig) -> bool {
        let gap = frame.timestamp - self.last_timestamp;
        let duration = frame.timestamp - self.first.timestamp;
        gap >= Duration::zero()
            && gap <= config.inactivity
            && duration <= config.max_duration
            && self
                .surface
                .is_compatible(&Surface::from_observation(frame))
    }

    fn split_reason(&self, frame: &FrameObservation, config: &SurfaceVisitConfig) -> &'static str {
        let incoming = Surface::from_observation(frame);
        if incoming.app != self.surface.app {
            "app_change"
        } else if frame.timestamp - self.last_timestamp > config.inactivity {
            "inactivity"
        } else if frame.timestamp - self.first.timestamp > config.max_duration {
            "max_duration"
        } else {
            "surface_change"
        }
    }

    fn push(&mut self, frame: &FrameObservation, config: &SurfaceVisitConfig) {
        if self.frame_count > 0 {
            let gap = frame.timestamp - self.last_timestamp;
            if gap > Duration::zero() && gap <= config.active_gap_cap {
                self.observed_active_seconds += gap.num_seconds();
            }
        }
        self.surface.absorb(&Surface::from_observation(frame));
        self.last_timestamp = frame.timestamp;
        self.last_frame_id = frame.id;
        self.frame_count += 1;
        if let Some(trigger) = clean_optional(frame.capture_trigger.as_deref()) {
            *self.triggers.entry(trigger.to_owned()).or_default() += 1;
        }

        if let Some(url) = self.surface.browser_url.as_deref() {
            if let Ok(parsed) = Url::parse(url) {
                if let Some(domain) = parsed.host_str() {
                    self.insert_entity("domain", domain, "browser_url");
                }
            }
        }

        let lines = frame
            .text
            .as_deref()
            .map(normalized_lines)
            .unwrap_or_default();
        self.source_text_chars += frame
            .text
            .as_deref()
            .map(|text| text.chars().count() as u64)
            .unwrap_or_default();
        let current = lines.iter().cloned().collect::<HashSet<_>>();
        for line in lines {
            let appeared_as_change = !self.previous_lines.contains(&line);
            let entry = self.text.entry(line.clone()).or_insert_with(|| TextStat {
                text: line.clone(),
                first_frame_id: frame.id,
                last_frame_id: frame.id,
                first_ordinal: self.ordinal,
                last_ordinal: self.ordinal,
                observed_frames: 0,
                appeared_as_change,
            });
            entry.last_frame_id = frame.id;
            entry.last_ordinal = self.ordinal;
            entry.observed_frames += 1;
            entry.appeared_as_change |= appeared_as_change;
            for ticket in ticket_regex().find_iter(&line) {
                self.insert_entity("ticket_candidate", ticket.as_str(), "changed_text");
            }
        }
        self.previous_lines = current;
        self.ordinal += 1;
    }

    fn insert_entity(&mut self, kind: &str, value: &str, source: &str) {
        let key = (kind.to_owned(), value.to_owned());
        self.entities.entry(key).or_insert_with(|| IndexedEntity {
            kind: kind.to_owned(),
            value: value.to_owned(),
            source: source.to_owned(),
        });
    }

    fn finish(self, reason: &str, config: &SurfaceVisitConfig) -> SurfaceVisit {
        let frame_count = self.frame_count.max(1);
        let mut changed = self
            .text
            .into_values()
            .filter(|line| line.appeared_as_change)
            .filter(|line| {
                let ratio = line.observed_frames as f32 / frame_count as f32;
                ratio < 0.85 || contains_entity_like_text(&line.text)
            })
            .map(|line| {
                let rarity = frame_count.saturating_sub(line.observed_frames) as i64 * 20;
                let entity_bonus = if contains_entity_like_text(&line.text) {
                    500
                } else {
                    0
                };
                let length_bonus = line.text.chars().count().min(200) as i64;
                let recency = line.last_ordinal.min(200) as i64;
                (rarity + entity_bonus + length_bonus + recency, line)
            })
            .collect::<Vec<_>>();
        changed.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| left.1.first_ordinal.cmp(&right.1.first_ordinal))
                .then_with(|| left.1.text.cmp(&right.1.text))
        });
        changed.truncate(config.max_changed_text);
        changed.sort_by_key(|(_, line)| line.first_ordinal);
        let changed_text: Vec<ChangedText> = changed
            .into_iter()
            .map(|(_, line)| ChangedText {
                text: line.text,
                first_frame_id: line.first_frame_id,
                last_frame_id: line.last_frame_id,
                observed_frames: line.observed_frames,
            })
            .collect();
        let indexed_text_chars = changed_text
            .iter()
            .map(|line: &ChangedText| line.text.chars().count() as u64)
            .sum();

        let surface_key = self.surface.key();
        let id = span_id(
            self.first.id,
            self.first.timestamp,
            &self.first.app_name,
            &surface_key,
        );
        SurfaceVisit {
            id,
            started_at: self.first.timestamp,
            ended_at: self.last_timestamp,
            wall_clock_seconds: (self.last_timestamp - self.first.timestamp)
                .num_seconds()
                .max(0),
            observed_active_seconds: self.observed_active_seconds,
            close_reason: reason.to_owned(),
            app_name: self.first.app_name,
            window_name: clean_optional(self.first.window_name.as_deref()).map(str::to_owned),
            browser_url: self.surface.browser_url,
            document_path: self.surface.document_path,
            surface_key,
            frame_count: self.frame_count,
            capture_triggers: self.triggers,
            source_text_chars: self.source_text_chars,
            indexed_text_chars,
            changed_text,
            entities: self.entities.into_values().collect(),
            first_frame_id: self.first.id,
            last_frame_id: self.last_frame_id,
        }
    }
}

pub fn build_surface_visits(
    mut frames: Vec<FrameObservation>,
    config: &SurfaceVisitConfig,
) -> Vec<SurfaceVisit> {
    frames.sort_by(|left, right| {
        left.timestamp
            .cmp(&right.timestamp)
            .then_with(|| left.id.cmp(&right.id))
    });
    let mut spans = Vec::new();
    let mut current: Option<VisitAccumulator> = None;
    for frame in frames
        .into_iter()
        .filter(|frame| !frame.app_name.trim().is_empty())
    {
        match current.as_mut() {
            Some(span) if span.can_accept(&frame, config) => span.push(&frame, config),
            Some(span) => {
                let reason = span.split_reason(&frame, config);
                let finished = current.take().expect("current span").finish(reason, config);
                spans.push(finished);
                current = Some(VisitAccumulator::new(frame, config));
            }
            None => current = Some(VisitAccumulator::new(frame, config)),
        }
    }
    if let Some(span) = current {
        spans.push(span.finish("end_of_input", config));
    }
    spans
}

fn clean_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn normalize_label(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn normalize_window_label(value: &str) -> String {
    // Some apps place volatile notification counts in an otherwise stable
    // title. Removing only this strongly labelled counter avoids splitting a
    // Slack-like surface whenever its unread count changes.
    let without_unread = unread_title_regex().replace_all(value, " - ");
    normalize_label(&without_unread)
}

fn normalize_path(value: &str) -> String {
    value.trim().replace('\\', "/")
}

pub fn normalize_browser_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let candidate = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let mut parsed = Url::parse(&candidate).ok()?;
    parsed.set_fragment(None);
    parsed.set_query(None);
    let _ = parsed.set_username("");
    let _ = parsed.set_password(None);
    Some(parsed.to_string().trim_end_matches('/').to_owned())
}

fn normalized_lines(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|line| line.chars().count() >= 3)
        .map(|line| truncate_chars(&line, 500))
        .filter(|line| seen.insert(line.clone()))
        .collect()
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_owned();
    }
    value.chars().take(max).collect()
}

fn ticket_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| Regex::new(r"\b[A-Z][A-Z0-9]{1,9}-[0-9]+\b").expect("ticket regex"))
}

fn unread_title_regex() -> &'static Regex {
    static VALUE: OnceLock<Regex> = OnceLock::new();
    VALUE.get_or_init(|| {
        Regex::new(r"(?i)\s*-\s*\d+\s+new\s+items?\s*-\s*").expect("unread title regex")
    })
}

fn contains_entity_like_text(value: &str) -> bool {
    ticket_regex().is_match(value)
        || value.contains("http://")
        || value.contains("https://")
        || value.contains("error")
        || value.contains("Error")
}

fn span_id(first_frame_id: i64, at: DateTime<Utc>, app: &str, surface: &str) -> String {
    let mut digest = Sha256::new();
    for value in [
        first_frame_id.to_string(),
        at.to_rfc3339(),
        app.to_owned(),
        surface.to_owned(),
    ] {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value.as_bytes());
    }
    format!("span_{}", &format!("{:x}", digest.finalize())[..20])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(id: i64, seconds: i64, app: &str, window: &str, text: &str) -> FrameObservation {
        FrameObservation {
            id,
            timestamp: DateTime::parse_from_rfc3339("2026-07-31T10:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
                + Duration::seconds(seconds),
            app_name: app.into(),
            window_name: Some(window.into()),
            browser_url: None,
            document_path: None,
            capture_trigger: Some("click".into()),
            text: Some(text.into()),
        }
    }

    #[test]
    fn splits_on_app_surface_inactivity_and_max_duration() {
        let frames = vec![
            frame(1, 0, "Slack", "#team", "A\nhello"),
            frame(2, 10, "Slack", "#team", "A\nhello\nnew"),
            frame(3, 20, "Slack", "#other", "B\nmessage"),
            frame(4, 30, "Code", "main.rs", "fn main"),
            frame(5, 400, "Code", "main.rs", "fn main"),
            frame(6, 680, "Code", "main.rs", "fn main"),
            frame(7, 960, "Code", "main.rs", "fn main"),
            frame(8, 1_240, "Code", "main.rs", "fn main"),
            frame(9, 1_320, "Code", "main.rs", "fn main"),
        ];
        let spans = build_surface_visits(frames, &SurfaceVisitConfig::default());
        assert_eq!(spans.len(), 5);
        assert_eq!(spans[0].close_reason, "surface_change");
        assert_eq!(spans[1].close_reason, "app_change");
        assert_eq!(spans[2].close_reason, "inactivity");
        assert_eq!(spans[3].close_reason, "max_duration");
        assert_eq!(spans[4].close_reason, "end_of_input");
    }

    #[test]
    fn records_text_changes_and_deterministic_entities() {
        let spans = build_surface_visits(
            vec![
                frame(1, 0, "Browser", "Ticket", "Header\nDYS-42 is open"),
                frame(
                    2,
                    10,
                    "Browser",
                    "Ticket",
                    "Header\nDYS-42 is complete\nNo errors",
                ),
            ],
            &SurfaceVisitConfig::default(),
        );
        assert_eq!(spans.len(), 1);
        assert!(spans[0]
            .entities
            .iter()
            .any(|entity| entity.kind == "ticket_candidate" && entity.value == "DYS-42"));
        assert!(spans[0]
            .changed_text
            .iter()
            .any(|line| line.text.contains("complete")));
    }

    #[test]
    fn strips_url_credentials_query_and_fragment() {
        assert_eq!(
            normalize_browser_url("https://user:pass@Example.com/a?token=x#part").as_deref(),
            Some("https://example.com/a")
        );
    }

    #[test]
    fn ignores_volatile_unread_counts_in_window_identity() {
        assert_eq!(
            normalize_window_label("#team - Acme - 3 new items - Slack"),
            normalize_window_label("#team - Acme - Slack")
        );
    }
}
