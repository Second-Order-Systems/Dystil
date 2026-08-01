use dystil_retrieval::{
    ContextRequest, DataStatus, EvidenceId, OverviewRequest, RangeRequest, RetrievalService,
    SearchRequest,
};
use dystil_storage::open_capture_database;
use tempfile::tempdir;

async fn seeded() -> (tempfile::TempDir, RetrievalService) {
    let directory = tempdir().unwrap();
    let pool = open_capture_database(directory.path().join("db.sqlite"))
        .await
        .unwrap();
    for (timestamp, app, window, url, text) in [
        (
            "2026-07-31T09:00:00Z",
            "Slack",
            "team-dystil",
            None,
            "Rahul: I updated DYS-142 with the authentication rollout details",
        ),
        (
            "2026-07-31T09:01:00Z",
            "Jira",
            "DYS-142 authentication",
            Some("https://jira.example/browse/DYS-142"),
            "DYS-142 authentication rollout owner Rahul status in progress",
        ),
        (
            "2026-07-31T09:02:00Z",
            "Cursor",
            "auth.rs — dystil",
            None,
            "Fixed token validation error in auth.rs for the rollout",
        ),
        (
            "2026-07-31T09:12:00Z",
            "Slack",
            "team-dystil",
            None,
            "Rahul: I updated DYS-142 with the authentication rollout details",
        ),
    ] {
        sqlx::query(
            "INSERT INTO frames(timestamp, app_name, window_name, browser_url, frame_text)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(timestamp)
        .bind(app)
        .bind(window)
        .bind(url)
        .bind(text)
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::query(
        "INSERT INTO ui_events(timestamp, event_type, text_content, app_name, window_title)
         VALUES ('2026-07-31T09:01:30Z', 'click', 'Opened DYS-142', 'Jira', 'DYS-142 authentication')",
    )
    .execute(&pool)
    .await
    .unwrap();
    (directory, RetrievalService::new(pool))
}

#[tokio::test]
async fn search_returns_stable_bounded_deduplicated_evidence() {
    let (_directory, retrieval) = seeded().await;
    let page = retrieval
        .search(SearchRequest {
            query: "DYS-142".into(),
            limit: Some(10),
            max_snippet_chars: Some(160),
            ..Default::default()
        })
        .await
        .unwrap();
    assert!(page.records.len() >= 2);
    assert!(page.records.iter().all(|record| {
        record.evidence_id.to_string().contains(':')
            && record.deep_link.starts_with("dystil://evidence/")
            && record.text.chars().count() <= 160
    }));
    assert_eq!(
        page.records
            .iter()
            .filter(|record| record.app_name.as_deref() == Some("Slack"))
            .count(),
        1
    );
}

#[tokio::test]
async fn source_context_and_range_expand_search_without_unbounded_payloads() {
    let (_directory, retrieval) = seeded().await;
    let source = retrieval
        .get_source(&"frame:2".parse::<EvidenceId>().unwrap(), Some(500))
        .await
        .unwrap();
    assert!(source.text.contains("authentication rollout"));

    let context = retrieval
        .context(ContextRequest {
            evidence_id: "frame:2".parse().unwrap(),
            before_seconds: Some(90),
            after_seconds: Some(90),
            limit: Some(10),
            max_content_chars: Some(500),
        })
        .await
        .unwrap();
    assert!(context
        .records
        .iter()
        .any(|record| record.source_type == "event"));
    assert!(context
        .records
        .iter()
        .any(|record| record.app_name.as_deref() == Some("Slack")));

    let range = retrieval
        .range(RangeRequest {
            start_time: "2026-07-31T09:00:00Z".into(),
            end_time: "2026-07-31T09:03:00Z".into(),
            app_name: Some("Cursor".into()),
            limit: Some(5),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(range.records.len(), 1);
    assert_eq!(range.records[0].app_name.as_deref(), Some("Cursor"));
}

#[tokio::test]
async fn overview_is_deterministic_and_diagnoses_coverage_and_filters() {
    let (_directory, retrieval) = seeded().await;
    let overview = retrieval
        .overview(OverviewRequest {
            start_time: "2026-07-31T09:00:00Z".into(),
            end_time: "2026-07-31T09:15:00Z".into(),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(overview.data_status, DataStatus::SparseCoverage);
    assert_eq!(overview.frame_count, 4);
    assert_eq!(overview.event_count, 1);
    assert_eq!(overview.estimated_active_minutes, 2.0);
    assert!(overview
        .transitions
        .iter()
        .any(|transition| { transition.from == "Slack" && transition.to == "Jira" }));
    assert!(overview.health.fts_consistent);

    let filtered = retrieval
        .overview(OverviewRequest {
            start_time: "2026-07-31T09:00:00Z".into(),
            end_time: "2026-07-31T09:15:00Z".into(),
            app_name: Some("Notion".into()),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(filtered.data_status, DataStatus::FiltersTooRestrictive);
}
