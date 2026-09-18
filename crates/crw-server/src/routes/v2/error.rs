//! Firecrawl-shaped error responses for the `/v2` compat surface.
//!
//! `AppError` is our native mapping and stays exactly as it is — `/v1` is our
//! documented API and its callers branch on its status codes and `errorCode`
//! spellings today. This wrapper exists so `/v2` can answer the way Firecrawl
//! answers without dragging `/v1` along.
//!
//! Two things differ from `AppError`:
//!
//! 1. **The key.** Firecrawl's envelope is `{success, code, error}`. Both
//!    official SDKs read `code`; our `errorCode` is invisible to every one of
//!    their error paths. We emit `code` with Firecrawl's spelling AND keep
//!    `errorCode` with ours — their SDKs ignore unknown keys, and the SaaS
//!    reads `errorCode` for `RequestLog` (`upstreamErrorCode` in
//!    `api-handler.ts`), so dropping it would blind our own request log.
//!
//! 2. **The status.** Firecrawl's `controllers/v2/scrape.ts` maps a handful of
//!    codes explicitly and lets everything else fall to
//!    `timeoutErr ? 408 : 500`. Notably a DNS failure is **HTTP 200** with
//!    `success:false`, not a 4xx.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use crw_core::error::CrwError;
use crw_core::types::ApiResponse;

use crate::error::AppError;

pub struct V2Error(pub CrwError);

impl From<CrwError> for V2Error {
    fn from(e: CrwError) -> Self {
        Self(e)
    }
}

impl From<AppError> for V2Error {
    fn from(e: AppError) -> Self {
        Self(e.0)
    }
}

impl From<JsonRejection> for V2Error {
    fn from(r: JsonRejection) -> Self {
        Self(AppError::from(r).0)
    }
}

/// Firecrawl's `ErrorCodes` spelling for one of ours, or `None` where Firecrawl
/// sends no `code` at all.
///
/// The `None` arms are captured, not guessed: a real
/// `api.firecrawl.dev/v2/scrape` 429 answers
/// `{"success":false,"error":"Rate limit exceeded. ..."}` with no `code`, and
/// `crawl-status.ts` answers a missing job with a bare
/// `{success:false, error:"Job not found"}`.
fn firecrawl_code(e: &CrwError) -> Option<&'static str> {
    match e {
        // NOT `SCRAPE_DNS_RESOLUTION_ERROR`, even though that is the obvious
        // pairing. Firecrawl splits what we merge — both captured live:
        //
        //   a name that does not resolve  -> HTTP 200, SCRAPE_DNS_RESOLUTION_ERROR
        //   a port that refuses           -> HTTP 500, SCRAPE_SITE_ERROR
        //                                    ("ERR_TUNNEL_CONNECTION_FAILED")
        //
        // `TargetUnreachable` is raised from `reqwest`'s `is_connect()`
        // (`http_only.rs:909`), which is true for both, and the message is
        // "error sending request" either way — there is nothing here to branch
        // on. Claiming the DNS code would be wrong every time the real cause was
        // a refused connection, and claiming HTTP 200 for a dead port tells the
        // caller "fine" about a request that failed. `SCRAPE_SITE_ERROR` is the
        // "URL failed to load" family and is true in both cases.
        //
        // Closing this properly means distinguishing resolver failures in the
        // renderer's error chain; tracked in conformance/FIRECRAWL-DIFF.md.
        CrwError::TargetUnreachable(_) => Some("SCRAPE_SITE_ERROR"),
        // lib/error.ts: ScrapeJobTimeoutError.
        CrwError::Timeout(_) => Some("SCRAPE_TIMEOUT"),
        // scrapeURL/error.ts: NoEnginesLeftError — "every engine we tried came
        // back unusable", which is what all three of these mean for us.
        //
        // `UnsupportedContentType` is NOT mapped to `SCRAPE_UNSUPPORTED_FILE_ERROR`,
        // which is the obvious-looking choice and the wrong one. That code is
        // raised only on Firecrawl's file-UPLOAD path. A *URL* that serves
        // binary just exhausts the waterfall — captured from the live API
        // against `mock.fastcrw.com/bytes/1024`:
        //
        //   HTTP 500  {"success":false,"code":"SCRAPE_ALL_ENGINES_FAILED",
        //              "error":"All scraping engines failed ... Engines tried:
        //              [index, fire-engine;chrome-cdp, ...]"}
        //
        // Our own `errorCode: unsupported_content_type` still rides along, so
        // no diagnostic detail is lost — only the SDK-facing key is flattened
        // to what the SDK would actually have seen.
        CrwError::HttpError(_)
        | CrwError::RendererError(_)
        | CrwError::UnsupportedContentType(_) => Some("SCRAPE_ALL_ENGINES_FAILED"),
        // Firecrawl validates with zod and returns a 400 without a code; its
        // taxonomy carries BAD_REQUEST for the hand-rolled cases.
        CrwError::InvalidRequest(_) | CrwError::UrlParseError(_) => Some("BAD_REQUEST"),
        // No `code` on the wire for these.
        CrwError::NotFound(_) | CrwError::RateLimited => None,
        _ => Some("UNKNOWN_ERROR"),
    }
}

/// The HTTP status Firecrawl answers with.
///
/// `SCRAPE_DNS_RESOLUTION_ERROR` returning **200** is not a typo — it is an
/// explicit branch in `controllers/v2/scrape.ts`:
///
/// ```js
/// // DNS resolution errors should return 200 with success: false
/// if (e.code === "SCRAPE_DNS_RESOLUTION_ERROR") {
///   return res.status(200).json({ success: false, code: e.code, error: e.message });
/// }
/// ```
fn firecrawl_status(e: &CrwError) -> StatusCode {
    match e {
        // Left at 422, the status this surface already answered. See
        // `firecrawl_code` — we cannot tell a DNS failure (their 200) from a
        // refused connection (their 500), so neither of their answers is safe,
        // and 422 at least does not claim success.
        CrwError::TargetUnreachable(_) => StatusCode::UNPROCESSABLE_ENTITY,
        CrwError::Timeout(_) => StatusCode::REQUEST_TIMEOUT,
        CrwError::InvalidRequest(_) | CrwError::UrlParseError(_) => StatusCode::BAD_REQUEST,
        CrwError::NotFound(_) => StatusCode::NOT_FOUND,
        CrwError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        CrwError::SearchDisabled(_) | CrwError::SearchDegraded(_) => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        // Left at 422 rather than swept into the catch-all below. This is what
        // `/v2/parse` answers for a corrupt upload, it is pinned by
        // `parse_tests::parse_corrupt_pdf_422`, and nothing captured or read
        // says Firecrawl's own parse controller answers differently — moving it
        // would be a guess that breaks a deliberate, tested behaviour.
        CrwError::ExtractionError(_) => StatusCode::UNPROCESSABLE_ENTITY,
        // Everything else, including the unsupported-file and all-engines-failed
        // cases that `AppError` reports as 422/502, lands on Firecrawl's
        // catch-all 500.
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

impl IntoResponse for V2Error {
    fn into_response(self) -> Response {
        let status = firecrawl_status(&self.0);
        let mut body =
            ApiResponse::<()>::err_with_code(self.0.to_string(), self.0.error_code().to_string());
        body.code = firecrawl_code(&self.0).map(str::to_string);
        (status, Json(body)).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status the wire actually carries, straight off `into_response`.
    fn status(e: CrwError) -> StatusCode {
        V2Error(e).into_response().status()
    }

    /// The body it carries. Rebuilt rather than drained: reading the response
    /// stream needs an async runtime, and `into_response` has no branch between
    /// these two lines and the wire.
    fn envelope(e: CrwError) -> serde_json::Value {
        let mut body = ApiResponse::<()>::err_with_code(e.to_string(), e.error_code().to_string());
        body.code = firecrawl_code(&e).map(str::to_string);
        serde_json::to_value(&body).unwrap()
    }

    /// Firecrawl answers a DNS failure with HTTP 200 and a refused connection
    /// with HTTP 500 (both captured live). `TargetUnreachable` is raised from
    /// `reqwest::Error::is_connect()`, which is true for both and carries the
    /// same "error sending request" message, so this surface must not claim
    /// either one. Guarding the 200 specifically: answering HTTP 200 for a
    /// request that failed is the one outcome that actively misleads a caller,
    /// and `v2_render_js::…_not_rejected_upfront` pins the 422.
    #[test]
    fn unreachable_target_does_not_claim_firecrawls_dns_answer() {
        let st = status(CrwError::TargetUnreachable("nx.example".into()));
        assert_ne!(st, StatusCode::OK, "a failed fetch must not answer 200");
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        let body = envelope(CrwError::TargetUnreachable("nx.example".into()));
        assert_eq!(body["success"], false);
        assert_eq!(body["code"], "SCRAPE_SITE_ERROR");
    }

    #[test]
    fn timeout_is_408_not_504() {
        let st = status(CrwError::Timeout(30_000));
        assert_eq!(st, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(
            envelope(CrwError::Timeout(30_000))["code"],
            "SCRAPE_TIMEOUT"
        );
    }

    #[test]
    fn binary_url_is_500_all_engines_failed_not_unsupported_file() {
        // Captured from api.firecrawl.dev against mock.fastcrw.com/bytes/1024.
        // SCRAPE_UNSUPPORTED_FILE_ERROR is the upload path only; pinning that
        // here is the mistake this test exists to prevent.
        let st = status(CrwError::UnsupportedContentType("application/zip".into()));
        assert_eq!(st, StatusCode::INTERNAL_SERVER_ERROR);
        let body = envelope(CrwError::UnsupportedContentType("application/zip".into()));
        assert_eq!(body["code"], "SCRAPE_ALL_ENGINES_FAILED");
        // Our own spelling survives for the request log.
        assert_eq!(body["errorCode"], "unsupported_content_type");
    }

    #[test]
    fn both_code_and_error_code_are_emitted() {
        // `code` is what firecrawl-py / firecrawl-js read; `errorCode` is what
        // the SaaS logs (upstreamErrorCode in api-handler.ts). Dropping either
        // breaks one of the two consumers.
        let body = envelope(CrwError::Timeout(1));
        assert_eq!(body["code"], "SCRAPE_TIMEOUT");
        assert_eq!(body["errorCode"], "timeout");
    }

    #[test]
    fn codeless_errors_omit_the_key_entirely() {
        // A captured Firecrawl 429 carries no `code`, and crawl-status.ts's
        // "Job not found" carries none either. Emitting one would be inventing
        // a field their SDKs would then see.
        assert!(envelope(CrwError::RateLimited).get("code").is_none());
        assert!(
            envelope(CrwError::NotFound("crawl job x".into()))
                .get("code")
                .is_none()
        );
    }

    #[test]
    fn not_found_stays_404() {
        assert_eq!(
            status(CrwError::NotFound("crawl job x".into())),
            StatusCode::NOT_FOUND
        );
    }
}
