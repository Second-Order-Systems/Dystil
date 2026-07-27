use dystil_protocol::ImageCompleteItem;
use sqlx::PgPool;

use crate::{DbError, Principal};

pub async fn upsert_prepared_capture_image(
    pool: &PgPool,
    principal: &Principal,
    image_id: &str,
    client_image_key: &str,
    content_hash: &str,
    object_key: &str,
    mime_type: &str,
    byte_size: i64,
    width: i32,
    height: i32,
    selection_reason: &str,
    sync_metadata: Option<&serde_json::Value>,
) -> Result<(), DbError> {
    sqlx::query(
        "INSERT INTO capture_images
         (org_id, user_id, device_id, image_id, client_image_key, content_hash, object_key,
          mime_type, byte_size, width, height, selection_reason, sync_metadata, status, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, 'prepared', now(), now())
         ON CONFLICT (org_id, user_id, device_id, client_image_key)
         DO UPDATE SET image_id = EXCLUDED.image_id,
                       content_hash = EXCLUDED.content_hash,
                       object_key = EXCLUDED.object_key,
                       mime_type = EXCLUDED.mime_type,
                       byte_size = EXCLUDED.byte_size,
                       width = EXCLUDED.width,
                       height = EXCLUDED.height,
                       selection_reason = EXCLUDED.selection_reason,
                       sync_metadata = EXCLUDED.sync_metadata,
                       updated_at = now()",
    )
    .bind(&principal.org_id)
    .bind(&principal.user_id)
    .bind(&principal.device_id)
    .bind(image_id)
    .bind(client_image_key)
    .bind(content_hash)
    .bind(object_key)
    .bind(mime_type)
    .bind(byte_size)
    .bind(width)
    .bind(height)
    .bind(selection_reason)
    .bind(sync_metadata)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn complete_capture_images(
    pool: &PgPool,
    principal: &Principal,
    images: &[ImageCompleteItem],
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    for image in images {
        sqlx::query(
            "UPDATE capture_images
             SET mime_type = $5,
                 byte_size = $6,
                 width = $7,
                 height = $8,
                 status = 'completed',
                 completed_at = now(),
                 updated_at = now(),
                 last_error = NULL
             WHERE org_id = $1 AND user_id = $2 AND device_id = $3 AND client_image_key = $4",
        )
        .bind(&principal.org_id)
        .bind(&principal.user_id)
        .bind(&principal.device_id)
        .bind(&image.client_image_key)
        .bind(&image.mime_type)
        .bind(image.byte_size as i64)
        .bind(image.width as i32)
        .bind(image.height as i32)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}
