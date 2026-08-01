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

        Ok(Self {
            db,
            data_dir,
            data_path,
        })
    }

    /// Runtime shutdown is owned by the capture session. Kept async so callers
    /// can retain their existing lifecycle without carrying Dystil workers.
    pub async fn shutdown(self) {}
}
