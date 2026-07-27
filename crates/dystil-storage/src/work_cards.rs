use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

use crate::StorageError;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NewWorkCard {
    pub window_id: String,
    pub start_time: String,
    pub end_time: String,
    pub close_reason: String,
    pub title: String,
    pub summary: String,
    pub applications: Vec<String>,
    pub artifacts: serde_json::Value,
    pub actions: serde_json::Value,
    pub last_observed_state: String,
    pub status: String,
    pub uncertainties: Vec<String>,
    pub card_json: serde_json::Value,
    pub model_id: String,
    pub source_hash: String,
    pub embedding_model_id: Option<String>,
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoredWorkCard {
    pub window_id: String,
    pub start_time: String,
    pub end_time: String,
    pub close_reason: String,
    pub title: String,
    pub summary: String,
    pub applications: Vec<String>,
    pub artifacts: serde_json::Value,
    pub actions: serde_json::Value,
    pub last_observed_state: String,
    pub status: String,
    pub uncertainties: Vec<String>,
    pub card_json: serde_json::Value,
    pub model_id: String,
    pub source_hash: String,
    pub embedding_model_id: Option<String>,
    pub embedding_dimensions: Option<u32>,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn upsert_work_card(pool: &SqlitePool, card: &NewWorkCard) -> Result<(), StorageError> {
    let applications_json = serde_json::to_string(&card.applications)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    let artifacts_json = serde_json::to_string(&card.artifacts)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    let actions_json = serde_json::to_string(&card.actions)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    let uncertainties_json = serde_json::to_string(&card.uncertainties)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    let card_json = serde_json::to_string(&card.card_json)
        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
    let embedding = card.embedding.as_deref().map(encode_embedding);
    let embedding_dimensions = card.embedding.as_ref().map(|values| values.len() as i64);
    let mut tx = pool.begin().await?;

    sqlx::query(
        "INSERT INTO work_cards (
            window_id,start_time,end_time,close_reason,title,summary,
            applications_json,artifacts_json,actions_json,last_observed_state,
            status,uncertainties_json,card_json,model_id,embedding_model_id,
            embedding_dimensions,embedding,source_hash
         ) VALUES (
            ?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18
         )
         ON CONFLICT(window_id) DO UPDATE SET
            start_time=excluded.start_time,
            end_time=excluded.end_time,
            close_reason=excluded.close_reason,
            title=excluded.title,
            summary=excluded.summary,
            applications_json=excluded.applications_json,
            artifacts_json=excluded.artifacts_json,
            actions_json=excluded.actions_json,
            last_observed_state=excluded.last_observed_state,
            status=excluded.status,
            uncertainties_json=excluded.uncertainties_json,
            card_json=excluded.card_json,
            model_id=excluded.model_id,
            embedding_model_id=excluded.embedding_model_id,
            embedding_dimensions=excluded.embedding_dimensions,
            embedding=excluded.embedding,
            source_hash=excluded.source_hash,
            updated_at=datetime('now')",
    )
    .bind(&card.window_id)
    .bind(&card.start_time)
    .bind(&card.end_time)
    .bind(&card.close_reason)
    .bind(&card.title)
    .bind(&card.summary)
    .bind(&applications_json)
    .bind(&artifacts_json)
    .bind(&actions_json)
    .bind(&card.last_observed_state)
    .bind(&card.status)
    .bind(&uncertainties_json)
    .bind(&card_json)
    .bind(&card.model_id)
    .bind(&card.embedding_model_id)
    .bind(embedding_dimensions)
    .bind(embedding)
    .bind(&card.source_hash)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM work_cards_fts WHERE window_id = ?1")
        .bind(&card.window_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO work_cards_fts (
            window_id,title,summary,applications,artifacts,actions,last_observed_state
         ) VALUES (?1,?2,?3,?4,?5,?6,?7)",
    )
    .bind(&card.window_id)
    .bind(&card.title)
    .bind(&card.summary)
    .bind(card.applications.join(" "))
    .bind(searchable_json(&card.artifacts))
    .bind(searchable_json(&card.actions))
    .bind(&card.last_observed_state)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn list_work_cards(
    pool: &SqlitePool,
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    let rows = sqlx::query("SELECT * FROM work_cards ORDER BY start_time DESC, window_id LIMIT ?1")
        .bind(limit.clamp(1, 200) as i64)
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_card).collect()
}

/// List cards whose observed interval overlaps the requested UTC range.
/// Results are chronological so callers can safely construct a timeline.
pub async fn list_work_cards_range(
    pool: &SqlitePool,
    start_time: &str,
    end_time: &str,
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    let rows = sqlx::query(
        "SELECT * FROM work_cards
         -- SQLite's datetime() normalizes RFC3339 offsets. The agent worker
         -- supplies UTC boundaries while persisted cards retain local offsets,
         -- so lexical TEXT comparison would be wrong near timezone boundaries.
         WHERE datetime(end_time) > datetime(?1) AND datetime(start_time) < datetime(?2)
         ORDER BY start_time ASC, window_id ASC
         LIMIT ?3",
    )
    .bind(start_time)
    .bind(end_time)
    .bind(limit.clamp(1, 500) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_card).collect()
}

/// Return exactly one derived card. This intentionally does not expose the
/// raw evidence tables to callers such as the MCP sidecar.
pub async fn get_work_card(
    pool: &SqlitePool,
    window_id: &str,
) -> Result<Option<StoredWorkCard>, StorageError> {
    let row = sqlx::query("SELECT * FROM work_cards WHERE window_id = ?1")
        .bind(window_id)
        .fetch_optional(pool)
        .await?;
    row.as_ref().map(row_to_card).transpose()
}

pub async fn search_work_cards(
    pool: &SqlitePool,
    query: &str,
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    let Some(expression) = fts_expression(query) else {
        return list_work_cards(pool, limit).await;
    };
    let rows = sqlx::query(
        "SELECT work_cards.*
         FROM work_cards_fts
         JOIN work_cards ON work_cards.window_id = work_cards_fts.window_id
         WHERE work_cards_fts MATCH ?1
         ORDER BY bm25(work_cards_fts, 0.0, 4.0, 2.0, 1.0, 2.0, 1.5, 2.0),
                  work_cards.start_time DESC
         LIMIT ?2",
    )
    .bind(expression)
    .bind(limit.clamp(1, 100) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_card).collect()
}

/// Search only cards that overlap the requested ISO-8601 interval. This avoids
/// old lexical matches crowding out relevant cards in teammate-agent requests.
pub async fn search_work_cards_range(
    pool: &SqlitePool,
    query: &str,
    start_time: &str,
    end_time: &str,
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    let Some(expression) = fts_expression(query) else {
        return list_work_cards_range(pool, start_time, end_time, limit).await;
    };
    let rows = sqlx::query(
        "SELECT work_cards.*
         FROM work_cards_fts
         JOIN work_cards ON work_cards.window_id = work_cards_fts.window_id
         WHERE work_cards_fts MATCH ?1
           AND datetime(work_cards.end_time) > datetime(?2)
           AND datetime(work_cards.start_time) < datetime(?3)
         ORDER BY bm25(work_cards_fts, 0.0, 4.0, 2.0, 1.0, 2.0, 1.5, 2.0),
                  work_cards.start_time DESC
         LIMIT ?4",
    )
    .bind(expression)
    .bind(start_time)
    .bind(end_time)
    .bind(limit.clamp(1, 100) as i64)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_card).collect()
}

pub async fn hybrid_search_work_cards(
    pool: &SqlitePool,
    query: &str,
    query_embedding: &[f32],
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    if query_embedding.is_empty() {
        return search_work_cards(pool, query, limit).await;
    }
    let lexical = search_work_cards(pool, query, 100).await?;
    let rows = sqlx::query(
        "SELECT * FROM work_cards
         WHERE embedding IS NOT NULL AND embedding_dimensions = ?1
         ORDER BY start_time DESC
         LIMIT 20000",
    )
    .bind(query_embedding.len() as i64)
    .fetch_all(pool)
    .await?;
    let mut dense = rows
        .iter()
        .filter_map(|row| {
            let blob = row.try_get::<Vec<u8>, _>("embedding").ok()?;
            let embedding = decode_embedding(&blob)?;
            let score = cosine(query_embedding, &embedding);
            row_to_card(row).ok().map(|card| (card, score))
        })
        .collect::<Vec<_>>();
    dense.sort_by(|left, right| right.1.total_cmp(&left.1));

    let lexical_first = lexical.first().map(|card| card.window_id.clone());
    let mut scores = HashMap::<String, f32>::new();
    let mut cards = HashMap::<String, StoredWorkCard>::new();
    for (rank, card) in lexical.into_iter().enumerate() {
        *scores.entry(card.window_id.clone()).or_default() += 1.5 / (60.0 + rank as f32 + 1.0);
        cards.insert(card.window_id.clone(), card);
    }
    for (rank, (card, _)) in dense.into_iter().take(100).enumerate() {
        *scores.entry(card.window_id.clone()).or_default() += 1.0 / (60.0 + rank as f32 + 1.0);
        cards.insert(card.window_id.clone(), card);
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    if let Some(lexical_first) = lexical_first {
        if let Some(index) = ranked
            .iter()
            .position(|(window_id, _)| window_id == &lexical_first)
        {
            let first = ranked.remove(index);
            ranked.insert(0, first);
        }
    }
    Ok(ranked
        .into_iter()
        .take(limit.clamp(1, 100) as usize)
        .filter_map(|(window_id, _)| cards.remove(&window_id))
        .collect())
}

/// Range-bounded hybrid search for agent requests. The dense candidate query
/// applies the same interval predicate as FTS before ranking.
pub async fn hybrid_search_work_cards_range(
    pool: &SqlitePool,
    query: &str,
    query_embedding: &[f32],
    start_time: &str,
    end_time: &str,
    limit: u32,
) -> Result<Vec<StoredWorkCard>, StorageError> {
    if query_embedding.is_empty() {
        return search_work_cards_range(pool, query, start_time, end_time, limit).await;
    }
    let lexical = search_work_cards_range(pool, query, start_time, end_time, 100).await?;
    let rows = sqlx::query(
        "SELECT * FROM work_cards
         WHERE embedding IS NOT NULL AND embedding_dimensions = ?1
           AND datetime(end_time) > datetime(?2) AND datetime(start_time) < datetime(?3)
         ORDER BY start_time DESC
         LIMIT 20000",
    )
    .bind(query_embedding.len() as i64)
    .bind(start_time)
    .bind(end_time)
    .fetch_all(pool)
    .await?;
    let mut dense = rows
        .iter()
        .filter_map(|row| {
            let blob = row.try_get::<Vec<u8>, _>("embedding").ok()?;
            let embedding = decode_embedding(&blob)?;
            let score = cosine(query_embedding, &embedding);
            row_to_card(row).ok().map(|card| (card, score))
        })
        .collect::<Vec<_>>();
    dense.sort_by(|left, right| right.1.total_cmp(&left.1));
    let lexical_first = lexical.first().map(|card| card.window_id.clone());
    let mut scores = HashMap::<String, f32>::new();
    let mut cards = HashMap::<String, StoredWorkCard>::new();
    for (rank, card) in lexical.into_iter().enumerate() {
        *scores.entry(card.window_id.clone()).or_default() += 1.5 / (60.0 + rank as f32 + 1.0);
        cards.insert(card.window_id.clone(), card);
    }
    for (rank, (card, _)) in dense.into_iter().take(100).enumerate() {
        *scores.entry(card.window_id.clone()).or_default() += 1.0 / (60.0 + rank as f32 + 1.0);
        cards.insert(card.window_id.clone(), card);
    }
    let mut ranked = scores.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    if let Some(lexical_first) = lexical_first {
        if let Some(index) = ranked.iter().position(|(id, _)| id == &lexical_first) {
            let first = ranked.remove(index);
            ranked.insert(0, first);
        }
    }
    Ok(ranked
        .into_iter()
        .take(limit.clamp(1, 100) as usize)
        .filter_map(|(id, _)| cards.remove(&id))
        .collect())
}

pub async fn delete_work_card(pool: &SqlitePool, window_id: &str) -> Result<(), StorageError> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM work_cards_fts WHERE window_id = ?1")
        .bind(window_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM work_cards WHERE window_id = ?1")
        .bind(window_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

fn row_to_card(row: &sqlx::sqlite::SqliteRow) -> Result<StoredWorkCard, StorageError> {
    Ok(StoredWorkCard {
        window_id: row.try_get("window_id")?,
        start_time: row.try_get("start_time")?,
        end_time: row.try_get("end_time")?,
        close_reason: row.try_get("close_reason")?,
        title: row.try_get("title")?,
        summary: row.try_get("summary")?,
        applications: decode_json(row.try_get("applications_json")?)?,
        artifacts: decode_json(row.try_get("artifacts_json")?)?,
        actions: decode_json(row.try_get("actions_json")?)?,
        last_observed_state: row.try_get("last_observed_state")?,
        status: row.try_get("status")?,
        uncertainties: decode_json(row.try_get("uncertainties_json")?)?,
        card_json: decode_json(row.try_get("card_json")?)?,
        model_id: row.try_get("model_id")?,
        source_hash: row.try_get("source_hash")?,
        embedding_model_id: row.try_get("embedding_model_id")?,
        embedding_dimensions: row
            .try_get::<Option<i64>, _>("embedding_dimensions")?
            .map(|value| value as u32),
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn decode_json<T: serde::de::DeserializeOwned>(value: String) -> Result<T, StorageError> {
    serde_json::from_str(&value)
        .map_err(|error| StorageError::Sqlx(sqlx::Error::Decode(Box::new(error))))
}

fn searchable_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(value) => value.clone(),
        serde_json::Value::Array(values) => values
            .iter()
            .map(searchable_json)
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Object(values) => values
            .values()
            .map(searchable_json)
            .collect::<Vec<_>>()
            .join(" "),
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
            value.to_string()
        }
    }
}

fn fts_expression(query: &str) -> Option<String> {
    let terms = query
        .split(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .filter(|value| value.chars().count() >= 2)
        .take(24)
        .map(|value| format!("\"{}\"", value.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    (!terms.is_empty()).then(|| terms.join(" OR "))
}

fn encode_embedding(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding(bytes: &[u8]) -> Option<Vec<f32>> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte chunk")))
            .collect(),
    )
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    let dot = left.iter().zip(right).map(|(a, b)| a * b).sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::open_capture_database;
    use tempfile::tempdir;

    fn card(window_id: &str, title: &str, summary: &str) -> NewWorkCard {
        NewWorkCard {
            window_id: window_id.into(),
            start_time: "2026-07-17T09:00:00+05:30".into(),
            end_time: "2026-07-17T09:15:00+05:30".into(),
            close_reason: "max_duration".into(),
            title: title.into(),
            summary: summary.into(),
            applications: vec!["DBeaver".into()],
            artifacts: serde_json::json!([{"kind":"file","value":"query.sql"}]),
            actions: serde_json::json!([{"text":"Reviewed a slow query"}]),
            last_observed_state: "Query editor remained open".into(),
            status: "in_progress".into(),
            uncertainties: vec![],
            card_json: serde_json::json!({"title": title}),
            model_id: "test-model".into(),
            source_hash: "sha256:test".into(),
            embedding_model_id: Some("embedder".into()),
            embedding: Some(vec![0.25, -0.5]),
        }
    }

    #[tokio::test]
    async fn upserts_lists_and_searches_work_cards_with_fts5() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        upsert_work_card(
            &pool,
            &card(
                "win_1",
                "Optimized a slow SQL query",
                "Reduced query latency",
            ),
        )
        .await
        .unwrap();
        upsert_work_card(
            &pool,
            &card("win_2", "Reviewed Slack discussion", "Discussed deployment"),
        )
        .await
        .unwrap();

        let results = search_work_cards(&pool, "database latency", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].window_id, "win_1");
        assert_eq!(results[0].embedding_dimensions, Some(2));

        let mut replacement = card("win_1", "Fixed database timeout", "Query now succeeds");
        replacement.embedding = None;
        replacement.embedding_model_id = None;
        upsert_work_card(&pool, &replacement).await.unwrap();
        assert!(search_work_cards(&pool, "optimized", 10)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(list_work_cards(&pool, 10).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn hybrid_search_combines_dense_and_lexical_ranks() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        let mut database = card("database", "SQL query", "Reviewed query performance");
        database.embedding = Some(vec![1.0, 0.0]);
        let mut chat = card("chat", "Slack discussion", "Discussed a deployment");
        chat.embedding = Some(vec![0.0, 1.0]);
        upsert_work_card(&pool, &database).await.unwrap();
        upsert_work_card(&pool, &chat).await.unwrap();

        let results = hybrid_search_work_cards(&pool, "performance", &[1.0, 0.0], 2)
            .await
            .unwrap();
        assert_eq!(results[0].window_id, "database");
    }

    #[tokio::test]
    async fn lists_overlapping_cards_in_chronological_order() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        let mut later = card("later", "Later", "later summary");
        later.start_time = "2026-07-17T11:00:00+05:30".into();
        later.end_time = "2026-07-17T11:15:00+05:30".into();
        let mut earlier = card("earlier", "Earlier", "earlier summary");
        earlier.start_time = "2026-07-17T10:00:00+05:30".into();
        earlier.end_time = "2026-07-17T10:15:00+05:30".into();
        upsert_work_card(&pool, &later).await.unwrap();
        upsert_work_card(&pool, &earlier).await.unwrap();

        let cards = list_work_cards_range(
            &pool,
            "2026-07-17T09:30:00+05:30",
            "2026-07-17T12:00:00+05:30",
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            cards.iter().map(|card| &card.window_id).collect::<Vec<_>>(),
            ["earlier", "later"]
        );
        assert_eq!(
            get_work_card(&pool, "later").await.unwrap().unwrap().title,
            "Later"
        );
    }

    #[tokio::test]
    async fn range_search_excludes_old_high_ranked_cards() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        let mut old = card("old", "Redis counter", "Redis counter investigation");
        old.start_time = "2026-06-01T09:00:00+05:30".into();
        old.end_time = "2026-06-01T09:15:00+05:30".into();
        let mut current = card("current", "Redis counter", "Redis counter investigation");
        current.start_time = "2026-07-17T09:00:00+05:30".into();
        current.end_time = "2026-07-17T09:15:00+05:30".into();
        upsert_work_card(&pool, &old).await.unwrap();
        upsert_work_card(&pool, &current).await.unwrap();
        let results = search_work_cards_range(
            &pool,
            "Redis counter",
            "2026-07-01T00:00:00+05:30",
            "2026-08-01T00:00:00+05:30",
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            results
                .iter()
                .map(|card| card.window_id.as_str())
                .collect::<Vec<_>>(),
            ["current"]
        );
    }

    #[tokio::test]
    async fn range_search_normalizes_timezone_offsets() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        let mut card = card("offset", "Redis counter", "Redis counter investigation");
        // This card is 18:40–18:50 UTC on July 16 even though its stored local
        // date is July 17. A lexical RFC3339 comparison would wrongly exclude it.
        card.start_time = "2026-07-17T00:10:00+05:30".into();
        card.end_time = "2026-07-17T00:20:00+05:30".into();
        upsert_work_card(&pool, &card).await.unwrap();

        let results = search_work_cards_range(
            &pool,
            "Redis counter",
            "2026-07-16T18:45:00+00:00",
            "2026-07-16T19:00:00+00:00",
            10,
        )
        .await
        .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].window_id, "offset");
    }
}
