use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Type)]
#[serde(rename_all = "snake_case")]
pub enum Construct {
    Recognition,
    ManualTransfer,
    UnchangedRepetition,
    TemporalPattern,
    RepeatedComposition,
}

impl Construct {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recognition => "recognition",
            Self::ManualTransfer => "manual_transfer",
            Self::UnchangedRepetition => "unchanged_repetition",
            Self::TemporalPattern => "temporal_pattern",
            Self::RepeatedComposition => "repeated_composition",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum HandoffType {
    Prompt,
    SavedPrompt,
    ExistingCapability,
    Runbook,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ObservationCertainty {
    Explicit,
    StronglyImplied,
    Tentative,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceQuality {
    High,
    Medium,
    Low,
}

impl EvidenceQuality {
    pub fn penalty(self) -> i32 {
        match self {
            Self::High => 0,
            Self::Medium => 4,
            Self::Low => 8,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum Cadence {
    None,
    Daily,
    Weekly,
    Monthly,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStatus {
    Watching,
    Eligible,
    Surfaced,
    Withdrawn,
    Retired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum DispositionKind {
    Accepted,
    Saved,
    NotAProblem,
    LeaveIt,
    CloseBut,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EvidenceRecord {
    pub evidence_id: String,
    pub source_namespace: String,
    pub source_id: String,
    pub occurred_at: String,
    pub app: Option<String>,
    pub window: Option<String>,
    pub excerpt: String,
    pub policy_allowed: bool,
    pub redaction_ready: bool,
    pub deleted: bool,
    pub sensitive: bool,
}

impl EvidenceRecord {
    pub fn admissible(&self) -> bool {
        self.policy_allowed && self.redaction_ready && !self.deleted && !self.sensitive
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservationRecord {
    pub observation_id: String,
    pub source_key: String,
    pub occurred_at: String,
    pub statement: String,
    pub certainty: ObservationCertainty,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExplorerObservationDraft {
    pub local_id: String,
    pub statement: String,
    pub certainty: ObservationCertainty,
    pub occurred_at: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExplorerOutput {
    pub schema_version: u32,
    pub observations: Vec<ExplorerObservationDraft>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OccurrenceDelta {
    pub local_id: String,
    pub observation_ids: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub steps: Vec<String>,
    pub distinctness_basis: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Handoff {
    pub kind: HandoffType,
    pub title: String,
    pub preview: String,
    pub capability_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingDraft {
    pub claim: String,
    pub why_worth_fixing: String,
    pub evidence_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RankSignals {
    pub actionability: u8,
    pub estimated_burden: u8,
    pub novelty: u8,
    pub user_relevance: u8,
    pub sensitivity_risk: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OpportunityDelta {
    pub local_id: String,
    pub existing_opportunity_id: Option<String>,
    pub construct: Construct,
    pub summary: String,
    pub signature: String,
    pub occurrences_to_add: Vec<OccurrenceDelta>,
    pub withdraw_current_finding: bool,
    pub retire: bool,
    pub transfer_established: bool,
    pub authorship_established: bool,
    pub cadence: Cadence,
    pub unresolved_questions: Vec<String>,
    pub evidence_quality: EvidenceQuality,
    pub handoff: Option<Handoff>,
    pub finding: Option<FindingDraft>,
    pub rank_signals: RankSignals,
    pub automation_potential: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReconciliationOutput {
    pub schema_version: u32,
    pub considered_observation_ids: Vec<String>,
    pub opportunities: Vec<OpportunityDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RankVector {
    pub occurrence_maturity: i32,
    pub occurrence_count: i32,
    pub explicit_evidence: i32,
    pub actionability: i32,
    pub estimated_burden: i32,
    pub novelty: i32,
    pub user_relevance: i32,
    pub sensitivity_risk: i32,
    pub unresolved_penalty: i32,
    pub evidence_quality: EvidenceQuality,
    pub evidence_quality_penalty: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorthFixingCard {
    pub finding_id: String,
    pub label: String,
    pub claim: String,
    pub why_worth_fixing: String,
    pub handoff_type: HandoffType,
    pub handoff_title: String,
    pub handoff_preview: String,
    pub occurrence_count: u32,
    pub cadence: Cadence,
    pub evidence_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorthFixingSummary {
    pub selected: Vec<WorthFixingCard>,
    pub eligible_count: u32,
    pub watching_count: u32,
    pub pending_observation_count: u32,
    pub processing: bool,
    pub stale_evidence_count: u32,
    pub provider_ready: bool,
    pub last_successful_wake_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct WorthFixingEvidenceLine {
    pub evidence_id: String,
    pub occurred_at: String,
    pub app: Option<String>,
    pub description: String,
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct FindingPage {
    pub items: Vec<WorthFixingCard>,
    pub next_cursor: Option<String>,
}
