//! OAuth stub — third-party OAuth integrations are excluded from the Dystil product.
//!
//! Type definitions are retained for specta bindings compatibility.
//! All commands return empty results or errors.

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, specta::Type, Clone)]
pub struct OAuthStatus {
    pub connected: bool,
    pub display_name: Option<String>,
    #[serde(default)]
    pub needs_attention: bool,
}

#[derive(Serialize, Deserialize, specta::Type, Clone)]
pub struct OAuthInstanceInfo {
    pub instance: Option<String>,
    pub display_name: Option<String>,
}

#[tauri::command]
#[specta::specta]
pub async fn oauth_connect(
    _app_handle: tauri::AppHandle,
    integration_id: String,
    _instance: Option<String>,
) -> Result<OAuthStatus, String> {
    Err(format!(
        "OAuth integrations are not available in this build: {}",
        integration_id
    ))
}

#[tauri::command]
#[specta::specta]
pub fn oauth_cancel(_integration_id: String) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn oauth_status(
    _integration_id: String,
    _instance: Option<String>,
) -> Result<OAuthStatus, String> {
    Ok(OAuthStatus {
        connected: false,
        display_name: None,
        needs_attention: false,
    })
}

#[tauri::command]
#[specta::specta]
pub async fn oauth_disconnect(
    _integration_id: String,
    _instance: Option<String>,
) -> Result<(), String> {
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn oauth_list_instances(
    _integration_id: String,
) -> Result<Vec<OAuthInstanceInfo>, String> {
    Ok(Vec::new())
}
