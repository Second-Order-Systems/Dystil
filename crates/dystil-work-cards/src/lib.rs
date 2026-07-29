//! Local, evidence-grounded work-card construction.
//!
//! This crate deliberately stops short of owning an inference runtime. It
//! turns capture evidence into bounded, deterministic model inputs and
//! validates generated cards against the evidence that was actually observed.

mod chunking;
mod compaction;
mod prebudget;
mod prompt;
mod sanitize;
mod schema;
mod validation;
mod windowing;

pub use chunking::{chunk_reduced_window, ChunkConfig, ChunkingStats, EvidenceChunk};
pub use compaction::{compact_window, CompactionConfig, CompactionStats};
pub use prebudget::{
    reduce_window_before_budget, PreBudgetReductionConfig, PreBudgetReductionStats,
    ReducedEvidenceWindow,
};
pub use prompt::{build_work_card_prompt, work_card_json_schema, PromptConfig};
pub use sanitize::{sanitize_work_card, SanitizationStats};
pub use schema::{
    CompactedEvidence, CompactedWindow, EvidenceWindow, ExportedSegment, GeneratedWorkCard,
    GroundedArtifact, GroundedClaim, PromptRecord, WorkCard, WorkCardStatus,
};
pub use validation::{validate_work_card, ValidationError, ValidationReport};
pub use windowing::{build_evidence_windows, build_evidence_windows_from_items, WindowConfig};
