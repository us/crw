use std::collections::HashMap;
use std::io::Write;

use axum::Router;
use axum::http::{StatusCode, header};
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
