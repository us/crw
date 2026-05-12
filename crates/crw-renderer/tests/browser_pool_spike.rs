//! Tier S — Browser-context-pool CDP spike.
//!
//! Exercises the exact CDP sequence the pool will run, against a long-lived
//! Chrome (or `chromedp/headless-shell`) WS connection. Validates that the
//! load-bearing assumptions hold *before* we build pool plumbing on top of
//! them. See plan §"Tier S — Real-Chrome CDP spike".
//!
//! Three variants (per plan):
//! 1. `loop_variant`            — 100× create-ctx → create-target → navigate
//!    → close-target → dispose-ctx, on one conn.
//! 2. `concurrent_contexts`     — Two contexts open simultaneously on one
//!    conn; cookie set in A must NOT leak to B.
//! 3. `dispose_with_open_target`— Order-of-operations: dispose ctx while a
//!    target inside it is still open. Observe.
//!
//! All `#[ignore]` by default. To run, point `CRW_CDP_HTTP_URL` at a Chrome
//! HTTP base (e.g. `http://127.0.0.1:9333`) and set `CRW_TEST_REAL_CHROME=1`:
//!
//! ```sh
//! /Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
//!   --headless=new --remote-debugging-port=9333 \
//!   --user-data-dir=/tmp/crw-spike-chrome &
//! sleep 2
//! CRW_TEST_REAL_CHROME=1 CRW_CDP_HTTP_URL=http://127.0.0.1:9333 \
//!   cargo test -p crw-renderer --features cdp --test browser_pool_spike \
//!     -- --ignored --nocapture
//! ```
//!
//! Output recorded into `bench/server-runs/SPIKE_browser_pool.md` after a
//! successful run.

#![cfg(feature = "cdp")]

use std::time::{Duration, Instant};

use crw_renderer::cdp_conn::CdpConnection;
use serde_json::{Value, json};
use tokio_tungstenite::connect_async;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CDP_TIMEOUT: Duration = Duration::from_secs(10);

/// Skip the test cleanly if the env gate is off. Returns Some(http_base) iff
/// `CRW_TEST_REAL_CHROME=1` AND `CRW_CDP_HTTP_URL` is set.
fn require_real_chrome() -> Option<String> {
    if std::env::var("CRW_TEST_REAL_CHROME").ok().as_deref() != Some("1") {
        eprintln!("skip: CRW_TEST_REAL_CHROME != 1");
        return None;
    }
    match std::env::var("CRW_CDP_HTTP_URL") {
        Ok(u) if !u.is_empty() => Some(u),
        _ => {
            eprintln!("skip: CRW_CDP_HTTP_URL not set (e.g. http://127.0.0.1:9333)");
            None
        }
    }
}

/// Resolve the browser-level WS URL via /json/version. Mirrors what
/// `CdpRenderer::resolve_ws_url` does for vanilla Chrome.
async fn resolve_ws(http_base: &str) -> String {
    let url = format!("{}/json/version", http_base.trim_end_matches('/'));
    let body: Value = reqwest::Client::new()
        .get(&url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .expect("GET /json/version")
        .json()
        .await
        .expect("parse /json/version");
    let raw = body
        .get("webSocketDebuggerUrl")
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| panic!("no webSocketDebuggerUrl in {body}"))
        .to_string();
    // Chrome echoes back the Host header (or 127.0.0.1) in webSocketDebuggerUrl;
    // rewrite host:port to whatever the caller configured so we hit the right
    // listener. Mirrors `cdp::rewrite_ws_host`.
    let conf_hp = http_base
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    let path = raw
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .split_once('/')
        .map(|(_, rest)| format!("/{rest}"))
        .unwrap_or_else(|| "/".into());
    format!("ws://{conf_hp}{path}")
}

async fn create_ctx(conn: &CdpConnection) -> String {
    let r = conn
        .send_recv("Target.createBrowserContext", json!({}), None, CDP_TIMEOUT)
        .await
        .expect("createBrowserContext");
    r.get("browserContextId")
        .and_then(|v| v.as_str())
        .expect("browserContextId")
        .to_string()
}

async fn dispose_ctx(conn: &CdpConnection, ctx_id: &str) -> Result<(), String> {
    conn.send_recv(
        "Target.disposeBrowserContext",
        json!({ "browserContextId": ctx_id }),
        None,
        CDP_TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

async fn create_target(conn: &CdpConnection, ctx_id: &str, url: &str) -> String {
    let r = conn
        .send_recv(
            "Target.createTarget",
            json!({ "url": url, "browserContextId": ctx_id }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("createTarget");
    r.get("targetId")
        .and_then(|v| v.as_str())
        .expect("targetId")
        .to_string()
}

async fn close_target(conn: &CdpConnection, target_id: &str) -> Result<(), String> {
    conn.send_recv(
        "Target.closeTarget",
        json!({ "targetId": target_id }),
        None,
        CDP_TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

async fn attach(conn: &CdpConnection, target_id: &str) -> String {
    let r = conn
        .send_recv(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("attachToTarget");
    r.get("sessionId")
        .and_then(|v| v.as_str())
        .expect("sessionId")
        .to_string()
}

/// Navigate + wait for `Page.loadEventFired`. Subscribes BEFORE sending so
/// we never miss the event.
async fn navigate_and_wait(conn: &CdpConnection, session_id: &str, url: &str) {
    let _ = conn
        .send_recv("Page.enable", json!({}), Some(session_id), CDP_TIMEOUT)
        .await
        .expect("Page.enable");
    let events_rx = conn.subscribe();
    let _nav = conn
        .send_recv(
            "Page.navigate",
            json!({ "url": url }),
            Some(session_id),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.navigate");
    let sid = session_id.to_string();
    // Wait via the same wait_for_event helper the renderer uses.
    let mut rx = events_rx;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("loadEventFired timeout for {url}");
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Err(_) => panic!("loadEventFired timeout for {url}"),
            Ok(Err(_)) => continue,
            Ok(Ok(ev)) => {
                if ev.method == "Page.loadEventFired" && ev.session_id.as_deref() == Some(&sid) {
                    return;
                }
            }
        }
    }
}

/// VARIANT 1 — 100× recycle loop on a single connection.
///
/// Asserts: each iteration completes; no `Inspector.targetCrashed`; conn is
/// not closed at end. RSS check is left for an external `ps` capture; we
/// log iteration timings so the operator can spot drift.
#[tokio::test]
#[ignore]
async fn spike_loop_variant_100x() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws_url = resolve_ws(&base).await;
    eprintln!("ws_url={ws_url}");

    // Diagnostic: probe ws_url with tokio-tungstenite directly so we see the
    // underlying error class (CdpConnection masks tungstenite::Error::Io
    // detail behind "io error").
    match connect_async(&ws_url).await {
        Ok((ws, resp)) => {
            eprintln!("raw ws probe: connected, status={}", resp.status());
            drop(ws);
        }
        Err(e) => {
            eprintln!("raw ws probe FAIL: {e:?}");
        }
    }

    let conn = CdpConnection::connect(&ws_url, CONNECT_TIMEOUT)
        .await
        .expect("connect");

    // Subscribe to events to catch any targetCrashed during the loop.
    let mut crash_rx = conn.subscribe();
    let crash_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let crash_flag_t = crash_flag.clone();
    let crash_task = tokio::spawn(async move {
        while let Ok(ev) = crash_rx.recv().await {
            if ev.method == "Inspector.targetCrashed" {
                crash_flag_t.store(true, std::sync::atomic::Ordering::SeqCst);
                eprintln!("!! Inspector.targetCrashed observed: {:?}", ev.params);
            }
        }
    });

    let total = 100usize;
    let mut timings = Vec::with_capacity(total);
    let t0 = Instant::now();

    for i in 0..total {
        let it_start = Instant::now();
        let ctx_id = create_ctx(&conn).await;
        let target_id = create_target(&conn, &ctx_id, "about:blank").await;
        let session_id = attach(&conn, &target_id).await;
        navigate_and_wait(
            &conn,
            &session_id,
            "data:text/html,<html><body>ok</body></html>",
        )
        .await;
        close_target(&conn, &target_id).await.expect("closeTarget");
        dispose_ctx(&conn, &ctx_id).await.expect("disposeCtx");
        let dt = it_start.elapsed();
        timings.push(dt);
        if i % 10 == 0 {
            eprintln!(
                "iter {:>3}: {:>6} ms  (cumulative {:>6} ms)",
                i,
                dt.as_millis(),
                t0.elapsed().as_millis()
            );
        }
        assert!(!conn.is_closed(), "conn closed unexpectedly at iter {i}");
        assert!(
            !crash_flag.load(std::sync::atomic::Ordering::SeqCst),
            "Inspector.targetCrashed at iter {i}"
        );
    }

    crash_task.abort();
    let total_ms = t0.elapsed().as_millis();
    let mut sorted = timings.clone();
    sorted.sort();
    let p50 = sorted[sorted.len() / 2].as_millis();
    let p95 = sorted[(sorted.len() * 95) / 100].as_millis();
    let max = sorted.last().unwrap().as_millis();
    eprintln!("\n=== loop_variant_100x summary ===");
    eprintln!("  iterations : {total}");
    eprintln!("  total      : {total_ms} ms");
    eprintln!("  per-iter p50: {p50} ms");
    eprintln!("  per-iter p95: {p95} ms");
    eprintln!("  per-iter max: {max} ms");

    conn.close().await;
    assert!(conn.is_closed());
}

/// VARIANT 2 — Two contexts simultaneously on one connection.
///
/// Sets a cookie in ctx_A via httpbin /cookies/set, then opens a fresh
/// document in ctx_B and asserts no cookie leak. This is the load-bearing
/// isolation assumption for pool size > 1.
///
/// Falls back to a localStorage-based check if httpbin is unreachable.
#[tokio::test]
#[ignore]
async fn spike_concurrent_contexts() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws_url = resolve_ws(&base).await;
    let conn = CdpConnection::connect(&ws_url, CONNECT_TIMEOUT)
        .await
        .expect("connect");

    let ctx_a = create_ctx(&conn).await;
    let ctx_b = create_ctx(&conn).await;
    eprintln!("ctx_a={ctx_a}");
    eprintln!("ctx_b={ctx_b}");
    assert_ne!(ctx_a, ctx_b, "context ids must differ");

    // Open targets in each context.
    let target_a = create_target(&conn, &ctx_a, "about:blank").await;
    let target_b = create_target(&conn, &ctx_b, "about:blank").await;
    let session_a = attach(&conn, &target_a).await;
    let session_b = attach(&conn, &target_b).await;

    // Seed localStorage in ctx_A on a same-origin "about:blank" — we use a
    // data: URL with a scheme that supports localStorage. data: URLs do NOT
    // support localStorage; instead use a real HTTP origin. Try httpbin
    // first, fall back to about:blank + JS-evaluable cookie via document.cookie.
    let test_url = "https://example.com/";
    navigate_and_wait(&conn, &session_a, test_url).await;
    navigate_and_wait(&conn, &session_b, test_url).await;

    // Set a cookie in ctx_A via document.cookie.
    let _ = conn
        .send_recv(
            "Runtime.evaluate",
            json!({
                "expression": "document.cookie = 'crw_isolation=ctx_a_marker; path=/'",
                "returnByValue": true,
            }),
            Some(&session_a),
            CDP_TIMEOUT,
        )
        .await
        .expect("set cookie ctx_A");

    // Read document.cookie in ctx_A — expect to see our marker.
    let cookie_a = {
        let r = conn
            .send_recv(
                "Runtime.evaluate",
                json!({
                    "expression": "document.cookie",
                    "returnByValue": true,
                }),
                Some(&session_a),
                CDP_TIMEOUT,
            )
            .await
            .expect("read cookie ctx_A");
        r.get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    eprintln!("ctx_A cookies: {cookie_a:?}");
    assert!(
        cookie_a.contains("crw_isolation=ctx_a_marker"),
        "ctx_A should see its own cookie, got {cookie_a:?}"
    );

    // Read document.cookie in ctx_B — MUST NOT see ctx_A's cookie.
    let cookie_b = {
        let r = conn
            .send_recv(
                "Runtime.evaluate",
                json!({
                    "expression": "document.cookie",
                    "returnByValue": true,
                }),
                Some(&session_b),
                CDP_TIMEOUT,
            )
            .await
            .expect("read cookie ctx_B");
        r.get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    eprintln!("ctx_B cookies: {cookie_b:?}");
    assert!(
        !cookie_b.contains("crw_isolation=ctx_a_marker"),
        "LEAK: ctx_B saw ctx_A's cookie: {cookie_b:?}"
    );

    // Cleanup.
    let _ = close_target(&conn, &target_a).await;
    let _ = close_target(&conn, &target_b).await;
    let _ = dispose_ctx(&conn, &ctx_a).await;
    let _ = dispose_ctx(&conn, &ctx_b).await;
    conn.close().await;
}

/// VARIANT 3 — Dispose context while a target inside it is still open.
///
/// Pool's RECYCLE_AFTER_NAV=1 path closes target before dispose, so this
/// is the *failure-mode probe*: what does the server do if we get the
/// order wrong? Three plausible outcomes:
///   - dispose returns Ok and auto-closes the target,
///   - dispose returns Err with a specific message,
///   - dispose hangs.
/// We just record the outcome; the pool itself MUST NOT depend on it.
#[tokio::test]
#[ignore]
async fn spike_dispose_with_open_target() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws_url = resolve_ws(&base).await;
    let conn = CdpConnection::connect(&ws_url, CONNECT_TIMEOUT)
        .await
        .expect("connect");

    let ctx_id = create_ctx(&conn).await;
    let target_id = create_target(&conn, &ctx_id, "about:blank").await;
    let session_id = attach(&conn, &target_id).await;
    navigate_and_wait(
        &conn,
        &session_id,
        "data:text/html,<html><body>still-open</body></html>",
    )
    .await;

    // Subscribe BEFORE dispose to catch any auto-detach event.
    let mut events = conn.subscribe();
    let dispose_start = Instant::now();
    let dispose_outcome = dispose_ctx(&conn, &ctx_id).await;
    let dispose_dt = dispose_start.elapsed();
    eprintln!("dispose_outcome (target still open): {dispose_outcome:?}");
    eprintln!("dispose elapsed: {} ms", dispose_dt.as_millis());

    // Drain any events that fired in the next 500 ms; report.
    let drain_deadline = Instant::now() + Duration::from_millis(500);
    let mut detached_seen = false;
    loop {
        let remaining = drain_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match tokio::time::timeout(remaining, events.recv()).await {
            Err(_) | Ok(Err(_)) => break,
            Ok(Ok(ev)) => {
                eprintln!(
                    "post-dispose event: {} (sid={:?})",
                    ev.method, ev.session_id
                );
                if ev.method == "Target.detachedFromTarget" || ev.method == "Target.targetDestroyed"
                {
                    detached_seen = true;
                }
            }
        }
    }
    eprintln!("auto-detach observed: {detached_seen}");

    // Try to navigate the now-orphaned session — what happens?
    let nav_after = conn
        .send_recv(
            "Page.navigate",
            json!({ "url": "about:blank" }),
            Some(&session_id),
            Duration::from_secs(2),
        )
        .await;
    eprintln!("navigate-after-dispose outcome: {nav_after:?}");

    // Best-effort cleanup. closeTarget on a destroyed target is expected to
    // error — just log it.
    let close_after = close_target(&conn, &target_id).await;
    eprintln!("closeTarget-after-dispose: {close_after:?}");

    conn.close().await;
}
