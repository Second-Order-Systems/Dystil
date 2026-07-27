//! Local ONNX-runtime inference of the v45_phase5_pruned PII text redactor.
//!
//! xlm-roberta-base BIO token classifier, INT8-quantized, vocab-pruned
//! (250k → 81k tokens). 27-label taxonomy (O + B-/I- for 13 PII types).
//!
//! Off by default — feature-gated with `onnx-cpu`, `onnx-coreml`, and
//! `onnx-directml`. Enabling exactly one of `onnx-coreml` / `onnx-directml`
//! selects the GPU execution provider for that platform; without either, the
//! CPU EP is used.
//!
//! NOTE on CoreML: the text model uses dynamic sequence lengths that the ANE
//! compiler rejects. `onnx-coreml` therefore aliases to `onnx-cpu` — the
//! image model (if added) is the one that actually benefits from CoreML/ANE.
//!
//! ## Model layout
//!
//! Expects a directory containing:
//!   - `model_quantized.onnx` (or `model.onnx`)
//!   - `tokenizer.json` (HuggingFace fast-tokenizers format)
//!   - `config.json` with `id2label` for the 27 BIO tags
//!   - `remap.json` — full-vocab id → pruned-row remap (~1.2 MB)
//!
//! Downloaded from HuggingFace on first run to
//! `~/.dystil/models/v45_phase5_pruned/`.

#![allow(dead_code)] // some utilities only used under specific feature gates

use std::path::PathBuf;

use async_trait::async_trait;

use crate::{RedactError, RedactedSpan, RedactionOutput, Redactor, SpanLabel, TextRedactor};

const ONNX_REDACTOR_NAME: &str = "v45_phase5_pruned";
const ONNX_REDACTOR_VERSION: u32 = 5;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct OnnxConfig {
    pub model_dir: PathBuf,
    pub model_file: Option<String>,
    /// Maximum sequence length. Inputs longer than this are processed in
    /// overlapping windows. Default 256 — matches the v45 training config.
    pub max_seq_len: usize,
}

impl Default for OnnxConfig {
    fn default() -> Self {
        Self {
            model_dir: Self::default_model_dir(),
            model_file: None,
            max_seq_len: 256,
        }
    }
}

impl OnnxConfig {
    pub fn default_model_dir() -> PathBuf {
        dirs::home_dir()
            .map(|h| h.join(".dystil").join("models").join("v45_phase5_pruned"))
            .unwrap_or_else(|| PathBuf::from(".dystil/models/v45_phase5_pruned"))
    }

    fn resolve_model_file(&self) -> PathBuf {
        if let Some(name) = &self.model_file {
            return self.model_dir.join(name);
        }
        let int8 = self.model_dir.join("model_quantized.onnx");
        if int8.exists() {
            return int8;
        }
        self.model_dir.join("model.onnx")
    }

    fn tokenizer_path(&self) -> PathBuf {
        self.model_dir.join("tokenizer.json")
    }

    pub const HF_REPO_BASE: &'static str =
        "https://huggingface.co/screenpipe/pii-redactor/resolve/main/v45_phase5_pruned";

    pub const FILES: &'static [(&'static str, &'static str)] = &[
        (
            "model_quantized.onnx",
            "a966fe75b8b7b9042b6c4a9a5d3878ca3e4a00fdbae26e8fbc9be4f4bebf5a61",
        ),
        (
            "tokenizer.json",
            "d0091a328b3441d754e481db5a390d7f3b8dabc6016869fd13ba350d23ddc4cd",
        ),
        (
            "config.json",
            "61dc24e4e4816d723143974268ef0b7a303d4b1f208bdd96db4d38a3359036f2",
        ),
        (
            "remap.json",
            "8b540d411419c32a7b9d4359d7a05760f595d61a83b662eec84d3e7e999f1fca",
        ),
    ];

    /// Download model files from HuggingFace into `model_dir` if absent.
    /// SHA-256 verified; partial downloads are written to `.partial` and
    /// atomically renamed on success.
    /// Only available when the `onnx-cpu` feature is enabled (requires reqwest).
    #[cfg(feature = "onnx-cpu")]
    pub async fn ensure_model_present(&self) -> Result<(), RedactError> {
        tokio::fs::create_dir_all(&self.model_dir)
            .await
            .map_err(|e| {
                RedactError::Runtime(format!("mkdir {}: {e}", self.model_dir.display()))
            })?;

        for (filename, expected_sha) in Self::FILES {
            let target = self.model_dir.join(filename);
            if target.exists() {
                if !expected_sha.starts_with("REPLACE_") && !sha256_matches(&target, expected_sha)?
                {
                    tracing::warn!(
                        "v45_phase5_pruned {} sha256 mismatch, re-downloading",
                        filename
                    );
                } else {
                    continue;
                }
            }

            let url = format!("{}/{}", Self::HF_REPO_BASE, filename);
            let tmp = target.with_extension(format!(
                "{}.partial",
                target
                    .extension()
                    .and_then(|s| s.to_str())
                    .unwrap_or("download")
            ));
            tracing::info!("downloading {} -> {}", url, target.display());

            let resp = reqwest::get(&url)
                .await
                .map_err(|e| RedactError::Runtime(format!("GET {url}: {e}")))?
                .error_for_status()
                .map_err(|e| RedactError::Runtime(format!("HTTP {url}: {e}")))?;
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| RedactError::Runtime(format!("download body {url}: {e}")))?;
            tokio::fs::write(&tmp, &bytes)
                .await
                .map_err(|e| RedactError::Runtime(format!("write {}: {e}", tmp.display())))?;

            if !expected_sha.starts_with("REPLACE_") && !sha256_matches(&tmp, expected_sha)? {
                let _ = tokio::fs::remove_file(&tmp).await;
                return Err(RedactError::Runtime(format!(
                    "{filename} sha256 mismatch after download from {url}"
                )));
            }

            tokio::fs::rename(&tmp, &target).await.map_err(|e| {
                RedactError::Runtime(format!(
                    "rename {} -> {}: {e}",
                    tmp.display(),
                    target.display()
                ))
            })?;
        }

        Ok(())
    }
}

fn sha256_matches(path: &std::path::Path, expected: &str) -> Result<bool, RedactError> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)
        .map_err(|e| RedactError::Runtime(format!("read {}: {e}", path.display())))?;
    let mut h = Sha256::new();
    h.update(&bytes);
    let got = hex_encode(&h.finalize());
    Ok(got.eq_ignore_ascii_case(expected))
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}

fn map_label(name: &str) -> Option<SpanLabel> {
    Some(match name {
        "private_person" => SpanLabel::Person,
        "private_email" => SpanLabel::Email,
        "private_phone" => SpanLabel::Phone,
        "private_address" => SpanLabel::Address,
        "private_url" => SpanLabel::Url,
        "private_id" => SpanLabel::Id,
        "private_date" => SpanLabel::Date,
        "private_company" => SpanLabel::Company,
        "private_handle" => SpanLabel::Handle,
        "private_channel" => SpanLabel::Channel,
        "private_repo" => SpanLabel::Repo,
        "secret" => SpanLabel::Secret,
        "private_sensitive" => SpanLabel::Sensitive,
        _ => return None,
    })
}

fn render_redacted(text: &str, spans: &[RedactedSpan]) -> String {
    if spans.is_empty() {
        return text.to_string();
    }
    let mut sorted = spans.to_vec();
    sorted.sort_by_key(|s| s.start);
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for s in &sorted {
        if s.start < cursor {
            continue;
        }
        out.push_str(&text[cursor..s.start]);
        out.push_str(s.label.placeholder());
        cursor = s.end;
    }
    out.push_str(&text[cursor..]);
    out
}

// ---------------------------------------------------------------------------
// Stub when ONNX features are off
// ---------------------------------------------------------------------------

#[cfg(not(feature = "onnx-cpu"))]
pub struct OnnxRedactor {
    _cfg: OnnxConfig,
}

#[cfg(not(feature = "onnx-cpu"))]
impl OnnxRedactor {
    pub fn load(cfg: OnnxConfig) -> Result<Self, RedactError> {
        let _ = cfg;
        Err(RedactError::Unavailable(
            "ONNX text redactor compiled out (enable feature `onnx-cpu`)".into(),
        ))
    }

    pub async fn load_or_download(cfg: OnnxConfig) -> Result<Self, RedactError> {
        let _ = cfg;
        Err(RedactError::Unavailable(
            "ONNX text redactor compiled out (enable feature `onnx-cpu`)".into(),
        ))
    }
}

#[cfg(not(feature = "onnx-cpu"))]
#[async_trait]
impl Redactor for OnnxRedactor {
    fn name(&self) -> &str {
        ONNX_REDACTOR_NAME
    }
    fn version(&self) -> u32 {
        ONNX_REDACTOR_VERSION
    }
    async fn redact_batch(&self, _texts: &[String]) -> Result<Vec<RedactionOutput>, RedactError> {
        Err(RedactError::Unavailable(
            "ONNX text redactor compiled out".into(),
        ))
    }
}

#[cfg(not(feature = "onnx-cpu"))]
#[async_trait]
impl TextRedactor for OnnxRedactor {
    fn name(&self) -> &'static str {
        ONNX_REDACTOR_NAME
    }
    fn version(&self) -> u32 {
        ONNX_REDACTOR_VERSION
    }
    async fn redact(&self, _text: &str) -> Result<String, String> {
        Err("ONNX text redactor compiled out (enable feature `onnx-cpu`)".into())
    }
}

// ---------------------------------------------------------------------------
// Real implementation behind `onnx-cpu`
// ---------------------------------------------------------------------------

#[cfg(feature = "onnx-cpu")]
mod runtime {
    use super::*;
    use ndarray::{Array, Axis};
    use ort::session::{builder::GraphOptimizationLevel, Session};
    use ort::value::TensorRef;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use tokenizers::Tokenizer;

    pub struct OnnxRedactor {
        cfg: OnnxConfig,
        session: Mutex<Session>,
        tokenizer: Tokenizer,
        id2label: Vec<String>,
        remap: Option<HashMap<u32, u32>>,
        unk_remapped: u32,
    }

    impl OnnxRedactor {
        pub async fn load_or_download(cfg: OnnxConfig) -> Result<Self, RedactError> {
            cfg.ensure_model_present().await?;
            Self::load(cfg)
        }

        pub fn load(cfg: OnnxConfig) -> Result<Self, RedactError> {
            let model_path = cfg.resolve_model_file();
            if !model_path.exists() {
                return Err(RedactError::Unavailable(format!(
                    "ONNX model not found at {}",
                    model_path.display()
                )));
            }
            let tokenizer_path = cfg.tokenizer_path();
            if !tokenizer_path.exists() {
                return Err(RedactError::Unavailable(format!(
                    "tokenizer not found at {}",
                    tokenizer_path.display()
                )));
            }
            let config_path = cfg.model_dir.join("config.json");
            if !config_path.exists() {
                return Err(RedactError::Unavailable(format!(
                    "config.json not found at {}",
                    config_path.display()
                )));
            }

            let id2label = parse_id2label(&config_path)?;

            let mut tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|e| {
                RedactError::Runtime(format!("tokenizer load {}: {e}", tokenizer_path.display()))
            })?;
            tokenizer
                .with_truncation(None)
                .map_err(|e| RedactError::Runtime(format!("disable tokenizer truncation: {e}")))?;

            let session = build_session(&model_path)?;
            let (remap, unk_remapped) = load_remap(&cfg.model_dir.join("remap.json"))?;

            Ok(Self {
                cfg,
                session: Mutex::new(session),
                tokenizer,
                id2label,
                remap,
                unk_remapped,
            })
        }

        fn infer(&self, text: &str) -> Result<RedactionOutput, RedactError> {
            if text.is_empty() {
                return Ok(RedactionOutput {
                    input: String::new(),
                    redacted: String::new(),
                    spans: Vec::new(),
                });
            }

            let enc = self
                .tokenizer
                .encode(text, true)
                .map_err(|e| RedactError::Runtime(format!("tokenize: {e}")))?;

            let max_len = self.cfg.max_seq_len.max(3);
            let ids = enc.get_ids();
            let mask = enc.get_attention_mask();
            let offsets = enc.get_offsets();

            let mut spans = if ids.len() <= max_len {
                let label_ids = self.run_window(ids, mask)?;
                bio_decode(text, &label_ids, offsets, &self.id2label)
            } else {
                self.infer_windowed(text, ids, offsets, max_len)?
            };
            merge_spans(&mut spans, text);

            let redacted = render_redacted(text, &spans);
            Ok(RedactionOutput {
                input: text.to_string(),
                redacted,
                spans,
            })
        }

        fn run_window(&self, ids: &[u32], mask: &[u32]) -> Result<Vec<usize>, RedactError> {
            let len = ids.len();
            let input_ids: Vec<i64> = match &self.remap {
                Some(remap) => ids
                    .iter()
                    .map(|x| *remap.get(x).unwrap_or(&self.unk_remapped) as i64)
                    .collect(),
                None => ids.iter().map(|x| *x as i64).collect(),
            };
            let attention_mask: Vec<i64> = mask.iter().map(|x| *x as i64).collect();

            let ids_arr = Array::from_shape_vec((1, len), input_ids)
                .map_err(|e| RedactError::Runtime(format!("ids shape: {e}")))?;
            let mask_arr = Array::from_shape_vec((1, len), attention_mask)
                .map_err(|e| RedactError::Runtime(format!("mask shape: {e}")))?;

            let mut session = self
                .session
                .lock()
                .map_err(|_| RedactError::Runtime("session mutex poisoned".into()))?;

            let outputs = session
                .run(ort::inputs![
                    "input_ids" => TensorRef::from_array_view(&ids_arr).map_err(|e| RedactError::Runtime(format!("ids tensor: {e}")))?,
                    "attention_mask" => TensorRef::from_array_view(&mask_arr).map_err(|e| RedactError::Runtime(format!("mask tensor: {e}")))?,
                ])
                .map_err(|e| RedactError::Runtime(format!("session.run: {e}")))?;

            let logits = outputs
                .get("logits")
                .ok_or_else(|| RedactError::Runtime("no logits output".into()))?;
            let logits_view = logits
                .try_extract_array::<f32>()
                .map_err(|e| RedactError::Runtime(format!("extract logits: {e}")))?;
            let logits_view = logits_view.view();
            let logits = logits_view.index_axis(Axis(0), 0);

            let mut label_ids = Vec::with_capacity(len);
            for row in logits.axis_iter(Axis(0)) {
                let mut best_i = 0usize;
                let mut best_v = f32::NEG_INFINITY;
                for (i, v) in row.iter().enumerate() {
                    if *v > best_v {
                        best_v = *v;
                        best_i = i;
                    }
                }
                label_ids.push(best_i);
            }
            Ok(label_ids)
        }

        fn infer_windowed(
            &self,
            text: &str,
            ids: &[u32],
            offsets: &[(usize, usize)],
            max_len: usize,
        ) -> Result<Vec<RedactedSpan>, RedactError> {
            let n = ids.len();
            let bos = ids[0];
            let eos = ids[n - 1];
            let content_ids = &ids[1..n - 1];
            let content_offsets = &offsets[1..n - 1];
            let content_len = content_ids.len();

            let win = max_len.saturating_sub(2).max(1);
            let overlap = (win / 4).min(48);
            let stride = win.saturating_sub(overlap).max(1);

            let mut spans = Vec::new();
            let mut start = 0usize;
            loop {
                let end = (start + win).min(content_len);
                let wlen = end - start + 2;
                let mut win_ids = Vec::with_capacity(wlen);
                let mut win_off = Vec::with_capacity(wlen);
                win_ids.push(bos);
                win_off.push((0usize, 0usize));
                win_ids.extend_from_slice(&content_ids[start..end]);
                win_off.extend_from_slice(&content_offsets[start..end]);
                win_ids.push(eos);
                win_off.push((0, 0));
                let win_mask = vec![1u32; wlen];

                let label_ids = self.run_window(&win_ids, &win_mask)?;
                let mut ws = bio_decode(text, &label_ids, &win_off, &self.id2label);
                spans.append(&mut ws);

                if end >= content_len {
                    break;
                }
                start += stride;
            }
            Ok(spans)
        }
    }

    #[async_trait]
    impl Redactor for OnnxRedactor {
        fn name(&self) -> &str {
            ONNX_REDACTOR_NAME
        }
        fn version(&self) -> u32 {
            ONNX_REDACTOR_VERSION
        }
        async fn redact_batch(
            &self,
            texts: &[String],
        ) -> Result<Vec<RedactionOutput>, RedactError> {
            let mut out = Vec::with_capacity(texts.len());
            for t in texts {
                out.push(self.infer(t)?);
            }
            Ok(out)
        }
    }

    #[async_trait]
    impl TextRedactor for OnnxRedactor {
        fn name(&self) -> &'static str {
            ONNX_REDACTOR_NAME
        }
        fn version(&self) -> u32 {
            ONNX_REDACTOR_VERSION
        }
        async fn redact(&self, text: &str) -> Result<String, String> {
            self.infer(text)
                .map(|o| o.redacted)
                .map_err(|e| e.to_string())
        }
    }

    fn bio_decode(
        text: &str,
        label_ids: &[usize],
        offsets: &[(usize, usize)],
        id2label: &[String],
    ) -> Vec<RedactedSpan> {
        let mut out = Vec::new();
        let mut cur: Option<(SpanLabel, usize, usize)> = None;

        let flush = |cur: &mut Option<(SpanLabel, usize, usize)>,
                     out: &mut Vec<RedactedSpan>,
                     text: &str| {
            if let Some((label, start, end)) = cur.take() {
                if end > start {
                    out.push(RedactedSpan {
                        start,
                        end,
                        label,
                        subtype: None,
                        text: text[start..end].to_string(),
                    });
                }
            }
        };

        for (i, &id) in label_ids.iter().enumerate() {
            let off = offsets.get(i).copied().unwrap_or((0, 0));
            if off.0 == off.1 {
                continue;
            }
            let tag = id2label.get(id).map(String::as_str).unwrap_or("O");
            if tag == "O" {
                flush(&mut cur, &mut out, text);
                continue;
            }
            let (prefix, category) = match tag.split_once('-') {
                Some((p, c)) => (p, c),
                None => {
                    flush(&mut cur, &mut out, text);
                    continue;
                }
            };
            let label = match map_label(category) {
                Some(l) => l,
                None => {
                    flush(&mut cur, &mut out, text);
                    continue;
                }
            };

            match prefix {
                "B" => {
                    if let Some((existing, _, end)) = cur.as_mut() {
                        if *existing == label {
                            *end = off.1;
                            continue;
                        }
                    }
                    flush(&mut cur, &mut out, text);
                    cur = Some((label, off.0, off.1));
                }
                "I" => {
                    if let Some((existing, _, end)) = cur.as_mut() {
                        if *existing == label {
                            *end = off.1;
                            continue;
                        }
                    }
                    flush(&mut cur, &mut out, text);
                    cur = Some((label, off.0, off.1));
                }
                _ => {
                    flush(&mut cur, &mut out, text);
                }
            }
        }
        flush(&mut cur, &mut out, text);
        out
    }

    fn merge_spans(spans: &mut Vec<RedactedSpan>, text: &str) {
        if spans.len() <= 1 {
            return;
        }
        spans.sort_by(|a, b| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
        let mut merged: Vec<RedactedSpan> = Vec::with_capacity(spans.len());
        for s in spans.drain(..) {
            if let Some(last) = merged.last_mut() {
                if s.start < last.end {
                    if s.end > last.end {
                        last.end = s.end;
                        last.text = text[last.start..last.end].to_string();
                    }
                    continue;
                }
            }
            merged.push(s);
        }
        *spans = merged;
    }

    fn load_remap(path: &std::path::Path) -> Result<(Option<HashMap<u32, u32>>, u32), RedactError> {
        if !path.exists() {
            return Ok((None, 0));
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| RedactError::Runtime(format!("read remap: {e}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| RedactError::Runtime(format!("parse remap: {e}")))?;
        let obj = parsed
            .get("remap")
            .and_then(|v| v.as_object())
            .ok_or_else(|| RedactError::Runtime("remap.json has no remap object".into()))?;
        let mut map: HashMap<u32, u32> = HashMap::with_capacity(obj.len());
        for (k, v) in obj {
            let old: u32 = k
                .parse()
                .map_err(|e| RedactError::Runtime(format!("remap key {k}: {e}")))?;
            let new = v
                .as_u64()
                .ok_or_else(|| RedactError::Runtime(format!("remap[{k}] not a u64")))?
                as u32;
            map.insert(old, new);
        }
        let unk = parsed
            .get("unk_new")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| RedactError::Runtime("remap.json has no unk_new".into()))?
            as u32;
        Ok((Some(map), unk))
    }

    fn parse_id2label(config_path: &std::path::Path) -> Result<Vec<String>, RedactError> {
        use std::collections::HashMap;
        let raw = std::fs::read_to_string(config_path)
            .map_err(|e| RedactError::Runtime(format!("read config: {e}")))?;
        let parsed: serde_json::Value = serde_json::from_str(&raw)
            .map_err(|e| RedactError::Runtime(format!("parse config: {e}")))?;
        let map = parsed
            .get("id2label")
            .and_then(|v| v.as_object())
            .ok_or_else(|| RedactError::Runtime("config.json has no id2label".into()))?;
        let mut entries: HashMap<usize, String> = HashMap::with_capacity(map.len());
        for (k, v) in map {
            let id: usize = k
                .parse()
                .map_err(|e| RedactError::Runtime(format!("id key {k}: {e}")))?;
            let label = v
                .as_str()
                .ok_or_else(|| RedactError::Runtime(format!("id2label[{k}] not string")))?
                .to_string();
            entries.insert(id, label);
        }
        let max_id = *entries.keys().max().unwrap_or(&0);
        let mut out = vec!["O".to_string(); max_id + 1];
        for (id, label) in entries {
            out[id] = label;
        }
        Ok(out)
    }

    fn build_session(model_path: &std::path::Path) -> Result<Session, RedactError> {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            || -> Result<Session, ort::Error> {
                let builder = Session::builder()?
                    .with_optimization_level(GraphOptimizationLevel::Level3)?
                    // Background batch worker — don't let the intra-op pool busy-spin.
                    .with_intra_op_spinning(false)?
                    .with_intra_threads((num_cpus_physical() / 2).max(2))?;

                // Note: CoreML EP intentionally omitted for this text model.
                // The ANE compiler rejects dynamic sequence lengths ("E5RT: unbounded
                // dimension is not supported"). CPU EP is the right choice here.
                #[cfg(feature = "onnx-directml")]
                let builder = builder.with_execution_providers([
                    ort::execution_providers::DirectMLExecutionProvider::default()
                        .with_device_id(0)
                        .build(),
                    ort::execution_providers::CPUExecutionProvider::default().build(),
                ])?;

                builder.commit_from_file(model_path)
            },
        )) {
            Ok(Ok(session)) => Ok(session),
            Ok(Err(e)) => Err(RedactError::Runtime(format!("ort session: {e}"))),
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&'static str>()
                    .map(|s| (*s).to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "unknown panic".to_string());
                Err(RedactError::Runtime(format!(
                    "ort session init panicked: {msg}"
                )))
            }
        }
    }

    fn num_cpus_physical() -> usize {
        std::thread::available_parallelism()
            .map(|n| n.get().clamp(1, 8))
            .unwrap_or(4)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn map_label_known() {
            assert_eq!(map_label("private_person"), Some(SpanLabel::Person));
            assert_eq!(map_label("private_sensitive"), Some(SpanLabel::Sensitive));
            assert_eq!(map_label("secret"), Some(SpanLabel::Secret));
        }

        #[test]
        fn map_label_unknown() {
            assert_eq!(map_label("totally_made_up"), None);
        }

        #[test]
        fn bio_decode_simple() {
            let id2label = vec![
                "O".to_string(),
                "B-private_person".to_string(),
                "I-private_person".to_string(),
            ];
            let text = "M  C  X";
            let label_ids = vec![1, 2, 0];
            let offsets = vec![(0, 1), (3, 4), (6, 7)];
            let spans = bio_decode(text, &label_ids, &offsets, &id2label);
            assert_eq!(spans.len(), 1);
            assert_eq!(spans[0].label, SpanLabel::Person);
            assert_eq!(spans[0].start, 0);
            assert_eq!(spans[0].end, 4);
        }

        #[test]
        fn merge_spans_dedups() {
            let text = "aaaa SECRETVALUE bbbb";
            let mut spans = vec![
                RedactedSpan {
                    start: 5,
                    end: 16,
                    label: SpanLabel::Secret,
                    subtype: None,
                    text: "SECRETVALUE".into(),
                },
                RedactedSpan {
                    start: 5,
                    end: 11,
                    label: SpanLabel::Secret,
                    subtype: None,
                    text: "SECRET".into(),
                },
            ];
            merge_spans(&mut spans, text);
            assert_eq!(spans.len(), 1);
            assert_eq!((spans[0].start, spans[0].end), (5, 16));
        }
    }
}

#[cfg(feature = "onnx-cpu")]
pub use runtime::OnnxRedactor;

// ---------------------------------------------------------------------------
// Cross-feature tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholder_renders_sensitive() {
        let spans = vec![RedactedSpan {
            start: 5,
            end: 18,
            label: SpanLabel::Sensitive,
            subtype: None,
            text: "Schizophrenia".to_string(),
        }];
        let text = "Note Schizophrenia at chart";
        let r = render_redacted(text, &spans);
        assert!(r.contains("[SENSITIVE]"));
        assert!(!r.contains("Schizophrenia"));
    }

    #[test]
    fn missing_model_path_is_unavailable() {
        let res = OnnxRedactor::load(OnnxConfig {
            model_dir: PathBuf::from("/nonexistent/dir"),
            model_file: None,
            max_seq_len: 256,
        });
        assert!(matches!(res, Err(RedactError::Unavailable(_))));
    }
}
