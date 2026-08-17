use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Datelike, Utc};

use crate::{
    Cadence, Construct, EvidenceQuality, HandoffType, ObservationCertainty, OpportunityDelta,
    RankVector, WorthFixingCard,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EligibilityContext {
    pub occurrence_count: usize,
    pub cadence_supported: bool,
    pub capability_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Eligibility {
    pub eligible: bool,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingCandidate {
    pub card: WorthFixingCard,
    pub construct: Construct,
    pub rank_score: i32,
    pub rank_vector: RankVector,
    pub active: bool,
}

pub fn derive_eligibility(
    proposal: &OpportunityDelta,
    context: &EligibilityContext,
) -> Eligibility {
    let mut errors = Vec::new();
    if proposal.automation_potential && proposal.handoff.is_none() {
        errors.push("automation potential cannot replace a usable handoff".into());
    }
    let Some(handoff) = proposal.handoff.as_ref() else {
        errors.push("an eligible finding requires a handoff".into());
        return Eligibility {
            eligible: false,
            errors,
        };
    };
    if handoff.title.trim().is_empty() || handoff.title.chars().count() > 160 {
        errors.push("handoff title is not usable".into());
    }
    if handoff.body.trim().is_empty() || handoff.body.chars().count() > 12_000 {
        errors.push("handoff body is not complete and bounded".into());
    }
    if handoff.preview_steps.is_empty()
        || handoff.preview_steps.len() > 6
        || handoff
            .preview_steps
            .iter()
            .any(|step| step.trim().is_empty() || step.chars().count() > 280)
    {
        errors.push("handoff preview steps are not complete and bounded".into());
    }
    let Some(finding) = proposal.finding.as_ref() else {
        errors.push("an eligible finding requires a finding projection".into());
        return Eligibility {
            eligible: false,
            errors,
        };
    };
    if finding.evidence_note.trim().is_empty() || finding.evidence_note.chars().count() > 360 {
        errors.push("finding evidence note is not complete and bounded".into());
    }
    if handoff.kind == HandoffType::ExistingCapability && !context.capability_verified {
        errors.push("existing capability is not verified".into());
    }
    match proposal.construct {
        Construct::Recognition => {
            if context.occurrence_count < 1 {
                errors.push("recognition requires one occurrence".into());
            }
        }
        Construct::ManualTransfer => {
            if context.occurrence_count < 1 || !proposal.transfer_established {
                errors.push("manual transfer requires one established directional transfer".into());
            }
        }
        Construct::UnchangedRepetition => {
            if context.occurrence_count < 2 {
                errors.push("unchanged repetition requires two occurrences".into());
            }
        }
        Construct::TemporalPattern => {
            if context.occurrence_count < 3 || !context.cadence_supported {
                errors.push(
                    "temporal pattern requires three occurrences and supported cadence".into(),
                );
            }
        }
        Construct::RepeatedComposition => {
            if context.occurrence_count < 3 || !proposal.authorship_established {
                errors.push("repeated composition requires three authored occurrences".into());
            }
        }
    }
    Eligibility {
        eligible: errors.is_empty(),
        errors,
    }
}

pub fn handoff_preview(body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut preview = normalized.chars().take(240).collect::<String>();
    if normalized.chars().count() > 240 {
        preview.push('…');
    }
    preview
}

pub fn evidence_quality(
    certainties: &[ObservationCertainty],
    unresolved_specificity: bool,
) -> EvidenceQuality {
    if unresolved_specificity
        || certainties
            .iter()
            .any(|item| *item == ObservationCertainty::Tentative)
    {
        EvidenceQuality::Low
    } else if certainties
        .iter()
        .any(|item| *item == ObservationCertainty::StronglyImplied)
    {
        EvidenceQuality::Medium
    } else {
        EvidenceQuality::High
    }
}

pub fn rank(
    proposal: &OpportunityDelta,
    occurrence_count: usize,
    certainties: &[ObservationCertainty],
) -> (i32, RankVector) {
    let explicit = certainties
        .iter()
        .filter(|item| **item == ObservationCertainty::Explicit)
        .count();
    let explicit_evidence = if certainties.is_empty() {
        0
    } else {
        ((explicit as f64 / certainties.len() as f64) * 10.0).round() as i32
    };
    let occurrence_maturity = (occurrence_count as i32 * 6).min(20);
    let unresolved_penalty = (proposal.unresolved_questions.len() as i32 * 2).min(12);
    let evidence_quality_penalty = proposal.evidence_quality.penalty();
    let vector = RankVector {
        occurrence_maturity,
        occurrence_count: occurrence_count as i32,
        explicit_evidence,
        actionability: proposal.rank_signals.actionability as i32,
        estimated_burden: proposal.rank_signals.estimated_burden as i32,
        novelty: proposal.rank_signals.novelty as i32,
        user_relevance: proposal.rank_signals.user_relevance as i32,
        sensitivity_risk: proposal.rank_signals.sensitivity_risk as i32,
        unresolved_penalty,
        evidence_quality: proposal.evidence_quality,
        evidence_quality_penalty,
    };
    let score = 15
        + occurrence_maturity
        + explicit_evidence
        + vector.actionability * 5
        + vector.estimated_burden * 3
        + vector.novelty * 2
        + vector.user_relevance * 4
        - vector.sensitivity_risk * 4
        - unresolved_penalty
        - evidence_quality_penalty;
    (score, vector)
}

pub fn cadence_supported(cadence: Cadence, starts: &[DateTime<Utc>]) -> bool {
    if cadence == Cadence::None || starts.len() < 3 {
        return false;
    }
    let mut values = starts.to_vec();
    values.sort();
    let days: Vec<f64> = values
        .windows(2)
        .map(|pair| (pair[1] - pair[0]).num_seconds() as f64 / 86_400.0)
        .collect();
    match cadence {
        Cadence::Daily => days.iter().all(|days| (0.5..=1.5).contains(days)),
        Cadence::Weekly => {
            days.iter().all(|days| (5.0..=9.0).contains(days))
                && values
                    .iter()
                    .all(|value| value.weekday() == values[0].weekday())
        }
        Cadence::Monthly => {
            days.iter().all(|days| (24.0..=38.0).contains(days))
                && values
                    .iter()
                    .all(|value| (value.day() as i32 - values[0].day() as i32).unsigned_abs() <= 4)
        }
        Cadence::None => false,
    }
}

pub fn user_label(construct: Construct, occurrence_count: usize, cadence: Cadence) -> String {
    match construct {
        Construct::Recognition => "There is a faster way".into(),
        Construct::ManualTransfer => "You are carrying this by hand".into(),
        Construct::UnchangedRepetition => format!("Seen {occurrence_count} times, unchanged"),
        Construct::RepeatedComposition => "You write this one over and over".into(),
        Construct::TemporalPattern => match cadence {
            Cadence::Daily => "This comes back every day".into(),
            Cadence::Weekly => "This comes back every week".into(),
            Cadence::Monthly => "This comes back every month".into(),
            Cadence::None => "A recurring pattern is taking shape".into(),
        },
    }
}

fn normalized_words(value: &str) -> HashSet<String> {
    value
        .split(|character: char| !character.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|word| word.len() > 2)
        .collect()
}

fn similarity(left: &str, right: &str) -> f64 {
    let left = normalized_words(left);
    let right = normalized_words(right);
    let intersection = left.intersection(&right).count();
    let union = left.union(&right).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

pub fn select_top(mut candidates: Vec<FindingCandidate>, limit: usize) -> Vec<FindingCandidate> {
    candidates.sort_by(|left, right| {
        right
            .rank_score
            .cmp(&left.rank_score)
            .then_with(|| left.card.finding_id.cmp(&right.card.finding_id))
    });
    let mut selected: Vec<FindingCandidate> = Vec::new();
    let mut counts: HashMap<Construct, usize> = HashMap::new();
    for candidate in candidates.into_iter().filter(|candidate| candidate.active) {
        if counts
            .get(&candidate.construct)
            .copied()
            .unwrap_or_default()
            >= 2
        {
            continue;
        }
        if selected
            .iter()
            .any(|chosen| similarity(&chosen.card.claim, &candidate.card.claim) >= 0.72)
        {
            continue;
        }
        *counts.entry(candidate.construct).or_default() += 1;
        selected.push(candidate);
        if selected.len() >= limit.min(5) {
            break;
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;
    use crate::{CompletionState, FindingDraft, Handoff, OpportunityDelta, RankSignals};

    fn proposal(construct: Construct, handoff_type: HandoffType) -> OpportunityDelta {
        OpportunityDelta {
            local_id: "opp_01".into(),
            existing_opportunity_id: None,
            construct,
            summary: "summary".into(),
            signature: "signature".into(),
            occurrences_to_add: vec![],
            withdraw_current_finding: false,
            retire: false,
            transfer_established: false,
            authorship_established: false,
            cadence: Cadence::None,
            unresolved_questions: vec![],
            evidence_quality: EvidenceQuality::High,
            handoff: Some(Handoff {
                kind: handoff_type,
                title: "title".into(),
                body: "A complete and directly usable handoff body.".into(),
                preview_steps: vec!["Use the complete handoff.".into()],
                capability_id: None,
            }),
            finding: Some(FindingDraft {
                claim: "A useful claim".into(),
                why_worth_fixing: "A useful reason".into(),
                evidence_note: "An observed pattern supports this.".into(),
                evidence_ids: vec!["frame:1".into()],
                completion_state: CompletionState::Completed,
                workflow_stages: vec!["input".into(), "transform".into(), "handoff".into()],
            }),
            rank_signals: RankSignals {
                actionability: 3,
                estimated_burden: 2,
                novelty: 2,
                user_relevance: 3,
                sensitivity_risk: 0,
            },
            automation_potential: false,
        }
    }

    #[test]
    fn usable_handoff_is_not_coupled_to_the_inference_construct() {
        let context = EligibilityContext {
            occurrence_count: 1,
            cadence_supported: false,
            capability_verified: false,
        };
        assert!(
            derive_eligibility(
                &proposal(Construct::Recognition, HandoffType::Prompt),
                &context
            )
            .eligible
        );
        assert!(
            derive_eligibility(
                &proposal(Construct::Recognition, HandoffType::Runbook),
                &context
            )
            .eligible
        );
    }

    #[test]
    fn complete_handoff_body_is_required_and_preview_is_derived() {
        let context = EligibilityContext {
            occurrence_count: 1,
            cadence_supported: false,
            capability_verified: false,
        };
        let mut incomplete = proposal(Construct::Recognition, HandoffType::Prompt);
        incomplete.handoff.as_mut().unwrap().body.clear();
        assert!(!derive_eligibility(&incomplete, &context).eligible);
        let long = format!("{} end", "word ".repeat(80));
        let preview = handoff_preview(&long);
        assert!(preview.chars().count() <= 241);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn labels_use_durable_count_and_cadence() {
        assert_eq!(
            user_label(Construct::UnchangedRepetition, 7, Cadence::None),
            "Seen 7 times, unchanged"
        );
        assert_eq!(
            user_label(Construct::TemporalPattern, 3, Cadence::Weekly),
            "This comes back every week"
        );
    }

    #[test]
    fn evidence_quality_lowers_rank_without_changing_hard_eligibility() {
        let high = proposal(Construct::Recognition, HandoffType::Prompt);
        let mut low = high.clone();
        low.evidence_quality = EvidenceQuality::Low;
        let (high_score, _) = rank(&high, 1, &[ObservationCertainty::Explicit]);
        let (low_score, _) = rank(&low, 1, &[ObservationCertainty::Tentative]);
        assert!(high_score > low_score);
        assert!(
            derive_eligibility(
                &low,
                &EligibilityContext {
                    occurrence_count: 1,
                    cadence_supported: false,
                    capability_verified: false
                }
            )
            .eligible
        );
    }

    #[test]
    fn cadence_requires_three_supported_dates() {
        let dates = [
            Utc.with_ymd_and_hms(2026, 1, 5, 9, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 12, 9, 0, 0).unwrap(),
            Utc.with_ymd_and_hms(2026, 1, 19, 9, 0, 0).unwrap(),
        ];
        assert!(cadence_supported(Cadence::Weekly, &dates));
        assert!(!cadence_supported(Cadence::Daily, &dates));
    }
}
