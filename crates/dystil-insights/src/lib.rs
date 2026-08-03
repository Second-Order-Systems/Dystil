//! Local Worth Fixing backend.
//!
//! This crate owns deterministic admission, compaction, durable jobs and
//! cursors, opportunity state, eligibility, ranking, dispositions, retention,
//! and app-facing DTOs. Provider execution remains behind `dystil-ai`.

mod artifact;
mod artifact_engine;
mod compaction;
mod engine;
mod kernel;
mod retention;
mod scheduler;
mod source_admission;
mod store;
#[cfg(test)]
mod test_support;
mod types;

pub use artifact::*;
pub use artifact_engine::*;
pub use compaction::*;
pub use engine::*;
pub use kernel::*;
pub use retention::*;
pub use scheduler::*;
pub use source_admission::*;
pub use store::*;
pub use types::*;
