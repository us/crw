//! Phase-2 tool smoke tests against a live CDP endpoint.
//!
//! Mirrors `walking_skeleton.rs`'s pattern (stdio JSON-RPC against the
//! spawned `crw-browse` binary) but exercises the new tier-2/3/4 tools.
//! All tests in this file are `#[ignore]`d — they require a real CDP server
//! and `CRW_BROWSE_WS_URL` set:
//!
//! ```sh
//! CRW_BROWSE_WS_URL=ws://localhost:9223 \
//!   cargo test -p crw-browse --test phase2_tools -- --ignored
//! ```

use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires a live CDP endpoint — set CRW_BROWSE_WS_URL and run with --ignored"]
async fn evaluate_text_and_script_against_example_com() {
    let ws_url = std::env::var("CRW_BROWSE_WS_URL")
        .expect("CRW_BROWSE_WS_URL must be set to run this ignored test");

    let binary = env!("CARGO_BIN_EXE_crw-browse");
    let mut child = Command::new(binary)
        .arg("--ws-url")
        .arg(&ws_url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn crw-browse");

    let mut stdin = child.stdin.take().expect("stdin");
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout).lines();

    initialize(&mut stdin, &mut reader).await;

    // goto → evaluate (arithmetic)
    call(
        &mut stdin,
        &mut reader,
        3,
        "goto",
        serde_json::json!({"url": "https://example.com"}),
    )
    .await;

    let eval = call(
        &mut stdin,
        &mut reader,
        4,
        "evaluate",
        serde_json::json!({"expression": "(2+3)*7"}),
    )
    .await;
    let eval_json = parse_content(&eval);
    assert_eq!(
        eval_json["data"]["value"].as_i64(),
        Some(35),
        "evaluate did not return 35: {eval_json}"
    );

    // text — example.com header should contain "Example Domain".
    let text = call(
        &mut stdin,
        &mut reader,
        5,
        "text",
        serde_json::json!({"selector": "h1"}),
    )
    .await;
    let text_json = parse_content(&text);
    assert!(
        text_json["data"]["text"]
            .as_str()
            .unwrap_or("")
            .contains("Example Domain"),
        "text did not contain heading: {text_json}"
    );

    // script — multi-step: evaluate document.title, then re-evaluate (smoke).
    let script_resp = call(
        &mut stdin,
        &mut reader,
        6,
        "script",
        serde_json::json!({
            "actions": [
                {"act": "evaluate", "expression": "document.title"},
                {"act": "evaluate", "expression": "1 + 1"},
            ]
        }),
    )
    .await;
    let script_json = parse_content(&script_resp);
    assert_eq!(
        script_json["data"]["completed"].as_u64(),
        Some(2),
        "script reported wrong completed count: {script_json}"
    );
    assert_eq!(script_json["data"]["aborted"], Value::Bool(false));

    drop(stdin);
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
}

async fn initialize(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) {
    send_line(
        stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    )
    .await;
    let _ = next_json(reader).await;
    send_line(
        stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;
}

async fn call(
    stdin: &mut tokio::process::ChildStdin,
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    id: u64,
    name: &str,
    arguments: Value,
) -> Value {
    let req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments },
    });
    send_line(stdin, &req.to_string()).await;
    next_json(reader).await
}

fn parse_content(rpc_response: &Value) -> Value {
    let text = rpc_response["result"]["content"][0]["text"]
        .as_str()
        .expect("text content");
    serde_json::from_str(text).expect("inner JSON parse")
}

async fn send_line(stdin: &mut tokio::process::ChildStdin, line: &str) {
    stdin.write_all(line.as_bytes()).await.expect("write stdin");
    stdin.write_all(b"\n").await.expect("newline");
    stdin.flush().await.expect("flush");
}

async fn next_json(reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>) -> Value {
    let line = timeout(RPC_TIMEOUT, reader.next_line())
        .await
        .expect("timeout")
        .expect("read")
        .expect("stream closed");
    serde_json::from_str(&line).expect("valid JSON-RPC")
}
