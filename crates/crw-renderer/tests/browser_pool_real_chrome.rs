//! Tier 4 — Real-Chrome integration tests for `BrowserContextPool`.
//!
//! All `#[ignore]` by default. Run against a long-lived `chromedp/headless-shell`
//! or vanilla Chrome with `--remote-debugging-port=9333`:
//!
//! ```sh
//! CRW_TEST_REAL_CHROME=1 CRW_CDP_HTTP_URL=http://127.0.0.1:9333 \
//!   cargo test -p crw-renderer --features cdp --test browser_pool_real_chrome \
//!     -- --ignored --nocapture
//! ```
//!
//! Each test owns its pool; teardown calls `pool.shutdown(...)` so the harness
//! never leaks Chrome targets between tests.
//!
//! Test inventory (plan §"Tier 4"):
//!   - T0  cookie isolation per release
//!   - T1  localStorage isolation per release
//!   - T2  200 acquire/release stress + all_slots cap
//!   - T3  conn-close mid-test → next acquire creates fresh
//!   - T4  drain under load
//!   - T4b drain with leaked guard (`mem::forget`) → inflight reaches 0
//!   - T6  closeTarget timeout → slot Dead, dispose skipped
//!   - T7  permit accounting under shutdown

#![cfg(feature = "cdp")]

use std::sync::Arc;
use std::time::{Duration, Instant};

use crw_renderer::browser_pool::{BrowserContextPool, ConnFactory, PoolCfg};
use crw_renderer::cdp_conn::CdpConnection;
use serde_json::{Value, json};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CDP_TIMEOUT: Duration = Duration::from_secs(10);

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

/// Build a pool whose factory connects to the real Chrome under test.
fn build_pool(ws_url: String, size: usize) -> Arc<BrowserContextPool<CdpConnection>> {
    let factory: ConnFactory<CdpConnection> = Arc::new(move || {
        let ws = ws_url.clone();
        Box::pin(async move {
            let conn = CdpConnection::connect(&ws, CONNECT_TIMEOUT).await?;
            Ok(Arc::new(conn))
        })
    });
    BrowserContextPool::new(
        PoolCfg {
            size,
            recycle_after_navs: 1,
            idle_timeout: Duration::from_secs(300),
            health_check_after: Duration::from_secs(60),
            shutdown_drain: Duration::from_secs(10),
            close_target_timeout: Duration::from_secs(2),
            dispose_ctx_timeout: Duration::from_secs(1),
            create_ctx_timeout: Duration::from_secs(1),
        },
        factory,
    )
}

/// One pooled navigation: create target inside the guard's ctx, attach, navigate
/// to `url`, optionally run `js_after_load` and return its String result.
async fn pooled_visit(
    pool: &Arc<BrowserContextPool<CdpConnection>>,
    url: &str,
    js_after_load: Option<&str>,
) -> String {
    let guard = pool.acquire().await.expect("acquire");
    let create = guard
        .conn
        .send_recv(
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": guard.ctx_id }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("createTarget");
    let target_id = create
        .get("targetId")
        .and_then(|v| v.as_str())
        .expect("targetId")
        .to_string();
    guard.record_target(target_id.clone());

    let attach = guard
        .conn
        .send_recv(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("attachToTarget");
    let sid = attach
        .get("sessionId")
        .and_then(|v| v.as_str())
        .expect("sessionId")
        .to_string();

    let _ = guard
        .conn
        .send_recv("Page.enable", json!({}), Some(&sid), CDP_TIMEOUT)
        .await
        .expect("Page.enable");
    let events_rx = guard.conn.subscribe();
    let _ = guard
        .conn
        .send_recv(
            "Page.navigate",
            json!({ "url": url }),
            Some(&sid),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.navigate");

    // Wait for loadEventFired (15s budget).
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
                    break;
                }
            }
        }
    }

    let out = if let Some(js) = js_after_load {
        let r = guard
            .conn
            .send_recv(
                "Runtime.evaluate",
                json!({ "expression": js, "returnByValue": true }),
                Some(&sid),
                CDP_TIMEOUT,
            )
            .await
            .expect("Runtime.evaluate");
        r.get("result")
            .and_then(|v| v.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };

    guard.release().await.expect("release");
    out
}

// ---------------------------------------------------------------------------
// T0 — cookie isolation per release
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn t0_cookie_isolation_per_release() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 2);

    // First visit: set a cookie.
    let after_set = pooled_visit(
        &pool,
        "https://example.com/",
        Some("document.cookie='crw_t0=marker; path=/'; document.cookie"),
    )
    .await;
    assert!(
        after_set.contains("crw_t0=marker"),
        "expected own cookie after set, got {after_set:?}"
    );

    // Second visit: must NOT see the cookie (different browser context).
    let on_second = pooled_visit(&pool, "https://example.com/", Some("document.cookie")).await;
    assert!(
        !on_second.contains("crw_t0=marker"),
        "LEAK: second visit saw prior context's cookie: {on_second:?}"
    );

    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// T1 — localStorage isolation per release
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn t1_local_storage_isolation_per_release() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 2);

    let after_set = pooled_visit(
        &pool,
        "https://example.com/",
        Some("localStorage.setItem('crw_t1','marker'); localStorage.getItem('crw_t1')||''"),
    )
    .await;
    assert_eq!(after_set, "marker", "expected own localStorage after set");

    let on_second = pooled_visit(
        &pool,
        "https://example.com/",
        Some("localStorage.getItem('crw_t1')||''"),
    )
    .await;
    assert_eq!(
        on_second, "",
        "LEAK: second visit saw prior context's localStorage: {on_second:?}"
    );

    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// T2 — 200 acquire/release stress; LIVE conns capped at pool size
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn t2_stress_200_cycles() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool_size = 4;
    let pool = build_pool(ws, pool_size);

    for i in 0..200 {
        // No JS, just navigate + recycle.
        let _ = pooled_visit(&pool, "about:blank", None).await;
        if i % 25 == 0 {
            eprintln!(
                "t2: iter={i:>3} inflight={} idle={}",
                pool.inflight(),
                pool.idle_len()
            );
        }
        assert!(
            pool.inflight() == 0,
            "inflight should be 0 between iterations, got {}",
            pool.inflight()
        );
        assert!(
            pool.idle_len() <= pool_size,
            "idle queue should never exceed pool size, got {}",
            pool.idle_len()
        );
    }

    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// T3 — conn-close mid-test → next acquire reconnects cleanly
// ---------------------------------------------------------------------------
//
// We don't have remote control over the chrome container, so we simulate "ws
// dies" by reaching into a guard's conn and calling close(). The pool's
// health-check on the next stale idle slot is skipped (timer not elapsed),
// so we instead rely on the recycle path to mark the slot dead when CDP
// commands fail on the now-closed conn.
#[tokio::test]
#[ignore]
async fn t3_conn_kill_recovers() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 2);

    // Use the pool once to populate a slot.
    let _ = pooled_visit(&pool, "about:blank", None).await;

    // Acquire, close the conn directly, release — release's recycle steps will
    // fail and mark the slot Dead.
    {
        let guard = pool.acquire().await.expect("acquire #2");
        guard.conn.close().await;
        // Drop the guard via release (it will fail internally but must not panic).
        let _ = guard.release().await;
    }

    // Next acquire must succeed via factory (fresh conn).
    let after = pooled_visit(&pool, "about:blank", None).await;
    let _ = after; // unused
    assert_eq!(pool.inflight(), 0);

    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// T4 — drain under load
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn t4_drain_under_load() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 4);

    // 5 concurrent in-flight requests. Tasks that lose the race against
    // shutdown's `Semaphore::close()` (acquire returns `Shutdown`) just exit;
    // they MUST NOT panic — that would pollute the test log without changing
    // the outcome.
    let mut handles = Vec::new();
    for _ in 0..5 {
        let p = pool.clone();
        handles.push(tokio::spawn(async move {
            // Inline pooled_visit but tolerate Shutdown on acquire.
            let Ok(guard) = p.acquire().await else { return };
            let _ = guard.release().await;
        }));
    }

    // Let them start.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Initiate shutdown with a 10s drain.
    let drain_handle = {
        let p = pool.clone();
        tokio::spawn(async move {
            p.shutdown(Duration::from_secs(10)).await;
        })
    };

    // All in-flight should complete or be drained within deadline.
    for h in handles {
        let _ = h.await;
    }
    drain_handle.await.expect("shutdown joined");

    assert_eq!(pool.inflight(), 0, "inflight must reach 0 after drain");
    assert_eq!(pool.idle_len(), 0, "idle slots must be drained");
}

// ---------------------------------------------------------------------------
// T4b — drain with leaked guard (`mem::forget`) → inflight reaches 0
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn t4b_drain_with_leaked_guard() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 2);

    // Acquire and forget — simulates Drop bypass (e.g. panic-during-Drop).
    let guard = pool.acquire().await.expect("acquire");
    assert_eq!(pool.inflight(), 1);
    std::mem::forget(guard);

    // Shutdown's phase 3 must force-close the conn AND decrement inflight
    // because it wins the terminator CAS for the leaked slot.
    pool.clone().shutdown(Duration::from_secs(5)).await;

    assert_eq!(
        pool.inflight(),
        0,
        "shutdown must reconcile leaked guard's inflight"
    );
}

// ---------------------------------------------------------------------------
// T6 — closeTarget timeout → slot Dead, dispose skipped
// ---------------------------------------------------------------------------
//
// Hard to engineer a "hanging" closeTarget against real Chrome reliably.
// Approach: open a target running an infinite-`beforeunload` dialog. Modern
// Chrome auto-dismisses beforeunload in headless mode, so a more reliable
// trigger is `Page.javaScriptDialogOpening` paired with deliberate non-handling
// — but again, headless suppresses these.
//
// Pragmatic fallback: assert the policy at the unit-test level (already
// covered in `tests/browser_pool_state.rs`), and here just exercise a normal
// close-target-after-load path to confirm no regression on the happy branch.
// The hard-to-engineer branch is documented in the plan as covered by P7
// state-machine tests.
#[tokio::test]
#[ignore]
async fn t6_close_target_normal_path() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 2);

    let _ = pooled_visit(&pool, "about:blank", None).await;
    assert_eq!(pool.inflight(), 0);
    assert!(
        pool.idle_len() >= 1,
        "slot should be back in idle after release"
    );

    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// T7 — permit accounting under shutdown
// ---------------------------------------------------------------------------
//
// Saturate the pool, then call shutdown; in-flight `release()` calls must
// complete normally without leaking permits.
#[tokio::test]
#[ignore]
async fn t7_permit_accounting_under_shutdown() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool_size = 3;
    let pool = build_pool(ws, pool_size);

    // Hold pool_size guards concurrently.
    let mut guards = Vec::new();
    for _ in 0..pool_size {
        guards.push(pool.acquire().await.expect("acquire"));
    }
    assert_eq!(pool.inflight(), pool_size);

    // Release them in parallel while shutdown drains.
    let drain = {
        let p = pool.clone();
        tokio::spawn(async move {
            p.shutdown(Duration::from_secs(10)).await;
        })
    };

    for g in guards {
        let _ = g.release().await;
    }
    drain.await.expect("shutdown joined");

    assert_eq!(pool.inflight(), 0);
}

// ---------------------------------------------------------------------------
// SS — screenshot capture smoke: `Page.captureScreenshot` returns a PNG.
//
// Mirrors the engine's capture path in `cdp.rs::post_navigate_phase`
// (format:"png", captureBeyondViewport, fromSurface) reading the raw base64
// `data` field. Chrome-gated like the rest of this file. A PNG's 8-byte magic
// header (`\x89PNG\r\n\x1a\n`) base64-encodes to the constant prefix
// `iVBORw0KGgo`, so we can assert it without pulling a base64 dep.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn ss_capture_screenshot_returns_png() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 1);

    let guard = pool.acquire().await.expect("acquire");
    let create = guard
        .conn
        .send_recv(
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": guard.ctx_id }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("createTarget");
    let target_id = create
        .get("targetId")
        .and_then(|v| v.as_str())
        .expect("targetId")
        .to_string();
    guard.record_target(target_id.clone());

    let attach = guard
        .conn
        .send_recv(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("attachToTarget");
    let sid = attach
        .get("sessionId")
        .and_then(|v| v.as_str())
        .expect("sessionId")
        .to_string();

    guard
        .conn
        .send_recv("Page.enable", json!({}), Some(&sid), CDP_TIMEOUT)
        .await
        .expect("Page.enable");
    let events_rx = guard.conn.subscribe();
    guard
        .conn
        .send_recv(
            "Page.navigate",
            json!({ "url": "https://example.com/" }),
            Some(&sid),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.navigate");

    let mut rx = events_rx;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("loadEventFired timeout");
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Err(_) => panic!("loadEventFired timeout"),
            Ok(Err(_)) => continue,
            Ok(Ok(ev)) => {
                if ev.method == "Page.loadEventFired" && ev.session_id.as_deref() == Some(&sid) {
                    break;
                }
            }
        }
    }

    let resp = guard
        .conn
        .send_recv(
            "Page.captureScreenshot",
            json!({ "format": "png", "captureBeyondViewport": false, "fromSurface": true }),
            Some(&sid),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.captureScreenshot");
    let b64 = resp
        .get("data")
        .and_then(|v| v.as_str())
        .expect("captureScreenshot returned `data`");
    assert!(!b64.is_empty(), "screenshot base64 must be non-empty");
    assert!(
        b64.starts_with("iVBORw0KGgo"),
        "screenshot must be a PNG (base64 PNG magic prefix), got {:?}",
        &b64[..b64.len().min(16)]
    );

    guard.release().await.expect("release");
    pool.shutdown(Duration::from_secs(5)).await;
}

// ---------------------------------------------------------------------------
// SS — tall full-page screenshot smoke: proves the #161 OOM-guard params
// (clip derived from Page.getLayoutMetrics) are accepted by Chrome and still
// return a PNG, for a page whose scroll height exceeds the 15000px cap that
// would otherwise make `captureBeyondViewport` rasterize the entire page.
// Mirrors `ss_capture_screenshot_returns_png`; drives the raw CDP calls since
// `capture_screenshot`/`full_page_clip` are private methods on `CdpRenderer`.
// ---------------------------------------------------------------------------
#[tokio::test]
#[ignore]
async fn ss_full_page_tall_returns_png() {
    let Some(base) = require_real_chrome() else {
        return;
    };
    let ws = resolve_ws(&base).await;
    let pool = build_pool(ws, 1);

    let guard = pool.acquire().await.expect("acquire");
    let create = guard
        .conn
        .send_recv(
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": guard.ctx_id }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("createTarget");
    let target_id = create
        .get("targetId")
        .and_then(|v| v.as_str())
        .expect("targetId")
        .to_string();
    guard.record_target(target_id.clone());

    let attach = guard
        .conn
        .send_recv(
            "Target.attachToTarget",
            json!({ "targetId": target_id, "flatten": true }),
            None,
            CDP_TIMEOUT,
        )
        .await
        .expect("attachToTarget");
    let sid = attach
        .get("sessionId")
        .and_then(|v| v.as_str())
        .expect("sessionId")
        .to_string();

    guard
        .conn
        .send_recv("Page.enable", json!({}), Some(&sid), CDP_TIMEOUT)
        .await
        .expect("Page.enable");
    let events_rx = guard.conn.subscribe();
    guard
        .conn
        .send_recv(
            "Page.navigate",
            json!({
                "url": "data:text/html,<body style=\"margin:0\"><div style=\"height:40000px;background:linear-gradient(red,blue)\"></div></body>"
            }),
            Some(&sid),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.navigate");

    let mut rx = events_rx;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            panic!("loadEventFired timeout");
        }
        match tokio::time::timeout(remaining, rx.recv()).await {
            Err(_) => panic!("loadEventFired timeout"),
            Ok(Err(_)) => continue,
            Ok(Ok(ev)) => {
                if ev.method == "Page.loadEventFired" && ev.session_id.as_deref() == Some(&sid) {
                    break;
                }
            }
        }
    }

    let metrics = guard
        .conn
        .send_recv("Page.getLayoutMetrics", json!({}), Some(&sid), CDP_TIMEOUT)
        .await
        .expect("Page.getLayoutMetrics");
    let content_size = metrics
        .get("cssContentSize")
        .or_else(|| metrics.get("contentSize"))
        .expect("cssContentSize/contentSize");
    let width = content_size
        .get("width")
        .and_then(|v| v.as_f64())
        .expect("width");
    let height = content_size
        .get("height")
        .and_then(|v| v.as_f64())
        .expect("height");
    assert!(
        height > 15000.0,
        "test page must exceed the 15000px cap, got {height}"
    );

    let resp = guard
        .conn
        .send_recv(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "fromSurface": true,
                "captureBeyondViewport": true,
                "clip": { "x": 0, "y": 0, "width": width, "height": 15000.0, "scale": 1 },
            }),
            Some(&sid),
            CDP_TIMEOUT,
        )
        .await
        .expect("Page.captureScreenshot");
    let b64 = resp
        .get("data")
        .and_then(|v| v.as_str())
        .expect("captureScreenshot returned `data`");
    assert!(!b64.is_empty(), "screenshot base64 must be non-empty");
    assert!(
        b64.starts_with("iVBORw0KGgo"),
        "screenshot must be a PNG (base64 PNG magic prefix), got {:?}",
        &b64[..b64.len().min(16)]
    );

    guard.release().await.expect("release");
    pool.shutdown(Duration::from_secs(5)).await;
}
