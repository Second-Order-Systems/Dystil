use chrono::Utc;
use dystil_protocol::{
    DeviceSyncStateResponse, ImageCompleteRequest, ImageCompleteResponse, ImagePrepareRequest,
    ImagePrepareResponse, SegmentUploadRequest, SegmentUploadResponse, SyncPolicy,
    WORK_INSIGHTS_IMAGE_SCHEMA_VERSION, WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION,
};
use flate2::write::GzEncoder;
use flate2::Compression;
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;

use crate::cursor::{recompute_cursor, resolved_cursor};
use crate::evidence::{filter_events, EvidenceFilterConfig};
use crate::segmenter::{build_segments, SegmentConfig};
use crate::state::{PendingSegment, SegmentStore};
use crate::types::{DystilSync, SyncError, SyncOutcome};
use crate::utils::sha256_hex;

const MAX_SEGMENTS_PER_UPLOAD: usize = 32;

impl DystilSync {
    fn device_request(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        let request = request.header("Authorization", format!("Device {}", self.device_token));
        let request = if let Some(version) = &self.app_version {
            request.header("X-Dystil-App-Version", version)
        } else {
            request
        };
        let request = if let Some(channel) = &self.build_channel {
            request.header("X-Dystil-Build-Channel", channel)
        } else {
            request
        };
        let request = if let Some(commit) = &self.build_commit {
            request.header("X-Dystil-Build-Commit", commit)
        } else {
            request
        };
        if self.sync_capabilities.is_empty() {
            request
        } else {
            request.header(
                "X-Dystil-Sync-Capabilities",
                self.sync_capabilities.join(","),
            )
        }
    }
    pub async fn sync_once(&self) -> Result<SyncOutcome, SyncError> {
        tracing::info!(
            local_segment_consent = self.local_permissions.segments,
            local_screenshot_consent = self.local_permissions.screenshots,
            "dystil-sync: applying local sync permissions"
        );
        if !self.local_permissions.segments {
            tracing::info!("dystil-sync: local segment consent disabled; skipping sync iteration");
            return Ok(SyncOutcome {
                uploaded_segments: 0,
                processed_events: 0,
                uploaded_images: 0,
                config: self.fallback_config.clone(),
            });
        }
        tracing::info!(
            db_path = %self.db_path.display(),
            state_db_path = %self.state_db_path.display(),
            cloud_base_url = %self.cloud_base_url,
            "dystil-sync: sync_once started"
        );
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.request_timeout_secs))
            .build()?;
        let config = self.fallback_config.clone();
        let store = SegmentStore::open(&self.state_db_path).await?;
        let mut state = store.load_state().await?;

        let mut effective_config = config.clone();
        match self.fetch_sync_policy(&client).await {
            Ok(policy) => {
                self.write_cached_policy(&policy)?;
                effective_config.policy = policy;
            }
            Err(err) => match self.read_cached_policy() {
                Ok(Some(policy)) => {
                    tracing::warn!(error = %err, policy_version = %policy.policy_version, "dystil-sync: remote policy unavailable; using cached policy");
                    effective_config.policy = policy;
                }
                Ok(None) => {
                    tracing::warn!(error = %err, "dystil-sync: remote policy unavailable; using compiled fallback")
                }
                Err(cache_err) => {
                    tracing::warn!(error = %err, cache_error = %cache_err, "dystil-sync: policy unavailable and cache unreadable; using compiled fallback")
                }
            },
        }
        let is_cold_start = state.cursor.screen_frame.last_id.is_none()
            && state.cursor.input_event.last_id.is_none()
            && state.next_segment_sequence == 1;

        if is_cold_start {
            match self.fetch_device_sync_state(&client).await {
                Ok(sync_state) => {
                    tracing::info!(
                        cloud_max_sequence = sync_state.max_sequence,
                        cloud_last_segment_id = ?sync_state.last_segment_id,
                        "dystil-sync: cold start — resolved cloud device state"
                    );
                    state.next_segment_sequence =
                        state.next_segment_sequence.max(sync_state.max_sequence + 1);
                    state.last_uploaded_segment_id = sync_state
                        .last_segment_id
                        .or(state.last_uploaded_segment_id.take());
                }
                Err(err) => {
                    tracing::warn!(
                        error = %err,
                        "dystil-sync: cold start cloud device state fetch failed, falling back to full cold start"
                    );
                }
            }
        }

        let effective_cursor = resolved_cursor(state.cursor.clone(), &effective_config, Utc::now());
        let events = self
            .read_events(&effective_cursor, &effective_config)
            .await?;
        let processed_events = events.len();
        let filter_outcome = filter_events(&events, &EvidenceFilterConfig::default())?;
        let new_items = filter_outcome.items;
        state.cursor = recompute_cursor(&state.cursor, &events);

        let existing = store.load_pending().await?;
        let mut preserved_stable: Vec<PendingSegment> = existing
            .iter()
            .filter(|segment| segment.status == "stable")
            .cloned()
            .collect();
        let mutable: Vec<PendingSegment> = existing
            .into_iter()
            .filter(|segment| segment.status != "stable")
            .collect();
        let mut items = BTreeMap::new();
        for item in mutable
            .iter()
            .flat_map(|segment| segment.envelope.items.iter())
            .chain(new_items.iter())
        {
            items.insert(item.item_id.clone(), item.clone());
        }

        let first_sequence = mutable
            .first()
            .map(|segment| segment.envelope.device_sequence)
            .unwrap_or(state.next_segment_sequence);
        let previous_segment_id = mutable
            .first()
            .and_then(|segment| segment.envelope.previous_segment_id.clone())
            .or_else(|| {
                preserved_stable
                    .last()
                    .map(|segment| segment.envelope.segment_id.clone())
            })
            .or_else(|| state.last_uploaded_segment_id.clone());
        let now = Utc::now();
        let segment_config = SegmentConfig::from_policy(&effective_config.policy);
        let rebuilt = build_segments(
            items.into_values().collect(),
            &segment_config,
            &self.machine_id,
            first_sequence,
            previous_segment_id,
            now,
        )?;
        let rebuilt_len = rebuilt.len();

        for (index, envelope) in rebuilt.into_iter().enumerate() {
            let age_seconds = (now - envelope.end_time).num_seconds();
            let is_last = index + 1 == rebuilt_len;
            let boundary_reached = !is_last
                || age_seconds >= segment_config.inactivity_seconds
                || (envelope.end_time - envelope.start_time).num_seconds()
                    >= segment_config.max_duration_seconds
                || envelope.token_estimate >= segment_config.max_tokens;

            let status = if boundary_reached { "stable" } else { "open" };
            preserved_stable.push(PendingSegment {
                status: status.to_string(),
                envelope,
            });
        }
        state.next_segment_sequence = first_sequence + rebuilt_len as u64;
        store.replace_pending(&state, &preserved_stable).await?;

        let mut uploaded_segments = 0;
        let mut stable = store.stable_segments().await?;
        for envelope in &mut stable {
            envelope.refresh_content_hash()?;
        }
        for chunk in stable.chunks(MAX_SEGMENTS_PER_UPLOAD) {
            let response = self.upload_segments(&client, chunk.to_vec()).await?;
            uploaded_segments += response.accepted.len();
            store.acknowledge(&response.accepted).await?;
        }
        let uploaded_images = if self.local_permissions.screenshots {
            self.sync_images(&client, &effective_config.policy).await?
        } else {
            tracing::info!("dystil-sync: local screenshot consent disabled; skipping image sync");
            0
        };
        tracing::info!(
            processed_events,
            uploaded_segments,
            uploaded_images,
            "dystil-sync: segment iteration completed"
        );

        Ok(SyncOutcome {
            uploaded_segments,
            processed_events,
            uploaded_images,
            config: effective_config,
        })
    }

    async fn fetch_device_sync_state(
        &self,
        client: &reqwest::Client,
    ) -> Result<DeviceSyncStateResponse, SyncError> {
        let response = self
            .device_request(client.get(format!(
                "{}/v1/ingest/segments/device-state",
                self.cloud_base_url
            )))
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SyncError::Unauthorized);
            }
            return Err(SyncError::Message(format!(
                "device state fetch failed with status {status}"
            )));
        }
        Ok(response.json::<DeviceSyncStateResponse>().await?)
    }

    async fn fetch_sync_policy(&self, client: &reqwest::Client) -> Result<SyncPolicy, SyncError> {
        let response = self
            .device_request(client.get(format!("{}/v1/ingest/config", self.cloud_base_url)))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(SyncError::Message(format!(
                "sync policy fetch failed with status {}",
                response.status()
            )));
        }
        let policy = response.json::<SyncPolicy>().await?;
        validate_policy(&policy)?;
        Ok(policy)
    }

    fn policy_cache_path(&self) -> std::path::PathBuf {
        self.state_db_path.with_extension("policy.json")
    }

    fn write_cached_policy(&self, policy: &SyncPolicy) -> Result<(), SyncError> {
        let path = self.policy_cache_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec(policy)?)?;
        Ok(())
    }

    fn read_cached_policy(&self) -> Result<Option<SyncPolicy>, SyncError> {
        let path = self.policy_cache_path();
        if !path.exists() {
            return Ok(None);
        }
        let modified = fs::metadata(&path)?.modified()?;
        let age = std::time::SystemTime::now()
            .duration_since(modified)
            .unwrap_or_default();
        let policy: SyncPolicy = serde_json::from_slice(&fs::read(&path)?)?;
        validate_policy(&policy)?;
        if age > std::time::Duration::from_secs(24 * 60 * 60) {
            return Ok(None);
        }
        Ok(Some(policy))
    }

    pub(crate) async fn upload_segments(
        &self,
        client: &reqwest::Client,
        segments: Vec<dystil_protocol::SegmentEnvelope>,
    ) -> Result<SegmentUploadResponse, SyncError> {
        let request = SegmentUploadRequest {
            schema_version: WORK_INSIGHTS_SEGMENT_SCHEMA_VERSION,
            client_sent_at: Utc::now(),
            segments,
        };
        let body = serde_json::to_vec(&request)?;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&body)?;
        let compressed = encoder.finish()?;
        let sha = sha256_hex(&compressed);
        let response = self
            .device_request(client.post(format!("{}/v1/ingest/segments", self.cloud_base_url)))
            .header("Content-Type", "application/json")
            .header("Content-Encoding", "gzip")
            .header("X-Dystil-Payload-Sha256", sha)
            .body(compressed)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SyncError::Unauthorized);
            }
            tracing::warn!(
                status = %status,
                body = %body,
                "dystil-sync: segment upload rejected"
            );
            return Err(SyncError::Message(format!(
                "segment upload failed with status {}: {}",
                status, body
            )));
        }
        Ok(response.json::<SegmentUploadResponse>().await?)
    }

    pub(crate) async fn prepare_images(
        &self,
        client: &reqwest::Client,
        request: &ImagePrepareRequest,
    ) -> Result<ImagePrepareResponse, SyncError> {
        let response = self
            .device_request(
                client.post(format!("{}/v1/ingest/images/prepare", self.cloud_base_url)),
            )
            .json(request)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SyncError::Unauthorized);
            }
            return Err(SyncError::Message(format!(
                "image prepare failed with status {}: {}",
                status, body
            )));
        }
        Ok(response.json::<ImagePrepareResponse>().await?)
    }

    pub(crate) async fn complete_images(
        &self,
        client: &reqwest::Client,
        images: Vec<dystil_protocol::ImageCompleteItem>,
    ) -> Result<ImageCompleteResponse, SyncError> {
        let response = self
            .device_request(
                client.post(format!("{}/v1/ingest/images/complete", self.cloud_base_url)),
            )
            .json(&ImageCompleteRequest {
                schema_version: WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
                images,
            })
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(SyncError::Unauthorized);
            }
            return Err(SyncError::Message(format!(
                "image complete failed with status {}: {}",
                status, body
            )));
        }
        Ok(response.json::<ImageCompleteResponse>().await?)
    }
}

fn validate_policy(policy: &SyncPolicy) -> Result<(), SyncError> {
    let image = &policy.image_sync;
    let segmenting = &policy.segmenting;
    if policy.schema_version != 1
        || policy.policy_version.trim().is_empty()
        || !(30..=86_400).contains(&policy.refresh_after_seconds)
        || !(1..=1_000).contains(&image.max_uploads_per_pass)
        || !(1_048_576..=2 * 1024 * 1024 * 1024).contains(&image.max_upload_bytes_per_pass)
        || !(40..=100).contains(&image.jpeg_quality)
        || !(320..=7_680).contains(&image.max_jpeg_width)
        || !(500..=100_000).contains(&segmenting.max_tokens)
        || !(30..=3_600).contains(&segmenting.inactivity_seconds)
        || !(60..=7_200).contains(&segmenting.max_duration_seconds)
    {
        return Err(SyncError::Message("invalid remote sync policy".to_string()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SyncConfig;
    use tempfile::tempdir;

    fn sync_with_permissions(local_permissions: crate::LocalSyncPermissions) -> DystilSync {
        let temp = tempdir().unwrap();
        let root = temp.keep();
        DystilSync {
            db_path: root.join("missing-capture.sqlite"),
            state_db_path: root.join("missing-sync-state.sqlite"),
            cloud_base_url: "http://127.0.0.1:9".to_string(),
            device_token: "test-device-token".to_string(),
            machine_id: "test-machine".to_string(),
            fallback_config: SyncConfig::default(),
            request_timeout_secs: 1,
            app_version: None,
            build_channel: None,
            build_commit: None,
            sync_capabilities: Vec::new(),
            local_permissions,
        }
    }

    #[tokio::test]
    async fn segment_consent_disabled_skips_all_storage_and_network_work() {
        let sync = sync_with_permissions(crate::LocalSyncPermissions {
            segments: false,
            screenshots: false,
        });

        let outcome = sync.sync_once().await.unwrap();

        assert_eq!(outcome.uploaded_segments, 0);
        assert_eq!(outcome.processed_events, 0);
        assert_eq!(outcome.uploaded_images, 0);
        assert!(!sync.state_db_path.exists());
    }
}
