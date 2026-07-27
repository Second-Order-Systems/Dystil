use chrono::{DateTime, Utc};
use dystil_protocol::{SegmentEnvelope, SegmentEvidenceKind};
use serde::{Deserialize, Serialize};

use crate::CompactionStats;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportedSegment {
    pub org_id: String,
    pub user_id: String,
    pub device_id: String,
    pub segment_id: String,
    pub revision: u32,
    pub content_hash: String,
    pub envelope: SegmentEnvelope,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceWindow {
    pub window_id: String,
    pub device_id: String,
    pub start_time: DateTime<Utc>,
    pub end_time: DateTime<Utc>,
    pub close_reason: String,
    pub segment_ids: Vec<String>,
    pub items: Vec<dystil_protocol::SegmentEvidenceItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactedEvidence {
    pub evidence_id: String,
    pub occurred_at: DateTime<Utc>,
    pub kind: SegmentEvidenceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub browser_url: Option<String>,
    pub text: String,
    #[serde(default)]
    pub source_ids: Vec<String>,
    pub estimated_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactedWindow {
    pub window: EvidenceWindow,
    pub evidence: Vec<CompactedEvidence>,
    pub stats: CompactionStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptRecord {
    pub window_id: String,
    pub prompt: String,
    pub evidence: Vec<CompactedEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedWorkCard {
    pub window_id: String,
    pub model_id: String,
    pub card: WorkCard,
    #[serde(default)]
    pub wall_time_ms: Option<u64>,
    #[serde(default)]
    pub peak_rss_kib: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundedClaim {
    pub text: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkCardStatus {
    Completed,
    InProgress,
    Blocked,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundedArtifact {
    pub kind: String,
    pub value: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkCard {
    pub title: String,
    pub summary: GroundedClaim,
    #[serde(default)]
    pub applications: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<GroundedArtifact>,
    #[serde(default)]
    pub actions: Vec<GroundedClaim>,
    pub last_observed_state: GroundedClaim,
    pub status: WorkCardStatus,
    #[serde(default)]
    pub uncertainties: Vec<String>,
}
