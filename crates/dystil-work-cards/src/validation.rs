use std::collections::{BTreeSet, HashSet};

use crate::{CompactedEvidence, GroundedClaim, WorkCard, WorkCardStatus};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub errors: Vec<ValidationError>,
    pub cited_evidence_ids: Vec<String>,
}

pub fn validate_work_card(card: &WorkCard, evidence: &[CompactedEvidence]) -> ValidationReport {
    let known = evidence
        .iter()
        .map(|item| item.evidence_id.as_str())
        .collect::<HashSet<_>>();
    let lookup = evidence
        .iter()
        .map(|item| {
            let searchable = [
                Some(item.text.as_str()),
                item.browser_url.as_deref(),
                item.window_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
            (item.evidence_id.as_str(), searchable)
        })
        .collect::<std::collections::HashMap<_, _>>();
    let known_applications = evidence
        .iter()
        .filter_map(|item| item.app_name.as_deref())
        .map(|value| value.to_lowercase())
        .collect::<HashSet<_>>();
    let mut errors = Vec::new();
    let mut cited = BTreeSet::new();

    validate_claim("summary", &card.summary, &known, &mut cited, &mut errors);
    validate_claim(
        "last_observed_state",
        &card.last_observed_state,
        &known,
        &mut cited,
        &mut errors,
    );
    for (index, action) in card.actions.iter().enumerate() {
        validate_claim(
            &format!("actions[{index}]"),
            action,
            &known,
            &mut cited,
            &mut errors,
        );
    }
    for (index, artifact) in card.artifacts.iter().enumerate() {
        validate_ids(
            &format!("artifacts[{index}].evidence_ids"),
            &artifact.evidence_ids,
            &known,
            &mut cited,
            &mut errors,
        );
        let needle = artifact.value.to_lowercase();
        if !needle.is_empty()
            && !artifact
                .evidence_ids
                .iter()
                .filter_map(|id| lookup.get(id.as_str()))
                .any(|text| text.contains(&needle))
        {
            errors.push(ValidationError {
                path: format!("artifacts[{index}].value"),
                message: "artifact value does not occur in its cited evidence".to_string(),
            });
        }
    }
    for (index, application) in card.applications.iter().enumerate() {
        if !known_applications.contains(&application.to_lowercase()) {
            errors.push(ValidationError {
                path: format!("applications[{index}]"),
                message: "application does not occur in supplied evidence".to_string(),
            });
        }
    }
    if card.title.trim().is_empty() {
        errors.push(ValidationError {
            path: "title".to_string(),
            message: "title must not be empty".to_string(),
        });
    }
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
    if matches!(card.status, WorkCardStatus::Completed)
        && !card
            .actions
            .iter()
            .chain(std::iter::once(&card.summary))
            .flat_map(|claim| claim.evidence_ids.iter())
            .filter_map(|id| lookup.get(id.as_str()))
            .any(|text| completion_terms.iter().any(|term| text.contains(term)))
    {
        errors.push(ValidationError {
            path: "status".to_string(),
            message: "completed status lacks explicit completion evidence".to_string(),
        });
    }

    ValidationReport {
        valid: errors.is_empty(),
        errors,
        cited_evidence_ids: cited.into_iter().collect(),
    }
}

fn validate_claim(
    path: &str,
    claim: &GroundedClaim,
    known: &HashSet<&str>,
    cited: &mut BTreeSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    if claim.text.trim().is_empty() {
        errors.push(ValidationError {
            path: format!("{path}.text"),
            message: "claim text must not be empty".to_string(),
        });
    }
    validate_ids(
        &format!("{path}.evidence_ids"),
        &claim.evidence_ids,
        known,
        cited,
        errors,
    );
}

fn validate_ids(
    path: &str,
    ids: &[String],
    known: &HashSet<&str>,
    cited: &mut BTreeSet<String>,
    errors: &mut Vec<ValidationError>,
) {
    if ids.is_empty() {
        errors.push(ValidationError {
            path: path.to_string(),
            message: "at least one evidence ID is required".to_string(),
        });
    }
    let mut unique = HashSet::new();
    for id in ids {
        if !unique.insert(id) {
            errors.push(ValidationError {
                path: path.to_string(),
                message: format!("duplicate evidence ID: {id}"),
            });
        }
        if !known.contains(id.as_str()) {
            errors.push(ValidationError {
                path: path.to_string(),
                message: format!("unknown evidence ID: {id}"),
            });
        } else {
            cited.insert(id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use dystil_protocol::SegmentEvidenceKind;

    fn evidence() -> Vec<CompactedEvidence> {
        vec![CompactedEvidence {
            evidence_id: "cev_1".into(),
            occurred_at: Utc::now(),
            kind: SegmentEvidenceKind::Screen,
            app_name: Some("Editor".into()),
            window_name: Some("auth.ts".into()),
            browser_url: None,
            text: "OAuth callback failure in auth.ts".into(),
            source_ids: vec!["item_1".into()],
            estimated_tokens: 8,
        }]
    }

    #[test]
    fn accepts_artifact_from_structured_url() {
        let mut evidence = evidence();
        evidence[0].browser_url = Some("https://example.test/ticket/42".into());
        let card = WorkCard {
            title: "Ticket work".into(),
            summary: GroundedClaim {
                text: "Viewed a ticket".into(),
                evidence_ids: vec!["cev_1".into()],
            },
            applications: vec!["Editor".into()],
            artifacts: vec![crate::GroundedArtifact {
                kind: "url".into(),
                value: "https://example.test/ticket/42".into(),
                evidence_ids: vec!["cev_1".into()],
            }],
            actions: Vec::new(),
            last_observed_state: GroundedClaim {
                text: "Ticket remained visible".into(),
                evidence_ids: vec!["cev_1".into()],
            },
            status: WorkCardStatus::Unknown,
            uncertainties: Vec::new(),
        };
        assert!(validate_work_card(&card, &evidence).valid);
    }

    #[test]
    fn rejects_unknown_citations_and_ungrounded_artifacts() {
        let card = WorkCard {
            title: "Authentication work".into(),
            summary: GroundedClaim {
                text: "Inspected auth".into(),
                evidence_ids: vec!["missing".into()],
            },
            applications: vec!["Editor".into()],
            artifacts: vec![crate::GroundedArtifact {
                kind: "file".into(),
                value: "invented.rs".into(),
                evidence_ids: vec!["cev_1".into()],
            }],
            actions: Vec::new(),
            last_observed_state: GroundedClaim {
                text: "auth.ts visible".into(),
                evidence_ids: vec!["cev_1".into()],
            },
            status: WorkCardStatus::Unknown,
            uncertainties: Vec::new(),
        };
        let report = validate_work_card(&card, &evidence());
        assert!(!report.valid);
        assert_eq!(report.errors.len(), 2);
    }
}
