//! Dystil's long-lived capture runtime state.
//!
//! Dystil's long-lived SQLite runtime state. Local capture is started by
//! `CaptureSession`; cloud work is owned by `DystilEngine`.

use std::path::PathBuf;
use std::sync::Arc;

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
    local_llm: Option<crate::local_llm::LocalLlmManager>,
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

        // Start local LLM servers if llama-server is available and no external
        // DYSTIL_WORK_CARD_LLM_URL is already configured. The manager downloads
        // models from HuggingFace on first run and sets the env vars that the
        // work card worker reads below.
        let local_llm = crate::local_llm::LocalLlmManager::start(&data_dir).await;
        if local_llm.is_generator_ready() {
            tracing::info!("local LLM endpoint ready — work card generation is active");
        }

        let (work_card_shutdown, work_card_task) =
            if let Some(worker_config) = crate::work_card_worker::LocalWorkCardConfig::from_env() {
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
                                match crate::work_card_worker::generate_closed_work_cards(
                                    &worker_pool,
                                    &worker_config,
                                ).await {
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
            } else {
                (None, None)
            };

        Ok(Self {
            db,
            data_dir,
            data_path,
            work_card_shutdown,
            work_card_task,
            local_llm: Some(local_llm),
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
        if let Some(mut llm) = self.local_llm.take() {
            llm.shutdown().await;
        }
    }
}
