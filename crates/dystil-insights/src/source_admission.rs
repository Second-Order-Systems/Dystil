use sqlx::{Row, SqlitePool};

use crate::{EvidenceRecord, InsightsError, Result};

#[derive(Debug, Clone, Default)]
pub struct CaptureAdmissionRules {
    /// Case-insensitive app fragments from the active capture exclusion policy.
    pub excluded_apps: Vec<String>,
    /// Case-insensitive title fragments from the active capture exclusion policy.
    pub excluded_windows: Vec<String>,
    pub excluded_urls: Vec<String>,
    pub ignore_private_windows: bool,
}

impl CaptureAdmissionRules {
    fn policy_allows(&self, app: Option<&str>, window: Option<&str>, url: Option<&str>) -> bool {
        let app = app.unwrap_or_default().to_lowercase();
        let window = window.unwrap_or_default().to_lowercase();
        let url = url.unwrap_or_default().to_lowercase();
        if self
            .excluded_apps
            .iter()
            .any(|value| app.contains(&value.to_lowercase()))
            || self
                .excluded_windows
                .iter()
                .any(|value| window.contains(&value.to_lowercase()))
            || self
                .excluded_urls
                .iter()
                .any(|value| url.contains(&value.to_lowercase()))
        {
            return false;
        }
        !self.ignore_private_windows
            || !["incognito", "private browsing", "inprivate"]
                .iter()
                .any(|value| window.contains(value))
    }
}

/// Resolves a live capture row into the only evidence shape accepted by the
/// insights engine. A missing row returns `None` and must be treated as a
/// deletion by callers holding an older reference.
pub async fn resolve_capture_evidence(
    capture: &SqlitePool,
    source_namespace: &str,
    source_id: &str,
    rules: &CaptureAdmissionRules,
) -> Result<Option<EvidenceRecord>> {
    let (kind, raw_id) = source_id
        .split_once(':')
        .ok_or_else(|| InsightsError::Invalid("capture source ID is not namespaced".into()))?;
    let row_id = raw_id
        .parse::<i64>()
        .map_err(|_| InsightsError::Invalid("capture source ID has an invalid row ID".into()))?;
    match kind {
        "frame" => {
            let row = sqlx::query(
                "SELECT timestamp,app_name,window_name,browser_url,frame_text FROM frames WHERE id=?1",
            )
            .bind(row_id)
            .fetch_optional(capture)
            .await?;
            let Some(row) = row else { return Ok(None) };
            let app = row.get::<Option<String>, _>("app_name");
            let window = row.get::<Option<String>, _>("window_name");
            let url = row.get::<Option<String>, _>("browser_url");
            let excerpt = row
                .get::<Option<String>, _>("frame_text")
                .unwrap_or_default();
            let redaction_ready = sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM dystil_text_redaction_state WHERE source_table='frames'
                 AND source_row_id=?1 AND surface='frame_text'
                 AND status IN ('complete','deterministic_fallback')",
            )
            .bind(row_id)
            .fetch_one(capture)
            .await?
                > 0;
            Ok(Some(EvidenceRecord {
                evidence_id: format!("{source_namespace}:frame:{row_id}"),
                source_namespace: source_namespace.into(),
                source_id: source_id.into(),
                occurred_at: row.get("timestamp"),
                app: app.clone(),
                window: window.clone(),
                excerpt,
                policy_allowed: rules.policy_allows(
                    app.as_deref(),
                    window.as_deref(),
                    url.as_deref(),
                ),
                redaction_ready,
                deleted: false,
                sensitive: false,
            }))
        }
        "event" => {
            let row = sqlx::query(
                "SELECT timestamp,app_name,window_title,browser_url,
                 trim(coalesce(text_content,'') || ' ' || coalesce(element_name,'') || ' ' || coalesce(element_value,'')) excerpt,
                 redacted_at FROM ui_events WHERE id=?1",
            ).bind(row_id).fetch_optional(capture).await?;
            let Some(row) = row else { return Ok(None) };
            let app = row.get::<Option<String>, _>("app_name");
            let window = row.get::<Option<String>, _>("window_title");
            let url = row.get::<Option<String>, _>("browser_url");
            Ok(Some(EvidenceRecord {
                evidence_id: format!("{source_namespace}:event:{row_id}"),
                source_namespace: source_namespace.into(),
                source_id: source_id.into(),
                occurred_at: row.get("timestamp"),
                app: app.clone(),
                window: window.clone(),
                excerpt: row.get("excerpt"),
                policy_allowed: rules.policy_allows(
                    app.as_deref(),
                    window.as_deref(),
                    url.as_deref(),
                ),
                // UI events are deterministically sanitized before insertion;
                // `redacted_at` is a legacy strengthening marker, not the safe
                // admission boundary used by the current capture writer.
                redaction_ready: true,
                deleted: false,
                sensitive: false,
            }))
        }
        _ => Err(InsightsError::Invalid(format!(
            "unsupported capture source {kind}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use sqlx::sqlite::SqlitePoolOptions;

    use super::*;

    #[tokio::test]
    async fn live_resolver_enforces_redaction_exclusion_private_and_deletion() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE frames(id INTEGER PRIMARY KEY,timestamp TEXT,app_name TEXT,window_name TEXT,browser_url TEXT,frame_text TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE dystil_text_redaction_state(source_table TEXT,source_row_id INTEGER,surface TEXT,status TEXT)").execute(&pool).await.unwrap();
        sqlx::query("CREATE TABLE ui_events(id INTEGER PRIMARY KEY,timestamp TEXT,app_name TEXT,window_title TEXT,browser_url TEXT,text_content TEXT,element_name TEXT,element_value TEXT,redacted_at INTEGER)").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO frames VALUES(1,'2026-08-02T10:00:00Z','Browser','Private Browsing',NULL,'safe text')").execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO dystil_text_redaction_state VALUES('frames',1,'frame_text','complete')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let rules = CaptureAdmissionRules {
            ignore_private_windows: true,
            ..Default::default()
        };
        let private = resolve_capture_evidence(&pool, "device:test", "frame:1", &rules)
            .await
            .unwrap()
            .unwrap();
        assert!(!private.policy_allowed);
        assert!(private.redaction_ready);
        assert!(
            resolve_capture_evidence(&pool, "device:test", "frame:999", &rules)
                .await
                .unwrap()
                .is_none()
        );
    }
}
