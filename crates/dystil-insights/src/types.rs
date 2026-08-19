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
    pub url: Option<String>,
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
    /// Complete, bounded artifact content. A preview is derived by the kernel.
    pub body: String,
    /// Short, user-visible steps for the Worth fixing decision surface. The
    /// complete reusable artifact remains `body`.
    pub preview_steps: Vec<String>,
    pub capability_id: Option<String>,
}

/// Steward's semantic reading of whether the observed work reached a usable
/// outcome. The backend combines this with timestamps; it never trusts a raw
/// event count as proof of recurrence.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompletionState {
    Completed,
    Partial,
    Cancelled,
    #[default]
    Unclear,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FindingDraft {
    pub claim: String,
    pub why_worth_fixing: String,
    /// A plain evidence-grounded note shown beside deterministic metrics.
    pub evidence_note: String,
    pub evidence_ids: Vec<String>,
    /// Required for new Steward output; defaults preserve historical records.
    #[serde(default)]
    pub completion_state: CompletionState,
    /// Broad workflow stages actually supported by the cited evidence, such as
    /// `input`, `transform`, `review`, or `handoff`.
    #[serde(default)]
    pub workflow_stages: Vec<String>,
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
    pub manual_refresh_ready: bool,
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

/// A deterministic, auditable metric for the Home Worth fixing surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeEvidenceStat {
    pub value: String,
    pub label: String,
}

/// The only current Home origin: findings raised from locally observed work.
/// User-requested work will be added when Ask for a fix is redesigned.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum HomeFindingOrigin {
    Dystil,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeWorthFixingItem {
    pub finding_id: String,
    pub origin: HomeFindingOrigin,
    pub occurred_at: String,
    pub title: String,
    pub evidence: Vec<HomeEvidenceStat>,
    pub evidence_note: String,
    pub offer: String,
    pub fix_name: String,
    pub steps: Vec<String>,
    pub save_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct HomeWorthFixingSummary {
    pub items: Vec<HomeWorthFixingItem>,
    pub watching_count: u32,
    pub processing: bool,
    pub provider_ready: bool,
    pub last_successful_wake_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct FindingPage {
    pub items: Vec<WorthFixingCard>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum ReadyArtifactAction {
    Copy,
    Open,
    Share,
    ShowHow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyArtifactCard {
    pub artifact_id: String,
    pub title: String,
    pub kind: HandoffType,
    pub description: String,
    pub last_used_at: Option<String>,
    pub primary_action: ReadyArtifactAction,
    pub secondary_action: ReadyArtifactAction,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactPage {
    pub items: Vec<ReadyArtifactCard>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChangeSummary {
    pub request: String,
    pub changed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyArtifactDetail {
    pub card: ReadyArtifactCard,
    pub body: String,
    pub kept_at: String,
    pub change_count: u32,
    pub changes: Vec<ArtifactChangeSummary>,
    pub provenance_available: bool,
    pub provenance_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeepFindingResult {
    pub artifact: ReadyArtifactCard,
    pub summary: WorthFixingSummary,
    pub already_kept: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyArtifactUseResult {
    pub artifact_id: String,
    pub last_used_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ReadyArtifactMutationResult {
    pub artifact_id: String,
    pub revision: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillBundleStatus {
    Pending,
    Running,
    Ready,
    Failed,
    Interrupted,
}

/// A concise, Dystil-owned build stage. Provider event payloads never cross
/// this boundary; the UI receives only this safe progress label.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillBundleStage {
    Preparing,
    Investigating,
    Building,
    Validating,
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillBundleView {
    pub job_id: Option<String>,
    pub bundle_id: Option<String>,
    pub revision: Option<u32>,
    pub skill_name: Option<String>,
    pub status: SkillBundleStatus,
    pub stage: Option<SkillBundleStage>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "snake_case")]
pub enum SkillInstallTarget {
    Codex,
    Claude,
    ClaudeUpload,
    Chatgpt,
    Pi,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallReceipt {
    pub install_id: String,
    pub bundle_id: String,
    pub target: SkillInstallTarget,
    pub destination: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallTargetAvailability {
    pub target: SkillInstallTarget,
    pub available: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactChangePreview {
    pub change_job_id: String,
    pub artifact_id: String,
    pub title: String,
    pub body: String,
    pub changed_line_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ArtifactChangeOutput {
    pub schema_version: u32,
    pub title: String,
    pub body: String,
}
