//! Dystil's long-lived capture runtime state.
//!
//! Dystil's long-lived SQLite runtime state. Local capture is started by
//! `CaptureSession`; cloud work is owned by `DystilEngine`.

use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use crate::capture_config::DystilCaptureConfig;
use sqlx::SqlitePool;

/// Database handle shared by the native capture session.
#[derive(Clone)]
pub struct CaptureDatabase {
    pub pool: SqlitePool,
}

/// Long-lived Dystil runtime state. This is deliberately not an HTTP server.
pub struct ServerCore {
    pub db: Arc<CaptureDatabase>,
    pub data_dir: PathBuf,
    pub data_path: PathBuf,
    work_card_shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    work_card_task: Option<tokio::task::JoinHandle<()>>,
    local_llm: Arc<tokio::sync::Mutex<Option<crate::local_llm::LocalLlmManager>>>,
    local_llm_start_task: Option<tokio::task::JoinHandle<()>>,
    local_processing_requested: Arc<AtomicBool>,
}

impl ServerCore {
    pub async fn start(config: &DystilCaptureConfig) -> Result<Self, String> {
        let data_dir = config.data_dir.clone();
        let data_path = data_dir.join("data");
        std::fs::create_dir_all(&data_path)
            .map_err(|error| format!("failed to create capture data directory: {error}"))?;

        let db_path = data_dir.join("db.sqlite");
        let pool = dystil_storage::open_capture_database(&db_path)
            .await
            .map_err(|error| format!("failed to open Dystil capture database: {error}"))?;

        let db = Arc::new(CaptureDatabase { pool });

        // Embeddings are always available for local retrieval, but never
        // delay capture startup. The model manager is retained once its
        // background preparation completes so it still owns process shutdown.
        let local_processing_requested = Arc::new(AtomicBool::new(
            std::env::var("DYSTIL_LOCAL_PROCESSING_ENABLED")
                .ok()
                .as_deref()
                == Some("1"),
        ));
        let local_llm = Arc::new(tokio::sync::Mutex::new(None));
        let local_llm_for_task = local_llm.clone();
        let local_processing_for_task = local_processing_requested.clone();
        let local_data_dir = data_dir.clone();
        let local_llm_start_task = tokio::spawn(async move {
            let mut manager =
                crate::local_llm::LocalLlmManager::start(&local_data_dir, false).await;
            if local_processing_for_task.load(Ordering::SeqCst) {
                if let Err(error) = manager.enable_generator(&local_data_dir).await {
                    tracing::warn!(%error, "local generator unavailable after background preparation");
                } else {
                    tracing::info!("local LLM endpoint ready — work card generation is active");
                }
            }
            *local_llm_for_task.lock().await = Some(manager);
        });

        // Resolve its configuration on every pass so a BYOK profile added
        // after capture starts immediately powers work-card generation.
        let (work_card_shutdown, work_card_task) = {
            let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
            let worker_pool = db.pool.clone();
            let task = tokio::spawn(async move {
                // Give capture/redaction a chance to settle before the first pass.
                let period = std::time::Duration::from_secs(120);
                let mut interval =
                    tokio::time::interval_at(tokio::time::Instant::now() + period, period);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        _ = interval.tick() => {
                            if !crate::work_card_worker::background_generation_allowed() {
                                continue;
                            }
                            let config = match crate::work_card_worker::configured_work_card_config(&worker_pool).await {
                                Ok(Some(config)) => config,
                                Ok(None) => continue,
                                Err(error) => {
                                    tracing::debug!(%error, "work-card model configuration unavailable");
                                    continue;
                                }
                            };
                            match crate::work_card_worker::generate_closed_work_cards(&worker_pool, &config).await {
                                Ok(report) if report.generated_cards > 0 => {
                                    tracing::info!(
                                        generated = report.generated_cards,
                                        elapsed_ms = report.elapsed_ms,
                                        "local work-card pass completed"
                                    );
                                }
                                Ok(_) => {}
                                Err(error) => {
                                    tracing::debug!(%error, "local work-card pass deferred");
                                }
                            }
                        }
                    }
                }
            });
            (Some(shutdown_tx), Some(task))
        };

        Ok(Self {
            db,
            data_dir,
            data_path,
            work_card_shutdown,
            work_card_task,
            local_llm,
            local_llm_start_task: Some(local_llm_start_task),
            local_processing_requested,
        })
    }

    /// Runtime shutdown is owned by the capture session. Kept async so callers
    /// can retain their existing lifecycle without carrying Dystil workers.
    pub async fn shutdown(mut self) {
        if let Some(shutdown) = self.work_card_shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.work_card_task.take() {
            let _ = task.await;
        }
        if let Some(task) = self.local_llm_start_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(mut llm) = self.local_llm.lock().await.take() {
            llm.shutdown().await;
        }
    }

    pub async fn enable_local_processing(&mut self) -> Result<(), String> {
        self.local_processing_requested
            .store(true, Ordering::SeqCst);
        let mut local_llm = self.local_llm.lock().await;
        if let Some(manager) = local_llm.as_mut() {
            if !manager.is_generator_ready() {
                manager.enable_generator(&self.data_dir).await?;
            }
        } else {
            tracing::info!("local generator queued until embedding preparation completes");
        }
        Ok(())
    }

    pub async fn set_local_processing_enabled(&mut self, enabled: bool) -> Result<(), String> {
        if enabled {
            return self.enable_local_processing().await;
        }
        self.local_processing_requested
            .store(false, Ordering::SeqCst);
        if let Some(manager) = self.local_llm.lock().await.as_mut() {
            manager.disable_generator().await;
        }
        Ok(())
    }

    pub fn local_processing_requested(&self) -> bool {
        self.local_processing_requested.load(Ordering::SeqCst)
    }
}
