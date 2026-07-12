//! Offline integration tests for the native `/v1/extract` surface.
//!
//! Network-free: they exercise request validation (empty urls, nothing-to-
//! extract, over-cap, basis-without-schema, all-invalid urls), the unknown-job
//! 404, and the `/v1/capabilities` advertisement. End-to-end completion (which
//! needs an LLM) is covered separately.

use axum::http::StatusCode;
use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::{AppState, ExtractStatus, PreparedUrl};
use serde_json::{Value, json};

fn test_app() -> TestServer {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    let app = create_app(state);
    TestServer::new(app)
}

#[tokio::test]
async fn v1_extract_requires_urls_400() {
    let s = test_app();
    let r = s
        .post("/v1/extract")
        .json(&json!({"prompt": "get the title"}))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_requires_prompt_or_schema_400() {
    let s = test_app();
    let r = s
        .post("/v1/extract")
        .json(&json!({"urls": ["https://example.com"]}))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_whitespace_prompt_no_schema_400() {
    let s = test_app();
    // A blank prompt with no schema is nothing-to-extract — reject upfront
    // rather than fetch then fail in the worker.
    let r = s
        .post("/v1/extract")
        .json(&json!({ "urls": ["https://example.com"], "prompt": "   " }))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_rejects_basis_without_schema_400() {
    let s = test_app();
    // Evidence is emitted per top-level scalar SCHEMA property, so a prompt-only
    // extraction has no fields to attribute. Reject upfront rather than fetch
    // every URL and hand back an empty `basis`.
    let r = s
        .post("/v1/extract")
        .json(&json!({
            "urls": ["https://example.com"],
            "prompt": "get the title",
            "basis": true
        }))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_rejects_base_url_400() {
    let s = test_app();
    // baseUrl must be rejected (not silently ignored) to avoid routing a BYOK
    // key to the wrong endpoint.
    let r = s
        .post("/v1/extract")
        .json(&json!({
            "urls": ["https://example.com"],
            "prompt": "x",
            "baseUrl": "https://evil.example"
        }))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_rejects_over_cap_400() {
    let s = test_app();
    // Default max_extract_urls is 50; 51 distinct URLs must be rejected upfront.
    let urls: Vec<String> = (0..51)
        .map(|i| format!("https://example.com/{i}"))
        .collect();
    let r = s
        .post("/v1/extract")
        .json(&json!({ "urls": urls, "prompt": "x" }))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_all_invalid_urls_400() {
    let s = test_app();
    let r = s
        .post("/v1/extract")
        .json(&json!({ "urls": ["not a url", "also-not-a-url"], "prompt": "x" }))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn v1_extract_status_unknown_404() {
    let s = test_app();
    let r = s
        .get("/v1/extract/00000000-0000-0000-0000-000000000000")
        .await;
    r.assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn worker_seeds_preflight_failures_in_order() {
    // The worker processes entries in original order and surfaces preflight
    // failures as `failed` results without fetching. An all-preflight-failed
    // job needs no network/LLM, so it exercises the ordering + per_url wiring
    // offline. (The handler rejects all-invalid with 400; calling the worker
    // directly bypasses that guard to unit-test the worker itself.)
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).expect("AppState::new failed");

    let entries = vec![
        PreparedUrl {
            url: "https://a.example/1".into(),
            preflight_error: Some("bad-1".into()),
        },
        PreparedUrl {
            url: "https://b.example/2".into(),
            preflight_error: Some("bad-2".into()),
        },
        PreparedUrl {
            url: "https://c.example/3".into(),
            preflight_error: Some("bad-3".into()),
        },
    ];
    let template = crw_core::types::ScrapeRequest::default();
    let id = state.start_extract_job(entries, template).await;

    // Poll until terminal (the worker task runs on the spawned executor).
    let rec = loop {
        {
            let jobs = state.extract_jobs.read().await;
            if let Some(r) = jobs.get(&id)
                && r.status != ExtractStatus::Processing
            {
                break r.clone();
            }
        }
        tokio::task::yield_now().await;
    };

    assert_eq!(
        rec.status,
        ExtractStatus::Failed,
        "all URLs failed → job Failed"
    );
    let urls: Vec<&str> = rec.per_url.iter().map(|r| r.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://a.example/1",
            "https://b.example/2",
            "https://c.example/3"
        ]
    );
    assert!(
        rec.per_url
            .iter()
            .all(|r| r.status == ExtractStatus::Failed)
    );
    assert_eq!(rec.per_url[0].error.as_deref(), Some("bad-1"));
}

#[tokio::test]
async fn v1_extract_completes_end_to_end_with_mocked_llm() {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // Allow the loopback mock server past the SSRF guard (test-only knob).
    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    }

    let mock = MockServer::start().await;
    // The page the extractor scrapes.
    Mock::given(method("GET"))
        .and(path("/page"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<html><body><h1>Widget</h1><p>Price: $9</p></body></html>"),
        )
        .mount(&mock)
        .await;
    // The Anthropic-style LLM endpoint: return a forced tool_use with the object.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{
                "type": "tool_use",
                "id": "t1",
                "name": "extract_data",
                "input": { "title": "Widget", "price": "$9" }
            }],
            "usage": { "input_tokens": 12, "output_tokens": 6 }
        })))
        .mount(&mock)
        .await;

    let mut config: AppConfig = toml::from_str(
        r#"
[extraction.llm]
api_key = "test-key"
provider = "anthropic"
model = "claude-test"
"#,
    )
    .unwrap();
    config.extraction.llm.as_mut().unwrap().base_url = Some(mock.uri());
    let state = AppState::new(config).expect("AppState::new failed");
    let server = TestServer::new(create_app(state));

    let page_url = format!("{}/page", mock.uri());
    let start = server
        .post("/v1/extract")
        .json(&json!({
            "urls": [page_url],
            "schema": {
                "type": "object",
                "properties": { "title": {"type":"string"}, "price": {"type":"string"} },
                "required": ["title"]
            }
        }))
        .await;
    start.assert_status_ok();
    let id = start.json::<Value>()["id"].as_str().unwrap().to_string();

    // Poll to terminal (bounded so a hang fails instead of spinning forever).
    let mut results = Value::Null;
    for _ in 0..2000 {
        let r = server.get(&format!("/v1/extract/{id}")).await;
        let body: Value = r.json();
        let Some(status) = body["status"].as_str() else {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            continue;
        };
        match status {
            "completed" => {
                results = body["results"].clone();
                break;
            }
            "failed" => panic!("extract job failed: {body}"),
            _ => tokio::time::sleep(std::time::Duration::from_millis(5)).await,
        }
    }

    assert_eq!(results[0]["url"], json!(page_url));
    assert_eq!(results[0]["status"], json!("completed"));
    assert_eq!(results[0]["data"]["title"], json!("Widget"));
    assert_eq!(results[0]["data"]["price"], json!("$9"));
    assert!(results[0]["llmUsage"]["totalTokens"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn v1_capabilities_extract_supported_tracks_the_llm_it_needs() {
    // `test_app()` has no [extraction.llm]. The route IS mounted, but it rejects
    // a request that brings no llmApiKey — so `supported` must be false. This
    // used to assert `true`, which was exactly the lie: a caller reading it
    // would enable extract and have every keyless request 400.
    let s = test_app();
    let r = s.get("/v1/capabilities").await;
    r.assert_status_ok();
    let body: Value = r.json();
    assert_eq!(body["extract"]["supported"], json!(false));
    // Honest capability: this build implements `basis`, so it says so even on a
    // keyless deploy that reports supported:false. A client that gates its
    // evidence UI on this flag must not be told `false` by a binary that would
    // honour the flag once a key is present.
    assert_eq!(body["extract"]["perFieldAttribution"], json!(true));
    assert!(body["extract"]["maxUrls"].is_number());
    // The per-leg output cap a budget estimator pins its worst case to
    // (charter 5.3): reported, not assumed. Never a 0.
    let max_out = body["extract"]["maxOutputTokens"].as_u64().unwrap();
    assert!(
        max_out > 0,
        "maxOutputTokens must be reported, got {max_out}"
    );

    // Prove the advertisement, rather than trusting it: the same instance really
    // does refuse a well-formed keyless extract.
    let r = s
        .post("/v1/extract")
        .json(&json!({"urls": ["https://example.com"], "prompt": "get the title"}))
        .await;
    r.assert_status(StatusCode::BAD_REQUEST);
    assert!(
        r.text().contains("requires an LLM"),
        "extract must refuse a keyless request when it advertises supported:false, got: {}",
        r.text()
    );
}
