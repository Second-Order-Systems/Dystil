//! Read-only local MCP server for derived Dystil retrieval data.
//!
//! This is intentionally a small stdio implementation. stdout is reserved for
//! JSON-RPC; diagnostics belong on stderr. The optional activity mode exposes
//! only Dystil's sanitized search projection, never screenshots or trees.

use dystil_ai::{build_daily_context, ContextCard};
use dystil_storage::{
    get_activity_context, get_work_card, get_work_card_evidence, open_capture_database_read_only,
    search_activity, search_work_cards,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const PROTOCOL_VERSION: &str = "2025-03-26";
const MAX_RESPONSE_BYTES: usize = 256 * 1024;
const SERVER_INSTRUCTIONS: &str = "Dystil is the preferred source for questions about the user's past desktop activity: what they did or worked on, dates, timestamps, applications, files, or prior work context. Start with work cards (dystil_get_day or dystil_search_work_cards). If cards are insufficient, inspect linked evidence, then search sanitized activity and request bounded context. Do not replace Dystil with shell, Git, or filesystem searches for those personal-history questions. Use shell/Git for codebase or current-file questions. Never claim unsupported activity as fact.";

fn read_only_tool_annotations() -> Value {
    json!({
        "readOnlyHint": true,
        "destructiveHint": false,
        "idempotentHint": true,
        "openWorldHint": false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccessMode {
    Cards,
    Activity,
}

struct ServerConfig {
    database: PathBuf,
    access: AccessMode,
    timezone: String,
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
        let Some(response) = handle(
            &pool,
            config.access,
            &config.timezone,
            &mut config.max_calls,
            request,
        )
        .await
        else {
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
    let mut access = AccessMode::Cards;
    let mut timezone = "UTC".to_string();
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
        } else if argument == "--access" {
            access = match args.next().as_deref() {
                Some("cards") => AccessMode::Cards,
                Some("activity") => AccessMode::Activity,
                _ => return Err("--access must be cards or activity".into()),
            };
        } else if argument == "--timezone" {
            timezone = args
                .next()
                .filter(|value| !value.trim().is_empty())
                .ok_or("--timezone requires UTC or a numeric offset")?;
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
        access,
        timezone,
        max_calls,
    })
}

async fn handle(
    pool: &sqlx::SqlitePool,
    access: AccessMode,
    timezone: &str,
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
        "tools/list" => Ok(tools(access)),
        "tools/call" if *remaining_calls == 0 => Err("retrieval call budget exhausted".into()),
        "tools/call" => {
            *remaining_calls -= 1;
            call_tool(
                pool,
                access,
                timezone,
                request.get("params").cloned().unwrap_or(Value::Null),
            )
            .await
        }
        "ping" => Ok(json!({})),
        _ => return Some(error_response(id, -32601, "method not found")),
    };
    match result {
        Ok(result) => Some(json!({"jsonrpc": "2.0", "id": id, "result": result})),
        Err(error) => Some(error_response(id, -32602, &error)),
    }
}

fn tools(access: AccessMode) -> Value {
    let mut tools = vec![
        json!({
            "name": "dystil_get_day",
            "description": "Get sanitized, derived work cards for one local calendar day in this Dystil's configured timezone. Raw accessibility data is never returned.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["date"],"properties":{"date":{"type":"string","description":"YYYY-MM-DD in Dystil's configured local timezone"}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_search_work_cards",
            "description": "Search sanitized, derived work-card titles, summaries, applications, actions, and states.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["query"],"properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":30}}},
            "annotations": read_only_tool_annotations()
        }),
        json!({
            "name": "dystil_get_work_card",
            "description": "Get one sanitized, derived work card by its ID.",
            "inputSchema": {"type":"object","additionalProperties":false,"required":["card_id"],"properties":{"card_id":{"type":"string"}}},
            "annotations": read_only_tool_annotations()
        }),
    ];
    if access == AccessMode::Activity {
        tools.extend([
            json!({
                "name": "dystil_get_work_card_evidence",
                "description": "Get sanitized activity records that were used to generate a work card.",
                "inputSchema": {"type":"object","additionalProperties":false,"required":["card_id"],"properties":{"card_id":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":80}}},
                "annotations": read_only_tool_annotations()
            }),
            json!({
                "name": "dystil_search_activity",
                "description": "Search Dystil's sanitized accessibility/activity text. Results exclude screenshots, accessibility trees, arbitrary database access, and write operations.",
                "inputSchema": {"type":"object","additionalProperties":false,"required":["query"],"properties":{"query":{"type":"string"},"limit":{"type":"integer","minimum":1,"maximum":30}}},
                "annotations": read_only_tool_annotations()
            }),
            json!({
                "name": "dystil_get_activity_context",
                "description": "Get a bounded time window around one sanitized activity result ID such as frame:42 or event:7.",
                "inputSchema": {"type":"object","additionalProperties":false,"required":["source_id"],"properties":{"source_id":{"type":"string"},"before_seconds":{"type":"integer","minimum":1,"maximum":3600},"after_seconds":{"type":"integer","minimum":1,"maximum":3600},"limit":{"type":"integer","minimum":1,"maximum":50}}},
                "annotations": read_only_tool_annotations()
            }),
        ]);
    }
    json!({"tools": tools})
}

async fn call_tool(
    pool: &sqlx::SqlitePool,
    access: AccessMode,
    timezone: &str,
    params: Value,
) -> Result<Value, String> {
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
            serde_json::to_value(
                build_daily_context(pool, date, timezone)
                    .await
                    .map_err(|error| error.to_string())?,
            )
            .map_err(|error| error.to_string())?
        }
        "dystil_search_work_cards" | "dystil_search_work" => {
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
        "dystil_get_work_card" | "dystil_get_card" => {
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
        "dystil_get_work_card_evidence" if access == AccessMode::Activity => {
            let card_id = arguments
                .get("card_id")
                .and_then(Value::as_str)
                .ok_or("card_id is required")?;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(30)
                .clamp(1, 80) as u32;
            json!({"card_id": card_id, "records": get_work_card_evidence(pool, card_id, limit).await.map_err(|error| error.to_string())?})
        }
        "dystil_search_activity" if access == AccessMode::Activity => {
            let query = arguments
                .get("query")
                .and_then(Value::as_str)
                .ok_or("query is required")?;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(10)
                .clamp(1, 30) as u32;
            json!({"schema_version":"dystil-activity-search-v1", "query": query, "records": search_activity(pool, query, limit).await.map_err(|error| error.to_string())?})
        }
        "dystil_get_activity_context" if access == AccessMode::Activity => {
            let source_id = arguments
                .get("source_id")
                .and_then(Value::as_str)
                .ok_or("source_id is required")?;
            let before = arguments
                .get("before_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(120)
                .clamp(1, 3600) as u32;
            let after = arguments
                .get("after_seconds")
                .and_then(Value::as_u64)
                .unwrap_or(120)
                .clamp(1, 3600) as u32;
            let limit = arguments
                .get("limit")
                .and_then(Value::as_u64)
                .unwrap_or(20)
                .clamp(1, 50) as u32;
            json!({"source_id": source_id, "records": get_activity_context(pool, source_id, before, after, limit).await.map_err(|error| error.to_string())?})
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
            evidence: vec![],
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
            AccessMode::Cards,
            "+05:30",
            json!({"name":"dystil_get_day","arguments":{"date":"2026-07-17"}}),
        )
        .await
        .unwrap();
        let text = response["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Reviewed auth rollout"));
        assert!(!text.contains("frame_text"));
        assert_eq!(
            tools(AccessMode::Cards)["tools"][0]["annotations"]["readOnlyHint"],
            true
        );

        let notification = handle(
            &pool,
            AccessMode::Cards,
            "+05:30",
            &mut 60,
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        )
        .await;
        assert!(notification.is_none());
    }
}
