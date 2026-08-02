use std::{collections::BTreeSet, env, str::FromStr};

use dystil_insights::*;
use serde_json::{json, Value};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row,
};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    let source_path = args
        .next()
        .ok_or("usage: replay_phase_g_fixture OLD_DB NEW_DB")?;
    let target_path = args
        .next()
        .ok_or("usage: replay_phase_g_fixture OLD_DB NEW_DB")?;
    let source = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(SqliteConnectOptions::from_str(&source_path)?.read_only(true))
        .await?;
    let target = open_insights_database(&target_path).await?;

    let evidence_rows = sqlx::query(
        "SELECT evidence_id,occurred_at,app,window FROM source_evidence ORDER BY evidence_id",
    )
    .fetch_all(&source)
    .await?;
    for row in evidence_rows {
        let evidence_id: String = row.get("evidence_id");
        let app: Option<String> = row.get("app");
        upsert_evidence(
            &target,
            &EvidenceRecord {
                evidence_id: evidence_id.clone(),
                source_namespace: "macos-golden-v1".into(),
                source_id: evidence_id,
                occurred_at: row.get("occurred_at"),
                app: app.clone(),
                window: row.get("window"),
                excerpt: format!(
                    "{} activity evidence",
                    app.unwrap_or_else(|| "Captured".into())
                ),
                policy_allowed: true,
                redaction_ready: true,
                deleted: false,
                sensitive: false,
            },
        )
        .await?;
    }
    let observation_rows = sqlx::query(
        "SELECT observation_id,source_key,statement,certainty,occurred_at,evidence_ids_json
         FROM source_observations ORDER BY occurred_at,observation_id",
    )
    .fetch_all(&source)
    .await?;
    for row in observation_rows {
        let certainty = match row.get::<String, _>("certainty").as_str() {
            "explicit" => ObservationCertainty::Explicit,
            "strongly_implied" => ObservationCertainty::StronglyImplied,
            _ => ObservationCertainty::Tentative,
        };
        admit_observation(
            &target,
            &ObservationRecord {
                observation_id: row.get("observation_id"),
                source_key: row.get("source_key"),
                occurred_at: row.get("occurred_at"),
                statement: row.get("statement"),
                certainty,
                evidence_ids: serde_json::from_str(row.get("evidence_ids_json"))?,
            },
        )
        .await?;
    }

    let rows = sqlx::query(
        "SELECT f.*,v.proposal_json FROM finding_versions f JOIN opportunity_versions v
         ON v.version_id=f.opportunity_version_id
         WHERE v.ordinal=(SELECT MAX(v2.ordinal) FROM opportunity_versions v2
           WHERE v2.opportunity_id=v.opportunity_id)
         ORDER BY f.finding_id",
    )
    .fetch_all(&source)
    .await?;
    let mut opportunities = Vec::new();
    let mut considered = BTreeSet::new();
    let mut filtered_legacy_findings = 0_u32;
    for row in rows {
        let construct = row.get::<String, _>("construct");
        let handoff: Value = serde_json::from_str(row.get("handoff_json"))?;
        let handoff_type = handoff
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let allowed = matches!(
            (construct.as_str(), handoff_type),
            (
                "recognition" | "manual_transfer",
                "prompt" | "existing_capability"
            ) | (
                "unchanged_repetition" | "temporal_pattern" | "repeated_composition",
                "runbook" | "saved_prompt"
            )
        );
        if !allowed {
            filtered_legacy_findings += 1;
            continue;
        }
        let proposal: Value = serde_json::from_str(row.get("proposal_json"))?;
        let version_id: String = row.get("opportunity_version_id");
        let occurrence_rows = sqlx::query(
            "SELECT o.* FROM opportunity_occurrences x JOIN occurrences o ON o.occurrence_id=x.occurrence_id
             WHERE x.opportunity_version_id=?1 ORDER BY x.ordinal,o.occurrence_id",
        ).bind(&version_id).fetch_all(&source).await?;
        let mut seen_occurrences = BTreeSet::new();
        let mut occurrences = Vec::new();
        for occurrence_row in occurrence_rows {
            let occurrence_id: String = occurrence_row.get("occurrence_id");
            if !seen_occurrences.insert(occurrence_id) {
                continue;
            }
            let old: Value = serde_json::from_str(occurrence_row.get("proposal_json"))?;
            let observation_ids: Vec<String> =
                serde_json::from_str(occurrence_row.get("observation_ids_json"))?;
            considered.extend(observation_ids.iter().cloned());
            occurrences.push(OccurrenceDelta {
                local_id: old
                    .get("occurrenceId")
                    .and_then(Value::as_str)
                    .unwrap_or("occ")
                    .into(),
                observation_ids,
                evidence_ids: serde_json::from_str(occurrence_row.get("evidence_ids_json"))?,
                steps: old
                    .get("steps")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                distinctness_basis: old
                    .get("distinctnessBasis")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
            });
        }
        let rank: Value = serde_json::from_str(row.get("rank_vector_json"))?;
        let evidence_state = proposal
            .get("evidenceState")
            .cloned()
            .unwrap_or(Value::Null);
        let construct = match construct.as_str() {
            "recognition" => Construct::Recognition,
            "manual_transfer" => Construct::ManualTransfer,
            "unchanged_repetition" => Construct::UnchangedRepetition,
            "temporal_pattern" => Construct::TemporalPattern,
            _ => Construct::RepeatedComposition,
        };
        let cadence = match evidence_state.get("cadence").and_then(Value::as_str) {
            Some("daily") => Cadence::Daily,
            Some("weekly") => Cadence::Weekly,
            Some("monthly") => Cadence::Monthly,
            _ => Cadence::None,
        };
        opportunities.push(OpportunityDelta {
            local_id: row.get::<String, _>("finding_id"),
            existing_opportunity_id: None,
            construct,
            summary: proposal
                .get("summary")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .into(),
            signature: serde_json::to_string(proposal.get("signature").unwrap_or(&Value::Null))?,
            occurrences_to_add: occurrences,
            withdraw_current_finding: false,
            retire: false,
            transfer_established: evidence_state
                .get("transferEstablished")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            authorship_established: evidence_state
                .get("authorshipEstablished")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            cadence,
            unresolved_questions: evidence_state
                .get("unresolvedQuestions")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect(),
            evidence_quality: EvidenceQuality::High,
            handoff: Some(Handoff {
                kind: match handoff_type {
                    "prompt" => HandoffType::Prompt,
                    "saved_prompt" => HandoffType::SavedPrompt,
                    "existing_capability" => HandoffType::ExistingCapability,
                    _ => HandoffType::Runbook,
                },
                title: handoff
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                preview: handoff
                    .get("preview")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .into(),
                capability_id: None,
            }),
            finding: Some(FindingDraft {
                claim: row.get("claim"),
                why_worth_fixing: row.get("why_worth_fixing"),
                evidence_ids: serde_json::from_str(row.get("evidence_ids_json"))?,
            }),
            rank_signals: RankSignals {
                actionability: rank
                    .get("actionability")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u8,
                estimated_burden: rank
                    .get("estimatedBurden")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u8,
                novelty: rank.get("novelty").and_then(Value::as_u64).unwrap_or(0) as u8,
                user_relevance: rank
                    .get("userRelevance")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u8,
                sensitivity_risk: rank
                    .get("sensitivityRisk")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u8,
            },
            automation_potential: proposal
                .get("automationPotential")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });
    }
    let observation_ids = considered.into_iter().collect::<Vec<_>>();
    let job_id = create_job(
        &target,
        NewJob {
            input_fingerprint: "macos-golden-v1-approved-contract",
            local_day: "2026-07-13",
            reason: "golden_fixture_replay",
            observation_ids: &observation_ids,
            prompt_hash: "accepted-artifact",
            schema_hash: "legacy-adapter-v1",
            model: "accepted-artifact",
            input_json: "{\"fixture\":\"phase-g-full-macos-incremental-v1\"}",
        },
    )
    .await?;
    claim_job(&target, &job_id).await?;
    let applied = apply_reconciliation(
        &target,
        &job_id,
        &ReconciliationOutput {
            schema_version: 1,
            considered_observation_ids: observation_ids,
            opportunities,
        },
        ApplyOptions::default(),
    )
    .await?;
    let before = projection_fingerprint(&target).await?;
    rebuild_projections(&target).await?;
    let rebuilt = projection_fingerprint(&target).await?;
    let summary = worth_fixing_summary(&target, true).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "source_observations": sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM source_observations").fetch_one(&source).await?,
            "source_evidence": sqlx::query_scalar::<_,i64>("SELECT COUNT(*) FROM source_evidence").fetch_one(&source).await?,
            "filtered_legacy_findings": filtered_legacy_findings,
            "accepted_opportunities": applied.opportunities_changed,
            "accepted_findings": applied.findings_created,
            "selected_cards": summary.selected.len(),
            "deterministic_rebuild": before == rebuilt,
            "projection_fingerprint": rebuilt,
        }))?
    );
    source.close().await;
    target.close().await;
    Ok(())
}
