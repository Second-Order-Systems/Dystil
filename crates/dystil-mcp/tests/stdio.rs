use dystil_storage::{open_capture_database, upsert_work_card, NewWorkCard};
use serde_json::{json, Value};
use std::process::Stdio;
use tempfile::tempdir;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

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

async fn read_json_line(
    lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> Value {
    serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap()
}

#[tokio::test]
async fn stdio_server_handles_initialize_and_returns_a_derived_card() {
    let directory = tempdir().unwrap();
    let database = directory.path().join("db.sqlite");
    let pool = open_capture_database(&database).await.unwrap();
    upsert_work_card(&pool, &card()).await.unwrap();
    // Keep Dystil's normal writer pool alive while the sidecar opens its own
    // read-only connection, matching the real desktop process arrangement.

    let mut child = Command::new(env!("CARGO_BIN_EXE_dystil-mcp"))
        .args(["--database", database.to_str().unwrap()])
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
    assert_eq!(
        read_json_line(&mut lines).await["result"]["serverInfo"]["name"],
        "dystil"
    );

    let request = json!({
        "jsonrpc":"2.0", "id":2, "method":"tools/call",
        "params":{"name":"dystil_get_card","arguments":{"card_id":"card-1"}}
    });
    stdin
        .write_all(format!("{request}\n").as_bytes())
        .await
        .unwrap();
    let response = read_json_line(&mut lines).await;
    let content = response["result"]["content"][0]["text"].as_str().unwrap();
    assert!(content.contains("Reviewed auth rollout"));
    assert!(!content.contains("frame_text"));

    drop(stdin);
    let status = child.wait().await.unwrap();
    assert!(status.success());
    pool.close().await;
}
