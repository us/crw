//! Local setup flow for CRW.

use crate::commands::setup::browser::{self, BrowserEngine};
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

    let browser_engine = prompt_browser_engine().await?;
    let browser_installed = match browser_engine {
        BrowserEngine::LightPanda => {
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
                ui::print_success(&format!("Using existing Chrome at {}", path.display()));
                true
            } else {
                ui::print_warning("Chrome not detected. You'll need to install it manually.");
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

    println!("  CRW's search feature uses SearXNG, a privacy-respecting");
    println!("  meta search engine that aggregates results from Google,");
    println!("  Bing, DuckDuckGo, and 70+ other sources.");
    println!();

    let searxng_url = if docker_available {
        prompt_searxng_setup().await?
    } else {
        ui::print_warning("Skipping SearXNG (Docker not available)");
        ui::print_detail("crw search command won't work without SearXNG");
        None
    };

    println!();

    // Step 4: LLM configuration (optional)
    ui::print_step(4, 5, "LLM Configuration (optional)");

    let llm_result = llm::run().await?;

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

    // Print configuration summary
    let browser_status = if browser_installed {
        "LightPanda installed"
    } else if browser::detect_chrome().is_some() {
        "Chrome (existing)"
    } else {
        "Not configured (HTTP only)"
    };

    let summary_items = vec![
        SummaryItem::new(
            "Browser Engine",
            browser_status,
            browser_installed || browser::detect_chrome().is_some(),
        ),
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

    if llm_result.is_some() {
        quick_start.push("crw example.com -f summary   # AI-powered summary");
    }

    quick_start.push("crw serve                    # Start API server");

    let mut extras = Vec::new();
    if searxng_url.is_some() {
        extras.push("SearXNG management:");
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
        .items(&[
            "Retry (I just started Docker)",
            "Continue without search (skip SearXNG)",
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
        "Docker runs SearXNG (search engine) in a container.",
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
        .items(&[
            "Continue without Docker (skip SearXNG)",
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

/// Prompt for browser engine choice.
async fn prompt_browser_engine() -> Result<BrowserEngine, SetupError> {
    let lightpanda_available = browser::get_platform_info().is_some();
    let chrome_detected = browser::detect_chrome();
    let lightpanda_detected = browser::detect_lightpanda();

    let mut items = Vec::new();
    let mut engines = Vec::new();

    // LightPanda option
    if lightpanda_available {
        if lightpanda_detected.is_some() {
            items.push("LightPanda (already installed) ✓".to_string());
        } else {
            items.push("LightPanda (recommended)\n      • Lightweight: ~50MB download\n      • Fast: Rust-native, optimized for scraping\n      • Best for: Most websites".to_string());
        }
        engines.push(BrowserEngine::LightPanda);
    }

    // Chrome option
    if let Some(path) = &chrome_detected {
        items.push(format!(
            "Chrome/Chromium (detected)\n      • Uses: {}\n      • Heavier: ~200MB memory per page\n      • Best for: Complex sites that need full Chrome",
            path.display()
        ));
    } else {
        items.push("Chrome/Chromium (not detected)\n      • Heavier: ~200MB memory per page\n      • Best for: Complex sites that need full Chrome".to_string());
    }
    engines.push(BrowserEngine::Chrome);

    // Skip option
    items.push("Skip (HTTP only)\n      • No JavaScript support\n      • Fastest, lowest resource usage\n      • Best for: Simple HTML sites, APIs".to_string());
    engines.push(BrowserEngine::None);

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Which browser engine would you like?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(engines[choice])
}

/// Handle download failure.
async fn handle_download_failure() -> Result<bool, SetupError> {
    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  What would you like to do?")
        .items(&[
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
        ui::print_success(&format!("SearXNG already running at {}", url));
        return Ok(Some(url.clone()));
    }

    let items = vec![
        "Yes, using Docker (recommended)\n      • Auto-managed container\n      • ~500MB disk space\n      • Starts automatically when needed",
        "No, I'll set it up myself\n      • Manual setup required\n      • See: https://docs.searxng.org",
        "Skip (no search feature)\n      • crw search command won't work\n      • Scraping still works fine",
    ];

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Set up SearXNG for web search?")
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
            ui::print_info("You can set up SearXNG manually and configure CRW_SEARXNG_URL");
            Ok(None)
        }
        2 => {
            ui::print_info("Skipping SearXNG setup");
            Ok(None)
        }
        _ => unreachable!(),
    }
}

/// Prompt for shell configuration.
fn prompt_shell_config() -> Result<bool, SetupError> {
    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Save configuration to your shell?")
        .items(&[
            "Yes, add to shell config (recommended)",
            "No, I'll configure manually",
        ])
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(choice == 0)
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
