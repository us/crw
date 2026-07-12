//! Local setup flow for CRW.

use crate::commands::setup::browser::{self, BrowserEngine};
use crate::commands::setup::config_file::{
    self, ExtractionSection, LlmSection, SearchSection, UserConfig,
};
use crate::commands::setup::docker::{self, DockerStatus};
use crate::commands::setup::llm::{self, LlmSetupResult};
use crate::commands::setup::searxng;
use crate::commands::setup::shell::{self, Shell, ShellConfig};
use crate::commands::setup::ui::{self, SetupError, SummaryItem};
use dialoguer::Select;

/// Run the local setup flow.
pub async fn run() -> Result<(), SetupError> {
    ui::print_section_header("🏠", "LOCAL SETUP");

    println!("  I'll set up everything you need to run CRW locally.");
    println!("  This includes a browser engine for JavaScript rendering");
    println!("  and a search engine for web searches.");
    println!();

    // Step 1: Check requirements
    ui::print_step(1, 5, "Check Requirements");

    let shell = shell::detect_shell();
    let docker_status = docker::check_docker();
    let platform = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    ui::print_success(&format!("Platform: {} {}", platform, arch));
    ui::print_success(&format!("Shell: {}", shell));

    // Check Docker
    let docker_available = match &docker_status {
        DockerStatus::Running { version } => {
            ui::print_success(&format!("Docker: found ({})", extract_version(version)));
            if let Some(disk) = docker::get_available_disk_space() {
                ui::print_detail("Running: Yes");
                ui::print_detail(&format!("Disk space: {}GB available", disk));
            } else {
                ui::print_detail("Running: Yes");
            }
            true
        }
        DockerStatus::NotRunning { version } => {
            ui::print_error(&format!(
                "Docker: found but not running ({})",
                extract_version(version)
            ));
            handle_docker_not_running().await?
        }
        DockerStatus::NotFound => {
            ui::print_error("Docker: not found");
            handle_docker_not_found().await?
        }
    };

    println!();

    // Step 2: Browser engine
    ui::print_step(2, 5, "Browser Engine (for JS rendering)");

    println!("  To scrape JavaScript-heavy sites (SPAs, React, etc.),");
    println!("  CRW needs a browser engine.");
    println!();

    // Cache filesystem-scanning detections once; reuse for both the prompt and
    // the post-prompt status label.
    let browser_chrome_present = browser::detect_chrome().is_some();

    let browser_engine = prompt_browser_engine().await?;
    let browser_installed = match browser_engine {
        BrowserEngine::LightPanda => {
            ui::print_warning("LightPanda is experimental and may timeout on some sites.");
            ui::print_detail("If you experience issues with --js, try Chrome instead.");
            // Check if already installed first
            if browser::detect_lightpanda().is_some() {
                ui::print_success("LightPanda already installed");
                true
            } else {
                ui::print_info("Downloading LightPanda...");
                match browser::download_lightpanda().await {
                    Ok(_) => true,
                    Err(e) => {
                        ui::print_error(&format!("Download failed: {}", e));
                        handle_download_failure().await?
                    }
                }
            }
        }
        BrowserEngine::Chrome => {
            if let Some(path) = browser::detect_chrome() {
                ui::print_success(&format!("Using Chrome at {}", path.display()));
                true
            } else {
                ui::print_warning("Chrome not detected. You'll need to install it manually.");
                ui::print_detail("Download from: https://google.com/chrome");
                false
            }
        }
        BrowserEngine::None => {
            ui::print_info("Skipping browser engine (HTTP-only mode)");
            false
        }
    };

    println!();

    // Step 3: Search engine
    ui::print_step(3, 5, "Search Engine (for web search)");

    println!("  CRW's search feature uses a privacy-respecting meta search");
    println!("  engine that aggregates results from Google, Bing, DuckDuckGo,");
    println!("  and 70+ other sources.");
    println!();

    let searxng_url = if docker_available {
        prompt_searxng_setup().await?
    } else {
        ui::print_warning("Skipping search backend (Docker not available)");
        ui::print_detail("crw search command won't work without a search backend");
        None
    };

    println!();

    // Step 4: LLM configuration (optional)
    ui::print_step(4, 5, "LLM Configuration (optional)");

    let llm_result = llm::run().await?;

    println!();

    // Always persist canonical state to ~/.config/crw/config.toml. The
    // shell rc write below is *additional* (env vars still take precedence
    // for CI/Docker users).
    let cfg_path = config_file::write_user_config(build_user_config(
        searxng_url.as_deref(),
        llm_result.as_ref(),
    ))?;
    ui::print_success(&format!("Saved {}", cfg_path.display()));
    println!();

    // Step 5: Shell configuration
    ui::print_step(5, 5, "Shell Configuration");

    let save_to_shell = prompt_shell_config()?;
    if save_to_shell {
        save_shell_config(
            shell,
            browser_installed,
            searxng_url.as_deref(),
            llm_result.as_ref(),
        )?;
    } else {
        show_manual_config(
            browser_installed,
            searxng_url.as_deref(),
            llm_result.as_ref(),
        );
    }

    // Print configuration summary. Reuse the chrome detection from
    // `prompt_browser_engine` rather than scanning the filesystem again.
    let chrome_present = browser_chrome_present;
    let (browser_status, browser_ok) =
        browser_status_label(browser_engine, browser_installed, chrome_present);

    let summary_items = vec![
        SummaryItem::new("Browser Engine", browser_status, browser_ok),
        SummaryItem::new(
            "Search Engine",
            searxng_url.as_deref().unwrap_or("Not configured"),
            searxng_url.is_some(),
        ),
        SummaryItem::new(
            "LLM Provider",
            llm_result
                .as_ref()
                .map(|l| l.provider.name())
                .unwrap_or("Not configured"),
            llm_result.is_some(),
        ),
    ];
    ui::print_summary("Configuration Summary", &summary_items);

    // Print completion banner
    let source_cmd = shell::source_command(shell);

    let mut quick_start = vec!["crw example.com              # Scrape (HTTP)"];

    if browser_installed {
        quick_start.push("crw example.com --js         # Scrape with JavaScript");
    }

    if searxng_url.is_some() {
        quick_start.push("crw search \"rust tutorials\"  # Web search");
    }

    quick_start.push("crw serve                    # Start API server");

    let mut extras = Vec::new();
    if searxng_url.is_some() {
        extras.push("Search backend management:");
        extras.push("  docker start searxng         # Start search engine");
        extras.push("  docker stop searxng          # Stop search engine");
        extras.push("");
    }
    extras.push("Documentation: https://fastcrw.com/docs");

    let extras_refs: Vec<&str> = extras.iter().map(|s| s.as_ref()).collect();

    ui::print_completion_banner(source_cmd.as_deref(), &quick_start, &extras_refs);

    Ok(())
}

/// Extract version number from docker version string.
fn extract_version(full: &str) -> &str {
    // "Docker version 24.0.5, build ..." -> "24.0.5"
    full.split_whitespace()
        .nth(2)
        .map(|s| s.trim_end_matches(','))
        .unwrap_or(full)
}

/// Handle Docker not running scenario.
async fn handle_docker_not_running() -> Result<bool, SetupError> {
    println!();
    println!("  Please start Docker Desktop and try again.");
    println!();

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  What would you like to do?")
        .items([
            "Retry (I just started Docker)",
            "Continue without search (skip search backend)",
            "Exit",
        ])
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    match choice {
        0 => {
            // Retry
            let status = docker::check_docker();
            if status.is_ready() {
                ui::print_success("Docker is now running");
                Ok(true)
            } else {
                ui::print_error("Docker still not running");
                Ok(false)
            }
        }
        1 => Ok(false),
        2 => Err(SetupError::Cancelled),
        _ => unreachable!(),
    }
}

/// Handle Docker not found scenario.
async fn handle_docker_not_found() -> Result<bool, SetupError> {
    let instructions = docker::docker_install_instructions();
    let mut lines = vec![
        "Docker is required for local search setup",
        "",
        "Docker runs the search engine in a container.",
        "Without it, you can still scrape but not search.",
        "",
        "Install Docker:",
    ];
    for inst in &instructions {
        lines.push(inst);
    }

    ui::print_info_box(&lines);

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  What would you like to do?")
        .items([
            "Continue without Docker (skip search backend)",
            "Exit and install Docker first",
        ])
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    match choice {
        0 => Ok(false),
        1 => Err("Please install Docker and run 'crw setup' again.".into()),
        _ => unreachable!(),
    }
}

/// Return the (label, ok) pair shown for the Browser Engine summary row.
///
/// `installed` reflects whether the chosen engine was actually set up; `chrome_present`
/// reflects whether a Chrome binary exists on disk regardless of the user's choice.
/// Pure — no I/O — so the truth table is unit-testable.
fn browser_status_label(
    engine: BrowserEngine,
    installed: bool,
    chrome_present: bool,
) -> (&'static str, bool) {
    match (engine, installed, chrome_present) {
        (BrowserEngine::Chrome, true, _) => ("Chrome (configured)", true),
        (BrowserEngine::LightPanda, true, _) => ("LightPanda (experimental)", true),
        // User picked a browser but install/detection failed — report the failure
        // explicitly rather than silently advertising any other browser.
        (BrowserEngine::Chrome, false, _) => ("Chrome (install failed)", false),
        (BrowserEngine::LightPanda, false, _) => ("LightPanda (install failed)", false),
        // User declined a browser; surface Chrome as an available fallback only
        // when it's actually present on disk.
        (BrowserEngine::None, _, true) => ("Chrome (available)", true),
        (BrowserEngine::None, _, false) => ("Not configured (HTTP only)", false),
    }
}

/// Build the browser engine selection menu.
///
/// Returns `(items, engines, default_index)`. Pure (no I/O) so the option
/// matrix is unit-testable.
///
/// Ordering rules:
///   - Chrome first when detected (recommended path).
///   - LightPanda shown if either the platform can download it OR a binary
///     is already on disk (user may have side-loaded it on an unsupported
///     platform).
///   - Chrome shown last as "not installed" if not detected.
///   - "Skip" is always last.
///
/// Default selection prefers, in order: detected Chrome, detected LightPanda,
/// otherwise Skip — never silently downgrade a user with a working browser.
fn build_browser_options(
    chrome_path: Option<&std::path::Path>,
    lightpanda_available: bool,
    lightpanda_path: Option<&std::path::Path>,
) -> (Vec<String>, Vec<BrowserEngine>, usize) {
    let mut items = Vec::new();
    let mut engines = Vec::new();
    let lightpanda_installed = lightpanda_path.is_some();
    // Show LightPanda whenever it's available to download OR already on disk.
    let show_lightpanda = lightpanda_available || lightpanda_installed;

    if let Some(path) = chrome_path {
        items.push(format!(
            "Chrome/Chromium (recommended)\n      • Uses: {}\n      • Full CDP support, maximum compatibility\n      • Best for: All JavaScript-heavy sites",
            path.display()
        ));
        engines.push(BrowserEngine::Chrome);
    }

    if show_lightpanda {
        let label = if lightpanda_installed {
            "LightPanda (experimental, installed)"
        } else {
            "LightPanda (experimental)"
        };
        let size_line = if lightpanda_installed {
            "Lightweight: ~50MB"
        } else {
            "Lightweight: ~50MB download"
        };
        items.push(format!(
            "{}\n      • ⚠️  May timeout on some sites (CDP compatibility)\n      • {}\n      • Best for: Simple JS sites only",
            label, size_line
        ));
        engines.push(BrowserEngine::LightPanda);
    }

    if chrome_path.is_none() {
        items.push("Chrome/Chromium (not installed)\n      • Full CDP support, maximum compatibility\n      • Install from: google.com/chrome".to_string());
        engines.push(BrowserEngine::Chrome);
    }

    items.push("Skip (HTTP only)\n      • No JavaScript support\n      • Fastest, lowest resource usage\n      • Best for: Simple HTML sites, APIs".to_string());
    engines.push(BrowserEngine::None);

    // Default: prefer a working browser the user already has.
    let default_choice = if chrome_path.is_some() {
        0 // Chrome is always first when detected.
    } else if lightpanda_installed {
        engines
            .iter()
            .position(|e| *e == BrowserEngine::LightPanda)
            .unwrap_or(items.len() - 1)
    } else {
        items.len() - 1 // Skip.
    };

    (items, engines, default_choice)
}

/// Prompt for browser engine choice.
async fn prompt_browser_engine() -> Result<BrowserEngine, SetupError> {
    let lightpanda_available = browser::get_platform_info().is_some();
    let chrome_detected = browser::detect_chrome();
    let lightpanda_detected = browser::detect_lightpanda();

    let (items, engines, default_choice) = build_browser_options(
        chrome_detected.as_deref(),
        lightpanda_available,
        lightpanda_detected.as_deref(),
    );

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Which browser engine would you like?")
        .items(&items)
        .default(default_choice)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(engines[choice])
}

/// Handle download failure.
async fn handle_download_failure() -> Result<bool, SetupError> {
    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  What would you like to do?")
        .items([
            "Retry download",
            "Skip LightPanda (use Chrome if available)",
            "Continue without browser (HTTP only)",
        ])
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    match choice {
        0 => {
            // Retry
            match browser::download_lightpanda().await {
                Ok(_) => Ok(true),
                Err(e) => {
                    ui::print_error(&format!("Download failed again: {}", e));
                    Ok(false)
                }
            }
        }
        1 => {
            if browser::detect_chrome().is_some() {
                ui::print_info("Will use Chrome for JavaScript rendering");
                Ok(true)
            } else {
                ui::print_warning("Chrome not detected, continuing without JS rendering");
                Ok(false)
            }
        }
        2 => Ok(false),
        _ => unreachable!(),
    }
}

/// Prompt for SearXNG setup.
async fn prompt_searxng_setup() -> Result<Option<String>, SetupError> {
    let status = searxng::check_status();

    // If already running, just return the URL
    if let searxng::SearxngStatus::Running { url } = &status {
        ui::print_success(&format!("Search backend already running at {}", url));
        return Ok(Some(url.clone()));
    }

    let items = vec![
        "Yes, using Docker (recommended)\n      • Auto-managed container\n      • ~500MB disk space\n      • Starts automatically when needed",
        "No, I'll set it up myself\n      • Manual setup required\n      • Point CRW_SEARXNG_URL at your own instance",
        "Skip (no search feature)\n      • crw search command won't work\n      • Scraping still works fine",
    ];

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Set up a search backend for web search?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    match choice {
        0 => {
            // Install with Docker
            searxng::pull_image().await?;
            let url = searxng::start_container().await?;
            Ok(Some(url))
        }
        1 => {
            ui::print_info(
                "You can set up a search backend manually and configure CRW_SEARXNG_URL",
            );
            Ok(None)
        }
        2 => {
            ui::print_info("Skipping search backend setup");
            Ok(None)
        }
        _ => unreachable!(),
    }
}

/// Prompt for shell configuration.
///
/// Note: `~/.config/crw/config.toml` is already the source of truth at this
/// point — shell exports are *only* needed if you want env vars to win over
/// the file (CI, Docker, scripts). Default is therefore No, to keep the
/// user's rc file clean.
fn prompt_shell_config() -> Result<bool, SetupError> {
    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Also export to your shell rc? (optional)")
        .items([
            "No, config.toml is enough (recommended)",
            "Yes — also add `export CRW_*` lines (for CI/Docker/scripts)",
        ])
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(choice == 1)
}

/// Build the `UserConfig` for `~/.config/crw/config.toml`. Only fills in
/// sections setup actually touched; everything else stays `None` so
/// `merge_config` preserves prior values across re-runs.
fn build_user_config(searxng_url: Option<&str>, llm_result: Option<&LlmSetupResult>) -> UserConfig {
    UserConfig {
        client: None,
        search: searxng_url.map(|url| SearchSection {
            searxng_url: Some(url.to_string()),
        }),
        extraction: llm_result.map(|llm| ExtractionSection {
            llm: Some(LlmSection {
                provider: Some(llm.provider.config_value().to_string()),
                api_key: Some(llm.api_key.clone()),
                model: Some(llm.model.clone()),
                base_url: llm.base_url.clone(),
                azure_api_version: llm.azure_api_version.clone(),
            }),
        }),
    }
}

/// Save configuration to shell RC file.
fn save_shell_config(
    shell: Shell,
    browser_installed: bool,
    searxng_url: Option<&str>,
    llm_result: Option<&LlmSetupResult>,
) -> Result<(), String> {
    let mut config = ShellConfig::new();

    // Add ~/.local/bin to PATH if browser was installed
    if browser_installed {
        config.add_to_path("$HOME/.local/bin");
    }

    // Add SearXNG URL if configured
    if let Some(url) = searxng_url {
        config.export("CRW_SEARXNG_URL", url);
    }

    // Add LLM config if provided
    if let Some(llm) = llm_result {
        llm::add_to_shell_config(&mut config, llm);
    }

    // Only write if we have something to add
    if config.lines.is_empty() {
        ui::print_info("No configuration changes needed");
        return Ok(());
    }

    let rc_path = shell::append_to_rc(shell, &config)?;

    ui::print_success(&format!("Added to {}:", rc_path.display()));
    for line in &config.lines {
        println!("    {}", line);
    }
    println!();

    Ok(())
}

/// Show manual configuration instructions.
fn show_manual_config(
    browser_installed: bool,
    searxng_url: Option<&str>,
    llm_result: Option<&LlmSetupResult>,
) {
    println!();
    println!("  Add these to your shell configuration:");
    println!();

    if browser_installed {
        println!("    export PATH=\"$HOME/.local/bin:$PATH\"");
    }

    if let Some(url) = searxng_url {
        println!("    export CRW_SEARXNG_URL=\"{}\"", url);
    }

    if let Some(llm) = llm_result {
        llm::show_manual_config(llm);
    }

    if !browser_installed && searxng_url.is_none() && llm_result.is_none() {
        println!("    (no configuration needed)");
    }

    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ---- build_browser_options ----------------------------------------------

    #[test]
    fn options_chrome_detected_lists_chrome_first_and_defaults_to_it() {
        let chrome = Path::new("/usr/bin/google-chrome");
        let (items, engines, default) = build_browser_options(Some(chrome), true, None);
        assert_eq!(engines[0], BrowserEngine::Chrome);
        assert_eq!(default, 0);
        assert!(items[0].contains("recommended"));
        assert!(items.iter().any(|i| i.contains("experimental")));
        assert_eq!(*engines.last().unwrap(), BrowserEngine::None);
    }

    #[test]
    fn options_no_chrome_with_lightpanda_installed_defaults_to_lightpanda() {
        // Regression: previously defaulted to Skip, silently downgrading existing
        // LightPanda users to HTTP-only.
        let lp = Path::new("/home/u/.local/bin/lightpanda");
        let (_, engines, default) = build_browser_options(None, true, Some(lp));
        assert_eq!(engines[default], BrowserEngine::LightPanda);
    }

    #[test]
    fn options_no_chrome_no_lightpanda_defaults_to_skip() {
        let (items, engines, default) = build_browser_options(None, false, None);
        assert_eq!(engines[default], BrowserEngine::None);
        assert!(items[default].contains("HTTP only"));
    }

    #[test]
    fn options_lightpanda_detected_but_platform_unsupported_still_shows_it() {
        // Regression: previously hidden when get_platform_info() returned None.
        let lp = Path::new("/opt/lp");
        let (items, engines, _) = build_browser_options(None, false, Some(lp));
        assert!(items.iter().any(|i| i.contains("experimental")));
        assert!(engines.contains(&BrowserEngine::LightPanda));
    }

    #[test]
    fn options_lightpanda_installed_label_says_installed() {
        let chrome = Path::new("/c");
        let lp = Path::new("/lp");
        let (items, _, _) = build_browser_options(Some(chrome), true, Some(lp));
        assert!(items.iter().any(|i| i.contains("installed")));
    }

    #[test]
    fn options_skip_is_always_last() {
        for &(chrome, lp_avail, lp_inst) in &[
            (true, true, true),
            (true, false, false),
            (false, true, true),
            (false, false, false),
        ] {
            let c = if chrome { Some(Path::new("/c")) } else { None };
            let l = if lp_inst { Some(Path::new("/l")) } else { None };
            let (_, engines, _) = build_browser_options(c, lp_avail, l);
            assert_eq!(
                *engines.last().unwrap(),
                BrowserEngine::None,
                "Skip must be last for chrome={} lp_avail={} lp_inst={}",
                chrome,
                lp_avail,
                lp_inst
            );
        }
    }

    // ---- browser_status_label -----------------------------------------------

    #[test]
    fn status_chrome_configured() {
        assert_eq!(
            browser_status_label(BrowserEngine::Chrome, true, true),
            ("Chrome (configured)", true)
        );
    }

    #[test]
    fn status_lightpanda_configured() {
        assert_eq!(
            browser_status_label(BrowserEngine::LightPanda, true, false),
            ("LightPanda (experimental)", true)
        );
    }

    #[test]
    fn status_chrome_install_failed_does_not_advertise_other_browser() {
        // Regression: previously masked install failure as "Chrome (available)".
        let (label, ok) = browser_status_label(BrowserEngine::Chrome, false, true);
        assert!(label.contains("install failed"));
        assert!(!ok);
    }

    #[test]
    fn status_lightpanda_install_failed_reports_failure() {
        let (label, ok) = browser_status_label(BrowserEngine::LightPanda, false, true);
        assert!(label.contains("install failed"));
        assert!(!ok);
    }

    #[test]
    fn status_skipped_with_chrome_on_disk_shows_chrome_available() {
        assert_eq!(
            browser_status_label(BrowserEngine::None, false, true),
            ("Chrome (available)", true)
        );
    }

    #[test]
    fn status_skipped_without_chrome_says_not_configured() {
        assert_eq!(
            browser_status_label(BrowserEngine::None, false, false),
            ("Not configured (HTTP only)", false)
        );
    }
}
