use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Cursor;

use chrono::Duration;
use dystil_protocol::{
    ImageCompleteItem, ImageFilterDecision, ImageManifest, ImagePrepareRequest, ImageSyncMetadata,
    ImageSyncMode, ImageSyncPolicy, SyncPolicy, WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::GenericImageView;
use sha2::Digest;
use sqlx::{Row, SqlitePool};

use crate::types::{
    DystilSync, ImageCandidate, ImageSyncCache, MonitorSelectionState, PendingCompleteImage,
    PendingUploadRetry, PreparedImage, SyncError,
};
use crate::utils::sha256_hex;

const MAX_IMAGE_RETRY_COUNT: u8 = 1;
const SNAPSHOT_MAX_AGE_HOURS: i64 = 2;
impl DystilSync {
    /// Delete local snapshot files older than the hard two-hour retention
    /// window, independently of cloud sync state.
    pub async fn cleanup_expired_snapshots_once(
        db_path: &std::path::Path,
    ) -> Result<(), SyncError> {
        let cutoff = chrono::Utc::now() - Duration::hours(SNAPSHOT_MAX_AGE_HOURS);
        let db_url = format!("sqlite:{}?mode=ro", db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        let rows = sqlx::query(
            r#"
            SELECT DISTINCT snapshot_path
            FROM frames
            WHERE timestamp < ?1
              AND snapshot_path IS NOT NULL
              AND snapshot_path != ''
            "#,
        )
        .bind(cutoff.to_rfc3339())
        .fetch_all(&pool)
        .await?;

        let mut deleted_count = 0usize;
        let mut failed_count = 0usize;
        for row in rows {
            let snapshot_path: String = row.try_get("snapshot_path")?;
            match fs::remove_file(&snapshot_path) {
                Ok(_) => deleted_count += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    failed_count += 1;
                    tracing::warn!(snapshot_path = %snapshot_path, error = %error, "dystil-sync: expired snapshot cleanup failed");
                }
            }
        }
        tracing::info!(cutoff = %cutoff.to_rfc3339(), deleted_count, failed_count, "dystil-sync: expired snapshot cleanup completed");
        Ok(())
    }

    pub(crate) async fn sync_images(
        &self,
        client: &reqwest::Client,
        policy: &SyncPolicy,
    ) -> Result<usize, SyncError> {
        let mut cache = self.read_image_cache()?;
        let mut completed_count = 0usize;
        completed_count += self
            .flush_pending_image_completions(client, &mut cache)
            .await?;
        completed_count += self
            .retry_pending_image_uploads(client, &mut cache, policy)
            .await?;

        let db_url = format!("sqlite:{}?mode=ro", self.db_path.display());
        let pool = SqlitePool::connect(&db_url).await?;
        let (candidates, next_monitor_state, monitor_state_checkpoints, max_eligible_frame_id) =
            self.read_image_candidates(
                &pool,
                cache.last_scanned_frame_id,
                &cache.monitor_state,
                &policy.image_sync,
            )
            .await?;
        if candidates.is_empty() {
            if let Some(max_eligible_frame_id) = max_eligible_frame_id {
                cache.last_scanned_frame_id =
                    cache.last_scanned_frame_id.max(max_eligible_frame_id);
                cache.monitor_state = next_monitor_state;
                self.write_image_cache(&cache)?;
            }
            tracing::info!(
                completed_count,
                last_scanned_frame_id = cache.last_scanned_frame_id,
                "dystil-sync: image sync found no new candidates"
            );
            return Ok(completed_count);
        }
        tracing::info!(
            candidate_count = candidates.len(),
            last_scanned_frame_id = cache.last_scanned_frame_id,
            "dystil-sync: image sync selected candidates"
        );
        let selected_candidate_count = candidates.len();

        let mut prepared =
            Vec::with_capacity(candidates.len().min(policy.image_sync.max_uploads_per_pass));
        let mut skipped_prepare_count = 0usize;
        let mut prepared_bytes = 0u64;
        let mut max_processed_frame_id = None;
        let mut attempted_count = 0usize;
        for candidate in &candidates {
            if attempted_count >= policy.image_sync.max_uploads_per_pass {
                break;
            }
            attempted_count += 1;
            match self.prepare_image(candidate, policy).await {
                Ok(image) => {
                    let image_bytes = image.jpeg_bytes.len() as u64;
                    if image_bytes > policy.image_sync.max_upload_bytes_per_pass {
                        tracing::warn!(
                            frame_id = candidate.frame_id,
                            image_bytes,
                            max_upload_bytes_per_pass = policy.image_sync.max_upload_bytes_per_pass,
                            "dystil-sync: skipping image larger than the configured per-pass byte limit"
                        );
                        skipped_prepare_count += 1;
                        max_processed_frame_id = Some(candidate.frame_id);
                    } else if !image_fits_upload_batch(
                        prepared.len(),
                        prepared_bytes,
                        image_bytes,
                        &policy.image_sync,
                    ) {
                        // Keep this and later frames for the next pass by not advancing the
                        // scan cursor beyond the images already admitted to this batch.
                        break;
                    } else {
                        prepared_bytes += image_bytes;
                        max_processed_frame_id = Some(candidate.frame_id);
                        prepared.push((candidate.clone(), image));
                    }
                }
                Err(err) => {
                    if matches!(&err, SyncError::Io(io_err) if io_err.kind() == std::io::ErrorKind::NotFound)
                    {
                        tracing::debug!(
                            frame_id = candidate.frame_id,
                            source_path = %candidate.source_path,
                            "dystil-sync: skipping expired image candidate"
                        );
                    } else {
                        tracing::warn!(
                            frame_id = candidate.frame_id,
                            source_path = %candidate.source_path,
                            error = %err,
                            "dystil-sync: skipping image candidate because local snapshot could not be prepared"
                        );
                    }
                    skipped_prepare_count += 1;
                    max_processed_frame_id = Some(candidate.frame_id);
                }
            }
        }
        if prepared.is_empty() {
            if attempted_count == selected_candidate_count {
                if let Some(max_eligible_frame_id) = max_eligible_frame_id {
                    cache.last_scanned_frame_id =
                        cache.last_scanned_frame_id.max(max_eligible_frame_id);
                    cache.monitor_state = next_monitor_state;
                }
            } else if let Some(max_processed_frame_id) = max_processed_frame_id {
                cache.last_scanned_frame_id =
                    cache.last_scanned_frame_id.max(max_processed_frame_id);
                persist_monitor_state_checkpoint(
                    &mut cache.monitor_state,
                    &monitor_state_checkpoints,
                    max_processed_frame_id,
                );
                self.write_image_cache(&cache)?;
            }
            tracing::info!(
                skipped_prepare_count,
                last_scanned_frame_id = cache.last_scanned_frame_id,
                "dystil-sync: image sync had no uploadable candidates after local snapshot preparation"
            );
            return Ok(completed_count);
        }
        tracing::info!(
            prepared_count = prepared.len(),
            prepared_bytes,
            skipped_prepare_count,
            "dystil-sync: image sync prepared normalized jpegs"
        );

        let response = self
            .prepare_images(
                client,
                &ImagePrepareRequest {
                    schema_version: WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
                    images: prepared
                        .iter()
                        .map(|(_, image)| image.manifest.clone())
                        .collect(),
                },
            )
            .await?;
        tracing::info!(
            prepared_count = response.results.len(),
            "dystil-sync: image sync received upload tickets"
        );
        let mut prepare_results = std::collections::BTreeMap::new();
        for result in response.results {
            prepare_results.insert(result.client_image_key.clone(), result);
        }

        let mut uploaded_count = 0usize;
        let mut queued_retry_count = 0usize;
        for (candidate, mut image) in prepared {
            let result = prepare_results
                .remove(&image.manifest.client_image_key)
                .ok_or_else(|| {
                    SyncError::Message(format!(
                        "prepare response missing client_image_key {}",
                        image.manifest.client_image_key
                    ))
                })?;
            image.complete_item.image_id = result.image_id.clone();
            let ticket = result.upload_ticket.ok_or_else(|| {
                SyncError::Message(format!("missing upload ticket for {}", result.image_id))
            })?;
            let upload_result = client
                .put(ticket.upload_url)
                .header("Content-Type", &image.complete_item.mime_type)
                .body(image.jpeg_bytes.clone())
                .send()
                .await?
                .error_for_status();
            match upload_result {
                Ok(_) => {
                    let complete_item = image.complete_item;
                    cache.pending_complete.push(PendingCompleteImage {
                        item: complete_item,
                        retry_count: 0,
                    });
                    self.write_image_cache(&cache)?;
                    uploaded_count += 1;
                }
                Err(err) => {
                    if err.status() == Some(reqwest::StatusCode::UNAUTHORIZED) {
                        return Err(SyncError::Unauthorized);
                    }
                    tracing::warn!(
                        frame_id = candidate.frame_id,
                        client_image_key = %image.manifest.client_image_key,
                        error = %err,
                        "dystil-sync: image upload failed, queued one retry"
                    );
                    cache.pending_upload_retry.push(PendingUploadRetry {
                        candidate,
                        retry_count: 1,
                    });
                    queued_retry_count += 1;
                }
            }
        }
        tracing::info!(
            uploaded_count,
            queued_retry_count,
            pending_complete = cache.pending_complete.len(),
            pending_upload_retry = cache.pending_upload_retry.len(),
            "dystil-sync: image sync uploaded objects to storage"
        );

        completed_count += self
            .flush_pending_image_completions(client, &mut cache)
            .await?;
        if attempted_count == selected_candidate_count {
            if let Some(max_eligible_frame_id) = max_eligible_frame_id {
                cache.last_scanned_frame_id =
                    cache.last_scanned_frame_id.max(max_eligible_frame_id);
                cache.monitor_state = next_monitor_state;
            }
        } else if let Some(max_processed_frame_id) = max_processed_frame_id {
            cache.last_scanned_frame_id = cache.last_scanned_frame_id.max(max_processed_frame_id);
            persist_monitor_state_checkpoint(
                &mut cache.monitor_state,
                &monitor_state_checkpoints,
                max_processed_frame_id,
            );
        }
        self.write_image_cache(&cache)?;
        Ok(completed_count)
    }

    fn image_cache_path(&self) -> std::path::PathBuf {
        self.state_db_path.with_extension("images.json")
    }

    pub(crate) fn read_image_cache(&self) -> Result<ImageSyncCache, SyncError> {
        Self::read_image_cache_at(&self.state_db_path)
    }

    pub(crate) fn read_image_cache_at(
        state_db_path: &std::path::Path,
    ) -> Result<ImageSyncCache, SyncError> {
        let path = state_db_path.with_extension("images.json");
        if !path.exists() {
            return Ok(ImageSyncCache::default());
        }
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    fn write_image_cache(&self, cache: &ImageSyncCache) -> Result<(), SyncError> {
        let path = self.image_cache_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec_pretty(cache)?)?;
        Ok(())
    }

    async fn flush_pending_image_completions(
        &self,
        client: &reqwest::Client,
        cache: &mut ImageSyncCache,
    ) -> Result<usize, SyncError> {
        if cache.pending_complete.is_empty() {
            return Ok(0);
        }
        let pending_count = cache.pending_complete.len();
        tracing::info!(
            pending_count,
            "dystil-sync: image sync replaying pending complete request"
        );
        let pending_items = cache.pending_complete.clone();
        let pending_complete_items = pending_items
            .iter()
            .map(|pending| pending.item.clone())
            .collect::<Vec<_>>();
        let complete_result = self
            .complete_images(client, pending_complete_items.clone())
            .await;
        if let Err(err) = complete_result {
            if matches!(err, SyncError::Unauthorized) {
                return Err(err);
            }
            let mut keep = Vec::new();
            let mut dropped_count = 0usize;
            for mut pending in pending_items {
                pending.retry_count += 1;
                if pending.retry_count > MAX_IMAGE_RETRY_COUNT {
                    dropped_count += 1;
                } else {
                    keep.push(pending);
                }
            }
            cache.pending_complete = keep;
            self.write_image_cache(cache)?;
            tracing::warn!(
                pending_count,
                kept_count = cache.pending_complete.len(),
                dropped_count,
                error = %err,
                "dystil-sync: image complete failed, pending items retained best-effort"
            );
            return Ok(0);
        }
        let max_completed_frame_id = pending_items
            .iter()
            .flat_map(|item| item.item.linked_frame_ids.iter().copied())
            .max();
        if let Some(max_completed_frame_id) = max_completed_frame_id {
            cache.last_scanned_frame_id = cache.last_scanned_frame_id.max(max_completed_frame_id);
        }
        cache.pending_complete.clear();
        self.write_image_cache(cache)?;
        tracing::info!(
            completed_count = pending_complete_items.len(),
            last_scanned_frame_id = cache.last_scanned_frame_id,
            "dystil-sync: image sync complete request acknowledged"
        );
        Ok(pending_complete_items.len())
    }

    async fn retry_pending_image_uploads(
        &self,
        client: &reqwest::Client,
        cache: &mut ImageSyncCache,
        policy: &SyncPolicy,
    ) -> Result<usize, SyncError> {
        if cache.pending_upload_retry.is_empty() {
            return Ok(0);
        }
        tracing::info!(
            pending_retry_count = cache.pending_upload_retry.len(),
            "dystil-sync: image sync replaying failed uploads"
        );
        let pending_retries = std::mem::take(&mut cache.pending_upload_retry);
        let mut completed_count = 0usize;
        let mut kept_retries = Vec::new();
        for mut retry in pending_retries {
            match self
                .upload_candidate_once(client, &retry.candidate, cache, policy)
                .await
            {
                Ok(true) => {
                    completed_count += self.flush_pending_image_completions(client, cache).await?;
                }
                Ok(false) => {}
                Err(SyncError::Unauthorized) => return Err(SyncError::Unauthorized),
                Err(err) => {
                    retry.retry_count += 1;
                    if retry.retry_count > MAX_IMAGE_RETRY_COUNT {
                        tracing::warn!(
                            frame_id = retry.candidate.frame_id,
                            error = %err,
                            "dystil-sync: image upload retry exhausted, dropping candidate"
                        );
                    } else {
                        tracing::warn!(
                            frame_id = retry.candidate.frame_id,
                            retry_count = retry.retry_count,
                            error = %err,
                            "dystil-sync: image upload retry failed, keeping candidate"
                        );
                        kept_retries.push(retry);
                    }
                }
            }
        }
        cache.pending_upload_retry = kept_retries;
        self.write_image_cache(cache)?;
        Ok(completed_count)
    }

    async fn upload_candidate_once(
        &self,
        client: &reqwest::Client,
        candidate: &ImageCandidate,
        cache: &mut ImageSyncCache,
        policy: &SyncPolicy,
    ) -> Result<bool, SyncError> {
        let prepared = match self.prepare_image(candidate, policy).await {
            Ok(prepared) => prepared,
            Err(err) => {
                tracing::warn!(
                    frame_id = candidate.frame_id,
                    source_path = %candidate.source_path,
                    error = %err,
                    "dystil-sync: dropping retry candidate because local snapshot could not be prepared"
                );
                return Ok(false);
            }
        };
        let response = self
            .prepare_images(
                client,
                &ImagePrepareRequest {
                    schema_version: WORK_INSIGHTS_IMAGE_SCHEMA_VERSION,
                    images: vec![prepared.manifest.clone()],
                },
            )
            .await?;
        let result = response.results.into_iter().next().ok_or_else(|| {
            SyncError::Message("prepare response missing image result".to_string())
        })?;
        let ticket = result.upload_ticket.ok_or_else(|| {
            SyncError::Message(format!("missing upload ticket for {}", result.image_id))
        })?;
        client
            .put(ticket.upload_url)
            .header("Content-Type", &prepared.complete_item.mime_type)
            .body(prepared.jpeg_bytes)
            .send()
            .await?
            .error_for_status()?;
        let mut complete_item = prepared.complete_item;
        complete_item.image_id = result.image_id;
        cache.pending_complete.push(PendingCompleteImage {
            item: complete_item,
            retry_count: 0,
        });
        self.write_image_cache(cache)?;
        Ok(true)
    }

    async fn read_image_candidates(
        &self,
        pool: &SqlitePool,
        last_scanned_frame_id: i64,
        previous_monitor_state: &BTreeMap<String, MonitorSelectionState>,
        image_policy: &ImageSyncPolicy,
    ) -> Result<
        (
            Vec<ImageCandidate>,
            BTreeMap<String, MonitorSelectionState>,
            BTreeMap<i64, BTreeMap<String, MonitorSelectionState>>,
            Option<i64>,
        ),
        SyncError,
    > {
        let rows = sqlx::query(
            r#"
            SELECT f.id,
                   f.timestamp,
                   f.device_name,
                   f.app_name,
                   f.window_name,
                   f.browser_url,
                   f.frame_text,
                   f.capture_trigger,
                   f.text_source,
                   f.snapshot_path AS source_path
            FROM frames f
            WHERE f.id > ?1
              AND f.snapshot_path IS NOT NULL
              AND f.snapshot_path != ''
            ORDER BY f.id ASC
            "#,
        )
        .bind(last_scanned_frame_id)
        .fetch_all(pool)
        .await?;
        let eligible_count = rows.len();
        let max_eligible_frame_id = rows
            .last()
            .map(|row| row.try_get::<i64, _>("id"))
            .transpose()?;

        let mut candidates = Vec::new();
        let mut next_monitor_state = previous_monitor_state.clone();
        let mut monitor_state_checkpoints = BTreeMap::new();

        for row in rows {
            let device_name = normalized_text(row.try_get::<String, _>("device_name")?);
            let app_name = normalized_optional_text(row.try_get::<Option<String>, _>("app_name")?);
            let window_name =
                normalized_optional_text(row.try_get::<Option<String>, _>("window_name")?);
            let browser_url =
                normalized_optional_text(row.try_get::<Option<String>, _>("browser_url")?);
            let candidate_text =
                normalized_optional_text(row.try_get::<Option<String>, _>("frame_text")?);
            let text_signature = build_text_signature(candidate_text.as_deref(), image_policy);
            let monitor_key = device_name.unwrap_or_else(|| "unknown_monitor".to_string());
            let monitor_state = next_monitor_state
                .entry(monitor_key)
                .or_insert_with(MonitorSelectionState::default);

            let text_change_distance = text_change_distance(
                monitor_state,
                row.try_get::<String, _>("timestamp")?.as_str(),
                &text_signature,
                image_policy,
            );

            let reason = if !monitor_state.initialized {
                Some("first_frame")
            } else if let (Some(current), Some(previous)) = (
                browser_url.as_ref(),
                monitor_state.last_browser_url.as_ref(),
            ) {
                (current != previous).then_some("url_change")
            } else if let (Some(current), Some(previous)) =
                (app_name.as_ref(), monitor_state.last_app_name.as_ref())
            {
                (current != previous).then_some("app_switch")
            } else if let (Some(current), Some(previous)) = (
                window_name.as_ref(),
                monitor_state.last_window_name.as_ref(),
            ) {
                (current != previous).then_some("window_change")
            } else if text_change_distance
                .map(|distance| distance >= image_policy.text_change_jaccard_distance_threshold)
                .unwrap_or(false)
            {
                Some("text_change")
            } else {
                None
            };

            let frame_id: i64 = row.try_get("id")?;
            let occurred_at = crate::utils::parse_sqlite_timestamp(
                row.try_get::<String, _>("timestamp")?.as_str(),
            );
            if reason == Some("text_change") {
                tracing::info!(
                    frame_id,
                    occurred_at = %occurred_at,
                    app_name = app_name.as_deref().unwrap_or("<NULL>"),
                    window_name = window_name.as_deref().unwrap_or("<NULL>"),
                    browser_url = browser_url.as_deref().unwrap_or("<NULL>"),
                    text_change_distance = text_change_distance.unwrap_or_default(),
                    threshold = image_policy.text_change_jaccard_distance_threshold,
                    signature_size = text_signature.len(),
                    "dystil-sync: image candidate selected by text change"
                );
            }

            candidates.push(ImageCandidate {
                frame_id,
                occurred_at,
                selection_reason: reason.unwrap_or("not_selected_by_filter").to_string(),
                source_path: row.try_get("source_path")?,
                app_name: app_name.clone(),
                capture_trigger: row.try_get("capture_trigger")?,
                text_source: row.try_get("text_source")?,
                filter_decision: ImageFilterDecision {
                    evaluator_version: image_policy.evaluator_version.clone(),
                    selected: reason.is_some(),
                    primary_reason: reason.map(str::to_string),
                    text_change_distance,
                    would_be_rate_limited: false,
                },
            });
            if reason.is_some() {
                update_monitor_state_after_selection(
                    monitor_state,
                    app_name,
                    window_name,
                    browser_url,
                    text_signature,
                    row.try_get::<String, _>("timestamp")?.as_str(),
                );
            } else {
                update_monitor_metadata(monitor_state, app_name, window_name, browser_url);
            }
            let needs_checkpoint = match image_policy.mode {
                ImageSyncMode::AllWithShadow => {
                    candidates.len() <= image_policy.max_uploads_per_pass
                }
                ImageSyncMode::Filtered => reason.is_some(),
            };
            if needs_checkpoint {
                monitor_state_checkpoints.insert(frame_id, next_monitor_state.clone());
            }
        }
        let pre_limit_selected_count = candidates
            .iter()
            .filter(|candidate| candidate.filter_decision.selected)
            .count();
        let accepted_ids: BTreeSet<i64> = limit_candidates_per_minute(
            candidates
                .iter()
                .filter(|candidate| candidate.filter_decision.selected)
                .cloned()
                .collect(),
            image_policy.max_selected_per_minute,
            image_policy.candidate_min_gap_seconds,
        )
        .into_iter()
        .map(|candidate| candidate.frame_id)
        .collect();
        for candidate in &mut candidates {
            if candidate.filter_decision.selected && !accepted_ids.contains(&candidate.frame_id) {
                candidate.filter_decision.selected = false;
                candidate.filter_decision.would_be_rate_limited = true;
                candidate.selection_reason = "rate_limited".to_string();
            }
        }
        let selected = apply_image_sync_mode(
            candidates,
            &image_policy.mode,
            image_policy.max_selected_per_minute,
            image_policy.candidate_min_gap_seconds,
        );

        tracing::info!(
            eligible_count,
            pre_limit_selected_count,
            selected_count = selected.len(),
            discarded_count = eligible_count.saturating_sub(selected.len()),
            rate_limited_count = pre_limit_selected_count.saturating_sub(selected.len()),
            last_scanned_frame_id,
            "dystil-sync: image candidate filtering completed"
        );

        Ok((
            selected,
            next_monitor_state,
            monitor_state_checkpoints,
            max_eligible_frame_id,
        ))
    }

    pub(crate) async fn cleanup_snapshots_before_cursor(
        pool: &SqlitePool,
        cache: &ImageSyncCache,
    ) -> Result<(), SyncError> {
        let Some(max_cleanup_frame_id) = max_snapshot_cleanup_frame_id(cache) else {
            return Ok(());
        };
        if max_cleanup_frame_id <= 0 {
            return Ok(());
        }

        let rows = sqlx::query(
            r#"
            SELECT DISTINCT snapshot_path
            FROM frames
            WHERE id <= ?1
              AND snapshot_path IS NOT NULL
              AND snapshot_path != ''
            "#,
        )
        .bind(max_cleanup_frame_id)
        .fetch_all(pool)
        .await?;
        let mut deleted_count = 0usize;
        let mut failed_count = 0usize;
        for row in rows {
            let snapshot_path: String = row.try_get("snapshot_path")?;
            match fs::remove_file(&snapshot_path) {
                Ok(_) => deleted_count += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    failed_count += 1;
                    tracing::warn!(snapshot_path = %snapshot_path, error = %error, "dystil-sync: synced snapshot cleanup failed");
                }
            }
        }
        tracing::info!(
            max_cleanup_frame_id,
            deleted_count,
            failed_count,
            "dystil-sync: synced snapshot cleanup completed"
        );
        Ok(())
    }

    async fn prepare_image(
        &self,
        candidate: &ImageCandidate,
        policy: &SyncPolicy,
    ) -> Result<PreparedImage, SyncError> {
        let dynamic = image::open(&candidate.source_path)
            .map_err(|err| SyncError::Message(format!("failed to open snapshot image: {err}")))?;

        let normalized = if dynamic.width() > policy.image_sync.max_jpeg_width {
            dynamic.resize(
                policy.image_sync.max_jpeg_width,
                u32::MAX,
                FilterType::Triangle,
            )
        } else {
            dynamic
        };
        let mut jpeg_bytes = Vec::new();
        {
            let mut cursor = Cursor::new(&mut jpeg_bytes);
            let mut encoder =
                JpegEncoder::new_with_quality(&mut cursor, policy.image_sync.jpeg_quality);
            encoder.encode_image(&normalized).map_err(|err| {
                SyncError::Message(format!("failed to encode normalized jpeg: {err}"))
            })?;
        }
        let (width, height) = normalized.dimensions();
        let content_hash = format!("sha256:{}", sha256_hex(&jpeg_bytes));
        let client_image_key = format!("frame:{}", candidate.frame_id);
        let sync_metadata = Some(ImageSyncMetadata {
            sync_mode: policy.image_sync.mode.clone(),
            policy_version: policy.policy_version.clone(),
            capture_trigger: candidate.capture_trigger.clone(),
            text_source: candidate.text_source.clone(),
            filter_decision: candidate.filter_decision.clone(),
        });
        let manifest = ImageManifest {
            client_image_key: client_image_key.clone(),
            content_hash: content_hash.clone(),
            mime_type: "image/jpeg".to_string(),
            byte_size: jpeg_bytes.len() as u64,
            width,
            height,
            selection_reason: candidate.selection_reason.clone(),
            linked_frame_ids: vec![candidate.frame_id],
            first_frame_timestamp: Some(candidate.occurred_at),
            last_frame_timestamp: Some(candidate.occurred_at),
            sync_metadata: sync_metadata.clone(),
        };
        let complete_item = ImageCompleteItem {
            image_id: String::new(),
            client_image_key,
            content_hash,
            mime_type: "image/jpeg".to_string(),
            byte_size: jpeg_bytes.len() as u64,
            width,
            height,
            selection_reason: candidate.selection_reason.clone(),
            linked_frame_ids: vec![candidate.frame_id],
            sync_metadata,
        };
        Ok(PreparedImage {
            manifest,
            complete_item,
            jpeg_bytes,
        })
    }
}

fn normalized_text(value: String) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn normalized_optional_text(value: Option<String>) -> Option<String> {
    value.and_then(normalized_text)
}

fn update_monitor_metadata(
    monitor_state: &mut MonitorSelectionState,
    app_name: Option<String>,
    window_name: Option<String>,
    browser_url: Option<String>,
) {
    if let Some(app_name) = app_name {
        monitor_state.last_app_name = Some(app_name);
    }
    if let Some(window_name) = window_name {
        monitor_state.last_window_name = Some(window_name);
    }
    if let Some(browser_url) = browser_url {
        monitor_state.last_browser_url = Some(browser_url);
    }
    monitor_state.initialized = true;
}

fn update_monitor_state_after_selection(
    monitor_state: &mut MonitorSelectionState,
    app_name: Option<String>,
    window_name: Option<String>,
    browser_url: Option<String>,
    selected_text_signature: Vec<u64>,
    selected_at: &str,
) {
    update_monitor_metadata(monitor_state, app_name, window_name, browser_url);
    monitor_state.last_selected_at = Some(crate::utils::parse_sqlite_timestamp(selected_at));
    monitor_state.last_selected_text_signature = selected_text_signature;
}

fn text_change_distance(
    monitor_state: &MonitorSelectionState,
    occurred_at: &str,
    current_signature: &[u64],
    policy: &ImageSyncPolicy,
) -> Option<f64> {
    if current_signature.len() < policy.min_text_change_tokens {
        return None;
    }
    let Some(last_selected_at) = monitor_state.last_selected_at else {
        return None;
    };
    let occurred_at = crate::utils::parse_sqlite_timestamp(occurred_at);
    if occurred_at - last_selected_at < Duration::seconds(policy.stable_text_change_min_seconds) {
        return None;
    }
    if monitor_state.last_selected_text_signature.is_empty() {
        return None;
    }
    Some(jaccard_distance(
        &monitor_state.last_selected_text_signature,
        current_signature,
    ))
}

fn build_text_signature(text: Option<&str>, policy: &ImageSyncPolicy) -> Vec<u64> {
    let Some(text) = text else {
        return Vec::new();
    };
    if text.len() < policy.min_text_change_chars {
        return Vec::new();
    }
    let tokens = normalized_tokens(text);
    if tokens.len() < policy.min_text_change_tokens {
        return Vec::new();
    }
    let mut signature = BTreeSet::new();
    if tokens.len() >= 3 {
        for window in tokens.windows(3) {
            signature.insert(hash_text_fragment(&window.join(" ")));
        }
    } else {
        for token in tokens {
            signature.insert(hash_text_fragment(&token));
        }
    }
    signature.into_iter().collect()
}

fn normalized_tokens(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(|token| token.trim().to_ascii_lowercase())
        .filter(|token| token.len() >= 2)
        .collect()
}

fn hash_text_fragment(fragment: &str) -> u64 {
    let digest = sha2::Sha256::digest(fragment.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    u64::from_be_bytes(bytes)
}

fn jaccard_distance(left: &[u64], right: &[u64]) -> f64 {
    let left: BTreeSet<u64> = left.iter().copied().collect();
    let right: BTreeSet<u64> = right.iter().copied().collect();
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count();
    if union == 0 {
        0.0
    } else {
        1.0 - (intersection as f64 / union as f64)
    }
}

fn image_fits_upload_batch(
    current_count: usize,
    current_bytes: u64,
    next_bytes: u64,
    policy: &ImageSyncPolicy,
) -> bool {
    current_count < policy.max_uploads_per_pass
        && current_bytes.saturating_add(next_bytes) <= policy.max_upload_bytes_per_pass
}

fn max_snapshot_cleanup_frame_id(cache: &ImageSyncCache) -> Option<i64> {
    let retry_floor = cache
        .pending_complete
        .iter()
        .flat_map(|pending| pending.item.linked_frame_ids.iter().copied())
        .chain(
            cache
                .pending_upload_retry
                .iter()
                .map(|retry| retry.candidate.frame_id),
        )
        .min();

    match retry_floor {
        Some(frame_id) => Some((frame_id - 1).min(cache.last_scanned_frame_id)),
        None if cache.last_scanned_frame_id > 0 => Some(cache.last_scanned_frame_id),
        None => None,
    }
}

fn persist_monitor_state_checkpoint(
    persisted: &mut BTreeMap<String, MonitorSelectionState>,
    checkpoints: &BTreeMap<i64, BTreeMap<String, MonitorSelectionState>>,
    frame_id: i64,
) {
    if let Some(checkpoint) = checkpoints.get(&frame_id) {
        *persisted = checkpoint.clone();
    }
}

fn apply_image_sync_mode(
    candidates: Vec<ImageCandidate>,
    mode: &ImageSyncMode,
    max_candidates_per_minute: usize,
    min_gap_secs: i64,
) -> Vec<ImageCandidate> {
    let eligible = match mode {
        // All candidates continue through the shadow evaluator, but the
        // configured per-capture-minute ceiling is a real selection limit.
        ImageSyncMode::AllWithShadow => candidates,
        ImageSyncMode::Filtered => candidates
            .into_iter()
            .filter(|candidate| candidate.filter_decision.selected)
            .collect(),
    };
    limit_candidates_per_minute(eligible, max_candidates_per_minute, min_gap_secs)
}

fn limit_candidates_per_minute(
    candidates: Vec<ImageCandidate>,
    max_candidates_per_minute: usize,
    min_gap_secs: i64,
) -> Vec<ImageCandidate> {
    let mut grouped: BTreeMap<i64, Vec<ImageCandidate>> = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry(candidate.occurred_at.timestamp() / 60)
            .or_default()
            .push(candidate);
    }

    let mut limited = Vec::new();
    for mut bucket in grouped.into_values() {
        bucket.sort_by_key(|candidate| (candidate.occurred_at, candidate.frame_id));
        if bucket.len() <= max_candidates_per_minute {
            limited.extend(bucket);
            continue;
        }

        let mut chosen_indices = Vec::new();
        let mut seen_apps = BTreeSet::new();
        for (index, candidate) in bucket.iter().enumerate() {
            let Some(app_name) = candidate.app_name.as_ref() else {
                continue;
            };
            if seen_apps.insert(app_name.clone()) {
                chosen_indices.push(index);
                if chosen_indices.len() == max_candidates_per_minute {
                    break;
                }
            }
        }

        if chosen_indices.len() < max_candidates_per_minute {
            let mut last_selected_at = chosen_indices
                .last()
                .map(|index| bucket[*index].occurred_at);
            for (index, candidate) in bucket.iter().enumerate() {
                if chosen_indices.contains(&index) {
                    continue;
                }
                let allow = last_selected_at
                    .map(|last| (candidate.occurred_at - last).num_seconds() >= min_gap_secs)
                    .unwrap_or(true);
                if allow {
                    chosen_indices.push(index);
                    last_selected_at = Some(candidate.occurred_at);
                    if chosen_indices.len() == max_candidates_per_minute {
                        break;
                    }
                }
            }
        }

        if chosen_indices.len() < max_candidates_per_minute {
            for index in 0..bucket.len() {
                if !chosen_indices.contains(&index) {
                    chosen_indices.push(index);
                    if chosen_indices.len() == max_candidates_per_minute {
                        break;
                    }
                }
            }
        }

        chosen_indices.sort_unstable();
        for index in chosen_indices {
            limited.push(bucket[index].clone());
        }
    }

    limited.sort_by_key(|candidate| (candidate.occurred_at, candidate.frame_id));
    limited
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::tempdir;

    fn candidate(frame_id: i64, selected: bool) -> ImageCandidate {
        ImageCandidate {
            frame_id,
            occurred_at: Utc::now(),
            selection_reason: if selected {
                "first_frame"
            } else {
                "not_selected_by_filter"
            }
            .to_string(),
            source_path: format!("/tmp/{frame_id}.jpg"),
            app_name: None,
            capture_trigger: Some("periodic".to_string()),
            text_source: Some("ocr".to_string()),
            filter_decision: ImageFilterDecision {
                evaluator_version: "test".to_string(),
                selected,
                primary_reason: selected.then(|| "first_frame".to_string()),
                text_change_distance: None,
                would_be_rate_limited: false,
            },
        }
    }

    #[test]
    fn all_with_shadow_keeps_candidates_the_legacy_filter_would_drop() {
        let candidates = vec![candidate(1, true), candidate(2, false)];
        let selected = apply_image_sync_mode(candidates, &ImageSyncMode::AllWithShadow, 3, 20);
        assert_eq!(selected.len(), 2);
        assert!(!selected[1].filter_decision.selected);
    }

    #[test]
    fn filtered_mode_keeps_only_legacy_filter_matches() {
        let candidates = vec![candidate(1, true), candidate(2, false)];
        let selected = apply_image_sync_mode(candidates, &ImageSyncMode::Filtered, 3, 20);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].frame_id, 1);
    }

    #[test]
    fn all_with_shadow_enforces_the_per_capture_minute_ceiling() {
        let mut candidates = (1..=5)
            .map(|frame_id| candidate(frame_id, frame_id == 1))
            .collect::<Vec<_>>();
        let minute = Utc::now();
        for (offset, candidate) in candidates.iter_mut().enumerate() {
            candidate.occurred_at = minute + Duration::seconds(offset as i64 * 5);
        }

        let selected = apply_image_sync_mode(candidates, &ImageSyncMode::AllWithShadow, 3, 20);

        assert_eq!(selected.len(), 3);
        assert!(selected
            .iter()
            .any(|candidate| !candidate.filter_decision.selected));
    }

    #[test]
    fn upload_batch_respects_count_and_byte_limits() {
        let policy = crate::types::default_sync_policy().image_sync;
        let policy = ImageSyncPolicy {
            max_uploads_per_pass: 2,
            max_upload_bytes_per_pass: 10,
            ..policy
        };
        assert!(image_fits_upload_batch(0, 0, 7, &policy));
        assert!(!image_fits_upload_batch(1, 7, 4, &policy));
        assert!(!image_fits_upload_batch(2, 7, 1, &policy));
    }

    #[test]
    fn persisted_monitor_state_matches_the_processed_cursor() {
        let mut first = BTreeMap::new();
        first.insert(
            "display-1".to_string(),
            MonitorSelectionState {
                last_app_name: Some("first-app".to_string()),
                initialized: true,
                ..MonitorSelectionState::default()
            },
        );
        let mut later = first.clone();
        later.get_mut("display-1").unwrap().last_app_name = Some("later-app".to_string());

        let checkpoints = BTreeMap::from([(100, first), (1_000, later)]);
        let mut persisted = BTreeMap::new();
        persist_monitor_state_checkpoint(&mut persisted, &checkpoints, 100);

        assert_eq!(
            persisted["display-1"].last_app_name.as_deref(),
            Some("first-app")
        );
    }

    #[test]
    fn snapshot_cleanup_watermark_stays_before_pending_retry() {
        let mut cache = ImageSyncCache {
            last_scanned_frame_id: 100,
            ..ImageSyncCache::default()
        };
        assert_eq!(max_snapshot_cleanup_frame_id(&cache), Some(100));

        cache.pending_upload_retry.push(PendingUploadRetry {
            candidate: candidate(40, true),
            retry_count: 1,
        });
        assert_eq!(max_snapshot_cleanup_frame_id(&cache), Some(39));
    }

    #[tokio::test]
    async fn expired_snapshot_cleanup_keeps_files_younger_than_two_hours() {
        let temp = tempdir().unwrap();
        let db_path = temp.path().join("capture.sqlite");
        let old_snapshot = temp.path().join("old.jpg");
        let recent_snapshot = temp.path().join("recent.jpg");
        fs::write(&old_snapshot, b"old").unwrap();
        fs::write(&recent_snapshot, b"recent").unwrap();

        let pool = SqlitePool::connect(&format!("sqlite:{}?mode=rwc", db_path.display()))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE frames (
                id INTEGER PRIMARY KEY,
                timestamp TEXT NOT NULL,
                snapshot_path TEXT
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        for (id, timestamp, path) in [
            (1_i64, Utc::now() - Duration::hours(3), &old_snapshot),
            (2_i64, Utc::now() - Duration::hours(1), &recent_snapshot),
        ] {
            sqlx::query("INSERT INTO frames VALUES (?1, ?2, ?3)")
                .bind(id)
                .bind(timestamp.to_rfc3339())
                .bind(path.to_string_lossy().as_ref())
                .execute(&pool)
                .await
                .unwrap();
        }
        pool.close().await;

        DystilSync::cleanup_expired_snapshots_once(&db_path)
            .await
            .unwrap();

        assert!(!old_snapshot.exists());
        assert!(recent_snapshot.exists());
    }
}
