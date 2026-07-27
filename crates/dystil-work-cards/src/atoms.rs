use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DistilledEventType {
    Opened,
    Viewed,
    Searched,
    Navigated,
    Edited,
    Executed,
    Tested,
    Communicated,
    Created,
    Deleted,
    Downloaded,
    Uploaded,
    ErrorObserved,
    ResultObserved,
    StateChanged,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistilledAtom {
    pub atom_id: String,
    pub occurred_at: DateTime<Utc>,
    pub event_type: DistilledEventType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_after: Option<String>,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DistilledEvidenceChunk {
    pub chunk_id: String,
    #[serde(default)]
    pub atoms: Vec<DistilledAtom>,
    #[serde(default)]
    pub uncertainties: Vec<GroundedAtomText>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundedAtomText {
    pub text: String,
    pub evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedAtoms {
    pub window_id: String,
    pub chunk_id: String,
    pub model_id: String,
    pub atoms: DistilledEvidenceChunk,
    #[serde(default)]
    pub wall_time_ms: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergedAtoms {
    pub window_id: String,
    pub atoms: Vec<DistilledAtom>,
    pub uncertainties: Vec<GroundedAtomText>,
}
