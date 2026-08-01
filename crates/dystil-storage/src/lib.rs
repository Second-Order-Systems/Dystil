//! Dystil-owned SQLite bootstrap for capture data.
//!
//! The schema is intentionally limited to what Dystil writes or reads. Older
//! Dystil databases are opened in place; unknown legacy tables are left
//! untouched and ignored.

use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;

mod activity_overview;
mod activity_search;

pub use activity_overview::{
    count_activity_in_range, get_activity_overview_raw, ActivityHealthRaw, ActivityOverviewRaw,
    FrameObservation,
};
pub use activity_search::{
    get_activity_context, get_activity_range, get_activity_source, search_activity,
    search_activity_filtered, ActivityRangeQuery, ActivityRecord, ActivitySearchQuery,
    ActivitySearchRecord,
};

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

pub async fn open_capture_database(path: impl AsRef<Path>) -> Result<SqlitePool, StorageError> {
    if let Some(parent) = path.as_ref().parent() {
        std::fs::create_dir_all(parent).map_err(|error| sqlx::Error::Io(error.into()))?;
    }
    // sqlx does not create a database file unless this is requested explicitly.
    // A fresh Dystil data directory must be enough to start local capture.
    let options = SqliteConnectOptions::new()
        .filename(path.as_ref())
        .create_if_missing(true)
        // Capture has concurrent frame, UI-event, and redaction writers. WAL
        // lets readers proceed while one writer is active; the timeout absorbs
        // the short periods where SQLite must still serialize writers.
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await?;
    initialize_capture_schema(&pool).await?;
    Ok(pool)
}

/// Open an existing capture database for a sidecar that must never mutate
/// capture data. Used by the Dystil MCP server.
pub async fn open_capture_database_read_only(
    path: impl AsRef<Path>,
) -> Result<SqlitePool, StorageError> {
    let path = path.as_ref();
    if !path.is_file() {
        return Err(StorageError::Sqlx(sqlx::Error::Io(
            std::io::Error::new(std::io::ErrorKind::NotFound, "Dystil database not found").into(),
        )));
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .busy_timeout(Duration::from_secs(10));
    SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .map_err(Into::into)
}

/// Idempotent baseline for new databases and additive compatibility setup for
/// existing ones. It deliberately does not drop unknown legacy tables.
pub async fn initialize_capture_schema(pool: &SqlitePool) -> Result<(), StorageError> {
    let mut tx = pool.begin().await?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS frames (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            device_name TEXT NOT NULL DEFAULT '',
            snapshot_path TEXT NOT NULL DEFAULT '',
            app_name TEXT,
            window_name TEXT,
            browser_url TEXT,
            document_path TEXT,
            focused BOOLEAN,
            capture_trigger TEXT,
            frame_text TEXT,
            text_source TEXT,
            accessibility_tree_json TEXT,
            ax_capture_diagnostics_json TEXT,
            content_hash INTEGER,
            simhash INTEGER,
            elements_ref_frame_id INTEGER,
            accessibility_redacted_at INTEGER
        )",
    )
    .execute(&mut *tx)
    .await?;
    // Migrate existing databases: rename accessibility_text → frame_text,
    // drop the duplicate full_text column. Errors are ignored — if the column
    // was already renamed / dropped the query silently fails.
    let _ = sqlx::query("ALTER TABLE frames RENAME COLUMN accessibility_text TO frame_text")
        .execute(&mut *tx)
        .await;
    let _ = sqlx::query("ALTER TABLE frames DROP COLUMN full_text")
        .execute(&mut *tx)
        .await;
    // Existing installations predate per-frame accessibility diagnostics.
    // SQLite has no portable `ADD COLUMN IF NOT EXISTS`, so an already-added
    // column is intentionally treated as an idempotent no-op here.
    let _ = sqlx::query("ALTER TABLE frames ADD COLUMN ax_capture_diagnostics_json TEXT")
        .execute(&mut *tx)
        .await;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS elements (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            frame_id INTEGER NOT NULL,
            source TEXT NOT NULL,
            role TEXT NOT NULL,
            text TEXT,
            parent_id INTEGER,
            depth INTEGER NOT NULL,
            left_bound REAL,
            top_bound REAL,
            width_bound REAL,
            height_bound REAL,
            confidence REAL,
            sort_order INTEGER,
            properties TEXT,
            on_screen INTEGER
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ui_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp TEXT NOT NULL,
            session_id TEXT,
            relative_ms INTEGER NOT NULL DEFAULT 0,
            event_type TEXT NOT NULL,
            x REAL, y REAL, delta_x REAL, delta_y REAL,
            button TEXT, click_count INTEGER, key_code TEXT, modifiers TEXT,
            text_content TEXT, text_length INTEGER,
            app_name TEXT, app_pid INTEGER, window_title TEXT, browser_url TEXT,
            element_role TEXT, element_name TEXT, element_value TEXT,
            element_description TEXT, element_automation_id TEXT,
            element_bounds TEXT, frame_id INTEGER,
            redacted_at INTEGER
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS dystil_text_redaction_state (
            source_table TEXT NOT NULL,
            source_row_id INTEGER NOT NULL,
            surface TEXT NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL DEFAULT 0,
            backend TEXT,
            last_error TEXT,
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            PRIMARY KEY (source_table, source_row_id, surface)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_frames_timestamp ON frames(timestamp, id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_ui_events_timestamp ON ui_events(timestamp, id)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_elements_frame_order ON elements(frame_id, sort_order)",
    )
    .execute(&mut *tx)
    .await?;
    // A deliberately narrow projection of capture rows for retrieval. This is
    // separate from the raw tables so callers never need arbitrary SQL or an
    // accessibility-tree/screenshot interface.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS activity_search_documents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            source_type TEXT NOT NULL CHECK (source_type IN ('frame', 'event')),
            source_row_id INTEGER NOT NULL,
            timestamp TEXT NOT NULL,
            app_name TEXT,
            window_name TEXT,
            browser_url TEXT,
            text TEXT NOT NULL,
            UNIQUE(source_type, source_row_id)
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_activity_search_documents_time
         ON activity_search_documents(timestamp, id)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_activity_search_documents_source_time
         ON activity_search_documents(source_type, timestamp, id)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_activity_search_documents_app_time
         ON activity_search_documents(app_name, timestamp, id)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE VIRTUAL TABLE IF NOT EXISTS activity_search_fts USING fts5(
            text, app_name, window_name, browser_url,
            content='activity_search_documents', content_rowid='id',
            tokenize = 'unicode61 remove_diacritics 2'
        )",
    )
    .execute(&mut *tx)
    .await?;
    for statement in [
        "CREATE TRIGGER IF NOT EXISTS activity_search_documents_ai AFTER INSERT ON activity_search_documents BEGIN
            INSERT INTO activity_search_fts(rowid, text, app_name, window_name, browser_url)
            VALUES (new.id, new.text, new.app_name, new.window_name, new.browser_url);
         END",
        "CREATE TRIGGER IF NOT EXISTS activity_search_documents_ad AFTER DELETE ON activity_search_documents BEGIN
            INSERT INTO activity_search_fts(activity_search_fts, rowid, text, app_name, window_name, browser_url)
            VALUES ('delete', old.id, old.text, old.app_name, old.window_name, old.browser_url);
         END",
        "CREATE TRIGGER IF NOT EXISTS activity_search_documents_au AFTER UPDATE ON activity_search_documents BEGIN
            INSERT INTO activity_search_fts(activity_search_fts, rowid, text, app_name, window_name, browser_url)
            VALUES ('delete', old.id, old.text, old.app_name, old.window_name, old.browser_url);
            INSERT INTO activity_search_fts(rowid, text, app_name, window_name, browser_url)
            VALUES (new.id, new.text, new.app_name, new.window_name, new.browser_url);
         END",
        "CREATE TRIGGER IF NOT EXISTS frames_activity_search_ai AFTER INSERT ON frames BEGIN
            INSERT OR REPLACE INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
            SELECT 'frame', new.id, new.timestamp, new.app_name, new.window_name, new.browser_url, new.frame_text
            WHERE trim(coalesce(new.frame_text, '')) <> '';
         END",
        "CREATE TRIGGER IF NOT EXISTS frames_activity_search_au AFTER UPDATE ON frames BEGIN
            DELETE FROM activity_search_documents WHERE source_type = 'frame' AND source_row_id = new.id;
            INSERT INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
            SELECT 'frame', new.id, new.timestamp, new.app_name, new.window_name, new.browser_url, new.frame_text
            WHERE trim(coalesce(new.frame_text, '')) <> '';
         END",
        "CREATE TRIGGER IF NOT EXISTS frames_activity_search_ad AFTER DELETE ON frames BEGIN
            DELETE FROM activity_search_documents WHERE source_type = 'frame' AND source_row_id = old.id;
         END",
        "CREATE TRIGGER IF NOT EXISTS events_activity_search_ai AFTER INSERT ON ui_events BEGIN
            INSERT OR REPLACE INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
            SELECT 'event', new.id, new.timestamp, new.app_name, new.window_title, new.browser_url,
                trim(coalesce(new.text_content, '') || ' ' || coalesce(new.element_name, '') || ' ' || coalesce(new.element_value, '') || ' ' || coalesce(new.element_description, ''))
            WHERE trim(coalesce(new.text_content, '') || coalesce(new.element_name, '') || coalesce(new.element_value, '') || coalesce(new.element_description, '')) <> '';
         END",
        "CREATE TRIGGER IF NOT EXISTS events_activity_search_au AFTER UPDATE ON ui_events BEGIN
            DELETE FROM activity_search_documents WHERE source_type = 'event' AND source_row_id = new.id;
            INSERT INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
            SELECT 'event', new.id, new.timestamp, new.app_name, new.window_title, new.browser_url,
                trim(coalesce(new.text_content, '') || ' ' || coalesce(new.element_name, '') || ' ' || coalesce(new.element_value, '') || ' ' || coalesce(new.element_description, ''))
            WHERE trim(coalesce(new.text_content, '') || coalesce(new.element_name, '') || coalesce(new.element_value, '') || coalesce(new.element_description, '')) <> '';
         END",
        "CREATE TRIGGER IF NOT EXISTS events_activity_search_ad AFTER DELETE ON ui_events BEGIN
            DELETE FROM activity_search_documents WHERE source_type = 'event' AND source_row_id = old.id;
         END",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    // Backfill databases created before the search projection was introduced.
    sqlx::query(
        "INSERT OR IGNORE INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
         SELECT 'frame', id, timestamp, app_name, window_name, browser_url, frame_text
         FROM frames WHERE trim(coalesce(frame_text, '')) <> ''",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT OR IGNORE INTO activity_search_documents(source_type, source_row_id, timestamp, app_name, window_name, browser_url, text)
         SELECT 'event', id, timestamp, app_name, window_title, browser_url,
             trim(coalesce(text_content, '') || ' ' || coalesce(element_name, '') || ' ' || coalesce(element_value, '') || ' ' || coalesce(element_description, ''))
         FROM ui_events
         WHERE trim(coalesce(text_content, '') || coalesce(element_name, '') || coalesce(element_value, '') || coalesce(element_description, '')) <> ''",
    )
    .execute(&mut *tx)
    .await?;
    // Collaboration state is local UI/audit state. It intentionally contains
    // only exchanged derived answers, never raw accessibility evidence.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_mailbox_state (
            id INTEGER PRIMARY KEY CHECK (id = 1),
            cursor INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT OR IGNORE INTO agent_mailbox_state (id) VALUES (1)")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS agent_messages (
            message_id TEXT PRIMARY KEY,
            conversation_id TEXT NOT NULL,
            sequence_id INTEGER NOT NULL,
            peer_user_id TEXT NOT NULL,
            direction TEXT NOT NULL,
            kind TEXT NOT NULL,
            local_status TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_agent_messages_conversation
         ON agent_messages(conversation_id, sequence_id)",
    )
    .execute(&mut *tx)
    .await?;
    // Durable, device-local chat state. Stored answers and citations let a
    // reopened inquiry render without rerunning retrieval or inference.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_chat_sessions (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS local_chat_messages (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL REFERENCES local_chat_sessions(id) ON DELETE CASCADE,
            role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
            mode TEXT NOT NULL CHECK (mode IN ('local', 'team')),
            question TEXT,
            answer TEXT,
            status TEXT NOT NULL CHECK (status IN ('pending', 'complete', 'failed')),
            citations_json TEXT,
            provider TEXT,
            model TEXT,
            elapsed_ms INTEGER,
            error_code TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_local_chat_messages_session
         ON local_chat_messages(session_id, created_at, id)",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_local_chat_sessions_updated
         ON local_chat_sessions(updated_at DESC, id DESC)",
    )
    .execute(&mut *tx)
    .await?;
    // Credentials remain in the OS keyring; this table contains routing
    // metadata only.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS ai_presets (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            provider_kind TEXT NOT NULL CHECK (provider_kind IN ('codex', 'claude', 'openai_compatible', 'ollama')),
            endpoint TEXT,
            model TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 0 CHECK (active IN (0, 1)),
            validation_status TEXT NOT NULL DEFAULT 'unknown' CHECK (validation_status IN ('unknown', 'ready', 'error')),
            validation_message TEXT,
            validated_at TEXT,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_ai_presets_active
         ON ai_presets(active) WHERE active = 1",
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Stable per-install machine identifier used by Dystil cloud sync.
pub fn get_or_create_machine_id(data_dir: impl AsRef<Path>) -> Result<String, StorageError> {
    let path: PathBuf = data_dir.as_ref().join("machine-id");
    if let Ok(value) = std::fs::read_to_string(&path) {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.to_owned());
        }
    }
    let value = uuid::Uuid::new_v4().to_string();
    std::fs::create_dir_all(data_dir.as_ref()).map_err(|error| sqlx::Error::Io(error.into()))?;
    std::fs::write(&path, format!("{value}\n")).map_err(|error| sqlx::Error::Io(error.into()))?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::{
        get_activity_context, initialize_capture_schema, open_capture_database, search_activity,
    };
    use sqlx::sqlite::SqlitePoolOptions;
    use tempfile::tempdir;

    #[tokio::test]
    async fn opens_and_initializes_a_missing_database_file() {
        let directory = tempdir().unwrap();
        let database_path = directory.path().join("new").join("capture.sqlite");

        let pool = open_capture_database(&database_path).await.unwrap();

        assert!(database_path.is_file());
        let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        let frame_table_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'frames'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(frame_table_exists, 1);
        let active_preset_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM ai_presets WHERE active = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(active_preset_count, 0);
    }

    #[tokio::test]
    async fn schema_is_idempotent_and_preserves_unknown_tables() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::query("CREATE TABLE legacy_unused (id INTEGER PRIMARY KEY, payload TEXT)")
            .execute(&pool)
            .await
            .unwrap();
        initialize_capture_schema(&pool).await.unwrap();
        initialize_capture_schema(&pool).await.unwrap();
        let _: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='legacy_unused'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn indexes_activity_and_keeps_context_bounded() {
        let directory = tempdir().unwrap();
        let pool = open_capture_database(directory.path().join("capture.sqlite"))
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO frames(timestamp, app_name, window_name, frame_text)
             VALUES ('2026-07-17T09:00:00Z', 'VS Code', 'Dystil', 'Implemented bounded retrieval ledger')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO ui_events(timestamp, event_type, text_content, app_name, window_title)
             VALUES ('2026-07-17T09:01:00Z', 'text', 'Reviewed retrieval tests', 'VS Code', 'Dystil')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let matches = search_activity(&pool, "bounded retrieval", 10)
            .await
            .unwrap();
        assert!(matches.iter().any(|record| record.source_id == "frame:1"));
        let context = get_activity_context(&pool, "frame:1", 60, 60, 10)
            .await
            .unwrap();
        assert_eq!(context.len(), 2);
        assert!(context
            .iter()
            .all(|record| !record.text.contains("accessibility_tree")));
    }
}
