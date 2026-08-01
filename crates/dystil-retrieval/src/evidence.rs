use std::{fmt, str::FromStr};

use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::{Result, RetrievalError, RetrievalService};

pub const DEFAULT_SNIPPET_CHARS: usize = 500;
pub const MAX_SNIPPET_CHARS: usize = 1_200;
pub const DEFAULT_SOURCE_CHARS: usize = 8_000;
pub const MAX_SOURCE_CHARS: usize = 24_000;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvidenceId {
    source_type: String,
    row_id: i64,
}

impl EvidenceId {
    pub fn new(source_type: impl Into<String>, row_id: i64) -> Result<Self> {
        let source_type = source_type.into();
        if !matches!(source_type.as_str(), "frame" | "event") || row_id < 1 {
            return Err(RetrievalError::InvalidRequest(
                "evidence ID must be frame:<positive-id> or event:<positive-id>".into(),
            ));
        }
        Ok(Self {
            source_type,
            row_id,
        })
    }

    pub fn source_type(&self) -> &str {
        &self.source_type
    }

    pub fn row_id(&self) -> i64 {
        self.row_id
    }

    pub fn deep_link(&self) -> String {
        format!("dystil://evidence/{}/{}", self.source_type, self.row_id)
    }
}

impl fmt::Display for EvidenceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}:{}", self.source_type, self.row_id)
    }
}

impl FromStr for EvidenceId {
    type Err = RetrievalError;

    fn from_str(value: &str) -> Result<Self> {
        let (source_type, row_id) = value
            .split_once(':')
            .ok_or_else(|| RetrievalError::InvalidRequest("malformed evidence ID".into()))?;
        let row_id = row_id
            .parse::<i64>()
            .map_err(|_| RetrievalError::InvalidRequest("malformed evidence ID".into()))?;
        Self::new(source_type, row_id)
    }
}

impl Serialize for EvidenceId {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for EvidenceId {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub evidence_id: EvidenceId,
    pub timestamp: String,
    pub source_type: String,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub text: String,
    pub deep_link: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvidencePage {
    pub records: Vec<Evidence>,
    pub offset: u32,
    pub limit: u32,
    pub has_more: bool,
}

impl RetrievalService {
    pub async fn get_source(&self, id: &EvidenceId, max_chars: Option<usize>) -> Result<Evidence> {
        let record = dystil_storage::get_activity_source(&self.pool, &id.to_string())
            .await?
            .ok_or(RetrievalError::NotFound)?;
        Ok(evidence_from_record(
            record,
            max_chars
                .unwrap_or(DEFAULT_SOURCE_CHARS)
                .clamp(160, MAX_SOURCE_CHARS),
        )?)
    }
}

pub(crate) fn evidence_from_record(
    record: dystil_storage::ActivityRecord,
    max_chars: usize,
) -> Result<Evidence> {
    let evidence_id: EvidenceId = record.source_id.parse()?;
    let (text, truncated) = clip_middle(&record.text, max_chars);
    Ok(Evidence {
        source_type: evidence_id.source_type().to_string(),
        deep_link: evidence_id.deep_link(),
        evidence_id,
        timestamp: record.timestamp,
        app_name: record.app_name,
        window_name: record.window_name,
        browser_url: record.browser_url,
        text,
        truncated,
    })
}

pub(crate) fn clip_middle(value: &str, max_chars: usize) -> (String, bool) {
    let count = value.chars().count();
    if count <= max_chars {
        return (value.to_string(), false);
    }
    let marker = " … ";
    let keep = max_chars.saturating_sub(marker.chars().count());
    let head = keep * 2 / 3;
    let tail = keep - head;
    let start = value.chars().take(head).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<String>();
    (format!("{start}{marker}{end}"), true)
}
