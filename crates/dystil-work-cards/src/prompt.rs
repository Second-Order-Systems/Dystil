use serde::Serialize;
use serde_json::json;

use crate::{CompactedEvidence, EvidenceWindow};

#[derive(Debug, Clone)]
pub struct PromptConfig {
    pub schema_version: String,
}

impl Default for PromptConfig {
    fn default() -> Self {
        Self {
            schema_version: "work-card-v1".to_string(),
        }
    }
}

pub fn build_work_card_prompt(
    window: &EvidenceWindow,
    evidence: &[CompactedEvidence],
    config: &PromptConfig,
) -> String {
    let prompt_evidence = evidence
        .iter()
        .map(PromptEvidence::from)
        .collect::<Vec<_>>();
    let evidence_json = serde_json::to_string(&prompt_evidence).expect("evidence is serializable");
    format!(
        r#"Return one JSON record describing the supplied computer-use records.

Rules:
- Use only the supplied evidence. Do not use outside knowledge.
- Describe the actual task or subject in the evidence, not this conversion process.
- Never mention "evidence", "work card", "observed activity", or "converted" in the title or summary.
- Do not call the person an "agent" or "the user"; describe the task directly. Product names such as "voice agent" may be preserved.
- The title must name the most specific visible task, feature, issue, conversation, document, or system.
- Start the title with a task-specific verb supported by the records, such as edited, reviewed, debugged, discussed, tested, queried, configured, or monitored. Do not default to "investigated".
- Reject generic titles such as "Computer activity summary" or "Application usage".
- Keep the summary to one or two factual sentences. Avoid narrating timestamps, UI telemetry, or the fact that data was captured.
- Return at most three distinct actions and four artifacts. Do not restate the same fact in multiple fields.
- Keep last_observed_state to one short sentence describing only the final visible state.
- Every summary, action, artifact, and last_observed_state claim must cite one or more evidence_id values.
- Copy evidence_id values exactly. Never add, remove, or repeat an ID prefix.
- Do not infer motivation, ownership, causality, project membership, communication, completion, or success.
- Set status to "completed" only when evidence explicitly shows submission, completion, success, or closure.
- Otherwise use "in_progress", "blocked", or "unknown". Prefer "unknown" when uncertain.
- Artifact values must be exact verbatim substrings of their cited evidence.
- Preserve exact filenames, URLs, ticket numbers, application names, and identifiers.
- Put uncertainty in uncertainties rather than guessing.
- Return JSON only. No Markdown and no commentary.

Schema version: {schema_version}
Window: {start_time} through {end_time}

Required JSON shape:
{{
  "title": "short, concrete task-or-subject title",
  "summary": {{"text": "...", "evidence_ids": ["cev_..."]}},
  "applications": ["..."],
  "artifacts": [{{"kind": "file|url|ticket|document|other", "value": "...", "evidence_ids": ["cev_..."]}}],
  "actions": [{{"text": "...", "evidence_ids": ["cev_..."]}}],
  "last_observed_state": {{"text": "...", "evidence_ids": ["cev_..."]}},
  "status": "completed|in_progress|blocked|unknown",
  "uncertainties": ["..."]
}}

Evidence JSON:
{evidence_json}"#,
        schema_version = config.schema_version,
        start_time = window.start_time.to_rfc3339(),
        end_time = window.end_time.to_rfc3339(),
    )
}

#[derive(Serialize)]
struct PromptEvidence<'a> {
    id: &'a str,
    at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    app: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<&'a str>,
    text: &'a str,
}

impl<'a> From<&'a CompactedEvidence> for PromptEvidence<'a> {
    fn from(value: &'a CompactedEvidence) -> Self {
        Self {
            id: &value.evidence_id,
            at: value.occurred_at.format("%H:%M:%S").to_string(),
            app: value.app_name.as_deref(),
            window: value.window_name.as_deref(),
            url: value.browser_url.as_deref(),
            text: &value.text,
        }
    }
}

pub fn work_card_json_schema(evidence: &[CompactedEvidence]) -> serde_json::Value {
    let evidence_ids = evidence
        .iter()
        .map(|item| item.evidence_id.clone())
        .collect::<Vec<_>>();
    let completion_terms = [
        "completed",
        "complete",
        "submitted",
        "resolved",
        "success",
        "succeeded",
        "finished",
        "closed",
        "merged",
        "deployed",
        "done",
    ];
    let has_explicit_completion = evidence.iter().any(|item| {
        let text = item.text.to_lowercase();
        completion_terms.iter().any(|term| text.contains(term))
    });
    let statuses = if has_explicit_completion {
        vec!["completed", "in_progress", "blocked", "unknown"]
    } else {
        vec!["in_progress", "blocked", "unknown"]
    };
    let citations = json!({
        "type": "array",
        "minItems": 1,
        "maxItems": 4,
        "items": {"type": "string", "enum": evidence_ids}
    });
    let claim = json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["text", "evidence_ids"],
        "properties": {
            "text": {"type": "string", "maxLength": 500},
            "evidence_ids": citations
        }
    });
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": [
            "title", "summary", "applications", "artifacts", "actions",
            "last_observed_state", "status", "uncertainties"
        ],
        "properties": {
            "title": {"type": "string", "maxLength": 120},
            "summary": claim,
            "applications": {
                "type": "array", "maxItems": 4,
                "items": {"type": "string", "maxLength": 100}
            },
            "artifacts": {
                "type": "array", "maxItems": 4,
                "items": {
                    "type": "object", "additionalProperties": false,
                    "required": ["kind", "value", "evidence_ids"],
                    "properties": {
                        "kind": {"type": "string", "maxLength": 32},
                        "value": {"type": "string", "maxLength": 500},
                        "evidence_ids": citations
                    }
                }
            },
            "actions": {"type": "array", "maxItems": 3, "items": claim},
            "last_observed_state": claim,
            "status": {"type": "string", "enum": statuses},
            "uncertainties": {
                "type": "array", "maxItems": 3,
                "items": {"type": "string", "maxLength": 300}
            }
        }
    })
}
