use reqwest::header::AUTHORIZATION;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::app_config::cloud_base_url as configured_cloud_base_url;

const SECRET_KEY: &str = "auth:dystil:state";

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct DystilUserSession {
    pub session_token: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct DystilUserOrg {
    pub id: String,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub roles: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct DystilUserProfile {
    pub id: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub image: Option<String>,
    pub org: Option<DystilUserOrg>,
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Default)]
pub struct DystilAuthState {
    pub status: String,
    pub session: Option<DystilUserSession>,
    pub user: Option<DystilUserProfile>,
    pub device_token_present: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PendingOnboardingSync {
    payload: Value,
    user_id: Option<String>,
    session_token_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthRecord {
    session: Option<DystilUserSession>,
    user: Option<DystilUserProfile>,
    device_token: Option<String>,
    pending_onboarding_sync: Option<PendingOnboardingSync>,
}

#[derive(Debug, Deserialize)]
struct DeviceRegistrationResponse {
    device_token: String,
}

async fn open_secret_store() -> Result<crate::secret_store::DystilSecretStore, String> {
    crate::secret_store::open_secret_store().await
}

async fn read_record() -> Result<AuthRecord, String> {
    let store = open_secret_store().await?;
    let bytes = store.get(SECRET_KEY).await.map_err(|e| e.to_string())?;
    match bytes {
        Some(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
        None => Ok(AuthRecord::default()),
    }
}

async fn write_record(record: &AuthRecord) -> Result<(), String> {
    let store = open_secret_store().await?;
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    store
        .set(SECRET_KEY, &bytes)
        .await
        .map_err(|e| e.to_string())
}

async fn clear_record() -> Result<(), String> {
    let store = open_secret_store().await?;
    store.delete(SECRET_KEY).await.map_err(|e| e.to_string())
}

fn auth_base_url() -> Result<String, String> {
    cloud_base_url()
}

pub(crate) fn cloud_base_url() -> Result<String, String> {
    configured_cloud_base_url()
        .map(str::to_owned)
        .ok_or_else(|| "cloud_unavailable: this build does not include Dystil Cloud".to_string())
}

fn hash_session_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn current_platform() -> String {
    std::env::consts::OS.to_string()
}

fn current_device_label() -> String {
    hostname::get()
        .ok()
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "dystil".to_string())
}

fn parse_user_org(value: &Value) -> Option<DystilUserOrg> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str()?.trim();
    if id.is_empty() {
        return None;
    }

    let roles = object
        .get("roles")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::trim))
                .filter(|item| !item.is_empty())
                .map(|item| item.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Some(DystilUserOrg {
        id: id.to_string(),
        name: object
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        slug: object
            .get("slug")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        roles,
    })
}

async fn store_session_token(token: String) -> Result<DystilAuthState, String> {
    tracing::info!(
        token_length = token.len(),
        "[auth-flow][native] storing session token"
    );
    let mut record = read_record().await?;
    record.session = Some(DystilUserSession {
        session_token: Some(token),
        expires_at: None,
    });
    write_record(&record).await?;
    tracing::info!("[auth-flow][native] session token persisted");
    // Bootstrap is best-effort during initial sign-in;
    // the frontend calls auth_fetch_profile separately after login.
    match bootstrap_from_cloud().await {
        Ok(state) => {
            tracing::info!(
                status = %state.status,
                has_user = state.user.is_some(),
                has_device_token = state.device_token_present,
                "[auth-flow][native] bootstrap_from_cloud succeeded after store"
            );
            Ok(state)
        }
        Err(e) => {
            tracing::warn!("cloud bootstrap deferred (API may not be running): {e}");
            Ok(auth_state_from_record(&record))
        }
    }
}

async fn bootstrap_from_cloud() -> Result<DystilAuthState, String> {
    tracing::info!("[auth-flow][native] bootstrap_from_cloud started");
    let mut record = read_record().await?;
    let session_token = record
        .session
        .as_ref()
        .and_then(|session| session.session_token.clone())
        .ok_or_else(|| "no stored Better Auth session".to_string())?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    let cloud_base = cloud_base_url()?;
    let me = client
        .get(format!("{cloud_base}/me"))
        .header(AUTHORIZATION, format!("Bearer {}", session_token))
        .send()
        .await
        .map_err(|e| format!("cloud /me request failed: {e}"))?;

    tracing::info!(
        status = %me.status(),
        "[auth-flow][native] cloud /me responded"
    );

    if me.status().as_u16() == 401 {
        record.session = None;
        record.user = None;
        record.device_token = None;
        write_record(&record).await?;
        tracing::warn!("[auth-flow][native] cloud /me returned 401; session cleared");
        return Ok(DystilAuthState {
            status: "signed_out".to_string(),
            session: None,
            user: None,
            device_token_present: false,
            error: None,
        });
    }

    if !me.status().is_success() {
        let status = me.status();
        let body = me.text().await.unwrap_or_default();
        return Err(format!("cloud /me returned {}: {}", status, body));
    }

    let identity: serde_json::Value = me
        .json()
        .await
        .map_err(|e| format!("cloud /me payload invalid: {e}"))?;

    record.user = Some(DystilUserProfile {
        id: identity
            .get("user_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        email: identity
            .get("email")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        name: identity
            .get("display_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        image: None,
        org: identity.get("org").and_then(parse_user_org),
    });

    if record.device_token.is_none() {
        tracing::info!("[auth-flow][native] device token missing; registering device");
        let register = client
            .post(format!("{cloud_base}/devices/register"))
            .header(AUTHORIZATION, format!("Bearer {}", session_token))
            .json(&serde_json::json!({
                "device_label": current_device_label(),
                "platform": current_platform(),
            }))
            .send()
            .await
            .map_err(|e| format!("cloud /devices/register request failed: {e}"))?;

        tracing::info!(
            status = %register.status(),
            "[auth-flow][native] devices/register responded"
        );

        if !register.status().is_success() {
            let status = register.status();
            let body = register.text().await.unwrap_or_default();
            return Err(format!(
                "cloud /devices/register returned {}: {}",
                status, body
            ));
        }

        let register_response: DeviceRegistrationResponse = register
            .json()
            .await
            .map_err(|e| format!("cloud /devices/register payload invalid: {e}"))?;
        record.device_token = Some(register_response.device_token);
    }

    if let Err(error) =
        try_sync_pending_onboarding_data(&mut record, &client, &cloud_base, &session_token).await
    {
        tracing::warn!("pending onboarding sync failed: {error}");
    }

    write_record(&record).await?;
    tracing::info!(
        has_user = record.user.is_some(),
        has_device_token = record.device_token.is_some(),
        "[auth-flow][native] bootstrap_from_cloud completed"
    );

    Ok(auth_state_from_record(&record))
}

async fn put_onboarding_data(
    client: &reqwest::Client,
    cloud_base: &str,
    session_token: &str,
    payload: &Value,
) -> Result<(), String> {
    let response = client
        .put(format!("{cloud_base}/me/onboarding"))
        .header(AUTHORIZATION, format!("Bearer {}", session_token))
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("cloud /me/onboarding request failed: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "cloud /me/onboarding returned {}: {}",
            status, body
        ));
    }

    Ok(())
}

async fn try_sync_pending_onboarding_data(
    record: &mut AuthRecord,
    client: &reqwest::Client,
    cloud_base: &str,
    session_token: &str,
) -> Result<(), String> {
    let Some(pending) = record.pending_onboarding_sync.clone() else {
        return Ok(());
    };

    let current_user_id = record
        .user
        .as_ref()
        .map(|user| user.id.as_str())
        .filter(|user_id| !user_id.trim().is_empty());
    let current_session_hash = hash_session_token(session_token);

    if should_drop_pending_onboarding_sync(&pending, current_user_id, current_session_hash.as_str())
    {
        tracing::warn!("dropping pending onboarding sync due to session/user mismatch");
        record.pending_onboarding_sync = None;
        return Ok(());
    }

    put_onboarding_data(client, cloud_base, session_token, &pending.payload).await?;
    record.pending_onboarding_sync = None;
    Ok(())
}

fn should_drop_pending_onboarding_sync(
    pending: &PendingOnboardingSync,
    current_user_id: Option<&str>,
    current_session_hash: &str,
) -> bool {
    let expected_user_id = pending
        .user_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let expected_session_hash = pending
        .session_token_sha256
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match (
        expected_user_id,
        current_user_id.map(str::trim),
        expected_session_hash,
    ) {
        (Some(expected), Some(current), _) if expected != current => true,
        (Some(_), None, Some(expected_hash)) if expected_hash != current_session_hash => true,
        (None, _, Some(expected_hash)) if expected_hash != current_session_hash => true,
        _ => false,
    }
}

pub(crate) async fn enqueue_onboarding_data_sync(payload: Value) -> Result<(), String> {
    if configured_cloud_base_url().is_none() {
        return Ok(());
    }
    let mut record = read_record().await?;
    let bound_user_id = record
        .user
        .as_ref()
        .map(|user| user.id.clone())
        .filter(|id| !id.trim().is_empty());
    let bound_session_hash = record
        .session
        .as_ref()
        .and_then(|session| session.session_token.as_deref())
        .filter(|token| !token.trim().is_empty())
        .map(hash_session_token);

    if bound_user_id.is_none() && bound_session_hash.is_none() {
        return Err("no authenticated session available to bind onboarding sync".to_string());
    }

    let pending = PendingOnboardingSync {
        payload,
        user_id: bound_user_id,
        session_token_sha256: bound_session_hash,
    };
    record.pending_onboarding_sync = Some(pending);
    write_record(&record).await?;
    Ok(())
}

pub(crate) async fn flush_pending_onboarding_data() -> Result<(), String> {
    if configured_cloud_base_url().is_none() {
        return Ok(());
    }
    let mut record = read_record().await?;
    let Some(session_token) = record
        .session
        .as_ref()
        .and_then(|session| session.session_token.clone())
    else {
        return Ok(());
    };

    let cloud_base = cloud_base_url()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;

    try_sync_pending_onboarding_data(&mut record, &client, &cloud_base, &session_token).await?;
    write_record(&record).await?;
    Ok(())
}

pub(crate) async fn current_device_token() -> Result<Option<String>, String> {
    Ok(read_record().await?.device_token)
}

fn auth_state_from_record(record: &AuthRecord) -> DystilAuthState {
    let has_session = record.session.is_some();
    let has_user = record.user.is_some();
    let status = if !has_session {
        "signed_out"
    } else if has_session && !has_user {
        "session_ready"
    } else if has_session && has_user && record.device_token.is_none() {
        "device_registering"
    } else {
        "ready"
    };

    DystilAuthState {
        status: status.to_string(),
        session: record.session.clone(),
        user: record.user.clone(),
        device_token_present: record.device_token.is_some(),
        error: None,
    }
}

pub(crate) async fn clear_and_re_register_device_token() -> Result<bool, String> {
    let mut record = read_record().await?;
    record.device_token = None;
    write_record(&record).await?;
    match bootstrap_from_cloud().await {
        Ok(state) => Ok(state.device_token_present),
        Err(e) => {
            tracing::warn!("device token re-registration failed: {e}");
            Ok(false)
        }
    }
}

#[tauri::command]
#[specta::specta]
pub async fn auth_get_state() -> Result<DystilAuthState, String> {
    let record = read_record().await?;
    Ok(auth_state_from_record(&record))
}

#[tauri::command]
#[specta::specta]
pub async fn auth_get_session() -> Result<Option<DystilUserSession>, String> {
    Ok(read_record().await?.session)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_get_device_token() -> Result<Option<String>, String> {
    Ok(read_record().await?.device_token)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_store_session(
    app_handle: tauri::AppHandle,
    token: String,
) -> Result<DystilAuthState, String> {
    tracing::info!("[auth-flow][native] auth_store_session command invoked");
    let state = store_session_token(token).await?;
    #[cfg(feature = "cloud-sync")]
    {
        if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
            tracing::warn!(%error, "failed to reconcile cloud sync after storing auth session");
        }
        crate::capture_state_reporter::schedule();
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_clear_session(app_handle: tauri::AppHandle) -> Result<DystilAuthState, String> {
    let mut record = read_record().await?;
    record.session = None;
    record.user = None;
    record.pending_onboarding_sync = None;
    write_record(&record).await?;
    let state = auth_state_from_record(&record);
    #[cfg(feature = "cloud-sync")]
    if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
        tracing::warn!(%error, "failed to reconcile cloud sync after clearing auth session");
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_clear_device_token(
    app_handle: tauri::AppHandle,
) -> Result<DystilAuthState, String> {
    let mut record = read_record().await?;
    record.device_token = None;
    write_record(&record).await?;
    let state = auth_state_from_record(&record);
    #[cfg(feature = "cloud-sync")]
    if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
        tracing::warn!(%error, "failed to reconcile cloud sync after clearing device token");
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_fetch_profile(app_handle: tauri::AppHandle) -> Result<DystilAuthState, String> {
    tracing::info!("[auth-flow][native] auth_fetch_profile command invoked");
    let state = bootstrap_from_cloud().await?;
    #[cfg(feature = "cloud-sync")]
    {
        if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
            tracing::warn!(%error, "failed to reconcile cloud sync after fetching auth profile");
        }
        crate::capture_state_reporter::schedule();
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_register_device(app_handle: tauri::AppHandle) -> Result<DystilAuthState, String> {
    let state = bootstrap_from_cloud().await?;
    #[cfg(feature = "cloud-sync")]
    {
        if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
            tracing::warn!(%error, "failed to reconcile cloud sync after device registration");
        }
        crate::capture_state_reporter::schedule();
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(state)
}

#[tauri::command]
#[specta::specta]
pub async fn auth_sign_out(app_handle: tauri::AppHandle) -> Result<DystilAuthState, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build http client: {e}"))?;
    if let Ok(record) = read_record().await {
        if let Some(session) = record.session.and_then(|s| s.session_token) {
            let _ = client
                .post(format!("{}/api/auth/sign-out", auth_base_url()?))
                .header(AUTHORIZATION, format!("Bearer {}", session))
                .send()
                .await;
        }
    }
    clear_record().await?;
    #[cfg(feature = "cloud-sync")]
    if let Err(error) = crate::work_insights_engine::reconcile(app_handle).await {
        tracing::warn!(%error, "failed to reconcile cloud sync after sign-out");
    }
    #[cfg(not(feature = "cloud-sync"))]
    let _ = app_handle;
    Ok(DystilAuthState {
        status: "signed_out".to_string(),
        session: None,
        user: None,
        device_token_present: false,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        auth_state_from_record, hash_session_token, should_drop_pending_onboarding_sync,
        AuthRecord, DystilUserProfile, DystilUserSession, PendingOnboardingSync,
    };
    use serde_json::json;

    #[test]
    fn computes_auth_state_from_record() {
        let record = AuthRecord {
            session: Some(DystilUserSession {
                session_token: Some("token".to_string()),
                expires_at: None,
            }),
            user: Some(DystilUserProfile {
                id: "user_1".to_string(),
                email: Some("user@example.com".to_string()),
                name: Some("Ada".to_string()),
                image: None,
                org: None,
            }),
            device_token: Some("device".to_string()),
            pending_onboarding_sync: None,
        };
        let state = auth_state_from_record(&record);
        assert_eq!(state.status, "ready");
        assert!(state.device_token_present);
    }

    #[test]
    fn pending_sync_without_user_binding_requires_same_session_hash() {
        let current_hash = hash_session_token("session-a");
        let pending = PendingOnboardingSync {
            payload: json!({ "v": 1 }),
            user_id: None,
            session_token_sha256: Some(current_hash.clone()),
        };

        assert!(!should_drop_pending_onboarding_sync(
            &pending,
            None,
            current_hash.as_str()
        ));
        assert!(should_drop_pending_onboarding_sync(
            &pending,
            None,
            hash_session_token("session-b").as_str()
        ));
    }

    #[test]
    fn pending_sync_with_user_binding_allows_session_rotation_for_same_user() {
        let pending = PendingOnboardingSync {
            payload: json!({ "v": 1 }),
            user_id: Some("user_123".to_string()),
            session_token_sha256: Some(hash_session_token("session-a")),
        };

        assert!(!should_drop_pending_onboarding_sync(
            &pending,
            Some("user_123"),
            hash_session_token("session-b").as_str()
        ));
        assert!(should_drop_pending_onboarding_sync(
            &pending,
            Some("user_999"),
            hash_session_token("session-a").as_str()
        ));
    }
}
