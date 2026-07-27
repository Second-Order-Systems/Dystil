use std::collections::{HashMap, HashSet};

use crate::{CompactedEvidence, WorkCard, WorkCardStatus};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SanitizationStats {
    pub removed_applications: usize,
    pub removed_artifacts: usize,
    pub removed_duplicate_citations: usize,
    pub downgraded_completion: bool,
}

/// Applies only deterministic, evidence-preserving repairs.
///
/// Unsupported optional fields are removed and an unsupported completion status
/// is downgraded. Claim text is never rewritten or invented here.
pub fn sanitize_work_card(
    card: &mut WorkCard,
    evidence: &[CompactedEvidence],
) -> SanitizationStats {
    let mut stats = SanitizationStats::default();
    let known_ids = evidence
        .iter()
        .map(|item| item.evidence_id.as_str())
        .collect::<HashSet<_>>();
    let searchable = evidence
        .iter()
        .map(|item| {
            let value = [
                Some(item.text.as_str()),
                item.browser_url.as_deref(),
                item.window_name.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
            (item.evidence_id.as_str(), value)
        })
        .collect::<HashMap<_, _>>();
    let applications = evidence
        .iter()
        .filter_map(|item| item.app_name.as_deref())
        .map(normalize)
        .collect::<HashSet<_>>();

    card.applications.retain(|application| {
        let keep = applications.contains(&normalize(application));
        stats.removed_applications += usize::from(!keep);
        keep
    });
    card.applications.dedup();

    card.artifacts.retain(|artifact| {
        let needle = artifact.value.to_lowercase();
        let keep = !needle.is_empty()
            && artifact
                .evidence_ids
                .iter()
                .all(|id| known_ids.contains(id.as_str()))
            && artifact
                .evidence_ids
                .iter()
                .filter_map(|id| searchable.get(id.as_str()))
                .any(|text| text.contains(&needle));
        stats.removed_artifacts += usize::from(!keep);
        keep
    });

    for ids in std::iter::once(&mut card.summary.evidence_ids)
        .chain(std::iter::once(&mut card.last_observed_state.evidence_ids))
        .chain(card.actions.iter_mut().map(|claim| &mut claim.evidence_ids))
        .chain(
            card.artifacts
                .iter_mut()
                .map(|artifact| &mut artifact.evidence_ids),
        )
    {
        let original = ids.len();
        let mut seen = HashSet::new();
        ids.retain(|id| known_ids.contains(id.as_str()) && seen.insert(id.clone()));
        stats.removed_duplicate_citations += original - ids.len();
    }

    if matches!(card.status, WorkCardStatus::Completed) {
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
        let grounded = card
            .actions
            .iter()
            .chain(std::iter::once(&card.summary))
            .flat_map(|claim| claim.evidence_ids.iter())
            .filter_map(|id| searchable.get(id.as_str()))
            .any(|text| completion_terms.iter().any(|term| text.contains(term)));
        if !grounded {
            card.status = WorkCardStatus::Unknown;
            stats.downgraded_completion = true;
        }
    }

    stats
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use dystil_protocol::SegmentEvidenceKind;

    use super::*;
    use crate::{GroundedArtifact, GroundedClaim};

    #[test]
    fn drops_only_unsupported_optional_values_and_downgrades_status() {
        let evidence = vec![CompactedEvidence {
            evidence_id: "cev_1".into(),
            occurred_at: Utc::now(),
            kind: SegmentEvidenceKind::Screen,
            app_name: Some("Editor".into()),
            window_name: Some("auth.ts".into()),
            browser_url: None,
            text: "Editing callback handler".into(),
            source_ids: vec![],
            estimated_tokens: 4,
        }];
        let mut card = WorkCard {
            title: "Authentication".into(),
            summary: GroundedClaim {
                text: "Worked on callback handler".into(),
                evidence_ids: vec!["cev_1".into(), "cev_1".into()],
            },
            applications: vec!["Editor".into(), "Invented App".into()],
            artifacts: vec![
                GroundedArtifact {
                    kind: "file".into(),
                    value: "auth.ts".into(),
                    evidence_ids: vec!["cev_1".into()],
                },
                GroundedArtifact {
                    kind: "file".into(),
                    value: "invented.ts".into(),
                    evidence_ids: vec!["cev_1".into()],
                },
            ],
            actions: vec![],
            last_observed_state: GroundedClaim {
                text: "Handler remained visible".into(),
                evidence_ids: vec!["cev_1".into()],
            },
            status: WorkCardStatus::Completed,
            uncertainties: vec![],
        };
        let stats = sanitize_work_card(&mut card, &evidence);
        assert_eq!(card.applications, ["Editor"]);
        assert_eq!(card.artifacts.len(), 1);
        assert!(matches!(card.status, WorkCardStatus::Unknown));
        assert_eq!(stats.removed_applications, 1);
        assert_eq!(stats.removed_artifacts, 1);
        assert!(stats.downgraded_completion);
    }
}
