//! `POST /v2/scrape` (+ `GET /v2/scrape/{job_id}` Tier-3 stub).

use std::collections::HashMap;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, State};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crw_core::Deadline;
use crw_core::error::CrwError;
use crw_core::types::{OutputFormat, RequestedRenderer, ScrapeRequest};
use crw_crawl::single::scrape_url;

use super::adapters::{V2Document, to_v2_document};
use super::formats::{self, FormatSpec, decompose};
use crate::error::AppError;
use crate::state::{AppState, validate_renderer_pin};

/// v2 `/v2/scrape` request. Lenient: unknown fields the SDK may send
/// (`mobile`, `actions`, `blockAds`, `storeInCache`, `maxAge`,
/// `origin`, `integration`, …) are ignored by serde — we must NOT
/// `deny_unknown_fields` or a newer SDK build would 400.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V2ScrapeRequest {
    pub url: String,
    #[serde(default = "default_v2_formats")]
    pub formats: Vec<FormatSpec>,
    #[serde(default = "default_true")]
    pub only_main_content: bool,
    #[serde(default)]
    pub include_tags: Vec<String>,
    #[serde(default)]
    pub exclude_tags: Vec<String>,
    #[serde(default)]
    pub wait_for: Option<u64>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// v2 `location` object. `country` is lowercased and mapped to the engine's
    /// 2-letter proxy-egress country.
    #[serde(default)]
    pub location: Option<V2Location>,
    /// v2 proxy mode. Default "auto" (NOT v1's "basic"). "stealth" routes to the
    /// residential chrome tier; everything else is reported as "basic".
    #[serde(default = "default_proxy")]
    pub proxy: String,
    /// BYOP proxy pool (crw extension) rotated per `proxy_rotation`. Distinct
    /// from the `proxy` MODE above — these are actual proxy URLs. Accepts the
    /// snake_case `proxy_list` alias (what the managed layer injects).
    #[serde(default, alias = "proxy_list")]
    pub proxy_list: Vec<String>,
    #[serde(default, alias = "proxy_rotation")]
    pub proxy_rotation: Option<crw_core::proxy::ProxyRotation>,
    /// v2 `timeout` (ms) → engine `deadline_ms`.
    #[serde(default)]
    pub timeout: Option<u64>,
    // BYOK passthrough (same names as v1 so the SaaS header path is unchanged).
    #[serde(default)]
    pub llm_api_key: Option<String>,
    #[serde(default)]
    pub llm_provider: Option<String>,
    #[serde(default)]
    pub llm_model: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub summary_prompt: Option<String>,
    /// Optional explicit renderer pin (crw extension, tolerated alongside v2).
    #[serde(default)]
    pub renderer: Option<RequestedRenderer>,
    /// Firecrawl `parsers` — document parsing directives. Accepts `["pdf"]` or
    /// `[{"type":"pdf","maxPages":N}]`. Omitted = auto-parse PDFs; `[]` = leave
    /// raw. See [`crw_core::types::ParserSpec`].
    #[serde(default)]
    pub parsers: Option<Vec<crw_core::types::ParserSpec>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct V2Location {
    #[serde(default)]
    pub country: Option<String>,
    #[serde(default)]
    pub languages: Option<Vec<String>>,
}

fn default_true() -> bool {
    true
}
fn default_proxy() -> String {
    "auto".to_string()
}
fn default_v2_formats() -> Vec<FormatSpec> {
    vec![FormatSpec::String("markdown".to_string())]
}

/// `{ success, data, warning? }` envelope.
#[derive(Debug, Serialize)]
pub struct V2ScrapeResponse {
    pub success: bool,
    pub data: V2Document,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    /// Anti-bot block message (vendor + reason); present iff `success == false`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Resolved proxy tier reported in `metadata.proxyUsed`.
pub(crate) fn proxy_tier(proxy: &str) -> &'static str {
    if proxy.eq_ignore_ascii_case("stealth") {
        "stealth"
    } else {
        "basic"
    }
}

/// Convert a v2 scrape request into the internal `ScrapeRequest` + the
/// decomposed-format side-data + the resolved proxy tier.
pub(crate) fn to_internal(
    v2: V2ScrapeRequest,
) -> Result<(ScrapeRequest, formats::DecomposedFormats, String), CrwError> {
    let decomposed = decompose(&v2.formats).map_err(CrwError::InvalidRequest)?;
    let tier = proxy_tier(&v2.proxy).to_string();
    // "stealth" → residential chrome tier; otherwise let the renderer chain
    // decide ("auto"). An explicit `renderer` pin always wins.
    let renderer = v2.renderer.or(if v2.proxy.eq_ignore_ascii_case("stealth") {
        Some(RequestedRenderer::ChromeProxy)
    } else {
        None
    });
    let country = v2
        .location
        .as_ref()
        .and_then(|l| l.country.as_ref())
        .map(|c| c.to_lowercase());

    let req = ScrapeRequest {
        url: v2.url,
        formats: decomposed.formats.clone(),
        only_main_content: v2.only_main_content,
        include_tags: v2.include_tags,
        exclude_tags: v2.exclude_tags,
        wait_for: v2.wait_for,
        headers: v2.headers,
        json_schema: decomposed.json_schema.clone(),
        change_tracking: decomposed.change_tracking.clone(),
        screenshot_full_page: decomposed.screenshot_full_page,
        country,
        deadline_ms: v2.timeout,
        llm_api_key: v2.llm_api_key,
        llm_provider: v2.llm_provider,
        llm_model: v2.llm_model,
        base_url: v2.base_url,
        summary_prompt: v2.summary_prompt,
        renderer,
        parsers: v2.parsers,
        proxy_list: v2.proxy_list,
        proxy_rotation: v2.proxy_rotation,
        ..Default::default()
    };
    Ok((req, decomposed, tier))
}

pub async fn scrape(
    State(state): State<AppState>,
    body: Result<Json<V2ScrapeRequest>, JsonRejection>,
) -> Result<Json<V2ScrapeResponse>, AppError> {
    let Json(v2) = body.map_err(AppError::from)?;

    let parsed_url = url::Url::parse(&v2.url)
        .map_err(|e| CrwError::InvalidRequest(format!("Invalid URL: {e}")))?;
    crw_core::url_safety::validate_safe_url_resolved(&parsed_url)
        .await
        .map_err(CrwError::InvalidRequest)?;

    let (req, decomposed, tier) = to_internal(v2)?;
    validate_renderer_pin(req.renderer, req.render_js, &state)?;

    let llm_config = state.config.extraction.llm.as_ref();
    if req.formats.contains(&OutputFormat::Summary)
        && llm_config.is_none()
        && req.llm_api_key.is_none()
    {
        return Err(AppError::from(CrwError::InvalidRequest(
            "summary format requires LLM config: set CRW_EXTRACTION__LLM__API_KEY \
             in server config or pass llm_api_key in the request body"
                .into(),
        )));
    }

    let user_agent = &state.config.crawler.user_agent;
    let default_stealth =
        state.config.crawler.stealth.enabled && state.config.crawler.stealth.inject_headers;
    let deadline = Deadline::from_request_ms(
        state
            .config
            .effective_deadline_ms(req.deadline_ms, req.wait_for),
    );

    let data = scrape_url(
        &req,
        &state.renderer,
        llm_config,
        &state.config.extraction,
        user_agent,
        default_stealth,
        state.config.renderer.render_js_default,
        deadline,
    )
    .await?;

    let warning = formats::unsupported_warning(&decomposed.unsupported);
    // HTTP-error-first gate (mirrors v1 scrape.rs): a genuine tiny 4xx/5xx page
    // is a plain HTTP error, not an anti-bot block. It must short-circuit before
    // the block check so classify()'s StructuralFailure can't mislabel it, and
    // so both API surfaces label the identical page the same way.
    let status_code = data.metadata.status_code;
    let http_error = if status_code >= 400 {
        let body_len = [
            data.markdown.as_deref(),
            data.plain_text.as_deref(),
            data.html.as_deref(),
            data.raw_html.as_deref(),
        ]
        .iter()
        .filter_map(|opt| opt.map(|t| t.len()))
        .max()
        .unwrap_or(0);
        (body_len < 200).then(|| {
            data.warning
                .clone()
                .unwrap_or_else(|| format!("Target returned HTTP {status_code}"))
        })
    } else {
        None
    };
    // Anti-bot verdict from the choke: a blocked page is `success:false` with an
    // error string, matching v1's behaviour. Read before `to_v2_document`
    // consumes `data`.
    let (success, error) = match (http_error, data.block.as_ref()) {
        (Some(msg), _) => (false, Some(msg)),
        (None, Some(b)) => (false, Some(b.message())),
        (None, None) => (true, None),
    };
    let doc = to_v2_document(data, &tier, Uuid::new_v4().to_string());
    Ok(Json(V2ScrapeResponse {
        success,
        data: doc,
        warning,
        error,
    }))
}

/// `GET /v2/scrape/{job_id}` (Tier-3). crw scrape is synchronous, so a scrape
/// "job" never exists to poll — the SDK only hits this when it used an async
/// scrape path we don't expose. Return a clear 404 so the SDK surfaces a
/// meaningful error rather than hanging.
pub async fn get_scrape_job(
    Path(job_id): Path<String>,
) -> Result<Json<V2ScrapeResponse>, AppError> {
    Err(AppError::from(CrwError::NotFound(format!(
        "scrape job {job_id} not found — this engine performs scrapes synchronously; \
         use POST /v2/scrape and read the response directly"
    ))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v2_scrape_accepts_snake_case_proxy_alias_and_threads_to_internal() {
        // The managed layer injects snake_case proxy_list/proxy_rotation; the v2
        // wire is camelCase, so the alias must accept both and to_internal must
        // thread them into the engine ScrapeRequest (not drop via Default).
        let body = serde_json::json!({
            "url": "http://example.com",
            "proxy_list": ["http://u:p@1.2.3.4:8080"],
            "proxy_rotation": "round_robin",
        });
        let v2: V2ScrapeRequest = serde_json::from_value(body).unwrap();
        assert_eq!(v2.proxy_list, vec!["http://u:p@1.2.3.4:8080"]);
        assert_eq!(
            v2.proxy_rotation,
            Some(crw_core::proxy::ProxyRotation::RoundRobin)
        );
        let (req, _, _) = to_internal(v2).unwrap();
        assert_eq!(req.proxy_list, vec!["http://u:p@1.2.3.4:8080"]);
        assert_eq!(
            req.proxy_rotation,
            Some(crw_core::proxy::ProxyRotation::RoundRobin)
        );
    }

    fn minimal_doc() -> V2Document {
        use crw_core::types::{PageMetadata, ScrapeData};
        let data = ScrapeData {
            markdown: Some("# hi".into()),
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
                title: None,
                description: None,
                og_title: None,
                og_description: None,
                og_image: None,
                canonical_url: None,
                source_url: "https://example.com".into(),
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
        };
        to_v2_document(data, "basic", "id".into())
    }

    #[test]
    fn v2_response_blocked_serializes_success_false_with_error() {
        let resp = V2ScrapeResponse {
            success: false,
            data: minimal_doc(),
            warning: None,
            error: Some("Blocked by anti-bot (cloudflare): CF challenge".into()),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["success"], false);
        assert_eq!(v["error"], "Blocked by anti-bot (cloudflare): CF challenge");
    }

    #[test]
    fn v2_response_clean_omits_error_key() {
        let resp = V2ScrapeResponse {
            success: true,
            data: minimal_doc(),
            warning: None,
            error: None,
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert_eq!(v["success"], true);
        assert!(v.get("error").is_none(), "error omitted on success");
    }

    #[test]
    fn v2_scrape_camelcase_proxy_list_also_works_and_mode_is_separate() {
        let body = serde_json::json!({
            "url": "http://example.com",
            "proxyList": ["http://1.2.3.4:8080"],
            "proxy": "stealth",
        });
        let v2: V2ScrapeRequest = serde_json::from_value(body).unwrap();
        assert_eq!(v2.proxy_list, vec!["http://1.2.3.4:8080"]);
        // `proxy` MODE is independent of the proxy_list BYOP URLs.
        assert_eq!(v2.proxy, "stealth");
        let (req, _, _) = to_internal(v2).unwrap();
        assert_eq!(req.proxy_list.len(), 1);
    }
}
