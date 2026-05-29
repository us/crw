//! `screenshot` — capture the current page as PNG/JPEG via the configured
//! Chrome fallback connection.
//!
//! Why not Lightpanda? `Page.captureScreenshot` returns a fixed bogus byte
//! string on Lightpanda v0.2.9 (verified during T1 functional re-probe), so
//! we cannot honour the screenshot contract there. The tool requires
//! `chrome_ws_url` to be configured at server startup; without it, callers
//! get `NOT_IMPLEMENTED` with an explanatory message.
//!
//! State caveat: the Chrome fallback opens a *separate* browser and
//! navigates it to the Lightpanda session's last URL. Cookies, login state,
//! scroll position, and form values are NOT transferred. Screenshots are
//! useful for visual regression and structural debugging, not for capturing
//! the post-interaction state of a Lightpanda session.

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine as _;
use rmcp::{ErrorData as McpError, model::CallToolResult, schemars};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncWriteExt;

use crate::errors::{ErrorCode, ErrorResponse};
use crate::response::ToolResponse;
use crate::server::CrwBrowse;
use crate::tools::common::{MAX_TIMEOUT_MS, clamp_timeout, err_result, no_session_err, ok_result};

/// Cap on the path length to keep the rendered error message bounded.
const MAX_PATH_LEN: usize = 1024;

/// Lower-bound byte threshold that distinguishes a real PNG/JPEG payload from
/// the Lightpanda stub. The stub is fixed at 30 bytes; every real image
/// header (PNG signature + IHDR, JPEG SOI + APP0) clears this bar comfortably.
const MIN_REAL_SCREENSHOT_BYTES: usize = 64;

/// True if the byte buffer is small enough to be the Lightpanda stub rather
/// than a real image. Pulled out of `handle()` so the threshold can be
/// asserted without a live CDP target.
fn is_likely_stub_screenshot(bytes_len: usize) -> bool {
    bytes_len <= MIN_REAL_SCREENSHOT_BYTES
}

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ScreenshotInput {
    /// Filesystem path to write the image to. When omitted, the image is
    /// returned base64-encoded under `data.base64`.
    #[serde(default)]
    pub path: Option<String>,
    /// `png` (default) or `jpeg`.
    #[serde(default)]
    pub format: Option<String>,
    /// Per-call timeout in milliseconds (default: 30000, capped at 120000).
    /// Covers the full sequence: target create, navigate, capture, close.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct ScreenshotData {
    /// Path the image was written to. `None` when the image was returned
    /// inline as base64.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Base64-encoded image bytes. Populated when `path` was not provided.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    /// Image format actually used.
    pub format: String,
    /// Decoded byte count (after base64 decode). Useful as a sanity signal
    /// — Lightpanda's "fake" screenshot is always 30 bytes.
    pub bytes: usize,
}

pub async fn handle(
    server: &CrwBrowse,
    input: ScreenshotInput,
) -> Result<CallToolResult, McpError> {
    let started = Instant::now();
    let (timeout, timeout_clamped) = clamp_timeout(input.timeout_ms, server.config().page_timeout);

    let format = input
        .format
        .clone()
        .unwrap_or_else(|| "png".into())
        .to_lowercase();
    if !matches!(format.as_str(), "png" | "jpeg") {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("format {format:?} not supported — expected png or jpeg"),
        )));
    }
    if let Some(p) = input.path.as_deref()
        && p.len() > MAX_PATH_LEN
    {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("path exceeds {MAX_PATH_LEN} bytes"),
        )));
    }

    // We need the Lightpanda session's URL to mirror it on the Chrome side.
    // Without a session, the screenshot has no anchor.
    let Some(session) = server.default_session_get().await else {
        return Ok(err_result(&no_session_err()));
    };
    let Some(target_url) = session.last_url().await else {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::NotFound,
            "no url to screenshot — call `goto` first",
        )));
    };

    let chrome_conn = match server.ensure_chrome_connection().await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::NotImplemented,
                "screenshot requires a Chrome/Chromium CDP endpoint. \
                 Lightpanda's Page.captureScreenshot returns a 30-byte fake \
                 stub, so this tool needs real Chrome. Start the server with \
                 --chrome-ws-url=ws://127.0.0.1:9222/devtools/browser/<id> \
                 (Chrome launched as `chrome --headless --remote-debugging-port=9222`). \
                 Both --ws-url and --chrome-ws-url can point at the same Chrome \
                 instance if you don't need Lightpanda for the primary session.",
            )));
        }
        Err(e) => {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::BrowserUnavailable,
                format!("failed to connect to Chrome: {e}"),
            )));
        }
    };

    let bytes = match capture_via_chrome(&chrome_conn, &target_url, &format, timeout).await {
        Ok(b) => b,
        Err(e) => return Ok(err_result(&e)),
    };
    // Lightpanda's `Page.captureScreenshot` returns a 30-byte fake stub. With
    // the auto-fallback (chrome_ws_url defaults to ws_url) added in v0.4.1, a
    // Lightpanda-only deployment would otherwise silently hand back the stub
    // and the caller would write it to disk. 64 bytes is a safe floor —
    // every real PNG/JPEG header is larger than that.
    if is_likely_stub_screenshot(bytes.len()) {
        return Ok(err_result(&ErrorResponse::new(
            ErrorCode::NotImplemented,
            format!(
                "screenshot returned only {} bytes — looks like a Lightpanda \
                 fake stub. Pass --chrome-ws-url pointing at real Chrome to \
                 capture real screenshots.",
                bytes.len()
            ),
        )));
    }

    let mut payload_data = ScreenshotData {
        path: None,
        base64: None,
        format: format.clone(),
        bytes: bytes.len(),
    };

    if let Some(path) = input.path {
        let safe_path = match resolve_screenshot_path(server, &path, &format).await {
            Ok(path) => path,
            Err(e) => return Ok(err_result(&e)),
        };
        let mut file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&safe_path)
            .await
        {
            Ok(file) => file,
            Err(e) => {
                return Ok(err_result(&ErrorResponse::new(
                    ErrorCode::Internal,
                    format!("failed to create {}: {e}", safe_path.display()),
                )));
            }
        };
        if let Err(e) = file.write_all(&bytes).await {
            return Ok(err_result(&ErrorResponse::new(
                ErrorCode::Internal,
                format!("failed to write {}: {e}", safe_path.display()),
            )));
        }
        payload_data.path = Some(safe_path.display().to_string());
    } else {
        payload_data.base64 = Some(base64::engine::general_purpose::STANDARD.encode(&bytes));
    }

    let mut payload = ToolResponse::new(&session.short_id, Some(target_url), payload_data)
        .with_elapsed_ms(started.elapsed().as_millis() as u64);
    if timeout_clamped {
        payload = payload.with_warning(format!(
            "timeout_ms clamped to {MAX_TIMEOUT_MS} ms (server-side cap)"
        ));
    }
    Ok(ok_result(&payload))
}

async fn resolve_screenshot_path(
    server: &CrwBrowse,
    requested: &str,
    format: &str,
) -> Result<PathBuf, ErrorResponse> {
    let root = server.config().screenshot_dir.as_ref().ok_or_else(|| {
        ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "screenshot path output requires --screenshot-dir; omit path to receive base64",
        )
    })?;
    let rel = Path::new(requested);
    if rel.is_absolute() {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "screenshot path must be relative to the configured screenshot directory",
        ));
    }
    let mut normal_components = 0;
    for component in rel.components() {
        match component {
            Component::Normal(_) => normal_components += 1,
            _ => {
                return Err(ErrorResponse::new(
                    ErrorCode::InvalidArgs,
                    "screenshot path must not contain traversal or special components",
                ));
            }
        }
    }
    if normal_components != 1 {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "screenshot path must be a single file name inside the screenshot directory",
        ));
    }
    let ext = rel
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());
    let expected = if format == "jpeg" { "jpg" } else { "png" };
    if !matches!(ext.as_deref(), Some("png" | "jpg" | "jpeg")) {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "screenshot path extension must be .png, .jpg, or .jpeg",
        ));
    }
    if format == "png" && ext.as_deref() != Some("png") {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            "png screenshots must use a .png path",
        ));
    }
    if format == "jpeg" && !matches!(ext.as_deref(), Some("jpg" | "jpeg")) {
        return Err(ErrorResponse::new(
            ErrorCode::InvalidArgs,
            format!("jpeg screenshots must use a .{expected} or .jpeg path"),
        ));
    }
    tokio::fs::create_dir_all(root).await.map_err(|e| {
        ErrorResponse::new(
            ErrorCode::Internal,
            format!(
                "failed to create screenshot directory {}: {e}",
                root.display()
            ),
        )
    })?;
    Ok(root.join(rel))
}

/// Open a fresh Chrome target, navigate it to `url`, capture the screenshot,
/// then close the target. We don't keep targets around between calls because
/// the caller usually just wants a one-shot snapshot — keeping them open
/// would slow down the next call (state to reset) without useful sharing.
async fn capture_via_chrome(
    conn: &std::sync::Arc<crw_renderer::cdp_conn::CdpConnection>,
    url: &str,
    format: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ErrorResponse> {
    // 1. Create + attach a target.
    let create = conn
        .send_recv(
            "Target.createTarget",
            serde_json::json!({ "url": "about:blank" }),
            None,
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Chrome Target.createTarget failed: {e}"),
            )
        })?;
    let target_id = create
        .get("targetId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                "Chrome Target.createTarget: no targetId",
            )
        })?
        .to_string();

    // From this point on, any error must still close the target. We always
    // send `Target.closeTarget` regardless of whether `capture_inner`
    // succeeded — leaking a Chromium target across a tight per-call timeout
    // would pin a render process forever.
    //
    // Using a fixed 3s budget here (independent of the caller's `timeout`)
    // because `Target.closeTarget` is a fast no-arg control message; if it's
    // hanging, capping the wait keeps a busy CDP from blocking the response,
    // and the orphan target is cheap (Chrome reaps closed sessions).
    let work = capture_inner(conn, &target_id, url, format, timeout).await;
    let close_timeout = Duration::from_secs(3);
    let _ = conn
        .send_recv(
            "Target.closeTarget",
            serde_json::json!({ "targetId": target_id }),
            None,
            close_timeout,
        )
        .await;
    work
}

async fn capture_inner(
    conn: &std::sync::Arc<crw_renderer::cdp_conn::CdpConnection>,
    target_id: &str,
    url: &str,
    format: &str,
    timeout: Duration,
) -> Result<Vec<u8>, ErrorResponse> {
    let attach = conn
        .send_recv(
            "Target.attachToTarget",
            serde_json::json!({ "targetId": target_id, "flatten": true }),
            None,
            timeout,
        )
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Chrome Target.attachToTarget failed: {e}"),
            )
        })?;
    let sid = attach
        .get("sessionId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                "Chrome Target.attachToTarget: no sessionId",
            )
        })?
        .to_string();

    for method in ["Page.enable", "Network.enable"] {
        conn.send_recv(method, serde_json::json!({}), Some(&sid), timeout)
            .await
            .map_err(|e| {
                ErrorResponse::new(ErrorCode::CdpError, format!("Chrome {method} failed: {e}"))
            })?;
    }
    crate::tools::goto::enable_outbound_guard(conn, &sid, timeout)
        .await
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Chrome Fetch.enable failed: {e}"),
            )
        })?;

    // Subscribe before navigate so we don't miss the load event.
    let mut events = conn.subscribe();
    let guard_rx = conn.subscribe();
    let guard_conn = conn.clone();
    let guard_sid = sid.clone();
    let guard = tokio::spawn(async move {
        crate::tools::goto::run_outbound_guard(guard_conn, guard_rx, &guard_sid).await;
    });

    let navigate = conn
        .send_recv(
            "Page.navigate",
            serde_json::json!({ "url": url }),
            Some(&sid),
            timeout,
        )
        .await;
    if let Err(e) = navigate {
        guard.abort();
        return Err(ErrorResponse::new(
            ErrorCode::NavBlocked,
            format!("Chrome Page.navigate failed: {e}"),
        ));
    }

    // Wait for load. Reuse the broadcast pattern from goto::wait_for_load.
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match tokio::time::timeout_at(deadline, events.recv()).await {
            Err(_) => {
                guard.abort();
                return Err(ErrorResponse::new(
                    ErrorCode::Timeout,
                    "Chrome Page.loadEventFired did not arrive",
                ));
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                guard.abort();
                return Err(ErrorResponse::new(
                    ErrorCode::CdpError,
                    "Chrome event channel closed",
                ));
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Ok(ev)) => {
                if ev.session_id.as_deref() == Some(&sid) && ev.method == "Page.loadEventFired" {
                    break;
                }
            }
        }
    }

    let resp = match conn
        .send_recv(
            "Page.captureScreenshot",
            serde_json::json!({ "format": format }),
            Some(&sid),
            timeout,
        )
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            guard.abort();
            return Err(ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Chrome Page.captureScreenshot failed: {e}"),
            ));
        }
    };
    guard.abort();
    let b64 = resp.get("data").and_then(|v| v.as_str()).ok_or_else(|| {
        ErrorResponse::new(
            ErrorCode::CdpError,
            "Chrome Page.captureScreenshot returned no `data` field",
        )
    })?;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| {
            ErrorResponse::new(
                ErrorCode::CdpError,
                format!("Chrome screenshot base64 decode failed: {e}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lightpanda_stub_size_is_flagged() {
        // Lightpanda's fake screenshot is documented as 30 bytes — verify the
        // threshold catches it.
        assert!(is_likely_stub_screenshot(30));
    }

    #[test]
    fn boundary_values_around_threshold() {
        assert!(is_likely_stub_screenshot(0));
        assert!(is_likely_stub_screenshot(63));
        assert!(is_likely_stub_screenshot(64));
        assert!(!is_likely_stub_screenshot(65));
        assert!(!is_likely_stub_screenshot(128));
    }

    #[test]
    fn real_png_signature_size_passes() {
        // The minimum valid PNG (signature + IHDR + IDAT + IEND chunks) is
        // around 67 bytes for a 1x1 image. Real screenshots are always
        // hundreds of bytes minimum.
        assert!(!is_likely_stub_screenshot(67));
        assert!(!is_likely_stub_screenshot(2_048));
    }

    #[tokio::test]
    async fn path_output_requires_configured_directory() {
        let server = CrwBrowse::new(crate::server::BrowseConfig::default());
        let err = resolve_screenshot_path(&server, "shot.png", "png")
            .await
            .expect_err("path output without screenshot_dir must fail");
        assert_eq!(err.code, ErrorCode::InvalidArgs);
    }

    #[tokio::test]
    async fn path_output_rejects_absolute_traversal_and_extension_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::server::BrowseConfig {
            screenshot_dir: Some(dir.path().to_path_buf()),
            ..crate::server::BrowseConfig::default()
        };
        let server = CrwBrowse::new(config);

        for bad in [
            "/tmp/shot.png",
            "../shot.png",
            "nested/shot.png",
            "shot.jpg",
        ] {
            let err = resolve_screenshot_path(&server, bad, "png")
                .await
                .expect_err(bad);
            assert_eq!(err.code, ErrorCode::InvalidArgs);
        }

        let ok = resolve_screenshot_path(&server, "shot.png", "png")
            .await
            .expect("single filename inside screenshot_dir");
        assert_eq!(ok, dir.path().join("shot.png"));
    }
}
