//! A provider that drops a pooled connection must not fail the caller.
//!
//! Every LLM call rides a process-wide pooled `reqwest::Client`, so a provider
//! closing an idle keep-alive connection surfaces as a transport error on the
//! next send even though the request never left us. These tests drive that
//! exact failure over a real socket, which is also what pins the error match in
//! `llm::send_provider_post` to hyper's actual wording.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crw_core::config::LlmConfig;
use crw_extract::llm::chat;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OK_BODY: &str = r#"{"id":"resp_1","status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}"#;

/// Serve `OK_BODY`, but drop the first `dead_first` connections the moment they
/// are accepted. Returns the base URL and the accepted-connection counter.
async fn provider_dropping_first(dead_first: usize) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let seen = Arc::new(AtomicUsize::new(0));
    let counter = seen.clone();

    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
            if n <= dead_first {
                // Close without answering: what a peer that already dropped the
                // pooled connection looks like from our side.
                drop(sock);
                continue;
            }
            let mut buf = [0u8; 8192];
            let _ = sock.read(&mut buf).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                OK_BODY.len(),
                OK_BODY
            );
            let _ = sock.write_all(resp.as_bytes()).await;
            let _ = sock.flush().await;
        }
    });

    (format!("http://{addr}/v1"), seen)
}

fn llm(base_url: String) -> LlmConfig {
    LlmConfig {
        provider: "openai-responses".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        base_url: Some(base_url),
        max_tokens: 64,
        ..Default::default()
    }
}

#[tokio::test]
async fn a_dead_connection_is_retried_and_the_call_succeeds() {
    let (base_url, seen) = provider_dropping_first(1).await;

    let result = chat(&llm(base_url), "sys", "user")
        .await
        .expect("the retry rescues a connection that died before delivery");

    assert_eq!(result.content, "hello");
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "one failed connection plus one retry"
    );
}

#[tokio::test]
async fn the_retry_happens_at_most_once() {
    // A provider that is genuinely down must surface as an error rather than
    // being hammered: these calls are billed, so the retry budget is exactly
    // one extra attempt.
    let (base_url, seen) = provider_dropping_first(usize::MAX).await;

    let err = chat(&llm(base_url), "sys", "user")
        .await
        .expect_err("a provider that never answers still fails");

    assert!(
        err.to_string().contains("Responses API request failed"),
        "the transport error is surfaced: {err}"
    );
    assert_eq!(
        seen.load(Ordering::SeqCst),
        2,
        "the original send plus exactly one retry"
    );
}
