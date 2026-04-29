use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use axum::routing::get;
use crw_renderer::http_only::HttpFetcher;
use crw_renderer::traits::PageFetcher;
use flate2::Compression;
use flate2::write::GzEncoder;
use tokio::net::TcpListener;

async fn spawn_server() -> String {
    let app = Router::new()
        .route("/gzip", get(gzip_handler))
        .route("/brotli", get(brotli_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    format!("http://{}", addr)
}

fn gzip_bytes(body: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(body).unwrap();
    encoder.finish().unwrap()
}

fn brotli_bytes(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut writer = brotli::CompressorWriter::new(&mut out, 4096, 5, 22);
        writer.write_all(body).unwrap();
    }
    out
}

async fn gzip_handler() -> impl axum::response::IntoResponse {
    let body = gzip_bytes(b"<html><body><h1>gzip works</h1><p>decoded</p></body></html>");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_ENCODING, "gzip"),
        ],
        body,
    )
}

async fn brotli_handler() -> impl axum::response::IntoResponse {
    let body = brotli_bytes(b"<html><body><h1>brotli works</h1><p>decoded</p></body></html>");
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_ENCODING, "br"),
        ],
        body,
    )
}

#[tokio::test]
async fn http_fetcher_decodes_gzip_and_brotli_responses() {
    let base_url = spawn_server().await;
    let fetcher = HttpFetcher::new("crw-test", None, true);

    let gzip_result = fetcher
        .fetch(&format!("{base_url}/gzip"), &HashMap::new(), None)
        .await
        .unwrap();
    assert!(gzip_result.html.contains("gzip works"));
    assert!(!gzip_result.html.contains('\u{1f}'));

    let brotli_result = fetcher
        .fetch(&format!("{base_url}/brotli"), &HashMap::new(), None)
        .await
        .unwrap();
    assert!(brotli_result.html.contains("brotli works"));
    assert!(!brotli_result.html.contains('\u{1f}'));
}

// ── Retry behavior ────────────────────────────────────────────────────

/// Handler that returns 502 the first N times it's called, then 200 after.
async fn flaky_502_handler(
    State(counter): State<Arc<AtomicU32>>,
) -> impl axum::response::IntoResponse {
    let n = counter.fetch_add(1, Ordering::SeqCst);
    if n == 0 {
        (StatusCode::BAD_GATEWAY, "upstream").into_response()
    } else {
        (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            "<html><body><h1>retry success</h1></body></html>",
        )
            .into_response()
    }
}

async fn always_502_handler() -> impl axum::response::IntoResponse {
    (StatusCode::BAD_GATEWAY, "still broken")
}

async fn always_500_handler() -> impl axum::response::IntoResponse {
    // 500 is NOT in the retry list — should fail fast.
    (StatusCode::INTERNAL_SERVER_ERROR, "permanent")
}

async fn spawn_retry_server() -> (String, Arc<AtomicU32>) {
    let counter = Arc::new(AtomicU32::new(0));
    let app = Router::new()
        .route("/flaky", get(flaky_502_handler))
        .with_state(counter.clone())
        .route("/always502", get(always_502_handler))
        .route("/always500", get(always_500_handler));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{}", addr), counter)
}

#[tokio::test]
async fn http_fetcher_retries_502_then_succeeds() {
    let (base, counter) = spawn_retry_server().await;
    let fetcher = HttpFetcher::new("crw-test", None, false);

    let result = fetcher
        .fetch(&format!("{base}/flaky"), &HashMap::new(), None)
        .await
        .expect("retry should succeed");

    assert_eq!(result.status_code, 200);
    assert!(result.html.contains("retry success"));
    // Two attempts: the initial 502 + the retry.
    assert_eq!(counter.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn http_fetcher_gives_up_after_one_retry_on_502() {
    let (base, _counter) = spawn_retry_server().await;
    let fetcher = HttpFetcher::new("crw-test", None, false);

    // Both attempts return 502; we surface the final status to the caller
    // without erroring (caller decides what to do with non-2xx).
    let result = fetcher
        .fetch(&format!("{base}/always502"), &HashMap::new(), None)
        .await
        .expect("non-2xx must be returned, not raised");
    assert_eq!(result.status_code, 502);
}

#[tokio::test]
async fn http_fetcher_does_not_retry_500() {
    // 500 is excluded from the retry set — return immediately.
    let (base, _counter) = spawn_retry_server().await;
    let fetcher = HttpFetcher::new("crw-test", None, false);
    let result = fetcher
        .fetch(&format!("{base}/always500"), &HashMap::new(), None)
        .await
        .expect("500 must surface as result");
    assert_eq!(result.status_code, 500);
}

#[tokio::test]
async fn http_fetcher_retries_on_connect_refused() {
    // Pick a port that nothing is listening on. We bind, capture, drop.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let dead_addr = listener.local_addr().unwrap();
    drop(listener);

    let fetcher = HttpFetcher::new("crw-test", None, false);
    // Should retry once and then give up with TargetUnreachable.
    let result = fetcher
        .fetch(&format!("http://{dead_addr}/x"), &HashMap::new(), None)
        .await;
    assert!(
        result.is_err(),
        "expected TargetUnreachable, got {result:?}"
    );
}
