use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

use crate::DbError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AiKeyRecord {
    pub id: Uuid,
    pub email: String,
    pub key_prefix: String,
    pub spend_limit_microusd: i64,
    pub spent_microusd: i64,
}

#[derive(Debug, Clone)]
pub struct NewAiUsage<'a> {
    pub key_id: Uuid,
    pub openai_request_id: Option<&'a str>,
    pub model: &'a str,
    pub input_tokens: i64,
    pub cached_input_tokens: i64,
    pub cache_write_tokens: i64,
    pub output_tokens: i64,
    pub cost_microusd: i64,
}

pub fn hash_ai_key(raw_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(raw_key.as_bytes());
    hex::encode(hasher.finalize())
}

pub async fn insert_ai_key(
    pool: &PgPool,
    email: &str,
    key_prefix: &str,
    raw_key: &str,
    spend_limit_microusd: i64,
) -> Result<Uuid, DbError> {
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO ai_keys
         (id, email, key_prefix, key_hash, spend_limit_microusd)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(email.trim())
    .bind(key_prefix)
    .bind(hash_ai_key(raw_key))
    .bind(spend_limit_microusd)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn resolve_active_ai_key(
    pool: &PgPool,
    raw_key: &str,
) -> Result<Option<AiKeyRecord>, DbError> {
    let record = sqlx::query_as::<_, AiKeyRecord>(
        "SELECT id, email, key_prefix, spend_limit_microusd, spent_microusd
         FROM ai_keys
         WHERE key_hash = $1 AND revoked_at IS NULL",
    )
    .bind(hash_ai_key(raw_key))
    .fetch_optional(pool)
    .await?;
    Ok(record)
}

pub async fn record_ai_usage(pool: &PgPool, usage: NewAiUsage<'_>) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query(
        "UPDATE ai_keys
         SET spent_microusd = spent_microusd + $2
         WHERE id = $1",
    )
    .bind(usage.key_id)
    .bind(usage.cost_microusd)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO ai_usage
         (id, key_id, openai_request_id, model, input_tokens,
          cached_input_tokens, cache_write_tokens, output_tokens, cost_microusd)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
    )
    .bind(Uuid::new_v4())
    .bind(usage.key_id)
    .bind(usage.openai_request_id)
    .bind(usage.model)
    .bind(usage.input_tokens)
    .bind(usage.cached_input_tokens)
    .bind(usage.cache_write_tokens)
    .bind(usage.output_tokens)
    .bind(usage.cost_microusd)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn revoke_ai_key(pool: &PgPool, key_prefix: &str) -> Result<bool, DbError> {
    let result = sqlx::query(
        "UPDATE ai_keys
         SET revoked_at = COALESCE(revoked_at, now())
         WHERE key_prefix = $1",
    )
    .bind(key_prefix)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::hash_ai_key;

    #[test]
    fn key_hash_is_deterministic_without_retaining_the_key() {
        assert_eq!(
            hash_ai_key("dst_live_a_secret"),
            hash_ai_key("dst_live_a_secret")
        );
        assert_ne!(
            hash_ai_key("dst_live_a_secret"),
            hash_ai_key("dst_live_b_secret")
        );
        assert!(!hash_ai_key("dst_live_a_secret").contains("secret"));
    }
}
