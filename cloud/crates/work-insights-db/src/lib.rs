pub mod agent_mailbox;
pub mod identity;
pub mod ingest;
pub mod segments;

pub async fn migrate(pool: &sqlx::PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Other(String),
}

#[derive(Debug, Clone)]
pub struct Principal {
    pub org_id: String,
    pub user_id: String,
    pub device_id: String,
}

#[derive(Debug, Default)]
pub struct SegmentWriteStats {
    pub inserted_count: usize,
    pub deduped_count: usize,
}
