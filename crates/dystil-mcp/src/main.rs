//! Read-only local MCP server for derived Dystil work cards.
//!
//! This is intentionally a small stdio implementation. stdout is reserved for
//! JSON-RPC; diagnostics belong on stderr. It never queries raw capture tables.

use dystil_ai::{build_daily_context, ContextCard};
use dystil_storage::{get_work_card, open_capture_database_read_only, search_work_cards};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;

#[tokio::main]
async fn main() {
    let database = match database_argument() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("dystil-mcp: {error}");
            std::process::exit(2);
        }
    };
    let pool = match open_capture_database_read_only(database).await {
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
        let Some(response) = handle(&pool, request).await else {
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

fn database_argument() -> Result<PathBuf, String> {
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        if argument == "--database" {
            let path = args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "--database requires an absolute path".to_string())?;
            return path
                .is_absolute()
                .then_some(path)
                .ok_or_else(|| "--database must be an absolute path".into());
        }
    }
    Err("missing required --database <path>".into())
}

async fn handle(pool: &sqlx::SqlitePool, request: Value) -> Option<Value> {
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
            "serverInfo": {"name": "dystil", "version": env!("CARGO_PKG_VERSION")}
        })),
        "tools/list" => Ok(tools()),
        "tools/call" => {
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
    json!({"tools": [
        {
            "name": "dystil_get_day",
            "description": "Get sanitized, derived work cards for one local calendar day. Raw accessibility data is never returned.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["date"],"properties":{"date":{"type":"string","description":"YYYY-MM-DD"},"timezone":{"type":"string","description":"UTC or numeric offset such as +05:30"}}}
        },
        {
            "name": "dystil_search_work",
            "description": "Search sanitized, derived work-card titles, summaries, applications, actions, and states.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["query"],"properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":30}}}
        },
        {
            "name": "dystil_get_card",
            "description": "Get one sanitized, derived work card by its ID.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["card_id"],"properties":{"card_id":{"type":"string"}}}
        }
    ]})
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
    let result = match name {
        "dystil_get_day" => {
            let date = arguments
                .get("date")
                .and_then(Value::as_str)
                .ok_or("date is required")?;
            let timezone = arguments
                .get("timezone")
                .and_then(Value::as_str)
                .unwrap_or("UTC");
            serde_json::to_value(
                build_daily_context(pool, date, timezone)
                    .await
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?
        }
        "dystil_search_work" => {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .ok_or("query is required")?;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 30) as u32;
            let cards = search_work_cards(pool, query, limit)
                .await
                .map_err(|error| error.to_string())?;
            json!({"schema_version":"dystil-search-v1","query":query,"cards":cards.iter().map(ContextCard::from).collect::<Vec<_>>()})
        }
        "dystil_get_card" => {
            let id = arguments
                .get("card_id")
                .and_then(Value::as_str)
                .ok_or("card_id is required")?;
            let card = get_work_card(pool, id)
                .await
                .map_err(|error| error.to_string())?
                .ok_or("work card not found")?;
            json!({"card": ContextCard::from(&card)})
        }
        _ => return Err("unknown tool".into()),
    };
    let text = serde_json::to_string(&result).map_err(|error| error.to_string())?;
    Ok(json!({"content":[{"type":"text","text":text}]}))
}

fn error_response(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[cfg(test)]
mod tests {
    use super::*;
    use dystil_storage::{open_capture_database, upsert_work_card, NewWorkCard};
    use tempfile::tempdir;

    fn card() -> NewWorkCard {
        NewWorkCard {
            window_id: "card-1".into(),
            start_time: "2026-07-17T09:00:00+05:30".into(),
            end_time: "2026-07-17T09:15:00+05:30".into(),
            close_reason: "max_duration".into(),
            title: "Reviewed auth rollout".into(),
            summary: "Checked deployment state".into(),
            applications: vec!["VS Code".into()],
            artifacts: json!([]),
            actions: json!([{"text":"Reviewed rollout"}]),
            last_observed_state: "Editor open".into(),
            status: "complete".into(),
            uncertainties: vec![],
            card_json: json!({}),
            model_id: "test".into(),
            source_hash: "sha256:test".into(),
            embedding_model_id: None,
            embedding: None,
        }
    }

    #[tokio::test]
    async fn tools_return_only_sanitized_derived_cards() {
        let dir = tempdir().unwrap();
        let pool = open_capture_database(dir.path().join("db.sqlite"))
            .await
            .unwrap();
        upsert_work_card(&pool, &card()).await.unwrap();

        let response = call_tool(
            &pool,
            json!({"name":"dystil_get_day","arguments":{"date":"2026-07-17","timezone":"+05:30"}}),
        )
        .await
        .unwrap();
        let text = response["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Reviewed auth rollout"));
        assert!(!text.contains("frame_text"));

        let notification = handle(
            &pool,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert!(notification.is_none());
    }
}
