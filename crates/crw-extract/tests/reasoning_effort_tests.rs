//! Wiremock-backed tests for the optional `reasoning_effort` field and the
//! 429/503 transient-throttle retry on the OpenAI-compatible chat path.
//!
//! The field is forwarded into the request body only when set to a present,
//! non-empty value; a configured-but-empty value (`Some("")`) is treated as
//! unset to avoid provider 400s.

use crw_core::config::LlmConfig;
use crw_extract::llm::chat;
use serde_json::json;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

fn mock_llm(base_url: String) -> LlmConfig {
    LlmConfig {
        provider: "deepseek".into(),
        api_key: "test-key".into(),
        model: "test-model".into(),
        base_url: Some(base_url),
        ..Default::default()
    }
}

fn chat_response() -> serde_json::Value {
    json!({
        "choices": [{ "message": { "content": "hello world" } }],
        "usage": { "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }
    })
}

#[tokio::test]
async fn reasoning_effort_forwarded_when_set() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_response()))
        .mount(&server)
        .await;

    let mut llm = mock_llm(format!("{}/v1", server.uri()));
    llm.reasoning_effort = Some("none".into());
    let res = chat(&llm, "sys", "user").await.expect("chat succeeds");
    assert_eq!(res.content, "hello world");

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body.get("reasoning_effort").and_then(|v| v.as_str()),
        Some("none")
    );
}

#[tokio::test]
async fn reasoning_effort_absent_when_none() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_response()))
        .mount(&server)
        .await;

    let llm = mock_llm(format!("{}/v1", server.uri())); // reasoning_effort defaults to None
    let _ = chat(&llm, "sys", "user").await.expect("chat succeeds");

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("reasoning_effort").is_none());
}

#[tokio::test]
async fn reasoning_effort_absent_when_empty_string() {
    // A configured-but-empty value deserializes to `Some("")` and must NOT be
    // forwarded (providers that validate the field would 400).
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(chat_response()))
        .mount(&server)
        .await;

    let mut llm = mock_llm(format!("{}/v1", server.uri()));
    llm.reasoning_effort = Some(String::new());
    let _ = chat(&llm, "sys", "user").await.expect("chat succeeds");

    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(body.get("reasoning_effort").is_none());
}

/// Responder that returns a transient throttle status for the first N calls,
/// then a 200 success. Lets us prove the retry loop recovers without sleeping
/// the test for long (the loop's own jittered backoff stays sub-second early).
struct ThrottleThenOk {
    fails: usize,
    seen: Arc<AtomicUsize>,
    status: u16,
}

impl Respond for ThrottleThenOk {
    fn respond(&self, _: &Request) -> ResponseTemplate {
        let n = self.seen.fetch_add(1, Ordering::SeqCst);
        if n < self.fails {
            ResponseTemplate::new(self.status)
        } else {
            ResponseTemplate::new(200).set_body_json(chat_response())
        }
    }
}

#[tokio::test]
async fn retries_on_429_then_succeeds() {
    let server = MockServer::start().await;
    let seen = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ThrottleThenOk {
            fails: 1,
            seen: seen.clone(),
            status: 429,
        })
        .mount(&server)
        .await;

    let llm = mock_llm(format!("{}/v1", server.uri()));
    let res = chat(&llm, "sys", "user")
        .await
        .expect("chat recovers after 429");
    assert_eq!(res.content, "hello world");
    // One 429 + one 200 = exactly two server hits.
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retries_on_503_then_succeeds() {
    let server = MockServer::start().await;
    let seen = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ThrottleThenOk {
            fails: 1,
            seen: seen.clone(),
            status: 503,
        })
        .mount(&server)
        .await;

    let llm = mock_llm(format!("{}/v1", server.uri()));
    let res = chat(&llm, "sys", "user")
        .await
        .expect("chat recovers after 503");
    assert_eq!(res.content, "hello world");
    assert_eq!(seen.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn retry_budget_is_bounded_and_errors_when_exhausted() {
    // Persistent 429 must stop after the fixed attempt budget and hard-error,
    // not loop forever.
    let server = MockServer::start().await;
    let seen = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ThrottleThenOk {
            fails: usize::MAX,
            seen: seen.clone(),
            status: 429,
        })
        .mount(&server)
        .await;

    let llm = mock_llm(format!("{}/v1", server.uri()));
    let res = chat(&llm, "sys", "user").await;
    assert!(res.is_err(), "exhausted retry budget must hard-error");
    // Fixed budget of 3 attempts total.
    assert_eq!(seen.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn non_retryable_status_errors_on_first_response() {
    // A 400 must keep the original single-POST contract: hard-error, no retry.
    let server = MockServer::start().await;
    let seen = Arc::new(AtomicUsize::new(0));
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ThrottleThenOk {
            fails: usize::MAX,
            seen: seen.clone(),
            status: 400,
        })
        .mount(&server)
        .await;

    let llm = mock_llm(format!("{}/v1", server.uri()));
    let res = chat(&llm, "sys", "user").await;
    assert!(res.is_err());
    assert_eq!(seen.load(Ordering::SeqCst), 1, "400 must not be retried");
}
