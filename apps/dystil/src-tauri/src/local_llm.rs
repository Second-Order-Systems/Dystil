use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

// ── Constants ──────────────────────────────────────────────────────

const GENERATOR_PORT: u16 = 18097;
const EMBEDDER_PORT: u16 = 18098;

const GENERATOR_MODEL_REPO: &str = "unsloth/Qwen3.5-2B-GGUF";
const GENERATOR_MODEL_FILENAME: &str = "Qwen3.5-2B-Q4_K_M.gguf";
const GENERATOR_MODEL_ID: &str = "qwen3.5-2b-q4_k_m";
const EMBEDDER_MODEL_REPO: &str = "LiquidAI/LFM2.5-Embedding-350M-GGUF";
const EMBEDDER_MODEL_FILENAME: &str = "LFM2.5-Embedding-350M-Q4_K_M.gguf";
const EMBEDDER_MODEL_ID: &str = "lfm2.5-embedding-350m-q4_k_m";

const HEALTH_CHECK_RETRIES: u32 = 30;
const HEALTH_CHECK_INTERVAL: Duration = Duration::from_secs(2);
const LLAMA_SERVER_VERSION: &str = "9789";

// ── Target triple for binary downloads ────────────────────────────

fn target_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "macos-arm64",
        ("macos", "x86_64") => "macos-x86_64",
        ("windows", "x86_64") => "win-x86_64",
        ("linux", "x86_64") => "ubuntu-x86_64",
        ("linux", "aarch64") => "ubuntu-arm64",
        _ => "unknown",
    }
}

fn archive_name() -> String {
    format!("llama-b{LLAMA_SERVER_VERSION}-bin-{}.zip", target_triple())
}

fn archive_url() -> String {
    format!(
        "https://github.com/ggml-org/llama.cpp/releases/download/b{LLAMA_SERVER_VERSION}/{name}",
        name = archive_name()
    )
}

// ── Managed server ─────────────────────────────────────────────────

struct ManagedServer {
    child: Child,
    port: u16,
    kind: &'static str,
}

impl ManagedServer {
    fn spawn(binary: &str, model_path: &str, port: u16, embed: bool) -> Result<Self, String> {
        let mut args: Vec<String> = vec![
            "-m".into(),
            model_path.into(),
            "--host".into(),
            "127.0.0.1".into(),
            "--port".into(),
            port.to_string(),
        ];
        if embed {
            args.extend_from_slice(&[
                "--embeddings".into(),
                "--gpu-layers".into(),
                "0".into(),
                "--ctx-size".into(),
                "1024".into(),
                "--batch-size".into(),
                "1024".into(),
                "--ubatch-size".into(),
                "1024".into(),
            ]);
        } else {
            args.extend_from_slice(&[
                "--no-mmproj".into(),
                "--ctx-size".into(),
                "16384".into(),
            ]);
        }
        let kind = if embed { "embedder" } else { "generator" };
        let child = Command::new(binary)
            .args(&args)
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("failed to spawn {binary}: {e}"))?;
        info!(port, kind, "llama-server started");
        Ok(Self { child, port, kind })
    }

    fn port(&self) -> u16 {
        self.port
    }

    async fn shutdown(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.child.wait().await;
        info!(port = self.port, kind = self.kind, "llama-server stopped");
    }
}

// ── Binary location & download ─────────────────────────────────────

fn bundled_bin_dir() -> PathBuf {
    let mut dirs = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            dirs.push(parent.to_path_buf());
            dirs.push(parent.join("binaries"));
        }
    }
    #[cfg(debug_assertions)]
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries"));
    dirs.into_iter().next().unwrap_or_else(|| PathBuf::from("."))
}

fn llama_server_name() -> String {
    if cfg!(target_os = "windows") {
        "llama-server.exe".into()
    } else {
        "llama-server".into()
    }
}

fn find_llama_server_on_path() -> Option<String> {
    let name = llama_server_name();
    std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path).find_map(|dir| {
                let candidate = dir.join(&name);
                if candidate.is_file() {
                    Some(candidate.to_string_lossy().to_string())
                } else {
                    None
                }
            })
        })
}

fn find_llama_server_bundled() -> Option<String> {
    let name = llama_server_name();
    let dir = bundled_bin_dir();
    let candidate = dir.join(&name);
    if candidate.is_file() {
        return Some(candidate.to_string_lossy().to_string());
    }
    None
}

fn find_llama_server() -> Option<String> {
    find_llama_server_on_path()
        .or_else(find_llama_server_bundled)
}

async fn download_llama_server(bin_dir: &Path) -> Result<String, String> {
    std::fs::create_dir_all(bin_dir).map_err(|e| format!("failed to create {bin_dir:?}: {e}"))?;

    let url = archive_url();
    let name = archive_name();
    let zip_path = bin_dir.join(&name);
    let server_name = llama_server_name();
    let dest = bin_dir.join(&server_name);

    if dest.is_file() {
        info!("llama-server already downloaded at {dest:?}");
        return Ok(dest.to_string_lossy().to_string());
    }

    info!(%url, "downloading llama-server");
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("llama-server download failed: {e}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("llama-server download read failed: {e}"))?;
    tokio::fs::write(&zip_path, &bytes)
        .await
        .map_err(|e| format!("llama-server zip write failed: {e}"))?;

    // Extract llama-server from the zip
    let file = std::fs::File::open(&zip_path).map_err(|e| format!("open zip: {e}"))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| format!("read zip: {e}"))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("zip entry {i}: {e}"))?;
        let entry_name = entry.name().to_string();
        let entry_path = Path::new(&entry_name);
        if entry_path.file_name().map(|n| n == server_name.as_str()).unwrap_or(false) {
            let mut out = std::fs::File::create(&dest)
                .map_err(|e| format!("create {dest:?}: {e}"))?;
            std::io::copy(&mut entry, &mut out)
                .map_err(|e| format!("extract {entry_name}: {e}"))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                    .ok();
            }
            info!("llama-server extracted to {dest:?}");
            let _ = std::fs::remove_file(&zip_path);
            return Ok(dest.to_string_lossy().to_string());
        }
    }
    Err("llama-server binary not found in downloaded archive".into())
}

// ── Model downloads ────────────────────────────────────────────────

async fn download_model(models_dir: &Path, repo: &str, filename: &str) -> Result<String, String> {
    let dest = models_dir.join(filename);
    if dest.is_file() {
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        if size > 0 {
            info!(path = ?dest, size, "model already cached");
            return Ok(dest.to_string_lossy().to_string());
        }
    }

    let url = format!("https://huggingface.co/{repo}/resolve/main/{filename}");
    let tmp = models_dir.join(format!("{}.{}.tmp", filename, Uuid::new_v4()));

    info!(%url, "downloading model");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1800))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("download {filename} failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("download {filename} returned {status}"));
    }

    let total = response.content_length().unwrap_or(0);
    let mut stream = std::io::Cursor::new(
        response
            .bytes()
            .await
            .map_err(|e| format!("read {filename} failed: {e}"))?,
    );
    {
        let mut file = std::fs::File::create(&tmp)
            .map_err(|e| format!("create {tmp:?}: {e}"))?;
        std::io::copy(&mut stream, &mut file)
            .map_err(|e| format!("write {filename} failed: {e}"))?;
    }
    std::fs::rename(&tmp, &dest).map_err(|e| format!("rename {tmp:?} -> {dest:?}: {e}"))?;
    info!(path = ?dest, total, "model downloaded");
    Ok(dest.to_string_lossy().to_string())
}

// ── Health checks ──────────────────────────────────────────────────

async fn wait_for_server(port: u16, kind: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/health");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    for attempt in 1..=HEALTH_CHECK_RETRIES {
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                info!(port, kind, "model server ready ({attempt} tries)");
                return Ok(());
            }
            _ => {
                sleep(HEALTH_CHECK_INTERVAL).await;
            }
        }
    }
    Err(format!("{kind} server on port {port} did not become healthy after {HEALTH_CHECK_RETRIES} tries"))
}

// ── Public API ─────────────────────────────────────────────────────

pub struct LocalLlmManager {
    generator: Option<ManagedServer>,
    embedder: Option<ManagedServer>,
}

impl LocalLlmManager {
    pub async fn start(data_dir: &Path) -> Self {
        if std::env::var("DYSTIL_WORK_CARD_LLM_URL").is_ok() {
            info!("DYSTIL_WORK_CARD_LLM_URL already set — using external endpoint");
            return Self { generator: None, embedder: None };
        }

        let models_dir = data_dir.join("models");
        if let Err(e) = std::fs::create_dir_all(&models_dir) {
            warn!("failed to create models dir {models_dir:?}: {e}");
            return Self { generator: None, embedder: None };
        }

        // Find or download llama-server binary
        let binary = match find_llama_server() {
            Some(b) => b,
            None => {
                info!("llama-server not on PATH — attempting download");
                let bin_dir = data_dir.join("bin");
                match download_llama_server(&bin_dir).await {
                    Ok(b) => b,
                    Err(e) => {
                        warn!("{e} — local LLM unavailable; set DYSTIL_WORK_CARD_LLM_URL to use an external endpoint");
                        return Self { generator: None, embedder: None };
                    }
                }
            }
        };

        // Download models in parallel
        let (gen_res, emb_res) = tokio::join!(
            download_model(&models_dir, GENERATOR_MODEL_REPO, GENERATOR_MODEL_FILENAME),
            download_model(&models_dir, EMBEDDER_MODEL_REPO, EMBEDDER_MODEL_FILENAME),
        );

        if let Err(ref e) = gen_res {
            warn!("generator model download failed: {e} — work card generation disabled");
        }
        let generator = gen_res.ok().and_then(|path| {
            ManagedServer::spawn(&binary, &path, GENERATOR_PORT, false)
                .map_err(|e| warn!("failed to spawn generator: {e}"))
                .ok()
        });

        if let Err(ref e) = emb_res {
            warn!("embedder model download failed: {e}");
        }
        let embedder = emb_res.ok().and_then(|path| {
            ManagedServer::spawn(&binary, &path, EMBEDDER_PORT, true)
                .map_err(|e| warn!("failed to spawn embedder: {e}"))
                .ok()
        });

        if let Some(ref server) = generator {
            if let Err(e) = wait_for_server(server.port(), "generator").await {
                warn!("{e}");
            } else {
                std::env::set_var(
                    "DYSTIL_WORK_CARD_LLM_URL",
                    format!("http://127.0.0.1:{GENERATOR_PORT}"),
                );
                std::env::set_var("DYSTIL_WORK_CARD_LLM_MODEL", GENERATOR_MODEL_ID);
                info!("local generator endpoint ready at port {GENERATOR_PORT}");
            }
        }

        if let Some(ref server) = embedder {
            if let Err(e) = wait_for_server(server.port(), "embedder").await {
                warn!("{e}");
            } else {
                std::env::set_var(
                    "DYSTIL_WORK_CARD_EMBEDDING_URL",
                    format!("http://127.0.0.1:{EMBEDDER_PORT}"),
                );
                std::env::set_var("DYSTIL_WORK_CARD_EMBEDDING_MODEL", EMBEDDER_MODEL_ID);
                info!("local embedder endpoint ready at port {EMBEDDER_PORT}");
            }
        }

        Self { generator, embedder }
    }

    pub async fn shutdown(&mut self) {
        if let Some(ref mut s) = self.embedder {
            s.shutdown().await;
        }
        if let Some(ref mut s) = self.generator {
            s.shutdown().await;
        }
    }

    pub fn is_generator_ready(&self) -> bool {
        std::env::var("DYSTIL_WORK_CARD_LLM_URL").is_ok()
    }
}
