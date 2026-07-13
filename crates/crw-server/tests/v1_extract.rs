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
use crw_server::state::{AppState, ExtractRecord, ExtractStatus, PreparedUrl, UrlResult};
use serde_json::{Value, json};
use std::time::{Duration, Instant, SystemTime};
use uuid::Uuid;

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
async fn v1_extract_cancel_rejects_malformed_and_unknown_ids() {
    let s = test_app();
    s.delete("/v1/extract/not-a-uuid")
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    s.delete("/v1/extract/00000000-0000-0000-0000-000000000000")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn expired_extract_ids_are_immediate_404_for_v1_get_delete_and_v2_get() {
    let mut config: AppConfig = toml::from_str("").unwrap();
    config.crawler.job_ttl_secs = 1;
    let state = AppState::new(config).unwrap();
    let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
    let mut expired = seeded_record(&["https://expired.example"], None);
    expired.created_at = Instant::now() - Duration::from_secs(2);
    {
        let mut jobs = state.extract_jobs.write().await;
        for id in ids {
            jobs.insert(id, expired.clone());
        }
    }
    let server = TestServer::new(create_app(state.clone()));

    server
        .get(&format!("/v1/extract/{}", ids[0]))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .delete(&format!("/v1/extract/{}", ids[1]))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    server
        .get(&format!("/v2/extract/{}", ids[2]))
        .await
        .assert_status(StatusCode::NOT_FOUND);
    assert!(state.extract_jobs.read().await.is_empty());
}

fn pending_slot(url: &str) -> UrlResult {
    UrlResult {
        url: url.into(),
        status: ExtractStatus::Processing,
        data: None,
        error: None,
        llm_usage: None,
        basis: None,
        basis_warnings: Vec::new(),
        llm_input_hash: None,
    }
}

fn seeded_record(urls: &[&str], claimed_index: Option<usize>) -> ExtractRecord {
    ExtractRecord {
        status: ExtractStatus::Processing,
        data: None,
        per_url: urls.iter().map(|url| pending_slot(url)).collect(),
        tokens_used: 0,
        credits_used: 0,
        error: None,
        created_at: Instant::now(),
        expires_at: SystemTime::now() + Duration::from_secs(3_600),
        claimed_index,
    }
}

#[tokio::test]
async fn cancel_before_dispatch_is_terminal_ordered_and_idempotent() {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).unwrap();
    let id = Uuid::new_v4();
    state.extract_jobs.write().await.insert(
        id,
        seeded_record(&["https://a.example", "https://b.example"], None),
    );
    let server = TestServer::new(create_app(state));

    let first: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(first["status"], "cancelled");
    assert_eq!(first["results"].as_array().unwrap().len(), 2);
    assert_eq!(first["results"][0]["url"], "https://a.example");
    assert_eq!(first["results"][1]["url"], "https://b.example");
    for result in first["results"].as_array().unwrap() {
        assert_eq!(result["status"], "cancelled");
        assert_eq!(result.as_object().unwrap().len(), 2);
    }

    let repeated: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(repeated, first, "repeated DELETE returns persisted state");
    let get: Value = server.get(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(get, first, "GET and DELETE share the canonical serializer");
}

#[tokio::test]
async fn terminal_envelope_uses_the_persisted_expiry_without_recomputation() {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).unwrap();
    let id = Uuid::new_v4();
    let mut record = seeded_record(&["https://done.example"], None);
    record.status = ExtractStatus::Completed;
    record.per_url[0].status = ExtractStatus::Completed;
    record.per_url[0].data = Some(json!({"done": true}));
    state.extract_jobs.write().await.insert(id, record);
    let server = TestServer::new(create_app(state.clone()));

    let first: Value = server.get(&format!("/v1/extract/{id}")).await.json();
    // Move the monotonic admission time without changing the persisted wall
    // expiry. The old read-time derivation would shift expiresAt by 10 seconds.
    state
        .extract_jobs
        .write()
        .await
        .get_mut(&id)
        .unwrap()
        .created_at = Instant::now() - Duration::from_secs(10);
    let repeated: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(repeated, first);
}

#[tokio::test]
async fn claimed_slot_holds_cancelling_barrier_until_its_result_persists() {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).unwrap();
    let id = Uuid::new_v4();
    state.extract_jobs.write().await.insert(
        id,
        seeded_record(
            &["https://claimed.example", "https://pending.example"],
            Some(0),
        ),
    );
    let server = TestServer::new(create_app(state.clone()));

    let cancelling: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(cancelling["status"], "cancelling");
    assert_eq!(cancelling["results"][0]["status"], "processing");
    assert_eq!(cancelling["results"][1]["status"], "processing");
    let repeated: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(repeated, cancelling);

    // Model the worker's atomic final write for the already claimed URL.
    {
        let mut jobs = state.extract_jobs.write().await;
        let rec = jobs.get_mut(&id).unwrap();
        rec.per_url[0].status = ExtractStatus::Completed;
        rec.per_url[0].data = Some(json!({"title": "persisted"}));
        rec.tokens_used = 9;
        rec.credits_used = 1;
        rec.claimed_index = None;
    }

    let terminal: Value = server.delete(&format!("/v1/extract/{id}")).await.json();
    assert_eq!(terminal["status"], "cancelled");
    assert_eq!(terminal["results"][0]["status"], "completed");
    assert_eq!(terminal["results"][0]["data"]["title"], "persisted");
    assert_eq!(terminal["results"][1]["status"], "cancelled");
    assert_eq!(terminal["tokensUsed"], 9);
    assert_eq!(terminal["creditsUsed"], 1);
}

#[tokio::test]
async fn delete_never_rewrites_completed_or_failed_terminal_state() {
    let config: AppConfig = toml::from_str("").unwrap();
    let state = AppState::new(config).unwrap();
    let completed_id = Uuid::new_v4();
    let failed_id = Uuid::new_v4();
    let mut completed = seeded_record(&["https://done.example"], None);
    completed.status = ExtractStatus::Completed;
    completed.per_url[0].status = ExtractStatus::Completed;
    completed.per_url[0].data = Some(json!({"ok": true}));
    let mut failed = seeded_record(&["https://failed.example"], None);
    failed.status = ExtractStatus::Failed;
    failed.per_url[0].status = ExtractStatus::Failed;
    failed.per_url[0].error = Some("failed".into());
    failed.error = Some("failed".into());
    {
        let mut jobs = state.extract_jobs.write().await;
        jobs.insert(completed_id, completed);
        jobs.insert(failed_id, failed);
    }
    let server = TestServer::new(create_app(state));

    let completed: Value = server
        .delete(&format!("/v1/extract/{completed_id}"))
        .await
        .json();
    assert_eq!(completed["status"], "completed");
    assert_eq!(completed["results"][0]["data"]["ok"], true);
    let failed: Value = server
        .delete(&format!("/v1/extract/{failed_id}"))
        .await
        .json();
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "failed");
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
async fn worker_cancel_after_persisted_result_settles_claim_and_stops_next_dispatch() {
    use crw_core::types::{OutputFormat, ScrapeRequest};
    use std::time::Duration;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    unsafe {
        std::env::set_var("CRW_ALLOW_LOOPBACK_FOR_TESTS", "1");
    }
    let mock = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/page-a"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string("<html><body><h1>A</h1></body></html>"),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/page-b"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("<h1>B</h1>")
                .set_delay(Duration::from_millis(75)),
        )
        .mount(&mock)
        .await;
    Mock::given(method("GET"))
        .and(path("/page-c"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<h1>C</h1>"))
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "content": [{
                "type": "tool_use",
                "id": "t1",
                "name": "extract_data",
                "input": { "title": "A" }
            }],
            "usage": { "input_tokens": 7, "output_tokens": 3 }
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
    let state = AppState::new(config).unwrap();
    let entries = vec![
        PreparedUrl {
            url: format!("{}/page-a", mock.uri()),
            preflight_error: None,
        },
        PreparedUrl {
            url: format!("{}/page-b", mock.uri()),
            preflight_error: None,
        },
        PreparedUrl {
            url: format!("{}/page-c", mock.uri()),
            preflight_error: None,
        },
    ];
    let template = ScrapeRequest {
        formats: vec![OutputFormat::Json],
        json_schema: Some(json!({
            "type": "object",
            "properties": { "title": {"type": "string"} }
        })),
        ..Default::default()
    };
    let id = state.start_extract_job(entries, template).await;

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let jobs = state.extract_jobs.read().await;
            if jobs[&id].claimed_index == Some(1)
                && jobs[&id].per_url[0].status == ExtractStatus::Completed
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second URL was never claimed after the first result persisted");
    let cancelling = state.cancel_extract_job(id).await.unwrap();
    assert_eq!(cancelling.status, ExtractStatus::Cancelling);

    let terminal = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let rec = state.extract_jobs.read().await[&id].clone();
            if rec.status == ExtractStatus::Cancelled {
                break rec;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancellation barrier never reached terminal");
    assert_eq!(terminal.per_url.len(), 3);
    assert_eq!(terminal.per_url[0].status, ExtractStatus::Completed);
    assert_eq!(terminal.per_url[0].data, Some(json!({"title": "A"})));
    assert_eq!(terminal.per_url[1].status, ExtractStatus::Completed);
    assert_eq!(terminal.per_url[2].status, ExtractStatus::Cancelled);
    assert!(terminal.tokens_used > 0);
    assert_eq!(terminal.credits_used, 2);

    let requests = mock.received_requests().await.unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.url.path() == "/page-a")
    );
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/page-c"),
        "cancellation must prevent the next URL dispatch"
    );
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
