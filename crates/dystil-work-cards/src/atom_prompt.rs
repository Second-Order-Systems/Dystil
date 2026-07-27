use crate::{CompactedEvidence, EvidenceChunk, EvidenceWindow, MergedAtoms};
use serde::Serialize;
use serde_json::json;

pub fn build_atom_prompt(chunk: &EvidenceChunk) -> String {
    let records = chunk
        .evidence
        .iter()
        .map(PromptEvidence::from)
        .collect::<Vec<_>>();
    format!(
        r#"Extract observable work events as JSON only. Do not summarize the session. Do not infer intent, success, completion, ownership, or causality. Omit an event rather than guessing. Every atom must cite one or more exact evidence IDs from the input. `occurred_at` must exactly copy a cited record's `at` value. Application, object, result, and state values must be directly visible in cited input; omit unsupported optional fields. Do not output URLs, query strings, credentials, or private values.\n\nchunk_id: {}\nAllowed event types: opened, viewed, searched, navigated, edited, executed, tested, communicated, created, deleted, downloaded, uploaded, error_observed, result_observed, state_changed, other\n\nReturn shape: {{\"chunk_id\":\"{}\",\"atoms\":[{{\"atom_id\":\"a1\",\"occurred_at\":\"RFC3339 from input\",\"event_type\":\"viewed\",\"application\":null,\"action\":\"short factual observation\",\"object\":null,\"result\":null,\"state_before\":null,\"state_after\":null,\"evidence_ids\":[\"cev_...\"]}}],\"uncertainties\":[]}}\n\nInput JSON:\n{}"#,
        chunk.chunk_id,
        chunk.chunk_id,
        serde_json::to_string(&records).expect("serializable")
    )
}

pub fn atom_json_schema(chunk: &EvidenceChunk) -> serde_json::Value {
    let ids = chunk
        .evidence
        .iter()
        .map(|x| x.evidence_id.clone())
        .collect::<Vec<_>>();
    let types = [
        "opened",
        "viewed",
        "searched",
        "navigated",
        "edited",
        "executed",
        "tested",
        "communicated",
        "created",
        "deleted",
        "downloaded",
        "uploaded",
        "error_observed",
        "result_observed",
        "state_changed",
        "other",
    ];
    let citation =
        json!({"type":"array", "minItems":1, "maxItems":4, "items":{"type":"string", "enum":ids}});
    let optional_text = json!({"type":["string","null"], "maxLength":300});
    let atom = json!({
        "type":"object", "additionalProperties":false,
        "required":["atom_id","occurred_at","event_type","action","evidence_ids"],
        "properties": {
            "atom_id":{"type":"string","maxLength":64}, "occurred_at":{"type":"string"},
            "event_type":{"enum":types}, "application":{"type":["string","null"],"maxLength":100},
            "action":{"type":"string","maxLength":300}, "object":optional_text,
            "result":optional_text, "state_before":optional_text, "state_after":optional_text,
            "evidence_ids":citation
        }
    });
    let uncertainty = json!({"type":"object","additionalProperties":false,"required":["text","evidence_ids"],"properties":{"text":{"type":"string","maxLength":200},"evidence_ids":citation}});
    json!({"type":"object","additionalProperties":false,"required":["chunk_id","atoms","uncertainties"],"properties":{"chunk_id":{"const":chunk.chunk_id},"atoms":{"type":"array","maxItems":32,"items":atom},"uncertainties":{"type":"array","maxItems":3,"items":uncertainty}}})
}

pub fn build_card_prompt_from_atoms(window: &EvidenceWindow, atoms: &MergedAtoms) -> String {
    let atoms = serde_json::to_string(&atoms.atoms).expect("serializable");
    format!(
        r#"Return one grounded work-card JSON only. Use only the supplied factual atoms. Do not infer success, completion, intent, or causality. Every claim citation must use original `evidence_ids` from an atom. Be specific, concise, and resumable. Window: {} through {}. Required work-card schema is the same as work-card-v1: title, summary, applications, artifacts, actions, last_observed_state, status, uncertainties. Atoms: {}"#,
        window.start_time.to_rfc3339(),
        window.end_time.to_rfc3339(),
        atoms
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
    text: &'a str,
}
impl<'a> From<&'a CompactedEvidence> for PromptEvidence<'a> {
    fn from(v: &'a CompactedEvidence) -> Self {
        Self {
            id: &v.evidence_id,
            at: v.occurred_at.to_rfc3339(),
            app: v.app_name.as_deref(),
            window: v.window_name.as_deref(),
            text: &v.text,
        }
    }
}
