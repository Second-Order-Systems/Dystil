use std::io::{Cursor, Read};

use axum::extract::{Multipart, State};
use axum::http::HeaderMap;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use work_insights_db::semantic_trees::{
    insert_semantic_tree_sample, SemanticTreeInsert, SemanticTreeWriteError,
    SemanticTreeWriteOutcome,
};

use crate::{auth, AppError, AppState};

const MAX_COMPRESSED_BYTES: usize = 1024 * 1024;
const MAX_DECOMPRESSED_BYTES: usize = 10 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub(crate) const MAX_MULTIPART_BODY_BYTES: usize = MAX_COMPRESSED_BYTES + MAX_MANIFEST_BYTES;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SemanticTreeManifest {
    sample_id: String,
    source_frame_id: Option<i64>,
    surface_key: String,
    layout_fingerprint: String,
    schema_version: i16,
    codec: String,
    payload_sha256: String,
    captured_at: DateTime<Utc>,
    platform: String,
    app_name: String,
    app_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SemanticTreePayload {
    schema_version: i64,
    nodes: Vec<Value>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SemanticTreeUploadResponse {
    ok: bool,
    sample_id: String,
    payload_sha256: String,
    inserted: bool,
}

pub(crate) async fn post_semantic_tree_sample(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Json<SemanticTreeUploadResponse>, AppError> {
    let principal = auth::authenticate_device(&state, &headers).await?;
    let mut manifest = None;
    let mut payload = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::BadRequest(format!("invalid multipart body: {error}")))?
    {
        match field.name() {
            Some("manifest") => {
                let bytes = field.bytes().await.map_err(|error| {
                    AppError::BadRequest(format!("invalid manifest field: {error}"))
                })?;
                if bytes.len() > MAX_MANIFEST_BYTES {
                    return Err(AppError::BadRequest("manifest is too large".to_string()));
                }
                manifest = Some(
                    serde_json::from_slice::<SemanticTreeManifest>(&bytes).map_err(|error| {
                        AppError::BadRequest(format!("invalid manifest JSON: {error}"))
                    })?,
                );
            }
            Some("payload") => {
                let bytes = field.bytes().await.map_err(|error| {
                    AppError::BadRequest(format!("invalid payload field: {error}"))
                })?;
                if bytes.len() > MAX_COMPRESSED_BYTES {
                    return Err(AppError::BadRequest(
                        "compressed payload exceeds 1 MiB".to_string(),
                    ));
                }
                payload = Some(bytes);
            }
            _ => {}
        }
    }

    let manifest =
        manifest.ok_or_else(|| AppError::BadRequest("missing manifest field".to_string()))?;
    let payload =
        payload.ok_or_else(|| AppError::BadRequest("missing payload field".to_string()))?;
    validate_manifest(&manifest)?;
    validate_payload(&manifest, &payload)?;

    let outcome = insert_semantic_tree_sample(
        &state.pool,
        &principal,
        &SemanticTreeInsert {
            sample_id: &manifest.sample_id,
            source_frame_id: manifest.source_frame_id,
            surface_key: &manifest.surface_key,
            layout_fingerprint: &manifest.layout_fingerprint,
            schema_version: manifest.schema_version,
            codec: &manifest.codec,
            payload_sha256: &manifest.payload_sha256,
            payload: &payload,
            captured_at: manifest.captured_at,
            platform: &manifest.platform,
            app_name: &manifest.app_name,
            app_version: manifest.app_version.as_deref(),
        },
    )
    .await
    .map_err(|error| match error {
        SemanticTreeWriteError::ConflictingSample => AppError::BadRequest(error.to_string()),
        SemanticTreeWriteError::Sqlx(error) => AppError::Sqlx(error),
    })?;

    tracing::info!(
        org_id = %principal.org_id,
        user_id = %principal.user_id,
        device_id = %principal.device_id,
        sample_id = %manifest.sample_id,
        inserted = matches!(outcome, SemanticTreeWriteOutcome::Inserted),
        compressed_bytes = payload.len(),
        "semantic_tree_sample_processed"
    );

    Ok(Json(SemanticTreeUploadResponse {
        ok: true,
        sample_id: manifest.sample_id,
        payload_sha256: manifest.payload_sha256,
        inserted: matches!(outcome, SemanticTreeWriteOutcome::Inserted),
    }))
}

fn validate_manifest(manifest: &SemanticTreeManifest) -> Result<(), AppError> {
    if uuid::Uuid::parse_str(&manifest.sample_id).is_err() {
        return Err(AppError::BadRequest("sampleId must be a UUID".to_string()));
    }
    if manifest.codec != "zstd" {
        return Err(AppError::BadRequest("codec must be zstd".to_string()));
    }
    if manifest.schema_version != 1 {
        return Err(AppError::BadRequest(
            "unsupported semantic tree schemaVersion".to_string(),
        ));
    }
    for (name, value) in [
        ("surfaceKey", manifest.surface_key.as_str()),
        ("layoutFingerprint", manifest.layout_fingerprint.as_str()),
        ("payloadSha256", manifest.payload_sha256.as_str()),
    ] {
        if !valid_sha256_id(value) {
            return Err(AppError::BadRequest(format!(
                "{name} must be a sha256 identifier"
            )));
        }
    }
    if manifest.app_name.trim().is_empty() || manifest.app_name.len() > 512 {
        return Err(AppError::BadRequest("invalid appName".to_string()));
    }
    if !matches!(manifest.platform.as_str(), "windows" | "macos" | "linux") {
        return Err(AppError::BadRequest("invalid platform".to_string()));
    }
    if manifest
        .app_version
        .as_ref()
        .is_some_and(|value| value.len() > 128)
    {
        return Err(AppError::BadRequest("appVersion is too long".to_string()));
    }
    Ok(())
}

fn validate_payload(manifest: &SemanticTreeManifest, compressed: &[u8]) -> Result<(), AppError> {
    if compressed.is_empty() {
        return Err(AppError::BadRequest("payload is empty".to_string()));
    }
    let mut decoder = zstd::stream::read::Decoder::new(Cursor::new(compressed))
        .map_err(|error| AppError::BadRequest(format!("invalid zstd payload: {error}")))?;
    let mut decoded = Vec::new();
    decoder
        .by_ref()
        .take((MAX_DECOMPRESSED_BYTES + 1) as u64)
        .read_to_end(&mut decoded)
        .map_err(|error| AppError::BadRequest(format!("invalid zstd payload: {error}")))?;
    if decoded.len() > MAX_DECOMPRESSED_BYTES {
        return Err(AppError::BadRequest(
            "decompressed payload exceeds 10 MiB".to_string(),
        ));
    }
    if sha256_id(&decoded) != manifest.payload_sha256 {
        return Err(AppError::BadRequest("payload SHA-256 mismatch".to_string()));
    }
    let tree: SemanticTreePayload = serde_json::from_slice(&decoded)
        .map_err(|error| AppError::BadRequest(format!("invalid semantic tree JSON: {error}")))?;
    if tree.schema_version != manifest.schema_version as i64 {
        return Err(AppError::BadRequest(
            "payload schema version does not match manifest".to_string(),
        ));
    }
    if tree.nodes.is_empty() || tree.nodes.len() > 5_000 {
        return Err(AppError::BadRequest(
            "semantic tree must contain between 1 and 5000 nodes".to_string(),
        ));
    }
    if manifest.platform == "macos" && tree.nodes.iter().any(macos_node_has_free_form_content) {
        return Err(AppError::BadRequest(
            "macOS structural samples must omit free-form content".to_string(),
        ));
    }
    Ok(())
}

fn macos_node_has_free_form_content(node: &Value) -> bool {
    ["text", "value", "help_text"]
        .iter()
        .any(|key| match node.get(*key) {
            None | Some(Value::Null) => false,
            Some(Value::String(value)) => !value.is_empty(),
            Some(_) => true,
        })
}

fn valid_sha256_id(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value[7..].bytes().all(|value| value.is_ascii_hexdigit())
}

fn sha256_id(value: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn manifest(platform: &str, decoded: &[u8]) -> SemanticTreeManifest {
        SemanticTreeManifest {
            sample_id: uuid::Uuid::new_v4().to_string(),
            source_frame_id: Some(1),
            surface_key: sha256_id(b"surface"),
            layout_fingerprint: sha256_id(b"layout"),
            schema_version: 1,
            codec: "zstd".to_string(),
            payload_sha256: sha256_id(decoded),
            captured_at: Utc::now(),
            platform: platform.to_string(),
            app_name: "Test".to_string(),
            app_version: Some("1".to_string()),
        }
    }

    #[test]
    fn accepts_valid_zstd_tree() {
        let decoded = serde_json::to_vec(&json!({
            "schema_version": 1,
            "nodes": [{"node_id": 1, "role": "Window", "text": "visible"}]
        }))
        .unwrap();
        let compressed = zstd::stream::encode_all(Cursor::new(&decoded), 1).unwrap();
        validate_payload(&manifest("windows", &decoded), &compressed).unwrap();
    }

    #[test]
    fn rejects_macos_free_form_content() {
        let decoded = serde_json::to_vec(&json!({
            "schema_version": 1,
            "nodes": [{"node_id": 1, "role": "AXStaticText", "text": "private"}]
        }))
        .unwrap();
        let compressed = zstd::stream::encode_all(Cursor::new(&decoded), 1).unwrap();
        let error = validate_payload(&manifest("macos", &decoded), &compressed).unwrap_err();
        assert!(error.to_string().contains("omit free-form"));
    }

    #[test]
    fn rejects_hash_mismatch() {
        let decoded = serde_json::to_vec(&json!({
            "schema_version": 1,
            "nodes": [{"node_id": 1, "role": "Window", "text": "visible"}]
        }))
        .unwrap();
        let compressed = zstd::stream::encode_all(Cursor::new(&decoded), 1).unwrap();
        let mut wrong = manifest("windows", &decoded);
        wrong.payload_sha256 = sha256_id(b"different");
        let error = validate_payload(&wrong, &compressed).unwrap_err();
        assert!(error.to_string().contains("mismatch"));
    }
}
