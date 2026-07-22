//! HTTP client for SearXNG's JSON search API.
//!
//! Mirrors `crw-saas/src/lib/searxng-client.ts`. The shape of the response
//! follows the SearXNG `search_api` docs and the `result_types/index` page —
//! every per-result field except `url`, `title`, and `engine` is treated as
//! optional because real-world engines are uneven.

use futures::StreamExt;
use serde::Deserialize;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

use crate::params::SearxngParams;

/// Hard cap on a SearXNG JSON response body (10 MiB). Real responses are
/// well under 1 MiB; anything bigger is a sign of upstream misbehavior or a
/// memory-amplification attack, so we abort the read instead of allocating it.
const MAX_RESPONSE_BYTES: usize = 10 * 1024 * 1024;

/// Tighter cap for non-2xx error bodies. We only surface the first 500 chars
/// to the caller anyway, so a 64 KiB ceiling is plenty for diagnostics while
/// closing the door on hostile upstreams that retaliate to invalid params
/// with multi-megabyte error pages.
const MAX_ERROR_BODY_BYTES: usize = 64 * 1024;

async fn read_capped(response: reqwest::Response, cap: usize) -> Result<Vec<u8>, SearchError> {
    if let Some(declared) = response.content_length()
        && declared as usize > cap
    {
        return Err(SearchError::InvalidResponse(format!(
            "response too large: declared {declared} bytes exceeds {cap} cap"
        )));
    }
    let mut buf: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e: reqwest::Error| SearchError::Transport(e.to_string()))?;
        if buf.len() + chunk.len() > cap {
            return Err(SearchError::InvalidResponse(format!(
                "response too large: exceeded {cap}-byte cap"
            )));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(buf)
}

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("SearXNG request timed out")]
    Timeout,
    #[error("SearXNG upstream error (status {status}): {body}")]
    Upstream { status: u16, body: String },
    #[error("SearXNG returned an invalid JSON response: {0}")]
    InvalidResponse(String),
    #[error("SearXNG transport error: {0}")]
    Transport(String),
}

/// A single result row from SearXNG. Every field is `Option` because real
/// engines occasionally return malformed rows (missing url/title/engine in
/// flaky upstream responses). The transform layer drops any row missing the
/// load-bearing fields rather than failing the entire search response — see
/// `transform.rs`.
#[derive(Debug, Clone, Deserialize)]
pub struct SearxngResult {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub engine: Option<String>,
    /// Snippet / description. SearXNG calls this `content`; the public API
    /// renames it to `description`.
    #[serde(default)]
    pub content: Option<String>,
    /// Relevance score (higher is better). Missing on engines that don't rank.
    #[serde(default)]
    pub score: Option<f64>,
    /// Per-engine identifiers that returned this row (SearXNG `format=json`
    /// emits this when a result is found by more than one engine). Used by the
    /// re-rank pipeline for engine-aware bookkeeping; harmless on the raw path.
    #[serde(default)]
    pub engines: Vec<String>,
    /// Per-engine ranks for this row (one entry per engine in `engines`).
    /// Drives Reciprocal Rank Fusion in the re-rank pipeline. Empty on the
    /// rare engines that don't report a position.
    #[serde(default)]
    pub positions: Vec<u32>,
    /// Top-level category bucket reported by SearXNG (`general`, `news`,
    /// `images`, `videos`, ...).
    #[serde(default)]
    pub category: Option<String>,
    /// Template hint (`default.html`, `images.html`, `videos.html`,
    /// `paper.html`, ...). Useful as a fallback when `category` is missing.
    #[serde(default)]
    pub template: Option<String>,
    /// ISO-formatted publish date for news results.
    #[serde(default, rename = "publishedDate")]
    pub published_date: Option<String>,
    /// Image URL — populated for image-template results.
    #[serde(default)]
    pub img_src: Option<String>,
    /// Thumbnail URL — populated for image / video results.
    #[serde(default)]
    pub thumbnail_src: Option<String>,
    #[serde(default)]
    pub img_format: Option<String>,
    #[serde(default)]
    pub resolution: Option<String>,
}

/// Top-level SearXNG `format=json` response envelope.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct SearxngResponse {
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub number_of_results: u64,
    #[serde(default)]
    pub results: Vec<SearxngResult>,
    #[serde(default)]
    pub answers: Vec<serde_json::Value>,
    #[serde(default)]
    pub corrections: Vec<String>,
    #[serde(default)]
    pub infoboxes: Vec<serde_json::Value>,
    #[serde(default)]
    pub suggestions: Vec<String>,
    #[serde(default)]
    pub unresponsive_engines: Vec<serde_json::Value>,
    /// Explicit degraded flag an upstream orchestrator may set. A plain
    /// SearXNG backend never sets this, hence the serde default.
    #[serde(default)]
    pub degraded: bool,
}

impl SearxngResponse {
    /// True when the backend answered with nothing AND reported that engines
    /// failed. Emptiness is a PREREQUISITE for both signals: a response that
    /// some later leg rescued into a non-empty pool is never degraded.
    pub fn is_degraded(&self) -> bool {
        self.results.is_empty() && (self.degraded || !self.unresponsive_engines.is_empty())
    }
}

/// Thin async client for SearXNG. One instance per server; reuse across
/// requests so the underlying `reqwest::Client` connection pool is hot.
#[derive(Debug, Clone)]
pub struct SearxngClient {
    http: Arc<reqwest::Client>,
    base_url: String,
    timeout: Duration,
}

impl SearxngClient {
    pub fn new(http: Arc<reqwest::Client>, base_url: impl Into<String>, timeout: Duration) -> Self {
        let base_url = base_url.into();
        let trimmed = base_url.trim_end_matches('/').to_string();
        Self {
            http,
            base_url: trimmed,
            timeout,
        }
    }

    /// Configured base URL (trailing slash trimmed). Exposed so the route layer
    /// can name the host in `target_unreachable` errors without leaking it raw
    /// (callers sanitize to the origin first — see crw-server `diagnostics`).
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Issue a JSON search request. Errors surface as a typed [`SearchError`]
    /// — the route layer maps them onto `CrwError` for HTTP responses.
    pub async fn fetch(&self, params: &SearxngParams) -> Result<SearxngResponse, SearchError> {
        let mut url = url::Url::parse(&format!("{}/search", self.base_url))
            .map_err(|e| SearchError::Transport(format!("invalid base_url: {e}")))?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("format", "json");
            q.append_pair("q", &params.q);
            if let Some(c) = &params.categories {
                q.append_pair("categories", c);
            }
            if let Some(l) = &params.language {
                q.append_pair("language", l);
            }
            if let Some(t) = &params.time_range {
                q.append_pair("time_range", t);
            }
            if let Some(e) = &params.engines {
                q.append_pair("engines", e);
            }
            if let Some(p) = params.pageno {
                q.append_pair("pageno", &p.to_string());
            }
            if let Some(s) = params.safesearch {
                q.append_pair("safesearch", &s.to_string());
            }
        }

        let response = self
            .http
            .get(url)
            .timeout(self.timeout)
            .send()
            .await
            .map_err(|e: reqwest::Error| {
                if e.is_timeout() {
                    SearchError::Timeout
                } else {
                    // `without_url()` strips reqwest's embedded request URL from
                    // the Display string — that URL can carry credentials/tokens
                    // (issue #90). The route layer re-attaches a sanitized origin.
                    SearchError::Transport(e.without_url().to_string())
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            // Apply the same streaming cap to the error path. Without it, a
            // hostile upstream could retaliate to an invalid query with a
            // multi-megabyte 4xx body and push us into unbounded allocation
            // — even though we only display the first 500 chars.
            let body_bytes = read_capped(response, MAX_ERROR_BODY_BYTES)
                .await
                .unwrap_or_default();
            let body = String::from_utf8_lossy(&body_bytes);
            let trimmed: String = body.chars().take(500).collect();
            return Err(SearchError::Upstream {
                status: status.as_u16(),
                body: trimmed,
            });
        }

        // Stream the body with a hard byte cap so a misbehaving upstream
        // can't push us into unbounded allocation. We refuse to parse past
        // `MAX_RESPONSE_BYTES`. `Content-Length` is not trusted (chunked
        // encoding sets none) — `read_capped` enforces on the running sum.
        let buf = read_capped(response, MAX_RESPONSE_BYTES).await?;
        serde_json::from_slice::<SearxngResponse>(&buf)
            .map_err(|e| SearchError::InvalidResponse(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_degraded_true_on_empty_results_with_unresponsive_engines() {
        let resp = SearxngResponse {
            unresponsive_engines: vec![serde_json::json!(["google", "timeout"])],
            ..Default::default()
        };
        assert!(resp.is_degraded());
    }

    #[test]
    fn is_degraded_false_on_genuine_zero_results() {
        // Empty results, no unresponsive engines, no degraded flag: a real
        // zero-result query, not a backend failure — must stay a normal 200.
        let resp = SearxngResponse::default();
        assert!(!resp.is_degraded());
    }

    #[test]
    fn is_degraded_false_when_results_non_empty() {
        // Emptiness is a prerequisite: a later leg that rescued the pool into
        // non-empty results is never degraded, even if engines failed.
        let resp: SearxngResponse = serde_json::from_value(serde_json::json!({
            "results": [{"url": "https://example.com", "title": "Example", "engine": "google"}],
            "unresponsive_engines": [["google", "timeout"]],
        }))
        .unwrap();
        assert!(!resp.is_degraded());
    }
}
