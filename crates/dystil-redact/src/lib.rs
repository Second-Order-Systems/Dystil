//! Dystil-owned text privacy boundary.
//!
//! This crate intentionally handles text only. Images are never inspected or
//! modified here. `sanitize_text` is deterministic and cheap enough to run
//! before every local SQLite write; an asynchronous model may strengthen the
//! result later but is not required for cloud safety.

use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::{Regex, RegexSet};
use sqlx::SqlitePool;
use thiserror::Error;

pub mod onnx;

// ---------------------------------------------------------------------------
// PII span taxonomy (used by onnx.rs and exposed for callers)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpanLabel {
    Person,
    Email,
    Phone,
    Address,
    Url,
    Company,
    Repo,
    Handle,
    Channel,
    Id,
    Date,
    Secret,
    Sensitive,
}

impl SpanLabel {
    pub fn placeholder(self) -> &'static str {
        match self {
            Self::Person => "[PERSON]",
            Self::Email => "[EMAIL]",
            Self::Phone => "[PHONE]",
            Self::Address => "[ADDRESS]",
            Self::Url => "[URL]",
            Self::Company => "[COMPANY]",
            Self::Repo => "[REPO]",
            Self::Handle => "[HANDLE]",
            Self::Channel => "[CHANNEL]",
            Self::Id => "[ID]",
            Self::Date => "[DATE]",
            Self::Secret => "[SECRET]",
            Self::Sensitive => "[SENSITIVE]",
        }
    }
}

/// A redacted region of an input string (byte offsets).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RedactedSpan {
    pub start: usize,
    pub end: usize,
    pub label: SpanLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    pub text: String,
}

/// Output from a single text redaction call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionOutput {
    pub input: String,
    pub redacted: String,
    pub spans: Vec<RedactedSpan>,
}

// ---------------------------------------------------------------------------
// Error type for model-backed redaction (distinct from DB StateError)
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum RedactError {
    #[error("redactor runtime error: {0}")]
    Runtime(String),
    #[error("redactor unavailable: {0}")]
    Unavailable(String),
}

// ---------------------------------------------------------------------------
// Internal batch trait — onnx.rs implements this
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Redactor: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> u32;
    async fn redact_batch(&self, texts: &[String]) -> Result<Vec<RedactionOutput>, RedactError>;
}

// ---------------------------------------------------------------------------
// Public single-string trait — the worker accepts Arc<dyn TextRedactor>
// ---------------------------------------------------------------------------

#[async_trait]
pub trait TextRedactor: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> u32;
    /// Redact one string in place. Returns the sanitized version.
    async fn redact(&self, text: &str) -> Result<String, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedactionStatus {
    Pending,
    Processing,
    Complete,
    DeterministicFallback,
}

impl RedactionStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Complete => "complete",
            Self::DeterministicFallback => "deterministic_fallback",
        }
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

/// Record one operational text-redaction surface. The source value is already
/// deterministic-safe before this function is called.
pub async fn record_state(
    pool: &SqlitePool,
    source_table: &str,
    source_row_id: i64,
    surface: &str,
    status: RedactionStatus,
    attempts: u32,
    backend: Option<&str>,
    last_error: Option<&str>,
) -> Result<(), StateError> {
    sqlx::query(
        "INSERT INTO dystil_text_redaction_state
            (source_table, source_row_id, surface, status, attempts, backend, last_error, updated_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now'))
         ON CONFLICT(source_table, source_row_id, surface) DO UPDATE SET
            status=excluded.status, attempts=excluded.attempts, backend=excluded.backend,
            last_error=excluded.last_error, updated_at=excluded.updated_at",
    )
    .bind(source_table)
    .bind(source_row_id)
    .bind(surface)
    .bind(status.as_str())
    .bind(attempts as i64)
    .bind(backend)
    .bind(last_error)
    .execute(pool)
    .await?;
    Ok(())
}

static PII_PATTERNS: Lazy<Vec<(Regex, &'static str)>> = Lazy::new(|| {
    vec![
        (Regex::new(r"\b(?:\d{4}[-\s]?){3}\d{4}\b").unwrap(), "CREDIT_CARD"),
        (Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap(), "SSN"),
        (Regex::new(r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Z|a-z]{2,}\b").unwrap(), "EMAIL"),
        (Regex::new(r"\+\d{1,3}[-.\s]?\(?[2-9]\d{2}\)?[-.\s]?\d{3}[-.\s]?\d{4}|\(?[2-9]\d{2}\)[-.\s]?\d{3}[-.\s]?\d{4}|[2-9]\d{2}[-.\s]\d{3}[-.\s]\d{4}").unwrap(), "PHONE"),
        (Regex::new(r"\b(?:(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\.){3}(?:25[0-5]|2[0-4][0-9]|[01]?[0-9][0-9]?)\b").unwrap(), "IP_ADDRESS"),
        (Regex::new(r"\beyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\b").unwrap(), "JWT_TOKEN"),
        (Regex::new(r"-----BEGIN[A-Z\s]+(?:PRIVATE KEY|SECRET)-----").unwrap(), "PRIVATE_KEY"),
        (Regex::new(r"(?i)(?:postgres|postgresql|mysql|mariadb|mongodb|mongodb\+srv|redis|rediss|amqp|amqps)://[^:]+:[^@]+@[^\s]+").unwrap(), "CONNECTION_STRING"),
        (Regex::new(r"[a-z][a-z0-9+.-]*://[^:]+:[^@]+@[^\s]+").unwrap(), "URL_WITH_CREDENTIALS"),
        (Regex::new(r"\b(?:sk_live|sk_test|pk_live|pk_test|whsec|rk_live|rk_test)_[A-Za-z0-9]{10,}").unwrap(), "STRIPE_KEY"),
        (Regex::new(r"\bsk-ant-(?:api|admin)\d{2}-[A-Za-z0-9_-]{40,}").unwrap(), "ANTHROPIC_KEY"),
        (Regex::new(r"\bsk-(?:proj-|svcacct-)?[A-Za-z0-9_-]{40,}\b").unwrap(), "OPENAI_KEY"),
        (Regex::new(r"\bAIza[A-Za-z0-9_-]{35}\b").unwrap(), "GOOGLE_API_KEY"),
        (Regex::new(r"\bhf_[A-Za-z0-9]{34}\b").unwrap(), "HUGGINGFACE_TOKEN"),
        (Regex::new(r"\bgh[pousr]_[A-Za-z0-9]{36,40}\b").unwrap(), "GITHUB_TOKEN"),
        (Regex::new(r"\b(?:xoxb|xoxp|xoxe|xoxa|xoxs|xapp)-[A-Za-z0-9-]{10,}").unwrap(), "SLACK_TOKEN"),
        (Regex::new(r"\bAKIA[0-9A-Z]{16}\b").unwrap(), "AWS_KEY"),
        (Regex::new(r"(?i)\b(?:authorization|bearer)\s*[:\s]\s*[A-Za-z0-9_-]{20,}").unwrap(), "AUTH_TOKEN"),
        (Regex::new(r"\b[A-Z][A-Z0-9_]*(?:SECRET|TOKEN|KEY|PASSWORD|CREDENTIAL)[A-Z0-9_]*\s*=\s*[^\s,;]{8,}").unwrap(), "ENV_SECRET"),
        (Regex::new(r"(?i)\b(?:seed|recovery|mnemonic|backup)\s*(?:phrase|words?)?\s*[:\s]\s*(?:[a-z]+\s+){11,23}[a-z]+").unwrap(), "SEED_PHRASE"),
        (Regex::new(r"[•·●○◦⦁⁃]{4,}|\.{8,}|\*{8,}").unwrap(), "PASSWORD_DOTS"),
    ]
});

static PASSWORD_CONTEXT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)((?:master\s+)?(?:password|passcode|passphrase|pin|secret\s*key|unlock\s*code|security\s*code)[\s]*[:=][\s]*)(\S+)").unwrap()
});

static PII_REGEX_SET: Lazy<RegexSet> =
    Lazy::new(|| RegexSet::new(PII_PATTERNS.iter().map(|(pattern, _)| pattern.as_str())).unwrap());

/// Deterministically replace sensitive text with stable category markers.
pub fn sanitize_text(text: &str) -> String {
    let matches: Vec<usize> = PII_REGEX_SET.matches(text).into_iter().collect();
    if matches.is_empty() && !PASSWORD_CONTEXT.is_match(text) {
        return text.to_string();
    }
    let mut sanitized = PASSWORD_CONTEXT
        .replace_all(text, "$1[PASSWORD]")
        .to_string();
    for index in matches {
        let (pattern, name) = &PII_PATTERNS[index];
        sanitized = pattern
            .replace_all(&sanitized, format!("[{name}]").as_str())
            .to_string();
    }
    sanitized
}

pub fn sanitize_optional(value: Option<&str>) -> Option<String> {
    value.map(sanitize_text)
}

#[cfg(test)]
mod tests {
    use super::sanitize_text;

    #[test]
    fn redacts_common_text_pii() {
        let output = sanitize_text("email a@example.com card 4111 1111 1111 1111");
        assert!(!output.contains("a@example.com"));
        assert!(!output.contains("4111"));
        assert!(output.contains("[EMAIL]"));
        assert!(output.contains("[CREDIT_CARD]"));
    }

    #[test]
    fn preserves_non_sensitive_text() {
        assert_eq!(
            sanitize_text("ordinary application title"),
            "ordinary application title"
        );
    }
}
