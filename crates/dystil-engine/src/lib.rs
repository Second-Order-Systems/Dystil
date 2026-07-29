use async_trait::async_trait;
use dystil_sync::{DystilSync, LocalSyncPermissions, SyncConfig, SyncError, SyncOutcome};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[async_trait]
pub trait EngineHost: Send + Sync {
    async fn device_token(&self) -> Result<Option<String>, String>;
    async fn machine_id(&self) -> Result<String, String>;
    async fn cloud_base_url(&self) -> Result<String, String>;
    async fn capture_db_path(&self) -> Result<PathBuf, String>;
    async fn sync_state_path(&self) -> Result<PathBuf, String>;
    async fn on_device_token_invalid(&self) -> Result<bool, String>;
    async fn local_sync_permissions(&self) -> Result<LocalSyncPermissions, String> {
        Ok(LocalSyncPermissions::default())
    }
    async fn app_version(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    async fn build_channel(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
    async fn build_commit(&self) -> Result<Option<String>, String> {
        Ok(None)
    }
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub fallback_sync_config: SyncConfig,
    pub request_timeout_secs: u64,
    pub idle_retry_secs: u64,
    pub error_retry_secs: u64,
    pub snapshot_cleanup_interval_secs: u64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        let fallback_sync_config = SyncConfig::default();
        Self {
            request_timeout_secs: fallback_sync_config.request_timeout_secs,
            idle_retry_secs: 10,
            error_retry_secs: 10,
            snapshot_cleanup_interval_secs: 30 * 60,
            fallback_sync_config,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("{0}")]
    Host(String),
    #[error(transparent)]
    Sync(#[from] SyncError),
}

#[derive(Debug, Clone)]
pub struct DystilEngine {
    config: EngineConfig,
}

impl DystilEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub async fn run_once<H: EngineHost>(
        &self,
        host: &H,
    ) -> Result<Option<SyncOutcome>, EngineError> {
        tracing::info!("dystil-engine: starting sync iteration");
        let Some(device_token) = host.device_token().await.map_err(EngineError::Host)? else {
            tracing::info!("dystil-engine: no device token yet, skipping iteration");
            return Ok(None);
        };
        let cloud_base_url = host.cloud_base_url().await.map_err(EngineError::Host)?;
        let db_path = host.capture_db_path().await.map_err(EngineError::Host)?;
        let state_db_path = host.sync_state_path().await.map_err(EngineError::Host)?;

        let sync = DystilSync {
            db_path,
            state_db_path,
            cloud_base_url,
            device_token,
            machine_id: host.machine_id().await.map_err(EngineError::Host)?,
            fallback_config: self.config.fallback_sync_config.clone(),
            request_timeout_secs: self.config.request_timeout_secs,
            app_version: host.app_version().await.map_err(EngineError::Host)?,
            build_channel: host.build_channel().await.map_err(EngineError::Host)?,
            build_commit: host.build_commit().await.map_err(EngineError::Host)?,
            sync_capabilities: vec![
                "image-shadow-decision-v1".to_string(),
                "remote-sync-policy-v1".to_string(),
                "evidence-v2".to_string(),
            ],
            local_permissions: host
                .local_sync_permissions()
                .await
                .map_err(EngineError::Host)?,
        };
        let outcome = match sync.sync_once().await {
            Ok(outcome) => outcome,
            Err(SyncError::Unauthorized) => {
                tracing::warn!("dystil-engine: device token unauthorized, clearing and requesting re-registration");
                let recovered = host
                    .on_device_token_invalid()
                    .await
                    .map_err(EngineError::Host)?;
                if recovered {
                    tracing::info!("dystil-engine: device token re-registered successfully");
                } else {
                    tracing::warn!("dystil-engine: device token re-registration failed");
                }
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        tracing::info!(
            uploaded_segments = outcome.uploaded_segments,
            processed_events = outcome.processed_events,
            uploaded_images = outcome.uploaded_images,
            sync_interval_secs = outcome.config.sync_interval_secs,
            "dystil-engine: sync iteration completed"
        );
        Ok(Some(outcome))
    }

    pub async fn run_forever<H: EngineHost + 'static>(self, host: Arc<H>) {
        // Run a best-effort pass as soon as the engine starts, then every
        // configured interval thereafter.
        let snapshot_cleanup_interval =
            Duration::from_secs(self.config.snapshot_cleanup_interval_secs.max(1));
        let mut last_snapshot_cleanup: Option<std::time::Instant> = None;
        loop {
            let delay = match self.run_once(host.as_ref()).await {
                Ok(Some(outcome)) => Duration::from_secs(outcome.config.sync_interval_secs.max(1)),
                Ok(None) => Duration::from_secs(self.config.idle_retry_secs.max(1)),
                Err(err) => {
                    tracing::warn!("dystil-engine sync iteration failed: {}", err);
                    Duration::from_secs(self.config.error_retry_secs.max(1))
                }
            };

            if last_snapshot_cleanup
                .is_none_or(|last| last.elapsed() >= snapshot_cleanup_interval)
            {
                let db_path = match host.capture_db_path().await {
                    Ok(db_path) => Some(db_path),
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "dystil-engine: unable to locate capture DB for expired snapshot cleanup"
                        );
                        None
                    }
                };

                if let Some(db_path) = db_path.as_deref() {
                    if let Err(err) = DystilSync::cleanup_expired_snapshots_once(db_path).await {
                        tracing::warn!(
                            error = %err,
                            "dystil-engine: expired snapshot cleanup failed"
                        );
                    }

                    match host.sync_state_path().await {
                        Ok(state_db_path) => {
                            if let Err(err) =
                                DystilSync::cleanup_synced_snapshots_once(db_path, &state_db_path)
                                    .await
                            {
                                tracing::warn!(
                                    error = %err,
                                    "dystil-engine: synced snapshot cleanup failed"
                                );
                            }
                        }
                        Err(err) => tracing::warn!(
                            error = %err,
                            "dystil-engine: unable to locate sync state for synced snapshot cleanup"
                        ),
                    }
                }
                tracing::info!("dystil-engine: periodic snapshot cleanup completed");
                last_snapshot_cleanup = Some(std::time::Instant::now());
            }
            sleep(delay).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NoTokenHost;

    #[async_trait]
    impl EngineHost for NoTokenHost {
        async fn device_token(&self) -> Result<Option<String>, String> {
            Ok(None)
        }

        async fn machine_id(&self) -> Result<String, String> {
            Ok("no-token-test-device".to_string())
        }

        async fn cloud_base_url(&self) -> Result<String, String> {
            Ok("http://localhost".to_string())
        }

        async fn capture_db_path(&self) -> Result<PathBuf, String> {
            Ok(PathBuf::from("/tmp/db.sqlite"))
        }

        async fn sync_state_path(&self) -> Result<PathBuf, String> {
            Ok(PathBuf::from("/tmp/dystil-segment-sync.sqlite"))
        }

        async fn on_device_token_invalid(&self) -> Result<bool, String> {
            Ok(false)
        }
    }

    struct ErrorHost;

    #[async_trait]
    impl EngineHost for ErrorHost {
        async fn device_token(&self) -> Result<Option<String>, String> {
            Err("device token unavailable".to_string())
        }

        async fn machine_id(&self) -> Result<String, String> {
            Err("machine id unavailable".to_string())
        }

        async fn cloud_base_url(&self) -> Result<String, String> {
            unreachable!()
        }

        async fn capture_db_path(&self) -> Result<PathBuf, String> {
            unreachable!()
        }

        async fn sync_state_path(&self) -> Result<PathBuf, String> {
            unreachable!()
        }

        async fn on_device_token_invalid(&self) -> Result<bool, String> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn run_once_returns_none_when_no_device_token() {
        let engine = DystilEngine::new(EngineConfig::default());
        let outcome = engine.run_once(&NoTokenHost).await.unwrap();
        assert!(outcome.is_none());
    }

    #[tokio::test]
    async fn run_once_surfaces_host_error() {
        let engine = DystilEngine::new(EngineConfig::default());
        let error = engine.run_once(&ErrorHost).await.unwrap_err();
        assert!(matches!(error, EngineError::Host(_)));
    }
}
