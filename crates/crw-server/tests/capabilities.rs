//! Integration tests for `GET /v1/capabilities`.
//!
//! The point of these tests is ANTI-LIE: every advertised capability must be
//! backed by the build feature / config that actually enables it. A test here
//! fails if the endpoint claims something this instance cannot do.

use axum_test::TestServer;
use crw_core::config::AppConfig;
use crw_server::app::create_app;
use crw_server::state::AppState;
use serde_json::{Value, json};

fn app_from(toml_str: &str) -> TestServer {
    let config: AppConfig = toml::from_str(toml_str).unwrap();
    let state = AppState::new(config).expect("AppState::new failed");
    TestServer::new(create_app(state))
}

async fn caps(server: &TestServer) -> Value {
    let r = server.get("/v1/capabilities").await;
    r.assert_status_ok();
    r.json()
}

/// Renderer tiers that can capture a screenshot. Mirrors
/// `crw_renderer::renderer_can_screenshot`, which the handler and the scrape
/// path share — asserted against it below so the two cannot drift.
fn can_screenshot(name: &str) -> bool {
    crw_renderer::renderer_can_screenshot(name)
}

// ── Search: answer / summarizeResults are never advertised when off ──────────

#[tokio::test]
async fn search_off_reports_unsupported_and_no_answer() {
    // Default config has no searxng_url → state.searxng = None.
    let body = caps(&app_from("")).await;

    assert_eq!(body["search"]["supported"], json!(false));
    assert_eq!(
        body["search"]["answer"],
        json!(false),
        "answer must not be advertised when search is off"
    );
    assert_eq!(body["search"]["summarizeResults"], json!(false));
}

#[tokio::test]
async fn search_on_without_server_llm_key_reports_no_answer() {
    // Search configured but no [extraction.llm] → answering requires a
    // caller-supplied llmApiKey, so the instance cannot answer on its own.
    let body = caps(&app_from(
        r#"
[search]
enabled = true
searxng_url = "http://127.0.0.1:18080"
"#,
    ))
    .await;

    assert_eq!(body["search"]["supported"], json!(true));
    assert_eq!(body["llm"]["serverKeyConfigured"], json!(false));
    assert_eq!(
        body["search"]["answer"],
        json!(false),
        "answer must not be advertised without a server LLM key"
    );
    assert_eq!(body["search"]["summarizeResults"], json!(false));
}

#[tokio::test]
async fn search_on_with_server_llm_key_reports_answer() {
    let body = caps(&app_from(
        r#"
[search]
enabled = true
searxng_url = "http://127.0.0.1:18080"

[extraction.llm]
provider = "anthropic"
api_key = "sk-test"
model = "claude-sonnet-4-20250514"
"#,
    ))
    .await;

    assert_eq!(body["search"]["supported"], json!(true));
    assert_eq!(body["llm"]["serverKeyConfigured"], json!(true));
    assert_eq!(body["search"]["answer"], json!(true));
    assert_eq!(body["search"]["summarizeResults"], json!(true));
}

#[tokio::test]
async fn search_disabled_flag_overrides_configured_url() {
    let body = caps(&app_from(
        r#"
[search]
enabled = false
searxng_url = "http://127.0.0.1:18080"
"#,
    ))
    .await;

    assert_eq!(body["search"]["supported"], json!(false));
    assert_eq!(body["search"]["answer"], json!(false));
}

// ── Screenshot / renderers: advertised only when a capable tier exists ───────

#[tokio::test]
async fn screenshot_matches_the_renderers_actually_constructed() {
    let body = caps(&app_from("")).await;

    let available: Vec<String> = serde_json::from_value(body["renderers"]["available"].clone())
        .expect("renderers.available must be a string array");
    let capable = available.iter().any(|n| can_screenshot(n));

    // The invariant: the advertised screenshot capability is exactly "a
    // constructed renderer can capture". Faking either side breaks this.
    assert_eq!(
        body["screenshot"]["supported"],
        json!(capable),
        "screenshot.supported must equal 'a constructed renderer can capture'; \
         renderers.available = {available:?}"
    );
    assert_eq!(
        body["screenshot"]["fullPage"],
        json!(capable),
        "fullPage capture has the same gate as screenshot capture"
    );

    // The formats list must agree with the screenshot gate.
    let formats: Vec<String> =
        serde_json::from_value(body["formats"]["supported"].clone()).unwrap();
    assert_eq!(
        formats.iter().any(|f| f == "screenshot"),
        capable,
        "the `screenshot` format must be listed iff a capable renderer exists"
    );
}

/// Without the `cdp` feature no CDP tier is constructable, so a screenshot can
/// never be produced — and must never be advertised.
#[cfg(not(feature = "cdp"))]
#[tokio::test]
async fn cdp_less_build_never_advertises_screenshot() {
    // Even with chrome fully configured, a build without `cdp` constructs no
    // renderer at all.
    let body = caps(&app_from(
        r#"
[renderer]
mode = "auto"

[renderer.chrome]
ws_url = "ws://127.0.0.1:9222"
"#,
    ))
    .await;

    let available: Vec<String> =
        serde_json::from_value(body["renderers"]["available"].clone()).unwrap();
    assert!(
        available.is_empty(),
        "a build without the `cdp` feature constructs no JS renderer, got {available:?}"
    );
    assert_eq!(body["screenshot"]["supported"], json!(false));
    assert_eq!(body["screenshot"]["fullPage"], json!(false));

    let formats: Vec<String> =
        serde_json::from_value(body["formats"]["supported"].clone()).unwrap();
    assert!(!formats.iter().any(|f| f == "screenshot"));
}

/// With `cdp` compiled in, the capability follows the CONFIG: a configured
/// chrome tier can capture, an unconfigured one cannot.
#[cfg(feature = "cdp")]
#[tokio::test]
async fn cdp_build_screenshot_follows_config() {
    let with_chrome = caps(&app_from(
        r#"
[renderer]
mode = "auto"

[renderer.chrome]
ws_url = "ws://127.0.0.1:9222"
"#,
    ))
    .await;
    let available: Vec<String> =
        serde_json::from_value(with_chrome["renderers"]["available"].clone()).unwrap();
    assert!(
        available.iter().any(|n| n == "chrome"),
        "a configured chrome tier must be advertised, got {available:?}"
    );
    assert_eq!(with_chrome["screenshot"]["supported"], json!(true));

    // Same build, no renderer endpoint configured → nothing constructed.
    let no_renderer = caps(&app_from(
        r#"
[renderer]
mode = "none"
"#,
    ))
    .await;
    let available: Vec<String> =
        serde_json::from_value(no_renderer["renderers"]["available"].clone()).unwrap();
    assert!(
        available.is_empty(),
        "mode=none constructs no renderer, got {available:?}"
    );
    assert_eq!(
        no_renderer["screenshot"]["supported"],
        json!(false),
        "screenshot must be config-derived, not feature-derived"
    );
}

/// LightPanda cannot capture (its CDP stub returns a ~30-byte image), so a
/// lightpanda-only instance must NOT advertise screenshots.
#[cfg(feature = "cdp")]
#[tokio::test]
async fn lightpanda_only_instance_does_not_advertise_screenshot() {
    let body = caps(&app_from(
        r#"
[renderer]
mode = "lightpanda"

[renderer.lightpanda]
ws_url = "ws://127.0.0.1:9223"
"#,
    ))
    .await;

    let available: Vec<String> =
        serde_json::from_value(body["renderers"]["available"].clone()).unwrap();
    assert_eq!(available, vec!["lightpanda".to_string()]);
    assert_eq!(
        body["screenshot"]["supported"],
        json!(false),
        "lightpanda cannot capture a screenshot — advertising it would 500 every request"
    );
}

// ── Documents: parsers/upload follow the pdf feature + config ────────────────

#[tokio::test]
async fn documents_follow_pdf_feature_and_config() {
    let enabled = caps(&app_from("")).await;
    let parsers: Vec<String> =
        serde_json::from_value(enabled["documents"]["parsers"].clone()).unwrap();
    assert_eq!(
        !parsers.is_empty(),
        crw_extract::pdf::PDF_SUPPORTED,
        "a parser may only be advertised when the pdf feature is compiled in"
    );
    assert_eq!(
        enabled["documents"]["fileUpload"]["supported"],
        json!(crw_extract::pdf::PDF_SUPPORTED)
    );

    let disabled = caps(&app_from(
        r#"
[document]
enabled = false
"#,
    ))
    .await;
    assert_eq!(disabled["documents"]["parsers"], json!([]));
    assert_eq!(
        disabled["documents"]["fileUpload"]["supported"],
        json!(false)
    );
    assert_eq!(disabled["documents"]["fileUpload"]["types"], json!([]));
}

// ── Limits: echo the effective config, never a hardcoded literal ─────────────

#[tokio::test]
async fn limits_echo_a_non_default_config() {
    let body = caps(&app_from(
        r#"
[crawler]
max_batch_urls = 77
max_extract_urls = 9

[search]
default_limit = 3
max_limit = 11

[document]
max_upload_bytes = 1048576
"#,
    ))
    .await;

    assert_eq!(body["limits"]["maxBatchUrls"], json!(77));
    assert_eq!(body["limits"]["maxExtractUrls"], json!(9));
    assert_eq!(body["limits"]["searchDefaultLimit"], json!(3));
    assert_eq!(body["limits"]["searchMaxLimit"], json!(11));
    assert_eq!(body["limits"]["maxUploadBytes"], json!(1_048_576));
    // The extract cap is reported in both places — they must agree.
    assert_eq!(body["extract"]["maxUrls"], json!(9));
    assert_eq!(
        body["documents"]["fileUpload"]["maxBytes"],
        json!(1_048_576),
        "the advertised upload cap must be the enforced one"
    );
}

#[tokio::test]
async fn upload_cap_is_clamped_to_the_hard_ceiling() {
    // An operator cannot raise the cap past the in-memory ceiling, so the
    // advertised value must be the clamped (enforced) one, not the raw knob.
    let body = caps(&app_from(
        r#"
[document]
max_upload_bytes = 2000000000
"#,
    ))
    .await;

    assert_eq!(
        body["limits"]["maxUploadBytes"],
        json!(crw_server::routes::v2::parse::MAX_UPLOAD_BYTES)
    );
}

/// A well-formed multipart upload carrying `payload_len` bytes in a PDF field.
fn multipart_upload(payload_len: usize) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(
        b"--BOUNDARY\r\n\
          Content-Disposition: form-data; name=\"file\"; filename=\"big.pdf\"\r\n\
          Content-Type: application/pdf\r\n\r\n",
    );
    body.extend_from_slice(&vec![b'a'; payload_len]);
    body.extend_from_slice(b"\r\n--BOUNDARY--\r\n");
    body
}

async fn upload_error(server: &TestServer, payload_len: usize) -> String {
    server
        .post("/firecrawl/v2/parse")
        .bytes(multipart_upload(payload_len).into())
        .add_header("content-type", "multipart/form-data; boundary=BOUNDARY")
        .expect_failure()
        .await
        .text()
}

#[tokio::test]
async fn advertised_upload_cap_is_the_one_enforced() {
    // The advertised cap must not be decorative. With a small
    // `max_upload_bytes` the body-limit layer cuts the multipart stream short,
    // so the upload is refused as an unreadable form...
    let tight = app_from(
        r#"
[document]
max_upload_bytes = 1024
"#,
    );
    assert_eq!(caps(&tight).await["limits"]["maxUploadBytes"], json!(1024));

    let tight_err = upload_error(&tight, 4096).await;
    assert!(
        tight_err.contains("invalid multipart form"),
        "an upload over the advertised cap must be cut off by the body limit, got: {tight_err}"
    );

    // ...while under the default cap the SAME bytes are read in full and fail
    // later on content. That contrast proves the rejection above was the
    // advertised size cap doing its job, not a malformed request.
    let roomy_err = upload_error(&app_from(""), 4096).await;
    assert!(
        !roomy_err.contains("invalid multipart form"),
        "under the default cap the same upload must be readable, got: {roomy_err}"
    );
}

// ── LLM: providers come from the dispatcher; formats declare their gates ─────

#[tokio::test]
async fn llm_providers_match_the_dispatcher() {
    let body = caps(&app_from("")).await;
    let providers: Vec<String> = serde_json::from_value(body["llm"]["providers"].clone()).unwrap();

    assert_eq!(
        providers,
        crw_extract::llm::SUPPORTED_PROVIDERS
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>(),
        "the advertised providers must be exactly the ones dispatch accepts"
    );
    for p in &providers {
        assert!(
            crw_extract::llm::is_supported_provider(p),
            "advertised provider `{p}` is rejected by the dispatcher"
        );
    }
}

#[tokio::test]
async fn llm_required_formats_are_declared() {
    let body = caps(&app_from("")).await;

    let supported: Vec<String> =
        serde_json::from_value(body["formats"]["supported"].clone()).unwrap();
    let llm_required: Vec<String> =
        serde_json::from_value(body["formats"]["llmRequired"].clone()).unwrap();

    // Anything credential-gated must be declared, and must be a real format.
    assert!(llm_required.iter().any(|f| f == "json"));
    assert!(llm_required.iter().any(|f| f == "summary"));
    for f in &llm_required {
        assert!(
            supported.contains(f),
            "llmRequired lists `{f}`, which is not in formats.supported"
        );
    }

    // changeTracking's json mode is LLM-backed; gitDiff is deterministic.
    let modes: Vec<String> =
        serde_json::from_value(body["formats"]["changeTrackingModes"].clone()).unwrap();
    let modes_llm: Vec<String> =
        serde_json::from_value(body["formats"]["changeTrackingModesLlmRequired"].clone()).unwrap();
    assert_eq!(modes_llm, vec!["json".to_string()]);
    for m in &modes_llm {
        assert!(modes.contains(m));
    }
    assert!(
        !modes_llm.iter().any(|m| m == "gitDiff"),
        "gitDiff is deterministic and needs no LLM"
    );
}

// ── Extract: `supported` tracks the LLM the route actually needs ─────────────

/// A config with a server LLM key, which is what `/v1/extract` needs to run a
/// request that brings no key of its own.
const SERVER_LLM_KEY: &str = r#"
[extraction.llm]
provider = "openai"
api_key = "sk-test"
model = "gpt-4o-mini"
"#;

#[tokio::test]
async fn extract_not_supported_without_an_llm() {
    // No [extraction.llm]: /v1/extract rejects a keyless request outright, so
    // advertising `supported: true` would be a lie the caller acts on.
    let body = caps(&app_from("")).await;
    assert_eq!(body["extract"]["supported"], json!(false));
    assert_eq!(body["llm"]["serverKeyConfigured"], json!(false));
}

#[tokio::test]
async fn extract_supported_with_a_server_llm_key() {
    let body = caps(&app_from(SERVER_LLM_KEY)).await;
    assert_eq!(body["extract"]["supported"], json!(true));
    assert_eq!(body["llm"]["serverKeyConfigured"], json!(true));
}

#[tokio::test]
async fn extract_not_supported_when_the_byok_header_guard_is_on() {
    // The header guard makes the SERVER key insufficient: /v1/extract rejects a
    // request that carries no llmApiKey even though a server key exists. So the
    // capability must go false while serverKeyConfigured stays true.
    let body = caps(&app_from(&format!(
        "{SERVER_LLM_KEY}require_byok_header = \"x-tenant-llm-key\"\n"
    )))
    .await;

    assert_eq!(body["llm"]["serverKeyConfigured"], json!(true));
    assert_eq!(body["llm"]["requireByokHeader"], json!("x-tenant-llm-key"));
    assert_eq!(
        body["extract"]["supported"],
        json!(false),
        "the BYOK header guard rejects keyless extract, so it must not be advertised"
    );
}

// ── The upload endpoint must be a path every surface actually serves ─────────

#[tokio::test]
async fn advertised_upload_endpoint_is_mounted_on_every_surface() {
    let server = app_from("");
    let endpoint = caps(&server).await["documents"]["fileUpload"]["endpoint"]
        .as_str()
        .expect("endpoint must be a string")
        .to_string();

    // Not a 404: the advertised path is really routed. (An empty body is a 4xx
    // on content, which is fine — we are only proving the route exists.)
    for path in [endpoint.clone(), format!("/firecrawl{endpoint}")] {
        let status = server.post(&path).await.status_code();
        assert_ne!(
            status,
            axum::http::StatusCode::NOT_FOUND,
            "the advertised upload endpoint `{path}` is not mounted"
        );
    }
}

#[tokio::test]
async fn renderer_mode_serializes_as_a_lowercase_string() {
    // The SaaS compares this against a string; a Rust variant name would break it.
    assert_eq!(
        caps(&app_from("")).await["renderers"]["mode"],
        json!("auto")
    );
    let body = caps(&app_from("[renderer]\nmode = \"none\"\n")).await;
    assert_eq!(body["renderers"]["mode"], json!("none"));
    assert_eq!(
        body["renderers"]["available"],
        json!([]),
        "mode=none constructs no JS renderer tier"
    );
    assert_eq!(body["screenshot"]["supported"], json!(false));
}

// ── Boundary guard: `basis` is not this workstream's to flip ─────────────────

#[tokio::test]
async fn per_field_attribution_stays_false_until_basis_ships() {
    let body = caps(&app_from("")).await;
    assert_eq!(
        body["extract"]["perFieldAttribution"],
        json!(false),
        "/v1/extract rejects `basis` today — advertising it would be a lie"
    );
}

// ── The v2 alias serves the identical document ──────────────────────────────

#[tokio::test]
async fn v2_alias_matches_v1() {
    let server = app_from("");
    let v1: Value = server.get("/v1/capabilities").await.json();
    let v2: Value = server.get("/v2/capabilities").await.json();
    assert_eq!(v1, v2);
}
