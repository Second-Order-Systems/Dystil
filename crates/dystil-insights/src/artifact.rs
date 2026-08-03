//! Durable Ready-to-use artifacts and the atomic finding promotion boundary.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::{
    finding_evidence, recompute_surface_status_tx,
    store::{disposition_str, handoff_str, parse_handoff, stable_id},
    ArtifactChangeSummary, ArtifactPage, DispositionKind, HandoffType, InsightsError,
    KeepFindingResult, ReadyArtifactAction, ReadyArtifactCard, ReadyArtifactDetail,
    ReadyArtifactMutationResult, ReadyArtifactUseResult, Result, WorthFixingEvidenceLine,
};

#[derive(Debug, Clone, Copy, Default)]
pub struct KeepOptions {
    #[cfg(test)]
    pub failpoint: Option<KeepFailpoint>,
}

impl KeepOptions {
    fn has_failpoint(self) -> bool {
        #[cfg(test)]
        {
            self.failpoint.is_some()
        }
        #[cfg(not(test))]
        {
            false
        }
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeepFailpoint {
    AfterArtifact,
    AfterVersion,
    AfterDisposition,
}

fn actions(kind: HandoffType) -> (ReadyArtifactAction, ReadyArtifactAction) {
    match kind {
        HandoffType::Prompt | HandoffType::SavedPrompt => {
            (ReadyArtifactAction::Copy, ReadyArtifactAction::Open)
        }
        HandoffType::Runbook => (ReadyArtifactAction::Open, ReadyArtifactAction::Share),
        HandoffType::ExistingCapability => {
            (ReadyArtifactAction::Open, ReadyArtifactAction::ShowHow)
        }
    }
}

fn description(body: &str) -> String {
    let normalized = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut value = normalized.chars().take(180).collect::<String>();
    if normalized.chars().count() > 180 {
        value.push('…');
    }
    value
}

fn card_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<ReadyArtifactCard> {
    let kind = parse_handoff(row.get("kind"))?;
    let (primary_action, secondary_action) = actions(kind);
    Ok(ReadyArtifactCard {
        artifact_id: row.get("artifact_id"),
        title: row.get("title"),
        kind,
        description: description(row.get("body")),
        last_used_at: row.get("last_used_at"),
        primary_action,
        secondary_action,
    })
}

async fn active_card(pool: &SqlitePool, artifact_id: &str) -> Result<ReadyArtifactCard> {
    let row = sqlx::query(
        "SELECT a.artifact_id,a.title,a.kind,a.last_used_at,v.body
         FROM artifacts a JOIN artifact_versions v
           ON v.artifact_id=a.artifact_id AND v.ordinal=a.current_version
         WHERE a.artifact_id=?1 AND a.status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    card_from_row(&row)
}

async fn existing_keep_result(
    pool: &SqlitePool,
    finding_id: &str,
    provider_ready: bool,
) -> Result<Option<KeepFindingResult>> {
    let Some(row) =
        sqlx::query("SELECT artifact_id,status FROM artifacts WHERE source_finding_id=?1")
            .bind(finding_id)
            .fetch_optional(pool)
            .await?
    else {
        return Ok(None);
    };
    if row.get::<String, _>("status") != "active" {
        return Err(InsightsError::Invalid("kept artifact was removed".into()));
    }
    let artifact_id: String = row.get("artifact_id");
    Ok(Some(KeepFindingResult {
        artifact: active_card(pool, &artifact_id).await?,
        summary: crate::worth_fixing_summary(pool, provider_ready).await?,
        already_kept: true,
    }))
}

pub async fn keep_finding(
    pool: &SqlitePool,
    finding_id: &str,
    provider_ready: bool,
) -> Result<KeepFindingResult> {
    keep_finding_with_options(pool, finding_id, provider_ready, KeepOptions::default()).await
}

pub async fn keep_finding_with_options(
    pool: &SqlitePool,
    finding_id: &str,
    provider_ready: bool,
    options: KeepOptions,
) -> Result<KeepFindingResult> {
    match keep_finding_once(pool, finding_id, provider_ready, options).await {
        Err(InsightsError::Sqlx(sqlx::Error::Database(error)))
            if matches!(error.code().as_deref(), Some("5" | "517")) && !options.has_failpoint() =>
        {
            // A concurrent keep can cause SQLite's deferred read transaction to
            // lose the write race. The winner is durable; converge on its
            // artifact, or retry once if it has not committed yet.
            tokio::task::yield_now().await;
            if let Some(result) = existing_keep_result(pool, finding_id, provider_ready).await? {
                return Ok(result);
            }
            keep_finding_once(pool, finding_id, provider_ready, options).await
        }
        result => result,
    }
}

async fn keep_finding_once(
    pool: &SqlitePool,
    finding_id: &str,
    provider_ready: bool,
    options: KeepOptions,
) -> Result<KeepFindingResult> {
    #[cfg(not(test))]
    let _ = options;
    if let Some(result) = existing_keep_result(pool, finding_id, provider_ready).await? {
        return Ok(result);
    }

    let mut tx = pool.begin().await?;
    let finding = sqlx::query(
        "SELECT f.opportunity_id,f.version_id,f.active,f.handoff_type,f.handoff_title,
          f.handoff_body,
          NOT EXISTS(SELECT 1 FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
            WHERE fe.finding_id=f.finding_id AND
            (NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive)) evidence_available
         FROM findings f WHERE f.finding_id=?1",
    )
    .bind(finding_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| InsightsError::Invalid("unknown finding".into()))?;
    if !finding.get::<bool, _>("active") {
        tx.rollback().await?;
        if let Some(result) = existing_keep_result(pool, finding_id, provider_ready).await? {
            return Ok(result);
        }
        return Err(InsightsError::Invalid("finding is not active".into()));
    }
    if !finding.get::<bool, _>("evidence_available") {
        return Err(InsightsError::Invalid(
            "finding evidence is no longer available".into(),
        ));
    }
    let kind = parse_handoff(finding.get("handoff_type"))?;
    let title: String = finding.get("handoff_title");
    let body: String = finding.get("handoff_body");
    if title.trim().is_empty()
        || title.chars().count() > 160
        || body.trim().is_empty()
        || body.chars().count() > 12_000
    {
        return Err(InsightsError::Invalid(
            "finding does not contain a complete bounded artifact".into(),
        ));
    }
    let capability_id = if kind == HandoffType::ExistingCapability {
        let capability_id = sqlx::query_scalar::<_, String>(
            "SELECT json_extract(v.proposal_json,'$.handoff.capability_id')
             FROM opportunity_versions v WHERE v.version_id=?1",
        )
        .bind(finding.get::<String, _>("version_id"))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            InsightsError::Invalid("capability handoff has no catalog identity".into())
        })?;
        let actionable = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM capabilities WHERE capability_id=?1
             AND action_kind IS NOT NULL AND action_target IS NOT NULL",
        )
        .bind(&capability_id)
        .fetch_one(&mut *tx)
        .await?;
        if actionable != 1 {
            return Err(InsightsError::Invalid(
                "capability handoff is not actionable".into(),
            ));
        }
        Some(capability_id)
    } else {
        None
    };
    let artifact_id = stable_id("wfa", &("finding", finding_id))?;
    let now = Utc::now().to_rfc3339();
    let inserted = sqlx::query(
        "INSERT OR IGNORE INTO artifacts(
           artifact_id,source_kind,source_finding_id,source_request_id,kind,title,current_version,
           status,capability_id,kept_at,last_used_at,updated_at,removed_at)
         VALUES(?1,'finding',?2,NULL,?3,?4,1,'active',?5,?6,NULL,?6,NULL)",
    )
    .bind(&artifact_id)
    .bind(finding_id)
    .bind(handoff_str(kind))
    .bind(&title)
    .bind(&capability_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    if inserted.rows_affected() == 0 {
        tx.rollback().await?;
        let row =
            sqlx::query("SELECT artifact_id,status FROM artifacts WHERE source_finding_id=?1")
                .bind(finding_id)
                .fetch_optional(pool)
                .await?
                .ok_or_else(|| InsightsError::IdentityCollision(artifact_id.clone()))?;
        if row.get::<String, _>("status") != "active" {
            return Err(InsightsError::Invalid("kept artifact was removed".into()));
        }
        let existing_id: String = row.get("artifact_id");
        if existing_id != artifact_id {
            return Err(InsightsError::IdentityCollision(finding_id.into()));
        }
        return Ok(KeepFindingResult {
            artifact: active_card(pool, &artifact_id).await?,
            summary: crate::worth_fixing_summary(pool, provider_ready).await?,
            already_kept: true,
        });
    }
    #[cfg(test)]
    if options.failpoint == Some(KeepFailpoint::AfterArtifact) {
        return Err(InsightsError::Invalid("injected keep failure".into()));
    }
    let version_id = stable_id("wav", &(&artifact_id, 1, finding_id, &body))?;
    sqlx::query(
        "INSERT INTO artifact_versions(
           version_id,artifact_id,ordinal,title,body,source_finding_version_id,change_job_id,created_at)
         VALUES(?1,?2,1,?3,?4,?5,NULL,?6)",
    )
    .bind(&version_id)
    .bind(&artifact_id)
    .bind(&title)
    .bind(&body)
    .bind(finding.get::<String, _>("version_id"))
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    #[cfg(test)]
    if options.failpoint == Some(KeepFailpoint::AfterVersion) {
        return Err(InsightsError::Invalid("injected keep failure".into()));
    }
    let disposition_id = stable_id("wfd", &(finding_id, "saved"))?;
    sqlx::query(
        "INSERT INTO dispositions(disposition_id,finding_id,kind,correction_text,intent,created_at)
         VALUES(?1,?2,?3,NULL,NULL,?4)",
    )
    .bind(&disposition_id)
    .bind(finding_id)
    .bind(disposition_str(DispositionKind::Saved))
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    #[cfg(test)]
    if options.failpoint == Some(KeepFailpoint::AfterDisposition) {
        return Err(InsightsError::Invalid("injected keep failure".into()));
    }
    let event_id = stable_id("wae", &(&artifact_id, "kept"))?;
    sqlx::query(
        "INSERT INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'kept',NULL,?3)",
    )
    .bind(event_id)
    .bind(&artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE findings SET active=0 WHERE finding_id=?1")
        .bind(finding_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE opportunities SET current_status='withdrawn',updated_at=?2
         WHERE opportunity_id=?1",
    )
    .bind(finding.get::<String, _>("opportunity_id"))
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    recompute_surface_status_tx(&mut tx).await?;
    tx.commit().await?;
    Ok(KeepFindingResult {
        artifact: active_card(pool, &artifact_id).await?,
        summary: crate::worth_fixing_summary(pool, provider_ready).await?,
        already_kept: false,
    })
}

pub async fn ready_artifacts(
    pool: &SqlitePool,
    after_artifact_id: Option<&str>,
    limit: u32,
) -> Result<ArtifactPage> {
    let rows = sqlx::query(
        "SELECT a.artifact_id,a.title,a.kind,a.last_used_at,v.body
         FROM artifacts a JOIN artifact_versions v
           ON v.artifact_id=a.artifact_id AND v.ordinal=a.current_version
         WHERE a.status='active' AND (?1 IS NULL OR a.artifact_id>?1)
         ORDER BY a.artifact_id LIMIT ?2",
    )
    .bind(after_artifact_id)
    .bind(limit.clamp(1, 50) as i64 + 1)
    .fetch_all(pool)
    .await?;
    let limit = limit.clamp(1, 50) as usize;
    let has_more = rows.len() > limit;
    let mut items = rows
        .iter()
        .take(limit)
        .map(card_from_row)
        .collect::<Result<Vec<_>>>()?;
    let next_cursor = has_more
        .then(|| items.last().map(|item| item.artifact_id.clone()))
        .flatten();
    items.shrink_to_fit();
    Ok(ArtifactPage { items, next_cursor })
}

pub async fn ready_artifact_detail(
    pool: &SqlitePool,
    artifact_id: &str,
) -> Result<ReadyArtifactDetail> {
    let row = sqlx::query(
        "SELECT a.artifact_id,a.title,a.kind,a.last_used_at,a.kept_at,a.source_finding_id,v.body,
          (SELECT COUNT(*) FROM artifact_change_jobs j
             WHERE j.artifact_id=a.artifact_id AND j.status='accepted') change_count,
          NOT EXISTS(SELECT 1 FROM finding_evidence fe JOIN evidence e ON e.evidence_id=fe.evidence_id
            WHERE fe.finding_id=a.source_finding_id AND
            (NOT e.policy_allowed OR NOT e.redaction_ready OR e.deleted OR e.sensitive)) provenance_available,
          COALESCE((SELECT claim FROM findings WHERE finding_id=a.source_finding_id),'Kept from Dystil') provenance_label
         FROM artifacts a JOIN artifact_versions v
           ON v.artifact_id=a.artifact_id AND v.ordinal=a.current_version
         WHERE a.artifact_id=?1 AND a.status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    let changes = sqlx::query(
        "SELECT request_text,accepted_at FROM artifact_change_jobs
         WHERE artifact_id=?1 AND status='accepted' ORDER BY accepted_at DESC LIMIT 20",
    )
    .bind(artifact_id)
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|item| ArtifactChangeSummary {
        request: item.get("request_text"),
        changed_at: item.get("accepted_at"),
    })
    .collect();
    Ok(ReadyArtifactDetail {
        card: card_from_row(&row)?,
        body: row.get("body"),
        kept_at: row.get("kept_at"),
        change_count: row.get::<i64, _>("change_count") as u32,
        changes,
        provenance_available: row.get("provenance_available"),
        provenance_label: row.get("provenance_label"),
    })
}

pub async fn ready_artifact_provenance(
    pool: &SqlitePool,
    artifact_id: &str,
) -> Result<Vec<WorthFixingEvidenceLine>> {
    let finding_id = sqlx::query_scalar::<_, String>(
        "SELECT source_finding_id FROM artifacts WHERE artifact_id=?1 AND status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact has no finding provenance".into()))?;
    finding_evidence(pool, &finding_id, 50).await
}

pub async fn record_artifact_used(
    pool: &SqlitePool,
    artifact_id: &str,
    action: ReadyArtifactAction,
) -> Result<ReadyArtifactUseResult> {
    let kind = sqlx::query_scalar::<_, String>(
        "SELECT kind FROM artifacts WHERE artifact_id=?1 AND status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    let kind = parse_handoff(&kind)?;
    let allowed = matches!(action, ReadyArtifactAction::Open)
        || matches!(
            (kind, action),
            (
                HandoffType::Prompt | HandoffType::SavedPrompt,
                ReadyArtifactAction::Copy
            ) | (HandoffType::Runbook, ReadyArtifactAction::Share)
                | (
                    HandoffType::ExistingCapability,
                    ReadyArtifactAction::ShowHow
                )
        );
    if !allowed {
        return Err(InsightsError::Invalid(
            "action is not available for this artifact".into(),
        ));
    }
    let now = Utc::now().to_rfc3339();
    let event_id = stable_id("wae", &(artifact_id, format!("{action:?}"), &now))?;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE artifacts SET last_used_at=?2,updated_at=?2 WHERE artifact_id=?1")
        .bind(artifact_id)
        .bind(&now)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'used',?3,?4)",
    )
    .bind(event_id)
    .bind(artifact_id)
    .bind(format!("{action:?}").to_lowercase())
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ReadyArtifactUseResult {
        artifact_id: artifact_id.into(),
        last_used_at: now,
    })
}

pub async fn capability_target(pool: &SqlitePool, artifact_id: &str) -> Result<String> {
    let row = sqlx::query(
        "SELECT c.action_kind,c.action_target FROM artifacts a JOIN capabilities c
          ON c.capability_id=a.capability_id
         WHERE a.artifact_id=?1 AND a.status='active' AND a.kind='existing_capability'",
    )
    .bind(artifact_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not an actionable capability".into()))?;
    let kind: String = row.get("action_kind");
    let target: String = row.get("action_target");
    if kind != "https_url"
        || !target.starts_with("https://")
        || target.chars().count() > 2_048
        || target.chars().any(char::is_control)
    {
        return Err(InsightsError::Invalid(
            "capability target is outside the allow-list".into(),
        ));
    }
    Ok(target)
}

pub async fn remove_artifact(
    pool: &SqlitePool,
    artifact_id: &str,
) -> Result<ReadyArtifactMutationResult> {
    let mut tx = pool.begin().await?;
    let revision = sqlx::query_scalar::<_, i64>(
        "SELECT current_version FROM artifacts WHERE artifact_id=?1 AND status='active'",
    )
    .bind(artifact_id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| InsightsError::Invalid("artifact is not active".into()))?;
    sqlx::query(
        "DELETE FROM artifact_change_attempts WHERE job_id IN
         (SELECT job_id FROM artifact_change_jobs WHERE artifact_id=?1)",
    )
    .bind(artifact_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query("DELETE FROM artifact_versions WHERE artifact_id=?1")
        .bind(artifact_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM artifact_change_jobs WHERE artifact_id=?1")
        .bind(artifact_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM artifact_events WHERE artifact_id=?1")
        .bind(artifact_id)
        .execute(&mut *tx)
        .await?;
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE artifacts SET title='',current_version=0,status='removed',capability_id=NULL,
         last_used_at=NULL,updated_at=?2,removed_at=?2 WHERE artifact_id=?1",
    )
    .bind(artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    let event_id = stable_id("wae", &(artifact_id, "removed"))?;
    sqlx::query(
        "INSERT INTO artifact_events(event_id,artifact_id,event_type,action,created_at)
         VALUES(?1,?2,'removed',NULL,?3)",
    )
    .bind(event_id)
    .bind(artifact_id)
    .bind(&now)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(ReadyArtifactMutationResult {
        artifact_id: artifact_id.into(),
        revision: revision as u32 + 1,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{test_support, upsert_evidence};

    async fn setup() -> (tempfile::TempDir, SqlitePool) {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::open_insights_database(directory.path().join("insights.sqlite"))
            .await
            .unwrap();
        (directory, pool)
    }

    #[tokio::test]
    async fn keep_rolls_back_at_every_material_boundary() {
        for failpoint in [
            KeepFailpoint::AfterArtifact,
            KeepFailpoint::AfterVersion,
            KeepFailpoint::AfterDisposition,
        ] {
            let (_directory, pool) = setup().await;
            let finding_id = test_support::seed_findings(&pool, 1).await.remove(0);
            assert!(keep_finding_with_options(
                &pool,
                &finding_id,
                true,
                KeepOptions {
                    failpoint: Some(failpoint)
                }
            )
            .await
            .is_err());
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM dispositions")
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                sqlx::query_scalar::<_, i64>("SELECT active FROM findings WHERE finding_id=?1")
                    .bind(&finding_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn keep_is_idempotent_and_promotes_the_next_card() {
        let (_directory, pool) = setup().await;
        test_support::seed_findings(&pool, 3).await;
        let before = crate::worth_fixing_summary(&pool, true).await.unwrap();
        assert_eq!(before.selected.len(), 2);
        let kept_id = before.selected[0].finding_id.clone();
        let first = keep_finding(&pool, &kept_id, true).await.unwrap();
        let retry = keep_finding(&pool, &kept_id, true).await.unwrap();
        assert!(!first.already_kept);
        assert!(retry.already_kept);
        assert_eq!(first.artifact.artifact_id, retry.artifact.artifact_id);
        assert_eq!(first.summary.selected.len(), 2);
        assert!(!first
            .summary
            .selected
            .iter()
            .any(|item| item.finding_id == kept_id));
        for table in ["artifacts", "artifact_versions", "dispositions"] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                1
            );
        }
    }

    #[tokio::test]
    async fn concurrent_keep_attempts_converge() {
        let (_directory, pool) = setup().await;
        let finding_id = test_support::seed_findings(&pool, 1).await.remove(0);
        let (left, right) = tokio::join!(
            keep_finding(&pool, &finding_id, true),
            keep_finding(&pool, &finding_id, true)
        );
        let left = left.unwrap();
        let right = right.unwrap();
        assert_eq!(left.artifact.artifact_id, right.artifact.artifact_id);
        assert_ne!(left.already_kept, right.already_kept);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM artifacts")
                .fetch_one(&pool)
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn evidence_deletion_before_keep_blocks_but_after_keep_preserves_body() {
        let (_directory, pool) = setup().await;
        let finding_ids = test_support::seed_findings(&pool, 2).await;
        let mut first_evidence = test_support::evidence(1);
        first_evidence.deleted = true;
        upsert_evidence(&pool, &first_evidence).await.unwrap();
        assert!(keep_finding(&pool, &finding_ids[0], true).await.is_err());

        let kept = keep_finding(&pool, &finding_ids[1], true).await.unwrap();
        let original_body = ready_artifact_detail(&pool, &kept.artifact.artifact_id)
            .await
            .unwrap()
            .body;
        let mut second_evidence = test_support::evidence(2);
        second_evidence.deleted = true;
        upsert_evidence(&pool, &second_evidence).await.unwrap();
        let detail = ready_artifact_detail(&pool, &kept.artifact.artifact_id)
            .await
            .unwrap();
        assert_eq!(detail.body, original_body);
        assert!(!detail.provenance_available);
    }

    #[tokio::test]
    async fn use_receipt_updates_timestamp_and_remove_erases_content() {
        let (_directory, pool) = setup().await;
        let finding_id = test_support::seed_findings(&pool, 1).await.remove(0);
        let kept = keep_finding(&pool, &finding_id, true).await.unwrap();
        assert!(kept.artifact.last_used_at.is_none());
        let receipt =
            record_artifact_used(&pool, &kept.artifact.artifact_id, ReadyArtifactAction::Copy)
                .await
                .unwrap();
        assert!(!receipt.last_used_at.is_empty());
        remove_artifact(&pool, &kept.artifact.artifact_id)
            .await
            .unwrap();
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM artifact_versions WHERE artifact_id=?1"
            )
            .bind(&kept.artifact.artifact_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
        let row = sqlx::query("SELECT title,status FROM artifacts WHERE artifact_id=?1")
            .bind(&kept.artifact.artifact_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("title"), "");
        assert_eq!(row.get::<String, _>("status"), "removed");
    }

    #[tokio::test]
    async fn delete_everything_clears_artifacts_versions_jobs_and_change_text() {
        let (_directory, pool) = setup().await;
        let finding_id = test_support::seed_findings(&pool, 1).await.remove(0);
        let kept = keep_finding(&pool, &finding_id, true).await.unwrap();
        sqlx::query(
            "INSERT INTO artifact_change_jobs(
              job_id,artifact_id,base_version,request_text,input_fingerprint,status,input_json,
              prompt_hash,schema_hash,model,created_at,updated_at)
             VALUES('job-delete',?1,1,'private change text','delete-fp','pending','private packet',
              'prompt','schema','mock','2026-01-01','2026-01-01')",
        )
        .bind(&kept.artifact.artifact_id)
        .execute(&pool)
        .await
        .unwrap();
        crate::delete_all_insights_data(&pool).await.unwrap();
        for table in [
            "artifacts",
            "artifact_versions",
            "artifact_change_jobs",
            "artifact_change_attempts",
            "findings",
            "evidence",
        ] {
            assert_eq!(
                sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM {table}"))
                    .fetch_one(&pool)
                    .await
                    .unwrap(),
                0
            );
        }
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT last_observation_sequence FROM insights_cursor")
                .fetch_one(&pool)
                .await
                .unwrap(),
            0
        );
    }
}
