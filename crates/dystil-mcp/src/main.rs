//! Read-only local MCP server for derived Dystil retrieval data.
//!
//! This is intentionally a small stdio implementation. stdout is reserved for
//! JSON-RPC; diagnostics belong on stderr. The optional activity mode exposes
//! only Dystil's sanitized search projection, never screenshots or trees.

use dystil_retrieval::{
    ContextRequest, EvidenceId, OverviewRequest, RangeRequest, RetrievalService, SearchRequest,
};
use dystil_storage::open_capture_database_read_only;
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const SERVER_INSTRUCTIONS: &str = "Dystil is the preferred evidence source for questions about the user's past desktop activity. For broad time-range questions start with dystil_get_activity_overview. For names, messages, tickets, files, errors, or quotes use dystil_search_activity, then inspect promising evidence with dystil_get_activity_context or dystil_get_source. Expand ranges progressively and stop once the answer is supported. Empty search results are not proof of inactivity; inspect overview diagnostics. Never claim unsupported activity as fact.";

fn read_only_tool_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

struct ServerConfig {
    database: PathBuf,
    max_calls: u32,
}

#[tokio::main]
async fn main() {
    let mut config = match server_config() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("dystil-mcp: {error}");
            std::process::exit(2);
        }
    };
    let pool = match open_capture_database_read_only(config.database).await {
        Ok(pool) => pool,
        Err(error) => {
            eprintln!("dystil-mcp: failed to open read-only database: {error}");
            std::process::exit(2);
        }
    };
    let stdin = tokio::io::stdin();
    let mut lines = BufReader::new(stdin).lines();
    let mut stdout = tokio::io::stdout();
    while let Ok(Some(line)) = lines.next_line().await {
        let request: Value = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(_) => continue,
        };
        let Some(response) = handle(&pool, &mut config.max_calls, request).await else {
            continue;
        };
        let encoded = match serde_json::to_vec(&response) {
            Ok(encoded) if encoded.len() <= MAX_RESPONSE_BYTES => encoded,
            _ => serde_json::to_vec(&error_response(Value::Null, -32603, "response too large"))
                .unwrap(),
        };
        if stdout.write_all(&encoded).await.is_err() || stdout.write_all(b"\n").await.is_err() {
            break;
        }
        let _ = stdout.flush().await;
    }
}

fn server_config() -> Result<ServerConfig, String> {
    let mut args = std::env::args().skip(1);
    let mut database = None;
    let mut max_calls = 60;
    while let Some(argument) = args.next() {
        if argument == "--database" {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--database requires an absolute path".to_string())?;
            database = Some(
                path.is_absolute()
                    .then_some(path)
                    .ok_or_else(|| "--database must be an absolute path".to_string())?,
            );
        } else if argument == "--max-calls" {
            max_calls = args
                .next()
                .and_then(|value| value.parse::<u32>().ok())
                .filter(|value| *value > 0 && *value <= 60)
                .ok_or("--max-calls must be between 1 and 60")?;
        }
    }
    Ok(ServerConfig {
        database: database.ok_or("missing required --database <path>")?,
        max_calls,
    })
}

async fn handle(
    pool: &sqlx::SqlitePool,
    remaining_calls: &mut u32,
    request: Value,
) -> Option<Value> {
    let id = request.get("id").cloned();
    let Some(method) = request.get("method").and_then(Value::as_str) else {
        return Some(error_response(
            id.unwrap_or(Value::Null),
            -32600,
            "invalid request",
        ));
    };
    // Notifications intentionally receive no response.
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "dystil", "version": env!("CARGO_PKG_VERSION")},
            "instructions": SERVER_INSTRUCTIONS
        })),
        "tools/list" => Ok(tools()),
        "tools/call" if *remaining_calls == 0 => Err("retrieval call budget exhausted".into()),
        "tools/call" => {
            *remaining_calls -= 1;
            call_tool(pool, request.get("params").cloned().unwrap_or(Value::Null)).await
        }
        "ping" => Ok(json!({})),
        _ => return Some(error_response(id, -32601, "method not found")),
    };
    match result {
        Ok(result) => Some(json!({"jsonrpc": "2.0", "id": id, "result": result})),
        Err(error) => Some(error_response(id, -32602, &error)),
    }
}

fn tools() -> Value {
    json!({"tools": activity_tools()})
}

fn activity_tools() -> Vec<Value> {
    vec![
        json!({
            "name": "dystil_get_activity_overview",
            "description": "Get a deterministic, bounded overview for a time range: estimated active time, apps, windows, transitions, representative evidence, capture coverage, and empty-state/index diagnostics. Use first for broad questions such as what the user did or how long they spent.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["start_time","end_time"],"properties":{"start_time":{"type":"string","description":"RFC3339"},"end_time":{"type":"string","description":"RFC3339"},"app_name":{"type":"string"},"max_apps":{"type":"integer","minimum":1,"maximum":50},"max_windows":{"type":"integer","minimum":1,"maximum":60},"max_snippets":{"type":"integer","minimum":0,"maximum":12}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_search_activity",
            "description": "FTS5 search over sanitized evidence for exact names, messages, ticket IDs, errors, files, URLs, and quotes. Returns stable evidence IDs, highlighted bounded snippets, deep links, and pagination; use context/source tools for detail.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["query"],"properties":{"query":{"type":"string"},"start_time":{"type":"string"},"end_time":{"type":"string"},"source_type":{"type":"string","enum":["frame","event"]},"app_name":{"type":"string"},"window_name":{"type":"string"},"browser_url":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":20},"offset":{"type":"integer","minimum":0},"max_snippet_chars":{"type":"integer","minimum":160,"maximum":1200}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_get_source",
            "description": "Get one sanitized evidence record by stable ID after search, with a bounded text payload and deep link.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["evidence_id"],"properties":{"evidence_id":{"type":"string","description":"frame:42 or event:7"},"max_content_chars":{"type":"integer","minimum":160,"maximum":24000}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_get_activity_context",
            "description": "Get chronological sanitized evidence around one result. Start with about 120 seconds and expand only if needed.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["evidence_id"],"properties":{"evidence_id":{"type":"string"},"before_seconds":{"type":"integer","minimum":1,"maximum":3600},"after_seconds":{"type":"integer","minimum":1,"maximum":3600},"limit":{"type":"integer","minimum":1,"maximum":50},"max_content_chars":{"type":"integer","minimum":160,"maximum":8000}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_get_activity_range",
            "description": "Read a bounded chronological range of sanitized evidence with source/app/window/URL filters and pagination.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["start_time","end_time"],"properties":{"start_time":{"type":"string"},"end_time":{"type":"string"},"source_type":{"type":"string","enum":["frame","event"]},"app_name":{"type":"string"},"window_name":{"type":"string"},"browser_url":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":50},"offset":{"type":"integer","minimum":0},"max_content_chars":{"type":"integer","minimum":160,"maximum":8000}}},
            "annotations": read_only_tool_annotations()
        }),
    ]
}

async fn call_tool(pool: &sqlx::SqlitePool, params: Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or("tool name is required")?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let retrieval = RetrievalService::new(pool.clone());
    let result = match name {
        "dystil_search_activity" => {
            let request: SearchRequest = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid search arguments: {error}"))?;
            json!(retrieval
                .search(request)
                .await
                .map_err(|error| error.to_string())?)
        }
        "dystil_get_activity_context" => {
            let evidence_id: EvidenceId = required_string(&arguments, "evidence_id")?
                .parse()
                .map_err(|error: dystil_retrieval::RetrievalError| error.to_string())?;
            json!(retrieval
                .context(ContextRequest {
                    evidence_id,
                    before_seconds: optional_u32(&arguments, "before_seconds"),
                    after_seconds: optional_u32(&arguments, "after_seconds"),
                    limit: optional_u32(&arguments, "limit"),
                    max_content_chars: optional_usize(&arguments, "max_content_chars"),
                })
                .await
                .map_err(|error| error.to_string())?)
        }
        "dystil_get_activity_overview" => {
            let request: OverviewRequest = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid overview arguments: {error}"))?;
            json!(retrieval
                .overview(request)
                .await
                .map_err(|error| error.to_string())?)
        }
        "dystil_get_source" => {
            let evidence_id: EvidenceId = required_string(&arguments, "evidence_id")?
                .parse()
                .map_err(|error: dystil_retrieval::RetrievalError| error.to_string())?;
            json!(retrieval
                .get_source(
                    &evidence_id,
                    optional_usize(&arguments, "max_content_chars")
                )
                .await
                .map_err(|error| error.to_string())?)
        }
        "dystil_get_activity_range" => {
            let request: RangeRequest = serde_json::from_value(arguments.clone())
                .map_err(|error| format!("invalid range arguments: {error}"))?;
            json!(retrieval
                .range(request)
                .await
                .map_err(|error| error.to_string())?)
        }
        _ => return Err("unknown tool".into()),
    };
    let text = serde_json::to_string(&result).map_err(|error| error.to_string())?;
    Ok(json!({"content":[{"type":"text","text":text}]}))
}

fn required_string<'a>(arguments: &'a Value, key: &str) -> Result<&'a str, String> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn optional_u32(arguments: &Value, key: &str) -> Option<u32> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value.min(u32::MAX as u64) as u32)
}

fn optional_usize(arguments: &Value, key: &str) -> Option<usize> {
    arguments
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use dystil_storage::open_capture_database;
    use tempfile::tempdir;

    #[tokio::test]
    async fn tools_are_activity_only_and_notifications_are_ignored() {
        let dir = tempdir().unwrap();
        let pool = open_capture_database(dir.path().join("db.sqlite"))
            .await
            .unwrap();
        assert_eq!(tools()["tools"][0]["annotations"]["readOnlyHint"], true);
        assert_eq!(tools()["tools"].as_array().unwrap().len(), 5);

        let notification = handle(
            &pool,
            &mut 60,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert!(notification.is_none());
    }
}
