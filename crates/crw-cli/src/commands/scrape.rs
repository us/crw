//! Scrape subcommand — fetch a single URL and extract content.

use clap::{Args, ValueEnum};
use crw_core::config::{RendererConfig, RendererMode, StealthConfig};
use crw_core::types::{OutputFormat, ScrapeRequest};
use crw_crawl::single::scrape_url;
use crw_renderer::FallbackRenderer;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, ValueEnum)]
pub enum Format {
    Markdown,
    Json,
    Html,
    Rawhtml,
    Text,
    Links,
}

#[derive(Args)]
pub struct ScrapeArgs {
    /// URL to scrape (http or https)
    pub url: String,

    /// Output format
    #[arg(short, long, value_enum, default_value = "markdown")]
    pub format: Format,

    /// Write output to file instead of stdout
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<String>,

    /// Disable main content extraction (return full page content)
    #[arg(long)]
    pub raw: bool,

    /// Enable JavaScript rendering (auto-detects LightPanda/Chrome, or use CRW_CDP_URL)
    #[arg(long)]
    pub js: bool,

    /// Extract only elements matching this CSS selector
    #[arg(long, value_name = "SELECTOR")]
    pub css: Option<String>,

    /// Extract only elements matching this XPath expression
    #[arg(long, value_name = "EXPR")]
    pub xpath: Option<String>,

    /// HTTP, HTTPS, or SOCKS5 proxy URL (e.g. http://user:pass@host:port or socks5://user:pass@host:1080)
    #[arg(long, value_name = "URL")]
    pub proxy: Option<String>,

    /// Enable stealth mode (rotate user agents, inject browser headers)
    #[arg(long)]
    pub stealth: bool,
}

pub async fn run(mut args: ScrapeArgs) {
    // Auto-prepend https:// if no scheme is provided
    if !args.url.contains("://") {
        args.url = format!("https://{}", args.url);
    }

    let stealth_config = StealthConfig {
        enabled: args.stealth,
        inject_headers: args.stealth,
        ..Default::default()
    };

    // For JSON output, we serialize the full ScrapeData — not LLM structured extraction.
    // Request all formats we might need for the output.
    let request_formats = match args.format {
        Format::Markdown => vec![OutputFormat::Markdown],
        Format::Json => vec![
            OutputFormat::Markdown,
            OutputFormat::Html,
            OutputFormat::Links,
        ],
        Format::Html => vec![OutputFormat::Html],
        Format::Rawhtml => vec![OutputFormat::RawHtml],
        Format::Text => vec![OutputFormat::PlainText],
        Format::Links => vec![OutputFormat::Links],
    };

    let cli_extraction_cfg = crw_core::config::ExtractionConfig::default();
    let env_cdp_url = std::env::var("CRW_CDP_URL").ok();

    // Two-phase fetch when `--js` is *not* explicitly set: try HTTP-only first
    // (no browser spawn cost), then escalate to a JS-capable renderer only if
    // the HTTP body extracted to a thin/empty document. This keeps plain-HTML
    // pages (example.com, news articles, blogs) fast while still automatically
    // rendering JS-heavy SPAs (React/Vue/etc.) that return an empty shell.
    //
    // When `--js` is set the user is explicitly opting into JS rendering, so
    // we skip phase 1 and spawn browsers up front.
    let force_js = args.js;

    let mut data: Option<crw_core::types::ScrapeData> = None;
    let mut keep_alive_guards: Vec<crw_renderer::browser::ManagedBrowser> = Vec::new();

    if !force_js {
        // Phase 1: HTTP-only. No browser spawn → no spawn cost on plain pages.
        let http_cfg = RendererConfig {
            mode: RendererMode::None,
            http_timeout_ms: Some(8_000),
            ..Default::default()
        };

        let req = build_request(
            args.url.clone(),
            request_formats.clone(),
            !args.raw,
            None, // render_js = None → auto, but with no JS renderer it stays HTTP
            args.css.clone(),
            args.xpath.clone(),
            args.proxy.clone(),
            args.stealth,
        );

        let http_renderer = match FallbackRenderer::new(
            &http_cfg,
            "crw/0.7.0",
            args.proxy.as_deref(),
            &stealth_config,
        ) {
            Ok(r) => Arc::new(r),
            Err(e) => {
                eprintln!("error: failed to build renderer: {e}");
                std::process::exit(1);
            }
        };

        let http_deadline = crw_core::Deadline::from_request_ms(8_000);
        match scrape_url(
            &req,
            &http_renderer,
            None,
            &cli_extraction_cfg,
            "crw/0.7.0",
            args.stealth,
            None,
            http_deadline,
        )
        .await
        {
            Ok(d) => {
                // Only auto-escalate for the "I just want the page" case
                // (default markdown format, no selectors). Filtered output
                // (--css/--xpath) or specific non-markdown formats are an
                // explicit user choice — measuring "thinness" of a 16-char
                // h1 extraction or a links-only response would always trip
                // the threshold and trigger pointless JS spawns.
                let can_escalate = args.css.is_none()
                    && args.xpath.is_none()
                    && matches!(args.format, Format::Markdown | Format::Json);
                let markdown_len = d.markdown.as_deref().map(str::len).unwrap_or(0);
                let html_text_len = d
                    .plain_text
                    .as_deref()
                    .map(str::len)
                    .unwrap_or_else(|| d.html.as_deref().map(str::len).unwrap_or(0));
                // Same threshold the renderer uses (`is_thin_markdown` < 100).
                // Also catch the "empty SPA shell" case where markdown is empty
                // but the raw HTML is also tiny.
                let is_thin = can_escalate && markdown_len < 100 && html_text_len < 400;
                if is_thin {
                    eprintln!(
                        "info: HTTP returned thin content ({markdown_len} chars markdown), \
                         escalating to JS renderer..."
                    );
                } else {
                    data = Some(d);
                }
            }
            Err(e) => {
                // HTTP-only failure → fall through to JS escalation below.
                eprintln!("info: HTTP fetch failed ({e}), trying JS renderer...");
            }
        }
    }

    // Phase 2 (or sole phase when --js): spawn browsers + run the full
    // HTTP → LightPanda → Chrome fallback chain.
    if data.is_none() {
        // CLI-tuned per-tier timeouts. Server defaults (30s each) assume a
        // long Tower envelope; for interactive CLI we want faster failover so
        // a hanging LightPanda still leaves enough budget for Chrome.
        let mut renderer_config = RendererConfig {
            http_timeout_ms: Some(8_000),
            lightpanda_timeout_ms: Some(12_000),
            chrome_timeout_ms: Some(25_000),
            ..Default::default()
        };

        if let Some(ws_url) = env_cdp_url {
            // Explicit CDP URL — honor it, skip spawn.
            renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
        } else {
            let browsers = crw_renderer::browser::spawn_all_headless().await;
            if browsers.is_empty() && force_js {
                eprintln!(
                    "warning: --js requested but no browser found. \
                     Install LightPanda or Chrome for JS rendering. \
                     Falling back to HTTP."
                );
            }
            for (guard, ws_url, kind) in browsers {
                match kind {
                    crw_renderer::browser::RendererKind::LightPanda => {
                        renderer_config.lightpanda = Some(crw_core::config::CdpEndpoint { ws_url });
                    }
                    crw_renderer::browser::RendererKind::Chrome => {
                        renderer_config.chrome = Some(crw_core::config::CdpEndpoint { ws_url });
                    }
                }
                keep_alive_guards.push(guard);
            }
            if keep_alive_guards.is_empty()
                && renderer_config.lightpanda.is_none()
                && renderer_config.chrome.is_none()
            {
                renderer_config.mode = RendererMode::None;
            }
        }

        let renderer = match FallbackRenderer::new(
            &renderer_config,
            "crw/0.7.0",
            args.proxy.as_deref(),
            &stealth_config,
        ) {
            Ok(r) => Arc::new(r),
            Err(e) => {
                eprintln!("error: failed to build renderer: {e}");
                std::process::exit(1);
            }
        };

        let req = build_request(
            args.url,
            request_formats,
            !args.raw,
            if force_js { Some(true) } else { None },
            args.css,
            args.xpath,
            args.proxy.clone(),
            args.stealth,
        );

        // Size the request deadline so the configured renderer ladder
        // (http + lightpanda + chrome + per-tier CDP overhead) can run
        // uncrushed. Mirrors the server's `auto_extend_deadline_for_ladder`.
        let cli_app_config = crw_core::config::AppConfig {
            renderer: renderer_config.clone(),
            request: crw_core::config::RequestConfig {
                deadline_ms_default: 8_000,
                auto_extend_deadline_for_ladder: true,
            },
            ..Default::default()
        };
        let deadline_ms = cli_app_config.effective_deadline_ms(req.deadline_ms, req.wait_for);
        let cli_deadline = crw_core::Deadline::from_request_ms(deadline_ms);

        match scrape_url(
            &req,
            &renderer,
            None,
            &cli_extraction_cfg,
            "crw/0.7.0",
            args.stealth,
            None,
            cli_deadline,
        )
        .await
        {
            Ok(d) => data = Some(d),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
    }

    let data = data.expect("data must be populated by phase 1 or phase 2");
    // Drop guards only after extraction is done so the browser stays alive
    // through the whole fetch + parse pipeline.
    drop(keep_alive_guards);

    let content = match args.format {
        Format::Markdown => data.markdown.unwrap_or_default(),
        Format::Json => match serde_json::to_string_pretty(&data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to serialize JSON: {e}");
                std::process::exit(1);
            }
        },
        Format::Html => data.html.unwrap_or_default(),
        Format::Rawhtml => data.raw_html.unwrap_or_default(),
        Format::Text => data.plain_text.unwrap_or_default(),
        Format::Links => data.links.unwrap_or_default().join("\n"),
    };

    match args.output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &content) {
                eprintln!("error: failed to write to {path}: {e}");
                std::process::exit(1);
            }
        }
        None => print!("{content}"),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_request(
    url: String,
    formats: Vec<OutputFormat>,
    only_main_content: bool,
    render_js: Option<bool>,
    css: Option<String>,
    xpath: Option<String>,
    proxy: Option<String>,
    stealth: bool,
) -> ScrapeRequest {
    ScrapeRequest {
        url,
        formats,
        only_main_content,
        render_js,
        wait_for: None,
        include_tags: vec![],
        exclude_tags: vec![],
        json_schema: None,
        headers: HashMap::new(),
        css_selector: css,
        xpath,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        proxy,
        stealth: if stealth { Some(true) } else { None },
        actions: None,
        extract: None,
        llm_api_key: None,
        llm_provider: None,
        llm_model: None,
        base_url: None,
        summary_prompt: None,
        max_content_chars: None,
        renderer: None,
        deadline_ms: None,
        debug: None,
    }
}
