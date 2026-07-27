use dystil_protocol::{DeviceSyncStateResponse, SegmentEnvelope, SegmentUploadRequest};
use sqlx::{PgPool, Postgres, Row, Transaction};

use crate::{DbError, Principal, SegmentWriteStats};

pub async fn apply_segment_upload(
    pool: &PgPool,
    principal: &Principal,
    request: &SegmentUploadRequest,
) -> Result<SegmentWriteStats, DbError> {
    let mut tx = pool.begin().await?;
    let mut stats = SegmentWriteStats::default();

    for segment in &request.segments {
        apply_segment(&mut tx, principal, segment, &mut stats).await?;
    }

    tx.commit().await?;
    Ok(stats)
}

async fn apply_segment(
    tx: &mut Transaction<'_, Postgres>,
    principal: &Principal,
    segment: &SegmentEnvelope,
    stats: &mut SegmentWriteStats,
) -> Result<(), DbError> {
    let existing = sqlx::query(
        "SELECT content_hash
         FROM memory_segments
         WHERE org_id = $1 AND segment_id = $2 AND revision = $3
         FOR UPDATE",
    )
    .bind(&principal.org_id)
    .bind(&segment.segment_id)
    .bind(segment.revision as i32)
    .fetch_optional(&mut **tx)
    .await?;

    if let Some(existing) = existing {
        let existing_hash: String = existing.try_get("content_hash")?;
        if existing_hash != segment.content_hash {
            return Err(DbError::Other(format!(
                "segment {} revision {} already exists with a different content hash",
                segment.segment_id, segment.revision
            )));
        }
        stats.deduped_count += 1;
        return Ok(());
    }

    let max_revision: Option<i32> = sqlx::query_scalar(
        "SELECT max(revision)
         FROM memory_segments
         WHERE org_id = $1 AND segment_id = $2",
    )
    .bind(&principal.org_id)
    .bind(&segment.segment_id)
    .fetch_one(&mut **tx)
    .await?;
    let is_latest = max_revision
        .map(|revision| segment.revision as i32 > revision)
        .unwrap_or(true);
    // `memory_episodes` used to be the authority for this check, but episode
    // storage is no longer part of the cloud schema. A processed revision is
    // the durable indication that this logical segment has already been
    // consumed, and lets corrections remain audit-only as before.
    let already_analyzed: bool = sqlx::query_scalar(
        "SELECT EXISTS (
             SELECT 1 FROM memory_segments
             WHERE org_id = $1 AND segment_id = $2 AND status = 'processed'
         )",
    )
    .bind(&principal.org_id)
    .bind(&segment.segment_id)
    .fetch_one(&mut **tx)
    .await?;

    if is_latest {
        sqlx::query(
            "UPDATE memory_segments
             SET status = 'superseded',
                 superseded_at = now(),
                 leased_by = NULL,
                 leased_until = NULL,
                 fencing_token = NULL,
                 updated_at = now()
             WHERE org_id = $1 AND segment_id = $2
               AND revision < $3 AND status != 'processed'",
        )
        .bind(&principal.org_id)
        .bind(&segment.segment_id)
        .bind(segment.revision as i32)
        .execute(&mut **tx)
        .await?;
    }

    // Correction episodes are deferred. Keep later revisions for audit without
    // putting an already-analyzed logical segment back on the episode queue.
    let status = if is_latest && !already_analyzed {
        "ready"
    } else {
        "superseded"
    };
    sqlx::query(
        "INSERT INTO memory_segments
         (org_id, user_id, device_id, segment_id, revision, device_sequence,
          previous_segment_id, start_time, end_time, closed_at, segmenter_version,
          evidence_version, content_hash, token_estimate, envelope_json,
          status, received_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                 $14, $15, $16, now(), now())",
    )
    .bind(&principal.org_id)
    .bind(&principal.user_id)
    .bind(&principal.device_id)
    .bind(&segment.segment_id)
    .bind(segment.revision as i32)
    .bind(segment.device_sequence as i64)
    .bind(&segment.previous_segment_id)
    .bind(segment.start_time)
    .bind(segment.end_time)
    .bind(segment.closed_at)
    .bind(&segment.segmenter_version)
    .bind(&segment.evidence_version)
    .bind(&segment.content_hash)
    .bind(segment.token_estimate as i32)
    .bind(serde_json::to_value(segment)?)
    .bind(status)
    .execute(&mut **tx)
    .await?;

    stats.inserted_count += 1;
    Ok(())
}

pub async fn get_device_sync_state(
    pool: &PgPool,
    principal: &Principal,
) -> Result<DeviceSyncStateResponse, DbError> {
    let row = sqlx::query(
        "SELECT max(device_sequence) AS max_sequence
         FROM memory_segments
         WHERE org_id = $1 AND device_id = $2",
    )
    .bind(&principal.org_id)
    .bind(&principal.device_id)
    .fetch_one(pool)
    .await?;

    let max_sequence: Option<i64> = row.try_get("max_sequence")?;
    let max_sequence = max_sequence.unwrap_or(0) as u64;

    let last_segment_id = if max_sequence > 0 {
        sqlx::query_scalar(
            "SELECT segment_id
             FROM memory_segments
             WHERE org_id = $1 AND device_id = $2 AND device_sequence = $3
             ORDER BY revision DESC
             LIMIT 1",
        )
        .bind(&principal.org_id)
        .bind(&principal.device_id)
        .bind(max_sequence as i64)
        .fetch_optional(pool)
        .await?
    } else {
        None
    };

    Ok(DeviceSyncStateResponse {
        ok: true,
        max_sequence,
        last_segment_id,
    })
}
