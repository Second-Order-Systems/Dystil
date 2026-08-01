use dystil_storage::open_capture_database;
use serde_json::{json, Value};
use std::process::Stdio;
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

async fn read_json_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> Value {
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn stdio_server_exposes_bounded_evidence_tools() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("db.sqlite");
    let pool = open_capture_database(&database).await.unwrap();
    sqlx::query(
        "INSERT INTO frames(timestamp, frame_text) VALUES ('2026-07-17T09:05:00Z', 'Investigated MCP activity retrieval')",
    )
    .execute(&pool)
    .await
    .unwrap();
    // Keep Dystil's normal writer pool alive while the sidecar opens its own
    // read-only connection, matching the real desktop process arrangement.

    let mut child = Command::new(env!("CARGO_BIN_EXE_dystil-mcp"))
        .args(["--database", database.to_str().unwrap(), "--max-calls", "1"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();

    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}\n")
        .await
        .unwrap();
    let initialized = read_json_line(&mut lines).await;
    assert_eq!(initialized["result"]["serverInfo"]["name"], "dystil");
    assert!(initialized["result"]["instructions"]
        .as_str()
        .unwrap()
        .contains("preferred evidence source"));

    let request = json!({"jsonrpc":"2.0", "id":2, "method":"tools/list"});
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut lines).await;
    let tools = response["result"]["tools"].as_array().unwrap();
    assert!(tools
        .iter()
        .any(|tool| tool["name"] == "dystil_get_activity_overview"));
    assert!(tools.iter().all(|tool| tool["name"]
        .as_str()
        .is_some_and(|name| name.starts_with("dystil_") && !name.contains("card"))));

    let request = json!({
        "jsonrpc":"2.0", "id":3, "method":"tools/call",
        "params":{"name":"dystil_search_activity","arguments":{"query":"MCP activity"}}
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut lines).await;
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(
        content.contains("Investigated [MCP] [activity] retrieval")
            && content.contains("dystil://evidence/frame/1"),
        "unexpected search response: {content}"
    );

    let request = json!({
        "jsonrpc":"2.0", "id":4, "method":"tools/call",
        "params":{"name":"dystil_get_source","arguments":{"evidence_id":"frame:1"}}
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut lines).await;
    assert_eq!(
        response["error"]["message"],
        "retrieval call budget exhausted"
    );

    drop(stdin);
    let status = child.wait().await.unwrap();
    assert!(status.success());
    pool.close().await;
}
