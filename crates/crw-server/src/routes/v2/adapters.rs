//! Response shapers: internal engine types → Firecrawl v2 wire shapes.
//!
//! These are pure functions over the existing `ScrapeData` / `CrawlState` so the
//! v1 wire shapes stay untouched — every v2-only field (`metadata.proxyUsed`,
//! `cacheState`, `creditsUsed`, `scrapeId`, crawl `next`/`expiresAt`) is
//! synthesized here, not added to the core types.

use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use uuid::Uuid;

use crw_core::types::{ChangeTrackingResult, CrawlState, CrawlStatus, ScrapeData};

/// Firecrawl v2 `Document`. Field order/casing matches the live API.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub markdown: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_html: Option<String>,
    /// Inside a Document, `links` is a flat string array (only `/v2/map` returns
    /// link objects — see `V2Link`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_tracking: Option<ChangeTrackingResult>,
    /// Page screenshot as a `data:image/png;base64,<...>` URL (Firecrawl-compat
    /// `screenshot` field). Present only when the `screenshot` format was asked.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    pub metadata: V2Metadata,
}

/// Firecrawl v2 `Document.metadata`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(rename = "sourceURL")]
    pub source_url: String,
    pub url: String,
    pub status_code: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    /// Resolved proxy tier ("basic" | "stealth"). v2 always reports one.
    pub proxy_used: String,
    /// crw has no read-through cache yet — always "miss".
    pub cache_state: String,
    /// Firecrawl-compat: whether the request was throttled by a concurrency
    /// cap. A self-host engine doesn't concurrency-limit individual requests,
    /// so this is always `false`.
    pub concurrency_limited: bool,
    pub credits_used: u32,
    pub scrape_id: String,
    /// Page count for paginated documents (PDF). Omitted for web pages.
    /// Serialized as `numPages` to match Firecrawl.
    #[serde(rename = "numPages", skip_serializing_if = "Option::is_none")]
    pub page_count: Option<usize>,
    /// Original filename for uploaded documents (via /v2/parse). Omitted otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_filename: Option<String>,
}

/// Map an engine `ScrapeData` to a v2 `Document`. `proxy_used` is the resolved
/// proxy tier; `scrape_id` is a per-document UUID.
pub fn to_v2_document(data: ScrapeData, proxy_used: &str, scrape_id: String) -> V2Document {
    let m = &data.metadata;
    let metadata = V2Metadata {
        title: m.title.clone(),
        description: m.description.clone(),
        language: m.language.clone(),
        source_url: m.source_url.clone(),
        url: m.source_url.clone(),
        status_code: m.status_code,
        content_type: data.content_type.clone(),
        proxy_used: proxy_used.to_string(),
        cache_state: "miss".to_string(),
        concurrency_limited: false,
        // Engine does not price requests (the SaaS layer bills); surface
        // whatever the engine attributed, defaulting to 1 like the live API.
        credits_used: if data.credit_cost == 0 {
            1
        } else {
            data.credit_cost
        },
        scrape_id,
        page_count: m.page_count,
        source_filename: m.source_filename.clone(),
    };
    V2Document {
        markdown: data.markdown,
        html: data.html,
        raw_html: data.raw_html,
        links: data.links,
        json: data.json,
        summary: data.summary,
        change_tracking: data.change_tracking,
        screenshot: data.screenshot,
        warning: data.warning,
        metadata,
    }
}

/// Firecrawl v2 crawl / batch-scrape status. Shared shape for `GET /v2/crawl/{id}`
/// and `GET /v2/batch/scrape/{id}` (the live API returns an identical envelope).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct V2CrawlStatus {
    pub success: bool,
    pub status: &'static str,
    pub total: u32,
    pub completed: u32,
    pub credits_used: u32,
    pub expires_at: String,
    /// Pagination cursor; `null` once the job is `completed` and no further
    /// pages remain.
    pub next: Option<String>,
    pub data: Vec<V2Document>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn status_str(s: CrawlStatus) -> &'static str {
    match s {
        CrawlStatus::InProgress => "scraping",
        CrawlStatus::Completed => "completed",
        CrawlStatus::Failed => "failed",
        CrawlStatus::Cancelled => "cancelled",
    }
}

/// Default page size for crawl/batch status pagination (documents per page).
pub const DEFAULT_PAGE_LIMIT: usize = 100;
/// Soft byte cap per status page. We stop adding documents to a page once the
/// accumulated markdown/html bytes exceed this, so a completed large crawl
/// paginates instead of serializing one oversized response (Firecrawl uses
/// ~10 MiB; we mirror it).
pub const PAGE_BYTE_CAP: usize = 10 * 1024 * 1024;

/// Build a v2 status response from a `CrawlState` snapshot, paginating from a
/// 0-based document offset `skip`.
///
/// `path_prefix` is `/v2/crawl` or `/v2/batch/scrape`; `base` is the
/// scheme+host the `next` URL should use (caller derives it from the inbound
/// `Host` header or a configured public base).
#[allow(clippy::too_many_arguments)]
pub fn build_crawl_status(
    state: &CrawlState,
    created_at: Instant,
    job_ttl_secs: u64,
    skip: usize,
    limit: usize,
    base: &str,
    path_prefix: &str,
    id: Uuid,
    proxy_used: &str,
) -> V2CrawlStatus {
    let total_docs = state.data.len();
    let limit = limit.max(1);

    // Slice [skip, skip+limit) with a soft byte cap so a single page can't grow
    // unbounded.
    let mut docs = Vec::new();
    let mut bytes = 0usize;
    let mut emitted = 0usize;
    if skip < total_docs {
        for d in state.data[skip..].iter().take(limit) {
            let doc_bytes = d.markdown.as_ref().map(String::len).unwrap_or(0)
                + d.html.as_ref().map(String::len).unwrap_or(0)
                + d.raw_html.as_ref().map(String::len).unwrap_or(0);
            if emitted > 0 && bytes + doc_bytes > PAGE_BYTE_CAP {
                break;
            }
            bytes += doc_bytes;
            emitted += 1;
            let sid = Uuid::new_v4().to_string();
            docs.push(to_v2_document(d.clone(), proxy_used, sid));
        }
    }

    let next_skip = skip + emitted;
    // Emit `next` when more buffered pages remain, OR while the job is still
    // running (so the SDK keeps polling forward even at a momentary page edge).
    let more_buffered = next_skip < total_docs;
    let running = matches!(state.status, CrawlStatus::InProgress);
    let next = if more_buffered || running {
        Some(format!("{base}{path_prefix}/{id}?skip={next_skip}"))
    } else {
        None
    };

    let credits_used: u32 = state
        .data
        .iter()
        .map(|d| if d.credit_cost == 0 { 1 } else { d.credit_cost })
        .sum();

    V2CrawlStatus {
        success: !matches!(state.status, CrawlStatus::Failed),
        status: status_str(state.status),
        total: state.total.max(total_docs as u32),
        completed: state.completed,
        credits_used,
        expires_at: expires_at_rfc3339(created_at, job_ttl_secs),
        next,
        data: docs,
        error: state.error.clone(),
    }
}

/// Job expiry as an RFC3339 UTC timestamp: `now + (ttl − elapsed)`.
pub fn expires_at_rfc3339(created_at: Instant, job_ttl_secs: u64) -> String {
    let remaining = job_ttl_secs.saturating_sub(created_at.elapsed().as_secs());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    rfc3339_utc(now + remaining)
}

/// Format a Unix-epoch second count as `YYYY-MM-DDTHH:MM:SS.000Z` (UTC).
/// Hand-rolled (Howard Hinnant's `civil_from_days`) to avoid a chrono/time
/// dependency.
pub fn rfc3339_utc(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let sod = unix_secs % 86_400;
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let mth = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if mth <= 2 { y + 1 } else { y };

    format!("{year:04}-{mth:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.000Z")
}

/// Firecrawl v2 `/v2/map` link object.
#[derive(Debug, Serialize)]
pub struct V2Link {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crw_core::types::PageMetadata;

    #[test]
    fn rfc3339_matches_known_epoch() {
        // Unix epoch.
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00.000Z");
        // Widely-referenced round value: 1_700_000_000 == 2023-11-14T22:13:20Z.
        assert_eq!(rfc3339_utc(1_700_000_000), "2023-11-14T22:13:20.000Z");
    }

    fn fake_doc(url: &str) -> ScrapeData {
        ScrapeData {
            markdown: Some("# hi".to_string()),
            source_hash: None,
            html: None,
            raw_html: None,
            plain_text: None,
            links: None,
            json: None,
            summary: None,
            llm_usage: None,
            chunks: None,
            warning: None,
            warnings: vec![],
            render_decision: None,
            credit_cost: 1,
            basis: None,
            basis_warnings: Vec::new(),
            llm_input_hash: None,
            metadata: PageMetadata {
                title: Some("T".into()),
                description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
                source_url: url.to_string(),
                language: None,
                status_code: 200,
                rendered_with: None,
                elapsed_ms: 0,
                page_count: None,
                source_filename: None,
                extra: Default::default(),
            },
            debug_extraction: None,
            content_type: Some("text/html".into()),
            change_tracking: None,
            screenshot: None,
            block: None,
        }
    }

    fn state(status: CrawlStatus, total: u32, completed: u32, n: usize) -> CrawlState {
        CrawlState {
            id: Uuid::nil(),
            success: true,
            status,
            total,
            completed,
            data: (0..n)
                .map(|i| fake_doc(&format!("https://x/{i}")))
                .collect(),
            error: None,
        }
    }

    #[test]
    fn pagination_skip_next_and_credits() {
        let s = state(CrawlStatus::Completed, 250, 250, 250);
        let now = Instant::now();

        let p0 = build_crawl_status(
            &s,
            now,
            86_400,
            0,
            100,
            "https://api.example",
            "/v2/crawl",
            Uuid::nil(),
            "basic",
        );
        assert_eq!(p0.data.len(), 100);
        assert_eq!(p0.total, 250);
        assert_eq!(p0.completed, 250);
        assert_eq!(p0.credits_used, 250);
        assert_eq!(
            p0.next.as_deref(),
            Some("https://api.example/v2/crawl/00000000-0000-0000-0000-000000000000?skip=100")
        );

        // Last page of a completed job → next is null.
        let p2 = build_crawl_status(
            &s,
            now,
            86_400,
            200,
            100,
            "https://api.example",
            "/v2/crawl",
            Uuid::nil(),
            "basic",
        );
        assert_eq!(p2.data.len(), 50);
        assert!(p2.next.is_none());

        // skip past the end → empty page, next null.
        let p3 = build_crawl_status(
            &s,
            now,
            86_400,
            300,
            100,
            "https://api.example",
            "/v2/crawl",
            Uuid::nil(),
            "basic",
        );
        assert_eq!(p3.data.len(), 0);
        assert!(p3.next.is_none());
    }

    #[test]
    fn running_job_emits_next_even_at_buffer_edge() {
        // 10 buffered docs, job still running (total unknown-ish at 50).
        let s = state(CrawlStatus::InProgress, 50, 10, 10);
        let p = build_crawl_status(
            &s,
            Instant::now(),
            86_400,
            0,
            100,
            "https://b",
            "/v2/crawl",
            Uuid::nil(),
            "basic",
        );
        assert_eq!(p.data.len(), 10);
        assert_eq!(p.status, "scraping");
        // SDK must keep polling forward even though we returned all buffered docs.
        assert!(p.next.is_some());
    }
}
