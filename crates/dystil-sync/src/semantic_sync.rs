use std::path::PathBuf;

use dystil_capture::semantic_tree::{PendingSemanticSample, SemanticTreeStore};
use serde::{Deserialize, Serialize};

use crate::SyncError;

const MAX_SAMPLES_PER_PASS: usize = 20;

#[derive(Debug, Clone)]
pub struct SemanticSyncConfig {
    pub store_path: PathBuf,
    pub cloud_base_url: String,
    pub device_token: String,
    pub request_timeout_secs: u64,
    pub app_version: Option<String>,
    pub build_channel: Option<String>,
    pub build_commit: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SemanticTreeManifest<'a> {
    sample_id: &'a str,
    source_frame_id: i64,
    surface_key: &'a str,
    layout_fingerprint: &'a str,
    schema_version: i64,
    codec: &'a str,
    payload_sha256: &'a str,
    captured_at: &'a str,
    platform: &'a str,
    app_name: &'a str,
    app_version: Option<&'a str>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SemanticTreeUploadResponse {
    ok: bool,
    sample_id: String,
    payload_sha256: String,
    #[serde(default, rename = "inserted")]
    _inserted: bool,
}

/// Upload one bounded pass from the independent semantic-tree outbox.
///
/// The caller deliberately decides whether an error should affect its own
/// workflow. The desktop engine treats every error here as best-effort so
/// semantic sampling cannot block segment or screenshot sync.
pub async fn upload_pending_semantic_samples(
    config: SemanticSyncConfig,
) -> Result<usize, SyncError> {
    let store_path = config.store_path.clone();
    let store = tokio::task::spawn_blocking(move || SemanticTreeStore::open(store_path))
        .await
        .map_err(|error| SyncError::Message(format!("semantic store task failed: {error}")))?
        .map_err(|error| SyncError::Message(format!("semantic store unavailable: {error}")))?;
    let pending_store = store.clone();
    let pending = tokio::task::spawn_blocking(move || pending_store.pending(MAX_SAMPLES_PER_PASS))
        .await
        .map_err(|error| SyncError::Message(format!("semantic outbox task failed: {error}")))?
        .map_err(|error| SyncError::Message(format!("semantic outbox unavailable: {error}")))?;
    if pending.is_empty() {
        return Ok(0);
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(
            config.request_timeout_secs.max(1),
        ))
        .build()?;
    let endpoint = format!(
        "{}/v1/semantic-tree-samples",
        config.cloud_base_url.trim_end_matches('/')
    );
    let mut uploaded = 0;

    for sample in pending {
        let manifest = serde_json::to_vec(&manifest(&sample))?;
        let form = reqwest::multipart::Form::new()
            .part(
                "manifest",
                reqwest::multipart::Part::bytes(manifest)
                    .mime_str("application/json")?
                    .file_name("manifest.json"),
            )
            .part(
                "payload",
                reqwest::multipart::Part::bytes(sample.payload.clone())
                    .mime_str("application/zstd")?
                    .file_name("semantic-tree.json.zst"),
            );
        let mut request = client
            .post(&endpoint)
            .header("Authorization", format!("Device {}", config.device_token))
            .multipart(form);
        if let Some(version) = &config.app_version {
            request = request.header("X-Dystil-App-Version", version);
        }
        if let Some(channel) = &config.build_channel {
            request = request.header("X-Dystil-Build-Channel", channel);
        }
        if let Some(commit) = &config.build_commit {
            request = request.header("X-Dystil-Build-Commit", commit);
        }
        request = request.header("X-Dystil-Sync-Capabilities", "semantic-tree-samples-v1");

        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(SyncError::Unauthorized);
        }
        if !status.is_success() {
            let detail = response.text().await.unwrap_or_default();
            return Err(SyncError::Message(format!(
                "semantic sample upload failed with status {status}: {}",
                detail.chars().take(512).collect::<String>()
            )));
        }
        let ack: SemanticTreeUploadResponse = response.json().await?;
        validate_ack(&sample, &ack)?;

        let ack_store = store.clone();
        let sample_id = sample.sample_id.clone();
        let payload_sha256 = sample.payload_sha256.clone();
        let acknowledged =
            tokio::task::spawn_blocking(move || ack_store.acknowledge(&sample_id, &payload_sha256))
                .await
                .map_err(|error| SyncError::Message(format!("semantic ACK task failed: {error}")))?
                .map_err(|error| {
                    SyncError::Message(format!("semantic ACK cleanup failed: {error}"))
                })?;
        if !acknowledged {
            return Err(SyncError::Message(format!(
                "semantic ACK no longer matched pending sample {}",
                sample.sample_id
            )));
        }
        uploaded += 1;
    }

    Ok(uploaded)
}

fn manifest(sample: &PendingSemanticSample) -> SemanticTreeManifest<'_> {
    SemanticTreeManifest {
        sample_id: &sample.sample_id,
        source_frame_id: sample.source_frame_id,
        surface_key: &sample.surface_key,
        layout_fingerprint: &sample.layout_fingerprint,
        schema_version: sample.schema_version,
        codec: &sample.codec,
        payload_sha256: &sample.payload_sha256,
        captured_at: &sample.captured_at,
        platform: &sample.platform,
        app_name: &sample.app_name,
        app_version: sample.app_version.as_deref(),
    }
}

fn validate_ack(
    sample: &PendingSemanticSample,
    ack: &SemanticTreeUploadResponse,
) -> Result<(), SyncError> {
    if !ack.ok || ack.sample_id != sample.sample_id || ack.payload_sha256 != sample.payload_sha256 {
        return Err(SyncError::Message(format!(
            "semantic endpoint returned a mismatched ACK for sample {}",
            sample.sample_id
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dystil_capture::semantic_tree::{SemanticSampleCandidate, MAX_SAMPLE_BYTES};
    use dystil_capture::AccessibilityNode;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn sample() -> PendingSemanticSample {
        PendingSemanticSample {
            sample_id: "de76f095-6808-4ae6-98c3-c0029d061f98".to_string(),
            source_frame_id: 42,
            surface_key: format!("sha256:{}", "1".repeat(64)),
            layout_fingerprint: format!("sha256:{}", "2".repeat(64)),
            schema_version: 1,
            codec: "zstd".to_string(),
            payload_sha256: format!("sha256:{}", "3".repeat(64)),
            payload: vec![1, 2, 3],
            captured_at: "2026-08-05T12:00:00Z".to_string(),
            platform: "windows".to_string(),
            app_name: "Teams".to_string(),
            app_version: Some("1.2.3".to_string()),
        }
    }

    #[test]
    fn manifest_uses_endpoint_contract_names() {
        let value = serde_json::to_value(manifest(&sample())).unwrap();
        assert_eq!(value["sourceFrameId"], 42);
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["appName"], "Teams");
        assert!(value.get("source_frame_id").is_none());
    }

    #[test]
    fn ack_must_match_both_identity_and_hash() {
        let sample = sample();
        let exact = SemanticTreeUploadResponse {
            ok: true,
            sample_id: sample.sample_id.clone(),
            payload_sha256: sample.payload_sha256.clone(),
            _inserted: true,
        };
        assert!(validate_ack(&sample, &exact).is_ok());
        assert!(validate_ack(
            &sample,
            &SemanticTreeUploadResponse {
                payload_sha256: format!("sha256:{}", "4".repeat(64)),
                ..exact
            }
        )
        .is_err());
    }

    #[tokio::test]
    async fn exact_http_ack_clears_the_local_payload() {
        let temp = tempfile::tempdir().unwrap();
        let store_path = temp.path().join("semantic.sqlite");
        let store = SemanticTreeStore::open(&store_path).unwrap();
        let node: AccessibilityNode = serde_json::from_value(serde_json::json!({
            "node_id": 1,
            "role": "window",
            "text": "visible content",
            "depth": 0,
            "bounds": {"left": 0.0, "top": 0.0, "width": 1.0, "height": 1.0},
            "on_screen": true
        }))
        .unwrap();
        store
            .record(SemanticSampleCandidate {
                source_frame_id: 42,
                captured_at: Utc::now(),
                platform: "windows",
                app_name: "Teams",
                app_version: Some("1.2.3"),
                window_name: Some("Chat"),
                browser_url: None,
                nodes: &[node],
            })
            .unwrap();
        let pending = store.pending(1).unwrap().pop().unwrap();
        assert!(!pending.payload.is_empty());
        assert!(pending.payload.len() <= MAX_SAMPLE_BYTES);

        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                // Some managed build sandboxes prohibit loopback sockets. The
                // pure ACK identity test and store cleanup test still run there;
                // ordinary CI hosts exercise this transport integration path.
                return;
            }
            Err(error) => panic!("failed to bind loopback test server: {error}"),
        };
        let address = listener.local_addr().unwrap();
        let ack_sample_id = pending.sample_id.clone();
        let ack_hash = pending.payload_sha256.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 8192];
            let expected_len = loop {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request closed before headers completed");
                request.extend_from_slice(&buffer[..read]);
                if let Some(header_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..header_end]);
                    assert!(headers.starts_with("POST /v1/semantic-tree-samples HTTP/1.1"));
                    assert!(headers
                        .to_ascii_lowercase()
                        .contains("authorization: device test-device-token"));
                    let content_length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|value| value.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    break header_end + 4 + content_length;
                }
            };
            while request.len() < expected_len {
                let read = socket.read(&mut buffer).await.unwrap();
                assert!(read > 0, "request closed before multipart body completed");
                request.extend_from_slice(&buffer[..read]);
            }
            assert!(request
                .windows(b"name=\"manifest\"".len())
                .any(|part| part == b"name=\"manifest\""));
            assert!(request
                .windows(b"name=\"payload\"".len())
                .any(|part| part == b"name=\"payload\""));
            let body = serde_json::json!({
                "ok": true,
                "sampleId": ack_sample_id,
                "payloadSha256": ack_hash,
                "inserted": true
            })
            .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let uploaded = upload_pending_semantic_samples(SemanticSyncConfig {
            store_path,
            cloud_base_url: format!("http://{address}"),
            device_token: "test-device-token".to_string(),
            request_timeout_secs: 5,
            app_version: Some("1.2.3".to_string()),
            build_channel: Some("test".to_string()),
            build_commit: None,
        })
        .await
        .unwrap();
        server.await.unwrap();

        assert_eq!(uploaded, 1);
        assert_eq!(store.pending_payload_bytes().unwrap(), 0);
        assert!(store.pending(1).unwrap().is_empty());
    }
}
