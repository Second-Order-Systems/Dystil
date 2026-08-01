use serde::{Deserialize, Serialize};

use crate::evidence::{
    clip_middle, evidence_from_record, Evidence, EvidencePage, DEFAULT_SNIPPET_CHARS,
    MAX_SNIPPET_CHARS,
};
use crate::{Result, RetrievalError, RetrievalService};

const DEFAULT_LIMIT: u32 = 5;
const MAX_LIMIT: u32 = 20;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SearchRequest {
    pub query: String,
    pub start_time: Option<String>,
    pub end_time: Option<String>,
    pub source_type: Option<String>,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub max_snippet_chars: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchPage {
    pub query: String,
    pub records: Vec<Evidence>,
    pub offset: u32,
    pub limit: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextRequest {
    pub evidence_id: crate::EvidenceId,
    pub before_seconds: Option<u32>,
    pub after_seconds: Option<u32>,
    pub limit: Option<u32>,
    pub max_content_chars: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RangeRequest {
    pub start_time: String,
    pub end_time: String,
    pub source_type: Option<String>,
    pub app_name: Option<String>,
    pub window_name: Option<String>,
    pub browser_url: Option<String>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    pub max_content_chars: Option<usize>,
}

impl RetrievalService {
    pub async fn search(&self, request: SearchRequest) -> Result<SearchPage> {
        if request.query.trim().is_empty() {
            return Err(RetrievalError::InvalidRequest(
                "search query cannot be empty".into(),
            ));
        }
        validate_source_type(request.source_type.as_deref())?;
        let limit = request.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
        let offset = request.offset.unwrap_or(0);
        let snippet_chars = request
            .max_snippet_chars
            .unwrap_or(DEFAULT_SNIPPET_CHARS)
            .clamp(160, MAX_SNIPPET_CHARS);
        let fetch_limit = (limit.saturating_mul(4)).min(80).saturating_add(1);
        let rows = dystil_storage::search_activity_filtered(
            &self.pool,
            &dystil_storage::ActivitySearchQuery {
                query: request.query.clone(),
                start_time: request.start_time,
                end_time: request.end_time,
                source_type: request.source_type,
                app_name: request.app_name,
                window_name: request.window_name,
                browser_url: request.browser_url,
                limit: fetch_limit,
                offset,
            },
        )
        .await?;

        let raw_page_full = rows.len() == fetch_limit as usize;
        let mut records = Vec::with_capacity(limit as usize);
        let mut fingerprints = std::collections::HashSet::new();
        for row in rows {
            let normalized = row
                .snippet
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase();
            let fingerprint = format!(
                "{}|{}|{}",
                row.record.app_name.as_deref().unwrap_or_default(),
                row.record.window_name.as_deref().unwrap_or_default(),
                normalized
            );
            if !fingerprints.insert(fingerprint) {
                continue;
            }
            let mut evidence = evidence_from_record(row.record, snippet_chars)?;
            let (snippet, truncated) = clip_middle(&row.snippet, snippet_chars);
            evidence.text = snippet;
            evidence.truncated = truncated;
            records.push(evidence);
            if records.len() > limit as usize {
                break;
            }
        }
        let has_more = records.len() > limit as usize || raw_page_full;
        records.truncate(limit as usize);
        Ok(SearchPage {
            query: request.query,
            records,
            offset,
            limit,
            has_more,
        })
    }

    pub async fn context(&self, request: ContextRequest) -> Result<EvidencePage> {
        let limit = request.limit.unwrap_or(20).clamp(1, 50);
        let max_chars = request.max_content_chars.unwrap_or(1_200).clamp(160, 8_000);
        let rows = dystil_storage::get_activity_context(
            &self.pool,
            &request.evidence_id.to_string(),
            request.before_seconds.unwrap_or(120).clamp(1, 3_600),
            request.after_seconds.unwrap_or(120).clamp(1, 3_600),
            limit.saturating_add(1),
        )
        .await?;
        evidence_page(rows, 0, limit, max_chars)
    }

    pub async fn range(&self, request: RangeRequest) -> Result<EvidencePage> {
        if request.start_time.trim().is_empty() || request.end_time.trim().is_empty() {
            return Err(RetrievalError::InvalidRequest(
                "start_time and end_time are required".into(),
            ));
        }
        validate_source_type(request.source_type.as_deref())?;
        let limit = request.limit.unwrap_or(20).clamp(1, 50);
        let offset = request.offset.unwrap_or(0);
        let max_chars = request.max_content_chars.unwrap_or(1_200).clamp(160, 8_000);
        let rows = dystil_storage::get_activity_range(
            &self.pool,
            &dystil_storage::ActivityRangeQuery {
                start_time: request.start_time,
                end_time: request.end_time,
                source_type: request.source_type,
                app_name: request.app_name,
                window_name: request.window_name,
                browser_url: request.browser_url,
                limit: limit.saturating_add(1),
                offset,
            },
        )
        .await?;
        evidence_page(rows, offset, limit, max_chars)
    }
}

fn evidence_page(
    rows: Vec<dystil_storage::ActivityRecord>,
    offset: u32,
    limit: u32,
    max_chars: usize,
) -> Result<EvidencePage> {
    let has_more = rows.len() > limit as usize;
    let records = rows
        .into_iter()
        .take(limit as usize)
        .map(|row| evidence_from_record(row, max_chars))
        .collect::<Result<Vec<_>>>()?;
    Ok(EvidencePage {
        records,
        offset,
        limit,
        has_more,
    })
}

fn validate_source_type(source_type: Option<&str>) -> Result<()> {
    if source_type.is_some_and(|value| !matches!(value, "frame" | "event")) {
        return Err(RetrievalError::InvalidRequest(
            "source_type must be frame or event".into(),
        ));
    }
    Ok(())
}
