//! Agent-safe retrieval over Dystil's sanitized evidence projection.
//!
//! Storage owns SQLite. This crate owns stable evidence identifiers, response
//! budgets, deduplication, deep links, and deterministic overview diagnosis so
//! every AI adapter observes the same behavior.

mod evidence;
mod overview;
mod search;

pub use evidence::{Evidence, EvidenceId, EvidencePage};
pub use overview::{ActivityOverview, DataStatus, OverviewRequest, RetrievalHealth};
pub use search::{ContextRequest, RangeRequest, SearchPage, SearchRequest};

use sqlx::SqlitePool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RetrievalError {
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("evidence not found")]
    NotFound,
    #[error(transparent)]
    Storage(#[from] dystil_storage::StorageError),
}

pub type Result<T> = std::result::Result<T, RetrievalError>;

#[derive(Clone)]
pub struct RetrievalService {
    pool: SqlitePool,
}

impl RetrievalService {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}
