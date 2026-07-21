//! Scrape subcommand — fetch a single URL and extract content.

use crate::teardown::CmdError;
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
    Images,
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

    /// Generate an AI summary of the page using the configured LLM.
    #[arg(long, conflicts_with = "extract")]
    pub summary: bool,

    /// Style/format hint for --summary (e.g. "in 3 bullet points", "as a haiku").
    #[arg(long, value_name = "TEXT", requires = "summary")]
    pub prompt: Option<String>,

    /// Extract structured data using a JSON Schema.
    /// Accepts inline JSON or @path/to/schema.json.
    #[arg(long, value_name = "SCHEMA")]
    pub extract: Option<String>,

    /// Override LLM provider for this request (anthropic, openai, deepseek, azure, openrouter).
    #[arg(long, value_name = "NAME")]
    pub llm_provider: Option<String>,

    /// Override LLM API key for this request.
    #[arg(long, value_name = "KEY")]
    pub llm_key: Option<String>,

    /// Override LLM model for this request.
    #[arg(long, value_name = "MODEL")]
    pub llm_model: Option<String>,

    /// Override LLM base URL (for OpenAI-compatible or Azure endpoints).
    #[arg(long, value_name = "URL")]
    pub llm_base_url: Option<String>,
}

pub async fn run(mut args: ScrapeArgs) -> Result<(), CmdError> {
    // Local document short-circuit: when the positional arg is an existing file
    // on disk (not a URL), parse it directly. Only PDF is supported.
    if std::path::Path::new(&args.url).is_file() {
        return run_local_file(&args).await;
    }

    // Auto-prepend https:// if no scheme is provided
    if !args.url.contains("://") {
        args.url = format!("https://{}", args.url);
    }

    // First-run nudge for plain scrapes only. AI modes already prompt
    // interactively when there's no config, so we'd be doubling up.
    if !args.summary && args.extract.is_none() {
        maybe_show_first_run_hint();
    }

    let stealth_config = StealthConfig {
        enabled: args.stealth,
        inject_headers: args.stealth,
        ..Default::default()
    };

    // Load app config (config.toml) so we can pick up the user's LLM setup from `crw setup`.
    let app_config = crw_core::config::AppConfig::load().unwrap_or_default();
    let mut cli_extraction_cfg = app_config.extraction.clone();
    let env_cdp_url = std::env::var("CRW_CDP_URL").ok();

    // --extract: inline JSON or `@path/to/schema.json`.
    let extract_schema: Option<serde_json::Value> = match args.extract.as_deref() {
        Some(s) if s.starts_with('@') => {
            let path = &s[1..];
            match std::fs::read_to_string(path) {
                Ok(body) => match serde_json::from_str(&body) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        eprintln!("error: invalid JSON in {path}: {e}");
                        return Err(CmdError::code_only(1));
                    }
                },
                Err(e) => {
                    eprintln!("error: failed to read {path}: {e}");
                    return Err(CmdError::code_only(1));
                }
            }
        }
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!(
                    "error: --extract is not valid JSON: {e}\n\
                     hint: use @path/to/schema.json for files"
                );
                return Err(CmdError::code_only(1));
            }
        },
        None => None,
    };

    let want_summary = args.summary;
    let want_extract = extract_schema.is_some();

    // Resolve effective LlmConfig: config-first, CLI overrides per-field.
    if want_summary || want_extract {
        let merged = match cli_extraction_cfg.llm.clone() {
            Some(mut cfg) => {
                if let Some(p) = args.llm_provider.clone() {
                    cfg.provider = p;
                }
                if let Some(k) = args.llm_key.clone() {
                    cfg.api_key = k;
                }
                if let Some(m) = args.llm_model.clone() {
                    cfg.model = m;
                }
                if args.llm_base_url.is_some() {
                    cfg.base_url = args.llm_base_url.clone();
                }
                Some(cfg)
            }
            None => {
                // No config — need at minimum provider + key + model on the CLI.
                match (
                    args.llm_provider.clone(),
                    args.llm_key.clone(),
                    args.llm_model.clone(),
                ) {
                    (Some(provider), Some(api_key), Some(model)) => {
                        Some(crw_core::config::LlmConfig {
                            provider,
                            api_key,
                            model,
                            base_url: args.llm_base_url.clone(),
                            ..Default::default()
                        })
                    }
                    _ => None,
                }
            }
        };
        let merged = match merged {
            Some(cfg) => Some(cfg),
            None => match run_inline_llm_setup().await {
                Ok(Some(cfg)) => Some(cfg),
                Ok(None) => {
                    eprintln!("Cancelled. --summary/--extract requires an LLM.");
                    return Err(CmdError::code_only(1));
                }
                Err(e) => {
                    eprintln!("error: LLM setup failed: {e}");
                    eprintln!(
                        "hint: run `crw setup` to configure manually, \
                         or pass --llm-provider/--llm-key/--llm-model."
                    );
                    return Err(CmdError::code_only(1));
                }
            },
        };
        cli_extraction_cfg.llm = merged;
    }

    // Request all formats we might need for the output.
    // When --summary/--extract is set, AI output formats are requested and `--format`
    // is ignored; we still include Markdown so phase 1 thinness detection works.
    let request_formats = if want_summary || want_extract {
        let mut v = vec![OutputFormat::Markdown];
        if want_summary {
            v.push(OutputFormat::Summary);
        }
        if want_extract {
            v.push(OutputFormat::Json);
        }
        v
    } else {
        match args.format {
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
            Format::Images => vec![OutputFormat::Images],
        }
    };

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
            args.prompt.clone(),
            extract_schema.clone(),
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
                return Err(CmdError::code_only(1));
            }
        };

        let http_deadline = crw_core::Deadline::from_request_ms(8_000);
        match scrape_url(
            &req,
            &http_renderer,
            cli_extraction_cfg.llm.as_ref(),
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
                    && !want_summary
                    && !want_extract
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
                return Err(CmdError::code_only(1));
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
            args.prompt,
            extract_schema,
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
            cli_extraction_cfg.llm.as_ref(),
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
                return Err(CmdError::code_only(1));
            }
        }
    }

    let data = data.expect("data must be populated by phase 1 or phase 2");
    // Drop guards only after extraction is done so the browser stays alive
    // through the whole fetch + parse pipeline.
    drop(keep_alive_guards);

    // AI output paths short-circuit `--format`. The backend populates
    // `data.summary` / `data.json` when those OutputFormats are requested.
    if want_summary {
        let summary = data.summary.clone().unwrap_or_default();
        match args.output {
            Some(path) => {
                if let Err(e) = std::fs::write(&path, &summary) {
                    eprintln!("error: failed to write to {path}: {e}");
                    return Err(CmdError::code_only(1));
                }
            }
            None => print!("{summary}"),
        }
        return Ok(());
    }
    if want_extract {
        let json = data
            .json
            .as_ref()
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default();
        match args.output {
            Some(path) => {
                if let Err(e) = std::fs::write(&path, &json) {
                    eprintln!("error: failed to write to {path}: {e}");
                    return Err(CmdError::code_only(1));
                }
            }
            None => println!("{json}"),
        }
        return Ok(());
    }

    let content = match args.format {
        Format::Markdown => data.markdown.unwrap_or_default(),
        Format::Json => match serde_json::to_string_pretty(&data) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: failed to serialize JSON: {e}");
                return Err(CmdError::code_only(1));
            }
        },
        Format::Html => data.html.unwrap_or_default(),
        Format::Rawhtml => data.raw_html.unwrap_or_default(),
        Format::Text => data.plain_text.unwrap_or_default(),
        Format::Links => data.links.unwrap_or_default().join("\n"),
        Format::Images => data
            .images
            .unwrap_or_default()
            .iter()
            .map(|i| i.url.clone())
            .collect::<Vec<_>>()
            .join("\n"),
    };

    match args.output {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &content) {
                eprintln!("error: failed to write to {path}: {e}");
                return Err(CmdError::code_only(1));
            }
        }
        None => print!("{content}"),
    }
    Ok(())
}

/// Print a one-line nudge to run `crw setup` the first time someone scrapes
/// without a config.toml. Idempotent across runs via a dotfile sentinel so
/// long-time CLI-only users (who never want setup) don't get nagged.
fn maybe_show_first_run_hint() {
    let Some(cfg_path) = crw_core::config::user_config_path() else {
        return;
    };
    if cfg_path.exists() {
        return;
    }
    let sentinel = cfg_path.with_file_name(".first-run-hint-shown");
    if sentinel.exists() {
        return;
    }
    eprintln!();
    eprintln!(
        "  Tip: run `crw setup` to enable AI features (--summary, --extract) and web search."
    );
    eprintln!();
    if let Some(parent) = sentinel.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::File::create(&sentinel);
}

/// Triggered when the user invokes `--summary` / `--extract` but no LLM is
/// configured. Asks once whether to set one up now; on yes, runs the same
/// interactive flow `crw setup` uses, writes the result to config.toml so it
/// sticks across runs, and returns the resolved `LlmConfig` to continue the
/// in-flight request.
async fn run_inline_llm_setup()
-> Result<Option<crw_core::config::LlmConfig>, crate::commands::setup::ui::SetupError> {
    use crate::commands::setup::config_file::{
        ExtractionSection, LlmSection, UserConfig, write_user_config,
    };
    use crate::commands::setup::{llm, ui};
    use dialoguer::{Confirm, theme::ColorfulTheme};

    ui::init_color(false);
    println!();
    println!(
        "  --summary / --extract requires an LLM, but none is configured in \
         ~/.config/crw/config.toml."
    );
    let confirm = Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt("Configure one now?")
        .default(true)
        .interact()
        .map_err(ui::handle_dialoguer_error)?;
    if !confirm {
        return Ok(None);
    }

    let result = match llm::run().await? {
        Some(r) => r,
        None => return Ok(None), // user picked "Skip" in the provider list
    };

    // Persist to config.toml so the next `crw … --summary` just works.
    let user_cfg = UserConfig {
        client: None,
        search: None,
        extraction: Some(ExtractionSection {
            llm: Some(LlmSection {
                provider: Some(result.provider.config_value().to_string()),
                api_key: Some(result.api_key.clone()),
                model: Some(result.model.clone()),
                base_url: result.base_url.clone(),
                azure_api_version: result.azure_api_version.clone(),
            }),
        }),
    };
    match write_user_config(user_cfg) {
        Ok(path) => {
            ui::print_success(&format!("Saved to {}", path.display()));
        }
        Err(e) => {
            // Don't bail — we can still run this one request with what we have.
            eprintln!("warning: failed to save config.toml: {e}");
        }
    }

    Ok(Some(crw_core::config::LlmConfig {
        provider: result.provider.config_value().to_string(),
        api_key: result.api_key,
        model: result.model,
        base_url: result.base_url,
        azure_api_version: result.azure_api_version,
        ..Default::default()
    }))
}

/// Parse a local document file (PDF) directly, bypassing the network fetch.
/// Supports markdown/json/text/links output plus `--summary`/`--extract` when a
/// server-side LLM is configured (via `crw setup`).
async fn run_local_file(args: &ScrapeArgs) -> Result<(), CmdError> {
    let path = args.url.clone();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: failed to read {path}: {e}");
            return Err(CmdError::code_only(1));
        }
    };

    let is_pdf = path.to_ascii_lowercase().ends_with(".pdf")
        || bytes
            .strip_prefix(&[0xEF, 0xBB, 0xBF])
            .unwrap_or(&bytes)
            .trim_ascii_start()
            .starts_with(b"%PDF-");
    if !is_pdf {
        eprintln!("error: local file parsing currently supports only PDF files (got {path})");
        return Err(CmdError::code_only(1));
    }

    let want_summary = args.summary;
    let want_extract = args.extract.is_some();

    // --extract schema (inline JSON or @path), mirroring the URL path.
    let extract_schema: Option<serde_json::Value> = match args.extract.as_deref() {
        Some(s) if s.starts_with('@') => match std::fs::read_to_string(&s[1..]) {
            Ok(body) => serde_json::from_str(&body).ok(),
            Err(e) => {
                eprintln!("error: failed to read {}: {e}", &s[1..]);
                return Err(CmdError::code_only(1));
            }
        },
        Some(s) => match serde_json::from_str(s) {
            Ok(v) => Some(v),
            Err(e) => {
                eprintln!("error: --extract is not valid JSON: {e}");
                return Err(CmdError::code_only(1));
            }
        },
        None => None,
    };

    let app_config = crw_core::config::AppConfig::load().unwrap_or_default();

    let formats = if want_summary || want_extract {
        let mut v = vec![OutputFormat::Markdown];
        if want_summary {
            v.push(OutputFormat::Summary);
        }
        if want_extract {
            v.push(OutputFormat::Json);
        }
        v
    } else {
        match args.format {
            Format::Markdown => vec![OutputFormat::Markdown],
            Format::Json => vec![OutputFormat::Markdown],
            Format::Html | Format::Rawhtml => vec![OutputFormat::Markdown],
            Format::Text => vec![OutputFormat::PlainText],
            Format::Links => vec![OutputFormat::Links],
            // PDFs carry no HTML image sources; request it anyway (returns empty).
            Format::Images => vec![OutputFormat::Images],
        }
    };

    let req = ScrapeRequest {
        formats,
        json_schema: extract_schema,
        summary_prompt: args.prompt.clone(),
        ..Default::default()
    };

    let source = crw_crawl::pdf::PdfSource {
        source_url: format!("file://{path}"),
        status_code: 200,
        elapsed_ms: 0,
        source_filename: std::path::Path::new(&path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned()),
    };

    let mut data = match crw_crawl::pdf::convert_pdf_bytes(bytes, &req, source).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("error: PDF conversion failed: {e}");
            return Err(CmdError::code_only(1));
        }
    };

    if (want_summary || want_extract)
        && let Err(e) =
            crw_crawl::pdf::apply_llm_formats(&mut data, &req, app_config.extraction.llm.as_ref())
                .await
    {
        eprintln!("error: {e}");
        eprintln!("hint: run `crw setup` to configure an LLM for --summary/--extract.");
        return Err(CmdError::code_only(1));
    }

    for w in &data.warnings {
        eprintln!("warning: {w}");
    }

    let content = if want_summary {
        data.summary.unwrap_or_default()
    } else if want_extract {
        data.json
            .as_ref()
            .map(|v| serde_json::to_string_pretty(v).unwrap_or_default())
            .unwrap_or_default()
    } else {
        match args.format {
            Format::Markdown => data.markdown.unwrap_or_default(),
            Format::Json => serde_json::to_string_pretty(&data).unwrap_or_default(),
            Format::Html | Format::Rawhtml => {
                eprintln!("warning: HTML output is unavailable for PDF; returning markdown");
                data.markdown.unwrap_or_default()
            }
            Format::Text => data.plain_text.unwrap_or_default(),
            Format::Links => data.links.unwrap_or_default().join("\n"),
            Format::Images => data
                .images
                .unwrap_or_default()
                .iter()
                .map(|i| i.url.clone())
                .collect::<Vec<_>>()
                .join("\n"),
        }
    };

    match &args.output {
        Some(p) => {
            if let Err(e) = std::fs::write(p, &content) {
                eprintln!("error: failed to write to {p}: {e}");
                return Err(CmdError::code_only(1));
            }
        }
        None => println!("{content}"),
    }
    Ok(())
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
    summary_prompt: Option<String>,
    extract_schema: Option<serde_json::Value>,
) -> ScrapeRequest {
    let extract = extract_schema
        .clone()
        .map(|s| crw_core::types::ExtractOptions {
            schema: Some(s),
            prompt: None,
        });
    ScrapeRequest {
        url,
        formats,
        only_main_content,
        render_js,
        wait_for: None,
        include_tags: vec![],
        exclude_tags: vec![],
        json_schema: extract_schema,
        basis: false,
        headers: HashMap::new(),
        css_selector: css,
        xpath,
        chunk_strategy: None,
        query: None,
        filter_mode: None,
        top_k: None,
        proxy,
        proxy_list: Vec::new(),
        proxy_rotation: None,
        country: None,
        stealth: if stealth { Some(true) } else { None },
        actions: None,
        extract,
        llm_api_key: None,
        llm_provider: None,
        llm_model: None,
        base_url: None,
        summary_prompt,
        max_content_chars: None,
        renderer: None,
        force_cloak: None,
        deadline_ms: None,
        debug: None,
        change_tracking: None,
        goal: None,
        judge_enabled: None,
        parsers: None,
        screenshot_full_page: false,
    }
}
