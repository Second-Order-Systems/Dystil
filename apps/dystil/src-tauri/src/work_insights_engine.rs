use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use dystil_engine::{DystilEngine, EngineConfig, EngineHost};
use dystil_sync::LocalSyncPermissions;
use tauri::AppHandle;
use tokio::sync::Mutex;

static ENGINE_TASK: std::sync::OnceLock<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> =
    std::sync::OnceLock::new();

fn engine_task() -> &'static Mutex<Option<tauri::async_runtime::JoinHandle<()>>> {
    ENGINE_TASK.get_or_init(|| Mutex::new(None))
}

struct TauriEngineHost {
    app: AppHandle,
}

#[async_trait]
impl EngineHost for TauriEngineHost {
    async fn device_token(&self) -> Result<Option<String>, String> {
        crate::auth::current_device_token().await
    }

    async fn machine_id(&self) -> Result<String, String> {
        let data_dir = crate::log_files::get_data_dir(&self.app).map_err(|err| err.to_string())?;
        dystil_storage::get_or_create_machine_id(data_dir).map_err(|err| err.to_string())
    }

    async fn cloud_base_url(&self) -> Result<String, String> {
        crate::auth::cloud_base_url()
    }

    async fn capture_db_path(&self) -> Result<PathBuf, String> {
        crate::log_files::get_data_dir(&self.app)
            .map(|dir| dir.join("db.sqlite"))
            .map_err(|err| err.to_string())
    }

    async fn sync_state_path(&self) -> Result<PathBuf, String> {
        crate::log_files::get_data_dir(&self.app)
            .map(|dir| dir.join("dystil-segment-sync.sqlite"))
            .map_err(|err| err.to_string())
    }

    async fn semantic_tree_store_path(&self) -> Result<Option<PathBuf>, String> {
        crate::log_files::get_data_dir(&self.app)
            .map(|dir| Some(dir.join("data").join("semantic-tree-samples.sqlite")))
            .map_err(|err| err.to_string())
    }

    async fn on_device_token_invalid(&self) -> Result<bool, String> {
        crate::auth::clear_and_re_register_device_token().await
    }

    async fn local_sync_permissions(&self) -> Result<LocalSyncPermissions, String> {
        let settings = crate::store::SettingsStore::get(&self.app)?.unwrap_or_default();
        let consent = settings.sync_consent.effective();
        Ok(LocalSyncPermissions {
            segments: consent.segments,
            screenshots: consent.screenshots,
        })
    }

    async fn app_version(&self) -> Result<Option<String>, String> {
        Ok(Some(env!("CARGO_PKG_VERSION").to_string()))
    }

    async fn build_channel(&self) -> Result<Option<String>, String> {
        Ok(Some(
            if cfg!(debug_assertions) {
                "local"
            } else {
                "prod"
            }
            .to_string(),
        ))
    }
}

fn spawn(app: AppHandle) -> tauri::async_runtime::JoinHandle<()> {
    let host = Arc::new(TauriEngineHost { app });
    let engine = DystilEngine::new(EngineConfig::default());
    tauri::async_runtime::spawn(async move {
        engine.run_forever(host).await;
    })
}

pub async fn reconcile(app: AppHandle) -> Result<(), String> {
    let settings = crate::store::SettingsStore::get(&app)?.unwrap_or_default();
    let permitted = settings.sync_consent.effective().segments
        && crate::auth::current_device_token().await?.is_some();
    let mut task = engine_task().lock().await;
    if permitted {
        if task
            .as_ref()
            .is_none_or(|handle| handle.inner().is_finished())
        {
            *task = Some(spawn(app));
        }
    } else if let Some(handle) = task.take() {
        handle.abort();
    }
    Ok(())
}
