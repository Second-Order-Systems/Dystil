use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::Principal;

#[derive(Debug, Clone)]
pub struct SemanticTreeInsert<'a> {
    pub sample_id: &'a str,
    pub source_frame_id: Option<i64>,
    pub surface_key: &'a str,
    pub layout_fingerprint: &'a str,
    pub schema_version: i16,
    pub codec: &'a str,
    pub payload_sha256: &'a str,
    pub payload: &'a [u8],
    pub captured_at: DateTime<Utc>,
    pub platform: &'a str,
    pub app_name: &'a str,
    pub app_version: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticTreeWriteOutcome {
    Inserted,
    Deduplicated,
}

#[derive(Debug, thiserror::Error)]
pub enum SemanticTreeWriteError {
    #[error("sample ID already exists with a different payload hash")]
    ConflictingSample,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
}

pub async fn insert_semantic_tree_sample(
    pool: &PgPool,
    principal: &Principal,
    sample: &SemanticTreeInsert<'_>,
) -> Result<SemanticTreeWriteOutcome, SemanticTreeWriteError> {
    if let Some(existing_hash) = sqlx::query_scalar::<_, String>(
        "SELECT payload_sha256
         FROM semantic_tree_samples
         WHERE org_id = $1 AND device_id = $2 AND sample_id = $3",
    )
    .bind(&principal.org_id)
    .bind(&principal.device_id)
    .bind(sample.sample_id)
    .fetch_optional(pool)
    .await?
    {
        return if existing_hash == sample.payload_sha256 {
            Ok(SemanticTreeWriteOutcome::Deduplicated)
        } else {
            Err(SemanticTreeWriteError::ConflictingSample)
        };
    }

    let inserted = sqlx::query(
        "INSERT INTO semantic_tree_samples (
            org_id, user_id, device_id, sample_id, source_frame_id,
            surface_key, layout_fingerprint, schema_version, codec,
            payload_sha256, payload, captured_at, platform, app_name, app_version
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
         )
         ON CONFLICT DO NOTHING",
    )
    .bind(&principal.org_id)
    .bind(&principal.user_id)
    .bind(&principal.device_id)
    .bind(sample.sample_id)
    .bind(sample.source_frame_id)
    .bind(sample.surface_key)
    .bind(sample.layout_fingerprint)
    .bind(sample.schema_version)
    .bind(sample.codec)
    .bind(sample.payload_sha256)
    .bind(sample.payload)
    .bind(sample.captured_at)
    .bind(sample.platform)
    .bind(sample.app_name)
    .bind(sample.app_version)
    .execute(pool)
    .await?;

    if inserted.rows_affected() == 1 {
        return Ok(SemanticTreeWriteOutcome::Inserted);
    }

    if let Some(existing_hash) = sqlx::query_scalar::<_, String>(
        "SELECT payload_sha256
         FROM semantic_tree_samples
         WHERE org_id = $1 AND device_id = $2 AND sample_id = $3",
    )
    .bind(&principal.org_id)
    .bind(&principal.device_id)
    .bind(sample.sample_id)
    .fetch_optional(pool)
    .await?
    {
        if existing_hash != sample.payload_sha256 {
            return Err(SemanticTreeWriteError::ConflictingSample);
        }
    }
    Ok(SemanticTreeWriteOutcome::Deduplicated)
}
