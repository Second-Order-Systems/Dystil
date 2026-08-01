//! Provider-neutral automation definitions, persistence, and execution primitives.

use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use thiserror::Error;
use tokio::sync::mpsc;
use uuid::Uuid;

pub const MEMORY_PROMPT: &str = r#"## Continuous improvement (memory)
Before doing anything else, read `./memory.md` if it exists and apply its lessons. If it is missing, create it with a `# memory` heading followed by a `## Lessons` heading.

After the run, append at most 1–3 new dated one-line lessons under `## Lessons`, only when the run taught you something durable and reusable. If nothing durable was learned, write nothing.

Keep memory healthy: treat it as append-only except for dated retractions; keep it near 150 lines or 8 KB by merging duplicates and dropping old low-value agent lessons; never remove user notes; save observations and rules rather than tasks; never edit this automation prompt."#;

#[derive(Debug, Error)]
pub enum AutomationError {
    #[error("invalid automation: {0}")]
    Invalid(String),
    #[error("automation not found: {0}")]
    NotFound(String),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("runner error: {0}")]
    Runner(String),
}

pub type Result<T> = std::result::Result<T, AutomationError>;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AutomationDocument {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub trigger: Trigger,
    #[serde(default)]
    pub preset: Option<String>,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub retry: RetryPolicy,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_yaml::Value>,
    #[serde(skip)]
    pub body: String,
    #[serde(skip)]
    pub directory: PathBuf,
}

fn default_timeout() -> u64 {
    300
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Trigger {
    #[default]
    Manual,
    Schedule {
        schedule: String,
    },
    Event {
        source: String,
        #[serde(default)]
        filter: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RetryPolicy {
    #[serde(default = "default_attempts")]
    pub max_attempts: u32,
    #[serde(default = "default_backoff")]
    pub backoff_seconds: u64,
}

fn default_attempts() -> u32 {
    1
}
fn default_backoff() -> u64 {
    5
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: default_attempts(),
            backoff_seconds: default_backoff(),
        }
    }
}

impl AutomationDocument {
    pub fn parse(path: &Path, source: &str) -> Result<Self> {
        let source = source.trim_start_matches('\u{feff}');
        let rest = source
            .strip_prefix("---\n")
            .or_else(|| source.strip_prefix("---\r\n"))
            .ok_or_else(|| {
                AutomationError::Invalid("automation.md must start with YAML frontmatter".into())
            })?;
        let marker = rest
            .find("\n---")
            .ok_or_else(|| AutomationError::Invalid("frontmatter has no closing ---".into()))?;
        let yaml = &rest[..marker];
        let after = &rest[marker + 4..];
        let mut document: AutomationDocument = serde_yaml::from_str(yaml)
            .map_err(|error| AutomationError::Serialization(error.to_string()))?;
        validate_name(&document.name)?;
        if document.timeout_seconds == 0 || document.timeout_seconds > 86_400 {
            return Err(AutomationError::Invalid(
                "timeout_seconds must be between 1 and 86400".into(),
            ));
        }
        if document.retry.max_attempts == 0 || document.retry.max_attempts > 10 {
            return Err(AutomationError::Invalid(
                "retry.max_attempts must be between 1 and 10".into(),
            ));
        }
        if let Trigger::Schedule { schedule } = &document.trigger {
            if schedule.trim().is_empty() {
                return Err(AutomationError::Invalid("schedule cannot be empty".into()));
            }
        }
        if let Trigger::Event { source, .. } = &document.trigger {
            if source.trim().is_empty() {
                return Err(AutomationError::Invalid(
                    "event source cannot be empty".into(),
                ));
            }
        }
        document.body = after.trim_start_matches(['\r', '\n']).trim().to_string();
        if document.body.is_empty() {
            return Err(AutomationError::Invalid(
                "prompt body cannot be empty".into(),
            ));
        }
        document.directory = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
        Ok(document)
    }

    pub fn load(path: &Path) -> Result<Self> {
        Self::parse(path, &std::fs::read_to_string(path)?)
    }

    pub fn prompt(&self) -> String {
        let body = if self.body.contains("./memory.md") {
            self.body.clone()
        } else {
            format!("{MEMORY_PROMPT}\n\n{}", self.body)
        };
        format!("{body}\n\n## Dystil outputs\nYou may create multiple artifact files in this automation folder. For a structured live result, write valid JSON to a file ending `.live-view.json`. For a notification, write valid JSON to a file ending `.notification.json` with `title`, `body`, and optional `actions`; each action has a `label` and the `automation` name it starts. Do not create these structured outputs unless they help this run.")
    }
}

fn validate_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 80
        || !name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(AutomationError::Invalid(
            "name must be a lowercase kebab-case identifier".into(),
        ));
    }
    Ok(())
}

pub fn discover(root: &Path) -> Result<Vec<AutomationDocument>> {
    std::fs::create_dir_all(root)?;
    let mut documents = Vec::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path().join("automation.md");
        if entry.file_type()?.is_dir() && path.is_file() {
            documents.push(AutomationDocument::load(&path)?);
        }
    }
    documents.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(documents)
}

pub fn create(root: &Path, markdown: &str) -> Result<AutomationDocument> {
    let provisional = AutomationDocument::parse(&root.join("draft/automation.md"), markdown)?;
    let directory = root.join(&provisional.name);
    if directory.exists() {
        return Err(AutomationError::Invalid(format!(
            "automation {} already exists",
            provisional.name
        )));
    }
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("automation.md");
    std::fs::write(&path, markdown)?;
    AutomationDocument::load(&path)
}

pub fn set_enabled(path: &Path, enabled: bool) -> Result<AutomationDocument> {
    let source = std::fs::read_to_string(path)?;
    let rest = source
        .strip_prefix("---\n")
        .or_else(|| source.strip_prefix("---\r\n"))
        .ok_or_else(|| {
            AutomationError::Invalid("automation.md must start with YAML frontmatter".into())
        })?;
    let marker = rest
        .find("\n---")
        .ok_or_else(|| AutomationError::Invalid("frontmatter has no closing ---".into()))?;
    let mut yaml: serde_yaml::Mapping = serde_yaml::from_str(&rest[..marker])
        .map_err(|error| AutomationError::Serialization(error.to_string()))?;
    yaml.insert(
        serde_yaml::Value::String("enabled".into()),
        serde_yaml::Value::Bool(enabled),
    );
    let frontmatter = serde_yaml::to_string(&yaml)
        .map_err(|error| AutomationError::Serialization(error.to_string()))?;
    let updated = format!("---\n{frontmatter}---{}", &rest[marker + 4..]);
    std::fs::write(path, updated)?;
    AutomationDocument::load(path)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    Retrying,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    Configuration,
    Trigger,
    ProviderAuth,
    Provider,
    Tool,
    Timeout,
    Cancelled,
    Internal,
}

impl ErrorCategory {
    pub fn retryable(&self) -> bool {
        matches!(
            self,
            Self::Provider | Self::Tool | Self::Timeout | Self::Internal
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    pub id: String,
    pub automation_name: String,
    pub status: RunStatus,
    pub trigger: String,
    pub source_event_key: Option<String>,
    pub attempt: u32,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub output: Option<String>,
    pub error_category: Option<ErrorCategory>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunEvent {
    pub kind: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub run_id: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub media_type: String,
}

pub async fn migrate(pool: &SqlitePool) -> Result<()> {
    for statement in [
        "CREATE TABLE IF NOT EXISTS automation_runs (id TEXT PRIMARY KEY, automation_name TEXT NOT NULL, status TEXT NOT NULL, trigger TEXT NOT NULL, source_event_key TEXT, attempt INTEGER NOT NULL DEFAULT 1, started_at TEXT, finished_at TEXT, provider TEXT, model TEXT, output TEXT, error_category TEXT, error_message TEXT, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        "CREATE UNIQUE INDEX IF NOT EXISTS automation_success_event ON automation_runs(automation_name, source_event_key) WHERE status = 'succeeded' AND source_event_key IS NOT NULL",
        "CREATE TABLE IF NOT EXISTS automation_run_events (id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL, kind TEXT NOT NULL, message TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        "CREATE TABLE IF NOT EXISTS automation_cursors (automation_name TEXT NOT NULL, source TEXT NOT NULL, cursor TEXT NOT NULL, updated_at TEXT NOT NULL DEFAULT (datetime('now')), PRIMARY KEY(automation_name, source))",
        "CREATE TABLE IF NOT EXISTS automation_artifacts (id TEXT PRIMARY KEY, run_id TEXT NOT NULL, relative_path TEXT NOT NULL, size_bytes INTEGER NOT NULL, media_type TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (datetime('now')), UNIQUE(run_id, relative_path))",
    ] { sqlx::query(statement).execute(pool).await?; }
    Ok(())
}

pub async fn enqueue(
    pool: &SqlitePool,
    automation: &str,
    trigger: &str,
    event_key: Option<&str>,
    attempt: u32,
) -> Result<Option<String>> {
    if let Some(key) = event_key {
        let exists: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM automation_runs WHERE automation_name=?1 AND source_event_key=?2 AND status IN ('queued','running','retrying','succeeded')")
            .bind(automation).bind(key).fetch_one(pool).await?;
        if exists > 0 {
            return Ok(None);
        }
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO automation_runs(id,automation_name,status,trigger,source_event_key,attempt) VALUES(?1,?2,'queued',?3,?4,?5)")
        .bind(&id).bind(automation).bind(trigger).bind(event_key).bind(attempt as i64).execute(pool).await?;
    Ok(Some(id))
}

pub async fn set_cursor(
    pool: &SqlitePool,
    automation: &str,
    source: &str,
    cursor: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO automation_cursors(automation_name,source,cursor) VALUES(?1,?2,?3) ON CONFLICT(automation_name,source) DO UPDATE SET cursor=excluded.cursor, updated_at=datetime('now')")
        .bind(automation).bind(source).bind(cursor).execute(pool).await?;
    Ok(())
}

pub async fn cursor(pool: &SqlitePool, automation: &str, source: &str) -> Result<Option<String>> {
    Ok(sqlx::query_scalar(
        "SELECT cursor FROM automation_cursors WHERE automation_name=?1 AND source=?2",
    )
    .bind(automation)
    .bind(source)
    .fetch_optional(pool)
    .await?)
}

pub async fn list_runs(
    pool: &SqlitePool,
    automation: Option<&str>,
    before: Option<&str>,
    limit: u32,
) -> Result<Vec<RunRecord>> {
    let rows = sqlx::query("SELECT id,automation_name,status,trigger,source_event_key,attempt,started_at,finished_at,provider,model,output,error_category,error_message FROM automation_runs WHERE (?1 IS NULL OR automation_name=?1) AND (?2 IS NULL OR created_at < ?2) ORDER BY created_at DESC,id DESC LIMIT ?3")
        .bind(automation).bind(before).bind(limit.clamp(1, 100) as i64).fetch_all(pool).await?;
    rows.into_iter().map(row_to_run).collect()
}

fn row_to_run(row: sqlx::sqlite::SqliteRow) -> Result<RunRecord> {
    let parse_status = |value: String| {
        serde_json::from_str::<RunStatus>(&format!("\"{value}\""))
            .map_err(|error| AutomationError::Serialization(error.to_string()))
    };
    let parse_category = |value: String| {
        serde_json::from_str::<ErrorCategory>(&format!("\"{value}\""))
            .map_err(|error| AutomationError::Serialization(error.to_string()))
    };
    let category: Option<String> = row.get("error_category");
    Ok(RunRecord {
        id: row.get("id"),
        automation_name: row.get("automation_name"),
        status: parse_status(row.get("status"))?,
        trigger: row.get("trigger"),
        source_event_key: row.get("source_event_key"),
        attempt: row.get::<i64, _>("attempt") as u32,
        started_at: row.get("started_at"),
        finished_at: row.get("finished_at"),
        provider: row.get("provider"),
        model: row.get("model"),
        output: row.get("output"),
        error_category: category.map(parse_category).transpose()?,
        error_message: row.get("error_message"),
    })
}

#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    pub run_id: String,
    pub prompt: String,
    pub working_directory: PathBuf,
    pub timeout_seconds: u64,
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub provider: String,
    pub model: String,
}

#[async_trait]
pub trait AutomationRunner: Send + Sync {
    async fn run(
        &self,
        request: ExecutionRequest,
        events: mpsc::Sender<RunEvent>,
    ) -> std::result::Result<ExecutionResult, (ErrorCategory, String)>;
}

pub async fn execute<R: AutomationRunner>(
    pool: &SqlitePool,
    document: &AutomationDocument,
    run_id: &str,
    runner: &R,
) -> Result<RunRecord> {
    sqlx::query(
        "UPDATE automation_runs SET status='running',started_at=datetime('now') WHERE id=?1",
    )
    .bind(run_id)
    .execute(pool)
    .await?;
    let before = snapshot_files(&document.directory)?;
    let (tx, mut rx) = mpsc::channel::<RunEvent>(128);
    let event_pool = pool.clone();
    let event_run = run_id.to_string();
    let log_directory = document.directory.join(".dystil").join("runs").join(run_id);
    std::fs::create_dir_all(&log_directory)?;
    let event_task = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let _ = sqlx::query("INSERT INTO automation_run_events(run_id,kind,message,created_at) VALUES(?1,?2,?3,?4)").bind(&event_run).bind(&event.kind).bind(&event.message).bind(&event.created_at).execute(&event_pool).await;
            let filename = if event.kind == "stderr" {
                "stderr.log"
            } else {
                "stdout.log"
            };
            if let Ok(mut file) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(log_directory.join(filename))
                .await
            {
                use tokio::io::AsyncWriteExt;
                let _ = file
                    .write_all(format!("{} {}\n", event.created_at, event.message).as_bytes())
                    .await;
            }
        }
    });
    let result = runner
        .run(
            ExecutionRequest {
                run_id: run_id.into(),
                prompt: document.prompt(),
                working_directory: document.directory.clone(),
                timeout_seconds: document.timeout_seconds,
            },
            tx,
        )
        .await;
    event_task
        .await
        .map_err(|error| AutomationError::Runner(error.to_string()))?;
    match result {
        Ok(result) => {
            sqlx::query("UPDATE automation_runs SET status='succeeded',finished_at=datetime('now'),provider=?1,model=?2,output=?3 WHERE id=?4").bind(result.provider).bind(result.model).bind(result.output).bind(run_id).execute(pool).await?;
            discover_artifacts(pool, run_id, &document.directory, &before).await?;
        }
        Err((category, message)) => {
            sqlx::query("UPDATE automation_runs SET status='failed',finished_at=datetime('now'),error_category=?1,error_message=?2 WHERE id=?3").bind(serde_json::to_value(&category).unwrap().as_str().unwrap()).bind(message).bind(run_id).execute(pool).await?;
        }
    }
    list_runs(pool, None, None, 100)
        .await?
        .into_iter()
        .find(|run| run.id == run_id)
        .ok_or_else(|| AutomationError::NotFound(run_id.into()))
}

fn snapshot_files(root: &Path) -> Result<HashMap<PathBuf, (u64, u64)>> {
    fn visit(
        root: &Path,
        path: &Path,
        out: &mut HashMap<PathBuf, (u64, u64)>,
    ) -> std::io::Result<()> {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if entry.file_name() == ".dystil" {
                    continue;
                }
                visit(root, &path, out)?;
            } else if path.file_name().and_then(|x| x.to_str()) != Some("automation.md")
                && path.file_name().and_then(|x| x.to_str()) != Some("memory.md")
            {
                let m = entry.metadata()?;
                let modified = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.insert(
                    path.strip_prefix(root).unwrap_or(&path).to_path_buf(),
                    (m.len(), modified),
                );
            }
        }
        Ok(())
    }
    let mut files = HashMap::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

async fn discover_artifacts(
    pool: &SqlitePool,
    run_id: &str,
    root: &Path,
    before: &HashMap<PathBuf, (u64, u64)>,
) -> Result<()> {
    for (path, (size, modified)) in snapshot_files(root)? {
        if before.get(&path) == Some(&(size, modified)) {
            continue;
        }
        let extension = path.extension().and_then(|x| x.to_str()).unwrap_or("");
        let media_type = match extension {
            "md" => "text/markdown",
            "json" => "application/json",
            "html" => "text/html",
            "csv" => "text/csv",
            _ => "application/octet-stream",
        };
        let id = format!("artifact:{}", hex_digest(run_id, &path));
        sqlx::query("INSERT OR IGNORE INTO automation_artifacts(id,run_id,relative_path,size_bytes,media_type) VALUES(?1,?2,?3,?4,?5)").bind(id).bind(run_id).bind(path.to_string_lossy().as_ref()).bind(size as i64).bind(media_type).execute(pool).await?;
    }
    Ok(())
}

fn hex_digest(run_id: &str, path: &Path) -> String {
    let mut h = Sha256::new();
    h.update(run_id);
    h.update(path.to_string_lossy().as_bytes());
    format!("{:x}", h.finalize())[..24].to_string()
}

pub fn new_event_key(source: &str, cursor: &str) -> String {
    let mut h = Sha256::new();
    h.update(source);
    h.update([0]);
    h.update(cursor);
    format!("{:x}", h.finalize())
}

pub fn now_event(kind: impl Into<String>, message: impl Into<String>) -> RunEvent {
    RunEvent {
        kind: kind.into(),
        message: message.into(),
        created_at: Utc::now().to_rfc3339(),
    }
}

pub fn authored_markdown(name: &str, title: &str, prompt: &str) -> Result<String> {
    validate_name(name)?;
    if prompt.trim().is_empty() {
        return Err(AutomationError::Invalid("prompt cannot be empty".into()));
    }
    let title_yaml =
        serde_yaml::to_string(title).map_err(|e| AutomationError::Serialization(e.to_string()))?;
    Ok(format!("---\nname: {name}\ntitle: {}enabled: false\ntrigger:\n  type: manual\nretry:\n  max_attempts: 1\n---\n\n{MEMORY_PROMPT}\n\n{}\n", title_yaml, prompt.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FixtureRunner;
    #[async_trait]
    impl AutomationRunner for FixtureRunner {
        async fn run(
            &self,
            request: ExecutionRequest,
            events: mpsc::Sender<RunEvent>,
        ) -> std::result::Result<ExecutionResult, (ErrorCategory, String)> {
            events
                .send(now_event("assistant", "fixture analyzed"))
                .await
                .unwrap();
            std::fs::write(
                request.working_directory.join("recap.md"),
                "# Fixture recap\n",
            )
            .unwrap();
            std::fs::write(
                request.working_directory.join("memory.md"),
                "# memory\n\n## Lessons\n",
            )
            .unwrap();
            Ok(ExecutionResult {
                output: "created recap.md".into(),
                provider: "fixture".into(),
                model: "deterministic".into(),
            })
        }
    }

    async fn memory_pool() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        migrate(&pool).await.unwrap();
        pool
    }

    #[test]
    fn parses_all_trigger_families_and_injects_memory() {
        for yaml in [
            "type: manual",
            "type: schedule\n  schedule: every 5 minutes",
            "type: event\n  source: frames",
        ] {
            let source =
                format!("---\nname: fixture-test\nenabled: true\ntrigger:\n  {yaml}\n---\nDo work");
            let doc =
                AutomationDocument::parse(Path::new("/tmp/fixture-test/automation.md"), &source)
                    .unwrap();
            assert!(doc.prompt().contains("./memory.md"));
        }
    }

    #[tokio::test]
    async fn event_dedup_cursor_execution_memory_and_artifacts_work() {
        let temp = tempfile::tempdir().unwrap();
        let markdown = authored_markdown(
            "fixture-recap",
            "Fixture recap",
            "Analyze captured fixture rows.",
        )
        .unwrap();
        let doc = create(temp.path(), &markdown).unwrap();
        let pool = memory_pool().await;
        set_cursor(&pool, &doc.name, "frames", "41").await.unwrap();
        assert_eq!(
            cursor(&pool, &doc.name, "frames").await.unwrap().as_deref(),
            Some("41")
        );
        let key = new_event_key("frames", "42");
        let run_id = enqueue(&pool, &doc.name, "event", Some(&key), 1)
            .await
            .unwrap()
            .unwrap();
        assert!(
            enqueue(&pool, &doc.name, "event", Some(&key), 1)
                .await
                .unwrap()
                .is_none(),
            "a queued delivery must suppress duplicate in-flight work"
        );
        let run = execute(&pool, &doc, &run_id, &FixtureRunner).await.unwrap();
        assert_eq!(run.status, RunStatus::Succeeded);
        assert!(doc.directory.join("memory.md").is_file());
        assert!(enqueue(&pool, &doc.name, "event", Some(&key), 1)
            .await
            .unwrap()
            .is_none());
        let failed_key = new_event_key("frames", "43");
        let failed_run = enqueue(&pool, &doc.name, "event", Some(&failed_key), 1)
            .await
            .unwrap()
            .unwrap();
        sqlx::query("UPDATE automation_runs SET status='failed' WHERE id=?1")
            .bind(failed_run)
            .execute(&pool)
            .await
            .unwrap();
        assert!(
            enqueue(&pool, &doc.name, "event", Some(&failed_key), 2)
                .await
                .unwrap()
                .is_some(),
            "a failed delivery must remain retryable"
        );
        let artifacts: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM automation_artifacts WHERE run_id=?1")
                .bind(run_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(artifacts, 1);
    }

    #[tokio::test]
    async fn historical_fixture_databases_expose_frames_for_event_sources() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixture");
        for name in ["macos.sqlite", "windows.sqlite"] {
            let path = root.join(name);
            let url = format!("sqlite:{}?mode=ro", path.display());
            let pool = SqlitePool::connect(&url).await.unwrap();
            let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM frames")
                .fetch_one(&pool)
                .await
                .unwrap();
            assert!(count > 0, "{name} should contain event-source frames");
            let newest: i64 = sqlx::query_scalar("SELECT MAX(id) FROM frames")
                .fetch_one(&pool)
                .await
                .unwrap();
            let state = memory_pool().await;
            let temp = tempfile::tempdir().unwrap();
            let markdown = authored_markdown(
                &format!("{}-fixture-event", name.trim_end_matches(".sqlite")),
                "Fixture event",
                "Create a fixture report.",
            )
            .unwrap();
            let document = create(temp.path(), &markdown).unwrap();
            set_cursor(&state, &document.name, "frames", &(newest - 1).to_string())
                .await
                .unwrap();
            let key = new_event_key("frames", &newest.to_string());
            let run_id = enqueue(&state, &document.name, "event", Some(&key), 1)
                .await
                .unwrap()
                .unwrap();
            let run = execute(&state, &document, &run_id, &FixtureRunner)
                .await
                .unwrap();
            assert_eq!(
                run.status,
                RunStatus::Succeeded,
                "{name} event should execute"
            );
        }
    }
}
