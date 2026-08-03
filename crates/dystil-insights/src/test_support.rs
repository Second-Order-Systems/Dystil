use sqlx::SqlitePool;

use crate::*;

pub(crate) fn evidence(index: usize) -> EvidenceRecord {
    EvidenceRecord {
        evidence_id: format!("frame:{index}"),
        source_namespace: "device:test".into(),
        source_id: format!("frame:{index}"),
        occurred_at: format!("2026-01-{:02}T09:00:00Z", index.min(28)),
        app: Some("Editor".into()),
        window: Some("Report".into()),
        excerpt: format!("Evidence for occurrence {index}"),
        policy_allowed: true,
        redaction_ready: true,
        deleted: false,
        sensitive: false,
    }
}

fn observation(index: usize) -> ObservationRecord {
    ObservationRecord {
        observation_id: format!("obl_{index:024x}"),
        source_key: format!("artifact-test:{index}"),
        occurred_at: format!("2026-01-{:02}T09:00:00Z", index.min(28)),
        statement: format!("A useful observation number {index}"),
        certainty: ObservationCertainty::Explicit,
        evidence_ids: vec![format!("frame:{index}")],
    }
}

pub(crate) async fn seed_findings(pool: &SqlitePool, count: usize) -> Vec<String> {
    let claims = [
        "A prompt can triage the overflowing inbox.",
        "A prompt can normalize spreadsheet headings.",
        "A prompt can draft calendar follow-ups.",
        "A prompt can prepare customer meeting notes.",
        "A prompt can check release documentation.",
        "A prompt can organize product feedback.",
        "A prompt can summarize support escalations.",
    ];
    let mut finding_ids = Vec::new();
    for index in 1..=count {
        upsert_evidence(pool, &evidence(index)).await.unwrap();
        admit_observation(pool, &observation(index)).await.unwrap();
        let observation_id = format!("obl_{index:024x}");
        let job_id = create_job(
            pool,
            NewJob {
                input_fingerprint: &format!("artifact-input-{index}"),
                local_day: "2026-01-01",
                reason: "test",
                observation_ids: &[observation_id.clone()],
                prompt_hash: "prompt",
                schema_hash: "schema",
                model: "mock",
                input_json: "{}",
            },
        )
        .await
        .unwrap();
        claim_job(pool, &job_id).await.unwrap();
        let output = ReconciliationOutput {
            schema_version: 1,
            considered_observation_ids: vec![observation_id.clone()],
            opportunities: vec![OpportunityDelta {
                local_id: format!("opp_{index}"),
                existing_opportunity_id: None,
                construct: Construct::Recognition,
                summary: format!("Prepare report {index}"),
                signature: format!("prepare-report-{index}"),
                occurrences_to_add: vec![OccurrenceDelta {
                    local_id: format!("occ_{index}"),
                    observation_ids: vec![observation_id],
                    evidence_ids: vec![format!("frame:{index}")],
                    steps: vec!["prepare report".into()],
                    distinctness_basis: vec![],
                }],
                withdraw_current_finding: false,
                retire: false,
                transfer_established: false,
                authorship_established: false,
                cadence: Cadence::None,
                unresolved_questions: vec![],
                evidence_quality: EvidenceQuality::High,
                handoff: Some(Handoff {
                    kind: HandoffType::Prompt,
                    title: format!("Prepare report {index}"),
                    body: format!("Use this complete prompt to prepare report {index}."),
                    capability_id: None,
                }),
                finding: Some(FindingDraft {
                    claim: claims[(index - 1) % claims.len()].into(),
                    why_worth_fixing: "This avoids rebuilding the same instructions.".into(),
                    evidence_ids: vec![format!("frame:{index}")],
                }),
                rank_signals: RankSignals {
                    actionability: 3,
                    estimated_burden: 2,
                    novelty: 2,
                    user_relevance: 3,
                    sensitivity_risk: 0,
                },
                automation_potential: false,
            }],
        };
        apply_reconciliation(pool, &job_id, &output, ApplyOptions::default())
            .await
            .unwrap();
        finding_ids.push(
            sqlx::query_scalar::<_, String>(
                "SELECT f.finding_id FROM findings f JOIN opportunities o
                 ON o.opportunity_id=f.opportunity_id WHERE o.signature=?1",
            )
            .bind(format!("prepare-report-{index}"))
            .fetch_one(pool)
            .await
            .unwrap(),
        );
    }
    finding_ids
}
