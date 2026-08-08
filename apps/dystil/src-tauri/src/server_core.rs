//! Dystil's long-lived capture runtime state.
//!
//! Dystil's long-lived SQLite runtime state. Local capture is started by
//! `CaptureSession`; cloud work is owned by `DystilEngine`.

use std::path::PathBuf;
use std::sync::Arc;

use crate::capture_config::DystilCaptureConfig;
use dystil_telemetry::{AppStartReason, Outcome, Telemetry};
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
    /// Local-only, always-on aggregate recorder.
    pub telemetry: Arc<Telemetry>,
    runtime_marker: PathBuf,
    resource_sampler: tokio::task::JoinHandle<()>,
    telemetry_exporter: Option<tokio::task::JoinHandle<()>>,
}

impl ServerCore {
    pub async fn start(config: &DystilCaptureConfig) -> Result<Self, String> {
        let data_dir = config.data_dir.clone();
        let data_path = data_dir.join("data");
        std::fs::create_dir_all(&data_path)
            .map_err(|error| format!("failed to create capture data directory: {error}"))?;

        // This intentionally contains no timestamp or identifier. Its sole
        // meaning is whether the previous local runtime reached `shutdown`.
        let runtime_marker = data_dir.join(".runtime-active");
        let previous_runtime_unclean = runtime_marker.exists();
        if let Err(error) = std::fs::write(&runtime_marker, b"1") {
            tracing::warn!(%error, "could not persist local runtime shutdown marker");
        }

        let db_path = data_dir.join("db.sqlite");
        let pool = dystil_storage::open_capture_database(&db_path)
            .await
            .map_err(|error| format!("failed to open Dystil capture database: {error}"))?;

        let db = Arc::new(CaptureDatabase { pool });
        let telemetry = Arc::new(Telemetry::new());
        let resource_sampler =
            crate::telemetry_resources::start(telemetry.clone(), data_dir.clone());
        let telemetry_instance_id = dystil_storage::get_or_create_machine_id(data_dir.clone())
            .map_err(|error| format!("failed to load telemetry installation id: {error}"))?;
        let telemetry_exporter =
            crate::telemetry_exporter::start(telemetry.clone(), telemetry_instance_id);
        telemetry.record_app_start(AppStartReason::Launch, Outcome::Succeeded);
        if previous_runtime_unclean {
            telemetry.record_app_start(AppStartReason::PreviousUncleanShutdown, Outcome::Succeeded);
        }

        Ok(Self {
            db,
            data_dir,
            data_path,
            telemetry,
            runtime_marker,
            resource_sampler,
            telemetry_exporter,
        })
    }

    /// Runtime shutdown is owned by the capture session. Kept async so callers
    /// can retain their existing lifecycle without carrying Dystil workers.
    pub async fn shutdown(self) {
        self.resource_sampler.abort();
        if let Some(exporter) = self.telemetry_exporter {
            exporter.abort();
        }
        if let Err(error) = std::fs::remove_file(&self.runtime_marker) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(%error, "could not clear local runtime shutdown marker");
            }
        }
    }
}
