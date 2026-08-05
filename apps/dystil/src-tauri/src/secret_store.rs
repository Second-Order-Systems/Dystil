//! Minimal owned secret store replacing `dystil_secrets::SecretStore`.
//!
//! Stores key-value pairs in a `secrets` table in the capture SQLite database.
//! Since vault encryption is excluded from the Dystil product, values are
//! stored as base64-encoded bytes (no AES-GCM layer).

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
    SqlitePool,
};
use std::time::Duration;

pub struct DystilSecretStore {
    pool: SqlitePool,
}

impl DystilSecretStore {
    pub async fn new(pool: SqlitePool) -> Result<Self, String> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS secrets (
                key TEXT NOT NULL PRIMARY KEY,
                value TEXT NOT NULL
            )",
        )
        .execute(&pool)
        .await
        .map_err(|e| format!("failed to create secrets table: {e}"))?;
        Ok(Self { pool })
    }

    pub async fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM secrets WHERE key = ?")
            .bind(key)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        match row {
            Some((encoded,)) => BASE64
                .decode(&encoded)
                .map(Some)
                .map_err(|e| format!("base64 decode failed: {e}")),
            None => Ok(None),
        }
    }

    pub async fn set(&self, key: &str, value: &[u8]) -> Result<(), String> {
        let encoded = BASE64.encode(value);
        sqlx::query("INSERT INTO secrets (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
            .bind(key)
            .bind(encoded)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub async fn delete(&self, key: &str) -> Result<(), String> {
        sqlx::query("DELETE FROM secrets WHERE key = ?")
            .bind(key)
            .execute(&self.pool)
            .await
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

/// Open a `DystilSecretStore` backed by the capture database.
pub async fn open_secret_store() -> Result<DystilSecretStore, String> {
    let data_dir = crate::dystil_paths::data_dir();
    let db_path = data_dir.join("db.sqlite");
    let options = SqliteConnectOptions::new()
        .filename(&db_path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(Duration::from_secs(30));
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .map_err(|e| format!("failed to open db at {}: {}", db_path.display(), e))?;
    DystilSecretStore::new(pool).await
}
