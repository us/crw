//! Walking-skeleton integration test.
//!
//! Spawns the `crw-browse` binary against a locally running CDP endpoint
//! (Lightpanda or Chromium) and exercises the Phase 1 tool surface:
//! `initialize` → `tools/list` → `tools/call goto` → `tools/call tree`.
//!
//! This test is `#[ignore]`d by default — it needs a real CDP server and
//! `CRW_BROWSE_WS_URL` set. Previously it returned silently when the env var
//! was unset, which made it look "passing" in CI when it was actually
//! skipped. Run locally with:
//!
//! ```sh
//! CRW_BROWSE_WS_URL=ws://localhost:9222 \
//!   cargo test -p crw-browse --test walking_skeleton -- --ignored
//! ```

use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires a live CDP endpoint — set CRW_BROWSE_WS_URL and run with --ignored"]
async fn walking_skeleton_goto_then_tree() {
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

    // initialize
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}"#,
    )
    .await;
    let init = next_json(&mut reader).await;
    assert_eq!(init["id"], 1);
    assert!(init["result"]["serverInfo"].is_object());

    // notifications/initialized
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    )
    .await;

    // tools/list
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    )
    .await;
    let listed = next_json(&mut reader).await;
    let tool_names: Vec<String> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect();
    // Phase 2 surface — every Tier-2/3/4 tool must be advertised. If a tool
    // is renamed or removed, this assertion will fail loudly so the change
    // gets noticed before it ships.
    let expected = [
        "goto",
        "tree",
        "evaluate",
        "text",
        "html",
        "console",
        "network",
        "storage",
        "click",
        "fill",
        "type_text",
        "wait",
        "screenshot",
        "script",
    ];
    for name in expected {
        assert!(
            tool_names.contains(&name.to_string()),
            "missing tool {name:?} in advertised list: {tool_names:?}"
        );
    }

    // tools/call goto
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"goto","arguments":{"url":"https://example.com"}}}"#,
    )
    .await;
    let goto = next_json(&mut reader).await;
    assert!(
        goto["result"]["isError"].as_bool() != Some(true),
        "goto returned error: {goto}"
    );

    // tools/call tree
    send_line(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"tree","arguments":{}}}"#,
    )
    .await;
    let tree = next_json(&mut reader).await;
    let content = tree["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        content.contains("\"ok\""),
        "tree response missing ok field: {content}"
    );

    drop(stdin);
    let _ = timeout(Duration::from_secs(5), child.wait()).await;
}

async fn send_line(stdin: &mut tokio::process::ChildStdin, line: &str) {
    stdin.write_all(line.as_bytes()).await.expect("write stdin");
    stdin.write_all(b"\n").await.expect("newline");
    stdin.flush().await.expect("flush");
}

async fn next_json(
    reader: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
) -> serde_json::Value {
    let line = timeout(RPC_TIMEOUT, reader.next_line())
        .await
        .expect("timeout")
        .expect("read")
        .expect("stream closed");
    serde_json::from_str(&line).expect("valid JSON-RPC")
}
