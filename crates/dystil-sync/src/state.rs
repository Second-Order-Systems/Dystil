use std::path::Path;

use dystil_protocol::{SegmentEnvelope, SegmentRevisionAck};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};

use crate::types::{SourceCursor, SyncError};

#[derive(Debug, Clone)]
pub(crate) struct LocalSyncState {
    pub cursor: SourceCursor,
    pub next_segment_sequence: u64,
    pub last_uploaded_segment_id: Option<String>,
}

impl Default for LocalSyncState {
    fn default() -> Self {
        Self {
            cursor: SourceCursor::default(),
            next_segment_sequence: 1,
            last_uploaded_segment_id: None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingSegment {
    pub status: String,
    pub envelope: SegmentEnvelope,
}

pub(crate) struct SegmentStore {
    pool: SqlitePool,
}

impl SegmentStore {
    pub async fn open(path: &Path) -> Result<Self, SyncError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        let store = Self { pool };
        store.initialize().await?;
        Ok(store)
    }

    async fn initialize(&self) -> Result<(), SyncError> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sync_state (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                cursor_json TEXT NOT NULL,
                next_segment_sequence INTEGER NOT NULL,
                last_uploaded_segment_id TEXT,
                updated_at TEXT NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS segments (
                segment_id TEXT NOT NULL,
                revision INTEGER NOT NULL,
                device_sequence INTEGER NOT NULL,
                status TEXT NOT NULL,
                envelope_json TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                last_error TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (segment_id, revision)
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_state(&self) -> Result<LocalSyncState, SyncError> {
        let row = sqlx::query(
            "SELECT cursor_json, next_segment_sequence, last_uploaded_segment_id
             FROM sync_state WHERE id = 1",
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else {
            return Ok(LocalSyncState::default());
        };
        Ok(LocalSyncState {
            cursor: serde_json::from_str(row.try_get("cursor_json")?)?,
            next_segment_sequence: row.try_get::<i64, _>("next_segment_sequence")? as u64,
            last_uploaded_segment_id: row.try_get("last_uploaded_segment_id")?,
        })
    }

    pub async fn load_pending(&self) -> Result<Vec<PendingSegment>, SyncError> {
        let rows = sqlx::query(
            "SELECT status, envelope_json
             FROM segments
             ORDER BY device_sequence ASC, revision ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| {
                Ok(PendingSegment {
                    status: row.try_get("status")?,
                    envelope: serde_json::from_str(row.try_get("envelope_json")?)?,
                })
            })
            .collect()
    }

    pub async fn replace_pending(
        &self,
        state: &LocalSyncState,
        segments: &[PendingSegment],
    ) -> Result<(), SyncError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM segments")
            .execute(&mut *tx)
            .await?;
        for segment in segments {
            sqlx::query(
                "INSERT INTO segments
                 (segment_id, revision, device_sequence, status, envelope_json, content_hash,
                  created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now'), datetime('now'))",
            )
            .bind(&segment.envelope.segment_id)
            .bind(segment.envelope.revision as i64)
            .bind(segment.envelope.device_sequence as i64)
            .bind(&segment.status)
            .bind(serde_json::to_string(&segment.envelope)?)
            .bind(&segment.envelope.content_hash)
            .execute(&mut *tx)
            .await?;
        }
        sqlx::query(
            "INSERT INTO sync_state
             (id, cursor_json, next_segment_sequence, last_uploaded_segment_id, updated_at)
             VALUES (1, ?1, ?2, ?3, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
               cursor_json = excluded.cursor_json,
               next_segment_sequence = excluded.next_segment_sequence,
               last_uploaded_segment_id = excluded.last_uploaded_segment_id,
               updated_at = excluded.updated_at",
        )
        .bind(serde_json::to_string(&state.cursor)?)
        .bind(state.next_segment_sequence as i64)
        .bind(&state.last_uploaded_segment_id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn stable_segments(&self) -> Result<Vec<SegmentEnvelope>, SyncError> {
        let rows = sqlx::query(
            "SELECT envelope_json FROM segments
             WHERE status = 'stable'
             ORDER BY device_sequence ASC, revision ASC",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| Ok(serde_json::from_str(row.try_get("envelope_json")?)?))
            .collect()
    }

    pub async fn acknowledge(&self, accepted: &[SegmentRevisionAck]) -> Result<(), SyncError> {
        if accepted.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        let mut last_uploaded: Option<(u64, String)> = None;
        for ack in accepted {
            let row = sqlx::query(
                "SELECT device_sequence FROM segments
                 WHERE segment_id = ?1 AND revision = ?2",
            )
            .bind(&ack.segment_id)
            .bind(ack.revision as i64)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(row) = row {
                let sequence = row.try_get::<i64, _>("device_sequence")? as u64;
                if last_uploaded
                    .as_ref()
                    .map(|(value, _)| sequence > *value)
                    .unwrap_or(true)
                {
                    last_uploaded = Some((sequence, ack.segment_id.clone()));
                }
            }
            sqlx::query("DELETE FROM segments WHERE segment_id = ?1 AND revision = ?2")
                .bind(&ack.segment_id)
                .bind(ack.revision as i64)
                .execute(&mut *tx)
                .await?;
        }
        if let Some((_, segment_id)) = last_uploaded {
            sqlx::query(
                "UPDATE sync_state SET last_uploaded_segment_id = ?1, updated_at = datetime('now')
                 WHERE id = 1",
            )
            .bind(segment_id)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dystil_protocol::{
        SegmentEvidenceItem, SegmentEvidenceKind, EVIDENCE_VERSION, SEGMENTER_VERSION,
    };

    fn sample_segment() -> SegmentEnvelope {
        let now = Utc::now();
        let mut segment = SegmentEnvelope {
            segment_id: "seg_test_00000001".to_string(),
            revision: 1,
            device_sequence: 1,
            previous_segment_id: None,
            start_time: now,
            end_time: now,
            closed_at: now,
            segmenter_version: SEGMENTER_VERSION.to_string(),
            evidence_version: EVIDENCE_VERSION.to_string(),
            content_hash: String::new(),
            token_estimate: 1,
            sync_policy_version: None,
            items: vec![SegmentEvidenceItem {
                item_id: "item_1".to_string(),
                kind: SegmentEvidenceKind::Screen,
                occurred_at: now,
                source_id: "screen_frame:1".to_string(),
                source_payload_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                text: "test".to_string(),
                app_name: None,
                window_name: None,
                browser_url: None,
                metadata: serde_json::json!({}),
            }],
            image_refs: Vec::new(),
        };
        segment.refresh_content_hash().unwrap();
        segment
    }

    #[tokio::test]
    async fn persists_and_acknowledges_stable_segments() {
        let path = std::env::temp_dir().join(format!(
            "dystil-segment-state-{}-{}.sqlite",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or_default()
        ));
        let store = SegmentStore::open(&path).await.unwrap();
        let mut state = LocalSyncState::default();
        state.next_segment_sequence = 2;
        let segment = sample_segment();
        store
            .replace_pending(
                &state,
                &[PendingSegment {
                    status: "stable".to_string(),
                    envelope: segment.clone(),
                }],
            )
            .await
            .unwrap();

        assert_eq!(store.load_state().await.unwrap().next_segment_sequence, 2);
        assert_eq!(
            store.stable_segments().await.unwrap(),
            vec![segment.clone()]
        );

        store
            .acknowledge(&[SegmentRevisionAck {
                segment_id: segment.segment_id.clone(),
                revision: 1,
                status: "accepted".to_string(),
            }])
            .await
            .unwrap();
        assert!(store.stable_segments().await.unwrap().is_empty());
        assert_eq!(
            store.load_state().await.unwrap().last_uploaded_segment_id,
            Some(segment.segment_id)
        );

        store.pool.close().await;
        let _ = std::fs::remove_file(path);
    }
}
