//! Cloud setup flow for CRW.

use crate::commands::setup::config_file::{
    self, ClientSection, ExtractionSection, LlmSection, UserConfig,
};
use crate::commands::setup::llm::{self, LlmSetupResult};
use crate::commands::setup::shell::{self, Shell, ShellConfig};
use crate::commands::setup::ui::{self, SetupError, SummaryItem};
use console::style;
use dialoguer::{Input, Select};
use serde::Deserialize;

const API_BASE_URL: &str = "https://api.fastcrw.com";
const DASHBOARD_URL: &str = "https://fastcrw.com/dashboard";

/// Account info returned from API key validation.
#[derive(Debug, Deserialize)]
struct AccountInfo {
    credits_remaining: Option<i64>,
    #[allow(dead_code)]
    email: Option<String>,
}

/// API key validation result.
#[derive(Debug)]
pub enum ApiKeyStatus {
    Valid { credits: i64 },
    Invalid,
    NetworkError(String),
}

/// Run the cloud setup flow.
pub async fn run() -> Result<(), SetupError> {
    ui::print_section_header("☁️", "CLOUD SETUP");

    // Step 1: Get CRW API key
    ui::print_step(1, 3, "Get your CRW API key");

    println!("  Visit: {}", style(DASHBOARD_URL).cyan().underlined());
    println!();
    println!("  1. Sign up (GitHub/Google, takes 10 seconds)");
    println!("  2. Copy your API key from the dashboard");
    println!();

    let api_key = get_api_key().await?;

    // Step 2: Configure LLM (optional)
    ui::print_step(2, 3, "Configure LLM (optional)");

    let llm_result = llm::run().await?;

    // Step 3: Save configuration
    ui::print_step(3, 3, "Save configuration");

    let shell = shell::detect_shell();
    ui::print_info(&format!("Detected shell: {}", shell));
    println!();

    // Always persist canonical state to ~/.config/crw/config.toml. The shell
    // rc / manual options below are *additional* convenience layers, not
    // alternatives — env vars (CRW_*) still take precedence over the file
    // for CI/Docker users.
    let cfg_path =
        config_file::write_user_config(build_user_config(&api_key, llm_result.as_ref()))?;
    ui::print_success(&format!("Saved {}", cfg_path.display()));
    println!();

    let save_location = prompt_save_location(shell)?;

    match save_location {
        SaveLocation::ShellRc => {
            save_to_shell_rc(shell, &api_key, llm_result.as_ref())?;
        }
        SaveLocation::ConfigFile => {
            // Already written above — nothing extra to do beyond the success line.
            ui::print_info("Config file is the source of truth for these settings.");
            println!();
        }
        SaveLocation::Manual => {
            show_manual_instructions(&api_key, llm_result.as_ref());
        }
    }

    // Print configuration summary
    let summary_items = vec![
        SummaryItem::new("Cloud API", "Connected (fastcrw.com)", true),
        SummaryItem::new(
            "LLM Provider",
            llm_result
                .as_ref()
                .map(|l| l.provider.name())
                .unwrap_or("Not configured"),
            llm_result.is_some(),
        ),
        SummaryItem::new(
            "Config saved",
            match save_location {
                SaveLocation::ShellRc => "~/.zshrc",
                SaveLocation::ConfigFile => "~/.config/crw/config.toml",
                SaveLocation::Manual => "Manual (env vars)",
            },
            true,
        ),
    ];
    ui::print_summary("Configuration Summary", &summary_items);

    // Build quick start based on what was configured
    let mut quick_start = vec![
        "crw example.com              # Scrape a page",
        "crw search \"rust tutorials\"  # Web search",
    ];

    quick_start.push("crw --help                   # See all commands");

    // Print completion banner
    let source_cmd = shell::source_command(shell);
    ui::print_completion_banner(source_cmd.as_deref(), &quick_start, &[]);

    Ok(())
}

/// Get and validate API key from user.
async fn get_api_key() -> Result<String, SetupError> {
    loop {
        let api_key: String = Input::with_theme(&ui::select_style())
            .with_prompt("  Paste your API key")
            .validate_with(|input: &String| {
                if input.trim().is_empty() {
                    Err("API key cannot be empty")
                } else if !input.starts_with("fc-")
                    && !input.starts_with("sk-")
                    && !input.starts_with("crw_")
                {
                    Err("API key should start with 'fc-', 'sk-', or 'crw_'")
                } else {
                    Ok(())
                }
            })
            .interact_text()
            .map_err(ui::handle_dialoguer_error)?;

        let api_key = api_key.trim().to_string();

        // Validate API key
        print!("  ");
        match validate_api_key(&api_key).await {
            ApiKeyStatus::Valid { credits } => {
                ui::print_success(&format!(
                    "API key validated ({} credits remaining)",
                    credits
                ));
                println!();
                return Ok(api_key);
            }
            ApiKeyStatus::Invalid => {
                ui::print_error("Invalid API key");
                println!();
                println!("  The API key couldn't be verified. Try these steps:");
                println!("  1. Check for extra spaces (copy exactly from dashboard)");
                println!("  2. Ensure key hasn't been revoked");
                println!("  3. Check network: curl -I {}/health", API_BASE_URL);
                println!();

                let choice = Select::with_theme(&ui::select_style())
                    .with_prompt("  What would you like to do?")
                    .items([
                        "Try again",
                        "Get a new key (opens browser)",
                        "Continue anyway (key not verified)",
                    ])
                    .default(0)
                    .interact()
                    .map_err(ui::handle_dialoguer_error)?;

                match choice {
                    0 => continue,
                    1 => {
                        open_browser(DASHBOARD_URL);
                        continue;
                    }
                    2 => {
                        ui::print_warning("Continuing with unverified API key");
                        return Ok(api_key);
                    }
                    _ => unreachable!(),
                }
            }
            ApiKeyStatus::NetworkError(err) => {
                ui::print_warning(&format!("Could not verify API key: {}", err));
                println!();

                let choice = Select::with_theme(&ui::select_style())
                    .with_prompt("  What would you like to do?")
                    .items(["Retry verification", "Continue anyway (key not verified)"])
                    .default(0)
                    .interact()
                    .map_err(ui::handle_dialoguer_error)?;

                match choice {
                    0 => continue,
                    1 => {
                        ui::print_warning("Continuing with unverified API key");
                        return Ok(api_key);
                    }
                    _ => unreachable!(),
                }
            }
        }
    }
}

/// Validate API key against the API.
async fn validate_api_key(key: &str) -> ApiKeyStatus {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return ApiKeyStatus::NetworkError(e.to_string()),
    };

    let resp = match client
        .get(format!("{}/v1/account", API_BASE_URL))
        .header("Authorization", format!("Bearer {}", key))
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ApiKeyStatus::NetworkError(e.to_string()),
    };

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return ApiKeyStatus::Invalid;
    }

    if !resp.status().is_success() {
        return ApiKeyStatus::NetworkError(format!("HTTP {}", resp.status()));
    }

    match resp.json::<AccountInfo>().await {
        Ok(info) => ApiKeyStatus::Valid {
            credits: info.credits_remaining.unwrap_or(0),
        },
        Err(_) => {
            // If we got a 200 but can't parse, assume it's valid
            ApiKeyStatus::Valid { credits: -1 }
        }
    }
}

/// Where to save the configuration.
#[derive(Debug, Clone, Copy)]
enum SaveLocation {
    ShellRc,
    ConfigFile,
    Manual,
}

/// Prompt for any extra export step on top of the already-written config.toml.
///
/// Re-ordered so `ConfigFile` (do-nothing-extra) is the default: the file
/// is already the canonical state. Shell exports are only useful when env
/// vars need to win over the file (CI / Docker / scripts).
fn prompt_save_location(shell: Shell) -> Result<SaveLocation, SetupError> {
    let rc_file = shell::get_rc_file(shell);
    let rc_name = rc_file
        .as_ref()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap_or("shell rc");

    let items = vec![
        "Nothing extra — config.toml is enough (recommended)".to_string(),
        format!(
            "Also append `export CRW_*` to ~/{} (for CI/Docker)",
            rc_name
        ),
        "Print env vars to copy/paste manually".to_string(),
    ];

    let choice = Select::with_theme(&ui::select_style())
        .with_prompt("  Anything else?")
        .items(&items)
        .default(0)
        .interact_opt()
        .map_err(ui::handle_dialoguer_error)?
        .ok_or(SetupError::Cancelled)?;

    Ok(match choice {
        0 => SaveLocation::ConfigFile,
        1 => SaveLocation::ShellRc,
        2 => SaveLocation::Manual,
        _ => unreachable!(),
    })
}

/// Save configuration to shell RC file.
fn save_to_shell_rc(
    shell: Shell,
    api_key: &str,
    llm_result: Option<&LlmSetupResult>,
) -> Result<(), String> {
    let mut config = ShellConfig::new();
    config.export("CRW_API_URL", API_BASE_URL);
    config.export("CRW_API_KEY", api_key);

    // Add LLM config if provided
    if let Some(llm) = llm_result {
        llm::add_to_shell_config(&mut config, llm);
    }

    let rc_path = shell::append_to_rc(shell, &config)?;

    ui::print_success(&format!("Added to {}:", rc_path.display()));
    println!("    export CRW_API_URL=\"{}\"", API_BASE_URL);
    println!(
        "    export CRW_API_KEY=\"{}...\"",
        &api_key[..std::cmp::min(8, api_key.len())]
    );

    if let Some(llm) = llm_result {
        println!(
            "    export CRW_EXTRACTION__LLM__PROVIDER=\"{}\"",
            llm.provider.config_value()
        );
        println!("    export CRW_EXTRACTION__LLM__MODEL=\"{}\"", llm.model);
    }

    println!();
    Ok(())
}

/// Mask an API key for display (show first 4 and last 4 chars).
fn mask_api_key(key: &str) -> String {
    if key.len() <= 12 {
        return "*".repeat(key.len());
    }
    format!("{}...{}", &key[..4], &key[key.len() - 4..])
}

/// Build the `UserConfig` we'll persist to `~/.config/crw/config.toml`.
///
/// Only sections the wizard actually touched are filled in. Anything else
/// (search, etc.) is left as `None` so a previous run's value survives the
/// merge in `config_file::write_user_config`.
fn build_user_config(api_key: &str, llm_result: Option<&LlmSetupResult>) -> UserConfig {
    UserConfig {
        client: Some(ClientSection {
            api_url: Some(API_BASE_URL.to_string()),
            api_key: Some(api_key.to_string()),
        }),
        search: None,
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

/// Show manual configuration instructions.
fn show_manual_instructions(api_key: &str, llm_result: Option<&LlmSetupResult>) {
    println!();
    println!("  Add these environment variables to your shell:");
    println!();
    println!("    export CRW_API_URL=\"{}\"", API_BASE_URL);
    println!("    export CRW_API_KEY=\"{}\"", mask_api_key(api_key));

    if let Some(llm) = llm_result {
        llm::show_manual_config(llm);
    } else {
        println!();
    }
}

/// Open URL in default browser.
fn open_browser(url: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/c", "start", url])
            .spawn();
    }
}
